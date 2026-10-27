//! The client profiles compiled into the launcher (DE4). **A stand-in, to be replaced**: the 1.14.7 and 1.12
//! modules (RU4, rustic-isles) and the revived module (RV2) serve their own profiles through content (DD12),
//! and once they do this table goes. Until then it is what the version list starts from.

use module_api::{ClientProfile, EpicBuild, GameVersion};

use super::KnownProfile;
use crate::core::pins;

/// The community archive's raw files, pinned to the commit that added each manifest (DE1's `tests/archive.rs`)
const ARCHIVE: &str = "https://raw.githubusercontent.com/addison2k/DauntlessManifestArchive";

struct Epic {
    build: &'static str,
    manifest_sha1: &'static str,
    /// The archive's commit and file
    archive: (&'static str, &'static str),
}

struct Row {
    version: &'static str,
    /// DD4's display names
    name: &'static str,
    executable_sha256: &'static str,
    /// None: no archived manifest, so the player locates an existing folder (DD22)
    epic: Option<Epic>,
}

/// The executable SHA-256s are the ones Mystic's guide pins (rustic-isles `docs/INSTALLING_DAUNTLESS.md`);
/// 1.4.4's is DR's (`core/legacy_pins.rs`).
const ROWS: [Row; 3] = [
    Row {
        version: "1.14.7",
        name: "Rustic Isles 1.14.7",
        executable_sha256: pins::PINNED_EXE_SHA256,
        epic: Some(Epic {
            build: "1.14.7.647472",
            manifest_sha1: "aa70748e4c0a776a991f2bac10ad342adaa31476",
            archive: (
                "92e6082335b1c2234c88e9aec7fe2b704fc24494",
                "Dauntless_1.14.7.647472.manifest",
            ),
        }),
    },
    Row {
        version: "1.12.0",
        name: "Rustic Isles 1.12",
        executable_sha256: "ee30d1821b4020ff1bcaa89a63fa6ef3515efb622f2969546b91bf018a379f43",
        epic: Some(Epic {
            build: "1.12.0.392819",
            manifest_sha1: "46b4097c070afc29ed783e77a68ffa936b6a3504",
            archive: (
                "2d29ce97939a3e848e68f4b26bf5e285dcc5a70b",
                "Dauntless_1.12.0.392819.manifest",
            ),
        }),
    },
    Row {
        version: "1.4.4",
        name: "Dauntless Revived",
        executable_sha256: "d3d41e614908d2befd518b27046d9822d6130ef12ba3504babbdb786bef9cff4",
        epic: None,
    },
];

fn profile(row: &Row) -> Option<KnownProfile> {
    let epic = match &row.epic {
        Some(epic) => Some(EpicBuild {
            version: epic.build.to_owned(),
            manifest_sha1: epic.manifest_sha1.parse().ok()?,
            manifest_sources: vec![format!(
                "{ARCHIVE}/{}/manifests/Windows/{}",
                epic.archive.0, epic.archive.1
            )],
            executable_sha256: row.executable_sha256.parse().ok()?,
        }),
        None => None,
    };
    Some(KnownProfile {
        game_version: row.version.parse::<GameVersion>().ok()?,
        display_name: row.name.to_owned(),
        // The command line and ini edits stay the launcher's own until the modules' profiles carry them
        client: ClientProfile {
            executable: pins::EXE_RELATIVE_PATH.to_owned(),
            executable_sha256: Some(row.executable_sha256.parse().ok()?),
            arguments: Vec::new(),
            ini_edits: Vec::new(),
            epic,
        },
    })
}

/// The compiled-in profiles, newest first. Every row parses (`the_table_is_whole`).
pub fn profiles() -> Vec<KnownProfile> {
    ROWS.iter().filter_map(profile).collect()
}
