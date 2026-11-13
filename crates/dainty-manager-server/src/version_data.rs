// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The 1.14.7 version data and facts the manager's bundle ships (RD27, port plan item 9): nothing is compiled in,
//! and the host kit reads the files from the workspace (`data/1.14.7/`, host.toml's `version_data` and
//! `version_facts`), so they are copied there from beside the executable before anything that needs them runs.

use std::path::{Path, PathBuf};

use dainty_host::hostconfig::{
    DEFAULT_VERSION_DATA, DEFAULT_VERSION_FACTS, HostConfig, load_host_config, version_data_path, version_facts_path,
};
use dainty_host::paths::Paths;

/// The Linux packages' resource folder beside `bin/`, as Tauri lays out deb, rpm and AppImage.
const LINUX_RESOURCES: &str = "../lib/Dainty Lass Manager";

/// Each shipped file: its default path, whether host.toml names its own, and where the host kit reads it.
type Shipped = (
    &'static str,
    fn(&HostConfig) -> bool,
    fn(&Paths, Option<&HostConfig>) -> PathBuf,
);

const FILES: [Shipped; 2] = [
    (DEFAULT_VERSION_DATA, |c| c.version_data.is_some(), version_data_path),
    (DEFAULT_VERSION_FACTS, |c| c.version_facts.is_some(), version_facts_path),
];

/// Where the bundle put `name`, from the executable's folder: beside it on Windows, Tauri's resource folder on Linux.
pub fn shipped(exe_dir: &Path, name: &str) -> Option<PathBuf> {
    [exe_dir.to_path_buf(), exe_dir.join(LINUX_RESOURCES)]
        .into_iter()
        .map(|dir| dir.join(name))
        .find(|file| file.is_file())
}

/// Copies `shipped` to the workspace's `name` when host.toml names no file of its own and the copy there differs,
/// so an updated manager brings its data with it. A host's own file is left alone.
fn install_one(paths: &Paths, config: Option<&HostConfig>, file: &Shipped, shipped: &Path) -> Result<(), String> {
    let (name, named, path) = file;
    if config.is_some_and(named) {
        return Ok(());
    }
    let target = path(paths, None);
    if target != paths.repo.join(name) {
        // A checkout's own copy is what it reads
        return Ok(());
    }
    let bytes = std::fs::read(shipped).map_err(|e| format!("could not read {}: {e}", shipped.display()))?;
    if std::fs::read(&target).is_ok_and(|current| current == bytes) {
        return Ok(());
    }
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    std::fs::write(&target, bytes).map_err(|e| format!("could not write {}: {e}", target.display()))
}

/// [`install_one`] for each file the bundle in `exe_dir` ships.
pub fn install(paths: &Paths, exe_dir: &Path) -> Result<(), String> {
    let config = load_host_config(paths).ok().flatten();
    for file in &FILES {
        if let Some(found) = shipped(exe_dir, file.0) {
            install_one(paths, config.as_ref(), file, &found)?;
        }
    }
    Ok(())
}

/// [`install`] from this executable's bundle; a build without one (a test, `cargo run`) installs nothing.
pub fn install_shipped(paths: &Paths) -> Result<(), String> {
    let exe = std::env::current_exe().ok();
    match exe.as_deref().and_then(Path::parent) {
        Some(dir) => install(paths, dir),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, tempfile::TempDir) {
        let ws = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let exe = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        for name in [DEFAULT_VERSION_DATA, DEFAULT_VERSION_FACTS] {
            let file = exe.path().join(name);
            std::fs::create_dir_all(file.parent().unwrap_or(exe.path())).unwrap_or_else(|e| panic!("{e}"));
            std::fs::write(&file, b"{\"v\":1}").unwrap_or_else(|e| panic!("{e}"));
        }
        (ws, exe)
    }

    #[test]
    fn the_bundles_copies_are_found_and_installed_and_kept_current() {
        let (ws, exe) = setup();
        let facts = exe.path().join(DEFAULT_VERSION_FACTS);
        assert_eq!(shipped(exe.path(), DEFAULT_VERSION_FACTS), Some(facts.clone()));
        let paths = Paths::new(ws.path());
        install(&paths, exe.path()).unwrap_or_else(|e| panic!("{e}"));
        for name in [DEFAULT_VERSION_DATA, DEFAULT_VERSION_FACTS] {
            let target = ws.path().join(name);
            assert_eq!(std::fs::read(&target).ok().as_deref(), Some(&b"{\"v\":1}"[..]));
        }
        std::fs::write(&facts, b"{\"v\":2}").unwrap_or_else(|e| panic!("{e}"));
        install(&paths, exe.path()).unwrap_or_else(|e| panic!("{e}"));
        let target = ws.path().join(DEFAULT_VERSION_FACTS);
        assert_eq!(std::fs::read(&target).ok().as_deref(), Some(&b"{\"v\":2}"[..]));
    }

    #[test]
    fn nothing_is_shipped_beside_a_bare_executable() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(shipped(dir.path(), DEFAULT_VERSION_DATA), None);
    }
}
