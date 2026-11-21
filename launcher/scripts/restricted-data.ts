// SPDX-License-Identifier: MIT OR Apache-2.0
// Copies the bridge's generated command table from dainty-launcher-kit (crates/dainty-launcher-kit/generated, MIT since RL13), so
// the hook's table is never committed here. Also refreshes ui's bridge types, which api.test.ts imports. The name
// is the old one: restricted/ is no longer a source.
import { copyFileSync, existsSync, mkdirSync } from "node:fs"
import path from "node:path"

export const launcherDir = path.resolve(import.meta.dir, "..")
export const repoDir = path.resolve(launcherDir, "..")
export const generatedDir = path.join(repoDir, "crates", "dainty-launcher-kit", "generated")

// The MIT Tauri shell the release builds (restricted/launcher is only its oracle tests now)
export const appDir = path.join(repoDir, "crates", "dainty-launcher-app")

// Where `tauri build` in appDir leaves the NSIS bundles: the root workspace's target
export function bundleDir(): string {
  return process.env.E2E_BUNDLE_DIR || path.join(repoDir, "target", "release", "bundle", "nsis")
}

function main(): void {
  const from = path.join(generatedDir, "commands.js")
  if (!existsSync(from)) {
    console.error(`missing ${from}: run RR_UPDATE_GENERATED=1 cargo test -p dainty-launcher-kit generated`)
    process.exit(1)
  }
  mkdirSync(path.join(launcherDir, "isolation"), { recursive: true })
  copyFileSync(from, path.join(launcherDir, "isolation", "commands.js"))

  const ui = Bun.spawnSync([process.execPath, "run", "restricted-data"], {
    cwd: path.join(repoDir, "ui"),
    stdio: ["ignore", "inherit", "inherit"],
  })
  if (ui.exitCode !== 0) process.exit(ui.exitCode ?? 1)
}

if (import.meta.main) main()
