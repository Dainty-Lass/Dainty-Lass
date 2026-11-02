// SPDX-License-Identifier: MIT
// Loads the isolation frame's two scripts as the frame does, in a realm of their own: nothing of the test's globals
// is visible to them, so the hook can only rely on what commands.js gives it.
import { readFileSync } from "node:fs"
import path from "node:path"
import vm from "node:vm"

export interface Shape {
  kind: string
  max?: number
  min?: number
  names?: string[]
  nullable?: boolean
  fields?: Record<string, Shape>
}

export type Commands = Record<string, Record<string, Shape>>

export interface Hook {
  hook: (payload: unknown) => unknown
  commands: Commands
  settingsKeys: string[]
}

const isolation = path.resolve(import.meta.dir, "../isolation")

export function loadHook(): Hook {
  const window: Record<string, unknown> = {}
  const context = vm.createContext({ window })
  for (const file of ["commands.js", "hook.js"]) {
    const where = path.join(isolation, file)
    vm.runInContext(readFileSync(where, "utf8"), context, { filename: where })
  }
  return {
    hook: window.__TAURI_ISOLATION_HOOK__ as Hook["hook"],
    commands: window.__RR_COMMANDS__ as Commands,
    settingsKeys: window.__RR_SETTINGS_KEYS__ as string[],
  }
}
