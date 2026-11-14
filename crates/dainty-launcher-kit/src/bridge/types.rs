// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The values that cross the bridge to the page, as JSON. Each answer is `{ok: true, ...}` or `{ok: false, error}`;
//! the page shows an [`ErrorCode`] or [`NoticeCode`] as its catalog sentence.

use serde::{Deserialize, Serialize, Serializer};

/// Serde's derived struct `Deserialize` also takes a sequence (`[]` fills the fields in order), but the bridge
/// refuses anything but an object for an object argument (§2.3.4). These impls read a map first, then decode it
/// with `$fields`: the type itself under `#[serde(remote = "Self")]`, or a field mirror under
/// `#[serde(remote = "Type")]` for types ts-rs reads (it cannot parse `remote`).
macro_rules! object_only {
    ($ty:ty, $fields:ty) => {
        impl<'de> serde::Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let map = <serde_json::Map<String, serde_json::Value>>::deserialize(deserializer)?;
                <$fields>::deserialize(serde_json::Value::Object(map)).map_err(serde::de::Error::custom)
            }
        }
    };
    ($ty:ty) => {
        object_only!($ty, $ty);

        impl serde::Serialize for $ty {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                <$ty>::serialize(self, serializer)
            }
        }
    };
}

mod manager;
mod player;

pub use manager::*;
pub use player::*;

/// Whether the server's game runtime is installed and current on this PC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum RuntimeState {
    /// Installed and matching what the server offers.
    Ok,
    /// Not installed.
    Missing,
    /// Installed, but the server offers a newer build.
    Outdated,
    /// Offered, but the player has not trusted it yet.
    Untrusted,
    /// The server offers no runtime.
    Unavailable,
    /// A local runtime folder is in use instead of the server's.
    Local,
}

/// One file of a runtime, as listed in its manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct RuntimeFileInfo {
    /// File name inside the runtime folder.
    pub name: String,
    /// Size in bytes.
    pub size: u64,
    /// Lowercase hex SHA-256 of the contents.
    pub sha256: String,
}

/// Why a bridge command failed. The page shows each code as its catalog sentence (`errors.<code>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The account is banned on this server.
    AccountBanned,
    /// The pasted text is not an invite link.
    InviteInvalidFormat,
    /// The invite is well formed but no longer accepted.
    InviteInvalid,
    /// The name breaks the username rules.
    UsernameInvalid,
    /// The name is already registered on this server.
    UsernameTaken,
    /// The server takes no new players.
    RegistrationClosed,
    /// The server refused the request as malformed.
    BadRequest,
    /// The server did not answer.
    ServerUnreachable,
    /// The server answered with an error of its own.
    ServerError,
    /// The text is not an account key.
    KeyInvalid,
    /// The server does not know the account key.
    KeyRejected,
    /// The OS key store could not hold the account key.
    KeyStorageUnavailable,
    /// The chosen folder cannot be used.
    FolderInvalid,
    /// No Dauntless install was found in the folder.
    GameFolderNotFound,
    /// The folder holds a Dauntless build other than 1.14.7.
    GameWrongVersion,
    /// The server offers no game runtime.
    RuntimeUnavailable,
    /// The runtime was not trusted before installing it.
    RuntimeUntrusted,
    /// Installing the runtime failed.
    RuntimeFailed,
    /// The server's runtime changed since it was installed.
    RuntimeDifferentOnServer,
    /// The runtime files are locked by a running game.
    RuntimeInUse,
    /// The local runtime folder does not hold a runtime.
    LocalRuntimeInvalid,
    /// A download failed.
    DownloadFailed,
    /// Not enough disk space; `detail` holds the amounts.
    DiskSpace,
    /// The game is already running.
    AlreadyRunning,
    /// The game process did not start.
    LaunchFailed,
    /// Proton or Wine was not found (Linux).
    CompatRuntimeMissing,
    /// The game's settings could not be written.
    ConfigFailed,
    /// Another task is still running.
    Busy,
    /// The player cancelled.
    Cancelled,
    /// The server's certificate does not match the invite's fingerprint.
    CertMismatch,
    /// The server's certificate changed since the last join.
    CertChanged,
    /// The server is rate-limiting this client.
    RateLimited,
    /// The local relay port is taken by another program.
    RelayPortBusy,
    /// The local relay did not start.
    RelayFailed,
    /// A leftover loader ini or unknown DLL sits next to the loader (RD9).
    LoaderLeftovers,
    /// A host tool failed; its log has the details.
    HostFailed,
    /// The game files do not match the server's manifest.
    ManifestMismatch,
    /// The server has no content server.
    NoContentServer,
    /// A file the manifest lists is missing on the server.
    FileMissingOnServer,
    /// A file on the server differs from its manifest entry.
    FileDifferentOnServer,
    /// A game library could not be installed.
    DllFailed,
    /// Game files are damaged.
    GameFilesInvalid,
    /// The Visual C++ runtime is missing.
    WindowsRuntimeMissing,
    /// The folder holds another game.
    FolderOtherGame,
    /// The folder is not empty.
    FolderNotEmpty,
    /// This build does not support that game edition.
    EditionOther,
    /// The player is not signed in to Epic Games.
    EpicSignedOut,
    /// The Epic library could not be read.
    EpicLibraryFailed,
    /// The Epic account does not own Dauntless.
    EpicNotOwned,
    /// The feature is not built yet.
    NotImplemented,
    /// This build of the app leaves the feature out.
    NotInThisBuild,
    /// The pasted Epic authorization code is not valid.
    EpicCodeInvalid,
    /// Epic Games refused the sign-in.
    EpicRejected,
    /// Epic Games did not answer.
    EpicUnreachable,
    /// Any other Epic failure; `detail` says which.
    EpicFailed,
    /// The installed game version could not be identified.
    GameVersionUnknown,
    /// The requested game version cannot be downloaded.
    GameVersionUnavailable,
    /// Epic Games gave no file list for the version.
    EpicManifestUnavailable,
    /// The guarded action id is unknown or expired.
    GuardedNotFound,
    /// The guarded action is not waiting for confirmation.
    GuardedNotWaiting,
    /// The command needs the embedded server running.
    ServerStopped,
    /// The server has no module with that id.
    ModuleUnknown,
    /// The module's state does not allow the request.
    ModuleBadState,
    /// The module refused the command input.
    ModuleInputInvalid,
    /// The module refused the settings.
    ModuleSettingsInvalid,
    /// The module reported a failure; `detail` holds its words.
    ModuleFailed,
    /// The history filter is not valid.
    HistoryBadFilter,
    /// The change history is not available.
    HistoryUnavailable,
    /// The history changed since the preview was made.
    HistoryPlanChanged,
    /// Updating the history failed; `detail` says why.
    HistoryFailed,
    /// The server has no realms.
    RealmUnavailable,
    /// No realm has that id.
    RealmNotFound,
    /// The realm reported a failure; `detail` says why.
    RealmFailed,
    /// The Proton inject helper (`dainty-inject.exe`) is missing from the launcher's resources.
    InjectHelperMissing,
    /// Anything not covered above; the logs have the details.
    Unknown,
}

/// A notice the page shows after an action succeeds, by catalog key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum NoticeCode {
    /// The launcher must stay open while the game runs.
    KeepOpen,
    /// A backup was written.
    BackupSaved,
    /// The runtime was installed.
    RuntimeInstalled,
    /// The runtime was installed and takes effect after the game restarts.
    RuntimeInstalledRestart,
    /// The game files passed verification.
    GameVerified,
    /// The SDK was imported into the runtime source.
    SdkImported,
    /// A host job finished.
    HostDone,
    /// The game install finished.
    InstallDone,
    /// The game repair finished.
    RepairDone,
}

/// A failed command's answer: the code and optional words for the `{detail}` placeholder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct LauncherError {
    /// Why the command failed.
    pub code: ErrorCode,
    /// Text the catalog sentence shows in place of `{detail}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "ts"), ts(optional))]
    pub detail: Option<String>,
}

impl LauncherError {
    /// A failure with no detail.
    pub fn new(code: ErrorCode) -> Self {
        Self { code, detail: None }
    }

    /// A failure whose sentence shows `detail`.
    pub fn with_detail(code: ErrorCode, detail: &str) -> Self {
        Self {
            code,
            detail: Some(detail.to_owned()),
        }
    }
}

/// The answer of a command with nothing to return: `{ok: true}` or `{ok: false, error}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionResult {
    /// The command succeeded.
    Ok,
    /// The command failed.
    Err(LauncherError),
}

impl ActionResult {
    /// A failure with no detail.
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

/// What a workspace's game-data folder holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum GameDataState {
    /// Every file is present and differs from its example.
    Real,
    /// At least one file is a copy of its `.example.json`.
    Synthetic,
    /// A file is missing or empty.
    Missing,
}

/// The runtime's upstream source in the workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct HostUpstream {
    /// The runtime source (`dllmain.cpp`) is present.
    pub present: bool,
    /// The source carries the runtime patches.
    pub patched: bool,
}

/// The Dumper-7 SDK imported into the runtime source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostSdk {
    /// `SDK.hpp` is present.
    pub present: bool,
    /// Number of `*_functions.cpp` files in the SDK folder.
    pub function_files: u32,
}

/// One Dumper-7 dump found under `C:\Dumper-7`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct HostDump {
    /// Path of the dump's `CppSDK` folder.
    pub dir: String,
    /// The dump's folder name.
    pub name: String,
    /// When `SDK.hpp` was written, RFC 3339 in UTC.
    pub modified: String,
}

/// Which workspace build outputs exist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct HostBuilds {
    /// The runtime DLL is built.
    pub runtime: bool,
    /// The runtime loader DLL is built.
    pub loader: bool,
    /// The CatalogExporter DLL is built.
    pub exporter: bool,
    /// The runtime files are in the publish folder.
    pub published: bool,
}

/// The build toolchain found on the host PC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostToolchain {
    /// Display name of the Visual Studio install with the C++ workload.
    pub visual_studio: Option<String>,
    /// MSBuild was found.
    pub msbuild: bool,
    /// Cargo was found.
    pub cargo: bool,
}

/// Game-data state of each service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct HostGameData {
    /// The backend's game data.
    pub backend: GameDataState,
    /// The director's game data.
    pub director: GameDataState,
}

/// The CatalogExporter's exported item data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostExports {
    /// The exports folder is present.
    pub present: bool,
    /// The exports came from a running game.
    pub in_game: bool,
}

/// Who may join the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum HostServerMode {
    /// Invite only, over a private network.
    Private,
    /// Reachable at a public address.
    Public,
}

/// Progression multipliers the host set for testing.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct HostMultipliers {
    /// Experience multiplier.
    pub xp: f64,
    /// Reward multiplier.
    pub rewards: f64,
}

/// The highest multiplier a host may set.
pub const MAX_MULTIPLIER: f64 = 100.0;

#[derive(Deserialize)]
#[serde(remote = "HostMultipliers")]
struct HostMultipliersFields {
    xp: f64,
    rewards: f64,
}

object_only!(HostMultipliers, HostMultipliersFields);

/// The server settings read from `settings.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct HostServerConfig {
    /// `settings.toml` exists and parsed.
    pub configured: bool,
    /// Server name players see.
    pub name: Option<String>,
    /// Private or public mode.
    pub mode: Option<HostServerMode>,
    /// Public host name, in public mode.
    pub host: Option<String>,
    /// Test multipliers, when set.
    pub multipliers: Option<HostMultipliers>,
}

/// Result of a workspace update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum HostUpdateState {
    /// Nothing new was fetched.
    UpToDate,
    /// New commits were pulled.
    Updated,
}

/// What a workspace update did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostUpdateView {
    /// Whether anything changed.
    pub state: HostUpdateState,
    /// The commit the workspace is now at.
    pub head: String,
    /// Number of files the update changed.
    pub files: u32,
    /// The update touched the runtime source, so the runtime needs a rebuild.
    pub runtime_changed: bool,
}

/// A program or component the host kit checks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum RequirementId {
    /// The game itself.
    Dauntless,
    /// Git.
    Git,
    /// Node.js.
    Node,
    /// MongoDB.
    Mongodb,
    /// Tailscale, for private mode.
    Tailscale,
    /// Visual Studio with the C++ workload.
    VisualStudio,
    /// The Rust toolchain.
    Rust,
    /// Dumper-7.
    Dumper7,
    /// The Visual C++ redistributable.
    VcRedist,
}

/// How one requirement checked out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum RequirementState {
    /// Found and usable.
    Ok,
    /// Not found.
    Missing,
    /// Found but unusable; `problem` says why.
    Problem,
}

/// One requirement check, shown in the host kit's list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct Requirement {
    /// Which requirement.
    pub id: RequirementId,
    /// The check's result.
    pub state: RequirementState,
    /// What was found, such as a version or path.
    pub detail: Option<String>,
    /// Why a found requirement is unusable.
    pub problem: Option<String>,
}

/// Lifecycle of the embedded or external server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum HostServerState {
    /// Not running.
    Stopped,
    /// Starting up.
    Starting,
    /// Every component is ready.
    Running,
    /// Shutting down.
    Stopping,
    /// A server this app did not start is running.
    External,
    /// Startup or a component failed.
    Failed,
}

/// An invite the server made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct HostInvite {
    /// The invite link to hand to a player.
    pub link: String,
    /// The player name the invite was made for, if any; `for` on the wire.
    #[serde(rename = "for")]
    pub for_name: Option<String>,
}

/// The server panel's snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct HostServerView {
    /// Server lifecycle state.
    pub state: HostServerState,
    /// Words on the last failure or transition.
    pub message: Option<String>,
    /// Recent console lines.
    pub lines: Vec<String>,
    /// The latest invite made.
    pub invite: Option<HostInvite>,
    /// The server runs inside this app.
    pub embedded: bool,
    /// Each service's state.
    pub components: Vec<HostServerComponent>,
}

/// One service of the running server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct HostServerComponent {
    /// Service name.
    pub name: String,
    /// Its lifecycle phase.
    pub phase: HostComponentPhase,
    /// The port it listens on, once known.
    pub port: Option<u16>,
    /// Words on its current phase.
    pub detail: String,
}

/// Lifecycle of one server component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum HostComponentPhase {
    /// Not running.
    Stopped,
    /// Starting up.
    Starting,
    /// Listening and ready.
    Ready,
    /// Shutting down.
    Stopping,
    /// Exited with an error.
    Failed,
    /// Turned off in the server's configuration.
    Disabled,
}

/// A long-running host-kit task; one runs at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum HostJobKind {
    /// Injecting Dumper-7 into the running game.
    Inject,
    /// Building the game runtime.
    BuildRuntime,
    /// Building the runtime loader.
    BuildLoader,
    /// Building the CatalogExporter.
    BuildExporter,
    /// Importing a Dumper-7 SDK.
    ImportSdk,
    /// Copying the CatalogExporter's exports.
    CopyExports,
    /// Writing synthetic game data from the examples.
    SyntheticData,
    /// Importing game data.
    ImportGamedata,
    /// Publishing the built runtime to the content server.
    PublishRuntime,
    /// Running `setup`.
    Setup,
    /// Generating game data.
    GenerateGamedata,
    /// Exporting game data.
    ExportGamedata,
    /// Making an invite.
    CreateInvite,
    /// Updating the workspace.
    Update,
    /// Setting the test multipliers.
    Multipliers,
    /// Fetching the runtime's upstream source.
    Upstream,
    /// Downloading the project source.
    DownloadProject,
    /// Writing a backup.
    Backup,
    /// Migrating a MongoDB database into the store.
    MigrateMongo,
}

/// Progress of a host job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum HostJobState {
    /// Still running.
    Running,
    /// Finished.
    Done,
    /// Failed; the log has the details.
    Failed,
}

/// The current or last host job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostJob {
    /// What the job does.
    pub kind: HostJobKind,
    /// How far it got.
    pub state: HostJobState,
    /// Last line of its output or its failure.
    pub message: Option<String>,
    /// Path of its log file.
    pub log_file: Option<String>,
}

/// The host kit panel's snapshot of a workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostKit {
    /// Workspace folder, if one is chosen.
    pub workspace: Option<String>,
    /// The folder is a workspace (`dainty-host` plus the module sources).
    pub workspace_ok: bool,
    /// Runtime upstream source state.
    pub upstream: HostUpstream,
    /// Imported SDK state.
    pub sdk: HostSdk,
    /// Dumper-7 dumps found, newest first.
    pub dumps: Vec<HostDump>,
    /// Path of the Dumper-7 DLL, once found.
    pub dumper_dll: Option<String>,
    /// Build outputs present.
    pub builds: HostBuilds,
    /// Toolchain found.
    pub toolchain: HostToolchain,
    /// Game-data state.
    pub game_data: HostGameData,
    /// Exported item data state.
    pub exports: HostExports,
    /// Server settings.
    pub server_config: HostServerConfig,
    /// Requirement checks.
    pub requirements: Vec<Requirement>,
    /// The current or last job.
    pub job: Option<HostJob>,
    /// The last workspace update.
    pub update: Option<HostUpdateView>,
    /// Folder the project download goes to.
    pub project_target: Option<String>,
}

/// An external page or download the page may open; the URL is fixed on the Rust side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ExternalTarget {
    /// Tailscale's download page.
    TailscaleDownload,
    /// Tailscale's guide to sharing a machine.
    TailscaleShare,
    /// The Visual C++ redistributable download.
    VcRedist,
    /// The server's source code.
    ServerSource,
    /// This project's source code.
    ProjectSource,
    /// This project's licence.
    ProjectLicense,
    /// This project's issue tracker.
    ProjectIssues,
    /// The players' guide.
    ProjectDocsPlayers,
    /// The hosting guide.
    ProjectDocsHosting,
    /// The SDK guide.
    ProjectDocsSdk,
    /// The installing guide.
    InstallingGuide,
    /// Legendary, the Epic Games client.
    Legendary,
    /// Dumper-7's project page.
    Dumper7,
    /// Visual Studio's download page.
    VisualStudio,
    /// Rust's download page.
    Rust,
    /// Node.js's download page.
    Nodejs,
    /// MongoDB's download page.
    Mongodb,
    /// Git's download page.
    Git,
    /// The Visual Studio installer download.
    VisualStudioInstaller,
    /// The Rust installer download.
    RustInstaller,
    /// The Dumper-7 release archive.
    #[serde(rename = "dumper7_zip")]
    Dumper7Zip,
    /// Dauntless Revived's source code.
    RevivedSource,
    /// Mystic Paradox's source code.
    MysticSource,
    /// Undaunted's source code.
    UndauntedSource,
}

/// A host-kit step the page starts directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum HostAction {
    /// Inject Dumper-7 into the running game.
    InjectDumper,
    /// Import a Dumper-7 SDK.
    ImportSdk,
    /// Build the game runtime.
    BuildRuntime,
    /// Build the runtime loader.
    BuildLoader,
    /// Build the CatalogExporter.
    BuildExporter,
    /// Inject the CatalogExporter into the running game.
    InjectExporter,
    /// Copy the CatalogExporter's exports.
    CopyExports,
    /// Write synthetic game data from the examples.
    SyntheticData,
    /// Publish the built runtime.
    PublishRuntime,
    /// Generate game data.
    GenerateGamedata,
    /// Export game data.
    ExportGamedata,
}

/// A workspace folder the page may open in the file manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum HostFolder {
    /// The workspace root.
    Workspace,
    /// The runtime source.
    RuntimeSource,
    /// The Dumper-7 dumps.
    Dumps,
    /// The game-data folder.
    GameData,
    /// The runtime publish folder.
    RuntimePublish,
    /// The host kit's logs.
    HostLogs,
    /// The game-data packs.
    GamedataPacks,
}

/// The page's `setup` form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostSetupRequest {
    /// Server name players see.
    pub name: String,
    /// Private or public mode.
    pub mode: HostServerMode,
    /// Public host name, required in public mode.
    pub public_host: Option<String>,
}

#[derive(Deserialize)]
#[serde(remote = "HostSetupRequest", rename_all = "camelCase")]
struct HostSetupRequestFields {
    name: String,
    mode: HostServerMode,
    public_host: Option<String>,
}

object_only!(HostSetupRequest, HostSetupRequestFields);

/// Which kind of realm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum RealmKind {
    /// A social hub.
    Hub,
    /// A hunt.
    Hunt,
}

/// Why a region could not be timed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum RegionProblem {
    /// The node did not answer within its deadline.
    Timeout,
    /// The node could not be reached.
    Unreachable,
    /// The node's certificate did not match.
    CertMismatch,
    /// The node answered something other than a ping reply.
    BadAnswer,
}

/// One region's ping result.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RegionRtt {
    /// Region id from the node list.
    pub region: String,
    /// Median round trip in milliseconds, to a tenth; none when no sample came back.
    pub rtt_ms: Option<f64>,
    /// Number of samples that came back.
    pub samples: u32,
    /// Why the region could not be timed.
    pub problem: Option<RegionProblem>,
}

/// The `region_ping` answer.
#[derive(Debug, Clone, PartialEq)]
pub enum RegionPing {
    /// The nodes were timed; `regions` is empty outside a network.
    Ok {
        /// The regions, nearest first, silent ones last.
        regions: Vec<RegionRtt>,
        /// The joined server took the report.
        reported: bool,
    },
    /// The ping failed.
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

/// How a game version is installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum GameVersionInstall {
    /// Downloaded through Epic Games.
    Epic,
    /// Found in a folder the player picks.
    Locate,
}

/// Who listed a game version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum GameVersionOrigin {
    /// Built into the launcher.
    Launcher,
    /// The joined server.
    Server,
}

/// Whether Epic's file list for a version could be fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum EpicManifestState {
    /// Fetched and valid.
    Ready,
    /// Epic did not answer.
    Unreachable,
    /// Epic's answer could not be read.
    Invalid,
}

/// Epic download details of a game version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct EpicVersionInfo {
    /// Epic's build version string.
    pub build_version: String,
    /// State of the file list.
    pub manifest: EpicManifestState,
    /// Bytes to download, from the file list.
    pub download_bytes: Option<u64>,
    /// Bytes on disk once installed, from the file list.
    pub install_bytes: Option<u64>,
}

/// One game version the player may install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct GameVersionEntry {
    /// Game version, such as `1.14.7`.
    pub version: String,
    /// Name shown to the player.
    pub display_name: String,
    /// Who listed it.
    pub origin: GameVersionOrigin,
    /// How it is installed.
    pub install: GameVersionInstall,
    /// Epic download details, for versions installed through Epic.
    pub epic: Option<EpicVersionInfo>,
}

/// The `game_versions` answer: `{ok: true, versions}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameVersionList {
    /// The versions, in display order.
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

/// What a module is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ModuleRole {
    /// Serves a game.
    Game,
    /// Adds to the server without serving a game.
    Plugin,
}

/// Where a module comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ModuleOrigin {
    /// Compiled into the server.
    CompiledIn,
    /// An external process the server launches.
    External,
}

/// Lifecycle of a module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ModuleRunState {
    /// Not loaded.
    NotLoaded,
    /// Loading.
    Starting,
    /// Running.
    Ready,
    /// Failed to load or crashed; `detail` says why.
    Failed,
}

/// A port range a module asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct ModulePortView {
    /// Name the module gave the range.
    pub name: String,
    /// `tcp` or `udp`.
    pub transport: String,
    /// First port of the range.
    pub first: u16,
    /// Last port of the range, inclusive.
    pub last: u16,
}

/// One argument of a module command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct ModuleArgView {
    /// Argument name.
    pub name: String,
    /// The command refuses to run without it.
    pub required: bool,
    /// Help text from the module.
    pub help: String,
}

/// A command a module registered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct ModuleCommandView {
    /// Command name.
    pub name: String,
    /// One-line summary from the module.
    pub summary: String,
    /// Its arguments.
    pub args: Vec<ModuleArgView>,
}

/// A module setting's value; its type is the default's.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(untagged)]
pub enum ModuleSettingValue {
    /// A boolean setting.
    Flag(bool),
    /// A numeric setting.
    Number(f64),
    /// A text setting.
    Text(String),
}

/// One setting a module registered.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct ModuleSettingView {
    /// Key name.
    pub name: String,
    /// Help text from the module.
    pub help: String,
    /// The module's default.
    pub default: ModuleSettingValue,
    /// The value in `settings.toml`, or the default.
    pub value: ModuleSettingValue,
}

/// A section of a module's settings (`[modules.<id>.<section>]`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct ModuleSectionView {
    /// Section name.
    pub name: String,
    /// Its keys.
    pub keys: Vec<ModuleSettingView>,
}

/// A module's game patch set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ModulePatchSetView {
    /// True when the module provides a patch set, false when it accepts one.
    pub provides: bool,
    /// Name of the provided set.
    pub display_name: Option<String>,
}

/// One colour a theme sets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct ModulePaletteEntry {
    /// CSS variable name.
    pub name: String,
    /// Checked colour value.
    pub value: String,
}

/// A theme a plugin provides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ModuleThemeView {
    /// Theme id.
    pub id: String,
    /// Name shown to the player.
    pub display_name: String,
    /// The colours it sets.
    pub palette: Vec<ModulePaletteEntry>,
    /// Layout name, if the theme picks one.
    pub layout: Option<String>,
    /// Logo path, if the theme has one.
    pub logo: Option<String>,
}

/// What a module's manifest says it does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ModuleCapabilitiesView {
    /// It hosts realms.
    pub realms: bool,
    /// It uses the network plugin.
    pub network: bool,
    /// It uses the chat plugin.
    pub chat: bool,
    /// Its patch set, if any.
    pub patch_set: Option<ModulePatchSetView>,
    /// Its theme, if any.
    pub theme: Option<ModuleThemeView>,
}

/// What the page is shown of one module.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ModuleView {
    /// Module id.
    pub id: String,
    /// Name from the manifest.
    pub display_name: Option<String>,
    /// Role from the manifest.
    pub role: Option<ModuleRole>,
    /// Game version from the manifest, for a game module.
    pub game_version: Option<String>,
    /// Where it comes from.
    pub origin: ModuleOrigin,
    /// It loads when the server starts.
    pub autoload: bool,
    /// Its lifecycle state.
    pub state: ModuleRunState,
    /// Words on a failure, bounded.
    pub detail: Option<String>,
    /// Built but not checked against recorded traffic (DD9).
    pub unverified: bool,
    /// Port ranges it holds.
    pub ports: Vec<ModulePortView>,
    /// Manifest capabilities.
    pub capabilities: ModuleCapabilitiesView,
    /// Commands it registered.
    pub commands: Vec<ModuleCommandView>,
    /// Settings it registered, with their values.
    pub settings: Vec<ModuleSectionView>,
}

/// The `module_list` answer.
#[derive(Debug, Clone, PartialEq)]
pub enum ModuleListOutcome {
    /// Every module the server knows.
    Ok(Vec<ModuleView>),
    /// The list could not be read.
    Err(LauncherError),
}

impl Serialize for ModuleListOutcome {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Listed<'a> {
            ok: bool,
            modules: &'a [ModuleView],
        }
        match self {
            Self::Ok(modules) => Listed { ok: true, modules }.serialize(serializer),
            Self::Err(error) => Failed { ok: false, error }.serialize(serializer),
        }
    }
}

/// The answer of a command about one module.
#[derive(Debug, Clone, PartialEq)]
pub enum ModuleOutcome {
    /// The module after the command.
    Ok(Box<ModuleView>),
    /// The command failed.
    Err(LauncherError),
}

impl Serialize for ModuleOutcome {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Found<'a> {
            ok: bool,
            module: &'a ModuleView,
        }
        match self {
            Self::Ok(module) => Found { ok: true, module }.serialize(serializer),
            Self::Err(error) => Failed { ok: false, error }.serialize(serializer),
        }
    }
}

/// The answer of `module_run`.
#[derive(Debug, Clone, PartialEq)]
pub enum ModuleCommandOutcome {
    /// The command ran.
    Ok {
        /// The module's text answer, control characters removed and bounded.
        text: String,
        /// The module's data answer, bounded, when it gave one.
        data: Option<String>,
    },
    /// The command failed.
    Err(LauncherError),
}

impl Serialize for ModuleCommandOutcome {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Ran<'a> {
            ok: bool,
            text: &'a str,
            data: Option<&'a str>,
        }
        match self {
            Self::Ok { text, data } => Ran {
                ok: true,
                text,
                data: data.as_deref(),
            }
            .serialize(serializer),
            Self::Err(error) => Failed { ok: false, error }.serialize(serializer),
        }
    }
}

/// A moderation action the page asks of the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum AdminActionKind {
    /// Look up a player.
    Lookup,
    /// Disconnect a player.
    Kick,
    /// Keep a player out until a given time.
    Timeout,
    /// Ban a player.
    Ban,
    /// Lift a ban.
    Unban,
}

/// A player as the moderation panel shows them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AdminPlayerView {
    /// Account id across modules.
    pub global_id: String,
    /// Name shown to players.
    pub display_name: String,
    /// The player's id inside the module.
    pub module_user_id: String,
    /// Connected now.
    pub online: bool,
    /// Banned on this server.
    pub banned: bool,
    /// Reason given for the ban.
    pub ban_reason: Option<String>,
    /// When the timeout ends, RFC 3339.
    pub timeout_until: Option<String>,
    /// When the account was made, RFC 3339.
    pub created_at: Option<String>,
    /// Last sign-in, RFC 3339.
    pub last_login_at: Option<String>,
}

/// What a moderation action did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AdminResultView {
    /// The action taken.
    pub action: AdminActionKind,
    /// Whether it took effect.
    pub ok: bool,
    /// Words from the server.
    pub message: Option<String>,
    /// The player after the action.
    pub player: Option<AdminPlayerView>,
}

/// The `module_admin` answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdminOutcome {
    /// The action ran.
    Ok(AdminResultView),
    /// The action could not run.
    Err(LauncherError),
}

impl Serialize for AdminOutcome {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Done<'a> {
            ok: bool,
            result: &'a AdminResultView,
        }
        match self {
            Self::Ok(result) => Done { ok: true, result }.serialize(serializer),
            Self::Err(error) => Failed { ok: false, error }.serialize(serializer),
        }
    }
}
