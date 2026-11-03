# Clean-Room Provenance Record: `crates/rifts-guard` (RD27)

- **Component**: `crates/rifts-guard` (crate `rifts_guard`, `license = "MIT"`, `Copyright (c) 2026 Silent Spud`)
- **Behaviour specification**: `docs/port/spec-rd27-rifts-guard.md` with the umbrella `docs/port/spec-rd27-rifts.md`
  (its §2 D3 rename table overriding the part spec's old identifiers), at dainty-lass `4f9c09e` (branch
  `feat/rd27-rifts-seam`, which holds the specs and the MIT `rifts-seam`)
- **Clean-room worktree**: `R:\source\dauntless\dainty-lass\.claude\worktrees\agent-aef2d0e44cd5fd333` (branch
  `feat/rd27-rifts-guard`, cut from `origin/feat/rd27-rifts-seam` at `4f9c09e`)
- **Implementer**: one Claude Code subagent session (model Claude Opus 5.5), 2026-11-02, briefed by the RD27
  coordinator with the rules below

## 1. Isolation

### 1.1 Rules the implementer worked under

It never read, listed, grepped or `git show`ed anything under `dainty-restricted` (any worktree of it),
`modules/rustic-rifts`, `modules/rustic-isles`, Mystic Paradox's repository or libjuice's sources, and ran no
`git log -p` or `git show` on any commit. It searched nothing on the web. The session's environment listed a
dainty-restricted worktree as an additional working directory; no tool call read from it. Its inputs were the two
specs, the other rifts part specs only where they name `rifts_guard` items (a grep for `rifts_guard`, `SqlGuardStore`,
`block_on_store`, `store_actor`, `SQLITE_MIGRATIONS`), and dainty-lass's MIT code.

### 1.2 Worktree

A full checkout of dainty-lass at `4f9c09e`. dainty-lass holds no Guard source at that revision:
`crates/rifts-guard` did not exist until the implementer created it.

## 2. Files read

- `docs/port/spec-rd27-rifts-guard.md` (whole), `docs/port/spec-rd27-rifts.md` (whole)
- `docs/port/spec-rd27-rifts-realms.md` §2.14.6 and `docs/port/spec-rd27-rifts-realms-store.md` §3.3 and the lines
  naming Guard items (how realms calls `SCHEMA`, `SqlGuardStore`, `block_on_store`, `store_actor`)
- `docs/port/clean-room/chat.md` (§1-§3), `docs/port/clean-room/rifts-seam.md` (for this record's format)
- `docs/security/content.md` (the checklist's structure)
- `AGENTS.md`, `Cargo.toml`, `crates/service-kit/Cargo.toml` (the `preserve_order` line), `crates/chat/Cargo.toml`
- `crates/rifts-seam/src/*.rs` (whole: the seam's types and traits)
- `crates/store/src/{lib.rs, database.rs, portable.rs, engine.rs, connection.rs}`, `crates/store/src/history.rs`
  (`Actor`, its check, the raw-SQL loading pattern), `crates/store/Cargo.toml`
- `crates/chat/src/history.rs` (lines 1-140 and its `sql_query` binds: how a crate embeds its two migration sets),
  `crates/network/src/home/ticket.rs` (one `QueryableByName` read), `crates/admin/tests/guard.rs` (lines 1-80: the
  `REFORGED_TEST_POSTGRES_URL` pattern)
- `fuzz/Cargo.toml`, `fuzz/fuzz_targets/content_news.rs` (a target's shape)
- The brief (`impl-brief.md` in the coordinator's scratchpad)

## 3. Files written

| File | What |
| --- | --- |
| `crates/rifts-guard/Cargo.toml` | Manifest, `license = "MIT"`, `serde_json` with `preserve_order` |
| `crates/rifts-guard/migrations/{sqlite,postgres}/2026-10-26-000002_rifts_guard/{up,down}.sql` | The DDL of §4, byte for byte as the spec gives it (106 and 7 lines; the two `up.sql` differ only in the surrogate keys) |
| `crates/rifts-guard/src/lib.rs` | `Guard`, `GuardParts`, `Stub`, `HuntVerdict`, `RewardPolicyStatus`, the five traits' impls, the evidence flows (§3.5), host writes and inventory hooks (§3.8-§3.9), the bucket cache (§3.7), re-exports |
| `crates/rifts-guard/src/json.rs` | §3.0 text rules, §3.4 parser |
| `crates/rifts-guard/src/digest.rs` | SHA-256 hex |
| `crates/rifts-guard/src/errors.rs` | §3.2 |
| `crates/rifts-guard/src/config.rs` | §3.1 |
| `crates/rifts-guard/src/evidence.rs` | §3.5.2-§3.5.5 readers, chain application, replies |
| `crates/rifts-guard/src/census.rs` | §3.12 |
| `crates/rifts-guard/src/traffic.rs` | §3.7 limits, budget, local limiter, reservations |
| `crates/rifts-guard/src/writes.rs` | §3.8 and the §3.9 denial functions |
| `crates/rifts-guard/src/fence.rs` | §3.9's `InventoryFence` |
| `crates/rifts-guard/src/grants.rs` | §3.10 |
| `crates/rifts-guard/src/operations.rs` | §3.11 |
| `crates/rifts-guard/src/routes.rs` | §3.3 |
| `crates/rifts-guard/src/ports.rs` | §3.14, §2.3 |
| `crates/rifts-guard/src/store.rs` | §3.15 records, `GuardStore`, memory and stub stores, `TABLES` |
| `crates/rifts-guard/src/sql.rs` | §3.15 `SqlGuardStore`, `block_on_store`, `store_actor`, `SCHEMA` |
| `crates/rifts-guard/src/testkit.rs`, `src/tests.rs` | §6.2 fixtures and tests |
| `crates/rifts-guard/tests/sql_store.rs` | §6.1, rewritten from the spec's description (the A file was not seen) |
| `fuzz/fuzz_targets/guard_evidence.rs`, `guard_grants.rs` | §6.3 |
| `fuzz/Cargo.toml`, `fuzz/Cargo.lock` | The two `[[bin]]` entries and the `rifts-guard`/`rifts-seam` path dependencies |
| `Cargo.toml`, `Cargo.lock` | Seam change: `rifts-guard = { path = "crates/rifts-guard" }` in `[workspace.dependencies]` |
| `docs/security/rifts-guard.md` | The fresh checklist (umbrella §4.5) |
| `docs/port/clean-room/rifts-guard.md` | This record |

Every `.rs` file starts with `// SPDX-License-Identifier: MIT` and `// Copyright (c) 2026 Silent Spud`.

## 4. Verification

- `cargo fmt --all --check`: clean. The fuzz targets were formatted with `rustfmt --edition 2024 --config
  max_width=120`.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: clean (after `Cargo.lock` took the new member).
- `cargo test -p rifts-guard --locked --no-fail-fast`: 105 unit tests and 6 `sql_store` tests passed, 0 failed. The
  two Postgres tests ran their skip path: no `REFORGED_TEST_POSTGRES_URL` on this machine. The SQLite store suite runs
  from sync code, a multi-thread runtime and a current-thread runtime (the three `block_on_store` paths).
- `cargo +nightly check --manifest-path fuzz/Cargo.toml --bin guard_evidence --bin guard_grants`: builds.

`cargo tree -e normal -p rifts-guard` (116 lines; first level, then the workspace crates in the whole tree):

```text
rifts-guard v0.0.0 (crates\rifts-guard)
├── diesel v2.3.14
├── diesel_migrations v2.3.2
├── rifts-seam v0.0.0 (crates\rifts-seam)
├── serde_json v1.0.154
├── sha2 v0.11.1
├── store v0.0.0 (crates\store)
├── thiserror v2.0.21
└── tokio v1.54.1

Workspace crates anywhere in the tree: module-api v0.1.0, rifts-seam v0.0.0, store v0.0.0 (all license = "MIT")
```

No AGPL crate.

## 5. Ambiguities and the assumptions taken

1. **D3 renames.** Routes are `/rustic/v1/hunts/:sessionId/...`; the header is the seam's `x-rustic-host-capability`;
   the settings are `RUSTIC_RIFTS_HOST_POLICY`, `RUSTIC_RIFTS_HUNT_EVIDENCE` and `RUSTIC_RIFTS_HOST_ROUTE_CENSUS`, and
   the startup lines print those names. The `P2P_*` keys carry no old name and are kept. The handler ids are
   `guard.event_batches`, `guard.completion`, `guard.reward_self`, `guard.shadow_comparisons`, `guard.reward_rollout`
   and `guard.alert_ack`; restricted's stub tests (realms `stubbed.rs`, isles `module.rs`) must expect these.
2. **The pinned hash.** §9 risk 1 asks for a body's hash computed on the AGPL crate. The clean room cannot run it, so
   `tests::known_body_hashes_to_the_pinned_value` pins `5781c8e2f3426ae61cc86d0114d32b033f0360e42610f378a84932dc9bc238fb`,
   computed independently (Python's `hashlib`) from §3.5.2's definition over the compact text of the body in the test.
   **The reviewer outside the clean room must confirm the AGPL crate gives the same value for that body**; if not, the
   rewrite's hash differs and chains do not survive the cutover (they do not anyway under D3, but acceptance §8.4 asks).
3. **`SQLITE_MIGRATIONS`/`POSTGRES_MIGRATIONS`** are `EmbeddedMigrations` constants (as realms' store spec declares
   its own); `SCHEMA` wraps them in closures.
4. **Record field types** the spec leaves open: times are `UnixMillis`, counts and sequences `u64`, `minutes` `u32`,
   `HostWriteRecord::kind` a `HostWriteKind`, `HostWriteRecord::host` an `AccountId` that is empty when the hunt has
   none (the column is `NOT NULL`), `TrafficWindowRecord::scope` a `TrafficScope`. Restricted's tests that build these
   records as literals need checking against them.
5. **Helper return types**: `MemoryGuardStore`'s `host_writes()`, `windows()`, `alert()` are infallible;
   `SqlGuardStore`'s return `Result<_, StoreError>`; `xp_summary` returns `Result<Option<XpSummaryRecord>, _>`.
6. **`json` helpers' signatures** (unused outside): `js_string(&str) -> String`, `js_number(f64) -> String`,
   `stringify(&Value) -> String`, `stringified_len(Option<&Value>) -> usize`, `safe_integer(&Value) -> Option<i64>`,
   `js_length(&str) -> usize`, `js_slice(&str, usize) -> &str`, `js_trim(&str) -> &str`,
   `js_to_number(&Value) -> f64`, `js_safe_integer(f64) -> Option<i64>`.
7. **`CHALLENGE_REWARD_CURRENCIES`** is `["CURRENCY_SEASONAL_COIN"]` and **`MAX_CHALLENGE_REWARD_PER_CLAIM`** is
   1000, the claim-funded coin rule's id and bound; the spec names the constants without values.
8. **"No quantity"** for a starter cosmetic is a `quantity` that is not a JSON number (absent included), since
   quantities are read only as numbers.
9. **Stacked items read as `Ok(None)`** (nothing stored) is a balance of 0, as `stacked_balance_of(None)` gives; only
   a failed read or an unreadable list is no balance.
10. **Least recently touched** in the local limiter is ordered by a touch counter, so keys touched at the same
    millisecond keep their order.
11. **A refused `reserve_traffic`** still keeps its purge of expired windows; neither window is changed.
12. **Hub presence** is read for the stored hunt's generation.
13. **A store error inside a hook** becomes the seam's `INTERNAL` refusal with no log line (the spec names none
    there); the routes log theirs as §3.3 says.
14. **`submit_parsed` with no chain row** starts from `ChainRecord::new(session, generation, None)` at revision 0, as
    §3.5.4 says; a store error while writing a refusal's finding propagates.
15. **The fence's tags and lines** use the runtime's session and generation (equal to the hunt's once allowed).
16. **`iso_time`** writes a four-digit (zero-padded) year; JavaScript's extended `±YYYYYY` years outside 0-9999 are
    not reproduced.
17. **`SystemClock::uuid`** draws from std's randomly keyed hasher, so the crate needs no `rand`.
18. **The checklist's name** is `docs/security/rifts-guard.md` (the umbrella's §4.5) rather than the part spec's
    `guard.md`.
19. **`Unhandled P2P route error`** and the `TABLES.node_collection` strings carry no old name and are kept as given.
20. **Diesel's `32-column-tables`** is not needed: the chain row is written with a boxed raw query and read with
    `QueryableByName`.

## 6. Similarity

Not measured here; the reviewer outside the clean room measures each file against restricted's `rifts-guard` at
`4efc8b5` (umbrella §5 item 5).

## 7. Integration

Seam changes made on this branch, for the integration pass to review:

- `[workspace.dependencies]`: `rifts-guard = { path = "crates/rifts-guard" }`; `Cargo.lock` takes the member.
- `fuzz/Cargo.toml`: `rifts-guard` and `rifts-seam` path dependencies and the `guard_evidence` and `guard_grants`
  `[[bin]]` entries; `fuzz/Cargo.lock` follows.
- No new crates.io dependency; `serde_json`'s `preserve_order` is turned on in this crate's manifest only (the
  workspace already unifies it through `service-kit`).

Left for the integration pass: the migration guide's §2 line for the crate, the NOTICE line, the `SECURITY_REVIEW.md`
index entry for `docs/security/rifts-guard.md`, hostkit's `RUSTIC_RIFTS_` server prefix and the `MYSTICPARADOX_*`
moves (umbrella D3), and dainty-restricted's switch of its `rifts-guard` dependency to this repository by git rev.

## 8. Restructure pass

2026-11-02, one Claude Code subagent session (model Claude Opus 5.5) under the isolation of section 1, on branch
`chore/rd27-rifts-restructure-seam-guard-host` cut from `origin/feat/rd27-rifts-mit`. No AGPL tree was read; the
only comparison run was of the crate against its own previous revision on this branch, as a proxy.

| File | What changed |
| --- | --- |
| `src/lib.rs` | `Guard` keeps its `GuardParts`; Guard's own refusals in one `Refuse` table; the batch loop split into `try_batch`, the traffic charge into `charge_local` and `charge_shared`, the bucket cache into lookup and store helpers, the inventory check into `draft_premium` and `grant_check`; hook futures still run on first poll; the `Stub` impls from one `unwritten!` macro over the method lists |
| `src/errors.rs` | `Display`, `Error` and `From` by hand (same texts); `thiserror` is no longer a dependency |
| `src/digest.rs` | Hex from a nibble table; test vectors as a table |
| `src/routes.rs` | Routes built by one `const fn`; the route refusals as a `Gate` table; internal faults hidden in one helper |
| `src/store.rs` | Each stored enum's text both ways from one `stored_as!` list; tables listed through `table` and `swept`; the memory store's windows and batches through helpers; `StubGuardStore` from one `no_database!` macro |
| `src/ports.rs` | UUID bits set on a `u128`; `iso_time` by 400-year cycles and month lengths instead of the days-to-civil formula; both logs from one `levelled_log!` macro; the policy snapshot as a method |
| `src/config.rs` | Each resolver as a default plus one change; startup lines from (source, effect) pairs; tier 1 gates as an ordered list |
| `src/writes.rs` | Write kind names from one table; the hub kind bars as lists; shared membership helpers; one envelope builder |
| `src/evidence.rs` | Event parsing in `parse_event`; the chain gate in `batch_gate`; findings through a `Findings` collector; the encounter move as one function; replies from `concat!` templates. The canonical text and the batch hash are computed as before (the pinned-hash test passes unchanged) |
| `src/fence.rs` | The draft token's tags, the presence read and the remembered decision in helpers |
| `src/sql.rs` | One `Param` bind list and one `query` builder replace the hand-spelled `.bind` chains; every table's column list drives its INSERT, UPDATE and SELECT text, which is byte-identical to before; the row structs from one `rows!` macro |
| `src/census.rs`, `src/json.rs`, `src/traffic.rs`, `src/grants.rs` | JSON written from field lists; status classes from a table; `js_number`'s layout, the escaper and the number parser rewritten; in-flight counters as a struct; classification and exemptions as ordered rules |

Public API and behaviour: no public item's name, type, fields, field order, variants or signature changed (trait
methods of `GuardStore` are listed in a different order). Wire bytes, the SQL text sent to the database, JSON field
order, error codes and texts and log lines are unchanged. `cargo test -p rifts-guard --locked --no-fail-fast`: 111
passed (105 unit, 6 SQLite store), as before; the Postgres half still needs `REFORGED_TEST_POSTGRES_URL`.

What cannot go lower: the public records and enums of `store.rs`, `ports.rs`, `config.rs` and `lib.rs`, the
`ERROR_STATUSES` table and the item id lists in `grants.rs` are fixed by the spec, so their lines stay as they were.

