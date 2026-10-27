//! RU5's `BountiesStore` group: Bounties, store, Lady Luck and platinum. RU3c: hook `entitlements_payload`; tables `STORE_SECTIONS`, `SEASONAL_COIN`, `GAME_DATA`.

use crate::http::Handlers;

/// Mounts this area's handlers (`handlers.on(Service::Backend, Method::Get, "/path", ...)`, the path exactly as the
/// route table has it); a route with none answers 501.
pub fn register(_handlers: &mut Handlers) {}
