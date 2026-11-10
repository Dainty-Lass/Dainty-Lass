// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The 1.14.7 module's `version_data.json`, when the build compiled one in (`DAINTY_VERSION_DATA`, build.rs):
//! handed to `spec_with` for the compiled-in module, to the generators and the db tool, and installed in the
//! backend game data folder for a module loaded as a library or a child, which reads the file (RD27,
//! spec-rustic-isles-1-14-7 §4.3 and its decision item 7). restricted/ builds with Mystic Paradox's document.

use std::path::Path;

use rustic_isles_1_14_7::{DATA_FILE, VersionData};

const COMPILED: &str = include_str!(concat!(env!("OUT_DIR"), "/version_data.json"));

/// The compiled-in document, or `None` in a build without one.
pub fn document() -> Option<&'static str> {
    (!COMPILED.is_empty()).then_some(COMPILED)
}

/// Parses the compiled-in document.
pub fn load() -> Result<VersionData, String> {
    let document =
        document().ok_or_else(|| format!("this build has no {DATA_FILE} compiled in (DAINTY_VERSION_DATA)"))?;
    let value: serde_json::Value =
        serde_json::from_str(document).map_err(|e| format!("{DATA_FILE} (compiled in): {e}"))?;
    VersionData::from_json(&value)
}

/// The legacy starter `db migrate` and `verify` repair with.
pub fn legacy_starter() -> Result<rustic_isles_db::LegacyStarter, String> {
    load()?
        .legacy_starter()
        .ok_or_else(|| format!("{DATA_FILE} (compiled in) has no legacyStarter"))
}

/// Writes the compiled-in document into `backend_game_data` when it holds something else; true when it wrote. A
/// build without one leaves the installed file as it is.
pub fn install(backend_game_data: &Path) -> std::io::Result<bool> {
    let Some(document) = document() else {
        return Ok(false);
    };
    let target = backend_game_data.join(DATA_FILE);
    if std::fs::read(&target).is_ok_and(|on_disk| on_disk == document.as_bytes()) {
        return Ok(false);
    }
    std::fs::create_dir_all(backend_game_data)?;
    std::fs::write(&target, document)?;
    Ok(true)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]
mod tests {
    use super::*;

    #[cfg(dainty_version_data)]
    #[test]
    fn the_compiled_in_document_is_complete() {
        let data = load().unwrap();
        assert!(data.legacy_starter().is_some());
        assert!(data.generator_inputs().is_some());
    }

    #[cfg(dainty_version_data)]
    #[test]
    fn install_writes_once_and_then_leaves_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let backend = dir.path().join("game-data");
        assert!(install(&backend).unwrap());
        assert!(!install(&backend).unwrap());
        std::fs::write(backend.join(DATA_FILE), "{}").unwrap();
        assert!(install(&backend).unwrap());
        assert_eq!(
            std::fs::read_to_string(backend.join(DATA_FILE)).unwrap(),
            document().unwrap()
        );
    }

    #[cfg(not(dainty_version_data))]
    #[test]
    fn a_build_without_a_document_installs_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(document().is_none());
        assert!(load().unwrap_err().contains("DAINTY_VERSION_DATA"));
        assert!(!install(dir.path()).unwrap());
        assert!(!dir.path().join(DATA_FILE).exists());
    }
}
