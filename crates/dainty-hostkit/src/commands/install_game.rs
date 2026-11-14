// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `dainty-host install-game` (DE6, DD22): installs Dauntless from the host's own Epic library for the game
//! servers (`crate::epicinstall`), then points settings.toml's `game_dir` at the new folder and rewrites the env files,
//! so the Director's `GAMESERVER_BINARY_PATH` follows (the same two steps `multipliers` takes).
//!
//! `--code-file <file>` or a pasted line on stdin gives the Epic authorization code; the session it opens is kept in
//! `.workspace/config/epic-session.env` (owner-only). `--sign-out` ends and deletes it.

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use dainty_epic::auth::{Accounts, EpicAccounts};
use dainty_epic::download::{Control, Options};
use dainty_epic::library::{EgsLibrary, Library, Ownership};

use crate::Outcome;
use crate::cli::InstallGameArgs;
use crate::epicinstall::{self, DEFAULT_VERSION, Env, Files, Plan};
use crate::error::HostError;
use crate::hostconfig::{
    HostConfig, Platform, build_env_files, load_host_config, load_or_create_secrets, save_host_config, write_env_files,
};
use crate::paths::Paths;

/// A pasted code, or the whole page Epic shows it in, is a few hundred bytes.
const CODE_MAX_BYTES: usize = 16 * 1024;

/// What happened to settings.toml.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Configured {
    /// `game_dir` now names the folder; the env files were rewritten if setup had made them
    Updated {
        /// The folder it named before.
        previous: Option<String>,
        /// Whether the env files were rewritten.
        env_files: bool,
    },
    /// It already did
    Unchanged,
    /// No settings.toml yet: setup takes the folder as `--game`
    NoSettings,
    /// Not the version `game_dir` is for
    OtherVersion,
}

/// The real thing: Epic's services, the CDN, this PC's stdin and Ctrl+C.
pub fn run(paths: &Paths, args: &InstallGameArgs) -> Outcome {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| HostError::message(format!("could not start the runtime: {e}")))?;
    let control = Control::default();
    let options = Options::new(Arc::new(|path: &Path| {
        free_bytes(path).ok_or_else(|| std::io::Error::other("free space unknown"))
    }));
    let commit = |repo: &Path| -> Result<String, HostError> {
        Ok(crate::run::capture("git", &["rev-parse", "--short=12", "HEAD"], Some(repo))?.unwrap_or_default())
    };
    runtime.block_on(async {
        let cancel = control.clone();
        // A stopped install keeps its resume file: the same command goes on from there
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                cancel.cancel();
            }
        });
        let stdin = std::io::stdin();
        let mut lines = stdin.lock();
        run_with(
            paths,
            args,
            Deps {
                accounts: &EpicAccounts,
                library: EgsLibrary,
                options,
                platform: Platform::current(),
                source_commit: &commit,
                plan: &Plan::for_version,
            },
            &control,
            &mut lines,
            &mut |line| println!("{line}"),
        )
        .await
    })
}

/// Free bytes on the drive that holds `dir` (or its nearest existing parent).
fn free_bytes(dir: &Path) -> Option<u64> {
    let mut current = Some(dir);
    while let Some(path) = current {
        if let Ok(free) = fs4::available_space(path) {
            return Some(free);
        }
        current = path.parent();
    }
    None
}

/// What the command talks to; the tests give doubles and a fake CDN.
pub struct Deps<'a, L: Library> {
    /// The Epic sign-in.
    pub accounts: &'a dyn Accounts,
    /// The host's Epic library.
    pub library: L,
    /// How chunks are fetched.
    pub options: Options,
    /// The OS the env files are rewritten for.
    pub platform: Platform,
    /// The short commit of the release or checkout, for the env files.
    pub source_commit: &'a dyn Fn(&Path) -> Result<String, HostError>,
    /// The build for a `--version` ([`Plan::for_version`] for the command; the tests plan their own)
    pub plan: &'a dyn Fn(&str) -> Result<Plan, HostError>,
}

/// The command with its services given. `stdin` is read only when a code is needed and `--code-file` is not.
pub async fn run_with<L: Library>(
    paths: &Paths,
    args: &InstallGameArgs,
    deps: Deps<'_, L>,
    control: &Control,
    stdin: &mut dyn BufRead,
    out: &mut (dyn FnMut(&str) + Send),
) -> Outcome {
    let files = Files::of(paths);
    let Deps {
        accounts,
        library,
        options,
        platform,
        source_commit,
        plan,
    } = deps;
    let ownership = Ownership::new(library, Some(files.data.clone()));
    if args.sign_out {
        epicinstall::sign_out(accounts, &ownership, &files, out).await?;
        return Ok(ExitCode::SUCCESS);
    }
    let plan = plan(args.version.as_deref().map_or(DEFAULT_VERSION, str::trim))?;
    let dir = match args.dir.as_deref().map(str::trim) {
        Some("") => return Err(HostError::message("--dir needs a folder")),
        Some(dir) if !Path::new(dir).is_absolute() => {
            return Err(HostError::message(format!(
                "--dir needs the full path of a folder, like \"/srv/dauntless\" (got {dir})"
            )));
        }
        Some(dir) => PathBuf::from(dir),
        None => files.default_dir.clone(),
    };
    let code_file = args.code_file.as_deref().map(str::trim).filter(|f| !f.is_empty());
    let mut ask_code = || match code_file {
        Some(file) => read_code_file(Path::new(file)),
        None => read_pasted(stdin),
    };
    let env = Env {
        accounts,
        ownership: &ownership,
        options,
    };
    let done = epicinstall::install_game(&env, &files, &plan, &dir, control, &mut ask_code, out).await?;
    if !done.already {
        out(&format!(
            "Installed {} into {} ({} files, {}).",
            plan.display_name,
            done.folder.display(),
            done.files,
            epicinstall::size(done.bytes)
        ));
    }
    let folder = done.folder.to_string_lossy().into_owned();
    if plan.version != DEFAULT_VERSION {
        out(&format!(
            "settings.toml's game_dir is for Dauntless {DEFAULT_VERSION} and was not changed."
        ));
        return Ok(ExitCode::SUCCESS);
    }
    match configure(paths, &folder, platform, source_commit)? {
        Configured::Updated { previous, env_files } => {
            out(&format!(
                "settings.toml: game_dir = \"{folder}\"{}",
                previous.map_or(String::new(), |old| format!(" (was \"{old}\")"))
            ));
            if env_files {
                out("The env files are rewritten, so the Director's GAMESERVER_BINARY_PATH names the new game.");
                out("Restart the server for it to take effect. `dainty-host doctor` checks the game folder.");
            } else {
                out("Run dainty-host setup to write the env files from it.");
            }
        }
        Configured::Unchanged => out("settings.toml already names this folder."),
        Configured::NoSettings => out(&format!(
            "No settings.toml yet: run setup with --game \"{folder}\" so the game servers run from it."
        )),
        Configured::OtherVersion => {}
    }
    Ok(ExitCode::SUCCESS)
}

/// Points `game_dir` at `folder` and, when setup has made them, rewrites the env files (as `multipliers` does).
pub fn configure(
    paths: &Paths,
    folder: &str,
    platform: Platform,
    source_commit: &dyn Fn(&Path) -> Result<String, HostError>,
) -> Result<Configured, HostError> {
    let Some(config) = load_host_config(paths)? else {
        return Ok(Configured::NoSettings);
    };
    if config.game_dir.as_deref() == Some(folder) {
        return Ok(Configured::Unchanged);
    }
    let previous = config.game_dir.clone();
    let config = HostConfig {
        game_dir: Some(folder.to_owned()),
        ..config
    };
    save_host_config(paths, &config)?;
    let made = paths.config.join("director.env").exists();
    if made {
        let (secrets, _) = load_or_create_secrets(&paths.config, super::setup::secrets::new_secrets)?;
        let commit = source_commit(&paths.repo)?;
        write_env_files(paths, &build_env_files(&config, &secrets, paths, &commit, platform))?;
    }
    Ok(Configured::Updated {
        previous,
        env_files: made,
    })
}

fn read_code_file(file: &Path) -> Result<String, HostError> {
    let meta = std::fs::metadata(file).map_err(|e| HostError::io(file, e))?;
    if meta.len() > CODE_MAX_BYTES as u64 {
        return Err(HostError::message(format!(
            "{} is larger than {} KiB: not an Epic code",
            file.display(),
            CODE_MAX_BYTES / 1024
        )));
    }
    std::fs::read_to_string(file).map_err(|e| HostError::io(file, e))
}

/// The code from stdin: one line, or the page Epic shows it in when that spans lines. The prompt goes to stderr.
fn read_pasted(stdin: &mut dyn BufRead) -> Result<String, HostError> {
    eprint!("Paste the code (or the page) here and press Enter: ");
    let mut text = String::new();
    loop {
        let mut line = String::new();
        let read = stdin
            .read_line(&mut line)
            .map_err(|e| HostError::message(format!("could not read the code: {e}")))?;
        if read == 0 {
            break;
        }
        text.push_str(&line);
        let seen = text.trim();
        if text.len() > CODE_MAX_BYTES {
            return Err(HostError::message("that is too long to be an Epic code"));
        }
        if seen.is_empty() {
            continue;
        }
        if !seen.starts_with('{') || serde_json::from_str::<serde_json::Value>(seen).is_ok() {
            break;
        }
    }
    if text.trim().is_empty() {
        return Err(HostError::message(
            "no code was given (standard input is closed): paste it when asked, or pass --code-file <file>",
        ));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;

    #[test]
    fn a_single_line_is_the_code() {
        let mut input: &[u8] = b"  0123456789abcdef0123456789abcdef \nnot read\n";
        assert_eq!(
            read_pasted(&mut input).unwrap().trim(),
            "0123456789abcdef0123456789abcdef"
        );
    }

    #[test]
    fn a_page_over_several_lines_is_read_whole() {
        let mut input: &[u8] = b"{\n  \"authorizationCode\": \"abc123\",\n  \"sid\": null\n}\nrest\n";
        let text = read_pasted(&mut input).unwrap();
        assert!(text.trim_end().ends_with('}') && text.contains("abc123"), "{text}");
    }

    #[test]
    fn a_closed_stdin_says_what_to_do() {
        let mut input: &[u8] = b"\n\n";
        let error = read_pasted(&mut input).unwrap_err().to_string();
        assert!(error.contains("--code-file"), "{error}");
    }

    #[test]
    fn a_huge_paste_is_refused() {
        let big = format!("{{{}", "a".repeat(CODE_MAX_BYTES + 10));
        let mut input = big.as_bytes();
        assert!(read_pasted(&mut input).is_err());
    }
}
