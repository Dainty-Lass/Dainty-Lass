//! Where the launcher keeps its files (row L26). Tauri's own app data folder would be
//! `…/io.github.freezamotoqueiro.dauntlessreforgedrevival`; it is not used. The folders use the DaintyLass
//! product names (`DaintyLass Launcher` and `Documents/DaintyLass/Account Keys`), and automatically migrate
//! existing `Dauntless Reforged Revival Launcher` and `Documents/Dauntless Reforged Revival` folders on first start.

use std::path::{Path, PathBuf};

/// The launcher's `userData` folder under the roaming app data folder (Windows) or
/// `$XDG_CONFIG_HOME` / `~/.config` (Linux).
pub const PRODUCT_NAME: &str = "DaintyLass Launcher";

/// The pre-rename 1.5.x Electron launcher's `userData` folder, migrated on first start.
pub const LEGACY_PRODUCT_NAME: &str = "Dauntless Reforged Revival Launcher";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub user_data: PathBuf,
    pub documents: PathBuf,
    pub home: PathBuf,
}

/// Recursively copies files and directories from `src` into `dst` without overwriting any file that already
/// exists in `dst`.
fn merge_missing_dir(src: &Path, dst: &Path) {
    let Ok(entries) = std::fs::read_dir(src) else {
        return;
    };
    let _ = std::fs::create_dir_all(dst);
    for entry in entries.flatten() {
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_dir() {
            merge_missing_dir(&src_path, &dst_path);
        } else if ft.is_file() && !dst_path.exists() {
            let _ = std::fs::copy(&src_path, &dst_path);
        }
    }
}

/// Moves `legacy` to `target` when `target` does not exist yet (falling back to copying if rename fails),
/// or merges any missing files from `legacy` into `target` when both exist.
fn migrate_one_dir(legacy: &Path, target: &Path) {
    if !legacy.is_dir() || legacy == target {
        return;
    }
    if !target.exists() {
        if let Some(parent) = target.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::rename(legacy, target).is_ok() {
            return;
        }
    }
    merge_missing_dir(legacy, target);
}

/// Migrates legacy `Dauntless Reforged Revival Launcher` user-data and `Documents/Dauntless Reforged Revival`
/// recovery folders into `DaintyLass Launcher` and `Documents/DaintyLass` on first start without losing or
/// overwriting newer files.
pub fn migrate_legacy_folders(user_data: &Path, documents: Option<&Path>) {
    if user_data.file_name().is_some_and(|name| name == PRODUCT_NAME)
        && let Some(parent) = user_data.parent()
    {
        let legacy_user_data = parent.join(LEGACY_PRODUCT_NAME);
        migrate_one_dir(&legacy_user_data, user_data);
    }
    if let Some(docs) = documents {
        let legacy_recovery = docs.join(LEGACY_RECOVERY_FOLDER);
        let new_recovery = docs.join(RECOVERY_FOLDER);
        migrate_one_dir(&legacy_recovery, &new_recovery);
    }
}

/// Parses `--user-data-dir <dir>` or `--user-data-dir=<dir>` from command-line arguments so multiple
/// launcher instances can run side by side with isolated profiles.
pub fn user_data_dir_from_args<S: AsRef<str>>(args: &[S]) -> Option<PathBuf> {
    let mut iter = args.iter().map(AsRef::as_ref);
    while let Some(arg) = iter.next() {
        if arg == "--user-data-dir"
            && let Some(dir) = iter.next()
            && !dir.is_empty()
        {
            return Some(PathBuf::from(dir));
        }
        if let Some(dir) = arg.strip_prefix("--user-data-dir=")
            && !dir.is_empty()
        {
            return Some(PathBuf::from(dir));
        }
    }
    None
}

impl Paths {
    /// From the operating system's folders, migrating any pre-rename folders on first start.
    pub fn from_system() -> Option<Self> {
        let home = dirs::home_dir()?;
        let args: Vec<String> = std::env::args().collect();
        if let Some(dir) = user_data_dir_from_args(&args) {
            let user_data = dir.join("User Data");
            let documents = dir.join("Documents");
            migrate_legacy_folders(&user_data, Some(&documents));
            return Some(Self {
                user_data,
                documents,
                home,
            });
        }
        // Throwaway profile when DAINTY_DEV_PROFILE is set, so a developer's or host's real settings and keys stay untouched
        if let Some(dir) = std::env::var_os("DAINTY_DEV_PROFILE").map(PathBuf::from) {
            let user_data = dir.join("User Data");
            let documents = dir.join("Documents");
            migrate_legacy_folders(&user_data, Some(&documents));
            return Some(Self {
                user_data,
                documents,
                home,
            });
        }
        let user_data = dirs::config_dir()?.join(PRODUCT_NAME);
        let documents = dirs::document_dir().unwrap_or_else(|| home.join("Documents"));
        migrate_legacy_folders(&user_data, Some(&documents));
        Some(Self {
            user_data,
            documents,
            home,
        })
    }

    pub fn keys_dir(&self) -> PathBuf {
        self.user_data.join("keys")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.user_data.join("logs")
    }

    /// The plain account key copies the player finds again after reinstalling the system:
    /// `Documents/DaintyLass/Account Keys/<server id>.txt`.
    pub fn recovery_dir(&self) -> PathBuf {
        self.documents.join(RECOVERY_FOLDER).join("Account Keys")
    }
}

/// The launcher's folder in Documents
pub const RECOVERY_FOLDER: &str = "DaintyLass";

/// The pre-rename 1.5.x folder in Documents, migrated on first start
pub const LEGACY_RECOVERY_FOLDER: &str = "Dauntless Reforged Revival";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_electron_folders() {
        let paths = Paths::from_system().unwrap();
        assert!(paths.user_data.ends_with(PRODUCT_NAME));
        assert_eq!(paths.keys_dir(), paths.user_data.join("keys"));
        assert_eq!(paths.logs_dir(), paths.user_data.join("logs"));
        assert!(
            paths
                .recovery_dir()
                .ends_with(std::path::Path::new(RECOVERY_FOLDER).join("Account Keys"))
        );
        if cfg!(windows) {
            let appdata = PathBuf::from(std::env::var_os("APPDATA").unwrap());
            assert_eq!(paths.user_data, appdata.join(PRODUCT_NAME));
        }
    }

    #[test]
    fn migrates_legacy_user_data_and_documents_folders_without_losing_or_overwriting_data() {
        let temp = tempfile::tempdir().unwrap();
        let config_root = temp.path().join("Roaming");
        let docs_root = temp.path().join("Documents");

        let legacy_user_data = config_root.join(LEGACY_PRODUCT_NAME);
        let new_user_data = config_root.join(PRODUCT_NAME);
        std::fs::create_dir_all(legacy_user_data.join("keys")).unwrap();
        std::fs::create_dir_all(legacy_user_data.join("Server/.workspace/config")).unwrap();
        std::fs::write(legacy_user_data.join("settings.json"), b"{\"language\":\"en\"}").unwrap();
        std::fs::write(legacy_user_data.join("keys/abc.key2"), b"encrypted-key-1").unwrap();
        std::fs::write(
            legacy_user_data.join("Server/.workspace/config/host.toml"),
            b"mode = \"private\"\n",
        )
        .unwrap();

        let legacy_keys = docs_root.join(LEGACY_RECOVERY_FOLDER).join("Account Keys");
        std::fs::create_dir_all(&legacy_keys).unwrap();
        std::fs::write(legacy_keys.join("abc.txt"), b"RRK_legacy_1").unwrap();

        // 1. Fresh migration moves legacy folders into DaintyLass names
        migrate_legacy_folders(&new_user_data, Some(&docs_root));
        assert!(new_user_data.join("settings.json").is_file());
        assert_eq!(
            std::fs::read_to_string(new_user_data.join("keys/abc.key2")).unwrap(),
            "encrypted-key-1"
        );
        assert!(new_user_data.join("Server/.workspace/config/host.toml").is_file());
        let new_keys = docs_root.join(RECOVERY_FOLDER).join("Account Keys");
        assert_eq!(
            std::fs::read_to_string(new_keys.join("abc.txt")).unwrap(),
            "RRK_legacy_1"
        );

        // 2. If both legacy and new folders exist, missing files merge without overwriting newer files
        std::fs::create_dir_all(legacy_user_data.join("keys")).unwrap();
        std::fs::write(legacy_user_data.join("keys/abc.key2"), b"stale-key").unwrap();
        std::fs::write(legacy_user_data.join("keys/def.key2"), b"encrypted-key-2").unwrap();
        std::fs::create_dir_all(&legacy_keys).unwrap();
        std::fs::write(legacy_keys.join("abc.txt"), b"RRK_stale").unwrap();
        std::fs::write(legacy_keys.join("def.txt"), b"RRK_legacy_2").unwrap();

        migrate_legacy_folders(&new_user_data, Some(&docs_root));
        assert_eq!(
            std::fs::read_to_string(new_user_data.join("keys/abc.key2")).unwrap(),
            "encrypted-key-1"
        );
        assert_eq!(
            std::fs::read_to_string(new_user_data.join("keys/def.key2")).unwrap(),
            "encrypted-key-2"
        );
        assert_eq!(
            std::fs::read_to_string(new_keys.join("abc.txt")).unwrap(),
            "RRK_legacy_1"
        );
        assert_eq!(
            std::fs::read_to_string(new_keys.join("def.txt")).unwrap(),
            "RRK_legacy_2"
        );
    }
}
