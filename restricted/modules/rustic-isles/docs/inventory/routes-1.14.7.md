# Route inventory: 1.14.7

RU3a. Every Express route that `ParadoxBackend` and `ParadoxDirector` register, as of `main` at `ce1c42e` (Mystic
Paradox `21b9ff5` with Reforged's patches 0001 to 0039), with the listeners that aren't Express at the end. RU4 answers each route 501 and RU5 ports them, one area group per agent; RF1 takes the
rifts rows. Since D3 (spec-rd27-rifts) the Rust port serves the `/mystic/v1/hunts` rows at `/rustic/v1/hunts`; the
rows keep Node's paths.

Since R21 (2026-11-14) `POST /launcher/v1/game-sessions` (row 163) takes `rustic-isles-runtime.dll`, not
`Rustic-Isles.dll`, as the core runtime file a launcher reports beside `winmm.dll` (`runtimeAuthorization.ts`'s
`CORE_INSTALLED_ARTIFACTS`); the route and its answers are otherwise unchanged.

[`routes-1.14.7.json`](routes-1.14.7.json) is the machine-readable list: one `{service, method, path, file, group,
side}` per registration. [`scripts/check-route-inventory.mjs`](../../scripts/check-route-inventory.mjs) extracts the
routes from the source and fails on a route the JSON lacks, a JSON entry with no route behind it, a row missing here,
a group outside its side's list, and a registration that can never answer.

```
node scripts/check-route-inventory.mjs           # the check (exit 1 on a problem)
node scripts/check-route-inventory.mjs --write   # add new routes to the JSON; known rows keep their group and side
node scripts/check-route-inventory.mjs --table   # the details below, one JSON line per route
```

A new route is added with `--write`; its group and side are then set by hand, and a row goes in the table here. The
check needs Node 22 and nothing installed; DX3 runs it in CI.

## Reading the tables

- **Paths** are written as Express sees them after every router prefix: `:name` is one non-empty segment, a regex
  route is written as its literal. Express 5.2.1 matches string paths case-insensitively and takes an optional
  trailing slash, so `/motd/` also answers `/motd`.
- **Files** are relative to the repository root. RU1 moved both services under `reference/1.14.7/`; a later move
  rewrites the prefix here, in the JSON and in the script's `BACKEND` and `DIRECTOR` constants.
- **Rows are in registration order**, which is Express's match order: an earlier route that matches wins unless its
  handler calls `next()`.
- **Side**: `rifts` for `p2p/`, `realms/` and Guard (DD14), `isles` for the rest.
- **Group**: one of RU5's nine area groups or the Director for isles rows, the source area (`p2p`, `realms`,
  `guard`) for rifts rows. The judgement calls are listed under [Groups](#groups).
- **When**: mounted only with `admin` (`RUSTIC_ISLES_ADMIN_API` on, or the manager bridge; otherwise every `/admin`
  path answers 404 before any router), `p2p` (the optional module in `src/p2p` is present and
  `RUSTIC_ISLES_P2P_MODULE` isn't off) or `realms` (`RUSTIC_ISLES_REALMS_MODULE` on).
- **Middleware** is what the route itself lists, plus `(router)` for a router-level hook that runs first:
  `launcher CORS` (`launcherAuth.ts`: 403 for an origin outside `LAUNCHER_ALLOWED_ORIGINS`, 204 for `OPTIONS`),
  `admin CORS` (`admin.ts`, `/admin/v1`: the same with the admin origins, plus `no-store` and a CSP), `P2P JSON
  72 KiB` (`p2pHunts.ts`: its own body parser, mounted before the app's), `no-store` and `HasLauncherAuth`
  (`realmsLauncher.ts`, on `/launcher/v1/realms`).
- **Handler** is the file and line of an inline handler, or the named function. **Calls** are the functions from
  other files it calls (up to four), for orientation.
- **Statuses** are the ones the handler and its middleware can set, read statically from the code: a best effort, not
  a contract. `LE` means a `LauncherApiError`, whose status comes from its code in `security/launcherErrors.ts`
  (`StatusByCode`, then the module's own codes, then 500); `dyn` a status computed at run time. Not listed on each row:
  429 from the per-address request budget (`security/rateLimit.ts`) on every route below it in the pipeline, 400 from
  the JSON parser on a malformed body, and 500 from Express 5's default error handler when an async handler throws.

## Backend pipeline

`reference/1.14.7/ParadoxBackend/src/app.ts`, in order. A request that no route answers reaches the last handler,
which logs `Unstubbed route` and answers 404.

1. `RequestContextMiddleware`: a request id for every log line.
2. The `/__origin/<host>/` strip: the runtime's `SetURL` hook prefixes the official host to the path; the prefix is
   removed, the host checked against `steelyard.ca`, `steelyard.online`, `ol.epicgames.com` and `api.epicgames.dev`
   (400 otherwise) and kept as `req.originHost`. The gauntlet routes branch on it.
3. `RUSTIC_ISLES_LOG_ORIGIN_BODIES=1` only: logs the bodies served to `/__origin` requests.
4. The request log and HTTP metrics (the `/mystic/v1/hunts/:id/lease` and `event-batches` lines only when refused).
5. The request budget on `/mystic/v1/hunts`, then the p2p module's early routes (`p2p`): the host-route census
   (`RUSTIC_ISLES_HOST_ROUTE_CENSUS`) and `p2pHuntsRouter`, ahead of the shared body parsers.
6. `express.json()` (keeping the raw bytes of `/admin/v1` bodies for the manager operator signature),
   `express.urlencoded()`, and the `RUSTIC_ISLES_BODY_CAPTURE` log.
7. `/`, `/health/live`, `/health/ready` and, with the admin API, `/health/release-gates` (loopback peers only): outside
   the request budget.
8. The request budget for everything below (429).
9. The routers, each mounted at `/`: metrics, login, eos, system, loginQueue, character, inventory, store, guild,
   tuning, matchmaking, party, progression, loadout, friends, launcherAuth, reforgedApi, launcherUpdates,
   launcherLogs.
10. The `/admin` gate (404 with the admin API off), then adminLauncherLogs and admin.
11. Realms (`MountRealmsModule`), the p2p module's routes (p2pAdmin, p2pLauncher, realmHosting), launcherGuard.
12. The 404 fallthrough, then the p2p module's body error handler.

Auth middleware, for reading the Middleware column: `HasParadoxBackendAuth` (the game's bearer token or a game
server's `x-rustic-isles-gameserver-apikey`, 401/403), `HasLauncherAuth` (the launcher's access token, `LE`),
`RequireActiveAccount` (an account that isn't deactivated; `(consent)` also needs the current consent),
`RequireHostCapability(...)` (a player host's capability token for that operation, p2p), `HasAdminAuth` (the admin
dashboard's cookie session with its CSRF token, or the manager's signed operator headers when that bridge is on),
`HostOnly` (`reforgedApi.ts`: `DAINTY_ADMIN_KEY` from a loopback peer, nothing forwarded, 403), `requireSyncToken`
(`LAUNCHER_LOG_SYNC_TOKEN` in `x-sync-token`, 401). Routes with no auth middleware answer anyone who reaches the listener;
several check a header inside the handler (`/reforged/api/*` reads the account key, the update publisher routes
`UPDATE_PUBLISHER_API_KEY`, `/internal/v1/metrics` `RUSTIC_ISLES_METRICS_TOKEN`, or a loopback peer outside
production).

## Backend routes

285 registrations: 219 isles, 66 rifts.

| # | Method | Path | Side | Group | When | Middleware | Handler | Calls | Statuses |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | GET | `/` | isles | reforged-api |  |  | reference/1.14.7/ParadoxBackend/src/app.ts:214 |  | 200 |
| 2 | GET | `/health/live` | isles | reforged-api |  |  | reference/1.14.7/ParadoxBackend/src/app.ts:221 | serviceRole.ts:ReadMetagameServiceRole | 200 |
| 3 | GET | `/health/ready` | isles | reforged-api |  |  | reference/1.14.7/ParadoxBackend/src/app.ts:225 | serviceReadiness.ts:GetServiceReadiness, index.ts:GetPersistenceLifecycle | 200 dyn |
| 4 | GET | `/health/release-gates` | isles | reforged-api | admin |  | reference/1.14.7/ParadoxBackend/src/routes/releaseGates.ts:84 CreateReleaseGatesHandler |  | 200 404 dyn |
| 5 | GET | `/internal/v1/metrics` | isles | reforged-api |  |  | reference/1.14.7/ParadoxBackend/src/routes/metrics.ts:34 | metrics.ts:RenderMetrics | 200 dyn |
| 6 | GET | `/features/platform/win` | isles | auth |  |  | reference/1.14.7/ParadoxBackend/src/routes/login.ts:21 |  | 200 |
| 7 | GET | `/account/link/epic/:AccId` | isles | auth |  |  | reference/1.14.7/ParadoxBackend/src/routes/login.ts:34 |  | 200 |
| 8 | POST | `/login` | isles | auth |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/login.ts:46 | index.ts:GetRepositories, sessionSupplies.ts:RefillSessionSupplies | 200 400 401 403 |
| 9 | GET | `/accountinfo` | isles | auth |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/login.ts:81 | login.ts:GetUsernameForUserId | 200 401 403 |
| 10 | GET | `/tags` | isles | auth |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/login.ts:96 |  | 200 401 403 |
| 11 | PUT | `/gamesession/epic` | isles | auth |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/login.ts:117 |  | 200 401 403 |
| 12 | PUT | `/gamesession/epiceos` | isles | auth |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/login.ts:117 |  | 200 401 403 |
| 13 | POST | `/accountinfo/public` | isles | auth |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/login.ts:142 | login.ts:GetUsernameForUserId | 200 401 403 |
| 14 | POST | `/migration/trigger` | isles | auth |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/login.ts:177 |  | 200 401 403 |
| 15 | GET | `/migration/status` | isles | auth |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/login.ts:187 |  | 200 401 403 |
| 16 | GET | `/isbanned` | isles | auth |  |  | reference/1.14.7/ParadoxBackend/src/routes/login.ts:203 |  | 200 |
| 17 | POST | `/account/api/oauth/token` | isles | auth |  |  | reference/1.14.7/ParadoxBackend/src/routes/eos.ts:99 | auth.ts:SignMetagameJWTForUid, auth.ts:GetUserIDForAPIKey, accountEligibility.ts:IsAccountEligible, launcherTokens.ts:HashOpaqueToken | 200 400 401 403 409 500 503 LE |
| 18 | GET | `/account/api/oauth/verify` | isles | auth |  |  | reference/1.14.7/ParadoxBackend/src/routes/eos.ts:345 | auth.ts:ValidateMetagameJWTAndGetPayload | 200 401 |
| 19 | GET | `/account/api/public/account/:AccId` | isles | auth |  |  | reference/1.14.7/ParadoxBackend/src/routes/eos.ts:399 | index.ts:GetRepositories | 200 404 |
| 20 | GET | `/account/api/public/account/:AccId/externalAuths` | isles | auth |  |  | reference/1.14.7/ParadoxBackend/src/routes/eos.ts:419 | index.ts:GetRepositories | 200 404 |
| 21 | GET | `/account/api/public/account/displayName/:displayName` | isles | auth |  |  | reference/1.14.7/ParadoxBackend/src/routes/eos.ts:434 | index.ts:GetRepositories | 200 404 |
| 22 | DELETE | `/account/api/oauth/sessions/kill` | isles | auth |  |  | reference/1.14.7/ParadoxBackend/src/routes/eos.ts:452 |  | 200 |
| 23 | DELETE | `/account/api/oauth/sessions/kill/:AuthToken` | isles | auth |  |  | reference/1.14.7/ParadoxBackend/src/routes/eos.ts:460 |  | 200 |
| 24 | GET | `/account/api/public/account` | isles | auth |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/eos.ts:468 | index.ts:GetRepositories | 200 401 403 |
| 25 | GET | `/dauntless-status` | isles | reforged-api |  |  | reference/1.14.7/ParadoxBackend/src/routes/system.ts:35 |  | 200 |
| 26 | POST | `/heartbeat` | isles | game-data |  |  | reference/1.14.7/ParadoxBackend/src/routes/system.ts:51 |  | 200 |
| 27 | POST | `/event` | isles | game-data |  |  | reference/1.14.7/ParadoxBackend/src/routes/system.ts:55 |  | 200 |
| 28 | GET | `/crash/CrashReporter/Ping` | isles | game-data |  |  | reference/1.14.7/ParadoxBackend/src/routes/system.ts:60 |  | 200 |
| 29 | POST | `/crash/CrashReporter/CheckReport` | isles | game-data |  |  | reference/1.14.7/ParadoxBackend/src/routes/system.ts:68 |  | 200 |
| 30 | GET | `/survey/config` | isles | game-data |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:75 |  | 200 401 403 |
| 31 | POST | `/account/migrate` | isles | game-data |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:104 |  | 200 401 403 |
| 32 | POST | `/profile/update` | isles | game-data |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:114 |  | 200 401 403 |
| 33 | POST | `/trials/leaderboards/all` | isles | game-data |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:127 |  | 200 401 403 |
| 34 | POST | `/trials/leaderboards/solo/individual` | isles | game-data |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:149 |  | 200 401 403 |
| 35 | GET | `/vivox/login` | isles | chat |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:161 |  | 200 401 403 404 |
| 36 | POST | `/motd/` | isles | game-data |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:168 |  | 200 204 401 403 |
| 37 | GET | `/motd/trigger` | isles | game-data |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:180 |  | 200 401 403 404 |
| 38 | GET | `/entitlementsv2` | isles | bounties-store |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:197 | entitlements.ts:GetEntitlementsForUser | 200 401 403 |
| 39 | POST | `/entitlementv2/:userId` | isles | bounties-store |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:207 |  | 200 401 403 |
| 40 | GET | `/playertreatments` | isles | game-data |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:218 |  | 200 401 403 |
| 41 | GET | `/playertreatments/` | isles | game-data |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:218 |  | 200 401 403 |
| 42 | GET | `/playertreatments/:userId` | isles | game-data |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:218 |  | 200 401 403 |
| 43 | GET | `/escalation/:escalationSeason/:userId` | isles | progression |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:253 | escalationProgress.ts:IsAuthorizedEscalationReader, escalationProgress.ts:GetEscalationProgress | 200 401 403 |
| 44 | POST | `/escalation/:escalationSeason/:userId` | isles | progression |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:305 | escalationProgress.ts:IsAuthorizedEscalationWriter, escalationProgress.ts:IsHostEscalationWriter, p2p.ts:GetP2PExtension, escalationProgress.ts:GetEscalationTuning | 200 401 403 dyn |
| 45 | GET | `/eventstats/` | isles | game-data |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:366 |  | 200 401 403 |
| 46 | GET | `/progression/config` | isles | game-data |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:487 |  | 200 401 403 |
| 47 | GET | `/config` | isles | game-data |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:505 | gauntletPayloads.ts:GauntletConfigPayload | 200 401 403 |
| 48 | GET | `/^\/progression\/([^/]*)\/([^/]+)$/i` | isles | game-data |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:559 | gauntletPayloads.ts:GauntletProgressionPayload | 200 401 403 |
| 49 | GET | `/^\/rewards\/personal\/([^/]*)\/([^/]+)$/i` | isles | game-data |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:565 |  | 200 401 403 |
| 50 | GET | `/^\/rewards\/guild\/([^/]*)\/([^/]+)$/i` | isles | game-data |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:571 | gauntletPayloads.ts:GauntletGuildRewardsPayload | 200 401 403 |
| 51 | GET | `/^\/leaderboard\/get_leaderboard\//i` | isles | game-data |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:579 | gauntletPayloads.ts:GauntletLeaderboardPayload | 200 401 403 |
| 52 | GET | `/huntpass/:userId` | isles | progression |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:585 |  | 200 401 403 |
| 53 | GET | `/cooldown/:userId` | isles | bounties-store |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:628 | cooldownState.ts:IsAuthorizedCooldownReader, economyMonitor.ts:MonitorCooldownLoad, economyMonitor.ts:MonitorSourceOf, cooldowns.ts:GetCooldownState | 200 401 403 |
| 54 | PUT | `/cooldown/batch/:userId` | isles | bounties-store |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:645 | cooldownState.ts:IsAuthorizedCooldownWriter, cooldownState.ts:IsHostCooldownWriter, economyMonitor.ts:MonitorCooldownSave, economyMonitor.ts:MonitorSourceOf | 200 400 401 403 dyn |
| 55 | PUT | `/cooldown/:userId/:cooldownId` | isles | bounties-store |  |  | reference/1.14.7/ParadoxBackend/src/controllers/cooldowns.ts:141 StartCooldownHandlers |  | 200 403 dyn |
| 56 | POST | `/cooldown/:userId/:cooldownId` | isles | bounties-store |  |  | reference/1.14.7/ParadoxBackend/src/controllers/cooldowns.ts:141 StartCooldownHandlers |  | 200 403 dyn |
| 57 | GET | `/bounty/game-data` | isles | bounties-store |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:694 |  | 200 401 403 |
| 58 | GET | `/bounty/:userId` | isles | bounties-store |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:756 | bountyState.ts:IsAuthorizedBountyReader, economyMonitor.ts:MonitorBountyLoad, economyMonitor.ts:MonitorSourceOf, bountyState.ts:BountyStatePayload | 200 401 403 |
| 59 | POST | `/bounty/:userId` | isles | bounties-store |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:774 | bountyState.ts:IsAuthorizedBountyWriter, bountyState.ts:IsHostBountyWriter, economyMonitor.ts:MonitorBountySave, economyMonitor.ts:MonitorSourceOf | 200 400 401 403 dyn |
| 60 | POST | `/bounty/delete/:userId` | isles | bounties-store |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:826 HandleBountyDelete |  | 200 400 401 403 dyn |
| 61 | DELETE | `/bounty/delete/:userId` | isles | bounties-store |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:826 HandleBountyDelete |  | 200 400 401 403 dyn |
| 62 | GET | `/all/` | isles | game-data |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/system.ts:872 |  | 200 401 403 |
| 63 | GET | `/patchnotes/:language/:gameversion` | isles | game-data |  |  | reference/1.14.7/ParadoxBackend/src/routes/system.ts:889 |  | 200 |
| 64 | GET | `/maintenance/status` | isles | reforged-api |  |  | reference/1.14.7/ParadoxBackend/src/routes/loginQueue.ts:17 |  | 200 |
| 65 | GET | `/character` | isles | characters-loadouts |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/character.ts:78 | character.ts:GetCharactersForUid | 200 401 403 |
| 66 | PUT | `/character` | isles | characters-loadouts |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/character.ts:243 | character.ts:CreateCharacterForUid | 200 400 401 403 |
| 67 | POST | `/character` | isles | characters-loadouts |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/character.ts:263 | character.ts:GetCharactersForUid, character.ts:UpdateCharacterForUid | 200 400 401 403 409 |
| 68 | POST | `/^\/inventory\/([^/]*)\/(.+)$/` | isles | inventory-reforge |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/inventory.ts:41 |  | 200 401 403 |
| 69 | POST | `/inventory/:characterId/:changeList` | isles | inventory-reforge |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/inventory.ts:51 |  | 200 401 403 |
| 70 | GET | `/inventory/:userId/:characterId` | isles | inventory-reforge |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/inventory.ts:61 | inventory.ts:GetInventoryForUserIdAndCharacterId | 200 400 401 403 |
| 71 | GET | `/inventory/:userId/` | isles | inventory-reforge |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/inventory.ts:91 | character.ts:GetCharactersForUid, inventory.ts:GetInventoryForUserIdAndCharacterId | 200 400 401 403 |
| 72 | POST | `/inventory` | isles | inventory-reforge |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/inventory.ts:132 | character.ts:GetCharactersForUid, capture.ts:CaptureInventoryTransaction, economyMonitor.ts:MonitorGrant, economyMonitor.ts:MonitorInventoryRequest | 200 400 401 403 409 dyn |
| 73 | POST | `/inventory/instanceditem` | isles | inventory-reforge |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/inventory.ts:397 | p2p.ts:GetP2PExtension, validation.ts:ValidateInstancedItemUpdateBody, capture.ts:CaptureEvent, inventory.ts:UpdateInstancedItem | 200 400 401 403 dyn |
| 74 | POST | `/reconcile` | isles | bounties-store |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/store.ts:24 | wallet.ts:BuildBalanceDict | 200 401 403 |
| 75 | GET | `/creator` | isles | bounties-store |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/store.ts:36 |  | 200 401 403 |
| 76 | GET | `/balance` | isles | bounties-store |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/store.ts:47 | wallet.ts:GetWallet, store.ts:GetNotesForUser, platinumWallet.ts:ProjectBalances | 200 401 403 |
| 77 | GET | `/product/skus/public` | isles | bounties-store |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/store.ts:125 | store.ts:GetStoreSkusForPlayer, platinumStoreFree.ts:EffectivePrice | 200 400 401 403 |
| 78 | GET | `/token/:currency/:catalogId` | isles | bounties-store |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/store.ts:203 | store.ts:FindStoreSkuByIdOrCatalogId, character.ts:GetCharactersForUid, auth.ts:SignStorePurchaseToken | 200 401 403 404 500 |
| 79 | POST | `/notification/:currency` | isles | bounties-store |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/store.ts:246 | auth.ts:ValidateStorePurchaseToken, store.ts:FindStoreSkuByIdOrCatalogId, store.ts:PurchaseStoreSku | 200 204 400 401 403 404 dyn |
| 80 | GET | `/guild/invite/player` | isles | chat |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/guild.ts:18 |  | 200 401 403 |
| 81 | GET | `/guild` | isles | chat |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/guild.ts:31 |  | 200 204 401 403 |
| 82 | GET | `/game_tuning/seasonal_event_schedule` | isles | game-data |  |  | reference/1.14.7/ParadoxBackend/src/routes/tuning.ts:49 |  | 200 |
| 83 | GET | `/game_tuning/huntpass_xp_config` | isles | game-data |  |  | reference/1.14.7/ParadoxBackend/src/routes/tuning.ts:62 |  | 200 |
| 84 | GET | `/game_tuning/island_content_config` | isles | game-data |  |  | reference/1.14.7/ParadoxBackend/src/routes/tuning.ts:80 |  | 200 |
| 85 | GET | `/game_tuning/bounty_game_data` | isles | game-data |  |  | reference/1.14.7/ParadoxBackend/src/routes/tuning.ts:94 |  | 200 |
| 86 | GET | `/game_tuning/bounty_game_data_daily` | isles | game-data |  |  | reference/1.14.7/ParadoxBackend/src/routes/tuning.ts:131 |  | 200 |
| 87 | GET | `/game_tuning/bounty_game_data_weekly` | isles | game-data |  |  | reference/1.14.7/ParadoxBackend/src/routes/tuning.ts:162 |  | 200 |
| 88 | POST | `/candidate/player/register` | isles | matchmaking-parties |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/matchmaking.ts:50 |  | 200 401 403 |
| 89 | POST | `/candidate/player/alive` | isles | matchmaking-parties |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/matchmaking.ts:60 | activityMonitor.ts:TouchGameSession | 200 401 403 |
| 90 | GET | `/candidate/regions` | isles | matchmaking-parties |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/matchmaking.ts:68 |  | 200 401 403 |
| 91 | POST | `/key/generate` | isles | auth |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/matchmaking.ts:126 | encryptionKeyStore.ts:EncryptionKeyTokenFingerprint | 200 401 403 |
| 92 | POST | `/key/consume` | isles | auth |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/matchmaking.ts:151 | encryptionKeyStore.ts:EncryptionKeyTokenFingerprint, runtimeRoster.ts:RuntimeRosterOf | 200 401 403 404 |
| 93 | GET | `/candidate/status` | isles | matchmaking-parties |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/matchmaking.ts:196 | matchmaking.ts:CheckAndUpdateQueueStatus, candidateStatus.ts:CandidateStatusAnswerFor, matchmaking.ts:IsHubGameSessionId, cityRooms.ts:RecordHubSessionServed | 200 401 403 404 dyn |
| 94 | POST | `/candidate/join` | isles | matchmaking-parties |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/matchmaking.ts:220 | party.ts:GetPartyForPlayer, p2p.ts:GetP2PExtension, matchmakingParty.ts:ResolveMatchmakingParty, activityMonitor.ts:IsGameRecentlyActive | 200 400 401 403 |
| 95 | DELETE | `/candidate/leave` | isles | matchmaking-parties |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/matchmaking.ts:335 CandidateLeaveHandler |  | 200 401 403 500 |
| 96 | POST | `/candidate/leave` | isles | matchmaking-parties |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/matchmaking.ts:335 CandidateLeaveHandler |  | 200 401 403 500 |
| 97 | PUT | `/candidate/leave` | isles | matchmaking-parties |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/matchmaking.ts:335 CandidateLeaveHandler |  | 200 401 403 500 |
| 98 | DELETE | `/candidate` | isles | matchmaking-parties |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/matchmaking.ts:335 CandidateLeaveHandler |  | 200 401 403 500 |
| 99 | DELETE | `/candidate/:candidateId` | isles | matchmaking-parties |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/matchmaking.ts:335 CandidateLeaveHandler |  | 200 401 403 500 |
| 100 | GET | `/QoS` | isles | matchmaking-parties |  |  | reference/1.14.7/ParadoxBackend/src/routes/matchmaking.ts:358 |  | 200 |
| 101 | POST | `/party` | isles | matchmaking-parties |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/party.ts:36 | party.ts:GetOrCreateParty, matchmaking.ts:GetPartyInstance, matchmaking.ts:GetPlayerCandidate, login.ts:GetUsernameForUserId | 200 401 403 |
| 102 | PUT | `/party/invite` | isles | matchmaking-parties |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/party.ts:69 | login.ts:GetUsernameForUserId, party.ts:InviteToParty | 200 401 403 |
| 103 | GET | `/party/invites` | isles | matchmaking-parties |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/party.ts:84 | party.ts:GetInvitesForPlayer | 200 401 403 |
| 104 | PUT | `/party/invite/accept/:inviterId` | isles | matchmaking-parties |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/party.ts:98 | party.ts:AcceptInvite | 200 401 403 |
| 105 | DELETE | `/party/member` | isles | matchmaking-parties |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/party.ts:110 | party.ts:LeaveParty | 200 401 403 |
| 106 | DELETE | `/party/member/:targetId` | isles | matchmaking-parties |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/party.ts:118 | party.ts:KickMember | 200 401 403 |
| 107 | PUT | `/party/member/promote/:targetId` | isles | matchmaking-parties |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/party.ts:126 | party.ts:PromoteMember | 200 401 403 |
| 108 | GET | `/encountered-content/:characterId/:contentType` | isles | progression |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/progression.ts:117 | progression.ts:QueryEncounteredContent | 200 401 403 |
| 109 | POST | `/encountered-content/query/:characterId` | isles | progression |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/progression.ts:144 | progression.ts:QueryEncounteredContent | 200 401 403 |
| 110 | POST | `/encountered-content/query` | isles | progression |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/progression.ts:171 |  | 200 401 403 |
| 111 | POST | `/encountered-content/:characterId/query` | isles | progression |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/progression.ts:180 | progression.ts:QueryEncounteredContent | 200 401 403 |
| 112 | POST | `/encountered-content/:characterId` | isles | progression |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/progression.ts:207 | progression.ts:IsCharacterOwnedByUser, progression.ts:AddEncounteredContent | 200 401 403 |
| 113 | POST | `/encountered-content` | isles | progression |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/progression.ts:246 |  | 200 401 403 |
| 114 | POST | `/encountered-content/` | isles | progression |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/progression.ts:250 |  | 200 401 403 |
| 115 | GET | `/progression/objectives/:userId` | isles | progression |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/progression.ts:255 | system.ts:WidenHuntPassWindows, progression.ts:GetPersistedObjectivesForUser | 200 401 403 |
| 116 | GET | `/breadcrumbs/` | isles | progression |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/progression.ts:314 |  | 200 401 403 |
| 117 | GET | `/breadcrumbs/:characterId` | isles | progression |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/progression.ts:328 | progression.ts:IsCharacterOwnedByUser, progression.ts:GetBreadcrumbsForCharacterIdAndUserId | 200 401 403 |
| 118 | POST | `/breadcrumbs/:characterId` | isles | progression |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/progression.ts:363 | progression.ts:IsCharacterOwnedByUser, progression.ts:SetBreadcrumbsForCharacterIdAndUserId | 200 401 403 |
| 119 | POST | `/progression/:userId/:progressionId/:amount` | isles | progression |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/progression.ts:437 | progression.ts:IsHostRuntimeAuth, progression.ts:IsApprovedXpProgressionTrack, progression.ts:DescribeUnapprovedXpTrack, progression.ts:RefuseUnauthorizedHostWrite | 200 400 401 403 409 dyn |
| 120 | POST | `/progression/:userId` | isles | progression |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/progression.ts:640 | progression.ts:IsHostRuntimeAuth, progression.ts:RefuseUnauthorizedHostWrite, progression.ts:IsAuthorizedProgressionReporter, economyMonitor.ts:MonitorProgressionReport | 200 400 401 403 dyn |
| 121 | POST | `/progression/:userId/:progressionId/:rank/confirm/public` | isles | progression |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/progression.ts:753 | progression.ts:IsHostRuntimeAuth, progression.ts:RefuseUnauthorizedHostWrite, index.ts:GetRepositories, progression.ts:ConfirmPublicProgressionRank | 200 400 401 403 dyn |
| 122 | GET | `/progression/:userId` | isles | progression |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/progression.ts:829 | progression.ts:IsAuthoritativeServerFor, reforge.ts:FinishStaleReforgesForUser, progression.ts:TagHostWrite, progression.ts:GetPersistedProgressionForUser | 200 401 403 |
| 123 | GET | `/pjm/:userId` | isles | progression |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/progression.ts:893 | progression.ts:GetPlayerJourney, progression.ts:IsAuthoritativeServerFor, slayersPath.ts:GetUnlockedNodeIds, slayersPath.ts:GrantSlayersPathRewards | 200 401 403 |
| 124 | GET | `/weapon_tracker/:userId/:itemId` | isles | progression |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/progression.ts:943 |  | 200 401 403 |
| 125 | POST | `/pjm/:userId` | isles | progression |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/progression.ts:952 | progression.ts:IsHostRuntimeAuth, progression.ts:RefuseUnauthorizedHostWrite, progression.ts:SavePlayerJourney, slayersPath.ts:RestoreJourneyNodesShownLocked | 200 401 403 dyn |
| 126 | GET | `/pjm` | isles | progression |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/progression.ts:1041 | progression.ts:GetPlayerJourney, slayersPath.ts:MaskJourneyNodesForPlayer | 200 401 403 |
| 127 | GET | `/progression/tracked_objectives/:phxAccountId` | isles | progression |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/progression.ts:1063 |  | 200 401 403 |
| 128 | POST | `/progression/tracked_objectives/:phxAccountId` | isles | progression |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/progression.ts:1081 |  | 200 401 403 |
| 129 | GET | `/progression/:userId/:progressionId` | isles | progression |  | inline, HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/progression.ts:1110 | index.ts:GetRepositories | 200 401 403 |
| 130 | DELETE | `/progression/:userId/:progressionId` | isles | inventory-reforge |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/progression.ts:1135 | progression.ts:IsHostRuntimeAuth, progression.ts:RefuseUnauthorizedHostWrite, reforge.ts:IsReforgeableTrack, reforge.ts:ReforgeReset | 200 401 403 409 dyn |
| 131 | POST | `/progression/purchase/:userId/:progressionId/:amount` | isles | inventory-reforge |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/progression.ts:1181 | progression.ts:IsHostRuntimeAuth, progression.ts:RefuseUnauthorizedHostWrite, index.ts:GetRepositories, reforge.ts:PlanPurchaseRestore | 200 400 401 403 409 dyn |
| 132 | GET | `/loadout/:userId//all` | isles | characters-loadouts |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/loadout.ts:105 |  | 200 400 401 403 |
| 133 | GET | `/loadout/:userId/:characterId/all` | isles | characters-loadouts |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/loadout.ts:120 | loadout.ts:GetLoadoutStateForUserIdAndCharacterId | 200 400 401 403 |
| 134 | POST | `/loadout/:userId/:characterId/unlock/:numSlots` | isles | characters-loadouts |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/loadout.ts:173 | progression.ts:GetPlayerJourney, loadoutSlots.ts:ResolvePlayerJourneyTotalLoadoutSlots, loadout.ts:EnsureTotalLoadoutSlots, loadout.ts:GetLoadoutStateForUserIdAndCharacterId | 200 400 401 403 dyn |
| 135 | POST | `/loadout/:userId/:characterId/active/:index` | isles | characters-loadouts |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/loadout.ts:228 | loadout.ts:SetActiveLoadoutIndexForUserIdAndCharacterId, loadout.ts:GetLoadoutStateForUserIdAndCharacterId | 200 400 401 403 |
| 136 | POST | `/loadout/:userId/:characterId/:index` | isles | characters-loadouts |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/loadout.ts:267 | capture.ts:InventoryCaptureEnabled, capture.ts:CaptureEvent, loadout.ts:SetLoadoutDataForUserIdAndCharacterId, loadout.ts:GetLoadoutStateForUserIdAndCharacterId | 200 400 401 403 |
| 137 | GET | `/friends/api/public/friends/:accountId` | isles | chat |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/friends.ts:41 | index.ts:GetRepositories | 200 401 403 |
| 138 | POST | `/friends/api/public/friends/:accountId/:friendId` | isles | chat |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/friends.ts:55 | index.ts:GetRepositories | 200 204 400 401 403 |
| 139 | DELETE | `/friends/api/public/friends/:accountId/:friendId` | isles | chat |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/friends.ts:90 | index.ts:GetRepositories | 200 204 401 403 |
| 140 | GET | `/friends/api/public/blocklist/:accountId` | isles | chat |  |  | reference/1.14.7/ParadoxBackend/src/routes/friends.ts:103 |  | 200 |
| 141 | GET | `/friends/api/public/list/:namespace/:accountId/recentPlayers` | isles | chat |  |  | reference/1.14.7/ParadoxBackend/src/routes/friends.ts:108 |  | 200 |
| 142 | GET | `/friends/api/v1/:accountId/settings` | isles | chat |  |  | reference/1.14.7/ParadoxBackend/src/routes/friends.ts:113 |  | 200 |
| 143 | POST | `/slayerlink/availability` | isles | chat |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/friends.ts:138 |  | 200 401 403 |
| 144 | GET | `/slayerlink/status_good` | isles | chat |  | HasParadoxBackendAuth | reference/1.14.7/ParadoxBackend/src/routes/friends.ts:142 |  | 200 401 403 |
| 145 | POST | `/launcher/v1/auth/register` | isles | auth |  | launcher CORS (router) | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:141 | launcherAuth.ts:RegisterAccount | 200 LE dyn |
| 146 | POST | `/launcher/v1/auth/login` | isles | auth |  | launcher CORS (router) | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:148 | launcherAuth.ts:LoginWithPassword | 200 LE dyn |
| 147 | POST | `/launcher/v1/auth/refresh` | isles | auth |  | launcher CORS (router) | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:155 | launcherAuth.ts:RefreshSession | 200 LE dyn |
| 148 | POST | `/launcher/v1/auth/logout` | isles | auth |  | launcher CORS (router), HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:162 | launcherAuth.ts:Logout | 200 LE dyn |
| 149 | POST | `/launcher/v1/auth/logout-all` | isles | auth |  | launcher CORS (router), HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:166 | launcherAuth.ts:LogoutAll | 200 LE dyn |
| 150 | POST | `/launcher/v1/auth/discord/start` | isles | auth |  | launcher CORS (router) | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:170 | discordAuth.ts:StartDiscordAuth | 200 LE dyn |
| 151 | GET | `/launcher/v1/auth/discord/callback` | isles | auth |  | launcher CORS (router) | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:172 | discordAuth.ts:HandleDiscordCallback | 200 302 LE |
| 152 | POST | `/launcher/v1/auth/discord/complete` | isles | auth |  | launcher CORS (router) | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:192 | discordAuth.ts:CompleteDiscordLogin | 200 LE dyn |
| 153 | POST | `/launcher/v1/auth/email/verify` | isles | auth |  | launcher CORS (router) | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:201 | launcherEmail.ts:VerifyEmailWithPassword | 200 LE dyn |
| 154 | POST | `/launcher/v1/auth/email/resend` | isles | auth |  | launcher CORS (router) | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:208 | launcherEmail.ts:ResendEmailCode | 200 LE dyn |
| 155 | POST | `/launcher/v1/auth/password/forgot` | isles | auth |  | launcher CORS (router) | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:212 | launcherEmail.ts:RequestPasswordResetCode | 200 LE dyn |
| 156 | POST | `/launcher/v1/auth/password/reset` | isles | auth |  | launcher CORS (router) | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:216 | launcherEmail.ts:ResetPasswordWithCode | 200 LE dyn |
| 157 | POST | `/launcher/v1/account/discord/link/start` | isles | auth |  | launcher CORS (router), HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:225 | discordAuth.ts:StartDiscordLink | 200 LE dyn |
| 158 | POST | `/launcher/v1/account/discord/unlink/start` | isles | auth |  | launcher CORS (router), HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:229 | discordAuth.ts:StartDiscordUnlink | 200 LE dyn |
| 159 | POST | `/launcher/v1/account/discord/unlink` | isles | auth |  | launcher CORS (router), HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:233 | discordAuth.ts:ConfirmDiscordUnlink | 200 LE dyn |
| 160 | GET | `/launcher/v1/me` | isles | auth |  | launcher CORS (router), HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:237 | launcherAuth.ts:GetAccountView | 200 LE dyn |
| 161 | GET | `/launcher/v1/username` | isles | auth |  | launcher CORS (router) | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:242 | launcherAuth.ts:CheckUsernameAvailable | 200 LE dyn |
| 162 | POST | `/launcher/v1/username` | isles | auth |  | launcher CORS (router), HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:247 | launcherAuth.ts:SetUsername | 200 LE dyn |
| 163 | POST | `/launcher/v1/game-sessions` | isles | auth |  | launcher CORS (router), HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:251 | runtimeAuthorization.ts:ParseRuntimeChannel, launcherAuth.ts:RequestGameExchangeCode | 200 LE dyn |
| 164 | GET | `/launcher/v1/policy` | isles | auth |  | launcher CORS (router), HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:282 | launcherPolicy.ts:GetLauncherPolicy | 200 LE dyn |
| 165 | GET | `/launcher/v1/status` | isles | auth |  | launcher CORS (router) | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:286 | index.ts:GetPersistenceLifecycle, officialPlay.ts:ReadOfficialPlay | 200 LE dyn |
| 166 | GET | `/launcher/v1/legal` | isles | auth |  | launcher CORS (router) | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:297 | legalPolicy.ts:CurrentLegalRequirements | 200 LE dyn |
| 167 | GET | `/launcher/v1/notices` | isles | auth |  | launcher CORS (router) | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:300 | launcherNotices.ts:GetPublicLauncherNotices | 200 |
| 168 | GET | `/launcher/v1/account/consent` | isles | auth |  | launcher CORS (router), HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:303 | launcherAccount.ts:GetConsentState | 200 LE dyn |
| 169 | POST | `/launcher/v1/account/consent` | isles | auth |  | launcher CORS (router), HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:307 | launcherAccount.ts:AcceptConsent | 200 LE dyn |
| 170 | GET | `/launcher/v1/account/deletion` | isles | auth |  | launcher CORS (router), HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:312 | launcherAccount.ts:GetDeletionState | 200 LE dyn |
| 171 | POST | `/launcher/v1/account/deletion` | isles | auth |  | launcher CORS (router), HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:316 | launcherAccount.ts:RequestAccountDeletion | 200 LE dyn |
| 172 | DELETE | `/launcher/v1/account/deletion` | isles | auth |  | launcher CORS (router), HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/routes/launcherAuth.ts:320 | launcherAccount.ts:CancelAccountDeletion | 200 LE dyn |
| 173 | POST | `/reforged/api/Register` | isles | reforged-api |  |  | reference/1.14.7/ParadoxBackend/src/routes/reforgedApi.ts:237 | launcherApiRules.ts:Refusal, launcherApiRules.ts:NormalizeUsername, launcherApiRules.ts:IsInviteCode, launcherApiRules.ts:MakeAccountKey | 200 400 401 500 dyn |
| 174 | GET | `/reforged/api/GetUserInfo` | isles | reforged-api |  |  | reference/1.14.7/ParadoxBackend/src/routes/reforgedApi.ts:314 |  | 200 401 403 |
| 175 | GET | `/reforged/api/ServerStatus` | isles | reforged-api |  |  | reference/1.14.7/ParadoxBackend/src/routes/reforgedApi.ts:331 |  | 200 500 |
| 176 | GET | `/reforged/api/RegistrationStatus` | isles | reforged-api |  |  | reference/1.14.7/ParadoxBackend/src/routes/reforgedApi.ts:346 |  | 200 |
| 177 | POST | `/reforged/api/CreateInvite` | isles | reforged-api |  | HostOnly | reference/1.14.7/ParadoxBackend/src/routes/reforgedApi.ts:375 | launcherApiRules.ts:InviteNoteFrom, launcherApiRules.ts:MakeInviteCode | 200 400 403 500 |
| 178 | GET | `/reforged/api/InviteCodes` | isles | reforged-api |  | HostOnly | reference/1.14.7/ParadoxBackend/src/routes/reforgedApi.ts:402 |  | 200 403 |
| 179 | DELETE | `/reforged/api/InviteCode/:code` | isles | reforged-api |  | HostOnly | reference/1.14.7/ParadoxBackend/src/routes/reforgedApi.ts:416 | launcherApiRules.ts:IsInviteCode | 200 400 403 dyn |
| 180 | POST | `/reforged/api/SetAccountStatus` | isles | reforged-api |  | HostOnly | reference/1.14.7/ParadoxBackend/src/routes/reforgedApi.ts:428 | launcherApiRules.ts:Refusal | 200 400 403 404 |
| 181 | POST | `/reforged/api/Kick` | isles | reforged-api |  | HostOnly | reference/1.14.7/ParadoxBackend/src/routes/reforgedApi.ts:453 | launcherApiRules.ts:Refusal, launcherApiRules.ts:DisconnectAccount | 200 400 403 404 |
| 182 | GET | `/launcher/v1/runtime/:target/:channel/:platform` | isles | reforged-api |  |  | reference/1.14.7/ParadoxBackend/src/routes/launcherUpdates.ts:394 |  | 200 204 400 404 LE dyn |
| 183 | GET | `/launcher/v1/runtime/:channel/:platform` | isles | reforged-api |  |  | reference/1.14.7/ParadoxBackend/src/routes/launcherUpdates.ts:401 |  | 200 204 400 404 LE dyn |
| 184 | GET | `/launcher/v1/runtime/:target/:channel/:platform/download` | isles | reforged-api |  |  | reference/1.14.7/ParadoxBackend/src/routes/launcherUpdates.ts:406 |  | 200 400 404 429 LE dyn |
| 185 | GET | `/launcher/v1/runtime/:channel/:platform/download` | isles | reforged-api |  |  | reference/1.14.7/ParadoxBackend/src/routes/launcherUpdates.ts:413 |  | 200 400 404 429 LE dyn |
| 186 | GET | `/launcher/v1/guard-manifests/:channel/:platform` | rifts | guard |  |  | reference/1.14.7/ParadoxBackend/src/routes/launcherUpdates.ts:423 |  | 200 400 404 LE dyn |
| 187 | POST | `/launcher/v1/admin/guard-manifests/:channel/:platform` | rifts | guard |  | express.json() | reference/1.14.7/ParadoxBackend/src/routes/launcherUpdates.ts:439 |  | 200 201 400 401 403 409 429 |
| 188 | POST | `/launcher/v1/admin/updates/runtime/:target/:channel/:platform` | isles | reforged-api |  | express.raw() | reference/1.14.7/ParadoxBackend/src/routes/launcherUpdates.ts:477 |  | 200 201 400 401 403 409 413 429 |
| 189 | POST | `/launcher/v1/admin/updates/runtime/:target/:channel/:platform/extra` | isles | reforged-api |  | express.raw() | reference/1.14.7/ParadoxBackend/src/routes/launcherUpdates.ts:549 |  | 200 201 400 401 403 409 413 429 |
| 190 | GET | `/launcher/v1/runtime/:target/:channel/:platform/extra/:filename` | isles | reforged-api |  |  | reference/1.14.7/ParadoxBackend/src/routes/launcherUpdates.ts:610 |  | 200 400 404 429 LE dyn |
| 191 | GET | `/launcher/v1/admin/updates/serving` | isles | reforged-api |  |  | reference/1.14.7/ParadoxBackend/src/routes/launcherUpdates.ts:637 |  | 200 401 403 |
| 192 | POST | `/launcher/v1/admin/updates/serving` | isles | reforged-api |  | express.json() | reference/1.14.7/ParadoxBackend/src/routes/launcherUpdates.ts:643 |  | 200 400 401 403 |
| 193 | GET | `/launcher/v1/updates/:target/:arch/download` | isles | reforged-api |  |  | reference/1.14.7/ParadoxBackend/src/routes/launcherUpdates.ts:784 |  | 200 302 400 404 429 |
| 194 | GET | `/launcher/v1/updates/:target/:arch/files/:version/:file` | isles | reforged-api |  |  | reference/1.14.7/ParadoxBackend/src/routes/launcherUpdates.ts:820 |  | 200 404 429 |
| 195 | GET | `/launcher/v1/updates/:target/:arch/:currentVersion` | isles | reforged-api |  |  | reference/1.14.7/ParadoxBackend/src/routes/launcherUpdates.ts:842 |  | 200 204 400 |
| 196 | PUT | `/launcher/v1/logs/sessions/:launchSessionId/:fileName` | isles | reforged-api |  | HasLauncherAuth, express.raw() | reference/1.14.7/ParadoxBackend/src/routes/launcherLogs.ts:53 |  | 200 201 413 429 LE dyn |
| 197 | GET | `/admin/v1/launcher-logs` | isles | reforged-api | admin | requireSyncToken | reference/1.14.7/ParadoxBackend/src/routes/adminLauncherLogs.ts:82 |  | 200 401 |
| 198 | GET | `/admin/v1/launcher-logs/:userId/:launchSessionId/:fileName` | isles | reforged-api | admin | requireSyncToken | reference/1.14.7/ParadoxBackend/src/routes/adminLauncherLogs.ts:87 |  | 200 400 401 404 |
| 199 | POST | `/admin/v1/auth/login` | isles | reforged-api | admin | admin CORS (router) | reference/1.14.7/ParadoxBackend/src/controllers/admin.ts:48 AdminLogin |  | 200 401 429 503 |
| 200 | POST | `/admin/v1/auth/logout` | isles | reforged-api | admin | admin CORS (router), HasAdminAuth | reference/1.14.7/ParadoxBackend/src/controllers/admin.ts:92 AdminLogout |  | 200 401 403 dyn |
| 201 | GET | `/admin/v1/auth/discord/start` | isles | reforged-api | admin | admin CORS (router) | reference/1.14.7/ParadoxBackend/src/routes/admin.ts:86 | adminDiscordAuth.ts:StartAdminDiscordAuth | 200 302 503 |
| 202 | GET | `/admin/v1/auth/discord/callback` | isles | reforged-api | admin | admin CORS (router) | reference/1.14.7/ParadoxBackend/src/routes/admin.ts:97 | adminDiscordAuth.ts:HandleAdminDiscordCallback, adminDiscordAuth.ts:AdminDiscordBindingCookie | 200 302 503 |
| 203 | GET | `/admin/v1/me` | isles | reforged-api | admin | admin CORS (router), HasAdminAuth | reference/1.14.7/ParadoxBackend/src/controllers/admin.ts:104 AdminMe |  | 200 401 403 dyn |
| 204 | GET | `/admin/v1/online-players` | isles | reforged-api | admin | admin CORS (router), HasAdminAuth | reference/1.14.7/ParadoxBackend/src/controllers/admin.ts:144 ListOnlinePlayers |  | 200 401 403 dyn |
| 205 | GET | `/admin/v1/players` | isles | reforged-api | admin | admin CORS (router), HasAdminAuth | reference/1.14.7/ParadoxBackend/src/controllers/admin.ts:117 ListPlayers |  | 200 401 403 dyn |
| 206 | PATCH | `/admin/v1/players/:userId/access` | isles | reforged-api | admin | admin CORS (router), HasAdminAuth | reference/1.14.7/ParadoxBackend/src/controllers/admin.ts:166 UpdatePlayerAccess |  | 200 400 401 403 404 409 dyn |
| 207 | PATCH | `/admin/v1/players/:userId/roles` | isles | reforged-api | admin | admin CORS (router), HasAdminAuth | reference/1.14.7/ParadoxBackend/src/controllers/admin.ts:245 UpdatePlayerRoles |  | 200 400 401 403 404 dyn |
| 208 | DELETE | `/admin/v1/players/:userId` | isles | reforged-api | admin | admin CORS (router), HasAdminAuth | reference/1.14.7/ParadoxBackend/src/controllers/admin.ts:306 DeletePlayer |  | 200 400 401 403 404 409 dyn |
| 209 | GET | `/admin/v1/audit` | isles | reforged-api | admin | admin CORS (router), HasAdminAuth | reference/1.14.7/ParadoxBackend/src/controllers/admin.ts:359 ListAudit |  | 200 401 403 dyn |
| 210 | GET | `/admin/v1/operations/overview` | isles | reforged-api | admin | admin CORS (router), HasAdminAuth | reference/1.14.7/ParadoxBackend/src/controllers/adminOperations.ts:13 GetOperationsOverview |  | 200 401 403 dyn |
| 211 | PATCH | `/admin/v1/operations/policy` | isles | reforged-api | admin | admin CORS (router), HasAdminAuth | reference/1.14.7/ParadoxBackend/src/controllers/adminOperations.ts:38 UpdateOperationsPolicy |  | 200 400 401 403 409 dyn |
| 212 | GET | `/admin/v1/launcher-notices` | isles | reforged-api | admin | admin CORS (router), HasAdminAuth | reference/1.14.7/ParadoxBackend/src/controllers/launcherNotices.ts:50 GetAdminLauncherNotices |  | 200 401 403 LE dyn |
| 213 | PUT | `/admin/v1/launcher-notices` | isles | reforged-api | admin | admin CORS (router), HasAdminAuth | reference/1.14.7/ParadoxBackend/src/controllers/launcherNotices.ts:77 UpdateLauncherNotices |  | 200 401 403 LE dyn |
| 214 | GET | `/admin/v1/guard/sessions` | rifts | guard | admin | admin CORS (router), HasAdminAuth | reference/1.14.7/ParadoxBackend/src/controllers/adminGuard.ts:53 ListGuardSessions |  | 200 400 401 403 dyn |
| 215 | GET | `/admin/v1/guard/sessions/:guardSessionId` | rifts | guard | admin | admin CORS (router), HasAdminAuth | reference/1.14.7/ParadoxBackend/src/controllers/adminGuard.ts:76 GetGuardSession |  | 200 401 403 404 dyn |
| 216 | GET | `/admin/v1/guard/findings` | rifts | guard | admin | admin CORS (router), HasAdminAuth | reference/1.14.7/ParadoxBackend/src/controllers/adminGuard.ts:87 ListGuardFindings |  | 200 401 403 dyn |
| 217 | GET | `/admin/v1/activity/players/:userId` | isles | reforged-api | admin | admin CORS (router), HasAdminAuth | reference/1.14.7/ParadoxBackend/src/controllers/adminActivity.ts:211 GetActivityPlayer |  | 200 400 401 403 500 dyn |
| 218 | GET | `/admin/v1/activity/overview` | isles | reforged-api | admin | admin CORS (router), HasAdminAuth | reference/1.14.7/ParadoxBackend/src/controllers/adminActivity.ts:248 GetActivityOverview |  | 200 401 403 500 dyn |
| 219 | GET | `/admin/v1/activity/flags` | isles | reforged-api | admin | admin CORS (router), HasAdminAuth | reference/1.14.7/ParadoxBackend/src/controllers/adminActivity.ts:269 ListActivityFlags |  | 200 400 401 403 500 dyn |
| 220 | PATCH | `/admin/v1/activity/flags/:id` | isles | reforged-api | admin | admin CORS (router), HasAdminAuth | reference/1.14.7/ParadoxBackend/src/controllers/adminActivity.ts:287 AcknowledgeActivityFlag |  | 200 400 401 403 404 500 dyn |
| 221 | POST | `/admin/v1/players/:userId/tester-max` | isles | reforged-api | admin | admin CORS (router), HasAdminAuth | reference/1.14.7/ParadoxBackend/src/controllers/testerMax.ts:105 TesterMaxPlayer |  | 200 400 401 403 404 409 422 500 503 dyn |
| 222 | GET | `/admin/v1/player-logs` | isles | reforged-api | admin | admin CORS (router), HasAdminAuth | reference/1.14.7/ParadoxBackend/src/controllers/adminPlayerLogs.ts:54 ListPlayerLogs |  | 200 400 401 403 dyn |
| 223 | GET | `/admin/v1/player-logs/:userId/:launchSessionId/:fileName` | isles | reforged-api | admin | admin CORS (router), HasAdminAuth | reference/1.14.7/ParadoxBackend/src/controllers/adminPlayerLogs.ts:92 ReadPlayerLogFileRoute |  | 200 400 401 403 404 413 dyn |
| 224 | DELETE | `/admin/v1/player-logs/:userId/:launchSessionId` | isles | reforged-api | admin | admin CORS (router), HasAdminAuth | reference/1.14.7/ParadoxBackend/src/controllers/adminPlayerLogs.ts:115 DeletePlayerLogSessionRoute |  | 200 400 401 403 500 dyn |
| 225 | POST | `/launcher/v1/guard/sessions` | rifts | guard |  | HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/routes/launcherGuard.ts:19 | launcherGuard.ts:StartLauncherGuard | 200 LE dyn |
| 226 | POST | `/launcher/v1/guard/heartbeat` | rifts | guard |  | HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/routes/launcherGuard.ts:22 | launcherGuard.ts:SubmitLauncherGuardHeartbeat | 200 LE dyn |
| 227 | GET | `/launcher/v1/guard/sessions/:guardSessionId` | rifts | guard |  | HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/routes/launcherGuard.ts:25 | launcherGuard.ts:GetLauncherGuardStatus | 200 LE dyn |
| 228 | POST | `/mystic/v1/hunts/:sessionId/tickets/self` | rifts | p2p | p2p | P2P JSON 72 KiB (router), HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/p2p/routes/p2pHunts.ts:118 | p2pAdmission.ts:IssueSelfAdmissionTicket | 200 LE |
| 229 | POST | `/mystic/v1/hunts/:sessionId/host-ready` | rifts | p2p | p2p | P2P JSON 72 KiB (router), RequireHostCapability() | reference/1.14.7/ParadoxBackend/src/p2p/routes/p2pHunts.ts:122 | p2pAdmission.ts:MarkHostReady | 200 LE |
| 230 | POST | `/mystic/v1/hunts/:sessionId/event-batches` | rifts | guard | p2p | P2P JSON 72 KiB (router), RequireHostCapability() | reference/1.14.7/ParadoxBackend/src/p2p/routes/p2pHunts.ts:135 | antiCheat.ts:GetAntiCheatExtension | 200 LE |
| 231 | POST | `/mystic/v1/hunts/:sessionId/completion` | rifts | p2p | p2p | P2P JSON 72 KiB (router), RequireHostCapability() | reference/1.14.7/ParadoxBackend/src/p2p/routes/p2pHunts.ts:142 | antiCheat.ts:GetAntiCheatExtension | 200 LE |
| 232 | POST | `/mystic/v1/hunts/:sessionId/abort` | rifts | p2p | p2p | P2P JSON 72 KiB (router), RequireHostCapability() | reference/1.14.7/ParadoxBackend/src/p2p/routes/p2pHunts.ts:149 | p2pHuntStatus.ts:AbortHunt | 200 LE |
| 233 | POST | `/mystic/v1/hunts/:sessionId/end` | rifts | p2p | p2p | P2P JSON 72 KiB (router), RequireHostCapability() | reference/1.14.7/ParadoxBackend/src/p2p/routes/p2pHunts.ts:158 | p2pHuntStatus.ts:EndHunt | 200 LE |
| 234 | GET | `/mystic/v1/hunts/:sessionId/status` | rifts | p2p | p2p | P2P JSON 72 KiB (router), HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/p2p/routes/p2pHunts.ts:165 | p2pHuntStatus.ts:GetHuntStatus | 200 LE |
| 235 | GET | `/mystic/v1/hunts/:sessionId/reward/self` | rifts | p2p | p2p | P2P JSON 72 KiB (router), HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/p2p/routes/p2pHunts.ts:169 | p2pHuntStatus.ts:GetSelfHuntReward | 200 LE |
| 236 | POST | `/internal/v1/hunt-guard/shadow-comparisons` | rifts | guard | p2p | P2P JSON 72 KiB (router) | reference/1.14.7/ParadoxBackend/src/p2p/routes/p2pHunts.ts:173 | antiCheat.ts:GetAntiCheatExtension | 200 LE |
| 237 | POST | `/mystic/v1/hunts/:sessionId/lease` | rifts | p2p | p2p | P2P JSON 72 KiB (router), RequireHostCapability() | reference/1.14.7/ParadoxBackend/src/p2p/routes/p2pHunts.ts:177 | p2pHostLifecycle.ts:RenewHostLease | 200 LE |
| 238 | POST | `/mystic/v1/hunts/:sessionId/tickets/consume` | rifts | p2p | p2p | P2P JSON 72 KiB (router), RequireHostCapability() | reference/1.14.7/ParadoxBackend/src/p2p/routes/p2pHunts.ts:186 | p2pAdmission.ts:ConsumeAdmissionTicket | 200 LE |
| 239 | POST | `/mystic/v1/hunts/:sessionId/peers/:admissionId/disconnect` | rifts | p2p | p2p | P2P JSON 72 KiB (router), RequireHostCapability() | reference/1.14.7/ParadoxBackend/src/p2p/routes/p2pHunts.ts:200 | p2pAdmission.ts:RecordPeerDisconnected | 200 LE |
| 240 | PATCH | `/admin/v1/operations/p2p-emergency-stop` | rifts | p2p | p2p, admin | HasAdminAuth | reference/1.14.7/ParadoxBackend/src/p2p/controllers/p2pAdminOperations.ts:213 UpdateP2PEmergencyStop |  | 200 400 401 403 409 dyn |
| 241 | PATCH | `/admin/v1/operations/reward-rollout` | rifts | p2p | p2p, admin | HasAdminAuth | reference/1.14.7/ParadoxBackend/src/p2p/controllers/p2pAdminOperations.ts:298 UpdateRewardRollout |  | 200 400 401 403 409 dyn |
| 242 | PATCH | `/admin/v1/operations/coop-transport` | rifts | p2p | p2p, admin | HasAdminAuth | reference/1.14.7/ParadoxBackend/src/p2p/controllers/p2pAdminOperations.ts:266 UpdateCoopTransport |  | 200 400 401 403 409 dyn |
| 243 | PATCH | `/admin/v1/operations/alerts/:alertId/ack` | rifts | p2p | p2p, admin | HasAdminAuth | reference/1.14.7/ParadoxBackend/src/p2p/controllers/p2pAdminOperations.ts:181 AcknowledgeOperationsAlert |  | 200 400 401 403 404 dyn |
| 244 | POST | `/launcher/v1/p2p/hunts/:sessionId/host-bootstrap` | rifts | p2p | p2p | HasLauncherAuth, RequireActiveAccount | reference/1.14.7/ParadoxBackend/src/p2p/routes/p2pLauncher.ts:32 | p2pLauncher.ts:CreateHostBootstrap | 200 LE |
| 245 | POST | `/launcher/v1/p2p/hunts/:sessionId/join-bootstrap` | rifts | p2p | p2p | HasLauncherAuth, RequireActiveAccount | reference/1.14.7/ParadoxBackend/src/p2p/routes/p2pLauncher.ts:36 | p2pLauncher.ts:CreateJoinBootstrap | 200 LE |
| 246 | POST | `/launcher/v1/p2p/hunts/:sessionId/route-ready` | rifts | p2p | p2p | HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/p2p/routes/p2pLauncher.ts:40 | p2pLauncher.ts:MarkLocalRouteReady | 200 LE |
| 247 | POST | `/launcher/v1/p2p/hunts/:sessionId/transport-config` | rifts | p2p | p2p | HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/p2p/routes/p2pLauncher.ts:45 | p2pIce.ts:CreateIceTransportConfig | 200 LE |
| 248 | POST | `/launcher/v1/p2p/hunts/:sessionId/signal` | rifts | p2p | p2p | HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/p2p/routes/p2pLauncher.ts:49 | p2pIce.ts:SubmitIceSignal | 200 LE |
| 249 | GET | `/launcher/v1/p2p/control` | rifts | p2p | p2p | HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/p2p/routes/p2pLauncher.ts:53 | p2pControl.ts:PollP2PControl | 200 LE |
| 250 | POST | `/launcher/v1/p2p/control/:messageId/ack` | rifts | p2p | p2p | HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/p2p/routes/p2pLauncher.ts:61 | p2pControl.ts:AcknowledgeP2PControl | 200 LE |
| 251 | POST | `/launcher/v1/p2p/host-presence` | rifts | p2p | p2p | HasLauncherAuth, RequireActiveAccount | reference/1.14.7/ParadoxBackend/src/p2p/routes/p2pLauncher.ts:68 | p2pHostLifecycle.ts:SubmitHostPresence | 200 LE |
| 252 | POST | `/launcher/v1/p2p/host-offers/:offerId/respond` | rifts | p2p | p2p | HasLauncherAuth, RequireActiveAccount | reference/1.14.7/ParadoxBackend/src/p2p/routes/p2pLauncher.ts:72 | p2pHostLifecycle.ts:RespondToHostOffer | 200 LE |
| 253 | POST | `/launcher/v1/p2p/hunts/:sessionId/observation` | rifts | p2p | p2p | HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/p2p/routes/p2pLauncher.ts:80 | p2pHostLifecycle.ts:RecordParticipantObservation | 200 LE |
| 254 | POST | `/launcher/v1/realm-hosting/hubs/start` | rifts | realms | p2p | HasLauncherAuth, RequireActiveAccount(consent) | reference/1.14.7/ParadoxBackend/src/p2p/routes/realmHosting.ts:70 | realmHubLifecycle.ts:CreateRealmHubSession | 200 LE |
| 255 | POST | `/launcher/v1/realm-hosting/hubs/stop` | rifts | realms | p2p | HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/p2p/routes/realmHosting.ts:74 | realmHubLifecycle.ts:StopRealmHubSession | 200 LE |
| 256 | POST | `/launcher/v1/p2p/hunts/:sessionId/peers` | rifts | realms | p2p | HasLauncherAuth, RequireActiveAccount | reference/1.14.7/ParadoxBackend/src/p2p/routes/realmHosting.ts:78 | realmHubAdmission.ts:ListHubPeers | 200 LE |
| 257 | GET | `/launcher/v1/realm-hosting/status` | rifts | realms | p2p | HasLauncherAuth | reference/1.14.7/ParadoxBackend/src/p2p/routes/realmHosting.ts:83 | realmHostingStatus.ts:GetRealmHostingStatus | 200 LE |
| 258 | POST | `/launcher/v1/realms` | rifts | realms | realms | no-store (router), HasLauncherAuth (router), RequireActiveAccount(consent) | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:120 | realmInput.ts:ParseCreateRealmBody | 200 LE dyn |
| 259 | GET | `/launcher/v1/realms/mine` | rifts | realms | realms | no-store (router), HasLauncherAuth (router) | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:124 |  | 200 LE dyn |
| 260 | GET | `/launcher/v1/realms/listed` | rifts | realms | realms | no-store (router), HasLauncherAuth (router) | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:126 | realmInput.ts:ParseListQuery | 200 LE dyn |
| 261 | GET | `/launcher/v1/realms/by-name/:name` | rifts | realms | realms | no-store (router), HasLauncherAuth (router) | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:130 | realmInput.ts:ParseNameParam, realmInput.ts:ParseKnownHostKeyQuery | 200 LE dyn |
| 262 | GET | `/launcher/v1/realms/invite/:code` | rifts | realms | realms | no-store (router), HasLauncherAuth (router) | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:138 | realmInput.ts:ParseInviteCodeParam | 200 LE dyn |
| 263 | POST | `/launcher/v1/realms/join` | rifts | realms | realms | no-store (router), HasLauncherAuth (router), RequireActiveAccount | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:142 | realmInput.ts:ParseJoinBody | 200 LE dyn |
| 264 | POST | `/launcher/v1/realms/by-name/:name/leave` | rifts | realms | realms | no-store (router), HasLauncherAuth (router) | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:146 | realmInput.ts:ParseNameParam | 200 LE dyn |
| 265 | POST | `/launcher/v1/realms/mine/settings` | rifts | realms | realms | no-store (router), HasLauncherAuth (router), RequireActiveAccount(consent) | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:150 | realmInput.ts:ParseSettingsBody | 200 LE dyn |
| 266 | POST | `/launcher/v1/realms/mine/invites` | rifts | realms | realms | no-store (router), HasLauncherAuth (router), RequireActiveAccount(consent) | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:154 | realmInput.ts:ParseInviteBody | 200 LE dyn |
| 267 | GET | `/launcher/v1/realms/mine/invites` | rifts | realms | realms | no-store (router), HasLauncherAuth (router) | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:158 |  | 200 LE dyn |
| 268 | POST | `/launcher/v1/realms/mine/invites/:code/revoke` | rifts | realms | realms | no-store (router), HasLauncherAuth (router), RequireActiveAccount(consent) | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:160 | realmInput.ts:ParseInviteCodeParam | 200 LE dyn |
| 269 | POST | `/launcher/v1/realms/mine/members/:accountId/kick` | rifts | realms | realms | no-store (router), HasLauncherAuth (router), RequireActiveAccount(consent) | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:164 |  | 200 LE dyn |
| 270 | POST | `/launcher/v1/realms/mine/members/:accountId/ban` | rifts | realms | realms | no-store (router), HasLauncherAuth (router), RequireActiveAccount(consent) | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:168 | realmInput.ts:ParseBanBody | 200 LE dyn |
| 271 | POST | `/launcher/v1/realms/mine/members/:accountId/unban` | rifts | realms | realms | no-store (router), HasLauncherAuth (router), RequireActiveAccount(consent) | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:175 |  | 200 LE dyn |
| 272 | POST | `/launcher/v1/realms/mine/heartbeat` | rifts | realms | realms | no-store (router), HasLauncherAuth (router), RequireActiveAccount(consent) | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:179 | realmInput.ts:ParseHeartbeatBody | 200 LE dyn |
| 273 | POST | `/launcher/v1/realms/mine/release` | rifts | realms | realms | no-store (router), HasLauncherAuth (router), RequireActiveAccount(consent) | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:183 |  | 200 LE dyn |
| 274 | GET | `/launcher/v1/realms/mine/requests` | rifts | realms | realms | no-store (router), HasLauncherAuth (router) | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:186 |  | 200 LE dyn |
| 275 | POST | `/launcher/v1/realms/mine/requests/:accountId/approve` | rifts | realms | realms | no-store (router), HasLauncherAuth (router), RequireActiveAccount(consent) | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:188 |  | 200 LE dyn |
| 276 | POST | `/launcher/v1/realms/mine/requests/:accountId/deny` | rifts | realms | realms | no-store (router), HasLauncherAuth (router), RequireActiveAccount(consent) | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:192 |  | 200 LE dyn |
| 277 | POST | `/launcher/v1/realms/mine/members/add` | rifts | realms | realms | no-store (router), HasLauncherAuth (router), RequireActiveAccount(consent) | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:196 | realmInput.ts:ParseUsernameBody | 200 LE dyn |
| 278 | POST | `/launcher/v1/realms/mine/player-invites` | rifts | realms | realms | no-store (router), HasLauncherAuth (router), RequireActiveAccount(consent) | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:201 | realmInput.ts:ParseUsernameBody | 200 LE dyn |
| 279 | POST | `/launcher/v1/realms/mine/player-invites/:accountId/cancel` | rifts | realms | realms | no-store (router), HasLauncherAuth (router), RequireActiveAccount(consent) | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:205 |  | 200 LE dyn |
| 280 | GET | `/launcher/v1/realms/player-invites` | rifts | realms | realms | no-store (router), HasLauncherAuth (router) | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:209 |  | 200 LE dyn |
| 281 | POST | `/launcher/v1/realms/player-invites/:name/accept` | rifts | realms | realms | no-store (router), HasLauncherAuth (router), RequireActiveAccount | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:211 | realmInput.ts:ParseNameParam | 200 LE dyn |
| 282 | POST | `/launcher/v1/realms/player-invites/:name/decline` | rifts | realms | realms | no-store (router), HasLauncherAuth (router) | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:215 | realmInput.ts:ParseNameParam | 200 LE dyn |
| 283 | POST | `/launcher/v1/realms/presence/heartbeat` | rifts | realms | realms | no-store (router), HasLauncherAuth (router), RequireActiveAccount | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:220 | activityMonitor.ts:TouchGameSession | 200 LE dyn |
| 284 | POST | `/launcher/v1/realms/presence/leave` | rifts | realms | realms | no-store (router), HasLauncherAuth (router) | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:226 |  | 200 LE dyn |
| 285 | GET | `/launcher/v1/realms/presence` | rifts | realms | realms | no-store (router), HasLauncherAuth (router) | reference/1.14.7/ParadoxBackend/src/realms/routes/realmsLauncher.ts:228 |  | 200 LE dyn |

## Director routes

`reference/1.14.7/ParadoxDirector/src/app.ts`. It binds `127.0.0.1` unless `BIND_HOST` says otherwise and only the
backend calls it, so no route checks a credential. `/health/ready`, `/api/hub-status` and `/api/instances` are
mounted before the request context and the body parsers; an unmatched path gets Express's own 404. 5 registrations: 4 isles, 1 rifts.

| # | Method | Path | Side | Group | When | Middleware | Handler | Calls | Statuses |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | GET | `/health/ready` | isles | director |  |  | reference/1.14.7/ParadoxDirector/src/app.ts:26 |  | 200 dyn |
| 2 | GET | `/api/hub-status` | isles | director |  |  | reference/1.14.7/ParadoxDirector/src/app.ts:33 | gameservers.ts:GetPersistentHubStatus | 200 |
| 3 | GET | `/api/instances` | isles | director |  |  | reference/1.14.7/ParadoxDirector/src/app.ts:38 | gameservers.ts:InstanceSummaries | 200 |
| 4 | POST | `/api/matchmaker/handle-matchmaking-for-player` | isles | director |  | express.json() | reference/1.14.7/ParadoxDirector/src/routes/matchmaker.ts:20 | matchmaker.ts:HandleMatchmakingRequest | 200 500 |
| 5 | POST | `/api/matchmaker/resolve-hunt-launch-manifest` | rifts | p2p | p2p | express.json() | reference/1.14.7/ParadoxDirector/src/p2p/index.ts:23 | huntLaunchManifest.ts:ResolvePlayerHostedLaunchManifest | 200 400 |

## Counts

| Group | Side | Backend | Director |
| --- | --- | --- | --- |
| auth | isles | 49 | |
| reforged-api | isles | 54 | |
| characters-loadouts | isles | 8 | |
| inventory-reforge | isles | 8 | |
| progression | isles | 25 | |
| bounties-store | isles | 17 | |
| matchmaking-parties | isles | 18 | |
| chat | isles | 11 | |
| game-data | isles | 29 | |
| director | isles | | 4 |
| p2p | rifts | 24 | 1 |
| realms | rifts | 32 | |
| guard | rifts | 10 | |
| **Total** | | **285** | **5** |

## Groups

RU5's nine groups, as the JSON names them: `auth`; `reforged-api` (the Reforged API); `characters-loadouts`;
`inventory-reforge` (inventory, storage policy and reforge); `progression` (progression, mastery, escalation and
session supplies); `bounties-store` (bounties, store, Lady Luck and platinum); `matchmaking-parties` (multipliers,
matchmaking, parties and the game servers' posts); `chat`; `game-data` (game data and scripts). A file stays with one
group where it can, so one agent ports it; the exceptions and the calls that could go either way:

- **`auth`** takes all of `login.ts`, `eos.ts` and `launcherAuth.ts`, the launcher's `/launcher/v1/status`, `legal`
  and `notices` included, and `/key/generate` and `/key/consume` from `matchmaking.ts`, because RB6 names the
  encryption key store as auth's. Session supplies (0023) are refilled inside `POST /login`: the route is auth's, the
  rule (`sessionSupplies.ts`) progression's.
- **`reforged-api`** takes `/reforged/api/*`, `/maintenance/status` (0018), the live `/dauntless-status`, the launcher
  update, runtime and log routes, the admin API (`/admin/v1/*` but Guard's) and the health, metrics and release-gate
  probes. The probes could equally sit with RU4's skeleton.
- **`inventory-reforge`** takes two routes of `progression.ts`: `DELETE /progression/:userId/:progressionId` (the
  reforge reset) and `POST /progression/purchase/...` (its banked restore), patch 0032.
- **`progression`** keeps the game servers' XP and objective reports (`POST
  /progression/:userId/:progressionId/:amount`, `POST /progression/:userId`, patch 0005) with the rest of
  `progression.ts`. "The game servers' posts" in the `matchmaking-parties` group is read as the posts in
  `matchmaking.ts`, `/candidate/player/register` and `/candidate/player/alive`. Multipliers (0007) have no route of
  their own: they apply inside `POST /inventory` (`MultiplyRewardItems`) and the XP route (`MultiplyXpGrant`). RU5 ports
  `testMultipliers.ts` once, in `areas/matchmaking/multipliers.rs`, which both routes call, and logs its start-up line.
- **`bounties-store`** takes the cooldowns (`/cooldown/*`, the bounty and daily-patrol gates of `cooldownState.ts`) and
  the entitlements (`/entitlementsv2`, `/entitlementv2/:userId`).
- **`chat`** takes friends, Slayer link and guild (all of `friends.ts` and `guild.ts`, most of them stubs) and the
  `/vivox/login` stub (404), as the social surface beside DD21's XMPP plugin.
- **`game-data`** takes the stubs and static answers: `tuning.ts`, `/progression/config`, the gauntlet and trials
  payloads, the patch notes, the MOTD, the mailbox (`/all/`), cohorts, telemetry and crash-reporter stubs.
- **Rifts**: `realms` is `realms/` and `p2p/routes/realmHosting.ts` (the realm host's hub routes, its peer list under
  `/launcher/v1/p2p/` included); `guard` is `launcherGuard.ts`, the Guard manifests in `launcherUpdates.ts`, the
  `/admin/v1/guard/*` reads in `admin.ts`, and the evidence intake in `p2pHunts.ts` (`event-batches`,
  `/internal/v1/hunt-guard/shadow-comparisons`); `p2p` is the rest of `p2p/` and the Director's
  `resolve-hunt-launch-manifest`.

## Registrations that never answer

The script fails on a registration an earlier one always takes, unless `SHADOWED_OK` names it with its reason. They
are listed in the tables like any other, because RU5 must match what the client sees, which is the earlier route:

| Registration | Taken by |
| --- | --- |
| `GET /dauntless-status`, `reference/1.14.7/ParadoxBackend/src/routes/reforgedApi.ts:350` | `system.ts:35`, mounted first: the client gets Paradox's welcome text, never the Reforged name, version and source URL |
| `POST /inventory/:characterId/:changeList`, `inventory.ts:51` | The regex route at `inventory.ts:41`, registered first on purpose (the empty character id); the same answer |
| `GET /launcher/v1/runtime/:channel/:platform/download`, `launcherUpdates.ts:413` | `GET /launcher/v1/runtime/:target/:channel/:platform` (`:394`): 400, since the channel isn't a target, or the manifest when the channel is `client` or `server`. The legacy download URL doesn't work |
| `GET /playertreatments/`, `system.ts:218` | `/playertreatments` in the same array: one handler |
| `POST /encountered-content/`, `progression.ts:250` | `POST /encountered-content` (`:246`): the same answer |

Routes that overlap but pass on with `next()`: the four gauntlet regex routes in `system.ts` answer only when
`req.originHost` contains `gauntlet` and otherwise fall through (so `GET /progression/:userId/:progressionId` still
answers, after `HasParadoxBackendAuth` ran twice); the first handler of `/cooldown/:userId/:cooldownId` skips the
route for `batch`, which `PUT /cooldown/batch/:userId` (registered earlier) already takes.

## Dynamic registrations

The static pass follows `app.use()` mounts through imports, `require()` and router factories, and resolves template
paths from the file's string constants and array paths into one row each. Four calls hand a router to code it can't
follow; the script's `HOOKS` table names them, and the target function's first parameter is treated as that router:

| Call | Target | Routes |
| --- | --- | --- |
| `Extension.http.mountEarlyRoutes(app)` | `reference/1.14.7/ParadoxBackend/src/p2p/index.ts` `mountEarlyRoutes` | `p2pHuntsRouter`, 12 |
| `Extension.http.mountRoutes(app)` | `reference/1.14.7/ParadoxBackend/src/p2p/index.ts` `mountRoutes` | `p2pAdminRouter`, `p2pLauncherRouter`, `realmHostingRouter`, 18 |
| `MountRealmsModule(app)` | `reference/1.14.7/ParadoxBackend/src/realms/index.ts` `MountRealmsModule` | `CreateRealmsRouter(...)`, 28 |
| `GetP2PExtension().mountMatchmakerRoutes(matchmakingRouter)` | `reference/1.14.7/ParadoxDirector/src/p2p/index.ts` `mountMatchmakerRoutes` | 1, under `/api/matchmaker` |

A router whose routes no service reaches fails the check until it is mounted, hooked or listed in `UNREACHED_OK`
(empty today). The private anti-cheat module the p2p code loads from `src/anticheat` (`p2p/antiCheat.ts`) has no
HTTP surface in its interface, so it adds no route.

## Listeners that aren't Express

| Service | Listener | Where | Side | Group | Notes |
| --- | --- | --- | --- | --- | --- |
| backend | HTTP, `PORT` (3000), every interface unless `BIND_HOST` | `reference/1.14.7/ParadoxBackend/src/server.ts` | | | The Express app. The Director and the game servers call it here, over plain HTTP with their API keys |
| backend | HTTPS, `HTTPS_PORT` (3443) | `reference/1.14.7/ParadoxBackend/src/server.ts` | | | The same app with the `paradox` certificate (`PARADOX_CERT_PEM_PATH`, `PARADOX_KEY_PEM_PATH`, `PARADOX_KEY_PASSPHRASE`); not opened when `RUSTIC_ISLES_TLS_TERMINATED_BY_PROXY=on` |
| backend | XMPP over WebSocket, upgrade on the HTTPS listener (on the HTTP one behind a proxy) | `reference/1.14.7/ParadoxBackend/src/realtime/RealtimeGateway.ts`, `realtime/index.ts` | isles | chat | Only with `REALTIME_XMPP_ENABLED=true` (503 otherwise). Paths `//`, `/` and `/__ws/xmpp` (`REALTIME_WS_PATHS`; `REALTIME_XMPP_WS_PATHS` replaces them; 404 for any other), subprotocol `xmpp` echoed when offered. 403 for a host outside `REALTIME_XMPP_ALLOWED_HOSTS`, 503 at the global connection cap, 429 at the per-IP cap or after failed auth |
| backend | HTTP, `QOS_PORT` (80; `off` for none) | `reference/1.14.7/ParadoxBackend/src/server.ts` | isles | matchmaking-parties | Answers every method and path `200 <!DOCTYPE html><html><body>pong</body></html>`: the client's region ping for each `/candidate/regions` URL. A bind failure is logged and leaves readiness alone. RU5, now dainty-lass's `rustic-isles-matchmaking`: answers the same bytes on 127.0.0.1 only, on the manifest's `qos` port (61010 by default), and opens nothing public: publishing `QOS_PORT` waits on a decision (dainty-lass's `docs/security/rustic-isles-matchmaking.md`). The host kit writes `QOS_PORT=0`, so it starts only when a host sets a port |
| director | HTTP, `PORT`, `BIND_HOST` (127.0.0.1) | `reference/1.14.7/ParadoxDirector/src/server.ts` | | | The Express app above |

Neither service opens a UDP socket or a raw TCP server. The game servers' own ports, which the Director allocates and
starts, belong to the game binary, not to these services.
