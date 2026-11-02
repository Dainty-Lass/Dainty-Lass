<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: the gateway and the allowlist helper (RD27)

This is the clean-room specification for relicensing dainty-restricted's AGPL gateway (`services/gateway`, which
holds the allowlist helper too) and its library wrapper (`services/gateway-plugin`) under MIT in dainty-lass. The
writer read the AGPL code; the implementer never does, and rebuilds the crate from these pages, the MIT crates it
already has (`proxy`, `service-kit`, `protocol`, `module-api`, `plugins/network`, `hostkit`, `service-plugin`,
`controller`) and the black-box test vectors named in section 5.

The gateway is the only listener off loopback and a trust boundary, and the allowlist helper runs with the most rights
of any server part. Every rule in these pages is a security rule unless it says otherwise. When a sentence here and a
parity fixture disagree, the fixture wins and the sentence is the bug: tell the spec writer.

It follows the method of [`spec-dl3-hostkit.md`](spec-dl3-hostkit.md), [`spec-proxy.md`](spec-proxy.md) and
[`clean-room/hostkit.md`](clean-room/hostkit.md); the similarity check of
[`clean-room/similarity.md`](clean-room/similarity.md) applies to the result (section 6).

## The pages

| Page | What it specifies |
| --- | --- |
| This page | Scope, the file classification (A/B), the library seam, the public API restricted needs, the target design, the tests, the risks |
| [`spec-rd27-gateway-front.md`](spec-rd27-gateway-front.md) | The gateway: settings, TLS and `make-cert`, connections and timers, the route policy, module routes, proxying, WebSocket upgrades, the network, node-link and enroll routes, the allowlist feed, rate and body limits, logging and redaction, the traffic capture, every status and body |
| [`spec-rd27-gateway-allowlist.md`](spec-rd27-gateway-allowlist.md) | The allowlist helper: settings, the address checks, the loopback HTTP endpoint, state and expiry, the audit log, the firewall sync, nftables, Windows Firewall, dry run, the module UDP ranges (guard and core plugin), the in-process use |

Words used throughout: *the core* is dainty-lass's MIT controller process; *restricted* is the AGPL repository;
*Node* is the Reforged Node gateway whose answers the parity fixtures recorded; *a refusal* is the gateway's own JSON
error answer (front page, section 13).

---

## 1. Scope

In: everything under restricted's `services/gateway/` (`src/`, `tests/`, `Cargo.toml`) and
`services/gateway-plugin/` (`src/lib.rs`, `tests/`, `Cargo.toml`), and the five fuzz targets that exercise them
(`gateway_policy`, `gateway_capture`, `gateway_modules`, `gateway_network`, `allowlist_input`).

Out, already MIT and only consumed: `crates/proxy` (hop-by-hop filtering and the idle-timed tunnel), `service-kit`
(`env::Env`, `js` helpers, `linelog` with its `Throttle`, `metrics::http_request`, `context::ServiceContext`,
`serve::stop_requested`), `protocol` (`invite::reforged_version`, `invite::version_alias`,
`account::is_plausible_account_key`, `GameVersion`), `module-api` (manifests, registrations, `CorePlugin`,
`PortRange`, `NodeIdentity`), `plugins/network` (`cluster`, `endpoint`, `wire`, `link::MAX_MESSAGE`), `hostkit::tls`
(self-signed certificate generation), `service-plugin::gateway` (the seam, section 3) and
`controller::service_library::gateway` (the core side of the seam).

## 2. File classification

Evidence used: restricted's history was given a new root on 2026-10-26 (`7338b10`, "a new root without the Dauntless
Reforged Revival line"). A file present at that root came down the DR to Reforged line, and the migration guide
section 2.3 maps each to the DR or Reforged Node file it ports. A file first added after it by a DaintyLass row
(DF4a, DF4d, DN3, DN5, DN7, RD27) is born here. Two files that moved after the root (`cli.rs` from `main.rs`,
`allowlist/cli.rs` from `bin/allowlist.rs`, commit `52eaddd`) keep their origin.

### 2.1 Category A: born in DaintyLass, may move to MIT as they are (17 files)

| File | Row and commit that added it | Note for the move |
| --- | --- | --- |
| `gateway/src/modules.rs` | DF4a, `a99292e` | Uses the policy's public helpers; works unchanged against a rewritten policy that keeps their names (front page, section 5) |
| `gateway/src/network.rs` | DN7, `47ec5d2` (extended by DN6/DN8/RM2) | None |
| `gateway/src/nodelink.rs` | DN3, `f0b0586` (extended by RM2, `109a59c`) | Reads `config::NodeLinkConfig`; keep that type's fields |
| `gateway/src/allowlist/guard.rs` | DF4d, `2a002d7` | Calls `ports::check_ports` and `MAX_RANGES`; keep those names |
| `gateway/src/allowlist/modules.rs` | DF4d, `2a002d7` (C2, `32d7bbf`) | None |
| `gateway/tests/modules.rs` | DF4a, `a99292e` | Uses `tests/common` (B): rewire to the new common helpers |
| `gateway/tests/network.rs` | DN7, `47ec5d2` | Same |
| `gateway/tests/nodelink.rs` | DN3, `f0b0586` | Same |
| `gateway/tests/home.rs` | DN5, `8308acc` | Same |
| `gateway/tests/launcher_region.rs` | DN7, `47ec5d2` | None |
| `gateway-plugin/Cargo.toml` | RD27, `0bf7226` | Licence line becomes MIT |
| `gateway-plugin/src/lib.rs` | RD27, `0bf7226` | Uses the public API in section 4; nothing else |
| `gateway-plugin/tests/library.rs` | RD27, `0bf7226` | None |
| `gateway-plugin/tests/parity.rs` | RD27, `0bf7226` | Includes `tests/parity/harness.rs` (B): point it at the rewritten harness |
| `gateway-plugin/tests/support/mod.rs` | RD27, `0bf7226` | None |
| `fuzz/fuzz_targets/gateway_modules.rs` | DF4a, `a99292e` | Moves to dainty-lass's `fuzz/` |
| `fuzz/fuzz_targets/gateway_network.rs` | DN7, `47ec5d2` | Same |

These may be copied as they are, with an SPDX MIT header added, provided nothing from a Category B file is pasted into
them while doing so. The implementer may read them (they are the API's first users).

### 2.2 Category B: from the DR / Reforged line, rewritten clean-room (40 files)

| File | Ported from (migration guide 2.3) |
| --- | --- |
| `gateway/Cargo.toml` | Root manifest; write a new one |
| `gateway/src/lib.rs` | Root |
| `gateway/src/cli.rs` | DR `main.rs` / `server.ts` (moved from `main.rs`, `52eaddd`) |
| `gateway/src/server.rs` | `server.ts` |
| `gateway/src/config.rs` | `config.ts` (DN3 node-link settings and DN7 node list added on top) |
| `gateway/src/policy.rs` | `policy.ts` (DN7's `NetworkRoute` added on top) |
| `gateway/src/gateway.rs` | `gateway.ts` (node-link requests, enroll, network routes, metrics added on top) |
| `gateway/src/guard.rs` | `gateway.ts` (Node's server timers) |
| `gateway/src/ip.rs` | `ip.ts` |
| `gateway/src/ratelimit.rs` | `ratelimit.ts` |
| `gateway/src/redact.rs` | `redact.ts` |
| `gateway/src/feed.rs` | `feed.ts` (in-process sink added by DN8) |
| `gateway/src/capture.rs` | DR `capture.ts` (DR M2, RD19) |
| `gateway/src/cert.rs` | `tools/make-cert.js` and the certificate checks of `server.ts` |
| `gateway/src/allowlist/mod.rs` | Root |
| `gateway/src/allowlist/cli.rs` | `allowlist/server.ts` (moved from `bin/allowlist.rs`) |
| `gateway/src/allowlist/config.rs` | `allowlist/config.ts` |
| `gateway/src/allowlist/ports.rs` | `allowlist/ports.ts` (DR's `CheckPorts`) |
| `gateway/src/allowlist/nft.rs` | `allowlist/nft.ts`, adapted from DR `deploy/unix-common/allowlist-core.mjs` |
| `gateway/src/allowlist/firewall.rs` | `allowlist/firewall.ts` |
| `gateway/src/allowlist/windows.rs` | DR's COM port of `firewall.ts` |
| `gateway/src/allowlist/helper.rs` | `allowlist/helper.ts` (in-process entry points added by DN8/DF4d) |
| `gateway/src/allowlist/state.rs` | `allowlist/state.ts` |
| `gateway/src/allowlist/sync.rs` | `allowlist/sync.ts` |
| `gateway/tests/common/mod.rs` | Root test helpers |
| `gateway/tests/gateway.rs` | Root |
| `gateway/tests/limits.rs` | Root |
| `gateway/tests/allowlist.rs` | Root |
| `gateway/tests/certificate.rs` | Root |
| `gateway/tests/parity.rs` | Root |
| `gateway/tests/parity/harness.rs` | Split out of the root `parity.rs` (RD27, `0bf7226`) |
| `gateway/tests/parity/record.cjs` | Root; Node, records from the Reforged Node gateway. Stays in restricted |
| `gateway/tests/parity/gateway.requests.json` | Root, data (section 5.1) |
| `gateway/tests/parity/gateway.expected.json` | Root, data recorded from Node (section 5.1) |
| `gateway/tests/fixtures/node-kit/*` (3 files) | Root, data: a certificate, key and invite the Node host kit made (section 5.1) |
| `fuzz/fuzz_targets/gateway_policy.rs` | DR X9 |
| `fuzz/fuzz_targets/gateway_capture.rs` | Root (RD19) |
| `fuzz/fuzz_targets/allowlist_input.rs` | Root (RX3) |

Counts: **A 17, B 40** (the node-kit row is three files). Of B, 23 are Rust sources to rewrite, 1 a
manifest, 7 Rust test files to rewrite, 3 fuzz targets to rewrite, and 5 data files plus `record.cjs` whose
treatment section 5.1 sets.

---

## 3. The seam: the gateway library's messages

The gateway and the allowlist run as one library the core loads (`services/gateway-plugin`, a cdylib). The messages
are dainty-lass's `crates/service-plugin/src/gateway.rs` (already MIT); the library's side is Category A. This is the
contract the rewritten crate must serve, through the public API of section 4.

Only control traffic crosses. Each message is a JSON object tagged by `type` (snake_case names) that refuses unknown
fields, nested in the service protocol as `{"type":"gateway","message":{...}}`. Byte fields travel as standard
base64. A body over 512 KiB goes first as `part` messages under its call's id, then the message that ends the call
carries the last piece; one call's parts add up to at most 1 MiB, at most 64 calls may have parts outstanding (a
65th id's parts are dropped), and a call over the limit is answered as too large on both sides.

### 3.1 Core to library (`ToGateway`)

| Message | Fields | What the library does |
| --- | --- | --- |
| `configure` | `registry` (bool), `allowlist` (mode), `ws_port` (number or null) | Sent before each start; remembers them and the outbox. `registry` true: routes come from attached modules, and the stand-in for the configured game is attached as a stand-in at start; false: the gateway runs its own stand-in only. `ws_port`: when the configured chat endpoint's port equals the backend's, the chat port becomes this |
| `placed` | `module`, `ports` (name to range), `running` | Records where a module's ports are; sent before its attach and again once it runs. The module routes and the allowlist plugin read endpoints and UDP ranges only from this |
| `stopped` | `module` | Marks it not running: its routes answer 503 `module_unavailable` |
| `admit_udp` | `ip`, `port` | Admits the address to the allowlist if the guard covers the port and the address passes the helper's checks; otherwise a warning and the port stays closed |
| `add_peer` / `remove_peer` | `node` | Adds or removes a node the node-link endpoint accepts calls from |
| `enroll` | `present` | Whether `POST /__node/enroll` is answered (by the core) |
| `part` | `id`, `data` | One part of a body under a call id |
| `node_answer` | `id`, `result` (bytes, or a failure: `too_large`, `no_service`, `refused{reason}`, `failed{reason}`) | The answer to a `node_call` |
| `enrolled` | `id`, `status`, `body` | The answer to an `enroll`; a body over the limit becomes 502 `{"error":"too_large"}` |
| `region_answer` | `id`, `accepted` | The answer to a `region_report` |

The allowlist mode is tagged by `mode`: `off` (no plugin: UDP ports are not guarded, private mode), `in_process`
with `env` (the helper runs in the library on these settings and takes the gateway's admits directly), `external`
with `base` (a helper process guards `base`; a module's UDP ranges must lie inside it), `absent` (a public server
without a helper: every module UDP port is refused). Bad `in_process` settings make the plugin answer as `absent` and
fail the start with the settings error.

### 3.2 Library to core (`FromGateway`)

| Message | Fields | When |
| --- | --- | --- |
| `node_links` | `identity` (node and region) | Once the gateway's node links are up |
| `part` | `id`, `data` | Body parts, as above |
| `node_call` | `id`, `from` (the caller its certificate named), `service`, `body` | A peer's call the node-link endpoint admitted |
| `enroll` | `id`, `body` | `POST /__node/enroll` while `present` |
| `region_report` | `id`, `api_key` (or null), `ticket` (or null), `regions` (`[{region, rtt_ms}]`) | `POST /network/v1/regions`, shape-checked already |

A call the core does not answer within 60 s is: `failed` ("the core did not answer") to the peer, 503
`{"error":"unavailable"}` to an enrollment, and a refused region report (401).

### 3.3 The library's run, in order

1. Its log lines go through `tracing` to the core (a library's statics are not the core's).
2. Without a prior `configure`, the start fails: "the core did not configure the gateway before its start".
3. Gateway settings from the env (front page, section 1); `ws_port` applied as above; capture settings.
4. With `in_process`, the allowlist helper starts first and must be ready within 60 s (so in public mode the game ports
   close before any game server listens); its failure fails the start, as "allowlist: ...". The gateway's feed then
   calls the helper directly.
5. With `registry`: node-link slot, region handler, stand-in attached as a stand-in for the configured game.
6. The gateway runs. It stops on the core's stop, or when the in-process allowlist ends under it (the start then fails
   with "allowlist: <reason>", "the allowlist stopped" when it ended cleanly). On the way out: the gateway first, then
   the helper (which closes the game ports).

The library's `serves` is routes and UDP ports; `attach` attaches the module's routes and then its UDP ranges, and
when the second refuses, detaches the first and refuses; `detach` detaches both and forgets the module's placement.

---

## 4. The public API restricted and the library need

The rewrite keeps these module paths and names: restricted's `services/host/src/compose.rs` and `lib.rs`,
`plugins/chat/tests/federation.rs`, `services/host/tests/*`, the Category A files and the fuzz targets use them.
Signatures may change shape only where every caller is updated in the same change. Everything not listed is private
to the crate.

| Path | Items |
| --- | --- |
| `gateway::now_ms` | Milliseconds since the epoch |
| `gateway::cli` | `main_from(args) -> ExitCode` (front page, section 16) |
| `gateway::config` | `Config` (`from_env(&Env)`, public fields at least `version`, `bind_host`, `port`, `tls`, `cert_file`, `key_file`, `secret`, `backend`, `content`, `ws`, `allowlist`, `limits`, `timeouts`, `network`, `node_links`), `Endpoint { host, port }` with `authority()`, `Allowlist`, `Limits`, `Rates`, `Timeouts`, `NodeLinkConfig { identity, cert_file, key_file, ca_file, peers }`, `DEFAULT_LIMITS`, `DEFAULT_TIMEOUTS`, `is_loopback_host`, `is_tailscale_ipv4`, `parse_loopback_url`, `check_secret`, `parse_game` |
| `gateway::server` | `load_tls`, `load_capture(&Env)`, `run`, `run_with_context`, `run_with_routes`, `run_with_node_links`, `run_with_network(config, capture, in_process_sink, ctx, routes, node_link_slot, region_handler)`, `NodeLinkSlot` |
| `gateway::gateway` | `Gateway` (`new`, `with_routes`, `serve(listener, stop)`, `set_region_handler`, `node_links`, `certificate`, `config`, `stats`, `routes`), `Tls { config, info }`, `Stats`, `forward_headers` |
| `gateway::cert` | `CertInfo { fingerprint, not_before, not_after, subject_alt_name }`, `Loaded`, `load`, `describe`, `server_config`, `load_or_generate`, `make_cert_main`, `parse_make_cert_args`, `is_dns_name`, `iso` |
| `gateway::policy` | `Upstream`, `LimitClass`, `FeedKind`, `NetworkRoute`, `Decision`, `PUBLIC_REFORGED_API`, `HOST_ONLY_PREFIXES`, `GAMESERVER_KEY_HEADER`, `GAMESERVER_KEY_HEADERS`, `HOST_ONLY_HEADERS`, `split_target`, `check_path`, `canonical_path`, `unwrap_origin` with `Unwrapped { path, origin }`, `under`, `classify`, `classify_for`, `has_bearer` |
| `gateway::modules` (A) | As it is: `RouteGroup`, `NODE_LINKS`, `GROUPS`, `RESERVED_PREFIXES`, `RegistryView`, `ManifestDefaults`, `stand_in`, `Routed`, `ModuleRoutes` |
| `gateway::network` (A) | As it is |
| `gateway::nodelink` (A) | As it is |
| `gateway::feed` | `AllowlistFeed` (`new`, `with_clock`, `from_in_process`, `with_in_process`, `report`, `idle`, `sent`), `Clock`, `InProcessSink` |
| `gateway::capture` | `CaptureOptions`, `load_capture_options`, `CaptureWriter` (`new`, `add_secret`, `write`, `prune`, public `options`), `redact_capture_text`, `redact_capture_json`, `redact_capture_headers`, `redact_capture_body`, `redact_chat_frame`, `redact_capture_target`, `decode_base64_lenient`, `FrameReader`, `ChatMessage`, `BodyTap`, `Capture`, `ChatTap`, `Direction` |
| `gateway::ip` | `IpClass`, `ParsedIp { version, canonical, class, bytes }`, `parse_ip`, `format_v6`, `peer_address`, `rate_limit_key`, `AllowlistCheck`, `check_allowlist_ip` |
| `gateway::ratelimit` | `BucketSpec`, `Take`, `RateLimiter`, `parse_bucket_spec` |
| `gateway::redact` | `redact_text`, `redact_target`, `redact_user_agent` |
| `gateway::guard` | `Guard`, `Phases` |
| `gateway::allowlist::cli` | `main_from(args) -> ExitCode` |
| `gateway::allowlist::config` | `AllowlistConfig` (public fields, `from_env`), `FirewallKind`, `check_rule_name`, re-export of `check_ports` |
| `gateway::allowlist::ports` | `check_ports`, `check_port_list`, `MAX_RANGES` |
| `gateway::allowlist::nft` | `NFT_TABLE`, `NftSettings`, `check_nft_table`, `check_interface`, `check_nft_binary`, `default_nft`, `nft_script`, `Feed`, `Nft`, `private_script`, `can_manage_nft` |
| `gateway::allowlist::firewall` | `RULE_NAME`, `RULE_DISPLAY_NAME`, `RULE_GROUPING`, `RULE_DESCRIPTION`, `RulePlan`, `plan`, `WindowsRule`, `WindowsChange`, `windows_change`, `Backend` |
| `gateway::allowlist::windows` (Windows only) | `apply_rule`, `read_rules`, `remove_rules`, `is_elevated` |
| `gateway::allowlist::helper` | `AuditLog`, `applier`, `applier_for`, `Helper` (`new`, `with_guard`, `start`, `close`, `list`, `status`, `expire_now`, `guard`, `sync`, `allow_ip`, `admit_udp`, `is_admitted`), `Running`, `close_ports_once`, `run_with_context`, `run_with_guard`, `declared_length`, `allow_body` |
| `gateway::allowlist::state` | `AllowlistState`, `Refresh` |
| `gateway::allowlist::sync` | `FirewallSync`, `Apply`, `OnResult`, `ApplyResult` |
| `gateway::allowlist::guard` (A), `::modules` (A) | As they are |

---

## 5. Tests that define the behaviour

### 5.1 Black-box vectors the implementer may reuse

These are data, not code. Reusing them verbatim is the recommendation; the maintainer confirms it at the cutover, as
for the content server's parity fixtures.

- `tests/parity/gateway.requests.json` and `gateway.expected.json`: 57 raw HTTP requests (each on its own TLS
  connection, the env `GATEWAY_MAX_BODY_BYTES=4096`, `GATEWAY_RATE_TOKEN=2,1`; a request may name its `local` source
  address and, for an upgrade, a text frame to `send` after the 101) and, for each, the status, the headers in the
  harness's list (`content-type`, `content-length`, `transfer-encoding`, `cache-control`, `connection`,
  `retry-after`, `x-powered-by`, `x-reforged-gateway`, `set-cookie`, `upgrade`, `sec-websocket-accept`,
  `sec-websocket-protocol`), the body and the access-log lines Node gave, minus the `ms` field. A 58th entry,
  `allowlist feed`, lists the addresses the helper was sent, sorted (`127.0.0.51`, `.52`, `.54`). The new harness must
  provide the same fake upstreams: a backend and content server that echo `{upstream, method, url, headers (without
  connection), bodyLength, bodySha}`, routes `/status/<n>` answering `status <n>` as chunked `text/plain` with two
  `set-cookie` headers, `POST /heartbeat` answering `20000` as `text/plain`, a `/slow` and `/hang`, a WebSocket
  upstream that answers `//` with 101 and echoes frames as `echo:<text>` and refuses `/refuse` with 404 `no room`,
  and a helper that records `POST /allow`. The local address of a request may be any `127.0.0.x` (the fixtures use
  `.51` and `.52` for the feed cases), which the gateway sees as the peer.
- `tests/fixtures/node-kit/`: a certificate, key and invite the Node host kit made. The gateway must load them, and
  the fingerprint it serves must equal the invite's `fp`.
- `record.cjs` stays in restricted; it is how the fixtures were made and is not needed to run them.

### 5.2 Behaviour the rewritten tests must cover (Category B tests, rewritten from this list)

The Category B test files are rewritten; these are the cases they hold, by file. Front-page and allowlist-page
section numbers say where the rule is.

- `gateway.rs`: a pinning client connects and sees the fingerprint; plain HTTP to the TLS port gets nothing usable;
  content and backend routing with path and query untouched; streamed bodies with hop-by-hop headers dropped; the
  forwarded address and secret set whatever the client claims; the game-server key header refused and never
  forwarded; only the public `/reforged` routes reach the backend; the feed reports only 2xx token and bearer
  heartbeats; one access line per request with no secret; `x-reforged-user-api-key` forwarded byte for byte and
  never logged; WebSocket upgrades proxied byte for byte, including the game's `//` chat target with
  `Sec-WebSocket-Protocol: xmpp`; refused upgrades passed on; 502 when an upstream is down; private mode serves plain
  HTTP with the same rules; `/__origin` and host-only routes end to end; the capture records requests, answers and
  chat redacted.
- `limits.rs`: the body cap by `Content-Length` (413, `Connection: close`, upstream untouched), exactly the cap
  passes, a chunked body cut at the cap (413 and the upstream request aborted, an access line with `bytesIn` over
  the cap), `Expect: 100-continue` (413 without a 100 over the cap; 100 then the answer under it, `expect` not
  forwarded); each rate bucket with its `retry-after` (general 1 s refill, register 300, token 60) and the ws bucket
  apart from the general one; connections per address and the new-connection rate (8 of 10 at `8,60`); the timeouts
  (slow head: 408 then close in about the headers time; slow body: closed near the request time, upstream aborted;
  unfinished handshake dropped; idle keep-alive closed near the keep-alive time); graceful shutdown (an in-flight
  request finishes with `Connection: close`, new connections refused, a hanging one cut after the grace).
- `allowlist.rs`: the helper over HTTP (every status of allowlist section 4), private addresses only when allowed,
  stop closes and saves and start restores, the nft script and the Windows rule after a login and a logout (dry
  run), a logged-in player's ports opened through the gateway and the helper, the dry-run setting required,
  `--close-ports` for both firewalls, the real firewall must be this platform's, the gateway refusing bad settings and
  a key that is not the certificate's, the helper and gateway as processes, a module's ports joining and leaving the
  rule.
- `certificate.rs`: section 5.1's node-kit vectors; `make-cert` reads a fingerprint and never replaces a certificate
  without `--force`.
- Unit cases: every example in the front and allowlist pages is a test case (the inputs and outputs in their tables).

### 5.3 Category A tests, moved as they are

`tests/modules.rs`, `network.rs`, `nodelink.rs`, `home.rs`, `launcher_region.rs`, and gateway-plugin's
`tests/library.rs`, `parity.rs`, `support/mod.rs`. They need the rewritten `tests/common` to offer what they call:
a test certificate (`TestCert` with its PEM files and fingerprint), a test config builder, a gateway starter that
returns its port and a stopper, HTTPS and raw-TLS clients with a chosen local address, log capture with
`log_len`/`log_since`/`log_lines`, `serial()` for tests that read the shared log, and the fake upstreams of 5.1.

### 5.4 Fuzz targets

| Target | Class | Invariant the rewrite must keep |
| --- | --- | --- |
| `gateway_policy` | B, rewrite | Any method, target and headers give a decision without panicking. A target that is not origin-form is a 400. Whatever is proxied: the path passed `check_path`, its unwrapping starts with `/`, the canonical path starts with `/`, it carries no host-only header, it is under no host-only prefix, under `/reforged` it is one of the four public routes with an allowed method, `Upstream::Content` only for an unwrapped `/content` path, feeds only `POST` heartbeat and token |
| `gateway_capture` | B, rewrite | Body, header, target and chat-frame redaction never panic; no account key and no whole token from the input survives in any output; the chat frame reader gives the same messages however the bytes are split |
| `allowlist_input` | B, rewrite | The `/allow` body and `Content-Length`, the state file, and the settings that reach a script never panic; an accepted address is exactly one script element (the script has 3 table lines, 2 sets of 3 lines, 8 chain lines, plus one `elements` line per non-empty set), one Windows remote address each; an accepted port text, table or interface name is one token; a guard attach either succeeds with the text grown or leaves it unchanged |
| `gateway_modules` | A, move | As it is |
| `gateway_network` | A, move | As it is |

### 5.5 The test that read AGPL source

restricted's policy unit test reads the backend's route file
(`modules/rustic-isles/reference/1.14.7/ParadoxBackend/src/routes/reforgedApi.ts`) to list every `/reforged/api`
route. dainty-lass cannot read it. In the MIT crate the test uses this fixed table of the backend's routes:
`POST Register`, `GET GetUserInfo`, `GET ServerStatus`, `GET RegistrationStatus`, `POST CreateInvite`,
`GET InviteCodes`, `DELETE InviteCode/:code`, `POST SetAccountStatus`, `POST Kick` (all under `/reforged/api/`, the
parameter filled with `ABCD-EFGH-JKMN`). Exactly the first four pass; the rest are 403 `admin_route`. restricted keeps
a test of its own that reads the file and checks it against the MIT crate's `PUBLIC_REFORGED_API`, so a route added
to the backend still fails a build.

---

## 6. Target MIT design

- **Where.** A new crate `services/gateway` in dainty-lass (package `gateway`, `license = "MIT"`, SPDX MIT headers,
  `Copyright (c) 2026 Silent Spud`), a library with the module tree of section 4. The Category A files go in at their
  paths. `services/gateway-plugin` moves to dainty-lass beside it (it then depends on MIT crates only). The three
  rewritten and two moved fuzz targets go to dainty-lass's `fuzz/`. The security checklists `gateway.md`,
  `allowlist.md` and `region-ping.md` move with the code to dainty-lass's `docs/security/` (the integration pass
  indexes them).
- **Dependencies** the crate needs that dainty-lass's workspace may not have yet: `rcgen` (RSA, PEM, aws-lc-rs),
  `x509-parser`, `tokio-rustls` (aws-lc-rs, TLS 1.2), `indexmap`, `regex`, `tempfile`, `time`, `sha2`, `url`,
  `windows` (firewall, COM, OLE, variant, security, threading features) on Windows; tests `axum`, `base64`, `sha1`.
  Adding them and the two workspace members is a seam the integration pass owns (AGENTS.md): the row reports the
  list, it does not edit the workspace `Cargo.toml`.
- **`unsafe_code`.** The Windows Firewall module needs COM calls. It is the only module that allows `unsafe_code`,
  with a one-line reason, as in restricted.
- **restricted after the move.** `services/gateway` and `services/gateway-plugin` are deleted there; `host`, the chat
  plugin's tests and anything else take `gateway` (and the library, for its tests) from dainty-lass as a git
  dependency at the workspace rev. `compose.rs`, `lib.rs` (`gateway_service_main`, the `allowlist` subcommand,
  `ensure_gateway_cert`) change only their imports. The policy test of section 5.5 stays in restricted.
- **What could follow (not in this row).** With the gateway MIT, the MIT core could link it instead of loading it;
  `ensure_gateway_cert` could use `hostkit::tls::write_gateway_cert_if_missing`, which already makes the same kind of
  certificate. Both are maintainer decisions.
- **Provenance record.** A `docs/port/clean-room/gateway.md` like `clean-room/hostkit.md`: the spec commit, the
  implementer's worktree with a sparse checkout that excludes restricted entirely (dainty-lass has no AGPL tree to
  exclude), the files each implementer session read and wrote, and the similarity measurement of `similarity.md` run
  against restricted's `services/gateway` at the spec's commit (pairs file to file).

## 7. The riskiest parts

1. **The route policy** (front page, section 4): percent-decoding, case folding, slash folding, the `/__origin`
   unwrapping and the host-only headers decide what reaches loopback. A lenient bug here exposes the admin API. The
   parity fixtures and `gateway_policy` catch most; the spelling-trick table must be a unit test.
2. **The connection timers** (front, section 3): Node's header, keep-alive and idle timers and the raw 408 are kept by
   a stream wrapper, not by hyper. They are easy to get subtly wrong (the 408 on a new silent connection but not on an
   idle keep-alive one, the one-second linger), and `limits.rs` checks them only by timing.
3. **The nftables script and the privileged process** (allowlist, sections 9 and 10): the exact script, one
   transaction, nft at a fixed path with no shell, re-checking every value at the script. A mistake opens the game
   ports to everyone or runs something as root.
4. **The capture's redaction** (front, section 12): placeholders the replay needs (`<token:...>`, `<key:...>`,
   `<plain:...>`) beside secrets that must never be written. Personal data.
5. **Node links on the public listener** (front, section 9): the ALPN split decides which certificate a client sees and
   whether its requests bypass the policy. A wrong split lets a player's connection reach `/__node`, or a node's reach
   the game.
6. **Byte-level parity**: header order and casing, `content-length` versus chunked, `connection: close` rules,
   error bodies. The fixtures were recorded from Node; hyper differs from Node in ways the original code worked
   around (front, section 13).
7. **A known weakness kept on purpose**: any `Authorization: Bearer <anything>` on `POST /heartbeat` opens the game
   ports for its sender, because the backend does not check it (front, section 10). The rewrite must not "fix" it
   silently: it is the Node behaviour and is tracked in the checklist.
8. **Windows Firewall through COM** with `unsafe` code, testable only on an elevated Windows machine (the maintainer's
   live check).
