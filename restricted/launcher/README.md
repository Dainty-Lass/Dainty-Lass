# Dainty Lass desktop apps (`dainty-launcher` & `dainty-manager`)

Since RL13 the apps are dainty-lass's MIT Tauri shell, `crates/dainty-launcher-app`, over `crates/dainty-launcher-kit`. This package is their AGPL half (RD27 spec §4.4): `src-tauri/` builds them with today's icons (L2), hands the manager the MIT embedded server (`crates/dainty-manager-server`), and runs the tests that hold dainty-launcher-kit to DR's recorded tables (`test/cases`) and drive its controller, relay and modules, and that are the oracle for `dainty-manager-server` (`tests/manager_server.rs`, `tests/controller.rs`'s `server-gui` cases). `test/electron-keystore` is the Electron key-store harness.

- **Dainty Lass Launcher (`dainty-launcher`)**: the player launcher (`src-tauri/tauri.conf.json`). Join servers by invite or address, register and sign in (with the account key stored only in the OS keystore), check server status and news, verify Dauntless 1.14.7 installs, download the server runtime, and host or join peer-to-peer realms.
- **Dainty Lass Manager (`dainty-manager`, Cargo feature `server-gui`)**: the standalone server manager GUI (`src-tauri/tauri.server-gui.conf.json`, built with `--no-default-features --features server-gui`). Embeds `crates/dainty-manager-server` (the host kit, the controller, the admin history and the 1.14.7 module) in-process so a host can set up, configure, migrate, run, and administer a community server without a terminal. Keeps the bundle identifier `io.github.freezamotoqueiro.dauntlessreforgedrevival.server` so existing server GUI installations update in place.

Each app embeds its own page from [dainty-lass's `ui/`](https://git.silentspud.com/silentspud/dainty-lass/src/branch/main/ui) (dainty-dash), whose `apps/web` builds every page from one source: the launcher serves it built with `DAINTY_MODE=launcher`, the manager with `DAINTY_MODE=manager`. `beforeBuildCommand` runs `crates/dainty-launcher-app/scripts/page.ts`, which builds the page's static export with Bun and copies it to `crates/dainty-launcher-app/dist/launcher` or `dist/server-manager`, where dainty-launcher-app's compiled-in context and these configs' `frontendDist` find it. Tauri hashes the exports' inline scripts into the CSP at build time, so `script-src 'self'` holds. The app's context (name, identifier, page, update feed, permissions, isolation hook) is dainty-launcher-app's; these configs give the bundler today's icons and the same identity, which `tests/bundles.rs` checks. The bridge's generated files are dainty-launcher-kit's (`crates/dainty-launcher-kit/generated`).

## Prerequisites

- **Rust** toolchain (`rustup`, matching workspace `rust-version`)
- **Bun** (`>= 1.4.3`, as the dashboard asks), which also installs the Tauri CLI (`@tauri-apps/cli`); the pages are a static export dainty-lass builds with Bun
- A **dainty-lass** checkout for the pages, the scripts, the isolation hook and the bridge tests (see above)
- **Windows**: WebView2 runtime (included in Windows 10/11) and NSIS (downloaded automatically by Tauri CLI on first bundle build)

## Building from a clean clone

From `launcher/`:

```bash
# 1. Install Tauri CLI and test dependencies
bun install

# 2. Build the player launcher NSIS installer (dainty-launcher)
bun run build:client

# 3. Build the server manager GUI NSIS installer (dainty-manager)
bun run build:manager
# (alias: bun run build:server-gui)

# Or build both installers in sequence:
bun run build:all
```

Each build first runs `bun ../../crates/dainty-launcher-app/scripts/page.ts build <page>`: `bun install --frozen-lockfile` and the theme build in the dashboard checkout, then the page's export, copied into `crates/dainty-launcher-app/dist/<page>`. `bun run build:pages` does both pages without building the apps. With the checkout elsewhere:

```bash
DAINTY_LASS_DIR=/path/to/dainty-lass bun run build:client
```

### Output Paths (Windows NSIS)

After building from the workspace root (`target/release/`):

- **Executables**:
  - `../target/release/dainty-launcher.exe`
  - `../target/release/dainty-manager.exe`
- **NSIS Installers**:
  - `../target/release/bundle/nsis/Dainty Lass Launcher_<version>_x64-setup.exe`
  - `../target/release/bundle/nsis/Dainty Lass Manager_<version>_x64-setup.exe`

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
# The bridge and isolation hook tests (dainty-lass's launcher/, against dainty-launcher-kit's generated files)
(cd "${DAINTY_LASS_DIR:-../../dainty-lass}/launcher" && bun install && bun run typecheck && bun run test)

# Run the Rust unit tests for each feature configuration
cargo test --manifest-path ../Cargo.toml -p dainty-launcher -j 6   # includes tests/bundles.rs, the two apps' configs
cargo test --manifest-path ../Cargo.toml -p dainty-launcher -j 6 --features realms
cargo test --manifest-path ../Cargo.toml -p dainty-launcher -j 6 --features server-gui,realms
```
