<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false, "code_blocks": false } } -->

# RD27 Clean-Room Specification: the host shell (`crates/host` in dainty-restricted)

This is the specification writer's output for relicensing what is left of dainty-restricted's host shell to MIT. It
follows the format of [`spec-dl3-hostkit.md`](spec-dl3-hostkit.md) (DL3a) and the measurement rules of
[`clean-room/similarity.md`](clean-room/similarity.md). It covers every file under `crates/host/` in
dainty-restricted at branch `feat/rd27-plugin-abi`, commit `849f45e`.

> **Clean-room rule.** The implementer of a Category B item reads this document, the MIT crates in dainty-lass
> (`crates/hostkit`, `crates/controller`, `crates/module-api`, `crates/remote`, `crates/store`, `crates/protocol`)
> and nothing else: not `crates/host` or `rust/services/host` at any commit of either repository, not
> `legacy/ReforgedHost` (Reforged's Node kit). Category A files are moved, not rewritten. Nothing written from this
> document names Reforged's Node files, DR commits or Node identifiers in comments.

The launcher half of RD27's relicensing is [`spec-rd27-launcher.md`](spec-rd27-launcher.md).

---

## 0. What is left of the shell

DL3b already moved the host kit's behaviour into `crates/hostkit` under MIT. Every area this row was asked to
cover is there, not in the shell:

| Area | Where it lives now | Specified by |
| --- | --- | --- |
| The `reforged-host` command line: every subcommand, its options, outputs and exit codes (`setup`, `start`, `serve`, `invite`, `doctor`, `gamedata`, `publish-runtime`, `install-game`, `preset`, `multipliers`, `update`, `backup`, `db`, `network`, `remote`, `history`, `revert`, `legacy`, the hidden service subcommands, `gameserver`) | `crates/hostkit/src/cli.rs`, `commands/*` | DL3 §2.20–2.22 (B) and §1.1 (A) |
| `serve`: the in-process controller, its registry, the `[services.*]` libraries (RD27) | `crates/hostkit/src/serve.rs` (A) | DL3 §1.1; `controller::service_library` |
| `gamedata`, `preset`, `publish-runtime`, game-data packs, captures, progression | `crates/hostkit/src/{gamedata,gamepack,preset,progression,publish,captures}.rs` | DL3 §2.9–2.16 |
| The console and dashboard (`repl`, `tui`) and `console_main` | `crates/hostkit/src/{repl,tui,lib}.rs` (A) | DL3 §1.1 |
| The seam the shell plugs into | `crates/hostkit/src/hooks.rs` (`HostHooks`, `set_hooks`, `get`) (A) | this document §3 |

What remains in dainty-restricted is the code that **links an AGPL crate** (`gateway`, `content`, `chat`,
`isles-1-14-7`, `isles-core`, `isles-db`, `isles-director`) and installs itself into hostkit through `HostHooks`,
plus the host kit's test suite. That is 2 source files (`src/lib.rs`, `src/compose.rs`), the manifest, 15 test
files and 99 fixture files.

The licence question for this code is mostly about **linkage, not provenance**. Most of it was written for
DaintyLass, but it exists to call AGPL APIs, so the binaries it builds are AGPL whatever its source licence, and
RD27's repository rule ("anything ... linking such code is this repository's", restricted `AGENTS.md`) keeps it in
dainty-restricted until the AGPL services are libraries the MIT core loads (RD27 phase 2). Section 4 is the plan.

---

## 1. File classification

### 1.1 Evidence

- **History.** dainty-restricted keeps the full history since the new root (`7338b10`, 2026-10-26); dainty-lass
  keeps the objects of the earlier `plan` history (`b190010`, `758f8a3`). The shell's `lib.rs` was rewritten as a
  thin hooks implementation in `758f8a3` ("DL3b+DD26: swap crates/host to clean-room MIT hostkit crate") after the
  clean room, then extended by RM2/RM5 (`4bc4b15`), RU9 (`bbe936e`, `19f220f`), DN5–DN10 (`728d44c`, `1bd902d`),
  DD1/DD9 and RD27 (`62d56c6`, which also moved `compose.rs` from dainty-lass's `controller::modules`).
- **NOTICE.** restricted `NOTICE.md`, "This project": `crates/host` is "the shell that wires dainty-lass's
  `hostkit` to `gateway`, `content`, `chat` and `isles-1-14-7`; its `compose.rs` is MIT code moved from
  dainty-lass's `controller::modules` and keeps its MIT header".
- **DL3.** `spec-dl3-hostkit.md` §1.3 classified the old `lib.rs` as the AGPL shell because it *links* AGPL
  services, not because it was ported; §1.1 classified `commands/db.rs` and `serve.rs` (whose code the new
  `lib.rs` partly carries) as Category A; §1.2 classified `gamedata.rs` as Category B.
- **Similarity.** The `similarity.md` measure (lines of trimmed, non-comment code that sit in a run of 3 or more
  identical lines), taken against every file of a baseline at once rather than one counterpart file. Two
  baselines: dainty-lass `b190010:rust/services/host` (the shell and host kit before the clean room,
  Reforged-derived in part) and DR's `rust/` at `e1c5fec` (`legacy-revived` keeps it).

| File | Lines (code) | vs `b190010` host | vs DR `rust/` | Where the matched runs come from |
| --- | ---: | ---: | ---: | --- |
| `src/lib.rs` | 2018 | 49.4% | 8.7% | old `lib.rs` (A per DL3: service mains, `BUILT_IN_SERVICES`), old `commands/db.rs` (A), old `serve.rs` (A), and one 68-line run from old `gamedata.rs` (B per DL3): the generator dispatch |
| `src/compose.rs` | 949 | 8.7% | — | dainty-lass `controller::modules` (MIT) |

### 1.2 Categories

- **Category A**: born in DaintyLass, no DR, Reforged or Paradox derivation. Moves to MIT as it is (its SPDX
  header changes to MIT, or keeps the one it has). The table notes what it depends on, because a dependency on an
  AGPL crate decides where it can go, not its licence.
- **Category B**: derived from DR, Reforged's Node kit or Paradox. Rewritten clean-room from §2.
- **A with a B part**: an A file holding one derived piece; only that piece is rewritten.
- **B-data**: test data produced by, or copied from, a derived program. Not code; see §4.

### 1.3 `crates/host/` file by file

| Path | Category | Evidence | Depends on | Target (§5) |
| --- | --- | --- | --- | --- |
| `Cargo.toml` | A | Written in DL3b and RD27; features name only DaintyLass crates and the AGPL services | — | restricted, shrinks with phase 2 |
| `src/compose.rs` | A | SPDX MIT header; moved by RD27 from dainty-lass `controller::modules` (8.7% vs `b190010`, the matches are that file) | **AGPL**: `gateway`, `content`, `chat`; MIT: `controller`, `service-kit`, `network_plugin`, `store`, `module-api` | restricted until phase 2 deletes it |
| `src/lib.rs` | A with a B part | Written in `758f8a3` after the clean room, then RM/RU/DN rows; B part: the generator dispatch carried from the pre-clean-room `gamedata.rs` (DL3 §1.2 item 11) | **AGPL**: `gateway`, `content`, `isles-1-14-7`, `isles-core`, `isles-db`, `isles-director`; MIT: `hostkit`, `controller`, `remote`, `network_plugin`, `service-kit`, `module-api`, `store`, `protocol` | split, §5.1 |
| `tests/admin_network.rs` | A | DN11/DD16/DD23 three-node admin tests; 1.0% vs `b190010` | `admin`, `controller`, `store` (MIT); `host` | restricted (links the shell) or dainty-lass once it targets the core |
| `tests/controller_allowlist.rs` | A | DF4d; 2.6% | `gateway` (AGPL) through `host::compose` | restricted |
| `tests/controller_network_wiring.rs` | A | DN3/DN13 wiring; 2.2% | `gateway` (AGPL) through `host::compose` | restricted |
| `tests/history.rs` | A | DH3b (born here); moved unchanged (99.6% vs `b190010`, which already had it) | `hostkit`, `admin`, `diesel` | dainty-lass `crates/hostkit/tests` |
| `tests/install_game.rs` | A | DE6 (born here) | `hostkit`, `epic` | dainty-lass `crates/hostkit/tests` |
| `tests/network.rs` | A | DN3 (born here) | `hostkit` | dainty-lass `crates/hostkit/tests` |
| `tests/config.rs` | B | Header: "The Node kit's tests of host.json, the secrets and the service env files"; ported in RH1 | `hostkit` + Node fixtures | restricted (§4.2) |
| `tests/doctor.rs` | B | "the Node kit's tests for its checks"; RH3 | `hostkit` | restricted |
| `tests/gamedata.rs` | B | "The Node kit's tests of the game-data commands", compared byte for byte with Node output | `hostkit` + Node fixtures | restricted |
| `tests/invite.rs` | B | "the Node kit's invite tests"; RH3 | `hostkit` | restricted |
| `tests/mongo.rs` | B | "the Node kit's tests"; RH3 | `hostkit` | restricted |
| `tests/node_fixtures.rs` | B | Reads the files the Node kit wrote and checks the bytes | `hostkit` + Node fixtures | restricted |
| `tests/setup.rs` | B | "the Node kit's tests for them", plus `rh3.json` from the Node kit's own code | `hostkit` + Node fixtures | restricted |
| `tests/start.rs` | B | RH2: the supervisor contract of the ported `start` (DL3 §1.2 item 21) | `hostkit` | restricted |
| `tests/update.rs` | B | RH5, the ported `update` | `hostkit` | restricted |
| `tests/fixtures/node/gen.ts`, `gen-gamedata.mjs`, `rh3/gen.ts`, `rh3/run.sh` | A | Written here to run the Node kit's own sources and record their output; they import those sources at run time | **Reforged's Node kit** (AGPL) at run time | restricted (useless without the Node sources) |
| `tests/fixtures/.gitattributes` | A | Line-ending rule | — | with the fixtures |
| `tests/fixtures/node/{posix,windows}/**` (env files, `host.json` files, `secrets.env`; 60 files) | B-data | Written by the Node kit's `hostconfig.ts`/`envfile.ts`; carry its header line | — | restricted |
| `tests/fixtures/node/rh3/rh3.json` | B-data | Written by the Node kit's `cli.ts`/`invite.ts`/`hostconfig.ts` | — | restricted |
| `tests/fixtures/node/gamedata/**`, `progression/**` (33 files) | B-data | Inputs and outputs of the Node kit's game-data code (`gen-gamedata.mjs`) | — | restricted |

### 1.4 Counts

| Category | Files |
| --- | ---: |
| A | 13 (`Cargo.toml`, `compose.rs`, 6 tests, 4 fixture scripts, `.gitattributes`) |
| A with a B part | 1 (`lib.rs`) |
| B | 9 (tests) |
| B-data | 94 (fixtures) |
| **Total** | **117** |

The only Category B *source* is the 68-line generator dispatch inside `lib.rs`.

---

## 2. Behaviour specification of the Category B parts

### 2.1 The game-data generator hook

**What it is.** hostkit's `gamedata` command runs a list of steps (DL3 §2.9). Each step names a
`hostkit::gamedata::GeneratorKind`; hostkit asks `HostHooks::run_generator(paths, step)` to run it in process. The
shell answers by calling Rustic Isles's generators. This is the only derived code in the shell. Its destination is
AGPL whatever its licence (it calls `isles-core` and `isles-director`), so §5 places it in the Rustic Isles module,
and rewriting it is only needed if the maintainer wants that glue clean too.

**Contract.** `run_generator(paths, step) -> Result<hostkit::run::RunResult, HostError>`:

- It never returns `Err`: a generator failure is a `RunResult` with `code: 1`.
- On success: `code: 0`, `output` = the generator's report text, `last_line` = the last non-empty line of
  `output` (each line trimmed), cut to its first 300 characters, or `None`.
- On failure: `code: 1`, `output` = the error text, `last_line` = the **first** non-empty trimmed line of the
  error, cut to 300 characters.
- A build without the 1.14.7 module answers every kind with `code: 1` and the error text
  `"<label> is unavailable: this binary was built without the isles-1-14-7 feature"`, where `<label>` is
  `GeneratorKind::label()`.

**Per kind** (all paths are `hostkit::paths::Paths` fields; "write" means the generator's write flag is on; when a
kind runs two generators their reports are joined with one `\n`, first then second):

| `GeneratorKind` | Generator(s) to call (public API of the 1.14.7 module crates) | Inputs | Output |
| --- | --- | --- | --- |
| `ProgressionConfig` | `isles_core::gamedata::generate_progression_config` | `raw_game_data` | `backend_game_data`, write |
| `HuntTables` | `isles_director::hunt_tables::import_hunt_tables` with: export dir `exports/hunts_1_14_7`, vendor dir `director_game_data`, repo root `repo`, apply on, update-existing off, verbose off, no backup stamp; the answer is the report's `output` text | `exports/hunts_1_14_7` | `director_game_data` |
| `SlayersPath` | `isles_core::gamedata::generate_slayers_path` | `exports/slayers_path_1_14_7/player_journey_nodes.jsonl` | `backend_game_data/slayers_path.json`, write |
| `SlayersPathDefinitions` | `isles_core::gamedata::generate_slayers_path_definitions`, version tag `1_14_7` | the same JSONL | `backend_game_data/slayers_path_definitions.json`, write |
| `RewardCache` | `generate_reward_cache_store`, then `generate_inventory_storage_policy` (no override file) | `exports/catalog_1_14_7.jsonl` | `backend_game_data/reward_cache_store.json`; the policy into `backend_game_data` |
| `LadyLuckGenerate` | `isles_core::gamedata::generate_ladyluck_store` | `raw_game_data` | `backend_game_data`, write |
| `LadyLuckEnrich` | `enrich_ladyluck_store`, then `generate_inventory_storage_policy` | `exports/catalog_1_14_7.jsonl`, the live capture `store_captures/Store/Product/Skus/ladyluckstore.json` | `backend_game_data/ladyluck_store.json`; the policy |
| `PlatinumAndStoragePolicy` | `generate_platinum_store`, then `generate_inventory_storage_policy` | the export files `resolve_platinum_export_file(exports, "catalog")` and `(exports, "store_item_images")`, the capture folder `store_captures/Store/Product/Skus`; the policy reads `exports/catalog_1_14_7.jsonl` | `backend_game_data` |

**Tests to write.** One per kind: with the module compiled in, a missing input file gives `code: 1` and a
`last_line` naming the first error line; the feature-off build gives the exact unavailable message; a successful
two-generator kind joins both reports with `\n` and takes its `last_line` from the second.

### 2.2 The Category B tests

These are the parity oracle for the MIT host kit: they hold the Node kit's recorded outputs and check hostkit
against them byte for byte. Their subjects, for a maintainer who decides to replace them (§4.2):

| File | What it checks |
| --- | --- |
| `config.rs` | `host.json`/`host.toml` reading and writing, `secrets.env`, and each service env file for the private and public modes on Windows, Linux and macOS paths, byte for byte against the Node fixtures |
| `doctor.rs` | each `doctor` check's level and line, and the printed report |
| `gamedata.rs` | packs (`--export`/`--import`), presets, progression, captures, runtime publishing; files compared with the Node kit's output |
| `invite.rs` | invite links for each mode and the `invite` command against a fake backend |
| `mongo.rs` | the replica-set check against a fake `mongod` speaking `OP_MSG`, and `backup` with a mocked `mongodump` |
| `node_fixtures.rs` | the Node-written `host.json` and env files read and written back unchanged |
| `setup.rs` | `setup` and `multipliers`: the same `host.json` as the Node kit for the same options (`rh3.json`), a fresh private and public setup with the PC's tools mocked |
| `start.rs` | `start`'s supervisor: the ready line, `stop` on stdin, stdin closing, Ctrl+C, nothing left running (the binary runs itself in fake roles through `HOST_TEST_ROLE`) |
| `update.rs` | `update` and setup's install step against made-up releases (`.zip` and `.tar.gz`) in a folder and behind a local HTTP server |

A clean-room replacement writes new cases from DL3's behaviour sections, with expected outputs derived from the
spec rather than recorded from Node. Section 4.2 explains why the recommendation is to keep these in
dainty-restricted instead.

---

## 3. Behaviour of the Category A shell (for the move, not for a clean room)

Category A code moves as it is; this section exists so that §5's split is unambiguous.

### 3.1 The hooks (`HostHooks` implemented by the shell)

`ensure_hooks()` installs the shell's hooks into hostkit (`hostkit::hooks::set_hooks`); every public entry point
of the shell calls it first, then delegates to the same-named hostkit function (`run`, `service_main`,
`host_main`, `console_main`, `doctor::run_doctor`, `doctor::store_database_check`, `gamedata::*`,
`serve::{compose, new_registry, ensure_server_files, run, prepare_embedded, minimal_main}`,
`commands::{dispatch, gamedata, publish_runtime, db, doctor, setup, start, update}`). `host_main` prints
`"\nError: <error>\n"` to stderr and exits 1 on an error.

| Hook | Behaviour |
| --- | --- |
| `built_in_services` | The service names this build runs alone, in order: `gateway`, `allowlist`, `content`, `admin` (each when its feature is on), `backend`, `director` (with the 1.14.7 module), `remote-agent` (always) |
| `run_generator` | §2.1 |
| `db_migrate(source_uri, db, dry_run)` | `isles_db::run_migrate` from the Mongo URI into the store target (SQLite file path, or `postgres://user[:password]@host:port/db` with the store's remote-Postgres flag), node `local`, all areas, no game-data dir; answers the per-table counts and no mismatches. Without the module: error `database migration is not compiled in` |
| `db_verify(source_uri, db)` | `isles_db::run_verify` the same way; answers `(collection, "expected count=<n> sha256=<h>, actual count=<n> sha256=<h>")` for each collection that differs; a `VerificationFailed(n)` error becomes one row `("<n> collection(s)", "verification failed")`. Without: `database verification is not compiled in` |
| `db_export_mongo(uri, out)` | `isles_db::run_export_mongo` from the store at `out` into the URI, all areas; answers the counts. Without: `MongoDB export is not compiled in` |
| `run_db_command(paths, args, out)` | The `db` subcommands `migrate`, `verify`, `export-mongo` and `replay` (§3.2). Without the module: error `this build does not include the Rustic Isles database tools (rebuild with --features isles-1-14-7)` |
| `compose_in_process_allowlist` | With the allowlist feature: makes the allowlist slot, adds the allowlist module from `allowlist.env`, answers the slot |
| `compose_in_process_content` | With the content feature: adds the content module from `content.env` |
| `compose_in_process_network_and_gateway` | With the gateway feature and `[network]` peers: loads the node certificate, key and cluster CA from the TLS folder, refuses (`the node certificate in <dir> is for <node> in <region>, but host.toml says <node> in <region>`) when the certificate's identity is not `host.toml`'s, loads or makes the session signing key, and adds the network module (leader, peers, database, game version, registry, allowlist slot, 5 s replication, `workspace/peer-logs`, the optional history peer with its state file `config/history-peer-state.json`, the wrong-node policy). Then, unless `[services.gateway]`/`[services.chat]` name a library, the in-process gateway (from `gateway.env`) and chat (from `gateway.env`, the node identity or standalone, the node database, the network leader and peers). Then, for a public, unguarded server without a gateway library: the allowlist guards the game's UDP ports (`hostconfig::game_ports`), or is marked absent in the opt-in Node build |
| `compose_service_library` | Delegates to `hostkit::serve::add_service_library` (the per-service compose functions are RD27 phase 2's) |
| `ensure_gateway_cert` | Public mode without a certificate file: makes one for `host.toml`'s host and prints `- gateway certificate made in <tls dir> (fingerprint <sha256>)` |
| `game_modules` | The compiled-in game module factories (`compose::game_modules`) |
| `run_service(name, args)` | Dispatches `gateway`, `allowlist`, `content`, `admin`, `backend`, `director`, `remote-agent` to §3.2's mains; `None` for any other name |

### 3.2 The service mains and the `db` subcommands

- **Env file of a service** (`<name> [serve] [--env-file <path> | --env-file=<path>]`): a leading `serve` is
  skipped; any other argument is refused (`unexpected argument: <arg>`, or `--env-file requires a path`). With no
  option the file is `<workspace>/.workspace/config/<name>.env` when it exists, else `./.env` when it exists, else
  none. The server folder is inferred from an env file whose parent is `config` inside `.workspace`; otherwise it
  is hostkit's `paths::locate_repo_root`.
- **`gateway`**: `make-cert`, `--help`, `-h`, `--version`, `-V` and any unreadable argument go to the gateway's own
  CLI unchanged. With `REFORGED_NETWORK_FULL_NODE=1` and a `host.toml`: the full-node test harness (below).
  Without a remote-controller state file and CA in the server folder, or when either does not load: the gateway's
  own CLI. Otherwise the gateway runs with node links, and once its links exist it adds every enrolled,
  unrevoked install as a peer and answers enrolment requests: the body is decoded as an enrolment request (400
  `{"error": ...}` when it does not decode); the state is reloaded from its file when that file exists; a redeemed
  token saves the state, adds the node as a peer and answers 200 with the encoded answer (500 when encoding fails);
  an invalid, used or expired token answers 403, any other error 400, each as `{"error": "<message>"}`. The remote
  controller service is attached to the links.
- **Full-node harness** (tests and the network rehearsal only): seeds the embedded example game data, writes a
  minimal synthetic progression table and hunt tables, a gameserver wrapper script that prints
  `RUSTIC_ISLES_GAMESERVER_READY launchId=<id> port=<port>` and sleeps, the server files, then sets the director's
  memory guard and start wait to 0 and the allowlist to dry run with private addresses allowed, composes and starts
  the whole controller, logs every autoloaded module that failed, and stops on the stop signal.
- **`remote-agent`**: finds the server folder as above (`Configuration: could not locate server workspace` when it
  cannot) and runs `hostkit::commands::remote::sync_with_controller` continuously; any error is printed and exits 1.
- **`backend` / `director`**: the 1.14.7 module alone, with only that service. `backend` first refuses unmigrated
  MongoDB data (hostkit's check, or, without `host.toml`, the presence of the Mongo data folder without the
  migrated marker; message names `db migrate`, `db verify`, `dainty-legacy import reforged` and
  `setup --fresh-start`). The module config is the defaults, plus: the env file path under `backend.env_file`,
  `PORT` as the service's port, `PORT_RANGE_BEGIN`..`PORT_RANGE_END` as the `game` range, the backend and director
  game-data folders when they exist, and `host.toml`'s module switches. The node identity is `[network]`'s or
  standalone; module log calls print `<module> | <level>: <message>`. Stops on the stop signal.
- **`db migrate|verify|export-mongo|replay`** (`run_db_command`): the Mongo URI, store target, node (`--node`,
  else `[network].node`, else `local`), areas (comma-separated, repeatable) and game-data folder are resolved as
  hostkit's `db` resolves them; errors have credentials redacted. Text output: `reforged-host db migrate
  (<dry-run|migrated>, from <uri> to <target>):`, then `  <table>: <count>` for each non-zero count, then
  `  rb11 transforms: approvals=<n>, inventory_arrays=<n>, starter_instances=<n>, loadout_slots=<n>` and
  `  skipped_invalid: <n>` when non-zero. `verify` prints `reforged-host db verify: OK (<n> collections verified
  against <uri> for <target>)` and `  [OK] <name> (area=<a>, count=<n>, sha256=<first 16>)`; a difference fails
  with `db verify failed: differences in [<names>]`; a full-area success writes the migrated marker
  (`verified against <uri>`). `export-mongo` mirrors `migrate`. `replay` takes its keys from the options, else
  `secrets.env`, prints a `PASS`/`DIFF` summary line, a diffs-by-kind line and one line per differing entry, and
  exits 2 on a difference. `--json` prints the report as pretty JSON instead. URIs are shown with the password
  redacted.

### 3.3 `compose.rs`

The AGPL services as `controller::Module`s, each built from its env file through its own `Config::from_env`:
`allowlist_slot`/`allowlist`/`allowlist_absent`/`allowlist_external`, `gateway`/`gateway_library`,
`content`/`content_library`, `network`/`network_with_sink` (with `NetworkOptions`), `chat`/`chat_library` (with
`ChatOptions`, `ChatNetworkOptions`), `game_modules`, and the lists `BUILT_IN` and `ALL`. Already MIT-headed.

---

## 4. What cannot be relicensed, and why

### 4.1 The AGPL links

`lib.rs` and `compose.rs` call into `gateway`, `content`, `chat` and the 1.14.7 module crates. Their *source* is
DaintyLass's own (but for §2.1) and could carry an MIT header, but:

- every binary that links them is AGPL, so an MIT header buys nothing while they link;
- the repository rule of RD27 keeps linking code in dainty-restricted.

So the shell is not relicensed; it is **emptied** (§5): its generic parts move into hostkit, and its linking parts
move into the AGPL libraries themselves as phase 2 lands, after which `crates/host` has nothing left.

### 4.2 The Reforged-derived tests and the Node fixtures

The 9 Category B tests are ports of the Node kit's tests, and the 94 fixture files are the Node kit's own output
(the env files carry the Node kit's header line, `host.json` its key order and number text). They are the evidence
that hostkit writes the same bytes as the kit hosts already run (plan section 6, "Host kit"). A clean-room path:

1. write new tests in `crates/hostkit/tests` from DL3's behaviour sections, with expected values computed from the
   spec (for example a whole env file written out in the test from DL3 §2.3's template table);
2. keep these tests in dainty-restricted, running against the MIT hostkit, until (1) covers the same cases;
3. then delete them, or keep them as an AGPL parity suite.

Recommendation: do (2) now and (1) only if the maintainer wants the parity suite out of dainty-restricted. Tests in
dainty-restricted that exercise an MIT crate do not change the crate's licence. The one recorded string the MIT
code itself must reproduce (the env files' first header line) is already in MIT hostkit as a byte-compatibility
constant, a precedent DL3 set.

### 4.3 The fixture scripts

`gen.ts`, `gen-gamedata.mjs` and `rh3/gen.ts` are DaintyLass's, but they only work by importing the Node kit's
AGPL sources. They stay with the fixtures.

---

## 5. Target MIT design

### 5.1 Where each part of the shell goes

| Part (today in `crates/host`) | Category | Goes to | Notes |
| --- | --- | --- | --- |
| Env-file resolution of a service, server-folder inference from an env file | A | dainty-lass `crates/hostkit` (`serve` or a new `service` module), used by `hostkit::service_main` | Generic; no AGPL dependency |
| `remote-agent` service main | A | dainty-lass `crates/hostkit` (`service_main` answers `remote-agent` itself; `BUILT_IN_SERVICES` includes it) | Depends on `remote` (MIT) only |
| Remote enrolment handler of the gateway service | A | dainty-lass `crates/remote` as a function over `RemoteControllerState`, `ClusterCa` and a peer-adding callback; the gateway library calls it | The gateway call itself stays AGPL |
| `backend`/`director` standalone mains | A | dainty-lass `crates/hostkit`: a generic "run one external module's service" over `plugin-host` and `[[modules.external]]`, so no build links the module | Replaces the in-tree link to `isles-1-14-7` |
| `ensure_gateway_cert` | A | the gateway library (restricted) | Calls the gateway's certificate maker |
| `compose_in_process_*`, `compose.rs` | A | deleted when the content, chat and gateway libraries land (RD27 phase 2); until then restricted | `controller::service_library` already composes a library from its env file |
| `db_*` hooks, `run_db_command`, the `db` output | A | the 1.14.7 module (restricted), exposed as module commands or a module-provided hook | `isles_db` is AGPL |
| Generator dispatch (§2.1) | B | the 1.14.7 module (restricted) | Clean-room rewrite optional: its destination is AGPL |
| Full-node harness | A | restricted test support, or dainty-lass `crates/dainty-lass-server` tests once libraries are loadable | Test-only |
| Wrapper modules (`ensure_hooks` then delegate) | A | deleted: `crates/dainty-lass-server` already sets its hooks once in `main` | — |
| Category A tests that use only MIT crates (`history.rs`, `install_game.rs`, `network.rs`) | A | dainty-lass `crates/hostkit/tests` | Move as they are |
| Category A tests that compose AGPL services | A | restricted | Until phase 2 |
| Category B tests and fixtures | B / B-data | restricted, run against MIT hostkit | §4.2 |

### 5.2 What dainty-restricted keeps

After phase 2: the AGPL libraries (gateway with allowlist, content, chat), the 1.14.7 module library with its
generators and `db` commands, the B tests and fixtures as a parity suite, and nothing named `crates/host`. The
server binaries become dainty-lass's `dainty-lass-server` plus pinned libraries.

### 5.3 Seams this needs (reported, not made here)

- `crates/hostkit`: a public "service env file" helper and `remote-agent` in `service_main` (hostkit is
  dainty-lass's; a row owns it).
- `crates/remote`: the enrolment handler as a library function.
- `crates/module-api` or `plugin-host`: a way for a module library to offer the game-data generators and the `db`
  tools to hostkit (a module command per `GeneratorKind`, or a `HostHooks`-like entry exported by the library).

---

## 6. Effort estimate

| Group | Work | Size |
| --- | --- | --- |
| Generator dispatch (§2.1) | Clean-room rewrite, ~70 lines plus tests; or none if it stays AGPL in the module | Small |
| Move the generic A parts into hostkit and `remote` (§5.1 rows 1–3) | Mechanical move plus seams | Small |
| Generic external-module service main in hostkit | New code over `plugin-host` | Medium |
| Delete `compose.rs`/`compose_in_process_*` | Follows the content, chat and gateway libraries (RD27 phase 2) | Medium (owned by phase 2) |
| Module-provided `db` and generator hooks | New seam in `module-api`/`plugin-host`, then move | Medium |
| Replace the B tests with spec-derived MIT tests (§4.2 option 1) | ~6,000 lines of tests | Large (optional) |

Suggested order: the generic A moves first (no clean room, unblocks deleting the shell's service mains), then the
module hooks with phase 2, and the test replacement last, if at all.

---

## 7. Acceptance for the implementer

- The rewritten generator hook passes §2.1's tests, and a reviewer outside the clean room runs the restricted
  `tests/gamedata.rs` suite against it unchanged.
- Similarity of the rewritten function against `crates/host/src/lib.rs` at `849f45e`, by `similarity.md`'s measure: below 30%, with no run longer than 6 lines outside import lists and match
  arms that name the same `GeneratorKind` variants.
- No AGPL crate appears in the dependency tree of any dainty-lass crate (`cargo tree` per crate, as
  `clean-room/launcher-kit.md` §4 records it).
