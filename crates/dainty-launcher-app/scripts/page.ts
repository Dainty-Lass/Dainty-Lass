// SPDX-License-Identifier: MIT
// tauri.conf.json's before-dev and before-build command: the launcher's page from the dashboard, built into this
// crate's dist/ (never restricted's).
import path from "node:path"

const crate = path.resolve(import.meta.dir, "..")
const script = path.resolve(crate, "..", "..", "launcher", "scripts", "dashboard-page.ts")
const child = Bun.spawnSync([process.execPath, script, ...process.argv.slice(2)], {
  env: { ...process.env, LAUNCHER_DIST_DIR: path.join(crate, "dist") },
  stdio: ["inherit", "inherit", "inherit"],
})
process.exit(child.exitCode ?? 1)
