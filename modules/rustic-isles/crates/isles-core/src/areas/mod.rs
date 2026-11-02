//! The backend's handlers, one file per RU5 area group, so each area's agent changes only its own file.

pub mod auth;
pub mod bounties_store;
pub mod characters;
pub mod chat;
pub mod game_data;
pub mod inventory;
pub mod matchmaking;
pub mod progression;
pub mod reforged_api;

use crate::http::Handlers;

/// Every area's backend handlers.
pub fn backend() -> Handlers {
    let mut handlers = Handlers::default();
    auth::register(&mut handlers);
    reforged_api::register(&mut handlers);
    characters::register(&mut handlers);
    inventory::register(&mut handlers);
    progression::register(&mut handlers);
    bounties_store::register(&mut handlers);
    matchmaking::register(&mut handlers);
    chat::register(&mut handlers);
    game_data::register(&mut handlers);
    handlers
}
