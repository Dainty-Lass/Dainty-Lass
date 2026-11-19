// The page's bridge to the Tauri backend (MIGRATION_PLAN.md RA2): one method per command in
// src-tauri/src/bridge/spec.rs, over Tauri's invoke and events. Each call goes through the isolation hook
// (isolation/hook.js), which drops anything that is not on the backend's list; a dropped call never
// settles, so every call here has a deadline.

import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type {
  CommandName,
  EpicLibraryCheck,
  EpicView,
  GameVersionList,
  InstallOutcome,
} from "./generated/types";
import type {
  ActionResult,
  HostKit,
  HostServerView,
  LauncherApi,
  Settings,
  Snapshot,
  TaskProgress,
} from "./types";

// A call that waits on the player (a file dialog) or on a long task (the executable check, the runtime
// download, a host kit build) gets an hour; any other a minute.
export const CALL_TIMEOUT_MS = 60_000;
export const LONG_TIMEOUT_MS = 3_600_000;

export class BridgeError extends Error {
  readonly command: string;

  constructor(command: string, reason: string) {
    super(`${command}: ${reason}`);
    this.name = "BridgeError";
    this.command = command;
  }
}

// What a failed call answers when the page expects an answer, as the Electron main process did.
const FAILED: ActionResult = { ok: false, error: { code: "unknown" } };

function call<T>(
  command: CommandName,
  args: Record<string, unknown> = {},
  timeoutMs = CALL_TIMEOUT_MS,
): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => reject(new BridgeError(command, "no answer")), timeoutMs);
    invoke<T>(command, args).then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error: unknown) => {
        clearTimeout(timer);
        reject(new BridgeError(command, String(error)));
      },
    );
  });
}

// An action: a failure is reported as the page reports any error.
async function act<T = ActionResult>(
  command: CommandName,
  args?: Record<string, unknown>,
  timeoutMs?: number,
): Promise<T> {
  try {
    return await call<T>(command, args, timeoutMs);
  } catch (error) {
    console.error(error);
    return FAILED as T;
  }
}

// A call with nothing to answer: a failure is only logged.
async function tell(command: CommandName, args?: Record<string, unknown>): Promise<void> {
  try {
    await call<unknown>(command, args);
  } catch (error) {
    console.error(error);
  }
}

function on<T>(event: string, fn: (payload: T) => void): void {
  listen<T>(event, (e) => fn(e.payload)).catch((error: unknown) =>
    console.error(new BridgeError(`listen ${event}`, String(error))),
  );
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
  useGamePath: (gamePath) => act("use_game_path", { gamePath: String(gamePath) }, LONG_TIMEOUT_MS),
  useDetectedGame: (index) => act("use_detected_game", { index: Number(index) }, LONG_TIMEOUT_MS),
  verifyGame: () => act("verify_game", {}, LONG_TIMEOUT_MS),
  // Dauntless 1.4.4 (the legacy edition): refused by a build without it
  revivedInstall: (mode) =>
    act("revived_install", { mode: mode === "repair" ? "repair" : "install" }, LONG_TIMEOUT_MS),
  revivedChooseFolder: () => act("revived_choose_folder", {}, LONG_TIMEOUT_MS),
  revivedUseDetected: (index) =>
    act("revived_use_detected", { index: Number(index) }, LONG_TIMEOUT_MS),
  trustRuntime: () => act("trust_runtime"),
  installRuntime: () => act("install_runtime", {}, LONG_TIMEOUT_MS),
  chooseLocalRuntime: () => act("choose_local_runtime", {}, LONG_TIMEOUT_MS),
  clearLocalRuntime: () => act("clear_local_runtime"),
  cancelTask: () => tell("cancel_task"),
  play: () => act("play"),
  // Only the settings fields go across
  setSettings: (patch: Partial<Settings>) =>
    call<Settings>("set_settings", {
      patch: {
        ...(patch.graphics !== undefined ? { graphics: patch.graphics } : {}),
        ...(patch.windowed !== undefined ? { windowed: patch.windowed } : {}),
        ...(patch.streamerMode !== undefined ? { streamerMode: patch.streamerMode } : {}),
        ...(patch.hostTools !== undefined ? { hostTools: patch.hostTools } : {}),
        ...(patch.language !== undefined ? { language: patch.language } : {}),
        ...(patch.gameLanguage !== undefined ? { gameLanguage: patch.gameLanguage } : {}),
        ...(patch.runtimeProfile !== undefined ? { runtimeProfile: patch.runtimeProfile } : {}),
        ...(patch.gameEdition !== undefined ? { gameEdition: patch.gameEdition } : {}),
        ...(patch.minimizeToTray !== undefined ? { minimizeToTray: patch.minimizeToTray } : {}),
        ...(patch.closeToTray !== undefined ? { closeToTray: patch.closeToTray } : {}),
        ...(patch.showGameConsole !== undefined ? { showGameConsole: patch.showGameConsole } : {}),
        ...(patch.injectHelper !== undefined ? { injectHelper: patch.injectHelper } : {}),
      },
    }),
  setStatusPolling: (on) => tell("set_status_polling", { on: on === true }),
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
  hostAction: (action) => act("host_action", { action: String(action) }, LONG_TIMEOUT_MS),
  hostOpen: (folder) => act("host_open", { folder: String(folder) }),
  hostServerGet: () => call<HostServerView>("host_server_get"),
  hostServerStart: () => act("host_server_start", {}, LONG_TIMEOUT_MS),
  hostServerStop: () => act("host_server_stop", {}, LONG_TIMEOUT_MS),
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
      LONG_TIMEOUT_MS,
    ),
  hostCreateInvite: (forName) =>
    act(
      "host_create_invite",
      { forName: forName === null ? null : String(forName) },
      LONG_TIMEOUT_MS,
    ),
  hostUpdate: () => act("host_update", {}, LONG_TIMEOUT_MS),
  hostSetMultipliers: (m) =>
    act(
      "host_set_multipliers",
      { multipliers: m === null ? null : { xp: Number(m.xp), rewards: Number(m.rewards) } },
      LONG_TIMEOUT_MS,
    ),
  copyText: async (text) => {
    try {
      return (await call<boolean>("copy_text", { text: String(text) })) === true;
    } catch (error) {
      console.error(error);
      return false;
    }
  },
  dismissNotice: () => tell("dismiss_notice"),
  dismissError: () => tell("dismiss_error"),
  minimize: () => getCurrentWindow().minimize(),
  toggleMaximize: async () => {
    const window = getCurrentWindow();
    await window.toggleMaximize();
    return window.isMaximized();
  },
  close: () => getCurrentWindow().close(),
  takeInviteLink: () => call<string | null>("take_invite_link"),
  gameLogGet: () => call<string[]>("game_log_get"),
  // The sign-in window waits on the player
  epicSignInStart: () => act("epic_sign_in_start", {}, LONG_TIMEOUT_MS),
  epicSignInCode: (code) => act("epic_sign_in_code", { code: String(code) }),
  epicGet: () => call<EpicView>("epic_get"),
  epicSignOut: () => act("epic_sign_out"),
  epicLibraryCheck: () => act<EpicLibraryCheck>("epic_library_check"),
  // Each Epic version's manifest is fetched for its sizes
  gameVersionList: () => act<GameVersionList>("game_version_list", {}, LONG_TIMEOUT_MS),
  // Refreshes the Epic session and checks the library before it answers
  gameInstallStart: (version) =>
    act<InstallOutcome>("game_install_start", { version: String(version) }, LONG_TIMEOUT_MS),
  gameInstallPause: (version) => act("game_install_pause", { version: String(version) }),
  gameInstallResume: (version) => act("game_install_resume", { version: String(version) }),
  gameInstallCancel: (version) => act("game_install_cancel", { version: String(version) }),
  gameInstallGet: (version) =>
    act<InstallOutcome>("game_install_get", { version: String(version) }),
  gameInstallChooseFolder: () => act("game_install_choose_folder", {}, LONG_TIMEOUT_MS),
  // Waits for the download to stop, then deletes its files
  gameInstallDiscard: (version) =>
    act("game_install_discard", { version: String(version) }, LONG_TIMEOUT_MS),
  getPlaytime: () => call<number | null>("get_playtime"),
  onSnapshot: (fn) => on<Snapshot>("snapshot", fn),
  // Progress comes over a channel of its own, already throttled by the backend
  onProgress: (fn) => {
    const channel = new Channel<TaskProgress>();
    channel.onmessage = fn;
    void tell("subscribe_progress", { onProgress: channel });
  },
  onHostKit: (fn) => on<HostKit>("host-kit", fn),
  onHostServer: (fn) => on<HostServerView>("host-server", fn),
  onInviteLink: (fn) => on<unknown>("invite-link", () => fn()),
  onWindowState: (fn) => on<unknown>("window-state", (maximized) => fn(maximized === true)),
  onGameLog: (fn) =>
    on<unknown>("game-log", (lines) =>
      fn(Array.isArray(lines) ? lines.filter((line) => typeof line === "string") : []),
    ),
};
