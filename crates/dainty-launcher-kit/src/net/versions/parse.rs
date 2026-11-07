// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! What a joined server's content answers on DF4b's version routes, read without the network. The server is
//! not trusted with more than its own versions: each version's profile is checked as a module manifest
//! would be, and one that fails is dropped on its own.

use dainty_module_api::{API_VERSION, Capabilities, ClientProfile, GameVersion, ModuleKind, ModuleManifest};
use serde::Deserialize;
use serde_json::Value;

use super::KnownProfile;

/// More than any server loads modules for
pub const MAX_VERSIONS: usize = 16;

/// One row of `GET /content/v1/versions` (`crates/dainty-content/src/modules.rs`'s `VersionSummary`); the fields
/// the launcher doesn't read (`module`, `invite_aliases`, `unverified`) and a newer server's extras are
/// ignored.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ServedVersion {
    /// The game version.
    pub game_version: GameVersion,
    /// Name shown to the player.
    pub display_name: String,
    /// The module has a client profile at `/versions/<version>/client`
    #[serde(default)]
    pub client: bool,
    /// The server holds the version's pinned Epic manifest
    #[serde(default)]
    pub epic_manifest: bool,
}

/// The rows of a `200` answer's `{ "default_version", "versions": [...] }`, the first of each version kept;
/// anything else serves none.
pub fn read_versions(body: Option<&Value>) -> Vec<ServedVersion> {
    let Some(rows) = body.and_then(|body| body.get("versions")).and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out: Vec<ServedVersion> = Vec::new();
    for row in rows.iter().take(MAX_VERSIONS) {
        let Ok(row) = ServedVersion::deserialize(row) else {
            continue;
        };
        if !out.iter().any(|known| known.game_version == row.game_version) {
            out.push(row);
        }
    }
    out
}

/// A `/client` answer: the module's `ClientProfile`, in module-api's own field names.
pub fn read_client(body: Option<&Value>) -> Option<ClientProfile> {
    ClientProfile::deserialize(body?).ok()
}

/// `ModuleManifest::validate` is the one check of a profile, so a row and its profile are wrapped in a
/// manifest for it.
pub fn checked(row: &ServedVersion, client: ClientProfile) -> Option<KnownProfile> {
    let manifest = ModuleManifest {
        id: "served-profile".parse().ok()?,
        kind: ModuleKind::Game,
        display_name: row.display_name.clone(),
        game_version: row.game_version.clone(),
        invite_aliases: Vec::new(),
        api: API_VERSION,
        ports: Vec::new(),
        capabilities: Capabilities::default(),
        unverified: false,
        client: Some(client),
    };
    manifest.validate().ok()?;
    Some(KnownProfile {
        game_version: manifest.game_version,
        display_name: manifest.display_name,
        client: manifest.client?,
    })
}
