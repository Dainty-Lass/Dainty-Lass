// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Rustic Isles' game-data area (RD27, written clean-room from `docs/port/spec-rustic-isles-gamedata.md`): the
//! stubs and static answers the 1.14.7 client asks for at login, in Ramsgate and on the Gauntlet host, the served
//! progression config, and the backend game-data generators `dainty-host gamedata` runs.

#![cfg_attr(
    test,
    allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")
)]

mod areas;
pub mod gauntlet;
pub mod generators;
pub mod patchnotes;
pub mod progression_config;
pub mod tuning;

use rustic_isles::{Handlers, Method};
use serde_json::{Value, json};

pub use generators::{
    BackendStep, GeneratorPaths, RewardCacheCategory, RewardCachePlan, RewardCacheSelection, TokenSource,
    VersionInputs, run,
};

/// The 29 `game-data` rows (spec §3.1), in the table's order.
const ROUTES: [(Method, &str); 31] = [
    (Method::Post, "/heartbeat"),
    (Method::Post, "/event"),
    (Method::Get, "/crash/CrashReporter/Ping"),
    (Method::Post, "/crash/CrashReporter/CheckReport"),
    (Method::Get, "/survey/config"),
    (Method::Post, "/account/migrate"),
    (Method::Post, "/profile/update"),
    (Method::Post, "/trials/leaderboards/all"),
    (Method::Post, "/trials/leaderboards/solo/individual"),
    (Method::Post, "/motd/"),
    (Method::Get, "/motd/trigger"),
    (Method::Get, "/playertreatments"),
    (Method::Get, "/playertreatments/"),
    (Method::Get, "/playertreatments/:userId"),
    (Method::Get, "/eventstats/"),
    (Method::Get, "/progression/config"),
    (Method::Get, "/config"),
    (Method::Get, r"/^\/progression\/([^/]*)\/([^/]+)$/i"),
    (Method::Get, r"/^\/rewards\/personal\/([^/]*)\/([^/]+)$/i"),
    (Method::Get, r"/^\/rewards\/guild\/([^/]*)\/([^/]+)$/i"),
    (Method::Get, r"/^\/leaderboard\/get_leaderboard\//i"),
    (Method::Put, r"/^\/(.*)$/"),
    (Method::Post, r"/^\/(.*)$/"),
    (Method::Get, "/all/"),
    (Method::Get, "/patchnotes/:language/:gameversion"),
    (Method::Get, "/game_tuning/seasonal_event_schedule"),
    (Method::Get, "/game_tuning/huntpass_xp_config"),
    (Method::Get, "/game_tuning/island_content_config"),
    (Method::Get, "/game_tuning/bounty_game_data"),
    (Method::Get, "/game_tuning/bounty_game_data_daily"),
    (Method::Get, "/game_tuning/bounty_game_data_weekly"),
];

/// The (method, path) pairs this crate answers.
pub fn routes() -> &'static [(Method, &'static str)] {
    &ROUTES
}

/// Registers the 29 handlers on the backend.
pub fn register(handlers: &mut Handlers) {
    areas::register(handlers);
}

/// `{"code":null,"message":"OK","payload":<payload>}`
pub fn envelope(payload: Value) -> Value {
    json!({ "code": null, "message": "OK", "payload": payload })
}
