<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Game modules in the server binaries (DF5)

The game modules are AGPL, so the binaries that link them are dainty-restricted's: `dainty-lass-server-console`,
`reforged-host` (same crate, `crates/server-console`) and `dainty-lass-server-minimal` link game modules by Cargo
feature (DD12). This repository's `crates/dainty-lass-server` links none; it loads modules and services `host.toml`
lists (RD27). A build with no module feature starts and serves the core plugins only; each feature adds one module
to dainty-restricted's `host::compose::game_modules()`, on top of `controller::modules::game_modules()` here, and
`serve` lists them at start (`[modules]` in `host.toml` picks which load).

## Features

The same names on dainty-restricted's `host`, `server-console`, `server-minimal` and, for the server GUI,
`launcher` (`--features server-gui,...`). `server-console` and `server-minimal` default to `isles-1-14-7` and
`realms` (RU9/RB14); `controller` here has only `stub-module`.

| Feature | Links | State |
| --- | --- | --- |
| `isles-1-14-7` | `isles-1-14-7` from `modules/rustic-isles` | linked |
| `realms` | rustic-rifts' server crates inside `isles-1-14-7` (RF2); on its own it does nothing | linked |
| `stub-module` | `module_api::stub::StubModule` (id `stub`, game version 0.0.1) | for tests and trying a build |
| `all-modules` | `isles-1-14-7`, `realms` (never the stub) | what the release builds |

## How the real crates link

dainty-restricted's root `Cargo.toml` lists `isles-1-14-7` (default features off) and the rifts crates in
`[workspace.dependencies]` as path dependencies into the in-tree `modules/rustic-isles/crates` and
`modules/rustic-rifts/crates`, and takes this repository's crates (`controller`, `module-api`, `hostkit`, ...) as git
dependencies at one rev. `host` takes the module crates as optional dependencies, and `isles_1_14_7::module()` is
the controller's own `GameModule`. Moving that rev moves every MIT crate at once; the first resolve fetches
dainty-lass over SSH.

## Adding the next module crate

For a crate that implements `GameModule` (say `<id>`), in dainty-restricted:

1. `Cargo.toml`: `<id> = { path = "modules/<repo>/crates/<crate>" }` in `[workspace.dependencies]`.
2. `crates/host/Cargo.toml`: the optional dependency, and `<id> = ["dep:<id>"]`; the same feature on
   `server-console`, `server-minimal` and `launcher`, and in their `all-modules`.
3. `crates/host/src/compose.rs`: a `#[cfg(feature = "<id>")]` push in `game_modules()` returning
   `Arc::new(<crate>::module())`.
4. `crates/server-console/tests/support/modules.rs`: add it to `expected()`.
5. `cargo test -p server-console -p server-minimal --test modules` with `--features <id>`.

If the crate links in-process but the module should run as its own process instead (DD5), do not link it: list it in
`[modules]` as an external module with its SHA-256.

## Tests

`crates/server-console/tests/support/modules.rs`, run by each bin's `tests/modules.rs`: the modules the features
link are exactly the ones named, all of them load on the registry at start with no port held twice, and stop with
the server. The feature set is the test's input, so the three cases are three runs (from dainty-restricted's root):

```bash
cargo test -p server-console --locked --test modules --no-default-features                    # zero
cargo test -p server-console --locked --test modules --no-default-features --features stub-module  # one
cargo test -p server-console --locked --test modules --features all-modules,stub-module       # every one
```

The same three with `-p server-minimal`. `cargo test --workspace` runs the default case.

## Release

dainty-restricted's `server-release.yml` builds
`-p server-console -p server-minimal --features server-console/all-modules,server-minimal/all-modules`, the
launcher's server GUI build (`tauri build ... --features server-gui` in `launcher/src-tauri`) links `isles-1-14-7`
and `realms` through `server-gui`, and the Docker image builds the features in `REFORGED_MODULES` (`all-modules` by
default).
