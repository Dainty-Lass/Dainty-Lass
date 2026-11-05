// SPDX-License-Identifier: MIT
// Runs one of the root launcher scripts (MIT, RD27) against restricted/:
//   bun scripts/lass.ts <dashboard-page|release-config|sign-windows> [args...]
// The repository root is DAINTY_LASS_DIR, or .. from restricted/. DAINTY_RESTRICTED_DIR is set to restricted/
// so the script finds it from a worktree or CI too.
import { existsSync } from "node:fs"
import path from "node:path"

const repo = path.resolve(import.meta.dir, "../..")
const lass = path.resolve(process.env.DAINTY_LASS_DIR ?? path.join(repo, ".."))
const [name, ...args] = process.argv.slice(2)
const script = path.join(lass, "launcher/scripts", `${name}.ts`)
if (!name || !/^[a-z-]+$/.test(name) || !existsSync(script)) {
  console.error(`no ${script}: set DAINTY_LASS_DIR to the repository root`)
  process.exit(2)
}
const run = Bun.spawnSync(["bun", script, ...args], {
  stdio: ["inherit", "inherit", "inherit"],
  env: { ...process.env, DAINTY_RESTRICTED_DIR: process.env.DAINTY_RESTRICTED_DIR ?? repo },
})
process.exit(run.exitCode ?? 1)
