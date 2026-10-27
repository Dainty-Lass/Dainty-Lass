// One method per command in DaintyLass's rust/launcher/src-tauri/src/bridge/spec.rs, plus the window calls and
// the events: the stand-in page's api.ts, moved here (DU2). DaintyLass's rust/launcher/test/api.test.ts drives
// this file against Tauri's IPC mocks and its isolation hook, and checks that every command has a method.
import { Channel } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { getCurrentWindow } from "@tauri-apps/api/window"

import { BridgeError, call, CALL_TIMEOUT_MS, LONG_TIMEOUT_MS } from "./call"
import type {
  ActionResult,
  Branding,
  CommandName,
  EpicLibraryCheck,
  EpicView,
  ExternalTarget,
  GameVersionList,
  GuardedList,
  GuardedOutcome,
  HostAction,
  HostFolder,
  HostKit,
  HostMultipliers,
  HostServerView,
  HostSetupRequest,
  InstallMode,
  InstallOutcome,
  InviteCheck,
  ModuleCommandOutcome,
  ModuleListOutcome,
  ModuleOutcome,
  NewsItem,
  RealmKind,
  RegionPing,
  RegisterOutcome,
  Settings,
  Snapshot,
  TaskProgress,
} from "./generated/types"

export { BridgeError, CALL_TIMEOUT_MS, LONG_TIMEOUT_MS }

// Who and when a history query or a revert covers (DD23): an account, an actor, RFC 3339 times
export interface RevertSelection {
  account?: string | null
  actor?: string | null
  since?: string | null
  until?: string | null
}

export interface HistoryFilter extends RevertSelection {
  cursor?: string | null
}

// A module command's form (DD11): each argument the manifest names, as text. The backend checks it against the
// command's registered arguments
export type ModuleInput = Record<string, string>

// `[modules.<id>.<section>]` values to save: section, key, then a value of the type of the key's default
export type ModuleSettingsPatch = Record<
  string,
  Record<string, boolean | number | string>
>

export interface LauncherApi {
  getSnapshot(): Promise<Snapshot>
  checkInvite(text: string): Promise<InviteCheck>
  submitInvite(
    text: string,
    acceptNewCertificate?: boolean,
    carryAccount?: boolean
  ): Promise<ActionResult>
  retryConnect(): Promise<ActionResult>
  forgetServer(): Promise<ActionResult>
  switchServer(id: string): Promise<ActionResult>
  removeSavedServer(id: string): Promise<ActionResult>
  register(username: string): Promise<RegisterOutcome>
  useExistingKey(text: string): Promise<ActionResult>
  importKeyFile(): Promise<ActionResult>
  saveKeyBackup(): Promise<ActionResult>
  dismissBackupOffer(): Promise<void>
  chooseGameFolder(): Promise<ActionResult>
  useGamePath(gamePath: string): Promise<ActionResult>
  useDetectedGame(index: number): Promise<ActionResult>
  verifyGame(): Promise<ActionResult>
  revivedInstall(mode: InstallMode): Promise<ActionResult>
  revivedChooseFolder(): Promise<ActionResult>
  revivedUseDetected(index: number): Promise<ActionResult>
  trustRuntime(): Promise<ActionResult>
  installRuntime(): Promise<ActionResult>
  chooseLocalRuntime(): Promise<ActionResult>
  clearLocalRuntime(): Promise<ActionResult>
  cancelTask(): Promise<void>
  play(): Promise<ActionResult>
  setSettings(patch: Partial<Settings>): Promise<Settings>
  setStatusPolling(on: boolean): Promise<void>
  refreshStatus(): Promise<void>
  getNews(): Promise<NewsItem[]>
  getBranding(): Promise<Branding>
  openExternal(target: ExternalTarget): Promise<ActionResult>
  openGameFolder(): Promise<void>
  openLogsFolder(): Promise<void>
  logout(): Promise<ActionResult>
  installUpdate(): Promise<void>
  hostGet(): Promise<HostKit>
  hostChooseWorkspace(): Promise<ActionResult>
  hostChooseDumper(): Promise<ActionResult>
  hostImportGameData(): Promise<ActionResult>
  hostImportSdkFolder(): Promise<ActionResult>
  hostDownloadProject(): Promise<ActionResult>
  hostAction(action: HostAction): Promise<ActionResult>
  hostOpen(folder: HostFolder): Promise<ActionResult>
  hostServerGet(): Promise<HostServerView>
  hostServerStart(): Promise<ActionResult>
  hostServerStop(): Promise<ActionResult>
  hostServerRestart(name: string): Promise<ActionResult>
  hostSetup(request: HostSetupRequest): Promise<ActionResult>
  hostCreateInvite(forName: string | null): Promise<ActionResult>
  hostUpdate(): Promise<ActionResult>
  hostSetMultipliers(multipliers: HostMultipliers | null): Promise<ActionResult>
  copyText(text: string): Promise<boolean>
  dismissNotice(): Promise<void>
  dismissError(): Promise<void>
  minimize(): Promise<void>
  toggleMaximize(): Promise<boolean>
  close(): Promise<void>
  takeInviteLink(): Promise<string | null>
  onSnapshot(fn: (snapshot: Snapshot) => void): void
  onProgress(fn: (progress: TaskProgress) => void): void
  onHostKit(fn: (kit: HostKit) => void): void
  onHostServer(fn: (view: HostServerView) => void): void
  onInviteLink(fn: () => void): void
  onWindowState(fn: (maximized: boolean) => void): void
  // Registered ahead of the rows that fill them (DF8): each answers not_implemented for now
  epicSignInStart(): Promise<ActionResult>
  epicSignInCode(code: string): Promise<ActionResult>
  epicGet(): Promise<EpicView>
  epicSignOut(): Promise<ActionResult>
  epicLibraryCheck(): Promise<EpicLibraryCheck>
  gameVersionList(): Promise<GameVersionList>
  // DE5's, filled: progress comes over onProgress (kind "install")
  gameInstallStart(version: string): Promise<InstallOutcome>
  gameInstallPause(version: string): Promise<ActionResult>
  gameInstallResume(version: string): Promise<ActionResult>
  gameInstallCancel(version: string): Promise<ActionResult>
  gameInstallGet(version: string): Promise<InstallOutcome>
  // DN7's, filled: the joined server's regions, nearest first
  regionPing(): Promise<RegionPing>
  // DU3's, filled (server GUI): modules by manifest id, never a path
  moduleList(): Promise<ModuleListOutcome>
  moduleGet(module: string): Promise<ModuleOutcome>
  moduleRunCommand(
    module: string,
    command: string,
    input: ModuleInput | null
  ): Promise<ModuleCommandOutcome>
  moduleSetSettings(
    module: string,
    values: ModuleSettingsPatch
  ): Promise<ModuleOutcome>
  moduleLoad(module: string): Promise<ModuleOutcome>
  moduleUnload(module: string): Promise<ModuleOutcome>
  realmHost(kind: RealmKind): Promise<ActionResult>
  realmJoin(realm: string): Promise<ActionResult>
  realmStop(): Promise<ActionResult>
  realmGet(): Promise<ActionResult>
  historyQuery(filter: HistoryFilter): Promise<ActionResult>
  historyRevertDryRun(selection: RevertSelection): Promise<ActionResult>
  historyRevertApply(plan: string): Promise<ActionResult>
  guardedList(): Promise<GuardedList>
  guardedApprove(id: string): Promise<GuardedOutcome>
  guardedCancel(id: string): Promise<GuardedOutcome>
}

// What a failed call answers when the page expects an answer, as the Electron main process did
const FAILED: ActionResult = { ok: false, error: { code: "unknown" } }

// An action: a failure is reported as the page reports any error
async function act<T = ActionResult>(
  command: CommandName,
  args?: Record<string, unknown>,
  timeoutMs?: number
): Promise<T> {
  try {
    return await call<T>(command, args, timeoutMs)
  } catch (error) {
    console.error(error)
    return FAILED as T
  }
}

// A call with nothing to answer: a failure is only logged
async function tell(
  command: CommandName,
  args?: Record<string, unknown>
): Promise<void> {
  try {
    await call<unknown>(command, args)
  } catch (error) {
    console.error(error)
  }
}

function optionalText(value: string | null | undefined): string | null {
  return value === undefined || value === null ? null : String(value)
}

function selection(value: RevertSelection): RevertSelection {
  return {
    account: optionalText(value.account),
    actor: optionalText(value.actor),
    since: optionalText(value.since),
    until: optionalText(value.until),
  }
}

function on<T>(event: string, fn: (payload: T) => void): void {
  listen<T>(event, (e) => fn(e.payload)).catch((error: unknown) =>
    console.error(new BridgeError(`listen ${event}`, String(error)))
  )
}

// Only the settings fields go across; the isolation hook refuses any other
function settingsPatch(patch: Partial<Settings>): Partial<Settings> {
  const keys = [
    "graphics",
    "windowed",
    "streamerMode",
    "hostTools",
    "language",
    "gameLanguage",
    "runtimeProfile",
    "gameEdition",
  ] as const satisfies readonly (keyof Settings)[]
  const out: Record<string, unknown> = {}
  for (const key of keys) if (patch[key] !== undefined) out[key] = patch[key]
  return out as Partial<Settings>
}

export const api: LauncherApi = {
  getSnapshot: () => call<Snapshot>("get_snapshot"),
  checkInvite: (text) => act("check_invite", { text: String(text) }),
  submitInvite: (text, acceptNewCertificate = false, carryAccount = false) =>
    act("submit_invite", {
      text: String(text),
      acceptNewCertificate: acceptNewCertificate === true,
      carryAccount: carryAccount === true,
    }),
  retryConnect: () => act("retry_connect"),
  forgetServer: () => act("forget_server"),
  switchServer: (id) => act("switch_server", { id: String(id) }),
  removeSavedServer: (id) => act("remove_saved_server", { id: String(id) }),
  register: (username) => act("register", { username: String(username) }),
  useExistingKey: (text) => act("use_existing_key", { text: String(text) }),
  importKeyFile: () => act("import_key_file", {}, LONG_TIMEOUT_MS),
  saveKeyBackup: () => act("save_key_backup", {}, LONG_TIMEOUT_MS),
  dismissBackupOffer: () => tell("dismiss_backup_offer"),
  chooseGameFolder: () => act("choose_game_folder", {}, LONG_TIMEOUT_MS),
  useGamePath: (gamePath) =>
    act("use_game_path", { gamePath: String(gamePath) }, LONG_TIMEOUT_MS),
  useDetectedGame: (index) =>
    act("use_detected_game", { index: Number(index) }, LONG_TIMEOUT_MS),
  verifyGame: () => act("verify_game", {}, LONG_TIMEOUT_MS),
  // Dauntless 1.4.4 (the legacy edition): refused by a build without it
  revivedInstall: (mode) =>
    act(
      "revived_install",
      { mode: mode === "repair" ? "repair" : "install" },
      LONG_TIMEOUT_MS
    ),
  revivedChooseFolder: () => act("revived_choose_folder", {}, LONG_TIMEOUT_MS),
  revivedUseDetected: (index) =>
    act("revived_use_detected", { index: Number(index) }, LONG_TIMEOUT_MS),
  trustRuntime: () => act("trust_runtime"),
  installRuntime: () => act("install_runtime", {}, LONG_TIMEOUT_MS),
  chooseLocalRuntime: () => act("choose_local_runtime", {}, LONG_TIMEOUT_MS),
  clearLocalRuntime: () => act("clear_local_runtime"),
  cancelTask: () => tell("cancel_task"),
  play: () => act("play"),
  setSettings: (patch) =>
    call<Settings>("set_settings", { patch: settingsPatch(patch) }),
  setStatusPolling: (enabled) =>
    tell("set_status_polling", { on: enabled === true }),
  refreshStatus: () => tell("refresh_status"),
  getNews: () => call("get_news"),
  getBranding: () => call("get_branding"),
  openExternal: (target) => act("open_external", { target: String(target) }),
  openGameFolder: () => tell("open_game_folder"),
  openLogsFolder: () => tell("open_logs_folder"),
  logout: () => act("logout"),
  installUpdate: () => tell("install_update"),
  hostGet: () => call<HostKit>("host_get", {}, LONG_TIMEOUT_MS),
  hostChooseWorkspace: () => act("host_choose_workspace", {}, LONG_TIMEOUT_MS),
  hostChooseDumper: () => act("host_choose_dumper", {}, LONG_TIMEOUT_MS),
  hostImportGameData: () => act("host_import_gamedata", {}, LONG_TIMEOUT_MS),
  hostImportSdkFolder: () => act("host_import_sdk_folder", {}, LONG_TIMEOUT_MS),
  hostDownloadProject: () => act("host_download_project", {}, LONG_TIMEOUT_MS),
  hostAction: (action) =>
    act("host_action", { action: String(action) }, LONG_TIMEOUT_MS),
  hostOpen: (folder) => act("host_open", { folder: String(folder) }),
  hostServerGet: () => call<HostServerView>("host_server_get"),
  hostServerStart: () => act("host_server_start", {}, LONG_TIMEOUT_MS),
  hostServerStop: () => act("host_server_stop", {}, LONG_TIMEOUT_MS),
  hostServerRestart: (name) =>
    act("host_server_restart", { name: String(name) }, LONG_TIMEOUT_MS),
  hostSetup: (req) =>
    act(
      "host_setup",
      {
        request: {
          name: String(req.name),
          mode: String(req.mode),
          publicHost: req.publicHost === null ? null : String(req.publicHost),
        },
      },
      LONG_TIMEOUT_MS
    ),
  hostCreateInvite: (forName) =>
    act(
      "host_create_invite",
      { forName: forName === null ? null : String(forName) },
      LONG_TIMEOUT_MS
    ),
  hostUpdate: () => act("host_update", {}, LONG_TIMEOUT_MS),
  hostSetMultipliers: (m) =>
    act(
      "host_set_multipliers",
      {
        multipliers:
          m === null ? null : { xp: Number(m.xp), rewards: Number(m.rewards) },
      },
      LONG_TIMEOUT_MS
    ),
  copyText: async (text) => {
    try {
      return (await call<boolean>("copy_text", { text: String(text) })) === true
    } catch (error) {
      console.error(error)
      return false
    }
  },
  dismissNotice: () => tell("dismiss_notice"),
  dismissError: () => tell("dismiss_error"),
  minimize: () => getCurrentWindow().minimize(),
  toggleMaximize: async () => {
    const window = getCurrentWindow()
    await window.toggleMaximize()
    return window.isMaximized()
  },
  close: () => getCurrentWindow().close(),
  takeInviteLink: () => call<string | null>("take_invite_link"),
  onSnapshot: (fn) => on<Snapshot>("snapshot", fn),
  // Progress comes over a channel of its own, already throttled by the backend
  onProgress: (fn) => {
    const channel = new Channel<TaskProgress>()
    channel.onmessage = fn
    void tell("subscribe_progress", { onProgress: channel })
  },
  onHostKit: (fn) => on<HostKit>("host-kit", fn),
  onHostServer: (fn) => on<HostServerView>("host-server", fn),
  onInviteLink: (fn) => on<unknown>("invite-link", () => fn()),
  onWindowState: (fn) =>
    on<unknown>("window-state", (maximized) => fn(maximized === true)),
  // The sign-in window waits on the player
  epicSignInStart: () => act("epic_sign_in_start", {}, LONG_TIMEOUT_MS),
  epicSignInCode: (code) => act("epic_sign_in_code", { code: String(code) }),
  epicGet: () => call<EpicView>("epic_get"),
  epicSignOut: () => act("epic_sign_out"),
  epicLibraryCheck: () => act<EpicLibraryCheck>("epic_library_check"),
  gameVersionList: () => act<GameVersionList>("game_version_list"),
  // Refreshes the Epic session and checks the library before it answers
  gameInstallStart: (version) =>
    act<InstallOutcome>(
      "game_install_start",
      { version: String(version) },
      LONG_TIMEOUT_MS
    ),
  gameInstallPause: (version) =>
    act("game_install_pause", { version: String(version) }),
  gameInstallResume: (version) =>
    act("game_install_resume", { version: String(version) }),
  gameInstallCancel: (version) =>
    act("game_install_cancel", { version: String(version) }),
  gameInstallGet: (version) =>
    act<InstallOutcome>("game_install_get", { version: String(version) }),
  regionPing: () => act<RegionPing>("region_ping"),
  moduleList: () => act<ModuleListOutcome>("module_list"),
  moduleGet: (module) =>
    act<ModuleOutcome>("module_get", { module: String(module) }),
  // The input's shape is the module manifest's, so it crosses as JSON text the backend checks
  moduleRunCommand: (module, command, input) =>
    act<ModuleCommandOutcome>(
      "module_run_command",
      {
        module: String(module),
        command: String(command),
        input: JSON.stringify(input ?? null),
      },
      LONG_TIMEOUT_MS
    ),
  moduleSetSettings: (module, values) =>
    act<ModuleOutcome>("module_set_settings", {
      module: String(module),
      values: JSON.stringify(values ?? {}),
    }),
  moduleLoad: (module) =>
    act<ModuleOutcome>(
      "module_load",
      { module: String(module) },
      LONG_TIMEOUT_MS
    ),
  moduleUnload: (module) =>
    act<ModuleOutcome>(
      "module_unload",
      { module: String(module) },
      LONG_TIMEOUT_MS
    ),
  realmHost: (kind) =>
    act(
      "realm_host",
      { kind: kind === "hunt" ? "hunt" : "hub" },
      LONG_TIMEOUT_MS
    ),
  realmJoin: (realm) =>
    act("realm_join", { realm: String(realm) }, LONG_TIMEOUT_MS),
  realmStop: () => act("realm_stop", {}, LONG_TIMEOUT_MS),
  realmGet: () => act("realm_get"),
  historyQuery: (filter) =>
    act("history_query", {
      filter: { ...selection(filter), cursor: optionalText(filter.cursor) },
    }),
  historyRevertDryRun: (value) =>
    act(
      "history_revert_dry_run",
      { selection: selection(value) },
      LONG_TIMEOUT_MS
    ),
  historyRevertApply: (plan) =>
    act("history_revert_apply", { plan: String(plan) }, LONG_TIMEOUT_MS),
  guardedList: () => act<GuardedList>("guarded_list"),
  guardedApprove: (id) =>
    act<GuardedOutcome>("guarded_approve", { id: String(id) }),
  guardedCancel: (id) =>
    act<GuardedOutcome>("guarded_cancel", { id: String(id) }),
}
