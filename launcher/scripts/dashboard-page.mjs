// Builds or serves one of dainty-lass dashboard/'s pages for the Tauri shells (its docs/integration.md):
// `launcher` for dainty-lass-client, `server-manager` for dainty-lass-manager. A build copies the static export
// (apps/<page>/dist/client) to launcher/dist/<page>, the config's fixed frontendDist, since Tauri's config
// can't read where the checkout is.
//   node scripts/dashboard-page.mjs build|dev launcher|server-manager
// The dashboard is DAINTY_DASH_DIR, else dainty-lass's dashboard/ (MIT, RD27): DAINTY_LASS_DIR's, as CI clones it at
// the rev Cargo.toml pins, or the dainty-lass checkout beside this one.
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scripts = path.dirname(fileURLToPath(import.meta.url));
const launcher = path.resolve(scripts, "..");
const repo = path.resolve(launcher, "..");
// The ports devUrl names in tauri.conf.json and tauri.server-gui.conf.json
const PAGES = { launcher: 4321, "server-manager": 4322 };

const [mode, page] = process.argv.slice(2);
if (!["build", "dev"].includes(mode) || !(page in PAGES)) {
  console.error("usage: node scripts/dashboard-page.mjs build|dev launcher|server-manager");
  process.exit(2);
}

const lass = process.env.DAINTY_LASS_DIR || path.join(repo, "..", "dainty-lass");
const dashboard = path.resolve(process.env.DAINTY_DASH_DIR || path.join(lass, "dashboard"));
const app = path.join(dashboard, "apps", page);
if (!fs.existsSync(path.join(app, "package.json"))) {
  console.error(
    `the dashboard is not at ${dashboard}: set DAINTY_LASS_DIR or DAINTY_DASH_DIR`,
  );
  process.exit(1);
}

const bun = (cwd, ...args) => {
  const result = spawnSync("bun", args, { cwd, stdio: "inherit" });
  if (result.error) console.error(`bun: ${result.error.message}`);
  if (result.status !== 0) process.exit(result.status ?? 1);
};

bun(dashboard, "install", "--frozen-lockfile");
// The apps import the theme's built CSS, as the dashboard's own `bun run build` does first
bun(dashboard, "run", "theme:build");
if (mode === "dev") {
  bun(app, "run", "dev", "--port", String(PAGES[page]), "--strictPort");
} else {
  bun(app, "run", "build");
  const from = path.join(app, "dist", "client");
  const to = path.join(launcher, "dist", page);
  fs.rmSync(to, { recursive: true, force: true });
  // .vite holds Vite's manifest, which the page never fetches
  fs.cpSync(from, to, { recursive: true, filter: (source) => path.basename(source) !== ".vite" });
  console.log(`${page}: ${from} -> ${to}`);
}
