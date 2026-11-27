// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Rustic Isles' launcher API (RD27, written clean-room from `docs/port/spec-rustic-isles-reforged-public.md` and
//! `-admin.md`): the probes, metrics and request log, the launcher-facing API, the update feeds and signed Guard
//! manifests, launcher logs, notices, the operations policy, the activity monitor, Launcher Guard and auth's seams
//! (the public half); the admin API, its sign-ins and players, tester max, the release gates, the host-only routes,
//! the module's admin actions and home operations, and the administrator bootstrap (the admin half).
//!
//! The composer builds one [`LauncherApiState`] and one [`admin::LauncherApiAdmin`] per module and puts them in the slots
//! with the store, an [`activity::ActivityHooks`] and the [`LauncherApiSeams`] it hands auth; passes a
//! [`RequestObserver`] over the state as core's observer; calls [`register_public`] and [`register_admin`]; and adds
//! [`public_startups`] and [`admin::admin_startups`].

#![cfg_attr(
    test,
    allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")
)]
#![allow(
    clippy::result_large_err,
    reason = "a refusal is the answer itself (an axum `Response`), as in core and the other areas"
)]

pub mod activity;
pub mod admin;
pub mod guard;
pub mod guard_manifest;
mod jobs;
pub mod launcher_api;
pub mod launcher_auth;
pub mod legacy_claims;
pub mod logs;
pub mod metrics;
pub mod notices;
pub mod oauth;
pub mod observe;
pub mod operations;
pub mod playtime;
pub mod probes;
pub mod queue;
mod seams;
pub mod state;
#[cfg(feature = "test-support")]
pub mod test_support;
pub mod updates;
pub mod util;

use rustic_isles::{Handlers, Method, Service};

pub use admin::{AdminAuth, admin_route, register_admin};
pub use jobs::{log_purge_on, public_startups, run_log_purge, run_ttl_purge};
pub use launcher_auth::{LauncherCaller, launcher_sign_in};
pub use metrics::{Metrics, Section};
pub use observe::{RequestObserver, metric_class, sanitize_url_for_log};
pub use seams::LauncherApiSeams;
pub use state::{LauncherApiState, MatchmakingCandidates, MatchmakingPresence};

use Method::{Delete, Get, Patch, Post, Put};

/// The public half's 47 rows (§1.2), the OAuth sign-in's two and the legacy claim, at the route table's exact paths.
pub fn register_public(handlers: &mut Handlers) {
    let b = Service::Backend;
    let admin = |h: &mut Handlers, method: Method, path: &'static str, run: AdminHandler| {
        h.on(b, method, path, move |call| admin_route(call, AdminAuth::Required, run));
    };
    handlers
        .on(b, Get, "/", probes::root)
        .on(b, Get, "/health/live", probes::live)
        .on(b, Get, "/health/ready", probes::ready)
        .on(b, Get, "/internal/v1/metrics", probes::metrics)
        .on(b, Get, "/dauntless-status", probes::dauntless_status)
        .on(b, Get, "/maintenance/status", probes::maintenance)
        .on(b, Post, "/dainty/api/Register", launcher_api::register)
        .on(b, Get, "/dainty/api/GetUserInfo", launcher_api::get_user_info)
        .on(b, Get, "/dainty/api/ServerStatus", launcher_api::server_status)
        .on(b, Post, "/dainty/api/GameTicket", launcher_api::game_ticket)
        .on(b, Post, "/dainty/api/OAuthStart", oauth::start)
        .on(b, Post, "/dainty/api/OAuthFinish", oauth::finish)
        .on(b, Post, "/dainty/api/LegacyClaim", legacy_claims::redeem)
        .on(
            b,
            Get,
            "/dainty/api/RegistrationStatus",
            launcher_api::registration_status,
        )
        .on(b, Get, "/dainty/api/Playtime", playtime::get_playtime)
        .on(b, Post, "/dainty/api/LoginQueue", queue::join)
        .on(b, Delete, "/dainty/api/LoginQueue", queue::leave)
        .on(
            b,
            Get,
            "/launcher/v1/runtime/:target/:channel/:platform",
            updates::runtime_by_target,
        )
        .on(
            b,
            Get,
            "/launcher/v1/runtime/:channel/:platform",
            updates::runtime_client,
        )
        .on(
            b,
            Get,
            "/launcher/v1/runtime/:target/:channel/:platform/download",
            updates::download_by_target,
        )
        .on(
            b,
            Get,
            "/launcher/v1/runtime/:channel/:platform/download",
            updates::download_client,
        )
        .on(
            b,
            Get,
            "/launcher/v1/guard-manifests/:channel/:platform",
            updates::guard_manifest_route,
        )
        .on(
            b,
            Post,
            "/launcher/v1/admin/guard-manifests/:channel/:platform",
            updates::publish_guard,
        )
        .on(
            b,
            Post,
            "/launcher/v1/admin/updates/runtime/:target/:channel/:platform",
            updates::publish_runtime,
        )
        .on(
            b,
            Post,
            "/launcher/v1/admin/updates/runtime/:target/:channel/:platform/extra",
            updates::publish_extra,
        )
        .on(
            b,
            Get,
            "/launcher/v1/runtime/:target/:channel/:platform/extra/:filename",
            updates::extra_download,
        )
        .on(b, Get, "/launcher/v1/admin/updates/serving", updates::serving_get)
        .on(b, Post, "/launcher/v1/admin/updates/serving", updates::serving_post)
        .on(
            b,
            Get,
            "/launcher/v1/updates/:target/:arch/download",
            updates::launcher_download,
        )
        .on(
            b,
            Get,
            "/launcher/v1/updates/:target/:arch/files/:version/:file",
            updates::launcher_file,
        )
        .on(
            b,
            Get,
            "/launcher/v1/updates/:target/:arch/:currentVersion",
            updates::launcher_update,
        )
        .on(
            b,
            Put,
            "/launcher/v1/logs/sessions/:launchSessionId/:fileName",
            logs::upload,
        )
        .on(b, Get, "/admin/v1/launcher-logs", logs::sync_list)
        .on(
            b,
            Get,
            "/admin/v1/launcher-logs/:userId/:launchSessionId/:fileName",
            logs::sync_file,
        )
        .on(b, Post, "/launcher/v1/guard/sessions", guard::start)
        .on(b, Post, "/launcher/v1/guard/heartbeat", guard::heartbeat)
        .on(b, Get, "/launcher/v1/guard/sessions/:guardSessionId", guard::status);
    admin(handlers, Get, "/admin/v1/operations/overview", |c| {
        Box::pin(operations::overview(c))
    });
    admin(handlers, Patch, "/admin/v1/operations/policy", |c| {
        Box::pin(operations::update_policy(c))
    });
    admin(handlers, Get, "/admin/v1/launcher-notices", |c| {
        Box::pin(notices::admin_read(c))
    });
    admin(handlers, Put, "/admin/v1/launcher-notices", |c| {
        Box::pin(notices::admin_save(c))
    });
    admin(handlers, Get, "/admin/v1/guard/sessions", |c| {
        Box::pin(guard::sessions(c))
    });
    admin(handlers, Get, "/admin/v1/guard/sessions/:guardSessionId", |c| {
        Box::pin(guard::session(c))
    });
    admin(handlers, Get, "/admin/v1/guard/findings", |c| {
        Box::pin(guard::findings(c))
    });
    admin(handlers, Get, "/admin/v1/activity/players/:userId", |c| {
        Box::pin(activity::player(c))
    });
    admin(handlers, Get, "/admin/v1/activity/overview", |c| {
        Box::pin(activity::overview(c))
    });
    admin(handlers, Get, "/admin/v1/activity/flags", |c| {
        Box::pin(activity::flags(c))
    });
    admin(handlers, Patch, "/admin/v1/activity/flags/:id", |c| {
        Box::pin(activity::acknowledge(c))
    });
    admin(handlers, Get, "/admin/v1/player-logs", |c| {
        Box::pin(logs::list_player_logs(c))
    });
    admin(
        handlers,
        Get,
        "/admin/v1/player-logs/:userId/:launchSessionId/:fileName",
        |c| Box::pin(logs::read_player_log(c)),
    );
    admin(
        handlers,
        Delete,
        "/admin/v1/player-logs/:userId/:launchSessionId",
        |c| Box::pin(logs::delete_player_logs(c)),
    );
}

/// An admin-gated handler as the registry keeps it.
pub type AdminHandler = fn(admin::AdminContext) -> dainty_module_api::BoxFuture<'static, axum::response::Response>;
