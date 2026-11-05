//! Restricted's `version_data.json` for the MIT 1.14.7 module (RD27, spec-rustic-isles-1-14-7 §4.3 and its decision
//! item 7): compiled in, handed to `spec_with` for the compiled-in module, to the generators and the db tool, and
//! installed in the backend game data folder for a module loaded as a library or a child, which reads the file.
//! isles-mit-parity's `version_data` test keeps it equal to restricted's sources.

use std::path::Path;

use rustic_isles_1_14_7::{DATA_FILE, VersionData};

pub const DOCUMENT: &str = include_str!("../../../modules/rustic-isles/version-data/1.14.7/version_data.json");

pub fn load() -> Result<VersionData, String> {
    let value: serde_json::Value =
        serde_json::from_str(DOCUMENT).map_err(|e| format!("{DATA_FILE} (compiled in): {e}"))?;
    VersionData::from_json(&value)
}

/// The legacy starter `db migrate` and `verify` repair with.
pub fn legacy_starter() -> Result<rustic_isles_db::LegacyStarter, String> {
    load()?
        .legacy_starter()
        .ok_or_else(|| format!("{DATA_FILE} (compiled in) has no legacyStarter"))
}

/// Writes the document into `backend_game_data` when it holds something else; true when it wrote.
pub fn install(backend_game_data: &Path) -> std::io::Result<bool> {
    let target = backend_game_data.join(DATA_FILE);
    if std::fs::read(&target).is_ok_and(|on_disk| on_disk == DOCUMENT.as_bytes()) {
        return Ok(false);
    }
    std::fs::create_dir_all(backend_game_data)?;
    std::fs::write(&target, DOCUMENT)?;
    Ok(true)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn the_compiled_in_document_is_complete() {
        let data = load().unwrap();
        assert!(data.legacy_starter().is_some());
        assert!(data.generator_inputs().is_some());
    }

    #[test]
    fn install_writes_once_and_then_leaves_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let backend = dir.path().join("game-data");
        assert!(install(&backend).unwrap());
        assert!(!install(&backend).unwrap());
        std::fs::write(backend.join(DATA_FILE), "{}").unwrap();
        assert!(install(&backend).unwrap());
        assert_eq!(std::fs::read_to_string(backend.join(DATA_FILE)).unwrap(), DOCUMENT);
    }
}
