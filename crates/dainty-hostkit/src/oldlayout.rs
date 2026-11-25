// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! A host's game data moved out of the retired Node reference tree (RB15), `modules/rustic-isles/reference/1.14.7`,
//! into `modules/rustic-isles/game-data/1.14.7`. Run by `setup`, which `update` reruns.

use std::path::{Path, PathBuf};

use crate::error::HostError;
use crate::paths::Paths;

/// The reference tree's folder the game data was in before RB15.
#[must_use]
pub fn old_root(p: &Paths) -> PathBuf {
    p.isles.join("reference").join("1.14.7")
}

/// Each old folder and where it goes now.
fn moves(p: &Paths) -> [(PathBuf, &Path); 4] {
    let old = old_root(p);
    [
        (old.join("ParadoxBackend").join("game-data"), &p.backend_game_data),
        (old.join("ParadoxDirector").join("game-data"), &p.director_game_data),
        (old.join("Items_Analysis"), &p.exports),
        (old.join("DauntlessEndpointDocumentation"), &p.store_captures),
    ]
}

/// Moves what the old folders hold into the new ones, never over a file already there; returns the lines to print.
/// Nothing to say once the old folders are gone or hold only what the new ones already have.
pub fn move_old_game_data(p: &Paths) -> Result<Vec<String>, HostError> {
    let mut lines = Vec::new();
    for (from, to) in moves(p) {
        if !from.is_dir() {
            continue;
        }
        if !to.exists() {
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).map_err(|e| HostError::io(parent, e))?;
            }
            std::fs::rename(&from, to).map_err(|e| HostError::io(&from, e))?;
            lines.push(format!("- moved {} -> {}", from.display(), to.display()));
            continue;
        }
        // A checkout or a release already has the new folder, holding the examples
        let (mut moved, mut kept) = (0, 0);
        for entry in std::fs::read_dir(&from).map_err(|e| HostError::io(&from, e))? {
            let entry = entry.map_err(|e| HostError::io(&from, e))?;
            let target = to.join(entry.file_name());
            // An empty folder setup made is no reason to leave the old one behind
            if target.is_dir() {
                let _ = std::fs::remove_dir(&target);
            }
            if target.exists() {
                kept += 1;
                continue;
            }
            std::fs::rename(entry.path(), &target).map_err(|e| HostError::io(entry.path(), e))?;
            moved += 1;
        }
        if moved > 0 {
            lines.push(format!(
                "- moved {moved} file(s) from {} to {}",
                from.display(),
                to.display()
            ));
            if kept > 0 {
                lines.push(format!(
                    "- {kept} file(s) in {} were already in {} and are left where they were",
                    from.display(),
                    to.display()
                ));
            }
        }
        let _ = std::fs::remove_dir(&from);
    }
    if !lines.is_empty() {
        lines.push(format!(
            "- {} is no longer used: delete it once you've checked what moved",
            old_root(p).display()
        ));
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;

    fn touch(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn moves_a_server_trees_old_game_data_once_and_overwrites_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let p = Paths::new(tmp.path());
        let old = old_root(&p);
        touch(&old.join("ParadoxBackend/game-data/platinum_store.json"), "real");
        touch(
            &old.join("ParadoxBackend/game-data/raw/progression_config.source.json"),
            "raw",
        );
        touch(&old.join("ParadoxBackend/game-data/slayers_path.json"), "old");
        touch(&old.join("ParadoxBackend/package.json"), "{}");
        touch(&old.join("ParadoxDirector/game-data/player_hunts_table.json"), "hunts");
        touch(&old.join("Items_Analysis/catalog_1_14_7.jsonl"), "catalog");
        // The release's examples and a file the host already has in the new place
        touch(&p.backend_game_data.join("platinum_store.example.json"), "example");
        touch(&p.backend_game_data.join("slayers_path.json"), "new");
        std::fs::create_dir_all(&p.raw_game_data).unwrap();

        let lines = move_old_game_data(&p).unwrap();
        assert_eq!(
            std::fs::read_to_string(p.backend_game_data.join("platinum_store.json")).unwrap(),
            "real"
        );
        assert!(p.raw_game_data.join("progression_config.source.json").is_file());
        assert_eq!(
            std::fs::read_to_string(p.backend_game_data.join("slayers_path.json")).unwrap(),
            "new"
        );
        assert!(old.join("ParadoxBackend/game-data/slayers_path.json").is_file());
        assert!(p.director_game_data.join("player_hunts_table.json").is_file());
        assert!(p.exports.join("catalog_1_14_7.jsonl").is_file());
        assert!(!old.join("Items_Analysis").exists());
        assert!(
            lines
                .iter()
                .any(|l| l.contains("1 file(s) in") && l.contains("left where they were"))
        );
        assert!(lines.last().unwrap().contains("no longer used"));

        // Once: what is left is already in the new place
        assert!(move_old_game_data(&p).unwrap().is_empty());
    }

    #[test]
    fn a_checkouts_old_folders_are_under_restricted() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("restricted/modules/rustic-isles")).unwrap();
        let p = Paths::new(tmp.path());
        touch(
            &tmp.path()
                .join("restricted/modules/rustic-isles/reference/1.14.7/DauntlessEndpointDocumentation/a.json"),
            "capture",
        );
        let lines = move_old_game_data(&p).unwrap();
        assert_eq!(lines.len(), 2);
        assert!(
            tmp.path()
                .join("restricted/modules/rustic-isles/game-data/1.14.7/captures/a.json")
                .is_file()
        );
    }

    #[test]
    fn nothing_to_move_says_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(move_old_game_data(&Paths::new(tmp.path())).unwrap().is_empty());
    }
}
