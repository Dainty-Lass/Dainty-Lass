// The bridge's surface as the page sees it: one method per command in src-tauri/src/bridge/spec.rs, plus the
// window calls and the events. The value types come from generated/types.ts (ts-rs, RR_UPDATE_GENERATED=1).

import type {
  ActionResult,
  Branding,
  GameVersionList,
  HostAction,
  HostFolder,
  HostKit,
  HostMultipliers,
  HostServerView,
  HostSetupRequest,
  InstallMode,
  InviteCheck,
  NewsItem,
  RealmKind,
  RegionPing,
  RegisterOutcome,
  Settings,
  Snapshot,
  TaskProgress,
} from "./generated/types.ts";

export type { ActionResult, GameVersionList, HostKit, HostServerView, RegionPing, Settings, Snapshot, TaskProgress };

export interface LauncherApi {
  getSnapshot(): Promise<Snapshot>;
  checkInvite(text: string): Promise<InviteCheck>;
  submitInvite(text: string, acceptNewCertificate?: boolean, carryAccount?: boolean): Promise<ActionResult>;
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
  hostServerRestart(name: string): Promise<ActionResult>;
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
  onSnapshot(fn: (snapshot: Snapshot) => void): void;
  onProgress(fn: (progress: TaskProgress) => void): void;
  onHostKit(fn: (kit: HostKit) => void): void;
  onHostServer(fn: (view: HostServerView) => void): void;
  onInviteLink(fn: () => void): void;
  onWindowState(fn: (maximized: boolean) => void): void;
  // Registered ahead of the rows that fill them (DF8): each answers not_implemented for now
  epicSignInStart(): Promise<ActionResult>;
  epicSignInCode(code: string): Promise<ActionResult>;
  epicGet(): Promise<ActionResult>;
  epicSignOut(): Promise<ActionResult>;
  epicLibraryCheck(): Promise<ActionResult>;
  // DE4's, filled: the installable versions, each Epic build with its sizes
  gameVersionList(): Promise<GameVersionList>;
  gameInstallStart(version: string): Promise<ActionResult>;
  gameInstallPause(version: string): Promise<ActionResult>;
  gameInstallResume(version: string): Promise<ActionResult>;
  gameInstallCancel(version: string): Promise<ActionResult>;
  gameInstallGet(version: string): Promise<ActionResult>;
  // DN7's, filled: the joined server's regions, nearest first
  regionPing(): Promise<RegionPing>;
  moduleList(): Promise<ActionResult>;
  moduleGet(module: string): Promise<ActionResult>;
  moduleRunCommand(module: string, command: string, input: unknown): Promise<ActionResult>;
  moduleSetSettings(module: string, values: Record<string, unknown>): Promise<ActionResult>;
  moduleLoad(module: string): Promise<ActionResult>;
  moduleUnload(module: string): Promise<ActionResult>;
  realmHost(kind: RealmKind): Promise<ActionResult>;
  realmJoin(realm: string): Promise<ActionResult>;
  realmStop(): Promise<ActionResult>;
  realmGet(): Promise<ActionResult>;
  historyQuery(filter: HistoryFilter): Promise<ActionResult>;
  historyRevertDryRun(selection: RevertSelection): Promise<ActionResult>;
  historyRevertApply(plan: string): Promise<ActionResult>;
  guardedList(): Promise<ActionResult>;
  guardedApprove(id: string): Promise<ActionResult>;
  guardedCancel(id: string): Promise<ActionResult>;
}

// Who and when a history query or a revert covers (DD23): an account, an actor, RFC 3339 times
export interface RevertSelection {
  account?: string | null;
  actor?: string | null;
  since?: string | null;
  until?: string | null;
}

export interface HistoryFilter extends RevertSelection {
  cursor?: string | null;
}

declare global {
  interface Window {
    launcher?: LauncherApi;
  }
}
