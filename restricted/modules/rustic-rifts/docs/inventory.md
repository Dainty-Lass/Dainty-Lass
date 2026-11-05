<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Inventory of realms (RF1)

What rustic-rifts takes over from Mystic Paradox's 1.14.7 line, item by item, with the DD2 crate that should own
it: `rifts-admission`, `rifts-relay`, `rifts-realms`, `rifts-guard` (server side, linked by `isles-1-14-7`) and
`rifts-host` (client side, linked by DaintyLass's launcher).

Read from rustic-isles `main` at `ce1c42e` (Mystic Paradox `21b9ff5` plus Reforged's patches) and this repository's
`src/` at `c1c5dca`, which is byte for byte rustic-isles' `ParadoxTransport/src`. Paths below are relative to
`ParadoxBackend/src/` unless they name another folder. The routes are also in [`inventory.json`](inventory.json).

| Component | Routes | Settings | Other items |
| --- | --- | --- | --- |
| rifts-admission | 5 | 5 | ticket format MPT2, key pair and its generator |
| rifts-relay | 3 | 14 | relay credentials (Cloudflare, `turn-rest`), pair keys |
| rifts-realms | 46 | 48 | realms directory, hunt and hub lifecycle, host selection, leases |
| rifts-guard | 14 | 19 | Hunt Guard stubs, host writes, host policy, traffic limits, Launcher Guard |
| rifts-host | 0 | 17 launch variables (section 3) | transport ABI (29 exports), launch contract with the runtime, the host PC's files |

## 1. Routes

68 routes: the 67 that RU3a's draft inventory marks `rifts` (`docs/inventory/routes-1.14.7.json`, uncommitted on
`docs/ru3a-routes-1-14-7` when this was written) and the realms router's catch-all, which RU3a leaves out. The p2p routers exist only when the `p2p/` folder is present and `MYSTICPARADOX_P2P_MODULE` is not off
(`extensions/p2p.ts`).

Middleware every row inherits from `app.ts`: `RequestContextMiddleware`, the request log (lease and event-batches
logged only when refused), the host route census when on (`p2p/observability/hostRouteCensus.ts`, mounted before
every router) and the per-address `RequestLimiter` (mounted on `/mystic/v1/hunts` before the early routes; a valid
host capability gets its own bucket through `http.rateLimitBucket`). The p2pHunts router parses JSON itself
(`ParseP2PJson`, 72 KiB, strict) for `/mystic/v1/hunts/*` and the shadow path, and its `Handle()` charges every
capability call to the Guard request budget and, except lifecycle calls, the host and session traffic limits
(`p2p/security/guardTraffic.ts`, `guardTrafficDistributed.ts`). `RequireHostCapability(scope)` checks
`x-mystic-host-capability` against `p2pHostCapabilities` by SHA-256 digest, session id, expiry and scope
(`p2p/security/hostCapabilities.ts`). Failures use the launcher envelope `{error:{code,message,requestId}}`
(`p2p/security/p2pErrors.ts`, `realms/realmErrors.ts`); `P2PBodyErrorHandler` turns parser errors on those paths
into `GUARD_BATCH_TOO_LARGE` and `GUARD_BATCH_INVALID`.

#### `p2p/routes/p2pHunts.ts` (mountEarlyRoutes (before the shared JSON parser); when p2p module loaded)

| Method | Path | Middleware | Handler | Component |
| --- | --- | --- | --- | --- |
| POST | `/mystic/v1/hunts/:sessionId/tickets/self` | HasLauncherAuth | `p2p/controllers/p2pAdmission.ts:IssueSelfAdmissionTicket` | rifts-admission |
| POST | `/mystic/v1/hunts/:sessionId/host-ready` | RequireHostCapability("HOST_READY") | `p2p/controllers/p2pAdmission.ts:MarkHostReady` (hunt state STARTING to HOST_READY; lives in p2pAdmission.ts) | rifts-realms |
| POST | `/mystic/v1/hunts/:sessionId/event-batches` | RequireHostCapability("EVENTS_WRITE") | `p2p/antiCheat.ts:GetAntiCheatExtension().hunts.submitEvidence` (private anticheat module; the stub answers HUNT_EVIDENCE_NOT_COLLECTED) | rifts-guard |
| POST | `/mystic/v1/hunts/:sessionId/completion` | RequireHostCapability("COMPLETE_REQUEST") | `p2p/antiCheat.ts:GetAntiCheatExtension().hunts.complete` (private anticheat module; stub refuses) | rifts-guard |
| POST | `/mystic/v1/hunts/:sessionId/abort` | RequireHostCapability("ABORT") | `p2p/controllers/p2pHuntStatus.ts:AbortHunt` | rifts-realms |
| POST | `/mystic/v1/hunts/:sessionId/end` | RequireHostCapability("ABORT") | `p2p/controllers/p2pHuntStatus.ts:EndHunt` | rifts-realms |
| GET | `/mystic/v1/hunts/:sessionId/status` | HasLauncherAuth | `p2p/controllers/p2pHuntStatus.ts:GetHuntStatus` (includes the Guard verdict from hunts.result) | rifts-realms |
| GET | `/mystic/v1/hunts/:sessionId/reward/self` | HasLauncherAuth | `p2p/controllers/p2pHuntStatus.ts:GetSelfHuntReward` (reads hunts.rewardReceipt) | rifts-guard |
| POST | `/internal/v1/hunt-guard/shadow-comparisons` | - | `p2p/antiCheat.ts:GetAntiCheatExtension().hunts.recordShadowComparison` (x-hunt-guard-shadow-key checked by the module; stub refuses) | rifts-guard |
| POST | `/mystic/v1/hunts/:sessionId/lease` | RequireHostCapability("LEASE_HEARTBEAT") | `p2p/controllers/p2pHostLifecycle.ts:RenewHostLease` (every ~2 s per host session; logged only when refused) | rifts-realms |
| POST | `/mystic/v1/hunts/:sessionId/tickets/consume` | RequireHostCapability("PEER_ADMITTED") | `p2p/controllers/p2pAdmission.ts:ConsumeAdmissionTicket` | rifts-admission |
| POST | `/mystic/v1/hunts/:sessionId/peers/:admissionId/disconnect` | RequireHostCapability("PEER_ADMITTED") | `p2p/controllers/p2pAdmission.ts:RecordPeerDisconnected` | rifts-admission |

#### `p2p/routes/p2pAdmin.ts` (mountRoutes, after the /admin gate; when p2p module loaded and admin API on)

| Method | Path | Middleware | Handler | Component |
| --- | --- | --- | --- | --- |
| PATCH | `/admin/v1/operations/p2p-emergency-stop` | HasAdminAuth | `p2p/controllers/p2pAdminOperations.ts:UpdateP2PEmergencyStop` | rifts-realms |
| PATCH | `/admin/v1/operations/reward-rollout` | HasAdminAuth | `p2p/controllers/p2pAdminOperations.ts:UpdateRewardRollout` (Tier 1 reward grants) | rifts-guard |
| PATCH | `/admin/v1/operations/coop-transport` | HasAdminAuth | `p2p/controllers/p2pAdminOperations.ts:UpdateCoopTransport` (ICE or STEAM_EXPERIMENTAL) | rifts-relay |
| PATCH | `/admin/v1/operations/alerts/:alertId/ack` | HasAdminAuth | `p2p/controllers/p2pAdminOperations.ts:AcknowledgeOperationsAlert` (central-guard alerts) | rifts-guard |

#### `p2p/routes/p2pLauncher.ts` (mountRoutes; when p2p module loaded)

| Method | Path | Middleware | Handler | Component |
| --- | --- | --- | --- | --- |
| POST | `/launcher/v1/p2p/hunts/:sessionId/host-bootstrap` | HasLauncherAuth, RequireActiveAccount({consent:false}) | `p2p/controllers/p2pLauncher.ts:CreateHostBootstrap` (mints the host capability and the host ticket) | rifts-realms |
| POST | `/launcher/v1/p2p/hunts/:sessionId/join-bootstrap` | HasLauncherAuth, RequireActiveAccount({consent:false}) | `p2p/controllers/p2pLauncher.ts:CreateJoinBootstrap` (wraps IssueSelfAdmissionTicket) | rifts-admission |
| POST | `/launcher/v1/p2p/hunts/:sessionId/route-ready` | HasLauncherAuth | `p2p/controllers/p2pLauncher.ts:MarkLocalRouteReady` | rifts-realms |
| POST | `/launcher/v1/p2p/hunts/:sessionId/transport-config` | HasLauncherAuth | `p2p/controllers/p2pIce.ts:CreateIceTransportConfig` (STUN/TURN servers, relay credential, pair keys) | rifts-relay |
| POST | `/launcher/v1/p2p/hunts/:sessionId/signal` | HasLauncherAuth | `p2p/controllers/p2pIce.ts:SubmitIceSignal` (queued to the other side as ICE_SIGNAL) | rifts-relay |
| GET | `/launcher/v1/p2p/control` | HasLauncherAuth | `p2p/controllers/p2pControl.ts:PollP2PControl` (long poll: afterSequence, waitMs) | rifts-realms |
| POST | `/launcher/v1/p2p/control/:messageId/ack` | HasLauncherAuth | `p2p/controllers/p2pControl.ts:AcknowledgeP2PControl` | rifts-realms |
| POST | `/launcher/v1/p2p/host-presence` | HasLauncherAuth, RequireActiveAccount({consent:false}) | `p2p/controllers/p2pHostLifecycle.ts:SubmitHostPresence` (carries Launcher Guard sessions and artifact hashes) | rifts-realms |
| POST | `/launcher/v1/p2p/host-offers/:offerId/respond` | HasLauncherAuth, RequireActiveAccount({consent:false}) | `p2p/controllers/p2pHostLifecycle.ts:RespondToHostOffer` | rifts-realms |
| POST | `/launcher/v1/p2p/hunts/:sessionId/observation` | HasLauncherAuth | `p2p/controllers/p2pHostLifecycle.ts:RecordParticipantObservation` | rifts-realms |

#### `p2p/routes/realmHosting.ts` (mountRoutes; when p2p module loaded)

| Method | Path | Middleware | Handler | Component |
| --- | --- | --- | --- | --- |
| POST | `/launcher/v1/realm-hosting/hubs/start` | HasLauncherAuth, RequireActiveAccount({consent:true}) | `p2p/controllers/realmHubLifecycle.ts:CreateRealmHubSession` | rifts-realms |
| POST | `/launcher/v1/realm-hosting/hubs/stop` | HasLauncherAuth | `p2p/controllers/realmHubLifecycle.ts:StopRealmHubSession` | rifts-realms |
| POST | `/launcher/v1/p2p/hunts/:sessionId/peers` | HasLauncherAuth, RequireActiveAccount({consent:false}) | `p2p/controllers/realmHubAdmission.ts:ListHubPeers` (hub host polls for peers and their pair keys (AddPeerH)) | rifts-admission |
| GET | `/launcher/v1/realm-hosting/status` | HasLauncherAuth | `p2p/controllers/realmHostingStatus.ts:GetRealmHostingStatus` | rifts-realms |

#### `realms/routes/realmsLauncher.ts` (MountRealmsModule (app.ts); when MYSTICPARADOX_REALMS_MODULE on)

| Method | Path | Middleware | Handler | Component |
| --- | --- | --- | --- | --- |
| POST | `/launcher/v1/realms` | Cache-Control no-store, HasLauncherAuth, RequireActiveAccount({consent:true}) | `realms/realmsService.ts:RealmsService.createRealm` (201) | rifts-realms |
| GET | `/launcher/v1/realms/mine` | Cache-Control no-store, HasLauncherAuth | `realms/realmsService.ts:RealmsService.getMine` | rifts-realms |
| GET | `/launcher/v1/realms/listed` | Cache-Control no-store, HasLauncherAuth | `realms/realmsService.ts:RealmsService.listListed` (limit, offset) | rifts-realms |
| GET | `/launcher/v1/realms/by-name/:name` | Cache-Control no-store, HasLauncherAuth | `realms/realmsService.ts:RealmsService.getByName` (knownHostKey query) | rifts-realms |
| GET | `/launcher/v1/realms/invite/:code` | Cache-Control no-store, HasLauncherAuth | `realms/realmsService.ts:RealmsService.previewInvite` | rifts-realms |
| POST | `/launcher/v1/realms/join` | Cache-Control no-store, HasLauncherAuth, RequireActiveAccount({consent:false}) | `realms/realmsService.ts:RealmsService.join` | rifts-realms |
| POST | `/launcher/v1/realms/by-name/:name/leave` | Cache-Control no-store, HasLauncherAuth | `realms/realmsService.ts:RealmsService.leave` | rifts-realms |
| POST | `/launcher/v1/realms/mine/settings` | Cache-Control no-store, HasLauncherAuth, RequireActiveAccount({consent:true}) | `realms/realmsService.ts:RealmsService.updateSettings` | rifts-realms |
| POST | `/launcher/v1/realms/mine/invites` | Cache-Control no-store, HasLauncherAuth, RequireActiveAccount({consent:true}) | `realms/realmsService.ts:RealmsService.createInvite` (201) | rifts-realms |
| GET | `/launcher/v1/realms/mine/invites` | Cache-Control no-store, HasLauncherAuth | `realms/realmsService.ts:RealmsService.listInvites` | rifts-realms |
| POST | `/launcher/v1/realms/mine/invites/:code/revoke` | Cache-Control no-store, HasLauncherAuth, RequireActiveAccount({consent:true}) | `realms/realmsService.ts:RealmsService.revokeInvite` | rifts-realms |
| POST | `/launcher/v1/realms/mine/members/:accountId/kick` | Cache-Control no-store, HasLauncherAuth, RequireActiveAccount({consent:true}) | `realms/realmsService.ts:RealmsService.kick` | rifts-realms |
| POST | `/launcher/v1/realms/mine/members/:accountId/ban` | Cache-Control no-store, HasLauncherAuth, RequireActiveAccount({consent:true}) | `realms/realmsService.ts:RealmsService.ban` | rifts-realms |
| POST | `/launcher/v1/realms/mine/members/:accountId/unban` | Cache-Control no-store, HasLauncherAuth, RequireActiveAccount({consent:true}) | `realms/realmsService.ts:RealmsService.unban` | rifts-realms |
| POST | `/launcher/v1/realms/mine/heartbeat` | Cache-Control no-store, HasLauncherAuth, RequireActiveAccount({consent:true}) | `realms/realmsService.ts:RealmsService.heartbeat` | rifts-realms |
| POST | `/launcher/v1/realms/mine/release` | Cache-Control no-store, HasLauncherAuth, RequireActiveAccount({consent:true}) | `realms/realmsService.ts:RealmsService.release` | rifts-realms |
| GET | `/launcher/v1/realms/mine/requests` | Cache-Control no-store, HasLauncherAuth | `realms/realmsService.ts:RealmsService.listJoinRequests` | rifts-realms |
| POST | `/launcher/v1/realms/mine/requests/:accountId/approve` | Cache-Control no-store, HasLauncherAuth, RequireActiveAccount({consent:true}) | `realms/realmsService.ts:RealmsService.approveJoinRequest` | rifts-realms |
| POST | `/launcher/v1/realms/mine/requests/:accountId/deny` | Cache-Control no-store, HasLauncherAuth, RequireActiveAccount({consent:true}) | `realms/realmsService.ts:RealmsService.denyJoinRequest` | rifts-realms |
| POST | `/launcher/v1/realms/mine/members/add` | Cache-Control no-store, HasLauncherAuth, RequireActiveAccount({consent:true}) | `realms/realmsService.ts:RealmsService.addMemberByUsername` (legacy, launchers to 0.1.68) | rifts-realms |
| POST | `/launcher/v1/realms/mine/player-invites` | Cache-Control no-store, HasLauncherAuth, RequireActiveAccount({consent:true}) | `realms/realmsService.ts:RealmsService.invitePlayerByUsername` (201) | rifts-realms |
| POST | `/launcher/v1/realms/mine/player-invites/:accountId/cancel` | Cache-Control no-store, HasLauncherAuth, RequireActiveAccount({consent:true}) | `realms/realmsService.ts:RealmsService.cancelPlayerInvite` | rifts-realms |
| GET | `/launcher/v1/realms/player-invites` | Cache-Control no-store, HasLauncherAuth | `realms/realmsService.ts:RealmsService.listMyPlayerInvites` | rifts-realms |
| POST | `/launcher/v1/realms/player-invites/:name/accept` | Cache-Control no-store, HasLauncherAuth, RequireActiveAccount({consent:false}) | `realms/realmsService.ts:RealmsService.acceptPlayerInvite` | rifts-realms |
| POST | `/launcher/v1/realms/player-invites/:name/decline` | Cache-Control no-store, HasLauncherAuth | `realms/realmsService.ts:RealmsService.declinePlayerInvite` | rifts-realms |
| POST | `/launcher/v1/realms/presence/heartbeat` | Cache-Control no-store, HasLauncherAuth, RequireActiveAccount({consent:false}) | `realms/realmsService.ts:RealmsService.presenceHeartbeat` (also TouchGameSession (activity monitor)) | rifts-realms |
| POST | `/launcher/v1/realms/presence/leave` | Cache-Control no-store, HasLauncherAuth | `realms/realmsService.ts:RealmsService.presenceLeave` | rifts-realms |
| GET | `/launcher/v1/realms/presence` | Cache-Control no-store, HasLauncherAuth | `realms/realmsService.ts:RealmsService.presenceStatus` | rifts-realms |
| ALL | `/launcher/v1/realms/*` | Cache-Control no-store, HasLauncherAuth | `realms/routes/realmsLauncher.ts:CreateRealmsRouter (catch-all)` (NOT_FOUND in the launcher envelope instead of the app 404) | rifts-realms |

#### `routes/launcherGuard.ts` (app.ts, last router; when always)

| Method | Path | Middleware | Handler | Component |
| --- | --- | --- | --- | --- |
| POST | `/launcher/v1/guard/sessions` | HasLauncherAuth | `controllers/launcherGuard.ts:StartLauncherGuard` (shared: core file, also used for official play) | rifts-guard |
| POST | `/launcher/v1/guard/heartbeat` | HasLauncherAuth | `controllers/launcherGuard.ts:SubmitLauncherGuardHeartbeat` (shared) | rifts-guard |
| GET | `/launcher/v1/guard/sessions/:guardSessionId` | HasLauncherAuth | `controllers/launcherGuard.ts:GetLauncherGuardStatus` (shared) | rifts-guard |

#### `routes/admin.ts` (app.ts adminRouter; when admin API on)

| Method | Path | Middleware | Handler | Component |
| --- | --- | --- | --- | --- |
| GET | `/admin/v1/guard/sessions` | HasAdminAuth | `controllers/adminGuard.ts:ListGuardSessions` (shared) | rifts-guard |
| GET | `/admin/v1/guard/sessions/:guardSessionId` | HasAdminAuth | `controllers/adminGuard.ts:GetGuardSession` (shared) | rifts-guard |
| GET | `/admin/v1/guard/findings` | HasAdminAuth | `controllers/adminGuard.ts:ListGuardFindings` (shared) | rifts-guard |

#### `routes/launcherUpdates.ts` (app.ts launcherUpdatesRouter; when always)

| Method | Path | Middleware | Handler | Component |
| --- | --- | --- | --- | --- |
| GET | `/launcher/v1/guard-manifests/:channel/:platform` | isNonStableChannelAllowed | `routes/launcherUpdates.ts:(inline)` (shared: manifest covers core roles GAME, LAUNCHER, CONTENT, INTERNAL_SERVER, UDP_PROXY) | rifts-guard |
| POST | `/launcher/v1/admin/guard-manifests/:channel/:platform` | express.json 256 KiB, rate limit update-publish, publisher IP allowlist, publisher key | `routes/launcherUpdates.ts:(inline)` (shared) | rifts-guard |

#### `Director:p2p/index.ts` (routes/matchmaker.ts under /api/matchmaker; when Director p2p module loaded)

| Method | Path | Middleware | Handler | Component |
| --- | --- | --- | --- | --- |
| POST | `/api/matchmaker/resolve-hunt-launch-manifest` | express.json | `Director:p2p/huntLaunchManifest.ts:ResolvePlayerHostedLaunchManifest` (Director; backend calls it only with MYSTICPARADOX_HUNT_MANIFEST_SOURCE=director) | rifts-realms |

## 2. The seam with the rest of the backend

What RF2's trait surface stands in for. Today the core reaches realms through two doors: the `P2PExtension` hook
object (`extensions/p2p.ts`, implemented in `p2p/index.ts`, with a dedicated-only default when the module is
absent) and the `realms/` module's exports, which the core imports directly. The anticheat module sits behind a
second hook object, `AntiCheatExtension` (`p2p/antiCheat.ts`), whose real implementation is private.

### 2.1 Hooks the core calls (isles to rifts)

| Hook | Called from | Implementation | Component |
| --- | --- | --- | --- |
| `server.serviceRoles` | `config/serviceRole.ts`, `config/runtimeSafety.ts` | anticheat module's roles (none in the stub) | rifts-guard |
| `server.runtimeSafetyIssues` | `config/runtimeSafety.ts` | `p2p/index.ts:P2PRuntimeSafetyIssues` | rifts-realms |
| `server.startupSummary`, `start`, `stop` | `server.ts` | lifecycle monitor, realm event sink, alert dispatcher, census | rifts-realms |
| `server.ensureDatabase` | `persistence/mongo/indexes.ts` | `p2p/persistence/mongo/indexes.ts:EnsureP2PDatabase` | rifts-realms |
| `server.accountDeletionSteps` | `persistence/mongo/accountDeletionPlan.ts` | `p2p/persistence/mongo/accountDeletionSteps.ts` | rifts-realms |
| `http.mountEarlyRoutes`, `mountRoutes`, `mountErrorHandlers`, `metricClass`, `requestLogTags` | `app.ts` | the four p2p routers, `P2PBodyErrorHandler` | all |
| `http.authenticate` | `middleware/HasParadoxBackendAuth.ts` | `p2p/security/hostRuntimeAuth.ts:AuthenticateHostRuntimeRequest`: a hosted game server's `x-mystic-host-capability` (scope `GAME_SERVER_RUNTIME`) becomes `AuthData.IsPlayerHostRuntime` with `RuntimeRosterAccountIds` | rifts-realms |
| `http.rateLimitBucket` | `security/rateLimit.ts` | `p2p/security/hostCapabilities.ts:ResolveHostRateLimitBucket` | rifts-guard |
| `inventory.authorizeTransaction` | `routes/inventory.ts` (`POST /inventory`) | `p2p/security/p2pInventoryAuthorization.ts` (grant classes, `hostGrantPolicy.ts`, draft premium caps) | rifts-guard |
| `hostWrites.authorize`, `record` | `routes/progression.ts` (XP, MASTERY, OBJECTIVES, SLAYERS_PATH, REFORGE, RANK_CONFIRM, HUNT_PASS_XP), `routes/loadout.ts` (LOADOUT_UNLOCK), `routes/system.ts` and `escalationProgress.ts` (ESCALATION), `routes/system.ts`, `controllers/cooldowns.ts`, `cooldownState.ts` (COOLDOWN), `routes/system.ts`, `bountyState.ts` (BOUNTY), `routes/inventory.ts` (ITEM_SAVE), `controllers/progression.ts` | `p2p/security/hostWrites.ts`, `hubAuthorization.ts` | rifts-guard |
| `hostPolicy.refuses`, `noteWouldRefuse` | `controllers/progression.ts` (XP ceilings, full player-level track) | `p2p/security/hostPolicyMode.ts` | rifts-guard |
| `matchmaking.dedicatedHuntsDisabled`, `restoreCandidate`, `candidateStatus`, `partyInstance`, `routeHunt`, `cancelCandidate` | `controllers/matchmaking.ts` | `p2p/controllers/p2pMatchmaking.ts`, `p2pRealmRouting.ts` | rifts-realms |
| `matchmaking.filtersOfflinePartyMembers` | `routes/matchmaking.ts` | `p2p/config/p2pPolicy.ts:IsP2PTrainingRequest` | rifts-realms |
| `matchmaking.partyMemberLeft` | `controllers/party.ts` | `p2p/controllers/p2pMatchmaking.ts:P2PPartyMemberLeft` | rifts-realms |
| `launcher.policyFields` | `controllers/launcherPolicy.ts` (`GET /launcher/v1/policy`: `p2pEmergencyStop`, `coopHunts`) | `p2p/index.ts` | rifts-realms |
| `launcher.guardManifestRoles` | `routes/launcherUpdates.ts` | `TRANSPORT_MODULE`, `TRANSPORT_RUNTIME`, `ADMISSION_KEY` | rifts-guard |
| `operations.policyFieldsValid`, `overview` | `services/operationsPolicy.ts`, `controllers/adminOperations.ts` | `p2p/services/p2pOperationsPolicy.ts`, `p2p/controllers/p2pAdminOperations.ts` | rifts-realms |
| `releaseGates` | `routes/releaseGates.ts` (`GET /health/release-gates`) | `p2p/config/p2pReleaseGates.ts` | rifts-realms |

Realms entry points the core imports from `realms/index.ts`, all rifts-realms:

| Export | Called from | What for |
| --- | --- | --- |
| `MountRealmsModule` | `app.ts` | the router and the 15 s sweeper |
| `AuthorizeGameSessionRealm` | `controllers/launcherAuth.ts:RequestGameExchangeCode` (`POST /launcher/v1/game-sessions`) | Play with a realm; the code stores `realmId` |
| `BindRealmPresenceAtLogin` | `routes/eos.ts` (`POST /account/api/oauth/token`) | the game login that consumes that code |
| `GetRealmsService().touchPresence` | `routes/matchmaking.ts` (`POST /candidate/player/alive`, `GET /candidate/status`) | keeps the member's presence alive |
| `GetRealmsService().findByHubRoomId`, `isMember`, `isBanned`, `HubRoomId` | `realtime/RoomService.ts` | chat rooms of a realm's hubs |
| (deletion steps for `realms`, `realmInvites`, `realmPresence`) | `persistence/mongo/accountDeletionPlan.ts:CoreAccountDeletionSteps` | written in the core, not through a hook |

### 2.2 What rifts takes from the core (rifts to isles)

Everything `p2p/` and `realms/` import from outside their folders (tests and test kits left out). RF2's seam has to
hand these to the rifts crates.

| Area | Imports |
| --- | --- |
| Accounts and auth | `HasLauncherAuth`, `HasAdminAuth`, `RequireActiveAccount`, `IsCurrentLauncherAccountEligible` (`middleware/`), `ValidateMetagameJWTAndGetPayload` (`controllers/auth`), `VersionAtLeast` (`security/launcherVersion`), `IsRateLimited` (`security/rateLimit`), `VerifyOperationsStepUp` (`controllers/adminOperations`), `launcherAccounts.findByDisplayNameNormalized`, the `accounts` collection for display names (`realms/persistence/mongo/MongoRealmsRepository.ts`) |
| Sessions, parties, matchmaking | `MatchmakingResult`, `GetHubGameSessionId` (`controllers/matchmaking`), `GetOrCreateParty`, `parties.findById` (`controllers/party`), `RefusesGauntletRequest` (`matchmakingHuntPolicy`), `DedicatedServersConfigured` (`config/dedicatedServers`), `roomService` (`realtime/RoomService`), `TouchGameSession`, `RecordP2PHuntActivity` (`activity/activityMonitor`), the Director over HTTP (`DEPLOYSERVER_URL`: `handle-matchmaking-for-player` for the dedicated fallback, `resolve-hunt-launch-manifest`) |
| Hunt data and economy | `characters.findByCharacterIdAndUserId`, `inventoryTransactions.findRecord`, `ComputeInventoryRequestHash`, `INVALID_STARTER_CATALOG_IDS`, `InventoryLedgerAnnotator` (`controllers/inventory`), `InventoryBodyHasGrantOrSpend` (`validation`), `IsHuntPassXpTrack`, `ACTIVE_HUNTPASS_PROGRESSION_ID` (`controllers/progression`), `IsPrestigeGatedReward` (`controllers/wallet`), `IsPlatinumCatalogId` (`platinumWallet`), `CHALLENGE_REWARD_CURRENCIES`, `MAX_CHALLENGE_REWARD_PER_CLAIM`, `UtcDay` (`bountyState`), `tryLoadGameData` (`gameData/loader`), `EntitlementRecord`, `InventoryRecord`, the economy monitor settings (`diagnostics/economyLedgerMonitor`), `shared/hunt-manifest` (`CreateHuntResolver`, `LoadHuntTables`, `ParseTrialsRotationMinutes`, `p2p/completionMode:ResolvePlayerHostedManifest`) |
| Launcher feed and Guard | `ReadPublishedRuntimeManifest`, `ReadPublishedGuardArtifacts` (`routes/launcherUpdates`), `launcherGuard.find`, `findHealthyForAccount` and the record types (`persistence/contracts/LauncherGuardRepository`), `GuardRuleRequires`, `GuardVerdictRefuses`, `GuardRequiredByEnvironment`, `NoteGuardWouldRefuse` (`services/guardMode`) |
| Operations | `ReadOperationsPolicyRecord`, `NormalizeOperationsPolicyRecord`, `ResolveOperationsPolicy` (`services/operationsPolicy`), `ReleaseGate`, `IncrementMetric`, `SetMetric`, `MetricsSection`, `RegisterMetricsSection`, `GetRequestId`, `logger`, `MetagameServiceRole` |
| Persistence and errors | `GetRepositories`, `GetUnitOfWork` (transactions span core and module collections), `GetMongoDb`, `GetMongoClient`, `ReadMongoRuntimeConfig`, `Collections`, `AccountDeletionStep`, `LauncherApiError`, `SendLauncherError`, `RegisterLauncherErrorStatuses` |

The Director has a hook object of its own (`ParadoxDirector/src/extensions/p2p.ts`: `mountMatchmakerRoutes`),
used for the one route in section 1, and two realm-related switches of its own, `RAMSGATE_HUB` and `TRAINING_HUB`
(section 5.2).

## 3. The runtime's hosting hooks

`ParadoxRuntime/P2PHooks.h`, implemented in `ParadoxRuntime/p2p/PlayerHost.cpp` (stub `P2PHooksStub.cpp`). It lives
in rustic-isles' runtime; its contract is with `rifts-host`, which starts the process.

| Hook | Call site | Does |
| --- | --- | --- |
| `ClaimAuthMode` | `dllmain.cpp:26459` | claims `MYSTICPARADOX_AUTH_MODE=HUNT_CAPABILITY` |
| `ReadLaunch` | `dllmain.cpp:26493` | checks the launch: `-` first, `-MULTIHOME=127.0.0.1`, the environment below |
| `InstallServerHooks` | `dllmain.cpp:25852` | `AntiCheatHooks::Initialize` when `MYSTICPARADOX_HUNT_GUARD_EVENTS=1` |
| `DescribeRole` | `dllmain.cpp:25757` | ` BootstrapValid= HeadlessPlayerHost= HostedRole=` in `[InitRole]` |
| `StartServer` | `dllmain.cpp:2249` | control pipe thread (`MYS1 FENCE` from the launcher's pid only), `AntiCheatHooks::StartWorker` |
| `PrepareListen` | `Networking.cpp:585` | listen on 127.0.0.1 only |
| `ReportsReadiness`, `ReportReady` | `Networking.cpp:697`, `:714` | readiness over the ready pipe instead of stdout |
| `TickShutdown` | `dllmain.cpp:7571` | applies a fence: `MpQuitGame`, `TerminateProcess(0xE105)` after 5 s |
| `OnListenFailed` | `dllmain.cpp:8120`, `Networking.cpp:720` | requests the fence |
| `RequestHeader` | `dllmain.cpp:8693` | `x-mystic-host-capability` on every backend request |
| `ObserveServerEvent` | `dllmain.cpp:18577` | feeds `AntiCheatHooks::ObserveServerEvent` |
| `HostedRoleIsHub`, `MaxPlayersOverride`, `HubAfkSecondsOverride` | `dllmain.cpp:4005`, `:4015`, `:4792`, `:3950` | a realm hub's role, player cap and idle kick |

`ParadoxRuntime/p2p/AntiCheatHooks.h` (`Present`, `Initialize`, `StartWorker`, `ObserveServerEvent`) is the Hunt
Guard evidence producer's door; only the stub `AntiCheatHooksStub.cpp` is in the tree (rifts-guard).

The runtime calls nothing in the transport. The two meet in three places: the game server's UDP port on 127.0.0.1
(`server_udp_port` in the transport's config), the Hunt Guard event pipe `\\.\pipe\MysticHuntGuard.events.<suffix>`
(the anticheat producer writes, the transport's `GuardEventPipe` reads after checking `expected_server_pid`), and
the host capability, which both send to the backend.

Environment the launcher gives the hosted server (`ParadoxLauncher/src-tauri/src/p2p/session/host.rs`):

| Variable | Value |
| --- | --- |
| `MYSTICPARADOX_AUTH_MODE` | `HUNT_CAPABILITY` |
| `MYSTICPARADOX_BIND_ADDRESS` | `127.0.0.1` |
| `MYSTICPARADOX_HOST_RUNTIME_CAPABILITY` | the session's host capability, 43 characters |
| `MYSTICPARADOX_HOSTED_ROLE` | `HUNT` or `HUB` |
| `MYSTICPARADOX_SESSION_ID`, `MYSTICPARADOX_SESSION_GENERATION` | 36-character id, decimal generation |
| `MYSTICPARADOX_GAMESERVER_LAUNCH_ID` | 16 to 96 safe characters |
| `MYSTICPARADOX_READY_PIPE`, `MYSTICPARADOX_CONTROL_PIPE` | `\\.\pipe\MysticInternalServer.*` |
| `MYSTICPARADOX_GUARD_PIPE` | the Hunt Guard event pipe |
| `MYSTICPARADOX_HUNT_GUARD_EVENTS` | `0` or `1` |
| `MYSTICPARADOX_HUNT_GUARD_COMPLETION_MODE` | `FIRST_DEFEAT`, `END_OF_HUNT` or `NO_COMPLETION` |
| `MYSTICPARADOX_RUNTIME_PROFILE` | `development` or `production` |
| `MYSTICPARADOX_LAUNCHER_PID` | the only process the control pipe accepts |
| `MYSTICPARADOX_MAX_PLAYERS` | hub only, 1 to 64 |
| `MYSTICPARADOX_HUB_AFK_SECONDS` | hub only, 0 to 7200 |
| `MYSTICPARADOX_TRIALS_WEEK` | Trials hunts only |

Cleared from the inherited environment: `MYSTICPARADOX_ALLOW_DAMAGE_DEBUG`, `MYSTICPARADOX_DEBUG_DAMAGE_MULTIPLIER`.

The launcher code RF7 ports into `rifts-host` (`ParadoxLauncher/src-tauri/src/`, lines): `p2p/mod.rs` (337, the
co-op switch and files), `p2p/transport.rs` (1812, loading the DLL, ABI 1 to 3), `p2p/session.rs` (2153),
`p2p/session/host.rs` (579), `p2p/session/hub.rs` (926), `p2p/session/ice.rs` (790), `p2p/control.rs` (1261, the
control poll, host presence, offers), `p2p/realm.rs` (2973, the hosting panel, hubs, limits),
`servers/realm_directory.rs` (433), `servers/realm_presence.rs` (213). `launch/guard.rs` (Launcher Guard) is shared.

## 4. The transport's ABI

`src/paradox_transport_api.hpp`: every export is `extern "C" __declspec(dllexport) ... __cdecl`;
`MysticTransport_AbiVersion` answers 3. Version 1's start functions stay exported but always fail, because an ICE
session needs its secrets file. Every version 2 function is a wrapper over a default handle. Consumer: `rifts-host`.

| Export | Signature | Since |
| --- | --- | --- |
| `MysticTransport_AbiVersion` | `int ()` | 1 |
| `MysticTransport_Initialize` | `int (char* error, uint32_t error_size)` | 1 |
| `MysticTransport_Shutdown` | `void ()` | 1 |
| `MysticTransport_PeerId` | `uint64_t ()` | 1 |
| `MysticTransport_NetworkingReady` | `int ()` | 1 |
| `MysticTransport_StartHost` | `int (const char* public_key_path, const char* config_path, const char* capability_path, const char* host_ticket_path, char* error, uint32_t error_size)`; always 0 | 1 |
| `MysticTransport_StartJoin` | `int (const char* public_key_path, const char* config_path, const char* ticket_path, char* error, uint32_t error_size)`; always 0 | 1 |
| `MysticTransport_WaitReady` | `int (uint32_t timeout_ms, char* ready, uint32_t ready_size)`; 1, 0 timeout, -1 ended | 1 |
| `MysticTransport_SessionRunning` | `int ()` | 1 |
| `MysticTransport_LastSessionResult` | `int ()`; 0 clean, 2 bad input, 3 setup, 4 refused admission, 5 lease failed | 1 |
| `MysticTransport_StopSession` | `void ()` | 1 |
| `MysticTransport_ReadDiagnostic` | `int (char* line, uint32_t line_size)` | 1 |
| `MysticTransport_StartHostV2` | `int (public_key_path, config_path, capability_path, host_ticket_path, const char* secrets_path, char* error, uint32_t error_size)` | 2 |
| `MysticTransport_StartJoinV2` | `int (public_key_path, config_path, ticket_path, const char* secrets_path, char* error, uint32_t error_size)` | 2 |
| `MysticTransport_ReadSignal` | `int (char* to_account, uint32_t, char* kind, uint32_t, char* data, uint32_t)`; -1 buffer too small (data up to 4097) | 2 |
| `MysticTransport_SubmitSignal` | `int (const char* from_account_id, const char* kind, const char* data)` | 2 |
| `MysticTransport_ReadPeerEvent` | `int (char* line, uint32_t line_size)` | 2 |
| `MysticTransport_HostOpen` | `int (public_key_path, config_path, capability_path, host_ticket_path or null, secrets_path, char* error, uint32_t error_size)`; handle > 0 | 3 |
| `MysticTransport_JoinOpen` | `int (public_key_path, config_path, ticket_path, secrets_path, char* error, uint32_t error_size)` | 3 |
| `MysticTransport_WaitReadyH` | `int (int handle, uint32_t timeout_ms, char* ready, uint32_t ready_size)` | 3 |
| `MysticTransport_SessionRunningH` | `int (int handle)` | 3 |
| `MysticTransport_StopSessionH` | `int (int handle)` | 3 |
| `MysticTransport_LastSessionResultH` | `int (int handle)`; -1 unknown handle | 3 |
| `MysticTransport_ReadSignalH` | `int (int handle, char* to_account, uint32_t, char* kind, uint32_t, char* data, uint32_t)` | 3 |
| `MysticTransport_SubmitSignalH` | `int (int handle, const char* from_account_id, const char* kind, const char* data)`; held 20 s for an account not yet on the roster | 3 |
| `MysticTransport_ReadPeerEventH` | `int (int handle, char* line, uint32_t line_size)` | 3 |
| `MysticTransport_ReadDiagnosticH` | `int (int handle, char* line, uint32_t line_size)` | 3 |
| `MysticTransport_AddPeerH` | `int (int handle, const char* account_id, uint64_t peer_id, const char* pair_key_base64url)`; key 43 characters | 3 |
| `MysticTransport_RemovePeerH` | `int (int handle, const char* account_id, const char* reason)` | 3 |

Twelve exports from version 1, five added by 2, twelve by 3: 29. Version 3 runs up to 8 sessions per process;
`maximum_peers` is 1 to 4 for a hunt and up to 64 for a hub. The launcher also looks up
`MysticTransport_UpdateRelayCredentialH(int, const char*, const char*)` as optional (`p2p/transport.rs`, added
2026-10-17); no transport in either tree exports it, so a hub keeps the relay credential it started with.

Peer event lines: `PEER_CONNECTED`, `PEER_ADMITTED`, `PEER_LOST reason=`, `PEER_REMOVED reason= admitted=`,
`PEER_ADD_REJECTED reason=`, each with `account=` on a host.

The files a session opens, all written by the launcher into the session folder (section 6):

| File | Keys |
| --- | --- |
| `bridge.conf`, every session | `key_id`, `session_id`, `party_id`, `hunt_definition_id`, `bridge_instance_id`, `party_revision`, `host_peer_id`, `generation`, `build`, `bridge_protocol`, `virtual_port`, `ice_stun_urls`, `ice_turn_urls`, `force_relay` |
| `bridge.conf`, host | `backend_url`, `server_udp_port`, `maximum_peers`, `session_role` (`HUNT`, `HUB`), `host_local_client` (`1`, `0`), `guard_pipe_name`, `expected_server_pid`, `hunt_evidence` |
| `bridge.conf`, joiner | `local_udp_port`, `local_peer_id`, `local_account_id`, `host_account_id` |
| `transport.secret` | `turn_username`, `turn_credential`, `peer=<accountId>,<peerId>,<pairKey>` lines |
| `capability.secret` | the host capability |
| `ticket.secret` | the MPT2 admission ticket (`src/admission_ticket.cpp`) |
| public key | the game folder's `p2p-admission-public.xy` |

Backend calls the transport makes (`src/admission_client.cpp`, HTTPS only, `x-mystic-host-capability`):
`tickets/consume`, `host-ready`, `lease`, `event-batches`, `completion` and `peers/:admissionId/disconnect` under
`/mystic/v1/hunts/:sessionId/`. All run on the backend lane, off the packet thread (`src/backend_lane.cpp`).

## 5. Settings

### 5.1 Backend settings rifts owns

Read from the backend's environment. Secrets in bold; RF4 moves them to `secrets.env`.

| Setting | Default | Reader | Component |
| --- | --- | --- | --- |
| `MYSTICPARADOX_P2P_MODULE` | on; `0`, `false`, `off`, `no` turn it off | `extensions/p2p.ts` | rifts-realms |
| `MYSTICPARADOX_REALMS_MODULE` | off; `on`, `true`, `1` | `realms/realmsConfig.ts` | rifts-realms |
| `MYSTICPARADOX_REALMS_HOSTING` | off; `on`, `true`, `1` | `p2p/realm/realmHubPolicy.ts` | rifts-realms |
| `MYSTICPARADOX_REALMS_MAX_MEMBERS` | 1000 (2 to 5000) | `realms/realmsConfig.ts` | rifts-realms |
| `MYSTICPARADOX_REALMS_MAX_BANS` | 1000 (1 to 5000) | `realms/realmsConfig.ts` | rifts-realms |
| `MYSTICPARADOX_ALLOW_LOCAL_P2P_DEV` | false; `P2P_HUNTS_ENABLED` outside production needs it | `p2p/index.ts` | rifts-realms |
| `MYSTICPARADOX_HUNT_MANIFEST_SOURCE` | `local`; `director`; anything else stops startup | `p2p/huntManifest/localHuntManifest.ts` | rifts-realms |
| `P2P_HUNTS_ENABLED` | false; literal `true` only | `p2p/config/p2pPolicy.ts`, `p2p/index.ts` | rifts-realms |
| `P2P_ALLOWED_ACCOUNTS` | empty CSV | `p2p/config/p2pPolicy.ts` | rifts-realms |
| `P2P_ALLOW_ALL_ACCOUNTS` | false | `p2p/config/p2pPolicy.ts` | rifts-realms |
| `P2P_ALLOWED_HUNTS` | empty CSV | `p2p/config/p2pPolicy.ts` | rifts-realms |
| `P2P_ALLOW_ALL_RESOLVABLE_HUNTS` | false | `p2p/config/p2pPolicy.ts` | rifts-realms |
| `P2P_ALLOW_HIGH_RISK_HUNT_CLASSES` | false | `p2p/config/p2pPolicy.ts` | rifts-realms |
| `P2P_PRIVATE_PARTIES_ONLY` | true unless `false` | `p2p/config/p2pPolicy.ts` | rifts-realms |
| `P2P_MAX_PARTY_SIZE` | 4 (1 to 4) | `p2p/config/p2pPolicy.ts` | rifts-realms |
| `P2P_PERCENT_ROLLOUT` | 0 (0 to 100) | `p2p/config/p2pPolicy.ts` | rifts-realms |
| `P2P_TIER0_CANARY_ENABLED` | false | `p2p/config/p2pPolicy.ts` | rifts-realms |
| `P2P_DISABLE_DEDICATED_HUNTS` | false; also on when `DEPLOYSERVER_URL` is blank | `p2p/config/p2pPolicy.ts` | rifts-realms |
| `P2P_TRAINING_ENABLED` | false | `p2p/config/p2pPolicy.ts` | rifts-realms |
| `P2P_HOST_SELECTION_ENABLED` | false; literal `true` | `p2p/controllers/p2pMatchmaking.ts`, `p2p/index.ts` | rifts-realms |
| `P2P_RUNTIME_CHANNEL` | `stable` | `p2p/config/p2pHostPolicy.ts` | rifts-realms |
| `P2P_HOST_MIN_LAUNCHER_VERSION` | `0.1.35` | `p2p/config/p2pHostPolicy.ts` | rifts-realms |
| `P2P_GAME_EXE_SHA256` | known hash for `TARGET_CHANGELIST`; a set value wins | `p2p/config/p2pHostPolicy.ts` | rifts-realms |
| `P2P_INTERNAL_SERVER_SHA256` | from the runtime feed | `p2p/config/p2pHostPolicy.ts` | rifts-realms |
| `P2P_WINMM_SHA256` | from the runtime feed's `winmm.dll` | `p2p/config/p2pHostPolicy.ts` | rifts-realms |
| `P2P_TRANSPORT_MODULE_SHA256` (old name `P2P_STEAM_BRIDGE_SHA256`) | Guard role `TRANSPORT_MODULE` | `p2p/config/p2pHostPolicy.ts` | rifts-realms |
| `P2P_TRANSPORT_RUNTIME_SHA256` (old name `P2P_STEAM_RUNTIME_SHA256`) | Guard role `TRANSPORT_RUNTIME`, else `TRANSPORT_MODULE` | `p2p/config/p2pHostPolicy.ts` | rifts-realms |
| `P2P_HOST_MIN_FREE_MEMORY_MIB` | 4096 (1024 to 262144) | `p2p/config/p2pHostPolicy.ts` | rifts-realms |
| `P2P_HOST_MIN_COMMIT_HEADROOM_MIB` | 6144 (1024 to 262144) | `p2p/config/p2pHostPolicy.ts` | rifts-realms |
| `P2P_HOST_MIN_LOGICAL_CPUS` | 4 (2 to 256) | `p2p/config/p2pHostPolicy.ts` | rifts-realms |
| `P2P_HOST_MAX_CPU_PERCENT` | 85 (10 to 100) | `p2p/config/p2pHostPolicy.ts` | rifts-realms |
| `P2P_HOST_MAX_WORST_RTT_MS` | 250 (20 to 2000) | `p2p/config/p2pHostPolicy.ts` | rifts-realms |
| `P2P_HOST_MAX_JITTER_MS` | 80 (1 to 1000) | `p2p/config/p2pHostPolicy.ts` | rifts-realms |
| `P2P_HOST_MAX_PACKET_LOSS_PERCENT` | 10 (0 to 100) | `p2p/config/p2pHostPolicy.ts` | rifts-realms |
| `P2P_HOST_PRESENCE_MAX_AGE_MS` | 30000 (5000 to 300000) | `p2p/config/p2pHostPolicy.ts` | rifts-realms |
| `P2P_HOST_OFFER_LIFETIME_MS` | 15000 (5000 to 120000) | `p2p/config/p2pHostPolicy.ts` | rifts-realms |
| `P2P_HOST_STARTUP_LIFETIME_MS` | 120000 (30000 to 600000) | `p2p/config/p2pHostPolicy.ts` | rifts-realms |
| `P2P_HOST_LEASE_LIFETIME_MS` | 10000 (4000 to 60000) | `p2p/config/p2pHostPolicy.ts` | rifts-realms |
| `P2P_HOST_SUSPECT_LIFETIME_MS` | 15000 (5000 to 120000) | `p2p/config/p2pHostPolicy.ts` | rifts-realms |
| `P2P_REJOIN_LIFETIME_MS` | 60000 (10000 to 300000) | `p2p/config/p2pHostPolicy.ts` | rifts-realms |
| `P2P_MAX_RECONNECTS` | 3 (1 to 10) | `p2p/config/p2pHostPolicy.ts` | rifts-realms |
| `P2P_HUNT_DRAIN_GRACE_MS` | no cap; 30000 to 600000 sets one | `p2p/config/p2pHostPolicy.ts` | rifts-realms |
| `P2P_REALM_HUB_CAPABILITY_TTL_HOURS` | 24 (1 to 168) | `p2p/realm/realmHubPolicy.ts` | rifts-realms |
| `P2P_REALM_MIN_FREE_MEMORY_MIB_PER_SERVER` | 3072 (512 to 262144) | `p2p/realm/realmHubPolicy.ts` | rifts-realms |
| `P2P_BRIDGE_PROTOCOL` | 1 (1 to 255) | `p2p/realm/realmHubPolicy.ts`, `p2p/controllers/p2pMatchmaking.ts` | rifts-realms |
| `P2P_VIRTUAL_PORT` | 1 (1 to 999) | `p2p/realm/realmHubPolicy.ts`, `p2p/controllers/p2pMatchmaking.ts` | rifts-realms |
| `P2P_BACKEND_URL` | none, required: `https://host[:port]`, or `http://127.0.0.1:port` outside production | `p2p/controllers/p2pLauncher.ts` | rifts-realms |
| `P2P_HOST_SERVER_UDP_PORT` | 8791 (1024 to 65535), 32 ports from there | `p2p/controllers/p2pLauncher.ts` | rifts-realms |
| `P2P_ADMISSION_KEY_ID` | none, required (safe id, at most 32) | `p2p/security/admissionTickets.ts`, `p2p/controllers/p2pLauncher.ts` | rifts-admission |
| **`P2P_ADMISSION_SIGNING_PRIVKEY_B64`** | none; base64 PEM, P-256 | `p2p/security/admissionTickets.ts` | rifts-admission |
| `P2P_ADMISSION_VERIFY_PUBKEY_B64` | none; base64 PEM | `p2p/security/admissionTickets.ts` | rifts-admission |
| `P2P_ADMISSION_TICKET_TTL_SECONDS` | 75 | `p2p/security/admissionTickets.ts` | rifts-admission |
| `P2P_ADMISSION_PUBLIC_XY_SHA256` | Guard role `ADMISSION_KEY` | `p2p/config/p2pHostPolicy.ts` | rifts-admission |
| `P2P_ICE_STUN_URLS` | `stun:stun.cloudflare.com:3478` | `p2p/config/p2pIcePolicy.ts` | rifts-relay |
| `P2P_TURN_PROVIDER` | `cloudflare`; `turn-rest`, `none` | `p2p/config/p2pIcePolicy.ts` | rifts-relay |
| `P2P_CLOUDFLARE_TURN_KEY_ID` | none | `p2p/config/p2pIcePolicy.ts` | rifts-relay |
| **`P2P_CLOUDFLARE_TURN_API_TOKEN`** | none | `p2p/config/p2pIcePolicy.ts` | rifts-relay |
| `P2P_TURN_REST_URLS` | none; `turn:` or `turns:` URLs, CSV | `p2p/config/p2pIcePolicy.ts` | rifts-relay |
| **`P2P_TURN_REST_SECRET`** | none; at least 16 characters (DD24's shared secret) | `p2p/config/p2pIcePolicy.ts` | rifts-relay |
| `P2P_CLOUDFLARE_ANALYTICS_ACCOUNT_ID` | none | `p2p/config/p2pIcePolicy.ts` | rifts-relay |
| **`P2P_CLOUDFLARE_ANALYTICS_API_TOKEN`** | none | `p2p/config/p2pIcePolicy.ts` | rifts-relay |
| **`P2P_TRANSPORT_KEY_SECRET`** | none; at least 32 characters; without it ICE is off | `p2p/config/p2pIcePolicy.ts` | rifts-relay |
| `P2P_TURN_MAX_TTL_SECONDS` | 10800 (300 to 172800) | `p2p/config/p2pIcePolicy.ts` | rifts-relay |
| `P2P_REALM_HUB_TURN_TTL_SECONDS` | 86400 (300 to 172800) | `p2p/config/p2pIcePolicy.ts` | rifts-relay |
| `P2P_RELAY_MONTHLY_QUOTA_GB` | 10 (0 = none, to 100000) | `p2p/config/p2pIcePolicy.ts` | rifts-relay |
| `P2P_ICE_CONFIGS_PER_HOUR` | 30 (1 to 1000) | `p2p/config/p2pIcePolicy.ts` | rifts-relay |
| `P2P_FORCE_SDR` | false; startup warning when hunts are on without it | `p2p/config/p2pPolicy.ts` | rifts-relay |
| `MYSTICPARADOX_ANTICHEAT_MODULE` | on, but the module folder is absent, so stubs | `p2p/antiCheat.ts` | rifts-guard |
| `MYSTICPARADOX_HOST_POLICY` | `observe`; `enforce` | `p2p/security/hostPolicyMode.ts` | rifts-guard |
| `MYSTICPARADOX_GUARD_MODE` | `observe`; `enforce`; any other value is enforce | `services/guardMode.ts` | rifts-guard |
| `MYSTICPARADOX_HUNT_EVIDENCE` | on at `P2P_TRUST_TIER=1`, off at 0 | `p2p/config/huntEvidence.ts` | rifts-guard |
| `MYSTICPARADOX_HOST_ROUTE_CENSUS` | on; `0`, `false`, `off`, `no` | `p2p/observability/hostRouteCensus.ts` | rifts-guard |
| `P2P_HOST_WRITES` | on; `0`, `false`, `off`, `no` | `p2p/security/hostWrites.ts` | rifts-guard |
| `P2P_LAUNCHER_GUARD_REQUIRED` | false | `services/guardMode.ts`, `p2p/controllers/realmHubLifecycle.ts` | rifts-guard |
| `P2P_DISTRIBUTED_TRAFFIC_LIMITER` | false; required in production with hunts on | `p2p/security/guardTrafficDistributed.ts`, `p2p/index.ts` | rifts-guard |
| `P2P_TRUST_TIER` | 0; only `1` raises it | `p2p/config/p2pPolicy.ts`, `huntEvidence.ts`, `p2pReleaseGates.ts` | rifts-guard |
| `P2P_TIER1_REWARDS_ENABLED` | false | `p2p/config/p2pPolicy.ts` | rifts-guard |
| `P2P_TIER1_SHADOW_APPROVED` | false | `p2p/config/p2pPolicy.ts` | rifts-guard |
| `P2P_TIER1_ALLOWED_ACCOUNTS` | empty CSV | `p2p/config/p2pPolicy.ts` | rifts-guard |
| `P2P_TIER1_ALLOWED_HUNTS` | empty CSV | `p2p/config/p2pPolicy.ts` | rifts-guard |
| `HUNT_GUARD_RULE_VERSION` | `observe-v1` | `p2p/controllers/p2pMatchmaking.ts` | rifts-guard |
| `HUNT_GUARD_REWARD_TABLE_VERSION` | `observe-no-rewards-v1` | `p2p/controllers/p2pMatchmaking.ts` | rifts-guard |
| `MYSTICPARADOX_OPERATIONS_ALERT_WEBHOOK_URL` | none; https in production | `p2p/services/operationsAlerts.ts` | rifts-guard |
| **`MYSTICPARADOX_OPERATIONS_ALERT_WEBHOOK_TOKEN`** | none | `p2p/services/operationsAlerts.ts` | rifts-guard |
| `MYSTICPARADOX_OPERATIONS_ALERT_TIMEOUT_MS` | 5000 | `p2p/services/operationsAlerts.ts` | rifts-guard |
| `MYSTICPARADOX_ALLOW_LOCAL_ALERT_SINK` | false | `p2p/services/operationsAlerts.ts` | rifts-guard |

### 5.2 Isles settings rifts reads

| Setting | Default | Reader | Used for |
| --- | --- | --- | --- |
| `DEPLOYSERVER_URL` | none | `config/dedicatedServers.ts`, `p2pHostLifecycle.ts`, `p2pMatchmaking.ts` | dedicated fallback, Director manifest |
| `TARGET_CHANGELIST` | 647472 in `realmHubPolicy.ts` | `p2pHostPolicy.ts`, `realmHubPolicy.ts` | build check, known game hash |
| `NODE_ENV` | none | `p2p/index.ts`, `p2pLauncher.ts`, `operationsAlerts.ts` | production rules |
| `PARADOX_HUNT_DATA_DIR` | `ParadoxDirector/game-data` | `p2p/huntManifest/localHuntManifest.ts` | local hunt tables |
| `TRIALS_ROTATION_MINUTES` | Trials rotation | `p2p/huntManifest/localHuntManifest.ts` | local hunt tables |
| `RAMSGATE_HUB` (Director) | on | `ParadoxDirector/src/controllers/gameservers.ts` | off when realms run their own Ramsgate |
| `TRAINING_HUB`, `TRAINING_ON_DEMAND` (Director) | on, off | `ParadoxDirector/src/controllers/gameservers.ts` | dedicated Training only as a fallback |

`docs/SELF_HOSTING.md` tells hosts to copy these from `ParadoxBackend/.env.p2p.example`, which is not in the tree;
`.env.example` names only `MYSTICPARADOX_GUARD_MODE`. The key generator is
`ParadoxBackend/scripts/p2p/generate-admission-key.mjs --output <dir> --key-id <id>`: P-256, the env lines and
`p2p-admission-public.xy` (rifts-admission; RF4 makes it a command).

## 6. What a realm host's PC keeps

Nothing in the transport persists: it reads the session files and writes no file of its own (its self-test writes
`selftest-<name>-<pid>/` next to the exe). Every path below comes from the launcher (`ParadoxLauncher/src-tauri/src`).

| What | Where | Lifetime | Component |
| --- | --- | --- | --- |
| `ParadoxTransport.dll` (Guard roles `TRANSPORT_MODULE`, `TRANSPORT_RUNTIME`), `p2p-admission-public.xy` (`ADMISSION_KEY`, the admission public key) | game folder, from the runtime feed's extra files; names from the signed Guard manifest | until a repair replaces them | rifts-host |
| `MysticSteam.dll`, `steam_api64.dll` | game folder, extra files (`scripts/p2p/runtime-extra-names.json`) | private Steam transport; not built here | none |
| `capability.secret`, `ticket.secret`, `transport.secret`, `bridge.conf` | `<app local data>/p2p-sessions/<sessionId>-<generation>-<16 hex>/` | removed with the folder when the session ends (`session.rs:cleanup_directory`) | rifts-host |
| session logs | `<launcher log dir or app local data>/p2p-logs/<sessionId>-<generation>-<role>-<12 hex>.log`, bounded | kept | rifts-host |
| `realm-host.log` | `%LOCALAPPDATA%\MysticParadox\Logs\` | kept | rifts-host |
| `p2p-host-settings.json`: `control_cursors` (account to last control sequence) | app config dir | kept | rifts-host |
| `realm-host-limits.json`: `version`, per server `max_players`, `max_servers`, `afk_seconds` (`.tmp`, `.bad` beside it) | app config dir | kept | rifts-host |
| realm entries in `servers.json`: kind `ParadoxRealm`, `realm_name`, `host_public_key` | app config dir | kept | rifts-host (server list is DaintyLass's) |
| `DISABLE_COOP_TRANSPORT.flag` | `%LOCALAPPDATA%\MysticParadox\` | test override, by hand | rifts-host |
| `guard-rollback.json` (Guard manifest anti-rollback floor); the Launcher Guard Ed25519 key lives in memory only | app config dir | kept | rifts-guard (shared) |
| `host-log-offsets.json` | launcher log dir | kept | launcher log upload, not rifts |
| `HUB_MAX_PLAYERS.txt`, `HUB_AFK_TIMEOUT_SECONDS.txt` | next to the game; the hub env overrides them | by hand | isles runtime |

There is no host key pair. The realm's `hostPublicKey` is 32 random bytes in hex, made at claim time
(`p2p/realm.rs:create_realm`) and pinned by joiners in their own `servers.json`; it is compared, never used to sign.
Pair keys and relay credentials arrive per session from `transport-config` and die with the session folder. The
account's refresh token stays DPAPI-protected in the launcher and never reaches the transport.

## 7. What the backend stores for realms

MongoDB only: `p2p/` and `realms/` call `GetMongoDb` directly, so the SQLite provider of a "local realm"
(`persistence/sqlite`, `realm-data/realm.sqlite`) has no realm tables at all. RF5 gives them their first non-Mongo
home in `store`. Contracts: `p2p/persistence/contracts/*.ts`, `realms/realmTypes.ts`; indexes:
`p2p/persistence/mongo/indexes.ts`, `realms/persistence/mongo/indexes.ts`.

| Collection | Key and indexes | Fields | Component |
| --- | --- | --- | --- |
| `p2pHuntSessions` | `sessionId`; `activePartyKey` unique; `roster.accountId`, `createdAt`, `state`+deadlines | `sessionId`, `generation`, `state` (14 states), `stateVersion`, `partyId`, `activePartyKey`, `partyRevision`, `huntDefinitionId`, `gameMode`, `build`, `bridgeProtocol`, `virtualPort`, `hostAccountId`, `hostSteamId`, `bridgeInstanceId`, `roster[]` (`accountId`, `steamId`, `candidateId`, `reconnectEpoch`, `admissionState`, `localBridgeInstanceId`, `localBridgePort`, `localBridgeReadyAt`, `disconnectedAt`, `rejoinDeadline`, `returnedToHubAt`), `lobbyId`, `coopTransport`, `leaseExpiresAt`, `createdAt`, `expiresAt`, `metadataNonce`, `sessionKind` (`REALM_HUB`), `hubKind`, `realmId`, `hostParticipates`, `hostLocalBridgePort`, `serverUdpPort`, `hostLocalBridgeReadyAt`, `hubRoomId`, `maxPeers`, `matchmakerHuntId`, `mapPath`, `behemothPath`, `expectedPlayerString`, offer, startup, heartbeat, suspect, recovery, fallback, drain and terminal fields, `triedHostAccountIds`, `cancelledByAccountId`, `contentDigest`, `ruleVersion`, `rewardTableVersion`, `trustTier`, `rewardPolicyDigest`, `seedCommitment`, `frozenLoadouts[]` | rifts-realms |
| (same documents) | | `guardCompletionMode`, `guardSeed`, `guardLastSequence`, `guardHeadHash`, `guardEncounterState`, `guardDisposition`, Guard counters (`guardFindingCount`, `guardBatchCount`, `guardEventCount`, `guardEventBytes`, monotonic times, damage totals, `guardEventTypes`, `guardHasUnattributedDamage`, `guardLastReceivedAt`) | rifts-guard |
| `p2pSteamLinks` | `_id` account; `steamId` unique | `steamId`, `status`, `revision`, `verifiedAt`, `lastUsedAt`, `verificationMethod` (`ICE_ACCOUNT_DERIVED` for ICE) | rifts-admission |
| `p2pHostCapabilities` | `secretDigest` unique; `sessionId`+`generation` | `capabilityId`, `secretDigest`, `sessionId`, `generation`, `hostAccountId`, `hostSteamId`, `bridgeInstanceId`, `scopes[]` (7), `issuedAt`, `expiresAt`, `revokedAt`, `lastSequence` | rifts-realms |
| `p2pAdmissionTickets` | `sessionId`+`generation`+`accountId` | `jtiDigest`, `keyId`, `sessionId`, `generation`, `accountId`, `joinerSteamId`, `hostSteamId`, `bridgeProtocol`, `virtualPort`, `issuedAt`, `expiresAt`, `status`, `consumedAt`, `consumerBridgeInstanceId`, `consumerRequestId` | rifts-admission |
| `p2pPeerAdmissions` | `activeAdmissionKey` unique | `admissionId`, `sessionId`, `generation`, `accountId`, `steamId`, `bridgeInstanceId`, `connectionHandle`, `admittedAt`, `disconnectedAt` | rifts-admission |
| `p2pControlSequences` | `_id` account | `nextSequence` | rifts-realms |
| `p2pControlMessages` | `accountId`+`sequence` unique; TTL `ttlAt` | `messageId`, `messageType` (12), `sessionId`, `generation`, `payload`, `createdAt`, `expiresAt`, `acknowledgedAt` | rifts-realms |
| `p2pHostPresences` | `_id` account; `expiresAt` | `revision`, `consentToHost`, `observedAt`, `expiresAt`, memory, CPU and network metrics, `runtimeSupported`, `steamRuntimeReady`, `steamId`, `artifacts` (`gameExe`, `internalServer`, `winmmProxy`, `steamBridge`, `steamRuntime`, `admissionPublicKey`), `artifactsVerified`, `launcherVersion`, `guardSessionId`, `guardManifestSequence`, `guardSequence`, `guardRiskScore`, `guardVerified`, `guardRoles[]`, `realmHostSessions` | rifts-realms |
| `p2pHostOffers` | `sessionId`+`generation`+`rank` | `offerId`, `accountId`, `steamId`, `rank`, `score`, `scoreBreakdown` (7 risks), `status`, `offeredAt`, `expiresAt`, `respondedAt`, `responseReason` | rifts-realms |
| `p2pHostReliability` | `_id` account | offer and start counters, `cleanCompletions`, `unexpectedFailures`, `cooldownUntil`, `updatedAt` | rifts-realms |
| `p2pParticipantObservations` | `sessionId`+`generation`+`observedAt`; `accountId`+`observedAt` | `observationId`, `hostReachable`, `localBridgeAlive`, `metricsAvailable`, `lastPacketAgeMs`, `routeKind`, `pingMs`, `jitterMs`, `packetLossPercent`, `observedAt` | rifts-realms |
| `p2pHuntTransitions` | `sessionId`+`at`+`stateVersion` | `transitionId`, `fromState`, `toState`, `stateVersion`, `reason`, `actorType`, `actorAccountId`, `at` | rifts-realms |
| `realmHubPresence` | `sessionId`/`accountId`/`realmId`+`state`; TTL `purgeAt` | `sessionId`, `generation`, `realmId`, `accountId`, `peerId`, `state` (7), `candidateId`, `entryEpoch`, `localBridgePort`, `localBridgeReadyAt`, `admissionId`, `route`, `requestedAt`, `ticketedAt`, `admittedAt`, `lostAt`, `leftAt`, `leftReason`, `expiresAt` | rifts-realms |
| `realmHubGates` | `_id` `sessionId:generation`; TTL `purgeAt` | the seat counter of one hub generation | rifts-realms |
| `guardTrafficWindows` | `_id` hashed key; TTL `expiresAt` | `scope`, `windowStart`, `requests`, `bytes`, `lastDecisionId`, `lastDecisionAccepted` | rifts-guard |
| `p2pHostWrites` | `sessionId`+`generation`; `targetUserId`, `hostAccountId`, `realmId` by `at` | `kind`, `targetUserId`, `hostAccountId`, `sessionId`, `generation`, `realmId`, `sessionKind`, `hubKind`, `summary`, `at` | rifts-guard |
| `p2pHuntXpSummaries` | `_id` `sessionId:member`; TTL `ttlAt` (about 400 days) | `sessionId`, `member`, `host`, `realmId`, `state`, `reason`, `runningAt`, `endedAt`, `minutes`, `weapon`, `player`, `huntPass`, `prestige`, `other`, `grants`, `perMinute`, `flagged`, `createdAt` | rifts-guard |
| `operationsAlerts` | `status`+`createdAt`; delivery index; never TTL | `alertId`, `source` (`central-guard`), `code`, `severity`, `message`, `details`, `status`, `createdAt`, delivery lease and retry fields, `acknowledgedAt`, `acknowledgedBy` | rifts-guard |
| `realms` | `name` unique, `hostAccountId` unique; `members.accountId`, `playerInvites.accountId`, listed by `onlineUntil`, `hubRoomIds` | `realmId`, `name`, `hostAccountId`, `hostPublicKey`, `joinPolicy` (`INVITE_ONLY`, `PUBLIC`, `SOLO`), `listed`, `members[]` (`accountId`, `role`, `joinedAt`), `bans[]` (`accountId`, `reason`, `at`), `createdAt`, `lastOnlineAt`, `onlineUntil`, `version`, `hosting` (`maxServers`, `maxPlayers`, `afkSeconds`, `hubSessionId`, `hubState`, `trainingState`, `activeServers`, `updatedAt`), `hubRoomIds[]`, `offlineNotifiedFor`, `joinRequests[]`, `playerInvites[]` | rifts-realms |
| `realmInvites` | `_id` code; `realmId`+`createdAt`; TTL `purgeAt` | `realmId`, `createdBy`, `createdAt`, `expiresAt`, `maxUses`, `uses`, `revokedAt` | rifts-realms |
| `realmPresence` | `_id` account; TTL `expiresAt`; `realmId`+`expiresAt` | `realmId`, `boundAt`, `lastSeenAt`, `expiresAt` | rifts-realms |

Core collections that hold realm data (isles owns the table, rifts reads or writes the fields):

| Collection | Realm part | Writer |
| --- | --- | --- |
| `operationsPolicy` (`_id: global`) | `p2pEmergencyStop`, `rewardRollout` (`version`, `tier0ShadowEnabled`, `tier1GrantsEnabled`), `coopTransport` | `p2p/services/p2pOperationsPolicy.ts` |
| `inventoryTransactions` | `p2pMonitor` (`sessionId`, `generation`, `createdAt`) and its index | `p2p/security/p2pInventoryAuthorization.ts`, indexes from `EnsureP2PDatabase` |
| `launcherGuardSessions`, `guardFindings` | Launcher Guard sessions with role `client` or `host`, module findings | `controllers/launcherGuard.ts` (core) |
| `activityHunts` | player-hosted hunt rows | `activity/activityMonitor.ts:RecordP2PHuntActivity` |
| game exchange codes | `realmId` of a realm Play | `controllers/launcherAuth.ts` |

The private anticheat module's collections (evidence batches active and archived, verdicts, reward receipts) are not
in the tree; RF6 defines its own for the evidence chain intake.

## Decisions needed

1. **Launcher Guard and Guard manifests.** RU3a marks `/launcher/v1/guard/*`, `/admin/v1/guard/*` and the two
   `guard-manifests` routes rifts, and this list follows. But they are core files (`controllers/launcherGuard.ts`,
   `routes/launcherUpdates.ts`) that every Play uses, and the manifest covers core roles (`GAME`, `LAUNCHER`,
   `CONTENT`, `INTERNAL_SERVER`, `UDP_PROXY`). Either `rifts-guard` owns them and isles calls it on every Play, or
   isles keeps them and rifts reads `launcherGuardSessions` through the seam. `MYSTICPARADOX_GUARD_MODE`
   (`services/guardMode.ts`) goes with that choice.
2. **The Director's route.** `POST /api/matchmaker/resolve-hunt-launch-manifest` is rifts code inside the Director,
   and its rule lives in `shared/hunt-manifest/p2p/completionMode.js`, which the backend's local manifest also uses.
   The backend's default source is `local`, so the route is used only with
   `MYSTICPARADOX_HUNT_MANIFEST_SOURCE=director`. Port it into `rifts-realms` behind the Director's hook, or drop the `director` source.
3. **Hooks spread through the core.** Host writes touch nine core files (section 2.1). The `HostWriteKind`
   checks stay in isles' handlers either way; only `authorize` and `record` cross. The realm deletion steps for
   `realms`, `realmInvites` and `realmPresence` sit in the core's `accountDeletionPlan.ts`, not behind the hook as the
   p2p ones are; RF2 should move them behind it.
4. **Component splits that could go either way:** `host-ready` and `host-bootstrap` (lifecycle or admission),
   `/launcher/v1/p2p/hunts/:sessionId/peers` (hub admission, carries pair keys), `coop-transport` (relay or realms),
   the census and host writes (guard or realms), `P2P_FORCE_SDR` and `P2P_TRUST_TIER`. The tables pick one.
5. **The runtime hooks** stay in isles' native runtime (`MysticParadox.dll` per version), but their contract is
   `rifts-host`'s. A change to the launch environment or the pipes is a change to both repositories.
6. **`MysticTransport_UpdateRelayCredentialH`** is expected by the launcher and exported by no transport; RF3 or RF7
   either adds it (ABI 3 stays, the export is optional) or the launcher drops the probe.
7. **`ParadoxBackend/.env.p2p.example`** is named by `docs/SELF_HOSTING.md` and missing; section 5.1 is its content.
