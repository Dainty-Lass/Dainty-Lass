// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The 1.14.7 version data the manager's bundle ships (RD27): nothing is compiled in, and the host kit reads the file
//! from the workspace (`data/1.14.7/version_data.json`, host.toml's `version_data`), so it is copied there from
//! beside the executable before anything that needs it runs.

use std::path::{Path, PathBuf};

use dainty_host::hostconfig::{DEFAULT_VERSION_DATA, load_host_config, version_data_path};
use dainty_host::paths::Paths;

/// The Linux packages' resource folder beside `bin/`, as Tauri lays out deb, rpm and AppImage.
const LINUX_RESOURCES: &str = "../lib/Dainty Lass Manager";

/// Where the bundle put the file, from the executable's folder: beside it on Windows, Tauri's resource folder on Linux.
pub fn shipped(exe_dir: &Path) -> Option<PathBuf> {
    [exe_dir.to_path_buf(), exe_dir.join(LINUX_RESOURCES)]
        .into_iter()
        .map(|dir| dir.join(DEFAULT_VERSION_DATA))
        .find(|file| file.is_file())
}

/// Copies `shipped` to the workspace's default path when host.toml names no file of its own and the copy there
/// differs, so an updated manager brings its data with it. A host's own `version_data` is left alone.
pub fn install(paths: &Paths, shipped: &Path) -> Result<(), String> {
    let config = load_host_config(paths).ok().flatten();
    if config.as_ref().is_some_and(|c| c.version_data.is_some()) {
        return Ok(());
    }
    let target = version_data_path(paths, None);
    if target != paths.repo.join(DEFAULT_VERSION_DATA) {
        // A checkout's own copy (restricted/'s) is what it reads
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

/// [`install`] from this executable's bundle; a build without one (a test, `cargo run`) installs nothing.
pub fn install_shipped(paths: &Paths) -> Result<(), String> {
    let exe = std::env::current_exe().ok();
    match exe.as_deref().and_then(Path::parent).and_then(shipped) {
        Some(file) => install(paths, &file),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, tempfile::TempDir, PathBuf) {
        let ws = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let exe = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let file = exe.path().join(DEFAULT_VERSION_DATA);
        std::fs::create_dir_all(file.parent().unwrap_or(exe.path())).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(&file, b"{\"v\":1}").unwrap_or_else(|e| panic!("{e}"));
        (ws, exe, file)
    }

    #[test]
    fn the_bundles_copy_is_found_and_installed_and_kept_current() {
        let (ws, exe, file) = setup();
        assert_eq!(shipped(exe.path()), Some(file.clone()));
        let paths = Paths::new(ws.path());
        install(&paths, &file).unwrap_or_else(|e| panic!("{e}"));
        let target = ws.path().join(DEFAULT_VERSION_DATA);
        assert_eq!(std::fs::read(&target).ok().as_deref(), Some(&b"{\"v\":1}"[..]));
        std::fs::write(&file, b"{\"v\":2}").unwrap_or_else(|e| panic!("{e}"));
        install(&paths, &file).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(std::fs::read(&target).ok().as_deref(), Some(&b"{\"v\":2}"[..]));
    }

    #[test]
    fn nothing_is_shipped_beside_a_bare_executable() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(shipped(dir.path()), None);
    }
}
