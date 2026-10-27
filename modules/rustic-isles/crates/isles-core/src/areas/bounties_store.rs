//! RU5's `BountiesStore` group: bounties, cooldowns, entitlements, the wallet, and the store with Lady Luck's and the
//! Platinum shops. RU3c: hook `entitlements_payload`; tables `STORE_SECTIONS`, `SEASONAL_COIN`, `GAME_DATA`; the
//! persistence switches `MYSTICPARADOX_BOUNTY_PERSISTENCE` and `_COOLDOWN_PERSISTENCE` carry 1.12.0's stubs.
//!
//! What other areas call: [`wallet`] (`get_wallet`, `add_currency`, `spend_platinum`, `merge_wallet_into_stacked`),
//! [`challenge_rewards`] and [`entitlements`] (inside their transactions), [`store`]. What the
//! pipeline must supply: [`authn::AuthData`] and [`authn::StoreSeams`] in the request's extensions.

pub mod authn;
pub mod bounty_state;
#[cfg(test)]
mod bounty_state_tests;
pub mod challenge_rewards;
pub mod cooldown_state;
#[cfg(test)]
mod cooldown_state_tests;
pub mod entitlements;
pub mod handlers;
#[doc(hidden)]
pub mod harness;
pub mod monitor;
pub mod platinum;
pub mod store;
#[cfg(test)]
mod store_tests;
pub mod util;
pub mod wallet;

use crate::http::{Call, Handlers};
use crate::routes::{Method, Service};

/// Mounts this area's handlers (`handlers.on(Service::Backend, Method::Get, "/path", ...)`, the path exactly as the
/// route table has it); a route with none answers 501.
pub fn register(handlers: &mut Handlers) {
    macro_rules! mount {
        ($method:ident, $path:literal, $handler:path) => {
            handlers.on(Service::Backend, Method::$method, $path, |call: Call| {
                Box::pin($handler(call))
            });
        };
    }
    mount!(Get, "/entitlementsv2", handlers::entitlements_get);
    mount!(Post, "/entitlementv2/:userId", handlers::entitlement_post);
    mount!(Get, "/cooldown/:userId", handlers::cooldown_get);
    mount!(Put, "/cooldown/batch/:userId", handlers::cooldown_batch);
    mount!(Put, "/cooldown/:userId/:cooldownId", handlers::cooldown_start);
    mount!(Post, "/cooldown/:userId/:cooldownId", handlers::cooldown_start);
    mount!(Get, "/bounty/game-data", handlers::bounty_game_data);
    mount!(Get, "/bounty/:userId", handlers::bounty_get);
    mount!(Post, "/bounty/:userId", handlers::bounty_post);
    mount!(Post, "/bounty/delete/:userId", handlers::bounty_delete);
    mount!(Delete, "/bounty/delete/:userId", handlers::bounty_delete);
    mount!(Post, "/reconcile", handlers::reconcile);
    mount!(Get, "/creator", handlers::creator);
    mount!(Get, "/balance", handlers::balance);
    mount!(Get, "/product/skus/public", handlers::product_skus);
    mount!(Get, "/token/:currency/:catalogId", handlers::token);
    mount!(Post, "/notification/:currency", handlers::notification);
}
