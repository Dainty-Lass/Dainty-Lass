# AGENTS.md

Project-specific guidance for AI coding agents.

The port to one Rust codebase is done in code: no Node runs on a player's PC or a host on the default path, and
Rustic Isles is MIT clean-room crates at the root. What is left is mostly hands-on checks for the maintainer.
[`MIGRATION_PLAN.md`](MIGRATION_PLAN.md) lists it and holds the RD decisions; the DD decisions and the rows code
cites are in [`docs/dainty-lass-plan.md`](docs/dainty-lass-plan.md). Read "What's left" and the decisions before
starting anything, and the row you were given in full.
[`docs/port/migration-guide.md`](docs/port/migration-guide.md) is the map: where every upstream file now lives,
what each DR name became, where the next upstream change goes and how one change is ported and recorded. Read
it before carrying an upstream change across, and cite its section rather than re-deriving the mapping in a brief
or a commit. Nothing outside `restricted/` is copied from Dauntless Revived or Mystic Paradox: what came from them
was rewritten clean-room from the specs in `docs/port/` (RD27; the reviews are in `docs/port/clean-room/`).

## Repository layout

DaintyLass is one monorepo with two licence-separated Cargo workspaces (RD27): the root workspace holds only MIT code
(`exclude = ["fuzz", "restricted"]`); **`restricted/`** holds every AGPL component and the binaries that link them in
its own Cargo workspace (`restricted/Cargo.toml`), taking the root MIT crates via relative `path = "../crates/<name>"`
dependencies.

| Path | What it is |
| --- | --- |
| root | The MIT cargo workspace, every package in `crates/<package name>`: the libraries (`controller`, `module-api`, `epic`, `history-ship`, `hostkit`, `launcher-kit`, `plugin-host`, `protocol`, `proxy`, `remote`, `service-kit`, `service-plugin`, `store`), the services (`admin`, `gateway`, `content`), the core plugins (`network`, `chat`), the `*-plugin` libraries (`gateway-plugin`, `content-plugin`, `chat-plugin`, `rustic-isles-1-14-7-plugin`), the realms crates (`rifts-seam`, `rifts-admission`, `rifts-relay`, `rifts-realms`, `rifts-guard`, `rifts-transport`, `rifts-host`), the game module (`rustic-isles` and its areas `rustic-isles-*`, composed for 1.14.7 by `rustic-isles-1-14-7`) and the `dainty-lass-server` binary. `backend` and `director` are RR1's empty stubs, superseded by the `rustic-isles` crates |
| `fuzz/` | `cargo fuzz` targets for the MIT crates, a workspace of its own (excluded from the main one) |
| `ui/` | dainty-dash: the staff dashboard, the launcher and server-manager pages `restricted/`'s Tauri shells serve, and the fleet agent. MIT; its generated bridge types come from `restricted/` at build time (`bun run restricted-data`) |
| `launcher/` | The launcher's JavaScript side (RD27): the isolation hook, the scripts the Tauri shells and the release run, and their tests. The Rust shell (`restricted/launcher/src-tauri`) is in `restricted/` |
| `scripts/` | `replay-capture.mjs` |
| `docs/` | `architecture.md`, the plans, `port/` (the migration guide, delta and catch-up lists), `security/` for the MIT components (indexed in [`SECURITY_REVIEW.md`](SECURITY_REVIEW.md)) |
| `restricted/` | The AGPL-3.0-only workspace (`restricted/Cargo.toml`): `crates/` (`host` = the AGPL shell around `hostkit` that composes the services in `compose.rs`, the `*-plugin` crates, and the server executables `server-console` and `server-minimal`), `launcher/src-tauri` (Tauri: `dainty-launcher`, `dainty-manager`), `modules/rustic-isles` and `modules/rustic-rifts`, `legacy/` (the 1.4.4 importer), `native/runtime-loader`, `docker/`, `compose.yaml`, the Tauri backend's language catalogs (`restricted/ui/`), and the hosting, player and Paradox docs. [`restricted/AGENTS.md`](restricted/AGENTS.md) covers them |

**Which workspace a change goes in** follows its licence: anything ported from DR, Paradox or the Reforged Revival
line, or linking such code, goes under `restricted/`. A crate in root `crates/` is MIT and depends on nothing in
`restricted/` or AGPL; CI fails a root package that is not MIT or Apache-2.0 or that references `restricted/`.
`restricted/crates/host` composes the services into the controller in-process, and the MIT core
`dainty-core` (`crates/dainty-lass-server`) loads the same services as libraries named in `host.toml`'s `[services.<name>]` (RD27 phase 2,
`crates/plugin-host`).

DaintyLass serves one game, Dauntless 1.14.7 (2026-10-31). On the default path (since the RU9 code cutover,
maintainer decision 2026-10-28), restricted's `host::serve` runs the compiled-in MIT `rustic-isles-1-14-7` module:
the backend on port 61000 and `rustic-isles-director` on port 61001, on `store` (SQLite by default, or Postgres).
restricted's `isles-1-14-7-plugin` builds the same module as a library with restricted's version data.

The server is one executable (RD26). Every service is a module of `crates/controller`, composed by `host::serve` from
the files `setup` writes, and still runnable alone as a hidden `dainty-host <service>` subcommand
(`crates/hostkit/src/cli.rs`).

`docs/architecture.md` is the reference for how the parts talk and what each trusts. Section 3 of the plan,
"Things that must not change", lists what every port keeps byte for byte: read it before touching anything a
player, host or the game client already depends on.

## How work lands

One row is one agent in its own worktree and branch (RD18; the plan's section 2). **Commit and push the branch, report
the commit SHA, and open no pull request.** The integration pass merges branches into `plan` with `--no-ff` in
dependency order, runs the checks and strikes the rows.

- **Stay inside your row's files.** The ownership table in the plan's section 2 says who owns each shared file. If the
  row needs a change to a seam (the workspace `Cargo.toml`, `crates/protocol`, the bridge spec, `store`
  migrations, the `clap` tree, `.forgejo/`) or to a file another row owns, stop and report it. Don't make the
  change yourself.
- **Never edit** `MIGRATION_PLAN.md`, `README.md`, `LICENSE.txt`, `SECURITY_REVIEW.md` or the `docs/` index pages
  from a row. Those belong to the integration pass. A row writes only its own page under `docs/`.
- **New launcher strings go at the end** of every language file, so the integration pass resolves only
  append-only conflicts.
- **Hands-on work isn't an agent's job:** the canary, cutovers, spikes that need a machine with the game, signing
  and publishing, the cut-off date. Write down the steps and leave them for the maintainer.
- `restricted/modules/rustic-isles/reference/1.14.7/` (the Node reference) takes any change, with fixes as the
  baseline (RD2): keep a change small, and list a new or changed route in RU3's inventory
  (`restricted/modules/rustic-isles/docs/inventory/`). It reaches the MIT `rustic-isles-*` crates only through a
  clean-room spec (`docs/port/spec-rustic-isles-*.md`), and restricted's `isles-mit-parity` replays the recorded
  goldens against the result. The catch-up lists (`docs/port/catchup-*.md`) are closed records.
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
- Never copy DR, Paradox or Reforged Revival code here, and never rewrite a function from one while looking at it:
  an upstream change reaches this repository through a clean-room spec (RD27). The guide's section 3 (the rename
  dictionary) translates DR names, headers, routes and env variables to Reforged's; never carry a DR name across
  untranslated.

### Launcher bridge

The page talks to Rust only through the commands in `restricted/launcher/src-tauri/src/bridge/spec.rs`.
The TS types (`ui/packages/bridge/src/generated/types.ts`), the isolation hook's command list
(`launcher/isolation/commands.js`) and the permission set (`permissions/app-commands.toml`) are generated from it
there and checked by a test. `bun run restricted-data`, in `ui/` and in `launcher/`, copies the first two in from
`restricted/` (or `DAINTY_RESTRICTED_DIR`); the copies are never committed outside `restricted/`. Never edit them by
hand: `RR_UPDATE_GENERATED=1 cargo test -p launcher generated` in `restricted/` rewrites them. The page's method for each command is `ui/packages/bridge/src/api.ts`;
`bun run test` in `launcher` drives it through the isolation hook and fails a command no method calls. The account
key never crosses the bridge.

## Security

Every row that writes a trust-boundary component delivers, with its code, its fuzz target in `fuzz/` and
its checklist in `docs/security/<component>.md` (RX3). Copy the structure of an existing one, such as
`gateway.md`. The components are listed in the plan's RX3 row.

Rules that read like overcaution:

- **The gateway is the only listener off loopback.** Every other service binds 127.0.0.1. Host-only routes and
  headers stay refused at the gateway.
- **The account key (`DLK_...`) goes only to the server and certificate it was made for.** In public mode the
  certificate's SHA-256 from the invite is checked before any byte is sent. The gateway loads its existing
  certificate and never regenerates it, because every invite's `fp` depends on it.
- **Secrets and keys are never logged.** Keep the redaction in the gateway, the launcher and the content server.
- **The invite parser accepts nothing it doesn't recognise.** `dauntless-revived://` is a typed refusal (RD5), not
  a second path.
- **The launcher never injects** (RD9). DLLs reach the game only through the runtime loader's
  `mystic_loader.ini`, and PLAY refuses to start while a leftover ini or unknown DLL sits next to the loader.

## Nothing from the game is committed

No game files, generated SDKs (`CppSDK/`, `SDK.hpp`), exported game data beyond the services' examples, or
`.workspace/`. Lady Luck's Store and the platinum store are built on each server from
`restricted/modules/rustic-isles/reference/1.14.7/ParadoxBackend/store-captures.json`. A server preset (`dainty-host preset`)
is a host's own and is never committed.

## Configuration

Every service reads one env file, given with `--env-file`, and nothing from its working directory
(`crates/service-kit/src/env.rs`, DR X7). As with Node's `--env-file`, the process environment wins over the file.
**The env files are the host kit's output, not hand-written:** `dainty-host setup` writes them from
`host.toml` and `secrets.env` (`crates/hostkit/src/hostconfig.rs`, `hosttoml.rs`), and `start` hands each
service its own. So a new setting is a field in `host.toml` (in `HostConfig`, and in `hosttoml.rs`'s key
table with a snake_case name and a comment for new files), a `setup` option where a host should choose it,
and a line in the service's file. It is never a variable the host is told to export.

### When you add, remove or rename a variable

It's never a one-file change:

- **The writer and the reader in the same commit.** `hostconfig.rs` writes it, the service's `config.rs`
  reads it (`from_env`), and a test checks the round trip. `update` reruns `setup` with the host's existing
  `host.toml` (or a Node kit's `host.json`), so a new field that an older file doesn't have must default to
  today's behaviour.
- **Use one of the server prefixes** (`DAINTY_`, `GATEWAY_`, `ALLOWLIST_`, `CONTENT_`, `GAMESERVER_`...).
  `start` hands each service the shell's environment and then its env file, and the file wins
  (`service_environment` in `crates/hostkit/src/services.rs`). The clean-room host kit has no `SERVER_PREFIXES`
  list, so a stale server setting left in the shell reaches a service whose file doesn't set it. Only the
  controller's registry filters, keeping the OS's own variables for an external module (`system_environment`).
- **A secret goes in `secrets.env`** (`SECRET_KEYS`), made once and never logged. It is never a `host.toml`
  field and never an option on the command line.
- **The docs move with it.** The settings table in `restricted/docs/hosting.md`, and `hosting-linux.md`
  and `hosting-docker.md` wherever a host sees the option. Delete the rows when the variable goes. A stale row is
  worse than a missing one, because it reads as supported.

### Docker

Docker (`restricted/compose.yaml`, `restricted/.env.example`, `restricted/docker/`) is AGPL, and its rules are in
`restricted/AGENTS.md`. A service added, moved or removed at the root is a change to `restricted/docker/` too.

## User-facing text

Every string a player or host reads comes from a language catalog in `en` and `pt-BR` (DU2). The pages' catalogs
are `ui/apps/web/messages/<launcher|manager>/en.json` and `pt-BR.json`, checked by `ui/apps/web/tests/catalog.test.ts`.
The Tauri backend takes its own dialogs from `restricted/ui/packages/i18n/catalogs`, through the generated
`restricted/launcher/src-tauri/gen/strings.json`.

- **Both languages in the same commit**, with the same keys and the same `{placeholders}`, never an English
  placeholder in `pt-BR`. The catalog tests fail a key that one language lacks. When you aren't
  sure of the Brazilian Portuguese, write your best and flag it in the commit body for the maintainer.
- **Append new keys at the end** of each catalog (the plan's section 2), never in the middle.
- **Never build a sentence by concatenation.** `` `${name} is full` `` can't be translated, because not every
  language puts the subject first. Put the whole sentence in the catalog with a placeholder
  (`"Join {name}?"`), and have Rust return a code or a key, not English.
- Times render in the player's locale through `TIME_LOCALES`. Don't call `toLocaleString()` without one.
- **What stays in English:** server logs, the host kit's terminal output and the JSON the services answer.
  Those are a machine contract or a host's tool, not launcher text.

## Tests

From the root: `cargo test --workspace --locked --no-fail-fast`. In `ui/`: `bun run test` and `bun run lint`
(biome). In `launcher/`: `bun run test`, which runs `bun run restricted-data` first. In `restricted/`: its own
`cargo test --workspace --locked --no-fail-fast` (on the maintainer's PC with `-j 6` or fewer). The Paradox Node
reference, Docker and the Tauri shell are tested there.

Not obvious from reading the suites:

- **Parity tests are the contract, not a snapshot.** `crates/gateway/tests/parity/` and
  `crates/content/tests/parity/` hold replies recorded from the Node services. When one fails, fix the Rust
  side. Re-record only when the Node side changed on purpose, and say why in the commit.
- **`crates/protocol/tests/cases/*.json` are shared tables**: the launcher and the servers check the same
  invites, statuses and usernames against them. A new case goes in the table, not in one crate's test.
- **Tests that read the shared log take `serial()`** (`crates/gateway/tests/common`). A test that passes alone and fails in
  the suite is usually one that doesn't.
- **Bind test listeners to `127.0.0.1:0`.** On the maintainer's Windows PC fixed ports can fall in Windows'
  reserved range, which is why two of Content's Node tests fail there.
- **Generated files are tested for staleness.** If a `generated` test fails, regenerate
  with `RR_UPDATE_GENERATED=1` and commit the result; don't edit the output.
- **The real nftables test is opt-in** (`DAINTY_TEST_NFT=1`, Linux, root or `CAP_NET_ADMIN`). Otherwise it
  skips, so `cargo test` passing says nothing about the rules. `restricted/docker/test-in-image.sh` turns
  it on when the container has the capability. Run that after a firewall change.
- Nothing here is played against the real 1.14.7 client in CI. When only the tests passed, say so; don't
  claim a gameplay change works.

## Native code

There is none outside `restricted/`. The game runtime, CatalogExporter, the runtime loader and `ParadoxTransport.dll`
are there, and `restricted/AGENTS.md` says how a runtime patch is applied.

## CI

`.forgejo/workflows/ci.yml` runs only the families a change touches (each workspace's Rust, with `cargo deny` and
the fuzz targets, and the UI), and `CI ok` sums them up. It runs on every pushed branch. A push to `plan` or `main`
runs every family. Workflows belong to RR3, and a row doesn't edit them. `server-release.yml` and
`launcher-release.yml` build from `restricted/`.

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

MIT ([`LICENSE.txt`](LICENSE.txt)); `ui/` carries its own copy (`ui/LICENSE`). The workspace's `license` is `"MIT"`
and every crate in `crates/` sets `license = "MIT"` itself; `cargo deny` refuses a dependency whose licence can't
ship in the AGPL-3.0-only release built from `restricted/`.

A crate here is MIT only while nothing in it is ported from DR, Paradox or the Reforged Revival line: no file from
those trees, no function rewritten from one while looking at it. Code that is ported belongs in `restricted/`, which
keeps its attribution and records each source in `restricted/NOTICE.md` (and the Mystic Paradox additional terms in
`restricted/ADDITIONAL_TERMS.md`). Never copy such a function into a crate here; port the behaviour from a clean-room spec
instead (RD27).
