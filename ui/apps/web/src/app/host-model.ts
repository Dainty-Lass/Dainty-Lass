// What the server screens show, as pure functions over the bridge's types so the decisions are tested without a
// DOM (host-model.test.ts): the Host page's checks and tabs, the component status, and the module panels, which
// are drawn from the manifest (DD11). Every string is a catalog key.
import type {
  ExternalTarget,
  HostComponentPhase,
  HostJobKind,
  HostKit,
  HostServerComponent,
  HostServerState,
  ModuleCommandView,
  ModuleInput,
  ModuleOrigin,
  ModulePaletteEntry,
  ModuleRole,
  ModuleRunState,
  ModuleSectionView,
  ModuleSettingValue,
  ModuleSettingsPatch,
  ModuleThemeView,
  ModuleView,
  RequirementId,
  Snapshot,
} from "@workspace/bridge"
import { isKey, type Key } from "@workspace/i18n"

// ------------------------------------------------------------------ the page and its tabs

// The Host page: in the server GUI always, in a player build when the player turned Host tools on
export function hostPageAvailable(snap: Snapshot | null): boolean {
  return (
    snap !== null &&
    snap.app.hostToolsAvailable &&
    (snap.app.serverGui || snap.settings.hostTools)
  )
}

export type HostTab =
  "server" | "status" | "modules" | "gamedata" | "build" | "requirements"

export const HOST_TAB_KEYS: Record<HostTab, Key> = {
  server: "host_tab_server",
  status: "host_tab_status",
  modules: "host_tab_modules",
  gamedata: "host_tab_gamedata",
  build: "host_tab_build",
  requirements: "host_tab_requirements",
}

// The component status and the module panels exist only where the server runs inside this window
export function hostTabs(snap: Snapshot | null): HostTab[] {
  const tabs: HostTab[] = ["server"]
  if (snap?.app.serverGui) tabs.push("status", "modules")
  tabs.push("gamedata", "build", "requirements")
  return tabs
}

// ------------------------------------------------------------------ the server

export const SERVER_STATE_KEYS: Record<
  HostServerState,
  { state: Key; text: Key }
> = {
  stopped: { state: "hsrv_state_stopped", text: "hsrv_text_stopped" },
  starting: { state: "hsrv_state_starting", text: "hsrv_text_starting" },
  running: { state: "hsrv_state_running", text: "hsrv_text_running" },
  stopping: { state: "hsrv_state_stopping", text: "hsrv_text_stopping" },
  external: { state: "hsrv_state_external", text: "hsrv_text_external" },
  failed: { state: "hsrv_state_failed", text: "hsrv_text_failed_noreason" },
}

export type CheckId = "workspace" | "setup" | "gamedata" | "runtime"

export interface Check {
  id: CheckId
  key: Key
  ok: boolean
  // Done, but worth a word: example game data starts a server nobody can play on
  warn: boolean
}

// "Before you start", from what the kit found on this PC
export function checks(kit: HostKit): Check[] {
  const { backend, director } = kit.gameData
  const real = backend === "real" && director === "real"
  const have = backend !== "missing" && director !== "missing"
  return [
    {
      id: "workspace",
      key: "hsrv_check_workspace",
      ok: kit.workspaceOk,
      warn: false,
    },
    {
      id: "setup",
      key: "hsrv_check_setup",
      ok: kit.serverConfig.configured,
      warn: false,
    },
    {
      id: "gamedata",
      key:
        have && !real ? "hsrv_check_gamedata_example" : "hsrv_check_gamedata",
      ok: have,
      warn: have && !real,
    },
    {
      id: "runtime",
      key: "hsrv_check_runtime",
      ok: kit.builds.published,
      warn: false,
    },
  ]
}

// The server starts when the project, its setup and some game data are there; a missing runtime only keeps
// friends from playing, so it doesn't stop the start
export function canStart(kit: HostKit): boolean {
  return checks(kit)
    .filter((check) => check.id !== "runtime")
    .every((check) => check.ok)
}

export const JOB_KEYS: Record<HostJobKind, Key> = {
  inject: "host_job_inject",
  build_runtime: "host_job_build_runtime",
  build_loader: "host_job_build_loader",
  build_exporter: "host_job_build_exporter",
  import_sdk: "host_job_import_sdk",
  copy_exports: "host_job_copy_exports",
  synthetic_data: "host_job_synthetic_data",
  import_gamedata: "host_job_import_gamedata",
  publish_runtime: "host_job_publish_runtime",
  setup: "host_job_setup",
  generate_gamedata: "host_job_generate_gamedata",
  export_gamedata: "host_job_export_gamedata",
  create_invite: "host_job_create_invite",
  update: "host_job_update",
  multipliers: "host_job_multipliers",
  upstream: "host_job_upstream",
  download_project: "host_job_download_project",
}

// A multiplier field: 1 to 100, as the backend takes them
export function multiplier(text: string): number | null {
  const value = Number(text.trim())
  return text.trim() !== "" &&
    Number.isFinite(value) &&
    value >= 1 &&
    value <= 100
    ? value
    : null
}

// ------------------------------------------------------------------ the status page

export const PHASE_KEYS: Record<HostComponentPhase, Key> = {
  stopped: "hstat_phase_stopped",
  starting: "hstat_phase_starting",
  ready: "hstat_phase_ready",
  stopping: "hstat_phase_stopping",
  failed: "hstat_phase_failed",
  disabled: "hstat_phase_disabled",
}

export type Tone = "good" | "busy" | "bad" | "idle"

export function phaseTone(phase: HostComponentPhase): Tone {
  switch (phase) {
    case "ready":
      return "good"
    case "starting":
    case "stopping":
      return "busy"
    case "failed":
      return "bad"
    default:
      return "idle"
  }
}

export interface StatusRow {
  component: HostServerComponent
  // A game module's id is also a component of the server (DF3): its card is on the Modules tab
  module: ModuleView | null
}

export function statusRows(
  components: HostServerComponent[],
  modules: ModuleView[]
): StatusRow[] {
  return components.map((component) => ({
    component,
    module: modules.find((m) => m.id === component.name) ?? null,
  }))
}

export function readyCount(components: HostServerComponent[]): {
  ready: number
  total: number
} {
  const live = components.filter((c) => c.phase !== "disabled")
  return {
    ready: live.filter((c) => c.phase === "ready").length,
    total: live.length,
  }
}

// ------------------------------------------------------------------ module panels

export const MODULE_STATE_KEYS: Record<ModuleRunState, Key> = {
  not_loaded: "mod_state_not_loaded",
  starting: "mod_state_starting",
  ready: "mod_state_ready",
  failed: "mod_state_failed",
}

export const MODULE_ROLE_KEYS: Record<ModuleRole, Key> = {
  game: "mod_role_game",
  plugin: "mod_role_plugin",
}

export const MODULE_ORIGIN_KEYS: Record<ModuleOrigin, Key> = {
  compiled_in: "mod_origin_compiled_in",
  external: "mod_origin_external",
}

export function moduleTone(state: ModuleRunState): Tone {
  switch (state) {
    case "ready":
      return "good"
    case "starting":
      return "busy"
    case "failed":
      return "bad"
    default:
      return "idle"
  }
}

export interface ModuleButtons {
  load: boolean
  unload: boolean
  restart: boolean
}

// A failed module is loaded again to restart it (the registry unloads it first); one that is starting waits
export function moduleButtons(state: ModuleRunState): ModuleButtons {
  return {
    load: state === "not_loaded" || state === "failed",
    unload: state === "ready" || state === "failed",
    restart: state === "ready",
  }
}

export function modulePorts(
  module: ModuleView
): { name: string; transport: string; range: string }[] {
  return module.ports.map((port) => ({
    name: port.name,
    transport: port.transport.toUpperCase(),
    range:
      port.first === port.last
        ? String(port.first)
        : `${port.first}-${port.last}`,
  }))
}

// The capability lines a module's manifest earns, as catalog keys with what fills them
export function capabilityLines(
  module: ModuleView
): { key: Key; vars?: Record<string, string> }[] {
  const caps = module.capabilities
  const lines: { key: Key; vars?: Record<string, string> }[] = []
  if (caps.realms) lines.push({ key: "mod_cap_realms" })
  if (caps.network) lines.push({ key: "mod_cap_network" })
  if (caps.chat) lines.push({ key: "mod_cap_chat" })
  if (caps.patchSet) {
    lines.push(
      caps.patchSet.provides
        ? {
            key: "mod_cap_patch_provides",
            vars: { name: caps.patchSet.displayName ?? module.id },
          }
        : { key: "mod_cap_patch_accepts" }
    )
  }
  return lines
}

// A command's form: every argument the manifest names, blank
export function blankInput(command: ModuleCommandView): Record<string, string> {
  return Object.fromEntries(command.args.map((arg) => [arg.name, ""]))
}

export interface InputProblem {
  reason: "missing_argument" | "argument_gap"
  name: string
}

// What the backend would refuse, found before the call: a required argument left blank, or an optional one
// left blank before a filled one (a module reads positions, not names)
export function inputProblem(
  command: ModuleCommandView,
  values: Record<string, string>
): InputProblem | null {
  let gap: string | null = null
  for (const arg of command.args) {
    const text = values[arg.name] ?? ""
    if (text === "") {
      if (arg.required) return { reason: "missing_argument", name: arg.name }
      gap ??= arg.name
    } else if (gap !== null) {
      return { reason: "argument_gap", name: gap }
    }
  }
  return null
}

export function commandInput(
  command: ModuleCommandView,
  values: Record<string, string>
): ModuleInput {
  const input: ModuleInput = {}
  for (const arg of command.args) {
    const text = values[arg.name] ?? ""
    if (text !== "") input[arg.name] = text
  }
  return input
}

export type SettingKind = "flag" | "number" | "text"

export function settingKind(value: ModuleSettingValue): SettingKind {
  return typeof value === "boolean"
    ? "flag"
    : typeof value === "number"
      ? "number"
      : "text"
}

// What an edit box holds for a setting
export function settingText(value: ModuleSettingValue): string {
  return String(value)
}

export function settingId(section: string, key: string): string {
  return `${section}.${key}`
}

export type PatchResult =
  { ok: true; patch: ModuleSettingsPatch } | { ok: false; id: string }

// The values the editor changed, in the shape the backend saves; not ok (with the setting's id) when a
// number box holds no number. `edits` holds each box's text (a switch's, "true" or "false"), by `section.key`.
export function settingsPatch(
  sections: ModuleSectionView[],
  edits: Record<string, string>
): PatchResult {
  const patch: ModuleSettingsPatch = {}
  for (const section of sections) {
    for (const key of section.keys) {
      const text = edits[settingId(section.name, key.name)]
      if (text === undefined || text === settingText(key.value)) continue
      let value: boolean | number | string
      switch (settingKind(key.default)) {
        case "flag":
          value = text === "true"
          break
        case "number": {
          const number = Number(text)
          if (text.trim() === "" || !Number.isFinite(number))
            return { ok: false, id: settingId(section.name, key.name) }
          value = number
          break
        }
        default:
          value = text
      }
      ;(patch[section.name] ??= {})[key.name] = value
    }
  }
  return { ok: true, patch }
}

// ------------------------------------------------------------------ a plugin's look (DD25)

// The backend already drops any other value; this is the page's own check before a colour reaches a style
const COLOUR =
  /^(#(?:[0-9a-fA-F]{3,4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})|(?:rgba?|hsla?|oklch|oklab|lab|lch)\([0-9a-zA-Z .,%/+-]{1,56}\))$/
const TOKEN = /^[a-z][a-z0-9-]{0,31}$/

export function safePalette(
  palette: ModulePaletteEntry[]
): ModulePaletteEntry[] {
  return palette.filter((e) => TOKEN.test(e.name) && COLOUR.test(e.value))
}

export interface ActiveTheme {
  module: string
  moduleName: string
  theme: ModuleThemeView
}

// The look of the first loaded plugin that brings one
export function activeTheme(modules: ModuleView[]): ActiveTheme | null {
  for (const module of modules) {
    const theme = module.capabilities.theme
    if (module.role === "plugin" && module.state === "ready" && theme) {
      return {
        module: module.id,
        moduleName: module.displayName ?? module.id,
        theme: { ...theme, palette: safePalette(theme.palette) },
      }
    }
  }
  return null
}

// Sets the palette as CSS variables on one element (through the CSSOM, which the page's CSP allows); the
// returned function puts it back
export function applyPalette(
  element: HTMLElement,
  palette: ModulePaletteEntry[]
): () => void {
  const names = safePalette(palette).map((entry) => {
    element.style.setProperty(`--${entry.name}`, entry.value)
    return `--${entry.name}`
  })
  return () => names.forEach((name) => element.style.removeProperty(name))
}

// ------------------------------------------------------------------ requirements

export const REQUIREMENT_GROUPS: { key: Key; ids: RequirementId[] }[] = [
  { key: "hreq_group_host", ids: ["git", "node", "mongodb", "tailscale"] },
  { key: "hreq_group_build", ids: ["visual_studio", "rust", "dumper7"] },
  { key: "hreq_group_play", ids: ["dauntless", "vc_redist"] },
]

export const REQUIREMENT_KEYS: Record<RequirementId, { name: Key; why: Key }> =
  {
    dauntless: { name: "req_dauntless_name", why: "req_dauntless_why" },
    git: { name: "req_git_name", why: "req_git_why" },
    node: { name: "req_node_name", why: "req_node_why" },
    mongodb: { name: "req_mongodb_name", why: "req_mongodb_why" },
    tailscale: { name: "req_tailscale_name", why: "req_tailscale_why" },
    visual_studio: {
      name: "req_visual_studio_name",
      why: "req_visual_studio_why",
    },
    rust: { name: "req_rust_name", why: "req_rust_why" },
    dumper7: { name: "req_dumper7_name", why: "req_dumper7_why" },
    vc_redist: { name: "req_vc_redist_name", why: "req_vc_redist_why" },
  }

// Where each requirement is downloaded, the first link being the main one
export const REQUIREMENT_LINKS: Record<RequirementId, ExternalTarget[]> = {
  dauntless: ["installing_guide"],
  git: ["git"],
  node: ["nodejs"],
  mongodb: ["mongodb"],
  tailscale: ["tailscale_download"],
  visual_studio: ["visual_studio"],
  rust: ["rust"],
  dumper7: ["dumper7"],
  vc_redist: ["vc_redist"],
}

// The "how to install" steps a requirement has in the catalog: `req_<id>_s1`, `_s2`...
export function requirementSteps(id: RequirementId): Key[] {
  const steps: Key[] = []
  for (let n = 1; ; n++) {
    const key = `req_${id}_s${n}`
    if (!isKey(key)) return steps
    steps.push(key)
  }
}
