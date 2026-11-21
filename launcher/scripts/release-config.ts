// SPDX-License-Identifier: MIT OR Apache-2.0
// Writes the overlays the release builds pass to `tauri build --config`: client.json for the launcher, and
// manager.json, the server manager's own config with the same overlay on top.
import { mkdirSync, readFileSync, writeFileSync } from "node:fs"
import path from "node:path"

import { restrictedDir } from "./restricted-data"

export type Json = null | boolean | number | string | Json[] | { [key: string]: Json }
type JsonObject = { [key: string]: Json }

const USAGE = "usage: bun scripts/release-config.ts [<out dir>] [--tauri-dir <dir>]"

function isObject(value: Json | undefined): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value)
}

// RFC 7386, which is how Tauri applies --config: objects merge key by key, null deletes, anything else replaces
export function mergePatch(target: Json | undefined, patch: Json): Json {
  if (!isObject(patch)) return patch
  const out: JsonObject = isObject(target) ? { ...target } : {}
  for (const [key, value] of Object.entries(patch)) {
    if (value === null) delete out[key]
    else out[key] = mergePatch(out[key], value)
  }
  return out
}

function readJson(file: string): Json {
  return JSON.parse(readFileSync(file, "utf8")) as Json
}

function writeJson(file: string, value: Json): void {
  writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`)
}

function usage(): never {
  console.error(USAGE)
  process.exit(2)
}

function parseArgs(argv: string[]): { outDir: string; tauriDir: string } {
  let outDir: string | undefined
  let tauriDir: string | undefined
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i] ?? ""
    if (arg === "--tauri-dir") {
      tauriDir = argv[++i]
      if (tauriDir === undefined) usage()
    } else if (!arg.startsWith("--") && outDir === undefined) {
      outDir = arg
    } else {
      usage()
    }
  }
  return {
    outDir: path.resolve(outDir ?? "."),
    tauriDir: path.resolve(tauriDir ?? path.join(restrictedDir(), "launcher", "src-tauri")),
  }
}

function updaterPubkey(config: Json): unknown {
  const plugins = isObject(config) ? config.plugins : undefined
  const updater = isObject(plugins) ? plugins.updater : undefined
  return isObject(updater) ? updater.pubkey : undefined
}

function main(): void {
  const { outDir, tauriDir } = parseArgs(process.argv.slice(2))
  const updater = (process.env.TAURI_SIGNING_PRIVATE_KEY ?? "") !== ""
  const codeSigning = (process.env.WINDOWS_CERTIFICATE_PATH ?? "") !== ""
  const bundle: JsonObject = {}

  if (updater) {
    const pubkey = updaterPubkey(readJson(path.join(tauriDir, "tauri.conf.json")))
    // A signed feed the launcher can't verify would strand every install (RD7)
    if (typeof pubkey !== "string" || pubkey.trim() === "") {
      console.error("::error::TAURI_SIGNING_PRIVATE_KEY is set but tauri.conf.json has no updater pubkey")
      process.exit(1)
    }
    bundle.createUpdaterArtifacts = true
  }
  if (codeSigning) {
    bundle.windows = {
      signCommand: { cmd: "bun", args: [path.join(import.meta.dir, "sign-windows.ts"), "%1"] },
    }
  }

  const overlay: JsonObject = { bundle }
  const manager = mergePatch(readJson(path.join(tauriDir, "tauri.server-gui.conf.json")), overlay)
  mkdirSync(outDir, { recursive: true })
  writeJson(path.join(outDir, "client.json"), overlay)
  writeJson(path.join(outDir, "manager.json"), manager)
  const state = (on: boolean) => (on ? "on" : "off")
  console.log(`updater signatures: ${state(updater)}; Windows code signing: ${state(codeSigning)}`)
}

if (import.meta.main) main()
