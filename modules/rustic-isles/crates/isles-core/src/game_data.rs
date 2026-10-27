//! The game data root per module (RU3c's `GAME_DATA`, seam 4): every export is per version, so each module points at
//! its own folders. The files are exported from the host's own install (CatalogExporter) and never committed.

use std::path::PathBuf;

/// The backend's files (`ParadoxBackend/game-data`).
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

/// The Director's hunt tables (`ParadoxDirector/game-data`).
pub const DIRECTOR_FILES: [&str; 5] = [
    "player_hunts_table.json",
    "matchmaker_hunts_table.json",
    "arena_easy_matchmaker_hunts.json",
    "arena_hard_matchmaker_hunts.json",
    "arena_elite_matchmaker_hunts.json",
];

/// Where one module's exports are: Node's `PARADOX_GAME_DATA_DIR`, one per service.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GameDataRoot {
    pub backend: Option<PathBuf>,
    pub director: Option<PathBuf>,
}

impl GameDataRoot {
    /// A backend file; `None` when the module has no folder for it (Node fails such a route as a missing file).
    pub fn backend_file(&self, name: &str) -> Option<PathBuf> {
        self.backend.as_deref().map(|dir| dir.join(name))
    }

    pub fn director_file(&self, name: &str) -> Option<PathBuf> {
        self.director.as_deref().map(|dir| dir.join(name))
    }

    /// From the module's `[game_data]` config: blank is no folder.
    pub fn from_config(backend: Option<&str>, director: Option<&str>) -> Self {
        let dir = |text: Option<&str>| text.map(str::trim).filter(|t| !t.is_empty()).map(PathBuf::from);
        Self {
            backend: dir(backend),
            director: dir(director),
        }
    }
}
