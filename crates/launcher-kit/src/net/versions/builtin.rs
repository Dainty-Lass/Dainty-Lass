// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The client profiles compiled into the launcher (DE4). **A stand-in, to be replaced**: the 1.14.7 module
//! (RU4, rustic-isles) serves its own profile through content (DD12), and once it does this table goes. Until
//! then it is what the version list starts from.

use epic::builds::{self, Build};
use module_api::{ClientProfile, EpicBuild, GameVersion};

use super::KnownProfile;
use crate::core::pins;

/// The Epic builds are `epic::builds`, which the host kit's `install-game` reads too.
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

/// The compiled-in profiles, newest first. Every row parses (`the_table_is_whole`).
pub fn profiles() -> Vec<KnownProfile> {
    builds::BUILDS.iter().filter_map(epic_profile).collect()
}
