# AGENTS.md

Project-specific guidance for AI coding agents.

The port to one Rust codebase is done in code; what is left is mostly hands-on checks for the maintainer.
[`MIGRATION_PLAN.md`](../MIGRATION_PLAN.md) at the monorepo root lists it and holds the RD decisions; read its
"What's left" and the decisions before starting anything, and the row you were given in full.
[`docs/port/migration-guide.md`](../docs/port/migration-guide.md) is the map: where every upstream file now lives,
what each DR name became, where the next upstream change goes and how one change is ported and recorded. Read
it before touching code that came from Dauntless Revived or Mystic Paradox, which is most of this workspace,
and cite its section rather than re-deriving the mapping in a brief or a commit.

## Repository layout

`restricted/` is the AGPL-3.0-only subtree and Cargo workspace of the Dainty Lass monorepo (RD27). It holds every AGPL
component and everything that links one; the monorepo root holds the MIT crates (`../crates/`), the UI (`../ui/`) and
the plan (`../MIGRATION_PLAN.md`, `../docs/port/`, `../docs/architecture.md`). This workspace takes the root MIT crates
via relative path dependencies (`path = "../crates/<name>"` in `Cargo.toml`, `[workspace.dependencies]`).

| Path | What it is |
| --- | --- |
| `restricted/` | One cargo workspace, every package of its own in `crates/<package name>`: `dainty-host-parity` (the Node host kit's tests and recorded output, run against the root's MIT `dainty-host`). The server binaries are the root's `dainty-server` and `dainty-server-minimal` (MIT since RD27). Also `launcher/src-tauri` (`dainty-launcher`, test-only: the oracle tests for the root's MIT launcher kit and the manager's embedded server; the apps are the root's `dainty-launcher-app`, spec-tauri-shell §12.1) and the game modules' crates |
| `modules/rustic-isles/` | The 1.14.7 module's version data (`version-data/1.14.7`), the recorded Node goldens replayed against it (`crates/rustic-isles-mit-parity`), the C++ CatalogExporter (`native/catalog-exporter`; it reads the host's SDK from `native/runtime`, which holds nothing else since the AGPL runtime was deleted at R21), and the Node reference tree (`reference/1.14.7`). In-tree since RD27, members of this workspace |
| `modules/rustic-rifts/` | Peer-to-peer realms' AGPL remainder: `oracle` (Mystic Paradox's Node vectors replayed against the MIT crates). The `rustic-rifts-*` crates themselves are the root's MIT clean-room ones (`../crates/rustic-rifts-*`, RD27) |
| `legacy/` | dainty-legacy: the 1.4.4 importer, a workspace of its own (an external plugin `settings.toml` pins) |
| `fuzz/` | `cargo fuzz` targets for the AGPL components, a workspace of its own |
| `docker/`, `compose.yaml` | Container deployment (RX4), laid out as the server release |
| `docs/` | Host and player guides, `paradox-patches.md`, and `security/` for the AGPL components |

Dainty Lass serves one game, Dauntless 1.14.7 (2026-10-31). On the default path (since the RU9 code cutover,
maintainer decision 2026-10-28), `host::serve` runs the compiled-in Rust `isles-1-14-7` backend (port 41000) and
in-process Director (port 41001; settings.toml's `[ports]` moves either) on `store` (SQLite by default, or Postgres), with `isles-1-14-7` and realms
linked in by the server binaries' `all-modules` feature (DF5). The module is the root's clean-room
`rustic-isles-1-14-7` (RD27), handed restricted's version data design at start
(`modules/rustic-isles/version-data/1.14.7/version_data.json`, kept current by rustic-isles-mit-parity's `version_data` test,
which also shows it and the root's MIT facts make the whole document);
the AGPL Rust ports it replaced (`isles-core`, `isles-db`, `isles-director`, `isles-chat`) were removed and stay in the
history. For one release after the cutover (until RB15), the
opt-in `paradox` Cargo feature on `host` retains Rustic Isles's Node backend and Director from
`modules/rustic-isles/reference/1.14.7/` and MongoDB as a rollback path.

The server is one executable (RD26). Every service is a module of the root's `controller`, composed by
`host::serve` (through the root's `crates/dainty-host/src/compose.rs`) from the files `setup` writes, and still runnable
alone as a hidden `dainty-host <service>` subcommand (the root's `crates/dainty-hostkit/src/cli.rs`), which Docker
and `start` use.
The same services also build as libraries the MIT core `dainty-lass` loads (RD27 phase 2, the root's `*-plugin`
crates, which the release and Docker build from the root workspace); `compose.rs` links them into restricted's own
binaries. No binary carries the 1.14.7 version data: the root's `dainty-host` reads Mystic Paradox's design at
start from the file `settings.toml`'s `version_data` names (default `data/1.14.7/version_data.json` in the server
folder, where the release and the server image ship it; a checkout falls back to
`modules/rustic-isles/version-data/1.14.7`), and the MIT facts beside it from `version_facts` (default
`data/1.14.7/version_facts.json`; a checkout falls back to the root's `crates/rustic-isles-1-14-7/data`).

**Which workspace a change goes in** follows its licence: anything ported from DR, Paradox or the Reforged Revival
line, or linking such code, goes here; code that is neither goes in the root workspace as MIT.

`docs/architecture.md` is the reference for how the parts talk and what each trusts. Section 3 of the plan,
"Things that must not change", lists what every port keeps byte for byte: read it before touching anything a
player, host or the game client already depends on.

## How work lands

One row is one agent in its own worktree and branch (RD18, section 5.1). **Commit and push the branch, report
the commit SHA, and open no pull request.** The integration pass merges branches into `plan` with `--no-ff` in
dependency order, runs the checks and strikes the rows.

- **Stay inside your row's files.** The ownership table in section 5.1 says who owns each shared file. If the
  row needs a change to a seam (the workspace `Cargo.toml`, `crates/dainty-protocol`, the bridge spec, `store`
  migrations, the `clap` tree, `.forgejo/`) or to a file another row owns, stop and report it. Don't make the
  change yourself.
- **Never edit** `MIGRATION_PLAN.md`, `README.md`, `NOTICE.md`, `SECURITY_REVIEW.md` or the `docs/` index pages
  from a row. Those belong to the integration pass. A row writes only its own page under `docs/`.
- **New launcher strings go at the end** of every language file, so the integration pass resolves only
  append-only conflicts.
- **Hands-on work isn't an agent's job:** the canary, cutovers, spikes that need a machine with the game, signing
  and publishing, the cut-off date. Write down the steps and leave them for the maintainer.
- `modules/rustic-isles/reference/1.14.7/` takes any change, with fixes as the baseline (RD2): keep a change small,
  and list a new or changed route in RU3's inventory (`modules/rustic-isles/docs/inventory/`). It reaches the MIT
  `rustic-isles-*` crates only through their specs (`../docs/port/spec-rustic-isles-*.md`), and `rustic-isles-mit-parity`
  replays the goldens against the result. The catch-up lists (`../docs/port/catchup-*.md`) are closed records.
- **An upstream change is placed before it is ported.** Section 4 of the migration guide says where each kind
  goes (a runtime patch, a backend patch, game data, a Mystic Paradox update);
  section 5 is the method and section 6 the catch-up entry it leaves behind. A row that adds a Rust module
  reports the line section 2 of the guide needs; the integration pass writes it.

## Rust

From the root: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
`cargo test --workspace --locked --no-fail-fast` (`--no-fail-fast` because one failing test binary otherwise
hides the ones after it). The toolchain is pinned in `rust-toolchain.toml`; `cargo deny check` runs in CI.

- **Lints that will fail the build:** `unsafe_code`, `unwrap_used`, `expect_used`, `dbg_macro` and `todo` are
  denied, with `unwrap` and `expect` allowed in tests only. Return errors instead. A panic in a request
  handler is caught, but it still drops the request.
- `max_width = 120`, edition 2024. A dependency goes in `[workspace.dependencies]` through the integration pass,
  never as a new version pinned in one crate.
- **Ports match Node exactly where a client can see it:** routes matched as Express matches them, JSON written
  as `JSON.stringify` does, the same status codes and field order. When the Node and Rust answers differ, the
  Rust one is the bug unless a decision row says otherwise.
- Code copied from Dauntless Revived (DR, at `e1c5fec`) keeps its structure. Make the Reforged changes the
  delta file lists and nothing else, so a later reader can diff it against DR. The guide's section 2 says
  which DR file a module came from and its section 3 (the rename dictionary) translates DR names, headers,
  routes and env variables to Reforged's; never carry a DR name across untranslated.

### Launcher bridge

The bridge is the root's MIT launcher kit's: the commands in `../crates/dainty-launcher-kit/src/bridge/spec.rs`, the
generated TS types, hook list and permissions in `../crates/dainty-launcher-kit/generated`, which the root's `ui/` and
`launcher/` copy in with `bun run restricted-data`. Nothing here generates or holds them. Never edit them by hand:
`RR_UPDATE_GENERATED=1 cargo test -p dainty-launcher-kit generated` at the root rewrites them. The page's method for each command is
the root's `ui/packages/bridge/src/api.ts`; the root's `launcher` tests (`bun run test`) drive it through the
isolation hook and fail a command no method calls. The account key never crosses the bridge.

## Security

Every row that writes a trust-boundary component delivers, with its code, its fuzz target in `fuzz/` and
its checklist in `docs/security/<component>.md` (RX3). Copy the structure of an existing one, such as
`relay.md`. The components are listed in the plan's RX3 row.

Rules that read like overcaution:

- **The gateway is the only listener off loopback.** Every other service binds 127.0.0.1. Host-only routes and
  headers stay refused at the gateway.
- **The account key (`DLK_...`) goes only to the server and certificate it was made for.** In public mode the
  certificate's SHA-256 from the invite is checked before any byte is sent. The gateway loads its existing
  certificate and never regenerates it, because every invite's `fp` depends on it.
- **Secrets and keys are never logged.** Keep the redaction in the gateway, the launcher and the content server.
- **The invite parser accepts nothing it doesn't recognise.** `dauntless-revived://` is a typed refusal (RD5), not
  a second path.
- **Inject only into a game this process just started** (RD9, amended 2026-11-12 and 13). The launcher and the
  Director start the game suspended, load only DLLs they chose by absolute path with checked hashes, and kill it
  rather than resume it half-loaded. PLAY still refuses a leftover `mystic_loader.ini`, a Host tools DLL, or a
  `winmm.dll` that doesn't stand down for an injected game (`../docs/launcher-injection.md`,
  ``../docs/director-injection.md`).

## Nothing from the game is committed

No game files, generated SDKs (`CppSDK/`, `SDK.hpp`), exported game data beyond the services' examples, or
`.workspace/`. Lady Luck's Store and the platinum store are built on each server from
`modules/rustic-isles/reference/1.14.7/ParadoxBackend/store-captures.json`. A server preset (`dainty-host preset`)
is a host's own and is never committed.

## Configuration

Every service reads one env file, given with `--env-file`, and nothing from its working directory
(the root's `crates/dainty-service-kit/src/env.rs`, DR X7). As with Node's `--env-file`, the process environment wins over the file.
**The env files are the host kit's output, not hand-written:** `dainty-host setup` writes them from
`settings.toml` and `secrets.env` (the root's `crates/dainty-hostkit/src/hostconfig.rs`, `settingstoml.rs`), and `start` hands each
service its own. So a new setting is a field in `settings.toml` (in `HostConfig`, and in `settingstoml.rs`'s key
table with a snake_case name and a comment for new files), a `setup` option where a host should choose it,
and a line in the service's file. It is never a variable the host is told to export.

### When you add, remove or rename a variable

It's never a one-file change:

- **The writer and the reader in the same commit.** `hostconfig.rs` writes it, the service's `config.rs`
  reads it (`from_env`), and a test checks the round trip. `update` reruns `setup` with the host's existing
  `settings.toml` (or a Node kit's `host.json`), so a new field that an older file doesn't have must default to
  today's behaviour.
- **Use one of the server prefixes** (`DAINTY_`, `GATEWAY_`, `ALLOWLIST_`, `CONTENT_`, `GAMESERVER_`...).
  `start` hands each service the shell's environment and then its env file, and the file wins
  (`service_environment` in the root's `crates/dainty-hostkit/src/services.rs`). The clean-room host kit has no
  `SERVER_PREFIXES` list, so a stale server setting left in the shell reaches a service whose file doesn't set it.
  Only the controller's registry filters, keeping the OS's own variables for an external module
  (`system_environment`).
- **A secret goes in `secrets.env`** (`SECRET_KEYS`), made once and never logged. It is never a `settings.toml`
  field and never an option on the command line.
- **The docs move with it.** The settings table in `docs/hosting.md`, and `hosting-linux.md` and
  `hosting-docker.md` wherever a host sees the option. Delete the rows when the variable goes. A stale row is
  worse than a missing one, because it reads as supported.

### Docker

`compose.yaml` and the root `.env.example` hold **only what Compose itself reads on the host**: profiles,
mounts, published ports, image names. Server settings stay setup's
(`docker compose run --rm host setup ...`), and every container mounts the volumes at the same paths, so the
env files setup writes are valid in all of them. A new Compose variable goes in `.env.example` with a comment,
and is interpolated as `${VAR:-default}` with the same default the comment gives. RX4 is kept current by the
rows that change what runs: a service added, moved or removed is a change to `docker/` in the same row.

## User-facing text

Every string a player or host reads comes from a language catalog in `en` and `pt-BR` (DU2). The pages' catalogs
are the root's `ui/apps/web/messages/`; the Tauri backend's own dialogs are the root launcher kit's MIT catalog
(`core::i18n`). Nothing here holds launcher text since the AGPL catalogs were retired (spec-launcher-catalogs C-1).

- **Both languages in the same commit**, with the same keys and the same `{placeholders}`, never an English
  placeholder in `pt-BR`. The `i18n` test fails a key the backend shows that a language lacks. When you aren't
  sure of the Brazilian Portuguese, write your best and flag it in the commit body for the maintainer.
- **Append new keys at the end** of each catalog (section 5.1), never in the middle.
- **Never build a sentence by concatenation.** `` `${name} is full` `` can't be translated, because not every
  language puts the subject first. Put the whole sentence in the catalog with a placeholder
  (`"Join {name}?"`), and have Rust return a code or a key, not English.
- Times render in the player's locale through `TIME_LOCALES`. Don't call `toLocaleString()` without one.
- **What stays in English:** server logs, the host kit's terminal output and the JSON the services answer.
  Those are a machine contract or a host's tool, not launcher text.

## Tests

From the root: `cargo test --workspace --locked --no-fail-fast`. The Paradox Node reference packages in
`modules/rustic-isles/reference/1.14.7/ParadoxBackend` and `ParadoxDirector`: `npm ci && npm test` from their
folders, Node 22 in CI. `bash docker/run-tests.sh` from the root builds the images and runs the server suites
and the compose smoke test inside them.

Not obvious from reading the suites:

- **Parity tests are the contract, not a snapshot.** The gateway's and content's, with the replies recorded from
  the Node services, are at the root since those crates went MIT (RD27). When one fails, fix the Rust side. So
  are Rustic Isles' goldens in `modules/rustic-isles/crates/rustic-isles-mit-parity/tests/golden`, replayed against the MIT
  module (890 of 890 replayed steps match; `docs/mit-parity-report.md` there).
- **the root's `crates/dainty-protocol/tests/cases/*.json` are shared tables**: the launcher and the servers check the same
  invites, statuses and usernames against them. A new case goes in the table, not in one crate's test.
- **Tests that read the shared log take `serial()`** (the root's `crates/dainty-gateway/tests/common`). A test that passes alone and fails in
  the suite is usually one that doesn't.
- **Bind test listeners to `127.0.0.1:0`.** On the maintainer's Windows PC fixed ports can fall in Windows'
  reserved range, which is why two of Content's Node tests fail there.
- **Generated files are tested for staleness.** If a `generated` test fails, regenerate
  with `RR_UPDATE_GENERATED=1` and commit the result; don't edit the output.
- **The real firewall test is opt-in** (`DAINTY_TEST_FIREWALL=1`: nftables on Linux with root or `CAP_NET_ADMIN`,
  Windows Firewall from an elevated prompt; the root's `crates/dainty-gateway/tests/firewall.rs`). Otherwise it
  skips, so `cargo test` passing says nothing about the rules. `docker/test-in-image.sh` turns it on when the
  container has the capability. Run that after a firewall change.
- Nothing here is played against the real 1.14.7 client in CI. When only the tests passed, say so; don't
  claim a gameplay change works.

## Native code

The game runtime is the root's MIT `rustic-isles-runtime.dll`. The AGPL C++ runtime retired at R21 and its
source was deleted (the tag `agpl-runtime-final` keeps it for the rollback DLL). A runtime patch written against
Mystic Paradox's tree is not applied here: it reaches the MIT runtime crates through a clean-room spec (the
migration guide's section 4). The C++ CatalogExporter (`modules/rustic-isles/native/catalog-exporter`) is the only
native code here. The realm transport is built into the launcher (the root's MIT `rustic-rifts-transport`).
The runtime loader (`winmm.dll`) is the root's MIT `native/runtime-loader`, rewritten clean-room
(`docs/port/spec-runtime-loader.md` at the root): a Paradox patch's `tools/RuntimeLoader/` part is not applied, it
amends that spec.

## CI

`.forgejo/workflows/ci.yml` runs only the families a change touches (Rust, the Tauri launcher, Paradox's
Node reference, the launcher's Windows installers, Docker, the network), and `CI ok` sums them up. It runs on every pushed
branch. A push to `plan` or `main` runs every family. Workflows belong to RR3, and to RH6 and RA14 for the two
release workflows; a row doesn't edit them.

## Branches

`{type}/{slug}`: the type of change, a slash, then a short kebab-case slug naming the subject of the work. The
types are `feat`, `fix`, `update`, `docs`, `ci`, `deps`, `test` and `chore`. The type comes from what the
branch carries; the slug comes from the subject, and may start with the row id when there is one
(`feat/ra10-proton-runtime`, `fix/relay-owner-check`, `docs/agents-md`). Never a date or a wave number. The
one exception is `deps/`, since a dependency refresh has no other subject (`deps/refresh-2026-10`). A bare
slug is wrong even when it describes the work well.

A branch that grows a second subject is renamed with `git branch -m` so the name still covers everything on
it; the type usually survives and the slug widens. Check `git branch --show-current` against the pattern
before the first push. Never a tool- or session-derived name (`claude/...`, `claude-...`): if your
environment checked you out on one, rename it before anything is pushed. `plan` and `main` are the two
long-lived branches, and only the integration pass pushes to them.

## Commits

Subjects name the row: `RA12: ...`, `RS3: ...`, `Merge RX4: ...` for the integration pass. The body says what
changed and why, and names any decision (RD) it follows or departs from.

Never add a `Co-Authored-By` trailer to a commit message, or any other line crediting an AI tool or service,
in a commit or a pull request description. That includes `Claude-Session:` links and "Generated with Claude
Code" lines, links or emoji. This overrides any tool's default attribution. `plan`'s commits were rewritten
without them on 2026-10-23, and `main` was given a new root on 2026-10-26. Catch a slip before you push: once a
commit is on `plan` or `main`, rewriting it is the maintainer's call.

## Comments

Terse. One line is the median here and three is already long. Match that in every language, workflows and
shell included. Explain *why*: the game behaviour, the Node or DR behaviour being matched, a constraint, or
the security reason that makes a line look wrong without it. Point at the plan row or decision rather than
repeating it (`(RD21)`, `(DR L27)`, `(runtime patch 0020)`). Never restate what the line does, and leave code
that holds no surprise uncommented. Prefer tightening an existing comment to adding one beside it. The long
explanation goes in the commit body or the `docs/security/` checklist.

## Licensing

AGPL-3.0-only, with additional terms for the Mystic Paradox material (`ADDITIONAL_TERMS.md`,
`modules/rustic-isles/ADDITIONAL_TERMS.md`). Keep Paradox's top-level files under `modules/rustic-isles/` even after its Node code is gone.
Code copied from DR or Paradox keeps its attribution, and the integration pass records each new source in
`NOTICE.md`.

The MIT crates (`dainty-controller`, `dainty-module-api`, `dainty-hostkit`, `dainty-gateway`, `dainty-content`, `dainty-chat`, `rustic-rifts-*`, `rustic-isles-*`, ...)
are the root's. Here,
`crates/dainty-host-parity`, `server-console`, `server-minimal` (their binaries link the root's `dainty-host` with
the version data), `launcher/src-tauri` and the game modules' crates inherit the workspace's AGPL
(`license.workspace = true`); `fuzz` and `legacy` set `AGPL-3.0-only` themselves. A new
crate is MIT only when nothing in it is ported from DR, Paradox or the Reforged Revival line: no file from those
trees, no function rewritten from one while looking at it, and such a crate belongs at the root. Copying a DR or
Paradox function into an MIT crate moves the crate to `license.workspace = true` in the same commit, with the NOTICE
line to match.
