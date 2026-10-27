//! RU5's `ReforgedApi` group: The Reforged API with health, metrics, runtime and launcher updates and the admin panel,
//! and Launcher Guard (RF1, decision 1). Node-scope routes: served from here whichever module is loaded, with its
//! `TARGET_CHANGELIST` (RU3c, seam 7).
// A handler refuses with the answer itself, as Express's middleware does
#![allow(clippy::result_large_err)]
// `segment` and the feed locators fail with Node's one "Invalid ..." and nothing to say beyond it
#![allow(clippy::result_unit_err)]

pub mod admin;
pub mod admin_guard;
pub mod guard;
pub mod guard_manifest;
pub mod host_routes;
pub mod http_get;
pub mod launcher_api;
pub mod launcher_auth;
pub mod logs;
pub mod metrics;
pub mod notices;
pub mod observe;
pub mod player_logs;
pub mod presence;
pub mod probes;
pub mod release_gates;
pub mod rules;
pub mod updates;
pub mod util;

use crate::http::Handlers;

/// Mounts this area's handlers (`handlers.on(Service::Backend, Method::Get, "/path", ...)`, the path exactly as the
/// route table has it); a route with none answers 501.
pub fn register(handlers: &mut Handlers) {
    probes::register(handlers);
    launcher_api::register(handlers);
    host_routes::register(handlers);
    updates::register(handlers);
    logs::register(handlers);
    guard::register(handlers);
    admin::register(handlers);
    notices::register(handlers);
    admin_guard::register(handlers);
    player_logs::register(handlers);
}
