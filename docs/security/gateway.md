<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: gateway policy and TLS (RX3)

The checklist for the gateway, the `gateway` binary of `crates/gateway` (MIGRATION_PLAN.md RS3, RD10, RD19,
RD20). `SECURITY_REVIEW.md` indexes it. The allowlist helper, the crate's other service, has its own list in
[allowlist.md](allowlist.md).

How to use it: before the gateway's cutover, copy this list into the cutover PR, check each item in the code (not
from memory), link the code or test that shows it, and sign at the end with the date and the commit reviewed. An
item that does not hold is a bug to fix before the cutover, not a box to leave open.

## What it is and where the boundary is

The gateway is the server's only public port. Everything a player's game, runtime and launcher send comes through
it; everything behind it (the Rustic Isles backend, its chat listener, the content server, the allowlist helper)
listens on loopback and trusts that the gateway already refused what must not reach it.

- **In, public mode:** TLS on `GATEWAY_BIND:GATEWAY_PORT` (default `0.0.0.0:443`) with the server's self-signed
  certificate. The launcher pins its SHA-256 fingerprint, the `fp` of every v2 invite, so the certificate is loaded
  and never made at start.
- **In, private mode (`GATEWAY_TLS=0`):** plain HTTP, default port 61000, only on a Tailscale address
  (100.64.0.0/10) or loopback. Tailscale encrypts the traffic; the gateway refuses to start on any other address.
- **Out:** plain HTTP to the backend (`GATEWAY_BACKEND_URL`), the content server (`GATEWAY_CONTENT_URL`) and the chat
  listener (`GATEWAY_WS_URL`), all on loopback only, each request carrying the shared secret `x-dainty-gateway`;
  `POST /allow` to the allowlist helper (`ALLOWLIST_URL`, also loopback) with `x-allowlist-secret`. The backend is
  the game module's route (DF4a): a gateway started alone fronts a stand-in for `GATEWAY_GAME` at
  `GATEWAY_BACKEND_URL`, one under the controller the modules it attaches, each at the loopback port the registry
  gave it.
- **Files:** the certificate and key (read only), the `.env` the host kit writes (trusted), and when the capture is
  on, the capture folder (`CAPTURE_DIR`, written).

Code: `crates/gateway/src/` (`policy.rs`, `modules.rs`, `gateway.rs`, `guard.rs`, `ip.rs`, `ratelimit.rs`, `redact.rs`,
`feed.rs`, `capture.rs`, `cert.rs`, `config.rs`, `server.rs`, `main.rs`), and as a service of the core, compiled in
or as the library `crates/gateway-plugin` exports, `service.rs` with dainty-lass's
`crates/service-plugin/src/gateway.rs` and `crates/controller/src/service_library/gateway.rs` (RD27).

Fuzz targets in `fuzz`:

- `gateway_policy`: a method, target and headers from the network give a decision, never a panic, and nothing
  proxied carries a host-only header or reaches a host-only prefix or a non-public `/reforged` route, whatever the
  spelling or the `/__origin` wrapping.
- `gateway_modules`: a module's routes of any shape attach or are refused, never panic, and no request reaches a
  module through one of its host-only routes, the node-link group or a host-only prefix, or anything off loopback.
- `gateway_capture`: the capture's redaction of bodies, headers, targets and chat frames never panics and never
  writes an account key or a whole token from its input; the chat frame reader gives the same messages however the
  bytes are split.

## Checklist

### TLS and the certificate

- [ ] Public mode loads `GATEWAY_CERT` and `GATEWAY_KEY` and never makes a certificate (`server.rs`, `load_tls`).
      The start stops if the key does not belong to the certificate (`cert::load`) or the certificate is not valid
      now; it warns within 30 days of expiry.
- [ ] A certificate made by the Node `tools/make-cert.js` loads, and the fingerprint the Node host kit put in an
      invite is the one the gateway serves (`tests/certificate.rs`, fixture in `tests/fixtures/node-kit`).
- [ ] `gateway make-cert` refuses to replace an existing certificate or key without `--force`, and says that a new
      one breaks every invite (`cert.rs`, `make_cert_main`). The key is written 0600 where the file system has
      modes. On Windows the folder's ACL is left to the host kit, which sets none today (`crates/hostkit`).
- [ ] TLS 1.2 and 1.3 only, aws-lc-rs, ALPN `http/1.1` only (`cert::server_config`). The handshake has its own
      timeout (`GATEWAY_HANDSHAKE_TIMEOUT_MS`, 10 s) and counts against the per-address connection rate.
- [ ] A public config without a certificate, or a private one with one, does not start (`Gateway::new`).
- [ ] Private mode binds only a Tailscale IPv4 address or loopback (`config.rs`, `is_tailscale_ipv4`); `0.0.0.0`,
      `::`, LAN and public addresses stop the start. `x-forwarded-proto` is then `http`.

### What reaches the backend

- [ ] The request target must be origin-form; absolute-form and `*` are 400 (`split_target`).
- [ ] `check_path` refuses backslashes, control characters, white space (Unicode spaces included), encoded `/`, `\`,
      NUL and `.`, and dot segments, before anything else looks at the path.
- [ ] `/__origin/<host><rest>` is unwrapped once, as the backend does (`unwrap_origin`): the host is
      percent-decoded, lower-cased and must be `[a-z0-9.-]{1,253}` without a leading or trailing dot or `..`; a bare
      `/__origin`, an undecodable or odd host, or a second `/__origin` inside is 400 `bad_origin`. Every rule below
      applies to `<rest>`.
- [ ] Paths are compared the way Express matches routes, and more leniently: percent-decoded, lower case, repeated
      slashes folded, trailing slashes dropped (`canonical_path`).
- [ ] Under `/reforged`, only `POST register`, `GET`/`HEAD getuserinfo`, `serverstatus` and `registrationstatus` under
      `/dainty/api/` pass; every other route (invites, account status) is 403 `admin_route`, even with a valid admin
      key and even as an upgrade. The test reads every route from `modules/rustic-isles/reference/1.14.7/ParadoxBackend/src/routes/reforgedApi.ts`, so a
      route added there is covered (`policy.rs` and `tests/gateway.rs`, `only_the_public_reforged_routes_*`).
- [ ] `/admin`, `/launcher`, `/internal` and `/health`, and everything under them, are 403 `host_only_route`
      (Rustic Isles's admin API, its own launcher API, metrics and readiness probes).
- [ ] A request with `x-rustic-isles-gameserver-apikey`, `x-rustic-isles-gameserver-key`, `x-dainty-admin-key` or
      `x-rustic-isles-metrics-token` is 403, whatever its value, letter case or route. So is each one's
      `x-mysticparadox-` form: the backend still reads those for one release after the Rustic Isles rename (DX4),
      so a player must not reach it with either. The admin key's name before the 2026-11-06 rename,
      `x-reforged-admin-key`, is refused too (`HOST_ONLY_HEADERS`, `GAMESERVER_KEY_HEADERS`;
      `refuses_the_admin_key_and_the_metrics_token_headers`, `the_game_server_key_header_is_always_refused`,
      `tests/gateway.rs`). The capture shows any of the game server key's names as `<gskey>` and the rest redacted.
- [ ] `/content` goes to the content server only when it was not behind `/__origin`.
- [ ] Only GET, HEAD, POST, PUT, DELETE, PATCH and OPTIONS pass (405 otherwise). An upgrade must be a `GET` with
      `Upgrade: websocket`.
- [ ] Client claims are dropped and the gateway's own set: `x-forwarded-for` is the real peer, `x-forwarded-proto`
      the mode, `x-dainty-gateway` the secret (a client's `x-reforged-gateway`, the old name, is dropped too);
      `forwarded`, `x-real-ip`, `cf-connecting-ip` and the like never
      cross; hop-by-hop headers and the headers `Connection` names never cross either way. Answers lose
      `x-powered-by` and `x-dainty-gateway` (`forward_headers`, `response_headers`).
- [ ] Upstream URLs must be `http://` on loopback with no path, query or credentials (`parse_loopback_url`), so the
      secret never leaves the machine.
- [ ] Parity: every reply in `tests/parity/gateway.requests.json` (status, headers, body, access log, allowlist feed)
      is the one Reforged's Node gateway gives (`tests/parity.rs`, recorded by `record.cjs`).

### Module routes (DF4a)

The gateway is the core plugin that serves module routes (`modules.rs`, `ModuleRoutes`, a `CorePlugin`). A request
goes to the game module its game version names (`ModuleManifest::answers_invite`; today `GATEWAY_GAME`'s version),
then to the loopback port of the longest public route that module registered. The policy above judges every request
first, and content and chat stay the gateway's own upstreams, so a module's routes never widen what reaches loopback.

- [ ] With the stand-in a gateway started alone attaches, every request goes where it went before, and the parity
      tests pass unchanged (`tests/parity.rs`, `modules.rs` `a_standalone_gateway_*`).
- [ ] A module's host-only route is 403 `host_only_route` from outside, under any spelling, and wins over a longer
      public route below it (`host_only_wins_over_a_longer_public_route`, `tests/modules.rs`). The host's tools reach
      the module on loopback, never through the gateway.
- [ ] The node-link group `/__node` (DN3's mutual-TLS links between nodes) is host-only and a stub: every request
      under it, wrapped in `/__origin` or not, upgrades included, is 403 `node_link` (`NODE_LINKS`).
- [ ] `attach` reruns `Registration::check` (an external module's registration is untrusted) and refuses a plugin's
      routes, a route under `/content`, `/__origin` or `/__node`, a public route under a host-only prefix, a prefix
      given twice in any case, and a module whose version or alias another attached module already answers (DD4).
- [ ] A path with no route is 404 `no_route`, a version with no module 503 `no_module`, a module the registry has no
      port for 503 `module_unavailable`. An endpoint the registry gives off loopback is 502 `upstream_not_loopback`,
      so the secret header never leaves the machine whatever the registry says.
- [ ] The access log's `route` is still the policy's (`backend`, `content`, `ws`), not the module's.

### The library's boundary (RD27)

The gateway and the allowlist run as one service of the core (`gateway::service`, RD27): compiled in, or as the
library `crates/gateway-plugin` exports (a cdylib, which `plugin-host` loads only at the SHA-256 `host.toml` pins;
dainty-lass's `docs/security/plugin-host.md`), through `controller::service_library::gateway` either way. Compiled in,
the same messages cross in-memory streams, and the service has a runtime of its own as a library's instance does. The library is the gateway: the listener, TLS and the
certificate, the policy, the proxying to backends, the node-link endpoint's admission and the capture all stay in
it, and every item above holds there unchanged. Only control traffic crosses, as `service_plugin::gateway` messages
in DD5's frames, and the library is in the core's process, so what crosses is a question of shape, not of trust.

- [ ] The data plane never crosses: no request, answer or header of a player goes to the core. The parity tests pass
      against the service compiled in and loaded as a library (`crates/gateway-plugin/tests/parity.rs`, the same
      harness as `tests/parity.rs`).
- [ ] A peer's call reaches the core (`NodeCall`) only after the library's endpoint admitted it: a listed or added
      peer, by its verified certificate, on a node link's route and method, the clocks agreeing and the body within
      `MAX_MESSAGE`. The caller sent is the one the certificate named; the core's answer over `MAX_MESSAGE` is
      refused by the endpoint as before (`tests/library.rs`).
- [ ] A body crosses as base64 parts of 512 KiB under its call's id; one call's parts add up to 1 MiB at most and 64
      calls at most have parts outstanding (`service_plugin::gateway::Parts`). A call over is answered `too_large`,
      on both sides; a core that does not answer within 60 s is `failed` (502) to the peer, `unavailable` (503) to an
      enrollment, and a refused region report (401).
- [ ] `POST /__node/enroll` is answered by the core's handler only while the core has one (`Enroll { present }`);
      without, the request goes through the policy and is refused as before. Region reports are shape-checked
      (`parse_region_report`) in the library before they cross.
- [ ] A module's endpoint comes only from the ports the core says the controller placed (`Placed`, before each
      attach, then `running` once the module started), always on 127.0.0.1; a module not running is 503
      `module_unavailable` as before.
- [ ] Every message is a tagged enum that refuses unknown fields, and only the gateway library's extension takes
      gateway messages: another library sending one breaks the protocol and is closed.
- [ ] The library's log lines reach the core's log as frames after the library's own redaction (`linelog` to
      `tracing` to `FromService::Log`); no secret is added to a message for it. The env the core sends at start is
      the env file the host kit wrote, which the core read already.

### Limits

- [ ] Bodies are capped at `GATEWAY_MAX_BODY_BYTES` (128 KiB): refused by `Content-Length` before reading and cut
      while streaming (413). A refused request's body is never read; its connection is closed instead.
- [ ] Headers: 16 KiB and 100 at most (431 / 400), read within `GATEWAY_HEADERS_TIMEOUT_MS`; a request must arrive
      within `GATEWAY_REQUEST_TIMEOUT_MS`; idle and keep-alive timeouts end slow clients (`guard.rs`).
- [ ] Connections: `GATEWAY_MAX_CONNECTIONS` in all, `GATEWAY_MAX_CONNECTIONS_PER_IP` per address (IPv6 by /64), a
      new-connection rate per address. Per-address token buckets for general, content, register, token and WebSocket
      requests; 429 with `retry-after` (`ratelimit.rs`, `tests/limits.rs`).
- [ ] Every warning about a client is throttled to once a minute per kind and address, so a client cannot flood the
      log.

### The allowlist feed

- [ ] Only a 2xx answer to `POST /account/api/oauth/token`, or to `POST /heartbeat` that carried a bearer token,
      reports the peer's address (`policy.rs`, `gateway.rs`). Nothing else does; in particular not `/game/login`
      (Dauntless Revived's route, which Paradox does not have) nor Paradox's `POST /login` (RD20).
- [ ] Known weakness, the same as the Node gateway: Paradox answers `POST /heartbeat` 200 without checking the token
      (`modules/rustic-isles/reference/1.14.7/ParadoxBackend/src/routes/system.ts`), so the gateway's `has_bearer` is the only check, and any
      `Authorization: Bearer <anything>` opens the game ports for its sender. The token route does check credentials.
      Closing this needs the backend to check the bearer on `/heartbeat` (a Paradox fix, RD2) or the gateway to feed
      from a route that does.
- [ ] The feed never delays a player's request and backs off from a helper that is down (3 failures in a row, then
      2 s doubling to 60 s; `feed.rs`).
- [ ] The helper's secret is never logged.

### The access log

- [ ] One line per request with the address, method, target, route, status and sizes; tokens, keys and invite codes
      in the target are replaced and the user agent is cut (`redact.rs`). No header names or values are logged.

### The traffic capture (RD19)

- [ ] Off unless `CAPTURE_DIR` is set; when on, the start logs a warning that says so.
- [ ] Bearer tokens become `<token:subject>` (read without the signature), account keys (`RRK_`, DR's `UUK_`) become
      `<key:sha256>`, the game-server key `<gskey>`, and every field or header whose name looks secret (`key`,
      `token`, `secret`, `pass`, `auth`, `jwt`, `sig`, `cred`, `recovery`, `invite`, `cookie`) `<redacted>`; hex runs
      of 40 or more and other long tokens are cut; cookies and `x-dainty-gateway` are dropped (`capture.rs`).
- [ ] The gateway secret and the helper's secret are cut from every line, wherever an answer echoes them.
- [ ] Chat: a SASL PLAIN login keeps only its account (`<plain:account>`), `<password>` elements are cut, the rest is
      redacted as text. Compressed or oversized messages are kept by size only.
- [ ] Bodies over `CAPTURE_MAX_BODY_BYTES` (256 KiB) and binary bodies are kept by size only.
- [ ] Files are one per UTC day, created 0600 in a 0700 folder where the file system has modes, and removed after
      `CAPTURE_RETENTION_DAYS` (14). On Windows the folder inherits its parent's ACL: put it where only the host's
      account can read.
- [ ] A write error stops the capture with one warning; the gateway keeps serving.
- [ ] What is written is still personal data (account ids, addresses, play history): the capture runs only for RB2 on
      the canary, with the players told.

- [ ] `GATEWAY_GAME=legacy` (Dauntless Revived's 1.4.4, served until 2026-10-31) stops the start; any value but
      `reborn` does too (`config.rs`, `parse_game`).

## Sign-off

Reviewed at commit `________` on `________` by `________`.
