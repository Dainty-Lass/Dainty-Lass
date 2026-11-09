<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false, "code_blocks": false } } -->

# Clean-Room Behaviour Specification: `crates/rustic-isles-reforged`, the public half (RD27)

> **Scope and purpose (RD27):** this is one of the two clean-room behaviour specifications for the MIT crate
> `rustic-isles-reforged`, unit 9 of [`spec-rustic-isles-core.md`](spec-rustic-isles-core.md) §16 (the Reforged API
> area of Rustic Isles). This half covers what players, launchers, the host kit, monitoring and the other areas see:
> the probes, the metrics exposition, the request log and metrics as core's `Observer`, the launcher-facing Reforged
> API (`/reforged/api/Register`, `GetUserInfo`, `ServerStatus`, `RegistrationStatus` and the rules the host kit's
> routes share), the runtime and launcher update feeds and the signed Guard manifests, the launcher logs (uploads, the
> sync script's reads, the operator's view, the 90-day purge), the launcher notices, the operations policy, the
> activity monitor, Launcher Guard and its dashboard views, and the implementations of
> [`spec-rustic-isles-auth.md`](spec-rustic-isles-auth.md) §11.2's `Seams` that live in this area. The other half,
> [`spec-rustic-isles-reforged-admin.md`](spec-rustic-isles-reforged-admin.md), covers the admin API (its wrapper,
> sign-in and Discord sign-in, players, audit), the tester-max tool, the release gates, the host kit's local routes
> (B177 to B181), the admin actions and home operations of core §7.6 and §7.8, and `bootstrap.rs`. §1.2 fixes the
> boundary file by file and §14 the calls between the halves.
>
> The source is dainty-restricted's `modules/rustic-isles/crates/isles-core/src/areas/reforged_api.rs` and
> `areas/reforged_api/*`, AGPL-3.0-only with Mystic Paradox's section 7 terms; an implementer who never sees it writes
> the MIT crate from this document, the other half and the MIT specs it names. Everything below is stated from
> observable contracts: the routes and their bytes, the files on disk, the stored rows (by
> [`spec-rustic-isles-store.md`](spec-rustic-isles-store.md)'s names), the exposition text, the log lines, the public
> names the composer and the other crates call, and the tests. No implementation code is reproduced and no private
> helper is named; Node names (`routes/launcherUpdates.ts`) appear only as provenance. Wire facts a shipped launcher,
> the host kit, the dashboard or the release tooling depend on (`/reforged/api/*`, `x-reforged-*`, `RRK_`,
> `ParadoxRuntime`, `MYSTIC-GUARD-2`, the two Ed25519 public keys, `paradox-backend`) are interface facts and are kept.
>
> Written against dainty-restricted `e0e238b4434316bb210677d88bc59e3448688794` (worktree
> `splashloader-integration-90130d`; the files of this unit last changed in `9ad97c6`) and dainty-lass
> `9753bd210b37d88f1008ca0d38fdfbafcd9bb8b1` (branch `feat/rustic-isles-clean-room`).

---

## 1. Provenance and licensing classification

### 1.1 Evidence used

| Source | What it says |
| --- | --- |
| `modules/rustic-isles/NOTICE.md` | The module is a modified Mystic Paradox; every file is AGPL-3.0-only with Mystic Paradox's section 7 terms on the Mystic Paradox contributions |
| File headers (`//!`) | Each file ports a Paradox TypeScript file (`routes/reforgedApi.ts`, `launcherApiRules.ts`, `routes/launcherUpdates.ts`, `routes/launcherLogs.ts`, `services/playerLogs.ts`, `services/launcherNotices.ts`, `services/operationsPolicy.ts`, `src/activity/*`, `controllers/launcherGuard.ts`, `observability/metrics.ts`, `security/logRedaction.ts`, `app.ts`'s probes) |
| `git log` of the files | 17 commits, one author (the maintainer); the last is the Rustic Isles rename `9ad97c6` |
| `tests/reforged_api.rs`, `tests/reforged_api_golden.rs` and the unit tests of each file | The behaviour §18 fixes (described, not copied) |
| `areas/auth/{wiring,runtime,erasure,account}.rs` | How the auth area reaches this one today (the seams of §13) |
| `module.rs`, `http.rs`, `areas/{inventory,progression,bounties_store}` | Who calls the presence, the status cache, the observer and the activity recorders (§12, §14) |
| The MIT specs (core, auth, store, economy) and `crates/rustic-isles-auth/src/seams.rs` | The seams and names this crate plugs into |

### 1.2 The split, file by file

Every file is Category B (it ports Paradox TypeScript) and is rewritten. "Public" is this document; "admin" is the
other half.

| Restricted file (`areas/reforged_api/`) | Half | Rewritten as |
| --- | --- | --- |
| `../reforged_api.rs` (the module list and `register`) | both | Each half registers its own rows (§15.3) |
| `util.rs` (replies, request reading, ISO, HMAC, base64, the attempt-window limiter) | public owns the limiter state; the helpers are shared conventions | §2, §3.2 |
| `probes.rs` | public, **except** the `GET /health/release-gates` handler (admin) | §4 |
| `metrics.rs`, `observe.rs` | public | §4.3 to §4.5 |
| `http_get.rs` | public | §5.4 |
| `presence.rs` | public | §12.1 |
| `launcher_auth.rs` | public (admin may reuse it) | §3.1 |
| `rules.rs`, `launcher_api.rs` | public (the admin half's host routes use §5.1's rules) | §5 |
| `host_routes.rs` | admin | the other half's §9 |
| `updates.rs`, `guard_manifest.rs` | public | §6 |
| `logs.rs`, `player_logs.rs` | public (the operator's view runs inside admin's route wrapper, §14.2) | §7 |
| `notices.rs` | public (inside admin's wrapper) | §8 |
| `operations.rs` | public (inside admin's wrapper) | §9 |
| `activity.rs` | public (the admin reads inside admin's wrapper) | §10 |
| `guard.rs`, `admin_guard.rs` | public (the dashboard views inside admin's wrapper) | §11 |
| `admin.rs`, `admin_discord.rs`, `release_gates.rs`, `tester_max.rs` | admin | the other half |
| `module.rs`'s admin actions and home operations (core §7.6, §7.8), `bootstrap.rs` | admin | the other half |
| `areas/auth/runtime.rs` `DefaultActivity`, `DefaultOperations`; `erasure.rs` `AllAreasErasure::blockers`; `account.rs`'s notices view; `wiring.rs` `run_log_purge` and the TTL purge task | public | §13, §12.3 |

Routes: of the 62 rows the Reforged API answers (54 `reforged-api` rows and the 8 Guard rows isles keeps, core §4.2),
this half registers 44: B1, B2, B3, B5, B25, B64, B173 to B176, B182 to B198 (B186 and B187 included), B210 to B220
(B214 to B216 included), B222 to B227. The admin half registers the other 18: B4, B177 to B181, B199 to B209 and B221
(its §1.2 and §16.2 agree).

### 1.3 What stays in dainty-restricted

- Every restricted file above, until the composer drops it (core §16's procedure), and the goldens
  (`tests/golden/reforged_api.json` and its capture script): they are replayed in restricted against this crate.
- The composition: restricted's composer (core §11.4) builds this crate's state slot, registers its handlers and
  observer, and fills its seams from today's matchmaking and chat until units 8 and 12 land.
- `AuthReforgedCore` (rustic-rifts' `IslesCore`), including its `operations_policy` and `verify_operations_step_up`,
  which may call this crate's public functions (§9.4).

---

## 2. Conventions

- **JSON** is written as `JSON.stringify` writes it (`service_kit::json::Json`, `content-type: application/json;
  charset=utf-8`, insertion order, JavaScript number text: a stored `1.0` is answered `1`). Every key order below is
  the wire order. The crate enables `serde_json`'s `preserve_order` in its own manifest (core §17 item 8).
- **Empty** is the status with no body and no content type. **Status text** is Express's `res.sendStatus(n)`:
  `text/plain; charset=utf-8` with the reason phrase (`Bad Request`, `Not Found`, ...), and no body for 204 and 304.
  **Error page** is `service_kit::body::error_page(status)`.
- **Launcher envelope**: `{"error":{"code":..,"message":..,"requestId":<fresh UUID v4>}}` with the status the code
  has in auth §2.3's table (or the one given). A launcher-auth refusal is auth's own `LauncherError` answer.
- **Admin error**: `{"error":{"code":..,"message":..}}`, no request id (the admin half's shape).
- **Times**: Unix ms from core's replay-aware clock (`rustic_isles::replay::now_ms`), ISO text as `toISOString`.
  `Date.parse` is `service_kit::js::date_parse`; JS trim is `service_kit::js::trim`; `Number(x)` is
  `service_kit::js::number`; a "safe integer" is `service_kit::js::safe_integer` (a JSON number that is whole and
  within ±(2^53−1); a string is not one).
- **Ids**: request ids in audits and envelopes, Guard session ids, challenges, invite codes and account keys come from
  the system CSPRNG, never the replay seed. A new **userId** comes from core's id source (`UserId::random`, replay
  seeded); a new **account id** is `AccountId::new(node)`.
- **Client address** ("the client"): auth §4.6 (`rustic_isles_auth::client_ip`): a loopback peer's
  `x-forwarded-for`, right to left, first non-loopback entry; no peer: `unknown`. Restricted reads only the first
  `x-forwarded-for` header line where auth reads every line; the gateway sends one, so the two agree (§21 item 9).
  **The peer** is the socket address's IP as text (`::ffff:`-mapped kept as written).
- **Segment**: a non-empty text of ASCII letters, digits, `.`, `_`, `-`, and neither `.` nor `..`. **Basename** is
  `path.basename`: trailing `/` and `\` trimmed, then the text after the last `/` or `\`.
- **Lenient base64** is `Buffer.from(text, "base64")`: every character outside `A-Za-z0-9+/-_` is dropped, `-` reads
  `+` and `_` reads `/`, a dangling single last character is dropped, the rest decoded without padding; text that
  still does not decode gives no bytes.
- **Pretty JSON file** is `JSON.stringify(value, null, 2)` followed by one `\n`.
- **State** (§12) is one per module: restricted keeps the status cache, readiness, metrics registry, recent-activity
  map, attempt windows, boot time and log-dedup key process-wide; this crate keeps each in its module's state slot
  (core §15 item 14, extended; one module behaves the same).
- Handlers read bodies with core's `read_body_parts_limit` rules (core §3.9) unless a route says otherwise. "Body
  fields" are the parsed JSON when it is an object, else none (an array, a scalar or no body has no fields).

---

## 3. Shared rules

### 3.1 Launcher sign-in (`HasLauncherAuth`, the bearer half)

The `authorization` header's first value: absent or not starting `bearer `, `Bearer ` or `BEARER `: 401
`AUTH_UNAUTHORIZED` / `Sign in required.`. Otherwise auth's `launcher_auth_bearer` (auth §4.2 step 1) decides and its
refusal is answered as auth answers it. The account key (`x-reforged-user-api-key`) is **not** accepted on these
routes. The caller is the account id, the userId, the session family and the launcher account.

### 3.2 Attempt windows (`IsRateLimited`)

A sliding window per key: an attempt counts while `now − t < window`; refused when the key already holds `max`
attempts; a refused attempt is not recorded. The clock is core's. Keys used by this half: `update-download:{ip}`
(180 / 60 s), `update-publish:{ip}` (30 / 60 s), `log-upload:{userId}` (120 / 1 h), `log-upload-ip:{ip}` (300 / 1 h),
`admin-operations-step-up:{userId}` (6 / 15 min). The admin half uses the same windows (§14.1). The crate uses the
module's auth limiter (`Auth::limiter().is_limited_at`, auth §4.7: same rule, bounded to 16 384 keys), not a map of
its own (§21 item 4).

### 3.3 The account key and who holds it

- **Shape check** ("looks like a key"): 8 to 128 bytes of ASCII letters, digits, `_`, `-`.
- **A new key** is `RRK_` followed by 48 lower-case hex digits (24 CSPRNG bytes); its stored form is auth's user key
  hash (`rustic_isles_auth::hashes::user_key_hash`, auth §5.3: HMAC-SHA256 of `user`, NUL, key, under
  `API_KEY_HASH_SECRET`). A secret under 32 UTF-16 units makes every hash fail.
- **The holder** of a presented key (`x-reforged-user-api-key`, first value, as sent): shape check, hash (fails:
  no holder), then every stored hash (`api_keys.find_all_user_key_hashes`) compared in constant time after hex
  decoding (hex decoding stops at the first pair that is not hex), every row compared, the first match winning. The
  holder: account id; userId = its game account's `user_id`, else the account id text; username = the launcher
  account's display name, else the game account's name, else the userId; admin = the launcher roles contain `admin`;
  the launcher account if any. A store failure is no holder.
- **Eligible holder** (`IsAccountEligible` of `security/accountEligibility.ts`): no launcher account, or one whose
  status is `active`, whose approval is `approved` (null reads approved) and whose `username_set` is not stored
  false. **Timeouts are not consulted** here, unlike auth's `is_account_eligible` (auth §6.1), so the crate keeps
  this rule of its own (`reforged_api::is_account_eligible`, §15.2) and the admin half uses it too (its §2.5).
- **Announcing a holder**: each time an eligible holder is found (`GetUserInfo`, `ServerStatus`), the home API is told
  `register_account_key_hash(<SHA-256 hex of the key text>, account)` and `register_directory_account(account,
  username)`; failures are ignored.

### 3.4 Paths on disk

- **Update root**: `LAUNCHER_UPDATE_ROOT` as set (even empty), else `updates`; made absolute against the working
  directory (no symlink resolution).
- **Log root**: `LAUNCHER_LOG_STORAGE_DIR` as set, else `../MysticLauncherLogs`; made absolute the same way.
- Every file name built from a request is a segment (§2) checked before the path is joined.

---

## 4. Probes, metrics and the request log

### 4.1 Probe routes

| Row | Route | Answer |
| --- | --- | --- |
| B1 | `GET /` | 200 `text/html; charset=utf-8`, body `ok` |
| B2 | `GET /health/live` | 200 `{"ok":true,"service":"paradox-backend","role":<role>}` |
| B3 | `GET /health/ready` | ready = the readiness state (§4.2) is ready **and** a store is composed and answers a trivial query (`SELECT 1` on a read); 200 when ready, else 503: `{"ok":<ready>,"service":"paradox-backend","role":<role>,"reason":<"READY" when ready, else the state's reason>,"persistence":<"ready"|"unavailable">}` |
| B64 | `GET /maintenance/status` | 200 `{"show-status":false}` |
| B25 | `GET /dauntless-status` | 200 `{"show-status":true,"en":W,"fr":W,"it":W,"es":W,"de":W,"pt":W,"ru":W,"ja":W}`, W = `Welcome to Rustic Isles!` (Paradox's `system.ts` answer, which shadows the Reforged route of the path; core §4.3) |

**Role** (`RUSTIC_ISLES_SERVICE_ROLE`, raw, trimmed, lower-cased): `api` is `api`; anything else, unset included, is
`combined` (Node refused other values at start; here they read as the default).

These rows sit before the budget mark (B1 to B4; core §3.2): no request budget, no launcher CORS. B64 and B25 do not.

### 4.2 Readiness

One state per module: ready, a reason (at most 96 characters), the time it last changed. It starts ready with reason
`API_BEHIND_PROXY_RUNNING`, its time taken the first time it is read. Nothing in this unit changes it; `set_readiness`
(§15.2) is public for a listener plan.

### 4.3 `GET /internal/v1/metrics` (B5)

**Authorized** when:

- `RUSTIC_ISLES_METRICS_TOKEN` trimmed is non-empty: the supplied token equals it in constant time (lengths must
  match). Supplied: if the `authorization` header lower-cased starts `bearer `, the text after its first 7 bytes,
  trimmed (an empty rest is supplied as empty; the header below is then not read); else `x-rustic-isles-metrics-token`
  trimmed, when non-empty.
- No token: `NODE_ENV` is not exactly `production` **and** the peer is exactly `127.0.0.1`, `::1` or
  `::ffff:127.0.0.1`.

Refused: `text/plain; charset=utf-8`, body `metrics unavailable\n`, status 401 when `RUSTIC_ISLES_METRICS_TOKEN` is
set to anything non-empty untrimmed (so a token of spaces refuses with 401 while the loopback rule applies), else
503. Authorized: 200, `content-type: text/plain; charset=utf-8; version=0.0.4`, `cache-control: no-store`, the
exposition of §4.4.

### 4.4 The exposition (Prometheus text 0.0.4, `observability/metrics.ts`)

Lines joined by `\n`, with one `\n` at the end. Numbers are JavaScript number text (`service_kit::jsjson::number`;
a non-finite value writes `0`). In order:

1. `# HELP rustic_isles_info Process identity and configured role.`, `# TYPE rustic_isles_info gauge`,
   `rustic_isles_info{role="<role>",service="metagame"} 1`.
2. `# HELP rustic_isles_service_ready Whether the service has completed its role startup.`,
   `# TYPE rustic_isles_service_ready gauge`, `rustic_isles_service_ready 1` (or `0`).
3. `# HELP rustic_isles_service_readiness_changed_timestamp_seconds Unix time of the last readiness transition.`,
   `# TYPE rustic_isles_service_readiness_changed_timestamp_seconds gauge`,
   `rustic_isles_service_readiness_changed_timestamp_seconds <changed ms / 1000>` (`12.5` stays `12.5`).
4. Derived sections registered by other areas, each line `# HELP n h`, `# TYPE n gauge`, `n v` (none in this unit;
   the extension point stays: a section names the raw gauges it renders itself, which step 6 then skips).
5. For every distinct counter name, sorted: `# HELP <name> Rustic Isles counter.` and `# TYPE <name> counter`
   (all these pairs first); then every counter sample.
6. Every gauge sample, each preceded by its own `# HELP <name> Rustic Isles gauge.` and `# TYPE <name> gauge`
   (repeated per label set).
7. Every histogram: `# HELP <name> Rustic Isles request latency histogram.`, `# TYPE <name> histogram`, then
   `<name>_bucket{<labels>,le="<b>"} <count>` for `b` in 5, 25, 100, 250, 500, 1000, 5000, `+Inf` (cumulative: an
   observation counts in every bucket whose bound it does not exceed; `le` is appended after the sorted labels),
   `<name>_sum{<labels>} <sum>`, `<name>_count{<labels>} <count>`.

A sample is `name{l1="v1",l2="v2"} value`, or `name value` without labels; values are escaped (`\` as `\\`, `"` as
`\"`, newline as `\n`). Samples within steps 5 to 7 are ordered by the text `<name>|{<label>="<value>",...}` where
each value is written in Rust's `{:?}` quoting, compared as bytes (the restricted registry's order; with the label
rules below no value needs escaping, so this is plain `name|{k="v",...}` order).

**Registry rules** (one per module):

- Metric name: `[a-z_:][a-z0-9_:]*`, at most 128 bytes; otherwise the sample is filed under
  `rustic_isles_invalid_metric`.
- Labels: a label is kept only when its name is one of `class`, `code`, `le`, `method`, `operation`, `result`, `role`,
  `service`, `status` (and matches `[a-z_][a-z0-9_]*`, at most 32 bytes) and its value is 1 to 48 characters of ASCII
  letters, digits, `_`, `.`, `:`, `-`; others are dropped silently. Kept labels are sorted by name and cut to 4.
- `increment(name, amount, labels)`: ignored unless finite and > 0; the total caps at 2^53−1. `set(name, value,
  labels)`: ignored unless finite. `adjust(name, delta, labels)`: ignored unless finite; the gauge never goes below
  0. `observe(name, value, labels)`: ignored unless finite and ≥ 0.

### 4.5 The request log and request metrics (core §11.2 `Observer`)

The crate's observer is what the composer hands core (core §11.4 `Composition.observer`). `begin` gets the request as
received (before the origin strip) and the request id; `finish` gets the answer's status.

**begin:**

1. The target is the request's path and query. If it starts `/__origin/`, the host segment (up to the first `/`, `?`
   or `#`) is lower-cased and kept for the log only (no validation here), and the rest is the target (`/` when
   empty).
2. **Class** from the target's path (up to `?` or `#`), case-sensitive prefixes: `/admin/` `admin`, `/health/`
   `health`, `/launcher/` `launcher`, anything else `game` (`/internal/v1/metrics` is `game`).
3. Counts `rustic_isles_http_requests_total{class,method}` by 1 (the method's text as the label; a method the label
   rules refuse loses the label) and adjusts `rustic_isles_http_inflight` by +1.
4. **Request line**: `[REQ] requestId=<id> <METHOD> <host><url>[ origin=<origin host>] gsKey=<Y|N> bearer=<Y|N>`,
   where host is the `host` header's text or `?`; url is the sanitized target (below); gsKey is `Y` when
   `x-rustic-isles-gameserver-apikey` is present (any value); bearer is `Y` when `authorization`, lower-cased,
   starts `bearer `.
5. **Quiet routes**: a path `/mystic/v1/hunts/<id>/lease` or `/mystic/v1/hunts/<id>/event-batches` (id non-empty,
   no further segment) is logged only when refused. Every other request logs the request line now, at info.

**finish(status):** adjusts the inflight gauge by −1; counts `rustic_isles_http_responses_total{class,status="<N>xx"}`
(N = status / 100); observes `rustic_isles_http_request_duration_ms{class}` with the elapsed whole milliseconds. A
quiet route with status < 400 logs nothing; a quiet route with status ≥ 400 logs its request line now, then the
response line. Every other request logs the response line:
`[RES] requestId=<id> <METHOD> <url> → <status> (<ms>ms)` (the arrow is U+2192), at info.

**Sanitizing the url** (`sanitizeUrlForLog`), three replacements in order:

1. `/sessions/kill/` (ASCII case-insensitive) followed by one or more characters other than `/`, `?`, `#`: the run
   after the prefix becomes `[REDACTED]`.
2. `eyJ`, then 5 or more of `[A-Za-z0-9_-]`, `.`, 5 or more of the same, `.`, then any number of the same: the whole
   match becomes `[REDACTED-JWT]`.
3. `?` or `&` followed by one of `token`, `access_token`, `refresh_token`, `code`, `exchange_code`, `authorization`,
   `api_key`, `apikey`, `signature` (ASCII case-insensitive) and `=`: the value up to the next `&` or `#` becomes
   `[REDACTED]`.

---

## 5. The Reforged launcher API and the host kit's routes (`routes/reforgedApi.ts`)

A route marked "store" answers core's 501 envelope (core §3.6) when the module has no store, before reading anything.

### 5.1 Registration mode and usernames

- **Mode** (`REGISTRATION_MODE`, raw, trimmed, upper-cased): `OPEN`, `NONE`; anything else, unset included,
  `INVITECODE`.
- **Username** (`NormalizeUsername`): a JSON string, JS-trimmed, 3 to 16 UTF-16 units, only ASCII letters, digits and
  `_`; the trimmed text is the name. Its normalized form is lower-cased.
- **Invite code shape**: 4 to 64 bytes of ASCII letters, digits, `-`. **A new code**: three groups of four from
  `ABCDEFGHJKMNPQRSTUVWXYZ23456789` (uniform, no modulo bias) joined by `-` (14 characters).
- **Invite note** (`InviteNoteFrom`): a JSON string; each character U+0000 to U+001F and U+007F to U+009F becomes a
  space; JS-trimmed; the first 64 characters; empty is none. Anything that is not a string is none.
- **Refusals** (`{"error":<code>,"message":<sentence>}`):

| Code | Message |
| --- | --- |
| `bad_request` | `The body is not JSON, or a field is missing or has the wrong type.` |
| `invite_invalid` | `That invite code is not valid, or has been used up.` |
| `not_found` | `No account has that name.` |
| `registration_closed` | `This server is not taking registrations.` |
| `username_invalid` | `A username is 3 to 16 characters: letters, digits and underscore.` |
| `username_taken` | `That name is taken.` |

### 5.2 `POST /reforged/api/Register` (B173; store)

1. Mode `NONE`: 400 `registration_closed` (before the body is read, so a broken body still gets this).
2. The JSON body (100 KiB); a parse failure is the error page.
3. `Username` not a string, or `InviteCode` present and not a string (`null` included): 400 `bad_request`.
4. Username invalid: 400 `username_invalid`.
5. The code is `InviteCode` JS-trimmed (empty when absent). Mode `INVITECODE` and the code fails the shape: 401
   `invite_invalid`. In mode `OPEN` a code is ignored (never spent).
6. A new key and its hash (§3.3); the hash fails: log error `[launcher-api] registration failed: API_KEY_HASH_SECRET
   must contain at least 32 characters.` and answer an empty 500.
7. One write, actor `Player { account: <new account id> }`, decided before its first change:
   - a live launcher account with the normalized name (`launcher_accounts.find_by_display_name_normalized`): 409
     `username_taken`, nothing written;
   - mode `INVITECODE` and `invites.spend(code, now)` false (expired codes too, D4): 401 `invite_invalid`, nothing written;
   - `accounts.create` (account id, the new userId, name = the username, notes 0), `launcher_accounts.create` (email
     none, display name and its normalized form, no password, status `active`, approval `approved`, roles
     `["player"]`, created now, `username_set` true, everything else none; a `DisplayNameTaken` outcome is 409
     `username_taken`), `api_keys.insert_user_key_hash(account, hash)`.
   A unique-constraint violation is 409 `username_taken`; any other store error logs `[launcher-api] registration
   failed: {error}` and answers an empty 500.
8. Logs info `[launcher-api] {username} registered as {userId}`; announces the key and the directory entry (§3.3,
   without the eligibility check); forgets the status cache (§12.2).
9. 200 `{"UUK":"RRK_..."}` (the only key) with header `x-reforged-account-id: <account id text>`.

### 5.3 `GET /reforged/api/GetUserInfo` (B174; store)

No holder (§3.3): 401 `{"error":"unauthorized"}`. A holder that is not eligible: 403
`{"error":<"account_banned" when the status is banned, else "account_disabled">,"reason":<the launcher account's
approval_reason cut to 500 characters, or "Ask the server's host." when empty or none>}` (a pending approval or an
unset username reads `account_disabled`). Else 200 `{"UserId":..,"Username":..,"IsAdmin":<bool>}` with
`x-reforged-account-id: <account id text>`.

### 5.4 `GET /reforged/api/ServerStatus` (B175; store) and the status body

Answer: 200, `cache-control: no-store`, `vary: x-reforged-user-api-key`, the body for the **full** view when the
caller is an eligible holder, else the **limited** view.

**The body** (the `ServerStatusBody` of dainty-lass `crates/dainty-protocol` `status.rs`, field for field, in this order):

| Field | Value |
| --- | --- |
| `name` | `REFORGED_SERVER_NAME`: control characters (U+0000 to U+001F, U+007F to U+009F) become spaces, JS-trimmed; empty: `Dainty Lass server`; else the first 64 characters |
| `online` | `true` |
| `version` | `REFORGED_SERVER_VERSION` JS-trimmed, first 48 characters; empty: `0.1.0` |
| `commit` | `REFORGED_SOURCE_COMMIT` JS-trimmed, first 64 characters (may be `""`) |
| `sourceUrl` | `REFORGED_SOURCE_URL` JS-trimmed; it must start `https://` or `HTTPS://` (exactly one of those two spellings); the authority (up to the first `/`, `?` or `#`) non-empty, without `@` or white space; answered as `https://` + the authority lower-cased + the rest, with `/` inserted when the rest is empty or starts `?` or `#`; anything else `null` |
| `registration` | the mode text |
| `uptimeSeconds` | whole seconds since the module's Reforged state was created |
| `contentPort` | `null` |
| `game` | `{"build":"rel-<game_version>-Archon","changelist":<target changelist>,"executableSha256":<APPROVED_EXECUTABLE_SHA256 raw lower-cased when it is exactly 64 hex digits, else null>}` |
| `gameData` | the backend game-data folder classified (below), cached 60 s per module |
| `chat` | `REALTIME_XMPP_ENABLED` raw is exactly `true` |
| `playersOnline`, `players`, `instances`, `limited` | full: the count of `players`, the players, the instances, `false`; limited: `0`, `[]`, `[]`, `true` |

**Game data class**: no backend folder: `missing`. Else for each of `progression_config.json`, `slayers_path.json`,
`slayers_path_definitions.json`, `ladyluck_store.json`, `platinum_store.json`, `reward_cache_store.json`,
`inventory_storage_policy.json`: missing → `missing` (at once); when the file and its `<stem>.example.json` both read
and are byte-equal, an example is in use. Any example in use: `synthetic`; else `real`.

**Instances** (full view): `GET http://<DEPLOYSERVER_URL>/api/instances` where the setting, trimmed, is used as the
host:port authority of a plain HTTP/1.1 request (`Host: <authority>`, `Accept: */*`, `Connection: close`), chunked or
plain body; anything but a 2xx with a JSON body, any failure, a blank setting or more than 2 s in all gives `[]`. The
answer is read field by field (`status_instances`):

- Not an array: no instances. At most the first 100 entries; an entry is skipped unless it is an object with `id` a
  non-empty string (cut to 64 characters) and `kind` exactly one of `city`, `hunt`, `dojo`, `tutorial`.
- Each kept entry: `{"id","kind","title","map","behemoth","players","maxPlayers","startedAt"}`: `map` the asset leaf of
  `map` (a string cut to 160, else `""`); `behemoth` the asset leaf of `behemoth` (string cut to 160) or `null`;
  `players`/`maxPlayers` a finite positive JSON number floored, else 0; `startedAt` `Date.parse` of a string as ISO,
  else `null`.
- **Asset leaf**: the text after the last `/`, up to its first `.`, first 64 characters
  (`/Game/Behemoths/Skarn.Skarn_C` is `Skarn`).
- **Title**: `city` `Ramsgate`, `dojo` `Training Grounds`, `tutorial` `Tutorial`; a non-empty behemoth: its leaf
  without a leading `BPC_` or `BP_` and a trailing `_C`, `_` as spaces, 64 characters; else the map leaf without a
  trailing `_persistent` (ASCII case-insensitive), `_` as spaces, 64 characters, and `Hunt` when that is empty.
- For a `hunt` with `playerIds` an array: its first 16 entries that are strings of at most 128 UTF-16 units map that
  player id to this instance id (a later instance overwrites).

**Players** (full view): the chat presence's online user ids (core §5.5; not the recent-activity map), at most 500;
each looked up (`accounts.find_by_user_id`, then `launcher_accounts.find_by_account`): name = the display name, else
the game name; a lookup error or an empty name skips the player; a whole read failure gives none. Each is
`{"name":<first 32 characters>,"where":<"hunt" when mapped to an instance, else "unknown">,"instance":<that id or
null>}`, sorted by `service_kit::collate::locale_compare` on the name.

**The status cache**: one body per view, kept 5 s per module; skipped (neither read nor written) while the request's
clock is pinned (`rustic_isles::replay::is_replay_pinned`). Forgotten by §12.2.

### 5.5 `GET /reforged/api/RegistrationStatus` (B176)

200 `{"mode":<OPEN|INVITECODE|NONE>}`. No store needed.

### 5.6 The host kit's routes (B177 to B181)

`CreateInvite`, `InviteCodes`, `InviteCode/:code`, `SetAccountStatus` and `Kick` are the admin half's (its §9: the
local admin call, the `Console` actor, the answers). They use §5.1's username, code, note and refusal rules and
§12.2's `forget_status`; the module registers their prefixes as host-only (core §7.2).

---

## 6. The update feeds and the signed Guard manifests (`routes/launcherUpdates.ts`)

### 6.1 The feed's settings and layout

Read at each request:

| Setting | Reading |
| --- | --- |
| `UPDATE_PUBLIC_BASE_URL` | raw (even empty), else `https://paradox.mysticfox.dev`; one trailing `/` removed |
| `LAUNCHER_UPDATE_URL_PREFIXES` | raw, else `https://github.com/pranav158/Mystic-Paradox/releases/download/`; split on `,`, each trimmed, kept when it starts `https://`, ends `/` and has no `?` or `#`. The **prefixes** are `<base>/launcher/v1/updates/` followed by these (the **configured** ones) |
| `RUNTIME_EXTRA_FILES` | `winmm.dll` followed by the comma list, each trimmed and lower-cased, kept when at most 96 bytes of segment characters: the **extra allowlist** |
| `UPDATE_PUBLISHER_ALLOWED_IPS` | comma list, each **normalized** (trimmed, a leading `::ffff:` removed, cut at the first `%`, lower-cased), empties dropped |
| `UPDATE_PUBLISHER_API_KEY` | trimmed (a secret) |
| `LAUNCHER_UPDATE_ROOT` | §3.4 |

Extension roles: rustic-rifts' `guard().guard_manifest_roles()` (`rifts_seam::Rifts`; none when dedicated-only).

Layout under the update root (C is the module's target changelist, decimal):

| Path | What |
| --- | --- |
| `runtime/C/<target>/<channel>/<platform>/latest.json` | the current runtime manifest |
| `runtime/C/<target>/<channel>/<platform>/<version>/` | `ParadoxRuntime-<version>-<target>-C.dll`, `manifest.json`, the extra files |
| `runtime/<target>/<channel>/<platform>/latest.json` | Node's layout: read (never written) when there is no current manifest and its `targetChangelist` is a JSON integer equal to C; its files then resolve under that folder |
| `guard/C/<channel>/<platform>/latest.json`, `guard/C/<channel>/<platform>/<sequence>/<manifestId>.json` | the current envelope and each immutable one |
| `guard/<channel>/<platform>/latest.json` | Node's layout: read when there is no current envelope, unless it is validly signed (§6.2 steps 2 and 3) for another changelist |
| `launcher/<target>/<arch>/latest.json`, `launcher/<target>/<arch>/<version>/<file>` | the launcher's own feed |
| `serving.json` | the serving switch `{"runtimeEnabled":<bool>,"updatedAt":<iso>}`; missing, unreadable or not exactly `false` reads enabled |

Files are read as UTF-8 and parsed as `JSON.parse` (`service_kit::jsjson`); an unreadable or unparseable file is
"none". **Located** means: target, channel and platform are segments (else the route's 400), then the current file,
else the legacy one under its rule.

### 6.2 The signed Guard manifest

The publisher signs offline; the server holds no private key. Two Ed25519 public keys are compiled in (SPKI base64;
the raw key is the last 32 bytes):

| `keyId` | Key | Use |
| --- | --- | --- |
| `runtime-2026-01` | `MCowBQYDK2VwAyEA3ZMtA7qUgs1F+1NQs2kmSG2zbOvXfjsh6+axI6eC/tc=` | the runtime DLL and its extra files; also accepted for Guard envelopes |
| `guard-2026-10` | `MCowBQYDK2VwAyEAcqOh1F/NOXtZ4ZSyefheKk0Md//2L85OgMiQH8Jyi/U=` | Guard envelopes |

**Verifying an envelope** for changelist C at time now (each failure is the error text shown, checked in this order):

1. An object with `schema` a safe integer equal to 1, and `keyId`, `payloadBase64`, `signature` all strings: else
   `Invalid Guard manifest envelope.`
2. `keyId` one of the two: else `Unknown Guard manifest signing key.`
3. The payload bytes (lenient base64) non-empty and at most 256 KiB, the signature 64 bytes (lenient base64), and the
   Ed25519 signature valid over the payload bytes: else `Guard manifest signature verification failed.`
4. The payload parses as JSON (`serde_json`): else the parser's error text.
5. Payload fields: `manifestId` a 36-byte text of hex digits and `-`; `sequence` a safe integer ≥ 1; `channel` a
   non-empty text of segment characters (`.` and `..` not refused here); `platform` exactly `windows-x86_64`;
   `minimumLauncherVersion` a **version** (`^\d+\.\d+\.\d+([-+][0-9A-Za-z.-]+)?$`); `issuedAt` and `expiresAt`
   strings; `artifacts` an array of 1 to 128; `schema` a safe integer 1 and `targetChangelist` a safe integer equal to
   C: else `Invalid Guard manifest payload.`
6. The window: both times `Date.parse` (else `Guard manifest timestamps are invalid.`); the expiry after the issue
   (else `Guard manifest expiry is not after its issue time.`); a lifetime of at most 365 days (else `Guard manifest
   lifetime exceeds the maximum allowed window.`); issued no later than now + 5 min (else `Guard manifest issue time
   is too far in the future.`); the expiry after now (else `Guard manifest has expired.`).
7. Each artifact: an object with string `name` and `role`; the name non-empty, without `/` or `\`, not repeated
   ignoring case; `size` a safe integer ≥ 1; `sha256` 64 hex digits (any case); `required` a boolean; the role one of
   `GAME`, `LAUNCHER`, `CONTENT`, `INTERNAL_SERVER`, `UDP_PROXY` or an extension role: else `Invalid Guard artifact
   entry.` A `protectedRegions` that is present and not `null` must be an array of at most 256 entries, each with
   `rva` a safe integer ≥ 0, `size` a safe integer 1 to 1 MiB and `sha256` 64 hex: else `Invalid Guard
   protected-region entry.`
8. `executableMemoryPolicy` an object with `maximumPrivateExecutableBytes` a safe integer 0 to 64 MiB,
   `maximumWritableExecutableBytes` absent or `null` (read 0) or a safe integer 0 to 16 MiB, and
   `allowlistedModuleHashes` an array of at most 512 64-hex strings: else `Invalid Guard executable-memory policy.`

The **verified payload**: manifest id, sequence, channel, platform, changelist, minimum launcher version, the
`expiresAt` text, the two memory limits, and each artifact as (role, name, lower-cased sha256).

**The published manifest** of (channel, platform) is the located envelope that verifies and whose payload channel
and platform equal the ones asked; anything else is none.

### 6.3 Gates shared by the feed routes

- **Download limit**: `update-download:<normalized client, or unknown>`, 180 per 60 s; refused: 429
  `{"error":"Too many requests. Please retry later."}`. **Publish limit**: `update-publish:<...>`, 30 per 60 s, the
  same answer.
- **Non-stable channels** (`isNonStableChannelAllowed`): a channel other than exactly `stable` needs §3.1's sign-in
  (its refusal answered as it is) by an account whose roles contain `admin`; any other signed-in account: 409
  launcher envelope `AUTH_CHANNEL_MISMATCH` / `Your account currently requires the stable runtime channel.`
- **Publisher**: the address check, then the key check. Address: the allowlist is non-empty and contains the
  normalized client address or the normalized peer. Key: the configured key is non-empty and `x-update-api-key`
  (JS-trimmed) equals it in constant time. The key refusal is always 401 `{"error":"Invalid update publisher
  credentials."}`; the address refusal's text is per route.
- **NO_CACHE** below is `no-cache, no-store, must-revalidate`; **immutable** is `public, max-age=31536000, immutable`.
- **Serving a file**: 200, `content-type: application/octet-stream`, `content-length`, and the route's cache header;
  a file that cannot be read: status text 404.

### 6.4 The runtime feed (players)

| Row | Route | Behaviour |
| --- | --- | --- |
| B182 | `GET /launcher/v1/runtime/:target/:channel/:platform` | target not `client`/`server`: status text 400; the non-stable gate; the serving switch off: status 204 with `cache-control: NO_CACHE` and no body (before the segment checks); located: a segment failure status text 400, none status text 404, else 200 with the manifest JSON as stored and `cache-control: NO_CACHE` |
| B183 | `GET /launcher/v1/runtime/:channel/:platform` | as B182 with target `client` |
| B184 | `GET /launcher/v1/runtime/:target/:channel/:platform/download` | download limit; the target check (400); the non-stable gate; located (400); the manifest's `version` a string that is not a segment: 400; the version missing, `file` not a string equal to its own basename, no manifest, or `<dir>/<version>/<file>` missing: status text 404; else the file, **immutable**. The serving switch does not apply |
| B185 | `GET /launcher/v1/runtime/:channel/:platform/download` | registered (target `client`) but never reached: B182 takes the path (core §4.3) |
| B190 | `GET /launcher/v1/runtime/:target/:channel/:platform/extra/:filename` | download limit; the target check; the non-stable gate; filename = the basename of the param, which must be a segment (else 400); located (400 / 404); `version` a string that is not a segment: 400, missing: 404; the manifest's `extraFiles` must be an array holding an entry whose `name` equals the filename exactly, and `<dir>/<version>/<filename>` must exist: else 404; else the file, **immutable** |

### 6.5 Publishing (the release tooling)

The bodies of B188 and B189 are read raw before anything else: only `application/octet-stream` (essence, ASCII
case-insensitive) is read, up to 200 MiB (over: the 413 error page); any other content type leaves no body. The file
work runs off the async threads.

**B188 `POST /launcher/v1/admin/updates/runtime/:target/:channel/:platform`:**

1. Publish limit; publisher address (refused: log warn `[update] publish denied for <normalized client>`, 403
   `{"error":"Update publishing is not allowed from this address."}`); publisher key.
2. Target, channel, platform and `x-update-version` segments; target `client`/`server`; platform exactly
   `windows-x86_64`; the version a version (§6.2 step 5); `Number(x-update-changelist)` equal to C; a body present:
   else 400 `{"error":"Invalid runtime update metadata."}`.
3. The body empty or over 200 MiB: 413 `{"error":"Runtime update is outside the allowed size."}`.
4. `x-update-signature` (lenient base64) 64 bytes and a valid Ed25519 signature of the body by the runtime key: else
   400 `{"error":"Runtime update signature verification failed."}`.
5. Under `runtime/C/<target>/<channel>/<platform>/<version>/`: `ParadoxRuntime-<version>-<target>-C.dll` already
   there: 409 `{"error":"That target/channel/version is already published and immutable."}`. Else it is created
   (create-new) with the body, and the manifest is written as pretty JSON to the version's `manifest.json` and to
   `latest.json`: `{"schema":1,"component":"ParadoxRuntime","target","version","channel","targetChangelist":C,
   "platform","size","sha256":<hex of the body>,"signature":<the header text as sent>,"file":<artifact name>,
   "publishedAt":<now>,"url":"<base>/launcher/v1/runtime/<enc target>/<enc channel>/<platform>/download",
   "extraFiles":[]}` (**enc** is `encodeURIComponent`: every byte but ASCII letters, digits and `-_.!~*'()`
   percent-encoded).
6. 201 `{"target","channel","version","sha256"}`. A file error: log error `[update] publish failed: {error}`, empty
   500.

**B189 `POST /launcher/v1/admin/updates/runtime/:target/:channel/:platform/extra`:**

1. Publish limit; address (warn `[update] extra-file publish denied for <ip>`, the same 403); key.
2. Target, channel, platform, `x-update-version` and the basename of `x-update-filename` segments: else 400
   `{"error":"Invalid extra-file metadata."}`. Target `client`/`server`, platform `windows-x86_64` and the filename
   lower-cased in the extra allowlist: else 400 `{"error":"Invalid extra-file target/platform/name."}`.
3. No body, an empty one or one over 200 MiB: 413 `{"error":"Extra file is outside the allowed size."}`.
4. Signature as B188 step 4 (`Extra file signature verification failed.`).
5. The current-layout `latest.json` must exist with `version` equal to the header's, and the version folder must
   exist: else 409 `{"error":"Publish the main runtime DLL for this version first."}`.
6. The file is written (replacing one of the same name); the entry `{"name":<the filename as sent>,"size","sha256",
   "signature":<standard base64 of the decoded signature>,"url":"<base>/launcher/v1/runtime/<enc target>/<enc
   channel>/<platform>/extra/<enc filename>"}` replaces any object entry of the same name (the others kept in order,
   non-objects dropped) and is appended; the manifest is rewritten to the version's `manifest.json` and `latest.json`.
7. 201 `{"name","sha256","size"}`; a file error: log error `[update] extra-file publish failed: {error}`, empty 500.

**B187 `POST /launcher/v1/admin/guard-manifests/:channel/:platform`** (body: JSON, 100 KiB, the error page on a
parse failure):

1. Publish limit; address: 403 `{"error":"Not allowed from this address."}` (no log); key.
2. Channel and platform segments: else 400 `{"error":"Invalid channel"}` / `{"error":"Invalid platform"}`. No JSON
   body: 400 `{"error":"Invalid Guard manifest envelope."}`. Verification (§6.2): its text as 400 `{"error":<text>}`.
3. The payload's channel and platform must equal the route's: else 400 `{"error":"Guard manifest route and signed
   payload do not match."}`.
4. The previous envelope: the current `latest.json`, else the legacy one when it verifies. When there is one it is
   verified again (a failure, an expired previous manifest included, is its text as 400) and the new sequence must be
   greater: else 409 `{"error":"Guard manifest sequence must increase monotonically."}`.
5. `guard/C/<channel>/<platform>/<sequence>/<manifestId>.json` exists: 409 `{"error":"That Guard manifest is already
   published and immutable."}`. Else it is written create-new (pretty JSON of the envelope as received), then
   `latest.json` the same; any file error: 400 `{"error":"Invalid Guard manifest."}`.
6. 201 `{"manifestId","sequence","keyId"}`.

**B186 `GET /launcher/v1/guard-manifests/:channel/:platform`:** the non-stable gate; located (a bad segment: status
text 400; none: status text 404); the envelope fails to verify: status text 400; its payload's channel or platform
differ from the route's: status text 404; else 200 with the envelope JSON and `cache-control: NO_CACHE`.

### 6.6 The serving switch (the publisher's pause)

| Row | Route | Behaviour |
| --- | --- | --- |
| B191 | `GET /launcher/v1/admin/updates/serving` | address: 403 `{"error":"Not allowed from this address."}`; key: 401; 200 `{"runtimeEnabled":<bool>}`. No limit |
| B192 | `POST /launcher/v1/admin/updates/serving` | the JSON body (100 KiB) first; address: warn `[update] serving toggle denied for <ip> (not in UPDATE_PUBLISHER_ALLOWED_IPS)`, 403 as B191; key: 401; `runtimeEnabled` not a boolean (no body included): 400 `{"error":"runtimeEnabled must be a boolean."}`; `serving.json` written as pretty JSON, folders created (an error: log error `[update] serving switch not saved: {error}`, empty 500); warn `[update] runtime update serving <ENABLED|DISABLED> by <ip>`; 200 `{"runtimeEnabled":<bool>}` |

### 6.7 The launcher's own feed (the Tauri updater)

- **Launcher version**: three dot-separated groups of 1 to 5 digits; compared group by group as numbers.
- **Allowed URL** for a prefix list: a string of at most 1024 UTF-16 units starting `https://`, with no `?` or `#`,
  whose authority (up to the first `/`) is non-empty and has no `@`, starting with one of the prefixes and longer
  than it.
- **Installer path** of (target, arch, version, file): the version a launcher version; the file
  `<stem>_x64-setup.exe`, the stem 1 to 120 of ASCII letters, digits, `.`, `_`, `-`, the file containing
  `_<version>_`; target and arch segments; then `launcher/<target>/<arch>/<version>/<file>`. Anything else: no path.

| Row | Route | Behaviour |
| --- | --- | --- |
| B195 | `GET /launcher/v1/updates/:target/:arch/:currentVersion` | target and arch segments: else status text 400. The manifest is `launcher/<target>/<arch>/latest.json`. **Not offered** (status 204, no body) when there is no manifest, or (the first that applies): its `version` is not a launcher version (`the manifest has no x.y.z version`); `signature` missing or empty (`the manifest has no signature`); `url` not allowed for the full prefix list (`the manifest's download url is not allowed`); the current version is a launcher version and the manifest's is not greater (`up to date`); the manifest has a string `file` and neither its installer path nor (when the file equals its basename) `<version>/<file>` exists (`the manifest's installer is not on this server`). **Offered**: 200 with `version`, `notes`, `pub_date`, `url`, `signature` copied in that order when present. Both carry `cache-control: NO_CACHE`. A reason other than `up to date`, with a manifest, logs warn `[update] launcher manifest not offered: <reason>` once per change of (manifest path, reason), per module |
| B193 | `GET /launcher/v1/updates/:target/:arch/download` | download limit; segments (400); with a string `file`, it must equal its basename and `version` must be a string segment (else 404), and `<dir>/<version>/<file>` must exist (else 404): served with `cache-control: NO_CACHE`. Without `file`: a `url` allowed for the **configured** prefixes only (the own base excluded): 302, `text/plain; charset=utf-8`, body `Found. Redirecting to <url>`, `location: <url>`, `vary: Accept`, `cache-control: NO_CACHE`; else (no manifest included) status text 404 |
| B194 | `GET /launcher/v1/updates/:target/:arch/files/:version/:file` | download limit; the installer path must exist as a regular file: else status text 404; served **immutable** with `content-disposition: attachment; filename="<file>"` |

### 6.8 What other parts read

`published_guard_manifest(channel, platform)` (§6.2) and its artifacts; `published_runtime_manifest(target, channel,
platform)` (the located manifest or none); `served_launcher_version(target, arch)` (the `version` of
`launcher/<target>/<arch>/latest.json` when it is a launcher version). §15.2 names them.

---

## 7. Launcher logs

### 7.1 Names

- **Session id**: 8 to 64 bytes of hex digits (any case) and `-`.
- **Launcher file names**: exactly `metadata.json` or `launcher.log`, or `runtime-rustic_isles_dll_port<digits>.log`
  (ASCII case-insensitive, at least one digit). Restricted also accepts the pre-rename
  `runtime-mysticparadox_dll_port<digits>.log`; this crate does not (§22).
- **Account folder**: 1 to 64 bytes of ASCII letters, digits, `-`.
- A session folder holds the uploaded files and `.uploaded-at.json` (`{"lastUploadedAt":"<iso>"}` and a newline).
  Its **upload time** is the marker's `lastUploadedAt` when it is a non-empty string that parses, else the folder's
  modification time.

### 7.2 `PUT /launcher/v1/logs/sessions/:launchSessionId/:fileName` (B196, testers)

1. §3.1's sign-in, **before** the body is read.
2. The body raw: `application/octet-stream` only, up to 10 MiB (over: the 413 error page).
3. Roles without `tester`: 403 launcher envelope `AUTH_TESTER_REQUIRED` / `Tester access is required to upload logs.`
4. `log-upload:<userId>` (120 per hour), then `log-upload-ip:<client>` (300 per hour; consulted only when the first
   passed): 429 `{"error":{"code":"LOG_UPLOAD_RATE_LIMITED","message":"Too many uploads. Try again later."}}`.
5. A bad session id or file name: 400 launcher envelope `AUTH_VALIDATION_FAILED` / `Invalid session id or file
   name.`; no or an empty body: `The uploaded file was empty.`; over 10 MiB: `The uploaded file is too large.`
6. The quota under `<log root>/<userId>/`: the session count (its sub-folders) and the total size of the files in
   them. A new session when the count is already 200: 413 `{"error":{"code":"LOG_UPLOAD_QUOTA","message":"Session
   limit reached for this account."}}`; the total plus this file over 200 MiB (a file being replaced still counts):
   the same code, `Storage quota reached for this account.`
7. Folders created, the file written (replacing), the marker written with now. 201 `{"uploadedAt":<iso>}`. A file
   error: log error `[logs] upload not saved: {error}`, empty 500.

### 7.3 The sync script's reads (B197, B198; mounted only with the admin API)

Each reads the JSON body first (a parse failure answers an **empty 400**, not the error page), then the sync token:
`LAUNCHER_LOG_SYNC_TOKEN` and `x-sync-token`, both JS-trimmed, both non-empty, equal in constant time; else 401
`{"error":"Invalid sync credentials."}`. The admin sign-in is not used.

- **B197 `GET /admin/v1/launcher-logs`**: `since` (the query value when given once) read with `Date.parse`; an
  empty or unparseable one filters nothing. Every account folder (sorted by name) that is a segment and a folder,
  and every session folder in it (sorted) that is a segment and a folder: its uploaded-at text is the marker's text
  as stored, else the folder's modification time as ISO; skipped when both it and `since` parse and it is not after
  `since`. Its files: every entry's name except the marker, sorted. 200 `{"sessions":[{"userId","launchSessionId",
  "uploadedAt","files"}...]}`, ordered by `locale_compare` on the uploaded-at text (a stable sort). No root: `[]`.
- **B198 `GET /admin/v1/launcher-logs/:userId/:launchSessionId/:fileName`**: the three params segments: else 400
  `{"error":"Invalid path."}`; not a file: status text 404; else 200 `application/octet-stream` with
  `content-length`.

### 7.4 The operator's view (B222 to B224, inside the admin half's route wrapper, §14.2)

Errors are admin errors (§2) with the codes below.

**Listing the uploads** of the root (optionally one account): only account folders and session ids, only real
folders (a symbolic link is never followed); a missing root gives none. Each upload's time in milliseconds is floored
for paging. A session's **summary**: `{"userId","launchSessionId","uploadedAt":<ISO of the floored time>,
"files":[{"name","size"}...],"totalBytes"}`, the files being the entries with launcher names that are regular files
(not links), sorted by name (bytes).

| Row | Route | Behaviour |
| --- | --- | --- |
| B222 | `GET /admin/v1/player-logs` | Query (a name given more than once is refused): `userId` (empty is none; not a segment: `userId must be one account id.`), `limit` (1 to 3 digits, 1 to 200, default 50; else `limit must be a whole number from 1 to 200.`), `before` (`YYYY-MM-DDTHH:MM:SS`, an optional `.` and 1 to 3 digits, then `Z`, and it must `Date.parse`; else `before must be a UTC time such as 2026-10-15T12:00:00.000Z.`); each refusal a 400 `PLAYER_LOGS_QUERY`. Uploads with a time below `before`, newest first, then userId, then session id (bytes). A page takes `limit` and then every further session with the same floored time as the last one taken. 200 `{"sessions":[<summary>...],"nextBefore":<ISO of the last taken time when more remain, else null>}` |
| B223 | `GET /admin/v1/player-logs/:userId/:launchSessionId/:fileName` | `download`: absent or `0` false, `1` true, anything else (or repeated) 400 `PLAYER_LOGS_QUERY` `download must be 1 or 0.` The file name not a segment: 400 `PLAYER_LOG_PATH` `The file name is not a valid file name.`; not a launcher name: 400 `PLAYER_LOG_NAME_REFUSED` `Only the launcher's log files can be read (metadata.json, launcher.log, runtime logs).`; the session located (below); the file not a regular file (a link included): 404 `PLAYER_LOG_NOT_FOUND` `No such file in that session.`; over 10 MiB (by its size, and again by the bytes read): 413 `PLAYER_LOG_TOO_LARGE` `That file is larger than the admin view serves.` 200 with `application/json; charset=utf-8` for `metadata.json`, else `text/plain; charset=utf-8`; `content-length`; `x-content-type-options: nosniff`; with `download`, `content-disposition: attachment; filename="<name>"` |
| B224 | `DELETE /admin/v1/player-logs/:userId/:launchSessionId` | `reason` (a body field, a string, JS-trimmed) of 8 to 200 UTF-16 units: else 400 `PLAYER_LOG_REASON` `Give a reason of 8 to 200 characters for the audit log.` The session located; its summary taken; the folder removed (a failure: 404 `PLAYER_LOG_NOT_FOUND` `No such uploaded session.`). Then one write as `Admin { key: <operator userId> }`: `admin.append_audit` with a fresh id, actor the operator's userId, target the userId param, action `player_logs.delete`, old state `{"launchSessionId","uploadedAt","files","totalBytes"}` from the summary, new state `{"deleted":true}`, the reason, the client address, a fresh request id, created now. The audit fails: log error `[PlayerLogs] a session was removed but its audit record could not be written (request <id>): {error}` and 500 `{"error":{"code":"PLAYER_LOG_AUDIT_FAILED","message":"The session was removed, but its audit record could not be written. Check the audit log."},"requestId":<id>}`. 200 `{"deleted":true,"requestId","userId","launchSessionId","files":<the file count>,"totalBytes"}` |

**Locating a session**: the userId and the session id segments, and the session id a session id: else 400
`PLAYER_LOG_PATH` `The account or session id is not a valid folder name.`; the root missing, either folder not a real
folder, or the session folder's canonical path not strictly inside the root's canonical path: 404
`PLAYER_LOG_NOT_FOUND` `No such uploaded session.`

### 7.5 Retention and removal (Privacy Policy section 5)

- **The 90-day purge** removes every listed session (§7.4's listing of the whole root) whose upload time is before
  now − 90 days, and reports the checked, expired, removed and failed counts. §12.3 runs it.
- **Removing an account's logs** (the erasure, §13): the userId must be an account folder (else the error `Refusing
  to remove uploaded logs for an account id that is not a plain folder name`); no folder: 0; else the folder is
  removed whole (an error: `The folder could not be removed`) and the count of its listed sessions answered. It runs
  off the async threads.

---

## 8. Launcher notices (`services/launcherNotices.ts`)

The notices are one record (`launcher_notices`, store §11.3.2): a version, who saved it and when, up to 3
announcements and up to 4 tags. The public read (`GET /launcher/v1/notices`) is auth's route (auth §3.4.9) answering
this crate's view through the `Notices` seam (§13).

### 8.1 The active window and the two views

- **Active at now**: a `showFrom` that is a string must `Date.parse` and now ≥ it; a `showUntil` that is a string must
  parse and now < it; a missing (or non-string) bound does not limit.
- **Public view** (record, now): `{"announcements":[...],"tags":[...],"serverTime":<ISO of now>}`, only active
  entries; each announcement keeps `id`, `text`, `tone`, `eventAt`, `showFrom`, `showUntil` (those present, in that
  order), each tag `id`, `text`, `tone`, `showFrom`, `showUntil`. No record: both lists empty.
- **Admin view**: `{"version":<0 when none>,"updatedAt":<text or null>,"updatedBy":<text or null>,"announcements":
  [<as stored>],"tags":[<as stored>],"serverTime":<now>}`.

### 8.2 Validating a save (each failure is the message; the first wins)

Lengths are counted in Unicode code points. **Control characters** are U+0000 to U+001F and U+007F to U+009F.

1. The body must be a JSON object: `The request body must be an object.`
2. `announcements` an array (`announcements must be a list.`) of at most 3 (`announcements holds at most 3
   entries.`); then `tags` an array (`tags must be a list.`) of at most 4 (`tags holds at most 4 entries.`); then every
   entry an object (`announcements[<i>] must be an object.`, `tags[<i>] ...`).
3. Each announcement, in order, label `announcements[<i>]`:
   - `id`: absent or `null` is none; else 1 to 64 bytes of ASCII letters, digits, `-`, `_`: else `<label>.id must be
     1 to 64 letters, digits, "-" or "_".`
   - `text`: a string (`<label>.text must be text.`), JS-trimmed, 1 to 280 (`<label>.text must be 1 to 280
     characters.`), no control character (`<label>.text must be one line of plain text (no tabs or line breaks).`),
     no `<` or `>` (`<label>.text must be plain text (no < or > characters).`), `{time}` at most once (`<label>.text
     may contain {time} once.`), and with every `{time}` removed no `{` or `}` (`<label>.text may only use the {time}
     placeholder.`).
   - `tone`: one of `info`, `warning`, `critical`: else `<label>.tone must be one of info, warning, critical.`
   - `eventAt`: absent or `null`, or an instant (below): else `<label>.eventAt must be an ISO time in UTC (for example
     2026-10-16T00:00:00.000Z).` A text holding `{time}` without an `eventAt`: `<label>.eventAt is required when the
     text contains {time}.`
   - `showFrom`, `showUntil`: as `eventAt` (`<label>.showFrom must be ...`); both given and `Date.parse(until) <
     Date.parse(from)`: `<label>.showUntil is before showFrom.`
   - Stored as: `id` first when given, then `text` (trimmed), `tone`, then `eventAt`, `showFrom`, `showUntil` when
     present (as normalized instants), then a generated `id` last when none was given.
4. Each tag, label `tags[<i>]`: `id` as above; `text` as above with at most 24 characters and, instead of the
   `{time}` rules, any `{` or `}` refused (`<label>.text does not take placeholders.`); `tone` one of `info`,
   `warning`, `danger`, `success` (`<label>.tone must be one of info, warning, danger, success.`); an `eventAt`
   present and not `null`: `<label> does not take eventAt.`; the window as above. Stored as `id`?, `text`, `tone`,
   `showFrom`?, `showUntil`?, generated `id`?.
5. A generated id is a fresh UUID v4, drawn in validation order (announcements, then tags).
6. An id used by two entries (announcements and tags together): `The id <id> is used by more than one notice or tag.`
7. `reason`: a string (else `A change reason of 8 to 200 characters is required.`), JS-trimmed, 8 to 200 code points
   (else `The change reason must be 8 to 200 characters.`).
8. `expectedVersion`: a JSON number that is a safe integer ≥ 0: else `expectedVersion must be the version you loaded
   (a whole number, 0 or more).`

**An instant**: a JSON string matching `YYYY-MM-DDTHH:MM`, optionally `:SS`, optionally (only after seconds) `.` and
1 to 9 digits, then `Z` or `±HH:MM`; then `Date.parse` (with `:00` seconds added when absent) must succeed; it is
stored as the ISO text of that instant (`2026-10-16T02:00+02:00` is `2026-10-16T00:00:00.000Z`;
`2026-10-16T00:00:00.5Z` is `...00.500Z`; `2026-10-16`, `2026-10-16T00:00`, `2026-10-16 00:00:00Z`,
`2026-10-16T00:00:00`, `2026-13-40T00:00:00Z` are refused).

### 8.3 The routes (inside the admin half's route wrapper)

| Row | Route | Behaviour |
| --- | --- | --- |
| B212 | `GET /admin/v1/launcher-notices` | `launcher_notices.get`: 200 the admin view; a read error: log error `[LauncherNotices] admin read failed: {error}` and 500 launcher envelope `INTERNAL` / `The launcher notices could not be read. Please try again.` |
| B213 | `PUT /admin/v1/launcher-notices` | §8.2 (a failure: 400 launcher envelope `NOTICES_INVALID` with the message). One write as `Admin { key: <operator userId> }`: the current record (none reads version 0, empty lists); its version not the expected one, or `replace_if_version(next, expected)` false: 409 launcher envelope `NOTICES_VERSION_CONFLICT` / `The launcher notices were saved by someone else. Reload them and apply your change again.` with the detail `current` (the admin view of the current record) first in the error object. Else the next record (version expected + 1, saved now by the operator's userId, the validated lists) is stored and the audit appended in the same write: action `launcher_notices.update`, no target, old and new state `{"version","announcements","tags"}`, the reason, the client address, a fresh request id (not answered), created at the save time. Log info `[LauncherNotices] saved version <v> by <userId>: <n> announcement(s), <m> tag(s)`; 200 the admin view of the saved record. A store error: log error `[LauncherNotices] save failed: {error}`, 500 `INTERNAL` / `The launcher notices could not be saved. Please try again.` |

---

## 9. The operations policy (`services/operationsPolicy.ts`)

One record (`operations_policy` row 1, store §11.3.1): Guard enforcement (`ENFORCE`/`OBSERVE`), the diagnostics
profile (`PRODUCTION`/`DEVELOPMENT`), a version, who changed it and when, and the expiry of a development lease.

### 9.1 Reading it

- **Normalize** (a stored record, now): malformed when the guard text is not `OBSERVE`/`ENFORCE` or the profile not
  `PRODUCTION`/`DEVELOPMENT`. Guard is `OBSERVE` only when not malformed and stored `OBSERVE`, else `ENFORCE`. The
  profile is `DEVELOPMENT` only when not malformed, stored `DEVELOPMENT` and the expiry parses to a time after now,
  else `PRODUCTION`; the expiry is kept only then. The version and the updated fields are kept as stored.
- **Resolve** (settings, record or none, now): no record: the default (`ENFORCE`, `PRODUCTION`, version 0, nothing
  else). A malformed one logs error `[OperationsPolicy] malformed global policy; forcing Guard ENFORCE and production
  diagnostics` (every time it is resolved). Then `GUARD_ENFORCEMENT_OVERRIDE` raw exactly `OBSERVE` forces `OBSERVE`.
- **The current policy**: resolve over `admin.read_operations_policy`; no store or a read error: none (callers use the
  default).
- **As JSON**: `{"guardEnforcement","diagnosticsProfile","version"}` then `updatedAt`, `updatedBy`,
  `diagnosticsExpiresAt` when set.

### 9.2 `GET /admin/v1/operations/overview` (B210, inside the admin wrapper)

One read: the policy record and `launcher_guard.list_sessions(no status, no minimum risk, skip 0, 100)`. Each session
becomes `{"accountId":<its userId, else the account id text>,"launcherVersion","manifestSequence","status",
"riskScore",["lastHeartbeatAt",]"expiresAt",["lastSignals",]"guardSessionId"}` (the bracketed ones only when set).
200 `{"observedAt":<now>,"policy":<resolved, as JSON>,"launcherGuard":[...]}`. A store error: log error `[ADMIN]
operations overview: {error}`, empty 500.

### 9.3 `PATCH /admin/v1/operations/policy` (B211, inside the admin wrapper)

1. `guardEnforcement` exactly `OBSERVE`/`ENFORCE`; `diagnosticsProfile` exactly `PRODUCTION`/`DEVELOPMENT`;
   `expectedVersion` = `Number(field)`, whole, ≥ 0 and ≤ 2^53−1 (so `"3"` and `null` (0) pass, a missing field does
   not); `reason` a string, JS-trimmed, its first 500 characters, at least 8 UTF-16 units. Any failure: 400 admin
   error `ADMIN_VALIDATION` / `A complete versioned policy and an audit reason of at least 8 characters are
   required.`
2. One write as `Admin { key: <operator userId> }`, with now taken once:
   - before = resolve(the stored record); its version not the expected one: **conflict**.
   - **Weakening codes** against before: `GUARD_DOWNGRADE` when before is `ENFORCE` and the request `OBSERVE`;
     `DEVELOPMENT_DIAGNOSTICS` when before is `PRODUCTION` and the request `DEVELOPMENT` (in that order).
   - With any code, the **step-up** must pass: the window `admin-operations-step-up:<operator userId>` (6 per 15 min,
     counted on every such attempt) not exhausted, a secret (`ADMIN_OPERATIONS_TOTP_SECRET`, else `ADMIN_TOTP_SECRET`,
     each JS-trimmed and non-empty), and auth's `verify_totp(secret, breakGlassTotp)` (auth §5.5). Failing: the audit
     `operations.policy.weakening_rejected` (old: before as JSON; new: `{"guardEnforcement","diagnosticsProfile",
     "weakeningCodes"}`) is written and the answer is 403 `{"error":{"code":"ADMIN_POLICY_STEP_UP_REQUIRED",
     "message":"A fresh operations authenticator code is required for this safety reduction."},"policy":<before>,
     "weakeningCodes":[...]}`.
   - The new record: the two values, `updated_at` now, `updated_by` the operator's userId, the diagnostics expiry
     (below); `update_operations_policy(expected, record)` false: **conflict**. After = resolve(the re-read record);
     the audit `operations.policy.update` (old: before; new: after as JSON plus `weakeningCodes`).
   - Every audit: a fresh id, the operator's userId, no target, the reason, the client address, the request's fresh
     id, created now.
3. Conflict: 409 `{"error":{"code":"ADMIN_POLICY_CONFLICT","message":"The operations policy changed. Refresh it before
   trying again."},"policy":<before>}`. Updated: 200 `{"policy":<after>,"requestId":<id>}`. A store error: log error
   `[ADMIN] operations policy: {error}`, empty 500.

**Diagnostics expiry** (current policy, requested profile, now): `PRODUCTION` none; a current `DEVELOPMENT` lease
whose expiry parses after now is kept (an unrelated edit does not renew it); else now + 2 hours.

### 9.4 What other parts read

The `Operations` seam (§13) answers the current policy's guard enforcement, profile and version (the default when
none). rustic-rifts' `IslesCore::operations_policy` and `verify_operations_step_up(key_id, code)` (restricted's
`AuthReforgedCore`) are normalize-plus-override as JSON text and the step-up rule above with `key_id` as the window's
user; the crate exposes both as public functions for that adapter (§15.2).

---

## 10. The activity monitor (`src/activity/*`)

### 10.1 Settings

`RUSTIC_ISLES_ACTIVITY_MONITOR` (raw, trimmed): `0`, `false`, `off`, `no` (ASCII case-insensitive) turn it off;
anything else, unset included, is on. Thresholds, each trimmed, empty or unset the default, else a decimal number
that is finite and ≥ 0 (Rust's float syntax), otherwise the default, silently:

| Setting | Default | Flag |
| --- | --- | --- |
| `RUSTIC_ISLES_KILLS_PER_HOUR_FLAG` | 90 | `KILLS_PER_HOUR` |
| `RUSTIC_ISLES_REWARDS_PER_HOUR_FLAG` | 600 | `REWARDS_PER_HOUR` |
| `RUSTIC_ISLES_PLAT_DAILY_FLAG` | 5000 | `PLATINUM_CREDIT_DAY` |
| `RUSTIC_ISLES_PLAT_SPEND_FLAG` | 10000 | `PLATINUM_SPEND_DAY` |
| `RUSTIC_ISLES_SEASONAL_COIN_DAILY_FLAG` | 20000 | `SEASONAL_COIN_DAY` |
| `RUSTIC_ISLES_XP_PER_MINUTE_FLAG` | 2500 | `XP_PER_MINUTE` |

`RUSTIC_ISLES_EXPECTED_LAUNCHER_VERSION` and `RUSTIC_ISLES_EXPECTED_LAUNCHER_BUILD_ID` are read (control characters
dropped, 64 and 96 characters, trimmed) and used by nothing (§20 item 16).

### 10.2 Days, keys and the daily document

- **Day** of a time: the first 10 characters of its ISO text. A **valid day** is `DDDD-DD-DD` whose midnight parses
  and gives the same day back. **TTL** of a day: its midnight + 400 days (the retention).
- **Safe key**: 1 to 80 bytes of ASCII letters, digits, `_` stays; anything else is `OTHER`.
- **The daily document** (`activity.add_to_daily`, `set_daily_fields`, store §11.3.3) per (account, day) holds dotted
  counters: `playtimeMinutes`, `sessions`, `hostingMinutes`, `huntsHosted`, `challengesClaimed`,
  `slayersPathUnlocks`, `reforges`, `huntsStarted.<kind>`, `huntsCompleted.<kind>`, `huntsAbandoned.<kind>` (kinds
  `dedicated`, `realm`, `playerHosted`), `kills.<key>`, `xp.<key>`, `itemsGained.<key>`,
  `platinumSpendsByItem.<key>`, `currency.<id>.credited|debited`, and the field `lastSeenMs`.
- **Recording increments** (account, time, increments): only with the monitor on and a non-empty list; one account
  write on the account's home, actor `System`: `add_to_daily` with the day's TTL, then each **daily flag** the new
  document raises is inserted if absent (`insert_flag_if_absent`): id `<KIND>:<userId>:<day>`, the kind, severity
  `review`, the account, the day, the detail, not acknowledged, created at the event's ISO time, the day's TTL.
- **Daily flags** of a document (in this order; numbers in the details are written as Rust's `{}` writes an `f64`:
  `5001`, `1.5`):
  - platinum credited (the sum over `currency.<id>.credited` where id is `CURRENCY_PLATINUM` or starts
    `CURRENCY_PLATINUM_`) above the credit threshold: `PLATINUM_CREDIT_DAY`, `credited=<n> threshold=<t>`;
  - platinum debited likewise above the spend threshold: `PLATINUM_SPEND_DAY`, `debited=<n> threshold=<t>`;
  - `currency.CURRENCY_SEASONAL_COIN.credited` above its threshold: `SEASONAL_COIN_DAY`, `credited=<n> threshold=<t>`;
  - only when `playtimeMinutes` ≥ 15: xp (the sum of `xp.*`) per minute above its threshold: `XP_PER_MINUTE`,
    `xp=<x> playtimeMinutes=<p> perMinute=<one decimal> threshold=<t>`; kills (sum of `kills.*`) per hour above:
    `KILLS_PER_HOUR`, `kills=<k> playtimeMinutes=<p> threshold=<t>`; items (sum of `itemsGained.*`) per hour above:
    `REWARDS_PER_HOUR`, `items=<i> playtimeMinutes=<p> threshold=<t>`.

### 10.3 The recorders (what other areas call; §13 and §15.2 name them)

Every recorder returns at once and does its work on a spawned task (nothing when no runtime is running); errors are
dropped silently; nothing happens without a store or with the monitor off. "Resolve" a user text: the game account
with that userId, else the text parsed as an account id, else nothing (the recorder stops). The time is taken when
the call is made.

| Recorder | Input | Effect |
| --- | --- | --- |
| Touch a game session | user text, time | Always: the recent-activity map (§12.1) notes the text at the time. Then, with a store and the monitor on: resolve; one account write (`System`): today's document's `lastSeenMs` (as an integer) is read; **playtime step**: no previous or ≤ 0: 0 minutes, 1 session, seen = now; now − previous ≤ 0: nothing, seen stays; a gap over 3 min: 0 minutes, 1 session, seen = now; else gap / 60 000 minutes, 0 sessions, seen = now; `set_daily_fields(lastSeenMs = seen)`; then the non-zero increments `playtimeMinutes`, `sessions` are recorded (§10.2) |
| An inventory transaction committed | the economy's `CommittedTransaction` (account, userId, transaction id, source, granted items, currency credits, currency debits) | Skipped for source `OPERATOR_TEST`. Granted items with `Number(quantity)` finite and > 0: a catalog id `TOKEN_KILL_COUNT_<rest>` (rest non-empty) adds the quantity to `kills.<safe rest>` and its truncated value to the kill tokens; any other adds to the item total and its safe key to the granted keys (at most 8 distinct). An item total > 0 adds to `itemsGained.<safe source>`. Each credit with a string `catalogId` adds `Number(amount)` (finite, > 0) to `currency.<safe id>.credited`; each debit likewise to `.debited`, and platinum debits (§10.2's ids) are summed: a sum > 0 adds it to `platinumSpendsByItem.<key>` for each granted key. Increments for one path merge (first appearance order). Record them. Then, when the kill tokens are > 0 and the source is `INVENTORY_GAMESERVER`: one account write: the account's dedicated hunts covering now (`find_dedicated_hunts_covering(at now, not before now − 6 h, grace floor now − 10 min)`): the first gets `add_hunt_kills(tokens)`; none: the flag `LOOT_WITHOUT_HUNT:<userId>:<day>`, severity `high`, detail `transactionId=<id> killTokens=<n> source=<source>` |
| An XP grant | user text, progression id, amount | Amount > 0 and a non-blank user: resolve; record `xp.<safe progression id>` += amount |
| Challenges claimed | user text, count | Count > 0: resolve; `challengesClaimed` += count |
| Slayers' Path unlocks | user text, node ids, source | Skipped for source `OPERATOR_TEST`; node ids trimmed, empties dropped (none: nothing); resolve; one account write: each unlock `{id: "<user text trimmed>:<node>", account, node, unlocked now, source trimmed, the day's TTL}` inserted if absent; the count inserted > 0: `slayersPathUnlocks` += it |
| A reforge | user text | Resolve; `reforges` += 1 |
| Dedicated hunt starts | user texts, hunt id, hunt ref | The users trimmed, empties dropped, de-duplicated in order; for each (resolve, else skip): one account write: every open dedicated hunt of the account (`find_open_dedicated_hunts`) closed (`close_hunt_if_open(ended now, "SUPERSEDED", abandoned)`), then the hunt `{id: "dedicated:<hunt ref>:<user>", account, host false, kind dedicated, hunt id, hunt ref (both trimmed), started now, no end, kills 0, the day's TTL}` inserted if absent; increments `huntsStarted.dedicated` (1 when inserted) and `huntsAbandoned.dedicated` (the closed count) |
| Dedicated hunt ends | user texts, end reason | The result is `completed` when the trimmed reason equals `ALL_GONE` ignoring ASCII case, else `abandoned`; per user: the latest open dedicated hunt closed with the trimmed reason and that result; when it closed: `huntsCompleted.dedicated` or `huntsAbandoned.dedicated` += 1 |

### 10.4 The launcher build (the `Activity` seam's two build calls)

- **Classify** (report): the pinned hash is the `LAUNCHER` artifact's sha256 of the published Guard manifest for channel
  `stable` (every account's runtime channel, auth §6.6; restricted passes the requested one) on platform `win64`,
  and the served version is `served_launcher_version("windows", "x86_64")`; auth's
  `launcher_build::launcher_verdict(report, pinned, served)` decides. Because a published manifest's platform
  is always `windows-x86_64` (§6.2 step 5), the `win64` lookup never finds one: **the verdict is always unknown**
  today (parity; §21 item 1 has the fix).
- **Record** (userId, report, verdict): only for an `unofficial` verdict with a pinned hash, the monitor on and a
  store: resolve; the flag `UNOFFICIAL_LAUNCHER:<userId>:<day>`, severity `high`, detail `launcher <version> sha256
  <first 16 of the report's hash>... is not the official build (Guard <channel> pins <first 16 of the pin>...; served
  version <served or none>)`. Never reached while classify is unknown.

### 10.5 The admin reads (inside the admin wrapper; admin errors)

**Range** (`from`, `to`, each a query value given once, trimmed): `to` empty is today; `from` empty is `to` − 29 days
(a `to` that does not parse here: 400 `ACTIVITY_RANGE_INVALID` `from and to must be UTC days, YYYY-MM-DD.`); both must
be valid days (the same 400); `from` after `to` (text order): 400 `ACTIVITY_RANGE_INVALID` `from must not be after
to.`; more than 400 days inclusive: 400 `ACTIVITY_RANGE_TOO_LONG` `The range may be at most 400 days (the
retention).`

**Totals** of documents: start from `playtimeMinutes`, `sessions`, `hostingMinutes`, `huntsHosted`,
`challengesClaimed`, `slayersPathUnlocks`, `reforges` (each 0), `huntsStarted`, `huntsCompleted`, `huntsAbandoned`
(each `{"dedicated":0,"realm":0,"playerHosted":0}`), `kills`, `xp`, `itemsGained`, `platinumSpendsByItem` (each `{}`),
`currency` (`{}`), in that order; each document adds its numbers (non-numbers read 0; a new key of the open groups is
appended; a currency entry is `{"credited","debited"}`). Whole numbers are written as integers.

**A flag** as answered: `{"id","kind","severity","userId":<the account's userId, else its id text>,"day","detail",
"status":<"open"|"acknowledged">,"createdAt","acknowledgedAt","acknowledgedBy":<a stored account id text read as its
userId, any other text as it is, or null>,"note"}`.

| Row | Route | Behaviour |
| --- | --- | --- |
| B217 | `GET /admin/v1/activity/players/:userId` | The param trimmed, empty or over 200 bytes: 400 `ACTIVITY_USER_INVALID` `The account id is missing or too long.`; the range. One read: resolve the param (none: every list empty); `find_daily_rows(from, to, 400)`, `find_hunts_for_account(50)`, `find_unlocks_for_account(200)`, `find_flags_for_account(50)`, `inventory_transactions.list_recent_for_account(50)`, `progression_grants.list_recent_for_account(50)`. 200 `{"userId":<the param trimmed>,"from","to","monitorEnabled","days":[<each document without the keys _id, userId, ttlAt>],"totals","unlocks":[{"nodeId","unlockedAt","source"}],"hunts":[{"role":<"host"|"participant">,"kind","huntId","huntRef","startedAt","endedAt","endReason","result","kills"}],"flags":[...],"timeline":[...]}`. A read error: log error `[ActivityMon] player read failed: {error}`, 500 `ACTIVITY_READ_FAILED` `The activity record could not be read.` |
| B218 | `GET /admin/v1/activity/overview` | The range; `find_daily_rows_in_range(from, to, 50 000)`; `truncated` when 50 000 came back. One entry per day from `from` to `to`: `{"day","activePlayers":<documents with playtimeMinutes or sessions > 0>, ...the day's totals}`. 200 `{"from","to","monitorEnabled","truncated","days"}`. Error: `[ActivityMon] overview read failed: {error}`, 500 `ACTIVITY_READ_FAILED` `The activity overview could not be read.` |
| B219 | `GET /admin/v1/activity/flags` | `status` (once, trimmed; empty: `open`): `open`, `acknowledged`, `all`, else 400 `ACTIVITY_FLAG_QUERY_INVALID` `status must be open, acknowledged or all.`; page and limit by the admin half's readers (default 25, maximum 100); `list_flags`. 200 `{"flags","total","page","limit","status"}`. Error: `[ActivityMon] flag list failed: {error}`, 500 `ACTIVITY_READ_FAILED` `The flags could not be read.` |
| B220 | `PATCH /admin/v1/activity/flags/:id` | The id empty or over 300 bytes: 400 `ACTIVITY_FLAG_INVALID` `The flag id is missing.`; `acknowledged` not exactly `true`: 400 `ACTIVITY_FLAG_INVALID` `Send {"acknowledged": true} (an optional note of up to 500 characters).`; `note` a string, JS-trimmed, its first 500 characters, empty is none. One write as `Admin { key: <operator userId> }`: no flag: 404 `ACTIVITY_FLAG_NOT_FOUND` `No activity flag has that id.`; `acknowledge_flag(id, <operator account>, now, note)`; newly acknowledged: the audit `activity.flag.acknowledge` (target the flag's userId, old `{"status":"open"}`, new `{"status":"acknowledged"}`, reason the note, the client address, a fresh request id, now). 200 `{"flag":<as answered>}`. Error: `[ActivityMon] flag acknowledge failed: {error}`, 500 `ACTIVITY_WRITE_FAILED` `The flag could not be acknowledged.` |

**The timeline** (at most 100, by `at` text descending, equal times keeping this order): each hunt with a start
`{"at":<start>,"type":"hunt","detail":<the hunt as above>}`; each unlock `{"at","type":"slayersPathUnlock",
"detail":{"nodeId","unlockedAt","source"}}`; each flag `{"at":<created>,"type":"flag","detail":{"id","kind",
"severity","status"}}`; each ledger row (`at` = `completedAt` when a non-empty string, else `createdAt`)
`{"at","type":"inventory","detail":{"status","economySource","economySessionId","characterId"}}` (missing ones
`null`); each grant `{"at":<created>,"type":"xp","detail":{"progressionId","amount","grantId"}}`. Entries with an
empty time are left out.

---

## 11. Launcher Guard (`controllers/launcherGuard.ts`; isles keeps it, RF1 decision 1)

A launcher opens a session against the current signed manifest, then sends signed heartbeats about the game process;
the server scores each and keeps the verdict. Sessions are `launcher_guard_sessions` and findings `guard_findings`
(store §11.3.4).

### 11.1 Entry (B225 to B227)

The JSON body is read first (100 KiB; a parse failure is the error page), then §3.1's sign-in (its refusal as it
is), then the store (none: core's 501). Refusals are launcher envelopes with the codes below (statuses from auth
§2.3: `GUARD_REPORT_INVALID` 400, `GUARD_SESSION_INVALID` 401, `GUARD_SIGNATURE_INVALID` 401,
`GUARD_MANIFEST_MISMATCH` 409, `GUARD_SEQUENCE_CONFLICT` 409, `GUARD_MANIFEST_UNAVAILABLE` 503). A store error logs
error `Unhandled Launcher Guard error: {error}` and answers 500 `INTERNAL` / `Launcher Guard failed.` Writes are
account writes as `Player { account }`. Success is 200 with the JSON shown.

### 11.2 `POST /launcher/v1/guard/sessions` (start)

1. `role`: absent or `null` is `client`; `client` or `host`; anything else: 400 `GUARD_REPORT_INVALID` `The Launcher
   Guard role is invalid.`
2. `channel` (a string, else `stable`); `launcherVersion` (a string, else `""`).
3. The published manifest for (channel, `windows-x86_64`) whose expiry parses after now: none: 503
   `GUARD_MANIFEST_UNAVAILABLE` `No current signed Guard manifest is available.`
4. `manifestId` equal to its id, `manifestSequence` a safe integer equal to its sequence, the launcher version a
   version (§6.2) and **at least** the manifest's minimum (§11.5): else 409 `GUARD_MANIFEST_MISMATCH` `The launcher
   runtime manifest is not current.`
5. `publicKey` a string whose lenient base64 is 32 bytes: else 400 `GUARD_REPORT_INVALID` `The ephemeral Guard public
   key is invalid.`
6. A session id (UUID v4), a challenge (24 CSPRNG bytes, unpadded base64url, 32 characters), created now, expiring now
   + 5 min. One write: `revoke_for_account(account, role, at the creation time)`, then `create` with last sequence 0,
   status `CHALLENGED`, risk 0, no heartbeat, no signals, no reasons, no history, the manifest's id and sequence, the
   channel and launcher version.
7. `{"schema":1,"role","guardSessionId","challenge","manifestId","manifestSequence","expiresAt",
   "heartbeatIntervalMs":15000}`.

### 11.3 `POST /launcher/v1/guard/heartbeat`

1. `guardSessionId` (a string, else `""`) of 36 bytes of hex digits and `-`; `sequence` = `Number(field)` whole, ≥ 1
   and ≤ 2^53−1; `observedAt` a string that `Date.parse`s; `signature` a string of 80 to 100 UTF-16 units: any failing:
   400 `GUARD_REPORT_INVALID` `The Launcher Guard heartbeat is invalid.` The same when `observedAt` is more than 60 s
   from now either way.
2. `signals` (§11.4): 400 `GUARD_REPORT_INVALID` `The Launcher Guard report is invalid.`
3. The session: of this account, its last sequence below the sent one, status not `REVOKED`/`EXPIRED`, and expiring
   after now: else 401 `GUARD_SESSION_INVALID` `The Launcher Guard session is invalid or expired.` A `role` that is
   sent and parses (`client`/`host`) but differs from the session's: 401 `GUARD_SESSION_INVALID` `The Launcher Guard
   role does not match the session.` (an unparseable role text is ignored).
4. **The signature** is Ed25519 by the session's public key (lenient base64, 32 bytes) over the 17 lines joined by
   `\n`: `MYSTIC-GUARD-2`; the session's role; the session id; the sequence (decimal); the session's stored challenge;
   its manifest id; its manifest sequence; `observedAt` as sent; `String(gameProcessId)`; `gameStartedAt` as sent;
   `String(moduleCount)`; `moduleSetDigest` lower-cased; `String(privateExecutableBytes)`;
   `String(writableExecutableBytes)`; `String(unexpectedModuleCount)`; `String(protectedPageMismatchCount)`; `1` or `0`
   for `debuggerPresent` (`String` is `service_kit::js::string` of the JSON value as sent). The signature is lenient
   base64 of 64 bytes. Invalid: 401 `GUARD_SIGNATURE_INVALID` `The Launcher Guard heartbeat could not be
   authenticated.`
5. The published manifest for the session's channel must still have the session's id and sequence and expire after
   now: else 409 `GUARD_MANIFEST_MISMATCH` `The Launcher Guard manifest changed; restart the game.`
6. **Score**: explain the signals with the manifest's memory limits, plus continuity against the session's stored
   signals (read back through §11.4; unreadable is none) and its last sequence; risk = min(100, the sum); reasons =
   the signal reasons then the continuity reasons; status `HEALTHY` when there is no hard violation and risk < 50,
   else `DEGRADED` (§11.5).
7. Now; a next challenge; expiry now + 5 min. `advance` with the expected (stored) challenge, the sequence, the next
   challenge, the status, the risk, the heartbeat time, the expiry, the stored signals, the reasons and the history
   entry `{"at":<heartbeat time>,"status","riskScore":<risk>,"reasons"}`; none came back: 409
   `GUARD_SEQUENCE_CONFLICT` `The Launcher Guard heartbeat was superseded.`
8. **Findings**: for each distinct (file name, sha256) of this report's module list, a sighting (file name, sha256,
   signer, size, account, seen at the heartbeat time, new in session = absent from the previous report's list), each
   its own account write; a failure logs warn `[GuardFindings] could not record <file name> for account=<account id>:
   {error}` and is otherwise ignored.
9. `{"schema":1,"accepted":true,"role","status","riskScore":<risk>,"reasons","nextChallenge","expiresAt"}`.

### 11.4 The report (`signals`)

An object with: `gameProcessId`, `moduleCount`, `privateExecutableBytes`, `writableExecutableBytes`,
`unexpectedModuleCount`, `protectedPageMismatchCount` safe integers ≥ 0; `gameStartedAt` a string that parses;
`moduleSetDigest` 64 hex digits (any case); `debuggerPresent` a boolean. `unexpectedModules`: absent or `null` is no
list; else an array of at most 32 objects, each: `fileName` a string of 1 to 260 UTF-16 units, not `.` or `..`, with
no `/`, `\`, no character up to U+001F and none of `< > : " | ? *`; `signer` a string of 1 to 256 units with no
character up to U+001F; `sha256` 64 hex digits (stored lower-cased); `size` a JSON number that is a safe integer 0 to
2^40. Anything else: the report is invalid. **Stored** as every key the launcher sent (unknown keys kept), the module
list replaced by the normalized entries `{"fileName","signer","sha256","size"}` in the same key position.

### 11.5 Scoring

- **Untrusted and signed modules**: of the listed modules, those whose trimmed signer is not `unsigned` or `invalid`
  (ASCII case-insensitive) are signed, capped at `unexpectedModuleCount`; untrusted = the count − signed.
- **Signal reasons and score** (in this order): debugger present +50 `DEBUGGER`; untrusted > 0: + min(50, 15 each)
  `UNEXPECTED_UNSIGNED:<n>`; signed > 0: + min(10, 2 each) `UNEXPECTED_SIGNED:<n>`; page mismatches > 0: + min(60, 30
  each) `PAGE_MISMATCH:<n>`; writable bytes above the writable limit +60 `WRITABLE_EXEC_OVER`; private bytes above the
  private limit +40 `PRIVATE_EXEC_OVER`; the score capped at 100.
- **Hard violation**: a debugger, any untrusted module, any page mismatch, or either limit exceeded.
- **Continuity** against the previous report: none: 0; a different `gameProcessId` value or `gameStartedAt` text:
  100 `PROCESS_CHANGED`; else, when the session's last sequence is at least 2 and the digest text differs: 50
  `MODULE_SET_CHANGED`; else 0.
- **Version at least** (`VersionAtLeast`, `security/launcherVersion.ts`; not auth's semver comparison): both sides
  JS-trimmed; a `+build` and a `-prerelease` split off (each, when present, non-empty of ASCII letters, digits, `.`,
  `-`); the core three non-empty digit groups, compared as integers (a group too long to read makes the side
  unreadable); unreadable: false. Cores differ: their order decides. Equal: the actual without a prerelease is at
  least; the actual with one against a minimum without is not; else the dot-separated identifiers pairwise: the
  actual's list ending first answers **true**, the minimum's ending first answers false; equal identifiers continue;
  two numeric ones compare as integers; numeric against alphanumeric: the numeric one is lower; else byte order. All
  equal: true.

### 11.6 `GET /launcher/v1/guard/sessions/:guardSessionId` (status)

The session by id, which must be the account's: else 401 `GUARD_SESSION_INVALID` `Launcher Guard session not found.`
(no expiry check). `{"schema":1,"role","guardSessionId":<the param>,"status","riskScore","manifestId",
"manifestSequence",["lastHeartbeatAt",]"expiresAt"}` (`lastHeartbeatAt` only after a heartbeat).

### 11.7 The dashboard's Guard views (`controllers/adminGuard.ts`; B214 to B216, inside the admin wrapper)

Read-only. A session as the dashboard sees it never carries its challenge or public key:
`{"guardSessionId","accountId":<the account's userId, else its id text>,"role","status","riskScore","reasons",
"channel","launcherVersion","manifestId","manifestSequence","lastSequence","createdAt","lastHeartbeatAt":<text or
null>,"expiresAt","lastSignals":<the stored report or null>}`. Page and limit are the admin half's readers (default
25, maximum 100); the offset is (page − 1) × limit. Errors are admin errors.

| Row | Route | Behaviour |
| --- | --- | --- |
| B214 | `GET /admin/v1/guard/sessions` | `status` (once, JS-trimmed): empty is any, else one of `CHALLENGED`, `HEALTHY`, `DEGRADED`, `REVOKED`, `EXPIRED` (exact case): else 400 `GUARD_QUERY_INVALID` `status must be one of CHALLENGED, HEALTHY, DEGRADED, REVOKED, EXPIRED`; `minRisk` (once, JS-trimmed): empty is none, else `Number(...)` must be a whole number 0 to 100: else 400 `GUARD_QUERY_INVALID` `minRisk must be a whole number from 0 to 100`. `launcher_guard.list_sessions(status, minRisk, offset, limit)` (histories emptied). 200 `{"sessions":[...],"total","page","limit"}`. A store error: log error `[ADMIN] guard sessions: {error}`, empty 500 |
| B215 | `GET /admin/v1/guard/sessions/:guardSessionId` | an empty id or none found: 404 `GUARD_SESSION_NOT_FOUND` `No Guard session has that id.`; else 200 `{"session":<as above>,"history":<the stored history, newest first>}`. Error: `[ADMIN] guard session: {error}`, empty 500 |
| B216 | `GET /admin/v1/guard/findings` | `signer` (once, JS-trimmed, first 100 characters; empty is none); `launcher_guard.list_findings(signer, offset, limit)`. 200 `{"findings":[{"fileName","sha256","signer","size","firstSeenAt","lastSeenAt","sessionCount","accountCount"}...],"total","page","limit"}` (the account list is never answered). Error: `[ADMIN] guard findings: {error}`, empty 500 |

---

## 12. State and background work

### 12.1 Who is online

Core's presence (core §5.5) is the chat sessions. This crate adds the **recent-activity map**: user text to the last
time a game session was touched (§10.3), per module. When it grows past 50 000 entries on a touch, entries older than
that touch's time − 1 hour are dropped. `recently_active(window)`: the texts seen within `window` ms of now
(inclusive). `active_players(window)`: core's online user ids, then the recent texts, each once, in that order.
Windows used: 60 s (the admin's online players), 2 min (the release gates), 3 min (the erasure blockers, core §7.6's
`Lookup`).

### 12.2 The status cache

§5.4's two cached bodies, per module. `forget_status()` drops both; called by registration (§5.2) and, in the admin
half, by the host's status change and kick (its §9) and core §7.6's admin actions (its §10) (§14.1).

### 12.3 Background jobs (started by the crate's start-up, core §11.4 `startups`, only with a store)

Each runs once at start and then every hour (a slow run delays the next; runs never overlap); each is aborted at stop.

| Job | When | Work | Logs |
| --- | --- | --- | --- |
| Log purge | `RUSTIC_ISLES_LOG_PURGE` JS-trimmed is `1`, `true`, `on` or `yes` (ASCII case-insensitive) | §7.5's 90-day purge off the async threads | info `[LogPurge] removed <n>, failed <m>` when either is non-zero |
| TTL purge | always (with a store) | one write as `System`: `purge_expired_ttl_rows(now)` (store §11.3.6) | info `[TtlPurge] purged <n> expired row(s)` when non-zero; warn `[TtlPurge] run failed: {error}` |

The 30-day erasure job is auth's (auth §8.2); this crate only answers its seam (§13).

---

## 13. The seams implemented here

### 13.1 `rustic_isles_auth::Seams` (auth §11.2; `crates/rustic-isles-auth/src/seams.rs`)

The crate provides one implementation of each trait below over the module's shared state; the composer passes them
in `Composition.auth_seams` (core §11.4). `realms`, `game_login`, `home`, `mail` and `discord` are other units'.

| Trait | Method | This crate's answer |
| --- | --- | --- |
| `Activity` | `touch_game_session(user_id, at_ms)` | §10.3's touch with that user text and time (the recent map always; the daily document with a store and the monitor on). The future completes at once; the store work is spawned |
| | `classify_launcher_build(report)` | §10.4's classify (channel `stable`): today always `Unknown` |
| | `record_launcher_build(user_id, report, verdict)` | §10.4's record |
| | `touch_matchmaking_presence(user_id, at_ms)` | forwarded to the matchmaking presence slot (§13.2); none: nothing |
| `Operations` | `policy()` | the current policy (§9.1) as `OperationsPolicy { guard_enforcement, diagnostics_profile, version }`; none: the default |
| `Notices` | `public_view(now_ms)` | no store: the empty view (`rustic_isles_auth::empty_notices(now_ms)`); `launcher_notices.get` fails: `Err(<its text>)` (auth answers the empty view and logs once per streak); else §8.1's public view at `now_ms` |
| `Erasure` | `blockers(account)` | in order: `account is currently online` when the account's userId is in `active_players(3 min)` (§12.1); `account has an active matchmaking candidate` when the candidate slot (§13.2) says so. Never `Err` |
| | `remove_uploaded_logs(account)` | §7.5's removal for the account's userId under the log root, off the async threads; its error text as `Err` (auth warns and goes on) |

The other areas' rows of an erased account are not this seam's: the store's `AuthStore` adapter erases them in the
same write (store §11.3.6, §12).

### 13.2 Slots this crate defines (filled by the composer; each defaults to nothing)

| Trait | Contract | Filled by |
| --- | --- | --- |
| `MatchmakingPresence` | `touch(user_id, at_ms)` | matchmaking (unit 8); restricted's composer today |
| `MatchmakingCandidates` | `has_candidate(user_id) -> bool` | matchmaking |

Chat connections need no slot: core's `Presence` (core §5.5) answers the online ids and closes connections.

### 13.3 What this crate offers other areas (through its slot)

| Caller | Call | §  |
| --- | --- | --- |
| Inventory (`Observer::committed`, `Observer::reforge_reset`; economy-inventory §11.3) | `record_inventory_transaction(&CommittedTransaction)`, `record_reforge(user text)` | 10.3 |
| Progression (activity port; economy-progression §9.5) | `record_xp_grant(user, track, amount)`, `record_slayers_path_unlocks(user, keys, source)` | 10.3 |
| Bounties (`ClaimActivity`; economy-bounties §6.4) | `record_challenge_claims(user, count)` | 10.3 |
| Matchmaking (its hunt-activity hook) | `dedicated_hunt_starts(users, hunt id, hunt ref)`, `dedicated_hunt_ends(users, end reason)` | 10.3 |
| Core (§11.4 `observer`) | the observer | 4.5 |
| rustic-rifts' `IslesCore` adapter | `operations_policy_json()`, `verify_operations_step_up(key id, code)` | 9.4 |

Each economy port's adapter lives with the composer: it converts the economy's call into this crate's recorder.

---

## 14. Calls between the two halves

Names follow the admin half's §15, so the two implementers write against one list.

### 14.1 The public half offers the admin half (its §15.2)

| Call | Contract |
| --- | --- |
| `ReforgedState` from the shared state (`Shared::slot`) | the per-module state of §12 (one value, both halves) |
| `reforged_api::is_account_eligible(&LauncherAccount) -> bool` | §3.3's rule (no timeout check; the admin half's §2.5): the admin sign-in, the Discord sign-in, the access change |
| `forget_status()` | §12.2: after the host's status change and kick, core §7.6's kick, ban and timeout |
| `recently_active(window_ms)`, `active_players(window_ms)` | §12.1: online players (60 s), release gates (2 min), `Lookup` (3 min) |
| `update_root(&Settings) -> PathBuf`, `log_storage_root(&Settings) -> PathBuf` | §3.4; the release gates look for `RELEASE_DURING_PLAY.allow` beside the update root |
| §5.1's rules: `RegistrationMode`, `normalize_username`, `is_invite_code`, `make_invite_code_with`, `invite_note`, `Refusal` | the host routes (the admin half's §9) |
| `encode_uri_component(&str) -> String` | §6.5's set (the admin cookies' and redirects' encoding) |
| `remove_account_logs(root, user_id)` | §7.5 (the admin player deletion does not call it today; §21 item 13) |
| `launcher_sign_in(&Shared, &HeaderMap) -> Result<LauncherCaller, Response>` | §3.1, should an admin route need it |

### 14.2 The admin half offers the public half (its §15.1)

| Call | Contract the public half relies on |
| --- | --- |
| `admin_route(call, Required, handler)` | The admin half's §3.1 wrapper: the body (100 KiB; an error page without admin headers), the `Origin` check (a disallowed origin: empty 403), no store: core's 501 with the admin headers, the session or manager authentication (its 401/403 answers with the admin headers), then the handler; the admin headers (`cache-control: no-store`, the CSP, the CORS echo for an allowed origin) on the handler's answer. B210 to B220 and B222 to B224 run inside it; B197 and B198 do not |
| `AdminContext` | the shared state, the store, the request (head, query pairs, raw body, fields), the decoded params, and `caller: Some(AdminCaller)` with the operator's userId and account (the audit actor and the store actor `Admin { key: <userId> }` via `admin_actor`) |
| `admin_error(status, code, message)` | §2's admin error |
| `page_of(request)`, `limit_of(request, default, max)`, `skip_of(page, limit)` | the list readers (the activity flags and the Guard views) |
| `user_id_of(account)` | the game account's userId, else the account id text |

The audit records both halves append go through `admin.append_audit` (store §11.3.1) with the fields each route lists;
the admin half's `GET /admin/v1/audit` reads them. Attempt windows of both halves share the module's auth limiter
(§3.2); this half's keys run on core's replay-aware clock (restricted's), the admin half's on its own choice (its
§2.4).

---

## 15. The public seam (the crate)

### 15.1 Shape

One crate, `rustic-isles-reforged` (lib), MIT, in dainty-lass `crates/rustic-isles-reforged`, holding both halves.
It depends on dainty-lass MIT crates (`rustic-isles`, `rustic-isles-auth`, `rustic-isles-store`, `rustic-rifts-seam`,
`module-api`, `service-kit`) and workspace dependencies (`axum`, `http`, `tokio`, `tracing`, `serde`, `serde_json`
with `preserve_order` in its own manifest, `sha2`, `base64`, `regex`, `percent-encoding`, `aws-lc-rs` for Ed25519
verification and random bytes, `subtle`). No new workspace dependency is needed for this half (restricted's
`ed25519-dalek` and `uuid` are replaced by `aws-lc-rs` and core's UUID formatting); the integration pass adds
`crates/rustic-isles-reforged` and `rustic-isles-reforged` to `[workspace.dependencies]` (a seam change, not this
row's).

### 15.2 Interface (names are the contract the composer, the admin half and later units use)

| Item | Contract |
| --- | --- |
| `pub struct ReforgedState` | §12: `new()`; `forget_status()`, `recently_active`, `active_players`, `touch(user, at)`, `readiness()`, `set_readiness(ready, reason)`, `metrics() -> &Metrics`; one per module, stored as a slot |
| `pub struct Metrics` | §4.4: `increment`, `set`, `adjust`, `observe`, `register_section(Section)`, `render(role, ready, changed_at_seconds) -> String`; `Section { render, hidden_gauges }` |
| `pub struct RequestObserver` | implements `rustic_isles::Observer` over a state's metrics (§4.5); `sanitize_url_for_log(&str)`, `metric_class(&str)` |
| `pub struct ReforgedSeams` | `activity()`, `operations()`, `notices()`, `erasure()` as the `Arc<dyn ...>` auth wants (§13.1) |
| `pub trait MatchmakingPresence`, `pub trait MatchmakingCandidates` | §13.2 |
| `pub mod activity` | §10.3 recorders (taking `&Shared`), `ActivitySettings::read(&Settings)`, `daily_flag_candidates`, `inventory_increments`, `advance_playtime`, `classify_dedicated_end_reason`, `ActivityInput` (§10.3's transaction fields) |
| `pub mod operations` | `Policy` (`default_policy`, `to_json`), `normalize`, `resolve`, `weakening_codes`, `diagnostics_expiry`, `current_policy(&Shared)`, `operations_policy_json(&Shared)`, `verify_operations_step_up(&Shared, key_id, code)` |
| `pub mod notices` | `parse_instant`, `validate_input(body, new_id)`, `validate_reason`, `validate_expected_version`, `is_active`, `public_view`, `admin_view` |
| `pub mod updates` | `published_guard_manifest`, `published_runtime_manifest`, `served_launcher_version`, `update_root`, `encode_uri_component`, `decide_launcher_update`, `launcher_installer_path`, `is_launcher_version` |
| `pub mod guard_manifest` | `Trusted` (`embedded()`, `with_keys(runtime, guard)` for tests), `GuardPayload`, `verify_envelope(value, changelist, extension_roles, now, &Trusted)`, `validate_window`, `is_version`, `is_segment` |
| `pub mod guard` | `Signals::parse`, `UnexpectedModule`, `signature_material`, `explain_signals`, `explain_continuity`, `hard_violation`, `guard_status`, `version_at_least` |
| `pub mod logs` | `is_session_id`, `is_launcher_file_name`, `is_account_folder`, `log_storage_root`, `list_uploaded`, `purge_expired(root, now, days)`, `remove_account_logs(root, user)`, `RETENTION_DAYS` |
| `pub mod reforged_api` | `RegistrationMode`, `normalize_username`, `is_invite_code`, `make_invite_code_with(pick)`, `invite_note`, `make_account_key`, `looks_like_account_key`, `is_account_eligible`, `Refusal`, `status_instances`, `instance_title`, `asset_leaf`, `GameDataKind`, `classify_game_data` |
| `pub fn register_public(handlers: &mut rustic_isles::Handlers)` | §15.3 |
| `pub fn public_startups() -> Vec<..>` | §12.3's jobs as core start-ups |
| feature `test-support` | `Trusted::with_keys`, a fixed-time helper, a recording `MatchmakingPresence` |

### 15.3 Registration and composition

`register_public` registers every row of §1.2's public list on the backend service, by the exact path text of the
route table (core §5.7; a typo fails the build with `UnknownRoute`). The admin half's `register_admin` registers the
rest; the composer calls both. The composer also: creates one `ReforgedState` and stores it as a slot; passes
`RequestObserver` as the observer; passes `ReforgedSeams` into `auth_seams`; fills §13.2's slots; and adds
`public_startups()`.

---

## 16. Settings

Read through core's settings (`raw` unless said) at each use; no new setting, `host.toml` field or env file line comes
with this crate. Secrets never reach a log or a `Debug` (core §17).

| Setting | Use | Secret |
| --- | --- | --- |
| `RUSTIC_ISLES_SERVICE_ROLE` | §4.1 | |
| `RUSTIC_ISLES_METRICS_TOKEN`, `NODE_ENV` | §4.3 | the token |
| `REGISTRATION_MODE` | §5.1 | |
| `API_KEY_HASH_SECRET` | §3.3 (through auth's hash) | yes |
| `REFORGED_SERVER_NAME`, `REFORGED_SERVER_VERSION`, `REFORGED_SOURCE_COMMIT`, `REFORGED_SOURCE_URL`, `APPROVED_EXECUTABLE_SHA256`, `REALTIME_XMPP_ENABLED`, `DEPLOYSERVER_URL` | §5.4 | |
| `LAUNCHER_UPDATE_ROOT`, `UPDATE_PUBLIC_BASE_URL`, `LAUNCHER_UPDATE_URL_PREFIXES`, `RUNTIME_EXTRA_FILES`, `UPDATE_PUBLISHER_ALLOWED_IPS` | §6.1 | |
| `UPDATE_PUBLISHER_API_KEY` | §6.3 | yes |
| `LAUNCHER_LOG_STORAGE_DIR` | §3.4 | |
| `LAUNCHER_LOG_SYNC_TOKEN` | §7.3 | yes |
| `RUSTIC_ISLES_LOG_PURGE` (switch `log_purge`, core §6.2) | §12.3 | |
| `GUARD_ENFORCEMENT_OVERRIDE` | §9.1 | |
| `ADMIN_OPERATIONS_TOTP_SECRET`, `ADMIN_TOTP_SECRET` (`get`: non-empty) | §9.3 (`ADMIN_TOTP_SECRET` is also the admin sign-in's) | yes |
| `RUSTIC_ISLES_ACTIVITY_MONITOR`, the six thresholds, `RUSTIC_ISLES_EXPECTED_LAUNCHER_VERSION`, `RUSTIC_ISLES_EXPECTED_LAUNCHER_BUILD_ID` | §10.1 | |

The update feed's and the log sync's secrets are compared, never stored or answered; the admin key is hashed before
comparison.

---

## 17. Logs

English, a machine contract for the host (AGENTS.md), never a setting's value, a key, a token or an account key:
every line quoted in §4.5, §5, §6, §7, §8, §9, §10.5, §11 and §12.3. Levels: `[REQ]`/`[RES]`, registrations,
the notices save, the log and TTL purge counts are info; publish and serving denials,
the serving toggle, the launcher manifest refusal, `[GuardFindings]` and `[TtlPurge] run failed` are warn; every
"failed"/"not saved"/"could not" line and the malformed policy line are error. The invite note and display names are
host- or player-chosen text and appear in lines; the account key never does.

---

## 18. Test cases

### 18.1 What may be copied

Nothing recorded: `tests/golden/reforged_api.json` (and its capture script) stays in dainty-restricted and is
replayed there against this crate once composed; a failure there is a bug on this side. The implementer neither copies
nor reads it. `tests/reforged_api.rs` and the unit tests of §1.2's files are authored test code, described below only.
The two public keys and the wire texts are interface facts. Tests that sign envelopes or uploads make their own key
pair (`Trusted::with_keys`).

### 18.2 Behaviour the authored tests fix (synthetic values; rewrite as black-box or unit tests)

| Area | Cases |
| --- | --- |
| Probes | `GET /` 200 `ok`, `text/html; charset=utf-8`; live `{"ok":true,"service":"paradox-backend","role":"combined"}`; `RUSTIC_ISLES_SERVICE_ROLE= API ` gives `api`; ready with a store `{"ok":true,...,"reason":"READY","persistence":"ready"}`; without one 503 `{"ok":false,"service":"paradox-backend","role":"combined","reason":"API_BEHIND_PROXY_RUNNING","persistence":"unavailable"}`; maintenance `{"show-status":false}` as JSON; `/dauntless-status` the welcome object, keys in the order of §4.1 |
| Metrics route | loopback, no token: 200, `text/plain; charset=utf-8; version=0.0.4`, `cache-control: no-store`, contains `rustic_isles_service_ready 1\n`; `NODE_ENV=production`: 503 `metrics unavailable\n` `text/plain; charset=utf-8`; a token: none sent 401, `Bearer wrong` 401, `Bearer <token>` 200, `x-rustic-isles-metrics-token: <token>` 200; `x-mysticparadox-metrics-token` is not read (decision) |
| Exposition | labels `zeta=x, method=GET, class="bad value", status=2xx` keep `method`, `status` in that order, written `{method="GET",status="2xx"}`; `Bad Name` files under `rustic_isles_invalid_metric`; increments 1 and 2 give `3`; `adjust(-3)` on a new gauge gives `0`; an observation of 30 gives `le="25"` 0, `le="100"` 1, `le="+Inf"` 1, `_sum` 30; the text starts with the info HELP line, contains `rustic_isles_info{role="combined",service="metagame"} 1`, `..._changed_timestamp_seconds 12.5`, `# TYPE <counter> counter`, ends with exactly one newline |
| Request log | `/notification/x?token=abc.def&x=1` → `?token=[REDACTED]&x=1`; `?CODE=secret&state=ok&access_token=another` → `?CODE=[REDACTED]&state=ok&access_token=[REDACTED]`; `/balance?currency=c` unchanged; `/account/api/oauth/sessions/kill/eyJ...sig` → `/account/api/oauth/sessions/kill/[REDACTED]`; `/x/eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ4In0.sigvalue/y` → `/x/[REDACTED-JWT]/y`; classes `/admin/v1/me` admin, `/health/live` health, `/launcher/v1/updates/x` launcher, `/character` game; `/mystic/v1/hunts/abc/lease` quiet, `/mystic/v1/hunts/abc/end` not; a quiet route answered 200 logs nothing, 409 logs both lines |
| Registration | `REGISTRATION_MODE=OPEN` answers `{"mode":"OPEN"}` as JSON; ` open ` OPEN, `none` NONE, `whatever` and unset INVITECODE; NONE refuses before the body; no body 400 `bad_request`; `InviteCode: null` or a number 400 `bad_request`; `ab`, `a b c`, 17 characters, `née_ab` `username_invalid`; `  Fox_9 ` is `Fox_9`; a 200 body has exactly one key `UUK`, `RRK_` + 48 hex (52 characters), and `x-reforged-account-id`; the same name in another case 409 `username_taken`; invite mode (codes inserted through the store): no code 401, ` <code> ` (spaces) 200, a 1-use code twice: 200 then 401, a name taken costs no use (the code still works), a 2-use code shows `usesRemaining` 2 and `note` `null` |
| Codes and keys | a picker returning 0, 1, 2, ... gives `ABCD-EFGH-JKMN`; codes are 14 characters of the alphabet; `abc-1` is a code, `abc` and `a b c d` and 65 letters are not; notes: `  for\u0000Alex\n` → `for Alex`, spaces → none, 100 × `x` → 64, a number → none; keys are `RRK_` + 48 and look like keys, `short` and `a b c d e f g` do not |
| GetUserInfo | no key or an odd key 401 `{"error":"unauthorized"}`; a registered key 200 `{"UserId","Username":"Fox_9","IsAdmin":false}` with the account id header; banned with reason `cheating`: 403 `account_banned` `cheating`; disabled: `account_disabled`; no reason: `Ask the server's host.`; back to active: 200 |
| ServerStatus | limited: 200, `no-store`, `vary: x-reforged-user-api-key`, keys exactly in §5.4's order; `REFORGED_SERVER_NAME=Fox hold`, version `1.2.3`, `https://example.com/src` kept; `http://...` → `null`; `APPROVED_EXECUTABLE_SHA256` = 64 upper-case hex → lower-cased in `game`, `{"build":"rel-1.14.7-Archon","changelist":647472,...}`; no folder `missing`; chat `false` unless exactly `true`; full with a key: `limited:false`, `0`, `[]`, `[]` with no Director |
| Instances | §5.4 rules: `{"id":"h1","kind":"hunt","map":"/Game/Maps/islands/foo/foo_persistent","behemoth":"/Game/Behemoths/BPC_Skarn_Prime.BPC_Skarn_Prime_C","players":3.7,"maxPlayers":5,"startedAt":"2026-10-20T00:00:00Z","playerIds":["a",7,"b"]}` → `{"id":"h1","kind":"hunt","title":"Skarn Prime","map":"foo_persistent","behemoth":"BPC_Skarn_Prime","players":3,"maxPlayers":5,"startedAt":"2026-10-20T00:00:00.000Z"}` and players `a`, `b` in `h1`; a city with `players: -2` → `Ramsgate`, 0, `startedAt` null; an unknown kind, a string, an entry without id skipped; a map `/Game/Maps/Marsh_Of_Fog_persistent.Marsh` → `Marsh Of Fog`; a hunt with nothing → `Hunt`; an object answer → none; HTTP answers: plain and chunked JSON read, a 500 or non-JSON → none |
| Game data | missing files → `missing`; both present → `real`; an equal `.example.json` → `synthetic`; a different example → `real` |
| Runtime feed | (with a test key pair and a temp root) a published manifest served at both manifest routes with `NO_CACHE`; missing → 404 `Not Found`; a bad target `x` → 400; `..` as a segment → 400; an unreadable `latest.json` → 404; `server` target works; serving off → 204 empty with `NO_CACHE`, also before a bad segment; an unreadable switch reads on; download with the file → 200 octet-stream, length, immutable; the file missing, `file: "a/b.dll"`, `version: "../x"` → 404 / 404 / 400; the shadowed two-segment download path answers as B182; extra listed and present 200, unlisted 404, listed but missing 404 |
| Publishing | a wrong address 403 (each route's text), no key 401, a key of another length 401; metadata wrong 400; a wrong changelist header 400; a bad signature 400; a good upload 201 then the same version 409; the manifest written has §6.5's keys in order; extra before the main DLL 409, then 201 and listed with its URL; guard: an empty envelope 400 `Invalid Guard manifest envelope.`, an unknown key, a bad signature, a sequence not increasing 409, the same manifest again 409, a route/payload mismatch 400 |
| Envelope rules | §6.2 window table at now `2026-10-20T00:00:00Z`: 2026-10-19 to 2026-11-19 ok; `soon` invalid; reversed; 2026-01-01 to 2028-01-01 too long; issued 00:06 too far, 00:05 ok; expired; versions `1.2.3`, `10.20.30-beta.1`, `0.1.65+build-7` yes, `1.2`, `1.2.3.4`, `a.b.c`, `1.2.3-`, `1.2.3-é`, `` no; envelopes `{}` and `schema: 2` invalid, `keyId: other` unknown key, `payloadBase64: "e30="` with signature `AAAA` signature failure; both embedded keys parse and refuse a zero signature |
| Launcher feed | an update offered with Tauri's five keys in order; up to date 204; a newer current 204; an unreadable current version offered; installer missing 204; no manifest 204; no signature, an `http://` url, a foreign url, a url with `?`, version `1.2` → 204 with one warning per change; a configured release prefix allowed; `LAUNCHER_UPDATE_URL_PREFIXES` set replaces the GitHub default; download: file 200 `NO_CACHE`, missing 404, a configured-prefix url 302 `Found. Redirecting to <url>`, the own base url 404, `file: "a/b"` 404; installer by version 200 with the disposition; `x.exe`, a name without `_<version>_`, a missing file → 404 |
| Logs | upload: no sign-in 401; not a tester 403 `AUTH_TESTER_REQUIRED`; bad names 400; empty 400; 201 `{"uploadedAt"}` and the marker; the 201st session 413; names `metadata.json`, `launcher.log`, `runtime-rustic_isles_dll_port7777.log`, `RUNTIME-Rustic_Isles_DLL_PORT1.LOG` allowed; ``, `other.log`, `runtime-rustic_isles_dll_port.log`, `...portx.log`, `../launcher.log`, `Metadata.json`, `launcher.log.exe`, `runtime-mysticparadox_dll_port7777.log` refused; session ids `abcdef12-3456`, 64 × `a` yes, `short`, `xyzxyzxyz` no. Sync: no token, a wrong token, none configured → 401; list oldest first (`u2` at 10-01 before `u1` at 10-02), files sorted, `bad name` skipped; `since=2026-10-01T12:00:00Z` → `u1` only; `since=garbage` → both; no root → `[]`; a file 200 octet-stream; missing 404; `..` → 400 `Invalid path.` |
| Operator's view | newest first: `aaaaaaaa-1` (u1, 10-03), `bbbbbbbb-2` (u2, 10-03), `cccccccc-3` (u1, 10-01); `bad name` skipped; the first summary `{"userId":"u1","launchSessionId":"aaaaaaaa-1","uploadedAt":"2026-10-03T00:00:00.000Z","files":[{"name":"launcher.log","size":4},{"name":"metadata.json","size":2}],"totalBytes":6}` (`evil.exe` not listed); limit 1 takes both 10-03 sessions and `nextBefore` `2026-10-03T00:00:00.000Z`, the next page `cccccccc-3`; `userId=u2` one; `../x` none. Read: `launcher.log` text, `metadata.json` JSON; `other.txt` `PLAYER_LOG_NAME_REFUSED`; `..`, a short session, `../u1` `PLAYER_LOG_PATH`; another user, an absent runtime log, no root `PLAYER_LOG_NOT_FOUND`; 10 MiB + 1 → 413 `PLAYER_LOG_TOO_LARGE`. Delete: `totalBytes` 7 and 2 files, the sibling session kept, a second delete 404, `..` `PLAYER_LOG_PATH`; a short reason 400 `PLAYER_LOG_REASON`; an audit row written |
| Retention | sessions at 06-01 (u1), 10-20 (u1), 06-02 (u2) and a folder `not an account` at now 2026-10-26: checked 3, expired 2, removed 2, failed 0; a second pass expires 0; removing `u1` → 1 then 0; `../u1` and `` refused; `not an account` untouched |
| Notices | §8.2's message for each case: `null`, `[]` not an object; missing lists; 4 announcements; a non-object entry; text missing, empty, with a tab, `<b>`, `{time}{time}`, `{name}`, `at {time}` without `eventAt`; tone `loud`; id `a b`; `showFrom: yesterday`; until before from; a tag `x{y}`, a tag with `eventAt`, a 25-character tag; a shared id. A good save stores `{"text":"Maintenance at {time}","tone":"warning","eventAt":"2026-10-16T12:00:00.000Z","showUntil":"2026-10-17T00:00:00.000Z","id":<generated>}` (id last) and `{"id":"kept","text":"Hello","tone":"info"}`. Instants of §8.2. Reasons `  fixing the date  ` → `fixing the date`, `short` refused, none refused; versions 0 and `4.0` accepted, -1, 1.5, `"1"`, `null` refused. Public view at 10-20: `now` (10-01 to 11-01) shown, `later` (from 12-01) and `over` (until 10-10) not; keys `announcements`, `tags`, `serverTime`; none → empty lists; the admin view of none `{"version":0,"updatedAt":null,"updatedBy":null,"announcements":[],"tags":[],"serverTime":..}`. Routes: a stale `expectedVersion` 409 with `current` first; a save bumps the version and appends one audit |
| Operations | weakening from the default to `OBSERVE`/`DEVELOPMENT`: both codes in order; no change, or a tightening, none. Normalize at 2026-10-26T12:00Z: `OBSERVE`/`DEVELOPMENT` until 13:00 kept; until 11:00 → `PRODUCTION`, no expiry; no expiry → `PRODUCTION`; `MAYBE` → malformed, `ENFORCE`/`PRODUCTION`, version kept; JSON `{"guardEnforcement":"OBSERVE","diagnosticsProfile":"DEVELOPMENT","version":3,"updatedAt":"u","updatedBy":"b","diagnosticsExpiresAt":"2026-10-26T13:00:00.000Z"}`; the default `{"guardEnforcement":"ENFORCE","diagnosticsProfile":"PRODUCTION","version":0}`. Expiry: a running lease kept, `PRODUCTION` none, a new or lapsed one now + 2 h. Routes: a short reason 400; a stale version 409 with `policy`; a weakening without a code 403 and the rejected audit written; with a valid code 200 and the update audit; the override `OBSERVE` read back |
| Activity | `advance_playtime`: none → (0, 1, now); a 2-minute gap → (2 minutes, 0); a 4-minute gap → (0, 1); a backwards clock → nothing. Flags: platinum credited 5001 → `PLATINUM_CREDIT_DAY` `credited=5001 threshold=5000`; 14 minutes of play with huge kills → no rate flags; 60 minutes and 91 kills → `KILLS_PER_HOUR`. Increments: a kill token of 3 → `kills.<name>` 3 and 3 tokens; an `OPERATOR_TEST` source → nothing; a platinum debit spreads over at most 8 granted keys. Day helpers: `2026-02-30` invalid, `2026-02-28` valid. Admin routes: a range over 400 days 400 `ACTIVITY_RANGE_TOO_LONG`; `from` after `to` 400; the player view keys in order; a flag acknowledged once writes one audit and twice answers the same flag |
| Guard | signature material: 17 lines, line 1 `MYSTIC-GUARD-2`, line 2 the role, line 4 the sequence, line 12 the digest lower-cased, line 17 `0`; client and host differ. Roles: none → client, `host`, `worker` refused. Continuity: a digest change at last sequence 1 → 0, at 2 → 50 `MODULE_SET_CHANGED`; another process id → 100 `PROCESS_CHANGED`. A manifest is current until its expiry, not after, not for another id or sequence. Memory: private 512 KiB / writable 32 KiB within 1 MiB / 64 KiB → 0 and healthy; writable + 1 → 60; private + 1 → 40; one page mismatch → 30; one unexpected module → 15, each degraded. Each cause: debugger 50, 2 unexpected 30, 5 unexpected 50, 3 mismatches 60, the limits 60 and 40; everything 100 with reasons in §11.5's order. A signed listed module → 2 `UNEXPECTED_SIGNED:1`, healthy; `unsigned`, ` Invalid `, `UNSIGNED` → hard; a listed module beyond the count adds nothing; 9 counted with 6 signed listed → 55 `UNEXPECTED_UNSIGNED:3`, `UNEXPECTED_SIGNED:6`. Report fields refused one by one as §11.4 (empty, `..`, `a/b.dll`, `a\b.dll`, `a:b.dll`, a control character, 261 units, a number; signer empty, with a newline, 257; sha `z`×64, 63 chars; size -1, 1.5, `"1"`, 2^40 + 1, missing); 33 modules refused, 32 accepted; an extra key kept. `version_at_least`: `0.1.65`≥`0.1.65` yes, `0.1.66`≥`0.1.65` yes, `0.1.64` no, `1.0.0`≥`0.9.9` yes, `0.1.65-beta.1`≥`0.1.65` no, `0.1.65`≥`0.1.65-beta.1` yes, `beta.2`≥`beta.1` yes, equal yes, `alpha`≥`1` yes, `1`≥`alpha` no, `rc.1`≥`rc` **no**, `rc`≥`rc.1` **yes**, `0.1.65+build`≥`0.1.65` yes, `x`, `y`, `0.1` no. Flow (a test key pair, a published manifest): start → `CHALLENGED`; a signed heartbeat → `HEALTHY`, a new challenge; a replayed sequence 401 `GUARD_SESSION_INVALID`; an old challenge 401 `GUARD_SIGNATURE_INVALID`; another account's session 401; a debugger and an unsigned module → `DEGRADED` 65 with both reasons and one finding; a process change → 100 `PROCESS_CHANGED`; history newest first, at most 20; a new start revokes the role's old session |
| Guard views | a session row never has `challenge` or `publicKeyBase64`; `status=healthy` (wrong case) 400 `GUARD_QUERY_INVALID`; `minRisk=50.5` and `101` 400, `50` filters `riskScore >= 50`; an unknown id 404 `GUARD_SESSION_NOT_FOUND`; one session read answers its history; findings filtered by a signer substring, ignoring case, `total` the filtered count |
| Seams | the notices seam with no store → the empty view; a failing read → `Err`; the erasure blockers name an online account; `remove_uploaded_logs` removes the folder; the operations seam answers the default without a store |
| Registration of handlers | every row of §1.2's public list has a handler and no other row is registered by this half (a build of the backend app with both halves leaves none of the 62 rows to the 501) |

---

## 19. Security behaviour (for `docs/security/rustic-isles-reforged.md` and RX3)

The crate's checklist is one file shared by the two halves; this half contributes the items below.

- **Untrusted input**: every launcher body (Guard reports, uploads, notices saves through the admin wrapper), every
  path parameter that becomes a file name, every update upload and envelope, and the Director's status answer. File
  names are checked as segments before any path is joined; the operator's view never follows symbolic links and
  checks the canonical path stays inside the root; the publisher's writes are create-new where they are immutable.
- **Signatures**: the server verifies, never signs; only the two compiled-in Ed25519 keys are trusted; uploads and
  envelopes are refused before anything is written when the signature fails.
- **Secrets**: the publisher key, the sync token and the metrics token are compared in constant time; account
  keys are stored only as auth's HMAC and compared in constant time over every row. None is logged; the request log
  redacts tokens, codes, signatures and JWT-shaped segments.
- **Trust boundary**: the publisher routes require an allowlisted address (the client address behind a loopback
  proxy, or the peer) and a key; the sync routes their own token; the operator's views, notices, operations, activity
  and Guard views the admin wrapper (the admin half's checklist covers it and the host routes).
- **Limits**: uploads 10 MiB per log file, 200 sessions and 200 MiB per account, 120 uploads per hour per account and
  300 per address; runtime uploads 200 MiB; envelopes 256 KiB payload; JSON bodies 100 KiB; Guard reports 32
  modules, names 260, signers 256; the Director read 2 s; the status 500 players and 100 instances.
- **Known weak spots to list**:
  - The log upload joins the caller's userId into the path without a check (launcher accounts have UUID userIds, so
    it holds today; §21 item 6).
  - The sync routes (B197, B198) follow symbolic links and serve any file in a session folder; they sit behind the
    admin API switch and the sync token only.
  - The launcher-build verdict is always unknown (§10.4), so `RUSTIC_ISLES_UNOFFICIAL_LAUNCHER=refuse` never refuses.
  - The metrics route is open to loopback without a token outside `NODE_ENV=production`.
  - Attempt windows are in memory and per module (a restart resets them).

**Fuzz targets (`fuzz/fuzz_targets/`):**

- `rustic_isles_reforged_guard`: arbitrary JSON as a Guard report, a heartbeat body and an envelope (with a fixed
  test key pair and a fixed time), and arbitrary text pairs for `version_at_least`: never panics; a parsed report's
  module list is at most 32 and every stored sha is lower-case hex; a verified envelope always has 1 to 128 artifacts
  and the requested changelist; the risk is always 0 to 100.
- `rustic_isles_reforged_text`: the url sanitizer (the output never contains a value following `token=` and never an
  `eyJ...` JWT shape the rule matches), metric names and labels (the rendered text always parses as one sample per
  line), the notices validation and instant parser, the username, code, note, key and segment checks, lenient
  base64, `status_instances` over arbitrary JSON (at most 100 instances, titles at most 64 characters), the launcher
  installer path and URL rules: never panic.
- `rustic_isles_reforged_paths`: arbitrary segments and file names through every path builder of §6 and §7 against a
  temporary root: a built path is always inside the root.

---

## 20. Behaviour that looks wrong but is kept (parity)

1. **Register** refuses `NONE` before reading the body, ignores (does not spend) a code in `OPEN` mode, and refuses
   `InviteCode: null`.
2. **Key holder eligibility** ignores timeouts (a timed-out player still gets `GetUserInfo` 200 and the full status),
   and a pending approval or an unset username is reported as `account_disabled`.
3. **ServerStatus**: `executableSha256` is `null` when `APPROVED_EXECUTABLE_SHA256` holds a list (auth reads it as a
   list); `sourceUrl` accepts only the scheme spellings `https://` and `HTTPS://`; the players list is the chat
   presence only, while the erasure and the admin views also count recent game sessions.
4. **The operations overview** shows the first 100 Guard sessions of any status, newest first; the dashboard's
   session list empties each history while the single-session read answers it whole.
5. **Metrics**: a token of spaces answers 401 while the loopback rule applies; an `authorization: Bearer` header with
   an empty rest blocks the `x-rustic-isles-metrics-token` fallback; the `service` label is `metagame`.
6. **The serving switch** pauses only the runtime manifests; downloads and extra files keep being served.
7. **Extra files** may be replaced (they are not immutable), and a missing body is 413 there but 400 on the main DLL.
8. **A Guard publish** is refused while the current envelope no longer verifies (for example once it has expired):
   its verification error is the answer (§21 item 10).
9. **The Director read** uses `DEPLOYSERVER_URL` as a host:port authority; a value with a scheme gives no instances,
   silently (§21 item 7).
10. **The Guard payload's JSON error** is the parser's own text.
11. **A heartbeat's unknown role text** is ignored; only a valid different role is refused.
12. **`version_at_least`** treats the side whose prerelease list ends first as the greater (`0.1.65-rc` ≥
    `0.1.65-rc.1`), unlike auth's semver comparison.
13. **The weakening check** compares against the resolved policy, so with `GUARD_ENFORCEMENT_OVERRIDE=OBSERVE` a change
    to `OBSERVE` needs no step-up; the operations `expectedVersion` is read with `Number(...)` (so `"3"` and `null`
    pass) while the notices one must be a JSON number.
14. **The malformed-policy error line** is logged at every resolve.
15. **The sync listing** answers a broken body with an empty 400, lists every file name (not only the launcher's) and
    shows the marker's text as stored, while the operator's view normalizes it.
16. **Activity settings**: `RUSTIC_ISLES_EXPECTED_LAUNCHER_VERSION`/`_BUILD_ID` are read and unused; the seasonal-coin
    threshold defaults to 20 000 here and 1000 in bounties (economy-bounties' parity list, item 14) from the same
    setting; the spend threshold here is `RUSTIC_ISLES_PLAT_SPEND_FLAG` (10 000) while bounties reads
    `RUSTIC_ISLES_PLAT_DAILY_SPEND_FLAG`.
17. **The quota** counts a file's old size while it is being replaced.
18. **Readiness** takes its time the first time it is read.
19. **The exposition** writes every counter's HELP/TYPE before any counter sample and repeats a gauge's HELP/TYPE per
    label set.
20. **A guard status read** does not check the session's expiry.

---

## 21. Open questions

1. **The launcher-build verdict is dead** (§10.4): the pinned hash is looked up on platform `win64`, which no valid
   manifest has, so the verdict is always unknown, `refuse` never refuses and `UNOFFICIAL_LAUNCHER` is never raised.
   Proposed fix (a behaviour change, so a decision row): look the pin up on `windows-x86_64` for channel `stable`.
   The implementer keeps parity unless the coordinator says otherwise.
2. **The recent-activity key.** Restricted touches the map with the account id text (`account:...@node`), so the
   erasure blocker and the admin's online lists, which compare userIds, never see game-session activity. The MIT
   `Activity` seam hands this crate the userId (`crates/rustic-isles-auth`'s call), and this spec keys the map by what
   it is given, which makes those checks work as Node meant. Confirm this departure (it changes who blocks an
   erasure and who shows online).
3. **Process-wide state per module** (§2): the status cache and presence were decided per module (core §17 item 14);
   this spec extends that to the readiness, metrics registry, attempt windows, recent-activity map, boot time (so
   `uptimeSeconds` counts from the module's state, not the process) and the launcher-feed log dedup. Confirm.
4. **Attempt windows on auth's bounded limiter** (§3.2) instead of an unbounded map of the crate's own: a deliberate
   improvement with the same answers below 16 384 keys.
5. **Legacy and Mystic names.** Dropped by decision: the `runtime-mysticparadox_dll_port` log name, the
   `MYSTICPARADOX_METRICS_TOKEN` alias and the header copy. Kept as parity, worth a row each: the default log folder
   `../MysticLauncherLogs` (existing hosts' data lives there), `UPDATE_PUBLIC_BASE_URL`'s default
   `https://paradox.mysticfox.dev` and the default release prefix `https://github.com/pranav158/Mystic-Paradox/...`
   (a host that sets nothing advertises and allows domains Dainty Lass does not control, like auth §14 item 8's mail
   default). Wire names kept: `ParadoxRuntime`, `MYSTIC-GUARD-2`, `paradox-backend`, `metagame`.
6. **The upload folder** is the caller's userId unchecked. Proposed: refuse (400 `AUTH_VALIDATION_FAILED`) a userId
   that is not an account folder (§7.1). A behaviour change for no existing account.
7. **The status's instances** come only from `DEPLOYSERVER_URL` over HTTP, used as a host:port authority; an
   in-process Director (core §5.6) is not consulted, and a value with a scheme fails silently. Proposed: read
   `hub_status`/instances through core's dedicated-servers slot when it is configured in-process.
8. **The runtime manifest path auth reads** (auth §6.6: `runtime/client/<channel>/windows-x86_64/latest.json`) is the
   legacy layout, while B188 writes `runtime/<changelist>/client/...`: a runtime published through B188 is never the
   approved one for game sessions unless the legacy file is also there. Not this crate's code; for the auth owner and
   the maintainer.
9. **The client address** reads only the first `x-forwarded-for` line in restricted; this crate uses auth's reader
   (every line). Equal behind the gateway.
10. **An expired current Guard manifest blocks the next publish** (§20 item 8). Proposed: skip the previous envelope's
    expiry check when comparing sequences.
11. **The sync routes** could refuse links and non-launcher names as the operator's view does.
12. **rustic-rifts' `IslesCore`** guard calls (`healthy_guard_session`, `published_guard_artifacts`) answer `NoCore`
    in restricted; this crate holds what they need and could serve them when unit 12 composes rifts.
13. **The admin player deletion** does not remove the uploaded logs (the erasure does); the admin half decides whether
    to call §14.1's `remove_account_logs`.

---

## 22. Coordinator decisions applied (2026-11-05)

- **No Mystic Paradox legacy names**: no `MYSTICPARADOX_*` aliases, no `x-mysticparadox-*` header copies (core §17
  item 2), no `mysticparadox_dll_port` log name. The admin half applies the same rule to its own legacy names.
- **Parity on odd behaviour**, listed in §20; the departures this spec proposes are only in §21, each awaiting a
  decision.
- **Secrets never in `Debug`**: the crate's state, settings readers and records print names only; keys, tokens,
  secrets, account keys, challenges and public keys never appear in a `Debug` or a log line.
- **Goldens stay in dainty-restricted** (auth §15 item 6); tests are written from §18.

---

## Coordinator decisions (2026-11-05, before implementation)

These settle §21 and override anything above that disagrees. Departures are candidate decision rows for the
maintainer to ratify.

- **File split:** as §1 now has it, matching spec-rustic-isles-reforged-admin.md §1.2 (host routes and the
  release-gates handler are the admin half's; `admin_guard.rs` is this half's).
- **Clocks:** this half's attempt windows keep core's replay-aware clock (parity); the admin half's trust decisions use
  the system clock as its decisions say. Replay pinning is gated by core §17 either way.
- **Item 2: taken.** The recent-activity map is keyed by userId, so the erasure blocker and online lists see
  game-session activity.
- **Items 3-4, 9:** as proposed (per-module state; auth's bounded limiter; auth's client-address reader).
- **Item 5:** the legacy names are dropped; the `../MysticLauncherLogs`, `paradox.mysticfox.dev` and `pranav158`
  defaults keep parity and are flagged for the maintainer.
- **Item 6: taken (security).** A userId that is not a safe single path segment (empty, `.`/`..`, a separator, a NUL,
  a drive prefix) is refused before any upload write.
- **Item 11: taken (security).** The sync routes never follow a symbolic link and serve only regular files inside the
  session folder.
- **Items 1, 7, 8, 10, 12, 13:** parity; 1, 8 and 10 are listed for the maintainer as bugs to fix in their own rows.
- **§20:** parity.
