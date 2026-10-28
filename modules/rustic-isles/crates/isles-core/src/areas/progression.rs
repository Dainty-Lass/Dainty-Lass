//! RU5's `Progression` group: Progression, mastery, escalation and session supplies. RU3c: hooks `huntpass_payload`, `objective_row`, `reforge_rules`, `mastery_persistence`; tables `ACTIVE_HUNTPASS_ID`, `MASKED_JOURNEY_NODES`, `GAME_DATA`.
//!
//! Ported from `routes/progression.ts`, the escalation, huntpass and stub routes of `routes/system.ts`, and the
//! controllers behind them (`progression.ts`, `masteryReport.ts`, `escalationProgress.ts`, `slayersPath.ts`,
//! `sessionSupplies.ts`, the XP half of `testMultipliers.ts`). 1.12.0 runs the same code through the hooks and tables
//! (DD8): the reforge rules and mastery persistence follow their switches, whose default is the version's.
//!
//! The rest of the pipeline is other areas': the auth middleware puts an [`AuthData`] in the request's extensions, and
//! the shared pipeline parses bodies (`service_kit::body::parse_bodies`) before routing. What this area takes from
//! other areas is in [`Ports`].

// A refusal is the finished `Response`, returned early the way Express's `return res.status(..)` does
#![allow(clippy::result_large_err, clippy::large_enum_variant)]

mod auth;
mod handlers;
mod ports;
mod req;
mod xp;

pub mod escalation;
pub mod journey;
pub mod mastery;
pub mod reforge;
pub mod supplies;
#[doc(hidden)]
pub mod testkit;
pub mod tracks;
pub mod util;

use crate::http::{Call, Handlers};
use crate::routes::{Method, Service};

pub use auth::AuthData;
pub use journey::{FirstCharacter, RewardTransaction};
pub use journey::{JourneyInventory, UnwiredInventory};
pub use ports::{
    Activity, ActivityMonitor, Characters, CharactersStarterBootstrap, NoActivity, NoStarterBootstrap, Ports,
    RepoCharacters, StarterBootstrap,
};

type Route = fn(Call, Ports) -> module_api::BoxFuture<'static, axum::response::Response>;

/// Mounts this area's handlers (`handlers.on(Service::Backend, Method::Get, "/path", ...)`, the path exactly as the
/// route table has it); a route with none answers 501.
pub fn register(handlers: &mut Handlers) {
    register_with(handlers, Ports::default());
}

/// [`register`] with the other areas' parts wired in.
pub fn register_with(handlers: &mut Handlers, ports: Ports) {
    let mut on = |method: Method, path: &'static str, route: Route| {
        let ports = ports.clone();
        handlers.on(Service::Backend, method, path, move |call| route(call, ports.clone()));
    };
    on(Method::Get, "/escalation/:escalationSeason/:userId", |c, p| {
        Box::pin(handlers::escalation_get(c, p))
    });
    on(Method::Post, "/escalation/:escalationSeason/:userId", |c, p| {
        Box::pin(handlers::escalation_post(c, p))
    });
    on(Method::Get, "/huntpass/:userId", |c, p| {
        Box::pin(handlers::huntpass(c, p))
    });
    on(Method::Get, "/encountered-content/:characterId/:contentType", |c, p| {
        Box::pin(handlers::encountered_get(c, p))
    });
    on(Method::Post, "/encountered-content/query/:characterId", |c, p| {
        Box::pin(handlers::encountered_query(c, p))
    });
    on(Method::Post, "/encountered-content/query", |c, p| {
        Box::pin(handlers::encountered_query_anonymous(c, p))
    });
    on(Method::Post, "/encountered-content/:characterId/query", |c, p| {
        Box::pin(handlers::encountered_query(c, p))
    });
    on(Method::Post, "/encountered-content/:characterId", |c, p| {
        Box::pin(handlers::encountered_add(c, p))
    });
    on(Method::Post, "/encountered-content", |c, p| {
        Box::pin(handlers::encountered_noop(c, p))
    });
    // Never answers: `/encountered-content` takes it first (RU3a), kept so the table's row has its handler
    on(Method::Post, "/encountered-content/", |c, p| {
        Box::pin(handlers::encountered_noop(c, p))
    });
    on(Method::Get, "/progression/objectives/:userId", |c, p| {
        Box::pin(handlers::objectives(c, p))
    });
    on(Method::Get, "/breadcrumbs/", |c, p| {
        Box::pin(handlers::breadcrumbs_anonymous(c, p))
    });
    on(Method::Get, "/breadcrumbs/:characterId", |c, p| {
        Box::pin(handlers::breadcrumbs_get(c, p))
    });
    on(Method::Post, "/breadcrumbs/:characterId", |c, p| {
        Box::pin(handlers::breadcrumbs_post(c, p))
    });
    on(Method::Post, "/progression/:userId/:progressionId/:amount", |c, p| {
        Box::pin(xp::grant(c, p))
    });
    on(Method::Post, "/progression/:userId", |c, p| Box::pin(xp::report(c, p)));
    on(
        Method::Post,
        "/progression/:userId/:progressionId/:rank/confirm/public",
        |c, p| Box::pin(xp::confirm(c, p)),
    );
    on(Method::Get, "/progression/:userId", |c, p| {
        Box::pin(handlers::progression_list(c, p))
    });
    on(Method::Get, "/pjm/:userId", |c, p| Box::pin(handlers::pjm_get(c, p)));
    on(Method::Get, "/weapon_tracker/:userId/:itemId", |c, p| {
        Box::pin(handlers::weapon_tracker(c, p))
    });
    on(Method::Post, "/pjm/:userId", |c, p| Box::pin(handlers::pjm_post(c, p)));
    on(Method::Get, "/pjm", |c, p| Box::pin(handlers::pjm_anonymous(c, p)));
    on(Method::Get, "/progression/tracked_objectives/:phxAccountId", |c, p| {
        Box::pin(handlers::tracked_objectives_get(c, p))
    });
    on(Method::Post, "/progression/tracked_objectives/:phxAccountId", |c, p| {
        Box::pin(handlers::tracked_objectives_post(c, p))
    });
    on(Method::Get, "/progression/:userId/:progressionId", |c, p| {
        Box::pin(handlers::progression_single(c, p))
    });
}

/// The routes this area answers, for the integration pass's checklist.
pub const ROUTES: usize = 25;
