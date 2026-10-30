// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

# Clean-Room Provenance Record — `crates/launcher-kit` (`DL4b`)

- **Component**: `rust/crates/launcher-kit` (`launcher-kit` crate, `license = "MIT"`, `Copyright (c) 2026 Silent Spud`)
- **Behaviour Specification**: `docs/port/spec-dl4-launcher.md` at commit `b987ae1` (merged on `plan` at `58ee88d`)
- **Clean-Room Worktree**: `r:\source\dauntless\dainty-lass\.claude\worktrees\l2-launcher-kit` (branch `feat/l2-launcher-kit-mit`)

## 1. Worktree Isolation (`git sparse-checkout list`)

The clean-room worktree excluded all AGPL source trees (`/legacy/`, `/modules/`, `/rust/services/`, `/rust/launcher/`, and `/reference/`):

```text
/*
!/legacy/
!/modules/
!/rust/services/
!/rust/launcher/
!/reference/
```

`Test-Path rust/launcher`, `Test-Path legacy`, and `Test-Path rust/services` all returned `False` before any clean-room implementer subagent was invoked.

## 2. Category A Files Moved / Decoupled (`Born-MIT`)

The following Category A files were written originally for DaintyLass without external derivation (provenance documented in `docs/port/spec-dl4-launcher.md` §1.1) and were placed in `rust/crates/launcher-kit/src/` with `SPDX-License-Identifier: MIT` headers:

- `src/bridge/modules.rs` (`DU3` / `DD11` module panel validation and sanitisation)
- `src/net/regionping.rs` (`DN7` / `DD19` multi-region RTT ping measurement and reporting)
- `src/net/regionping/parse.rs` (`DN7` node list parser, compiled into fuzz target `launcher_region`)
- `src/net/regionping/harness.rs` (`DN12` Docker network latency harness check)
- `src/net/regionping/tests.rs` (`DN7` loopback gateway ping tests)
- `src/net/versions.rs` (`DE4` / `DD22` server version and client profile discovery)
- `src/net/versions/builtin.rs` (`DE4` compiled-in client profiles)
- `src/net/versions/parse.rs` (`DE4` module manifest validation for served profiles)
- `src/net/versions/tests.rs` (`DE4` synthetic manifest and source tests)
- `src/platform/loader_session.rs` (`RD9` zero-injection `mystic_loader.ini` session management)

## 3. Clean-Room Implementer Sessions (Category B Modules)

Two isolated clean-room subagents implemented the Category B (`DL4 Rewrite`) modules purely from `docs/port/spec-dl4-launcher.md` (`58ee88d`):

### 3.1 Implementer 1 — Core, Bridge, and Network Verification & Runtime Sync
- **Subagent Conversation ID**: `82d733d1-33bc-4e9c-afc7-683e27848369`
- **Transcript Log**: `C:/Users/Nick/.gemini/antigravity/brain/82d733d1-33bc-4e9c-afc7-683e27848369/.system_generated/logs/transcript.jsonl`
- **Files Read**:
  - `docs/port/spec-dl4-launcher.md`
  - `rust/crates/protocol/src/lib.rs`, `rust/crates/protocol/src/manifest.rs`, `rust/crates/protocol/src/text.rs`, `rust/crates/protocol/src/invite.rs`, `rust/crates/protocol/src/account.rs`
  - `rust/crates/launcher-kit/Cargo.toml` and Category A files in `rust/crates/launcher-kit/src/` (`bridge/modules.rs`, `net/regionping.rs`, `net/versions.rs`, `platform/loader_session.rs`)
  - `rust/fuzz/fuzz_targets/runtime_input.rs`
- **Files Written**:
  - `rust/crates/launcher-kit/src/lib.rs`
  - `rust/crates/launcher-kit/src/core.rs`
  - `rust/crates/launcher-kit/src/core/pins.rs`
  - `rust/crates/launcher-kit/src/core/text.rs`
  - `rust/crates/launcher-kit/src/core/legacy_pins.rs`
  - `rust/crates/launcher-kit/src/bridge.rs`
  - `rust/crates/launcher-kit/src/bridge/types.rs`
  - `rust/crates/launcher-kit/src/bridge/spec.rs`
  - `rust/crates/launcher-kit/src/net.rs`
  - `rust/crates/launcher-kit/src/net/pinned.rs`
  - `rust/crates/launcher-kit/src/net/http.rs`
  - `rust/crates/launcher-kit/src/net/http/to_file.rs`
  - `rust/crates/launcher-kit/src/net/hostapi.rs`
  - `rust/crates/launcher-kit/src/net/verify.rs`
  - `rust/crates/launcher-kit/src/net/verify/cache_file.rs`
  - `rust/crates/launcher-kit/src/net/runtime_files.rs`
  - `rust/crates/launcher-kit/src/net/runtime_files/manifest.rs`
  - `rust/crates/launcher-kit/src/net/runtime_files/tests.rs`

### 3.2 Implementer 2 — Platform Hostkit, HostServer, Requirements, Dialogs, Linux Runtime & System
- **Subagent Conversation ID**: `775f8bf0-eab9-48c5-b2c3-e1e66de4e72c`
- **Transcript Log**: `C:/Users/Nick/.gemini/antigravity/brain/775f8bf0-eab9-48c5-b2c3-e1e66de4e72c/.system_generated/logs/transcript.jsonl`
- **Files Read**:
  - `docs/port/spec-dl4-launcher.md`
  - `rust/crates/launcher-kit/Cargo.toml` and Category A files in `rust/crates/launcher-kit/src/`
  - `rust/fuzz/fuzz_targets/launcher_host.rs`
- **Files Written**:
  - `rust/crates/launcher-kit/src/platform.rs`
  - `rust/crates/launcher-kit/src/platform/hostkit.rs`
  - `rust/crates/launcher-kit/src/platform/hostkit/parse.rs`
  - `rust/crates/launcher-kit/src/platform/hostserver.rs`
  - `rust/crates/launcher-kit/src/platform/requirements.rs`
  - `rust/crates/launcher-kit/src/platform/dialogs.rs`
  - `rust/crates/launcher-kit/src/platform/linux_runtime.rs`
  - `rust/crates/launcher-kit/src/platform/system.rs`

## 4. Verification & Dependency Tree (`cargo tree -p launcher-kit`)

- `cargo test -p launcher-kit -j 2 -- --skip the_gateway_serves_these_routes --test-threads=1`: **82 passed; 0 failed**
- `cargo clippy -p launcher-kit --all-targets -j 2 -- -D warnings`: **0 warnings**
- Prohibited-term and RD9 no-injection grep across `rust/crates/launcher-kit/src/**/*.rs`: **0 matches**
- `cargo tree -p launcher-kit --depth 1`:

```text
launcher-kit v0.0.0
├── base64 v0.23.1
├── bytes v1.12.1
├── chrono v0.4.45
├── dirs v6.0.0
├── epic v0.0.0
├── fs4 v1.1.0
├── futures-util v0.3.34
├── http-body-util v0.1.5
├── hyper v1.12.0
├── hyper-rustls v0.27.10
├── hyper-util v0.1.21
├── module-api v0.1.0
├── percent-encoding v2.3.2
├── protocol v0.0.0
├── rand v0.10.3
├── regex v1.13.1
├── rustls v0.23.46
├── serde v1.0.229
├── serde_json v1.0.154
├── sha1 v0.11.0
├── sha2 v0.11.1
├── tokio v1.54.1
├── tokio-rustls v0.26.6
├── tokio-util v0.7.20
├── toml_edit v0.25.17+spec-1.1.0
├── tracing v0.1.44
├── url v2.5.8
└── webpki-roots v1.0.10
[dev-dependencies]
├── protocol v0.0.0
├── rcgen v0.14.10
├── sha1_smol v1.0.1
├── tempfile v3.27.0
├── tokio-stream v0.1.19
└── ts-rs v12.0.1
```

Every workspace dependency of `launcher-kit` (`epic`, `module-api`, `protocol`) is MIT-licensed. Zero AGPL crates appear anywhere in normal or dev dependencies of `launcher-kit`.
