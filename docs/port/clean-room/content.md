# Clean-Room Provenance Record — `services/content` (RD27)

- **Component**: `services/content` (`content` crate, `license = "MIT"`, `Copyright (c) 2026 Silent Spud`)
- **Behaviour Specification**: `docs/port/spec-rd27-content.md` at commit `df11ac9` (branch `docs/rd27-spec-content`)
- **Clean-Room Worktree**: `R:\source\dauntless\.wt\content-mit` (branch `feat/rd27-content-mit`, created from
  `docs/rd27-spec-content` at `df11ac9`)

## 1. Worktree Isolation (`git sparse-checkout list`)

The worktree is a dainty-lass worktree. At `df11ac9` dainty-lass holds only its MIT tree (the AGPL services, the
launcher and the game modules are dainty-restricted's since the RD27 split), so no AGPL source exists in it to exclude.
The sparse checkout additionally leaves out `ui/`, which the implementation does not need:

```text
/*
!/ui/
```

Checks run before any code was written, from the worktree root:

- `Test-Path ui`, `Test-Path services/content`, `Test-Path rust`, `Test-Path modules` all returned `False`.
- `git ls-files | grep -E '^(services/content|rust/|modules/|legacy/|reference/)'` returned nothing.

The implementer never opened, listed, searched or ran git commands against dainty-restricted, `legacy-revived`,
`rustic-*`, `dainty-dashboard`, `dainty-legacy`, `dainty-lass-ui` or another worktree under `R:\source\dauntless\.wt`,
and never looked at any historical version of `services/content`, `rust/services/content` or
`services/content-plugin` in dainty-lass's history. No web search was made. (The session's own working directory was
a dainty-restricted worktree; every command used absolute paths into `content-mit` and nothing under the working
directory was read.)

## 2. Category A Files Not Carried Here

The Category A files of the spec's section 1.2 live in dainty-restricted, which the implementer may not open. They
are left for the integration pass to copy in unchanged:

- `services/content/src/modules.rs` (with its two log calls pointed at `service_kit::linelog`, spec 1.4). This branch
  holds a **stand-in** `modules.rs` written clean-room from the spec's summary (section 2.14), so the crate builds and
  its routes can be tested; the integration pass replaces it. The rest of the crate calls it only through:
  `ContentModules::{new(Arc<dyn RegistryView>), none(), resolve(&str) -> Option<GameVersion>,
  resolve_edition(&str) -> Option<GameVersion>, versions() -> VersionsBody, client_profile(&GameVersion) ->
  Option<Option<ClientProfile>>, runtime_folders(&GameVersion) -> Vec<String>, epic_manifest(&GameVersion) ->
  Result<EpicManifest { bytes, sha1 }, EpicManifestError>}`, the `CorePlugin` impl, and
  `RegistryView::module_dir(&ModuleId) -> Option<PathBuf>` (in `service.rs`). Where the real file's names differ, the
  call sites in `app.rs`, `server.rs` and `service.rs` adapt to it.
- `services/content-plugin/{Cargo.toml, src/lib.rs, tests/parity.rs}`.
- `services/content/tests/parity/{content,content-none,content-invalid,content-index}.{requests,expected}.json`.
  Until they arrive, `tests/parity.rs` skips those four scripts with a `SKIPPED` line; `spec-selftest`, a script
  written for this rewrite from the spec alone, runs the harness meanwhile.

## 3. Clean-Room Implementer Session

One Claude Code subagent session (2026-11-02), launched by the maintainer's session with the clean-room rules.

- **Files Read**:
  - `docs/port/spec-rd27-content.md`, `docs/port/clean-room/hostkit.md`, `AGENTS.md`
  - `Cargo.toml`, `Cargo.lock` (versions only), `.forgejo/workflows/ci.yml` (the "every package is MIT" step)
  - `crates/service-plugin/src/lib.rs`, `crates/service-plugin/src/chat.rs` (head), `crates/service-plugin/Cargo.toml`
  - `crates/module-api/src/{lib,version,manifest,module,registrar,stub,ids}.rs` (the parts naming the types used)
  - `crates/protocol/src/{manifest,invite}.rs`, `crates/protocol/Cargo.toml`, `crates/protocol/src/lib.rs` (grep)
  - `crates/service-kit/src/{env,js,jsjson,linelog,context,serve}.rs`, `crates/service-kit/Cargo.toml`
  - `crates/controller/src/service_library.rs` (head), `crates/controller/tests/echo-service/{Cargo.toml,src/lib.rs}`
  - `services/admin/{Cargo.toml,src/lib.rs,src/server.rs,src/upstream.rs}` (patterns for an MIT service)
  - `crates/hostkit/src/hostconfig.rs` (the `content.env` lines, by grep)
  - `fuzz/Cargo.toml`, `fuzz/fuzz_targets/runtime_input.rs`, `docs/security/plugin-host.md`
- **Files Written**:
  - `services/content/Cargo.toml`
  - `services/content/src/{lib,app,auth,branding,cli,client,config,input,limits,log,modules,news,runtime,server,
    service}.rs` (`modules.rs` is the stand-in of section 2)
  - `services/content/tests/app.rs`, `services/content/tests/parity.rs`, `services/content/tests/parity/harness.rs`
  - `services/content/tests/parity/spec-selftest.{requests,expected}.json` (expected replies derived by hand from the
    spec, not recorded)
  - `fuzz/fuzz_targets/{content_request,content_runtime_index,content_news}.rs` and their `[[bin]]` entries in
    `fuzz/Cargo.toml`
  - `docs/security/content.md`, this record
  - `Cargo.toml`: the `services/content` member; `content`, `bytes`, `http`, `http-body` in
    `[workspace.dependencies]`

## 4. Verification

With `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=line-tables-only`, `-j 4`, on Windows:

- `cargo fmt --all --check`: clean
- `cargo clippy --workspace --all-targets -- -D warnings`: 0 warnings
- `cargo test -p content`: 46 unit tests, 7 HTTP tests (`tests/app.rs`), 5 parity tests (4 skipped until the
  fixtures arrive, `spec_selftest` run); 0 failed
- `cargo test --workspace --no-fail-fast`: 673 passed, 0 failed
- `cargo metadata --no-deps`: all 19 packages MIT
- `cargo check --manifest-path fuzz/Cargo.toml --bin content_request --bin content_runtime_index --bin content_news`:
  builds

## 5. Similarity

Left for the reviewer (spec section 4.5): someone other than the implementer runs the `similarity.md` method against
dainty-restricted's `services/content` at `ee7d90b` and DR's `rust/services/content` at `24b7624`, and appends the
per-file table to `similarity.md`.
