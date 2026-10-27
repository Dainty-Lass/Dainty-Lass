//! RU5's `InventoryReforge` group: Inventory, storage policy and reforge. RU3c: hooks `no_player_policy`, `weapons_at_base_level`, `reforge_rules`; tables `SEASONAL_COIN`, `GAME_DATA`.
//!
//! Eight routes: the inventory read and the transaction (`routes/inventory.ts`), the migration stub the 1.14.7 client
//! reaches with an empty characterId, and the weapon reforge's reset and banked restore (`routes/progression.ts`).
//! What this area takes from the others is in [`ports`]; [`transaction`] and [`reforge`] are what they take from it.

pub mod body;
pub mod clock;
pub mod items;
pub mod multipliers;
pub mod ports;
pub mod progression_config;
pub mod reforge;
pub mod respond;
pub mod routes;
pub mod sha256;
pub mod storage_policy;
#[doc(hidden)]
pub mod testkit;
pub mod transaction;
pub mod validation;
pub mod weapon_level;

use std::sync::Arc;

use crate::http::Handlers;
use crate::routes::{Method, Service};

pub use ports::Ports;

/// Mounts this area's handlers (`handlers.on(Service::Backend, Method::Get, "/path", ...)`, the path exactly as the
/// route table has it); a route with none answers 501.
pub fn register(handlers: &mut Handlers) {
    register_with(handlers, Ports::default());
}

/// [`register`] with the other areas' pieces plugged in.
pub fn register_with(handlers: &mut Handlers, ports: Ports) {
    let ports = Arc::new(ports);
    macro_rules! route {
        ($method:expr, $path:expr, $handler:expr) => {{
            let ports = ports.clone();
            handlers.on(Service::Backend, $method, $path, move |call| {
                let ports = (*ports).clone();
                Box::pin($handler(call, ports))
            });
        }};
    }
    // The regex route is registered before the string route on purpose: Express 5 doesn't match :characterId against
    // an empty segment, so the 1.14.7 client's POST /inventory//<build> reaches only the regex
    handlers.on(Service::Backend, Method::Post, r"/^\/inventory\/([^/]*)\/(.+)$/", {
        let ports = ports.clone();
        move |call| {
            let ports = (*ports).clone();
            Box::pin(routes::migration_stub(call, ports, false))
        }
    });
    handlers.on(Service::Backend, Method::Post, "/inventory/:characterId/:changeList", {
        let ports = ports.clone();
        move |call| {
            let ports = (*ports).clone();
            Box::pin(routes::migration_stub(call, ports, true))
        }
    });
    route!(Method::Get, "/inventory/:userId/:characterId", routes::get_by_character);
    route!(Method::Get, "/inventory/:userId/", routes::get_without_character);
    route!(Method::Post, "/inventory", routes::post_transaction);
    route!(Method::Post, "/inventory/instanceditem", routes::post_instanced_item);
    route!(
        Method::Delete,
        "/progression/:userId/:progressionId",
        routes::reforge_delete
    );
    route!(
        Method::Post,
        "/progression/purchase/:userId/:progressionId/:amount",
        routes::reforge_purchase
    );
}
