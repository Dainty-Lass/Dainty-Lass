// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The 1.14.7 module's `version_data.json`, read at start from the file host.toml's `version_data` names (RD27,
//! spec-rustic-isles-1-14-7 §4.3): handed to `spec_with` for the compiled-in module, to the generators and the db
//! tool, and installed in the backend game data folder for a module loaded as a library or a child, which reads it
//! there. The document is Mystic Paradox's and ships beside the MIT binaries, never inside them.

use std::path::{Path, PathBuf};

use dainty_hostkit::hostconfig::{self, HostConfig};
use dainty_hostkit::paths::Paths;
use rustic_isles_1_14_7::{DATA_FILE, VersionData};

/// The document as read, with where it came from.
#[derive(Clone)]
pub struct Loaded {
    /// The file it was read from.
    pub path: PathBuf,
    /// Its bytes, as `install` copies them.
    pub document: String,
    /// The checked values.
    pub data: VersionData,
}

/// The file host.toml names, or the shipped default.
pub fn path(paths: &Paths, config: Option<&HostConfig>) -> PathBuf {
    hostconfig::version_data_path(paths, config)
}

/// Reads and checks the document at `path`. The error says what is wrong and how a host fixes it.
pub fn load_file(path: &Path) -> Result<Loaded, String> {
    let document = std::fs::read_to_string(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!(
                "the 1.14.7 version data is missing: {} does not exist. The server release ships it as {}; \
                 set version_data in host.toml to read it from somewhere else",
                path.display(),
                hostconfig::DEFAULT_VERSION_DATA
            )
        } else {
            format!("could not read the 1.14.7 version data {}: {e}", path.display())
        }
    })?;
    let value: serde_json::Value = serde_json::from_str(&document)
        .map_err(|e| format!("the 1.14.7 version data {} is not JSON: {e}", path.display()))?;
    let data = VersionData::from_json(&value)
        .map_err(|e| format!("the 1.14.7 version data {} is not valid: {e}", path.display()))?;
    Ok(Loaded {
        path: path.to_path_buf(),
        document,
        data,
    })
}

/// Reads the document `paths`' host.toml names (or the default when it has none or cannot be read).
pub fn load(paths: &Paths) -> Result<Loaded, String> {
    let config = hostconfig::load_host_config(paths).ok().flatten();
    load_file(&path(paths, config.as_ref()))
}

/// The legacy starter `db migrate` and `verify` repair with.
pub fn legacy_starter(paths: &Paths) -> Result<rustic_isles_db::LegacyStarter, String> {
    let loaded = load(paths)?;
    loaded
        .data
        .legacy_starter()
        .ok_or_else(|| format!("{} has no legacyStarter", loaded.path.display()))
}

/// Writes the document into `backend_game_data` when it holds something else; true when it wrote.
pub fn install(loaded: &Loaded, backend_game_data: &Path) -> std::io::Result<bool> {
    let target = backend_game_data.join(DATA_FILE);
    if std::fs::read(&target).is_ok_and(|on_disk| on_disk == loaded.document.as_bytes()) {
        return Ok(false);
    }
    std::fs::create_dir_all(backend_game_data)?;
    std::fs::write(&target, &loaded.document)?;
    Ok(true)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]
mod tests {
    use super::*;

    fn write_synthetic(path: &Path) -> String {
        let document = rustic_isles_1_14_7::test_support::synthetic_document().to_string();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, &document).unwrap();
        document
    }

    #[test]
    fn the_default_is_read_from_the_server_folder() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        write_synthetic(&dir.path().join(hostconfig::DEFAULT_VERSION_DATA));
        let loaded = load(&paths).unwrap();
        assert_eq!(loaded.path, dir.path().join(hostconfig::DEFAULT_VERSION_DATA));
        assert!(loaded.data.legacy_starter().is_some());
    }

    #[test]
    fn a_missing_or_broken_file_says_where_and_what() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("version_data.json");
        let missing = load_file(&file).err().unwrap();
        assert!(
            missing.contains("is missing") && missing.contains("version_data in host.toml"),
            "{missing}"
        );
        std::fs::write(&file, "{").unwrap();
        assert!(load_file(&file).err().unwrap().contains("is not JSON"));
        std::fs::write(&file, "{}").unwrap();
        let invalid = load_file(&file).err().unwrap();
        assert!(invalid.contains("is not valid: version_data.json: "), "{invalid}");
    }

    #[test]
    fn install_writes_once_and_then_leaves_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("in").join("version_data.json");
        let document = write_synthetic(&file);
        let loaded = load_file(&file).unwrap();
        let backend = dir.path().join("game-data");
        assert!(install(&loaded, &backend).unwrap());
        assert!(!install(&loaded, &backend).unwrap());
        std::fs::write(backend.join(DATA_FILE), "{}").unwrap();
        assert!(install(&loaded, &backend).unwrap());
        assert_eq!(std::fs::read_to_string(backend.join(DATA_FILE)).unwrap(), document);
    }
}
