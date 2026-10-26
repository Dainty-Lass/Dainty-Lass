# Version diff: 1.12.0 to 1.14.7

RU3c (DD8). Every route of [`routes-1.14.7.md`](routes-1.14.7.md) and [`routes-1.12.0.md`](routes-1.12.0.md), compared
handler by handler between the tag `dauntless-1.12.0` (`4d5dfaa`) and `ParadoxBackend`/`ParadoxDirector` as `plan`
holds them (`ce1c42e`'s tree), and each difference turned into what the shared Rust crates carry: nothing (the shared
code serves both), a **table** in the version module, or a **hook** on it. RU4 builds the seams from the summary below;
each RU5 agent takes its group's section.

[`diff-1.12.0-1.14.7.json`](diff-1.12.0-1.14.7.json) has one `{group, method, path, class, hook?, table?, note}` per
1.14.7 isles route (222; 1.12.0's 146 are a strict subset by method and path), plus the non-route differences with
`method: "-"`. Groups and order are RU3a's; RU3b put 26 routes in other groups, noted where it matters.

## How to read it

- **Classes.** `same`: the shared crate answers both. `table`: same code, a per-version value. `hook`: the code
  differs; the hook names the trait method. `1.14.7-only`: the 1.12 module answers as 1.12.0 did, which is the
  unstubbed 404 for every such route. `1.12.0-only`: none; nothing 1.12.0 registers is gone.
- **What `same` covers.** The tag is Mystic's public legacy-1.12 tree: it lacks most of the hardening written in the
  1.12 era (prestige gating, ownership checks, idempotent ledgers, persistence of bounties, cooldowns, escalation and
  parties). Server-side fixes like these change no contract the 1.12 client sees, or change it in the direction the
  1.12 era already wanted, so they are `same` and the 1.12 module inherits them. A change is a hook or table when its
  reason is the 1.14.7 client or data (`[1.14.7 ...]` comments from 2026-10-03 on, 1.14.7 RVAs, 1.14.7 paks) and the
  1.12 answer differs on the wire.
- **New on both.** 43 routes absent from 1.12.0 are `same` with the note "serve on both": launcher, operator and
  backend-facing routes (their consumer is not a game version), plus two the 1.12 client calls and 1.12.0 404'd
  (`POST /escalation/...`, `POST /loadout/.../active/:index`). The other 33 new routes are `1.14.7-only`.
- **Unverified (DD9).** Every 1.12 value below is 1.12.0's answer or the 1.12 setting the code itself names. None has
  been played on 1.12 against the Rust port; a hook's 1.12 side is the place to flip if a 1.12 recording shows the
  1.14.7 behaviour is wanted there too.

## Counts

| Group | same | table | hook | 1.14.7-only | Routes |
| --- | --- | --- | --- | --- | --- |
| auth | 42 | 3 | 2 | 2 | 49 |
| reforged-api | 39 | 5 | 0 | 9 | 53 |
| characters-loadouts | 6 | 0 | 2 | 0 | 8 |
| inventory-reforge | 1 | 0 | 4 | 3 | 8 |
| progression | 15 | 3 | 5 | 2 | 25 |
| bounties-store | 9 | 3 | 1 | 4 | 17 |
| matchmaking-parties | 12 | 0 | 1 | 5 | 18 |
| chat | 9 | 0 | 0 | 2 | 11 |
| game-data | 19 | 2 | 2 | 6 | 29 |
| director | 3 | 1 | 0 | 0 | 4 |
| **all** | 155 | 17 | 17 | 33 | 222 |

## Hooks

Methods on the version module's trait (`trait GameVersion`, name RU4's). Signatures are sketches.

| Hook | Sketch | 1.12.0 | 1.14.7 | Groups |
| --- | --- | --- | --- | --- |
| `no_player_policy` | `fn no_player_policy(&self) -> NoPlayerPolicy` (`Off` / `ConfiguredAccount`) | `Off`: `INVALID` and bearer-less gameserver calls get empty data, 400 or `[]` | `ConfiguredAccount`: `GAMESERVER_NO_PLAYER_USER_ID ?? DEV_USER_ID` replaces `INVALID` path segments of gameserver GET/HEAD and is the userId of bearer-less gameserver calls; empty `characterId`/`accountId` from a gameserver resolve to the account's first character | cross-cutting (auth middleware), characters-loadouts, inventory-reforge |
| `game_session_payload` | `fn game_session_payload(&self, token: &str) -> Value` | `{error_code: null, sessionid: "SESSION_ID_LOL", sessionToken}` | `{error_code: null, sessionid: token, sessiontoken: token, sessionToken: token}`; `/gamesession/epiceos` alias | auth |
| `network_encryption_keys` | `fn network_encryption_keys(&self) -> bool` | false: `POST /key/generate` 400 empty, no `/key/consume` | true: `{key, nonce, token}` issued per token, handed to the hub by `/key/consume` | auth |
| `weapons_at_base_level` | `fn weapons_at_base_level(&self) -> bool` | false: stored `CurrentLevel` | true unless `KEEP_WEAPON_ITEM_LEVELS=true` (1.14.7 paks have no per-level power tables, Reforged 0036) | inventory-reforge |
| `reforge_rules` | `fn reforge_rules(&self) -> bool` (or `MYSTICPARADOX_REFORGE`'s default) | false: XP grants never classified, no stale-reforge repair, reforge routes absent | true: prestige +1 only from an open reforge, banked restore, repair on load | progression, inventory-reforge |
| `mastery_persistence` | `fn mastery_persistence(&self) -> bool` (or `MYSTICPARADOX_MASTERY_PERSISTENCE`'s default) | false: report captured, answer `payload: {}` | true: a gameserver report updates mastery and the answer carries it | progression |
| `huntpass_payload` | `fn huntpass_payload(&self, id: &str) -> Value` | `"season19"` (the payload is the string) | `{huntpass_id: id, season: id}` | progression |
| `objective_row` | `fn objective_row(&self, user: &str, row: &ObjectiveRow) -> Value` | `{objective_id, value, completed_count}` | `{phx_account_id, objective_id, progress, completed_count, created_date, last_modified_date}` | progression |
| `progression_config` | `fn progression_config(&self, file: &Value, now: SystemTime) -> Value` | the file as is | hunt-pass windows rewritten: the active pass a rolling 12-week window (seasonWindow.ts), every other pass ended by 2024-12-03, `RECENT_HUNTPASS_SEASONS` added with 2099 windows (`GAMESERVER_WIDEN_HUNTPASS_WINDOWS=0` off) | game-data |
| `survey_config` | `fn survey_config(&self) -> StubReply` | 400, empty body | 200 `{code: null, message: "OK", payload: {Triggers: []}}` | game-data |
| `entitlements_payload` | `fn entitlements_payload(&self, list: &[Entitlement]) -> Value` | `{code: null, message: "OK", payload: []}` | `{entitlements: [{activatedDate, duration, name}]}` from storage | bounties-store |
| `candidate_destination` | `fn candidate_destination(&self, hunt_id: &str, realm_training: bool) -> (GameMode, Option<String>)` | always `ISLAND` with the hunt id (Ramsgate's too) | Ramsgate `CITY` with `huntId: null`, realm Training `SHARED`, else `ISLAND` | matchmaking-parties |

The four booleans are plain enough to live in the manifest; they are hooks because the code paths behind them differ.

## Tables

| Table | 1.12.0 | 1.14.7 | Used by |
| --- | --- | --- | --- |
| `TARGET_CHANGELIST` | 392819 (hard-coded in launcherUpdates.ts and the Director's runtimeUpdater.ts) | 647472 (`TARGET_CHANGELIST` env; Director constant) | oauth token, launcher game-sessions and status, runtime manifests and publishing, character `LastChangelist`, candidate `buildId`, Director runtime updater |
| `GAME_VERSION` | `"1.12.0"` | `"1.14.7"` | `serverInfo.buildId` = `<CL>_<version>_shipping` |
| `ACTIVE_HUNTPASS_ID` | `season19` | `season43` (`GAMESERVER_HUNTPASS_ID` overrides) | `/huntpass`, default progression tracks, approved XP tracks, `selected_huntpass` alias (skuProgression.ts), window rewrite |
| `RECENT_HUNTPASS_SEASONS` | none | `season43`..`season50` (`GAMESERVER_RECENT_SEASONS`) | `progression_config` |
| `PATCHNOTES_RELEASE_VERSION` | `"1.12.0"` | `""` | `/patchnotes` |
| `BOUNTY_ITEM_GRANT_DATA` | `[]` | `TOKEN_BOUNTY_DRAFT_PREMIUM` x1 and `CURRENCY_PRESTIGE` x5, 2026-01-01..2027-12-31 | `/game_tuning/bounty_game_data` (the daily and weekly keep `[]` on both) |
| `STORE_SECTIONS` | `ladyluckstore` | plus `season_store` (Reward Cache), `webstore`, `dyes`, `hp_level_skip_rank`, `huntpass_store`, `exchange_vendor_slot_2`/`_3`, `fountain_daily_free_bundle` (daily claim) | `/product/skus/public`, `/token`, `/notification` |
| `SEASONAL_COIN` | none (1.12.0 has no funding rule; RU5 to confirm whether 1.12's season coin, `CURRENCY_S19_COIN`, should take the role) | `CURRENCY_SEASONAL_COIN`: season-challenge reward (at most 1,000 a claim), Reward Cache price, season43 prestige payout | `POST /inventory`, store |
| `MASKED_JOURNEY_NODES` | none | `Gauntlet_Unlock` served Locked while `MYSTICPARADOX_GAUNTLET` is not on | `/pjm` reads and saves |
| `STARTER_MANIFEST` | 14 stacked items, `starter-1.12-v2` | plus `TOKEN_DAILY_CHALLENGE_DRAFT` and nine `EJ_*` emojis with a default quick-emoji wheel (1.14.7 catalog), `starter-1.12-v5` | account seeding, starter reconcile |
| `GAME_DATA` | the 1.12 exports | the 1.14.7 exports | `progression_config.json`, `slayers_path.json`, `ladyluck_store.json`, `reward_cache_store.json`, `platinum_store.json`, `inventory_storage_policy.json`, `mastery_objectives.json`, `escalation_tuning.json`, `fountain_daily_bundle.json`; the Director's player and matchmaker hunt tables |
| `CONFIG_DEFAULTS` | QoS listener off (1.12.0 has none) | `QOS_PORT` 80 (1.14.7 QOS FIX) | server listeners |
| `GAMESERVER_ARGS` (Director) | `-EpicPortal -server -nullrhi -warp -RepDriverEnable -LogCmds="LogPhoenixCharacter VeryVerbose, LogOnline Verbose"` | adds `-nosound`; `-LogCmds` adds `LogLoad Off` and takes `GAMESERVER_LOG_CMDS`; spawned with `windowsHide` (`GAMESERVER_WINDOWS_HIDE=0` shows it) | game-server launch |

The positional launch arguments (API key, port, map, behemoth, matchmaker hunt id, expected players, `ip:port`), the
ready marker and the hub maps (`ramsgate_01_persistent`, `training_dojo_persistent`) are the same in both.

## Auth (RU3b also had the runtime-update and admin routes here; RU3a moved them to the Reforged API)

Login, EOS and launcher accounts. The launcher routes are version-independent; the new ones (email codes, password reset, Discord link, policy, legal, notices, consent, deletion) are new on both.

| Route | Class | Hook / table | Note |
| --- | --- | --- | --- |
| `POST /login` | same |  | Reforged session supplies refilled (REFORGED_SESSION_SUPPLIES_FILE); the file names catalog ids of the module's version |
| `PUT /gamesession/epic` | hook | game_session_payload | 1.12.0 {error_code:null, sessionid:'SESSION_ID_LOL', sessionToken}; 1.14.7 sessionid=token, adds sessiontoken (captured epiceos contract) |
| `PUT /gamesession/epiceos` | 1.14.7-only | game_session_payload | alias registered for the 1.14.7 client; 1.12 answers 404 |
| `POST /accountinfo/public` | same |  | DEV_USER_ID default 'mysticparadox' -> 'mystpax' (the alias the 1.12 client sends) |
| `POST /account/api/oauth/token` | table | TARGET_CHANGELIST | LAUNCHER mode checks code build against it; adds runtime channel/hash re-check, APIKEY client_credentials, eligibility 403, realm bind (no-op without realms) |
| `POST /key/generate` | hook | network_encryption_keys | 1.12.0 400 empty (client connects unencrypted); 1.14.7 {key, nonce, token} kept per token for /key/consume |
| `POST /key/consume` | 1.14.7-only | network_encryption_keys | the 1.14.7 hub's key fetch; 1.12 answers 404 |
| `POST /launcher/v1/game-sessions` | table | TARGET_CHANGELIST | version-independent (launcher, operator or backend-facing API, not the game client); buildChangelist must equal it; runtime artifact set checked |
| `GET /launcher/v1/status` | table | TARGET_CHANGELIST | version-independent (launcher, operator or backend-facing API, not the game client); supportedBuildChangelist |

The group's other 40 routes are `same` (the JSON has each).

- **Hooks:** `game_session_payload`, `network_encryption_keys`.
- **Tables:** `TARGET_CHANGELIST`.

## Reforged API (with health, metrics, runtime and launcher updates, and the admin panel)

Nothing here is read by the game client except `/`, `/dauntless-status` and `/maintenance/status`. The admin and launcher-log routes added since 1.12.0 are new on both.

| Route | Class | Hook / table | Note |
| --- | --- | --- | --- |
| `GET /dauntless-status` | same |  | welcome text lost 'v0.0.3' (operator text) |
| `GET /maintenance/status` | 1.14.7-only |  | login-queue-prod: the 1.14.7 client asks it before login (Reforged 0018); 1.12 answers 404 |
| `POST /reforged/api/Register` | 1.14.7-only |  | Reforged API group: not diffed (brief). Launcher-facing, so whether the 1.12 module mounts it is RU4's call |
| `GET /reforged/api/GetUserInfo` | 1.14.7-only |  | Reforged API group: not diffed (brief). Launcher-facing, so whether the 1.12 module mounts it is RU4's call |
| `GET /reforged/api/ServerStatus` | 1.14.7-only |  | Reforged API group: not diffed (brief). Launcher-facing, so whether the 1.12 module mounts it is RU4's call |
| `GET /reforged/api/RegistrationStatus` | 1.14.7-only |  | Reforged API group: not diffed (brief). Launcher-facing, so whether the 1.12 module mounts it is RU4's call |
| `POST /reforged/api/CreateInvite` | 1.14.7-only |  | Reforged API group: not diffed (brief). Launcher-facing, so whether the 1.12 module mounts it is RU4's call |
| `GET /reforged/api/InviteCodes` | 1.14.7-only |  | Reforged API group: not diffed (brief). Launcher-facing, so whether the 1.12 module mounts it is RU4's call |
| `DELETE /reforged/api/InviteCode/:code` | 1.14.7-only |  | Reforged API group: not diffed (brief). Launcher-facing, so whether the 1.12 module mounts it is RU4's call |
| `POST /reforged/api/SetAccountStatus` | 1.14.7-only |  | Reforged API group: not diffed (brief). Launcher-facing, so whether the 1.12 module mounts it is RU4's call |
| `GET /launcher/v1/runtime/:target/:channel/:platform` | table | TARGET_CHANGELIST | version-independent (launcher, operator or backend-facing API, not the game client); manifests checked against the changelist (1.12.0 hard-codes 392819) |
| `GET /launcher/v1/runtime/:channel/:platform` | table | TARGET_CHANGELIST | version-independent (launcher, operator or backend-facing API, not the game client) |
| `GET /launcher/v1/runtime/:target/:channel/:platform/download` | table | TARGET_CHANGELIST | version-independent (launcher, operator or backend-facing API, not the game client) |
| `GET /launcher/v1/runtime/:channel/:platform/download` | table | TARGET_CHANGELIST | version-independent (launcher, operator or backend-facing API, not the game client) |
| `POST /launcher/v1/admin/updates/runtime/:target/:channel/:platform` | table | TARGET_CHANGELIST | version-independent (launcher, operator or backend-facing API, not the game client); x-update-changelist must equal it; artifact ParadoxRuntime-<ver>-<target>-<CL>.dll |

The group's other 38 routes are `same` (the JSON has each).

- **Hooks:** none.
- **Tables:** `TARGET_CHANGELIST`.

## Characters and loadouts

The loadout wire change (default slot reported as a character slot, unlock path value read as a delta) is written for both versions: loadoutSlots.ts gives the 1.12 RVA of the same native code.

| Route | Class | Hook / table | Note |
| --- | --- | --- | --- |
| `GET /character` | hook | no_player_policy, TARGET_CHANGELIST | 1.14.7 resolves a bearer-less gameserver read to GAMESERVER_NO_PLAYER_USER_ID ?? DEV_USER_ID; 1.12.0 answers []. LastChangelist stamped from the table. Dojo series reconcile is same (written for 1.12). GAMESERVER_CHARACTER_* experiment switches are default-off diagnostics: drop |
| `POST /character` | hook | no_player_policy, TARGET_CHANGELIST | 1.14.7 resolves an empty characterId from a gameserver to the account's first character; missing record now 409 not 500 (same). 409 Jetty HTML identical in both |
| `GET /loadout/:userId//all` | same |  | wire slot split as unlock |
| `GET /loadout/:userId/:characterId/all` | same |  | wire slot split as unlock; active_index persisted |
| `POST /loadout/:userId/:characterId/unlock/:numSlots` | same |  | path value is a delta, slots from LoadoutSlot_01..05; wire counts 0/0 account + N/6 character; loadoutSlots.ts says 1.12 shares the contract (1.12 RVA 0x01A6D4F0) |
| `POST /loadout/:userId/:characterId/active/:index` | same |  | absent from 1.12.0 (404 there); serve on both: captured 1.12 traffic uses UpdateLoadoutSlotSetActiveEndpoint |
| `POST /loadout/:userId/:characterId/:index` | same |  | wire slot split as unlock |

The group's other 1 routes are `same` (the JSON has each).

- **Hooks:** `no_player_policy`.
- **Tables:** `TARGET_CHANGELIST`.

## Inventory, storage policy and reforge

The reforge routes in RU3a's progression file are this group's.

| Route | Class | Hook / table | Note |
| --- | --- | --- | --- |
| `POST /^\/inventory\/([^/]*)\/(.+)$/` | 1.14.7-only |  | regex for the 1.14.7 empty-characterId form /inventory//dauntlessrel-1.14.7:647472; same stub answer as the named route, which 1.12 keeps |
| `GET /inventory/:userId/:characterId` | hook | weapons_at_base_level | 1.14.7 serves weapon CurrentLevel 0 (Reforged 0036, KEEP_WEAPON_ITEM_LEVELS=true off); 1.12.0 serves stored levels |
| `GET /inventory/:userId/` | hook | no_player_policy, weapons_at_base_level | 1.14.7 resolves a gameserver read with no characterId to the account's character; 1.12.0 400 |
| `POST /inventory` | hook | no_player_policy, weapons_at_base_level, SEASONAL_COIN, GAME_DATA | 1.14.7 fills empty accountId/characterId for gameservers. Same: grant/spend authority, minted instance ids, stacked-item 409s, storage normalisation (inventory_storage_policy.json from the version's catalog) |
| `POST /inventory/instanceditem` | hook | weapons_at_base_level | response through the base-level filter |
| `DELETE /progression/:userId/:progressionId` | 1.14.7-only | reforge_rules | weapon reforge reset (1.14.7 DeleteProgressionEndpoint) |
| `POST /progression/purchase/:userId/:progressionId/:amount` | 1.14.7-only | reforge_rules | reforge banked re-grant (1.14.7) |

The group's other 1 routes are `same` (the JSON has each).

- **Hooks:** `no_player_policy`, `weapons_at_base_level`, `reforge_rules`.
- **Tables:** `SEASONAL_COIN`, `GAME_DATA` (inventory_storage_policy.json).

## Progression, mastery, escalation and session supplies

Ownership and authority checks on every read and write are `same`.

| Route | Class | Hook / table | Note |
| --- | --- | --- | --- |
| `GET /escalation/:escalationSeason/:userId` | same |  | persisted instead of zeros; same five fields (1.12 serializer) |
| `POST /escalation/:escalationSeason/:userId` | same |  | absent from 1.12.0 (404 there); serve on both: 1.12 hunt servers post here (404 retries in the 2026-07-25 repro) |
| `GET /huntpass/:userId` | hook | huntpass_payload, ACTIVE_HUNTPASS_ID | 1.12.0 payload 'season19' (scalar); 1.14.7 {huntpass_id, season} = season43 |
| `GET /progression/objectives/:userId` | hook | objective_row | 1.12.0 rows {objective_id, value, completed_count}; 1.14.7 {phx_account_id, objective_id, progress, completed_count, created_date, last_modified_date}. GAMESERVER_OBJECTIVES_AS_CONFIG A/B: drop |
| `POST /progression/:userId/:progressionId/:amount` | hook | reforge_rules, ACTIVE_HUNTPASS_ID | approved track list includes the active pass; reforge classification of PrestigeTrack_/ExperienceTrack_ grants (refuses prestige without an open reforge); minted gs- grant id, adds grant_id/replayed (same) |
| `POST /progression/:userId` | hook | mastery_persistence, GAME_DATA | 1.14.7 applies a gameserver's report to mastery (mastery_objectives.json) and answers the mastery payload; 1.12.0 captures and answers {} |
| `POST /progression/:userId/:progressionId/:rank/confirm/public` | same |  | bounded mastery sync |
| `GET /progression/:userId` | hook | reforge_rules, ACTIVE_HUNTPASS_ID | default track list ends with the active pass; 1.14.7 finishes stale reforges on a gameserver load |
| `GET /pjm/:userId` | table | MASKED_JOURNEY_NODES | 1.14.7 serves Gauntlet_Unlock Locked while MYSTICPARADOX_GAUNTLET is off |
| `GET /weapon_tracker/:userId/:itemId` | 1.14.7-only |  | 1.14.7 weapon-tracker loader; 1.12 answers 404 |
| `POST /pjm/:userId` | table | MASKED_JOURNEY_NODES | masked nodes restored before save |
| `GET /pjm` | table | MASKED_JOURNEY_NODES, GAME_DATA | slayers_path.json fallback |
| `GET /progression/:userId/:progressionId` | 1.14.7-only | reforge_rules | single-track read the 1.14.7 reforge uses |

The group's other 12 routes are `same` (the JSON has each).

- **Hooks:** `huntpass_payload`, `objective_row`, `reforge_rules`, `mastery_persistence`.
- **Tables:** `ACTIVE_HUNTPASS_ID`, `MASKED_JOURNEY_NODES`, `GAME_DATA` (progression_config.json, slayers_path.json, mastery_objectives.json).

## Bounties, store, Lady Luck and platinum (RU3b had the cooldowns in progression)

Bounty, cooldown and escalation persistence keep 1.12.0's wire shapes, so they are `same`.

| Route | Class | Hook / table | Note |
| --- | --- | --- | --- |
| `GET /entitlementsv2` | hook | entitlements_payload | 1.12.0 {code:null, message:'OK', payload:[]}; 1.14.7 {entitlements:[{activatedDate, duration, name}]} from storage |
| `GET /cooldown/:userId` | same |  | persisted id->date map; MYSTICPARADOX_COOLDOWN_PERSISTENCE=off is 1.12.0's {} byte for byte |
| `PUT /cooldown/batch/:userId` | same |  | persisted; 403 without gameserver authority; off switch as above |
| `PUT /cooldown/:userId/:cooldownId` | 1.14.7-only |  | 1.14.7 hub sprout harvest (StartCooldownEndpoint) |
| `POST /cooldown/:userId/:cooldownId` | 1.14.7-only |  | as above |
| `GET /bounty/:userId` | same |  | persisted; empty payload identical to the 1.12.0 stub (season dates 2020-08-23..2099) |
| `POST /bounty/:userId` | same |  | persisted and merged; a claim later funds the season-challenge coin grant at POST /inventory |
| `POST /bounty/delete/:userId` | 1.14.7-only |  | claim deletes the bounty first (Reforged) |
| `DELETE /bounty/delete/:userId` | 1.14.7-only |  | as above |
| `GET /balance` | same |  | fixed currency list identical in both; platinum buckets collapsed to CURRENCY_PLATINUM |
| `GET /product/skus/public` | table | STORE_SECTIONS, GAME_DATA | 1.12.0 serves ladyluckstore only; entitlements/skuProgression/images now from data |
| `GET /token/:currency/:catalogId` | table | STORE_SECTIONS | SKU looked up across the version's sections; token carries storeTag |
| `POST /notification/:currency` | table | STORE_SECTIONS, SEASONAL_COIN | purchase across sections; daily claim window; Platinum free unless PLATINUM_STORE_FREE=0 |

The group's other 4 routes are `same` (the JSON has each).

- **Hooks:** `entitlements_payload`.
- **Tables:** `STORE_SECTIONS`, `SEASONAL_COIN`, `GAME_DATA` (the store files).

## Multipliers, matchmaking, parties and the game servers' posts

Parties moved from memory to storage with an outbox; the answers keep their shape.

| Route | Class | Hook / table | Note |
| --- | --- | --- | --- |
| `GET /candidate/status` | hook | candidate_destination, TARGET_CHANGELIST, GAME_VERSION | buildId `<CL>_<version>_shipping`; 1.12.0 gameMode 'ISLAND' with the real huntId for Ramsgate, 1.14.7 'CITY' with huntId null. Same: CANCELED status, loopback for local clients, live hub revalidation |
| `POST /candidate/join` | same |  | adds candidateStatusPeriodMillis; forwards buildId; party online = XMPP or recent activity |
| `DELETE /candidate/leave` | 1.14.7-only |  | 1.14.7 Cancel (FUN_140db6dd0) |
| `POST /candidate/leave` | 1.14.7-only |  | alias |
| `PUT /candidate/leave` | 1.14.7-only |  | alias |
| `DELETE /candidate` | 1.14.7-only |  | alias |
| `DELETE /candidate/:candidateId` | 1.14.7-only |  | alias |
| `POST /party` | same |  | parties in storage; X-Mystic-Party-Revision header; size 4, invite TTL 10 min |

The group's other 10 routes are `same` (the JSON has each).

- **Hooks:** `candidate_destination`.
- **Tables:** `TARGET_CHANGELIST`, `GAME_VERSION`.

## Chat (friends, guild, voice stubs)

Friends, guild and Vivox are unchanged; the XMPP changes are under Cross-cutting.

| Route | Class | Hook / table | Note |
| --- | --- | --- | --- |
| `POST /slayerlink/availability` | 1.14.7-only |  | social-prod Slayer Link |
| `GET /slayerlink/status_good` | 1.14.7-only |  | social-prod Slayer Link |

The group's other 9 routes are `same` (the JSON has each).

- **Hooks:** none.
- **Tables:** none.

## Game data and scripts (RU3b had `/progression/config` in progression)

The Gauntlet routes branch on `req.originHost` and pass every other host on with `next()`, so leaving them out of the 1.12 module changes nothing else.

| Route | Class | Hook / table | Note |
| --- | --- | --- | --- |
| `GET /survey/config` | hook | survey_config | 1.12.0 400 empty; 1.14.7 200 {code:null, message:'OK', payload:{Triggers:[]}} |
| `POST /account/migrate` | same |  | RU3a: game-data, RU3b: auth |
| `GET /motd/trigger` | 1.14.7-only |  | explicit 404 (same status as 1.12's fallback) |
| `GET /playertreatments` | same |  | COHORT_TREATMENT_LEGACY default keeps CohortTreatment.Dojo.B |
| `GET /playertreatments/` | same |  | as above |
| `GET /playertreatments/:userId` | same |  | as above |
| `GET /progression/config` | hook | progression_config, ACTIVE_HUNTPASS_ID, RECENT_HUNTPASS_SEASONS, GAME_DATA | 1.14.7 rewrites hunt-pass windows (active pass rolling 12 weeks, others ended 2024-12-03, season43..50 added with 2099 windows); 1.12.0 serves the file as is |
| `GET /config` | 1.14.7-only |  | gauntlet-prod only (next() otherwise) |
| `GET /^\/progression\/([^/]*)\/([^/]+)$/i` | 1.14.7-only |  | gauntlet-prod only; next() for other origins, so it never shadows 1.12 routes |
| `GET /^\/rewards\/personal\/([^/]*)\/([^/]+)$/i` | 1.14.7-only |  | gauntlet-prod only |
| `GET /^\/rewards\/guild\/([^/]*)\/([^/]+)$/i` | 1.14.7-only |  | gauntlet-prod only |
| `GET /^\/leaderboard\/get_leaderboard\//i` | 1.14.7-only |  | gauntlet-prod only |
| `GET /patchnotes/:language/:gameversion` | table | PATCHNOTES_RELEASE_VERSION | release_version '1.12.0' vs '' (the 1.14.7 runtime collapses the line); date/description/notes are operator text |
| `GET /game_tuning/bounty_game_data` | table | BOUNTY_ITEM_GRANT_DATA | item_grant_data [] vs two windows covering now |

The group's other 15 routes are `same` (the JSON has each).

- **Hooks:** `progression_config`, `survey_config`.
- **Tables:** `ACTIVE_HUNTPASS_ID`, `RECENT_HUNTPASS_SEASONS`, `PATCHNOTES_RELEASE_VERSION`, `BOUNTY_ITEM_GRANT_DATA`, `GAME_DATA`.

## Director

Routes: `POST /api/matchmaker/handle-matchmaking-for-player` is the only one 1.12.0 has, and only its data changes
(`GAME_DATA` hunt tables; a missing row or failed start is now an error, not Ramsgate). `GET /health/ready`,
`/api/hub-status` and `/api/instances` are backend-facing, new on both.

Launch: `GAMESERVER_ARGS` and `TARGET_CHANGELIST` above. Everything else the Director gained (hub supervisor with
backoff, `TRAINING_HUB`/`RAMSGATE_HUB`, port leases, memory admission, the hunt-table gap log) is `same`.

- **Hooks:** none.
- **Tables:** `GAMESERVER_ARGS`, `TARGET_CHANGELIST` (runtime updater), `GAME_DATA` (hunt tables).

## Cross-cutting

- **Pipeline** (`app.ts`): 1.14.7 adds the request id, the per-address 429 budget (600 a minute, 6000 for gameserver
  keys) ahead of every router but `/`, the health probes and the release gates, HTTP metrics, and the admin gate:
  1.12.0 always mounts `/admin/v1`, 1.14.7 answers 404 for `/admin` unless `MYSTICPARADOX_ADMIN_API` (or the manager
  bridge) is on. The `/__origin/<host>` strip and its allowlist are unchanged. All `same`.
- **Auth** (`HasParadoxBackendAuth`): `no_player_policy` is the one version difference. `AUTH_MODE`
  `NONE`/`APIKEY`/`LAUNCHER` exist in both; `APIKEY` gains the `client_credentials` token and the eligibility 403.
- **Configuration:** `TARGET_CHANGELIST` (table); `DEV_USER_ID` default `mysticparadox` became `mystpax`, the alias
  the 1.12 client itself sends (`same`); `PORT` defaults to 3000 (1.12.0: unset); `QOS_PORT` (`CONFIG_DEFAULTS`); the
  new switches (`MYSTICPARADOX_GAUNTLET`, `_REFORGE`, `_MASTERY_PERSISTENCE`, `_BOUNTY_PERSISTENCE`,
  `_COOLDOWN_PERSISTENCE`, consent, email verification, registration approval, Guard mode) are shared settings whose
  1.12 defaults follow the hooks. `MYSTICPARADOX_BOUNTY_PERSISTENCE=off` and `_COOLDOWN_PERSISTENCE=off` reproduce
  1.12.0's stub answers byte for byte.
- **Storage:** 21 collections added (`schemaMigrations`, `progressionTransactions`, `escalationProgress`,
  `bountyStates`, `cooldownStates`, `economyDaily`, `playerActivityDaily`, `activityHunts`, `slayersPathUnlocks`,
  `activityFlags`, `emailCodes`, `accountDeletions`, `entitlements`, `launcherGuardSessions`, `guardFindings`,
  `operationsPolicy`, `launcherNotices`, `parties`, `partyMemberships`, `partyInvites`, `partyOutbox`) and the SQLite
  provider. One schema serves both modules; a 1.12 account simply has no rows in the 1.14.7-only ones.
  `STARTER_MANIFEST`'s bootstrap version marks which seeding an account got.
- **XMPP:** party events as headline messages from `party-events@`, an offline presence leaving the party, City rooms
  authorised by the `gameSessionId` served in `/candidate/status` (served on 1.12 too, so it works there),
  `REALTIME_XMPP_DEV_CITY_MUC` default false. All `same`; DD21's chat rules take them.
- **Listeners:** the QoS pong listener (`CONFIG_DEFAULTS`); TLS behind a proxy and `BIND_HOST` are new and `same`.

## What RU4's seams must expose

1. **A per-version route set** in the manifest: the 33 `1.14.7-only` routes are absent from the 1.12 module, so they
   fall to the shared unstubbed 404, in registration order as Express matches them.
2. **The trait above**, reachable from the auth middleware (`no_player_policy` runs before any router) and from each
   area crate.
3. **Typed tables** on the module: `TARGET_CHANGELIST`, `GAME_VERSION`, `ACTIVE_HUNTPASS_ID`,
   `RECENT_HUNTPASS_SEASONS`, `PATCHNOTES_RELEASE_VERSION`, `BOUNTY_ITEM_GRANT_DATA`, `STORE_SECTIONS`, `SEASONAL_COIN`,
   `MASKED_JOURNEY_NODES`, `STARTER_MANIFEST`, `CONFIG_DEFAULTS`.
4. **A game-data root per module** (`GAME_DATA`) for the backend's files and the Director's hunt tables, since every
   export is per version.
5. **Director config per module:** `GAMESERVER_ARGS`, the changelist and the hunt tables (`isles-director`).
6. **Per-version defaults for shared switches** (`QOS_PORT`, and the reforge and mastery switches if the booleans land
   in the manifest rather than the trait), with the host's own setting still winning.
7. **The launcher-facing routes** (`/launcher/v1`, `/admin/v1`, health, metrics, `/reforged/api`) are not game
   versions. They need a home that serves them once per node whichever module is loaded; the Reforged API group is
   marked `1.14.7-only` here only because the brief left it out of the diff. The exception is the changelist: the
   runtime feeds, `game-sessions`, `/launcher/v1/status` and the oauth exchange-code check take the module's
   `TARGET_CHANGELIST`, so a node with both modules keeps one runtime feed per changelist.
