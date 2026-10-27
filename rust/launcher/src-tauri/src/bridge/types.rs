//! The values that cross the bridge between the page and the backend: Reforged's `src/shared/types.ts`
//! (`legacy/ReforgedLauncher`), field for field. The 1.4.4 parts (`RevivedInfo`, `Snapshot.revived`, the
//! 1.4.4 error and notice codes) are in every build, so the page and the generated types are the same
//! with and without the `legacy` feature; only a build with it fills them. The TypeScript declarations in
//! `ui/packages/bridge/src/generated/types.ts` are generated from these (`generate.rs`), so the page and the
//! backend cannot drift apart.

/// The game edition the launcher plays (`reborn` 1.14.7, `legacy` 1.4.4), the `gameEdition` setting
pub use protocol::game::Game;
use protocol::invite::ServerMode;
use protocol::status::ServerStatus;
use serde::{Deserialize, Serialize, Serializer};

/// The launcher's languages, made from the page's strings by build.rs (L14; RA3 makes them `en` and
/// `pt-BR`).
pub use crate::core::i18n::Language;

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

/// The game runtime's log profile (`MYSTICPARADOX_RUNTIME_PROFILE`). "development" also honours the
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

/// The server's game runtime (the Mystic Paradox DLL and its loader, built by the host).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum RuntimeState {
    /// Installed next to the game and matching the server's list
    Ok,
    /// Not installed yet
    Missing,
    /// Installed, but the server now offers another build
    Outdated,
    /// Offered, but not accepted on this PC yet (the player is asked first)
    Untrusted,
    /// The server offers no runtime: its host has not built one yet
    Unavailable,
    /// A local runtime folder is used instead (Settings > Advanced)
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    AccountBanned,
    InviteInvalidFormat,
    InviteInvalid,
    UsernameInvalid,
    UsernameTaken,
    RegistrationClosed,
    BadRequest,
    ServerUnreachable,
    ServerError,
    KeyInvalid,
    KeyRejected,
    KeyStorageUnavailable,
    FolderInvalid,
    /// No Dauntless game in the layouts looked at
    GameFolderNotFound,
    /// The executable is not the 1.14.7 build
    GameWrongVersion,
    /// The server offers no runtime yet
    RuntimeUnavailable,
    /// The player has not accepted this server's runtime
    RuntimeUntrusted,
    /// Downloading or installing the runtime failed
    RuntimeFailed,
    /// A file did not match the server's own list
    RuntimeDifferentOnServer,
    /// A runtime file is held by a program that would not let it be replaced
    RuntimeInUse,
    LocalRuntimeInvalid,
    DownloadFailed,
    DiskSpace,
    AlreadyRunning,
    LaunchFailed,
    /// Linux: no Proton or Wine to start the game with (DR's; RA10 decides its text)
    CompatRuntimeMissing,
    ConfigFailed,
    Busy,
    Cancelled,
    CertMismatch,
    /// A public invite whose certificate differs from the one this PC's key belongs to
    CertChanged,
    RateLimited,
    RelayPortBusy,
    RelayFailed,
    /// PLAY refused: a `mystic_loader.ini` or a Host tools DLL is beside the runtime loader (RD9); the
    /// detail lists the files
    LoaderLeftovers,
    /// A Host tools action failed (detail says which)
    HostFailed,
    // Dauntless 1.4.4 (Dauntless Revived servers, `legacy` feature):
    /// The server's list of game files is not the pinned 1.4.4 one
    ManifestMismatch,
    /// The server offers no game download
    NoContentServer,
    FileMissingOnServer,
    FileDifferentOnServer,
    /// The pinned Undaunted DLLs could not be fetched or installed
    DllFailed,
    /// A 1.4.4 file is missing or not the pinned one: Check and repair
    GameFilesInvalid,
    /// The Visual C++ or DirectX runtime the game needs (detail: the files)
    WindowsRuntimeMissing,
    /// The folder chosen for 1.4.4 holds another Dauntless: never written into
    FolderOtherGame,
    /// The folder chosen for 1.4.4 holds other files
    FolderNotEmpty,
    /// The server plays the other edition (detail: `reborn` or `legacy`): the page offers to switch
    EditionOther,
    // Epic (DD22):
    /// No Epic account is signed in, or its session has expired: sign in again
    EpicSignedOut,
    /// Epic's library could not be read
    EpicLibraryFailed,
    /// The signed-in Epic account does not own Dauntless: nothing is downloaded (DE3)
    EpicNotOwned,
    /// A command registered ahead of the row that fills it (DF8; detail: the command's name)
    NotImplemented,
    /// A command this build does not serve, such as a server GUI one in the player launcher (detail: the
    /// command's name)
    NotInThisBuild,
    // Epic sign-in (DE2):
    /// Neither an authorization code nor Epic's page with one
    EpicCodeInvalid,
    /// Epic refused the code (used or expired) or the stored session (signed out elsewhere)
    EpicRejected,
    EpicUnreachable,
    /// Epic's answer could not be read, or the sign-in window could not open (detail: `window`)
    EpicFailed,
    Unknown,
}

/// Short messages that are not errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum NoticeCode {
    KeepOpen,
    BackupSaved,
    RuntimeInstalled,
    RuntimeInstalledRestart,
    GameVerified,
    SdkImported,
    HostDone,
    InstallDone,
    RepairDone,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LauncherError {
    pub code: ErrorCode,
    /// Extra detail for the message, e.g. a file path or a byte count. Never a key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub detail: Option<String>,
}

impl LauncherError {
    pub fn new(code: ErrorCode) -> Self {
        Self { code, detail: None }
    }

    pub fn with_detail(code: ErrorCode, detail: &str) -> Self {
        Self {
            code,
            detail: Some(detail.to_owned()),
        }
    }
}

/// `{ ok: true } | { ok: false, error }`, as the page reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionResult {
    Ok,
    Err(LauncherError),
}

impl ActionResult {
    pub fn err(code: ErrorCode) -> Self {
        Self::Err(LauncherError::new(code))
    }
}

#[derive(Serialize)]
struct Failed<'a> {
    ok: bool,
    error: &'a LauncherError,
}

impl Serialize for ActionResult {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Done {
            ok: bool,
        }
        match self {
            Self::Ok => Done { ok: true }.serialize(serializer),
            Self::Err(error) => Failed { ok: false, error }.serialize(serializer),
        }
    }
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
pub struct RuntimeFileInfo {
    pub name: String,
    pub size: u64,
    pub sha256: String,
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
}

/// What `setSettings` may change: any of the settings, each checked; an unknown field or a wrong value
/// refuses the whole patch (`ipc-validate.ts`).
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

// ------------------------------------------------------------------ Host tools

/// What the Host page shows comes from files on this PC; nothing here is secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum GameDataState {
    Real,
    Synthetic,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct HostUpstream {
    /// .workspace\Mystic-Paradox holds the pinned source
    pub present: bool,
    /// And the Reforged patches are applied
    pub patched: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostSdk {
    /// ParadoxRuntime\SDK.hpp
    pub present: bool,
    /// ParadoxRuntime\SDK\*_functions.cpp
    pub function_files: u32,
}

/// A Dumper-7 CppSDK output found on this PC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct HostDump {
    pub dir: String,
    pub name: String,
    pub modified: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct HostBuilds {
    /// ParadoxRuntime\x64\Release\MysticParadox.dll
    pub runtime: bool,
    /// tools\RuntimeLoader\target\release\winmm.dll
    pub loader: bool,
    /// tools\CatalogExporter\x64\Release\CatalogExporter.dll
    pub exporter: bool,
    /// Both runtime files in the content server's runtime folder
    pub published: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostToolchain {
    /// e.g. "Visual Studio 2022 (17.14)"
    pub visual_studio: Option<String>,
    pub msbuild: bool,
    pub cargo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct HostGameData {
    pub backend: GameDataState,
    pub director: GameDataState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostExports {
    /// Items_Analysis in the workspace
    pub present: bool,
    /// And beside the game (CatalogExporter's output)
    pub in_game: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum HostServerMode {
    Private,
    Public,
}

/// The server's .workspace\config\host.json (no secrets there: they are in secrets.env).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct HostServerConfig {
    pub configured: bool,
    pub name: Option<String>,
    pub mode: Option<HostServerMode>,
    pub host: Option<String>,
    /// The test XP and reward multipliers (private servers only), null when off
    pub multipliers: Option<HostMultipliers>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostKit {
    /// A DaintyLass checkout
    pub workspace: Option<String>,
    pub workspace_ok: bool,
    pub upstream: HostUpstream,
    pub sdk: HostSdk,
    pub dumps: Vec<HostDump>,
    /// The Dumper-7 DLL the host picked
    pub dumper_dll: Option<String>,
    pub builds: HostBuilds,
    pub toolchain: HostToolchain,
    pub game_data: HostGameData,
    pub exports: HostExports,
    pub server_config: HostServerConfig,
    pub requirements: Vec<Requirement>,
    pub job: Option<HostJob>,
    /// What the last "Update" found, since the launcher started
    pub update: Option<HostUpdateView>,
    /// Where the project folder is downloaded to: beside the game's folder
    pub project_target: Option<String>,
}

/// XP and reward multipliers for a private test server: 1 to `MAX_MULTIPLIER` each.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct HostMultipliers {
    pub xp: f64,
    pub rewards: f64,
}

pub const MAX_MULTIPLIER: f64 = 100.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum HostUpdateState {
    UpToDate,
    Updated,
}

/// The project compared with GitHub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostUpdateView {
    pub state: HostUpdateState,
    /// The short commit this PC is at now
    pub head: String,
    /// Files the update changed
    pub files: u32,
    /// The game runtime changed: build MysticParadox.dll again and publish it
    pub runtime_changed: bool,
}

/// What the Requirements tab lists, each one detected on this PC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum RequirementId {
    Dauntless,
    Git,
    Node,
    Mongodb,
    Tailscale,
    VisualStudio,
    Rust,
    Dumper7,
    VcRedist,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum RequirementState {
    Ok,
    Missing,
    Problem,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Requirement {
    pub id: RequirementId,
    pub state: RequirementState,
    /// The version or place found; never a secret
    pub detail: Option<String>,
    /// What is wrong when state is "problem": node "too_old"; mongodb "service_running"; visual_studio
    /// "no_cpp"
    pub problem: Option<String>,
}

/// The server the Host page starts and stops. `external`: the server answers, but was started elsewhere
/// (a terminal): the launcher cannot stop it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum HostServerState {
    Stopped,
    Starting,
    Running,
    Stopping,
    External,
    Failed,
}

/// The invite made last: a secret, shown blurred in streamer mode and never logged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct HostInvite {
    pub link: String,
    #[serde(rename = "for")]
    pub for_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct HostServerView {
    pub state: HostServerState,
    /// The error line when it failed
    pub message: Option<String>,
    /// The newest output of the server and of the host kit commands started here
    pub lines: Vec<String>,
    pub invite: Option<HostInvite>,
    /// The server runs inside this launcher (the server GUI, RD26), not as `reforged-host start`
    pub embedded: bool,
    /// Its services while it runs inside the launcher; empty otherwise
    pub components: Vec<HostServerComponent>,
}

/// One service of the server inside the launcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct HostServerComponent {
    /// One of `spec::HOST_SERVER_COMPONENTS`
    pub name: String,
    pub phase: HostComponentPhase,
    pub port: Option<u16>,
    /// What it last said about itself (an error, for a failed one)
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum HostComponentPhase {
    Stopped,
    Starting,
    Ready,
    Stopping,
    Failed,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum HostJobKind {
    Inject,
    BuildRuntime,
    BuildLoader,
    BuildExporter,
    ImportSdk,
    CopyExports,
    SyntheticData,
    ImportGamedata,
    PublishRuntime,
    Setup,
    GenerateGamedata,
    ExportGamedata,
    CreateInvite,
    Update,
    Multipliers,
    Upstream,
    DownloadProject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum HostJobState {
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostJob {
    pub kind: HostJobKind,
    pub state: HostJobState,
    /// The last line of its log, never a secret
    pub message: Option<String>,
    pub log_file: Option<String>,
}

/// The links the page can ask the backend to open. The page names one of these; it never passes a URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ExternalTarget {
    TailscaleDownload,
    TailscaleShare,
    VcRedist,
    ServerSource,
    ProjectSource,
    ProjectLicense,
    ProjectIssues,
    ProjectDocsPlayers,
    ProjectDocsHosting,
    ProjectDocsSdk,
    InstallingGuide,
    Legendary,
    Dumper7,
    VisualStudio,
    Rust,
    Nodejs,
    Mongodb,
    Git,
    VisualStudioInstaller,
    RustInstaller,
    #[serde(rename = "dumper7_zip")]
    Dumper7Zip,
    RevivedSource,
    MysticSource,
    UndauntedSource,
}

/// Host page actions, by name (the page never passes a path or a command).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum HostAction {
    /// Start the game without the runtime, with Dumper-7 on the runtime loader's list (RD9)
    InjectDumper,
    /// Copy the newest Dumper-7 CppSDK into the runtime source
    ImportSdk,
    BuildRuntime,
    BuildLoader,
    BuildExporter,
    /// Start the game as PLAY does, with CatalogExporter on the runtime loader's list (RD9)
    InjectExporter,
    /// Copy the exporter's Items_Analysis into the workspace
    CopyExports,
    /// Copy the example game data (startup tests only)
    SyntheticData,
    /// Copy the built runtime files to the content server
    PublishRuntime,
    GenerateGamedata,
    ExportGamedata,
}

/// The folders the Host page may open, by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum HostFolder {
    Workspace,
    RuntimeSource,
    Dumps,
    GameData,
    RuntimePublish,
    HostLogs,
    GamedataPacks,
}

/// The Host page's server setup form. The game folder is the launcher's own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostSetupRequest {
    pub name: String,
    pub mode: HostServerMode,
    /// Public mode: the server's public IPv4 or DNS name
    pub public_host: Option<String>,
}

/// What `realm_host` hosts (DU5)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
#[cfg_attr(not(test), allow(dead_code, reason = "RF7's realm_host reads it"))]
pub enum RealmKind {
    Hub,
    Hunt,
}

/// Why a node has no round trip (DN7)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum RegionProblem {
    Timeout,
    Unreachable,
    /// The node's certificate is not the one its home listed: nothing was sent to it
    CertMismatch,
    BadAnswer,
}

/// One region's round trip, as `region_ping` orders them: nearest first, the silent ones last (DN7).
/// The region is an id (`oregon`); its name is the page's to show.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RegionRtt {
    pub region: String,
    /// The median round trip in milliseconds, to a tenth; null when the node never answered
    pub rtt_ms: Option<f64>,
    /// How many answers the median was taken over
    pub samples: u32,
    pub problem: Option<RegionProblem>,
}

/// `region_ping`: `{ ok: true, regions, reported } | { ok: false, error }`. `reported`: the home took
/// the order. A server outside a network answers no regions, and nothing is measured.
#[derive(Debug, Clone, PartialEq)]
pub enum RegionPing {
    Ok { regions: Vec<RegionRtt>, reported: bool },
    Err(LauncherError),
}

impl Serialize for RegionPing {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Pinged<'a> {
            ok: bool,
            regions: &'a [RegionRtt],
            reported: bool,
        }
        match self {
            Self::Ok { regions, reported } => Pinged {
                ok: true,
                regions,
                reported: *reported,
            }
            .serialize(serializer),
            Self::Err(error) => Failed { ok: false, error }.serialize(serializer),
        }
    }
}

/// How a version gets onto this PC (DE4): from the player's Epic library, or from a folder the player
/// already has (1.4.4, which has no archived manifest: DD22)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum GameVersionInstall {
    Epic,
    Locate,
}

/// Whose profile listed a version: the launcher's compiled-in one, or a joined server's content (DE4)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum GameVersionOrigin {
    Launcher,
    Server,
}

/// An Epic build's manifest, as the list read it: `unreachable` when no source answered, `invalid` when
/// none answered with the pinned file
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum EpicManifestState {
    Ready,
    Unreachable,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct EpicVersionInfo {
    /// Epic's build version (`1.14.7.647472`)
    pub build_version: String,
    pub manifest: EpicManifestState,
    /// Bytes, from the manifest once it is `ready`
    pub download_bytes: Option<u64>,
    pub install_bytes: Option<u64>,
}

/// One version on `game_version_list`, newest first. The display name is the module's (DD4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct GameVersionEntry {
    pub version: String,
    pub display_name: String,
    pub origin: GameVersionOrigin,
    pub install: GameVersionInstall,
    /// Null for `locate`
    pub epic: Option<EpicVersionInfo>,
}

/// `game_version_list`: `{ ok: true, versions }`. The backend always answers it; the page's
/// `{ ok: false, error }` arm is `act()`'s own, for a call that fails on the way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameVersionList {
    pub versions: Vec<GameVersionEntry>,
}

impl Serialize for GameVersionList {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Listed<'a> {
            ok: bool,
            versions: &'a [GameVersionEntry],
        }
        Listed {
            ok: true,
            versions: &self.versions,
        }
        .serialize(serializer)
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
