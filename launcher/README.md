# DaintyLass Desktop Apps (`dainty-lass-client` & `dainty-lass-manager`)

This package contains the Tauri 2 desktop backend (`src-tauri/`) and bridge test suite (`test/`) for both DaintyLass desktop applications:

- **DaintyLass Launcher (`dainty-lass-client`)** — the player launcher (`src-tauri/tauri.conf.json`). Join servers by invite or address, register and sign in (with the account key stored only in the OS keystore), check server status and news, verify Dauntless 1.14.7 installs, download the server runtime, and host or join peer-to-peer realms.
- **DaintyLass Manager (`dainty-lass-manager`, Cargo feature `server-gui`)** — the standalone server manager GUI (`src-tauri/tauri.server-gui.conf.json`, built with `--no-default-features --features server-gui`). Embeds `crates/host`, `server-controller`, `admin`, and the Rust game modules (`isles-1-14-7`) in-process so a host can set up, configure, migrate, run, and administer a community server without a terminal. Keeps the bundle identifier `io.github.freezamotoqueiro.dauntlessreforgedrevival.server` so existing server GUI installations update in place.

Each app embeds its own page from [dainty-lass's `dashboard/`](https://git.silentspud.com/silentspud/dainty-lass/src/branch/main/dashboard): the launcher serves `apps/launcher`, the manager `apps/server-manager`. `beforeBuildCommand` runs dainty-lass's `launcher/scripts/dashboard-page.ts` (MIT, RD27) through `scripts/lass.ts`, which builds the page's static export with Bun (`apps/<page>/dist/client`) and copies it to `dist/launcher` or `dist/server-manager` here, the configs' `frontendDist`. dainty-lass is `DAINTY_LASS_DIR`, or `dainty-lass` beside this repository; the dashboard is its `dashboard/`, or `DAINTY_DASH_DIR`. Tauri hashes the exports' inline scripts into the CSP at build time, so `script-src 'self'` holds. The bridge types stay in `../../ui/packages/bridge` (the `ui/` submodule), generated from `src-tauri/src/bridge/spec.rs`.

## Prerequisites

- **Rust** toolchain (`rustup`, matching workspace `rust-version`)
- **Bun** (`>= 1.4.3`, as the dashboard asks), which also installs the Tauri CLI (`@tauri-apps/cli`); **Node.js** (`>= 22.18`) runs the dashboard pages' Astro build
- A **dainty-lass** checkout for the pages, the scripts, the isolation hook and the bridge tests (see above)
- **Windows**: WebView2 runtime (included in Windows 10/11) and NSIS (downloaded automatically by Tauri CLI on first bundle build)

## Building from a Clean Clone

From `launcher/`:

```bash
# 1. Install Tauri CLI and test dependencies
bun install

# 2. Build the player launcher NSIS installer (dainty-lass-client)
bun run build:client

# 3. Build the server manager GUI NSIS installer (dainty-lass-manager)
bun run build:manager
# (alias: bun run build:server-gui)

# Or build both installers in sequence:
bun run build:all
```

Each build first runs `bun scripts/lass.ts dashboard-page build <page>`: `bun install --frozen-lockfile` and the theme build in the dashboard checkout, then the page's export, copied into `dist/<page>`. `bun run build:pages` does both pages without building the apps. With the checkout elsewhere:

```bash
DAINTY_LASS_DIR=/path/to/dainty-lass bun run build:client
```

### Output Paths (Windows NSIS)

After building from the workspace root (`target/release/`):

- **Executables**:
  - `../target/release/dainty-lass-client.exe`
  - `../target/release/dainty-lass-manager.exe`
- **NSIS Installers**:
  - `../target/release/bundle/nsis/DaintyLass Launcher_<version>_x64-setup.exe`
  - `../target/release/bundle/nsis/DaintyLass Manager_<version>_x64-setup.exe`

## Development Mode

```bash
# Run the client launcher against the launcher page's dev server (port 4321)
bun run dev

# Run Dainty Lass Manager against the server-manager page's dev server (port 4322)
bun run dev:manager
# (alias: bun run dev:server-gui)
```

## Tests and Typechecks

```bash
# The bridge and isolation hook tests (dainty-lass's launcher/, against this checkout's generated files)
(cd "${DAINTY_LASS_DIR:-../../dainty-lass}/launcher" && bun install && bun run typecheck && bun run test)

# Run the Rust unit tests for each feature configuration
cargo test --manifest-path ../Cargo.toml -p launcher   # includes tests/bundles.rs, the two apps' configs
cargo test --manifest-path ../Cargo.toml -p launcher --features realms
cargo test --manifest-path ../Cargo.toml -p launcher --no-default-features --features server-gui
```
