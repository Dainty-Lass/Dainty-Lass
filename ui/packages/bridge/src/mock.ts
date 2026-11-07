// A stand-in for the Tauri bridge, so the launcher and server manager pages run in a plain browser. It keeps
// a believable snapshot and walks the phases the backend would; it never touches a key, a file or a server.
import type {
  ActionResult,
  EpicView,
  HostKit,
  HostServerView,
  InstallInfo,
  NewsItem,
  Settings,
  Snapshot,
  TaskProgress,
} from "./generated/types";
import type { LauncherApi } from "./types";

const OK: ActionResult = { ok: true };
const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

function initialSnapshot(): Snapshot {
  return {
    phase: "join",
    busy: false,
    server: null,
    savedServers: [],
    connect: { checking: false, problem: null, tailscaleInstalled: true, lastCheckedAt: null },
    account: { username: null, hasKey: false, offerBackup: false },
    game: {
      root: null,
      problem: "not_set",
      versionText: null,
      versionOk: false,
      exeVerified: false,
      detected: [{ root: "C:\\Games\\Dauntless", source: "epic", version: "1.14.7" }],
    },
    revived: null,
    runtime: { state: "missing", version: "2026.10.30", files: [], localDir: null },
    task: null,
    running: { game: false, relayPort: null },
    settings: {
      graphics: 2,
      windowed: false,
      language: "en",
      gameLanguage: "auto",
      runtimeProfile: "production",
      streamerMode: false,
      hostTools: true,
      gameEdition: "reborn",
      minimizeToTray: false,
      closeToTray: false,
      showGameConsole: false,
    },
    app: {
      version: "2.0.0-dev",
      packaged: false,
      updateReady: false,
      platform: "win32",
      hostToolsAvailable: true,
      legacyAvailable: false,
      gameVersions: ["1.14.7"],
      serverGui: false,
    },
    status: null,
    statusUnsupported: false,
    lastError: null,
    notice: null,
  };
}

function initialHostKit(): HostKit {
  return {
    workspace: "C:\\DaintyLass",
    workspaceOk: true,
    upstream: { present: true, patched: true },
    sdk: { present: true, functionFiles: 412 },
    dumps: [
      {
        dir: "C:\\DaintyLass\\.workspace\\dumps\\1.14.7",
        name: "1.14.7",
        modified: "2026-10-28T19:02:00Z",
      },
    ],
    dumperDll: "C:\\Tools\\Dumper-7.dll",
    builds: { runtime: true, loader: true, exporter: false, published: true },
    toolchain: { visualStudio: "Visual Studio 2022 (17.14)", msbuild: true, cargo: true },
    gameData: { backend: "real", director: "real" },
    exports: { present: true, inGame: false },
    serverConfig: {
      configured: true,
      name: "Dainty Friends",
      mode: "private",
      host: null,
      multipliers: null,
    },
    requirements: [
      { id: "dauntless", state: "ok", detail: "1.14.7", problem: null },
      { id: "git", state: "ok", detail: "2.51.0", problem: null },
      { id: "node", state: "ok", detail: "22.23.4", problem: null },
      { id: "mongodb", state: "ok", detail: "8.0", problem: null },
      { id: "tailscale", state: "ok", detail: "1.92", problem: null },
      { id: "visual_studio", state: "ok", detail: "17.14", problem: null },
      { id: "rust", state: "ok", detail: "1.93.0", problem: null },
      { id: "dumper7", state: "missing", detail: null, problem: null },
      { id: "vc_redist", state: "ok", detail: "14.44", problem: null },
    ],
    job: null,
    update: null,
    projectTarget: "C:\\Games\\DaintyLass",
  };
}

export function mockLauncher(): LauncherApi {
  let snapshot = initialSnapshot();
  let kit = initialHostKit();
  let server: HostServerView = {
    state: "stopped",
    message: null,
    lines: [],
    invite: null,
    embedded: false,
    components: [],
  };
  const snapshotListeners = new Set<(s: Snapshot) => void>();
  const progressListeners = new Set<(p: TaskProgress) => void>();
  const kitListeners = new Set<(k: HostKit) => void>();
  const serverListeners = new Set<(v: HostServerView) => void>();

  const emit = (patch: Partial<Snapshot>) => {
    snapshot = { ...snapshot, ...patch };
    for (const fn of snapshotListeners) fn(snapshot);
  };
  const emitServer = (patch: Partial<HostServerView>) => {
    server = { ...server, ...patch };
    for (const fn of serverListeners) fn(server);
  };
  const emitKit = (patch: Partial<HostKit>) => {
    kit = { ...kit, ...patch };
    for (const fn of kitListeners) fn(kit);
  };
  const log = (line: string) =>
    emitServer({
      lines: [...server.lines, `${new Date().toISOString().slice(11, 19)} ${line}`].slice(-400),
    });

  const status = () => ({
    name: snapshot.server?.name ?? "Dainty Friends",
    online: true,
    version: "1.14.7",
    commit: "f891d92",
    sourceUrl: "https://git.silentspud.com/silentspud/dainty-lass",
    registration: "INVITECODE" as const,
    playersOnline: 3,
    players: [
      { name: "Moonwick", where: "city" as const, instance: "hub-1" },
      { name: "Tamsin_", where: "hunt" as const, instance: "hunt-2" },
      { name: "Grevel", where: "hunt" as const, instance: "hunt-2" },
    ],
    instances: [
      {
        id: "hub-1",
        kind: "city" as const,
        title: "Ramsgate",
        map: "Ramsgate",
        behemoth: null,
        players: 1,
        maxPlayers: 40,
        startedAt: new Date(Date.now() - 7_200_000).toISOString(),
      },
      {
        id: "hunt-2",
        kind: "hunt" as const,
        title: "Heroic+ Shrike",
        map: "Coldrift",
        behemoth: "Shrike",
        players: 2,
        maxPlayers: 4,
        startedAt: new Date(Date.now() - 600_000).toISOString(),
      },
    ],
    uptimeSeconds: 86_400 + 3_600 * 5,
    game: { build: "1.14.7", changelist: 647472 },
    gameData: "real" as const,
    chat: true,
    contentPort: null,
    limited: false,
  });

  let epic: EpicView = { signedIn: false, displayName: null, signingIn: false };
  const EPIC_TOTAL = 19_800_000_000;
  let install: InstallInfo = {
    version: "1.14.7",
    state: "not_installed",
    folder: "C:\\Users\\slayer\\Games\\Dauntless\\1.14.7",
    progress: null,
    error: null,
  };
  let installTimer: ReturnType<typeof setInterval> | null = null;
  function stopInstall() {
    if (installTimer) clearInterval(installTimer);
    installTimer = null;
  }
  // A download that moves while it is not paused, finishing at EPIC_TOTAL
  function runInstall() {
    stopInstall();
    installTimer = setInterval(() => {
      const done = Math.min(EPIC_TOTAL, (install.progress?.doneBytes ?? 0) + EPIC_TOTAL / 40);
      const p: TaskProgress = {
        kind: "install",
        totalBytes: EPIC_TOTAL,
        doneBytes: done,
        filesTotal: 400,
        filesDone: Math.round((done / EPIC_TOTAL) * 400),
        bytesPerSecond: 48_000_000,
        etaSeconds: Math.round((EPIC_TOTAL - done) / 48_000_000),
        currentFile: "Archon/Content/Paks/pakchunk0-WindowsNoEditor.pak",
      };
      for (const fn of progressListeners) fn(p);
      install =
        done >= EPIC_TOTAL
          ? { ...install, state: "installed", progress: null }
          : { ...install, state: "downloading", progress: p };
      if (done >= EPIC_TOTAL) stopInstall();
    }, 400);
  }

  async function task(kind: TaskProgress["kind"], total: number) {
    emit({ busy: true });
    for (let done = 0; done <= total; done += total / 10) {
      const p: TaskProgress = {
        kind,
        totalBytes: total,
        doneBytes: done,
        filesTotal: 10,
        filesDone: Math.round((done / total) * 10),
        bytesPerSecond: 12_000_000,
        etaSeconds: Math.round((total - done) / 12_000_000),
        currentFile: "Rustic-Isles.dll",
      };
      emit({ task: p });
      for (const fn of progressListeners) fn(p);
      await wait(180);
    }
    emit({ busy: false, task: null });
  }

  return {
    getSnapshot: async () => snapshot,
    checkInvite: async (text) =>
      text.trim().startsWith("dainty://") || text.trim().startsWith("reforged://")
        ? { ok: true, certificateChanged: false, previousFingerprint: null, movedFrom: null }
        : { ok: false, error: { code: "invite_invalid_format" } },
    submitInvite: async () => {
      emit({ phase: "connect", connect: { ...snapshot.connect, checking: true } });
      await wait(600);
      const server = {
        game: "reforged" as const,
        mode: "private" as const,
        host: "100.64.0.7",
        port: 61005,
        name: "Dainty Friends",
        hasShare: true,
        hasPendingInvite: true,
        loopback: false,
        fingerprint: null,
      };
      emit({
        phase: "register",
        server,
        savedServers: [
          {
            id: "slot-1",
            game: "reforged",
            mode: "private",
            host: server.host,
            port: server.port,
            name: server.name,
            current: true,
            username: null,
            unsupported: false,
          },
        ],
        connect: { ...snapshot.connect, checking: false, lastCheckedAt: new Date().toISOString() },
        status: status(),
      });
      return OK;
    },
    retryConnect: async () => OK,
    forgetServer: async () => {
      emit({ phase: "join", server: null, savedServers: [], status: null });
      return OK;
    },
    switchServer: async () => OK,
    removeSavedServer: async (id) => {
      emit({ savedServers: snapshot.savedServers.filter((s) => s.id !== id) });
      return OK;
    },
    register: async (username) => {
      if (!/^\w{3,16}$/.test(username)) return { ok: false, error: { code: "username_invalid" } };
      emit({ phase: "locate", account: { username, hasKey: true, offerBackup: true } });
      return { ok: true, username };
    },
    useExistingKey: async () => {
      emit({
        phase: "locate",
        account: { username: "Moonwick", hasKey: true, offerBackup: false },
      });
      return OK;
    },
    importKeyFile: async () => OK,
    saveKeyBackup: async () => {
      emit({ account: { ...snapshot.account, offerBackup: false }, notice: "backup_saved" });
      return OK;
    },
    dismissBackupOffer: async () => emit({ account: { ...snapshot.account, offerBackup: false } }),
    chooseGameFolder: async () => OK,
    useGamePath: async (root) => {
      emit({
        phase: "runtime",
        game: {
          ...snapshot.game,
          root,
          problem: null,
          versionOk: true,
          exeVerified: true,
          versionText: "dauntless_rel-1.14.7_Shipping",
        },
      });
      return OK;
    },
    useDetectedGame: async (index) => {
      const found = snapshot.game.detected[index];
      if (!found) return { ok: false, error: { code: "game_folder_not_found" } };
      emit({
        phase: "runtime",
        game: {
          ...snapshot.game,
          root: found.root,
          problem: null,
          versionOk: true,
          exeVerified: true,
          versionText: "dauntless_rel-1.14.7_Shipping",
        },
      });
      return OK;
    },
    verifyGame: async () => {
      await task("verify", 4_000_000_000);
      emit({ notice: "game_verified" });
      return OK;
    },
    revivedInstall: async () => ({ ok: false, error: { code: "edition_other" } }),
    revivedChooseFolder: async () => ({ ok: false, error: { code: "edition_other" } }),
    revivedUseDetected: async () => ({ ok: false, error: { code: "edition_other" } }),
    trustRuntime: async () => OK,
    installRuntime: async () => {
      emit({ phase: "installing" });
      await task("runtime", 48_000_000);
      emit({
        phase: "ready",
        runtime: { ...snapshot.runtime, state: "ok" },
        notice: "runtime_installed",
      });
      return OK;
    },
    chooseLocalRuntime: async () => OK,
    clearLocalRuntime: async () => OK,
    cancelTask: async () => emit({ busy: false, task: null }),
    play: async () => {
      emit({ phase: "running", running: { game: true, relayPort: 61100 } });
      setTimeout(() => emit({ phase: "ready", running: { game: false, relayPort: null } }), 8000);
      return OK;
    },
    setSettings: async (patch) => {
      const settings: Settings = { ...snapshot.settings, ...patch };
      emit({ settings });
      return settings;
    },
    setStatusPolling: async () => {},
    refreshStatus: async () => {
      if (snapshot.server) emit({ status: status() });
    },
    getNews: async (): Promise<NewsItem[]> => [
      {
        date: "2026-10-30",
        title: "Heroic escalations are back",
        body: "Escalations rotate every Tuesday. Bring friends.",
      },
      {
        date: "2026-10-24",
        title: "Server moved to Dainty Lass",
        body: "Your account key carries over; paste the new invite once.",
      },
    ],
    getBranding: async () => ({ backgrounds: [], accent: null }),
    openExternal: async () => OK,
    openGameFolder: async () => {},
    openLogsFolder: async () => {},
    logout: async () => {
      emit({ phase: "register", account: { username: null, hasKey: false, offerBackup: false } });
      return OK;
    },
    installUpdate: async () => {},
    hostGet: async () => kit,
    hostChooseWorkspace: async () => OK,
    hostChooseDumper: async () => OK,
    hostImportGameData: async () => OK,
    hostImportSdkFolder: async () => OK,
    hostDownloadProject: async () => OK,
    hostAction: async (action) => {
      emitKit({
        job: {
          kind: action === "inject_dumper" ? "inject" : (action as never),
          state: "running",
          message: null,
          logFile: null,
        },
      });
      await wait(1500);
      emitKit({ job: { ...kit.job!, state: "done", message: "Done." } });
      return OK;
    },
    hostOpen: async () => OK,
    hostServerGet: async () => server,
    hostServerStart: async () => {
      emitServer({ state: "starting", message: null });
      for (const line of [
        "gateway   | listening on 0.0.0.0:61005",
        "backend   | ready on 127.0.0.1:61000",
        "director  | Ramsgate up (port 8770)",
        "admin     | GraphQL on 127.0.0.1:61003",
      ]) {
        await wait(300);
        log(line);
      }
      emitServer({ state: "running" });
      return OK;
    },
    hostServerStop: async () => {
      emitServer({ state: "stopping" });
      await wait(500);
      log("host      | stopped");
      emitServer({ state: "stopped" });
      return OK;
    },
    hostSetup: async (req) => {
      emitKit({
        serverConfig: {
          ...kit.serverConfig,
          configured: true,
          name: req.name,
          mode: req.mode,
          host: req.publicHost,
        },
      });
      return OK;
    },
    hostCreateInvite: async (forName) => {
      emitServer({
        invite: {
          link: `dainty://join/100.64.0.7:61005/${Math.random().toString(36).slice(2, 10)}`,
          for: forName,
        },
      });
      return OK;
    },
    hostUpdate: async () => {
      emitKit({
        update: { state: "up_to_date", head: "f891d92", files: 0, runtimeChanged: false },
      });
      return OK;
    },
    hostSetMultipliers: async (multipliers) => {
      emitKit({ serverConfig: { ...kit.serverConfig, multipliers } });
      return OK;
    },
    copyText: async (text) => {
      try {
        await navigator.clipboard.writeText(text);
        return true;
      } catch {
        return false;
      }
    },
    dismissNotice: async () => emit({ notice: null }),
    dismissError: async () => emit({ lastError: null }),
    minimize: async () => {},
    toggleMaximize: async () => false,
    close: async () => {},
    takeInviteLink: async () => null,
    gameLogGet: async () => [
      "LogInit: Display: Running engine for game: Dauntless",
      "LogInit: Command Line:  -NoEAC -AUTH_PASSWORD=<hidden> -epicapp=Archon",
      "[Rustic Isles] runtime 2026.10.30 loaded",
      "LogNet: Browse: 127.0.0.1:61000/Game/Maps/Ramsgate",
    ],
    epicSignInStart: async () => {
      epic = { ...epic, signingIn: true };
      await wait(1_500);
      epic = { signedIn: true, displayName: "Slayer", signingIn: false };
      return OK;
    },
    epicSignInCode: async (code) => {
      if (code.trim().length < 8) return { ok: false, error: { code: "epic_code_invalid" } };
      epic = { signedIn: true, displayName: "Slayer", signingIn: false };
      return OK;
    },
    epicGet: async () => epic,
    epicSignOut: async () => {
      epic = { signedIn: false, displayName: null, signingIn: false };
      return OK;
    },
    epicLibraryCheck: async () =>
      epic.signedIn
        ? { ok: true, owned: true, checkedAt: new Date().toISOString() }
        : { ok: false, error: { code: "epic_signed_out" } },
    gameVersionList: async () => {
      await wait(600);
      return {
        ok: true,
        versions: [
          {
            version: "1.14.7",
            displayName: "Rustic Isles 1.14.7",
            origin: "launcher",
            install: "epic",
            epic: {
              buildVersion: "1.14.7.647472",
              manifest: "ready",
              downloadBytes: EPIC_TOTAL,
              installBytes: 31_200_000_000,
            },
          },
        ],
      };
    },
    gameInstallStart: async (version) => {
      if (!epic.signedIn) return { ok: false, error: { code: "epic_signed_out" } };
      install = {
        version,
        state: "preparing",
        folder: install.folder,
        progress: install.progress,
        error: null,
      };
      runInstall();
      return { ok: true, install };
    },
    gameInstallPause: async () => {
      stopInstall();
      install = { ...install, state: "paused" };
      return OK;
    },
    gameInstallResume: async () => {
      runInstall();
      return OK;
    },
    gameInstallCancel: async () => {
      stopInstall();
      install = { ...install, state: "resumable" };
      return OK;
    },
    gameInstallGet: async () => ({ ok: true, install }),
    gameInstallChooseFolder: async () => {
      if (installTimer || install.state === "paused" || install.state === "resumable")
        return { ok: false, error: { code: "busy" } };
      install = { ...install, folder: "D:\\Games\\Dauntless\\1.14.7" };
      return OK;
    },
    onSnapshot: (fn) => void snapshotListeners.add(fn),
    onProgress: (fn) => void progressListeners.add(fn),
    onHostKit: (fn) => void kitListeners.add(fn),
    onHostServer: (fn) => void serverListeners.add(fn),
    onInviteLink: () => {},
    onWindowState: () => {},
    onGameLog: () => {},
  };
}
