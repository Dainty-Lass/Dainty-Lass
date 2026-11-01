# DaintyLass Desktop Apps (`dainty-lass-client` & `dainty-lass-manager`)

This package contains the Tauri 2 desktop backend (`src-tauri/`) and bridge test suite (`test/`) for both DaintyLass desktop applications:

- **DaintyLass Launcher (`dainty-lass-client`)** — the player launcher (`src-tauri/tauri.conf.json`). Join servers by invite or address, register and sign in (with the account key stored only in the OS keystore), check server status and news, verify Dauntless 1.14.7 installs, download the server runtime, and host or join peer-to-peer realms.
- **DaintyLass Manager (`dainty-lass-manager`, Cargo feature `server-gui`)** — the standalone server manager GUI (`src-tauri/tauri.server-gui.conf.json`, built with `--no-default-features --features server-gui`). Embeds `services/host`, `server-controller`, `admin`, and the Rust game modules (`isles-1-14-7`) in-process so a host can set up, configure, migrate, run, and administer a community server without a terminal. Keeps the bundle identifier `io.github.freezamotoqueiro.dauntlessreforgedrevival.server` so existing server GUI installations update in place.

Both apps embed the SolidJS frontend in `../../ui/apps/web/dist`, built automatically by Bun via `beforeBuildCommand` in `src-tauri/tauri.conf.json`.

## Prerequisites

- **Rust** toolchain (`rustup`, matching workspace `rust-version`)
- **Bun** (`>= 1.2`) for building `../../ui`
- **Node.js** (`>= 22.18`) and **npm** for the Tauri CLI (`@tauri-apps/cli`) and bridge tests
- **Windows**: WebView2 runtime (included in Windows 10/11) and NSIS (downloaded automatically by Tauri CLI on first bundle build)

## Building from a Clean Clone

From `rust/launcher/`:

```bash
# 1. Install Tauri CLI and test dependencies
npm install

# 2. Build the player launcher NSIS installer (dainty-lass-client)
npm run build:client

# 3. Build the server manager GUI NSIS installer (dainty-lass-manager)
npm run build:manager
# (alias: npm run build:server-gui)

# Or build both installers in sequence:
npm run build:all
```

Each build automatically runs `bun install --cwd ../../ui --frozen-lockfile && bun run --cwd ../../ui build` before compiling the Rust binary and packaging the installer. No manual asset copy step is needed.

### Output Paths (Windows NSIS)

After building from the workspace root (`rust/target/release/`):

- **Executables**:
  - `../target/release/dainty-lass-client.exe`
  - `../target/release/dainty-lass-manager.exe`
- **NSIS Installers**:
  - `../target/release/bundle/nsis/DaintyLass Launcher_<version>_x64-setup.exe`
  - `../target/release/bundle/nsis/DaintyLass Manager_<version>_x64-setup.exe`

## Development Mode

```bash
# Run the client launcher against the Vite dev server
npm run dev

# Run DaintyLass Manager against the Vite dev server
npm run dev:manager
# (alias: npm run dev:server-gui)
```

## Tests and Typechecks

```bash
# Typecheck and run the Node bridge + isolation hook + bundle config tests
npm run typecheck
npm test

# Run the Rust unit tests for each feature configuration
cargo test --manifest-path ../Cargo.toml -p launcher
cargo test --manifest-path ../Cargo.toml -p launcher --features realms
cargo test --manifest-path ../Cargo.toml -p launcher --no-default-features --features server-gui
```
