//! The values that cross the bridge between the page and the backend. The 1.4.4 parts (`RevivedInfo`,
//! `Snapshot.revived`, the 1.4.4 error and notice codes) are in every build, so the page and the
//! generated types are the same with and without the `legacy` feature; only a build with it fills them.
//! The TypeScript declarations in `ui/packages/bridge/src/generated/types.ts` are generated from these
//! (`generate.rs`), so the page and the backend cannot drift apart.

use protocol::GameVersion;
use protocol::invite::ServerMode;
use protocol::status::ServerStatus;
use serde::{Deserialize, Serialize, Serializer};

/// The launcher's languages, made from the page's strings by build.rs (L14; RA3 makes them `en` and
/// `pt-BR`).
pub use crate::core::i18n::Language;
pub use launcher_kit::bridge::types::*;

/// The game the launcher plays, as settings.json's `gameEdition` and the page name it (`reborn` 1.14.7,
/// `legacy` 1.4.4). The launcher's own setting, not the servers' vocabulary: invites and modules name a
/// [`GameVersion`] (DD4), and DU3 moves the setting onto the client profiles' versions (DD12).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum Game {
    #[default]
    Reborn,
    Legacy,
}

impl Game {
    /// The edition that plays `version`: legacy for 1.4.4, reborn for every other.
    pub fn of(version: &GameVersion) -> Self {
        if protocol::invite::is_revived(version) {
            Self::Legacy
        } else {
            Self::Reborn
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reborn => "reborn",
            Self::Legacy => "legacy",
        }
    }
}

impl std::fmt::Display for Game {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// -1 = leave graphics to the in-game menu, 0..4 = Low, Medium, High, Epic, Cinematic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "i8", into = "i8")]
pub struct GraphicsPreset(i8);

impl GraphicsPreset {
    pub const GAME_MENU: Self = Self(-1);
    /// `DEFAULT_GRAPHICS`: leave graphics to the game
    pub const DEFAULT: Self = Self::GAME_MENU;

    pub fn value(self) -> i8 {
        self.0
    }
}

impl TryFrom<i8> for GraphicsPreset {
    type Error = &'static str;

    fn try_from(value: i8) -> Result<Self, Self::Error> {
        if (-1..=4).contains(&value) {
            Ok(Self(value))
        } else {
            Err("a graphics preset is -1 to 4")
        }
    }
}

impl From<GraphicsPreset> for i8 {
    fn from(preset: GraphicsPreset) -> Self {
        preset.0
    }
}

/// The game's own text language, sent to it as -epiclocale; "auto" follows the operating system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub enum GameLanguage {
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "en-US")]
    EnUs,
    #[serde(rename = "de-DE")]
    DeDe,
    #[serde(rename = "es-ES")]
    EsEs,
    #[serde(rename = "fr-FR")]
    FrFr,
    #[serde(rename = "it-IT")]
    ItIt,
    #[serde(rename = "ja-JP")]
    JaJp,
    #[serde(rename = "pt-BR")]
    PtBr,
    #[serde(rename = "ru-RU")]
    RuRu,
}

/// The game runtime's log profile (`RUSTIC_ISLES_RUNTIME_PROFILE`). "development" also honours the
/// runtime's diagnostic .flag files next to the game executable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum RuntimeProfile {
    Production,
    Development,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Loading,
    /// No server yet: paste an invite
    Join,
    /// Server known, not reachable (or still checking)
    Connect,
    /// Server reachable, no account key yet
    Register,
    /// Account ready, but no Dauntless 1.14.7 folder chosen, found or accepted
    Locate,
    /// Game found; the server's game runtime is not installed, out of date or not trusted yet
    Runtime,
    /// A check or the runtime download is running
    Installing,
    /// PLAY
    Ready,
    /// The game is running
    Running,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ConnectProblem {
    /// Private mode only
    TailscaleMissing,
    /// No answer within 5 s
    Unreachable,
    /// Something answered but it is not a Reforged server
    BadAnswer,
    /// Public mode: the certificate is not the one from the invite
    CertMismatch,
}

/// Why the chosen game folder cannot be used yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum GameProblem {
    /// Nothing chosen and nothing found on this PC
    NotSet,
    /// The chosen folder no longer holds the game
    NotFound,
    /// Found, but its executable has not been checked yet (or changed since)
    Unverified,
    /// The executable is not the Dauntless 1.14.7 build (CL 647472)
    WrongVersion,
}

/// Which game a saved server plays, as settings.json names it (`game`): `revived` for a Dauntless
/// Revived (1.4.4) server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum ServerGame {
    Reforged,
    Revived,
}

#[derive(Serialize)]
struct Failed<'a> {
    ok: bool,
    error: &'a LauncherError,
}

/// Asked before joining from an invite. `certificate_changed`: this PC has a key for the invite's
/// host:port that belongs to another certificate (`previous_fingerprint`, None if not recorded).
/// `moved_from`: a private server this PC has an account on, now at the invite's new address (its old
/// address).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InviteCheck {
    Ok {
        certificate_changed: bool,
        previous_fingerprint: Option<String>,
        moved_from: Option<String>,
    },
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterOutcome {
    Ok { username: String },
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

/// `epic_library_check` (DE3): whether the signed-in Epic account owns Dauntless, and when Epic last said so
/// (`checkedAt`, as `toISOString` writes it; the answer is cached a while). Not owning it is `owned: false`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EpicLibraryCheck {
    Ok { owned: bool, checked_at: String },
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum TaskKind {
    /// Hashing the game executable
    Verify,
    /// Downloading and installing the server's runtime
    Runtime,
    /// Downloading Dauntless 1.4.4
    Download,
    /// Installing the 1.4.4 client DLLs
    Dlls,
    /// Installing a version from Epic (DE5): bytes are the chunks', files those checked
    Install,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TaskProgress {
    pub kind: TaskKind,
    pub total_bytes: u64,
    pub done_bytes: u64,
    pub files_total: u32,
    pub files_done: u32,
    pub bytes_per_second: f64,
    pub eta_seconds: Option<f64>,
    pub current_file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub game: ServerGame,
    pub mode: ServerMode,
    pub host: String,
    pub port: u16,
    pub name: String,
    pub has_share: bool,
    /// An invite code is stored and not used yet
    pub has_pending_invite: bool,
    pub loopback: bool,
    /// Public mode: the pinned certificate fingerprint (not secret)
    pub fingerprint: Option<String>,
}

/// A server joined before, for switching back to it without the invite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct SavedServerInfo {
    /// The account key slot id (not secret)
    pub id: String,
    pub game: ServerGame,
    pub mode: ServerMode,
    pub host: String,
    pub port: u16,
    pub name: String,
    pub current: bool,
    pub username: Option<String>,
    /// A Dauntless Revived (1.4.4) server in a build without the `legacy` feature: listed with its key
    /// kept, never joined (RD5, RD22)
    pub unsupported: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum GameSource {
    Epic,
    Legendary,
    Heroic,
    Chosen,
    /// Dauntless Revived's own launcher's 1.4.4 folder
    Revived,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DetectedGame {
    pub root: String,
    pub source: GameSource,
    /// What the store says (Legendary, Heroic), or Version.txt
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct GameInfo {
    /// The "Dauntless" folder, the one that holds Archon\
    pub root: Option<String>,
    pub problem: Option<GameProblem>,
    /// Version.txt, e.g. dauntless_rel-1.14.7_Shipping_2024-04-01_14-59-21_647472
    pub version_text: Option<String>,
    /// Version.txt names 1.14.7, changelist 647472
    pub version_ok: bool,
    /// The executable's SHA-256 is the 1.14.7 one (checked once per file change)
    pub exe_verified: bool,
    /// Installs found on this PC (Epic, Legendary, Heroic)
    pub detected: Vec<DetectedGame>,
}

/// Dauntless 1.4.4, for a Dauntless Revived server: the folder it is in (or goes into), and whether every
/// file is the pinned one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RevivedInfo {
    /// The chosen folder, or where a download goes when none is chosen
    pub root: String,
    /// The player chose it (or it was found), rather than the default
    pub chosen: bool,
    /// `not_set`: nothing there yet; `unverified`: there, but not every file checked
    pub problem: Option<GameProblem>,
    /// Files to download, as far as a look by size knows
    pub missing_files: u32,
    /// The whole game
    pub total_bytes: u64,
    /// The pinned Undaunted DLLs are next to the game
    pub dlls_ok: bool,
    /// 1.4.4 installs found on this PC (Dauntless Revived's launcher)
    pub detected: Vec<DetectedGame>,
}

/// `revivedInstall`'s mode: everything checked once by hash, or (repair) always
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum InstallMode {
    Install,
    Repair,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    pub state: RuntimeState,
    /// The server's runtime version
    pub version: Option<String>,
    /// What the server offers (shown when the player is asked to trust it)
    pub files: Vec<RuntimeFileInfo>,
    pub local_dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[cfg_attr(test, ts(type = "GraphicsPreset"))]
    pub graphics: GraphicsPreset,
    pub windowed: bool,
    pub language: Language,
    pub game_language: GameLanguage,
    pub runtime_profile: RuntimeProfile,
    /// Hide addresses, fingerprints, paths and player names on screen
    pub streamer_mode: bool,
    /// Show the Host page in the rail
    pub host_tools: bool,
    /// Which game the launcher plays: its folder, pins and PLAY. Always `reborn` without the `legacy` feature
    pub game_edition: Game,
    /// Minimising hides the window to the tray
    pub minimize_to_tray: bool,
    /// Closing hides the window to the tray; the tray's Quit closes it
    pub close_to_tray: bool,
    /// Leave the game's console window on screen; hidden otherwise, its lines read into Game log
    pub show_game_console: bool,
}

/// What `setSettings` may change: any of the settings, each checked; an unknown field or a wrong value
/// refuses the whole patch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsPatch {
    pub graphics: Option<GraphicsPreset>,
    pub windowed: Option<bool>,
    pub streamer_mode: Option<bool>,
    pub host_tools: Option<bool>,
    pub language: Option<Language>,
    pub game_language: Option<GameLanguage>,
    pub runtime_profile: Option<RuntimeProfile>,
    pub game_edition: Option<Game>,
    pub minimize_to_tray: Option<bool>,
    pub close_to_tray: Option<bool>,
    pub show_game_console: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SnapshotConnect {
    pub checking: bool,
    pub problem: Option<ConnectProblem>,
    pub tailscale_installed: bool,
    pub last_checked_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SnapshotAccount {
    pub username: Option<String>,
    pub has_key: bool,
    pub offer_backup: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SnapshotRunning {
    pub game: bool,
    pub relay_port: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SnapshotApp {
    pub version: String,
    pub packaged: bool,
    pub update_ready: bool,
    /// `process.platform`: "win32" or "linux"
    pub platform: String,
    /// False in a player-only build (no `host` feature): the page offers no Host tools
    pub host_tools_available: bool,
    /// False without the `legacy` feature: the page offers no edition setting
    pub legacy_available: bool,
    /// The game versions an invite may name in this build (`protocol::game::supported_versions`)
    pub game_versions: Vec<String>,
    /// The server GUI (`server-gui` feature): the page shows the server screens (DU3)
    pub server_gui: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub phase: Phase,
    pub busy: bool,
    pub server: Option<ServerInfo>,
    pub saved_servers: Vec<SavedServerInfo>,
    pub connect: SnapshotConnect,
    pub account: SnapshotAccount,
    pub game: GameInfo,
    /// The current server is a Dauntless Revived one, in the legacy edition
    pub revived: Option<RevivedInfo>,
    pub runtime: RuntimeInfo,
    pub task: Option<TaskProgress>,
    pub running: SnapshotRunning,
    pub settings: Settings,
    pub app: SnapshotApp,
    pub status: Option<ServerStatus>,
    /// The host answers but has no ServerStatus endpoint
    pub status_unsupported: bool,
    pub last_error: Option<LauncherError>,
    pub notice: Option<NoticeCode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct NewsItem {
    pub date: Option<String>,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Background {
    /// An app-internal `rr-art://` URL
    pub url: String,
    pub credit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Branding {
    pub backgrounds: Vec<Background>,
    pub accent: Option<String>,
}

/// `epic_get`: who is signed in to Epic (DE2). The session itself stays in the keystore (DD22).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct EpicView {
    pub signed_in: bool,
    /// The account's Epic display name; null when signed out or Epic gave none
    pub display_name: Option<String>,
    /// The sign-in window is open, or a code is being exchanged
    pub signing_in: bool,
}

/// Where a version's install from Epic stands (DE5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum InstallState {
    NotInstalled,
    /// An earlier download stopped (cancelled, failed or the launcher closed): starting picks it up
    Resumable,
    Preparing,
    Downloading,
    Paused,
    /// Checking the executable
    Finishing,
    Installed,
    Failed,
}

/// One version's install from Epic, as `game_install_get` answers it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct InstallInfo {
    pub version: String,
    pub state: InstallState,
    /// The version's own folder
    pub folder: Option<String>,
    pub progress: Option<TaskProgress>,
    /// Why the last attempt stopped
    pub error: Option<LauncherError>,
}

/// `game_install_start` and `game_install_get`: `{ ok: true, install } | { ok: false, error }`.
#[derive(Debug, Clone, PartialEq)]
pub enum InstallOutcome {
    Ok(InstallInfo),
    Err(LauncherError),
}

impl Serialize for InstallOutcome {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Found<'a> {
            ok: bool,
            install: &'a InstallInfo,
        }
        match self {
            Self::Ok(install) => Found { ok: true, install }.serialize(serializer),
            Self::Err(error) => Failed { ok: false, error }.serialize(serializer),
        }
    }
}

/// Where a guarded admin action stands (DH6; the admin API's `GuardedState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum GuardedState {
    /// Waiting for its delay to end or for a second operator
    Pending,
    /// Approved; the server runs it within seconds
    Approved,
    Running,
    Done,
    Failed,
    Canceled,
    Expired,
}

/// What a guarded action waits for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum GuardedMode {
    /// Its delay ends and it runs by itself
    Delay,
    /// A second operator approves it
    Approval,
}

/// One destructive or bulk admin action held for a delay or a second operator (DD23, DH6). Times are RFC 3339.
/// Never the parameters it runs with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct GuardedActionInfo {
    /// 16 hex digits
    pub id: String,
    /// `ban`, `timeout`, `revokeKey` or `revert`
    pub kind: String,
    /// One line of what it will do, in English: it comes from the admin API
    pub summary: String,
    pub state: GuardedState,
    pub mode: GuardedMode,
    /// The admin key that asked: `owner` or a key id
    pub initiated_by: String,
    /// The lowest admin role that may approve it
    pub needs_role: String,
    pub created_at: String,
    /// When a delay ends; null for an approval
    pub release_at: Option<String>,
    pub expires_at: String,
    pub approved_by: Option<String>,
    pub canceled_by: Option<String>,
    pub ended_at: Option<String>,
    pub result: Option<String>,
}

/// `guarded_list`: `{ ok: true, actions } | { ok: false, error }`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
    not(feature = "server-gui"),
    allow(dead_code, reason = "only the server GUI fills it")
)]
pub enum GuardedList {
    Ok(Vec<GuardedActionInfo>),
    Err(LauncherError),
}

impl Serialize for GuardedList {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Listed<'a> {
            ok: bool,
            actions: &'a [GuardedActionInfo],
        }
        match self {
            Self::Ok(actions) => Listed { ok: true, actions }.serialize(serializer),
            Self::Err(error) => Failed { ok: false, error }.serialize(serializer),
        }
    }
}

/// `guarded_approve` and `guarded_cancel`: `{ ok: true, action } | { ok: false, error }`. The action is as it
/// stands after: approved (the server runs it within seconds) or canceled.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
    not(feature = "server-gui"),
    allow(dead_code, reason = "only the server GUI fills it")
)]
pub enum GuardedOutcome {
    Ok(Box<GuardedActionInfo>),
    Err(LauncherError),
}

impl Serialize for GuardedOutcome {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Decided<'a> {
            ok: bool,
            action: &'a GuardedActionInfo,
        }
        match self {
            Self::Ok(action) => Decided { ok: true, action }.serialize(serializer),
            Self::Err(error) => Failed { ok: false, error }.serialize(serializer),
        }
    }
}

// ------------------------------------------------------------------ history and revert (DH3b)

/// One change in a change log: who changed which row, and when. The rows are JSON text with an admin
/// key's hash hidden.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntryView {
    pub id: String,
    /// `node` (the game), `admin` or `chat`
    pub database: String,
    /// RFC 3339
    pub at: String,
    /// `player:<account>`, `admin:<key id>`, `host:<command>` or `system:<task>`
    pub actor: String,
    pub table: String,
    /// The row's key, as JSON
    pub key: String,
    /// `insert`, `update`, `delete`, `soft_delete`, `restore` or `migrate`
    pub op: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

/// Changes newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HistoryPageView {
    pub entries: Vec<HistoryEntryView>,
    /// Pass it back as the filter's `cursor` for the next, older page; null at the end
    pub next: Option<String>,
}

/// `history_query`: `{ ok: true, page } | { ok: false, error }`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
    not(feature = "server-gui"),
    allow(dead_code, reason = "only the server GUI answers with a page")
)]
pub enum HistoryOutcome {
    Ok(HistoryPageView),
    Err(LauncherError),
}

impl Serialize for HistoryOutcome {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Found<'a> {
            ok: bool,
            page: &'a HistoryPageView,
        }
        match self {
            Self::Ok(page) => Found { ok: true, page }.serialize(serializer),
            Self::Err(error) => Failed { ok: false, error }.serialize(serializer),
        }
    }
}

/// One row a revert changes
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RevertRowView {
    pub database: String,
    pub table: String,
    pub key: String,
    /// `insert` (a deleted row put back), `update`, `delete` (an added row removed) or `soft_delete`
    pub kind: String,
    /// The row now and as the revert leaves it, as JSON
    pub current: Option<String>,
    pub target: Option<String>,
}

/// A row the revert leaves as it is
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RevertConflictView {
    pub database: String,
    pub table: String,
    pub key: String,
    /// `changed_since`, `not_as_logged`, `key_changed` or `schema_changed`
    pub reason: String,
    /// `changed_since`: the newer entry, who made it and when
    pub entry: Option<String>,
    pub by: Option<String>,
    pub at: Option<String>,
    pub detail: Option<String>,
}

/// What a revert would do. `history_revert_apply` with `id` does exactly this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RevertPlanView {
    pub id: String,
    /// Log entries the selection matches
    pub entries: u32,
    pub action_count: u32,
    pub conflict_count: u32,
    pub unchanged_count: u32,
    /// The first rows of each list; `truncated` says the counts are higher
    pub actions: Vec<RevertRowView>,
    pub conflicts: Vec<RevertConflictView>,
    pub truncated: bool,
}

/// `history_revert_dry_run`: `{ ok: true, plan } | { ok: false, error }`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
    not(feature = "server-gui"),
    allow(dead_code, reason = "only the server GUI answers with a page")
)]
pub enum RevertPlanOutcome {
    Ok(RevertPlanView),
    Err(LauncherError),
}

impl Serialize for RevertPlanOutcome {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Found<'a> {
            ok: bool,
            plan: &'a RevertPlanView,
        }
        match self {
            Self::Ok(plan) => Found { ok: true, plan }.serialize(serializer),
            Self::Err(error) => Failed { ok: false, error }.serialize(serializer),
        }
    }
}

/// How many rows a revert changed in one database
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RevertedInView {
    pub database: String,
    pub rows: u32,
}

/// What a revert did
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RevertDoneView {
    /// RFC 3339; the revert's own entries carry it
    pub at: Option<String>,
    pub rows_changed: u32,
    pub databases: Vec<RevertedInView>,
    /// Rows left as they are
    pub conflicts: Vec<RevertConflictView>,
    pub conflict_count: u32,
    pub truncated: bool,
}

/// `history_revert_apply`: `{ ok: true, done } | { ok: false, error }`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
    not(feature = "server-gui"),
    allow(dead_code, reason = "only the server GUI answers with a page")
)]
pub enum RevertApplyOutcome {
    Ok(RevertDoneView),
    Err(LauncherError),
}

impl Serialize for RevertApplyOutcome {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Found<'a> {
            ok: bool,
            done: &'a RevertDoneView,
        }
        match self {
            Self::Ok(done) => Found { ok: true, done }.serialize(serializer),
            Self::Err(error) => Failed { ok: false, error }.serialize(serializer),
        }
    }
}

// ------------------------------------------------------------------ realms (RF7, DU5)

/// A local realm session's state (RF7, DU5)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
#[cfg_attr(
    not(feature = "realms"),
    allow(dead_code, reason = "only a build with realms fills it")
)]
pub enum RealmState {
    Starting,
    Ready,
    Running,
    Ended,
}

/// One hosted or joined realm session on this PC (RF7, DU5)
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(
    not(feature = "realms"),
    allow(dead_code, reason = "only a build with realms fills it")
)]
pub struct RealmSessionView {
    pub session: String,
    pub kind: RealmKind,
    pub hosting: bool,
    pub generation: u32,
    pub state: RealmState,
    /// Local UDP port the game or server routes through when ready
    pub local_port: Option<u16>,
    /// Clean exit or failure code when `state` is `ended`
    pub result: Option<String>,
}

/// The launcher's realm state on this PC (`realm_get`, `realm_host`, `realm_join`, `realm_stop`; RF7, DU5)
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(
    not(feature = "realms"),
    allow(dead_code, reason = "only a build with realms fills it")
)]
pub struct RealmView {
    /// Whether this build and PC can host or join a realm (`ParadoxTransport.dll` present and co-op not disabled)
    pub available: bool,
    /// Active or most recently ended session on this PC
    pub active: Option<RealmSessionView>,
    /// All tracked local sessions
    pub sessions: Vec<RealmSessionView>,
}

/// `realm_get`, `realm_host`, `realm_join`, `realm_stop`: `{ ok: true, realm } | { ok: false, error }`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    not(feature = "realms"),
    allow(dead_code, reason = "only a build with realms fills it")
)]
pub enum RealmOutcome {
    Ok(RealmView),
    Err(LauncherError),
}

impl Serialize for RealmOutcome {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Found<'a> {
            ok: bool,
            realm: &'a RealmView,
        }
        match self {
            Self::Ok(realm) => Found { ok: true, realm }.serialize(serializer),
            Self::Err(error) => Failed { ok: false, error }.serialize(serializer),
        }
    }
}

// ------------------------------------------------------------------ server GUI doctor, config, migration, invites and admin (B3)

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
#[cfg_attr(
    not(feature = "server-gui"),
    allow(dead_code, reason = "only the server GUI fills it")
)]
pub enum DoctorLevel {
    Ok,
    Warning,
    Problem,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(
    not(feature = "server-gui"),
    allow(dead_code, reason = "only the server GUI fills it")
)]
pub struct DoctorCheckView {
    pub level: DoctorLevel,
    pub label: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(
    not(feature = "server-gui"),
    allow(dead_code, reason = "only the server GUI fills it")
)]
pub struct DoctorReportView {
    pub checks: Vec<DoctorCheckView>,
    pub warnings: u32,
    pub problems: u32,
}

/// `server_doctor`: `{ ok: true, report } | { ok: false, error }`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    not(feature = "server-gui"),
    allow(dead_code, reason = "only the server GUI fills it")
)]
pub enum ServerDoctorOutcome {
    Ok(DoctorReportView),
    Err(LauncherError),
}

impl Serialize for ServerDoctorOutcome {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Found<'a> {
            ok: bool,
            report: &'a DoctorReportView,
        }
        match self {
            Self::Ok(report) => Found { ok: true, report }.serialize(serializer),
            Self::Err(error) => Failed { ok: false, error }.serialize(serializer),
        }
    }
}

/// Where MongoDB migration stands for this workspace (M1, M4, B3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
#[cfg_attr(
    not(feature = "server-gui"),
    allow(dead_code, reason = "only the server GUI fills it")
)]
pub enum MongoMigrationState {
    /// No MongoDB data or configuration found
    None,
    /// `.mongo-migrated` shows data was migrated and verified
    Migrated,
    /// Host opted out (`fresh_start = true`)
    Skipped,
    /// Unmigrated MongoDB data exists and must be migrated (or explicitly skipped) before starting
    Unmigrated,
    /// External MongoDB is configured from an earlier setup, has no migration marker, and is unreachable
    Unreachable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(
    not(feature = "server-gui"),
    allow(dead_code, reason = "only the server GUI fills it")
)]
pub struct MongoMigrationView {
    pub state: MongoMigrationState,
    pub source: Option<String>,
    pub detail: Option<String>,
}

/// `server_config_get` and `server_config_save`: the server's `host.toml` settings and migration status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(
    not(feature = "server-gui"),
    allow(dead_code, reason = "only the server GUI fills it")
)]
pub struct ServerConfigView {
    pub configured: bool,
    pub name: String,
    pub mode: HostServerMode,
    pub public_host: Option<String>,
    pub registration: String,
    pub database: String,
    pub max_hubs: u32,
    pub max_hunts: u32,
    pub fresh_start: bool,
    pub mongo_migration: MongoMigrationView,
    pub config_file: Option<String>,
}

/// `server_config_get`, `server_config_save`, `server_confirm_fresh_start`:
/// `{ ok: true, config } | { ok: false, error }`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    not(feature = "server-gui"),
    allow(dead_code, reason = "only the server GUI fills it")
)]
pub enum ServerConfigOutcome {
    Ok(ServerConfigView),
    Err(LauncherError),
}

impl Serialize for ServerConfigOutcome {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Found<'a> {
            ok: bool,
            config: &'a ServerConfigView,
        }
        match self {
            Self::Ok(config) => Found { ok: true, config }.serialize(serializer),
            Self::Err(error) => Failed { ok: false, error }.serialize(serializer),
        }
    }
}

/// One invite code on the running server (`server_invite_list`). The link is a secret: blurred in streamer
/// mode and never logged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(
    not(feature = "server-gui"),
    allow(dead_code, reason = "only the server GUI fills it")
)]
pub struct ServerInviteEntry {
    pub code: String,
    pub link: String,
    #[serde(rename = "for")]
    pub for_name: Option<String>,
    pub uses_remaining: u32,
    pub infinite_uses: bool,
    pub created_at: String,
}

/// `server_invite_list`: `{ ok: true, invites } | { ok: false, error }`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    not(feature = "server-gui"),
    allow(dead_code, reason = "only the server GUI fills it")
)]
pub enum ServerInviteListOutcome {
    Ok(Vec<ServerInviteEntry>),
    Err(LauncherError),
}

impl Serialize for ServerInviteListOutcome {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Listed<'a> {
            ok: bool,
            invites: &'a [ServerInviteEntry],
        }
        match self {
            Self::Ok(invites) => Listed { ok: true, invites }.serialize(serializer),
            Self::Err(error) => Failed { ok: false, error }.serialize(serializer),
        }
    }
}

// ------------------------------------------------------------------ remote management (DD27, RM4)

/// One enrolled server install tracked by a controller (`remote_status`, DD27).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(
    not(feature = "server-gui"),
    allow(dead_code, reason = "only the server GUI fills it")
)]
pub struct ManagedInstallView {
    pub node: String,
    pub region: String,
    pub label: Option<String>,
    /// `"online"`, `"degraded"`, `"offline"` or `"revoked"`
    pub state: String,
    pub enrolled_at_ms: i64,
    pub last_seen_ms: i64,
    pub revoked: bool,
    pub applied_revision: u64,
    pub players_online: u32,
    pub active_hunts: u32,
    pub version: Option<String>,
    pub queued_commands: u32,
}

/// Remote management state for the current workspace (`remote_status`, DD27).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(
    not(feature = "server-gui"),
    allow(dead_code, reason = "only the server GUI fills it")
)]
pub struct RemoteStatusView {
    /// `"standalone"`, `"controller"` or `"managed"`
    pub role: String,
    pub node: Option<String>,
    pub region: Option<String>,
    pub controller_url: Option<String>,
    pub controller_cert_fp: Option<String>,
    pub controller_node: Option<String>,
    pub revision: u64,
    pub pending_enrollments: u32,
    pub installs: Vec<ManagedInstallView>,
    pub detached: bool,
}

/// `remote_status`, `remote_init_controller`, `remote_enroll`, `remote_revoke`, `remote_leave`:
/// `{ ok: true, status } | { ok: false, error }`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    not(feature = "server-gui"),
    allow(dead_code, reason = "only the server GUI fills it")
)]
pub enum RemoteStatusOutcome {
    Ok(RemoteStatusView),
    Err(LauncherError),
}

impl Serialize for RemoteStatusOutcome {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Found<'a> {
            ok: bool,
            status: &'a RemoteStatusView,
        }
        match self {
            Self::Ok(status) => Found { ok: true, status }.serialize(serializer),
            Self::Err(error) => Failed { ok: false, error }.serialize(serializer),
        }
    }
}

/// One-time enrollment invite minted on a controller (`remote_invite`, DD27).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(
    not(feature = "server-gui"),
    allow(dead_code, reason = "only the server GUI fills it")
)]
pub struct RemoteInviteView {
    pub uri: String,
    pub node: String,
    pub region: String,
    pub label: Option<String>,
    pub controller_url: String,
    pub controller_fp: String,
    pub expires_at_ms: i64,
}

/// `remote_invite`: `{ ok: true, invite } | { ok: false, error }`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    not(feature = "server-gui"),
    allow(dead_code, reason = "only the server GUI fills it")
)]
pub enum RemoteInviteOutcome {
    Ok(RemoteInviteView),
    Err(LauncherError),
}

impl Serialize for RemoteInviteOutcome {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Found<'a> {
            ok: bool,
            invite: &'a RemoteInviteView,
        }
        match self {
            Self::Ok(invite) => Found { ok: true, invite }.serialize(serializer),
            Self::Err(error) => Failed { ok: false, error }.serialize(serializer),
        }
    }
}

/// Queued remote command on a controller (`remote_exec`, DD27).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(
    not(feature = "server-gui"),
    allow(dead_code, reason = "only the server GUI fills it")
)]
pub struct RemoteExecView {
    pub command_id: String,
    pub node: String,
    pub action: String,
    pub status: String,
}

/// `remote_exec`: `{ ok: true, result } | { ok: false, error }`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    not(feature = "server-gui"),
    allow(dead_code, reason = "only the server GUI fills it")
)]
pub enum RemoteExecOutcome {
    Ok(RemoteExecView),
    Err(LauncherError),
}

impl Serialize for RemoteExecOutcome {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Found<'a> {
            ok: bool,
            result: &'a RemoteExecView,
        }
        match self {
            Self::Ok(result) => Found { ok: true, result }.serialize(serializer),
            Self::Err(error) => Failed { ok: false, error }.serialize(serializer),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn guarded_outcomes_read_as_the_page_expects() {
        let info = GuardedActionInfo {
            id: "0123456789abcdef".into(),
            kind: "ban".into(),
            summary: "ban Ada: griefing".into(),
            state: GuardedState::Pending,
            mode: GuardedMode::Delay,
            initiated_by: "58ef5419".into(),
            needs_role: "moderator".into(),
            created_at: "2026-10-27T12:00:00Z".into(),
            release_at: Some("2026-10-27T12:10:00Z".into()),
            expires_at: "2026-10-29T12:10:00Z".into(),
            approved_by: None,
            canceled_by: None,
            ended_at: None,
            result: None,
        };
        let listed = serde_json::to_value(GuardedList::Ok(vec![info.clone()])).unwrap();
        assert_eq!(listed["ok"], true);
        assert_eq!(listed["actions"][0]["initiatedBy"], "58ef5419");
        assert_eq!(listed["actions"][0]["state"], "pending");
        assert_eq!(listed["actions"][0]["releaseAt"], "2026-10-27T12:10:00Z");
        let decided = serde_json::to_value(GuardedOutcome::Ok(Box::new(info))).unwrap();
        assert_eq!(decided["action"]["id"], "0123456789abcdef");
        assert_eq!(
            serde_json::to_value(GuardedOutcome::Err(LauncherError::new(ErrorCode::GuardedNotFound))).unwrap(),
            json!({ "ok": false, "error": { "code": "guarded_not_found" } })
        );
        assert_eq!(
            serde_json::to_value(GuardedList::Err(LauncherError::new(ErrorCode::GuardedNotWaiting))).unwrap(),
            json!({ "ok": false, "error": { "code": "guarded_not_waiting" } })
        );
    }

    #[test]
    fn install_outcomes_read_as_the_page_expects() {
        let info = InstallInfo {
            version: "1.14.7".into(),
            state: InstallState::Downloading,
            folder: Some("D:/Games/Dauntless/1.14.7".into()),
            progress: None,
            error: None,
        };
        assert_eq!(
            serde_json::to_value(InstallOutcome::Ok(info)).unwrap(),
            json!({ "ok": true, "install": {
                "version": "1.14.7", "state": "downloading", "folder": "D:/Games/Dauntless/1.14.7",
                "progress": null, "error": null,
            } })
        );
        assert_eq!(
            serde_json::to_value(InstallOutcome::Err(LauncherError::new(ErrorCode::EpicNotOwned))).unwrap(),
            json!({ "ok": false, "error": { "code": "epic_not_owned" } })
        );
        assert_eq!(serde_json::to_value(TaskKind::Install).unwrap(), json!("install"));
    }

    #[test]
    fn results_read_as_the_page_expects() {
        assert_eq!(serde_json::to_value(ActionResult::Ok).unwrap(), json!({ "ok": true }));
        assert_eq!(
            serde_json::to_value(ActionResult::err(ErrorCode::KeyInvalid)).unwrap(),
            json!({ "ok": false, "error": { "code": "key_invalid" } })
        );
        let check = InviteCheck::Ok {
            certificate_changed: true,
            previous_fingerprint: None,
            moved_from: Some("100.64.0.1:61000".into()),
        };
        assert_eq!(
            serde_json::to_value(check).unwrap(),
            json!({ "ok": true, "certificateChanged": true, "previousFingerprint": null, "movedFrom": "100.64.0.1:61000" })
        );
        let registered = RegisterOutcome::Ok {
            username: "Slayer".into(),
        };
        assert_eq!(
            serde_json::to_value(registered).unwrap(),
            json!({ "ok": true, "username": "Slayer" })
        );
        let pinged = RegionPing::Ok {
            regions: vec![
                RegionRtt {
                    region: "uk".into(),
                    rtt_ms: Some(18.4),
                    samples: 5,
                    problem: None,
                },
                RegionRtt {
                    region: "oregon".into(),
                    rtt_ms: None,
                    samples: 0,
                    problem: Some(RegionProblem::CertMismatch),
                },
            ],
            reported: true,
        };
        assert_eq!(
            serde_json::to_value(pinged).unwrap(),
            json!({ "ok": true, "regions": [
                { "region": "uk", "rttMs": 18.4, "samples": 5, "problem": null },
                { "region": "oregon", "rttMs": null, "samples": 0, "problem": "cert_mismatch" },
            ], "reported": true })
        );
        assert_eq!(
            serde_json::to_value(RegionPing::Err(LauncherError::new(ErrorCode::ServerUnreachable))).unwrap(),
            json!({ "ok": false, "error": { "code": "server_unreachable" } })
        );
        let listed = GameVersionList {
            versions: vec![
                GameVersionEntry {
                    version: "1.14.7".into(),
                    display_name: "Rustic Isles 1.14.7".into(),
                    origin: GameVersionOrigin::Launcher,
                    install: GameVersionInstall::Epic,
                    epic: Some(EpicVersionInfo {
                        build_version: "1.14.7.647472".into(),
                        manifest: EpicManifestState::Ready,
                        download_bytes: Some(15_985_792_608),
                        install_bytes: Some(16_521_629_423),
                    }),
                },
                GameVersionEntry {
                    version: "1.4.4".into(),
                    display_name: "Dauntless Revived".into(),
                    origin: GameVersionOrigin::Server,
                    install: GameVersionInstall::Locate,
                    epic: None,
                },
            ],
        };
        assert_eq!(
            serde_json::to_value(listed).unwrap(),
            json!({ "ok": true, "versions": [
                { "version": "1.14.7", "displayName": "Rustic Isles 1.14.7", "origin": "launcher", "install": "epic",
                  "epic": { "buildVersion": "1.14.7.647472", "manifest": "ready", "downloadBytes": 15_985_792_608_u64,
                            "installBytes": 16_521_629_423_u64 } },
                { "version": "1.4.4", "displayName": "Dauntless Revived", "origin": "server", "install": "locate",
                  "epic": null },
            ] })
        );
        let epic = EpicView {
            signed_in: true,
            display_name: Some("Slayer".into()),
            signing_in: false,
        };
        assert_eq!(
            serde_json::to_value(epic).unwrap(),
            json!({ "signedIn": true, "displayName": "Slayer", "signingIn": false })
        );
        assert_eq!(
            serde_json::to_value(ErrorCode::EpicCodeInvalid).unwrap(),
            json!("epic_code_invalid")
        );
        let error = LauncherError::with_detail(ErrorCode::HostFailed, "setup_form");
        assert_eq!(
            serde_json::to_value(RegisterOutcome::Err(error)).unwrap(),
            json!({ "ok": false, "error": { "code": "host_failed", "detail": "setup_form" } })
        );
    }

    #[test]
    fn module_outcomes_read_as_the_page_expects() {
        let error = LauncherError::with_detail(ErrorCode::ModuleBadState, "loaded");
        assert_eq!(
            serde_json::to_value(ModuleOutcome::Err(error.clone())).unwrap(),
            json!({ "ok": false, "error": { "code": "module_bad_state", "detail": "loaded" } })
        );
        assert_eq!(
            serde_json::to_value(ModuleListOutcome::Ok(Vec::new())).unwrap(),
            json!({ "ok": true, "modules": [] })
        );
        assert_eq!(
            serde_json::to_value(ModuleCommandOutcome::Ok {
                text: "hi".into(),
                data: Some("{\"a\":1}".into()),
            })
            .unwrap(),
            json!({ "ok": true, "text": "hi", "data": "{\"a\":1}" })
        );
        assert_eq!(
            serde_json::to_value(ModuleCommandOutcome::Ok {
                text: String::new(),
                data: None,
            })
            .unwrap(),
            json!({ "ok": true, "text": "", "data": null })
        );
        assert_eq!(
            serde_json::to_value([
                ModuleSettingValue::Flag(true),
                ModuleSettingValue::Number(2.5),
                ModuleSettingValue::Text("x".into())
            ])
            .unwrap(),
            json!([true, 2.5, "x"])
        );
        assert_eq!(
            serde_json::to_value(ModuleRunState::NotLoaded).unwrap(),
            json!("not_loaded")
        );
        assert_eq!(
            serde_json::to_value(ModuleOrigin::CompiledIn).unwrap(),
            json!("compiled_in")
        );
    }

    #[test]
    fn names_match_the_typescript_ones() {
        assert_eq!(
            serde_json::to_value(ExternalTarget::Dumper7Zip).unwrap(),
            json!("dumper7_zip")
        );
        assert_eq!(serde_json::to_value(ExternalTarget::Dumper7).unwrap(), json!("dumper7"));
        assert_eq!(
            serde_json::to_value(ExternalTarget::ProjectDocsSdk).unwrap(),
            json!("project_docs_sdk")
        );
        assert_eq!(serde_json::to_value(GameLanguage::PtBr).unwrap(), json!("pt-BR"));
        assert_eq!(
            serde_json::to_value(ErrorCode::RuntimeDifferentOnServer).unwrap(),
            json!("runtime_different_on_server")
        );
        assert_eq!(
            serde_json::to_value(NoticeCode::RuntimeInstalledRestart).unwrap(),
            json!("runtime_installed_restart")
        );
        assert_eq!(
            serde_json::to_value(HostFolder::GamedataPacks).unwrap(),
            json!("gamedata_packs")
        );
        assert_eq!(
            serde_json::to_value(HostUpdateState::UpToDate).unwrap(),
            json!("up_to_date")
        );
        let invite = HostInvite {
            link: "x".into(),
            for_name: None,
        };
        assert_eq!(
            serde_json::to_value(invite).unwrap(),
            json!({ "link": "x", "for": null })
        );
    }

    #[test]
    fn a_settings_patch_is_checked_whole() {
        let patch: SettingsPatch =
            serde_json::from_value(json!({ "runtimeProfile": "development", "graphics": -1, "hostTools": true }))
                .unwrap();
        assert_eq!(patch.runtime_profile, Some(RuntimeProfile::Development));
        assert_eq!(patch.graphics, Some(GraphicsPreset::GAME_MENU));
        assert_eq!(patch.host_tools, Some(true));
        for bad in [
            json!({ "graphics": 5 }),
            json!({ "unexpected": true }),
            json!({ "huntRegion": "us" }),
            json!({ "exposure": "basic" }),
            json!({ "language": "xx" }),
            json!({ "windowed": "yes" }),
            json!({ "runtimeProfile": "debug" }),
            json!([]),
        ] {
            assert!(serde_json::from_value::<SettingsPatch>(bad.clone()).is_err(), "{bad}");
        }
    }
}
