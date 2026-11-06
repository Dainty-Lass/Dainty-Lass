//! Where the launcher keeps its files (row L26), over launcher-kit's `platform::paths` (RL3). Tauri's own app
//! data folder would be `…/io.github.freezamotoqueiro.dauntlessreforgedrevival`; it is not used. Pre-rename
//! folders are migrated on first start.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use launcher_kit::platform::paths as kit;

pub use kit::Paths;

pub const PRODUCT_NAME: &str = kit::APP_FOLDER;
pub const LEGACY_PRODUCT_NAME: &str = kit::LEGACY_APP_FOLDER;
pub const RECOVERY_FOLDER: &str = kit::DOCUMENTS_FOLDER;
pub const LEGACY_RECOVERY_FOLDER: &str = kit::LEGACY_DOCUMENTS_FOLDER;

/// The shell's names for the MIT `Paths`; `from_system` also migrates pre-rename folders.
pub trait PathsExt: Sized {
    fn from_system() -> Option<Self>;
    fn keys_dir(&self) -> PathBuf;
    fn logs_dir(&self) -> PathBuf;
    fn recovery_dir(&self) -> PathBuf;
}

impl PathsExt for Paths {
    fn from_system() -> Option<Self> {
        let paths = Self::resolve()?;
        migrate_legacy_folders(&paths.user_data, Some(&paths.documents));
        Some(paths)
    }

    fn keys_dir(&self) -> PathBuf {
        self.keys()
    }

    fn logs_dir(&self) -> PathBuf {
        self.logs()
    }

    fn recovery_dir(&self) -> PathBuf {
        self.recovery()
    }
}

/// Moves or merges the pre-rename `userData` and Documents folders; Documents only when given.
pub fn migrate_legacy_folders(user_data: &Path, documents: Option<&Path>) {
    if user_data.file_name().is_some_and(|name| name == PRODUCT_NAME)
        && let Some(parent) = user_data.parent()
    {
        let _ = kit::migrate_folder(&parent.join(LEGACY_PRODUCT_NAME), user_data);
    }
    if let Some(docs) = documents {
        let _ = kit::migrate_folder(&docs.join(LEGACY_RECOVERY_FOLDER), &docs.join(RECOVERY_FOLDER));
    }
}

/// `--user-data-dir <dir>` or `--user-data-dir=<dir>`, for side-by-side profiles.
pub fn user_data_dir_from_args<S: AsRef<str>>(args: &[S]) -> Option<PathBuf> {
    let args: Vec<OsString> = args.iter().map(|arg| OsString::from(arg.as_ref())).collect();
    kit::user_data_dir_arg(&args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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
