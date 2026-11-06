<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Game modules in the server binaries (DF5)

The game module, Rustic Isles for 1.14.7, is MIT (`crates/rustic-isles-1-14-7` and the `rustic-isles-*` areas it
composes, RD27). The binaries that compile it in are `restricted/`'s, because the host shell they wrap is AGPL:
`dainty-lass-server-console`, `reforged-host` (same crate, `restricted/crates/server-console`) and
`dainty-lass-server-minimal` link game modules by Cargo feature (DD12). The root's `crates/dainty-lass-server` links
none; it loads the modules and services `host.toml` lists as libraries (RD27). A build with no module feature starts
and serves the core plugins only; each feature adds one module to `restricted/crates/host/src/compose.rs`'s
`game_modules()`, on top of `controller::modules::game_modules()`, and `serve` lists them at start (`[modules]` in
`host.toml` picks which load).

## Features

The same names on restricted's `host`, `server-console`, `server-minimal` and, for the server GUI, `launcher`
(`--features server-gui,...`). `server-console` and `server-minimal` default to `isles-1-14-7` and `realms`
(RU9/RB14); `controller` at the root has only `stub-module`.

| Feature | Links | State |
| --- | --- | --- |
| `isles-1-14-7` | `rustic-isles-1-14-7` with restricted's version data (`restricted/modules/rustic-isles/version-data/1.14.7`), falling back to the crate's own | linked |
| `realms` | the Rustic Rifts crates (`crates/rifts-*`) inside `rustic-isles-1-14-7` (RF2); on its own it does nothing | linked |
| `stub-module` | `module_api::stub::StubModule` (id `stub`, game version 0.0.1) | for tests and trying a build |
| `all-modules` | `isles-1-14-7`, `realms` (never the stub) | what the release builds |
| `paradox` | the Node backend and Director on MongoDB, the rollback path until RB15 | opt-in |

## How the real crates link

`restricted/Cargo.toml` lists the root's crates in `[workspace.dependencies]` as path dependencies
(`path = "../crates/<name>"`), `rustic-isles-1-14-7` with default features off among them, so a change at the root
reaches restricted's build at once. `host` takes the module crates as optional dependencies, and `compose.rs` builds
the controller's `GameModule` with `rustic_isles_1_14_7::spec_with(<version data>)`.

## Adding the next module crate

For a crate that implements `GameModule` (say `<id>`):

1. `restricted/Cargo.toml`: `<id> = { path = "../crates/<crate>" }` in `[workspace.dependencies]` (or a path under
   `modules/` if the crate is AGPL).
2. `restricted/crates/host/Cargo.toml`: the optional dependency, and `<id> = ["dep:<id>"]`; the same feature on
   `server-console`, `server-minimal` and `launcher`, and in their `all-modules`.
3. `restricted/crates/host/src/compose.rs`: a `#[cfg(feature = "<id>")]` push in `game_modules()` returning the
   module.
4. `restricted/crates/server-console/tests/support/modules.rs`: add it to `expected()`.
5. `cargo test -p server-console -p server-minimal --test modules` with `--features <id>`.

If the crate links in-process but the module should run as its own process instead (DD5), do not link it: list it in
`[modules]` as an external module with its SHA-256. A module built as a library (like `rustic-isles-1-14-7-plugin`)
is loaded by `crates/plugin-host` instead.

## Tests

`restricted/crates/server-console/tests/support/modules.rs`, run by each bin's `tests/modules.rs`: the modules the
features link are exactly the ones named, all of them load on the registry at start with no port held twice, and stop
with the server. The feature set is the test's input, so the three cases are three runs (from `restricted/`):

```bash
cargo test -p server-console --locked --test modules --no-default-features                    # zero
cargo test -p server-console --locked --test modules --no-default-features --features stub-module  # one
cargo test -p server-console --locked --test modules --features all-modules,stub-module       # every one
```

The same three with `-p server-minimal`. `cargo test --workspace` runs the default case.

## Release

`.forgejo/workflows/server-release.yml` builds, in `restricted/`,
`-p server-console -p server-minimal --features server-console/all-modules,server-minimal/all-modules` and the
service and module libraries, the launcher's server GUI build (`tauri build ... --features server-gui` in
`restricted/launcher/src-tauri`) links `isles-1-14-7` and `realms` through `server-gui`, and the Docker image builds
the features in `REFORGED_MODULES` (`all-modules` by default).
