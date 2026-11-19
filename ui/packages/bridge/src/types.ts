// The bridge's surface as the page sees it: one method per command in src-tauri/src/bridge/spec.rs, plus the
// window calls and the events. The value types come from generated/types.ts (ts-rs, RR_UPDATE_GENERATED=1).

import type {
  ActionResult,
  Branding,
  EpicLibraryCheck,
  EpicView,
  GameVersionList,
  HostAction,
  HostFolder,
  HostKit,
  HostMultipliers,
  HostServerView,
  HostSetupRequest,
  InstallMode,
  InstallOutcome,
  InviteCheck,
  NewsItem,
  RegisterOutcome,
  Settings,
  Snapshot,
  TaskProgress,
  UninstallOutcome,
} from "./generated/types";

export type { ActionResult, HostKit, HostServerView, Settings, Snapshot, TaskProgress };

export interface LauncherApi {
  getSnapshot(): Promise<Snapshot>;
  checkInvite(text: string): Promise<InviteCheck>;
  submitInvite(
    text: string,
    acceptNewCertificate?: boolean,
    carryAccount?: boolean,
  ): Promise<ActionResult>;
  retryConnect(): Promise<ActionResult>;
  forgetServer(): Promise<ActionResult>;
  switchServer(id: string): Promise<ActionResult>;
  removeSavedServer(id: string): Promise<ActionResult>;
  register(username: string): Promise<RegisterOutcome>;
  useExistingKey(text: string): Promise<ActionResult>;
  importKeyFile(): Promise<ActionResult>;
  saveKeyBackup(): Promise<ActionResult>;
  dismissBackupOffer(): Promise<void>;
  chooseGameFolder(): Promise<ActionResult>;
  useGamePath(gamePath: string): Promise<ActionResult>;
  useDetectedGame(index: number): Promise<ActionResult>;
  verifyGame(): Promise<ActionResult>;
  revivedInstall(mode: InstallMode): Promise<ActionResult>;
  revivedChooseFolder(): Promise<ActionResult>;
  revivedUseDetected(index: number): Promise<ActionResult>;
  trustRuntime(): Promise<ActionResult>;
  installRuntime(): Promise<ActionResult>;
  chooseLocalRuntime(): Promise<ActionResult>;
  clearLocalRuntime(): Promise<ActionResult>;
  cancelTask(): Promise<void>;
  /** PLAY waiting in the server's login queue gives up its place. */
  leaveQueue(): Promise<void>;
  play(): Promise<ActionResult>;
  setSettings(patch: Partial<Settings>): Promise<Settings>;
  setStatusPolling(on: boolean): Promise<void>;
  refreshStatus(): Promise<void>;
  getNews(): Promise<NewsItem[]>;
  getBranding(): Promise<Branding>;
  openExternal(target: string): Promise<ActionResult>;
  openGameFolder(): Promise<void>;
  openLogsFolder(): Promise<void>;
  logout(): Promise<ActionResult>;
  installUpdate(): Promise<void>;
  hostGet(): Promise<HostKit>;
  hostChooseWorkspace(): Promise<ActionResult>;
  hostChooseDumper(): Promise<ActionResult>;
  hostImportGameData(): Promise<ActionResult>;
  hostImportSdkFolder(): Promise<ActionResult>;
  hostDownloadProject(): Promise<ActionResult>;
  hostAction(action: HostAction): Promise<ActionResult>;
  hostOpen(folder: HostFolder): Promise<ActionResult>;
  hostServerGet(): Promise<HostServerView>;
  hostServerStart(): Promise<ActionResult>;
  hostServerStop(): Promise<ActionResult>;
  hostSetup(request: HostSetupRequest): Promise<ActionResult>;
  hostCreateInvite(forName: string | null): Promise<ActionResult>;
  hostUpdate(): Promise<ActionResult>;
  hostSetMultipliers(multipliers: HostMultipliers | null): Promise<ActionResult>;
  copyText(text: string): Promise<boolean>;
  dismissNotice(): Promise<void>;
  dismissError(): Promise<void>;
  minimize(): Promise<void>;
  toggleMaximize(): Promise<boolean>;
  close(): Promise<void>;
  takeInviteLink(): Promise<string | null>;
  /** The game console's lines so far this game, the account key masked. */
  gameLogGet(): Promise<string[]>;
  /** Answers once the sign-in window closes. The Epic session stays in the keystore (DE2). */
  epicSignInStart(): Promise<ActionResult>;
  epicSignInCode(code: string): Promise<ActionResult>;
  epicGet(): Promise<EpicView>;
  epicSignOut(): Promise<ActionResult>;
  epicLibraryCheck(): Promise<EpicLibraryCheck>;
  gameVersionList(): Promise<GameVersionList>;
  /** Progress comes over onProgress (kind "install") as well as gameInstallGet. */
  gameInstallStart(version: string): Promise<InstallOutcome>;
  gameInstallPause(version: string): Promise<ActionResult>;
  gameInstallResume(version: string): Promise<ActionResult>;
  gameInstallCancel(version: string): Promise<ActionResult>;
  gameInstallGet(version: string): Promise<InstallOutcome>;
  /** A folder dialog for where versions install; refused (busy) while an install runs or waits to continue. */
  gameInstallChooseFolder(): Promise<ActionResult>;
  /** Cancels a download altogether: stops it and deletes what it downloaded. */
  gameInstallDiscard(version: string): Promise<ActionResult>;
  /** The server asks for a newer launcher: update now and restart, or answer launcher_update_failed. */
  updateLauncher(): Promise<ActionResult>;
  /** The signed-in account's online seconds; null when the server gives none. */
  getPlaytime(): Promise<number | null>;
  /** Sends the last crash's report to the server it was played on (`snapshot.crash`). */
  crashReportSend(): Promise<ActionResult>;
  /** Forgets the last crash; it is not offered again. */
  crashReportDismiss(): Promise<void>;
  onSnapshot(fn: (snapshot: Snapshot) => void): void;
  onProgress(fn: (progress: TaskProgress) => void): void;
  onHostKit(fn: (kit: HostKit) => void): void;
  onHostServer(fn: (view: HostServerView) => void): void;
  onInviteLink(fn: () => void): void;
  onWindowState(fn: (maximized: boolean) => void): void;
  onGameLog(fn: (lines: string[]) => void): void;
  /** What uninstalling a version deletes: its folder, the bytes freed, and whether the whole folder goes. */
  gameUninstallPlan(version: string): Promise<UninstallOutcome>;
  /** Deletes an installed version for good (no Recycle Bin); refused while the game runs. */
  gameUninstall(version: string): Promise<ActionResult>;
}

declare global {
  interface Window {
    launcher?: LauncherApi;
  }
}
