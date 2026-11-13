//! Which runtime DLL in the loader folder is loaded.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Same list and order as the launcher's `RUNTIME_DLLS` (`crates/dainty-launcher-kit`); the two must not drift.
pub const RUNTIME_DLLS: [&str; 3] = ["Rustic-Isles.dll", "MysticParadox.dll", "MystPaxInternalServer.dll"];

/// Loaded before the choice whenever it is beside the loader: never a candidate (spec section 3, RT-5).
pub const MIT_RUNTIME_DLL: &str = "rustic-isles-runtime.dll";

/// The MIT runtime in `folder`, when it is a regular file there.
pub fn mit(folder: &Path) -> Option<PathBuf> {
    let path = folder.join(MIT_RUNTIME_DLL);
    std::fs::metadata(&path)
        .is_ok_and(|metadata| metadata.is_file())
        .then_some(path)
}

pub fn is_runtime_name(name: &str) -> bool {
    RUNTIME_DLLS.iter().any(|runtime| runtime.eq_ignore_ascii_case(name))
}

/// The newest runtime present; a tie goes to the canonical name, then to the later alias.
pub fn choose(folder: &Path) -> Option<PathBuf> {
    RUNTIME_DLLS
        .iter()
        .enumerate()
        .filter_map(|(index, name)| {
            let path = folder.join(name);
            let metadata = std::fs::metadata(&path).ok().filter(|metadata| metadata.is_file())?;
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let canonical = name.eq_ignore_ascii_case(RUNTIME_DLLS[0]);
            Some(((modified, canonical, index), path))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, path)| path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::time::Duration;

    const OLD: Duration = Duration::from_secs(1_700_000_000);
    const NEW: Duration = Duration::from_secs(1_800_000_000);

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rustic-isles-loader-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(dir: &Path, name: &str, modified: Duration) {
        let file = File::create(dir.join(name)).unwrap();
        file.set_modified(SystemTime::UNIX_EPOCH + modified).unwrap();
    }

    fn chosen(dir: &Path) -> Option<String> {
        let path = choose(dir);
        fs::remove_dir_all(dir).unwrap();
        path.map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
    }

    #[test]
    fn only_the_canonical_name() {
        let dir = folder("canonical");
        put(&dir, "Rustic-Isles.dll", OLD);
        assert_eq!(chosen(&dir).as_deref(), Some("Rustic-Isles.dll"));
    }

    #[test]
    fn only_each_alias() {
        for alias in ["MysticParadox.dll", "MystPaxInternalServer.dll"] {
            let dir = folder(alias);
            put(&dir, alias, OLD);
            assert_eq!(chosen(&dir).as_deref(), Some(alias));
        }
    }

    #[test]
    fn none_present() {
        let dir = folder("none");
        put(&dir, "UndauntedInternalServer.dll", NEW);
        assert_eq!(chosen(&dir), None);
    }

    #[test]
    fn a_newer_alias_beats_an_older_canonical() {
        let dir = folder("newer-alias");
        put(&dir, "Rustic-Isles.dll", OLD);
        put(&dir, "MysticParadox.dll", NEW);
        assert_eq!(chosen(&dir).as_deref(), Some("MysticParadox.dll"));
    }

    #[test]
    fn an_equal_time_goes_to_the_canonical() {
        let dir = folder("tie-canonical");
        put(&dir, "MysticParadox.dll", NEW);
        put(&dir, "Rustic-Isles.dll", NEW);
        put(&dir, "MystPaxInternalServer.dll", NEW);
        assert_eq!(chosen(&dir).as_deref(), Some("Rustic-Isles.dll"));
    }

    #[test]
    fn an_equal_time_between_aliases_goes_to_the_older_alias_name() {
        let dir = folder("tie-aliases");
        put(&dir, "MystPaxInternalServer.dll", NEW);
        put(&dir, "MysticParadox.dll", NEW);
        put(&dir, "Rustic-Isles.dll", OLD);
        assert_eq!(chosen(&dir).as_deref(), Some("MystPaxInternalServer.dll"));
    }

    #[test]
    fn a_folder_with_the_name_is_ignored() {
        let dir = folder("folder");
        fs::create_dir(dir.join("Rustic-Isles.dll")).unwrap();
        put(&dir, "MysticParadox.dll", OLD);
        assert_eq!(chosen(&dir).as_deref(), Some("MysticParadox.dll"));
    }

    #[test]
    fn the_mit_runtime_is_found_beside_and_never_chosen() {
        let dir = folder("mit");
        assert_eq!(mit(&dir), None);
        fs::create_dir(dir.join(MIT_RUNTIME_DLL)).unwrap();
        assert_eq!(mit(&dir), None);
        fs::remove_dir(dir.join(MIT_RUNTIME_DLL)).unwrap();
        put(&dir, MIT_RUNTIME_DLL, NEW);
        put(&dir, "Rustic-Isles.dll", OLD);
        assert_eq!(mit(&dir), Some(dir.join(MIT_RUNTIME_DLL)));
        assert!(!is_runtime_name(MIT_RUNTIME_DLL));
        assert_eq!(chosen(&dir).as_deref(), Some("Rustic-Isles.dll"));
    }

    #[test]
    fn runtime_names_match_in_any_ascii_case() {
        assert!(is_runtime_name("rustic-isles.DLL"));
        assert!(is_runtime_name("MYSTPAXINTERNALSERVER.dll"));
        assert!(is_runtime_name("mysticparadox.dll"));
        assert!(!is_runtime_name("UndauntedInternalServer.dll"));
        assert!(!is_runtime_name("sub\\Rustic-Isles.dll"));
    }
}
