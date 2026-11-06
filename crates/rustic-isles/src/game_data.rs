// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The game data root and loader (core spec §8.1, §8.2). Nothing generated from the game is committed: the files
//! come from the host's own install, read only from the configured folders by fixed names.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use module_api::ModuleConfig;
use serde_json::Value;

use crate::settings::config_text;

pub const BACKEND_FILES: [&str; 11] = [
    "progression_config.json",
    "slayers_path.json",
    "slayers_path_definitions.json",
    "ladyluck_store.json",
    "reward_cache_store.json",
    "platinum_store.json",
    "inventory_storage_policy.json",
    "mastery_objectives.json",
    "escalation_tuning.json",
    "fountain_daily_bundle.json",
    "tester_max_kit.json",
];

pub const DIRECTOR_FILES: [&str; 5] = [
    "player_hunts_table.json",
    "matchmaker_hunts_table.json",
    "arena_easy_matchmaker_hunts.json",
    "arena_hard_matchmaker_hunts.json",
    "arena_elite_matchmaker_hunts.json",
];

/// A version with Trials tables (1.14.7) also has these, after the rest.
pub const DIRECTOR_TRIALS_FILES: [&str; 2] = [
    "arena_hard_matchmaker_hunts_new.json",
    "arena_elite_matchmaker_hunts_new.json",
];

pub fn director_files(trials: bool) -> Vec<&'static str> {
    let mut files = DIRECTOR_FILES.to_vec();
    if trials {
        files.extend(DIRECTOR_TRIALS_FILES);
    }
    files
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoadError {
    #[error("{0}")]
    Missing(String),
    #[error("{0}")]
    Unreadable(String),
}

/// One module's folders, and its cache of required reads (per module, not per process).
#[derive(Debug, Clone, Default)]
pub struct GameDataRoot {
    pub backend: Option<PathBuf>,
    pub director: Option<PathBuf>,
    cache: Arc<Mutex<HashMap<PathBuf, Value>>>,
}

fn folder(value: Option<&Value>) -> Option<PathBuf> {
    let text = config_text(value?)?;
    let text = text.trim();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

fn missing(path: &str) -> LoadError {
    LoadError::Missing(format!(
        "[game-data] Required game data file is missing: {path}\nThis project does not ship game data. Generate it \
         from your own installation (see docs/GENERATING_GAME_DATA.md), or copy the matching *.example.json for a \
         smoke test."
    ))
}

/// Bytes as lossy UTF-8, one BOM dropped, parsed as `JSON.parse` does.
fn read(path: &Path) -> Result<Option<Value>, LoadError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(LoadError::Unreadable(format!("{}: {e}", path.display()))),
    };
    let text = String::from_utf8_lossy(&bytes);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    service_kit::jsjson::parse(text)
        .map(Some)
        .map_err(|e| LoadError::Unreadable(format!("{}: {e}", path.display())))
}

impl GameDataRoot {
    pub fn new(backend: Option<PathBuf>, director: Option<PathBuf>) -> Self {
        Self {
            backend,
            director,
            cache: Arc::default(),
        }
    }

    /// The `game_data` section's `backend_dir` and `director_dir` (trimmed; blank is none).
    pub fn from_config(config: &ModuleConfig) -> Self {
        Self::new(
            folder(config.get("game_data", "backend_dir")),
            folder(config.get("game_data", "director_dir")),
        )
    }

    pub fn backend_file(&self, name: &str) -> Option<PathBuf> {
        self.backend.as_ref().map(|dir| dir.join(name))
    }

    pub fn director_file(&self, name: &str) -> Option<PathBuf> {
        self.director.as_ref().map(|dir| dir.join(name))
    }

    /// None when the file or folder is missing; a file that exists but fails is an error.
    pub fn try_load_backend(&self, name: &str) -> Result<Option<Value>, LoadError> {
        match self.backend_file(name) {
            Some(path) => read(&path),
            None => Ok(None),
        }
    }

    pub fn load_backend(&self, name: &str) -> Result<Value, LoadError> {
        let Some(path) = self.backend_file(name) else {
            return Err(missing(&format!("<no backend game data folder>/{name}")));
        };
        read(&path)?.ok_or_else(|| missing(&path.display().to_string()))
    }

    /// The first successful read is kept for the module's life (Node loads `GET /progression/config`'s file at
    /// import); a changed file needs a restart.
    pub fn load_backend_cached(&self, name: &str) -> Result<Value, LoadError> {
        let key = self.backend_file(name).unwrap_or_else(|| PathBuf::from(name));
        if let Some(value) = self.cache.lock().unwrap_or_else(PoisonError::into_inner).get(&key) {
            return Ok(value.clone());
        }
        let value = self.load_backend(name)?;
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, value.clone());
        Ok(value)
    }

    /// Optional read of a Director file.
    pub fn try_load_director(&self, name: &str) -> Result<Option<Value>, LoadError> {
        match self.director_file(name) {
            Some(path) => read(&path),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loader() {
        let dir = tempfile::tempdir().unwrap();
        let root = GameDataRoot::new(Some(dir.path().to_path_buf()), None);
        std::fs::write(
            dir.path().join("a.json"),
            "\u{feff}{\"n\":1.0,\"big\":12345678901234567890}",
        )
        .unwrap();
        let value = root.load_backend("a.json").unwrap();
        assert_eq!(
            service_kit::jsjson::stringify(&value),
            r#"{"n":1,"big":12345678901234567000}"#
        );
        assert_eq!(root.try_load_backend("none.json"), Ok(None));
        assert!(matches!(root.load_backend("none.json"), Err(LoadError::Missing(_))));
        std::fs::write(dir.path().join("bad.json"), "{").unwrap();
        assert!(matches!(
            root.try_load_backend("bad.json"),
            Err(LoadError::Unreadable(_))
        ));
        let bare = GameDataRoot::default();
        let Err(LoadError::Missing(text)) = bare.load_backend("x.json") else {
            panic!("no folder is missing");
        };
        assert!(
            text.starts_with("[game-data] Required game data file is missing: <no backend game data folder>/x.json\n")
        );
        assert_eq!(bare.try_load_backend("x.json"), Ok(None));

        std::fs::write(dir.path().join("c.json"), "[1]").unwrap();
        assert_eq!(root.load_backend_cached("c.json").unwrap(), serde_json::json!([1]));
        std::fs::write(dir.path().join("c.json"), "[2]").unwrap();
        assert_eq!(root.load_backend_cached("c.json").unwrap(), serde_json::json!([1]));
        assert_eq!(root.load_backend("c.json").unwrap(), serde_json::json!([2]));
    }

    #[test]
    fn director_lists() {
        assert_eq!(director_files(false).len(), 5);
        assert_eq!(director_files(true)[6], "arena_elite_matchmaker_hunts_new.json");
        let config = ModuleConfig {
            sections: [(
                "game_data".to_owned(),
                [
                    ("backend_dir".to_owned(), Value::from("  ")),
                    ("director_dir".to_owned(), Value::from(" /d ")),
                ]
                .into(),
            )]
            .into(),
            ..ModuleConfig::default()
        };
        let root = GameDataRoot::from_config(&config);
        assert_eq!(root.backend, None);
        assert_eq!(root.director_file("x"), Some(PathBuf::from("/d").join("x")));
    }
}
