//! The probes `app.ts` and its small routers answer: `/`, `/health/live`, `/health/ready`, `/health/release-gates`
//! (`routes/releaseGates.ts`), `/internal/v1/metrics` (`routes/metrics.ts`), `/maintenance/status`
//! (`routes/loginQueue.ts`) and `/dauntless-status` (`routes/system.ts`, the one that answers: the Reforged route of
//! the same path is shadowed by it, RU3a).

use std::sync::{Mutex, PoisonError};

use axum::response::Response;
use serde_json::{Value, json};

use super::release_gates::{self, Gate};
use super::util::{self, Req};
use super::{metrics, presence};
use crate::http::{Call, Shared};

/// `GetServiceReadiness`: a listener is up when a request arrives, so the module starts ready; `set_readiness`
/// is for the listener plan to say otherwise.
struct Readiness {
    ready: bool,
    reason: String,
    changed_at: i64,
}

static READINESS: Mutex<Option<Readiness>> = Mutex::new(None);

pub fn set_readiness(ready: bool, reason: &str) {
    let mut guard = READINESS.lock().unwrap_or_else(PoisonError::into_inner);
    *guard = Some(Readiness {
        ready,
        reason: reason.chars().take(96).collect(),
        changed_at: util::now_ms(),
    });
}

fn readiness() -> (bool, String, i64) {
    let mut guard = READINESS.lock().unwrap_or_else(PoisonError::into_inner);
    let state = guard.get_or_insert_with(|| Readiness {
        ready: true,
        reason: "API_BEHIND_PROXY_RUNNING".to_owned(),
        changed_at: util::now_ms(),
    });
    (state.ready, state.reason.clone(), state.changed_at)
}

/// `RUSTIC_ISLES_SERVICE_ROLE`: combined or api; anything else is a start-up error in Node, and here the default.
pub fn service_role(shared: &Shared) -> &'static str {
    match shared
        .settings
        .raw("RUSTIC_ISLES_SERVICE_ROLE")
        .unwrap_or_default()
        .trim()
        .to_lowercase()
        .as_str()
    {
        "api" => "api",
        _ => "combined",
    }
}

/// `GetPersistenceLifecycle().isHealthy()`: a database is configured and answers.
async fn persistence_healthy(shared: &Shared) -> bool {
    let Some(store) = &shared.store else {
        return false;
    };
    store
        .read(|conn, _| {
            use diesel::RunQueryDsl;
            diesel::sql_query("SELECT 1").execute(conn)?;
            Ok(())
        })
        .await
        .is_ok()
}

fn live(call: &Call) -> Response {
    util::json(
        200,
        &json!({ "ok": true, "service": "paradox-backend", "role": service_role(&call.shared) }),
    )
}

async fn ready(shared: std::sync::Arc<Shared>) -> Response {
    let (state_ready, reason, _) = readiness();
    let persistence = persistence_healthy(&shared).await;
    let ok = state_ready && persistence;
    util::json(
        if ok { 200 } else { 503 },
        &json!({
            "ok": ok,
            "service": "paradox-backend",
            "role": service_role(&shared),
            "reason": if ok { "READY".to_owned() } else { reason },
            "persistence": if persistence { "ready" } else { "unavailable" },
        }),
    )
}

pub async fn collect_release_gates(shared: &Shared) -> Vec<Gate> {
    let env = shared.settings.env_map();
    let playing = presence::active_players(release_gates::ACTIVE_PLAY_WINDOW_MS).len();
    let root = update_root(shared);
    let overridden = root
        .parent()
        .is_some_and(|dir| dir.join(release_gates::RELEASE_DURING_PLAY_FILE).exists());
    let mut gates = vec![release_gates::active_play_gate(playing, overridden)];
    gates.extend(release_gates::core_gates(env));
    gates
}

/// `LAUNCHER_UPDATE_ROOT`, else `./updates`.
pub fn update_root(shared: &Shared) -> std::path::PathBuf {
    let root = shared
        .settings
        .raw("LAUNCHER_UPDATE_ROOT")
        .map_or_else(|| std::path::PathBuf::from("updates"), std::path::PathBuf::from);
    std::path::absolute(&root).unwrap_or(root)
}

async fn release_gates(call: Call) -> Response {
    let shared = call.shared.clone();
    let Ok(req) = Req::read(call.request, util::BodyKind::None).await else {
        return util::empty(404);
    };
    if !util::is_loopback_peer(req.remote_address().as_deref()) || release_gates::has_forwarding_header(&req.headers) {
        return util::empty(404);
    }
    let gates = collect_release_gates(&shared).await;
    let body = release_gates::body(&gates);
    util::json(if body["ok"] == Value::Bool(true) { 200 } else { 503 }, &body)
}

fn metrics_authorized(shared: &Shared, req: &Req) -> bool {
    let token = shared
        .settings
        .raw("RUSTIC_ISLES_METRICS_TOKEN")
        .map(str::trim)
        .filter(|t| !t.is_empty());
    let Some(expected) = token else {
        let production = shared.settings.raw("NODE_ENV") == Some("production");
        return !production && util::is_loopback_peer(req.remote_address().as_deref());
    };
    let supplied = req
        .header("authorization")
        .filter(|a| a.to_lowercase().starts_with("bearer "))
        .map(|a| a[7..].trim().to_owned())
        .or_else(|| {
            req.header("x-rustic-isles-metrics-token")
                .map(|t| t.trim().to_owned())
                .filter(|t| !t.is_empty())
        });
    supplied.is_some_and(|actual| util::constant_time_eq(expected.as_bytes(), actual.as_bytes()))
}

async fn metrics_route(call: Call) -> Response {
    let shared = call.shared.clone();
    let Ok(req) = Req::read(call.request, util::BodyKind::None).await else {
        return util::empty(400);
    };
    if !metrics_authorized(&shared, &req) {
        let status = if shared
            .settings
            .raw("RUSTIC_ISLES_METRICS_TOKEN")
            .is_some_and(|t| !t.is_empty())
        {
            401
        } else {
            503
        };
        return util::text(status, "text/plain; charset=utf-8", "metrics unavailable\n");
    }
    let (state_ready, _, changed) = readiness();
    let text = metrics::render(
        util::now_ms(),
        service_role(&shared),
        state_ready,
        changed as f64 / 1000.0,
    );
    let mut response = util::text(200, "text/plain; charset=utf-8; version=0.0.4", text);
    util::cache_control(&mut response, "no-store");
    response
}

/// The welcome text `system.ts` answers (the Reforged route of this path never does).
pub fn dauntless_status() -> Value {
    let welcome = "Welcome to Rustic Isles!";
    json!({
        "show-status": true,
        "en": welcome,
        "fr": welcome,
        "it": welcome,
        "es": welcome,
        "de": welcome,
        "pt": welcome,
        "ru": welcome,
        "ja": welcome,
    })
}

pub fn register(handlers: &mut crate::http::Handlers) {
    use crate::routes::{Method, Service};
    let b = Service::Backend;
    handlers.on(b, Method::Get, "/", |_| {
        Box::pin(async { util::text(200, "text/html; charset=utf-8", "ok") })
    });
    handlers.on(b, Method::Get, "/health/live", |call| {
        Box::pin(async move { live(&call) })
    });
    handlers.on(b, Method::Get, "/health/ready", |call| {
        Box::pin(ready(call.shared.clone()))
    });
    handlers.on(b, Method::Get, "/health/release-gates", |call| {
        Box::pin(release_gates(call))
    });
    handlers.on(b, Method::Get, "/internal/v1/metrics", |call| {
        Box::pin(metrics_route(call))
    });
    handlers.on(b, Method::Get, "/maintenance/status", |_| {
        Box::pin(async { util::json(200, &json!({ "show-status": false })) })
    });
    handlers.on(b, Method::Get, "/dauntless-status", |_| {
        Box::pin(async { util::json(200, &dauntless_status()) })
    });
}
