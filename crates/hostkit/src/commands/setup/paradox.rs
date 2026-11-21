// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

use std::path::PathBuf;

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
    pub backend_dir: PathBuf,
    pub director_dir: PathBuf,
    pub runtime_source: PathBuf,
    pub loader_dir: PathBuf,
    pub sdk_header: PathBuf,
    pub sdk_dir: PathBuf,
    pub exporter_dir: PathBuf,
    pub exports: PathBuf,
    pub backend_game_data: PathBuf,
    pub director_game_data: PathBuf,
    pub raw_game_data: PathBuf,
}

impl Layout {
    #[must_use]
    pub fn new(p: &Paths) -> Self {
        Self {
            repo: p.repo.clone(),
            paradox: p.paradox.clone(),
            isles: p.isles.clone(),
            backend_dir: p.backend_dir.clone(),
            director_dir: p.director_dir.clone(),
            runtime_source: p.runtime_source.clone(),
            loader_dir: p.loader_dir.clone(),
            sdk_header: p.sdk_header.clone(),
            sdk_dir: p.sdk_dir.clone(),
            exporter_dir: p.exporter_dir.clone(),
            exports: p.exports.clone(),
            backend_game_data: p.backend_game_data.clone(),
            director_game_data: p.director_game_data.clone(),
            raw_game_data: p.raw_game_data.clone(),
        }
    }
}

#[must_use]
pub fn missing_paradox(l: &Layout) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if !l.backend_dir.join("package.json").is_file() {
        missing.push("modules/rustic-isles/reference/1.14.7/ParadoxBackend");
    }
    if !l.director_dir.join("package.json").is_file() {
        missing.push("modules/rustic-isles/reference/1.14.7/ParadoxDirector");
    }
    if !l.runtime_source.join("Rustic-Isles.vcxproj").is_file() {
        missing.push("native/runtime");
    }
    if !l.loader_dir.join("Cargo.toml").is_file() {
        missing.push("native/runtime-loader");
    }
    if !l.exporter_dir.join("CatalogExporter.vcxproj").is_file() {
        missing.push("native/catalog-exporter");
    }
    missing
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::fs;

    use super::*;

    #[test]
    fn missing_paradox_detects_absent_files_and_clears_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path());
        let layout = Layout::new(&paths);
        assert_eq!(missing_paradox(&layout).len(), 5);

        fs::create_dir_all(&layout.backend_dir).unwrap();
        fs::write(layout.backend_dir.join("package.json"), "{}").unwrap();
        fs::create_dir_all(&layout.director_dir).unwrap();
        fs::write(layout.director_dir.join("package.json"), "{}").unwrap();
        fs::create_dir_all(&layout.runtime_source).unwrap();
        fs::write(layout.runtime_source.join("Rustic-Isles.vcxproj"), "{}").unwrap();
        fs::create_dir_all(&layout.loader_dir).unwrap();
        fs::write(layout.loader_dir.join("Cargo.toml"), "{}").unwrap();
        fs::create_dir_all(&layout.exporter_dir).unwrap();
        fs::write(layout.exporter_dir.join("CatalogExporter.vcxproj"), "{}").unwrap();

        assert!(missing_paradox(&layout).is_empty());
    }
}
