<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false, "code_blocks": false } } -->

# Clean-Room Behaviour Specification: `crates/rustic-isles-matchmaking` (RD27)

> **Scope and purpose (RD27):** this is the clean-room behaviour specification for an MIT crate,
> `rustic-isles-matchmaking`: unit 8 of [`spec-rustic-isles-core.md`](spec-rustic-isles-core.md) §16. It covers
> Rustic Isles' **matchmaking-parties** area: the 18 backend rows B88-B90 and B93-B107 of core Appendix A (the
> candidate routes, the QoS route, the party routes), the in-memory queues, candidates, party instances and hub game
> session ids, the game-presence table, the party outbox dispatcher, the QoS pong listener, the HTTP client of the
> Director, the `rifts_seam::IslesCore` answers that are matchmaking's, and the two `ModuleHooks` that core's Director
> invocation (core §7.8) calls. Its source is dainty-restricted's `modules/rustic-isles/crates/isles-core/src/areas/
> matchmaking.rs` and `areas/matchmaking/*` (less `multipliers.rs`, §1.2), plus the Director-invocation and offline
> parts of `src/module.rs`, AGPL-3.0-only with Mystic Paradox's section 7 terms; an implementer who never sees them
> writes the MIT crate from this document alone.
>
> Everything below is stated from observable contracts: the routes and their bytes, the timings, the candidate
> object the realms side exchanges, the Director's wire, the store calls by the store spec's names, the seam, the
> settings and the tests. No implementation code is reproduced and no private helper is named. Node file names
> appear only as provenance. Wire names the 1.14.7 client, the launcher relay, the runtime or rustic-rifts depend on
> (`/candidate/*`, `/party*`, `/QoS`, `x-reforged-*`, `x-rustic-isles-request-id`, `X-Mystic-Party-Revision`,
> `REALM_HUB`, the reason codes) are interface facts and are kept.
>
> Written against dainty-restricted `e0e238b4434316bb210677d88bc59e3448688794` (worktree
> `splashloader-integration-90130d`; the unit's files last changed in `7cd9db0`, 2026-11-01) and dainty-lass
> `d7dfb5125164bf7108e7f00e3c232ce4dba89a1d` (branch `feat/rustic-isles-clean-room`).

---

## 1. Provenance and licensing classification

### 1.1 Evidence used

| Source | What it says |
| --- | --- |
| `modules/rustic-isles/NOTICE.md` | The whole module is a modified Mystic Paradox, AGPL-3.0-only with the section 7 terms |
| File headers (`//!`) | `controller.rs` ports `controllers/matchmaking.ts`; `status.rs` `controllers/candidateStatus.ts`; `party.rs` `controllers/party.ts` (mm2-prod's party as captured in `DauntlessEndpointDocumentation/Matchmaking`); `resolve.rs` `matchmakingParty.ts` and `matchmakingHuntPolicy.ts`; `outbox.rs` `realtime/PartyOutboxDispatcher.ts`; `qos.rs` `server.ts`/`config/listenerPlan.ts`; `presence.rs` `activityMonitor.ts`'s `GameLastSeen`; `deploy.rs` the DeployServer client; `rifts_core.rs` RF1 §2.2 |
| `git log` of the files | 12 commits, one author (the maintainer): born in `19ee75d` (RU4 seams) and `1728e5a` (RU5 matchmaking-parties), with `ab07f6c` (one `AuthData`, Express body readers), `9ad97c6` (rename), `7cd9db0` (QoS on a loopback port) |
| `reference/1.14.7/ParadoxBackend/src/{controllers,routes}` | The Node files named above (provenance only; not read for this spec beyond file names) |
| `crates/isles-core/tests/matchmaking_routes.rs`, `matchmaking_controller.rs`, the unit tests, `tests/golden/{scenario.json,matchmaking-parties.json}` | What the routes answered under Node 22 and the controller rules (described in §13, never copied) |
| dainty-lass `crates/rifts-seam`, `crates/module-api/src/{network.rs,chat.rs,module.rs}`, the core, store, auth, chat and director specs | The contracts this crate answers and calls |

### 1.2 Per file: category and where it goes

All of it is Category B (Paradox behaviour, or DaintyLass code wrapped around it). The implementer writes every part
from this spec.

| Restricted file | Rewritten as |
| --- | --- |
| `areas/matchmaking.rs` (routes, auth reading, ring join, logs) | §3, §4 |
| `areas/matchmaking/controller.rs` | §5, §6 |
| `areas/matchmaking/state.rs`, `util.rs` | §5.1, §5.2, §5.9 |
| `areas/matchmaking/status.rs` | §3.5, §3.6 |
| `areas/matchmaking/resolve.rs` | §5.4, §5.5 |
| `areas/matchmaking/party.rs` | §7 |
| `areas/matchmaking/outbox.rs` | §8 |
| `areas/matchmaking/qos.rs` | §9 |
| `areas/matchmaking/presence.rs` | §5.8 |
| `areas/matchmaking/deploy.rs` (the HTTP client) | §10 |
| `areas/matchmaking/rifts_core.rs` | §11 |
| `src/module.rs`: Director invocation, offline hook, wiring | §12 |
| `areas/matchmaking/multipliers.rs` | **Not this unit.** It has no route; progression's and inventory's grants call it, and core §16 row 7 assigns "multipliers' reward and XP hooks" to economy. Its only tie to matchmaking is its folder |
| `areas/matchmaking/testkit.rs` | Test support, described in §13 only |

### 1.3 What stays in dainty-restricted

- `tests/golden/{scenario.json,matchmaking-parties.json,capture.cjs}` and `matchmaking_routes.rs`: replayed in
  restricted against this crate once the composer uses it; a failure there is a bug on this side.
- The composer's wiring (core §11.4) until unit 12: it puts this crate's state in a slot, its handlers in `Handlers`,
  its start-ups in the start-up list, its `IslesCore` under rifts, and its hooks in `ModuleHooks`.
- `multipliers.rs` until economy (row 7) takes it.
- The realms side itself (rustic-rifts' P2P module, which answers the `RealmsHooks` this crate calls).

---

## 2. Conventions

- **JSON** is written in insertion order (`serde_json` with `preserve_order` in this crate's own manifest, core §17
  item 8). An answer named "JSON" is core's JSON response: `application/json; charset=utf-8`, compact.
- **The 500 page** is Express's default error page (`text/html; charset=utf-8`, the `Internal Server Error` page core
  §3 uses); it is always preceded by an error log `{route name}: {error text}` (route name: core `RouteDef::name()`).
  **The 400 page** is Express's `Bad Request` page, from core's body reader. **The 501** is core's `not_implemented`
  for the route and request id (core §3.6). A **bare status** has no body and no `content-type`.
- **Times** are Unix ms from the module's matchmaking clock (§5.9); ISO texts are `toISOString` (UTC, three
  fraction digits, `Z`; core `season::to_iso_string`).
- **Ids drawn here** (candidate ids, hub game session ids, hunt launch refs, and the random draw of §5.6) come from
  core's replay-aware `replay::next_uuid()` (lower-case UUID text, 36 characters), so a pinned replay is
  deterministic. The outbox owner id is system-random (§8).
- **A userId** is the text the game token carries (auth §4.1's `AuthData.user_id`); every in-memory table is keyed
  by it, as are the realms' `AccountId`s. A **global id** is `module_api::GlobalId` text (`kind:ULID@node`).
- "**Resolve the account**" of a userId: when the text parses as a `GlobalId` of kind `Account`, that id; else, with
  a store, the account whose `user_id` it is (store `accounts.find_by_user_id`), its global id; else none. "**Foreign**"
  means a resolved account whose owner node is not this node.

---

## 3. Routes

Every route is on the `backend` service. Rows and paths exactly as core Appendix A has them; the match order is
core's. "Game auth" is §4.1. Unless a row says otherwise, a missing userId after game auth is the 500 page (logged
`{route}: no userId in AuthData`), and every successful answer is JSON with status 200.

| Row | Method, path | Auth | Answer |
| --- | --- | --- | --- |
| B88 | `POST /candidate/player/register` | game auth | `{}`; log `userId {id or undefined} is registering for matchmaking!`. No userId is fine |
| B89 | `POST /candidate/player/alive` | game auth | `{}`. With a userId: the realm presence touch (§4.2) and a game-presence touch (§5.8) at now. No userId is fine |
| B90 | `GET /candidate/regions` | game auth | §3.1 |
| B93 | `GET /candidate/status` | game auth, userId | §3.5 |
| B94 | `POST /candidate/join` | game auth, userId | §3.3 |
| B95-B99 | `DELETE /candidate/leave`, `POST /candidate/leave`, `PUT /candidate/leave`, `DELETE /candidate`, `DELETE /candidate/:candidateId` | game auth, userId | §3.4 (all five are one handler; `:candidateId` is ignored) |
| B100 | `GET /QoS` | none | §3.2 |
| B101 | `POST /party` | game auth, userId | §3.7 |
| B102 | `PUT /party/invite` | game auth, userId | §3.8 |
| B103 | `GET /party/invites` | game auth, userId | §3.9 |
| B104 | `PUT /party/invite/accept/:inviterId` | game auth, userId | §3.10 |
| B105 | `DELETE /party/member` | game auth, userId | §3.11 |
| B106 | `DELETE /party/member/:targetId` | game auth, userId | §3.12 |
| B107 | `PUT /party/member/promote/:targetId` | game auth, userId | §3.13 |

**Bodies** are read with core's reader (core §3.9) only by the routes that say so; its failures (the 400 page for
malformed JSON, 413, 415) are answered as they are, after authentication. A "string field" of the body is the value
when it is a JSON string, else absent.

### 3.1 `GET /candidate/regions`

Log `Querying regions for QoS`. JSON:

```json
{"code":200,"message":"success","payload":{"maxPingingStepTime":3,"pingCount":5,"pingFrequency":0.25,"regionUrls":[<QOS_TARGET_URL>]}}
```

`<QOS_TARGET_URL>` is the setting's raw value as a JSON string (an empty value is `""`), or `null` when unset.

### 3.2 `GET /QoS`

Log `QoS Ping`. 200, `content-type: text/html; charset=utf-8`, body exactly
`<!DOCTYPE html><html><body>pong</body></html>` (the **pong**, §9).

### 3.3 `POST /candidate/join`

In this order:

1. Game auth; userId; realm presence touch (§4.2); body read. No parsed body (core's reader gave none: no body or not
   a JSON/form content type) is the 500 page (`req.body is undefined`). `{}` is a body.
2. Fields: `gameMode` kept as the raw JSON value (any type) for the echo below, and as the **mode** when it is a
   string; `gameArgs`, `playerHuntId` (the **hunt id**), `buildId` string fields; `partyId` a string field, absent
   when blank after trimming (the **requested party id**).
3. **Ring join (DN6).** The **target** is the first of `instanceId`, `playerHuntId`, `gameArgs` that is a string
   field (a present but unparsable `instanceId` hides the other two), parsed as a `GlobalId` of kind `Hunt`,
   `HubInstance` or `Party`. With a target and a resolved account, ask the home `ring_resolve_join(account, client
   address or "127.0.0.1", target)` (client address: §3.6):

   | Answer | Response |
   | --- | --- |
   | `Local(record)` | A new candidate for the player: matching `record.queue` with a fresh id, then ready, host and port the record's, game session id the record id's text, release 0 (§5.1). JSON `{"candidateId":<id>,"candidateStatusPeriodMillis":1000,"gameMode":<echo>,"huntId":<record.queue>,"instanceId":<record id>,"status":"MATCHING","statusReason":null}` (`gameMode` left out when the body had none) and header `x-reforged-instance-id: <record id>` |
   | `Refused { instance, owner, region, message }` | 409 JSON `{"error":"WRONG_NODE","message":..,"instance":..,"hostNode":<owner>,"region":..}` |
   | `Redirect { instance, owner, gateway, ticket }` | 200 JSON `{"status":"REDIRECT","instance":..,"hostNode":<owner>,"region":<gateway.region>,"gateway":<PeerGateway as serde writes it: node, region, host, port, fingerprint when set>,"sessionTicket":<ticket.token>}`, headers `x-reforged-redirect-host`, `x-reforged-redirect-port`, `x-reforged-redirect-fingerprint` (only when set), `x-reforged-session-ticket` (each only when it is a valid header value) |
   | `Err(NotInNetwork)` | go on |
   | any other error | warn `ring_resolve_join for {target} failed: {error}`; go on |

4. Log `UserId {user} wants to join a game with GameMode {mode} & GameArgs {args} & HuntId {hunt} & gameType={s}
   privateMatch={s} isPrivate={s} partyId={present|absent}` (absent strings print `undefined`; each `{s}` is the
   field's **shape**: a string as its first 40 characters JSON-quoted, a boolean or number as its JSON text, anything
   else `absent`; never the party id).
5. The player's party (§7, `for_player`). No database: the 501; another failure: the 500 page.
6. Whether the realms side wants offline members filtered: `filters_offline_party_members(mode or "", hunt id)`; a
   seam error is the 500 page.
7. The party resolution (§5.4), a member being **online** when core's chat presence has them (core §5.5) or their
   game touched the backend within the last 75,000 ms (§5.8). Then one warning when it applies:
   - a mismatch and a party of more than one member: `UserId {user} requested solo/stale-party ISLAND travel; client
     partyId={requested or <missing>} does not match authoritative partyId={id}. Launching for this player only.`
   - else, members excluded: `UserId {user} {mode or undefined} party contains {n} offline member(s); launching with
     {m} active member(s). excluded={comma-joined userIds}`.
8. The join (§6.1) with the resolution's party id, members and revision, the party's global id (only when the
   resolution kept the party), `buildId`, and the client address. `false` is a bare 400; an error the 500 page.
9. The candidate poll (§6.2) for the player. None is the 500 page (`no candidate after a join`); an error the 500
   page. JSON `{"candidateId":..,"candidateStatusPeriodMillis":<§5.1 period at now>,"gameMode":<echo>,"huntId":..,
   "status":"MATCHING","statusReason":null}` (`gameMode` left out when absent), and `x-reforged-instance-id: <game
   session id>` when the candidate's game session id starts with `hunt:` or `hub_instance:`.

The answer is `MATCHING` even when the join cancelled the search (Gauntlet, no Director): the client learns it from
its next status poll.

### 3.4 The candidate leave (B95-B99)

The 1.14.7 client sends `DELETE /candidate/leave` with no body, from the party leader only (FUN_140db6dd0); the
other four shapes are aliases kept until a log shows which one the client sends.

1. Game auth; userId; body read (its failure answered). The body's **key names** are kept for the log: at most the
   first 12 keys of an object body, each cut to 40 characters with any character outside `A-Z a-z 0-9 _ . -`
   replaced by `?`, joined with `,` (no keys for a non-object or absent body). Values are never logged.
2. No database: the 501.
3. The cancel (§6.4), with the party lookup of §7 `for_player`.
4. Success: `{}`, and log `[CandidateLeave] user={user} method={METHOD} path={path} bodyKeys=[{keys}]
   candidate={candidate id or none} state={state} -> {outcome}`. An error: **500 with the JSON body `{}`** (not the
   500 page), and log at error `[CandidateLeave] user=.. method=.. path=.. bodyKeys=[..] candidate=none state=error ->
   error: {error}`.

### 3.5 `GET /candidate/status`

1. Game auth; userId; realm presence touch.
2. The candidate poll (§6.2); an error is the 500 page.
3. The status answer (§3.6) for the client address.
4. When the answer has `serverInfo.gameSessionId` and that id is one of this module's **hub game session ids** (§5.2;
   a ring-assigned `hub_instance:` id is not one), and a store holds the userId's account: record a hub session in the
   module's `HubSessions` (chat spec §5.10) for the account's global id, that game session id, this node's region and
   now. A store error warns `hub session of {user} not recorded: {error}` and changes nothing.
5. The answer's status, with its JSON body, or a bare status when it has none.

### 3.6 The status answer (`candidateStatus.ts`)

**Client address**: the peer address (core's connection info) and the `x-forwarded-for` header. No peer: none. Else
the list is the header's comma-split, trimmed, non-empty entries followed by the peer; from the right, every entry
that is a loopback address (after dropping a leading `::ffff:`, parsed as an IP; `127.0.0.0/8` or `::1`) is skipped,
stopping at the first entry; that entry is the address. This is the backend's own reading for matchmaking, not
auth §4.6's (it keeps entries over 64 bytes and, when all are loopback, answers the leftmost; §15 item 9).

**Body**: an object whose keys, in this order, are `candidateId`, `candidateStatusPeriodMillis`, `gameMode`,
`huntId`, `playerStates` (`{"<userId>":{}}`), then `serverInfo` when travelling, then `status`, `statusDuration`
(`0`), `statusReason`, then `retryAfterSeconds` when given.

The candidate's **destination** is core's hook `candidate_destination(hunt id, realm hub)` (realm hub: the candidate's
source is `REALM_HUB`): its game mode text (`ISLAND`, `CITY`, `SHARED`) is `gameMode`, its hunt id (or null) the
**client hunt id**. The period is §5.1's at now.

| Candidate | Status | Body |
| --- | --- | --- |
| none | 404, bare | log error `UserId {user} was not found in the MatchmakingMap` |
| cancelled | 200 | `huntId` null, `status` `CANCELED`, `statusReason` the candidate's reason or `HOST_ROUTE_FAILED`; reason `REALM_FULL` adds `retryAfterSeconds` `30`. Log `Canceling matchmaking for user {user}: {reason}` |
| not ready | 200 | `huntId` the client hunt id for a realm hub, else the candidate's own hunt id; `MATCHING`, reason null. Log `MM not ready yet!` |
| ready, Ramsgate to verify, hub not verified | 200 | `candidateStatusPeriodMillis` `1000`, `gameMode` `CITY`, `huntId` null, `MATCHING`, reason null. Warn `Ramsgate candidate for {user} is stale (hub not verified as serving) - holding the client in MATCHING` |
| ready (otherwise) | 200 | `huntId` the client hunt id; `serverInfo` `{"buildId":<core build_id()>,"gameSessionId":..,"host":<travel host>,"port":<port>}`; `IN_PROGRESS`, reason null. Log `Telling user {user} to travel to {host}:{port} candidateId={id} local={0|1} verified={realm|1}` |

"Ready" is the client-visible ready (§5.1): a candidate inside its release window still answers `MATCHING`.

**Ramsgate verification** applies when the candidate's hunt id is exactly `ShatteredIsles_ReturnToRamsgate`, it is
not a realm hub, and its game session id does not start with `hub_instance:`. The dedicated servers' `hub_status()`
(core §5.6) must give a Ramsgate hub that is ready; its port is served (a different port warns `Ramsgate port changed
({old} -> {new}); serving the live port`). Without a Director configured the hub is never verified.

**Travel host**: strip a leading `::ffff:` from the client address. The client is **local** when the address and
the candidate's host are both non-empty and either they are equal, or the address is `127.0.0.1` or `::1` and the
candidate is not ring-assigned (game session id starting `hunt:` or `hub_instance:`) or its host is `127.0.0.1`.
Local travels to `127.0.0.1` (a client behind the hub's public address cannot count on NAT hairpin for UDP); else to
the candidate's host.

### 3.7 `POST /party`

Game auth; userId; body read; `buildId` string field (default `""`).

1. The caller's party, created solo when none (§7 `get_or_create`). No database: the 501; no account or another
   failure: the 500 page.
2. The **party instance** (§6.3) for its party id and global id; the caller's **own candidate** (§6.5), else the
   party instance. Errors: the 500 page.
3. For each member in party order, its display name (§7 `username`; a failure as step 1).
4. JSON, header `X-Mystic-Party-Revision: <revision>`:

```json
{"candidateId":<own candidate's id or null>,"candidateState":<state>,"gauntletLevel":null,"leaderPlayerId":<leader>,"partyId":<partyId>,"playerHuntId":<instance hunt id or null>,"playerStates":[{"consoleSessionId":null,"displayName":<name>,"isMemberOfCandidate":<bool>,"platform":"win","playerId":<userId>}, ...]}
```

`candidateState`: `IN_PROGRESS` when there is a party instance and it is client-visibly ready, `MATCHING` when there
is one that is not, else `QUEUED_FOR_START`. `isMemberOfCandidate`: true for every member when there is a party
instance, else only for the leader.

### 3.8 `PUT /party/invite`

Game auth; userId; body read; `buildId` (default `""`). When `recipientPlayerId` is a non-empty string: the caller's
display name, then §7 `invite` (platform `win`); a failure as in §3.7 step 1; log `{user} invited {recipient} to their
party`. Then `{}`. Without a recipient nothing is read or written, so the route answers `{}` even with no database.

### 3.9 `GET /party/invites`

Game auth; userId; §7 `invites_for`; failure as §3.7 step 1. JSON `{"invitations":[{"partyId":..,
"recipientPlayerId":<caller>,"sendingDisplayName":..,"sendingPlatform":..,"sendingPlayerId":..}, ...]}` in the
store's order (newest first).

### 3.10 `PUT /party/invite/accept/:inviterId`

Game auth; userId; body read; `buildId` (default `""`); §7 `accept` (caller, `inviterId` param, build id); failure as
§3.7 step 1. Log `{user} accepted party invite from {inviter}`; `{}` (whether or not anything changed).

### 3.11 `DELETE /party/member`

Game auth; userId; §7 `leave`; failure as §3.7 step 1. Log `{user} left their party`; `{}`.

### 3.12 `DELETE /party/member/:targetId`

Game auth; userId; §7 `kick` (caller, `targetId`); failure as §3.7 step 1. Log `{user} kicked {target} from the
party`; `{}`.

### 3.13 `PUT /party/member/promote/:targetId`

Game auth; userId; §7 `promote`; failure as §3.7 step 1. Log `{user} promoted {target} to party leader`; `{}`.

---

## 4. Reading the caller

### 4.1 Game auth

When the pipeline already put auth's `AuthData` in the request's extensions, that is the caller; else this crate
asks `Shared::auth().game_auth(&mut request)` (auth §4.1) and answers its refusal as it is (empty 401, empty 403, the
500 page). Only `user_id` and `is_gameserver` are used here; a game server key with a token is a player like any other.

### 4.2 The realm presence touch

Only when rifts is loaded (`Rifts::loaded()`): `rifts.entry().touch_presence(<userId as AccountId>)` on a detached
task, never awaited by the request; a failure warns `[Realms] presence touch failed for {account}: {error}`. Done by
B89, B93, B94 (before the body is read), never by the party routes or the leave.

---

## 5. State and pure rules

### 5.1 The candidate (`MatchmakingResult`)

One player's travel attempt: `ready`, `canceled`, `status_reason` (optional), `hunt_id`, `candidate_id`,
`game_session_id`, `host` (default `""`), `port` (integer, default 0), `ready_at` (ms; 0 = no release time),
`source` (`REALM_HUB` or none), `source_session_id` (optional: the hub session a realm hub's status is read from).

- **Matching** (hunt, id): not ready, not cancelled, no reason, that hunt, candidate id = game session id = the id,
  no host, port 0, release 0, no source.
- **Client ready** at `now`: `ready` and (`ready_at` is 0 or `now >= ready_at`). The **client-visible** candidate is
  a copy whose `ready` is the client-ready value. `ready` alone is internal server readiness; the release is the
  synchronised Go Now countdown.
- **Period** at `now`: `1000` while not ready or `ready_at > now`, else `10000` (the client otherwise waits its
  default 10 s before its first poll).
- **Node's object** (what the realms side sends and receives as `rifts_seam::Candidate`'s JSON text), keys in order:
  `Ready` (bool), `Canceled` (only when true, `true`), `StatusReason` (when set), `HuntId`, `CandidateId`,
  `GameSessionId`, `Host`, `Port`, `ReadyAt`, `Source` (`"REALM_HUB"` when set), `SourceSessionId` (when set).
  Reading one: it must be an object with string `HuntId`, `CandidateId` and `GameSessionId` (else it is not a
  candidate); `Ready`/`Canceled` booleans default false; `Host` string default `""`; `Port` and `ReadyAt` default 0
  when absent or not an integer (`7777.0` reads as 0); `Source` is `REALM_HUB` only for exactly that string;
  `StatusReason` and `SourceSessionId` only when strings.

### 5.2 What the module keeps in memory (DD15; one set per module)

| Table | Key | Value |
| --- | --- | --- |
| queues | hunt id | players (in join order, duplicates allowed), time of the last add, resolved (launching) |
| candidates | userId | the candidate |
| party instances | Node `partyId` | the party's shared candidate, and when it was made |
| launch refs | `host:port` | the activity monitor's id of the dedicated hunt running there |
| hub game session ids | `host:port` | a UUID, made once and stable for the life of the hub (the `City-<id>` room) |
| ring instances | player global id | the ring instance global id placed for that player on this node |
| last ring instance | — | the most recent ring instance recorded (any player, or a hub) |

Queries others use: `candidate(userId)` (stored, not client-visible), `has_candidate(userId)` (the erasure blocker
`account has an active matchmaking candidate`, which a cancelled candidate also triggers), `set_candidate`,
`cancel_hunt` (§5.3), `hub_game_session_id(host, port)` (makes one if needed), `is_hub_game_session_id(id)` (any
value equal), `record_ring_instance(instance, players)` (each player mapped to it; the last ring instance set, also
for an empty player list), `ring_instance_for_player(player)` (the player's, else the last ring instance, else none).

Each step between two awaits is one critical section of the tables, so the only interleavings are the ones Node had
between its `await`s.

### 5.3 Cancelling a hunt (`CancelHuntMatchmaking`)

For each distinct player of a list (first occurrences): a new matching candidate for the hunt with a fresh id,
cancelled, with the reason; it replaces the player's candidate. Always answers true (a cancelled search is a taken
join).

### 5.4 The party a join travels with (`ResolveMatchmakingParty`)

Inputs: the mode, the requester, the requested party id, the party (id, members in order, revision), an online test,
and the realms side's "filter offline" flag.

| Case | Party id | Members | Revision | Excluded | Mismatch |
| --- | --- | --- | --- | --- | --- |
| no party | none | none | none | none | false |
| mode `ISLAND` and the requested id is not the party's id (absent included) | none | none | none | every member but the requester | true |
| otherwise | the party's | the requester and, when the mode is `ISLAND` or the flag is set, only online members; else all, in order | the party's | the members not kept | false |

ISLAND requests carry the id of the party the client is actually travelling with, so a stale server party can never
turn a solo or Private Hunt into a two-player server whose airship waits forever. CITY requests omit it (captured in
1.12), so the authoritative party is kept and returning members fan out to the shared hub.

### 5.5 Hunt ids

- **Effective hunt id** (`GetFallbackHuntId`): the hunt id when it is non-blank after trimming; else
  `ShatteredIsles_ReturnToRamsgate` for mode `CITY`; else `ShatteredIsles_IslandA` when the game args contain
  `/Game/Maps/islands/1705/dia_moss_triforce`; else the hunt id as given (a blank one stays as it is, `"  "` stays
  `"  "`), or `""` when absent.
- **Needs a queue**: the effective id contains `CR19`.
- **Hub hunt id**: contains `ReturnToRamsgate` or `TrainingDojo`. **Hub destination**: mode `CITY` or a hub hunt id.
- **Gauntlet**: the id contains `gauntlet` in any ASCII case; refused while `RUSTIC_ISLES_GAUNTLET` is not exactly
  `on` (trimmed, any case), with reason `GAUNTLET_DISABLED`.

### 5.6 The Go Now release (`CreateGoNowReleaseAt`)

A synchronised, load-aware countdown, drawn once per launch (all players of one launch share it):

- per joiner = `GO_NOW_LOAD_PADDING_PER_JOINER_MS` (default 1000), cap = `GO_NOW_LOAD_PADDING_MAX_MS` (20000);
  in flight = the number of stored candidates (all players) whose release is after now; padding = min(in flight ×
  per joiner, cap).
- base: a hub `HUB_GO_NOW_MIN_DELAY_MS` (0) and `HUB_GO_NOW_MAX_DELAY_MS` (1000); a hunt `HUNT_GO_NOW_MIN_DELAY_MS`
  (10000) and `HUNT_GO_NOW_MAX_DELAY_MS` (20000).
- low = min(base min, base max); high = max(low, base max) + padding; release = now + a uniform integer in
  [low, high], both ends included. The draw takes the next replay UUID's 128 bits modulo the span.
- Each delay setting reads as JavaScript `Number` on its trimmed raw value (blank is 0; only digits, `.`, `+`, `-`,
  `e`, `E` are a number, anything else NaN), floored; it must be finite and above 0, else the default. So `0` and
  negatives read as the default (§15 item 6).

Hubs are already-running servers, so their window only spreads a burst of returning players; a hunt's masks the
dedicated server's spin-up.

### 5.7 Timings

| Constant | Value |
| --- | --- |
| Queue size | 4 (a full queue launches at once) |
| Queue wait | 20,000 ms after the last add, checked by the next poll that finds the queue (strictly more than) |
| Party instance reuse | 120,000 ms after it was made (strictly less than) |
| Party member online window | 75,000 ms (2.5 realm presence heartbeats of 30 s; shorter than the 2 min release gate so a member who just closed the game is not frozen into a roster) |
| Candidate poll period | 1000 ms / 10,000 ms (§5.1) |
| Go Now | §5.6 |
| Invite lifetime | 600,000 ms |
| Party size cap | 4 |
| Outbox | poll 250 ms, batch 64, stale claim 30,000 ms (§8) |
| Director request | 300 s to the end of the answer (§10) |
| Realm full retry hint | 30 s |

### 5.8 Game presence (`GameLastSeen`)

Per module, in memory: the last time each userId's running game touched the backend. Touched by B89 (about every 5 s
while matchmaking or in game), the realms' `touch_game_session` (§11), and a game-session start (auth's activity
seam "touch matchmaking presence", auth §11.2, wired by the composer).

- `touch(user, now)`: an empty userId is ignored; the time only moves forward (an older touch keeps the newer); after
  a touch, when more than 100,000 users are tracked, every entry older than 2 h before now is dropped.
- `is_recently_active(user, within, now)`: tracked and `now - last <= within`.
- `recently_active(within, now)`: every such userId (the operator dashboard's online count; the Reforged unit reads it).
- `forget_stale(now)`: drops entries older than 2 h.

### 5.9 The clock

`now` is core's `replay::now_ms()` plus an offset a test moves forward (feature `test-support`); never the system
clock directly. Node's tests mock `Date.now` the same way.

---

## 6. The controller (`controllers/matchmaking.ts`)

### 6.1 The join (`HandlePlayerMatchmaking`): true when taken, false for a 400

1. `MATCHMAKING_MODE` raw: `DISABLED` warns `Matchmaking is disabled, refusing MM!` → false; exactly `DEPLOYSERVER`
   goes on; anything else (unset, empty, other casing) logs error `Unsupported MATCHMAKING_MODE!` → false.
2. The effective hunt id (§5.5); **everyone** = the party members, else the player alone.
3. A refused Gauntlet id: warn `[HuntRoute] Gauntlet hunt {id} refused reason=GAUNTLET_DISABLED (RUSTIC_ISLES_GAUNTLET
   is not on)`; cancel the hunt for everyone with `GAUNTLET_DISABLED` → true. This is before the realms route, so it
   applies on every route.
4. **Realms route**: `route_hunt(HuntRouteContext { game_mode: mode or "", hunt_id: effective, player, party: the
   party global id as a rifts `PartyId` (§11), party_members: the members, party_revision (when it is not negative),
   build_id, is_hub })`. `Handled { result }` → result. A seam error fails the request.
5. **Party** = (party id, members) when there are more than one member.
6. Not a hub destination, effective id non-blank, and a party: the party hunt (§6.6).
7. Effective id blank, or not needing a queue: the immediate instance (§6.7).
8. A party (only a hub destination with a `CR19` id reaches here): the party hunt.
9. Else the queue (§6.8).

### 6.2 The candidate poll (`CheckAndUpdateQueueStatus`)

1. The stored candidate; none: the realms side's `restore_candidate(player)` (a candidate owned by an optional module
   may outlive the process); read as §5.1, stored, and used; nothing: none.
2. The realms side's `candidate_status(player, current)`: a candidate read from it is the answer **as it is** (not
   made client-visible here).
3. When the candidate is neither ready nor cancelled:
   - its candidate id parses as a `GlobalId` of kind `Ticket` (a ring ticket, DN8): `ring_try_form_matches(hunt id,
     game version)` (skipped when core's `game_version` does not parse; answer ignored), then
     `ring_ticket_state(ticket)`: `Claimed { match_id }` with `ring_lookup_instance(match_id)` found: the stored
     candidate, if not cancelled, takes the record's host and port, game session id the match id's text, ready, and
     a hunt release (§5.6); `Cancelled`: not ready, cancelled, reason `PLAYER_CANCELED`; `Searching` or any error:
     nothing;
   - else, when a queue exists for its hunt and more than 20 s passed since its last add: pop it (§6.9). A cancelled
     candidate with no queue behind it has nothing to pop.
4. The stored candidate, client-visible at now (none when gone).

A pop's error fails the poll.

### 6.3 The party instance (`GetPartyInstance`)

The local party instance for the Node party id, client-visible at now; else, when a party global id was given and
converts to a rifts `PartyId`, the realms side's `party_instance(party)` read as §5.1 (not made client-visible); else
none.

### 6.4 The cancel (`CancelMatchmakingForLeave`)

The answer is an **outcome**, a **state** text and the candidate id.

1. The current candidate (stored, else restored as §6.2 step 1). None: `no-candidate`, state `none`, no id.
2. The realms side's `cancel_candidate(player, current)`: `Handled { outcome, state }` is the answer.
3. A hub hunt id (§5.5): `hub-not-cancelled`, state `hub` (a hub candidate is never cancelled).
4. Already cancelled: `already-canceled`, state `canceled` (a second press is a no-op).
5. A ring-ticket candidate id: `ring_cancel_ticket(ticket)`, answer ignored.
6. State: `ready` when client-ready, `go-now` when ready but not yet released, `queued` when the player is in its
   hunt's queue, else `matching`.
7. **Accounts**: the player's party (the lookup; an error fails the request) when it has 2 or more members and its
   local party instance's game session id equals the current candidate's: the player, then every other member whose
   stored candidate is not cancelled and has that game session id; otherwise the player alone.
8. Any of those accounts client-ready: `too-late` with the state (a search already told to travel is left alone).
9. Each account with a stored candidate: not ready, cancelled, reason `PLAYER_CANCELED` for the player and
   `PARTY_MEMBER_CANCELED` for the others, host `""`, port 0, release 0. A queue for the hunt that is not resolved
   drops them (removed when it empties); a resolved queue is already launching and its pop skips cancelled players.
   The party's local instance (when the party was used) is removed. Log `Search for {hunt} cancelled by its player:
   {n} account(s), state={state}` → `canceled` with the state.

`already-ended` is an outcome only the realms side gives.

### 6.5 The player's own candidate (`GetPlayerCandidate`)

The stored candidate client-visible at now, else the candidate poll (§6.2).

### 6.6 The party hunt (`HandlePartyMatchmaking`)

A whole party joining one hunt lands in one instance.

1. **Reuse**: a local party instance for the party id with the same hunt id, made less than 120 s ago: the player
   gets a copy with a fresh candidate id (the destination is party-wide; a candidate is one client's travel attempt);
   the hunt's launch ref is the one stored for its `host:port`, else the `host:port` text itself; the activity
   seam's hunt start for `[player]` with the hunt and that ref → true. A different hunt or a stale instance launches
   anew.
2. Launch (§6.10) with mode `ISLAND`, args `""`, the hunt, the members as expected players. `Refused` with a reason:
   cancel the hunt for the members with it → true; `Refused` with none → false; an error fails the request.
3. `Online { host, port }`: register a ring hunt instance (§6.11) for the members. The shared candidate: matching with
   a fresh id, game session id the ring instance's text when registered (else the candidate id), ready, the host and
   port, a hunt release. Log `Party {party id} hunt '{hunt}' is online; synchronized Go Now release in {ms}ms for {n}
   member(s)`. It is stored as the party instance (made now); each member gets a copy with a fresh candidate id → true.

### 6.7 The immediate instance (hubs, and hunts that need no queue)

1. **Ring Ramsgate (DN9)**: a hub destination whose effective id is exactly `ShatteredIsles_ReturnToRamsgate`, with a
   client address, a resolved account and a parsable game version, and `ring_assign_ramsgate(account, address, game)`
   answering a record: the record's host and port, and the record id as the ring instance.
2. Otherwise launch (§6.10) with the request's mode and args, the effective id, everyone. `Refused` with a reason:
   cancel the effective hunt for everyone → true; with none → false. `Online`: for a hunt (not a hub), register a ring
   hunt instance for everyone.
3. A hub: the activity seam's hunt end for everyone with `RETURNED` (travelling to a hub ends the party's dedicated
   hunts, completed or abandoned, once each).
4. The shared candidate: matching with a fresh id; game session id the ring instance's text when there is one, else
   for a hub the hub game session id of `host:port` (§5.2), else the candidate id; ready, host, port, and a release of
   the hub or hunt kind. Log `{Hub|Hunt} '{id}' is online; synchronized Go Now release in {ms}ms candidateId={id}
   gameSessionId={id}`.
5. With a party (more than one member): stored as the party instance (made now) and each member gets a copy with a
   fresh candidate id, so all follow the leader. Without: the request's party id (if any) loses its party instance,
   and the player gets the shared candidate itself → true.

### 6.8 The queue (`QueuePlayer`)

1. **Ring ticket (DN8)**: with a client address, a resolved account and a parsable game version:
   `ring_submit_ticket(account, hunt, game, 1, address)`. Answered: `ring_try_form_matches(hunt, game)` (ignored); the
   player's candidate is matching with the ticket id's text as candidate id; when `ring_ticket_state` is already
   `Claimed` and its instance is found, it takes the record's host and port, game session id the match id, ready, a
   hunt release → true. `NotInNetwork`: on. Another error: warn `ring_submit_ticket for {player} failed ({error});
   using local queue`; on. Without a client address the hunt stays local (the hosting node could not admit the
   player).
2. **Local queue**: none for the hunt: a new one with the player. An unresolved one: the player appended (again if
   already there), the last add now; full at 4 or more. A resolved one (launching): false.
3. A full queue is popped (§6.9) **before** the player's candidate is made.
4. The player's candidate: matching with a fresh id → true. Because of step 3, the fourth player's candidate is made
   after the launch and waits on a queue that no longer exists (kept as Node does, §15 item 3).

### 6.9 Popping a queue (`PopQueue`)

No queue or already resolved: nothing. Mark it resolved; launch (§6.10) with `ISLAND`, `""`, the hunt and the queued
players. `Online`: one hunt release for all; log `Hunt '{hunt}' is online; synchronized Go Now release in {ms}ms for
{n} queued player(s)`; each queued player with a stored candidate that is not cancelled takes host, port, ready and
the release; the queue is removed. `Refused`: the players are cancelled with the reason, else `DEDICATED_HUNT_DISABLED`;
the queue is removed. An error propagates and leaves the queue resolved (§15 item 4).

### 6.10 Launching (`LaunchGameOnDeployserver`)

1. No Director configured (core §5.6 `configured()` false): warn `[HuntRoute] no dedicated server (DEPLOYSERVER_URL
   is unset): refused {hub|hunt} huntId={id}` → `Refused` with `NO_DEDICATED_SERVERS` (no request is made, for a hub
   or a hunt alike).
2. A hunt (not a hub destination) while the realms side says dedicated hunts are disabled: error `[HuntRoute] blocked
   dedicated hunt launch (dedicated hunts are disabled) huntId={id}` → `Refused` with no reason. A seam error fails.
3. Log `Querying DeployServer for GameMode: {mode or undefined} HuntId {id} with {n} Expected Players!`; core's
   dedicated servers `launch(LaunchRequest { game_mode, game_args, hunt_id, expected_players, request_id: the
   request's id })`. An error fails the request (Express's 500).
4. `Online` for a hunt: a fresh launch ref stored for `host:port`, and the activity seam's hunt start for the
   expected players with the hunt and the ref (the launch never waits for the activity rows).

### 6.11 Registering a ring hunt instance

Only with a parsable game version. The players that resolve to accounts (others are left out); a new `GlobalId` of
kind `Hunt` on this node; `ring_register_instance(InstanceRecord { id, region: this node's, game, queue: the hunt,
server_host, server_port (the port as u16, else 7777), capacity 4, players, shard none, created_at_ms now })`. Success
records it as the ring instance of those players (§5.2) and gives the id; any error (`NotInNetwork` included) gives
none, silently.

---

## 7. Parties (`controllers/party.ts`, store spec §11.2)

Membership, revisions and the outbox live in the database under global ids (DD18); the client keeps seeing Node's
`partyId` and userIds. Every operation needs a store: without one it is **no database** (the routes' 501).

- **Party id** (`MakePartyId`): chat's room spelling of the party's global id (chat spec §4.3, `ids::room_id_text`),
  `_`, then the build id in standard padded base64 (`<ULID>.oregon-1_NjQ3NDcy` for `647472`; an empty build id leaves
  a trailing `_`). A new party's ids are a new `GlobalId` of kind `Party` on this node and that text (`NewParty`).
- **Account of a userId**: `accounts.find_by_user_id`; "requires an account" fails with `{userId} has no account`.
- **Writes** are `account_write` on the acting account (store §3.4's home check) with actor `Player { account }`
  carrying the request id when there is one; a write that fails with an error whose text contains
  `PARTY_REVISION_CONFLICT` is retried, three attempts in all, and the third conflict is that error.
- **The member-left hook**: after a leave or a kick, `party_member_left(<target userId>)` on the realms side, so a
  forming player-hosted or realm hunt stops waiting for him; a failure warns `[PartyLeave] could not update the
  forming hunt of {user}: {error}` and never fails the change. Not called by the realms' own calls into the core.

| Operation | Contract |
| --- | --- |
| `for_player(user)` | No account: none. Else `parties.find_by_member` |
| `get_or_create(user, build)` | Requires an account; `parties.get_or_create_solo(account, build, new party, now ISO)` |
| `invite(sender, recipient, build, name, platform)` | Empty recipient or the sender itself: nothing. Requires the sender's account; a recipient with no account: nothing (no one could ever read it). Else `parties.create_invite(NewInvite { sender, recipient, build, name, platform, new party, now, expires now + 10 min })` |
| `invites_for(user)` | No account: empty. Else `parties.list_pending_invites(account, now)`, each as party id, the sender's userId, display name and platform |
| `accept(recipient, inviter, build)` | Requires the recipient's account; an inviter with no account: the recipient's party as `for_player` (nothing written). Else `parties.accept_invite(AcceptInvite { recipient, inviter, build, new party, now, max 4 })` |
| `leave(user)` | Requires the account; `parties.leave`; then the member-left hook for the user |
| `kick(actor, target)` | Requires the actor's account; a target with an account: `parties.kick` (the store does nothing unless the actor leads a party the target is in); then **always** the member-left hook for the target text (§15 item 8) |
| `promote(actor, target)` | Requires the actor's account; a target with no account: nothing; else `parties.promote` |
| `username(user)` | The launcher account's display name (store `launcher_accounts.find_by_account`) when found and non-empty (a lookup failure falls back), else the game account's name, else the userId text itself |

The offline hook (core §7.7, a chat connection gone): the player leaves their party (`leave`, no request id; errors
ignored).

---

## 8. The party outbox dispatcher (`PartyOutboxDispatcher.ts`)

A start-up task (only with a store) that publishes the party outbox as headlines. Node wrote a `<message
type="headline">` to each recipient's connections; the connections are the chat plugin's (DD21), so the module emits
`module_api::chat::HEADLINE_EVENT` (`isles.party.headline`) with `Headline { id, recipients, body }` and the plugin
delivers it (chat spec §4.4, §13 item 3). Evicting a leaving member from the party room is the plugin's too.

- **Owner**: a system-random UUID v4 per dispatcher start (not a replay id).
- **A pass** (up to 64 events): `claim_next_outbox(owner, now ISO, (now - 30 s) ISO)` as a system write; none ends the
  pass. The headline: recipients are the event's recipient userIds that have a live account (`accounts.
  find_by_user_id`), each as `HeadlineRecipient { account: <global id>, user_id }`, in recipient order; the body is
  chat's `headline::public_event(event id, event type text, created_at, party id, party revision, payload)` as JSON
  text; the id is the event id. Emit it (the payload is the serialised `Headline`). Success: `mark_outbox_published
  (event, owner, now ISO)` (its answer ignored) and info `[PartyOutbox] {EVENT_TYPE} party={party id}
  revision={revision}`. A failure building or emitting it: `release_outbox(event, owner)`, error `[PartyOutbox]
  publish failed event={id}: {error}`, and the pass stops (Node's `break`), so order is kept.
- A claim older than 30 s is taken over by the next pass, so a crash between claim and publish loses nothing.
- **The loop**: a pass, then 250 ms, until stopped (the stop ends the wait at once). A store error in a pass is a
  failed pass: the first failure and every 120th consecutive one log error `[PartyOutbox] database polling failed;
  dispatcher will retry ({n}): {error}` (Node: the first, then one each 30 s); the first success after failures logs
  info `[PartyOutbox] database polling recovered after {n} failed attempt(s)`.
- It is one of the area's start-ups (core §11.4): it returns its stoppable task.

---

## 9. The QoS pong listener (`server.ts`, `listenerPlan.ts`)

The client measures regions by pinging each `regionUrls` entry of B90 over **plain HTTP on TCP** (not UDP) and expects
the pong on every path. Nothing answering leaves every region `9999ms Invalid` and the client travelling with no
region at all. Node opened it on every interface at `QOS_PORT` (80); a game module opens no public listener
(AGENTS.md), so the module answers on loopback and announces the port.

- **Plan** from `QOS_PORT` (the env file, else the switch `qos_port`, else the version's switch default): blank or
  unset is the default 80; `off` (any case) or `0` starts none (`0` is the host kit's spelling, written under
  DaintyLass, where `QOS_TARGET_URL` points at the launcher's relay, which answers the pings on the player's machine);
  digits only from 1 to 65535 are that public port; anything else starts none with the problem `QOS_PORT='{value}' is
  not a port number or 'off'` (trimmed value; not fatal).
- **Start-up**: none planned: warn `QoS listener not started: {problem}`, or info `QoS latency-test listener off
  (QOS_PORT=off); the relay or a proxy answers the region pings`. Else bind `127.0.0.1` on the module's `qos` port
  (the config's assigned range's first port, else the spec default, 61010 for 1.14.7; 0 in tests); a bind failure
  warns `QoS listener could not start: {error}` and the module goes on (the game is playable without a latency
  measurement). Serve with a 1 s drain; info `QoS latency-test listener on {address} for public port {port} (answers
  pong for region pings)`; a serve failure warns `QoS listener failed: {error}`.
- **Answers**: every method and path: 200, `content-type: text/html` (no charset, unlike B100), the pong.
- The start-up returns the listener with the recorded addresses `qos` (the bound address) and `qos_public_port` (the
  public port as text), which core records after the services (core §7.3 step 7).

Publishing the public port is the gateway's decision, not this crate's (restricted's `docs/security/qos.md`).

---

## 10. The Director over HTTP (`DEPLOYSERVER_URL`)

When the Director does not run in-process (director spec §12.3 puts its own `InProcessDirector` in the slot), the
composer puts this crate's HTTP client of core's `DedicatedServers` (core §5.6) in the slot. `DEPLOYSERVER_URL`
unset or blank after trimming: core's `Unconfigured`. Else the value is an **authority** `host[:port]` (no scheme;
Node fetched `http://${DEPLOYSERVER_URL}<path>`), port 80 when it has no `:`.

- `configured()`: true.
- **A request**: one TCP connection to the authority; `HTTP/1.1` with `Host: <authority>`, `Connection: close`, the
  given headers, and for a body `Content-Length` and the body; the answer read to the end of the connection, the
  whole exchange within 300 s. The status is the second space-separated field of the first line; a
  `transfer-encoding` header containing `chunked` (any case) is de-chunked (hex sizes, extensions after `;` ignored,
  ending at a 0 chunk or a short one). Failures are texts `fetch failed: {io error}`, `fetch failed: timed out`,
  `fetch failed: a truncated answer` (no blank line), `fetch failed: no status line`, `fetch failed: a bad chunk`.
- `launch(request)`: `POST /api/matchmaker/handle-matchmaking-for-player` (D4), headers `Content-Type:
  application/json` and `x-rustic-isles-request-id: <id>` when there is one, body core's `LaunchRequest::body()`.
  A request failure is the error. A status other than 200: error log `DeployServer returned status {n}` → `Refused`
  with no reason. A body that is not JSON: the error `invalid DeployServer JSON: {e}`. Else `Online { host: the
  string "host" or "", port: the integer "port" or 0 }`, info `DeployServer returned gameserver {host}:{port}`.
- `hub_status()`: `GET /api/hub-status` (D2), no extra headers. 200 with JSON: `{ ramsgate: <hub of "ramsgate">,
  training_dojo: <hub of "trainingDojo"> }`, a hub being an object read as `ready` (bool, default false), `host`
  (default `""`), `port` (integer, default 0), anything else none. A non-200 warns `hub-status returned {n}; treating
  cached hub candidates as unverified`; a failure or bad JSON warns `hub-status unreachable ({e}); treating cached hub
  candidates as unverified`; both answer none.

---

## 11. The rifts `IslesCore` (RF1 §2.2)

`MatchmakingCore { store, node, state, fallback }` implements dainty-lass's `rifts_seam::IslesCore`. The parts that
are matchmaking's are answered here; the rest goes to `fallback` (the composer's core for accounts, Launcher Guard and
operations: auth's today, the Reforged unit's later). Accounts cross as their userId text. A **refusal** is
`SeamError::Refused(Refusal { code, message })`.

**Party ids across the seam**: rifts names a party `<ULID>@<node>` (`PartyId`); isles keys it `party:<ULID>@<node>`.
Isles to rifts: the global id text after its first `:` parsed as a `PartyId` (none when it does not parse). Rifts to
isles: `party:` + the text parsed as a `GlobalId`.

| Method | Answer |
| --- | --- |
| `display_name`, `find_by_display_name`, `launcher_account_eligible`, `record_hunt_activity`, `healthy_guard_session`, `published_guard_artifacts`, `guard_mode`, `operations_policy`, `verify_operations_step_up` | the fallback's |
| `host_grant_catalog`, `stacked_items`, `accepted_inventory_replay` (trait defaults in dainty-lass) | the fallback's (§15 item 11) |
| `party(id)` | An id that does not convert: `Ok(None)` (before any store check). No store: refusal `NO_DATABASE` `no database is configured`. Else `parties.find_by_id`; a store error is refusal `PARTY_READ_FAILED` with its text; found: the seam party |
| `get_or_create_party(account)` | §7 `get_or_create` with build id `""` (the realms side does not know the client's build; the party keeps it blank until a client names one); any error, no database included, is refusal `PARTY_WRITE_FAILED` with its text; the seam party |
| `set_candidate(account, candidate)` | The JSON text must parse (else refusal `INVALID_CANDIDATE` `a candidate is a JSON object: {e}`) and read as §5.1 (else `INVALID_CANDIDATE` `a candidate has HuntId, CandidateId and GameSessionId`); stored for the userId |
| `cancel_travel(hunt, accounts, reason)` | §5.3 → `Ok(true)` |
| `dedicated_servers_configured()` | core's dedicated servers `configured()` |
| `touch_game_session(account)` | §5.8 touch at now → `Ok(())` |

**The seam party**: id from the global id's ULID and owner (`<ULID>@<node>`; one that does not parse is refusal
`INVALID_PARTY` `a party id is not a ULID and a node`), leader and members as userIds, revision (negative: 0).

The realms hooks this crate calls (`Rifts::realms()`): `dedicated_hunts_disabled`, `filters_offline_party_members`,
`route_hunt`, `restore_candidate`, `candidate_status`, `cancel_candidate`, `party_instance`, `party_member_left`; and
`Rifts::entry().touch_presence`. Under `Rifts::dedicated_only()` they answer: not disabled, no filter, not handled,
none, none, not handled, none, nothing.

---

## 12. The Director invocation and ring recording (core §7.8)

Core answers `DirectorInvocation`s (DN8, DN9) and asks the composer's `ModuleHooks` for two things this crate supplies:

- **Players of a placed hunt** (`place_hunt_players`): each player global id whose owner is this node and that a store
  finds as an account (`accounts.find`) becomes its userId; any other stays its global id text. Order kept.
- **Ring recording**: `record_ring_instance(instance, players)` of §5.2: `PlaceHunt` that came `Online` records the
  match id for its players; `PlaceHub` (a ready live Ramsgate answered at once, or a `CITY` launch `Online`) records
  the hub id with no players, which makes it the last ring instance (§15 item 10).

The rest of the invocation (the launch, `PlacedInstance`, the port outside u16 as 7777, the refusal texts) is core's
contract (core §7.8) over the same `DedicatedServers` slot.

---

## 13. Settings

Read with core's `raw` (the value as set, empty included). None is a secret.

| Setting | Reading | Default |
| --- | --- | --- |
| `MATCHMAKING_MODE` | exactly `DEPLOYSERVER` matches; `DISABLED` refuses with its warning; anything else refuses as unsupported | unset (refuses) |
| `DEPLOYSERVER_URL` | §10 (composer's choice of slot) | unset (no Director over HTTP) |
| `QOS_TARGET_URL` | B90's `regionUrls[0]` | unset (`null`) |
| `QOS_PORT` (switch `qos_port`) | §9 | 1.14.7 switch default `80` |
| `HUNT_GO_NOW_MIN_DELAY_MS`, `HUNT_GO_NOW_MAX_DELAY_MS` | §5.6 | 10000, 20000 |
| `HUB_GO_NOW_MIN_DELAY_MS`, `HUB_GO_NOW_MAX_DELAY_MS` | §5.6 | 0, 1000 |
| `GO_NOW_LOAD_PADDING_PER_JOINER_MS`, `GO_NOW_LOAD_PADDING_MAX_MS` | §5.6 | 1000, 20000 |
| `RUSTIC_ISLES_GAUNTLET` | `on` (trimmed, any case) allows Gauntlet hunts | off |

No legacy names (core §17 item 2). These are the env file's names the host kit already writes; nothing new is added,
so `hostconfig.rs` and the hosting docs are untouched by this row.

---

## 14. The public seam

### 14.1 Shape

One crate, `rustic-isles-matchmaking` (lib), MIT, in dainty-lass `crates/rustic-isles-matchmaking`. Depends on
dainty-lass MIT crates `rustic-isles` (core: `Shared`, `Handlers`, `Call`, `read_body`, `not_implemented`,
`DedicatedServers` and its types, `Settings`, `replay`, `season::to_iso_string`, `Presence`, slots, `Composer`'s
start-up and hook types), `rustic-isles-store` (`IslesStore`, `repo::parties`, the account repositories,
`RepoError`), `rustic-isles-auth` (`AuthData`), `rustic-isles-chat` (`ids::room_id_text`, `headline::public_event`,
`HubSessions`, the re-exported `Headline` types), `rifts-seam`, `module-api` (`GlobalId`, `IdKind`, `NodeId`,
`ActorContext`, the `ring_*` home calls and records, `chat::HEADLINE_EVENT`, `Event`), `service-kit` (`serve`); and
workspace `axum`, `http`, `tokio`, `tracing`, `serde`, `serde_json` (own `preserve_order`), `uuid`, `thiserror`. It
does not depend on economy: the "items" dependency of core §16 row 8 was `multipliers.rs`, which is economy's (§1.2).

**Seam changes for the integration pass** (not this row's): `crates/rustic-isles-matchmaking` and
`rustic-isles-matchmaking` in `[workspace.dependencies]`; the fuzz targets' entries in `fuzz/Cargo.toml`.

### 14.2 Interface (names are the contract the composer and the later units are written against)

| Item | Contract |
| --- | --- |
| `pub fn register(handlers: &mut Handlers)` | The 18 handlers of §3, paths exactly as core's rows |
| `pub struct MatchmakingState` | §5.2, §5.8, §5.9: one per module, in a core slot. `new()`; `clock()` (`now_ms`, and `advance(ms)` under `test-support`); `presence() -> &GamePresence`; `candidate`, `has_candidate`, `set_candidate`, `cancel_hunt`, `hub_game_session_id`, `is_hub_game_session_id`, `record_ring_instance`, `ring_instance_for_player`; `set_hunt_activity(Arc<dyn HuntActivity>)` |
| `pub struct MatchCandidate` | §5.1: the fields, `matching`, `is_client_ready`, `client_visible`, `status_period_millis`, `to_json`, `from_json` |
| `pub struct GamePresence` | §5.8: `touch`, `is_recently_active`, `recently_active`, `forget_stale` |
| `pub trait HuntActivity: Send + Sync` + `NoHuntActivity` | `dedicated_hunt_starts(users, hunt_id, hunt_ref)`, `dedicated_hunt_ends(users, end_reason)`; synchronous, never awaited by a launch; the Reforged unit implements it over the activity rows |
| `pub struct MatchmakingCore` | §11: `new(store: Option<IslesStore>, node: NodeId, state: Arc<MatchmakingState>, fallback: Arc<dyn IslesCore>)`; implements `rifts_seam::IslesCore` |
| `pub struct HttpDirector` | §10: `from_settings(&Settings) -> Arc<dyn DedicatedServers>` (core's `Unconfigured` when blank); implements core's `DedicatedServers`; `Debug` prints the authority only |
| `pub fn startups() -> Vec<Startup>` | §8 (only with a store) and §9, in that order of registration: QoS, then the outbox |
| `pub fn place_hunt_players(shared, players) -> BoxFuture<Vec<String>>`, `pub fn record_ring_instance(shared, instance, players)`, `pub fn player_offline(shared, user) -> BoxFuture<()>` | §12 and §7's offline hook, for the composer's `ModuleHooks` |
| Pure helpers | `make_party_id(&GlobalId, build_id)`, `resolve_matchmaking_party(..) -> PartyResolution`, `effective_hunt_id(mode, args, hunt)`, `is_hub_hunt_id`, `is_gauntlet_hunt_id`, `gauntlet_enabled(&Settings)`, `client_address(peer, forwarded)`, `qos_plan(raw, default) -> QosPlan { port, problem }`, `QOS_PONG` |
| Constants | `RAMSGATE_HUNT`, `NO_DEDICATED_SERVERS_REASON`, `GAUNTLET_DISABLED_REASON`, `REALM_FULL_RETRY_AFTER_SECONDS`, `INVITE_TTL_MS`, `MAX_PARTY_SIZE`, `PLATFORM` (`win`), `DEPLOYSERVER_MATCHMAKING_PATH` |
| feature `test-support` | A fake `DedicatedServers` (online at a host and port, refusing, or failing with an error; a settable Ramsgate hub; counts its hub-status asks), a bare `Shared` with `MATCHMAKING_MODE=DEPLOYSERVER` and the 1.14.7 destination hook, and a stand-in auth layer naming the user by a test header |

### 14.3 How it plugs into core

- **Routes**: the composer calls `register`; every path matches a core row (core §5.7).
- **State**: the composer makes one `MatchmakingState` per module and puts it, the module's `HubSessions` (chat) and
  the store in core slots; the handlers ask for them. Chat presence is core's (§5.5), read directly.
- **Director**: core's dedicated-servers slot: `HttpDirector::from_settings` at compose time, replaced by the
  in-process Director when it runs (director spec §12.3).
- **Rifts**: the composer builds `MatchmakingCore` over the fallback core and gives it to the version's rifts
  composition.
- **Hooks**: `place_hunt_players`, ring recording and `player_offline` into `ModuleHooks`.
- **What others call**: auth's activity seam "touch matchmaking presence" → `GamePresence::touch` (wired by the
  Reforged unit); auth's erasure blockers → `has_candidate`; the Reforged dashboard → `recently_active`; economy's
  foreign-player grants → `ring_instance_for_player` (§15 item 12 on where that type lives).

---

## 15. Open questions and behaviour that looks wrong but is kept

Parity unless a decision says otherwise; each is a candidate decision row for the maintainer.

1. **`X-Mystic-Party-Revision`** is Node's own header name (Rustic Isles' `routes/party.ts`), not one of the
   `x-mysticparadox-*` copies core §17 item 2 drops; the goldens and restricted's shadow replay compare it, and no
   client reads it. Kept as a wire fact. Renaming it (`x-rustic-isles-party-revision`) is a separate decision.
2. **The leave's error is 500 with JSON `{}`**, not the 500 page every other failure gives. Kept.
3. **The fourth player of a full queue** pops it before getting a candidate, so that player never hears of the server
   and waits on a queue that no longer exists (Node's order). Kept.
4. **A launch that throws during a pop** leaves the queue marked resolved for ever: every later join of that hunt
   gets a bare 400 until the process restarts. Kept; proposed fix (clear `resolved` on error) for a decision.
5. **A player may sit in one queue twice** (a second join appends again); cancel de-duplicates. Kept.
6. **A delay of `0`** (and any non-positive value) reads as its default, so `HUNT_GO_NOW_MIN_DELAY_MS=0` means 10 s and
   `HUB_GO_NOW_MIN_DELAY_MS=0` is the default anyway. Kept (`ReadDelayMs`). Hex and `Infinity` texts read as NaN here
   where Node reads `0x10` as 16; only the default applies either way except for hex.
7. **No database answers 501** on the join, the leave and every party route, but `PUT /party/invite` without a
   recipient answers `{}` and the status route works without one. Kept.
8. **A kick by anyone runs the member-left hook for the target** even when the store changed nothing (not the leader,
   not a member, no such account): any signed-in player can make the realms side drop another player from a forming
   player-hosted hunt. A security weak spot (§16). Proposed: run the hook only when the kick removed the target, a
   departure; decision wanted. The implementer keeps today's behaviour unless the coordinator says otherwise.
9. **Two client-address readings**: matchmaking's (§3.6) keeps `x-forwarded-for` entries over 64 bytes and answers the
   leftmost when every entry is loopback; auth §4.6 drops long entries and answers the peer. They agree for every
   real proxy chain. Kept; moving matchmaking onto auth's `client_ip` would be a small departure.
10. **A placed hub becomes the "last ring instance"**, which economy's foreign-player grants use as a fallback hunt
    instance. Kept.
11. **The three defaulted `IslesCore` methods** (`host_grant_catalog`, `stacked_items`, `accepted_inventory_replay`):
    restricted's matchmaking core does not forward them, so they answer the trait defaults even when the fallback
    implements them. Proposed: forward to the fallback (same answer today; correct once economy implements them).
12. **Where the ring-instance map lives.** Economy (row 7, earlier) reads `ring_instance_for_player` and resolves
    foreign accounts, and core §5.7 says a shared type is the lower unit's. Proposed: economy (or core) defines a
    small `RingInstances` slot type with §5.2's two calls and this crate writes it; until then this crate keeps it
    and economy reads it through the composer. Decision wanted before economy's spec closes.
13. **Raw client strings in the join log** (mode, args, hunt id; the shapes are cut to 40 characters): a client can
    put newlines or very long text into the backend log. Kept for parity; proposed: escape control characters and cut
    each to 120 characters.
14. **`MATCHMAKING_MODE`, `DEPLOYSERVER_URL`, `QOS_*`, `GO_NOW_*`, `HUB_*`, `HUNT_*` lack a server prefix**
    (AGENTS.md). They are Node's names and the host kit already writes them; kept, as chat §13 item 21.
15. **A realms answer is passed through as it is**: `candidate_status` and `party_instance` from the realms side are
    not made client-visible, unlike local candidates. Kept (the realms side owns their release).
16. **The HTTP client reads the Director's answer to the end without a size cap.** The Director is on loopback and
    trusted; kept. A cap would be a departure.
17. **The hub-session record only fires for a dedicated hub's stable id**; a ring-assigned `hub_instance:` Ramsgate
    never opens the region room through this path. Kept.
18. **`AuthData` from a session ticket** (auth §4.1 step 3) names the account by its global id text, so the same
    player can appear under two keys (userId and global id) in the in-memory tables across nodes. Kept (DN5).

---

## 16. Security behaviour (for `docs/security/rustic-isles-matchmaking.md` and RX3)

This is a trust-boundary component: it parses client JSON and headers, the realms side's candidate JSON, the
Director's HTTP answers, and it runs a listener.

- **Identity is the token's.** Every route but B100 authenticates through auth's game auth; the caller's own userId
  keys every read and write. `:candidateId` is ignored; a party route acts on the caller's party only; the store
  enforces leadership for kick and promote and the home for every party write.
- **The cancel is authoritative**: a leave cancels the caller's own candidate and only those party members whose
  candidate is the same forming instance; a candidate already told to travel is never touched.
- **The client's party id is not trusted**: an ISLAND join that names a party id other than the stored party travels
  solo (§5.4).
- **Listeners**: the QoS pong binds `127.0.0.1` only and answers a fixed body; publishing it is the gateway's. The
  backend routes are core's (127.0.0.1).
- **Secrets**: the session ticket of a ring redirect goes only to the caller (body and `x-reforged-session-ticket`)
  and is never logged; no setting here is secret; `HttpDirector`'s and the state's `Debug` hold no token; the
  `AuthData` claims are never logged. The leave logs body key names only, sanitised, never values.
- **Bounded inputs**: bodies through core's reader (100 KiB); the leave's logged keys at most 12 × 40 characters;
  queue growth is bounded only by joins (one entry per join); presence capped at 100,000 tracked users with a 2 h
  prune; hub session ids grow with distinct `host:port` (Director-chosen).
- **Known weak spots to list**: §15 items 4, 8, 13, 16; the outbox holds recipients' userIds in the event bus payload
  (the chat plugin's to deliver, chat §13 item 3); the in-memory tables are lost on restart (a candidate mid-search is
  gone; the realms side restores its own).

**Fuzz targets (`fuzz/fuzz_targets/`):**

- `rustic_isles_matchmaking_inputs`: arbitrary bytes as a join body (through `serde_json`), as an `x-forwarded-for`
  value with an arbitrary peer, as a candidate JSON text (§5.1 reading), as a `QOS_PORT` value and as a build id for
  `make_party_id`: never panics; a candidate that reads writes back and reads again to the same value; the client
  address is always one of the inputs' entries; `qos_plan` gives a port in 1-65535 or none; the party id always
  starts with the room spelling and `_`, and its suffix is valid padded base64.
- `rustic_isles_matchmaking_director_answer`: arbitrary bytes as an HTTP/1.1 answer for §10's reader: never panics,
  never reads past the input, and a de-chunked body is never longer than the input.

---

## 17. Test cases

### 17.1 What may be copied

Nothing recorded: `tests/golden/*` stays in restricted and is replayed there. Restricted's tests are authored code,
described here only. The values below are this spec's own.

### 17.2 Behaviour the authored tests fix

**Pure rules**

| Area | Case | Expected |
| --- | --- | --- |
| Candidate JSON | ready, host `203.0.113.10`, port 7777, release 5, realm hub with session `s` | `{"Ready":true,"HuntId":..,"CandidateId":"c1","GameSessionId":"c1","Host":"203.0.113.10","Port":7777,"ReadyAt":5,"Source":"REALM_HUB","SourceSessionId":"s"}`; reads back equal; `{"Ready":true}` is not a candidate |
| Period | not ready; ready with release 200 at 100; at 200; release 0 | 1000; 1000; 10000; 10000 |
| Hub ids | the same `host:port` twice; another port; a hunt id | equal, 36 characters; different; not a hub id |
| Cancel hunt | players `a`, `b`, `a` with reason `WHY` | each once, cancelled, not ready, reason `WHY`, game session id = candidate id, distinct ids |
| Effective hunt id | `CITY` none; `CITY` `"  "`; `ISLAND` `X`; args with `dia_moss_triforce`; `ISLAND` `"  "`; nothing | Ramsgate; Ramsgate; `X`; `ShatteredIsles_IslandA`; `"  "`; `""` |
| Hubs | `CITY` + anything; `ISLAND` + `ShatteredIsles_TrainingDojo`; `ShatteredIsles_IslandA`; `CR19_PlayerHunt_X` | hub; hub; not; needs a queue |
| Party resolution | §5.4 with party `party-live` (`leader`, `online-member`, `offline-member`, revision 7): ISLAND with the id; ISLAND without; CITY offline; no party; `SHARED` with and without the flag; ISLAND with the flag and no id | `[leader, online-member]` excl. `offline-member`; solo, mismatch, both others excluded; all kept; solo, no mismatch; all / filtered; mismatch |
| Gauntlet | `on`, ` ON `, `On` / ``, `off`, `true`, `1`, `yes`, `onn`; ids with `Gauntlet` / `gauntlet` vs `Arena_Elite`, `Trials_Island_01`, Ramsgate | on / off; Gauntlet / not |
| Delays | ` 12 `, ``, `abc`, `inf`, `1e3`; `2500.9` (default 5), `0`, `-3`, `x`, unset | 12, 0, NaN, NaN, 1000; 2500, 5, 5, 5, 5 |
| Draw | 200 draws in [10, 15); [4, 4) | in range; 4 |
| Presence | touch at 1000 then 500; window 75000 at 76000 / 76001; empty id; forget after 2 h | active / not; the newer kept; ignored; empty |
| Client address | peer `203.0.113.7` + XFF `198.51.100.1`; loopback peer + `198.51.100.1`; + `10.0.0.1, 198.51.100.1`; loopback, no XFF; `::1` + `127.0.0.1`; no peer | `203.0.113.7`; `198.51.100.1`; `198.51.100.1`; `127.0.0.1`; `127.0.0.1`; none |
| Party id | node `oregon-1`, build `647472`; empty build; base64 of ``, `f`, `fo`, `foo`, `foobar`, `392819_1.12.0` | `<ULID>.oregon-1_NjQ3NDcy`, read back by chat's `instance_id` as the party; ends `_`; ``, `Zg==`, `Zm8=`, `Zm9v`, `Zm9vYmFy`, `MzkyODE5XzEuMTIuMA==` |
| QoS plan | unset; `  `; `OFF`; `8080`; `0`; `70000`; `http`; default 8080 unset | 80; 80; none; 8080; none; none + problem text; none + problem; 8080 |
| Launch body | `ISLAND`, no args, `ShatteredIsles_IslandA`, `[a,b]` | `{"GameMode":"ISLAND","HuntId":"ShatteredIsles_IslandA","ExpectedPlayers":["a","b"]}` |
| Director answer | plain `200` with `{}`; chunked `500` `3 abc 2 de 0`; `HTTP/1.1 200` with no blank line | (200, `{}`); (500, `abcde`); error |
| Director URL | unset; `  `; `127.0.0.1:1` | unconfigured; unconfigured; configured |

**Controller** (a bare `Shared` with the fake Director, `MATCHMAKING_MODE=DEPLOYSERVER`; the clock moved, never slept)

| Case | Expected |
| --- | --- |
| Solo hunt cancelled before its release | leave → `canceled`/`matching`; status `CANCELED` `PLAYER_CANCELED`; a second leave `already-canceled`; a re-join searches again (`MATCHING`) |
| Party's forming hunt cancelled by the leader | every member `CANCELED` (leader `PLAYER_CANCELED`, others `PARTY_MEMBER_CANCELED`); the leader searches again and the members follow the new instance |
| Leave after the release | `too-late`, state `ready`; candidates untouched |
| Hub candidate | `hub-not-cancelled`, state `hub` |
| Queued search cancelled | leaves its queue (removed when empty); a later join queues again |
| No candidate | `no-candidate`, state `none` |
| Dedicated-only rifts | the leave is the local cancel |
| Four players of a `CR19` hunt | the fourth's join launches the queue; the three earlier ones travel together; the fourth waits on nothing (Node's order) |
| Three players then 20 s | the next poll pops; all three ready with one shared release |
| No Director | Ramsgate join cancelled with `NO_DEDICATED_SERVERS`; a party's hunt cancels every member with it; a cached ready Ramsgate candidate's status holds `MATCHING` (unconfigured gives no hub); a hub game session id is still 36 characters |
| Director refuses / throws | join false (400) / an error (500) |
| `MATCHMAKING_MODE` `DISABLED`, ``, `deployserver`, `SOMETHING` | false each |
| Gauntlet off | requester and party cancelled `GAUNTLET_DISABLED` |
| Gauntlet on; `CR19_PlayerHunt_Arena_Elite` off | queued; queued |
| Party returning to a hub (`CITY`, Ramsgate, two members) | every member gets the destination with its own candidate id and the same game session id, the hub's stable id; the party instance's hunt is Ramsgate; the launch expects the whole party; no hunt start recorded |
| Status: none; still-matching hunt; cancelled with a reason; `REALM_FULL` | 404 bare; `ISLAND` with its hunt id; `CANCELED` + reason, no retry; `retryAfterSeconds` 30 |
| Ramsgate status: hub not ready; hub ready on port 8791 | `MATCHING` period 1000 `CITY` null, one hub ask; `IN_PROGRESS`, `serverInfo` `{"buildId":"647472_1.14.7_shipping","gameSessionId":"hub-room","host":..,"port":8791}`, period 10000 |
| Travel host for hub `198.51.100.9` | client `198.51.100.9`, `::ffff:198.51.100.9`, `127.0.0.1`, `::1` → `127.0.0.1`; `203.0.113.7`, none → `198.51.100.9` |
| Realm hubs | a realm Ramsgate is never checked against the Director; a realm Training hub answers `SHARED` with its hunt id waiting or running; a realm hub cancelled with its reason has no retry hint |
| Poll period | 1000 while matching and in the release window, then 10000 |
| Ring (recording home, standalone node, real temp store) | a `CR19` join with an address submits a ticket with it; a `CITY` Ramsgate join with an address asks `ring_assign_ramsgate` with it; a join without an address makes no ring call |

**Routes** (the 18 rows over a temp store with accounts `alice`, `bob`, `carol`, the fake Director online at
`203.0.113.10:7777` with a ready Ramsgate there, all Go Now delays 40 ms and padding 1 ms, `QOS_TARGET_URL=
qos.example.net`, a stand-in auth layer naming the user): the scenario of register, alive, regions, `/QoS`, an
unauthenticated status (empty 401), status with no candidate (bare 404), a Ramsgate join and its status before and
after 150 ms (`MATCHING` with the Ramsgate hunt id, then `IN_PROGRESS` with `huntId` null and the hub's stable
session id), party create (revision header 1, `QUEUED_FOR_START`, the caller's Ramsgate candidate id), invite, invite
without a recipient, invites, accept, the member's party (revision 2, its `candidateId` null), an ISLAND join without
the party id (solo, warned), its status, the five leave shapes (`{}` each), the cancelled status, an ISLAND party join
with the party id and both members' statuses (distinct candidate ids, `MATCHING`), the member's party `MATCHING` then
`IN_PROGRESS` after 150 ms, his `IN_PROGRESS` status, promote, kick, leave, a join with no body (500 page), a join of
`{}` (`MATCHING` with `huntId` `""` and no `gameMode`), and `POST /party` with `{` (400 page). Each answer's status,
content type, revision header and key order as §3 says; ids compared as shapes.

**Outbox** (temp store): events published oldest first, once; a failing publish is released and stops the pass; a
claim older than 30 s is taken over by another owner; a recipient without an account is left out of the headline; the
body is chat's public event.

**QoS listener**: `QOS_PORT=8081` binds loopback, reports public port 8081, answers `GET /`, `POST /anything/at/all`,
`GET /QoS` with the pong and `text/html`; `off` starts none.

**`IslesCore`**: `party` of an unknown id is none and without a store `NO_DATABASE`; `set_candidate` with bad JSON and
with a missing field refuses `INVALID_CANDIDATE`; `cancel_travel` cancels each account once; `touch_game_session`
makes the account recently active; `dedicated_servers_configured` follows the slot.

**Director invocation hooks**: a local stored account becomes its userId; a foreign or unknown id stays its global id
text; `PlaceHub` recording leaves the hub as the last ring instance.

---

## Coordinator decisions (2026-11-05, before implementation)

These settle the open questions and override anything above that disagrees.

- **Item 8 (kicks): taken, a security departure.** The member-left hook runs only when the kick removed the target.
- **Item 4 (stuck queue): taken.** A launch that fails while a queue pops clears the launching mark.
- **Item 12 (ring instances):** the economy crate's `RingInstances` slot trait (spec-rustic-isles-economy-inventory.md)
  is the type; this crate implements and fills it.
- **Item 11:** the three defaulted `IslesCore` methods forward to the fallback, as proposed.
- **Item 13: taken.** Client text in the join log has control characters escaped and is capped in length.
- **Item 1:** `X-Mystic-Party-Revision` stays (a wire fact).
- **Everything listed as parity:** parity; the departures above are candidate decision rows for the maintainer to
  ratify.

## Coordinator decisions, amended after implementation (2026-11-05)

Read against the parity implementation after the crate was written; these override §17.2 where they disagree.

- **§17.2, "Solo hunt cancelled before its release": the vector was wrong; §6.4 stands.** A solo `ISLAND` join of a
  hunt that needs no queue launches at once (§6.7): its candidate is ready on the server with a hunt release 10 to
  20 s ahead, so the status poll still answers `MATCHING` (it is not client-ready). A leave in that window answers
  outcome `canceled` with state **`go-now`** (§6.4 step 6: client-ready is `ready`, ready but not released is
  `go-now`, then `queued` for a player in its hunt's queue, else `matching`), and the candidate id of the search. The
  status then answers `CANCELED` with `PLAYER_CANCELED` and the same candidate id (a cancel keeps the id); a second
  leave answers `already-canceled`, state `canceled`, with that id, and the reason is unchanged; a re-join is taken,
  answers `MATCHING` again and launches a fresh hunt (a second Director launch). The case should read: leave →
  `canceled`/`go-now`; status `CANCELED` `PLAYER_CANCELED`, same candidate id; a second leave `already-canceled`/
  `canceled`; a re-join searches again (`MATCHING`) with a new launch. `canceled`/`matching` is what a candidate that
  is not yet ready and not queued gives, which this flow never reaches.
- **The party case's state** follows the same rule: a party hunt launched online and cancelled by its leader inside
  the release window answers `canceled`/`go-now`.
