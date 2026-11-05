<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false, "code_blocks": false } } -->

# Clean-Room Behaviour Specification: `crates/rustic-isles-characters` (RD27)

> **Scope and purpose (RD27):** this is the clean-room behaviour specification for an MIT crate,
> `rustic-isles-characters`: the characters-and-loadouts area of Rustic Isles (`GET`/`PUT`/`POST /character` and the
> five `/loadout` routes), the character-data rules every read and write applies, and the starter manifest a new
> character is seeded with (and the repair a read does when a starter record is missing). It is unit 5 of
> [`spec-rustic-isles-core.md`](spec-rustic-isles-core.md) §16, plugs into core as §8 here says, and reads and writes
> only through [`spec-rustic-isles-store.md`](spec-rustic-isles-store.md)'s repositories. Its source is
> dainty-restricted's `modules/rustic-isles/crates/isles-core/src/areas/characters.rs` and `areas/characters/*`,
> which are AGPL-3.0-only with Mystic Paradox's section 7 terms; an implementer who never sees them writes the MIT
> crate from this document alone.
>
> Everything below is stated from observable contracts: the routes and their bytes, the stored rows, the order of
> replay-seeded id draws, the public names other crates call, and the tests. No implementation code is reproduced and
> no private helper is named. Node names (`GetCharactersForUid`, `routes/character.ts`) appear only as provenance.
> Game-derived values (catalog ids, the default appearance, the tutorial flag names, the Dojo series blob, the starter
> wallet) are **not** in this document: they are inputs the version crate supplies (§6), and every test here uses
> synthetic ones.
>
> Written against dainty-restricted `e0e238b4434316bb210677d88bc59e3448688794` (worktree
> `splashloader-integration-90130d`) and dainty-lass `b4172b7b980905e1a33e175cda075bbf6a46b60e` (branch
> `feat/isles-auth-clean-room`).

---

## 1. Provenance and licensing classification

### 1.1 Evidence used

| Source | What it says |
| --- | --- |
| `modules/rustic-isles/NOTICE.md` | Every file of the module is AGPL-3.0-only with Mystic Paradox's section 7 terms |
| File headers (`//!`) | `characters.rs` ports `routes/character.ts` and `routes/loadout.ts`; `controller.rs` `controllers/character.ts` and `controllers/loadout.ts`; `data.rs` `controllers/character.ts`'s data rules; `starter.rs` `controllers/starterManifest.ts` (and `STARTER_WALLET` from `controllers/wallet.ts`) |
| `golden.rs` | RU5's golden replay rig for this area (stays in restricted, §13.1) |
| `tests/characters.rs` (isles-core) | Authored integration tests: repositories, unlock gate, the 501 without auth, starter id uniqueness (described only, §13) |
| `crates/isles-1-14-7/src/lib.rs` | 1.14.7's `starter_manifest` table (bootstrap version, stacked ids, quick emojis): read only to learn which values are the version's |
| Callers in `areas/{inventory,progression,bounties_store}`, `isles-db/src/transforms.rs` | What other units call (§8.3) |

### 1.2 Classification and what is rewritten

All four files are Category B (they port Paradox TypeScript); `golden.rs` is Category A test rig. **The implementer
writes all of B from this spec**; A stays in restricted.

| Restricted file (`src/areas/`) | Rewritten as |
| --- | --- |
| `characters.rs` (handlers, auth entry, registration) | §3, §4, §8 |
| `characters/controller.rs` (account lookup, character and loadout controllers) | §3, §5 |
| `characters/data.rs` (normalisation, progression guard, Dojo series, dates) | §5.4 to §5.8 |
| `characters/starter.rs` (manifest, instance ids, repair, chat wheels) | §6, §7 |
| `characters/golden.rs` | Not rewritten (§13.1) |

### 1.3 What stays in dainty-restricted

- `golden.rs`, `tests/golden/characters/*` and `crates/isles-1-14-7/tests/characters_golden.rs`: run there against
  this crate once composed; a failure there is a bug on this side.
- **The 1.14.7 values of §6's inputs** (in restricted's version crate, `isles-1-14-7`, until unit 12 decides where
  they live; §15 item 3).
- The adapters restricted's remaining areas use until they are ported (inventory's `StarterBootstrap` and
  `Characters` ports, progression's `StarterBootstrap` and `JourneyInventory`, bounties-store's first-character
  lookup, `isles-db`'s starter id set): each becomes a call to §8.2.

---

## 2. Conventions

- **JSON** on the wire as core §2.1 (`service_kit::json::Json`, insertion order); every key order below is the wire
  order. **Stored and nested JSON text** (character `data`, its double-encoded members, loadout and inventory columns)
  is `service_kit::jsjson::stringify`'s compact JavaScript text; "parse" is `service_kit::jsjson::parse`
  (`JSON.parse`). Map keys keep insertion order; an assignment to an existing key keeps its position, a new key is
  appended (JavaScript's rule).
- **Empty** is the status with no body and no content type. **500 page** is `service_kit::body::error_page(500)`
  (Express's HTML page for a thrown error). **501 envelope** is core §3.6's `NOT_IMPLEMENTED` answer for the call's
  route and request id.
- **Bodies** are read with core's `read_body` (§3.9 there: 100 KiB, JSON or form, "absent" otherwise); a body error is
  answered as core makes it (400/413/415 pages).
- **Params** are core's decoded path params (never empty, core §4.1). `Number(x)` is JavaScript's
  (`service_kit::js::number`: trims white space, `""` and `" "` are 0, `0x`/`0o`/`0b` prefixes, exponents,
  `Infinity`, else NaN). `String(x)` is `service_kit::js::string` (`5` → `5`, `true` → `true`, an object →
  `[object Object]`, an array → its elements joined with `,`).
- **A body field as text** ("`body.f` as text"): absent or `null` is none; a string is itself; anything else is
  `String(value)`.
- **Clock**: `now_secs` = `floor(rustic_isles::replay::now_ms() / 1000)`; **ids**: `rustic_isles::replay::next_uuid()`
  (both honour the replay scope, core §9). Nothing here reads the system clock or RNG.
- **Dates**: `login_time(s)` is `YYYY.MM.DD-HH.MM.SS` in UTC, zero-padded (0 → `1970.01.01-00.00.00`,
  1792000000 → `2026.10.14-17.46.40`). `display_date(s)` is `toLocaleDateString("en-US", {month:"short",
  day:"numeric", year:"numeric"})` in UTC, independent of any locale setting: `Mon D, YYYY` with English month
  abbreviations and an unpadded day (1792000000 → `Oct 14, 2026`; 951782400 → `Feb 29, 2000`).
- **The sentinel** `INVALID` is UE4's "no logged-in player" userId (a client wire value). "No player" means the user
  is none or exactly `INVALID`.
- **Wire field names** in this document (`accountId`, `PlayerAccountProgressStep`, `slot_index`, `quick_chats`, ...),
  the client's enum texts (`EArchonCharacterCreationState::FaceComplete`, the `EPlayerAccountProgressState` names of
  §5.5), the 409 page and the JSON refusal codes are interface facts the 1.14.7 client reads, and are kept.

---

## 3. Routes

Eight backend rows of core's table (Appendix A there), group `characters-loadouts`, in this order:

| Row | Method | Path (exactly as the table writes it) |
| --- | --- | --- |
| B65 | GET | `/character` |
| B66 | PUT | `/character` |
| B67 | POST | `/character` |
| B132 | GET | `/loadout/:userId//all` |
| B133 | GET | `/loadout/:userId/:characterId/all` |
| B134 | POST | `/loadout/:userId/:characterId/unlock/:numSlots` |
| B135 | POST | `/loadout/:userId/:characterId/active/:index` |
| B136 | POST | `/loadout/:userId/:characterId/:index` |

Matching is core's (Express 5): `HEAD` reaches the `GET` rows; the empty middle segment of B132 is a literal, so
`/loadout/u//all` is B132 and never B133; `POST /loadout/u/c/all` and `POST /loadout/u/c/active` are B136 with
index `all` / `active` (refused, §3.8); `POST /loadout/u/c/unlock/3` is B134 (five segments).

### 3.1 Entry: authentication, store, order of checks

**Authenticate** (every route):

1. When the request's extensions already hold auth's `AuthData` (auth §11.2), it is used.
2. Otherwise, when the request has **none** of `authorization`, `x-rustic-isles-gameserver-apikey`,
   `x-rustic-isles-gameserver-key`: the **501 envelope** (parity with restricted; §15 item 1).
3. Otherwise `shared.auth().game_auth(request)` (auth §4.1): its refusal is the answer (401/403 empty, 500 page, a
   realm refusal); its `AuthData` is the caller. game auth also rewrites an `INVALID` path segment for a keyed `GET`
   (auth §4.1.1), but the params core already bound keep `INVALID` (that is what lets B132/B133 see the sentinel).

The caller, as these routes read it: `user` (auth's `user_id`), `is_gameserver`, and `host_runtime` (auth's
`runtime`; never set with `is_gameserver`). A session-ticket caller's `user` is the admitted account's global id
text (auth §4.1 step 3), which this crate treats like any userId (§15 item 9).

**Store**: `shared.slot::<IslesStore>()` (core §5.1's typed slot). None: the **501 envelope**. **Area state**:
`shared.slot::<Characters>()` (§8.2). None: the **501 envelope**.

**Order per route** (the first step that answers ends the request):

| Route | Order |
| --- | --- |
| GET `/character` | authenticate, store, §3.2 |
| PUT `/character` | read body, authenticate, store, body absent → 500 page, §3.3 |
| POST `/character` | read body, authenticate, store, body absent → 500 page, §3.4 |
| GET B132 | authenticate, §3.5 (no store needed) |
| GET B133 | authenticate, sentinel check, store, §3.6 |
| POST B134 | authenticate, the unlock gate (§3.7; may need the store), validation, store, the rest |
| POST B135 | authenticate, validation, store, the rest (§3.8) |
| POST B136 | read body, authenticate, body not an object → empty 400, sentinel check, store, the rest (§3.9) |

A body error therefore answers before an auth refusal on PUT, POST `/character` and B136.

**The actor** of every write (store §2.3), with the request id attached (`ActorContext::with_request`): `Player
{ account }` when the caller is neither a game server nor a host runtime and the acting userId already has an
account (§5.1 lookup, before any creation); otherwise `System`. B134 always writes as `System`.

**A failure** (a repository error, a data error of §5, a `Failed` text of §5.9) is logged at error level
`{METHOD} {route path}: {error} (request {request id})` and answered with the **500 page**.

### 3.2 `GET /character`

1. `user` = the caller's user. When it is "no player" and the caller is a game server, the version's no-player
   policy is asked for its **bearer-less account** (core §5.3: `GAMESERVER_NO_PLAYER_USER_ID`, else `DEV_USER_ID`,
   trimmed, none when blank or when the policy is `Off`); when it names one, that is the user (logged at info
   `GET /character: gameserver no-player request resolved to '{account}'`).
2. Still none or `INVALID`: **200 `[]`**.
3. Otherwise §5.2 "characters for a user" (creating the account and the first character when there are none):
   **200** with the JSON array of wire characters (§5.3), in creation order. Logged at info
   `Retrieved {n} characters for {user}`.

### 3.3 `PUT /character` (create)

1. `name` = `body.name` as text, else the literal `undefined`.
2. The caller's user is "no player": **empty 400** (a bearer-less game server under the `Off` policy, a token whose
   userId is `INVALID`, a host runtime).
3. §5.2 "create a character" for that user with `name`: **200** with the new wire character (§5.3; its Dojo flag is
   always off). Logged at info `Creating a character named {name} for user {user}`.

There is **no limit** on characters per account, and no name rule: any text is kept as it is (no trim, no length cap
beyond the body limit, not unique). There is no delete route; characters leave only through the account erasure of
other units (store §11.3.6).

### 3.4 `POST /character` (save)

1. `characterId` = `body.characterId` as text (none when absent).
2. **The acting user**: for a game server, `body.accountId` as text, else the caller's user; for anyone else the
   caller's user (a player cannot name another account).
3. **The hub's phantom save**: when the version's no-player policy is `ConfiguredAccount`, the caller is a game
   server, `characterId` is none or empty, and the acting user is not "no player": §5.2 "characters for a user" for
   that user (which creates the first character if needed); the first one's non-empty `id` becomes `characterId`
   (info `POST /character (no characterId): gameserver request resolved to '{id}' for {user}`), else warn
   `POST /character (no characterId): no character exists for {user} - the write cannot be attributed`. A failure
   here is the 500 page.
4. The acting user is "no player": **empty 400**.
5. `characterId` still none: the literal `undefined`.
6. §5.6 "update a character" with `body.data`, `body.updateVersion` and whether the caller is a game server:
   - refused (`false`): **409** with the conflict page below (warn `Update conflict for characterId {id} (userId
     {user}) - returning HTTP 409 HTML`);
   - accepted: the character re-read (§5.2 "one character"): found → **200** with it; not found → **200** with
     `content-type: application/json; charset=utf-8` and an empty body (Node's `res.json(undefined)`).

**The conflict page** (a wire constant: the client recognises the 409 and follows with a `GET /character`; it may
compare the bytes, so the spacing, the space after `409` in the title and the four spaces before `Conflict` are
kept). Status 409, `content-type: text/html; charset=utf-8`, body exactly these 12 lines, each ending in `\n`:

```text
<html>
<head>
<meta http-equiv="Content-Type" content="text/html;charset=ISO-8859-1"/>
<title>Error 409 </title>
</head>
<body>
<h2>HTTP ERROR: 409</h2>
<p>Problem accessing /character. Reason:
<pre>    Conflict</pre></p>
<hr />
</body>
</html>
```

(245 bytes in all. The `meta` says ISO-8859-1;
the header says `utf-8`, as Express relabels a string body.)

### 3.5 `GET /loadout/:userId//all`

The **requestor** of a loadout route: for a game server the `userId` param, otherwise the caller's user.

- Requestor exactly `INVALID`: **200** with the **empty loadout answer** (§4.2).
- Anything else (a player, none, another id): **empty 400**.

### 3.6 `GET /loadout/:userId/:characterId/all`

1. Requestor exactly `INVALID`: **200** empty loadout answer, no store touched.
2. Store (501 if none). User = the requestor, or the empty string when none.
3. §5.9 "loadout state" for (user, `characterId`): **200** with the loadout answer (§4.1); info
   `Fetched {n} loadout(s) for userId {user} and characterId {characterId}`. Not the user's character (or no
   account): **empty 400** (warn `GET /loadout rejected for userId {user}: character {characterId} does not belong
   to the user`).

### 3.7 `POST /loadout/:userId/:characterId/unlock/:numSlots`

The **target** is the `userId` param for every caller. The path count is only a hint: the entitlement is derived from
the player's Slayer's Path, so no caller can grant more slots than the player unlocked.

**The gate**, in order:

1. **A host runtime**: store needed (501 if none). The target's account (§5.1 lookup): none → **403**
   `{"code":"P2P_HOST_WRITE_DENIED","message":"This host may not write this player's state.","payload":null}`; a
   lookup failure → 500 page. Then `shared.rifts.guard().authorize_host_write(runtime, HostWriteKind::LoadoutUnlock,
   AccountId(<the account's global id text>), HostWriteOptions::default())`: no refusal → admitted; a refusal
   `Reply { status, body }` → that status (one that is not a valid HTTP status: 403) with `content-type:
   application/json; charset=utf-8` and the body text as it is; a seam error → logged `host write authorization:
   {error}`, 500 page. (The `RuntimeAuth` this needs is §15 item 2.)
2. **A game server**: admitted (any account), no hook asked.
3. **Anyone else** (a player's own token, or no authority at all): **403**
   `{"code":"forbidden","message":"Loadout slot unlocks require gameserver authority.","payload":null}`, warn
   `POST character loadout unlock rejected without gameserver authority userId={target} characterId={characterId}`.

**Validation**: `requested` = `Number(numSlots)`; valid when a whole number, at most 2^53 − 1 in magnitude and in
1..6. Target exactly `INVALID`, or not valid: **400**
`{"code":"invalid_loadout_slot_count","message":"Loadout slot unlock delta must be between 1 and 6.","payload":null}`.

**Then** (store needed; actor `System`):

1. The target's journey nodes: the target empty or `INVALID`, no account, or no journey row → none; else the row's
   `nodes` parsed (an unparsable text reads as `{}`). `total` = store's `resolve_player_journey_total(nodes)` (1 plus
   the `LoadoutSlot_01`..`05` nodes whose `node_status` is the number 2).
2. §5.10 "ensure total slots" (target, `characterId`, `total`), then §5.9 "loadout state".
3. Not the target's character: **empty 400** (warn `POST character loadout unlock rejected for userId {target}:
   character does not belong to the user`). A failure: 500 page.
4. Info `Loadout slot entitlement reconciled userId={target} characterId={characterId} gameserverDelta={requested}
   authoritativeTotalSlots={n} wire=account:{a}/{b},character:{c}/{d}` (the four numbers of §4.1).
5. **A host runtime's unlock is tagged**: when the caller is a host runtime and the target's account is found,
   `record_host_write(runtime, HostWrite { kind: LoadoutUnlock, target: <account text>, summary })` with summary the
   compact text `{"gameserverDelta":<requested as a float>,"authoritativeTotalSlots":<n>}` (`1.0`, not `1`; §15
   item 5): counts only, never the character id. A failure is logged at error `[HostWrite] tag failed
   kind=LOADOUT_UNLOCK target={target}: {error}` and does not change the answer.
6. **200** with the loadout answer.

### 3.8 `POST /loadout/:userId/:characterId/active/:index`

1. `index` = `Number(index)`; in range when a whole number with 0 ≤ index < 6. Requestor "no player" (none or
   `INVALID`), or out of range: **400**
   `{"code":"invalid_active_loadout","message":"Active loadout index is invalid.","payload":null}`.
2. Store. §5.11 "set the active index" (requestor, `characterId`, index):
   - not the user's character: **empty 400** (warn `POST active loadout rejected for userId {user}: character does
     not belong to the user`);
   - refused (`false`): **400**
     `{"code":"loadout_slot_not_unlocked","message":"The requested loadout slot is not unlocked.","payload":null}`;
   - failure: 500 page.
3. §5.9 "loadout state": **200** with the loadout answer (info `Active loadout updated userId={user}
   characterId={characterId} index={active_index}`).

### 3.9 `POST /loadout/:userId/:characterId/:index` (replace a slot or `persistent`)

1. Body absent or not a JSON object: **empty 400** (warn `POST /loadout rejected: request body is not an object`).
2. Requestor exactly `INVALID`: **empty 400**. (None goes on as the empty user and fails at step 3.)
3. Store. §5.12 "set loadout data" (user, `characterId`, the `index` param text, `body.data`): refused → **empty
   400** (error `Failed to update loadout index {index} for userId {user} and characterId {characterId}`);
   accepted → §5.9 "loadout state" → **200** with the loadout answer. A failure: 500 page.

---

## 4. Answers

### 4.1 The loadout answer

```text
{"code":null,"message":"OK","payload":{"loadouts":[<visible slots>],"persistent":<persistent>,
 "num_account_slots":0,"max_account_slots":0,"num_character_slots":<n>,"max_character_slots":6,
 "active_index":<i>,"needs_migration":false}}
```

(one line on the wire; keys in this order). `<visible slots>` and `<persistent>` are §5.9's state; the four counts
are store's `wire_counts(<number of visible slots>)` (`0`, `0`, the count clamped to 1..6, `6`); `active_index` is
the state's.

### 4.2 The empty loadout answer (the hub phantom's)

Byte for byte:

`{"code":null,"message":"OK","payload":{"loadouts":[],"persistent":null,"num_account_slots":0,"max_account_slots":0,"num_character_slots":1,"max_character_slots":6,"active_index":0,"needs_migration":false}}`

It is never a 2xx for B134: the sentinel is refused there (a 2xx would loop the client's success branch).

### 4.3 The wire character

`{"accountId":<the userId the request named>,"catalogDaoId":null,"createdDate":..,"data":<text>,"id":<character
id>,"lastModifiedDate":..,"name":..,"updateVersion":<integer>}` in that order. `accountId` is the client's userId
(never the account's global id); `data` is a JSON **string** holding §5.7's read form of the stored data.

---

## 5. Business rules

### 5.1 Accounts

A client names a player by **userId**; rows belong to the account's global id (store §2.1), found with
`accounts.find_by_user_id`.

- **Lookup**: the live account with that userId, or none.
- **Find or create**: the lookup; when none, one `IslesStore::write` (no home check: the account is new here) that
  looks again inside the transaction and otherwise creates `AccountRecord { id: AccountId::new(<this node>),
  user_id, name: <the userId>, notes: 0 }`. A trusted caller's userId (a game server's configured account, a dev or
  service token, a session ticket's global id text) gets an account this way; Node kept characters under any userId.

### 5.2 Characters

**Username** (the first character's name): the launcher account of that account (`launcher_accounts.find_by_account`)
when it exists and its `display_name` is not empty; else the account's `name`; else the userId. A read error at either
step falls through to the next.

**Create a character** (inside one `account_write` with the caller's actor; one transaction):

1. `character_id` = the next replay id as hyphenated text (**the first id draw**).
2. `CharacterRecord { character_id, account, created_date: display_date(now), last_modified_date: <the same>, name,
   update_version: 0, data: normalize("{}") }` (§5.4), created with `characters.create`.
3. §7.1 "seed a new character" (inventory, loadout, wallet) in the same transaction (**the instance id draws**).

The PUT route does: find or create the account, then create, then answers the wire character with the Dojo flag off.

**Characters for a user** (`GetCharactersForUid`; also what other units call, §8.2):

1. Find or create the account.
2. Read the account's live characters (`characters.find_many_by_account`, creation order) and, when there is at least
   one, the Dojo flag (§5.8; a journey read error is logged at warn `[dojo-reconcile] Unable to read Player Journey
   for account {account}: {error}` and reads as locked).
3. None: one `account_write` (the caller's actor) that reads again, does nothing if a concurrent request made one,
   and otherwise creates a character named after the username. Then step 2 again.
4. Each record wired (§4.3) with that one Dojo flag and the same `now`.

**One character** (`GetCharacterWithUid`): the account lookup (none → none); `characters.find(id, account)` (another
account's character is none); found → wired with its Dojo flag; else none.

### 5.3 Wiring

§4.3, with `data` = §5.7 "the read form" of the stored `data` with the account's Dojo flag. Wiring does not write:
the normalisation on the way out is recomputed on every read. A stored `data` that fails §5.4 is a data error (500).
`lastModifiedDate` never changes after creation (no write updates it; §14 item 6).

### 5.4 Normalise (`NormalizeCharacterData`)

Input: a text (empty reads as `{}`), the version's changelist, `now_secs`. The text must parse to a JSON object; else
a data error (`character data is not a JSON object`, or `character data: {parse error}`). In order:

1. **Fill**: for each `(key, value)` of the version's **fill list** (§6.1), in order, set `key` to `value` when it is
   absent or `null`. Then the same for `LoginTime` and `LastLoginTime` with `login_time(now)`.
2. **`LastChangelist`** = the changelist as decimal text (always assigned).
3. **The returning-player state** (every account reads as "onboarding complete, in Ramsgate", so the login screen goes
   straight to the hub; the 1.14.7 tutorial hunt starves on beginner-loadout data and its cinematic recurses):
   1. `PlayerAccountProgressStep`: when its ordinal (§5.5; `null` reads as absent) is below `EnteredRamsgate`'s (3),
      set to `EnteredRamsgate`.
   2. `AppearanceData`: take it as an object (an object as it is; a string that parses to an object; anything else
      the version's **default appearance** object, §6.1). Set `CreationState` to
      `EArchonCharacterCreationState::FaceComplete`. When `Data` is not a non-empty array: the default's `Data`.
      When `StringData` is not a non-empty array, **or any entry is "legacy"**: the default's `StringData`. An entry is
      legacy when it is an object that has a `CustomizationName` or `ValueName` key or lacks `Key`, or when it is not an
      object and not `null`, `false` or `""` (a number, `0` included, `true`, a non-empty string, an array). When
      `AssetReferences` is not an array: `[]`. Write it back as its compact text (a string).
   3. `CharacterFlagData`: take the blob as an object (as above; else `{"Flags":[]}`), its `Flags` when an array
      (else empty). For each `(key, value)` of the version's **forced character flags** (§6.1), in order, whose key is
      not already a string `FlagKey` of an entry, append `{"FlagKey":key,"FlagValue":value}`. Set the blob's `Flags`
      to the list (its other members and their order kept) and write the blob back as its compact text.
   4. `LoginFlagData`: the same with the one **forced login flag** (§6.1).
4. **The progression invariant**: when the appearance's `CreationState` is not `FaceComplete`, a progress ordinal of
   `SavedCharacter` (2) or more is set back to `DefeatedGnasher`, a top-level member named like the forced login
   flag's key is removed, and entries with that `FlagKey` are removed from `LoginFlagData`'s `Flags`. After step 3
   the state is always complete, so this step changes nothing in a normalise; it is kept so the rule stands where
   data reaches it otherwise.
5. The result is the object's compact text.

Normalise is idempotent (a second run, even at a later `now`, gives the same text, because the times are only
filled when absent).

### 5.5 Progress ordinals (`EPlayerAccountProgressState`, client enum names)

`New` 0, `DefeatedGnasher` 1, `SavedCharacter` 2, `EnteredRamsgate` 3, `FinishedFirstHunt` 4, `FinishedSecondHunt`
5, `Final` 5. Absent, `null`, a non-string or an unknown name: 0. The name of ordinal n is the list's n-th of the
first six (0 → `New` .. 5 → `FinishedSecondHunt`).

### 5.6 Update a character (`UpdateCharacterForUid`)

Inputs: the acting user, `characterId`, `body.data`, `body.updateVersion`, whether the caller is a game server.
Answers accepted or refused.

1. **Current**: §5.2 "one character". None: **refused** (warn `UpdateCharacterForUid: no character record for
   characterId {id} userId {user} (isGameserver={bool}) - treating as a rejected write instead of throwing`).
2. `current` = the current `updateVersion`; `requested` = `Number(body.updateVersion)` (absent → NaN);
   **effective** = `requested` when it is a whole number of magnitude at most 2^53 − 1, else none.
3. When `current >= requested` (false for NaN): a player is **refused**; a game server is authoritative, so
   effective = `current + 1` (info `[gameserver-force] Stale gameserver write for characterId {id} - was
   updateVersion {requested}, forcing to {current+1} (server is authoritative for progress/PJM)`).
4. **The new data** (computed before anything else is decided, so bad data is a 500 even when nothing is written):
   1. `incoming` = `body.data` when it is a non-empty string; else `String(body.data)` when it is truthy; else `{}`.
   2. **Progression guard** (§5.7) of `incoming` against the current wire `data` (the read form).
   3. **Triggers** (§5.8) of that, then normalise (§5.4).
   4. **Keep the Dojo series**: when the current wire data has a non-null `SERIE_dojo` (its key from §6.1), the
      legacy Dojo series is ensured on the result (§5.8; info `[dojo-reconcile] Persisting SERIE_dojo for userId
      {user} characterId {id}` when it was added), so a stale client blob cannot remove the gate again.
5. Effective none (NaN, a fraction, too large, from a player whose version was newer): **accepted with no write**
   (§14 item 3).
6. The account lookup: none → **refused**.
7. One `account_write`: `characters.update_data_conditional(id, account, data, effective)` (writes only when the
   stored version is below `effective`; a lost race is silent). **Accepted**.

### 5.7 Progression guard and the read form

**Guard** (`PlayerAccountProgressStep` moves forward only, one step at a time): parse both texts (empty reads as
`{}`). Either not a JSON object (or not parsable): warn `[progression-guard] Non-JSON incoming data or parse error -
skipping guard` and the incoming text unchanged. Otherwise, with ordinals of §5.5: incoming below existing → the
incoming step is set to the existing value (warn `[progression-guard] Rejecting backward progression; keeping
existing`); incoming more than one above existing → set to the name of existing + 1 (warn `[progression-guard]
Rejecting skip; clamped to {name}`); then the incoming object's compact text.

**The read form** (`EnsureLegacyDojoSeries`): normalise (§5.4); then when the Dojo flag is on and `SERIE_dojo` is
absent or `null`, set it to the version's **Dojo series** text (§6.1). Answers the text and whether it was added.

### 5.8 The Dojo series

- **Dojo flag** (`IsDojoUnlockedForUser`) from the journey row's `nodes` text (empty reads as `{}`; unparsable:
  off): on when the member named by the version's **Dojo node id** has `node_status` the number 1 or 2, or when any
  member's value has `node_id` equal to that id and `node_status` 1 or 2 (captures whose map key differs).
- **Trigger** (`ProcessTriggers`): normalise first; then when the version's **trigger series** key is present and
  not `null`: its value as a string is parsed (a parse error is a data error: 500); any other value is parsed from
  `String(value)` (an object becomes `[object Object]`: a data error; a number parses to a number). When that value's
  member named by the **trigger node** has `Status` equal to the **trigger status** and `SERIE_dojo` is absent or
  `null`: `SERIE_dojo` is set to the Dojo series text (info `Injecting SERIE_dojo!`). The compact text.
- **Fresh onboarding** (`IsFreshOnboarding`, for matchmaking's FTUE choice): an account with no live character is
  fresh; otherwise fresh unless some character's `data` parses to an object whose appearance (as an object or a
  parsed string) has `CreationState` `FaceComplete` and whose progress ordinal is at least `SavedCharacter`.
  Unparsable data counts as fresh.

### 5.9 Loadout state (`GetLoadoutStateForUserIdAndCharacterId`)

1. **Ensure the starter records** (§7.2) for (user, character). Not owned: the route's "not the user's character".
2. From the loadout row:
   - `loadouts` parses to an array of 1 to 6 entries, else `Failed`: `Persisted loadout JSON is invalid for character
     {id}` (unparsable), `Persisted loadout count {n} is invalid for character {id}`, `Persisted loadout count
     non-array is invalid for character {id}`.
   - **Visible** = store's `resolve_visible_total(<array length>, <unlocked_total_slots>)` (its `SlotRangeError` text
     is a `Failed`).
   - `persistent` parses, else `Failed`: `Persisted loadout persistent is invalid: {error}`.
3. State: the first *visible* entries; `persistent` with the **default chat wheels** (§6.3); `active_index` =
   store's `resolve_active_index(<stored index>, visible)`.

### 5.10 Ensure total slots (`EnsureTotalLoadoutSlots`)

Grows the stored slots to the entitlement and records it. Never relocks a slot; stored excess slots stay dormant.

1. `desired` outside 1..6: `Failed` `Total loadout slot count must be between 1 and 6` (unreachable from B134).
2. Ensure the starter records (§7.2). Account lookup: none → not owned.
3. One `account_write`, up to **10 attempts** inside it:
   1. The loadout row; none → `Failed` `Bootstrap recovery did not create loadout for {id}`.
   2. Parse `loadouts` as in §5.9 (same errors). When `unlocked_total_slots` is stored, `resolve_visible_total` must
      accept it (its error is a `Failed`).
   3. `target` = store's `resolve_requested_total(<stored entitlement>, desired)` (the larger of the two).
   4. Extend: while the array is shorter than `target`, append a copy of entry 0 with `slot_index` = its new index,
      `update_version` 0 and `custom_name` `""` (existing keys keep their position, missing ones are appended).
   5. Nothing appended and the stored entitlement already equals `target`: done, no write.
   6. `loadouts.replace_all_and_entitlement_if_revision_matches(<the array's jsjson text>, target, <stored revision or
      0>)`: applied → done; else warn `Loadout unlock revision conflict for characterId {id}, attempt {k}/10 -
      retrying` and the next attempt.
   7. After 10: `Failed` `Loadout unlock for characterId {id} failed after 10 revision-conflict retries`.

### 5.11 Set the active index (`SetActiveLoadoutIndexForUserIdAndCharacterId`)

Index outside 0..5: refused. Ensure the starter records (§7.2; not owned → not owned). Account lookup: none →
refused. One `account_write`, up to 10 attempts: no row → refused; parse and visible as §5.9 (errors are failures);
index ≥ visible → refused; `loadouts.update_active_index_if_revision_matches(index, <revision or 0>)` → true:
accepted (store answers true for the index already stored whatever the revision, store §16 item 8); false: warn
`Active loadout revision conflict for characterId {id} index {index}, attempt {k}/10 - retrying`. After 10 attempts
(error log `Active loadout write for characterId {id} index {index} failed after 10 attempts`): refused.

### 5.12 Set loadout data (`SetLoadoutDataForUserIdAndCharacterId`)

**Validation** (`ValidateLoadoutWriteData`; refused with an error log `Loadout write rejected for characterId {id}:
{reason}`), first failing reason:

| Check | Reason |
| --- | --- |
| index is `persistent`, or the canonical decimal text of 0..5 (`0`..`5`; not `05`, `+1`, `-0`, ` 1`) | `unsupported loadout index {index}` |
| `data` is a non-empty JSON string | `data must be a non-empty JSON string` |
| its length is at most 1,048,576 UTF-16 units | `data is too large` |
| it parses | `data is not valid JSON` |
| it parses to an object | `data must be a JSON object` |

Then: account lookup (none → refused). One `account_write`, up to 10 attempts, **without ensuring the starter
records first**: no loadout row for (character, account) (also another account's character) → refused; `persistent`
→ `loadouts.update_persistent_if_revision_matches(<data text as sent>, revision)`; a slot index →
`loadouts.replace_slot_if_revision_matches(index, <data text>, revision)` (store refuses an index at or beyond the
visible total, and the attempt loop retries that refusal, §14 item 9). Applied → accepted; else warn `Loadout
revision conflict for characterId {id} index {index}, attempt {k}/10 - retrying`; after 10 (error `Loadout write for
characterId {id} index {index} failed after 10 revision-conflict retries`) refused.

---

## 6. Inputs the version supplies (game-derived, not committed here)

### 6.1 `CharacterData` (the character-data rules' values)

| Field | Shape | Used by |
| --- | --- | --- |
| `fill` | ordered `(key, JSON value)` list | §5.4 step 1 (1.14.7: nine entries, starting `RecentPlayers`, `AppearanceData` (the default appearance's compact text), `PlayerAccountProgressStep` (`New`), `CharacterFlagData`, then repair and quest-series blobs; each value is a string) |
| `default_appearance` | JSON object with `Data` and `StringData` arrays | §5.4 step 3.2; §6.2's slot `appearance` |
| `forced_character_flags` | ordered `(flag key, flag value)` list | §5.4 step 3.3 (1.14.7: 18 entries, value `True`) |
| `forced_login_flag` | one `(flag key, flag value)` | §5.4 steps 3.4 and 4 (1.14.7: value `true`, lower case) |
| `dojo_series_key` | text | the member §5.6-§5.8 read and set (1.14.7: `SERIE_dojo`, the name this spec uses) |
| `dojo_series` | JSON object | stored as its compact text (§5.7, §5.8) |
| `dojo_node_id` | text | §5.8's journey node |
| `trigger_series_key`, `trigger_node`, `trigger_status` | text, text, number | §5.8's trigger (1.14.7's status is 3) |

The changelist is core's `target_changelist` table.

### 6.2 `StarterKit` (the manifest's values)

| Field | Shape | Used by |
| --- | --- | --- |
| `instanced` | ordered catalog ids | §7.1 instanced items, in this order (1.14.7: 23) |
| `default_weapon_parts` | ordered catalog ids, a subset of `instanced` | §7.2 reconcile (1.14.7: 8) |
| `equipment` | ordered `(slot key, catalog id)`; keys are wire keys of a loadout slot | §6.4 (1.14.7: `weapon`, `helmet`, `chest`, `arms`, `legs`, `lantern`) |
| `instance_data` | `(catalog id, JSON object)` list | the equipment entry whose catalog id has one gets its compact text as `instance_data`; the others `null` (1.14.7: the weapon only) |
| `flask`, `banner`, `glider` | catalog ids in `instanced` | §6.4 (their instance ids, not catalog ids, go in the slot and `persistent`) |
| `banner_customization` | JSON object | `persistent.bannerCustomization` as its compact text |
| `intro_emote`, `flare` | catalog ids | `persistent` |
| `default_quick_chats` | nine texts | §6.3, §6.4 |
| `wallet` | ordered `(catalog id, amount)` | §7.1's wallet (shared with bounties-store, §15 item 4) |

From core's `starter_manifest` table (core §5.2): `bootstrap_version`, `stacked` (ordered catalog ids) and
`quick_emojis` (optional, nine texts). Every catalog id that §6.4 looks up must be in `instanced`; when one is not,
building fails with a data error `Missing inventory instance ID for starter banner {id}` (likewise `glider`, `flask`)
or `Missing inventory instance ID for starter loadout item {id}` (an equipment entry), answered 500.

### 6.3 Default chat wheels (on the way out)

Only when the version has `quick_emojis`: a `persistent` that is an object gets `quick_chats` = the default quick
chats when its `quick_chats` is "empty", and `emojis` = the version's quick emojis when its `emojis` is "empty"
(empty: absent, not an array, or an array whose every entry is `""` or not a string). Anything else is unchanged; a
non-object `persistent` is returned as it is. Without `quick_emojis` nothing is filled.

### 6.4 The starter slot and `persistent` (wire shapes, values from §6.2)

`ids` maps each instanced catalog id to the instance id chosen for it.

**`persistent`**, keys in this order: `manual_emotes` (six `""`), `intro_emote`, `banner` (the banner's instance id),
`bannerCustomization` (text), `flare`, `title` `""`, `head_accessory` `""`, `back_accessory` `""`, `pet` `""`,
`glider` (the glider's instance id), `update_version` 0, `quick_chats` (with quick emojis: the default quick chats;
without: nine `""`), `emojis` (the quick emojis, or nine `""`), `quick_curiosities_items` (eight
`{"item_index":i,"item_id":"","instance_id":""}`, i = 0..7), `quickwheel` `[]`.

**The slot**, keys in this order: one member per `equipment` entry, `{ "item_id": <catalog id>, "instance_id":
<ids[catalog id]>, "instance_data": <text or null> }`; then `player_role` null, `subweapon` null, `appearance` (the
default appearance's compact text), `flask` (the flask's instance id), `quick_items` `[]`, `slot_index` 0,
`update_version` 0, `custom_name` `""`, `persistent` (the object above, nested).

---

## 7. Starter records

### 7.1 Instance ids and seeding

**An instance id** is 26 characters of `A`-`Z0`-`9` (the shape of every captured id). It is drawn from replay ids:
take the next UUID's 16 bytes in RFC 4122 order; each byte below 252 (7 × 36) yields the alphabet's
`byte mod 36`-th character (`ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789`), others are skipped; when 26 are collected the
rest of that UUID is discarded; otherwise the next UUID continues. (Under a pinned replay seed the ids are fixed; the
goldens depend on this order of draws.) **Valid instance id**: exactly 26 bytes, each an ASCII upper-case letter or
digit.

**A manifest** draws one instance id per `instanced` catalog id, in list order: instanced items
`{"catalogId","instanceId","itemData":null,"updateVersion":0}` in that order; stacked items
`{"catalogId":<id>,"quantity":1}` in `stacked` order; `persistent` and one slot (§6.4).

**Seed a new character** (`SeedNewAccountAtomically`; in the creating transaction):

- `inventories.create(InventoryRecord { character_id, account, instanced_items: <items' jsjson text>, stacked_items:
  <text>, revision: 0, bootstrap_version: <the version's> })`;
- `loadouts.create(LoadoutRecord { character_id, account, loadouts: <[slot] text>, persistent: <persistent text>,
  active_index: none, unlocked_total_slots: none, revision: 0, bootstrap_version })` (store's defaults: index 0,
  entitlement 1);
- `wallets.create_if_missing(WalletRecord { account, balances: <the kit's wallet>, bootstrap_version })`: a wallet a
  balance request created first is kept unchanged.

### 7.2 Ensure the starter records (`EnsureStarterBootstrapRecords`)

**Outside a transaction** (what loadout routes call): account lookup (none → not owned); one `read`: the character
(`characters.find`; none → not owned), `inventories.find_by_character_id`, `loadouts.find`. Both present and
**nothing to repair** → the two records, no write opened. Otherwise one `account_write` running the in-transaction
form.

**Nothing to repair**: both rows exist, the instanced reconcile would append nothing and the stacked reconcile would
append nothing (below).

**In a transaction** (`EnsureStarterBootstrapRecordsInTransaction`; what other units call, §8.2): the ownership gate
first (`characters.find`; none → `NotOwned { character_id, user }`, because recovery can create records), then by
which rows exist. A stored item list that does not parse as an array reads as `[]` (§14 item 8).

| Inventory | Loadout | Done |
| --- | --- | --- |
| yes | yes | **Reconcile**: append each `default_weapon_parts` id not present as a non-empty string `catalogId`, with a fresh instance id unique against the list's `instanceId`s (`{"catalogId","instanceId","itemData":null,"updateVersion":0}`); append each `stacked` id not present, `{"catalogId":id,"quantity":1}`. When anything was appended: `inventories.update_both_if_revision_matches(<both texts>, <revision or 0>)`; none answered → data error `Default weapon-part reconciliation conflict for inventory {id}`. Info logs name the added ids: `[inventory] reconciled missing default weapon parts [{ids}] for characterId {id} (userId {user})`, `[inventory] reconciled missing starter stacked items [...]` likewise |
| no | no | A full manifest: inventory and loadout created as in §7.1 (**no wallet**) |
| no | yes | **Repair** (below) with no items and the loadout's first array entry as source; the inventory created with the repaired items and the full stacked list |
| yes | no | **Repair** of the stored items with no source; when it changed anything, `update_both_if_revision_matches(<repaired>, <stored stacked text unchanged>, revision)` (none → data error `Starter recovery conflict for inventory {id}`); then `persistent` and a slot from the repaired ids, and the loadout created as in §7.1 |

Answers the inventory and loadout records as they now are.

**Repair** (keeps every valid item; repairs only what a missing record needs, each with a fresh unique id):

1. Keep the entries that are objects (dropping any other is a change).
2. Known ids: walking the kept items, an item with a non-empty string `instanceId` not seen before and a string
   `catalogId` records `catalogId → instanceId` (the first per catalog id wins). Seen ids are the used set.
3. Candidates from the source slot: for each `equipment` entry, the source's member of that key whose `item_id` is
   that catalog id and whose `instance_id` is a valid instance id; the source's `flask` when it is a valid instance id
   (for the flask's catalog id). A placeholder that is not a valid instance id is never reused.
4. For each `instanced` catalog id in order without a known id: its candidate when there is one not yet used, else a
   fresh instance id unique against the used set; the first item with that `catalogId` gets it as `instanceId`
   (`itemData` set to `null` when absent, `updateVersion` to 0 when not a number), else a new item is appended. Each
   is a change.

---

## 8. The public seam

### 8.1 Shape

One crate, `rustic-isles-characters` (lib), MIT, in dainty-lass `crates/rustic-isles-characters`. It depends on
dainty-lass MIT crates `rustic-isles` (core: `Shared`, `Call`, `Handlers`, `Service`, `Method`, `read_body`,
`not_implemented`, `VersionTables`, `NoPlayerPolicy`, `replay`), `rustic-isles-store` (`IslesStore`, `Repositories`,
`Tx`, `AnyConnection`, the records, `repo::characters::slots`, `RepoError`, `AccountId`, `UserId`),
`rustic-isles-auth` (`AuthData`, `game_auth`), `rifts-seam` (`HostWriteKind`, `HostWrite`, `HostWriteOptions`,
`AccountId`, `Json`, `RuntimeAuth`), `module-api` (`Actor`, `ActorContext`), `service-kit` (`json`, `jsjson`, `js`,
`body::error_page`), and workspace `axum`, `http`, `serde_json` (its own `preserve_order`, core §17), `tracing`,
`thiserror`. No new workspace dependency.

**Seam changes for the integration pass** (not this row's): `crates/rustic-isles-characters` as a member and
`rustic-isles-characters` in `[workspace.dependencies]`; auth's `RuntimeCaller` handle (§15 item 2).

### 8.2 Interface (names are the contract the composer and the later units are written against)

| Item | Contract |
| --- | --- |
| `pub struct CharacterData`, `pub struct StarterKit` | §6.1, §6.2 (owned values; `Clone`, `Debug`) |
| `pub struct Characters` | `new(data: CharacterData, kit: StarterKit)`; the area's state, put in a typed slot by the composer (`Arc<Characters>`) |
| `pub fn routes() -> &'static [(Method, &'static str)]` | §3's eight rows in order, paths exactly as core's table |
| `pub fn register(handlers: &mut Handlers)` | Registers the eight backend handlers (core §5.7); each reads `Characters` and `IslesStore` from the shared slots |
| `Characters::characters_for_user(&self, store: &IslesStore, tables: &VersionTables, actor: &ActorContext, user_id: &str) -> Result<Vec<Value>, CharactersError>` | §5.2 (wire characters, creating the first) |
| `Characters::character_for_user(&self, store, tables, user_id, character_id) -> Result<Option<Value>, CharactersError>` | §5.2 "one character" |
| `Characters::ensure_starter(&self, store, tables, actor, user_id, character_id) -> Result<(InventoryRecord, LoadoutRecord), CharactersError>` | §7.2 outside a transaction |
| `Characters::ensure_starter_in(&self, tx: &mut Tx, repos: &Repositories, tables, account: &AccountId, user_id, character_id) -> Result<(InventoryRecord, LoadoutRecord), EnsureError>` | §7.2 in the caller's transaction (inventory's and progression's starter bootstrap, auth's session supplies) |
| `Characters::needs_repair(&self, Option<&InventoryRecord>, Option<&LoadoutRecord>, tables) -> bool` | §7.2 "nothing to repair", negated |
| `Characters::loadout_state(&self, store, tables, actor, user_id, character_id) -> Result<LoadoutState, CharactersError>`, `pub fn loadout_payload(&LoadoutState) -> Value`, `pub fn empty_loadout_payload() -> Value` | §5.9, §4.1, §4.2; `LoadoutState { loadouts: Vec<Value>, persistent: Value, active_index: u32 }` |
| `Characters::instanced_catalog_ids(&self) -> &[String]` | the kit's `instanced` (`rustic-isles-db`'s transforms) |
| `Characters::starter_wallet(&self) -> &[(String, f64)]` | the kit's wallet (§15 item 4) |
| `pub fn is_fresh_onboarding(conn: &mut AnyConnection, repos: &Repositories, account: &AccountId) -> RepoResult<bool>` | §5.8 |
| `pub mod data` | `normalize`, `guard_progression`, `process_triggers`, `read_form` (§5.7), `dojo_unlocked`, `is_fresh`, `login_time`, `display_date`, `validate_loadout_write -> Option<String>` (§5.12's reasons); each takes the `CharacterData` it needs; pure, for tests and fuzzing |
| `pub mod starter` | `generate_instance_id()`, `is_instance_id(&str)`, `build_manifest(&StarterKit, &VersionTables) -> RepoResult<Manifest>` (`instanced_items`, `stacked_items`, `loadout_slot`, `persistent`), `with_default_chat_wheels(Value, &StarterKit, &VersionTables) -> Value` |
| `pub enum CharactersError` | `NotOwned` (`the character does not belong to the user`), `Repo(RepoError)` (transparent), `Data(String)` (its text), `Failed(String)` (its text) |
| `pub enum EnsureError` | `NotOwned { character_id, user }` (`Character {character_id} does not belong to user {user}`), `Repo(RepoError)` (transparent) |

### 8.3 How it plugs into core and what calls it

- **Handlers**: the composer calls `register` on its backend `Handlers` and puts `Arc<Characters>` and the
  `IslesStore` in the shared slots. No start-up, no background task, no setting of its own.
- **Reads from shared state only**: `version.hooks.no_player_policy()`, `version.tables` (`target_changelist`,
  `starter_manifest`), `settings` (only through the no-player policy), `auth()`, `rifts.guard()`, the two slots.
- **Called by** (each through `Characters` in the slot, never through private state; core §5.7): bounties-store's
  first-character lookup and progression's journey inventory (`characters_for_user`, first `id`); inventory's and
  progression's starter bootstrap and auth's session supplies (`ensure_starter_in`); inventory's ownership check
  (`character_for_user`, or the store's `characters.find` directly); matchmaking's FTUE choice
  (`is_fresh_onboarding`; no caller at `e0e238b`); `rustic-isles-db` (`instanced_catalog_ids`).
- **Auth data**: restricted's areas read a characters-owned `AuthData` copy today (inventory re-exports it); the MIT
  units read auth's `AuthData` directly, so this crate exports none.

---

## 9. Persistent state (all through store's repositories; this crate owns no table)

| Repository (store §) | Calls |
| --- | --- |
| `accounts` (6.1) | `find_by_user_id`, `find`, `create` |
| `launcher_accounts` (6.2) | `find_by_account` (username) |
| `characters` (7.1) | `find_many_by_account`, `find`, `create`, `update_data_conditional` |
| `loadouts` (7.2) | `find`, `create`, `replace_slot_if_revision_matches`, `replace_all_and_entitlement_if_revision_matches`, `update_persistent_if_revision_matches`, `update_active_index_if_revision_matches` |
| `inventories` (9.1) | `find_by_character_id`, `create`, `update_both_if_revision_matches` |
| `wallets` (11.1) | `create_if_missing` |
| `player_journey` (10.5) | `find` |
| slots (7.3) | `resolve_visible_total`, `resolve_requested_total`, `resolve_player_journey_total`, `resolve_active_index`, `wire_counts`, `MAX_TOTAL_LOADOUT_SLOTS` |

Writes: account creation through `write`; every other write through `account_write` (the home check; `NotHome` and
`Home` errors are failures, so a request for an account homed elsewhere answers the 500 page). Reads through `read`.
Each listed "one `account_write`" is one transaction; writes are serial, so the read-then-write inside one is atomic.

---

## 10. Settings

None of its own. `GAMESERVER_NO_PLAYER_USER_ID` and `DEV_USER_ID` reach it only through core's no-player policy
(core §5.3).

---

## 11. Logs

English, at the levels given in §3 and §5. Client-supplied text in a log line (a name, a userId or characterId param,
an index) is written escaped (Rust `Debug` of the string) so a newline cannot forge a line (§15 item 6). Never a header
value, a token or a game server key.

---

## 12. What stays in dainty-restricted

- The goldens and their rig (§1.3); `tests/characters.rs` (authored; §13 describes its cases).
- The 1.14.7 values of `CharacterData` and `StarterKit`, which restricted's composer passes to `Characters::new`.
- Restricted's adapters for its unported areas (§1.3), and its copy of `STARTER_WALLET` in bounties-store until the
  economy unit reads `starter_wallet()`.

Then `areas/characters.rs` and `areas/characters/*` (except `golden.rs`) are deleted.

---

## 13. Test cases

### 13.1 What may be copied

Nothing recorded: `tests/golden/characters/*` and `node-*.golden.json` stay in restricted and are replayed there
against this crate once composed. `tests/characters.rs` and the unit tests beside the restricted files are authored
test code, described below only. Every fixture is synthetic: made-up catalog ids (`WP_TEST_00`, `PART_TEST_A`), a
made-up default appearance, flag names (`Flag_A`, `Flag_B`), a made-up Dojo series and trigger node. Tests run on
store's `TempStore` (store §13.2, SQLite), Postgres when `REFORGED_TEST_POSTGRES_URL` is set, with the replay clock
pinned at 1792000000000 ms unless a row says otherwise.

### 13.2 Behaviour the authored tests fix

| Area | Cases |
| --- | --- |
| Routes | `routes()` equals core's eight `characters-loadouts` rows in order; `register` builds with no `UnknownRoute` |
| Entry | no auth header → 501 envelope naming `GET /character` with the request id; a valid token but no `IslesStore` slot → 501; a bad bearer → auth's empty 401; `PUT` with `content-type: application/json` and body `nope` → 400 page before auth |
| GET | new token userId `p1` whose launcher display name is `Ash` → 200, one character named `Ash`, `accountId` `p1`, `catalogDaoId` null, `updateVersion` 0, both dates `Oct 14, 2026`, key order of §4.3; a second GET makes none; no launcher account → named `p1`; a game server with no bearer and `GAMESERVER_NO_PLAYER_USER_ID=hub` → `hub`'s characters; policy `Off` → `[]`; a player token whose userId is `INVALID` → `[]`; journey `{"Dojo":{"node_status":2}}` → `data` has `SERIE_dojo` (not stored); `{"X":{"node_id":"Dojo","node_status":1}}` too; status 0 or `"2"` → absent |
| PUT | `{"name":"Bo"}` → 200, named `Bo`; the inventory has the kit's items in order and stacked items quantity 1, the loadout one slot, the wallet the kit's balances; an existing wallet of 5000 kept; `{}` → `undefined`; `{"name":5}` → `5`; no body → 500 page; a bearer-less game server under `Off` → empty 400; two PUTs → two characters listed in creation order |
| POST | stored version 0: `updateVersion` 1 → 200 version 1; again 1 → 409 with §3.4's exact bytes and content type; a game server sending 1 → 200 version 2; `updateVersion` `"abc"` or 1.5 → 200 unchanged, nothing written; unknown `characterId` → 409; another account's character → 409; data `5` → 500 page; data as an object (not a string) → 500 page; data absent → `{}` normalised; a game server with `accountId` `p2` writes `p2`'s; a game server with empty `characterId` under `ConfiguredAccount` writes the first character; under `Off` → 409 (`undefined` id) |
| Guard | existing `SavedCharacter`: incoming `New` → `SavedCharacter`; `EnteredRamsgate` kept; `FinishedSecondHunt` → `EnteredRamsgate`; `Bogus` → `SavedCharacter`; `not json` returned as it is |
| Triggers | trigger node `Status` 3 as a string blob → `SERIE_dojo` set to the series text; an existing `SERIE_dojo` `mine` kept; `Status` 2 → absent; trigger value `nope` → data error; a stored `SERIE_dojo` survives a client save without it |
| Normalise | `{}` → keys: the fill list, `LoginTime`, `LastLoginTime`, `LastChangelist`, then `LoginFlagData` (and `AppearanceData`, `CharacterFlagData`, `PlayerAccountProgressStep` where the fill list lacks them); step `EnteredRamsgate`; `LastChangelist` `"647472"`; `LoginTime` `2026.10.14-17.46.40`; flags = the forced list; idempotent at a later time; `not json`, `[1]` → data errors; `""` normalises; kept: `FinishedFirstHunt`, an existing `LoginTime`, a flag's existing value, appearance `Data` `[1]`, a valid `StringData`, an extra member; healed: a `StringData` entry with `CustomizationName`, an entry `0`; `CreationState` always `FaceComplete`; `AssetReferences` added as `[]` |
| Dates | `login_time` 0 and 1792000000; `display_date` 1792000000 and 951782400 (§2) |
| Fresh | no characters → fresh; `{}` and `garbage` → fresh; normalised `SavedCharacter` → not fresh; incomplete creator with `Final` → fresh |
| Loadouts | GET of a new character → §4.1 with one slot, `num_character_slots` 1, `active_index` 0; another account's character → empty 400; B132 with a game server and path `INVALID` → §4.2's exact bytes; B132 with `p1` → empty 400; B133 with a game server and path `INVALID` → §4.2 |
| Unlock | a player → 403 `forbidden` body exactly, no hook asked; no authority at all → the same 403; `numSlots` `0`, `7`, `1.5`, `abc` or target `INVALID` → §3.7's 400 exactly; a game server with journey nodes `LoadoutSlot_01` and `LoadoutSlot_02` at `node_status` 2 → three slots, slots 1 and 2 copies of slot 0 with their `slot_index`, `update_version` 0, `custom_name` `""`; later with no unlocked nodes → still three (monotonic); `numSlots` `0x2` and ` 3` (`%203`) accepted; a host runtime whose guard admits → 200 and one tag of kind `LoadoutUnlock` whose summary contains `authoritativeTotalSlots` and not the character id; a guard refusal `(403, <body>)` → exactly that; a target with no account → 403 `P2P_HOST_WRITE_DENIED` |
| Active | index 1 with one visible slot → 400 `loadout_slot_not_unlocked`; `6`, `-1`, `abc`, `1.5` → 400 `invalid_active_loadout`; after unlocking three, `0x2` → 200 `active_index` 2; `%20` (Number 0) → 200 index 0; a player whose userId is `INVALID` → `invalid_active_loadout` |
| Set | `0` with an object `data` → 200, slot 0 replaced; `persistent` → replaced, and an emptied wheel reads back filled when the version has quick emojis, untouched without; `6`, `01`, `-0`, `x` → empty 400; `data` absent, `""`, `[1]`, `nope`, `5` (a number) → empty 400; body `[]` → empty 400; another account's character → empty 400; index 2 with one visible → empty 400 after 10 logged attempts |
| Starter | a synthetic manifest: one item per `instanced` id, every instance id valid, the equipment, flask, banner and glider ids match the items', the instance data only on its catalog id; two manifests share no id; a kit whose banner is not in `instanced` → `Missing inventory instance ID for starter banner ...` |
| Repair | inventory row soft-deleted → GET loadout recreates it, reusing the loadout's valid weapon and flask ids; a placeholder `instance_id` not reused; loadout row soft-deleted → recreated from the inventory's ids; one default part removed → appended once; a stacked id removed → appended, another's quantity 9 kept; an existing item with `instanceId` `""` and `updateVersion` `"x"` → given a valid id, `itemData` null, `updateVersion` 0; a non-object entry dropped; a read with nothing to repair opens no write (no change-log row) |
| Replay | two creates under the same pinned seed give the same character id and instance ids; the character id is the first draw |
| Actor | a player's writes are logged as `player:account:...`; a game server's as `system:isles`; every write carries the request id |

---

## 14. Behaviour that looks wrong but is kept (parity)

1. **No auth header answers 501** (§3.1 step 2), not auth's 401 (§15 item 1).
2. **Names and ids default to the literal `undefined`** (a nameless PUT, a POST without `characterId`).
3. **A non-numeric or fractional `updateVersion` is a 200 with no write**; the answer is the unchanged character.
4. **An unknown or foreign character on POST is a 409**, the "conflict, re-read" page, not a 404.
5. **A game server's stale save is forced** to the next version (the server is authoritative).
6. **`lastModifiedDate` never changes**; both dates are display texts, not ISO times.
7. **The first character of a new player is created under the `System` actor** (the account did not exist when the
   actor was chosen), and any userId a trusted caller names gets an account.
8. **A corrupt stored item list reads as `[]`** and the reconcile then writes the starter parts over it.
9. **A slot index beyond the visible total is retried 10 times** before the refusal (only logs show it).
10. **B136 does not ensure the starter records**, so a write before any read of a character with no loadout row is a
    400, while a read would have created it.
11. **The full-manifest repair seeds no wallet** (only a create does).
12. **Unlimited characters per account**, each with a full manifest.
13. **A legacy `StringData` entry `0` counts as legacy** and resets the appearance strings.
14. **`Number()` on index params** accepts `0x2`, `1e0`, ` 1` and an encoded space (0).
15. **The 1 MiB loadout data cap is unreachable** under the 100 KiB body limit.
16. **A request for an account homed on another node answers the 500 page** (no DN5 forwarding here).

---

## 15. Open questions

1. **No auth header → 501.** Restricted answers 501 when none of the three auth headers is present (a test rig
   accommodation: "the auth middleware did not reach"), so a session-ticket-only request (`x-reforged-session-ticket`)
   and a host runtime sending only `x-rustic-host-capability` get 501, never game auth. Node's
   `HasParadoxBackendAuth` would run and answer 401, or authenticate the ticket or runtime. Proposed: always run
   game auth (no 501 except for a missing store or slot). A behaviour change; parity until the coordinator says.
2. **Auth's `RuntimeCaller` carries no `rifts_seam::RuntimeAuth`.** MIT auth's `AuthData.runtime` holds a roster only,
   but the unlock gate must hand rifts' guard the `RuntimeAuth` (restricted's `AuthData` carries it). Proposed: auth
   adds an opaque handle to `RuntimeCaller` (`with_handle(Arc<dyn Any + Send + Sync>)` / `handle()`), restricted's
   realms adapter stores the `RuntimeAuth` in it, and this crate downcasts; without one, a host runtime caller is
   refused by the gate as if the guard had refused (403 `forbidden`, the dedicated-only guard's answer). A request to
   the auth implementer.
3. **Game-derived inputs.** §6's values (catalog ids, the appearance, flag names, the Dojo GUID blob, the wallet,
   the quick chats) are inputs; restricted's composer supplies 1.14.7's. Whether unit 12 (`rustic-isles-1-14-7`, MIT)
   may carry them as wire facts, as core's tables are, is the maintainer's call (AGENTS.md "Nothing from the game is
   committed").
4. **The starter wallet is duplicated** in bounties-store (its first-access grant). Proposed: one input, the kit's,
   read by the economy unit through `starter_wallet()`.
5. **The host-write summary writes `requested` as a float** (`1.0`); Node's `JSON.stringify` would write `1`. It is
   rifts' guard record, not client-visible. Parity unless the coordinator prefers `1`.
6. **Log escaping** (§11) departs from restricted, which writes client text raw. Logs are not a client contract;
   proposed as specified.
7. **`is_fresh_onboarding` has no caller** at `e0e238b`. Kept public for matchmaking (unit 8).
8. **No character deletion route** exists in either Node or restricted; erasure is the Reforged unit's.
9. **Session-ticket callers** (DN5) are keyed by the global id text as a userId, so a networked player gets a second
   local account and character set. Only reachable once item 1 lets tickets through. Proposed: a decision row with
   the network work, not this port.

---

## 16. Security behaviour (for `docs/security/rustic-isles-characters.md` and RX3)

The crate parses untrusted input (character data blobs, loadout slot and `persistent` data, path params, body
fields), so it gets a checklist page and a fuzz target.

- **Trust**: every route needs game auth (§3.1); a player acts only for its own token's userId; only a game server
  names another account (`accountId`, the loadout `userId` param); a host runtime only through rifts' guard
  (`LoadoutUnlock`), and only for an account the node knows.
- **Ownership**: every character, loadout and inventory read or write is keyed on (character id, account); another
  account's character is "not owned" (400 or 409), never read or written.
- **Entitlement**: slot unlocks come from the player's own journey rows, never from the request's count.
- **Bounds**: bodies 100 KiB (core); loadout `data` must parse to an object; indexes 0..5; slot totals 1..6;
  instance ids drawn uniformly (rejection above 252) from the replay id source.
- **Failure mode**: malformed stored or sent data is a 500 page or a refusal, never a panic; the store's revision
  guards make concurrent writes safe.
- **No secrets** pass through the crate; `Debug` of its types holds none; logs never carry headers or tokens (§11).
- **Known weak spots to list**: unlimited characters per account (storage growth bounded only by the request budget);
  a game server key can write any account's character and loadouts; names are unvalidated and echoed to every
  client that reads them; normalisation parses nested JSON strings recursively within the body limit (serde_json's
  depth limit gives a data error, a 500).

**Fuzz target (`fuzz/fuzz_targets/rustic_isles_characters_data.rs`)**: arbitrary bytes (lossy UTF-8) as character
data, existing data, a trigger blob and loadout write data, with a synthetic `CharacterData`: `normalize` never panics
and, when it succeeds, its output parses to an object and normalising it again gives the same text; `guard_progression`
never panics and never moves the step back or more than one forward when both parse; `read_form` and
`process_triggers` never panic; `validate_loadout_write` never panics and accepts only an object text with an allowed
index; `dojo_unlocked` and `is_fresh` never panic; repair of an arbitrary item array never panics and yields a valid,
unique instance id for every `instanced` catalog id.

---

## Coordinator decisions (2026-11-05, before implementation)

These settle §15 and override anything above that disagrees.

- **Items 1, 5, 9:** parity (no auth header answers 501 as restricted does).
- **Item 2:** `rustic-isles-auth`'s `RuntimeCaller` gains an opaque handle (`Option<Arc<dyn Any + Send + Sync>>`
  with an accessor) that the realms seam fills when it authenticates a runtime; the characters implementer makes
  that small auth change in its own commit and passes the handle back to rifts.
- **Item 3:** the §6 values stay inputs; none is committed to dainty-lass unless the maintainer rules them wire facts.
- **Item 4:** one starter wallet, read through `starter_wallet()`; the economy unit uses it.
- **Item 6:** log text escaped, as proposed (logs are not a client contract).
- **Items 7-8:** as specified.
- **Amended 2026-11-05 (maintainer's ruling):** the starter manifest's stacked ids and quick emojis are wire facts in
  core's table; `StarterKit` carries only the other run-time values.
