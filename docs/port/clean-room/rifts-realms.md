# Clean-Room Provenance Record: `crates/rifts-realms` (RD27)

- **Component**: `crates/rifts-realms` (crate `rifts_realms`, `license = "MIT"`, `Copyright (c) 2026 Silent Spud`)
- **Behaviour specification**: `docs/port/spec-rd27-rifts-realms-store.md` (persistence and wiring) and
  `docs/port/spec-rd27-rifts-realms.md` (domain), with the umbrella `docs/port/spec-rd27-rifts.md` (its §2 D3 rename
  table overriding the part specs' old identifiers), at dainty-lass `b6ddc44` (branch `feat/rd27-rifts-mit`, which
  holds the specs and the MIT `rifts-seam`, `rifts-relay`, `rifts-admission`, `rifts-guard`)
- **Written in two parts, one implementer after the other** (umbrella §4.2 step 4): part 1 the store half, part 2
  the domain rules, on the same branch `feat/rd27-rifts-realms`

## Part 1: persistence and wiring

- **Clean-room worktree**: `R:\source\dauntless\dainty-lass\.claude\worktrees\agent-ad3d2a93ab0ae88f2` (branch
  `feat/rd27-rifts-realms`, cut from `origin/feat/rd27-rifts-mit` at `b6ddc44`)
- **Implementer**: one Claude Code subagent session (model Claude Opus 5.5), 2026-11-02, briefed by the RD27
  coordinator with the rules below

### 1. Isolation

It never read, listed, grepped or `git show`ed anything under `dainty-restricted` (any worktree of it),
`modules/rustic-rifts`, `modules/rustic-isles`, Mystic Paradox's repository or libjuice's sources, and ran no
`git log -p` or `git show` on any commit. It searched nothing on the web. The session's environment listed a
dainty-restricted worktree as an additional working directory; no tool call read from it. dainty-lass held no realms
source at `b6ddc44`: `crates/rifts-realms` did not exist until the implementer created it.

### 2. Files read

- `docs/port/spec-rd27-rifts.md` (whole), `docs/port/spec-rd27-rifts-realms-store.md` (whole)
- `docs/port/spec-rd27-rifts-realms.md`: §1, §2.0-§2.4, §2.6.1-§2.6.2, §2.7, §2.8 (to §2.8.6), §2.9 (to §2.9.3),
  §2.14, §2.15, §2.16.13, §3-§8
- `docs/port/clean-room/rifts-guard.md` (this record's format)
- `AGENTS.md`, `Cargo.toml`, `crates/rifts-admission/Cargo.toml`
- `crates/rifts-seam/src/*.rs` (whole), `crates/rifts-guard/src/{lib.rs (API, `Guard`, the host write, inventory and
  bucket paths), ports.rs, sql.rs (query idioms, `block_on_store`), config.rs (`GuardConfig`), routes.rs, store.rs
  (`GuardStore`, `HostWriteRecord`), writes.rs (the denial rules), testkit.rs}`, `crates/rifts-guard/tests/sql_store.rs`
  (the Postgres test harness), `crates/rifts-guard/migrations/sqlite/.../up.sql` (head)
- `crates/rifts-admission/src/routes.rs` and `crates/rifts-relay/src/routes.rs` (their neutral handler ids),
  the `rand` call sites in `crates/rifts-*/src` and `crates/admin/src`
- `crates/store/src/{lib.rs, database.rs, engine.rs, connection.rs, portable.rs}`
- `crates/module-api/src/{network.rs (NodeId, GlobalId), ids.rs, lib.rs}`, `crates/module-api/Cargo.toml`
- The brief (`impl-brief.md` in the coordinator's scratchpad)

### 3. Files written

| File | What |
| --- | --- |
| `crates/rifts-realms/Cargo.toml` | Manifest, `license = "MIT"`, `serde_json` with `preserve_order` (store spec §14) |
| `migrations/{sqlite,postgres}/2026-10-26-000001_rifts_realms/{up,down}.sql` | §4.2's DDL byte for byte; the two `up.sql` differ in the `COLLATE "C"` line (and a comment line saying why); `down.sql` as §4.4 |
| `src/lib.rs` | Modules, re-exports, `RealmsError` and its conversions, `TABLES` (§2) |
| `src/db.rs` | `RealmsDb`, the migration triple, `RepoError`, `store_actor` (§3) |
| `src/repo/mod.rs`, `realms.rs`, `hub.rs`, `hunts.rs` | §6-§8: every method, one `Database::write` per change |
| `src/routes.rs` | `ROUTES` (46, mount order, D3 paths and neutral ids), `RealmsRoutes(Parts)`, `stub_routes`, the request helpers and the dispatch skeleton (§9) |
| `src/service.rs` | `RealmsService(Parts)`, `realms_enabled_or_default` (§10) |
| `src/ports.rs` | §11 in full |
| `src/stub.rs` | §12 |
| `src/time.rs`, `src/ids.rs` | Domain §2.2 and §2.3 in full (the repositories and tests need them) |
| `src/realm.rs` | Domain §2.4's enums and records, `apply_hosting_patch`, `default_hosting`, `INVITE_PURGE_GRACE_MS`, invite code generation, `hub_room_id` (label `rustic-realm-hub:v1:`), `realms_module_enabled`, `js_trim`, `RealmLimits`/`DEFAULT_REALM_LIMITS` |
| `src/hunt.rs` | Domain §2.7's records in full (serde shapes included) |
| `src/hub.rs` | Domain §2.8.4: presence states, events, table, expiry, the entry decision |
| `src/capabilities.rs` | Domain §2.14.5-§2.14.6: `HubPresenceCache`, `RealmsHuntSessions`; `HostCapabilities` with `new`, `hub_presence`, `hunt_sessions` only |
| `src/control.rs` | Domain §2.15 in full (`ControlChannel`, views, poll, ack, send) |
| `src/tiers.rs` | `HostingPolicy`'s fields (§2.9.2), `js_number` (§2.9.1), `relay_only_warning` |
| `src/lifecycle.rs` | `GuardRole` (the enum the ports name) |
| `src/directory.rs` | `DirectoryParts`, `RealmDirectory::new` |
| `src/hosting.rs` | The `HuntLifecycle` trait and `HostBootstrap`, `HuntStatus`, `Observation`, `StartHub` |
| `src/leases.rs`, `input.rs`, `selection.rs`, `host_policy.rs` | Placeholders for part 2 (`Hosting` cannot be built yet) |
| `tests/common/mod.rs` | Two-engine targets, the Postgres switch, fixtures (§13.1) |
| `tests/realms_repository.rs`, `hub_repository.rs`, `guard_adapter.rs`, `stubbed.rs` | §13.2 |
| `Cargo.toml`, `Cargo.lock` | Seam change: `rifts-realms = { path = "crates/rifts-realms" }` in `[workspace.dependencies]`; the lock gains the new package (no new external crate) |
| `docs/port/clean-room/rifts-realms.md` | This record |

Every `.rs` file starts with `// SPDX-License-Identifier: MIT` and `// Copyright (c) 2026 Silent Spud`.

### 4. Ambiguities and the choices made

1. **D3 renames**: paths `/rustic/v1/hunts/:sessionId/...`, header `x-rustic-host-capability` (rifts-seam's
   constant), setting `RUSTIC_RIFTS_REALMS_MODULE`, hub room label `rustic-realm-hub:v1:`. The domain spec's hub room
   vectors were computed with the old label and no longer hold; the tests check shape and stability instead.
2. **Neutral handler ids** were invented as `realms.<operation>` (`realms.join`, `realms.mark_host_ready`,
   `realms.unknown_route` for the catch-all, ...), as admission, relay and Guard did. They replace the Node ids of
   §9.1; restricted's inventory must take them at the cutover.
3. **`UnixMillis`** in this crate is `pub type UnixMillis = i64` (`time`), per the domain spec's "UnixMillis, i64".
   rifts-seam's newtype appears only at the Guard and seam boundary. If restricted's callers pass the seam newtype to
   realms APIs, part 2 or the integration pass must adjust.
4. **`HuntSession` field types** not spelled out: `party_revision`, `build`, `bridge_protocol`, `virtual_port`,
   `max_peers`, ports and sequences are `i64`; `trust_tier` is `Option<u8>`; `hub_kind` is rifts-seam's `HubKind`
   serialised as `RAMSGATE`/`TRAINING`.
5. **`HuntLifecycle`'s signatures** are not given by either spec. Chosen: methods taking owned seam ids and returning
   `BoxFuture<'_, Result<T, RealmsError>>`; `HuntStatus.verdict: Option<rifts_guard::HuntVerdict>`;
   `Observation`'s metrics as `Option`s. Part 2 owns the trait and may reshape it to match restricted's callers.
6. **`reserve_participant_ticket`**: "the participant is disconnected" is read as `disconnected_at` set.
7. **`create_offer`**: an accepted offer is refused when any accepted offer of its (session, generation) exists, the
   same offer id included (as written).
8. **Order ties** (store §17 item 7): `list_transitions` adds `transition_id`, `list_live_realm_hunts`,
   `list_lifecycle_hunts` and `list_host_offers` add the id as a final key.
9. **`realms_enabled_or_default`**: a non-blank value defers to `realms_module_enabled`, which treats only
   `on`/`true`/`1` as on (domain §2.4.10), so `yes` is off; the store spec's "0, false, off, no as off" wording would
   have kept it on. Followed the domain spec's rule.
10. **`request_entries`' live count** uses the generation's rows; "newcomers" use each account's row of the session
    in any generation, as written. A `Reuse` keeps a waiting row of an older generation (the entry decision looks
    only at state and expiry).
11. **Hub `remove_*` and `end_session`** apply the event to the live *states* (no expiry test), as the presence table
    says; `expire_stale` reaches only waiting rows.
12. **Postgres** was not run in this session (`REFORGED_TEST_POSTGRES_URL` unset); every suite ran on SQLite only.

### 5. What part 2 must fill in

- **Modules to write**: `directory` (all of §2.6 on `RealmDirectory`, which holds `DirectoryParts` behind an
  `allow(dead_code)` field), `leases` (`Hosting`, `HostingParts`, `HostingRuntime`, `HostingSettings`, `SweepStep`;
  `Hosting` is a placeholder with a private unit field today), `lifecycle` (rules, steps, `RealmsRecovery`,
  `Recovery`), `selection`, `host_policy`, `input`, the rest of `hub` (§2.8.1-§2.8.3, §2.8.5-§2.8.6), the rest of
  `realm` (names, `decide_join`, online state, `validate_hosting_patch`, `normalize_*`, `RealmLimits::from_settings`,
  the remaining constants), `tiers` (`HostingPolicy::from_settings`, routing, the bucket), `capabilities`
  (`HostCapabilities::check`, `mint`, `revoke`, `resolve_runtime`, `runtime_identity`, `runtime_auth`,
  `SignedUser`, the secret helpers, `hub_present_accounts`), and `hosting` (every route behaviour of §2.16, the
  adapters for rifts-admission and rifts-relay, `RealmEventSink` and `HuntLifecycle` for `Hosting`,
  `resolve_manifest_for_hunt`).
- **Root re-exports to add** once they exist: `resolve_manifest_for_hunt` (hosting), `HostingRuntime` (leases),
  `RealmsRecovery` (lifecycle).
- **`routes.rs`**: every place that answers `Err(unimplemented(handler))` after its pre-checks is a domain call to
  write, in the step order of §9.3-§9.4: realms routes 17-44 (input parsers, then the directory), session routes
  1-5 (`HostCapabilities::check` before the body, route 3's three scopes in turn), route 4's caller rules, 6 and 46
  (`HuntLifecycle` on `Hosting`), 7, 8, 11-16. Routes 9, 10 (control) and 45 (catch-all) are complete.
- **`service.rs`**: the `domain(..)` answers: `authenticate` (resolve runtime, identity, `runtime_auth`),
  `restore_candidate`, `candidate_status`, `route_hunt` (with the directory's active presence), and the
  `RealmsEntry` calls with a directory (`authorize_game_session_realm` and its id conversion,
  `bind_realm_presence_at_login`, `touch_presence`, `find_by_hub_room_id`, `is_member`, `is_banned`).
- **Tests**: `tests/common/kit.rs` and the domain suites of domain §5.1 (`realm_rules`, `hub_rules`,
  `lifecycle_rules`, `tiers`, `host_policy`, `leases`, `directory`, `directory_presence`, `capabilities_control`,
  `hosting_bootstrap`); route dispatch tests for each route once it answers.
- **Dependencies**: `rifts-admission` and `rifts-relay` are in the manifest (store spec §14) but unused by part 1's
  source; part 2's adapters use them.

### 6. Verification

- `cargo fmt --all --check`: clean.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: clean (after the lock gained `rifts-realms`).
- `cargo test -p rifts-realms --no-fail-fast` (SQLite; Postgres skipped, `REFORGED_TEST_POSTGRES_URL` unset):
  lib 41, `guard_adapter` 6, `hub_repository` 20, `realms_repository` 31, `stubbed` 2: **100 passed, 0 failed**.
- `cargo tree -e normal -p rifts-realms --depth 1`:

```text
rifts-realms v0.0.0 (crates\rifts-realms)
├── diesel v2.3.14
├── diesel_migrations v2.3.2
├── module-api v0.1.0 (crates\module-api)
├── rand v0.10.3
├── rifts-admission v0.0.0 (crates\rifts-admission)
├── rifts-guard v0.0.0 (crates\rifts-guard)
├── rifts-relay v0.0.0 (crates\rifts-relay)
├── rifts-seam v0.0.0 (crates\rifts-seam)
├── serde v1.0.229
├── serde_json v1.0.154
├── sha2 v0.11.1
├── store v0.0.0 (crates\store)
├── thiserror v2.0.21
└── tokio v1.54.1
```

Every path dependency is an MIT crate of this workspace; no AGPL crate is in the tree.

### 7. Seam changes for the integration pass

- `[workspace.dependencies]`: `rifts-realms = { path = "crates/rifts-realms" }`.
- `serde_json`'s `preserve_order` turned on in this crate's own manifest (Cargo unifies it workspace-wide).
- Restricted's route inventory must take the neutral handler ids of §4 item 2.

## Part 2: the domain rules

- **Clean-room worktree**: `R:\source\dauntless\dainty-lass\.claude\worktrees\agent-aa2f36b7dc710a153`, on local
  branch `feat/rd27-rifts-realms-domain`, cut from `origin/feat/rd27-rifts-realms` at `dbe7bfe` (part 1). The name
  `feat/rd27-rifts-realms` was held by part 1's worktree, so git refused to check it out a second time; the branch
  tracks `origin/feat/rd27-rifts-realms` and is meant to land there.
- **Implementer**: one Claude Code subagent session (model Claude Opus 5.5), 2026-11-02, briefed by the RD27
  coordinator with the same rules, plus the spec writer's public-API errata (`realms-api-errata.md` in the
  coordinator's scratchpad: `rifts_seam::UnixMillis` times, `HostBootstrap`, `StartHub`, `HuntStatus`, `Observation`,
  `HuntLifecycle`, `leases::ObservationReply`)

### 1. Isolation

It never read, listed, grepped or `git show`ed anything under `dainty-restricted` (any worktree of it),
`modules/rustic-rifts`, `modules/rustic-isles`, Mystic Paradox's repository or libjuice's sources, ran no `git log -p`
or `git show`, and searched nothing on the web. The session's environment listed a dainty-restricted worktree as an
additional working directory; no tool call read from it.

### 2. Files read

- `docs/port/spec-rd27-rifts.md`, `docs/port/spec-rd27-rifts-realms.md` (whole), `docs/port/spec-rd27-rifts-realms-store.md`
  (§1-§3, §9-§17), this record's part 1
- The brief and the errata (coordinator's scratchpad), `AGENTS.md`
- Part 1's `crates/rifts-realms` (all of `src/`, the test harness and the repository tests' head)
- `crates/rifts-seam/src/{store.rs, hooks.rs, http.rs, id.rs, lib.rs}` (types), `crates/rifts-admission/src/{lib.rs,
  service.rs (AdmissionHunts, Admission, the bootstrap and consume paths), store.rs, ticket.rs (types), keys.rs
  (`Es256Tickets::from_settings`)}`, `crates/rifts-relay/src/{lib.rs, rules.rs (types), service.rs (IceHunts,
  OperationsPolicy, ControlMessage)}`, `crates/rifts-guard/src/sql.rs` (`block_on_store`)

### 3. Files written or changed

| File | What |
| --- | --- |
| `src/time.rs` | `UnixMillis` is now rifts-seam's newtype (errata); `now`, `iso`, `parse_iso` use it; `iso_at` for stored `i64` times |
| `src/ports.rs` | `Clock`, `SystemClock`, `ManualClock` on the newtype |
| `src/realm.rs` | §2.4: invite states, names, release, ownership, invite rules, `decide_join`, online state, `validate_hosting_patch`, `RealmLimits::from_settings`, presence constants and the touch rule, `normalize_hub_room_id` |
| `src/input.rs` | §2.5 in full |
| `src/tiers.rs` | §2.9: `HostingPolicy::from_settings`, routing, the bucket |
| `src/host_policy.rs` | §2.10 in full |
| `src/hub.rs` | §2.8.1-§2.8.3, §2.8.5-§2.8.6 |
| `src/lifecycle.rs` | §2.11: rules, the transaction steps, `Recovery`, `RealmsRecovery` |
| `src/capabilities.rs` | §2.14.1-§2.14.4: secrets, mint, check, revoke, runtime resolution and identity |
| `src/directory.rs` | §2.6 in full |
| `src/leases.rs` | §2.12: `Hosting`, `HostingParts`, `HostingRuntime`, `HostingSettings`, `SweepStep`, renew, `end_hub`, observations, the monitor |
| `src/selection.rs` | §2.13 in full |
| `src/hosting.rs` | §2.16: the session routes, matchmaking, the admission and relay adapters, `RealmEventSink`, `HuntLifecycle`, `resolve_manifest_for_hunt` |
| `src/routes.rs`, `src/service.rs` | The domain dispatch (store spec §9.3-§9.4) and the service's domain answers (§10.3-§10.4) |
| `src/lib.rs` | Re-exports `resolve_manifest_for_hunt`, `HostingRuntime`, `RealmsRecovery` |
| record and repository files of part 1 | The plain `i64` in place of the old alias (records and repositories keep `i64` times) |
| `tests/common/kit.rs` (and its `mod` line in `common/mod.rs`) | The directory kit |
| `tests/realm_rules.rs`, `hub_rules.rs`, `lifecycle_rules.rs`, `tiers.rs`, `host_policy.rs`, `directory.rs`, `directory_presence.rs`, `capabilities_control.rs`, `leases.rs`, `hosting_bootstrap.rs`, `routes_dispatch.rs` | Domain §5.1, and the route dispatch with its services |

### 4. Ambiguities and the choices made

1. **Times** (errata): the public API takes `rifts_seam::UnixMillis` (`time`, `Clock`, `HostCapabilities::check`,
   `mint`, `revoke`, `resolve_runtime`, `hub_present_accounts`); stored records, repository arguments and the pure
   rules over records (`decide_join(.., now: i64)`, the hub tick, eligibility) keep `i64`, as the records do.
2. **Reserved realm names**: the brief forbids the four names `paradox`, `mysticparadox`, `mystic`, `mysticfox` in
   any string, so `RESERVED_REALM_NAMES` has 25 entries, not the spec's 29, and those four names are claimable. The
   maintainer should decide whether to restore them (or reserve them some other way).
3. **D3 renames** applied beyond part 1's: `RUSTIC_RIFTS_REALMS_MAX_MEMBERS`, `_MAX_BANS`, `_REALMS_HOSTING`; the ICE
   peer id is rifts-admission's derivation (label `rustic-ice-peer-v1`), so the spec's peer-id and hub-room vectors
   no longer hold; the tests pin the rollout bucket and the capability digest, which carry no label.
4. **Host artifacts**: the runtime feed's applicability (target changelist) gates only the manifest's values;
   Guard's roles and the known game exe hash apply regardless.
5. **Presence report**: missing or invalid `artifacts` refuse the report as malformed ("any missing").
6. **`HuntLifecycle::start_hub`'s node** is the runtime's `node` (`local` without one).
7. **`observe` steps 3-4** are checked together (generation and roster) before the terminal-state answer.
8. **`touch_presence`**: the fresh row is cached only when no write raced the touch.
9. **The route's realm `name` parameter** for leave and the player-invite answers is passed as written (§9.3 names no
   parser).
10. **`check_hub_entry`** re-reads the hub, and an entry it requests uses the effective peer id as the row's peer.
11. **Hub status counts**: `admittedPlayers` counts every ADMITTED row, the host's own included, so the end-to-end
    test sees 2 after one member is admitted (the spec's §5.1 summary says 1).
12. **`RealmEventSink` for `Hosting`** also invalidates the hub presence cache for every session a removal touched.
13. **Not written**: ICE signalling through rifts-relay's routes in `hosting_bootstrap` (the test admits through
    rifts-admission's service directly, not its routes); `Recovery` to a dedicated server stays isles'.
14. **Postgres** was not run (`REFORGED_TEST_POSTGRES_URL` unset).

### 5. Verification

- `cargo fmt --all --check`: clean. `cargo clippy --workspace --all-targets --locked -- -D warnings`: clean.
- `cargo test -p rifts-realms --locked --no-fail-fast` (SQLite): lib 42, `capabilities_control` 4, `directory` 11,
  `directory_presence` 3, `guard_adapter` 6, `host_policy` 7, `hosting_bootstrap` 1, `hub_repository` 20, `hub_rules`
  6, `leases` 4, `lifecycle_rules` 7, `realm_rules` 14, `realms_repository` 31, `routes_dispatch` 2, `stubbed` 2,
  `tiers` 5: **165 passed, 0 failed**.
- `cargo tree -e normal -p rifts-realms --depth 1`: unchanged from part 1 (no new dependency); every path dependency
  is an MIT crate of this workspace.

### 6. Seam changes

None beyond part 1's.

## API correction: times as `UnixMillis`

- **Clean-room worktree**: `R:\source\dauntless\dainty-lass\.claude\worktrees\agent-ab114e45c7269ccc2`, branch
  `fix/rd27-rifts-realms-times`, cut from `origin/feat/rd27-rifts-mit` at `614684f`.
- **Implementer**: one Claude Code subagent session (model Claude Opus 5.5), 2026-11-02, under the same brief, applying
  the spec writer's second public-API errata (`realms-api-errata-2.md` in the coordinator's scratchpad). It corrects
  errata 1: records no longer keep plain `i64` times.
- **Isolation**: as in parts 1 and 2. Read only this crate, `crates/rifts-seam/src/store.rs` (`UnixMillis`), the brief,
  the errata, `AGENTS.md` and this record.

### What changed

- Every time instant in the public API is `rifts_seam::UnixMillis`: the record fields the errata lists (hunt, realm,
  hub, `repo::hub`, `repo::hunts`), `HubTicketCheck`'s `presence` and `now`, `HostReliabilityRecord::new`, and every
  public function, repository method and `HuntTx` method parameter or return named `now`, `at`, `*_at`, `*_until`,
  `*_deadline`, `since` or `observed_after` (`entry_expires_at`, `presence_purge_at` and `realm_name_release_at`
  return one). Durations, ttls, counts, generations, sequences, ports and ids stay `i64`.
- Conversion is at the row boundary: the `QueryableByName` row structs keep `i64` (columns stay BIGINT) and wrap into
  `UnixMillis` when they become records; binds take `.0`. No SQL text changed.
- `HuntSession`, `Participant` and `HostPresenceRecord` keep their stored `detail` JSON byte for byte: the instants go
  through `time::millis` / `time::opt_millis`, which write and read the bare number. The JSON views still write ISO
  text through `time::iso`; `time::iso_at` (the `i64` shim) is gone.
- Crate-private `time::Later::saturating_add` moves an instant by a duration, so the existing saturating arithmetic
  reads as before; a difference of two instants is taken on `.0` and stays `i64`.
- The internal clocks (`directory`, `leases`, `capabilities::TimeSource`) return `UnixMillis`, and the two caches
  (realm presence, hub presence) store their read time as one.
- Tests: literals in time positions are wrapped (`UnixMillis(T0 + 10)`); the test constants stay `i64`. No assertion's
  value changed.

### Ambiguities

1. Internal helpers that parse client input (`selection`'s presence report, `leases`' observation) keep the parsed
   number as `i64` until it lands in a record; the errata covers the public API only.

### Verification

- `cargo fmt --all --check`: clean. `cargo clippy --workspace --all-targets --locked -- -D warnings`: clean.
- `cargo test -p rifts-realms --locked --no-fail-fast` (SQLite): lib 42 and the same 15 suites as part 2, per suite
  unchanged: **165 passed, 0 failed**. Postgres was not run (`REFORGED_TEST_POSTGRES_URL` unset).
- No dependency or seam change.
