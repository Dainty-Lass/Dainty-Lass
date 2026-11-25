<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Game modules in the server binaries (DF5)

The game module, Rustic Isles for 1.14.7, is MIT (`crates/rustic-isles-1-14-7` and the `rustic-isles-*` areas it
composes, RD27). The binaries that compile it in are MIT too, and read its version data at start (RD27):
`dainty-server`, `dainty-host` (same crate, `crates/dainty-server`) and
`dainty-server-minimal` link game modules by Cargo feature (DD12). The root's `crates/dainty-lass` links
none; it loads the modules and services `settings.toml` lists as libraries (RD27). A build with no module feature starts
and serves the core plugins only; each feature adds one module to `crates/dainty-host/src/compose.rs`'s
`game_modules()`, on top of `controller::modules::game_modules()`, and `serve` lists them at start (`[modules]` in
`settings.toml` picks which load).

## Features

The same names on `dainty-host`, `dainty-server`, `dainty-server-minimal` and, for the server GUI,
`dainty-launcher-app` (`--features server-gui,...`). `dainty-server` and `dainty-server-minimal` default to
`isles-1-14-7` and `realms` (RU9/RB14); `dainty-controller` has only `stub-module`.

| Feature | Links | State |
| --- | --- | --- |
| `isles-1-14-7` | `rustic-isles-1-14-7` with the version data `settings.toml`'s `version_data` (design) and `version_facts` name, read at start (defaults `data/1.14.7/version_data.json` and `version_facts.json`; restricted holds the design in `modules/rustic-isles/version-data/1.14.7`, the root the facts in `crates/rustic-isles-1-14-7/data`) | linked |
| `realms` | the Rustic Rifts crates (`crates/rustic-rifts-*`) inside `rustic-isles-1-14-7` (RF2); on its own it does nothing | linked |
| `stub-module` | `module_api::stub::StubModule` (id `stub`, game version 0.0.1) | for tests and trying a build |
| `all-modules` | `isles-1-14-7`, `realms` (never the stub) | what the release builds |

## How the real crates link

The root `Cargo.toml` lists every crate in `[workspace.dependencies]`. `dainty-host` takes the module crates as
optional dependencies, and `compose.rs` builds the controller's `GameModule` with
`rustic_isles_1_14_7::spec_with(<version data>)`.

## Adding the next module crate

For a crate that implements `GameModule` (say `<id>`):

1. The root `Cargo.toml`: `<id> = { path = "crates/<crate>" }` in `[workspace.dependencies]` (through the
   integration pass). The crate is MIT: AGPL code can't link into these binaries (RD27).
2. `crates/dainty-host/Cargo.toml`: the optional dependency, and `<id> = ["dep:<id>"]`; the same feature on
   `dainty-server`, `dainty-server-minimal` and `dainty-launcher-app`, and in their `all-modules`.
3. `crates/dainty-host/src/compose.rs`: a `#[cfg(feature = "<id>")]` push in `game_modules()` returning the
   module.
4. `crates/dainty-server/tests/support/modules.rs`: add it to `expected()`.
5. `cargo test -p dainty-server -p dainty-server-minimal --test modules` with `--features <id>`.

If the crate links in-process but the module should run as its own process instead (DD5), do not link it: list it in
`[modules]` as an external module with its SHA-256. A module built as a library (like `rustic-isles-1-14-7-plugin`)
is loaded by `crates/dainty-plugin-host` instead.

## Tests

`crates/dainty-server/tests/support/modules.rs`, run by each bin's `tests/modules.rs`: the modules the
features link are exactly the ones named, all of them load on the registry at start with no port held twice, and stop
with the server. The feature set is the test's input, so the three cases are three runs (from the root):

```bash
cargo test -p dainty-server --locked --test modules --no-default-features                    # zero
cargo test -p dainty-server --locked --test modules --no-default-features --features stub-module  # one
cargo test -p dainty-server --locked --test modules --features all-modules,stub-module       # every one
```

The same three with `-p dainty-server-minimal`. `cargo test --workspace` runs the default case.

## Release

`.forgejo/workflows/server-release.yml` builds, from the root workspace,
`-p dainty-server -p dainty-server-minimal --features dainty-server/all-modules,dainty-server-minimal/all-modules` and the
service and module libraries, the launcher's server GUI build (`tauri build ... --features server-gui` in
`crates/dainty-launcher-app`) links `isles-1-14-7` and `realms` through `server-gui`, and the Docker image builds
the features in `DAINTY_MODULES` (`all-modules` by default).
