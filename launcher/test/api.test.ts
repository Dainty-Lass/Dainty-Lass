// The Tauri bridge (dainty-lass-ui's packages/bridge/src/api.ts, from the stand-in page's api.ts, DU2;
// MIGRATION_PLAN.md RA2; DR's test/api.test.ts) against Tauri's own IPC mocks: every LauncherApi method of
// Reforged's preload.ts reaches the command of the same name with the arguments the preload sends, and
// every call the page can make gets through the isolation hook. resolve-ui.ts loads the UI's sources.

import { test, mock } from "node:test";
import assert from "node:assert/strict";
import type { InvokeArgs } from "@tauri-apps/api/core";

// The Tauri API reads window.__TAURI_INTERNALS__, which the mocks install on window
(globalThis as unknown as { window: unknown }).window = globalThis;

import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { api, CALL_TIMEOUT_MS } from "../../ui/packages/bridge/src/api.ts";
import { loadHook } from "./hook.ts";

type Call = { cmd: string; args: InvokeArgs | undefined };

function record(answer: (cmd: string, args: InvokeArgs | undefined) => unknown = () => ({ ok: true })): Call[] {
  const calls: Call[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    return answer(cmd, args);
  });
  return calls;
}

/** Every method the page can call, with arguments of the kinds it sends. */
async function callEverything(): Promise<void> {
  await api.getSnapshot();
  await api.checkInvite("reforged-revival://join?v=1");
  await api.submitInvite("x", true, true);
  await api.submitInvite("x");
  await api.retryConnect();
  await api.forgetServer();
  await api.switchServer("0123456789abcdef01234567");
  await api.removeSavedServer("0123456789abcdef01234567");
  await api.register("Slayer");
  await api.useExistingKey("RRK_x");
  await api.importKeyFile();
  await api.saveKeyBackup();
  await api.dismissBackupOffer();
  await api.chooseGameFolder();
  await api.useGamePath("C:\\Games\\Dauntless");
  await api.useDetectedGame(2);
  await api.verifyGame();
  await api.revivedInstall("repair");
  await api.revivedChooseFolder();
  await api.revivedUseDetected(2);
  await api.trustRuntime();
  await api.installRuntime();
  await api.chooseLocalRuntime();
  await api.clearLocalRuntime();
  await api.cancelTask();
  await api.play();
  await api.setSettings({ graphics: -1, windowed: true, streamerMode: true, hostTools: true, language: "pt-BR", gameLanguage: "auto", runtimeProfile: "development" });
  await api.setStatusPolling(true);
  await api.refreshStatus();
  await api.getNews();
  await api.getBranding();
  await api.openExternal("project_docs_sdk");
  await api.openGameFolder();
  await api.openLogsFolder();
  await api.logout();
  await api.installUpdate();
  await api.hostGet();
  await api.hostChooseWorkspace();
  await api.hostChooseDumper();
  await api.hostImportGameData();
  await api.hostImportSdkFolder();
  await api.hostDownloadProject();
  await api.hostAction("build_runtime");
  await api.hostOpen("dumps");
  await api.hostServerGet();
  await api.hostServerStart();
  await api.hostServerStop();
  await api.hostServerRestart("content");
  await api.hostSetup({ name: "Friends", mode: "private", publicHost: null });
  await api.hostSetup({ name: "Hunts", mode: "public", publicHost: "play.example.org" });
  await api.hostCreateInvite(null);
  await api.hostCreateInvite("Sam");
  await api.hostUpdate();
  await api.hostSetMultipliers(null);
  await api.hostSetMultipliers({ xp: 2, rewards: 3.5 });
  await api.copyText("reforged-revival://join?v=1");
  await api.takeInviteLink();
  await api.dismissNotice();
  await api.dismissError();
  api.onProgress(() => undefined);
  await api.epicSignInStart();
  await api.epicSignInCode("0123456789abcdef0123456789abcdef");
  await api.epicGet();
  await api.epicSignOut();
  await api.epicLibraryCheck();
  await api.gameVersionList();
  await api.gameInstallStart("1.14.7");
  await api.gameInstallPause("1.14.7");
  await api.gameInstallResume("1.14.7");
  await api.gameInstallCancel("1.14.7");
  await api.gameInstallGet("1.14.7");
  await api.regionPing();
  await api.moduleList();
  await api.moduleGet("isles-1-14-7");
  await api.moduleRunCommand("isles-1-14-7", "reload_catalog", { scope: "items" });
  await api.moduleSetSettings("isles-1-14-7", { game: { max_players: 4 } });
  await api.moduleLoad("isles-1-14-7");
  await api.moduleUnload("isles-1-14-7");
  await api.realmHost("hunt");
  await api.realmJoin("realm-01");
  await api.realmStop();
  await api.realmGet();
  await api.historyQuery({ actor: "key_01", since: "2026-10-01T00:00:00Z" });
  await api.historyRevertDryRun({ actor: "key_01", since: "2026-10-01T00:00:00Z" });
  await api.historyRevertApply("plan_01");
  await api.guardedList();
  await api.guardedApprove("guard_01");
  await api.guardedCancel("guard_01");
  await api.serverDoctor();
  await api.serverBackup();
  await api.serverConfigGet();
  await api.serverConfigSave({
    registration: "INVITECODE",
    database: "sqlite",
    maxHubs: 1,
    maxHunts: 3,
    freshStart: false,
  });
  await api.serverMigrateMongo();
  await api.serverConfirmFreshStart();
  await api.serverInviteList();
  await api.adminAction({ action: "lookup", player: "Slayer" });
  await api.remoteStatus();
  await api.remoteInitController({ node: "ctrl-main", region: "us-east" });
  await api.remoteInvite({ node: "managed-01", region: "eu-west", label: "EU 1" });
  await api.remoteEnroll({ uri: "dainty-lass-remote://enroll?v=1", label: "EU 1" });
  await api.remoteRevoke("managed-01");
  await api.remoteLeave();
  await api.remoteExec({ node: "managed-01", action: "doctor" });
}

test("each call reaches its command with the preload's arguments", async () => {
  const calls = record();
  try {
    await api.checkInvite("reforged-revival://join?v=1");
    await api.submitInvite("x", true, true);
    await api.submitInvite("x");
    await api.useDetectedGame("3" as unknown as number);
    await api.openExternal("project_docs_sdk");
    await api.hostSetup({ name: "Hunts", mode: "public", publicHost: "play.example.org" });
    await api.hostCreateInvite(null);
    await api.hostSetMultipliers({ xp: "2" as unknown as number, rewards: 3 });
    assert.deepEqual(
      calls.map((c) => [c.cmd, c.args]),
      [
        ["check_invite", { text: "reforged-revival://join?v=1" }],
        ["submit_invite", { text: "x", acceptNewCertificate: true, carryAccount: true }],
        ["submit_invite", { text: "x", acceptNewCertificate: false, carryAccount: false }],
        ["use_detected_game", { index: 3 }],
        ["open_external", { target: "project_docs_sdk" }],
        ["host_setup", { request: { name: "Hunts", mode: "public", publicHost: "play.example.org" } }],
        ["host_create_invite", { forName: null }],
        ["host_set_multipliers", { multipliers: { xp: 2, rewards: 3 } }],
      ],
    );
  } finally {
    clearMocks();
  }
});

test("every call the page makes passes the isolation hook", async () => {
  const { hook, commands } = loadHook();
  const calls = record((cmd) => (cmd === "copy_text" ? true : { ok: true }));
  try {
    await callEverything();
    await new Promise((resolve) => setImmediate(resolve));
    const used = new Set<string>();
    for (const { cmd, args } of calls) {
      // As Tauri sends it: a Channel goes as its "__CHANNEL__:<id>" string
      const payload = { cmd, callback: 1, error: 2, payload: JSON.parse(JSON.stringify(args ?? {})), options: undefined };
      assert.equal(hook(payload), payload, `${cmd} ${JSON.stringify(args)}`);
      used.add(cmd);
    }
    assert.deepEqual([...used].sort(), Object.keys(commands).sort(), "every command is called by some page method");
  } finally {
    clearMocks();
  }
});

test("settings go across without unknown fields, as in preload.ts", async () => {
  const calls = record((_cmd, args) => (args as { patch: unknown }).patch);
  try {
    await api.setSettings({ runtimeProfile: "development", huntRegion: "us", unexpected: "discard" } as never);
    assert.deepEqual(calls.at(-1), { cmd: "set_settings", args: { patch: { runtimeProfile: "development" } } });
  } finally {
    clearMocks();
  }
});

test("1.4.4 calls reach their commands, the mode one of two names (the legacy edition)", async () => {
  const calls = record();
  try {
    await api.revivedInstall("install");
    await api.revivedInstall("anything" as never);
    await api.revivedChooseFolder();
    await api.revivedUseDetected("1" as unknown as number);
    await api.setSettings({ gameEdition: "legacy" });
    assert.deepEqual(
      calls.map((c) => [c.cmd, c.args]),
      [
        ["revived_install", { mode: "install" }],
        ["revived_install", { mode: "install" }],
        ["revived_choose_folder", {}],
        ["revived_use_detected", { index: 1 }],
        ["set_settings", { patch: { gameEdition: "legacy" } }],
      ],
    );
  } finally {
    clearMocks();
  }
});

test("planned calls send their arguments in the shapes the hook checks", async () => {
  const calls = record();
  try {
    await api.moduleRunCommand("isles-1-14-7", "reload_catalog", { scope: "items" });
    await api.realmHost("raid" as never);
    await api.historyQuery({ actor: "key_01" });
    await api.historyRevertDryRun({ since: "2026-10-01T00:00:00Z", cursor: "x" } as never);
    assert.deepEqual(
      calls.map((c) => [c.cmd, c.args]),
      [
        ["module_run_command", { module: "isles-1-14-7", command: "reload_catalog", input: '{"scope":"items"}' }],
        ["realm_host", { kind: "hub" }],
        ["history_query", { filter: { account: null, actor: "key_01", since: null, until: null, cursor: null } }],
        ["history_revert_dry_run", { selection: { account: null, actor: null, since: "2026-10-01T00:00:00Z", until: null } }],
      ],
    );
  } finally {
    clearMocks();
  }
});

test("a failed action answers like the Electron main process did", async () => {
  record(() => {
    throw new Error("refused");
  });
  const errors = mock.method(console, "error", () => undefined);
  try {
    assert.deepEqual(await api.play(), { ok: false, error: { code: "unknown" } });
    await api.cancelTask();
    assert.equal(await api.copyText("x"), false);
    assert.equal(errors.mock.callCount(), 3);
    await assert.rejects(api.getSnapshot(), /get_snapshot: Error: refused/);
  } finally {
    errors.mock.restore();
    clearMocks();
  }
});

test("a call the isolation hook dropped times out instead of hanging", async () => {
  mock.timers.enable({ apis: ["setTimeout"] });
  mockIPC(() => new Promise(() => undefined));
  const errors = mock.method(console, "error", () => undefined);
  try {
    const snapshot = api.getSnapshot();
    const play = api.play();
    mock.timers.tick(CALL_TIMEOUT_MS);
    await assert.rejects(snapshot, /get_snapshot: no answer/);
    assert.deepEqual(await play, { ok: false, error: { code: "unknown" } });
  } finally {
    errors.mock.restore();
    mock.timers.reset();
    clearMocks();
  }
});

test("pushes arrive as events and progress over a channel", async () => {
  const calls: Call[] = [];
  mockIPC(
    (cmd, args) => {
      calls.push({ cmd, args });
      return null;
    },
    { shouldMockEvents: true },
  );
  try {
    const progress: unknown[] = [];
    api.onProgress((p) => progress.push(p));
    await new Promise((resolve) => setImmediate(resolve));
    const subscribe = calls.find((c) => c.cmd === "subscribe_progress");
    assert.ok(subscribe, "subscribed");
    const channel = (subscribe.args as { onProgress: { id: number } }).onProgress;
    const internals = (globalThis as unknown as { __TAURI_INTERNALS__: { runCallback(id: number, data: unknown): void } }).__TAURI_INTERNALS__;
    internals.runCallback(channel.id, { index: 0, message: { kind: "runtime" } });
    assert.deepEqual(progress, [{ kind: "runtime" }]);

    const states: boolean[] = [];
    const kits: unknown[] = [];
    const servers: unknown[] = [];
    api.onWindowState((maximized) => states.push(maximized));
    api.onHostKit((kit) => kits.push(kit));
    api.onHostServer((view) => servers.push(view));
    await new Promise((resolve) => setImmediate(resolve));
    const { emit } = await import("@tauri-apps/api/event");
    await emit("window-state", true);
    await emit("window-state", "yes");
    await emit("host-kit", { workspace: null });
    await emit("host-server", { state: "stopped" });
    assert.deepEqual(states, [true, false]);
    assert.deepEqual(kits, [{ workspace: null }]);
    assert.deepEqual(servers, [{ state: "stopped" }]);
  } finally {
    clearMocks();
  }
});

test("the window controls use Tauri's window calls", async () => {
  mockWindows("main");
  let maximized = false;
  const calls = record((cmd) => {
    if (cmd === "plugin:window|toggle_maximize") maximized = !maximized;
    if (cmd === "plugin:window|is_maximized") return maximized;
    return null;
  });
  try {
    await api.minimize();
    assert.equal(await api.toggleMaximize(), true);
    assert.equal(await api.toggleMaximize(), false);
    await api.close();
    assert.deepEqual(
      calls.map((c) => c.cmd),
      ["plugin:window|minimize", "plugin:window|toggle_maximize", "plugin:window|is_maximized", "plugin:window|toggle_maximize", "plugin:window|is_maximized", "plugin:window|close"],
    );
    const { hook } = loadHook();
    for (const { cmd, args } of calls) {
      const payload = { cmd, callback: 1, error: 2, payload: args ?? {}, options: undefined };
      assert.equal(hook(payload), payload, cmd);
    }
  } finally {
    clearMocks();
  }
});
