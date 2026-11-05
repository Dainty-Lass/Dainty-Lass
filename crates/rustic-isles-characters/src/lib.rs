// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Rustic Isles' characters and loadouts area (RD27, written clean-room from
//! `docs/port/spec-rustic-isles-characters.md`): `GET`, `PUT` and `POST /character`, the five `/loadout` routes, the
//! character-data rules every read and write applies, and the starter records a new character is seeded with (and
//! repaired from when one goes missing). Game-derived values are inputs ([`CharacterData`], [`StarterKit`]).

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
// A refusal is the answer itself (an axum `Response`), as in core and auth
#![allow(clippy::result_large_err)]

mod controller;
pub mod data;
mod routes;
pub mod starter;

use rustic_isles::Method;
use rustic_isles_store::RepoError;
use serde_json::{Map, Value};

pub use controller::{Characters, LoadoutState, empty_loadout_payload, is_fresh_onboarding, loadout_payload};
pub use routes::register;

/// The character-data rules' values (spec §6.1); the version crate supplies them.
#[derive(Debug, Clone, PartialEq)]
pub struct CharacterData {
    /// Set, in order, where a key is absent or `null`
    pub fill: Vec<(String, Value)>,
    /// With `Data` and `StringData` arrays
    pub default_appearance: Map<String, Value>,
    pub forced_character_flags: Vec<(String, Value)>,
    pub forced_login_flag: (String, Value),
    /// The member the Dojo series is kept in (`SERIE_dojo` for 1.14.7)
    pub dojo_series_key: String,
    /// Stored as its compact text
    pub dojo_series: Map<String, Value>,
    /// The journey node that unlocks the Dojo
    pub dojo_node_id: String,
    pub trigger_series_key: String,
    pub trigger_node: String,
    pub trigger_status: f64,
}

/// The starter manifest's run-time values (spec §6.2); the stacked ids and quick emojis are core's table's.
#[derive(Debug, Clone, PartialEq)]
pub struct StarterKit {
    /// Catalog ids given an instance id each, in this order
    pub instanced: Vec<String>,
    /// A subset of `instanced` a read appends when missing
    pub default_weapon_parts: Vec<String>,
    /// `(slot key, catalog id)`, the slot's wire keys
    pub equipment: Vec<(String, String)>,
    /// `(catalog id, object)`: that equipment entry's `instance_data`
    pub instance_data: Vec<(String, Map<String, Value>)>,
    pub flask: String,
    pub banner: String,
    pub glider: String,
    pub banner_customization: Map<String, Value>,
    pub intro_emote: String,
    pub flare: String,
    /// Nine texts
    pub default_quick_chats: Vec<String>,
    /// `(catalog id, amount)`, the starter wallet (shared with the economy unit)
    pub wallet: Vec<(String, f64)>,
    /// The slot's `appearance`: [`CharacterData::default_appearance`], which [`Characters::new`] copies in
    pub default_appearance: Map<String, Value>,
}

#[derive(Debug, thiserror::Error)]
pub enum CharactersError {
    #[error("the character does not belong to the user")]
    NotOwned,
    #[error(transparent)]
    Repo(#[from] RepoError),
    #[error("{0}")]
    Data(String),
    #[error("{0}")]
    Failed(String),
}

impl From<data::DataError> for CharactersError {
    fn from(error: data::DataError) -> Self {
        Self::Data(error.0)
    }
}

/// The in-transaction starter check's errors.
#[derive(Debug, thiserror::Error)]
pub enum EnsureError {
    #[error("Character {character_id} does not belong to user {user}")]
    NotOwned { character_id: String, user: String },
    #[error(transparent)]
    Repo(#[from] RepoError),
}

/// The eight `characters-loadouts` rows (spec §3), in the table's order.
const ROUTES: [(Method, &str); 8] = [
    (Method::Get, "/character"),
    (Method::Put, "/character"),
    (Method::Post, "/character"),
    (Method::Get, "/loadout/:userId//all"),
    (Method::Get, "/loadout/:userId/:characterId/all"),
    (Method::Post, "/loadout/:userId/:characterId/unlock/:numSlots"),
    (Method::Post, "/loadout/:userId/:characterId/active/:index"),
    (Method::Post, "/loadout/:userId/:characterId/:index"),
];

/// The (method, path) pairs this crate answers.
pub fn routes() -> &'static [(Method, &'static str)] {
    &ROUTES
}
