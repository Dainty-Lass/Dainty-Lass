# Clean-Room Behaviour Specification: `crates/rustic-isles` (RD27)

> **Ports moved 2026-11-10.** The loopback services' defaults are now backend 41000, Director 41001, content
> 41002, admin 41003, allowlist 41005, chat 41006 and QoS 41010, below the dynamic range Windows lends to Hyper-V,
> WSL and Docker. settings.toml's `[ports]` moves any of them, and on Windows setup and start move one Windows has lent
> out (a private gateway's 61000 to 41080). The numbers below are the ones recorded upstream: read each 610xx
> service port as its 410xx counterpart. A private gateway's default stays 61000.

> **Scope and purpose (RD27):** this is the clean-room behaviour specification for an MIT crate, `rustic-isles`, the
> module skeleton of Rustic Isles: how a request enters, the route table in Express's match order, the pipeline, the
> state the areas share, the settings, start-up and stop, the game data root and loader, the replay clock and ids,
> and the module's place in dainty-lass's `module-api`. It hosts `rustic-isles-auth`
> ([`spec-rustic-isles-auth.md`](spec-rustic-isles-auth.md) §11, §15) and is what later area crates plug into. Its
> source is dainty-restricted's `modules/rustic-isles/crates/isles-core` outside `src/areas/*` and `src/repo/*`,
> which is AGPL-3.0-only with Mystic Paradox's section 7 terms; an implementer who never sees it writes the MIT crate
> from this document alone.
>
> Everything below is stated from observable contracts: the routes and their bytes, the pipeline's order and
> answers, the settings, the module's manifest, registration, events and answers to the controller, the file formats
> and the tests. No implementation code is reproduced and no private helper is named. Node file names appear only as
> provenance. Wire names the 1.14.7 client, a shipped launcher, the runtime or rustic-rifts depends on (`/__origin/`,
> the `steelyard.ca` apexes, `x-rustic-isles-*`, `x-mystic-host-capability`, `/mystic/v1/hunts/`, `mystpax`) are
> interface facts and are kept.
>
> Written against dainty-restricted `e0e238b4434316bb210677d88bc59e3448688794` (worktree
> `splashloader-integration-90130d`; the files of this unit last changed in `7cd9db0`) and dainty-lass
> `7851ea55a47534b3a005b9072f84bf13aa343099` (branch `feat/isles-auth-clean-room`).

---

## 1. Provenance and licensing classification

### 1.1 Evidence used

| Source | What it says |
| --- | --- |
| `modules/rustic-isles/NOTICE.md` | The module is a modified Mystic Paradox (up to `21b9ff5`); "every crate and file" of it is AGPL-3.0-only with Mystic Paradox's section 7 terms, which apply to the Mystic Paradox contributions only |
| File headers (`//!`) | Which TypeScript each file ports, or which Dainty Lass row it was written for (table below) |
| `git log` of each file | Every commit is the maintainer's (37 commits, one author); born in `19ee75d` (RU4 skeleton), with `0a29728` (RU8 replay), `2824b6d` (RB11 generators, bootstrap), `8ad8443`/`c0482b0` (RU5 game data), `9ad97c6` (Rustic Isles rename, legacy names), `7cd9db0` (QoS) |
| `docs/inventory/routes-1.14.7.md` and `.json` (RU3a) | The 290 registrations of `ParadoxBackend` and `ParadoxDirector` in registration order, with the pipeline of `app.ts`, mount conditions and shadowed registrations |
| `docs/ru5-game-data.md`, `docs/GENERATING_GAME_DATA.md`, `docs/shadow-replay.md` | The game data files and loader, the replay pinning headers |
| `crates/isles-1-14-7`, `rustic-isles-plugin`, `isles-director`, `isles-db`, `isles-chat` | How the skeleton is used (read for the seam only; §16) |

### 1.2 Classification and what is rewritten

Category A is code written for Dainty Lass (RU4, RU8, DN rows) with no Paradox code in it; Category B ports Paradox
TypeScript. The maintainer holds the copyright of A and may relicense it, but the MIT crate's shape differs (no
`repo`, no `areas`; §11), and several A files call B, so **this spec covers every file and the implementer writes
all of it** (§15 item 11).

| Restricted file (`crates/isles-core/src/`) | Cat. | Source | Rewritten as |
| --- | --- | --- | --- |
| `lib.rs` | A | RU4 | §11 crate layout |
| `version.rs` | A | DD8 | §5.2 |
| `ids.rs` | A | DD17, DD18 | §2.2 |
| `legacy.rs` | A | rename `9ad97c6` | §6.5 (and §15 item 2) |
| `tables.rs` | A (shape) | RU3c seam 3, 6 | §5.2; the values are wire facts of the version crate |
| `hooks.rs` | mixed | RU3c; the no-player chain is `HasParadoxBackendAuth` | §5.2, §5.3 |
| `settings.rs` | mixed | RU3c seam 6 (A); `security/rateLimit.ts`, `config/adminApi.ts`, `security/managerOperator.ts` readers (B) | §6 |
| `module.rs` | A | DD3-DD5, DD8, DN2, DN5, RD25 admin actions | §7 |
| `http.rs` | B | `app.ts` pipeline, `observability/requestContext.ts`, `isAllowedOriginHost`, Express body rules | §3 |
| `routes/mod.rs` | A | RU3a's Express model (`check-route-inventory.mjs`) | §4.1 |
| `routes/table.rs` | A (data) | RU3a's inventory, itself Paradox's registrations (wire facts) | §4, Appendix A |
| `db.rs` | A | DD15, DD23, DN2 | §5.4 (the home seam); the database layer is §16's store unit |
| `testing.rs` | A | RU4 test support | §13.3 (described only) |
| `replay.rs` | A | RU8 | §9.3 summary; the harness is §16's replay unit |
| `game_data.rs` | A | RU3c seam 4 | §8.1 |
| `game_data/loader.rs` | B | `gameData/loader.ts` | §8.2 |
| `game_data/season_window.rs` | mixed | `seasonWindow.ts` (B); the replay scope (A, `0a29728`) | §8.4, §9 |
| `game_data/{progression_config,tuning,gauntlet,patchnotes}.rs` | B | `routes/system.ts`, `routes/tuning.ts`, `gauntletPayloads.ts` | Not here: the game-data unit (§16) |
| `game_data/generators.rs`, `src_tokens.txt` | B | `ParadoxBackend/scripts/generate_*` | Not here: the game-data unit (§16) |
| `game_data/golden.rs` | A | RU5 golden harness | Not rewritten: stays with the restricted goldens (§13.1) |
| `bootstrap.rs` | B | `scripts/bootstrap_admin.ts` | Not here: called by nothing at run time, only by a test (§15 item 10) |

### 1.3 What stays in dainty-restricted

- `src/areas/*`, `src/repo/*`, the migrations, `crates/isles-chat`, `isles-director`, `isles-db`, `isles-1-14-7`,
  `rustic-isles-plugin`, `native/`, `tools/RuntimeLoader`, `reference/`, and every golden under `tests/golden/`.
- The **composition** (§11.4): restricted's version crate implements this crate's `Composer` with today's areas,
  `IslesStore`, the auth store adapter, rifts, chat rules, the QoS listener and the party outbox, until each is
  ported (§16).
- `game_data/golden.rs` and the golden replays: they run in restricted against this crate once composed.

---

## 2. Conventions

### 2.1 Text, JSON, times

As `spec-rustic-isles-auth.md` §2.1 and §2.2: JSON as `JSON.stringify` writes it (`service_kit::json::Json`,
`content-type: application/json; charset=utf-8`, insertion order, so `serde_json` with `preserve_order`; §15 item 8),
"empty" is a status with no body and no content type, times are Unix ms from the replay-aware clock (§9) and ISO
texts are `Date#toISOString`'s.

### 2.2 Ids

- **Account id**: `module_api::GlobalId` of kind `Account`, `account:<ULID>@<node>`, minted on the module's node
  (`GlobalId::new(IdKind::Account, node)`); its owner is its home. Parsing text that is a global id of another kind
  is refused ("an account id is account:<ULID>@<node>").
- **userId**: the client-visible id, any text; a new one is a UUID v4 text (36 characters) from the id source (§9.2).

---

## 3. The pipeline

One dispatcher per listener answers every request (no per-route framework routing). In order:

### 3.1 Every service, before matching

1. **Legacy header copy** (§6.5; §15 item 2): for each `x-mysticparadox-<rest>` header whose `x-rustic-isles-<rest>`
   name the request does not also send, every value is appended under the new name. The old headers stay.
2. **Replay scope** (§9.1): read from `x-isles-replay-now-ms` and `x-isles-replay-seed`; the rest of the request
   runs inside it.
3. **Request id**: the first of `x-rustic-isles-request-id`, `x-request-id` (each the first value, as text) that is
   *safe* once trimmed: 8 to 96 characters, the first an ASCII letter or digit, the rest ASCII letters, digits, `.`,
   `_`, `:`, `-`. It is used trimmed. None safe: a fresh UUID v4 (from the system, not the replay seed).
4. **Observation** (§11.2 `Observer`): begins with the request and the id (the request log and metrics are the
   Reforged API's; it reads the target before the origin strip).
5. The answer (§3.2 for the backend, §3.8 for others).
6. The observation finishes with the answer's status, then `x-rustic-isles-request-id: <id>` is set on the answer
   (replacing any), on every answer including 400, 404, 429 and 501.

### 3.2 The backend: order of steps

Positions below are indexes in the full route table (Appendix A). Two marks are fixed at build time:

- **The budget mark**: the position of `GET /internal/v1/metrics` (B5). Routes before it (`/`, `/health/live`,
  `/health/ready`, `/health/release-gates`) are outside the request budget and outside launcher CORS.
- **The admin mark**: the position of the first route whose path starts `/admin/`.

Steps:

1. **Origin strip** (§3.3). A refusal is an empty 400.
2. **Early budget**: a path that is `/mystic/v1/hunts` or starts `/mystic/v1/hunts/` (ASCII case-insensitive) is
   counted now (§3.4); refused: the 429 answer, nothing else runs. Counted requests are not counted again.
3. **Walk the mounted routes in table order.** On reaching the budget mark (the first mounted route at or after it):
   count the request unless already counted (refused: 429, end), then run **launcher CORS** (§3.5). On reaching the
   admin mark: with the admin API off, a path that is `/admin` or starts `/admin/` (ASCII case-insensitive) answers an
   empty 404, end.
4. For each route: skip unless its method accepts the request's (§4.1; `HEAD` reaches `GET` routes); skip unless its
   pattern matches the (stripped) path; a parameter that is not valid percent-encoded UTF-8 answers an empty 400, end;
   a `gauntlet` route (§4.2) is skipped unless the origin host contains `gauntlet`. The first route left answers
   (§3.6), end.
5. **No route matched.** If the walk never reached the budget mark: count now (refused: 429, end) and run launcher
   CORS. If it never reached the admin mark, the admin gate applies here too. Then log warning
   `Unstubbed route {METHOD} {path}` and answer an empty 404.

Every answer after counting carries the budget headers (§3.4); every answer after CORS gave headers carries them
(set, replacing names already present). A CORS refusal or `OPTIONS` answer is returned as CORS made it, with the
budget headers added.

### 3.3 The `/__origin/<host>/` strip

The 1.14.7 runtime's `SetURL` hook prefixes the official host to every path. On the request target (path and
query):

- Not starting `/__origin/`: unchanged, no origin host.
- The host segment runs to the first `/`, `?` or `#` after the prefix. Empty: unchanged, no origin host (the path
  keeps the prefix and usually ends in the 404).
- The segment is decoded as `decodeURIComponent` (a `%` not followed by two hex digits, or bytes that are not UTF-8:
  refused), then lower-cased (Unicode). It must be 1 to 253 bytes of `a-z`, `0-9`, `.`, `-`, not start or end with
  `.`, contain no `..`, and equal one of `steelyard.ca`, `steelyard.online`, `ol.epicgames.com`, `api.epicgames.dev`
  or end with `.` + one of them. Otherwise refused.
- The rest becomes the target: empty becomes `/`; one starting `/` is kept; anything else gets a leading `/`
  (`?x=1` becomes `/?x=1`). A target that does not parse as a URI: refused.
- The host is kept as the request's **origin host** for handlers (and for `gauntlet` routes).

Refused means an empty 400 before anything is counted.

### 3.4 The request budget (placement only)

The bucket, limit, headers and 429 body are `spec-rustic-isles-auth.md` §4.7 (the crate's limiter); this crate
decides where the one count happens (§3.2) and picks the bucket:

1. Off (`RUSTIC_ISLES_RATE_LIMIT` off, auth §10.1): nothing is counted and no budget header is sent.
2. The request carries `x-rustic-isles-gameserver-apikey` (present, any value, empty included): bucket
   `gameserver:{rate-limit key of the client address}`, the game server limit. The key is not checked here; auth
   checks it later.
3. Else `x-mystic-host-capability`, first value trimmed, non-empty: rifts' guard is asked for a bucket name given a
   request summary (path, that one header trimmed, the **peer** address, the request id). A non-empty name is the
   bucket with the host limit; an empty name, none, or an error falls through.
4. Else `client:{rate-limit key}` with the client limit.

The client address and its rate-limit key are auth §4.6 (`x-forwarded-for` only from a loopback peer; IPv6 grouped
by /56). The verdict's clock is the replay-aware clock (§9; §15 item 1).

### 3.5 Launcher CORS (placement only)

At the budget mark the backend asks the auth crate's `launcher_cors` (auth §4.5): pass, an answer to return (403 or
the `OPTIONS` 204), or headers to set on whatever answers. No other service has CORS. There is no Express automatic
`OPTIONS` answer outside launcher paths: `OPTIONS /character` is the 404 fallthrough (§15 item 3).

### 3.6 Answering a matched route

The handler gets a **call**: the shared state (§5.1), the route row, the decoded params, the request (its URI already
stripped, every header as received), the origin host, the request id.

- **An area registered a handler** for the row: its answer.
- **No handler** and the row is not a rifts row: 501 `{"error":{"code":"NOT_IMPLEMENTED","message":"isles: {METHOD} {path} is not implemented yet","requestId":"<request id>"}}`,
  where `{path}` is the row's path as written (a regex row shows its literal, e.g. `isles: GET /^\/progression\/([^/]*)\/([^/]+)$/i is not implemented yet`).
- **A rifts row answered by rustic-rifts** (§4.2): relayed (§3.7).

### 3.7 Relaying a rifts route

1. **Caller**: auth's rifts caller for (method, request path, route path, headers) (auth §4.4); a refusal is answered
   as auth made it.
2. The call to rifts carries: the rifts route, the decoded params, the raw query (without `?`, empty when none), the
   request path, every header whose value is valid text (others dropped), the remote address (the client address of
   auth §4.6 when it parses as an IP, else the peer), the request id, the caller, and the body bytes (at most 100 KiB:
   more is an empty 413).
3. The answer:
   - a reply: its status (an invalid one 500) with `content-type: application/json; charset=utf-8` and the reply's
     body text as it is;
   - "unimplemented": rifts-seam's 501 reply for it (`{"error":{"code":"NOT_IMPLEMENTED","message":"<component>: <operation> is not implemented yet","requestId":<id>}}`);
   - a refusal with code and message: `{"error":{"code":<code>,"message":<message>,"requestId":<request id>}}` with
     the status auth's `status_for(code)` gives (auth §2.3; note the request's id here, not a fresh UUID; §15 item 7);
   - any other error: logged `rifts route {METHOD} {path}: {error}`, empty 500.

### 3.8 Other services (the Director)

Same §3.1 steps; no origin strip, no budget, no CORS, no admin gate. Mounted routes in table order, same matching,
the 400 for a bad parameter, the same 501 for a row without a handler, the 404 fallthrough with its log line. A
service may wrap the dispatcher (the Director adds its own body parser and Express 404 page).

### 3.9 Bodies

The pipeline parses no body. Handlers read `req.body` with the crate's helper, which follows auth §2.2's rules with
a limit (default 100 KiB): reuse a body `service_kit::body` already parsed, else parse here. "Has a body" is a
`transfer-encoding` header, or a `content-length` that reads as a number, or no `host` header. Only
`application/json` and `application/x-www-form-urlencoded` (essence, ASCII case-insensitive) parse; others, or no
content type, or no body, give "absent". A charset other than `utf-8`: 415; a `content-encoding` other than
`identity`: 415; over the limit: 413; bytes that are not UTF-8: 400; JSON: a leading BOM dropped, empty text is `{}`,
the first non-blank character must be `{` or `[` (400 otherwise), parsed as `JSON.parse` does
(`service_kit::jsjson`); forms with `service_kit::body::parse_form`. Errors answer Express's error pages
(`service_kit::body::error_page`). A malformed body on a route whose handler never reads it is not refused
(§15 item 4).

---

## 4. The route table

### 4.1 Matching (Express 5)

A row's path is an Express path or a regex literal.

- **Express path**: starts `/`; segments are literals or `:name` (named, non-empty name); a literal may not contain
  `*`, `{`, `}`, `?`, `(`, `)`. One trailing `/` of the pattern is dropped (not of `/` itself). Empty segments are
  literals: `/loadout/:userId//all` has an empty third segment.
- **Request path**: must start `/`; one trailing `/` dropped (not of `/`). Segment counts must be equal; a literal
  matches ASCII case-insensitively on the raw (still percent-encoded) text; a param matches any **non-empty** segment.
  Params are then percent-decoded as UTF-8 (`+` stays `+`); a param that fails decoding refuses the request (400).
  `/a//` does not match `/a`.
- **Regex literal** `/^...$/flags`: the text between the leading `/` and the last `/` is the regex, flags may only be
  `i` (case-insensitive). It runs against the raw path as written (no implicit anchors beyond its own); captures are
  params named `0`, `1`, ... (an unmatched group is the empty string), percent-decoded as above.
- **Methods**: `GET`, `POST`, `PUT`, `PATCH`, `DELETE`. A `GET` row also takes `HEAD`.

| Pattern | Path | Result |
| --- | --- | --- |
| `/account/api/oauth/token` | `/Account/API/oauth/TOKEN/` | match, no params |
| `/account/api/oauth/token` | `/account/api/oauth/token//` | no match |
| `/motd/` | `/motd` | match |
| `/` | `/`; `/x` | match; no match |
| `/friends/:id` | `/friends/` | no match |
| `/loadout/:userId//all` | `/loadout/A%20B//ALL` | `userId` = `A B` |
| `/loadout/:userId//all` | `/loadout/a/all` | no match |
| `/loadout/:userId/:characterId/all` | `/loadout/a//all` | no match |
| `/^\/inventory\/([^/]*)\/(.+)$/` | `/inventory//dauntlessrel-1.14.7:647472` | `0` = ``, `1` = `dauntlessrel-1.14.7:647472` |
| `/^\/inventory\/([^/]*)\/(.+)$/` | `/INVENTORY/a/b` | no match (no `i`) |
| `/^\/leaderboard\/get_leaderboard\//i` | `/Leaderboard/get_leaderboard/x/y`; `/leaderboard/get_leaderboard` | match; no match |
| `/x/:id` | `/x/%E2%28` | refused, bad param `%E2%28` |

### 4.2 Rows and mount conditions

Each row is (service, method, path, group, side, when). **Service** is `backend` or `director`; **group** is RU5's
area group for isles rows (`auth`, `reforged-api`, `characters-loadouts`, `inventory-reforge`, `progression`,
`bounties-store`, `matchmaking-parties`, `chat`, `game-data`, `director`) or the rifts source area (`guard`, `p2p`,
`realms`); **side** `isles` or `rifts`. **When**:

| When | Mounted |
| --- | --- |
| always | always |
| `admin` | only with the admin API on (§6.3) |
| `gauntlet` | always, but answers only when the origin host contains `gauntlet`, else the walk goes on |
| `rifts` | only when rustic-rifts is loaded (`Rifts::loaded()`); answered by an isles handler for the row if one is registered, else by the rifts route with the same method and path text (none: building the app fails `rustic-rifts is loaded but answers no {METHOD} {path}`) |

A row's **scope** is "node" when its path starts `/launcher/`, `/admin/`, `/health/`, `/internal/v1/metrics` or
`/reforged/api/`, else "version" (the launcher's, operators' and probes' routes are no game version's; areas may use
it). A row's **name** is `{METHOD} {path}`.

Counts (tests check them): 306 rows, 301 backend (234 isles, 67 rifts) and 5 Director (4 isles, 1 rifts); 33
`admin` (every `/admin/...` row plus `/health/release-gates`), 7 `gauntlet` (including `/config`), 60 `rifts`. The 8
rifts-side Guard rows isles keeps (RF1 decision 1: `/launcher/v1/guard/sessions`, `/launcher/v1/guard/heartbeat`,
`/launcher/v1/guard/sessions/:guardSessionId`, both `guard-manifests` rows and the three `/admin/v1/guard/*` reads)
are `always` or `admin`; every other rifts-side row is `rifts`; no isles-side row is `rifts`.

### 4.3 What clients see of the order

The table is Express's registration order; the first matching mounted row answers. Registrations that never answer
(an earlier row always takes them) stay in the table because the client sees the earlier one:

| Registration | Taken by |
| --- | --- |
| The Reforged API's own `GET /dauntless-status` (one row in the table, B25) | Paradox's `system.ts` handler, mounted first: the client gets Paradox's welcome text, never the Reforged name, version and source URL |
| B69 `POST /inventory/:characterId/:changeList` | B68, the inventory regex registered first on purpose (the empty character id); the same answer |
| B185 `GET /launcher/v1/runtime/:channel/:platform/download` | B182 `GET /launcher/v1/runtime/:target/:channel/:platform`: 400 (the channel is not a target), or the manifest when the channel is `client` or `server` |
| B41 `GET /playertreatments/` | B40 `GET /playertreatments` (one handler) |
| B114 `POST /encountered-content/` | B113 `POST /encountered-content` (the same answer) |

Rows that overlap but pass on: the seven `gauntlet` rows answer only for a Gauntlet origin, so `GET /progression/a/b`
reaches `GET /progression/:userId/:progressionId`, and the two any-path writes (B51a `PUT`, B51b `POST`) take every
write on the Gauntlet host before a later row can, while a write elsewhere walks on; literal rows registered before `:param` rows win
(`/progression/objectives/u` is `GET /progression/objectives/:userId`).

The full table is Appendix A. A new upstream route is added there in its registration position, never appended.

---

## 5. Shared state and how areas plug in

### 5.1 What every handler shares (one per module)

| Item | Contract |
| --- | --- |
| version | §5.2: hooks, tables, route set |
| settings | §6 |
| node | `module_api::NodeIdentity` from the module context (`NodeIdentity::standalone()` in a bare test module) |
| game data root | §8.1 |
| database wanted | whether `REFORGED_DB_FILE` or `REFORGED_DB_ENGINE` is set non-empty; the composer opens it (§11.4) |
| rifts | `rifts_seam::Rifts` (`Rifts::dedicated_only()` when none) |
| admin API | §6.3, read once at start |
| auth | the module's `rustic_isles_auth::Auth` (§10) |
| home | the home API (§5.4) |
| presence | §5.5 |
| dedicated servers | §5.6 |
| area state | typed slots the composer fills (the store, matchmaking state, anything else an area keeps per module); a handler asks for a type and gets it or none |

A bare module (tests): no database, standalone node, empty settings, no game data folders, dedicated-only rifts,
admin API off, an auth with no store, empty slots.

### 5.2 The version

- **Tables**: `target_changelist` (u32; 1.14.7: 647472), `game_version` (`1.14.7`), `active_huntpass_id`,
  `recent_huntpass_seasons`, `patchnotes_release_version`, `bounty_item_grant_data` (item id, amount, ISO start and
  end), `store_sections`, `seasonal_coin` (catalog id, max per claim, optional), `masked_journey_nodes` (node, the
  switch that unmasks it), `starter_manifest` (bootstrap version, stacked item ids, optional quick-emoji wheel),
  `switch_defaults` (§6.2). `build_id()` is `<changelist>_<game version>_shipping` (`647472_1.14.7_shipping`). The
  values are wire facts the version crate supplies (§16).
- **Hooks** (one trait object; the areas ask, the version answers): `no_player_policy() -> NoPlayerPolicy`;
  `game_session_payload(token) -> JSON`; `weapons_at_base_level(settings) -> bool`; `reforge_rules() -> bool`;
  `mastery_persistence() -> bool`; `huntpass_payload(id) -> JSON`; `objective_row(user id, row) -> JSON` (row:
  objective id, value, completed count, created and last modified dates); `progression_config() ->
  ProgressionConfigRule` (`AsIs` or `RollingHuntpassWindows`); `survey_config() -> StubReply` (status, optional
  JSON); `entitlements_payload(list) -> JSON` (entitlement: name, duration as a number, optional activated date);
  `candidate_destination(hunt id, realm hub) -> Destination` (game mode `ISLAND`, `CITY` or `SHARED`; hunt id or
  null).
- **Route set**: the version's share of the table; 1.14.7's is every row.

### 5.3 The no-player policy (`HasParadoxBackendAuth`'s chain; auth §4.1 uses it)

`Off`: nothing. `ConfiguredAccount`:

- **Path account** (the `INVALID` sentinel): the raw value of `GAMESERVER_NO_PLAYER_USER_ID`, else of `DEV_USER_ID`,
  else `mystpax`, then trimmed. A set-but-empty value is taken (it does not fall through).
- **Bearer-less account**: the same two settings, no fallback, trimmed; none when unset or blank.

| Settings | Path account | Bearer-less |
| --- | --- | --- |
| none | `mystpax` | none |
| `DEV_USER_ID= dev ` | `dev` | `dev` |
| `GAMESERVER_NO_PLAYER_USER_ID=` (empty), `DEV_USER_ID=dev` | `` (empty) | none |
| `Off` with `DEV_USER_ID=dev` | none | none |

### 5.4 Home

The home API the auth crate and the store use: the module context's (`resolve_home`, `session_ticket`,
`forward_home`, `verify_session_ticket`, `register_account_key_hash`, `register_directory_account` and the `ring_*`
calls), where a failure of the context call itself becomes `HomeError::Unreachable(<its text>)`. In a bare module,
`module_api::network::StandaloneHome` for the node.

### 5.5 Chat presence

The module keeps the set of user ids the chat plugin reports online (§7.7). Areas read it two ways: "is this user
online" (matchmaking) and "who is online" plus "close this user's connections with code and reason" (the Reforged
API; closing only forgets the user here and answers 1 if it was online, else 0). One set per module, never a process
global (§15 item 14).

### 5.6 Dedicated servers (the Director seam, from matchmaking's DeployServer client)

A replaceable per-module slot, default "unconfigured", which the Director's service fills when it runs in-process:

- `configured()`: whether a Director can be asked (`DEPLOYSERVER_URL` set and not blank, or one in-process).
- `launch(request)`: request is game mode (optional), game args (optional), hunt id, expected players (user ids),
  Gauntlet level (optional, the client's combined season and level), request id (optional, sent as
  `x-rustic-isles-request-id`); its body on the wire is
  `{"GameMode":..,"GameArgs":..,"HuntId":..,"ExpectedPlayers":[..],"GauntletLevel":..}` with absent options left
  out, in that order.
  Answers `Online { host, port }`, `Refused { reason }` (a reason only when nothing was asked), or an error text.
  Unconfigured answers `Refused` with no reason.
- `hub_status()`: Ramsgate's and the Training Dojo's hubs (ready, host, port) or none when unreachable.

### 5.7 How areas register and call each other

- **Handlers**: a registry keyed by (service, method, exact path text of the row). An area registers its routes'
  handlers; building a service's app fails `a handler for {service} {METHOD} {path}, which the route table doesn't
  have` when a key matches no row (a typo), and with the pattern error when a row's path fails to compile.
- **Calls between areas** go through the shared state: the auth crate's public functions (game auth, launcher auth,
  tokens), the hooks and tables, the presence, the dedicated servers slot and typed area slots. An area crate never
  reaches into another's private state; anything two areas share is a slot type defined by the lower one (§16).

---

## 6. Settings

### 6.1 Layers

A module's settings are three maps, read top first: **forced** (set by the module's own config, §6.4), the **env
file**, the **switches** section. The process environment is never read (a compiled-in module shares the
controller's).

- `raw(name)`: the value as set, empty included (Node's `process.env.X`).
- `get(name)`: `raw`, but none when empty.
- `on_unless_off(name)`: false only when the raw value (not trimmed) is `0`, `false`, `off` or `no` (ASCII
  case-insensitive); unset is on.
- `vars()`: every setting as one map (switches, then the env file over them, then forced): what the Director's game
  servers inherit.
- An **overlay** of another settings value puts its three maps over these (tests).
- The settings answer `rifts_seam::Settings::get` with `raw`, and the auth crate's `AuthSettings::raw` with `raw`.
- `Debug` of the settings lists names only, never values (§14). Restricted's derived `Debug` prints every value,
  secrets included; this is a deliberate improvement, keep it.

### 6.2 Switches (`[switches]`)

The module registers a `switches` config section. The version's switches, each with its default from the version's
`switch_defaults` (empty text when it has none) and its help:

| Key | Setting | Help | 1.14.7 default |
| --- | --- | --- | --- |
| `qos_port` | `QOS_PORT` | The region-ping listener's port, or off | `80` |
| `reforge` | `RUSTIC_ISLES_REFORGE` | Weapon reforge rules: on, log or off | `on` |
| `mastery_persistence` | `RUSTIC_ISLES_MASTERY_PERSISTENCE` | Whether a game server's mastery report is saved: on or off | `on` |
| `bounty_persistence` | `RUSTIC_ISLES_BOUNTY_PERSISTENCE` | Whether bounty saves are stored: on or off | `on` |
| `cooldown_persistence` | `RUSTIC_ISLES_COOLDOWN_PERSISTENCE` | Whether cooldown saves are stored: on or off | `on` |

Optional keys, default JSON `null`, help `Optional host switch ({SETTING})`: `account_erasure`
(`RUSTIC_ISLES_ACCOUNT_ERASURE`), `log_purge` (`RUSTIC_ISLES_LOG_PURGE`), `rate_limit` (`RUSTIC_ISLES_RATE_LIMIT`),
`rate_limit_per_minute`, `gameserver_rate_limit_per_minute`, `host_rate_limit_per_minute` (the
`RUSTIC_ISLES_*_PER_MINUTE` names).

Reading the section: each known key present maps to its setting name; a string is taken as it is, `null` is
skipped, any other JSON value is its JSON text (`80` is `80`, `true` is `true`). Unknown keys are ignored.

### 6.3 Readers the crate owns

| Setting | Reading |
| --- | --- |
| `RUSTIC_ISLES_ADMIN_API` | trimmed, lower-cased: `on` true, `off` false, empty: follow `RUSTIC_ISLES_MANAGER_ADMIN` (trimmed, lower-cased: empty or `off` false, `on` true, else the error `RUSTIC_ISLES_MANAGER_ADMIN: must be on or off (blank means off)`); anything else the error `RUSTIC_ISLES_ADMIN_API: must be on or off (blank follows RUSTIC_ISLES_MANAGER_ADMIN)`. An error fails the module's start |
| `REFORGED_DB_FILE`, `REFORGED_DB_ENGINE` | §5.1 "database wanted" (`get`) |
| `RUSTIC_ISLES_RATE_LIMIT`, `_PER_MINUTE`, `RUSTIC_ISLES_GAMESERVER_RATE_LIMIT_PER_MINUTE`, `RUSTIC_ISLES_HOST_RATE_LIMIT_PER_MINUTE` | auth §10.1 (on unless `0`/`off`/`false`/`no` trimmed any case; per-minute: an optional `+` then the leading digits, under 1 or unreadable is the default 600 / 6000 / 6000) |
| `PORT_RANGE_BEGIN`, `PORT_RANGE_END` | §6.4 |
| `GAMESERVER_NO_PLAYER_USER_ID`, `DEV_USER_ID` | §5.3 |

Every other setting is an area's.

### 6.4 The env file and forced values

The `backend` section's `env_file` (strings as they are, `null` none, other JSON its text), trimmed; blank means no
file and an empty env map. Otherwise the env map is, in increasing priority: the sibling `admin.env`, then the
sibling `director.env` (each only when the env file's name is exactly `backend.env`, it has a parent folder, and the
sibling is a file), then the file itself. Files are dotenv (`dotenvy`'s format, as `service_kit::env`). A file that
cannot be read or parsed fails the start with `{path}: {message}`.

When the module's config assigns the `game` port range, `PORT_RANGE_BEGIN` and `PORT_RANGE_END` are forced to its
first and last port unless the env map has both **and** the range is a default window (8770-8777 or 8780-8787); in
that case the env file's (possibly wider) range stays.

| Env file has | Assigned game range | `PORT_RANGE_BEGIN`-`END` |
| --- | --- | --- |
| 8766-8777 | 8770-8777 | 8766-8777 |
| 8766-8777 | 8778-8785 | 8778-8785 |
| nothing | 8770-8777 | 8770-8777 |

### 6.5 Legacy names (rename of 2026-11-01)

Restricted reads `MYSTICPARADOX_<rest>` as `RUSTIC_ISLES_<rest>` when the new name is not in the env map (the old
stays; one warning per name per process: `{old} is deprecated and will stop working in the next release: rename it
to {new}`, never the value), and copies `x-mysticparadox-*` headers (§3.1). They were to be removed "with the release
after". Proposed: the MIT crate carries neither (as auth §15 item 9); restricted maps the env map before handing it
over if a release still needs it (§15 item 2).

---

## 7. The module (`module_api::GameModule`)

A version crate fills a **module spec** (§11.2) and gets a `GameModule` it runs compiled in, as a child executable
(`module_api::child::main`) or as a plugin library (`module_api::export_plugin!`).

### 7.1 Manifest

`id` and `game_version` from the spec (both must parse; the version crate's test checks it), `kind` Game, the
display name, the invite aliases, `api` `API_VERSION`, ports in this order: `backend` TCP, `director` TCP, `qos` TCP
(each a single port from the spec's defaults) and `game` UDP (the spec's range), the capabilities, `unverified`, and
no client profile. 1.14.7: id `isles-1-14-7`, `Rustic Isles 1.14.7`, alias `reforged`, ports 61000, 61001, 61010,
UDP 8770-8777, capabilities realms (with the feature), network, chat, patch set `Accepts`.

### 7.2 Registration

- `route("/", "backend")`, and `host_only_route(prefix, "backend")` for each of `/reforged/api/CreateInvite`,
  `/reforged/api/InviteCodes`, `/reforged/api/InviteCode`, `/reforged/api/SetAccountStatus`, `/reforged/api/Kick`.
- `udp_ports("game")`.
- Admin actions `Kick`, `Ban`, `Unban`, `LiftTimeout`, `Lookup`.
- Emits `isles.listening` and `isles.party.headline` (`module_api::chat::HEADLINE_EVENT`), then
  `game.server.up`, `game.server.down`, `game.availability.changed` (§7.9); subscribes `chat.presence`
  (`module_api::chat::PRESENCE_EVENT`).
- Game calls `Availability`, `Servers`, `CreateInvite`, `RevokeInvite` (§7.9).
- Config sections, in this order: `backend` with `env_file` = `""` (help `The env file the backend and Director read,
  under Node's variable names`); `switches` (§6.2, the version's then the optional keys); `game_data` with
  `backend_dir` = `""` (`The backend's game data exports for this version`) and `director_dir` = `""` (`The
  Director's hunt tables for this version`).

### 7.3 Start

Phase `Ready` or `Starting` already: `Invalid` `already started`. Otherwise phase `Starting`, then:

1. Settings (§6.4) and the admin API (§6.3).
2. The game data root from `game_data.backend_dir` / `director_dir` (§8.1).
3. The composer composes (§11.4) with the settings, node, module id, context, game data root and "database
   wanted": it opens the database when wanted (off the async threads), and returns the area state, the auth store
   and seams, the rifts, the handlers, the hooks of §7.5-§7.8 and the area start-ups.
4. The shared state (§5.1) with the auth built over the composer's store (auth §11.2 `Auth::new`).
5. The area start-ups run in the composer's order (each may log, fail, return background tasks or listeners); then
   the auth's own `start` (auth §8.3), whose failure text fails the start; only with a database.
6. For each port service in spec order: its port is the config's assigned range's first port, else the spec default
   (`backend`, `director`), else the start fails `no {name} port`; its app is built (failure fails the start); it
   binds `127.0.0.1:<port>` (failure: `{name} on 127.0.0.1:{port}: {error}`); its local address is recorded under
   its name; it serves with a 2 s drain on stop (`service_kit::serve::serve`).
7. Listeners the area start-ups returned (the QoS pong: `qos` and `qos_public_port`) are recorded after the services.
8. Phase `Ready`, detail `{name} {address}, ...` over every recorded address in order; each service's `started()`;
   context log Info `{display name} listening: {detail}`; event `isles.listening` with the JSON object of the recorded
   addresses as text, in order (`{"backend":"127.0.0.1:61000","director":"127.0.0.1:61001"}` plus the QoS keys when
   it started).

Any failure: phase `Failed`, detail the error's message, the error returned (`ModuleError::failed(text)`).

### 7.4 Stop and status

Stop: each service's `stopping()` (the Director lets go of its game servers); phase `Stopping`; the running state and
chat answerer are dropped; background tasks aborted; each listener told to stop and awaited up to 5 s (warning `a
listener did not stop within 5 s`); the composer's stop (the store's checkpoint; a failure is a warning
`checkpoint: {error}`); phase `Stopped`, no detail; context log Info `{display name} stopped`. Status: the phase
(`Loaded` before any start) and the detail.

### 7.5 Chat

Not running: `Unavailable` `the module is not running`. Otherwise the composer's chat answerer for the query (Rustic
Isles' chat rules, §16).

### 7.6 Admin actions

No database: `Unavailable` `the module has no database`. `Ban` with an `until`: before anything else, `until` must
parse as RFC 3339 (else `Invalid` `a timeout ends at an RFC 3339 time`); it is rewritten as the ISO text of its
instant (`toISOString`, milliseconds) and must sort after now's ISO text (else `Invalid` `a timeout ends in the
future`). Then the composer's admin hook answers, which must keep this contract (restricted's today, an area crate's
later):

- **Player lookup**: the given text trimmed (empty: `NotFound`); as an account global id, the game account with that
  id (its userId and name), else the launcher account with that id (the id text as userId, the display name); else
  the game account with that userId; else the launcher account whose normalized name is the text lower-cased (the
  userId from its game account, else the id text). None: `NotFound`.
- `Kick`: unused exchange codes revoked (sessions kept), chat connections closed with 1008 `kicked by the host`,
  the Reforged status cache forgotten; `Done`.
- `Ban` with `until`: timeout set (until, since now, reason cut to 500 characters), then as kick with `timed out by
  the host`; `Done`.
- `Ban` without: status `banned` (updated now, by `host`, reason cut to 500), any timeout cleared, every refresh
  session and unused exchange code revoked, chat closed with `banned by the host`; `Done`.
- `Unban`: status `active` with the reason; `LiftTimeout`: timeout cleared; both `Done`.
- `Lookup`: `Player { account, name, online, restriction }`: online when the userId is in the chat presence or was
  active in the last 3 minutes; restriction: banned or disabled first (`Ban`/`Disabled`, reason the stored reason if
  not empty, since its update time, no until), else a timeout whose until sorts after now (`Timeout`, reason, since,
  until), else none.
- Every write is the call's actor, as an account write on the account's home.

### 7.7 Events

`chat.presence` (other events ignored; a payload that does not parse is ignored): online adds the userId to the
presence (§5.5); offline removes it and then calls the composer's "offline" hook (today: the player leaves their
party; errors ignored). Always `Ok`.

### 7.8 Commands

Not running: `Unavailable` `the module is not running`. A **Director invocation** (`module_api::network::
DirectorInvocation::from_command`; a parse error is returned) and a **home invocation**
(`HomeInvocation::from_command`) answer `CommandOutput { text: "ok", data }`; anything else
`Unsupported` `command {name}`.

- **Director invocation**: dedicated servers not configured: `Unavailable` `dedicated servers are not configured on
  this node`. `PlaceHunt { match_id, queue, players }`: each player that is an account homed on this node and found
  in the store becomes its userId, any other stays its global id text; launch `ISLAND` with empty game args, hunt id
  = queue, those players; `Online`: the instance is recorded for the players (matchmaking's ring map) and the answer
  is `{"server_host":..,"server_port":..}` (a port outside u16 becomes 7777); `Refused`: `failed` with the reason or
  `director refused hunt launch`. `PlaceHub { hub_id }`: a ready live Ramsgate hub answers at once (recorded with no
  players); else launch `CITY` with no game args, hunt `ShatteredIsles_ReturnToRamsgate`, no players; the same
  answers, `director refused hub launch`.
- **Home invocation**: no database: `Unavailable` `the module has no database`; the account owned by another node:
  `Invalid` `account {account} is homed on {owner}, not {node}`; not an account id: `Invalid` with the id error.
  Then the composer's home operation for its name, with actor `Player { account }`; an unknown name: `NotFound`
  `unknown home operation {name}`. The operations today (DN5's wire, kept by whoever implements them):

| Operation | Body | Answer |
| --- | --- | --- |
| `progression.grant` | `xp` (integer, negatives 0, default 0), `track` (default `ExperienceTrack_PlayerLevel`) | `{"ok":true,"account":..,"progression_id":..,"progress":..}` |
| `progression.read` | — | `{"tracks":[{"progression_id","progress","update_version"}...]}` |
| `inventory.write` | `loot`: strings `catalogId` or `catalogId:qty` (qty parsed, at least 1, default 1) | `{"ok":true,"account":..,"loot_applied":<count of loot entries>}`; adds to the first character's stacked items (creating a character `char-<account>` named `Slayer` and its inventory when there are none) |
| `inventory.read` | — | `{"character_id","stacked_items","instanced_items"}`, or both lists empty without a character |
| `bounty.write` | `bounty_progress` (default `{}`) | appends it as a base-group bounty; `{"ok":true,"account":..,"update_version":<new>}` |
| `bounty.read` | — | `{"count":n,"bounties":[...]}` |
| `character.load`, `account.write` | — | `{"ok":true,"account":..,"operation":<name>}` |
| `chat.login` | `authcid`, `token` | the chat rules' login check; `{"account","user_id","name"}`; a refusal is `Invalid` with its code text; another account's token `Invalid` `token account does not match target account` |
| `chat.player` | — | `{"account","user_id","name"}` or `{"player":null}` |
| `chat.party` | — | `{"party_id":<id or null>}` |

---

### 7.9 Game calls (MS3, added 2026-11-08)

`module_api::game` (the module-sharing plan's A1 and A3). One **ask** of the dedicated-servers slot (§5.6): its
hub status and then, when that answered, its server list, each given 5 s (a slower answer counts as none). A slot
that can't list servers (over `DEPLOYSERVER_URL`) gives the ready hubs from the hub status instead: id
`city-{port}` (Ramsgate, kind `hub`, title `ramsgate`) or `dojo-{port}` (Training, kind `dojo`, title
`training_dojo`), empty map, its host and port, 0 players, 0 max, no start time.

**Availability** from the ask, in this order: no hub status → `director_unreachable`; Ramsgate listed and ready →
`up`, its hub the listed `hub`-kind server, or one built as above; Ramsgate listed, not ready →
`hub_not_ready`; Ramsgate absent and `RAMSGATE_HUB` off (director spec §11's switch) or the slot not configured
(cannot launch) → `no_hub`; absent otherwise → `hub_not_ready` (the hub is starting or retrying with backoff).
`free_hunts` is left out. Ramsgate is persistent: the Director starts it at boot and keeps it up (director spec
§7), never on a player's first request, so "the Director is ready to start one" is never "up".

- `Availability`: not running → `down(module_not_ready)`, never an error.
- Every other call, not running: `Unavailable` with code `module_not_ready` (`UnavailableReason::error`).
- `Servers`: the ask's list; no hub status → `Unavailable` `director_unreachable`.
- `CreateInvite { uses, expires_in_secs, note, require_up }`: `uses` outside 1-1000 → `Invalid` `uses is from 1
  to 1000`; `expires_in_secs` 0 → `Invalid` `expires_in_secs is at least 1`; no database → §7.6's `no database`;
  `require_up` and the ask (made now, not reused) not `up` → `Unavailable` with its reason's code. Then the
  composer's invite hook: CreateInvite's write (reforged-admin spec §9) with `infinite_uses` false, the note
  cleaned as CreateInvite cleans `name`, and `expires_at` = now + the seconds as `toISOString` text (none when
  absent). `Invite { code }`. Five taken codes: `failed`.
- `RevokeInvite { code }`: no database → `no database`; the composer's hook deletes it as `DELETE
  /reforged/api/InviteCode/:code` does (a code that is not an invite code deletes nothing); deleted → `Done`, else
  `NotFound` `no such invite`.

**The watch**: started with the module (a task stopped at stop), it asks every 10 s, the first at once. Against
what it last told the bus: each server id no longer listed → `game.server.down` with its last `GameServer`; each
new id → `game.server.up` with it; then, when `(up, reason)` moved (or on the first pass),
`game.availability.changed` with the `Availability`. An ask without a list changes no server (a Director blip
downs nothing); player counts changing are no event. Invite codes are never in an event.

## 8. Game data

Nothing generated from the game is committed (AGENTS.md). The files are exported from the host's install
(CatalogExporter) and written by the generators (`reforged-host gamedata`, §16).

### 8.1 The root

One root per module: an optional backend folder and an optional Director folder, from the `game_data` section
(trimmed; blank is none). A file path is the folder joined with the name; no folder means no path.

| Backend file | Read by (area) | Made from |
| --- | --- | --- |
| `progression_config.json` | game-data (`GET /progression/config`), progression, inventory, bounties-store | a progression-config response the host supplies (`payload.paths` an array) |
| `slayers_path.json` | progression | CatalogExporter `EXPORT_SLAYERS_PATH` |
| `slayers_path_definitions.json` | progression | the same |
| `ladyluck_store.json` | bounties-store | a store response the host supplies (array of entries: `id`, `prices`, `items`, ...) |
| `reward_cache_store.json` | bounties-store | the catalog |
| `platinum_store.json` | bounties-store | the catalog and store images |
| `inventory_storage_policy.json` | inventory, auth (tester kit), reforged-api | the catalog |
| `mastery_objectives.json` | progression | |
| `escalation_tuning.json` | progression | |
| `fountain_daily_bundle.json` | bounties-store | |
| `tester_max_kit.json` | reforged-api | |

Director files: `player_hunts_table.json`, `matchmaker_hunts_table.json`, `arena_easy_matchmaker_hunts.json`,
`arena_hard_matchmaker_hunts.json`, `arena_elite_matchmaker_hunts.json`, and for a version with Trials tables (1.14.7)
`arena_hard_matchmaker_hunts_new.json`, `arena_elite_matchmaker_hunts_new.json`, in that order. The shape of each file
is its reader's contract (each area's spec); this crate treats them as JSON values.

### 8.2 The loader (`loadGameData`, `tryLoadGameData`)

- A file is read as bytes, lossy UTF-8, one leading BOM dropped, and parsed as `JSON.parse` does
  (`service_kit::jsjson`: `1.0` becomes `1`, `12345678901234567890` becomes `12345678901234567000` when written back).
- **Optional read**: none when the file does not exist (or the root has no folder); a file that exists but cannot be
  read or parsed is an error `{path}: {message}`.
- **Required read**: as optional, but a missing file (or folder) is the error
  `[game-data] Required game data file is missing: {path}` + newline + `This project does not ship game data.
  Generate it from your own installation (see docs/GENERATING_GAME_DATA.md), or copy the matching *.example.json for
  a smoke test.`, where `{path}` is `<no backend game data folder>/{name}` without a folder.
- Each read reads the file again, except the **cached required read**: the first successful read of a path is kept
  for the life of the process and returned after (a changed file needs a restart; `GET /progression/config`'s file,
  as Node loads it at import).

### 8.3 Version-specific Director files

The Director asks the root for its tables by name; the list is the version's (§8.1).

### 8.4 The Hunt Pass season window (`seasonWindow.ts`)

Shared by game data, progression, bounties and cooldowns.

- Anchor 1 727 370 000 000 ms (`2024-09-26T17:00:00.000Z`, a Thursday); length 84 days (7 257 600 000 ms).
- The window containing `t` is `[start, start + length)`, `start = anchor + floor((t - anchor) / length) * length`
  (floor division, so before the anchor the cycles continue backwards).
- As text: ISO of each end with `.000Z` written `+00:00` (`IsoUtc`); a time with milliseconds keeps `Z`.

| `t` | Window |
| --- | --- |
| `2026-10-22T16:59:59.999Z` | `2026-07-30T17:00:00+00:00` to `2026-10-22T17:00:00+00:00` |
| `2026-10-22T17:00:00.000Z` | `2026-10-22T17:00:00+00:00` to 84 days later |
| `2026-10-13T18:00:00.000Z` | `2026-07-30T17:00:00+00:00` to `2026-10-22T17:00:00+00:00` |
| anchor − 1 ms | anchor − 84 days to the anchor |

ISO text (`toISOString`, years 0000-9999): 0 is `1970-01-01T00:00:00.000Z`, −1 is `1969-12-31T23:59:59.999Z`,
`2024-02-29T23:59:59.999Z` round-trips; `IsoUtc` of `2026-10-22T17:00:00.250Z` stays as it is.

---

## 9. Replay, clock and ids

### 9.1 The replay scope

A request may carry `x-isles-replay-now-ms` (an integer of ms, or any text `Date.parse` reads; trimmed) and
`x-isles-replay-seed` (trimmed; empty is none). Either makes the request's **replay scope** active. The scope applies
to everything the request runs, the database work it hands to blocking threads included (the store runs its closures
inside the scope).

- **Clock** (`now`): the scope's time when it has one; else a test pin (tests only, process-wide); else the system
  time in ms.
- **Replay time only**: the scope's time, ignoring a test pin (so a pinned test does not backdate live token
  expiries).
- **Ids** (`next uuid`): with a seed, deterministic: the seed key is SHA-256 of the bytes `isles-replay-seed:`
  followed by the seed text; the n-th id of the request (n from 0) is the first 16 bytes of SHA-256(seed key ‖ n as
  8 bytes little-endian), with byte 6's high nibble set to 4 and byte 8's top two bits to `10` (a valid v4). Without
  a seed, a random v4.
- **Pinned?**: whether a scope time or a test pin is in force.

Areas take time and ids only from these (the auth crate's `Clock` and `IdSource` are them, auth §11.2).

### 9.2 Who uses them

New userIds, character ids, item instance ids, grant ids, party ids, hub game session ids (matchmaking), the season
window's `now`, and the request budget's clock (§3.4). The request id fallback (§3.1) is not seeded.

### 9.3 The shadow replay harness (summary; §16's replay unit)

The harness reads the gateway's capture `.jsonl` (`kind: "http"` records: `t`, `src`, `conn`, `seq`, `ip`, `method`,
`target`, `route`, request and response headers and bodies (`null`, `{"json"}`, `{"form":[[k,v]...]}`, `{"text"}`,
`{"binary":n}`, `{"omitted":n}`), `status`, `ms`, `aborted`; `kind: "chat"` records), sorts a corpus by `t`,
substitutes placeholders (`<token:...>`, `<token>`, `<key:sha256>`, `<gskey>`, `<plain:...>`), sends each request with
the two headers above set from the record's time and `<conn>:<seq>:<method>:<target>`, and classifies differences
(status, header, value, text, chat stanza) after normalizing minted tokens, volatile headers, unpinned times and
first-seen dynamic ids. It is a host tool, not the module; it needs only this section's headers from the module.

---

## 10. Hosting `rustic-isles-auth`

What auth §11.3 asks of isles, as this crate does it:

- One `Auth` per module in the shared state, built at start (§7.3) with `AuthConfig { target_changelist,
  game_version (parsed from the tables), node, no_player (§5.3, from the hooks), session_payload (the hook),
  engine_is_postgres (from the composer), clock (§9.1), ids (§9.1) }`, the settings (§6.1) as `AuthSettings`, the
  composer's store (none without a database) and its `Seams` (the home of §5.4 included).
- Every `rustic_isles_auth::routes()` entry is registered as a backend handler calling `Auth::handle` with the call's
  request and params; their (method, path) pairs are exactly the table's auth rows (a mismatch fails the build).
- The pipeline calls `launcher_cors` (§3.5), `client_ip` / `rate_limit_key` and the limiter's
  `check_request_budget_at` (§3.4), and, for rifts rows, `rifts_caller_kind` with launcher auth and the active-account
  check to make the rifts caller (§3.7).
- Areas get `game_auth` (it rewrites the `INVALID` sentinel in the URI and stores `AuthData` in the request's
  extensions), `launcher_auth`, `require_active_account`, the token functions and `status_for` through the shared
  state.
- The auth's `start` runs at §7.3 step 5; its background tasks are aborted at stop.
- `Auth<S>` is generic over its store; the crate holds `Auth<Arc<dyn AuthStore>>`, so it needs `AuthStore`
  implemented for `Arc<T>` where `T: AuthStore + ?Sized` (a request to the auth implementer; §15 item 9).

---

## 11. The public seam

### 11.1 Shape

One crate, `rustic-isles` (lib), MIT, in dainty-lass `crates/rustic-isles`. It depends only on dainty-lass MIT crates
(`module-api`, `service-kit`, `rustic-rifts-seam`, `rustic-isles-auth`) and workspace dependencies (`axum`, `http`,
`tokio`, `tracing`, `serde`, `serde_json` with `preserve_order`, `sha2`, `uuid`, `regex`, `percent-encoding`,
`dotenvy`, `chrono`, `thiserror`). It does not depend on `store`: the database is the composer's.

**Seam changes for the integration pass**: `serde_json`'s `preserve_order` in the workspace (shared with auth);
`crates/rustic-isles` and `rustic-isles` in `[workspace.dependencies]`. `service_kit::routing::Pattern` refuses empty
segments and regex rows, so the crate has its own matcher (§4.1); moving it into `service-kit` is optional and not
this row's.

### 11.2 Interface (names are the contract the restricted composer is written against)

| Item | Contract |
| --- | --- |
| `pub enum Service { Backend, Director }`, `pub enum Method`, `pub enum Group`, `pub enum Side`, `pub enum When { Always, Admin, Gauntlet, Rifts }`, `pub enum Scope { Version, Node }` | §4.2; each has `as_str()` (`backend`, `GET`, `reforged-api`, `isles`, ...); `Method::accepts(&http::Method)` |
| `pub struct RouteDef { service, method, path: &'static str, group, side, when }` | `scope()`, `is_regex()`, `name()`; `pub fn routes::all() -> &'static [RouteDef]` (Appendix A) |
| `pub enum RouteSet { V1147 }` | `routes(service)` |
| `pub struct Pattern`, `pub struct Params`, `pub struct BadParam`, `PatternError` | `Pattern::parse(&str) -> Result<Pattern, PatternError>`, `matches(path) -> Option<Result<Params, BadParam>>`, `Params::get(name)` (§4.1) |
| `pub struct Settings` | §6: `new(env, switches)`, `from_pairs`, `forced`, `overlay`, `switches_from_config`, `read_env_file`, `vars`, `env_map`, `raw`, `get`, `on_unless_off`, `rate_limit_enabled`, `rate_limit_per_minute`, `gameserver_rate_limit_per_minute`, `host_rate_limit_per_minute`, `admin_api() -> Result<bool, SettingsError>`; `SharedSwitch { env, key, help }`, `SWITCHES`, `OPTIONAL_SWITCHES`; implements `rifts_seam::Settings` and `rustic_isles_auth::AuthSettings` |
| `pub struct VersionTables` and its parts, `pub trait VersionHooks`, `NoPlayerPolicy` (`path_account`, `bearerless_account`), `StubReply`, `ObjectiveRow`, `Entitlement`, `GameMode`, `Destination`, `ProgressionConfigRule`, `pub struct IslesVersion { hooks, tables, routes }` | §5.2, §5.3 |
| `pub struct AccountId`, `pub struct UserId` | §2.2 (`AccountId::new(node)`, `global()`, `home()`, `FromStr`, `TryFrom<GlobalId>`; `UserId::random()`) |
| `pub struct GameDataRoot { backend, director }`, `BACKEND_FILES`, `DIRECTOR_FILES`, `DIRECTOR_TRIALS_FILES`, `director_files(trials)` | §8.1; `from_config`, `backend_file`, `director_file`, `load_backend`, `try_load_backend`, `load_backend_cached`; `LoadError { Missing, Unreadable }` (§8.2) |
| `pub mod season` | §8.4: `ANCHOR_MS`, `LENGTH_MS`, `active_season_window_ms`, `active_season_window`, `to_iso_string`, `iso_utc` |
| `pub mod replay` | §9.1: `REPLAY_NOW_HEADER`, `REPLAY_SEED_HEADER`, `ReplayScope { new(now, seed), is_active, enter(f) }`, `with_replay_scope(scope, future)`, `current_replay_scope()`, `now_ms()`, `replay_now_ms()`, `is_replay_pinned()`, `next_uuid()`, `pin_now_for_tests(Option<i64>)` (hidden) |
| `pub struct Shared` | §5.1: `version`, `settings`, `node`, `game_data`, `database_wanted`, `rifts`, `admin_api`, `auth() -> &Arc<Auth<..>>`, `home() -> Arc<dyn HomeApi>`, `presence() -> &Presence`, `dedicated_servers()`/`set_dedicated_servers(..)`, `slot::<T>() -> Option<Arc<T>>`; `Shared::bare(version)` |
| `pub struct Presence` | §5.5: `set_online`, `set_offline`, `is_online`, `online_user_ids`, `close_connections(user, code, reason) -> usize` |
| `pub trait DedicatedServers`, `LaunchRequest` (`body()`), `Launch`, `LiveHub`, `LiveHubStatus`, `Unconfigured` | §5.6 |
| `pub struct Call { shared, route, params, request, origin_host, request_id }`, `pub type Handler`, `pub struct Handlers` | §3.6, §5.7: `on(service, method, path, handler)`, `get(route)`, `check() -> Result<(), MountError>`; `MountError { Pattern, UnknownRoute, NoRiftsHandler }` |
| `pub fn app(shared: Arc<Shared>, service: Service, handlers: &Handlers) -> Result<axum::Router, MountError>` | §3; the router's fallback is the dispatcher |
| `pub fn not_implemented(route, request_id) -> Response`, `REQUEST_ID_HEADER`, `BODY_LIMIT` | §3.6, §3.1, §3.9 |
| `pub async fn read_body(&mut Request)`, `read_body_parts(parts, body)`, `read_body_parts_limit(parts, body, limit)` | §3.9 |
| `pub trait Observer` | `begin(&Request, request_id) -> Box<dyn Observation>`; `Observation::finish(status)`; default does nothing (the Reforged API's request log and metrics plug in) |
| `pub trait PortService` | `port() -> &'static str`, `app(Arc<Shared>) -> Result<axum::Router, ModuleError>`, `started()`, `stopping() -> BoxFuture<()>`; `BackendService` (the backend over the composer's handlers) |
| `pub struct DefaultPorts { backend, director, qos, game }`, `pub struct ModuleSpec { id, display_name, invite_aliases, capabilities, unverified, ports, version, services, composer }` | §7.1 |
| `pub struct IslesModule` | `new(spec)`, `spec()`, `settings(&ModuleConfig) -> Result<Settings, ModuleError>` (§6.4); implements `GameModule` (§7); `LISTENING_EVENT`, `HOST_ONLY_PREFIXES` |
| `pub mod testing` (feature `test-support`) | §13.3: `sample_path`, `request`, `listening`, `Mounts`, `sweep`, `drive`, `GAUNTLET_ORIGIN` |

### 11.3 Logs

`Unstubbed route {METHOD} {path}` (warn), `rifts route {name}: {error}` (error), `a listener did not stop within 5 s`
(warn), `checkpoint: {error}` (warn), the context logs of §7.3 and §7.4. English, never a setting's value.

### 11.4 The composer (what restricted implements until the areas are ported)

`pub trait Composer: Send + Sync + 'static` with:

- `compose(&self, inputs: ComposeInputs) -> BoxFuture<Result<Composition, ModuleError>>`; inputs: settings, node,
  module id, context, game data root, database wanted.
- `Composition`: `slots` (typed area state), `auth_store: Option<Arc<dyn rustic_isles_auth::AuthStore>>`,
  `auth_seams: rustic_isles_auth::Seams`, `engine_is_postgres`, `rifts`, `handlers` (backend; the auth routes are added
  by this crate), `observer`, `hooks` (a `ModuleHooks` object: `chat`, `admin`, `home_operation`, `place_hunt_players`
  and ring recording for §7.8, `player_offline`, `stop`, each with a default that answers as "no database" or does
  nothing), and `startups` (ordered; each `start(&Arc<Shared>) -> BoxFuture<Result<Started, String>>`, `Started`
  being background tasks, stoppable listeners with their recorded addresses, and log lines).

A bare composer (no database, no areas) gives a module whose every non-auth backend route answers 501 (§13.2).

---

## 12. What stays in dainty-restricted

- The areas and repositories (§1.3), as handlers and slots the composer registers, until each unit lands (§16).
- `IslesStore` (the database on `store`, the soft-delete table list, the actor mapping, home-checked account writes)
  and the migrations, with the `AuthStore` adapter over `repo/auth.rs` (auth §15 item 14).
- The QoS pong, the party outbox dispatcher, the multipliers start-up line, the hunt-activity and chat-presence
  wiring to matchmaking, the `AuthReforgedCore` and matchmaking `IslesCore` for rifts, and rifts' composition (the
  version crate's `rifts_with_core`).
- The admin-action and home-operation bodies (§7.6, §7.8) behind `ModuleHooks`.
- The game-data payload modules, generators, golden harness and `bootstrap.rs`.
- The legacy aliases, if a release still wants them (§6.5).

---

## 13. Test cases

### 13.1 What may be copied

Nothing recorded: `tests/golden/*` (including `request_budget.json`) stays in restricted and is replayed there against
this crate once composed; a failure there is a bug on this side. `tests/pipeline.rs`, `tests/inventory.rs`,
`testing.rs` and every unit test of the files in §1.2 are authored test code, described below only. Appendix A is
the route table itself (wire facts), and the crate's own test checks its counts.

### 13.2 Behaviour the authored tests fix

| Area | Cases |
| --- | --- |
| Table | 306 rows; 301 backend; `RouteSet::V1147.routes(Backend)` 301; 33 `admin` rows, all `/admin/...` or `/health/release-gates`; 7 `gauntlet` including `/config`; every isles-side row not `rifts`; rifts-side rows `rifts` except the 8 Guard rows of §4.2; every path compiles |
| Matching | §4.1's table |
| 501 | a bare module (no handlers): `GET /character` → 501, `content-type: application/json; charset=utf-8`, body `{"error":{"code":"NOT_IMPLEMENTED","message":"isles: GET /character is not implemented yet","requestId":"<the x-rustic-isles-request-id header>"}}`; `HEAD /CHARACTER/` → 501, empty body; `GET /nothing/here` → empty 404 |
| Order | `GET /progression/a/b` → message names `GET /progression/:userId/:progressionId`; the same under `/__origin/gauntlet-prod.steelyard.ca` → names `GET /^\/progression\/([^/]*)\/([^/]+)$/i`; `GET /progression/objectives/u` → `GET /progression/objectives/:userId`; `POST /inventory//dauntlessrel-1.14.7:647472` → `POST /^\/inventory\/([^/]*)\/(.+)$/`; `GET /__origin/evil.example/character` → 400 |
| Admin gate | off: `GET /admin/v1/me`, `GET /Admin`, `GET /health/release-gates` → 404; on: the first two → 501 |
| Handlers | a handler for `GET /` answering `200 ok` replaces the 501; a handler for `GET /charactr` fails the build with `UnknownRoute` |
| Origin strip | `/__origin/Gauntlet-Prod.steelyard.ca/config?x=1` → host `gauntlet-prod.steelyard.ca`, target `/config?x=1`; `/__origin/api.epicgames.dev` → `/`; refused: `evil.example/x`, `steelyard.ca.evil/x`, `a%ZZ.steelyard.ca/x`, `x:1@steelyard.ca/x`; `/character` untouched |
| Request id | `abcd-1234` kept; `short` replaced; `-abcdefgh` replaced by a 36-character UUID |
| Budget buckets | limits client 2, game server 3, host 2, replay time pinned, `x-forwarded-for: 198.51.100.10` from loopback: client 200, 200, 429; with `x-rustic-isles-gameserver-apikey: unverified-key` (own bucket): 200, then `GET /accountinfo` 401 with `ratelimit: "3-in-1min"; r=1; t=60`, 200, 429; a capability the guard maps to `host:cap-123`: 200, 200, 429; another capability its own bucket 200; an invalid capability, a guard error or `   `: the exhausted client bucket, 429 with `"2-in-1min"; r=0; t=60`; an empty game server key header still selects the game server bucket |
| Budget switch | `0`, `off`, `OFF`, ` false `, `No` with limit 1: four 200s, no `ratelimit`/`ratelimit-policy`; limits `0`, `-10`, `not-a-number` → `"600-in-1min"; r=599; t=60`, `"6000-in-1min"; r=5999; t=60` (game server), the same for a host |
| Budget placement | limit 2: `/`, `/health/live`, `/health/ready`, `/health/release-gates` (admin on) five times each: 200, no budget header; `GET /mystic/v1/hunts/s1/status` 200 `r=1`; `GET /mystic/v1/hunts/unknown-tail` 404 `r=0` (counted once); the next hunts request 429; the probes still 200 without headers |
| Client address | limit 1, loopback peer: `2001:db8:abcd:ef12:1:2:3:4` 200, `2001:db8:abcd:ef99:9:8:7:6` 429 (same /56), `2001:db8:abcd:ee00::1` 200; peer `198.51.100.77`: XFF `203.0.113.1` 200, `203.0.113.2` 429 (XFF ignored) |
| Settings | the env file wins over a switch (`reforge=off` in the section, `RUSTIC_ISLES_REFORGE=log` in the file → `log`); `qos_port=80` (a number) → `80`; an absent switch is none; `on_unless_off` default true, `OFF` false; admin API default off, ` On ` on, `RUSTIC_ISLES_MANAGER_ADMIN=on` on, `yes` an error |
| Env merge | `backend.env` (`PORT=61000`, `DEPLOYSERVER_URL`, `REGISTRATION_MODE`), sibling `admin.env` (`REFORGED_DB_ENGINE=sqlite`, `REFORGED_DB_FILE`, `PORT=61003`), `director.env` (`PORT=61001`, `MY_IP`, `PORT_RANGE_BEGIN=8766`, `PORT_RANGE_END=8777`, `GAMESERVER_BINARY_PATH`, `METAGAME_API_KEY`): `PORT` 61000, the database and Director values present; §6.4's port-range table |
| No-player | §5.3's table |
| Season | §8.4's tables; the anchor's weekday is Thursday |
| Loader | `\u{feff}{"n":1.0,"big":12345678901234567890}` → written back `{"n":1,"big":12345678901234567000}`; a missing optional file none, required `Missing`; `{` → `Unreadable`; no folder `Missing`; the cached read returns the first content after the file changes |
| Replay | the same seed gives the same id sequence in two requests; ids are valid v4; a scope time overrides a test pin; `replay_now_ms` ignores the test pin; database closures see the scope |
| Ids | an account id round-trips through text and reports its node as home; a hunt id is not an account id; a random userId is 36 characters |
| Module | §13.3's drive over a bare composer: register, start on ephemeral ports, every backend route answers per `sweep`, the Director's `/health/ready` 503 and `/api/instances` `200 []` (when the Director service is composed), status `Ready`, unload; start twice → `already started`; a bad admin API value fails the start and leaves `Failed` with the text |
| Admin | `Ban` with an RFC 3339 time in the past → `Invalid`; not RFC 3339 → `Invalid`; no database → `Unavailable` (the rest is the composer's: a timeout keeps a player out until it ends or is lifted, lookup shows it, a ban clears it) |
| Commands | not running → `Unavailable`; an unknown command `Unsupported`; a home invocation for an account of another node → `Invalid` naming both nodes |

### 13.3 Test support (feature `test-support`)

- `sample_path(route)`: each `:param` and each regex group `x`; a regex ending in `\/` gets a trailing `x`; a
  `gauntlet` row is prefixed with `/__origin/gauntlet-prod.steelyard.ca`.
- `request(address, method, path)`: one HTTP/1.1 request on a new connection (`Connection: close`,
  `Content-Length: 0`), answering status, lower-cased headers and body.
- `listening(host)`: the addresses a started module emitted in `isles.listening`, from a recording host.
- `sweep(service, address, mounts)`: every row of the service except those an area handles (and those whose sample
  path another handled row takes) must answer: a mounted row (or one whose path a mounted row takes) 501 in the
  envelope with a request id, unless gated by the admin API (404); an unmounted one empty 404; a rifts row behind
  launcher auth 401 `AUTH_UNAUTHORIZED`; every answer carries `x-rustic-isles-request-id`.
- `drive(driver, host, env_file, mounts)`: register, configure every port to 0 and the env file, start, sweep the
  backend, check the QoS pong is loopback when present and the Director's two probes, check `Ready`, unload.

---

## 14. Security behaviour (for `docs/security/rustic-isles.md` and RX3)

- **Trust boundary.** Every listener binds 127.0.0.1; the gateway is the only public listener and refuses the
  host-only prefixes this module registers (§7.2). `x-forwarded-for` is honoured only from a loopback peer (auth
  §4.6). The backend does not check the gateway's `x-reforged-gateway` secret: loopback binding is the boundary
  (§15 item 5).
- **Origin strip** accepts only the four apexes and their subdomains, strict host characters, `decodeURIComponent`
  failures refused; the strip never yields a path without a leading `/`.
- **Request id** is bounded (8-96 characters of a fixed set), so a client's id cannot inject log lines or headers.
- **Bodies**: 100 KiB by default, strict JSON first character, identity encoding and UTF-8 only; rifts bodies capped
  at 100 KiB before relay.
- **Rate limits**: one count per request, before routes, probes excluded; buckets per client /56 or address, per
  game server address, per host capability.
- **Admin API** off by default; every `/admin` path is 404 before any admin router when off.
- **Secrets never logged**: settings values (signing keys, `API_KEY_HASH_SECRET`, `METAGAME_API_KEY`, SMTP and
  Discord secrets) never reach a log or a `Debug`; the legacy warning names variables, never values; the
  Unstubbed-route line logs the path without the query.
- **Game data** is read only from the configured folders, by fixed file names.
- **Known weak spots to list**:
  - **Replay headers are honoured from any client** (§9.1): the gateway forwards `x-isles-replay-now-ms` and
    `x-isles-replay-seed` unchanged (it drops only forwarding and gateway headers), so a player can pin the clock of
    their own requests (a new budget window per request, so no rate limit; daily and cooldown rules at a chosen
    time) and make ids predictable. §15 item 1 proposes a fix; until then, list it.
  - A game server key header selects the higher game-server budget before the key is checked (6000/min per address).
  - The cached game data read never refreshes.

**Fuzz targets (`fuzz/fuzz_targets/`):** `rustic_isles_routes`: any path bytes (lossy UTF-8) against every row of
the table never panics, takes time linear in the path, and agrees with a reference model of §4.1 (segment count,
ASCII case-insensitive literals, non-empty params, decode failures refused). `rustic_isles_pipeline_text`: the origin
strip, the request id normalizer, `decodeURIComponent`, the `/admin` and `/mystic/v1/hunts` prefix checks, the
switches reader and the per-minute parser never panic; a stripped target always starts `/`; an accepted host always
ends in an allowed apex; a kept request id is always safe.

---

## 15. Open questions and behaviour that looks wrong but is kept

1. **Replay headers from anyone** (§14). Proposed: honour them only when a new setting
   `RUSTIC_ISLES_REPLAY_HEADERS=on` (off by default; set by `reforged-host db replay`'s target only), and have the
   gateway drop `x-isles-replay-*` from the internet. A behaviour change and a seam change (the setting,
   `settings.toml`, the gateway): a decision row. The implementer keeps today's behaviour unless the coordinator says
   otherwise.
2. **Legacy names.** `MYSTICPARADOX_*` env aliases and `x-mysticparadox-*` header copies were for "one release" after
   2026-11-01. Proposed: drop from the MIT crate (as auth §15 item 9); restricted pre-maps env if it must.
3. **No automatic `OPTIONS`/`405`.** Express answers `OPTIONS` on a matched path with `200` and an `Allow` list; here
   it is the 404 fallthrough outside launcher CORS. Kept (the 1.14.7 client never sends it).
4. **Bodies are parsed per handler.** Express parses JSON before any route, so a malformed JSON body on a route that
   ignores its body is a 400 in Node and passes here. Kept; a handler that reads its body answers as Node.
5. **The gateway secret** is not checked by the module; the 127.0.0.1 bind is the boundary. Kept.
6. **Two request-id conventions**: the 501 and rifts refusals carry the request's id; auth's launcher envelope a fresh
   UUID (auth §2.3). Kept for parity.
7. **Rifts refusals** use the request id while auth's do not (item 6). Kept.
8. **`preserve_order`** is not on in dainty-lass's workspace `serde_json`; every JSON answer here needs insertion
   order. A seam change for the integration pass (shared with auth).
9. **`Auth<Arc<dyn AuthStore>>`** needs a blanket `AuthStore` impl for `Arc<T>` in `rustic-isles-auth` (§10).
10. **`bootstrap.rs`** (`bootstrap_admin.ts`) is reached only from a test. Proposed: specify it with the Reforged API
    admin unit as a `reforged-host` command, or drop it.
11. **Category A files** (§1.2) could move as they are under the maintainer's copyright, but NOTICE.md puts the whole
    module under Paradox's section 7 terms in one sentence. Proposed: rewrite all from this spec (as done here) and
    let the maintainer confirm that the A files carry no Paradox terms if any is ever moved instead.
12. **The 501 text** keeps the `isles:` prefix; it is client-visible only for unported routes. Kept.
13. **Module id** stays `isles-1-14-7` (settings.toml, the gateway's routing and invites name it) though the MIT crates
    are `rustic-isles-*`.
14. **Process globals**: restricted keeps the chat presence and the Reforged status cache as process-wide statics and
    the game data cache keyed by path; two modules in one process would share presence. The MIT crate keeps them per
    module (§5.5); one module behaves the same.
15. **`Ban` validation first**: an `until` in the past is `Invalid` even for an unknown player. Kept.

---

## 16. Port units for the rest of rustic-isles

Each unit is a spec, then an MIT crate in dainty-lass, then restricted's composer drops the AGPL code it replaces.
Dependencies seen at `e0e238b`: areas call auth everywhere (`game_auth`, tokens, policy); characters calls only auth;
inventory, progression and bounties-store call each other in a cycle (inventory uses the wallet, progression's
reforge and tracks; progression uses inventory's transaction; bounties uses progression's tracks and inventory's
transaction and ports); matchmaking uses inventory's items; the Reforged API uses auth policy, inventory ports,
progression tracks, matchmaking state and the platinum store, and auth calls back into it (activity, operations,
notices, logs) through auth's `Seams`; chat uses auth tokens and policy; game-data uses only `game_auth`. `isles-db`
uses the schema, ids, repositories, the characters starter list, inventory storage policy and the replay harness;
`isles-director` uses the skeleton (`PortService`, `Shared`, settings, game data root, legacy env names) and
matchmaking's deploy seam (now §5.6 here); `isles-1-14-7` composes everything.

| # | Unit (MIT crate) | Covers (restricted) | Needs from earlier units |
| --- | --- | --- | --- |
| 0 | `rustic-isles-auth` | `areas/auth*` | — (in progress) |
| 1 | `rustic-isles` (this spec) | §1.2 | auth |
| 2 | `rustic-isles-store` | `db.rs`'s database layer (open with the schema, the soft-delete list, the actor mapping System→`isles`, Console→host `console`, Admin→its key, Player→the account text, Module→its id; account writes refused off the home, `NotHome`; the replay scope inside closures), the migrations (both engines, versions unchanged), every repository trait and SQL implementation (`repo/*`), and the `AuthStore` adapter | `store`, core ids and replay scope; auth's record types |
| 3 | `rustic-isles-gamedata` | `areas/game_data.rs`, `game_data/{progression_config,tuning,gauntlet,patchnotes}.rs`, the generators (`generators.rs`, `src_tokens.txt`) and their `reforged-host gamedata` entry | core (loader, season window, hooks, tables), auth (`game_auth`) |
| 4 | `rustic-isles-director` | `isles-director` (instances, hubs, port leases, memory guard, hunt tables and their importer, the fake game server bin) | core (`PortService`, settings `vars()`, game data root, §5.6) |
| 5 | `rustic-isles-characters` | `areas/characters*`, the starter manifest rules | core, store, auth |
| 6 | `rustic-isles-chat` | `isles-chat` (JIDs, rooms, login, headlines, presence source) and `areas/chat*` (friends, guild stubs, `/vivox/login`) | core, store, auth (tokens, policy); `module-api::chat` |
| 7 | `rustic-isles-economy` (three specs: inventory-reforge, progression, bounties-store; one crate because of the cycle) | `areas/{inventory,progression,bounties_store}*`, multipliers' reward and XP hooks | core, store, auth, characters |
| 8 | `rustic-isles-matchmaking` | `areas/matchmaking*` (queues, candidates, parties, outbox, QoS pong, rifts `IslesCore`, presence), the Director invocation and ring recording | core (§5.5, §5.6), store, auth, economy (items) |
| 9 | `rustic-isles-reforged` | `areas/reforged_api*` (probes, metrics, request log as the `Observer`, launcher API, updates, logs, notices, activity, operations, guard, admin API and admin Discord, tester max, release gates), the admin actions and home operations of §7.6 and §7.8, `bootstrap.rs` if kept | everything above; auth's `Seams` are implemented here |
| 10 | `rustic-isles-replay` | `replay.rs` and `isles-db`'s replay CLI (§9.3) | core replay headers, auth test keys |
| 11 | `rustic-isles-db` | `isles-db` migrate, verify, export-mongo (until RB15 removes Mongo) | store, characters (starter ids), economy (storage policy) |
| 12 | `rustic-isles-1-14-7` and its plugin | `isles-1-14-7` (hooks' wire payloads, `TABLES`, `DIRECTOR`, the module spec, rifts composition over dainty-lass's MIT `rustic-rifts-*`), `rustic-isles-plugin` (one line: `export_plugin!`) | all; the composer then lives here and restricted keeps nothing of the module but `native/`, `tools/RuntimeLoader`, `reference/` and the goldens |

Order: 0 and 1 now; 2 next (every area needs it, and auth's store adapter moves with it); 3 and 4 in parallel with
2 (they need only core); then 5, 6, 7, 8, 9 in that order (9 last because auth's seams and the admin hooks land
there); 10 and 11 after 2 and 7; 12 last. Each spec's tests replay restricted's goldens for its routes through the
composer; native code, `tools/RuntimeLoader` and `reference/` stay in restricted.

---

## Appendix A. The route table (registration order = match order)

`B` rows are the backend's, `D` rows the Director's. When: empty is always.

| # | Method | Path | Group | Side | When |
| --- | --- | --- | --- | --- | --- |
| B1 | GET | `/` | reforged-api | isles |  |
| B2 | GET | `/health/live` | reforged-api | isles |  |
| B3 | GET | `/health/ready` | reforged-api | isles |  |
| B4 | GET | `/health/release-gates` | reforged-api | isles | admin |
| B5 | GET | `/internal/v1/metrics` | reforged-api | isles |  |
| B6 | GET | `/features/platform/win` | auth | isles |  |
| B7 | GET | `/account/link/epic/:AccId` | auth | isles |  |
| B8 | POST | `/login` | auth | isles |  |
| B9 | GET | `/accountinfo` | auth | isles |  |
| B10 | GET | `/tags` | auth | isles |  |
| B11 | PUT | `/gamesession/epic` | auth | isles |  |
| B12 | PUT | `/gamesession/epiceos` | auth | isles |  |
| B13 | POST | `/accountinfo/public` | auth | isles |  |
| B14 | POST | `/migration/trigger` | auth | isles |  |
| B15 | GET | `/migration/status` | auth | isles |  |
| B16 | GET | `/isbanned` | auth | isles |  |
| B17 | POST | `/account/api/oauth/token` | auth | isles |  |
| B18 | GET | `/account/api/oauth/verify` | auth | isles |  |
| B19 | GET | `/account/api/public/account/:AccId` | auth | isles |  |
| B20 | GET | `/account/api/public/account/:AccId/externalAuths` | auth | isles |  |
| B21 | GET | `/account/api/public/account/displayName/:displayName` | auth | isles |  |
| B22 | DELETE | `/account/api/oauth/sessions/kill` | auth | isles |  |
| B23 | DELETE | `/account/api/oauth/sessions/kill/:AuthToken` | auth | isles |  |
| B24 | GET | `/account/api/public/account` | auth | isles |  |
| B25 | GET | `/dauntless-status` | reforged-api | isles |  |
| B26 | POST | `/heartbeat` | game-data | isles |  |
| B27 | POST | `/event` | game-data | isles |  |
| B28 | GET | `/crash/CrashReporter/Ping` | game-data | isles |  |
| B29 | POST | `/crash/CrashReporter/CheckReport` | game-data | isles |  |
| B30 | GET | `/survey/config` | game-data | isles |  |
| B31 | POST | `/account/migrate` | game-data | isles |  |
| B32 | POST | `/profile/update` | game-data | isles |  |
| B33 | POST | `/trials/leaderboards/all` | game-data | isles |  |
| B34 | POST | `/trials/leaderboards/solo/individual` | game-data | isles |  |
| B34a | POST | `/trials/leaderboards` | game-data | isles |  |
| B34b | POST | `/trials/leaderboards/solo` | game-data | isles |  |
| B34c | POST | `/trials/leaderboards/group` | game-data | isles |  |
| B34d | POST | `/trials/leaderboards/group/individual` | game-data | isles |  |
| B34e | POST | `/^\/trials\/(.*)$/` | game-data | isles |  |
| B35 | GET | `/vivox/login` | chat | isles |  |
| B36 | POST | `/motd/` | game-data | isles |  |
| B37 | GET | `/motd/trigger` | game-data | isles |  |
| B38 | GET | `/entitlementsv2` | bounties-store | isles |  |
| B39 | POST | `/entitlementv2/:userId` | bounties-store | isles |  |
| B40 | GET | `/playertreatments` | game-data | isles |  |
| B41 | GET | `/playertreatments/` | game-data | isles |  |
| B42 | GET | `/playertreatments/:userId` | game-data | isles |  |
| B43 | GET | `/escalation/:escalationSeason/:userId` | progression | isles |  |
| B44 | POST | `/escalation/:escalationSeason/:userId` | progression | isles |  |
| B45 | GET | `/eventstats/` | game-data | isles |  |
| B46 | GET | `/progression/config` | game-data | isles |  |
| B47 | GET | `/config` | game-data | isles | gauntlet |
| B48 | GET | `/^\/progression\/([^/]*)\/([^/]+)$/i` | game-data | isles | gauntlet |
| B49 | GET | `/^\/rewards\/personal\/([^/]*)\/([^/]+)$/i` | game-data | isles | gauntlet |
| B50 | GET | `/^\/rewards\/guild\/([^/]*)\/([^/]+)$/i` | game-data | isles | gauntlet |
| B51 | GET | `/^\/leaderboard\/get_leaderboard\//i` | game-data | isles | gauntlet |
| B51a | PUT | `/^\/(.*)$/` | game-data | isles | gauntlet |
| B51b | POST | `/^\/(.*)$/` | game-data | isles | gauntlet |
| B52 | GET | `/huntpass/:userId` | progression | isles |  |
| B53 | GET | `/cooldown/:userId` | bounties-store | isles |  |
| B54 | PUT | `/cooldown/batch/:userId` | bounties-store | isles |  |
| B55 | PUT | `/cooldown/:userId/:cooldownId` | bounties-store | isles |  |
| B56 | POST | `/cooldown/:userId/:cooldownId` | bounties-store | isles |  |
| B57 | GET | `/bounty/game-data` | bounties-store | isles |  |
| B58 | GET | `/bounty/:userId` | bounties-store | isles |  |
| B59 | POST | `/bounty/:userId` | bounties-store | isles |  |
| B60 | POST | `/bounty/delete/:userId` | bounties-store | isles |  |
| B61 | DELETE | `/bounty/delete/:userId` | bounties-store | isles |  |
| B62 | GET | `/all/` | game-data | isles |  |
| B63 | GET | `/patchnotes/:language/:gameversion` | game-data | isles |  |
| B64 | GET | `/maintenance/status` | reforged-api | isles |  |
| B65 | GET | `/character` | characters-loadouts | isles |  |
| B66 | PUT | `/character` | characters-loadouts | isles |  |
| B67 | POST | `/character` | characters-loadouts | isles |  |
| B68 | POST | `/^\/inventory\/([^/]*)\/(.+)$/` | inventory-reforge | isles |  |
| B69 | POST | `/inventory/:characterId/:changeList` | inventory-reforge | isles |  |
| B70 | GET | `/inventory/:userId/:characterId` | inventory-reforge | isles |  |
| B71 | GET | `/inventory/:userId/` | inventory-reforge | isles |  |
| B72 | POST | `/inventory` | inventory-reforge | isles |  |
| B73 | POST | `/inventory/instanceditem` | inventory-reforge | isles |  |
| B74 | POST | `/reconcile` | bounties-store | isles |  |
| B75 | GET | `/creator` | bounties-store | isles |  |
| B76 | GET | `/balance` | bounties-store | isles |  |
| B77 | GET | `/product/skus/public` | bounties-store | isles |  |
| B78 | GET | `/token/:currency/:catalogId` | bounties-store | isles |  |
| B79 | POST | `/notification/:currency` | bounties-store | isles |  |
| B80 | GET | `/guild/invite/player` | chat | isles |  |
| B81 | GET | `/guild` | chat | isles |  |
| B82 | GET | `/game_tuning/seasonal_event_schedule` | game-data | isles |  |
| B83 | GET | `/game_tuning/huntpass_xp_config` | game-data | isles |  |
| B84 | GET | `/game_tuning/island_content_config` | game-data | isles |  |
| B85 | GET | `/game_tuning/bounty_game_data` | game-data | isles |  |
| B86 | GET | `/game_tuning/bounty_game_data_daily` | game-data | isles |  |
| B87 | GET | `/game_tuning/bounty_game_data_weekly` | game-data | isles |  |
| B88 | POST | `/candidate/player/register` | matchmaking-parties | isles |  |
| B89 | POST | `/candidate/player/alive` | matchmaking-parties | isles |  |
| B90 | GET | `/candidate/regions` | matchmaking-parties | isles |  |
| B91 | POST | `/key/generate` | auth | isles |  |
| B92 | POST | `/key/consume` | auth | isles |  |
| B93 | GET | `/candidate/status` | matchmaking-parties | isles |  |
| B94 | POST | `/candidate/join` | matchmaking-parties | isles |  |
| B95 | DELETE | `/candidate/leave` | matchmaking-parties | isles |  |
| B96 | POST | `/candidate/leave` | matchmaking-parties | isles |  |
| B97 | PUT | `/candidate/leave` | matchmaking-parties | isles |  |
| B98 | DELETE | `/candidate` | matchmaking-parties | isles |  |
| B99 | DELETE | `/candidate/:candidateId` | matchmaking-parties | isles |  |
| B100 | GET | `/QoS` | matchmaking-parties | isles |  |
| B101 | POST | `/party` | matchmaking-parties | isles |  |
| B102 | PUT | `/party/invite` | matchmaking-parties | isles |  |
| B103 | GET | `/party/invites` | matchmaking-parties | isles |  |
| B104 | PUT | `/party/invite/accept/:inviterId` | matchmaking-parties | isles |  |
| B105 | DELETE | `/party/member` | matchmaking-parties | isles |  |
| B106 | DELETE | `/party/member/:targetId` | matchmaking-parties | isles |  |
| B107 | PUT | `/party/member/promote/:targetId` | matchmaking-parties | isles |  |
| B108 | GET | `/encountered-content/:characterId/:contentType` | progression | isles |  |
| B109 | POST | `/encountered-content/query/:characterId` | progression | isles |  |
| B110 | POST | `/encountered-content/query` | progression | isles |  |
| B111 | POST | `/encountered-content/:characterId/query` | progression | isles |  |
| B112 | POST | `/encountered-content/:characterId` | progression | isles |  |
| B113 | POST | `/encountered-content` | progression | isles |  |
| B114 | POST | `/encountered-content/` | progression | isles |  |
| B115 | GET | `/progression/objectives/:userId` | progression | isles |  |
| B116 | GET | `/breadcrumbs/` | progression | isles |  |
| B117 | GET | `/breadcrumbs/:characterId` | progression | isles |  |
| B118 | POST | `/breadcrumbs/:characterId` | progression | isles |  |
| B119 | POST | `/progression/:userId/:progressionId/:amount` | progression | isles |  |
| B120 | POST | `/progression/:userId` | progression | isles |  |
| B121 | POST | `/progression/:userId/:progressionId/:rank/confirm/public` | progression | isles |  |
| B122 | GET | `/progression/:userId` | progression | isles |  |
| B123 | GET | `/pjm/:userId` | progression | isles |  |
| B124 | GET | `/weapon_tracker/:userId/:itemId` | progression | isles |  |
| B125 | POST | `/pjm/:userId` | progression | isles |  |
| B126 | GET | `/pjm` | progression | isles |  |
| B127 | GET | `/progression/tracked_objectives/:phxAccountId` | progression | isles |  |
| B128 | POST | `/progression/tracked_objectives/:phxAccountId` | progression | isles |  |
| B129 | GET | `/progression/:userId/:progressionId` | progression | isles |  |
| B130 | DELETE | `/progression/:userId/:progressionId` | inventory-reforge | isles |  |
| B131 | POST | `/progression/purchase/:userId/:progressionId/:amount` | inventory-reforge | isles |  |
| B132 | GET | `/loadout/:userId//all` | characters-loadouts | isles |  |
| B133 | GET | `/loadout/:userId/:characterId/all` | characters-loadouts | isles |  |
| B134 | POST | `/loadout/:userId/:characterId/unlock/:numSlots` | characters-loadouts | isles |  |
| B135 | POST | `/loadout/:userId/:characterId/active/:index` | characters-loadouts | isles |  |
| B136 | POST | `/loadout/:userId/:characterId/:index` | characters-loadouts | isles |  |
| B137 | GET | `/friends/api/public/friends/:accountId` | chat | isles |  |
| B138 | POST | `/friends/api/public/friends/:accountId/:friendId` | chat | isles |  |
| B139 | DELETE | `/friends/api/public/friends/:accountId/:friendId` | chat | isles |  |
| B140 | GET | `/friends/api/public/blocklist/:accountId` | chat | isles |  |
| B141 | GET | `/friends/api/public/list/:namespace/:accountId/recentPlayers` | chat | isles |  |
| B142 | GET | `/friends/api/v1/:accountId/settings` | chat | isles |  |
| B143 | POST | `/slayerlink/availability` | chat | isles |  |
| B144 | GET | `/slayerlink/status_good` | chat | isles |  |
| B145 | POST | `/launcher/v1/auth/register` | auth | isles |  |
| B146 | POST | `/launcher/v1/auth/login` | auth | isles |  |
| B147 | POST | `/launcher/v1/auth/refresh` | auth | isles |  |
| B148 | POST | `/launcher/v1/auth/logout` | auth | isles |  |
| B149 | POST | `/launcher/v1/auth/logout-all` | auth | isles |  |
| B150 | POST | `/launcher/v1/auth/discord/start` | auth | isles |  |
| B151 | GET | `/launcher/v1/auth/discord/callback` | auth | isles |  |
| B152 | POST | `/launcher/v1/auth/discord/complete` | auth | isles |  |
| B153 | POST | `/launcher/v1/auth/email/verify` | auth | isles |  |
| B154 | POST | `/launcher/v1/auth/email/resend` | auth | isles |  |
| B155 | POST | `/launcher/v1/auth/password/forgot` | auth | isles |  |
| B156 | POST | `/launcher/v1/auth/password/reset` | auth | isles |  |
| B157 | POST | `/launcher/v1/account/discord/link/start` | auth | isles |  |
| B158 | POST | `/launcher/v1/account/discord/unlink/start` | auth | isles |  |
| B159 | POST | `/launcher/v1/account/discord/unlink` | auth | isles |  |
| B160 | GET | `/launcher/v1/me` | auth | isles |  |
| B161 | GET | `/launcher/v1/username` | auth | isles |  |
| B162 | POST | `/launcher/v1/username` | auth | isles |  |
| B163 | POST | `/launcher/v1/game-sessions` | auth | isles |  |
| B164 | GET | `/launcher/v1/policy` | auth | isles |  |
| B165 | GET | `/launcher/v1/status` | auth | isles |  |
| B166 | GET | `/launcher/v1/legal` | auth | isles |  |
| B167 | GET | `/launcher/v1/notices` | auth | isles |  |
| B168 | GET | `/launcher/v1/account/consent` | auth | isles |  |
| B169 | POST | `/launcher/v1/account/consent` | auth | isles |  |
| B170 | GET | `/launcher/v1/account/deletion` | auth | isles |  |
| B171 | POST | `/launcher/v1/account/deletion` | auth | isles |  |
| B172 | DELETE | `/launcher/v1/account/deletion` | auth | isles |  |
| B173 | POST | `/reforged/api/Register` | reforged-api | isles |  |
| B174 | GET | `/reforged/api/GetUserInfo` | reforged-api | isles |  |
| B175 | GET | `/reforged/api/ServerStatus` | reforged-api | isles |  |
| B176 | GET | `/reforged/api/RegistrationStatus` | reforged-api | isles |  |
| B177 | POST | `/reforged/api/CreateInvite` | reforged-api | isles |  |
| B178 | GET | `/reforged/api/InviteCodes` | reforged-api | isles |  |
| B179 | DELETE | `/reforged/api/InviteCode/:code` | reforged-api | isles |  |
| B180 | POST | `/reforged/api/SetAccountStatus` | reforged-api | isles |  |
| B181 | POST | `/reforged/api/Kick` | reforged-api | isles |  |
| B182 | GET | `/launcher/v1/runtime/:target/:channel/:platform` | reforged-api | isles |  |
| B183 | GET | `/launcher/v1/runtime/:channel/:platform` | reforged-api | isles |  |
| B184 | GET | `/launcher/v1/runtime/:target/:channel/:platform/download` | reforged-api | isles |  |
| B185 | GET | `/launcher/v1/runtime/:channel/:platform/download` | reforged-api | isles |  |
| B186 | GET | `/launcher/v1/guard-manifests/:channel/:platform` | guard | rifts |  |
| B187 | POST | `/launcher/v1/admin/guard-manifests/:channel/:platform` | guard | rifts |  |
| B188 | POST | `/launcher/v1/admin/updates/runtime/:target/:channel/:platform` | reforged-api | isles |  |
| B189 | POST | `/launcher/v1/admin/updates/runtime/:target/:channel/:platform/extra` | reforged-api | isles |  |
| B190 | GET | `/launcher/v1/runtime/:target/:channel/:platform/extra/:filename` | reforged-api | isles |  |
| B191 | GET | `/launcher/v1/admin/updates/serving` | reforged-api | isles |  |
| B192 | POST | `/launcher/v1/admin/updates/serving` | reforged-api | isles |  |
| B193 | GET | `/launcher/v1/updates/:target/:arch/download` | reforged-api | isles |  |
| B194 | GET | `/launcher/v1/updates/:target/:arch/files/:version/:file` | reforged-api | isles |  |
| B195 | GET | `/launcher/v1/updates/:target/:arch/:currentVersion` | reforged-api | isles |  |
| B196 | PUT | `/launcher/v1/logs/sessions/:launchSessionId/:fileName` | reforged-api | isles |  |
| B197 | GET | `/admin/v1/launcher-logs` | reforged-api | isles | admin |
| B198 | GET | `/admin/v1/launcher-logs/:userId/:launchSessionId/:fileName` | reforged-api | isles | admin |
| B199 | POST | `/admin/v1/auth/login` | reforged-api | isles | admin |
| B200 | POST | `/admin/v1/auth/logout` | reforged-api | isles | admin |
| B201 | GET | `/admin/v1/auth/discord/start` | reforged-api | isles | admin |
| B202 | GET | `/admin/v1/auth/discord/callback` | reforged-api | isles | admin |
| B203 | GET | `/admin/v1/me` | reforged-api | isles | admin |
| B204 | GET | `/admin/v1/online-players` | reforged-api | isles | admin |
| B205 | GET | `/admin/v1/players` | reforged-api | isles | admin |
| B206 | PATCH | `/admin/v1/players/:userId/access` | reforged-api | isles | admin |
| B207 | PATCH | `/admin/v1/players/:userId/roles` | reforged-api | isles | admin |
| B208 | DELETE | `/admin/v1/players/:userId` | reforged-api | isles | admin |
| B209 | GET | `/admin/v1/audit` | reforged-api | isles | admin |
| B210 | GET | `/admin/v1/operations/overview` | reforged-api | isles | admin |
| B211 | PATCH | `/admin/v1/operations/policy` | reforged-api | isles | admin |
| B212 | GET | `/admin/v1/launcher-notices` | reforged-api | isles | admin |
| B213 | PUT | `/admin/v1/launcher-notices` | reforged-api | isles | admin |
| B214 | GET | `/admin/v1/guard/sessions` | guard | rifts | admin |
| B215 | GET | `/admin/v1/guard/sessions/:guardSessionId` | guard | rifts | admin |
| B216 | GET | `/admin/v1/guard/findings` | guard | rifts | admin |
| B217 | GET | `/admin/v1/activity/players/:userId` | reforged-api | isles | admin |
| B218 | GET | `/admin/v1/activity/overview` | reforged-api | isles | admin |
| B219 | GET | `/admin/v1/activity/flags` | reforged-api | isles | admin |
| B220 | PATCH | `/admin/v1/activity/flags/:id` | reforged-api | isles | admin |
| B221 | POST | `/admin/v1/players/:userId/tester-max` | reforged-api | isles | admin |
| B222 | GET | `/admin/v1/player-logs` | reforged-api | isles | admin |
| B223 | GET | `/admin/v1/player-logs/:userId/:launchSessionId/:fileName` | reforged-api | isles | admin |
| B224 | DELETE | `/admin/v1/player-logs/:userId/:launchSessionId` | reforged-api | isles | admin |
| B225 | POST | `/launcher/v1/guard/sessions` | guard | rifts |  |
| B226 | POST | `/launcher/v1/guard/heartbeat` | guard | rifts |  |
| B227 | GET | `/launcher/v1/guard/sessions/:guardSessionId` | guard | rifts |  |
| B228 | POST | `/mystic/v1/hunts/:sessionId/tickets/self` | p2p | rifts | rifts |
| B229 | POST | `/mystic/v1/hunts/:sessionId/host-ready` | p2p | rifts | rifts |
| B230 | POST | `/mystic/v1/hunts/:sessionId/event-batches` | guard | rifts | rifts |
| B231 | POST | `/mystic/v1/hunts/:sessionId/completion` | p2p | rifts | rifts |
| B232 | POST | `/mystic/v1/hunts/:sessionId/abort` | p2p | rifts | rifts |
| B233 | POST | `/mystic/v1/hunts/:sessionId/end` | p2p | rifts | rifts |
| B234 | GET | `/mystic/v1/hunts/:sessionId/status` | p2p | rifts | rifts |
| B235 | GET | `/mystic/v1/hunts/:sessionId/reward/self` | p2p | rifts | rifts |
| B236 | POST | `/internal/v1/hunt-guard/shadow-comparisons` | guard | rifts | rifts |
| B237 | POST | `/mystic/v1/hunts/:sessionId/lease` | p2p | rifts | rifts |
| B238 | POST | `/mystic/v1/hunts/:sessionId/tickets/consume` | p2p | rifts | rifts |
| B239 | POST | `/mystic/v1/hunts/:sessionId/peers/:admissionId/disconnect` | p2p | rifts | rifts |
| B240 | PATCH | `/admin/v1/operations/p2p-emergency-stop` | p2p | rifts | rifts |
| B241 | PATCH | `/admin/v1/operations/reward-rollout` | p2p | rifts | rifts |
| B242 | PATCH | `/admin/v1/operations/coop-transport` | p2p | rifts | rifts |
| B243 | PATCH | `/admin/v1/operations/alerts/:alertId/ack` | p2p | rifts | rifts |
| B244 | POST | `/launcher/v1/p2p/hunts/:sessionId/host-bootstrap` | p2p | rifts | rifts |
| B245 | POST | `/launcher/v1/p2p/hunts/:sessionId/join-bootstrap` | p2p | rifts | rifts |
| B246 | POST | `/launcher/v1/p2p/hunts/:sessionId/route-ready` | p2p | rifts | rifts |
| B247 | POST | `/launcher/v1/p2p/hunts/:sessionId/transport-config` | p2p | rifts | rifts |
| B248 | POST | `/launcher/v1/p2p/hunts/:sessionId/signal` | p2p | rifts | rifts |
| B249 | GET | `/launcher/v1/p2p/control` | p2p | rifts | rifts |
| B250 | POST | `/launcher/v1/p2p/control/:messageId/ack` | p2p | rifts | rifts |
| B251 | POST | `/launcher/v1/p2p/host-presence` | p2p | rifts | rifts |
| B252 | POST | `/launcher/v1/p2p/host-offers/:offerId/respond` | p2p | rifts | rifts |
| B253 | POST | `/launcher/v1/p2p/hunts/:sessionId/observation` | p2p | rifts | rifts |
| B254 | POST | `/launcher/v1/realm-hosting/hubs/start` | realms | rifts | rifts |
| B255 | POST | `/launcher/v1/realm-hosting/hubs/stop` | realms | rifts | rifts |
| B256 | POST | `/launcher/v1/p2p/hunts/:sessionId/peers` | realms | rifts | rifts |
| B257 | GET | `/launcher/v1/realm-hosting/status` | realms | rifts | rifts |
| B258 | POST | `/launcher/v1/realms` | realms | rifts | rifts |
| B259 | GET | `/launcher/v1/realms/mine` | realms | rifts | rifts |
| B260 | GET | `/launcher/v1/realms/listed` | realms | rifts | rifts |
| B261 | GET | `/launcher/v1/realms/by-name/:name` | realms | rifts | rifts |
| B262 | GET | `/launcher/v1/realms/invite/:code` | realms | rifts | rifts |
| B263 | POST | `/launcher/v1/realms/join` | realms | rifts | rifts |
| B264 | POST | `/launcher/v1/realms/by-name/:name/leave` | realms | rifts | rifts |
| B265 | POST | `/launcher/v1/realms/mine/settings` | realms | rifts | rifts |
| B266 | POST | `/launcher/v1/realms/mine/invites` | realms | rifts | rifts |
| B267 | GET | `/launcher/v1/realms/mine/invites` | realms | rifts | rifts |
| B268 | POST | `/launcher/v1/realms/mine/invites/:code/revoke` | realms | rifts | rifts |
| B269 | POST | `/launcher/v1/realms/mine/members/:accountId/kick` | realms | rifts | rifts |
| B270 | POST | `/launcher/v1/realms/mine/members/:accountId/ban` | realms | rifts | rifts |
| B271 | POST | `/launcher/v1/realms/mine/members/:accountId/unban` | realms | rifts | rifts |
| B272 | POST | `/launcher/v1/realms/mine/heartbeat` | realms | rifts | rifts |
| B273 | POST | `/launcher/v1/realms/mine/release` | realms | rifts | rifts |
| B274 | GET | `/launcher/v1/realms/mine/requests` | realms | rifts | rifts |
| B275 | POST | `/launcher/v1/realms/mine/requests/:accountId/approve` | realms | rifts | rifts |
| B276 | POST | `/launcher/v1/realms/mine/requests/:accountId/deny` | realms | rifts | rifts |
| B277 | POST | `/launcher/v1/realms/mine/members/add` | realms | rifts | rifts |
| B278 | POST | `/launcher/v1/realms/mine/player-invites` | realms | rifts | rifts |
| B279 | POST | `/launcher/v1/realms/mine/player-invites/:accountId/cancel` | realms | rifts | rifts |
| B280 | GET | `/launcher/v1/realms/player-invites` | realms | rifts | rifts |
| B281 | POST | `/launcher/v1/realms/player-invites/:name/accept` | realms | rifts | rifts |
| B282 | POST | `/launcher/v1/realms/player-invites/:name/decline` | realms | rifts | rifts |
| B283 | POST | `/launcher/v1/realms/presence/heartbeat` | realms | rifts | rifts |
| B284 | POST | `/launcher/v1/realms/presence/leave` | realms | rifts | rifts |
| B285 | GET | `/launcher/v1/realms/presence` | realms | rifts | rifts |
| D1 | GET | `/health/ready` | director | isles |  |
| D2 | GET | `/api/hub-status` | director | isles |  |
| D3 | GET | `/api/instances` | director | isles |  |
| D4 | POST | `/api/matchmaker/handle-matchmaking-for-player` | director | isles |  |
| D5 | POST | `/api/matchmaker/resolve-hunt-launch-manifest` | p2p | rifts | rifts |

---

## 17. Coordinator decisions (2026-11-05, before implementation)

These settle §15 and override anything above that disagrees.

- **Item 1: replay headers are gated (a security departure from parity).** The crate honours
  `x-isles-replay-now-ms` and `x-isles-replay-seed` only when the module setting `RUSTIC_ISLES_REPLAY_HEADERS` reads
  on (`on`, `true`, `1`, `yes`, trimmed, any case); off by default, and off means the headers are ignored as if absent.
  Restricted's shadow replay turns it on. The gateway dropping the headers is a separate gateway row. Recorded here
  as a candidate decision row for the maintainer.
- **Item 2: no legacy names.** No `MYSTICPARADOX_*` env aliases and no `x-mysticparadox-*` header copies, as auth
  §15 item 9.
- **Item 8: not a seam change.** Every crate that needs key order enables `serde_json`'s `preserve_order` in its own
  manifest, as `service-kit`, `content` and `rustic-rifts-realms` already do.
- **Item 9:** `rustic-isles-auth` provides `impl<T: AuthStore + ?Sized> AuthStore for Arc<T>` (requested of the auth
  implementer).
- **Item 10:** `bootstrap.rs` is not in this crate; the Reforged unit decides its fate.
- **Item 11:** everything is rewritten from this spec; nothing moves as it is.
- **Item 14:** per-module presence and status cache, as specified.
- **Settings `Debug`** prints names only, never values (security over parity).
- **Everything else in §15:** parity, as specified.
- **Unit order:** §16 as proposed.
- **Amended 2026-11-05 with the composition spec:**
  - D3 cutover: the 11 `/mystic/v1/hunts` rows of Appendix A are `/rustic/v1/hunts`, the early-budget prefix of
    §3.2 step 2 is `/rustic/v1/hunts`, and the host capability header is `x-rustic-host-capability`
    (`rifts_seam::HOST_CAPABILITY_HEADER`), no aliases.
  - `quick_emoji_wheel` is a list of texts (nine for 1.14.7), not one optional text.
  - The season window's arithmetic saturates instead of overflowing on an extreme (replay-pinned) time.
