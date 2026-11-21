// SPDX-License-Identifier: MIT OR Apache-2.0
// Silent installs of the NSIS bundles Tauri builds.
import { spawnSync } from "node:child_process"
import { existsSync, readdirSync, readFileSync } from "node:fs"
import path from "node:path"

import { restrictedDir } from "../../scripts/restricted-data"

// NSIS takes /D= unquoted and last, so the command line is written by hand rather than quoted per argument
function runVerbatim(exe: string, args: string): number | null {
  const child = spawnSync(exe, [args], { argv0: `"${exe}"`, windowsVerbatimArguments: true, stdio: "inherit" })
  if (child.error) throw child.error
  return child.status
}

export function install(installer: string, dir: string, exe: string): string {
  const status = runVerbatim(installer, `/S /D=${dir}`)
  if (status !== 0) throw new Error(`${path.basename(installer)} exited ${status}`)
  const installed = path.join(dir, exe)
  if (!existsSync(installed)) throw new Error(`${path.basename(installer)} left no ${installed}`)
  return installed
}

// _?= runs the uninstaller in place and waits for it; it also removes the Add/Remove Programs entry
export function uninstall(dir: string): void {
  const uninstaller = path.join(dir, "uninstall.exe")
  if (!existsSync(uninstaller)) return
  try {
    runVerbatim(uninstaller, `/S _?=${dir}`)
  } catch {
    // Best effort: the temporary root goes anyway
  }
}

function bundleVersion(): string {
  const tauriDir = path.join(restrictedDir(), "launcher", "src-tauri")
  const config = JSON.parse(readFileSync(path.join(tauriDir, "tauri.conf.json"), "utf8")) as { version?: unknown }
  if (typeof config.version === "string" && /^\d/.test(config.version)) return config.version
  // Without a version in the config, Tauri takes the crate's
  const manifest = readFileSync(path.join(tauriDir, "Cargo.toml"), "utf8")
  const pkg = manifest.split(/^\[/m).find((section) => section.startsWith("package]")) ?? ""
  const version = /^version\s*=\s*"([^"]+)"/m.exec(pkg)?.[1]
  if (version === undefined) throw new Error(`no version in ${tauriDir}'s tauri.conf.json or Cargo.toml`)
  return version
}

function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
}

export function installer(bundleDir: string, productName: string): string {
  const pattern = new RegExp(`^${escapeRegExp(`${productName}_${bundleVersion()}_x64-setup.exe`)}$`)
  const found = existsSync(bundleDir) ? readdirSync(bundleDir).filter((name) => pattern.test(name)) : []
  if (found.length !== 1) {
    throw new Error(`expected one ${productName} installer in ${bundleDir}, found ${found.length}`)
  }
  return path.join(bundleDir, found[0] as string)
}
