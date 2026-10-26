# Route inventory: 1.12.0

Every route the Dauntless 1.12.0 backend and Director register, read from the tag `dauntless-1.12.0` (commit
`4d5dfaa`, "Last commit where every component targets Dauntless 1.12.0"). The services are `ParadoxBackend`
and `ParadoxDirector`, both on Express 5.2.1. This is RU3b of the DaintyLass plan; RU3c diffs it against
[`routes-1.14.7.md`](routes-1.14.7.md), and the 1.12 hooks of RU5 come from that diff (DD8).

[`routes-1.12.0.json`](routes-1.12.0.json) holds the same routes in the format of the 1.14.7 inventory, one
object per method and path: `{service, method, path, file, group, side}`. `file` is the file that registers
the route, as a path in the tag's tree (RU2's copy at `reference/1.12.0/` keeps them under the same relative
paths).
`path` is the full mounted Express path, written as the source writes it, so `/inventory/:userId/` keeps its
trailing slash and `/loadout/:userId//all` its double slash.

The list was built with a static extractor over the tag and checked by hand against every route file and the
controllers they call. RU3a's `check-route-inventory.mjs`, pointed at the tag's tree, finds the same 146
routes and accepts every group and side.

## Summary

| Service | Registrations | Inventory entries | Unreachable |
| --- | --- | --- | --- |
| backend | 143 (16 routers and `GET /` on the app) | 145 | 3 |
| director | 1 | 1 | 0 |

One backend registration takes an array of three paths (`/playertreatments`), so it makes three entries.

**Side.** Every route is `isles`. 1.12.0 has no realms, no p2p hosting and no Guard: no `p2p/` or `realms/`
folder, and nothing in either service mentions them. Rustic Rifts has nothing to take from this version.

**Groups** are RU5's nine areas, with the slugs the 1.14.7 inventory uses, plus `director`:

| Group | RU5 area | Backend routes |
| --- | --- | --- |
| `auth` | auth (with launcher accounts, runtime updates and the admin panel, RB6) | 51 |
| `reforged-api` | the Reforged API | 0 (`/reforged/api` came later, with Reforged's patches) |
| `characters-loadouts` | characters and loadouts | 7 |
| `inventory-reforge` | inventory, storage policy and reforge | 5 |
| `progression` | progression, mastery, escalation and session supplies | 25 |
| `bounties-store` | bounties, store, Lady Luck and platinum | 11 |
| `matchmaking-parties` | multipliers, matchmaking, parties and the game servers' posts | 14 |
| `chat` | chat (with friends, guild and voice stubs) | 9 |
| `game-data` | game data and scripts (with the game's static and stub endpoints) | 23 |
| `director` | the Director | 1 (Director) |

Where an area was a judgement call: `/admin/v1` (Mystic's own admin panel) is in `auth`, since its routes are
accounts, sessions and bans; the runtime and launcher update routes are in `auth`, which owns launcher version
checks and runtime authorisation (RB6); cooldowns and the hunt pass are in `progression`; entitlements are in
`bounties-store`.

## Reading the tables

**Auth** column:

| Mark | Meaning |
| --- | --- |
| `-` | No authentication. |
| `P` | `HasParadoxBackendAuth`: a game JWT bearer, or `X-MysticParadox-Gameserver-ApiKey` (with the player's bearer added when the game server forwards one). 401 when missing or invalid, 403 when the launcher account is no longer eligible. A valid game-server key with an invalid bearer throws, which answers 500. |
| `P + GS` | `P`, and the handler also refuses a call without the game-server key (403). |
| `L` | `HasLauncherAuth`: a launcher access token whose refresh-session family is still active. 401, or 403 for a banned, disabled, pending or rejected account. |
| `A` | `HasAdminAuth`: the admin session cookie and, on anything but GET, `X-CSRF-Token`. 401, or 403 when the account lost the admin role or eligibility, or the CSRF token is wrong. |
| `U` | Update publisher: the caller's IP in `UPDATE_PUBLISHER_ALLOWED_IPS` (403) and `X-Update-Api-Key` equal to `UPDATE_PUBLISHER_API_KEY` (401). |

Other route-level middleware is listed after the mark as the source names it.

**Status** lists what the handler and its auth answer. Not repeated on each row: 400 or 413 from the body
parsers (malformed JSON, more than 100 KB), 500 from Express's default error handler for anything thrown or
rejected (Express 5 catches async handlers too), and the extra 403 and 204 of the CORS middleware on
`/launcher/` and `/admin/v1` paths. `(500)` marks a 500 the code reaches on a normal request.

**Handler** is `registration file:line`, then `→` the controller function the route delegates to, when it
delegates. Paths are relative to each service's `src/`.

## Backend

The app (`ParadoxBackend/src/app.ts`) runs, in order:

1. `trust proxy` set to `loopback` (line 56), so `req.ip` is the forwarded client only behind a local proxy.
2. `express.json()` and `express.urlencoded({ extended: true })` (lines 58, 60).
3. The `/__origin/<host>` rewrite (line 71): a request whose path starts with `/__origin/<host>` has that prefix
   removed and `<host>` recorded, provided the host is `steelyard.ca`, `steelyard.online`, `ol.epicgames.com` or
   `api.epicgames.dev` or a subdomain of one; any other host, or bad percent-encoding, is 400. So every route below
   also answers under `/__origin/<allowed host>/...`.
4. The request logger (line 110). `MYSTICPARADOX_BODY_CAPTURE` also logs the bodies of POSTs whose URL names
   progression, inventory, store and similar areas.
5. `GET /` (line 125), then the routers, all mounted at `/` in this order: login, eos, system, character,
   inventory, store, guild, tuning, matchmaking, party, progression, loadout, friends, launcherAuth,
   launcherUpdates, admin. The table follows that order, which is the order Express tries them in.
6. Inside launcherAuth, a router-level middleware (`routes/launcherAuth.ts:64`) that every request still
   unanswered passes through. For paths under `/launcher/` it checks `Origin` against `http://localhost:1420`,
   `https://tauri.localhost` and `LAUNCHER_ALLOWED_ORIGINS` (403 otherwise), sets the CORS headers and answers
   OPTIONS with 204. Because launcherUpdates is mounted after it, the update and runtime routes get the same
   check.
7. Inside admin, `use("/admin/v1")` (`routes/admin.ts:27`): `Origin` checked against `ADMIN_ALLOWED_ORIGINS`
   (403), CORS with credentials, `Cache-Control: no-store` and a `default-src 'none'` CSP, OPTIONS answered 204.
8. The catch-all (line 146): 404 with an empty body for anything unmatched, logged as "Unstubbed route".

There is no error middleware of the app's own. Routing is Express 5's default: case-insensitive, and a
trailing slash optional.

| Method | Path | Group | Auth | Handler | Status | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| GET | `/` | game-data | - | app.ts:125 | 200 | Plain text `ok`. |
| GET | `/features/platform/win` | auth | - | routes/login.ts:20 | 200 | Static: crossplay and crossprogression true. |
| GET | `/account/link/epic/:AccId` | auth | - | routes/login.ts:33 | 200 | Static `isLinked: true`. |
| POST | `/login` | auth | P | routes/login.ts:45 | 200, 400 | 400 when the token's userId differs from `body.email` or the account is missing. |
| GET | `/accountinfo` | auth | P | routes/login.ts:77 | 200 |  |
| GET | `/tags` | auth | P | routes/login.ts:92 | 200 | Stub, empty tags. |
| PUT | `/gamesession/epic` | auth | P | routes/login.ts:101 | 200 (500) | Echoes the bearer as `sessionToken`; a game-server call without a bearer throws (500). |
| POST | `/accountinfo/public` | auth | P | routes/login.ts:123 | 200 | `DEV_USER_ID` lookups resolve to the caller's own name. |
| POST | `/migration/trigger` | auth | P | routes/login.ts:158 | 200 | Stub. |
| GET | `/migration/status` | auth | P | routes/login.ts:168 | 200 | Stub. |
| GET | `/isbanned` | auth | - | routes/login.ts:184 | 200 | Stub, always `isBanned: false`. |
| POST | `/account/api/oauth/token` | auth | - | routes/eos.ts:92 → controllers/auth.ts:SignMetagameJWTForUid, controllers/auth.ts:GetUserIDForAPIKey | 200, 400, 401, 403, 409, 500 | Branches on `AUTH_MODE`: `NONE` (dev, needs `ALLOW_NO_AUTH_DEV_MODE`), `APIKEY` (400 bad key), `LAUNCHER` (one-time exchange code: 400 missing, 401 unknown or used, 409 build changelist differs from `TARGET_CHANGELIST`, 403 account not eligible; `grant_type=client_credentials` without a code gets a token for `__launcher_client_credentials__`); 500 when no mode is set. |
| GET | `/account/api/oauth/verify` | auth | bearer (inline) | routes/eos.ts:277 → controllers/auth.ts:ValidateMetagameJWTAndGetPayload | 200, 401 |  |
| GET | `/account/api/public/account/:AccId` | auth | - (bearer optional) | routes/eos.ts:331 | 200, 404 | Reads both `accounts` and `launcherAccounts`. |
| GET | `/account/api/public/account/:AccId/externalAuths` | auth | - | routes/eos.ts:351 | 200, 404 | Always `[]` when the account exists. |
| GET | `/account/api/public/account/displayName/:displayName` | auth | - | routes/eos.ts:366 | 200, 404 | Normalised (trimmed, lower-case) display-name lookup. |
| DELETE | `/account/api/oauth/sessions/kill` | auth | - | routes/eos.ts:384 | 200 | Stub `{}`. |
| DELETE | `/account/api/oauth/sessions/kill/:AuthToken` | auth | - | routes/eos.ts:392 | 200 | Stub `{}`. |
| GET | `/account/api/public/account` | auth | P | routes/eos.ts:400 | 200 | Bulk lookup by repeated `?accountId=`; defaults to the caller. |
| GET | `/dauntless-status` | game-data | - | routes/system.ts:20 | 200 | Static status banner. |
| POST | `/heartbeat` | game-data | - | routes/system.ts:36 | 200 | Plain text `20000`. |
| POST | `/event` | game-data | - | routes/system.ts:40 | 200 | Telemetry sink, `{}`. |
| GET | `/crash/CrashReporter/Ping` | game-data | - | routes/system.ts:45 | 200 | Plain text `OK`. |
| POST | `/crash/CrashReporter/CheckReport` | game-data | - | routes/system.ts:53 | 200 | Stub `{}`. |
| GET | `/survey/config` | game-data | P | routes/system.ts:60 | 400 | Stub: always 400. |
| POST | `/account/migrate` | auth | P | routes/system.ts:67 | 200 | Stub. |
| POST | `/profile/update` | game-data | P | routes/system.ts:77 | 200 | Leaderboard profile stub, empty body. |
| POST | `/trials/leaderboards/all` | game-data | P | routes/system.ts:90 | 200 | Empty leaderboard echoing `trial_id`, `difficulty`, `page`, `page_size`. |
| POST | `/trials/leaderboards/solo/individual` | game-data | P | routes/system.ts:112 | 200 | Stub. |
| GET | `/vivox/login` | chat | P | routes/system.ts:124 | 404 | Stub: voice chat off. |
| POST | `/motd/` | game-data | P | routes/system.ts:131 | 204 | Stub. |
| GET | `/entitlementsv2` | bounties-store | P | routes/system.ts:138 | 200 | Stub, empty. |
| POST | `/entitlementv2/:userId` | bounties-store | P | routes/system.ts:149 | 200 | Stub, empty. |
| GET | `/playertreatments` | game-data | P | routes/system.ts:160 | 200 | One registration with a path array (see below); static cohort `CohortTreatment.Dojo.B`. |
| GET | `/playertreatments/` | game-data | P | routes/system.ts:160 | 200 | Same registration; never reached, since non-strict routing gives `/playertreatments/` to the entry above. |
| GET | `/playertreatments/:userId` | game-data | P | routes/system.ts:160 | 200 | Same registration. |
| GET | `/escalation/:escalationSeason/:userId` | progression | P | routes/system.ts:171 | 200 | Stub: level 0, no progress. |
| GET | `/eventstats/` | game-data | P | routes/system.ts:190 | 200 | Stub. |
| GET | `/progression/config` | progression | P | routes/system.ts:199 | 200 | `game-data/progression_config.json`. Wins over `GET /progression/:userId` because systemRouter is mounted first. |
| GET | `/huntpass/:userId` | progression | P | routes/system.ts:206 | 200 | Stub: `season19`. |
| GET | `/cooldown/:userId` | progression | P | routes/system.ts:219 | 200 | Stub, empty. |
| PUT | `/cooldown/batch/:userId` | progression | P | routes/system.ts:232 | 200 | Stub, empty; nothing stored. |
| GET | `/bounty/game-data` | bounties-store | P | routes/system.ts:245 | 200 | Static bounty config. Registered before `/bounty/:userId`, so it answers. |
| GET | `/bounty/:userId` | bounties-store | P | routes/system.ts:276 | 200 | Stub: no bounties, empty drafts. |
| POST | `/bounty/:userId` | bounties-store | P | routes/system.ts:312 | 200 | Stub: same empty payload; nothing stored. |
| GET | `/all/` | game-data | P | routes/system.ts:348 | 200 | Mailbox stub, no messages. |
| GET | `/patchnotes/:language/:gameversion` | game-data | - | routes/system.ts:362 | 200 | Stub; `release_version: "1.12.0"` hard-coded. |
| GET | `/character` | characters-loadouts | P | routes/character.ts:30 → controllers/character.ts:GetCharactersForUid | 200 | `INVALID` or missing userId returns `[]` without touching the database. |
| PUT | `/character` | characters-loadouts | P | routes/character.ts:49 → controllers/character.ts:CreateCharacterForUid | 200, 400 | Creates a character from `body.name`. |
| POST | `/character` | characters-loadouts | P | routes/character.ts:69 → controllers/character.ts:UpdateCharacterForUid, controllers/character.ts:GetCharacterWithUid | 200, 400, 409 | Game server may name `body.accountId`. 409 on an `updateVersion` conflict, with the Jetty-style HTML body the 1.12 client expects. |
| POST | `/inventory/:characterId/:changeList` | inventory-reforge | P | routes/inventory.ts:28 | 200 | Stub `{code:"NONE"}`. |
| GET | `/inventory/:userId/:characterId` | inventory-reforge | P | routes/inventory.ts:38 → controllers/inventory.ts:GetInventoryForUserIdAndCharacterId | 200, 400 | `:userId` honoured only with the game-server key. |
| GET | `/inventory/:userId/` | inventory-reforge | P | routes/inventory.ts:68 | 200, 400 | Empty inventory for `INVALID`, otherwise 400. |
| POST | `/inventory` | inventory-reforge | P | routes/inventory.ts:87 → controllers/inventory.ts:RunInventoryTransaction | 200, 400, 403, 409 | Transaction (add/remove/save, idempotent on `transactionId`). Grants and spends need the game-server key (403). 409: insufficient balance, reused id with another body, transaction in progress. |
| POST | `/inventory/instanceditem` | inventory-reforge | P | routes/inventory.ts:234 → controllers/inventory.ts:UpdateInstancedItem | 200, 400 | Updates one instanced item's data (`updateVersion`). |
| POST | `/reconcile` | bounties-store | P | routes/store.ts:22 | 200 | Wallet balances, `refreshInventory: true`. |
| GET | `/creator` | bounties-store | P | routes/store.ts:34 | 200 | Static creator code. |
| GET | `/balance` | bounties-store | P | routes/store.ts:45 → controllers/wallet.ts:GetWallet, controllers/store.ts:GetNotesForUser | 200 | Fixed currency list (weapon tokens 25) overlaid with the wallet, written as both `CURRENCY_X` and `id_currency_x`. |
| GET | `/product/skus/public` | bounties-store | P | routes/store.ts:122 → controllers/store.ts:GetLadyLuckSkus | 200, 400 | Lady Luck SKUs for `?requiredTags=ladyluckstore`; other tags `[]`; missing tag 400. |
| GET | `/token/:currency/:catalogId` | bounties-store | P | routes/store.ts:188 → controllers/store.ts:FindLadyLuckSkuByIdOrCatalogId, controllers/character.ts:GetCharactersForUid, controllers/auth.ts:SignLadyLuckPurchaseToken | 200, 404, 500 | Signs a Lady Luck purchase token for the caller's first character (500 when none). |
| POST | `/notification/:currency` | bounties-store | P | routes/store.ts:226 → controllers/auth.ts:ValidateLadyLuckPurchaseToken, controllers/store.ts:PurchaseLadyLuckSku | 204, 400, 401, 402, 403, 404, 409, 500 | Redeems `?token=`: 401 invalid, 403 other user's token; purchase failures by reason (404 unknown SKU, 402 balance, 409 conflict, 500 transaction). |
| GET | `/guild/invite/player` | chat | P | routes/guild.ts:18 | 200 | Stub, no invites. |
| GET | `/guild` | chat | P | routes/guild.ts:31 | 204 | Stub: no guild. |
| GET | `/game_tuning/seasonal_event_schedule` | game-data | - | routes/tuning.ts:17 | 200 | Static. |
| GET | `/game_tuning/huntpass_xp_config` | game-data | - | routes/tuning.ts:30 | 200 | Static. |
| GET | `/game_tuning/island_content_config` | game-data | - | routes/tuning.ts:48 | 200 | Static. |
| GET | `/game_tuning/bounty_game_data` | game-data | - | routes/tuning.ts:62 | 200 | Static. |
| GET | `/game_tuning/bounty_game_data_daily` | game-data | - | routes/tuning.ts:92 | 200 | Static. |
| GET | `/game_tuning/bounty_game_data_weekly` | game-data | - | routes/tuning.ts:123 | 200 | Static. |
| POST | `/candidate/player/register` | matchmaking-parties | P | routes/matchmaking.ts:25 | 200 | Stub `{}`. |
| POST | `/candidate/player/alive` | matchmaking-parties | P | routes/matchmaking.ts:35 | 200 | Stub `{}`. |
| GET | `/candidate/regions` | matchmaking-parties | P | routes/matchmaking.ts:40 | 200 | One region, `QOS_TARGET_URL`. |
| POST | `/key/generate` | matchmaking-parties | P | routes/matchmaking.ts:58 | 400 | Stub: always 400. |
| GET | `/candidate/status` | matchmaking-parties | P | routes/matchmaking.ts:63 → controllers/matchmaking.ts:CheckAndUpdateQueueStatus, controllers/matchmaking.ts:GetCandidateStatusPeriodMillis | 200, 404 | `MATCHING` or `IN_PROGRESS` with `serverInfo`; `buildId` is `TARGET_CHANGELIST + "_1.12.0_shipping"`. 404 when the caller has no candidate. |
| POST | `/candidate/join` | matchmaking-parties | P | routes/matchmaking.ts:118 → controllers/party.ts:GetPartyForPlayer, controllers/matchmaking.ts:HandlePlayerMatchmaking, controllers/matchmaking.ts:CheckAndUpdateQueueStatus | 200, 400 | Resolves the party (drops offline members, solo on a party-id mismatch) and calls the Director. |
| GET | `/QoS` | matchmaking-parties | - | routes/matchmaking.ts:206 | 200 | HTML `pong`. |
| POST | `/party` | matchmaking-parties | P | routes/party.ts:37 → controllers/party.ts:GetOrCreateParty, controllers/matchmaking.ts:GetPartyInstance, controllers/matchmaking.ts:GetPlayerCandidate | 200 | Get or create the caller's party (in memory). |
| PUT | `/party/invite` | matchmaking-parties | P | routes/party.ts:69 → controllers/party.ts:InviteToParty | 200 |  |
| GET | `/party/invites` | matchmaking-parties | P | routes/party.ts:84 → controllers/party.ts:GetInvitesForPlayer | 200 |  |
| PUT | `/party/invite/accept/:inviterId` | matchmaking-parties | P | routes/party.ts:98 → controllers/party.ts:AcceptInvite | 200 |  |
| DELETE | `/party/member` | matchmaking-parties | P | routes/party.ts:110 → controllers/party.ts:LeaveParty | 200 | Leave. |
| DELETE | `/party/member/:targetId` | matchmaking-parties | P | routes/party.ts:118 → controllers/party.ts:KickMember | 200 | Kick. |
| PUT | `/party/member/promote/:targetId` | matchmaking-parties | P | routes/party.ts:126 → controllers/party.ts:PromoteMember | 200 |  |
| GET | `/encountered-content/:characterId/:contentType` | progression | P | routes/progression.ts:88 → controllers/progression.ts:QueryEncounteredContent | 200 |  |
| POST | `/encountered-content/query/:characterId` | progression | P | routes/progression.ts:113 → controllers/progression.ts:QueryEncounteredContent | 200 |  |
| POST | `/encountered-content/query` | progression | P | routes/progression.ts:138 | 200 | Stub: empty content per requested type. |
| POST | `/encountered-content/:characterId/query` | progression | P | routes/progression.ts:147 → controllers/progression.ts:QueryEncounteredContent | 200 |  |
| POST | `/encountered-content/:characterId` | progression | P | routes/progression.ts:172 → controllers/progression.ts:AddEncounteredContent | 200 | Records one `content_type`/`content_id`. |
| POST | `/encountered-content` | progression | P | routes/progression.ts:205 | 200 | Stub; also answers `/encountered-content/` (non-strict routing). |
| POST | `/encountered-content/` | progression | P | routes/progression.ts:209 | 200 | Never reached: the entry above takes the trailing-slash form first. |
| GET | `/progression/objectives/:userId` | progression | P | routes/progression.ts:214 → controllers/progression.ts:GetPersistedObjectivesForUser | 200 |  |
| GET | `/breadcrumbs/` | progression | P | routes/progression.ts:238 | 200 | Stub, also answers `/breadcrumbs`. |
| GET | `/breadcrumbs/:characterId` | progression | P | routes/progression.ts:252 → controllers/progression.ts:GetBreadcrumbsForCharacterIdAndUserId | 200 |  |
| POST | `/breadcrumbs/:characterId` | progression | P | routes/progression.ts:281 → controllers/progression.ts:SetBreadcrumbsForCharacterIdAndUserId | 200 |  |
| POST | `/progression/:userId/:progressionId/:amount` | progression | P + GS | routes/progression.ts:311 → controllers/progression.ts:GrantProgressionXp | 200, 400, 403 | Grants XP (1..100000) to a track; game server only. |
| POST | `/progression/:userId` | progression | P | routes/progression.ts:362 → controllers/progression.ts:CaptureProgressionObjectiveEvent | 200 | Objective events, captured; always 200. |
| POST | `/progression/:userId/:progressionId/:rank/confirm/public` | progression | P + GS | routes/progression.ts:399 → controllers/progression.ts:ConfirmPublicProgressionRank | 200, 400, 403, 409 | Confirms a rank; 400 unknown track or rank out of range, 409 not enough XP. |
| GET | `/progression/:userId` | progression | P | routes/progression.ts:453 → controllers/progression.ts:GetPersistedProgressionForUser | 200 | Fixed list of 24 track ids (mastery, experience, prestige, `season19`). |
| GET | `/pjm/:userId` | progression | P | routes/progression.ts:499 → controllers/progression.ts:GetPlayerJourney, controllers/slayersPath.ts:GetUnlockedNodeIds, controllers/slayersPath.ts:GrantSlayersPathRewards | 200 | Slayer's Path. With the game-server key, a read also grants the rewards of unlocked nodes. |
| POST | `/pjm/:userId` | progression | P + GS | routes/progression.ts:530 → controllers/progression.ts:SavePlayerJourney, controllers/slayersPath.ts:GrantSlayersPathRewards, controllers/slayersPath.ts:GetUnlockedNodeIds | 200, 403 | Saves nodes and grants their rewards. |
| GET | `/pjm` | progression | P | routes/progression.ts:581 → controllers/progression.ts:GetPlayerJourney | 200 | Caller's saved journey, else `game-data/slayers_path.json`. |
| GET | `/progression/tracked_objectives/:phxAccountId` | progression | P | routes/progression.ts:603 | 200 | Stub. |
| POST | `/progression/tracked_objectives/:phxAccountId` | progression | P | routes/progression.ts:620 | 200 | Stub; nothing stored. |
| GET | `/loadout/:userId//all` | characters-loadouts | P | routes/loadout.ts:69 | 200, 400 | Literal `//`: the no-character form. Empty loadouts for `INVALID`, otherwise 400. |
| GET | `/loadout/:userId/:characterId/all` | characters-loadouts | P | routes/loadout.ts:84 → controllers/loadout.ts:GetAllLoadoutsForUserIdAndCharacterId, controllers/loadout.ts:GetPersistentLoadoutForUserIdAndCharacterId | 200, 400 | `:userId` honoured only with the game-server key; 400 when the character isn't the user's. |
| POST | `/loadout/:userId/:characterId/unlock/:numSlots` | characters-loadouts | P + GS | routes/loadout.ts:124 → controllers/loadout.ts:EnsureTotalLoadoutSlots, controllers/loadout.ts:GetPersistentLoadoutForUserIdAndCharacterId | 200, 400, 403 | Total slot count within `DEFAULT_ACCOUNT_LOADOUT_SLOTS..MAX_TOTAL_LOADOUT_SLOTS`. |
| POST | `/loadout/:userId/:characterId/:index` | characters-loadouts | P | routes/loadout.ts:164 → controllers/loadout.ts:SetLoadoutDataForUserIdAndCharacterId, controllers/loadout.ts:GetAllLoadoutsForUserIdAndCharacterId, controllers/loadout.ts:GetPersistentLoadoutForUserIdAndCharacterId | 200, 400 | Writes one loadout slot. |
| GET | `/friends/api/public/friends/:accountId` | chat | P | routes/friends.ts:41 | 200 | Owner is the token's user; `:accountId` is ignored. `?includePending=true`. |
| POST | `/friends/api/public/friends/:accountId/:friendId` | chat | P | routes/friends.ts:55 | 204, 400 | Request, or accept an inbound request; 400 for self. |
| DELETE | `/friends/api/public/friends/:accountId/:friendId` | chat | P | routes/friends.ts:90 | 204 | Removes both edges. |
| GET | `/friends/api/public/blocklist/:accountId` | chat | - | routes/friends.ts:103 | 200 | Stub `[]`. |
| GET | `/friends/api/public/list/:namespace/:accountId/recentPlayers` | chat | - | routes/friends.ts:108 | 200 | Stub `[]`. |
| GET | `/friends/api/v1/:accountId/settings` | chat | - | routes/friends.ts:113 | 200 | Static privacy settings. |
| POST | `/launcher/v1/auth/register` | auth | - | routes/launcherAuth.ts:127 → controllers/launcherAuth.ts:RegisterAccount | 202, 400, 409, 429, 500 | Closed-test registration, `approvalRequired: true`. 429 after 5 an hour per IP. |
| POST | `/launcher/v1/auth/login` | auth | - | routes/launcherAuth.ts:134 → controllers/launcherAuth.ts:LoginWithPassword | 200, 401, 403, 429, 500 | 403 banned, disabled, pending or rejected. 429 after 10 in 5 min per IP or email. |
| POST | `/launcher/v1/auth/refresh` | auth | - | routes/launcherAuth.ts:141 → controllers/launcherAuth.ts:RefreshSession | 200, 401, 403, 500 | Rotates the refresh token. |
| POST | `/launcher/v1/auth/logout` | auth | L | routes/launcherAuth.ts:148 → controllers/launcherAuth.ts:Logout | 200, 401, 403, 500 |  |
| POST | `/launcher/v1/auth/logout-all` | auth | L | routes/launcherAuth.ts:152 → controllers/launcherAuth.ts:LogoutAll | 200, 401, 403, 500 |  |
| POST | `/launcher/v1/auth/discord/start` | auth | - | routes/launcherAuth.ts:156 → controllers/discordAuth.ts:StartDiscordAuth | 200, 503, 500 | 503 when Discord OAuth isn't configured. |
| GET | `/launcher/v1/auth/discord/callback` | auth | - | routes/launcherAuth.ts:158 → controllers/discordAuth.ts:HandleDiscordCallback | 302 | Always a redirect to `${LAUNCHER_DEEPLINK_SCHEME}://auth/complete?code=` or `://auth/error?code=`. |
| POST | `/launcher/v1/auth/discord/complete` | auth | - | routes/launcherAuth.ts:178 → controllers/discordAuth.ts:CompleteDiscordLogin | 200, 400, 403, 500 |  |
| GET | `/launcher/v1/me` | auth | L | routes/launcherAuth.ts:185 → controllers/launcherAuth.ts:GetAccountView | 200, 401, 403, 404, 500 |  |
| GET | `/launcher/v1/username` | auth | - | routes/launcherAuth.ts:190 → controllers/launcherAuth.ts:CheckUsernameAvailable | 200, 500 | Availability check, `?name=`. |
| POST | `/launcher/v1/username` | auth | L | routes/launcherAuth.ts:195 → controllers/launcherAuth.ts:SetUsername | 200, 400, 401, 403, 404, 409, 500 |  |
| POST | `/launcher/v1/game-sessions` | auth | L | routes/launcherAuth.ts:199 → controllers/launcherAuth.ts:RequestGameExchangeCode | 200, 400, 401, 403, 429, 500 | One-time exchange code for the game. 400 missing fields or `GAME_BUILD_UNSUPPORTED` (changelist or executable SHA-256 not approved). |
| GET | `/launcher/v1/status` | auth | - | routes/launcherAuth.ts:217 | 200, 500 | `online` (database health) and `supportedBuildChangelist`. |
| GET | `/launcher/v1/runtime/:target/:channel/:platform` | auth | - | routes/launcherUpdates.ts:179 | 200, 204, 400, 404 | Runtime manifest; `:target` client or server. 204 while serving is switched off. Also catches `/launcher/v1/runtime/:channel/:platform/download` (see below). The Director fetches its runtime here. |
| GET | `/launcher/v1/runtime/:channel/:platform` | auth | - | routes/launcherUpdates.ts:190 | 200, 204, 400, 404 | Legacy form, target `client`. |
| GET | `/launcher/v1/runtime/:target/:channel/:platform/download` | auth | - | routes/launcherUpdates.ts:198 | 200, 400, 404 | Streams the DLL. |
| GET | `/launcher/v1/runtime/:channel/:platform/download` | auth | - | routes/launcherUpdates.ts:208 | (never) | Unreachable: the three-parameter manifest route above is registered first and always answers. |
| POST | `/launcher/v1/admin/updates/runtime/:target/:channel/:platform` | auth | U, `runtimePublishRateLimiter`, `express.raw({ type: "application/octet-stream", limit: "200mb" })` | routes/launcherUpdates.ts:216 | 201, 400, 401, 403, 409, 413, 429 | Raw body up to 200 MB, Ed25519 signature checked against the built-in key; changelist must be 392819; versions are immutable (409). Rate limit 30/min. |
| POST | `/launcher/v1/admin/updates/runtime/:target/:channel/:platform/extra` | auth | U, `runtimePublishRateLimiter`, `express.raw({ type: "application/octet-stream", limit: "200mb" })` | routes/launcherUpdates.ts:286 | 201, 400, 401, 403, 409, 413, 429 | Extra signed `.dll` for a published version (409 when the main DLL isn't published). |
| GET | `/launcher/v1/runtime/:target/:channel/:platform/extra/:filename` | auth | -, `runtimeDownloadRateLimiter` | routes/launcherUpdates.ts:345 | 200, 400, 404, 429 | Rate limit 180/min. |
| GET | `/launcher/v1/admin/updates/serving` | auth | U | routes/launcherUpdates.ts:370 | 200, 401, 403 |  |
| POST | `/launcher/v1/admin/updates/serving` | auth | U, `express.json({ limit: "4kb" })` | routes/launcherUpdates.ts:376 | 200, 400, 401, 403 | `{runtimeEnabled: bool}`, JSON up to 4 KB. |
| GET | `/launcher/v1/updates/:target/:arch/download` | auth | -, `runtimeDownloadRateLimiter` | routes/launcherUpdates.ts:400 | 200, 400, 404, 429 | Launcher installer. Rate limit 180/min. |
| GET | `/launcher/v1/updates/:target/:arch/:currentVersion` | auth | - | routes/launcherUpdates.ts:425 | 200, 204, 400 | Launcher update manifest; `:currentVersion` is ignored. |
| POST | `/admin/v1/auth/login` | auth | -, `adminLoginRateLimiter` | routes/admin.ts:50 → controllers/admin.ts:AdminLogin | 200, 401, 429, 503 | Email, password, admin role and TOTP (`ADMIN_TOTP_SECRET`; 503 when unset in production). Sets the session cookie, returns the CSRF token. Limited twice: express-rate-limit and the handler's own, 6 per 15 min. |
| POST | `/admin/v1/auth/logout` | auth | A | routes/admin.ts:51 → controllers/admin.ts:AdminLogout | 200, 401, 403 |  |
| GET | `/admin/v1/me` | auth | A | routes/admin.ts:52 → controllers/admin.ts:AdminMe | 200, 401, 403 |  |
| GET | `/admin/v1/online-players` | auth | A | routes/admin.ts:53 → controllers/admin.ts:ListOnlinePlayers | 200, 401, 403 | From the XMPP session registry. |
| GET | `/admin/v1/players` | auth | A | routes/admin.ts:54 → controllers/admin.ts:ListPlayers | 200, 401, 403 | Paged. |
| PATCH | `/admin/v1/players/:userId/access` | auth | A | routes/admin.ts:55 → controllers/admin.ts:UpdatePlayerAccess | 200, 400, 401, 403, 404, 409 | Approval and account status; audited. |
| DELETE | `/admin/v1/players/:userId` | auth | A | routes/admin.ts:56 → controllers/admin.ts:DeletePlayer | 200, 400, 401, 403, 404, 409 | Needs a reason; 409 for one's own account; audited. |
| GET | `/admin/v1/audit` | auth | A | routes/admin.ts:57 → controllers/admin.ts:ListAudit | 200, 401, 403 | Paged. |

## Director

The app (`ParadoxDirector/src/app.ts`) parses JSON and URL-encoded bodies and mounts one router at
`/api/matchmaker`. It has no catch-all and no error middleware, so an unknown path gets Express's own 404 page
(`Cannot GET ...`), unlike the backend's empty 404. It sets no `trust proxy`.

| Method | Path | Group | Auth | Handler | Status | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| POST | `/api/matchmaker/handle-matchmaking-for-player` | director | -, `express.json()` | routes/matchmaker.ts:19 → controllers/matchmaker.ts:HandleMatchmakingRequest | 200, 500 | Called by the backend (`DEPLOYSERVER_URL`). No auth. Starts or reuses a game server for `GameMode`, `GameArgs`, `HuntId`, `ExpectedPlayers`; 500 `{error:"matchmaking_failed"}`. |

## Unreachable and order-dependent registrations

Three entries never answer, because a route registered earlier takes their requests:

| Entry | Taken by | Why |
| --- | --- | --- |
| `POST /encountered-content/` (`routes/progression.ts:209`) | `POST /encountered-content` (line 205) | Non-strict routing: the first route matches the trailing-slash form too. Both answer the same stub. |
| `GET /playertreatments/` (`routes/system.ts:160`) | `GET /playertreatments`, same registration | Same handler; only the entry is dead. |
| `GET /launcher/v1/runtime/:channel/:platform/download` (`routes/launcherUpdates.ts:208`) | `GET /launcher/v1/runtime/:target/:channel/:platform` (line 179) | Both have three segments after `runtime/`. The earlier route reads `download` as `:platform` and always answers: 400 unless the first segment is `client` or `server`, else a manifest lookup that finds nothing (404). |

Order also decides these, and a port has to keep it:

- `GET /progression/config` (systemRouter) answers before `GET /progression/:userId` (progressionRouter, mounted
  later) would take `config` as a user id.
- `GET /bounty/game-data` is registered before `GET /bounty/:userId`. The source's comment says the param route
  "masks" it, but the order makes `game-data` answer.
- `GET /breadcrumbs/` answers `/breadcrumbs` as well, and `GET /inventory/:userId/` answers `/inventory/<id>`.
- `GET /launcher/v1/updates/:target/:arch/download` is registered before `.../:currentVersion`, so `download` is
  never read as a version.

## Registrations reviewed by hand

1.12.0 has no regular-expression paths, no `app.route()`, no routes registered in loops or behind conditions, no
router built inside a function and no router loaded through `require()`. Every router is a module-level
`Router()` mounted once in `app.ts`. What the extractor couldn't settle alone:

- **The path array** at `routes/system.ts:160`, `["/playertreatments", "/playertreatments/",
  "/playertreatments/:userId"]`: one registration, three entries.
- **The two publish routes** at `routes/launcherUpdates.ts:216` and `:286`, written over several lines with a
  rate limiter and `express.raw()` before the handler.
- **The launcher auth routes**: each handler is an inline wrapper around `Handle()` (`routes/launcherAuth.ts:112`),
  which answers its success status (200, or 202 for register) and turns a thrown `LauncherApiError` into its
  status through `StatusByCode` in `security/launcherErrors.ts`. The statuses listed come from the error codes the
  called controllers (`controllers/launcherAuth.ts`, `controllers/discordAuth.ts`,
  `security/accountEligibility.ts`) can throw.
- **Statuses chosen from a table**: `StatusByReason` in `routes/store.ts:256` (Lady Luck purchases) and
  `routes/progression.ts:429` (rank confirmation).
- **`AUTH_MODE` branches** in `POST /account/api/oauth/token`: one route, three login modes, each with its own
  failures.
- **The WebSocket upgrade**, which never reaches Express (next section).

## Non-Express listeners

| Service | Listener | Where | Notes |
| --- | --- | --- | --- |
| backend | HTTP, the Express app | `server.ts:74`, `app.listen(PORT)` | All interfaces; `PORT` has no default. |
| backend | HTTPS, the same app | `server.ts:94`, `https.createServer(...).listen(HTTPS_PORT ?? 3443)` | All interfaces. Started only when the certificate and key load (`PARADOX_CERT_PEM_PATH`, `PARADOX_KEY_PEM_PATH`, optional `PARADOX_KEY_PASSPHRASE`); otherwise logged and skipped. |
| backend | XMPP over WebSocket | `realtime/RealtimeGateway.ts:49`, the HTTPS server's `upgrade` event | HTTPS only, never plain HTTP. Off unless `REALTIME_XMPP_ENABLED=true`. Paths `//`, `/` and `/__ws/xmpp` (`realtime/types.ts:15`, replaced by `REALTIME_XMPP_WS_PATHS`), subprotocol `xmpp`, frames up to 64 KiB. Refused before the handshake with a bare status line: 503 when disabled, shutting down or over the global limit, 404 for another path, 403 for a host outside `REALTIME_XMPP_ALLOWED_HOSTS`, 429 over the per-IP limit or after repeated failed logins. Group `chat`. |
| director | HTTP, the Express app | `server.ts:28`, `app.listen(PORT)` | All interfaces. Its one route has no authentication, so the Director must not be reachable from outside. |
| director | Game servers | `controllers/gameservers.ts:345`, `spawn(GAMESERVER_BINARY_PATH, ...)` | Not Node listeners: the game servers the Director starts listen on ports from `PORT_RANGE_BEGIN` to `PORT_RANGE_END` and call the backend with `METAGAME_API_KEY` as their game-server key. |

Calls between the services, for the port's wiring: the backend posts to the Director at
`http://${DEPLOYSERVER_URL}/api/matchmaker/handle-matchmaking-for-player` (`controllers/matchmaking.ts:17`). At
startup the Director fetches its runtime DLL from the backend's `GET /launcher/v1/runtime/server/stable/windows-x86_64`
(`runtimeUpdater.ts`, `SERVER_RUNTIME_MANIFEST_URL`). The backend calls Discord for launcher sign-in.

## Values tied to 1.12.0

Literals in these routes that name the game version or its content. They are where the 1.12 module's hooks and
tables (DD8) will come from:

- `routes/matchmaking.ts:82`: `serverInfo.buildId` is `TARGET_CHANGELIST + "_1.12.0_shipping"`.
- `routes/system.ts:374`: patch notes report `release_version: "1.12.0"`.
- `routes/launcherUpdates.ts:243` and `:259`: runtime publishes must carry changelist `392819`, which also names
  the stored DLL; the Director pins the same changelist (`runtimeUpdater.ts:10`).
- `routes/character.ts:130`: the 409 on a character update conflict is the HTML page the 1.12 client recognises
  before it fetches the character again.
- `routes/progression.ts:33`: the default track list ends with `season19`, and `GET /huntpass/:userId` answers
  `season19`.
- `routes/store.ts:53`: the fixed currency list of `GET /balance` (season coins S13 to S20, events, marks).
