//! Dauntless Revived's 1.4.4 game download (`CONTENT_GAME=legacy`; RD5 reversed), from DR's content
//! server at 24b7624: the game manifest, the game folder's start-up check, the request paths, HTTP
//! ranges and `content verify`. The routes themselves are in `app/game_files.rs`.

pub mod gamedir;
pub mod manifest;
pub mod paths;
pub mod range;
pub mod verify;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::json;
use sha2::{Digest, Sha256};

use crate::log;
use gamedir::FileState;
use manifest::{EXPECTED_BUILD, Manifest, ManifestFile};

/// The game a legacy content server hands out: DR's `Deps` minus what Reforged's already has.
pub struct LegacyGame {
    pub index: HashMap<String, ManifestFile>,
    pub game_dir: PathBuf,
    pub states: Mutex<HashMap<String, FileState>>,
    pub manifest_text: String,
    pub manifest_etag: String,
}

fn state_json(state: Option<&FileState>) -> serde_json::Value {
    match state {
        Some(FileState::SizeMismatch { actual }) => json!({"status": "size_mismatch", "actual": actual}),
        Some(other) => json!({"status": other.status()}),
        None => serde_json::Value::Null,
    }
}

impl LegacyGame {
    /// A game served from `game_dir` by `manifest` with the files' states as the check found them.
    pub fn new(manifest: &Manifest, game_dir: PathBuf, states: HashMap<String, FileState>) -> Self {
        let manifest_text = manifest::to_json(manifest);
        let digest: String = Sha256::digest(manifest_text.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        Self {
            index: manifest::index(manifest),
            game_dir,
            states: Mutex::new(states),
            manifest_text,
            manifest_etag: format!("\"{digest}\""),
        }
    }

    /// DR's start-up (`prepare`): the manifest and the size check of the game folder. Files that fail
    /// are not served; none at all is the reason it can't start.
    pub fn prepare(game_dir: &Path, manifest_path: Option<&Path>) -> Result<Self, String> {
        let shown = manifest_path.map_or_else(|| "built-in".to_owned(), |p| p.display().to_string());
        let manifest = manifest::load(manifest_path).map_err(|e| format!("Manifest {shown}: {e}"))?;
        if manifest.build != EXPECTED_BUILD {
            log::warn(
                "manifest is for a different build than this server was written for",
                json!({"build": manifest.build}),
            );
        }
        let report = gamedir::check(game_dir, &manifest);
        log::info(
            "game folder checked (sizes only; content verify hashes everything)",
            json!({
                "gameDir": game_dir.display().to_string(),
                "build": manifest.build,
                "files": manifest.files.len(),
                "ok": report.ok,
                "missing": report.missing.len(),
                "mismatched": report.mismatched.len(),
                "bytesOk": report.bytes_ok,
                "totalBytes": manifest.total_bytes,
            }),
        );
        if report.ok == 0 {
            return Err(format!(
                "None of the {} manifest files is in {}. Is CONTENT_GAME_DIR the folder that contains Archon\\?",
                manifest.files.len(),
                game_dir.display()
            ));
        }
        for path in report.missing.iter().take(20) {
            log::warn("missing game file (not served)", json!({"path": path}));
        }
        for path in report.mismatched.iter().take(20) {
            log::warn(
                "game file has the wrong size or is not a regular file (not served)",
                json!({"path": path, "state": state_json(report.states.get(path))}),
            );
        }
        if report.missing.len() + report.mismatched.len() > 40 {
            log::warn(
                "more problems than listed; run content verify for the full list",
                json!({}),
            );
        }
        Ok(Self::new(&manifest, game_dir.to_path_buf(), report.states))
    }
}
