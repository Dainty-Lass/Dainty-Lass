// Generated from the launcher backend (launcher/src-tauri/src/bridge); do not edit.
// Regenerate with: RR_UPDATE_GENERATED=1 cargo test -p dainty-launcher generated

export type ServerMode = "private" | "public";
export type PlayerWhere = "menu" | "city" | "hunt" | "dojo" | "tutorial" | "unknown";
export type InstanceKind = "city" | "hunt" | "dojo" | "tutorial";
export type RegistrationMode = "OPEN" | "INVITECODE" | "NONE";
export type StatusPlayer = { 
/**
 * Display name, at most 32 characters.
 */
name: string, 
/**
 * Where the player is, `where` on the wire.
 */
where: PlayerWhere, 
/**
 * Id of the player's instance, if any.
 */
instance: string | null, };
export type StatusInstance = { 
/**
 * Instance id, at most 64 characters.
 */
id: string, 
/**
 * What kind of instance it is.
 */
kind: InstanceKind, 
/**
 * Display title, empty when the server sends none.
 */
title: string, 
/**
 * Map name, empty when the server sends none.
 */
map: string, 
/**
 * Behemoth being hunted, if any.
 */
behemoth: string | null, 
/**
 * Players in the instance, clamped to 1000.
 */
players: number, 
/**
 * Instance capacity, clamped to 1000.
 */
maxPlayers: number, 
/**
 * Start time as an ISO 8601 UTC string, when valid.
 */
startedAt: string | null, };
export type GameDataKind = "real" | "synthetic" | "missing" | "unknown";
export type StatusGame = { 
/**
 * Build name.
 */
build: string, 
/**
 * Game changelist, if reported.
 */
changelist: number | null, };
export type ServerStatus = { 
/**
 * Server name, at most 64 characters.
 */
name: string, 
/**
 * Whether the game backend is up; false when missing.
 */
online: boolean, 
/**
 * Server software version, empty when missing.
 */
version: string, 
/**
 * Server software commit, empty when missing.
 */
commit: string, 
/**
 * Source code link, `https` only.
 */
sourceUrl: string | null, 
/**
 * Registration mode, if reported.
 */
registration: RegistrationMode | null, 
/**
 * Count of online players, zero in a limited view.
 */
playersOnline: number, 
/**
 * Online players, at most 500; empty in a limited view.
 */
players: Array<StatusPlayer>, 
/**
 * Running instances, at most 100; empty in a limited view.
 */
instances: Array<StatusInstance>, 
/**
 * Seconds since the server started.
 */
uptimeSeconds: number, 
/**
 * Game build, if reported.
 */
game: StatusGame | null, 
/**
 * Where the game data comes from.
 */
gameData: GameDataKind, 
/**
 * Whether the server runs chat.
 */
chat: boolean, 
/**
 * Port of the content server, when it is published separately.
 */
contentPort: number | null, 
/**
 * Whether player and instance lists were withheld (see [`limited_view`]).
 */
limited: boolean, };
export type Language = "en" | "pt-BR";
export type GameLanguage = "auto" | "en-US" | "de-DE" | "es-ES" | "fr-FR" | "it-IT" | "ja-JP" | "pt-BR" | "ru-RU";
export type RuntimeProfile = "production" | "development";
export type Phase = "loading" | "join" | "connect" | "register" | "locate" | "runtime" | "installing" | "ready" | "running";
export type ConnectProblem = "tailscale_missing" | "unreachable" | "bad_answer" | "cert_mismatch";
export type GameProblem = "not_set" | "not_found" | "unverified" | "wrong_version";
export type ServerGame = "reforged" | "revived";
export type Game = "reborn" | "legacy";
export type RuntimeState = "ok" | "missing" | "outdated" | "untrusted" | "unavailable" | "local";
export type ErrorCode = "account_banned" | "invite_invalid_format" | "invite_invalid" | "username_invalid" | "username_taken" | "registration_closed" | "bad_request" | "server_unreachable" | "server_error" | "key_invalid" | "key_rejected" | "key_storage_unavailable" | "folder_invalid" | "game_folder_not_found" | "game_wrong_version" | "runtime_unavailable" | "runtime_untrusted" | "runtime_failed" | "runtime_different_on_server" | "runtime_in_use" | "local_runtime_invalid" | "download_failed" | "disk_space" | "already_running" | "launch_failed" | "compat_runtime_missing" | "config_failed" | "busy" | "cancelled" | "cert_mismatch" | "cert_changed" | "rate_limited" | "relay_port_busy" | "relay_failed" | "loader_leftovers" | "host_failed" | "manifest_mismatch" | "no_content_server" | "file_missing_on_server" | "file_different_on_server" | "dll_failed" | "game_files_invalid" | "windows_runtime_missing" | "folder_other_game" | "folder_not_empty" | "edition_other" | "epic_signed_out" | "epic_library_failed" | "epic_not_owned" | "not_implemented" | "not_in_this_build" | "epic_code_invalid" | "epic_rejected" | "epic_unreachable" | "epic_failed" | "game_version_unknown" | "game_version_unavailable" | "epic_manifest_unavailable" | "guarded_not_found" | "guarded_not_waiting" | "server_stopped" | "module_unknown" | "module_bad_state" | "module_input_invalid" | "module_settings_invalid" | "module_failed" | "history_bad_filter" | "history_unavailable" | "history_plan_changed" | "history_failed" | "realm_unavailable" | "realm_not_found" | "realm_failed" | "unknown";
export type NoticeCode = "keep_open" | "backup_saved" | "runtime_installed" | "runtime_installed_restart" | "game_verified" | "sdk_imported" | "host_done" | "install_done" | "repair_done";
export type LauncherError = { 
/**
 * Why the command failed.
 */
code: ErrorCode, 
/**
 * Text the catalog sentence shows in place of `{detail}`.
 */
detail?: string, };
export type TaskKind = "verify" | "runtime" | "download" | "dlls" | "install";
export type TaskProgress = { kind: TaskKind, totalBytes: number, doneBytes: number, filesTotal: number, filesDone: number, bytesPerSecond: number, etaSeconds: number | null, currentFile: string | null, };
export type ServerInfo = { game: ServerGame, mode: ServerMode, host: string, port: number, name: string, hasShare: boolean, 
/**
 * An invite code is stored and not used yet
 */
hasPendingInvite: boolean, loopback: boolean, 
/**
 * Public mode: the pinned certificate fingerprint (not secret)
 */
fingerprint: string | null, };
export type SavedServerInfo = { 
/**
 * The account key slot id (not secret)
 */
id: string, game: ServerGame, mode: ServerMode, host: string, port: number, name: string, current: boolean, username: string | null, 
/**
 * A Dauntless Revived (1.4.4) server in a build without the `legacy` feature: listed with its key
 * kept, never joined (RD5, RD22)
 */
unsupported: boolean, };
export type GameSource = "epic" | "legendary" | "heroic" | "chosen" | "revived";
export type DetectedGame = { root: string, source: GameSource, 
/**
 * What the store says (Legendary, Heroic), or Version.txt
 */
version: string | null, };
export type GameInfo = { 
/**
 * The "Dauntless" folder, the one that holds Archon\
 */
root: string | null, problem: GameProblem | null, 
/**
 * Version.txt, e.g. dauntless_rel-1.14.7_Shipping_2024-04-01_14-59-21_647472
 */
versionText: string | null, 
/**
 * Version.txt names 1.14.7, changelist 647472
 */
versionOk: boolean, 
/**
 * The executable's SHA-256 is the 1.14.7 one (checked once per file change)
 */
exeVerified: boolean, 
/**
 * Installs found on this PC (Epic, Legendary, Heroic)
 */
detected: Array<DetectedGame>, };
export type RevivedInfo = { 
/**
 * The chosen folder, or where a download goes when none is chosen
 */
root: string, 
/**
 * The player chose it (or it was found), rather than the default
 */
chosen: boolean, 
/**
 * `not_set`: nothing there yet; `unverified`: there, but not every file checked
 */
problem: GameProblem | null, 
/**
 * Files to download, as far as a look by size knows
 */
missingFiles: number, 
/**
 * The whole game
 */
totalBytes: number, 
/**
 * The pinned Undaunted DLLs are next to the game
 */
dllsOk: boolean, 
/**
 * 1.4.4 installs found on this PC (Dauntless Revived's launcher)
 */
detected: Array<DetectedGame>, };
export type InstallMode = "install" | "repair";
export type RuntimeFileInfo = { 
/**
 * File name inside the runtime folder.
 */
name: string, 
/**
 * Size in bytes.
 */
size: number, 
/**
 * Lowercase hex SHA-256 of the contents.
 */
sha256: string, };
export type RuntimeInfo = { state: RuntimeState, 
/**
 * The server's runtime version
 */
version: string | null, 
/**
 * What the server offers (shown when the player is asked to trust it)
 */
files: Array<RuntimeFileInfo>, localDir: string | null, };
export type Settings = { graphics: GraphicsPreset, windowed: boolean, language: Language, gameLanguage: GameLanguage, runtimeProfile: RuntimeProfile, 
/**
 * Hide addresses, fingerprints, paths and player names on screen
 */
streamerMode: boolean, 
/**
 * Show the Host page in the rail
 */
hostTools: boolean, 
/**
 * Which game the launcher plays: its folder, pins and PLAY. Always `reborn` without the `legacy` feature
 */
gameEdition: Game, 
/**
 * Minimising hides the window to the tray
 */
minimizeToTray: boolean, 
/**
 * Closing hides the window to the tray; the tray's Quit closes it
 */
closeToTray: boolean, 
/**
 * Leave the game's console window on screen; hidden otherwise, its lines read into Game log
 */
showGameConsole: boolean, };
export type SnapshotConnect = { checking: boolean, problem: ConnectProblem | null, tailscaleInstalled: boolean, lastCheckedAt: string | null, };
export type SnapshotAccount = { username: string | null, hasKey: boolean, offerBackup: boolean, };
export type SnapshotRunning = { game: boolean, relayPort: number | null, };
export type SnapshotApp = { version: string, packaged: boolean, updateReady: boolean, 
/**
 * `process.platform`: "win32" or "linux"
 */
platform: string, 
/**
 * False in a player-only build (no `host` feature): the page offers no Host tools
 */
hostToolsAvailable: boolean, 
/**
 * Always false since the legacy edition went (1.4.4, RD5): the page offers no edition setting
 */
legacyAvailable: boolean, 
/**
 * The game versions an invite may name in this build (`dainty_protocol::invite::played_versions`)
 */
gameVersions: Array<string>, 
/**
 * The server GUI (`server-gui` feature): the page shows the server screens (DU3)
 */
serverGui: boolean, };
export type Snapshot = { phase: Phase, busy: boolean, server: ServerInfo | null, savedServers: Array<SavedServerInfo>, connect: SnapshotConnect, account: SnapshotAccount, game: GameInfo, 
/**
 * The current server is a Dauntless Revived one, in the legacy edition
 */
revived: RevivedInfo | null, runtime: RuntimeInfo, task: TaskProgress | null, running: SnapshotRunning, settings: Settings, app: SnapshotApp, status: ServerStatus | null, 
/**
 * The host answers but has no ServerStatus endpoint
 */
statusUnsupported: boolean, lastError: LauncherError | null, notice: NoticeCode | null, };
export type NewsItem = { date: string | null, title: string, body: string, };
export type Background = { 
/**
 * An app-internal `rr-art://` URL
 */
url: string, credit: string | null, };
export type Branding = { backgrounds: Array<Background>, accent: string | null, };
export type GameDataState = "real" | "synthetic" | "missing";
export type HostUpstream = { 
/**
 * The runtime source (`dllmain.cpp`) is present.
 */
present: boolean, 
/**
 * The source carries the runtime patches.
 */
patched: boolean, };
export type HostSdk = { 
/**
 * `SDK.hpp` is present.
 */
present: boolean, 
/**
 * Number of `*_functions.cpp` files in the SDK folder.
 */
functionFiles: number, };
export type HostDump = { 
/**
 * Path of the dump's `CppSDK` folder.
 */
dir: string, 
/**
 * The dump's folder name.
 */
name: string, 
/**
 * When `SDK.hpp` was written, RFC 3339 in UTC.
 */
modified: string, };
export type HostBuilds = { 
/**
 * The runtime DLL is built.
 */
runtime: boolean, 
/**
 * The runtime loader DLL is built.
 */
loader: boolean, 
/**
 * The CatalogExporter DLL is built.
 */
exporter: boolean, 
/**
 * The runtime files are in the publish folder.
 */
published: boolean, };
export type HostToolchain = { 
/**
 * Display name of the Visual Studio install with the C++ workload.
 */
visualStudio: string | null, 
/**
 * MSBuild was found.
 */
msbuild: boolean, 
/**
 * Cargo was found.
 */
cargo: boolean, };
export type HostGameData = { 
/**
 * The backend's game data.
 */
backend: GameDataState, 
/**
 * The director's game data.
 */
director: GameDataState, };
export type HostExports = { 
/**
 * The exports folder is present.
 */
present: boolean, 
/**
 * The exports came from a running game.
 */
inGame: boolean, };
export type HostServerMode = "private" | "public";
export type HostServerConfig = { 
/**
 * `host.toml` exists and parsed.
 */
configured: boolean, 
/**
 * Server name players see.
 */
name: string | null, 
/**
 * Private or public mode.
 */
mode: HostServerMode | null, 
/**
 * Public host name, in public mode.
 */
host: string | null, 
/**
 * Test multipliers, when set.
 */
multipliers: HostMultipliers | null, };
export type HostKit = { 
/**
 * Workspace folder, if one is chosen.
 */
workspace: string | null, 
/**
 * The folder is a workspace (`dainty-host` plus the module sources).
 */
workspaceOk: boolean, 
/**
 * Runtime upstream source state.
 */
upstream: HostUpstream, 
/**
 * Imported SDK state.
 */
sdk: HostSdk, 
/**
 * Dumper-7 dumps found, newest first.
 */
dumps: Array<HostDump>, 
/**
 * Path of the Dumper-7 DLL, once found.
 */
dumperDll: string | null, 
/**
 * Build outputs present.
 */
builds: HostBuilds, 
/**
 * Toolchain found.
 */
toolchain: HostToolchain, 
/**
 * Game-data state.
 */
gameData: HostGameData, 
/**
 * Exported item data state.
 */
exports: HostExports, 
/**
 * Server settings.
 */
serverConfig: HostServerConfig, 
/**
 * Requirement checks.
 */
requirements: Array<Requirement>, 
/**
 * The current or last job.
 */
job: HostJob | null, 
/**
 * The last workspace update.
 */
update: HostUpdateView | null, 
/**
 * Folder the project download goes to.
 */
projectTarget: string | null, };
export type HostMultipliers = { 
/**
 * Experience multiplier.
 */
xp: number, 
/**
 * Reward multiplier.
 */
rewards: number, };
export type HostUpdateState = "up_to_date" | "updated";
export type HostUpdateView = { 
/**
 * Whether anything changed.
 */
state: HostUpdateState, 
/**
 * The commit the workspace is now at.
 */
head: string, 
/**
 * Number of files the update changed.
 */
files: number, 
/**
 * The update touched the runtime source, so the runtime needs a rebuild.
 */
runtimeChanged: boolean, };
export type RequirementId = "dauntless" | "git" | "node" | "mongodb" | "tailscale" | "visual_studio" | "rust" | "dumper7" | "vc_redist";
export type RequirementState = "ok" | "missing" | "problem";
export type Requirement = { 
/**
 * Which requirement.
 */
id: RequirementId, 
/**
 * The check's result.
 */
state: RequirementState, 
/**
 * What was found, such as a version or path.
 */
detail: string | null, 
/**
 * Why a found requirement is unusable.
 */
problem: string | null, };
export type HostServerState = "stopped" | "starting" | "running" | "stopping" | "external" | "failed";
export type HostInvite = { 
/**
 * The invite link to hand to a player.
 */
link: string, 
/**
 * The player name the invite was made for, if any; `for` on the wire.
 */
for: string | null, };
export type HostServerView = { 
/**
 * Server lifecycle state.
 */
state: HostServerState, 
/**
 * Words on the last failure or transition.
 */
message: string | null, 
/**
 * Recent console lines.
 */
lines: Array<string>, 
/**
 * The latest invite made.
 */
invite: HostInvite | null, 
/**
 * The server runs inside this app.
 */
embedded: boolean, 
/**
 * Each service's state.
 */
components: Array<HostServerComponent>, };
export type HostServerComponent = { 
/**
 * Service name.
 */
name: string, 
/**
 * Its lifecycle phase.
 */
phase: HostComponentPhase, 
/**
 * The port it listens on, once known.
 */
port: number | null, 
/**
 * Words on its current phase.
 */
detail: string, };
export type HostComponentPhase = "stopped" | "starting" | "ready" | "stopping" | "failed" | "disabled";
export type HostJobKind = "inject" | "build_runtime" | "build_loader" | "build_exporter" | "import_sdk" | "copy_exports" | "synthetic_data" | "import_gamedata" | "publish_runtime" | "setup" | "generate_gamedata" | "export_gamedata" | "create_invite" | "update" | "multipliers" | "upstream" | "download_project" | "backup" | "migrate_mongo";
export type HostJobState = "running" | "done" | "failed";
export type HostJob = { 
/**
 * What the job does.
 */
kind: HostJobKind, 
/**
 * How far it got.
 */
state: HostJobState, 
/**
 * Last line of its output or its failure.
 */
message: string | null, 
/**
 * Path of its log file.
 */
logFile: string | null, };
export type ExternalTarget = "tailscale_download" | "tailscale_share" | "vc_redist" | "server_source" | "project_source" | "project_license" | "project_issues" | "project_docs_players" | "project_docs_hosting" | "project_docs_sdk" | "installing_guide" | "legendary" | "dumper7" | "visual_studio" | "rust" | "nodejs" | "mongodb" | "git" | "visual_studio_installer" | "rust_installer" | "dumper7_zip" | "revived_source" | "mystic_source" | "undaunted_source";
export type HostAction = "inject_dumper" | "import_sdk" | "build_runtime" | "build_loader" | "build_exporter" | "inject_exporter" | "copy_exports" | "synthetic_data" | "publish_runtime" | "generate_gamedata" | "export_gamedata";
export type HostFolder = "workspace" | "runtime_source" | "dumps" | "game_data" | "runtime_publish" | "host_logs" | "gamedata_packs";
export type HostSetupRequest = { 
/**
 * Server name players see.
 */
name: string, 
/**
 * Private or public mode.
 */
mode: HostServerMode, 
/**
 * Public host name, required in public mode.
 */
publicHost: string | null, };
export type RealmKind = "hub" | "hunt";
export type RegionProblem = "timeout" | "unreachable" | "cert_mismatch" | "bad_answer";
export type RegionRtt = { 
/**
 * Region id from the node list.
 */
region: string, 
/**
 * Median round trip in milliseconds, to a tenth; none when no sample came back.
 */
rttMs: number | null, 
/**
 * Number of samples that came back.
 */
samples: number, 
/**
 * Why the region could not be timed.
 */
problem: RegionProblem | null, };
export type GameVersionInstall = "epic" | "locate";
export type GameVersionOrigin = "launcher" | "server";
export type EpicManifestState = "ready" | "unreachable" | "invalid";
export type EpicVersionInfo = { 
/**
 * Epic's build version string.
 */
buildVersion: string, 
/**
 * State of the file list.
 */
manifest: EpicManifestState, 
/**
 * Bytes to download, from the file list.
 */
downloadBytes: number | null, 
/**
 * Bytes on disk once installed, from the file list.
 */
installBytes: number | null, };
export type GameVersionEntry = { 
/**
 * Game version, such as `1.14.7`.
 */
version: string, 
/**
 * Name shown to the player.
 */
displayName: string, 
/**
 * Who listed it.
 */
origin: GameVersionOrigin, 
/**
 * How it is installed.
 */
install: GameVersionInstall, 
/**
 * Epic download details, for versions installed through Epic.
 */
epic: EpicVersionInfo | null, };
export type EpicView = { signedIn: boolean, 
/**
 * The account's Epic display name; null when signed out or Epic gave none
 */
displayName: string | null, 
/**
 * The sign-in window is open, or a code is being exchanged
 */
signingIn: boolean, };
export type InstallState = "not_installed" | "resumable" | "preparing" | "downloading" | "paused" | "finishing" | "installed" | "failed";
export type InstallInfo = { version: string, state: InstallState, 
/**
 * The version's own folder
 */
folder: string | null, progress: TaskProgress | null, 
/**
 * Why the last attempt stopped
 */
error: LauncherError | null, };
export type GuardedState = "pending" | "approved" | "running" | "done" | "failed" | "canceled" | "expired";
export type GuardedMode = "delay" | "approval";
export type GuardedActionInfo = { 
/**
 * 16 hex digits
 */
id: string, 
/**
 * `ban`, `timeout`, `revokeKey` or `revert`
 */
kind: string, 
/**
 * One line of what it will do, in English: it comes from the admin API
 */
summary: string, state: GuardedState, mode: GuardedMode, 
/**
 * The admin key that asked: `owner` or a key id
 */
initiatedBy: string, 
/**
 * The lowest admin role that may approve it
 */
needsRole: string, createdAt: string, 
/**
 * When a delay ends; null for an approval
 */
releaseAt: string | null, expiresAt: string, approvedBy: string | null, canceledBy: string | null, endedAt: string | null, result: string | null, };
export type ModuleRole = "game" | "plugin";
export type ModuleOrigin = "compiled_in" | "external";
export type ModuleRunState = "not_loaded" | "starting" | "ready" | "failed";
export type ModulePortView = { 
/**
 * Name the module gave the range.
 */
name: string, 
/**
 * `tcp` or `udp`.
 */
transport: string, 
/**
 * First port of the range.
 */
first: number, 
/**
 * Last port of the range, inclusive.
 */
last: number, };
export type ModuleArgView = { 
/**
 * Argument name.
 */
name: string, 
/**
 * The command refuses to run without it.
 */
required: boolean, 
/**
 * Help text from the module.
 */
help: string, };
export type ModuleCommandView = { 
/**
 * Command name.
 */
name: string, 
/**
 * One-line summary from the module.
 */
summary: string, 
/**
 * Its arguments.
 */
args: Array<ModuleArgView>, };
export type ModuleSettingValue = boolean | number | string;
export type ModuleSettingView = { 
/**
 * Key name.
 */
name: string, 
/**
 * Help text from the module.
 */
help: string, 
/**
 * The module's default.
 */
default: ModuleSettingValue, 
/**
 * The value in `host.toml`, or the default.
 */
value: ModuleSettingValue, };
export type ModuleSectionView = { 
/**
 * Section name.
 */
name: string, 
/**
 * Its keys.
 */
keys: Array<ModuleSettingView>, };
export type ModulePatchSetView = { 
/**
 * True when the module provides a patch set, false when it accepts one.
 */
provides: boolean, 
/**
 * Name of the provided set.
 */
displayName: string | null, };
export type ModulePaletteEntry = { 
/**
 * CSS variable name.
 */
name: string, 
/**
 * Checked colour value.
 */
value: string, };
export type ModuleThemeView = { 
/**
 * Theme id.
 */
id: string, 
/**
 * Name shown to the player.
 */
displayName: string, 
/**
 * The colours it sets.
 */
palette: Array<ModulePaletteEntry>, 
/**
 * Layout name, if the theme picks one.
 */
layout: string | null, 
/**
 * Logo path, if the theme has one.
 */
logo: string | null, };
export type ModuleCapabilitiesView = { 
/**
 * It hosts realms.
 */
realms: boolean, 
/**
 * It uses the network plugin.
 */
network: boolean, 
/**
 * It uses the chat plugin.
 */
chat: boolean, 
/**
 * Its patch set, if any.
 */
patchSet: ModulePatchSetView | null, 
/**
 * Its theme, if any.
 */
theme: ModuleThemeView | null, };
export type ModuleView = { 
/**
 * Module id.
 */
id: string, 
/**
 * Name from the manifest.
 */
displayName: string | null, 
/**
 * Role from the manifest.
 */
role: ModuleRole | null, 
/**
 * Game version from the manifest, for a game module.
 */
gameVersion: string | null, 
/**
 * Where it comes from.
 */
origin: ModuleOrigin, 
/**
 * It loads when the server starts.
 */
autoload: boolean, 
/**
 * Its lifecycle state.
 */
state: ModuleRunState, 
/**
 * Words on a failure, bounded.
 */
detail: string | null, 
/**
 * Built but not checked against recorded traffic (DD9).
 */
unverified: boolean, 
/**
 * Port ranges it holds.
 */
ports: Array<ModulePortView>, 
/**
 * Manifest capabilities.
 */
capabilities: ModuleCapabilitiesView, 
/**
 * Commands it registered.
 */
commands: Array<ModuleCommandView>, 
/**
 * Settings it registered, with their values.
 */
settings: Array<ModuleSectionView>, };
export type HistoryEntryView = { id: string, 
/**
 * `node` (the game), `admin` or `chat`
 */
database: string, 
/**
 * RFC 3339
 */
at: string, 
/**
 * `player:<account>`, `admin:<key id>`, `host:<command>` or `system:<task>`
 */
actor: string, table: string, 
/**
 * The row's key, as JSON
 */
key: string, 
/**
 * `insert`, `update`, `delete`, `soft_delete`, `restore` or `migrate`
 */
op: string, before: string | null, after: string | null, };
export type HistoryPageView = { entries: Array<HistoryEntryView>, 
/**
 * Pass it back as the filter's `cursor` for the next, older page; null at the end
 */
next: string | null, };
export type RevertRowView = { database: string, table: string, key: string, 
/**
 * `insert` (a deleted row put back), `update`, `delete` (an added row removed) or `soft_delete`
 */
kind: string, 
/**
 * The row now and as the revert leaves it, as JSON
 */
current: string | null, target: string | null, };
export type RevertConflictView = { database: string, table: string, key: string, 
/**
 * `changed_since`, `not_as_logged`, `key_changed` or `schema_changed`
 */
reason: string, 
/**
 * `changed_since`: the newer entry, who made it and when
 */
entry: string | null, by: string | null, at: string | null, detail: string | null, };
export type RevertPlanView = { id: string, 
/**
 * Log entries the selection matches
 */
entries: number, actionCount: number, conflictCount: number, unchangedCount: number, 
/**
 * The first rows of each list; `truncated` says the counts are higher
 */
actions: Array<RevertRowView>, conflicts: Array<RevertConflictView>, truncated: boolean, };
export type RevertedInView = { database: string, rows: number, };
export type RevertDoneView = { 
/**
 * RFC 3339; the revert's own entries carry it
 */
at: string | null, rowsChanged: number, databases: Array<RevertedInView>, 
/**
 * Rows left as they are
 */
conflicts: Array<RevertConflictView>, conflictCount: number, truncated: boolean, };
export type RealmState = "starting" | "ready" | "running" | "ended";
export type RealmSessionView = { session: string, kind: RealmKind, hosting: boolean, generation: number, state: RealmState, 
/**
 * Local UDP port the game or server routes through when ready
 */
localPort: number | null, 
/**
 * Clean exit or failure code when `state` is `ended`
 */
result: string | null, };
export type RealmView = { 
/**
 * Whether this build and PC can host or join a realm (`ParadoxTransport.dll` present and co-op not disabled)
 */
available: boolean, 
/**
 * Active or most recently ended session on this PC
 */
active: RealmSessionView | null, 
/**
 * All tracked local sessions
 */
sessions: Array<RealmSessionView>, };
export type DoctorLevel = "ok" | "warning" | "problem";
export type DoctorCheckView = { level: DoctorLevel, label: string, detail: string, };
export type DoctorReportView = { checks: Array<DoctorCheckView>, warnings: number, problems: number, };
export type MongoMigrationState = "none" | "migrated" | "skipped" | "unmigrated" | "unreachable";
export type MongoMigrationView = { state: MongoMigrationState, source: string | null, detail: string | null, };
export type ServerConfigView = { configured: boolean, name: string, mode: HostServerMode, publicHost: string | null, registration: string, database: string, maxHubs: number, maxHunts: number, freshStart: boolean, mongoMigration: MongoMigrationView, configFile: string | null, };
export type ServerInviteEntry = { code: string, link: string, for: string | null, usesRemaining: number, infiniteUses: boolean, createdAt: string, };
export type AdminActionKind = "lookup" | "kick" | "timeout" | "ban" | "unban";
export type AdminPlayerView = { 
/**
 * Account id across modules.
 */
globalId: string, 
/**
 * Name shown to players.
 */
displayName: string, 
/**
 * The player's id inside the module.
 */
moduleUserId: string, 
/**
 * Connected now.
 */
online: boolean, 
/**
 * Banned on this server.
 */
banned: boolean, 
/**
 * Reason given for the ban.
 */
banReason: string | null, 
/**
 * When the timeout ends, RFC 3339.
 */
timeoutUntil: string | null, 
/**
 * When the account was made, RFC 3339.
 */
createdAt: string | null, 
/**
 * Last sign-in, RFC 3339.
 */
lastLoginAt: string | null, };
export type AdminResultView = { 
/**
 * The action taken.
 */
action: AdminActionKind, 
/**
 * Whether it took effect.
 */
ok: boolean, 
/**
 * Words from the server.
 */
message: string | null, 
/**
 * The player after the action.
 */
player: AdminPlayerView | null, };
export type ManagedInstallView = { node: string, region: string, label: string | null, 
/**
 * `"online"`, `"degraded"`, `"offline"` or `"revoked"`
 */
state: string, enrolledAtMs: number, lastSeenMs: number, revoked: boolean, appliedRevision: number, playersOnline: number, activeHunts: number, version: string | null, queuedCommands: number, };
export type RemoteStatusView = { 
/**
 * `"standalone"`, `"controller"` or `"managed"`
 */
role: string, node: string | null, region: string | null, controllerUrl: string | null, controllerCertFp: string | null, controllerNode: string | null, revision: number, pendingEnrollments: number, installs: Array<ManagedInstallView>, detached: boolean, };
export type RemoteInviteView = { uri: string, node: string, region: string, label: string | null, controllerUrl: string, controllerFp: string, expiresAtMs: number, };
export type RemoteExecView = { commandId: string, node: string, action: string, status: string, };
export type GraphicsPreset = -1 | 0 | 1 | 2 | 3 | 4;
export type ActionResult = { ok: true } | { ok: false; error: LauncherError };
export type InviteCheck = { ok: true; certificateChanged: boolean; previousFingerprint: string | null; movedFrom: string | null } | { ok: false; error: LauncherError };
export type RegisterOutcome = { ok: true; username: string } | { ok: false; error: LauncherError };
export type RegionPing = { ok: true; regions: Array<RegionRtt>; reported: boolean } | { ok: false; error: LauncherError };
export type EpicLibraryCheck = { ok: true; owned: boolean; checkedAt: string } | { ok: false; error: LauncherError };
export type GameVersionList = { ok: true; versions: Array<GameVersionEntry> } | { ok: false; error: LauncherError };
export type InstallOutcome = { ok: true; install: InstallInfo } | { ok: false; error: LauncherError };
export type GuardedList = { ok: true; actions: Array<GuardedActionInfo> } | { ok: false; error: LauncherError };
export type GuardedOutcome = { ok: true; action: GuardedActionInfo } | { ok: false; error: LauncherError };
export type ModuleListOutcome = { ok: true; modules: Array<ModuleView> } | { ok: false; error: LauncherError };
export type ModuleOutcome = { ok: true; module: ModuleView } | { ok: false; error: LauncherError };
export type ModuleCommandOutcome = { ok: true; text: string; data: string | null } | { ok: false; error: LauncherError };
export type HistoryOutcome = { ok: true; page: HistoryPageView } | { ok: false; error: LauncherError };
export type RevertPlanOutcome = { ok: true; plan: RevertPlanView } | { ok: false; error: LauncherError };
export type RevertApplyOutcome = { ok: true; done: RevertDoneView } | { ok: false; error: LauncherError };
export type RealmOutcome = { ok: true; realm: RealmView } | { ok: false; error: LauncherError };
export type ServerDoctorOutcome = { ok: true; report: DoctorReportView } | { ok: false; error: LauncherError };
export type ServerConfigOutcome = { ok: true; config: ServerConfigView } | { ok: false; error: LauncherError };
export type ServerInviteListOutcome = { ok: true; invites: Array<ServerInviteEntry> } | { ok: false; error: LauncherError };
export type AdminOutcome = { ok: true; result: AdminResultView } | { ok: false; error: LauncherError };
export type RemoteStatusOutcome = { ok: true; status: RemoteStatusView } | { ok: false; error: LauncherError };
export type RemoteInviteOutcome = { ok: true; invite: RemoteInviteView } | { ok: false; error: LauncherError };
export type RemoteExecOutcome = { ok: true; result: RemoteExecView } | { ok: false; error: LauncherError };
export type CommandName = "get_snapshot" | "check_invite" | "submit_invite" | "retry_connect" | "forget_server" | "switch_server" | "remove_saved_server" | "register" | "use_existing_key" | "import_key_file" | "save_key_backup" | "dismiss_backup_offer" | "choose_game_folder" | "use_game_path" | "use_detected_game" | "verify_game" | "revived_install" | "revived_choose_folder" | "revived_use_detected" | "trust_runtime" | "install_runtime" | "choose_local_runtime" | "clear_local_runtime" | "cancel_task" | "play" | "set_settings" | "set_status_polling" | "refresh_status" | "get_news" | "get_branding" | "open_external" | "open_game_folder" | "open_logs_folder" | "logout" | "install_update" | "host_get" | "host_choose_workspace" | "host_choose_dumper" | "host_import_gamedata" | "host_import_sdk_folder" | "host_download_project" | "host_action" | "host_open" | "host_server_get" | "host_server_start" | "host_server_stop" | "host_server_restart" | "host_setup" | "host_create_invite" | "host_update" | "host_set_multipliers" | "copy_text" | "take_invite_link" | "game_log_get" | "dismiss_notice" | "dismiss_error" | "subscribe_progress" | "epic_sign_in_start" | "epic_sign_in_code" | "epic_get" | "epic_sign_out" | "epic_library_check" | "game_version_list" | "game_install_start" | "game_install_pause" | "game_install_resume" | "game_install_cancel" | "game_install_get" | "region_ping" | "module_list" | "module_get" | "module_run_command" | "module_set_settings" | "module_load" | "module_unload" | "realm_host" | "realm_join" | "realm_stop" | "realm_get" | "history_query" | "history_revert_dry_run" | "history_revert_apply" | "guarded_list" | "guarded_approve" | "guarded_cancel" | "server_doctor" | "server_backup" | "server_config_get" | "server_config_save" | "server_migrate_mongo" | "server_confirm_fresh_start" | "server_invite_list" | "admin_action" | "remote_status" | "remote_init_controller" | "remote_invite" | "remote_enroll" | "remote_revoke" | "remote_leave" | "remote_exec";
