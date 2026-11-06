# Clean-Room Behaviour Specification: `crates/rustic-isles-reforged`, the admin half (RD27)

> **Scope and purpose (RD27):** unit 9 of [`spec-rustic-isles-core.md`](spec-rustic-isles-core.md) §16, the MIT crate
> `rustic-isles-reforged`, has two specs because the area is about 13 000 lines. This one is the **admin half**: the
> admin API under `/admin/v1` (its authentication, sign-in with auth's TOTP, the manager operator credential, players,
> roles, deletion, audit), admin Discord sign-in, tester max, the release gates probe, the host-only
> `/reforged/api/*` routes, the admin actions of core §7.6, the home operations of core §7.8, and the fate of
> `bootstrap.rs` (core §17 item 10). Its sibling, [`spec-rustic-isles-reforged-public.md`](spec-rustic-isles-reforged-public.md),
> covers the probes, metrics, the request log `Observer`, the launcher-facing API, updates, logs, notices, activity,
> operations, Guard and auth's `Seams`. §1.2 fixes the boundary file by file; §15 the calls between the halves.
>
> The source is dainty-restricted's `modules/rustic-isles/crates/isles-core` (AGPL-3.0-only with Mystic Paradox's
> section 7 terms) and, where the Rust port is missing a contract a client of this repository depends on, the Node
> reference it ports (`reference/1.14.7/ParadoxBackend`, same licence). Everything below is stated from observable
> contracts: routes and bytes, store calls by the store spec's names, settings, logs, tests. No implementation code is
> reproduced and no private helper is named; Node and Rust file names appear only as provenance.
>
> Written against dainty-restricted `e0e238b4434316bb210677d88bc59e3448688794` (worktree
> `splashloader-integration-90130d`; this half's files last changed in `9ad97c6`, `2e0428d`, `0c6747b`, `2824b6d`,
> `7cd9db0`) and dainty-lass `9753bd210b37d88f1008ca0d38fdfbafcd9bb8b1` (branch `feat/rustic-isles-clean-room`).

---

## 1. Provenance, classification and the split

### 1.1 Evidence used

| Source | What it says |
| --- | --- |
| File headers and `git log` | Every file of this half is the maintainer's (one author); `admin.rs`, `admin_discord.rs`, `tester_max.rs`, `release_gates.rs`, `host_routes.rs` port `routes/admin.ts`, `controllers/admin.ts`, `middleware/HasAdminAuth.ts`, `controllers/adminDiscordAuth.ts`, `controllers/testerMax*.ts`, `observability/releaseGates.ts`, `routes/reforgedApi.ts` (`HostOnly`); `bootstrap.rs` ports `scripts/bootstrap_admin.ts` (RB11, `2824b6d`); `module.rs`'s admin and home wiring is DN2, DN5 and RD25 |
| `tests/reforged_admin.rs` (restricted) | The authored tests of the admin routes (the notices, Guard, player-logs, operations and activity tests in the same file are the public half's) |
| `security/managerOperator.ts`, `HasAdminAuth.ts`, `routes/admin.ts`, `app.ts` (Node reference) | The manager operator credential, which restricted's Rust does not have yet ("Not here yet" in `admin.rs`'s header), and the router-wide admin middleware |
| dainty-lass `crates/admin/src/upstream.rs`, `ui/apps/web/src/lib/game/backend.ts`, `crates/hostkit/src/hostconfig.rs`, `crates/hostkit/src/invite.rs` | The MIT clients of this half: the admin service and the dashboard sign every `/admin/v1` call as the manager operator; `setup` writes `RUSTIC_ISLES_MANAGER_ADMIN=on` and `RUSTIC_ISLES_MANAGER_OPERATOR_SECRET` into `backend.env`; `reforged-host invite` calls the host-only invite routes |
| `spec-rustic-isles-core.md`, `-auth.md`, `-store.md`, `-chat.md` | The pipeline and admin gate (core §3.2), §7.6 and §7.8, auth's TOTP, passwords, limiter and Discord client, the repositories, the `chat.*` home operations |

Consequence of the third row: on a DaintyLass host today, restricted answers every manager-signed `/admin/v1` call 401
`ADMIN_UNAUTHORIZED`, so the dashboard's game pages and the admin bot's moderation cannot reach the backend. The
rewrite implements the credential (§3.3). This is parity with Node, where the Rust port is the bug (AGENTS.md).

### 1.2 The split, file by file (restricted `crates/isles-core/src/`)

| File | Half | Notes |
| --- | --- | --- |
| `areas/reforged_api/admin.rs` | **admin** | The wrapper, `HasAdminAuth`, sign-in, players, roles, deletion, audit, online players (§3, §4, §6) |
| `areas/reforged_api/admin_discord.rs` | **admin** | §5 |
| `areas/reforged_api/tester_max.rs` | **admin** | §7 |
| `areas/reforged_api/release_gates.rs` | **admin** | §8 |
| `areas/reforged_api/probes.rs` | split | The `/health/release-gates` handler and its gate collection are **admin** (§8); every other probe is public |
| `areas/reforged_api/host_routes.rs` | **admin** | §9 (`HostOnly`, `x-reforged-admin-key`) |
| `module.rs`: `handle_admin`, its player resolution and restriction view | **admin** | §10; the `until` validation before the hook is core's (§7.6) |
| `module.rs`: `handle_home_invocation`'s operations | **admin** | §11; the preconditions before the hook are core's (§7.8); `chat.*` are the chat crate's (chat §5.9) |
| `bootstrap.rs` | **admin** | §12 |
| `areas/reforged_api.rs` | public | Registration order (both halves register into it) |
| `activity.rs`, `admin_guard.rs`, `guard.rs`, `guard_manifest.rs`, `http_get.rs`, `launcher_api.rs`, `launcher_auth.rs`, `logs.rs`, `metrics.rs`, `notices.rs`, `observe.rs`, `operations.rs`, `player_logs.rs`, `presence.rs`, `rules.rs`, `updates.rs`, `util.rs` | public | The admin-gated routes among them (activity, Guard views, notices, operations, player logs) run inside this half's admin wrapper (§15.1); `launcher-logs` (B197-B198) does not (its own sync token) |

Route rows (core Appendix A) answered by this half: **B4**; **B177-B181**; **B199-B209**; **B221**. Rows of the public
half that call this half's admin wrapper: B210-B213, B214-B216 (Guard views), B217-B220, B222-B224. Rifts rows whose
path starts `/admin/` (B240-B243) are relayed by core with this half's authenticator (§3.5).

### 1.3 Classification

All of this half is Category B (ports Paradox TypeScript) except `module.rs`'s wiring (Category A, DaintyLass's
DN rows) and `release_gates.rs`'s active-play gate (DaintyLass's). The implementer writes all of it from this spec
(core §17 item 11).

### 1.4 What stays in dainty-restricted

- The files above, until restricted's composer drops them for this crate; the goldens; `reference/`.
- The legacy names: the `mysticparadox_admin` session cookie and the `mysticparadox_admin_oauth` binding cookie are
  read by restricted "for one release"; the MIT crate reads neither (core §17 item 2).
- Restricted's process-wide registries (`set_credentials`, `set_chat_sessions`, the attempt windows, the tester-max
  busy set): the MIT crate keeps each per module or calls the auth crate directly (§2.4).

---

## 2. Conventions

### 2.1 JSON, text, times

- JSON as `JSON.stringify` writes it (`service_kit::json::Json`, `content-type: application/json; charset=utf-8`,
  insertion order; the crate enables `serde_json`'s `preserve_order` in its own manifest, core §17). Key orders below
  are exact.
- **Empty** answers have a status, no body, no content type. **Error page** is `service_kit::body::error_page(status)`.
- ISO texts are `Date#toISOString` (`YYYY-MM-DDTHH:MM:SS.mmmZ`), comparisons of ISO texts are text comparisons.
- **JS trim** is `service_kit::js::trim`. "First N characters" counts Unicode scalar values (restricted's rule; Node
  counted UTF-16 units, §19 item 1).
- **Admin error envelope**: `{"error":{"code":<code>,"message":<message>}}`. **Internal**: an empty 500, logged
  `[ADMIN] {error}` at error level.

### 2.2 Clocks and ids

| What | Clock / source |
| --- | --- |
| Records' times (sessions, audit, access changes, timeouts, tester-max writes, home operations) | core's replay-aware clock (core §9), as restricted |
| Trust decisions: session expiry lookup, the attempt windows of §2.4, the TOTP step, the operator timestamp window and nonce TTL | **the system clock**, never the replay clock (a security departure, §20 item 1) |
| Session ids, audit ids, `requestId`s, Discord nonces, binding keys, PKCE verifiers, opaque tokens | system randomness (UUID v4, 32-byte randoms); never core's replay id source |

### 2.3 Reading a request

- **Body**: up to 100 KiB, read as raw bytes, then parsed with core §3.9's rules (JSON and forms; 413, 415, 400 error
  pages). The raw bytes are kept for the operator's body hash (§3.3). A body that does not parse to a JSON object
  reads as no fields ("fields" below); a field's absence and JSON `null` differ only where said.
- **Query**: split on `&`, each pair at the first `=`, `+` as space, percent-decoded leniently (invalid UTF-8 replaced).
  "single" means the name appears exactly once (a repeated name reads as absent, Express's array); "joined" means every
  value joined with `,` (`String(array)`).
- **Header** "as text": the first value when it is valid visible ASCII/UTF-8 text; otherwise absent.
- **Client address**: auth §4.6 `client_ip` (`unknown` with no peer). **Peer**: the TCP peer address only.

### 2.4 Attempt windows

The sliding windows of Node's `IsRateLimited` are auth's limiter (`Auth::limiter().is_limited_at(key, max, window_ms,
now)`, auth §4.7: an allowed attempt is recorded, a refused one is not; per module; bounded) with the system clock.
This half's keys:

| Key | Max | Window |
| --- | ---: | --- |
| `admin-login:{client address}` | 6 | 15 min |
| `admin-discord-start:{client address}` | 12 | 15 min |
| `admin-discord-callback:{client address}` | 10 | 15 min |

Every attempt counts, successful ones included.

### 2.5 Eligibility (the launcher API's rule)

An account is **eligible** when its status is `active`, its approval is `approved` or null, and `username_set` is not
stored false. A timeout is **not** part of it here (unlike auth §6.1's play eligibility): a timed-out administrator
still signs in to `/admin/v1`. Both halves use the one rule (§15.2).

### 2.6 Actors (store §2.3)

| Who | `module_api::Actor` |
| --- | --- |
| An admin caller (session or manager) | `Admin { key: <caller userId> }` (`manager:<id>` for the manager) |
| The host routes (§9) | `Console` |
| Session revocation inside `HasAdminAuth`, Discord's transaction and account resolution | `System` |
| Tester max's progression writes | `Player { account: <target's global id> }` |
| Admin actions (§10) | the call's own actor |
| Home operations (§11) | `Player { account: <the invocation's account> }` (the invocation's own actor is ignored, §19) |

---

## 3. Admin authentication (the trust boundary)

### 3.1 The admin wrapper (every admin-gated handler of either half)

Core answers an empty 404 for `/admin` and `/admin/...` when the admin API is off (core §3.2, §6.3), before any of
this. With it on, every handler registered for a row under `/admin/v1` (except `launcher-logs`) runs inside the
wrapper, in this order:

1. **Body** (§2.3). A body error answers its error page, without admin headers.
2. **Origin check**. When the request has an `Origin` header (as text) that is not in the allowed list, answer an
   empty 403 (no admin headers), logging warn `[ADMIN] rejected origin {origin}`.
3. **No database**: core's 501 envelope (core §3.6) for the row, with the admin headers.
4. **Authentication** when the route asks for it (every route but `POST /admin/v1/auth/login` and the two Discord
   routes): §3.2 or §3.3; a refusal is answered with the admin headers.
5. The handler; its answer gets the admin headers.

**Allowed origins**: the raw `ADMIN_ALLOWED_ORIGINS` split on `,`, each JS-trimmed, empties dropped. Unset or blank
(after trim): none when `NODE_ENV` is exactly `production`, else `http://localhost:4173` and
`http://localhost:5174`. Compared exactly (case-sensitive).

**Admin headers** (set, replacing any): with an allowed `Origin`: `access-control-allow-origin: <origin>`,
`access-control-allow-credentials: true`, `access-control-allow-headers: Content-Type, X-CSRF-Token`,
`access-control-allow-methods: GET, POST, PUT, PATCH, DELETE, OPTIONS`, `vary: Origin`; always
`cache-control: no-store` and `content-security-policy: default-src 'none'; frame-ancestors 'none'`.

`OPTIONS` and unmatched `/admin/v1` paths are not answered by the wrapper (core §15 item 3; §20 item 4).

### 3.2 The session cookie

When the manager credential does not apply (§3.3 step 0):

1. **Cookie** `rustic_isles_admin`: the `Cookie` header (as text) split on `;`, each part trimmed, split at its first
   `=` (no `=`: the part is the name, the value empty); the first part whose name is exactly the cookie's; its value
   decoded with `decodeURIComponent` (`service_kit::js::decode_uri_component`). A value that does not decode: an
   **error page 500** (Node threw). Absent or empty: 401 `ADMIN_UNAUTHORIZED` `Administrator sign-in required.`
2. **Session**: `admin.find_active_session_by_token_hash(<SHA-256 hex of the value's UTF-8>, <now ISO>)`. None: 401
   `ADMIN_UNAUTHORIZED` `Administrator session expired.` A store error: internal.
3. **Account**: `launcher_accounts.find_by_account(session.account)`. Missing, without the `admin` role, or not
   eligible (§2.5): `admin.revoke_session(session.id, now)` (actor `System`; its failure ignored), then 403
   `ADMIN_FORBIDDEN` `Administrator access denied.`
4. **CSRF**: a method other than `GET`, `HEAD`, `OPTIONS` must carry `x-csrf-token` (as text) equal to the session's
   `csrf_token`, compared in constant time (an improvement that changes no answer); else 403 `ADMIN_CSRF`
   `Invalid request token.`
5. The **caller**: the session, the account, and the userId = `accounts.find(session.account)`'s `user_id`, else the
   account id text.

### 3.3 The manager operator credential (Node `managerOperator.ts`; missing in restricted)

The admin service (`crates/admin`) and the dashboard sign `/admin/v1` calls for a staff member; no cookie, no CSRF
token. Step 0: the credential applies when the request carries **any** header whose lower-cased name starts
`x-paradox-operator-` **and** `RUSTIC_ISLES_MANAGER_ADMIN` reads on (trimmed, lower-cased `on`; any other value,
blank included, is off at request time). Otherwise the request goes to §3.2 and the operator headers are ignored.

Checks, in this order; the first that fails answers (admin envelope, admin headers) and logs warn
`[ADMIN] manager operator request refused: {code} {METHOD} {path}` (path without the query; never a header value):

| # | Check | Refusal |
| --- | --- | --- |
| 1 | The **peer** (not the client address) is loopback: `::1`, or dotted `127.x.x.x`, optionally `::ffff:`-prefixed | 403 `ADMIN_OPERATOR_PEER_REFUSED` `Operator requests are accepted only from the loopback interface.` |
| 2 | No header whose lower-cased name starts `x-forwarded-`, nor `x-real-ip`, `forwarded`, `via` | 403 `ADMIN_OPERATOR_FORWARDED` `Operator requests must not pass through a forwarding proxy.` |
| 3 | `x-paradox-operator-id`, `-timestamp`, `-signature` present, each exactly once; `x-paradox-operator-name` at most once; values trimmed; id `^\d{1,32}$`, timestamp `^\d{1,15}$`, signature `^[0-9a-fA-F]{64}$` | 401 `ADMIN_OPERATOR_INVALID` `The operator credential is malformed.` |
| 4 | `RUSTIC_ISLES_MANAGER_OPERATOR_SECRET`, JS-trimmed, has at least 32 UTF-16 units | 503 `ADMIN_OPERATOR_NOT_CONFIGURED` `The operator credential is not configured on this server.` |
| 5 | `now − timestamp` (system clock, ms) is at most 60 000 and `timestamp − now` at most 5 000 | 401 `ADMIN_OPERATOR_STALE` `The operator request timestamp is outside the accepted window.` |
| 6 | A body is present (`transfer-encoding`, or `content-length` other than `0`) only with a JSON content type whose bytes were read (§2.3) | 400 `ADMIN_OPERATOR_BODY` `An operator request with a body must send Content-Type application/json.` |
| 7 | The signature, lower-cased, equals HMAC-SHA256 (key: the trimmed secret's UTF-8) over the canonical string, hex, compared in constant time | 401 `ADMIN_OPERATOR_SIGNATURE` `The operator signature does not match the request.` |
| 8 | The lower-cased signature was not accepted in the last 120 s (per-module nonce set; expired entries dropped at each check; only verified signatures enter) | 401 `ADMIN_OPERATOR_REPLAY` `This operator request was already accepted.` |

**Canonical string**: five lines joined by `\n`, no trailing newline: the method upper-cased; the request target
(path and query exactly as received; §20 item 5); the timestamp as sent (trimmed); the operator id as sent
(trimmed); the lower-case SHA-256 hex of the raw body bytes (no body:
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`). This is byte for byte what
`crates/admin`'s `operator_signature` computes.

**Accepted**: the caller is a synthetic administrator, never stored: userId `manager:<id>`; display name = the name
header with U+0000-U+001F and U+007F removed, cut to 64 UTF-16 units, JS-trimmed, `operator` when empty; normalized
name its lower case; status `active`; approval `approved`; roles `["admin"]`; `username_set` true; `created_at` now's
ISO; no email; no session. The CSRF check does not apply.

**Start-up** (this crate's start-up, core §7.3 step 5): with `RUSTIC_ISLES_MANAGER_ADMIN` on and the secret shorter
than 32 units (or blank), the module's start fails with `RUSTIC_ISLES_MANAGER_OPERATOR_SECRET must contain at least
32 characters when RUSTIC_ISLES_MANAGER_ADMIN=on.` (Node's `AssertManagerOperatorConfig`). An invalid switch value
already fails core's admin API reader (core §6.3).

### 3.4 The caller

`AdminCaller { user_id, account, session: Option<session>, via: Session | Manager }`. Its `Debug` shows `user_id`,
`via` and the account's id only: never the session's token hash or CSRF token.

### 3.5 Rifts admin routes (B240-B243)

Core relays rows whose path starts `/admin/` with auth's caller kind `Admin` (auth §4.4). For those, core calls this
half's **header authenticator** (§16.2) with the method, request target, peer, headers and the raw body (core reads it,
at most 100 KiB, for the relay anyway): the origin check of §3.1 (a refused origin: empty 403), then §3.3 or §3.2.
A refusal is answered with the admin headers; success gives core `Caller::Admin { key_id: <caller userId> }`.
No database: the caller is anonymous (restricted's rule; rifts then refuses). On success the admin headers are
**not** added to rifts' answer (restricted's behaviour; §19 item 2).

---

## 4. Sign-in routes

### 4.1 `POST /admin/v1/auth/login` (B199; no authentication)

1. Attempt window `admin-login:{client address}` (§2.4) refused: 429 `ADMIN_RATE_LIMITED` `Too many sign-in attempts.`
2. `email`: a string field, JS-trimmed, lower-cased (Unicode); else empty. `password`: a string field, else empty.
3. `launcher_accounts.find_by_email(email)`. A store error: internal.
4. Refused with 401 `ADMIN_INVALID_CREDENTIALS` `Administrator credentials are invalid.` when: no account; auth's
   `verify_password(account's hash, password)` is false; the account lacks the `admin` role; or it is not eligible
   (§2.5). (No account answers without verifying a dummy hash: §18 weak spot.)
5. **TOTP** (auth §5.5): `ADMIN_TOTP_SECRET` (`get`, so empty is unset). Unset and `NODE_ENV` exactly `production`:
   503 `ADMIN_MFA_NOT_CONFIGURED` `Administrator MFA is not configured.` Set: auth's `verify_totp(secret, <the body's
   totp field as JSON, or absent>, <system now>)` false: 401 `ADMIN_MFA_INVALID` `The authenticator code is invalid.`
   Unset outside production: no second factor.
6. **Session**: raw token and CSRF token are each 32 random bytes as lower-case hex (64 characters);
   `admin.create_session({ id: UUID v4, token_hash: SHA-256 hex of the raw token, account, csrf_token, created_at: now,
   expires_at: now + 8 h, ip: client address, user_agent: the header as text cut to 300 characters, or "", revoked_at:
   none })` with actor `Admin { key: <the account's userId> }` (userId = game account's, else the id text). A store
   error: internal.
7. 200 `{"csrfToken":<csrf>,"account":<public account>}` with
   `set-cookie: rustic_isles_admin=<raw token, encodeURIComponent>; Path=/admin/v1; HttpOnly; SameSite=Strict; Max-Age=28800`
   plus `; Secure` when `NODE_ENV` is exactly `production` or the raw `ADMIN_COOKIE_SECURE` is `1` or `true` (any case).

### 4.2 `POST /admin/v1/auth/logout` (B200)

Manager caller: 200 `{}`, no cookie. Session caller: `admin.revoke_session(session.id, now)` (admin actor; error:
internal), then 200 `{}` with `set-cookie: rustic_isles_admin=; Path=/admin/v1; HttpOnly; SameSite=Strict; Max-Age=0`
(plus `; Secure` as above).

### 4.3 `GET /admin/v1/me` (B203)

200 `{"csrfToken":<the session's, or null for the manager>,"account":<public account of the caller>}`.

### 4.4 The public account

From a launcher account and a userId, keys in this order, optional ones left out when none:

`userId`, `displayName`, `email` (`""` when none), `approvalStatus` (`pending`/`approved`/`rejected`, null reads
`approved`), `status` (`active`/`banned`/`disabled`), `roles`, `rolesUpdatedAt`?, `rolesUpdatedBy`?, `usernameSet`
(true unless stored false), `createdAt`, `lastLoginAt`?, `approvalUpdatedAt`?, `approvalUpdatedBy`?,
`approvalReason`?, then only while a timeout is in force at now (store's `timeout_at`): `timeoutUntil`,
`timeoutSince`, `timeoutReason`? (the timeout keys are DaintyLass's addition over Node, kept).

The manager's synthetic account reads
`{"userId":"manager:<id>","displayName":<name>,"email":"","approvalStatus":"approved","status":"active","roles":["admin"],"usernameSet":true,"createdAt":<now>}`.

---

## 5. Admin Discord sign-in (B201, B202; no authentication)

A browser flow: both ends are top-level navigations answered with 302s.

### 5.1 Configuration (read at each request)

A **secure URL** reading of a setting `{VAR}` (raw, JS-trimmed):

| Rule (in order) | Error code |
| --- | --- |
| Blank | `{VAR}_NOT_CONFIGURED` |
| Contains `#`; no `://`; scheme (any case) not `http`/`https`; authority (up to the first `/` after `://`, the query split off at the first `?` first) empty or containing `@`; a bracketed host without `]`; an empty host (before `:`, or the bracketed part) | `{VAR}_INVALID` |
| Scheme not `https`, unless `NODE_ENV` is not exactly `production`, the scheme is `http` and the host (lower-cased) is `localhost`, `127.0.0.1` or `[::1]` | `{VAR}_MUST_USE_HTTPS` |

Its **origin** is the lower-cased scheme, `://`, the lower-cased authority (port kept as written); its **path** the rest
before `?` (`/` when none); its **query** the text after the first `?`. It is written back as origin + path +
(`?` + query when the query is non-empty).

- **Frontend** `ADMIN_FRONTEND_URL`: a secure URL with no `?` (else `ADMIN_FRONTEND_URL_INVALID`); its path gets a
  trailing `/` when it lacks one.
- **Redirect URI** `ADMIN_DISCORD_REDIRECT_URI`: a secure URL with no `?` whose origin equals the frontend's (else
  `ADMIN_DISCORD_REDIRECT_URI_INVALID`).
- **Client**: `DISCORD_CLIENT_ID` and `DISCORD_CLIENT_SECRET` (`get`), each required (else
  `ADMIN_DISCORD_NOT_CONFIGURED`). Checked before the URLs.
- **Allow list** `ADMIN_DISCORD_ALLOWED_IDS`: raw, trimmed; blank: `ADMIN_DISCORD_ALLOWED_IDS_NOT_CONFIGURED`; split on
  `,`, each trimmed, empties dropped; any entry not 17-20 ASCII digits: `ADMIN_DISCORD_ALLOWED_IDS_INVALID`;
  de-duplicated in order; none left: `..._INVALID`.

Order: client id, client secret, redirect URI, frontend, allow list. The configuration's `Debug` never shows the
secret.

A **frontend redirect** with an error code is the frontend URL with query `error=<code>` (form encoding: ASCII
letters, digits and `*-._` as they are, space `+`, every other byte `%XX` upper-case).

### 5.2 `GET /admin/v1/auth/discord/start`

1. Configuration; an error: log error `[ADMIN] Discord auth start failed: {code}`, answer 503
   `ADMIN_DISCORD_NOT_CONFIGURED` `Discord sign-in isn't configured safely on this server yet.`
2. Window `admin-discord-start:{client address}` refused: an empty 302 to the frontend redirect with `rate_limited`
   (no cookie).
3. **Browser binding**: 32 random bytes, base64url without padding (43 characters). **State**:
   `v1.<nonce>.<tag>`: nonce 24 random bytes as lower-case hex (48), tag the lower-case hex HMAC-SHA256 keyed with the
   binding's 32 decoded bytes over the bytes `v1`, NUL, nonce. **PKCE**: verifier = base64url (no padding) of 32
   random bytes; challenge = base64url (no padding) of SHA-256 of the verifier's ASCII.
4. `discord_oauth_transactions.create({ state, code_verifier, created_at: now, expires_at: now + 5 min, every other
   field none })`, actor `System`. A store error: logged as step 1, the same 503.
5. An **empty 302**, `location: https://discord.com/api/oauth2/authorize?<params>` with, in order, `client_id`,
   `redirect_uri` (the written-back redirect URI), `response_type=code`, `scope=identify`, `state`, `code_challenge`,
   `code_challenge_method=S256`, each name and value form-encoded as above; and
   `set-cookie: rustic_isles_admin_oauth=<binding>; Path=/admin/v1/auth/discord; HttpOnly; SameSite=Lax; Max-Age=300`
   (plus `; Secure` under §4.1's rule).

### 5.3 `GET /admin/v1/auth/discord/callback`

Every answer carries the **burn cookie**
`rustic_isles_admin_oauth=; Path=/admin/v1/auth/discord; HttpOnly; SameSite=Lax; Max-Age=0` (`; Secure` as above).
"Redirect with X" is an empty 302 to the frontend redirect with error `X` and the burn cookie; "redirect home" is the
frontend URL with no query.

1. Configuration error: log error `[ADMIN] Discord auth callback configuration failed: {code}`, answer the 503 of §5.2
   step 1 with the burn cookie.
2. Window `admin-discord-callback:{client address}` refused: redirect with `rate_limited`.
3. `code` and `state` must both be single query values (an `error=` from Discord leaves `code` absent): else redirect
   with `discord_cancelled`.
4. `discord_oauth_transactions.consume_by_state(state, now)` (actor `System`): none: redirect with `discord_expired`; a
   store error: log error `[ADMIN] Discord auth completion failed: {error}`, redirect with `discord_failed`.
5. **Binding**: the `rustic_isles_admin_oauth` cookie (§3.2's parsing; a value that does not decode reads absent).
   The state verifies when the binding is 43 characters of `A-Z a-z 0-9 _ -` decoding to 32 bytes, the state is `v1.`,
   a 48-character lower-case hex nonce, `.`, a 64-character lower-case hex tag, and the tag equals the one §5.2 step 3
   computes (constant time). Else log warn `[ADMIN] rejected Discord OAuth state without its browser binding
   ip={client address}` and redirect with `discord_state_mismatch`. (The transaction is consumed before this check:
   a mismatched attempt burns the state; §19 item 3.)
6. **Exchange** through auth's Discord client (`DiscordApi::exchange_code`, auth §7.5.5) with the client id, secret,
   written-back redirect URI, `code` and the transaction's verifier. Any failure, or a user id that is not 17-20 ASCII
   digits: redirect with `discord_cancelled`.
7. Not in the allow list: log warn `[ADMIN] rejected Discord admin login from disallowed id {id}`, redirect with
   `discord_not_allowed`.
8. **Resolution** (§5.4). Ineligible: log warn `[ADMIN] rejected Discord admin login for ineligible linked account id
   {id}`, redirect with `discord_account_ineligible`. An error: log error as step 4, redirect with `discord_failed`.
   An admitted account without the `admin` role or not eligible: redirect with `discord_not_allowed`.
9. **Session** as §4.1 step 6, with user agent the header as text or `discord-oauth`, cut to 300, actor
   `Admin { key: <account's userId> }`. A store error: as step 4.
10. An empty 302 to the frontend (no error) with two cookies: the burn cookie, then the session cookie of §4.1 step 7.

### 5.4 Account resolution (one write, actor `System`)

- **A linked identity** (`auth_identities.find_by_provider_subject(<discord id>)`): its launcher account
  (`find_by_account`) missing: error `ADMIN_DISCORD_LINKED_ACCOUNT_MISSING`. Not eligible: **ineligible**. Has `admin`:
  admitted. Else **promotion**: roles + `admin` (appended), `launcher_accounts.update_roles(account, roles, now,
  "system:admin-discord-oauth")` (none: error `ADMIN_DISCORD_ACCOUNT_DISAPPEARED`); `admin.append_audit({ id: UUID,
  actor_user_id: "system:admin-discord-oauth", target_user_id: <account's userId>, action: "admin.discord.promote",
  old_state: {"roles":<before>}, new_state: {"roles":<after>}, reason: "Allow-listed Discord identity promoted an
  eligible linked account.", ip: client address, request_id: UUID, created_at: now })`; log info
  `[ADMIN] Eligible linked account promoted via Discord: {userId}`; admitted.
- **No identity: bootstrap.** Display name = `A` followed by the Discord id (a decimal number) in base 36, lower-case
  digits (`100000000000000001` → `Arcn1hssight`; 12-14 characters). A new account on this node (`AccountId::new`), a
  new random userId; `launcher_accounts.create` with email none, password hash none, status `active`, approval
  `approved` updated now by `system:admin-discord-oauth`, no reason, roles `["admin"]` updated now by the same, created
  now, `username_set` true, everything else none. Not `Created` (the name taken): error
  `ADMIN_DISCORD_BOOTSTRAP_CONFLICT`. `auth_identities.create({ subject: <id>, account, provider_username: <Discord
  username>, provider_avatar_url: https://cdn.discordapp.com/avatars/<id>/<avatar>.png when an avatar, linked_at: now
  })` false: error `ADMIN_DISCORD_IDENTITY_CONFLICT`. Audit `admin.discord.bootstrap` (as above, target the new userId,
  old state none, new `{"status":"active","approvalStatus":"approved","roles":["admin"]}`, reason
  `Allow-listed Discord identity bootstrapped the administrator account.`). Log info
  `[ADMIN] Administrator account bootstrapped via Discord: {userId}`. Admitted.

An error rolls the write back.

---

## 6. Players and audit (all authenticated)

Common: **page** = the joined `page` query (default `1`) read as `Number.parseInt(text, 10)` (leading white space, an
optional sign, leading digits; no digits is NaN); NaN or 0 is 1; then at least 1. **limit(default, max)** = the joined
`limit` read the same way, NaN or 0 is the default, then clamped to `1..=max`. **skip** = (page − 1) × limit.
**Target** by userId: the live launcher account whose game account has that userId (`accounts.find_by_user_id` then
`launcher_accounts.find_by_account`, else `launcher_accounts.find_by_user_id`). **Not found**: 404 `NOT_FOUND`
`Player not found.` **Reason**: a string `reason` field, JS-trimmed, first 500 characters; else empty.

### 6.1 `GET /admin/v1/online-players` (B204)

Active players over 60 s (§15.2: chat presence first, then recently active, each userId once, in that order). Each
looked up (game account by userId, then its launcher account); unknown ones skipped. Sorted by display name with
`service_kit::collate::locale_compare` (stable). 200
`{"count":n,"players":[{"userId":..,"displayName":..}...],"observedAt":<now ISO>,"source":"xmpp+presence"}`.

### 6.2 `GET /admin/v1/players` (B205)

- **`ids`** (joined) present and not blank: split on `,`, each trimmed, kept when 1-64 of `A-Z a-z 0-9 _ -`,
  de-duplicated in order, cut to 100. Each looked up in order (unknown skipped). 200
  `{"accounts":[<public account>...],"total":<found>,"page":1,"limit":<count of kept ids>}`.
- Otherwise a filtered page: `approvalStatus` (joined) `pending`/`approved`/`rejected` else no filter; `status`
  (joined) `active`/`disabled`/`banned`, or `timedOut` (no status filter; `timed_out_at` = now ISO), else none;
  `search` (single) JS-trimmed, first 100 characters (empty means no search, store's rule); page, limit(25, 100).
  `launcher_accounts.list_for_admin(filter)`, each with its userId (game account's, else the id text). 200
  `{"accounts":[...],"total":<filtered total>,"page":<page>,"limit":<limit>}`.

### 6.3 `PATCH /admin/v1/players/:userId/access` (B206)

1. `approvalStatus` (null counts absent): not one of the three texts: 400 `ADMIN_VALIDATION` `Invalid approval status.`
   Then `status` likewise: `Invalid account status.` Neither given: `No access-state change was supplied.`
2. Reason shorter than 3 characters: 400 `ADMIN_ACCESS_REASON` `Provide a reason of at least 3 characters for every
   server access change.`
3. The path userId equals the caller's userId and the change would reject, set pending, disable or ban (any approval
   other than `approved`, any status other than `active`): 409 `ADMIN_SELF_LOCKOUT` `You cannot reject, disable, or ban
   your own administrator account.`
4. Target (404 when none).
5. One `account_write` on the target with the admin actor: `launcher_accounts.set_access_state(account, { approval,
   status, approval_updated_at: now, approval_updated_by: <caller userId>, approval_reason: <reason> })` (none: 404);
   if the result is not eligible, `refresh_sessions.revoke_all_for_account` and
   `game_exchange_codes.revoke_unused_for_account` (now); `admin.append_audit({ action: "player.access.update",
   target_user_id: <path userId>, old_state: {"approvalStatus","status"} before, new_state: the same after, reason,
   ip, request_id: <fresh UUID>, created_at: now, actor_user_id: <caller userId> })`.
6. 200 `{"account":<public account of the result with the path userId>,"requestId":<that UUID>}`. `NotHome`: 409
   `ADMIN_NOT_HOME` `That player's account lives on another server.` Other errors: internal.

### 6.4 `PATCH /admin/v1/players/:userId/roles` (B207)

1. `roles` must be an array of strings: else 400 `ADMIN_VALIDATION` `roles must be an array of strings.` Every entry
   must be assignable (only `tester`): else 400 `ADMIN_VALIDATION` `Only these roles can be toggled here: tester.`
2. Target (404). New roles = the stored non-assignable roles in stored order, then the requested in request order
   (duplicates kept, §19).
3. One `account_write`: `launcher_accounts.update_roles(account, roles, now, <caller userId>)` (none: 404); if
   `tester` was held before and not after, `game_exchange_codes.revoke_unused_for_account`; audit
   `player.roles.update`, old `{"roles":..}`, new `{"roles":..}`, reason only when non-empty.
4. 200 `{"account":..,"requestId":..}`. Any error (`NotHome` included): internal (§19).

### 6.5 `DELETE /admin/v1/players/:userId` (B208)

1. Path userId = caller's: 409 `ADMIN_SELF_DELETE` `You cannot delete your own administrator account.`
2. Target (404).
3. `confirmation` must be exactly `DELETE <display name>`: else 400 `ADMIN_DELETE_CONFIRMATION`
   `Type "DELETE <display name>" to confirm permanent deletion.`
4. Reason shorter than 3: 400 `ADMIN_DELETE_REASON` `Provide a deletion reason for the audit log.`
5. One `account_write`: `admin.delete_player_data(account, userId, audit)` with audit `player.account.delete`, old
   `{"approvalStatus","status","roles"}`, new `{"deleted":true}`, the reason, a fresh request id; then
   `refresh_sessions.delete_all_for_account` and `game_exchange_codes.revoke_unused_for_account`.
6. 200 `{"requestId":..,"deletedCounts":{<name>:<count>...},"characterCount":<n>}`, counts in the store's order,
   `characterCount` the store's (the characters the account had). Restricted re-reads live characters after the
   soft delete and so always answers 0; Node answered the count; the rewrite answers the store's (§19 item 6).
   Errors: internal.

### 6.6 `GET /admin/v1/audit` (B209)

Page, limit(50, 100), `targetUserId` single (empty means all). `admin.list_audit(target, skip, limit)`. 200
`{"records":[<audit>...],"total":..,"page":..,"limit":..}`, each record with keys `id`, `actorUserId`,
`targetUserId`?, `action`, `oldState`?, `newState`?, `reason`?, `ip`, `requestId`, `createdAt` (optional ones left
out when none), newest first.

---

## 7. Tester max: `POST /admin/v1/players/:userId/tester-max` (B221)

Operator-triggered funding of a tester account from the kit file, idempotent per `runId`.

### 7.1 The route, in order

1. Target (404).
2. Without the `tester` role: 409 `ADMIN_TESTER_REQUIRED` `Only an account with the tester role can be maxed.`
3. `confirm` must be exactly `MAX <display name>`: else 400 `ADMIN_TESTER_MAX_CONFIRMATION` `Type "MAX <display
   name>" to confirm.`
4. `runId` a string of 8-64 characters, the first an ASCII letter or digit, the rest letters, digits, `.`, `_`, `-`:
   else 400 `ADMIN_TESTER_MAX_RUN_ID` `runId must be 8 to 64 characters: letters, digits, dot, underscore or hyphen.`
5. **Kit**: game data backend file `tester_max_kit.json` (core §8.1 `backend_file`). No folder, no file, not a regular
   file, unreadable: 503 `ADMIN_TESTER_MAX_KIT_MISSING` `Copy game-data/tester_max_kit.example.json to
   tester_max_kit.json and fill it in.` Not JSON (after dropping a leading BOM): 422
   `{"error":{"code":"ADMIN_TESTER_MAX_KIT_INVALID","message":"The tester kit cannot run.","problems":["tester_max_kit.json is not valid JSON"]}}`.
6. Validation (§7.2) with problems: 422 with the same envelope and the problems in order.
7. A run for this target userId already in progress in this module: 409 `ADMIN_TESTER_MAX_BUSY` `A max run for this
   account is already in progress.` (The busy mark is released after the run, whatever happened.)
8. The run (§7.3) with a fresh request id.
9. **Audit**, whatever the run did: action `player.tester_max` (new state: the summary) or `player.tester_max.failed`
   (new state `{"refused":<code>}`), old `{"roles":<target roles>}`, reason `tester max run <runId>`, the caller as
   actor; its write error is ignored.
10. Report: 200 `{"requestId":..,"report":<report>}`, log info `[TesterMax] run={runId} user={userId} status={status}
    actor={caller userId}`. Refusal: 409 `{"error":{"code":..,"message":..},"requestId":..}`, log warn
    `[TesterMax] run={runId} user={userId} refused: {code}`.

### 7.2 Kit validation (problems, in this order)

Not an object: only `the kit must be a JSON object`. Then, for each top-level key in order: `slayersPath` gives
`slayersPath was removed (16 Oct): a tester unlocks the Slayer's Path in game`; any key other than `comment`,
`progression`, `inventory`, `currencies` gives `unknown section "<key>"`. `comment` present, not null, not a string:
`comment must be a string`. Each section present and not null:

| Section | Must be | Per entry `[i]` (`at` = `<section>[<i>]`) |
| --- | --- | --- |
| `progression` | an array of at most 50, else `progression must be an array of at most 50 entries` | `progressionId` a string that is an approved XP track (§15.3), and not seen before (`{at}: {id} is listed twice`), else `{at}: "<String(value)>" is not an approved XP track (progression config)`; `targetProgress` `"max"` (then the track must have a top rank, else `{at}: {id} has no top rank, so "max" is not defined for it; give a number`) or a safe integer 1-10 000 000, else `{at}: targetProgress must be "max" or an integer from 1 to 10000000` |
| `inventory` | at most 200, else `inventory must be an array of at most 200 entries` | `catalogId` a known catalog id (the inventory storage policy) else `{at}: "<String>" is not in the inventory storage policy (do not invent catalog ids)`; known and starting `CURRENCY_`: `{at}: {id} is a currency; list it under currencies`; `quantity` a safe integer 1-1000 else `{at}: quantity must be an integer from 1 to 1000` |
| `currencies` | at most 50, else `currencies must be an array of at most 50 entries` | `catalogId` starting `CURRENCY_` and known, else `{at}: "<String>" is not a known CURRENCY_* id`; the Platinum aggregate `CURRENCY_PLATINUM`: `{at}: {id} is the Platinum aggregate; grant a storefront bucket (CURRENCY_PLATINUM_*) instead`; `quantity` 1-10 000 000 else `{at}: quantity must be an integer from 1 to 10000000` |

`<String>` is JS `String(value)` (`service_kit::js::string`: `undefined`, `null`, numbers as JS writes them).
"Safe integer" is `service_kit::js::safe_integer` (a JSON number that is integral within ±2^53−1; `5.0` counts).

### 7.3 The run

Parsing keeps only well-formed entries (progression: string id and `"max"` or a safe integer; stacks: string id and
safe integer).

1. The target's characters (`characters.find_many_by_account`; a store error reads as none); the first with a
   non-blank id. None: refusal `ADMIN_TESTER_MAX_NO_CHARACTER` `The tester has no character yet; the account must sign
   in to the game once first.`
2. Report `{ runId, userId: <path userId>, characterId, status: "done", progression: [], items: none }`.
3. **Each progression entry** in kit order. Start = the stored track's progress (`progression_tracks.get`, 0 when
   none); target = the cap XP for `max` (none: error `the track has no top rank`) or the number. **Plan**: from
   max(start, 0) up to target in blocks aligned to the grant ceiling (100 000; the max a single grant may carry): a step
   `(index, amount)` covers `[s, min(target, (index + 1) × ceiling))` with `index = s / ceiling`; nothing when start ≥
   target. For each step, grant id = `tmax-` + the first 40 hex characters of SHA-256 of
   `<runId>` NUL `xp:<progressionId>:<index>`; the grant (one `account_write`, player actor): an approved track
   (else error `Unsupported progression track '<id>'.`); request hash = SHA-256 hex of the JSON text
   `{"version":1,"userId":<path userId>,"progressionId":<id>,"amount":<amount>}`; an existing
   `progression_grants` row with that id must match account, track, amount and hash (else data error `Progression grant
   <grant id> was already used with a different request.`) and is then skipped; otherwise
   `progression_tracks.increment` and `progression_grants.insert_applied` with the result. Each step adds its amount
   to `granted` and 1 to `grants` (also when skipped as already applied). Then the rank for max(start, target)
   (§15.3), when above 0, is **confirmed**: its required XP (unknown track: error `rank <r> confirm refused:
   unknown_track`; out of range: `rank <r> confirm refused: rank_out_of_range`); when above 0 and the stored progress is
   below it: data error `rank <r> confirm refused: insufficient_xp`; else
   `progression_tracks.set_confirmed_fremium_rank`; `confirmedRank` = r. Any error marks the row `error` with
   `error` = the error's display (store §2.4, so data errors read `stored data: ...`) cut to 200 characters, and the
   report `partial`; later entries still run.
4. **Items**: inventory and currency stacks merged by catalog id (quantities summed), ordered by catalog id (byte
   order). When any: one inventory transaction (§15.3) for the target's userId and character, transaction id
   `tmax-` + 40 hex of SHA-256 of `<runId>` NUL `items`, adds `[{"catalogId":..,"quantity":..}...]`, economy source
   `OPERATOR_TEST`, ownership already validated, operator test funding. Applied: items `{status: done, stacks: n}`;
   not owned: `{status: error, error: "the inventory transaction did not apply"}`; an error: its text cut to 200; the
   latter two make the report `partial`.

**Report JSON**: `{"runId","userId","characterId","status","progression":[{"progressionId","target","startProgress","granted","grants","confirmedRank"?,"status","error"?}...],"items"?:{"status","error"?,"stacks"}}`.
**Summary** (the audit's new state): `{"runId","status","progression":[{"progressionId","target","startProgress","granted","grants","confirmedRank":<n or null>,"status"}...],"items"?:{"status","stacks"}}`.

---

## 8. Release gates: `GET /health/release-gates` (B4)

Mounted only with the admin API on (core: 404 otherwise); outside the request budget (core §3.2). No admin wrapper.

1. The **peer** must be exactly `127.0.0.1`, `::1` or `::ffff:127.0.0.1`, and the request must carry none of
   `x-forwarded-for`, `x-real-ip`, `forwarded`: else an empty 404.
2. Gates, in order: `no_active_play`, then `guard_mode`, `registration_approval_mode`, `tls_proxy_consistency`,
   `no_legacy_env_keys`, each `{"name","ok","detail"}`, read from the **env file map** only (core §6.1, not switches or
   forced values).
3. 200 when there is at least one gate and all are ok, else 503; body `{"ok":<bool>,"gates":[...]}`.

| Gate | Rule and detail texts |
| --- | --- |
| `no_active_play` | Active players over 2 minutes (§15.2). 0: ok `nobody in game`. Else, when the file `RELEASE_DURING_PLAY.allow` exists in the parent folder of the update root (§15.2): ok `{n} in game; held release let through by RELEASE_DURING_PLAY.allow`; else not ok `{n} player(s) in game within the last 2 minutes: release held until they stop (emergency override: RELEASE_DURING_PLAY.allow in the backend's shared folder)` |
| `guard_mode` | `RUSTIC_ISLES_GUARD_MODE` trimmed, lower-cased: empty `mode=observe (default)`; `observe` `mode=observe`; `enforce` `mode=enforce`; anything else not ok `RUSTIC_ISLES_GUARD_MODE has an unrecognised value (must be observe or enforce)` |
| `registration_approval_mode` | `RUSTIC_ISLES_REGISTRATION_APPROVAL` trimmed, lower-cased: empty `mode=off (default)`, `off` `mode=off`, `required` `mode=required`, other not ok `RUSTIC_ISLES_REGISTRATION_APPROVAL has an unrecognised value (treated as required)`; an ok detail gets `; RUSTIC_ISLES_REGISTRATION_AUTO_APPROVE is set and has no effect` when that variable is non-blank |
| `tls_proxy_consistency` | `RUSTIC_ISLES_TLS_TERMINATED_BY_PROXY` trimmed, lower-cased (empty and `REFORGED_HTTP_ONLY` one of `1 true yes on`: `on`). `""`/`0`/`off`/`false`/`no`: ok `proxy mode off (in-process HTTPS)`. `1`/`on`/`true`/`yes`: `BIND_HOST` (trimmed, lower-cased) empty: not ok `proxy mode on but BIND_HOST is unset (every interface)`; `127.0.0.1`, `::1`, `localhost`: ok `proxy mode on, BIND_HOST is loopback`; else not ok `proxy mode on but BIND_HOST is not a loopback address`. Other values: not ok `RUSTIC_ISLES_TLS_TERMINATED_BY_PROXY must be on or off` |
| `no_legacy_env_keys` | Env names starting `MYSTPAX_`, sorted: none: ok `0 MYSTPAX_ keys`; else not ok `{n} MYSTPAX_ key(s): {names joined ", "}` |

A detail carries names, counts, modes and booleans, never a value.

---

## 9. Host-only routes (`/reforged/api/*`, B177-B181)

The host kit's own calls (`reforged-host invite`, the admin service's kick). Core registers these prefixes as host-only
(core §7.2), so the gateway refuses them from outside. Each handler, in order: the body (§2.3; an error page first);
the **local admin check**; no database: core's 501 envelope.

**Local admin check** (else 403 `{"error":"forbidden"}`): `REFORGED_ADMIN_KEY` (raw) has at least 32 UTF-16 units; the
peer is `::1` or dotted `127.x.x.x` (each part 1-3 digits), optionally `::ffff:`-prefixed; the request carries neither
`x-reforged-gateway` nor `x-forwarded-for`; `x-reforged-admin-key` (as text) has the same SHA-256 as the setting,
compared in constant time.

**Launcher API rules used here** (owned by the public half's rules, restated): a **username** is a JSON string,
JS-trimmed, 3-16 UTF-16 units of ASCII letters, digits, `_`; an **invite code** typed by a host is 4-64 of ASCII
letters, digits, `-`; a new code is three groups of four from `ABCDEFGHJKMNPQRSTUVWXYZ23456789` joined by `-`, each
character uniformly random; an **invite note** is a string with U+0000-U+001F and U+007F-U+009F replaced by spaces,
JS-trimmed, first 64 characters, none when empty; the **not found** refusal is
`{"error":"not_found","message":"No account has that name."}`.

| Route | Contract |
| --- | --- |
| `POST /reforged/api/CreateInvite` | `uses`: absent or null is 1; must be a safe integer 1-1000. `infinite`: exactly `true` or not. `name`: absent or a string (null refused, §19). Else 400 `{"error":"bad_request","message":"uses is a whole number from 1 to 1000; name is text"}`. Up to 5 tries, each its own write (actor `Console`): `invites.insert_if_absent({ code: new, uses_remaining: uses, infinite_uses, note, created_at: now })`; inserted: log info `[launcher-api] invite made: {unlimited | <uses> use(s)}{ for <note>}`, 200 `{"code":<code>}`; a clash tries again; a store error: log error `[launcher-api] CreateInvite: {error}`, empty 500; five clashes: empty 500 |
| `GET /reforged/api/InviteCodes` | `invites.list(500)`; 200 `{"InviteCodes":[{"code","usesRemaining","infiniteUses","note":<text or null>,"createdAt"}...]}`; error: log `[launcher-api] InviteCodes: {error}`, empty 500 |
| `DELETE /reforged/api/InviteCode/:code` | After the check, the decoded `code` not an invite code: 400 `{"error":"bad_request"}`. `invites.delete(code)` (Console): empty 200 when one went, empty 404 otherwise; error: log, empty 500 |
| `POST /reforged/api/SetAccountStatus` | `Status` exactly `active`, `banned` or `disabled`, and `Username` a username: else 400 `{"error":"bad_request","message":"Username and Status (active, banned or disabled) are required."}` (the name is looked up only when the status is valid). Account by normalized (lower-cased) name (`find_by_display_name_normalized`; store error: log, empty 500); none: 404 not found. One `account_write` (Console): `set_access_state(account, { status, approval_updated_at: now, approval_updated_by: "host", approval_reason: <Reason string's first 500 characters, untrimmed, or none> })`. Then the status cache is forgotten (§15.2), log info `[launcher-api] {display name} ({userId}) set {status}`, 200 `{"UserId":..,"Username":<display name>,"Status":<the stored status after, or the requested one when none came back>}`. No session is revoked (§19). Error: log `[launcher-api] SetAccountStatus: {error}`, empty 500 |
| `POST /reforged/api/Kick` (RD25) | `Username` not a username: 400 `{"error":"bad_request","message":"Username is required."}`. Lookup as above (404 not found). One `account_write` (Console): `refresh_sessions.revoke_all_for_account` and `game_exchange_codes.revoke_unused_for_account` (now). Chat connections closed: core presence `close_connections(userId, 1008, "kicked by the host")` → n. Status cache forgotten. Log info `[launcher-api] {display name} ({userId}) kicked, {n} chat connection(s) closed`. 200 `{"UserId":..,"Username":..,"Closed":n}`. Errors: log `[launcher-api] Kick: {error}`, empty 500 |

The userId is the game account's, else the account id text.

---

## 10. Admin actions (core §7.6, the composer's admin hook)

Core has already refused without a database and validated and normalized a `Ban`'s `until` (core §7.6). This hook
answers the rest; every write is an `account_write` with **the call's actor**; a store error is
`ModuleError::failed(<its text>)`.

**Player resolution** (one read): the text trimmed (empty: `NotFound`). If it parses as an account `GlobalId`: the game
account with that id (`accounts.find`: userId and name), else the launcher account with that id (userId = the id text,
name = display name). Else the game account with that userId. Else the launcher account whose normalized name is the
text lower-cased (userId from its game account, else the id text; name = display name). None: `NotFound`.

| Action | Effect, in order | Answer |
| --- | --- | --- |
| `Kick` | `game_exchange_codes.revoke_unused_for_account` (sessions kept); chat closed 1008 `kicked by the host`; status cache forgotten | `Done` |
| `Ban` with `until` | `launcher_accounts.set_timeout({ until: <core's normalized text>, since: now, reason: <first 500 characters> })`, cache forgotten; then as `Kick` with reason text `timed out by the host` | `Done` |
| `Ban` without | `set_access_state({ status: banned, approval_updated_at: now, approval_updated_by: "host", approval_reason: <first 500, or none> })`, cache forgotten; `set_timeout(none)`, cache forgotten; `refresh_sessions.revoke_all_for_account` and exchange codes revoked; chat closed `banned by the host`; cache forgotten | `Done` |
| `Unban` | `set_access_state({ status: active, by "host", reason })`; cache forgotten (any timeout stays) | `Done` |
| `LiftTimeout` | `set_timeout(none)`; cache forgotten | `Done` |
| `Lookup` | `Player { account: <global id>, name, online, restriction }`: online when the userId is among the active players over 3 minutes (§15.2); restriction from the launcher account (none without one): `banned` → `Ban`, `disabled` → `Disabled` (reason = stored reason when non-empty, since = approval updated time, no until); else a timeout in force at now → `Timeout` (its reason, since, until); else none | `Player` |

Each effect is its own write (no single transaction across them; §19).

---

## 11. Home operations (core §7.8, the composer's home-operation hook)

Core has refused without a database, for an account of another node and for a non-account id. The hook dispatches by
name; `chat.login`, `chat.player`, `chat.party` go to the chat crate (chat §5.9); an unknown name is `NotFound`
`unknown home operation {name}`. Writes are `account_write`s with `Player { account }`; a store error is
`failed(<text>)`. `account` in answers is the invocation's global id text. Key order as written.

| Operation | Body | Effect and answer |
| --- | --- | --- |
| `progression.grant` | `xp` (a JSON integer in i64; anything else 0; negatives 0), `track` (a string, default `ExperienceTrack_PlayerLevel`) | `progression_tracks.increment(account, track, xp)` (0 still creates or touches the track); `{"ok":true,"account":..,"progression_id":<track>,"progress":<after>}` |
| `progression.read` | — | `get_all_for_account`; `{"tracks":[{"progression_id","progress","update_version"}...]}` |
| `inventory.write` | `loot`: array; its string entries | One write: the first character (`find_many_by_account`), else a new one: id `char-<account id text>`, name `Slayer`, `update_version` 1, data `{}`, created and modified now. Its inventory (`find_by_character_id`): stacked items parsed as a JSON array (unparseable: empty), instanced text kept, revision (none: 0); none: a new one. Each entry: `catalogId:qty` split at the first `:` (qty parsed as i64, unparseable 1, at least 1) or `catalogId` with 1; added to the stack with that `catalogId` (its quantity, non-integer read 0, plus qty) or appended `{"catalogId","quantity"}`. New: `inventories.create({ instanced "[]", stacked, revision 1, no bootstrap version, account })`; else `update_both_if_revision_matches(.., revision)`, whose none is ignored. `{"ok":true,"account":..,"loot_applied":<number of string entries>}` |
| `inventory.read` | — | The first character's inventory: `{"character_id","stacked_items":<parsed or []>,"instanced_items":<parsed or []>}`; none: `{"stacked_items":[],"instanced_items":[]}` |
| `bounty.write` | `bounty_progress` (any JSON, default `{}`) | One write: the stored state (`bounty_states.get`) or empty with a new `created_at`; append a base-group bounty `{ bounty: <it>, no claim, no credit }`; version = stored + 1 (1 when none); `save_if_version(record with both daily dates none, updated now, expected: <stored version or none>)`, whose none is ignored. `{"ok":true,"account":..,"update_version":<version>}` |
| `bounty.read` | — | `{"count":n,"bounties":[<each stored bounty's JSON>...]}` |
| `character.load`, `account.write` | — | No effect; `{"ok":true,"account":..,"operation":<name>}` |

These are DN5's cross-node test operations; their odd rules are listed in §19.

---

## 12. `bootstrap.rs`: decision

**Kept, as a library function of this crate and a proposed host command.** Reasons:

- It is the only way to make a **password** administrator, which `POST /admin/v1/auth/login` needs: the roles route
  toggles only `tester`, and Discord's bootstrap (§5.4) makes accounts without a password or email.
- The cookie sign-in is the documented fallback when the manager or Discord is down; without the function a fresh
  host has no way into it.
- It is small, offline, guarded by a confirmation word, and needs nothing at run time.

Against: the dashboard and admin bot reach `/admin/v1` through the manager credential (§3.3), which needs no account,
and Discord's allow list covers dashboard sign-in. That makes the function rarely needed, not unneeded.

**Contract** `bootstrap_admin(store, request) -> Result<outcome, error>` (one write, actor `Console`):

1. Request `{ email, from_email?, username?, confirm }`. `email` JS-trimmed and ASCII-lower-cased; `from_email`,
   `username` the same, blank as none. `email` empty or `confirm` not exactly `GRANT_ADMIN_ROLE`: **usage** error.
2. The account: with a username, `find_by_display_name_normalized(username)`; else `find_by_email(from_email or
   email)`. None, and `from_email` given and different from `email`: `find_by_email(email)` (a rerun after the rename).
   None: `Launcher account not found by username=<u>.` or `... by email=<from_email or email>.`
3. No password hash: `Matched account is not a password-based launcher account.` `username_set` stored false: `The
   launcher account must have a username before becoming an admin.`
4. Another live account holds `email`: `Cannot rename account: <email> is already in use.`
5. The account's email set to `email`; roles + `admin` when missing (appended); `set_access_state({ approval:
   approved, status: active, approval_updated_at: now (system clock), approval_updated_by: "bootstrap-script",
   approval_reason: "Initial administrator bootstrap" })`. Outcome `{ userId, email }`.

Usage text: `Usage: reforged-host admin bootstrap --email=you@example.com [--from-email=old@example.com |
--username=ExistingName] --confirm=GRANT_ADMIN_ROLE` (Node's named `npm run admin:bootstrap`, which no longer exists).
The subcommand itself is a `clap`-tree change for the integration pass (§20 item 7); the function needs a store method
to set an email (§20 item 8).

---

## 13. Store use (store spec names)

| Repository | Calls |
| --- | --- |
| `admin` | `create_session`, `find_active_session_by_token_hash`, `revoke_session`, `append_audit`, `list_audit`, `delete_player_data` |
| `launcher_accounts` | `find_by_account`, `find_by_user_id`, `find_by_email`, `find_by_display_name_normalized`, `list_for_admin`, `set_access_state`, `set_timeout`, `update_roles`, `create`; a new `set_email` (§12) |
| `accounts` | `find`, `find_by_user_id` |
| `auth_identities` | `find_by_provider_subject`, `create` |
| `discord_oauth_transactions` | `create`, `consume_by_state` |
| `refresh_sessions` | `revoke_all_for_account`, `delete_all_for_account` |
| `game_exchange_codes` | `revoke_unused_for_account` |
| `invites` | `insert_if_absent`, `list`, `delete` |
| `characters` | `find_many_by_account`, `create` |
| `inventories` | `find_by_character_id`, `create`, `update_both_if_revision_matches` |
| `progression_tracks` | `get`, `get_all_for_account`, `increment`, `set_confirmed_fremium_rank` |
| `progression_grants` | `find_by_grant_id`, `insert_applied` |
| `bounty_states` | `get`, `save_if_version` |

Through `IslesStore::read`, `write`, `account_write` (store §3.2); `NotHome` is store §2.4's.

---

## 14. Settings

All read from core's settings (`raw` unless `get` is said); English logs never print a value.

| Setting | Read | Use |
| --- | --- | --- |
| `NODE_ENV` | raw, exactly `production` | Origins default, cookie `Secure`, MFA required, Discord's https rule |
| `ADMIN_ALLOWED_ORIGINS` | raw | §3.1 |
| `ADMIN_COOKIE_SECURE` | raw, `1` or `true` (any case) | Cookie `Secure` |
| `ADMIN_TOTP_SECRET` (secret) | `get` | §4.1 |
| `RUSTIC_ISLES_MANAGER_ADMIN` | trimmed, lower-cased `on` | §3.3 (core reads it for the admin API too, core §6.3) |
| `RUSTIC_ISLES_MANAGER_OPERATOR_SECRET` (secret) | JS-trimmed, ≥ 32 units | §3.3; start-up check |
| `DISCORD_CLIENT_ID`, `DISCORD_CLIENT_SECRET` (secret) | `get` | §5 |
| `ADMIN_FRONTEND_URL`, `ADMIN_DISCORD_REDIRECT_URI`, `ADMIN_DISCORD_ALLOWED_IDS` | raw | §5.1 |
| `REFORGED_ADMIN_KEY` (secret) | raw, ≥ 32 units | §9 |
| `RUSTIC_ISLES_GUARD_MODE`, `RUSTIC_ISLES_REGISTRATION_APPROVAL`, `RUSTIC_ISLES_REGISTRATION_AUTO_APPROVE`, `RUSTIC_ISLES_TLS_TERMINATED_BY_PROXY`, `REFORGED_HTTP_ONLY`, `BIND_HOST`, `MYSTPAX_*` (names only) | env file map | §8 |

`setup` already writes `REFORGED_ADMIN_KEY`, `RUSTIC_ISLES_MANAGER_ADMIN=on` and the operator secret into
`backend.env` (`hostconfig.rs`); nothing here adds a variable.

---

## 15. Calls between the two halves

One crate, two halves; these are crate-internal contracts both implementers write against. Names are proposals the
two implementers keep in step.

### 15.1 This half provides

| Item | Contract |
| --- | --- |
| `admin_route(call, auth: Required \| Open, handler)` | §3.1's wrapper. The handler gets an `AdminContext { shared, store, request (head, query, raw body, fields), params, caller: Option<AdminCaller> }`; with `Required` the caller is always some |
| `AdminCaller` | §3.4; `admin_actor(&caller)` gives `Admin { key: user_id }` |
| `admin_error(status, code, message)`, `admin_internal(error)`, `player_not_found()` | §2.1, §6 |
| `page_of(request)`, `limit_of(request, default, max)`, `skip_of(page, limit)`, `parse_int(text)` | §6 |
| `user_id_of(account)` (game account's userId, else the id text), `find_target(userId)`, `public_account(account, userId)` | §6, §4.4 |
| `authenticate_admin(method, target, peer, headers, raw body)` | §3.5's header authenticator, for core's rifts relay and any admin path outside the wrapper |
| `collect_release_gates()` | §8's gates, for the public half's metrics or overview should it want them |

The public half's admin-gated handlers (B210-B220, B222-B224) use the wrapper with `Required` and take the caller's
userId for their audit actor and store actor. Operations' break-glass code calls auth's `verify_totp` itself.

### 15.2 This half needs from the public half

| Item | Contract |
| --- | --- |
| `recently_active(window_ms) -> Vec<userId>` | The activity monitor's "seen recently" map (per module; the userIds touched within the window, at the system or replay clock the public half uses) |
| `active_players(window_ms)` | Core presence's online userIds (core §5.5), then `recently_active(window)`, each once in that order. Used with 60 s (online players), 2 min (release gates), 3 min (Lookup) |
| `forget_status()` | Drops the launcher API's server-status cache (per module, core §17 item 14) |
| `update_root()` | `LAUNCHER_UPDATE_ROOT` raw as a path, else `updates`, made absolute against the process's working folder (the updates feed's root) |
| `is_account_eligible(account)` | §2.5's rule, owned by the launcher API |
| The launcher API rules | §9's username, invite code, invite note, `not_found` refusal |

### 15.3 This half needs from other crates

| From | Item |
| --- | --- |
| core (`rustic-isles`) | `Shared` (settings, node, game data root, presence, `auth()`), `Handlers::on`, `Call`, the 501 envelope, the admin gate, `ModuleHooks`, `Started` |
| auth (`rustic-isles-auth`) | `verify_password`, `verify_totp(secret, candidate, now_ms)`, `Auth::limiter().is_limited_at`, `client_ip`, the `DiscordApi` the composer gave auth's `Seams` (passed to this crate's constructor) |
| store (`rustic-isles-store`) | §13 |
| chat (`rustic-isles-chat`) | `home_operation` for `chat.*` |
| economy (unit 7, not yet specified) | A **tester funding port** the composer supplies until the economy crate lands: `is_approved_xp_track(id)` (the approved tracks, today's hunt pass included, that have a curve in `progression_config.json`), `track_cap_xp(id) -> Option<i64>`, `rank_for_progress(id, progress) -> Option<i64>`, `required_xp_for_rank(id, rank) -> Option<f64>` (none for an unknown track is distinguished from out of range), `is_known_catalog_id(id)` (an entry in `inventory_storage_policy.json`), `max_xp_grant()` (100 000), and `apply_operator_test_stacks(userId, characterId, transaction id, stacks) -> Applied \| NotOwned \| Failed(text)` (idempotent per transaction id through the inventory ledger) |

---

## 16. The seam (this half's part of the crate's interface)

### 16.1 Shape

`rustic-isles-reforged` (lib, MIT) in `crates/rustic-isles-reforged`; depends on `rustic-isles`, `rustic-isles-auth`,
`rustic-isles-store`, `rustic-isles-chat`, `module-api`, `service-kit`, and workspace `axum`, `http`, `serde`,
`serde_json` (own `preserve_order`), `tokio`, `tracing`, `sha2`, `hmac`, `subtle`, `base64`, `rand` or `uuid`,
`percent-encoding`, `chrono`, `thiserror`. No new workspace dependency beyond what auth already asks for. Seam
changes for the integration pass: `crates/rustic-isles-reforged` in `[workspace.dependencies]`; the `clap`
subcommand of §12; the store's `set_email` (§20 item 8); core's admin authenticator slot (§16.2).

### 16.2 Interface (admin half)

| Item | Contract |
| --- | --- |
| `register_admin(handlers)` | Registers B4, B177-B181, B199-B209, B221 (exact table paths) |
| `ReforgedAdmin::new(shared, store, discord, tester_funding)` | Per-module state: the nonce set (§3.3), the tester-max busy set; `Debug` without secrets |
| `admin_action(&self, AdminCall) -> Result<AdminResponse, ModuleError>` | §10, for `ModuleHooks::admin` |
| `home_operation(&self, &HomeInvocation) -> Result<Value, ModuleError>` | §11, for `ModuleHooks::home_operation` (delegating `chat.*`) |
| `startup()` | §3.3's start-up check, a composer start-up |
| `AdminAuthenticator` (a slot) | `authenticate(method, target, peer, headers, body) -> Result<AdminCaller, Response>`; **core** looks it up for rifts' `Admin` caller kind (a request to the core implementer: core §3.7 step 1 needs it; without it, anonymous) |
| `bootstrap_admin(&IslesStore, BootstrapAdminRequest) -> Result<BootstrapAdminOutcome, BootstrapError>` | §12; errors display the texts there |
| `TesterFunding` (trait) | §15.3 |
| feature `test-support` | A `TesterFunding` fake and a signer for operator headers (the canonical string of §3.3) |

### 16.3 Logs (English, never a value of a secret, token, cookie or code)

`[ADMIN] rejected origin {origin}`, `[ADMIN] {error}`, `[ADMIN] manager operator request refused: {code} {METHOD}
{path}`, `[ADMIN] Discord auth start failed: {code}`, `[ADMIN] Discord auth callback configuration failed: {code}`,
`[ADMIN] Discord auth completion failed: {error}`, `[ADMIN] rejected Discord OAuth state without its browser binding
ip={ip}`, `[ADMIN] rejected Discord admin login from disallowed id {id}`, `[ADMIN] rejected Discord admin login for
ineligible linked account id {id}`, `[ADMIN] Eligible linked account promoted via Discord: {userId}`, `[ADMIN]
Administrator account bootstrapped via Discord: {userId}`, `[TesterMax] ...` (§7.1), `[launcher-api] ...` (§9).

---

## 17. Test cases

### 17.1 What may be copied

Nothing recorded. `tests/reforged_admin.rs` and the unit tests of this half's files are authored test code, described
below; the tests use store's `TempStore` and auth's `StubDiscord`.

### 17.2 Behaviour the authored tests fix

Rig: admin `Boss` (`boss@example.test`, roles `player`, `admin`, password hash fixed), player `Fox_9`
(`fox@example.test`, `player`), a stored session for Boss with raw token `raw-session-token` and CSRF `csrf-token-1`,
admin API on.

| Area | Cases |
| --- | --- |
| Session gate | no cookie → 401 `{"error":{"code":"ADMIN_UNAUTHORIZED","message":"Administrator sign-in required."}}`; `rustic_isles_admin=nope` → `Administrator session expired.`; `%E0%A4%A` → 500; the right cookie → 200 with `cache-control: no-store` and the CSP; `/me`'s account keys exactly `userId, displayName, email, approvalStatus, status, roles, usernameSet, createdAt`; `mysticparadox_admin=` → 401 (no legacy cookie) |
| CSRF | `PATCH .../roles` without the header → 403 `{"error":{"code":"ADMIN_CSRF","message":"Invalid request token."}}`; wrong token → 403 |
| Revocation | Boss loses `admin` → 403 `ADMIN_FORBIDDEN`; restored → 401 (the session was revoked) |
| Origins | `https://evil.example` → empty 403; `http://localhost:4173` → 200 with the five CORS headers exactly; `ADMIN_ALLOWED_ORIGINS= https://panel.example , ` admits that origin and refuses localhost; production refuses localhost; no `Origin` → no check |
| Login | wrong email, wrong password, non-admin, `{}` → 401 exact `ADMIN_INVALID_CREDENTIALS` body; ` BOSS@example.test ` with the right password → 200, cookie `rustic_isles_admin=...; Path=/admin/v1; HttpOnly; SameSite=Strict; Max-Age=28800`, 64-character CSRF, `displayName` `Boss`; logout → 200 `{}` and `rustic_isles_admin=; Path=/admin/v1; HttpOnly; SameSite=Strict; Max-Age=0`; then 401. With a TOTP secret: a wrong code → `ADMIN_MFA_INVALID`, the right one → 200 (auth's TOTP at a fixed system time); production without a secret → 503 `ADMIN_MFA_NOT_CONFIGURED`; six attempts from one address, the seventh 429 exact body |
| Players | default page `total` 2, `page` 1, `limit` 25, keys `accounts, total, page, limit`; `page=2&limit=1&search=fox` → page 2, limit 1, no accounts; `limit=1000` → 100; `limit=abc&page=-4` → 25; `ids=<fox>,nobody,bad%20id,<fox>` → total 1, page 1, limit 2; online → count 0, source `xmpp+presence` |
| Access | the three `ADMIN_VALIDATION` texts; reason ` ab ` → `ADMIN_ACCESS_REASON`; banning oneself → 409 `ADMIN_SELF_LOCKOUT`; `nobody` → 404 exact; ban Fox with `  cheating  ` → status `banned`, `approvalReason` `cheating`, `approvalUpdatedBy` Boss's userId, one refresh revocation and one code revocation; the audit for Fox has total 1, the request id, `oldState` `{"approvalStatus":"approved","status":"active"}`, `newState` `{"approvalStatus":"approved","status":"banned"}`, keys in §6.6's order |
| Roles | `"tester"` (not an array) and `["admin"]` refused with their texts; `["tester"]` with a reason → `["player","tester"]`, `rolesUpdatedBy` Boss; `[]` → `["player"]` and a second code revocation; the audit's newest record is `player.roles.update` without `reason`; `limit=1&page=3` reaches the access record |
| Delete | own → `ADMIN_SELF_DELETE`; `nobody` → 404; `DELETE fox_9` → `ADMIN_DELETE_CONFIRMATION` with message `Type "DELETE Fox_9" to confirm permanent deletion.`; reason `x` → `ADMIN_DELETE_REASON`; done → keys `requestId, deletedCounts, characterCount`, `adminSessions` 0, `characterCount` = the characters Fox had (a rig with one character answers 1), audit `player.account.delete` with `{"deleted":true}` |
| Admin off | `RUSTIC_ISLES_ADMIN_API=off` → `/admin/v1/me` 404 (core) |
| Discord | unconfigured start → 503 `ADMIN_DISCORD_NOT_CONFIGURED`; configured (`client-123`, `secret-456`, frontend `http://localhost:4173/admin`, redirect `http://localhost:4173/admin/v1/auth/discord/callback`, allow `100000000000000001`) → 302 to `https://discord.com/api/oauth2/authorize?` containing `client_id=client-123` and `code_challenge_method=S256`, cookie starting `rustic_isles_admin_oauth=`; callback `?error=access_denied` → `http://localhost:4173/admin/?error=discord_cancelled`; `?code=abc&state=bad` → `...?error=discord_expired`; a full flow with `StubDiscord` (`code-100000000000000001`) and the start's cookie → 302 home with the burn and session cookies, a new `Arcn1hssight` admin, audit `admin.discord.bootstrap`; the same with the wrong binding → `discord_state_mismatch`; a disallowed id → `discord_not_allowed`; a linked non-admin eligible account → promoted, audit `admin.discord.promote` |
| Discord units | allow list: none, empty, `not-a-snowflake`, `100000000000000001,123` refused; ` 100000000000000001, 100000000000000001,123456789012345678 ` → the two ids; display name `Arcn1hssight`, 3-16 alphanumerics; state with binding `[0x11; 32]` and nonce `a`×48 is deterministic, verifies with it, not with `[0x22; 32]`, not with none, not with a trailing `x` |
| Tester max | non-tester → 409 `ADMIN_TESTER_REQUIRED`; `MAX fox_9` → 400 `ADMIN_TESTER_MAX_CONFIRMATION`; `runId` `short` → 400 `ADMIN_TESTER_MAX_RUN_ID`; no kit → 503 `ADMIN_TESTER_MAX_KIT_MISSING`; a kit with a fake funding port: a full run `done` with grants per block, a re-run with the same runId grants nothing new and reports the same totals; no character → 409 `ADMIN_TESTER_MAX_NO_CHARACTER` with an audit `player.tester_max.failed`; two concurrent runs → one 409 `ADMIN_TESTER_MAX_BUSY` |
| Tester units | plan `(0, 250000, 100000)` → `(0,100000) (1,100000) (2,50000)`; `(200000, 250000)` → `(2,50000)`; equal → none; `(1234, 100000)` → `(0,98766)`; derived id deterministic, 45 characters, `tmax-` prefix; run ids `abcdefgh` ok, `abcdefg` and `-leading-hyphen` refused; a good kit has no problems; `slayersPath` gives exactly its one problem; `CURRENCY_PLATINUM` gives one Platinum-aggregate problem and `CURRENCY_SEASONAL_COIN` none |
| Release gates | aggregate ok only with gates and all ok, keys `ok, gates`; guard mode texts (` Observe ` → `mode=observe`, `sometimes` not ok without echoing it); registration texts (`maybe` not echoed; auto-approve note); TLS: `off` text; `on` with `127.0.0.1`, `::1`, `localhost` ok; `0.0.0.0`, `203.0.113.9`, unset not ok (the address not echoed); `perhaps` not ok; legacy `MYSTPAX_B`, `MYSTPAX_A` → `2 MYSTPAX_ key(s): MYSTPAX_A, MYSTPAX_B` (the value never shown); core gates in order; active play `nobody in game`, 3 held with the 2-minute text, 3 with the override ok; the route from a non-loopback peer or with `x-forwarded-for` → empty 404 |
| Manager operator (new) | Secret `s`×40, switch on, operator `111111111111111111`: a signed `GET /admin/v1/me` → 200, `csrfToken` null, account `manager:111111111111111111`, name `admin-api`; signed `PATCH .../access` with a JSON body → 200 and the audit's actor `manager:...` (no CSRF needed); the same signature again → 401 `ADMIN_OPERATOR_REPLAY`; timestamp 61 s old → `ADMIN_OPERATOR_STALE`, 6 s ahead → stale, 5 s ahead → ok; a tampered target, body or method → `ADMIN_OPERATOR_SIGNATURE`; upper-case signature hex accepted; id `abc`, missing timestamp, two id headers → `ADMIN_OPERATOR_INVALID`; `x-forwarded-for` or `via` present → 403 `ADMIN_OPERATOR_FORWARDED`; a non-loopback peer → 403 `ADMIN_OPERATOR_PEER_REFUSED` (checked before the format); secret of 31 characters → 503 `ADMIN_OPERATOR_NOT_CONFIGURED`; a form body → 400 `ADMIN_OPERATOR_BODY`; switch off → the operator headers are ignored and the request is 401 `ADMIN_UNAUTHORIZED`; start with the switch on and no secret → `Failed` with §3.3's text; the canonical string and signature agree with `crates/admin`'s `operator_signature` for a fixed input |
| Host routes | without the key or with `x-reforged-gateway` or `x-forwarded-for` or a 31-character key → 403 `{"error":"forbidden"}`; CreateInvite defaults to 1 use, `uses: 0`, `uses: 1001`, `name: 5`, `name: null` refused with the exact body; a made code matches `^[A-HJ-NP-Z2-9]{4}(-[A-HJ-NP-Z2-9]{4}){2}$`; InviteCodes lists it with `note` null; DELETE `a` → 400, unknown → empty 404, known → empty 200; SetAccountStatus bad status, bad name → 400 exact, unknown → 404 not-found body, `banned` → 200 keys `UserId, Username, Status`; Kick → 200 keys `UserId, Username, Closed`, refresh sessions revoked |
| Admin actions | (core's validation is core's test) `Kick` keeps refresh sessions and revokes codes; a timeout keeps the player out until it ends or is lifted and `Lookup` shows `Timeout` with since and until; a ban clears it and `Lookup` shows `Ban` with the reason; `Unban` leaves a timeout; resolution by global id, by userId, by name in any case; empty or unknown text → `NotFound` |
| Home operations | `progression.grant` `{"xp":-5}` → progress unchanged, version + 1; `inventory.write` `["A:2","A","B:x"]` on an account without characters → a `Slayer` character, stacks `A` 3, `B` 1, `loot_applied` 3; `inventory.read` without a character has no `character_id`; `bounty.write` twice → versions 1, 2, `bounty.read` count 2; `character.load` echo; `nope.op` → `NotFound` |
| Bootstrap | wrong confirm → usage; `Admin@Example.com` from `old@example.com` on a disabled, pending password account → admin role, email `admin@example.com`, active and approved by `bootstrap-script`; a rerun finds it by the new email; a Discord-only account → not password-based; a collision on the new email → the collision text |
| Debug | `Debug` of the Discord config, the caller, the per-module state and the operator verdict never contains the secret, a token, a token hash or the CSRF token |

---

## 18. Security behaviour (RX3: `docs/security/rustic-isles-reforged.md`, admin section)

The admin API is a trust boundary: it can ban, delete and fund any account.

- **Reachability.** Every isles listener binds 127.0.0.1; the gateway refuses `/admin` from outside as host-only
  (gateway `modules.rs`), so the admin API is reached by the dashboard and admin service on the host, or a browser on
  the host. The admin API is off unless `RUSTIC_ISLES_ADMIN_API` or `RUSTIC_ISLES_MANAGER_ADMIN` is on (core §6.3);
  `setup` turns the latter on.
- **Three credentials**, each checked before any handler: a session cookie (HttpOnly, `SameSite=Strict`, path
  `/admin/v1`, 8 hours, stored as SHA-256 only) with a CSRF token on every state change; the manager credential
  (loopback peer, no forwarding header, HMAC over method, target, time, operator and body, 60 s window, 5 s skew, a
  120 s nonce); the host key for `/reforged/api/*` (loopback, no gateway stamp, no forwarding header, ≥ 32 characters,
  constant-time). A session's account is re-checked on every request and the session revoked when it loses `admin`
  or eligibility.
- **Sign-in** is limited to 6 attempts per client address per 15 minutes, needs the password and the `admin` role, and
  TOTP when a secret is set (always in production: no secret is a 503). Discord sign-in is allow-listed by numeric
  id, PKCE (S256) and browser-bound (HMAC state keyed by an HttpOnly `SameSite=Lax` cookie, 5 minutes, burned on every
  callback), its transaction single-use.
- **Clocks.** Every trust decision uses the system clock (§2.2), so replay headers, even if a host turns them on
  (core §17 item 1), cannot reopen an attempt window, replay an old TOTP code, or stretch the operator window.
- **Origins.** A cross-origin browser call is answered only for a listed origin; production lists none by default.
  Every admin answer is `no-store` with a deny-all CSP.
- **Self-protection.** An admin cannot ban, reject, disable or delete their own account; deletion needs the typed
  name and a reason; every change is audited in the same transaction with actor, IP and request id.
- **Secrets never logged or in `Debug`**: `ADMIN_TOTP_SECRET`, the operator secret, `REFORGED_ADMIN_KEY`,
  `DISCORD_CLIENT_SECRET`, session tokens and hashes, CSRF tokens, the PKCE verifier, the binding. Restricted's
  derived `Debug` of the Discord config prints the client secret and the caller's prints the CSRF token and token
  hash: a deliberate improvement here (as core §17's settings `Debug`).
- **Known weak spots to list:**
  - The login answers a missing email without a password check, so its timing tells whether an address has an
    account (rate-limited; Node does the same).
  - The origin check passes a request without an `Origin` header (non-browser clients are expected); browser
    protection rests on the cookie's `SameSite=Strict` and the CSRF token.
  - The operator's display name is unsigned and only labels the caller; the audit records the signed id.
  - The Discord state is consumed before the binding is checked, so a stray callback burns a legitimate attempt.
  - The release-gates detail text is fixed, but the route reveals how many players are active to any loopback caller.
  - `SetAccountStatus` bans without revoking sessions (§19 item 7); the next token refresh is refused by auth.

**Fuzz targets (`fuzz/fuzz_targets/`):**

- `rustic_isles_reforged_admin_auth`: arbitrary header sets, peer, method, target, body bytes and clock against the
  operator verifier (a pure function of those, the secret and a nonce set): never panics; accepts only when a reference
  HMAC over the canonical string matches; never accepts the same signature twice inside 120 s; refuses any forwarding
  header and any non-loopback peer before looking at the signature. The cookie reader and `decodeURIComponent` on
  arbitrary bytes never panic and find only exact names.
- `rustic_isles_reforged_admin_text`: the secure-URL reader (§5.1) never panics and never accepts a URL with `@` in the
  authority, a `#`, or a non-https scheme off localhost in production; the bound-state verifier never panics and
  accepts only states the creator makes for that binding; `parse_int`, page and limit never panic and stay in range;
  the player-ids reader yields at most 100 ids of the allowed characters; the kit validator and run-id check never
  panic on any JSON; the local admin check never panics on any header set.

---

## 19. Behaviour that looks wrong but is kept (parity)

1. Text lengths count Unicode scalars here (restricted) where Node counted UTF-16 units (reasons, user agents, notes):
   differs only for astral characters.
2. Rifts admin routes get the admin headers only on a refusal, not on rifts' answer (§3.5).
3. The Discord callback consumes the transaction before the binding check (§5.3 step 5).
4. Discord answers are empty 302s; Node's `res.redirect` wrote a short `Found. Redirecting to ...` body.
5. The secure-URL reader is not WHATWG's: it keeps a default port in the origin (`https://x:443` ≠ `https://x`) and
   does not validate the port; Node's `new URL` normalized both. A host writing the same text in both settings is
   unaffected.
6. `characterCount` follows Node, not restricted (§6.5): a fix of restricted's bug, not a departure.
7. `SetAccountStatus` (host route) sets `banned` or `disabled` without revoking sessions or codes; the admin API's
   access change does revoke them; the admin action `Ban` does too.
8. `Kick` as a host route revokes refresh sessions (the player signs in again with their key); `Kick` as an admin
   action keeps them (only codes and chat).
9. An access change through the admin API does not forget the status cache; the host routes and admin actions do.
10. `NotHome` is a 409 for an access change but an internal 500 for roles and deletion.
11. Requested roles keep duplicates (`["tester","tester"]` stores both).
12. CreateInvite refuses `"name": null` while accepting an absent name.
13. Admin actions run each effect as its own write: a failure halfway leaves the earlier ones applied.
14. `Unban` leaves a timeout; `approval_updated_by` is `host` whoever the actor is.
15. Home operations ignore the invocation's actor; `inventory.write` overwrites unparseable stacked items with the new
    list and ignores a lost revision race while reporting the loot applied; `bounty.write` clears both stored daily
    dates and ignores a lost version race; `catalogId:` and `:5` entries are taken as written.
16. Tester max counts an already-applied block as granted again in a rerun's report; a rerun whose plan cuts a block
    differently (the stored progress moved) hits a grant id with a different amount and reports that track `error`.
17. Tester max reads characters with a store error as "no character".
18. Bootstrap lower-cases ASCII only, while the admin login lower-cases Unicode: an address with non-ASCII capitals
    is stored in a form the login lookup does not produce.
19. Two `x-forwarded-for` headers: restricted read the first only; auth's `client_ip` reads all. The rewrite uses
    auth's (one client-address rule for the module).

---

## 20. Open questions

1. **System clock for admin trust decisions** (§2.2): a security departure from restricted (which used the
   replay-aware clock everywhere). With core §17 item 1 gating replay headers off by default the risk is small, but a
   host that turns them on would otherwise let a client reset the login window, replay a recorded TOTP code and stretch
   sessions. Proposed as specified; a candidate decision row.
2. **The manager operator credential** (§3.3) is implemented from Node because restricted lacks it and dainty-lass's
   admin service and dashboard depend on it. Proposed as specified; confirm.
3. **Header name `x-paradox-operator-*`** keeps a Paradox name. It is the wire contract `crates/admin` and the
   dashboard already send, not a legacy alias, so it stays; renaming is a coordinated change across three crates.
4. **`OPTIONS` and unmatched `/admin/v1` paths.** Node's admin router answered `OPTIONS` on any `/admin/v1` path with
   204 after the origin check, and put the admin headers (and the origin refusal) on unmatched paths and on routes
   mounted after it (the rifts admin routes). Core §15 item 3 keeps no `OPTIONS`; a cross-origin dashboard's preflight
   therefore fails. Proposed: keep core's rule (the shipped dashboard is same-host and server-side); revisit if a
   browser dashboard on another origin is wanted.
5. **The signed target** is the request target as received. Core hands handlers the target after the `/__origin/`
   strip; the manager never sends that prefix, so the two are the same for every real call. Proposed: sign what the
   handler sees; core need not keep the original.
6. **Core's admin authenticator slot** (§16.2): core §3.7 must ask it for the `Admin` caller kind. A request to the
   core implementer; until it lands, rifts admin routes see an anonymous caller and refuse.
7. **The bootstrap subcommand** (`reforged-host admin bootstrap`) is a `clap`-tree change; which binary carries it
   (hostkit's CLI or the module's own command, run through the controller) is the integration pass's call.
8. **`set_email`** is not in the store's `LauncherAccountRepository`; bootstrap's rename needs it (restricted wrote the
   column directly). A request to the store implementer: `set_email(tx, &AccountId, email)`; it does not touch
   `roles_updated_*`.
9. **Legacy gate.** With `MYSTICPARADOX_*` aliases gone (core §17 item 2), a host's leftover `MYSTICPARADOX_*` lines
   are silently ignored. Proposed: the `no_legacy_env_keys` gate also lists `MYSTICPARADOX_` names (a behaviour
   addition, so a decision row); parity until then.
10. **Bootstrap audit.** Neither Node nor restricted audits the bootstrap. Proposed: keep parity; an audit row
    (`admin.bootstrap`, actor `host:console`) is a small addition the maintainer may want.
11. **`reforged-host invite list`** reads `uses`, `maxUses`, `infinite` from `InviteCodes`, which answers
    `usesRemaining` and `infiniteUses`: the host sees `uses: 0/0`. The backend keeps its wire (the admin service and
    tests read it); the host kit's reader is the bug (a hostkit row).
12. **The economy port** (§15.3) is a placeholder until unit 7's spec; its names follow restricted's progression and
    inventory functions and may change with that spec.
13. **Home operations** are DN5's test vehicles with lost-update gaps (§19 item 15). Proposed: parity now; route them
    through the economy crate's transactions when unit 7 lands.

---

## Coordinator decisions (2026-11-05, before implementation)

These settle §20 and override anything above that disagrees.

- **Item 2: the manager operator credential is implemented (§3.3).** Restricted's lack of it is the bug (AGENTS.md:
  where Node and Rust differ the Rust side is wrong); `crates/admin` and the dashboard already sign every `/admin/v1`
  call this way.
- **Item 1:** admin trust decisions read the system clock (security); a candidate decision row.
- **Item 3:** the `x-paradox-operator-*` names stay (live contract with `crates/admin`).
- **Items 4-5:** as proposed (no `OPTIONS`, per core §15 item 3; the signature covers the stripped target).
- **Item 6:** core defines an `AdminAuthenticator` slot trait and consults it for rifts' `Admin` caller kind; this
  crate implements it (requested of the core implementer).
- **Item 7:** bootstrap is a library function here; the `reforged-host admin bootstrap` subcommand is a clap-tree row.
- **Item 8:** `LauncherAccountRepository::set_email` is added to the store spec's decisions.
- **Items 9-10:** no (the aliases are gone; no bootstrap audit record).
- **Item 11:** the `reforged-host invite list` field mismatch is a separate hostkit fix row.
- **Item 12:** the economy specs have landed; their names win and this crate calls them.
- **Item 13 and §19:** parity.
- **File split:** reconciled with the public half's §1 when it lands; `host_routes.rs` and the release-gates handler
  are this half's unless the coordinator records otherwise there.
