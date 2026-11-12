// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! How the runtime and the Host tools reach the game (RD9 as of 2026-11-12, `docs/launcher-injection.md`). On
//! Windows the launcher loads them into the game it starts suspended (`dainty-inject`). Under Proton the player
//! picks: the runtime loader (`winmm.dll`, the default) or the `dainty-inject.exe` helper, which Proton runs in
//! place of the game to do the same inside Wine.

use std::ffi::OsString;
use std::path::Path;

pub use dainty_inject::helper::HELPER_EXE;
pub use dainty_inject::{InjectDll, InjectError};

/// This build loads the DLLs into the game itself; under Proton the loader or the helper does.
pub const INJECTS: bool = cfg!(windows);

/// How the DLLs reach a game started under Proton.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtonLoad {
    /// The runtime loader beside the game, and `mystic_loader.ini` for the Host tools
    Loader,
    /// The `dainty-inject.exe` helper
    Helper,
}

/// Loads `dlls`, in order, into `child`, which this launcher just started suspended, then resumes it. The caller
/// kills the child on an error.
#[cfg(windows)]
pub fn load_and_resume(child: &tokio::process::Child, dlls: &[InjectDll]) -> Result<(), InjectError> {
    let (Some(pid), Some(handle)) = (child.id(), child.raw_handle()) else {
        return Err(InjectError::Process("the game already exited".to_owned()));
    };
    dainty_inject::load_and_resume(handle, pid, dlls)
}

/// Off Windows nothing is injected by the launcher itself.
#[cfg(not(windows))]
pub fn load_and_resume(_child: &tokio::process::Child, _dlls: &[InjectDll]) -> Result<(), InjectError> {
    Err(InjectError::Unsupported)
}

/// The helper's arguments for `dlls` and the game `exe`, in Wine's paths, followed by the game's arguments.
/// Each DLL is hashed now; the helper checks the hash again through a held handle before loading it.
pub fn helper_line(exe: &Path, dlls: &[InjectDll], game_args: &[OsString]) -> Result<Vec<OsString>, InjectError> {
    let wine = |path: &Path| dainty_inject::helper::wine_path(path).ok_or_else(|| InjectError::BadPath(path.into()));
    let dlls = dlls
        .iter()
        .map(|dll| Ok((wine(&dll.path)?, dll.sha256.clone())))
        .collect::<Result<Vec<_>, InjectError>>()?;
    Ok(dainty_inject::helper::command_line(&dlls, &wine(exe)?, game_args))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_windows_injects_itself() {
        assert_eq!(INJECTS, cfg!(windows));
    }

    #[test]
    fn the_helper_line_names_wine_paths_and_keeps_the_game_arguments() {
        let sha = "a".repeat(64);
        let dlls = [InjectDll {
            path: "/g/Win64/Rustic-Isles.dll".into(),
            sha256: sha.clone(),
        }];
        let line = helper_line(
            Path::new("/g/Win64/Dauntless-Win64-Shipping.exe"),
            &dlls,
            &["-NoEAC".into()],
        )
        .unwrap();
        let expected: Vec<OsString> = [
            "--dll",
            "Z:\\g\\Win64\\Rustic-Isles.dll",
            &sha,
            "--",
            "Z:\\g\\Win64\\Dauntless-Win64-Shipping.exe",
            "-NoEAC",
        ]
        .iter()
        .map(OsString::from)
        .collect();
        assert_eq!(line, expected);
        let relative = [InjectDll {
            path: "Rustic-Isles.dll".into(),
            sha256: sha,
        }];
        assert!(helper_line(Path::new("/g/x.exe"), &relative, &[]).is_err());
    }
}
