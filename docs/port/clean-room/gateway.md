# Clean-Room Provenance Record — `services/gateway` (RD27)

- **Component**: `services/gateway` (`gateway` crate, `license = "MIT"`, `Copyright (c) 2026 Silent Spud`): the
  gateway, the allowlist helper, and the gateway library's `service_plugin::Service` (`gateway::service`)
- **Behaviour Specification**: `docs/port/spec-rd27-gateway.md`, `spec-rd27-gateway-front.md` and
  `spec-rd27-gateway-allowlist.md` at commit `26748dd` (branch `docs/rd27-spec-gateway`)
- **Clean-Room Worktree**: `R:\source\dauntless\.wt\gateway-mit` (branch `feat/rd27-gateway-mit`, cut from
  `26748dd`)

## 1. Worktree Isolation (`git sparse-checkout list`)

dainty-lass holds no AGPL tree (spec section 6), so nothing of restricted's could be checked out here. The sparse
checkout also left out the dainty-lass pages that describe the AGPL gateway's code or history rather than its
behaviour (the gateway catch-up list, the servers' delta file, the AGPL inventory, the hand-off notes):

```text
/*
!/docs/port/catchup-gateway.md
!/docs/port/delta-servers.md
!/docs/port/agpl-inventory.md
!/docs/port/handoff-*.md
!/docs/port/gemini-antigravity-prompt.md
```

Checks made after `git checkout` populated the worktree, before any code was written:

- `test -e services/gateway` reported `services/gateway absent`.
- `test -e docs/port/catchup-gateway.md` reported it absent.
- `git ls-tree -r --name-only HEAD | grep -iE 'gateway|allowlist'` listed only `crates/controller/src/service_library/gateway.rs`,
  `crates/service-plugin/src/gateway.rs`, `docs/port/catchup-gateway.md` (excluded above) and the three spec pages.

The implementer did not open, list, grep or `git show` anything under `R:\source\dauntless\dainty-restricted`,
`legacy-revived`, `rustic-*`, `dainty-dashboard`, `dainty-legacy`, `dainty-lass-ui`, any other `.wt\*` worktree, or
any historical version of `services/gateway`, `rust/services/gateway`, `services/gateway-plugin` or the allowlist.
No web search was made.

**Disclosure.** The session that ran the implementer was started (by the harness, not by the implementer) with its
working directory at `R:\source\dauntless\.wt\both\dainty-restricted`, a dainty-restricted worktree. The harness
attached that worktree's `git status` snapshot to the session's first message: its branch name, five recent commit
subjects (merge subjects of the RD27 core exe, chat library and gateway library rows) and two changed file names
(`crates/controller/tests/chat-service/Cargo.lock`, `scratch-test.txt`). No file content, diff or code came with
it, and every command the implementer ran used absolute paths into the clean-room worktree only.

## 2. Category A Files (left for the integration pass)

The spec's 17 Category A files live in restricted, which the implementer could not open. The crate builds and
its tests run against **placeholders** written only from the spec's description of what those files rely on, each
headed `PLACEHOLDER for a Category A file`. The integration pass replaces them with restricted's files unchanged
(SPDX MIT header added) and reconciles any signature the rewrite guessed differently (section 5).

| Category A file | Here |
| --- | --- |
| `gateway/src/modules.rs` | Placeholder `src/modules.rs` |
| `gateway/src/network.rs` | Placeholder `src/network.rs` |
| `gateway/src/nodelink.rs` | Placeholder `src/nodelink.rs` |
| `gateway/src/allowlist/guard.rs` | Placeholder `src/allowlist/guard.rs` |
| `gateway/src/allowlist/modules.rs` | Placeholder `src/allowlist/modules.rs` |
| `gateway/tests/{modules,network,nodelink,home,launcher_region}.rs` | Not present; copy in |
| `gateway-plugin/{Cargo.toml,src/lib.rs,tests/library.rs,tests/parity.rs,tests/support/mod.rs}` | Not present; copy in as `services/gateway-plugin` |
| `fuzz/fuzz_targets/{gateway_modules,gateway_network}.rs` | Not present; copy in |

Also left in restricted for the integration pass, as data (spec 5.1): `tests/parity/gateway.requests.json`,
`gateway.expected.json`, `tests/fixtures/node-kit/*` (3 files), and the security pages `docs/security/gateway.md`,
`allowlist.md` and `region-ping.md`.

## 3. Clean-Room Implementer Session (Category B)

One implementer (a Claude Code subagent, session `58d9615e-ca36-42f4-a71f-946172c4b359`; its transcript is the
session's log under `C:\Users\Nick\.claude\projects\R--source-dauntless-dainty-lass\`) wrote every Category B
source, test and fuzz target.

- **Files read**:
  - `docs/port/spec-rd27-gateway.md`, `spec-rd27-gateway-front.md`, `spec-rd27-gateway-allowlist.md`
  - `docs/port/clean-room/hostkit.md` (this record's format); `AGENTS.md`
  - `Cargo.toml`, `.cargo/config.toml`, `clippy.toml`, `.forgejo/workflows/ci.yml` (the MIT check, by grep)
  - `crates/service-plugin/src/{lib,gateway}.rs`, `crates/service-plugin/Cargo.toml`
  - `crates/controller/src/service_library/gateway.rs` (the core side of the seam)
  - `crates/proxy/src/lib.rs`, `crates/proxy/Cargo.toml`
  - `crates/service-kit/src/{env,linelog,js,jsjson,context,serve}.rs` (signatures and bodies of the helpers
    used), `crates/service-kit/Cargo.toml`
  - `crates/hostkit/src/tls.rs`, `crates/hostkit/Cargo.toml`
  - `plugins/network/src/{endpoint,wire,link,cluster}.rs` (parts), `plugins/network/Cargo.toml`
  - `crates/module-api/src/{registrar,module,network,manifest,ids,lib}.rs` (signatures, by grep)
  - `crates/protocol/src/{account,invite,lib}.rs` (signatures), `crates/protocol/Cargo.toml`
  - `crates/launcher-kit/src/net/regionping{.rs,/tests.rs}` (grep: the region report's body shape)
  - `fuzz/Cargo.toml`, `fuzz/fuzz_targets/protocol_input.rs` (the target format)
  - Third-party sources in the cargo registry, for API signatures only: `rcgen` 0.14.10, `rustls` 0.23.46,
    `windows` 0.62.2 (Windows Firewall, COM, OLE, Variant, Security, Threading), `hyper-util` 0.1.21
- **Files written**:
  - `services/gateway/Cargo.toml`
  - `services/gateway/src/{lib,cli,config,policy,gateway,guard,ip,ratelimit,redact,feed,capture,cert,server,service}.rs`
  - `services/gateway/src/allowlist/{mod,cli,config,ports,nft,firewall,windows,helper,state,sync}.rs`
  - Placeholders: `services/gateway/src/{modules,network,nodelink}.rs`,
    `services/gateway/src/allowlist/{guard,modules}.rs`
  - `services/gateway/tests/{common/mod,gateway,limits,allowlist,certificate}.rs`
  - `fuzz/fuzz_targets/{gateway_policy,gateway_capture,allowlist_input}.rs`, their `[[bin]]`s in `fuzz/Cargo.toml`
  - `Cargo.toml` (workspace member and dependencies), `Cargo.lock`
  - This record

## 4. Verification & Dependency Tree

- `cargo test -p gateway`: 70 unit tests, 20 integration tests (6 `gateway`, 6 `limits`, 6 `allowlist`, 2
  `certificate`), all passing on Windows.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: clean. `cargo fmt --all --check`: clean.
- The only `unsafe` is in `src/allowlist/windows.rs` (COM), each function `#[allow(unsafe_code)]` with a
  `SAFETY:` note per block. It is not exercised by the tests (the maintainer's elevated live check).
- `cargo tree -e normal -p gateway --depth 1` names only MIT workspace crates (`module-api`, `network`,
  `protocol`, `proxy`, `service-kit`, `service-plugin`) and third-party crates.

## 5. Assumptions the integration pass must check

The placeholders' interfaces are the implementer's guesses from the spec; restricted's Category A files may
differ. Where they do, the rewritten Category B side is what changes (spec section 4: signatures may change shape
when every caller changes with them).

- `modules::ModuleRoutes` (`new`, `with_stand_in`, `attach_stand_in`, `attach`, `detach`, `placed`, `stopped`,
  `route`), `modules::Routed { decision, endpoint }`, `modules::stand_in`.
- `policy::Decision` is an enum (`Reject { status, error, reason, class }`, `Proxy { upstream, class, feed }`,
  `Network { route, class }`); `policy::unwrap_origin` returns `Result<Unwrapped, BadOrigin>`.
- `network::{RegionReport, RegionHandler, parse_region_report}`; the body is the launcher's
  `{"regions":[{"region","rttMs"}]}`.
- `nodelink::{NodeLinks, EnrollHandler}`; `server::NodeLinkSlot = Arc<Mutex<Option<Arc<NodeLinks>>>>`.
- `allowlist::guard::PortGuard` (`new`, `text`, `covers`, `attach`, `detach`, `module_ports`, `set_on_change`) and
  `allowlist::modules::{UdpPorts, UdpMode}`.
- `feed::InProcessSink` is a trait with `allow(&self, ip)`; `feed::Clock` is a millisecond closure.

## 6. Similarity Measurement

Left for the reviewer: run `similarity.md`'s measurement against restricted's `services/gateway` at the spec's
commit, file to file.
