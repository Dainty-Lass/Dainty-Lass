// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The launcher's folders, and the move from the folders older launchers used (spec §2.7.3).

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

/// The `userData` folder's name inside the OS config folder.
pub const APP_FOLDER: &str = "DaintyLass Launcher";
/// The older launcher's `userData` folder, a sibling of [`APP_FOLDER`].
pub const LEGACY_APP_FOLDER: &str = "Dauntless Reforged Revival Launcher";
/// The launcher's folder in the documents folder.
pub const DOCUMENTS_FOLDER: &str = "DaintyLass";
/// The older launcher's folder in the documents folder.
pub const LEGACY_DOCUMENTS_FOLDER: &str = "Dauntless Reforged Revival";
/// The argument that points the launcher at a profile folder.
pub const USER_DATA_DIR_ARG: &str = "--user-data-dir";
/// The environment variable read when [`USER_DATA_DIR_ARG`] is absent.
pub const DEV_PROFILE_VAR: &str = "DAINTY_DEV_PROFILE";
/// Left in an older folder once its files were copied over, so later starts skip it.
pub const MIGRATED_MARKER: &str = ".moved-to-dainty-lass";

/// The launcher's folders on this PC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// Settings, keys, logs and caches.
    pub user_data: PathBuf,
    /// The player's documents folder; the key recovery copies live under it.
    pub documents: PathBuf,
    /// The player's home folder.
    pub home: PathBuf,
}

/// The OS folders [`Paths`] is resolved from, so tests can supply their own.
#[derive(Debug, Clone, Default)]
pub struct OsFolders {
    /// The home folder.
    pub home: Option<PathBuf>,
    /// Roaming AppData on Windows; `$XDG_CONFIG_HOME` or `~/.config` elsewhere.
    pub config: Option<PathBuf>,
    /// The OS documents folder.
    pub documents: Option<PathBuf>,
}

impl OsFolders {
    /// This PC's folders.
    #[must_use]
    pub fn current() -> Self {
        Self {
            home: dirs::home_dir(),
            config: dirs::config_dir(),
            documents: dirs::document_dir(),
        }
    }
}

/// The non-empty value of `--user-data-dir <dir>` or `--user-data-dir=<dir>` among `args`.
#[must_use]
pub fn user_data_dir_arg(args: &[OsString]) -> Option<PathBuf> {
    let prefix = format!("{USER_DATA_DIR_ARG}=");
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let Some(text) = arg.to_str() else { continue };
        if text == USER_DATA_DIR_ARG {
            if let Some(value) = iter.next().filter(|v| !v.is_empty()) {
                return Some(PathBuf::from(value));
            }
        } else if let Some(value) = text.strip_prefix(&prefix).filter(|v| !v.is_empty()) {
            return Some(PathBuf::from(value));
        }
    }
    None
}

impl Paths {
    /// This process's folders: its arguments, [`DEV_PROFILE_VAR`] and the OS folders. None without a home folder.
    #[must_use]
    pub fn resolve() -> Option<Self> {
        let args: Vec<OsString> = std::env::args_os().collect();
        Self::resolve_from(&args, std::env::var_os(DEV_PROFILE_VAR), &OsFolders::current())
    }

    /// The folders for the given arguments, profile variable and OS folders. None without a home folder.
    #[must_use]
    pub fn resolve_from(args: &[OsString], dev_profile: Option<OsString>, os: &OsFolders) -> Option<Self> {
        let home = os.home.clone().filter(|h| !h.as_os_str().is_empty())?;
        let profile = user_data_dir_arg(args).or_else(|| dev_profile.filter(|v| !v.is_empty()).map(PathBuf::from));
        if let Some(profile) = profile {
            return Some(Self {
                user_data: profile.join("User Data"),
                documents: profile.join("Documents"),
                home,
            });
        }
        let config = os.config.clone().unwrap_or_else(|| home.join(".config"));
        let documents = os.documents.clone().unwrap_or_else(|| home.join("Documents"));
        Some(Self {
            user_data: config.join(APP_FOLDER),
            documents,
            home,
        })
    }

    /// `<userData>/keys`.
    #[must_use]
    pub fn keys(&self) -> PathBuf {
        self.user_data.join("keys")
    }

    /// `<userData>/logs`.
    #[must_use]
    pub fn logs(&self) -> PathBuf {
        self.user_data.join("logs")
    }

    /// `<documents>/DaintyLass/Account Keys`, where key recovery copies go.
    #[must_use]
    pub fn recovery(&self) -> PathBuf {
        self.documents.join(DOCUMENTS_FOLDER).join("Account Keys")
    }

    /// Brings the older launcher's folders over, run on every start. Each entry is (from, to, outcome); only
    /// folders not yet brought over are listed. A failure is the caller's to log: it must not stop the start.
    pub fn migrate_legacy(&self) -> Vec<(PathBuf, PathBuf, io::Result<Migrated>)> {
        let mut out = Vec::new();
        // A profile folder (`User Data`) has no older sibling to bring over.
        if self.user_data.file_name().is_some_and(|n| n == APP_FOLDER)
            && let Some(parent) = self.user_data.parent()
        {
            let legacy = parent.join(LEGACY_APP_FOLDER);
            let outcome = migrate_folder(&legacy, &self.user_data);
            out.push((legacy, self.user_data.clone(), outcome));
        }
        let legacy_docs = self.documents.join(LEGACY_DOCUMENTS_FOLDER);
        let current = self.documents.join(DOCUMENTS_FOLDER);
        let outcome = migrate_folder(&legacy_docs, &current);
        out.push((legacy_docs, current, outcome));
        out.retain(|(_, _, outcome)| !matches!(outcome, Ok(Migrated::Nothing)));
        out
    }
}

/// What [`migrate_folder`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Migrated {
    /// There was no older folder, or it was brought over on an earlier start.
    Nothing,
    /// The older folder was renamed to the new name.
    Renamed,
    /// The rename failed, so the older folder was copied; it stays where it was.
    Copied,
    /// The new folder existed, and this many files it lacked were copied in.
    Merged(usize),
}

/// Moves `legacy` to `current` when `current` doesn't exist (a rename, else a copy), or else copies in the files
/// `current` lacks, recursively, never overwriting one. A folder that stays behind is marked
/// ([`MIGRATED_MARKER`]) so the copy runs once, not on every start; it is kept, since it may hold keys.
pub fn migrate_folder(legacy: &Path, current: &Path) -> io::Result<Migrated> {
    let marker = legacy.join(MIGRATED_MARKER);
    if !legacy.is_dir() || std::fs::symlink_metadata(&marker).is_ok() {
        return Ok(Migrated::Nothing);
    }
    let done = if std::fs::symlink_metadata(current).is_err() {
        if std::fs::rename(legacy, current).is_ok() {
            return Ok(Migrated::Renamed);
        }
        copy_missing(legacy, current)?;
        Migrated::Copied
    } else {
        Migrated::Merged(copy_missing(legacy, current)?)
    };
    // An unwritable older folder is copied again next start, which overwrites nothing.
    let _ = std::fs::write(&marker, current.display().to_string());
    Ok(done)
}

/// Copies every file under `from` that `to` lacks; returns how many. Links are skipped, not followed.
fn copy_missing(from: &Path, to: &Path) -> io::Result<usize> {
    std::fs::create_dir_all(to)?;
    let mut copied = 0;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copied += copy_missing(&entry.path(), &target)?;
        } else if kind.is_file() && std::fs::symlink_metadata(&target).is_err() {
            std::fs::copy(entry.path(), &target)?;
            copied += 1;
        }
    }
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(home: &Path) -> OsFolders {
        OsFolders {
            home: Some(home.to_path_buf()),
            config: Some(home.join("cfg")),
            documents: Some(home.join("docs")),
        }
    }

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn default_folders_and_children() {
        let home = PathBuf::from("/home/p");
        let paths = Paths::resolve_from(&args(&["launcher"]), None, &os(&home)).unwrap();
        assert_eq!(paths.user_data, home.join("cfg").join(APP_FOLDER));
        assert_eq!(paths.documents, home.join("docs"));
        assert_eq!(paths.keys(), paths.user_data.join("keys"));
        assert_eq!(paths.logs(), paths.user_data.join("logs"));
        assert_eq!(
            paths.recovery(),
            home.join("docs").join("DaintyLass").join("Account Keys")
        );
    }

    #[test]
    fn missing_os_folders_fall_back_to_home() {
        let home = PathBuf::from("/home/p");
        let folders = OsFolders {
            home: Some(home.clone()),
            ..OsFolders::default()
        };
        let paths = Paths::resolve_from(&[], None, &folders).unwrap();
        assert_eq!(paths.user_data, home.join(".config").join(APP_FOLDER));
        assert_eq!(paths.documents, home.join("Documents"));
        assert_eq!(Paths::resolve_from(&[], None, &OsFolders::default()), None);
    }

    #[test]
    fn profile_from_argument_or_variable() {
        let home = PathBuf::from("/home/p");
        for list in [
            &["x", "--user-data-dir", "/prof"][..],
            &["x", "--user-data-dir=/prof"][..],
        ] {
            let paths = Paths::resolve_from(&args(list), Some("/env".into()), &os(&home)).unwrap();
            assert_eq!(paths.user_data, Path::new("/prof").join("User Data"));
            assert_eq!(paths.documents, Path::new("/prof").join("Documents"));
        }
        // An empty argument falls through to the variable, and an empty variable to the OS folders.
        for list in [
            &["x", "--user-data-dir="][..],
            &["x", "--user-data-dir", ""][..],
            &["x", "--user-data-dir"][..],
        ] {
            let paths = Paths::resolve_from(&args(list), Some("/env".into()), &os(&home)).unwrap();
            assert_eq!(paths.user_data, Path::new("/env").join("User Data"));
        }
        let paths = Paths::resolve_from(&args(&["x"]), Some(OsString::new()), &os(&home)).unwrap();
        assert_eq!(paths.user_data, home.join("cfg").join(APP_FOLDER));
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn legacy_folders_are_renamed_when_new_ones_are_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let os = os(tmp.path());
        let legacy = tmp.path().join("cfg").join(LEGACY_APP_FOLDER);
        write(&legacy.join("settings.json"), "{}");
        write(&legacy.join("keys").join("a.key"), "k");
        let legacy_docs = tmp.path().join("docs").join(LEGACY_DOCUMENTS_FOLDER);
        write(&legacy_docs.join("Account Keys").join("backup.txt"), "b");

        let paths = Paths::resolve_from(&[], None, &os).unwrap();
        let report = paths.migrate_legacy();
        let outcomes: Vec<Migrated> = report.into_iter().map(|(_, _, r)| r.unwrap()).collect();
        assert_eq!(outcomes, [Migrated::Renamed, Migrated::Renamed]);
        assert_eq!(read(&paths.user_data.join("settings.json")), "{}");
        assert_eq!(read(&paths.keys().join("a.key")), "k");
        assert_eq!(read(&paths.recovery().join("backup.txt")), "b");
        assert!(!legacy.exists());

        // Nothing left to bring on the next start.
        assert!(paths.migrate_legacy().is_empty());
    }

    #[test]
    fn existing_folders_get_only_missing_files() {
        let tmp = tempfile::tempdir().unwrap();
        let os = os(tmp.path());
        let paths = Paths::resolve_from(&[], None, &os).unwrap();
        let legacy = tmp.path().join("cfg").join(LEGACY_APP_FOLDER);
        write(&legacy.join("settings.json"), "old");
        write(&legacy.join("keys").join("a.key"), "old-a");
        write(&legacy.join("keys").join("b.key"), "old-b");
        write(&paths.user_data.join("settings.json"), "new");
        write(&paths.keys().join("a.key"), "new-a");

        let report = paths.migrate_legacy();
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].2.as_ref().unwrap(), &Migrated::Merged(1));
        assert_eq!(read(&paths.user_data.join("settings.json")), "new");
        assert_eq!(read(&paths.keys().join("a.key")), "new-a");
        assert_eq!(read(&paths.keys().join("b.key")), "old-b");
        assert!(legacy.join("keys").join("b.key").exists());

        // Marked, so the next start neither walks it again nor logs a move.
        assert!(legacy.join(MIGRATED_MARKER).is_file());
        assert!(paths.migrate_legacy().is_empty());
        assert!(!paths.user_data.join(MIGRATED_MARKER).exists());
    }

    #[test]
    fn profile_folders_are_not_migrated_from_a_sibling() {
        let tmp = tempfile::tempdir().unwrap();
        let profile = tmp.path().join("prof");
        write(&profile.join(LEGACY_APP_FOLDER).join("settings.json"), "{}");
        let paths = Paths::resolve_from(
            &args(&["--user-data-dir", profile.to_str().unwrap()]),
            None,
            &os(tmp.path()),
        )
        .unwrap();
        assert!(paths.migrate_legacy().is_empty());
        assert!(!paths.user_data.exists());
    }

    #[test]
    fn copy_fallback_and_missing_legacy() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            migrate_folder(&tmp.path().join("none"), &tmp.path().join("new")).unwrap(),
            Migrated::Nothing
        );
        let legacy = tmp.path().join("old");
        write(&legacy.join("deep").join("f.txt"), "f");
        assert_eq!(copy_missing(&legacy, &tmp.path().join("copy")).unwrap(), 1);
        assert_eq!(read(&tmp.path().join("copy").join("deep").join("f.txt")), "f");
    }
}
