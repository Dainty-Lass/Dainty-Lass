# Clean-Room Behaviour Specification: `rustic-isles-economy`, part 3 of 3: bounties and the store (RD27)

> **Scope and purpose (RD27):** unit 7 of [`spec-rustic-isles-core.md`](spec-rustic-isles-core.md) §16 is one MIT
> crate, `rustic-isles-economy`, written from three specs because inventory, progression and bounties-store call each
> other in a cycle. This is the third: bounties, cooldowns, entitlements, the wallet and Platinum, the store (Lady
> Luck's store, the Reward Cache, the Platinum store's four sections, the Middleman's slots, the fountain's daily
> bundle), store purchase tokens and the economy ledger monitor. The other two are
> `spec-rustic-isles-economy-inventory.md` and `spec-rustic-isles-economy-progression.md`; §6 here is where the three
> meet. The source is dainty-restricted's `modules/rustic-isles/crates/isles-core/src/areas/bounties_store.rs` and
> `areas/bounties_store/*`, AGPL-3.0-only with Mystic Paradox's section 7 terms; an implementer who never sees it
> writes the MIT part from this document alone.
>
> Everything below is stated from observable contracts: routes and their bytes, the rules, the store calls, the
> settings, the file formats, the log lines a host greps and the tests. No implementation code is reproduced and no
> private helper is named. Node names (`MergeBountyState`, `PurchaseStoreSku`) appear only as provenance. Wire names
> the 1.14.7 client or a game server depends on (paths, JSON keys, currency and catalog ids, section tags, the
> `INVALID` sentinel) are interface facts and are kept.
>
> Written against dainty-restricted `e0e238b4434316bb210677d88bc59e3448688794` (worktree
> `splashloader-integration-90130d`; the files of this unit last changed in `a8681fe`) and dainty-lass
> `731f928af2c098c1c4d61e1448c0b80ce69609ef` (branch `feat/isles-auth-clean-room`).

---

## 1. Provenance and licensing classification

### 1.1 Evidence used

| Source | What it says |
| --- | --- |
| File headers (`//!`) | Each file ports one Paradox TypeScript file: `routes/system.ts`, `routes/store.ts`, `controllers/{cooldowns,entitlements,challengeRewards,store,wallet}.ts`, `bountyState.ts`, `cooldownState.ts`, `platinumWallet.ts`, `platinumStoreFree.ts`, `diagnostics/{economyMonitor,economyLedgerMonitor}.ts` |
| `areas/inventory/ports.rs`, `areas/inventory/transaction.rs` | How inventory calls the wallet, the challenge-reward rules and the economy ledger, and how it runs a store purchase (read for §6 only) |
| `areas/auth/tokens.rs` | The store token issuer behind the purchase seam (now `rustic-isles-auth` §5.1) |
| `isles-1-14-7/src/lib.rs` | The 1.14.7 tables this part reads (`store_sections`, `seasonal_coin`, switch defaults) and the `entitlements_payload` hook |
| `areas/matchmaking.rs`, `areas/reforged_api/activity.rs`, `module.rs` | The calls this part makes to later units (foreign accounts, ring forwarding, claim activity) and the home operations that read the same rows (read for §6.4 only) |
| `tests/golden/bounties_store/1.14.7.json` (80 cases), `isles-1-14-7/tests/bounties_store.rs`, `*_tests.rs` | Which behaviour is fixed (read for §12's coverage; nothing is copied) |

### 1.2 Classification and what is rewritten

Every file is Category B (a Paradox port) except `harness.rs` (Category A test support). The whole part is rewritten.

| Restricted file (`areas/bounties_store/`) | Source | Rewritten as |
| --- | --- | --- |
| `../bounties_store.rs` | RU5 group root | §10.3 registration |
| `handlers.rs` | `routes/system.ts`, `routes/store.ts`, `controllers/cooldowns.ts` | §3 |
| `authn.rs` | `req.AuthData`, the store seams | §3.1, §6 |
| `bounty_state.rs` | `bountyState.ts` | §4.1 to §4.5 |
| `cooldown_state.rs` | `cooldownState.ts` | §4.6 |
| `challenge_rewards.rs` | `controllers/challengeRewards.ts` | §4.5, §6.2 |
| `entitlements.rs` | `controllers/entitlements.ts` | §4.7 |
| `wallet.rs`, `platinum.rs` | `controllers/wallet.ts`, `platinumWallet.ts`, `platinumStoreFree.ts` | §4.8 |
| `store.rs` | `controllers/store.ts` | §4.9 to §4.13 |
| `economy_ledger.rs` | `diagnostics/economyLedgerMonitor.ts` | §4.14 |
| `monitor.rs` | `diagnostics/economyMonitor.ts` | §9 |
| `util.rs` | JavaScript value habits | §2 |
| `harness.rs` | RU5 test support (A) | Not rewritten; §12.3 describes what the crate's own test support needs |

### 1.3 What stays in dainty-restricted

The golden file and its recorder (`tests/golden/bounties_store/`), the version crate's tests, the home operations
`bounty.read`/`bounty.write` in `module.rs` (core §7.8, the Reforged unit's), and every captured or generated store
file (§8). The generators that build the store files are `rustic-isles-gamedata` §5.4 to §5.7.

---

## 2. Conventions

- **Text, JSON, times**: as core §2.1 and auth §2.1, §2.2. JSON is written as `JSON.stringify` writes it
  (`service_kit::json::Json`: insertion order, then integer-like keys first in ascending order, as JavaScript objects
  do). "Empty" is a status with no body. "500 page" is `service_kit::body::error_page(500)` (Node threw). "501" is
  core's `not_implemented(route, request_id)` envelope.
- **The clock** is core's replay-aware `now_ms()` (core §9.1) unless a rule says "system clock". "Now ISO" is
  `to_iso_string(now_ms())`. Parsing a date is `service_kit::js::date_parse`; NaN compares false against everything.
- **JavaScript numbers**: `Number(x)` is `service_kit::js::number`; "`Number(x) || 0`" is that with NaN and 0 giving 0;
  `String(x)` is `service_kit::js::string` (absent is `undefined`). A number written back to JSON is written as an
  integer when it is a whole safe integer, else as the float (so `5000.0` is written `5000`).
- **Lengths** marked "UTF-16 units" are JavaScript `.length`; "bytes" are UTF-8.
- **The no-player sentinel** is the literal path text `INVALID` (case-sensitive). Route params are bound before game
  auth rewrites the path (auth §4.1.1), so a handler sees `INVALID` in its params even when the URI was rewritten.
- **SHA-256 hex** is lower-case hex of the UTF-8 text; "the first N, upper-cased" takes N hex digits and upper-cases.
- **The envelope** of the state routes is `{"code":null,"message":<text>,"payload":<value>}`.
- **The store error body** is `{"code":"<status as text>","message":<reason>}`.

---

## 3. Routes

Seventeen handlers on the backend (core Appendix A rows B38, B39, B53 to B61, B74 to B79). Paths exactly as the route
table has them; matching is core §4.1 (Express 5: case-insensitive literals, one optional trailing slash).

### 3.1 Every route, before its rules

In this order:

1. **The batch guard** (only `PUT`/`POST /cooldown/:userId/:cooldownId`): `userId` exactly `batch` answers empty 404
   before anything else (no body read, no auth). `PUT /cooldown/batch/<x>` never gets here (row B54 matches first), so
   this is `POST /cooldown/batch/<x>` (§14 item 12).
2. **The body**, read by core's `read_body` rules (core §3.9): a malformed JSON body is the 400 page, an oversize one
   413, a bad charset or encoding 415, before auth. Every route reads it, even those that ignore it.
3. **Game auth** (`rustic_isles_auth::Auth::game_auth`, auth §4.1): its refusal (empty 401 or 403, 500 page, a realm
   refusal) is the answer. The caller then has `user_id` (optional), `is_gameserver`, and a host runtime when the realms
   seam authenticated one (its roster and its `rifts_seam::RuntimeAuth`, §14 item 1).
4. The route's rules below. Without a database (no store slot, §10.3), a route that needs one answers 501.

**Caller kinds** used below: a **game server** (`is_gameserver`); a **host runtime** (a runtime is present and the
caller is not a game server); a **player** (neither). The **monitor source** is `gs`, `host` or `player` in that order
of precedence. **The actor** of a write is `System` for a game server or a host runtime, else `Player { account }`
(the target account's global id), with the request id attached.

**Persistence switches**: `bounty persistence` and `cooldown persistence` are on unless the raw setting
`RUSTIC_ISLES_BOUNTY_PERSISTENCE` (resp. `RUSTIC_ISLES_COOLDOWN_PERSISTENCE`), or when unset the version's switch
default (1.14.7: `on`), is `0`, `false`, `off` or `no` (ASCII case-insensitive, **not trimmed**; unset everywhere is
on).

**Account lookup**: the target's account is `accounts.find_by_user_id(<userId text>)` (store §6.1); a store failure is
the 500 page (logged).

### 3.2 `GET /entitlementsv2` (B38)

Caller's `user_id`, absent read as `INVALID`. Empty, `INVALID`, or no database: the list is empty. Otherwise the
account (none: empty list) and its entitlements (§4.7, sorted by name). 200 with the version's
`entitlements_payload(list)`; 1.14.7's is `{"entitlements":[{"activatedDate":<iso or null>,"duration":<number>,"name":<text>},...]}`.
A store failure: 500 page.

### 3.3 `POST /entitlementv2/:userId` (B39)

A stub: 200 `{"code":null,"message":"OK","payload":[]}`. Nothing is read or written.

### 3.4 `GET /cooldown/:userId` (B53)

1. The account is looked up only when cooldown persistence is on, `userId` is not `INVALID` and there is a database,
   or when the caller is a host runtime (then with a database only; without one it stays none).
2. **Reader check** (§4.1): refused is empty 403, logged `Rejected cross-account cooldown read actor={user or unknown} target={userId}`.
3. Persistence off or `INVALID`: 200 envelope `OK` with payload `{}`.
4. No database: 501. No account: payload `{}`. Else the stored state's payload (§4.6.5).

### 3.5 `PUT /cooldown/batch/:userId` (B54)

1. **Writer check** (§4.1): a game server writes as the dedicated server; a host runtime writes under the host rules;
   anyone else empty 403 (logged `Rejected cooldown write without gameserver authority actor=.. target=..`).
2. Persistence off or `INVALID`: 200 envelope `OK` payload `{}`, nothing stored.
3. No database: 501. No account: 200 envelope `OK` payload `{}`, nothing stored.
4. Host runtime: the host-write check (§6.5); a refusal is answered with the reply's status and JSON body.
5. The save (§4.6.2, or §4.6.3 for a host), compare-and-retry on the stored version up to 8 times inside one account
   write. Success: 200 envelope `OK` with the new state's payload; a host write is then recorded (§6.5, route
   `batch`). A validation error: 400 `{"code":null,"message":"Bad Request","payload":{}}`. Still contended after 8:
   data error `Cooldown save remained contended for {account}`, 500 page. A store failure (including the home check):
   500 page.

### 3.6 `PUT` and `POST /cooldown/:userId/:cooldownId` (B55, B56)

`StartCooldownEndpoint`: one cooldown started at the backend's own now. A 2xx releases a harvest reward in the game, so
it never answers 2xx without storing. Any body is ignored (a caller cannot backdate). In order, each refusal an
envelope with payload `{}` unless said:

| Condition | Answer |
| --- | --- |
| §3.1 step 1 | empty 404 |
| Not a cooldown writer | empty 403 |
| `cooldownId` not `^[A-Za-z0-9_.:-]{1,200}$` | 400 `Bad Request` |
| `userId` is `INVALID` | 400 `Bad Request` |
| Cooldown persistence off | 503 `Service Unavailable` |
| No database | 501 |
| No account | 404 `Not Found` |
| Host runtime refused by the host-write check | the reply |
| Saved | 200 `OK` with the state's payload |
| A validation error from the merge | 400 `Bad Request` |
| Store failure | 500 page |

The save is the batch save of one entry `{cooldown_id, cooldown_started_date: <now ISO>}` under the same rules (host
rules for a host runtime, so a host cannot move a stored date earlier). A host write is recorded with route `start`,
`id` and `startedAt` (§6.5).

### 3.7 `GET /bounty/game-data` (B57)

A stub (any authenticated caller): 200 envelope `OK` with this payload, in this order:

`{"max_slots":4,"num_draft_options":3,"num_spicy_options":1,"bounty_token_id":"TOKEN_BOUNTY_DRAFT","premium_bounty_token_id":"TOKEN_BOUNTY_DRAFT_PREMIUM","num_tokens_hp_start":4,"num_tokens_per_day":0,"bounty_token_grant_hour":0,"history_length":10,"bronze_count":9,"silver_count":3,"gold_count":1,"new_season_reset_bounties":false,"bounty_data":[],"item_grant_data":[],"token_rollover_warning_days":1000,"automatic_draft":false,"automatic_claim":false,"delete_claimed_bounties":false}`

### 3.8 `GET /bounty/:userId` (B58)

As §3.4 with bounty persistence and the bounty reader check (403 logged `Rejected cross-account bounty read ...`).
Persistence off or `INVALID`: 200 envelope `OK` with the **empty state payload** (§4.2.6). No database: 501. No
account: the empty state payload. Else the stored state's payload.

### 3.9 `POST /bounty/:userId` (B59)

1. Writer check (§4.1): game server, or host runtime under host rules; else empty 403 (`Rejected bounty write without
   gameserver authority ...`).
2. Persistence off or `INVALID`: 200 envelope `OK` with the empty state payload, nothing stored.
3. No database: 501.
4. **A foreign account** (game server only, not a host runtime): when the path `userId` resolves to an account whose
   home is another node (§6.4 `foreign_account`), the save is forwarded to that home through the ring
   (`HomeApi::ring_forward_hunt_rewards`, §6.4) and the answer is 200 envelope `OK` with the empty state payload; a
   forward failure is the 500 page. Nothing is stored here.
5. No account: 200 envelope `OK` with the empty state payload, nothing stored.
6. Host runtime: the host-write check (§6.5).
7. The save (§4.2, or §4.3 for a host), compare-and-retry up to 8 times in one account write. Success: 200 envelope
   `OK` with the **full merged state's payload** (as the live service answered a save); the count of challenges this
   save newly claimed (§4.2.7) goes to the claim activity (§6.4), and a host write is recorded with the summary of
   §4.3.4. A validation error: 400 `{"code":null,"message":"Bad Request","payload":{}}`. Still contended: data error
   `Bounty state save remained contended for {account}`, 500 page. Store failure: 500 page.

### 3.10 `POST` and `DELETE /bounty/delete/:userId` (B60, B61)

`HandleBountyDelete`. Claiming a completed bounty deletes it first; the game grants the reward only after a 200.

1. Writer check as §3.9 (logged `Rejected bounty delete without gameserver authority ...`).
2. The account is looked up when the caller is a host runtime, or when bounty persistence is on and `userId` is not
   `INVALID` (no database: 501).
3. Host runtime: no account is empty 403; then the host-write check.
4. **Names nothing** (no body, a JSON `null`, or an object with none of `bounty_ids`, `bountyIds`, `bounties`,
   `delete_claimed_bounties`): 200 envelope `OK` payload `{}`, nothing changed (the game's ResetBounties).
5. Persistence off or `INVALID`: 200 envelope `OK` with the empty state payload.
6. No account: the same.
7. The delete (§4.4), compare-and-retry up to 8 times. Success: 200 envelope `OK` with the new state's payload. A
   validation error: 400 `Bad Request` envelope with payload `{}`. Contended: `Bounty delete remained contended for
   {account}`, 500 page.

### 3.11 `POST /reconcile` (B74)

The caller's wallet (§3.12's rule). 200 `{"balances":<projection of the wallet over an empty base>,"refreshInventory":true}`
(§4.8.4).

### 3.12 `GET /balance` (B76)

**The caller's wallet**: `user_id` absent, empty or `INVALID`: an empty wallet (no database needed). Else no
database: 501; no account: empty wallet; else the stored balances, or, when none is stored, the wallet created now
with the starter grant (§4.8.1) in one account write. A store failure (home check included) is the 500 page.

The answer is 200 with the **balance base** (below) laid under the wallet's projection (§4.8.4). The base, in this
order, every value 0 except: `id_currency_weapon_token` and `CURRENCY_WEAPON_TOKEN` 25, and `CURRENCY_NOTES` and
`id_currency_notes`, which are the wallet's `CURRENCY_NOTES` or 0:

`id_currency_s20_coin`, `CURRENCY_GAUNTLET_COIN_FADED`, `CURRENCY_S20_COIN`, `CURRENCY_S18_COIN`,
`id_currency_seasonal_coin`, `id_currency_s18_coin`, `id_currency_weapon_token`, `id_currency_celldust`,
`id_currency_event_ramsgiving`, `CURRENCY_NOTES`, `id_currency_event_frostfall`, `CURRENCY_EVENT_DARKHARVEST`,
`CURRENCY_S19_COIN`, `id_currency_s16_coin`, `CURRENCY_S16_COIN`, `id_currency_gauntlet_coin`, `id_currency_s13_coin`,
`CURRENCY_MARKS_STEEL`, `CURRENCY_S13_COIN`, `CURRENCY_EVENT_FROSTFALL`, `CURRENCY_GAUNTLET_COIN`,
`id_currency_marks_steel`, `id_currency_rewardcache`, `CURRENCY_PRESTIGE`, `CURRENCY_SEASONAL_COIN`,
`CURRENCY_REWARDCACHE`, `id_currency_token_exchange_speed_up`, `id_currency_event_springtide`,
`CURRENCY_TOKEN_EXCHANGE_SPEED_UP`, `id_currency_gauntlet_coin_faded`, `CURRENCY_S15_COIN`, `CURRENCY_PLATINUM`,
`id_currency_platinum`, `id_currency_s15_coin`, `id_currency_marks_gilded`, `id_currency_event_darkharvest`,
`id_currency_event_saintsbond`, `CURRENCY_EVENT_SPRINGTIDE`, `id_currency_s19_coin`, `id_currency_notes`,
`id_currency_prestige`, `id_currency_s13_daily`, `CURRENCY_WEAPON_TOKEN`, `CURRENCY_MARKS_GILDED`,
`CURRENCY_S13_DAILY`, `CURRENCY_CELLDUST`, `CURRENCY_S14_COIN`, `CURRENCY_EVENT_SAINTSBOND`, `CURRENCY_S17_COIN`,
`id_currency_s14_coin`, `CURRENCY_EVENT_RAMSGIVING`, `id_currency_s17_coin` (52 keys).

Logged `Fetched balance for userId {user}: Rams(NOTES)={n} CombatMerit(PJM_WEAPON)={n}`.

### 3.13 `GET /creator` (B75)

A stub: 200 `{"expirationDate":"2099-01-01T01:00:00.041Z","slug":"MROWMROW","success":true}`.

### 3.14 `GET /product/skus/public?requiredTags=<tag>` (B77)

**The query** is read by splitting on `&`, each pair at its first `=`, `+` as space, then `decodeURIComponent` (the
raw text when that fails); a name is matched exactly. The value must occur once and be non-empty, else 400
`{"code":"400","message":"missing requiredTags query parameter"}` (absent, empty, or repeated).

The section's SKUs (§4.10) for the player (§4.13.2 for a daily section). None: 200 `[]` (logged `Store SKUs requested
for unimplemented tag '{tag}' - returning empty` unless the tag is a daily one). Else 200, an array of each SKU's
listing (§4.11). A store failure during a daily check's account lookup: 500 page.

### 3.15 `GET /token/:currency/:catalogId` (B78)

Mints a purchase token. The `currency` segment is client routing only: the price and currency charged come from the
SKU when the token is redeemed.

1. Find the SKU by `catalogId` (§4.10 lookup); none or ambiguous: 404 `{"code":"404","message":"unknown_sku"}`
   (logged `[Store] GetPurchaseToken: unknown or ambiguous sku/catalogId '{id}' (currency={c}) for {actor}`).
2. No database: 501.
3. The character: with a non-empty `user_id`, its account (none: no character), then the account's first character
   (§6.4 `Characters::first_character_id`, which may create one). No character: 500 `{"code":"500","message":"no_character"}`. A
   lookup failure: 500 page.
4. Sign the token (auth §5.1, `Auth::sign_store_token`) with `userId` (the caller's or empty), `characterId`,
   `skuId` (the SKU's own id, even when found by an item catalog id) and `storeTag` (the section the lookup found). 200
   `{"purchaseToken":<token>}`; a signing failure is the 500 page.

### 3.16 `POST /notification/:currency?token=<token>` (B79)

Redeems a purchase token. Success is **204 with no body**.

| Step | Refusal |
| --- | --- |
| `token` query (rules of §3.14) once and non-empty | 400 `{"code":"400","message":"missing token"}` |
| A database | 501 |
| The token validates (auth §5.1 `validate_store_token`: signature, issuer, audience, expiry on the system clock) | 401 `{"code":"401","message":"invalid_token"}` |
| The token's `userId` equals the caller's `user_id` (absent never equals) | 403 `{"code":"403","message":"token_user_mismatch"}` |
| The section: the token's `storeTag`, else (a token without one) the lookup of its `skuId` (§4.10) | 404 `unknown_sku` |
| The purchase (§4.12) | its reason (below) |

| Reason | Status | Message |
| --- | ---: | --- |
| The SKU is not in that section now | 404 | `unknown_sku` |
| Not enough currency | 402 | `insufficient_balance` |
| The ledger row is in progress, or a revision conflict | 409 | `conflict` |
| The transaction id was used with another body, or the daily claim is held by another character | 409 | `already_purchased_different_request` |
| Anything else (invalid price, no account, not owned, a refused grant, a store failure) | 500 | `transaction_failed` |

Each refusal body is `{"code":"<status>","message":<reason>}`. Logged `[Store:{tag}] purchase completed for {actor}
sku={sku}` or `[Store:{tag}] purchase denied for {actor} sku={sku}: {reason}`.

---

## 4. Rules

### 4.1 Who may read and write

| State | Read | Write |
| --- | --- | --- |
| Bounties, cooldowns | A game server: any target. A host runtime: a target whose `userId` text or account global id text is in its roster (an empty roster or no account match refuses). A player: only when the caller's `user_id` equals the path `userId` exactly | A game server (the dedicated server's rules). A host runtime (the host rules, after the host-write check). A player: never (a player could unclaim or re-date a challenge and collect it again) |
| Wallet, entitlements, store | The caller's own `user_id` | Through the caller's own reads (wallet creation) and purchases only |

### 4.2 Bounty state

#### 4.2.1 The stored record

Store §11.1 `BountyStateRecord`: an ordered list of entries `{group, bounty, claimedAt?, rewardCredit?}`, the draft
data per group (`base`, `daily`, `weekly`, each optional), `daily_reward_credited_on`, `daily_premium_draft_granted_on`,
`update_version`, `created_at`, `updated_at`. `bounty` is the game server's object as it sent it (numbers normalized as
`JSON.parse` would give them, integer-like keys first).

**Groups**: `base`, `daily`, `weekly`, `unassigned` (`unassigned` only in imported rows). **Inferred group** of a
bounty id: starts `Challenge_Daily_` is daily; contains `Challenge_Season` is weekly; else base. An entry's
**effective group** is its stored group, or the inferred one when stored as `unassigned`.

#### 4.2.2 Validation of a save (`ValidateBountySave`)

The first failure refuses the save (400), nothing stored. Texts are logged only:

| Rule | Error |
| --- | --- |
| The body is an object | `body must be an object` |
| `bounties` absent or `null` (an empty list), else an array | `bounties must be an array` |
| At most 512 bounties | `too many bounties ({n})` |
| Each is an object | `every bounty must be an object` |
| Each has `bounty_id`, a non-empty string of at most 200 UTF-16 units | `every bounty needs a bounty_id string` |
| `objectives`, when the key is present (even `null`), is an array | `objectives of {id} must be an array` |
| Each draft key present (`draft_data`, `draft_data_daily`, `draft_data_weekly`; even `null`) is an object | `{key} must be an object` |

The draft keys present decide the group: `draft_data` base, `draft_data_daily` daily, `draft_data_weekly` weekly.

#### 4.2.3 Merging a save (`MergeBountyState`)

Starting from the stored list and draft data (empty when none):

- **Exactly one draft key**: that group's draft data is replaced by the body's value. When the save carries at least
  one bounty and the group is daily or weekly, it is a **replace**: every stored entry of that effective group whose id
  the save does not carry is removed (the base group is never replaced: drafting a bounty token saves only the new
  bounty, and several tokens can be active). Each bounty is then upserted **filed under the key's group**. The mode is
  `replace` for daily and weekly, `upsert` for base.
- **No draft key or several**: each present draft key replaces its group's data; each bounty is upserted filed under
  its inferred group; the mode is `upsert`.

**Upsert** of one bounty: the first stored entry with the same `bounty_id` (strict equality). None: appended. Found:
when it is the **same instance** (same `bounty_id`, and the same `drafted_timestamp`, absent and `null` alike, compared
strictly) the two are merged (below), else the incoming replaces it; the entry keeps its stored group unless that was
`unassigned` (then the new filing), and gets claim metadata (§4.2.5).

**Merging one instance** (`MergeBountyInstance`): the incoming object, with:

- `objectives`: the incoming objectives in their order, each whose `objective_id` (a string) the stored copy has with a
  greater `Number(progress) || 0` takes the stored progress (written back as a number in its place); objectives
  without a string id pass as they are; then every stored objective with an id the incoming list lacks, appended.
- `claimed`: `true` when the stored copy's `claimed` is `true` (never goes back).
- `update_version`: the stored value when it is a finite number and either the incoming is not finite or the stored is
  greater.

A key the merge sets keeps its position when the incoming object had it, else it is appended.

**The record written**: the (capped, §4.2.4) list, the draft data, both day fields carried from the stored record,
`update_version` = stored + 1 (1 for a new record), `created_at` kept (now for a new one), `updated_at` = now ISO.

#### 4.2.4 The cap

At most 1000 entries. Over it, entries are ranked by: protected entries last (host saves only, §4.3.3), then claimed
before unclaimed, then oldest `drafted_timestamp` first (`Date.parse` of `String(drafted_timestamp)`, an absent,
`null` or unparseable stamp counting as 0), ties in list order; the first (count − 1000) are dropped and the rest keep
their order.

#### 4.2.5 Claim metadata

`claimedAt` and `rewardCredit` belong to one drafted instance. For an upserted entry: when the stored entry is the
same instance, its non-empty `claimedAt` is kept, else `claimedAt` becomes now when the incoming is claimed and the
stored was not; `rewardCredit` is kept. When it is a new instance (or a new entry), both are cleared, and `claimedAt`
is now when the incoming is claimed.

#### 4.2.6 The payload (`BountyStatePayload`)

`{"season_start_date":"2020-08-23T00:00:00.000Z","season_end_date":"2099-01-01T00:00:00.000Z","bounties":[<each entry's bounty object, in stored order>],"draft_data":D,"draft_data_daily":D,"draft_data_weekly":D}`

Each `D` is the stored draft data of that group or the default
`{"current_draft_choices":[],"previous_draft_selections":[],"bronze_count":0,"silver_count":0,"gold_count":0}`. The
**empty state payload** is this with no record (`bounties` `[]`, three defaults).

#### 4.2.7 Newly claimed

The ids of the entries whose `bounty.claimed` is `true` and whose `claimedAt` equals the record's `updated_at` (claimed
in this very save). A claim stored before `claimedAt` existed is never reported.

### 4.3 Host saves (a player-host runtime)

Weekly season challenges are a fixed schedule: 11 weeks × 10 per Hunt Pass window. A host save is held to it; only the
impossible is refused, entry by entry, and the rest of the save applies.

#### 4.3.1 Weekly ids and stamps

**A weekly id** is `<season>_<week>_<slot>_Challenge_Season_<name>-season<season2>-<index>`: `season` and `season2`
one or more ASCII digits (not compared with each other, §14 item 9; `season` must fit an i64); `week` 1 to 11 and
`slot` 1 to 10, written canonically (no leading zero); `name` one or more of ASCII letters, digits and `_`; `index`
exactly the canonical text of `week − 1`. The parts are found at the first three `_`, the `Challenge_Season_` prefix,
the last `-`, then the last `-season`.

**Accepted stamps** for week `w` at now: with `S` the start of the active season window at now (core §8.4), `S + (w −
1) × 7 days`; and, during the first 2 hours after `S`, also the previous window's `S − window length + (w − 1) × 7
days`. A bounty's stamp is `Date.parse` of a string `drafted_timestamp` (anything else is NaN, never accepted); stamps
compare as instants, so two notations of one instant are equal.

**Protected**: a stored, claimed entry whose id infers weekly, parses as a weekly id and carries an accepted stamp.

#### 4.3.2 Judging the entries (`PlanHostBountySave`)

Validation is §4.2.2. Then each sent bounty, in order, against the stored weekly instances as the earlier entries of
the same save left them (the first stored entry of an id counts):

1. Every objective (a plain object) whose `Number(progress)` exceeds 100,000,000 is clamped to 100,000,000 (counted).
2. A non-weekly id passes as it is.
3. A weekly id that does not parse, or whose stamp is not accepted: **refused**.
4. A stored instance with the same stamp (as an instant): the same instance; it passes with the stored stamp's
   notation, and a claim in it marks the instance claimed for the later entries.
5. A stored instance with a finite stamp later than this one: **refused** (a stamp moves only forward).
6. The window of this stamp (the season window containing it) already holds 110 weekly ids and this id is not one of
   them: **refused**.
7. Otherwise it passes, and the id moves to its stamp's window.

A refused entry counts as **kept claimed weekly** when the stored instance is claimed, else **ignored weekly** (each
with up to 10 sample ids). The passing entries are merged as §4.2.3 with three differences: each entry is filed by its
inferred group even under a single draft key (a `Challenge_Daily_*` id sent under the weekly key must not become a
fundable weekly claim); in a replace, the ids the host **sent** (refused ones included) decide what is kept, and a
protected entry the host left out stays (counted as kept claimed weekly); and protected entries rank last in the cap.
`incoming` counts what was sent.

#### 4.3.3 Summary of a host save (the host-write tag)

An object in this order: `mode`, `group` (when one), `incoming`, `removed`, `newlyClaimedCount`, `newlyClaimed` (at
most 24 ids), `keptClaimedWeekly`, `ignoredWeekly`, `clampedProgress`, `keptClaimedWeeklyIds` and `ignoredWeeklyIds`
(each only when non-empty), `version` (the new `update_version`).

### 4.4 Deletes (`DeleteBounties`)

**Parsing**: the body must be an object (`body must be an object`). Each of `bounty_ids`, `bountyIds`, `bounties`, when
present, must be an array (`{key} must be an array`) of at most 512 (`too many bounties ({n})`), each item a string or
an object with a string `bounty_id`, non-empty and at most 200 UTF-16 units (`every entry of {key} needs a bounty id
string`); the ids are collected once each in first-seen order. `delete_claimed_bounties` is `true` exactly. No id and
not `delete_claimed_bounties`: `no bounty to delete`.

**Applying**: every stored entry of effective group weekly is kept (season challenges are never deleted); any other
entry whose id is named, or that is claimed when `delete_claimed_bounties` is set, is removed. The record keeps the
draft data and `created_at`, `update_version` + 1, `updated_at` now, and **both day fields cleared** (§14 item 3). The
summary names the deleted ids and the named ids not deleted.

### 4.5 Season challenge rewards (`challengeRewards.ts`)

Completing a season challenge makes the game server claim it (a bounty save) and then post coins (`POST /inventory`).
Each completed challenge pays once.

- **A challenge-reward grant**: the version has a seasonal coin (1.14.7: `CURRENCY_SEASONAL_COIN`, at most 1000 per
  claim), the catalog id is that coin, and the quantity is finite, above 0 and at most the ceiling. No seasonal coin in
  the version: no funding rule.
- **The game day key** of an instant: the most recent 17:00 UTC at or before it, as ISO text.
- **Daily paid**: the record's `daily_reward_credited_on` equals today's game day key.
- **The fundable claim**: among entries that are claimed and have no `rewardCredit`, of effective group weekly, or
  daily when the daily is not paid: the one with the greatest `claimedAt` text (absent as empty, so pre-`claimedAt`
  claims count as oldest); the first wins a tie.
- **Funding** (inside the caller's transaction): the stored record (none: not funded), its fundable claim (none: not
  funded), then a credit `{catalogId, amount: quantity, transactionId: <the transaction's, or "unknown">, at: now ISO}`
  on every entry with that `bounty_id` (an entry of that id that is not a fundable claim is the data error `bounty
  {id} is not a fundable claim`, which rolls the transaction back); a daily one also sets `daily_reward_credited_on`
  to the game day key of the credit's time; `update_version` + 1, `updated_at` now; saved with the stored version
  expected. A version mismatch is "not funded" (the claim is no longer ours to charge).
- **Daily limited**: a challenge-reward grant, the daily paid, and some claimed, uncredited entry of effective group
  daily: the one case in which inventory skips the coins and keeps the rest of the transaction.

### 4.6 Cooldowns

The game keeps its once-per-period gates here (the daily bounty, the patrol bonus, a harvested sprout). A save carries
`{cooldowns:[{cooldown_id, cooldown_started_date}]}`; the read answers a map `{"<id>":"<date>"}`. Dates are stored and
served exactly as written.

#### 4.6.1 Validation (`ValidateCooldownSave`)

| Rule | Error |
| --- | --- |
| The body is an array, or an object whose `cooldowns` is an array | `cooldowns must be an array` |
| At most 256 | `too many cooldowns ({n})` |
| Each `cooldown_id` a non-empty string of at most 200 UTF-16 units | `every cooldown needs a cooldown_id string` |
| Each `cooldown_started_date` a non-empty string of at most 64 UTF-16 units that `Date.parse` reads | `cooldown {id} needs a cooldown_started_date date string` |

#### 4.6.2 The dedicated merge (`MergeCooldownState`)

Starting from the stored entries in order: a stored **gatherable** entry (an id of exactly 32 ASCII hex digits, either
case) whose start is more than 48 hours before now is dropped, unless this save writes that id (an unparseable date
keeps the entry). Then each incoming entry replaces the stored entry of its id in place (the latest write for an id
wins) or is appended, stamped `updated_at` = now. Over 1024 entries: sorted by `updated_at` descending
(`locale_compare`, stable) and cut to 1024. `update_version` + 1, `created_at` kept, `updated_at` now. No other entry is
removed.

#### 4.6.3 The host merge (`MergeHostCooldownState`)

A start date may move only later, and not far ahead. Room = 1024 − stored count. Each incoming entry in order, against
the stored dates as earlier entries of this save left them:

| Case | Effect | Counted as |
| --- | --- | --- |
| The date is later than the **latest allowed** (below) and is not the very date stored (as an instant) | refused; the stored date stays; costs no room | `refusedFuture` (id, attempted, latest ISO, stored when any) |
| Not stored, no room | dropped | `droppedNew` |
| Not stored | accepted (room − 1) | `added` (id, to) |
| Stored date finite and later than the incoming | the stored date stays | `keptLater` (id, stored, attempted) |
| Equal instant | accepted | `unchanged` |
| Otherwise (later, or the stored date unparseable) | accepted | `raised` (id, from, to) |

The accepted entries then go through §4.6.2. **The latest allowed**: for a **season id** (`season` in any ASCII case,
then one or more digits, then at most one ASCII letter: `season43`, `season10a`), the end of the active season window
at now (core §8.4); for any other id, now + 48 hours. The change lists keep at most 20 entries each; the counts are
exact. `received` is the incoming count.

**The tag** (`hostWrites.record`), in order: `route` (`batch` or `start`), for a start `id` and `startedAt`, then
`received`, `unchanged`, `added` (`[{id,to}]`), `addedCount`, `raised` (`[{id,from,to}]`), `raisedCount`,
`keptLater` (`[{id,stored,attempted}]`), `keptLaterCount`, `droppedNew`, `refusedFuture` (`[{id,attempted,latest,stored?}]`),
`refusedFutureCount`, and for a batch `version`.

#### 4.6.4 Start (§3.6)

The id rule is `^[A-Za-z0-9_.:-]{1,200}$` (ASCII bytes). The start date is now ISO from the backend's clock.

#### 4.6.5 The payload (`CooldownStatePayload`)

An object: each stored id → its stored date, in stored order, written by the JSON writer (so integer-like ids come
first, ascending). An id that collides with an object prototype name (`__proto__`, `constructor`) is plain data. No
record: `{}`.

### 4.7 Entitlements (`controllers/entitlements.ts`)

The third grant a SKU can carry, and how Hunt Pass Premium and the Middleman's extra slots are owned.

- **Read**: the account's live entitlements (store §11.1 `EntitlementRepository::find`), as `{name, duration,
  activated_date}`, sorted by name with `service_kit::collate::locale_compare`; none is an empty list.
- **Holds**: whether the account has a live entitlement of that exact name (`DoesUserHoldEntitlement`; no caller at
  `e0e238b`, kept for §6).
- **Grant in a transaction** (`GrantEntitlementsInTransaction`), for a list of `(name, duration)` and an optional
  source SKU id: a name failing store §11.1's `is_valid_entitlement_name` refuses the whole grant with the data error
  `Refusing entitlement grant for {account}: unsafe or missing entitlement name {name quoted}` (a data defect, not
  skipped). A non-finite duration is 0. Each grant inserts `{name, duration, activated_date: now ISO when duration >
  0 else none, granted_at: now ISO, source_sku_id}` unless the name is held (live or soft-deleted, store §16 item 4).
  Answers the names it added; logs `[Entitlement] {account} granted {a, b}{ (sku={id})}` when any.

### 4.8 The wallet and Platinum

#### 4.8.1 The wallet

An account's balance map (store §11.1 `WalletRecord`, balances in first-written order). **The starter grant**, written
once when the wallet is first created: `CURRENCY_NOTES` 5000, `CURRENCY_PJM_WEAPON` 50, `CURRENCY_PJM_ARMOR` 50,
`CURRENCY_PJM_ALCHEMY` 50, `CURRENCY_PJM_AIRSHIP` 50, `CURRENCY_PJM_FISHING` 50, no bootstrap version.

| Operation | Contract |
| --- | --- |
| Get (in a write) | `wallets.create_if_missing(starter)`: the stored balances, never resetting an existing wallet |
| Read (on a read connection) | The stored balances, or the starter's without writing |
| Balance of one id | Get, then that id's balance or 0 |
| Add `(id, delta)` | An empty id, a non-finite or zero delta: the current balance (which still creates the wallet). An id failing `is_valid_balance_catalog_id` (store §11.1): `UnsafeCatalogId`, `Refusing currency change: unsafe balance catalogId {id quoted}`, before any wallet is created. Else `increment_balance`; nothing applied and no wallet: create the starter wallet and increment again. Applied: logs `[Wallet] {account} {id} {+ when ≥ 0}{delta} -> {new}` and answers the new balance. Not applied (a debit beyond the balance): `Insufficient`, `Insufficient balance: user {account} has {current} of {id}, cannot apply delta {delta}` (never clamped) |
| Spend Platinum `(amount)` | §4.8.2 |
| Merge into stacked items | Each wallet balance over an inventory's `stackedItems`: an entry with that `catalogId` gets `catalogId` and `quantity` set (other keys kept, in place), else `{"catalogId":..,"quantity":..}` is appended. A projection only; nothing is written back |

#### 4.8.2 Platinum buckets (`platinumWallet.ts`)

Platinum is stored as 13 buckets and shown as one number. **Spend order** (most disposable first):
`CURRENCY_PLATINUM_CS`, `CURRENCY_PLATINUM_UNIV`, `CURRENCY_PLATINUM_PLATFORM_BONUS`, `CURRENCY_PLATINUM_EPIC_BONUS`,
`CURRENCY_PLATINUM_XBOX_BONUS`, `CURRENCY_PLATINUM_PSN_BONUS`, `CURRENCY_PLATINUM_SWITCH_BONUS`, `CURRENCY_PLATINUM`
(the aggregate id), `CURRENCY_PLATINUM_PLATFORM`, `CURRENCY_PLATINUM_EPIC`, `CURRENCY_PLATINUM_XBOX`,
`CURRENCY_PLATINUM_PSN`, `CURRENCY_PLATINUM_SWITCH`. Only these 13 are Platinum; `CURRENCY_PLATINUM_STEAM` or
`ID_CURRENCY_PLATINUM` are not.

- **Total**: the sum of the finite balances of the 13.
- **Spend `amount`**: requested = `trunc(amount)`; not finite or ≤ 0: nothing, an empty report. Else get the wallet;
  total < requested: logged `[Wallet] {account} platinum debit rejected: requested {r}, holds {a} across all buckets`
  and `Insufficient` (id `CURRENCY_PLATINUM`, delta −requested, current = total); no partial plan. Else each bucket in
  spend order with a finite positive balance gives `min(balance, outstanding)` until covered, each applied as an Add.
  The report lists each touched bucket with its new balance, then `CURRENCY_PLATINUM` with the new total (so a touched
  aggregate bucket appears twice; a map built from the report keeps the total, §14 item 8).

#### 4.8.3 Free Platinum (`platinumStoreFree.ts`)

Platinum prices are 0 unless the raw setting `PLATINUM_STORE_FREE` is exactly `0`. **Effective price**: 0 when free
and the currency id lower-cased is `id_currency_platinum`; else the price.

#### 4.8.4 Projection (`ProjectBalances`)

Over a base map: every wallet balance in wallet order under its catalog id and its **id form** (`id_currency_` + the id
with a leading `CURRENCY_` removed, lower-cased); a key already in the base keeps its position. Then, only when the
total is not 0 or the wallet holds a `CURRENCY_PLATINUM` balance, `CURRENCY_PLATINUM` and `id_currency_platinum` are
set to the total (so an account without Platinum keeps the base's value). Numbers as §2.

### 4.9 The store catalog (`controllers/store.ts`)

#### 4.9.1 Sections

A version serves the sections its `store_sections` table names (1.14.7: all nine). **Section order** (the lookup's
order): `exchange_vendor_slot_2`, `exchange_vendor_slot_3`, `ladyluckstore`, `season_store`, `webstore`, `dyes`,
`hp_level_skip_rank`, `huntpass_store`, `fountain_daily_free_bundle`. An unserved tag is an empty section.

| Section | Offers |
| --- | --- |
| `ladyluckstore` | `ladyluck_store.json` |
| `season_store` (the Reward Cache) | `reward_cache_store.json` |
| `webstore`, `dyes`, `hp_level_skip_rank`, `huntpass_store` | the SKUs of `platinum_store.json` (read when any of the four is served) whose tags include the section's tag; one SKU can be in two (a dye is in `webstore` and `dyes`) |
| `exchange_vendor_slot_2`, `_3` | One fixed SKU each (§4.9.3) |
| `fountain_daily_free_bundle` | `fountain_daily_bundle.json` (optional, §4.9.4); the only **daily** section |

**Reading a file**: from the module's backend game data folder (core §8.1 `backend_file`; no folder is "missing"),
one UTF-8 BOM dropped, parsed with JavaScript number semantics (`service_kit::jsjson::parse`). Missing:
`[Store] game data file {file} is missing (generate it from your own installation) - its sections are empty`; not an
array: `[Store] {file} is not a list of offers - its sections are empty`; unreadable or bad JSON: `[Store] {file}
could not be read - its sections are empty: {error}` (all errors). An element that is not an object with a string `id`
is skipped.

**The catalog is read once per module**, on first use, and never refreshed (Node reads at start). There is no runtime
rotation: Lady Luck's rotation is whatever `ladyluck_store.json` holds (built by gamedata §5.5 or §5.6); a new rotation
is a regenerated file and a restart.

#### 4.9.2 An offer as read

From the file object, kept whole for the listing, and read as: `id`; `tags` (the string entries); `prices` (each
`currencyId` as `String(..)`, `price` as written or `null`); `maxAllowed` as written or `null`; `items` (each
`catalogId` as `String(..)`, `quantity` as written or `null`, `instanced` = exactly `true`); `entitlements` (each
`name` as `String(..)`, `duration` as written or `null`); `skuProgression` unless absent or `null`. **One-time**:
`maxAllowed` is the number 1.

#### 4.9.3 The Middleman's slots

Slot 1 is free; slots 2 and 3 are unlocked by the entitlements `exchange_slot_2` and `exchange_slot_3`. No live capture
exists, so each is this SKU (slot `n`):

`{"id":"single_exchange_slot_<n>","displayName":"Cell Fusion Slot <n>","displayDescription":"Unlocks cell fusion slot <n> at the Middleman permanently.","displayPriority":<n>,"prices":[{"currencyId":"id_currency_platinum","price":1000}],"maxAllowed":1,"tags":["exchange_vendor_slot_<n>"],"items":[],"entitlements":[{"name":"exchange_slot_<n>","duration":0}],"duplicateInstancedItems":[]}`

#### 4.9.4 The fountain's daily bundle (`ParseFountainDailyBundle`, fail-closed)

The file is an array, or an object whose `skus` is an array; it must hold exactly one offer (the client uses only the
first). Any problem refuses the whole offer, logged `[Store:fountain_daily_free_bundle] fountain_daily_bundle.json
rejected - the fountain stays unavailable: {problems joined "; "}`. A missing file logs `... no
fountain_daily_bundle.json - the fountain stays unavailable (optional)`; an unreadable one `... could not be read -
...: {error}`. Problems (`<v>` is the value as compact JSON, or `undefined`):

| Rule | Problem |
| --- | --- |
| Shape | `expected { "skus": [ <one offer> ] } or an array holding one offer`; `expected exactly one offer (the client only uses the first), found {n}` |
| `id` a string of 1 to 128 bytes of ASCII letters, digits, `_`, `.`, `-` | `invalid id <v> (letters, digits, _ . - only: it is a URL segment of /token/<slug>/<sku>)` |
| `tags` an array containing `fountain_daily_free_bundle` | `tags must include fountain_daily_free_bundle` |
| `prices` exactly `[{currencyId:"id_currency_platinum", price: <the number 0>}]` | `prices must be exactly [{ "currencyId": "id_currency_platinum", "price": 0 }] (the client opens the coin toss only for PlatPrice 0)` |
| `entitlements` absent, `null` or `[]` | `entitlements are not allowed (stacked items only)` |
| `skuProgression` absent or `null` | `skuProgression is not allowed (stacked items only)` |
| `loadoutSlots` absent or `null` | `loadoutSlots are not allowed (stacked items only)` |
| `items` a non-empty array | `items must list at least one stacked item` |
| Each item's `catalogId` a non-empty string (else the item's other rules are skipped) | `item with invalid catalogId <v>` |
| `instanced` exactly `false` | `{id}: instanced items are not allowed (set "instanced": false; stacked items only)` |
| `quantity` a whole number, 1 to 2^53 − 1 | `{id}: quantity must be a positive integer` |
| `inventory_storage_policy.json`'s `entries[id].storage` is `stacked` | `{id}: inventory_storage_policy.json says {storage or nothing}, not stacked (regenerate it after choosing a new item)` |
| and its `maxQuantity` is not above 0 | `{id}: capped at maxQuantity {n}; a daily offer may only grant uncapped stacked items` |
| Not a prestige-gated reward | `{id}: a prestige-gated reward currency cannot be granted by the store` |

**Prestige-gated rewards**: from `progression_config.json`'s `payload.paths`, every path with a string
`progression_id` and a finite `prestige.xp_per_level` above 0 contributes the `catalog_id` of each
`prestige.free_rewards.stacked_items` entry whose `quantity` is finite and above 0. A missing or bad file: none.

**The offer served**, in this order: `id`; `displayName` (the string, else `Daily Free Bundle`); `displayDescription`
(the string, else `""`); `displayPriority` (a finite number, else 0); `prices`
`[{"currencyId":"id_currency_platinum","price":0}]`; `maxAllowed` 1; `images` (an object, else `{}`); `tags` (the
strings); `items` each `{"catalogId":..,"quantity":..,"instanced":false}`; `duplicateInstancedItems` `[]`.

**Id clash**: when the offer's id equals any SKU id of the two slots, Lady Luck, the Reward Cache or the whole Platinum
file, the section is empty, logged `[Store:fountain_daily_free_bundle] SKU id {ids} is already used by another store -
section disabled` (a token names only the SKU id).

### 4.10 Lookup (`FindStoreSkuByIdOrCatalogId`)

1. The exact SKU id in the first section (section order) holding it; daily sections included.
2. Else the SKUs, across the non-daily sections in section order, with an item whose `catalogId` equals the text. When
   they are all one SKU id (one offer in several sections is not ambiguous): the first, with its section. Two
   different SKU ids, or none: not found.

`from_section(tag, sku_id)` is the SKU of that id in that section only.

### 4.11 The listing (one SKU in `GET /product/skus/public`)

An object in this order:

| Key | Value |
| --- | --- |
| `id`, `displayName`, `displayDescription`, `displayPriority` | Copied from the file object, each only when present |
| `prices` | Each `{"currencyId":..,"price":<0 when its effective price for a price of 1 is 0, else as written>,"salesPrice":null,"multiPrice":null}` |
| `maxAllowed` | As read (`null` when absent) |
| `remaining` | 0 for a one-time SKU the caller owns (below), else 999 when `maxAllowed` is `null`, else `maxAllowed` |
| `duplicateInstancedItems` | The file's, else `null` |
| `images` | The file's unless absent or `null`, else `{}` |
| `tags` | The file's, else `null` |
| `scheduledTags` | `null` |
| `items` | Each `{"catalogId":..,"quantity":..}` (no `instanced`) |
| `entitlements` | Each `{"name":..,"duration":..}` |
| `skuProgression` | `null`, else an object of `progressionId`, `ranks`, `xp` in that order, each only when present |
| `loadoutSlots`, `availableFrom`, `availableTo`, `timeAvailabilityReason`, `platformOfferId`, `missingEntitlementNames` | `null` each |

**Owned** (a departure from Node, which always answers `maxAllowed`; the live service's captures answer 0 for an
offer the player bought, and the 1.14.7 client shows such an offer as owned): outside a daily section, when the caller
is a player (a user id other than empty or `INVALID`) whose account exists, a one-time SKU is owned when its purchase
transaction id (§4.12 step 2) is in the account's ledger, under any of its characters or among its 500 most recent rows
(as §4.13.2), all looked up in one read. A failed lookup logs `[Store:{tag}] {user} ownership lookup failed - one-time
offers stay listed[: {error}]` (warn) and owns nothing; a repeat purchase still replays (§4.12). Any owned SKU logs
`[Store:{tag}] {user} owns {sku ids sorted, joined ", "}` (info).

### 4.12 The purchase flow (`PurchaseStoreSku`)

Given the token's claims and the section:

1. **The SKU**: `from_section(section, skuId)`; none: `unknown_sku`.
2. **The plan** (pure; before any account lookup):
   - **Transaction id**: a daily section: §4.13.1's id. Else a one-time SKU: SHA-256 hex of
     `{transaction scope}:{userId}:{skuId}`, the first 32, upper-cased. Else a fresh UUID v4 (32 hex, no hyphens),
     upper-cased (system random, §4.15).
   - **Prices**: any price that is not a finite number ≥ 0 refuses the purchase (`transaction_failed`, logged `[Store:{tag}]
     {sku} has an invalid price - purchase refused`); a negative price would be a credit through the debit path.
   - **Stacked removes**: each price whose effective price is above 0 gives `{"catalogId":<wallet currency>,"quantity":<price>}`.
     **Wallet currency**: `id_currency_marks_gilded` → `CURRENCY_MARKS_GILDED`, `id_currency_marks_steel` →
     `CURRENCY_MARKS_STEEL`, `id_currency_s19_coin` → `CURRENCY_S19_COIN`, `id_currency_seasonal_coin` →
     `CURRENCY_SEASONAL_COIN` (the Reward Cache is priced in the 1.14.7 seasonal coin), `id_currency_rewardcache` →
     `CURRENCY_REWARDCACHE`, `id_currency_platinum` → `CURRENCY_PLATINUM` (the aggregate; the wallet splits it);
     anything else upper-cased as it is (§14 item 6).
   - **Instanced adds**: for each item with `instanced`, `count` = its `quantity` as a number (a non-number gives
     none); for `i` = 0, 1, ... while `i < count`: `{"catalogId":..,"instanceId":<id>,"itemData":null,"updateVersion":0}`,
     the instance id being the SHA-256 hex of `{instance scope}:{userId}:{skuId}:{catalogId}:{i}` (with
     `:{window key}` appended for a daily section), the first 26, upper-cased. Deterministic, so a replayed purchase
     builds the identical request and the ledger does not see a mismatch.
   - **Stacked adds**: each other item `{"catalogId":..,"quantity":<as written>}`.
   - **Grants**: the SKU's entitlements as `(name, Number(duration) || 0)` and its `skuProgression` object.
3. **The account** of the claims' `userId`; none or a failure: `transaction_failed`.
4. **A daily claim** (§4.13.3) when the section is daily.
5. **The transaction**: inventory's `run_inventory_transaction` (§6.1 I1) with the account, `userId`,
   the token's `characterId`, the plan, ownership already validated (the signed token binds user to character), label
   `store:{tag}:{skuId}`, and economy source `STORE` with the SKU id. Its outcome maps as §3.16's table. Success logs
   `[Store:{tag}] {user} purchased {sku} (transactionId={id})`; a daily claim also logs, when the economy monitor is on,
   `[GrantMon] txn={id} user={user} src=store source="store:{tag}" sku={sku} window={key} add=[{catalogId xquantity, ...}] remove=[...] claim={new|replay (no new grant)}`.

**Transaction scopes** (`{transaction, instance}`; Lady Luck's are the deployed strings):

| Section | Transaction scope | Instance scope |
| --- | --- | --- |
| `ladyluckstore` | `ladyluck-purchase` | `ladyluck-purchase-instance` |
| `webstore`, `dyes`, `hp_level_skip_rank`, `huntpass_store` | `store-purchase:platinum` | `store-purchase-instance:platinum` |
| any other `<tag>` | `store-purchase:<tag>` | `store-purchase-instance:<tag>` |

**Atomicity**: everything a purchase changes (the ledger row, the currency debit across buckets, the item grants, the
entitlements, the progression XP) commits in one write or not at all; inventory owns that write (§6.1). A one-time SKU's
transaction id makes the inventory ledger the ownership check: a second purchase replays the stored result (204, no
second charge, no second grant). The SKU's entitlement and progression grants run only for a new transaction id.

### 4.13 Daily claims

#### 4.13.1 The window

Once per reset window, which opens at 17:00 UTC (the instant the client stores for its daily bounty cooldown): start =
`floor((now − 17 h) / 1 day) × 1 day + 17 h`, end = start + 1 day, **key** = the start as ISO text (at
`2026-10-10T16:59:59.999Z` the key is `2026-10-09T17:00:00.000Z`; at `17:00:00.000Z` it is `2026-10-10T17:00:00.000Z`).
**The claim's transaction id**: SHA-256 hex of `{transaction scope}:{userId}:{skuId}:{window key}`, the first 32,
upper-cased. Deterministic, so a second claim in the window replays the ledger row and the listing finds this window's
claim without a cooldown record.

#### 4.13.2 Listing a daily section

The SKUs are served only for a player whose claim this window is not in the inventory ledger (the only "claimed"
signal the client honours), and hidden whenever the claim cannot be checked:

| Case | Served | Log |
| --- | --- | --- |
| No offer loaded | none | `[Store:{tag}] no daily offer loaded (fountain_daily_bundle.json missing or rejected) - serving none` (info) |
| No `user_id`, empty or `INVALID` | none | `... daily offer requested without a player id - serving none` (warn) |
| No database, no account | none | `... {user} claim lookup unavailable - hiding the offer` (warn) |
| Per SKU: the claim lookup (§6.1 `find_claim`) finds a row | hidden | |
| Finds none | served | |
| Fails | hidden | `... {user} claim lookup failed for {sku} - hiding the offer: {error}` (warn) |

Then `[Store:{tag}] {user} window={key} -> open ({ids}) | claimed or unavailable`.

#### 4.13.3 Claiming

Before the transaction, `find_claim(account, transaction id)`: a failure refuses (`transaction_failed`, fail closed);
a row held under **another** character refuses `already_purchased_different_request`; a row under this character goes
on (the ledger replays it); none goes on as new. The transaction's own ledger stays the authority for a same-character
replay.

### 4.14 The economy ledger monitor (`economyLedgerMonitor.ts`)

Monitoring only: nothing here refuses, limits or changes a grant. Inventory builds one ledger per committed
transaction and commits it after its own commit (§6.2). Settings §7.

- **Sources**: `STORE`, `OPERATOR`, `OPERATOR_TEST`, `QUEST`, `HUNT_PASS`, `INVENTORY_GAMESERVER`, `INVENTORY_HOST`,
  `OTHER`. A scope is a source plus optional store SKU id, P2P session id and host account id.
- **Platinum movement**: the aggregate id or any of the 13 buckets.
- **Hunt Pass reward ids**: `CURRENCY_SEASONAL_COIN`, `CURRENCY_REWARDCACHE`, `CURRENCY_PRESTIGE`,
  `GRANTPROGRESSITEM_HUNTPASS`; ids matching `^CURRENCY_S\d+_(COIN|DAILY)$`, starting `TOKEN_HUNTPASS`,
  `TOKEN_HUNT_PASS` or `CONTAINER_CORE_PRESTIGE_`; and from `progression_config.json` (read per ledger; missing gives
  none), the `catalog_id`s of the `stacked_items` of `free_rewards` and `premium_rewards` of every **pass track**
  (`String(progression_id)` lower-cased starting `season`, `minipass` or `test_minipass`) and of
  `prestige.free_rewards` and `prestige.premium_rewards` of every track (each reward group an array, a single object,
  or absent).
- **Credit source**: challenge-funded → `QUEST`; a `STORE` or `OPERATOR` scope keeps its source; a Hunt Pass reward id
  → `HUNT_PASS`; else the scope's source.
- **Recording**: a Platinum credit `(id, amount, balance after, challenge funded)` (amount not above 0 or NaN is
  ignored); a Platinum spend `(amount, balance after)` summed per transaction; a reward `(id, amount, challenge
  funded)` for a non-Platinum id that is challenge-funded or a Hunt Pass reward id.
- **Commit** at now (nothing when the source is `OPERATOR_TEST`, the monitor is off or nothing was recorded): `day` =
  the UTC date `YYYY-MM-DD`, `ttl_at` = now + 400 days. Log tokens are plain when 1 to 128 bytes of ASCII letters,
  digits, `_ . : -`, else the first 64 characters as a JSON string; absent is `-`.
  - Each credit: `[PlatMon] credit user={u} amount={a} balanceAfter={b} source={s} ref={txn}`; increments `credited`,
    `count`, `bySource.{s}.credited`, `bySource.{s}.count`.
  - A spend: `[PlatMon] spend user={u} amount={a} balanceAfter={b} source={s} for={f} session={p2p or -} host={host or -} ref={txn}`,
    where `for` is the store SKU id for a `STORE` scope that names one, else the granted items as `id:quantity` joined
    by `,`, the first 8 then `,+{n} more` (`-` when none); increments `debited`, `count`, `bySource.{s}.debited`,
    `bySource.{s}.count`, and for each distinct item of the first 8 (an id of 1 to 80 bytes of letters, digits, `_`,
    else `OTHER`) `byItem.{id}.count` + 1 and `byItem.{id}.debited` + the whole spend.
  - Each reward: `[HuntPassMon] reward user={u} item={id} amount={a} source={s} ref={txn}`; per currency increments
    `credited`, `count`, `bySource.{s}.credited`, `bySource.{s}.count`.
  - The Platinum increments are one rollup write under currency `CURRENCY_PLATINUM`, each reward currency one write
    (store §11.1 `add_to_rollup`, a plain system write, never failing the grant: a failure logs `[PlatMon]` or
    `[HuntPassMon]` `rollup not stored user={u} day={d} currency={c}: {error}` as a warning).
  - **Flags** after each write, from the returned day document (a value read as `Number`, NaN as 0): crossed when the
    day total was at or under the threshold before this transaction's amount and is over it after.
    `[PlatMon] FLAG credit user={u} day={d} credited={total} threshold={t} (this transaction +{a})`,
    `[PlatMon] FLAG spend user={u} day={d} debited={total} threshold={t} (this transaction -{a})`,
    `[HuntPassMon] FLAG user={u} day={d} item=CURRENCY_SEASONAL_COIN credited={total} threshold={t} (this transaction +{a})`
    (only for the seasonal coin). Warnings.

### 4.15 Clocks and randomness

- **No bounty is rolled here.** Drafts, choices and objectives come from the game server; `GET /bounty/game-data` is a
  stub and the tuning tables are `rustic-isles-gamedata` §3.4. The server never chooses a bounty.
- **The only randomness** is the repeatable purchase's fresh transaction id: system random, not the replay id source
  (§14 item 5). Every other id is a SHA-256 derivation (§4.12, §4.13).
- **Clocks**: the replay-aware clock (core §9.1) for every `now` above (claim stamps, cooldown starts, merges, the daily
  window, the game day, entitlement and credit times, host limits, the season window); the **system clock** for a
  purchase token's expiry check (auth §5.1), the economy ledger's commit time, and the wait for a fundable claim (§6.2).
- **Windows**: the season window is core §8.4's; the daily reset and the game day both start at 17:00 UTC.

---

## 5. Persistent state (store names)

All rows are `rustic-isles-store`'s (store §4.7, §11.1); this part owns no migration. Every bounty, cooldown, wallet
and entitlement write is an `IslesStore::account_write` for the target account (the home check, store §3.4: an account
homed elsewhere is a store failure, so a 500 page); the rollup writes are plain `write`s as the system.

| Need | Store call |
| --- | --- |
| Account of a `userId` | `accounts.find_by_user_id` |
| Bounty state | `bounty_states.get`, `bounty_states.save_if_version` (none expected: insert-if-absent; an answer of none is a lost race, retried) |
| Cooldowns | `cooldown_states.get`, `cooldown_states.save_if_version` |
| Entitlements | `entitlements.find`, `entitlements.grant_if_missing` |
| Wallet | `wallets.find`, `wallets.create_if_missing`, `wallets.increment_balance` |
| Rollups | `economy_rollups.add_to_rollup` |

The compare-and-retry loops (8 attempts) run inside one account write each, so the retries see one consistent state;
they exist for parity with Node's optimistic loop.

---

## 6. Calls into and out of this part

Names are the contract the three parts meet on; each part's spec owns its side. "In the caller's transaction" means
the call takes the caller's `Tx` and repositories and does no write of its own outside it.

### 6.1 Calls this part makes to inventory

The inventory spec's row I1 (its §12.3), called with a `TxContext` built from the module's `Shared`, the store slot
and `InventoryPorts::default()`; this part never calls an inventory handler.

| # | Call | Inputs | Output | Errors | Atomicity |
| --- | --- | --- | --- | --- | --- |
| I1 | `run_inventory_transaction(ctx, TxRequest, TxOptions, request_id)` (Node `RunInventoryTransaction`) | `TxRequest { user_id: <the token's>, character_id: <the token's>, transaction_id: <§4.12>, add_instanced: <the plan's instanced adds>, add_stacked: <the plan's stacked adds>, remove_instanced: [], remove_stacked: <the plan's removes>, save_instanced: [] }`; `TxOptions { ownership_already_validated: true, diagnostic_label: "store:{tag}:{skuId}", on_grant: <the grant hook of §6.3, only when the SKU has entitlements or a progression payload>, allow_challenge_reward_funding: false, operator_test_funding: false, economy: Economy { source: "STORE", p2p_session_id: none, store_sku_id: <skuId> } }`; the request id | `TxOutcome::Done(result)` (new, or the stored result of a replay); `TxOutcome::NotOwned` | `TxError::InsufficientBalance` → `insufficient_balance`; `Conflict` → `conflict`; `Mismatch` → `already_purchased_different_request`; `NotOwned` and every other `TxError` (prestige, stacked item, denied, policy, repository) → `transaction_failed`, logged `[Store:{tag}] {user} purchase of {sku} failed: {error}` | One inventory transaction: ledger begin, the currency step (inventory's `ApplyCurrencyDeltas`, inventory §9.1, over this part's W2/W3), **then** the grant hook, then the inventory write and ledger completion; any `Err` rolls all of it back. The hook runs only for a new transaction id. After commit, inventory's observer hands the committed transaction to W9 |

This part's own store types (`PurchasePlan`, `SkuGrants { entitlements: Vec<(name, duration)>, progression:
Option<Json> }`) are what it builds the request and the hook from; the transaction types are inventory's (its §11.2).

**The daily claim lookup** (`FindClaimInInventoryLedger`) is this part's own read, over the store's repositories (no
inventory call): `find_claim(store, account, transaction_id) -> Result<Option<DailyClaim { character_id:
Option<String> }>, String>`. One read: for each of the account's live characters in order
(`characters.find_many_by_account`), the ledger row (account, character, id) (`inventory_transactions.find_record`):
found answers its character; else the account's 500 most recent ledger rows
(`inventory_transactions.list_recent_for_account`), the first whose `transactionId` equals, answering its
`characterId` (absent allowed). A store failure is `Err(text)` (the callers fail closed).

### 6.2 Calls inventory makes to this part (inventory's W1 to W9)

Names, inputs, outputs, errors and atomicity exactly as the inventory spec's §12.1. "In tx": the call takes the
caller's `&mut Tx` and `&Repositories` and adds its writes to the caller's transaction; it never opens its own.

| # | Call | Inputs | Output | Errors | Atomicity |
| --- | --- | --- | --- | --- | --- |
| W1 | `wallet::get_wallet` | tx, repos, account | balances, ordered `(catalog id, f64)` | repository | In tx; `wallets.create_if_missing` with the starter balances (§4.8.1), never resetting an existing wallet |
| W2 | `wallet::add_currency` | tx, repos, account, catalog id, signed delta (`f64`) | the new balance | `WalletError::Insufficient { account, catalog_id, delta, current }` (display `Insufficient balance: user <account> has <current> of <id>, cannot apply delta <delta>`), `WalletError::UnsafeCatalogId(id)`, `WalletError::Repo` | In tx; §4.8.1's Add: an empty id, a non-finite or zero delta answers the current balance (via W1, so it may create the wallet); a debit never goes below zero (one guarded `increment_balance`); a credit with no wallet creates the starter wallet first |
| W3 | `wallet::spend_platinum` | tx, repos, account, amount | each bucket debited with its new balance, in the spend order, then (`CURRENCY_PLATINUM`, the new aggregate) | `Insufficient` naming `CURRENCY_PLATINUM` with the aggregate held as `current`; repository | In tx; §4.8.2: `trunc(amount)` not above 0 → nothing; never a partial plan |
| W4 | `wallet::merge_wallet_into_stacked` | balances, stacked items | stacked items with each balance's entry quantity replaced (a number value), the rest appended `{catalogId, quantity}` | — | Pure (§4.8.1) |
| W5 | `challenge_rewards::fund_challenge_reward` | tx, repos, the version's coin (`Option<SeasonalCoin>`), account, catalog id, quantity, transaction id (optional; `unknown` when none) | the funded claim (`FundedClaim { bounty_id, claimed_at }`) or none | repository (a non-fundable entry of the chosen id is a data error, rolling back) | In tx; only for a challenge-reward grant (§4.5); records the credit on one claimed, uncredited season challenge with a version-checked `bounty_states.save_if_version`; a version miss is none |
| W6 | `challenge_rewards::is_daily_challenge_reward_limited` | tx, repos, coin, account, catalog id, quantity | bool | repository | In tx, read only (§4.5 "daily limited") |
| W7 | `wait_for_fundable_season_claim` | store, account | bool | — (a read error reads as "not found") | Outside any tx: up to 1.5 s, a read (`bounty_states.get`) every 100 ms for a fundable season claim (§4.5) at the system clock's now (§14 item 5); the coin grant may arrive about 50 ms before the bounty save that claims it |
| W8 | `platinum::PLATINUM_AGGREGATE_ID` | — | `CURRENCY_PLATINUM` | — | Constant |
| W9 | `economy_ledger::commit_transaction(shared, &CommittedTransaction)` | the committed transaction (inventory §6.1: account, user id, transaction id, source, `granted`, `currency_credits`, `currency_debits`, result), the module's settings and game data | — | logged only | After the inventory commit, in the background (a spawned task on the current runtime; none: nothing); never fails the request. §4.14 with: the source text mapped to its `EconomySource` (unknown → `OTHER`), the scope that source alone (no SKU, session or host; §14 item 7), `granted` entries with a string `catalogId` and a finite positive quantity; the **Platinum balance after** = the sum over the result's `updatedStackedItems` of Platinum-movement ids of `max(Number(quantity), 0)`; each credit `{catalogId, amount}` with a finite positive amount: a Platinum movement records a Platinum credit with that balance after (when it is 0, that id's own `updatedStackedItems` quantity, else 0) and challenge-funded = source `QUEST`, any other id a reward with the same flag; each debit of a Platinum movement with a finite positive amount records a spend with the balance after; then `commit_to_store(store, account, system now)` when anything was recorded |

Inventory decides **when** to call W1 to W7 (its §9.1 `ApplyCurrencyDeltas`, the prestige gate and the Sparkforge
conversion are inventory's); this part specifies only what each primitive does. Also public, for the routes here and
any caller: `wallet::read_wallet(conn, repos, account)` (the balances or the starter's, writing nothing),
`wallet::project_balances(balances, base)` (§4.8.4, Node `BuildBalanceDict`), `platinum::{is_platinum_catalog_id,
sum_platinum, effective_price, platinum_store_free}`, `challenge_rewards::is_challenge_reward_grant(coin, catalog_id,
quantity)` (§4.5; inventory §9.4 states the same rule), and the economy ledger's parts (`EconomyLedger` with
`record_platinum_credit`, `record_platinum_spend`, `record_reward`, `is_empty`, `commit(now) -> CommitPlan`,
`commit_to_store(store, account, now)`; `EconomyMonitorSettings::read`; `hunt_pass_reward_ids(game_data_root)`).

### 6.3 Calls this part makes to progression (the grant hook)

Inside the store transaction, after the wallet step, when the SKU has entitlements or a progression payload:

1. The entitlement grant (§4.7, this part), source SKU id the SKU's.
2. When `skuProgression` is present: `progression::resolve_track_id(shared, id)` with `progressionId` as a string (else
   empty): the selected Hunt Pass alias resolves to the active pass, any other id only when the progression config has
   a curve for it; none skips the payload silently.
3. `xp` = `trunc` of a numeric `xp` (else 0), `ranks` = `trunc` of a numeric `ranks` (else 0). `xp > 0`:
   `progression_tracks.increment(tx, account, track, xp)`. Else `ranks > 0`: with the track's current progress
   (`progression_tracks.get`, none is 0) and its curve (`progression::curves(shared).get(track)`; none skips): the rank
   for that progress (`rank_for_progress`, none is 0); when below the curve's top rank, the target is `min(rank + ranks,
   top)` and `required_xp_for_rank(target)` minus the current progress, when above 0, is the increment.

| Call | Inputs | Output | Errors | Atomicity |
| --- | --- | --- | --- | --- |
| `progression::resolve_track_id(shared, id)` | id | `Option<track id>` | none | Pure over the cached curves |
| `progression::curves(shared)` with `get(track)`, `rank_for_progress(track, progress)`, `TrackCurve::max_rank`, `TrackCurve::required_xp_for_rank(rank)` | | | none | Pure |
| `progression_tracks.get(conn, account, track)`, `progression_tracks.increment(tx, account, track, amount)` (store §10.2) | | record | `RepoError` (rolls the purchase back) | In the store transaction |

Progression makes **no call** into this part at `e0e238b` (its wallet mention is a log text). Nothing else of
progression's reaches the store.

### 6.4 Calls this part makes elsewhere (seams for later units)

| Call | Owner | Inputs | Output | Errors | Used by |
| --- | --- | --- | --- | --- | --- |
| `Auth::game_auth` | auth §4.1 | the request | `AuthData` or the refusal | as auth | every route |
| `Auth::sign_store_token(&StorePurchase)`, `Auth::validate_store_token(text)` | auth §5.1 | `user_id`, `character_id`, `sku_id`, `store_tag: Option` | token / claims | `TokenError` (sign: 500 page; validate: 401) | §3.15, §3.16 |
| `Characters::first_character_id(shared, store, actor, user)` | inventory's `Characters` port (inventory §11.3; characters unit 5 behind it) | the account's stored `user_id` (the account id text when the account row is gone), actor `Player { account }` | `Option<character id>`: the first entry's `id` of the user's character list, which **creates** the first character when the account has none | repository error (500 page) | §3.15 |
| `ForeignAccounts::foreign_account(shared, user_id)` | matchmaking (unit 8), through this part's slot (§10.3) | path `userId` | `Option<GlobalId>`: the account the text names (a global account id text of kind account as it is, else the live local account's id) when its owner node is not this node | none (a lookup failure is none) | §3.9 step 4 |
| `RingInstances::ring_instance_for_player(&GlobalId)` | inventory's slot trait (inventory §11.3; matchmaking fills it) | global id | `Option<GlobalId>`: the hunt instance the player was placed in, else the last ring instance | none | §3.9 step 4 (none: a new hunt id minted on this node, `GlobalId::new(IdKind::Hunt, node)`) |
| `HomeApi::ring_forward_hunt_rewards(&HuntRewards)` | `module-api` (core §5.4) | `{hunt, module: "isles-<game version with . as ->", account, xp_grant: none, loot_write: none, objective_write: {"bounty_progress": <the body when an object, else {"bounty": <the body or null>}>}}` | ok | error text (500 page; an invalid module id also 500) | §3.9 step 4 |
| `ClaimActivity::challenges_claimed(user_id, count)` | Reforged API (unit 9), through this part's slot | path `userId`, the newly claimed count | none | none (fire and forget; nothing for 0 or a blank id) | §3.9 step 7 |
| `rifts.guard().authorize_host_write(runtime, kind, target, options default)` | `rustic-rifts-seam` | the runtime, `HostWriteKind::Cooldown` or `Bounty`, the target account's global id text | `Option<Reply>` (some: refused, answered with its status and JSON body) | `SeamError`: 500 page, logged `hostWrites.authorize: {e}` | §3.5, §3.6, §3.9, §3.10 |
| `rifts.guard().record_host_write(runtime, HostWrite { kind, target, summary })` | `rustic-rifts-seam` | the summary as compact JSON text | none | logged `hostWrites.record: {e}`, never fails the answer | after an accepted host write |

**What receives the forward** is the home node's ring handling (core §7.8's home operations; the Reforged unit's).
Restricted's `bounty.write` home operation appends the progress as a raw base entry without this part's merge; that
is the home side's behaviour, recorded here only so the three specs agree on who owns it (§14 item 13).

### 6.5 Calls other parts make to this part, besides inventory

| Caller | Call |
| --- | --- |
| characters (unit 5) | None: it creates the starter wallet itself with the same six balances and its bootstrap version (`wallets.create_if_missing`), so a wallet made at character creation is adopted unchanged here |
| Reforged API (unit 9) | `platinum::AGGREGATE_ID` (the tester tool); the tester tool's grants go through inventory with operator funding |
| restricted's composer | §10.3's registration and slot |

---

## 7. Settings

All read with core §6 (`raw` unless said). None is a secret.

| Setting | Reading | Default | Used by |
| --- | --- | --- | --- |
| `RUSTIC_ISLES_BOUNTY_PERSISTENCE` (switch `bounty_persistence`) | §3.1 persistence (raw, then the version default) | `on` (1.14.7) | §3.8 to §3.10 |
| `RUSTIC_ISLES_COOLDOWN_PERSISTENCE` (switch `cooldown_persistence`) | the same | `on` | §3.4 to §3.6 |
| `RUSTIC_ISLES_ECONOMY_MONITOR` | `on_unless_off` | on | §9's monitor lines, the `[GrantMon]` line, the economy ledger |
| `RUSTIC_ISLES_PLAT_DAILY_FLAG`, `RUSTIC_ISLES_PLAT_DAILY_SPEND_FLAG`, `RUSTIC_ISLES_SEASONAL_COIN_DAILY_FLAG`, `RUSTIC_ISLES_XP_PER_MINUTE_FLAG` | JS-trimmed; empty or unset the default; else `Number(..)` finite and ≥ 0, otherwise the default with the warning `[PlatMon] {NAME}={first 40 chars as a JSON string} is not a non-negative number; using {default}` | 5000, 5000, 1000, 2000 | §4.14 (the XP threshold is read and carried, not used by this part) |
| `PLATINUM_STORE_FREE` | raw, exactly `0` turns prices on | free | §4.8.3 |
| `GAMESERVER_NO_PLAYER_USER_ID`, `DEV_USER_ID` | through auth's game auth only (the bearer-less game server's `user_id`, core §5.3) | | §3.2, §3.12, §3.15 |

Version tables read: `store_sections`, `seasonal_coin`, `switch_defaults`, `game_version` (the ring's module name);
the hook `entitlements_payload`.

---

## 8. Game data (inputs, never committed)

The store files are built on each server from the host's own install and from dainty-restricted's
`modules/rustic-isles/reference/1.14.7/ParadoxBackend/store-captures.json` (AGENTS.md); none is committed here, and
the crate's tests write their own small files (§12.3).

| File (backend game data folder) | Required | Format read here | Built by |
| --- | --- | --- | --- |
| `ladyluck_store.json` | for `ladyluckstore` | An array of store SKUs (gamedata §4.2 "a store SKU") | gamedata §5.5 or §5.6 |
| `reward_cache_store.json` | for `season_store` | The same | gamedata §5.4 |
| `platinum_store.json` | for the four Platinum sections | The same; a SKU's section membership is its `tags` | gamedata §5.7 |
| `fountain_daily_bundle.json` | optional | §4.9.4: one offer, as an array or `{"skus":[..]}` | The host, by hand (no generator) |
| `inventory_storage_policy.json` | for the fountain check | `{"entries":{"<catalogId>":{"storage":"stacked"|"instanced","maxQuantity":<n>,..}}}` | gamedata §5.8 |
| `progression_config.json` | for the fountain check and the Hunt Pass reward ids | `{"payload":{"paths":[{"progression_id","free_rewards","premium_rewards","prestige":{"xp_per_level","free_rewards","premium_rewards"}},..]}}`, reward groups with `stacked_items` `[{catalog_id, quantity}]` | gamedata §5.1 |

**A store SKU** (gamedata §4.2): `id`, `displayName`, `displayDescription`, `displayPriority`, `prices`
`[{currencyId, price}]`, `maxAllowed`, `images` (`{}` or `{"standard":"Engine.Texture2D'<path>'"}`), `tags`, `items`
`[{catalogId, quantity, instanced}]`, `duplicateInstancedItems`, then optional `entitlements` `[{name, duration}]` and
`skuProgression` `{progressionId, xp?, ranks?}`. Fields this part does not interpret are echoed by the listing as
written. The currencies a file may price in are the store-service ids (`id_currency_marks_steel`,
`id_currency_marks_gilded`, `id_currency_seasonal_coin`, `id_currency_platinum`, ...), mapped to wallet ids by §4.12.

---

## 9. Logs

English (a host's tool). The monitor lines are on unless `RUSTIC_ISLES_ECONOMY_MONITOR` is off; they never fail a
request.

| Line | When |
| --- | --- |
| `[DraftMon] BOUNTY-LOAD user={u} src={s} served={what}` | a bounty read; `what` is `persistence off (stub)`, `no-player sentinel`, or `stored {counts} version={v}` |
| `[DraftMon] BOUNTY-SAVE user={u} src={s} keys=[{body keys}] bounties={n}(claimed {c})[{first 6 as id{p=progress/... claimed=0|1}}{, ...}] daily{..} weekly{..} base{..} => {outcome}` | a bounty save; each draft part `choices={n} previous={n} b/s/g={b}/{s}/{g}` or `-`; outcome `stored {mode}{:group} incoming={n} removed={n} -> {counts} version={v}{ host: keptClaimedWeekly={n} ignoredWeekly={n} clampedProgress={n}}`, `REJECTED {error}`, `persistence off (stub), not stored`, `no-player sentinel, not stored`, `unknown account, not stored` |
| `[DraftMon] BOUNTY-DELETE user={u} ... body={first 2000 characters of the body as JSON}` | a delete (info; `REJECTED` warn); `deleted=[..] kept=[..] -> {counts} version={v}` on success |
| `Bounty reset request acknowledged, nothing changed (source={s})` | §3.10 step 4 |
| `[CooldownMon] LOAD user={u} src={s} served=[id@date, ...] ({detail})` | a cooldown read |
| `[CooldownMon] SAVE user={u} src={s} keys=[..] cooldowns=[id@date, ...] => {outcome}` | a batch; outcome `stored -> {n} cooldown(s) version={v}{ host: added= raised= unchanged= keptLater= droppedNew=}` or as above |
| `[CooldownMon] START user={u} src={s} id={id} => {outcome}` | a start; `user` and `id` are bare only when they pass the cooldown id rule, else the first 64 characters as a JSON string; outcomes `REFUSED ...`, `REJECTED ...`, `stored @{date} -> {n} cooldown(s) version={v}`, plus ` (kept the stored later date: a host cannot move a cooldown earlier)` or ` (refused: a host cannot start a cooldown that far ahead)`, `ERROR {e}` |
| `{counts}` | `{"base":n,"daily":n,"weekly":n,"unassigned":n,"claimed":n,"credited":n}` (stored groups, claimed bounties, credited claims) |
| `Reconcile for {u}: Rams={n} CombatMerit={n}` | §3.11 |
| `[Store...]`, `[Entitlement]`, `[Wallet]`, `[PlatMon]`, `[HuntPassMon]`, `[GrantMon]` | as in §3 and §4 |

A purchase token and a signing key never reach a log or a `Debug`. A store failure's display is logged with the route
(`{route}: {error}`).

---

## 10. The public seam

### 10.1 Shape

Part of one crate, `rustic-isles-economy` (lib, MIT, `crates/rustic-isles-economy`), as module `bounties` (with
`wallet`, `platinum`, `challenge_rewards`, `entitlements`, `store`, `economy_ledger` beside it, the names inventory's
W1 to W9 use). Depends
on dainty-lass MIT crates `rustic-isles` (core), `rustic-isles-store`, `rustic-isles-auth`, `rustic-rifts-seam`,
`module-api`, `service-kit`, and workspace `serde_json` (own `preserve_order`), `sha2`, `uuid` (v4) or `rand`,
`tracing`, `thiserror`, `tokio`, `axum`, `http`. Characters, matchmaking and the Reforged API are reached only through
inventory's ports and slots and this part's slot (§10.3), so the crate does not depend on units 5, 8 or 9.

**Seam changes for the integration pass** (not this row's): `crates/rustic-isles-economy` and `rustic-isles-economy`
in `[workspace.dependencies]`; auth's `RuntimeCaller` opaque handle (§14 item 1, already decided for inventory and
characters).

### 10.2 Interface (names are the contract the composer and the other two parts are written against)

| Item | Contract |
| --- | --- |
| `bounties::register(handlers: &mut rustic_isles::Handlers)` | Mounts §3's 17 handlers (`Service::Backend`, the paths exactly as the table has them) |
| `bounties::BountiesSeams { foreign: Arc<dyn ForeignAccounts>, activity: Arc<dyn ClaimActivity> }` | This part's slot (§10.3); `Default`: no foreign accounts, no activity |
| `pub trait ForeignAccounts { fn foreign_account(&self, &Shared, &str) -> BoxFuture<Option<GlobalId>> }` | §6.4 (matchmaking fills it) |
| `pub trait ClaimActivity { fn challenges_claimed(&self, user_id: &str, count: usize) }` | §6.4 (the Reforged API fills it) |
| Reused from inventory (its §11.2, §11.3) | the `Characters` port (through `InventoryPorts::default()`), the `RingInstances` slot trait, `TxContext`, `TxRequest`, `TxOptions`, `GrantHook`, `Economy`, `TxOutcome`, `TxError`, `CommittedTransaction` |
| `bounties::state` | Pure: validation, merge, host plan, delete, payload, newly claimed, group inference, weekly id parse, accepted stamps (§4.2 to §4.4); errors `BountySaveError(text)` |
| `bounties::cooldowns` | Pure: validation, dedicated and host merge, payload, season id, id rule (§4.6); `CooldownSaveError(text)`, `HostCooldownChanges` with its tag |
| `challenge`, `wallet`, `platinum`, `entitlements`, `economy` | §6.2's calls; `WalletError { Insufficient { account, catalog_id, delta, current }, UnsafeCatalogId(String), Repo(RepoError) }` with §4.8.1's texts |
| `store::{Catalog, Sku, Section}` | `Catalog::load(game_data_root, served)`, `Catalog::cached(shared)` (once per module), `skus(tag)`, `from_section(tag, id)`, `lookup(id_or_catalog_id) -> Option<(tag, &Sku)>`; `listing(settings, &Sku) -> Json`; `plan_purchase(settings, &Sku, tag, user_id, now_ms, fresh_id) -> Result<PurchasePlan, PurchaseReason>`; `daily_window(now_ms)`, `daily_claim_transaction_id(tag, user, sku, key)`, `transaction_scope(tag)`; `PurchaseReason` with `as_str()`, `status()`; `parse_fountain_bundle(raw, root) -> (Vec<Sku>, Vec<String>)` |
| `SkuGrants`, `DailyClaim`, `find_claim` | §6.1 (this part's) |
| `wallet`, `platinum`, `challenge_rewards`, `wait_for_fundable_season_claim`, `economy_ledger::commit_transaction` | W1 to W9 of §6.2 |
| feature `test-support` | §12.3 |

### 10.3 Registration, the slot and the store

- The composer calls `bounties::register` (core §5.7) and puts into the module's slots (core §5.1): the
  `rustic_isles_store::IslesStore` (no slot: no database, the routes answer as §3), a `BountiesSeams` (no slot: its
  default) and inventory's `RingInstances` slot (no slot: no ring instance). The first character comes through
  inventory's `Characters` port default. Restricted fills `BountiesSeams` from its matchmaking and activity code until units 8 and 9
  land, then those crates provide implementations.
- The store routes need the store and the auth: with a store slot, tokens are auth's and the inventory calls are this
  crate's inventory part; without one, `token` and `notification` answer 501 at §3.15 step 2 and §3.16's second row.
- The catalog is per module (a slot or a cell in the part's own module state), never a process global (core §17
  item 14's rule).

---

## 11. What stays in dainty-restricted

- `tests/golden/bounties_store/` (the 80-case replay, its recorder and fixture files), replayed there through the
  composer against this crate; a failure there is a bug on this side.
- The version crate's `entitlements_payload` hook and tables (until unit 12), the home operations `bounty.read` and
  `bounty.write` (until unit 9), the characters, matchmaking and activity implementations behind the slot (until units
  5, 8, 9), and every generated or captured store file.

---

## 12. Test cases

### 12.1 What may be copied

Nothing recorded. The goldens and the restricted tests (`bounty_state_tests.rs`, `cooldown_state_tests.rs`,
`store_tests.rs`, the unit tests of each file, `isles-1-14-7/tests/bounties_store.rs`) are authored test code,
described below only; the crate's tests are written from these tables.

### 12.2 Behaviour the authored tests fix

**Routes** (a module with a store on a temporary SQLite file, a fake game auth that admits `gs`, `player:<id>` and
`host:<id>` with roster `player-1`, `player-2`, and fakes for the slot and the token signer):

| # | Request | Expected |
| --- | --- | --- |
| R1 | `GET /bounty/game-data` as a player | 200, §3.7's payload byte for byte |
| R2 | any route without a credential | game auth's refusal (empty 401) |
| R3 | `GET /bounty/player-1` as player-1, nothing stored | 200, the empty state payload |
| R4 | `GET /bounty/player-2` as player-1 | empty 403 |
| R5 | `GET /bounty/player-2` as `gs` | 200 |
| R6 | `POST /bounty/player-1` as player-1 | empty 403 |
| R7 | `POST /bounty/player-1` as `gs` with a weekly seed and `draft_data_weekly` | 200 with the full state; `GET` returns the same bounties and draft |
| R8 | the same seed re-sent with one objective at a lower progress, then with `claimed:true` | progress keeps the maximum; claimed sticks; `claimedAt` set once |
| R9 | `POST` bodies `{"bounties":5}`, `[]`, malformed JSON, no body | 400 envelope with payload `{}`; 400 envelope; 400 page; 400 envelope (`body must be an object`) |
| R10 | `POST /bounty/INVALID` as `gs`; `GET /bounty/INVALID` | 200 empty state payload, nothing stored |
| R11 | `POST /bounty/delete/player-1` naming a claimed base bounty and a weekly one | the base gone, the weekly kept |
| R12 | `POST /bounty/delete/player-1` with no body; with `{}`; as a player | 200 payload `{}`; 200 payload `{}`; empty 403 |
| R13 | `PUT /cooldown/batch/player-1` as a player; as `gs` with two entries | empty 403; 200 map of both |
| R14 | `GET /cooldown/player-1` as player-1 / player-2 | the map / empty 403 |
| R15 | `PUT /cooldown/batch/player-1` with `{"cooldowns":[{"cooldown_id":""}]}`; on `INVALID` | 400 envelope; 200 payload `{}` |
| R16 | `PUT /cooldown/player-1/Sprout_1` as `gs`; `POST` with a body naming an old date | 200, the date is now (the body ignored) |
| R17 | start with id `bad id`; on `INVALID`; as a player; persistence off | 400; 400; empty 403; 503 envelope `Service Unavailable` |
| R18 | `POST /cooldown/batch/player-1` | empty 404, even without a credential |
| R19 | `GET /entitlementsv2` with none; `POST /entitlementv2/x` | `{"entitlements":[]}`; `{"code":null,"message":"OK","payload":[]}` |
| R20 | `GET /balance` as a new player | 200, the 52-key base with `CURRENCY_NOTES`/`id_currency_notes` 5000 and the starter currencies under both names appended; the wallet is now stored |
| R21 | `GET /balance` as `gs` with no configured account | the base with notes 0 |
| R22 | `POST /reconcile` | `{"balances":{..},"refreshInventory":true}` with the wallet's ids in both forms only |
| R23 | `GET /creator` | §3.13's body |
| R24 | `/product/skus/public` without a tag, with `requiredTags=`, with two `requiredTags` | 400 `missing requiredTags query parameter` each |
| R25 | `requiredTags=nothing` | 200 `[]` |
| R26 | `ladyluckstore`, `webstore`, `dyes`, `season_store`, `exchange_vendor_slot_2`, `fountain_daily_free_bundle` | each section's listings (§4.11), Platinum prices 0 by default |
| R27 | `GET /token/x/<sku id>`; `/token/x/<an item catalog id of one SKU>`; an unknown id; a fountain id | 200 token whose claims name the SKU and its section; the same; 404 `unknown_sku`; 200 with `storeTag` `fountain_daily_free_bundle` |
| R28 | `POST /notification/x` without `token`; with a bad token; with player-2's token as player-1 | 400 `missing token`; 401 `invalid_token`; 403 `token_user_mismatch` |
| R29 | a one-time Lady Luck SKU bought with enough steel marks; bought again | 204 and the marks debited once; 204 again, no second debit |
| R30 | a SKU priced above the balance | 402 `insufficient_balance` |
| R31 | a repeatable SKU twice | two debits, two grants |
| R32 | the fountain claimed; listed again; listed for player-2; listed for `gs` | 204; `[]`; the offer; `[]` |
| R33 | an exchange slot bought (Platinum free) | 204, the entitlement `exchange_slot_2` held, no debit |
| R34 | `PLATINUM_STORE_FREE=0`, an exchange slot with 1000 Platinum split as 25 CS, 50 UNIV, rest PLATFORM | the buckets drawn CS, UNIV, then PLATFORM; the balance total drops by 1000 |
| R35 | an unrouted path | empty 404 |

**Bounty rules**: authority table of §4.1 (game server, host runtime with and without the target in its roster, player
self and other); groups by id; a daily draft replaces the previous daily and leaves season challenges alone; a second
base bounty keeps the first; a new `drafted_timestamp` is a new instance (claim and credit cleared); an omitted
objective is kept; an empty `bounties` with draft data updates the draft and deletes nothing; the cap drops the oldest
claimed first; a reseed keeps a claimed and credited challenge so it cannot pay twice; the day fields survive saves;
newly claimed is reported once per claim and again after a redraft; the host summary's key order and its 24-id cap.

**Host bounty rules** (clock pinned inside a window, and at a roll): every real weekly id shape parses and the
week/slot/index must agree; non-canonical or out-of-range ids do not; accepted stamps are the current schedule plus the
previous one for exactly 2 hours (2 h − 1 ms accepted, 2 h refused); a seed on schedule stores as a dedicated
server's; a claimed instance cannot be restamped or dropped by omission; a refused entry leaves the rest of the save
applied; entries are judged in order; one instant in two notations is one instance; a stale server's stamp cannot
bring back a claim after a roll; invented weekly ids are not stored; a `Challenge_Daily_*` id under the weekly key is
filed daily; 110 per window; a flood of new bounties cannot evict claimed weekly instances; daily and base merge as for
a dedicated server; progress above 100,000,000 is clamped and counted.

**Deletes**: by id or bounty object, deduplicated; season challenges never; `delete_claimed_bounties`; must name
something; a body that names nothing is the reset.

**Rewards**: the coin up to 1000 is a grant, 1001, 0, NaN, another id and no table are not; each claimed challenge
funds exactly one reward, newest `claimedAt` first; a daily claim funds at most one reward per game day from 17:00 UTC
(the second is "daily limited"); a version race is "not funded".

**Cooldowns**: §4.6.1's errors; a bare array body; a later batch replaces only its ids; `__proto__` as plain data;
gatherable pruning at 48 h, never for ids the save writes; the host table of §4.6.3 row by row (add, repeat, raise,
earlier kept, never deleted, duplicates in order, flood refused at 1024, lists capped at 20 with exact counts, an
unparseable stored date replaced, a season id limited by the window end and following the roll, a far-future date
already stored written back is not refused, a refused new id costs no room); the dedicated merge has no future limit;
the start id rule.

**Store**: the daily window at 16:59:59.999 and 17:00; the one-time transaction id is 32 upper-case hex and stable;
the instance id 26 characters; a steel price is a removal of `CURRENCY_MARKS_STEEL`; Platinum free unless `0`; a
negative or string price refuses; `webstore` and `dyes` share one scope; Lady Luck's scope strings; a daily id
changes at 17:00 and not within the window; the lookup's exact-id-first, unique catalog id, one dye in two sections
not ambiguous, two offers with one item ambiguous; the fountain file validated fail-closed (each problem row), a
missing file leaves the section empty, a valid bundle reachable only by its own id, an id clash disables it; a version
serves only its sections.

**Wallet and Platinum**: the UNIV bucket shown as the aggregate; the aggregate sums every bucket; no Platinum keeps the
base value; a spend takes promotional buckets first and spills into paid ones only when they are gone; an exact spend
leaves 0; an overdraw is refused whole; the 13 ids are Platinum and look-alikes are not; merge into stacked replaces
and appends; an unsafe catalog id is refused before a wallet exists.

**Economy ledger**: the settings defaults, off values and bad thresholds; Hunt Pass ids from pass ranks and every
track's prestige; the fixed ids and patterns; credit sources; `for=` with 8 items then `+n more`; a log token quoted
when it could forge a line; a flag fires only on the crossing write; no ledger when off; the exact credit, spend and
reward lines; a store spend names its SKU; a host spend names session and host; nothing for an empty ledger or
`OPERATOR_TEST`.

### 12.3 Test support (feature `test-support`)

A module over a temporary store with the bounties part registered, a fake game auth (the three identities of §12.2),
fakes for `BountiesSeams`, `RingInstances` and the `Characters` port, a store token signer on throwaway keys, and a writer of small synthetic store files into a
temporary game data folder (a Lady Luck SKU or two, a Reward Cache SKU, Platinum SKUs including a dye in two sections,
a fountain bundle, a storage policy and a progression config naming one prestige reward). No captured or generated
game data.

---

## 13. Security behaviour (RX3)

The trust boundary components here are the game server's and host's state writes, the store's money path and the
data files. Its checklist is this part's section of `docs/security/rustic-isles-economy.md` (shared with the other two
parts).

- **Authority**: bounty and cooldown writes only from a game server or, under the host rules and rifts' host-write
  check, a player-host runtime; reads only for the caller's own `userId`, a game server, or a runtime's roster. A
  player can never unclaim, re-date or restart a once-per-period gate.
- **Host limits**: weekly stamps only on the schedule, forward only, 110 per window, claimed instances protected from
  omission and the cap; cooldowns only later, at most 48 h ahead (season ids: the window end), never deleted; progress
  clamped at 100,000,000. Each limit refuses one entry, never the save.
- **Bounded input**: 512 bounties and 256 cooldowns per save, ids 200 UTF-16 units, dates 64; 1000 stored bounties,
  1024 cooldowns; the body limit of core §3.9.
- **Money path**: the price comes from the server's SKU, never from the request (the `currency` segment and any body
  are ignored); a purchase is bound to a signed, 10-minute token whose `userId` must be the caller; prices must be
  finite and ≥ 0; a debit never goes below zero; Platinum is all buckets or nothing; one transaction for everything;
  one-time and daily ids derived, so replays never pay or grant twice; the daily claim fails closed.
- **Data files**: read only from the configured folder by fixed names; a fountain offer is fail-closed (stacked,
  uncapped, not prestige-gated, free, id-safe); a store id clash disables the section.
- **Identifiers into rows**: entitlement names and balance ids are checked against the store's safe-identifier rules
  before any write.
- **Logs**: no token or key logged; the start line and the economy lines quote any value that is not a plain token.
  **Weak spot to list**: the bounty and cooldown load/save lines and the cross-account refusal lines print the path
  `userId` bare, so a percent-encoded CR/LF can forge a log line (§14 item 10).
- **Weak spots to list**: a delete clears the daily reward day (§14 item 3); a one-time SKU is one-time per character
  (§14 item 4); a game server with a configured bearer-less account can buy for that account (§14 item 11).

**Fuzz targets (`fuzz/fuzz_targets/`)**:

- `rustic_isles_economy_bounties`: an arbitrary JSON body and an arbitrary stored record through validation, the
  dedicated merge, the host plan and merge (any clock), and the delete. Never panics; a validated save never yields
  more than 1000 entries; no claimed entry becomes unclaimed; `update_version` strictly grows; a host merge never
  drops a protected entry and never stores a weekly id that does not parse; the payload always has its six keys.
- `rustic_isles_economy_cooldowns`: arbitrary bodies and stored entries through validation and both merges. Never
  panics; at most 1024 entries; a host merge never moves a stored parseable date earlier, never removes an id, never
  accepts a date past the latest allowed unless it equals the stored one.
- `rustic_isles_economy_store_data`: arbitrary JSON as each store file and as the fountain file, then the catalog, the
  lookup of arbitrary text, the listing and the purchase plan. Never panics; a plan's removes are never negative; an
  accepted fountain offer always passes §4.9.4; the listing always has its keys in §4.11's order.
- `rustic_isles_economy_text`: the weekly id parser, the season cooldown id, the cooldown id rule, the query reader and
  the log token. Never panics; a parsed weekly id rebuilds to the same text; a plain log token never contains a
  control character.

---

## 14. Open questions and behaviour that looks wrong but is kept

1. **Host runtime in auth's `AuthData`** (seam, settled). Host writes need the `rifts_seam::RuntimeAuth` the realms
   seam authenticated (the guard's `authorize_host_write` and `record_host_write` take it). Per the inventory spec's
   decision (item 2 of its closing decisions): `rustic-isles-auth`'s `RuntimeCaller` carries an opaque handle
   (`Option<Arc<dyn Any + Send + Sync>>`, added by the characters unit) that the realms seam fills; this part
   downcasts it to `RuntimeAuth`. A handle that is missing or of another type means no host-write authority: the
   writer check treats the caller as a host runtime but the host-write check answers empty 403.
2. **Persistence switches are not trimmed** (` off ` is on) and an empty value is on. Parity.
3. **A delete clears `daily_reward_credited_on`** (and the premium day), so a daily challenge claimed after a delete
   can fund a second daily reward in the same game day. Node's delete record. Parity; a candidate fix row (carry both
   fields as a save does).
4. **One-time SKUs are one-time per character**: the ledger key includes the character, and only the daily section
   checks other characters. With one slayer per account today it does not show. Parity; a candidate fix row.
5. **Clocks and ids outside the replay**: the repeatable purchase id is system random, the fundable-claim wait and the
   ledger's commit time are the system clock, and a token's expiry is checked on the system clock while its `iat`
   comes from the replay clock (a pinned past clock mints expired tokens). Parity; the replay goldens do not cover
   repeatable purchases.
6. **Unmapped store currencies** (`id_currency_s20_coin`, anything outside §4.12's map) upper-case to `ID_CURRENCY_...`,
   which the wallet does not treat as a currency, so inventory tries a stacked item removal and the purchase fails
   (`transaction_failed`). No shipped file prices in one. Parity.
7. **The `STORE` scope's SKU id never reaches the ledger**: I1 passes `store_sku_id`, but inventory's
   `CommittedTransaction` (W9's input) carries only the source, so a store spend's `for=` lists the granted items, not
   the SKU. The `store_sku_id` branch of §4.14 is reachable only by a direct ledger caller. Parity; a candidate fix
   (add the SKU id to `CommittedTransaction`) needs both parts.
8. **`spend_platinum`'s report** lists the `CURRENCY_PLATINUM` bucket and then the aggregate under the same id. A map
   keeps the aggregate. Parity.
9. **Weekly ids**: `season` and `season2` are not compared. Parity.
10. **Bare `userId` in logs** (§13). Proposed: quote non-plain values as the start line does (a log-only change, not
    on the wire). Recommended as a security departure; parity unless the coordinator agrees.
11. **A bearer-less game server** has the configured account as its `user_id` (core §5.3), so it reads that account's
    wallet and entitlements and can mint and redeem tokens for it. Parity.
12. **`POST /cooldown/batch/<x>`** is an empty 404 before auth; `POST /cooldown/BATCH/<x>` is not caught (the guard is
    case-sensitive) and starts a cooldown named `<x>` for user `BATCH`. Parity.
13. **The ring forward's receiver** (`bounty.write` at the home) appends a raw base entry without the merge, the
    version guard or the host rules. Not this part's code; listed so the Reforged unit decides.
14. **`SEASONAL_COIN_DAILY_FLAG` has two defaults**: 1000 here, 20,000 in the Reforged activity monitor, read from the
    same setting. Parity; the Reforged unit should note it.
15. **The catalog never refreshes** until restart; a regenerated store file is not seen. Parity (Node reads at start).
16. **`GET /token` may create a character** (the first-character call creates one for an account with none), and
    `GET /balance` creates the wallet: GETs that write. Parity.
17. **A path `userId` that is a global account id of this node** is looked up as a `user_id` text and finds nothing, so
    a game server's save under it is answered 200 and not stored. Parity.
18. **`POST /bounty/delete`** (two segments) is `POST /bounty/:userId` with `userId` `delete`. Parity (route table).
19. **`daily_premium_draft_granted_on`** is only carried (set by imported data), and `holds_entitlement` has no caller.
    Kept for parity and for the Hunt Pass Premium check a later row may add.
20. **The 401 for a missing `AuthData`** is unreachable once game auth runs in front of every route; kept as the
    answer if a composer ever mounts a handler without it.

---

## Coordinator decisions (2026-11-05, before implementation)

These settle §14 and the mismatches and override anything above that disagrees.

- **W9:** this spec's `commit_transaction(shared, &CommittedTransaction)` is the contract; the inventory part calls it
  as written here.
- **`find_claim`:** this part's own read, as written.
- **SKU id in the ledger:** parity (the `for=` names the granted items).
- **Missing or mistyped runtime handle on a host write:** empty 403, as written.
- **Progression names (§6.3):** reconciled with the progression spec when it lands; where they differ, the progression
  spec's names win and this part calls them.
- **Log quoting: taken** (a security departure; logs are not a client contract): client text in bounty and cooldown
  log lines is quoted/escaped so CR/LF cannot forge a line.
- **Daily reward after a delete; one-time SKUs per character:** parity; candidate fix rows for the maintainer.
- **Everything else in §14:** parity, as listed.
