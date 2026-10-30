// Generated from the launcher backend (rust/launcher/src-tauri/src/bridge); do not edit.
// Regenerate with: RR_UPDATE_GENERATED=1 cargo test -p launcher generated

export type ServerMode = "private" | "public";
export type PlayerWhere = "menu" | "city" | "hunt" | "dojo" | "tutorial" | "unknown";
export type InstanceKind = "city" | "hunt" | "dojo" | "tutorial";
export type RegistrationMode = "OPEN" | "INVITECODE" | "NONE";
export type StatusPlayer = { name: string, where: PlayerWhere, instance: string | null, };
export type StatusInstance = { id: string, kind: InstanceKind, title: string, map: string, behemoth: string | null, players: number, maxPlayers: number, startedAt: string | null, };
export type GameDataKind = "real" | "synthetic" | "missing" | "unknown";
export type StatusGame = { build: string, changelist: number | null, };
export type ServerStatus = { name: string, online: boolean, version: string, commit: string, sourceUrl: string | null, registration: RegistrationMode | null, playersOnline: number, players: Array<StatusPlayer>, instances: Array<StatusInstance>, uptimeSeconds: number, game: StatusGame | null, gameData: GameDataKind, chat: boolean, contentPort: number | null, limited: boolean, };
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
export type LauncherError = { code: ErrorCode, detail?: string, };
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
export type RuntimeFileInfo = { name: string, size: number, sha256: string, };
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
gameEdition: Game, };
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
 * False without the `legacy` feature: the page offers no edition setting
 */
legacyAvailable: boolean, 
/**
 * The game versions an invite may name in this build (`protocol::game::supported_versions`)
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
export type HostUpstream = { present: boolean, patched: boolean, };
export type HostSdk = { present: boolean, functionFiles: number, };
export type HostDump = { dir: string, name: string, modified: string, };
export type HostBuilds = { runtime: boolean, loader: boolean, exporter: boolean, published: boolean, };
export type HostToolchain = { visualStudio: string | null, msbuild: boolean, cargo: boolean, };
export type HostGameData = { backend: GameDataState, director: GameDataState, };
export type HostExports = { present: boolean, inGame: boolean, };
export type HostServerMode = "private" | "public";
export type HostServerConfig = { configured: boolean, name: string | null, mode: HostServerMode | null, host: string | null, multipliers: HostMultipliers | null, };
export type HostKit = { workspace: string | null, workspaceOk: boolean, upstream: HostUpstream, sdk: HostSdk, dumps: Array<HostDump>, dumperDll: string | null, builds: HostBuilds, toolchain: HostToolchain, gameData: HostGameData, exports: HostExports, serverConfig: HostServerConfig, requirements: Array<Requirement>, job: HostJob | null, update: HostUpdateView | null, projectTarget: string | null, };
export type HostMultipliers = { xp: number, rewards: number, };
export type HostUpdateState = "up_to_date" | "updated";
export type HostUpdateView = { state: HostUpdateState, head: string, files: number, runtimeChanged: boolean, };
export type RequirementId = "dauntless" | "git" | "node" | "mongodb" | "tailscale" | "visual_studio" | "rust" | "dumper7" | "vc_redist";
export type RequirementState = "ok" | "missing" | "problem";
export type Requirement = { id: RequirementId, state: RequirementState, detail: string | null, problem: string | null, };
export type HostServerState = "stopped" | "starting" | "running" | "stopping" | "external" | "failed";
export type HostInvite = { link: string, for: string | null, };
export type HostServerView = { state: HostServerState, message: string | null, lines: Array<string>, invite: HostInvite | null, embedded: boolean, components: Array<HostServerComponent>, };
export type HostServerComponent = { name: string, phase: HostComponentPhase, port: number | null, detail: string, };
export type HostComponentPhase = "stopped" | "starting" | "ready" | "stopping" | "failed" | "disabled";
export type HostJobKind = "inject" | "build_runtime" | "build_loader" | "build_exporter" | "import_sdk" | "copy_exports" | "synthetic_data" | "import_gamedata" | "publish_runtime" | "setup" | "generate_gamedata" | "export_gamedata" | "create_invite" | "update" | "multipliers" | "upstream" | "download_project" | "backup" | "migrate_mongo";
export type HostJobState = "running" | "done" | "failed";
export type HostJob = { kind: HostJobKind, state: HostJobState, message: string | null, logFile: string | null, };
export type ExternalTarget = "tailscale_download" | "tailscale_share" | "vc_redist" | "server_source" | "project_source" | "project_license" | "project_issues" | "project_docs_players" | "project_docs_hosting" | "project_docs_sdk" | "installing_guide" | "legendary" | "dumper7" | "visual_studio" | "rust" | "nodejs" | "mongodb" | "git" | "visual_studio_installer" | "rust_installer" | "dumper7_zip" | "revived_source" | "mystic_source" | "undaunted_source";
export type HostAction = "inject_dumper" | "import_sdk" | "build_runtime" | "build_loader" | "build_exporter" | "inject_exporter" | "copy_exports" | "synthetic_data" | "publish_runtime" | "generate_gamedata" | "export_gamedata";
export type HostFolder = "workspace" | "runtime_source" | "dumps" | "game_data" | "runtime_publish" | "host_logs" | "gamedata_packs";
export type HostSetupRequest = { name: string, mode: HostServerMode, publicHost: string | null, };
export type RealmKind = "hub" | "hunt";
export type RegionProblem = "timeout" | "unreachable" | "cert_mismatch" | "bad_answer";
export type RegionRtt = { region: string, rttMs: number | null, samples: number, problem: RegionProblem | null, };
export type GameVersionInstall = "epic" | "locate";
export type GameVersionOrigin = "launcher" | "server";
export type EpicManifestState = "ready" | "unreachable" | "invalid";
export type EpicVersionInfo = { buildVersion: string, manifest: EpicManifestState, downloadBytes: number | null, installBytes: number | null, };
export type GameVersionEntry = { version: string, displayName: string, origin: GameVersionOrigin, install: GameVersionInstall, epic: EpicVersionInfo | null, };
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
export type ModulePortView = { name: string, transport: string, first: number, last: number, };
export type ModuleArgView = { name: string, required: boolean, help: string, };
export type ModuleCommandView = { name: string, summary: string, args: Array<ModuleArgView>, };
export type ModuleSettingValue = boolean | number | string;
export type ModuleSettingView = { name: string, help: string, default: ModuleSettingValue, value: ModuleSettingValue, };
export type ModuleSectionView = { name: string, keys: Array<ModuleSettingView>, };
export type ModulePatchSetView = { provides: boolean, displayName: string | null, };
export type ModulePaletteEntry = { name: string, value: string, };
export type ModuleThemeView = { id: string, displayName: string, palette: Array<ModulePaletteEntry>, layout: string | null, logo: string | null, };
export type ModuleCapabilitiesView = { realms: boolean, network: boolean, chat: boolean, patchSet: ModulePatchSetView | null, theme: ModuleThemeView | null, };
export type ModuleView = { id: string, displayName: string | null, role: ModuleRole | null, gameVersion: string | null, origin: ModuleOrigin, autoload: boolean, state: ModuleRunState, detail: string | null, unverified: boolean, ports: Array<ModulePortView>, capabilities: ModuleCapabilitiesView, commands: Array<ModuleCommandView>, settings: Array<ModuleSectionView>, };
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
export type AdminPlayerView = { globalId: string, displayName: string, moduleUserId: string, online: boolean, banned: boolean, banReason: string | null, timeoutUntil: string | null, createdAt: string | null, lastLoginAt: string | null, };
export type AdminResultView = { action: AdminActionKind, ok: boolean, message: string | null, player: AdminPlayerView | null, };
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
export type CommandName = "get_snapshot" | "check_invite" | "submit_invite" | "retry_connect" | "forget_server" | "switch_server" | "remove_saved_server" | "register" | "use_existing_key" | "import_key_file" | "save_key_backup" | "dismiss_backup_offer" | "choose_game_folder" | "use_game_path" | "use_detected_game" | "verify_game" | "revived_install" | "revived_choose_folder" | "revived_use_detected" | "trust_runtime" | "install_runtime" | "choose_local_runtime" | "clear_local_runtime" | "cancel_task" | "play" | "set_settings" | "set_status_polling" | "refresh_status" | "get_news" | "get_branding" | "open_external" | "open_game_folder" | "open_logs_folder" | "logout" | "install_update" | "host_get" | "host_choose_workspace" | "host_choose_dumper" | "host_import_gamedata" | "host_import_sdk_folder" | "host_download_project" | "host_action" | "host_open" | "host_server_get" | "host_server_start" | "host_server_stop" | "host_server_restart" | "host_setup" | "host_create_invite" | "host_update" | "host_set_multipliers" | "copy_text" | "take_invite_link" | "dismiss_notice" | "dismiss_error" | "subscribe_progress" | "epic_sign_in_start" | "epic_sign_in_code" | "epic_get" | "epic_sign_out" | "epic_library_check" | "game_version_list" | "game_install_start" | "game_install_pause" | "game_install_resume" | "game_install_cancel" | "game_install_get" | "region_ping" | "module_list" | "module_get" | "module_run_command" | "module_set_settings" | "module_load" | "module_unload" | "realm_host" | "realm_join" | "realm_stop" | "realm_get" | "history_query" | "history_revert_dry_run" | "history_revert_apply" | "guarded_list" | "guarded_approve" | "guarded_cancel" | "server_doctor" | "server_backup" | "server_config_get" | "server_config_save" | "server_migrate_mongo" | "server_confirm_fresh_start" | "server_invite_list" | "admin_action";
