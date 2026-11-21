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

use crate::{envelope, gauntlet, gauntlet_runs, patchnotes, progression_config, trials, tuning};

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
    use Method::{Get, Post, Put};

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
    on(handlers, Post, "/trials/leaderboards/solo/individual", Game, |c| {
        trials_query(c, TrialsBoard::SoloOwn)
    });
    on(handlers, Post, "/trials/leaderboards", Game, trials_submit);
    on(handlers, Post, "/trials/leaderboards/solo", Game, |c| {
        trials_query(c, TrialsBoard::Solo)
    });
    on(handlers, Post, "/trials/leaderboards/group", Game, |c| {
        trials_query(c, TrialsBoard::Group)
    });
    on(handlers, Post, "/trials/leaderboards/group/individual", Game, |c| {
        trials_query(c, TrialsBoard::GroupOwn)
    });
    on(handlers, Post, r"/^\/trials\/(.*)$/", Game, trials_unknown);
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
        |c| async move { json(envelope(gauntlet_progression(c).await)) },
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
        |c| async move {
            let season = c.request.uri().path().rsplit('/').next().unwrap_or("").to_owned();
            json(envelope(gauntlet_runs::leaderboard(&c.shared, &season).await))
        },
    );
    on(handlers, Put, r"/^\/(.*)$/", Game, gauntlet_write);
    on(handlers, Post, r"/^\/(.*)$/", Game, gauntlet_write);
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
    let shared = ctx.shared.clone();
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
    let query = trials::BoardQuery::from_body(&body);
    let boards = trials::boards(&shared, &query.trial_id, query.difficulty).await;
    tracing::info!(
        "Trials leaderboard all trial_id={:?} difficulty={}: {} solo, {} group",
        dainty_service_kit::js::string(Some(&trial_id)),
        dainty_service_kit::js::string(Some(&difficulty)),
        boards.solo.len(),
        boards.group.len()
    );
    // The per-weapon boards only once they hold a line, so an empty answer stays the stub's
    let mut solo = Map::new();
    solo.insert(
        "all".into(),
        json!({ "difficulty": difficulty, "entries": boards.solo_page(&query, None) }),
    );
    for (name, weapon) in trials::WEAPON_BOARDS {
        let entries = boards.solo_page(&query, Some(weapon));
        if !entries.is_empty() {
            solo.insert(name.into(), json!({ "difficulty": difficulty, "entries": entries }));
        }
    }
    json(envelope(json!({
        "difficulty": difficulty,
        "guild": {},
        "page": page,
        "page_size": page_size,
        "trial_id": trial_id,
        "world": {
            "group": { "difficulty": difficulty, "entries": boards.group_page(&query) },
            "solo": solo
        }
    })))
}

#[derive(Clone, Copy)]
enum TrialsBoard {
    Solo,
    Group,
    SoloOwn,
    GroupOwn,
}

/// The body of a Trials request, read and logged (credentials dropped) while the client's requests are being learned.
async fn trials_body(request: Request) -> Result<(http::request::Parts, Map<String, Value>), Box<Response>> {
    let target = request
        .uri()
        .path_and_query()
        .map_or_else(String::new, |p| p.as_str().to_owned());
    let (parts, body) = request.into_parts();
    let body = read_body_parts_limit(&parts, body, BODY_LIMIT)
        .await
        .map_err(Box::new)?;
    let shown = body.as_ref().map_or_else(
        || "(none)".to_owned(),
        |b| dainty_service_kit::jsjson::stringify(&gauntlet::redacted(b.clone())),
    );
    tracing::info!("[Trials] {} {target} body={shown}", parts.method);
    let map = match body {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    Ok((parts, map))
}

/// A board page (`/solo`, `/group`), or the caller's own line with its rank (`/solo/individual`,
/// `/group/individual`; `{}` before their first run).
async fn trials_query(ctx: Ctx, board: TrialsBoard) -> Response {
    let shared = ctx.shared.clone();
    let (parts, body) = match trials_body(ctx.request).await {
        Ok(read) => read,
        Err(page) => return *page,
    };
    let query = trials::BoardQuery::from_body(&body);
    let boards = trials::boards(&shared, &query.trial_id, query.difficulty).await;
    let caller = parts
        .extensions
        .get::<rustic_isles_auth::AuthData>()
        .and_then(|a| a.user_id.clone())
        .unwrap_or_default();
    let page = |entries: Vec<Value>| {
        json!({
            "difficulty": query.difficulty,
            "page": query.page,
            "page_size": query.page_size,
            "trial_id": query.trial_id,
            "entries": entries,
        })
    };
    let payload = match board {
        TrialsBoard::Solo => page(boards.solo_page(&query, None)),
        TrialsBoard::Group => page(boards.group_page(&query)),
        TrialsBoard::SoloOwn => boards.solo_of(&caller).unwrap_or_else(|| json!({})),
        TrialsBoard::GroupOwn => boards.group_of(&caller).unwrap_or_else(|| json!({})),
    };
    json(envelope(payload))
}

/// `POST /trials/leaderboards`: the hunt server's finished Trial (first Trials test, 2026-11-19). Only a game server
/// records one; a player's own token is an empty 403.
async fn trials_submit(ctx: Ctx) -> Response {
    let shared = ctx.shared.clone();
    let server = ctx
        .request
        .extensions()
        .get::<rustic_isles_auth::AuthData>()
        .is_some_and(|a| a.is_gameserver || a.runtime.is_some());
    if !server {
        tracing::warn!("[Trials] POST /trials/leaderboards refused: the caller is not a game server");
        return StatusCode::FORBIDDEN.into_response();
    }
    let body = match trials_body(ctx.request).await {
        Ok((_, body)) => Value::Object(body),
        Err(page) => return *page,
    };
    match trials::FinishedTrial::parse(&body) {
        Some(run) => trials::record(&shared, &run).await,
        None => tracing::warn!("[Trials] POST /trials/leaderboards with a body it could not read"),
    }
    json(envelope(json!({})))
}

/// Any other Trials write: logged with its body and answered with an empty envelope, until its shape is known.
async fn trials_unknown(ctx: Ctx) -> Response {
    match trials_body(ctx.request).await {
        Ok(_) => json(envelope(json!({}))),
        Err(page) => *page,
    }
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

/// A write to the Gauntlet host: logged with its body (credentials dropped) and answered with an empty envelope, until
/// its shape is known and stored (first Gauntlet test, 2026-11-19).
async fn gauntlet_write(ctx: Ctx) -> Response {
    let shared = ctx.shared.clone();
    let method = ctx.request.method().clone();
    let target = ctx
        .request
        .uri()
        .path_and_query()
        .map_or_else(String::new, |p| p.as_str().to_owned());
    let (parts, body) = ctx.request.into_parts();
    let body = match read_body_parts_limit(&parts, body, BODY_LIMIT).await {
        Ok(body) => body,
        Err(page) => return page,
    };
    // The hunt server's level end (first Gauntlet test, 2026-11-19); a player's own token may not record a run
    if method == http::Method::PUT && parts.uri.path() == "/process_run" {
        let server = parts
            .extensions
            .get::<rustic_isles_auth::AuthData>()
            .is_some_and(|a| a.is_gameserver || a.runtime.is_some());
        if !server {
            tracing::warn!("[Gauntlet] PUT /process_run refused: the caller is not a game server");
            return StatusCode::FORBIDDEN.into_response();
        }
        match body.as_ref().and_then(gauntlet_runs::FinishedRun::parse) {
            Some(run) => gauntlet_runs::record(&shared, &run).await,
            None => tracing::warn!("[Gauntlet] PUT /process_run with a body it could not read"),
        }
    }
    let shown = body.map_or_else(
        || "(none)".to_owned(),
        |b| dainty_service_kit::jsjson::stringify(&gauntlet::redacted(b)),
    );
    tracing::info!("[Gauntlet] {method} {target} body={shown}");
    json(envelope(json!({})))
}

/// B48: level access is the level after the player's best, progression their best. The season and the player come
/// from the path, except level access, whose player is `?account_id=`.
async fn gauntlet_progression(ctx: Ctx) -> Value {
    let first = ctx.params.get("0").unwrap_or("").to_owned();
    let second = ctx.params.get("1").unwrap_or("").to_owned();
    let on = gauntlet_on(&ctx);
    let user = query_value(&ctx.request, "account_id");
    let shared = ctx.shared;
    if gauntlet::is_level_access(&first) {
        let best = match (on, user) {
            (true, Some(user)) => gauntlet_runs::best_level(&shared, &second, &user).await,
            _ => 0,
        };
        return gauntlet::progression_payload(&first, gauntlet::level_access(on, best), best);
    }
    let best = gauntlet_runs::best_level(&shared, &first, &second).await;
    gauntlet::progression_payload(&first, 0, best)
}

/// The first `name=` value of the query, percent-decoded.
fn query_value(request: &Request, name: &str) -> Option<String> {
    request.uri().query()?.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| dainty_service_kit::js::decode_uri_component(value).unwrap_or_else(|| value.to_owned()))
    })
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
