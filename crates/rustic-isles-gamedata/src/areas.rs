// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The 29 handlers (spec §3). A "game" row runs game auth first, so a refusal is answered before anything else.

use std::future::Future;
use std::sync::Arc;

use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use dainty_service_kit::json::Json;
use http::{StatusCode, header};
use rustic_isles::{BODY_LIMIT, Call, Handlers, Method, Params, Service, Shared, read_body_parts_limit, replay};
use serde_json::{Map, Value, json};

use crate::{envelope, gauntlet, patchnotes, progression_config, tuning};

/// What a handler gets once its gate passed.
struct Ctx {
    shared: Arc<Shared>,
    params: Params,
    request: Request,
    origin_host: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Gate {
    Game,
    Open,
    /// Game auth, except the Gauntlet's level-access query: the 1.14.7 client sends it with no bearer (first local
    /// test, 2026-11-10) and its answer names no account.
    GameUnlessLevelAccess,
}

fn on<F, Fut>(handlers: &mut Handlers, method: Method, path: &str, gate: Gate, answer: F)
where
    F: Fn(Ctx) -> Fut + Send + Sync + Copy + 'static,
    Fut: Future<Output = Response> + Send + 'static,
{
    handlers.on(Service::Backend, method, path, move |call: Call| async move {
        let Call {
            shared,
            params,
            mut request,
            origin_host,
            ..
        } = call;
        let level_access = params.get("0").is_some_and(|s| s.to_lowercase() == "level_access");
        let gated = gate == Gate::Game || (gate == Gate::GameUnlessLevelAccess && !level_access);
        if gated && let Err(refusal) = shared.auth().game_auth(&mut request).await {
            return refusal;
        }
        answer(Ctx {
            shared,
            params,
            request,
            origin_host,
        })
        .await
    });
}

fn json(value: Value) -> Response {
    Json(value).into_response()
}

fn text(body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body).into_response()
}

pub(crate) fn register(handlers: &mut Handlers) {
    use Gate::{Game, GameUnlessLevelAccess, Open};
    use Method::{Get, Post};

    on(handlers, Post, "/heartbeat", Open, |_| async { text("20000") });
    on(handlers, Post, "/event", Open, |_| async { json(json!({})) });
    on(handlers, Get, "/crash/CrashReporter/Ping", Open, |_| async {
        text("OK")
    });
    on(handlers, Post, "/crash/CrashReporter/CheckReport", Open, |_| async {
        json(json!({}))
    });
    on(
        handlers,
        Get,
        "/survey/config",
        Game,
        |c| async move { survey(&c.shared) },
    );
    on(handlers, Post, "/account/migrate", Game, |_| async {
        json(json!({"migration_failed": false, "migration_finished": true}))
    });
    on(handlers, Post, "/profile/update", Game, |_| async {
        StatusCode::OK.into_response()
    });
    on(handlers, Post, "/trials/leaderboards/all", Game, trials_all);
    on(
        handlers,
        Post,
        "/trials/leaderboards/solo/individual",
        Game,
        |_| async { json(envelope(json!({}))) },
    );
    on(handlers, Post, "/motd/", Game, |_| async {
        StatusCode::NO_CONTENT.into_response()
    });
    // A 200 opened an empty popup in the client (2026-10-15)
    on(handlers, Get, "/motd/trigger", Game, |_| async {
        StatusCode::NOT_FOUND.into_response()
    });
    for path in ["/playertreatments", "/playertreatments/", "/playertreatments/:userId"] {
        on(handlers, Get, path, Game, |c| async move { treatments(&c.shared) });
    }
    on(handlers, Get, "/eventstats/", Game, |_| async {
        json(json!({"stats": []}))
    });
    on(handlers, Get, "/progression/config", Game, |c| async move {
        progression(&c.shared)
    });
    on(handlers, Get, "/config", Game, |c| async move { gauntlet_config(&c) });
    on(
        handlers,
        Get,
        r"/^\/progression\/([^/]*)\/([^/]+)$/i",
        GameUnlessLevelAccess,
        |c| async move {
            let access = if gauntlet_on(&c) { gauntlet::LEVEL_ACCESS_ON } else { 0 };
            json(envelope(gauntlet::progression_payload(
                c.params.get("0").unwrap_or(""),
                access,
            )))
        },
    );
    on(
        handlers,
        Get,
        r"/^\/rewards\/personal\/([^/]*)\/([^/]+)$/i",
        Game,
        |_| async { json(envelope(gauntlet::personal_rewards_payload())) },
    );
    on(
        handlers,
        Get,
        r"/^\/rewards\/guild\/([^/]*)\/([^/]+)$/i",
        Game,
        |_| async { json(envelope(gauntlet::guild_rewards_payload())) },
    );
    on(
        handlers,
        Get,
        r"/^\/leaderboard\/get_leaderboard\//i",
        Game,
        |_| async { json(envelope(gauntlet::leaderboard_payload())) },
    );
    on(handlers, Get, "/all/", Game, |_| async {
        json(envelope(json!({"messages": []})))
    });
    // Open: the login screen asks before there is a token
    on(
        handlers,
        Get,
        "/patchnotes/:language/:gameversion",
        Open,
        |c| async move {
            let language = c.params.get("language").unwrap_or("");
            let version = c.params.get("gameversion").unwrap_or("");
            tracing::info!("Patch notes language={language:?} gameversion={version:?}");
            let release = c.shared.version().tables.patchnotes_release_version;
            json(patchnotes::payload(language, release))
        },
    );
    on(handlers, Get, "/game_tuning/seasonal_event_schedule", Open, |_| async {
        json(tuning::seasonal_event_schedule())
    });
    on(handlers, Get, "/game_tuning/huntpass_xp_config", Open, |_| async {
        json(tuning::huntpass_xp_config())
    });
    on(handlers, Get, "/game_tuning/island_content_config", Open, |_| async {
        json(tuning::island_content_config())
    });
    on(handlers, Get, "/game_tuning/bounty_game_data", Open, |c| async move {
        json(tuning::bounty_game_data(
            c.shared.version().tables.bounty_item_grant_data,
        ))
    });
    on(handlers, Get, "/game_tuning/bounty_game_data_daily", Open, |_| async {
        json(tuning::bounty_game_data_daily())
    });
    on(handlers, Get, "/game_tuning/bounty_game_data_weekly", Open, |_| async {
        json(tuning::bounty_game_data_weekly())
    });
}

fn survey(shared: &Shared) -> Response {
    let reply = shared.version().hooks.survey_config();
    // Express's res.status throws on a code outside 100-999, which its handler answers as a 500 page
    let Some(status) = (100..=999)
        .contains(&reply.status)
        .then(|| StatusCode::from_u16(reply.status).ok())
        .flatten()
    else {
        return dainty_service_kit::body::error_page(StatusCode::INTERNAL_SERVER_ERROR);
    };
    match reply.body {
        Some(body) => (status, Json(body)).into_response(),
        None => status.into_response(),
    }
}

fn treatments(shared: &Shared) -> Response {
    let legacy = shared
        .settings()
        .raw("COHORT_TREATMENT_LEGACY")
        .is_none_or(|v| v == "1");
    let list = if legacy {
        json!(["CohortTreatment.Dojo.B"])
    } else {
        json!([])
    };
    json(json!({ "treatments": list }))
}

async fn trials_all(ctx: Ctx) -> Response {
    let (parts, body) = ctx.request.into_parts();
    let body = match read_body_parts_limit(&parts, body, BODY_LIMIT).await {
        Ok(body) => body,
        Err(page) => return page,
    };
    let body = match body {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    // `body.field ?? default`
    let field = |name: &str, default: Value| match body.get(name) {
        None | Some(Value::Null) => default,
        Some(v) => v.clone(),
    };
    let difficulty = field("difficulty", json!(1));
    let page = field("page", json!(0));
    let page_size = field("page_size", json!(100));
    let trial_id = field("trial_id", json!(""));
    tracing::info!(
        "Trials leaderboard all (stubbed empty) trial_id={:?} difficulty={}",
        dainty_service_kit::js::string(Some(&trial_id)),
        dainty_service_kit::js::string(Some(&difficulty))
    );
    json(envelope(json!({
        "difficulty": difficulty,
        "guild": {},
        "page": page,
        "page_size": page_size,
        "trial_id": trial_id,
        "world": {
            "group": { "difficulty": difficulty, "entries": [] },
            "solo": { "all": { "difficulty": difficulty, "entries": [] } }
        }
    })))
}

fn progression(shared: &Shared) -> Response {
    let file = match shared.game_data().load_backend_cached("progression_config.json") {
        Ok(file) => file,
        Err(error) => {
            tracing::error!("GET /progression/config: {error}");
            return dainty_service_kit::body::error_page(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    let version = shared.version();
    let served = progression_config::served(
        version.hooks.progression_config(),
        &file,
        version.tables,
        shared.settings(),
        replay::now_ms(),
    );
    json(served)
}

/// `RUSTIC_ISLES_GAUNTLET` reads `on`, trimmed, any case (matchmaking's switch).
fn gauntlet_on(ctx: &Ctx) -> bool {
    ctx.shared
        .settings()
        .raw("RUSTIC_ISLES_GAUNTLET")
        .is_some_and(|v| dainty_service_kit::js::trim(v).eq_ignore_ascii_case("on"))
}

fn gauntlet_config(ctx: &Ctx) -> Response {
    let host = ctx.origin_host.as_deref().unwrap_or("");
    let probe = ctx
        .shared
        .settings()
        .raw("GAUNTLET_CONFIG_PROBE")
        .is_none_or(|v| v == "1");
    tracing::info!(
        "Gauntlet config requested from {host} (probe {})",
        if probe { "on" } else { "off" }
    );
    if probe {
        json(envelope(gauntlet::config_payload(replay::now_ms())))
    } else {
        json(envelope(json!({})))
    }
}
