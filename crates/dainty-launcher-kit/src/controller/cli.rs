// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! `dainty-launcher play|export|servers`: PLAY and the Host page's export session from a terminal, through the
//! same controller calls the page makes, so every check, the relay, the key, the injection and the clean-up are
//! theirs. The terminal output is English (a host's tool) and never carries the account key.

use std::io::Write;
use std::path::{Path, PathBuf};

use futures_util::future::BoxFuture;

use super::{ConsoleSink, Controller, FileFilter, FolderPurpose, Platform};
use crate::bridge::types::{ActionResult, ErrorCode, LauncherError, Snapshot};
use crate::core::i18n::Language;
use crate::platform::inject::InjectDll;
use crate::platform::launch::exit_code_hex;

/// The first words of the line printed once the game is up; a detached run's parent waits for it.
pub const STARTED: &str = "the game started";
/// The hidden flag a detached run's child carries (the shell's own business).
pub const HELD_FLAG: &str = "--held";
/// The exit code of a refused command line or a refused launch.
pub const REFUSED_EXIT: i32 = 1;
/// The exit code of a command line this parser does not take.
pub const USAGE_EXIT: i32 = 2;

/// The commands' help.
pub const USAGE: &str = "\
usage: dainty-launcher play   [--server <id>] [--wait]
       dainty-launcher export [--server <id>] [--exporter <dll>] [--wait]
       dainty-launcher servers

  play      start the game as PLAY does, for the current saved server
  export    start the game with CatalogExporter, as the Host page's export session does
  servers   list the saved servers and their ids; * marks the current one

  --server <id>      switch to that saved server first, as the Servers page does (it stays current)
  --exporter <dll>   the CatalogExporter build to load (default: the Host workspace's)
  --wait             return when the game exits, with its exit code
";

/// What the command line asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliCommand {
    /// PLAY.
    Play,
    /// The export session, with this CatalogExporter or the workspace's.
    Export {
        /// `--exporter`.
        exporter: Option<PathBuf>,
    },
    /// The saved servers.
    Servers,
}

/// A parsed command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliRequest {
    /// The command.
    pub command: CliCommand,
    /// `--server`.
    pub server: Option<String>,
    /// `--wait`.
    pub wait: bool,
    /// The detached child of a run without `--wait`.
    pub held: bool,
}

impl CliRequest {
    /// Whether the shell starts this run detached: a launch without `--wait` returns once the game is up, while
    /// the relay it needs lives on in a child.
    #[must_use]
    pub fn detaches(&self) -> bool {
        !self.wait && !self.held && self.command != CliCommand::Servers
    }
}

/// The command line after the executable, when it names a command (`None`: the window opens as usual).
///
/// # Errors
///
/// Unknown or repeated options, a missing value, or an option the command does not take.
pub fn parse<S: AsRef<str>>(args: &[S]) -> Option<Result<CliRequest, String>> {
    let first = args.first()?.as_ref();
    let mut command = match first {
        "play" => CliCommand::Play,
        "export" => CliCommand::Export { exporter: None },
        "servers" => CliCommand::Servers,
        _ => return None,
    };
    Some(
        parse_options(&mut command, &args[1..]).map(|(server, wait, held)| CliRequest {
            command,
            server,
            wait,
            held,
        }),
    )
}

fn parse_options<S: AsRef<str>>(command: &mut CliCommand, rest: &[S]) -> Result<(Option<String>, bool, bool), String> {
    let (mut server, mut wait, mut held) = (None, false, false);
    let mut rest = rest.iter().map(AsRef::as_ref);
    while let Some(arg) = rest.next() {
        let mut value = |slot: &mut Option<String>| match (slot.is_some(), rest.next()) {
            (true, _) => Err(format!("{arg} given twice")),
            (false, Some(value)) if !value.starts_with("--") => {
                *slot = Some(value.to_owned());
                Ok(())
            }
            (false, _) => Err(format!("{arg} needs a value")),
        };
        match (arg, &mut *command) {
            ("--server", CliCommand::Play | CliCommand::Export { .. }) => value(&mut server)?,
            ("--exporter", CliCommand::Export { exporter }) => {
                let mut text = None;
                if exporter.is_some() {
                    return Err(format!("{arg} given twice"));
                }
                value(&mut text)?;
                *exporter = text.map(PathBuf::from);
            }
            ("--wait", CliCommand::Play | CliCommand::Export { .. }) if !wait => wait = true,
            (HELD_FLAG, CliCommand::Play | CliCommand::Export { .. }) if !held => held = true,
            _ => return Err(format!("unexpected argument: {arg}")),
        }
    }
    Ok((server, wait, held))
}

/// A refusal's English sentence, for the terminal.
#[must_use]
pub const fn error_text(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::AccountBanned => "the account is banned on this server",
        ErrorCode::InviteInvalidFormat => "the text is not an invite link",
        ErrorCode::InviteInvalid => "the invite is no longer accepted",
        ErrorCode::UsernameInvalid => "the name breaks the username rules",
        ErrorCode::UsernameTaken => "the name is already registered on this server",
        ErrorCode::RegistrationClosed => "the server takes no new players",
        ErrorCode::BadRequest => "the server refused the request as malformed",
        ErrorCode::ServerUnreachable => "no server to play on, or it did not answer",
        ErrorCode::ServerError => "the server answered with an error",
        ErrorCode::KeyInvalid => "no account key is stored for this server",
        ErrorCode::KeyRejected => "the server does not know the account key",
        ErrorCode::KeyStorageUnavailable => "the OS key store is unavailable",
        ErrorCode::FolderInvalid => "the folder cannot be used",
        ErrorCode::GameFolderNotFound => "no Dauntless install is set or found",
        ErrorCode::GameWrongVersion => "the game folder holds a build other than 1.14.7",
        ErrorCode::RuntimeUnavailable => "the server offers no game runtime",
        ErrorCode::RuntimeUntrusted => "the server's runtime is not trusted yet; trust it in the launcher",
        ErrorCode::RuntimeFailed => "the game runtime is not installed or cannot be loaded",
        ErrorCode::RuntimeDifferentOnServer => "the server's runtime changed since it was installed",
        ErrorCode::RuntimeInUse => "the runtime files are locked by a running game",
        ErrorCode::LocalRuntimeInvalid => "the local runtime folder does not hold a runtime",
        ErrorCode::DownloadFailed => "a download failed",
        ErrorCode::DiskSpace => "not enough disk space",
        ErrorCode::AlreadyRunning => "the game is already running",
        ErrorCode::LaunchFailed => "the game did not start, or stopped with an error",
        ErrorCode::CompatRuntimeMissing => "Proton or Wine was not found",
        ErrorCode::ConfigFailed => "the game's settings could not be written",
        ErrorCode::Busy => "the launcher is busy with another task",
        ErrorCode::Cancelled => "cancelled",
        ErrorCode::CertMismatch => "the server's certificate does not match the invite's",
        ErrorCode::CertChanged => "the server's certificate changed since the last join",
        ErrorCode::RateLimited => "the server is rate-limiting this client",
        ErrorCode::RelayPortBusy => "the local relay's port is taken by another program",
        ErrorCode::RelayFailed => "the local relay did not start",
        ErrorCode::LoaderLeftovers => "loader files are left beside the game; remove them first",
        ErrorCode::HostFailed => "a Host tool refused or failed; the launcher log has the details",
        ErrorCode::ManifestMismatch => "the game files do not match the server's manifest",
        ErrorCode::NoContentServer => "the server has no content server",
        ErrorCode::FileMissingOnServer => "a file the manifest lists is missing on the server",
        ErrorCode::FileDifferentOnServer => "a file on the server differs from its manifest entry",
        ErrorCode::DllFailed => "a game library could not be installed",
        ErrorCode::GameFilesInvalid => "game files are damaged",
        ErrorCode::WindowsRuntimeMissing => "the Visual C++ runtime is missing",
        ErrorCode::FolderOtherGame => "the folder holds another game",
        ErrorCode::FolderNotEmpty => "the folder is not empty",
        ErrorCode::EditionOther => "this build does not support that game edition",
        ErrorCode::EpicSignedOut => "not signed in to Epic Games",
        ErrorCode::EpicLibraryFailed => "the Epic library could not be read",
        ErrorCode::EpicNotOwned => "the Epic account does not own Dauntless",
        ErrorCode::NotImplemented => "not built yet",
        ErrorCode::NotInThisBuild => "this build of the launcher leaves that out",
        ErrorCode::EpicCodeInvalid => "the Epic authorization code is not valid",
        ErrorCode::EpicRejected => "Epic Games refused the sign-in",
        ErrorCode::EpicUnreachable => "Epic Games did not answer",
        ErrorCode::EpicFailed => "Epic Games failed",
        ErrorCode::GameVersionUnknown => "the installed game version could not be identified",
        ErrorCode::GameVersionUnavailable => "that game version cannot be downloaded",
        ErrorCode::EpicManifestUnavailable => "Epic Games gave no file list for the version",
        ErrorCode::GuardedNotFound => "the guarded action is unknown or expired",
        ErrorCode::GuardedNotWaiting => "the guarded action is not waiting for confirmation",
        ErrorCode::ServerStopped => "the embedded server is not running",
        ErrorCode::ModuleUnknown => "the server has no module with that id",
        ErrorCode::ModuleBadState => "the module's state does not allow that",
        ErrorCode::ModuleInputInvalid => "the module refused the input",
        ErrorCode::ModuleSettingsInvalid => "the module refused the settings",
        ErrorCode::ModuleFailed => "the module failed",
        ErrorCode::HistoryBadFilter => "the history filter is not valid",
        ErrorCode::HistoryUnavailable => "the change history is not available",
        ErrorCode::HistoryPlanChanged => "the history changed since the preview",
        ErrorCode::HistoryFailed => "updating the history failed",
        ErrorCode::RealmUnavailable => "the server has no realms",
        ErrorCode::RealmNotFound => "no realm has that id",
        ErrorCode::RealmFailed => "the realm failed",
        ErrorCode::InjectHelperMissing => "the Proton inject helper is missing from the launcher's resources",
        ErrorCode::GameMissingArchon => "the folder has game files but no Archon folder with the executable",
        ErrorCode::GameWrongBuild => "the game folder holds 1.14.7, but not the build this launcher plays",
        ErrorCode::GameRevived => "the game folder is a Dauntless Revived copy",
        ErrorCode::GamePathTooLong => "the game folder's path is too long",
        ErrorCode::LauncherTooOld => "the server asks for a newer launcher; update it first",
        ErrorCode::LauncherUpdateFailed => "the launcher could not update itself; install the newer one by hand",
        ErrorCode::Unknown => "something failed; the launcher log has the details",
        ErrorCode::ServerDnsFailed => "the server's name did not resolve; try another DNS server",
        ErrorCode::ServerRefused => "the server's address refused the connection",
        ErrorCode::ServerTimedOut => "the server did not answer in time",
        ErrorCode::ServerTlsFailed => "the secure connection to the server failed",
        ErrorCode::OauthUnavailable => "the server has no sign-in",
        ErrorCode::OauthFailed => "the sign-in provider refused or did not answer",
        ErrorCode::OauthExpired => "the sign-in took too long; start again",
        ErrorCode::OauthNotMember => "the sign-in account is not in the server's required group",
        ErrorCode::OauthPortBusy => "another program holds port 61190, which the sign-in comes back to",
        ErrorCode::OauthUsernameNeeded => "pick a name: the sign-in's name can't be used",
        ErrorCode::OauthRequired => "the server makes new accounts only through its sign-in",
        ErrorCode::OauthDenied => "the sign-in was declined",
        ErrorCode::OauthBadUrl => "the server sent a sign-in address the launcher does not open",
    }
}

/// A refusal as one terminal line: `error: <code>: <sentence> (<detail>)`.
#[must_use]
pub fn error_line(error: &LauncherError) -> String {
    let code = serde_json::to_value(error.code)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default();
    let mut line = format!("error: {code}: {}", error_text(error.code));
    if let Some(detail) = error.detail.as_deref().filter(|d| !d.is_empty()) {
        line.push_str(&format!(" ({detail})"));
    }
    line
}

/// The controller's app without a window: no dialogs, nothing to emit, and the game's console hidden as the
/// window's PLAY hides it.
#[derive(Debug, Default)]
pub struct HeadlessPlatform;

impl Platform for HeadlessPlatform {
    fn choose_folder(&self, _: Language, _: FolderPurpose, _: Option<PathBuf>) -> BoxFuture<'_, Option<PathBuf>> {
        Box::pin(async { None })
    }

    fn choose_save_file(&self, _: Language, _: String, _: FileFilter) -> BoxFuture<'_, Option<PathBuf>> {
        Box::pin(async { None })
    }

    fn choose_open_file(&self, _: Language, _: FileFilter) -> BoxFuture<'_, Option<PathBuf>> {
        Box::pin(async { None })
    }

    fn emit_snapshot(&self, _: &Snapshot) {}

    fn watch_game_console(&self, pid: u32, hide: bool, sink: ConsoleSink) {
        crate::platform::game_console::watch(pid, hide, Box::new(move |lines| sink(lines)));
    }
}

// A detached run's parent has gone once the game is up, so its pipe may be closed
macro_rules! say {
    ($out:expr, $($arg:tt)*) => {{
        let _ = writeln!($out, $($arg)*);
        let _ = $out.flush();
    }};
}

impl Controller {
    /// Runs a parsed command line to its end: once the game exited and its clean-up is done, so the relay lives
    /// as long as the game. The answer is the process's exit code: the game's with `--wait`, else 0 once it
    /// started; [`REFUSED_EXIT`] when anything was refused.
    pub async fn run_cli<W: Write>(&self, request: &CliRequest, out: &mut W) -> i32 {
        self.start_up().await;
        if request.command == CliCommand::Servers {
            self.print_servers(out);
            return 0;
        }
        if let Some(id) = request.server.as_deref()
            && self.server().is_none_or(|current| current.id() != id)
        {
            if !self.stored().saved_servers().iter().any(|saved| saved.id() == id) {
                say!(
                    out,
                    "error: no saved server has the id {id}; `dainty-launcher servers` lists them"
                );
                return REFUSED_EXIT;
            }
            if let ActionResult::Err(error) = self.switch_server(id).await {
                say!(out, "{}", error_line(&error));
                return REFUSED_EXIT;
            }
        }
        let Some(server) = self.server() else {
            say!(out, "{}", error_line(&LauncherError::new(ErrorCode::ServerUnreachable)));
            return REFUSED_EXIT;
        };
        say!(
            out,
            "server: {} ({}:{}, id {})",
            server.name,
            server.host,
            server.port,
            server.id()
        );
        // The runtime offer PLAY checks comes with the server's answer
        let _ = self.connect().await;

        let exits = self.shared.exits.subscribe();
        let before = exits.borrow().0;
        let started = std::time::SystemTime::now();
        let answer = match &request.command {
            CliCommand::Export { exporter } => {
                let exporter = exporter.as_deref().map(absolute);
                let dll = exporter.clone().or_else(|| self.host_exporter_dll());
                if let Some(found) = dll.as_deref().and_then(|dll| InjectDll::as_found(dll).ok()) {
                    say!(out, "exporter: {}, SHA-256 {}", found.path.display(), found.sha256);
                }
                self.host_export(exporter.as_deref()).await
            }
            _ => self.play().await,
        };
        if let ActionResult::Err(error) = answer {
            say!(out, "{}", error_line(&error));
            return REFUSED_EXIT;
        }
        match self.shared.game.pid() {
            Some(pid) => say!(out, "{STARTED} (pid {pid})"),
            None => say!(out, "{STARTED}"),
        }

        let code = self.game_exit(exits, before).await;
        match code {
            Some(0) => say!(out, "the game exited with code 0"),
            Some(code) => say!(out, "the game exited with code {}", exit_code_hex(code)),
            None => say!(out, "the game exited without a code"),
        }
        if matches!(request.command, CliCommand::Export { .. }) {
            self.print_exports(started, out);
        }
        if request.wait { code.unwrap_or(REFUSED_EXIT) } else { 0 }
    }

    /// The code of the first game to exit after `before`, once its clean-up is done.
    async fn game_exit(&self, mut exits: tokio::sync::watch::Receiver<(u64, Option<i32>)>, before: u64) -> Option<i32> {
        match exits.wait_for(|(count, _)| *count > before).await {
            Ok(seen) => seen.1,
            Err(_) => None,
        }
    }

    fn print_servers<W: Write>(&self, out: &mut W) {
        let current = self.server().map(|server| server.id());
        let saved = self.stored().saved_servers();
        if saved.is_empty() {
            say!(out, "no saved servers; join one with an invite in the launcher");
        }
        for server in saved {
            let id = server.id();
            let mark = if current.as_deref() == Some(id.as_str()) {
                '*'
            } else {
                ' '
            };
            say!(out, "{mark} {id}  {} ({}:{})", server.name, server.host, server.port);
        }
    }

    fn print_exports<W: Write>(&self, started: std::time::SystemTime, out: &mut W) {
        let candidates = self.export_candidates();
        if candidates.is_empty() {
            say!(out, "no game folder to look for exports in");
            return;
        }
        match self.latest_exports(Some(started)) {
            Some(dir) if Some(&dir) == candidates.first() => say!(out, "exports: {}", dir.display()),
            // A build with an export root reads its flags there too, not the file the launcher wrote beside the game
            Some(dir) => say!(
                out,
                "exports: {} (CatalogExporter's own export root; its flags came from there)",
                dir.display()
            ),
            None => {
                let looked: Vec<String> = candidates.iter().map(|dir| dir.display().to_string()).collect();
                say!(
                    out,
                    "no exports written since the game started (looked in {})",
                    looked.join(", ")
                );
            }
        }
    }
}

/// `--exporter` as given, made absolute against the working directory (the injector takes absolute paths only).
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;

    fn request(command: CliCommand, server: Option<&str>, wait: bool) -> CliRequest {
        CliRequest {
            command,
            server: server.map(str::to_owned),
            wait,
            held: false,
        }
    }

    #[test]
    fn only_a_command_word_is_a_command_line() {
        assert_eq!(parse::<&str>(&[]), None);
        assert_eq!(parse(&["dauntless://invite"]), None);
        assert_eq!(parse(&["--read-game-console", "12"]), None);
        assert_eq!(parse(&["play"]), Some(Ok(request(CliCommand::Play, None, false))));
    }

    #[test]
    fn options_parse_per_command() {
        assert_eq!(
            parse(&["play", "--wait", "--server", "private:127.0.0.1:41000"]),
            Some(Ok(request(CliCommand::Play, Some("private:127.0.0.1:41000"), true)))
        );
        assert_eq!(
            parse(&["export", "--exporter", "C:/x/CatalogExporter.dll", "--wait"]),
            Some(Ok(request(
                CliCommand::Export {
                    exporter: Some(PathBuf::from("C:/x/CatalogExporter.dll"))
                },
                None,
                true
            )))
        );
        assert_eq!(parse(&["servers"]), Some(Ok(request(CliCommand::Servers, None, false))));
    }

    #[test]
    fn bad_options_are_refused() {
        for args in [
            &["play", "--exporter", "x.dll"][..],
            &["play", "--server"],
            &["play", "--server", "--wait"],
            &["play", "--wait", "--wait"],
            &["play", "--server", "a", "--server", "b"],
            &["export", "--exporter", "a.dll", "--exporter", "b.dll"],
            &["servers", "--wait"],
            &["play", "extra"],
        ] {
            assert!(matches!(parse(args), Some(Err(_))), "{args:?}");
        }
    }

    #[test]
    fn only_a_launch_without_wait_detaches() {
        assert!(request(CliCommand::Play, None, false).detaches());
        assert!(!request(CliCommand::Play, None, true).detaches());
        assert!(!request(CliCommand::Servers, None, false).detaches());
        let mut held = request(CliCommand::Play, None, false);
        held.held = true;
        assert!(!held.detaches());
        assert!(
            parse(&["export", HELD_FLAG]).unwrap().unwrap().held,
            "the child's flag parses"
        );
    }

    #[test]
    fn a_refusal_prints_its_code_sentence_and_detail() {
        assert_eq!(
            error_line(&LauncherError::with_detail(ErrorCode::LoaderLeftovers, "winmm.dll")),
            "error: loader_leftovers: loader files are left beside the game; remove them first (winmm.dll)"
        );
        assert_eq!(
            error_line(&LauncherError::new(ErrorCode::KeyInvalid)),
            "error: key_invalid: no account key is stored for this server"
        );
    }
}
