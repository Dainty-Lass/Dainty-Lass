<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false, "code_blocks": false } } -->

# Clean-Room Behaviour Specification: `rifts-seam`, `rifts-admission`, `rifts-relay` (RD27)

> **Scope and purpose (RD27):** the clean-room behaviour specification for relicensing three crates of Rustic Rifts
> to MIT. They live in dainty-restricted (AGPL) at `modules/rustic-rifts/crates/rifts-seam`, `crates/rifts-admission`
> and `crates/rifts-relay`. Rustic Rifts' `NOTICE.md` says the Rust workspace "ports Mystic Paradox's `p2p/`,
> `realms/`, Guard and launcher realm-hosting modules", so every source file is rewritten here, from this document
> alone, by an implementer who never sees the AGPL code.
>
> Everything below is stated from observable contracts: the public Rust API the other rifts crates, `isles-core`,
> `isles-1-14-7`, the launcher and the fuzz targets call; the ticket and control-frame byte layouts the transport
> (`ParadoxTransport.dll`) reads; the HTTP routes and their JSON; the KDF inputs; the settings; the error texts the
> recorded Node vectors compare. No implementation code is reproduced and no private helper is named.
>
> Spec written against dainty-restricted `4efc8b5` (branch `claude/rd27-spec-rifts-c45124`, crates last changed in
> `ca76df9`).

---

## 1. Provenance and licensing classification

### 1.1 Evidence used

| Source | What it says |
| --- | --- |
| `modules/rustic-rifts/NOTICE.md` | The Rust workspace ports Mystic Paradox's `p2p/`, `realms/`, Guard and launcher modules (`21b9ff5`, AGPL-3.0-only); every port stays AGPL |
| Crate headers (`//!`) | `rifts-admission`: ported from `p2p/security/admissionTickets.ts`, `p2p/controllers/p2pAdmission.ts`, `realmHubAdmission.ts`, `p2pLauncher.ts`, `p2p/security/transportPeerIds.ts`, `scripts/p2p/generate-admission-key.mjs`, and the transport's `src/admission_ticket.cpp`, `src/control_codec.cpp`. `rifts-relay`: `p2p/config/p2pIcePolicy.ts`, `p2p/services/relayCredentials.ts`, `p2p/security/iceSessionRules.ts`, `p2p/controllers/p2pIce.ts`, `p2pAdminOperations.ts`. `rifts-seam`: Node's `P2PExtension` hook object (`extensions/p2p.ts`), the `realms/` exports and `p2p/antiCheat.ts` |
| Commits | `b3f9cd3` (RF2, seam), `e61e8e4` (RF4, admission and relay), `58525cd` (fuzz targets), `c695cb2` (security checklists), `64853ca`, `210eb22`, `97853a5` (routes, seams), `ca76df9` (seam: host grant catalog) |
| `tests/vectors/make-vectors.mts` | Runs Mystic Paradox's own Node modules in `node:22` and records their answers into `node-admission.json` and `node-relay.json` |

Category definitions follow `spec-rd27-launcher.md` §1.2: **A** born in DaintyLass and moves as is (with the edits
named), **B** derived and rewritten clean-room from this spec, **D** data recorded from Mystic Paradox's programs (and
the harness that replays it), stays in dainty-restricted as an oracle, **G** generated (none here).

### 1.2 Files

Line counts are `wc -l` at `4efc8b5`. Paths are relative to `modules/rustic-rifts/`.

| Path | Lines | Cat. | Evidence | Target |
| --- | ---: | --- | --- | --- |
| `crates/rifts-seam/Cargo.toml` | 15 | A | Manifest; only `thiserror` | MIT crate, `license = "MIT"` |
| `crates/rifts-seam/src/lib.rs` | 50 | B | Hook object re-shaped, `stub`, `poll_once` | §2 |
| `crates/rifts-seam/src/core.rs` | 186 | B | `IslesCore`, Node function names per method | §2.1.3 |
| `crates/rifts-seam/src/dedicated.rs` | 382 | B | Node's `DEDICATED_ONLY`, `LoadP2PExtension` | §2.3 |
| `crates/rifts-seam/src/error.rs` | 67 | B | `LauncherApiError` codes | §2.1.1 |
| `crates/rifts-seam/src/hooks.rs` | 307 | B | `P2PExtension` hooks | §2.1.4 |
| `crates/rifts-seam/src/http.rs` | 231 | B | Routes as data, `p2pErrors.ts` envelope | §2.1.5 |
| `crates/rifts-seam/src/id.rs` | 211 | B | Global ids (DD18); in a ported crate | §2.1.6 |
| `crates/rifts-seam/src/store.rs` | 101 | B | Table shapes of Node's collections | §2.1.7 |
| `crates/rifts-admission/Cargo.toml` | 23 | A | Manifest | MIT crate, `license = "MIT"`, description without "MPT2" if renamed (§9) |
| `crates/rifts-admission/src/lib.rs` | 260 | B | Traits, `DeriveIcePeerId` | §3.1 |
| `crates/rifts-admission/src/ticket.rs` | 526 | B | `admissionTickets.ts`, `admission_ticket.cpp` | §3.2 |
| `crates/rifts-admission/src/ticket/tests.rs` | 402 | B | Unit tests | rewrite from §6.1 |
| `crates/rifts-admission/src/keys.rs` | 372 | B | `generate-admission-key.mjs`, key loading | §3.3 |
| `crates/rifts-admission/src/control.rs` | 178 | B | `control_codec.cpp` | §3.4 |
| `crates/rifts-admission/src/store.rs` | 485 | B | `p2pSteamLinks`, `p2pAdmissionTickets`, `p2pPeerAdmissions`, in-memory repository | §3.5 |
| `crates/rifts-admission/src/service.rs` | 785 | B | `p2pAdmission.ts`, `realmHubAdmission.ts`, `p2pLauncher.ts` | §3.6 |
| `crates/rifts-admission/src/service/tests.rs` | 532 | B | Unit tests | rewrite from §6.1 |
| `crates/rifts-admission/src/routes.rs` | 281 | B | Route table and dispatch | §3.7 |
| `crates/rifts-admission/examples/generate-admission-key.rs` | 42 | B | `generate-admission-key.mjs` as a command | §3.8 |
| `crates/rifts-admission/examples/mint-ticket.rs` | 100 | B | Defaults from `admissionTickets.test.ts` | §3.8 |
| `crates/rifts-admission/tests/node_vectors.rs` | 129 | D | Replays `node-admission.json` | stays in restricted (§6.2) |
| `crates/rifts-admission/tests/vectors/node-admission.json` | 5931 | D | Node's output | stays in restricted |
| `crates/rifts-admission/tests/vectors/make-vectors.mts` | 186 | D | Drives Mystic's Node modules | stays in restricted |
| `crates/rifts-relay/Cargo.toml` | 23 | A | Manifest | MIT crate, `license = "MIT"` |
| `crates/rifts-relay/src/lib.rs` | 207 | B | Traits, `TransportKeys` | §4.1 |
| `crates/rifts-relay/src/wire.rs` | 333 | B | `JSON.stringify`, `toISOString`, `Number` rules | §4.2 |
| `crates/rifts-relay/src/config.rs` | 420 | B | `p2pIcePolicy.ts` | §4.3 |
| `crates/rifts-relay/src/credentials.rs` | 971 | B | `relayCredentials.ts` | §4.4 |
| `crates/rifts-relay/src/rules.rs` | 649 | B | `iceSessionRules.ts` | §4.5 |
| `crates/rifts-relay/src/service.rs` | 368 | B | `p2pIce.ts`, `UpdateCoopTransport` | §4.6 |
| `crates/rifts-relay/src/service/tests.rs` | 373 | B | Unit tests | rewrite from §6.1 |
| `crates/rifts-relay/src/routes.rs` | 214 | B | Route table and dispatch | §4.7 |
| `crates/rifts-relay/examples/mint-relay-file.rs` | 95 | B | `tools/mint-relay-test-file.cjs` | §4.8 |
| `crates/rifts-relay/tests/node_vectors.rs` | 160 | D | Replays `node-relay.json` | stays in restricted (§6.2) |
| `crates/rifts-relay/tests/vectors/node-relay.json` | 596 | D | Node's output | stays in restricted |
| `fuzz/fuzz_targets/admission_control.rs` | 22 | A | RX3 target born here (Mystic has no fuzzing); asserts over the public API only | dainty-lass `fuzz/` |
| `fuzz/fuzz_targets/admission_ticket.rs` | 58 | A | As above | dainty-lass `fuzz/` |
| `fuzz/fuzz_targets/relay_signal.rs` | 50 | A | As above | dainty-lass `fuzz/` |
| `fuzz/Cargo.toml` (three `[[bin]]` entries and the `rifts-admission`, `rifts-relay`, `rifts-seam` deps) | — | A | Born here | dainty-lass `fuzz/Cargo.toml` (a seam file; §7.2) |
| `docs/security/admission.md` | 122 | A | RX3 checklist written here | dainty-lass `docs/security/`, with the edits in §7.3 |
| `docs/security/relay-credentials.md` | 106 | A | RX3 checklist written here | dainty-lass `docs/security/`, with the edits in §7.3 |

Counts: **A 8** files (3 manifests, 3 fuzz targets, 2 checklists) plus the fuzz manifest entries; **B 28**; **D 5**;
**G 0**.

The A files carry no Mystic code but cite private test-function names of the B files (the checklists) and the
`rifts_admission::keys`/`ticket`/`control` and `rifts_relay::rules`/`wire` module paths (the fuzz targets), so those
module paths are part of the public API (§2-§4) and the checklist references are edited after the rewrite (§7.3).

### 1.3 Tests: what is rewritten, what stays as an oracle

- **Rewritten with the B code** (from §6.1): every `#[cfg(test)]` module inside the B sources, and the three test
  files `rifts-admission/src/ticket/tests.rs`, `rifts-admission/src/service/tests.rs`,
  `rifts-relay/src/service/tests.rs`. The two `routes_match_the_inventory` tests read
  `modules/rustic-rifts/docs/inventory.json`, which is not in these crates; the MIT rewrite checks `ROUTES` against
  the tables in §3.7 and §4.7 instead.
- **Stay in dainty-restricted as oracles** (D): both `tests/node_vectors.rs`, both JSON files and
  `make-vectors.mts`. §6.2 lists exactly which public items they call, so they run against the MIT crates unchanged.

---

## 2. `rifts-seam`

The seam between the 1.14.7 module (`isles-core`, `isles-1-14-7`) and the rifts component crates: the hooks isles
calls, what isles provides back, routes as data, ids, storage shapes, and the dedicated-only default isles runs when
the player-hosted module is off. It names no web framework and depends only on `thiserror`.

### 2.0 Conventions

- "Async" methods return `BoxFuture<'_, Result<T, E>>` (§2.1.5) so the traits stay object-safe.
- All traits that isles holds behind `Arc<dyn ...>` are `Send + Sync` unless stated.
- Derives are part of the API: callers hash, compare, clone and default these types. Each table names them.
- JSON templates in this document (all three crates) are wrapped to fit the page. The real output has no whitespace
  between tokens; a line break inside a quoted message stands for exactly one space. `<x>` is a placeholder written as
  a JSON value (strings quoted and escaped by `wire::json_string`), `..` a field whose value is described beside it.

### 2.1 Public API (complete)

Every item below is `pub` at the crate root (the crate's modules are private; everything is re-exported from the
root). Callers outside the crate (grep at `4efc8b5`): `isles-core`, `isles-1-14-7`, `rifts-admission`, `rifts-relay`,
`rifts-realms`, `rifts-guard`, `rifts-host`, `launcher/src-tauri/src/controller/realms.rs` (which uses `AccountId`,
`GlobalId`, `HubKind`, `NodeId`, `Secret`, `SessionId`, `Ulid`).

#### 2.1.1 Errors

| Item | Shape | Derives / traits | Behaviour |
| --- | --- | --- | --- |
| `Component` | enum `Admission`, `Relay`, `Realms`, `Guard`, `Host`, `Isles` | Debug, Clone, Copy, PartialEq, Eq, Hash, Display | Display: `rifts-admission`, `rifts-relay`, `rifts-realms`, `rifts-guard`, `rifts-host`, `isles` |
| `Unimplemented` | struct, pub fields `component: Component`, `operation: &'static str` | Debug, Clone, Copy, PartialEq, Eq, `std::error::Error` | `const fn new(component, operation)`. Display: `{component}: {operation} is not implemented yet` |
| `Refusal` | struct, pub fields `code: String`, `message: String` | Debug, Clone, PartialEq, Eq, Error | Display: `{code}: {message}` |
| `SeamError` | `#[non_exhaustive]` enum `Unimplemented(Unimplemented)`, `Refused(Refusal)` | Debug, Clone, PartialEq, Eq, Error; `From<Unimplemented>`, `From<Refusal>` | Display is the inner value's (transparent). `const fn unimplemented(component, operation) -> Self`; `fn is_unimplemented(&self) -> bool` |

#### 2.1.2 Free functions

| Item | Signature | Behaviour |
| --- | --- | --- |
| `stub` | `fn stub<T, E>(component: Component, operation: &'static str) -> BoxFuture<'static, Result<T, E>> where T: Send + 'static, E: From<Unimplemented> + Send + 'static` | A ready future yielding `Err(Unimplemented::new(component, operation).into())` |
| `poll_once` | `fn poll_once<F: Future>(future: F) -> Option<F::Output>` | Polls the future exactly once with a no-op waker and no runtime; `Some(output)` if ready, else `None` |
| `p2p_module_enabled` | `fn p2p_module_enabled(settings: &dyn Settings) -> bool` | §2.3.1 |

#### 2.1.3 What isles provides (`IslesCore`, `HostGrantCatalog`, `Party`, `GuardMode`, `NoCore`)

`Party`: struct, pub fields `id: PartyId`, `leader: AccountId`, `members: Vec<AccountId>`, `revision: u64`; Debug,
Clone, PartialEq, Eq.

`GuardMode`: enum `Observe` (the `Default`), `Enforce`; Debug, Clone, Copy, PartialEq, Eq, Default.
`fn from_setting(value: Option<&str>) -> Self`: the value is trimmed (Rust `trim`); empty (or `None`) or
`observe` in any ASCII case is `Observe`; anything else is `Enforce` (`off` enforces).

`HostGrantCatalog` (trait, `Send + Sync`), all sync:

| Method | Returns |
| --- | --- |
| `tags_of(&self, catalog_id: &str)` | `Option<Vec<String>>` |
| `is_prestige_reward(&self, catalog_id: &str)` | `bool` |
| `is_prestige_gated(&self, catalog_id: &str)` | `bool` |
| `is_entitlement_name(&self, name: &str)` | `bool` (name is lower case) |

`IslesCore` (trait, `Send + Sync`). "Fut<T>" below means `BoxFuture<'_, Result<T, SeamError>>`.

| Method | Signature | Default |
| --- | --- | --- |
| `display_name` | `(&self, account: AccountId) -> Fut<Option<String>>` | required |
| `find_by_display_name` | `(&self, name: String) -> Fut<Option<AccountId>>` | required |
| `launcher_account_eligible` | `(&self, account: AccountId) -> Fut<bool>` | required |
| `party` | `(&self, party: PartyId) -> Fut<Option<Party>>` | required |
| `get_or_create_party` | `(&self, account: AccountId) -> Fut<Party>` | required |
| `set_candidate` | `(&self, account: AccountId, candidate: Candidate) -> Result<(), SeamError>` | required (sync) |
| `cancel_travel` | `(&self, hunt_id: &str, accounts: &[AccountId], reason: &str) -> Result<bool, SeamError>` | required (sync) |
| `dedicated_servers_configured` | `(&self) -> bool` | required |
| `touch_game_session` | `(&self, account: AccountId) -> Fut<()>` | required |
| `record_hunt_activity` | `(&self, session: SessionId, activity: Json) -> Fut<()>` | required |
| `healthy_guard_session` | `(&self, account: AccountId) -> Fut<Option<Json>>` | required |
| `published_guard_artifacts` | `(&self) -> Fut<Json>` | required |
| `guard_mode` | `(&self) -> GuardMode` | required |
| `operations_policy` | `(&self) -> Fut<Json>` | required |
| `verify_operations_step_up` | `(&self, key_id: String, step_up: String) -> Fut<bool>` | required |
| `host_grant_catalog` | `(&self) -> Option<Arc<dyn HostGrantCatalog>>` | `None` |
| `stacked_items` | `(&self, character_id: String) -> Fut<Option<Json>>` | `Unimplemented(Isles, "stacked_items")` |
| `accepted_inventory_replay` | `(&self, transaction: InventoryTransaction) -> Fut<bool>` | `Unimplemented(Isles, "accepted_inventory_replay")` |

`NoCore`: unit struct; Debug, Clone, Copy, Default; implements `IslesCore`. Every async method yields
`Err(SeamError::Unimplemented(Unimplemented { component: Isles, operation: <the method name> }))` (operation strings
are the method names exactly as in the table); `set_candidate` and `cancel_travel` return the same error synchronously;
`dedicated_servers_configured` is `false`; `guard_mode` is `Observe`; the three defaulted methods keep their defaults.

#### 2.1.4 Hooks isles calls (`hooks`)

`Settings` (trait, no `Send` bound): `fn get(&self, name: &str) -> Option<&str>`. Implemented for
`std::collections::HashMap<String, String, S>` for any `S: BuildHasher`, and for `BTreeMap<String, String>`, both by
key lookup.

`Lifecycle` (trait, `Send + Sync`):

| Method | Signature |
| --- | --- |
| `runtime_safety_issues` | `(&self, settings: &dyn Settings) -> Result<Vec<String>, SeamError>` |
| `startup_summary` | `(&self, role: &str) -> Result<Option<Json>, SeamError>` |
| `start` | `(&self, role: &str) -> Result<(), SeamError>` |
| `stop` | `(&self) -> BoxFuture<'_, Result<(), SeamError>>` |
| `tables` | `(&self) -> &'static [TableSpec]` |
| `account_deletion_steps` | `(&self) -> Vec<DeletionStep>` |

Data types (all pub fields; all Debug, Clone, PartialEq, Eq unless noted):

| Type | Shape | Extra derives |
| --- | --- | --- |
| `Candidate` | tuple `(pub Json)` | — |
| `HuntRouteContext` | `game_mode: String`, `hunt_id: String`, `player: AccountId`, `party: Option<PartyId>`, `party_members: Vec<AccountId>`, `party_revision: Option<u64>`, `build_id: Option<String>`, `is_hub: bool` | — |
| `HuntRouteOutcome` | enum `NotHandled`, `Handled { result: bool }` | — |
| `CandidateLeaveOutcome` | enum `Canceled`, `AlreadyCanceled`, `AlreadyEnded`, `TooLate`, `HubNotCancelled` | Copy |
| `CandidateLeave` | enum `NotHandled`, `Handled { outcome: CandidateLeaveOutcome, state: String }` | — |
| `OperationsOverview` | `policy: Vec<(String, Json)>`, `sections: Vec<(String, Json)>` | Default |
| `ReleaseGate` | tuple `(pub Json)` | — |
| `AuthOutcome` | enum `NotMine`, `Authenticated(RuntimeAuth)`, `Refused(Reply)` | — |
| `HubKind` | enum `Ramsgate`, `Training` | Copy, Hash |
| `RealmEntry` | `realm: RealmId`, `is_host: bool` | — |
| `InventoryTransaction` | `user: AccountId`, `character_id: String`, `transaction_id: String`, `body: Option<Json>` | — |
| `HostRouteSample` | `method: String`, `route: String`, `status_code: u16`, `session_kind: Option<String>`, `hub_kind: Option<String>` | Default |
| `TransactionPhase` | enum `Applying`, `Accepted`, `Failed` | Copy |
| `InventoryDecision` | enum `Authorized(Box<dyn InventoryFence>)`, `Refused(Reply)` | **no derives** |
| `HostWriteKind` | enum `Xp`, `Mastery`, `Objectives`, `Escalation`, `Cooldown`, `Bounty`, `SlayersPath`, `Reforge`, `RankConfirm`, `LoadoutUnlock`, `HuntPassXp`, `ItemSave` (this order) | Copy, Hash |
| `HostWriteOptions` | `probe: bool`, `parity: bool` | Copy, Default |
| `HostWrite` | `kind: HostWriteKind`, `target: AccountId`, `summary: Json` | — |

`TransactionReads` (trait, no bounds): `fn stacked_items(&self, character_id: &str) -> Result<Option<Json>,
SeamError>`.

`InventoryFence` (trait, `Send + Sync`): `fn on_grant(&self, grant: &Json) -> Result<(), Reply>`;
`fn on_grant_in(&self, grant: &Json, reads: &dyn TransactionReads) -> Result<(), Reply>` with a default that ignores
`reads` and calls `on_grant`; `fn report(&self, phase: TransactionPhase)`.

`RealmsHooks: Lifecycle` ("Fut<T>" as above):

| Method | Signature |
| --- | --- |
| `metric_class` | `(&self, path: &str) -> Option<&'static str>` |
| `request_log_tags` | `(&self, request: &RequestInfo) -> String` |
| `authenticate` | `(&self, request: RequestInfo) -> Fut<AuthOutcome>` |
| `dedicated_hunts_disabled` | `(&self) -> Result<bool, SeamError>` |
| `filters_offline_party_members` | `(&self, game_mode: &str, hunt_id: Option<&str>) -> Result<bool, SeamError>` |
| `restore_candidate` | `(&self, player: AccountId) -> Fut<Option<Candidate>>` |
| `candidate_status` | `(&self, player: AccountId, current: Candidate) -> Fut<Option<Candidate>>` |
| `party_instance` | `(&self, party: PartyId) -> Fut<Option<Candidate>>` |
| `route_hunt` | `(&self, context: HuntRouteContext) -> Fut<HuntRouteOutcome>` |
| `cancel_candidate` | `(&self, player: AccountId, current: Candidate) -> Fut<CandidateLeave>` |
| `party_member_left` | `(&self, account: AccountId) -> Fut<()>` |
| `launcher_policy_fields` | `(&self) -> Result<Vec<(String, Json)>, SeamError>` |
| `operations_policy_fields_valid` | `(&self, record: &Json) -> Result<bool, SeamError>` |
| `operations_overview` | `(&self) -> Fut<OperationsOverview>` |
| `release_gates` | `(&self) -> Fut<Vec<ReleaseGate>>` |

`RealmsEntry` (trait, `Send + Sync`):

| Method | Signature |
| --- | --- |
| `authorize_game_session_realm` | `(&self, account: AccountId, realm: Option<Json>) -> Fut<Option<RealmEntry>>` |
| `bind_realm_presence_at_login` | `(&self, account: AccountId, realm: Option<RealmId>) -> Fut<()>` |
| `touch_presence` | `(&self, account: AccountId) -> Fut<()>` |
| `find_by_hub_room_id` | `(&self, room: String) -> Fut<Option<RealmId>>` |
| `is_member` | `(&self, realm: RealmId, account: AccountId) -> Fut<bool>` |
| `is_banned` | `(&self, realm: RealmId, account: AccountId) -> Fut<bool>` |
| `hub_room_id` | `(&self, realm: &RealmId, kind: HubKind) -> Result<String, SeamError>` |

`GuardHooks: Lifecycle`:

| Method | Signature |
| --- | --- |
| `service_roles` | `(&self) -> &'static [&'static str]` |
| `guard_manifest_roles` | `(&self) -> &'static [&'static str]` |
| `census` | `(&self, sample: &HostRouteSample) -> Result<(), SeamError>` |
| `rate_limit_bucket` | `(&self, request: RequestInfo) -> Fut<Option<String>>` |
| `authorize_inventory` | `(&self, runtime: RuntimeAuth, transaction: InventoryTransaction) -> Fut<InventoryDecision>` |
| `authorize_host_write` | `(&self, runtime: RuntimeAuth, kind: HostWriteKind, target: AccountId, options: HostWriteOptions) -> Fut<Option<Reply>>` |
| `record_host_write` | `(&self, runtime: RuntimeAuth, write: HostWrite) -> Fut<()>` |
| `host_policy_refuses` | `(&self) -> Result<bool, SeamError>` |
| `note_would_refuse` | `(&self, what: &str, detail: &str) -> Result<(), SeamError>` |

#### 2.1.5 Routes as data (`http`)

| Item | Shape | Derives | Behaviour |
| --- | --- | --- | --- |
| `BoxFuture<'a, T>` | type alias `Pin<Box<dyn Future<Output = T> + Send + 'a>>` | — | — |
| `HOST_CAPABILITY_HEADER` | `&str = "x-mystic-host-capability"` | — | Header name, lower case (contract; §9) |
| `Method` | enum `Get`, `Post`, `Patch`, `All` | Debug, Clone, Copy, PartialEq, Eq, Hash | `const fn as_str(self)`: `GET`, `POST`, `PATCH`, `ALL` |
| `Mount` | enum `Early`, `Normal`, `Admin`, `Director` | Debug, Clone, Copy, PartialEq, Eq, Hash | Where isles mounts the route |
| `Route` | pub fields `method: Method`, `path: &'static str` (Express pattern), `mount: Mount`, `handler: &'static str` | Debug, Clone, Copy, PartialEq, Eq | — |
| `Json` | tuple `(pub String)`: JSON text spliced verbatim | Debug, Clone, PartialEq, Eq, Default | `fn empty_object() -> Self` is `{}` |
| `Reply` | pub fields `status: u16`, `body: Json` | Debug, Clone, PartialEq, Eq | `fn not_implemented(unimplemented: &Unimplemented, request_id: &str) -> Self` (below) |
| `RequestInfo` | pub fields `path: String`, `headers: Vec<(String, String)>` (lower-case names), `remote: Option<IpAddr>`, `request_id: String` | Debug, Clone, PartialEq, Eq, Default | `fn header(&self, name: &str) -> Option<&str>`: the first pair whose name equals `name` exactly (no case folding) |
| `CapabilityScope` | enum `HostReady`, `LeaseHeartbeat`, `PeerAdmitted`, `GameServerRuntime`, `EventsWrite`, `CompleteRequest`, `Abort` | Debug, Clone, Copy, PartialEq, Eq, Hash | — |
| `RuntimeAuth` | pub fields `capability_id: String`, `session: SessionId`, `generation: u64`, `host_account: AccountId`, `scopes: Vec<CapabilityScope>`, `roster: Vec<AccountId>`, `host_peer: Option<PeerId>`, `bridge_instance: Option<String>`, `wire_session: Option<String>` | Debug, Clone, PartialEq, Eq | — |
| `Caller` | enum `Anonymous`, `Launcher(AccountId)`, `Admin { key_id: String }`, `HostRuntime(RuntimeAuth)` | Debug, Clone, PartialEq, Eq | — |
| `RouteCall` | pub fields `route: Route`, `params: Vec<(String, String)>` (Express params in path order), `query: String`, `request: RequestInfo`, `caller: Caller`, `body: Vec<u8>` | Debug, Clone, PartialEq, Eq | — |
| `RouteHandler` | trait `Send + Sync`: `fn routes(&self) -> &'static [Route]`; `fn handle(&self, call: RouteCall) -> BoxFuture<'_, Result<Reply, SeamError>>` | — | — |
| `StubRoutes` | pub fields `component: Component`, `routes: &'static [Route]` | Debug, Clone, Copy | `RouteHandler`: `routes()` returns the field; `handle` yields `Err(SeamError::Unimplemented(Unimplemented::new(component, call.route.handler)))` |

`Reply::not_implemented` answers status **501** with the body, as one line with no spaces,
`{"error":{"code":"NOT_IMPLEMENTED","message":<M>,"requestId":<R>}}` where `<M>` is the `Unimplemented`'s Display
text and `<R>` the request id, each as a JSON string escaped by this rule: `"` → `\"`, `\` → `\\`, LF → `\n`,
CR → `\r`, TAB → `\t`, any other character below U+0020 (U+0008 and U+000C included) → `\u` plus four lower-case
hex digits; everything else (including `/` and non-ASCII) is written as is. Note this differs from
`rifts_relay::wire::json_string` (§4.2), which writes U+0008 and U+000C as `\b` and `\f`; keep both behaviours.

Example (from the tests): `Unimplemented { Realms, "realms.join" }` with request id `req "1"` gives

```text
{"error":{"code":"NOT_IMPLEMENTED","message":"rifts-realms: realms.join is not implemented yet","requestId":"req \"1\""}}
```

#### 2.1.6 Ids (`id`)

| Item | Shape | Derives | Behaviour |
| --- | --- | --- | --- |
| `IdError` | enum `BadUlid`, `BadNode`, `BadGlobal` | Debug, Clone, PartialEq, Eq, Error | Display: `a ULID is 26 Crockford base32 characters`; `a node id is 1 to 32 of a-z, 0-9 and '-', not starting or ending with '-'`; `a global id is <ULID>@<node>` |
| `Ulid` | tuple `(pub u128)` | Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Display, FromStr | below |
| `NodeId` | private `String` | Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Display | `fn new(name: &str) -> Result<Self, IdError>`; `fn as_str(&self) -> &str`; Display writes the name |
| `GlobalId` | pub fields `ulid: Ulid`, `node: NodeId` | Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Display, FromStr (`Err = IdError`) | `<ULID>@<node>` |
| `SessionId`, `PartyId`, `TicketId`, `RealmId`, `AdmissionId` | each a tuple `(pub GlobalId)` | Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Display, FromStr (`Err = IdError`) | Display and FromStr are `GlobalId`'s |
| `AccountId` | tuple `(pub String)` | Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Display | Display writes the string. **No** FromStr |
| `PeerId` | tuple `(pub u64)` | Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash | No Display |

`Ulid`:

- `const fn from_parts(unix_millis: u64, random: u128) -> Self`: the low 48 bits of `unix_millis` in bits 80..128,
  the low 80 bits of `random` in bits 0..80.
- `const fn unix_millis(self) -> u64`: bits 80..128.
- Display: 26 characters of the Crockford alphabet `0123456789ABCDEFGHJKMNPQRSTVWXYZ`, most significant first;
  character *i* (0-based) is bits `5*(25-i)` .. `5*(25-i)+5` of the value (the first character takes the top 3 bits,
  so it is `0`-`7`). `Ulid(u128::MAX)` is `7ZZZZZZZZZZZZZZZZZZZZZZZZZ`.
- FromStr: exactly 26 bytes, the first byte `0`-`7`, every byte (ASCII-uppercased) in the alphabet above (no
  aliases: `I`, `L`, `O`, `U` are refused); otherwise `BadUlid`. Lower case parses.

`NodeId::new`: 1 to 32 bytes, each `a`-`z`, `0`-`9` or `-`, not starting or ending with `-`; otherwise `BadNode`
(upper case is refused).

`GlobalId` FromStr: split at the first `@` (none → `BadGlobal`), the left part parsed as `Ulid` (`BadUlid`), the right
as `NodeId` (`BadNode`).

#### 2.1.7 Storage shapes (`store`)

| Item | Shape | Derives | Behaviour |
| --- | --- | --- | --- |
| `UnixMillis` | tuple `(pub i64)` | Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash | Unix time in milliseconds |
| `Secret` | private `String` | Clone, PartialEq, Eq; manual Debug | `fn new(value: String) -> Self`; `fn expose(&self) -> &str`; Debug writes exactly `Secret(<redacted>)` |
| `Actor` | enum `Account(AccountId)`, `HostRuntime { session: SessionId, host: AccountId }`, `Admin { key_id: String }`, `System(&'static str)` | Debug, Clone, PartialEq, Eq | — |
| `TableSpec` | pub fields `name: &'static str`, `node_collection: &'static str`, `owner: Component`, `keys: &'static str`, `expires_by: Option<&'static str>` | Debug, Clone, Copy, PartialEq, Eq | — |
| `DeletionAction` | enum `Delete`, `Detach` | Debug, Clone, Copy, PartialEq, Eq | — |
| `DeletionStep` | pub fields `table: &'static str`, `account_column: &'static str`, `action: DeletionAction` | Debug, Clone, Copy, PartialEq, Eq | — |
| `StoreError` | `#[non_exhaustive]` enum `Unimplemented(Unimplemented)` (transparent, `From`), `Database(String)` | Debug, Clone, PartialEq, Eq, Error | `Database` Display is the string |

#### 2.1.8 The dedicated-only default and the holder

| Item | Shape | Behaviour |
| --- | --- | --- |
| `DedicatedOnly` | unit struct; Debug, Clone, Copy, Default | Implements `Lifecycle`, `RealmsHooks`, `GuardHooks`, `RealmsEntry` (§2.3.2) |
| `RiftsParts` | pub fields `realms: Arc<dyn RealmsHooks>`, `entry: Arc<dyn RealmsEntry>`, `guard: Arc<dyn GuardHooks>`, `routes: Vec<Arc<dyn RouteHandler>>`; no derives | The component crates' answers |
| `Rifts` | private fields; `Clone` only (no Debug) | §2.3.3 |

`Rifts` methods: `fn dedicated_only() -> Self`; `fn new(parts: RiftsParts) -> Self`;
`fn load(settings: &dyn Settings, parts: impl FnOnce() -> RiftsParts) -> Self`; `fn loaded(&self) -> bool`;
`fn realms(&self) -> &dyn RealmsHooks`; `fn entry(&self) -> &dyn RealmsEntry`; `fn guard(&self) -> &dyn GuardHooks`;
`fn routes(&self) -> impl Iterator<Item = (&'static Route, &dyn RouteHandler)>`.

### 2.2 Behaviour that is not a plain data type

All of §2.1's types are inert data except: `GuardMode::from_setting` (§2.1.3), `NoCore` (§2.1.3),
`Reply::not_implemented` and `RequestInfo::header` (§2.1.5), the id parsers (§2.1.6), `Secret`'s Debug (§2.1.7),
and §2.3.

### 2.3 The dedicated-only implementation and module switch

#### 2.3.1 `p2p_module_enabled`

Reads `MYSTICPARADOX_P2P_MODULE` (contract key; §9). Missing is treated as empty. After Rust `trim`, the module is
**off** only when the value equals `0`, `false`, `off` or `no` in any ASCII case; anything else (including empty,
missing and `on`) is on.

#### 2.3.2 `DedicatedOnly`

Every async answer is an immediately ready future. Exact answers:

| Trait method | Answer |
| --- | --- |
| `Lifecycle::runtime_safety_issues` | `Ok(vec![])` |
| `startup_summary` | `Ok(None)` |
| `start` | `Ok(())` |
| `stop` | `Ok(())` |
| `tables` | empty slice |
| `account_deletion_steps` | empty vec |
| `RealmsHooks::metric_class` | `None` |
| `request_log_tags` | empty string |
| `authenticate` | `Ok(AuthOutcome::NotMine)` |
| `dedicated_hunts_disabled` | `Ok(false)` |
| `filters_offline_party_members` | `Ok(false)` |
| `restore_candidate`, `candidate_status`, `party_instance` | `Ok(None)` |
| `route_hunt` | `Ok(HuntRouteOutcome::NotHandled)` |
| `cancel_candidate` | `Ok(CandidateLeave::NotHandled)` |
| `party_member_left` | `Ok(())` |
| `launcher_policy_fields` | `Ok(vec![])` |
| `operations_policy_fields_valid` | `Ok(true)` |
| `operations_overview` | `Ok(OperationsOverview::default())` |
| `release_gates` | `Ok(vec![])` |
| `GuardHooks::service_roles`, `guard_manifest_roles` | empty slice |
| `census` | `Ok(())` |
| `rate_limit_bucket` | `Ok(None)` |
| `authorize_inventory` | `Ok(InventoryDecision::Refused(R1))` |
| `authorize_host_write` | `Ok(Some(R2))` |
| `record_host_write` | `Ok(())` |
| `host_policy_refuses` | `Ok(true)` |
| `note_would_refuse` | `Ok(())` |
| `RealmsEntry::authorize_game_session_realm` | `Ok(None)` when `realm` is `None`; otherwise `Err(SeamError::Refused(Refusal { code: "REALMS_DISABLED", message: "Realms are not available on this server." }))` |
| `bind_realm_presence_at_login`, `touch_presence` | `Ok(())` |
| `find_by_hub_room_id` | `Ok(None)` |
| `is_member`, `is_banned` | `Ok(false)` |
| `hub_room_id` | the same `REALMS_DISABLED` refusal |

`R1` and `R2` are `Reply { status: 403, body }` with bodies exactly:

- R1: `{"code":"forbidden","message":"This runtime cannot mutate inventory.","payload":null}`
- R2: `{"code":"forbidden","message":"This runtime cannot write player state.","payload":null}`

#### 2.3.3 `Rifts`

- `dedicated_only()`: `loaded() == false`; `realms()`, `entry()`, `guard()` are `DedicatedOnly`; no routes.
- `new(parts)`: `loaded() == true`; the parts as given.
- `load(settings, parts)`: when `p2p_module_enabled(settings)`, `new(parts())`; otherwise `dedicated_only()` and the
  closure is **never called** (a test passes a closure that panics).
- `routes()`: for each handler in `parts.routes` order, each of its `routes()` in order, paired with the handler.

---

## 3. `rifts-admission`

Admission: who may join a hosted hunt or hub. A joiner's launcher gets a signed ticket (MPT2); the joiner's transport
sends it to the host's transport in an MPC1 control frame; the host's transport checks the signature against the
public key file and asks the backend to consume it once.

Dependencies: `base64`, `getrandom`, `p256`, `rifts-relay`, `rifts-seam`, `serde_json`, `sha2`, `thiserror`.

### 3.1 Public API (complete)

Module layout that callers see: the root, and the public modules `control`, `keys`, `ticket` (the fuzz targets and
the vector harness import through these paths). `routes`, `service` and `store` are private modules whose items are
re-exported at the root.

Used outside the crate (grep): `isles-1-14-7` (`Admission`, `AdmissionError`, `AdmissionHunts`, `AdmissionRoutes`,
`AdmissionService`, `AdmissionStore`, `CapabilityResolver`, `Es256Tickets` with `from_settings`, `HostCapability`,
`KeyId`, `MemoryAdmissionStore`, `P256KeyGenerator::generate_with_pems`, `TicketAuthority`, `derive_ice_peer_id`,
`stub_routes`); `rifts-realms` (those plus `AdmissionHunt`, `JOIN_TICKET_LIFETIME_SECONDS`, `LinkStatus`,
`PeerAdmissionRecord`, `PeerLinkRecord`, `SessionUuid`, `TicketBindings`, `TicketRecord`, `TicketStatus`,
`AdmissionStore::put_peer_link`); fuzz (`control::*`, `ticket::{decode_claims, encode_claims, parse_unverified}`,
`keys::signing_key_from_pem`, `Es256Tickets` fields, `KeyId`, `P256KeyGenerator`, `TicketClaims`, `TicketError`).

#### 3.1.1 Root items

| Item | Shape | Notes |
| --- | --- | --- |
| `AdmissionError` | `#[non_exhaustive]` enum: `Unimplemented(Unimplemented)`, `Refused(Refusal)`, `Ticket(TicketError)`, `Key(KeyError)`, `Store(StoreError)` (all `From`, transparent), `Conflict` | Debug, Clone, PartialEq, Eq, Error. `Conflict` Display: `the record already exists` |
| `KeyId` | tuple `(pub String)` | Debug, Clone, PartialEq, Eq, Hash |
| `PublicXy` | tuple `(pub [u8; 64])` | Debug, Clone, Copy, PartialEq, Eq |
| `GeneratedKeyPair` | pub fields `key_id: KeyId`, `signing_key: Secret`, `verify_key: String`, `public_xy: PublicXy` | Debug, Clone, PartialEq, Eq; `fn env_lines(&self) -> Secret` (§3.3) |
| `KeyGenerator` | trait `Send + Sync`: `fn generate(&self, key_id: KeyId) -> Result<GeneratedKeyPair, AdmissionError>` | — |
| `TicketAuthority` | trait `Send + Sync`: `fn key_id(&self) -> KeyId`; `fn mint(&self, bindings: TicketBindings, now: UnixMillis, lifetime_seconds: Option<u32>) -> Result<MintedTicket, AdmissionError>`; `fn verify(&self, ticket: &str, now: UnixMillis) -> Result<TicketClaims, AdmissionError>` | Implemented by `Es256Tickets` (§3.2.8) and `Stub` |
| `derive_ice_peer_id` | `fn(account: &AccountId) -> PeerId` | §3.2.10 |
| `AdmissionService` | trait `Send + Sync`, five async methods (§3.6) | — |
| `Stub` | unit struct; Debug, Clone, Copy, Default | Implements `KeyGenerator`, `TicketAuthority`, `AdmissionService` with `Unimplemented(Admission, op)`: ops `generate`, `mint`, `verify`, `issue_self_ticket`, `create_join_bootstrap`, `consume_ticket`, `record_peer_disconnected`, `list_hub_peers`; `key_id()` is `KeyId("")` |
| Re-exports from `keys` | `KeyError`, `P256KeyGenerator`, `SECRET_SETTINGS`, `write_key_set` | — |
| Re-exports from `ticket` | `Es256Tickets`, `Jti`, `MintedTicket`, `SessionUuid`, `TicketBindings`, `TicketClaims`, `TicketError` | — |
| From `routes` | `AdmissionRoutes`, `CapabilityResolver`, `ROUTES`, `stub_routes` | §3.7 |
| From `service` | `Admission`, `AdmissionHunt`, `AdmissionHunts`, `ConsumeTicket`, `ConsumedTicket`, `HostCapability`, `HubPeer`, `HubPeers`, `IssuedTicket`, `JOIN_TICKET_LIFETIME_SECONDS`, `JoinBootstrap`, `MAXIMUM_PEER_LIST`, `refused` | §3.6 |
| From `store` | `AdmissionStore`, `ConsumeOutcome`, `ConsumeRequest`, `DisconnectRequest`, `LinkStatus`, `MemoryAdmissionStore`, `PeerAdmissionRecord`, `PeerLinkRecord`, `StubAdmissionStore`, `TABLES`, `TicketRecord`, `TicketStatus` | §3.5 |

#### 3.1.2 `ticket` module (all `pub`)

Constants: `TOKEN_PREFIX: &str = "mpt1"`, `PAYLOAD_MAGIC: &[u8; 4] = b"MPT2"`, `PAYLOAD_VERSION: u8 = 2`,
`FIXED_BYTES: usize = 106`, `MAX_TOKEN_BYTES: usize = 4096`, `MIN_LIFETIME_SECONDS: u32 = 15`,
`MAX_LIFETIME_SECONDS: u64 = 180`, `CLOCK_SKEW_SECONDS: u64 = 30`, `DEFAULT_LIFETIME_SECONDS: u32 = 75`.

Types and functions:

| Item | Shape |
| --- | --- |
| `TicketError` | enum (§3.2.9); Debug, Clone, PartialEq, Eq, Error; `fn is_expired(&self) -> bool` (true only for `Expired`) |
| `SessionUuid` | tuple `(pub [u8; 16])`; Debug, Clone, Copy, PartialEq, Eq, Hash; `fn from_bytes([u8; 16]) -> Result<Self, TicketError>`; FromStr (`Err = TicketError`); Display |
| `Jti` | tuple `(pub [u8; 16])`; Debug, Clone, Copy, PartialEq, Eq, Hash; `fn random() -> Result<Self, TicketError>`; `fn parse(text: &str) -> Result<Self, TicketError>`; `fn digest(&self) -> String`; Display |
| `hash_jti` | `fn(jti: &str) -> Result<String, TicketError>` |
| `TicketBindings` | pub fields `account: AccountId`, `joiner_peer: PeerId`, `host_peer: PeerId`, `session: SessionUuid`, `generation: u32`, `party_id: String`, `party_revision: u64`, `hunt_definition_id: String`, `build: u32`, `bridge_protocol: u16`, `virtual_port: u16`; Debug, Clone, PartialEq, Eq |
| `TicketClaims` | pub fields `key_id: KeyId`, `jti: Jti`, `bindings: TicketBindings`, `issued_at: u64`, `not_before: u64`, `expires_at: u64` (Unix seconds); Debug, Clone, PartialEq, Eq; `fn jti_digest(&self) -> String` |
| `safe_key_id` | `fn(text: &str) -> bool` |
| `encode_claims` | `fn(claims: &TicketClaims) -> Result<Vec<u8>, TicketError>` |
| `decode_claims` | `fn(payload: &[u8]) -> Result<TicketClaims, TicketError>` |
| `Es256Tickets` | pub fields `key_id: KeyId`, `signing: Option<p256::ecdsa::SigningKey>`, `verifying: p256::ecdsa::VerifyingKey`, `default_lifetime: f64`; Clone; manual Debug |
| `MintedTicket` | pub fields `text: Secret`, `claims: TicketClaims`, `jti_digest: String`; Debug, Clone, PartialEq, Eq |
| `Es256Tickets::mint` (inherent) | `fn(&self, bindings: TicketBindings, now_seconds: u64, lifetime_seconds: Option<u32>) -> Result<MintedTicket, TicketError>` |
| `Es256Tickets::sign` | `fn(&self, claims: TicketClaims) -> Result<MintedTicket, TicketError>` |
| `Es256Tickets::verify` (inherent) | `fn(&self, token: &str, now_seconds: u64) -> Result<TicketClaims, TicketError>` |
| `parse_signed` | `fn(token: &str, verify: impl FnOnce(&[u8], &p256::ecdsa::Signature) -> bool) -> Result<TicketClaims, TicketError>` |
| `check_lifetime` | `fn(claims: &TicketClaims, now_seconds: u64) -> Result<(), TicketError>` |
| `parse_unverified` | `fn(token: &str) -> Result<TicketClaims, TicketError>` |

Note the inherent `mint`/`verify` (seconds, `TicketError`) and the `TicketAuthority` trait methods of the same names
(`UnixMillis`, `AdmissionError`) coexist; method-call syntax resolves to the inherent ones, which the vector harness
and the example rely on.

#### 3.1.3 `keys` module (all `pub`)

Constants: `KEY_ID_SETTING = "P2P_ADMISSION_KEY_ID"`, `SIGNING_KEY_SETTING = "P2P_ADMISSION_SIGNING_PRIVKEY_B64"`,
`VERIFY_KEY_SETTING = "P2P_ADMISSION_VERIFY_PUBKEY_B64"`, `LIFETIME_SETTING = "P2P_ADMISSION_TICKET_TTL_SECONDS"`,
`SECRET_SETTINGS: &[&str] = &[SIGNING_KEY_SETTING]`, `PRIVATE_PEM_FILE = "admission-private.pkcs8.pem"`,
`PUBLIC_PEM_FILE = "admission-public.spki.pem"`, `PUBLIC_XY_FILE = "p2p-admission-public.xy"`,
`ENV_FILE = "metagame-admission.env"`.

| Item | Shape |
| --- | --- |
| `KeyError` | enum (§3.3.5); Debug, Clone, PartialEq, Eq, Error |
| `signing_key_from_pem` | `fn(pem: &str) -> Option<SigningKey>` |
| `verifying_key_from_pem` | `fn(pem: &str) -> Option<VerifyingKey>` |
| `key_id_from_settings` | `fn(settings: &dyn Settings) -> Result<KeyId, KeyError>` |
| `Es256Tickets::from_settings` | `fn(settings: &dyn Settings) -> Result<Es256Tickets, KeyError>` |
| `Es256Tickets::public_xy` | `fn(&self) -> PublicXy` |
| `public_xy` | `fn(key: &VerifyingKey) -> PublicXy` |
| `P256KeyGenerator` | unit struct; Debug, Clone, Copy, Default; implements `KeyGenerator` |
| `P256KeyGenerator::generate_with_pems` | `fn(&self, key_id: KeyId) -> Result<(GeneratedKeyPair, Secret, String), KeyError>` (pair, private PEM, public PEM) |
| `write_key_set` | `fn(output: &Path, key_id: KeyId, process_id: u32, now_millis: i64) -> Result<GeneratedKeyPair, KeyError>` |

#### 3.1.4 `control` module (all `pub`)

Constants `CONTROL_MAGIC: &[u8; 4] = b"MPC1"`, `CONTROL_VERSION: u8 = 1`, `ADMISSION_REQUEST: u8 = 1`,
`ADMISSION_RESULT: u8 = 2`, `HEADER_BYTES: usize = 12`, `MAX_CONTROL_FRAME: usize = 4608`.
`AdmissionResult` (pub fields `admitted: bool`, `code: Vec<u8>`, `admission_id: Vec<u8>`; Debug, Clone, PartialEq, Eq).
Functions `encode_admission_request(ticket: &[u8]) -> Option<Vec<u8>>`, `decode_admission_request(input: &[u8]) ->
Option<&[u8]>` (borrows from the input), `encode_admission_result(result: &AdmissionResult) -> Option<Vec<u8>>`,
`decode_admission_result(input: &[u8]) -> Option<AdmissionResult>`.

### 3.2 The MPT2 ticket

#### 3.2.1 Text form

`mpt1.<P>.<S>`: the literal prefix `mpt1` (lower case, even though the payload magic is `MPT2`), a dot, the payload
`P` in base64url **without padding**, a dot, the signature `S` in base64url without padding. `S` decodes to exactly
64 bytes: the ECDSA P-256 signature over the raw payload bytes with SHA-256, in IEEE P1363 form (`r || s`, each 32
bytes big-endian). The whole token is at most 4096 bytes. A signed token is at most about 830 characters, so the
4096-byte limit is only reached by hostile input.

#### 3.2.2 Payload layout

Big-endian throughout. 106 fixed bytes, then four identifier strings with no separators:

| Offset | Size | Field | Encoding |
| ---: | ---: | --- | --- |
| 0 | 4 | magic | ASCII `MPT2` |
| 4 | 1 | version | `2` |
| 5 | 1 | key id length | u8 |
| 6 | 2 | account length | u16 |
| 8 | 2 | hunt definition length | u16 |
| 10 | 2 | party id length | u16 |
| 12 | 2 | bridge protocol | u16 |
| 14 | 2 | virtual port | u16 |
| 16 | 2 | reserved | `0` |
| 18 | 4 | generation | u32 |
| 22 | 4 | build | u32 |
| 26 | 8 | party revision | u64 |
| 34 | 8 | issued at | u64 Unix seconds |
| 42 | 8 | not before | u64 Unix seconds |
| 50 | 8 | expires at | u64 Unix seconds |
| 58 | 8 | joiner peer id | u64 |
| 66 | 8 | host peer id | u64 |
| 74 | 16 | session | UUID bytes |
| 90 | 16 | jti | 16 random bytes |
| 106 | k | key id | ASCII |
| 106+k | a | account id | ASCII |
| 106+k+a | h | hunt definition id | ASCII |
| 106+k+a+h | p | party id | ASCII |

Worked example (from the tests): key `k`, account `account-1`, hunt `hunt/island-1`, party `party_abc==`, bridge
protocol 1, virtual port 3, generation 7, build 392819: bytes 0..6 are `MPT2 02 01`, 6..18 are
`00 09 00 0d 00 0b 00 01 00 03 00 00`, 18..26 are `00 00 00 07 00 05 fe 73`, and the tail is
`kaccount-1hunt/island-1party_abc==`. The signature text is 86 characters.

#### 3.2.3 Field rules (the check, in this order; the first failure is the error)

1. Identifiers, all four checked together, any failure is `UnsafeIdentifier`:
   key id 1-32 bytes of `[A-Za-z0-9._-]`; account 1-96 of `[A-Za-z0-9._:-]`; hunt definition 1-128 of
   `[A-Za-z0-9._:/-]`; party id 1-192 of `[A-Za-z0-9._:+/=-]`. (`safe_key_id` is the key-id rule alone.)
2. Virtual port > 999 → `OutOfRange("virtualPort")`.
3. Virtual port = 0 → `ZeroVirtualPort`.
4. Party revision > 2^53 - 1 → `OutOfRange("partyRevision")`.
5. Joiner or host peer id = 0 → `InvalidPeerId`.
6. Session bytes not a UUID of version 1-5 (high nibble of byte 6 in 1..=5) and variant 8-b (high nibble of byte 8 in
   8..=0xb) → `SessionNotUuid`.

Generation, build and bridge protocol may be 0 (Node allows it; the transport does not; §9).

`encode_claims` runs these checks and then writes §3.2.2. The key id length is written as one byte, the other three
as u16.

#### 3.2.4 Canonical payload rule (`decode_claims`)

1. Fewer than 106 bytes, or the first four not `MPT2` → `MalformedPayload`.
2. Version ≠ 2, reserved ≠ 0, total length ≠ 106 + the four declared lengths, or any declared length 0 →
   `Framing`.
3. Read every field. Identifiers are decoded as UTF-8 with replacement characters (so invalid UTF-8 then fails the
   identifier rule rather than erroring here).
4. Re-encode with `encode_claims`; its error (step order of §3.2.3) is returned as is. In particular a party revision
   above 2^53 - 1 reports `OutOfRange("partyRevision")`.
5. If the re-encoding differs from the input in any byte, or any of issued at, not before, expires at has no exact
   IEEE-754 double (converting to f64 and back does not give the same u64, or the double is ≥ 2^64), →
   `NoncanonicalPayload`.

So only the one canonical payload for a set of claims decodes.

#### 3.2.5 Session and jti text forms

- `SessionUuid` FromStr: exactly 36 bytes with `-` at offsets 8, 13, 18, 23; with every `-` removed, exactly 32 hex
  digits in either case; then the version/variant rule. Any failure → `SessionNotUuid`. Display: lower-case
  `8-4-4-4-12` hex.
- `Jti::parse`: exactly 32 hex digits, either case; else `InvalidJti`. Display: 32 lower-case hex digits.
- `Jti::random`: 16 bytes from the OS random source (`getrandom`); failure → `Random`.
- `Jti::digest` (and `TicketClaims::jti_digest`, `hash_jti`): SHA-256 of the 16 raw bytes, 64 lower-case hex
  digits. This digest is also the peer admission's id. `hash_jti("A"*32) == hash_jti("a"*32)`.

#### 3.2.6 Minting (`Es256Tickets::mint`)

1. Lifetime = the argument if given, else `default_lifetime` (an f64). It must be a whole number in 15..=180,
   otherwise `LifetimeSetting` (NaN, 0, 14, 181, 75.5 all fail).
2. Claims: the authority's key id, a fresh `Jti::random()`, the bindings, `issued_at = not_before = now_seconds`,
   `expires_at = now_seconds + lifetime`.
3. `sign(claims)`: `encode_claims` (its errors first), then no signing key → `NoSigningKey`, then sign the payload,
   build the text form, and if it exceeds 4096 bytes → `TokenTooLarge`. The result's `jti_digest` is the claims'.

Signatures may be deterministic (RFC 6979) or random; both verify everywhere (Node signs randomly).

#### 3.2.7 Verification (`parse_signed`, `check_lifetime`, `Es256Tickets::verify`)

`parse_signed(token, verify)`:

1. Token longer than 4096 bytes → `MissingOrOversized`.
2. Split on every `.`; not exactly three parts → `MalformedToken`. First part ≠ `mpt1`, or payload or signature part
   empty → `MalformedToken`.
3. Payload part must decode as unpadded base64url and re-encode to the identical text (this refuses `=` padding,
   `+`/`/`, and non-zero trailing bits) → else `NoncanonicalEncoding`.
4. Signature part: the same canonical rule, and exactly 64 decoded bytes → else `NoncanonicalEncoding`.
5. Interpret as P1363 `r || s`; if `r` or `s` is zero or not below the curve order → `BadSignature`.
6. Call `verify(payload, signature)`; `false` → `BadSignature`. The signature is checked **before** the payload is
   decoded.
7. `decode_claims(payload)`.

`Es256Tickets::verify(token, now_seconds)`: `parse_signed` with ECDSA P-256/SHA-256 verification under `verifying`;
high-S signatures **are accepted** (no low-S normalisation requirement, as Node and BCrypt accept them); then claims'
key id ≠ the authority's → `KeyNotActive`; then `check_lifetime`.

`check_lifetime(claims, now)`:

1. `expires_at < issued_at`, or `not_before < issued_at`, or `expires_at - issued_at > 180` → `InvalidLifetime`.
2. `issued_at > now + 30` or `not_before > now + 30` (saturating) → `NotYetValid`.
3. `now > expires_at + 30` (saturating) → `Expired`.

So a 75-second ticket minted at `T` verifies from `T - 30` to `T + 105` inclusive.

`parse_unverified(token)` is `parse_signed` with a verifier that accepts everything (framing and payload checks only).

#### 3.2.8 `TicketAuthority` for `Es256Tickets`

`key_id()` clones the field. `mint(bindings, now, lifetime)` and `verify(ticket, now)` convert `now` to seconds as
`floor(now.0 / 1000)` (Euclidean), with a negative result taken as 0, call the inherent methods and wrap errors as
`AdmissionError::Ticket`.

`Es256Tickets`' Debug shows `key_id`, `signing` as `Some("<redacted>")` or `None`, and `default_lifetime`, and never
the key material.

#### 3.2.9 `TicketError` variants and Display texts (contract: the vectors compare them)

| Variant | Display |
| --- | --- |
| `UnsafeIdentifier` | `admission ticket contains an unsafe identifier` |
| `OutOfRange(&'static str)` | `{0} is outside its unsigned range` (`virtualPort`, `partyRevision`, `generation`) |
| `ZeroVirtualPort` | `virtualPort must be a nonzero Steam logical port` |
| `InvalidPeerId` | `invalid SteamID64` |
| `SessionNotUuid` | `sessionId must be a canonical UUID` |
| `LifetimeSetting` | `P2P admission ticket lifetime must be between 15 and 180 seconds` |
| `TokenTooLarge` | `admission token exceeds size limit` |
| `MissingOrOversized` | `admission token is missing or oversized` |
| `MalformedToken` | `malformed admission token` |
| `NoncanonicalEncoding` | `noncanonical admission token encoding` |
| `BadSignature` | `invalid admission token signature` |
| `MalformedPayload` | `malformed admission ticket payload` |
| `Framing` | `invalid admission ticket framing` |
| `NoncanonicalPayload` | `noncanonical admission ticket payload` |
| `KeyNotActive` | `admission token key is not active` |
| `InvalidLifetime` | `invalid admission token lifetime` |
| `NotYetValid` | `admission token is not yet valid` |
| `Expired` | `admission token expired` |
| `InvalidJti` | `invalid admission JTI` |
| `NoSigningKey` | `Missing required P2P admission key P2P_ADMISSION_SIGNING_PRIVKEY_B64.` |
| `Random` | `the system random source failed` |

#### 3.2.10 ICE peer ids (`derive_ice_peer_id`)

SHA-256 of the UTF-8 bytes of `mysticparadox-ice-peer-v1` + LF + the account id text; the first 8 digest bytes as a
big-endian u64; clear the top bit (`& 0x7fff_ffff_ffff_ffff`); if the result is 0, use 1. The label is a contract
(§9).

### 3.3 Keys

#### 3.3.1 Formats

- **Signing key** (`P2P_ADMISSION_SIGNING_PRIVKEY_B64`, a secret in `secrets.env`): standard base64 (with padding) of
  the PEM text of a PKCS#8 `-----BEGIN PRIVATE KEY-----` P-256 key with LF line endings. A SEC1
  `-----BEGIN EC PRIVATE KEY-----` PEM is also read.
- **Verify key** (`P2P_ADMISSION_VERIFY_PUBKEY_B64`): standard base64 of the SPKI `-----BEGIN PUBLIC KEY-----` PEM,
  LF line endings. Only SPKI is read.
- **Public XY file** (`p2p-admission-public.xy`): exactly 64 bytes, the uncompressed SEC1 point without its `0x04`
  tag: X (32 bytes big-endian) then Y (32 bytes). This is what the transport hands to BCrypt.

#### 3.3.2 Reading settings (`key_id_from_settings`, `Es256Tickets::from_settings`)

1. Key id: `P2P_ADMISSION_KEY_ID` (missing = empty) must pass `safe_key_id`, else `KeyError::KeyId`.
2. Verify key: missing or empty → `Missing("P2P_ADMISSION_VERIFY_PUBKEY_B64")`. Decode leniently (below); the bytes as
   UTF-8 with replacement; parse as SPKI PEM; any failure → `Unreadable("P2P_ADMISSION_VERIFY_PUBKEY_B64")`.
3. Signing key: missing or empty → `signing = None` (a verify-only authority; minting fails later with
   `NoSigningKey`). Present → lenient decode, UTF-8 with replacement, parse as PKCS#8 PEM then SEC1 PEM; failure →
   `Unreadable("P2P_ADMISSION_SIGNING_PRIVKEY_B64")`.
4. `default_lifetime`: setting missing → 75.0; present → `js_number(value)` (§4.2: blank is 0, garbage is NaN); not
   validated until minting.

Lenient base64 (Node's `Buffer.from(text, "base64")`): take characters up to the first `=`; map `-` → `+` and `_` →
`/`; drop every character outside `[A-Za-z0-9+/]` (line breaks, spaces, stray characters); if the remaining length
mod 4 is 1, drop its last character; decode without padding, ignoring non-zero trailing bits. A decode failure is
`Unreadable`.

#### 3.3.3 Generation (`P256KeyGenerator`)

`generate_with_pems(key_id)`: unsafe key id → `KeyError::KeyId`. A new P-256 key from the OS random source (failure
→ `Random`); PKCS#8 PEM and SPKI PEM with LF endings; `signing_key` = standard base64 of the private PEM text,
`verify_key` = standard base64 of the public PEM text, `public_xy` per §3.3.1. Returns the pair, the private PEM (as
`Secret`) and the public PEM. `KeyGenerator::generate` returns only the pair, errors as `AdmissionError::Key`.

`GeneratedKeyPair::env_lines()`: a `Secret` holding exactly three LF-terminated lines:
`P2P_ADMISSION_KEY_ID=<id>`, `P2P_ADMISSION_SIGNING_PRIVKEY_B64=<signing_key>`,
`P2P_ADMISSION_VERIFY_PUBKEY_B64=<verify_key>`.

#### 3.3.4 Writing a key set (`write_key_set`)

1. Unsafe key id → `KeyError::Usage`.
2. Make `output` absolute (`std::path::absolute`; failure → `Io(text)`). Its parent must be an existing directory,
   else `NoParent(parent)`. If `output` exists → `Exists(absolute output)`.
3. Create the temporary directory `<parent>/.admission-key-<process_id>-<now_millis>` (failure → `Io`).
4. Generate, then create (create-new, never overwrite) and fsync four files in it: `admission-private.pkcs8.pem` (the
   private PEM; mode 0600 on Unix), `admission-public.spki.pem` (public PEM), `p2p-admission-public.xy` (64 bytes),
   `metagame-admission.env` (`env_lines`; mode 0600 on Unix). Then rename the temporary directory to `output`.
5. On any failure in step 4, remove the temporary directory recursively and return the error (`Io(text)` for I/O).

#### 3.3.5 `KeyError` variants and Display

| Variant | Display |
| --- | --- |
| `KeyId` | `P2P_ADMISSION_KEY_ID is missing or invalid.` |
| `Missing(&'static str)` | `Missing required P2P admission key {0}.` |
| `Unreadable(&'static str)` | `{0} is not a P-256 key in base64 PEM` |
| `Random` | `the system random source failed` |
| `Usage` | `Usage: --output <new-directory> --key-id <safe-id>` |
| `NoParent(PathBuf)` | `The output parent does not exist: {path}` (path in `Path::display` form) |
| `Exists(PathBuf)` | `Refusing to overwrite an existing key directory: {path}` |
| `Io(String)` | `could not write the key set: {0}` |

### 3.4 MPC1 control frames

Header (12 bytes, big-endian): `MPC1`, version `1`, type (`1` request, `2` result), two zero bytes, payload length as
u32. Then the payload. A frame is at most 4608 bytes in total and its payload is never empty.

- **Request payload**: ticket length (u16), then the ticket bytes. Ticket 1-4096 bytes.
- **Result payload**: `admitted` (0 or 1), code length (u8), admission id length (u8), one zero byte, the code bytes,
  the admission id bytes. Code 1-64 bytes; id 0-64 bytes; when admitted, the id is exactly 64 bytes.

Encoding: `encode_admission_request` → `None` for an empty ticket or one over 4096 bytes. `encode_admission_result` →
`None` for an empty code, a code over 64 or an id over 64 bytes (an admitted result with a short id still encodes;
it then fails to decode). Both build the header with the declared length equal to the payload length.

Decoding (`None` on any failure): total length 12..=4608; magic, version 1, the expected type, bytes 6-7 zero;
declared length non-zero and equal to the bytes after the header. Request: payload ≥ 3 bytes, the u16 length
non-zero, ≤ 4096 and equal to payload length - 2; returns the ticket slice. Result: payload ≥ 5 bytes, first byte ≤ 1,
fourth byte 0, code length 1-64, id length ≤ 64, payload length = 4 + both; if admitted, id length must be 64.

Example: the request for the 12-byte ticket `mpt1.abc.def` starts `4d 50 43 31 01 01 00 00 00 00 00 0e 00 0c`.

### 3.5 Store

#### 3.5.1 Tables (`TABLES`, in this order, all `owner: Component::Admission`, `expires_by: None`)

| `name` | `node_collection` | `keys` |
| --- | --- | --- |
| `rifts_peer_links` | `p2pSteamLinks` | `account; peer unique` |
| `rifts_admission_tickets` | `p2pAdmissionTickets` | `jti digest; session + generation + account` |
| `rifts_peer_admissions` | `p2pPeerAdmissions` | `admission; active admission key unique` |

#### 3.5.2 Records (all pub fields; Debug, Clone, PartialEq, Eq)

| Type | Fields |
| --- | --- |
| `LinkStatus` | enum `Verified`, `Revoked` (+ Copy) |
| `PeerLinkRecord` | `account: AccountId`, `peer: PeerId`, `status: LinkStatus`, `revision: u64`, `verified_at: UnixMillis`, `verification_method: String` |
| `TicketStatus` | enum `Issued`, `Consumed`, `Revoked` (+ Copy) |
| `TicketRecord` | `jti_digest: String`, `key_id: String`, `session: String` (wire text), `generation: u64`, `account: AccountId`, `joiner_peer: PeerId`, `host_peer: PeerId`, `bridge_protocol: u16`, `virtual_port: u16`, `issued_at: UnixMillis`, `expires_at: UnixMillis`, `status: TicketStatus`, `consumed_at: Option<UnixMillis>`, `consumer_bridge_instance_id: Option<String>`, `consumer_request_id: Option<String>` |
| `ConsumeRequest` | `jti_digest: String`, `session: String`, `generation: u64`, `account: AccountId`, `joiner_peer: PeerId`, `host_peer: PeerId`, `bridge_instance_id: String`, `request_id: String`, `now: UnixMillis` |
| `ConsumeOutcome` | enum `Consumed(TicketRecord)`, `Idempotent(TicketRecord)`, `Expired`, `Replay`, `Invalid` |
| `PeerAdmissionRecord` | `admission_id: String`, `active_key: Option<String>`, `session: String`, `generation: u64`, `account: AccountId`, `peer: PeerId`, `bridge_instance_id: String`, `connection_handle: String`, `admitted_at: UnixMillis`, `disconnected_at: Option<UnixMillis>`, `superseded_at: Option<UnixMillis>` |
| `DisconnectRequest` | `admission_id: String`, `session: String`, `generation: u64`, `bridge_instance_id: String`, `at: UnixMillis` |

Only the jti digest is stored, never the ticket text.

#### 3.5.3 `AdmissionStore` (trait, `Send + Sync`, sync methods, errors `StoreError`)

| Method | Signature |
| --- | --- |
| `peer_link` | `(&self, account: &AccountId) -> Result<Option<PeerLinkRecord>, StoreError>` |
| `put_peer_link` | `(&self, actor: &Actor, link: &PeerLinkRecord) -> Result<(), StoreError>` |
| `insert_ticket` | `(&self, actor: &Actor, ticket: &TicketRecord) -> Result<bool, StoreError>` |
| `consume_ticket` | `(&self, actor: &Actor, request: &ConsumeRequest) -> Result<ConsumeOutcome, StoreError>` |
| `insert_admission` | `(&self, actor: &Actor, admission: &PeerAdmissionRecord) -> Result<bool, StoreError>` |
| `find_admission` | `(&self, admission_id: &str) -> Result<Option<PeerAdmissionRecord>, StoreError>` |
| `supersede_active` | `(&self, actor: &Actor, active_key: &str, at: UnixMillis) -> Result<bool, StoreError>` |
| `disconnect` | `(&self, actor: &Actor, request: &DisconnectRequest) -> Result<bool, StoreError>` |

`StubAdmissionStore` (unit; Debug, Clone, Copy, Default): every method returns
`StoreError::Unimplemented(Unimplemented(Admission, "store.<method name>"))`, e.g. `store.peer_link`.

#### 3.5.4 `MemoryAdmissionStore` semantics

One process, a mutex (a poisoned lock is recovered, never a panic), three maps: links by account, tickets by jti
digest, admissions by admission id. `Default`, Debug. The `actor` argument is ignored. Extra method
`fn ticket(&self, jti_digest: &str) -> Option<TicketRecord>`.

- `peer_link`: the stored link or `None`. `put_peer_link`: insert or replace by account.
- `insert_ticket`: `false` if the digest exists; else store and `true`.
- `consume_ticket` (compare-and-set on status). No ticket for the digest → `Invalid`. Let *match* = same session,
  generation and joiner peer.
  1. Status `Issued`, *match*, same account, same host peer, and `expires_at > now` → set status `Consumed`,
     `consumed_at = now`, record the request's bridge instance id and request id; return `Consumed(updated record)`.
  2. Else status `Consumed` → `Idempotent(record)` when the stored consumer bridge instance id and consumer request
     id both equal the request's and *match* holds; otherwise `Replay`.
  3. Else (`Issued` with a mismatch or expired, or `Revoked`) → `Expired` when `expires_at <= now`, otherwise
     `Invalid`. Nothing changes.
- `insert_admission`: `false` if the admission id exists or the record's `active_key` (when `Some`) is held by any
  stored admission; else store and `true`.
- `find_admission`: by id.
- `supersede_active(key, at)`: the admission holding `active_key == key` gets `disconnected_at = at`,
  `superseded_at = at`, `active_key = None`; `true`. None holds it → `false`.
- `disconnect`: unknown id → `false`; session, generation or bridge instance id differ, or already disconnected →
  `false`; else `disconnected_at = at`, `active_key = None`, `true`. A second disconnect is `false`; afterwards a new
  admission with the same active key can be inserted.

### 3.6 Service

#### 3.6.1 Types

Constants: `JOIN_TICKET_LIFETIME_SECONDS: u32 = 180`, `MAXIMUM_PEER_LIST: usize = 64`; peer lists per hour per hub
generation: 3600 (private).

`refused(code: &str, message: &str) -> AdmissionError`: `AdmissionError::Refused(Refusal { code, message })`.

| Type | Shape (pub fields) | Derives |
| --- | --- | --- |
| `AdmissionHunt` | `snapshot: rifts_relay::rules::HuntSnapshot`, `session: SessionUuid`, `party_id: String`, `party_revision: u64`, `hunt_definition_id: String`, `build: u32`, `bridge_protocol: u16`, `virtual_port: u16`, `lobby_id: Option<String>`, `bridge_instance_id: Option<String>` | Debug, Clone, PartialEq, Eq |
| `HostCapability` | `session: SessionId`, `wire_session: String`, `generation: u64`, `host_account: AccountId`, `host_peer: PeerId`, `bridge_instance_id: String` | Debug, Clone, PartialEq, Eq |
| `IssuedTicket` | `ticket: MintedTicket`, `hunt: AdmissionHunt` | Debug, Clone, PartialEq, Eq |
| `JoinBootstrap` | `common: Json`, `ticket: Secret`, `lobby_id: String` | Debug, Clone, PartialEq, Eq |
| `ConsumeTicket` | `ticket: Secret`, `authenticated_peer: String`, `bridge_instance_id: String`, `connection_handle: String`, `request_id: String` | Debug, Clone, PartialEq, Eq |
| `ConsumedTicket` | `idempotent: bool`, `admission_id: String`, `account: AccountId`, `peer: PeerId`, `wire_session: String`, `generation: u32` | Debug, Clone, PartialEq, Eq |
| `HubPeer` | `account: AccountId`, `peer: PeerId`, `pair_key: Secret`, `state: &'static str` | Debug, Clone, PartialEq, Eq |
| `HubPeers` | `generation: u64`, `peers: Vec<HubPeer>` | Debug, Clone, PartialEq, Eq |
| `Admission` | pub `tickets: Arc<dyn TicketAuthority>`, `store: Arc<dyn AdmissionStore>`, `hunts: Arc<dyn AdmissionHunts>`, `transport_key_secret: Option<Secret>`, `clock: Arc<dyn Fn() -> UnixMillis + Send + Sync>`, plus a private hourly counter | none |

`Admission::new(tickets, store, hunts, transport_key_secret, clock) -> Self` (that argument order).

`HostCapability::from_runtime(runtime: &RuntimeAuth) -> Option<Self>`: `None` if `host_peer` or `bridge_instance` is
absent; otherwise the session, generation and host account copied, `wire_session` = `runtime.wire_session` or, when
absent, the 26-character ULID text of `runtime.session`.

`AdmissionHunts` (trait, `Send + Sync`, sync, errors `AdmissionError`), implemented by realms:

| Method | Signature | Meaning |
| --- | --- | --- |
| `gate` | `(&self) -> Result<P2PGate, AdmissionError>` | `rifts_relay::P2PGate { enabled, emergency_stop }` |
| `find_hunt` | `(&self, session: &SessionId) -> Result<Option<AdmissionHunt>, _>` | — |
| `reserve_participant` | `(&self, hunt: &AdmissionHunt, account: &AccountId, now: UnixMillis) -> Result<Option<AdmissionHunt>, _>` | The participant's next attempt within the rejoin window and limit; the hunt after, or `None` when refused |
| `check_hub_entry` | `(&self, hub: &AdmissionHunt, account: &AccountId, link: Option<&PeerLinkRecord>, now: UnixMillis) -> Result<PeerId, _>` | The member's peer id, or the refusal |
| `mark_hub_ticketed` | `(&self, hub: &AdmissionHunt, account: &AccountId, now: UnixMillis, ticket_expires_at: UnixMillis) -> Result<bool, _>` | — |
| `mark_participant_admitted` | `(&self, hunt: &AdmissionHunt, account: &AccountId, admitted_at: UnixMillis) -> Result<bool, _>` | — |
| `admit_hub_peer` | `(&self, hub: &AdmissionHunt, account: &AccountId, admission_id: &str, admitted_at: UnixMillis) -> Result<(), _>` | — |
| `peer_disconnected` | `(&self, admission: &PeerAdmissionRecord, at: UnixMillis) -> Result<(), _>` | — |
| `hub_presence` | `(&self, hub: &AdmissionHunt, limit: usize) -> Result<Vec<PresenceRow>, _>` | — |
| `join_bootstrap_common` | `(&self, hunt: &AdmissionHunt, key_id: &str, bridge_instance_id: &str) -> Result<Option<Json>, _>` | Realms' bootstrap fields as one JSON object, or `None` |

`AdmissionService` (trait, `Send + Sync`), each returning `BoxFuture<'_, Result<_, AdmissionError>>`:
`issue_self_ticket(account: AccountId, session: SessionId) -> IssuedTicket`;
`create_join_bootstrap(account: AccountId, session: SessionId) -> JoinBootstrap`;
`consume_ticket(capability: HostCapability, request: ConsumeTicket) -> ConsumedTicket`;
`record_peer_disconnected(capability: HostCapability, admission_id: String, disconnected_at: Option<UnixMillis>) ->
bool`; `list_hub_peers(account: AccountId, session: SessionId, generation: Option<i64>) -> HubPeers`. `Admission`
implements it; each operation runs synchronously and returns a ready future.

The "bindings" of an `AdmissionHunt` for an account and joiner peer: the account, the joiner peer,
`host_peer = snapshot.host_peer`, the hunt's `session`, `generation = snapshot.generation` as u32 (larger →
`AdmissionError::Ticket(OutOfRange("generation"))`), and the hunt's party id, party revision, hunt definition id,
build, bridge protocol and virtual port.

Recording a minted ticket (`insert_ticket`) writes a `TicketRecord` from the claims: digest, key id,
`session = snapshot.wire_session`, `generation = snapshot.generation`, account, joiner and host peers, bridge protocol,
virtual port, `issued_at` and `expires_at` as the claims' seconds × 1000, status `Issued`, no consumer fields. A
`false` from the store is `AdmissionError::Conflict`. The actor is `Actor::Account(account)`.

#### 3.6.2 Refusal texts used below

| Code | Message (exact) | Where |
| --- | --- | --- |
| `P2P_DISABLED` | `Player-hosted hunts are currently disabled.` | self ticket, consume: gate not enabled |
| `P2P_DISABLED` | `Player-hosted hunts are disabled.` | join bootstrap, hub peers: gate not enabled |
| `P2P_DISABLED` | `Player-hosted hunts are currently unavailable.` | emergency stop in self ticket, consume, disconnect |
| `P2P_EMERGENCY_STOP` | `Player-hosted hunts are currently stopped by operations policy.` | hub peers |
| `P2P_SESSION_NOT_FOUND` | `Hunt session not found.` | self ticket, consume |
| `P2P_SESSION_NOT_FOUND` | `Hub session not found.` | hub peers |
| `P2P_SESSION_NOT_READY` | `The hunt is not accepting participants.` | self ticket |
| `P2P_SESSION_NOT_READY` | `The rejoin window or attempt limit has been reached.` | self ticket |
| `P2P_SESSION_NOT_READY` | `The host lobby is not ready.` | join bootstrap |
| `P2P_SESSION_NOT_READY` | `The hub is not running.` | hub peers |
| `P2P_NOT_ROSTERED` | `This account is not in the frozen hunt roster.` | self ticket |
| `P2P_NOT_ROSTERED` | `This account has no open entry into the hub.` | hub ticket |
| `P2P_NOT_ROSTERED` | `Only the hub's host may list its peers.` | hub peers |
| `P2P_STEAM_LINK_REQUIRED` | `The current Steam co-op transport no longer matches this hunt.` | self ticket |
| `P2P_TICKET_INVALID` | `The admission request is malformed.` | consume body |
| `P2P_TICKET_INVALID` / `P2P_TICKET_EXPIRED` | `The admission ticket is invalid or expired.` | consume: verify failed |
| `P2P_TICKET_INVALID` | `The ticket does not match the authenticated Steam connection.` | consume |
| `P2P_TICKET_INVALID` | `The admission ticket is not valid for this hunt.` | consume: store `Invalid` |
| `P2P_TICKET_EXPIRED` | `The admission ticket expired.` | consume: store `Expired` |
| `P2P_TICKET_REPLAYED` | `The admission ticket was already consumed.` | consume: store `Replay` |
| `P2P_TICKET_INVALID` | `Invalid peer admission identifier.` | disconnect |
| `P2P_TICKET_INVALID` | `The hub generation is missing.` | hub peers |
| `P2P_GENERATION_FENCED` | `The admission ticket does not match the active host generation.` | consume: capability mismatch |
| `P2P_GENERATION_FENCED` | `This hunt generation no longer accepts admission.` | consume: hunt mismatch |
| `P2P_GENERATION_FENCED` | `The participant admission raced a fenced generation.` | consume: mark admitted failed |
| `P2P_GENERATION_FENCED` | `This request belongs to a stale hub generation.` | hub peers |
| `P2P_ICE_UNAVAILABLE` | `The co-op transport is not configured on this server.` | hub peers |
| `P2P_SIGNAL_LIMIT` | `Too many peer list requests for this hub.` | hub peers |

#### 3.6.3 `issue_self_ticket`

1. `gate()`; not enabled → `P2P_DISABLED` (currently disabled). Emergency stop → `P2P_DISABLED` (currently
   unavailable).
2. `find_hunt(session)`; none → `P2P_SESSION_NOT_FOUND` (Hunt).
3. **Hub** (`snapshot.is_hub()`): `now = clock()`; `link = store.peer_link(account)`;
   `peer = hunts.check_hub_entry(hub, account, link, now)` (its error propagates); mint with the hub's bindings for
   `peer`, lifetime 180; `hunts.mark_hub_ticketed(hub, account, now, expires_at_ms)`; `false` →
   `P2P_NOT_ROSTERED` (no open entry); record the ticket; answer with the hub as `hunt`. No state or expiry check.
4. **Hunt**: `now = clock()`; state not ticketable (`HostReady`, `Admitting`, `Running`) or
   `snapshot.expires_at <= now` → `P2P_SESSION_NOT_READY` (not accepting).
5. The account's roster entry; none → `P2P_NOT_ROSTERED` (frozen roster).
6. `store.peer_link(account)` must exist with status `Verified` and peer equal to the roster entry's peer, else
   `P2P_STEAM_LINK_REQUIRED`.
7. `reserve_participant(hunt, account, now)`; `None` → `P2P_SESSION_NOT_READY` (rejoin window).
8. The account's peer in the **reserved** hunt's roster (none → `P2P_NOT_ROSTERED`); mint with the reserved hunt's
   bindings, lifetime 180; record the ticket; answer `IssuedTicket { ticket, hunt: reserved }`.

`IssuedTicket::to_json()` (field order is the contract):
`{"ticket":<text>,"expiresAt":<ISO>,"rendezvous":{"sessionId":<wire_session>,"generation":<n>,"lobbyId":<lobby>,
"hostSteamId":"<decimal>","virtualPort":<n>,"bridgeProtocol":<n>,"build":<n>}}`, where `expiresAt` is
`iso_time(expires_at seconds × 1000)` (§4.2), `lobbyId` is omitted when `None`, and `hostSteamId` is the host peer id
as a decimal **string**.

#### 3.6.4 `create_join_bootstrap`

1. `gate()` not enabled → `P2P_DISABLED` (`Player-hosted hunts are disabled.`).
2. Run §3.6.3 (its own checks again, its refusals as there).
3. `find_hunt(session)` again; none → `P2P_SESSION_NOT_READY` (host lobby); `lobby_id` none → same.
4. A fresh bridge instance id: 16 OS-random bytes as 32 lower-case hex characters (random failure →
   `AdmissionError::Ticket(TicketError::Random)`).
5. `join_bootstrap_common(hunt, tickets.key_id(), instance)`; `None` → `P2P_SESSION_NOT_READY` (host lobby).
6. `JoinBootstrap { common, ticket: the issued ticket text, lobby_id }`.

`JoinBootstrap::to_json()`: take `common`'s text, trimmed; remove one trailing `}` if present; if what is left, with
trailing whitespace ignored, ends in `{`, add nothing, else add `,`; then append
`"ticket":<json string>,"lobbyId":<json string>}`. So `{"a":1}` becomes `{"a":1,"ticket":"...","lobbyId":"..."}`
and `{}` becomes `{"ticket":"...","lobbyId":"..."}`.

#### 3.6.5 `consume_ticket` (in this order)

1. `gate()` not enabled → `P2P_DISABLED` (currently disabled).
2. `now = clock()`; `tickets.verify(ticket, now)`. Failure → `P2P_TICKET_EXPIRED` if it is
   `AdmissionError::Ticket(e)` with `e.is_expired()`, else `P2P_TICKET_INVALID`; message for both: `The admission
   ticket is invalid or expired.`
3. The ticket's session (lower-case UUID text) ≠ `capability.wire_session`, or the joiner peer id as decimal text ≠
   `request.authenticated_peer` → `P2P_TICKET_INVALID` (authenticated Steam connection).
4. Capability generation ≠ ticket generation, capability host peer ≠ ticket host peer, or capability bridge instance ≠
   request bridge instance → `P2P_GENERATION_FENCED` (active host generation).
5. Emergency stop → `P2P_DISABLED` (currently unavailable).
6. `find_hunt(capability.session)`; none → `P2P_SESSION_NOT_FOUND`.
7. Hunt generation ≠ ticket generation, hunt host peer ≠ ticket host peer, hunt `bridge_instance_id` ≠
   `Some(request bridge instance)`, or state not ticketable → `P2P_GENERATION_FENCED` (no longer accepts).
8. Actor `HostRuntime { session: capability.session, host: capability.host_account }`. `admission_id` = the jti
   digest. `store.consume_ticket` with digest, `capability.wire_session`, ticket generation, account, joiner, host,
   request bridge instance, request id, `now`. Outcomes: `Consumed` → not idempotent; `Idempotent` → idempotent;
   `Expired` → `P2P_TICKET_EXPIRED` (expired); `Replay` → `P2P_TICKET_REPLAYED`; `Invalid` → `P2P_TICKET_INVALID`
   (not valid for this hunt).
9. Not idempotent: active key `<wire_session>:<generation>:<joiner peer decimal>`. For a hub only,
   `store.supersede_active(actor, key, now)` first. Then `insert_admission` with that active key, the wire session,
   generation, account, joiner peer, bridge instance, connection handle, `admitted_at = now`, no disconnect or
   supersede. `false` → `AdmissionError::Conflict`. Idempotent: skip this step entirely.
10. Hub: `admit_hub_peer(hunt, account, admission_id, now)`. Hunt: `mark_participant_admitted(hunt, account, now)`;
    `false` → `P2P_GENERATION_FENCED` (raced). Step 10 runs on idempotent retries too.
11. Answer `ConsumedTicket { idempotent, admission_id, account, peer: joiner, wire_session: capability.wire_session,
    generation: ticket generation }`.

The idempotent answer is rebuilt from the verified ticket and the capability, so it is byte-identical to the first
answer except `idempotent: true`; nothing about the answer is stored beyond the ticket's consumer fields.

`ConsumedTicket::to_json()`: `{"admitted":true,"idempotent":<b>,"admissionId":<id>,"accountId":<account>,
"steamId":"<joiner decimal>","sessionId":<wire>,"generation":<n>}`.

`ConsumeTicket::from_body(body: &serde_json::Value, request_id: &str) -> Result<Self, AdmissionError>`: the body's
`ticket`, `authenticatedSteamId`, `bridgeInstanceId`, `connectionHandle` must all be JSON strings; the bridge instance
16-96 and the handle 1-32 long, counted in UTF-16 units; otherwise `P2P_TICKET_INVALID` (`The admission request is
malformed.`). The ticket becomes a `Secret`.

#### 3.6.6 `record_peer_disconnected`

1. `admission_id` must be exactly 64 characters of `0-9a-f` (lower case only), else `P2P_TICKET_INVALID` (Invalid
   peer admission identifier).
2. `at = disconnected_at` or `clock()`.
3. Emergency stop → `P2P_DISABLED` (currently unavailable). The `enabled` flag is **not** checked here.
4. `find_admission`; none → `Ok(false)`. Its session ≠ `capability.wire_session` or generation ≠
   `capability.generation` → `Ok(false)`.
5. `store.disconnect` with the id, the capability's wire session, generation and bridge instance, and `at` (actor
   `HostRuntime`). When it returns `true`, call `hunts.peer_disconnected(<the record as found in step 4>, at)`.
   Answer the store's boolean.

#### 3.6.7 `list_hub_peers`

1. `gate()`: not enabled → `P2P_DISABLED` (disabled); emergency stop → `P2P_EMERGENCY_STOP`.
2. `generation` none → `P2P_TICKET_INVALID` (hub generation missing).
3. `transport_key_secret` none → `P2P_ICE_UNAVAILABLE`.
4. `find_hunt(session)` kept only if a hub; else `P2P_SESSION_NOT_FOUND` (Hub).
5. Caller ≠ `snapshot.host_account` → `P2P_NOT_ROSTERED` (only the host).
6. `snapshot.generation` (as i64) ≠ `generation` → `P2P_GENERATION_FENCED` (stale hub).
7. State not ICE-active (§4.5.1) → `P2P_SESSION_NOT_READY` (hub not running).
8. Hourly counter (§4.4.7) key `<wire_session>:<generation>`, limit 3600 → `P2P_SIGNAL_LIMIT`.
9. `hub_presence(hub, 65)`; drop the host's rows and rows not live (`PENDING`, `TICKETED`, `ADMITTED`); keep the first
   64; for each: `pair_key = derive_pair_key(secret, wire_session, generation, host_account, row.account)` (§4.5.3),
   `state` = the presence state's text.

`HubPeers::to_json()`: `{"generation":<n>,"peers":[{"accountId":..,"peerId":"<decimal>","transportKey":..,
"state":..},...]}`.

### 3.7 Routes

`ROUTES` (this order; the `handler` strings are also the `Unimplemented` operation names of `stub_routes()` and must
be kept exactly):

| Method | Path | Mount | `handler` |
| --- | --- | --- | --- |
| POST | `/mystic/v1/hunts/:sessionId/tickets/self` | Early | `p2pAdmission.ts:IssueSelfAdmissionTicket` |
| POST | `/mystic/v1/hunts/:sessionId/tickets/consume` | Early | `p2pAdmission.ts:ConsumeAdmissionTicket` |
| POST | `/mystic/v1/hunts/:sessionId/peers/:admissionId/disconnect` | Early | `p2pAdmission.ts:RecordPeerDisconnected` |
| POST | `/launcher/v1/p2p/hunts/:sessionId/join-bootstrap` | Normal | `p2pLauncher.ts:CreateJoinBootstrap` |
| POST | `/launcher/v1/p2p/hunts/:sessionId/peers` | Normal | `realmHubAdmission.ts:ListHubPeers` |

`stub_routes() -> StubRoutes { component: Admission, routes: ROUTES }`.

`CapabilityResolver` = `Arc<dyn Fn(Option<&str>, &str, CapabilityScope) -> BoxFuture<'static, Result<HostCapability,
AdmissionError>> + Send + Sync>` (header value, raw `sessionId` param, scope).

`AdmissionRoutes` (Clone): `fn new(service: Arc<dyn AdmissionService>, resolver: Option<CapabilityResolver>, node:
NodeId) -> Self`; implements `RouteHandler` with `routes() == ROUTES`.

Dispatch by `call.route.handler`; any other handler → `Unimplemented(Admission, handler)`. Shared rules:

- **Param**: the first param with that name, else empty.
- **Session id from the param**: if the raw text contains `:`, keep only what follows the first `:`. If that parses
  as `<ULID>@<node>` (`SessionId`), use it. Otherwise, if it is a 36-character UUID with `-` at 8, 13, 18, 23 and
  only `0-9a-f` (lower case) elsewhere, use the 128-bit value as the ULID with the route's `node`; otherwise ULID 0 with
  the route's `node`.
- **Launcher caller**: `Caller::Launcher(account)`, else `AUTH_UNAUTHORIZED` (`Sign in required.`).
- **Host capability**: `Caller::HostRuntime(r)` with `HostCapability::from_runtime(r)` `Some` → that. Else, no
  resolver → `P2P_CAPABILITY_REQUIRED` (`Missing host capability header.`). Else call the resolver with the
  `x-mystic-host-capability` header value, the raw `sessionId` param and `CapabilityScope::PeerAdmitted`, and use or
  propagate its result.
- **Object body**: empty body → `{}`; otherwise it must parse as a JSON object, else `P2P_TICKET_INVALID` (`The
  request body must be a JSON object.`).
- **Generation from a body**: the `generation` field when it is a JSON integer fitting i64 (any i64), or a finite
  float with no fraction and magnitude ≤ 2^53 - 1; else `None`.
- **Error mapping**: `AdmissionError::Unimplemented` → `SeamError::Unimplemented`; `Refused` → `SeamError::Refused`;
  every other error → `SeamError::Refused(Refusal { code: "INTERNAL", message: <the error's Display> })`.

Per route (all success replies are status 200 with the body shown):

| Handler | Steps |
| --- | --- |
| IssueSelfAdmissionTicket | launcher caller; session from param; `issue_self_ticket`; `IssuedTicket::to_json()` |
| ConsumeAdmissionTicket | host capability **first**; then object body; request id = `call.request.request_id`, or `missing` when empty; `ConsumeTicket::from_body`; `consume_ticket`; `ConsumedTicket::to_json()` |
| RecordPeerDisconnected | host capability; `admissionId` param; `record_peer_disconnected(cap, id, None)`; body `{"disconnected":true}` or `{"disconnected":false}`; the request body is not read |
| CreateJoinBootstrap | launcher caller; session; `create_join_bootstrap`; `JoinBootstrap::to_json()` |
| ListHubPeers | launcher caller; session; object body; generation from body; `list_hub_peers`; `HubPeers::to_json()` |

isles turns a `SeamError::Refused` into status `status_for(code)` with
`{"error":{"code":..,"message":..,"requestId":..}}` (§5).

### 3.8 Examples (CLI behaviour)

Both find a flag by its first occurrence among the arguments and take the next argument as its value.

**`generate-admission-key`** (`--output <dir> --key-id <id>`):

- Either missing → stderr `Usage: generate-admission-key --output <new-directory> --key-id <safe-id>`, exit 2.
- `write_key_set(output, KeyId(id), current process id, current Unix millis)`.
- Success → stdout, three lines: `Generated a new P-256 admission key set in: <output as given>`;
  `Keep the private PEM and metagame-admission.env only on the server: the signing key goes to secrets.env.`;
  `Publish p2p-admission-public.xy with the runtime as an extra file (Guard role ADMISSION_KEY).`; exit 0.
- Failure → the `KeyError` Display on stderr, exit 1. No key is ever printed.

**`mint-ticket`** (`--env <file> --out <file>` plus optional bindings):

- `--env` or `--out` missing → stderr `Usage: mint-ticket --env <metagame-admission.env> --out <ticket file>
  [bindings]`, exit 1.
- Env file: each line containing `=` splits at the first `=`; key and value trimmed (no comment or quote handling).
  Read error → `<path>: <io error>`. `Es256Tickets::from_settings` errors → their Display.
- Options and defaults: `--account` `account-1`; `--joiner` `76561198000000001`; `--host` `76561198000000002`;
  `--session` `01234567-89ab-4def-8123-456789abcdef`; `--generation` 7; `--party` `party_abc==`; `--party-revision`
  12; `--hunt` `hunt/island-1`; `--build` 392819; `--bridge-protocol` 1; `--virtual-port` 1; `--now` the system clock
  in Unix seconds; `--lifetime` (u32) none, so the setting's default. An unparsable number →
  `--<name> is not a number`; a bad session → its `TicketError` Display.
- Mint with the inherent `mint`; write the ticket text to `--out` (create or truncate; mode 0600 on Unix).
  stdout `jti digest <digest> expires at <expires_at seconds>`, exit 0. Every error goes to stderr, exit 1.

---

## 4. `rifts-relay`

The ICE servers and TURN credentials a session's transports use, the pair keys that bind a joiner to its host, the
ICE signal relay, and the admin `coop-transport` switch. It listens on nothing; Cloudflare is reached through
`ProviderHttp`, which isles implements.

Dependencies: `base64`, `getrandom`, `hmac`, `rifts-seam`, `serde_json`, `sha1`, `sha2`, `thiserror`.

### 4.1 Public API (complete)

Public modules `rules` and `wire` (imported by path by `rifts-admission`, `rifts-realms` and the fuzz target);
`config`, `credentials`, `routes`, `service` are private and re-exported at the root.

Used outside (grep): `isles-1-14-7` (`Ice`, `IceHunts`, `IceService`, `OperationsPolicy`, `ProviderAnswer`,
`ProviderHttp`, `ProviderRequest`, `ProviderUnreachable`, `RelayConfig` with `from_settings` and the
`transport_key_secret` field, `RelayGrants`, `RelayRoutes`, `RelayUsageCache`, `stub_routes`); `rifts-realms` (those
plus `ControlMessage`, `CoopTransportOutcome`, `P2PGate`, `RelayError`, `TurnProvider`, `TurnRest`, and
`rules::{CoopTransport, HuntSnapshot, HuntState, PresenceRow, PresenceState, RosterEntry, SessionKind}`);
`rifts-admission` (`HourlyCounter`, `P2PGate`, `rules::{HuntSnapshot, PresenceRow, derive_pair_key}`,
`wire::{JsonObject, hex, iso_time, json_array, json_string, js_number, format_uuid}`); fuzz
(`rules::{MAX_SIGNAL_BYTES, SignalKind, filter_relay_only, parse_ice_signal}`, `wire::js_trim`).

#### 4.1.1 Root items

| Item | Shape | Notes |
| --- | --- | --- |
| `RelayError` | `#[non_exhaustive]` enum `Unimplemented(Unimplemented)`, `Refused(Refusal)`, `Provider(ProviderFailure)`, `Store(StoreError)` (all `From`, transparent), `Random` | Debug, Clone, PartialEq, Eq, Error; `Random` Display `the system random source failed` |
| `RelayCredential` | pub `username: String`, `credential: Secret`, `urls: Vec<String>`, `expires_at: UnixMillis` | Debug, Clone, PartialEq, Eq |
| `CredentialIssuer` | trait `Send + Sync`: `fn provider(&self) -> TurnProvider`; `fn issue(&self, account: AccountId, ttl_seconds: u32, now: UnixMillis) -> BoxFuture<'_, Result<IssuedCredential, RelayError>>` | — |
| `PairKeys` | trait `Send + Sync`: `fn pair_key(&self, wire_session: &str, generation: u64, host: &AccountId, joiner: &AccountId) -> Result<Secret, RelayError>` | — |
| `TransportKeys` | tuple `(pub Option<Secret>)`; Debug, Clone | `PairKeys`: `None` → `P2P_ICE_UNAVAILABLE` (`The co-op transport is not configured on this server.`), else `derive_pair_key` |
| `IceService` | trait `Send + Sync`: `transport_config(account: AccountId, session: SessionId, generation: Option<i64>) -> BoxFuture<Result<Json, RelayError>>`; `submit_signal(account: AccountId, session: SessionId, body: serde_json::Value) -> BoxFuture<Result<Json, RelayError>>`; `set_coop_transport(admin_key_id: String, body: serde_json::Value) -> BoxFuture<Result<Reply, RelayError>>` | — |
| `Stub` | unit; Debug, Clone, Copy, Default | `CredentialIssuer` (provider `None`, issue → `Unimplemented(Relay, "issue")`), `PairKeys` (`pair_key`), `IceService` (`transport_config`, `submit_signal`, `set_coop_transport`) |
| From `config` | `CloudflareAnalytics`, `CloudflareTurn`, `RelayConfig`, `SECRET_SETTINGS`, `TurnProvider`, `TurnRelayPorts`, `TurnRest`, `is_stun_url`, `is_turn_url` | §4.3 |
| From `credentials` | `CLOUDFLARE_GRAPHQL_API`, `CLOUDFLARE_RTC_API`, `CloudflareIssuer`, `HourlyCounter`, `IceServer`, `IssuedCredential`, `PROVIDER_TIMEOUT_MS`, `ProviderAnswer`, `ProviderFailure`, `ProviderHttp`, `ProviderIceServers`, `ProviderRequest`, `ProviderUnreachable`, `RelayDenied`, `RelayGrant`, `RelayGrants`, `RelayStatus`, `RelayUsageCache`, `TurnRestIssuer`, `mint_cloudflare`, `mint_turn_rest`, `month_window`, `normalize_provider_ice_servers`, `read_monthly_relay_egress_bytes`, `relay_account_tag`, `relay_ttl_seconds` | §4.4 |
| From `routes` | `ROUTES`, `RelayRoutes`, `stub_routes` | §4.7 |
| From `rules` (also at root) | `CoopTransport`, `HuntSnapshot`, `derive_pair_key` | §4.5 |
| From `service` | `ControlMessage`, `CoopTransportOutcome`, `Ice`, `IceHunts`, `OperationsPolicy`, `P2PGate`, `refusal` | §4.6 |

### 4.2 `wire`: JSON and JavaScript rules

| Item | Behaviour |
| --- | --- |
| `json_string(text: &str) -> String` | A JSON string as `JSON.stringify` writes it: `"`→`\"`, `\`→`\\`, U+0008→`\b`, U+000C→`\f`, LF→`\n`, CR→`\r`, TAB→`\t`, other < U+0020 → `\u00xx` (lower-case hex); everything else (`/`, non-ASCII) as is |
| `JsonObject` | tuple struct over the text being built; Debug, Clone, Default. `new()`, then chained `string(key, &str)`, `optional_string(key, Option<&str>)` (omitted when `None`), `number(key, impl Display)` (written with `Display`), `boolean(key, bool)`, `raw(key, json_text)` (spliced verbatim), `strings(key, &[String])` (array of JSON strings), and `finish() -> String`. Fields appear in call order, comma-separated, no spaces; `new().finish()` is `{}`. (`JsonObject::default()` starts from an empty text, not `{`; nothing uses it.) |
| `json_array(items: impl IntoIterator<Item = String>) -> String` | `[` + items joined by `,` + `]` (items are already JSON) |
| `iso_time(at: UnixMillis) -> String` | `Date.toISOString`: `YYYY-MM-DDTHH:MM:SS.mmmZ`, proleptic Gregorian UTC, Euclidean division so negatives work (`-1` → `1969-12-31T23:59:59.999Z`); years 0-9999 |
| `js_trim(text: &str) -> &str` | Trims JavaScript white space at both ends: TAB, LF, VT, FF, CR, space, U+00A0, U+1680, U+2000-U+200A, U+2028, U+2029, U+202F, U+205F, U+3000, U+FEFF. Not U+0085 |
| `MAX_SAFE_INTEGER: f64` | 9007199254740991.0 |
| `is_safe_integer(value: f64) -> bool` | finite, no fraction, `|value| <= 2^53 - 1` |
| `js_number(text: &str) -> f64` | `Number(string)`: trim with `js_trim`; empty → 0; `0x`/`0o`/`0b` prefixes (prefix any case, at least one digit, no sign) → that integer; `Infinity`, `+Infinity`, `-Infinity`; otherwise an optional sign and a decimal literal (digits, optional fraction, or a bare fraction like `.5`, optional `e`/`E` exponent with optional sign and at least one digit) parsed as f64; anything else NaN. `-0x10`, `.`, `inf`, `12px` are NaN; `1e2` is 100; `0x12c` is 300 |
| `js_number_value(value: &serde_json::Value) -> f64` | `Number(x)` for a JSON value: null → 0; bool → 0/1; number → its f64; string → `js_number`; array: `[]` and `[null]` → 0, `[number]` → it, `[string]` → `js_number`, `[array]` → recurse, else NaN; object → NaN |
| `safe_integer(value: &serde_json::Value) -> Option<i64>` | JSON numbers only (never strings): an integer within ±(2^53 - 1), or a float that `is_safe_integer`. `3`, `3.0`, `-4` → yes; `3.5`, `"3"`, `9007199254740992` → no |
| `random_uuid() -> Result<String, getrandom::Error>` | 16 OS-random bytes, version nibble 4, variant `10xx`, lower-case dashed text |
| `format_uuid(bytes: &[u8; 16]) -> String` | lower-case `8-4-4-4-12` |
| `hex(bytes: &[u8]) -> String` | lower-case hex |

All JSON this crate writes is built with these, so field order and escaping are Node's.

### 4.3 Configuration (`RelayConfig`)

Types (pub fields; Debug, Clone, PartialEq, Eq):

- `TurnProvider`: enum `Cloudflare` (Default), `TurnRest`, `None`; also Copy; `const fn as_str` → `cloudflare`,
  `turn-rest`, `none`.
- `CloudflareTurn { key_id: String, api_token: Secret }`; `TurnRest { urls: Vec<String>, secret: Secret }`;
  `CloudflareAnalytics { account_id: String, api_token: Secret }`.
- `RelayConfig { stun_urls: Vec<String>, provider: TurnProvider, cloudflare: Option<CloudflareTurn>, turn_rest:
  Option<TurnRest>, credential_ttl_seconds: u32, hub_credential_ttl_seconds: u32, monthly_quota_bytes: u64,
  configs_per_hour: u32, analytics: Option<CloudflareAnalytics>, transport_key_secret: Option<Secret>, force_relay:
  bool }`.
- `TurnRelayPorts { listen: u16, tls: Option<u16>, relay: RangeInclusive<u16> }`; Default: 3478, `None`,
  `49152..=65535`.
- `SECRET_SETTINGS = ["P2P_CLOUDFLARE_TURN_API_TOKEN", "P2P_TURN_REST_SECRET", "P2P_CLOUDFLARE_ANALYTICS_API_TOKEN",
  "P2P_TRANSPORT_KEY_SECRET"]` (this order).

`RelayConfig::default()`: STUN `["stun:stun.cloudflare.com:3478"]`, provider Cloudflare, no Cloudflare key, no
turn-rest, TTL 10800, hub TTL 86400, quota 10 000 000 000 bytes, 30 configs per hour, no analytics, no transport key,
`force_relay` false. `from_settings` on empty settings equals the default.

URL patterns:

- `is_stun_url`: `stun:` or `stuns:`, then a host of one or more `[A-Za-z0-9.-]`, optionally `:` and 1-5 digits;
  nothing else.
- `is_turn_url`: `turn:` or `turns:`, host as above, optional `:port` (1-5 digits), optionally followed by exactly
  `?transport=udp` or `?transport=tcp`.

`RelayConfig::from_settings(settings: &dyn Settings) -> Self`, key by key (all values pass through `js_trim` where it
says "trimmed"; a bad value falls back, it never fails):

| Key | Rule |
| --- | --- |
| `P2P_ICE_STUN_URLS` | split on `,`, each trimmed, keep those `is_stun_url`; none left → the default list |
| `P2P_TURN_PROVIDER` | trimmed, lower-cased: `turn-rest` → TurnRest, `none` → None, anything else or missing → Cloudflare |
| `P2P_CLOUDFLARE_TURN_KEY_ID` + `P2P_CLOUDFLARE_TURN_API_TOKEN` | `cloudflare` is set only when the provider is Cloudflare, the trimmed key id is 8-64 `[A-Za-z0-9]`, and the trimmed token is 8-512 of `[A-Za-z0-9._~+/=-]` |
| `P2P_TURN_REST_URLS` + `P2P_TURN_REST_SECRET` | `turn_rest` is set only when the provider is TurnRest, at least one comma-separated trimmed URL passes `is_turn_url` (the kept list), and the trimmed secret is ≥ 16 UTF-16 units |
| `P2P_CLOUDFLARE_ANALYTICS_ACCOUNT_ID` + `P2P_CLOUDFLARE_ANALYTICS_API_TOKEN` | `analytics` set when the trimmed account is 8-64 `[A-Za-z0-9]` and the token passes the token rule (any provider) |
| `P2P_TURN_MAX_TTL_SECONDS` | bounded number (below), default 10800, 300..=172800 |
| `P2P_REALM_HUB_TURN_TTL_SECONDS` | default 86400, 300..=172800 |
| `P2P_RELAY_MONTHLY_QUOTA_GB` | default 10, 0..=100000; stored × 1 000 000 000 |
| `P2P_ICE_CONFIGS_PER_HOUR` | default 30, 1..=1000 |
| `P2P_TRANSPORT_KEY_SECRET` | trimmed; kept only when ≥ 32 UTF-16 units |
| `P2P_FORCE_SDR` | `true` when the trimmed value equals `true` in any ASCII case (`1` is false) |

Bounded number: missing, or empty after trimming → default; else `js_number`; a safe integer within the range is
taken, anything else (NaN, fraction, out of range) → default. `-1` → default; `999999` → default; `1e2` → 100.

`relay_configured(&self) -> bool`: Cloudflare → `cloudflare.is_some()`; TurnRest → `turn_rest.is_some()`; None →
false.

`summary_json(&self) -> String` (no secret, this field order):
`{"iceStunServers":<count>,"iceTurnProvider":<as_str>,"iceRelayConfigured":<bool>,"iceRelayQuotaGb":<quota/1e9>,
"iceRelayQuotaMeasured":<quota > 0 && provider == Cloudflare && analytics set>,"iceTransportKeyConfigured":<bool>}`.

### 4.4 Credentials

#### 4.4.1 Relay tag

`relay_account_tag(account: &AccountId) -> String`: the first 32 characters of the lower-case hex SHA-256 of the
UTF-8 bytes of `mysticparadox-relay-tag-v1` + LF + the account id. The provider sees this, never the account id.

#### 4.4.2 `turn-rest` (`mint_turn_rest`)

`mint_turn_rest(turn_rest: &TurnRest, account: &AccountId, ttl_seconds: u32, now: UnixMillis) -> RelayCredential`:

- `expiry = floor(now.0 / 1000) + ttl_seconds` (Euclidean floor);
- `username = "<expiry>:<relay tag>"` (decimal expiry);
- `credential` = standard base64 **with padding** of HMAC-SHA1(key = the secret's UTF-8 bytes, message = the
  username's UTF-8 bytes) (28 characters);
- `urls` = the configured URLs; `expires_at = now + ttl_seconds × 1000`.

This is the TURN REST scheme coturn (`use-auth-secret`) and turn-rs (`static-auth-secret`) check. RFC 2202 case 2
(key `Jefe`, message `what do ya want for nothing?`) gives HMAC
`effcdf6ae5eb2fa2d27416d5f184df9c259a7c79`.

`TurnRestIssuer(pub TurnRest)` (Debug, Clone): `provider()` TurnRest; `issue` → `IssuedCredential { stun_urls: [],
credential: mint_turn_rest(...) }`, never fails.

#### 4.4.3 Provider HTTP

- `ProviderRequest { url: String, bearer: Secret, body: String (JSON text), timeout_ms: u64 }`; Debug, Clone,
  PartialEq, Eq.
- `ProviderAnswer { status: u16, json: Option<serde_json::Value> }` (`None` when the body did not parse); Debug,
  Clone, PartialEq.
- `ProviderUnreachable`: unit struct; Debug, Clone, PartialEq, Eq, Error; Display `relay provider request failed`.
- `ProviderHttp` (trait `Send + Sync`): `fn post(&self, request: ProviderRequest) -> BoxFuture<'_,
  Result<ProviderAnswer, ProviderUnreachable>>`. The implementation sends a POST with `Authorization: Bearer <token>`
  and the JSON body, applies the timeout, and logs neither the token nor the body.
- `PROVIDER_TIMEOUT_MS = 5000`; `CLOUDFLARE_RTC_API = "https://rtc.live.cloudflare.com/v1"`;
  `CLOUDFLARE_GRAPHQL_API = "https://api.cloudflare.com/client/v4/graphql"`.

#### 4.4.4 Cloudflare credential (`mint_cloudflare`)

`async fn mint_cloudflare(cloudflare: &CloudflareTurn, account: &AccountId, ttl_seconds: u32, now: UnixMillis, http:
&dyn ProviderHttp) -> Result<IssuedCredential, ProviderFailure>`:

1. URL `https://rtc.live.cloudflare.com/v1/turn/keys/<key_id>/credentials/generate-ice-servers` (the key id is
   alphanumeric, inserted as is), bearer the API token, timeout 5000.
2. POST body `{"ttl":<ttl>,"customIdentifier":"<relay tag>"}`.
3. If the status is 400, POST again with `{"ttl":<ttl>}` and use that answer.
4. Unreachable (either call) → `ProviderFailure::Unreachable`. Status outside 200..=299 → `Status(status)`.
5. `normalize_provider_ice_servers(answer.json)`; no TURN entry → `NoRelayEntry`.
6. `IssuedCredential { stun_urls: <normalized STUN URLs>, credential: RelayCredential { username, credential, urls:
   <TURN URLs>, expires_at: now + ttl × 1000 } }`.

`ProviderFailure` (Debug, Clone, PartialEq, Eq, Error): `Status(u16)` → `Cloudflare TURN credential request answered
{0}`; `NoRelayEntry` → `Cloudflare TURN response carried no usable relay entry`; `Unreachable(ProviderUnreachable)`
(transparent, `From`).

`IssuedCredential { stun_urls: Vec<String>, credential: RelayCredential }`; Debug, Clone, PartialEq, Eq.

`CloudflareIssuer { pub key: CloudflareTurn, pub http: Arc<dyn ProviderHttp> }` (Clone): `provider()` Cloudflare;
`issue` → `mint_cloudflare`, errors as `RelayError::Provider`.

#### 4.4.5 Normalising a provider answer

`normalize_provider_ice_servers(body: Option<&Value>) -> ProviderIceServers` where `ProviderIceServers { stun_urls:
Vec<String>, turn: Option<(Vec<String>, String, Secret)> }` (Debug, Clone, PartialEq, Eq, Default):

1. Entries: `body.iceServers` when an array; missing or `null` → none; any other value → that one value as the only
   entry.
2. For each entry in order: URLs = `urls` when a string (one URL) or an array (its string elements; others
   skipped); otherwise none. Drop every URL that ends with `:53` or contains `:53?`.
3. Append each URL passing `is_stun_url` to `stun_urls` unless already present.
4. If no TURN entry has been taken yet and the entry has URLs passing `is_turn_url`: take it when both `username`
   and `credential` are strings of 1-512 bytes, each byte 0x21-0x7E; the TURN entry is (those TURN URLs, username,
   credential). An entry with bad credentials is skipped and a later entry may still supply the TURN entry.

#### 4.4.6 Monthly usage (`month_window`, `read_monthly_relay_egress_bytes`, `RelayUsageCache`)

`month_window(now) -> (String, String, String)`: from `iso_time(now)`: (`YYYY-MM-01`, `YYYY-MM-DD`, `YYYY-MM`).

`async fn read_monthly_relay_egress_bytes(analytics: &CloudflareAnalytics, account: &AccountId, now: UnixMillis, http:
&dyn ProviderHttp) -> Option<f64>`:

1. The GraphQL query text, with `<A>`, `<FROM>`, `<TO>`, `<TAG>` each written as `json_string` of the account id, the
   month's first day, today and the relay tag (so they appear in double quotes), and two line breaks with their
   indentation exactly as shown (8 spaces before `callsTurnUsageAdaptiveGroups`, 12 before `customIdentifier`):

   ```text
   query { viewer { usage: accounts(filter: { accountTag: <A> }) {
           callsTurnUsageAdaptiveGroups(filter: { date_geq: <FROM>, date_leq: <TO>,
               customIdentifier: <TAG> }, limit: 1, orderBy: [customIdentifier_ASC]) { sum { egressBytes } } } } }
   ```

2. POST to `CLOUDFLARE_GRAPHQL_API`, bearer the analytics token, timeout 5000, body `{"query":<json string of the
   query>}` (`json_string`, so the line breaks become `\n`).
3. Unreachable → `None`. Status ≠ 200, or `errors` is a non-empty array → `None`.
4. `data.viewer.usage`; when that is missing or `null`, `data.viewer.accounts`. Its first element's
   `callsTurnUsageAdaptiveGroups` must be an array, else `None`.
5. Sum over the groups of `js_number_value(group.sum.egressBytes)`, with NaN (and a missing value) counted as 0.
   Result `Some(sum)` only when finite and ≥ 0, else `None`. (`"5"` and `7` sum to 12; `"x"` gives 0; `-3` gives
   `None`.)

`RelayUsageCache` (Debug, Default): per account, (month, bytes, fetched at). `get(&self, account, month: &str, now)
-> Option<f64>`: the bytes when the month matches and `now - fetched_at < 600 000` ms. `set(&self, account, month,
bytes, now)`: if more than 10 000 entries are held, clear them all first; then store.

#### 4.4.7 `HourlyCounter`

Debug, Default; per key, the timestamps of allowed hits. `allow(&self, key: &str, limit: u32, now: UnixMillis) ->
bool`: keep only the key's hits with `now - at < 3 600 000`; if as many as `limit` remain, store the pruned list and
answer `false`; otherwise record `now`, store, and, when more than 10 000 keys are held, drop every key with no hit in
the last hour; answer `true`. In process only.

#### 4.4.8 The grant (`RelayGrants`)

Types: `RelayDenied` enum `NotConfigured`, `QuotaExceeded`, `ProviderError` (Copy; `as_str` →
`RELAY_NOT_CONFIGURED`, `RELAY_QUOTA_EXCEEDED`, `RELAY_PROVIDER_ERROR`); `RelayStatus` enum `Allowed { ttl_seconds:
u32, expires_at: UnixMillis }`, `Denied(RelayDenied)` (Copy); `IceServer { urls: Vec<String>, username:
Option<String>, credential: Option<Secret> }` with `stun(urls)`, `turn(&RelayCredential)`, `to_json()`;
`RelayGrant { ice_servers: Vec<IceServer>, relay: RelayStatus }` with `ice_servers_json()`, `relay_json()`,
`result_label()`; `RelayGrants { config: Arc<RelayConfig>, http: Arc<dyn ProviderHttp>, usage: Arc<RelayUsageCache> }`
(Clone) with `issuer()` and `grant()`. All pub fields.

- `RelayGrants::issuer(&self) -> Option<Box<dyn CredentialIssuer>>`: Cloudflare with its key →
  `CloudflareIssuer`; TurnRest with its config → `TurnRestIssuer`; otherwise `None`.
- `async fn grant(&self, account: &AccountId, ttl_seconds: u32, now: UnixMillis) -> RelayGrant` (never fails):
  1. No issuer → STUN only, `Denied(NotConfigured)`.
  2. When analytics are set, the provider is Cloudflare and the quota is > 0: the month from `month_window`; cached
     usage, else read it (and cache it when read); usage ≥ quota → STUN only, `Denied(QuotaExceeded)`. An unreadable
     figure does not block.
  3. Issue. Success → ice servers `[STUN entry, TURN entry]` where the STUN entry's URLs are the configured STUN URLs
     followed by the provider's extra STUN URLs not already listed, and the TURN entry is the credential; relay
     `Allowed { ttl_seconds, expires_at: now + ttl × 1000 }`. Failure → STUN only, `Denied(ProviderError)`.

  "STUN only" is one entry `IceServer::stun(config.stun_urls)`.

- `IceServer::to_json()`: `{"urls":[...],"username":..,"credential":..}`, the last two omitted when `None`. This is
  the one place a credential is written out.
- `ice_servers_json()`: the JSON array of the entries.
- `relay_json()`: allowed → `{"allowed":true,"ttlSeconds":<n>,"expiresAt":<iso_time>}`; denied →
  `{"allowed":false,"reason":<RELAY_...>}`.
- `result_label()`: `ISSUED` or the denial's `as_str`.

`relay_ttl_seconds(hunt_expires_at: UnixMillis, now: UnixMillis, max_ttl_seconds: u32) -> u32`: remaining =
`ceil((hunt_expires_at - now) / 1000)` (may be negative); wanted = remaining + 600; result = `min(max(wanted, 300),
max_ttl_seconds)`. Examples with cap 10800: one hour left → 4200; an hour past → 300; 1.5 s left → 602.

### 4.5 ICE session rules (`rules`)

#### 4.5.1 Types (pub; Debug, Clone, Copy, PartialEq, Eq, Hash for the enums)

- `HuntState`: `Requested`, `HostSelecting`, `HostOffered`, `Starting`, `HostReady`, `Admitting`, `Running`,
  `Draining`, `Suspect`, `Fenced`, `Aborting`, `Recovering`, `Completed`, `Aborted`.
  `const fn is_ice_active(self)`: `Starting`, `HostReady`, `Admitting`, `Running`, `Draining`, `Suspect`.
  `const fn is_ticketable(self)`: `HostReady`, `Admitting`, `Running`.
- `SessionKind`: `Hunt`, `RealmHub`.
- `CoopTransport`: `Ice`, `SteamExperimental`; `as_str` → `ICE`, `STEAM_EXPERIMENTAL`; `fn parse(&str) ->
  Option<Self>` (exact text only).
- `PresenceState`: `Pending`, `Ticketed`, `Admitted`, `Lost`, `Left`, `Removed`, `Expired`; `const fn is_live` for
  the first three; `as_str` → upper-case names (`PENDING` ... `EXPIRED`).
- `IceRole`: `Host`, `Join` (Copy, no Hash); `as_str` → `HOST`, `JOIN`.
- `SignalKind`: `Description`, `Candidate`, `GatheringDone` (Copy, no Hash); `as_str` → `DESCRIPTION`, `CANDIDATE`,
  `GATHERING_DONE`.
- Structs (Debug, Clone, PartialEq, Eq; pub fields): `RosterEntry { account: AccountId, peer: PeerId,
  returned_to_hub: bool }`; `PresenceRow { account: AccountId, peer: PeerId, state: PresenceState }`;
  `HuntSnapshot { wire_session: String, generation: u64, state: HuntState, kind: SessionKind, realm: Option<String>,
  coop_transport: CoopTransport, host_account: AccountId, host_peer: PeerId, expires_at: UnixMillis, roster:
  Vec<RosterEntry> }` with `is_hub()` (kind is `RealmHub`) and `is_realm_session()` (`realm.is_some()`);
  `IcePeer { account: AccountId, role: IceRole, transport_key: Secret }`; `IceSignal { generation: i64, to:
  AccountId, kind: SignalKind, data: String }`.
- Constants `MAXIMUM_HUB_PEERS: usize = 64`, `MAX_SIGNAL_BYTES: usize = 4096`.

#### 4.5.2 Roles and peers

- `resolve_role(hunt, account, presence: Option<&[PresenceRow]>) -> Option<IceRole>`: the host account → `Host`. A
  hub: `Join` when `presence` is given and has a live row for the account, else `None` (a `LOST` member needs a fresh
  entry). A hunt: `Join` when the account has a roster entry not returned to the hub, else `None`.
- `active_joiners(hunt, presence) -> Vec<AccountId>`: a hub: distinct accounts of live rows other than the host, in
  row order, at most 64. A hunt: roster accounts other than the host and not returned to the hub, in roster order.
- `peer_id_of(hunt, account, presence) -> Option<PeerId>`: the host → `host_peer`; a hub → the first presence row
  for the account (any state), `None` without presence; a hunt → its roster entry's peer.
- `ice_peers_for(hunt, account, role, secret: &Secret, presence) -> Vec<IcePeer>`: for `Host`, one `IcePeer` per
  active joiner with role `Join` and the pair key for (host, joiner); for `Join`, one peer: the host account, role
  `Host`, the pair key for (host, this account).

#### 4.5.3 Pair keys (`derive_pair_key`)

`derive_pair_key(secret: &Secret, wire_session: &str, generation: u64, host: &AccountId, joiner: &AccountId) ->
Secret`: HMAC-SHA256 with key = the secret's UTF-8 bytes over the UTF-8 message
`mysticparadox-ice-pair-v1` LF `<wire_session>` LF `<generation decimal>` LF `<host>` LF `<joiner>` (no trailing LF);
the 32-byte MAC as base64url **without padding**: 43 characters. Derived, never stored; the same for host and joiner;
different for every pair and generation.

#### 4.5.4 Signals

`parse_ice_signal(body: &Value) -> Option<IceSignal>`, `None` on any failure:

1. `generation`: `safe_integer` (§4.2) and ≥ 0.
2. `toAccountId`: a string of 8-64 bytes of `[A-Za-z0-9-]`.
3. `kind`: exactly `DESCRIPTION`, `CANDIDATE` or `GATHERING_DONE`.
4. `data`: missing or `null` → empty string; a string → it; anything else → refuse.
5. `data` at most 4096 bytes, every byte 0x20-0x7E, CR or LF.
6. For kinds other than `GATHERING_DONE`, `data` must not be empty after `js_trim`.

`signal_route_allowed(hunt, from, to, presence) -> bool`: `false` when `from == to`; otherwise both must resolve to a
role and exactly one of them must be `Host` (host↔joiner only, never joiner↔joiner).

`filter_relay_only(kind, data) -> Option<String>` (applied only when `force_relay`):

- `GATHERING_DONE`: `Some(data)` unchanged.
- Otherwise the separator is CRLF when `data` contains CRLF, else LF. Split `data` on LF; drop one trailing CR from
  each piece; `js_trim` each; drop empty lines. A *candidate line* starts with `candidate:` after an optional `a=`
  prefix; it is kept only if it contains ` typ relay` followed by a space or the end of the line (`typ relayed` is not
  relay). For `DESCRIPTION`, non-candidate lines are kept only when they start with `a=ice-ufrag:`, `a=ice-pwd:`,
  `a=ice-options:`, or equal `a=end-of-candidates`; for `CANDIDATE`, no non-candidate line is kept.
- `CANDIDATE`: `Some(the line)` when exactly one line is kept, else `None`.
- `DESCRIPTION`: `Some(kept lines joined with the separator)` when some kept line starts with `a=ice-ufrag:`, else
  `None`.

### 4.6 Service operations

Types: `P2PGate { enabled: bool, emergency_stop: bool }` (Debug, Clone, Copy, PartialEq, Eq);
`ControlMessage { account: AccountId, message_type: &'static str, wire_session: String, generation: u64, payload:
Json, created_at: UnixMillis, expires_at: UnixMillis }`; `CoopTransportOutcome` enum `Updated(Json)`,
`Conflict(Json)`; `refusal(code, message) -> RelayError` (a `Refused`).

`IceHunts` (trait, `Send + Sync`, sync): `gate(&self) -> Result<P2PGate, RelayError>`; `find_hunt(&self, session:
&SessionId) -> Result<Option<HuntSnapshot>, RelayError>`; `hub_presence(&self, hunt: &HuntSnapshot, limit: usize) ->
Result<Vec<PresenceRow>, RelayError>`; `enqueue(&self, message: ControlMessage) -> Result<(), RelayError>`.

`OperationsPolicy` (trait, `Send + Sync`): `set_coop_transport(&self, admin_key_id: &str, transport: CoopTransport,
expected_version: u64, reason: &str, request_id: &str) -> Result<CoopTransportOutcome, RelayError>`.

`Ice` (pub fields `hunts: Arc<dyn IceHunts>`, `grants: RelayGrants`, `policy: Arc<dyn OperationsPolicy>`, `clock:
Arc<dyn Fn() -> UnixMillis + Send + Sync>`, plus two private hourly counters); `Ice::new(hunts, grants, policy,
clock)`; implements `IceService`.

Shared steps:

- **Running gate**: `gate()`; not enabled → `P2P_DISABLED` (`Player-hosted hunts are disabled.`); emergency stop →
  `P2P_EMERGENCY_STOP` (`Player-hosted hunts are currently stopped by operations policy.`).
- **Active ICE hunt** (session, generation): generation `None` → `P2P_TICKET_INVALID` (`The hunt generation is
  missing.`); `find_hunt` none → `P2P_SESSION_NOT_FOUND` (`Hunt session not found.`); the hunt's frozen
  `coop_transport` ≠ `Ice` → `P2P_TRANSPORT_MISMATCH` (`This hunt does not use the ICE co-op transport.`); hunt
  generation ≠ requested → `P2P_GENERATION_FENCED` (`This request belongs to a stale hunt generation.`); state not
  ICE-active → `P2P_SESSION_NOT_READY` (`The hunt is not running.`).
- **Presence**: for a hub, `hub_presence(hunt, 65)`; for a hunt, none.

`transport_config(account, session, generation)`:

1. Running gate. 2. No `transport_key_secret` → `P2P_ICE_UNAVAILABLE` (`The co-op transport is not configured on this
   server.`). 3. Active ICE hunt. 4. Presence. 5. Role; none → `P2P_NOT_ROSTERED` (`This account is not part of the
   hunt.`). 6. `now = clock()`; limit = `configs_per_hour`, or `max(configs_per_hour, 120)` for a realm session;
   `HourlyCounter` key = the account id alone; refused → `P2P_RELAY_RATE_LIMITED` (`Too many co-op transport
   requests; try again later.`). 7. TTL = `relay_ttl_seconds(hunt.expires_at, now, cap)` with cap
   `hub_credential_ttl_seconds` for a hub, else `credential_ttl_seconds`. 8. `grants.grant(account, ttl, now)`.
   9. `ice_peers_for`.

Reply body (field order is the contract):
`{"sessionId":<wire>,"generation":<n>,"role":"HOST"|"JOIN","localAccountId":<account>,"localPeerId":"<decimal or
empty>","hostAccountId":<host>,"hostPeerId":"<decimal>","forceRelay":<bool>,"iceServers":<ice_servers_json>,
"relay":<relay_json>,"peers":[{"accountId":..,"role":..,"transportKey":..,"peerId":"<decimal or empty>"},...]}`.
Peer ids are decimal strings from `peer_id_of`, the empty string when unknown.

`submit_signal(account, session, body)`:

1. Running gate. 2. `parse_ice_signal`; `None` → `P2P_SIGNAL_INVALID` (`The ICE signal is malformed or too large.`).
3. Active ICE hunt with the signal's generation. 4. Presence. 5. `signal_route_allowed(hunt, account, signal.to)`
   else `P2P_NOT_ROSTERED` (`Signals go only between the host and a joiner of this hunt.`). 6. `now = clock()`; limit
   3000 when the hunt is a hub and the sender is its host, else 300; key `<wire>:<generation>:<sender account>`;
   refused → `P2P_SIGNAL_LIMIT` (`Too many ICE signals for this hunt.`). 7. When `force_relay`, `filter_relay_only`;
   `None` → reply `{"delivered":false,"reason":"FILTERED_RELAY_ONLY"}` (status 200; the hit was already counted).
8. `signal_id = random_uuid()` (failure → `RelayError::Random`). 9. Enqueue `ControlMessage { account: signal.to,
   message_type: "ICE_SIGNAL", wire_session, generation, payload, created_at: now, expires_at: min(hunt.expires_at,
   now + 120 000) }` with payload `{"sessionId":<wire>,"generation":<n>,"signalId":<uuid>,"fromAccountId":<sender>,
   "fromRole":<HOST|JOIN, omitted if none>,"kind":<KIND>,"data":<data after filtering>}`. 10. Reply
   `{"delivered":true,"signalId":<uuid>}`.

`set_coop_transport(admin_key_id, body)` answers a whole `Reply` (the admin API's statuses, not the launcher
envelope):

- `coopTransport` must be the string `ICE` or `STEAM_EXPERIMENTAL`; `expectedVersion` must be present and its
  `js_number_value` a safe integer ≥ 0 (`null` counts as 0); `reason` (a string, else empty) is `js_trim`med and cut
  to its first 500 UTF-16 units, and must then be ≥ 8 units. Any failure → status **400**, body exactly
  `{"error":{"code":"ADMIN_VALIDATION","message":"coopTransport (ICE or STEAM_EXPERIMENTAL), expectedVersion, and an
  audit reason of at least 8 characters are required."}}` (one line).
- `request_id = random_uuid()` (failure → `RelayError::Random`); call `policy.set_coop_transport(key, transport,
  expected as u64, reason, request_id)`; its errors propagate.
- `Updated(policy)` → **200** `{"policy":<policy JSON>,"requestId":<uuid>}`. `Conflict(policy)` → **409**
  `{"error":{"code":"ADMIN_POLICY_CONFLICT","message":"The operations policy changed. Refresh it before trying
  again."},"policy":<policy JSON>}`.

### 4.7 Routes

`ROUTES` (this order):

| Method | Path | Mount | `handler` |
| --- | --- | --- | --- |
| PATCH | `/admin/v1/operations/coop-transport` | Admin | `p2pAdminOperations.ts:UpdateCoopTransport` |
| POST | `/launcher/v1/p2p/hunts/:sessionId/transport-config` | Normal | `p2pIce.ts:CreateIceTransportConfig` |
| POST | `/launcher/v1/p2p/hunts/:sessionId/signal` | Normal | `p2pIce.ts:SubmitIceSignal` |

`stub_routes() -> StubRoutes { component: Relay, routes: ROUTES }`. `RelayRoutes` (Clone): `fn new(service:
Arc<dyn IceService>, node: NodeId) -> Self`; implements `RouteHandler`.

Dispatch, session-id, launcher-caller, object-body, generation and error-mapping rules are those of §3.7, with
`Component::Relay`. Per route:

| Handler | Steps |
| --- | --- |
| UpdateCoopTransport | key id = `Caller::Admin { key_id }`'s, else the literal `admin`; body parsed as any JSON value, an unparsable or empty body taken as `{}` (no error); `set_coop_transport`; its `Reply` is returned as is |
| CreateIceTransportConfig | launcher caller; session; object body; generation; `transport_config`; status 200 |
| SubmitIceSignal | launcher caller; session; object body; `submit_signal(account, session, body)`; status 200 |

### 4.8 Example (`mint-relay-file`)

`mint-relay-file <output file> <backend env file>` (exactly two positional arguments, else stderr
`usage: mint-relay-file <output file> <backend env file>`, exit 2):

1. Env reader: each line trimmed; skip empty lines and lines starting with `#`; split at the first `=`; key trimmed;
   value trimmed with one pair of matching surrounding `"` or `'` removed.
2. `RelayConfig::from_settings`; unless the provider is `turn-rest` with its config set → stderr
   `P2P_TURN_PROVIDER=turn-rest, P2P_TURN_REST_URLS and a P2P_TURN_REST_SECRET of 16 or more characters are not all
   set in <env file>`, exit 2.
3. `mint_turn_rest` for a fixed self-test account id, TTL 600, now from the system clock.
4. UDP relays: URLs starting `turn:` that contain no `?` or end with `?transport=udp`. None → stderr
   `P2P_TURN_REST_URLS names no UDP relay (turn:host:port or ?transport=udp)`, exit 2.
5. Write (create or truncate, 0600 on Unix) four LF-terminated lines: `ice_stun_urls=<STUN URLs joined by ,>`,
   `ice_turn_urls=<UDP URLs joined by ,>`, `turn_username=<username>`, `turn_credential=<credential>`. Write error →
   `<output>: <error>`, exit 2.
6. stdout `Wrote relay test file (10-minute turn-rest credentials, <n> UDP relay URLs).`, exit 0. No secret printed.

The self-test account id is `paradox-selftest` today; it only changes the relay tag in the username, so the MIT
example may use any fixed id (§9).

---

## 5. Status codes the refusals reach

isles maps a `SeamError::Refused` code to a status through its registry (`isles-core` `auth/errors.rs`
`status_for`; unknown codes are 500) and writes `{"error":{"code":..,"message":..,"requestId":..}}`. For the codes
these crates use:

| Code | Status |
| --- | ---: |
| `AUTH_UNAUTHORIZED` | 401 |
| `INTERNAL` | 500 |
| `P2P_DISABLED`, `P2P_EMERGENCY_STOP`, `P2P_ICE_UNAVAILABLE` | 503 |
| `P2P_SESSION_NOT_FOUND` | 404 |
| `P2P_SESSION_NOT_READY`, `P2P_STEAM_LINK_REQUIRED`, `P2P_TICKET_REPLAYED`, `P2P_GENERATION_FENCED`, `P2P_TRANSPORT_MISMATCH` | 409 |
| `P2P_NOT_ROSTERED` | 403 |
| `P2P_TICKET_INVALID`, `P2P_SIGNAL_INVALID` | 400 |
| `P2P_TICKET_EXPIRED` | 410 |
| `P2P_RELAY_RATE_LIMITED`, `P2P_SIGNAL_LIMIT` | 429 |
| `P2P_CAPABILITY_REQUIRED` | not registered: 500 (§9) |
| `REALMS_DISABLED` (seam) | per isles' registry |

`Unimplemented` reaches the client as `Reply::not_implemented` (501).

---

## 6. Tests that define the behaviour

### 6.1 Unit tests to rewrite with the B code

Each line is one test the MIT crate carries, stated as its assertion; the values are the ones above.

`rifts-seam`:

- `GuardMode::from_setting`: `None`, ` observe `, `OBSERVE` → Observe; `enforce`, `off` → Enforce.
- `NoCore::display_name` yields `Unimplemented(Isles, "display_name")`; `set_candidate` errors.
- `p2p_module_enabled`: missing, `on`, empty → on; `0`, `false`, ` OFF `, `No` → off.
- `Rifts::load` with the module off never calls the parts closure; `loaded()` false, no routes.
- Dedicated-only: `route_hunt` → `NotHandled`; `dedicated_hunts_disabled` → false; `authenticate` → `NotMine`;
  `authorize_host_write` → 403 with R2 exactly; `authorize_inventory` → `Refused` with status 403;
  `host_policy_refuses` → true; realm `None` → `Ok(None)`, realm `Some` → `REALMS_DISABLED`.
- `Reply::not_implemented` example of §2.1.5; `RequestInfo::header` finds `x-mystic-host-capability`, not `x-other`.
- Ulid round trip through text (upper and lower case), `unix_millis`, `Ulid(u128::MAX)` text; `8ZZZ...` (26) and a
  `U` and a 25-character string refused; `GlobalId` without `@` → `BadGlobal`; node `-x` and `New-York` → `BadNode`.
- `Secret` Debug is `Secret(<redacted>)`.

`rifts-admission`:

- Round trip of every binding through mint and verify; the payload layout bytes of §3.2.2's example; signature text
  86 characters; `decode_claims(payload) == claims`.
- Tampered signature → `BadSignature`; expired → `Expired` with `is_expired()` and a message containing `expired`.
- Virtual port 999 passes, 1000 → `OutOfRange("virtualPort")`, 0 → `ZeroVirtualPort`.
- Another key id → `KeyNotActive`; another key → `BadSignature`; skew edges `NOW-30` ok, `NOW-31` `NotYetValid`,
  `NOW+105` ok, `NOW+106` `Expired`; a signed 181-second ticket → `InvalidLifetime`; `not_before < issued_at` →
  `InvalidLifetime`.
- Minting lifetimes: 14 and 181 refused; 180 and default 75 accepted; NaN and 75.5 defaults refused; no signing key →
  `NoSigningKey`.
- Identifier and peer checks: `a b`, 97-character account, empty party, `hunt=1`, zero joiner, party revision 2^53,
  all-zero session, key id `a/b`; zero generation, build and bridge protocol are accepted.
- Canonical encodings: trailing `=` on the token, padded payload, standard-alphabet payload, an 85-character
  signature → `NoncanonicalEncoding`; prefix `mpt2`, two parts, empty payload, four parts → `MalformedToken`; 4097
  bytes → `MissingOrOversized`; a payload with a non-zero trailing bit is refused.
- A high-S signature (negated `s`) verifies; `r = 0` → `BadSignature`.
- Payload framing: 105 bytes and a bad magic → `MalformedPayload`; version 3, reserved byte, key length off by one,
  key length 0, trailing byte → `Framing`; port bytes → `ZeroVirtualPort` / `OutOfRange("virtualPort")`; session
  version nibble 0 → `SessionNotUuid`; a space or 0xFF in the first identifier byte → `UnsafeIdentifier`; a time with
  no exact double → `NoncanonicalPayload`; zero joiner → `InvalidPeerId`.
- Session UUIDs in upper case parse; malformed ones (wrong dashes, version 0 or 6, variant 7 or c) refused;
  `hash_jti("zz")` → `InvalidJti`; case-insensitive jti digests.
- `parse_unverified` reads a minted ticket; `mpt1.AAAA.AAAA` errors.
- Keys: a generated pair loads back through settings, its XY equals the public point, default lifetime 75, the env
  lines start `P2P_ADMISSION_KEY_ID=k1\nP2P_ADMISSION_SIGNING_PRIVKEY_B64=` and end with LF, Debug never shows the
  signing key; settings errors (`bad id` → `KeyId`, missing verify → `Missing`, missing signing → verify-only,
  `bm90IGEga2V5` signing → `Unreadable`, blank lifetime → 0.0, generator with `../x` → `Key(KeyId)`); SEC1 private
  keys and base64 wrapped at 60 characters with CRLF load; the key set is written once, files match, the temporary
  folder is gone, a second write → `Exists(absolute)`, a missing parent → `NoParent`, an empty id → `Usage`.
- MPC1: the request example bytes; empty and 4097-byte tickets refused, 4096 accepted; the admitted result round
  trip (`01 02 40 00` after the header), a 63-byte id fails to decode, a refusal round trip; bent magic, version, type,
  reserved, declared length, ticket length, a truncated frame, an oversize frame and an empty input are refused.
- Memory store: consumed once, then idempotent for the same bridge and request id, a replay for another request id or
  bridge; an unknown digest → `Invalid`; expired → `Expired`; another host → `Invalid`; a revoked ticket →
  `Invalid`; one live admission per active key; disconnect only by the admitting bridge, once; supersede clears the
  key and sets `superseded_at`. `StubAdmissionStore` answers `store.peer_link`; `TABLES` all owned by Admission and
  prefixed `rifts_`.
- Service (with a fake `AdmissionHunts`): a joiner is ticketed, admitted once, idempotent on retry, disconnected;
  self-ticket refusals in §3.6.3 order; consume refusals in §3.6.5 order; an expired ticket answers
  `P2P_TICKET_EXPIRED`; a hub member gets one ticket per entry and a new entry supersedes the old admission; a hub host
  lists live peers with pair keys (43 characters) and nobody else may; the join bootstrap appends `ticket` and
  `lobbyId` to realms' fields.
- `derive_ice_peer_id` is nonzero, top bit clear, stable, account-dependent. Stubs answer `Unimplemented` with the
  operation names of §3.1.1. `ROUTES` equals §3.7's table; `stub_routes()` answers every route with
  `Unimplemented(Admission, handler)`.

`rifts-relay`:

- Config defaults and bounds (§4.3 examples), providers needing their keys (`" TURN-REST "` with a short secret is
  TurnRest without config; Cloudflare keys ignored under turn-rest; `none`; `coturn` → Cloudflare), URL patterns
  (`turns:...:5349?transport=tcp`, `turn:1.2.3.4` yes; port of 6 digits, `?transport=sctp`, `turn::3478`,
  `stun:host?transport=udp` no; `stuns:host:5349` yes), numbers (`1e2`, `0x12c`, ` True `, `1`, quota `0`), the
  summary JSON exact text for a turn-rest config, no secret in Debug.
- Credentials: provider answers in both shapes, port 53 dropped, bad credentials refused, `None`/`null` body;
  turn-rest username and password, RFC 2202 case 2; relay tag 32 lower-case hex, stable, not the account; no provider
  request carries the account id; STUN only until configured; Cloudflare tagged then untagged on 400 (two calls,
  bearer, URL, 5000 ms, bodies), provider error → `RELAY_PROVIDER_ERROR`; quota from analytics, cached for 10
  minutes, unreadable figure does not block; the analytics query text exactly (§4.4.6); usage sums; hourly counter,
  month window, TTLs (4200, 300, 602); both providers through `CredentialIssuer`.
- Rules: roles (host, joiners, returned-to-hub refused, strangers refused); ICE-active and ticketable sets; pair keys
  equal for host and joiner, different across pairs and generations, 43 characters; signal parsing cases (§4.5.4,
  including `café`, a number as data, `-1` generation, `"3"` generation, blank description, `GATHERING_DONE` without
  data); routing host↔joiner only; the relay-only filter on the CRLF description example (keeps ufrag, pwd, options,
  the relay candidate and end-of-candidates, in order), host candidate refused, relay candidate kept, a description
  with only a relay candidate → `None`, `GATHERING_DONE` passes, `typ relayed` refused; hub members from presence rows.
- Wire: `json_string` of `a"b\c/é\u0001\u0008\n` is `"a\"b\\c/é\u0001\b\n"`; object order; ISO examples (0, 2026-10-13
  08:00, 2000-02-29 .123, -1); `js_number` cases; `safe_integer` cases; `js_trim` (BOM and U+2028 trimmed, U+0085 not);
  `random_uuid` version 4.
- Service (with fakes): a joiner gets STUN, turn-rest and its pair with the host; the host gets every active joiner;
  transport-config refusals in §4.6 order; per-account config limit; a hub uses presence rows and the hub TTL; signals
  are queued to the other side with the payload of §4.6; relay-only filtering (`FILTERED_RELAY_ONLY`) and refusals;
  per-sender signal limits; coop-transport validation (400 body), update (200) and conflict (409).
- `ROUTES` equals §4.7's table; `stub_routes()` answers `Unimplemented(Relay, handler)`. Stubs and `TransportKeys`
  (`None` → `P2P_ICE_UNAVAILABLE`; a 32-character secret → a 43-character key).

### 6.2 Oracle suites that stay in dainty-restricted (D)

They need no change to run against the MIT crates; only their crate's dev-dependencies change (they need
`serde_json` and, for admission, `base64`). The JSON is loaded with `include_str!("vectors/<file>.json")`, so each
harness must stay beside its `vectors/` folder: keep them as `tests/` of a small oracle package in dainty-restricted
(for example `modules/rustic-rifts/crates/rifts-oracles`, `publish = false`, AGPL) whose dev-dependencies are the MIT
`rifts-admission`, `rifts-relay` and `rifts-seam`.

`rifts-admission/tests/node_vectors.rs` (4 tests; 5 issued tickets, 349 verdicts, 3 jti digests, 4 ICE peer ids):

- `rifts_admission::Es256Tickets::from_settings(&HashMap<String, String>)` with only `P2P_ADMISSION_KEY_ID` and
  `P2P_ADMISSION_VERIFY_PUBKEY_B64`; `.public_xy().0` (64 bytes, hex compared); inherent `.verify(&str, u64)`.
- `TicketClaims` fields `key_id.0`, `jti` (Display), `bindings.{account.0, joiner_peer.0, host_peer.0, session
  (Display), generation, party_id, party_revision, hunt_definition_id, build, bridge_protocol, virtual_port}`,
  `issued_at`, `not_before`, `expires_at`, `jti_digest()`.
- `rifts_admission::ticket::{decode_claims, encode_claims, hash_jti}`; `TicketError` Display texts (compared
  verbatim against Node's messages) and the variant `TicketError::NoncanonicalPayload`.
- `rifts_admission::derive_ice_peer_id(&AccountId).0`; `rifts_seam::AccountId`.

`rifts-relay/tests/node_vectors.rs` (6 tests):

- `rifts_relay::relay_account_tag`, `mint_turn_rest` (fields `username`, `credential.expose()`, `urls`),
  `TurnRest { urls, secret }`, `derive_pair_key`, `normalize_provider_ice_servers` (fields `stun_urls`,
  `turn: Option<(Vec<String>, String, Secret)>`), `RelayConfig::from_settings` (fields `stun_urls`,
  `turn_rest.urls`, `cloudflare.key_id`, `credential_ttl_seconds`, `hub_credential_ttl_seconds`, `configs_per_hour`,
  `monthly_quota_bytes`, `analytics.account_id`; `summary_json()`), `read_monthly_relay_egress_bytes` with a
  recording `ProviderHttp` (fields `ProviderRequest.url`, `.body`, compared byte for byte with Node's),
  `CloudflareAnalytics { account_id, api_token }`, `ProviderAnswer { status, json }`, `ProviderUnreachable`.
- `rifts_relay::rules::{SignalKind, filter_relay_only, parse_ice_signal}` (`IceSignal` fields `generation`, `to.0`,
  `kind.as_str()`, `data`).
- `rifts_seam::{AccountId, BoxFuture, Secret, UnixMillis, poll_once}`.

`make-vectors.mts` regenerates both JSON files from Mystic's Node modules in `node:22`; it is the oracle's provenance
and runs only in dainty-restricted.

### 6.3 Fuzz targets (A, move to dainty-lass `fuzz/`)

- `admission_control`: any input that decodes as a request re-encodes to itself (ticket 1-4096, frame ≤ 4608); any
  result that decodes re-encodes to itself, code 1-64, id ≤ 64, admitted implies a 64-byte id.
- `admission_ticket`: a decoded payload is in the transport's ranges and re-encodes to itself; `parse_unverified` on
  UTF-8 input gives sane claims; with a fixed generated key (built through `P256KeyGenerator::generate_with_pems`,
  `keys::signing_key_from_pem` and a struct literal of `Es256Tickets`'s four pub fields), a successful `verify` at the
  unverified `issued_at` equals the unverified parse.
- `relay_signal`: an accepted signal is bounded printable SDP; the filter output only contains lines the sender sent
  (as split, CR-dropped and `js_trim`med), every candidate line in it contains ` typ relay`, a `CANDIDATE` output is a
  candidate line, and `GATHERING_DONE` is unchanged.

---

## 7. Target MIT crates

### 7.1 Where they go and what they depend on

Three packages in dainty-lass, keeping their package and crate names so dainty-restricted switches by changing only
`[workspace.dependencies]` to git deps at one rev: `crates/rifts-seam`, `crates/rifts-admission`,
`crates/rifts-relay`, each `license = "MIT"`, `version/edition/rust-version/repository/publish` from the workspace,
`[lints] workspace = true`.

| Crate | Dependencies (all `workspace = true`) |
| --- | --- |
| `rifts-seam` | `thiserror` |
| `rifts-relay` | `base64`, `getrandom`, `hmac`, `rifts-seam`, `serde_json`, `sha1`, `sha2`, `thiserror` |
| `rifts-admission` | `base64`, `getrandom`, `p256`, `rifts-relay`, `rifts-seam`, `serde_json`, `sha2`, `thiserror` |

dainty-lass's `[workspace.dependencies]` already has `base64 = "0.23"`, `hmac = "0.13"`, `serde_json` (with
`float_roundtrip`), `sha1 = "0.11"`, `sha2 = "0.11"`, `thiserror = "2"`.

### 7.2 Seam changes (integration pass)

1. **New workspace dependencies**: `getrandom = "0.4"` (the key generator uses its system RNG as the `rand_core`
   source for p256) and `p256 = { version = "0.14", default-features = false, features = ["ecdsa", "pem", "pkcs8",
   "std"] }`, matching dainty-restricted's (`p256` 0.14.0, `getrandom` 0.4.3 in its lock). `p256` types are in the
   public API (`Es256Tickets` fields, `parse_signed`'s closure, `signing_key_from_pem`), so both repositories must
   resolve the same `p256` major version.
2. **Three workspace members** and path entries `rifts-seam`, `rifts-relay`, `rifts-admission` in
   `[workspace.dependencies]`.
3. **`fuzz/Cargo.toml`**: dependencies on the three crates and `serde_json` (with `float_roundtrip`), and three
   `[[bin]]` entries (`admission_control`, `admission_ticket`, `relay_signal`, `test = false`, `doc = false`,
   `bench = false`).
4. **dainty-restricted** afterwards: `[workspace.dependencies] rifts-seam/rifts-admission/rifts-relay` become git deps
   at the dainty-lass rev; the three crate folders, the three fuzz targets and the two checklists are deleted there;
   the oracle package of §6.2 is added; `NOTICE.md` lines move.
5. `cargo deny`: no new licence (all deps are MIT/Apache-2.0 already allowed).

### 7.3 Edits to the A files when they move

- Manifests: `license = "MIT"` instead of `license.workspace = true`; `rifts-admission`'s description may keep
  "MPT2" (a wire name) or drop it (§9).
- `docs/security/admission.md` and `relay-credentials.md`: replace the private test-function names they cite with the
  MIT tests' names, point "tests/node_vectors.rs" at the restricted oracle package, and keep the rest. They become
  `docs/security/admission.md` and `docs/security/relay-credentials.md` in dainty-lass and are indexed by the
  integration pass in `SECURITY_REVIEW.md`.
- Fuzz targets: unchanged (they use only the public paths listed in §3.1 and §4.1).

### 7.4 Clean-room procedure

The implementer reads this document, the A files and the public API of `isles-core`/`isles-1-14-7` call sites, never
the B files. The reviewer measures each new file against the B file it replaces with `clean-room/similarity.md` §1
(target under 30% of 3+-line runs per file), then runs §6.2's oracles in dainty-restricted against the new crates.

---

## 8. Acceptance criteria

1. The three crates build in dainty-lass with `cargo clippy --workspace --all-targets --locked -- -D warnings`, no
   `unsafe`, no `unwrap`/`expect` outside tests, `license = "MIT"`.
2. Every public item of §2.1, §3.1 and §4.1 exists with the stated name, path, signature, field names and derives;
   dainty-restricted builds `isles-core`, `isles-1-14-7`, `rifts-realms`, `rifts-guard`, `rifts-host` and the
   launcher against them with only the dependency switch.
3. §6.1's tests pass in dainty-lass.
4. §6.2's oracles pass unchanged in dainty-restricted against the MIT crates: all 349 ticket verdicts message for
   message, the 5 issued tickets re-encode byte for byte, and every relay vector, including the usage query body.
5. The fuzz targets build and run (`cargo +nightly fuzz run <target> -- -max_total_time=60` clean for each).
6. Similarity per file under 30% (§7.4).
7. Rust-minted tickets are accepted by the transport's verifier and `ParadoxTransport.dll`'s joiner (the maintainer's
   harness, RF4; hands-on, not CI).

---

## 9. Risks and ambiguities

1. **Mystic names in wire contracts** (umbrella spec decides keep or rename; renaming any of them breaks a deployed
   peer):
   - the header `x-mystic-host-capability` (`HOST_CAPABILITY_HEADER`) and the route prefix `/mystic/v1/hunts/...`
     (the host transport calls them);
   - the KDF and tag labels `mysticparadox-ice-peer-v1`, `mysticparadox-relay-tag-v1`, `mysticparadox-ice-pair-v1`
     (changing one changes every derived peer id, relay tag (and so Cloudflare's per-account usage history) or pair
     key, and the transport and the backend must agree);
   - the setting `MYSTICPARADOX_P2P_MODULE`, and the doc mention of `MYSTICPARADOX_GUARD_MODE`;
   - the magic strings `MPT2` and `MPC1` and the token prefix `mpt1` (the transport parses them);
   - the files `p2p-admission-public.xy` and `metagame-admission.env` (hosts' existing key sets);
   - the `Route.handler` strings (`p2pAdmission.ts:...`, `p2pIce.ts:...`, `realmHubAdmission.ts:...`,
     `p2pLauncher.ts:...`, `p2pAdminOperations.ts:...`) name Mystic's Node files and appear to clients in 501
     messages; they are also the stubs' operation names that tests compare;
   - the Node collection names in `TableSpec.node_collection` (`p2pSteamLinks`, ...), documentation only;
   - the example's self-test account id `paradox-selftest` (free to rename; only the test file's username changes).
2. **Steam-flavoured names** (`hostSteamId`, `steamId`, `authenticatedSteamId`, `P2P_STEAM_LINK_REQUIRED`, `invalid
   SteamID64`) are wire fields and error texts the launcher and transport read; keep them.
3. **`P2P_CAPABILITY_REQUIRED` is not in isles' status registry**, so a missing capability answers 500, not 401.
   Recorded as observed; the fix belongs to isles' registry, not to this rewrite.
4. **Two JSON string escapers.** The seam's (`Reply::not_implemented`) writes U+0008/U+000C as `\u0008`/`\u000c`;
   `wire::json_string` writes `\b`/`\f` as `JSON.stringify` does. Keep both until a seam change unifies them.
5. **`generation` parsing in the routes** accepts any i64 integer, not only safe integers, while floats are bounded to
   ±(2^53 - 1); `parse_ice_signal` uses `safe_integer`. Recorded as observed.
6. **Idempotent consume re-runs realms' admit step** (`admit_hub_peer` / `mark_participant_admitted`), which must
   itself be idempotent; a `false` there turns a retry into `P2P_GENERATION_FENCED`.
7. **No transaction**: consume writes ticket status, then admission, then realms' rows without a transaction (store's
   API pending). A crash in between leaves a consumed ticket with no admission.
8. **Disconnect ignores the `enabled` flag** (only the emergency stop), and `disconnectedAt` from the body is not read
   by the route. Recorded as observed.
9. **Session ids from route params**: a param that is neither `<ULID>@<node>` nor a lower-case UUID becomes ULID 0 on
   the route's node instead of a refusal; upper-case UUIDs fall to ULID 0 too.
10. **`p256` in the public API** ties both repositories to the same `p256` 0.14 (a pre-1.0 RustCrypto line); a later
    bump is a coordinated change. RFC 6979 deterministic signing is acceptable (Node signs randomly; both verify).
11. **High-S acceptance must be preserved**: a verifier that requires low-S would refuse tickets Node or the
    transport consider valid.
12. **`AdmissionError::Conflict` and ticket `OutOfRange("generation")` surface as `INTERNAL` 500** with their Display
    text as the message; that text reaches the client.
13. **The route tests in restricted read `docs/inventory.json`**, which does not move; the MIT tests compare against
    this spec's tables instead, so a later route change must update both.
14. **`JsonObject::default()`** yields a builder without the opening brace; it is derived, public and unused. Keep the
    derive for API compatibility or drop it as a breaking change agreed with the umbrella spec.
15. **Hand-run checks remain**: the transport's acceptance of Rust-minted tickets and the turn-rs relay file
    (`mint-relay-file`) are the maintainer's; CI passing says nothing about them.
