<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false, "code_blocks": false } } -->

# Clean-Room Behaviour Specification: `rustic-rifts-realms`, the domain half (RD27)

> **Scope and purpose (RD27):** this is the clean-room behaviour specification for the *domain* modules of
> `modules/rustic-rifts/crates/rustic-rifts-realms` in dainty-restricted: the realm directory and its rules, the realm hub and
> hunt state machines, leases and the lifecycle monitor, host selection, hosting and trust tiers, host policy, host
> capabilities, the launcher's control channel, strict input parsing, id and time spellings, and the session routes in
> `hosting.rs`. The persistence half (`repo/`, `db.rs`, migrations, `routes.rs`, `service.rs`, `ports.rs`, `stub.rs`,
> `lib.rs`, `Cargo.toml`, the repository tests, `guard_adapter`, `stubbed`) is
> [`spec-rd27-rifts-realms-store.md`](spec-rd27-rifts-realms-store.md); this document names the table shapes only
> through the repository methods it calls (§3) and does not repeat them. The umbrella is
> [`spec-rd27-rifts.md`](spec-rd27-rifts.md) (categories, decisions D1-D5, crate placement).
>
> Everything below is stated from observable contracts: JSON bodies and answers (field names and order), error codes
> and texts, status codes, constants, state tables, control-message kinds and payloads, the public Rust API other
> crates call, and the tests. No implementation code is reproduced and no private helper is named.
>
> Written against dainty-restricted `4efc8b5` (worktree `rd27-spec-rifts-c45124`).

---

## 1. Provenance and classification

### 1.1 Evidence

| Source | What it says |
| --- | --- |
| `modules/rustic-rifts/NOTICE.md` §"Mystic Paradox: ParadoxTransport and realms" | The Rust workspace, `rustic-rifts-realms` included, "ports Mystic Paradox's `p2p/`, `realms/`, Guard and launcher realm-hosting modules (`21b9ff5`, `AGPL-3.0-only`)" |
| File headers (`//!`) | Every source file names the Node module it ports (`realms/realmRules.ts`, `realms/realmsService.ts`, `p2p/realm/realmHubRules.ts`, `p2p/controllers/p2pHostLifecycle.ts`, `p2p/config/p2pPolicy.ts`, `p2p/config/p2pHostPolicy.ts`, `p2p/security/hostCapabilities.ts`, `p2p/controllers/p2pControl.ts`, `realms/realmInput.ts`); each test file names the Node test it ports |
| Commits `b3f9cd3` (RF2), `39859ba` (RF5), `97853a5` (RF4/RF5) | The seams, the port onto `store`, and the hub/bootstrap/admission/relay wiring |
| Umbrella §1.2 | No Category A in rifts; files without a Paradox source of their own are rewritten too |

### 1.2 Files of this half

All files are **B** (rewritten clean-room from this document). `ids.rs` and `time.rs` have no direct Paradox
counterpart (they translate Node's spellings onto Dainty Lass's seams) but are rewritten with the rest, per the
umbrella. There is no D (no recorded vectors in `rustic-rifts-realms`) and no G.

| Path | Lines | Cat. | Ports | Notes |
| --- | ---: | --- | --- | --- |
| `src/realm.rs` | 735 | B | `realms/realmTypes.ts`, `realmRules.ts`, `realmsConfig.ts` | §2.4 |
| `src/directory.rs` | 1846 | B | `realms/realmsService.ts`, `realms/entry.ts` | §2.6 |
| `src/hub.rs` | 599 | B | `p2p/realm/realmHubRules.ts`, `hubManifests.ts`, `realmHubPolicy.ts` | §2.8 |
| `src/hunt.rs` | 586 | B | `p2p/persistence/contracts/P2PHuntRepository.ts` (records) | §2.7 |
| `src/lifecycle.rs` | 940 | B | `p2p/controllers/p2pHostLifecycle.ts` (rules, steps, recovery) | §2.11 |
| `src/leases.rs` | 952 | B | `RenewHostLease`, `RecordParticipantObservation`, `P2PHuntLifecycleMonitor`, `EndRealmHub` | §2.12 |
| `src/selection.rs` | 723 | B | `SubmitHostPresence`, `RankHosts`, `RankRealmHost`, `BeginHostSelection`, `RespondToHostOffer` | §2.13 |
| `src/tiers.rs` | 356 | B | `p2p/config/p2pPolicy.ts` | §2.9 |
| `src/host_policy.rs` | 443 | B | `p2p/config/p2pHostPolicy.ts`, `security/launcherVersion.ts` | §2.10 |
| `src/capabilities.rs` | 663 | B | `p2p/security/hostCapabilities.ts`, `hostRuntimeAuth.ts` | §2.14 |
| `src/control.rs` | 169 | B | `p2p/controllers/p2pControl.ts` | §2.15 |
| `src/input.rs` | 489 | B | `realms/realmInput.ts` | §2.5 |
| `src/ids.rs` | 139 | B | (seam spellings) | §2.2 |
| `src/time.rs` | 108 | B | (`toISOString`) | §2.3 |
| `src/hosting.rs` | 2530 | B | `p2pLauncher.ts`, `p2pAdmission.ts`, `p2pHuntStatus.ts`, `realmHubLifecycle.ts`, `realmHostingStatus.ts`, `huntLaunchManifest.ts`, `p2pAdminOperations.ts` | §2.16 |
| `tests/realm_rules.rs` | 1096 | B | `realmRules.test.ts`, `realmInput.test.ts`, `realmHubRules.test.ts` | rewrite (§5) |
| `tests/hub_rules.rs` | 754 | B | `realmHubRules.test.ts` | rewrite |
| `tests/lifecycle_rules.rs` | 414 | B | `p2pHostLifecycle.test.ts` | rewrite |
| `tests/tiers.rs` | 389 | B | `p2pPolicy.test.ts` | rewrite |
| `tests/host_policy.rs` | 334 | B | `p2pHostPolicy.test.ts` | rewrite |
| `tests/leases.rs` | 342 | B | `hostLease.test.ts`, `p2pLifecycleMonitor.test.ts` | rewrite |
| `tests/directory.rs` | 1188 | B | `realmsService.test.ts` | rewrite |
| `tests/directory_presence.rs` | 1724 | B | `realmsPresence/Hosting/Solo/Entry.test.ts` | rewrite |
| `tests/capabilities_control.rs` | 348 | B | `hostCapabilities.test.ts`, `p2pControl.test.ts` | rewrite |
| `tests/hosting_bootstrap.rs` | 569 | B | (end-to-end through admission and relay routes) | rewrite; also kept as an oracle (§5.2) |
| `tests/common/kit.rs` | 255 | B | `realms/realmsTestKit.ts` | shared with the store half's tests; rewrite here (`common/mod.rs` is the store half's) |

Totals: 15 source files, 11,278 lines; 11 test files, 7,413 lines. A 0, B 26, D 0, G 0.

---

## 2. Behaviour

### 2.0 Conventions

1. **Strings are measured as JavaScript measures them.** "Length" is the UTF-16 code-unit count. "Trim" removes, at
   both ends, every character with the Unicode `White_Space` property plus U+FEFF (call this *JS-trim*).
   "Lower-case"/"upper-case" are the full Unicode case mappings.
2. **Times** are Unix milliseconds (`UnixMillis`, i64) inside; every time a client sees is the `toISOString()` text
   of §2.3.
3. **A refusal** is `RealmsError::Refused(Refusal { code, message })`; a store failure is
   `RealmsError::Database(text)` (answered by the service as `INTERNAL` / "The realms request failed.", store half);
   `RealmsError::Unimplemented` is the seam's 501. `RealmsError` implements `Display` and `Error`
   (`thiserror`), `From<Refusal>` and `From<Unimplemented>`.
4. **HTTP status** is not this crate's: isles-core maps a code to a status. The table it holds today, for the codes
   this half emits: `AUTH_RATE_LIMITED` 429, `AUTH_UNAUTHORIZED` 401, `INTERNAL` 500, `REALMS_DISABLED` 501,
   `P2P_DISABLED` 503, `P2P_EMERGENCY_STOP` 503, `P2P_SESSION_NOT_FOUND` 404, `P2P_SESSION_NOT_READY` 409,
   `P2P_NOT_ROSTERED` 403, `P2P_STEAM_LINK_REQUIRED` 409, `P2P_TICKET_INVALID` 400, `P2P_CAPABILITY_INVALID` 401,
   `P2P_GENERATION_FENCED` 409, `P2P_SIGNAL_INVALID` 400, `REALM_HUB_FULL` 409, `GUARD_REQUIRED` 403,
   `REALM_VALIDATION_FAILED` 400, `REALM_NAME_INVALID` 400, `REALM_NAME_RESERVED` 400, `REALM_NAME_TAKEN` 409,
   `REALM_ALREADY_EXISTS` 409, `REALM_USERNAME_REQUIRED` 409, `REALM_NAME_IS_USERNAME` 409, `REALM_NOT_FOUND` 404,
   `REALM_NOT_MEMBER` 404, `REALM_BANNED` 403, `REALM_INVITE_REQUIRED` 403, `REALM_INVITE_INVALID` 404,
   `REALM_INVITE_EXPIRED` 410, `REALM_INVITE_LIMIT` 409, `REALM_FULL` 409, `REALM_OWNER_CHANGED` 409,
   `REALM_INVALID_TARGET` 400, `REALM_HOST_CANNOT_LEAVE` 409, `REALM_JOIN_REQUESTED` 409, `REALM_REQUESTS_FULL` 409,
   `REALM_REQUEST_NOT_FOUND` 404, `REALM_PLAYER_NOT_FOUND` 404, `REALM_PLAYER_INVITES_FULL` 409,
   `REALM_PLAYER_INVITE_NOT_FOUND` 404, `REALM_OFFLINE` 409, `REALM_SOLO` 409. Codes absent from that table
   (`REALM_HOSTING_DISABLED`, `REALM_SERVER_LIMIT`, `REALM_CAPACITY_EXCEEDED`, `P2P_HOST_INELIGIBLE`,
   `ADMIN_VALIDATION`) fall to 500 there (§8, R6). Every successful answer is 200.
5. **JSON objects keep insertion order** (the restricted workspace builds `serde_json` with `preserve_order`). Every
   field list below is in wire order. "Omitted" means the key is absent; "null" means present with `null`.
6. **Actors.** Every repository write names an `Actor` (rifts-seam): `Account(id)` for a player or host, or
   `System(label)`. Labels used by this half: `realms-p2p` (directory calls from the p2p side), `realms-sweeper`,
   `realms-lifecycle` (monitor), `realms-selection`, `realms-recovery`, `realms-directory` (event sink),
   `realms-route` (routed hunt with no members), `realms` (capability revoke). They reach the audit trail only.

### 2.1 Public API

#### 2.1.1 Used outside the crate (must keep these names and signatures)

Callers: `modules/rustic-isles/crates/isles-1-14-7/src/lib.rs` and its tests `module.rs`, `inventory_reforge_host.rs`.
Items the store half owns (`db::RealmsDb`, `ports::*`, `repo::*`, `RealmsRoutes*`, `RealmsService*`,
`realms_enabled_or_default`, `stub_routes`, `Stub`) are listed in its spec.

| Path | Item | Shape |
| --- | --- | --- |
| crate root | `RealmsError` | enum `Unimplemented(Unimplemented)`, `Refused(Refusal)`, `Database(String)` (§2.0.3) |
| crate root | re-exports | `RealmsHuntSessions`, `HostBootstrap`, `HuntLifecycle`, `HuntStatus`, `Observation`, `StartHub`, `resolve_manifest_for_hunt`, `HostingRuntime`, `RealmsRecovery` |
| `capabilities` | `HostCapabilities` | `new(RealmsDb) -> Self`; `async check(&self, header: Option<&str>, session_id: &str, scope: CapabilityScope, now: UnixMillis) -> Result<HostCapabilityRecord, RealmsError>`; `async mint(&self, &HuntSession, bridge_instance_id: &str, Vec<CapabilityScope>, issued_at, expires_at) -> Result<(Secret, HostCapabilityRecord), RealmsError>`; plus §2.14 |
| `capabilities` | `RealmsHuntSessions` | `new(RealmsDb, Arc<dyn Clock>)`; implements `rifts_guard::ports::HuntSessions` (§2.14.6) |
| `control` | `ControlChannel` | `new(RealmsDb, Arc<dyn Clock>)`; §2.15 |
| `directory` | `DirectoryParts`, `RealmDirectory` | pub fields `db, accounts, events: Option<Arc<dyn RealmEventSink>>, clock, limiter, ids, limits: RealmLimits, log`; `RealmDirectory::new(DirectoryParts)` |
| `host_policy` | `ArtifactSources`, `read_artifact_policy`, `ArtifactPolicy`, `EligibilityPolicy` | §2.10; `ArtifactSources: Default` |
| `hunt` | `HuntSession`, `HuntState`, `AdmissionState`, `Participant`, `ArtifactHashes`, `HostCapabilityRecord`, `HostPresenceRecord`, `HostReliabilityRecord` | all fields `pub` (tests build `HuntSession` and `Participant` literals); `Participant::pending(account, steam_id, candidate_id)` |
| `ids` | `seam_session_id`, `seam_realm_id` | §2.2 |
| `leases` | `Hosting`, `HostingParts`, `HostingRuntime`, `HostingSettings`, `SweepStep` | §2.12; `HostingSettings` fields `policy`, `eligibility`, `realm_hub`, `realm_hosting` are `pub` and isles mutates `policy.enabled`, `policy.allow_all_accounts`, `realm_hosting` after `from_settings` |
| `lifecycle` | `RealmsRecovery`, `Recovery` | `RealmsRecovery::new(RealmsDb, Arc<dyn IslesCore>, Arc<dyn Clock>, Arc<dyn RealmsLog>)` |
| `realm` | `RealmLimits`, `DEFAULT_REALM_LIMITS`, … | `RealmLimits::from_settings(&dyn Settings)` |
| `selection` | `derive_ice_peer_id` | `fn(&str) -> String` |
| `tiers` | `HostingPolicy` | `from_settings(&dyn Settings)`; `pub` fields `enabled`, `allow_all_accounts`, `trust_tier` (u8) read and written by isles |
| `time` | `now`, `iso`, `parse_iso` | §2.3 |

`HostingRuntime` has `pub` fields `capabilities: Arc<HostCapabilities>`, `admission_store: Arc<dyn
rifts_admission::AdmissionStore>`, `tickets: Arc<rifts_admission::Es256Tickets>`, `core: Arc<dyn IslesCore>`,
`backend_url: String`, `host_server_udp_port: u16`, `realms_enabled: bool`, `node: module_api::NodeId`, and is
`Clone`. `HostingParts` has `pub` fields `db, settings: HostingSettings, clock, operations: Arc<dyn OperationsView>,
guard: Arc<dyn GuardSessions>, recovery: Arc<dyn Recovery>, expected_artifacts: Arc<dyn Fn() -> Result<ArtifactHashes,
String> + Send + Sync>, log`. `Hosting::new(HostingParts) -> Hosting`, `Hosting::with_runtime(self, HostingRuntime) ->
Self` (`#[must_use]`).

`Hosting` implements `rifts_admission::AdmissionHunts`, `rifts_relay::IceHunts`, `rifts_relay::OperationsPolicy`,
`ports::RealmEventSink` and `HuntLifecycle` (§2.16.10-14); isles hands it out as each of those trait objects.

#### 2.1.2 Used by the store half (the seam between the two halves)

The store half's `routes.rs` and `service.rs` call, by these names:

- `RealmDirectory`: every launcher operation of §2.6.5 by its method name (`create_realm`, `get_mine`,
  `list_listed`, `get_by_name`, `preview_invite`, `join`, `leave`, `update_settings`, `create_invite`,
  `list_invites`, `revoke_invite`, `kick`, `ban`, `unban`, `heartbeat`, `release`, `list_join_requests`,
  `approve_join_request`, `deny_join_request`, `add_member_by_username`, `invite_player_by_username`,
  `cancel_player_invite`, `list_my_player_invites`, `accept_player_invite`, `decline_player_invite`,
  `presence_heartbeat`, `presence_leave`, `presence_status`), and `presence_of`, `touch_presence`,
  `find_by_hub_room_id`, `is_member`, `is_banned`, `authorize_game_session_realm`, `bind_presence_at_login`. Each
  answer type is `Serialize` and becomes the 200 body as is.
- `Hosting`: `mark_host_ready`, `abort_hunt`, `finish_hunt`, `get_hunt_status`, `create_host_bootstrap`,
  `mark_local_route_ready`, `start_realm_hub`, `stop_realm_hub`, `get_realm_hosting_status`, `renew`,
  `submit_presence`, `respond_to_offer`, `observe`, and (crate-visible) `restore_matchmaking_candidate`,
  `poll_candidate_status`, `route_matchmaking_hunt`; `HuntLifecycle::set_emergency_stop` and
  `HuntLifecycle::resolve_launch_manifest` for the admin and manifest routes.
- `HostCapabilities`: `check`, `resolve_runtime`, `runtime_auth`; free functions `runtime_identity`, `SignedUser`.
- `ControlChannel`: `poll(account, after: Option<&str>, wait: Option<&str>)`, `ack(account, message_id)`.
- `input::*` parsers and `ParseFailure { code: &'static str, message: String }`.
- `realm::{hub_room_id, realms_module_enabled}`, `ids::{seam_realm_id, wire_realm_id}`, `tiers::relay_only_warning`.

### 2.2 Session and realm id spellings (`ids`)

A session has three spellings:

1. **Wire form**: lower-case `8-4-4-4-12` hex UUID text, 36 characters. It is the tables' key, the admission ticket's
   16 bytes and what `bridge.conf` holds. Only exactly this form parses: length 36, `-` at indexes 8, 13, 18, 23, and
   `0-9a-f` elsewhere (upper-case is refused).
2. **Its 128 bits are a ULID.** A new session's bits: the first 6 bytes are the creation time's Unix milliseconds
   (big-endian, low 48 bits), the remaining 10 are random, then byte 6 = `(b6 & 0x0f) | 0x40` and byte 8 =
   `(b8 & 0x3f) | 0x80`, so the text is also a valid v4 UUID.
3. **Seam ids**: `<ULID>@<node>` (rifts-seam `SessionId`, `RealmId`) and module-api's `GlobalId`
   `hub_instance:<ULID>@<node>` for a hub generation, `hunt:<ULID>@<node>` otherwise.

Public functions (all pure): `random_uuid() -> String` (v4 from 16 random bytes with the same two masks);
`hex(&[u8]) -> String` (lower-case); `new_session_ulid(UnixMillis) -> u128`; `uuid_text(u128) -> String`;
`session_bits(&str) -> Option<u128>`; `global_session_id(wire, hub: bool, &NodeId) -> Option<GlobalId>`;
`seam_session_id(wire, &NodeId) -> Option<SessionId>`; `wire_session_id(&SessionId) -> String`;
`seam_realm_id(wire, &NodeId) -> Option<RealmId>`; `wire_realm_id(&RealmId) -> String`. A Node-made v4 id maps the
same way (its bits read as a ULID). Round trip: `wire_session_id(seam_session_id(w)) == w`.

### 2.3 Time text (`time`)

- `now() -> UnixMillis`: the system clock; 0 before the epoch, saturating at `i64::MAX`.
- `iso(UnixMillis) -> String`: exactly `new Date(ms).toISOString()` for years 0000-9999:
  `YYYY-MM-DDTHH:MM:SS.mmmZ`, proleptic Gregorian, negative milliseconds floor (`-1` →
  `1969-12-31T23:59:59.999Z`).
- `parse_iso(&str) -> Option<UnixMillis>`: only that 24-character shape: `-` at 4 and 7, `T` at 10, `:` at 13 and 16,
  `.` at 19, `Z` at 23, ASCII digits in every other position; month 1-12, day within the month (leap years by the
  Gregorian rule), hour ≤ 23, minute ≤ 59, second ≤ 59. Anything else (no millis, a space for `T`, `2026-02-30`) is
  `None`.

Vectors: `0` ↔ `1970-01-01T00:00:00.000Z`; `1_792_000_000_123` ↔ `2026-10-14T17:46:40.123Z`; `951_782_400_000` ↔
`2000-02-29T00:00:00.000Z`.

### 2.4 Realm records and pure rules (`realm`)

#### 2.4.1 Enums and their wire text

| Type | Values (wire text) |
| --- | --- |
| `JoinPolicy` | `InviteOnly` `INVITE_ONLY`, `Public` `PUBLIC`, `Solo` `SOLO` |
| `MemberRole` | `Host` `HOST`, `Member` `MEMBER` |
| `HubState` | `None` `NONE`, `Starting` `STARTING`, `Running` `RUNNING`, `Stopping` `STOPPING` |
| `InviteState` | `Usable` `USABLE`, `Revoked` `REVOKED`, `Expired` `EXPIRED`, `Exhausted` `EXHAUSTED` |
| `HubKind` (rifts-seam) | `Ramsgate` `RAMSGATE`, `Training` `TRAINING` (`hub_kind_name`, `parse_hub_kind`) |
| `RemovalReason` | `Kick`, `Ban`, `Leave` |
| `NameRefusal` | `Invalid`, `Reserved` |
| `JoinVia` | `Member`, `Public`, `Invite` |
| `JoinDenial` | `Banned` → `REALM_BANNED`, `Solo` → `REALM_SOLO`, `InviteRequired` → `REALM_INVITE_REQUIRED`, `InviteInvalid` → `REALM_INVITE_INVALID`, `InviteExpired` → `REALM_INVITE_EXPIRED` (`code()`) |

Each text enum has `as_str()` and `parse(&str) -> Option<Self>` (exact, case-sensitive).

#### 2.4.2 Records

- `RealmMember { account_id, role, joined_at }`, `RealmBan { account_id, reason: Option<String>, at }`,
  `AccountAt { account_id, at }` (a join request or a player invite).
- `RealmHosting { max_servers, max_players, afk_seconds: Option, hub_session_id: Option<String>, hub_state,
  training_state, active_servers, updated_at }` (i64 numbers).
- `Patch<T>`: `Keep` (default), `Remove` (Node's `null`), `Set(T)`.
  `HostingPatch { max_servers: Option, max_players: Option, afk_seconds: Patch<i64>, hub_session_id: Patch<String>,
  hub_state: Option<HubState>, training_state: Option<HubState>, active_servers: Option }` (Default = nothing).
- `RealmRecord { realm_id, name (canonical lower case, unique), host_account_id, host_public_key, join_policy, listed,
  members (the HOST first, then in joining order), bans, created_at, last_online_at: Option, online_until: Option,
  version: Option<String>, hosting: Option<RealmHosting>, hub_room_ids: Vec<String>, offline_notified_for: Option,
  join_requests: Vec<AccountAt>, player_invites: Vec<AccountAt> }`. `offline_notified_for` is the `online_until` for
  which HOST_OFFLINE was emitted (one notice per lapse).
- `RealmInviteRecord { code, realm_id, created_by, created_at, expires_at, max_uses, uses, revoked_at: Option }`.
- `RealmPresenceRecord { account_id, realm_id, bound_at, last_seen_at, expires_at }`, one per account.
- `RealmEvent`: `MemberRemoved { realm_id, account_id, reason: RemovalReason }`,
  `RealmReleased { realm_id, host_account_id }`, `HostOffline { realm_id, host_account_id }`.
- `HeartbeatUpdate { last_online_at, online_until, version: Option<String> }`.
- All derive `Debug, Clone, PartialEq, Eq`.

#### 2.4.3 Names

1. `RESERVED_REALM_NAMES` (29, in this order): `official`, `paradox`, `mysticparadox`, `mystic`, `mysticfox`, `admin`,
   `administrator`, `support`, `mod`, `moderator`, `staff`, `system`, `root`, `dauntless`, `phoenixlabs`, `launcher`,
   `backend`, `directory`, `realm`, `realms`, `api`, `www`, `help`, `security`, `owner`, `team`, `local`, `dedicated`,
   `default`. A name is reserved when, with every `-` removed, it equals one of these.
2. `matches_name_pattern`: 3 to 24 bytes, each `a-z`, `0-9` or `-`.
3. `normalize_realm_name(raw)`: `None` when the raw length exceeds 256 UTF-16 units; else JS-trim then lower-case.
4. `validate_realm_name(raw) -> Result<String, NameRefusal>`: normalize (`None` → `Invalid`); pattern fails, or it
   starts or ends with `-` → `Invalid`; reserved → `Reserved`; else the canonical name. Format is checked before the
   reserved list.
5. `realm_name_for_username(username) -> Option<String>`: `None` past 256 UTF-16 units. JS-trim, lower-case; walk the
   characters: `a-z`/`0-9` are kept; any run of other characters becomes one `-`, written only between two kept
   characters (never leading, never trailing). The result must match the pattern (3-24), else `None` (no truncation;
   `00000000-0000-4000-8000-000000000000` gives `None` since it is 36 long). Reserved words are **not** refused here.
6. Release: `NEVER_ONLINE_RELEASE_MS` = 3 days (259,200,000), `LAST_ONLINE_RELEASE_MS` = 30 days (2,592,000,000).
   `realm_name_release_at(created_at, last_online_at)` = `last_online_at + 30 d` if set, else `created_at + 3 d`.
   `is_realm_name_releasable(realm, now)` = `now >= release_at`.
7. `detect_ownership_change(known_name, known_key, realm)`: the lower-cased names are equal and `known_key` differs
   (exact compare) from `realm.host_public_key`.

#### 2.4.4 Invite codes

1. `INVITE_ALPHABET` = `ABCDEFGHJKLMNPQRSTUVWXYZ23456789` (32 symbols: no `I`, `O`, `0`, `1`), `INVITE_CODE_LENGTH` 10.
2. `invite_code_from([u8; 10])`: each byte's low 5 bits index the alphabet. `generate_invite_code()` feeds 10 random
   bytes (unbiased since 32 divides 256).
3. `normalize_invite_code(raw) -> Option<String>`: `None` past 40 UTF-16 units; drop every `-` and every whitespace
   character (§2.0.1), upper-case; must then be exactly 10 bytes, each in the alphabet.
4. Limits: `INVITE_DEFAULT_MAX_USES` 10, `INVITE_MAX_USES_LIMIT` 100, `INVITE_DEFAULT_TTL_HOURS` 168,
   `INVITE_MAX_TTL_HOURS` 720, `INVITE_PURGE_GRACE_MS` 7 days (the store half purges an invite a week past expiry).
5. `invite_state(invite, now)`, first match: revoked → `Revoked`; `now >= expires_at` → `Expired`;
   `uses >= max_uses` → `Exhausted`; else `Usable`.
6. `build_invite(code, realm_id, created_by, now, max_uses, ttl_hours)`: `created_at = now`,
   `expires_at = now + ttl_hours × 3,600,000`, `uses = 0`, not revoked.

#### 2.4.5 The join decision

`decide_join(realm, account, invite: Option<&RealmInviteRecord>, now) -> Result<JoinVia, JoinDenial>`, first match:

1. the account is in `bans` → `Banned` (beats a member, a public realm, a valid invite);
2. policy `SOLO` and the account is not the host → `Solo`;
3. the account is in `members` (the host is) → `Ok(Member)`;
4. policy `PUBLIC` → `Ok(Public)`;
5. no invite → `InviteRequired`;
6. the invite's `realm_id` differs → `InviteInvalid`;
7. the invite is not `Usable` at `now` → `InviteExpired`;
8. → `Ok(Invite)` (the caller consumes a use).

`sort_newest_first(&[AccountAt])`: descending by `at`, stable (equal times keep stored order).

#### 2.4.6 Online state

- `HEARTBEAT_ONLINE_MS` = 90,000. `is_realm_online(realm, now)` = `online_until > now`.
- `is_realm_hub_ready(realm)` = hosting present and `hub_state == RUNNING` (Training's state does not count).
- `is_realm_serving(realm, now)` = online and hub ready.
- `apply_heartbeat(now, online: Option<bool>, version)` → `last_online_at = now`; `online_until = now` when
  `online == Some(false)`, else `now + 90,000`; version passed through.

#### 2.4.7 Hosting record

- Limits: `HOSTING_MAX_SERVERS_LIMIT` 8, `HOSTING_MAX_PLAYERS_LIMIT` 64, `HOSTING_MAX_AFK_SECONDS` 7200,
  `HOSTING_ACTIVE_SERVERS_LIMIT` 64, hub session id at most 128 UTF-16 units.
- `default_hosting(at)`: `max_servers` 1, `max_players` 16, no afk, no hub session, `hub_state` and `training_state`
  `NONE`, `active_servers` 0, `updated_at = at`.
- `apply_hosting_patch(current, patch, at)`: start from `current` or the default; set each `Some`/`Set` field;
  `Remove` clears an optional one; `Keep`/`None` leave it; `updated_at = at` always.
- `validate_hosting_patch(patch) -> Option<&'static str>`, the first bad field by name, in this order: `maxServers`
  (outside 1-8), `maxPlayers` (1-64), `afkSeconds` (a `Set` outside 0-7200), `activeServers` (0-64),
  `hubSessionId` (a `Set` that is empty or over 128 units).

#### 2.4.8 Hub room ids

`hub_room_id(realm_id, kind)`: SHA-256 of the UTF-8 text `paradox-realm-hub:v1:<realm_id>:<KIND>` (`KIND` =
`RAMSGATE`/`TRAINING`); take the first 16 bytes, set byte 6 = `(b6 & 0x0f) | 0x50` and byte 8 = `(b8 & 0x3f) | 0x80`,
write as the lower-case wire UUID (§2.2). Deterministic; it is what the game and chat know the hub by.
`hub_room_ids_of(realm_id)` = `[ramsgate, training]` in that order. `normalize_hub_room_id(raw)`: ASCII lower-case,
then it must be a wire UUID, else `None`.

Vectors: `hub_room_id("realm-1", Ramsgate)` = `d270fd46-96fc-554c-b78c-9f56b771c529`;
`hub_room_id("realm-1", Training)` = `d2ece5f8-cf1e-5515-9dee-19a3fff3c4b1`.

#### 2.4.9 Presence rule constants

`PRESENCE_TTL_MS` 120,000 (each bind and touch), `PRESENCE_CACHE_MS` 2,000 (a process trusts its own `presence_of`
answer this long), `PRESENCE_TOUCH_WRITE_AFTER_MS` 30,000, `PRESENCE_TOUCH_MIN_LEFT_MS` 60,000.
`presence_touch_needs_write(last_seen_at, expires_at, now)` = `now - last_seen_at > 30,000` **or**
`expires_at - now < 60,000`. Consequence the tests check: touches at most 60 s apart never let a row lapse; a touch
every 10 s writes once per 40 s.

#### 2.4.10 Settings

- `realms_module_enabled(settings)`: `MYSTICPARADOX_REALMS_MODULE` trimmed equals `on`, `true` or `1`, any case.
- `RealmLimits { max_members, max_bans, max_active_invites, max_join_requests, max_player_invites }` (u32).
  `DEFAULT_REALM_LIMITS` = 1000, 1000, 25, 50, 50. `from_settings`: `MYSTICPARADOX_REALMS_MAX_MEMBERS` (default 1000,
  allowed 2-5000) and `MYSTICPARADOX_REALMS_MAX_BANS` (default 1000, allowed 1-5000); a value counts only when,
  trimmed, it is 1 to 6 ASCII digits within the range; anything else is the default. The other three are fixed.
- `js_whitespace(c)`, `js_trim(&str) -> &str`, `iso_or_null(Option<UnixMillis>) -> Option<String>` are `pub` helpers.

### 2.5 Strict input parsing (`input`)

Types are checked, never coerced. Bodies arrive already parsed as JSON (the route turns an empty body into "absent"
and an unparsable one into `REALM_VALIDATION_FAILED` / "The request body is not valid JSON.", store half). An absent
or `null` body is an empty object; any non-object body fails "The request body must be a JSON object.". A key that is
present (even `null`) is checked; unknown keys are ignored. Every failure is a `ParseFailure` whose code is
`REALM_VALIDATION_FAILED` unless stated.

`whole_number(&Value) -> Option<i64>` is `Number.isSafeInteger`: a JSON number whose value is an integer within
±9,007,199,254,740,991 (`5.0` passes, `5.5`, strings and booleans do not).

| Parser | Field | Rule | Failure message |
| --- | --- | --- | --- |
| `parse_create_realm_body` → `CreateRealmInput { name: Option, host_public_key, join_policy, listed, version: Option }` | `name` (optional) | string, `validate_realm_name` | reserved: `REALM_NAME_RESERVED` "That realm name is reserved."; else (incl. non-string) `REALM_NAME_INVALID` "A realm name is 3-24 letters, digits or hyphens, and cannot start or end with a hyphen." |
| | `hostPublicKey` (required) | host key (below) | "hostPublicKey must be the host's public key as text." |
| | `joinPolicy` (default `INVITE_ONLY`) | exact `SOLO`/`INVITE_ONLY`/`PUBLIC` | "joinPolicy must be SOLO, INVITE_ONLY or PUBLIC." |
| | `listed` (default false) | boolean | "listed must be true or false." |
| | `version` (optional) | version (below) | "version is malformed." |
| `parse_settings_body` → `SettingsInput { join_policy, listed }` | `joinPolicy`, `listed` | as above | as above; neither present: "Nothing to change: send joinPolicy and/or listed." |
| `parse_invite_body` → `InviteInput { max_uses, ttl_hours }` | `maxUses` (default 10) | whole number 1-100 | "maxUses must be a whole number from 1 to 100." |
| | `expiresInHours` (default 168) | whole number 1-720 | "expiresInHours must be a whole number from 1 to 720." |
| `parse_join_body` → `JoinInput { name, invite_code, known_host_key }` | `name` | string, existing-name rule (below) | `REALM_NAME_INVALID` "That is not a realm name." |
| | `inviteCode` | string, `normalize_invite_code` | "That is not an invite code." |
| | (neither name nor code) | | "Send a realm name and/or an invite code." |
| | `knownHostKey` (checked after the above) | host key | "knownHostKey is malformed." |
| `parse_heartbeat_body` → `HeartbeatInput { online, version, hosting }` | `online` | boolean | "online must be true or false." |
| | `version` | version | "version is malformed." |
| | `hosting` | object | "hosting must be an object." |
| | `hosting.maxServers` (required) | whole 1-8 | "hosting.maxServers must be a whole number from 1 to 8." |
| | `hosting.maxPlayers` (required) | whole 1-64 | "hosting.maxPlayers must be a whole number from 1 to 64." |
| | `hosting.afkSeconds` (optional; present `null` fails) | whole 0-7200 | "hosting.afkSeconds must be a whole number from 0 to 7200." |
| `parse_entry_realm(&Value)` → `EntryRealmInput { name, invite_code, known_host_key }` (the `realm` of `POST /launcher/v1/game-sessions`) | (the value) | object | "realm must be an object with a name." |
| | `name` (required) | key missing: "realm.name is required."; then the path-name rule | `REALM_NAME_INVALID` "That is not a realm name." |
| | `inviteCode`, `knownHostKey` | as join | as join |
| `parse_ban_body` → `Option<String>` | `reason` (optional) | string; JS-trimmed; ≤ 200 UTF-16 units; no char ≤ U+001F and no U+007F | non-string: "reason must be text."; else "reason must be at most 200 characters of plain text."; an empty trimmed reason is `None` |
| `parse_username_body` → `String` | `username` (required) | string; JS-trimmed; 1-64 UTF-16 units; no control chars | missing: "username is required."; non-string: "username must be text."; else "username must be 1 to 64 characters of plain text." |

Other parsers:

- **Host key** (`parse_host_public_key(&Value) -> Option<String>`): a string, JS-trimmed, 16-512 bytes of
  `A-Za-z0-9+/_=-` (base64, base64url or hex by alphabet only). The trimmed text is kept.
- **Version** (`parse_version`): a string, JS-trimmed, 1-64 bytes of `A-Za-z0-9._+-`.
- **Existing name** (join, entry, path): a well-formed reserved name passes (it names an existing realm) as its
  JS-trimmed lower-case text; an invalid one fails.
- `parse_account_id_param(&str) -> Option<String>`: 1-128 bytes of `A-Za-z0-9._:-` (no trim). The route's failure is
  "accountId is malformed." (store half).
- `parse_list_query(limit: Option<&str>, offset: Option<&str>) -> Parsed<(u32, u32)>`: defaults
  (`LIST_DEFAULT_LIMIT` 25, 0). `limit`: 1-3 ASCII digits, value 1-50 (`LIST_MAX_LIMIT`), else "limit must be a whole
  number from 1 to 50."; `offset`: 1-5 ASCII digits, value ≤ 10,000 (`LIST_MAX_OFFSET`), else "offset must be a whole
  number from 0 to 10000.". No sign, no space, no empty value.
- `parse_known_host_key_query(Option<&str>)`: absent → `None`; else the host-key rule or "knownHostKey is malformed.".
- `parse_name_param(Option<&str>)`: the existing-name rule; else `REALM_NAME_INVALID` "That is not a realm name.".
- `parse_invite_code_param(&str)`: `normalize_invite_code` or "That is not an invite code.".
- Constants: `BAN_REASON_MAX_LENGTH` 200, `USERNAME_MAX_LENGTH` 64.

### 2.6 The realm directory (`directory`)

#### 2.6.1 Construction and dependencies

`RealmDirectory::new(DirectoryParts)`. It uses the store half's `RealmsRepository` and `RealmPresenceRepository`
(§3.1-3.2) and the ports `Accounts` (`display_names(Vec<String>) -> BTreeMap<id, name>`,
`resolve_username(String) -> Option<account>`), `RealmEventSink` (`emit(RealmEvent)`), `Clock`, `RateLimiter`
(`is_limited(key, max, window_ms) -> bool`), `IdSource` (`realm_id()`, `invite_code()`), `RealmsLog`
(`info`, `warn`). `presence_io() -> (u64, u64)` returns the counts of presence rows read from the database and of
touches written (the tests measure the cache and the touch rule with it).

#### 2.6.2 Rate budgets

Every launcher operation first asks the limiter with key `realms:<bucket>:<account>`; limited →
`AUTH_RATE_LIMITED` "Too many realm requests. Please wait a moment and try again.". The calls for the p2p side, the
core and the sweeper (§2.6.7-2.6.8) are not throttled.

| Bucket | Max | Window | Operations |
| --- | ---: | --- | --- |
| `read` | 240 | 60 s | get_mine, get_by_name, list_listed, list_invites, list_join_requests, list_my_player_invites, presence_status |
| `write` | 60 | 60 s | update_settings, revoke_invite, leave, kick, ban, unban, approve, deny, add_member_by_username, cancel_player_invite, accept/decline_player_invite, release |
| `heartbeat` | 30 | 60 s | heartbeat, presence_heartbeat, presence_leave |
| `create` | 5 | 1 h | create_realm |
| `invite` | 30 | 1 h | create_invite, invite_player_by_username |
| `join` | 30 | 10 min | preview_invite, join |
| `entry` | 30 | 10 min | authorize_entry (Play) |

#### 2.6.3 Answers (fields in wire order)

- **`RealmPublicInfo`**: `realmId`, `name`, `host` (`{ displayName: string|null }`), `hostPublicKey`, `joinPolicy`,
  `listed`, `online`, `serving`, `hubReady`, `playerCount`, `version` (string|null), `memberCount`, `membership`
  (`"HOST"`/`"MEMBER"`/null: the viewer's role), `requested` (viewer has a join request), `invited` (viewer has a
  player invite), `ownershipChanged` (omitted unless the caller sent a known host key; then
  `detect_ownership_change(realm.name, key, realm)`).
  - `serving` = `is_realm_serving` **and** (policy is not `SOLO` **or** the viewer is the host).
  - `playerCount` = the realm's count of active presence rows (0 when absent).
  - Display names and player counts are best effort: a failed lookup logs `[Realms] display-name lookup failed:
    <error>` or `[Realms] player-count lookup failed: <error>` (warn) and the answer goes on with nulls/zeros.
- **`RealmHostView`**: the `RealmPublicInfo` fields (with `ownershipChanged` omitted), then `createdAt`,
  `lastOnlineAt` (null), `onlineUntil` (null), `nameReleaseAt` (always set, §2.4.3.6), `members` (`[{ accountId,
  displayName, role, joinedAt }]` in stored order), `bans` (`[{ accountId, displayName, reason, at }]`),
  `joinRequestCount`, `playerInvites` (`[{ accountId, displayName, at }]` newest first, stable). Names are looked up
  for the host, members, bans and invitees together.
- **`InviteView`**: `code`, `createdAt`, `expiresAt`, `maxUses`, `uses`, `revokedAt` (null), `state` (at now).
- **`PresenceView`**: `realm` (`{ name, serving, hubReady, hostOnline }` or null), `expiresAt` (null). Here `serving`
  is plain `is_realm_serving` (no SOLO filter). A presence whose realm no longer exists answers both null.
- Envelopes: `HostedReply { realm: RealmHostView }`, `RealmReply { realm }`, `RealmsReply { realms }`,
  `MineReply { hosted: RealmHostView|null, memberOf: [RealmPublicInfo] }`, `InviteReply { invite }`,
  `InvitesReply { invites }`, `JoinReply { realm, joined }`, `LeftReply { left }`, `StatusReply { accountId, status }`,
  `ChangedReply { accountId, status, changed }`, `RequestsReply { requests: [{ accountId, displayName, at }] }`,
  `AddedReply { accountId, displayName, added }`, `InvitedReply { accountId, displayName, status }`,
  `PlayerInvitesReply { invites: [{ realm: RealmPublicInfo, at }] }`, `DeclinedReply { declined }`,
  `ReleasedReply { released }`, `HeartbeatReply { online, lastOnlineAt, onlineUntil, ttlSeconds, version }`.
- `EntryAuthorization`: `Ok { realm_id, is_host }` or `Refused(&'static str code)`.

#### 2.6.4 Entry messages

`entry_message(code) -> &'static str` (the refusals the join route and Play share; empty for any other code):

| Code | Message |
| --- | --- |
| `REALM_NOT_FOUND` | There is no realm with that name. |
| `REALM_OWNER_CHANGED` | This realm now belongs to a different host. |
| `REALM_BANNED` | You cannot join this realm. |
| `REALM_INVITE_REQUIRED` | This realm is invite-only. You need an invite code. |
| `REALM_INVITE_INVALID` | That invite code is not valid for this realm. |
| `REALM_INVITE_EXPIRED` | That invite has expired, was revoked or has been used up. |
| `REALM_FULL` | This realm cannot take more members. |
| `REALM_OFFLINE` | This realm is offline: its host is not running it right now. |
| `REALM_SOLO` | This realm is in solo mode: its host is playing alone right now. |
| `REALM_JOIN_REQUESTED` | Request sent to the host. Wait for their approval, then press Play again. |
| `REALM_REQUESTS_FULL` | This realm has too many join requests waiting. Try again later. |

"Not hosted" below means `REALM_NOT_FOUND` "You do not host a realm." (every host-only operation answers it to an
account that hosts nothing, before anything else).

#### 2.6.5 Launcher operations

All take the caller's account id first. The internal steps shared by several:

- **Admit**: when the decision is `Invite`, consume one use of the invite first (`consume_invite`; nothing consumed →
  `REALM_INVITE_EXPIRED`), then `add_member` (role MEMBER, `joined_at` now, cap `max_members`). If the add did not
  return `Added`, give the use back (`refund_invite`). Map: `Banned` → `REALM_BANNED`, `Full` → `REALM_FULL`,
  `NotFound` → `REALM_NOT_FOUND`, `AlreadyMember` → ok/not added, `Added` → ok/added, logging `[Realms] member joined
  realmId=<id> via=<INVITE|PUBLIC>`.
- **Request**: `add_join_request` with `{account, at: now}`, cap `max_join_requests`. `Added` (log `[Realms] join
  requested realmId=<id>`) or `Already` → refused `REALM_JOIN_REQUESTED`; `AlreadyMember` → treat as a member;
  `Banned` → `REALM_BANNED`; `Full` → `REALM_REQUESTS_FULL`; `NotFound` → `REALM_NOT_FOUND`.
- **Accept a player invite**: `add_member` (MEMBER, now, cap). `Banned`/`Full`/`NotFound` → that code. On
  `AlreadyMember`, also `remove_player_invite` (an `Added` drops it in the same write, §3.1). Then
  `remove_join_request`. Log `[Realms] player invite accepted realmId=<id>` only when `Added`.

Operations:

1. **`create_realm(account, &CreateRealmInput) -> HostedReply`** (bucket create). (a) The account already hosts a
   realm → `REALM_ALREADY_EXISTS` "You already host a realm. Release it before claiming another name." (checked
   before any name release). (b) The name: look up the account's display name (best effort) and derive
   `own = realm_name_for_username(it)`. No name sent: `own`, or `REALM_USERNAME_REQUIRED` "Set a username before you
   host a realm: your realm is named after it.". A name sent that differs from `own`: `resolve_username(name)` (a
   failure is `Database`); held by another account → `REALM_NAME_IS_USERNAME` "That name is another player's
   username. A realm is named after its host.". (c) Build the record: `realm_id` from `IdSource`, `hub_room_ids` of
   it, the host as the only member (role HOST, now), no bans, `created_at` now, not online, `version` from the input,
   no hosting. (d) `insert_realm`. On `NameTaken`: if the holder is releasable at now and
   `delete_realm_if_last_online(holder, holder.last_online_at)` succeeds, log `[Realms] name released realmId=<old>
   name=<name>`, drop every presence row of the old realm (no event) and insert again. Results: `HostHasRealm` →
   `REALM_ALREADY_EXISTS` (same message); `NameTaken` → `REALM_NAME_TAKEN` "That realm name is taken."; `Inserted` →
   log `[Realms] realm created realmId=<id> name=<n> policy=<P> listed=<bool>` and answer the host view.
2. **`get_mine`** (read): `hosted` = host view of the hosted realm or null; `memberOf` = the realms the account is a
   member of (repository, by name, at most 100) except one it hosts, as public infos.
3. **`get_by_name(account, name, known_host_key)`** (read): not found → `REALM_NOT_FOUND` "There is no realm with that
   name."; else `RealmReply` with `ownershipChanged` when a key was sent.
4. **`preview_invite(account, code)`** (join): no such invite → `REALM_INVITE_INVALID` "That invite code is not
   valid."; not usable → `REALM_INVITE_EXPIRED` "That invite has expired, was revoked or has been used up."; its realm
   gone → `REALM_INVITE_INVALID` (same text); caller banned → `REALM_BANNED` "You cannot join this realm."; else the
   public info. Uses nothing.
5. **`list_listed(account, limit, offset)`** (read): the repository's listed page (online first, then by name), minus
   `SOLO` realms the caller does not host.
6. **`update_settings`** (write): require hosted; `set_settings(realm, host, join_policy, listed)` false → not hosted;
   reload (gone → not hosted); log `[Realms] settings changed realmId=<id> policy=<P> listed=<bool>`; host view.
7. **`create_invite(account, &InviteInput)`** (invite): require hosted; `count_active_invites(realm, now) >=
   max_active_invites` → `REALM_INVITE_LIMIT` "You can have at most 25 open invites; revoke one first." (the number is
   the limit). Up to 5 attempts: build with `IdSource::invite_code()` and `insert_invite`; the first `Inserted` logs
   `[Realms] invite created realmId=<id> maxUses=<n> expiresAt=<iso>` and answers the view. Five clashes →
   `Database("Could not generate a unique invite code.")`.
8. **`list_invites`** (read): require hosted; newest first, at most 100.
9. **`revoke_invite(account, code)`** (write): require hosted; `revoke_invite(realm, code, now)` none →
   `REALM_INVITE_INVALID` "That invite code is not valid."; log `[Realms] invite revoked realmId=<id>`; the view
   (revoked once; a second revoke keeps the first time).
10. **`join(account, &JoinInput) -> JoinReply`** (join):
    1. a code was sent: find it, or `REALM_INVITE_INVALID` "That invite code is not valid.";
    2. the realm: by name when a name was sent, else the invite's realm; none → entry `REALM_NOT_FOUND`;
    3. an invite of another realm → entry `REALM_INVITE_INVALID`;
    4. a known key and an ownership change → entry `REALM_OWNER_CHANGED` (before anything else is decided);
    5. `decide_join`. If `InviteRequired` and the caller holds a player invite: accept it (refusal → entry error of
       its code) and answer the reloaded realm with `joined: true`;
    6. if still `InviteRequired`: request (a refusal → entry error); otherwise reload and treat as `Member`;
    7. any other denial → entry error of its code;
    8. `Member` → `joined: false`, no write;
    9. `Public`/`Invite` → admit; refusal → entry error; answer the reloaded realm with `joined` = added.
11. **`authorize_entry(account, &EntryRealmInput) -> EntryAuthorization`** (entry). Refusals are returned, not
    thrown. Steps: code sent but unknown → `REALM_INVITE_INVALID`; no realm by name → `REALM_NOT_FOUND`; foreign invite
    → `REALM_INVITE_INVALID`; ownership change → `REALM_OWNER_CHANGED`; `decide_join`; `InviteRequired` with a player
    invite → accept it (refusal code returned), then `Member`; still `InviteRequired` → request (refusal code
    returned; `AlreadyMember` → `Member`); other denial → its code. Then `is_host`; a non-host and the realm not
    serving → `REALM_OFFLINE` (so an offline refusal uses no invite and adds no member; a recorded join request or an
    accepted player invite does happen before it). Finally a `Public`/`Invite` decision admits (refusal code
    returned). `Ok { realm_id, is_host }`. The host enters its own realm even when nothing serves.
12. **`leave(account, name)`** (write): unknown → `REALM_NOT_FOUND` "There is no realm with that name."; the host →
    `REALM_HOST_CANNOT_LEAVE` "You host this realm. Release it instead of leaving it."; `remove_member` false →
    `REALM_NOT_MEMBER` "You are not a member of this realm."; log `[Realms] member left realmId=<id>`; drop the
    account's presence in that realm; emit `MemberRemoved{Leave}`; `{ left: true }`.
13. **`kick(account, target)`** (write): require hosted; target is the caller → `REALM_INVALID_TARGET` "You cannot
    remove yourself from your own realm."; `remove_member` false → `REALM_NOT_MEMBER` "That account is not a member of
    your realm."; log `[Realms] member kicked realmId=<id>`; drop presence; emit `MemberRemoved{Kick}`;
    `{ accountId: target, status: "KICKED" }`. A kicked member may come back.
14. **`ban(account, target, reason)`** (write): require hosted; self → `REALM_INVALID_TARGET` "You cannot ban yourself
    from your own realm."; `add_ban({target, reason, now}, max_bans)`: `NotFound` → not hosted; `Full` → `REALM_FULL`
    "The ban list is full; lift a ban first."; `Banned` → log `[Realms] account banned realmId=<id>`, drop presence,
    emit `MemberRemoved{Ban}` (member or stranger alike); `AlreadyBanned` → nothing. Answer
    `{ accountId, status: "BANNED", changed: result == Banned }`.
15. **`unban`** (write): require hosted; `remove_ban` → `changed`; log `[Realms] ban lifted realmId=<id>` when changed;
    `{ accountId, status: "UNBANNED", changed }` (never an error).
16. **`list_join_requests`** (read): require hosted; newest first (stable); `{ accountId, displayName, at }`.
17. **`approve_join_request(account, target)`** (write): require hosted; target has no request →
    `REALM_REQUEST_NOT_FOUND` "That player has not asked to join your realm."; `add_member`: `NotFound` → not hosted;
    `Banned` → remove the request, then `REALM_BANNED` "That player is banned from your realm."; `Full` → `REALM_FULL`
    "This realm cannot take more members." (the request stays); otherwise remove the request, log `[Realms] join
    request approved realmId=<id>`, `{ accountId, status: "APPROVED" }`.
18. **`deny_join_request`** (write): require hosted; `remove_join_request` false → `REALM_REQUEST_NOT_FOUND` (same
    text); log `[Realms] join request denied realmId=<id>`; `status: "DENIED"`. The player may ask again.
19. **`add_member_by_username(account, username)`** (write; launchers to 0.1.68): require hosted; JS-trim;
    `resolve_username` none → `REALM_PLAYER_NOT_FOUND` "No player has that username."; `add_member`: `NotFound` → not
    hosted; `Banned` → `REALM_BANNED` "That player is banned from your realm. Lift the ban first."; `Full` →
    `REALM_FULL` "This realm cannot take more members."; then `remove_join_request`; log `[Realms] player added by the
    host realmId=<id>` when added; `displayName` = looked-up name or the trimmed username;
    `{ accountId, displayName, added }`.
20. **`invite_player_by_username(account, username)`** (invite): require hosted; resolve (none →
    `REALM_PLAYER_NOT_FOUND`); target is the caller → `REALM_INVALID_TARGET` "You are the host of this realm.";
    display name as above. If the target has a join request: accept-a-player-invite for them; ok → log `[Realms]
    invited player had asked to join; let in realmId=<id>` and `status: "MEMBER"`; `REALM_BANNED` → the banned
    refusal below; `REALM_FULL` → `REALM_FULL` "This realm cannot take more members."; other → not hosted. Otherwise
    `add_player_invite({target, now}, max_player_invites)`: `NotFound` → not hosted; `Banned` → `REALM_BANNED` "That
    player is banned from your realm. Lift the ban first."; `Full` → `REALM_PLAYER_INVITES_FULL` "Too many invites are
    waiting for an answer. Cancel one first."; `AlreadyMember` → `MEMBER`; `Added` (log `[Realms] player invited
    realmId=<id>`) / `Already` → `INVITED`. Answer `{ accountId, displayName, status }`.
21. **`cancel_player_invite(account, target)`** (write): require hosted; remove false →
    `REALM_PLAYER_INVITE_NOT_FOUND` "That player has no invite waiting."; log `[Realms] player invite cancelled
    realmId=<id>`; `status: "CANCELLED"`.
22. **`list_my_player_invites`** (read): realms holding an invite for the caller (repository, by name, at most 100),
    each with its invite's `at` (the realm's `created_at` if missing), sorted by `at` descending (stable);
    `{ invites: [{ realm, at }] }`.
23. **`accept_player_invite(account, name)`** (write): unknown → `REALM_NOT_FOUND` "There is no realm with that
    name."; no invite → `REALM_PLAYER_INVITE_NOT_FOUND` "You have no invite to this realm."; accept; on refusal, a
    `REALM_BANNED` also removes the invite; the refusal is the entry error of its code; success →
    `{ realm: reloaded, joined: true }`.
24. **`decline_player_invite(account, name)`** (write): unknown → `REALM_NOT_FOUND`; remove false →
    `REALM_PLAYER_INVITE_NOT_FOUND` "You have no invite to this realm."; log `[Realms] player invite declined
    realmId=<id>`; `{ declined: true }`.
25. **`heartbeat(account, &HeartbeatInput)`** (heartbeat): require hosted; `apply_heartbeat`; `record_heartbeat`
    false → not hosted (a heartbeat without a version keeps the stored one, §3.1). With `hosting`: patch
    `max_servers`, `max_players` and `afk_seconds` (`Set` when sent, `Remove` when omitted) at now; the hub fields are
    never self-reported. Answer `online` = `online != Some(false)`, the update's times, `ttlSeconds` 90, `version` =
    the sent version or the realm's stored one.
26. **`release`** (write): require hosted; `delete_realm_of_host` false → not hosted; log `[Realms] realm released
    realmId=<id> name=<n>`; drop every presence row of the realm; emit `RealmReleased`; `{ released: true }`. The
    name is free at once; members and invites go with the realm.
27. **`presence_heartbeat`** (heartbeat): touch (§2.6.6), then the presence view.
28. **`presence_leave`** (heartbeat): remove the account's presence; idempotent; `{ left: true }`.
29. **`presence_status`** (read): `presence_of`, then the view.

Events go to the sink when one is configured; a sink failure logs `[Realms] event sink failed: <error>` (warn) and
never fails the request. "Drop presence in realm" is `remove(account, Some(realm))`; a failure logs `[Realms] could
not remove presence realmId=<id>: <error>` (warn); the account's cache entry is invalidated either way. "Drop every
presence row of the realm" is `remove_for_realm`; a failure logs `[Realms] could not remove presence of realmId=<id>:
<error>`; the whole cache is cleared either way.

#### 2.6.6 Presence and its cache

`presence_of` is on the matchmaking hot path, so each process keeps its answers (including "none") for 2 s.

- **State**: per account an entry `(read_at, Option<row>)`; per account an epoch counter; one global clear-epoch. An
  account's "epoch" is the pair (clear-epoch, its counter).
- **`presence_of(account)`**: if an entry exists with `0 <= now - read_at < 2000`, answer it, filtered to `None` when
  the cached row's `expires_at <= now` (a lapsed row is never active). Otherwise note the epoch, count one database
  read, `find_active(account, now)`, and store the answer only if the epoch has not moved meanwhile (a write that
  raced the read cannot be overwritten by the stale answer).
- **Store**: with 10,000 entries (`PRESENCE_CACHE_LIMIT`) or more, first drop the entries 2 s old or older; if still
  10,000, drop all; then insert.
- **Invalidate(account)**: bump its counter; when more than 50,000 counters exist, clear them all and bump the
  clear-epoch; remove its entry.
- **Clear**: bump the clear-epoch, drop all counters and entries.
- **`bind_presence(account, realm: Option)`**: `None` deletes the row; `Some` writes `{ bound_at = last_seen_at =
  now, expires_at = now + 120 s }` (replacing any). Invalidate.
- **`touch_presence(account)`**: `presence_of` (cache allowed); none → `None` (an absent or lapsed row is never
  written back). If the touch rule (§2.4.9) says no write, answer the current row. Else note the epoch, count a
  touch, `touch(account, now, now + 120 s)` (repository extends only a row still active), invalidate, and cache the
  result if the epoch had not moved.
- **`leave_presence(account)`**: delete; invalidate.

#### 2.6.7 Calls for the core

- **`authorize_game_session_realm(account, realm: Option<&Value>, module_enabled) -> Result<Option<(realm_id,
  is_host)>>`**: absent or `null` → `None` (Play unchanged). The module off → `REALMS_DISABLED` "Realms are not
  available on this server." (never played as official). Parse with `parse_entry_realm` (its failure code and
  message). `authorize_entry`: refused → the entry error of the code; ok → log `[Realms] game session authorized
  realmId=<id> host=<bool>`, `Some`.
- **`bind_presence_at_login(account, realm_id: Option<&str>, module_enabled)`**: module off → nothing. `None` → unbind;
  a failure logs `[Realms] could not reset realm presence at login: <error>` and is swallowed. `Some` → bind; a
  failure is the error.

#### 2.6.8 Calls for the p2p side and the sweeper (no throttle)

- **`set_hosting(realm_id, &HostingPatch) -> bool`**: `validate_hosting_patch` fails → `Database("setHosting: invalid
  <field>")`; else `patch_hosting` at now (actor `realms-p2p`), true when the realm exists.
- **`mark_offline(realm_id) -> bool`**: ends the online state now; emits nothing.
- `find_by_hub_room_id(room)`: normalize (§2.4.8) or `None`; then the repository. `get_realm(id)`,
  `get_realm_by_host(host)`, `is_member(realm, account)` (in `members`; unknown realm false), `is_banned` (in `bans`).
- **`sweep_lapsed_hosts() -> usize`**: `claim_lapsed_hosts(now, since = now - 10 min (SWEEP_LOOKBACK_MS = 600,000),
  limit 100)` (actor `realms-sweeper`); for each, log `[Realms] host offline realmId=<id>` and emit `HostOffline`.
  Each lapse is reported once; a realm closed on purpose (`online: false`) is reported by the next sweep; one closed by
  `mark_offline` is not. A failing sink does not fail the sweep.

### 2.7 Hunt and hub records (`hunt`)

1. **`HuntState`** (wire text in SCREAMING_SNAKE): `REQUESTED`, `HOST_SELECTING`, `HOST_OFFERED`, `STARTING`,
   `HOST_READY`, `ADMITTING`, `RUNNING`, `DRAINING` (a superseded Tier 0 hunt whose server still serves its players),
   `SUSPECT`, `FENCED`, `ABORTING`, `RECOVERING`, `COMPLETED`, `ABORTED`. `ALL` in that order; `as_str`, `parse`;
   serde uses the same text.
2. **`AdmissionState`**: `PENDING`, `ISSUED`, `ADMITTED`, `LOST`.
3. **`Participant`**: `account_id`, `steam_id` (the transport peer id), `candidate_id`, `reconnect_epoch` (i64),
   `admission_state`, `local_bridge_instance_id`, `local_bridge_port`, `local_bridge_ready_at`, `admitted_at`,
   `disconnected_at`, `rejoin_deadline`, `returned_to_hub_at` (all `Option` after `admission_state`).
   `pending(account, steam, candidate)` = epoch 0, `PENDING`, every option `None`.
4. **`HuntSession`** (Serialize/Deserialize, camelCase; this is the row's JSON `detail` shape, store half): fields in
   order `session_id`, `generation`, `state`, `state_version`, `party_id`, `active_party_key?`, `party_revision`,
   `hunt_definition_id`, `game_mode?`, `build`, `bridge_protocol`, `virtual_port`, `host_account_id`,
   `host_steam_id`, `bridge_instance_id`, `roster` (not serialized: rows of its own), `lobby_id?`, `coop_transport?`,
   `lease_expires_at?`, `created_at`, `expires_at`, `metadata_nonce?`, `hub_kind?` (`RAMSGATE`/`TRAINING` text),
   `realm_id?`, `host_participates?`, `host_local_bridge_port?`, `server_udp_port?`, `host_local_bridge_ready_at?`,
   `hub_room_id?`, `max_peers?`, `guard_completion_mode?`, `host_offer_id?`, `offer_expires_at?`,
   `startup_deadline?`, `last_heartbeat_at?`, `last_lease_sequence?`, `suspect_since?`, `suspect_deadline?`,
   `pre_suspect_state?`, `failure_reason?`, `tried_host_account_ids` (omitted when empty), `terminal_at?`,
   `recovery_deadline?`, `recovery_retry_at?`, `completion_reason?`, `drain_deadline?`, `cancelled_by_account_id?`,
   `trust_tier?`, `guard_encounter_state?`, then `extra` (flattened map: keys this port does not name, kept so a
   write never drops one). `?` = `Option`, omitted when `None`, defaulted when missing. Times serialize as JSON
   integers of milliseconds. An unknown `hubKind` text fails deserialization. `is_realm_hub()` = `hub_kind` is set;
   `participant(account)` finds a roster entry.
5. **`TransitionActor`**: `SYSTEM`, `HOST`, `PARTICIPANT`, `ADMIN`. **`HuntTransition`** { `transition_id` (empty →
   the repository assigns one), `session_id`, `generation`, `from_state`, `to_state`, `state_version`, `reason`,
   `actor_type` (text), `actor_account_id`, `at` }.
6. **Scopes** (`scope_name`/`parse_scope`): `HOST_READY`, `LEASE_HEARTBEAT`, `PEER_ADMITTED`, `GAME_SERVER_RUNTIME`,
   `EVENTS_WRITE`, `COMPLETE_REQUEST`, `ABORT`.
7. **`HostCapabilityRecord`** { `capability_id`, `secret_digest` (SHA-256 hex), `session_id`, `generation`,
   `host_account_id`, `host_steam_id`, `bridge_instance_id`, `scopes`, `issued_at`, `expires_at`, `revoked_at`,
   `last_sequence` }.
8. **`ArtifactHashes`** (camelCase serde; Default): `game_exe`, `internal_server`, `winmm_proxy`, `steam_bridge`,
   `steam_runtime`, `admission_public_key`.
9. **`GuardRole`** (stored form; camelCase): `role` (`client`/`host`), `guard_session_id`, `manifest_sequence`,
   `sequence`, `risk_score` (f64).
10. **`HostPresenceRecord`** (camelCase serde): `account_id`, `revision`, `consent_to_host`, `observed_at`,
    `expires_at` (ms), `freePhysicalMemoryMiB`, `commitHeadroomMiB` (these two keys spelled so), `logical_cpu_count`,
    `cpu_load_percent`, `power_saver`, `runtime_supported`, `steam_runtime_ready`, `steam_id?`, `backend_rtt_ms`,
    `worst_peer_rtt_ms`, `jitter_ms`, `packet_loss_percent`, `artifacts`, `artifacts_verified`,
    `launcher_version`, `guard_session_id?`, `guard_risk_score?`, `guard_verified?`, `guard_roles` (omitted when
    empty), `realm_host_sessions?`. Numbers are f64 except `revision` and `realm_host_sessions`.
11. **`ScoreBreakdown`** (camelCase): `latency_risk`, `network_risk`, `memory_risk`, `cpu_risk`, `stability_risk`,
    `host_load_risk`, `integrity_risk` (each 0-100).
12. **`OfferStatus`**: `OFFERED`, `ACCEPTED`, `DECLINED`, `EXPIRED`, `CANCELLED`. **`HostOfferRecord`** { `offer_id`,
    `session_id`, `generation`, `account_id`, `steam_id`, `rank`, `score`, `score_breakdown`, `status`, `offered_at`,
    `expires_at`, `responded_at`, `response_reason` }.
13. **`HostReliabilityRecord`** { `account_id`, `offers_accepted`, `offers_declined`, `offers_expired`,
    `starts_succeeded`, `starts_failed`, `clean_completions`, `unexpected_failures`, `cooldown_until`, `updated_at` };
    `new(account, at)` all zero. **`ReliabilityIncrements`** (Default zero) has the seven counters.
14. **`ObservationRecord`** { `observation_id`, `session_id`, `generation`, `account_id`, `steam_id`,
    `host_reachable`, `local_bridge_alive`, `metrics_available`, `last_packet_age_ms?`, `route_kind`
    (`DIRECT`/`RELAY`/`CONNECTING`/`UNKNOWN`), `ping_ms?`, `jitter_ms?`, `packet_loss_percent?` (f64), `observed_at` }.
15. **`ControlMessageRecord`** { `message_id`, `account_id`, `sequence`, `message_type`, `session_id?`, `generation?`,
    `payload` (JSON), `created_at`, `expires_at`, `acknowledged_at?` }.

### 2.8 Realm hub rules (`hub`)

#### 2.8.1 Identity, classification and manifests

- `hub_party_key(realm_id, kind)` = `realmhub:<realm_id>:<kind lower-case>` (`ramsgate`/`training`). It is both the
  hub session's `party_id` and its `active_party_key`, so a realm has one live hub per kind.
- `RealmRequest`: `Hub(HubKind)` or `Hunt`. `classify_realm_request(game_mode, hunt_id)`: game mode exactly `CITY` →
  Ramsgate; else a hunt id containing `ReturnToRamsgate` → Ramsgate; containing `TrainingDojo` → Training; else
  `Hunt`.
- Constants: `DEFAULT_HUB_MAX_PLAYERS` 16 (host included), `MAXIMUM_HUB_MAX_PLAYERS` 64,
  `HUB_SESSION_LIFETIME_MS` 7 days (604,800,000), `HUB_OFFER_LIFETIME_MS` 120,000.
- `HubManifest { hub_kind, hunt_definition_id, game_mode, map_path, matchmaker_hunt_id, behemoth_path,
  expected_player_string, guard_completion_mode }` (all `&'static str` but the kind), `hub_manifest(kind)` (const):

| Field | Ramsgate | Training |
| --- | --- | --- |
| `hunt_definition_id` | `ShatteredIsles_ReturnToRamsgate` | `ShatteredIsles_TrainingDojo` |
| `game_mode` | `CITY` | `SHARED` |
| `map_path` | `/Game/Maps/ramsgate/ramsgate_01_persistent` | `/Game/Maps/islands/dojo/training_dojo_persistent` |
| `matchmaker_hunt_id` | `NO_MM_HUNTID` | `NO_MM_HUNTID` |
| `behemoth_path` | `NO_BEHEMOTH` | `NO_BEHEMOTH` |
| `expected_player_string` | `NO_EXPECTED_PLAYERS` | `NO_EXPECTED_PLAYERS` |
| `guard_completion_mode` | `NO_COMPLETION` | `NO_COMPLETION` |

- `HUB_CAPABILITY_SCOPES` = `HOST_READY`, `LEASE_HEARTBEAT`, `PEER_ADMITTED`, `GAME_SERVER_RUNTIME`, `ABORT` (no
  `EVENTS_WRITE`, no `COMPLETE_REQUEST`: a hub has neither evidence nor completion).
- `parse_hub_start_body(&Value) -> Result<(HubKind, i64), &'static str>`: not an object → "The request body must be an
  object."; `hubKind` not the string `RAMSGATE`/`TRAINING` → "hubKind must be RAMSGATE or TRAINING."; `maxPlayers`
  absent → 16, else `whole_number` 1-64 or "maxPlayers must be a whole number from 1 to 64.".
  `parse_hub_stop_body` → the kind, with the same first two messages.

#### 2.8.2 Hub state machine

State sets: `HUB_LIVE_STATES` = HOST_OFFERED, STARTING, HOST_READY, ADMITTING, RUNNING, SUSPECT;
`HUB_TERMINAL_STATES` = COMPLETED, ABORTED; `HUB_RUNTIME_STATES` (the host's runtime holds a live credential) =
STARTING, HOST_READY, ADMITTING, RUNNING, SUSPECT; `HUB_LEASED_STATES` = HOST_READY, ADMITTING, RUNNING.

| `HubEvent` | Applies in | Goes to |
| --- | --- | --- |
| `HostBootstrap` | HOST_OFFERED | STARTING |
| `HostReady` | STARTING, HOST_READY | HOST_READY |
| `HostRouteReady` | HOST_READY | ADMITTING |
| `HostAdmitted` | ADMITTING | RUNNING |
| `LeaseLapsed` | HOST_READY, ADMITTING, RUNNING | SUSPECT |
| `LeaseBack` | SUSPECT | the pre-suspect state, only when it is a leased state |
| `SuspectTimeout` | SUSPECT | ABORTED |
| `Stop` | any live state | COMPLETED |
| `Abort` | any live state | ABORTED |

- `next_hub_state(state, event, pre_suspect) -> Option<HuntState>`: `None` when the event does not apply.
- `is_hub_transition_allowed(from, to)`: some event takes `from` to `to` (for `LeaseBack`: SUSPECT to any leased
  state). The tests assert the table has exactly these transitions.
- `is_hub_live(state)`; `hub_ran(state, pre_suspect)` = RUNNING, or SUSPECT with pre-suspect RUNNING.
- The routes of §2.16 do not walk this table step by step: a hub is created STARTING, host-ready moves it to
  HOST_READY, and the host's route-ready moves it straight to RUNNING (§8, R8).

#### 2.8.3 The hub tick

`decide_hub_tick(hub, now, emergency_stop, hosting_enabled) -> HubTick` (`None`, `Suspect{reason}`, `Recover`,
`End{reason}`), first match:

1. not live → `None`;
2. emergency stop → `End OPERATOR_P2P_STOP`;
3. hosting disabled → `End REALM_HOSTING_DISABLED`;
4. `expires_at <= now` → `End HUB_SESSION_EXPIRED`;
5. HOST_OFFERED with `offer_expires_at <= now` → `End HUB_OFFER_EXPIRED`;
6. STARTING with `startup_deadline <= now` → `End HUB_STARTUP_TIMEOUT`;
7. HOST_READY or ADMITTING with `startup_deadline <= now` → `End HUB_STARTUP_TIMEOUT` (a host that reported ready but
   never published its route is still on the startup clock);
8. HOST_READY, ADMITTING or RUNNING with `lease_expires_at <= now` → `Suspect HOST_LEASE_MISSED`;
9. SUSPECT: a lease that is set and not lapsed, and `LeaseBack` applies → `Recover`; else `suspect_deadline <= now` →
   `End HOST_LEASE_LOST`; else `None`;
10. otherwise `None`.

An unset deadline never lapses. Every hub end from the tick is ABORTED (no clean-end rule, no observation quorum, no
recovery for hubs). `hub_end_counts_as_host_failure(reason)` is true only for `HOST_LEASE_LOST`,
`HOST_PROCESS_UNHEALTHY` and `HUB_STARTUP_TIMEOUT`.

#### 2.8.4 Hub presence (members in a hub)

`PresenceState`: `PENDING` (routed, no ticket yet; expires), `TICKETED` (ticket issued; expires with it), `ADMITTED`
(connected; the host's own row starts so), `LOST`, `LEFT` (travelled to a hunt or left), `REMOVED` (kick, ban,
release), `EXPIRED`. Sets: present = ADMITTED; live = PENDING, TICKETED, ADMITTED; expiring = PENDING, TICKETED; all =
the seven in that order. `parse` matches the text.

`next_presence_state(from, event)`:

| `PresenceEvent` | From | To |
| --- | --- | --- |
| `EntryStarted` | any | PENDING |
| `TicketIssued` | PENDING, TICKETED | TICKETED |
| `Admitted` | PENDING, TICKETED, ADMITTED | ADMITTED |
| `Disconnected` | ADMITTED | LOST |
| `Travelled` | ADMITTED | LEFT |
| `Expired` | PENDING, TICKETED | EXPIRED |
| `Removed` | live | REMOVED |
| `SessionEnded` | live | LOST |

- `ENTRY_LIFETIME_MS` 90,000 (`entry_expires_at(now) = now + 90 s`); `PRESENCE_PURGE_AFTER_MS` 7 days
  (`presence_purge_at(at) = at + 7 d`: when a row that stopped being live may be deleted).
- `is_presence_expired(state, expires_at, now)` = an expiring state with `expires_at <= now`; `is_presence_live` =
  a live state and not expired.
- `decide_entry_request(existing: Option<(state, expires_at)>, now) -> EntryDecision`: none → `Create`; an expiring
  row not expired → `Reuse` (neither the candidate churns nor a ticket in flight goes stale); an expiring row
  expired → `Reenter`; any other state → `Reenter`.

#### 2.8.5 Ticket eligibility

`HubTicketCheck { hub: &HuntSession, generation: Option<i64>, account_id, presence: Option<(PresenceState,
expires_at, peer_id)>, is_member, is_banned, link: Option<(status, peer_id)>, now }`.
`hub_ticket_denial(check) -> Option<HubTicketDenial>`, first match: hub state not RUNNING → `HubNotRunning`;
`hub.expires_at <= now` → `HubExpired`; a generation given that differs → `GenerationMismatch`; the account is the
hub's host → `IsHost`; no presence in an expiring state → `NotExpected`; it is expired → `EntryExpired`; banned →
`Banned`; not a member → `NotMember`; link `("VERIFIED", peer)` with a peer other than the entry's →
`PeerIdMismatch`; any other link, or none → `PeerLinkMissing`; else allowed.

`hub_ticket_denial_error(denial) -> (code, message)`:

| Denials | Code | Message |
| --- | --- | --- |
| HubNotRunning, HubExpired | `P2P_SESSION_NOT_READY` | The realm's hub is not accepting players. |
| GenerationMismatch | `P2P_GENERATION_FENCED` | This request belongs to a stale hub generation. |
| IsHost, NotExpected, EntryExpired | `P2P_NOT_ROSTERED` | This account has no open entry into the hub. |
| Banned | `REALM_BANNED` | You cannot join this realm. |
| NotMember | `REALM_NOT_MEMBER` | You are not a member of this realm. |
| PeerLinkMissing, PeerIdMismatch | `P2P_STEAM_LINK_REQUIRED` | The current co-op transport no longer matches this hub entry. |

#### 2.8.6 Capacity and realm hosting settings

- `RealmCapacityInput { active_servers: i64, max_servers: i64, free_physical_memory_mib, commit_headroom_mib,
  minimum_free_memory_mib, minimum_commit_headroom_mib }` (f64 memory). `decide_realm_capacity`, first match:
  `active >= max` → `ServerLimit`; commit < its minimum → `CommitLow`; free < its minimum → `OkLowFreeMemory` (goes
  ahead; the caller logs it: running out of commit is what ends a server); else `Ok`.
- `hub_is_full(present, max_peers, already_present)` = `!already_present && present >= max_peers`.
- `realm_hosting_enabled(settings)`: `MYSTICPARADOX_REALMS_HOSTING` trimmed is `on`/`true`/`1`, any case.
- `RealmHubPolicy { capability_ttl_ms: i64, minimum_free_memory_mib: f64, minimum_commit_headroom_mib: f64,
  default_max_servers: i64 }`, `from_settings`: `P2P_REALM_HUB_CAPABILITY_TTL_HOURS` (default 24, allowed 1-168,
  × 3,600,000); `P2P_REALM_MIN_FREE_MEMORY_MIB_PER_SERVER` (default 3072, allowed 512-262,144). A value counts only
  when, trimmed, it is 1-10 ASCII digits in range. `minimum_commit_headroom_mib` is fixed at 4096 and
  `default_max_servers` at 6.
- `realm_host_ineligibility_reason(presence, policy, min_free, min_commit, guard_required, now)`: the host check of
  §2.10.4 with the realm's relaxations: worst-peer RTT and jitter unbounded, packet loss up to 100 %, the memory floors
  replaced by the given ones, no other hosted sessions counted (0) and no reliability record (so no cooldown).

### 2.9 Hosting and trust tiers (`tiers`)

#### 2.9.1 Reading numbers as JavaScript does

`js_number(text) -> Option<f64>` (crate-visible; also used by host policy and the control channel): trimmed; empty →
`Some(0)`; `0x`/`0X` followed by hex digits → that integer (bad hex → `None`); `Infinity`, `+Infinity`, `-Infinity`;
otherwise every byte must be a digit, `.`, `e`, `E`, `+` or `-` and the text must parse as an f64, else `None` (NaN).

#### 2.9.2 `HostingPolicy` (`ReadP2PPolicy`)

"Strict true" = the value trimmed equals `true` ignoring case (`1`, `on`, `yes` are false). "CSV set" = split on `,`,
trim each, drop empties, as a `BTreeSet<String>`.

| Field | Setting | Rule |
| --- | --- | --- |
| `enabled` | `P2P_HUNTS_ENABLED` | strict true |
| `allowed_accounts` | `P2P_ALLOWED_ACCOUNTS` | CSV set |
| `allow_all_accounts` | `P2P_ALLOW_ALL_ACCOUNTS` | strict true |
| `allowed_hunts` | `P2P_ALLOWED_HUNTS` | CSV set |
| `allow_all_resolvable_hunts` | `P2P_ALLOW_ALL_RESOLVABLE_HUNTS` | strict true |
| `allow_high_risk_hunt_classes` | `P2P_ALLOW_HIGH_RISK_HUNT_CLASSES` | strict true |
| `private_parties_only` | `P2P_PRIVATE_PARTIES_ONLY` | true unless the value trimmed is `false` (any case) |
| `max_party_size` (i64) | `P2P_MAX_PARTY_SIZE` | default 4; a `js_number` integer 1-4, else 4 (blank → 4) |
| `percent_rollout` (f64) | `P2P_PERCENT_ROLLOUT` | finite 0-100, else 0 (blank → 0) |
| `force_sdr` | `P2P_FORCE_SDR` | strict true |
| `canary_enabled` | `P2P_TIER0_CANARY_ENABLED` | strict true |
| `trust_tier` (u8) | `P2P_TRUST_TIER` | 1 only when the value trimmed is exactly `1`; else 0 |
| `tier1_rewards_enabled` | `P2P_TIER1_REWARDS_ENABLED` | strict true |
| `tier1_shadow_approved` | `P2P_TIER1_SHADOW_APPROVED` | strict true |
| `tier1_allowed_accounts` | `P2P_TIER1_ALLOWED_ACCOUNTS` | CSV set |
| `tier1_allowed_hunts` | `P2P_TIER1_ALLOWED_HUNTS` | CSV set |
| `dedicated_hunts_disabled` | `P2P_DISABLE_DEDICATED_HUNTS` | strict true, **or** no DeployServer configured |
| `training_enabled` | `P2P_TRAINING_ENABLED` | strict true |

`dedicated_servers_configured(settings)` = `DEPLOYSERVER_URL` trimmed non-empty. Tier 0 is no-reward transport;
tier 1 decides rewards from Guard's evidence and needs its own switches. Every gate fails closed.

#### 2.9.3 Routing a hunt

- `is_training_hunt_id(id)` = contains `TrainingDojo`. `is_p2p_training_request(game_mode, id, policy)` = enabled,
  training enabled, mode `SHARED` and a training id.
- Player-hosted hunt shape: mode `ISLAND` or `SHARED`; id non-blank after trim and not containing
  `ReturnToRamsgate`; a training id only with training enabled and mode `SHARED`.
- High-risk classes: the lower-cased id contains any of `trial`, `arena`, `leaderboard`, `gauntlet`, `escalation`,
  `race`, `competitive`.
- `is_known_runtime_incompatible(id)`: some `_`-separated segment equals `ftue` ignoring case.
- `rollout_bucket(account_ids, hunt_id) -> f64`: dedupe and sort the ids (byte order), join with `,`; SHA-256 of
  `<joined>\n<hunt_id>`; the first 4 bytes as a big-endian u32 ÷ 4,294,967,296 × 100 (in [0, 100)). Vector:
  `(["player-b", "player-a"], "hunt")` → `99.4095321977511`, for any order or duplicates.
- `RouteInput { game_mode, hunt_id, account_ids, party_size, is_private_party }`; `RouteDecision { route:
  HuntRoute (Dedicated | PlayerHosted), reason: RouteReason, rollout_bucket: Option<f64> }`; `RouteReason::as_str`
  gives the codes below.
- `decide_hunt_route(input, policy, player_hosted_implemented, emergency_stop)`, gates in order; the first that fails
  decides `Dedicated` with its reason and no bucket unless stated:
  1. emergency stop → `P2P_EMERGENCY_STOP`;
  2. not enabled → `FEATURE_DISABLED`;
  3. not the player-hosted shape → `NOT_A_PLAYER_HOSTED_HUNT`;
  4. private-only and not private → `PRIVATE_PARTY_REQUIRED`;
  5. size < 1 or > max → `PARTY_SIZE_EXCEEDED`;
  6. not allow-all and the account set empty → `ACCOUNT_ALLOWLIST_EMPTY`;
  7. not allow-all and some account missing → `ACCOUNT_NOT_ALLOWLISTED`;
  8. not all-resolvable and the hunt set empty → `HUNT_ALLOWLIST_EMPTY`;
  9. not all-resolvable and the hunt missing → `HUNT_NOT_ALLOWLISTED`;
  10. known incompatible → `HUNT_RUNTIME_INCOMPATIBLE` (no switch overrides it);
  11. a high-risk class without its switch → `HUNT_CLASS_EXCLUDED`;
  12. canary off → `CANARY_DISABLED`;
  13. tier 1 only: rewards off → `TIER1_REWARDS_DISABLED`; shadow not approved → `TIER1_SHADOW_NOT_APPROVED`; an
      account outside the tier-1 set → `TIER1_ACCOUNT_NOT_ALLOWLISTED`; the hunt outside it →
      `TIER1_HUNT_NOT_ALLOWLISTED`;
  14. rollout ≤ 0 → `ROLLOUT_DISABLED`;
  15. bucket ≥ rollout → `ROLLOUT_EXCLUDED` (with the bucket);
  16. not implemented → `PLAYER_HOSTED_NOT_IMPLEMENTED` (with the bucket);
  17. → `PlayerHosted`, `ELIGIBLE`, with the bucket.
- `relay_only_warning(settings)`: `Some` only when `P2P_HUNTS_ENABLED` is strict true and `P2P_FORCE_SDR` is not:
  `WARNING: P2P hunts are enabled without P2P_FORCE_SDR=true: players may see each other's IP address during a hunt,
  which the Privacy Policy says they do not. Set P2P_FORCE_SDR=true to relay all hunt traffic.` (one line).

### 2.10 Host policy (`host_policy`)

#### 2.10.1 Artifact hashes

- `KNOWN_GAME_EXE_SHA256: &[(i64, &str)]` = one entry: changelist 647472 →
  `f3f95fbf22709ef077a11632ed95d675b7209b7e0172d9ecb9ba7d02a1fbb66b` (Dauntless 1.14.7,
  `Dauntless-Win64-Shipping.exe`, 121,407,264 bytes).
- `ArtifactSources { runtime_manifest: Option<Value> (the client runtime feed's latest.json), guard_artifacts:
  Option<Vec<(role, sha256)>> }`, Default empty.
- `TARGET_CHANGELIST` is read with `js_number` and kept only when an integer of magnitude ≤ 2^53-1.
- The runtime feed applies only when the target changelist is unset, or the manifest has no `targetChangelist`, or
  that is the same integer.
- Feed values: `internal_server` = the manifest's `sha256`; `winmm_proxy` = the `extraFiles` entry whose `name` equals
  `winmm.dll` ignoring case, its `sha256`; `steam_bridge` = Guard role `TRANSPORT_MODULE`; `steam_runtime` = Guard role
  `TRANSPORT_RUNTIME`, else `TRANSPORT_MODULE` (the ICE transport has no separate runtime); `admission_public_key` =
  Guard role `ADMISSION_KEY`; `game_exe` = the known hash of the target changelist. A feed value counts only when it
  is 64 hex characters; it is lower-cased.
- `read_artifact_policy(settings, sources) -> Result<ArtifactPolicy { hashes, stale_pins: Vec<String> }, String>`.
  Per artifact, the settings names in order (the first one set non-blank wins; later names are older spellings):

| Artifact | Settings |
| --- | --- |
| `game_exe` | `P2P_GAME_EXE_SHA256` |
| `internal_server` | `P2P_INTERNAL_SERVER_SHA256` |
| `winmm_proxy` | `P2P_WINMM_SHA256` |
| `steam_bridge` | `P2P_TRANSPORT_MODULE_SHA256`, `P2P_STEAM_BRIDGE_SHA256` |
| `steam_runtime` | `P2P_TRANSPORT_RUNTIME_SHA256`, `P2P_STEAM_RUNTIME_SHA256` |
| `admission_public_key` | `P2P_ADMISSION_PUBLIC_XY_SHA256` |

  A pin that is not 64 hex → `Err("Missing or invalid <first name>.")`. A pin wins over the feed (lower-cased); when
  the feed has a different value, push to `stale_pins`: `[P2PArtifacts] <the set name> pins <first 12 of pin>... but
  the feed serves <first 12 of feed>...; the pin wins, so hosts on the feed's file are refused
  (HOST_RUNTIME_UNVERIFIED). Remove the pin to follow the feed.` No pin and no feed value → the same `Err`. The
  artifacts are evaluated in table order; the first error returns.
- `artifact_hashes_match(actual, expected)`: all six actual values are 64 hex and equal the expected ignoring case.

#### 2.10.2 Launcher versions

`version_at_least(actual, minimum)`: both must parse as semver after trim: `MAJOR.MINOR.PATCH` of non-empty digit
runs, then optionally `-prerelease` and/or `+build` (each of `A-Za-z0-9.-`, non-empty); anything else → false.
Compare the numbers; with equal cores: no prerelease on `actual` → true; prerelease on `actual` only → false; both →
compare dot-separated identifiers in turn: numeric vs numeric by value, numeric < alphanumeric, alphanumeric by byte
order; when one list runs out first, it is smaller; all equal → true. Build metadata is ignored.

#### 2.10.3 `EligibilityPolicy` (`ReadP2PHostEligibilityPolicy`)

"Bounded" = `js_number(value)` finite and within the range, else the default (unset counts as invalid).

| Field | Setting | Default | Range |
| --- | --- | ---: | --- |
| `minimum_launcher_version` | `P2P_HOST_MIN_LAUNCHER_VERSION` | `0.1.35` | trimmed, three non-empty digit runs joined by `.`; else the default |
| `minimum_free_memory_mib` | `P2P_HOST_MIN_FREE_MEMORY_MIB` | 4096 | 1024-262,144 |
| `minimum_commit_headroom_mib` | `P2P_HOST_MIN_COMMIT_HEADROOM_MIB` | 6144 | 1024-262,144 |
| `minimum_logical_cpu_count` | `P2P_HOST_MIN_LOGICAL_CPUS` | 4 | 2-256 |
| `maximum_cpu_load_percent` | `P2P_HOST_MAX_CPU_PERCENT` | 85 | 10-100 |
| `maximum_worst_peer_rtt_ms` | `P2P_HOST_MAX_WORST_RTT_MS` | 250 | 20-2000 |
| `maximum_jitter_ms` | `P2P_HOST_MAX_JITTER_MS` | 80 | 1-1000 |
| `maximum_packet_loss_percent` | `P2P_HOST_MAX_PACKET_LOSS_PERCENT` | 10 | 0-100 |
| `presence_maximum_age_ms` | `P2P_HOST_PRESENCE_MAX_AGE_MS` | 30,000 | 5,000-300,000 |
| `offer_lifetime_ms` | `P2P_HOST_OFFER_LIFETIME_MS` | 15,000 | 5,000-120,000 |
| `startup_lifetime_ms` | `P2P_HOST_STARTUP_LIFETIME_MS` | 120,000 | 30,000-600,000 |
| `lease_lifetime_ms` | `P2P_HOST_LEASE_LIFETIME_MS` | 10,000 | 4,000-60,000 |
| `suspect_lifetime_ms` | `P2P_HOST_SUSPECT_LIFETIME_MS` | 15,000 | 5,000-120,000 |
| `rejoin_lifetime_ms` | `P2P_REJOIN_LIFETIME_MS` | 60,000 | 10,000-300,000 |
| `maximum_reconnects` | `P2P_MAX_RECONNECTS` | 3 | 1-10, floored |

The `_ms` fields and `maximum_reconnects` are i64 (truncated); the rest f64. `hunt_drain_cap_ms(settings)`:
`P2P_HUNT_DRAIN_GRACE_MS` blank or unset → `None`; a finite `js_number` in 30,000-600,000 → `Some(floor)`; anything
else → `None`.

#### 2.10.4 Eligibility

`host_ineligibility_reason(presence, reliability: Option, active_host_sessions, policy, guard_required, now)`, first
match:

1. not `consent_to_host` → `HOST_CONSENT_REQUIRED`;
2. `launcher_version` below the minimum (§2.10.2) → `HOST_LAUNCHER_OUTDATED`;
3. not `artifacts_verified`, not `runtime_supported` or not `steam_runtime_ready` → `HOST_RUNTIME_UNVERIFIED`;
4. `guard_required` and `guard_verified != Some(true)` → `HOST_GUARD_UNVERIFIED`;
5. `expires_at <= now` or `now - observed_at > presence_maximum_age_ms` → `HOST_PRESENCE_STALE`;
6. free memory or commit headroom below its minimum → `HOST_MEMORY_LOW`;
7. CPUs below the minimum, load above the maximum, or `power_saver` → `HOST_CPU_UNSUITABLE`;
8. worst-peer RTT, jitter or loss above its maximum → `HOST_NETWORK_UNSUITABLE`;
9. `active_host_sessions > 0` → `HOST_ALREADY_ACTIVE`;
10. a reliability `cooldown_until` after now → `HOST_COOLDOWN`;
11. → `None`.

#### 2.10.5 Score (lower is better)

`score_host(presence, reliability: Option, policy, active_host_sessions) -> (f64, ScoreBreakdown)`. Each risk but
integrity is clamped to 0-100:

- latency = 100 × worst-peer RTT ÷ max RTT;
- network = 50 × loss ÷ max(max loss, 1) + 50 × jitter ÷ max jitter;
- memory = 100 × minimum commit ÷ max(commit headroom, 1);
- cpu = the load percent;
- stability = 25 with no attempts, else 100 × (failed + unexpected) ÷ (succeeded + failed + unexpected), from the
  reliability record (none = zeros);
- host load = 100 when `active_host_sessions > 0`, else 0;
- integrity = 100 when artifacts are unverified; else the Guard risk score (0 if unset) when `guard_verified ==
  Some(true)`; else 25.

Score = 0.35 latency + 0.20 network + 0.15 memory + 0.10 cpu + 0.10 stability + 0.05 host load + 0.05 integrity,
rounded to three decimals (`round(x × 1000) ÷ 1000`, half away from zero).

### 2.11 Lifecycle rules, steps and recovery (`lifecycle`)

#### 2.11.1 State sets and constants

- `LIFECYCLE_STATES` (what the monitor scans): HOST_SELECTING, HOST_OFFERED, STARTING, HOST_READY, ADMITTING,
  RUNNING, DRAINING, SUSPECT, ABORTING, RECOVERING.
- `HOST_END_STATES` (the host's own end applies): HOST_READY, ADMITTING, RUNNING, SUSPECT.
- `HUB_RETURN_END_STATES` (the last hub return closes these): HOST_READY, ADMITTING, RUNNING.
- `SUPERSEDE_STATES` (a party's new request may supersede): HOST_SELECTING, HOST_OFFERED, STARTING, HOST_READY,
  ADMITTING, RUNNING, SUSPECT, ABORTING.
- Started states (not public): HOST_READY, ADMITTING, RUNNING, SUSPECT.
- `CANCELLABLE_FORMING_STATES`: HOST_SELECTING, HOST_OFFERED, STARTING, HOST_READY, ADMITTING.
- `DRAIN_COMPLETION_REASON` = `SUPERSEDED_REQUESTER_LEFT_HUNT`; `HOST_GUARD_START_GRACE_MS` = 90,000.
- A hunt "ran" when RUNNING, DRAINING, or SUSPECT with pre-suspect RUNNING.

#### 2.11.2 Pure rules

1. `select_recovery_participants(roster, before_active)`: pre-active → the whole roster; active → the ADMITTED
   participants with both a `local_bridge_ready_at` and a `local_bridge_port`.
2. `should_use_dedicated_pre_active_fallback(before_active, dedicated_hunts_disabled)` = before active and dedicated
   not disabled.
3. `is_pre_active_operator_stop_state(state)` = HOST_SELECTING, HOST_OFFERED, STARTING or HOST_READY.
4. `GuardRole`: `Client` < `Host` (ordered), wire `client`/`host`. `GuardListError { code: &'static str, message:
   &'static str }`.
5. `parse_guard_presence_sessions(body) -> Result<Vec<(GuardRole, String)>, GuardListError>`: `guardSessions` absent
   or null → the legacy `guardSessionId` (a string of 1-64 UTF-16 units without NUL) as one `client` session, or none.
   Otherwise an array of at most 2 entries, each with `role` `client`/`host` and `guardSessionId` of 1-64 units, no
   NUL, exactly 36 bytes of hex digits and `-`, roles not repeated; anything else → `P2P_TICKET_INVALID` "The Launcher
   Guard session list is malformed.".
6. `parse_guard_required_roles(body, sessions, realm_host_only)`: `guardRequiredRoles` absent or null → the sessions'
   roles in order. Else an array of at most 2 distinct `client`/`host` strings, else `P2P_TICKET_INVALID` "The
   Launcher Guard role list is malformed.". A list with `host` and without `client` is refused unless
   `realm_host_only` and the list is exactly `["host"]`: `GUARD_REQUIRED` "A hosted process requires both client and
   host Guard roles." (a stale server session must not stand in for the game client).
7. `host_guard_still_starting(created_at, required, verified, now)`: some required role is unverified, every
   unverified one is `Host`, and `0 <= now - created_at < 90,000`.
8. `participants_still_in_hunt(roster, host)`: account ids, in roster order, of the entries with no
   `returned_to_hub_at` that are ADMITTED, or are the host and not LOST.
9. `clean_end_terminal_state(hunt)`: COMPLETED when the trust tier (unset = 0) is 0 and the hunt ran; else ABORTED (a
   tier-1 COMPLETED is Guard's completion).
10. `host_loss_is_clean_end(hunt) -> Option<&'static str>`: only for tier 0 in a started state; Guard encounter state
    `COMPLETION_REQUESTED` → `TIER0_HUNT_COMPLETED`; it ran and nobody is still in it (host included) →
    `TIER0_PARTY_LEFT`; else `None`.
11. `mark_participant_returned_to_hub(roster, account, at, host) -> Option<(Vec<Participant>, bool, Vec<String>)>`:
    `None` when the account is not rostered, or is not the host and is neither ADMITTED nor LOST. The bool says the
    entry had no return time (the first time is kept); the list is rule 8 on the new roster.
12. `PartySessionAction`: `Reuse`, `Supersede(&'static str)`, `Drain(&'static str)`, `Conflict(&'static str)`.
    `classify_existing_party_session(existing, hunt_id, requester, now)`:
    1. ABORTING or SUSPECT → `Supersede SESSION_ENDING`;
    2. Guard encounter state `COMPLETION_REQUESTED` or `ABORTED` → `Supersede HUNT_FINISHED`;
    3. a started state with `lease_expires_at <= now` → `Supersede HOST_LEASE_EXPIRED`;
    4. a started state and the requester has left (rostered and ADMITTED or LOST, or has a return time, or the session
       is RUNNING and the requester is its host) → `Drain REQUESTER_LEFT_HUNT` when RUNNING, tier 0 and not a hub,
       else `Supersede REQUESTER_LEFT_HUNT`;
    5. a different hunt id → `Supersede PARTY_CHANGED_HUNT`;
    6. the requester not rostered → `Conflict ROSTER_FROZEN` when RUNNING or ADMITTING, else
       `Supersede ROSTER_CHANGED`;
    7. → `Reuse`.

    Then any `Supersede(reason)` of a RUNNING, tier-0, non-hub session becomes `Drain(reason)` unless the reason is
    `HOST_LEASE_EXPIRED` or `SESSION_ENDING`.
13. `is_cancellable_forming(hunt)`: a cancellable forming state and no ADMITTED participant.
14. `recovery_destination(before_active, hunt_id) -> (&'static str game_mode, String hunt_id, bool is_hub)`: active →
    `("CITY", "ShatteredIsles_ReturnToRamsgate", true)`; pre-active training id → `("SHARED", id, true)`; else
    `("ISLAND", id, false)`.

#### 2.11.3 Transaction steps

Each runs inside one repository transaction (`HuntTx`, §3.3) and returns the new session, or `None` when the session
had moved on. A *transition* is appended with the given reason, actor type and account; `from` = the state read;
`to`, `state_version` and `generation` = the new row's. A *fence* is a `FENCE_NOW` control message to an account with
the session id and generation. *CAS* is the repository's compare-and-set: the change applies only while the
generation, one of the expected states and the read `state_version` still hold, and it bumps `state_version` by one.

1. **`end_session(tx, session_id, generation, reason, allowed, actor, actor_account, fence_bridges, now)`**
   (`EndP2PHuntSession`, realms' part): missing, another generation or a state outside `allowed` → `None`. Terminal =
   `clean_end_terminal_state`. Revoke the generation's capabilities. CAS from the read state: state = terminal,
   `terminal_at` now, `completion_reason` = reason, `failure_reason` = reason only when ABORTED; clear
   `active_party_key`, `lease_expires_at`, `suspect_since`, `suspect_deadline`, `pre_suspect_state`,
   `offer_expires_at`, `startup_deadline`, `drain_deadline`, `recovery_retry_at`. Transition with the reason. If it
   was HOST_OFFERED with an offer id and a host: the offer OFFERED → CANCELLED with the reason. COMPLETED with a host:
   `clean_completions + 1` (no cooldown). If the read state was not HOST_SELECTING and `fence_bridges`: fence every
   roster account, payload `{ reason, recoveryAction: "NONE", cleanEnd: true }`, life 10 min. Admission tickets and
   the party's activity are the caller's.
2. **`mark_suspect(tx, hunt, reason, suspect_lifetime_ms, now)`**: CAS from the hunt's state: SUSPECT,
   `pre_suspect_state` = the old state, `suspect_since` now, `suspect_deadline` now + lifetime, `failure_reason` =
   reason; transition (SYSTEM).
3. **`recover_from_suspect(tx, hunt)`**: no pre-suspect state → `None`; CAS from SUSPECT back to it, `failure_reason` =
   `LEASE_RECOVERED`, the three suspect fields cleared. It appends no transition itself (the hub tick appends one,
   §2.12.6).
4. **`retry_pre_active_host(tx, hunt, reason, now)`**: revoke the generation; CAS from its state: generation + 1,
   HOST_SELECTING, host account, host peer and bridge instance emptied, `failure_reason` = reason, and
   `host_offer_id`, `offer_expires_at`, `startup_deadline`, `lobby_id`, `lease_expires_at`, `metadata_nonce` cleared;
   transition (SYSTEM). With a host: `starts_failed + 1` (no cooldown) and a fence to the old host for the old
   generation, payload `{ reason }`, life 5 min.
5. **`expire_offer(tx, hunt, now)`**: no offer id → `None`; the offer OFFERED → EXPIRED with reason `TIMEOUT` (nothing
   moved → `None`); `offers_expired + 1`; reset to selection; transition `HOST_OFFER_EXPIRED` (SYSTEM).
6. **`reset_to_selection(tx, hunt)`**: CAS from HOST_OFFERED: HOST_SELECTING, host account and peer emptied, offer id
   and expiry cleared. No transition.

#### 2.11.4 Recovery

`trait Recovery: Send + Sync` with `prepare(&self, session_id: String, generation: i64, reason: String,
before_active: bool) -> BoxFuture<'_, Result<(), RealmsError>>` and `advance(&self, session: HuntSession) ->
BoxFuture<'_, Result<(), RealmsError>>` (`FinalizeRecovery` for a RECOVERING session whose window ran out,
`ResumeRecovery` for a due ABORTING one). Recovery to a dedicated server or back to Ramsgate is isles' (DeployServer,
party); `RealmsRecovery` is the implementation on `store` without one: an unrecoverable session ends ABORTED (actor
`realms-recovery`).

**`prepare`**, in one transaction:

1. missing, another generation, or COMPLETED, ABORTED, RECOVERING or DRAINING → nothing.
2. ABORTING → `end_session(NO_DEDICATED_SERVERS, allowed [ABORTING], SYSTEM, fence)`; remembered as "was aborting".
3. Otherwise: revoke the generation; CAS to FENCED with `failure_reason` = reason; transition with the reason; if it
   was HOST_OFFERED with an offer and a host, the offer OFFERED → CANCELLED (reason); CAS FENCED → ABORTED with
   `terminal_at` now, `failure_reason` = reason, and `active_party_key`, `lease_expires_at`, `startup_deadline`,
   `recovery_retry_at`, `offer_expires_at`, `suspect_deadline`, `suspect_since`, `pre_suspect_state`,
   `host_offer_id` cleared; transition with reason `<reason>_P2P_ONLY` (before active), `<reason>_REALM_ONLY` (an
   active realm hunt) or `<reason>_NO_DEDICATED_FALLBACK`. Cooldown = now + 5 min.
   - Before active: to each roster account a `HOST_OFFER_CANCELLED`, payload `{ reason, recoveryAction:
     "CANCEL_MATCHMAKING" }`, life 10 min; for a realm hunt whose non-empty host is not rostered, the same to the host;
     with a host, `starts_failed + 1` and the cooldown.
   - Active: a fence to every roster account and to the host (once, when not rostered), payload `{ reason,
     recoveryAction: "NONE" }`, life 10 min; with a host, `unexpected_failures + 1` and the cooldown.

After the commit, when something ended: if its roster is not empty, `IslesCore::cancel_travel(hunt_definition_id,
roster accounts, reason)` (result ignored); then one warn line:

- was aborting: `[P2P] ending <id> instead of recovering it: dedicated recovery seam not wired reason=<reason>`;
- before active: `[P2P] pre-active failure canceled without dedicated hunt fallback session=<id> reason=<reason>`;
- an active realm hunt: `[P2P] active realm hunt failed; ended without recovery session=<id> reason=<reason>`;
- otherwise: `[P2P] active hunt failed without dedicated recovery seam; ended session=<id> reason=<reason>`.

The leases test checks that a warn line contains the session id and the reason.

**`advance(session)`**: RECOVERING → in a transaction, if still RECOVERING with that generation, CAS to ABORTED with
`terminal_at`, `failure_reason` = `RECOVERY_WINDOW_EXPIRED`, `active_party_key` and `recovery_deadline` cleared;
transition `RECOVERY_WINDOW_EXPIRED` (SYSTEM); warn `[P2P] recovery window expired; ended session=<id>`. ABORTING →
warn the "was aborting" line with its `failure_reason` (or `HOST_FAILED`), then `end_session(NO_DEDICATED_SERVERS,
[ABORTING], SYSTEM, fence)`. Other states: nothing.

### 2.12 Leases and the lifecycle monitor (`leases`)

#### 2.12.1 Settings and parts

`HostingSettings { policy: HostingPolicy, eligibility: EligibilityPolicy, realm_hub: RealmHubPolicy, realm_hosting:
bool }` (Clone, PartialEq); `from_settings` reads each (§2.9, §2.10, §2.8.6). Constants: at most 100 sessions per
monitor pass; observations count for 15,000 ms; admitted peers ≤ 4 for a hunt and ≤ 64 for a hub; control messages
live 10 min. Two body helpers are crate-visible: *NumberIn(value, min, max)* = a JSON number, finite, within range;
*IntegerIn* = that and a whole value.

#### 2.12.2 Lease renewal

`renew(capability, body) -> LeaseReply { renewed, sessionId, generation, leaseExpiresAt, state }` (`RenewHostLease`;
the route has checked the capability for scope `LEASE_HEARTBEAT`).

1. Validation, all `P2P_TICKET_INVALID` "The host heartbeat is malformed.": `leaseSequence` integer 1-2^53-1;
   `bridgeAlive` and `serverReady` booleans; `admittedPeerCount` integer 0-64; `queuePressure` integer 0-100; a peer
   count above 4 only when the capability's session is a realm hub.
2. `bridgeAlive` or `serverReady` false → the host process is down (below), then `P2P_GENERATION_FENCED` "The host
   process reported a fatal failure.".
3. Emergency stop → `P2P_EMERGENCY_STOP` "Player-hosted hunt operations are temporarily stopped.".
4. One transaction (actor: the host account): advance the capability's sequence (strictly newer and unrevoked; else
   `P2P_GENERATION_FENCED` "The lease sequence is stale."), then renew the session's lease to now +
   `lease_lifetime_ms` (the repository requires the generation, host peer id and bridge instance of the capability, a
   state among HOST_READY, ADMITTING, RUNNING, DRAINING, SUSPECT, and a sequence strictly above the session's
   `last_lease_sequence`; else `P2P_GENERATION_FENCED` "The hunt generation no longer accepts heartbeats."). A refusal
   rolls both back.
5. Answer with the session's id, generation and state, and the new expiry. A SUSPECT session stays SUSPECT until the
   monitor's next tick recovers it.

**Host process down**: a hub → `end_hub(HOST_PROCESS_UNHEALTHY, ABORTED, HOST, host)`; a DRAINING hunt of the
capability's generation → `end_hunt` with its `completion_reason` (or `SUPERSEDED_REQUESTER_LEFT_HUNT`), allowed
[DRAINING], HOST, fence; the same generation and `host_loss_is_clean_end` → `end_hunt` with `<clean>_HOST_STOPPED`,
allowed `HOST_END_STATES`, HOST, fence; otherwise `Recovery::prepare(session, generation, HOST_PROCESS_UNHEALTHY,
false)`.

`end_hunt(actor, hunt, reason, allowed, transition_actor, account, fence)` runs `end_session` in a transaction and,
when something ended, logs (info) `[P2P] session <id> generation=<g> ended cleanly state=<STATE> reason=<reason>`.

#### 2.12.3 Ending a hub (`end_hub`, `EndRealmHub`)

`end_hub(session_id, generation, reason, terminal, actor_type, actor_account) -> Result<Option<HuntSession>>`. One
transaction (actor: the account when given, else `realms-lifecycle`): missing, not a hub, another generation or not
live → `None`. Revoke the generation; CAS from its state to `terminal`, with `terminal_at` now, `completion_reason` =
reason, `failure_reason` = reason when ABORTED, and `active_party_key`, `lease_expires_at`, the three suspect fields,
`offer_expires_at`, `startup_deadline` cleared. Transition (reason, actor type, account). End the generation's hub
presence (every live row → LOST, purge at now + 7 d) and, to each returned row of this session whose account is not
the host, queue `REALM_SESSION_LEAVE` with payload `{ sessionId, generation, reason, hubKind, realmId }` (life 10
min). Queue a fence to the host, payload `{ reason, recoveryAction: "NONE", cleanEnd: terminal == COMPLETED }` (life
10 min). Reliability, without cooldown (the launcher restarts a dead hub within seconds): COMPLETED →
`clean_completions + 1`; a host-failure reason (§2.8.3) → `starts_failed + 1` for `HUB_STARTUP_TIMEOUT`, else
`unexpected_failures + 1`; any other reason → nothing.

After the commit: info `[RealmHub] hub <id> ended state=<STATE> reason=<reason>`; mirror the hub as `NONE`
(§2.12.4); for a Ramsgate hub with a realm, `mark_offline(realm, now)`, a failure logged warn `[RealmHub] could not
mark realm <realm> offline: <error>`.

#### 2.12.4 Mirroring a hub into its realm

`mirror_hub(hub, state)`: nothing without a realm. `active` = `count_active_realm_servers(realm)` capped at 32 (a
failed count leaves it out). Training: patch `training_state`; Ramsgate: patch `hub_state` and `hub_session_id`
(`Remove` for `NONE`, else `Set(session id)`); both add `active_servers` when counted. Written at now with actor
`realms-lifecycle`; a failure logs warn `[RealmHub] could not mirror hub state into realm <realm>: <error>`.

#### 2.12.5 Observations

`observe(account, session_id, body) -> ObservationReply { accepted, sessionId, generation, stored (omitted when None),
state }` (`RecordParticipantObservation`). The body's `generation` is read as a JSON integer. "The roster refusal" is
`P2P_GENERATION_FENCED` "The observation is not bound to the active roster generation.".

1. Unknown session → the roster refusal.
2. A hub: the account must be its host or have a hub presence row, and the generation must match, else
   `P2P_GENERATION_FENCED` "The observation is not bound to the active hub generation."; answer accepted with
   `stored: false` (a hub's health is its host's lease).
3. COMPLETED, ABORTED or DRAINING: the generation must match and the account be rostered, else the roster refusal;
   accepted, `stored: false`.
4. Not rostered, or another generation → the roster refusal.
5. The body, every failure `P2P_TICKET_INVALID` "The participant observation is malformed or stale.": `observedAt` a
   string of 1-40 units without NUL that `parse_iso` reads; `hostReachable`, `localBridgeAlive`, `metricsAvailable`
   booleans; `observedAt` within 15,000 ms of now either way; `routeKind` (missing = empty) one of `DIRECT`, `RELAY`,
   `CONNECTING`, `UNKNOWN`. With metrics: the route is not `UNKNOWN`, `lastPacketAgeMs` an integer 0-600,000,
   `pingMs` and `jitterMs` numbers 0-60,000, `packetLossPercent` 0-100. Without: the route is `UNKNOWN` and those four
   keys are absent or null.
6. Emergency stop → `P2P_EMERGENCY_STOP` "Player-hosted hunt operations are temporarily stopped.".
7. One transaction (actor: the account): re-read; still that generation and still rostered, else the roster refusal;
   record a new observation (uuid id, the participant's steam id, the three flags, the route, `lastPacketAgeMs`,
   `pingMs`, `jitterMs` rounded to integers and `packetLossPercent` as is, all only with metrics, and `observedAt`).
   Answer accepted with `stored` omitted, generation and state as read in step 1.

#### 2.12.6 The monitor

`sweep() -> Result<Vec<(String, SweepStep)>, RealmsError>` (`P2PHuntLifecycleMonitor.tick`): list up to 100 sessions
in `LIFECYCLE_STATES`, oldest first; read the emergency stop once; tick each in turn. A failing tick logs warn `[P2P
Lifecycle] session <id> tick failed: <error>` and the others go on. Nothing in the crate schedules `sweep` or
`sweep_lapsed_hosts`: the caller drives them (§8, R4).

`SweepStep`: `Suspect { reason: String }`, `Recovered`, `OfferExpired`, `RetriedHost { reason }`, `Ended { reason,
state: HuntState }`, `Selected`, `Recovery { reason, before_active: bool }`, `RecoveryAdvanced`,
`HubEntriesExpired(usize)` (Debug, Clone, PartialEq, Eq).

One session's tick (writes with actor `realms-lifecycle`):

1. **A hub**: the hub tick (§2.8.3) with `hosting_enabled = realm_hosting && policy.enabled`. `End` →
   `end_hub(reason, ABORTED, SYSTEM)` → `Ended`. `Suspect` → `mark_suspect` with `suspect_lifetime_ms` → `Suspect`.
   `Recover` → `recover_from_suspect` and, in the same transaction, a transition SUSPECT → the restored state with
   reason `LEASE_RECOVERED` (SYSTEM) → `Recovered`. `None` while RUNNING → expire the hub's waiting rows past their
   time (`expire_stale`, purge at now + 7 d) → `HubEntriesExpired(n)` when n > 0. Otherwise nothing.
2. **DRAINING**: reason = its `completion_reason` or `SUPERSEDED_REQUESTER_LEFT_HUNT`. *Lapsed* = no lease, or lease
   + `suspect_lifetime_ms` ≤ now. Lapsed and no stop → end without fences; a stop, or `drain_deadline <= now` → end
   with fences; else nothing. End = `end_hunt(allowed [DRAINING], SYSTEM)` → `Ended`. A drained hunt never goes
   SUSPECT and never recovers.
3. **Emergency stop**, state not ABORTING or RECOVERING → `Recovery::prepare(OPERATOR_P2P_STOP, before_active =
   is_pre_active_operator_stop_state(state))` → `Recovery`.
4. HOST_SELECTING → `begin_host_selection` → `Selected`.
5. HOST_OFFERED with `offer_expires_at <= now` → `expire_offer`; when it moved, `begin_host_selection` →
   `OfferExpired`.
6. STARTING with `startup_deadline <= now` → retry with `HOST_STARTUP_TIMEOUT`.
7. HOST_READY, ADMITTING, RUNNING:
   - *lapsed* = `lease_expires_at <= now`; lapsed and `host_loss_is_clean_end` → `end_hunt(<clean>_LEASE_RELEASED,
     HOST_END_STATES, SYSTEM, fence)` → `Ended`;
   - *failing* = the distinct accounts with an observation of this generation newer than now - 15,000 that reports the
     host unreachable or the bridge dead;
   - lapsed, or failing count ≥ min(roster size, 2) → `mark_suspect` with `PARTICIPANT_FAILURE_QUORUM` when failing ≥
     2, else `HOST_LEASE_MISSED` → `Suspect`. (An empty roster meets the quorum at once, §8 R9.)
8. SUSPECT: a lease set and after now, with a pre-suspect state → `recover_from_suspect` → `Recovered`. Else nothing
   until `suspect_deadline <= now`; then a clean end `<clean>_LEASE_RELEASED` (as in 7) if `host_loss_is_clean_end`;
   else pre-suspect HOST_READY → retry with `HOST_FAILED_BEFORE_RELEASE`; else `Recovery::prepare(failure_reason or
   HOST_FAILED, false)` → `Recovery`.
9. RECOVERING with `recovery_deadline <= now`, or ABORTING with no `recovery_retry_at` or one that is due →
   `Recovery::advance` → `RecoveryAdvanced`.
10. Anything else → nothing.

*Retry*: `retry_pre_active_host` (§2.11.3); when it moved, `begin_host_selection` → `RetriedHost { reason }`.

**Cadence and thresholds** (defaults of §2.10.3): an accepted heartbeat makes the lease live `lease_lifetime_ms`
(10 s), so a host renews well inside that; sequences are fenced per capability (strictly increasing) and per session
(strictly above `last_lease_sequence`, which host-ready resets to 0). A tick that finds the lease lapsed marks the
session SUSPECT for `suspect_lifetime_ms` (15 s); a renewal inside the window recovers it on a later tick; past the
window it is retried (pre-active), ended cleanly (tier 0 with nobody left or Guard's completion) or handed to
recovery. The leases test runs (lease to +10 s): a tick at +10 s → `Suspect HOST_LEASE_MISSED` (expiry equal to now
has lapsed); a heartbeat at +12 s, then a tick → `Recovered`; a tick at +22 s → SUSPECT again, one at +37 s →
`Recovery { HOST_LEASE_MISSED, false }`. With `RealmsRecovery` wired (ticks at +10 s and +25 s) the hunt ends ABORTED
with `failure_reason` `HOST_LEASE_MISSED`, lease and party key cleared, the capability revoked, `FENCE_NOW` queued for
the other player, and a later sweep finds nothing.

### 2.13 Host selection (`selection`)

#### 2.13.1 Peer ids

- `derive_ice_peer_id(account) -> String`: SHA-256 of the UTF-8 text `mysticparadox-ice-peer-v1\n<account>`; the first
  8 bytes as a big-endian u64 with the top bit cleared; 0 becomes 1; written in decimal. rifts-admission's tickets use
  the same derivation. Vector: `derive_ice_peer_id("account-1")` = `518169688431504238`.
- An ICE peer report: a string of 1-20 ASCII digits, no leading `0`, whose value fits a u64.
- A Steam transport id: a string of exactly 17 ASCII digits whose value is within 76,561,197,960,265,728 and that plus
  0xFFFFFFFF, inclusive.

#### 2.13.2 Artifacts in a presence body

`parse_artifacts(Option<&Value>) -> Option<ArtifactHashes>`: the value must be an object; for each field, the first
present non-null key among: `gameExe`; `internalServer`; `winmmProxy`; `transportModule` then `steamBridge`;
`transportRuntime` then `steamRuntime`; `admissionPublicKey`. Its value must be a string of 1-64 units without NUL,
else it reads as empty. All six must be 64 hex characters, else `None`.

#### 2.13.3 Presence (`submit_presence`, `SubmitHostPresence`)

`submit_presence(account, body) -> PresenceReply { accepted, revision, expiresAt, artifactsVerified, guardVerified }`.

1. Read `transportReady` (else `steamRuntimeReady`) as a boolean; the reported peer = `peerId` when present and
   non-null, else `steamId` when non-null; the artifacts; `observedAt` (1-40 units, `parse_iso`); `consentToHost`,
   `powerSaver`, `runtimeSupported` booleans. Any missing → `P2P_TICKET_INVALID` "The host presence report is
   malformed or stale.". The same refusal when `observedAt` is more than 30,000 ms from now either way, or
   `preflightOnly` is present, non-null and not a boolean.
2. Metrics, all required, else `P2P_TICKET_INVALID` "The host presence metrics are outside their bounds.":
   `freePhysicalMemoryMiB` and `commitHeadroomMiB` integers 0-1,048,576; `logicalCpuCount` integer 1-1024;
   `cpuLoadPercent` 0-100; `backendRttMs`, `worstPeerRttMs`, `jitterMs` 0-60,000; `packetLossPercent` 0-100.
3. `expires_at` = `observedAt` + `presence_maximum_age_ms`. `revision` = the account's stored revision (any age) + 1,
   1 for the first.
4. `artifactsVerified` = `artifact_hashes_match(parsed, expected_artifacts())`; an `Err` from that source logs warn
   `[P2PArtifacts] expected host artifact hashes unavailable (<error>); hosts stay unverified` and counts as false.
5. Transport: with ICE (`OperationsView::ice_transport()`) the reported peer must be an ICE peer report and the stored
   id is `derive_ice_peer_id(account)` (the reported id is only the module instance); without ICE it must be a Steam
   transport id, stored as is. Ready without a valid id, or not ready with a peer present → `P2P_TICKET_INVALID` "The
   co-op transport observation is malformed.".
6. Guard: the sessions (§2.11.2.5; its refusal as is); `realmHostSessions` an integer 0-8 (else 0); *realm host only*
   = that > 0 and the server confirms the account hosts a realm with a running server: realm hosting on, the account
   hosts a realm, and `count_running_realm_servers(realm) > 0` (a lookup error logs warn `[P2P] realm host check for
   <account> failed; the client role stays required: <error>` and counts as false); the required roles
   (§2.11.2.6; its refusal as is). For each session ask `GuardSessions::find_healthy(account, id, role)` (an error is
   `Database`). `guardVerified` = at least one role is required and every required role has a healthy session.
   Required but unverified: `OperationsView::guard_refuses()` → `GUARD_REQUIRED` "A current healthy Launcher Guard
   session is required to host."; else info `[GuardObserve] would refuse hosting for <account>` and go on (observe
   mode refuses nothing).
7. `preflightOnly: true` with artifacts verified → answer without writing; `revision` = the stored one (0 if none).
8. Upsert the presence (actor: the account): the metrics and flags, `steam_runtime_ready` = transport ready,
   `steam_id`, `launcher_version` = `launcherVersion` (1-64 units) or `unknown`, `guard_session_id` and
   `guard_risk_score` of the healthy client session (else the host one), `guard_verified: Some(..)`, `guard_roles` =
   every healthy session (`role` `client`/`host`, its id, manifest sequence, `sequence` = its last sequence, risk
   score), `realm_host_sessions: Some(..)`. When transport-ready with an id that parses as u64 and a runtime is wired:
   put a VERIFIED peer link in rifts-admission's store (revision = max(revision, 1), verified now, method
   `ICE_ACCOUNT_DERIVED` with ICE, else `HOST_PRESENCE`; the result is ignored). Answer `accepted: true` with the new
   revision.

#### 2.13.4 Ranking

- **`rank_hosts(hunt) -> Vec<RankedHost { account_id, steam_id, score, breakdown }>`** (`RankHosts`): the roster's
  presences observed since now - `presence_maximum_age_ms` and unexpired; for each rostered participant not in
  `tried_host_account_ids` and with a presence: its reliability and `count_active_host_sessions`; ineligible (guard
  required = `guard_refuses()`) → info `[P2P] host candidate <account> ineligible reason=<reason>` and skip; else
  score it. Sort by score ascending (total order on f64), then account id. `steam_id` is the participant's.
- **`rank_realm_host(hunt) -> Result<Result<Vec<RankedHost>, ()>, RealmsError>`** (`RankRealmHost`; `Err(())` means
  REALM_FULL): no realm id, no such realm, or its host already tried → empty. No fresh presence of the host → info
  `[P2P] realm host <host> for realm <realm> has no fresh presence`, empty. `realm_host_ineligibility_reason(presence,
  policy, 0, 0, guard_refuses(), now)` (the memory floors are the capacity check's) → info `[P2P] realm host <host>
  for realm <realm> ineligible reason=<reason>`, empty. Capacity with `active_servers` = max(count_active_realm_servers
  - 1, 0) (this hunt already counts), `max_servers` = the realm's hosting value or 6, and the realm hub floors:
  `ServerLimit`/`CommitLow` → `Err(())`; `OkLowFreeMemory` → warn `[RealmCapacity] realm <realm> hunt <session> starts
  on a host with <free> MiB free physical memory`. Then one candidate: the host, `steam_id` = its derived ICE peer id,
  scored with no reliability and 0 active sessions.

#### 2.13.5 Offering (`begin_host_selection`, `BeginHostSelection`)

`begin_host_selection(session_id)`: missing or not HOST_SELECTING → nothing. Emergency stop →
`Recovery::prepare(OPERATOR_P2P_STOP, true)`. Policy disabled → `prepare(P2P_DISABLED, true)` (the monitor still
runs so recorded hunts can end, but starts no selection). Rank: a realm hunt by its realm host, else the roster. No
candidate → `prepare(REALM_FULL` when the realm was full, else `NO_ELIGIBLE_HOST, true)`. Otherwise one transaction
(actor `realms-selection`), if the session is still HOST_SELECTING: CAS to HOST_OFFERED with the candidate's account
and peer, a new offer id (uuid), `offer_expires_at` = now + `offer_lifetime_ms`, and the account appended to
`tried_host_account_ids` once; transition `HOST_CANDIDATE_SELECTED` (SYSTEM); create the offer (rank 1, the score and
breakdown, OFFERED, offered now, expiring with the session's offer); queue `HOST_OFFERED` to the candidate with payload
`{ offerId, huntDefinitionId, partySize (roster length), expiresAt (iso), estimatedMemoryMiB (the policy's minimum
commit headroom), score }`, expiring with the offer. When offered, info `[P2P] host offer <offer> session=<id>
account=<account> score=<score>`.

#### 2.13.6 Answering an offer (`respond_to_offer`, `RespondToHostOffer`)

`respond_to_offer(account, offer_id, body) -> OfferReply { accepted, offerId, sessionId, generation }`.

1. The offer id must be 36 bytes of hex digits and `-`, and `accept` a boolean, else `P2P_TICKET_INVALID` "The host
   offer response is malformed.".
2. The offer must exist and be this account's, else `P2P_SESSION_NOT_FOUND` "Host offer not found.".
3. `accept` and the offer already ACCEPTED → success (a lost ACK is retried).
4. Not OFFERED, or `expires_at <= now` → `P2P_GENERATION_FENCED` "The host offer has expired or was replaced.".
5. `accept` with the policy disabled → `Recovery::prepare(P2P_DISABLED, true)`, then `P2P_DISABLED` "Player-hosted
   hunts are disabled.".
6. Emergency stop → `prepare(OPERATOR_P2P_STOP, true)`, then `P2P_DISABLED` "Player-hosted hunts are stopped by
   operations policy.".
7. One transaction (actor: the account): the offer OFFERED → ACCEPTED or DECLINED with `reason` (a string of 1-96
   units, optional); nothing moved → `P2P_GENERATION_FENCED` "The host offer was already answered."; the session must
   have the offer's generation, this offer id and be HOST_OFFERED, else `P2P_GENERATION_FENCED` "The host offer no
   longer owns this generation.". Accept: `offers_accepted + 1`; queue `HUNT_BOOTSTRAP` to the account, payload
   `{ sessionId, generation, role: "HOST" }`, life `startup_lifetime_ms`. Decline: `offers_declined + 1`; reset to
   selection (nothing → `P2P_GENERATION_FENCED` "The host generation changed."); transition with the reason or
   `HOST_OFFER_DECLINED` (HOST, the account).
8. After a decline, `begin_host_selection` again. Answer with `accepted` = the request's `accept`.

### 2.14 Host capabilities (`capabilities`)

#### 2.14.1 The secret

- A capability secret is the unpadded base64url (alphabet `A-Z a-z 0-9 - _`) of 32 random bytes: exactly 43
  characters. `is_opaque_capability(raw)` = length 43 and every byte in that alphabet.
- Only its digest is stored: `hash_capability_secret(secret) -> Option<String>` = the lower-case hex SHA-256 of the
  secret's text, `None` unless it is opaque. Vector: 43 × `a` →
  `66d34fba71f8f450f7e45598853e53bfc23bbd129027cbb131a2f4ffd7878cd0`.
- `generate_capability_secret() -> (Secret, String)`: the secret (rifts-seam `Secret`, never logged) and its digest.
- The hosted game server and its transport send it in the header `x-mystic-host-capability` (rifts-seam's
  `HOST_CAPABILITY_HEADER`).

#### 2.14.2 Minting, checking, revoking

- `mint(hunt, bridge_instance_id, scopes, issued_at, expires_at) -> (Secret, HostCapabilityRecord)`: a new uuid id,
  the digest, the hunt's session id, generation, host account and host peer id, the given bridge instance and scopes,
  not revoked, `last_sequence` 0; stored with the host account as actor. The secret goes to the host's launcher once.
- `check(header, session_id, scope, now) -> HostCapabilityRecord` (`RequireHostCapability`), every refusal
  `P2P_CAPABILITY_INVALID`: no header or not opaque → "A valid hunt host capability is required."; the lookup fails →
  "The hunt host capability could not be verified."; the route's session id is taken after its first `:` when it has
  one; valid when found, not revoked, `expires_at > now`, its session id equals the given one or its seam spelling
  does, and it holds the scope; otherwise "The hunt host capability is invalid or expired.".
- `revoke(session_id, generation, at)`: revoke every live capability of the generation (actor `realms`).
- `hub_present_accounts(session_id, generation, now)`: the ADMITTED accounts of a hub generation through the cache of
  §2.14.5.

#### 2.14.3 The bearer parser and per-player identity

- `parse_optional_bearer(Option<&str>) -> Result<Option<&str>, &'static str>` (`ParseOptionalHostRuntimeBearerToken`):
  none → `Ok(None)`. Split at the first space: no space, a scheme other than `bearer` ignoring ASCII case, an empty
  token or a token holding whitespace → `Err("malformed Bearer authorization")`. `Bearer abc` and `bearer abc` pass;
  `abc`, `Bearer`, `Bearer `, `Basic abc`, `Bearer a b`, `Bearer  abc` (two spaces) fail.
- `request_identity(params: &[(String, String)], path: &str, body: Option<&Value>) -> Option<String>`
  (`ResolvePlayerHostRuntimeRequestIdentity`): the first of the params `userId`, `accountId`, `phxAccountId`, in that
  order, that is non-blank after JS-trim, trimmed; else, when the lower-cased path is exactly `/inventory`,
  `/inventory/instanceditem` or `/character` and the body is an object whose `accountId` is a string non-blank after
  trim, that trimmed; else `None`.

#### 2.14.4 Runtime resolution

- `RuntimeContext { capability: HostCapabilityRecord, hunt: HuntSession, present: Option<Vec<String>> }` (`present`
  only for a hub). `RuntimeIdentity { user_id, roster: Vec<String> }`. `SignedUser<'a>`: `Absent`, `User(&'a str)`,
  `Missing` (a JWT without `userId`, refused).
- `resolve_runtime(header, now) -> Result<Option<RuntimeContext>, RealmsError>` (`ResolveActiveHostRuntimeContext`):
  `None` unless the header is opaque, the capability exists, is not revoked, has not expired and holds
  `GAME_SERVER_RUNTIME`, and its session exists with the capability's exact generation, host account and host peer
  id. A hub must be in `HUB_RUNTIME_STATES` (then `present` = its ADMITTED accounts, §2.14.5); a hunt in STARTING,
  HOST_READY, ADMITTING, RUNNING or DRAINING (a drained hunt's server keeps serving its players).
- `runtime_identity(context, signed, requested) -> Option<RuntimeIdentity>` (`BuildPlayerHostRuntimeAuthData`):
  `Missing` → `None`. The user: the signed one, else the requested account JS-trimmed, else the host. A hub: roster =
  the host then the present accounts, without repeats; `None` unless the user is in it. A hunt: roster = the roster's
  accounts in order; a user not rostered → `Some` only when the hunt says the host does not participate
  (`host_participates == Some(false)`) and the user is the host (roster plus the host; its writes stay refused
  elsewhere); a rostered user who is not the host and not ADMITTED → `None`; otherwise `Some`. The host of a hunt
  always resolves.
- `runtime_auth(context, identity) -> Option<RuntimeAuth>`: `capability_id`, `session` (the seam spelling; `None` when
  it does not parse), `generation` (as u64), `host_account`, `scopes`, `roster` (the identity's), `host_peer` (the
  host peer id when it parses as u64), `bridge_instance: Some`, `wire_session: Some`.

#### 2.14.5 Hub presence cache

`HubPresenceCache` (Debug, Default): key (session id, generation) → (read at, accounts). A hit needs
`now - read_at < 1000` ms. A store with 512 or more entries first drops those 1 s old or more. `invalidate(session_id)`
drops every generation of that session. A miss reads at most 256 ADMITTED rows and stores them. `HostCapabilities` owns
one behind an `Arc` and exposes it as `hub_presence()`; `HostCapabilities::hunt_sessions(clock)` builds a
`RealmsHuntSessions` that shares it. The route-ready, admission and disconnect paths of §2.16 invalidate it.

#### 2.14.6 `RealmsHuntSessions` (rifts-guard's `HuntSessions` port)

Constructors: `new(db, clock)`, `with_cache(db, Arc<HubPresenceCache>, clock)`, `with_guard_clock(db, Arc<dyn
rifts_guard::Clock>)`, `with_cache_and_guard_clock(db, cache, guard clock)`; `presence_cache()`;
`invalidate_hub_presence(&SessionId)` (both the wire and the seam spelling). Its methods block on the store
(`rifts_guard::block_on_store`); store errors become `StoreError::Database(text)`.

- `hunt(&SessionId) -> Option<HuntView>`: find by the wire spelling, then by the seam text; map to `HuntView { session
  (the seam id, else the stored text parsed, else the asked id), generation (u64, 0 when negative), state (text),
  pre_suspect_state (text), host (`None` when empty), realm (the seam realm id, else the text parsed), hub, roster:
  [{ account, admitted (ADMITTED) }], frozen_characters, completion_mode (rifts-guard `CompletionMode::parse` of
  `guard_completion_mode`) }`. `frozen_characters` come from the hunt's `extra`: `frozenLoadouts` (else
  `frozen_loadouts`), an array of objects with `accountId`/`account_id` and `characterId`/`character_id`, both
  non-blank after trim (others skipped), as `(AccountId, character)` pairs.
- `hub_present_accounts(&SessionId, generation: u64)`: a generation above `i64::MAX` → empty. Through the cache keyed
  by the wire id; on a miss ADMITTED rows by the wire id, else by the seam text (each at most 256).
- `capability_by_digest(digest) -> Option<CapabilityView { capability_id, expires_at, revoked }>`.
- `resolve_capability(digest, session_id, scope, now) -> Option<RuntimeAuth>`: the capability is unrevoked, unexpired
  and has the scope, its session id parses to a seam id, the given session equals its wire or seam spelling, and its
  generation is not negative; the `RuntimeAuth` has an empty roster.

### 2.15 The control channel (`control`)

The launcher's long poll for the messages queued for one account (offers, bootstraps, fences, ICE signals) and their
acknowledgement. Constants: the longest wait 25,000 ms, a poll step of 250 ms, at most 32 messages per answer.
`ControlChannel::new(db, clock)`.

**`ControlMessageView`**, fields in order: `_id` (the message id), `accountId`, `messageType`, `sessionId` (omitted
when none), `generation` (omitted when none), `payload`, `createdAt`, `expiresAt`, `sequence`, `ttlAt` (same as
`expiresAt`), `acknowledgedAt` (omitted when none), `messageId`. `From<&ControlMessageRecord>`.

**`poll(account, after: Option<&str>, wait: Option<&str>) -> PollReply { messages, staleCursor (omitted unless true)
}`** (`PollP2PControl`, query `afterSequence` and `waitMs` as text):

1. `after` absent → 0, else `js_number`; `wait` absent → 25,000, else `js_number` clamped to 0-25,000. Either
   unreadable, or `after` negative, fractional or above 2^53-1 → `P2P_TICKET_INVALID` "The control cursor is
   invalid.". So `-1`, `1.5`, `abc`, `NaN` are refused for the cursor, `abc` for the wait, and `waitMs=-50` waits 0.
2. *Stale cursor*: `after > 0` and above the account's latest sequence (0 if it never had a message): a cursor that
   belongs to another account (launchers to 0.1.72 keep one per PC). It is answered from the start, unacknowledged
   messages only.
3. Until a wall-clock deadline of `wait` ms: list the account's unexpired messages (expiry against the injected
   clock) with a sequence above `after` (above 0 and unacknowledged only, when stale), by sequence, at most 32. Any →
   answer them, with `staleCursor: true` when stale. None → sleep min(250 ms, the time left).
4. At the deadline → `{ messages: [] }` without `staleCursor` (also when stale).

**`ack(account, message_id) -> AckReply { acknowledged: true, messageId }`** (`AcknowledgeP2PControl`): the id must be
36 bytes of hex digits and `-` (any case), else `P2P_TICKET_INVALID` "The control message ID is invalid."; the
repository acknowledges only the account's own message (actor: the account); none → `P2P_SESSION_NOT_FOUND` "Control
message not found.".

**`send(actor, &NewControlMessage) -> i64`**: queue one message and return its sequence.

**The message kinds this half queues** (Node has 12 kinds; `ICE_SIGNAL` and others come from rifts-relay and isles):

| `messageType` | To | Payload (in order) | Life |
| --- | --- | --- | --- |
| `HOST_OFFERED` | the candidate host | `offerId`, `huntDefinitionId`, `partySize`, `expiresAt`, `estimatedMemoryMiB`, `score` | the offer's |
| `HUNT_BOOTSTRAP` | the host that accepted | `sessionId`, `generation`, `role: "HOST"` | `startup_lifetime_ms` |
| `JOIN_BOOTSTRAP` | each non-host roster member at host ready (hunts only) | `sessionId`, `generation`, `role: "JOIN"` | 10 min |
| `FENCE_NOW` | §2.11.3, §2.11.4, §2.12.3 | `reason`, `recoveryAction: "NONE"`, and `cleanEnd` on a clean end or a hub end; only `reason` on a pre-active retry | 10 min (retry 5 min) |
| `HOST_OFFER_CANCELLED` | roster (and an unrostered realm host) on a pre-active failure | `reason`, `recoveryAction: "CANCEL_MATCHMAKING"` | 10 min |
| `REALM_SESSION_LEAVE` | the hub's members when it ends | `sessionId`, `generation`, `reason`, `hubKind`, `realmId` | 10 min |
| `REALM_HUB_JOIN` | each non-host member routed into a hub | `sessionId`, `generation`, `hubKind`, `realmId` | 10 min |

Every message also carries its session id and generation in their own fields.

### 2.16 Session routes and adapters (`hosting`)

#### 2.16.1 Routes this half answers

The route table, the auth envelope and the dispatch are the store half's (`routes.rs`); the behaviour behind each is
here. "Cap(S)" = the route first runs `HostCapabilities::check` with the header `x-mystic-host-capability`, the path's
`:sessionId` and scope S. "Launcher" = a signed-in launcher account, else `AUTH_UNAUTHORIZED` "Sign in required.". A
P2P body is a JSON object; an empty body is `{}`; anything else is `P2P_TICKET_INVALID` "The request body must be a
JSON object.". No per-account rate limit applies to `/launcher/v1/p2p/*` or the lease route.

| Method | Path | Auth | Behaviour |
| --- | --- | --- | --- |
| POST | `/mystic/v1/hunts/:sessionId/host-ready` | Cap(`HOST_READY`) | `mark_host_ready` §2.16.4 |
| POST | `/mystic/v1/hunts/:sessionId/abort` | Cap(`ABORT`) | `abort_hunt` §2.16.6 |
| POST | `/mystic/v1/hunts/:sessionId/end` | Cap(`COMPLETE_REQUEST`), else Cap(`GAME_SERVER_RUNTIME`), else Cap(`ABORT`) (the last refusal is answered) | `finish_hunt` §2.16.6 |
| GET | `/mystic/v1/hunts/:sessionId/status` | launcher (its account), host runtime (its host account), admin (no account), or anonymous with Cap(`LEASE_HEARTBEAT`) (no account); else `AUTH_UNAUTHORIZED` "Sign in required." | `get_hunt_status` §2.16.7 |
| POST | `/mystic/v1/hunts/:sessionId/lease` | Cap(`LEASE_HEARTBEAT`) | `renew` §2.12.2 |
| PATCH | `/admin/v1/operations/p2p-emergency-stop` | admin key, else `AUTH_UNAUTHORIZED` "Admin authentication required." | `set_emergency_stop` §2.16.13 |
| POST | `/launcher/v1/p2p/hunts/:sessionId/host-bootstrap` | launcher | `create_host_bootstrap` §2.16.3 |
| POST | `/launcher/v1/p2p/hunts/:sessionId/route-ready` | launcher | `mark_local_route_ready` §2.16.5 |
| GET | `/launcher/v1/p2p/control` | launcher | `ControlChannel::poll` §2.15 |
| POST | `/launcher/v1/p2p/control/:messageId/ack` | launcher | `ControlChannel::ack` §2.15 |
| POST | `/launcher/v1/p2p/host-presence` | launcher | `submit_presence` §2.13.3 |
| POST | `/launcher/v1/p2p/host-offers/:offerId/respond` | launcher | `respond_to_offer` §2.13.6 |
| POST | `/launcher/v1/p2p/hunts/:sessionId/observation` | launcher | `observe` §2.12.5 |
| POST | `/launcher/v1/realm-hosting/hubs/start` | launcher | `start_realm_hub` §2.16.8 |
| POST | `/launcher/v1/realm-hosting/hubs/stop` | launcher | `stop_realm_hub` §2.16.8 |
| GET | `/launcher/v1/realm-hosting/status` | launcher | `get_realm_hosting_status` §2.16.9 |
| POST | `/api/matchmaker/resolve-hunt-launch-manifest` | (store half) | `resolve_launch_manifest` §2.16.13 |

Every success is 200 with the JSON below (fields in order). Refusal statuses: §2.0.4.

#### 2.16.2 Constants and shared rules

- Defaults: game build 647472, bridge protocol 1, virtual port 1; a participant's rejoin window 180,000 ms; at most 5
  reconnects (ticket reservations); control messages live 10 min.
- `HUNT_CAPABILITY_SCOPES` (a hunt host's capability, in this order): `HOST_READY`, `LEASE_HEARTBEAT`,
  `PEER_ADMITTED`, `EVENTS_WRITE`, `COMPLETE_REQUEST`, `GAME_SERVER_RUNTIME`, `ABORT`. A hub host gets
  `HUB_CAPABILITY_SCOPES` (§2.8.1).
- **Session id from a path**: drop everything up to the first `:`; if the rest parses as a seam `SessionId`
  (`<ULID>@<node>`), use its wire form; else use the rest. Look the session up by that, then (when different) by the
  raw text.
- **Peer id as a number**: the stored peer id text as u64 when it parses and is above 0; else the account's derived
  ICE peer id; else 1.
- **Verified link**: with a runtime wired and a peer id that parses as u64, put a VERIFIED peer link (revision 1,
  verified now, method `ICE_ACCOUNT_DERIVED`) in rifts-admission's store as the account; the result is ignored.
- **Self ticket** (`host_ticket`, for a host that also plays): with a runtime wired and the session id parsing as an
  admission `SessionUuid`, mint a ticket through rifts-admission's `TicketAuthority` with bindings: the account; the
  joiner peer (its peer id as a number); the host peer (the session's host peer id as a number); the session;
  generation (at least 1, as u32); party id with every `@` replaced by `:`; party revision (at least 1); hunt
  definition id; build (at least 1, as u32, else 647472); bridge protocol clamped 1-255; virtual port clamped 1-65535;
  lifetime `JOIN_TICKET_LIFETIME_SECONDS`. Store the ticket record (status ISSUED, issued and expiry = the claims'
  seconds × 1000, key id and digest from the mint) as the account, ignoring the result, and return the ticket text.
  Any failure → no ticket.
- **Candidate** (the matchmaker's view of where an account's game connects): a JSON object `{ HuntId, CandidateId,
  GameSessionId, Host, Port, Ready, ReadyAt: 0 }` plus, for a hub, `Source: "REALM_HUB"` and `SourceSessionId` (the
  hub's session id); serialized to text and handed to `IslesCore::set_candidate(account, ..)` (result ignored).
  `Host` is `127.0.0.1` when ready, else empty; `GameSessionId` is the hub's room id (else its session id) for a hub,
  the session id for a hunt.

#### 2.16.3 Host bootstrap (`create_host_bootstrap`, `CreateHostBootstrap`)

`create_host_bootstrap(account, raw_session_id) -> Value`:

1. Policy disabled → `P2P_DISABLED` "Player-hosted hunts are disabled."; emergency stop → `P2P_EMERGENCY_STOP`
   "Player-hosted hunts are stopped by operations policy."; no runtime wired → `P2P_DISABLED` "Host bootstrap runtime
   is not configured.".
2. Unknown session → `P2P_SESSION_NOT_FOUND` "Hunt session not found."; the account is not the session's host →
   `P2P_NOT_ROSTERED` "Only the selected host may bootstrap this session.".
3. Bridge instance: `bridge-<uuid>` when the session has none or is HOST_OFFERED, else its own. Metadata nonce: the
   session's, else 32 lower-case hex of 16 random bytes. Startup deadline = now + `startup_lifetime_ms`.
4. HOST_OFFERED → one transaction (actor: the account): begin host startup (the repository moves HOST_OFFERED →
   STARTING for this host of an unexpired session, writing the bridge instance, nonce, deadline, and the server UDP
   port = the session's or the runtime's; clearing lobby, lease and suspect fields) and append a transition
   `HOST_BOOTSTRAP_ISSUED` (HOST, the account); nothing moved → `P2P_GENERATION_FENCED` "The session is no longer
   waiting for host startup.". STARTING, HOST_READY, ADMITTING, RUNNING → reuse the session as is (a repeated
   bootstrap). Any other state → `P2P_GENERATION_FENCED` "This session generation cannot be bootstrapped.".
5. The host peer = the session's host peer id, or the account's derived ICE id when empty; ensure a verified link.
6. Mint a capability for the session's bridge instance: hub scopes and expiry now + `capability_ttl_ms` for a hub;
   hunt scopes and the session's `expires_at` for a hunt.
7. A self ticket when `host_participates` (unset = true).
8. Answer the bootstrap JSON (§2.16.10) with `hubAfkSeconds: null`.

#### 2.16.4 Host ready (`mark_host_ready`, `MarkHostReady`)

`mark_host_ready(capability, body) -> Value`:

1. Emergency stop → `P2P_EMERGENCY_STOP` "Player-hosted hunt operations are temporarily stopped.".
2. Unknown session → `P2P_SESSION_NOT_FOUND` "Hunt session not found."; another generation →
   `P2P_GENERATION_FENCED` "The host capability belongs to a stale generation.".
3. Already HOST_READY, ADMITTING or RUNNING → answer at once (idempotent): `{ ready: true, sessionId, generation,
   state, leaseExpiresAt }` with the stored lease (or now + lease lifetime when none).
4. `lobbyId` (1-64 units, no NUL) or `ICE`; `virtualPort` integer 1-65535 or the session's.
5. One transaction (actor: the host): the repository's host-ready mark (requires the capability's generation, host
   peer id and bridge instance, the session's virtual port equal to the given one, and STARTING or HOST_READY; sets
   HOST_READY, the lobby, `lease_expires_at` = now + `lease_lifetime_ms`, `last_heartbeat_at` now,
   `last_lease_sequence` 0); nothing → `P2P_GENERATION_FENCED` "The hunt generation is not waiting for host
   readiness.". When the previous state was not HOST_READY: transition `HOST_READY` (HOST, the host);
   `starts_succeeded + 1`; for a hunt (not a hub), queue `JOIN_BOOTSTRAP` to each roster account other than the host,
   payload `{ sessionId, generation, role: "JOIN" }`, life 10 min.
6. A hub → mirror it as `STARTING`.
7. Answer `{ ready: true, sessionId, generation, state, leaseExpiresAt }` (the new lease).

#### 2.16.5 Local route ready (`mark_local_route_ready`, `MarkLocalRouteReady`)

`mark_local_route_ready(account, raw_session_id, body) -> Value`:

1. `localBridgePort` (else `localPort`) integer 1-65535, else `P2P_TICKET_INVALID` "localBridgePort must be between 1
   and 65535.".
2. Unknown session → `P2P_SESSION_NOT_FOUND` "Hunt session not found."; a JSON-integer `generation` that differs →
   `P2P_GENERATION_FENCED` "The route-ready report belongs to a stale generation.".
3. **A hub**: no realm → `P2P_SESSION_NOT_FOUND` "Hub session has no realm.".
   - The host: one transaction (actor: the host): CAS from STARTING, HOST_READY, ADMITTING or RUNNING to RUNNING with
     `host_local_bridge_port` = the port, `host_local_bridge_ready_at` now, `lobby_id` `ICE` when unset, and when no
     lease is set: lease now + lease lifetime, `last_heartbeat_at` now, `last_lease_sequence` 0; a transition
     `HUB_HOST_ROUTE_READY` (HOST, the host) when it was not RUNNING. Nothing → `P2P_GENERATION_FENCED` "Hub session is
     not active.". Then the hub repository's `admit_host` (the host's own row ADMITTED with its peer id = the host peer
     id or its derived ICE id, and the port), invalidate the hub presence cache for the session, mirror `RUNNING`,
     set the host's candidate (`CandidateId` = `cand-<session>-<account>`, `Ready: true`, `Port`), and answer
     `{ ready: true, sessionId, generation, localBridgePort, state }`.
   - A member: the hub repository's `mark_route_ready` (the member's waiting row not expired, or a connected one, gets
     the port and ready time); none → `P2P_NOT_ROSTERED` "This account has no active entry into the realm hub.";
     invalidate the cache; set the candidate; answer the same shape with the hub's state.
4. **A hunt**: when the account is the host and the hunt is STARTING, first run host ready (§2.16.4) with a synthetic
   capability of the session's own generation, host, peer and bridge instance and an empty body (a host whose
   transport reports route-ready while still STARTING), then re-read the session. Not rostered → `P2P_NOT_ROSTERED`
   "This account is not in the hunt roster.". Bridge instance = `bridgeInstanceId` (1-96 units), else the
   participant's, else `local-<uuid>`. The repository marks the participant's route ready (matched by account and its
   peer id, in a live roster state, no version bump); none → `P2P_GENERATION_FENCED` "The hunt generation does not
   accept route updates.". Set the candidate (`CandidateId` = the participant's); answer `{ ready: true, sessionId,
   generation, localBridgePort, state }`.

#### 2.16.6 Abort and end

- `abort_hunt(capability, body)` (`AbortHunt`): reason = `reason` (1-128 units, no NUL) or `HOST_ABORTED`; unknown
  session → `P2P_SESSION_NOT_FOUND` "Hunt session not found."; another generation → `P2P_GENERATION_FENCED` "Stale hunt
  generation.". A hub → `end_hub(reason, ABORTED, HOST, host)`; a hunt → `end_hunt(reason, allowed STARTING,
  HOST_READY, ADMITTING, RUNNING, DRAINING, SUSPECT, HOST, host, fence)`. Answer `{ aborted: true, sessionId,
  generation, state }`, `state` = the ended session's or, when nothing ended, the state read.
- `finish_hunt(capability, body)` (`EndHunt`): reason default `HOST_COMPLETED`; same checks; a hub →
  `end_hub(reason, COMPLETED, HOST, host)`; a hunt → `end_hunt(reason, HOST_END_STATES, HOST, host, fence)`. Answer
  `{ ended: true, sessionId, generation, state }`.

#### 2.16.7 Status (`get_hunt_status`, `GetHuntStatus`)

`get_hunt_status(caller: Option<&str>, raw_session_id) -> Value`: unknown → `P2P_SESSION_NOT_FOUND` "Hunt session not
found."; with a caller, it must be the host, rostered, or (a hub) hold a hub presence row, else `P2P_NOT_ROSTERED` "You
are not part of this session.". Answer `{ sessionId, generation, state, hostAccountId, partyId, huntDefinitionId,
completionReason (null), failureReason (null) }`.

#### 2.16.8 Realm hubs (`start_realm_hub`, `stop_realm_hub`)

**`start_realm_hub(account, body) -> Value`** (`CreateRealmHubSession`):

1. Realm hosting or the policy off → `REALM_HOSTING_DISABLED` "Realm hosting is not enabled on this server.";
   emergency stop → `P2P_EMERGENCY_STOP` "Player-hosted operations are temporarily stopped."; no runtime →
   `REALM_HOSTING_DISABLED` "Realm hosting runtime is not configured.".
2. `parse_hub_start_body`, its message under `REALM_VALIDATION_FAILED`. `afkSeconds` (else `hubAfkSeconds`) an integer
   0-7200, else 0.
3. The account must host a realm, else `REALM_NOT_FOUND` "You do not host an active realm.".
4. A live hub of that kind (by party key) → `end_hub(SUPERSEDED_BY_HOST, COMPLETED, HOST, account)` first.
5. `active` = `count_active_realm_servers(realm)`; `max` = the realm's hosting `max_servers` or 6; `active >= max` →
   `REALM_SERVER_LIMIT` "Your realm has reached its active server limit.".
6. With a fresh presence of the account (observed since now - max age, unexpired):
   `realm_host_ineligibility_reason(presence, policy, 0, 0, guard_refuses(), now)` → `P2P_HOST_INELIGIBLE` "Host
   presence is ineligible (<reason>)."; capacity (§2.8.6, the realm hub floors) `ServerLimit` or `CommitLow` →
   `REALM_CAPACITY_EXCEEDED` "Insufficient host capacity to start another realm server.". Without a fresh presence the
   start goes ahead unchecked.
7. Create the session (actor: the account): a new ULID wire id; generation 1; STARTING; state version 1; party id and
   active party key = the hub party key; party revision 1; the manifest's hunt definition id and game mode; build
   647472, bridge protocol 1, virtual port 1; the host account and its derived ICE peer id (a verified link ensured);
   bridge instance `bridge-<uuid>`; empty roster; lobby and co-op transport `ICE`; lease now + lease lifetime;
   `created_at` now; `expires_at` now + 7 d; a random 32-hex nonce; the hub kind, realm id, `host_participates` true;
   server UDP port = the runtime's port, +1 for Training; hub room id (§2.4.8); `max_peers` = maxPlayers; the
   manifest's completion mode; startup deadline now + startup lifetime; `last_heartbeat_at` now,
   `last_lease_sequence` 0; `tried_host_account_ids` = [the account]; trust tier = the policy's; everything else unset.
8. Mirror `STARTING`. Mint a capability with the hub scopes, expiry now + `capability_ttl_ms`; a self ticket.
9. Answer the bootstrap JSON (§2.16.10) with `hubAfkSeconds` = the afk value.

**`stop_realm_hub(account, body) -> Value`** (`StopRealmHubSession`): `parse_hub_stop_body` (`REALM_VALIDATION_FAILED`);
no hosted realm → `REALM_NOT_FOUND` "You do not host an active realm."; no live hub of that kind → `{ stopped: false,
hubKind, sessionId: null }`; else `end_hub(HOST_STOPPED_HUB, COMPLETED, HOST, account)` → `{ stopped: <ended>, hubKind,
sessionId }`.

#### 2.16.9 Hosting status (`get_realm_hosting_status`, `GetRealmHostingStatus`)

`enabled` = realm hosting and policy enabled. No hosted realm → `{ enabled, realmId: null, realmName: null, hosting:
null, ramsgate: null, training: null, activeHunts: [] }`. Else `{ enabled, realmId, realmName, hosting, ramsgate,
training, activeHunts }` where:

- `hosting` = null or `{ maxServers, maxPlayers, afkSeconds (null), hubSessionId (null), hubState, trainingState,
  activeServers, updatedAt }`;
- `ramsgate`/`training` = null, or for the live hub of that kind `{ sessionId, generation, state, admittedPlayers
  (count of ADMITTED rows of the generation), maxPlayers (`max_peers` or 16) }`;
- `activeHunts` = the realm's live hunts hosted by the account (never hubs, at most 64) as `{ sessionId, generation,
  state, huntDefinitionId, partySize (roster length) }`.

#### 2.16.10 The bootstrap JSON

Fields in order: `keyId` (the ticket authority's key id; `local-key-1` without a runtime), `sessionId`, `partyId` (`@`
→ `:`), `partyRevision` (≥ 1), `huntDefinitionId`, `generation` (≥ 1), `build` (≥ 1), `bridgeProtocol` (clamped
1-255), `virtualPort` (clamped 1-65535), `hostPeerId` (the host peer id, or the host's derived ICE id when empty),
`hostSteamId` (the same), `bridgeInstanceId`, `metadataNonce` (the session's or `0123456789abcdef0123456789abcdef`),
`forceRelay` (`policy.force_sdr`), `guardCompletionMode` (the session's, else the manifest's), `sessionRole` (`HUB` or
`HUNT`), `hubKind` (text or null), `capability` (the 43-character secret), `hostTicket` (text or null), `backendUrl`
(the runtime's or `http://127.0.0.1:8080`), `serverUdpPort` (the session's if it fits u16, else the runtime's, else
8791), `maximumPeers` (`max_peers` clamped 1-64, else 16 for a hub, 4 for a hunt), `hostLocalClient`
(`host_participates`, unset = true), `huntEvidence` (not a hub), `matchmakerHuntId`, `mapPath`, `behemothPath`,
`expectedPlayers`, `expectedPlayerString` (the same), `runtimeProfile: "production"`, `maxPlayers` (`maximumPeers` for
a hub, null for a hunt), `hubAfkSeconds`. The four manifest fields come from `resolve_manifest_for_hunt(hunt definition
id, roster account ids)`.

`resolve_manifest_for_hunt(hunt_definition_id, expected_players) -> Value` (`pub`): an id containing
`ReturnToRamsgate` or equal to `CITY` ignoring case → the Ramsgate manifest; an id containing `TrainingDojo` → the
Training manifest; each as `{ huntDefinitionId, gameMode, mapPath, behemothPath, matchmakerHuntId,
expectedPlayerString, guardCompletionMode }`. Any other id → `{ huntDefinitionId: id, gameMode: "ISLAND", mapPath:
"/Game/Maps/islands/1705/dia_moss_triforce", behemothPath: "Behemoth_Gnasher", matchmakerHuntId: id,
expectedPlayerString: <players joined by "," or "1" when none>, guardCompletionMode: "FIRST_DEFEAT" }` (a placeholder,
§8 R7).

#### 2.16.11 Matchmaking (crate-visible, called by the service)

- **`create_routed_hunt(party_id, party_revision, hunt_definition_id, game_mode, members, realm_id) -> HuntSession`**
  (`pub`): a new ULID wire id; generation 1; HOST_SELECTING; state version 1; party id and active party key = the
  party id; revision ≥ 1; the hunt id and mode; build 647472, protocol 1, port 1; host account = the first member and
  its derived ICE id as host peer (empty when no members); empty bridge instance; roster = each member PENDING with its
  derived ICE id (a verified link ensured) and candidate `cand-<session>-<account>`; co-op transport `ICE`;
  `created_at` now, `expires_at` now + 2 h; a random 32-hex nonce; the realm id; `host_participates` true; server UDP
  port = the runtime's when wired; `max_peers` 4; completion mode `FIRST_DEFEAT`; trust tier = the policy's;
  everything else unset. Create it (actor: the first member, else `realms-route`), `begin_host_selection`, and answer
  the stored session.
- **`restore_matchmaking_candidate(player)`**: the player's newest active hunt (never a hub) where it is rostered →
  its candidate (`CandidateId` the participant's, `GameSessionId` the session, ready when the participant has a port
  and ready time, `Port` the port or 0). Else its newest live hub row whose hub is live → the hub candidate (`Source`,
  `SourceSessionId`). Else `None`.
- **`poll_candidate_status(player, current)`**: the current candidate text must parse as JSON, else `None`. With
  `Source: "REALM_HUB"`: its `SourceSessionId` session must exist; a COMPLETED or ABORTED hub → the candidate with
  `Ready: false`, `Canceled: true`, `StatusReason` = the hub's failure reason, else completion reason, else `HUB_ENDED`;
  else when the player's hub row has a port and ready time → `Ready: true`, `Host: "127.0.0.1"`, `Port`; else the
  candidate unchanged. Otherwise its `GameSessionId` session must exist; ABORTED → `Ready: false`, `Canceled: true`,
  `StatusReason` = failure reason or `HUNT_ABORTED`; else when the participant has a port and ready time →
  `Ready`/`Host`/`Port` set; else unchanged. (Keys are updated in place; new keys go at the end.)
- **`route_matchmaking_hunt(context, realm_id) -> HuntRouteOutcome`**: no runtime → `NotHandled`. Members = the party
  members, else the player alone.
  - With a realm and a hub request (§2.8.1): no live hub of that kind → `IslesCore::cancel_travel(hunt id, members,
    "REALM_HUB_NOT_RUNNING")` and `Handled { result: true }`. Else the game session id = the hub's room id (else
    computed). For each member other than the host: a verified link and an entry request (`peer` = derived ICE id,
    candidate `cand-<hub session>-<account>`); the hub repository's `request_entries` with the hub's `max_peers` (16
    when unset) and expiry now + 90 s (actor: the requesting player). `Full` → `cancel_travel(.., "REALM_HUB_FULL")`
    and handled. `Placed` → for each placed row, set its candidate (hub form) and queue `REALM_HUB_JOIN` (payload
    `{ sessionId, generation, hubKind, realmId }`, life 10 min, actor: that account; failures ignored). If the host is
    among the members, set its candidate with `CandidateId` `host:<hub session>`, ready when its port is above 0 and
    a ready time is set. `Handled { result: true }`.
  - Otherwise decide whether to host: with a realm, policy enabled, no emergency stop and not `context.is_hub`;
    without, `decide_hunt_route` (private party assumed, size = member count, implemented = true) is `PlayerHosted`.
    No → `NotHandled`. Yes → party key = the context's party id text, else `solo:<player>`; reuse the live session of
    that party key, else `create_routed_hunt(party key, revision (≥ 1; 1 when absent), hunt id, mode, members,
    realm)`; set every roster member's candidate (hunt form); `Handled { result: true }`.

#### 2.16.12 Trait adapters for rifts-admission and rifts-relay

All block on the store; store errors become the crate's `Store(Database(text))` error.

- Mapping a session: `HuntSnapshot { wire_session, generation (≥ 1, u64), state (same names), kind (`RealmHub` for a
  hub, else `Hunt`), realm, coop_transport (parsed, else ICE), host_account, host_peer (as a number), expires_at,
  roster [{ account, peer (as a number), returned_to_hub }] }`. `AdmissionHunt` adds the `SessionUuid` (no admission
  hunt when the id does not parse), party id (`@` → `:`), party revision (≥ 1), hunt id, build (≥ 1 as u32, else
  647472), protocol (1-255), virtual port (1-65535), `lobby_id` (the session's, else `ICE` when STARTING, HOST_READY,
  ADMITTING or RUNNING) and `bridge_instance_id` (`None` when empty).
- **`AdmissionHunts`**:
  - `gate()` → `P2PGate { enabled: policy.enabled, emergency_stop }`.
  - `find_hunt(session)` by the wire spelling.
  - `reserve_participant(hunt, account, now)` → the repository's ticket reservation (a rostered participant in a live
    roster state, under 5 reconnects, inside its rejoin window or never disconnected: `reconnect_epoch + 1`, ISSUED,
    rejoin deadline now + 180 s); the updated hunt or `None`.
  - `check_hub_entry(hub, account, link, now) -> PeerId`: the hub must exist (`P2P_SESSION_NOT_FOUND` "Hub session not
    found.") and have a realm (`P2P_SESSION_NOT_READY` "The realm's hub is not accepting players."). Banned = in the
    realm's bans; member = the realm's host or in its members. Without a link on an ICE hub, ensure a verified link
    for the derived id. The effective peer = the link's peer, else the derived ICE id; the link is then treated as
    VERIFIED with that peer. When the account's row is missing, not waiting, or expired, and it is a member, not
    banned, not the host, and the hub is live: request an entry (expiry now + 90 s, `max_peers` or 16); `Full` →
    `REALM_HUB_FULL` "The realm's hub is full.". Then the ticket check (§2.8.5) on the hub with STARTING, HOST_READY or
    ADMITTING read as RUNNING, the hub generation given; a denial → its code and message. Answer the effective peer as
    a number.
  - `mark_hub_ticketed(hub, account, now, ticket_expires_at)` → the hub repository's ticketed mark; true when a row
    changed.
  - `mark_participant_admitted(hunt, account, at)`: one transaction: the participant ADMITTED (admitted at, disconnect
    and rejoin cleared); if the hunt is HOST_READY or ADMITTING, target = RUNNING when every roster entry is ADMITTED,
    else ADMITTING; when different, CAS from HOST_READY/ADMITTING and transition `ALL_PARTICIPANTS_ADMITTED` or
    `FIRST_PARTICIPANT_ADMITTED` (HOST, the account). True when the participant was marked.
  - `admit_hub_peer(hub, account, admission_id, at)`: the host → nothing; else the hub repository's admitted mark
    (route `ice`), then invalidate the hub presence cache.
  - `peer_disconnected(admission, at)`: a hub → its row LOST (only for the current admission id), purge at + 7 d,
    invalidate the cache; a hunt → the participant LOST with `disconnected_at` and a rejoin deadline at + 180 s (live
    roster states HOST_READY, ADMITTING, RUNNING, DRAINING, SUSPECT). Unknown session → nothing.
  - `hub_presence(hub, limit)` → live rows (at most min(limit, 256)) as `PresenceRow { peer, account, state }`.
  - `join_bootstrap_common(hunt, key_id, bridge_instance_id) -> Option<Json>`: `{ keyId, sessionId, partyId,
    partyRevision, huntDefinitionId, generation, build, bridgeProtocol, virtualPort, hostPeerId, hostSteamId,
    bridgeInstanceId, metadataNonce, forceRelay, guardCompletionMode, sessionRole }` with the admission hunt's values,
    the stored nonce (or the 32-hex placeholder) and completion mode (or `NO_COMPLETION` for a hub, `FIRST_DEFEAT`).
- **`IceHunts`**: `gate()`, `find_hunt` (snapshot), `hub_presence` as above; `enqueue(ControlMessage)`: the payload
  text must parse as JSON, else `P2P_SIGNAL_INVALID` with the parser's message; queue it with its account, type, wire
  session and generation (as the account).
- **`OperationsPolicy::set_coop_transport(admin_key_id, transport, expected_version, reason, request_id)`**: an
  `expected_version` above 1 → `Conflict` with `{ version: 1, p2pEmergencyStop, coopTransport ("ICE" when
  `ice_transport()`, else "STEAM_EXPERIMENTAL"), updatedAt }`; else `Updated` with `{ version: expected + 1,
  p2pEmergencyStop, coopTransport: the requested one, updatedAt, updatedBy, reason, requestId }`. Nothing is stored
  (§8, R5).

#### 2.16.13 `RealmEventSink` and `HuntLifecycle`

- **`RealmEventSink::emit`** (Hosting receives the directory's events): `MemberRemoved` → every live hub row of that
  account in that realm becomes REMOVED (purge at now + 7 d; actor `realms-directory`), info `[Realms] removed account
  <account> from realm <realm> hub presence (<Kick|Ban|Leave>)`; `RealmReleased` → every live row of the realm
  REMOVED (as the host), then each live hub of the realm ends `REALM_RELEASED`, COMPLETED, HOST, the host;
  `HostOffline` → each live hub ends `REALM_HOST_OFFLINE`, ABORTED, SYSTEM, the host. Hub-end errors are ignored; a
  presence error is the sink's error text.
- **`HuntLifecycle`** (a typed facade over the same calls, with seam ids):
  - `host_bootstrap(account, session) -> HostBootstrap { session, generation (u64, default 1), capability: Secret,
    host_ticket: Option<Secret>, backend_url, server_udp_port (default 8791), maximum_peers (default 4) }` from the
    bootstrap JSON;
  - `route_ready(account, session, port)` → route ready with `{ localBridgePort }`;
  - `host_ready(runtime)`, `abort(runtime, reason)`, `end(runtime, reason)` → the routes' behaviour with a capability
    built from the `RuntimeAuth` (its wire session or the seam id's wire form, generation, host, peer, bridge
    instance, scopes);
  - `status(account, session) -> HuntStatus { state, generation, verdict: None }` (an unparsable state reads RUNNING);
  - `observe(account, session, Observation { host_reachable, local_bridge_alive, route_kind: Option<String>, ping_ms,
    jitter_ms (u32), packet_loss_percent (u8), last_packet_age_ms (u64) })` → `observe` with `generation: 1`,
    `observedAt` now, `metricsAvailable` = a route kind given, `routeKind` (or `UNKNOWN`) and the metrics (null when
    none) (§8, R10);
  - `start_hub(account, StartHub { kind, max_players: u8, afk_seconds: u32 })` → `start_realm_hub` with `{ hubKind,
    maxPlayers, afkSeconds }`, the session id converted to the runtime's node (`local` without a runtime; an invalid
    one → refusal `INTERNAL` "Invalid node id." / "Invalid hub session id."), `maximum_peers` default `max_players`;
  - `stop_hub(account, kind)`; `hosting_status(account) -> Json`;
  - `set_emergency_stop(admin_key, policy: Json)`: the body must parse (`ADMIN_VALIDATION` "Invalid emergency stop
    payload."); `p2pEmergencyStop` boolean (`ADMIN_VALIDATION` "p2pEmergencyStop must be a boolean."); `expectedVersion`
    u64 default 1; `reason` trimmed of at least 8 bytes (`ADMIN_VALIDATION` "An audit reason of at least 8 characters
    is required."). Answer `{ policy: { version: expected + 1, p2pEmergencyStop, coopTransport, updatedAt, updatedBy,
    reason }, requestId: <uuid> }`. Nothing is stored (§8, R5);
  - `resolve_launch_manifest(request: Json)`: must parse, else `P2P_TICKET_INVALID` "Invalid launch manifest
    request."; `huntDefinitionId` non-blank after trim, else `P2P_TICKET_INVALID` "huntDefinitionId is required.";
    `expectedPlayers` the string entries of an array (else none); answer `resolve_manifest_for_hunt`.

---

## 3. Repository methods this half calls

The store half defines the tables and these methods (names and semantics must agree; its spec has the SQL). Every
write takes an `&Actor` first unless noted; times are `UnixMillis`; errors are the store half's `RepoError`, which this
half turns into `RealmsError::Database(text)`.

### 3.1 `RealmsRepository` (realms, members, bans, requests, invites)

| Method | Semantics relied on |
| --- | --- |
| `new(RealmsDb)` | |
| `find_by_id(id)`, `find_by_name(name)`, `find_by_host(host)` | one realm with its children, or none |
| `find_by_member(account, limit)` | realms the account is in (its own included), by name |
| `find_by_player_invite(account, limit)` | realms holding a player invite for the account, by name |
| `list_listed(limit, offset)` | listed realms, `online_until` descending with absent last, then by name |
| `insert_realm(realm) -> InsertRealmResult { Inserted, NameTaken, HostHasRealm }` | name and host are each unique |
| `delete_realm_if_last_online(realm_id, expected_last_online_at) -> bool` | deletes (with its invites) only if not online since the value read |
| `delete_realm_of_host(realm_id, host) -> bool` | deletes the host's realm with its invites |
| `set_settings(realm_id, host, Option<JoinPolicy>, Option<bool>) -> bool` | nothing to change answers true without looking |
| `record_heartbeat(realm_id, host, &HeartbeatUpdate) -> bool` | no version keeps the stored one |
| `patch_hosting(realm_id, &HostingPatch, at) -> bool` | `apply_hosting_patch` on the stored hosting (defaults when absent); false when the realm is gone |
| `mark_offline(realm_id, at) -> bool` | when online after `at`: `online_until = at` and `offline_notified_for = at` (so the sweeper never announces it) |
| `claim_lapsed_hosts(now, since, limit) -> Vec<RealmRecord>` | realms with `online_until` in `(since, now]` not yet notified for that value; marks each claimed (one notice per lapse) |
| `find_by_hub_room_id(room)` | the realm whose `hub_room_ids` holds it (lower-case form) |
| `add_member(realm_id, &RealmMember, max) -> AddMemberResult { Added, AlreadyMember, Banned, Full, NotFound }` | refuses banned and full; an `Added` drops the account's player invite in the same write |
| `remove_member(realm_id, account) -> bool` | a plain member only, never the host |
| `add_ban(realm_id, &RealmBan, max) -> AddBanResult { Banned, AlreadyBanned, Full, NotFound }` | also removes the membership (never the host's), join request and player invite |
| `remove_ban(realm_id, account) -> bool` | |
| `add_join_request(realm_id, &AccountAt, max) -> AddEntryResult` | checks in order: missing realm `NotFound`, `Banned`, `AlreadyMember`, `Already`, `Full`, then `Added` |
| `remove_join_request(realm_id, account) -> bool` | |
| `add_player_invite(realm_id, &AccountAt, max) -> AddEntryResult` | same order as join requests |
| `remove_player_invite(realm_id, account) -> bool` | |
| `insert_invite(&RealmInviteRecord) -> InsertInviteResult { Inserted, Duplicate }` | the code is unique |
| `find_invite(code)`, `list_invites(realm_id, limit)` (newest first) | |
| `count_active_invites(realm_id, now) -> i64` | not revoked, not expired, not used up |
| `consume_invite(code, now) -> Option<RealmInviteRecord>` | one more use only if usable at `now` (atomic: two joins cannot share the last use) |
| `refund_invite(code)` | one use back, never below zero |
| `revoke_invite(realm_id, code, now) -> Option<RealmInviteRecord>` | revoked once (the first time stays); `None` when not this realm's |

### 3.2 `RealmPresenceRepository` (one row per account)

`new`; `bind(&RealmPresenceRecord)` (replaces); `find_active(account, now)` (only while `now < expires_at`);
`touch(account, last_seen_at, expires_at) -> Option<record>` (extends a row still active; never brings one back);
`remove(account, Option<realm_id>) -> bool` (with a realm, only a row bound to it); `remove_for_realm(realm_id) ->
usize`; `count_active_by_realm(&[realm_id], now) -> BTreeMap<realm_id, i64>` (realms with none absent).

### 3.3 `HuntRepository` and its transaction `HuntTx`

`HuntRepository::new(db)`; `transaction(actor, f)` runs `f(&mut HuntTx)` as one write transaction and rolls every step
back on `Err` (a refusal included; the closure's error type converts from `RepoError`).

| On `HuntTx` | Semantics |
| --- | --- |
| `find_hunt(id)` | the session with its roster |
| `compare_and_set(id, generation, &[states], state_version, change) -> Option<HuntSession>` | applies `change` only while generation, one of the states and the version hold; bumps the version |
| `append_transition(&HuntTransition)` | an empty `transition_id` gets a fresh one |
| `revoke_generation(id, generation, at)` | revokes every live capability of the generation |
| `update_offer(offer_id, account, expected: OfferStatus, to: OfferStatus, at, reason: Option<&str>) -> Option<offer>` | moves that account's offer only from `expected` |
| `create_offer(&HostOfferRecord)` | one accepted offer per generation |
| `enqueue_control(&NewControlMessage) -> ControlMessageRecord` | gives an id and the account's next sequence (sequences survive purges) |
| `increment_reliability(account, ReliabilityIncrements, at, cooldown: Option<UnixMillis>)` | adds (a new row starts at zero); a cooldown given replaces the old |
| `advance_capability_sequence(capability_id, sequence) -> bool` | strictly newer sequence on an unrevoked capability |
| `renew_host_lease(&LeaseRenewal { session_id, generation, host_steam_id, bridge_instance_id, lease_sequence, heartbeat_at, lease_expires_at }) -> Option<HuntSession>` | §2.12.2 conditions; writes sequence, heartbeat time and lease; bumps the version |
| `begin_host_startup(&HostStartup { session_id, host_account_id, bridge_instance_id, metadata_nonce, startup_deadline, server_udp_port: Option }, now) -> Option` | HOST_OFFERED → STARTING for that host of an unexpired session (§2.16.3) |
| `mark_host_ready(&HostReadyMark { session_id, generation, host_steam_id, bridge_instance_id, lobby_id, virtual_port, lease_expires_at }, now) -> Option` | §2.16.4 |
| `mark_participant_admitted(id, generation, account, at) -> Option` | in a live roster state; bumps the version |
| `end_hub_presence(id, generation, now, purge_at) -> Vec<HubPresenceRecord>` | every live row of the generation → LOST; the rows as they were |
| `record_observation(&ObservationRecord)` | |

| On `HuntRepository` | Semantics |
| --- | --- |
| `find_hunt(id)` | |
| `find_active_hunt_for_party(party_key)` | by `active_party_key` |
| `find_active_hunt_for_account(account)` | the account's newest active hunt, never a hub |
| `create_hunt(actor, &HuntSession)` | session id and active party key unique |
| `list_lifecycle_hunts(&[states], limit)` | oldest first |
| `mark_participant_route_ready(actor, id, generation, account, steam_id, bridge_instance, port, at) -> Option` | matched by account and peer id, live roster state, no version bump |
| `reserve_participant_ticket(actor, id, generation, account, now, rejoin_deadline, max_reconnects) -> Option` | §2.16.12 |
| `mark_participant_disconnected(actor, id, generation, account, at, rejoin_deadline) -> Option` | §2.16.12 |
| `create_host_capability(actor, &record)`, `find_host_capability_by_digest(digest)` | |
| `revoke_generation(actor, id, generation, at)` | |
| `enqueue_control_message(actor, &msg) -> ControlMessageRecord` | |
| `list_control_messages(account, after, now, limit, unacknowledged_only) -> Vec<record>` | unexpired at `now`, sequence above `after`, by sequence |
| `latest_control_sequence(account) -> i64` | 0 when none |
| `acknowledge_control_message(actor, account, message_id, at) -> bool` | only the account's own |
| `upsert_host_presence(actor, &HostPresenceRecord)` | |
| `list_host_presences(&[accounts], observed_after, now) -> Vec` | observed after the bound and unexpired at `now`, in the accounts' order (`(0, i64::MIN)` reads any) |
| `count_active_host_sessions(account) -> i64` | the host's live sessions holding a party key (a draining one has given it up) |
| `count_active_realm_servers(realm) -> i64` | the realm's live servers: hubs, Training and hunts |
| `count_running_realm_servers(realm) -> i64` | sessions whose server still runs on the host's PC, draining included |
| `list_live_realm_hunts(realm, host) -> Vec` | live realm hunts (never hubs) of that host, at most 64 |
| `find_host_reliability(account)`, `find_host_offer(offer_id)` | |
| `list_recent_participant_observations(id, generation, since) -> Vec` | newest first |

### 3.4 `RealmHubRepository` (hub presence rows)

`HubPresenceRecord` has at least `session_id`, `generation`, `realm_id`, `account_id`, `peer_id`, `candidate_id`,
`state: PresenceState`, `local_bridge_port: Option<i64>`, `local_bridge_ready_at: Option`, `expires_at`, `purge_at`
(the store half lists the rest). Methods called: `find_presence(session, account)`; `find_for_account(account, None)`
(newest live row, any hub); `list_by_state(session, generation, &[states], limit)` (oldest request first);
`count_by_state(session, generation, &[states]) -> i64`; `request_entries(actor, &EntryAt { session_id, generation,
realm_id, now, expires_at }, max_peers, &[HubEntryRequest { account_id, peer_id, candidate_id }]) -> HubEntryBatch`
(`Placed(Vec<(account, row, EntryOutcome)>)`, or `Full { live, max_peers }` with nothing written; a member with a live
row takes no new seat and is never refused); `admit_host(actor, session, generation, realm, account, peer, port, now)`;
`mark_route_ready(actor, session, generation, account, port, ready_at, now) -> Option<row>`; `mark_ticketed(actor,
session, generation, account, now, ticket_expires_at) -> Option<row>`; `mark_admitted(actor, session, generation,
account, admission_id, admitted_at, now, route: Option<&str>)`; `mark_lost(actor, session, generation, account,
Option<admission_id>, at, purge_at)`; `remove_account_from_realm(actor, realm, account, now, purge_at)`;
`remove_realm(actor, realm, now, purge_at)`; `expire_stale(actor, Option<session>, now, purge_at) -> Vec<row>`.

### 3.5 Other ports

`ports::{Clock, RateLimiter, IdSource, Accounts, RealmEventSink, RealmsLog, GuardSessions (find_healthy(account, id,
GuardRole) -> Option<HealthyGuard { role, guard_session_id, manifest_sequence, last_sequence, risk_score }>),
OperationsView (emergency_stop() -> Result<bool, String>, ice_transport(), guard_refuses())}` are the store half's.
`lifecycle::GuardRole` is this half's and `ports::HealthyGuard` names it. From rifts-seam: `IslesCore::cancel_travel`
and `set_candidate`. From rifts-admission: `AdmissionStore::put_peer_link`, `insert_ticket`, `TicketAuthority::mint`,
`Es256Tickets::key_id`, `JOIN_TICKET_LIFETIME_SECONDS`.

---

## 4. Contract strings at a glance

- **Error codes emitted by this half** (texts are in §2): `AUTH_RATE_LIMITED`, `REALMS_DISABLED`,
  `REALM_VALIDATION_FAILED`, `REALM_NAME_INVALID`, `REALM_NAME_RESERVED`, `REALM_NAME_TAKEN`, `REALM_ALREADY_EXISTS`,
  `REALM_USERNAME_REQUIRED`, `REALM_NAME_IS_USERNAME`, `REALM_NOT_FOUND`, `REALM_NOT_MEMBER`, `REALM_BANNED`,
  `REALM_SOLO`, `REALM_INVITE_REQUIRED`, `REALM_INVITE_INVALID`, `REALM_INVITE_EXPIRED`, `REALM_INVITE_LIMIT`,
  `REALM_FULL`, `REALM_OWNER_CHANGED`, `REALM_INVALID_TARGET`, `REALM_HOST_CANNOT_LEAVE`, `REALM_JOIN_REQUESTED`,
  `REALM_REQUESTS_FULL`, `REALM_REQUEST_NOT_FOUND`, `REALM_PLAYER_NOT_FOUND`, `REALM_PLAYER_INVITES_FULL`,
  `REALM_PLAYER_INVITE_NOT_FOUND`, `REALM_OFFLINE`, `REALM_HOSTING_DISABLED`, `REALM_SERVER_LIMIT`,
  `REALM_CAPACITY_EXCEEDED`, `REALM_HUB_FULL`, `P2P_DISABLED`, `P2P_EMERGENCY_STOP`, `P2P_SESSION_NOT_FOUND`,
  `P2P_SESSION_NOT_READY`, `P2P_NOT_ROSTERED`, `P2P_STEAM_LINK_REQUIRED`, `P2P_TICKET_INVALID`,
  `P2P_CAPABILITY_INVALID`, `P2P_GENERATION_FENCED`, `P2P_HOST_INELIGIBLE`, `P2P_SIGNAL_INVALID`, `GUARD_REQUIRED`,
  `ADMIN_VALIDATION`, `INTERNAL`.
- **Host ineligibility reasons**: `HOST_CONSENT_REQUIRED`, `HOST_LAUNCHER_OUTDATED`, `HOST_RUNTIME_UNVERIFIED`,
  `HOST_GUARD_UNVERIFIED`, `HOST_PRESENCE_STALE`, `HOST_MEMORY_LOW`, `HOST_CPU_UNSUITABLE`, `HOST_NETWORK_UNSUITABLE`,
  `HOST_ALREADY_ACTIVE`, `HOST_COOLDOWN`.
- **Lifecycle reasons** (transitions, `failure_reason`, `completion_reason`, recovery): `HOST_CANDIDATE_SELECTED`,
  `HOST_OFFER_EXPIRED`, `HOST_OFFER_DECLINED`, `TIMEOUT`, `HOST_BOOTSTRAP_ISSUED`, `HOST_READY`,
  `HUB_HOST_ROUTE_READY`, `FIRST_PARTICIPANT_ADMITTED`, `ALL_PARTICIPANTS_ADMITTED`, `HOST_STARTUP_TIMEOUT`,
  `HOST_FAILED_BEFORE_RELEASE`, `HOST_FAILED`, `HOST_LEASE_MISSED`, `PARTICIPANT_FAILURE_QUORUM`, `LEASE_RECOVERED`,
  `HOST_PROCESS_UNHEALTHY`, `HOST_ABORTED`, `HOST_COMPLETED`, `OPERATOR_P2P_STOP`, `P2P_DISABLED`, `NO_ELIGIBLE_HOST`,
  `REALM_FULL`, `NO_DEDICATED_SERVERS`, `RECOVERY_WINDOW_EXPIRED`, `TIER0_HUNT_COMPLETED`, `TIER0_PARTY_LEFT`, with
  the suffixes `_LEASE_RELEASED`, `_HOST_STOPPED`, `_P2P_ONLY`, `_REALM_ONLY`, `_NO_DEDICATED_FALLBACK`;
  `SUPERSEDED_REQUESTER_LEFT_HUNT`; hub: `OPERATOR_P2P_STOP`, `REALM_HOSTING_DISABLED`, `HUB_SESSION_EXPIRED`,
  `HUB_OFFER_EXPIRED`, `HUB_STARTUP_TIMEOUT`, `HOST_LEASE_LOST`, `SUPERSEDED_BY_HOST`, `HOST_STOPPED_HUB`,
  `REALM_RELEASED`, `REALM_HOST_OFFLINE`; party actions: `SESSION_ENDING`, `HUNT_FINISHED`, `HOST_LEASE_EXPIRED`,
  `REQUESTER_LEFT_HUNT`, `PARTY_CHANGED_HUNT`, `ROSTER_FROZEN`, `ROSTER_CHANGED`; travel cancels:
  `REALM_HUB_NOT_RUNNING`, `REALM_HUB_FULL`; candidate status: `HUB_ENDED`, `HUNT_ABORTED`.
- **Route reasons** (§2.9.3) and **status words** `KICKED`, `BANNED`, `UNBANNED`, `APPROVED`, `DENIED`,
  `CANCELLED`, `MEMBER`, `INVITED`.
- **Settings keys**: `MYSTICPARADOX_REALMS_MODULE`, `MYSTICPARADOX_REALMS_MAX_MEMBERS`,
  `MYSTICPARADOX_REALMS_MAX_BANS`, `MYSTICPARADOX_REALMS_HOSTING`, `P2P_REALM_HUB_CAPABILITY_TTL_HOURS`,
  `P2P_REALM_MIN_FREE_MEMORY_MIB_PER_SERVER`, the `P2P_*` keys of §2.9.2 and §2.10, `DEPLOYSERVER_URL`,
  `TARGET_CHANGELIST`.
- **Hash labels**: `paradox-realm-hub:v1:` (hub room ids), `mysticparadox-ice-peer-v1\n` (ICE peer ids).
- **Log lines**: every line in §2 is a host's tool, not a contract, except where a test reads it: the leases test
  checks that a recovery warn line contains the session id and the reason. Keep the prefixes (`[Realms]`, `[P2P]`,
  `[P2P Lifecycle]`, `[RealmHub]`, `[RealmCapacity]`, `[P2PArtifacts]`, `[GuardObserve]`) for operators' greps.

---

## 5. Tests that define the behaviour

### 5.1 Rewritten with the MIT crate (from this document)

Every test file of §1.2 is rewritten. They cover, by file:

- `realm_rules.rs`: names (pattern, hyphens, reserved in any case and with inner hyphens, every reserved name
  well-formed), username-to-name, release times, ownership change, invite alphabet and unbiased mapping, invite
  normalization, invite states, `decide_join` order, heartbeat, stable newest-first, every input parser's rules and
  messages, realm limits from settings, the module switch, hosting patches and validation, hub room ids and their
  normalization, the entry realm parser.
- `hub_rules.rs`: classification, manifests, party keys, start body, the exact transition table, the tick (healthy,
  lapsed, suspect back or aborted, deadlines, expiry, operator stop, hosting switch), host-failure reasons, the
  presence table, entry expiry and decisions, ticket denials and their errors, capacity, `hub_is_full`, realm hub
  settings, realm host eligibility.
- `lifecycle_rules.rs`: the scan set, operator-stop states, recovery participants, the dedicated rail, Guard session and
  role parsing, clean-end rules, hub returns, party session classification incl. drain, recovery destination, the
  Guard start grace.
- `tiers.rs`: every gate failing closed and independently, the bucket, tier 1, training, FTUE, high-risk switch,
  DeployServer rail, the relay warning.
- `host_policy.rs`: the six hashes, older names, pins vs feed, changelists, drain cap, eligibility order, launcher
  floor, score ordering.
- `leases.rs`, `directory.rs`, `directory_presence.rs`, `capabilities_control.rs`: on SQLite and, when
  `REFORGED_TEST_POSTGRES_URL` is set, on Postgres (the store half's `common/mod.rs`); `common/kit.rs` builds a
  directory with a settable clock, counting ids (`realm-1`..., codes `ABCDEFGHJK`...), recorded events and quiet logs.
- `hosting_bootstrap.rs`: one realm hub end to end: create a realm, start the Ramsgate hub (bootstrap JSON with a host
  ticket), host ready and route ready, a member routed in, admission ticket and ICE signalling through rifts-admission
  and rifts-relay routes, the control poll sees `ICE_SIGNAL`, ticket consumed (`admitted: true`), member route ready,
  status shows the hub with `admittedPlayers` 1, stop answers `stopped: true`.

Golden values the rewrite must reproduce (computed from the rules above): §2.3 time vectors; §2.4.8 hub room ids;
§2.9.3 rollout bucket; §2.13.1 ICE peer id; §2.14.1 capability digest.

### 5.2 Oracles kept in dainty-restricted

`rustic-rifts-realms` has no recorded vectors. Restricted keeps its own copies of these tests and runs them against the MIT
crate by git dependency before switching: `hosting_bootstrap.rs` (it ties realms to the admission and relay crates)
and the isles-1-14-7 suites that drive the crate (`tests/module.rs`: lease with an unhealthy heartbeat fenced,
recovery to ABORTED, presence and hub start through the mounted routes; `tests/inventory_reforge_host.rs`), plus
`modules/rustic-isles/crates/isles-1-14-7` built with the `realms` feature.

---

## 6. Target MIT crate shape

- Package `rustic-rifts-realms` at `crates/rustic-rifts-realms` in dainty-lass, `license = "MIT"`, as umbrella D4. Modules as today
  (`realm`, `directory`, `hub`, `hunt`, `lifecycle`, `leases`, `selection`, `tiers`, `host_policy`, `capabilities`,
  `control`, `input`, `ids`, `time`, `hosting`) so the paths of §2.1 hold; private helpers are free.
- Dependencies of this half: `rustic-rifts-seam`, `rustic-rifts-admission`, `rustic-rifts-relay`, `rustic-rifts-guard` (the MIT rewrites, umbrella
  D4), `module-api`, `store` (through the store half), `rand`, `serde`, `serde_json`, `sha2`, `thiserror`, `tokio`
  (`time` for the poll's sleep). All are in dainty-lass's `[workspace.dependencies]` except the four rifts crates,
  which the umbrella adds.
- **Seam:** dainty-lass's `serde_json` lacks `preserve_order`, and every answer here depends on insertion order (the
  `json!` objects and the flattened host view). Either the crate turns the feature on in its own manifest
  (`features = ["preserve_order"]`, no workspace change; Cargo unifies it workspace-wide, which other MIT crates must
  tolerate) or the integration pass adds it to the workspace entry. Flag for the integration pass.
- Lints: no `unwrap`/`expect` outside tests; every number conversion checked.

---

## 7. Acceptance criteria

1. Every public item of §2.1.1 exists with its name and signature; restricted's isles-1-14-7 builds against the MIT
   crate by changing only its dependency paths.
2. The rewritten tests of §5.1 pass on SQLite (and Postgres when configured), including the golden values.
3. The oracles of §5.2 pass against the MIT crate.
4. Every JSON answer matches §2 field for field and in order; every error code and message matches §2 byte for byte.
5. The similarity measure of `clean-room/similarity.md` §1 is under 30 % per file against `4efc8b5`.
6. `cargo fmt --check`, `clippy -D warnings`, `cargo test` and `cargo deny` pass in dainty-lass.

---

## 8. Risks and ambiguities

- **R1. Paradox names on the wire.** `x-mystic-host-capability`, `/mystic/v1/hunts/...`, the hash labels
  `paradox-realm-hub:v1:` and `mysticparadox-ice-peer-v1\n` (they change every hub room id and every ICE peer id if
  touched, and the hosted runtime and admission tickets depend on them), the settings `MYSTICPARADOX_REALMS_*`, and
  the reserved names `paradox`, `mysticparadox`, `mystic`, `mysticfox`. Recommend keeping all as contracts (umbrella
  D3); the umbrella decides.
- **R2. `HuntSession` is a public struct literal.** isles' test builds it field by field; adding or reordering a field
  breaks restricted. Keep the field set exactly (§2.7.4), `extra` included.
- **R3. JavaScript string rules.** Whitespace is Rust's `White_Space` plus U+FEFF; JavaScript's `trim` differs on
  U+0085 (Rust trims it, JS does not). Keep the Rust behaviour the tests pin.
- **R4. No scheduler.** Nothing at `4efc8b5` calls `Hosting::sweep` or `RealmDirectory::sweep_lapsed_hosts` outside
  tests, so leases, offers, startup deadlines and HOST_OFFLINE never advance in production. Out of scope for the
  rewrite (keep the methods); flag for the maintainer.
- **R5. The emergency stop and co-op transport writes store nothing.** `set_emergency_stop` and `set_coop_transport`
  echo a policy with `version: expected + 1` but persist no state (the stop is read from `OperationsView`). Reproduce
  as is; a real store is a later row.
- **R6. Statuses of unlisted codes.** `REALM_HOSTING_DISABLED`, `REALM_SERVER_LIMIT`, `REALM_CAPACITY_EXCEEDED`,
  `P2P_HOST_INELIGIBLE` and `ADMIN_VALIDATION` are not in isles-core's status table and answer 500 there. isles' fix,
  not this crate's.
- **R7. Placeholder hunt manifest.** A non-hub hunt's manifest is fixed (`/Game/Maps/islands/1705/dia_moss_triforce`,
  `Behemoth_Gnasher`), whatever the hunt. Reproduce; isles' real manifest source is a later row.
- **R8. Hub table vs routes.** The pure hub table (§2.8.2) goes HOST_READY → ADMITTING → RUNNING, but the host's
  route-ready moves a hub from STARTING or HOST_READY straight to RUNNING (§2.16.5). Both are tested as they are.
- **R9. Empty-roster quorum.** In the running tick, `failing >= min(roster, 2)` holds for an empty roster with no
  failures, so a non-hub session with no roster goes SUSPECT on its first tick. Routed hunts always have a roster;
  reproduce.
- **R10. `HuntLifecycle::observe` sends generation 1.** A hunt past generation 1 (after a retry) refuses the typed
  observation with the roster refusal. Reproduce; flag for the seam owner.
- **R11. Preserve-order seam** (§6).
- **R12. `GuardRole` name clash.** `hunt::GuardRole` (stored struct) and `lifecycle::GuardRole` (enum) share a name in
  different modules; both are public. Keep both.
- **R13. Timing.** The control poll's deadline uses the wall clock while message expiry uses the injected clock; the
  directory's presence cache uses the injected clock. Tests rely on both.
