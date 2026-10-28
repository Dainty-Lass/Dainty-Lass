//! The client profiles compiled into the launcher (DE4). **A stand-in, to be replaced**: the 1.14.7 and 1.12
//! modules (RU4, rustic-isles) and the revived module (RV2) serve their own profiles through content (DD12),
//! and once they do this table goes. Until then it is what the version list starts from.

use epic::builds::{self, Build};
use module_api::{ClientProfile, EpicBuild, GameVersion};

use super::KnownProfile;
use crate::core::pins;

/// Dauntless Revived's 1.4.4 has no archived manifest, so the player locates an existing folder (DD22). Its
/// executable SHA-256 is DR's (`core/legacy_pins.rs`). The Epic builds are `epic::builds`, which the host kit's
/// `install-game` reads too.
const REVIVED: (&str, &str, &str) = (
    "1.4.4",
    "Dauntless Revived",
    "d3d41e614908d2befd518b27046d9822d6130ef12ba3504babbdb786bef9cff4",
);

fn epic_profile(build: &Build) -> Option<KnownProfile> {
    Some(KnownProfile {
        game_version: build.version.parse::<GameVersion>().ok()?,
        display_name: build.display_name.to_owned(),
        // The command line and ini edits stay the launcher's own until the modules' profiles carry them
        client: ClientProfile {
            executable: pins::EXE_RELATIVE_PATH.to_owned(),
            executable_sha256: Some(build.executable_sha256.parse().ok()?),
            files: Vec::new(),
            arguments: Vec::new(),
            environment: std::collections::BTreeMap::new(),
            ini_edits: Vec::new(),
            epic: Some(EpicBuild {
                version: build.build_version.to_owned(),
                manifest_sha1: build.manifest_sha1.parse().ok()?,
                manifest_sources: vec![build.manifest_source()],
                executable_sha256: build.executable_sha256.parse().ok()?,
            }),
        },
    })
}

fn revived_profile() -> Option<KnownProfile> {
    let (version, name, sha256) = REVIVED;
    Some(KnownProfile {
        game_version: version.parse::<GameVersion>().ok()?,
        display_name: name.to_owned(),
        client: ClientProfile {
            executable: pins::EXE_RELATIVE_PATH.to_owned(),
            executable_sha256: Some(sha256.parse().ok()?),
            files: Vec::new(),
            arguments: Vec::new(),
            environment: std::collections::BTreeMap::new(),
            ini_edits: Vec::new(),
            epic: None,
        },
    })
}

/// The compiled-in profiles, newest first. Every row parses (`the_table_is_whole`).
pub fn profiles() -> Vec<KnownProfile> {
    builds::BUILDS
        .iter()
        .filter_map(epic_profile)
        .chain(revived_profile())
        .collect()
}
