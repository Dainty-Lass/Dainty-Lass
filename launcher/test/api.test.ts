// SPDX-License-Identifier: MIT
// The page's API (ui/packages/bridge/src/api.ts) against Tauri's IPC mocks, with every call put through the
// isolation hook as the isolation frame would: what the page sends must be what the table allows.
import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test"
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks"
import { readFileSync } from "node:fs"
import path from "node:path"

import type { LauncherApi } from "../../ui/packages/bridge/src/api.ts"
import { loadHook } from "./hook"

// ui's node_modules needn't be installed: the bridge's Tauri imports are pointed at this package's copy, the one
// the mocks above come from. Bun applies onLoad plugins only to imports it can't see statically, hence the
// computed path.
Bun.plugin({
  name: "bridge-tauri-from-launcher",
  setup(build) {
    build.onLoad({ filter: /ui.packages.bridge.src.\w+\.ts$/ }, (args) => ({
      contents: readFileSync(args.path, "utf8").replace(
        /(from\s+")(@tauri-apps\/api\/[a-z]+)(")/g,
        (_, open: string, specifier: string, close: string) =>
          open + Bun.resolveSync(specifier, import.meta.dir).replaceAll("\\", "/") + close
      ),
      loader: "ts",
    }))
  },
})

// @tauri-apps/api reads its internals from window
;(globalThis as unknown as { window: unknown }).window = globalThis

const bridge = path.join(import.meta.dir, "..", "..", "ui", "packages", "bridge", "src", "api.ts")
const { api, CALL_TIMEOUT_MS } = (await import(bridge)) as typeof import("../../ui/packages/bridge/src/api.ts")
const { hook, commands } = loadHook()

interface Internals {
  runCallback(id: number, data: unknown): void
}

function internals(): Internals {
  return (globalThis as unknown as { __TAURI_INTERNALS__: Internals }).__TAURI_INTERNALS__
}

interface Call {
  cmd: string
  args: Record<string, unknown>
}

let calls: Call[] = []
let dropped: Call[] = []
let listeners = new Map<string, number>()
let nextEventId = 1

// What Tauri puts on the wire: undefined fields go, a Channel becomes its __CHANNEL__ string
function wire(args: unknown): Record<string, unknown> {
  return JSON.parse(JSON.stringify(args ?? {})) as Record<string, unknown>
}

function answer(cmd: string, args: Record<string, unknown>): unknown {
  switch (cmd) {
    case "plugin:event|listen":
      listeners.set(String(args.event), Number(args.handler))
      return nextEventId++
    case "plugin:window|is_maximized":
      return true
    case "copy_text":
      return true
    case "take_invite_link":
      return null
    case "get_news":
      return []
    case "subscribe_progress":
      return null
    default:
      return { ok: true }
  }
}

beforeEach(() => {
  calls = []
  dropped = []
  listeners = new Map()
  mockWindows("main")
  mockIPC((cmd, raw) => {
    const call = { cmd, args: wire(raw) }
    calls.push(call)
    const payload = { cmd, payload: call.args }
    const passed = hook(payload)
    if (passed !== payload) {
      dropped.push(call)
      return passed
    }
    return answer(cmd, call.args)
  })
})

afterEach(() => {
  clearMocks()
})

async function settle(): Promise<void> {
  for (let i = 0; i < 5; i++) await Bun.sleep(0)
}

const noop = () => {}

// One set of arguments per method; a method missing here fails the first test
const methodArgs: Record<keyof LauncherApi, unknown[]> = {
  getSnapshot: [],
  checkInvite: ["dainty-lass://join?x=1"],
  submitInvite: ["dainty-lass://join?x=1", true, false],
  retryConnect: [],
  forgetServer: [],
  switchServer: ["0123456789abcdef01234567"],
  removeSavedServer: ["0123456789abcdef01234567"],
  register: ["Slayer_42"],
  useExistingKey: ["RRK_test"],
  importKeyFile: [],
  saveKeyBackup: [],
  dismissBackupOffer: [],
  chooseGameFolder: [],
  useGamePath: ["C:/Games/Dauntless"],
  useDetectedGame: [0],
  verifyGame: [],
  revivedInstall: ["repair"],
  revivedChooseFolder: [],
  revivedUseDetected: [1],
  trustRuntime: [],
  installRuntime: [],
  chooseLocalRuntime: [],
  clearLocalRuntime: [],
  cancelTask: [],
  play: [],
  setSettings: [{ language: "pt-BR", windowed: true, graphics: 2 }],
  setStatusPolling: [true],
  refreshStatus: [],
  getNews: [],
  getBranding: [],
  openExternal: ["project_docs_players"],
  openGameFolder: [],
  openLogsFolder: [],
  logout: [],
  installUpdate: [],
  hostGet: [],
  hostChooseWorkspace: [],
  hostChooseDumper: [],
  hostImportGameData: [],
  hostImportSdkFolder: [],
  hostDownloadProject: [],
  hostAction: ["synthetic_data"],
  hostOpen: ["workspace"],
  hostServerGet: [],
  hostServerStart: [],
  hostServerStop: [],
  hostServerRestart: ["gateway"],
  hostSetup: [{ name: "Hunts", mode: "public", publicHost: "hunts.example.org" }],
  hostCreateInvite: ["Slayer42"],
  hostUpdate: [],
  hostSetMultipliers: [{ xp: 2, rewards: 1.5 }],
  copyText: ["copied"],
  dismissNotice: [],
  dismissError: [],
  minimize: [],
  toggleMaximize: [],
  close: [],
  takeInviteLink: [],
  onSnapshot: [noop],
  onProgress: [noop],
  onHostKit: [noop],
  onHostServer: [noop],
  onInviteLink: [noop],
  onWindowState: [noop],
  epicSignInStart: [],
  epicSignInCode: ["code"],
  epicGet: [],
  epicSignOut: [],
  epicLibraryCheck: [],
  gameVersionList: [],
  gameInstallStart: ["1.14.7"],
  gameInstallPause: ["1.14.7"],
  gameInstallResume: ["1.14.7"],
  gameInstallCancel: ["1.14.7"],
  gameInstallGet: ["1.14.7"],
  regionPing: [],
  moduleList: [],
  moduleGet: ["rustic-isles"],
  moduleRunCommand: ["rustic-isles", "grant", { player: "Slayer_42" }],
  moduleSetSettings: ["rustic-isles", { rates: { xp: 2 } }],
  moduleLoad: ["rustic-isles"],
  moduleUnload: ["rustic-isles"],
  realmHost: ["hunt"],
  realmJoin: ["realm-1"],
  realmStop: [],
  realmGet: [],
  historyQuery: [{ account: "a", cursor: "c" }],
  historyRevertDryRun: [{ actor: "admin", since: "2026-11-01T00:00:00Z" }],
  historyRevertApply: ["plan-1"],
  guardedList: [],
  guardedApprove: ["0123456789abcdef"],
  guardedCancel: ["0123456789abcdef"],
  serverDoctor: [],
  serverBackup: [],
  serverConfigGet: [],
  serverConfigSave: [{ registration: "INVITE", database: "sqlite", maxHubs: 4, maxHunts: 8, freshStart: false }],
  serverMigrateMongo: [],
  serverConfirmFreshStart: [],
  serverInviteList: [],
  adminAction: [{ action: "ban", player: "Slayer_42", reason: "test" }],
  remoteStatus: [],
  remoteInitController: [],
  remoteInvite: [{ node: "charlie", region: "ap-east" }],
  remoteEnroll: [{ uri: "dainty-lass-remote://enroll?x=1" }],
  remoteRevoke: ["bravo"],
  remoteLeave: [],
  remoteExec: [{ node: "alpha", action: "announce", reason: "hello" }],
}

function invokeMethod(name: string): unknown {
  const method = (api as unknown as Record<string, (...args: unknown[]) => unknown>)[name]
  if (typeof method !== "function") throw new Error(`api.${name} is not a method`)
  return method(...(methodArgs[name as keyof LauncherApi] ?? []))
}

describe("every method", () => {
  test("has arguments in this test", () => {
    const missing = Object.keys(api).filter((name) => !(name in methodArgs))
    const stale = Object.keys(methodArgs).filter((name) => !(name in api))
    expect({ missing, stale }).toEqual({ missing: [], stale: [] })
  })

  test("reaches table commands with the table's argument names, and the hook lets it through", async () => {
    const reached = new Set<string>()
    for (const name of Object.keys(api)) {
      calls = []
      await invokeMethod(name)
      await settle()
      expect(calls.length, `api.${name} made no call`).toBeGreaterThan(0)
      for (const call of calls) {
        const fields = commands[call.cmd]
        if (fields === undefined) continue
        reached.add(call.cmd)
        expect(Object.keys(call.args).sort(), `api.${name} -> ${call.cmd}`).toEqual(Object.keys(fields).sort())
      }
    }
    expect(dropped).toEqual([])
    const unreached = Object.keys(commands).filter((cmd) => !reached.has(cmd))
    expect(unreached, "commands no page method calls").toEqual([])
  })
})

describe("a call the hook drops", () => {
  test("times out to the page's failure answer", async () => {
    const realSetTimeout = globalThis.setTimeout
    // call.ts arms its deadline with the global setTimeout: shorten the page's deadlines, keep everything else
    const shortened = (fn: () => void, ms?: number) => realSetTimeout(fn, (ms ?? 0) >= CALL_TIMEOUT_MS ? 5 : ms)
    globalThis.setTimeout = shortened as unknown as typeof setTimeout
    const quiet = spyOn(console, "error").mockImplementation(() => {})
    try {
      const max = commands.register?.username?.max ?? 0
      expect(await api.register("u".repeat(max + 1))).toEqual({ ok: false, error: { code: "unknown" } })
      expect(await api.copyText("c".repeat((commands.copy_text?.text?.max ?? 0) + 1))).toBe(false)
      const tooLong = { language: "x".repeat(17) } as unknown as Parameters<LauncherApi["setSettings"]>[0]
      await expect(api.setSettings(tooLong)).rejects.toThrow("no answer")
      expect(dropped.map((call) => call.cmd)).toEqual(["register", "copy_text", "set_settings"])
    } finally {
      globalThis.setTimeout = realSetTimeout
      quiet.mockRestore()
    }
  })
})

async function handlerFor(event: string): Promise<number> {
  for (let i = 0; i < 50; i++) {
    const handler = listeners.get(event)
    if (handler !== undefined) return handler
    await Bun.sleep(1)
  }
  throw new Error(`nothing listens to ${event}`)
}

async function emit(event: string, payload: unknown): Promise<void> {
  internals().runCallback(await handlerFor(event), { event, id: 1, payload })
  await settle()
}

describe("events", () => {
  test("reach their listeners", async () => {
    const seen: unknown[] = []
    api.onSnapshot((snapshot) => seen.push(["snapshot", snapshot]))
    api.onHostKit((kit) => seen.push(["host-kit", kit]))
    api.onHostServer((view) => seen.push(["host-server", view]))
    api.onInviteLink(() => seen.push(["invite-link"]))
    api.onWindowState((maximized) => seen.push(["window-state", maximized]))

    await emit("snapshot", { phase: "join" })
    await emit("host-kit", { kit: 1 })
    await emit("host-server", { state: "running" })
    await emit("invite-link", null)
    await emit("window-state", true)
    await emit("window-state", false)
    await emit("window-state", "yes")
    await emit("window-state", { maximized: true })

    expect(seen).toEqual([
      ["snapshot", { phase: "join" }],
      ["host-kit", { kit: 1 }],
      ["host-server", { state: "running" }],
      ["invite-link"],
      ["window-state", true],
      ["window-state", false],
      ["window-state", false],
      ["window-state", false],
    ])
    expect(dropped).toEqual([])
  })

  test("progress arrives over the channel subscribe_progress is given", async () => {
    const seen: unknown[] = []
    api.onProgress((progress) => seen.push(progress))
    await settle()
    const subscribe = calls.find((call) => call.cmd === "subscribe_progress")
    const channel = String(subscribe?.args.onProgress)
    expect(channel).toMatch(/^__CHANNEL__:[0-9]{1,16}$/)
    const id = Number(channel.slice("__CHANNEL__:".length))
    const first = { kind: "download", totalBytes: 10, doneBytes: 1 }
    const second = { kind: "download", totalBytes: 10, doneBytes: 10 }
    internals().runCallback(id, { message: first, index: 0 })
    internals().runCallback(id, { message: second, index: 1 })
    expect(seen).toEqual([first, second])
    expect(dropped).toEqual([])
  })
})

describe("the window controls", () => {
  test("make Tauri's window calls in order, each allowed", async () => {
    await api.minimize()
    expect(await api.toggleMaximize()).toBe(true)
    await api.close()
    expect(calls).toEqual([
      { cmd: "plugin:window|minimize", args: { label: "main" } },
      { cmd: "plugin:window|toggle_maximize", args: { label: "main" } },
      { cmd: "plugin:window|is_maximized", args: { label: "main" } },
      { cmd: "plugin:window|close", args: { label: "main" } },
    ])
    expect(dropped).toEqual([])
  })
})
