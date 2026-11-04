// SPDX-License-Identifier: MIT
// Builds or serves one of the dashboard's pages for a Tauri shell: the shells' beforeBuildCommand and
// beforeDevCommand run this, so the page always comes from the dashboard checkout.
import { cpSync, existsSync, rmSync } from "node:fs"
import path from "node:path"

import { repoDir, restrictedDir } from "./restricted-data"

const USAGE = "usage: bun scripts/dashboard-page.ts <build|dev> <launcher|server-manager>"

// The ports the two Tauri configs' devUrl name
const DEV_PORTS: Record<string, number> = { launcher: 4321, "server-manager": 4322 }
// Each shell's DAINTY_MODE in ui/apps/web, which builds every page from one source
const MODES: Record<string, string> = { launcher: "launcher", "server-manager": "manager" }

function run(argv: string[], cwd: string, env: Record<string, string> = {}): void {
  const child = Bun.spawnSync(argv, { cwd, env: { ...process.env, ...env }, stdio: ["inherit", "inherit", "inherit"] })
  if (child.exitCode !== 0) process.exit(child.exitCode ?? 1)
}

function main(): void {
  const args = process.argv.slice(2)
  const [mode, page] = args
  if (args.length !== 2 || (mode !== "build" && mode !== "dev") || page === undefined || !(page in DEV_PORTS)) {
    console.error(USAGE)
    process.exit(2)
  }

  const restricted = restrictedDir()
  // The dashboard is this repository's since RD27; the built page still lands in the restricted checkout
  const dashboard = process.env.DAINTY_DASH_DIR || path.join(repoDir, "ui")
  const app = path.join(dashboard, "apps", "web")
  if (!existsSync(path.join(app, "package.json"))) {
    console.error(`missing ${path.join(app, "package.json")}: set DAINTY_DASH_DIR to a dashboard checkout`)
    process.exit(1)
  }

  const bun = process.execPath
  run([bun, "install", "--frozen-lockfile"], dashboard)
  // The apps import the theme's built CSS
  run([bun, "run", "theme:build"], dashboard)

  if (mode === "dev") {
    run([bun, "run", "dev", "--port", String(DEV_PORTS[page]), "--strictPort"], app, { DAINTY_MODE: MODES[page] })
    return
  }

  run([bun, "run", "build"], app, { DAINTY_MODE: MODES[page] })
  const from = path.join(app, "dist", "client")
  const out = process.env.LAUNCHER_DIST_DIR || path.join(restricted, "launcher", "dist")
  const to = path.join(out, page)
  rmSync(to, { recursive: true, force: true })
  // Vite's manifest, which the shell never fetches
  cpSync(from, to, { recursive: true, filter: (source) => path.basename(source) !== ".vite" })
  console.log(`${page}: ${from} -> ${to}`)
}

if (import.meta.main) main()
