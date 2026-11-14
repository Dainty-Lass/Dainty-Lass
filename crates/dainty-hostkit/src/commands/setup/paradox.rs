// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Where Rustic Isles' sources are in a checkout, and which of them are missing.

use std::path::PathBuf;

use crate::paths::Paths;

/// The generated SDK files a runtime build needs.
pub const SDK_FILES: [&str; 6] = [
    "SDK.hpp",
    "Assertions.inl",
    "NameCollisions.inl",
    "PropertyFixup.hpp",
    "UnrealContainers.hpp",
    "UtfN.hpp",
];

/// Where Rustic Isles' parts are in this server folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// The server folder.
    pub repo: PathBuf,
    /// Rustic Isles' 1.14.7 reference tree.
    pub paradox: PathBuf,
    /// `modules/rustic-isles`.
    pub isles: PathBuf,
    /// The reference backend.
    pub backend_dir: PathBuf,
    /// The reference Director.
    pub director_dir: PathBuf,
    /// The runtime loader's sources.
    pub loader_dir: PathBuf,
    /// The generated SDK header.
    pub sdk_header: PathBuf,
    /// The generated SDK folder.
    pub sdk_dir: PathBuf,
    /// CatalogExporter's sources.
    pub exporter_dir: PathBuf,
    /// The game exports.
    pub exports: PathBuf,
    /// The backend's game data.
    pub backend_game_data: PathBuf,
    /// The Director's game data.
    pub director_game_data: PathBuf,
    /// The backend's raw game data.
    pub raw_game_data: PathBuf,
}

impl Layout {
    /// The layout of `p`.
    #[must_use]
    pub fn new(p: &Paths) -> Self {
        Self {
            repo: p.repo.clone(),
            paradox: p.paradox.clone(),
            isles: p.isles.clone(),
            backend_dir: p.backend_dir.clone(),
            director_dir: p.director_dir.clone(),
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

/// The sources a checkout lacks, by their path in the checkout.
#[must_use]
pub fn missing_paradox(l: &Layout) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if !l.backend_dir.join("package.json").is_file() {
        missing.push("modules/rustic-isles/reference/1.14.7/ParadoxBackend");
    }
    if !l.director_dir.join("package.json").is_file() {
        missing.push("modules/rustic-isles/reference/1.14.7/ParadoxDirector");
    }
    if !l.loader_dir.join("Cargo.toml").is_file() {
        missing.push("native/runtime-loader");
    }
    if !l.exporter_dir.join("Cargo.toml").is_file() {
        missing.push("native/rustic-isles-exporter");
    }
    missing
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use std::fs;

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
        fs::create_dir_all(&layout.loader_dir).unwrap();
        fs::write(layout.loader_dir.join("Cargo.toml"), "{}").unwrap();
        fs::create_dir_all(&layout.exporter_dir).unwrap();
        fs::write(layout.exporter_dir.join("Cargo.toml"), "{}").unwrap();

        assert!(missing_paradox(&layout).is_empty());
    }
}
