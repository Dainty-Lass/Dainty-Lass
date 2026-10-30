// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum RuntimeState {
    Ok,
    Missing,
    Outdated,
    Untrusted,
    Unavailable,
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct RuntimeFileInfo {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
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
    GameFolderNotFound,
    GameWrongVersion,
    RuntimeUnavailable,
    RuntimeUntrusted,
    RuntimeFailed,
    RuntimeDifferentOnServer,
    RuntimeInUse,
    LocalRuntimeInvalid,
    DownloadFailed,
    DiskSpace,
    AlreadyRunning,
    LaunchFailed,
    CompatRuntimeMissing,
    ConfigFailed,
    Busy,
    Cancelled,
    CertMismatch,
    CertChanged,
    RateLimited,
    RelayPortBusy,
    RelayFailed,
    LoaderLeftovers,
    HostFailed,
    ManifestMismatch,
    NoContentServer,
    FileMissingOnServer,
    FileDifferentOnServer,
    DllFailed,
    GameFilesInvalid,
    WindowsRuntimeMissing,
    FolderOtherGame,
    FolderNotEmpty,
    EditionOther,
    EpicSignedOut,
    EpicLibraryFailed,
    EpicNotOwned,
    NotImplemented,
    NotInThisBuild,
    EpicCodeInvalid,
    EpicRejected,
    EpicUnreachable,
    EpicFailed,
    GameVersionUnknown,
    GameVersionUnavailable,
    EpicManifestUnavailable,
    GuardedNotFound,
    GuardedNotWaiting,
    ServerStopped,
    ModuleUnknown,
    ModuleBadState,
    ModuleInputInvalid,
    ModuleSettingsInvalid,
    ModuleFailed,
    HistoryBadFilter,
    HistoryUnavailable,
    HistoryPlanChanged,
    HistoryFailed,
    RealmUnavailable,
    RealmNotFound,
    RealmFailed,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
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
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct LauncherError {
    pub code: ErrorCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "ts"), ts(optional))]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum GameDataState {
    Real,
    Synthetic,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct HostUpstream {
    pub present: bool,
    pub patched: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostSdk {
    pub present: bool,
    pub function_files: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct HostDump {
    pub dir: String,
    pub name: String,
    pub modified: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct HostBuilds {
    pub runtime: bool,
    pub loader: bool,
    pub exporter: bool,
    pub published: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostToolchain {
    pub visual_studio: Option<String>,
    pub msbuild: bool,
    pub cargo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct HostGameData {
    pub backend: GameDataState,
    pub director: GameDataState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostExports {
    pub present: bool,
    pub in_game: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum HostServerMode {
    Private,
    Public,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct HostMultipliers {
    pub xp: f64,
    pub rewards: f64,
}

pub const MAX_MULTIPLIER: f64 = 100.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct HostServerConfig {
    pub configured: bool,
    pub name: Option<String>,
    pub mode: Option<HostServerMode>,
    pub host: Option<String>,
    pub multipliers: Option<HostMultipliers>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum HostUpdateState {
    UpToDate,
    Updated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostUpdateView {
    pub state: HostUpdateState,
    pub head: String,
    pub files: u32,
    pub runtime_changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
#[serde(rename_all = "lowercase")]
pub enum RequirementState {
    Ok,
    Missing,
    Problem,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct Requirement {
    pub id: RequirementId,
    pub state: RequirementState,
    pub detail: Option<String>,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum HostServerState {
    Stopped,
    Starting,
    Running,
    Stopping,
    External,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct HostInvite {
    pub link: String,
    #[serde(rename = "for")]
    pub for_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct HostServerView {
    pub state: HostServerState,
    pub message: Option<String>,
    pub lines: Vec<String>,
    pub invite: Option<HostInvite>,
    pub embedded: bool,
    pub components: Vec<HostServerComponent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct HostServerComponent {
    pub name: String,
    pub phase: HostComponentPhase,
    pub port: Option<u16>,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
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
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
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
    Backup,
    MigrateMongo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum HostJobState {
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostJob {
    pub kind: HostJobKind,
    pub state: HostJobState,
    pub message: Option<String>,
    pub log_file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostKit {
    pub workspace: Option<String>,
    pub workspace_ok: bool,
    pub upstream: HostUpstream,
    pub sdk: HostSdk,
    pub dumps: Vec<HostDump>,
    pub dumper_dll: Option<String>,
    pub builds: HostBuilds,
    pub toolchain: HostToolchain,
    pub game_data: HostGameData,
    pub exports: HostExports,
    pub server_config: HostServerConfig,
    pub requirements: Vec<Requirement>,
    pub job: Option<HostJob>,
    pub update: Option<HostUpdateView>,
    pub project_target: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum HostAction {
    InjectDumper,
    ImportSdk,
    BuildRuntime,
    BuildLoader,
    BuildExporter,
    InjectExporter,
    CopyExports,
    SyntheticData,
    PublishRuntime,
    GenerateGamedata,
    ExportGamedata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HostSetupRequest {
    pub name: String,
    pub mode: HostServerMode,
    pub public_host: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum RealmKind {
    Hub,
    Hunt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum RegionProblem {
    Timeout,
    Unreachable,
    CertMismatch,
    BadAnswer,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RegionRtt {
    pub region: String,
    pub rtt_ms: Option<f64>,
    pub samples: u32,
    pub problem: Option<RegionProblem>,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum GameVersionInstall {
    Epic,
    Locate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum GameVersionOrigin {
    Launcher,
    Server,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum EpicManifestState {
    Ready,
    Unreachable,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct EpicVersionInfo {
    pub build_version: String,
    pub manifest: EpicManifestState,
    pub download_bytes: Option<u64>,
    pub install_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct GameVersionEntry {
    pub version: String,
    pub display_name: String,
    pub origin: GameVersionOrigin,
    pub install: GameVersionInstall,
    pub epic: Option<EpicVersionInfo>,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ModuleRole {
    Game,
    Plugin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ModuleOrigin {
    CompiledIn,
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ModuleRunState {
    NotLoaded,
    Starting,
    Ready,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct ModulePortView {
    pub name: String,
    pub transport: String,
    pub first: u16,
    pub last: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct ModuleArgView {
    pub name: String,
    pub required: bool,
    pub help: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct ModuleCommandView {
    pub name: String,
    pub summary: String,
    pub args: Vec<ModuleArgView>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(untagged)]
pub enum ModuleSettingValue {
    Flag(bool),
    Number(f64),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct ModuleSettingView {
    pub name: String,
    pub help: String,
    pub default: ModuleSettingValue,
    pub value: ModuleSettingValue,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct ModuleSectionView {
    pub name: String,
    pub keys: Vec<ModuleSettingView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ModulePatchSetView {
    pub provides: bool,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub struct ModulePaletteEntry {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ModuleThemeView {
    pub id: String,
    pub display_name: String,
    pub palette: Vec<ModulePaletteEntry>,
    pub layout: Option<String>,
    pub logo: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ModuleCapabilitiesView {
    pub realms: bool,
    pub network: bool,
    pub chat: bool,
    pub patch_set: Option<ModulePatchSetView>,
    pub theme: Option<ModuleThemeView>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ModuleView {
    pub id: String,
    pub display_name: Option<String>,
    pub role: Option<ModuleRole>,
    pub game_version: Option<String>,
    pub origin: ModuleOrigin,
    pub autoload: bool,
    pub state: ModuleRunState,
    pub detail: Option<String>,
    pub unverified: bool,
    pub ports: Vec<ModulePortView>,
    pub capabilities: ModuleCapabilitiesView,
    pub commands: Vec<ModuleCommandView>,
    pub settings: Vec<ModuleSectionView>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ModuleListOutcome {
    Ok(Vec<ModuleView>),
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

#[derive(Debug, Clone, PartialEq)]
pub enum ModuleOutcome {
    Ok(Box<ModuleView>),
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

#[derive(Debug, Clone, PartialEq)]
pub enum ModuleCommandOutcome {
    Ok { text: String, data: Option<String> },
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum AdminActionKind {
    Lookup,
    Kick,
    Timeout,
    Ban,
    Unban,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AdminPlayerView {
    pub global_id: String,
    pub display_name: String,
    pub module_user_id: String,
    pub online: bool,
    pub banned: bool,
    pub ban_reason: Option<String>,
    pub timeout_until: Option<String>,
    pub created_at: Option<String>,
    pub last_login_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AdminResultView {
    pub action: AdminActionKind,
    pub ok: bool,
    pub message: Option<String>,
    pub player: Option<AdminPlayerView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdminOutcome {
    Ok(AdminResultView),
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
