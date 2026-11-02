// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `reforged-host install-game` (DD22, DE6): the game servers' Dauntless installed from the host's own Epic library,
//! on `crates/epic` (DE1 to DE5), the launcher's installer. Epic's account service, the library and the CDN are
//! behind the crate's own seams, so the tests put doubles and a fake CDN in their place.
//!
//! The host signs in with the pasted-code flow (the launcher's fallback): the login URL is printed, the player opens
//! it anywhere and pastes the code Epic shows. The session it gets is kept in `epic-session.env` beside
//! `secrets.env`, owner-only, and nowhere else: never in host.toml, a command line option, an env file or a log.
//! Nothing here logs; `egs-api` logs through `log`, which this command never installs a logger for.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use epic::auth::{Accounts, AuthError, AuthorizationCode, LOGIN_URL, Session, Token};
use epic::builds::{self, Build};
use epic::download::{
    self, Control, DownloadError, Executable, Https, ManifestOrigin, Options, Owned, Progress, Stage, Target,
    fetch_manifest,
};
use epic::library::{APP_NAME, Library, LibraryError, Ownership, OwnershipError};
use epic::manifest::{InstallPath, Sha1Digest};
use sha2::{Digest, Sha256};

use crate::envfile::format_env;
use crate::error::HostError;
use crate::hostconfig::{read_env_file, write_secret_file};
use crate::paths::Paths;

/// The session, in the config folder with the other secrets.
pub const SESSION_FILE: &str = "epic-session.env";
/// The default `--dir`, in the workspace.
pub const DEFAULT_GAME_FOLDER: &str = "game";
/// The version installed when `--version` is left out: the one the game servers run.
pub const DEFAULT_VERSION: &str = "1.14.7";

const ACCOUNT_ID: &str = "REFORGED_EPIC_ACCOUNT_ID";
const DISPLAY_NAME: &str = "REFORGED_EPIC_DISPLAY_NAME";
const ACCESS_TOKEN: &str = "REFORGED_EPIC_ACCESS_TOKEN";
const ACCESS_EXPIRES: &str = "REFORGED_EPIC_ACCESS_EXPIRES_AT";
const REFRESH_TOKEN: &str = "REFORGED_EPIC_REFRESH_TOKEN";
const REFRESH_EXPIRES: &str = "REFORGED_EPIC_REFRESH_EXPIRES_AT";

/// An access token this close to its end is refreshed first: the library check needs it.
const REFRESH_MARGIN: i64 = 300;

/// Where this command keeps its files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Files {
    pub session: PathBuf,
    /// The resume files, the ownership answer and the records of finished installs: `.workspace/epic`
    pub data: PathBuf,
    /// `--dir` unless given: `.workspace/game`
    pub default_dir: PathBuf,
}

impl Files {
    pub fn of(paths: &Paths) -> Self {
        Self {
            session: paths.config.join(SESSION_FILE),
            data: paths.workspace.join("epic"),
            default_dir: paths.workspace.join(DEFAULT_GAME_FOLDER),
        }
    }

    pub fn resume_file(&self, version: &str) -> PathBuf {
        self.data.join(format!("{version}.resume"))
    }

    /// The record that `version` was installed, holding its folder (the launcher's `userData/epic` has the same).
    pub fn installed_record(&self, version: &str) -> PathBuf {
        self.data.join(format!("{version}.installed"))
    }
}

// ------------------------------------------------------------------ the stored session

/// Owner-only on Unix, written to a temporary file and renamed over the old one.
pub fn save_session(file: &Path, session: &Session) -> Result<(), HostError> {
    let entries = [
        (ACCOUNT_ID, session.account_id().to_owned()),
        (DISPLAY_NAME, session.display_name().to_owned()),
        (ACCESS_TOKEN, session.access_token().expose().to_owned()),
        (ACCESS_EXPIRES, session.access_expires_at().to_string()),
        (REFRESH_TOKEN, session.refresh_token().expose().to_owned()),
        (REFRESH_EXPIRES, session.refresh_expires_at().to_string()),
    ];
    let text = format_env(
        &[
            "DaintyLass: the Epic session `reforged-host install-game` signed in with. SECRET: whoever has this file can act",
            "as the Epic account for weeks. Never share or commit it; `reforged-host install-game --sign-out` ends it.",
        ],
        &entries,
    )?;
    write_secret_file(file, &text)
}

/// The stored session; None when there is none, or when the file is not one this command wrote (the next run signs
/// in again and replaces it).
pub fn load_session(file: &Path) -> Result<Option<Session>, HostError> {
    if !file.exists() {
        return Ok(None);
    }
    #[cfg(unix)]
    tighten(file);
    let env = read_env_file(file)?;
    let text = |key| env.get(key).filter(|v| !v.is_empty());
    let seconds = |key| env.get(key).and_then(|v| v.parse::<i64>().ok());
    let (Some(account), Some(access), Some(refresh), Some(access_at), Some(refresh_at)) = (
        text(ACCOUNT_ID),
        text(ACCESS_TOKEN),
        text(REFRESH_TOKEN),
        seconds(ACCESS_EXPIRES),
        seconds(REFRESH_EXPIRES),
    ) else {
        return Ok(None);
    };
    Ok(Some(Session::new(
        account.to_owned(),
        env.get(DISPLAY_NAME).unwrap_or_default().to_owned(),
        Token::new(access.to_owned()),
        access_at,
        Token::new(refresh.to_owned()),
        refresh_at,
    )))
}

/// A file left readable by others (copied by hand, restored from a backup) is closed again.
#[cfg(unix)]
fn tighten(file: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(file)
        && meta.permissions().mode() & 0o077 != 0
    {
        let _ = std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600));
    }
}

pub fn delete_session(file: &Path) -> Result<(), HostError> {
    match std::fs::remove_file(file) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(HostError::io(file, e)),
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_secs()).unwrap_or(i64::MAX))
}

// ------------------------------------------------------------------ what to install

/// One version to install: a compiled-in build (`epic::builds`), or the tests' own.
#[derive(Debug, Clone)]
pub struct Plan {
    pub version: String,
    pub display_name: String,
    pub build_version: String,
    /// The pin every manifest source's bytes are checked against
    pub pin: Sha1Digest,
    pub sources: Vec<String>,
    pub executable: Executable,
}

impl Plan {
    pub fn for_build(build: &Build) -> Result<Self, HostError> {
        let broken = || HostError::message(format!("the build table has a bad row for {}", build.version));
        Ok(Self {
            version: build.version.to_owned(),
            display_name: build.display_name.to_owned(),
            build_version: build.build_version.to_owned(),
            pin: build.pin().ok_or_else(broken)?,
            sources: vec![build.manifest_source()],
            executable: Executable {
                path: InstallPath::new(builds::EXECUTABLE).map_err(|_| broken())?,
                sha256: build.executable_sha256_bytes().ok_or_else(broken)?,
            },
        })
    }

    /// For `--version`: refuses a version Epic cannot give, saying what to do instead.
    pub fn for_version(version: &str) -> Result<Self, HostError> {
        match builds::find(version) {
            Some(build) => Self::for_build(build),
            None => Err(HostError::message(format!(
                "cannot install version {version} from Epic: {}",
                available()
            ))),
        }
    }
}

fn available() -> String {
    let versions: Vec<&str> = builds::BUILDS.iter().map(|build| build.version).collect();
    format!("the versions are {}", versions.join(" and "))
}

/// Everything the install talks to. The real command uses `EpicAccounts`, `EgsLibrary` and the CDN; the tests
/// use doubles and a fake CDN (`Options::bases`, `Options::tls`).
pub struct Env<'a, L: Library> {
    pub accounts: &'a dyn Accounts,
    pub ownership: &'a Ownership<L>,
    pub options: Options,
}

/// What a finished run reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done {
    pub folder: PathBuf,
    /// It was installed already and its executable still checked out: nothing was fetched
    pub already: bool,
    pub files: usize,
    pub bytes: u64,
}

type Out<'a> = Mutex<&'a mut (dyn FnMut(&str) + Send)>;

fn say(out: &Out<'_>, line: &str) {
    if let Ok(mut out) = out.lock() {
        (*out)(line);
    }
}

// ------------------------------------------------------------------ signing in

fn auth_message(error: &AuthError) -> String {
    match error {
        AuthError::BadCode => {
            "that is not an Epic authorization code (letters and digits, or the whole page Epic showed)".into()
        }
        AuthError::Rejected => {
            "Epic refused the sign-in: the code is used or has expired. Open the login address again for a new one"
                .into()
        }
        AuthError::Unreachable => "Epic's account service could not be reached".into(),
        _ => "Epic's account service gave an unexpected answer".into(),
    }
}

/// The signed-in session: the stored one while it lasts, refreshed when it is about to end, a new one from a
/// pasted code when there is none (or Epic has ended it). `ask_code` is called only then.
pub async fn session(
    accounts: &dyn Accounts,
    file: &Path,
    ask_code: &mut dyn FnMut() -> Result<String, HostError>,
    out: &Out<'_>,
) -> Result<Session, HostError> {
    let now = unix_now();
    if let Some(stored) = load_session(file)? {
        if stored.access_expires_at() > now + REFRESH_MARGIN {
            say(out, &format!("Signed in to Epic as {}.", who(&stored)));
            return Ok(stored);
        }
        if stored.refresh_expires_at() > now {
            match epic::auth::refresh(accounts, &stored).await {
                Ok(fresh) => {
                    // Epic turns the refresh token over each time: the old one is dead
                    save_session(file, &fresh)?;
                    say(out, &format!("Signed in to Epic as {}.", who(&fresh)));
                    return Ok(fresh);
                }
                Err(AuthError::Unreachable) => return Err(HostError::message(auth_message(&AuthError::Unreachable))),
                Err(_) => say(out, "The stored Epic session has ended. Signing in again."),
            }
        } else {
            say(out, "The stored Epic session has expired. Signing in again.");
        }
    }
    say(out, "Sign in to Epic to install Dauntless from your own library.");
    say(
        out,
        "  1. Open this address in a browser, on any computer, and sign in:",
    );
    say(out, &format!("     {LOGIN_URL}"));
    say(
        out,
        "  2. Epic then shows a page of text with an \"authorizationCode\". Copy the code (or the whole page).",
    );
    say(
        out,
        "The code is spent by this one sign-in and never stored. What is stored is the session it opens, owner-only,",
    );
    say(
        out,
        "so the next run, a resume or another version needs no code. `install-game --sign-out` ends it.",
    );
    let pasted = ask_code()?;
    let code = AuthorizationCode::from_pasted(&pasted).map_err(|e| HostError::message(auth_message(&e)))?;
    let fresh = epic::auth::sign_in(accounts, code)
        .await
        .map_err(|e| HostError::message(auth_message(&e)))?;
    save_session(file, &fresh)?;
    say(out, &format!("Signed in to Epic as {}.", who(&fresh)));
    Ok(fresh)
}

fn who(session: &Session) -> &str {
    if session.display_name().is_empty() {
        "an Epic account"
    } else {
        session.display_name()
    }
}

/// `--sign-out`: ends the session at Epic and deletes the file whether or not Epic answers, and the cached
/// ownership answer with it.
pub async fn sign_out<L: Library>(
    accounts: &dyn Accounts,
    ownership: &Ownership<L>,
    files: &Files,
    out: &mut (dyn FnMut(&str) + Send),
) -> Result<(), HostError> {
    let stored = load_session(&files.session)?;
    delete_session(&files.session)?;
    ownership.forget();
    match stored {
        None => out("No Epic session is stored."),
        Some(session) => match epic::auth::sign_out(accounts, session).await {
            Ok(()) => out("Signed out of Epic. The stored session is deleted."),
            Err(_) => out(
                "The stored session is deleted. Epic could not be reached to end it there; it ends by itself in a few weeks, or sign out at epicgames.com/account/password.",
            ),
        },
    }
    Ok(())
}

// ------------------------------------------------------------------ installing

fn library_message(error: &LibraryError) -> &'static str {
    match error {
        LibraryError::SessionExpired => "the Epic session has expired: run install-game again",
        _ => "Epic's library could not be read: try again in a few minutes",
    }
}

fn ownership_message(error: &OwnershipError) -> String {
    match error {
        OwnershipError::NotOwned => {
            "this Epic account does not own Dauntless, so nothing was downloaded: sign in with the account whose library has it (install-game --sign-out first)".into()
        }
        OwnershipError::Library(e) => library_message(e).into(),
        _ => "Epic's library could not be read".into(),
    }
}

fn download_message(error: &DownloadError) -> String {
    match error {
        DownloadError::NotOwned => ownership_message(&OwnershipError::NotOwned),
        DownloadError::SignedOut => library_message(&LibraryError::SessionExpired).into(),
        DownloadError::Library => library_message(&LibraryError::Unreachable).into(),
        DownloadError::NoSpace { needed, available } => format!(
            "not enough free space: {} are needed and {} are free",
            size(*needed),
            size(*available)
        ),
        DownloadError::ChunkUnavailable { deleted: true, .. } => {
            "Epic has deleted part of this version from its servers, so it cannot be installed".into()
        }
        DownloadError::Cancelled => "stopped; run the same command again to continue where it left off".into(),
        DownloadError::ExecutableHash { .. } | DownloadError::ExecutableMissing { .. } => {
            "the downloaded game executable is not the one this version pins".into()
        }
        other => format!("the download stopped: {other}. Run the same command again to continue"),
    }
}

/// `15.1 GiB`
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Prints a progress line each time the download passes another 2%, and each stage.
struct Reporter<'a> {
    out: &'a Out<'a>,
    state: Mutex<ReporterState>,
}

struct ReporterState {
    /// Bytes already there when the first line was seen (a resume starts above zero), and when
    first: Option<(u64, Instant)>,
    step: u64,
    stage: Option<Stage>,
}

impl<'a> Reporter<'a> {
    fn new(out: &'a Out<'a>) -> Self {
        Self {
            out,
            state: Mutex::new(ReporterState {
                first: None,
                step: u64::MAX,
                stage: None,
            }),
        }
    }

    fn update(&self, progress: &Progress) {
        let Ok(mut state) = self.state.lock() else { return };
        let changed = state.stage != Some(progress.stage);
        state.stage = Some(progress.stage);
        match progress.stage {
            Stage::Preparing if changed => say(self.out, "Checking the free space and making the files."),
            Stage::Finishing if changed => say(self.out, "Checking the game executable."),
            Stage::Paused if changed => say(self.out, "Paused."),
            Stage::Downloading => {
                let (from, at) = *state.first.get_or_insert((progress.downloaded, Instant::now()));
                let percent = progress
                    .downloaded
                    .saturating_mul(100)
                    .checked_div(progress.download_total)
                    .unwrap_or(100);
                let step = percent / 2;
                if changed || step != state.step {
                    state.step = step;
                    let elapsed = at.elapsed().as_secs_f64();
                    let rate = if elapsed >= 1.0 {
                        (progress.downloaded.saturating_sub(from)) as f64 / elapsed
                    } else {
                        0.0
                    };
                    let mut line = format!(
                        "  {percent:>3}%  {} of {}",
                        size(progress.downloaded),
                        size(progress.download_total)
                    );
                    if rate > 0.0 {
                        let _ = write!(line, "  {}/s", size(rate as u64));
                    }
                    let _ = write!(line, "  files checked {}/{}", progress.files_done, progress.files_total);
                    say(self.out, &line);
                }
            }
            _ => {}
        }
    }
}

fn sha256_of(path: &Path) -> std::io::Result<[u8; 32]> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            return Ok(hasher.finalize().into());
        }
        hasher.update(&buffer[..n]);
    }
}

/// A finished install whose executable is still the pinned one needs nothing more.
fn already_installed(files: &Files, plan: &Plan, folder: &Path) -> bool {
    let Ok(record) = std::fs::read_to_string(files.installed_record(&plan.version)) else {
        return false;
    };
    Path::new(record.trim()) == folder
        && sha256_of(&plan.executable.path.under(folder)).is_ok_and(|actual| actual == plan.executable.sha256)
}

/// Installs `plan` into `<dir>/<version>`: signs in, checks the library, fetches the pinned manifest, downloads
/// and checks every file, resuming whatever an earlier run left (`Files::resume_file`).
pub async fn install_game<L: Library>(
    env: &Env<'_, L>,
    files: &Files,
    plan: &Plan,
    dir: &Path,
    control: &Control,
    ask_code: &mut dyn FnMut() -> Result<String, HostError>,
    out: &mut (dyn FnMut(&str) + Send),
) -> Result<Done, HostError> {
    let out: Out<'_> = Mutex::new(out);
    let folder = dir.join(&plan.version);
    let session = session(env.accounts, &files.session, ask_code, &out).await?;

    // Before anything is fetched or created: an account without Dauntless gets nothing (DE3)
    env.ownership
        .require(&session)
        .await
        .map_err(|e| HostError::message(ownership_message(&e)))?;
    say(&out, "Dauntless is in this Epic library.");

    if already_installed(files, plan, &folder) {
        say(
            &out,
            &format!("{} is already installed in {}.", plan.display_name, folder.display()),
        );
        return Ok(Done {
            folder,
            already: true,
            files: 0,
            bytes: 0,
        });
    }

    let https = Https::new(env.options.tls.clone(), env.options.timeout)
        .map_err(|e| HostError::message(format!("could not set up the connection: {e}")))?;
    let (manifest, origin) = fetch_manifest(
        None::<std::future::Ready<Result<Vec<u8>, String>>>,
        &plan.sources,
        &plan.pin,
        &https,
    )
    .await
    .map_err(|unavailable| {
        let tried: Vec<String> = unavailable.tried.iter().map(|(_, why)| why.clone()).collect();
        HostError::message(format!(
            "no source gave the pinned manifest for {} ({}); try again later",
            plan.version,
            tried.join("; ")
        ))
    })?;
    if manifest.app_name != APP_NAME || manifest.build_version != plan.build_version {
        return Err(HostError::message(format!(
            "the manifest is not {} {}",
            APP_NAME, plan.build_version
        )));
    }
    if let ManifestOrigin::Source(url) = &origin {
        say(&out, &format!("Manifest for {}: {url}", plan.build_version));
    }
    say(
        &out,
        &format!(
            "Installing {} ({}) into {}: {} to download, {} on disk, {} files.",
            plan.display_name,
            plan.build_version,
            folder.display(),
            size(manifest.download_size()),
            size(manifest.install_size()),
            manifest.files().len()
        ),
    );

    std::fs::create_dir_all(&files.data).map_err(|e| HostError::io(&files.data, e))?;
    let target = Target {
        manifest: &manifest,
        manifest_sha1: plan.pin,
        folder: folder.clone(),
        executable: Some(plan.executable.clone()),
        resume_file: files.resume_file(&plan.version),
    };
    let reporter = Reporter::new(&out);
    let installed = download::install(
        &target,
        &env.options,
        &Owned(env.ownership, &session),
        control,
        &|progress| reporter.update(progress),
    )
    .await
    .map_err(|e| HostError::message(download_message(&e)))?;
    let record = files.installed_record(&plan.version);
    std::fs::write(&record, installed.folder.to_string_lossy().as_bytes()).map_err(|e| HostError::io(&record, e))?;
    Ok(Done {
        folder: installed.folder,
        already: false,
        files: installed.files,
        bytes: installed.bytes,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn sizes_read_in_binary_units() {
        assert_eq!(size(0), "0 B");
        assert_eq!(size(1023), "1023 B");
        assert_eq!(size(1536), "1.5 KiB");
        assert_eq!(size(16 * 1024 * 1024 * 1024), "16.0 GiB");
    }

    #[test]
    fn the_hosts_pin_is_the_builds() {
        let build = builds::find("1.14.7").unwrap_or_else(|| panic!("no 1.14.7"));
        assert_eq!(build.executable_sha256, crate::paths::PINNED_EXE_SHA256);
        assert_eq!(
            builds::EXECUTABLE,
            crate::paths::game_exe_relative().to_string_lossy().replace('\\', "/")
        );
    }

    #[test]
    fn a_version_epic_cannot_give_is_refused_with_a_way_forward() {
        assert!(Plan::for_version("1.14.7").is_ok() && Plan::for_version("1.12.0").is_err());
        let other = Plan::for_version("2.0.0").unwrap_err().to_string();
        assert!(other.contains("1.14.7"), "{other}");
    }
}
