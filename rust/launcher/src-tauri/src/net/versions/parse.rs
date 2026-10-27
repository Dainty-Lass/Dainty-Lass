//! The client profiles a joined server's content serves (DF4b), read without the network. The server is
//! not trusted with more than its own versions: each entry is checked as a module manifest would be, and an
//! entry that fails is dropped on its own.

use module_api::{API_VERSION, Capabilities, ClientProfile, GameVersion, ModuleKind, ModuleManifest};
use serde::Deserialize;
use serde_json::Value;

use super::KnownProfile;

/// More than any server loads modules for
pub const MAX_PROFILES: usize = 16;

/// One entry of `{ "profiles": [...] }`: the module manifest's own fields, under its own names.
#[derive(Deserialize)]
struct Served {
    game_version: GameVersion,
    display_name: String,
    client: ClientProfile,
}

/// `ModuleManifest::validate` is the one check of a profile, so an entry is wrapped in a manifest for it.
fn checked(served: Served) -> Option<KnownProfile> {
    let manifest = ModuleManifest {
        id: "served-profile".parse().ok()?,
        kind: ModuleKind::Game,
        display_name: served.display_name,
        game_version: served.game_version,
        invite_aliases: Vec::new(),
        api: API_VERSION,
        ports: Vec::new(),
        capabilities: Capabilities::default(),
        unverified: false,
        client: Some(served.client),
    };
    manifest.validate().ok()?;
    Some(KnownProfile {
        game_version: manifest.game_version,
        display_name: manifest.display_name,
        client: manifest.client?,
    })
}

/// The profiles in a `200` answer's JSON, the first of each version kept; anything else serves none.
pub fn read_profiles(body: Option<&Value>) -> Vec<KnownProfile> {
    let Some(entries) = body.and_then(|body| body.get("profiles")).and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out: Vec<KnownProfile> = Vec::new();
    for entry in entries.iter().take(MAX_PROFILES) {
        let Some(profile) = Served::deserialize(entry).ok().and_then(checked) else {
            continue;
        };
        if !out.iter().any(|known| known.game_version == profile.game_version) {
            out.push(profile);
        }
    }
    out
}
