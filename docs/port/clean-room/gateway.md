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

Measured by the integration pass (which read restricted and DR) with `similarity.md` §1's method (trimmed non-blank,
non-comment lines, 3+-line runs), on the files as committed after integration: each Category B file against its
namesake in restricted `92c5e5b` (`feat/rd27-same-both-ways`), and against every file of DR's gateway at legacy-revived
`24b7624` and `cfcad3eb` (`rust/services/gateway/**/*.rs`, the gateway fuzz targets), the best one named. The full
table is `similarity.md` §2.8; every file at or above 25 % is read in its §3.29-§3.36.

### 6.1 Totals

| Files | New lines | Restricted matched | % | Best-DR matched | % |
| --- | ---: | ---: | ---: | ---: | ---: |
| The 23 `src` files | 7652 | 1564 | **20.4** | 1083 | 14.2 |
| The 5 rewritten tests | 1364 | 86 | **6.3** | 97 | 7.1 |
| The 3 rewritten fuzz targets | 156 | 19 | **12.2** | 6 | 3.8 |
| All 31 Category B files | 9172 | 1669 | **18.2** | 1186 | 12.9 |
| The same 31 at the implementer's `7250852` | 9147 | 1627 | 17.8 | 1159 | 12.7 |

Highest: `allowlist/mod.rs` 100 % (12 `pub mod` lines), `lib.rs` 48.8 %, `policy.rs` 35.1 %, `allowlist/ports.rs`
30.4 %, `allowlist/firewall.rs` 30.3 %, `allowlist/nft.rs` 27.2 %, `allowlist/sync.rs` 27.1 %; `server.rs`,
`config.rs`, `redact.rs` and `fuzz/gateway_policy.rs` 25.0 %. Every other file is under 25 %; the rewritten tests are
at 0-2.1 % but `tests/common` (13.4 %).

### 6.2 Every run of 6 lines or more, and the verdict

- `lib.rs`, `allowlist/mod.rs`: the module tree and `now_ms` (spec §4). Contract.
- `policy.rs`: `Upstream`, `LimitClass`, `FeedKind` with their texts, `Decision` with the field names the Category A
  `modules.rs` destructures, the unit tables of front §4.3 (the example lists, verbatim). Contract. The 28-line run of
  `split_target`, `check_path` and the head of `canonical_path` matches DR as well: front §4.1 lists the checks in
  that order and the code is the list. Judged spec-forced; it is the closest run in the crate, and a reviewer who
  objects should look at the spec's sentence first.
- `config.rs`: `Endpoint` and `authority()`, `Rates`, `Timeouts`, `Config`'s public fields (spec §4), the secret
  check's error text (front §1), the timeout bounds, the `Ok(Self { .. })` field list. Contract and idiom.
- `server.rs`: the certificate-validity error text (front §2.1), the ready line's `mode` texts (front §16). Contract.
- `gateway.rs`: `Limiters` (one per `Rates` field), the stripped forwarding headers (front §7.1's list), two
  `http_body::Body` delegations (`is_end_stream`, `size_hint`), the 60 s sweeper (front §11), the throttled
  `clienterror` line (front §3.2), `Gateway::new`'s signature (the Category A tests' calls, section 7.2), the
  `forward_headers` test's expected names. Contract and idiom.
- `guard.rs`, `capture.rs`, `windows.rs`: `use` blocks; `capture.rs`' `ChatTap` `AsyncRead`/`AsyncWrite`
  delegation, `redact_capture_headers`' skip list (front §12.4), `ChatMessage` (spec §4). Idiom and contract.
- `ip.rs`: `ParsedIp`'s fields (spec §4), the allowlist refusal reasons (allowlist §4), the IPv6 bad-input table.
- `ratelimit.rs`, `redact.rs`, `feed.rs`: error and warning texts the front page fixes (§11, §14, §10).
- `cert.rs`: the rustls server builder (TLS 1.3 and 1.2, ALPN `http/1.1`, front §2.1) and the certificate's
  distinguished name and key usages (front §2.2). Contract and library idiom.
- `allowlist/config.rs`: `FirewallKind` with `as_str`, `AllowlistConfig`'s public fields. Contract.
- `allowlist/ports.rs`, `firewall.rs`, `nft.rs`: error texts, the four rule constants, the expected nft script and the
  private file's settings, all given by the allowlist page (§2, §9, §10). Contract.
- `allowlist/helper.rs`: the `status` JSON (allowlist §6), two accessors, and a 7-line accept-loop fragment
  (`try_acquire_owned` on a connection semaphore, then spawn) that DR also has. Idiom.
- `allowlist/state.rs`: `Refresh`'s variants (spec §4), `len`/`is_empty`. Contract and idiom.
- `allowlist/sync.rs`: `ApplyResult` and the private `State`'s ten fields, in the order allowlist §8 lists the state.
  Spec-forced (the spec enumerates the private state).
- `tests/common/mod.rs`: `Answer::header`. Idiom.

**Verdict.** No run is listed for a clean-room redo. Every run of 6 lines or more is a public name or shape that spec
§4 or a Category A caller fixes, a text or table the spec pages give, or a short Rust idiom. Two runs follow the
spec's wording closely enough that the spec, not the code, is the place to look if a reviewer objects: `check_path`'s
ordered checks (front §4.1) and the sync's private state (allowlist §8).

### 6.3 Measured apart

| File | Written by | Against | New lines | Matched | % |
| --- | --- | --- | ---: | ---: | ---: |
| `tests/common/compat.rs` | integration (section 7.4) | restricted `tests/common/mod.rs` | 172 | 41 | 23.8 |
| `tests/parity/harness.rs` | integration (section 7.5) | restricted `tests/parity/harness.rs` | 434 | 49 | 11.3 |
| `tests/parity.rs` | integration (section 7.5) | restricted `tests/parity.rs` | 48 | 10 | 20.8 |
| the rewrite's `src/service.rs` (dropped, section 7.3) | implementer | restricted `src/service.rs` (Category A) | 452 | 93 | 20.6 |

The integration files' longest runs are the twelve recorded header names (spec §5.1), `harness::Server` and
`harness::run`'s signature (the Category A `gateway-plugin/tests/parity.rs` calls them), and the `Ports`, `Options`
and `Reply` fields the Category A tests use.

## 7. Integration

Done in `R:\source\dauntless\.wt\gateway-int` (branch `feat/rd27-gateway-mit-int`, from `7250852`). The integration
pass read restricted's Category A files and the spec, wrote the code of sections 7.2-7.5 from those alone, and only
then opened restricted's Category B files and DR, for section 6.

### 7.1 Category A, copied in

From restricted `92c5e5b` (`feat/rd27-same-both-ways`), byte for byte, with `// SPDX-License-Identifier: MIT`,
`// Copyright (c) 2026 Silent Spud` and a blank line prepended to each Rust file: `src/{modules,network,nodelink}.rs`,
`src/allowlist/{guard,modules}.rs`, `tests/{modules,network,nodelink,home,launcher_region}.rs`,
`gateway-plugin/{src/lib.rs,tests/library.rs,tests/parity.rs,tests/support/mod.rs}`,
`fuzz/fuzz_targets/{gateway_modules,gateway_network}.rs`. Each, less its first three lines, is identical to
`92c5e5b`. `src/service.rs` is restricted's too (section 7.3). `gateway-plugin/Cargo.toml` takes `license = "MIT"`
and `network_plugin` by path, as `gateway` has it. Data, as spec §5.1 recommends, unchanged:
`tests/parity/gateway.{requests,expected}.json` (they differ from DR's parity files) and `tests/fixtures/node-kit/*`.
The security pages `docs/security/{gateway,allowlist,region-ping}.md`, unchanged (spec §6).

How each was checked: `git log --follow` in restricted gives every Rust file's first commit as the spec's row says
(DF4a `a99292e`, DN7 `47ec5d2`, DN3 `f0b0586`, DF4d `2a002d7`, DN5 `8308acc`, RD27 `0bf7226`); `launcher_region.rs`
came in at the split (`62d56c6`) from DN7's work in dainty-lass. None is present at restricted's new root `7338b10`.
The security pages: `region-ping.md` is DN7's; `gateway.md` and `allowlist.md` were present at the new root (their
earlier history is in the pre-root bundle, not checked) and share no 3-line run with any markdown file of DR or
dainty-legacy. The spec moves them (§6); the maintainer may still want them rewritten, as content's page was.

`launcher-kit` joins `gateway`'s dev-dependencies (`launcher_region.rs`). `services/gateway-plugin` joins the
workspace `members`; the two moved fuzz targets get their `[[bin]]` entries.

The branch merges `feat/rd27-library-hooks` (`d86890c`): the both-ways `gateway-plugin` tests call
`controller::service_library::gateway::gateway_compiled_in`, which only that branch has. It must reach `plan` first.

### 7.2 The rewritten side fitted to the real Category A files

No Category A file was edited to fit.

| Where | Assumed (section 5) | Real | Edit |
| --- | --- | --- | --- |
| `policy.rs` | `Decision` field `class`; `Upstream::Chat`; `unwrap_origin -> Result<_, BadOrigin>` | `limit_class`; `Upstream::Ws`; `Option<Unwrapped>` | renamed; `BadOrigin` gone |
| `gateway.rs`, `server.rs` | `ModuleRoutes::with_stand_in`, `route(&GameVersion, ..)` | `ModuleRoutes::standalone(&Config)`, `route(&str, &GameVersion, ..)` | the stand-in from the config; the version as text |
| `gateway.rs`, `server.rs` | `RegionHandler(RegionReport)` | `network::RegionReportHandler(key, ticket, regions)` | called with the three |
| `gateway.rs`, `server.rs` | `nodelink::EnrollHandler`; `NodeLinks::{caller, enroll, info, identity}`; `server_config() -> Arc` | `network::EnrollHandler`; `nodelink::caller(chain) -> NodeCaller`, `enroll_handler`, `certificate`, no `identity`; `-> &Arc` | renamed; the node and region read from `NodeLinkConfig` |
| `config.rs` | its own `NetworkNode` | `network::NetworkNode` | the real one used |
| `feed.rs`, `allowlist/helper.rs` | `InProcessSink` a trait | `Arc<dyn Fn(&str) -> bool>` | a type alias; `HelperSink::into_sink` |
| `allowlist/helper.rs`, `allowlist_input` | `PortGuard` (`set_on_change(ports, modules)`, `covers`, `module_ports`, `attach(&str)`) | `Guard` (`on_change(Fn())`, `covers_port`, `modules`, `attach(&ModuleId)`) | renamed; the callback reads the guard (weakly) for its text; the unused `ports::ranges` dropped |
| `gateway.rs` | `new`/`with_routes(.., AllowlistFeed, ..) -> Self`; `serve(&self, listener, impl Future)`; `set_region_handler(Option<_>)`; node links loaded by `server.rs` | the Category A tests call `new(.., None, None) -> Arc<Gateway>`, `serve(listener, watch::Receiver)`, `set_region_handler(handler)`, and read `node_links()` right after `new` | `Option<AllowlistFeed>`, `Arc<Self>`, `serve(self: Arc<Self>, ..)` over the old body (now `serve_until`); `with_routes` loads the node links the config names (moved from `server.rs`, which still logs them) |

### 7.3 `GatewayService`: restricted's kept, the rewrite's `src/service.rs` dropped

The both-ways `gateway-plugin/src/lib.rs` is one line, `export_service!(gateway::service::GatewayService::library())`.
Its glue moved from the Category A `gateway-plugin/src/lib.rs` of `0bf7226` to restricted's
`services/gateway/src/service.rs` (`36a0d68`, RD27). The brief preferred the rewrite's service; it was not kept,
because fitting it to the real Category A types needed new code, not signature fixes: a placement view implementing
`modules::RegistryView` and `allowlist::modules::PortView` (the guessed `ModuleRoutes::placed`/`stopped` and
`UdpPorts` held it), an `AllowlistPlugin` chosen per allowlist mode, the stand-in's own port, and a `library()`
constructor that routes linelog through tracing. The integration had read restricted's version, so writing those would
not have been clean-room. Restricted's file is Category A by ancestry, uses only the rewrite's public API, and compiled
against it once `feed::InProcessSink` was the closure type it names. The rewrite's version is measured in section 6.3
only.

### 7.4 The moved tests' helpers (`tests/common/compat.rs`)

The Category A tests call helpers by names the rewrite's `tests/common` did not have (spec §5.3 says what they need,
not the names): `make_cert`, `Ports`, `dead_port`, `test_config`, `start_gateway`, `start_upstream`,
`start_ws_upstream`, `Options`, `Reply`, `https_request`, `tcp_connect`, `tls_connect`, `GATEWAY_SECRET`,
`Running::close`, `Upstream::{count, last}`, and `TestCert`'s `cert_file`/`key_file`. They are thin wrappers over the
rewrite's helpers, written from those calls and the spec before any restricted Category B file was opened.
`log_lines()` now returns the shared log (`(level, JSON line)` pairs), as `gateway-plugin/tests/parity.rs` reads it;
the rewrite's own tests use `log_texts()`.

### 7.5 The parity harness (`tests/parity/harness.rs`, `tests/parity.rs`)

Neither existed. Written from spec §5.1, the fixtures and `gateway-plugin/tests/parity.rs`'s calls, before any
restricted Category B file was opened; `record.cjs` was not read. The fixtures settle three points the spec leaves
open: the fake backend answers `POST /heartbeat` without a bearer with 401 (empty `text/plain`),
`POST /account/api/oauth/token` with `{"access_token":"x"}` and `POST /game/login` and `/login` with `{}`; the chat
upstream echoes what it gets as `echo:<bytes>`, unframed (the recorded `websocket closed` counts are 5 in and 10 out
for `hello`); and it answers `connection: upgrade` in lower case.

### 7.6 Verification

On Windows, with `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=line-tables-only` and `-j 4`:

- `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --locked -- -D warnings`: clean.
- `cargo test -p gateway -p gateway-plugin --locked --no-fail-fast` (after `cargo build -p gateway-plugin`): every
  test passes (89 unit; `allowlist` 6, `certificate` 2, `gateway` 6, `home` 1, `launcher_region` 2, `limits` 6,
  `modules` 1, `network` 5, `nodelink` 12; `library` 2) but the two parity tests.
- Parity three ways (in-process, `gateway/tests/parity.rs`; compiled in and through the library,
  `gateway-plugin/tests/parity.rs`, the library half run alone from a temporary copy because the first half fails):
  57 of 58 entries match in each. The one that differs is a rewrite defect: `GET http://evil.example/reforged/api/
  CreateInvite` (absolute form) must be 400 `bad_request` `target_not_origin_form`, logged with the target as sent;
  the rewrite judges hyper's parsed path, answers 403 `admin_route` and logs `/reforged/api/CreateInvite`. Front §4.3
  step 1 is clear. The fix (classify the request line's target as sent) is left to a clean-room session.
- `cargo check --manifest-path fuzz/Cargo.toml`: the five gateway targets build; the fuzz workspace fails on
  `launcher_host` (`parse::server_config` is not in `launcher-kit`), which this branch does not touch.
- Not verified: the Windows Firewall COM path elevated, the real nftables test, anything against the game client.
