// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The player's side of the wire (RD27 spec §2.3.8): the snapshot and its sections, settings, and the answers
//! of the join and account commands.

use dainty_protocol::invite::ServerMode;
use dainty_protocol::status::ServerStatus;
use serde::{Deserialize, Serialize, Serializer};

use super::{Failed, LauncherError, NoticeCode, RuntimeFileInfo, RuntimeState};
use crate::core::i18n::Language;

/// The language the game itself is told to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub enum GameLanguage {
    /// Follow the launcher's language (§2.8.1).
    #[default]
    #[serde(rename = "auto")]
    Auto,
    /// English (US).
    #[serde(rename = "en-US")]
    EnUs,
    /// German.
    #[serde(rename = "de-DE")]
    DeDe,
    /// Spanish.
    #[serde(rename = "es-ES")]
    EsEs,
    /// French.
    #[serde(rename = "fr-FR")]
    FrFr,
    /// Italian.
    #[serde(rename = "it-IT")]
    ItIt,
    /// Japanese.
    #[serde(rename = "ja-JP")]
    JaJp,
    /// Brazilian Portuguese.
    #[serde(rename = "pt-BR")]
    PtBr,
    /// Russian.
    #[serde(rename = "ru-RU")]
    RuRu,
}

/// Which runtime build PLAY loads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum RuntimeProfile {
    /// The published runtime.
    #[default]
    Production,
    /// A host's own build.
    Development,
}

/// Where the launcher is in getting the player into the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    /// Reading stored state.
    Loading,
    /// No server yet: the page asks for an invite.
    Join,
    /// Reaching the server.
    Connect,
    /// No account on this server yet.
    Register,
    /// The game folder is missing or wrong.
    Locate,
    /// The runtime needs trust or an install.
    Runtime,
    /// A download or install is running.
    Installing,
    /// PLAY is available.
    Ready,
    /// The game is running.
    Running,
    /// The server asks for a newer launcher: PLAY is off until it updates.
    #[serde(rename = "launcher_too_old")]
    LauncherTooOld,
}

/// Why the server could not be reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ConnectProblem {
    /// A private server, and Tailscale is not installed.
    TailscaleMissing,
    /// Nothing answered.
    Unreachable,
    /// Something answered, but not as a server does.
    BadAnswer,
    /// The certificate is not the invite's.
    CertMismatch,
}

/// What is wrong with the chosen game folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum GameProblem {
    /// No folder chosen.
    NotSet,
    /// The folder or its executable is gone.
    NotFound,
    /// The executable's hash is not known.
    Unverified,
    /// Another game version.
    WrongVersion,
    /// 1.14.7 by its version text, but another build of the executable.
    WrongBuild,
    /// Game files, but no `Archon` folder with the executable.
    MissingArchon,
    /// A Dauntless Revived copy.
    Revived,
}

/// The game a server runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum ServerGame {
    /// The current game.
    Reforged,
    /// The 1.4.4 game.
    Revived,
}

/// The game edition the settings choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum Game {
    /// The current game.
    #[default]
    Reborn,
    /// The 1.4.4 game.
    Legacy,
}

impl Game {
    /// The edition a game version belongs to.
    pub fn from_version(version: &str) -> Self {
        if version == "1.4.4" { Self::Legacy } else { Self::Reborn }
    }
}

/// The game's graphics preset: −1 leaves it to the game's menu, 0..=4 are Low..Cinematic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "i64", into = "i8")]
pub struct GraphicsPreset(i8);

impl GraphicsPreset {
    /// Leave the preset to the game.
    pub const UNSET: Self = Self(-1);

    /// The preset `value`, if it is one.
    pub const fn new(value: i64) -> Option<Self> {
        match value {
            #[allow(clippy::cast_possible_truncation, reason = "the range is checked")]
            -1..=4 => Some(Self(value as i8)),
            _ => None,
        }
    }

    /// Its number.
    pub const fn get(self) -> i8 {
        self.0
    }
}

impl Default for GraphicsPreset {
    fn default() -> Self {
        Self::UNSET
    }
}

impl TryFrom<i64> for GraphicsPreset {
    type Error = String;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        Self::new(value).ok_or_else(|| format!("graphics preset {value} is not -1..4"))
    }
}

impl From<GraphicsPreset> for i8 {
    fn from(preset: GraphicsPreset) -> Self {
        preset.0
    }
}

/// What a running task is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum TaskKind {
    /// Hashing the game's files.
    Verify,
    /// Fetching the runtime.
    Runtime,
    /// Downloading files from the server.
    Download,
    /// Placing the runtime's DLLs.
    Dlls,
    /// Installing the game.
    Install,
}

/// How far a task has got; sent on the progress channel (§2.3.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TaskProgress {
    /// The task.
    pub kind: TaskKind,
    /// Bytes in all.
    pub total_bytes: u64,
    /// Bytes done.
    pub done_bytes: u64,
    /// Files in all.
    pub files_total: u32,
    /// Files done.
    pub files_done: u32,
    /// Recent speed.
    pub bytes_per_second: u64,
    /// Seconds left, once the speed is known.
    pub eta_seconds: Option<u64>,
    /// The file being worked on.
    pub current_file: Option<String>,
}

impl TaskProgress {
    /// Every byte and file is done.
    pub const fn is_final(&self) -> bool {
        self.done_bytes >= self.total_bytes && self.files_done >= self.files_total
    }
}

/// The server the launcher is joined to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    /// The game it runs.
    pub game: ServerGame,
    /// Private or public.
    pub mode: ServerMode,
    /// Its address.
    pub host: String,
    /// Its port.
    pub port: u16,
    /// Its name.
    pub name: String,
    /// The invite carried a Tailscale share link.
    pub has_share: bool,
    /// An invite is waiting to be used for registration.
    pub has_pending_invite: bool,
    /// It runs on this PC.
    pub loopback: bool,
    /// The pinned certificate's SHA-256, in public mode.
    pub fingerprint: Option<String>,
}

/// A server in the saved list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct SavedServerInfo {
    /// Its id, 24 lower-case hex digits.
    pub id: String,
    /// The game it runs.
    pub game: ServerGame,
    /// Private or public.
    pub mode: ServerMode,
    /// Its address.
    pub host: String,
    /// Its port.
    pub port: u16,
    /// Its name.
    pub name: String,
    /// The one joined now.
    pub current: bool,
    /// The account's name there, when known.
    pub username: Option<String>,
    /// This build cannot play on it.
    pub unsupported: bool,
}

/// Where a game folder was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum GameSource {
    /// The Epic Games launcher's manifests.
    Epic,
    /// Legendary's install list.
    Legendary,
    /// Heroic's install list.
    Heroic,
    /// The player chose it.
    Chosen,
    /// The 1.4.4 install.
    Revived,
}

/// A game folder found on this PC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct DetectedGame {
    /// The folder.
    pub root: String,
    /// Where it was found.
    pub source: GameSource,
    /// Its version, when it could be read.
    pub version: Option<String>,
}

/// The game folder in use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct GameInfo {
    /// The folder, once chosen.
    pub root: Option<String>,
    /// What is wrong with it.
    pub problem: Option<GameProblem>,
    /// The version it reports.
    pub version_text: Option<String>,
    /// The version is the server's.
    pub version_ok: bool,
    /// The executable's hash is known.
    pub exe_verified: bool,
    /// Other folders found.
    pub detected: Vec<DetectedGame>,
}

/// The 1.4.4 install (null in these builds).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RevivedInfo {
    /// Its folder.
    pub root: String,
    /// The player chose it.
    pub chosen: bool,
    /// What is wrong with it.
    pub problem: Option<GameProblem>,
    /// Files still to fetch.
    pub missing_files: u32,
    /// Bytes still to fetch.
    pub total_bytes: u64,
    /// Its DLLs are in place.
    pub dlls_ok: bool,
    /// Other folders found.
    pub detected: Vec<DetectedGame>,
}

/// What `revived_install` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum InstallMode {
    /// Fetch what is missing.
    Install,
    /// Fetch what is missing or different.
    Repair,
}

/// The runtime PLAY would load.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    /// Whether it can be used.
    pub state: RuntimeState,
    /// Its version, when known.
    pub version: Option<String>,
    /// Its files.
    pub files: Vec<RuntimeFileInfo>,
    /// A local runtime folder that replaces the server's.
    pub local_dir: Option<String>,
}

/// The player's settings, as the page shows them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Graphics preset written to the game's config.
    #[cfg_attr(any(test, feature = "ts"), ts(type = "GraphicsPreset"))]
    pub graphics: GraphicsPreset,
    /// Windowed rather than full screen.
    pub windowed: bool,
    /// The launcher's language.
    pub language: Language,
    /// The game's language.
    pub game_language: GameLanguage,
    /// Which runtime build to load.
    pub runtime_profile: RuntimeProfile,
    /// Hide addresses and names on screen.
    pub streamer_mode: bool,
    /// Show the Host page.
    pub host_tools: bool,
    /// The game edition.
    pub game_edition: Game,
    /// Minimising hides the window in the tray.
    pub minimize_to_tray: bool,
    /// Closing hides the window in the tray.
    pub close_to_tray: bool,
    /// Show the game's console window.
    pub show_game_console: bool,
    /// Under Proton, load the runtime with the `dainty-inject.exe` helper rather than the runtime loader.
    pub inject_helper: bool,
}

/// A change to the settings: only the named ones change. Anything else is refused whole (§2.3.4).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsPatch {
    /// New graphics preset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graphics: Option<GraphicsPreset>,
    /// New windowed flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windowed: Option<bool>,
    /// New streamer mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streamer_mode: Option<bool>,
    /// New Host page flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_tools: Option<bool>,
    /// New launcher language.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<Language>,
    /// New game language.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_language: Option<GameLanguage>,
    /// New runtime profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_profile: Option<RuntimeProfile>,
    /// New game edition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_edition: Option<Game>,
    /// New minimise-to-tray flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimize_to_tray: Option<bool>,
    /// New close-to-tray flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_to_tray: Option<bool>,
    /// New game console flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_game_console: Option<bool>,
    /// New Proton helper flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inject_helper: Option<bool>,
}

object_only!(SettingsPatch);

/// The connection's state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SnapshotConnect {
    /// A check is running.
    pub checking: bool,
    /// Why the last check failed.
    pub problem: Option<ConnectProblem>,
    /// Tailscale is installed.
    pub tailscale_installed: bool,
    /// When the server last answered (ISO 8601).
    pub last_checked_at: Option<String>,
}

/// The account on the joined server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SnapshotAccount {
    /// Its name.
    pub username: Option<String>,
    /// A key is stored for it.
    pub has_key: bool,
    /// The page should offer a key backup.
    pub offer_backup: bool,
}

/// What is running.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SnapshotRunning {
    /// The game.
    pub game: bool,
    /// The local relay's port.
    pub relay_port: Option<u16>,
}

/// Facts about this launcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SnapshotApp {
    /// Its version.
    pub version: String,
    /// An installed release rather than a development run.
    pub packaged: bool,
    /// An update is downloaded and waiting.
    pub update_ready: bool,
    /// `win32` or `linux`.
    pub platform: String,
    /// This build has the Host page.
    pub host_tools_available: bool,
    /// This build plays 1.4.4.
    pub legacy_available: bool,
    /// Game versions this build plays.
    pub game_versions: Vec<String>,
    /// This build is the server manager.
    pub server_gui: bool,
}

/// Everything the page draws, published whenever it changes (event `snapshot`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// Where the launcher is.
    pub phase: Phase,
    /// A command is running.
    pub busy: bool,
    /// The joined server.
    pub server: Option<ServerInfo>,
    /// The saved servers.
    pub saved_servers: Vec<SavedServerInfo>,
    /// The connection.
    pub connect: SnapshotConnect,
    /// The account.
    pub account: SnapshotAccount,
    /// The game folder.
    pub game: GameInfo,
    /// The 1.4.4 install.
    pub revived: Option<RevivedInfo>,
    /// The runtime.
    pub runtime: RuntimeInfo,
    /// The running task.
    pub task: Option<TaskProgress>,
    /// What is running.
    pub running: SnapshotRunning,
    /// The settings.
    pub settings: Settings,
    /// This launcher.
    pub app: SnapshotApp,
    /// The server's last status.
    pub status: Option<ServerStatus>,
    /// The server has no status route.
    pub status_unsupported: bool,
    /// The last failure, until dismissed.
    pub last_error: Option<LauncherError>,
    /// The last notice, until dismissed.
    pub notice: Option<NoticeCode>,
}

/// One news post.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct NewsItem {
    /// When it was posted.
    pub date: Option<String>,
    /// Its title.
    pub title: String,
    /// Its text.
    pub body: String,
}

/// A background picture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct Background {
    /// Where it is.
    pub url: String,
    /// Who made it.
    pub credit: Option<String>,
}

/// The server's look.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct Branding {
    /// Background pictures.
    pub backgrounds: Vec<Background>,
    /// Accent colour.
    pub accent: Option<String>,
}

/// The `check_invite` answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InviteCheck {
    /// The invite can be used.
    Ok {
        /// The server's certificate differs from the saved one.
        certificate_changed: bool,
        /// The saved certificate's fingerprint, when it changed.
        previous_fingerprint: Option<String>,
        /// The saved server this one replaces at a new address.
        moved_from: Option<String>,
    },
    /// It cannot.
    Err(LauncherError),
}

impl Serialize for InviteCheck {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Checked<'a> {
            ok: bool,
            certificate_changed: bool,
            previous_fingerprint: &'a Option<String>,
            moved_from: &'a Option<String>,
        }
        match self {
            Self::Ok {
                certificate_changed,
                previous_fingerprint,
                moved_from,
            } => Checked {
                ok: true,
                certificate_changed: *certificate_changed,
                previous_fingerprint,
                moved_from,
            }
            .serialize(serializer),
            Self::Err(error) => Failed { ok: false, error }.serialize(serializer),
        }
    }
}

/// The `register` answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterOutcome {
    /// Registered under this name.
    Ok {
        /// The name the server took.
        username: String,
    },
    /// Refused.
    Err(LauncherError),
}

impl Serialize for RegisterOutcome {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Registered<'a> {
            ok: bool,
            username: &'a str,
        }
        match self {
            Self::Ok { username } => Registered { ok: true, username }.serialize(serializer),
            Self::Err(error) => Failed { ok: false, error }.serialize(serializer),
        }
    }
}

/// The `epic_library_check` answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EpicLibraryCheck {
    /// The library was read.
    Ok {
        /// The account owns the game.
        owned: bool,
        /// When (ISO 8601).
        checked_at: String,
    },
    /// It could not be.
    Err(LauncherError),
}

impl Serialize for EpicLibraryCheck {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Checked<'a> {
            ok: bool,
            owned: bool,
            checked_at: &'a str,
        }
        match self {
            Self::Ok { owned, checked_at } => Checked {
                ok: true,
                owned: *owned,
                checked_at,
            }
            .serialize(serializer),
            Self::Err(error) => Failed { ok: false, error }.serialize(serializer),
        }
    }
}
