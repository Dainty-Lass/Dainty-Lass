<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Game modules in the server binaries (DF5)

`dainty-lass-server-console`, `reforged-host` (same crate) and `dainty-lass-server-minimal` link game modules by
Cargo feature (DD12). A build with none starts and serves the core plugins only; each feature adds one module to
`controller::modules::game_modules()`, which `serve` lists at start (`[modules]` in `host.toml` picks which load).

## Features

The same names on `controller`, `host`, `server-console`, `server-minimal` and, for the server GUI, `launcher`
(`--features server-gui,...`). None is a default.

| Feature | Links | State |
| --- | --- | --- |
| `isles-1-14-7` | `isles-1-14-7` from `modules/rustic-isles` | linked |
| `isles-1-12-0` | `isles-1-12-0` from `modules/rustic-isles` | linked |
| `realms` | rustic-rifts' server crates inside `isles-1-14-7` (RF2); on its own it does nothing | linked |
| `revived-1-4-4` | the 1.4.4 module | **not linked**: `modules/legacy-revived` has no `GameModule` crate yet (RV1) |
| `stub-module` | `module_api::stub::StubModule` (id `stub`, game version 0.0.1) | for tests and trying a build |
| `all-modules` | `isles-1-14-7`, `isles-1-12-0`, `revived-1-4-4`, `realms` (never the stub) | what the release builds |

A build with `revived-1-4-4` starts, and says at start that it has nothing to load for it
(`controller::modules::UNLINKED`), so a release never lacks a module unnoticed.

## How the real crates link

`rust/Cargo.toml` lists `isles-1-14-7` (default features off, `realms` is ours) and `isles-1-12-0` in
`[workspace.dependencies]` as path dependencies into the submodule; the controller takes them as optional. The module
repositories depend on `module-api`, `service-kit`, `store` and rustic-rifts' crates as git dependencies at a fixed
rev (DD2, DF1's fallback). Inside this tree those would be a second copy of `module-api`, so the workspace has
`[patch."ssh://git@git.silentspud.com:222/silentspud/dainty-lass.git"]` and the rustic-rifts equivalent, pointing at
`crates/` and `../modules/rustic-rifts/crates/`. One copy of each lands in the tree and `isles_1_14_7::module()` is the
controller's own `GameModule`.

- Any cargo command in `rust/` needs the submodules checked out (`git submodule update --init`), also with every
  module feature off: cargo reads the optional dependencies' manifests. CI and the release already check out
  `submodules: recursive`.
- The first resolve fetches the two git sources over SSH (the `[patch]` needs their indexes).
- Bumping a module's pin: the module repository's `Cargo.lock` is not read here; `rust/Cargo.lock` is.

## Adding the next module crate

For `revived-1-4-4`, when RV1 holds a crate that implements `GameModule` (say `revived-1-4-4`, in a workspace under
`modules/legacy-revived`):

1. `rust/Cargo.toml`: `revived-1-4-4 = { path = "../modules/legacy-revived/<path to the crate>" }` in
   `[workspace.dependencies]`. If its workspace names `module-api`, `protocol`, `service-kit` or `store` as git
   dependencies, add them to the `[patch]` table of that git URL; if as `registry = "silentspud"`, patch that
   registry the same way.
2. `crates/controller/Cargo.toml`: the optional dependency, and `revived-1-4-4 = ["dep:revived-1-4-4"]`.
3. `crates/controller/src/modules.rs`: a `#[cfg(feature = "revived-1-4-4")]` push in `game_modules()` returning
   `Arc::new(revived_1_4_4::module())`, and remove the feature from `UNLINKED`.
4. `bins/modules_test.rs`: add it to `expected()`.
5. `cargo test -p server-console -p server-minimal --test modules` with `--features revived-1-4-4`.

If the crate links in-process but the module should run as its own process instead (DD5), do not link it: list it in
`[modules]` as an external module with its SHA-256.

## Tests

`bins/modules_test.rs`, run by each bin's `tests/modules.rs`: the modules the features link are exactly the ones
named, all of them load on the registry at start with no port held twice, and stop with the server. The feature set is
the test's input, so the three cases are three runs (from `rust/`, for `server-console` and `server-minimal`):

```bash
cargo test -p server-console --locked --test modules                                          # zero
cargo test -p server-console --locked --test modules --features stub-module                   # one
cargo test -p server-console --locked --test modules --features all-modules,stub-module       # every one
```

The same three with `-p server-minimal`. `cargo test --workspace` runs the zero case.

## Release

`server-release.yml` (RH6's, not edited here) line 121 should become

```bash
cargo build --release --locked --target "$TARGET" -p server-console -p server-minimal --bins \
  --features server-console/all-modules,server-minimal/all-modules
```

and the launcher's server GUI build, `tauri build ... --features server-gui` in `launcher/src-tauri`, gains
`all-modules`. The Docker images (`docker/`, DS6's) build the same way. CI's Rust family could also run the three test
commands above; that is RR3's workflow.
