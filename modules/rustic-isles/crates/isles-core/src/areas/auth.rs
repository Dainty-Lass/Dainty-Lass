//! RU5's `Auth` group: Login, EOS and launcher accounts. RU3c: hooks `game_session_payload` and `network_encryption_keys` (and the auth middleware's `no_player_policy`), table `TARGET_CHANGELIST`.

use crate::http::Handlers;

/// Mounts this area's handlers (`handlers.on(Service::Backend, Method::Get, "/path", ...)`, the path exactly as the
/// route table has it); a route with none answers 501.
pub fn register(_handlers: &mut Handlers) {}
