<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Spike DF1: the cargo layout of DD2

Does DD2's layout build? Module repositories as submodules under `modules/<name>/`, outside DaintyLass's workspace,
each with a workspace of its own; `module-api`, `protocol`, `service-kit` and `store` taken by version from Forgejo's
Cargo registry and `[patch]`ed to DaintyLass's paths; rustic-isles taking rustic-rifts the same way. Run 2026-10-26
on Windows 11 (MSVC) and Linux (`rust:1.99.1-bookworm` in Docker), cargo 1.99.1 on both.

**Verdict: the layout works. The kill criterion is not met; modules take the registry, not a git tag.** Nothing was
published to the real registry: a local sparse registry stood in for it. Forgejo's own behaviour is the maintainer's
check at the end, and the real registry already differs from DD2 in one way: it needs a token to read.

## The toy

Built in a scratch directory outside the repository, from nothing, by one script run on both systems:

- **host**, a git repository standing in for DaintyLass: members `protocol`, `module-api` (depends on `protocol`) and
  a `server` bin with the game module as a default feature.
- **toy-rifts**, a module repository standing in for rustic-rifts: one library crate taking `module-api` from the
  registry.
- **toy-isles**, a module repository standing in for rustic-isles: a library and binary taking `module-api` and
  `rifts-core` from the registry.
- **toy-gitmod**, for the fallback: `module-api` as a git dependency on a tag of the host.
- A sparse registry served over HTTP on 127.0.0.1, named `silentspud` in every config, so the manifests below are
  the ones DaintyLass would use. `protocol`, `module-api` and `rifts-core` were packaged with
  `cargo package --registry silentspud` and indexed.

The module repositories were added to the host with `git submodule add` under `modules/`, and every check was
repeated in a fresh `git clone --recurse-submodules`.

## Results

Both systems gave the same result for every check.

| # | Check | Windows | Linux |
| --- | --- | --- | --- |
| 1 | A module builds alone against the registry: rifts, then isles taking rifts and `module-api` by version | Pass | Pass |
| 2 | The host builds as one tree: one `module-api` and one `protocol`, the host's paths, used by isles and by rifts through isles (`cargo tree -d` empty) | Pass | Pass |
| 3 | No complaint about nested workspaces with `exclude`; `workspace = true` inheritance resolves from the module's own root inside the host | Pass | Pass |
| 3a | Without `exclude`, the host pulls the module crate into its workspace and fails (`dependency.rifts-core was not found in workspace.dependencies`) | Fails, as expected | Fails, as expected |
| 3b | A module built alone from `modules/<name>` inside the host checkout uses its own workspace and lockfile, and leaves the host's tree clean | Pass | Pass |
| 3c | Workspace in `rust/`, `modules/` at the repository root beside it (DaintyLass's layout): paths become `../modules/...`, `exclude` is a no-op, `exclude = ["../modules"]` is accepted | Pass | Pass |
| 4 | Lockfiles: see below | As below | As below |
| 5 | `--locked`: module alone, host, fresh clone with submodules, module alone inside the host checkout | Pass | Pass |
| 5a | `--locked --offline` and `--locked` with the registry down and its cache deleted, in the host | Pass | Pass |
| 6 | Fallback: `module-api` as a git dependency on a tag, alone and `[patch]`ed to the host's path | Pass | Pass |

### Lockfiles

- **A module alone** records each registry crate with `source = "sparse+<index>"` and a checksum. Commit it: modules
  build an executable (DD5).
- **The host** records the module crates as path packages and has **no** registry entry for a patched crate, so
  `--locked` needs no registry at all (5a). The modules' own lockfiles are ignored inside the host.
- **A fresh resolution needs the registry** even when every crate is patched: `cargo generate-lockfile` with the
  registry down fails. So does any change that re-resolves, such as moving a submodule to a commit that changes a
  module's dependencies. The host's `Cargo.lock` then changes, and `--locked` fails until it is committed with the
  pin.
- **An unused patch is silent.** With the host's `module-api` at 0.2.0 and a module asking for `0.1`, cargo 1.99.1
  printed no warning and wrote no `[[patch.unused]]`: it fetched `module-api` 0.1.0 from the registry and built
  both. `cargo tree -d` shows it, and the host's `Cargo.lock` gains a `source = "sparse+..."` line.

### What the layout needs

- **`registry = "silentspud"` on the host's path dependencies.** Without it, `cargo package` writes the dependency
  as a crates.io one (`version = "0.1.0"` and no `registry-index`), so the published `module-api` would ask
  crates.io for `protocol`.
- **Real versions on the four published crates.** The workspace's `0.0.0` can't be required by a module, and under
  caret rules `0.0.x` pins one exact version. A breaking `module-api` (0.1 to 0.2) is published first, then each
  module raises its requirement, then the pins and the host's lockfile move together; a host change a module uses
  before its release builds inside the host and fails alone, as it should.
- **Submodules are not optional.** A clone without them fails before building anything: a `[patch]` path that
  doesn't exist is an error (`patch location ... does not contain packages matching rifts-core`). CI and developers'
  clones use `--recurse-submodules`.
- **Registry config in every repository.** Config is read from the working directory upward, and `modules/` sits
  outside `rust/`, so `rust/.cargo/config.toml` doesn't reach a module built from its own folder. Each module
  repository carries its own `.cargo/config.toml`, and its own `rust-toolchain.toml` for the same reason.
- **`--workspace` stops at the host's members.** `cargo test --workspace`, `clippy --workspace` and `fmt --all` in
  `rust/` don't cover the modules; each module's checks run from its own folder, and need registry read access.
- **A guard for the silent patch:** host CI fails when `rust/Cargo.lock` holds a
  `sparse+https://git.silentspud.com/` source. That is a CI change (RR3, DX3), not this row's.
- **Windows long paths** for git: `core.longpaths=true`, or a deep checkout under a long path fails to clone a
  submodule's objects. Cargo's own libgit2 checkout of a git dependency failed the same way under a long
  `CARGO_HOME` (the fallback only; the registry path never hit it).

## Manifests

Crate names under `modules/` are illustrative; the real ones are the module rows' choice.

### DaintyLass: `rust/Cargo.toml`

```toml
[workspace]
resolver = "3"
members = ["crates/module-api", "crates/protocol", "crates/service-kit", "crates/store", "..."]
# A no-op while the workspace is in rust/ and the modules beside it; required if the root moves above modules/
exclude = ["fuzz", "../modules"]

[workspace.dependencies]
# registry = "silentspud" is what a published dependent keeps; without it cargo package writes a crates.io one
module-api = { path = "crates/module-api", version = "0.1.0", registry = "silentspud" }
protocol = { path = "crates/protocol", version = "0.1.0", registry = "silentspud" }
service-kit = { path = "crates/service-kit", version = "0.1.0", registry = "silentspud" }
store = { path = "crates/store", version = "0.1.0", registry = "silentspud" }
isles-1-14-7 = { path = "../modules/rustic-isles/crates/isles-1-14-7" }

# Modules ask for these by version; inside a checkout they are the working copy
[patch.silentspud]
module-api = { path = "crates/module-api" }
protocol = { path = "crates/protocol" }
service-kit = { path = "crates/service-kit" }
store = { path = "crates/store" }
rifts-core = { path = "../modules/rustic-rifts/crates/rifts-core" }
```

### DaintyLass: a published crate, `rust/crates/module-api/Cargo.toml`

```toml
[package]
name = "module-api"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license = "MIT"
publish = ["silentspud"]

[dependencies]
protocol.workspace = true
```

### DaintyLass and every module repository: `.cargo/config.toml`

```toml
[registries.silentspud]
index = "sparse+https://git.silentspud.com/api/packages/silentspud/cargo/"
```

In DaintyLass this goes in the existing `rust/.cargo/config.toml`.

### A module repository: `Cargo.toml` at its root

```toml
[workspace]
resolver = "3"
members = ["crates/*"]

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.99"
license = "AGPL-3.0-only"
publish = ["silentspud"]

[workspace.dependencies]
module-api = { version = "0.1", registry = "silentspud" }
protocol = { version = "0.1", registry = "silentspud" }
service-kit = { version = "0.1", registry = "silentspud" }
store = { version = "0.1", registry = "silentspud" }
# rustic-isles only: rustic-rifts by version, patched to modules/rustic-rifts inside DaintyLass
rifts-core = { version = "0.1", registry = "silentspud" }
```

With it: `.cargo/config.toml` as above, `rust-toolchain.toml` matching DaintyLass's, `/target` in `.gitignore`
and `Cargo.lock` committed.

### A module crate: `crates/isles-1-14-7/Cargo.toml`

```toml
[package]
name = "isles-1-14-7"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish.workspace = true

[dependencies]
module-api.workspace = true
rifts-core.workspace = true
```

## The kill criterion

Not met: the registry and `[patch]` worked on both systems, so the fallback is not needed. It does work as well:

```toml
# module workspace
module-api = { git = "https://git.silentspud.com/silentspud/dainty-lass", tag = "module-api-v0.1.0" }
# DaintyLass rust/Cargo.toml
[patch."https://git.silentspud.com/silentspud/dainty-lass"]
module-api = { path = "crates/module-api" }
```

It would cost more than it saves. Cargo clones DaintyLass **and checks out its submodules** (every module and
`ui/`) for each module's build. The lockfile pins a commit of DaintyLass, which is the git dependency DD2 avoids.
The `silentspud` registry still has to be defined, because `module-api`'s manifest names it: without it cargo
skips the package as malformed and reports `no matching package named module-api`. And while the instance needs
sign-in, cargo needs git credentials (`net.git-fetch-with-cli = true` and a token) for all of it.

## Maintainer: confirm on the real registry

Not done here (nothing published). As of 2026-10-26, an anonymous request for the index's `config.json` returns
**401** (`reqPackageAccess`), `/api/v1/orgs/silentspud` returns 403 ("Only signed in user is allowed to call APIs")
and repository pages redirect to the sign-in page. The instance requires sign-in, so DD2's "public on the instance,
no token for CI and clones" doesn't hold today.

1. **Decide on reads.** Either let anonymous users read the instance and the `silentspud` organisation, or give CI
   and every developer a read token. Check with
   `curl -s -o /dev/null -w '%{http_code}\n' https://git.silentspud.com/api/packages/silentspud/cargo/config.json`:
   200 means anonymous reads work.
2. **Initialise the index once.** Organisation settings, Packages, Cargo: "Initialize index", which creates the
   `_cargo-index` repository and its config.
3. **Make a token** (user settings, Applications) with package write access. Then either run
   `cargo login --registry silentspud` and paste `Bearer <token>`, with the word `Bearer`, or set
   `CARGO_REGISTRIES_SILENTSPUD_TOKEN="Bearer <token>"`. Point cargo at the registry in a scratch folder with
   `CARGO_REGISTRIES_SILENTSPUD_INDEX=sparse+https://git.silentspud.com/api/packages/silentspud/cargo/`.
4. **Publish a throwaway pair** from a scratch folder outside every repository. First `df1-probe-api` 0.0.1 with
   `publish = ["silentspud"]`, then `cargo publish --registry silentspud`. Then `df1-probe-mod` 0.0.1 with
   `df1-probe-api = { version = "0.0.1", registry = "silentspud" }`, run `cargo build`, then
   `cargo build --locked`, then publish it. Its `Cargo.lock` must show
   `source = "sparse+https://git.silentspud.com/api/packages/silentspud/cargo/"` with a checksum. This is check 1,
   and the rifts-to-isles case.
5. **Patch it.** In a scratch workspace with `df1-probe-api` as a path member and a member taking
   `df1-probe-mod = { version = "0.0.1", registry = "silentspud" }`, add
   `[patch.silentspud] df1-probe-api = { path = "<member path>" }`. `cargo tree -i df1-probe-api` must show only
   the path copy, and `cargo build --locked` must pass (checks 2 and 5).
6. **Read without the token.** Unset `CARGO_REGISTRIES_SILENTSPUD_TOKEN`, take a fresh `CARGO_HOME`, and run
   `cargo build --locked` in `df1-probe-mod`. A pass means modules' CI needs no secret. A 401 means anonymous
   reads are still off (step 1); then confirm the same build passes with the token set, which is what modules' CI
   would carry.
7. Steps 4 to 6 on Windows and in `docker run rust:1.99.1-bookworm`.
8. **Delete both probe packages** (organisation, Packages, the crate, Settings, Delete). Forgejo refuses a second
   upload of the same name and version.

If step 4 or 5 fails, the kill criterion applies: modules take `module-api` as a git dependency on a tag, as in the
section above.
