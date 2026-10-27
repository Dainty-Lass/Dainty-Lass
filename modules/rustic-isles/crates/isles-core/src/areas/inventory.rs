//! RU5's `InventoryReforge` group: Inventory, storage policy and reforge. RU3c: hooks `no_player_policy`, `weapons_at_base_level`, `reforge_rules`; tables `SEASONAL_COIN`, `GAME_DATA`.

use crate::http::Handlers;

/// Mounts this area's handlers (`handlers.on(Service::Backend, Method::Get, "/path", ...)`, the path exactly as the
/// route table has it); a route with none answers 501.
pub fn register(_handlers: &mut Handlers) {}
