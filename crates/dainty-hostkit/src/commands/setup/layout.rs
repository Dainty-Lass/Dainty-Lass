// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Where Rustic Isles' parts are in a server folder, and which of a checkout's sources are missing.

use std::path::PathBuf;

use crate::paths::Paths;

/// The generated SDK files a CatalogExporter build reads.
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
    /// `modules/rustic-isles`.
    pub isles: PathBuf,
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
            isles: p.isles.clone(),
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
pub fn missing_sources(l: &Layout) -> Vec<&'static str> {
    let mut missing = Vec::new();
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
    fn missing_sources_detects_absent_files_and_clears_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path());
        let layout = Layout::new(&paths);
        assert_eq!(missing_sources(&layout).len(), 2);

        fs::create_dir_all(&layout.loader_dir).unwrap();
        fs::write(layout.loader_dir.join("Cargo.toml"), "{}").unwrap();
        fs::create_dir_all(&layout.exporter_dir).unwrap();
        fs::write(layout.exporter_dir.join("Cargo.toml"), "{}").unwrap();

        assert!(missing_sources(&layout).is_empty());
    }
}
