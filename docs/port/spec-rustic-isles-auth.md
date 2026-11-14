# Clean-Room Behaviour Specification: `crates/rustic-isles-auth` (RD27)

> Since R21 (`spec-runtime-r21.md`) the AGPL runtime is gone; the runtime manifest's core file is
> `rustic-isles-runtime.dll` (`CORE_DLL`, R21 P-4), and its top-level `sha256` is that DLL's.

> **Ports moved 2026-11-10.** The loopback services' defaults are now backend 41000, Director 41001, content
> 41002, admin 41003, allowlist 41005, chat 41006 and QoS 41010, below the dynamic range Windows lends to Hyper-V,
> WSL and Docker. settings.toml's `[ports]` moves any of them, and on Windows setup and start move one Windows has lent
> out (a private gateway's 61000 to 41080). The numbers below are the ones recorded upstream: read each 610xx
> service port as its 410xx counterpart. A private gateway's default stays 61000.

> **Scope and purpose (RD27):** this is the clean-room behaviour specification for an MIT crate, `rustic-isles-auth`, that
> dainty-restricted's Rustic Isles module (`modules/rustic-isles`, crate `isles-core`) can call in place of its auth
> area (`crates/isles-core/src/areas/auth.rs` and `areas/auth/*`). That area is a Rust port of Mystic Paradox's
> `ParadoxBackend` sign-in code and is AGPL-3.0-only with Mystic Paradox's section 7 terms; an implementer who never
> sees it writes the MIT crate from this document alone.
>
> Everything below is stated from observable contracts: the HTTP routes and their bytes, the token and hash formats,
> the rows the area reads and writes, the settings, the refusals and the tests. No implementation code is reproduced
> and no private helper is named. Node function and file names appear only in §1 as provenance; Paradox wire names
> the 1.14.7 client or a shipped launcher depends on (the `paradox-backend` audience, the `mystpax` alias, the
> `x-rustic-isles-*` headers) are interface facts and are kept.
>
> Written against dainty-restricted `e0e238b4434316bb210677d88bc59e3448688794` (worktree
> `splashloader-integration-90130d`; the area's last change is `9ad97c6`) and dainty-lass
> `3503a69701363cf8217c4d2d5c1932356cf603df` (branch `feat/isles-auth-clean-room`, now carrying `rustic-isles-auth`).

---

## 1. Provenance and licensing classification

### 1.1 Evidence used

| Source | What it says |
| --- | --- |
| `modules/rustic-isles/NOTICE.md` | The module is a modified Mystic Paradox (up to `21b9ff5`); Paradox's own list of its work includes "Launcher authentication, Discord OAuth, and signed launcher/runtime update distribution". Everything in the module is AGPL-3.0-only plus `ADDITIONAL_TERMS.md` |
| Root `NOTICE.md` (dainty-restricted) | `isles-core` is the Rust port of `ParadoxBackend/` at `21b9ff5` |
| File headers (`//!`) | Every area file but three names the TypeScript it ports: `routes/login.ts`, `routes/eos.ts`, `routes/matchmaking.ts` (keys), `routes/launcherAuth.ts`, `controllers/{launcherAuth,launcherAccount,launcherPolicy,launcherNotices,launcherEmail,emailCodes,discordAuth,apikeys,auth}.ts`, `security/{launcherErrors,launcherTokens,apiKeyHash,emailCodes,passwords,accountEligibility,accountConsent,usernamePolicy,unofficialLauncherPolicy,launcherVersion,runtimeAuthorization,rateLimit,totp,testerFeatures}.ts`, `legal/{legalPolicy,retentionPolicy}.ts`, `config/*.ts`, `mail/{mailConfig,codeEmail,mailSender}.ts`, `accountDeletion/erasure.ts`, `encryptionKeyStore.ts` |
| `docs/inventory/routes-1.14.7.md` (RU3a) | 49 rows in group `auth`, side `isles`, all from `reference/1.14.7/ParadoxBackend/src/routes/{login,eos,matchmaking,launcherAuth}.ts` |
| `git log` of the area | Born in `1cd9f84` (RU5 auth, 2026-10-27) and `0c6747b` (`totp.rs`, `wiring.rs`); Dainty Lass additions in `66601f2` (session tickets, account-key registration with the home, DN5), `cad9d44` (`x-reforged-user-api-key`, RF5), `9e5afc1` (request budget, key cap), `e33dff5` (timeouts), `b909a45` (chat login), `9ad97c6` (Paradox to Rustic Isles rename, legacy names) |

The 3+-line similarity method of `clean-room/similarity.md` §1 does not apply to the source (TypeScript ported to
Rust); it applies to the rewrite, measured against the restricted files of §1.2 at `e0e238b` (§11.4).

### 1.2 What is rewritten (Category B: everything)

Every file of the area is a port of Paradox code or written inside a port, so nothing moves as it is:

| Restricted file | Rewritten as (this spec) |
| --- | --- |
| `areas/auth.rs` | §10.1 start-up, §3 route list |
| `auth/errors.rs` | §2.3 error envelope and status table |
| `auth/util.rs` | §2.1 conventions, §4.6 client address |
| `auth/middleware.rs` | §4.1-§4.4 |
| `auth/game.rs` | §3.1-§3.3 |
| `auth/launcher.rs`, `launcher_routes.rs`, `account.rs` | §3.4, §4.5 CORS |
| `auth/tokens.rs`, `crypto.rs`, `totp.rs`, `apikeys.rs` | §5 |
| `auth/policy.rs`, `runtime_authorization.rs` | §6 |
| `auth/email.rs`, `mail.rs`, `discord.rs`, `discord_flow.rs` | §7 |
| `auth/runtime.rs` | §4.7 limiter, §5.6 encryption keys, §11 seams |
| `auth/erasure.rs`, `wiring.rs` (jobs part) | §8 |
| `auth/data.rs` | folded into §9's store trait |
| `auth/test_support.rs` | not rewritten from; the MIT crate writes its own test support (§12.3) |

### 1.3 What stays in dainty-restricted

These are isles glue, not part of the MIT crate; they implement §11's traits or call the crate:

- `repo/auth.rs` and migrations `0001_accounts`, `0020_auth`, `0610_timeouts` (sqlite and postgres): they become the
  isles implementation of `AuthStore` (§9). The tables are isles-core's and other areas read them (§9.4).
- `wiring.rs`'s `AuthReforgedCore` (`rifts_seam::IslesCore` for matchmaking) and the registration of the launcher
  authenticator and admin sign-in with the Reforged API: they call the crate's public functions (§11.2).
- The default seam implementations that reach other areas (activity monitor, operations policy, inventory storage
  policy, starter bootstrap, erasure of other areas' rows, uploaded-log removal, notices view, matchmaking presence,
  rifts calls).
- `http.rs`'s pipeline: route matching, the request id, `/__origin/` strip, the `/admin` gate, the body parser, the
  legacy header copy (`x-mysticparadox-*` to `x-rustic-isles-*`). It calls the crate for CORS and the request budget
  (§4.5, §4.7).
- The Settings type and its legacy env aliases (`MYSTICPARADOX_*` read as `RUSTIC_ISLES_*`): the crate sees only
  §11's settings trait.
- The 90-day log purge job and the TTL purge job (Reforged API tables only).

---

## 2. Conventions

### 2.1 Text, times, ids

- **Times** are Unix milliseconds from the crate's clock (§11), which isles points at its replay-aware clock. Stored
  and answered times are `Date#toISOString` text: UTC, `YYYY-MM-DDTHH:MM:SS.mmmZ` (0 is `1970-01-01T00:00:00.000Z`;
  `1790000000123` is `2026-09-21T14:13:20.123Z`); an out-of-range value writes the epoch. Stored ISO texts are
  compared as strings, which orders them as times.
- **Parsing a stored time** is JavaScript's `Date.parse` (`service_kit::js::date_parse`), `None` for NaN.
- **JS trim** is `service_kit::js::trim` (JavaScript's `String#trim` white space set). "Lower-cased" is Unicode
  lower-casing (JavaScript `toLowerCase`) unless it says ASCII.
- **Lengths** "UTF-16 units" are what JavaScript's `.length` reports; "bytes" are UTF-8.
- **Hex** is lower case. "N random bytes hex" is 2N hex digits from a CSPRNG; "N random bytes base64url" is unpadded
  URL-safe base64 of N random bytes (32 bytes give 43 characters).
- **UUIDs** are v4, lower-case hyphenated (`crypto.randomUUID()`), from an id source isles can make deterministic
  for replays (§11).
- **userId** is the client-visible account id (a UUID for every launcher account; arbitrary text for dev and service
  identities). **Account id** is the account's network-wide id, `account:<ULID>@<node>` (`module_api::GlobalId`,
  DD18), minted on the module's node at registration.

### 2.2 HTTP answers

- **JSON** is written as `JSON.stringify` writes it: `service_kit::json::Json`, `content-type: application/json;
  charset=utf-8`, keys in insertion order (`serde_json` with `preserve_order`; every key order below is the wire
  order). Non-ASCII is written as itself (`–` U+2013 appears in a message).
- **Empty** means the status with no body and no content type (Express's `res.status(n).send()`).
- **Error page** means `service_kit::body::error_page(status)`: Express's HTML page (`text/html; charset=utf-8`,
  `<pre>Internal Server Error</pre>` for 500). Where this spec says "500 page" Node threw inside a handler.
- **Body** is `req.body` as Express 5's `express.json` (100 KiB) and `express.urlencoded({extended: true})` leave it:
  absent ("undefined") without a JSON or form content type or without a body; a JSON body must start (after
  whitespace and a BOM) with `{` or `[`, else 400 page; an empty JSON body is `{}`; a non-UTF-8 charset 415 page; a
  content encoding other than identity 415 page; over the limit 413 page. Use the pipeline's parsed body
  (`service_kit::body::ExpressBody`) when present, otherwise parse with `service_kit::body`'s parsers under the same
  rules. "A field" of the body is `body?.name` (a field of an array or scalar body is absent).
- **Query** values are decoded as `qs` does (`+` is space, invalid percent-encoding left as it is); a name that
  repeats is an array, not a string.
- **Path params** arrive percent-decoded from isles' router (Express 5 matching: case-insensitive, one optional
  trailing slash).

### 2.3 The launcher error envelope

Every launcher route refusal (`/launcher/v1/...`) is:

`{"error":{<details...>,"code":<code>,"message":<message>,"requestId":<uuid>}}`

with the details first (in the order added) and the three fixed fields last; a detail named `code`, `message` or
`requestId` is overwritten in place by the fixed field (it keeps its position). `requestId` is a fresh UUID per
error, not the request's `x-rustic-isles-request-id`. The status is the error's own when it carries one, else the
table; a code not in the table answers 500. Any store failure, and anything unexpected, is logged and answered as
`INTERNAL` / `Something went wrong. Please try again.` (500).

| Code | Status | Code | Status |
| --- | ---: | --- | ---: |
| `AUTH_VALIDATION_FAILED` | 400 | `CONSENT_REQUIRED` | 400 |
| `AUTH_INVALID_CREDENTIALS` | 401 | `CONSENT_AGE_REQUIRED` | 400 |
| `AUTH_EMAIL_TAKEN` | 409 | `LEGAL_VERSION_OUTDATED` | 409 |
| `AUTH_DISPLAY_NAME_TAKEN` | 409 | `DELETION_CONFIRMATION_MISMATCH` | 400 |
| `AUTH_ACCOUNT_DISABLED` | 403 | `DELETION_HOLD_EXPIRED` | 409 |
| `AUTH_ACCOUNT_BANNED` | 403 | `ACCOUNT_DEACTIVATED` | 403 |
| `AUTH_APPROVAL_PENDING` | 403 | `GAME_EXCHANGE_CODE_EXPIRED` | 410 |
| `AUTH_APPROVAL_REJECTED` | 403 | `GAME_BUILD_UNSUPPORTED` | 400 |
| `AUTH_USERNAME_REQUIRED` | 403 | `RUNTIME_BUILD_UNSUPPORTED` | 409 |
| `AUTH_REFRESH_INVALID` | 401 | `GUARD_MANIFEST_UNAVAILABLE` | 503 |
| `AUTH_UNAUTHORIZED` | 401 | `GUARD_MANIFEST_MISMATCH` | 409 |
| `AUTH_RATE_LIMITED` | 429 | `GUARD_REPORT_INVALID` | 400 |
| `AUTH_DISCORD_NOT_CONFIGURED` | 503 | `GUARD_SESSION_INVALID` | 401 |
| `AUTH_DISCORD_CANCELLED` | 400 | `GUARD_SIGNATURE_INVALID` | 401 |
| `AUTH_DISCORD_ALREADY_LINKED` | 409 | `GUARD_SEQUENCE_CONFLICT` | 409 |
| `AUTH_EMAIL_UNVERIFIED` | 403 | `OFFICIAL_PLAY_UNAVAILABLE` | 503 |
| `AUTH_CODE_INVALID` | 400 | `LAUNCHER_UNOFFICIAL` | 403 |
| `AUTH_EMAIL_UNAVAILABLE` | 503 | `NOTICES_INVALID` | 400 |
| `AUTH_DISCORD_NOT_LINKED` | 403 | `NOTICES_VERSION_CONFLICT` | 409 |
| `AUTH_DISCORD_ACCOUNT_HAS_LINK` | 409 | `NOT_FOUND` | 404 |
| `AUTH_DISCORD_ONLY_LOGIN` | 409 | `INTERNAL` | 500 |
| `AUTH_TESTER_REQUIRED` | 403 | `NOT_IMPLEMENTED` | 501 |
| `AUTH_CHANNEL_MISMATCH` | 409 | | |

The same table carries the codes rustic-rifts refuses with, so a realm refusal relayed through §3.4.10 gets its
status: `P2P_DISABLED` 503, `P2P_EMERGENCY_STOP` 503, `P2P_SESSION_NOT_FOUND` 404, `P2P_SESSION_NOT_READY` 409,
`P2P_NOT_ROSTERED` 403, `P2P_STEAM_LINK_REQUIRED` 409, `P2P_TICKET_INVALID` 400, `P2P_TICKET_EXPIRED` 410,
`P2P_TICKET_REPLAYED` 409, `P2P_CAPABILITY_INVALID` 401, `P2P_GENERATION_FENCED` 409, `P2P_CONFLICT` 409,
`P2P_TRANSPORT_MISMATCH` 409, `P2P_ICE_UNAVAILABLE` 503, `P2P_RELAY_RATE_LIMITED` 429, `P2P_SIGNAL_INVALID` 400,
`P2P_SIGNAL_LIMIT` 429, `REALM_HUB_EXISTS` 409, `REALM_HOST_INELIGIBLE` 409, `REALM_TRANSPORT_UNSUPPORTED` 409,
`REALM_HOST_OFFLINE` 409, `REALM_HUB_FULL` 409, `REALM_HUNT_RUNTIME_INCOMPATIBLE` 409, `REALM_UNAVAILABLE` 503,
`P2P_HOST_PORTS_EXHAUSTED` 409, `GUARD_REQUIRED` 403, `GUARD_BATCH_INVALID` 400, `GUARD_BATCH_TOO_LARGE` 413,
`GUARD_BATCH_REPLAY` 409, `GUARD_EVIDENCE_LIMIT` 409, `GUARD_TRAFFIC_RATE_LIMITED` 429,
`GUARD_REQUEST_BUDGET_EXHAUSTED` 503, `GUARD_TRAFFIC_UNAVAILABLE` 503, `GUARD_FINALIZATION_BUSY` 503,
`GUARD_CHAIN_CONFLICT` 409, `GUARD_CHAIN_FROZEN` 409, `GUARD_COMPLETION_INVALID` 409, `GUARD_COMPLETION_CONFLICT`
409, `GUARD_COMPLETION_PENDING` 409, `GUARD_SHADOW_INVALID` 403, `REWARD_CAPACITY_RECONCILIATION_REQUIRED` 503,
`HUNT_EVIDENCE_NOT_COLLECTED` 404, `HUNT_REWARDS_UNAVAILABLE` 404, `REALM_VALIDATION_FAILED` 400,
`REALM_NAME_INVALID` 400, `REALM_NAME_RESERVED` 400, `REALM_NAME_TAKEN` 409, `REALM_ALREADY_EXISTS` 409,
`REALM_USERNAME_REQUIRED` 409, `REALM_NAME_IS_USERNAME` 409, `REALM_NOT_FOUND` 404, `REALM_NOT_MEMBER` 404,
`REALM_BANNED` 403, `REALM_INVITE_REQUIRED` 403, `REALM_INVITE_INVALID` 404, `REALM_INVITE_EXPIRED` 410,
`REALM_INVITE_LIMIT` 409, `REALM_FULL` 409, `REALM_OWNER_CHANGED` 409, `REALM_INVALID_TARGET` 400,
`REALM_HOST_CANNOT_LEAVE` 409, `REALM_JOIN_REQUESTED` 409, `REALM_REQUESTS_FULL` 409, `REALM_REQUEST_NOT_FOUND` 404,
`REALM_PLAYER_NOT_FOUND` 404, `REALM_PLAYER_INVITES_FULL` 409, `REALM_PLAYER_INVITE_NOT_FOUND` 404, `REALM_OFFLINE`
409, `REALM_SOLO` 409, `REALMS_DISABLED` 501.

`AUTH_ACCOUNT_TIMED_OUT` (§6.1) is not in the table and answers 500 (§14 item 1).

A launcher route's success is its JSON with the route's success status (202 for register, 200 for the rest); a
controller that returns nothing answers `{}`.

---

## 3. Routes

All 49 are the backend service's (port 61000), mounted in isles' route table at these paths (its Express match order
is isles'). "Game auth" is §4.1; "launcher auth" is §4.2. A route with game auth answers the refusal §4.1 returns
before anything else, then reads its body.

### 3.1 Game client: sign-in stubs and account reads (`routes/login.ts`)

| Method, path | Auth | Answer |
| --- | --- | --- |
| `GET /features/platform/win` | none | 200 `{"code":null,"message":"OK","payload":{"crossplay":true,"crossprogression":true}}` |
| `GET /account/link/epic/:AccId` | none | 200 `{"code":null,"message":"OK","payload":{"isLinked":true}}` |
| `GET /isbanned` | none | 200 `{"isBanned":false}` |
| `POST /migration/trigger` | game | 200 `{"migration_failed":false,"migration_finished":true}` |
| `GET /migration/status` | game | 200 `{"code":null,"message":"OK","payload":{"migration_failed":false,"migration_finished":true}}` |
| `GET /tags` | game | 200 `{"accountId":<userId>,"tags":[]}`; `accountId` omitted when the caller has no userId |

**`POST /login`.** Body absent: 500 page. The body's `email` must be a string equal to the caller's userId, and the
caller must have one, else empty 400. No game account (`accounts`) for that userId: empty 400; store failure: 500
page. Then the session-supplies hook (§11, isles' `REFORGED_SESSION_SUPPLIES_FILE` refill) runs for the account and
the answer is 200 `{"error_code":"TicketRateOk","message":"","state":"OPEN","timeout":8000,"title":""}`.

**Username of a userId** (used below): given the JSON value the client sent. Absent: failure (500 page). Not a
string: the value itself. A string: the launcher account's display name if it exists and is non-empty (a launcher
lookup failure is ignored), else the game account's `name`, else the text itself; a game-account lookup failure is a
failure (500 page).

**`GET /accountinfo`** (game): the username of the caller's userId (absent userId: 500 page), then 200
`{"accountId":<userId>,"creationDate":"2000-01-01 00:00:00","email":null,"preferredLanguage":null,"username":<name>,"verified":true}`
(`accountId` omitted without a userId, which cannot reach here).

**`PUT /gamesession/epic`, `PUT /gamesession/epiceos`** (game): the token is the `authorization` header with its
first 7 bytes dropped (whatever they spell), else the trimmed `x-reforged-session-ticket` header, else 500 page. 200
`{"code":null,"message":"OK","payload":<payload>}` where the payload is the version's (§11 config); 1.14.7's is
`{"error_code":null,"sessionid":<token>,"sessiontoken":<token>,"sessionToken":<token>}`.

**`POST /accountinfo/public`** (game): body absent: 500 page. `requested` is the body's `accountId` (any JSON value).
If `requested` is a string equal to `DEV_USER_ID` (default `mystpax`) and the caller has a userId different from it,
the name is looked up for the caller's userId (the 1.14.7 client asks once more under the retained `mystpax`
self-alias after launcher sign-in); otherwise for `requested`. Username failure (absent `accountId`): 500 page.
Answer 200:
`{"accountId":<requested>,"isSubscribed":true,"language":null,"linkedAccounts":[{"accountId":<requested>,"accountType":"epic"}],"username":<name>}`,
both `accountId`s omitted when `requested` is absent (unreachable: it fails first). Anybody may look up anybody.

### 3.2 Game client: the Epic Online Services stand-in (`routes/eos.ts`)

Constants: `expires_at` `2085-09-09T01:01:01.703Z`; the service identity userId `__launcher_client_credentials__`.

**Token answer** (game token for a userId, §5.1): 200, in order: `access_token` (the token), `token_type` `bearer`,
`expires_at`, `features` `["Achievements","AntiCheat","Ecom","Voice"]`, `organization_id`
`o-krlzxj88qrtb69fredeuaf887bl5az`, `product_id` `prod-jackal`, `sandbox_id` `jackal`, `deployment_id`
`53565ba467df4edbb6f5a3d939a8b4f2`, `expires_in` `86400`; a **login** answer adds `refresh_token`
`refresh.token.lol`, `refresh_expires_at` (the same date) and `account_id` (the userId). A signing failure: 500 page.

**Dev mode** is on only when `ALLOW_NO_AUTH_DEV_MODE` is exactly `true` and `NODE_ENV` is not exactly `production`.
**Ensuring a dev user** creates the game account (new account id on this node, the userId, `name` = `DEV_USER_NAME`
default `Dev Slayer`, `notes` 0) unless one exists for the userId, as the system actor; failure: 500 page.

**EOS bearer:** the `authorization` header split on single spaces must give exactly two parts, the first `bearer` in
any ASCII case; the second is the token. **User from the bearer:** the token's `userId` claim when it is a valid game
token and the claim is a non-empty string, else none (an invalid token is logged as a warning).

**`POST /account/api/oauth/token`** (no auth). A body parse error answers its page. Then by `AUTH_MODE` (raw, exact):

| Mode | Behaviour |
| --- | --- |
| `NONE` with dev mode | Body absent: 500 page. userId = `exchange_code` if a non-empty string other than `INVALID`, else the user from the bearer, else `DEV_USER_ID` (default `dev-user`). Ensure the dev user; login answer |
| `APIKEY` | Body absent: 500 page. `key` = `exchange_code` when a string. Key absent or empty and `grant_type` is `client_credentials`: token answer for the service identity. A key of 1 to 256 UTF-16 units is looked up (§5.3, store failure 500 page). If it names a userId whose launcher account exists and is not eligible (§6.1): empty 403. If it names a userId: the SHA-256 hex of the raw key is registered with the home (`register_account_key_hash`, the account's global id) and the account in the directory (`register_directory_account`, its game account name), both only when the game account exists and both failures ignored; login answer. Otherwise empty 400 |
| `LAUNCHER` | Body absent: 500 page. `exchange_code` not a non-empty string: `grant_type` `client_credentials` gives the service token answer, else empty 400. Otherwise the exchange below |
| anything else (or `NONE` without dev mode) | empty 500, logged |

The **launcher exchange** of a code: consume the exchange code whose hash is the SHA-256 hex of the code (§5.2), as
the system actor; store failure 500 page. None (missing, expired or used): in dev mode ensure a dev user named by the
raw code and give its login answer; otherwise empty 401. Found: read the launcher account by account id (failure 500
page), then in this order:

1. the code's `build_changelist` differs from the module's target: empty 409;
2. no launcher account, or not eligible (§6.1): empty 403;
3. the code's runtime channel does not match the account (§6.6) or its `runtime_sha256` is not the published
   manifest's current main hash (§6.6): empty 403 (a tester revocation or a republished runtime inside the code's 60 s
   invalidates it);
4. the code's `realm_id` does not parse as a realm id: empty 503;
5. the realms seam binds the account's realm presence to the code's realm or to none (§11); a failure: empty 503;
6. login answer for the account's userId.

**`GET /account/api/oauth/verify`** (no auth): EOS bearer absent or not a valid game token (a launcher token is not):
empty 401. Else 200 `{"active":true,"scope":"basic_profile friends_list presence","token_type":"bearer","expires_in":86400,"expires_at":"2085-09-09T01:01:01.703Z","account_id":<userId claim as it is>,"client_id":"xyza7891lhxMVYGCON7LgnKZZ8HQGD5H","application_id":"fghi4567O03HROxEjwbn7kgXpBhnhWwv"}`,
`account_id` omitted when the token has no `userId` claim.

**Public account** of an id: look up the game account and the launcher account by userId (failures read as none).
Neither: none. Else `{"id":<id>,"displayName":<name>,"externalAuths":{}}` where the name is, first present: an
override, the launcher display name if non-empty, the game account name, the id.

| Method, path | Auth | Answer |
| --- | --- | --- |
| `GET /account/api/public/account/:AccId` | none (bearer read) | The public account of `AccId`, 200, or empty 404. Override: when `AccId` equals `DEV_USER_ID` (default `dev-user`) and the user from the bearer exists and differs, the caller's launcher display name (if the caller has a launcher account) |
| `GET /account/api/public/account/:AccId/externalAuths` | none | Game account (not launcher) by userId: 200 `[]`; none: empty 404; failure: 500 page |
| `GET /account/api/public/account/displayName/:displayName` | none | The param JS-trimmed and lower-cased, launcher account by normalized display name: 200 `{"id":<userId>,"displayName":<display name>,"externalAuths":{}}`; none: empty 404; failure: 500 page |
| `DELETE /account/api/oauth/sessions/kill`, `DELETE /account/api/oauth/sessions/kill/:AuthToken` | none | 200 `{}` |
| `GET /account/api/public/account` | game | The `accountId` query values in order (decoded, empties dropped, duplicates kept); none: the caller's userId (or `""`). Each id's public account, override as above with `DEV_USER_ID` default `dev-user` and the caller's launcher display name; missing ones skipped. 200 array |

### 3.3 Game client: network encryption keys (`routes/matchmaking.ts`)

**`POST /key/generate`** (game). `key` = 32 random bytes hex; `nonce` = 32 random bytes base64url; `token` = the
`authorization` header with a leading lower-case `bearer ` removed (exactly that spelling; `Bearer x` stays
`Bearer x`), `""` without the header. Then the body (parse error answers its page). `candidateId` = the body's
`candidateId` converted with JavaScript `String(...)` when it is present and not `null`, `false`, `0` or `""`, else
`""`. The key is issued (§5.6) with the caller's userId when non-empty. 200 `{"key":..,"nonce":..,"token":..}`.

**`POST /key/consume`** (game; the hub's fetch, normally with a game server key). Body (parse error answers its page;
absent reads as `{}`). `candidateId` and `token` each: the body field converted with `String(...)` when present and
not `null`, else the query value (a repeated query name reads as `""`), else `""`. Lookup (§5.6) with the caller's
runtime roster when the caller is a player host's runtime. A roster caller that gets no key, or any caller with no
key: 404 `{"error":"no issued key"}`. Else 200 `{"key":..,"nonce":..,"token":..}` of the issued key. Logs name the
token only by its fingerprint (§5.6).

### 3.4 Launcher API (`routes/launcherAuth.ts`)

Every path below is under the CORS gate (§4.5). "IP" is the client address of §4.6 (not the `/56` key). "Device
fields" are the body's `deviceId` (a non-empty string, else a fresh UUID) and `deviceName` (a non-empty string, else
`Unknown device`). Messages are exact.

**Account view** (`account` in answers; field order is the contract):
`{"userId","displayName","email","discordLinked","discord","emailVerified","status","approvalStatus","needsUsername"}`:
`email` is the stored address or `""`; `discord` is `{"linked":false}` or `{"linked":true,"username":<Discord
username>}` from a fresh identity read; `emailVerified` is whether `email_verified_at` is set; `status` is
`active|banned|disabled`; `approvalStatus` is the effective one (§6.1); `needsUsername` is true only when
`username_set` is stored as false.

**Session answer:** `{"accessToken":<launcher token>,"refreshToken":<64 hex>,"account":<view>}` (§5.1, §5.2).
**Signing in** an account (password, verify, reset): write `last_login_at` = now as that player, create a session
(§5.2), answer the session answer with the view showing the new `last_login_at`.

#### 3.4.1 `POST /launcher/v1/auth/register` (202)

In order:

1. Limit `register:{IP}` 5 per hour: `AUTH_RATE_LIMITED` / `Too many registration attempts. Try again later.`
2. `displayName`: absent, `null` or a string that is blank after JS trim means "no name"; anything else is validated
   (§6.3: format, then allowed).
3. `email` must be a valid address (§7.1) else `AUTH_VALIDATION_FAILED` / `Enter a valid email address.`; `password`
   a string of at least 8 UTF-16 units else `AUTH_VALIDATION_FAILED` / `Password must be at least 8 characters.`
4. Consent (§6.2, registration rules).
5. Normalized email = JS trim, lower-cased. With verification required (§6.4) the code mail must be configured
   (§7.2, `AUTH_EMAIL_UNAVAILABLE`).
6. An account with that email: 409 `AUTH_EMAIL_TAKEN` / `That email is already registered.` A given name taken in any
   case: 409 `AUTH_DISPLAY_NAME_TAKEN` / `That display name is already taken.`
7. Create (as the system actor): userId a fresh UUID; account id new on this node; email normalized; display name the
   given name, or the userId; normalized name the given name lower-cased, or the userId lower-cased; Argon2id hash
   (§5.4); status `active`; approval status from §6.4; roles `["player"]`; `created_at` now; `username_set` whether a
   name was given; the consent record if any; `email_verification_pending_since` = `created_at` when verification is
   required. The store also writes the game account (name = display name, notes 0) unless it exists. The store's
   uniqueness answers (email, name) give the same two 409s.
8. Answer `{"approvalRequired":<status is pending>,"account":<view>}`; with verification required
   `{"approvalRequired":..,"verificationRequired":true,"email":<normalized>,"account":<view>}` after issuing the
   first verify code (§7.3; a store failure there is only logged). No token is issued at registration.

#### 3.4.2 `POST /launcher/v1/auth/login`

1. Limit `login:{IP}` 10 per 5 minutes; then, if `email` is present and neither `null` nor a string, `INTERNAL`
   (500); then limit `login:{normalized email or ""}` 10 per 5 minutes. Either limit:
   `AUTH_RATE_LIMITED` / `Too many login attempts. Try again later.`
2. `email` and `password` not both strings: 401 `AUTH_INVALID_CREDENTIALS` / `The email or password is incorrect.`
3. Account by normalized email; the password verified (§5.4; a dummy hash is verified when there is no account or
   no hash, so the time is the same). No account or wrong password: the same 401.
4. Banned or disabled: 403 `AUTH_ACCOUNT_BANNED` / `This account has been banned.`, `AUTH_ACCOUNT_DISABLED` /
   `This account has been disabled.`
5. Verification required and the email not verified: the code mail must be configured (503), a verify code is issued
   to the stored address (§7.3; limited or failed mail still refuses), and 403 `AUTH_EMAIL_UNVERIFIED` /
   `Enter the 6-digit code we sent to {masked}. Older launchers: update the launcher to verify your email.`
6. Approval pending: 403 `AUTH_APPROVAL_PENDING` / `Your closed-test access request is waiting for approval.`;
   rejected: 403 `AUTH_APPROVAL_REJECTED` / `Your closed-test access request was not approved.`
7. Sign in.

A form body (`email=...&password=...`) works like JSON.

#### 3.4.3 `POST /launcher/v1/auth/refresh`

`refreshToken` not a non-empty string, or no session with its hash: 401 `AUTH_REFRESH_INVALID` /
`Session expired. Please sign in again.` The session found:

- already revoked: revoke the whole family (reuse of a rotated token) and refuse;
- `expires_at` earlier than now: refuse (no revoke);
- no launcher account: revoke the family and refuse;
- not admitted (§6.1): revoke the family and answer that refusal (403s);
- else note the sign-in (§3.4.11), then in one write as that player revoke this session (at now) and create the next
  in the same family: new id, new token, device id and name from the device fields (the old ones only if those are
  empty, which the route never makes them), 30-day expiry. Answer the session answer for the family.

#### 3.4.4 `POST /launcher/v1/auth/logout`, `POST /launcher/v1/auth/logout-all` (launcher auth)

Logout revokes every unrevoked session of the caller's family (`sid`; for an account-key caller `apikey`, which
matches nothing); logout-all revokes every unrevoked session of the account. 200 `{}`.

#### 3.4.5 Email codes

| Route | Behaviour, in order |
| --- | --- |
| `POST /launcher/v1/auth/email/verify` | Mail configured (503). Limit `email-verify-ip:{IP}` 30 per 15 min. `email`, `password` not both strings: 401 `AUTH_INVALID_CREDENTIALS`. Limit `email-verify:{normalized}` 10 per 15 min. Account by email and password verified (dummy on miss): 401 on failure. Banned/disabled 403. The `code` accepted for `verify` (§7.3) else 400 `AUTH_CODE_INVALID` / `That code is wrong or has expired. Check the latest email or resend the code.` Set `email_verified_at` = now (clears the pending mark). Then: approval pending answers 200 `{"approvalRequired":true,"account":<view>}`; rejected 403; else sign in |
| `POST /launcher/v1/auth/email/resend` | Mail configured (503). Limit `email-resend-ip:{IP}` 20 per hour. Valid address else 400 `Enter a valid email address.` Limit `email-resend:{normalized}` 5 per hour. Answer 200 `{"sent":true}` at once; afterwards, in the background, mail a verify code if the address has an `active` account whose email is unverified |
| `POST /launcher/v1/auth/password/forgot` | As resend with keys `password-forgot-ip:{IP}` (20/h) and `password-forgot:{normalized}` (5/h); in the background a `reset` code to an `active` account |
| `POST /launcher/v1/auth/password/reset` | Mail configured (503). Limit `password-reset-ip:{IP}` 30 per 15 min. Valid address (400). Limit `password-reset:{normalized}` 10 per 15 min. `newPassword` a string of at least 8 UTF-16 units else 400 `Password must be at least 8 characters.` No account: 400 `AUTH_CODE_INVALID`. Code not accepted for `reset`: 400 `AUTH_CODE_INVALID`. Banned/disabled 403 (checked after the code, so only the owner of the address learns it). In one write as that player: new Argon2id hash, `email_verified_at` = now, every session of the account revoked. Then as verify: pending answer, rejected 403, or sign in |

All limits here answer `AUTH_RATE_LIMITED` / `Too many attempts. Try again later.` Background delivery failures are
logged by error code only. Resend and forgot never say whether the address has an account.

#### 3.4.6 Discord (§7.5 has the flow)

| Route | Answer |
| --- | --- |
| `POST /launcher/v1/auth/discord/start` | 200 `{"authorizeUrl":..}` for a sign-in transaction; 503 `AUTH_DISCORD_NOT_CONFIGURED` / `Discord sign-in isn't set up on this server yet.` |
| `GET /launcher/v1/auth/discord/callback?code&state` | Always a 302 redirect (§7.5) |
| `POST /launcher/v1/auth/discord/complete` | Device fields; 200 session answer, or 400 `AUTH_DISCORD_CANCELLED` |
| `POST /launcher/v1/account/discord/link/start` (launcher auth) | Configured (503) first, account (404 `NOT_FOUND` / `Account not found.`), already linked 409 `AUTH_DISCORD_ACCOUNT_HAS_LINK` / `This account already has a Discord account linked.`; 200 `{"authorizeUrl":..}` for a link transaction |
| `POST /launcher/v1/account/discord/unlink/start` (launcher auth) | §7.5.4 |
| `POST /launcher/v1/account/discord/unlink` (launcher auth, then body) | §7.5.4 |

#### 3.4.7 Account and usernames

| Route | Behaviour |
| --- | --- |
| `GET /launcher/v1/me` (launcher auth) | 200 view, or 404 `NOT_FOUND` / `Account not found.` |
| `GET /launcher/v1/username?name=` (none) | The `name` values joined with `,` (absent `""`, as `String(req.query.name ?? "")`). Format or policy refusal: 200 `{"available":false,"reason":<its message>}`; taken by any account in any case: 200 `{"available":false,"reason":"That username is already taken."}`; else 200 `{"available":true}` |
| `POST /launcher/v1/username` (launcher auth, then body) | `username` validated for format (a non-string validates as `""`). Account (404). The policy check (§6.3) applies unless the account's username is already set and the new name equals its normalized name (keeping one's own name is allowed). Taken by another account: 409 `AUTH_DISPLAY_NAME_TAKEN` / `That username is already taken.` Write as that player: display name, normalized name, `username_set` true, and the game account's `name`; the store's race answer gives the same 409. 200 view |

#### 3.4.8 Consent and deletion (launcher auth)

- `GET /launcher/v1/account/consent`: account (404), then 200
  `{"required":{"termsVersion":"2026-10-14","privacyVersion":"2026-10-17","minimumAge":16},"accepted":<null or {"termsVersion","privacyVersion","ageConfirmed":true,"acceptedAt"}>,"upToDate":<§6.2 current>}`.
- `POST /launcher/v1/account/consent` (body; a non-object reads as `{}`): `ageConfirmed` not exactly `true`: 400
  `CONSENT_AGE_REQUIRED` / `You must be at least 16 years old to play Rustic Isles.`; `termsVersion` or
  `privacyVersion` not strings: 400 `AUTH_VALIDATION_FAILED` / `termsVersion and privacyVersion are required.`; terms
  not the current one or privacy not accepted (§6.2): 409 `LEGAL_VERSION_OUTDATED` / `These Terms of Use or this
  Privacy Policy have changed. Update the launcher, read them again and accept.` with details `termsVersion`,
  `privacyVersion` (the current ones). Account (404). Store terms = current, privacy = as sent, accepted at now; 200
  the consent state.
- `GET /launcher/v1/account/deletion`: account (404), 200 `{"request":<null or {"requestedAt","scheduledFor"}>,"holdDays":30}`.
- `POST /launcher/v1/account/deletion` (body): account (404) first; `confirmUsername` JS-trimmed, non-empty and equal
  to the display name ignoring case, else 400 `DELETION_CONFIRMATION_MISMATCH` / `Type your username exactly as it is
  shown to confirm the deletion.`; an existing request is answered unchanged; else requested at now, scheduled for
  now + 30 days; 200 the state.
- `DELETE /launcher/v1/account/deletion`: account (404); no request: 200 `{"request":null,"holdDays":30}`; scheduled
  time at or before now: 409 `DELETION_HOLD_EXPIRED` / `The 30-day hold has ended, so this account can no longer be
  restored.`; else cleared, 200 with `request` null.

#### 3.4.9 Reads

- `GET /launcher/v1/policy` (launcher auth): account (404). Operations policy from its seam (default
  `ENFORCE`/`PRODUCTION`/version 0). "Development" when the profile is `DEVELOPMENT` and the roles include `admin`.
  200: `policyVersion` = `{roles_updated_at or created_at}:ops-{version}`, `roles`, `channel` `stable`,
  `managedFeatureIds` (`["diagnostics.verbose"]` in development, else `[]`), `logUpload` `{"auto":<development>}`,
  `guardEnforcement`, `diagnosticsProfile` (`DEVELOPMENT` or `PRODUCTION`), then each `(name, JSON text)` field the
  realms seam adds (`p2pEmergencyStop`, `coopHunts`), parsed; a seam refusal is its launcher error, an unparseable
  field `INTERNAL`.
- `GET /launcher/v1/status` (none): 200 `{"online":<the store answers a trivial query>,"supportedBuildChangelist":<target>,"officialPlay":{"available":..,"message"?}}`
  (§6.5; `message` only when unavailable).
- `GET /launcher/v1/legal` (none): 200 `{"termsVersion":"2026-10-14","privacyVersion":"2026-10-17","minimumAge":16}`.
- `GET /launcher/v1/notices` (none): 200 with the notices seam's JSON (the Reforged API's public view, e.g.
  `{"announcements":[],"tags":[],"serverTime":<iso>}`) and `cache-control: public, max-age=30`. A store read failure
  answers the empty view; one error line per streak of failures.

#### 3.4.10 `POST /launcher/v1/game-sessions` (launcher auth, then body)

The route reads the fields before checking any:

1. `runtimeChannel` not `stable`, `beta` or `dev`: 400 `AUTH_VALIDATION_FAILED` / `runtimeChannel must be stable,
   beta, or dev.`
2. `buildChangelist` as JavaScript `Number(...)` not finite: 400 `buildChangelist is required.`
3. `executableSha256` not a non-empty string: 400 `executableSha256 is required.`

Then:

4. Limit `game-session:{userId}` 10 per 5 minutes: `AUTH_RATE_LIMITED` / `Too many launch attempts. Try again in a
   few minutes.`
5. Launcher account by userId, none: 401 `AUTH_UNAUTHORIZED` / `Sign in required.`
6. Eligible for play (§6.1: admitted, not timed out, username set), not deactivated (§6.2, 403 `ACCOUNT_DEACTIVATED`
   with `scheduledFor`), consent when enforced (§6.2, 403 `CONSENT_REQUIRED`).
7. `realm` absent or `null` and official play off (§6.5): 503 `OFFICIAL_PLAY_UNAVAILABLE` / `The official server is
   currently unavailable. Join or host a realm.`
8. Channel matches the account (§6.6, 409 `AUTH_CHANNEL_MISMATCH`).
9. The reported runtime set (`runtimeManifestVersion`, `runtimeArtifacts`) approved (§6.6).
10. `buildChangelist` not the target: 400 `GAME_BUILD_UNSUPPORTED` / `This Dauntless build isn't supported. Verify or
    repair your installation.`
11. `executableSha256` JS-trimmed and lower-cased must be 64 hex and in `APPROVED_EXECUTABLE_SHA256` (comma list, each
    JS-trimmed, lower-cased, kept only when 64 hex; empty approves nothing): else 400 `GAME_BUILD_UNSUPPORTED` / `This
    Dauntless installation isn't recognized. Verify or repair your installation.` (logged with userId and target).
12. Launcher build (§6.7): the `launcher` report is parsed, classified by the activity seam; an unofficial verdict that
    the policy refuses: 403 `LAUNCHER_UNOFFICIAL` / `This launcher isn't an official Rustic Isles build. Install the
    official launcher to play on Rustic Isles.` (policy warnings and the verdict are logged).
13. The realms seam authorizes the realm (`realm` as JSON text, or none); a refusal is its code and message with the
    table's status; any other seam error `NOT_IMPLEMENTED`. With realms off (the default seam) a realm answers 501
    `REALMS_DISABLED` / `Realms are not available on this server.`; no realm passes. This is last because entering a
    realm may use an invite.
14. Store the exchange code (§5.2) as that player: hash, account id, launcher session id (`sid`), the target
    changelist, the normalized executable hash, channel, the approved runtime hash, manifest version and artifact-set
    digest, the realm id if any, created now, expires now + 60 s.
15. Tell the activity seam: a game session started, matchmaking presence touched (userId, now), the launcher build
    recorded. 200 `{"exchangeCode":<64 hex>,"expiresInSeconds":60}`.

#### 3.4.11 Noting a sign-in

Discord sign-ins and refreshes record `last_login_at` = now as that player unless the stored one parses and is 0 to
under 5 minutes old (then nothing is written and the account is answered as it is).

---

## 4. Middleware and pipeline hooks

### 4.1 Game auth (every game route that §3 marks "game", and every other area's game routes)

Headers are read as Node reads them: a repeated `authorization` keeps its first value; any other repeated header is
its values joined with `, `. In this order:

1. **Game server key.** `x-rustic-isles-gameserver-apikey`, else `x-rustic-isles-gameserver-key`, present (even
   empty): its HMAC (§5.3, `gameserver` scope) is compared with every stored game server hash; a store or secret
   failure: 500 page; no match: empty 401 (logged `Invalid Gameserver API Key Auth`). On a match:
   - the `INVALID` sentinel is re-pointed (§4.1.1);
   - with an `authorization` header: its first 7 bytes are dropped (no scheme check) and the rest must be a valid game
     token, else 500 page (Node does not guard it); then the eligibility check (§4.1.2): ineligible empty 403, failure
     500 page; the caller is that token's userId, a game server;
   - without one: the caller is a game server whose userId is the version's bearer-less account (§11 config:
     1.14.7's is `GAMESERVER_NO_PLAYER_USER_ID`, else `DEV_USER_ID`, trimmed, none when unset or blank) and has no
     claims.
2. **Player host runtime.** The realms seam may authenticate the request (path, headers, peer, request id):
   authenticated gives a caller with that runtime (its roster is the accounts it may act for, empty ids dropped);
   a refusal is answered with its status and JSON body; "not mine" goes on; a seam error is a 500 page.
3. **Session ticket (DN5).** `x-reforged-session-ticket`, trimmed, non-empty: it must parse as a
   `module_api::network::SessionTicket` token (else empty 401) and verify with the home for this module's game
   version (`HomeApi::verify_session_ticket`; else empty 401; an unparseable configured game version is a 500 page).
   The caller's userId is the admitted account's **global id text**, with claims `{"userId":..}`. No eligibility check.
4. **Bearer.** `authorization` must start with `bearer `, `Bearer ` or `BEARER ` (other casings are refused), else
   empty 401 (logged with method and path). The rest (after 7 bytes) must be a valid game token (§5.1), else empty
   401. Eligibility (§4.1.2): eligible goes on; ineligible empty 403; a failure empty 401.

The caller (**AuthData**) is: userId (when the claim is a string), whether a game server key authenticated it, the
token's claims, and the runtime if any. It is put into the request's extensions for other areas to read.

#### 4.1.1 The `INVALID` sentinel

Only for `GET` and `HEAD` with a valid game server key, when the version's policy names a path account (1.14.7:
`GAMESERVER_NO_PLAYER_USER_ID`, else `DEV_USER_ID`, else `mystpax`, trimmed; an empty set value stays empty) and it
is non-empty: every non-overlapping match, left to right, of `INVALID` as a whole path segment (preceded by the start
or `/`, followed by `/` or the end; case-sensitive) is replaced by the account encoded as `encodeURIComponent`. Only
the request URI's path changes (the query is kept); route params already bound keep the original. Logged:
`Gameserver INVALID sentinel resolved to '{account}' for {method} {path}`. Writes are never rewritten.

#### 4.1.2 Eligibility of a token's account

A token whose `userId` claim is not a string is eligible (service, dev and game-server identities). Otherwise the
launcher account of that userId: none is eligible; one is eligible when §6.1's play check passes (so a banned,
disabled, pending, rejected, timed-out or nameless account's game token is refused).

### 4.2 Launcher auth

1. An `authorization` header (any value) decides: without a `bearer `/`Bearer `/`BEARER ` prefix: 401
   `AUTH_UNAUTHORIZED` / `Sign in required.`; the rest must be a valid launcher token (§5.1), its account must exist,
   and its family (`sid`) must have an unrevoked, unexpired session of that account (§9.3); any of these failing,
   including a store failure: 401 `AUTH_UNAUTHORIZED` / `Your session has expired. Please sign in again.` Then
   admitted (§6.1, the 403s). The caller is `{userId, sid, account id}`.
2. Else `x-reforged-user-api-key`, trimmed, non-empty: the user key lookup (§5.3; a failure: the "expired" 401; no
   match: `Sign in required.`), its launcher account (none: "expired"), admitted. `sid` is `apikey`.
3. Else 401 `Sign in required.`

The crate also exposes the bearer half with the account it found (the Reforged API's launcher routes need both).

### 4.3 Active account

Given a userId and whether consent counts: no launcher account: 401 `AUTH_UNAUTHORIZED` / `Sign in required.`; a
store failure: `INTERNAL`; a deletion request: 403 `ACCOUNT_DEACTIVATED` with `scheduledFor`; with consent, §6.2's
enforced check (403 `CONSENT_REQUIRED`).

### 4.4 Rifts route callers

isles relays rustic-rifts routes; which auth stands in front of one is decided by method and route path:

| Route path | Caller |
| --- | --- |
| starts `/admin/` | the Reforged API's admin auth (isles' own) |
| starts `/launcher/v1/realms` | launcher auth; then active account with consent for `POST /launcher/v1/realms` and `POST /launcher/v1/realms/mine/...`; without consent for `POST /launcher/v1/realms/join`, `POST /launcher/v1/realms/presence/heartbeat` and `POST /launcher/v1/realms/player-invites/.../accept` |
| starts `/launcher/v1/realm-hosting/hubs/start` | launcher auth, active with consent |
| starts `/launcher/v1/p2p/` or `/launcher/v1/realm-hosting/` | launcher auth; active without consent for `/launcher/v1/p2p/host-presence`, `/launcher/v1/p2p/host-offers/.../respond`, and paths ending `/host-bootstrap`, `/join-bootstrap`, `/peers` |
| starts `/launcher/v1/guard/` | launcher auth |
| starts `/mystic/v1/hunts/` | launcher auth when it ends `/tickets/self`, `/status` or `/reward/self`; else anonymous (the host capability is checked by rifts) |
| anything else | anonymous |

The launcher caller is identified to rifts by its userId text. Refusals are the launcher envelope.

### 4.5 Launcher CORS (the pipeline calls it after the request budget)

For a path whose ASCII-lower-cased form starts `/launcher/` (any launcher path, routed or not). Allowed origins are
`http://localhost:1420`, `https://tauri.localhost` and `LAUNCHER_ALLOWED_ORIGINS` (comma list, JS-trimmed, empties
dropped), compared exactly.

- A non-empty `Origin` outside the list: empty 403, whatever the method (logged `[CORS] Rejected launcher origin`).
- An allowed `Origin` gives the headers `access-control-allow-origin: <origin>`, `vary: Origin`,
  `access-control-allow-methods: GET, POST, DELETE, OPTIONS`, `access-control-allow-headers: Content-Type,
  Authorization`, which are set (replacing any of the same name, so a redirect's `vary: Accept` becomes `Origin`) on
  whatever answers.
- `OPTIONS`: 204, with those headers when the origin is allowed, bare without an `Origin`.
- Other paths, and requests with no `Origin`, pass untouched. Game routes never get CORS.

### 4.6 Client address

The peer address comes from `axum::extract::ConnectInfo<service_kit::serve::ConnectionInfo>` (none: `unknown`). A
loopback peer (`::1`, or dotted `127.x.x.x`, optionally `::ffff:`-prefixed) speaks for a client: the
`x-forwarded-for` values (every header, comma-split, trimmed, empties and entries over 64 bytes dropped) are read
right to left and the first non-loopback entry is the client; none: the peer. Any other peer is its own address.

The **rate-limit key** of an address (for the request budget only) is `express-rate-limit` 8's `ipKeyGenerator(ip,
56)`: IPv4-mapped IPv6 becomes the IPv4; an IPv6 literal containing a dot that is IPv4-compatible becomes the IPv4;
other IPv6 is its `/56` network written `<compressed address>/56`; anything else stays as it is.

### 4.7 Rate limiting

Two in-memory limiters per module, reset on restart:

- **Attempt windows** (sliding): a key allows `max` attempts within `window` ms (an attempt counts while `now - t <
  window`). An attempt that is allowed is recorded; a refused one is not. Each window checked in a chain that passes
  records its attempt even when a later one refuses.
- **Request budget** (fixed, `express-rate-limit` `MemoryStore`): a key's window opens at its first hit and lasts
  60 s; every hit counts, refused ones included; refused when hits exceed the limit. It answers headers
  `ratelimit: "{limit}-in-1min"; r={max(limit - hits, 0)}; t={seconds to reset, rounded up}` and
  `ratelimit-policy: "{limit}-in-1min"; q={limit}; w=60; pk=:{pk}:` where `pk` is standard base64 of the first 12
  characters of the SHA-256 hex of the key; when refused also `retry-after: {seconds}`. The pipeline answers a
  refusal 429 `{"error":"Too many requests. Please retry later."}` and adds the headers to every answer. Buckets:
  `gameserver:{ip key}` (6000/min, any request carrying `x-rustic-isles-gameserver-apikey`), a player host's bucket
  from the guard seam when `x-mystic-host-capability` is sent (6000/min), else `client:{ip key}` (600/min). The
  pipeline decides where it counts (once per request).
- **Bounds** (a deliberate improvement over Node, keep it): each limiter tracks at most 16 384 keys; expired keys are
  swept at most once a minute (and when the cap is reached or the clock goes back); a new key at the cap evicts the
  key that expires first.

| Limit key | Max | Window | Refusal |
| --- | ---: | --- | --- |
| `register:{IP}` | 5 | 1 h | §3.4.1 |
| `login:{IP}`, then `login:{normalized email}` | 10 each | 5 min | §3.4.2 |
| `game-session:{userId}` | 10 | 5 min | §3.4.10 |
| `email-verify-ip:{IP}` / `email-verify:{email}` | 30 / 10 | 15 min | `Too many attempts. Try again later.` |
| `email-resend-ip:{IP}` / `email-resend:{email}` | 20 / 5 | 1 h | same |
| `password-forgot-ip:{IP}` / `password-forgot:{email}` | 20 / 5 | 1 h | same |
| `password-reset-ip:{IP}` / `password-reset:{email}` | 30 / 10 | 15 min | same |
| `unlink-start-ip:{IP}` / `unlink-start:{userId}` | 20 / 5 | 1 h | same |
| `unlink-confirm-ip:{IP}` / `unlink-confirm:{userId}` | 30 / 10 | 15 min | same |
| `mail-minute:{purpose}:{userId}`, `mail-hour:{purpose}:{userId}`, `mail-ip:{IP}` | 1, 5, 20 | 1 min, 1 h, 1 h | no mail (§7.3) |

---

## 5. Tokens, sessions, codes and keys

### 5.1 Signed tokens (RS256 JWT)

One RSA key pair: `AUTH_SIGNING_PRIVKEY_B64` and `AUTH_SIGNING_PUBKEY_B64`, each standard base64 (whitespace-trimmed)
of a PEM (PKCS#8 or PKCS#1 private key; SPKI or PKCS#1 public key). Parsed once per distinct pair. The header is
`{"alg":"RS256","typ":"JWT"}`. `iat` is floor(now / 1000) from the crate's clock; `exp` = `iat` + TTL.

| Kind | Claims, in order | TTL | Accepted `iss` / `aud` |
| --- | --- | --- | --- |
| Game account token | `userId`, `iat`, `exp`, `aud` `paradox-backend`, `iss` `paradox-backend` | 24 h | `paradox-backend` / `paradox-backend` |
| Launcher access token | `userId`, `sid` (refresh family), `iat`, `exp`, `aud` `rustic-isles-launcher`, `iss` `paradox-backend` | 12 min | `paradox-backend` / `rustic-isles-launcher` (and see §14 item 9) |
| Store purchase token | `userId`, `characterId`, `skuId`, `storeTag` (omitted when none), `iat`, `exp`, `aud` `rustic-isles-store-purchase`, `iss` `rustic-isles-store` | 10 min | `rustic-isles-store` / `rustic-isles-store-purchase` |

Validation: RS256 only, signature, `iss` and `aud` present and accepted, `exp` present and not passed (no leeway;
checked against the system clock), `nbf` checked when present. A game token answers all its claims. A launcher token
must carry string `userId` and `sid`, else invalid. A store token must carry string `userId`, `characterId`, `skuId`
and an optional string `storeTag`. The audiences keep one kind from being replayed as another. Isles uses the game
token for chat login (`userId` of a valid token) and the store token for the bounties store.

### 5.2 Opaque tokens

Refresh tokens, game exchange codes and Discord completion codes are 32 random bytes hex (64 characters), stored only
as the SHA-256 hex of their UTF-8 text (`abc` hashes to
`ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad`). Refresh sessions last 30 days (`expires_at` =
created + 30 d), rotate on every refresh, and reuse of a revoked one revokes its family. Exchange codes last 60 s and
are consumed at most once (the consume is atomic: unconsumed, `expires_at` after now). Session and family ids are
UUIDs.

### 5.3 HMAC hashes (`API_KEY_HASH_SECRET`)

The secret is the JS-trimmed setting; fewer than 32 UTF-16 units is refused (start fails with
`API_KEY_HASH_SECRET must contain at least 32 characters.`, and every hash fails). A hash is HMAC-SHA256 under the
secret's UTF-8 bytes of its parts joined by one NUL byte, lower-case hex:

| Use | Parts |
| --- | --- |
| Game server key | `gameserver`, key |
| Player account key (`RRK_...`, issued by the Reforged API) | `user`, key |
| Email code | `email-code`, purpose (`verify`, `reset`, `unlink`), userId, code |
| Erasure pseudonym | `account-pseudonym`, account id |

Stored hashes are compared in constant time after hex decoding; malformed hex, empty or unequal lengths never match.
A lookup compares every stored record (so the time does not reveal the position) and takes the first match. A user
key resolves to its account's userId (the `accounts` row of the matching `user_api_keys.account_id`).

`GAMESERVER_API_KEYS` at start: a comma list, JS-trimmed, empties dropped, de-duplicated in order. Non-empty: it is
the complete set (store replaces the stored hashes with exactly these; logged `Synchronized {n} Gameserver API key
hash(es) from GAMESERVER_API_KEYS`). Empty or unset: the stored keys stay (logged `Registered 0 new Gameserver API
Key(s) on boot!`). A key is never logged.

### 5.4 Passwords

Argon2id, version 19, m = 19456 KiB, t = 2, p = 1, 16-byte random salt, 32-byte output, PHC string
(`$argon2id$v=19$m=19456,t=2,p=1$...`), computed off the async threads. Verification accepts any well-formed Argon2
PHC string with its own parameters; a malformed or missing hash is false. With no hash (no account, or a Discord-only
account) a fixed dummy hash is verified so the time matches.

### 5.5 TOTP (the admin sign-in, exposed for the Reforged API)

RFC 6238: HMAC-SHA1, 30-second steps, 6 digits, the current step or one either side, compared in constant time. The
candidate must be a JSON string of exactly six ASCII digits. The secret is base32 (RFC 4648 alphabet), upper-cased,
with white space, `=` and `-` ignored; a character outside the alphabet refuses.

### 5.6 Encryption keys for the hub

In memory, one store per module. Issuing a key records `{key, nonce, token, at, account}` under the candidate id (if
non-empty), under the normalized token (if non-empty), and as the most recent. **Normalizing a token**: JS-trim; a
leading `bearer` (any ASCII case) followed by white space is removed; trim again (`bearerxyz` stays). Entries live
30 minutes (`now - at <= 30 min`); expired ones are dropped from the candidate and token maps at each lookup.

Lookup without a roster: by candidate, else by token, else the most recent within its TTL (flagged "unknown token"
when a token was given; it may belong to another player), else none. With a roster (a player host's runtime): by
candidate, then by token, a key counts only when its account is in the roster (otherwise "roster rejected"); never the
most-recent fallback. The **fingerprint** for logs is the first 12 hex of the SHA-256 of the normalized token, `none`
for an empty one.

---

## 6. Policy rules

### 6.1 Admission and play

- **Admitted** (sign-in): status `banned` 403 `AUTH_ACCOUNT_BANNED`, `disabled` 403 `AUTH_ACCOUNT_DISABLED`; then the
  effective approval (`null` reads as `approved`): `pending` 403 `AUTH_APPROVAL_PENDING`, `rejected` 403
  `AUTH_APPROVAL_REJECTED` (messages in §3.4.2).
- **Not blocked** (before codes are mailed): only the banned and disabled refusals.
- **Eligible for play**: admitted; not timed out (a timeout is in force while its `until` text is greater than now's
  ISO text; refused with code `AUTH_ACCOUNT_TIMED_OUT`, message `This account is timed out until {until}.`, details
  `timeoutUntil` and, when set, `timeoutReason`); `username_set` not stored false, else 403 `AUTH_USERNAME_REQUIRED` /
  `Choose a launcher username before playing.` A timed-out account still signs in to the launcher.

### 6.2 Legal, consent and deletion

Terms `2026-10-14`, privacy `2026-10-17`, minimum age 16; privacy versions still accepted: `2026-10-16`,
`2026-10-15`, `2026-10-14.2`. Deletion hold 30 days.

- **Current consent**: a stored consent whose terms equal the current and whose privacy is current or still accepted.
- **Required at sign-up** (`RUSTIC_ISLES_CONSENT_REQUIRED`, JS-trimmed, lower-cased): `0`, `false`, `off`, `no` off;
  any other non-empty value on; blank follows the database engine (on for Postgres, off for SQLite or none).
- **Enforced at play and hosting** (`RUSTIC_ISLES_CONSENT_ENFORCE`): on only for `1`, `true`, `on`, `yes`; then an
  account without a current consent is refused 403 `CONSENT_REQUIRED` / `Update the launcher to read and accept the
  current Terms of Use and Privacy Policy, then try again.`
- **Registration consent** (the body's `consent`): not an object: refused (400 `CONSENT_REQUIRED`, same message) when
  required, else none recorded. `ageConfirmed` not exactly `true`: 400 `CONSENT_AGE_REQUIRED` / `You must be at least
  16 years old to create an account.` Versions not current/accepted: refused when required, else none recorded.
  Otherwise recorded: terms current, privacy as sent, accepted now.
- **Deactivated**: a deletion request refuses with 403 `ACCOUNT_DEACTIVATED` / `This account is scheduled for
  deletion and is deactivated. Restore it from the Account page to play again.` and detail `scheduledFor`.

### 6.3 Usernames

**Format** (on the JS-trimmed string; a non-string is `""`): 3 to 16 UTF-16 units, else `Username must be 3–16
characters.` (U+2013); only ASCII letters and digits, else `Username can only contain letters and numbers (no spaces
or symbols).`; at least one letter, else `Username needs at least one letter.` All `AUTH_VALIDATION_FAILED`.

**Allowed.** Forms of the name: lower-cased, and that with digits read as letters (`0`→o, `1`→i, `3`→e, `4`→a, `5`→s,
`7`→t, `8`→b, `9`→g), and again with `1`→l; duplicates dropped. "Collapsed" text replaces each run of three or more
of one character by one (doubles stay). A term list **matches anywhere** when a form contains a term, or a collapsed
form contains a collapsed term whose collapsed length is at least 4 characters. A term list **matches whole** when
one of {a form, the form without trailing ASCII digits, the collapsed form} equals a term.

1. Not allowed (`That username isn't allowed. Choose another.`): anywhere in `fuck shit cunt bitch whore slut nigg
   faggot fagg retard penis vagina pussy dildo porn jizz twat wanker bollock asshole arsehole motherf chink tranny
   hitler kkk molest incest pedophil paedo sexy boob killyourself`; whole in `cock dick rape rapist coon spic pedo cum
   anal sex tits fag nig kms kys homo ass arse wank nazi dyke kike wtf`; anywhere in the operator's
   `RUSTIC_ISLES_USERNAME_BLOCKLIST` (split on JS white space and commas, trimmed, lower-cased, kept when 2 to 32
   characters of `a-z0-9`).
2. Reserved (`That username is reserved. Choose another.`): anywhere in `admin moderat owner staff support official
   developer gamemaster rusticisles mysticparadox mysticfox phoenixlabs`; whole in `mod mods dev devs gm root system
   server paradox dauntless null undefined everyone guest anonymous host player unknown`.

The message never says which term matched. Both are `AUTH_VALIDATION_FAILED`. Names are unique ignoring case among
live accounts (normalized = lower-cased).

### 6.4 Host switches

| Setting | Reading |
| --- | --- |
| `RUSTIC_ISLES_EMAIL_VERIFICATION` | JS-trimmed, lower-cased: blank off (default), `off`, `required`; anything else required (warned at start) |
| `RUSTIC_ISLES_REGISTRATION_APPROVAL` | blank off (new accounts `approved`), `off`, `required` (new accounts `pending`); anything else required (warned, with the value). A non-blank `RUSTIC_ISLES_REGISTRATION_AUTO_APPROVE` only warns that it has no effect |

### 6.5 Official play

`RUSTIC_ISLES_OFFICIAL_PLAY` (JS-trimmed, lower-cased): `on` available, `off` unavailable; blank available when
`DEPLOYSERVER_URL` is not blank, else unavailable; anything else unavailable, warned once per distinct value
(`RUSTIC_ISLES_OFFICIAL_PLAY must be on or off (got "{value}"); official play is treated as off`). Unavailable
carries the message `The official server is currently unavailable. Join or host a realm.`

### 6.6 Runtime authorization

- **Channel**: every account's expected channel is `stable` (the tester role no longer moves a launcher); another
  requested channel: 409 `AUTH_CHANNEL_MISMATCH` / `Your account currently requires the {expected} runtime channel.`
- **Manifest**: `<update root>/runtime/client/<channel>/windows-x86_64/latest.json`, the update root being
  `LAUNCHER_UPDATE_ROOT` (as set, even empty) else `updates`, made absolute against the working directory. Missing or
  not JSON: 409 `RUNTIME_BUILD_UNSUPPORTED` / `No approved {channel} runtime is currently published.`
- **Main hash** (exchange redemption): the manifest's `targetChangelist` (JavaScript `Number`) must equal the target
  (`The published runtime does not match the supported game build.`), its `sha256` JS-trimmed lower-cased must be 64
  hex (`The published runtime manifest has an invalid SHA-256 hash.`); the reported hash (normalized the same way,
  else 400 `runtimeSha256 must be a SHA-256 hash.`) must equal it (`The installed {channel} runtime is not the
  approved release.`). All 409 `RUNTIME_BUILD_UNSUPPORTED` except the validation.
- **Artifact set** (game-sessions): `runtimeManifestVersion` JS-trimmed must be a safe version
  (`^\d+\.\d+\.\d+([-+][0-9A-Za-z.-]+)?$`) else 400 `runtimeManifestVersion is invalid.` The manifest's `version`
  (trimmed) must be safe and its target right (`The published runtime manifest is invalid or targets another game
  build.`), `sha256` valid, `extraFiles` an array (`The published runtime manifest is incomplete.`), each extra file
  a safe name (1 to 96 of `A-Za-z0-9._-`) not repeated ignoring case and a valid hash (`The published runtime
  manifest has a duplicate or unsafe artifact.`); the approved set is `Rustic-Isles.dll` (the main hash) plus the
  extra files and must contain `Rustic-Isles.dll` and `winmm.dll` ignoring case (`The published runtime manifest
  does not contain the complete required artifact set.`). The reported `runtimeArtifacts` must be an array no longer
  than the known names (core plus the manifest's optional files), else 400 `The complete runtime artifact set is
  required.`; each entry's `name` (the legacy alias of §14 item 9 read as `Rustic-Isles.dll`) safe, unique ignoring
  case and known, else 400 `The runtime artifact set is malformed.`; each `sha256` normalized (400 `runtimeSha256 must
  be a SHA-256 hash.`); the core set required and the optional files all or none, else 400 `The complete runtime
  artifact set is required.` Then the reported version must equal the manifest's and every reported file's hash its
  manifest hash (name ignoring case), else 409 `The installed {channel} runtime set is not the approved release.`
- **Set digest**: the verified set sorted by `service_kit::collate::locale_compare` on the name, each line
  `{lower-cased name}:{sha256}`, joined by `\n`, SHA-256 hex. A core-only report and a full one digest differently.

### 6.7 Launcher builds

- **Report** (`launcher`): an object with `version` (three dot-separated groups of 1 to 5 digits, optionally `-` and
  a prerelease of 1 to 32 of `A-Za-z0-9.-`) and `sha256` (64 hex, lower-cased when kept); anything else is no report,
  never a refusal.
- **Verdict** (the activity seam computes it from the pinned launcher hash and the served version; default unknown):
  official when the hash equals the pinned one ignoring case; outdated when the report's version is below the served
  one; else unofficial; unknown with no report or no pin.
- **Policy**: `RUSTIC_ISLES_UNOFFICIAL_LAUNCHER` `refuse` refuses; blank or `observe` observes; anything else
  observes with a warning (value shown up to 32 characters). `RUSTIC_ISLES_LAUNCHER_ALLOWED_HASHES` (split on JS white
  space and commas, lower-cased, 64-hex kept, a warning counting the rest). `RUSTIC_ISLES_UNOFFICIAL_LAUNCHER_TESTERS_EXEMPT`
  exempts `admin` roles unless `0`, `false`, `off`, `no`. Only an unofficial verdict is refused, and not when its hash
  is allowed or the account is an exempt admin.
- **Version order** (semver precedence): a trailing `+build` is ignored when well-formed; core numbers compare as
  big integers (an overlong part counts as the largest); a release beats its prereleases; prerelease identifiers
  compare numeric-as-number, numeric below alphanumeric, else as strings; a longer equal-prefix list is greater;
  unparseable is never "at least".

---

## 7. Email codes, mail and Discord

### 7.1 Addresses

Valid: a string of 1 to 254 UTF-16 units with exactly one `@`, not first, a local part of at most 64 units, a domain
of at least 3 units that contains a dot and neither starts nor ends with one, and no space, tab, CR or LF anywhere.
Linear time on any input. **Masked**: split at the last `@` (none, or at position 0: `***`); the local part's first
UTF-16 unit, `***`, its last unit when the local part is longer than 2 units, `@`, the domain (`maxine@gmail.com` →
`m***e@gmail.com`, `mx@gmail.com` → `m***@gmail.com`).

### 7.2 SMTP settings

| Setting | Rule |
| --- | --- |
| `RUSTIC_ISLES_SMTP_HOST` | required (JS-trimmed, non-blank); problem `RUSTIC_ISLES_SMTP_HOST` |
| `RUSTIC_ISLES_SMTP_PORT` | blank 587; else decimal 1 to 65535; problem `RUSTIC_ISLES_SMTP_PORT (a port number)` |
| `RUSTIC_ISLES_SMTP_SECURE` | blank or `starttls` (required STARTTLS), `tls` (TLS from the first byte), any case; else problem `RUSTIC_ISLES_SMTP_SECURE (starttls or tls)` |
| `RUSTIC_ISLES_SMTP_USER` | required; problem `RUSTIC_ISLES_SMTP_USER` |
| `RUSTIC_ISLES_SMTP_PASSWORD` | required non-blank, kept exactly as set (not trimmed); problem `RUSTIC_ISLES_SMTP_PASSWORD` |
| `RUSTIC_ISLES_MAIL_FROM` | blank: `Rustic Isles <no-reply@mysticfox.dev>` (§14 item 8) |

Problems name keys, never values. "Mail configured" means no problems; while not configured the code routes answer
503 `AUTH_EMAIL_UNAVAILABLE` / `Email codes aren't available on this server right now. Try again later.`

### 7.3 Codes

Six ASCII digits, uniformly random, leading zeros kept; live for 5 minutes and 5 tries; one live code per account and
purpose (`verify`, `reset`, `unlink`), a new one replacing it. Only the HMAC (§5.3) is stored.

**Issuing** to an address: take the send slots (`mail-minute`, `mail-hour`, `mail-ip`, §4.7); any refused: "limited",
nothing stored or mailed. Else store the code (as that player) and mail it; a mail failure is "failed" (logged by
error code only; the stored code stays) and still used the minute slot.

**Accepting** a submitted code: not a string of six ASCII digits: refused, no try counted. Read the live code
(unexpired, tries below 5). If its hash equals the submitted one (constant time): delete it conditionally (still that
hash, unexpired, tries left) and accept when this call deleted it (so a second or parallel use fails). Otherwise, in
one atomic write: an existing live code counts one try (if that makes 5, it is removed); an expired or used-up one is
removed; refused.

### 7.4 The code mail

Subjects (exact, followed by `: {code}`): verify `Your Rustic Isles code`, reset `Your Rustic Isles password reset
code`, unlink `Your Rustic Isles code to unlink Discord`. A `multipart/alternative` message with a plain-text and an
HTML part. Both carry the code and `This code expires in {n} minute(s) and works only once.` (`minute` for 1); the
reset HTML contains `Reset your password`, the unlink HTML `Unlink Discord`, the unlink text `Discord stays linked`;
the HTML holds the code as an element's whole text (`>482913<`) and has no `<img`, `<script` or `<link`. Nothing the
user typed goes in, every value is HTML-escaped, and rendering refuses a code that is not six digits or an expiry
under 1 minute. Beyond those, the copy and the HTML design are the implementer's own (§14 item 7).

**SMTP dialogue**: connect (10 s timeout); every read and write 20 s. TLS mode: TLS first (webpki roots, SNI the
host). STARTTLS mode: greeting 220, `EHLO localhost` (250) must list a line equal to `STARTTLS` ignoring ASCII case,
else fail `ETLS` (the login never goes in clear), `STARTTLS` (220), TLS, `EHLO localhost` again. Then `AUTH PLAIN
<base64 of NUL user NUL password>` (235, else `EAUTH`), `MAIL FROM:<address>` (the address inside the From header's
angle brackets, or the trimmed text) 250, `RCPT TO:<to>` 250, `DATA` 354, the message with dot-stuffing then `.` (250),
then `QUIT` (its reply ignored). The message: `From`, `To`, `Subject` (RFC 2047 `=?UTF-8?B?...?=` when not ASCII),
`MIME-Version: 1.0`, `Content-Type: multipart/alternative; boundary="=_mp_<24 hex>"`, then the text and HTML parts,
each `charset=utf-8`, `Content-Transfer-Encoding: base64`, wrapped at 76. Replies: `NNN-` continues, `NNN ` ends; a
line under 4 characters is `ECONNECTION`. Failure codes (all that may be logged): `NOT_CONFIGURED`, `ERENDER`,
`ETIMEDOUT`, `ECONNECTION`, `ESERVERNAME`, `ETLS`, `EIO`, `EAUTH`, `EPROTOCOL`, `EPROTOCOL<code>`. The SMTP reply text
is never logged.

### 7.5 Discord

Configured when `DISCORD_CLIENT_ID`, `DISCORD_CLIENT_SECRET` and `DISCORD_REDIRECT_URI` are all non-empty. Discord
never creates an account; only `identify` is asked, so no email comes from it. The deep-link scheme is
`LAUNCHER_DEEPLINK_SCHEME` (default `rusticisles`).

#### 7.5.1 Starting

A transaction: `state` 24 random bytes hex (48), `code_verifier` 32 random bytes base64url, created now, expires in 5
minutes, and for a link the signed-in account id. `authorizeUrl` is
`https://discord.com/api/oauth2/authorize?` with `client_id`, `redirect_uri`, `response_type=code`, `scope=identify`,
`state`, `code_challenge` (base64url-unpadded SHA-256 of the verifier's ASCII), `code_challenge_method=S256`, in that
order, each name and value encoded as `URLSearchParams` does (`A-Za-z0-9*-._` kept, space `+`, the rest `%XX` upper
case).

#### 7.5.2 Callback

`code` and `state` count only when each appears exactly once. The answer is a redirect to a deep link; the channel is
`auth` until a link transaction is found, then `link`.

1. Either missing: `{scheme}://auth/error?code=AUTH_DISCORD_CANCELLED`.
2. Consume the transaction by `state` (atomic: unconsumed, unexpired). None (unknown, expired, replayed): auth error
   `AUTH_DISCORD_CANCELLED`.
3. Exchange the code with Discord (§7.5.5) with the stored verifier; a failure `AUTH_DISCORD_CANCELLED` (Discord's
   status logged, never shown); not configured `AUTH_DISCORD_NOT_CONFIGURED`.
4. A link transaction: the account must still exist (else `AUTH_DISCORD_CANCELLED`); an account that already has a
   link `AUTH_DISCORD_ACCOUNT_HAS_LINK`; a Discord user linked to any account `AUTH_DISCORD_ALREADY_LINKED`; store the
   identity (provider `discord`, subject = Discord id, username, avatar URL
   `https://cdn.discordapp.com/avatars/{id}/{avatar}.png` when Discord sent an avatar, linked now), the store's
   uniqueness refusal giving `AUTH_DISCORD_ALREADY_LINKED`: `{scheme}://link/done`.
5. A sign-in: the account linked to the Discord id, else `AUTH_DISCORD_NOT_LINKED`; a completion code (§5.2), its
   hash, an expiry of now + 60 s and the account attached to the transaction: `{scheme}://auth/complete?code={code}`.
6. Any refusal: `{scheme}://{channel}/error?code={code}` with the code percent-encoded (keeping `A-Za-z0-9-_.!~*`);
   a store failure is `INTERNAL` and logged.

**The redirect**: 302, `location` the URL, `vary: Accept`. HTML is chosen when `accept` contains `text/html` and
neither `text/plain` nor `*/*` occur, or `text/html` occurs first (substring positions, the `text/plain` position
taken before `*/*`'s): `content-type: text/html; charset=utf-8`, body `<p>Found. Redirecting to {URL with & " ' < >
escaped as &amp; &quot; &#39; &lt; &gt;}</p>`. Otherwise `content-type: text/plain; charset=utf-8`, body
`Found. Redirecting to {URL}`.

#### 7.5.3 Completing

`code` not a non-empty string: 400 `AUTH_DISCORD_CANCELLED` / `Discord sign-in didn't complete. Please try again.`
Consume by its hash (atomic: completion unconsumed, unexpired): none or no account attached: 400
`AUTH_DISCORD_CANCELLED` / `That sign-in link has expired. Please try again.` The launcher account (none: the first
message), admitted (§6.1), note the sign-in (§3.4.11), create a session; 200 session answer. Email verification is
not checked here.

#### 7.5.4 Unlinking

Both routes take their two limits first (§4.7), then in order: account (404 `NOT_FOUND`); no Discord link 403
`AUTH_DISCORD_NOT_LINKED` / `This account has no Discord account linked.`; no password hash 409
`AUTH_DISCORD_ONLY_LOGIN` / `Discord is this account's only sign-in, so it can't be unlinked.`; no or empty email 409
`AUTH_DISCORD_ONLY_LOGIN` / `This account has no email address to send the unlink code to, so Discord can't be
unlinked.`; mail configured (503).

- Start: issue an `unlink` code to the account's email; "failed" answers 503 `AUTH_EMAIL_UNAVAILABLE` / `The code
  email couldn't be sent. Try again in a minute.`; "sent" and "limited" (the earlier code is still live) answer 200
  `{"sent":true,"email":<masked>}`.
- Confirm (`code`): not accepted for `unlink`: 400 `AUTH_CODE_INVALID` (§3.4.5 message), the link stays; else delete
  the account's Discord identity as that player and answer 200 `{"account":<view>}` (now showing no link).

#### 7.5.5 Discord calls (behind a trait; a test never reaches the network)

`POST https://discord.com/api/oauth2/token`, `content-type: application/x-www-form-urlencoded`, body
`client_id`, `client_secret`, `grant_type=authorization_code`, `code`, `redirect_uri`, `code_verifier`, each name and
value percent-encoded with every non-alphanumeric byte as `%XX` (space `%20`). A 2xx JSON with string `access_token`
and `token_type`; then `GET https://discord.com/api/users/@me` with `authorization: {token_type} {access_token}`, a
2xx JSON user `{id, username, global_name?, avatar?}`. HTTPS only, webpki roots, HTTP/1.1, 15 s per request. A
non-2xx is a failure with Discord's status; anything else a failure without one. The client secret stays out of
`Debug`.

---

## 8. Background work and erasure

### 8.1 Unverified-registration sweep

Started only while verification is required: hourly (the first run at start, a slow run not overlapped). Up to 500
launcher accounts whose `email_verification_pending_since` is before now − 48 h and whose email is unverified, oldest
pending first; for each, one system write that soft-deletes the launcher and game account only if still pending and
unverified, then deletes its codes and sessions. Logs the count when non-zero (`[EmailSweep] removed {n} unverified
registration(s) older than 48 h`). The address and name are free again.

### 8.2 The 30-day erasure (Privacy Policy section 5)

**Due** accounts: live, `deletion_scheduled_for` at or before now, oldest scheduled first, a limit (0 = none).
**Erasing one**: no deletion request is an `INTERNAL` launcher error; a scheduled time that does not parse or is
after now is `DELETION_HOLD_EXPIRED` / `The hold has not ended yet.`; the erasure seam's blockers (live activity: the
account online, an active matchmaking candidate, a realm it hosts) leave it for the next run ("blocked" with the
reasons); `apply` false reports "dry run". Applied, in one write as that player: delete its refresh sessions, email
codes, exchange codes, Discord transactions (signed in or linking it), identities, user key, friend edges, the other
areas' rows (the seam), and soft-delete the launcher and game accounts (DD23, so the host's revert can bring them
back until the retention purge). Answers the counts named `emailCodes`, `gameExchangeCodes`,
`discordOAuthTransactions`, `authIdentities`, `userApiKeys`, `friendships`, `otherAreas`, `accounts`. Then the seam
removes the account's uploaded launcher logs (a failure is a warning). Logged `Account {userId} erased`.

**The job** runs only with `RUSTIC_ISLES_ACCOUNT_ERASURE` on (`1`, `true`, `on`, `yes`, JS-trimmed, any case):
hourly, 50 due accounts per run, logging waits and failures by code.

### 8.3 Start-up (`start`)

With no store configured, nothing. Otherwise, in order: the HMAC secret (§5.3) and the token keys (§5.1) must read
(start fails with their texts: `API_KEY_HASH_SECRET must contain at least 32 characters.`, `the token signing keys
can't be read: {not base64|no RSA private key|no RSA public key}`); log the verification mode (an unrecognised value
warned as required); under required, the SMTP settings must be configured, else start fails with
`RUSTIC_ISLES_EMAIL_VERIFICATION=required needs working SMTP settings ({problems joined by ", "} not set or invalid;
the required keys are RUSTIC_ISLES_SMTP_HOST, RUSTIC_ISLES_SMTP_USER and RUSTIC_ISLES_SMTP_PASSWORD)`; log the
approval mode and its warnings; synchronize the game server keys (§5.3); start the sweep (when required) and the
erasure job (when on). Returns the task handles, which isles aborts at stop.

---

## 9. Persistent state

### 9.1 Tables (isles-core migrations; the MIT crate does not own them)

Text times are §2.1's ISO text; `deleted_at` is a soft-delete stamp (`store`, DD23) and every read below sees live
rows only.

| Table | Columns (meaning) |
| --- | --- |
| `accounts` (`0001_accounts`) | `id` PK (account id), `user_id` unique (userId), `name` (game-facing name, mirrors the display name), `notes` (0), `deleted_at` |
| `launcher_accounts` | `account_id` PK → `accounts.id`; `email` (normalized, nullable; unique among live non-null); `display_name_normalized` (unique among live); `display_name`; `password_hash` (PHC, null for Discord-only); `status` (`active`/`banned`/`disabled`); `approval_status` (`pending`/`approved`/`rejected`, null = approved); `approval_updated_at`, `approval_updated_by`, `approval_reason` (admin writes; `""` when no reason); `roles` (JSON array text); `roles_updated_at`, `roles_updated_by`; `created_at`; `last_login_at`; `username_set` (1/0/null; null = legacy, counts as set); `consent_terms_version`, `consent_privacy_version`, `consent_accepted_at` (a consent only when all three are set); `deletion_requested_at`, `deletion_scheduled_for` (a request only when both set); `email_verified_at`; `email_verification_pending_since`; `deleted_at`; `timeout_until`, `timeout_since`, `timeout_reason` (`0610_timeouts`; a timeout only when until and since are set) |
| `auth_identities` | PK (`provider`, `provider_subject`); `provider` always `discord`; `account_id`; `provider_username`; `provider_avatar_url`; `linked_at`. At most one per account and one per subject |
| `refresh_sessions` | `id` PK; `token_hash`; `account_id`; `family_id`; `device_id`; `device_name`; `created_at`; `expires_at`; `revoked_at` |
| `game_exchange_codes` | `code_hash` PK; `account_id`; `launcher_session_id`; `build_changelist` (integer); `executable_sha256`; `runtime_channel`; `runtime_sha256`; `runtime_manifest_version`; `runtime_artifact_set_sha256`; `realm_id`; `created_at`; `expires_at`; `consumed_at` |
| `discord_oauth_transactions` | `state` PK; `code_verifier`; `created_at`; `expires_at`; `consumed_at`; `completion_code_hash`; `completion_expires_at`; `completion_consumed_at`; `account_id` (signed in); `link_account_id` (to link) |
| `email_codes` | PK (`account_id`, `purpose`); `code_hash`; `attempts`; `created_at`; `expires_at` |
| `game_server_api_keys` | `key_hash` PK |
| `user_api_keys` | `account_id` PK; `key_hash` (one key per account; written by the Reforged API) |

`friend_edges` (`0070_chat_friend_edges`) is touched only by the erasure.

### 9.2 Writes and actors

A write is attributed to an actor (`module_api::ActorContext`, DD23): **player** for a write to one account's own
rows (DD17: refused unless this node is the account's home, which surfaces as a store error and so `INTERNAL`),
**system** for registration, Discord transactions, exchange-code consumption, the sweep and key synchronization.

### 9.3 Operations the crate needs (the store trait, §11)

Each is atomic where it says so; isles' `store` serializes writes, so a select-then-update inside one write is.

| Group | Operations |
| --- | --- |
| Game accounts | by userId; by account id; create unless the userId exists (dev users) |
| Launcher accounts | by userId (joined to `accounts`); by account id; by normalized email; by normalized name; create (refuses email taken, then name taken; writes the game account unless it exists; returns created / email taken / name taken); set last login; set username (refuses a name another live account holds; also sets `accounts.name` and `username_set` 1); set consent; set or clear the deletion; set email verified (clears the pending mark); set password + verify + revoke all sessions (one write); list due deletions; list stale unverified registrations; conditionally delete an unverified registration with its codes and sessions (one write, whether it did); erase an account (§8.2, one write including the other areas, returns the counts) |
| Identities | by subject; by account; create (false when the subject or the account is linked); delete for account (count) |
| Refresh sessions | create; by token hash (revoked ones included); rotate (revoke one by id, create the next; one write); revoke family (unrevoked only); revoke all for account; is family active (an unrevoked session of the family and account whose `expires_at` is after now) |
| Exchange codes | create; consume by hash (atomic; unconsumed, `expires_at` after now; sets `consumed_at`, returns the row) |
| Discord transactions | create; consume by state (atomic; unconsumed, unexpired); attach completion (hash, expiry, account); consume by completion hash (atomic; completion unconsumed, unexpired) |
| Email codes | replace (one live per account and purpose); peek live; consume if still this hash, unexpired, tries left (whether it did); count a try (expired or used-up row removed, `None`) and remove the row when that try uses it up (one write) |
| Keys | all game server hashes; replace the game server set (an empty list empties it); all user hashes with their account ids |
| Health | a trivial query succeeds |

### 9.4 What the rest of isles expects

The Reforged API (admin, launcher API, host routes) writes launcher-account access state, roles, timeouts and user
keys and revokes unused exchange codes in the same tables; chat reads `accounts` and display names; matchmaking reads
eligibility and display names (`AuthReforgedCore`); isles-db's import, export and verify read and write all of them.
The crate must keep every column's meaning above.

---

## 10. Settings

### 10.1 Read by the crate

Read at each use through the settings trait (Node read `process.env`), so a test or overlay change applies at once.

| Setting | Default | Use |
| --- | --- | --- |
| `AUTH_MODE` | unset (500 at token) | §3.2 |
| `ALLOW_NO_AUTH_DEV_MODE`, `NODE_ENV` | off | §3.2 dev mode |
| `DEV_USER_ID` | `dev-user` (EOS), `mystpax` (`/accountinfo/public`, the sentinel) | §3.1, §3.2, §4.1 |
| `DEV_USER_NAME` | `Dev Slayer` | §3.2 |
| `GAMESERVER_NO_PLAYER_USER_ID` | unset | §4.1 (1.14.7 policy) |
| `AUTH_SIGNING_PRIVKEY_B64`, `AUTH_SIGNING_PUBKEY_B64` | required with a store | §5.1 (secret) |
| `API_KEY_HASH_SECRET` | required with a store | §5.3 (secret) |
| `GAMESERVER_API_KEYS` | keep stored | §5.3 (secret) |
| `APPROVED_EXECUTABLE_SHA256` | none approved | §3.4.10 |
| `LAUNCHER_UPDATE_ROOT` | `updates` | §6.6 |
| `LAUNCHER_ALLOWED_ORIGINS` | the two defaults | §4.5 |
| `LAUNCHER_DEEPLINK_SCHEME` | `rusticisles` | §7.5 |
| `DISCORD_CLIENT_ID`, `DISCORD_CLIENT_SECRET` (secret), `DISCORD_REDIRECT_URI` | off | §7.5 |
| `DEPLOYSERVER_URL`, `RUSTIC_ISLES_OFFICIAL_PLAY` | §6.5 | §6.5 |
| `RUSTIC_ISLES_EMAIL_VERIFICATION`, `RUSTIC_ISLES_REGISTRATION_APPROVAL`, `RUSTIC_ISLES_REGISTRATION_AUTO_APPROVE` | off | §6.4 |
| `RUSTIC_ISLES_CONSENT_REQUIRED`, `RUSTIC_ISLES_CONSENT_ENFORCE` | engine; off | §6.2 |
| `RUSTIC_ISLES_USERNAME_BLOCKLIST` | none | §6.3 |
| `RUSTIC_ISLES_UNOFFICIAL_LAUNCHER`, `RUSTIC_ISLES_LAUNCHER_ALLOWED_HASHES`, `RUSTIC_ISLES_UNOFFICIAL_LAUNCHER_TESTERS_EXEMPT` | observe | §6.7 |
| `RUSTIC_ISLES_SMTP_HOST`, `_PORT`, `_SECURE`, `_USER`, `_PASSWORD` (secret), `RUSTIC_ISLES_MAIL_FROM` | off | §7.2 |
| `RUSTIC_ISLES_ACCOUNT_ERASURE` | off | §8.2 |
| `RUSTIC_ISLES_RATE_LIMIT` (on unless `0`, `off`, `false`, `no`, trimmed, any case), `RUSTIC_ISLES_RATE_LIMIT_PER_MINUTE` (600), `RUSTIC_ISLES_GAMESERVER_RATE_LIMIT_PER_MINUTE` (6000), `RUSTIC_ISLES_HOST_RATE_LIMIT_PER_MINUTE` (6000) | | §4.7; the per-minute values read an optional `+` then the leading digits, a value under 1 or unreadable falls back |

### 10.2 Supplied by isles, not settings

The target changelist (1.14.7: 647472), the game version (for session tickets), the node identity, the version's
no-player policy and session payload, and the database engine (for §6.2's default).

No new setting, settings.toml field or env file line comes with the crate: it reads the module's existing settings.

---

## 11. The public seam

### 11.1 Shape

One crate, `rustic-isles-auth` (lib), MIT, in dainty-lass `crates/rustic-isles-auth`. It owns the routes and rules; isles owns
routing order, the pipeline, the tables and the neighbours. The crate depends only on dainty-lass MIT crates
(`module-api` for `HomeApi`, `SessionTicket`, `GlobalId`, `NodeIdentity`, `GameVersion`, `ActorContext`,
`BoxFuture`; `service-kit` for `json`, `jsjson`, `js`, `body`, `collate`, `serve::ConnectionInfo`, `routing`) and
workspace dependencies (`axum`, `http`, `serde`, `serde_json` with `preserve_order`, `tokio`, `tracing`, `sha2`,
`hmac`, `subtle`, `base64`, `rand`, `aws-lc-rs` for RS256 and HMAC-SHA1, `percent-encoding`, `regex`, `chrono`,
`thiserror`, `hyper`, `hyper-util`, `hyper-rustls`, `http-body-util`, `rustls`, `tokio-rustls`). It does not depend
on `rustic-rifts-seam`: restricted's rifts types are its own crate, so the crate defines the four realm calls it needs and
restricted adapts them.

**Seam changes for the integration pass** (not this row's): `argon2` (0.5, to read the PHC hashes accounts already
have) and `webpki-roots` (SMTP roots) as workspace dependencies; optionally `uuid` (v4) and `jsonwebtoken` (or
RS256 by hand on `aws-lc-rs`); `crates/rustic-isles-auth` and `rustic-isles-auth` in `[workspace.dependencies]`.

### 11.2 Interface (names are the contract the isles adapter is written against)

| Item | Contract |
| --- | --- |
| `pub trait AuthSettings: Send + Sync` | `fn raw(&self, name: &str) -> Option<&str>`: the value as set, empty included (Node's `process.env.X`) |
| `pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>` | Unix ms; isles passes its replay-aware clock |
| `pub type IdSource = Arc<dyn Fn() -> String + Send + Sync>` | A UUID v4 text; isles passes its replay-seeded source |
| `pub enum NoPlayerPolicy { Off, ConfiguredAccount }` | §4.1 and §4.1.1; `Off` keeps `INVALID` and gives a bearer-less game server no user |
| `pub struct AuthConfig` | `target_changelist: u32`, `game_version: module_api::GameVersion`, `node: module_api::NodeIdentity`, `no_player: NoPlayerPolicy`, `session_payload: fn(&str) -> serde_json::Value`, `engine_is_postgres: bool`, `clock: Clock`, `ids: IdSource` |
| `pub trait AuthStore: Send + Sync + 'static` | §9.3's operations, each `fn ...(&self, ...) -> BoxFuture<'_, Result<T, StoreError>>`, every write taking `&ActorContext`; records are the crate's public structs (`GameAccount`, `LauncherAccount` with `Consent`, `Deletion`, `Timeout`, `Status`, `Approval`; `Identity`; `RefreshSession`; `ExchangeCode` with `Channel`; `DiscordTransaction`; `EmailCode` with `Purpose`; `CreateOutcome`; `ErasureCounts`); `StoreError` carries a message (logged, answered `INTERNAL`) |
| `pub struct Seams` | `activity: Arc<dyn Activity>` (touch game session, classify launcher build, record launcher build, touch matchmaking presence), `operations: Arc<dyn Operations>` (guard enforcement, diagnostics profile, version), `realms: Arc<dyn Realms>` (authenticate a runtime: authenticated with roster / refused with status and JSON / not mine; launcher policy fields; authorize a game-session realm; bind realm presence at login), `notices: Arc<dyn Notices>` (the public JSON view), `game_login: Arc<dyn GameLogin>` (session supplies at `POST /login`), `erasure: Arc<dyn Erasure>` (blockers; uploaded-log removal), `home: Arc<dyn module_api::network::HomeApi>`, `mail: Arc<dyn MailTransport>`, `discord: Arc<dyn DiscordApi>`. `Default` = nothing changes: unknown verdicts, the default operations policy, realms off (§3.4.10's 501), empty notices, no supplies, no blockers, a standalone home, the crate's SMTP and HTTPS Discord |
| `pub struct Auth<S: AuthStore>` | `new(config: AuthConfig, settings: Arc<dyn AuthSettings>, store: Option<S>, seams: Seams) -> Arc<Self>` (`None` store: no database, as the skeleton module) |
| `Auth::start(self: &Arc<Self>) -> impl Future<Output = Result<Vec<JoinHandle<()>>, StartError>>` | §8.3; `StartError` displays the texts there |
| `pub struct Route { method: http::Method, path: &'static str, id: RouteId }`, `pub fn routes() -> &'static [Route]` | §3's 49, paths exactly as listed (isles' table has the same strings) |
| `Auth::handle(&self, id: RouteId, request: axum::extract::Request, params: Vec<(String, String)>) -> BoxFuture<'_, Response>` | Answers one matched request (the peer from `ConnectInfo<ConnectionInfo>`) |
| `Auth::game_auth(&self, request: &mut axum::extract::Request) -> BoxFuture<'_, Result<AuthData, Response>>` | §4.1; rewrites the URI and inserts `AuthData` into the extensions |
| `pub struct AuthData` | `user_id: Option<String>`, `is_gameserver: bool`, `claims: serde_json::Map<String, Value>`, `runtime: Option<RuntimeCaller>` (`roster()`: the account texts it may act for) |
| `Auth::launcher_auth(&self, headers: &HeaderMap)`, `Auth::launcher_auth_bearer(&self, authorization: Option<&str>)`, `Auth::require_active_account(&self, user_id: &str, consent: bool)` | §4.2, §4.3; `LauncherAuth { user_id, sid, account }`; errors are `LauncherError` |
| `pub fn rifts_caller_kind(method: &str, route_path: &str) -> CallerKind` | §4.4 (`None`, `Launcher { active: Option<bool> }`, `HostCapability`, `Admin`) |
| `Auth::launcher_cors(&self, request: &Request) -> Cors` | §4.5: `Pass`, `Answer(Response)`, `Headers(Vec<(HeaderName, HeaderValue)>)` |
| `pub struct RateLimiter`, `Auth::limiter(&self)` | `is_limited_at(key, max, window_ms, now) -> bool`, `check_request_budget_at(key, limit, now) -> BudgetVerdict { limited, rate_limit, rate_limit_policy, retry_after }`; `BudgetConfig::from_settings` |
| `pub fn client_ip(request: &Request) -> String`, `pub fn rate_limit_key(ip: &str) -> String` | §4.6 |
| `pub struct LauncherError` | `code`, `message`, `status: Option<u16>`, `details: Vec<(String, Value)>`; `status()`, `body(request_id)`, `IntoResponse`; `pub fn status_for(code: &str) -> u16` |
| Tokens on `Auth` | `sign_game_token(user_id)`, `validate_game_token(token) -> Map`, `sign_launcher_token(user_id, sid)`, `validate_launcher_token(token) -> (user_id, sid)`, `sign_store_token(&StorePurchase)`, `validate_store_token(token) -> StorePurchase` (`user_id`, `character_id`, `sku_id`, `store_tag: Option`); `TokenError` |
| Free functions | `hash_password(&str)`, `verify_password(Option<&str>, &str) -> bool`, `verify_totp(secret, candidate: Option<&Value>, now_ms) -> bool`, `is_account_eligible(&LauncherAccount, now_iso) -> bool`, `assert_account_admitted(&LauncherAccount) -> Result<(), LauncherError>`, `pseudonymous_account_id(...)` |
| Jobs on `Auth` | `sweep_unverified_registrations(now, limit) -> usize`, `due_deletions(now_iso, limit)`, `erase_account(&LauncherAccount, apply) -> ErasureOutcome` (`Blocked(Vec<String>)`, `DryRun`, `Erased(counts)`), `run_erasures(now_iso) -> usize`, `synchronize_gameserver_keys(value) -> usize` |
| `pub trait MailTransport`, `pub trait DiscordApi` | `send(&self, settings, MailMessage { to, subject, text, html }) -> BoxFuture<Result<(), MailError>>`; `exchange_code(&self, config, code, verifier) -> BoxFuture<Result<DiscordUser, DiscordFailure>>`; `SmtpTransport`, `HttpsDiscord` |
| `Auth::router(self: Arc<Self>) -> axum::Router` | Every route on `service_kit::routing` (Express matching) with the CORS gate, for the crate's own tests and a standalone host |
| feature `test-support` | `MemoryStore` (an `AuthStore` in memory with the atomic rules of §9.3), `RecordingMail` (keeps messages, can fail on demand), `StubDiscord` (code `code-<id>` is Discord user `<id>` with username `user-<last 4 of id>`, anything else fails) |

### 11.3 What isles does afterwards

`isles-core` implements `AuthStore` over `repo/auth.rs`, `AuthSettings` over its `Settings`, and `Seams` over the
Reforged API, inventory, progression, matchmaking, rifts and the module's home; builds one `Auth` per module in
`Shared`; registers each `routes()` entry with its `Handlers` calling `handle`; keeps `areas::auth::game_auth` as a
one-line call so the other areas are untouched; calls `launcher_cors` and the limiter from `http.rs`; points
`GameTokens` (chat), `StoreTokens` (bounties store), the Reforged API's launcher authenticator and admin sign-in, and
`AuthReforgedCore` at the crate. Then `areas/auth/*` is deleted.

### 11.4 Clean-room procedure

As `docs/port/clean-room/hostkit.md` records: the implementer's worktree has no dainty-restricted checkout and no
`modules/rustic-isles` tree; it reads this spec, the MIT crates and `docs/`. Record the session in
`docs/port/clean-room/rustic-isles-auth.md`, and measure the new files against restricted `e0e238b`'s `areas/auth/*` and
`repo/auth.rs` with `clean-room/similarity.md` §1's script; explain any file at or above 25 % (wire strings, status
tables, word lists and pinned constants are expected to match).

---

## 12. Test cases

### 12.1 What may be copied

- `crates/isles-core/tests/golden/auth/golden.json` (220 entries: 215 answers and the 5 `env`/`mailcode` steps) and
  `scenario.json` (the same 220 steps) are **data**: the
  first is the Node reference backend's recorded, normalized wire answers, the second the requests (generated). They
  may be copied into `crates/rustic-isles-auth/tests/golden/` and replayed against `Auth::router()` with the same
  normalization (§12.2), subject to §14 item 6.
- `auth_golden.rs`, `golden/auth/{make_scenario.py,record.cjs,run.sh}`, `auth_flows.rs`, `auth_repos.rs`,
  `auth_reforged_seams.rs`, `areas/auth/test_support.rs` and every unit test in the area are **authored test code**:
  described below only.

### 12.2 The golden replay

Base settings: `AUTH_MODE=LAUNCHER`, target 647472, `API_KEY_HASH_SECRET=test-only-api-key-hash-secret-0123456789abcdef`,
a throwaway RSA-2048 pair, `GAMESERVER_API_KEYS=gs-key-1`, `APPROVED_EXECUTABLE_SHA256` = 64 × `a`, a stable manifest
`{"version":"0.1.78","targetChangelist":647472,"sha256":<64 × b>,"extraFiles":[{"name":"winmm.dll","sha256":<64 ×
c>}]}`, `RUSTIC_ISLES_OFFICIAL_PLAY=on`, `RUSTIC_ISLES_RATE_LIMIT=off`, `NODE_ENV=production`, realms off; the stub
Discord and a recording mail. Each request carries `x-forwarded-for: 198.51.100.{n mod 250 + 1}` (n the step
number) from a loopback peer, so the per-IP limits do not bite. `env` steps add settings for the rest of the run
(Discord on; verification required with SMTP set); `mailcode` steps read the code last mailed to an address.
Normalization of every answer (strings and headers): JWTs `eyJ...` → `<jwt>`; UUIDs → `<uuid>`; ISO times →
`<iso>` except `2085-09-09...`; runs of exactly 64 hex → `<hex64>`, 48 → `<hex48>`; `code_challenge=` + 43 chars →
`<challenge>`; a `nonce` field → `<nonce>`. Kept headers: `content-type`, `location`, `cache-control`,
`access-control-allow-{origin,methods,headers}`, `vary`. JSON bodies are compared after `JSON.stringify` of the
normalized value (so field order counts); others as normalized text.

Selected cases (`{err CODE}` is the envelope with that code and §3's message; `-` is an empty body):

| Request | Expected |
| --- | --- |
| `GET /launcher/v1/username?name=ab` / `?name=Slay%20er` / `AdminBob` / `Dick69` / none / `Slayer1&name=Slayer2` | 200 `available:false` with the length / symbol / reserved / not-allowed / length / symbol reason |
| register `{"email":"nope",...}`; `{"email":..,"password":"short"}`; name `ab`; `AdminBob`; `Dick`; no body | 400 `{err AUTH_VALIDATION_FAILED}` with the email, password, length, reserved, not-allowed, email message |
| register `Slayer1`, `Slayer1@Example.Test` | 202 `{"approvalRequired":false,"account":{"userId":"<uuid>","displayName":"Slayer1","email":"slayer1@example.test","discordLinked":false,"discord":{"linked":false},"emailVerified":false,"status":"active","approvalStatus":"approved","needsUsername":false}}` |
| register same email; name `SLAYER1` | 409 `AUTH_EMAIL_TAKEN`; 409 `AUTH_DISPLAY_NAME_TAKEN` |
| register without a name; with `"  "` | 202, `displayName` `<uuid>`, `needsUsername` true |
| login wrong password; unknown email; `{}`; no body | 401 `{err AUTH_INVALID_CREDENTIALS}` |
| login `{"email":5,...}` | 500 `{err INTERNAL}` |
| login `" SLAYER1@example.test "`; form body | 200 session answer |
| login raw `{"email":` / `"text"` as JSON | 400 HTML page `Bad Request` |
| `GET /launcher/v1/me` no token; `Bearer not.a.token`; `Basic abc` | 401 `Sign in required.`; `Your session has expired...`; `Sign in required.` |
| set username `x`; `Moderator`; `slayer1`; `5`; `Slayer2` twice; no token | 400 length; 400 reserved; 409 taken; 400 length; 200 view ×2; 401 |
| refresh `{}`; 64 × `f`; the token; the same token again; then `me` with the second access token; refresh with the second refresh token | 401 `AUTH_REFRESH_INVALID`; 401; 200; 401 (family revoked); 401 expired; 401 |
| logout; `me` after; logout-all; `me` from another device | 200 `{}`; 401; 200 `{}`; 401 |
| policy | 200 `{"policyVersion":"<iso>:ops-0","roles":["player"],"channel":"stable","managedFeatureIds":[],"logUpload":{"auto":false},"guardEnforcement":"ENFORCE","diagnosticsProfile":"PRODUCTION"}` |
| consent without age; missing versions; terms `2020-01-01`; privacy `2026-10-16` | 400 `CONSENT_AGE_REQUIRED`; 400 `AUTH_VALIDATION_FAILED`; 409 `{"error":{"termsVersion":"2026-10-14","privacyVersion":"2026-10-17","code":"LEGAL_VERSION_OUTDATED",...}}`; 200 with `accepted.privacyVersion` `2026-10-16`, `upToDate` true |
| deletion `nope`; no body; `" slayer1 "`; again; game-sessions while deactivated; DELETE twice | 400 ×2; 200 request; 200 same; 403 `{"error":{"scheduledFor":"<iso>","code":"ACCOUNT_DEACTIVATED",...}}`; 200 `{"request":null,"holdDays":30}` ×2 |
| game-sessions no token; `preview`; no changelist; no executable; `beta`; changelist 1; exe 64 × `d`; manifest `0.1.77`; changed runtime hash; unknown extra file; a realm | 401; 400 channel; 400; 400; 409 `AUTH_CHANNEL_MISMATCH`; 400 build; 400 installation; 409 set; 409 set; 400 `The complete runtime artifact set is required.`; 501 `REALMS_DISABLED` |
| game-sessions ok | 200 `{"exchangeCode":"<hex64>","expiresInSeconds":60}` |
| oauth token, no body; `{"grant_type":"exchange_code"}`; `client_credentials` (JSON or form); unknown code; the code; the code again | 500 HTML; 400 `-`; 200 token answer without account fields; 401 `-`; 200 login answer; 401 `-` |
| oauth verify with the game token; a launcher token; none; `Bearer a b` | 200 (§3.2 order); 401 ×3 |
| public account; with a token; `nobody`; `dev-user` with a token; externalAuths; of `nobody`; `displayName/%20SLAYER1%20`; `Nobody` | 200; 200; 404; 404; 200 `[]`; 404; 200; 404 |
| bulk no token; `?accountId=a&accountId=b&accountId=nobody`; none; `dev-user` | 401; 200 two entries in order; 200 the caller; 200 `[]` |
| `POST /login` own userId; another; no body; no token; `garbage`; launcher token; `bearer ` lower; `BeArEr `; key only; wrong key; key + good token; key + `garbage` | 200 `TicketRateOk`; 400; 500 HTML; 401; 401; 401; 200; 401; 400; 401; 200; 500 HTML |
| accountinfo; of a key-only game server; tags; tags key-only | 200 `{"accountId":..,"creationDate":"2000-01-01 00:00:00","email":null,"preferredLanguage":null,"username":"Slayer1","verified":true}`; 500 HTML; 200; 200 `{"tags":[]}` |
| gamesession epic / epiceos; key-only | 200 payload of the token ×3 keys; 500 HTML |
| accountinfo/public b; `nobody`; `mystpax`; `{}`; no body; no token | 200 Slayer2; 200 `nobody` echoed with username `nobody`; 200 `mystpax` echoed with username `Slayer1`; 500; 500; 401 |
| key generate `cand-1`; `{}`; no token; consume by candidate, by token (`Bearer {b}`), by `?token=` (bare token), unknown token, no body | 200 `{"key":"<hex64>","nonce":"<nonce>","token":"Bearer <jwt>"}`; 200; 401; 200 ×5 |
| discord start / link start unconfigured | 503 `AUTH_DISCORD_NOT_CONFIGURED` |
| callback: nothing; unknown state; repeated `code`; for `accept: text/html,...` | 302 `rusticisles://auth/error?code=AUTH_DISCORD_CANCELLED`, text body; HTML body for the last |
| configured: start; callback unlinked user; link start; failed exchange; link callback; replayed; `me`; link twice; link for b of a linked user; sign-in; complete; again; without a code | 200 URL (§7.5.1, normalized); 302 `auth/error?code=AUTH_DISCORD_NOT_LINKED`; 200; 302 `link/error?code=AUTH_DISCORD_CANCELLED`; 302 `link/done`; 302 `auth/error?code=AUTH_DISCORD_CANCELLED`; 200 with `discord.username` `user-0001`; 409 `AUTH_DISCORD_ACCOUNT_HAS_LINK`; 302 `link/error?code=AUTH_DISCORD_ALREADY_LINKED`; 302 `auth/complete?code=<hex64>`; 200 session; 400 `That sign-in link has expired...`; 400 `Discord sign-in didn't complete...` |
| unlink start, resend, forgot without mail | 503 `AUTH_EMAIL_UNAVAILABLE` |
| `OPTIONS /launcher/v1/auth/login` origin `http://localhost:1420`; `https://evil.example`; none; `OPTIONS /launcher/v1/nothing` origin `https://tauri.localhost`; `GET legal` allowed; evil; `GET /isbanned` evil | 204 + 4 headers; 403 `-`; 204 `-`; 204 + headers; 200 + headers; 403; 200 no CORS |
| verification required: register `Slayer4`; login ×2; wrong password; verify without password; wrong password; `12ab`; the mailed code; again; `me` | 202 with `verificationRequired`, `email`; 403 `AUTH_EMAIL_UNVERIFIED` `s***4@example.test` ×2; 401; 401; 401; 400 `AUTH_CODE_INVALID`; 200 `emailVerified` true; 400; 200 |
| resend `nope`; verified account; nobody; forgot; forgot `""`; reset short; wrong code; nobody; the code; old access token; new password; old password; an old unverified account's login | 400; 200 `{"sent":true}` ×3; 400; 400; 400; 400; 200; 401; 200; 401; 403 `AUTH_EMAIL_UNVERIFIED` |
| unlink start; wrong code; the code; start again | 200 `{"sent":true,"email":"s***1@example.test"}`; 400; 200 `{"account":..discordLinked false..}`; 403 `AUTH_DISCORD_NOT_LINKED` |

### 12.3 Behaviour the authored tests fix (rewrite as black-box or unit tests)

| Area | Cases |
| --- | --- |
| Envelope | `AUTH_RATE_LIMITED` 429, `REALMS_DISABLED` 501, unknown 500, an explicit status wins; details first and never replace `code`: `{"error":{"scheduledFor":"2026-11-01T00:00:00.000Z","code":"ACCOUNT_DEACTIVATED","message":"gone","requestId":"rid"}}` |
| Time text | 0 → `1970-01-01T00:00:00.000Z`; 1790000000123 → `2026-09-21T14:13:20.123Z` |
| Client address | peer 8.8.8.8 + XFF 1.2.3.4 → 8.8.8.8; 127.0.0.1 + `1.2.3.4, 5.6.7.8` → 5.6.7.8; + `1.2.3.4, 127.0.0.2` → 1.2.3.4; `::1` → `::1`; `::ffff:127.0.0.1` + 9.9.9.9 → 9.9.9.9 |
| Rate-limit key | `127.0.0.1`, `198.51.100.10`, `unknown` unchanged; `::1` → `::/56`; `::ffff:127.0.0.1` → `127.0.0.1`; `2001:db8:abcd:ef12:1:2:3:4` and `2001:db8:abcd:ef99::1` → `2001:db8:abcd:ef00::/56`; `fe80::1ff:fe23:4567:890a` → `fe80::/56`; `2001:0db8:85a3:0000:0000:8a2e:0370:7334` → `2001:db8:85a3::/56` |
| Query | `name=Fox%20X` → `Fox X`; repeated → array (not a string); absent |
| Windows | max 2, window 1000: allowed at 0 and 10, refused at 20 and 30, allowed at 1011; other keys independent. 36 384 keys stay at 16 384 tracked and finish in under 5 s; at 70 000 ms one new key leaves 1 tracked |
| Budget `pk` | key `client:127.0.0.1` → `pk=:YjU5MDczYThlN2Fj:`; `gameserver:127.0.0.1` → `:OGJjODNiMzMzMjM2:` (computed from §4.7's rule) |
| Encryption keys | two players' keys found by their own tokens; `eyJleader.payload.sig` and `  bearer eyJleader.payload.sig ` find `BEARER eyJleader.payload.sig`'s key; `Bearer  abc ` → `abc`; `bearerxyz` stays; candidate beats token; a reissue under the same token replaces it; an unknown token gets the most recent with the flag; after the TTL nothing; fingerprint 12 hex, not in the token, `none` for empty (`eyJleader.payload.sig` → `ee5827e0de42`); roster lookups serve only roster accounts, flag rejection and unknown tokens, never the fallback, refuse keys without an account and an empty roster |
| Tokens | game round trip (`aud` `paradox-backend`); launcher round trip; neither opens the other; `garbage` invalid; store token with `storeTag` round trips and no other kind opens it; chat reads the game token's `userId` and refuses a launcher token; unreadable keys are a keys error |
| Hashes (secret of §12.2) | `user`,`RRK_x` → `b3a5c200029e048fe7747e1a3630111f6779d88f804a066b10dd3319f9c383ba`; `gameserver`,`gs-key-1` → `4ce6fee736d315052f090e75a9093695ea4a2903b3af0d220c5545192e319043`; `email-code`,`verify`,`u1`,`123456` → `2facdf75f9131ef6545a27938ee253222b949cd76c003e107835c872f7dfc246` (computed from §5.3); scopes differ; a short or missing secret fails; email codes differ by account, purpose and code |
| Hash compare | `00ff`=`00ff`; not `00fe`, not `00ff00`, not `""`=`""`, not `zz`; first matching record of `[aa, none, bb, bb]` for `bb` is the third |
| Codes and masks | `012345` valid; `12345`, `1234567`, `12 456`, `12345a`, full-width digits invalid; masks of §7.1, `a@b.co` → `a***@b.co`, `nope` and `@x.com` → `***` |
| Passwords | hash starts `$argon2id$v=19$m=19456,t=2,p=1$`, verifies, wrong one false, `not a hash` false, none false |
| TOTP | secret `GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ`, `287082` at 59 s and 89 s yes, at 149 s no; a number, 5 digits, `28708x`, absent, `not*base32` no; `gezd gnbv-gy3t qojq gezd gnbv gy3t qojq====` reads the same |
| Game server keys | ` first, second ,,first,third ` → first, second, third; unset, empty, ` , ` → none; a new configuration replaces the old (old refused, one 64-hex row); a blank configuration keeps stored keys; a user key finds its account, another key nothing |
| Eligibility | legacy (no approval) approved and eligible; pending, rejected, disabled, banned, `username_set` false ineligible; a timeout until 13:00 refuses play at 12:00 with `timeoutUntil`, `timeoutReason`, admits sign-in, ends at 13:00 |
| Consent | §6.2 defaults (Postgres on, SQLite and none off, blank follows the engine, `on`/`off` win); enforce off by default, `ON` on; registration consent cases (absent: refused on Postgres, none on SQLite; `null` none; `ageConfirmed` false or `"yes"` age refusal; old terms refused on Postgres, none on SQLite; privacy `2026-10-15` kept as sent); deactivated carries `scheduledFor`; enforced 403; transition versions accepted, `2026-10-14` not |
| Usernames | valid: `Fox`, `MysticFox`, `Fox99`, `x12`, `Sixteen16Chars16`, `  MysticFox  ` → trimmed; symbol: `Mystic_Fox`, `fox!`, `fox-x`, `fox x`, `fox.x`, `fóx`, `fox@home`, `<b>fox`; `123` needs a letter; `ab`, 17 × `A` length; absent and `5` refused. Reserved: `Admin AdminBob TheAdmin7 Moderator ModeratorX TheOwner Owner1 StaffFox Support FoxSupport Official Developer GameMaster RusticIsles MysticParadox MysticFox MysticFox2 PhoenixLabs Mod mod7 Dev GM Root Paradox Paradox99 Dauntless Player1 Host Guest Null`. Not allowed: `Fuuuuck Shiiiit Sh1t B1tch N1gg4 Pen15 Wh0re S3xy Dick Dick69 D1ck Cock Rape Nazi Hitler KKKfox Cum Tits Wanker Rapist Kys Killyourself Pussy Porn Faggot Retard Kike Coon` and assembled `fucker`, `Bigshit`, `nigga`, `nigger7`. Pass: `Peacock Hancock Grapefruit Therapist Raccoon Torpedo Sussex Essex Sextant Skyshard Swanky Nazir VanDyke Kikeo Assassin Classic Cassandra Bassist Cumulus Analyst Dickens Titan Shiitake Night Nigel Niger Homer Assault Moderna ParadoxHunter Hostage Embermane Mysticzzz Tempest7 QuietFox Kestrel Brimvane R4in Fox99 Glass Scrapes Drape Spice Cockpit`. Blocklist `Grief, toxic  b@d x`: `GriefLord`, `ToXiCfox`, `Toxic7` refused; `Badger`, `Xenon` pass; without it `GriefLord` passes |
| Switches | verification: default, blank default, `OFF` off, ` Required ` on, `on`/`true`/`maybe`/`requried` required + unrecognised; approval: default approved, ` OFF `, ` Required ` pending, `requried` pending with the value, legacy flag warns; official play: `DEPLOYSERVER_URL` set → on, none → off, blank URL off, blank value + URL on, ` ON ` on, `Off` off, `1 0 yes true disabled onn of` off |
| Launcher builds | policy default observe, ` Refuse ` refuse, `refuze` observe + 1 warning, exemption default on, `off` off; allowed hashes upper-cased kept lower, one malformed warned; refuse refuses only unofficial, not official, outdated, unknown, an allowed hash, an exempt admin, and refuses an admin when not exempt; verdicts (pinned `a1`×32, served `0.1.68`): pinned → official, `0.1.67` other → outdated, `0.1.68` and `0.1.69` other → unofficial, no report or no pin → unknown; reports: `0.1.68` + upper hash kept lower, `0.1.68-rc.1` ok, `0.1`, short hash, array, string, numeric version dropped |
| Versions | `0.1.35` ≥ `0.1.35`; `0.1.35+build.7` ≥; `0.1.35-rc.1` not; `rc.2` ≥ `rc.1`; `alpha` not ≥ `rc.1`; `999999999999999999.0.0` ≥ `2.0.0`; `not-a-version` not |
| Runtime | channels: `stable`, `beta`, `dev` parse, `preview` and absent refuse; every role set expects stable, beta and dev mismatch; main hash accepted upper-cased, other refused, missing or wrong-target manifest refused; full core set accepted (upper-cased hashes), names `Rustic-Isles.dll`, `winmm.dll`, digest 64 hex; a changed proxy hash 409, omitted file 400, extra unknown 400, old version 409, bad version 400; old runtime name equals the new; both names 400; empty `extraFiles` 409; optional set all or none with its own digest, each mutation 409, partial 400; a core-only manifest refuses unpublished files; extra manifest files not required of a launcher. Core digest for hashes b/c: `2aa2a8904fc44b11bb0be6ee5177c8904dc0f428c3dbb6ff2201a065fa5fff20` (computed) |
| Email flows | a code is mailed and only its HMAC stored; a right code accepted once, parallel uses succeed once; malformed codes cost no try; five wrong tries use the code up (the right code then fails); four wrong then right succeeds; accepted 4 min 59 s after issue, refused after 5 min and gone; a new code replaces the old, purposes separate; one per minute per purpose, reset has its own minute; 20 per hour per IP (the 21st refused); a failed mail uses the minute |
| Sweep | removes only stale unverified registrations with their codes and sessions; an account verified after being listed is kept |
| Sign-in notes | a stale or missing date is written; within 5 minutes skipped; a refresh writes the sign-in |
| Active account | unknown 401; deactivated 403 with `scheduledFor` (with or without consent); enforcement off by default; enforced hosting 403 `CONSENT_REQUIRED`; joining not gated; a current consent passes |
| Approval | default approved; required → pending and login 403 `AUTH_APPROVAL_PENDING`; ` OFF ` approved; typo pending |
| Game middleware | `GET /loadout/INVALID//all?x=1` with a key → `/loadout/mystpax//all?x=1`, with `GAMESERVER_NO_PLAYER_USER_ID= some user ` → `/loadout/some%20user//all?x=1`; a POST unchanged; bearer-less key call: `configured`, `dev-account` from `DEV_USER_ID`, none; a wrong key 401; a banned account's game token 403; the service identity's token passes |
| Erasure | a due account is erased (dry run first), counts include `userApiKeys` 1 and `accounts` 1, it is gone from every lookup and its key no longer resolves, another account untouched, its name and address free again; a hold not ended `DELETION_HOLD_EXPIRED`; blockers keep the account |
| Reforged seams | the registered authenticator admits a session and refuses the rest; registering through the Reforged API writes each row once; the admin sign-in checks hash and TOTP; the erasure job erases what is due once |
| Mail | complete settings: 587, STARTTLS, default From, password kept; 465 + `TLS` + From set; ` starttls `; missing or blank host, user, password each a problem naming the key; a bad port or mode a problem without its value; `0`, `65536`, `587.5` refused; subjects and phrases of §7.4; bad codes and expiry refused; envelope address from angle brackets, dot-stuffing; a scripted dialogue ends with `QUIT` and `RCPT TO:<fox@example.test>`; a 535 is `EAUTH` without the server's text; unconfigured `NOT_CONFIGURED` |
| Discord | `URLSearchParams` encoding (`a b*c-d.e_f` → `a+b*c-d.e_f`; `https://x/y?z=1` → `https%3A%2F%2Fx%2Fy%3Fz%3D1`); the token form (`a=b%20c&d%26=e%3Df`); a user with or without optional fields; the secret not in `Debug`. PKCE check: verifier `dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk` → challenge `E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM` (RFC 7636) |
| Legal | `GET /launcher/v1/legal` body `{"termsVersion":"2026-10-14","privacyVersion":"2026-10-17","minimumAge":16}` |
| Store | `auth_repos.rs` runs §9.3's operations on SQLite and a throwaway Postgres; for the crate, the same operations against `MemoryStore`, and isles keeps its two-engine test of its implementation |

---

## 13. Security behaviour (for `docs/security/rustic-isles-auth.md` and RX3)

- **Trust boundary.** The backend listens on loopback behind the gateway; `x-forwarded-for` is honoured only from a
  loopback peer (§4.6). The gateway refuses the host-only headers (`x-rustic-isles-gameserver-apikey` and the old
  `x-mysticparadox-*` name) from the internet; the crate must still treat a game server key as a credential checked
  against stored HMACs, never trusted by presence.
- **Secrets never logged:** the signing keys, `API_KEY_HASH_SECRET`, game server and account keys, passwords and
  their hashes, refresh tokens, exchange and completion codes, email codes, the Discord secret and verifier, the SMTP
  password and SMTP reply text. Encryption-key tokens appear only as fingerprints. Mail failures are logged by code;
  an address appears only masked. `Debug` of records and configs leaves out hashes, secrets and verifiers.
- **Stored only as hashes:** opaque tokens (SHA-256), keys and codes (HMAC-SHA256 with a ≥ 32-character secret),
  passwords (Argon2id). Comparisons of stored hashes are constant time and touch every record.
- **Enumeration:** login and verify answer the same 401 for an unknown address and a wrong password with equal work
  (dummy hash); resend and forgot always answer `{"sent":true}` and mail after answering; reset reveals a ban only
  after a right code. Registration's `AUTH_EMAIL_TAKEN` is the one admitted leak.
- **Single use and expiry:** exchange codes (60 s), completion codes (60 s), OAuth states (5 min), email codes (5 min,
  5 tries, atomic consume and try counting); refresh rotation with family revocation on reuse; logout, logout-all and
  password reset end sessions; the launcher access token is honoured only while its family is live.
- **Audiences** separate the three JWT kinds; RS256 only, no `none`, no leeway.
- **Admission** is server-side: pending accounts get no token; banned and disabled never get a code; the game token
  is refused for ineligible launcher accounts; exchange redemption re-checks build, eligibility, channel and runtime
  hash, and binds the realm authoritatively.
- **Rate limits** (§4.7) per IP and per account or address; memory bounded at 16 384 keys per limiter.
- **Input bounds:** JSON bodies 100 KiB; strict JSON first character; email validation linear time; API keys at most
  256 UTF-16 units at the token route; artifact names 1 to 96 safe characters, at most as many entries as known
  names; launcher versions and hashes by fixed grammar; manifest read from a fixed path under the update root only.
- **CORS:** launcher paths only, exact-match allowlist, unknown origins refused 403.
- **Discord:** PKCE S256, single-use state, `identify` scope only, HTTPS only with a 15 s timeout; redirects only to
  the configured deep-link scheme with a code-only error.
- **SMTP:** STARTTLS required on 587 (fail, never fall back to clear), certificate verification with webpki roots.
- **Known weak spots to list:** the encryption-key most-recent fallback can hand one player's key to another (§5.6,
  kept for request shapes that name nothing; host runtimes never get it); dev mode logs in as any id (two explicit
  flags, never with `NODE_ENV=production`); the INVALID sentinel and bearer-less game server attribution act as a
  configured account; the session-ticket path skips eligibility.

**Fuzz targets (`fuzz/fuzz_targets/`):** `rustic_isles_auth_text` (any bytes, lossy UTF-8): username format and allowed
checks, email validation and masking, the launcher report and version parsers, the safe-version and artifact-name
rules, the encryption-token normalizer, the client-address and rate-limit-key functions, the base32 TOTP reader and
the query decoder never panic, take time linear in the input, and keep their invariants (a valid username is 3 to 16
ASCII alphanumerics with a letter; a valid email has one `@`; a mask never contains the local part's middle; a
normalized token has no leading `bearer` + space). `rustic_isles_auth_tokens`: JWT validation of arbitrary text never
panics and accepts nothing not signed by the test key; the runtime artifact-set check of arbitrary JSON against a
fixed manifest never panics and accepts only sets matching the manifest.

---

## 14. Open questions and behaviour that looks wrong but is kept

1. **`AUTH_ACCOUNT_TIMED_OUT` answers 500** at `POST /launcher/v1/game-sessions`, because the code was added (timeouts,
   `e33dff5`) without a status row. The launcher may key on the code; 403 is surely meant. Kept until a decision row
   says otherwise.
2. **A game server key with a bad bearer is a 500 page**, and `/accountinfo` and `/gamesession/*` for a key-only
   caller are 500 pages (Node threw). Kept for parity; the golden records them.
3. **The session-ticket path** (DN5) admits without the eligibility check and gives the account's global id, not its
   userId, as `userId`. Areas that look the caller up by userId see a different text for a ticket caller than for a
   bearer caller. Kept; worth a DN row.
4. **Refresh device fallback**: the route always fills `deviceId`/`deviceName`, so a refresh without them gets a new
   random device id and `Unknown device` instead of keeping the session's. Kept.
5. **`Vary`**: an allowed origin's CORS headers replace a redirect's `vary: Accept` with `vary: Origin`. Kept.
6. **Copying the golden files.** They are recorded output of the AGPL Node reference plus a generated request list;
   they carry no code. This spec treats them as data (as the gateway and content parity files are), but whether data
   recorded from AGPL software may sit in an MIT crate's tests is the maintainer's call. If not, §12.2's table and a
   fresh recording against the MIT crate replace them.
7. **The code mail's copy and HTML.** Mystic Paradox's mail template is expressive text and layout, so §7.4 fixes
   only what tests and users depend on and leaves the rest to the implementer. The tested subjects and phrases are
   kept verbatim.
8. **The default From address** is `Rustic Isles <no-reply@mysticfox.dev>`, a domain Dainty Lass does not control. A
   host that sets no `RUSTIC_ISLES_MAIL_FROM` sends as someone else's domain (and fails SPF). Proposed: keep for parity
   in the rewrite, and decide separately to require the setting or change the default (a behaviour change, so a row).
9. **Pre-rename names** (`crate::legacy`, "remove with the release after" 2026-11-01): launcher tokens with audience
   `mysticparadox-launcher`, store tokens with issuer `mysticparadox-store` / audience
   `mysticparadox-store-purchase`, and a runtime artifact reported as `MysticParadox.dll` are still accepted. The MIT
   crate should not carry them unless the maintainer wants them for one more release; the isles adapter can map the
   DLL name before calling the crate if needed. The live wire names `paradox-backend` (game token), `mystpax`, `x-mystic-host-capability` and `/mystic/v1/hunts/` stay: the 1.14.7 client, shipped
   launchers or rustic-rifts use them.
10. **Paradox's terms.** Everything here is behaviour observed from a Mystic Paradox port. As with chat (spec-rd27-chat
    §7 item 8), the spec names wire facts and copies no code, so the additional terms should not reach the MIT crate;
    the word lists of §6.3 and the message texts are functional contract. The maintainer confirms before the
    implementer starts.
11. **`DELETION_HOLD_EXPIRED` for a hold that has not ended** (erasure called early) reuses a code whose name says the
    opposite. Internal only (not a route); kept.
12. **Discord completion skips email verification** while password sign-in enforces it under `required`. A linked
    account can sign in by Discord with an unverified email. Kept; flag for a decision.
13. **The JWT clock**: tokens are signed with the crate's (replay-aware) clock but validated against the system clock.
    A replay with a pinned past time signs tokens that may validate as expired. Kept; replays don't validate tokens.
14. **Seam ownership.** `AuthStore` makes isles' `repo/auth.rs` the only SQL; if the maintainer prefers the crate to
    own a SQL implementation on `store` (and the auth migrations), the table schema of §9.1 is the one it must match
    byte for byte, and the migrations would move with it (a seam change for the integration pass).

---

## 15. Coordinator decisions (2026-11-05, before implementation)

These settle §14 for the implementer and override anything above that disagrees.

- **Name.** The crate is `rustic-isles-auth` in `crates/rustic-isles-auth` (the MIT crates keep the module's name
  as `rustic-isles-*`; restricted's AGPL crates stay `isles-*`, so the two never collide in one graph).
- **Items 1-5, 11-13: parity.** Keep the behaviour as specified, including the 500s; each is a candidate decision
  row for the maintainer, not a change in this port.
- **Item 6: the goldens stay in dainty-restricted.** The implementer neither copies nor reads `golden.json` or
  `scenario.json`. Its tests are written from §12's tables. The golden replay is run in dainty-restricted against this
  crate once the isles adapter (§11.3) lands there, and a failure there is a bug on this side.
- **Item 7: own copy.** The code mail keeps only §7.4's tested subjects and phrases; the rest of its text and HTML is
  the implementer's own.
- **Item 8: parity for now.** The default From stays as specified; changing it is a separate row.
- **Item 9: no pre-rename names.** The crate does not accept the `mysticparadox-*` token names or the
  `MysticParadox.dll` alias; restricted's adapter maps the DLL name if a release still needs it.
- **Item 10: proceed.** The crate carries wire facts, word lists and message texts as functional contract only. It
  carries no Mystic Paradox code, so the section 7 additional terms do not attach; it names no Paradox project in
  prose beyond the wire names §14 item 9 keeps.
- **Item 14: the store trait.** `AuthStore` stays; restricted keeps the SQL and the migrations until the isles
  database layer is specified and ported as its own crate.
- **Amended 2026-11-05 with the D3 cutover (core §17):** §4.4's rifts caller kinds match `/rustic/v1/hunts/`, not
  `/mystic/v1/hunts/`, with no alias; the host capability header is `x-rustic-host-capability`. §14 item 9's
  `x-mystic-host-capability` and `/mystic/v1/hunts/` no longer stay.
