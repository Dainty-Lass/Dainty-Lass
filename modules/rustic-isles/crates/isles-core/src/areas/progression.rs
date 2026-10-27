//! RU5's `Progression` group: Progression, mastery, escalation and session supplies. RU3c: hooks `huntpass_payload`, `objective_row`, `reforge_rules`, `mastery_persistence`; tables `ACTIVE_HUNTPASS_ID`, `MASKED_JOURNEY_NODES`, `GAME_DATA`.

use crate::http::Handlers;

/// Mounts this area's handlers (`handlers.on(Service::Backend, Method::Get, "/path", ...)`, the path exactly as the
/// route table has it); a route with none answers 501.
pub fn register(_handlers: &mut Handlers) {}
