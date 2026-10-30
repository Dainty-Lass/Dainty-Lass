// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::HostError;
use crate::paths::Paths;

pub const SDK_FILES: [&str; 6] = [
    "SDK.hpp",
    "Assertions.inl",
    "NameCollisions.inl",
    "PropertyFixup.hpp",
    "UnrealContainers.hpp",
    "UtfN.hpp",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub repo: PathBuf,
    pub paradox: PathBuf,
    pub isles: PathBuf,
    pub revived: PathBuf,
    pub old_upstream: PathBuf,
    pub pre_paradox: PathBuf,
    pub pre_native: PathBuf,
    pub backend_dir: PathBuf,
    pub director_dir: PathBuf,
    pub runtime_source: PathBuf,
    pub sdk_header: PathBuf,
    pub sdk_dir: PathBuf,
    pub exporter_dir: PathBuf,
    pub exports: PathBuf,
    pub backend_game_data: PathBuf,
    pub director_game_data: PathBuf,
}

impl Layout {
    #[must_use]
    pub fn new(p: &Paths) -> Self {
        Self {
            repo: p.repo.clone(),
            paradox: p.paradox.clone(),
            isles: p.isles.clone(),
            revived: p.revived.clone(),
            old_upstream: p.old_upstream.clone(),
            pre_paradox: p.repo.join("paradox"),
            pre_native: p.repo.join("native"),
            backend_dir: p.backend_dir.clone(),
            director_dir: p.director_dir.clone(),
            runtime_source: p.runtime_source.clone(),
            sdk_header: p.sdk_header.clone(),
            sdk_dir: p.sdk_dir.clone(),
            exporter_dir: p.exporter_dir.clone(),
            exports: p.exports.clone(),
            backend_game_data: p.backend_game_data.clone(),
            director_game_data: p.director_game_data.clone(),
        }
    }
}

#[must_use]
pub fn missing_paradox(l: &Layout) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if !l.backend_dir.join("package.json").is_file() {
        missing.push("ParadoxBackend");
    }
    if !l.director_dir.join("package.json").is_file() {
        missing.push("ParadoxDirector");
    }
    if !l.runtime_source.is_dir() {
        missing.push("MysticParadox");
    }
    if !l.exporter_dir.is_dir() {
        missing.push("CatalogExporter");
    }
    missing
}

#[must_use]
pub fn old_workspace_items(l: &Layout) -> Vec<(PathBuf, PathBuf)> {
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

#[must_use]
pub fn pre_submodule_items(l: &Layout) -> Vec<(PathBuf, PathBuf)> {
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

#[must_use]
pub fn old_folders(l: &Layout) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in [
        l.old_upstream.clone(),
        l.pre_paradox.clone(),
        l.pre_native.join("runtime"),
        l.pre_native.join("catalog-exporter"),
    ] {
        if dir.exists() {
            out.push(dir);
        }
    }
    out
}

pub fn migrate_old_workspace(l: &Layout, repo: &Path) -> Result<Vec<String>, HostError> {
    let mut moved = Vec::new();
    let pairs = pre_submodule_items(l)
        .into_iter()
        .chain(old_workspace_items(l));
    for (src, dst) in pairs {
        if src == dst || !src.exists() || dst.exists() {
            continue;
        }
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|e| HostError::io(parent, e))?;
        }
        fs::rename(&src, &dst).map_err(|e| HostError::io(&src, e))?;
        let rel_src = src.strip_prefix(repo).unwrap_or(&src);
        let rel_dst = dst.strip_prefix(repo).unwrap_or(&dst);
        moved.push(format!("{} -> {}", rel_src.display(), rel_dst.display()));
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn missing_paradox_detects_absent_files_and_clears_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path());
        let layout = Layout::new(&paths);
        assert_eq!(missing_paradox(&layout).len(), 4);

        fs::create_dir_all(&layout.backend_dir).unwrap();
        fs::write(layout.backend_dir.join("package.json"), "{}").unwrap();
        fs::create_dir_all(&layout.director_dir).unwrap();
        fs::write(layout.director_dir.join("package.json"), "{}").unwrap();
        fs::create_dir_all(&layout.runtime_source).unwrap();
        fs::create_dir_all(&layout.exporter_dir).unwrap();

        assert!(missing_paradox(&layout).is_empty());
    }

    #[test]
    fn migrate_old_workspace_moves_legacy_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path());
        let layout = Layout::new(&paths);

        let legacy_exports = layout.old_upstream.join("Items_Analysis");
        fs::create_dir_all(&legacy_exports).unwrap();
        fs::write(legacy_exports.join("items.json"), "[]").unwrap();

        let moved = migrate_old_workspace(&layout, &paths.repo).unwrap();
        assert_eq!(moved.len(), 1);
        assert!(layout.exports.join("items.json").is_file());
    }
}
