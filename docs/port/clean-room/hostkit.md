# Clean-Room Provenance Record — `crates/hostkit` (`DL3b`)

- **Component**: `rust/crates/hostkit` (`hostkit` crate, `license = "MIT"`, `Copyright (c) 2026 Silent Spud`)
- **Behaviour Specification**: `docs/port/spec-dl3-hostkit.md` at commit `0802cba` (merged on `plan` at `c98b633`)
- **Clean-Room Worktree**: `r:\source\dauntless\dainty-lass\.claude\worktrees\l2-hostkit` (branch `feat/l2-hostkit-mit`)

## 1. Worktree Isolation (`git sparse-checkout list`)

The clean-room worktree excluded all AGPL source trees (`/legacy/`, `/modules/`, `/rust/services/host/`, `/rust/services/gateway/`, `/rust/services/content/`, `/rust/services/backend/`, `/rust/services/director/`, `/rust/launcher/`, and `/reference/`):

```text
/*
!/legacy/
!/modules/
!/rust/services/host/
!/rust/services/gateway/
!/rust/services/content/
!/rust/services/backend/
!/rust/services/director/
!/rust/launcher/
!/reference/
```

`Test-Path rust/services/host` returned `False` before any clean-room implementer subagent was invoked.

## 2. Category A Files Moved / Decoupled (`Born-MIT`)

The following 20 Category A files were written originally for DaintyLass without external derivation (provenance documented in `docs/port/spec-dl3-hostkit.md` §1.1) and were placed in `rust/crates/hostkit/src/` with `SPDX-License-Identifier: MIT` headers and decoupled from AGPL crates via `crate::hooks::HostHooks`:

- `src/error.rs`
- `src/hooks.rs`
- `src/legacy.rs`
- `src/hosttoml.rs`
- `src/hostconfig/yaml.rs`
- `src/hostconfig/guard.rs`
- `src/hostconfig/modules.rs`
- `src/epicinstall.rs`
- `src/guarded.rs`
- `src/repl.rs`
- `src/tui.rs`
- `src/serve.rs`
- `src/lib.rs`
- `src/commands/backup.rs`
- `src/commands/db.rs`
- `src/commands/history.rs`
- `src/commands/install_game.rs`
- `src/commands/network.rs`
- `src/commands/setup/database.rs`
- `src/commands/setup/secrets.rs`
- `src/commands/start/binaries.rs`

## 3. Clean-Room Implementer Sessions (Category B Modules)

Four isolated clean-room subagents implemented the 22 Category B module groups purely from `docs/port/spec-dl3-hostkit.md` (`0802cba`):

### 3.1 Implementer 1 — Core Config, Process Supervision, TLS & CLI
- **Subagent Conversation ID**: `4323935b-6e42-416d-899b-4a8a39294a4b`
- **Transcript Log**: `C:/Users/Nick/.gemini/antigravity/brain/4323935b-6e42-416d-899b-4a8a39294a4b/.system_generated/logs/transcript.jsonl`
- **Files Read**:
  - `docs/port/spec-dl3-hostkit.md`
  - `docs/app-structure-plan.md`, `docs/config-toml-plan.md`, `docs/hosting.md`, `docs/hosting-linux.md`
  - `rust/Cargo.toml`, `rust/crates/hostkit/Cargo.toml`
  - Category A files in `rust/crates/hostkit/src/` (`lib.rs`, `error.rs`, `hooks.rs`, `legacy.rs`, `hosttoml.rs`, `hostconfig/{yaml,guard,modules}.rs`, `serve.rs`, `guarded.rs`, `repl.rs`, `tui.rs`, `epicinstall.rs`, `commands/{backup,db,history,install_game,network}.rs`, `commands/setup/{database,secrets}.rs`, `commands/start/binaries.rs`)
  - MIT workspace crates (`rust/crates/{service-kit,protocol,controller,store,module-api}/**`, `rust/plugins/network/src/cluster.rs`, `rust/bins/server-{console,minimal}/src/main.rs`)
- **Files Written**:
  - `rust/crates/hostkit/src/paths.rs`
  - `rust/crates/hostkit/src/envfile.rs`
  - `rust/crates/hostkit/src/hostconfig.rs`
  - `rust/crates/hostkit/src/run.rs`
  - `rust/crates/hostkit/src/services.rs`
  - `rust/crates/hostkit/src/tls.rs`
  - `rust/crates/hostkit/src/cli.rs`

### 3.2 Implementer 2 — Captures, Game Data, Packs, Preset, Progression & Publish
- **Subagent Conversation ID**: `61074018-b606-4918-b0a2-b0b1c50b12ce`
- **Transcript Log**: `C:/Users/Nick/.gemini/antigravity/brain/61074018-b606-4918-b0a2-b0b1c50b12ce/.system_generated/logs/transcript.jsonl`
- **Files Read**:
  - `docs/port/spec-dl3-hostkit.md`
  - `docs/hosting.md`, `docs/hosting-linux.md`, `docs/sdk-and-game-data.md`, `docs/security/host-gamedata.md`, `docs/port/catchup-host.md`, `docs/port/catchup-isles.md`
  - `rust/crates/hostkit/Cargo.toml` and Category A files in `rust/crates/hostkit/src/`
  - `rust/crates/service-kit/src/{lib,js,jsjson}.rs`
  - `rust/fuzz/fuzz_targets/host_gamedata_input.rs`
- **Files Written**:
  - `rust/crates/hostkit/src/captures.rs`
  - `rust/crates/hostkit/src/gamedata.rs`
  - `rust/crates/hostkit/src/gamepack.rs`
  - `rust/crates/hostkit/src/preset.rs`
  - `rust/crates/hostkit/src/progression.rs`
  - `rust/crates/hostkit/src/publish.rs`

### 3.3 Implementer 3 — Invite, Doctor, Mongo Wire, Linux, Selfupdate & Wine
- **Subagent Conversation ID**: `a34131c9-cceb-4551-a5c5-49641efcc99a`
- **Transcript Log**: `C:/Users/Nick/.gemini/antigravity/brain/a34131c9-cceb-4551-a5c5-49641efcc99a/.system_generated/logs/transcript.jsonl`
- **Files Read**:
  - `docs/port/spec-dl3-hostkit.md`
  - `docs/hosting.md`, `docs/hosting-linux.md`, `docs/app-structure-plan.md`
  - `rust/Cargo.toml`, `rust/.cargo/config.toml`, `rust/crates/hostkit/Cargo.toml`
  - Category A files in `rust/crates/hostkit/src/`
  - MIT workspace crates (`rust/crates/{protocol,service-kit,store}/**`)
- **Files Written**:
  - `rust/crates/hostkit/src/invite.rs`
  - `rust/crates/hostkit/src/doctor.rs`
  - `rust/crates/hostkit/src/mongo.rs`
  - `rust/crates/hostkit/src/mongo/wire.rs`
  - `rust/crates/hostkit/src/linux.rs`
  - `rust/crates/hostkit/src/selfupdate.rs`
  - `rust/crates/hostkit/src/wine.rs`

### 3.4 Implementer 4 — Command Handlers, Setup, Start/Control & Verification
- **Subagent Conversation ID**: `d222d98c-ceab-4763-b801-7091164ccdec`
- **Transcript Log**: `C:/Users/Nick/.gemini/antigravity/brain/d222d98c-ceab-4763-b801-7091164ccdec/.system_generated/logs/transcript.jsonl`
- **Files Read**:
  - `docs/port/spec-dl3-hostkit.md` (lines 1–1648)
  - `docs/port/catchup-host.md`, `docs/port/migration-guide.md`, `docs/security/host-start.md`, `docs/security/host-gamedata.md`, `docs/hosting.md`, `docs/hosting-linux.md`, `docs/app-structure-plan.md`, `MIGRATION_PLAN.md`
  - `rust/Cargo.toml`, `rust/crates/hostkit/Cargo.toml`, and `rust/crates/hostkit/src/**`
  - MIT workspace crates (`rust/crates/{controller,service-kit,store}/**`, `rust/services/admin/Cargo.toml`, `rust/plugins/{chat,network}/Cargo.toml`, `rust/bins/server-{console,minimal}/src/main.rs`, `rust/fuzz/fuzz_targets/launcher_host.rs`)
- **Files Written**:
  - `rust/crates/hostkit/src/commands/mod.rs`
  - `rust/crates/hostkit/src/commands/doctor.rs`
  - `rust/crates/hostkit/src/commands/gamedata.rs`
  - `rust/crates/hostkit/src/commands/invite.rs`
  - `rust/crates/hostkit/src/commands/multipliers.rs`
  - `rust/crates/hostkit/src/commands/preset.rs`
  - `rust/crates/hostkit/src/commands/publish_runtime.rs`
  - `rust/crates/hostkit/src/commands/setup.rs`
  - `rust/crates/hostkit/src/commands/setup/options.rs`
  - `rust/crates/hostkit/src/commands/setup/firewall.rs`
  - `rust/crates/hostkit/src/commands/setup/install.rs`
  - `rust/crates/hostkit/src/commands/setup/paradox.rs`
  - `rust/crates/hostkit/src/commands/start/mod.rs`
  - `rust/crates/hostkit/src/commands/start/control.rs`
  - `rust/crates/hostkit/src/commands/update.rs`

## 4. Verification & Dependency Tree (`cargo tree -e normal -p hostkit`)

- `cargo test -p hostkit`: **113 passed; 0 failed**
- `cargo test -p hostkit --features paradox`: **113 passed; 0 failed**
- `cargo clippy -p hostkit --all-targets -- -D warnings`: **0 warnings**
- `cargo clippy -p hostkit --features paradox --all-targets -- -D warnings`: **0 warnings**
- `cargo tree -e normal -p hostkit --depth 1`:

```text
hostkit v0.0.0
├── admin v0.0.0
├── aws-lc-rs v1.18.2
├── base64 v0.23.1
├── bytes v1.12.1
├── chrono v0.4.45
├── clap v4.6.7
├── controller v0.0.0
├── crossterm v0.29.0
├── epic v0.0.0
├── flate2 v1.1.10
├── fs4 v1.1.0
├── http-body-util v0.1.5
├── hyper v1.12.0
├── hyper-rustls v0.27.10
├── hyper-util v0.1.21
├── module-api v0.1.0
├── network v0.0.0
├── protocol v0.0.0
├── ratatui v0.30.2
├── rustls v0.23.46
├── serde_json v1.0.154
├── service-kit v0.0.0
├── sha2 v0.11.1
├── store v0.0.0
├── tar v0.4.46
├── thiserror v2.0.21
├── tokio v1.54.1
├── toml_edit v0.25.17+spec-1.1.0
├── tracing v0.1.44
├── url v2.5.8
├── windows v0.62.2
└── zip v9.0.5
```

Every workspace dependency of `hostkit` (`admin`, `controller`, `epic`, `module-api`, `network`, `protocol`, `service-kit`, `store`) is MIT-licensed. Zero AGPL crates appear anywhere in `cargo tree -e normal -p hostkit`.
