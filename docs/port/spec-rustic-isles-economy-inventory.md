# Clean-Room Behaviour Specification: `crates/rustic-isles-economy`, part 1: inventory and reforge (RD27)

> **Scope and purpose (RD27):** this is one of the three clean-room behaviour specifications for the MIT crate
> `rustic-isles-economy` (core spec §16, unit 7). This part covers the inventory and weapon-reforge area: the eight
> routes of the `inventory-reforge` group, the inventory transaction and its idempotency ledger, the item rules
> (grants, stacking, removals, saves, minted instance ids), the storage policy, the currency half of a transaction
> (`ApplyCurrencyDeltas`, with the prestige gate and the Sparkforge conversion), the weapon reforge's reset and banked
> restore, the weapons-at-base-level serving rule, and the test reward multiplier that `POST /inventory` applies. The
> other two parts are [`spec-rustic-isles-economy-progression.md`](spec-rustic-isles-economy-progression.md) and
> [`spec-rustic-isles-economy-bounties.md`](spec-rustic-isles-economy-bounties.md); §12 is where the three meet.
>
> The source is dainty-restricted's `modules/rustic-isles/crates/isles-core/src/areas/inventory.rs` and
> `areas/inventory/*`, plus the reward half of `areas/matchmaking/multipliers.rs`. It is AGPL-3.0-only with Mystic
> Paradox's section 7 terms. An implementer who never sees it writes the MIT code from this document alone.
>
> Everything below is stated from observable contracts: routes and their bytes, rules, store use by the store spec's
> names, settings, game-data formats and tests. No implementation code is reproduced and no private helper is named.
> Node names (`RunInventoryTransaction`, `inventory.ts`) appear only as provenance. Wire names the 1.14.7 client and
> its game servers depend on (`phx_account_id`, `INVALID`, `CURRENCY_*`, `x-rustic-isles-*`) are interface facts and
> are kept.
>
> Written against dainty-restricted `e0e238b4434316bb210677d88bc59e3448688794` (worktree
> `splashloader-integration-90130d`; the files of this part last changed in `7cd9db0`) and dainty-lass
> `731f928af2c098c1c4d61e1448c0b80ce69609ef` (branch `feat/isles-auth-clean-room`).

---

## 1. Provenance and licensing classification

### 1.1 Evidence used

| Source | What it says |
| --- | --- |
| File headers (`//!`) of `areas/inventory/*` | Each file ports a Paradox TypeScript file: `routes/inventory.ts`, the two reforge routes of `routes/progression.ts`, `controllers/inventory.ts`, `controllers/reforge.ts`, `validation.ts`, `inventoryStoragePolicy.ts`, `weaponBaseLevel.ts` (Reforged patch 0036), `wallet.ts`'s `ApplyCurrencyDeltas` and `IsPrestigeGatedReward`, `testMultipliers.ts` |
| `modules/rustic-isles/NOTICE.md` | The module is a modified Mystic Paradox; every file is AGPL-3.0-only with Paradox's section 7 terms |
| `crates/isles-1-14-7/tests/inventory_reforge.rs`, `inventory_reforge_host.rs`, `crates/isles-core/tests/inventory_store.rs`, the unit tests in each file | Authored behaviour (described in §13, never copied) |
| `tests/golden/inventory/{scenarios,expected}.json` | Recorded Node answers; read to confirm statuses only. They stay in restricted (§13.1) |
| Callers in `areas/{bounties_store,progression,reforged_api,auth,matchmaking}` and `isles-db` | The calls of §12 |
| dainty-lass `spec-rustic-isles-core.md`, `-store.md`, `-auth.md`, `migration-guide.md` §3 | The skeleton, repositories, game auth and names this part is written against |

### 1.2 Classification

Every file of this part is Category B (ports Paradox TypeScript) except `testkit.rs` and `clock.rs` (test support and a
date formatter, Category A). All of it is rewritten from this spec.

| Restricted file (`areas/`) | Source | Rewritten as |
| --- | --- | --- |
| `inventory.rs` | `routes/inventory.ts` registration | §3 (registration), §11 |
| `inventory/routes.rs` | `routes/inventory.ts`, `routes/progression.ts` (reforge rows) | §4, §5 |
| `inventory/transaction.rs` | `controllers/inventory.ts` | §6 |
| `inventory/items.rs` | `controllers/inventory.ts` item rules | §7 |
| `inventory/storage_policy.rs` | `inventoryStoragePolicy.ts` | §8 |
| `inventory/ports.rs` | `HasParadoxBackendAuth`, `wallet.ts` (`ApplyCurrencyDeltas`, `MergeWalletIntoStacked`), starter manifest, characters | §6.6, §9, §11.3 |
| `inventory/reforge.rs` | `controllers/reforge.ts` (reset, purchase plan, banked restore) | §5 |
| `inventory/progression_config.rs` | `wallet.ts` (`IsPrestigeGatedReward`, prestige sources) | §9.3 |
| `inventory/validation.rs` | `validation.ts` | §4.6 |
| `inventory/weapon_level.rs` | `weaponBaseLevel.ts` | §7.7 |
| `inventory/respond.rs`, `body.rs`, `clock.rs`, `sha256.rs` | Express answers, `Date#toISOString`, `crypto.createHash` | §2 |
| `inventory/testkit.rs` | RU5 test support | §13 (described only) |
| `matchmaking/multipliers.rs` (reward half) | `testMultipliers.ts` | §10 |

### 1.3 What stays in dainty-restricted

The goldens (`tests/golden/inventory/*`, including its `game-data/`), the recording scripts, `testkit.rs` and the
restricted copies of the files above until the composer switches to this crate.

---

## 2. Conventions

- **JSON** is written as `JSON.stringify` writes it (core §2.1): `service_kit::json::Json`, `content-type:
  application/json; charset=utf-8`, keys in insertion order. Every key order below is the wire order.
- **Empty** is a status with no body and no content type. **Empty JSON** is status 200 with `content-type:
  application/json; charset=utf-8` and a zero-length body (Express's `res.json(undefined)`).
- **500 page** is `service_kit::body::error_page(500)` (Express's production error page): Node threw.
- **Refusal** is `{"code":null,"message":<message>,"payload":{}}` with the status given.
- **501 envelope** is `{"error":{"code":"NOT_IMPLEMENTED","message":<message>,"requestId":<id>}}`, status 501.
- **JavaScript value rules** use `service_kit::js`: `number` (`Number(x)`), `string` (`String(x)`), `safe_integer`
  (`Number.isSafeInteger`), `is_number` (`typeof x === "number"`), `is_nullish`, `strict_eq` (`===` on JSON values),
  `date_parse`, `trim`; and `service_kit::jsjson`: `parse` (`JSON.parse`), `stringify` (`JSON.stringify`), `to_js`.
- **A JS number as JSON** ("number value" below): a finite whole number below 2^53 in magnitude is written as an
  integer (`5`, not `5.0`); anything else as the float (NaN and infinities become `null`, as `JSON.stringify` does).
- **Times**: `now_ms` is core's replay-aware clock (core §9.1); ISO texts are `Date#toISOString`'s
  (`1970-01-01T00:00:00.000Z`; years 0000 to 9999; a negative time is before the epoch, `-1` is
  `1969-12-31T23:59:59.999Z`). The ISO formatter here must agree with core's `season::to_iso_string`.
- **SHA-256 hex** is lower-case hex of the UTF-8 bytes.
- **`INVALID`** is the 1.14.7 engine's own "no logged-in player" sentinel: never created, read or written for.
- **The no-player policy** and its two accounts are core §5.3 (`path_account`, `bearerless_account`).
- **The store** is `rustic_isles_store::IslesStore` (store spec §3), taken from the module's slots
  (`Shared::slot::<IslesStore>()`, core §5.1). Repository names are store spec §5.1's fields.
- **Actors**: a write for a player's account names `module_api::Actor::Player { account }` with the request id as
  `ActorContext::request`; the first-character lookup of a game server request names `Actor::System` with the request
  id (§11.3).

---

## 3. Routes

The eight rows of core Appendix A in the `inventory-reforge` group, registered on `Service::Backend` with these exact
path texts (core §5.7). The regex row is registered before the string row on purpose: Express 5 does not match
`:characterId` against an empty segment, so the 1.14.7 client's `POST /inventory//<build>` reaches only the regex.

| Row | Method | Path text | Handler | § |
| --- | --- | --- | --- | --- |
| B68 | POST | `/^\/inventory\/([^/]*)\/(.+)$/` | migration stub (unnamed params `0`, `1`) | 4.1 |
| B69 | POST | `/inventory/:characterId/:changeList` | migration stub | 4.1 |
| B70 | GET | `/inventory/:userId/:characterId` | inventory read | 4.2 |
| B71 | GET | `/inventory/:userId/` | inventory read without a character | 4.3 |
| B72 | POST | `/inventory` | transaction | 4.4 |
| B73 | POST | `/inventory/instanceditem` | single item save | 4.5 |
| B130 | DELETE | `/progression/:userId/:progressionId` | reforge reset | 5.2 |
| B131 | POST | `/progression/purchase/:userId/:progressionId/:amount` | reforge banked restore | 5.3 |

`GET` rows also answer `HEAD` (core §4.1). Params arrive percent-decoded and bound before authentication, so a game
server's `INVALID` path rewrite (auth §4.1.1) is invisible to these handlers: a param still reads `INVALID`.

### 3.1 Every route: body, then authentication (`HasParadoxBackendAuth`)

In this order, before any handler rule:

1. **Body** (core §3.9): read with core's body helper (100 KiB, Express rules). A parse failure answers its Express
   error page (400, 413, 415) before authentication. An absent body is "undefined".
2. **Game auth, only on wire credentials.** If the request's extensions carry no auth data yet **and** the request
   has an `authorization` header or an `x-rustic-isles-gameserver-apikey` or `x-rustic-isles-gameserver-key` header
   (present, any value), the auth crate's game auth runs (`Auth::game_auth`, auth §4.1): its refusal (empty 401,
   empty 403 or 500 page) is the answer; on success its `AuthData` is put in the extensions. A request with only a
   session ticket or only a host-capability header does not trigger it here (§14 item 1).
3. **The authenticator port** (§11.3) turns the extensions' auth data into the caller: user id (optional), is a game
   server, and the player-host runtime (optional; never together with "is a game server"). No auth data: **empty
   401**.

"The caller's user" below means: for a game server, what the route says (a path param or the body's `accountId`);
otherwise the caller's own user id from the token.

### 3.2 No database

Where a route below "needs the store" and the module has none, it answers the 501 envelope with message
`isles: no database is configured (REFORGED_DB_FILE or REFORGED_DB_ENGINE)` and the request's id.

### 3.3 Repository errors

A repository error that reaches a route answers: `RepoError::Unimplemented` → the 501 envelope with message
`isles: <the error's display>` (store spec §2.4) and the request's id; any other → logged error
`inventory: <error>` and the 500 page. Exception (parity, §14 item 7): the account lookup of the two reforge routes
uses an empty `requestId` in that envelope.

---

## 4. Inventory routes

### 4.1 Migration stub (B68, B69)

After §3.1, answers 200 `{"code":"NONE","message":""}`. Logged: B69 `Inventory migration (stubbed)`; B68
`Inventory migration (stubbed) characterId='<param 0>' changeList='<param 1>'`. Needs no store, writes nothing.

### 4.2 `GET /inventory/:userId/:characterId` (B70)

1. User: a game server's is the `userId` param; anyone else's is their token's user id (the param is ignored).
2. User is `INVALID`: 200 `{"characterId":<characterId param>,"instancedItems":[],"stackedItems":[]}`. Needs no
   store.
3. Needs the store (§3.2). Logged `UserId <user or undefined> requested inventory for CharacterId <id>`.
4. The inventory read (§6.3) for (user, characterId): the inventory → 200 with it, served at base level (§7.7);
   none (no user, not that user's character, unknown character) → **empty 400**; an error → §6.5's mapping.

### 4.3 `GET /inventory/:userId/` (B71)

1. User as §4.2 step 1.
2. User is `INVALID`: 200 `{"characterId":"INVALID","instancedItems":[],"stackedItems":[]}`.
3. Only when the version's no-player policy is `ConfiguredAccount`, the caller is a game server and the user is
   non-empty: needs the store; the user's first character (§11.3; it may create the account and its first character);
   a repository error → §3.3. A non-empty id: the inventory read (§6.3); an inventory → logged
   `GET /inventory (no characterId): gameserver request resolved to '<id>' for <user>` and 200 with it, served at base
   level; none falls through; an error → §6.5.
4. Otherwise (a player, policy `Off`, no first character, no inventory): **empty 400**.

### 4.4 `POST /inventory` (B72)

In this order:

1. **Body must be an object.** Else logged warning and 400
   `{"code":null,"message":"Invalid inventory request: body must be an object","payload":{}}`. An absent body and an
   array are refused; a form-encoded body parses to an object and goes on.
2. **User.** A game server: the body's `accountId` when it is a string; the caller's user id when it is absent or
   `null`; none when it is any other value. Anyone else: the caller's user id.
3. **Bearer-less game server's account.** Policy `ConfiguredAccount`, a game server, and the user none, empty or
   `INVALID`, and the policy's bearer-less account is set: the user becomes it (logged
   `POST /inventory: gameserver request with no accountId resolved to '<user>'`).
4. **Empty character.** Policy `ConfiguredAccount`, a game server, the body's `characterId` not a non-empty string,
   and the user non-empty and not `INVALID`: needs the store; the user's first character (§11.3); a repository error
   → §3.3; a non-empty id is written into the body's `characterId` (so validation and the hash see it) and logged
   `POST /inventory: gameserver request with no characterId resolved to '<id>' for <user>`.
5. **Sentinel echo.** User `INVALID`: 200 (no validation, no store):
   `{"createdInstancedItems":[],"updatedInstancedItems":<saveInstancedItems>,"updatedStackedItems":<addStackedItems>,"removedInstancedItems":<removeInstancedItems>}`,
   each field the body's value as sent, or `[]` when absent or `null` (a non-array is echoed as it is).
6. **Validation** (§4.6). A message → logged and 400
   `{"code":null,"message":"Invalid inventory request: <message>","payload":{}}`.
7. **Host runtime.** The caller has a player-host runtime: rifts' guard `authorize_inventory(runtime,
   {user: <user or "">, character_id, transaction_id, body: <the body as compact JSON text>})`.
   `Authorized(fence)` keeps the fence; `Refused(reply)` answers the reply (its status, its body text, JSON content
   type); a seam error → §4.7.
8. **Player authority.** Not a game server, no fence, and the body grants or spends (any of `addInstancedItems`,
   `addStackedItems`, `removeInstancedItems`, `removeStackedItems` is a non-empty array): logged error and 403
   `{"code":null,"message":"Authoritative inventory mutations require gameserver authority","payload":{}}`. A player
   may save only (`saveInstancedItems`).
9. Needs the store (§3.2).
10. **Another node's player** (a game server, no fence): the user resolves to an account global id (the text parses
    as a `GlobalId` of kind `Account`, else the live account of that `userId`) whose owner is not this node. Then the
    grant is forwarded home instead of applied (§4.4.1).
11. **Season challenge wait.** A dedicated server (a game server with no fence) or a fenced host, and some
    `addStackedItems` entry is a challenge reward grant (§9.4): the account of the user (no character check; a
    repository error → §3.3); when found, the wallet port's wait (§12 W7). A `false` only logs
    `transactionId <id>: challenge-reward currency with no claimed, uncredited season challenge after 1.5 s`.
12. **Test reward multiplier** (§10): only a dedicated server, only when `addStackedItems` is an array. The
    multiplied list (when anything changed) replaces `addStackedItems` for everything after, the request hash
    included; logged `[TestMultiplier] transactionId <id> source="<first 80 characters of String(source)>" rewards multiplied for userId <user>`.
13. **The transaction** (§6.1) with: the user (empty text when none), the body's `characterId` and `transactionId`,
    the five collections (absent is `null`), and options: challenge funding allowed for a dedicated server or a fenced
    host; economy source `INVENTORY_GAMESERVER` for a dedicated server, `OTHER` otherwise; for a fenced host, a grant
    hook (§4.4.2). With a fence, the fence is told `Applying` before the transaction starts.
14. **Answer.** Done → fence told `Accepted`; logged `Ran transactionId <t> for userId <u> and characterId <c>`; 200
    with the result served at base level (§7.7). Not owned → fence told `Failed`; logged error; **empty 400**. An error
    → §6.5 (the fence is told nothing; §14 item 9).

#### 4.4.1 Forwarding a foreign player's grant home

The loot list, in order: for each `addStackedItems` entry with a string `catalogId`, `<catalogId>:<q>` where `q` is
the quantity when it is a JSON integer, else 1, and at least 1; then for each `addInstancedItems` entry with a string
`catalogId`, `<catalogId>:1`. Removals, saves and item data are not forwarded (§14 item 8).

The hunt: the ring instance the matchmaking state knows for that player (§11.4 `RingInstances`), else a new
`GlobalId` of kind `Hunt` on this node. The module id: `isles-` and the version's game version with `.` replaced by
`-` (`isles-1-14-7`; an invalid id → 500 page). Then the home API's `ring_forward_hunt_rewards` with
`HuntRewards { hunt, module, account: <the foreign global id>, xp_grant: none, loot_write: {"loot":[...]},
objective_write: none }`. An error → logged `ring_forward_hunt_rewards (loot) for <account>: <error>` and the 500
page. Success → 200:
`{"createdInstancedItems":[],"updatedInstancedItems":<save or []>,"updatedStackedItems":<add stacked or []>,"removedInstancedItems":<remove instanced or []>,"forwarded_to_home":true}`
(the echo rule of step 5; `addStackedItems` is the one sent, unmultiplied).

#### 4.4.2 The host's grant hook

Runs inside the transaction (§6.1 step 5) for a new transaction id only: the fence's `on_grant` with the body as
`JSON.stringify` text. `Ok` continues with no ledger annotation. `Err(reply)` rolls the whole transaction back
(ledger row included) and answers the reply's status with its body parsed as JSON (`null` if it does not parse).

### 4.5 `POST /inventory/instanceditem` (B73)

1. **Body must be an object.** Else 400
   `{"code":null,"message":"Invalid item update: body must be an object","payload":{}}`.
2. **Host runtime.** The caller has a runtime: its target is the caller's user id when non-empty. No target: 403
   `{"code":"forbidden","message":"Player-host runtime cannot mutate inventory.","payload":null}`. Else rifts'
   guard `authorize_host_write(runtime, ItemSave, target, default options)`: a reply answers as it is; a seam error →
   §4.7. A refusal is logged `POST /inventory/instanceditem rejected: the host write was refused`.
3. **User**: as §4.4 step 2 (a game server's body `accountId`).
4. **Sentinel echo.** User `INVALID`: 200 with an object of the body's `characterId`, `instanceId`, `catalogId`,
   `itemData`, `updateVersion` in that order, each only when the body has the key (any value, `null` included). No
   validation, no store.
5. **Validation** (§4.6). A message → 400 `{"code":null,"message":"Invalid item update: <message>","payload":{}}`.
6. Needs the store.
7. **The single item save** (§6.4). An error → §6.5.
8. **Tag.** With a runtime, an item answered and a user: rifts' guard `record_host_write(runtime, {kind: ItemSave,
   target: user, summary})`, summary `{"instanceId":<first 64 characters of String(instanceId)>,"catalogId":<first
   128 of String(catalogId)>,"updateVersion":<as sent, only when present>,"itemDataBytes":<UTF-16 length of itemData
   when a string, else 0>}` as `JSON.stringify` text. A tagging error is logged
   `[HostWrite] tag failed kind=ItemSave target=<user>: <error>` and changes nothing.
9. **Answer.** The item → 200, served at base level. None → **empty JSON** (200).

### 4.6 Validation (`validation.ts`)

Lengths are UTF-16 units. "Non-empty string" is a string of 1 to 256 units. An integer is a JSON number that is
finite with no fraction.

**Transaction body**, checked in this order, the first failure is the message:

| Check | Message |
| --- | --- |
| Not an object | `request body must be an object` (unreachable from the route) |
| `characterId` not a non-empty string | `characterId must be a non-empty string` |
| `transactionId` not a non-empty string | `transactionId must be a non-empty string` |
| For `addInstancedItems`, `removeInstancedItems`, `saveInstancedItems` in that order (absent or `null` passes) | |
| not an array | `<field> must be an array` |
| more than 4096 entries | `<field> has too many items` |
| an entry not an object | `<field> contains a non-object entry` |
| `catalogId` not a non-empty string | `<field> entry has an invalid catalogId` |
| `instanceId` not a non-empty string | `<field> entry has an invalid instanceId` |
| `itemData` present, not `null` and not a string of at most 262 144 units | `<field> entry has invalid itemData` |
| `updateVersion` present, not `null` and not an integer from 0 to 1 000 000 000 | `<field> entry has an invalid updateVersion` |
| For `addStackedItems`, `removeStackedItems` in that order: array, size, object, `catalogId` as above | |
| `quantity` not an integer from 0 to 1 000 000 000 | `<field> entry has an invalid quantity` |

Entries are checked one at a time; within an entry, the checks in the table order.

**Item update body**: `characterId`, `instanceId`, `catalogId` in that order, each a non-empty string, else
`<field> must be a non-empty string`; `itemData` absent, `null` or a string of at most 262 144 units, else
`itemData must be a string`; `updateVersion` absent, `null` or an integer 0 to 1 000 000 000, else
`updateVersion must be a bounded non-negative integer`.

### 4.7 Seam errors from rifts' guard

`SeamError::Unimplemented` → rifts-seam's 501 reply for it with the request's id
(`{"error":{"code":"NOT_IMPLEMENTED","message":"<component>: <operation> is not implemented yet","requestId":<id>}}`);
`SeamError::Refused { code, message }` → **400** `{"error":{"code":<code>,"message":<message>,"requestId":<id>}}`
(always 400 here, not `status_for(code)`: §14 item 10).

---

## 5. Weapon reforge (`controllers/reforge.ts`)

### 5.1 What the game does

`ServerPrestigeExperience` on a 1.14.7 game server, in order: pays the cost (`POST /inventory`), `DELETE
/progression/<acct>/ExperienceTrack_Weapon_<W>` (the reset; the game stops unless it answers 200 to 206 with a JSON
object), reads the track back, `POST .../PrestigeTrack_Weapon_<W>/1` once (progression's XP route), about 0.5 s later
`POST /progression/purchase/<acct>/ExperienceTrack_Weapon_<W>/<banked>` (the progress above the cap), then the
Aetherheart and the rank confirm. None of it is guarded against a duplicate RPC, so the backend is: the reset stores a
**reforge record** on the weapon's ExperienceTrack row (store spec §10.1 `WeaponReforge`), and the +1 and the banked
restore are each accepted once against it, marked inside the grant's own transaction.

- **Reforgeable track**: `ExperienceTrack_Weapon_<W>` with `W` one of `Axe`, `ChainBlades`, `Hammer`, `Repeaters`,
  `Spear`, `Strikers`, `Sword` (progression's rule, §12 P6).
- **Reforge mode** (§12 P1): `RUSTIC_ISLES_REFORGE` trimmed, ASCII lower-cased: `off`, `log`, else `on`; unset follows
  the version's `reforge_rules` hook (`on` when true, `off` when false).
- **Cap**: the track's cap XP from `progression_config.json` (§12 P6), as an integer; none or not above 0 means the
  track has no cap.
- **Stale**: an open record whose `openedAt` (parsed as `Date.parse`) is at least 120 000 ms (`STALE_REFORGE_MS`)
  before now, or does not parse.

### 5.2 `DELETE /progression/:userId/:progressionId` (B130)

1. Logged `[Reforge] DELETE-REQ user=<u> track=<t> gs=<0|1>` (+ ` src=host` for a runtime).
2. **Authorize** (§5.4). A refusal is the answer.
3. `userId` is `INVALID` or the track is not reforgeable: logged warning and 409
   `{"code":null,"message":"Conflict","payload":{}}`.
4. Needs the store. The live account with that `userId` (`accounts.find_by_user_id`); none → 409 Conflict; an error
   → §3.3 with an empty request id.
5. **The reset** (§5.2.1) at `now_ms`. An error → §6.5.
6. **Tag.** A runtime and the outcome `Reset`: `record_host_write(runtime, {kind: Reforge, target: userId param,
   summary: {"route":"delete","track":<t>,"reforgeId":..,"priorProgress":..,"cap":..,"banked":..}})` (the last four
   from the stored record; a tagging error is logged only).
7. **Answer.** Status 200: `{"code":null,"message":"OK","payload":<single track payload>}`; any other status (always
   409): `{"code":null,"message":"Conflict","payload":{}}`.

**Single track payload**: `{"phx_account_id":<userId param>,"progression_id":<track>,"progress":..,"confirmed_fremium_rank":..,"confirmed_premium_rank":..,"confirmed_date":<the row's updatedAt>}`;
with no row, progress and ranks 0 and `confirmed_date` the ISO now.

#### 5.2.1 The reset

| Step | Outcome | Status, record |
| --- | --- | --- |
| Mode not `on` (logged warning naming the mode) | `Disabled` | 409, none |
| No cap for the track | `NoTrack` | 409, none |
| Then up to 4 attempts, each one `account_write` (player actor): | | |
| No live track row | `NoTrack` | 409, none |
| An open record, not stale (logged `[Reforge] REPLAY ...`) | `Replay` | 200, the row unchanged |
| An open record, stale: progression's stale finish (§12 P4), then re-read | `StaleFinished` | 409, the re-read row |
| Progress below the cap (logged) | `BelowCap` | 409, the row |
| Else a new record: `reforgeId` a random UUID v4 (system randomness, §14 item 4), open, `priorProgress` = progress, `cap`, `banked` = progress − cap, `bankedRestored` 0, `prestigeGranted` false, `openedAt` = ISO of the reset time; `progression_tracks.begin_reforge` at the row's progress and version | `Reset` (logged `[Reforge] RESET ... prior=.. cap=.. banked=..`) | 200, the re-read row (progress 0, ranks 0, `updatedAt` = `openedAt`) |
| `begin_reforge` answered none | next attempt | |
| Four attempts used (logged `... contended`) | `Contended` | 409, none |

A closed record does not count as open: a reset over it starts a new reforge. On `Reset` the observer is told
(§11.3 `reforge_reset`).

### 5.3 `POST /progression/purchase/:userId/:progressionId/:amount` (B131)

1. `amount` = `Number(<amount param>)`. Logged `[Reforge] PURCHASE-REQ user=.. track=.. amount=<raw> gs=..`.
2. **Authorize** (§5.4).
3. `userId` is `INVALID`, or the amount is not a finite whole number, or not above 0, or above 100 000
   (`MAX_PROGRESSION_GRANT_AMOUNT`): 400 `{"code":null,"message":"Bad Request","payload":{}}`.
4. Needs the store. The account of `userId` (an error → §3.3 with an empty request id); when found, its live track
   for the progression id (`progression_tracks.get`; an error → §3.3).
5. **The plan** (§5.3.1) for (track, amount, row, `now_ms`, mode). A refusal, no grant id, or no account: logged
   `[Reforge] DUP/REFUSED purchase ... : <reason>` and 409 Conflict.
6. **Plan `Normal` replays only from the ledger.** `progression_grants.find_by_grant_id(grant id)`: none → 409
   Conflict (reason `the banked XP was already restored (stale-reforge recovery)`); an error → §3.3.
7. **The banked restore** (§5.3.2). A reforge conflict or a grant conflict → 409 Conflict; any other failure → §6.5.
8. **Tag.** A runtime and not replayed: `record_host_write(runtime, {kind: Reforge, target: userId, summary:
   {"route":"purchase","track":..,"amount":<number value>,"grantId":..,"reforgeId":<from the row read in step 4,
   when it had a record>}})`.
9. **Answer** 200:
   `{"code":null,"message":"OK","payload":{"phx_account_id":<userId>,"progression_id":<track>,"progress":..,"confirmed_fremium_rank":..,"confirmed_premium_rank":..,"confirmed_date":<updatedAt>,"grant_id":<grant id>,"replayed":<bool>}}`
   from the grant's track record.

#### 5.3.1 The purchase plan (`PlanPurchaseRestore`)

| Condition, in order | Plan |
| --- | --- |
| Mode not `on`, or the track not reforgeable | Refuse `not a reforge banked restore` |
| The row has no record (or no row) | Refuse `no reforge for this weapon` |
| Grant id is `reforge-<reforgeId>-banked`. Progression's classification (§12 P2) says `Restore` | `Restore`, with the grant id |
| `bankedRestored >= banked` and amount = `banked` | `Normal`, with the grant id (a retry the ledger replays) |
| Else | Refuse: the classification's reason when it refused, else `amount <a> is not the banked <banked − bankedRestored>` |

The fourth row is checked even when the classification refused (a duplicate within 15 minutes): a retry of an applied
restore reaches the ledger and replays.

#### 5.3.2 The banked restore (the `beforeApply` path of `GrantProgressionXp`)

Preconditions (each failure is a data error → 500 page): the track is reforgeable and has a curve
(`Unsupported progression track '<t>'.`); 0 < amount ≤ 100 000 and the grant id is valid
(`Invalid authoritative progression grant.`). The request hash is progression's (§12 P6) of (`userId` param, track,
amount).

One `account_write` (player actor):

1. A grant with that id exists: if its hash, account, track or amount differs → grant conflict
   (`Progression grant <id> was already used with a different request.`); else the stored track record, replayed.
2. Mark the reforge use (§12 P3) for the plan (a `Normal` plan marks nothing). A conflict rolls back.
3. `progression_tracks.increment(track, amount)`, then `progression_grants.insert_applied` with the result and
   `created_at` ISO now. Logged `[ProgressionGrant] grant=.. user=.. track=.. delta=.. old=.. new=.. source=host-or-gameserver applied=1`.
4. The new track record, not replayed.

### 5.4 Authorizing a reforge route

A runtime: rifts' guard `authorize_host_write(runtime, Reforge, <userId param>, default)`: none → allowed; a reply →
answered as it is; a seam error → §4.7. No runtime and not a game server: 403
`{"code":null,"message":"Forbidden","payload":{}}`. A game server: allowed.

---

## 6. The inventory transaction and reads (`controllers/inventory.ts`)

### 6.1 Running a transaction

Inputs: user id, character id, transaction id, the five collections (`addInstancedItems`, `addStackedItems`,
`removeInstancedItems`, `removeStackedItems`, `saveInstancedItems`, each a JSON value, `null` when absent), and
options (§11.2 `TxOptions`): ownership already validated, a diagnostic label, a grant hook, challenge funding allowed,
operator test funding, the economy (source, P2P session id, store SKU id; default source `OTHER`).

1. **Owner.** The live account with that `userId`; unless ownership is already validated, also the live character
   of that account (`characters.find`). Either missing → logged error and **not owned** (Node's `false`). Errors are
   repository errors.
2. **Request hash** (§7.1) of the five collections as given.
3. **Storage policy** (§8) loaded; a failure is a policy error (500 page).
4. **Grant normalization** (§8.3) of the two add collections (non-arrays are empty). An impossible quantity is a
   policy error. Corrections are logged warning
   `[InventoryStorage] transactionId <t> normalized <n> entr(y|ies): <catalogId>:<from>-><to>x<quantity>, ...`.
5. **One `account_write`** for the account (home check, store spec §3.4; player actor). Inside, all or nothing:
   1. `inventory_transactions.try_begin(key, hash, ISO now)` with key (account, character, transaction id).
      `Existing(row)` ends the closure with nothing written (step 7).
   2. **Currency** (§9.1) for the normalized stacked adds and the raw stacked removes. Its errors roll back.
   3. **Grant hook** when given: may refuse (rolls back) or return an annotator.
   4. **The inventory**: the live inventory of the character; none → logged
      `Recovering bootstrap records for characterId <c>` and the starter bootstrap (§11.3) creates it (a not-owned
      answer there is a data error). The stored revision (none reads 0); both item texts parsed with `JSON.parse`
      (not an array: data error `inventory instancedItems are not an array`; a parse failure:
      `inventory instancedItems: <error>`; the same for `stackedItems`).
   5. **Item rules** (§7) with the normalized adds, the raw removes and saves, and the currency step's touched
      balances. An insufficient stacked item rolls back.
   6. `inventories.update_both_if_revision_matches` with both arrays as `JSON.stringify` text at the read revision.
      None (another write landed) → logged warning and a **conflict** (rolls back).
   7. **Result** `{"createdInstancedItems":[..],"updatedInstancedItems":[..],"updatedStackedItems":[..],"removedInstancedItems":<the request's removeInstancedItems, or [] when null>}`.
   8. **Ledger completion**: `inventory_transactions.complete(key, result, extra, ISO now)`, extra
      `{"economySource":<source>}` plus `"economySessionId"` when a P2P session id is set, then the annotator's fields
      when it returns an object (given the created instanced items and the accepted stacked adds, §7.3).
6. **Done.** The observer is told (§11.3 `committed`). When a label is set or the whole call took 1000 ms or more:
   logged `[InventoryTiming] label=<label or inventory> total=<ms>ms`. Answer the result.
7. **Existing ledger row.** A stored hash that differs from this one → logged error and **mismatch**; no stored hash
   (a legacy row) → logged warning and replayed on the old rules; then `completed` → the stored result (as `to_js`
   reads it; never re-applied, logged `[Inventory] Replayed ...`); `pending` → **conflict**.

A rolled-back attempt leaves no ledger row, so the same id may be used again (with any body).

The **committed transaction** given to the observer: account, user id, transaction id, source, the result, and three
lists computed from the normalized adds before the closure:

- `granted`: each normalized instanced add as `{catalogId, quantity: 1}` then each normalized stacked add as
  `{catalogId, quantity}`, keeping only entries whose `catalogId` is a string not starting `CURRENCY_` and whose
  quantity is a safe integer.
- `currency_credits`: normalized stacked adds whose `catalogId` starts `CURRENCY_` with a safe-integer quantity above
  0, each as `{catalogId, amount}`.
- `currency_debits`: the same rule over the raw `removeStackedItems`.

### 6.2 The ledger's request binding

The hash binds the id to the body: the same id with the same collections (in any key order, missing as empty)
replays; another body is refused. Because the hash is taken after the test multiplier (§4.4 step 12), a change of the
host's multiplier between a send and its retry turns the retry into a mismatch (§14 item 11).

### 6.3 Reading an inventory (`GetInventoryForUserIdAndCharacterId`)

No user → none. Owner (§6.1 step 1, with the character check) missing → none. Policy loaded (failure → policy error).
Then one `account_write` (player actor):

1. The starter bootstrap (§11.3): creates a missing inventory or loadout, or appends missing starter parts and stacks
   (so a read writes). Not owned there is a data error.
2. Both arrays parsed (as §6.1 step 5.4).
3. **Storage repair** (§8.4). When it changed anything: `update_both_if_revision_matches` at the read revision; none
   is data error `Inventory storage reconciliation for characterId <c> failed`; logged warning
   `[InventoryStorage] repaired legacy storage for userId <u> characterId <c>: <catalogId>:instanced->stacked x<n> (<result>), ...`
   and, when issues remain, `... still has <n> stacked non-stackable entries requiring manual review`; the arrays are
   re-read from the updated row.
4. **Wallet merge** (§12 W1, W4): the account's wallet (created with the starter balances when missing, so a read
   writes) merged over the stacked items.
5. `{"characterId":<c>,"instancedItems":[..],"stackedItems":[..]}`.

### 6.4 Saving one item (`UpdateInstancedItem`)

Inputs: character id, user id (optional), instance id, catalog id (JSON values), item data and update version
(optional JSON values; absent is "none").

No user, or the owner check (with the character) fails → logged error and none. Then up to 5 attempts, each one
`account_write`; before every attempt after the first, a sleep of 5 + (`now_ms` mod 20) ms:

1. No live inventory → logged error, none.
2. The instanced entry whose `instanceId` strictly equals the given one (first match); none → logged warning, none.
3. Its `catalogId` not strictly equal to the given one (a transmutation) → logged warning, none.
4. The save guard (§7.6) with the given item data and version: "do not write" → the **stored item** (no write).
5. Replace the entry, write both texts (the stacked text unchanged) at the read revision. Written → logged
   `Updated instanced item <catalogId> for characterId <c> and userId <u>` and the new item. Not written → next
   attempt (logged warning `... attempt <n>/5 - retrying`).

Five conflicts → logged error, none.

### 6.5 Error mapping (`TxError`)

| Error | Answer |
| --- | --- |
| Insufficient balance (wallet) | logged warning; 409 `{"code":null,"message":"Insufficient balance","payload":{}}` |
| Insufficient banked prestige | logged warning; 409 `... "Insufficient banked prestige progress" ...` |
| Insufficient stacked item | logged warning; 409 `... "Insufficient stacked item" ...` |
| Mismatch | 409 `... "Transaction id reused with a different request body" ...` |
| Conflict (pending duplicate, revision conflict) | 409 `... "Transaction already in progress" ...` |
| Denied by a grant hook | its status and JSON body as given |
| Policy (game data missing or unreadable, impossible quantity) | logged error; 500 page |
| Repository | §3.3 |

Display texts (logs, and what other areas see through §12): insufficient balance and prestige carry the wallet's
message; insufficient stacked item is
`Insufficient stacked item: user <u> has <owned> of <catalogId>, cannot remove <requested>` (numbers as JavaScript
prints them); mismatch `Inventory transaction <t> was reused with a different request body`; conflict
`Inventory transaction <t> is already in progress (concurrent duplicate request)`; denied `denied (<status>)`;
policy `[game-data] <text>` or `Invalid quantity <String(quantity)> for non-stackable catalog item <catalogId>`.

---

## 7. Item rules

Pure functions over JSON item lists; the transaction around them is the only place that touches the database.

### 7.1 The request hash (`ComputeInventoryRequestHash`)

SHA-256 hex of the **stable stringify** of
`{"addInstancedItems":..,"addStackedItems":..,"removeInstancedItems":..,"removeStackedItems":..,"saveInstancedItems":..}`
(each collection as given, `null` replaced by `[]`). Stable stringify: arrays in order, objects with their keys sorted
by UTF-16 code units, every key and scalar written as `JSON.stringify` writes it, no spaces. The hash of five empty
collections is the SHA-256 of
`{"addInstancedItems":[],"addStackedItems":[],"removeInstancedItems":[],"removeStackedItems":[],"saveInstancedItems":[]}`.

### 7.2 Instanced items

- **Placeholder id** (`NeedsMintedInstanceId`): an object whose `instanceId` is not a string, or is empty, or equals
  its `catalogId`. A non-object is not a placeholder.
- **Minted id** (`MintInstanceId`): take three UUID v4 texts from core's replay id source (`next_uuid`), concatenate
  their 108 bytes, SHA-256; read the first 130 bits of the digest most-significant first as 26 groups of 5 bits; each
  indexes `ABCDEFGHIJKLMNOPQRSTUVWXYZ234567`. Always 26 characters; deterministic under a replay seed.
- **Normalization** (`NormalizeInstancedItems`, per entry): dropped unless an object with a non-empty string
  `catalogId` that is not `DYE_BANNER_BACKGROUND_DEFAULT` or `DYE_BANNER_SIGIL_DEFAULT` (1.12 starter dyes never
  stored), and a non-empty string `instanceId`. Kept entries keep every field in order; `itemData` absent or `null`
  is set to `null`, `updateVersion` not a number is set to `0` (a key that was absent is appended at the end).

### 7.3 Applying a transaction (in Node's order)

Inputs: the user id (for messages), the stored instanced and stacked arrays, the normalized adds, the raw removes and
saves, the touched balances (an ordered map of catalog id to number from §9.1), the minter.

1. **Removal check** against the stored stacked array (before any add, so a request never funds its own removal):
   owned = per `catalogId` the sum of `Number(quantity)` (NaN as 0) over stored entries; requested = per `catalogId`
   (first-seen order) the sum of each removal's quantity when it is a number, else 0, skipping entries without a
   string `catalogId` and those starting `CURRENCY_`. The first requested above owned is an **insufficient stacked
   item** (requested, owned).
2. **Instanced adds**, each in order: a placeholder with a string `catalogId` gets a minted id (logged
   `[InstanceId] minted <id> for <catalogId> (server sent <JSON of the old instanceId>) transactionId <t>`); then
   normalized (§7.2); a kept entry removes any stored entry with the same `instanceId`, is appended to the instanced
   array and to `createdInstancedItems`.
3. **Stacked adds**, each in order, over the stored stacked array (an index of `catalogId` to the **last** stored
   entry with it):
   - a starter dye id: skipped entirely;
   - an entry with a string `catalogId` and a safe-integer quantity above 0 is **accepted** (`{catalogId, quantity}`,
     for the annotator);
   - a `catalogId` starting `CURRENCY_`: nothing more (the wallet holds currency, reported in step 6);
   - no string `catalogId`: appended as sent and reported;
   - a `catalogId` already indexed: that entry's quantity becomes the number value of `Number(stored) +
     Number(added)` and the entry is reported;
   - else appended as sent (all its fields kept), indexed, reported.
   A report is a reference to the stored entry, so later changes in this call (step 5) show in it, and an entry added
   twice is reported twice.
4. **Instanced removes**: the non-empty string `instanceId`s of the removal entries; none → nothing. A removal whose
   `instanceId` equals its `catalogId` and matches nothing stored is logged warning
   `[InstanceId] remove references placeholder <id> that matches no stored item - the server did not adopt a minted id`.
   Every stored entry with one of those ids is removed.
5. **Stacked removes** (only when the list is non-empty): for each entry with a string `catalogId`, the **first**
   remaining stored entry with it (none: skipped): its quantity becomes the number value of `Number(stored) − by`,
   `by` being the removal's quantity when it is a number, else the whole stored quantity. Then every stored entry
   whose `Number(quantity)` is not above 0 is dropped (pre-existing zero entries included).
6. **Reports**, appended to `updatedStackedItems` after step 3's: for each removal entry (in order) with a non-empty
   string `catalogId` not starting `CURRENCY_` and not already reported (by a step-3 report or an earlier removal),
   `{"catalogId":<id>,"quantity":<the remaining stored entry's quantity, or 0 when it is gone>}`; then each touched
   balance in touch order as `{"catalogId":<id>,"quantity":<balance>}`.
7. **Stale currency rows**: every stored stacked entry whose `catalogId` starts `CURRENCY_` is dropped.
8. **Saves**, each in order: normalized (§7.2; dropped entries skipped); the stored instanced entry whose `instanceId`
   strictly equals it (none: logged warning `Ignoring save for unknown instanceId ...` and skipped, a save is never a
   grant); a different `catalogId` (strict) is logged warning `Rejecting save that would change catalogId ...` and
   skipped; the save guard (§7.6) with its `itemData` and `updateVersion`: a new item replaces the stored one and is
   appended to `updatedInstancedItems`.

Outputs: the new instanced and stacked arrays, created, updated instanced, updated stacked, accepted stacked adds.
Stored order: kept entries in their order, then new ones appended.

### 7.4 Examples (authored, not recorded)

| Stored stacked | Request | Stored after | `updatedStackedItems` |
| --- | --- | --- | --- |
| `MAT` 4, `CURRENCY_NOTES` 9 | add `MAT` 3, `NEW` 2 (with `"source":"x"`), `CURRENCY_NOTES` 50, `DYE_BANNER_BACKGROUND_DEFAULT` 1 | `MAT` 7, `NEW` 2 with its `source` | the same two (plus the touched balances) |
| `MAT` 6 | add `MAT` 3, remove `MAT` 5 | `MAT` 4 | `MAT` 4 (the add's report shows the removal) |
| `CORE` 1 | remove `CORE` 1, `CURRENCY_NOTES` 10 | empty | `CORE` 0 |
| `CORE` 2 | add `CORE` 5, remove `CORE` 2 then `CORE` 1 | — | insufficient: has 2, cannot remove 3 |
| `OWN` 3 | remove `OWN` (no quantity), `CURRENCY_NOTES` 99999 | empty | `OWN` 0 (currency is the wallet's to guard) |
| none, touched `CURRENCY_B` 5 then `CURRENCY_A` 7 | add `MAT` 1 | `MAT` 1 | `MAT` 1, `CURRENCY_B` 5, `CURRENCY_A` 7 |

| Stored instanced | Instanced adds | Result |
| --- | --- | --- |
| `KEEP` (`OLD`) | `WP_X` with id `WP_X`; `WP_X` with id `WP_X` and `"extra":true`; `LT` with id `KEEP`, `itemData` `"d"`, version 4; a starter dye; empty `catalogId`; the number 7 | created: two minted `WP_X` (the second keeps `extra` before the appended `itemData`, `updateVersion`), then `LT`/`KEEP`; stored order minted, minted, `KEEP` (replaced, moved to the end) |

### 7.5 Save examples (stored `WP`/`i`, `itemData` `"old"`, version 3)

| Save | Updated? |
| --- | --- |
| `itemData` `"new"`, version 4 | yes, version 4 |
| version 2 | no (stale) |
| `"old"`, version 3 | no (nothing changed) |
| `"div"`, version 3 | yes (same version, divergent data: applied and logged for review) |
| no version | no (normalizes to 0, stale against 3) |
| `catalogId` `BETTER`, version 9 | no (transmutation) |
| another `instanceId` | no (not owned) |

### 7.6 The save guard (`ApplyItemDataSave`)

Stored version: the stored `updateVersion` when a number, else 0. Incoming version: the given one when a number, else
the stored version. Incoming data: the given value, `null` when none; stored data: the stored `itemData`, `null` when
absent.

- incoming version < stored → logged warning `Ignoring stale itemData save ...`; do not write.
- equal versions and strictly equal data → do not write.
- equal versions, different data → logged warning `Same-version divergent itemData save ...`; write.
- Write: a copy of the stored entry with `itemData` = incoming data and `updateVersion` = the number value of the
  incoming version (existing keys keep their position).

So the single-item route with no `itemData` and no version over stored non-null data writes `itemData: null`
(§14 item 12).

### 7.7 Weapons at base level (`weaponBaseLevel.ts`, Reforged patch 0036)

The 1.14.7 paks carry no per-level power tables, so weapons are served at item level 0; the stored level is untouched.
It runs when the version's `weapons_at_base_level(settings)` hook says so (1.14.7: unless `KEEP_WEAPON_ITEM_LEVELS`
trimmed is `true`, ASCII any case).

- **One item**: a `catalogId` string starting `WP_`, an `itemData` string containing `CurrentLevel` that parses
  (`JSON.parse`) to an object whose `CurrentLevel` is a JSON number above 0: the data re-written by `JSON.stringify`
  with `CurrentLevel` set to `0` in place. Anything else is unchanged.
- **A response**: a copy in which every object having both a `catalogId` and an `itemData` key, found in arrays or
  object fields up to depth 4 (the response itself is depth 0; an item at depth 5 is not served), has its `itemData`
  served as above. Applied to the inventory read, the transaction result (new or replayed) and the single item save.
  Not applied to the sentinel echoes, the forwarded answer or the reforge routes.

---

## 8. Storage policy (`inventoryStoragePolicy.ts`)

### 8.1 The file (game data, never committed)

`inventory_storage_policy.json` in the module's backend game-data folder (core §8.1), made from the host's catalog
export by the game-data generator (gamedata spec §5.8). Read as core's loader reads (UTF-8, a leading BOM dropped,
`JSON.parse` rules). Shape:

`{"schema":1,"entries":{"<catalogId>":{"storage":"instanced"|"stacked","maxQuantity":<number>,"tags":["<tag>",..]},..}}`

Unknown fields are ignored. A `storage` other than the two is "other" (present, not acted on). `maxQuantity` not a
number reads as NaN; `tags` keeps the string entries only. A catalog id absent from `entries` has no policy (it keeps
its incoming representation).

**Loading.** Loaded on first use and kept for the life of the module (a module without the file fails when first
asked, not at start). Missing file: policy error
`[game-data] Required game data file is missing: <path>` followed by the hint lines
`This project does not ship game data. Generate it from your own installation (see docs/GENERATING_GAME_DATA.md), or
copy the matching *.example.json for a smoke test.`; unreadable or not JSON: `[game-data] <path>: <error>`; no game
data folder: `[game-data] this module has no game data folder for inventory_storage_policy.json`. A failed load is not
kept (the next call tries again).

### 8.2 Ownership unlock (`IsOwnershipUnlock`)

A `stacked` entry whose `maxQuantity` is exactly 1, or with a tag that lower-cases to one of `transmog`, `owned`,
`accessory`, `dye`, `bannercustomization`, `emoji`, `emote`.

### 8.3 Grant normalization (`NormalizeInventoryGrantCollections`)

Only the two add collections, at the authoritative boundary, after the hash.

1. **Stacked adds**, in order: an entry whose catalog id's policy is not `instanced` stays a stacked add as sent.
   An `instanced` one needs `Number(quantity)` to be a whole number, at most 2^53 − 1 in magnitude, above 0 and at
   most 1000, else the policy error `Invalid quantity <String(quantity)> for non-stackable catalog item <catalogId>`
   (the whole transaction fails, 500 page). It becomes that many instanced adds
   `{"catalogId":..,"instanceId":<deterministic>,"itemData":null,"updateVersion":0}`, the deterministic id being the
   first 26 characters of the SHA-256 hex of `inventory-storage-normalize:<transactionId>:<catalogId>:<ordinal>`,
   upper-cased, the ordinal counting from 0 across the whole call. Correction `stacked -> instanced` with the
   quantity.
2. **Instanced adds**, in order, appended after step 1's: an entry whose policy is not `stacked` stays as sent. A
   `stacked` one adds 1 to the first stacked add with that catalog id (its quantity NaN reads 0; the sum as a number
   value) or appends `{"catalogId":..,"quantity":1}`. Correction `instanced -> stacked` x1.

So the normalized instanced adds are the converted stackables first, then the kept instanced adds; the normalized
stacked adds are the kept stacked adds, then the converted instanced ones. Same transaction id and body give the same
ids, so a replay is stable.

### 8.4 Storage repair on read (`MigrateInventoryStorageArrays`)

1. Instanced entries whose policy is `stacked` are taken out and counted per catalog id (first-seen order); the rest
   stay.
2. For each counted id: existing = the sum over stacked entries with that id of `max(Number(quantity), 0)` (NaN as
   0); those entries are removed; one `{"catalogId":..,"quantity":<final>}` is appended, final = 1 for an ownership
   unlock, else existing + count. Change result `deduplicated-to-one` when an ownership unlock had more than 1 in
   total, else `moved`.
3. **Issues** (logged only): stacked entries whose policy is `instanced` stay where they are, for manual review.

| Policy | Stored | After |
| --- | --- | --- |
| `WP_AC_TRIALS_00` stacked, max 1, Transmog; `LT_TRIALS_00` instanced | instanced A, B (`WP_AC_TRIALS_00`), C (`LT_TRIALS_00`); stacked `WP_AC_TRIALS_00` 1 | instanced C; stacked `WP_AC_TRIALS_00` 1; change migrated 2, existing 1, `deduplicated-to-one` |
| `MAT_STACK` stacked, max 99, Material | instanced `MAT_STACK` m1; stacked `MAT_STACK` 4, `LT_TRIALS_00` 2 | stacked `LT_TRIALS_00` 2, `MAT_STACK` 5 (`moved`); one issue `LT_TRIALS_00` 2 |

### 8.5 Other readers

The policy is public (§11.2): the auth crate's tester kit asks "is this catalog id instanced" (auth runtime's game-data
seam; restricted answers false when the file is missing), the Reforged tester-max kit refuses catalog ids absent from
it, and `isles-db`'s import runs §8.4 over legacy rows.

---

## 9. Currency in a transaction (`ApplyCurrencyDeltas`)

### 9.1 The rule

Runs inside the transaction (§6.1 step 5.2) through the wallet port (§11.3). Inputs: the account, the user id, the
normalized stacked adds, the raw stacked removes, the transaction id, "challenge funding allowed", "operator test
funding". Output: the **touched balances**, an ordered map (first touch keeps its position, a later touch updates the
value) of catalog id to the number value of the balance after.

- User id empty or `INVALID`: nothing, an empty map.
- **Sparkforge hearts**: the floor of (the sum of the safe-integer quantities above 0 of removals of
  `CURRENCY_PJM_PRESTIGE_EMPTY` (Aetherspark)) / 200.
- **Each add** whose `catalogId` is a string starting `CURRENCY_`, in order, quantity `q` = `Number(quantity)`:
  1. Challenge funding allowed: bounties' `fund_challenge_reward` (§12 W5). Funded → skip the gate.
  2. Else, challenge funding allowed and bounties' `is_daily_challenge_reward_limited` (§12 W6) → **skip this add**
     entirely (not credited, not touched; the rest of the transaction goes on).
  3. Else operator test funding → skip the gate.
  4. Else `CURRENCY_PJM_PRESTIGE_FILLED` (Aetherheart) with hearts left and `q` a safe integer above 0: converted =
     min(q, hearts), hearts reduced by it; any remainder (q − converted) goes through the prestige gate (§9.2).
  5. Else the prestige gate for `q`.
  6. Credit: bounties' `add_currency(catalogId, q)` (§12 W2); its new balance is touched.
- **Each removal** whose `catalogId` is a string starting `CURRENCY_`, in order, `q` = `Number(quantity)` (NaN as 0):
  `CURRENCY_PLATINUM` → bounties' `spend_platinum(q)` (§12 W3), every bucket it reports (the aggregate last) touched;
  any other → `add_currency(catalogId, −q)`, touched.
- Errors: the wallet's insufficient balance → **insufficient balance** (409); an unsafe catalog id (not 1 to 128 ASCII
  letters, digits, `_`) → a data error (500 page); repository errors as they are.

A zero quantity still touches: `add_currency` with a zero delta answers the current balance and creates the starter
wallet when there is none (§14 item 14).

### 9.2 The prestige gate (`SpendBankedPrestigeForReward`)

A reward currency a prestige track pays per level may be credited only against banked prestige levels, so a game
server cannot mint it.

- `q` not finite or not above 0: allowed.
- No `progression_config.json`, or it does not load, or no prestige source pays this catalog id: allowed (§14 item 15).
- Else, for each source (§9.3) in order, reading the live track of the source's progression id in the transaction:
  claimed before = its `prestige_claimed` (none 0); progress (none 0); prestige levels = floor(max(progress − cap,
  0) / xp per level); banked = max(levels − claimed before, 0); available = banked × reward per level, added to the
  total. With something still needed and available: take min(needed, available); levels to spend = take / reward per
  level, and a source whose levels to spend is not a whole number is skipped; else claim (source, claimed before,
  levels, min progress = trunc(cap + (claimed before + levels) × xp per level)) and reduce what is needed by levels ×
  reward per level.
- Still needed: **insufficient banked prestige**,
  `Insufficient banked prestige: user <u> requested <q> of <catalogId> but only <total> banked across eligible tracks`.
- Each claim: `progression_tracks.claim_prestige_levels(account, source, claimed before, levels, min progress)`; a
  `false` (a concurrent claim) is the same error. All claims are in the transaction, so a refusal undoes them.

### 9.3 Prestige sources (from `progression_config.json`)

The file is progression's (gamedata spec §3.7; shape `{"payload":{"paths":[..]}}`). For each path with a non-empty
string `progression_id` whose `prestige.xp_per_level` (as `Number`) is finite and above 0: its cap is the sum of its
`requirements[].xp_required` (non-finite as 0); for each `prestige.free_rewards.stacked_items[]` entry with a string
`catalog_id` and a finite `Number(quantity)` above 0: a source (progression id, xp per level, reward per level =
quantity, cap) for that catalog id, in file order. **Prestige-gated** catalog ids are those with a source. Loaded once
and kept like the policy (§8.1); a failed load answers "no sources".

### 9.4 Challenge reward grant

`catalogId` equal to the version's `seasonal_coin` table's catalog id (1.14.7: `CURRENCY_SEASONAL_COIN`, at most 1000
a claim) and a finite quantity above 0 and at most that ceiling. No table: never.

---

## 10. Test reward multiplier (`testMultipliers.ts`, reward half)

A private server (friends over Tailscale) may multiply the loot its game servers grant. The XP half (`multiply_xp_grant`)
is progression's (its spec); the reader below is shared, and both live in this crate.

### 10.1 Reading the settings (`ReadTestMultipliers`)

- Each of `REFORGED_XP_MULTIPLIER`, `REFORGED_REWARD_MULTIPLIER` (raw): unset or blank after JS trim → 1; else the
  text trimmed must be only ASCII digits, `.`, `-`, `+`, `e`, `E` and parse as a decimal floating-point number, finite,
  from 1 to 100 (`MAX_TEST_MULTIPLIER`). Either failing → off, reason `a multiplier is not a number from 1 to 100`.
- Both 1 → off, reason `not set`.
- `REFORGED_SERVER_MODE` (raw) not exactly `private` → off, `this is not a private server`.
- `REFORGED_SERVER_HOST` (raw, unset as empty) not a Tailscale host → off,
  `the server host is not a Tailscale address`.
- Else on with both values.

**Tailscale host**: trimmed (JS trim) and lower-cased; an IPv4 address in 100.64.0.0/10; or a name ending `.ts.net`
whose remaining part is non-empty and every `.`-separated label non-empty and made of `a-z`, `0-9`, `-`.

The start-up line (`[TestMultiplier] ON: XP x<xp>, rewards x<rewards> (private Tailscale server)` or
`[TestMultiplier] off (<reason>)`) is logged by the module at start (core §12 keeps that wiring with the composer).

### 10.2 Multiplying (`MultiplyRewardItems`)

Nothing (no change) when the rewards multiplier is 1 or the body's `source` is not a reward source: a string equal to
`PlayerLoot`, `MISC`, `patrol_chest` or `quest`, or starting `BUNDLE_GATHER_`. Else each stacked add, quantity `q` =
`Number(quantity)`, is kept as it is unless its `catalogId` is a string starting `CURRENCY_NOTES`, `CURRENCY_PJM_`,
`BREAK_` or `GATHER_`, `q` is a finite whole number above 0, and it is not excluded; a multiplied entry keeps its
fields and order with `quantity` = the number value of floor(q × rewards). Excluded (`POST /inventory`): a
prestige-gated catalog id (§9.3) or a challenge reward grant (§9.4). No entry changed → no change.

| Settings | Result |
| --- | --- |
| mode `private`, host `100.101.102.103`, XP `5`, rewards `3` | on (5, 3) |
| the same with host `my-pc.tail1234.ts.net` | on |
| mode `public`, or unset; host `203.0.113.7`, `192.168.1.10`, `hunts.example.net`; XP `1000`; rewards `0.5`; XP `abc`; both unset | off, with a reason |
| host `100.63.255.255` or `100.128.0.1` | not Tailscale |

---

## 11. The seam

### 11.1 Shape

This part is the module `inventory` of the crate `rustic-isles-economy` (lib, MIT, `crates/rustic-isles-economy`),
with `multipliers` beside it. The crate depends on dainty-lass MIT crates only: `rustic-isles` (core: `Shared`,
`Handlers`, `Call`, `Service`, `Method`, `VersionTables`, `VersionHooks`, `NoPlayerPolicy`, `AccountId`, `UserId`,
`GameDataRoot`, the replay clock and ids, `read_body_parts`), `rustic-isles-store` (`IslesStore`, `Repositories`,
`RepoError`, the records), `rustic-isles-auth` (`AuthData`, `Auth::game_auth`), `rustic-isles-characters` (§11.3),
`rifts-seam` (guard calls, `RuntimeAuth`, `Reply`, `SeamError`), `module-api` (`ActorContext`, `GlobalId`,
`HuntRewards`, `HomeApi`), `service-kit` (`js`, `jsjson`, `json`, `body`), and workspace `axum`, `http`, `serde_json`
(own `preserve_order`, core §17 item 8), `sha2`, `tokio`, `tracing`, `thiserror`, `uuid` (v4 for the reforge id).

### 11.2 Interface (names are the contract the composer and the other two parts are written against)

| Item | Contract |
| --- | --- |
| `pub fn register(handlers: &mut Handlers, ports: InventoryPorts)` | Adds the eight handlers of §3 (core §5.7) |
| `pub struct InventoryPorts` (`Clone`, `Default`) | §11.3: `auth`, `wallet`, `starter`, `characters`, `observer`, `ring`, and `repos: Option<Repositories>` (tests; none = the store's own) |
| `pub struct TxContext<'a> { shared: &Arc<Shared>, store: &IslesStore, ports: &InventoryPorts }` | `repos()`; `owner_of(user, character, check_character) -> RepoResult<Option<AccountId>>` (§6.1 step 1) |
| `pub struct TxRequest { user_id, character_id, transaction_id, add_instanced, add_stacked, remove_instanced, remove_stacked, save_instanced }` | §6.1 (collections as JSON values, `Null` when absent) |
| `pub struct TxOptions { ownership_already_validated, diagnostic_label, on_grant: Option<GrantHook>, allow_challenge_reward_funding, operator_test_funding, economy: Option<Economy> }` (`Default`) | §6.1 |
| `pub struct Economy { source, p2p_session_id, store_sku_id }` | Monitoring only |
| `pub type GrantHook`, `pub type Annotator` | A one-shot callable given the transaction (`&mut Tx`, `&Repositories`) answering `Result<Option<Annotator>, TxError>`; an annotator is a one-shot callable given (created instanced items, accepted stacked adds) answering optional JSON fields for the ledger row |
| `pub enum TxOutcome { NotOwned, Done(Value) }`, `pub enum TxError { InsufficientBalance(String), InsufficientPrestige(String), InsufficientStackedItem(..), Mismatch(String), Conflict(String), Denied { status, body }, Policy(PolicyError), Repo(RepoError) }` | §6.5 |
| `pub async fn run_inventory_transaction(ctx, TxRequest, TxOptions, request_id: Option<&str>) -> Result<TxOutcome, TxError>` | §6.1 |
| `pub async fn get_inventory(ctx, user_id: Option<&str>, character_id, request_id) -> Result<Option<Value>, TxError>` | §6.3 |
| `pub async fn update_instanced_item(ctx, &ItemUpdate, request_id) -> Result<Option<Value>, TxError>`, `pub struct ItemUpdate` | §6.4 |
| `pub fn player_actor(&AccountId, request_id) -> ActorContext`, `pub fn system_actor(request_id) -> ActorContext` | §2 |
| `pub fn inventory_record(character, &AccountId, instanced, stacked) -> InventoryRecord` | A new record at revision 0 (seeding callers) |
| `pub mod items` | `compute_inventory_request_hash`, `stable_stringify`, `mint_instance_id`, `needs_minted_instance_id`, `normalize_instanced_item`, `apply_item_data_save`, `apply_transaction` (with `Delta`, `Applied`, `InsufficientStackedItem`), `is_wallet_currency_id`, `number_value` (§7) |
| `pub mod storage_policy` | `StoragePolicy { from_document, load, entry }`, `Storage { Instanced, Stacked, Other }`, `PolicyEntry`, `PolicyError`, `is_ownership_unlock`, `normalize_inventory_grant_collections` (`NormalizedGrants`, `Correction`), `migrate_inventory_storage_arrays` (`Migration`, `MigrationChange`, `MigrationIssue`), `read_game_data` (§8) |
| `pub mod prestige` | `ProgressionConfig { from_document, load, is_prestige_gated_reward, prestige_sources }`, `PrestigeRewardSource`; `is_prestige_gated_reward(shared, id)` (§9.3) |
| `pub mod reforge` | `reforge_reset` (`ResetOutcome`, `ResetResult`), `plan_purchase_restore` (`PurchasePlan`), `grant_banked_restore` (`RestoreGrant`, `GrantResult`, `GrantFailure`), `single_track_payload` (§5) |
| `pub mod weapon_level` | `weapon_item_data_at_base_level`, `serve_weapons_at_base_level(value, enabled)` (§7.7) |
| `pub mod validation` | `validate_inventory_transaction_body`, `validate_instanced_item_update_body`, `inventory_body_has_grant_or_spend` (§4.6) |
| `pub mod multipliers` (crate root) | `TestMultipliers { xp, rewards, reason; OFF; startup_line }`, `read_test_multipliers`, `is_tailscale_host`, `is_reward_source`, `is_multiplied_reward_item`, `multiply_reward_items`, `MAX_TEST_MULTIPLIER`; `multiply_xp_grant` is progression's (its spec) |
| `pub struct CommittedTransaction` | §6.1: `account`, `user_id`, `transaction_id`, `source`, `granted`, `currency_credits`, `currency_debits`, `result` |

### 11.3 Ports (what this part takes from outside; each has a production default)

| Port | Contract | Default |
| --- | --- | --- |
| `Authenticator` | `authenticate(shared, {method, path, headers, extensions, request_id}) -> Authenticated(caller) or Answer(response)`; the caller is user id, is game server, runtime | The `AuthData` in the extensions (from auth, or put there by the composer), else empty 401 |
| `Wallet` | `apply_currency_deltas(tx, repos, change) -> Result<touched map, WalletError>` (§9.1); `merge_into_stacked(tx, repos, account, user, stacked) -> RepoResult<Vec<Value>>` (§6.3 step 4); `wait_for_fundable_season_claim(store, account) -> bool` (§12 W7). `WalletError { InsufficientBalance(String), InsufficientPrestige(String), Repo }` | §9 over bounties' wallet (§12) |
| `StarterBootstrap` | `ensure(tx, repos, shared, account, user, character) -> RepoResult<Result<InventoryRecord, CharacterOwnership>>` | The characters crate's starter bootstrap: ownership gate (the live character of that account, else `Character <c> does not belong to user <u>`), then creates a missing inventory and loadout from the version's starter manifest, or appends starter weapon parts and stacks a newer manifest added, answering the inventory |
| `Characters` | `first_character_id(shared, store, actor, user) -> RepoResult<Option<String>>` | The characters crate's character list for a user (`characters_for_user`, which finds or creates the account and makes the first character when it has none) and its first entry's `id` string; a characters error that is not a repository error becomes a data error with its text |
| `Observer` | `committed(shared, &CommittedTransaction)`; `reforge_reset(shared, account, at_ms)`. Never on the wire, never fails a request | Bounties' economy ledger for `committed` (its spec) and the Reforged API's activity hooks (`record_inventory_transaction`, `record_reforge`) through a slot the Reforged unit fills; none → nothing |
| `RingInstances` (new slot trait, defined here so matchmaking can fill it) | `ring_instance_for_player(&GlobalId) -> Option<GlobalId>`: the hunt instance the player was placed in, else the last ring instance | None (a new hunt id, §4.4.1) |

`CharacterOwnership` displays `Character <characterId> does not belong to user <userId>`.

### 11.4 Registration by the composer

Restricted's composer (core §11.4) calls `register(&mut handlers, InventoryPorts::default())`, puts the
`IslesStore` in the slots, and fills the `RingInstances` and Reforged activity slots when those units are composed.
Until the characters crate lands, the `StarterBootstrap` and `Characters` defaults are restricted adapters passed in
the ports.

---

## 12. Calls into and out of this part

The three parts of `rustic-isles-economy` call each other in a cycle; these are the contracts they meet on. "In tx"
means the call takes the caller's `&mut Tx` and `&Repositories` and adds its writes to the caller's transaction (one
`IslesStore::write` or `account_write` closure; committed on `Ok`, rolled back on `Err`); it never opens its own.

### 12.1 Out: inventory → bounties-store

| # | Call | Inputs | Output | Errors | Atomicity |
| --- | --- | --- | --- | --- | --- |
| W1 | `wallet::get_wallet` | tx, repos, account | balances, ordered (catalog id, f64) | repository | In tx; `wallets.create_if_missing` with the starter balances (`CURRENCY_NOTES` 5000, `CURRENCY_PJM_WEAPON`, `_ARMOR`, `_ALCHEMY`, `_AIRSHIP`, `_FISHING` 50 each), never resetting an existing wallet |
| W2 | `wallet::add_currency` | tx, repos, account, catalog id, signed delta (f64) | the new balance | `Insufficient { account, catalog_id, delta, current }` (display `Insufficient balance: user <account> has <current> of <id>, cannot apply delta <delta>`), `UnsafeCatalogId(id)`, repository | In tx; an empty id, a non-finite or zero delta answers the current balance (via W1, so it may create the wallet); a debit never goes below zero (one guarded `increment_balance`); a credit with no wallet creates the starter wallet first |
| W3 | `wallet::spend_platinum` | tx, repos, account, amount | each bucket debited with its new balance, in the spend order, then (`CURRENCY_PLATINUM`, the new aggregate) | `Insufficient` naming `CURRENCY_PLATINUM` with the aggregate held; repository | In tx; `trunc(amount)` not above 0 → nothing; never a partial plan |
| W4 | `wallet::merge_wallet_into_stacked` | balances, stacked items | stacked items with each balance's entry quantity replaced (number value), the rest appended `{catalogId, quantity}` | — | Pure |
| W5 | `challenge_rewards::fund_challenge_reward` | tx, repos, the version's coin, account, catalog id, quantity, transaction id | the funded claim or none | repository | In tx; only for a challenge reward grant (§9.4); records the credit on one claimed, uncredited season challenge with a version-checked `bounty_states.save_if_version`; a version miss is none |
| W6 | `challenge_rewards::is_daily_challenge_reward_limited` | tx, repos, coin, account, catalog id, quantity | bool | repository | In tx, read only |
| W7 | `wait_for_fundable_season_claim` | store, account | bool | — (errors read as "not found") | Outside any tx: up to 1.5 s, a read every 100 ms of the bounty state for a fundable season claim (wall clock: §14 item 5) |
| W8 | `platinum::PLATINUM_AGGREGATE_ID` | — | `CURRENCY_PLATINUM` | — | Constant |
| W9 | The economy ledger's commit of a `CommittedTransaction` | the committed transaction, settings, game data | — | logged only | After the inventory commit, in the background; never fails the request |

### 12.2 Out: inventory → progression

| # | Call | Inputs | Output | Errors | Atomicity |
| --- | --- | --- | --- | --- | --- |
| P1 | `reforge::reforge_mode` | shared | `On`, `Log`, `Off` (§5.1) | — | Pure |
| P2 | `reforge::classify_grant` | mode, progression id, amount, the ExperienceTrack row, now ms | `ReforgePlan { Normal, Prestige{experience_track, reforge}, Restore{experience_track, reforge}, Refuse(reason) }` | — | Pure |
| P3 | `reforge::mark_reforge_use` | tx, repos, account, plan | `Ok` or `ReforgeConflict(text)` (`reforge <id> changed concurrently (restore)`) | repository | In tx; for `Restore`, `progression_tracks.update_reforge` expecting `{"status":"open","prestigeGranted":..,"bankedRestored":..}` as classified, setting `bankedRestored` = banked and, when the +1 was already granted, `status` `closed` and `closedAt` now; `Normal` marks nothing |
| P4 | `reforge::finish_stale_reforge` | tx, repos, account, user id, ExperienceTrack id, the open record | whether this call closed it | repository | In tx; closes the record (all follow-ups marked done) by compare-and-set, then increments the ExperienceTrack by the unrestored banked XP and the PrestigeTrack by 1 when not granted |
| P5 | `reforge::STALE_REFORGE_MS` | — | 120 000 | — | Constant |
| P6 | `tracks::{is_reforgeable_track, track_cap_xp, MAX_PROGRESSION_GRANT_AMOUNT, grant_request_hash, is_valid_grant_id}` | ids, shared | §5.1; 100 000; SHA-256 hex of `JSON.stringify({"version":1,"userId":..,"progressionId":..,"amount":..})`; `^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$` | — | Pure (the cap reads `progression_config.json`'s curves) |
| P7 | Repositories | `progression_tracks.{get, begin_reforge, increment, claim_prestige_levels}`, `progression_grants.{find_by_grant_id, insert_applied}` | store spec §10 | repository | In this part's own `account_write` (§5.2.1, §5.3.2, §9.2) |

### 12.3 In: other parts → inventory

| # | Caller | Call | Inputs | Output, errors | Atomicity |
| --- | --- | --- | --- | --- | --- |
| I1 | bounties-store (store purchase) | `run_inventory_transaction` | user, character, transaction id; instanced and stacked adds, stacked removes, empty instanced removes and saves; options: ownership validated as the signed store token says, the purchase's label, a grant hook when the SKU also grants entitlements or progression, economy `STORE` with the SKU id | `Done(result)`; `NotOwned`; `InsufficientBalance`, `Conflict`, `Mismatch`; any other error as its text | One inventory transaction; the grant hook's entitlement and progression writes (`entitlements.grant_if_missing`, `progression_tracks.increment`, specified by the bounties part) are in tx and roll back with it |
| I2 | progression (Slayer's Path) | `get_inventory` | user, the first character, no request id | the inventory's `instancedItems` | §6.3 (its own `account_write`, may write) |
| I3 | progression (Slayer's Path rewards) | `run_inventory_transaction` | instanced adds and instanced removes (`{catalogId, instanceId}`), ownership validated, label `slayers-path-reward`, economy `OTHER` | `Done` → granted; `NotOwned` → false; error → its text | One inventory transaction |
| I4 | Reforged API (tester max) | `run_inventory_transaction` with `operator_test_funding`; `StoragePolicy::load` | kit stacks and currencies | as I1 | One transaction per kit entry |
| I5 | auth (tester kit game-data seam) | `StoragePolicy::load`, `entry` | catalog id | instanced or not (missing file: false) | Read only |
| I6 | `isles-db` import | `StoragePolicy::from_document`, `migrate_inventory_storage_arrays` | legacy arrays | §8.4 | Pure |
| I7 | progression (XP route) | `multipliers::read_test_multipliers` | settings | §10.1 | Pure |
| I8 | Reforged API (activity) | reads `CommittedTransaction` | — | — | Through the observer slot |

The progression and bounties parts must not call the inventory handlers; they call I1 to I4 with a `TxContext` built
from the module's `Shared`, the store slot and `InventoryPorts::default()`.

---

## 13. Test cases

### 13.1 What may be copied

Nothing recorded: `tests/golden/inventory/*` (scenarios, expected answers, the golden game data) stays in restricted
and replays there against this crate once composed; a failure there is a bug on this side. The tables here are
authored cases written from the rules above, with their own values.

### 13.2 Route cases (with a fake authenticator reading the caller from a test header, a temporary SQLite store, and a policy file written by the test)

| # | Request | Expect |
| --- | --- | --- |
| R1 | B69 and B68 (`/inventory//dauntlessrel-1.14.7:647472`) as a game server | 200 `{"code":"NONE","message":""}`; no caller → empty 401 |
| R2 | B70 as a game server for its user's character | 200; a weapon whose stored `CurrentLevel` is 5 answers 0 |
| R3 | B70 as a player with another user in the path | the player's own inventory (path ignored) |
| R4 | B70 for a character of another user, or unknown | empty 400 |
| R5 | B70 with user `INVALID` (game server) | 200 empty lists with the path's character id, nothing written |
| R6 | B71 as a game server, policy `ConfiguredAccount` | the first character's inventory; as a player: empty 400; `INVALID`: `characterId` `INVALID` |
| R7 | B72 grant of a stack and a currency | 200; stack merged; currency reported last with the wallet's balance; the blob holds no `CURRENCY_` row |
| R8 | B72 the same id and body again, then with the keys reordered | the same 200 body, applied once |
| R9 | B72 the same id, another body | 409 reused |
| R10 | B72 removing more than owned | 409 insufficient stacked item; no ledger row; the same id with a fixed body then applies (200) |
| R11 | B72 overspending a currency | 409 insufficient balance |
| R12 | B72 instanced add whose `instanceId` equals its `catalogId` | created with a 26-character minted id |
| R13 | B72 saves: newer, stale, transmutation | as §7.5 |
| R14 | B72 a stackable sent instanced and a non-stackable sent as `quantity` 2 | both corrected; two deterministic 26-character upper-case hex ids; the same request with another transaction id gives other ids |
| R15 | B72 a non-stackable sent stacked with quantity 0, 1001, 1.5 or `"x"` | 0 and 1001 pass validation and fail normalization: 500 page; 1.5 and `"x"` fail validation: 400 invalid quantity |
| R16 | B72 starter dye ids in both add lists | not stored, not reported |
| R17 | B72 player bearer granting; player bearer saving | 403 authority; 200 |
| R18 | B72 game server with `accountId` in the body; with empty `characterId`; bearer-less with empty `accountId` and `characterId` | the named user; the first character; the configured account's first character |
| R19 | B72 user `INVALID` with odd collections (`removeInstancedItems` a string) | 200 echo, the string echoed |
| R20 | B72 an array body; malformed JSON; no body (game server, character resolved); bad quantity; a save entry without `instanceId`; a form-encoded body | 400 body must be an object; 400 page; 400 `transactionId must be a non-empty string`; 400 messages of §4.6; form: validated as an object |
| R21 | B72 no policy file | 500 page |
| R22 | B73 newer version; stale version; no version; unknown instance; transmutation; another user's character | the new item; the stored item; the stored item when the sent data equals it; empty JSON (200) for each of the last three |
| R23 | B73 user `INVALID`; an array body; missing `instanceId`; `updateVersion` 1e10 | echo of present keys; 400s of §4.6 |
| R24 | B130 as a player | 403 Forbidden |
| R25 | B130 on `ExperienceTrack_PlayerLevel`, on `INVALID`, below the cap, for an unknown user | 409 Conflict each |
| R26 | B130 at the cap + 3500 | 200, progress 0, ranks 0, `confirmed_date` = the reset time; a second DELETE within 2 minutes: 200 unchanged, no second record |
| R27 | B130 three minutes after an unfinished reset | 409; the banked XP and the PrestigeTrack +1 applied once |
| R28 | B131 before any reset; wrong amount; as a player; amounts `0`, `1.5`, `100001`; `INVALID`; a non-weapon track | 409; 409; 403; 400 each; 400; 409 |
| R29 | B131 with the banked amount after a reset | 200 `replayed:false`, progress = banked, `grant_id` `reforge-<id>-banked`; again: 200 `replayed:true`, applied once |
| R30 | `RUSTIC_ISLES_REFORGE=off` and `log` | B130 409; B131 409 |
| R31 | No store | B70, B72, B73, B130, B131 answer the 501 envelope with the no-database message after their earlier checks |

### 13.3 Host runtime cases (a recording guard)

| # | Case | Expect |
| --- | --- | --- |
| H1 | rifts dedicated-only (no module): B72, B73, B130, B131 from a runtime | 403 with rifts-seam's `forbidden` bodies (`This runtime cannot mutate inventory.`, `This runtime cannot write player state.`); B73 with no user: `Player-host runtime cannot mutate inventory.` |
| H2 | An authorizing guard, B72 with a `source` | guard sees `authorize_inventory <user> <character> <tx>`, then `Applying`, then the grant (the body's `JSON.stringify` text, `source` included), then `Accepted`; 200 |
| H3 | The fence refuses the grant with a 403 body | that status and body; the inventory unchanged; no ledger row |
| H4 | B73 from a runtime | `authorize_host_write ItemSave <user>`, then the tag `{"instanceId":..,"catalogId":..,"updateVersion":..,"itemDataBytes":..}`; a refused target answers the guard's reply |
| H5 | B130, B130 again, B131 from a runtime | authorized each time; one `delete` tag (with `reforgeId`, `priorProgress`, `cap`, `banked`), none for the replay, one `purchase` tag |

### 13.4 Unit cases

| Area | Cases |
| --- | --- |
| Hash | key order and missing-as-empty give the same 64-character hash; five empty collections hash the canonical text of §7.1; stable stringify sorts nested keys and keeps non-ASCII as itself |
| Mint | 26 characters of the alphabet; two mints differ; the same replay seed gives the same sequence |
| Placeholder | id equal to `catalogId`, missing, empty → yes; a real id, a non-object → no |
| Items | §7.4 and §7.5's tables |
| Policy | §8.4's tables; storage of an unknown id and of a non-string id is none; ownership unlock for max 1 or a listed tag, not for max 99 Material, not for an instanced entry; the loader drops a BOM and reports a missing file with the hint |
| Normalization | §8.3 both ways; deterministic ids repeat for the same transaction id and differ between ordinals |
| Validation | §4.6 row by row; `null` reads as absent; a 257-unit id fails |
| Weapon level | a levelled `WP_` item served at 0 with the other keys and their order kept; level 0, no level, a non-weapon, unparsable data, a non-string catalog id unchanged; nested in a response; depth 4 served, depth 5 not; hook off unchanged |
| Currency | user `INVALID` touches nothing; Aetherspark 400 converts 2 Aetherhearts without a prestige claim, a third needs banked prestige; a gated reward with no banked levels is 409 prestige; operator funding skips the gate; a funded season coin skips the gate; a daily-limited coin is dropped and the rest applies; a platinum removal reports each bucket then the aggregate |
| Prestige | sources from a config with a capped weapon track and a prestige track paying `CURRENCY_PJM_PRESTIGE_FILLED` 1 a level; `CURRENCY_PJM_PRESTIGE_EMPTY` is not gated |
| Reforge plan | §5.3.1's rows, including a retry after the restore (`Normal` with the grant id) |
| Multipliers | §10.2's table; a non-reward source, a cosmetic, a fractional or zero quantity, an excluded id keep their quantity |
| Store | an inventory is written by revision (a stale revision writes nothing) and logged; the ledger's `try_begin` replays and lists (store spec §15) |

---

## 14. Behaviour that looks wrong but is kept (parity), and open questions

1. **Game auth only on wire credentials** (§3.1 step 2): a request carrying only `x-reforged-session-ticket` (DN5) or
   only a host-capability runtime credential is not authenticated here and answers 401, unless the composer or the
   pipeline already put auth data in the extensions. Other areas always run game auth. Open: is this intended for the
   inventory routes, or should the full chain run (a behaviour change, a decision row)?
2. **Runtime type gap.** The guard calls take `rifts_seam::RuntimeAuth` (capability, session, generation, ...), but the
   MIT auth crate's `AuthData.runtime` is a `RuntimeCaller` holding only a roster. Open: the auth crate carries the
   `rifts_seam::RuntimeAuth` (a dependency it avoided, auth §11.1), or the composer inserts it into the extensions
   beside `AuthData` and the authenticator port reads both. Proposed: the latter, no auth change.
3. **Reads write.** B70, B71 and §6.3 run the starter bootstrap, the storage repair and the starter wallet inside an
   account write; a B71 or B72 game-server call for an unknown user creates the account and its first character.
   Kept.
4. **The reforge id is system-random**, not from the replay id source; a replayed reset gets another id (visible only
   in host tags and stored rows). Kept, as store spec §16 item 11's ids.
5. **The season-claim wait uses the wall clock**, not the replay clock, and blocks the request up to 1.5 s. Kept;
   a replay with a pinned time may judge the claim differently.
6. **Process-wide caches.** Restricted keeps the storage policy and the prestige sources in process globals keyed by
   path, never refreshed. Proposed: per module (as core §17 item 14), loaded on first use, never refreshed; failed
   loads not kept.
7. **Empty `requestId`** in the reforge routes' account-lookup 501 envelope. Kept.
8. **Forwarding a foreign player's grant** drops removals, saves and item data, answers as if applied, and is not
   idempotent here (the home's ring call decides). Kept.
9. **The fence is not told `Failed`** when the transaction ends in an error (only on not owned). Kept.
10. **A guard refusal is always 400** here, while core's rifts relay uses `status_for(code)`. Kept.
11. **The hash covers the multiplied adds** (§6.2). Kept; it is what makes a retry with the same multiplier replay.
12. **A single-item save without `itemData` at the stored version writes `null`** over stored data (§7.6). Kept.
13. **Duplicate stacks** in a stored blob: adds go to the last, removals take the first; a stacked add of quantity 0
    creates a zero stack that survives until a call with removals. Kept.
14. **A zero currency add creates the starter wallet** and is reported (§9.1). Kept.
15. **No `progression_config.json` ungates prestige rewards** (§9.2): a module without the file lets a game server
    credit Aetherhearts freely. Kept for parity; worth a decision row (fail closed).
16. **`StaleFinished` answers 409** although it repaired the track: the game sees a failed reset. Kept.
17. **Two serializations of one body**: the guard's `authorize_inventory` gets compact `serde_json` text, the fence's
    grant gets `JSON.stringify` text (they differ only in number and escape forms). Kept; the implementer may use
    `JSON.stringify` for both if the guard spec agrees.
18. **Grant amplification**: one gameserver body under 100 KiB may expand to about two million instanced items
    (§8.3, 1000 per entry). Only dedicated-server or fenced authority reaches it. Listed for the security page;
    proposed bound (a decision row): a total cap on converted instances per transaction.
19. **NotHome is a 500 page** here (no DN5 forwarding for inventory writes). Kept.
20. **`log` reforge mode refuses** exactly like `off` on B130 and B131 (it only adds the log line). Kept.
21. **Which part owns `ApplyCurrencyDeltas`**: this spec puts it, the prestige gate and the Sparkforge conversion in
    inventory (restricted's `inventory/ports.rs`), over bounties' wallet primitives W1 to W8. The bounties part must
    not specify it again. To confirm with that part's author.

---

## 15. Security behaviour (for `docs/security/rustic-isles-economy.md`, inventory section; RX3)

- **Trust boundary.** The routes are reached only through the gateway; the module binds 127.0.0.1 (core §14).
  Authority is the auth crate's: a game server key (HMAC-checked), a player token, or a runtime the realms seam
  vouches for. A player may only save already-owned instances; grants and spends need a game server or a fenced
  runtime (§4.4 step 8). Reforge routes refuse players outright.
- **Untrusted input**: the whole `POST /inventory` and `POST /inventory/instanceditem` body, the path params, the
  `source` string. Validation (§4.6) bounds ids (256 units), entries (4096 a list), quantities and versions (1e9),
  item data (262 144 units) before anything is stored; the body limit is 100 KiB.
- **Idempotency**: every transaction is bound to its body by a SHA-256 over a canonical form; a reused id with another
  body is refused, a concurrent duplicate is refused while pending, and a failure leaves no ledger row.
- **Atomicity**: ledger, wallet, prestige claims, challenge credit, grant hook and the revision-guarded inventory
  write commit together or not at all; the reforge marks are compare-and-set inside the grant's transaction.
- **No minting**: currency is the wallet's (never stored in the blob), debits never go below zero, prestige-gated
  rewards need banked levels (except §14 item 15), a season coin needs a claimed challenge, catalog ids that become
  wallet keys are strict identifiers.
- **Secrets**: none handled here; logs carry user ids, character ids, transaction ids and catalog ids, never a token
  or key. Log lines truncate nothing user-supplied except the `source` (80 characters); ids are bounded by validation
  before most log lines (the pre-validation log lines print the user id).
- **Known weak spots**: §14 items 1, 3, 15, 18; the replay headers (core §17 item 1, now gated) would pin the clock of
  the reforge stale window and the challenge rules.

**Fuzz target** (`fuzz/fuzz_targets/rustic_isles_economy_inventory.rs`): arbitrary bytes as a JSON body through
`validate_inventory_transaction_body` and `validate_instanced_item_update_body`; for a body that validates, the
request hash, the grant normalization (with a fixed small policy) and `apply_transaction` (stored arrays also
arbitrary JSON) never panic; a hash never changes when object keys are permuted; normalization never yields more
than 1000 instances per entry and never an instance without a 26-character id; `apply_transaction` never stores a
`CURRENCY_` row and never leaves a stored stack at or below 0 after a call with removals; `serve_weapons_at_base_level`
and `weapon_item_data_at_base_level` never panic on arbitrary JSON and never change anything but `CurrentLevel`;
`read_test_multipliers` and `is_tailscale_host` never panic on arbitrary text.

---

## 16. Settings read by this part

| Setting | Reading | Where |
| --- | --- | --- |
| `RUSTIC_ISLES_REFORGE` (switch `reforge`, core §6.2) | §5.1 | reforge |
| `KEEP_WEAPON_ITEM_LEVELS` | through the version's `weapons_at_base_level` hook (§7.7) | serving |
| `REFORGED_XP_MULTIPLIER`, `REFORGED_REWARD_MULTIPLIER`, `REFORGED_SERVER_MODE`, `REFORGED_SERVER_HOST` | §10.1 (written by the host kit for a private server) | multiplier |
| `GAMESERVER_NO_PLAYER_USER_ID`, `DEV_USER_ID` | core §5.3 through the no-player policy | user resolution |
| `REFORGED_DB_FILE`, `REFORGED_DB_ENGINE` | whether a store exists (core §5.1) | §3.2 |

Game data read: `inventory_storage_policy.json` (§8.1), `progression_config.json` (§9.3, and progression's curves for
the cap). Tables read: `seasonal_coin`, `game_version`, `starter_manifest` (through the characters crate). Hooks:
`no_player_policy`, `weapons_at_base_level`, `reforge_rules`. The settings `Debug` rule is core's (names only).

---

## Coordinator decisions (2026-11-05, before implementation)

These settle §14 and override anything above that disagrees.

- **Item 1:** parity (only an `authorization` header or a game-server key runs game auth).
- **Item 2: one runtime mechanism for every area.** Not the request-extension route: `rustic-isles-auth`'s
  `RuntimeCaller` carries an opaque handle (`Option<Arc<dyn Any + Send + Sync>>`, added by the characters unit, see
  spec-rustic-isles-characters.md's decisions) that the realms seam fills; the area downcasts it to
  `rifts_seam::RuntimeAuth` for the guard calls.
- **Item 3:** per-module caches, as proposed.
- **Items 4-5:** parity; candidate decision rows for the maintainer (refuse prestige-gated currency without
  `progression_config.json`; a total grant cap per transaction).
- **Items 6, 8, 9:** parity.
- **Item 7:** `ApplyCurrencyDeltas`, the prestige gate and the Sparkforge conversion belong to this part, on the
  bounties part's wallet primitives (W1-W8); the bounties spec is reconciled to match.
- **Security page:** one page for the crate, `docs/security/rustic-isles-economy.md`, with a section per part.
- **Amended with the progression spec:** the shared multiplier reader (§10.1) reads text as JavaScript `Number` does
  (parity), not digits only; §12.3's I5 is also called by progression's session-supplies refill.
