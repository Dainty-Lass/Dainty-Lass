//! RU5's `CharactersLoadouts` group: Characters and loadouts. RU3c: hook `no_player_policy`, table `TARGET_CHANGELIST`.

use crate::http::Handlers;

/// Mounts this area's handlers (`handlers.on(Service::Backend, Method::Get, "/path", ...)`, the path exactly as the
/// route table has it); a route with none answers 501.
pub fn register(_handlers: &mut Handlers) {}
