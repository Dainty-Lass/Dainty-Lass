<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: a Ramsgate Reborn save, as data (RD27)

This page states, as facts, what a Ramsgate Reborn deployment (a Mystic Paradox port to Dauntless 1.12.0) keeps in
MongoDB for one player, and what its offline component's `.rrsave` file holds, so that an MIT importer can copy a
player's save into Dainty Lass (1.14.7) on demand: when a player first logs in with no save, the importer finds that
player's Ramsgate account (by Discord user id, or by a one-time claim code for a player without Discord) and copies
what maps to 1.14.7. The implementer works from this page alone and never opens the Ramsgate sources.

Everything below is stated from observable contracts: collection names, field names and types, id formats, key
formats, indexes, invariants and the file format. No code is reproduced and no private helper is named. Quoted text in
`code` is exact. "1.14.7 collection" means the Mystic Paradox 1.14.7 collection of the same role as listed in
[`spec-rustic-isles-db.md`](spec-rustic-isles-db.md) section 7.4 (the "target spec"); "RR" means Ramsgate Reborn.

---

## 1. Provenance

| Fact | Value |
| --- | --- |
| Artifact | The RR offline component (RR's own item number RR-38), extracted from the RR launcher installer |
| Corresponding source named by the artifact | the `ZFXSTATIC/ramsgate-reborn` repository, commit `19e8c766e04b30769c327ba099dd06d06e6f8360` |
| Bundles built | `2026-11-19T01:16:10.501Z` (build manifest); component manifest `2026-11-19T01:16:14Z` |
| Backend package | `paradox-backend` version `0.0.3`, bundled from `RamsgateBackend/src/**` (144 inputs) |
| Licence | AGPL-3.0-only with additional terms (the bundle's `LICENSE`, `NOTICE.md`, `ADDITIONAL_TERMS.md`) |
| MongoDB Node driver | `mongodb` 7.5.0 (the bundle's `package.json` and `node_modules/mongodb`) |
| MongoDB server shipped offline | MongoDB Community Server 8.0.34, unmodified |
| Game build | Dauntless 1.12.0; default changelist `392819` (the offline tool's default; the backend reads `TARGET_CHANGELIST`) |
| Starter bootstrap marker | `starter-1.12-v2` (see `bootstrapVersion`, section 2) |

**Connection settings (environment, read by the backend).** `MONGODB_URI` is required; the database name is
`MONGODB_DB`, default **`mysticparadox`** on a live server; the offline stack always uses **`rr_offline`** (it refuses
any other name) on `127.0.0.1:39302`, replica set `rrOffline`, direct connection. Client app name default
`paradox-backend` (`rr-offline-backend` offline). Pool and timeout knobs (`MONGODB_CONNECT_TIMEOUT_MS`,
`MONGODB_SERVER_SELECTION_TIMEOUT_MS`, `MONGODB_MAX_POOL_SIZE`, `MONGODB_MAX_IDLE_TIME_MS`) do not affect the data.
Multi-document transactions are used, so a live deployment is a replica set.

**What the bundle does not contain.** The offline build strips the live-only parts: the launcher's Discord sign-in and
account routes, the admin console's routes, the snapshot *export* (the live side that writes `.rrsave`), and the
Dauntless Revived claim flow (`drClaims`). Their collections are known from the repositories and the delete-player
routine, which are bundled; the exact values those live routes write (for example the `provider` string for Discord)
are section 8's open questions.

**Boot-time data migrations the backend runs on every start** (so a dump taken from a running server has had them):
a `wallets` document whose `balances` is a JSON string is rewritten to the parsed object (an unparsable string becomes
`{}`); an `accounts` document that has `displayNameNormalized` but no `name` gets `name` = its `displayName` and
`notes` = 0. There is no schema-version marker anywhere (section 7).

---

## 2. Collections

Scope: **A** per account (keyed or filtered by the user id), **C** per character (keyed by character id, also carrying
the user id), **G** global. Verdict: **carry** (player save data the importer copies when it maps), **read** (read to
identify the player, never copied), **skip** (credentials, sessions, logs, bookkeeping or other players' data).

Unless stated, every time written by the backend is an ISO 8601 UTC string with milliseconds and `Z`
(`2026-11-19T01:16:10.501Z`); "Date" means a native BSON date; "JSON text" means a string holding JSON (section 7.3).

### 2.1 Summary

| Collection | Scope | `_id` | Verdict | 1.14.7 counterpart |
| --- | :-: | --- | :-: | --- |
| `accounts` | A | user id | read (+ display name) | `accounts`: same role, different field set (2.2) |
| `authIdentities` | A | ObjectId | read | `authIdentities`: same shape |
| `userApiKeys` | A | user id | skip | `userApiKeys`: same shape, different hash |
| `gameServerApiKeys` | G | ObjectId | skip | `gameServerApiKeys`: same |
| `refreshSessions` | A | session id | skip | `refreshSessions`: same shape |
| `gameExchangeCodes` | A | code hash | skip | `gameExchangeCodes`: fewer fields |
| `discordOAuthTransactions` | A | state | skip | `discordOAuthTransactions`: fewer fields |
| `adminSessions` | A | session id | skip | `adminSessions`: same role |
| `adminAudit` | G | audit id | skip | `adminAudit`: same role |
| `characters` | C | character id | carry | `characters`: no `ord`, different date text |
| `inventories` | C | character id | carry | `inventories`: JSON text, different item shape |
| `loadouts` | C | character id | carry | `loadouts`: same fields, 1.12 slot layout |
| `wallets` | A | user id | carry | `wallets`: same shape |
| `playerJourney` | A | user id | carry | `playerJourney`: no `revision` |
| `breadcrumbs` | C | character id | carry | `breadcrumbs`: same |
| `encounteredContent` | C | character id | carry | `encounteredContent`: same |
| `progressionTracks` | A | `<user id>::<progression id>` | carry | `progressionTracks`: no `reforge`, `prestigeClaimed` |
| `progressionObjectives` | A | `<user id>::<objective id>` | carry | `progressionObjectives`: same |
| `escalationProgress` | A | `<user id>::<season>` | carry | `escalationProgress`: different shape |
| `bounties` | A | `<user id>::<bounty id>` | carry | RR-only; folds into `bountyStates.bounties` |
| `bountyDrafts` | A | user id | carry | RR-only; folds into `bountyStates.draftData` |
| `cooldowns` | A | `<user id>::<cooldown id>` | carry | RR-only; folds into `cooldownStates.entries` |
| `trackedObjectives` | A | ObjectId | carry if a home exists | RR-only; none in the target spec |
| `playtime` | A | user id | carry if a home exists | RR-only; none in the target spec |
| `progressionObjectiveEvents` | A | ObjectId | skip | `progressionObjectiveEvents`: same (a log) |
| `inventoryTransactions` | C | SHA-256 hex | skip | `inventoryTransactions`: different `_id` |
| `friendships` | A | `<owner>:<other>` | skip | `friendships`: same shape |
| `patrolKeyRolls` | A | roll key | skip | RR-only ledger |
| `coinRuleState` | A | prefixed key | skip | RR-only, expires after 2 days |
| `trialruns` | G | run key | skip | RR-only leaderboard |
| `progressionTracks_backup` | A | `<track id>::<time>` | skip | RR-only backups |
| `deletedAccountBackups` | A | `<user id>:<epoch ms>` | skip | RR-only backups |
| `offlineImportBackups` | A | `<user id>::<time>` | skip | RR-only (offline database only) |
| `drClaims` | A | unknown | skip | RR-only, live-only (Revived claims) |

Not in RR at all (present in 1.14.7): `progressionTransactions`, `entitlements`, `bountyStates`, `cooldownStates`,
`economyDaily`, `emailCodes`, `partyMemberships`, `parties`, `partyInvites`, `partyOutbox`, and every collection of the
target spec's `reforged` area. RR keeps parties in memory only.

**Entitlements are not stored.** RR answers every player the same fixed entitlement list (season and vault premium
passes, Slayers Club, the pass entitlements named in `progression_config.json`), unless the host overrides the list
with the `ENTITLEMENTS_DEFAULT` setting. There is nothing per player to carry; the importer grants 1.14.7's own
defaults.

### 2.2 `accounts` (A, read)

One document per account; `_id` = `userId` (the user id, a string). Two kinds share the collection:

- a **launcher account** has the key `displayNameNormalized` (the backend's own test for "is a launcher account");
- a **metagame-only account** has only `_id`, `userId`, `name`, `notes` (made on first game login when no launcher
  account exists; in practice a development path).

| Field | Type | Meaning |
| --- | --- | --- |
| `userId` | string | the user id, equal to `_id` |
| `name` | string | the in-game name; kept equal to `displayName` for launcher accounts |
| `notes` | number | always 0 |
| `email` | string, absent | normalized (lower-case) e-mail; unique among documents where it is a string |
| `displayName` | string | the launcher username as typed |
| `displayNameNormalized` | string | `displayName` lower-cased; unique among documents where it is a string |
| `passwordHash` | string, absent | launcher password hash (format not visible in the bundle); never carried |
| `status` | string | `active`, `banned` or `disabled` |
| `approvalStatus` | string, absent | `pending`, `approved` or `rejected`; **absent means approved** |
| `approvalUpdatedAt`, `approvalUpdatedBy`, `approvalReason` | string | last approval change; reason may be `""` |
| `roles` | array of strings, absent | **absent reads as `["player"]`** (1.14.7's default is `["user"]`) |
| `createdAt` | string | ISO time |
| `lastLoginAt` | string, absent | ISO time |
| `usernameSet` | boolean, absent | `false` means the player must still choose a username |
| `bannedAt`, `bannedBy`, `banReason` | string | present while `status` is `banned` |
| `banUntil` | string, absent | ISO time; absent while banned means a permanent ban |
| `origin` | object, absent | offline profiles only (section 6.4) |

Indexes: unique partial on `email` (where a string), unique partial on `displayNameNormalized` (where a string).

Against 1.14.7 `accounts`: same `_id`/`userId`/`name`/`notes` core and the same launcher fields up to `usernameSet`;
RR has **no** consent fields, deletion-request fields, `rolesUpdatedAt`/`rolesUpdatedBy`, e-mail verification fields
or nested `approval`; RR **adds** the four ban fields, where 1.14.7 has `timeout_*` (target spec: not migrated).

### 2.3 `authIdentities` (A, read)

A link from an external sign-in to an account. No custom `_id` (ObjectId).

| Field | Type | Meaning |
| --- | --- | --- |
| `provider` | string | the sign-in provider (Discord's value: section 8, Q1) |
| `providerSubject` | string | the provider's user id; for Discord, the Discord user id (snowflake) |
| `userId` | string | the RR user id it signs in as |
| `providerUsername` | string | the provider's username at link time |
| `providerAvatarUrl` | string, absent | avatar URL |
| `linkedAt` | string | ISO time |

Indexes: unique on (`provider`, `providerSubject`); on `userId`. Same shape as 1.14.7 (the target spec reads
`providerSubject|subject`, `providerUsername|username`, `linkedAt|createdAt`; RR writes only the first of each).

### 2.4 Credentials and sessions (all skip)

| Collection | Fields | Notes |
| --- | --- | --- |
| `userApiKeys` | `_id` = `userId`, `userId`, `keyHash` | one per account. `keyHash` is lower-case hex of HMAC-SHA-256 keyed by the server's `API_KEY_HASH_SECRET` over the scope `user`, a NUL byte, then the key; useless without that secret and never carried. Index on `keyHash` |
| `gameServerApiKeys` | ObjectId, `keyHash` | the same HMAC with scope `gameserver`; replaced from the host's configured keys at every start |
| `refreshSessions` | `_id` = session id, `tokenHash`, `userId`, `familyId`, `deviceId`, `deviceName`, `createdAt`, `expiresAt`, `revokedAt` (absent or ISO), `ttlAt` Date | TTL on `ttlAt`. Indexes: unique `tokenHash`; `familyId`; `userId` |
| `gameExchangeCodes` | `_id` = `codeHash`, `codeHash`, `userId`, `launcherSessionId`, `buildChangelist` (number), `executableSha256`, `createdAt`, `expiresAt`, `consumedAt`, `revoked` (boolean, absent), `ttlAt` Date | one-time game-login codes. Offline codes: SHA-256 hex of a 64-hex-char code, `launcherSessionId` `rr-offline`, `executableSha256` `""`, two-minute life. No runtime-hash fields (1.14.7 has five more) |
| `discordOAuthTransactions` | `_id` = `state`, `state`, `codeVerifier`, `createdAt`, `expiresAt`, `consumedAt`, `completionCodeHash`, `completionExpiresAt`, `completionConsumedAt`, `userId`, `ttlAt` Date | TTL is the expiry plus one hour. No `linkUserId` (1.14.7 has it) |
| `adminSessions` | `_id` = `id`, `tokenHash`, `userId`, `csrfToken`, `createdAt`, `expiresAt`, `ip`, `userAgent`, `revokedAt`, `ttlAt` Date | TTL on `ttlAt`; unique `tokenHash` |
| `adminAudit` | `_id` = `id`, `actorUserId`, `targetUserId`, `action`, `oldState`, `newState`, `reason`, `ip`, `requestId`, `createdAt` | the admin console's log, including the record of a deleted player |

### 2.5 `characters` (C, carry)

`_id` = `characterId`. Index on `userId`.

| Field | Type | Meaning |
| --- | --- | --- |
| `characterId` | string | a random UUID, lower-case with dashes (`3f2b...-...`), equal to `_id` |
| `userId` | string | owning account |
| `name` | string | character name |
| `createdDate`, `lastModifiedDate` | string | **US-English short date, not ISO**: month abbreviation, day, comma, year (`Nov 19, 2026`) |
| `updateVersion` | number | optimistic version; the game sends a higher one to write |
| `data` | JSON text | the game's character blob, an object (section 5.5) |

Against 1.14.7: no `ord`, no `createdAt`/`updatedAt`; the date strings are not times (the target spec's time reader
keeps unparsable text as it is, section 7.2). The backend creates one character on an account's first character
request and serialises concurrent creations, so one character per account is the norm; nothing enforces it (section 7).

### 2.6 `inventories` (C, carry)

`_id` = `characterId`. Index on `userId`.

| Field | Type | Meaning |
| --- | --- | --- |
| `characterId`, `userId` | string | owner |
| `instancedItems` | JSON text | array of instanced items (section 5.2) |
| `stackedItems` | JSON text | array of `{catalogId, quantity}` |
| `revision` | number, absent | optimistic version; absent reads 0 |
| `bootstrapVersion` | string, absent | `starter-1.12-v2` when the starter kit made it |

Against 1.14.7: same field names; RR always stores the two lists as **JSON text** (1.14.7 may hold arrays: the target
spec reads either); the **item shape differs** (section 5.2).

### 2.7 `loadouts` (C, carry)

`_id` = `characterId`. Index on `userId`.

| Field | Type | Meaning |
| --- | --- | --- |
| `characterId`, `userId` | string | owner |
| `loadouts` | JSON text | array of loadout slots (section 5.3) |
| `persistent` | JSON text | the character's persistent cosmetics object (section 5.3) |
| `unlockedTotalSlots` | integer | slots the player has unlocked, 1 to 6; written 1 on creation |
| `activeIndex` | integer, absent | selected slot |
| `revision` | number, absent | optimistic version |
| `bootstrapVersion` | string, absent | as for inventories |

Same field set as 1.14.7. Invariants RR enforces on writes: 1 ≤ `unlockedTotalSlots` ≤ 6 and
`unlockedTotalSlots` ≤ the number of stored slots. A document written before `unlockedTotalSlots` existed reads its
visible count as max(1, stored slots − 1).

### 2.8 `wallets` (A, carry)

`_id` = `userId`; fields `userId`, `balances` (object: currency catalog id → number), `bootstrapVersion` (string,
absent). A balance key matches 1 to 128 characters of `A-Z a-z 0-9 _`. Balances are never driven below 0. A new
wallet starts with `CURRENCY_NOTES` 5000 and 50 each of `CURRENCY_PJM_WEAPON`, `CURRENCY_PJM_ARMOR`,
`CURRENCY_PJM_ALCHEMY`, `CURRENCY_PJM_AIRSHIP`, `CURRENCY_PJM_FISHING`. Same shape as 1.14.7. The wallet is the
authority for currencies: the inventory replies merge it into the stacked list, but stored `stackedItems` are not the
source of a currency amount.

### 2.9 `playerJourney` (A, carry)

`_id` = `userId`; fields `userId`, `nodes` (JSON text: an object of node id → `{node_id, node_status, objectives}`),
`updateVersion` (number). This is the Slayer's Path. `node_status` 2 is completed. Against 1.14.7: no `revision`.
A player with no document is served the captured default path (every node not started).

### 2.10 `breadcrumbs`, `encounteredContent` (C, carry)

Both `_id` = `characterId`, with `characterId`, `userId`, index on `userId`.
`breadcrumbs`: `breadcrumbs` (JSON text), `updateVersion` (number). `encounteredContent`: `encounteredcontent` (JSON
text; the key is all lower case). Same as 1.14.7.

### 2.11 `progressionTracks`, `progressionObjectives` (A, carry)

`progressionTracks`: `_id` = `<userId>::<progressionId>`; `userId`, `progressionId`, `progress`,
`confirmedFremiumRank`, `confirmedPremiumRank`, `updateVersion` (numbers; BSON int32 or double), `createdAt`,
`updatedAt`. Progress and ranks only grow, except an admin reset, which moves the document to
`progressionTracks_backup` under `_id` `<track id>::<ISO time>` and deletes it. Against 1.14.7: no `reforge`, no
`prestigeClaimed`.

`progressionObjectives`: `_id` = `<userId>::<objectiveId>`; `userId`, `objectiveId`, `value`, `completedCount`
(numbers), `createdAt` (absent on a document first made by the "if greater" path), `updatedAt`. Same as 1.14.7.

Both indexed on `userId`. `progressionObjectiveEvents` (skip) is the raw log of objective posts: ObjectId, `userId`,
`rawBody`, `receivedAt`; index (`userId`, `receivedAt`).

### 2.12 `escalationProgress` (A, carry)

`_id` = `<userId>::<season>`; `userId`, `season` (string, the route's escalation season), `payload` (object),
`updatedAt`. `payload` holds `escalation_level` (default 1), `next_level_xp` (0), `talents_progress` (array),
`unlock_progress` (array), `update_version` (0), and any other key the game server posted (stored merged over the
previous payload). A missing document reads as those defaults.

Against 1.14.7: **different shape**. 1.14.7 keys by `seasonId` and holds the five values at the top level with
`createdAt`; RR names the season `season`, nests the values under `payload` and has no `createdAt`.

### 2.13 `bounties`, `bountyDrafts` (A, carry)

`bounties`: one document per bounty. `_id` = `<userId>::<bountyId>`; `userId`, `bountyId` (string), `slotIndex`
(integer or null), `updateVersion` (integer), `data` (a native object: the game's bounty element, with `bounty_id`,
`slot_index`, `update_version`, `objectives` as a list of `{objective_id, progress}`, `claimed` boolean,
`drafted_timestamp` and other game keys), `updatedAt`, `rev` (integer, a write counter). The board lists them by
`slotIndex` (null last), then `bountyId`; an element with no string `drafted_timestamp` is given the document's
`updatedAt`. Objective progress and `claimed` never go backwards.

`bountyDrafts`: `_id` = `userId`; `data` (object: `current_draft_choices`, `previous_draft_selections`, `bronze_count`,
`silver_count`, `gold_count`), `updatedAt`.

Against 1.14.7: neither exists; 1.14.7 has one `bountyStates` document per account whose `bounties` is the list of
elements and `draftData` the draft object. The fold is: `bounties` = the `data` of each RR bounty, in board order
(with the `drafted_timestamp` fill above); `draftData` = `bountyDrafts.data`. RR has no daily-credit fields.

### 2.14 `cooldowns` (A, carry)

One document per cooldown. `_id` = `<userId>::<cooldownId>`; `userId`, `cooldownId` (string), `startedDate` (ISO
string), `updatedAt`. A cooldown id of exactly 32 upper-case hex characters is a gatherable, pruned 24 hours after it
started. Against 1.14.7: folds into one `cooldownStates` document whose `entries` maps cooldown id → started date
(the shape RR itself answers the game with). Gatherables older than a day are not worth carrying.

### 2.15 `trackedObjectives`, `playtime` (A, carry if 1.14.7 has a home)

`trackedObjectives`: ObjectId `_id`, found by `userId` (one per account, not unique-indexed); `current_set` (default
`quest_slayer_links`), `omitted_quests`, `tracked_craftables`, `tracked_quests` (arrays), `updatedAt`. The objectives
the player pinned.

`playtime`: `_id` = `userId`; `seconds`, `sessions` (numbers), `lastSessionStart`, `lastSessionEnd` (epoch
milliseconds). RR's own counter, not game state.

Neither has a counterpart in the target spec's section 7. Carrying them is a decision (section 8, Q8).

### 2.16 Bookkeeping (all skip)

| Collection | Facts |
| --- | --- |
| `inventoryTransactions` | idempotency ledger. `_id` = lower-case hex SHA-256 of the JSON array `[userId, characterId, transactionId]`; `transactionId`, `userId`, `characterId`, `requestHash`, `status` (`pending`, `completed`), `result`, `createdAt`, `completedAt`. Index (`userId`, `characterId`). 1.14.7 keys by the three fields instead |
| `friendships` | `_id` = `<ownerId>:<otherId>` (one colon); `ownerId`, `otherId` (user ids), `status` (`ACCEPTED`, `PENDING`), `direction` (`INBOUND`, `OUTBOUND`, null), `created`, `favorite` boolean. Two edges per friendship. Same shape as 1.14.7. Skipped: it names other players, who may never import |
| `patrolKeyRolls` | one roll per kill: `_id` a roll key, `userId`, `result` (includes `capped`), `granted`, `grantTransactionId`, `grantedAt`, `createdAt` (Date). Indexes (`userId`, `createdAt`), (`result`, `createdAt`) |
| `coinRuleState` | `_id` `hunt:<userId>`, `txn:<key>`, `outcome:<userId>`; field `at` Date with a two-day TTL |
| `trialruns` | the trials leaderboard. `_id` `<bucket>:<trialId>:<difficulty>:<runKey>`; `entries` lists players by `phx_account_id` (the user id). Other players' rows |
| `progressionTracks_backup`, `deletedAccountBackups`, `offlineImportBackups` | copies kept before a destructive operation (section 7.1) |
| `drClaims` | live-only; filtered by `userId`; applies a Dauntless Revived import to a new character. Not in the offline build |

---

## 3. Identity

- **User id.** The account's id is `accounts._id` = `accounts.userId`, a string. Every per-account document stores it
  as `userId` (or as `_id`, or as the prefix of a compound `_id`); `friendships` use `ownerId`/`otherId` and the trials
  leaderboard `phx_account_id`. The game sees the user id as its account id (`epicaccountid`, `phx_account_id`). The
  live format is not in the bundle (Q2); the `.rrsave` contract requires at least 8 characters, and its re-keying
  treats the id as case- and dash-insensitive, which fits a UUID. **Offline profiles** are `OFF-` followed by a random
  lower-case UUID; an id with that prefix is never a live account.
- **Character id.** A random UUID; offline imports use `OFFC-` followed by a UUID.
- **Discord user id.** Stored only in `authIdentities.providerSubject`, with `provider` naming Discord and `userId`
  the account. Lookup by Discord id: the one `authIdentities` document with (`provider` = Discord's value,
  `providerSubject` = the Discord id), unique by index, then its `userId`, then `accounts` by `_id`. One Discord id
  maps to at most one account; one account may in principle have several identities (one per provider). The Discord
  username at link time is `providerUsername`. `accounts` holds no Discord field.
- **Display name.** `accounts.displayName` (launcher accounts), else `accounts.name`, else the user id. 1 to 32
  characters offline (letters, digits, space, `_`, `.`, `-`); the live rule is not in the bundle.
- **Bans.** `accounts.status` = `banned` with `bannedAt`, `bannedBy`, `banReason` and optional `banUntil`; the ban is
  in force while `banUntil` is absent, unparsable or in the future. `status` = `disabled` refuses sign-in too.
  `approvalStatus` `pending` or `rejected` refuses sign-in (closed test). `usernameSet` false asks for a username.
- **Roles and admin.** `accounts.roles`, an array of strings, absent = `["player"]`. The admin check is not in the
  bundle (Q3); the offline profiles have `[]`.

---

## 4. One player's save: the query plan

Given the user id `U` (from section 3) the whole save is:

| Collection | Filter | Expect |
| --- | --- | --- |
| `accounts` | `_id` = `U` | 1 (identity, display name, ban state) |
| `characters` | `userId` = `U` | 0 or more (normally 1); their `_id`s are the character set `K` |
| `inventories` | `_id` in `K` | 0 or 1 per character |
| `loadouts` | `_id` in `K` (and `userId` = `U`) | 0 or 1 per character |
| `breadcrumbs`, `encounteredContent` | `_id` in `K` (and `userId` = `U`) | 0 or 1 per character |
| `wallets` | `_id` = `U` | 0 or 1 |
| `playerJourney` | `_id` = `U` | 0 or 1 |
| `progressionTracks`, `progressionObjectives` | `userId` = `U` | 0 or more |
| `escalationProgress` | `userId` = `U` | 0 or more (one per season) |
| `bounties` | `userId` = `U` | 0 or more |
| `bountyDrafts` | `_id` = `U` | 0 or 1 |
| `cooldowns` | `userId` = `U` | 0 or more |
| `trackedObjectives` | `userId` = `U` | 0 or 1 |
| `playtime` | `_id` = `U` | 0 or 1 |

Character-to-account link: `characters.userId`. Character-keyed documents also carry `userId`; RR reads inventories
by `_id` alone but the others by (`_id`, `userId`), so a character-keyed document whose `userId` disagrees with its
character's owner is unreachable in RR and must not be carried. The set above is a superset of RR's own definition of a
save: the `.rrsave` holds exactly the 14 collections `characters`, `inventories`, `loadouts`, `breadcrumbs`,
`encounteredContent`, `wallets`, `playerJourney`, `bountyDrafts`, `progressionTracks`, `progressionObjectives`,
`bounties`, `escalationProgress`, `cooldowns`, `trackedObjectives` (no `accounts`, no `playtime`).

Every compound `_id` uses `::` after the user id, so "owned by `U`" for any RR save document is: `userId` = `U`, or
(no `userId` and `_id` is `U`, starts with `U::`, or is in `K`). A document with a `userId` other than `U` is not `U`'s
even when its `_id` matches.

---

## 5. Game-version specifics (1.12 values)

General rule for the importer: every game value below is identified by a string id; a 1.12 value is kept when its id
exists in the matching 1.14.7 id space (the 1.14.7 catalog, progression config, Slayer's Path or currency set) and is
dropped and reported otherwise. The RR file that defines each 1.12 space is named so the two can be compared offline;
none of those files is copied into Dainty Lass.

### 5.1 Item catalog ids

Upper-case words joined by `_`, the first word naming the kind: `WP_` weapons (`WP_EB_BEGINNER`, `WP_AC_BEGINNER`;
the second word is the weapon type: `EB`, `GA`, `IH`, `CB`, `MS`, `DP`, `AC`), `PART_` weapon parts
(`PART_EB_SPECIAL_DEFAULT`; soul parts `PART_SOUL_<weapon id>`), `AR_` armour (`AR_BEGINNER_CHEST`,
`AR_UNEQUIPPED_HELM`), `LT_` lanterns, `FL_` flasks, `GD_` gliders, `BN_` banners, `BNC_` banner customisations,
`EM_` emotes, `DYE_` dyes, `QI_` quick items and flares, `AC_` accessories, `TITLE_`, `CELL_`, `CURRENCY_`, `TOKEN_`.
Defined by the game; the RR files that list them are `item_kinds.json` (`stackable`: 1807 ids that belong in the
stacked list), `vault_rewards.json`, `webstore_catalog.json`, `ladyluck_store.json`. 1.12 has no full catalog file in
the component, so the comparison is against the 1.14.7 catalog only.

### 5.2 Inventory items

An instanced item is an object with `catalogId`, `instanceId` (26 characters of `A-Z 0-9`, random), `itemData`
(JSON text or null) and `updateVersion` (number), and sometimes `surged` (true, weapons only). For `WP_` items
`itemData` holds `TransmogCatalogID`, `UnlockedAsTransmog`, `DataVersion` (1), `EquippedCells` (array),
`CurrentXP`, `CurrentLevel` (0 to 20); `AR_` items add `Primary`, `Secondary`, `Tertiary` (perk ids or `None`).
Other kinds may have null `itemData`.

1.14.7's instanced item shape is different (the target spec's section 8.3 seeds `grantDate`, `markedNew`,
`craftingTrackId`, `durability`, `currentXp`, `surged`, `talents`); a 1.12 item is translated field by field, not
copied (Q5). The instance id format is the same as 1.14.7's valid id, so ids can be kept.

A stacked item is `{catalogId, quantity}`. RR's own offline import moves any instanced entry whose catalog id is in
`item_kinds.json`'s `stackable` list into the stacked list (quantity 1 if not already there) and fills missing gear
`itemData` fields with the defaults above; a 1.12 save may still hold such misplaced entries.

### 5.3 Loadouts

At most 6 slots: 1 default plus up to 5 unlocked by the Slayer's Path nodes `LoadoutSlot_01` to `LoadoutSlot_05`
(completed = `node_status` 2). The game is told 0 account slots and 1 to 6 character slots, maximum 6. A slot is an
object with `weapon`, `helmet`, `chest`, `arms`, `legs`, `lantern` (each `{item_id, instance_id, instance_data}`:
catalog id, inventory instance id, JSON text or null), `player_role`, `subweapon`, `appearance` (JSON text),
`flask` (catalog id), `quick_items`, `slot_index`, `update_version`, `custom_name`, and a copy of `persistent`.

`persistent` holds `manual_emotes` (6 strings), `intro_emote`, `banner` (a **banner catalog id**, `BN_...`),
`bannerCustomization` (JSON text naming `BNC_` and `DYE_` ids), `flare`, `title`, `head_accessory`,
`back_accessory`, `pet`, `glider` (a **glider catalog id**, `GD_...`), `update_version`, `quick_chats` and `emojis`
(9 strings each), `quick_curiosities_items` (8 of `{item_index, item_id, instance_id}`), `quickwheel`. 1.14.7's
starter writes `banner` and `glider` as instance ids (target spec section 8.3), so those two differ in kind.

### 5.4 Currencies

Wallet keys are `CURRENCY_<NAME>`: `CURRENCY_NOTES` (rams), `CURRENCY_PJM_WEAPON`, `_ARMOR`, `_ALCHEMY`,
`_AIRSHIP`, `_FISHING`, `CURRENCY_CELLDUST`, `CURRENCY_MARKS_STEEL`, `CURRENCY_MARKS_GILDED`,
`CURRENCY_ESCALATION_HEROIC`, `CURRENCY_PRESTIGE`, `CURRENCY_S19_COIN` and other season coins, event currencies
(`CURRENCY_EVENT_FROSTFALL`), `CURRENCY_GAUNTLET_COIN`. Store prices use the lower-case alias
`id_currency_<name>` (`id_currency_notes`); the alias maps to `CURRENCY_` + upper case. Any key containing
`PLATINUM` (any case) is never shown or charged by RR. Defined by the game; RR's files that name them are
`slayers_path_definitions.json`, `webstore_catalog.json`, `ladyluck_store.json`.

### 5.5 Progression, journey, escalation, character data

- **Progression ids** (`progression_config.json`, `payload.paths[].progression_id`, 25 in 1.12):
  `ExperienceTrack_PlayerLevel`, `ExperienceTrack_Weapon_<Axe|ChainBlades|Hammer|Repeaters|Spear|Strikers|Sword>`,
  `MasteryTrack_PlayerLevel`, `MasteryTrack_Behemoth`, `MasteryTrack_Weapon_<same 7>`,
  `PrestigeTrack_Weapon_<same 7>`, and the season pass `season19` (gated by `season19_premium`). RR's offline import
  raises an `ExperienceTrack_Weapon_*` progress of exactly 27774 to 27775 (the rank-20 total), a known 1.12 off-by-one.
- **Journey node ids** (`slayers_path.json`, 466 nodes; costs and children in `slayers_path_definitions.json`):
  `Aetherdrive_Tonic_Unlock`, `Aetherdrive_Tonic_Upgrade_0`, `LoadoutSlot_01`.
- **Escalation seasons**: the season string the game server posts (Q6).
- **Trials ids**: `Arena_MatchmakerHunt_<Easy|Hard|Elite>_<001-181>` (leaderboard only).
- **Character `data`**: an object whose quest series are keys `SERIE_<name>` (`SERIE_cr19_series_1_ftue`,
  `SERIE_cr20_pjm_quests`, `SERIE_dojo`), each value JSON text keyed by 32-hex quest GUIDs with `Status` numbers;
  `LastChangelist` (the build it last saved under, `392819`); appearance and flags. The keys are the game's; 1.14.7
  decides which it reads.

---

## 6. The `.rrsave` file

### 6.1 Container

A player downloads a copy of their online save from the live RR service as a file named `online-save-<1 to 20
digits>.rrsave`; the offline stack refuses another name, a file over 64 MiB, or one inside the offline folder.

| Layer | Content |
| --- | --- |
| File | gzip (RFC 1952), at most 32 MiB (33 554 432 bytes) |
| Decompressed | UTF-8 JSON, at most 128 MiB; one object |

### 6.2 The object

| Key | Type | Rule |
| --- | --- | --- |
| `kind` | string | exactly `rr-offline-snapshot` |
| `schema` | number | exactly 1 |
| `sourceUserId` | string | the live user id, at least 8 characters; refused when it starts with `OFF-` |
| `sourceCharacterIds` | array of strings | the live character ids, each at least 8 characters |
| `displayName` | string | the player's name (RR keeps the first 64 characters) |
| `targetChangelist` | integer | the game build; RR refuses one that differs from its own (`392819`) |
| `collections` | object | keys only from section 4's 14 names; each value an array of documents |
| `sourceRegion` | any, optional | recorded, not checked |
| `exportedAt` | any, optional | recorded, not checked (the live exporter's time) |
| `keyId` | string | names the signing key |
| `signature` | string | base64 Ed25519 signature |

Documents are **MongoDB Extended JSON, canonical mode** (numbers as `{"$numberInt": ...}`, `$numberDouble`,
`$numberLong`; dates `{"$date": {"$numberLong": ...}}`; ObjectIds `{"$oid": ...}`), so BSON types survive.

### 6.3 Signature

The signed bytes are the **canonical JSON** of the whole object without `keyId` and `signature`: object keys sorted by
UTF-16 code unit order, keys whose value is undefined left out, no whitespace, strings and numbers as JSON writes
them; a non-finite number makes the file invalid. The signature is Ed25519 over those UTF-8 bytes. The only key the
offline build trusts is `rr-live-1`, an Ed25519 SPKI public key whose base64 DER is
`MCowBQYDK2VwAyEAF9+W/KlqBV7IbAtR0aYtDpk5O6MJI7QrO+gI5nrAENA=`. Unsigned files are refused. The signature proves
the RR live service exported the save; it does **not** bind the file to whoever presents it (section 7.4).

### 6.4 How the offline stack imports it

1. Decode and check 6.1 to 6.3, then the changelist.
2. Choose the offline profile (the one named, else the oldest `OFF-` account, else a new `OFF-<uuid>`) and give each
   source character a new id `OFFC-<uuid>`.
3. **Re-key**: in every string value and every object key of every document, replace each source id with its new id,
   also in its lower-case, upper-case and dash-less forms (longest first; dash-less forms map to the dash-less new id;
   forms shorter than 8 characters are not used). Then refuse the import if any source id, in any of those forms,
   still appears anywhere (case-insensitively, dashes ignored).
4. Deserialize the Extended JSON; a document whose `_id` is an ObjectId gets a fresh ObjectId.
5. Repair `inventories` and `progressionTracks` (sections 5.2 and 5.5).
6. Refuse the import if any document is not owned by the profile (section 4's rule).
7. In one transaction: copy every existing row of the profile (in `accounts` and the 14 collections) into one
   `offlineImportBackups` document (`_id` `<profile>::<ISO time>`, `userId`, `at`, `backup`: collection → rows) and
   delete them; insert the new rows; insert the account `{_id, userId: profile, displayName, displayNameNormalized
   (lower case), name, status active, approvalStatus approved, usernameSet true, roles [], createdAt, notes 0,
   origin}` where `origin` is `{importedFrom: sourceRegion, exportedAt, signed: true, sourceChangelist, importedAt}`
   (a fresh profile's is `{freshStart: true, createdAt}`).

---

## 7. What makes a copy unsafe or ambiguous

### 7.1 Deletions and duplicates

- **No soft deletes.** Removing a player deletes every row (accounts, credentials, friendships both ways, character-
  and account-keyed rows) in one transaction, after writing a best-effort copy of `characters`, `inventories`,
  `loadouts`, `wallets`, `progressionTracks` and `drClaims` to `deletedAccountBackups` (`_id` `<user>:<epoch ms>`,
  `userId`, `at`, `backup`). Backup collections are not live data and must never be read as a save.
- **Revocation is a field**, not a delete: `refreshSessions.revokedAt`, `gameExchangeCodes.consumedAt`/`revoked`.
- **Several characters** per account are possible (section 2.5); the importer must decide which (Q4).
- **Two accounts for one person** are possible: a launcher account and an orphan metagame-only account, or two launcher
  accounts with different e-mails. Only the `authIdentities` link names the Discord user, so the Discord match is
  unambiguous; a match by display name or e-mail is not.
- **Orphans**: character-keyed rows whose character is gone, or whose `userId` disagrees with the character's (4).

### 7.2 Types and times

- **Ids are strings**, except ObjectIds in `authIdentities`, `gameServerApiKeys`, `progressionObjectiveEvents`,
  `trackedObjectives` (and any document inserted without an `_id`). None of those is a key the importer needs.
- **Times**: mostly ISO strings; native Dates in every `ttlAt`, in `coinRuleState.at` and in `patrolKeyRolls`;
  epoch milliseconds (numbers) in `playtime`; `characters.createdDate`/`lastModifiedDate` are US-English date text
  with no time. Compare and order only after parsing; never as strings across formats.
- **Numbers** come back as BSON int32, int64 or double depending on the write path (`$inc` and `$max` on an int32 keep
  it); read every counter as a number of any BSON numeric type.
- **JSON text vs native**: `characters.data`, `inventories.instancedItems`/`stackedItems`, `loadouts.loadouts`/
  `persistent`, `playerJourney.nodes`, `breadcrumbs.breadcrumbs`, `encounteredContent.encounteredcontent` and nested
  `itemData`, `appearance`, `instance_data`, `bannerCustomization` and quest-series values are JSON text;
  `wallets.balances`, `bounties.data`, `bountyDrafts.data`, `escalationProgress.payload` and `trackedObjectives`
  lists are native. A text
  field that fails to parse is treated by RR as empty (`[]` or `{}`), never as an error.

### 7.3 Versions and markers

There is no schema version on any document and no migrations collection. The only markers are `bootstrapVersion`
(`starter-1.12-v2`, or absent on older rows), `characters.data.LastChangelist`, and `.rrsave`'s `schema` and
`targetChangelist`. `updateVersion`/`revision`/`rev` are optimistic-concurrency counters, not schema versions; they
should restart at 0 in the target unless 1.14.7's rule needs otherwise.

### 7.4 Identity hazards

- The `.rrsave` signature is not bound to the presenting person: a file passed to someone else imports as theirs, and
  its `sourceUserId` names an RR account the importer cannot check without the live database. Accepting `.rrsave`
  for an on-demand claim needs its own proof that the claimant owns `sourceUserId` (for example the RR database's
  Discord link for that id).
- RR user ids and character ids appear inside other values (compound `_id`s, `characters.data`, journey and bounty
  payloads). Any id the importer changes must be re-keyed everywhere, as RR's own import does (6.4 step 3), and
  checked afterwards. Item instance ids are per inventory and need no re-keying.
- A banned or disabled RR account: carrying its save, its ban, or neither is a decision (Q7).
- Offline profiles (`OFF-`) in an `rr_offline` database are local-only saves with no Discord link; they are the
  "player without Discord" case if a host is given such a database.

---

## 8. Open questions

1. **Discord's `provider` value** in `authIdentities` (written by the live launcher sign-in, not bundled). Expected
   `discord`; confirm from a live dump before matching.
2. **Live user id format** (the live account-creation route is not bundled). Section 3's evidence points to a UUID;
   confirm, and whether it is ever upper case.
3. **Admin role names** in `roles` and whether any should carry to Dainty Lass (the admin check is not bundled).
4. **More than one character**: carry the first by `createdDate`, the one with the newest `lastModifiedDate`, or all?
   1.14.7's own limit decides.
5. **1.12 to 1.14.7 item translation**: the 1.14.7 fields with no 1.12 source (`grantDate`, `durability`,
   `talents`...) need defaults; whether `itemData`'s `CurrentLevel`/`CurrentXP` map to 1.14.7's `currentXp` or stay in
   item data is a 1.14.7 fact this page cannot settle.
6. **Escalation season ids**: the set of `season` values 1.12 posts, and whether any is a 1.14.7 season.
7. **Bans**: whether an RR ban (or pending approval) blocks the import or is carried.
8. **`trackedObjectives` and `playtime`**: whether 1.14.7 has a place for them (none in the target spec).
9. **`drClaims`**: its fields (live-only). A pending Dauntless Revived claim in RR is skipped unless Dainty Lass wants
   it; the legacy importer already handles Revived saves directly (`spec-legacy-import.md`).
10. **Live `.rrsave` export**: whether the live service ever exports a schema other than 1, or more collections
    than section 4's 14; and whether `sourceRegion`/`exportedAt` have fixed formats.
11. **Live database name**: the default is `mysticparadox`, but a live RR host may set `MONGODB_DB`; the importer
    should take the name as a setting.
