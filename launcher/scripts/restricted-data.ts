// SPDX-License-Identifier: MIT
// Copies the bridge's generated command table from dainty-launcher-kit (crates/dainty-launcher-kit/generated, MIT since RL13), so
// the hook's table is never committed here. Also refreshes ui's bridge types, which api.test.ts imports. The name
// is the old one: restricted/ is no longer a source, but still where restricted's bundles are (restrictedDir).
import { copyFileSync, existsSync, mkdirSync } from "node:fs"
import path from "node:path"

export const launcherDir = path.resolve(import.meta.dir, "..")
export const repoDir = path.resolve(launcherDir, "..")
export const generatedDir = path.join(repoDir, "crates", "dainty-launcher-kit", "generated")

// The same rule as the e2e tests and release-config.ts use for restricted's bundles
export function restrictedDir(): string {
  return process.env.DAINTY_RESTRICTED_DIR || path.resolve(repoDir, "restricted")
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
