// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Removing a game the launcher installed: the whole folder when the install made it, otherwise only the files the
//! manifest names and the launcher's own runtime and loader files, so nothing the player kept there goes. A link
//! (a symlink or a junction) is removed as itself and never followed, and the deletes are permanent: there is no
//! Recycle Bin without a shell API the launcher does not otherwise need.

use std::fs::{self, Metadata};
use std::io;
use std::path::{Path, PathBuf};

use dainty_epic::manifest::InstallPath;
use dainty_protocol::manifest::MIT_RUNTIME_DLL;

use crate::core::pins::WIN64_RELATIVE_DIR;
use crate::net::runtime_files::{ASIDE_MARK, DOWNLOAD_SUFFIX, RETIRED_RUNTIME_DLLS};
use crate::platform::loader_session::{ASIDE_SUFFIX, EXPORTER_DLL, LOADER_DLL, LOADER_INI};

/// What goes from a folder the install did not make.
pub enum Scope<'a> {
    /// The install made the folder: all of it goes.
    Whole,
    /// The folder was there before: only these files go.
    Listed(&'a [InstallPath]),
}

/// Why a removal stopped.
#[derive(Debug)]
pub enum RemovalError {
    /// The folder is gone, or is not a folder.
    Missing,
    /// The folder is itself a link: what it points at was never installed there.
    FolderIsLink,
    /// `path` could not be read or deleted.
    Io {
        /// Where.
        path: PathBuf,
        /// Why.
        error: io::Error,
    },
}

impl std::fmt::Display for RemovalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => f.write_str("the folder is missing"),
            Self::FolderIsLink => f.write_str("the folder is a link"),
            Self::Io { path, error } => write!(f, "{}: {error}", path.display()),
        }
    }
}

impl std::error::Error for RemovalError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    File,
    Link { dir: bool },
}

/// What a removal deletes, found without changing anything.
#[derive(Debug)]
pub struct Removal {
    folder: PathBuf,
    whole: bool,
    entries: Vec<(PathBuf, Kind)>,
    /// Deepest first.
    dirs: Vec<PathBuf>,
    bytes: u64,
}

/// The launcher's own files in Win64 beside the game's (RD9): the runtimes, the loader and its list.
fn own_file(name: &str) -> bool {
    name == MIT_RUNTIME_DLL
        || RETIRED_RUNTIME_DLLS.contains(&name)
        || [LOADER_DLL, LOADER_INI, EXPORTER_DLL].contains(&name)
        || name.ends_with(DOWNLOAD_SUFFIX)
        || name.ends_with(ASIDE_SUFFIX)
        || name.contains(ASIDE_MARK)
}

/// A symlink, a junction, or any other directory reparse point Windows would follow.
fn is_link(meta: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
        let attributes = meta.file_attributes();
        if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 && attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}

fn link_kind(meta: &Metadata) -> Kind {
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTypeExt;
        let kind = meta.file_type();
        Kind::Link {
            dir: kind.is_symlink_dir() || (!kind.is_symlink_file() && meta.is_dir()),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = meta;
        Kind::Link { dir: false }
    }
}

/// The entry at `path`, read without following it; none when it is gone.
fn look(path: &Path) -> Result<Option<Metadata>, RemovalError> {
    match fs::symlink_metadata(path) {
        Ok(meta) => Ok(Some(meta)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(RemovalError::Io {
            path: path.to_path_buf(),
            error,
        }),
    }
}

impl Removal {
    /// Finds what removing `folder` under `scope` deletes, and the bytes that frees.
    pub fn plan(folder: &Path, scope: &Scope<'_>) -> Result<Self, RemovalError> {
        let meta = look(folder)?.ok_or(RemovalError::Missing)?;
        if is_link(&meta) {
            return Err(RemovalError::FolderIsLink);
        }
        if !meta.is_dir() {
            return Err(RemovalError::Missing);
        }
        let mut removal = Self {
            folder: folder.to_path_buf(),
            whole: matches!(scope, Scope::Whole),
            entries: Vec::new(),
            dirs: Vec::new(),
            bytes: 0,
        };
        match scope {
            Scope::Whole => removal.walk(folder)?,
            Scope::Listed(files) => {
                for file in *files {
                    removal.listed(file.components())?;
                }
                let win64: Vec<&str> = WIN64_RELATIVE_DIR.split('/').collect();
                if let Some(dir) = removal.real_dir(&win64)? {
                    let names = fs::read_dir(&dir).map_err(|error| RemovalError::Io {
                        path: dir.clone(),
                        error,
                    })?;
                    for name in names.flatten() {
                        let name = name.file_name().to_string_lossy().into_owned();
                        if own_file(&name) {
                            let mut parts = win64.clone();
                            parts.push(&name);
                            removal.listed(parts.into_iter())?;
                        }
                    }
                }
                removal.entries.sort();
                removal.entries.dedup();
                removal.dirs.sort();
                removal.dirs.dedup();
                // Deepest first, so a parent is tried only once its children are gone
                removal
                    .dirs
                    .sort_by_key(|dir| std::cmp::Reverse(dir.components().count()));
            }
        }
        Ok(removal)
    }

    /// The folder, when every component under it is a real folder rather than a link.
    fn real_dir(&self, parts: &[&str]) -> Result<Option<PathBuf>, RemovalError> {
        let mut path = self.folder.clone();
        for part in parts {
            path.push(part);
            match look(&path)? {
                Some(meta) if meta.is_dir() && !is_link(&meta) => {}
                _ => return Ok(None),
            }
        }
        Ok(Some(path))
    }

    /// One file the manifest names; skipped when it is gone or a folder on its way is a link.
    fn listed<'a>(&mut self, parts: impl Iterator<Item = &'a str>) -> Result<(), RemovalError> {
        let parts: Vec<&str> = parts.collect();
        let Some((name, parents)) = parts.split_last() else {
            return Ok(());
        };
        let Some(dir) = self.real_dir(parents)? else {
            return Ok(());
        };
        let path = dir.join(name);
        let Some(meta) = look(&path)? else {
            return Ok(());
        };
        if is_link(&meta) {
            self.entries.push((path, link_kind(&meta)));
        } else if meta.is_file() {
            self.bytes += meta.len();
            self.entries.push((path, Kind::File));
        } else {
            return Ok(());
        }
        let mut parent = dir;
        while parent != self.folder {
            self.dirs.push(parent.clone());
            if !parent.pop() {
                break;
            }
        }
        Ok(())
    }

    /// Everything under `dir`, children before their folder.
    fn walk(&mut self, dir: &Path) -> Result<(), RemovalError> {
        let io = |error| RemovalError::Io {
            path: dir.to_path_buf(),
            error,
        };
        for entry in fs::read_dir(dir).map_err(io)? {
            let path = entry.map_err(io)?.path();
            let Some(meta) = look(&path)? else { continue };
            if is_link(&meta) {
                self.entries.push((path, link_kind(&meta)));
            } else if meta.is_dir() {
                self.walk(&path)?;
            } else {
                self.bytes += meta.len();
                self.entries.push((path, Kind::File));
            }
        }
        if dir != self.folder {
            self.dirs.push(dir.to_path_buf());
        }
        Ok(())
    }

    /// The folder removed from.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Whether the folder itself goes.
    pub fn whole(&self) -> bool {
        self.whole
    }

    /// Bytes the deleted files take.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Files and links deleted.
    pub fn count(&self) -> usize {
        self.entries.len()
    }

    /// Deletes what the plan found. It stops at the first file that will not go, so a retry finds the rest; a
    /// folder the player's own files keep is left.
    pub fn remove(&self) -> Result<(), RemovalError> {
        for (path, kind) in &self.entries {
            let io = |error| RemovalError::Io {
                path: path.clone(),
                error,
            };
            // Read again: what was a file may since have been swapped for a link
            let Some(meta) = look(path)? else { continue };
            let kind = if is_link(&meta) { link_kind(&meta) } else { *kind };
            match kind {
                Kind::Link { dir: true } => fs::remove_dir(path).map_err(io)?,
                Kind::Link { dir: false } => fs::remove_file(path).map_err(io)?,
                Kind::File if meta.is_dir() => {}
                Kind::File => remove_file(path, &meta).map_err(io)?,
            }
        }
        for dir in &self.dirs {
            remove_empty(dir, self.whole)?;
        }
        if self.whole {
            remove_empty(&self.folder, true)?;
        }
        Ok(())
    }
}

/// Clears read-only first: Windows will not delete a read-only file.
fn remove_file(path: &Path, meta: &Metadata) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied && meta.permissions().readonly() => {
            let mut permissions = meta.permissions();
            #[allow(clippy::permissions_set_readonly_false, reason = "only to delete it")]
            permissions.set_readonly(false);
            fs::set_permissions(path, permissions)?;
            fs::remove_file(path)
        }
        other => other,
    }
}

/// Removes `dir` when it is empty; with `required`, one that is not is an error.
fn remove_empty(dir: &Path, required: bool) -> Result<(), RemovalError> {
    match fs::remove_dir(dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) if !required => Ok(()),
        Err(error) => Err(RemovalError::Io {
            path: dir.to_path_buf(),
            error,
        }),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

    use super::*;

    fn write(root: &Path, name: &str, bytes: usize) {
        let path = InstallPath::new(name).unwrap().under(root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![7u8; bytes]).unwrap();
    }

    fn paths(names: &[&str]) -> Vec<InstallPath> {
        names.iter().map(|name| InstallPath::new(name).unwrap()).collect()
    }

    const EXE: &str = "Archon/Binaries/Win64/Dauntless-Win64-Shipping.exe";
    const PAK: &str = "Archon/Content/Paks/a.pak";

    /// A link at `link` to the folder `target`: a junction on Windows (no rights needed), a symlink elsewhere.
    fn dir_link(target: &Path, link: &Path) {
        #[cfg(windows)]
        {
            let status = std::process::Command::new("cmd")
                .arg("/C")
                .arg("mklink")
                .arg("/J")
                .arg(link)
                .arg(target)
                .stdout(std::process::Stdio::null())
                .status()
                .unwrap();
            assert!(status.success());
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
    }

    #[test]
    fn a_folder_the_install_made_goes_whole() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("1.14.7");
        write(&folder, EXE, 10);
        write(&folder, PAK, 30);
        write(&folder, "Archon/Saved/Logs/Archon.log", 5);
        let removal = Removal::plan(&folder, &Scope::Whole).unwrap();
        assert_eq!((removal.bytes(), removal.count(), removal.whole()), (45, 3, true));
        removal.remove().unwrap();
        assert!(!folder.exists());
        assert!(temp.path().exists());
    }

    #[test]
    fn a_folder_the_player_had_keeps_everything_the_manifest_does_not_name() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("games");
        write(&folder, EXE, 10);
        write(&folder, PAK, 30);
        write(&folder, "Archon/Content/Paks/mine.pak", 4);
        write(&folder, "Archon/Saved/Config/Game.ini", 2);
        write(&folder, "notes.txt", 1);
        let win64 = folder.join("Archon/Binaries/Win64");
        for own in [
            MIT_RUNTIME_DLL,
            "winmm.dll",
            "mystic_loader.ini",
            "Rustic-Isles.dll.rr-old-17",
        ] {
            fs::write(win64.join(own), b"ours").unwrap();
        }
        fs::write(win64.join("steam_api64.dll"), b"theirs").unwrap();
        let files = paths(&[EXE, PAK, "Archon/Content/Movies/gone.bk2"]);
        let removal = Removal::plan(&folder, &Scope::Listed(&files)).unwrap();
        assert_eq!((removal.bytes(), removal.count(), removal.whole()), (56, 6, false));
        removal.remove().unwrap();
        let left = |name: &str| InstallPath::new(name).unwrap().under(&folder).exists();
        assert!(!left(EXE) && !left(PAK));
        assert!(left("Archon/Content/Paks/mine.pak"));
        assert!(left("Archon/Saved/Config/Game.ini"));
        assert!(left("notes.txt"));
        assert!(win64.join("steam_api64.dll").exists());
        assert!(!win64.join(MIT_RUNTIME_DLL).exists() && !win64.join("winmm.dll").exists());
        assert!(!win64.join("mystic_loader.ini").exists());
    }

    #[test]
    fn emptied_manifest_folders_go_and_the_picked_folder_stays() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("games");
        write(&folder, EXE, 1);
        write(&folder, PAK, 1);
        let files = paths(&[EXE, PAK]);
        Removal::plan(&folder, &Scope::Listed(&files))
            .unwrap()
            .remove()
            .unwrap();
        assert!(folder.is_dir());
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 0);
    }

    #[test]
    fn a_partial_install_removes_what_is_there() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("games");
        write(&folder, PAK, 8);
        let files = paths(&[EXE, PAK, "Archon/Content/Paks/b.pak"]);
        let removal = Removal::plan(&folder, &Scope::Listed(&files)).unwrap();
        assert_eq!((removal.bytes(), removal.count()), (8, 1));
        removal.remove().unwrap();
        assert!(!InstallPath::new(PAK).unwrap().under(&folder).exists());
        // An install that never got as far as making its folder has nothing to remove
        let missing = temp.path().join("never");
        assert!(matches!(
            Removal::plan(&missing, &Scope::Whole),
            Err(RemovalError::Missing)
        ));
    }

    #[test]
    fn read_only_files_go_too() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("1.14.7");
        write(&folder, EXE, 3);
        let exe = InstallPath::new(EXE).unwrap().under(&folder);
        let mut permissions = fs::metadata(&exe).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&exe, permissions).unwrap();
        let files = paths(&[EXE]);
        Removal::plan(&folder, &Scope::Listed(&files))
            .unwrap()
            .remove()
            .unwrap();
        assert!(!exe.exists());
    }

    #[test]
    fn a_link_inside_is_removed_and_never_followed() {
        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("outside");
        write(&outside, "Archon/Content/Paks/a.pak", 50);
        write(&outside, "precious.txt", 9);

        // A folder the install made, with a link to a folder outside it
        let made = temp.path().join("1.14.7");
        write(&made, EXE, 10);
        dir_link(&outside, &made.join("shortcut"));
        let removal = Removal::plan(&made, &Scope::Whole).unwrap();
        assert_eq!(removal.bytes(), 10, "nothing behind the link counts");
        removal.remove().unwrap();
        assert!(!made.exists());
        assert!(outside.join("precious.txt").exists());

        // A picked folder whose Archon links outside: the manifest's paths run through the link, so none go
        let picked = temp.path().join("games");
        fs::create_dir_all(&picked).unwrap();
        dir_link(&outside, &picked.join("Archon"));
        let files = paths(&[PAK]);
        let removal = Removal::plan(&picked, &Scope::Listed(&files)).unwrap();
        assert_eq!((removal.bytes(), removal.count()), (0, 0));
        removal.remove().unwrap();
        assert!(outside.join("Archon/Content/Paks/a.pak").exists());
    }

    #[test]
    fn a_folder_that_is_a_link_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("outside");
        write(&outside, EXE, 1);
        let link = temp.path().join("1.14.7");
        dir_link(&outside, &link);
        assert!(matches!(
            Removal::plan(&link, &Scope::Whole),
            Err(RemovalError::FolderIsLink)
        ));
        assert!(InstallPath::new(EXE).unwrap().under(&outside).exists());
    }
}
