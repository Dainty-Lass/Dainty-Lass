<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false, "code_blocks": false } } -->

# Clean-Room Behaviour Specification: `crates/rustic-isles-chat` (RD27)

> **Scope and purpose (RD27):** this is the clean-room behaviour specification for an MIT crate,
> `rustic-isles-chat`: unit 6 of [`spec-rustic-isles-core.md`](spec-rustic-isles-core.md) §16. It is two things.
> First, Rustic Isles' **chat rules**: the answers a 1.14.7 module gives dainty-lass's chat plugin through
> `module_api::chat` (the JID of a player, what a room local part names, who may join a room, the SASL login check,
> friends, whispers, the account behind a JID local part), the hub-session registry that opens region rooms, the
> party headline JSON, and the three `chat.*` home operations. Second, the **chat area routes** of the backend: the
> friends API, the Slayer link stubs, the guild stubs and `/vivox/login` (11 rows of core Appendix A). Its source is
> dainty-restricted's `modules/rustic-isles/crates/isles-chat` and `crates/isles-core/src/areas/chat.rs` +
> `areas/chat/rules.rs`, AGPL-3.0-only with Mystic Paradox's section 7 terms; an implementer who never sees them
> writes the MIT crate from this document alone.
>
> Everything below is stated from observable contracts: the routes and their bytes, the answers to each chat query,
> the XMPP-visible names and frames, the settings, the store calls by the store spec's names, the seam, and the tests.
> No implementation code is reproduced and no private helper is named. Node file names appear only as provenance.
> Wire names the 1.14.7 client depends on (`prod.ol.epicgames.com`, `party-events@`, `Party-`, `City-`, `Hunt-`,
> the Epic friends shapes) are interface facts and are kept.
>
> Written against dainty-restricted `e0e238b4434316bb210677d88bc59e3448688794` (worktree
> `splashloader-integration-90130d`; the unit's files last changed in `9ad97c6`, 2026-11-01) and dainty-lass
> `669f72552c15b65bc7268744360a435830f76092` (branch `feat/isles-auth-clean-room`).

---

## 1. Provenance and licensing classification

### 1.1 Evidence used

| Source | What it says |
| --- | --- |
| `modules/rustic-isles/NOTICE.md` | The whole module is a modified Mystic Paradox, AGPL-3.0-only with the section 7 terms |
| File headers (`//!`) | `isles-chat` ports `realtime/XMPPAuth.ts`, `RoomService.ts#authorizeJoin`, `cityRooms.ts` (Reforged patch 0025) and `PartyOutboxDispatcher.ts`'s event shape, recast for DD21; `areas/chat.rs` ports `routes/friends.ts`, `routes/guild.ts` and `/vivox/login` of `routes/system.ts` |
| `git log` of the files | 12 commits, one author (the maintainer): born in `579c822` (RU5 routes) and `245cfdc` (RU5 rules), with `b909a45` (DN10b cross-node login and home forwarding), `ab07f6c` (one `AuthData`, Express body readers), `0a29728` (replay clock), `9ad97c6` (rename) |
| `crates/isles-core/tests/chat_golden.rs`, `chat_rules.rs`, `friendships.rs`, `tests/golden/chat/` | What the routes answered under Node 22 with a fixed clock, and the rules over a real store (described in §11, never copied) |
| dainty-lass `crates/dainty-module-api/src/chat.rs`, `crates/dainty-chat` (`naming.rs`, `hooks.rs`, `muc.rs`, `server.rs`, `service.rs`), `docs/port/spec-rd27-chat.md` | The contract this crate answers and how the plugin uses each answer |

### 1.2 Per file: category and where it goes

All of it is Category B (Paradox behaviour, or Dainty Lass code wrapped around it). The implementer writes every part
from this spec.

| Restricted file | Source | Rewritten as |
| --- | --- | --- |
| `isles-chat/src/lib.rs` | DD21 dispatch of `ChatQuery` | §5 |
| `isles-chat/src/ids.rs` | Dainty Lass (DD18 ids in room names) | §4.3, §5.3 |
| `isles-chat/src/rooms.rs` | `XMPPAuth.ts` JID, `RoomService.ts#authorizeJoin` | §5.2, §5.3, §5.4 |
| `isles-chat/src/login.rs` | `XMPPAuth.ts#authenticateSasl` | §5.5 |
| `isles-chat/src/hubs.rs` | `cityRooms.ts` (patch 0025) | §5.10 |
| `isles-chat/src/headline.rs` | `PartyOutboxDispatcher.ts` (`PublicEvent`, `HeadlineFrame`) | §4.4, §5.11 |
| `isles-chat/src/source.rs` | Dainty Lass (what the rules read) | §5.1, §9 |
| `isles-chat/src/fake.rs` | test support | §11 (described only) |
| `isles-core/src/areas/chat/rules.rs` | the rules' source over the store; dev switches of `RoomService.ts` | §5.1, §7, §8 |
| `isles-core/src/areas/chat.rs` | `routes/friends.ts`, `routes/guild.ts`, `/vivox/login` | §6 |
| `isles-core/src/module.rs`, `chat.*` home operations only | DN5/DN10b | §5.9 |

### 1.3 What stays in dainty-restricted

- `tests/golden/chat/` (`chat.json`, `record.cjs`, `run.sh`) and `chat_golden.rs`: replayed in restricted against this
  crate once the composer uses it; a failure there is a bug on this side.
- The party outbox dispatcher (claim, publish, mark) until the matchmaking unit (core §16 row 8) lands; it calls this
  crate's headline builder.
- The matchmaking call that records a hub session (`/candidate/status`) until row 8; it records into this crate's
  registry (§5.10).
- The composer's wiring (core §11.4): it builds this crate's source and hands its answerer to `ModuleHooks::chat`.

---

## 2. Conventions

- JSON as `JSON.stringify` writes it (`service_kit::json::Json`, `content-type: application/json; charset=utf-8`,
  insertion order, so `serde_json` with `preserve_order` in this crate's own manifest, core §17 item 8). `U+007F` and
  `U+2028` are written raw; control characters below `U+0020` as `\uXXXX` except `\b \f \n \r \t`.
- "Empty" is a status with no body and no `content-type`.
- ISO times are `Date#toISOString`'s (`2026-10-20T12:00:00.000Z`), from core's replay-aware clock (core §9.1:
  `season::to_iso_string(replay::now_ms())`). Rules that compare against "now" in milliseconds use core's
  `replay::now_ms()` (§13 item 17).
- **userId** is the client's id for an account (core §2.2); **account** is a `GlobalId` of kind `Account`. The client
  only ever sees userIds.
- The **domain** is `prod.ol.epicgames.com` everywhere below (the captured logical XMPP domain).

---

## 3. What this crate adds, and what it must not duplicate

**Already in dainty-lass, used as is:**

| Owner | What | This crate |
| --- | --- | --- |
| `module_api::chat` | `Jid` (parts 1 to 255 bytes, no `@`, `/`, whitespace or control), `RoomKind { Network, Region, Party, Hunt }`, `RoomRules { may_join, may_speak, history }`, `ChatQuery`, `ChatAnswer`, `PRESENCE_EVENT` + `PresenceEvent`, `HEADLINE_EVENT` (`isles.party.headline`) + `Headline { id, recipients, body }` + `HeadlineRecipient { account, user_id }` | Answers the queries with these types; may re-export them; never defines its own JID, room kind or headline type |
| `crates/dainty-chat` (the plugin) | XMPP, SASL framing and `not-authorized`/`temporary-auth-failure`, bind and resources, MUC (occupants, nicknames, history, eviction sweep), whispers, presence emission (`chat.presence`), the default JID and default rules for a module that answers `Unsupported`, and its **own** room naming (`naming::room_kind_of`: `General*`, `City-<id>`, `Region-<name>`, `Party-<id>`, `Hunt-<id>`, ids at most 64 bytes) | Nothing of it. The crate never depends on `chat`: a game module talks to chat only through `module-api` (spec-rd27-chat §6.2) |
| `rustic-isles` (core) | presence set (§5.5), `chat.presence` handling (§7.7), the `ModuleHooks::chat` slot (§7.5, §11.4), home invocations (§7.8), `Handlers`, `not_implemented`, the body reader (§3.9), the ISO clock | Uses them; does not keep a presence set or its own ISO formatter |
| `rustic-isles-auth` | game auth (`Auth::game_auth`, `AuthData`), `Auth::validate_game_token`, `is_account_eligible(&LauncherAccount, now_iso)` | Uses them; does not define its own `AuthData` (restricted's chat area had a two-field copy: drop it) |
| `rustic-isles-store` | `IslesStore`, `Repositories` (`accounts`, `launcher_accounts`, `friendships`, `parties`), `RepoError` | §7 |

**What this crate adds:** the rules behind each `ChatQuery` for 1.14.7 (§5); the reading of a room local part the
plugin's own naming refused (§5.3; it reaches the module only through `RoomOf`); the room spelling of a party or hunt
id that the party area writes and the rules read back (§4.3); the hub-session registry (§5.10); the headline JSON and
its constants (§5.11); the `chat.*` home operations (§5.9); the 11 routes (§6).

**Precedence, as the plugin runs it** (spec-rd27-chat §2.13.1): the plugin's naming is tried first and the module's
`RoomOf` only when it gives none. So `City-<alnum or ->` up to 64 bytes and `Party-`/`Hunt-` ids up to 64 bytes never
reach this crate's reading; it decides only the longer or odder names (§13 item 14).

---

## 4. XMPP-visible facts

### 4.1 A player's JID

`<userId>@prod.ol.epicgames.com`, no resource (the plugin adds the bound resource). The client's SASL PLAIN authcid is
the userId and its password the game token (auth §5.1); the plugin refuses a login whose JID local part differs from
the authcid, which is why the JID is the userId. A userId that is not a valid JID local part (holds `@`, `/`,
whitespace or a control character, or is over 255 bytes) has no JID (§5.2).

### 4.2 Rooms (names the 1.14.7 client builds; the MUC domain `muc.prod.ol.epicgames.com` and nicknames are the plugin's)

| Local part | Meaning | Kind |
| --- | --- | --- |
| `Party-<partyId>` | the party's room; `<partyId>` exactly as the party area handed it to the client (§4.3) | `Party(<party global id>)` |
| `City-<gameSessionId>` | a hub's room (Ramsgate or a dojo), from the `gameSessionId` matchmaking served; one room per region under DD21 | `Region(..)` |
| `Hunt-<id>` | a hunt's room | `Hunt(<hunt global id>)` |
| `General`, `General<id>` | network-wide room (the plugin's naming) | `Network` |

Prefixes are case-sensitive.

### 4.3 Party and hunt ids inside room names

The **room spelling** of a global id is `<ULID>.<node>` (the ULID's canonical 26-character upper-case text, a dot, the
owner node's name): a `GlobalId`'s own text (`party:<ULID>@<node>`) cannot sit in a JID. The party area's client-facing
party id (`MakePartyId`, core §16 row 8) is the room spelling, `_`, then the build id in standard padded base64
(`<ULID>.oregon-1_NjQ3NDcy` for build `647472`; an empty build id leaves a trailing `_`). This crate owns the room
spelling and its reading; the party area uses both.

**Reading an id** for kind `K` on this node `N` (Party or Hunt):

1. Keep the text before the first `_` (the build suffix is ignored). Empty, or over 300 bytes: nothing.
2. The text parses as a full `GlobalId` (`kind:ULID@node`): it, when its kind is `K`; another kind is nothing.
3. Else it holds a `.`: split at the first `.` into ULID and owner and read `K:<ULID>@<owner>` as a `GlobalId` (the
   ULID must be canonical upper case and the owner a valid node name, so `01j9...` and `.Oregon` are nothing).
4. Else it is a UUID (8-4-4-4-12 hex digits, either case): the ULID with the same 128 bits, owned by `N`.
5. Anything else (a bare ULID with no node, `xyz`, a UUID one digit short): nothing.

### 4.4 Party headlines

The party area's outbox events reach each recipient as one stanza per recipient connection (Node's frame; the
deliverer writes it, §13 item 3):

`<message type="headline" from="party-events@prod.ol.epicgames.com" to="{userId}@prod.ol.epicgames.com" id="{eventId}"><body>{json}</body></message>`

`userId`, `eventId` and the JSON text are XML-escaped: `&` `&amp;`, `<` `&lt;`, `>` `&gt;`, `"` `&quot;`, `'`
`&apos;`, nothing else. No `xmlns` attribute in Node's frame. The JSON is §5.11's public event.

---

## 5. The chat rules

### 5.1 The source: what the rules read, and how errors map

The rules read the game through one source over the module's store, the module's auth, its node identity, its module
id, its home API and the chat settings (§8). Error mapping, for every query:

| Situation | Answer |
| --- | --- |
| No database (the module runs without a store) | `Unsupported` (`chat without a database`; login: `chat login without a token verifier`): the plugin then uses its default JID and rules |
| An account `GlobalId` of another kind where an account is needed | `Invalid` with the id error's text |
| A store read fails | `Unavailable` with the error's display; warning logged `chat rules: {error}`. The plugin refuses the join or login rather than open what the rules would close |

A **player** is (account, userId, display name = `accounts.name`).

- **Player by account**: the live `accounts` row of that id (store §6.1 `find`). None and the account is owned by
  another node: the home operation `chat.player` is forwarded to the account's home (§5.9; actor System, body `{}`);
  an answer with string `user_id` and `name` is the player (with the asked account). A forward failure or any other
  answer is "no player".
- **Player by userId** (sign-in and lookups): the live game account with that userId; none is "no player". A launcher
  account for that userId (store §6.2 `find_by_user_id`) must be eligible for play (auth §6.1 via
  `is_account_eligible`, at core's replay-aware ISO now); none is eligible (dev, service and game-server identities
  keep working). Ineligible is "no player".
- **Party of an account**: the party it is a member of (store §11.2 `find_by_member`), as its client-facing id
  (`PartyRecord.party_id`). None and the account is another node's: `chat.party` forwarded to its home (actor
  System, body `{}`); a string `party_id` is the id, anything else (or a failure) none.
- **Friends of an account**: the account's edges in list order (store §8 `list_for_owner`), `ACCEPTED` only, as the
  `other` accounts.

### 5.2 `Jid { account }`

The player by account (§5.1); none: `NotFound` `no account {account}`. Then `Jid::new(userId, domain, no resource)`;
refused: `Invalid` `the userId is not a JID local part ({jid error})`. Answer `ChatAnswer::Jid`.

### 5.3 `RoomOf { name }`

| Name | Answer |
| --- | --- |
| `Party-<rest>` | `Party(id)` for §4.3's reading of `rest` as a party on this node, else none |
| `Hunt-<rest>` | `Hunt(id)` likewise for a hunt |
| `City-<rest>` | the region of a live hub session whose game session id is exactly `rest` (§5.10); else `rest` read as a `Region` name (module-api's name rule: 1 to 64 of `a-z 0-9 -`, starting with a letter, not ending `-`); else, when `rest` is not empty, this node's region; `City-` alone is none |
| anything else (`General`, `Guild-...`, `party-x`) | none |

Always `Ok(ChatAnswer::RoomOf(..))`; this query never reads the store.

### 5.4 `Room { room, account }`

| Room | Rules |
| --- | --- |
| `Network` | refused (Paradox has no network room) |
| `Region(r)` | open when `REALTIME_XMPP_DEV_CITY_MUC` is on, or the account's live hub session is in region `r` (§5.10); else refused |
| `Hunt(_)` | open when `REALTIME_XMPP_DEV_HUNT_MUC` is on (the default), else refused (no account-to-hunt mapping exists) |
| `Party(p)` | the party of the account (§5.1): none is refused; its id read as a party on this node (§4.3) equal to `p` is open, else refused. A store error is the error (§5.1) |

**Open** is `{may_join: true, may_speak: true, history: false}`, **refused** all three false. History is never kept
(Paradox's rooms were in memory; a joiner sees only what is said after joining). The account is not checked for
existence. Region and Hunt rules read no store, so they answer even without a database; Party without one is
`Unsupported`. The plugin re-asks every minute and evicts whoever is no longer admitted (a party leave, kick or
disband).

### 5.5 `Login { authcid, token }`

No database: `Unsupported` (§5.1). Otherwise, in order (refusal reasons are for the log only; the client sees the
plugin's `not-authorized` or `temporary-auth-failure`):

1. The token is checked as a **game token** (`Auth::validate_game_token`: RS256, `iss`/`aud` `paradox-backend`, not
   expired; a launcher or store token is refused by audience). Invalid: refusal `jwt verify failed`.
2. Its `userId` claim must be a non-empty string: else `no userId in token`.
3. `userId` = `__launcher_client_credentials__` (the launcher client-credentials grant, auth §3.2): `client-credentials
   token not allowed`.
4. `userId` must equal `authcid` byte for byte: else `authcid/token mismatch`. Identity comes from the token, never the
   JID.
5. Player by userId (§5.1): a player is `ChatAnswer::Login { account, name }`; no player `account not eligible`; a store
   error `account lookup error`.

Outcome: `account lookup error` answers `Unavailable` with that text (the plugin says `temporary-auth-failure`). Any
other refusal first tries the **cross-node login**; if that finds a player it is the answer, else `Invalid` with the
refusal text.

**Cross-node login** (a player homed on another node connecting here), errors swallowed:

1. The token parses as a `module_api::network::SessionTicket` token (`SessionTicket::from_token`): the home verifies it
   for this module's game version (`HomeApi::verify_session_ticket`, version from core tables, 1.14.7). Admitted: the
   player by account of the admitted account (§5.1, which may forward `chat.player`); it is the answer when the
   admitted account's id text or the player's userId equals `authcid`.
2. Else, `authcid` parses as an account `GlobalId` owned by another node: `chat.login` is forwarded to that home (actor
   `Player { account }`, body `{"authcid":..,"token":..}`); an answer with string `user_id` and `name` is the player
   (account = the parsed authcid).
3. Else none.

### 5.6 `Friends { account }`

§5.1's friends, as `ChatAnswer::Friends(Vec<GlobalId>)`. Not asked by the plugin today (§13 item 4).

### 5.7 `MayWhisper { from, to }`

Always `MayWhisper(true)`: 1.14.7 has no block list (the blocklist route is an empty stub, §6.4). The plugin then
never treats two players as blocked.

### 5.8 `AccountOf { local }`

1. Player by userId (§5.1) of `local`: its account.
2. Else `local` trimmed (Unicode whitespace) and ASCII-lower-cased; empty: none.
3. The live launcher account with that normalized display name (store §6.2 `find_by_display_name_normalized`), when
   eligible for play: its account. Else none.

Store errors are §5.1's `Unavailable`.

### 5.9 Home operations (core §7.8, run on the account's home; the composer's home-operation hook dispatches to them)

| Operation | Body | Answer |
| --- | --- | --- |
| `chat.login` | `authcid`, `token` (strings; absent or not a string reads as empty) | §5.5 steps 1-5 only (no cross-node fallback, even `account lookup error` is `Invalid`): a refusal is `Invalid` with its text; a player whose account is not the invocation's account `Invalid` `token account does not match target account`; else `{"account":<id text>,"user_id":..,"name":..}` |
| `chat.player` | — | the live `accounts` row of the invocation's account: `{"account":<id text>,"user_id":..,"name":..}`; none `{"player":null}` |
| `chat.party` | — | `{"party_id":<client-facing id or null>}` |

A store error is a failure of the invocation. Key order as written. dainty-lass's plugin uses `chat.login` too, with
actor System (spec-rd27-chat §3.4); the operation does not check the actor.

### 5.10 Hub sessions (patch 0025, recast for DD21)

A per-module, in-memory registry of the hub each account was last sent to: (game session id, region, time ms).

- **Record** (account, game session id, region, now): an empty game session id records nothing (and keeps the old
  record). Otherwise it replaces the account's record and makes it the newest. Over 50,000 accounts, the oldest
  recorded go first.
- **Live**: `now - at <= 7 days` (604,800,000 ms; the seventh day is still in, one ms later is out; a record from the
  future is live).
- **May join region** (account, region, now): the live record's region equals it.
- **Region of hub** (game session id, now): the region of any live record with that game session id.
- **May join City room** (account, game session id, now): the live record's game session id equals it. Kept for the
  day the room name reaches the Room query; nothing asks it today.

Matchmaking records when `/candidate/status` sends a player to a hub (with the node's region and matchmaking's
clock). Restricted keeps one registry per process; the MIT crate keeps one per module, as core does with presence
(core §17 item 14; §13 item 12).

### 5.11 The headline body (`PublicEvent`)

A JSON object, keys in this order: `eventId`; `eventType` (the public name below, **left out** when the type has
none, as `JSON.stringify` drops `undefined`); `schemaVersion` `1`; `occurredAt` (the outbox row's `created_at`);
`partyId` (client-facing); `partyRevision` (integer); `payload` (the stored payload as it is).

| Outbox `event_type` | `eventType` |
| --- | --- |
| `PARTY_CREATED` | `party.created` |
| `PARTY_INVITE_CREATED` | `party.invited` |
| `PARTY_MEMBERSHIP_CHANGED` | `party.membership_changed` |
| `PARTY_LEADER_CHANGED` | `party.leader_changed` |
| `PARTY_ACTIVITY_CHANGED` | `party.activity_changed` |
| `PARTY_PRESENCE_CHANGED` | `party.presence_changed` |
| `PARTY_HUNT_STATUS_CHANGED` | `party.hunt_status_changed` |

The module emits `HEADLINE_EVENT` with `Headline { id: <eventId>, recipients, body: <the JSON text> }`, recipients
being the outbox recipients (userIds) that have a live local game account, each with its account. Building recipients
and emitting is the party area's; this crate supplies the constants (`party-events@prod.ol.epicgames.com`, the domain),
the name table and the body.

---

## 6. Routes

### 6.1 Common rules

The 11 rows are backend, side `isles`, group `chat`, mounted always (core Appendix A rows B35, B80, B81, B137-B144),
matched by core (§4.1: ASCII case-insensitive literals, non-empty params, `HEAD` on `GET` rows).

- **Game auth** where marked: the request's `AuthData` if one is already in its extensions, else `Auth::game_auth`
  (auth §4.1). A refusal is answered exactly as auth made it (empty 401, empty 403, or its 500 page). The **caller** is
  `AuthData.user_id`, which can be absent (a game server key with no bearer and no bearer-less account, or a token
  whose `userId` is not a string).
- **No store** on a route that needs one: after auth, core's 501 envelope (`not_implemented`).
- **The `:accountId` path param is ignored**: the owner is always the caller (§13 item 6).
- **Store errors**: `NotHome` or `Home` (the account lives elsewhere, DN5 not yet forwarding) is an empty 503 with a
  warning `{METHOD} {row path}: {error}`; any other an empty 500 with that line at error level.
- **Envelope**: `{"code":null,"message":"OK","payload":<payload>}`.

| Row | Route | Auth | Answer |
| --- | --- | --- | --- |
| B35 | `GET /vivox/login` | game | empty 404 (no Unstubbed log line) |
| B80 | `GET /guild/invite/player` | game | 200 envelope, payload `{"invites":[]}` |
| B81 | `GET /guild` | game | empty 204 |
| B137 | `GET /friends/api/public/friends/:accountId` | game | §6.2 |
| B138 | `POST /friends/api/public/friends/:accountId/:friendId` | game | §6.3 |
| B139 | `DELETE /friends/api/public/friends/:accountId/:friendId` | game | §6.3 |
| B140 | `GET /friends/api/public/blocklist/:accountId` | none | 200 `[]` |
| B141 | `GET /friends/api/public/list/:namespace/:accountId/recentPlayers` | none | 200 `[]` |
| B142 | `GET /friends/api/v1/:accountId/settings` | none | 200 `{"acceptInvites":"public","mutualPrivacy":"ALL"}` |
| B143 | `POST /slayerlink/availability` | body, then game | §6.4 |
| B144 | `GET /slayerlink/status_good` | game | 200 envelope, payload `{"config":{"invite_expiry_hours":48,"link_duration_hours":168},"invites":[],"links":[]}` |

### 6.2 The friends list (B137)

Order: auth; store (501 without); read `includePending`; no caller: 200 `[]`. The caller's game account by userId
(store §6.1); none: `[]`. Else the caller's edges (store §8 `list_for_owner`, first-written order; an edge replaced
in place keeps its position), keeping `ACCEPTED`, plus `PENDING` when `includePending`; `BLOCKED` never. Each, keys in
this order:

| Key | Value |
| --- | --- |
| `accountId` | the edge's `other_user_id` |
| `status` | `ACCEPTED` or `PENDING` |
| `direction` | the stored direction (`INBOUND`/`OUTBOUND`), `OUTBOUND` when null |
| `created` | the stored ISO text |
| `favorite` | the stored boolean |

Logged at info: `Friends list for {caller}: {count} (includePending={true|false})`.

**`includePending`** is read as Express 5's simple query parser would: the query split on `&`, empty pieces skipped,
each split at its first `=` (none: the value is empty), `+` read as a space, then percent-decoded (invalid UTF-8
replaced). True only when exactly one pair has the key `includePending` and its value is exactly `true`.

| Query | `includePending` |
| --- | --- |
| `includePending=true`, `a=1&includePending=tr%75e` | true |
| none, `includePending=TRUE`, `includePending`, `includePending=`, `includePending[]=true`, `includePending=true&includePending=true` | false |

### 6.3 Invite, accept, remove (B138, B139)

**POST** order: auth; store (501); `friendId` empty or equal to the caller: empty 400; no caller: empty 204; invite;
empty 204.

**Invite** (caller userId `o`, friend userId `f`; the time is read once from the route's clock as soon as both
accounts are found, whatever happens next):

1. Both game accounts are looked up by userId on this node. Either missing: nothing (no edge for a stranger, DD18).
2. The caller's edge to the friend (store §8 `find`).
3. It is `ACCEPTED`: nothing.
4. It is `PENDING` with direction `INBOUND` (the friend invited first): the caller's edge becomes `ACCEPTED`,
   direction null, `created` kept from that edge; then the friend's edge to the caller `ACCEPTED`, direction null,
   `created` now. Log `{o} accepted friend invite from {f}`.
5. Anything else (no edge, an outbound pending, a blocked edge, a pending one with null direction): the caller's edge
   `PENDING` `OUTBOUND` now, then the friend's `PENDING` `INBOUND` now. Log `{o} sent friend invite to {f}`.

Every edge written carries `other_user_id` = the other's userId and `favorite` false (store §8 `upsert`). Each edge is
its own account write on **its owner's** home with actor `Player { <caller's account> }` (store §3.2
`account_write`); the two writes are separate transactions.

**DELETE** order: auth; store (501); no caller: empty 204; remove; empty 204. No self check.

**Remove** (`o`, `f`): the caller's game account by userId; none: nothing. Each of the caller's edges whose
`other_user_id` is `f` is removed (an account write on the caller's account). Then, if `f` has a game account on this
node, its edge to the caller is removed (an account write on the friend's account, actor the caller). Log `{o}
removed/rejected friend {f}`. The caller's edge goes even when the friend's account is gone.

### 6.4 Slayer link availability (B143)

1. **The body first**, as Express's `express.json()` ran before every router: core's body reader (core §3.9, 100 KiB).
   A body error is answered there (400 for malformed or non-object/array JSON, 413 over the limit, 415 for a charset
   or encoding), **before auth**. A content type other than JSON (or form) leaves the body absent.
2. Game auth.
3. Payload `{"availability":[..]}`: when the body is an object whose `account_ids` is an array, its elements that are
   strings of 1 to 128 UTF-16 code units, in order, the first 256 of those, each `{"account_id":<id>,"available":false}`.
   Anything else (no body, an array body, `account_ids` not an array, an empty JSON body) is an empty list. 200
   envelope.

---

## 7. Store use (store spec names)

| Repository (store §) | Call | Used by |
| --- | --- | --- |
| `accounts` (6.1) | `find`, `find_by_user_id` | §5.1, §5.9, §6 |
| `launcher_accounts` (6.2) | `find_by_user_id`, `find_by_display_name_normalized` | eligibility (§5.1), `AccountOf` |
| `friendships` (8) | `list_for_owner`, `find`, `upsert`, `remove` | §5.1 friends, §6 |
| `parties` (11.2) | `find_by_member` | §5.1 party, `chat.party` |

Reads go through `IslesStore::read`; writes through `account_write` (store §3.2, §3.4: a write for an account homed
elsewhere is `NotHome`). Eligibility needs auth's `LauncherAccount`; store §12.1's mapping gives it (the implementer
may read through `IslesAuthStore` over the same store). No migration, no table of its own: `friend_edges` is
`0070_chat_friend_edges` (store §4.8).

---

## 8. Settings

Read from core's settings with `raw` (env file and forced values, core §6), trimmed (Unicode whitespace).

| Variable | Rule | Default |
| --- | --- | --- |
| `REALTIME_XMPP_DEV_CITY_MUC` | on only when the trimmed value is exactly `true` | off (RU3c; Node's was on unless `false`). The host kit writes `false` (`hostconfig.rs`) |
| `REALTIME_XMPP_DEV_HUNT_MUC` | off only when the trimmed value is exactly `false` | on (Node's) |

| Value | City | Hunt |
| --- | --- | --- |
| unset | off | on |
| ` true ` | on | on |
| `` (empty), `false`, `TRUE` | off | empty on, `false` off, `TRUE` on |
| ` false ` | off | off |

Read once when the source is built. The game token keys are auth's (§5.1 there), never read here. No new variable.

---

## 9. The public seam

### 9.1 Shape

One crate, `rustic-isles-chat` (lib), MIT, in dainty-lass `crates/rustic-isles-chat`. Depends on dainty-lass
`module-api`, `service-kit` (`json`), `rustic-isles` (core: `Handlers`, `Call`, `Shared`, `Service`, `Method`,
`not_implemented`, body reader, `replay`, `season`, `Settings`, `AccountId`, `UserId`), `rustic-isles-store`,
`rustic-isles-auth`; workspace `axum`, `http`, `serde`, `serde_json` (own `preserve_order`), `percent-encoding`,
`tracing`, `thiserror`. Core does not depend on it (the composer does). **Seam changes for the integration pass:**
the crate in the workspace members and `[workspace.dependencies]`; nothing else.

### 9.2 Interface (names are the contract the composer and the matchmaking unit are written against)

| Item | Contract |
| --- | --- |
| `pub const XMPP_DOMAIN`, `pub const HEADLINE_FROM` | `prod.ol.epicgames.com`, `party-events@prod.ol.epicgames.com` |
| `pub const CLIENT_CREDENTIALS_USER_ID` | `__launcher_client_credentials__` |
| `pub struct ChatSettings { dev_city_muc, dev_hunt_muc }` | §8; `Default` off/on; `from_settings(&Settings)` |
| `pub struct Player { account, user_id, name }` | §5.1 |
| `pub trait ChatSource` | node identity, settings, a token check (none without a store), player by account, player by userId, account of a local part, cross-node login, party of, friends; async methods answering `Result<_, ModuleError>`. Tests implement it in memory |
| `pub struct StoreChat` | the source over the module (`new(node, module_id, store: Option<IslesStore>, settings, auth, home)`); `Debug` prints no key or token |
| `pub trait ChatRules` + `pub struct IslesChat` | `answer(&dyn ChatSource, &HubSessions, &ChatQuery) -> BoxFuture<Result<ChatAnswer, ModuleError>>`, one method per query (§5.2-§5.8) with the defaults as specified, so another version can override one |
| `pub enum Refusal` + `as_str()` | §5.5's six reasons; `pub async fn authenticate(authcid, token, &dyn ChatSource) -> Result<Player, Refusal>` (steps 1-5) |
| `pub mod ids` | `room_id_text(&GlobalId) -> String`, `instance_id(text, IdKind, &NodeId) -> Option<GlobalId>` (§4.3) |
| `pub struct HubSessions` | `new`, `record`, `may_join_region`, `region_of_hub`, `may_join_city_room` (§5.10), `HUB_SESSION_TTL_MS`, `MAX_ACCOUNTS`; one per module, put in a core slot by the composer for matchmaking to record into |
| `pub mod headline` | `event_name(&str) -> Option<&'static str>`, `public_event(event_id, event_type, occurred_at, party_id, revision, &Value) -> Value` (§5.11), `frame(user_id, event_id, body) -> String` (§4.4); re-exports `HEADLINE_EVENT`, `Headline`, `HeadlineRecipient` |
| `pub async fn home_operation(name, &StoreChat, &HomeInvocation) -> Option<Result<Value, ModuleError>>` | §5.9; `None` for a name that is not `chat.*` |
| `pub fn register(&mut Handlers)`, `pub fn register_with(&mut Handlers, Clock)`, `pub type Clock = Arc<dyn Fn() -> String + Send + Sync>` | the 11 handlers (§6); `register` uses core's ISO now |
| Pure helpers | `include_pending(Option<&str>) -> bool`, `slayer_link_availability(Option<&Value>) -> Value`, `slayer_link_status() -> Value`, `SLAYER_LINK_MAX_ACCOUNTS` (256) |

### 9.3 How it plugs into core

- **Routes**: the composer calls `register` on its `Handlers`; every path matches a core row (core §5.7).
- **Chat queries** (core §7.5): the composer's `ModuleHooks::chat` answers with `IslesChat` over its `StoreChat` and
  the module's `HubSessions`. The source is built at start with the module context's id (not a hard-coded
  `isles-1-14-7`).
- **Home operations** (core §7.8): the composer's home-operation hook gives `chat.*` names to `home_operation`.
- **Presence** stays core's (§5.5, §7.7); the offline hook (party leave) is matchmaking's.
- **Headlines**: the outbox (restricted, then row 8) builds the body here and emits `HEADLINE_EVENT`; core already
  declares it (§7.2).
- **Hub sessions**: a typed slot; matchmaking records, the rules read.

---

## 10. What stays in dainty-restricted

§1.3, plus restricted's `isles-chat` crate and `areas/chat*` themselves, deleted once the composer uses this crate.
No native code, no game data.

---

## 11. Test cases

### 11.1 What may be copied

Nothing recorded: `tests/golden/chat/*` stays in restricted and is replayed there. Restricted's tests are authored
code, described here only. The values below are this spec's own.

### 11.2 Behaviour the authored tests fix

**Routes** (an in-memory or temp store with accounts `alice`, `bob`, `carol`; a stand-in auth layer putting
`AuthData` in the extensions; the clock `2026-10-20T12:00:<n>.000Z` with `n` counting from 0 on each read):

| # | Request (caller) | Status | Body |
| --- | --- | --- | --- |
| 1 | `GET /friends/api/public/friends/alice` (alice) | 200 | `[]` |
| 2 | the same, no `AuthData` and no bearer | 401 | empty |
| 3 | `GET /friends/api/public/friends/zz` (empty caller) | 200 | `[]` |
| 4 | `POST .../friends/alice/bob` (alice), twice | 204, 204 | empty |
| 5 | `POST .../friends/alice/alice` (alice) | 400 | empty |
| 6 | `GET .../friends/alice` (alice) | 200 | `[]` (pending hidden) |
| 7 | `GET .../friends/alice?includePending=true` (alice) | 200 | one `bob`, `PENDING`, `OUTBOUND`, created at tick 1 (the second invite rewrote it), `favorite` false |
| 8 | `GET .../friends/bob?includePending=true` (bob) | 200 | `alice`, `PENDING`, `INBOUND`, tick 1 |
| 9 | `POST .../friends/alice/carol` (alice), then `POST .../friends/bob/alice` (bob) | 204 | — |
| 10 | alice's list with pending | 200 | `bob` `ACCEPTED` `OUTBOUND` created tick 3, then `carol` `PENDING` `OUTBOUND` tick 2 |
| 11 | bob's list | 200 | `alice` `ACCEPTED` `OUTBOUND` created tick 1 |
| 12 | `DELETE .../friends/alice/bob` (alice), then `DELETE .../friends/alice/nobody` | 204, 204 | — |
| 13 | `DELETE .../friends/carol/alice` (carol); alice's list with pending | 204; 200 | `[]` |
| 14 | `POST .../friends/alice/ghost` (alice); alice's list with pending | 204; 200 | `[]` (no edge for a stranger) |
| 15 | blocklist, recentPlayers (any namespace), settings, all without auth | 200 | §6.1's bodies |
| 16 | `GET /guild/invite/player`, `GET /guild`, `GET /slayerlink/status_good` (alice; then none) | 200/204/200; 401 | §6.1; empty |
| 17 | `GET /vivox/login` (alice; then none) | 404; 401 | empty |
| 18 | `POST /slayerlink/availability` `{"account_ids":["AB","",7,"CD"]}` | 200 | envelope, `AB` and `CD` unavailable |
| 19 | the same with `account_ids` a string; no body; `["a"]`; `text/plain` body; empty `application/json` body | 200 | envelope, empty list |
| 20 | 300 ids `ID0`..`ID299` | 200 | the first 256 |
| 21 | 64 and 65 copies of an astral emoji; a 129-character id | 200 | only the 64-emoji id |
| 22 | an id holding `"`, `\`, a newline, `U+0001`, `U+007F`, `/`, `U+2028` | 200 | escaped as §2 (`\"`, `\\`, `\n`, `\u0001`, the rest raw) |
| 23 | `{nope` as JSON with no auth; `7` as JSON with auth; 101 KiB of JSON | 400; 400; 413 | — |
| 24 | any store route with no store in the module | auth first, then 501 envelope | — |
| 25 | an invite whose friend's edge write gets `NotHome` | 503 | empty, warning logged |

**Rules** (a source in memory and over a temp store):

| Case | Expect |
| --- | --- |
| `Jid` of an account with userId `u-1` | `u-1@prod.ol.epicgames.com`, no resource |
| `Jid` of an unknown local account; of a hunt id; userId `has space` | `NotFound`; `Invalid`; `Invalid` |
| `Jid` / Party `Room` with no database | `Unsupported` |
| Party room: member (stored id in room spelling with a `_` suffix, or the full global id text); non-member; member of another party; party lookup fails | open; refused; refused; `Unavailable` |
| Region room: player recorded for region `local` at 0, asked at 1; another player; region `germany`; dev City switch on | open; refused; refused; open for anyone |
| Hunt room default; with the switch `false`; Network room | open; refused; refused |
| `RoomOf`: `Party-<room spelling>_QnVpbGQ=`, `Hunt-<UUID upper case>`, `City-<recorded session>`, `City-germany`, `City-1abc`, `City-`, `General`, `Party-xyz` | Party; Hunt on this node with the UUID's bits; the recorded region; `germany`; this node's region; none; none; none |
| Id reading | `ulid.node` round trip; full global text; another kind none; UUID either case; `0`×26 and the all-ones ULID texts; a 301-byte id none; lower-case ULID, `.Oregon`, bare ULID none |
| Login (a fake token check: `ok:<user>` valid) | `alice`/`ok:alice` the player; `forged` `jwt verify failed`; `ok:` `no userId in token`; `mallory`/`ok:alice` and `alice`/`ok:mallory` `authcid/token mismatch`; `ghost`/`ok:ghost` `account not eligible`; the client-credentials id refused; a banned launcher account `account not eligible`; a store failure `Unavailable` |
| Login over real tokens | a game token's `userId` read; a launcher token refused |
| Friends | only `ACCEPTED` edges, in order |
| `AccountOf` | a userId; ` Alice ` matching normalized `alice` of an eligible launcher account; an ineligible one none; blank none |
| `MayWhisper` | true |
| Hub sessions | §5.10's rules: last hub only; TTL inclusive at exactly 7 days; empty id ignored; the 50,001st account evicts the first |
| Settings | §8's table |
| Headline | `public_event("e1","PARTY_INVITE_CREATED","2026-10-20T12:00:00.000Z","p1_QnVpbGQ=",3,{"accountId":"bob","action":"KICK"})` → keys in §5.11 order with `party.invited`; an unknown type omits `eventType`; every name of the table; a `Headline` round-trips through JSON; `frame` escapes all five characters |
| `chat.login` home op | a valid login for another account `Invalid` `token account does not match target account`; `chat.player` unknown `{"player":null}` |

---

## 12. Security behaviour (for `docs/security/rustic-isles-chat.md` and RX3)

- **Identity is the token's.** The login binds the player from a valid game token whose `userId` equals the authcid;
  the JID, nickname and room name are never trusted for identity. The client-credentials token is never a player.
  Ineligible (banned, disabled, pending, rejected, timed-out, nameless) accounts cannot sign in to chat.
- **Room authorization is by account**: party rooms by stored membership, region rooms by the hub registry or the dev
  switch, hunt rooms by the dev switch.
- **Friends routes act only for the caller**: the path's `:accountId` is ignored, so one player cannot read or change
  another's list.
- **Secrets**: the token is never logged or put in an error text; refusal reasons are fixed texts; `StoreChat`'s and
  any token holder's `Debug` print no key or token (decision: secrets never in `Debug`). `ChatQuery::Login` derives
  `Debug` with the token in module-api: never format a query with `{:?}` (§13 item 5).
- **Inputs it parses from clients**: room local parts (`RoomOf`), the authcid and `AccountOf` local part, the friends
  path params and query, the Slayer link JSON body. All bounded: ids at most 300 bytes, query parsed linearly, body
  100 KiB through core, the availability list capped at 256 entries of at most 128 units.
- **Known weak spots to list**:
  - Hunt rooms are open to any signed-in player who names one (the switch defaults on, as Node's); the id has 80
    random bits.
  - The hub registry is memory only: a restart shuts every region room until players are sent to a hub again.
  - Friend invites to any existing userId are limited only by the request budget.
  - An invite's two edge writes are not one transaction (a failure leaves a one-sided edge).

**Fuzz target (`fuzz/fuzz_targets/`):** `rustic_isles_chat_names`: any bytes (lossy UTF-8) as a `RoomOf` name, an
id for §4.3, an `includePending` query and a `Jid` local part never panic and finish in time linear in the input;
an id longer than 300 bytes before `_` is never read; every id read back writes the same room spelling;
`slayer_link_availability` over arbitrary JSON never answers more than 256 entries, each 1 to 128 UTF-16 units; the
headline `frame` never holds an unescaped `<`, `&` or quote inside an attribute or the body.

---

## 13. Open questions and behaviour that looks wrong but is kept

Parity unless a decision says otherwise; each is a candidate decision row for the maintainer.

1. **No legacy names** (core §17 item 2): nothing here had any; none are added.
2. **Version string**: restricted picked `1.12.0` for a module id containing `1-12` in the cross-node ticket check.
   1.12 is dropped; the MIT crate uses the module's game version from core's tables. Same answer for 1.14.7.
3. **Nobody delivers headlines.** dainty-lass's chat plugin has no subscriber for `isles.party.headline`; the events
   are emitted and lost, so party updates reach the client only through polling. Proposed: a chat-plugin row that
   delivers `Headline` as §4.4's frame to every bound session of each recipient. Not this crate's.
4. **`Friends` is answered but not asked**: the plugin does no friend presence fan-out yet.
5. **`ChatQuery::Login`'s `Debug` prints the token** (module-api derive). Proposed: a module-api change to redact it.
   Not this crate's file; flagged.
6. **`:accountId` ignored** on every friends route (Node read the caller from its auth). Kept.
7. **An empty-identity caller** gets `[]` and 204s with nothing written. Kept.
8. **A stranger takes no edge**: Node stored an edge for any string; here both userIds must be local accounts (DD18).
   So friends across nodes cannot be made today (the friend's account is looked up only locally). Kept; a cross-node
   friend needs DN5 forwarding.
9. **Re-inviting** a pending friend rewrites `created` on both edges; **a blocked edge** is overwritten by an invite;
   **`favorite`** is reset to false by every invite and accept; on accept the caller keeps the invite time and the
   friend gets now. Kept.
10. **`DELETE` oneself** answers 204 (no self check, unlike POST). Kept.
11. **Body errors on B143 before auth**: restricted used its own reader (bare-status 400/413, no charset or encoding
    check, a BOM not dropped). Proposed: core's §3.9 reader, which is Express's and what Node ran; statuses match for
    every case the tests cover, the 400/413 bodies become Express's error pages, and 415 appears for a non-UTF-8
    charset. Decision wanted.
12. **Hub registry per module** instead of per process (core §17 item 14's reasoning). Proposed; the same for one module.
13. **City rules**: `City-<rest>` where `rest` happens to be a valid region name (a lower-case UUID starting with a
    letter is one) names that region, and any other non-empty `rest` names this node's region. Region rooms open by
    "last sent to a hub in this region", not by the exact hub (DD21 merges hubs per region). Kept.
14. **The module's `RoomOf` is shadowed** by the plugin's naming for the common spellings (§3): the hub-to-region
    mapping and the 300-byte limit matter only for odd names. Kept; the two id readers differ in their limit (64 in
    the plugin, 300 here) and in accepting the full global id text. Moving the room spelling into `module-api` would
    give one reader; a seam change, not this row's.
15. **History** is never kept (`history: false` everywhere), unlike the plugin's default for party and hunt rooms.
    Paradox's behaviour. Kept.
16. **Network room refused** although the plugin's naming maps `General` to it. Kept (Paradox has none).
17. **Clocks**: the hub check in restricted used the system clock while matchmaking recorded with its own. Here both
    sides use core's `replay::now_ms()` (the system clock outside a replay scope), so tests can pin it.
18. **`AccountOf`'s fallback** lower-cases ASCII only and trims Unicode whitespace; the launcher's own normalization
    may differ for non-ASCII names, in which case the lookup misses. Kept.
19. **The query parser** has no 1,000-pair cap (Node's `querystring` ignores pairs past 1,000). Only a request with
    more than 1,000 pairs before `includePending` differs. Kept.
20. **Two edge writes, two transactions** (§12). Kept for DD17: the friend's edge may live on another home.
21. **`REALTIME_XMPP_*` lack a server prefix** (AGENTS.md). They are Node's names and the host kit already writes one;
    kept.

---

## Coordinator decisions (2026-11-05, before implementation)

These settle §13 and override anything above that disagrees.

- **Item 1 (headlines undelivered), item 2 (`ChatQuery::Login`'s `Debug`), item 5 (one id reader in `module-api`):**
  outside this crate; each is a follow-up row for the chat plugin or `module-api`. This crate emits headlines as
  specified.
- **Item 3: parity.** `POST /slayerlink/availability` keeps restricted's own body reader (bare 400/413, no charset or
  encoding check, BOM kept), not core §3.9's.
- **Item 4:** the hub registry is per module, as proposed.
- **Item 8:** the hub check reads core's `replay::now_ms()`, as proposed (replay pinning is gated by core §17).
- **Everything else in §13:** parity, as listed.
