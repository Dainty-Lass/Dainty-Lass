/// <reference types="bun" />
// The server screens' decisions: the Host page's checks and tabs, the component status, and what a module panel
// draws and sends, from the manifest alone (DD11)
import { expect, test } from "bun:test"

import type {
  HostKit,
  HostServerComponent,
  ModuleCommandView,
  ModuleView,
  Snapshot,
} from "@workspace/bridge"
import { CATALOGS, isKey, LANGUAGES } from "@workspace/i18n"

import {
  activeTheme,
  blankInput,
  canStart,
  capabilityLines,
  checks,
  commandInput,
  HOST_TAB_KEYS,
  hostPageAvailable,
  hostTabs,
  inputProblem,
  JOB_KEYS,
  MODULE_ORIGIN_KEYS,
  MODULE_ROLE_KEYS,
  MODULE_STATE_KEYS,
  moduleButtons,
  modulePorts,
  multiplier,
  PHASE_KEYS,
  phaseTone,
  readyCount,
  REQUIREMENT_GROUPS,
  REQUIREMENT_KEYS,
  requirementSteps,
  safePalette,
  SERVER_STATE_KEYS,
  settingsPatch,
  statusRows,
} from "../src/app/host-model"
import { errorMessage } from "../src/app/model"

function app(over: Partial<Snapshot["app"]>, hostTools = false): Snapshot {
  return {
    app: {
      version: "0.1.0",
      packaged: false,
      updateReady: false,
      platform: "win32",
      hostToolsAvailable: true,
      serverGui: false,
      legacyAvailable: false,
      gameVersions: ["1.14.7"],
      ...over,
    },
    settings: { hostTools },
  } as Snapshot
}

function kit(over: Partial<HostKit> = {}): HostKit {
  return {
    workspace: "C:/Dauntless/Reforged",
    workspaceOk: true,
    upstream: { present: true, patched: true },
    sdk: { present: true, functionFiles: 12 },
    dumps: [],
    dumperDll: null,
    builds: { runtime: true, loader: true, exporter: true, published: true },
    toolchain: { visualStudio: "VS 2022", msbuild: true, cargo: true },
    gameData: { backend: "real", director: "real" },
    exports: { present: true, inGame: true },
    serverConfig: {
      configured: true,
      name: "Friday Hunts",
      mode: "private",
      host: "100.64.0.1",
      multipliers: null,
    },
    requirements: [],
    job: null,
    update: null,
    projectTarget: null,
    ...over,
  }
}

function module(over: Partial<ModuleView> = {}): ModuleView {
  return {
    id: "stub",
    displayName: "Stub",
    role: "game",
    gameVersion: "0.0.1",
    origin: "compiled_in",
    autoload: true,
    state: "ready",
    detail: null,
    unverified: true,
    ports: [
      { name: "http", transport: "tcp", first: 65001, last: 65001 },
      { name: "game", transport: "udp", first: 65010, last: 65019 },
    ],
    capabilities: {
      realms: false,
      network: true,
      chat: true,
      patchSet: { provides: false, displayName: null },
      theme: null,
    },
    commands: [],
    settings: [],
    ...over,
  }
}

const give: ModuleCommandView = {
  name: "give",
  summary: "Gives an item",
  args: [
    { name: "account", required: true, help: "Who" },
    { name: "item", required: false, help: "What" },
    { name: "count", required: false, help: "How many" },
  ],
}

test("the Host page is the server GUI's, or a player build's once Host tools are on", () => {
  expect(hostPageAvailable(null)).toBe(false)
  expect(hostPageAvailable(app({}))).toBe(false)
  expect(hostPageAvailable(app({}, true))).toBe(true)
  expect(hostPageAvailable(app({ serverGui: true }))).toBe(true)
  expect(hostPageAvailable(app({ hostToolsAvailable: false }, true))).toBe(
    false
  )
})

test("the status and module tabs exist only where the server runs in this window", () => {
  expect(hostTabs(app({}, true))).toEqual([
    "server",
    "gamedata",
    "build",
    "requirements",
  ])
  expect(hostTabs(app({ serverGui: true }))).toEqual([
    "server",
    "status",
    "modules",
    "gamedata",
    "build",
    "requirements",
  ])
  for (const id of hostTabs(app({ serverGui: true }))) {
    expect(isKey(HOST_TAB_KEYS[id])).toBe(true)
  }
})

test("before you start: the project, the setup and some game data; a missing runtime only warns", () => {
  expect(canStart(kit())).toBe(true)
  expect(canStart(kit({ builds: { ...kit().builds, published: false } }))).toBe(
    true
  )
  expect(canStart(kit({ workspaceOk: false }))).toBe(false)
  expect(
    canStart(
      kit({ serverConfig: { ...kit().serverConfig, configured: false } })
    )
  ).toBe(false)
  expect(
    canStart(kit({ gameData: { backend: "real", director: "missing" } }))
  ).toBe(false)
  const example = checks(
    kit({ gameData: { backend: "synthetic", director: "synthetic" } })
  ).find((c) => c.id === "gamedata")
  expect(example).toEqual({
    id: "gamedata",
    key: "hsrv_check_gamedata_example",
    ok: true,
    warn: true,
  })
  expect(checks(kit()).every((c) => c.ok && !c.warn)).toBe(true)
})

test("multipliers are 1 to 100", () => {
  expect(multiplier("1")).toBe(1)
  expect(multiplier(" 12.5 ")).toBe(12.5)
  expect(multiplier("100")).toBe(100)
  for (const bad of ["", "0", "0.9", "101", "x", "NaN", "Infinity"]) {
    expect(multiplier(bad), bad).toBeNull()
  }
})

test("a component is ready, busy, failed or idle, and a module's id is a component too", () => {
  expect(phaseTone("ready")).toBe("good")
  expect(phaseTone("starting")).toBe("busy")
  expect(phaseTone("stopping")).toBe("busy")
  expect(phaseTone("failed")).toBe("bad")
  expect(phaseTone("stopped")).toBe("idle")
  expect(phaseTone("disabled")).toBe("idle")
  const components: HostServerComponent[] = [
    { name: "gateway", phase: "ready", port: 443, detail: "" },
    { name: "stub", phase: "ready", port: null, detail: "" },
    { name: "director", phase: "failed", port: null, detail: "no data" },
    { name: "admin", phase: "disabled", port: null, detail: "" },
  ]
  expect(readyCount(components)).toEqual({ ready: 2, total: 3 })
  const rows = statusRows(components, [module()])
  expect(rows.map((r) => r.module?.id ?? null)).toEqual([
    null,
    "stub",
    null,
    null,
  ])
})

test("a module's buttons follow its state", () => {
  expect(moduleButtons("not_loaded")).toEqual({
    load: true,
    unload: false,
    restart: false,
  })
  expect(moduleButtons("ready")).toEqual({
    load: false,
    unload: true,
    restart: true,
  })
  // A failed module is loaded again to restart it
  expect(moduleButtons("failed")).toEqual({
    load: true,
    unload: true,
    restart: false,
  })
  expect(moduleButtons("starting")).toEqual({
    load: false,
    unload: false,
    restart: false,
  })
})

test("a module panel lists its ports and the capabilities its manifest earns", () => {
  expect(modulePorts(module())).toEqual([
    { name: "http", transport: "TCP", range: "65001" },
    { name: "game", transport: "UDP", range: "65010-65019" },
  ])
  expect(capabilityLines(module()).map((l) => l.key)).toEqual([
    "mod_cap_network",
    "mod_cap_chat",
    "mod_cap_patch_accepts",
  ])
  const plugin = module({
    capabilities: {
      realms: true,
      network: false,
      chat: false,
      patchSet: { provides: true, displayName: "Freeza's tuning" },
      theme: null,
    },
  })
  expect(capabilityLines(plugin)).toEqual([
    { key: "mod_cap_realms" },
    { key: "mod_cap_patch_provides", vars: { name: "Freeza's tuning" } },
  ])
})

test("a command's form is drawn from its arguments and sends only what was filled", () => {
  expect(blankInput(give)).toEqual({ account: "", item: "", count: "" })
  const values = { account: "a1", item: "", count: "" }
  expect(inputProblem(give, values)).toBeNull()
  expect(commandInput(give, values)).toEqual({ account: "a1" })
  expect(
    commandInput(give, { account: "a1", item: "sword", count: "" })
  ).toEqual({ account: "a1", item: "sword" })
  expect(inputProblem(give, { account: "", item: "", count: "" })).toEqual({
    reason: "missing_argument",
    name: "account",
  })
  // The backend reads positions: a blank argument before a filled one is refused
  expect(inputProblem(give, { account: "a1", item: "", count: "3" })).toEqual({
    reason: "argument_gap",
    name: "item",
  })
  expect(blankInput({ ...give, args: [] })).toEqual({})
  expect(inputProblem({ ...give, args: [] }, {})).toBeNull()
})

const sections: ModuleView["settings"] = [
  {
    name: "game",
    keys: [
      {
        name: "greeting",
        help: "What echo starts with",
        default: "hello",
        value: "hi",
      },
      { name: "slots", help: "Hunt slots", default: 8, value: 8 },
      { name: "pvp", help: "Player fights", default: false, value: false },
    ],
  },
]

test("settings send only what changed, in the type of the key's default", () => {
  expect(settingsPatch(sections, {})).toEqual({ ok: true, patch: {} })
  expect(settingsPatch(sections, { "game.greeting": "hi" })).toEqual({
    ok: true,
    patch: {},
  })
  expect(
    settingsPatch(sections, {
      "game.greeting": "hey",
      "game.slots": "12",
      "game.pvp": "true",
    })
  ).toEqual({
    ok: true,
    patch: { game: { greeting: "hey", slots: 12, pvp: true } },
  })
  expect(settingsPatch(sections, { "game.slots": "many" })).toEqual({
    ok: false,
    id: "game.slots",
  })
  expect(settingsPatch(sections, { "game.slots": "" })).toEqual({
    ok: false,
    id: "game.slots",
  })
})

test("only a loaded plugin's look is drawn, and only with colours that can't carry anything else", () => {
  const theme = {
    id: "freeza",
    displayName: "Freeza",
    palette: [
      { name: "primary", value: "#ff8800" },
      { name: "background", value: "oklch(0.2 0.02 260)" },
      { name: "Bad", value: "#fff" },
      { name: "ring", value: "url(https://x.test/a.png)" },
      { name: "accent", value: "rgb(1,2,3);color:red" },
    ],
    layout: null,
    logo: null,
  }
  expect(safePalette(theme.palette).map((e) => e.name)).toEqual([
    "primary",
    "background",
  ])
  const caps = {
    realms: false,
    network: false,
    chat: false,
    patchSet: null,
    theme,
  }
  const plugin = module({ id: "freeza", role: "plugin", capabilities: caps })
  expect(activeTheme([module()])).toBeNull()
  expect(activeTheme([module({ ...plugin, state: "not_loaded" })])).toBeNull()
  // A game module with a theme is not a plugin's look
  expect(activeTheme([module({ capabilities: caps })])).toBeNull()
  const active = activeTheme([module(), plugin])
  expect(active?.module).toBe("freeza")
  expect(active?.theme.palette.map((e) => e.name)).toEqual([
    "primary",
    "background",
  ])
})

test("the requirements have their words in the catalog, steps included", () => {
  const ids = REQUIREMENT_GROUPS.flatMap((g) => g.ids)
  expect(new Set(ids).size).toBe(9)
  for (const id of ids) {
    expect(isKey(REQUIREMENT_KEYS[id].name), id).toBe(true)
    expect(requirementSteps(id).length, id).toBeGreaterThan(0)
  }
  expect(requirementSteps("git")).toEqual([
    "req_git_s1",
    "req_git_s2",
    "req_git_s3",
  ])
})

test("every state, phase, job and module word the screens use is in both languages", () => {
  const keys = [
    ...Object.values(SERVER_STATE_KEYS).flatMap((k) => [k.state, k.text]),
    ...Object.values(PHASE_KEYS),
    ...Object.values(JOB_KEYS),
    ...Object.values(MODULE_STATE_KEYS),
    ...Object.values(MODULE_ROLE_KEYS),
    ...Object.values(MODULE_ORIGIN_KEYS),
  ]
  for (const lang of LANGUAGES) {
    for (const key of keys)
      expect(CATALOGS[lang][key], `${lang}.${key}`).toBeTruthy()
  }
})

test("a module's refusal says which argument or setting, in the player's words", () => {
  const say = (code: string, detail?: string) =>
    errorMessage({ code, detail } as never, null)
  expect(say("module_bad_state", "loaded")).toEqual({ key: "mod_err_loaded" })
  expect(say("module_bad_state", "not_loaded")).toEqual({
    key: "mod_err_not_loaded",
  })
  expect(say("module_bad_state")).toEqual({ key: "err_module_bad_state" })
  expect(say("module_input_invalid", "missing_argument:account")).toEqual({
    key: "mod_err_input_missing_argument",
    vars: { name: "account" },
  })
  expect(say("module_input_invalid", "json")).toEqual({
    key: "mod_err_input_json",
    vars: { name: "" },
  })
  // A reason that needs a name the backend couldn't give, or one with no sentence, gets the code's own
  expect(say("module_input_invalid", "unknown_argument")).toEqual({
    key: "err_module_input_invalid",
  })
  expect(say("module_input_invalid", "from_the_future:x")).toEqual({
    key: "err_module_input_invalid",
  })
  expect(say("module_settings_invalid", "type:game.slots")).toEqual({
    key: "mod_err_settings_type",
    vars: { name: "game.slots" },
  })
  expect(say("module_settings_invalid", "unknown_section").key).toBe(
    "mod_err_settings_unknown_section"
  )
  expect(say("module_failed", "host_toml")).toEqual({
    key: "mod_err_failed_host_toml",
  })
  expect(say("module_failed", "it broke")).toEqual({
    key: "err_module_failed",
    vars: { detail: "it broke" },
  })
  expect(say("module_unknown", "stub")).toEqual({ key: "err_module_unknown" })
  expect(say("server_stopped")).toEqual({ key: "err_server_stopped" })
})
