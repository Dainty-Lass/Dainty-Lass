// SPDX-License-Identifier: MIT
// Copies the generated command table from a dainty-restricted checkout: the bridge it is generated from is AGPL,
// so the table is never committed here (RD27). Also refreshes ui's bridge types, which api.test.ts imports.
import { copyFileSync, existsSync, mkdirSync } from "node:fs"
import path from "node:path"

export const launcherDir = path.resolve(import.meta.dir, "..")
export const repoDir = path.resolve(launcherDir, "..")

// The same rule as ui/packages/bridge/scripts/restricted-data.ts, so both copy from one checkout
export function restrictedDir(): string {
  return process.env.DAINTY_RESTRICTED_DIR || path.resolve(repoDir, "../dainty-restricted")
}

function main(): void {
  const restricted = restrictedDir()
  const from = path.join(restricted, "launcher", "isolation", "commands.js")
  if (!existsSync(from)) {
    console.error(`missing ${from}: set DAINTY_RESTRICTED_DIR to a dainty-restricted checkout`)
    process.exit(1)
  }
  mkdirSync(path.join(launcherDir, "isolation"), { recursive: true })
  copyFileSync(from, path.join(launcherDir, "isolation", "commands.js"))

  const ui = Bun.spawnSync([process.execPath, "run", "restricted-data"], {
    cwd: path.join(repoDir, "ui"),
    env: { ...process.env, DAINTY_RESTRICTED_DIR: restricted },
    stdio: ["ignore", "inherit", "inherit"],
  })
  if (ui.exitCode !== 0) process.exit(ui.exitCode ?? 1)
}

if (import.meta.main) main()
