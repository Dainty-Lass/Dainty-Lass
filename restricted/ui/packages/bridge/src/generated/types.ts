// Generated from the launcher backend (launcher/src-tauri/src/bridge); do not edit.
// Regenerate with: RR_UPDATE_GENERATED=1 cargo test -p launcher generated

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
export type TaskProgress = { 
/**
 * The task.
 */
kind: TaskKind, 
/**
 * Bytes in all.
 */
totalBytes: number, 
/**
 * Bytes done.
 */
doneBytes: number, 
/**
 * Files in all.
 */
filesTotal: number, 
/**
 * Files done.
 */
filesDone: number, 
/**
 * Recent speed.
 */
bytesPerSecond: number, 
/**
 * Seconds left, once the speed is known.
 */
etaSeconds: number | null, 
/**
 * The file being worked on.
 */
currentFile: string | null, };
export type ServerInfo = { 
/**
 * The game it runs.
 */
game: ServerGame, 
/**
 * Private or public.
 */
mode: ServerMode, 
/**
 * Its address.
 */
host: string, 
/**
 * Its port.
 */
port: number, 
/**
 * Its name.
 */
name: string, 
/**
 * The invite carried a Tailscale share link.
 */
hasShare: boolean, 
/**
 * An invite is waiting to be used for registration.
 */
hasPendingInvite: boolean, 
/**
 * It runs on this PC.
 */
loopback: boolean, 
/**
 * The pinned certificate's SHA-256, in public mode.
 */
fingerprint: string | null, };
export type SavedServerInfo = { 
/**
 * Its id, 24 lower-case hex digits.
 */
id: string, 
/**
 * The game it runs.
 */
game: ServerGame, 
/**
 * Private or public.
 */
mode: ServerMode, 
/**
 * Its address.
 */
host: string, 
/**
 * Its port.
 */
port: number, 
/**
 * Its name.
 */
name: string, 
/**
 * The one joined now.
 */
current: boolean, 
/**
 * The account's name there, when known.
 */
username: string | null, 
/**
 * This build cannot play on it.
 */
unsupported: boolean, };
export type GameSource = "epic" | "legendary" | "heroic" | "chosen" | "revived";
export type DetectedGame = { 
/**
 * The folder.
 */
root: string, 
/**
 * Where it was found.
 */
source: GameSource, 
/**
 * Its version, when it could be read.
 */
version: string | null, };
export type GameInfo = { 
/**
 * The folder, once chosen.
 */
root: string | null, 
/**
 * What is wrong with it.
 */
problem: GameProblem | null, 
/**
 * The version it reports.
 */
versionText: string | null, 
/**
 * The version is the server's.
 */
versionOk: boolean, 
/**
 * The executable's hash is known.
 */
exeVerified: boolean, 
/**
 * Other folders found.
 */
detected: Array<DetectedGame>, };
export type RevivedInfo = { 
/**
 * Its folder.
 */
root: string, 
/**
 * The player chose it.
 */
chosen: boolean, 
/**
 * What is wrong with it.
 */
problem: GameProblem | null, 
/**
 * Files still to fetch.
 */
missingFiles: number, 
/**
 * Bytes still to fetch.
 */
totalBytes: number, 
/**
 * Its DLLs are in place.
 */
dllsOk: boolean, 
/**
 * Other folders found.
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
export type RuntimeInfo = { 
/**
 * Whether it can be used.
 */
state: RuntimeState, 
/**
 * Its version, when known.
 */
version: string | null, 
/**
 * Its files.
 */
files: Array<RuntimeFileInfo>, 
/**
 * A local runtime folder that replaces the server's.
 */
localDir: string | null, };
export type Settings = { 
/**
 * Graphics preset written to the game's config.
 */
graphics: GraphicsPreset, 
/**
 * Windowed rather than full screen.
 */
windowed: boolean, 
/**
 * The launcher's language.
 */
language: Language, 
/**
 * The game's language.
 */
gameLanguage: GameLanguage, 
/**
 * Which runtime build to load.
 */
runtimeProfile: RuntimeProfile, 
/**
 * Hide addresses and names on screen.
 */
streamerMode: boolean, 
/**
 * Show the Host page.
 */
hostTools: boolean, 
/**
 * The game edition.
 */
gameEdition: Game, 
/**
 * Minimising hides the window in the tray.
 */
minimizeToTray: boolean, 
/**
 * Closing hides the window in the tray.
 */
closeToTray: boolean, 
/**
 * Show the game's console window.
 */
showGameConsole: boolean, };
export type SnapshotConnect = { 
/**
 * A check is running.
 */
checking: boolean, 
/**
 * Why the last check failed.
 */
problem: ConnectProblem | null, 
/**
 * Tailscale is installed.
 */
tailscaleInstalled: boolean, 
/**
 * When the server last answered (ISO 8601).
 */
lastCheckedAt: string | null, };
export type SnapshotAccount = { 
/**
 * Its name.
 */
username: string | null, 
/**
 * A key is stored for it.
 */
hasKey: boolean, 
/**
 * The page should offer a key backup.
 */
offerBackup: boolean, };
export type SnapshotRunning = { 
/**
 * The game.
 */
game: boolean, 
/**
 * The local relay's port.
 */
relayPort: number | null, };
export type SnapshotApp = { 
/**
 * Its version.
 */
version: string, 
/**
 * An installed release rather than a development run.
 */
packaged: boolean, 
/**
 * An update is downloaded and waiting.
 */
updateReady: boolean, 
/**
 * `win32` or `linux`.
 */
platform: string, 
/**
 * This build has the Host page.
 */
hostToolsAvailable: boolean, 
/**
 * This build plays 1.4.4.
 */
legacyAvailable: boolean, 
/**
 * Game versions this build plays.
 */
gameVersions: Array<string>, 
/**
 * This build is the server manager.
 */
serverGui: boolean, };
export type Snapshot = { 
/**
 * Where the launcher is.
 */
phase: Phase, 
/**
 * A command is running.
 */
busy: boolean, 
/**
 * The joined server.
 */
server: ServerInfo | null, 
/**
 * The saved servers.
 */
savedServers: Array<SavedServerInfo>, 
/**
 * The connection.
 */
connect: SnapshotConnect, 
/**
 * The account.
 */
account: SnapshotAccount, 
/**
 * The game folder.
 */
game: GameInfo, 
/**
 * The 1.4.4 install.
 */
revived: RevivedInfo | null, 
/**
 * The runtime.
 */
runtime: RuntimeInfo, 
/**
 * The running task.
 */
task: TaskProgress | null, 
/**
 * What is running.
 */
running: SnapshotRunning, 
/**
 * The settings.
 */
settings: Settings, 
/**
 * This launcher.
 */
app: SnapshotApp, 
/**
 * The server's last status.
 */
status: ServerStatus | null, 
/**
 * The server has no status route.
 */
statusUnsupported: boolean, 
/**
 * The last failure, until dismissed.
 */
lastError: LauncherError | null, 
/**
 * The last notice, until dismissed.
 */
notice: NoticeCode | null, };
export type NewsItem = { 
/**
 * When it was posted.
 */
date: string | null, 
/**
 * Its title.
 */
title: string, 
/**
 * Its text.
 */
body: string, };
export type Background = { 
/**
 * Where it is.
 */
url: string, 
/**
 * Who made it.
 */
credit: string | null, };
export type Branding = { 
/**
 * Background pictures.
 */
backgrounds: Array<Background>, 
/**
 * Accent colour.
 */
accent: string | null, };
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
export type EpicView = { 
/**
 * Signed in.
 */
signedIn: boolean, 
/**
 * The Epic account's name.
 */
displayName: string | null, 
/**
 * The sign-in window is open.
 */
signingIn: boolean, };
export type InstallState = "not_installed" | "resumable" | "preparing" | "downloading" | "paused" | "finishing" | "installed" | "failed";
export type InstallInfo = { 
/**
 * The version.
 */
version: string, 
/**
 * Where it stands.
 */
state: InstallState, 
/**
 * Its folder.
 */
folder: string | null, 
/**
 * How far it has got.
 */
progress: TaskProgress | null, 
/**
 * Why it failed.
 */
error: LauncherError | null, };
export type GuardedState = "pending" | "approved" | "running" | "done" | "failed" | "canceled" | "expired";
export type GuardedMode = "delay" | "approval";
export type GuardedActionInfo = { 
/**
 * Its id, 16 lower-case hex digits.
 */
id: string, 
/**
 * What it does.
 */
kind: string, 
/**
 * One line for the list.
 */
summary: string, 
/**
 * Where it stands.
 */
state: GuardedState, 
/**
 * What releases it.
 */
mode: GuardedMode, 
/**
 * Who asked for it.
 */
initiatedBy: string, 
/**
 * The role an approver needs.
 */
needsRole: string, 
/**
 * When it was asked for.
 */
createdAt: string, 
/**
 * When its delay ends.
 */
releaseAt: string | null, 
/**
 * When it lapses.
 */
expiresAt: string, 
/**
 * Who approved it.
 */
approvedBy: string | null, 
/**
 * Who cancelled it.
 */
canceledBy: string | null, 
/**
 * When it finished.
 */
endedAt: string | null, 
/**
 * What it reported.
 */
result: string | null, };
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
export type HistoryEntryView = { 
/**
 * Its id.
 */
id: string, 
/**
 * The database.
 */
database: string, 
/**
 * When (ISO 8601).
 */
at: string, 
/**
 * Who made it.
 */
actor: string, 
/**
 * The table.
 */
table: string, 
/**
 * The row's key.
 */
key: string, 
/**
 * Insert, update or delete.
 */
op: string, 
/**
 * The row before, as JSON.
 */
before: string | null, 
/**
 * The row after, as JSON.
 */
after: string | null, };
export type HistoryPageView = { 
/**
 * The changes, newest first.
 */
entries: Array<HistoryEntryView>, 
/**
 * The cursor of the next page.
 */
next: string | null, };
export type RevertRowView = { 
/**
 * The database.
 */
database: string, 
/**
 * The table.
 */
table: string, 
/**
 * The row's key.
 */
key: string, 
/**
 * Insert, update or delete.
 */
kind: string, 
/**
 * The row now, as JSON.
 */
current: string | null, 
/**
 * The row after the revert, as JSON.
 */
target: string | null, };
export type RevertConflictView = { 
/**
 * The database.
 */
database: string, 
/**
 * The table.
 */
table: string, 
/**
 * The row's key.
 */
key: string, 
/**
 * Why.
 */
reason: string, 
/**
 * The later change in the way.
 */
entry: string | null, 
/**
 * Who made it.
 */
by: string | null, 
/**
 * When.
 */
at: string | null, 
/**
 * More words.
 */
detail: string | null, };
export type RevertPlanView = { 
/**
 * The plan's id, for `history_revert_apply`.
 */
id: string, 
/**
 * Changes selected.
 */
entries: number, 
/**
 * Rows it would write.
 */
actionCount: number, 
/**
 * Rows it would skip.
 */
conflictCount: number, 
/**
 * Rows already as they were.
 */
unchangedCount: number, 
/**
 * The rows it would write (perhaps cut short).
 */
actions: Array<RevertRowView>, 
/**
 * The rows it would skip (perhaps cut short).
 */
conflicts: Array<RevertConflictView>, 
/**
 * The lists were cut short.
 */
truncated: boolean, };
export type RevertedInView = { 
/**
 * The database.
 */
database: string, 
/**
 * Rows changed.
 */
rows: number, };
export type RevertDoneView = { 
/**
 * When (ISO 8601).
 */
at: string | null, 
/**
 * Rows changed.
 */
rowsChanged: number, 
/**
 * Per database.
 */
databases: Array<RevertedInView>, 
/**
 * Rows skipped (perhaps cut short).
 */
conflicts: Array<RevertConflictView>, 
/**
 * Rows skipped.
 */
conflictCount: number, 
/**
 * The list was cut short.
 */
truncated: boolean, };
export type RealmState = "starting" | "ready" | "running" | "ended";
export type RealmSessionView = { 
/**
 * Its id.
 */
session: string, 
/**
 * Hub or hunt.
 */
kind: RealmKind, 
/**
 * This PC hosts it.
 */
hosting: boolean, 
/**
 * Bumped each time it restarts.
 */
generation: number, 
/**
 * Where it stands.
 */
state: RealmState, 
/**
 * The local port the game connects to.
 */
localPort: number | null, 
/**
 * How it ended.
 */
result: string | null, };
export type RealmView = { 
/**
 * The server offers realms.
 */
available: boolean, 
/**
 * The session in use.
 */
active: RealmSessionView | null, 
/**
 * Every known session.
 */
sessions: Array<RealmSessionView>, };
export type DoctorLevel = "ok" | "warning" | "problem";
export type DoctorCheckView = { 
/**
 * How it came out.
 */
level: DoctorLevel, 
/**
 * What was checked.
 */
label: string, 
/**
 * What was found.
 */
detail: string, };
export type DoctorReportView = { 
/**
 * The checks.
 */
checks: Array<DoctorCheckView>, 
/**
 * How many warned.
 */
warnings: number, 
/**
 * How many failed.
 */
problems: number, };
export type MongoMigrationState = "none" | "migrated" | "skipped" | "unmigrated" | "unreachable";
export type MongoMigrationView = { 
/**
 * Where it stands.
 */
state: MongoMigrationState, 
/**
 * The database found.
 */
source: string | null, 
/**
 * More words.
 */
detail: string | null, };
export type ServerConfigView = { 
/**
 * A config exists.
 */
configured: boolean, 
/**
 * The server's name.
 */
name: string, 
/**
 * Private or public.
 */
mode: HostServerMode, 
/**
 * The public address.
 */
publicHost: string | null, 
/**
 * Who may register.
 */
registration: string, 
/**
 * The database engine.
 */
database: string, 
/**
 * Most hubs at once.
 */
maxHubs: number, 
/**
 * Most hunts at once.
 */
maxHunts: number, 
/**
 * The next start wipes the data.
 */
freshStart: boolean, 
/**
 * The MongoDB migration.
 */
mongoMigration: MongoMigrationView, 
/**
 * The config file's path.
 */
configFile: string | null, };
export type ServerInviteEntry = { 
/**
 * Its code.
 */
code: string, 
/**
 * The link to hand out. A secret: never logged.
 */
link: string, 
/**
 * Who it is meant for.
 */
for: string | null, 
/**
 * Uses left.
 */
usesRemaining: number, 
/**
 * It never runs out.
 */
infiniteUses: boolean, 
/**
 * When it was made.
 */
createdAt: string, };
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
export type ManagedInstallView = { 
/**
 * Its node id.
 */
node: string, 
/**
 * Its region.
 */
region: string, 
/**
 * The host's label for it.
 */
label: string | null, 
/**
 * Its state.
 */
state: string, 
/**
 * When it enrolled (Unix ms).
 */
enrolledAtMs: number, 
/**
 * When it last reported (Unix ms).
 */
lastSeenMs: number, 
/**
 * Its access was taken away.
 */
revoked: boolean, 
/**
 * The config revision it runs.
 */
appliedRevision: number, 
/**
 * Players on it.
 */
playersOnline: number, 
/**
 * Hunts running on it.
 */
activeHunts: number, 
/**
 * Its version.
 */
version: string | null, 
/**
 * Commands waiting for it.
 */
queuedCommands: number, };
export type RemoteStatusView = { 
/**
 * Controller, node or neither.
 */
role: string, 
/**
 * Its node id.
 */
node: string | null, 
/**
 * Its region.
 */
region: string | null, 
/**
 * The controller's address.
 */
controllerUrl: string | null, 
/**
 * The controller certificate's SHA-256.
 */
controllerCertFp: string | null, 
/**
 * The controller's node id.
 */
controllerNode: string | null, 
/**
 * The config revision.
 */
revision: number, 
/**
 * Invites not used yet.
 */
pendingEnrollments: number, 
/**
 * Managed installs, for a controller.
 */
installs: Array<ManagedInstallView>, 
/**
 * The node left its controller.
 */
detached: boolean, };
export type RemoteInviteView = { 
/**
 * The URI to give the node. A secret: never logged.
 */
uri: string, 
/**
 * The node id it is for.
 */
node: string, 
/**
 * The node's region.
 */
region: string, 
/**
 * The host's label.
 */
label: string | null, 
/**
 * The controller's address.
 */
controllerUrl: string, 
/**
 * The controller certificate's SHA-256.
 */
controllerFp: string, 
/**
 * When it lapses (Unix ms).
 */
expiresAtMs: number, };
export type RemoteExecView = { 
/**
 * Its id.
 */
commandId: string, 
/**
 * The node.
 */
node: string, 
/**
 * What it asks.
 */
action: string, 
/**
 * Where it stands.
 */
status: string, };
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
