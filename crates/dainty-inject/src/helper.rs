// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `dainty-inject.exe`, which Proton runs in place of the game when a Linux player picks the helper over the
//! runtime loader: `dainty-inject.exe --dll <path> <sha256> [--dll ...] -- <game.exe> [game arguments]`. It starts
//! the game suspended inside the same Wine prefix, loads the DLLs as the Windows launcher does, resumes it, waits,
//! and exits with the game's code. The launcher builds the line with [`command_line`]; paths are Wine's (`Z:\...`).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::InjectDll;

/// The helper's file name, beside the launcher's other resources.
pub const HELPER_EXE: &str = "dainty-inject.exe";
/// Exit code: the command line is not the helper's.
pub const EXIT_BAD_ARGS: u32 = 0xD1A0_0001;
/// Exit code: the game could not be started.
pub const EXIT_START_FAILED: u32 = 0xD1A0_0002;
/// Exit code: a DLL was not loaded; the game was killed.
pub const EXIT_INJECT_FAILED: u32 = 0xD1A0_0003;
/// The most DLLs one start loads (the runtime and CatalogExporter).
const MAX_DLLS: usize = 4;

/// What the helper was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperArgs {
    /// Loaded in order before the game runs
    pub dlls: Vec<InjectDll>,
    /// The game's executable
    pub program: PathBuf,
    /// Its arguments, passed on untouched
    pub args: Vec<OsString>,
}

fn is_sha256(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The helper's own arguments (after its name), or `None` when they are not exactly its form.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Option<HelperArgs> {
    let mut args = args.into_iter();
    let mut dlls = Vec::new();
    loop {
        let flag = args.next()?;
        if flag == "--" {
            break;
        }
        if flag != "--dll" || dlls.len() == MAX_DLLS {
            return None;
        }
        let path = PathBuf::from(args.next()?);
        let sha256 = args.next()?.into_string().ok().filter(|s| is_sha256(s))?;
        dlls.push(InjectDll { path, sha256 });
    }
    let program = PathBuf::from(args.next()?);
    (!dlls.is_empty()).then(|| HelperArgs {
        dlls,
        program,
        args: args.collect(),
    })
}

/// `path` as Wine sees it: its `Z:` drive maps `/`. None for a relative or non-Unicode path.
#[must_use]
pub fn wine_path(path: &Path) -> Option<String> {
    let text = path.to_str()?;
    text.starts_with('/').then(|| format!("Z:{}", text.replace('/', "\\")))
}

/// The helper's arguments for `dlls` (Wine paths) and the game, followed by the game's own arguments.
#[must_use]
pub fn command_line(dlls: &[(String, String)], program: &str, game_args: &[OsString]) -> Vec<OsString> {
    let mut line = Vec::new();
    for (path, sha256) in dlls {
        line.extend([OsString::from("--dll"), path.into(), sha256.into()]);
    }
    line.push("--".into());
    line.push(program.into());
    line.extend(game_args.iter().cloned());
    line
}

/// Runs the helper: starts the game suspended, loads the DLLs, resumes it and waits. The process exit code.
#[cfg(windows)]
#[must_use]
pub fn run(args: impl IntoIterator<Item = OsString>) -> u32 {
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;

    const CREATE_SUSPENDED: u32 = 0x4;
    let Some(asked) = parse(args) else {
        eprintln!("dainty-inject: expected --dll <path> <sha256> ... -- <game.exe> [arguments]");
        return EXIT_BAD_ARGS;
    };
    let mut command = std::process::Command::new(&asked.program);
    command.args(&asked.args).creation_flags(CREATE_SUSPENDED);
    if let Some(dir) = asked.program.parent().filter(|d| !d.as_os_str().is_empty()) {
        command.current_dir(dir);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            eprintln!("dainty-inject: {} did not start: {error}", asked.program.display());
            return EXIT_START_FAILED;
        }
    };
    if let Err(error) = crate::load_and_resume(child.as_raw_handle(), child.id(), &asked.dlls) {
        // Never resumed: a game without its runtime must not reach the menu
        let _ = child.kill();
        let _ = child.wait();
        eprintln!("dainty-inject: {error}");
        return EXIT_INJECT_FAILED;
    }
    match child.wait() {
        Ok(status) => status.code().map_or(EXIT_START_FAILED, i32::cast_unsigned),
        Err(error) => {
            eprintln!("dainty-inject: waiting for the game failed: {error}");
            EXIT_START_FAILED
        }
    }
}

/// Off Windows the helper has nothing to do: it runs under Wine.
#[cfg(not(windows))]
#[must_use]
pub fn run(_args: impl IntoIterator<Item = OsString>) -> u32 {
    eprintln!("dainty-inject: runs under Windows or Wine only");
    EXIT_BAD_ARGS
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn the_launchers_line_reads_back() {
        let dlls = [
            ("Z:\\g\\Win64\\rustic-isles-runtime.dll".to_owned(), SHA.to_owned()),
            ("Z:\\w\\CatalogExporter.dll".to_owned(), SHA.to_owned()),
        ];
        let game_args = os(&["-AUTH_PASSWORD=x", "--", "-windowed"]);
        let line = command_line(&dlls, "Z:\\g\\Win64\\Dauntless-Win64-Shipping.exe", &game_args);
        let parsed = parse(line).unwrap();
        assert_eq!(parsed.dlls.len(), 2);
        assert_eq!(parsed.dlls[1].path, PathBuf::from("Z:\\w\\CatalogExporter.dll"));
        assert_eq!(
            parsed.program,
            PathBuf::from("Z:\\g\\Win64\\Dauntless-Win64-Shipping.exe")
        );
        // The game's own "--" is its argument, not the helper's
        assert_eq!(parsed.args, game_args);
    }

    #[test]
    fn anything_else_is_refused() {
        for bad in [
            os(&[]),
            os(&["--", "game.exe"]),
            os(&["--dll", "a.dll", SHA]),
            os(&["--dll", "a.dll", "ABC", "--", "game.exe"]),
            os(&["--dll", "a.dll", &SHA.to_uppercase(), "--", "game.exe"]),
            os(&["--dll", "a.dll", SHA, "--"]),
            os(&["--load", "a.dll", SHA, "--", "game.exe"]),
            os(&["game.exe"]),
        ] {
            assert_eq!(parse(bad.clone()), None, "{bad:?}");
        }
        let too_many: Vec<String> = (0..=MAX_DLLS)
            .flat_map(|i| ["--dll".to_owned(), format!("{i}.dll"), SHA.to_owned()])
            .chain(["--".to_owned(), "game.exe".to_owned()])
            .collect();
        assert_eq!(parse(too_many.into_iter().map(OsString::from)), None);
    }

    #[test]
    fn wine_sees_unix_paths_on_z() {
        assert_eq!(
            wine_path(Path::new("/home/p/Games/Win64/rustic-isles-runtime.dll")).as_deref(),
            Some("Z:\\home\\p\\Games\\Win64\\rustic-isles-runtime.dll")
        );
        assert_eq!(wine_path(Path::new("relative/x.dll")), None);
    }
}
