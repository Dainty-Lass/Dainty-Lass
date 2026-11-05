//! `gameData/loader.ts`: a game data file as `JSON.parse(readFileSync(path, "utf8").replace(/^﻿/, ""))` reads
//! it, from the module's own export folder ([`GameDataRoot`]). Nothing from the game is committed (AGENTS.md).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::Value;

use super::GameDataRoot;

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// `loadGameData`'s throw; the module has no folder for the file, or the file isn't there.
    #[error(
        "[game-data] Required game data file is missing: {0}\nThis project does not ship game data. Generate it from \
         your own installation (see docs/GENERATING_GAME_DATA.md), or copy the matching *.example.json for a smoke test."
    )]
    Missing(String),
    #[error("{path}: {message}")]
    Unreadable { path: String, message: String },
}

/// `tryLoadGameData`: `Ok(None)` when the file isn't there, an error when it is but can't be read or parsed.
pub fn read_optional(path: &Path) -> Result<Option<Value>, LoadError> {
    let text = match std::fs::read(path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(LoadError::Unreadable {
                path: path.display().to_string(),
                message: error.to_string(),
            });
        }
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    service_kit::jsjson::parse(text)
        .map(Some)
        .map_err(|error| LoadError::Unreadable {
            path: path.display().to_string(),
            message: error.to_string(),
        })
}

/// `loadGameData`: the file, or [`LoadError::Missing`].
pub fn read_required(path: &Path) -> Result<Value, LoadError> {
    read_optional(path)?.ok_or_else(|| LoadError::Missing(path.display().to_string()))
}

impl GameDataRoot {
    /// `loadGameData` on one of the backend's files; `file` is read again each call.
    pub fn load_backend(&self, file: &str) -> Result<Value, LoadError> {
        match self.backend_file(file) {
            Some(path) => read_required(&path),
            None => Err(LoadError::Missing(format!("<no backend game data folder>/{file}"))),
        }
    }

    /// `tryLoadGameData` on one of the backend's files.
    pub fn try_load_backend(&self, file: &str) -> Result<Option<Value>, LoadError> {
        match self.backend_file(file) {
            Some(path) => read_optional(&path),
            None => Ok(None),
        }
    }

    /// A required backend file read once per process, as a Node module that loads it at import keeps it
    /// (`routes/system.ts`'s `progressionconfig`). The cache is keyed by path, since a node can hold two modules.
    pub fn load_backend_cached(&self, file: &str) -> Result<Arc<Value>, LoadError> {
        let Some(path) = self.backend_file(file) else {
            return Err(LoadError::Missing(format!("<no backend game data folder>/{file}")));
        };
        static CACHE: Mutex<Option<HashMap<PathBuf, Arc<Value>>>> = Mutex::new(None);
        let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
        let cache = cache.get_or_insert_with(HashMap::new);
        if let Some(value) = cache.get(&path) {
            return Ok(value.clone());
        }
        let value = Arc::new(read_required(&path)?);
        cache.insert(path, value.clone());
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_reads_like_json_parse_after_the_bom_strip() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.json"),
            "\u{feff}{\"n\":1.0,\"big\":12345678901234567890}",
        )
        .unwrap();
        let root = GameDataRoot::from_config(dir.path().to_str(), None);
        let value = root.load_backend("a.json").unwrap();
        assert_eq!(
            service_kit::jsjson::stringify(&value),
            r#"{"n":1,"big":12345678901234567000}"#
        );
        assert_eq!(root.try_load_backend("b.json").unwrap(), None);
        assert!(matches!(root.load_backend("b.json"), Err(LoadError::Missing(_))));
        std::fs::write(dir.path().join("bad.json"), "{").unwrap();
        assert!(matches!(
            root.try_load_backend("bad.json"),
            Err(LoadError::Unreadable { .. })
        ));
        assert!(matches!(
            GameDataRoot::default().load_backend("a.json"),
            Err(LoadError::Missing(_))
        ));
    }

    #[test]
    fn the_cached_load_keeps_the_first_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.json");
        std::fs::write(&path, "{\"v\":1}").unwrap();
        let root = GameDataRoot::from_config(dir.path().to_str(), None);
        let first = root.load_backend_cached("c.json").unwrap();
        std::fs::write(&path, "{\"v\":2}").unwrap();
        assert_eq!(root.load_backend_cached("c.json").unwrap(), first);
        assert_eq!(first["v"], 1);
    }
}
