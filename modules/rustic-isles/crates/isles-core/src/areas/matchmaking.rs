//! RU5's `MatchmakingParties` group: Multipliers, matchmaking, parties and the game servers' posts. RU3c: hook `candidate_destination`; tables `TARGET_CHANGELIST`, `GAME_VERSION`, and `QOS_PORT`'s listener (a public one, so the gateway's to place, DD3).

use crate::http::Handlers;

/// Mounts this area's handlers (`handlers.on(Service::Backend, Method::Get, "/path", ...)`, the path exactly as the
/// route table has it); a route with none answers 501.
pub fn register(_handlers: &mut Handlers) {}
