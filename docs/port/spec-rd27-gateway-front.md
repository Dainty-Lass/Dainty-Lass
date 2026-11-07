<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false, "code_blocks": false } } -->

# Clean-room behaviour specification: the gateway (RD27)

Part of [`spec-rd27-gateway.md`](spec-rd27-gateway.md), which has the classification, the public API names, the
seam and the tests. This page is the gateway's behaviour: everything a player, the launcher, a peer node, an
upstream or the host can observe. The allowlist helper is
[`spec-rd27-gateway-allowlist.md`](spec-rd27-gateway-allowlist.md).

Quoted text in `code` is exact: error strings, log messages, JSON field names and bodies. JSON bodies are compact
(no spaces) unless shown otherwise. "Node reads it as a number" means `service_kit::js::number` (JavaScript's
`Number(text)`), which the MIT `service-kit` already provides.

---

## 1. Settings

Read once at start from the env file (`service_kit::env::Env`; the process environment wins over the file). A bad
setting stops the start; the CLI prints it as `Configuration: <text>` (section 16). "Optional" values are trimmed and
an empty value counts as unset. Whole numbers are read as Node reads a number and must be whole and within their
range, else `<NAME> must be a whole number from <min> to <max>`.

| Setting | Default | Rule and error text |
| --- | --- | --- |
| `GATEWAY_GAME` | `reborn` | Trimmed, case-insensitive. Unset, empty or `reborn`: the 1.14.7 version (`protocol::invite::reforged_version()`). `legacy`: `GATEWAY_GAME=legacy (Dauntless Revived's 1.4.4) is no longer served; this gateway serves reborn only`. Anything else: `GATEWAY_GAME must be reborn (got <the raw value as a JSON string>)` |
| `GATEWAY_BIND` | `0.0.0.0` | Must parse as an IP address, else `GATEWAY_BIND must be an IP address (got <JSON string>)` |
| `GATEWAY_TLS` | public | Exactly `0` is private mode; any other value (even `false`) is public mode |
| (private mode) | | The bind must be a Tailscale IPv4 address (100.64.0.0/10: first octet 100, second 64 to 127) or loopback (`localhost`, an IPv4 starting `127.`, `::1`, `[::1]`), else `GATEWAY_TLS=0 (private mode) listens only on this PC's Tailscale address (100.64.0.0/10) or on loopback, not on <bind>`. An IPv4-mapped IPv6 address is not Tailscale |
| `GATEWAY_CERT`, `GATEWAY_KEY` | | Public mode needs both: `GATEWAY_CERT and GATEWAY_KEY must name the PEM certificate and key (make them with \`gateway make-cert\`), or set GATEWAY_TLS=0 for a private server over Tailscale`. Made absolute against the working folder. Private mode ignores them (empty paths) |
| `GATEWAY_PORT` | 443 public, 61000 private | Whole number 1 to 65535 |
| `GATEWAY_SECRET` | | Required. See "secrets" below |
| `GATEWAY_CONTENT_URL` | `http://127.0.0.1:61002` | A loopback URL, below |
| `GATEWAY_BACKEND_URL` | `http://127.0.0.1:61000` | Same |
| `GATEWAY_WS_URL` | `http://127.0.0.1:61000` | Same (the backend serves chat on its own listener) |
| `GATEWAY_ALLOWLIST` | on | Exactly `0` turns the feed off; then the four allowlist settings below are not read |
| `ALLOWLIST_URL` | `http://127.0.0.1:61005` | A loopback URL; kept as `http://<authority>` |
| `ALLOWLIST_SECRET` | | Required when the feed is on: `ALLOWLIST_SECRET is not set (the allowlist helper's secret; GATEWAY_ALLOWLIST=0 runs without the feed, and then no game ports open for anyone)`. Then the secret rule |
| `GATEWAY_ALLOWLIST_REFRESH_SECONDS` | 60 | Whole number 5 to 540 |
| `ALLOWLIST_ADDITIONAL_URLS` | none | Comma-separated, each trimmed, empties dropped; more than four: `At most four additional allowlist helpers are supported`; each a loopback URL (named `ALLOWLIST_ADDITIONAL_URLS` in its error) |
| (feed timeout) | 3 s | Fixed |
| `GATEWAY_MAX_BODY_BYTES` | 131072 | Whole number 1024 to 67108864 |
| `GATEWAY_MAX_CONNECTIONS` | 2048 | 1 to 100000 |
| `GATEWAY_MAX_CONNECTIONS_PER_IP` | 128 | 1 to 100000 |
| (max headers) | 100 | Fixed |
| `GATEWAY_RATE_GENERAL` | `300,180` | A bucket spec (section 11) |
| `GATEWAY_RATE_CONTENT` | `600,600` | Same |
| `GATEWAY_RATE_REGISTER` | `5,0.2` | Same |
| `GATEWAY_RATE_TOKEN` | `10,1` | Same |
| `GATEWAY_RATE_WS` | `20,12` | Same (the game reconnects chat at most every 15-45 s after a failure) |
| `GATEWAY_RATE_CONNECT` | `200,300` | Same (new TCP connections) |
| `GATEWAY_HANDSHAKE_TIMEOUT_MS` | 10000 | 1000 to 120000 |
| `GATEWAY_HEADERS_TIMEOUT_MS` | 10000 | 1000 to 120000 |
| `GATEWAY_REQUEST_TIMEOUT_MS` | 30000 | 1000 to 600000 |
| `GATEWAY_KEEPALIVE_TIMEOUT_MS` | 65000 | 1000 to 600000 |
| `GATEWAY_IDLE_TIMEOUT_MS` | 120000 | 1000 to 3600000 |
| `GATEWAY_UPSTREAM_IDLE_TIMEOUT_MS` | 120000 | 1000 to 3600000 |
| `GATEWAY_WS_IDLE_TIMEOUT_MS` | 300000 | 1000 to 86400000 |
| `GATEWAY_SHUTDOWN_GRACE_MS` | 10000 | 0 to 300000 |
| (check interval) | 1 s | Fixed, unused by behaviour |
| `GATEWAY_NODE_ID` ... | off | Node links, below |
| `CAPTURE_DIR` ... | off | The capture, section 12 |

The checks run in this order, so the first failure is the one reported: game, bind, private-mode address, cert and
key, port, secret, content, backend, ws, allowlist (url, secret, refresh, additional), body cap, connection limits,
the six rates, node links, the timeouts.

**Loopback URLs** (`parse_loopback_url(text, name)`). Must parse as a URL (`<name> is not a URL`), have scheme
`http` (`<name> must be an http:// URL on this machine`), a host that is loopback once IPv6 brackets are stripped
(`<name> must point at 127.0.0.1 or ::1 (got <host as written>)`), and path `/` with no query, user or password
(`<name> must be just http://host:port`). The endpoint is that host (brackets stripped) and the port (80 when none).
An endpoint's `authority()` is `host:port`, with the host in brackets when it contains `:`. Why: every upstream
request carries the gateway secret, which must never leave the machine.

**Secrets** (`check_secret(value, name)`). Unset: `<name> is not set`. Otherwise 32 to 256 bytes, each printable
ASCII without space (0x21 to 0x7E), else `<name> must be 32 to 256 printable characters without spaces (for
example 64 hex characters)`.

**Node links** (off unless `GATEWAY_NODE_ID` is set):

- `GATEWAY_NETWORK_NODES` without `GATEWAY_NODE_ID`: `GATEWAY_NETWORK_NODES needs GATEWAY_NODE_ID, this node's name`.
- Private mode: `GATEWAY_NODE_ID (node links) needs the TLS gateway: a private server (GATEWAY_TLS=0) is plain HTTP`.
- `GATEWAY_NODE_ID` parsed as a `NodeId` (`GATEWAY_NODE_ID: <parse error>`); `GATEWAY_NODE_REGION` required
  (`GATEWAY_NODE_REGION is not set (this node's region)`) and parsed as a region (`GATEWAY_NODE_REGION: <error>`).
- `GATEWAY_NODE_CERT`, `GATEWAY_NODE_KEY`, `GATEWAY_NODE_CA` each required and made absolute:
  `<NAME> must name the PEM file \`network issue-node-cert\` wrote for this node`. The CA file holds the cluster CA's
  certificate, never its key.
- `GATEWAY_NETWORK_NODES`: a JSON list of `network_plugin::cluster::Peer` (`GATEWAY_NETWORK_NODES: <serde error>`),
  checked with `cluster::check_peers` against this node (`GATEWAY_NETWORK_NODES: <error>`). Unset is an empty list.
- The node list the launcher pings (`config.network`): the peers that carry a fingerprint, in list order, the first
  per region only, at most 16; each as `{region, host, port, fingerprint}`. Empty outside a network.

## 2. TLS and the certificate

### 2.1 Loading at start (public mode)

The gateway loads the certificate and key it is given and never makes one: their SHA-256 fingerprint is the `fp` of
every invite already sent.

1. The certificate file is read as a PEM chain; no certificate: `no certificate in the file`. The key file is one PEM
   private key. Any read or parse error, or the key not matching the first certificate (checked with aws-lc-rs):
   `the key does not belong to the certificate`. Every such error is reported as
   `Certificate <cert path> / key <key path>: <error>`.
2. The first certificate is described: fingerprint = lowercase hex SHA-256 of its DER (64 characters); validity
   dates; subject alternative names as Node's `X509Certificate.subjectAltName` writes them, joined with `, `:
   `DNS:<name>`, `IP Address:a.b.c.d`, IPv6 as `IP Address:` and eight upper-case hex groups with nothing folded
   (`2001:DB8:0:0:0:0:0:1`), `email:<x>`, `URI:<x>`; other kinds left out. A DER that does not parse:
   `not a certificate: <error>`; a date out of range: `certificate date out of range`.
3. Not valid now (not-after at or before now, or not-before after now): `The certificate is not valid now (valid <ISO>
   to <ISO>). Make a new one with \`gateway make-cert\`; invites then need the new fingerprint.` ISO is JavaScript's
   `toISOString` (`YYYY-MM-DDTHH:MM:SS.mmmZ`).
4. Expires within 30 days: a warning `the certificate expires within 30 days; make a new one and send new invites`
   with `{"notAfter": <ISO>}`.
5. The TLS settings: aws-lc-rs, TLS 1.3 and 1.2 only, no client authentication, the whole chain, ALPN `http/1.1`
   only.

A public gateway built without TLS settings, or a private one with them, refuses to construct:
`the TLS gateway needs its certificate and key`, or
`a private gateway (GATEWAY_TLS=0) serves plain HTTP and takes no certificate`.

### 2.2 `make-cert`

`gateway make-cert` makes a certificate for a new install or prints one's fingerprint. Its arguments are read in
order: `--host <v>` (repeatable), `--out <dir>`, `--cert <file>`, `--key <file>`, `--fingerprint <file>`, `--force`,
`--json`, `--help`/`-h`. A value that is missing or starts with `--`: `<flag> needs a value`. Anything else:
`unknown argument <JSON string>`. Usage text, printed to stderr after any error (prefixed `make-cert: <message>` on
its own line when there is one), exit code 2:

```text
usage: gateway make-cert --host <public IP or DNS name> [--host ...] (--out <dir> | --cert <file> --key <file>) [--force] [--json]
       gateway make-cert --fingerprint <cert.pem> [--json]

  gateway make-cert --host 203.0.113.7 [--host play.example.org] --out C:\ReforgedServer\tls
  gateway make-cert --fingerprint C:\ReforgedServer\tls\gateway-cert.pem
```

- `--fingerprint <file>`: reads the first PEM certificate and describes it (a failure is a usage error
  `cannot read <file>: <error>`). With `--json`, one line `{"cert": <absolute path>, "fingerprint", "notBefore",
  "notAfter", "names"}`; otherwise three lines, the labels padded to 13 columns: `Names:       <names>`,
  `Valid:       <ISO> to <ISO>`, `Fingerprint: <hex>`. Exit 0.
- Otherwise the hosts become alternative names. None: `give at least one --host (the server's public IP address or DNS
  name)`. Each is trimmed and lower-cased, duplicates dropped (first kept). An IP address with a `%` zone:
  `<raw>: no zone ids`. An IP address becomes an IP name; a DNS name (at most 253 characters, at least one letter
  somewhere, dot-separated labels of 1 to 63 letters, digits or hyphens, not starting or ending with a hyphen; so no
  wildcard and no trailing dot) a DNS name; anything else `<JSON string> is neither an IP address nor a DNS name`.
- Where: `--out <dir>` gives `<dir>/gateway-cert.pem` and `<dir>/gateway-key.pem`; or both `--cert` and `--key`.
  `--out` with either of the others: `use either --out or --cert/--key`; neither:
  `say where to write: --out <dir>, or --cert <file> and --key <file>`. Paths are made absolute.
- Without `--force`, an existing certificate or key file: `<path> exists. A new certificate changes the fingerprint and
  breaks every invite already sent; add --force if that is what you want`.
- The certificate: self-signed RSA 2048 with SHA-256; serial 16 random bytes made positive and non-zero (top bit
  cleared, lowest bit of the first byte set); not-before now minus one day; not-after ten calendar years after
  not-before; subject CN = the first host, O = `Dainty Lass server`; not a CA (explicit); key usage digital signature
  and key encipherment; extended key usage server authentication; the alternative names in order. The key is PKCS#8
  PEM.
- Writing: the folders are made; the key is written first, mode 0600 where the file system has modes; then the
  certificate. A failure prints `make-cert: <error>` and exits 1.
- Output with `--json`: one line `{"cert", "key", "fingerprint", "notBefore", "notAfter", "names"}`. Otherwise:

```text
Certificate: <cert path>
Key:         <key path>  (keep private)
Names:       <names>
Valid:       <ISO> to <ISO>
Fingerprint: <hex>

Invites for this server carry: fp=<hex>
```

`load_or_generate(cert, key, hosts)` (the host kit's cold start, never the gateway's start): both files exist, load
them; exactly one exists, refuse with `only one of <cert> and <key> exists; refusing to overwrite`; neither, make one
as above for `hosts` (default `127.0.0.1` and `localhost`), write it the same way, load it, and say it was made.

## 3. Connections

### 3.1 Admission

Each accepted TCP connection is admitted or dropped before any byte is read, in this order:

1. While the gateway is closing, dropped.
2. The peer address (section 15) and its rate key (IPv4 address, IPv6 /64).
3. Open connections at `GATEWAY_MAX_CONNECTIONS`: dropped; warn `connection refused: the gateway is at its connection
   limit` `{ip, max}`, throttled under the key `drop`.
4. Open connections from this rate key at the per-address limit: dropped; warn `connection refused: too many open
   connections from one address` `{ip, open}`, throttled under `perip:<key>`. No connect token is spent.
5. The connect bucket (section 11) refuses: dropped; warn `connection refused: too many new connections from one
   address` `{ip}`, throttled under `connrate:<key>`.
6. Admitted: counted in both totals until the connection ends.

"Throttled" means `service_kit::linelog::Throttle` with a 60 s window: the first per key per minute is logged.

TCP_NODELAY is set. In private mode the connection is served as plain HTTP. In public mode the TLS handshake must
finish within the handshake timeout; a failure or timeout is a debug line `tls handshake failed` `{ip, error}` (the
error is `timed out` on the clock), throttled under `tls:<ip>`, and the connection ends. With node links, the
handshake is split by ALPN (section 9.1).

### 3.2 HTTP on a connection

HTTP/1.1 only, upgrades allowed, at most 100 headers and 16 KiB of head. hyper's own header-read timer is off: the
timers below replace it, because hyper's runs while a keep-alive connection waits too and would cut it at its header
time instead of the keep-alive time. Malformed heads, oversized heads and timeouts that end a connection are logged
as one `request` line `{ip, route: "gateway", status, reason}`, throttled under `clienterror:<ip>`: an oversized head
is 431 `HPE_HEADER_OVERFLOW`, a parse error 400 `client_error`, a timeout 408 `ERR_HTTP_REQUEST_TIMEOUT`; resets and
clients that went away log nothing. hyper answers 400 or 431 itself where it can.

### 3.3 The connection timers

The stream under hyper keeps Node's timers. A connection is in one of three phases:

- **Awaiting a head**, since an instant, for a limit. A new connection starts here with the headers timeout as its
  limit; after each answer it returns here with the keep-alive timeout. When the first byte of a head arrives, that
  instant is remembered and the whole head must then arrive within the headers timeout.
- **In a request**: from the moment a head is handed to the handler until the answer's body is finished or dropped.
  Only the idle timer applies.
- **Upgraded**: a WebSocket. Only the idle timer applies, with the WebSocket idle time.

The idle timer: any phase ends when no byte moved either way for the idle time (the WebSocket idle time once
upgraded). Each byte read or written resets it.

The deadline at any moment is the earliest of: last activity plus idle; and, while awaiting a head, either (first
byte of the head plus the headers timeout) or (the phase's start plus its limit).

When a read is pending and the deadline has passed:

- If the head is overdue (awaiting a head and either its first byte came more than the headers timeout ago, or no byte
  came and this connection has never answered a request and the phase's limit passed), the stream writes exactly
  these bytes, then shuts its write side:

  ```text
  HTTP/1.1 408 Request Timeout\r\nContent-Type: application/json\r\nContent-Length: 27\r\nConnection: close\r\n\r\n{"error":"request_timeout"}
  ```

  It then reads and discards what the client still sends until the client closes or one second has passed since the
  408 began, and then ends the connection with a timed-out error. (A socket closed with unread input is reset, and on
  Windows a reset drops the answer the client has not read yet.)
- Otherwise (an idle keep-alive connection that already answered a request, a request or upgrade gone idle) the
  connection ends with a timed-out error and no bytes.

A pending write past the deadline also ends with a timed-out error. Shutting the write side (hyper does this after an
answer that closes the connection, a 413 before the body was read for example) is followed by the same discard
linger of at most one second.

Examples from the tests, with headers 10 s, keep-alive 65 s, idle 120 s: a new silent connection gets the 408 at 10 s;
a slow head that keeps dripping still gets its 408 at 10 s and the connection ends at 11 s; a connection that answered
a request, then received one byte at 60 s and nothing more, gets the 408 at 70 s and ends at 71 s; a request in
progress with no traffic ends at 120 s; an upgraded connection with a 300 s WebSocket idle ends at 300 s.

### 3.4 Shutdown

On the stop signal: stop accepting (the listener is dropped, new connections are refused), mark the gateway closing,
and ask every connection to finish gracefully: requests in flight complete and their answers carry
`Connection: close`; WebSockets are cut at once (the chat client reconnects). Whatever is still open after
`GATEWAY_SHUTDOWN_GRACE_MS` is aborted. Then the allowlist feed is given time to send what it queued. The whole stop
never waits more than the grace plus five seconds. A request that hangs is cut at the grace; a request that finishes
early lets the stop finish early.

Every 60 s the rate limiters drop their full buckets (section 11).

## 4. The route policy

Decided from the method, the request target and the headers only, before any body is read. The routes are the
backend's: the 1.14.7 runtime sends the game's calls as `/__origin/<official host>/<path>`, which the backend
unwraps before routing, so the rules judge that inner path.

### 4.1 Path helpers

- **`split_target(raw)`**: the target must start with `/` (origin form); absolute form (`http://host/...`), `*` and
  the empty string are refused. The path is everything before the first `?`; the query after it (empty when none).
- **`check_path(path)`**: false if the path contains a backslash, any character up to U+001F or U+007F, or any
  JavaScript white-space character (`js::is_space`, so U+00A0 and U+FEFF too); or, case-insensitively, `%2f`, `%5c`,
  `%00` or `%2e`; or any `/`-separated segment that is exactly `.` or `..`. `/a.b/c..d/.e` is fine.
- **`canonical_path(path)`**: percent-decoded as `decodeURIComponent` does (`js::decode_uri_component`; failure is
  `None`), lower-cased (Unicode), runs of `/` folded to one, trailing `/` removed unless the path is `/`. Examples:
  `/Reforged/API/Register/` to `/reforged/api/register`; `//a///b//` to `/a/b`; `/%41` to `/a`; `/%zz` and `/%C3`
  fail.
- **`under(canonical, prefix)`**: the path equals the prefix or continues with `/` after it.
- **`unwrap_origin(path)`**: a path whose first segment, after any number of leading slashes, is `__origin`
  (case-insensitive) followed by `/` or the end. Others are returned as they are, with no origin. For a wrapped path:
  after `__origin` there must be a `/` and then a host made of the characters before the next `/`, `?` or `#`; an
  empty host fails. The host is percent-decoded (failure fails), lower-cased, and must be 1 to 253 characters of
  `a-z`, `0-9`, `.`, `-`, not starting or ending with `.` and without `..`; else it fails (so `user@host` and
  `host:443` fail). The inner path is the rest (`/` when empty, a `/` added in front when it lacks one). An inner
  path that is itself wrapped fails. Failure means 400 `bad_origin` (below). Examples:
  `/__origin/gamedev-prod.steelyard.ca/party` to `/party` with that host; `/__origin/API.EpicGames.dev` to `/` with
  `api.epicgames.dev`; `//__Origin/a%2Db.example/x` to `/x` with `a-b.example`; `/__originx/y` is not wrapped;
  `/__origin`, `/__origin/`, a 254-character host and `/__origin/x/__origin/y/z` fail.

### 4.2 Constants

- Public `/reforged/api` routes (by canonical name after `/reforged/api/`): `register` (POST), `getuserinfo` (GET,
  HEAD), `serverstatus` (GET, HEAD), `registrationstatus` (GET, HEAD).
- Host-only prefixes: `/admin` (the backend's admin API), `/launcher` (its own launcher API), `/internal` (metrics),
  `/health` (readiness probes), `/metrics`.
- Game-server key headers: `x-rustic-isles-gameserver-apikey`, `x-rustic-isles-gameserver-key`,
  `x-mysticparadox-gameserver-apikey`, `x-mysticparadox-gameserver-key` (the backend still reads the old names for
  one release, DX4).
- Host-only headers: the four above, `x-reforged-admin-key`, `x-rustic-isles-metrics-token`,
  `x-mysticparadox-metrics-token`.
- Allowed methods: GET, HEAD, POST, PUT, DELETE, PATCH, OPTIONS.
- Network routes (answered by the gateway, section 9.3): `/network/v1/nodes` (GET), `/network/v1/ping` (GET),
  `/network/v1/regions` (POST).

### 4.3 `classify(method, target, headers, is_upgrade)`

The method is upper-cased. Each step returns a decision; a rejection carries a status, an error code (the body) and a
reason (the access log), and every decision carries the rate-limit class.

1. Not origin form: 400 `bad_request`, reason `target_not_origin_form`, class general.
2. `check_path` fails: 400 `bad_path`, reason `bad_path`, class general.
3. `unwrap_origin` fails: 400 `bad_path`, reason `bad_origin`, class general.
4. `canonical_path` of the inner path fails: 400 `bad_path`, reason `bad_percent_encoding`, class general.
5. Content means: not wrapped, and the canonical path under `/content`.
6. The class: an upgrade is `ws`; else canonical `/reforged/api/register` is `register`; `/account/api/oauth/token`
   is `token`; content is `content`; anything else `general`. (Steps 1 to 4 always use general.)
7. Method not allowed: 405 `method_not_allowed`, reason `method`.
8. Any host-only header present (by name; any value, even empty): 403 `forbidden`, reason `gameserver_key_header` for
   the four key names, `host_only_header` for the rest.
9. Not wrapped and the canonical path is exactly a network route: an upgrade is 400 `bad_request`
   `unsupported_upgrade`; another method than the route's is 405 `method_not_allowed` `method`; else the network
   route (answered by the gateway).
10. Canonical path under `/reforged`: it passes to the backend only if it is `/reforged/api/<name>` with a non-empty
    name without `/`, the name is public, the request is not an upgrade, and the method is one of the name's. Anything
    else under `/reforged` is 403 `forbidden`, reason `admin_route`, even with an admin key (which step 8 refused
    anyway), and even as an upgrade.
11. Under a host-only prefix: 403 `forbidden` `host_only_route`. (`/administrator` and `/launchers` are not under
    them.)
12. An upgrade: it must be a GET and its `Upgrade` header, lower-cased, exactly `websocket`; else 400 `bad_request`
    `unsupported_upgrade`. Then: the chat upstream.
13. Content: the content upstream.
14. `POST` and canonical `/heartbeat`: the backend, feeding as a heartbeat. `POST` and canonical
    `/account/api/oauth/token`: the backend, feeding as a token. (Not `/game/login`, which was Dauntless Revived's,
    nor `POST /login`.)
15. Anything else: the backend.

Because the rules use the inner path, `/__origin/<host>/reforged/api/CreateInvite`, `/__origin/<host>/admin/...` and
so on are refused the same way, and `/__origin/<host>/content/...` goes to the backend (which answers 404 itself).

Examples that must hold (the policy's unit table): every one of these POSTs is 403 or 400, never proxied:
`/REFORGED/API/CREATEINVITE`, `/reforged/api/createinvite`, `/Reforged/Api/CreateInvite/`,
`/reforged/api/CreateInvite//`, `/reforged//api/CreateInvite`, `//reforged/api/CreateInvite`,
`/reforged/api/%43reateInvite`, `/%72eforged/api/CreateInvite`, `/reforged/api/CreateInvite?x=/reforged/api/Register`,
`/reforged/api/Register/../CreateInvite`, `/reforged/api/./CreateInvite`, `/reforged%2fapi/CreateInvite`,
`/reforged/api%2FCreateInvite`, `/reforged/api/CreateInvite%00`, `/reforged/api\CreateInvite`,
`/reforged/api/%2e%2e/api/CreateInvite`, `/reforged/api/Register;/CreateInvite`, `/reforged`, `/reforged/api`,
`/reforged/api/`, `/reforged/api/Register/x`, `/reforged/api/%zz`; while `POST /reforged/api/Register/` passes
(the trailing slash is the same route, as Express treats it). Other cases: `HEAD /reforged/api/Register`,
`POST /reforged/api/RegistrationStatus` and `GET /reforged/api/UsernameAvailable?Username=x` are 403; `TRACE` and
`CONNECT` are 405; an `h2c` upgrade or a `POST` upgrade is 400; an upgrade to `/reforged/api/ServerStatus` is 403; an
upgrade to `/health/live` is `host_only_route`; the game's chat upgrade `GET //` with `Upgrade: websocket` goes to the
chat upstream; `/account127.0.0.1:61000` (the client's own odd path) goes to the backend;
`/inventory/<uuid>/dauntlessrel-1.4.4:239827` passes.

### 4.4 `has_bearer(headers)`

True when the `Authorization` header is valid text, its first six characters are `bearer` in any case, then at least
one JavaScript white-space character, then a non-white-space character (`/^bearer\s+\S+/i`). `Bearer abc`,
`bearer abc`, `BEARER\t x` pass; `Bearer `, `Bearerabc`, `Basic abc` and none do not. It checks only the shape (a
known weakness, section 10).

## 5. Module routes

`modules.rs` is Category A and moves unchanged; this section says what it relies on, so the rewrite keeps it working.
`ModuleRoutes::route(version, game, method, target, headers, is_upgrade)` first runs `classify_for(game, ...)` (which
is `classify`; the game argument is unused since only reborn is served). It re-reads the target with `split_target`,
`check_path`, `unwrap_origin` and `canonical_path`, refuses the gateway's `/__node` group (403 `forbidden`
`node_link`, on the raw or the inner path), and looks up the module and its longest matching route. Its refusals:
404 `not_found` `no_route`, 503 `unavailable` `no_module`, 503 `unavailable` `module_unavailable`, 502 `bad_gateway`
`upstream_not_loopback`, 403 `forbidden` `host_only_route`. It refuses a module route under `HOST_ONLY_PREFIXES`. It
returns the decision and, for a module-bound request, the module's loopback endpoint; content and (unless a module
registered a WebSocket route) chat keep the gateway's own endpoints.

A gateway started without a controller attaches the stand-in (`modules::stand_in`: module `isles-1-14-7`, "Rustic
Isles 1.14.7", the game version and its invite alias, one TCP port `backend` with default 61000, one route `/`)
pointed at `GATEWAY_BACKEND_URL`.

## 6. Handling one request

On a public or private connection (a node connection is section 9.1), in order:

1. The phase becomes "in a request".
2. If the request is an upgrade (an `Upgrade` header and `upgrade` among the `Connection` tokens), section 8.
3. If the method is `POST`, the path is exactly `/__node/enroll`, and node links are on with an enroll handler set,
   section 9.2.
4. The access record is made (section 14) and the request routed (section 5) as not an upgrade.
5. The rate-limit class's bucket is taken for the peer's rate key (section 11). Refused: 429 `rate_limited`, reason
   `rate_limited:<class>`, header `retry-after: <seconds>`. The bucket is taken before the decision is acted on, so a
   request the policy refuses still spends a token.
6. A rejection is answered as a refusal (section 13). A network route is section 9.3.
7. The upstream's endpoint: content is the content setting, chat is the module's WebSocket endpoint or else the ws
   setting, the backend is the module's endpoint. None: 502 `bad_gateway`, reason `no_upstream`.
8. A `Content-Length` that, read as a number, exceeds the body cap: 413 `body_too_large` before any byte of the body
   is read (so an `Expect: 100-continue` client never gets its 100).
9. Proxy (section 7).

Each request also records a metric (`service_kit::metrics::http_request("gateway", route, method, status, start)`),
where route is `backend`, `content`, `ws`, `node`, `enroll`, `network` or `refused`.

## 7. Proxying

### 7.1 Request headers (`forward_headers`)

Start from the client's headers without the hop-by-hop ones and those the `Connection` header names
(`proxy::end_to_end`), and without the client's address and origin claims: `x-forwarded-for`, `x-forwarded-proto`,
`x-forwarded-host`, `x-forwarded-port`, `x-forwarded-prefix`, `forwarded`, `x-real-ip`, `x-client-ip`,
`true-client-ip`, `cf-connecting-ip`, `x-reforged-gateway`, and `expect` (the gateway answered any 100-continue
itself). Keep the rest in their order (`host`, `authorization`, `x-reforged-user-api-key` and every other header pass
byte for byte). Then set, replacing never appending: `x-forwarded-for` to the peer address, `x-forwarded-proto` to
`https` in public mode and `http` in private, `x-reforged-gateway` to the gateway secret; for an upgrade also
`connection: Upgrade` and `upgrade: websocket`. Example: client headers `x-forwarded-for`, `forwarded`, `x-real-ip`,
`x-reforged-gateway`, `expect`, `connection: keep-alive, x-hidden`, `x-hidden`, `upgrade`, `authorization`, `host`
leave, in order, `authorization`, `host`, `x-forwarded-for`, `x-forwarded-proto`, `x-reforged-gateway`.

### 7.2 The exchange

- The upstream request goes to `http://<endpoint authority><path and query exactly as received>` with the client's
  method. A pooled HTTP/1.1 client (TCP_NODELAY, at most 64 idle connections per host, idle ones closed after 30 s).
  A target that cannot form a URL: 400 `bad_request`, reason `target_not_origin_form`.
- The request body streams through as it arrives, counted. Past the body cap the stream fails (the upstream request
  is aborted) and the exchange is marked over the cap. If the body has not finished by the request timeout (counted
  from the start of the exchange) the stream fails and the exchange is marked timed out. A client body error marks it
  failed.
- The upstream must answer its status line and headers within the upstream idle timeout.
- Failures, in this order: over the cap, 413 `body_too_large` (access route `gateway`); timed out, 408
  `request_timeout`, reason `ERR_HTTP_REQUEST_TIMEOUT` (route `gateway`); the upstream's answer did not come in time,
  504 `upstream_timeout`, reason `upstream_timeout`; another failure, 502 `bad_gateway`, reason
  `upstream_<CODE>` (route stays the upstream's). The access line's `bytesIn` is what the client sent so far.
- `<CODE>` walks the error's source chain and takes the first I/O error: connection refused `ECONNREFUSED`; reset,
  aborted, broken pipe or unexpected end `ECONNRESET`; timed out `ETIMEDOUT`; address not available `EADDRNOTAVAIL`;
  any other I/O kind `error`. A hyper error that is an incomplete message or a closed connection, met before any I/O
  error, is `ECONNRESET`. Nothing found: `error`. (An I/O error of kind "other" wrapping a refused connection is
  `error`: the first I/O error decides.)
- The feed (section 10) is told about the peer when the answer is 2xx and the decision feeds, as a token, or as a
  heartbeat whose request had a bearer.
- The answer: the upstream's status; its headers without hop-by-hop ones, those its `Connection` names,
  `x-powered-by` and `x-reforged-gateway`; plus `connection: close` while the gateway is closing. The body streams
  through, counted; if no frame comes for the upstream idle time it fails ("upstream timed out") and the client's
  connection is cut. Repeated headers (`set-cookie`) stay repeated.
- The access line is written when the answer body ends or is dropped: `bytesOut` is the bytes passed on, and
  `aborted: true` is added when it neither ended nor sent its whole declared length.

## 8. WebSocket upgrades

1. The access record is made; the capture records upgrades as chat, not as an HTTP record.
2. Every refusal on this path is a refusal (section 13) that always carries `connection: close`, made as if the
   request had no body.
3. While closing: 503 `shutting_down`, reason `shutting_down`.
4. Routed as an upgrade; the ws bucket taken (429 as in section 6). A rejection is answered as such; a network route
   cannot reach here (the policy refused it) but would be 400 `bad_request` `unsupported_upgrade`. No endpoint: 502
   `bad_gateway` `no_upstream`. From here the access route is `ws`.
5. A new TCP connection to the endpoint (TCP_NODELAY), an HTTP/1.1 handshake, and a request with the client's path and
   query, no body, and the forwarded headers for an upgrade. Connecting plus the upstream's answer head must finish
   within the headers timeout: else 504 `upstream_timeout`; a failure 502 `bad_gateway` `upstream_<CODE>`.
6. An answer that is not 101: its status and its end-to-end headers (as in 7.2) without `content-length`, plus
   `connection: close`; its body read to the end, keeping at most 64 KiB, and sent whole (with its length).
   Logged with `bytesOut` = that body's length. Example: an upstream 404 `no room` reaches the client as 404,
   `text/plain`, `content-length: 7`, `connection: close`, `no room`.
7. A 101: the client gets 101 with the upstream's headers except `x-powered-by` and `x-reforged-gateway` (hop-by-hop
   ones such as `connection`, `upgrade` and the `sec-websocket-*` headers pass as they are). The access line is
   written now: status 101, `bytesIn` 0, `bytesOut` 0. Both sides are upgraded and joined byte for byte
   (`proxy::tunnel`) with the WebSocket idle time; the connection's phase becomes upgraded; the open-WebSocket count
   goes up. When the gateway starts closing, the tunnel is dropped at once (its counts then read 0). When it ends, the
   count goes down and an info line `websocket closed` `{ip, target, bytesIn, bytesOut, ms}` is written (in from the
   client, out to it; target as the access log redacts it).

## 9. Node links, enrollment and the network routes

### 9.1 Node links on the public listener (DN3)

Only in public mode with node links configured. At start the node's certificate, key and the cluster CA are loaded
through `network_plugin::cluster::NodeCredentials`, and refused when they fail
(`Node certificate <cert> / key <key> / CA <ca>: <error>`) or name another node or region
(`The node certificate is for <node> in <region>, but this node is <node> in <region> (GATEWAY_NODE_ID,
GATEWAY_NODE_REGION)`). The endpoint (`network_plugin::endpoint::NodeEndpoint`) accepts the listed peers.

At each TLS connection the ClientHello is read first. A client that offers the node ALPN name
(`cluster::NODE_ALPN`) gets the node's mutual-TLS settings (it must present a certificate that chains to the cluster
CA); any other client gets the public certificate and the public policy, as without node links. The handshake timeout
covers both. A node handshake that fails, or whose verified certificate names no node, is a warning (not a debug line)
`node link refused at the handshake` `{ip, error}` (error `its certificate names no node` for the second), throttled
under `nodetls:<ip>`. A public handshake that fails is section 3.1's debug line.

On a node connection every request is a node request: no policy, no rate limit, no capture, the caller is the node its
verified certificate names and nothing else. For each request:

1. The endpoint admits it from the method, the path and query, the `Content-Length` (parsed as an integer), and the
   time header (`wire::TIME_HEADER`) when exactly one is present. A refusal is its reply.
2. The body is read up to `link::MAX_MESSAGE` within the request timeout: over, 413 with the wire's `too_large` code;
   an error or the clock, 400 with `bad_request`.
3. The endpoint serves it; its reply is sent.

A reply: its status, `content-type: application/json`, `content-length`, `cache-control: no-store`, the time header
with the reply's milliseconds, and `connection: close` when the gateway is closing or when the reply refused the
request before its body was read and the request had one (`transfer-encoding` present, or a `content-length` other
than `0`). The access line has route `node`, no user agent, and the reply's error code as reason when the status is
not 200. Without node links (cannot happen on a node connection) the reply is 404 with the wire's `not_found` code.

### 9.2 `POST /__node/enroll`

Only when node links are on and the core set an enroll handler; otherwise the path goes through the policy and the
module routes refuse it (403 `node_link`). The register bucket is taken (429 `rate_limited`,
`rate_limited:register`, `retry-after`). The body is read up to 64 KiB within the request timeout: over, 413
`body_too_large`; an error or the clock, 400 `bad_request`, reason `body_error`. The handler gets the bytes and returns
a status and a body, which are sent with `content-type: application/json`, `content-length`, `cache-control:
no-store` (and `connection: close` while closing). Access route `enroll`.

### 9.3 The network routes (DN7)

Answered by the gateway, never proxied; counted against the general bucket (step 5 of section 6).

- `GET /network/v1/nodes`: 200, `content-type: application/json`, `cache-control: no-store`, body
  `{"nodes":[{"region","host","port","fingerprint"},...]}` (at most 16; `{"nodes":[]}` outside a network).
- `GET /network/v1/ping`: 204, `cache-control: no-store`, no body.
- `POST /network/v1/regions`: the credentials first, before the body: an `x-reforged-user-api-key` that passes
  `protocol::account::is_plausible_account_key` (8 to 128 of letters, digits, `_`, `-`) or an
  `x-reforged-session-ticket` of 1 to 4096 bytes; neither is 401 `unauthorized`, reason `no_account_key`. Then the
  body, up to 4096 bytes within the request timeout: over, 413 `body_too_large`; an error, 400 `bad_request`
  `body_error`; the clock, 408 `request_timeout` `ERR_HTTP_REQUEST_TIMEOUT`. A body `network::parse_region_report`
  refuses is 400 `bad_request` `bad_region_report`. With a region handler set, it gets the key, the ticket and the
  regions; `false` is 401 `unauthorized` `invalid_credentials`. Otherwise 204 as for ping. Without a handler the report
  is stored nowhere.

`connection: close` is added to these answers while closing. Access route `network`.

## 10. The allowlist feed

The feed tells the allowlist helper which addresses belong to players who logged in. It never delays a player's
request: reporting is fire and forget.

- **Who is reported.** Only a 2xx answer to a request the policy marked as feeding: the token route always, the
  heartbeat only when the request carried a bearer (4.4). The backend answers `POST /heartbeat` without checking the
  token, so any `Authorization: Bearer <anything>` opens the game ports for its sender. That weakness is Node's and is
  kept; closing it is a backend change.
- **In-process.** When the core gave the gateway an in-process sink, a report of any address but `unknown` calls the
  sink (and counts as sent) and nothing goes over HTTP, not to the additional helpers either. A gateway with only a
  sink (feed setting off) still reports to it.
- **Over HTTP**, per helper (the main one and each additional one, each with its own state; a report goes to every
  additional helper as well as the main one):
  - `unknown` is never sent.
  - While backing off (below), nothing is sent, and the address is not recorded, so it is offered again after.
  - An address the helper accepted is not sent again for the refresh interval; one already on its way is not sent
    again. When more than 50000 addresses are remembered, those older than the interval are forgotten.
  - The request: `POST <helper>/allow`, `content-type: application/json`, `content-length`, `x-allowlist-secret:
    <secret>`, body `{"ip":"<address>"}`; the whole exchange (answer and its body) within 3 s, else `timed out`. A
    pooled client keeps at most 4 idle connections per helper, closing them after 2 s.
  - 200: the helper is up again, the address is remembered as sent, a debug line `allowlist refreshed` `{ip}`.
  - 400: the helper is up; the address is remembered as sent too (asking again would not change the answer); a
    warning `allowlist helper refused a player's address; that player's game ports stay closed` `{ip}`, throttled
    per `refused:<ip>`.
  - Any other status (`status <n>`) or a failure: a failure.
  - Backoff: failures in a row are counted; any answer clears the count and the backoff. From the third failure in a
    row the helper is left alone for 2 s, doubling with each further failure, at most 60 s. Each failure warns
    `allowlist helper did not take an address; players who log in now get no game ports until it answers`
    `{detail, suppressed}`, throttled under `down`.
  - Example (tested): 12 distinct addresses against a helper answering 503 cost three requests; 1.5 s later nothing
    is sent; 2.5 s after the backoff began one probe is sent. A helper that answers 200 after the backoff is used at
    once for the next address.
- **Idle.** The feed can wait until nothing is on its way (tests and shutdown), for every helper.
- With no feed and no sink the start warns `allowlist feed is off (GATEWAY_ALLOWLIST=0): the game ports open for
  nobody unless the firewall is managed another way`.

## 11. Rate limits and body limits

**Buckets** (per class, per rate key; the key is the IPv4 address or the IPv6 /64, section 15). A bucket holds up to
`burst` tokens and refills continuously at `per minute`. A new key starts full. Each request takes one token; with
less than one, it is refused with a retry-after of the seconds until one token is back, rounded up (after taking a
millionth of a second off to absorb floating-point dust), at least 1; a bucket that never refills says 3600. Buckets
that have refilled completely are forgotten when swept (every 60 s, and when a limiter is full). A limiter holds at
most 100000 keys: when a new key arrives at the limit and a sweep frees nothing, the key that came first is forgotten.
Keys are kept in the order they first came. Examples: burst 3 at 60/min allows three at once, refuses the fourth with
1 s, allows one more after 1 s, refuses at once again, allows one at 2.5 s; burst 1 at 0.2/min refuses the second
with 300 s, and the bucket is still kept at 299 s and forgotten at 300.1 s.

**Bucket spec text** `"<burst>,<per minute>"`: split at the first comma (none: the shape error); each side trimmed of
JavaScript white space; the burst must be digits only, the rate digits or digits `.` digits. Shape error:
`<NAME> must look like "<burst>,<per minute>", for example "300,180"`. Then the burst must be 1 to 1000000 and the
rate at most 1000000: `<NAME>: burst must be 1 to 1000000 and the rate at most 1000000 per minute`. `300,180` and
` 5 , 0.2 ` pass; empty, `5`, `0,1`, `-1,2`, `a,b`, `5,1,2`, `5,.2`, `5,2.`, `1e3,1` fail.

The classes: `general`, `content`, `register`, `token`, `ws` per request (section 4.3, step 6) and `connect` per new
connection (section 3.1). Chat reconnects spend only the ws bucket, never the one the game's HTTP needs; an empty
general bucket still lets chat connect.

**Bodies.** The cap is `GATEWAY_MAX_BODY_BYTES`: by `Content-Length` before reading (413), and while streaming
(section 7.2). A body of exactly the cap passes. Region reports 4 KiB, enrollment 64 KiB, node messages
`MAX_MESSAGE`. A refused request's body is never read; its connection is closed instead (section 13).

## 12. The traffic capture (RD19)

Off unless `CAPTURE_DIR` is set. When on, the start warns `traffic capture is on: every request and answer is
recorded, redacted, for the shadow replay (personal data; see docs/security/gateway.md)` `{dir, retentionDays}`.

### 12.1 Settings

`CAPTURE_DIR` (trimmed; made absolute), `CAPTURE_RETENTION_DAYS` (default 14, whole 1 to 90),
`CAPTURE_MAX_BODY_BYTES` (default 262144, whole 0 to 16777216). A bad number:
`<NAME> must be a whole number from <min> to <max> (got <JSON string>)`, reported as `Traffic capture: <error>`. The
source name is `gateway`.

### 12.2 Files

The folder is made (with parents) mode 0700 where the file system has modes; failure is `Traffic capture: <dir>:
<error>`. Records go to `<dir>/gateway-<YYYY-MM-DD>.jsonl` by UTC day, appended, created 0600. At creation and at
each new day, files named `<lowercase letters>-<YYYY-MM-DD>.jsonl` whose day is before the cutoff day (now minus the
retention) are deleted (a failure is left for next time); other files are left alone. Each line is a JSON object
whose first two fields are `t` (ISO time) and `src` (`gateway`), then the record's fields, then a newline. Before
writing, every occurrence of the gateway secret and the allowlist secret (when at least 8 characters; as they appear
inside a JSON string, so escaped) in the line's text is replaced with `<redacted>`. Lines are written whole under a
lock. A failure to open or write stops the capture for good with one warning `traffic capture stopped: <error>`; the
gateway keeps serving.

Example: a line written at 2026-10-21T00:00:01Z with `{"kind":"http","seq":2,"echo":"<the secret>","s":"short"}`
reads `{"t":"2026-10-21T00:00:01.000Z","src":"gateway","kind":"http","seq":2,"echo":"<redacted>","s":"short"}`.

### 12.3 Records

- Connection numbers start at 1 and are given to a connection the first time it has something recorded. Sequence
  numbers start at 1, one per HTTP record.
- **HTTP**, one per answered request (written when the answer ends or is dropped; node requests and upgrades never):
  `kind: "http"`, `conn`, `seq`, `ip`, `method`, `target` (the capture's redaction of the raw target), `route` (as in
  the access log), `reqHeaders`, `reqBody`, `status`, `resHeaders`, `resBody`, `ms`, and `aborted: true` when the
  answer did not complete. Bodies keep at most the capture's body cap of bytes and count all of them. `resHeaders`
  are the upstream's (or the refusal's) as received.
- **Chat**, for an upgrade that switched: `{kind: "chat", conn, event: "open", ip, target}` (target redacted), then a
  record per whole message in either direction `{kind: "chat", conn, dir: "in"|"out", text}` (text redacted as a chat
  frame), or `omitted: <size>` past the cap, or `compressed: <size>` for a compressed message, and at the end
  `{kind: "chat", conn, event: "close", reason: "socket"}`. `in` is from the game, `out` from the backend.

### 12.4 Redaction

- **Text** (`redact_capture_text`): every U+0001 becomes `?` first (it marks placeholders). Then, in order: every JWT
  (`eyJ` then base64url characters, a dot, base64url characters, a dot, base64url characters; each part non-empty)
  becomes `<token:<subject>>` when its middle part decodes (lenient base64, below) to JSON with a string `userId`, or
  else a non-null `sub`, of 1 to 64 characters from `[A-Za-z0-9_-]`, else `<token>`; every account key
  (`RRK_` or `UUK_` then letters and digits) becomes `<key:<lowercase hex SHA-256 of the whole key>>`; then every run
  of 40 or more hex digits becomes `<redacted>`, then every run of 100 or more of `[A-Za-z0-9_+/=-]` becomes
  `<redacted>`. The token and key placeholders are put in after the last two rules, so those never cut a hash.
- **Lenient base64** (`decode_base64_lenient`): both alphabets (`+`/`-` are 62, `/`/`_` are 63), other characters
  skipped, decoding stops at the first `=`; leftover bits dropped.
- **Named values**: a value is redacted as text; if its name matches, case-insensitively, any of `key`, `token`,
  `secret`, `pass`, `auth`, `jwt`, `sig`, `cred`, `recovery`, `invite`, `cookie`, the result is kept only when it is
  exactly one token or key placeholder, else it becomes `<redacted>`.
- **JSON** (`redact_capture_json`): strings by the name of the field that holds them (array items by the array's
  name; the top level has no name); other values unchanged. Example: `{"Username":"Alex","InviteCode":"ABCD-EFGH",
  "Password":"hunter2","UUK":"<key>","nested":[{"access_token":"<jwt>","note":"x"}]}` becomes `{"Username":"Alex",
  "InviteCode":"<redacted>","Password":"<redacted>","UUK":"<key:hash>","nested":[{"access_token":"<token:UID-...>",
  "note":"x"}]}`.
- **Headers** (`redact_capture_headers`): an object of every header name to its values joined with `, ` (bytes read
  as Latin-1); `cookie`, `set-cookie` and `x-reforged-gateway` left out; the game-server key names become `<gskey>`;
  `authorization` is redacted as text; any other is a named value, with the name stripped first of `x-reforged-`,
  `x-rustic-isles-` or `x-mysticparadox-`. So `x-reforged-user-api-key` becomes `<key:hash>`, `x-reforged-admin-key`
  and the metrics tokens `<redacted>`, `user-agent` stays.
- **Bodies** (`redact_capture_body(kept, content type, total, max)`): total 0 is `null`; total over the max, or fewer
  bytes kept than total, is `{"omitted": total}`. Otherwise (the bytes read as UTF-8, lossy): when the content type
  (lower-cased) contains `json` or the text, after leading white space, starts with `[` or `{`, and it parses as JSON:
  `{"json": <redacted JSON>}`. Else when the type contains `x-www-form-urlencoded`: `{"form": [[name, named value],
  ...]}` in order. Else when there is no control character in U+0000-U+0008 or U+000E-U+001F and the bytes are valid
  UTF-8: `{"text": <redacted text>}`. Else `{"binary": total}`. Examples: `20000` with a JSON type is `{"json":20000}`;
  `{nope` with a JSON type is `{"text":"{nope"}`; bytes `FF 41` are `{"binary":2}`.
- **Chat frames** (`redact_chat_frame`): in each `<auth ...>X</auth>`, X (trimmed) is decoded leniently; its second
  NUL-separated part, if it is 1 to 64 characters from `[A-Za-z0-9_-]`, gives `<plain:<account>>`, else `<redacted>`
  (the element's tags kept). Each `<password ...>...</password>` content becomes `<redacted>`. Then the whole frame is
  redacted as text.
- **Targets**: redacted as text.

### 12.5 Reading chat frames

Each direction of the tunnel is read through a frame reader that never changes the bytes. It parses RFC 6455 frame
headers (7-bit, 16-bit and 64-bit lengths, the mask), unmasks the payload, and hands back each whole data message:
a message begins with a non-control frame whose opcode is not 0 (a new one drops a message still open), continues
with continuation frames, ends at a FIN frame; control frames (opcode 8 and up) are skipped, also between fragments.
A message's first frame with RSV1 set makes it `compressed(total)`; one over the cap is `omitted(total)` (bytes past
cap plus one are not kept); otherwise `text` (lossy UTF-8). A zero-length frame ends at its header. The reader gives
the same messages however the bytes are split, one at a time included.

## 13. Every answer the gateway makes itself

**A refusal** is: the status; body `{"error":"<code>"}`; `content-type: application/json`; `content-length`;
`cache-control: no-store`; `connection: close` when the request has a body (a `transfer-encoding` header, or a
`content-length` that read as a number is above 0) or the gateway is closing; plus `retry-after` for a 429. The
refusal's body is never read: the connection closes instead of draining it. The access line has the reason, and route
`gateway` unless noted.

| Status | Code | Reasons |
| --- | --- | --- |
| 400 | `bad_request` | `target_not_origin_form`, `unsupported_upgrade`, `body_error`, `bad_region_report` |
| 400 | `bad_path` | `bad_path`, `bad_origin`, `bad_percent_encoding` |
| 401 | `unauthorized` | `no_account_key`, `invalid_credentials` (route `network`) |
| 403 | `forbidden` | `gameserver_key_header`, `host_only_header`, `admin_route`, `host_only_route`, `node_link` |
| 404 | `not_found` | `no_route` |
| 405 | `method_not_allowed` | `method` |
| 408 | `request_timeout` | `ERR_HTTP_REQUEST_TIMEOUT` (body too slow; region report too slow) |
| 413 | `body_too_large` | `body_too_large` |
| 429 | `rate_limited` | `rate_limited:<class>`, with `retry-after` |
| 502 | `bad_gateway` | `no_upstream`, `upstream_<CODE>` (route the upstream's), `upstream_not_loopback` |
| 503 | `unavailable` | `no_module`, `module_unavailable` |
| 503 | `shutting_down` | `shutting_down` (upgrades only) |
| 504 | `upstream_timeout` | `upstream_timeout` (route the upstream's) |

Also: the raw 408 of section 3.3; hyper's own 400 and 431 for heads it cannot read; the node-link replies of 9.1
(their body is the network wire's error body); the enroll handler's answer; 200 and 204 of the network routes.
Refused upgrades always add `connection: close`.

Parity notes (from the fixtures): `GET /%zz` is 400 `{"error":"bad_path"}` with reason `bad_percent_encoding`;
`TRACE /heartbeat` 405, reason `method`; `POST /character` with `Expect: 100-continue` and a length over the cap is
413 with `bytesIn` 0; a chunked body of 1024-byte chunks over a 4096 cap is 413 with `bytesIn` 5120 (the count includes
the chunk that crossed the cap, and nothing after it is read); the third token request from one address at `2,1` is 429
with `retry-after: 60`; an upstream's chunked `text/plain` answer stays chunked, with both of its `set-cookie`
headers.

## 14. Logging and redaction

All lines go through `service_kit::linelog` as JSON with `level` and `msg` first. No header names or values are ever
logged; no secret (the gateway's, the helper's, an account key, a token) is ever logged.

**The access log**: one `info` line `request` per request, written when the answer is done or dropped, with fields in
this order: `ip`, `method`, `target`, `route`, `status`, `bytesIn`, `bytesOut`, `ms`, then `reason` when there is
one, `ua` when there is one, `aborted: true` when the answer did not complete. `bytesOut` counts only bytes that came
from an upstream (0 for the gateway's own answers). `route` is `gateway` for refusals, else `backend`, `content`,
`ws`, `network`, `enroll` or `node`. An upgrade's line is written at the 101 (section 8).

**Target redaction** (`redact_target`), for the access log:

1. Split at the first `?`.
2. The path is redacted "encoded": decode it as `decodeURIComponent` (on failure keep it); if the decoded text differs
   and redacting it changes it, the decoded and redacted text is what is logged (so a token hidden by `%2E` between
   its parts is still removed); else the raw text redacted.
3. The query is split on `&`. A part without `=` is redacted encoded (with `+` read as a space). A part whose decoded
   name contains, case-insensitively, `key`, `token`, `secret`, `pass`, `auth`, `code`, `session`, `jwt`, `sig` or
   `cred` becomes `<redacted name>=<redacted>`. Other parts are `<redacted name>=<redacted value>`. Rejoined with `&`
   after a `?`.
4. Control characters (up to U+001F, and U+007F) become `?`.
5. Longer than 400 UTF-16 units: the first 400 units (whole characters) and `...`.

**Text redaction** (`redact_text`), in order: every `eyJ<part>.<part>.<optional part>` (parts of letters, digits,
`_`, `-`; the first two non-empty) becomes `<token>`; every `RRK_` followed by letters and digits becomes `<key>` (a
bare `RRK_` stays); every run of 40 or more hex digits becomes `<redacted>`; every run of 64 or more of letters,
digits, `_`, `-` becomes `<redacted>`. Examples: `eyJ eyJa. eyJa.b eyJa.b. x` becomes `eyJ eyJa. eyJa.b <token> x`;
`/progression/UID-00000000-0000-4000-8000-00000000f609` is unchanged; `/a?accountId=UID-1&key=abc&access_token=zzz&
Code=1234` becomes `/a?accountId=UID-1&key=<redacted>&access_token=<redacted>&Code=<redacted>`; `/a?flag&b=%zz` is
unchanged; `/a\r\nforged` becomes `/a??forged`.

**The user agent** (`ua`): read as Latin-1, redacted as text, control characters to `?`, clipped at 160 UTF-16 units
with `...`; omitted when there is none.

**Other lines**: section 3 (connections, handshakes, client errors), 9.1 (node handshakes), 10 (the feed), 12 (the
capture), 16 (start and stop). Warnings about a client are throttled per kind and address, once a minute, so a client
cannot flood the log.

## 15. Addresses (`ip`)

Strict single addresses only: no range, CIDR, zone id, leading zero or surrounding white space.

- **IPv4**: four dot-separated parts, each `0` or 1 to 3 digits without a leading zero, at most 255. `010.0.0.1`,
  `1.2.3`, `256.1.1.1`, ` 1.2.3.4`, `0x7f.0.0.1`, `1..2.3` fail.
- **IPv6**: 2 to 45 characters of hex digits, `:` and `.` only. A last group containing `.` is a dotted IPv4 tail (by
  the IPv4 rule) giving the last two words. At most one `::`, standing for at least one zero word; groups of 1 to 4
  hex digits; exactly eight words without `::`, at most seven known with it. `fe80::1%eth0`, `2001:db8::/64`,
  `2001:db8:::1`, `:1::`, nine groups, eight groups plus `::`, `g::1`, brackets and `::ffff:010.0.0.1` fail.
- `::ffff:a.b.c.d` (also written in hex) is that IPv4 address (a dual-stack peer).
- **Canonical text**: IPv4 dotted decimal; IPv6 per RFC 5952: lower case, no leading zeros, the longest run of two or
  more zero words (the first on a tie) as `::`. `2001:0DB8:0:0:0:0:0:0001` is `2001:db8::1`; `2001:db8:0:0:1:0:0:1`
  is `2001:db8::1:0:0:1`.
- **Classes**: IPv4 `0/8` unspecified, `127/8` loopback, `169.254/16` link-local, `224/4` multicast, `240/4` (with
  255.255.255.255) and `192.0.0/24` reserved, `10/8`, `172.16/12`, `192.168/16`, `100.64/10` and `198.18/15` private,
  the rest public. IPv6 `::` unspecified, `::1` loopback, `ff00::/8` multicast, `fe80::/10` link-local, `fc00::/7`
  and `fec0::/10` private, outside `2000::/3` reserved (NAT64 `64:ff9b::/96`, `::1.2.3.4`, `100::1`), the rest public.
- **Peer address** (`peer_address`): the socket's remote address canonicalized, or `unknown`.
- **Rate key** (`rate_limit_key`): the canonical IPv4 address, or the IPv6 address's first four words with the rest
  zero, canonical, plus `/64` (`2001:db8:1:2:aaaa::1` is `2001:db8:1:2::/64`); text that does not parse is its own
  key.
- **The allowlist check** (`check_allowlist_ip(text, allow_private)`), used by the helper: no text (not a string) is
  `not_a_string`; text containing `/`, `,`, `-` or `*`, or equal case-insensitively to `any`, `localsubnet`, `dns`,
  `dhcp`, `wins`, `defaultgateway`, `internet`, `intranet` or `localhost`, is `range_not_allowed`; not an address is
  `not_an_ip`; unspecified `unspecified`, multicast `multicast`, reserved `reserved`; private, loopback and
  link-local are `private` unless allowed; public passes. A pass returns the parsed address.

## 16. The command line, start and stop

`gateway serve [--env-file <file>]` (also the default with no subcommand) and `gateway make-cert <args...>` (section
2.2; its arguments are passed through as they are, including `--help`). Without `--env-file`, a `.env` in the
working folder is read if it is a file. Every fatal error is one `error` line with the message and exit code 1:
settings as `Configuration: <error>`, the env file as `Configuration: <error>`, the capture as `Traffic capture:
<error>`, a runtime that cannot start, and any start error.

At start (`server::run_with_network`), the service status goes `starting`; the TLS settings load (public mode;
failure fails the status and the start); the feed is set up; the listener binds `GATEWAY_BIND:GATEWAY_PORT`
(`Could not listen on <bind>:<port>: <error>`); then, if node links are on, an info line `node links on` `{node,
region, fingerprint, certificateValidUntil}` and, within 30 days of the node certificate's expiry, a warning `the node
certificate expires within 30 days; issue a new one (\`network issue-node-cert\`)` with the same fields; the node
links are left in the caller's slot. The status goes `ready` with the listen address and `TLS` or `HTTP`, and an info
line `gateway ready` with, in order: `listen`, `mode` (`public (TLS)` or `private (plain HTTP inside the tailnet)`),
in public mode `fingerprint`, `certificateNames`, `certificateValidUntil`, then `backend`, `content`, `websocket`
(authorities), `allowlistFeed` (the helper URL or `off`), `maxBodyBytes`, `maxConnections`, `maxConnectionsPerIp`,
`rate` (`general`, `content`, `register`, `token`, `ws`, `connect`, each `{burst, perMinute}`).

At stop: status `stopping`, info `stopping` `{signal: "stop", graceMs}`, the shutdown of section 3.4, status
`stopped`, info `stopped`. The `run_*` entry points differ only in what they attach: the in-process sink, the module
routes (else the stand-in), the node-link slot, and the region handler.
