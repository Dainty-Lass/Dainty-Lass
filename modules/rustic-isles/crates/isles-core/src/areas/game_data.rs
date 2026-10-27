//! RU5's `GameData` group: Game data and scripts. RU3c: hooks `progression_config`, `survey_config`; tables
//! `ACTIVE_HUNTPASS_ID`, `RECENT_HUNTPASS_SEASONS`, `PATCHNOTES_RELEASE_VERSION`, `BOUNTY_ITEM_GRANT_DATA`,
//! `GAME_DATA`.
//!
//! The stubs and static answers of `routes/system.ts` and `routes/tuning.ts` (DD6, DD8): the payloads are in
//! [`crate::game_data`], the handlers here only gate, read the version's hook or table and write the reply.

use axum::body::to_bytes;
use axum::extract::{FromRequestParts, Request};
use axum::http::header::{AUTHORIZATION, CONTENT_ENCODING, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use module_api::BoxFuture;
use serde_json::{Map, Value, json};
use service_kit::body::{BodyError, DEFAULT_LIMIT, ExpressBody, error_page, parse_form, parse_json};
use service_kit::js;
use service_kit::json::Json;

use crate::game_data::gauntlet::{self, envelope};
use crate::game_data::progression_config::served;
use crate::game_data::season_window::now_ms;
use crate::game_data::{patchnotes, tuning};
use crate::http::{Call, Handlers};
use crate::routes::{Method, RouteSet, Service};

const GAMESERVER_KEY_HEADER: &str = "x-mysticparadox-gameserver-apikey";

fn json_reply(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

fn ok(body: Value) -> Response {
    json_reply(StatusCode::OK, body)
}

/// `res.type("text/plain").send(text)`.
fn text(body: &'static str) -> Response {
    (
        [(CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"))],
        body,
    )
        .into_response()
}

/// `HasParadoxBackendAuth`'s refusal for a request with no credential at all: 401 and an empty body. Whether a
/// credential is valid (the JWT, the game server key, the launcher account's eligibility, the `INVALID` sentinel) is
/// RU5 auth's; until its middleware runs ahead of the areas, a request that names a credential passes here. Every
/// route behind this gate answers static, account-free data.
fn unauthenticated(call: &Call) -> Option<Response> {
    let headers = call.request.headers();
    let bearer = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| ["bearer ", "Bearer ", "BEARER "].iter().any(|p| value.starts_with(p)));
    if bearer || headers.contains_key(GAMESERVER_KEY_HEADER) {
        return None;
    }
    tracing::error!(
        "Unauthenticated {} to {} which needs Mystic Paradox Metagame auth!",
        call.request.method(),
        call.request.uri().path()
    );
    Some(StatusCode::UNAUTHORIZED.into_response())
}

fn open(run: fn(Call) -> Response) -> impl Fn(Call) -> BoxFuture<'static, Response> + Send + Sync + 'static {
    move |call| Box::pin(async move { run(call) })
}

fn gated(run: fn(Call) -> Response) -> impl Fn(Call) -> BoxFuture<'static, Response> + Send + Sync + 'static {
    move |call| {
        Box::pin(async move {
            match unauthenticated(&call) {
                Some(refusal) => refusal,
                None => run(call),
            }
        })
    }
}

/// `req.body` as Express's parsers leave it (`app.use(express.json())`): what the pipeline's parser stored, else the
/// body parsed here for the same content types, until that parser runs ahead of the areas.
async fn request_body(request: Request) -> Result<Option<Value>, BodyError> {
    let (mut parts, body) = request.into_parts();
    let Ok(ExpressBody(parsed)) = ExpressBody::from_request_parts(&mut parts, &()).await;
    if parsed.is_some() {
        return Ok(parsed);
    }
    let content_type = parts
        .headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let essence = content_type.split(';').next().unwrap_or_default().trim().to_owned();
    let json_body = match essence.as_str() {
        "application/json" => true,
        "application/x-www-form-urlencoded" => false,
        _ => return Ok(None),
    };
    let charset_ok = content_type
        .split(';')
        .skip(1)
        .filter_map(|part| part.split_once('='))
        .filter(|(name, _)| name.trim() == "charset")
        .all(|(_, value)| value.trim().trim_matches('"') == "utf-8");
    if !charset_ok {
        return Err(BodyError::UnsupportedCharset);
    }
    let encoded = parts
        .headers
        .get(CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| !value.trim().eq_ignore_ascii_case("identity"));
    if encoded {
        return Err(BodyError::UnsupportedEncoding);
    }
    let bytes = to_bytes(body, DEFAULT_LIMIT).await.map_err(|_| BodyError::TooLarge)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| BodyError::Malformed)?;
    let parsed = if json_body { parse_json(text) } else { parse_form(text) };
    parsed.map(Some)
}

/// `body.field ?? default`.
fn field(body: &Map<String, Value>, name: &str, default: Value) -> Value {
    match body.get(name) {
        None | Some(Value::Null) => default,
        Some(value) => value.clone(),
    }
}

fn heartbeat(_: Call) -> Response {
    text("20000")
}

fn event(_: Call) -> Response {
    ok(json!({}))
}

fn crash_ping(_: Call) -> Response {
    tracing::debug!("Crash reporter ping (stubbed)");
    text("OK")
}

fn crash_check_report(_: Call) -> Response {
    tracing::debug!("Crash reporter check report (stubbed)");
    ok(json!({}))
}

/// RU3c `survey_config`: 1.12.0 answers 400 with no body, 1.14.7 the empty trigger list.
fn survey_config(call: Call) -> Response {
    tracing::debug!("Survey config (stubbed)");
    let reply = call.shared.version.hooks.survey_config();
    let status = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    match reply.body {
        Some(body) => json_reply(status, body),
        None => status.into_response(),
    }
}

fn account_migrate(_: Call) -> Response {
    tracing::info!("Account migration (stubbed)");
    ok(json!({ "migration_failed": false, "migration_finished": true }))
}

fn profile_update(_: Call) -> Response {
    tracing::info!("Leaderboard update profile (stubbed)");
    StatusCode::OK.into_response()
}

async fn trials_leaderboards_all(call: Call) -> Response {
    if let Some(refusal) = unauthenticated(&call) {
        return refusal;
    }
    let body = match request_body(call.request).await {
        Ok(body) => body,
        Err(error) => return error.into_response(),
    };
    let empty = Map::new();
    let body = match &body {
        Some(Value::Object(fields)) => fields,
        _ => &empty,
    };
    let difficulty = field(body, "difficulty", json!(1));
    let trial_id = field(body, "trial_id", json!(""));
    tracing::info!(
        "Trials leaderboard all (stubbed empty) trial_id={:?} difficulty={}",
        js::string(Some(&trial_id)),
        js::string(Some(&difficulty))
    );
    ok(envelope(json!({
        "difficulty": difficulty,
        "guild": {},
        "page": field(body, "page", json!(0)),
        "page_size": field(body, "page_size", json!(100)),
        "trial_id": trial_id,
        "world": {
            "group": { "difficulty": difficulty, "entries": [] },
            "solo": { "all": { "difficulty": difficulty, "entries": [] } }
        }
    })))
}

fn trials_leaderboards_solo_individual(_: Call) -> Response {
    tracing::info!("Trials leaderboard solo/individual (stubbed - no persisted run)");
    ok(envelope(json!({})))
}

fn motd(_: Call) -> Response {
    tracing::info!("MOTD (stubbed)");
    StatusCode::NO_CONTENT.into_response()
}

/// 1.14.7 only. A 200 opened an empty message-of-the-day popup in the client (2026-10-15); no MOTD is stored.
fn motd_trigger(_: Call) -> Response {
    tracing::debug!("MOTD trigger: no message of the day (404)");
    StatusCode::NOT_FOUND.into_response()
}

fn player_treatments(call: Call) -> Response {
    tracing::debug!("Cohorts (stubbed)");
    // 1.14.7: COHORT_TREATMENT_LEGACY=1 (the default) keeps the 1.12 treatment; 1.12.0 always serves it
    let legacy = call.shared.version.routes == RouteSet::V1120
        || call.shared.settings.raw("COHORT_TREATMENT_LEGACY").unwrap_or("1") == "1";
    let treatments: Vec<&str> = if legacy { vec!["CohortTreatment.Dojo.B"] } else { vec![] };
    ok(json!({ "treatments": treatments }))
}

fn eventstats(_: Call) -> Response {
    tracing::debug!("Event stats (stubbed)");
    ok(json!({ "stats": [] }))
}

/// RU3c `progression_config`: the file with its Hunt Pass windows rewritten (1.14.7), or as it is (1.12.0).
fn progression_config(call: Call) -> Response {
    tracing::info!("Progression Config (stubbed)");
    let shared = &call.shared;
    // Node reads the file when the module loads and refuses to start without it
    let file = match shared.game_data.load_backend_cached("progression_config.json") {
        Ok(file) => file,
        Err(error) => {
            tracing::error!("GET /progression/config: {error}");
            return error_page(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    let rule = shared.version.hooks.progression_config();
    ok(served(rule, &file, &shared.version.tables, &shared.settings, now_ms()))
}

/// `GET /config` on the Gauntlet host (the dispatcher passes other origins on, as Node's `next()` does).
fn gauntlet_config(call: Call) -> Response {
    tracing::info!(
        "Gauntlet config requested by {} (1.14.7 stub)",
        call.origin_host.as_deref().unwrap_or_default()
    );
    let probe = call.shared.settings.raw("GAUNTLET_CONFIG_PROBE").unwrap_or("1") == "1";
    ok(envelope(if probe {
        gauntlet::config_payload(now_ms())
    } else {
        json!({})
    }))
}

fn gauntlet_progression(call: Call) -> Response {
    let season = call.params.get("0").unwrap_or_default();
    tracing::info!(
        "Gauntlet progression requested season={season:?} user={:?} (1.14.7 stub)",
        call.params.get("1").unwrap_or_default()
    );
    ok(envelope(gauntlet::progression_payload(season)))
}

fn gauntlet_rewards_personal(call: Call) -> Response {
    tracing::info!(
        "Gauntlet personal rewards requested season={:?} user={:?} (1.14.7 stub)",
        call.params.get("0").unwrap_or_default(),
        call.params.get("1").unwrap_or_default()
    );
    ok(envelope(json!({})))
}

fn gauntlet_rewards_guild(call: Call) -> Response {
    tracing::info!(
        "Gauntlet guild rewards requested season={:?} user={:?} (1.14.7 stub)",
        call.params.get("0").unwrap_or_default(),
        call.params.get("1").unwrap_or_default()
    );
    ok(envelope(gauntlet::guild_rewards_payload()))
}

fn gauntlet_leaderboard(_: Call) -> Response {
    tracing::info!("Gauntlet leaderboard requested (1.14.7 stub)");
    ok(envelope(gauntlet::leaderboard_payload()))
}

fn mailbox(_: Call) -> Response {
    tracing::info!("Mailbox (stubbed)");
    ok(envelope(json!({ "messages": [] })))
}

/// Not gated: the login screen asks before there is a token. `release_version` is RU3c's
/// `PATCHNOTES_RELEASE_VERSION`.
fn patch_notes(call: Call) -> Response {
    let language = call.params.get("language").unwrap_or("en");
    tracing::info!(
        "Patch notes {language:?}/{:?}",
        call.params.get("gameversion").unwrap_or_default()
    );
    ok(patchnotes::payload(
        language,
        call.shared.version.tables.patchnotes_release_version,
    ))
}

fn seasonal_event_schedule(_: Call) -> Response {
    tracing::info!("Seasonal Event Schedule (stubbed)");
    ok(tuning::seasonal_event_schedule())
}

fn huntpass_xp_config(_: Call) -> Response {
    tracing::info!("Huntpass XP Config (stubbed)");
    ok(tuning::huntpass_xp_config())
}

fn island_content_config(_: Call) -> Response {
    tracing::debug!("Island content config (stubbed)");
    ok(tuning::island_content_config())
}

/// RU3c `BOUNTY_ITEM_GRANT_DATA`: the windows 1.14.7 needs to find the bounty season, none on 1.12.0.
fn bounty_game_data(call: Call) -> Response {
    tracing::debug!("Bounty game data (stubbed)");
    ok(tuning::bounty_game_data(
        call.shared.version.tables.bounty_item_grant_data,
    ))
}

fn bounty_game_data_daily(_: Call) -> Response {
    tracing::debug!("Bounty game data daily (stubbed)");
    ok(tuning::bounty_game_data_daily())
}

fn bounty_game_data_weekly(_: Call) -> Response {
    tracing::debug!("Bounty game data weekly (stubbed)");
    ok(tuning::bounty_game_data_weekly())
}

/// Mounts this area's handlers (`handlers.on(Service::Backend, Method::Get, "/path", ...)`, the path exactly as the
/// route table has it); a route with none answers 501.
pub fn register(handlers: &mut Handlers) {
    use Method::{Get, Post};
    let backend = Service::Backend;
    handlers
        .on(backend, Post, "/heartbeat", open(heartbeat))
        .on(backend, Post, "/event", open(event))
        .on(backend, Get, "/crash/CrashReporter/Ping", open(crash_ping))
        .on(
            backend,
            Post,
            "/crash/CrashReporter/CheckReport",
            open(crash_check_report),
        )
        .on(backend, Get, "/survey/config", gated(survey_config))
        .on(backend, Post, "/account/migrate", gated(account_migrate))
        .on(backend, Post, "/profile/update", gated(profile_update))
        .on(backend, Post, "/trials/leaderboards/all", |call| {
            Box::pin(trials_leaderboards_all(call))
        })
        .on(
            backend,
            Post,
            "/trials/leaderboards/solo/individual",
            gated(trials_leaderboards_solo_individual),
        )
        .on(backend, Post, "/motd/", gated(motd))
        .on(backend, Get, "/motd/trigger", gated(motd_trigger))
        .on(backend, Get, "/playertreatments", gated(player_treatments))
        .on(backend, Get, "/playertreatments/", gated(player_treatments))
        .on(backend, Get, "/playertreatments/:userId", gated(player_treatments))
        .on(backend, Get, "/eventstats/", gated(eventstats))
        .on(backend, Get, "/progression/config", gated(progression_config))
        .on(backend, Get, "/config", gated(gauntlet_config))
        .on(
            backend,
            Get,
            r"/^\/progression\/([^/]*)\/([^/]+)$/i",
            gated(gauntlet_progression),
        )
        .on(
            backend,
            Get,
            r"/^\/rewards\/personal\/([^/]*)\/([^/]+)$/i",
            gated(gauntlet_rewards_personal),
        )
        .on(
            backend,
            Get,
            r"/^\/rewards\/guild\/([^/]*)\/([^/]+)$/i",
            gated(gauntlet_rewards_guild),
        )
        .on(
            backend,
            Get,
            r"/^\/leaderboard\/get_leaderboard\//i",
            gated(gauntlet_leaderboard),
        )
        .on(backend, Get, "/all/", gated(mailbox))
        .on(backend, Get, "/patchnotes/:language/:gameversion", open(patch_notes))
        .on(
            backend,
            Get,
            "/game_tuning/seasonal_event_schedule",
            open(seasonal_event_schedule),
        )
        .on(
            backend,
            Get,
            "/game_tuning/huntpass_xp_config",
            open(huntpass_xp_config),
        )
        .on(
            backend,
            Get,
            "/game_tuning/island_content_config",
            open(island_content_config),
        )
        .on(backend, Get, "/game_tuning/bounty_game_data", open(bounty_game_data))
        .on(
            backend,
            Get,
            "/game_tuning/bounty_game_data_daily",
            open(bounty_game_data_daily),
        )
        .on(
            backend,
            Get,
            "/game_tuning/bounty_game_data_weekly",
            open(bounty_game_data_weekly),
        );
}
