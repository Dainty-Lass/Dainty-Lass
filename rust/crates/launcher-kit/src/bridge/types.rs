// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidInvite,
    RevivedInvite,
    InviteLegacyEdition,
    NoServer,
    ServerUnreachable,
    ServerError,
    CertMismatch,
    InvalidUsername,
    UsernameTaken,
    InvalidInviteCode,
    InviteExhausted,
    RegistrationClosed,
    InvalidKey,
    KeyRejected,
    AccountBanned,
    AccountPending,
    AccountRequired,
    KeystoreFailed,
    GameNotFound,
    GameFolderNotFound,
    GameNotVerified,
    GameWrongVersion,
    VerifyFailed,
    DiskSpace,
    RuntimeUnavailable,
    RuntimeNotOffered,
    RuntimeUntrusted,
    RuntimeFailed,
    RuntimeDifferentOnServer,
    RuntimeInUse,
    RuntimeLocalIncomplete,
    LocalRuntimeInvalid,
    LoaderLeftovers,
    AlreadyRunning,
    GameRunning,
    LaunchFailed,
    RelayFailed,
    CompatRuntimeMissing,
    HostFailed,
    ServerBusy,
    ServerStopped,
    Busy,
    Cancelled,
    NotInThisBuild,
    UpdateFailed,
    EpicSignInFailed,
    EpicNotSignedIn,
    EpicSignedOut,
    EpicNotOwned,
    EpicSessionExpired,
    EpicUnreachable,
    EpicLibraryFailed,
    GameVersionUnknown,
    GameVersionUnavailable,
    InstallBusy,
    InstallFailed,
    GuardedNotFound,
    GuardedNotWaiting,
    GuardedForbidden,
    GuardedInvalidState,
    GuardedFailed,
    ModuleUnknown,
    ModuleInvalidId,
    ModuleNotFound,
    ModuleInputInvalid,
    ModuleSettingsInvalid,
    ModuleInvalidSettings,
    ModuleInvalidCommand,
    ModuleCommandFailed,
    ModuleFailed,
    HistoryInvalidFilter,
    HistoryPlanExpired,
    HistoryPlanChanged,
    HistoryFailed,
    RealmUnavailable,
    RealmInvalidId,
    RealmNotFound,
    RealmFull,
    RealmBusy,
    RealmFailed,
    AdminInvalidInput,
    AdminNotFound,
    AdminFailed,
    ManifestMismatch,
    ManifestUnavailable,
    NoContentServer,
    FileMissingOnServer,
    FileDifferentOnServer,
    DllFailed,
    GameFilesInvalid,
    WindowsRuntimeMissing,
    FolderOtherGame,
    FolderNotEmpty,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct LauncherError {
    pub code: ErrorCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl LauncherError {
    pub fn new(code: ErrorCode) -> Self {
        Self { code, detail: None }
    }

    pub fn with_detail(code: ErrorCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: Some(detail.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum SimpleOutcome {
    Ok,
    Err { error: LauncherError },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct BuildCapabilities {
    pub kind: String,
    pub version: String,
    pub commit: String,
    pub platform: String,
    pub arch: String,
    pub commands: Vec<String>,
    pub events: Vec<String>,
    pub server_gui: bool,
    pub realms: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ServerFlavor {
    Reforged,
    Revived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ServerConnectionState {
    Unknown,
    Online,
    Offline,
    Unreachable,
    CertMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ServerProfile {
    pub id: String,
    pub display_name: String,
    pub host: String,
    pub port: u16,
    pub cert_pin: String,
    pub flavor: ServerFlavor,
    #[serde(default)]
    pub game_versions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    pub has_key: bool,
    pub connection: ServerConnectionState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct GameInstallState {
    pub verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable_sha256: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum GameStatus {
    Idle,
    Verifying,
    DownloadingRuntime,
    Installing,
    Launching,
    Running,
    Stopping,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct LinuxCompatSettings {
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_runner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix_dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct LauncherSettings {
    pub close_to_tray: bool,
    pub verbose_logging: bool,
    pub auto_check_updates: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linux_compat: Option<LinuxCompatSettings>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct LauncherSnapshot {
    pub servers: Vec<ServerProfile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_server_id: Option<String>,
    pub game: GameInstallState,
    #[serde(default)]
    pub game_installs: BTreeMap<String, GameInstallState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_game_version: Option<String>,
    pub game_status: GameStatus,
    pub settings: LauncherSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct InvitePreview {
    pub display_name: String,
    pub host: String,
    pub port: u16,
    pub cert_pin: String,
    pub flavor: ServerFlavor,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invite_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum InviteOutcome {
    Ok {
        preview: InvitePreview,
        #[serde(skip_serializing_if = "Option::is_none")]
        existing_server_id: Option<String>,
    },
    Err {
        error: LauncherError,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ServerProbe {
    pub reachable: bool,
    pub online: bool,
    pub registration_open: bool,
    pub invite_required: bool,
    #[serde(default)]
    pub game_versions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub player_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ServerProbeOutcome {
    Ok { probe: ServerProbe },
    Err { error: LauncherError },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum AccountOutcome {
    Ok {
        server: ServerProfile,
        #[serde(skip_serializing_if = "Option::is_none")]
        key_backup: Option<String>,
    },
    Err {
        error: LauncherError,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum KeyExportOutcome {
    Ok { key: String },
    Err { error: LauncherError },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum VerifyOutcome {
    Ok { game: GameInstallState },
    Err { error: LauncherError },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum LaunchOutcome {
    Ok,
    Err { error: LauncherError },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum GameVersionInstall {
    Epic,
    Locate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum GameVersionOrigin {
    Launcher,
    BuiltIn,
    Server,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum EpicManifestState {
    Unknown,
    Ready,
    Invalid,
    Unreachable,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct EpicVersionInfo {
    pub build_version: String,
    pub manifest: EpicManifestState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct GameVersionEntry {
    pub version: String,
    pub display_name: String,
    pub origin: GameVersionOrigin,
    pub install: GameVersionInstall,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub epic: Option<EpicVersionInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct GameVersionList {
    pub versions: Vec<GameVersionEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct EpicAccountView {
    pub signed_in: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owns_dauntless: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum EpicSignInStartOutcome {
    Ok {
        login_url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        account: Option<EpicAccountView>,
    },
    Err {
        error: LauncherError,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum EpicAccountOutcome {
    Ok { account: EpicAccountView },
    Err { error: LauncherError },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum InstallStage {
    Preparing,
    FetchingManifest,
    Planning,
    Downloading,
    Verifying,
    Finalizing,
    Complete,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct InstallProgress {
    pub version: String,
    pub stage: InstallStage,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub files_done: u32,
    pub files_total: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed_bps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum InstallOutcome {
    Ok { game: GameInstallState },
    Err { error: LauncherError },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct LinuxCompatCandidate {
    pub id: String,
    pub label: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct LinuxCompatReport {
    pub supported: bool,
    pub candidates: Vec<LinuxCompatCandidate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    pub update_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum UpdateOutcome {
    Ok { info: UpdateInfo },
    Err { error: LauncherError },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum RegionProblem {
    CertMismatch,
    Timeout,
    Unreachable,
    BadAnswer,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RegionRtt {
    pub region: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtt_ms: Option<f64>,
    pub samples: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<RegionProblem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RegionPing {
    Ok {
        regions: Vec<RegionRtt>,
        reported: bool,
    },
    Err(LauncherError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
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
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum RequirementState {
    Ok,
    Found,
    Missing,
    Problem,
    Outdated,
    Checking,
    Installing,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Requirement {
    pub id: RequirementId,
    pub state: RequirementState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RequirementItem {
    pub id: RequirementId,
    pub label: String,
    pub required: bool,
    pub state: RequirementState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    pub can_install: bool,
    pub can_locate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RequirementsReport {
    pub items: Vec<RequirementItem>,
    pub all_required_met: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum GameDataState {
    Missing,
    Synthetic,
    Real,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostToolchain {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visual_studio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msbuild: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cargo: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostDump {
    pub dir: String,
    pub name: String,
    pub modified: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostMultipliers {
    pub xp: f64,
    pub rewards: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostServerConfig {
    pub configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multipliers: Option<HostMultipliers>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostJob {
    pub kind: String,
    pub step: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostUpdateStatus {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostKit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    pub workspace_ok: bool,
    pub sdk_present: bool,
    pub sdk_functions: u32,
    pub runtime_built: bool,
    pub loader_built: bool,
    pub exporter_built: bool,
    pub runtime_published: bool,
    pub backend_data: GameDataState,
    pub director_data: GameDataState,
    pub exports_ready: bool,
    pub server: HostServerConfig,
    pub toolchain: HostToolchain,
    pub dumps: Vec<HostDump>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dumper_dll: Option<String>,
    pub requirements: Vec<Requirement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job: Option<HostJob>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_update: Option<HostUpdateStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum HostServerStatus {
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostServerState {
    pub status: HostServerStatus,
    pub config: HostServerConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cert_pin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invite_string: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_players: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uptime_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum HostServerOutcome {
    Ok { state: HostServerState },
    Err { error: LauncherError },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostLogEntry {
    pub seq: u64,
    pub timestamp_ms: u64,
    pub level: String,
    pub source: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum GuardedExecutionState {
    Waiting,
    Approved,
    Denied,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct GuardedExecutionView {
    pub id: String,
    pub title: String,
    pub command: String,
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub reason: String,
    pub state: GuardedExecutionState,
    pub created_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum GuardedListOutcome {
    Ok { items: Vec<GuardedExecutionView> },
    Err { error: LauncherError },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum GuardedOutcome {
    Ok { item: GuardedExecutionView },
    Err { error: LauncherError },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ModuleRole {
    Game,
    Plugin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ModuleOrigin {
    Compiled,
    CompiledIn,
    Directory,
    Configured,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ModuleRunState {
    Stopped,
    Starting,
    Ready,
    Running,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ModulePortView {
    pub name: String,
    pub transport: String,
    pub first: u16,
    pub last: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ModuleArgView {
    pub name: String,
    pub required: bool,
    pub help: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ModuleCommandView {
    pub name: String,
    pub summary: String,
    pub args: Vec<ModuleArgView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(untagged)]
pub enum ModuleSettingValue {
    Flag(bool),
    Number(f64),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ModuleSettingView {
    pub name: String,
    pub help: String,
    pub default: ModuleSettingValue,
    pub value: ModuleSettingValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ModuleSectionView {
    pub name: String,
    pub keys: Vec<ModuleSettingView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ModulePatchSetView {
    pub provides: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ModulePaletteEntry {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ModuleThemeView {
    pub id: String,
    pub display_name: String,
    pub palette: Vec<ModulePaletteEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ModuleCapabilitiesView {
    pub realms: bool,
    pub network: bool,
    pub chat: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch_set: Option<ModulePatchSetView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<ModuleThemeView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ModuleView {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<ModuleRole>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_version: Option<String>,
    pub origin: ModuleOrigin,
    pub autoload: bool,
    pub state: ModuleRunState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub unverified: bool,
    pub ports: Vec<ModulePortView>,
    pub capabilities: ModuleCapabilitiesView,
    pub commands: Vec<ModuleCommandView>,
    pub settings: Vec<ModuleSectionView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ModuleListOutcome {
    Ok { modules: Vec<ModuleView> },
    Err { error: LauncherError },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum ModuleOutcome {
    Ok { module: ModuleView },
    Err { error: LauncherError },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ModuleCommandOutcome {
    Ok {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(any(test, feature = "ts"), ts(type = "unknown"))]
        data: Option<serde_json::Value>,
    },
    Err {
        error: LauncherError,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntryView {
    pub id: String,
    pub timestamp_ms: u64,
    pub category: String,
    pub action: String,
    pub actor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub summary: String,
    pub reversible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HistoryPageView {
    pub entries: Vec<HistoryEntryView>,
    pub total: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum HistoryListOutcome {
    Ok { page: HistoryPageView },
    Err { error: LauncherError },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RollbackStepView {
    pub entry_id: String,
    pub description: String,
    pub impact: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RollbackPlanView {
    pub plan_id: String,
    pub target_entry_id: String,
    pub steps: Vec<RollbackStepView>,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RollbackPreviewOutcome {
    Ok { plan: RollbackPlanView },
    Err { error: LauncherError },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RollbackApplyOutcome {
    Ok { applied_count: u32 },
    Err { error: LauncherError },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum RealmState {
    Stopped,
    Starting,
    Online,
    Stopping,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RealmView {
    pub id: String,
    pub name: String,
    pub module_id: String,
    pub game_version: String,
    pub state: RealmState,
    pub players_online: u32,
    pub max_players: u32,
    pub invite_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RealmListOutcome {
    Ok { realms: Vec<RealmView> },
    Err { error: LauncherError },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RealmOutcome {
    Ok { realm: RealmView },
    Err { error: LauncherError },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum AdminActionKind {
    Ban,
    Unban,
    Kick,
    Mute,
    Unmute,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum AdminOutcome {
    Ok {
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    Err {
        error: LauncherError,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum AdminAccountStatus {
    Active,
    Pending,
    Banned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AdminAccountView {
    pub id: String,
    pub username: String,
    pub status: AdminAccountStatus,
    pub created_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum AdminAccountListOutcome {
    Ok { accounts: Vec<AdminAccountView> },
    Err { error: LauncherError },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum AdminAccountOutcome {
    Ok { account: AdminAccountView },
    Err { error: LauncherError },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AdminInviteView {
    pub code: String,
    pub max_uses: u32,
    pub uses: u32,
    pub created_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub revoked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum AdminInviteListOutcome {
    Ok { invites: Vec<AdminInviteView> },
    Err { error: LauncherError },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum AdminInviteOutcome {
    Ok { invite: AdminInviteView },
    Err { error: LauncherError },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AdminSessionView {
    pub session_id: String,
    pub account_id: String,
    pub username: String,
    pub connected_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub realm_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum AdminSessionListOutcome {
    Ok { sessions: Vec<AdminSessionView> },
    Err { error: LauncherError },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum LauncherEvent {
    Snapshot { snapshot: LauncherSnapshot },
    GameStatus { status: GameStatus },
    VerifyProgress {
        stage: String,
        bytes_done: u64,
        bytes_total: u64,
    },
    InstallProgress { progress: InstallProgress },
    ServerConnection {
        server_id: String,
        connection: ServerConnectionState,
    },
    RequirementsUpdated { report: RequirementsReport },
    HostServerUpdated { state: HostServerState },
    HostLog { entry: HostLogEntry },
    GuardedUpdated { item: GuardedExecutionView },
    ModulesUpdated { modules: Vec<ModuleView> },
    RealmsUpdated { realms: Vec<RealmView> },
    Notice {
        level: String,
        code: ErrorCode,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RuntimeFileInfo {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum RuntimeState {
    Ok,
    UpToDate,
    Installed,
    Missing,
    Outdated,
    Untrusted,
    Unavailable,
    Local,
    Failed,
}

