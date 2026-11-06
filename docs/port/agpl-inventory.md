<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# AGPL provenance inventory and MIT rewrite scope (L0 — DD26, DL2, DL3a, DL4a)

> **Historical** (L0): the inventory that scoped the MIT rewrite. Everything it lists for rewriting was rewritten
> clean-room (RD27): the MIT crates are at the root, the AGPL ones are under `restricted/`, and `dainty-lass-ui` and
> the `rust/` folder are gone. The provenance records that replace it are in `clean-room/`.

This document inventories every crate in `dainty-lass/rust` (including `fuzz`), `native/runtime-loader`, and every
package in the `ui/` workspace (`silentspud/dainty-lass-ui`) at `plan` (`b190010` / `5dd3e28`). It records each file's
provenance and evidence, maps the crate dependency graph (including every MIT-to-AGPL dependency edge), determines the
effective licence of every shipped binary and Docker image today and after the rewrite, analyses the candidate crates
beyond `DL3` and `DL4`, and lists the four decisions that require the maintainer's answer before `L1` begins.

---

## 1. Crate-by-crate and package-by-package provenance inventory

### 1.1 Provenance categories and primary sources

Every source file in `dainty-lass` falls into one or more of four provenance lines:

| Category | Upstream source | Upstream licence | Evidence sources |
| --- | --- | --- | --- |
| **Born here** | Written first in `dainty-lass` / `dainty-lass-ui` | `MIT` | Top-of-file row/decision comments (`DF*`, `DD*`, `DN*`, `DH*`, `DE*`, `DU*`, `RD*`), `NOTICE.md`, `ui/NOTICE.md`, `docs/port/migration-guide.md` §2 |
| **Reforged-derived** | Ported from Freeza's `Dauntless-Reforged-Revival` (`ReforgedHost`, `ReforgedLauncher`, `ReforgedGateway`, `ReforgedContent`, `patches/`) | `AGPL-3.0-only` (subject to `DL2` maintainer question) | `docs/port/spec-dl3-hostkit.md`, `docs/port/spec-dl4-launcher.md`, `docs/port/delta-*.md`, `docs/port/migration-guide.md` §2 |
| **DR-derived** | Copied or adapted from `Dauntless Revived` (`e1c5fec` for 1.14.7, `24b7624` for 1.4.4) in commits `5281638`, `c9f9d6a`, `a6e1870`, `e34cf60` | `AGPL-3.0-only` | `NOTICE.md`, `docs/port/migration-guide.md` §2, `docs/port/delta-*.md`, top-of-file `DR` provenance comments |
| **Paradox-derived** | Derived from `Mystic Paradox` (`modules/rustic-isles`, `modules/rustic-rifts`, `native/runtime-loader`) | `AGPL-3.0-only` + `ADDITIONAL_TERMS.md` §§1–2 (`native/runtime-loader` declares `Apache-2.0` in `Cargo.toml` and ships an `LGPL-3.0` `LICENSE` file; see §1.4) | `NOTICE.md`, `modules/rustic-isles/NOTICE.md`, `modules/rustic-rifts/NOTICE.md`, `native/runtime-loader/Cargo.toml` |

### 1.2 Summary of all 20 Rust crates, `fuzz`, `native/runtime-loader`, and `ui/`

| Path | Package name | `Cargo.toml` / `package.json` `license` | Files (`.rs` / `.ts` / `.tsx` / `.json`) | Lines | File provenance summary | Direct AGPL workspace / git dependencies |
| --- | --- | --- | --- | --- | --- | --- |
| `crates/module-api` | `module-api` | `MIT` | 18 `.rs` | 4,895 | 100% **born here** (`DF2`, `DD3`–`DD5`, `DN2`, `DN10`, `DH1`, `DD25`) | None |
| `crates/epic` | `epic` | `MIT` | 18 `.rs` | 4,636 | 100% **born here** (`DE1`–`DE5`, `DD22`, from Legendary's documented BuildPatchServices format) | None |
| `crates/history-ship` | `history-ship` | `MIT` | 9 `.rs` | 2,583 | 100% **born here** (`DH4`, `DD23`) | `store` (`AGPL-3.0-only`) |
| `crates/network` | `network` | `MIT` | 25 `.rs` | 11,743 | 100% **born here** (`DN3`–`DN9`, `DN11`, `DN13`, `DH7`, `DD16`–`DD20`) | Default feature `directory`: `protocol`, `store`; default feature `log-ship`: `history-ship` -> `store` |
| `crates/admin` | `admin` | `MIT` | 29 `.rs` | 12,042 | 100% **born here** (`RD25`, `DF4c`, `DH2`, `DH3b`, `DH6`, `DN11`) | Normal: `protocol`, `service-kit`, `store`; dev: `gateway` |
| `crates/controller` | `server-controller` | `MIT` | 10 `.rs` | 3,629 | 100% **born here** (`RD26`, `DF3`, `DN3`–`DN9`) | Unconditional: `protocol`, `service-kit`; default features: `gateway`, `content`, `store`; optional features: `isles-1-14-7`, `isles-1-12-0` |
| `crates/server-console` | `server-console` | `MIT` | 3 `.rs` | 12 (+ 58 in `bins/modules_test.rs`) | 100% **born here** (`RD26`, `DF5`) | `host` (`AGPL-3.0-only`, with default `isles-1-14-7`, `isles-1-12-0`, `realms`) |
| `crates/server-minimal` | `server-minimal` | `MIT` | 2 `.rs` | 7 (+ 58 in `bins/modules_test.rs`) | 100% **born here** (`RD26`, `DF5`) | `host` (`AGPL-3.0-only`, with default `isles-1-14-7`, `isles-1-12-0`, `realms`) |
| `crates/backend` | `backend` | `MIT` | 2 `.rs` | 15 | 100% **born here** (`RR1` / `DL1` stub) | None |
| `crates/director` | `director` | `MIT` | 2 `.rs` | 15 | 100% **born here** (`RR1` / `DL1` stub) | None |
| `crates/protocol` | `protocol` | `AGPL-3.0-only` (`workspace`) | 10 `.rs` | 2,284 | **DR-derived + Reforged-derived** (`5281638` from DR `e1c5fec` row X4 + `RS1` port of `ReforgedContent` / `ReforgedLauncher` helpers) | None (`module-api` is `MIT`) |
| `crates/proxy` | `proxy` | `AGPL-3.0-only` (`workspace`) | 1 `.rs` | 193 | **DR-derived** verbatim (`5281638` from DR `e1c5fec` row X5) | None |
| `crates/service-kit` | `service-kit` | `AGPL-3.0-only` (`workspace`) | 17 `.rs` | 3,211 | **DR-derived** (15 files from DR `e1c5fec` row B1 in `5281638`) + **born here** (2 files: `context.rs`, `telemetry.rs` from `RD26`) | None |
| `crates/store` | `store` | `AGPL-3.0-only` (`workspace`) | 20 `.rs` | 6,318 | **DR-derived** (7 files from DR `e1c5fec` rows D10/D11/D21 in `5281638`) + **born here** (13 files from `DF7`, `DH1`, `DH3a`, `DH5`, `DD15`, `DD23`) | None (`module-api` is `MIT`) |
| `crates/chat` | `chat` | `AGPL-3.0-only` (`workspace`) | 15 `.rs` | 7,438 | **DR-derived** (8 files from DR `e1c5fec` `services/metagame/src/chat/` in `DN10a`) + **born here** (7 files in `DN10a`/`DN10b`) | Normal: `protocol`, `service-kit`, `store`; dev: `gateway` |
| `crates/gateway` | `gateway` | `AGPL-3.0-only` (`workspace`) | 40 `.rs` | 15,874 | **DR-derived + Reforged-derived** (31 files from DR `e1c5fec` + `ReforgedGateway` in `c9f9d6a`, `RS3`/`RS4`) + **born here** (9 files in `DF4a`, `DF4d`, `DN3`, `DN7`) | `protocol`, `proxy`, `service-kit` |
| `crates/content` | `content` | `AGPL-3.0-only` (`workspace`) | 24 `.rs` | 6,118 | **DR-derived + Reforged-derived** (23 files from DR `e1c5fec`/`24b7624` + `ReforgedContent` in `c9f9d6a`, `RS2`) + **born here** (1 file: `modules.rs` in `DF4b`) | `protocol`, `service-kit` |
| `crates/host` | `host` | `AGPL-3.0-only` (`workspace`) | 68 `.rs` | 30,445 | **Born here** (21 files) + **Reforged-derived** (44 files from `ReforgedHost`) + **DR-derived / AGPL shell** (3 files: `lib.rs`, `legacy/mod.rs`, `legacy/off.rs`); see `docs/port/spec-dl3-hostkit.md` | `protocol`, `service-kit`, `store`, `gateway`, `content`, `isles-1-14-7`, `isles-1-12-0`, `isles-core`, `isles-db`, `isles-director` |
| `launcher/src-tauri` | `launcher` | `AGPL-3.0-only` (`workspace`) | 109 `.rs` | 38,411 | **Born here** (31 files) + **Reforged-derived** (18 files from `ReforgedLauncher`) + **DR-derived + Reforged-modified** (60 files from DR `e1c5fec`/`24b7624` in `a6e1870`); see `docs/port/spec-dl4-launcher.md` | `protocol`, `proxy`; under `realms`: `rifts-host`, `rifts-seam`; under `server-gui`: `service-kit`, `host` (with `isles-*`, `revived-1-4-4`, `realms`) |
| `fuzz` | `reforged-fuzz` | *(none)* | 39 `.rs` | 2,238 | **DR-derived** (6 targets from DR X9) + **born here / Reforged-derived** (33 targets in `RX3`/`DX2`, several `include!`-ing `launcher/src-tauri/src/*.rs`) | `chat`, `content`, `gateway`, `host`, `protocol`, `proxy`, `service-kit`, `store` |
| `native/runtime-loader` | `MysticLoader` | `Apache-2.0` (`Cargo.toml`) / `LGPL-3.0` (`LICENSE`) | 3 `.rs` | 433 | **Paradox-derived + Reforged-derived** (`Mystic Paradox` `tools/RuntimeLoader/` from `coldloader-proxy`, plus Freeza's patches `0004` and `0013`) | None (`winapi` only) |
| `ui/apps/web` | `web` | `MIT` | 38 `.ts`/`.tsx` | 7,612 | 100% **born here** (`DD11`, `DU1`–`DU5`, `DE7`, `DH3b`, `DH6`) | `@workspace/bridge`, `@workspace/i18n` |
| `ui/packages/ui` | `@workspace/ui` | `MIT` | 15 `.tsx`/`.ts` | 1,420 | 100% **born here** (`DD11`, `DU1`, shadcn/Radix primitives) | None |
| `ui/packages/bridge` | `@workspace/bridge` | `MIT AND AGPL-3.0-only` | 4 `.ts` | 1,896 | `src/{api,call,index}.ts` **born here** (`MIT`); `src/generated/types.ts` generated from `launcher/src-tauri/src/bridge/{types,spec}.rs` (`AGPL-3.0-only` per `ui/NOTICE.md`) | `@workspace/i18n` (`type` import of `CatalogKey`) |
| `ui/packages/i18n` | `@workspace/i18n` | `MIT AND AGPL-3.0-only` | 2 `.ts` + 2 `.json` | 2,418 | `src/index.ts` and `test/catalogs.test.ts` **born here** (`MIT`); `catalogs/en.json` and `catalogs/pt-BR.json` **DR-derived + Reforged-derived + born-here keys appended after `err_edition_other`** (`AGPL-3.0-only` per `ui/NOTICE.md`) | None |

> **Factual licence metadata discrepancies noted during audit:**
>
> 1. **`crates/chat`**: `AGENTS.md` line 268 parenthetically lists `crates/chat` among MIT crates
>    (`controller, module-api, epic, history-ship, crates/network, crates/chat, admin, bins`), whereas `NOTICE.md`
>    line 114, `docs/dainty-lass-plan.md` `DD26` line 125 (*"AGPL for good: ... `crates/chat` (DR XMPP core)"*),
>    and `crates/chat/Cargo.toml` (`license.workspace = true` -> `AGPL-3.0-only`) all classify `crates/chat` as
>    `AGPL-3.0-only`.
> 2. **`native/runtime-loader`**: `native/runtime-loader/Cargo.toml` declares `license = "Apache-2.0"`, while
>    `native/runtime-loader/LICENSE` contains the text of the GNU Lesser General Public License v3 (`LGPL-3.0`)
>    inherited from `coldloader-proxy` / `Mystic Paradox`.

---

### 1.3 Detailed file-by-file provenance for every Rust crate

#### 1.3.1 `crates/module-api` (`license = "MIT"`)

- **Provenance**: 100% **born here** (`DF2`, `DD3`–`DD5`, `DN2`, `DN10`, `DH1`, `DD25`, Section 6.2).
- **Files (18)**:
  - `src/lib.rs`, `src/actor.rs`, `src/chat.rs`, `src/child.rs`, `src/driver.rs`, `src/frame.rs`, `src/host.rs`,
    `src/ids.rs`, `src/manifest.rs`, `src/module.rs`, `src/network.rs`, `src/peer.rs`, `src/registrar.rs`,
    `src/stub.rs`, `src/tree.rs`, `src/version.rs`, `src/bin/module-api-stub.rs`, `tests/in_and_out_of_process.rs`.
- **Evidence**: `NOTICE.md` lines 31, 45; `docs/dainty-lass-plan.md` rows `DF2`, `DD3`–`DD5`, `DD25`.
- **Dependencies**: External crates only (`bytes`, `serde`, `serde_json`, `thiserror`, `tokio`).
- **Dependents**: `controller`, `protocol`, `store`, `crates/network`, `crates/chat`, `crates/admin`,
  `crates/content`, `crates/gateway`, `crates/host`, `launcher`, `fuzz`, and submodule workspaces
  `rustic-isles`, `rustic-rifts`, `legacy-revived`.

#### 1.3.2 `crates/epic` (`license = "MIT"`)

- **Provenance**: 100% **born here** (`DE1`–`DE5`, `DD22`). Clean-room implementation of Epic Games Store OAuth,
  library enumeration, binary/JSON BuildPatchServices manifests, and resumable chunk download from Legendary's
  documented format (`docs/port/spike-de1-epic.md`).
- **Files (18)**:
  - `src/lib.rs`, `src/auth.rs`, `src/builds.rs`, `src/chunk.rs`, `src/library.rs`, `src/manifest.rs`,
    `src/manifest/{path,reader,synthetic,tests}.rs`, `src/download.rs`,
    `src/download/{cdn,md5,resume,sources,tests}.rs`, `src/testing.rs`, `tests/archive.rs`.
- **Evidence**: `NOTICE.md` lines 32, 46; `docs/dainty-lass-plan.md` rows `DE1`–`DE5`, `DD22`.
- **Dependencies**: External crates only (`flate2`, `http-body-util`, `hyper`, `hyper-rustls`, `md-5`, `reqwest`,
  `rustls`, `serde`, `serde_json`, `sha1`, `sha2`, `thiserror`, `tokio`, `url`, `zeroize`).
- **Dependents**: `crates/host`, `launcher`, `fuzz`.

#### 1.3.3 `crates/history-ship` (`license = "MIT"`)

- **Provenance**: 100% **born here** (`DH4`, `DD23`).
- **Files (9)**:
  - `src/lib.rs`, `src/bucket.rs`, `src/lag.rs`, `src/layout.rs`, `src/restore.rs`, `src/s3.rs`, `src/ship.rs`,
    `tests/minio.rs`, `tests/ship.rs`.
- **Evidence**: `NOTICE.md` lines 33, 47; `docs/dainty-lass-plan.md` rows `DH4`, `DD23`.
- **Dependencies**: **`store`** (`AGPL-3.0-only`, for `store::Database`, `store::history::record::{ChangeRow, EntryRow,
  SnapshotManifest}`, and `store::recovery::restore`) + external crates (`aws-credential-types`, `aws-sdk-s3`,
  `aws-types`, `bytes`, `flate2`, `hex`, `serde`, `serde_json`, `sha2`, `thiserror`, `tokio`, `tracing`).
- **Dependents**: `crates/network` (default feature `log-ship`), `controller` (default feature `network`), `fuzz`.

#### 1.3.4 `crates/network` (`license = "MIT"`)

- **Provenance**: 100% **born here** (`DN3`–`DN9`, `DN11`, `DN13`, `DH7`, `DD16`–`DD20`).
- **Files (25)**:
  - `src/lib.rs`, `src/cluster.rs`, `src/directory.rs`, `src/endpoint.rs`,
    `src/home/{mod,client,keys,protocol,service,ticket}.rs`, `src/inprocess.rs`, `src/link.rs`, `src/logship.rs`,
    `src/messages.rs`, `src/relay.rs`, `src/ring.rs`, `src/tls.rs`, `src/username.rs`, `src/wire.rs`,
    `tests/{cluster,directory,home,logship,ring,common/mod}.rs`.
- **Evidence**: `NOTICE.md` lines 34, 48; `docs/dainty-lass-plan.md` rows `DN3`–`DN9`, `DN11`, `DN13`, `DH7`.
- **Dependencies**:
  - Unconditional: `module-api` (`MIT`) + external crates.
  - Default feature `directory`: **`protocol`** (`AGPL-3.0-only`, used only in `src/username.rs` for
    `protocol::account::check_username`) and **`store`** (`AGPL-3.0-only`, used in `directory.rs`, `relay.rs`,
    `home/ticket.rs`).
  - Default feature `log-ship`: `history-ship` (`MIT`, which depends on **`store`**) and **`store`** (`AGPL-3.0-only`).
- **Dependents**: `controller`, `crates/gateway` (`default-features = false`), `crates/admin`, `crates/host`,
  `crates/chat`, `fuzz`.

#### 1.3.5 `crates/admin` (`license = "MIT"`)

- **Provenance**: 100% **born here** (`RD25`, `DF4c`, `DH2`, `DH3b`, `DH6`, `DN11`). Note: `src/upstream.rs` is a new
  Rust HTTP/HMAC client written against the `/admin/v1/*` and `/reforged/api/*` wire contract (`RD25`).
- **Files (29)**:
  - `src/lib.rs`, `src/app.rs`, `src/cli.rs`, `src/config.rs`, `src/guard.rs`, `src/guarded.rs`, `src/guarded_cli.rs`,
    `src/history.rs`, `src/history_text.rs`, `src/hmac.rs`, `src/input.rs`, `src/key_cli.rs`, `src/keys.rs`,
    `src/log.rs`, `src/module_control.rs`, `src/modules.rs`, `src/relay.rs`, `src/schema.rs`, `src/server.rs`,
    `src/timeouts.rs`, `src/upstream.rs`, `tests/{api,api/guarded,guard,guarded_cli,history,key_cli,keys,network}.rs`.
- **Evidence**: `NOTICE.md` lines 35, 49; `docs/dainty-lass-plan.md` rows `RD25`, `DF4c`, `DH2`, `DH3b`, `DH6`, `DN11`.
- **Dependencies**: `module-api` (`MIT`), `network` (`MIT`), **`protocol`** (`AGPL-3.0-only`), **`service-kit`**
  (`AGPL-3.0-only`), **`store`** (`AGPL-3.0-only`), plus dev-dependency **`gateway`** (`AGPL-3.0-only`).
- **Dependents**: `controller` (default feature `admin`), `crates/host` (default feature `admin` + dev-dependency),
  `launcher` (feature `server-gui`), `fuzz`.

#### 1.3.6 `crates/controller` (`server-controller`, `license = "MIT"`)

- **Provenance**: 100% **born here** (`RD26`, `DF3`, `DN3`–`DN9`).
- **Files (10)**:
  - `src/lib.rs`, `src/controller.rs`, `src/modules.rs`, `src/node_services.rs`, `src/ports.rs`, `src/registry.rs`,
    `src/bin/controller-test-module.rs`, `tests/{allowlist,network_wiring,registry}.rs`.
- **Evidence**: `NOTICE.md` lines 30, 44; `docs/port/migration-guide.md` §2 lines 107–108 (`RD26` `feat/app-structure`
  commit `e34cf60`).
- **Dependencies**:
  - Unconditional: `module-api` (`MIT`), `network` (`MIT`, `default-features = false`), **`protocol`** (`AGPL-3.0-only`),
    **`service-kit`** (`AGPL-3.0-only`, for `service_kit::{ServiceContext, Telemetry}`).
  - Default features (`gateway`, `allowlist`, `content`, `admin`, `network`): **`gateway`** (`AGPL-3.0-only`),
    **`content`** (`AGPL-3.0-only`), `admin` (`MIT`), `history-ship` (`MIT`), **`store`** (`AGPL-3.0-only`).
  - Optional game-module features: **`isles-1-14-7`** and **`isles-1-12-0`** (`AGPL-3.0-only` + Mystic Paradox
    additional terms). (`revived-1-4-4` is currently an empty feature `[]` in `controller/Cargo.toml`.)
- **Dependents**: `crates/host`, `launcher` (feature `server-gui`), `crates/server-console` (dev),
  `crates/server-minimal` (dev).

#### 1.3.7 `crates/server-console` and `crates/server-minimal` (`license = "MIT"`)

- **Provenance**: 100% **born here** (`RD26`, `DF5`).
- **Files (5 across both crates + 1 shared test file)**:
  - `crates/server-console/src/main.rs` (`dainty-lass-server-console`),
    `crates/server-console/src/host_main.rs` (`dainty-lass-host` / `reforged-host`),
    `crates/server-console/tests/modules.rs`;
  - `crates/server-minimal/src/main.rs` (`dainty-lass-server-minimal`),
    `crates/server-minimal/tests/modules.rs`;
  - `bins/modules_test.rs`.
- **Evidence**: `NOTICE.md` lines 36, 50; `docs/port/migration-guide.md` §2 lines 113–114.
- **Dependencies**: **`host`** (`AGPL-3.0-only`, with `features = ["modules"]` and default features
  `["isles-1-14-7", "isles-1-12-0", "realms"]`).

#### 1.3.8 `crates/backend` and `crates/director` (`license = "MIT"`)

- **Provenance**: 100% **born here** (`RR1` workspace stubs, marked `MIT` in `DL1`).
- **Files (4)**: `crates/backend/src/lib.rs`, `crates/backend/tests/stub.rs`,
  `crates/director/src/lib.rs`, `crates/director/tests/stub.rs`.
- **Dependencies**: None. Only referenced by `docker/Dockerfile` test stage (`-p backend -p director`).

#### 1.3.9 `crates/protocol` (`license = "AGPL-3.0-only"`)

- **Provenance**: **DR-derived + Reforged-derived** (`5281638` from DR `e1c5fec` row X4, fitted to Reforged in `RS1`).
- **Files (10)**:
  - **DR-derived + Reforged-modified (4 src + 3 test files)**: `src/lib.rs`, `src/account.rs`, `src/invite.rs`,
    `src/status.rs`, `tests/invite.rs`, `tests/tables.rs`, `tests/username_status.rs` (plus `tests/cases/*.json` test
    vectors).
  - **Reforged-derived (2 src + 1 test file)**: `src/manifest.rs` (ported from `ReforgedContent/src/runtime.ts` and
    `ReforgedLauncher/src/main/hostapi.ts`), `src/text.rs` (ported from `ReforgedLauncher/src/shared/text.ts`),
    `tests/manifest.rs`.
- **Evidence**: `NOTICE.md` line 106; `docs/port/migration-guide.md` §2 lines 40, 128–133;
  `docs/port/delta-protocol.md`.
- **Dependencies**: `module-api` (`MIT`), `percent-encoding`, `serde`, `serde_json`, `thiserror`, `url`.
- **Dependents**: `controller`, `crates/network` (feature `directory`), `crates/chat`, `crates/admin`,
  `crates/content`, `crates/gateway`, `crates/host`, `launcher`, `fuzz`.

#### 1.3.10 `crates/proxy` (`license = "AGPL-3.0-only"`)

- **Provenance**: **DR-derived** verbatim (`5281638` from DR `e1c5fec` row X5).
- **Files (1)**: `src/lib.rs` (193 lines: RFC 9110 §7.6.1 hop-by-hop header stripping and `tokio::io::copy_bidirectional`
  wrapper).
- **Evidence**: `NOTICE.md` line 107; `docs/port/migration-guide.md` §2 lines 40, 134.
- **Dependencies**: `http`, `tokio`.
- **Dependents**: `crates/gateway`, `launcher`, `fuzz`.

#### 1.3.11 `crates/service-kit` (`license = "AGPL-3.0-only"`)

- **Provenance**: Mixed **DR-derived** (15 files from DR `e1c5fec` row B1 in `5281638`) and **born here** (2 files from
  `RD26` `feat/app-structure` in `e34cf60`).
- **Files (17)**:
  - **Born here (2 files, 485 lines)**: `src/context.rs` (`ServiceContext`, `ShutdownSignal`, `ServiceStatus`,
    `ServiceState`, `StatusWatch`), `src/telemetry.rs` (`Telemetry`, `LogEntry`).
  - **DR-derived (12 src + 3 test files, 2,726 lines)**: `src/lib.rs`, `src/body.rs`, `src/collate.rs`, `src/env.rs`,
    `src/js.rs`, `src/jsjson.rs`, `src/json.rs`, `src/linelog.rs`, `src/log.rs`, `src/ratelimit.rs`, `src/routing.rs`,
    `src/serve.rs`, `tests/body.rs`, `tests/ratelimit.rs`, `tests/routing.rs`.
- **Evidence**: `NOTICE.md` line 108; `docs/port/migration-guide.md` §2 lines 40, 109–110, 135.
- **Dependencies**: External crates only (`bytes`, `http`, `http-body-util`, `hyper`, `hyper-util`, `percent-encoding`,
  `ryu`, `serde`, `serde_json`, `socket2`, `thiserror`, `time`, `tokio`).
- **Dependents**: `controller`, `crates/admin`, `crates/content`, `crates/gateway`, `crates/host`,
  `crates/chat`, `launcher` (feature `server-gui`), `fuzz`, and external submodules `rustic-isles` and
  `legacy-revived`.

#### 1.3.12 `crates/store` (`license = "AGPL-3.0-only"`)

- **Provenance**: Mixed **DR-derived** (7 files from DR `e1c5fec` rows D10/D11/D21 in `5281638`) and **born here** (13
  files from `DF7`, `DH1`, `DH3a`, `DH5`, `DD15`, `DD23`).
- **Files (20)**:
  - **Born here (11 src + 2 test files, 5,361 lines)**: `src/connection.rs`, `src/database.rs`, `src/declared.rs`,
    `src/engine.rs`, `src/history.rs`, `src/history/chain.rs`, `src/history/record.rs`, `src/portable.rs`,
    `src/recovery.rs` *(note: calls `crate::backup::backup_sqlite` and `crate::snapshot::verify_sqlite`)*,
    `src/revert.rs`, `src/transfer.rs`, `tests/database.rs`, `tests/revert.rs`.
  - **DR-derived (6 src + 1 test file, 957 lines)**: `src/lib.rs` (DR `Store` wrapper, pragmas, error types),
    `src/adopt.rs`, `src/backup.rs`, `src/drizzle.rs`, `src/readonly.rs`, `src/snapshot.rs`, `tests/store.rs`.
- **Evidence**: `NOTICE.md` line 109; `docs/port/migration-guide.md` §2 lines 40, 136; `docs/dainty-lass-plan.md` rows
  `DF7`, `DH1`, `DH3a`, `DH5`.
- **Dependencies**: `module-api` (`MIT`) + external crates (`diesel`, `flate2`, `hex`, `libsqlite3-sys`, `pq-sys`,
  `rusqlite`, `serde`, `serde_json`, `sha2`, `thiserror`, `tokio`, `url`).
- **Dependents**: `controller` (feature `network`), `history-ship`, `crates/network` (feature `directory`),
  `crates/chat`, `crates/admin`, `crates/host`, `fuzz`, and external submodules `rustic-isles`, `rustic-rifts`,
  `legacy-revived`.

#### 1.3.13 `crates/chat` (`license = "AGPL-3.0-only"`)

- **Provenance**: Mixed **DR-derived** (`DR` `e1c5fec` `services/metagame/src/chat/`, rows `DN10a`/`DN10b`) and **born
  here** (`DN10a`/`DN10b`).
- **Files (15)**:
  - **DR-derived (8 files)**: `src/lib.rs`, `src/connection.rs`, `src/muc.rs`, `src/nick.rs`, `src/redact.rs`,
    `src/server.rs`, `src/settings.rs`, `src/xmpp.rs`.
  - **Born here (4 src + 3 test files)**: `src/federation.rs`, `src/history.rs`, `src/hooks.rs`, `src/naming.rs`
    (room-naming rules from Mystic Paradox `RoomService.ts`/`cityRooms.ts` behaviour, no code copied),
    `tests/chat.rs`, `tests/federation.rs`, `tests/history.rs`.
- **Evidence**: `NOTICE.md` line 114; `docs/dainty-lass-plan.md` `DD26` line 125 and rows `DN10a`, `DN10b`.
- **Dependencies**: `module-api` (`MIT`), `network` (`MIT`), **`protocol`** (`AGPL-3.0-only`), **`service-kit`**
  (`AGPL-3.0-only`), **`store`** (`AGPL-3.0-only`), plus dev-dependency **`gateway`** (`AGPL-3.0-only`).
- **Dependents**: `fuzz` (not yet wired into `controller::modules`).

#### 1.3.14 `crates/gateway` (`license = "AGPL-3.0-only"`)

- **Provenance**: **DR-derived + Reforged-derived** (`c9f9d6a` from DR `e1c5fec` row B2/B9 + `ReforgedGateway` in
  `RS3`/`RS4`) and **born here** (`DF4a`, `DF4d`, `DN3`, `DN7`).
- **Files (40)**:
  - **Born here (5 src + 4 test files)**: `src/modules.rs` (`DF4a`), `src/network.rs` (`DN7`), `src/nodelink.rs` (`DN3`),
    `src/allowlist/guard.rs` & `src/allowlist/modules.rs` (`DF4d`), `tests/{home,modules,network,nodelink}.rs`.
  - **DR-derived + Reforged-derived (21 src + 10 test files)**: `src/lib.rs`, `src/capture.rs`, `src/cert.rs`,
    `src/cli.rs`, `src/config.rs`, `src/feed.rs`, `src/gateway.rs`, `src/guard.rs`, `src/ip.rs`, `src/policy.rs`,
    `src/policy/legacy.rs`, `src/ratelimit.rs`, `src/redact.rs`, `src/server.rs`,
    `src/allowlist/{mod,cli,config,firewall,helper,nft,ports,state,sync,windows}.rs`,
    `tests/{allowlist_firewall,allowlist_helper,allowlist_parity,allowlist_ports,allowlist_sync,cert_and_tls,feed_and_capture,legacy,parity,proxy_and_limits}.rs`.
- **Evidence**: `NOTICE.md` line 110; `docs/port/migration-guide.md` §2 lines 41, 47–65; `docs/port/delta-gateway.md`;
  `docs/port/delta-allowlist.md`.
- **Dependencies**: `module-api` (`MIT`), `network` (`MIT`, `default-features = false`), **`protocol`**
  (`AGPL-3.0-only`), **`proxy`** (`AGPL-3.0-only`), **`service-kit`** (`AGPL-3.0-only`).
- **Dependents**: `controller` (default features `gateway`, `allowlist`), `crates/host` (default features `gateway`,
  `allowlist`), `fuzz`, and dev-dependencies of `crates/admin` and `crates/chat`.

#### 1.3.15 `crates/content` (`license = "AGPL-3.0-only"`)

- **Provenance**: **DR-derived + Reforged-derived** (`c9f9d6a` from DR `e1c5fec`/`24b7624` row B4 + `ReforgedContent`
  in `RS2`) and **born here** (`DF4b`).
- **Files (24)**:
  - **Born here (1 file)**: `src/modules.rs` (`DF4b`).
  - **Reforged-derived (1 file)**: `src/runtime.rs` (from `ReforgedContent/src/runtime.ts`: runtime manifest,
    `MysticParadox.dll` / `winmm.dll` file serving, `Range: bytes=0-3` probe).
  - **DR-derived + Reforged-modified (19 src + 3 test files)**: `src/lib.rs`, `src/app.rs`, `src/app/game_files.rs`,
    `src/auth.rs`, `src/branding.rs`, `src/cli.rs`, `src/client.rs`, `src/config.rs`, `src/input.rs`, `src/limits.rs`,
    `src/log.rs`, `src/news.rs`, `src/server.rs`, `src/legacy/{mod,gamedir,manifest,paths,range,verify}.rs`,
    `tests/{app,legacy,parity}.rs`.
- **Evidence**: `NOTICE.md` line 111; `docs/port/migration-guide.md` §2 lines 41, 66–81; `docs/port/delta-content.md`.
- **Dependencies**: `module-api` (`MIT`), **`protocol`** (`AGPL-3.0-only`), **`service-kit`** (`AGPL-3.0-only`).
- **Dependents**: `controller` (default feature `content`), `crates/host` (default feature `content`), `fuzz`.

#### 1.3.16 `crates/host` (`license = "AGPL-3.0-only"`)

- **Provenance**: Detailed in `docs/port/spec-dl3-hostkit.md` (`DL3a`), plus `src/commands/db.rs` added in `M3` (`b4e924b`):
  - **Category A — Born here (18 src + 3 test files)**:
    `src/error.rs`, `src/hosttoml.rs`, `src/hostconfig/{yaml,guard,modules}.rs`, `src/epicinstall.rs`, `src/guarded.rs`,
    `src/repl.rs`, `src/tui.rs`, `src/serve.rs`, `src/commands/{backup,db,history,install_game,network}.rs`,
    `src/commands/setup/{database,secrets}.rs`, `src/commands/start/binaries.rs`,
    `tests/{history,install_game,network}.rs`.
  - **Category B — Reforged-derived (`DL3` clean-room rewrite scope: 22 module groups / 38 src + 9 test files, ported
    from Freeza's `ReforgedHost`)**:
    `src/paths.rs`, `src/envfile.rs`, `src/hostconfig.rs`, `src/run.rs`, `src/services.rs`, `src/services/tree.rs`,
    `src/invite.rs`, `src/doctor.rs`, `src/mongo.rs`, `src/mongo/wire.rs`, `src/gamedata.rs`, `src/gamepack.rs`,
    `src/captures.rs`, `src/preset.rs` *(also notes DR `vendor/*.json` layout)*, `src/progression.rs`,
    `src/publish.rs`, `src/linux.rs`, `src/selfupdate.rs`, `src/wine.rs`, `src/cli.rs`,
    `src/commands/{mod,doctor,gamedata,invite,multipliers,preset,publish_runtime,setup,update}.rs`,
    `src/commands/setup/{firewall,install,options,paradox}.rs`, `src/commands/start/{mod,control}.rs`,
    `tests/{config,doctor,gamedata,invite,mongo,node_fixtures,setup,start,update}.rs`.
  - **Category C — DR-derived / AGPL shell (3 files)**:
    `src/legacy/mod.rs` and `src/legacy/off.rs` (DR 1.4.4 `metagame`/`deploy` setup and supervision), and `src/lib.rs`
    (entry point wiring `gateway`, `content`, `admin`, `isles-1-14-7`, `isles-1-12-0`).
- **Evidence**: `NOTICE.md` line 112; `docs/port/spec-dl3-hostkit.md`; `docs/port/migration-guide.md` §2 lines 142–188;
  `docs/port/delta-host-*.md`.
- **Dependencies**:
  - Workspace `MIT`: `server-controller`, `module-api`, `network`, `epic`, `admin` (default feature).
  - Workspace `AGPL-3.0-only`: **`protocol`**, **`service-kit`**, **`store`**, **`gateway`** (default feature),
    **`content`** (default feature).
  - Submodule `AGPL-3.0-only` + Mystic Paradox additional terms (under default features `isles-1-14-7`, `isles-1-12-0`,
    or `paradox`): **`isles-1-14-7`**, **`isles-1-12-0`**, **`isles-core`**, **`isles-db`**, **`isles-director`**.
    Specifically:
    - `src/gamedata.rs` calls `isles_core::areas::game_data::generators` and `isles_director::hunt_tables` in-process
      and embeds `modules/rustic-isles/reference/1.14.7/.../*.example.json` and `store-captures.json` via
      `include_str!`;
    - `src/commands/db.rs`, `src/commands/setup.rs`, `src/commands/update.rs`, and `src/commands/start/mod.rs` call
      `isles_db::{migrate, verify, export_to_mongo, MongoClient}` in-process;
    - `src/cli.rs` and `src/lib.rs` dispatch `backend` and `director` subcommands to `isles_1_14_7::server` and
      `isles_director::server`.
- **Dependents**: `crates/server-console`, `crates/server-minimal`, `launcher` (feature `server-gui`), `fuzz`.

#### 1.3.17 `launcher/src-tauri` (`license = "AGPL-3.0-only"`)

- **Provenance**: Detailed in `docs/port/spec-dl4-launcher.md` (`DL4a`):
  - **Category A — Born here (31 files)**:
    `src/server_gui.rs`, `src/server_gui/modules.rs`, `src/server_gui/modules/tests.rs`, `src/server_history.rs`,
    `src/bridge/modules.rs`, `src/controller/{guarded,install,loader,realms,realms_off,region,versions}.rs`,
    `src/controller/tests/realms.rs`, `src/net/regionping.rs`, `src/net/regionping/{harness,parse,tests}.rs`,
    `src/net/versions.rs`, `src/net/versions/{builtin,parse,tests}.rs`,
    `src/platform/{epic_library,epic_login,epic_session,loader_session}.rs`, `src/platform/keystore/epic.rs`.
  - **Category B — Reforged-derived (`DL4` clean-room rewrite scope: 8 module groups / 18 files, ported from Freeza's
    `ReforgedLauncher` additions)**:
    `src/net/runtime_files.rs`, `src/net/runtime_files/{manifest,tests}.rs`, `src/net/verify.rs` (1.14.7 `HashCache` /
    `ExeCheck`), `src/net/verify/cache_file.rs`, `src/platform/hostkit.rs`, `src/platform/hostkit/parse.rs`,
    `src/platform/hostserver.rs`, `src/platform/requirements.rs`, `src/platform/dialogs.rs`, `src/controller/host.rs`,
    `src/controller/host_off.rs`, `src/controller/runtime.rs`, `fixtures/host/src/fake_host_kit.rs`.
  - **Category C — DR-derived + Reforged-modified (60 files, copied from DR `e1c5fec`/`24b7624` in `a6e1870` and fitted
    to Reforged)**:
    `build.rs`, `src/main.rs`, `src/lib.rs`, `src/window.rs`,
    `src/bridge/{mod,backend,commands,generate,progress,spec,types}.rs`,
    `src/core/{mod,game_language,i18n,invite,legacy_pins,manifest,manifest_check,pins,status,text,username}.rs`,
    `src/net/{mod,autoregion,connector,downloader,downloader/tests,epic_install,hostapi,hostapi/legacy,hostapi/parse,hostapi/tests,http,http/to_file,pinned,relay,relay/local,relay/tests,relay_owner,relay_owner/table,verify/install}.rs`,
    `src/platform/{mod,art,client_mods,clipboard,game_config,game_config_tests,game_folder,invite_link,launch,links,linux_runtime,log,paths,settings,system,updater}.rs`,
    `src/platform/keystore/{mod,format,legacy,os}.rs`,
    `src/controller/{mod,game,legacy,legacy_off,platform,stored,tests,tests/legacy}.rs`.
  - **Non-Rust files in `launcher/`**:
    - `launcher/isolation/hook.js`: DR-derived Tauri isolation script (`AGPL-3.0-only`).
    - `launcher/src-tauri/gen/dauntless-1.4.4.json`: DR-derived 1.4.4 file manifest (`AGPL-3.0-only`).
    - `launcher/src-tauri/gen/strings.json`: Generated from `ui/packages/i18n/catalogs/{en,pt-BR}.json`
      (`AGPL-3.0-only`).
    - `launcher/test/cases/*.json`: Recorded parity test tables from DR and `ReforgedLauncher` (`AGPL-3.0-only`).
    - `launcher/test/{bundles,generated,isolation-hook}.test.ts` and `test/e2e-desktop.mjs`: Born here (`MIT`).
- **Evidence**: `NOTICE.md` line 113; `docs/port/spec-dl4-launcher.md`; `docs/port/migration-guide.md` §2 lines 83–104;
  `docs/port/delta-launcher-*.md`.
- **Dependencies**:
  - Unconditional: `module-api` (`MIT`), `epic` (`MIT`), **`protocol`** (`AGPL-3.0-only`), **`proxy`**
    (`AGPL-3.0-only`).
  - Feature `realms` (enabled in default client builds and in `server-gui`): **`rifts-host`** and **`rifts-seam`**
    (`AGPL-3.0-only` + Mystic Paradox additional terms).
  - Feature `server-gui`: `admin` (`MIT`), `server-controller` (`MIT`), **`service-kit`** (`AGPL-3.0-only`), **`host`**
    (`AGPL-3.0-only`, with `isles-1-14-7`, `isles-1-12-0`, `revived-1-4-4`, `realms`).

#### 1.3.18 `fuzz` (no `license` field in `Cargo.toml`)

- **Provenance**: 6 targets from DR row X9 (`http_body`, `proxy_headers`, `gateway_policy`, `gateway_policy_legacy`,
  `content_range`, `launcher_input`) + 33 targets born here or ported in `RX3`/`DX2`. Eight targets (`launcher_files`,
  `launcher_host`, `launcher_input`, `launcher_legacy`, `launcher_net`, `launcher_region`, `relay_input`,
  `runtime_input`) use `include!("../../launcher/src-tauri/src/...")`.
- **Dependencies**: `admin`, `chat`, `content`, `epic`, `gateway`, `history-ship`, `host`, `module-api`, `network`,
  `protocol`, `proxy`, `service-kit`, `store`.

---

### 1.4 `native/runtime-loader` (`MysticLoader`)

- **Provenance**: **Paradox-derived + Reforged-derived** (`src/lib.rs`, `src/exports.rs`, `src/proxy.rs`, 433 lines).
  Derived from `Mystic Paradox` `tools/RuntimeLoader/` (itself derived from `coldloader-proxy`), modified by Freeza's
  patches `0004` and `0013` (`mystic_loader.ini` and `MysticParadox.dll` / `MystPaxInternalServer.dll` selection).
- **Licence**: `native/runtime-loader/Cargo.toml` sets `license = "Apache-2.0"`, while `native/runtime-loader/LICENSE`
  contains the `LGPL-3.0` licence text. `NOTICE.md` lines 124–129 classify `native/runtime-loader` under Mystic Paradox
  (`AGPL-3.0-only` with additional terms) + Reforged patches.
- **Role**: Compiled to `winmm.dll` and placed in the server's runtime directory alongside `MysticParadox.dll`; served
  by `crates/content` (`GET /content/v1/runtime/files/winmm.dll`) and downloaded by `dainty-lass-client` into the
  player's Dauntless game folder after the player accepts the runtime prompt.

---

### 1.5 `ui/` workspace (`silentspud/dainty-lass-ui`)

| Package | `package.json` `license` | Provenance and files | Evidence |
| --- | --- | --- | --- |
| `ui/apps/web` | `MIT` | 100% **born here** (`DD11`, `DU1`–`DU5`, `DE7`, `DH3b`, `DH6`): `src/main.tsx`, `src/app/*.{ts,tsx}`, `src/screens/*.tsx`, `test/*.test.ts`. | `ui/NOTICE.md` lines 25–30 |
| `ui/packages/ui` | `MIT` | 100% **born here** (`DD11`, `DU1`): `src/components/*.tsx`, `src/lib/utils.ts` (shadcn/Radix primitives). | `ui/NOTICE.md` lines 26, 31 |
| `ui/packages/bridge` | `MIT AND AGPL-3.0-only` | `src/{api,call,index}.ts` are **born here** (`MIT`); `src/generated/types.ts` is generated from `launcher/src-tauri/src/bridge/{types,spec}.rs` (`AGPL-3.0-only`). | `ui/NOTICE.md` lines 27, 32, 37, 50–51 |
| `ui/packages/i18n` | `MIT AND AGPL-3.0-only` | `src/index.ts` and `test/catalogs.test.ts` are **born here** (`MIT`); `catalogs/en.json` and `catalogs/pt-BR.json` carry DR strings (`e1c5fec`/`24b7624`), `ReforgedLauncher` strings (`src/shared/i18n.ts`), and born-here keys appended after `err_edition_other` (`AGPL-3.0-only`). | `ui/NOTICE.md` lines 28, 33, 38, 43–49; `docs/dainty-lass-plan.md` `DD26` line 127 |

---

## 2. Workspace dependency graph and MIT-to-AGPL edges

### 2.1 Complete dependency matrix

| Crate | Declared licence | Depends on (`MIT` crates) | Depends on (`AGPL` crates / submodules) | Depended on by |
| --- | --- | --- | --- | --- |
| `crates/module-api` | `MIT` | — | — | `controller`, `protocol`, `store`, `network`, `chat`, `admin`, `content`, `gateway`, `host`, `launcher`, `fuzz`, `rustic-isles`, `rustic-rifts`, `legacy-revived` |
| `crates/epic` | `MIT` | — | — | `host`, `launcher`, `fuzz` |
| `crates/history-ship` | `MIT` | — | **`store`** | `network` (`log-ship`), `controller` (`network`), `fuzz` |
| `crates/network` | `MIT` | `module-api`, `history-ship` (`log-ship`) | **`protocol`** (`directory`), **`store`** (`directory`, `log-ship`) | `controller`, `gateway`, `admin`, `host`, `chat`, `fuzz` |
| `crates/admin` | `MIT` | `module-api`, `network` | **`protocol`**, **`service-kit`**, **`store`**; dev: **`gateway`** | `controller` (`admin`), `host` (`admin`), `launcher` (`server-gui`), `fuzz` |
| `crates/controller` | `MIT` | `module-api`, `network`, `admin` (`admin`), `history-ship` (`network`) | Unconditional: **`protocol`**, **`service-kit`**; default features: **`gateway`**, **`content`**, **`store`**; optional: **`isles-1-14-7`**, **`isles-1-12-0`** | `host`, `launcher` (`server-gui`), `server-console` (dev), `server-minimal` (dev) |
| `crates/server-console` | `MIT` | — (`controller` in dev) | **`host`** (with `modules`, `isles-1-14-7`, `isles-1-12-0`, `realms`) | Shipped binaries `dainty-lass-server-console`, `dainty-lass-host` |
| `crates/server-minimal` | `MIT` | — (`controller` in dev) | **`host`** (with `modules`, `isles-1-14-7`, `isles-1-12-0`, `realms`) | Shipped binary `dainty-lass-server-minimal` |
| `crates/backend` | `MIT` | — | — | None (Docker test stage stub) |
| `crates/director` | `MIT` | — | — | None (Docker test stage stub) |
| `crates/protocol` | `AGPL-3.0-only` | `module-api` | — | `controller`, `network`, `chat`, `admin`, `content`, `gateway`, `host`, `launcher`, `fuzz` |
| `crates/proxy` | `AGPL-3.0-only` | — | — | `gateway`, `launcher`, `fuzz` |
| `crates/service-kit` | `AGPL-3.0-only` | — | — | `controller`, `admin`, `content`, `gateway`, `host`, `chat`, `launcher` (`server-gui`), `fuzz`, `rustic-isles`, `legacy-revived` |
| `crates/store` | `AGPL-3.0-only` | `module-api` | — | `controller`, `history-ship`, `network`, `chat`, `admin`, `host`, `fuzz`, `rustic-isles`, `rustic-rifts`, `legacy-revived` |
| `crates/chat` | `AGPL-3.0-only` | `module-api`, `network` | **`protocol`**, **`service-kit`**, **`store`**; dev: **`gateway`** | `fuzz` |
| `crates/gateway` | `AGPL-3.0-only` | `module-api`, `network` (`no-default-features`) | **`protocol`**, **`proxy`**, **`service-kit`** | `controller`, `host`, `fuzz`, `admin` (dev), `chat` (dev) |
| `crates/content` | `AGPL-3.0-only` | `module-api` | **`protocol`**, **`service-kit`** | `controller`, `host`, `fuzz` |
| `crates/host` | `AGPL-3.0-only` | `controller`, `module-api`, `network`, `epic`, `admin` | **`protocol`**, **`service-kit`**, **`store`**, **`gateway`**, **`content`**, **`isles-1-14-7`**, **`isles-1-12-0`**, **`isles-core`**, **`isles-db`**, **`isles-director`** | `server-console`, `server-minimal`, `launcher` (`server-gui`), `fuzz` |
| `launcher/src-tauri` | `AGPL-3.0-only` | `module-api`, `epic`, `admin` (`server-gui`), `controller` (`server-gui`) | **`protocol`**, **`proxy`**, **`rifts-host`** (`realms`), **`rifts-seam`** (`realms`), **`service-kit`** (`server-gui`), **`host`** (`server-gui`) | Shipped binaries `dainty-lass-client`, `dainty-lass-manager` |

### 2.2 Existing MIT-to-AGPL dependency edges

Today, six crates declared `license = "MIT"` in their `Cargo.toml` depend directly on `AGPL-3.0-only` workspace crates:

1. **`crates/history-ship` (`MIT`) -> `crates/store` (`AGPL-3.0-only`)**:
   - Uses `store::Database`, `store::history::record::{ChangeRow, EntryRow, SnapshotManifest}`, and
     `store::recovery::restore` (all of which live in born-here files of `store`, while `store` as a crate is
     `AGPL-3.0-only` because of its 6 DR-derived files).
2. **`crates/network` (`MIT`) -> `crates/protocol` (`AGPL-3.0-only`) and `crates/store` (`AGPL-3.0-only`)**:
   - Under default feature `directory`, `src/username.rs` calls `protocol::account::check_username`, and
     `src/directory.rs`, `src/relay.rs`, and `src/home/ticket.rs` use `store::Database`.
   - Under default feature `log-ship`, `src/logship.rs` uses `history-ship` and `store::Database`.
   - Only when built with `--no-default-features` does `crates/network` have zero AGPL dependencies.
3. **`crates/admin` (`MIT`) -> `crates/protocol`, `crates/service-kit`, `crates/store` (`AGPL-3.0-only`)**:
   - Uses `protocol` (invite/status/username types), `service-kit` (HTTP server, body reader, JSON/line logging,
     routing, rate limiting, env parsing), and `store` (history query, revert, guarded action store).
4. **`crates/controller` (`MIT`) -> `crates/protocol`, `crates/service-kit`, `crates/store`, `crates/gateway`,
   `crates/content`, `isles-1-14-7`, `isles-1-12-0` (`AGPL-3.0-only`)**:
   - Unconditionally links `protocol` and `service-kit` (`ServiceContext`, `Telemetry`).
   - By default enables `gateway`, `allowlist`, `content`, and `network` (`store`).
5. **`crates/server-console` and `crates/server-minimal` (`MIT`) -> `crates/host` (`AGPL-3.0-only`)**:
   - Thin entry points calling `host::run_from_env()` / `host::serve::run_minimal_from_env()`.

And in `Part 2 (R2)`, the new remote-management crates (`crates/remote-protocol` and the remote-management code in the
Manager and managed installs) must be MIT and **must not depend on any AGPL crate**.

---

## 3. Effective licence of each shipped binary and Docker image

### 3.1 Shipped binaries

| Binary | Built from | Cargo features in release | Effective licence today | What still makes the compiled binary AGPL after the `DL3b` + `DL4b` rewrite |
| --- | --- | --- | --- | --- |
| **`dainty-lass-client`** (`DaintyLass Launcher`) | `launcher/src-tauri` | Default (`custom-protocol`, `realms`, `legacy`) | **`AGPL-3.0-only` with Mystic Paradox additional terms** | 1. **DR-derived launcher core** (`launcher/src-tauri` Category C: 60 files including `controller/mod.rs`, `net/{pinned,http,hostapi,relay,downloader,autoregion}`, `platform/{keystore,settings,game_config,game_folder,launch,invite_link,art,clipboard,client_mods}`, `core/manifest*`, `isolation/hook.js`).<br>2. **`crates/protocol` and `crates/proxy`** (unless rewritten as MIT).<br>3. **`rifts-host` and `rifts-seam`** (`modules/rustic-rifts`, `AGPL-3.0-only` + Mystic Paradox additional terms) compiled in by `--features realms`.<br>4. **Embedded UI strings**: `ui/packages/i18n/catalogs/{en,pt-BR}.json` and `launcher/src-tauri/gen/strings.json` (`AGPL-3.0-only`), plus `gen/dauntless-1.4.4.json` under `--features legacy`.<br>5. *(At runtime, not linked into the installer)*: for 1.14.7/1.12.0 servers, the client downloads `MysticParadox.dll` (`AGPL-3.0-only` + Mystic Paradox additional terms) and `winmm.dll` (`native/runtime-loader`, `Apache-2.0` / `LGPL-3.0`) from the server's content service after user consent. |
| **`dainty-lass-manager`** (`DaintyLass Manager`) | `launcher/src-tauri` | `--no-default-features --features custom-protocol,server-gui` | **`AGPL-3.0-only` with Mystic Paradox additional terms** | Everything listed for `dainty-lass-client` above, **plus**:<br>6. **`crates/host` AGPL shell** (`src/lib.rs`, `src/legacy/*`) linked by `server-gui`.<br>7. **In-process server services**: `crates/gateway`, `crates/content`, `crates/service-kit`, `crates/store`.<br>8. **Compiled-in game modules and generators**: `server-gui` enables `"isles-1-14-7", "isles-1-12-0", "revived-1-4-4", "host?/realms"`, linking `rustic-isles` (`isles-1-14-7`, `isles-1-12-0`, `isles-core`, `isles-db`, `isles-director`) and `rustic-rifts` (`AGPL-3.0-only` + Mystic Paradox additional terms) directly into the binary. |
| **`dainty-lass-server-console`** and **`dainty-lass-host`** (`reforged-host`) | `crates/server-console` | Default (`isles-1-14-7`, `isles-1-12-0`, `realms`) or `--features all-modules` in `.forgejo/workflows/server-release.yml` | **`AGPL-3.0-only` with Mystic Paradox additional terms** | 1. **Compiled-in game modules (`all-modules`)**: `isles-1-14-7`, `isles-1-12-0`, `isles-core`, `isles-db`, `isles-director` (`modules/rustic-isles`) and `rifts-*` (`modules/rustic-rifts`), which are `AGPL-3.0-only` with Mystic Paradox additional terms (plus `revived-1-4-4` if wired into `controller`).<br>2. **Direct `crates/host` links to `isles-core`, `isles-db`, `isles-director`** (for `gamedata`, `db migrate|verify|export-mongo`, `backend`, `director`, and embedded `*.example.json` / `store-captures.json`).<br>3. **`crates/host` AGPL shell** (`legacy/mod.rs` DR 1.4.4 supervision).<br>4. **Front-door and shared crates**: `crates/gateway`, `crates/content`, `crates/store`, `crates/service-kit`, `crates/protocol`, `crates/proxy` (unless rewritten or moved out of process). |
| **`dainty-lass-server-minimal`** | `crates/server-minimal` | Default (`isles-1-14-7`, `isles-1-12-0`, `realms`) or `--features all-modules` | **`AGPL-3.0-only` with Mystic Paradox additional terms** | Same as `dainty-lass-server-console` above (links `host` with `modules` and the same default/`all-modules` feature set). |

### 3.2 Docker images (`docker/Dockerfile` and `compose.yaml`)

| Image target (`docker/Dockerfile`) | Contents shipped in image | Effective licence today | Effective licence after `DL3b` + `DL4b` rewrite |
| --- | --- | --- | --- |
| **`front`** (`reforged-revival-front`, used by `gateway`, `content`, `allowlist`, `admin` services in `compose.yaml`) | `/opt/reforged/bin/reforged-host` (compiled from `crates/server-console` with `all-modules` in `rust-build`) + `/opt/reforged/bin/reforged-host-docker` (`docker/entrypoint.sh`). Image label: `org.opencontainers.image.licenses="AGPL-3.0-only"`. | **`AGPL-3.0-only` with Mystic Paradox additional terms** (because `reforged-host` is compiled once in `rust-build` with `isles-1-14-7`, `isles-1-12-0`, `realms`, `gateway`, `content`, `store`, `service-kit`, `protocol`). | Remains **`AGPL-3.0-only` with Mystic Paradox additional terms** as long as `front` copies the same `all-modules` `reforged-host` binary and runs `gateway`/`content`/`allowlist`. |
| **`server`** (`reforged-revival-server`, used by `host`, `configure`, `backend`, `director`) | `/opt/reforged/bin/reforged-host` (with `all-modules`; and in `REFORGED_EDITION=legacy`, also `metagame` and `deploy` from `modules/legacy-revived`), `/opt/reforged/native/runtime-loader`, `/opt/reforged/modules/rustic-isles/native/{ParadoxRuntime,CatalogExporter}`, `/opt/reforged/modules/rustic-isles/reference/1.14.7/.../*.example.json` and `store-captures.json`. | **`AGPL-3.0-only` with Mystic Paradox additional terms** | Remains **`AGPL-3.0-only` with Mystic Paradox additional terms** because it ships the `rustic-isles` game modules, C++ runtime/exporter sources, and 1.14.7 example data (or `legacy-revived` `metagame`/`deploy` binaries). |
| **`server-rollback`** (`reforged-revival-server-rollback`, used by `backend-rollback`, `director-rollback`) | Extends `server` with Node 22, `mongodump`/`mongorestore`, and the compiled TypeScript `ParadoxBackend` and `ParadoxDirector` trees from `modules/rustic-isles/reference/1.14.7/`. | **`AGPL-3.0-only` with Mystic Paradox additional terms** | Remains **`AGPL-3.0-only` with Mystic Paradox additional terms** (kept for one release until `RB15`). |
| **`netem`** | Debian `bookworm-slim` + `iproute2` + `/usr/local/bin/netem-entrypoint.sh` (`docker/network/netem.sh`, born here). | Labelled `AGPL-3.0-only` in `Dockerfile` (contains only Debian packages + born-here shell script). | Born-here script is `MIT`; container carries Debian packages. |
| **`test`** | Extends `server` with Rust toolchain and workspace sources for CI `docker/test-in-image.sh`. Never deployed. | **`AGPL-3.0-only` with Mystic Paradox additional terms** | Not a distributed release artefact. |

---

## 4. Proposed scope and trade-off analysis of candidate crates

### 4.1 Baseline scope already specified in `DD26`, `DL3a`, and `DL4a`

1. **`DL3b` — Host kit (`crates/hostkit`, `license = "MIT"`)**:
   - **What moves**: The 18 born-here source files (Category A) move from `crates/host` to `crates/hostkit`
     with `git mv` (separate mechanical commit).
   - **What is rewritten clean-room**: The 22 Reforged-derived module groups (Category B, specified in
     `docs/port/spec-dl3-hostkit.md`) are implemented fresh in `crates/hostkit` by an isolated subagent in a
     sparse-checkout worktree that excludes `crates/host` and all other AGPL paths.
   - **Decoupling `crates/hostkit` from `rustic-isles` (`isles-core`, `isles-db`, `isles-director`)**:
     Today, `crates/host` directly calls `isles_core::areas::game_data::generators`, `isles_director::hunt_tables`,
     and `isles_db::{migrate, verify, export_to_mongo, MongoClient}`, and embeds `modules/rustic-isles/reference/1.14.7/`
     JSON files. To make `crates/hostkit` 100% MIT with zero AGPL dependencies, `crates/hostkit` must define traits (or
     callbacks) for game-data generation and legacy MongoDB migration/verification/export that `crates/host` (the thin
     AGPL shell) injects when `isles-1-14-7` is enabled.
   - **What stays in `crates/host` (`AGPL-3.0-only`)**: The thin binary/library shell (`src/lib.rs`,
     `src/legacy/mod.rs`, `src/legacy/off.rs`, and the `isles-*` trait wiring).

2. **`DL4b` — Launcher kit (`crates/launcher-kit`, `license = "MIT"`)**:
   - **What moves**: The 31 born-here files (Category A) move from `launcher/src-tauri` to
     `crates/launcher-kit` with `git mv` (separate mechanical commit) once decoupled from DR-derived internal
     modules.
   - **What is rewritten clean-room**: The 8 Reforged-derived module groups (Category B, specified in
     `docs/port/spec-dl4-launcher.md`: `runtime_files`, `verify` 1.14.7 `HashCache`/`ExeCheck`, `hostkit`, `hostserver`,
     `requirements`, `dialogs`, `controller/host`, `controller/runtime`) are implemented fresh in
     `crates/launcher-kit` by an isolated subagent in a sparse-checkout worktree that excludes
     `launcher/src-tauri` and all other AGPL paths.
   - **What stays in `launcher/src-tauri` (`AGPL-3.0-only` unless expanded below)**: The DR-derived Tauri shell,
     relay, downloader, keystore, 1.4.4 verifier/legacy controller, and `rifts-host` realm wiring.

### 4.2 Candidate crates beyond `DL3` and `DL4`

Because `crates/hostkit`, `crates/launcher-kit`, `crates/controller`, `crates/history-ship`, `crates/network`,
`crates/admin`, and Part 2's remote-management crates (`crates/remote-protocol`, Manager remote client, and managed
agent) must not depend on AGPL crates, the following crates are candidates for clean-room MIT rewrite or dependency
inversion:

| Candidate crate | Size to rewrite | Why it matters (what it unblocks) | Trade-off / effort |
| --- | --- | --- | --- |
| **Candidate 1: `crates/protocol`** | **10 files, 2,284 lines** (5 small modules: `account`, `invite`, `status`, `manifest`, `text` + table tests) | **Highest leverage.** Depended on by `controller`, `crates/network`, `crates/admin`, `crates/hostkit`, `crates/launcher-kit`, `crates/gateway`, `crates/content`, and Part 2's remote management. Rewriting `protocol` clean-room from a behaviour spec makes `protocol` `MIT` and eliminates the single most pervasive AGPL dependency edge across the entire workspace. | Small, self-contained pure data/validation crate with zero I/O and clear wire specifications. **Strongly recommended.** |
| **Candidate 2: `crates/store` (6 DR-derived files)** | **6 src files, ~957 lines** (`lib.rs`, `adopt.rs`, `backup.rs`, `drizzle.rs`, `readonly.rs`, `snapshot.rs`; the other 13 files / 5,361 lines in `store` are already **born here**) | Depended on by `crates/history-ship` (`MIT`), `crates/network` (`MIT`), `crates/admin` (`MIT`), `crates/controller` (`MIT`), and `crates/hostkit` (`MIT`). Rewriting the ~957 lines of thin SQLite pragma/backup/snapshot/drizzle-migration helpers clean-room from a behaviour spec makes the entire `crates/store` crate `MIT`. That immediately removes the AGPL dependency from `history-ship`, `crates/network`, `crates/admin`, `controller`, and `hostkit`. | Small rewrite (~957 lines of standard `rusqlite` pragma/backup/table-inspection logic) with massive licensing payoff across five MIT crates. **Strongly recommended.** |
| **Candidate 3: `crates/service-kit`** | **Option 3A (split born-here `context.rs` + `telemetry.rs` + rewrite small `.env`/JSON/HTTP helpers, ~1,500 lines)** or **Option 3B (rewrite all 12 DR-derived src files, 2,726 lines, as MIT)** | `controller` and `hostkit` only need `ServiceContext`, `Telemetry` (both already **born here** in `RD26`, 485 lines), and basic `.env`/JSON helpers. `crates/admin` (`MIT`) also uses `service-kit`'s HTTP server (`serve.rs`), `routing.rs`, `body.rs`, `ratelimit.rs`, `log.rs`, `linelog.rs`, `env.rs`, and `json.rs`. Option 3B (rewriting the 12 DR-derived files of `service-kit` clean-room as MIT) makes `service-kit` 100% `MIT`, freeing `controller`, `hostkit`, **and** `crates/admin` from any `service-kit` AGPL dependency. | Medium-small effort (~2,700 lines of standard Hyper 1.x server, router, token-bucket rate limiter, `.env` parser, and line logger). **Recommended (Option 3B, or Option 3A if `admin`'s HTTP helpers are kept minimal).** |
| **Candidate 4: `crates/controller` (`server-controller`) feature inversion** | **Build / feature refactoring (~200 lines in `controller`, `host`, `bins`)** | `crates/controller` is 100% born-here (`MIT`), but its `Cargo.toml` has default features `gateway`, `allowlist`, `content`, and optional `isles-1-14-7`, `isles-1-12-0` that make `controller` depend directly on those AGPL crates. If `controller`'s default features are narrowed to MIT crates (or if `crates/host` / `bins/*` register `gateway`, `content`, and `isles-*` into `controller::ModuleRegistry` from outside), `crates/controller` has **zero** normal AGPL dependencies. | Very low effort; does not require a clean-room rewrite because `controller` is already 100% born-here. **Strongly recommended.** |
| **Candidate 5: `crates/proxy`** | **1 file, 193 lines** | Tiny RFC 9110 §7.6.1 hop-by-hop header stripper and bidirectional stream copy helper. Used by `crates/gateway` and `launcher/src-tauri` (`net/relay`). | Trivial (~100 lines clean-room from RFC 9110 §7.6.1). Worth including if `gateway` or the launcher relay is rewritten. |
| **Candidate 6: `crates/gateway` (with allowlist) and `crates/content`** | **54 DR/Reforged files across `gateway` (15,874 lines) and `content` (6,118 lines) — ~22,000 lines total** | `DD26` currently states: *"A clean-room `gateway` and `content` (about 18k lines, from the parity fixtures) is not planned."* Leaving them `AGPL-3.0-only` means any server binary or Docker `front` image that links `gateway` and `content` in-process is `AGPL-3.0-only` (unless `gateway` and `content` are spawned as separate out-of-process `module-api` child binaries or rewritten clean-room). | Large effort (~22k lines across TLS, ACME, nftables/netsh firewall allowlist, range requests, legacy 1.4.4 policies). Not recommended unless the maintainer wants the front-door binaries themselves to be MIT. |
| **Candidate 7: Remaining DR-derived core of `launcher/src-tauri` and `ui/packages/i18n/catalogs/*.json`** | **60 Rust files (~22,000 lines) + 2 JSON catalogs (~1,150 keys each)** | `DD26` and `DL4a` specify rewriting the Reforged-derived host/runtime modules into `crates/launcher-kit` (`MIT`) while leaving the DR-derived Tauri shell in `launcher/src-tauri` (`AGPL-3.0-only`) and keeping `en.json`/`pt-BR.json` word-for-word (`AGPL-3.0-only`). Consequently, the shipped `dainty-lass-client` and `dainty-lass-manager` binaries remain `AGPL-3.0-only` unless all 60 DR-derived files and both language catalogs are also replaced clean-room (and `realms` / `server-gui` AGPL game modules are moved out of process). | Very large effort (~22k lines of Rust + full UI copy rewrite). `DD26` explicitly scoped `DL4` to `crates/launcher-kit` (`MIT`) with a thin AGPL `src-tauri` shell. |

---

## 5. Questions for the maintainer (`L0` gate) — Answered 2026-10-30

The maintainer answered the four `L0` questions on 2026-10-30:

1. **DL2 (Freeza's licence consent)**: **No.**
   - All Reforged-derived code in scope must be rewritten clean-room from the behaviour specs (`DL3b`, `DL4b`, and
     `protocol`).
2. **Scope (which crates to rewrite as MIT)**: **As much as reasonably possible.**
   - In scope for clean-room MIT rewrite and dependency inversion (`L1`–`L3`):
     - **`DL3b` (`crates/hostkit`)**: Clean-room MIT rewrite of the 22 Reforged-derived modules in `crates/host` + move
       of the 18 born-here modules + trait decoupling from `isles-core`/`isles-db`/`isles-director`, leaving
       `crates/host` as a thin AGPL binary/legacy shell;
     - **`DL4b` (`crates/launcher-kit`)**: Clean-room MIT rewrite of the 8 Reforged-derived module groups in
       `launcher/src-tauri` + move of the born-here modules;
     - **`crates/protocol`**: Full clean-room MIT rewrite (~2.3k lines);
     - **`crates/proxy`**: Full clean-room MIT rewrite (1 file, 193 lines);
     - **`crates/store`**: Clean-room MIT rewrite of the 6 DR-derived SQLite helper files (`lib.rs`, `adopt.rs`,
       `backup.rs`, `drizzle.rs`, `readonly.rs`, `snapshot.rs` and `tests/store.rs`, ~957 lines) so the entire
       `crates/store` crate becomes `MIT`;
     - **`crates/service-kit`**: Clean-room MIT rewrite of the 12 DR-derived files (~2.7k lines) alongside the 2
       born-here files (`context.rs`, `telemetry.rs`) so the entire `crates/service-kit` crate becomes `MIT`;
     - **`crates/controller` (`server-controller`)**: Invert/decouple the `gateway`, `allowlist`, `content`, and
       `isles-*` dependencies so `controller` has zero normal AGPL dependencies.
   - Left `AGPL-3.0-only` (per Section 4.2 trade-off analysis and `DD26`): `crates/gateway`, `crates/content`,
     `crates/chat`, the DR-derived core of `launcher/src-tauri`, `ui/packages/i18n/catalogs/{en,pt-BR}.json`, and the
     submodule game repositories (`rustic-isles`, `rustic-rifts`, `legacy-revived`).
3. **Server binaries (`all-modules` in-process vs out-of-process `module-api`)**: **Keep them compiled in.**
   - The server binaries (`dainty-lass-server-console`, `dainty-lass-host`, `dainty-lass-server-minimal`, and
     `dainty-lass-manager`) keep `all-modules` compiled in, so the distributed binaries remain `AGPL-3.0-only` (with
     Mystic Paradox additional terms), while the MIT crates (`module-api`, `epic`, `protocol`, `proxy`, `service-kit`,
     `store`, `history-ship`, `crates/network`, `crates/admin`, `controller`, `hostkit`, `launcher-kit`, `remote`,
     `bins/*`) are 100% MIT with zero AGPL normal dependencies.
4. **Release (exact licence and copyright line)**:
   - `license = "MIT"`
   - `Copyright (c) 2026 Silent Spud`

---

## 6. Implementation status (`L1`–`L3` — Completed 2026-10-30)

All seven crates in the approved `L0` scope have been rewritten clean-room in isolated sparse-checkout worktrees and
swapped into the workspace under `license = "MIT"` (`Copyright (c) 2026 Silent Spud`):

| Crate | Behaviour spec (`L1`) | Provenance record (`L2`) | Licence after `L3` | Direct normal or dev AGPL dependencies |
| --- | --- | --- | --- | --- |
| `crates/protocol` | [`docs/port/spec-protocol.md`](spec-protocol.md) | [`docs/port/clean-room/protocol.md`](clean-room/protocol.md) | `MIT` | None |
| `crates/proxy` | [`docs/port/spec-proxy.md`](spec-proxy.md) | [`docs/port/clean-room/proxy.md`](clean-room/proxy.md) | `MIT` | None |
| `crates/store` | [`docs/port/spec-store.md`](spec-store.md) | [`docs/port/clean-room/store.md`](clean-room/store.md) | `MIT` | None |
| `crates/service-kit` | [`docs/port/spec-service-kit.md`](spec-service-kit.md) | [`docs/port/clean-room/service-kit.md`](clean-room/service-kit.md) | `MIT` | None |
| `crates/controller` | [`docs/port/clean-room/controller.md`](clean-room/controller.md) | [`docs/port/clean-room/controller.md`](clean-room/controller.md) | `MIT` | None (AGPL `gateway`, `content`, and `isles-*` injected via `CorePluginFactory`) |
| `crates/hostkit` | [`docs/port/spec-dl3-hostkit.md`](spec-dl3-hostkit.md) | [`docs/port/clean-room/hostkit.md`](clean-room/hostkit.md) | `MIT` | None (`crates/host` remains the thin AGPL binary/legacy/isles adapter shell) |
| `crates/launcher-kit` | [`docs/port/spec-dl4-launcher.md`](spec-dl4-launcher.md) | [`docs/port/clean-room/launcher-kit.md`](clean-room/launcher-kit.md) | `MIT` | None (`launcher/src-tauri` remains the AGPL Tauri shell) |


