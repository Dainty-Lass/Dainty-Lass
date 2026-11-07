<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false, "code_blocks": false } } -->

# RD27 Clean-Room Specification: `rustic-rifts-guard` (Rustic Rifts' Guard in observe mode)

> **Scope and purpose (RD27):** the behaviour specification from which an implementer who never sees the AGPL code
> writes an MIT `rustic-rifts-guard`. In dainty-restricted the crate lives at `modules/rustic-rifts/crates/rustic-rifts-guard`, with
> two fuzz targets in `modules/rustic-rifts/fuzz/fuzz_targets/` and its checklist in
> `modules/rustic-rifts/docs/security/guard.md`. It is Guard on stubs: the Hunt Guard evidence chain intake, host
> write authorization, the native inventory fence, the host grant policy and the daily premium draft token cap, the
> Guard traffic bounds, the host route census and two admin operations routes, with its `store` tables.
>
> Everything below is stated from observable contracts: routes, statuses, error codes and texts, JSON replies (field
> order included), log lines the tests read, configuration keys, SQL tables, the hash input, and the public Rust API
> that other crates call. No implementation code is reproduced and no private helper is named.
>
> Spec written against dainty-restricted `4efc8b5` and dainty-lass `49197a8`.

---

## 1. Provenance and classification

### 1.1 Evidence

| Source | What it says |
| --- | --- |
| `modules/rustic-rifts/NOTICE.md` | Every Rust crate of rustic-rifts, `rustic-rifts-guard` included, ports Mystic Paradox's `p2p/`, realms, Guard and launcher modules (`21b9ff5`, AGPL-3.0-only) |
| Crate doc of `src/lib.rs` | Names the Node files the crate ports (the anticheat stub side, host writes, hub authorization, inventory transactions and authorization, grant policy, draft premium, host policy mode, traffic, evidence setting, census, operations alerts) |
| `git log` of the crate | `aac3fb5` (RF6, the port), `b5c25ff` (SHA-256 from `sha2`), `64853ca` (seams), `210eb22` (`SqlGuardStore`, the migrations and `tests/sql_store.rs` written on Dainty Lass's `store`), `748fe84` (capability route resolution), `a10c21d` (migration moved here), `ca76df9` (enforce mode) |
| `modules/rustic-rifts/docs/inventory.json` | The Node route inventory; the route-order test of `routes.rs` reads it |

### 1.2 Categories

- **A**: born in Dainty Lass, free of Mystic Paradox derivation; moves as is (with the edits named).
- **B**: ported from Mystic Paradox, or written inside the ported structure; rewritten clean-room from this document.
- **D**: data recorded from Mystic Paradox's programs; stays in dainty-restricted as an oracle.
- **G**: generated.

### 1.3 Files

| Path (under `modules/rustic-rifts/`) | Lines | Cat. | Note |
| --- | ---: | :---: | --- |
| `crates/rustic-rifts-guard/Cargo.toml` | 27 | A | Moves with `license = "MIT"`, `rustic-rifts-seam` from its MIT home, `serde_json` with `preserve_order` (§7) |
| `crates/rustic-rifts-guard/migrations/sqlite/2026-10-26-000002_rifts_guard/up.sql` | 106 | A | Born in `210eb22` on `store` (DD15); DDL is a contract existing databases carry (§4). Byte-identical |
| `crates/rustic-rifts-guard/migrations/sqlite/2026-10-26-000002_rifts_guard/down.sql` | 7 | A | Same |
| `crates/rustic-rifts-guard/migrations/postgres/2026-10-26-000002_rifts_guard/up.sql` | 106 | A | Same |
| `crates/rustic-rifts-guard/migrations/postgres/2026-10-26-000002_rifts_guard/down.sql` | 7 | A | Same |
| `crates/rustic-rifts-guard/tests/sql_store.rs` | 559 | A | Born in `210eb22`; black-box over the public store API only (§6.1). Reused as is |
| `crates/rustic-rifts-guard/src/lib.rs` | 1294 | B | `Guard`, `Stub`, the hooks, the evidence flow, the fence, the operations flows |
| `crates/rustic-rifts-guard/src/evidence.rs` | 745 | B | Batch and completion readers, chain application, replies |
| `crates/rustic-rifts-guard/src/json.rs` | 355 | B | The P2P body parser and JavaScript text helpers |
| `crates/rustic-rifts-guard/src/config.rs` | 451 | B | Settings |
| `crates/rustic-rifts-guard/src/errors.rs` | 145 | B | `GuardError`, the status table |
| `crates/rustic-rifts-guard/src/digest.rs` | 73 | B | SHA-256 hex |
| `crates/rustic-rifts-guard/src/census.rs` | 360 | B | Host route census |
| `crates/rustic-rifts-guard/src/routes.rs` | 345 | B | The six routes |
| `crates/rustic-rifts-guard/src/traffic.rs` | 367 | B | Windows and the in-flight budget |
| `crates/rustic-rifts-guard/src/operations.rs` | 331 | B | Admin operations rules and replies |
| `crates/rustic-rifts-guard/src/ports.rs` | 444 | B | Ports, clocks, logs, ISO time |
| `crates/rustic-rifts-guard/src/store.rs` | 599 | B | Records, `GuardStore`, in-memory and stub stores |
| `crates/rustic-rifts-guard/src/sql.rs` | 1040 | B | `SqlGuardStore`. Written here (`210eb22`) but on the ported record shapes; the lowest-risk B file, and its SQL text is contract (§4) |
| `crates/rustic-rifts-guard/src/grants.rs` | 1118 | B | Grant classes, the draft token decision |
| `crates/rustic-rifts-guard/src/writes.rs` | 627 | B | Who a host may write for |
| `crates/rustic-rifts-guard/src/tests.rs` | 1162 | B | Unit tests on in-memory parts (rewrite from §6.2) |
| `crates/rustic-rifts-guard/src/testkit.rs` | 164 | B | Test fixtures |
| `fuzz/fuzz_targets/guard_evidence.rs` | 62 | B | Rewrite from §6.3 into dainty-lass `fuzz/` |
| `fuzz/fuzz_targets/guard_grants.rs` | 65 | B | Same |
| `docs/security/guard.md` | 195 | B | Rewrite as `docs/security/guard.md` in dainty-lass, `relay.md`'s structure, test names of the new crate |

Counts: **A 6, B 20, D 0, G 0.** The crate has no recorded vectors. `docs/inventory.json` is read by one test but
belongs to rustic-rifts' docs, not this crate; the MIT test compares against the literal route list in §3.3 instead.

### 1.4 What depends on what

The B files depend on the A files only through the migrations (`SQLITE_MIGRATIONS`/`POSTGRES_MIGRATIONS` embed the
two folders) and `tests/sql_store.rs` depends on the public API of §2 only. Nothing else crosses.

---

## 2. Public API (what must exist, by name)

The MIT crate must keep these names, paths and signatures so dainty-restricted switches by changing only its Cargo
dependency. "Ext" says who outside the crate uses each: **R** rifts-realms (src), **Rt** rifts-realms tests, **I**
isles-1-14-7 (src and tests), **F** the fuzz targets, **T** `tests/sql_store.rs`. Items marked "—" are public today
but used by no caller; keep them (cheap) unless the umbrella spec decides otherwise.

Types from `rustic-rifts-seam` (`AccountId`, `SessionId`, `RealmId`, `UnixMillis`, `Json`, `Reply`, `Refusal`, `SeamError`,
`StoreError`, `Unimplemented`, `Component`, `Actor`, `RuntimeAuth`, `CapabilityScope`, `HubKind`, `HostWriteKind`,
`HostWrite`, `HostWriteOptions`, `InventoryTransaction`, `InventoryDecision`, `InventoryFence`, `TransactionReads`,
`TransactionPhase`, `HostGrantCatalog`, `IslesCore`, `GuardHooks`, `Lifecycle`, `RouteHandler`, `RouteCall`, `Route`,
`Method`, `Mount`, `Caller`, `RequestInfo`, `StubRoutes`, `HostRouteSample`, `TableSpec`, `DeletionStep`,
`DeletionAction`, `Secret`, `Settings`, `BoxFuture`, `HOST_CAPABILITY_HEADER`) are the seam's and are only used here.

### 2.1 Crate root

| Item | Kind and signature (prose) | Ext |
| --- | --- | --- |
| `Guard` | struct; `Guard::new(GuardParts) -> Guard`; `config(&self) -> &GuardConfig`; `request_budget(&self) -> &RequestBudget`; `record_census(&self, &HostRouteSample)`; `flush_census(&self)`; `reward_policy_status(&self, hunt_id: &str, expected_version: Option<&str>) -> RewardPolicyStatus`; `open_chain(&self, SessionId, generation: u64, completion_mode: Option<&str>) -> Result<(), GuardError>`; `chain_status(&self, &SessionId, u64) -> Result<Option<ChainRecord>, GuardError>`; `submit_parsed(&self, &RuntimeAuth, Option<&serde_json::Value>) -> Result<Json, GuardError>`; `complete_parsed(&self, &RuntimeAuth, Option<&Value>) -> Result<Json, GuardError>`; `self_reward(&self, &AccountId, session: &str) -> Result<Json, GuardError>`. Implements `Lifecycle`, `GuardHooks`, `EvidenceIntake`, `TrafficLimits`, `GuardOperations`; `Send + Sync`; `Arc<Guard>` coerces to `Arc<dyn GuardHooks>` | I, Rt |
| `GuardParts` | struct, all fields `pub`: `config: GuardConfig`, `store: Arc<dyn GuardStore>`, `hunts: Arc<dyn HuntSessions>`, `operations: Arc<dyn OperationsPolicyPort>`, `core: Arc<dyn IslesCore>`, `clock: Arc<dyn Clock>`, `log: Arc<dyn GuardLog>` | I, Rt |
| `Stub` | unit struct (`Debug, Clone, Copy, Default`); implements the same five traits, every operation `Unimplemented` (§3.13) | Rt |
| `HuntVerdict` | struct `{ pub progression_applied: bool, pub rewards_applied: bool }`, `const OBSERVED`, `to_json(&self) -> Json` | — |
| `RewardPolicyStatus` | enum `Ready { version: String, digest: String }`, `NotReady { reason: &'static str }` | — |
| `EvidenceIntake` | trait: `submit_evidence(&self, RuntimeAuth, Json)`, `complete(&self, RuntimeAuth, Json)` → `BoxFuture<Result<Json, GuardError>>`; `verdict(&self, SessionId, u64)` → `Result<Option<HuntVerdict>, GuardError>`; `reward_receipt(&self, SessionId, u64, AccountId)` → `Result<Option<Json>, GuardError>`; `record_shadow_comparison(&self, Option<Secret>, Json)` → `Result<Json, GuardError>` (all boxed futures) | — |
| `TrafficDecision` | enum `Accepted`, `RateLimited`, `Unavailable` | — |
| `TrafficLimits` | trait: `charge(&self, host: AccountId, session: SessionId, bytes: u64) -> BoxFuture<Result<TrafficDecision, GuardError>>` | — |
| `GuardOperations` | trait: `set_reward_rollout(&self, admin_key: String, body: Option<Value>, ip: String)`, `acknowledge_alert(&self, admin_key: String, alert_id: String, body: Option<Value>, ip: String)` → `BoxFuture<Result<Reply, GuardError>>` | — |
| `GUARD_MANIFEST_ROLES` | `&[&str] = ["TRANSPORT_MODULE", "TRANSPORT_RUNTIME", "ADMISSION_KEY"]` | — |
| `GuardConfig` | struct with `pub` fields `host_policy: HostPolicySetting`, `hunt_evidence: HuntEvidenceSetting`, `trust_tier: TrustTier`, `tier1: Tier1Policy`, `census: bool`, `host_writes: bool`, `distributed_traffic: bool`; `Default`; `from_settings(&dyn Settings)`; `host_policy_refuses(&self) -> bool`; `tier1_refusal(&self, &[AccountId], hunt_id: &str) -> Option<&'static str>` | I, Rt |
| `HostPolicyMode`, `HostPolicySetting`, `HuntEvidenceSetting`, `Tier1Policy`, `TrustTier` | §3.1 | — |
| `resolve_host_policy`, `log_host_policy`, `resolve_hunt_evidence`, `log_hunt_evidence` | §3.1 | — |
| `sha256_hex` | `fn(&[u8]) -> String` (lower-case hex) | — |
| `GuardError`, `ERROR_STATUSES`, `status_of` | §3.2 | F (`GuardError::code`) |
| `RewardRollout`, `DEFAULT_REWARD_ROLLOUT` | §3.11 | — |
| `GuardRoutes` | struct: `new(Arc<Guard>)`, `with_store(Arc<Guard>, has_store: bool)`; implements `RouteHandler`; `Clone` | I |
| `ROUTES`, `stub_routes() -> StubRoutes`, `envelope(&Refusal, request_id: &str) -> Reply` | §3.3 | I, Rt (`stub_routes`) |
| `HostRouteCensus`, `HostRouteRow`, `HostRouteSample` (re-export), `HostRouteSummary` | §3.12 | — |
| `SqlGuardStore`, `SCHEMA`, `SQLITE_MIGRATIONS`, `POSTGRES_MIGRATIONS`, `XP_SUMMARY_TTL_MS`, `block_on_store`, `store_actor` | §3.15 | R, I, Rt, T |
| `GuardStore`, `MemoryGuardStore`, `StubGuardStore`, `TABLES`, `GENESIS_HASH` and the records `AlertRecord`, `BatchRecord`, `ChainRecord`, `ChainUpdate`, `CompletionMode`, `CompletionRecord`, `Disposition`, `EncounterState`, `FindingRecord`, `HostWriteRecord`, `TrafficReservation`, `TrafficScope`, `TrafficWindowRecord`, `XpSummaryRecord` | §3.15, §4 | R, I, Rt, F, T |
| `HUNT_GUARD_HOST_TRAFFIC_LIMIT`, `HUNT_GUARD_SESSION_TRAFFIC_LIMIT`, `HUNT_GUARD_REQUEST_BUDGET`, `HuntOperation`, `LocalTrafficLimiter`, `RequestBudget`, `TrafficLimit` | §3.7 | — |
| Port re-exports `AuditEntry`, `CapabilityView`, `Clock`, `CoreOperationsPolicy`, `GuardLog`, `HuntSessions`, `HuntView`, `ManualClock`, `MemoryLog`, `MemoryOperations`, `NoHuntSessions`, `OperationsPolicyPort`, `PolicySnapshot`, `RosterEntry`, `StderrLog`, `SystemClock`, `iso_time` | §3.14 | R, I, Rt |
| `host_write_denial`, `inventory_body_denial`, `inventory_denial`, `kind_name`, `loot_refusal`, `write_refusal` | §3.8, §3.9 | — |

### 2.2 Public modules

The modules `census`, `evidence`, `grants`, `json`, `ports`, `sql`, `store` are `pub`. Paths callers use:

- `rifts_guard::ports::{CapabilityView, HuntSessions, HuntView, RosterEntry, SystemClock, StderrLog}` (R, I, Rt).
- `rifts_guard::evidence::{MAX_BATCH_EVENTS, MAX_BATCH_EVENT_BYTES, parse_batch, apply_batch, parse_completion,
  apply_completion}` and the types they return (`EvidenceBatch` with `pub` fields `batch_id, first_sequence,
  last_sequence, previous_hash, events, event_bytes, batch_hash`; `EvidenceEvent` with `sequence, monotonic_ms, kind,
  actor, payload`; `ChainRefusal { pub error: GuardError, pub finding: Option<(String, String)> }`;
  `CompletionRequest { pub final_sequence, pub final_hash }`; `BatchProblem`) (F).
- `rifts_guard::json::parse_p2p_body` (F); the rest of `json` (`P2P_MAX_HTTP_BODY_BYTES`, `BodyError`,
  `parse_admin_body`, `js_string`, `js_number`, `stringify`, `stringified_len`, `safe_integer`, `js_length`,
  `js_slice`, `js_trim`, `js_to_number`, `js_safe_integer`) is public but unused outside.
- `rifts_guard::grants::{Allowances, GrantSession, HOST_DRAFT_PREMIUM_BALANCE_CAP, HOST_DRAFT_PREMIUM_ID,
  NoGameData, classify, decide_draft_premium, grant_denial, stacked_balance_of, violations}` with `Violation` (`pub`
  fields `catalog_id, denied_class, field`, `PartialEq`), `GrantDenial` (`classes: Vec<&'static str>, violations,
  summary`), `DraftPremiumDecision` (`allowed, reason: &'static str, balance: Option<(i128, i128)>`), `DenyClass`,
  `GrantField` (F). Also public: `HOST_STARTER_COSMETIC_IDS`, `CHALLENGE_REWARD_CURRENCIES`,
  `MAX_CHALLENGE_REWARD_PER_CLAIM`, `is_claim_funded_coin_grant`, `is_prestige_funded_grant`,
  `is_host_starter_cosmetic_add`, `is_host_draft_premium_add`, `body_adds_host_draft_premium`,
  `is_platinum_spend_item`, `body_has_grant_or_spend`.
- `rifts_guard::census::{HOST_ROUTE_CENSUS_WINDOW_MS, HOST_ROUTE_CENSUS_MAX_ROWS, HostRouteKey, status_class,
  normalize, route_of, aggregate, summarize}`.

### 2.3 Signatures callers depend on exactly

| Item | Shape |
| --- | --- |
| `HuntSessions` | trait `Send + Sync`: `hunt(&self, &SessionId) -> Result<Option<HuntView>, StoreError>`; `hub_present_accounts(&self, &SessionId, generation: u64) -> Result<Vec<AccountId>, StoreError>`; `capability_by_digest(&self, digest: &str) -> Result<Option<CapabilityView>, StoreError>`; `resolve_capability(&self, digest: &str, session_id: &str, scope: CapabilityScope, now: UnixMillis) -> Result<Option<RuntimeAuth>, StoreError>` **with a default body returning `Ok(None)`** (realms overrides it) |
| `HuntView` | struct, all `pub`: `session: SessionId`, `generation: u64`, `state: String`, `pre_suspect_state: Option<String>`, `host: Option<AccountId>`, `realm: Option<RealmId>`, `hub: Option<HubKind>`, `roster: Vec<RosterEntry>`, `frozen_characters: Vec<(AccountId, String)>`, `completion_mode: Option<CompletionMode>`; methods `is_hub()`, `session_kind() -> Option<&'static str>` (`"REALM_HUB"` for a hub), `hub_kind() -> Option<&'static str>` (`"RAMSGATE"`, `"TRAINING"`); `Debug, Clone, PartialEq, Eq` |
| `RosterEntry` | `{ pub account: AccountId, pub admitted: bool }` |
| `CapabilityView` | `{ pub capability_id: String, pub expires_at: UnixMillis, pub revoked: bool }` |
| `Clock` | trait `Send + Sync`: `now(&self) -> UnixMillis`; `uuid(&self) -> String` |
| `ManualClock` | `at(UnixMillis) -> Self`, `advance(&self, millis: u64)`; `now` is the set time; `uuid` counts up (§3.14) |
| `SystemClock` | `Default`; real time; random v4 UUIDs |
| `GuardLog` | trait `Send + Sync`: `info/warn/error(&self, line: &str)`. `StderrLog` (unit, `Default`), `MemoryLog` (`Default`, `lines() -> Vec<String>`, each `"<level> <line>"`) |
| `CompletionMode` | enum `FirstDefeat`, `EndOfHunt`, `NoCompletion`; `parse(&str) -> Option<Self>` on `FIRST_DEFEAT`, `END_OF_HUNT`, `NO_COMPLETION` (exact case) |
| `block_on_store` | `fn<F, T>(F) -> Result<T, StoreError>` where `F: Future<Output = Result<T, StoreError>> + Send`, `T: Send` (§3.15) |
| `SqlGuardStore` | `Clone`; `open(&DatabaseConfig, &DatabaseOptions) -> Result<Self, store::Error>`; `new(store::Database) -> Self`; `database(&self) -> &Database`; `host_writes()`, `windows()`, `alert(&str)`, `xp_summary(&SessionId, &AccountId)`, `purge_expired(UnixMillis) -> Result<u64, StoreError>`; implements `GuardStore` |
| `SCHEMA` | `store::DatabaseSchema { sqlite: SQLITE_MIGRATIONS, postgres: POSTGRES_MIGRATIONS, soft_delete: &[] }` |
| `CoreOperationsPolicy` | `new(Arc<dyn IslesCore>) -> Self`; implements `OperationsPolicyPort` |

Public API size: about 120 named public items across the root and the seven modules, of which 57 are used outside
the crate (28 store and root items, 12 ports, 6 in `evidence`, 1 in `json`, 10 in `grants`).

---

## 3. Behaviour

### 3.0 Conventions

- **Times** are Unix milliseconds from the `Clock`. **ISO time** is `new Date(ms).toISOString()`:
  `YYYY-MM-DDTHH:MM:SS.mmmZ`, proleptic Gregorian, UTC, four-digit year, works for negative times
  (`-1` → `1969-12-31T23:59:59.999Z`).
- **UUIDs** are version 4 text, 36 characters, lower-case hex, `xxxxxxxx-xxxx-4xxx-Vxxx-xxxxxxxxxxxx` with V in
  `8..b`. `SystemClock` draws them from any non-cryptographic random source (they name records only).
  `ManualClock`'s n-th id (from 1) is the 16 bytes `0…0 ‖ n as big-endian u64` with the version and variant bits
  then set (so the first is `00000000-0000-4000-8000-000000000001`).
- **JS text.** "JSON-quoted" is `JSON.stringify` of a string: `"` and `\` escaped, `\b \f \n \r \t` by letter,
  other code points below U+0020 as `\u00xx` (lower-case hex), everything else (U+2028, non-ASCII, DEL) literal
  UTF-8. "JS number" is ECMAScript `Number::toString` (shortest round-trip digits; integer form up to 21 digits, `0.`
  form down to 1e-6, else `d.ddde+x`/`e-x`; `-0` is `0`; NaN/infinite are `null`). "UTF-16 length/slice" counts
  UTF-16 units and never splits a character. "JS trim" removes Unicode white space and U+FEFF from both ends.
- **Canonical text** of a JSON value (`stringify`): `JSON.stringify` of the parsed value, no whitespace, keys in the
  **order they first appear in the document** (a duplicate key keeps its first position and its last value, as
  `JSON.parse` does), strings JSON-quoted, every number written as a JS number of its `f64` value. This needs
  `serde_json`'s `preserve_order` (restricted's workspace turns it on; §7, §9 risk 1).
- **Safe integer** (`Number.isSafeInteger`) of a JSON value: a JSON number whose `f64` is integral and within
  ±(2^53 − 1). A string is never one. "Non-negative safe integer" additionally ≥ 0.
- **Log lines** go to `GuardLog` at the level named; `MemoryLog` stores `"info …"`, `"warn …"`, `"error …"`.
  `[HUB]` below is ` hub=RAMSGATE` / ` hub=TRAINING` when the hunt is a hub, else nothing; `[REALM]` is
  ` realm=<realm id>` when the hunt has one. `SessionId`'s display is the seam's.

### 3.1 Configuration

All keys are read once through `rifts_seam::Settings` by `GuardConfig::from_settings`; absent is the same as empty.

| Key | Parsing | Default | Field |
| --- | --- | --- | --- |
| `MYSTICPARADOX_HOST_POLICY` | trim, lower-case; `""` → observe (default), `observe`, `enforce`; anything else → observe with a warning | observe | `host_policy` |
| `MYSTICPARADOX_HUNT_EVIDENCE` | trim, lower-case; `on`, `off`; `""` → the tier default (default); anything else → the tier default with a warning | on at tier 1, off at tier 0 | `hunt_evidence` |
| `P2P_TRUST_TIER` | Tier 1 only when the trimmed value is exactly `1`; anything else (absent, `2`, `true`) is tier 0 | 0 | `trust_tier` |
| `P2P_TIER1_REWARDS_ENABLED` | strict true: trimmed, case-insensitive `true` only (`1`, `on`, `yes` are false) | false | `tier1.rewards_enabled` |
| `P2P_TIER1_SHADOW_APPROVED` | strict true | false | `tier1.shadow_approved` |
| `P2P_TIER1_ALLOWED_ACCOUNTS` | split on `,`, trim each, drop empties, a set | empty | `tier1.allowed_accounts` |
| `P2P_TIER1_ALLOWED_HUNTS` | same | empty | `tier1.allowed_hunts` |
| `MYSTICPARADOX_HOST_ROUTE_CENSUS` | on unless trimmed value is case-insensitively `0`, `false`, `off` or `no` | on | `census` |
| `P2P_HOST_WRITES` | same switch | on | `host_writes` |
| `P2P_DISTRIBUTED_TRAFFIC_LIMITER` | strict true | false | `distributed_traffic` |

`HostPolicySetting { mode: HostPolicyMode, is_default: bool, warning: Option<String> }`;
`HostPolicyMode::{Observe (default), Enforce}`, `as_str` → `observe`/`enforce`.
`HuntEvidenceSetting { enabled, is_default, warning }`. `TrustTier::{Tier0 (default), Tier1}`,
`from_setting(Option<&str>)`, `number()` → 0/1. `Tier1Policy { rewards_enabled, shadow_approved, allowed_accounts:
BTreeSet<String>, allowed_hunts: BTreeSet<String> }`. `is_default` is true only for the empty value.

Warnings (`<v>` is the trimmed, lower-cased value, JSON-quoted; the second ends in `(on)` or `(off)`):

```text
MYSTICPARADOX_HOST_POLICY must be observe or enforce (got <v>); it is treated as observe, so host policy refuses nothing
MYSTICPARADOX_HUNT_EVIDENCE must be on or off (got <v>); the trust tier's default applies (on)
```

`host_policy_refuses()` is true only in enforce mode. `tier1_refusal(accounts, hunt_id)`: `None` at tier 0;
otherwise, in this order, `TIER1_REWARDS_DISABLED`, `TIER1_SHADOW_NOT_APPROVED`, `TIER1_ACCOUNT_NOT_ALLOWLISTED` (any
account not in the set), `TIER1_HUNT_NOT_ALLOWLISTED`, else `None`.

**Startup lines** (`log_host_policy`, `log_hunt_evidence`; `Lifecycle::start` logs host policy first, then evidence).
A warning, when present, is logged first at warn as `[Startup] WARNING: <warning>`. Then at info:

- observe: `[Startup] MYSTICPARADOX_HOST_POLICY<src>: host grant classes, the draft token cap and write kinds refuse
  nothing; would-be refusals are logged as [HostPolicyObserve]`, `<src>` = ` not set (default observe)` when
  `is_default`, else `=observe`.
- enforce: `[Startup] MYSTICPARADOX_HOST_POLICY=enforce: host grant classes, the draft token cap and write kinds
  refuse as before`
- evidence on: `[Startup] MYSTICPARADOX_HUNT_EVIDENCE<src>: hosted hunts send Hunt Guard evidence (the host's
  transport posts it on its packet thread)`; off: `[Startup] MYSTICPARADOX_HUNT_EVIDENCE<src>: hosted hunts send no
  Hunt Guard evidence`. `<src>` = ` not set (the trust tier's default)` when `is_default`, else `=on`/`=off` (also
  after a warning).

### 3.2 Errors and statuses

`GuardError` (`Debug, Clone, PartialEq, Eq`, `thiserror`, `#[non_exhaustive]`): `Unimplemented(Unimplemented)`
(transparent), `Refused(Refusal)` (transparent), `Internal(String)` (displays the string). Constructors and
conversions: `GuardError::refused(code: &str, message: &str)`; `code(&self) -> Option<&str>` (the refusal's code, else
`None`); `From<Unimplemented>`, `From<Refusal>`, `From<StoreError>` (its `Unimplemented` stays `Unimplemented`, any
other becomes `Internal(error.to_string())`), `From<SeamError>` (`Unimplemented` and `Refused` kept, others
`Internal`). Converting back for a hook or route answer: `Unimplemented` and `Refused` as they are; `Internal`
becomes the refusal `INTERNAL` / `The player-hosted hunt request failed.`

`ERROR_STATUSES` (in this order) and `status_of(code)` (500 for any other code):

| Code | Status | | Code | Status |
| --- | ---: | --- | --- | ---: |
| `P2P_SESSION_NOT_FOUND` | 404 | | `GUARD_FINALIZATION_BUSY` | 503 |
| `P2P_CAPABILITY_INVALID` | 401 | | `GUARD_CHAIN_CONFLICT` | 409 |
| `P2P_GENERATION_FENCED` | 409 | | `GUARD_CHAIN_FROZEN` | 409 |
| `GUARD_REQUIRED` | 403 | | `GUARD_COMPLETION_INVALID` | 409 |
| `GUARD_BATCH_INVALID` | 400 | | `GUARD_COMPLETION_CONFLICT` | 409 |
| `GUARD_BATCH_TOO_LARGE` | 413 | | `GUARD_COMPLETION_PENDING` | 409 |
| `GUARD_BATCH_REPLAY` | 409 | | `GUARD_SHADOW_INVALID` | 403 |
| `GUARD_EVIDENCE_LIMIT` | 409 | | `REWARD_CAPACITY_RECONCILIATION_REQUIRED` | 503 |
| `GUARD_TRAFFIC_RATE_LIMITED` | 429 | | `HUNT_EVIDENCE_NOT_COLLECTED` | 404 |
| `GUARD_REQUEST_BUDGET_EXHAUSTED` | 503 | | `HUNT_REWARDS_UNAVAILABLE` | 404 |
| `GUARD_TRAFFIC_UNAVAILABLE` | 503 | | | |

`GUARD_REQUIRED`, `GUARD_COMPLETION_PENDING`, `GUARD_SHADOW_INVALID` and `REWARD_CAPACITY_RECONCILIATION_REQUIRED` are
registered but never thrown here. `AUTH_UNAUTHORIZED` and `INTERNAL` are thrown but their status is isles'.

`envelope(refusal, request_id)` → `Reply { status: status_of(code), body:
{"error":{"code":<q>,"message":<q>,"requestId":<q>}} }` (JSON-quoted values, that field order).

**Why the statuses matter to the host.** The host's transport resends the same batch on no answer, 408, 429 or any
5xx, with backoff; any other non-200 (400, 401, 404, 409, 413) ends that hunt's evidence for good (it logs
`GUARD_EVIDENCE_LOSS action=completion_fenced reason=batch_refused status=<n>` and requests no completion). So the
traffic and budget refusals must stay 429/503 and the chain refusals 409.

### 3.3 Routes, in mount order

`ROUTES` is exactly, in this order (`Route { method, path, mount, handler }`):

| # | Method | Path | Mount | Handler id |
| ---: | --- | --- | --- | --- |
| 1 | POST | `/mystic/v1/hunts/:sessionId/event-batches` | `Early` | `antiCheat.ts:hunts.submitEvidence` |
| 2 | POST | `/mystic/v1/hunts/:sessionId/completion` | `Early` | `antiCheat.ts:hunts.complete` |
| 3 | GET | `/mystic/v1/hunts/:sessionId/reward/self` | `Early` | `p2pHuntStatus.ts:GetSelfHuntReward` |
| 4 | POST | `/internal/v1/hunt-guard/shadow-comparisons` | `Early` | `antiCheat.ts:hunts.recordShadowComparison` |
| 5 | PATCH | `/admin/v1/operations/reward-rollout` | `Admin` | `p2pAdminOperations.ts:UpdateRewardRollout` |
| 6 | PATCH | `/admin/v1/operations/alerts/:alertId/ack` | `Admin` | `p2pAdminOperations.ts:AcknowledgeOperationsAlert` |

`Early` routes come before isles' shared JSON parser (Guard parses their bodies itself, §3.4). The handler ids are
the `Unimplemented` operation names the stub and the store-less handler answer with (risk 3).
`stub_routes()` → `StubRoutes { component: Component::Guard, routes: ROUTES }`.

`GuardRoutes` answers a `RouteCall`:

1. If built `with_store(_, false)`: every route → `SeamError::Unimplemented(Guard, <handler id>)`.
2. Routes 1-4 (the hunts router):
   1. Parse the body with the P2P parser (§3.4). Too large → `GUARD_BATCH_TOO_LARGE` / `The Guard request body
      exceeds its bounded limit.`; invalid → `GUARD_BATCH_INVALID` / `The Guard request body is not valid JSON.`
      This happens before any authentication.
   2. For routes 1 and 2 with an `Anonymous` caller, resolve the `x-mystic-host-capability` header: the value must be
      exactly 43 characters of `[A-Za-z0-9_-]`; its SHA-256 lower-case hex goes to
      `HuntSessions::resolve_capability(digest, <:sessionId param>, scope, now)` with scope `EventsWrite` (route 1)
      or `CompleteRequest` (route 2). `Some(runtime)` makes the caller `HostRuntime(runtime)`; a malformed header,
      `None` or an error leaves it anonymous. Any other caller is taken as given.
   3. Routes 1-2 with a host runtime: charge the in-flight budget with `bytes` = the UTF-8 length of the body's
      canonical text (4, the length of `null`, when there is no body); no permit → `GUARD_REQUEST_BUDGET_EXHAUSTED`
      / `Guard request capacity is currently exhausted; retry shortly.` The permit is held until the answer is made.
      Then charge the traffic windows (§3.7) with the same bytes: `RateLimited` → `GUARD_TRAFFIC_RATE_LIMITED` /
      `Guard traffic exceeded the bounded host or hunt-session budget; retry shortly.`; `Unavailable` →
      `GUARD_TRAFFIC_UNAVAILABLE` / `Guard traffic capacity is temporarily unavailable; retry shortly.` Then
      §3.5's submit or completion. 200 with the JSON it returns.
   4. Routes 1-2 without a host runtime → `P2P_CAPABILITY_INVALID` / `A valid hunt host capability is required.`
   5. Route 3 with a `Launcher(account)` caller → §3.6; any other caller → `AUTH_UNAUTHORIZED` / `Authentication is
      required.`
   6. Route 4, any caller → `HUNT_EVIDENCE_NOT_COLLECTED` (§3.6). No budget or traffic is charged.
3. Routes 5-6: caller must be `Admin { key_id }`, else `AUTH_UNAUTHORIZED` / `Authentication is required.` The body
   is parsed leniently (empty or unparsable → `None`; no content-type check). `ip` is the request's remote address
   as text, or `unknown`. Then §3.11. Their replies are returned as `Ok(Reply)` with Node's own shapes, not the
   launcher envelope.
4. Any `GuardError::Internal(detail)` is logged at error as
   `Unhandled P2P route error requestId=<request id, or missing when empty>: <detail>` and answered as `INTERNAL`.
   Refusals come back as `SeamError::Refused` (isles writes the envelope with §3.2's status).

### 3.4 The P2P body parser (`json::parse_p2p_body`)

`parse_p2p_body(headers: &[(String, String)], body: &[u8]) -> Result<Option<Value>, BodyError>`, Express 5's
`express.json({ limit: 72 KiB, strict: true })`. Header names are matched exactly as given (lower case).
`BodyError::{TooLarge, Invalid}`; `P2P_MAX_HTTP_BODY_BYTES = 73_728`.

1. `content-type` absent, or its media type (before the first `;`, trimmed) not case-insensitively
   `application/json` → `Ok(None)` (body left undefined, never read).
2. There is a body when `transfer-encoding` is present, or `content-length` (trimmed) parses as a `u64`, or the
   bytes are non-empty. No body → `Ok(None)`.
3. More than 73,728 bytes → `TooLarge` (on the actual bytes).
4. Zero bytes (a declared empty body) → `Ok(Some({}))`.
5. Decode UTF-8 lossily (invalid sequences become U+FFFD). Skip leading space, tab, LF, CR; the first character must
   be `{` or `[`, else `Invalid`. Then parse as JSON (duplicate keys: last value wins); failure → `Invalid`.

`parse_admin_body(bytes)`: empty → `None`; else the JSON value, or `None` if it does not parse.

### 3.5 The evidence chain

#### 3.5.1 Gate

`submit_parsed(runtime, body)` and `complete_parsed(runtime, body)` both start:

1. Evidence off (`hunt_evidence.enabled` false) → `HUNT_EVIDENCE_NOT_COLLECTED` / `This server does not collect hunt
   evidence.` Nothing is read or stored.
2. Read the hunt by `runtime.session`. None, or its generation differs from `runtime.generation`, or its host is not
   `runtime.host_account` → `P2P_GENERATION_FENCED` / `The hunt generation no longer belongs to this host.`
3. The hunt is a hub → `HUNT_EVIDENCE_NOT_COLLECTED`.

The `EvidenceIntake` methods parse their `Json` text (unparsable → no body) and call these.

#### 3.5.2 The batch body

`{"schema":1,"batchId":…,"firstSequence":…,"lastSequence":…,"previousHash":…,"events":[…]}`. Each event is
`{"sequence":…,"monotonicMs":…,"type":…,"actorAccountId":…,"payload":{…}}` with `actorAccountId` optional. Field order
on the wire is the transport's, but any order parses. Constants: `BATCH_SCHEMA = 1`, `MAX_BATCH_EVENTS = 128`,
`MAX_PAYLOAD_BYTES = 8192`, `MAX_BATCH_EVENT_BYTES = 65_536`, `MAX_CHAIN_EVENTS = 262_144`,
`MAX_CHAIN_EVENT_BYTES = 67_108_864` (all `pub` in `evidence`).

`parse_batch(Option<&Value>) -> Result<EvidenceBatch, BatchProblem>`. `BatchProblem::{TooLarge, Invalid(&'static
str)}`; `into_error()` gives `GUARD_BATCH_TOO_LARGE` / `The Guard request body exceeds its bounded limit.` or
`GUARD_BATCH_INVALID` / `The Guard event batch is invalid.` (the `Invalid` text is a diagnostic, never shown). Rules,
checked in this order (only the TooLarge/Invalid distinction is observable):

1. The body is an object whose keys are all among the six above; otherwise Invalid.
2. `schema` is a safe integer equal to 1.
3. `batchId` is a string of 1-64 bytes, each ASCII alphanumeric or `-`.
4. `previousHash` is exactly 64 characters of `0-9a-f` (lower case only).
5. `firstSequence` is a non-negative safe integer ≥ 1; `lastSequence` a non-negative safe integer.
6. `events` is an array; empty → Invalid; more than 128 entries → **TooLarge**.
7. For each event at index i, in order: an object with keys only among the five; `sequence` a non-negative safe
   integer equal to `firstSequence + i`; `monotonicMs` a non-negative safe integer not below the previous event's;
   `type` a string of 1-64 bytes of ASCII alphanumerics and `_`; `actorAccountId` absent, or a string of 1-96 bytes
   of ASCII alphanumerics and `._:-` (present but not such a string, `null` included, is Invalid); `payload` an object
   whose canonical text is at most 8192 bytes; a type of `COMPLETION_REQUESTED` or `HUNT_ABORTED` (the terminal types)
   only as the last event. The event's own canonical text length in bytes is added to `event_bytes`.
8. `event_bytes` above 65,536 → **TooLarge**.
9. `lastSequence` equals the last event's sequence.

**The batch hash** (`batch_hash`): SHA-256 over the bytes of the ASCII string `rifts-guard-batch-v1` followed by one
LF (0x0A), immediately followed by the UTF-8 bytes of the canonical text (§3.0) of the **whole body object as
parsed** (all six fields, `previousHash` and every payload included). Written as 64 lower-case hex digits. It is the
value the next batch must name as `previousHash`. Two bodies that differ only in insignificant whitespace hash the
same; key order matters. The transport writes keys in a fixed order (`schema, batchId, firstSequence, lastSequence,
previousHash, events`; per event `sequence, monotonicMs, type, [actorAccountId,] payload`), and it never computes the
hash itself: it reads `batchHash` from the reply.

`GENESIS_HASH` is 64 `0` characters: the head before the first batch.

#### 3.5.3 Applying a batch

`apply_batch(chain: &ChainRecord, batch: &EvidenceBatch, roster: &[AccountId], now: UnixMillis) ->
Result<ChainUpdate, ChainRefusal>`. `roster` is every roster account of the hunt (admitted or not). In order:

1. The chain has a completion, or its encounter state is terminal (`COMPLETION_REQUESTED`, `ABORTED`) →
   `GUARD_CHAIN_FROZEN` / `The Guard chain of this hunt generation is closed.`, no finding.
2. `previousHash` ≠ the chain's head, or `firstSequence` ≠ `last_sequence + 1` → `GUARD_CHAIN_CONFLICT` / `The Guard
   batch does not continue this hunt generation's chain.`, finding `CHAIN_CONFLICT` with detail
   `expected=<last_sequence+1>:<head> got=<firstSequence>:<previousHash>`.
3. `event_count + events > 262,144` or `event_bytes + batch bytes > 67,108,864` → `GUARD_EVIDENCE_LIMIT` / `This hunt
   generation's Guard evidence limit is reached.`, finding `EVIDENCE_LIMIT` with detail `events=<chain event_count>`.
4. Otherwise the update. Findings, in this order, each with the event's sequence and `at = now`:
   - `MONOTONIC_REGRESSION` (detail `from=<chain last_monotonic_ms> to=<first event's monotonicMs>`) when the chain
     has a last monotonic time and the batch's first event is below it.
   - Then per event, in order: the event's type count in `event_types` goes up by one; the encounter state moves
     (below); a type of exactly `EVIDENCE_LOSS` gives `EVIDENCE_LOSS` (empty detail); an actor that is not in the
     roster gives `ACTOR_NOT_ROSTERED` (detail `actor=<actor>`); no actor on a type that **contains** `DAMAGE`, when
     the chain has not yet seen unattributed damage, sets that flag and gives `UNATTRIBUTED_DAMAGE` (empty detail;
     once per chain).
   - Encounter state, ordered `CREATED < SESSION_STARTED < WORLD_READY < ENCOUNTER_ACTIVE < BEHEMOTH_DEFEATED <
     COMPLETION_REQUESTED < ABORTED`: `SESSION_STARTED`, `WORLD_READY`, `ENCOUNTER_STARTED` or `DAMAGE` (exact),
     `BEHEMOTH_DEFEATED` move it forward to that state (never back); `COMPLETION_REQUESTED` and `HUNT_ABORTED` set
     `COMPLETION_REQUESTED` / `ABORTED` outright; other types leave it.
   - The new chain: revision + 1, `last_sequence` = `lastSequence`, head = `batch_hash`, `batch_count` + 1,
     `event_count` + events, `event_bytes` + batch bytes, `last_monotonic_ms` = last event's, `last_received_at` =
     now, `finding_count` + findings, disposition `OBSERVE_FLAGGED` once `finding_count` > 0 (it never returns to
     clear).
   - The batch record: session, generation, id, previous hash, hash, first and last sequence, event count and bytes,
     `event_types` = the distinct types of this batch sorted by byte order, `received_at` = now. Payloads are never
     stored.

A refusal with a finding is recorded by `finding_update(chain, code, detail, now)`: revision + 1, `finding_count` + 1,
disposition `OBSERVE_FLAGGED`, one finding with **no** sequence, no batch.

#### 3.5.4 Submit flow and idempotent resend

After the gate and `parse_batch`, up to 3 attempts:

1. Read the chain (none → a fresh chain for this session and generation with no completion mode, revision 0).
2. Look up a stored batch with this `batchId` for the session and generation. Found with the **same** hash → answer
   the duplicate reply (below), write nothing; this applies even when the chain has since closed. Found with another
   hash → `GUARD_BATCH_REPLAY` / `This Guard batch id was already used for other evidence.`
3. `apply_batch`. Success → `apply_chain(actor, update, expected = the revision read)`; if it reports written, log
   each finding and answer the batch reply; if not, next attempt. Refusal → if it carries a finding, write
   `finding_update` once (expected = the revision read; ignore "not written") and log its finding if written; then
   answer the refusal.

After 3 lost writes → `GUARD_CHAIN_CONFLICT` with the conflict message. The actor for chain writes is
`Actor::HostRuntime { session, host }` of the runtime.

Finding log (warn): `[HuntGuardObserve] would refuse <code> session=<s> generation=<g> sequence=<n or ->` followed by
` <detail>` when the detail is non-empty.

Replies (exact templates; `<sid>` is the session id JSON-quoted):

- batch: `{"accepted":true,"batchHash":"<new head>","sessionId":<sid>,"generation":<g>,"lastSequence":<chain
  last_sequence>,"encounterState":"<state>","disposition":"<disposition>","duplicate":false,"observeOnly":true}`
- duplicate: the same with `batchHash` and `lastSequence` from the **stored batch**, `encounterState` and
  `disposition` from the current chain, `"duplicate":true`.

`accepted` must come first and `batchHash` second: the transport looks for the substring `"accepted":true` and takes
the first `"batchHash":"<64 hex>"` match.

#### 3.5.5 Completion

Body `{"finalSequence":…,"finalHash":…}` and nothing else (`parse_completion`): an object with keys only among the
two; `finalSequence` a non-negative safe integer ≥ 1; `finalHash` 64 lower-case hex. Any failure →
`GUARD_COMPLETION_INVALID` / `The Guard completion request is invalid.`

Flow, up to 3 attempts: read the chain; none → `GUARD_COMPLETION_INVALID` / `The Guard chain has not requested
completion.`. Then `apply_completion(chain, request, now) -> Result<Option<ChainUpdate>, GuardError>`:

1. Already completed: same sequence and hash → `None` (answer the reply, write nothing); else
   `GUARD_COMPLETION_CONFLICT` / `The Guard completion does not match this hunt generation's chain.`
2. Completion mode `NO_COMPLETION` → `GUARD_COMPLETION_INVALID` / `This hunt generation has no Guard completion.`
3. Encounter state not `COMPLETION_REQUESTED` (an `ABORTED` chain included) → `GUARD_COMPLETION_INVALID` / `The Guard
   chain has not requested completion.`
4. Sequence ≠ `last_sequence` or hash ≠ head → `GUARD_COMPLETION_CONFLICT`.
5. Update: revision + 1, completion = `{final_sequence, final_hash, at: now}`; state, disposition and counts unchanged.

On a written update, log (info) `[HuntGuardObserve] completion session=<s> generation=<g> lastSequence=<n>
findings=<finding_count> disposition=<disposition>` and answer. After 3 lost writes → `GUARD_FINALIZATION_BUSY` /
`The Guard completion is busy; retry shortly.`

Reply: `{"accepted":true,"sessionId":<sid>,"generation":<g>,"finalSequence":<n>,"finalHash":"<h>","disposition":"<d>",
"findingCount":<c>,"observeOnly":true,"progressionApplied":false,"rewardsApplied":false}`, with sequence and hash from
the completion (or the chain's last sequence and head if none). The transport requires `"accepted":true` and
`"progressionApplied":false`.

#### 3.5.6 Other chain operations

- `open_chain(session, generation, completion_mode)`: if a chain row exists, nothing. Else write a fresh chain
  (`last_sequence` 0, `GENESIS_HASH`, `CREATED`, `OBSERVE_CLEAR`, the mode parsed with `CompletionMode::parse`, unknown
  → none) at revision 1 with expected revision 0, actor `Actor::System("guard")`; a lost write is not an error.
- `chain_status`: the stored chain.
- `verdict(session, generation)`: `Some(HuntVerdict::OBSERVED)` when the chain has a completion, else `None`.
  `HuntVerdict::OBSERVED` is both flags false; `to_json` → `{"progressionApplied":<b>,"rewardsApplied":<b>}`.
- `reward_receipt`: always `Ok(None)`. `reward_policy_status`: always `NotReady { reason: "ANTICHEAT_MODULE_ABSENT" }`.
- `record_shadow_comparison`: always `HUNT_EVIDENCE_NOT_COLLECTED`; the key is never read or logged.

### 3.6 Self reward and shadow routes

`self_reward(account, session_text)`: `session_text` that does not parse as a `SessionId`, an unknown hunt, or an
account not anywhere in the hunt's roster (admitted or not) → `P2P_SESSION_NOT_FOUND` / `Hunt reward receipt not
found.`; a member → `HUNT_REWARDS_UNAVAILABLE` / `This server grants no player-hosted hunt rewards.` A store error is
`Internal`. The shadow route answers `HUNT_EVIDENCE_NOT_COLLECTED` to anyone (after the body parse of §3.3).

### 3.7 Traffic bounds

`TrafficLimit { max_requests, max_bytes, window_ms }` (all `u64`). Constants:

| Constant | Value |
| --- | --- |
| window | 60,000 ms |
| `HUNT_GUARD_HOST_TRAFFIC_LIMIT` | 180 requests, 8,388,608 bytes, 60,000 ms |
| `HUNT_GUARD_SESSION_TRAFFIC_LIMIT` | 120 requests, 4,194,304 bytes, 60,000 ms |
| `HUNT_GUARD_REQUEST_BUDGET` | `RequestBudgetLimit { max_in_flight_requests: 64, max_in_flight_bytes: 1_048_576 }` |
| local limiter capacity | 4,096 keys (`LocalTrafficLimiter::default()`) |
| bucket cache | 1,024 entries, 10,000 ms |

A limit is valid when each of its three numbers is in `1..=2^53−1`.

**Request budget** (`RequestBudget::new(limit)`, `try_acquire(bytes) -> Option<BudgetPermit>`, `snapshot() -> (u64,
u64)`; `Clone` shares the counters): refuse when `bytes > 2^53−1`, or requests in flight ≥ the maximum, or bytes in
flight + `bytes` > the maximum; otherwise count it. Dropping the permit releases exactly its one request and its bytes
(saturating). No queue.

**The charge** for a host account and a session: keys `guard-host:<account>` and `guard-session:<session id>`.

- **Local** (`P2P_DISTRIBUTED_TRAFFIC_LIMITER` not true), `LocalTrafficLimiter::consume(key, bytes, limit, now) ->
  bool`: false without change if the key is empty, `bytes` > 2^53−1, the limit invalid or `bytes` > `max_bytes`. A
  key's window restarts at `now` when it has none, when `now − start ≥ window_ms`, or when `now` is before its start.
  The key is touched at `now`. Refuse (keeping the touched window) when its requests ≥ `max_requests` or its bytes +
  `bytes` > `max_bytes`; else count one request and the bytes, then evict least-recently-touched keys while more than
  the capacity are held. The host key is charged first, then the session key; a session refusal does **not** give back
  the host's charge. Either refusal → `RateLimited`. `new(max_keys)`, `len()`, `is_empty()`.
- **Distributed:** build one reservation per key (`TrafficScope::Host`, `TrafficScope::Session`). A key that fails
  validation (empty, longer than 256 UTF-16 units, bytes > 2^53−1, invalid limit, bytes > `max_bytes`) → `RateLimited`
  without touching the store. Else `GuardStore::reserve_traffic(Actor::System("guard"), host, session)`: `Ok(true)` →
  `Accepted`, `Ok(false)` → `RateLimited`, any error → `Unavailable` (never a local fallback).

Reservation: `window_start = floor(now / window_ms) * window_ms` (Euclidean); `window_id = "<scope>:<start>:<hex>"`
where `<scope>` is `host`/`session`, `<start>` the decimal start and `<hex>` the SHA-256 lower-case hex of the UTF-8
text `<scope>` NUL `<key>` NUL `<start>`; `expires_at = start + 2 * window_ms`; plus `bytes`, `max_requests`,
`max_bytes`. The key itself is never stored. `TrafficReservation { window_id, scope, window_start, expires_at, bytes,
max_requests, max_bytes }` (all `pub`).

`reserve_traffic` semantics (both stores): in one transaction, delete windows whose `expires_at ≤ host.window_start`;
then the host window (created at 0 requests, 0 bytes with the reservation's scope, start and expiry when missing) is
accepted when its requests < `max_requests` and bytes + `bytes` ≤ `max_bytes`; then the session window the same way
(when both ids are equal, it sees the host's new counts). Both accepted → both counted and stored, `true`; otherwise
neither is changed, `false`.

`HuntOperation::{EventBatch, Completion, Shadow, Lifecycle}`, `of(path)`: a path containing `event-batches`, else
`completion`, else `shadow-comparisons`, else lifecycle. `counts_against_guard_traffic()` is true for all but
`Lifecycle` (a hunt's lease renewals must never be starved by its own evidence).

**Rate-limit bucket** (`GuardHooks::rate_limit_bucket(request)`): take the `x-mystic-host-capability` header; absent
or not 43 characters of `[A-Za-z0-9_-]` → `None` (nothing cached). Otherwise key a cache by the value's SHA-256 hex: a
cached answer younger than 10,000 ms is returned as is (`None` answers included). Else
`HuntSessions::capability_by_digest`: a capability not revoked whose `expires_at` is after now → `host:<capability
id>`; anything else, a lookup error included, → `None`. Store the answer with its time; when the cache already holds
1,024 entries, first drop the oldest inserted. Re-storing an existing digest updates it in place without moving it.
The header value is never stored or logged.

### 3.8 Host write authorization

`kind_name(HostWriteKind) -> &'static str`: `XP`, `MASTERY`, `OBJECTIVES`, `ESCALATION`, `COOLDOWN`, `BOUNTY`,
`SLAYERS_PATH`, `REFORGE`, `RANK_CONFIRM`, `LOADOUT_UNLOCK`, `HUNT_PASS_XP`, `ITEM_SAVE`.

Write kinds allowed: a hunt allows every kind but `REFORGE` (and no kind given); a Ramsgate hub allows every kind
but `XP` (a kind must be given); a Training hub every kind but `XP` and `REFORGE`.

`host_write_denial(runtime, hunt: Option<&HuntView>, target, kind: Option<HostWriteKind>,
presence: Option<&[AccountId]>, check_kind: bool) -> Option<WriteDenial>` (`WriteDenial` is public through the
function; `as_str`, `is_policy`):

1. No hunt, or its session or generation differs from the runtime's → `HUNT_NOT_FOUND`.
2. Not running → `HUNT_NOT_RUNNING`. Running is state `RUNNING` or `DRAINING`, or `SUSPECT` with pre-suspect state
   `RUNNING`.
3. A hub: `check_kind` and the hub does not allow the kind → `WRITE_KIND_NOT_ALLOWED_IN_HUB`; the target is neither
   the hunt's host nor in `presence` (none = empty) → `MEMBER_NOT_PRESENT`; else allowed.
4. A hunt: `check_kind` and the kind not allowed → `WRITE_KIND_NOT_ALLOWED`; target not in the roster →
   `NOT_A_MEMBER`; target not the host and its roster entry not admitted → `MEMBER_NOT_ADMITTED`; else allowed.

`is_policy` is true for the two write-kind reasons only.

`write_refusal(message)` → `Reply { 403, {"code":"P2P_HOST_WRITE_DENIED","message":<q>,"payload":null} }`.

`GuardHooks::authorize_host_write(runtime, kind, target, options)` (`None` lets the write through):

1. `P2P_HOST_WRITES` off → refuse `Host writes are switched off.`; unless `probe` or `parity`, log warn
   `[HostWrite] refused kind=<KIND> target=<t> reason=HOST_WRITES_OFF`.
2. `parity` in enforce mode → refuse `This host may not do a dedicated server's work here.` (no log).
3. Read the hunt by the runtime's session (error → seam error); for a hub, read its present accounts for its
   generation (error → seam error).
4. `parity`: refuse `This host may not write this player's state.` when the identity check (`check_kind` false)
   denies; else allow. Never logs.
5. Otherwise the full check (`check_kind` true). If the reason is a policy one, not a probe and the mode is observe:
   run the identity check; if it passes, log warn `[HostPolicyObserve] would refuse write kind=<KIND> target=<t>
   session=<s> reason=<REASON>[HUB]` and allow; if it fails, its reason replaces the policy one.
6. A remaining reason → unless `probe`, log warn `[HostWrite] refused kind=<KIND> target=<t> session=<s>
   reason=<REASON>[HUB]`; refuse `This host may not write this player's state.`

`GuardHooks::record_host_write(runtime, write)`: read the hunt by the runtime's session (error → seam error). None or
another generation → log warn `[HostWrite] record without authorize kind=<KIND> target=<t>` and return `Ok`. Else log
info `[HostWrite] kind=<KIND> target=<t> host=<hunt host or empty> session=<s> generation=<g>[REALM][HUB]
summary=<summary text, first 400 UTF-16 units>` and store a `HostWriteRecord` (kind, target, host, session,
generation, realm, `session_kind` = `REALM_HUB` for a hub, `hub_kind`, summary, `at` now) with the runtime's actor. A
store failure is logged at error `[HostWrite] tag not stored kind=<KIND> target=<t> session=<s>: <error>` and is not
an error. The realm and hub tags come from the stored hunt, never the request.

`host_policy_refuses()` → enforce mode. `note_would_refuse(what, detail)` → warn
`[HostPolicyObserve] would refuse <what> <detail>`.

### 3.9 Inventory transactions and the fence

`inventory_body_denial(runtime, hunt, account, character_id, body: Option<&Value>, presence) -> Option<&'static str>`
(`inventory_denial` is the same with no body):

1. No hunt, or another session or generation, or state not exactly `RUNNING`/`DRAINING` (a `SUSPECT` hunt is refused
   here, unlike §3.8) → `HUNT_NOT_RUNNING`.
2. A hub: account neither the host nor present → `PARTICIPANT_NOT_ADMITTED`; body `accountId` present, not `null`, and
   not the account's string → `ACCOUNT_MISMATCH`; else allowed.
3. A hunt: no admitted roster entry for the account → `PARTICIPANT_NOT_ADMITTED`; the `accountId` check →
   `ACCOUNT_MISMATCH`; the member's frozen character (from `frozen_characters`) not equal to `character_id`, or none →
   `CHARACTER_MISMATCH`.

`loot_refusal(code)` → `Reply { 403, {"code":<q code>,"message":"Player-hosted hunt loot grant is not
authorized.","payload":null} }`.

`GuardHooks::authorize_inventory(runtime, transaction)`; `InventoryTransaction { user, character_id, transaction_id,
body: Option<Json> }`:

1. Read the hunt; for a hub, its present accounts (errors → seam errors). Parse `body` (unparsable → none).
2. Denied by `inventory_body_denial` → log warn `[P2PInventory] rejected code=<reason> account=<user> session=<s>
   generation=<g>` and refuse `loot_refusal("P2P_LOOT_GRANT_DENIED")`.
3. If the body adds the draft token (§3.10): ask `IslesCore::accepted_inventory_replay(transaction)` (error → false).
   Not a replay → read `IslesCore::stacked_items(character_id)` and decide early (`decide_draft_premium` with the
   balance; a failed read or an unreadable list is no balance).
4. With a body: the grant policy with session flags (`no_completion` = completion mode `NO_COMPLETION`; `realm_hub`
   = a hub), the catalogue `IslesCore::host_grant_catalog()` or `NoGameData`, and the draft allowance = replay, or the
   early decision allowed it. On a denial, tags = `account=<user> session=<s> generation=<g> transaction=<id>
   classes=<c1,c2,…> denied=<summary>`, then ` draftPremium=<reason>` if an early decision exists and refused, then
   `[HUB]`. Observe → warn `[HostPolicyObserve] would refuse grant <tags>` and go on. Enforce → warn
   `[HostGrantDeny] <tags>` and refuse `{ 403, {"code":"P2P_HOST_GRANT_DENIED","message":"A player host may not grant
   this item class.","payload":{"classes":[<q class>,…]}} }`.
5. Authorize with a fence. Fence tags = `account=<user> character=<character> session=<s> generation=<g>
   transaction=<id>[REALM][HUB]` (from the hunt read in step 1). The fence carries the draft decision only when the
   body adds the token and the transaction is not a replay.

The fence (`InventoryFence`):

- `on_grant(grant)` and `on_grant_in(grant, reads)`: re-read the hunt (error → refuse `P2P_LOOT_GRANT_DENIED`) and, for
  a hub, its presence (error → same). The body checked is the authorized body, or else the grant text parsed. A
  denial → refuse `P2P_LOOT_GRANT_<reason>`. Then, when the fence carries the draft token: decide again from
  `reads.stacked_items(character)` inside the transaction (`on_grant_in`), or keep the early decision (`on_grant`).
  Allowed → remember it with the UTC day (`YYYY-MM-DD` of now) and pass. Else reason = the decision's, or
  `NOT_A_GRANT` when there is none: enforce → refuse `P2P_LOOT_GRANT_DRAFT_PREMIUM_<reason>`; observe → warn
  `[HostPolicyObserve] would refuse draft premium <fence tags> item=TOKEN_DAILY_CHALLENGE_DRAFT_PREMIUM
  reason=<reason> at=transaction<balance tags>` and pass.
- A refusal with code C logs warn `[P2PInventory] transaction denied code=C <fence tags>`; when C starts with
  `P2P_LOOT_GRANT_DRAFT_PREMIUM_` it also logs warn `[HostGrantDeny] account=<user> session=<s> generation=<g>
  transaction=<id> item=TOKEN_DAILY_CHALLENGE_DRAFT_PREMIUM quantity=1 reason=<rest of C> at=transaction`; then
  `Err(loot_refusal(C))`.
- `report(Applying)` → info `[P2PInventory] applying <fence tags>`; `report(Accepted)` → info `[P2PInventory]
  transaction accepted <fence tags>`, then, once, if a draft grant was remembered: info `[HostGrantAllow]
  account=<user> session=<s> generation=<g> transaction=<id> item=TOKEN_DAILY_CHALLENGE_DRAFT_PREMIUM quantity=1
  reason=<reason> day=<day><balance tags>`; `report(Failed)` → error `[P2PInventory] transaction failed <fence tags>`.

`<balance tags>` is ` balance=<before>-><after> cap=1` when the decision read a balance, else nothing.

### 3.10 Host grant policy and the draft token

Pure functions over the JSON body of `POST /inventory`. Item lists are `addStackedItems`, `addInstancedItems`,
`removeStackedItems`, `removeInstancedItems`; a list that is not an array is empty; an item without a string
`catalogId` is skipped. `quantity` is read only as a JSON number.

**Canonical id:** JS-trim the id; if its first 12 bytes are case-insensitively `id_currency_`, the id is `CURRENCY_`
+ the rest, all upper-cased; otherwise the trimmed id.

**`classify(catalog_id, catalog) -> Option<DenyClass>`** (`DenyClass::{PremiumCurrency, HuntPass, Cosmetic,
Entitlement}`, `as_str` → `PREMIUM_CURRENCY`, `HUNT_PASS`, `COSMETIC`, `ENTITLEMENT`). Empty after trim → none. With
`id` the canonical id, `UP` its upper case, and tags = `catalog.tags_of(id)` lower-cased (none = empty), the first
match wins:

1. **PREMIUM_CURRENCY:** `UP` starts with `CURRENCY_PLATINUM` or `DEBUG_`; or `id` is one of the 13 Platinum ids
   (`CURRENCY_PLATINUM_CS`, `CURRENCY_PLATINUM_UNIV`, `CURRENCY_PLATINUM_PLATFORM_BONUS`,
   `CURRENCY_PLATINUM_EPIC_BONUS`, `CURRENCY_PLATINUM_XBOX_BONUS`, `CURRENCY_PLATINUM_PSN_BONUS`,
   `CURRENCY_PLATINUM_SWITCH_BONUS`, `CURRENCY_PLATINUM`, `CURRENCY_PLATINUM_PLATFORM`, `CURRENCY_PLATINUM_EPIC`,
   `CURRENCY_PLATINUM_XBOX`, `CURRENCY_PLATINUM_PSN`, `CURRENCY_PLATINUM_SWITCH`); or tag `platinum`.
2. **HUNT_PASS:** `UP` is `CURRENCY_SEASONAL_COIN`, `CURRENCY_REWARDCACHE`, `CURRENCY_PRESTIGE` or
   `GRANTPROGRESSITEM_HUNTPASS`; or `UP` is `CURRENCY_S` + one or more ASCII digits + `_COIN` or `_DAILY` exactly; or
   starts with `TOKEN_HUNTPASS`, `TOKEN_HUNT_PASS` or `CONTAINER_CORE_PRESTIGE_`; or ends with `_DRAFT_PREMIUM`; or
   `catalog.is_prestige_reward(id)`; or tag `seasonal_currency`.
3. **COSMETIC:** `UP` is `CHANGE_APPEARANCE_TOKEN`, `CURRENCY_TOKEN_TRANSMOG` or `CONTAINER_CORE_SLAYERPACK`; or starts
   with one of `DYE_`, `EM_`, `EJ_`, `TITLE_`, `BNC_`, `BN_`, `AC_`, `GD_`, `CC_`, `TY_`, `MP_`, `CONTAINER_PROMO_`,
   `CONTAINER_1P_`, `CONTAINER_CORE_FOUNDER_`; or any tag in `dye`, `emote`, `emoji`, `title`, `bannercustomization`,
   `banner`, `accessory`, `glider`, `toy`, `companion`, `stylehunter`, `founder`, `transmogtoken`.
4. **ENTITLEMENT:** `catalog.is_entitlement_name(lower-case id)`; or `id` (case as is) is one of `ent`, `pass`,
   `eventpass`, `minipass`, `exchange_slot` + `_` + a non-empty tail of `[a-z0-9_.-]`, or a non-empty such tail +
   `_premium`.

**Exemptions** (an item matching one is never a violation):

- In `addStackedItems` only: a claim-funded coin (`catalogId` exactly `CURRENCY_SEASONAL_COIN`, quantity a whole safe
  integer in 1..=1000); a prestige-funded grant (`catalog.is_prestige_gated(catalogId)` on the raw id, quantity a
  positive safe integer, and the id classifies as HUNT_PASS); the draft token, only when the allowance is set, the
  body's `addStackedItems` names `TOKEN_DAILY_CHALLENGE_DRAFT_PREMIUM` exactly once, and this item is that id
  (exact case) with quantity exactly 1.
- In `removeStackedItems` only: a Platinum spend (canonical id `CURRENCY_PLATINUM` or one of the 13 ids).
- In either add list: a starter cosmetic (one of `EM_INTRO_GENERIC_001`, `EM_INTRO_GENERIC_002`,
  `EM_INTRO_GENERIC_003`, `TITLE_BLANK`, `EJ_SYSTEM_TYPING`, `EJ_PLAYER_BOSUN_JOY`, `EJ_PLAYER_KAT_PRAY`,
  `EJ_PLAYER_MOYRA_MUSCLE`, `EJ_PLAYER_GREGARIO_QUESTION`, `EJ_PLAYER_MIDDLEMAN_HMM`, `EJ_PLAYER_PRIYANI_WINK`,
  `EJ_PLAYER_ZAI_ANGRY`, `EJ_PLAYER_ZAI_SWEAT`, exact case, `HOST_STARTER_COSMETIC_IDS` in this order) with no
  `quantity` or a quantity of exactly 1.

**`violations(body, catalog, allowances) -> Vec<Violation>`**: scan the lists in the order add-stacked,
add-instanced, remove-stacked, remove-instanced, items in order; an item that is not exempt and classifies is a
violation `{ catalog_id: <raw id>, denied_class, field }` (`GrantField` `as_str` = the list name), except that in the
two remove lists only PREMIUM_CURRENCY and ENTITLEMENT count.

**`grant_denial(session, body, catalog, allowances) -> Option<GrantDenial>`**: when `session.no_completion &&
!session.realm_hub` and the body is an object with at least one non-empty item list → classes `["NO_REWARD_SESSION"]`,
no violations, summary `any grant or spend`. Else none when there are no violations; otherwise classes = distinct
class names in order of first appearance, summary = the first 16 violations as `<id>(<CLASS>,<field>)` joined by `,`,
followed by `,+<n>` when n more exist.

`GrantSession { no_completion, realm_hub }`, `Allowances { draft_premium }` (both `Default`). `NoGameData`: no tags,
nothing prestige or entitlement.

**Draft token:** `HOST_DRAFT_PREMIUM_ID = "TOKEN_DAILY_CHALLENGE_DRAFT_PREMIUM"`,
`HOST_DRAFT_PREMIUM_BALANCE_CAP: i128 = 1`. `body_adds_host_draft_premium` = an `addStackedItems` item with exactly
that id.

`stacked_balance_of(stored: Option<&str>, id) -> Option<i128>`: no stored text → `Some(0)`; text that is not a JSON
array → `None`; otherwise sum the quantities of items with that exact `catalogId` (other entries, `null`s included,
are skipped); a matching item whose quantity is not a non-negative safe integer → `None`.

`decide_draft_premium(body, balance_before: Option<i128>) -> Option<DraftPremiumDecision>`: `None` if the body does
not add the token. More than one such add, or the one not with quantity exactly 1 → refused
`ONE_WHOLE_TOKEN_PER_TRANSACTION` (no balance). No balance → refused `BALANCE_UNREADABLE`. Else `after = before −
(sum of positive safe-integer quantities of the token in removeStackedItems) + 1`; allowed with
`WITHIN_BALANCE_CAP` when `after ≤ 1`, else refused `OVER_BALANCE_CAP`; balance `(before, after)`.
`balance_tags()` as §3.9.

### 3.11 Admin operations

`RewardRollout { version: u64, tier0_shadow_enabled: bool, tier1_grants_enabled: bool }` (`Copy`, `Default` =
`DEFAULT_REWARD_ROLLOUT` = version 0, shadow true, grants false); `to_json` →
`{"version":<n>,"tier0ShadowEnabled":<b>,"tier1GrantsEnabled":<b>}`; `from_value(Option<&Value>)` accepts an object
with `version` a non-negative safe integer and the two fields JSON booleans, else `None`.

Weakening codes: `["TIER1_REWARD_GRANTS_ENABLE"]` when grants go from off to on, else none.
Reason = the body's `reason` if a string, JS-trimmed, first 500 UTF-16 units; must be at least 8 UTF-16 units.

Error replies are `{"error":{"code":<q>,"message":<q>}<extra>}`.

**`PATCH /admin/v1/operations/reward-rollout`** (`set_reward_rollout(admin_key, body, ip)`):

1. Valid request: `rewardRollout` per `from_value`; `expectedVersion` = JavaScript `Number(value)` that is a
   non-negative safe integer (so `"4"` → 4, `null` → 0, `true` → 1, `[3]` → 3, missing → invalid); the reason rule;
   `breakGlassTotp` a string if present. Else 400 `ADMIN_VALIDATION` / `A complete reward rollout, expectedVersion,
   and audit reason of at least 8 characters are required.`
2. `request_id` = a new UUID. Read the policy (`OperationsPolicyPort::policy`).
3. Policy version ≠ expected → 409 `ADMIN_POLICY_CONFLICT` / `The operations policy changed. Refresh it before trying
   again.` with extra `,"policy":<policy json>`.
4. New rollout version ≤ the current rollout's → 409 `ADMIN_REWARD_ROLLOUT_CONFLICT` / `The reward rollout version
   must advance beyond the current central version.` with `,"policy":…`.
5. Weakening codes present: stepped up only if `breakGlassTotp` is given and
   `IslesCore::verify_operations_step_up(admin_key, code)` says so (its error propagates). Not stepped up → append
   audit `operations.policy.weakening_rejected` (old = the policy json, new =
   `{"rewardRollout":<rollout>,"weakeningCodes":[…]}`) and answer 403 `ADMIN_POLICY_STEP_UP_REQUIRED` / `A fresh
   operations authenticator code is required for this safety reduction.` with `,"policy":<json>,"weakeningCodes":[…]`.
6. `set_reward_rollout(rollout, admin_key, expected)`; `None` → the step 3 conflict.
7. Append audit `operations.reward_rollout.update` (old = the policy before, new = the policy after with
   `"weakeningCodes":[…]` added as its last field, request id as step 2).
8. 200 `{"policy":<after json>,"requestId":<q>}`.

Audit rows (`AuditEntry { id, actor, action, old_state: Json, new_state: Json, reason, ip, request_id, created_at }`)
take a fresh UUID each, `actor` = the admin key id, `created_at` = ISO now. "Added as last field" is textual: drop
the policy text's trailing `}` and append `,"weakeningCodes":[…]}` (no comma when the object was `{}`); text not
ending in `}` becomes `{"weakeningCodes":[…]}`.

**`PATCH /admin/v1/operations/alerts/:alertId/ack`** (`acknowledge_alert(admin_key, alert_id, body, ip)`):

1. The id JS-trimmed is 8-128 bytes of ASCII alphanumerics and `-`, and the reason rule holds; else 400
   `ADMIN_VALIDATION` / `A valid alert ID and an audit reason of at least 8 characters are required.`
2. `request_id` = new UUID; `at` = now. `GuardStore::acknowledge_alert(Actor::Admin { key_id: admin_key }, id, at)`
   false → 404 `ADMIN_ALERT_NOT_OPEN` / `The alert is missing or has already been acknowledged.` (a store error →
   `INTERNAL`).
3. Audit `operations.alert.acknowledge`, old `{"alertId":<q>,"status":"OPEN"}`, new
   `{"alertId":<q>,"status":"ACKNOWLEDGED","acknowledgedAt":"<iso>"}`, `created_at` = the same ISO time.
4. 200 `{"alertId":<q>,"status":"ACKNOWLEDGED","acknowledgedAt":<q iso>,"requestId":<q>}`.

### 3.12 Host route census

`HOST_ROUTE_CENSUS_WINDOW_MS = 60_000`, `HOST_ROUTE_CENSUS_MAX_ROWS = 200`.

- `status_class(code)`: `Nxx` for 100-599 (N = code / 100), else `other`.
- `normalize(sample) -> HostRouteKey { method, route, status, session_kind, hub_kind: Option<String> }`: method kept if
  one of `GET HEAD POST PUT PATCH DELETE OPTIONS` (exact), else `OTHER`; route kept when 1-160 UTF-16 units, else
  `(no route)`; status class; a kind label (session or hub) is kept when it is 1-32 bytes of `A-Z` and `_`, else
  session kind `unknown` and hub kind none.
- `route_of(pattern: Option<&str>, base_url) ->` `base_url + pattern`, or `(no route)`.
- `aggregate(samples)` counts per normalized key. `summarize(rows, window_seconds, max_rows)`: drop zero counts; none
  left → `None`; sort by count descending, ties by the key text `method|route|status|session_kind|hub_kind` (empty for
  no hub) in byte order; `total` = sum of all; keep the first `max_rows`, `omitted_rows` = how many were cut.
- `HostRouteSummary::to_json` → `{"windowSeconds":<js number>,"total":<n>,"routes":[{"method":<q>,"route":<q>,
  "status":<q>,"sessionKind":<q>,"hubKind":<q or null>,"count":<n>},…],"omittedRows":<n>}`.
- `HostRouteCensus::new(enabled)` (window 60,000, 200 rows), `with_window(enabled, window_ms, max_rows)`, `enabled()`,
  `window_ms()`, `record(&sample)` (counts only when enabled), `peek()` (the summary so far, window seconds =
  `window_ms / 1000`), `flush() -> Option<String>`: the line `[HostRoute] summary <to_json>` when there was traffic,
  then clears.

On `Guard`: the `census` hook and `record_census` record; `flush_census` logs the flushed line at info;
`Lifecycle::stop` flushes. Example the tests hold byte for byte (one line):

```text
[HostRoute] summary {"windowSeconds":60,"total":3,"routes":[{"method":"GET","route":"/inventory/:userId","status":"2xx","sessionKind":"unknown","hubKind":null,"count":2},{"method":"POST","route":"/progression/:userId/:track","status":"4xx","sessionKind":"REALM_HUB","hubKind":"RAMSGATE","count":1}],"omittedRows":0}
```

### 3.13 Lifecycle, hooks and the stub

`Guard` as `Lifecycle`: `runtime_safety_issues` → `Ok(vec![])`; `startup_summary(_)` →
`{"antiCheatLoaded":false,"hostPolicy":"<observe|enforce>","huntEvidence":<bool>,"trustTier":<0|1>}`; `start(_)` logs
§3.1's lines; `stop()` flushes the census; `tables()` → `TABLES`; `account_deletion_steps()` → three `Delete` steps,
in order: `rifts_host_writes`.`target`, `rifts_host_writes`.`host`, `rifts_hunt_xp_summaries`.`member`.
As `GuardHooks`: `service_roles` → `[]`; `guard_manifest_roles` → `GUARD_MANIFEST_ROLES`; the rest per §3.7-§3.9,
§3.12. As `TrafficLimits`: §3.7's charge. As `GuardOperations`: §3.11.

`Stub`: `tables` → `TABLES`, `account_deletion_steps` → `[]`, `service_roles` → `[]`, `guard_manifest_roles` →
`GUARD_MANIFEST_ROLES`; every other method answers `Unimplemented(Component::Guard, <method name>)`, the name being
the Rust method name (`runtime_safety_issues`, `startup_summary`, `start`, `stop`, `census`, `rate_limit_bucket`,
`authorize_inventory`, `authorize_host_write`, `record_host_write`, `host_policy_refuses`, `note_would_refuse`,
`submit_evidence`, `complete`, `verdict`, `reward_receipt`, `record_shadow_comparison`, `charge`,
`set_reward_rollout`, `acknowledge_alert`), through `SeamError` or `GuardError` as the trait returns.

### 3.14 Ports

- `HuntSessions` (§2.3) is realms' (`RealmsHuntSessions`); `NoHuntSessions` answers none, empty, none (and the default
  `resolve_capability`).
- `OperationsPolicyPort`: `policy() -> BoxFuture<Result<PolicySnapshot, SeamError>>`; `set_reward_rollout(rollout,
  actor: String, expected_version: u64) -> BoxFuture<Result<Option<PolicySnapshot>, SeamError>>` (writes only at the
  expected version and bumps it; `None` on conflict); `append_audit(AuditEntry) -> BoxFuture<Result<(), SeamError>>`.
  `PolicySnapshot { version: u64, reward_rollout: RewardRollout, json: Json }`.
- `MemoryOperations` (`Default`, `audit() -> Vec<AuditEntry>`): starts at version 0 with the default rollout; its
  policy json is `{"version":<v>,"p2pEmergencyStop":false,"rewardRollout":<rollout json>,"coopTransport":"ICE"}`;
  set bumps the version by one.
- `CoreOperationsPolicy`: `policy` reads `IslesCore::operations_policy()`: version = its `version` when a non-negative
  safe integer, else 0; rollout = `from_value(rewardRollout)` or the default; json = the text as given. If the core
  read fails, the in-memory fallback's policy. `set_reward_rollout` and `append_audit` go to an in-memory
  `MemoryOperations` (risk 5).
- `Clock`, `SystemClock`, `ManualClock`, `GuardLog`, `StderrLog` (writes `info <line>` etc. to standard error),
  `MemoryLog`: §2.3, §3.0. `iso_time(UnixMillis) -> String`: §3.0.
- `IslesCore` methods Guard calls: `host_grant_catalog()`, `stacked_items(character_id)`,
  `accepted_inventory_replay(transaction)`, `verify_operations_step_up(admin_key, code)`, `operations_policy()`.

### 3.15 Stores

`GuardStore` (`Send + Sync`), each method one transaction:

| Method | Contract |
| --- | --- |
| `record_host_write(&self, &Actor, &HostWriteRecord) -> Result<(), StoreError>` | append |
| `reserve_traffic(&self, &Actor, host: &TrafficReservation, session: &TrafficReservation) -> Result<bool, StoreError>` | §3.7 |
| `chain(&self, &SessionId, generation: u64) -> Result<Option<ChainRecord>, StoreError>` | read |
| `apply_chain(&self, &Actor, &ChainUpdate, expected_revision: u64) -> Result<bool, StoreError>` | `false`, writing nothing, when the stored revision (0 for no row) ≠ expected, or when the update's batch id already exists for the session and generation; else write the chain (insert or update guarded by the expected revision; a concurrent insert or update that loses is `false`), the batch, and the findings in order, `true` |
| `batch(&self, &SessionId, u64, batch_id: &str) -> Result<Option<BatchRecord>, StoreError>` | read |
| `findings(&self, &SessionId, u64) -> Result<Vec<FindingRecord>, StoreError>` | in insertion order |
| `put_xp_summary(&self, &Actor, &XpSummaryRecord) -> Result<(), StoreError>` | upsert by (session, member); `ttl_at = created_at + XP_SUMMARY_TTL_MS` (saturating) |
| `insert_alert(&self, &Actor, &AlertRecord) -> Result<(), StoreError>` | insert if absent; a retried insert keeps the first row |
| `acknowledge_alert(&self, &Actor, alert_id: &str, at: UnixMillis) -> Result<bool, StoreError>` | only an existing, unacknowledged alert: set `acknowledged_at` and `acknowledged_by` (the actor's name: admin key id, account id, host account id of a runtime, or the system task name) |

Records (all fields `pub`, `Debug, Clone, PartialEq, Eq`): `HostWriteRecord { kind, target, host, session,
generation, realm: Option<RealmId>, session_kind: Option<String>, hub_kind: Option<String>, summary: Json, at }`;
`TrafficWindowRecord { window_id, scope, window_start, requests, bytes, expires_at }`; `ChainRecord { session,
generation, revision, completion_mode, last_sequence, head_hash, encounter_state, disposition, finding_count,
batch_count, event_count, event_bytes, last_monotonic_ms: Option<u64>, event_types: BTreeMap<String, u64>,
has_unattributed_damage, last_received_at: Option<UnixMillis>, completion: Option<CompletionRecord> }` with
`ChainRecord::new(session, generation, completion_mode)` (revision 0, sequence 0, `GENESIS_HASH`, `CREATED`,
`OBSERVE_CLEAR`, zero counts, no times, empty types, no completion); `CompletionRecord { final_sequence, final_hash,
at }`; `BatchRecord` (§3.5.3 fields, `event_types: Vec<String>`); `FindingRecord { session, generation, sequence:
Option<u64>, code, detail, at }`; `ChainUpdate { chain, batch: Option<BatchRecord>, findings: Vec<FindingRecord> }`;
`XpSummaryRecord { session, member, host, realm, minutes: u32, grants: Json, flagged: bool, created_at }`;
`AlertRecord { alert_id, code, severity, message, details: Json, created_at, acknowledged_at: Option<UnixMillis>,
acknowledged_by: Option<String> }`. `EncounterState` (`Copy, Ord, Default` = `Created`) with `as_str` (the names in
§3.5.3, `ABORTED` for aborted) and `is_terminal`; `Disposition::{ObserveClear (default), ObserveFlagged}` with `as_str`
`OBSERVE_CLEAR`/`OBSERVE_FLAGGED`; `TrafficScope::{Host, Session}` (`Copy, Ord, Hash`) with `as_str`.

- `MemoryGuardStore` (`Default`; helpers `host_writes()`, `windows()`, `alert(id)`): the same semantics serially
  behind one lock (`reserve_traffic` purges with `expires_at > host.window_start` kept).
- `StubGuardStore`: every method `StoreError::Unimplemented(Guard, "store.<method>")`.
- `SqlGuardStore` on `store::Database` (§2.3): reads through `Database::read`, writes through `Database::write` with
  `store_actor(actor)` (`Account` → `store::Actor::Player(account)`, `HostRuntime` → `Player(host)`, `Admin` →
  `Admin(key_id)`, `System(task)` → `System(task)`). SQL uses `?` placeholders numbered with
  `store::portable::numbered`; upserts and insert-if-absent use `store::portable::{upsert, insert_ignore}`. Any
  failure is `StoreError::Database(<text>)`; a row that does not map back (unknown enum text, negative count,
  unparsable JSON or id) is a failure, never skipped. `host_writes()` orders by `id`, `windows()` by `window_id`.
  `purge_expired(now)`, actor `System("rifts-guard-purge")`, deletes windows with `expires_at ≤ now` and summaries
  with `ttl_at ≤ now` and returns the total removed. `XP_SUMMARY_TTL_MS = 34_560_000_000` (400 days).
- `block_on_store(future)`: inside a multi-thread Tokio runtime, block in place on the current handle; inside a
  current-thread runtime, run the future on a process-wide two-worker fallback runtime from a scoped thread (a panic
  there is `StoreError::Database("store worker thread panicked")`); outside any runtime, on the fallback runtime.
  Failure to build the fallback is `StoreError::Database(<text>)`.

---

## 4. Schema and stored formats

Migration `2026-10-26-000002_rifts_guard`, one folder per engine; the files move byte for byte (A). The two `up.sql`
differ only in the two surrogate keys: SQLite `id INTEGER PRIMARY KEY AUTOINCREMENT`, Postgres `id BIGSERIAL PRIMARY
KEY`. SQLite `up.sql` (the comment lines included):

```sql
-- RF6's Guard persistence tables (DD15): evidence chain state, batches, findings, traffic windows,
-- observed host writes, realm hunt XP summaries and operations alerts.

CREATE TABLE rifts_hunt_guard (
    session TEXT NOT NULL,
    generation BIGINT NOT NULL,
    revision BIGINT NOT NULL,
    completion_mode TEXT,
    last_sequence BIGINT NOT NULL,
    head_hash TEXT NOT NULL,
    encounter_state TEXT NOT NULL,
    disposition TEXT NOT NULL,
    finding_count BIGINT NOT NULL,
    batch_count BIGINT NOT NULL,
    event_count BIGINT NOT NULL,
    event_bytes BIGINT NOT NULL,
    last_monotonic_ms BIGINT,
    event_types TEXT NOT NULL,
    has_unattributed_damage INTEGER NOT NULL,
    last_received_at BIGINT,
    final_sequence BIGINT,
    final_hash TEXT,
    completed_at BIGINT,
    PRIMARY KEY (session, generation)
);

CREATE TABLE rifts_guard_batches (
    session TEXT NOT NULL,
    generation BIGINT NOT NULL,
    batch_id TEXT NOT NULL,
    previous_hash TEXT NOT NULL,
    batch_hash TEXT NOT NULL,
    first_sequence BIGINT NOT NULL,
    last_sequence BIGINT NOT NULL,
    event_count BIGINT NOT NULL,
    event_bytes BIGINT NOT NULL,
    event_types TEXT NOT NULL,
    received_at BIGINT NOT NULL,
    PRIMARY KEY (session, generation, batch_id)
);
CREATE INDEX rifts_guard_batches_session_seq ON rifts_guard_batches (session, generation, first_sequence);

CREATE TABLE rifts_guard_findings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session TEXT NOT NULL,
    generation BIGINT NOT NULL,
    sequence BIGINT,
    code TEXT NOT NULL,
    detail TEXT NOT NULL,
    at BIGINT NOT NULL
);
CREATE INDEX rifts_guard_findings_session_at ON rifts_guard_findings (session, generation, at);

CREATE TABLE rifts_traffic_windows (
    window_id TEXT PRIMARY KEY,
    scope TEXT NOT NULL,
    window_start BIGINT NOT NULL,
    requests BIGINT NOT NULL,
    bytes BIGINT NOT NULL,
    expires_at BIGINT NOT NULL
);
CREATE INDEX rifts_traffic_windows_expires ON rifts_traffic_windows (expires_at);

CREATE TABLE rifts_host_writes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    target TEXT NOT NULL,
    host TEXT NOT NULL,
    session TEXT NOT NULL,
    generation BIGINT NOT NULL,
    realm TEXT,
    session_kind TEXT,
    hub_kind TEXT,
    summary TEXT NOT NULL,
    at BIGINT NOT NULL
);
CREATE INDEX rifts_host_writes_session ON rifts_host_writes (session, generation, at);
CREATE INDEX rifts_host_writes_target ON rifts_host_writes (target, at);
CREATE INDEX rifts_host_writes_host ON rifts_host_writes (host, at);

CREATE TABLE rifts_hunt_xp_summaries (
    session TEXT NOT NULL,
    member TEXT NOT NULL,
    host TEXT NOT NULL,
    realm TEXT,
    minutes BIGINT NOT NULL,
    grants TEXT NOT NULL,
    flagged INTEGER NOT NULL,
    created_at BIGINT NOT NULL,
    ttl_at BIGINT NOT NULL,
    PRIMARY KEY (session, member)
);
CREATE INDEX rifts_hunt_xp_summaries_member ON rifts_hunt_xp_summaries (member, created_at);
CREATE INDEX rifts_hunt_xp_summaries_ttl ON rifts_hunt_xp_summaries (ttl_at);

CREATE TABLE rifts_operations_alerts (
    alert_id TEXT PRIMARY KEY,
    code TEXT NOT NULL,
    severity TEXT NOT NULL,
    message TEXT NOT NULL,
    details TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    acknowledged_at BIGINT,
    acknowledged_by TEXT
);
CREATE INDEX rifts_operations_alerts_created ON rifts_operations_alerts (created_at);
```

`down.sql` (both engines): `DROP TABLE` of `rifts_operations_alerts`, `rifts_hunt_xp_summaries`, `rifts_host_writes`,
`rifts_traffic_windows`, `rifts_guard_findings`, `rifts_guard_batches`, `rifts_hunt_guard`, one statement per line in
that order.

Column encodings existing rows already hold: times and counts as `BIGINT` Unix ms / integers; booleans as `INTEGER`
0/1; `completion_mode` `FIRST_DEFEAT`/`END_OF_HUNT`/`NO_COMPLETION` or NULL; `encounter_state` and `disposition` as
`as_str`; `scope` `host`/`session`; `kind` as `kind_name`; `session`/`realm` as the seam ids' display (parsed back
with `FromStr`); the chain's `event_types` as a JSON object of counts with keys in byte order (e.g.
`{"DAMAGE":2,"WORLD_READY":1}`); a batch's `event_types` as a JSON array of strings; `final_sequence`, `final_hash`,
`completed_at` all set (a completion) or all NULL; `summary`, `grants`, `details` as the JSON text given.

`TABLES` (`&[TableSpec]`, in this order; the test checks the migrations create exactly these, in this order):

| `name` | `node_collection` | `keys` | `expires_by` |
| --- | --- | --- | --- |
| `rifts_hunt_guard` | `p2pHuntSessions.guard*` | `session + generation` | none |
| `rifts_guard_batches` | `(the private module's evidence batches)` | `session + generation + batch unique` | none |
| `rifts_guard_findings` | `(the private module's findings)` | `session + generation + at` | none |
| `rifts_traffic_windows` | `guardTrafficWindows` | `window id (scope, window start, hashed key)` | `expires_at` |
| `rifts_host_writes` | `p2pHostWrites` | `session + generation; target, host, realm by at` | none |
| `rifts_hunt_xp_summaries` | `p2pHuntXpSummaries` | `session + member` | `ttl_at` |
| `rifts_operations_alerts` | `operationsAlerts` | `alert; status + created_at` | none |

`owner` is `Component::Guard` for each. `SCHEMA` lists no soft-delete tables.

---

## 5. Hooks isles calls through `rustic-rifts-seam`

Isles never names `rustic-rifts-guard`; it holds the `Arc<dyn GuardHooks>` that `isles-1-14-7::rifts_with_core` builds from
`Guard::new` and the `GuardRoutes::with_store(guard, has_store)` route handler (`has_store` false when no database is
configured, with `StubGuardStore` and `NoHuntSessions`). What isles calls:

| Hook | Where isles calls it |
| --- | --- |
| `rate_limit_bucket(request)` | `isles-core` `http.rs`, for every request, to pick the per-capability rate-limit bucket |
| `authorize_inventory(runtime, transaction)`, then the fence's `on_grant`/`on_grant_in`/`report` | `areas/inventory/routes.rs` (`POST /inventory` from a host runtime) |
| `authorize_host_write(runtime, kind, target, options)` | `areas/inventory/routes.rs`, `areas/characters.rs`, `areas/progression/auth.rs`, `areas/bounties_store/handlers.rs` |
| `record_host_write(runtime, write)` | the same four, after an accepted write (errors are logged by isles and ignored) |
| `host_policy_refuses()` | `areas/progression/auth.rs` (an error counts as true) |
| `note_would_refuse(what, detail)` | `areas/progression/auth.rs`, `xp.rs` |
| `Lifecycle` (`start`, `stop`, `startup_summary`, `tables`, `account_deletion_steps`) | isles' module lifecycle |

**Not wired today:** isles calls neither `GuardHooks::census` nor `Guard::record_census`/`flush_census`, so the
census counts nothing and only `stop` would flush (risk 4). Realms writes the chain row itself and does not call
`open_chain`.

---

## 6. Tests that define the behaviour

### 6.1 Reused as they are

- `tests/sql_store.rs` (A): migrations create exactly `TABLES`; host write round trip; traffic pair rollback;
  chain revision guard, duplicate batch id refusal leaving chain and findings untouched, findings order, completion
  update; XP summary upsert and `purge_expired` (returns 4 in the async suite); alert insert-if-absent and single
  acknowledgement; reopen preserves rows; Postgres when `REFORGED_TEST_POSTGRES_URL` is set.
- Outside the crate, as oracles in dainty-restricted: `rustic-rifts-realms/tests/guard_adapter.rs` (Guard over
  `RealmsHuntSessions` and `SqlGuardStore`: authorize/record host writes, inventory decisions, rate-limit bucket,
  fresh and existing migrations), `rustic-rifts-realms/tests/stubbed.rs` and `isles-1-14-7/tests/{module.rs,
  inventory_reforge_host.rs}` (stub routes, `rifts.guard()`).

### 6.2 Rewritten from this document (B)

Each unit test of the B files names the behaviour it pins; the rewrite needs at least these, by section:

- §3.1: mode parsing and warnings; startup lines (4 lines each for the three-setting sequences); evidence tier
  defaults and explicit on/off; trust tier literal `1`; the four Tier 1 gates in order; the switches table
  (`P2P_HOST_WRITES`, census, distributed literal-true).
- §3.2/§3.3: statuses; `INTERNAL` mapping; the envelope text; route list equality with §3.3 (replacing the
  `docs/inventory.json` read); stub routes and store-less routes answer `Unimplemented(<handler id>)`.
- §3.4: object/array only, 72 KiB + 1 → TooLarge, broken → Invalid, no body → `None`, `content-length: 0` → `{}`,
  no JSON type → `None`, duplicate key last wins; JS number and string text vectors (§3.0, e.g. `1e21` →
  `1e+21`, `1e-7` → `1e-7`, `123456789012345680000` stays integral, `9007199254740993` → `9007199254740992`);
  `stringified_len(None) = 4`; `Number()` conversions (`" 0x10 "` → 16, `""` → 0, `"1.5"` not safe).
- §3.5: the transport's exact body parses (two events, first without actor) and hashes deterministically to 64
  hex; every malformation in §3.5.2 refused, 129 events → TooLarge; chaining by hash and sequence; a stale batch →
  conflict with `CHAIN_CONFLICT`; findings order `ACTOR_NOT_ROSTERED, UNATTRIBUTED_DAMAGE, EVIDENCE_LOSS` and batch
  `event_types` `["DAMAGE","EVIDENCE_LOSS"]`; closed chain → frozen; completion before request → invalid, wrong head
  → conflict, right → done, same again → no-op, `NO_COMPLETION` → invalid; strict completion body; replies start
  with `{"accepted":true,"batchHash":"…"`. Through the routes: evidence off → `HUNT_EVIDENCE_NOT_COLLECTED` and the
  budget back to (0, 0); bounded and strict bodies; anonymous → `P2P_CAPABILITY_INVALID`; the full chain flow
  (resend → `duplicate:true` same hash; same id other content → replay; gap → conflict + finding + log line; terminal
  batch flags; completion → verdict `OBSERVED`; counts (2, 4, 1); late batch → frozen; completion again → 200);
  wrong generation / no hunt → fenced, hub → not collected; a stub store's error surfaces as `Unimplemented
  ("store.chain")`.
- §3.6: member/non-member/unknown session on `reward/self`.
- §3.7: local limiter (requests, bytes, window reset, eviction at capacity); budget release exactly once; window
  start/id shape and validation; operation classification; local windows per host and session with a new window after
  60 s; distributed limit then `Unavailable` on a stub store; window ids never contain the key; route-level 429;
  bucket cache (one lookup for two requests, revoked/unknown/malformed/absent/failing → `None`, never logged).
- §3.8: every `writes.rs` case (running, drained, suspect-from-running, fenced, other generation, non-member,
  not admitted, hub kinds per hub, presence, host always present, reforge refused in hunts, rank confirm and loadout
  unlock); observe logs a would-refuse and lets XP through in a Ramsgate hub; probe strict and quiet; Training reforge
  observed; parity quiet in observe, refused in enforce; enforce refuses kinds; kill switch text and log; tags on
  record (hub row has realm/hub, hunt row none, gone session not tagged).
- §3.9/§3.10: every `grants.rs` classification list (§3.10's ids are the test vectors), no-game-data rules,
  violations and summary with `,+2`, no-reward sessions, claim-funded coin bounds and case, prestige-gated rules,
  Platinum spend, draft token exemption, starter grant, malformed entries skipped, stacked balance, the decision
  table; hook level: fence re-checks presence (`P2P_LOOT_GRANT_PARTICIPANT_NOT_ADMITTED` text), observe vs enforce
  grant class lines and the exact 403 body, no-reward session, draft over cap early and inside, decided again from
  reads (`P2P_LOOT_GRANT_DRAFT_PREMIUM_OVER_BALANCE_CAP`, the `[HostGrantDeny] … at=transaction` and
  `[HostGrantAllow] … day=… balance=0->1 cap=1` lines), unreadable balance, accepted replay, token next to another
  Hunt Pass item.
- §3.11: rollout 400/409/409/403 (with audit)/200 (audit `ip` is the remote address, `requestId` matches); ack 400,
  200, 404, anonymous `AUTH_UNAUTHORIZED`; rollout state validation; field-order texts.
- §3.12: status classes, normalization, route labels, merge/sort/cap, ties, the exact summary line, peek, disabled.
- §3.13: lifecycle summary text, deletion steps, start lines (exactly two in enforce); the stub's answers.
- §3.14/§3.0: ISO time vectors (`0`, `1_792_000_000_123` → `2026-10-14T17:46:40.123Z`, a leap day, `-1`); v4 UUIDs.
- `digest`: SHA-256 vectors (`""`, `abc`, the 448-bit message, one million `a`), split updates.

### 6.3 Fuzz targets (B, rewrite into dainty-lass `fuzz/`)

- `guard_evidence`: bytes through `parse_p2p_body` (JSON content type); `parse_completion` and `parse_batch` on the
  result; for an accepted batch assert 1-128 events, first ≥ 1, last = first + n − 1, event bytes ≤ 65,536, a
  64-hex hash, consecutive sequences, non-decreasing monotonic times, a deterministic hash; apply it to a chain whose
  last sequence and head it continues and assert the new sequence, head, revision + 1, event count, finding count =
  findings, flagged iff findings; any refusal's code starts with `GUARD_`; try the completion on the result.
- `guard_grants`: UTF-8 input to `classify` and `stacked_balance_of` (balance ≥ 0); the parsed body to `violations`
  with and without the allowance (the allowance removes at most one violation, and only the draft token's);
  `grant_denial` for the three session shapes (`NO_REWARD_SESSION` only for no-completion non-hub; otherwise the
  violations equal the strict ones, classes non-empty; none iff no violations); `decide_draft_premium` with balances
  none/0/1/u32::MAX never allows above the cap.

---

## 7. The target MIT crate

- **Where:** `crates/rustic-rifts-guard` in dainty-lass (flat layout), package `rustic-rifts-guard`, `license = "MIT"`, lints from
  the workspace. Fuzz targets in dainty-lass `fuzz/fuzz_targets/` with `[[bin]]` entries; the checklist at
  `docs/security/guard.md`.
- **Dependencies** (all already in dainty-lass `[workspace.dependencies]` unless noted): `diesel`, `diesel_migrations`,
  `serde_json` **with `features = ["preserve_order"]`** at the crate (as `service-kit` does), `sha2`, `store`,
  `thiserror`, `tokio` with `rt` and `rt-multi-thread`; dev: `tempfile`, `url`, `tokio` with `macros`. Fuzz:
  `libfuzzer-sys`, `serde_json`.
- **Seam changes to flag:** (1) `rustic-rifts-seam` must be a dainty-lass workspace dependency (its own MIT port is another
  RD27 spec); (2) restricted's workspace `diesel` has `32-column-tables` and dainty-lass's does not. The chain row
  binds 22 parameters and reads 19 columns by name; with boxed raw SQL and `QueryableByName` this should not need the
  feature, but if the implementer's design uses typed tuples above 16 it does: a workspace seam change, report it.
- **dainty-restricted afterwards:** replace the path dependency with the git dependency at the new rev, delete
  `modules/rustic-rifts/crates/rustic-rifts-guard`, the two fuzz bins and the security page there. `rustic-rifts-realms` and
  `isles-1-14-7` change no source.

## 8. Acceptance criteria

1. The §2 API compiles against the unchanged callers: rifts-realms (src and tests), isles-1-14-7 (src and tests) and
   the fuzz workspace build with only the dependency switched.
2. `tests/sql_store.rs` passes unchanged on SQLite (and on Postgres where configured); `guard_adapter.rs`,
   `stubbed.rs`, `module.rs` and `inventory_reforge_host.rs` pass in dainty-restricted against the MIT crate.
3. The §6.2 tests exist in the new crate and pass; every exact string of §3 (codes, messages, reply templates, log
   lines the tests read) matches byte for byte.
4. A database written by the AGPL crate opens and keeps working: same migration name and DDL, same column encodings,
   and a chain mid-hunt continues (its head was computed with §3.5.2's hash, so the rewrite must produce identical
   hashes for identical bodies).
5. 3+-line similarity of each new file against its AGPL counterpart under 30% (`clean-room/similarity.md` §1).
6. `cargo fmt`, `clippy -D warnings`, the workspace lints (no `unwrap`/`expect` outside tests, no `unsafe`) and
   `cargo deny` pass; the fuzz targets build on nightly.

## 9. Risks and ambiguities

1. **Hash input key order.** The batch hash and the 8 KiB/64 KiB byte counts use document key order only because
   restricted's workspace unifies `serde_json/preserve_order` on. A rewrite built without it would sort keys, change
   every hash for the same body and break chains of hunts running across the upgrade. The MIT crate must turn the
   feature on itself (§7) and a test must pin one known body to its known hash (compute it once on the AGPL crate and
   record it in the MIT test). The fuzz workspace builds without the feature; nothing there depends on the order.
2. **Wire identifiers carrying the Mystic name** are contracts kept verbatim and the umbrella spec decides keep or
   rename: the header `x-mystic-host-capability` (the seam's constant), the paths `/mystic/v1/hunts/...`, the keys
   `MYSTICPARADOX_HOST_POLICY`, `MYSTICPARADOX_HUNT_EVIDENCE`, `MYSTICPARADOX_HOST_ROUTE_CENSUS`, the startup lines
   that print them, and `x-hunt-guard-shadow-key` (named only in docs).
3. **Route handler ids** (`antiCheat.ts:…`, `p2pHuntStatus.ts:…`, `p2pAdminOperations.ts:…`) are Node file and
   function names, visible as `Unimplemented` operation names and matching `docs/inventory.json`. Kept verbatim here;
   renaming them needs the stub tests in realms and isles checked. `TABLES.node_collection` strings likewise name
   Node collections; they are metadata only.
4. **Census not wired.** No caller records or flushes the census, so `[HostRoute] summary` lines never appear outside
   `stop`. Behaviour to keep as specified; wiring it is isles' (and possibly a seam change for the sample).
5. **Operations writes are in memory.** With `CoreOperationsPolicy`, rollout updates and audit rows go to an
   in-memory store while `policy()` reads isles' document, so a successful update is not persisted and the next
   read shows the old version. Keep as is; flagged for RU4.
6. **Local traffic charge is not atomic across host and session** (a session refusal keeps the host's charge), unlike
   the distributed one. Specified as observed; tests do not cover the asymmetry, so the implementer must not "fix"
   it silently.
7. **Shadow route and the budget.** The security page says shadow calls with a capability take a budget slot; the
   code answers the shadow route before any budget or traffic charge. This spec follows the code.
8. **SUSPECT asymmetry.** Host writes accept a `SUSPECT` hunt that was running; inventory transactions do not. Kept.
9. **Lossy input decoding and parser limits.** Non-UTF-8 bytes become U+FFFD before hashing; lone surrogate escapes,
   numbers beyond `f64` and nesting past `serde_json`'s 128 levels are `GUARD_BATCH_INVALID` though `JSON.parse`
   would take them. Integers beyond 2^53 in payloads are canonicalized through `f64`.
10. **`AUTH_UNAUTHORIZED`'s status** is not in `ERROR_STATUSES` (`status_of` would say 500); isles supplies its own.
11. **Classification of `sql.rs`, the migrations and `tests/sql_store.rs`.** Git history shows all three were
    written on Dainty Lass's `store` (`210eb22`), with no Mystic Paradox counterpart (Node used MongoDB). This spec
    takes the DDL and the black-box test as A and `sql.rs` as B because its record shapes are the ported ones; the
    reviewer may move `sql.rs` to A.
12. **`block_on_store` from a current-thread runtime** spawns a scoped OS thread per call; fine for tests, slow if a
    production caller ever ran on a current-thread runtime. Behaviour to keep.
