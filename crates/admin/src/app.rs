//! The HTTP side: `POST /graphql` for the bot, `GET /health` for `dainty-host start`'s ready probe.
//! Every GraphQL request is checked before its body is read: a loopback peer, no proxy headers, a key that
//! is the owner's or an issued one that is not revoked (`keys::Keys`), and the operator id.

use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::json;
use service_kit::serve::ConnectionInfo;

use crate::input::{self, Refusal};
use crate::keys::{AuthError, Caller, Keys};
use crate::log;
use crate::schema::AdminSchema;

#[derive(Clone)]
pub struct AppState {
    pub schema: AdminSchema,
    pub keys: Keys,
    pub admin: std::sync::Arc<crate::schema::Admin>,
    /// `RUSTIC_ISLES_METRICS_TOKEN`: the backend's own metrics are added to `/metrics` with it
    pub metrics_token: Option<String>,
}

fn json_reply(status: StatusCode, body: serde_json::Value) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, "application/json; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body.to_string(),
    )
        .into_response()
}

fn refuse(refusal: Refusal) -> Response {
    let status = StatusCode::from_u16(refusal.status()).unwrap_or(StatusCode::BAD_REQUEST);
    json_reply(
        status,
        json!({"errors": [{"message": refusal.code().to_lowercase().replace('_', " "), "extensions": {"code": refusal.code()}}]}),
    )
}

/// Whose request this is. A refusal is logged with the key's id when there is one, never the key (DH2);
/// the caller is told only 401, whatever the reason.
async fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<Caller, Refusal> {
    let given = input::bearer(headers).inspect_err(|refusal| {
        log::warn("admin API request refused", json!({"code": refusal.code()}));
    })?;
    let (key, role) = match state.keys.authenticate(given).await {
        Ok(found) => found,
        Err(error) => {
            let (refusal, why, key) = match error {
                AuthError::Unknown => (Refusal::Unauthenticated, "unknown key", None),
                AuthError::Revoked(id) => (Refusal::Unauthenticated, "revoked key", Some(id)),
                AuthError::Unavailable => (Refusal::KeysUnavailable, "keys unavailable", None),
            };
            log::warn(
                "admin API request refused",
                json!({"code": refusal.code(), "why": why, "key": key}),
            );
            return Err(refusal);
        }
    };
    let operator = input::operator(headers).inspect_err(|refusal| {
        log::warn("admin API request refused", json!({"code": refusal.code(), "key": key}));
    })?;
    Ok(Caller { key, role, operator })
}

async fn graphql(
    State(state): State<AppState>,
    ConnectInfo(info): ConnectInfo<ConnectionInfo>,
    request: Request,
) -> Response {
    // The listener is on loopback already; a peer elsewhere means a bind setting went wrong
    if !info.peer.ip().is_loopback() {
        return refuse(Refusal::Forwarded);
    }
    let (parts, body) = request.into_parts();
    let caller = match authenticate(&state, &parts.headers).await {
        Ok(caller) => caller,
        Err(refusal) => return refuse(refusal),
    };
    if !input::is_json(&parts.headers) {
        return refuse(Refusal::NotJson);
    }
    let Ok(bytes) = axum::body::to_bytes(body, input::MAX_BODY).await else {
        return refuse(Refusal::BadBody);
    };
    let request = match input::parse_body(&bytes) {
        Ok(request) => request,
        Err(refusal) => return refuse(refusal),
    };
    let response = state.schema.execute(request.data(caller)).await;
    match serde_json::to_value(&response) {
        Ok(body) => json_reply(StatusCode::OK, body),
        Err(_) => json_reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"errors": [{"message": "internal error"}]}),
        ),
    }
}

/// Every component's metrics in Prometheus' text format, for any admin key: this process's registry (the
/// services, chat, the processes it started), the game servers' players from the Director, and the
/// backend's own `/internal/v1/metrics`.
async fn metrics(
    State(state): State<AppState>,
    ConnectInfo(info): ConnectInfo<ConnectionInfo>,
    headers: HeaderMap,
) -> Response {
    if !info.peer.ip().is_loopback() {
        return refuse(Refusal::Forwarded);
    }
    let given = match input::bearer(&headers) {
        Ok(given) => given,
        Err(refusal) => return refuse(refusal),
    };
    if state.keys.authenticate(given).await.is_err() {
        return refuse(Refusal::Unauthenticated);
    }
    use service_kit::metrics as m;
    const PLAYERS: &str = "dainty_game_server_players";
    const MAX: &str = "dainty_game_server_max_players";
    m::clear(PLAYERS);
    m::clear(MAX);
    match state.admin.upstream.instances().await {
        Ok(instances) => {
            m::gauge("dainty_director_up", "Whether the Director answered", &[], 1.0);
            for i in &instances {
                let kind = match i.kind {
                    input::InstanceKind::City => "city",
                    input::InstanceKind::Hunt => "hunt",
                    input::InstanceKind::Dojo => "dojo",
                    input::InstanceKind::Tutorial => "tutorial",
                };
                let labels = [("instance", i.id.as_str()), ("kind", kind)];
                m::gauge(
                    PLAYERS,
                    "Players on a running game server",
                    &labels,
                    f64::from(i.players),
                );
                m::gauge(MAX, "A game server's player cap", &labels, f64::from(i.max_players));
            }
        }
        Err(_) => m::gauge("dainty_director_up", "Whether the Director answered", &[], 0.0),
    }
    let mut text = m::render();
    if let Some(token) = &state.metrics_token {
        let up = match state.admin.upstream.backend_metrics(token).await {
            Ok(body) => {
                text.push_str(&String::from_utf8_lossy(&body));
                1
            }
            Err(_) => 0,
        };
        text.push_str(&format!(
            "# HELP dainty_backend_metrics_up Whether the backend's metrics answered\n\
             # TYPE dainty_backend_metrics_up gauge\ndainty_backend_metrics_up {up}\n"
        ));
    }
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        text,
    )
        .into_response()
}

async fn health() -> Response {
    json_reply(StatusCode::OK, json!({"ok": true}))
}

async fn not_found() -> Response {
    (StatusCode::NOT_FOUND, Body::empty()).into_response()
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/graphql", post(graphql))
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .fallback(not_found)
        .layer(axum::middleware::from_fn_with_state(
            "admin",
            service_kit::metrics::track,
        ))
        .with_state(state)
}
