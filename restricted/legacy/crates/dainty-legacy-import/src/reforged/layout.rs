// Ported from Dainty Lass's `rust/crates/dainty-hostkit/src/commands/setup/paradox.rs` and `paths.rs` (plan `fb46a4a`), MIT,
// Copyright (c) 2026 Silent Spud; LICENSE-MIT.txt there. Only the old-install half moved here.

//! Where an old checkout kept its files, and where Dainty Lass keeps them: the Node kit's `.workspace/Mystic-Paradox`
//! upstream clone and the pre-submodule `paradox/` and `native/` folders move under `modules/rustic-isles`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{ImportError, Result};
use crate::report::Report;

const SDK_FILES: [&str; 6] = [
    "SDK.hpp",
    "Assertions.inl",
    "NameCollisions.inl",
    "PropertyFixup.hpp",
    "UnrealContainers.hpp",
    "UtfN.hpp",
];

/// A checkout's paths, as Dainty Lass's `Paths::new` lays them out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// The checkout.
    pub repo: PathBuf,
    /// `.workspace`.
    pub workspace: PathBuf,
    /// `.workspace/config`.
    pub config: PathBuf,
    /// The Node kit's `host.json`.
    pub host_json: PathBuf,
    /// Dainty Lass's `host.toml`.
    pub host_toml: PathBuf,
    /// The Node kit's MongoDB data folder.
    pub mongo_data: PathBuf,
    /// The Node kit's `.workspace/Mystic-Paradox` upstream clone.
    pub old_upstream: PathBuf,
    /// The pre-submodule `paradox/` folder.
    pub pre_paradox: PathBuf,
    /// The pre-submodule `native/` folder.
    pub pre_native: PathBuf,
    /// Where the game runtime's source lives now.
    pub runtime_source: PathBuf,
    /// The runtime's generated SDK folder.
    pub sdk_dir: PathBuf,
    /// CatalogExporter's folder.
    pub exporter_dir: PathBuf,
    /// The game data exports (`Items_Analysis`).
    pub exports: PathBuf,
    /// The backend's game data folder.
    pub backend_game_data: PathBuf,
    /// The director's game data folder.
    pub director_game_data: PathBuf,
}

impl Layout {
    #[must_use]
    /// The layout of the checkout at `repo`.
    pub fn new(repo: &Path) -> Self {
        let repo = repo.to_path_buf();
        let workspace = repo.join(".workspace");
        let config = workspace.join("config");
        let isles = repo.join("modules").join("rustic-isles");
        let paradox = isles.join("reference").join("1.14.7");
        let runtime_source = isles.join("native").join("runtime");
        Self {
            host_json: config.join("host.json"),
            host_toml: config.join("host.toml"),
            mongo_data: workspace.join("mongodb"),
            old_upstream: workspace.join("Mystic-Paradox"),
            pre_paradox: repo.join("paradox"),
            pre_native: repo.join("native"),
            sdk_dir: runtime_source.join("SDK"),
            exporter_dir: isles.join("native").join("catalog-exporter"),
            exports: paradox.join("Items_Analysis"),
            backend_game_data: paradox.join("ParadoxBackend").join("game-data"),
            director_game_data: paradox.join("ParadoxDirector").join("game-data"),
            runtime_source,
            config,
            workspace,
            repo,
        }
    }
}

fn old_workspace_items(l: &Layout) -> Vec<(PathBuf, PathBuf)> {
    let mut items = Vec::new();
    let old_rt = l.old_upstream.join("MysticParadox");
    for name in SDK_FILES {
        items.push((old_rt.join(name), l.runtime_source.join(name)));
    }
    items.push((old_rt.join("SDK"), l.sdk_dir.clone()));
    items.push((old_rt.join("x64"), l.runtime_source.join("x64")));
    items.push((
        l.old_upstream.join("CatalogExporter").join("x64"),
        l.exporter_dir.join("x64"),
    ));
    items.push((l.old_upstream.join("Items_Analysis"), l.exports.clone()));
    items.push((
        l.old_upstream.join("ParadoxBackend").join("game-data"),
        l.backend_game_data.clone(),
    ));
    items.push((
        l.old_upstream.join("ParadoxDirector").join("game-data"),
        l.director_game_data.clone(),
    ));
    items
}

fn pre_submodule_items(l: &Layout) -> Vec<(PathBuf, PathBuf)> {
    let mut items = Vec::new();
    let pre_rt = l.pre_native.join("runtime");
    for name in SDK_FILES {
        items.push((pre_rt.join(name), l.runtime_source.join(name)));
    }
    items.push((pre_rt.join("SDK"), l.sdk_dir.clone()));
    items.push((pre_rt.join("x64"), l.runtime_source.join("x64")));
    items.push((
        l.pre_native.join("catalog-exporter").join("x64"),
        l.exporter_dir.join("x64"),
    ));
    items.push((l.pre_paradox.join("Items_Analysis"), l.exports.clone()));
    items.push((
        l.pre_paradox.join("ParadoxBackend").join("game-data"),
        l.backend_game_data.clone(),
    ));
    items.push((
        l.pre_paradox.join("ParadoxDirector").join("game-data"),
        l.director_game_data.clone(),
    ));
    items
}

/// The old folders still there once their contents moved, for the host to delete by hand.
#[must_use]
pub fn old_folders(l: &Layout) -> Vec<PathBuf> {
    [
        l.old_upstream.clone(),
        l.pre_paradox.clone(),
        l.pre_native.join("runtime"),
        l.pre_native.join("catalog-exporter"),
    ]
    .into_iter()
    .filter(|dir| dir.exists())
    .collect()
}

fn relative(path: &Path, repo: &Path) -> String {
    path.strip_prefix(repo).unwrap_or(path).display().to_string()
}

/// Moves what the old layouts kept (the generated SDK, built binaries, exported and game data) to where Dainty Lass
/// looks. A destination that already exists is left alone and reported; nothing is overwritten or deleted.
pub fn migrate_old_workspace(l: &Layout, dry_run: bool, report: &mut Report) -> Result<()> {
    let pairs = pre_submodule_items(l).into_iter().chain(old_workspace_items(l));
    for (src, dst) in pairs {
        if src == dst || !src.exists() {
            continue;
        }
        if dst.exists() {
            report.skip(
                "folder",
                relative(&src, &l.repo),
                format!("{} is already there", relative(&dst, &l.repo)),
            );
            continue;
        }
        if !dry_run {
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent).map_err(|e| ImportError::io(parent, e))?;
            }
            fs::rename(&src, &dst).map_err(|e| ImportError::io(&src, e))?;
        }
        report.count("folders moved");
        report.note(format!(
            "moved {} -> {}",
            relative(&src, &l.repo),
            relative(&dst, &l.repo)
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_old_workspace_moves_legacy_dirs_and_keeps_what_is_there() {
        let tmp = tempfile::tempdir().unwrap();
        let layout = Layout::new(tmp.path());

        let legacy_exports = layout.old_upstream.join("Items_Analysis");
        fs::create_dir_all(&legacy_exports).unwrap();
        fs::write(legacy_exports.join("items.json"), "[]").unwrap();
        let pre_sdk = layout.pre_native.join("runtime").join("SDK.hpp");
        fs::create_dir_all(pre_sdk.parent().unwrap()).unwrap();
        fs::write(&pre_sdk, "// sdk").unwrap();
        fs::create_dir_all(&layout.runtime_source).unwrap();
        fs::write(layout.runtime_source.join("SDK.hpp"), "// newer").unwrap();

        let mut dry = Report::new("reforged", true);
        migrate_old_workspace(&layout, true, &mut dry).unwrap();
        assert_eq!(dry.imported("folders moved"), 1);
        assert!(legacy_exports.is_dir(), "a dry run moves nothing");

        let mut report = Report::new("reforged", false);
        migrate_old_workspace(&layout, false, &mut report).unwrap();
        assert_eq!(report.imported("folders moved"), 1);
        assert!(layout.exports.join("items.json").is_file());
        assert_eq!(
            fs::read_to_string(layout.runtime_source.join("SDK.hpp")).unwrap(),
            "// newer"
        );
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(old_folders(&layout).len(), 2);
    }
}
