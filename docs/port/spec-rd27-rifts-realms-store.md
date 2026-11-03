<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false, "code_blocks": false } } -->

# Clean-Room Behaviour Specification: `rifts-realms`, persistence and wiring (RD27)

> **Scope and purpose (RD27).** This is one of two part specs for the crate `rifts-realms` (umbrella:
> [`spec-rd27-rifts.md`](spec-rd27-rifts.md)). It covers the persistence and wiring half: the manifest, the crate root,
> the database handle, the three repositories, the route dispatcher, the service that answers `rifts-seam`'s hooks,
> the ports, the stub, both migration sets and their tests. The domain half (realm, directory, hub, hunt, lifecycle,
> leases, selection, tiers, host policy, capabilities, control, input, ids, time, hosting) is
> [`spec-rd27-rifts-realms.md`](spec-rd27-rifts-realms.md). This file states no rule that one owns: where a repository
> applies a domain rule (a presence transition table, a hosting patch), it names the rule and says when it is applied.
>
> Everything here is stated from contracts: the SQL schema (given exactly, it is a contract), the repository
> methods' preconditions, atomicity and returned values, the route table, the JSON the service writes, the public Rust
> API that callers outside the crate use, and the tests. No implementation code is reproduced and no private helper
> is named.
>
> Source: dainty-restricted `4efc8b5` (`plan`), `modules/rustic-rifts/crates/rifts-realms`; dainty-lass `49197a8`
> for `crates/store`.

---

## 1. Provenance and classification

### 1.1 Evidence

- rustic-rifts' `NOTICE.md` (lines 16-28): the Rust workspace ports Mystic Paradox's `p2p/`, `realms/`, Guard and
  launcher realm-hosting modules. Every source file here opens with a `//!` line naming the Node module it ports
  (`realms/persistence/RealmsRepository.ts`, `p2p/persistence/contracts/RealmHubRepository.ts`, Node's
  `P2PHuntRepository`, `p2pErrors.ts`) or the RF/RD row that built it.
- The tests name the Node contract suites they port (`realmsRepositoryContract.ts`,
  `realmPresenceRepositoryContract.ts`, `realmHubRepositoryContract.ts`, `realmsTestKit.ts`).
- The migrations are DaintyLass's own tables, but their shape is Node's collections flattened (RF1 section 7).

With that, nothing in this half is Category A. Every file is rewritten from this document.

### 1.2 Files

| Path (in `crates/rifts-realms`) | Lines | Cat. | What it is |
| --- | ---: | :-: | --- |
| `Cargo.toml` | 37 | B | Manifest (§8) |
| `src/lib.rs` | 189 | B | Module list, re-exports, `RealmsError`, `TABLES` (§2.2, §2.3) |
| `src/db.rs` | 139 | B | `RealmsDb`, migrations, `SCHEMA`, `RepoError`, actor mapping (§3) |
| `src/repo/mod.rs` | 7 | B | The repository modules and one re-export |
| `src/repo/realms.rs` | 1240 | B | `RealmsRepository`, `RealmPresenceRepository` (§5) |
| `src/repo/hub.rs` | 850 | B | `RealmHubRepository` (§6) |
| `src/repo/hunts.rs` | 1760 | B | `HuntRepository`, `HuntTx` (§7) |
| `src/routes.rs` | 856 | B | `ROUTES`, `RealmsRoutes`, `stub_routes` (§9) |
| `src/service.rs` | 395 | B | `RealmsService`, `realms_enabled_or_default` (§10) |
| `src/ports.rs` | 325 | B | Clock, ids, rate limiter, accounts, events, log, Guard and operations views (§11) |
| `src/stub.rs` | 199 | B | `Stub` (§12) |
| `migrations/sqlite/2026-10-26-000001_rifts_realms/up.sql` | 272 | B | Schema (§4); DDL carried as the contract |
| `migrations/sqlite/2026-10-26-000001_rifts_realms/down.sql` | 19 | B | Drops (§4.4) |
| `migrations/postgres/2026-10-26-000001_rifts_realms/up.sql` | 272 | B | Schema, Postgres (§4) |
| `migrations/postgres/2026-10-26-000001_rifts_realms/down.sql` | 19 | B | Drops (§4.4) |
| `tests/common/mod.rs` | 201 | B | Two-engine targets, the Postgres switch, fixtures (§13.1) |
| `tests/common/kit.rs` | 255 | B | The directory test kit (§13.1) |
| `tests/realms_repository.rs` | 1354 | B | Realms and presence repository contract (§13.2) |
| `tests/hub_repository.rs` | 971 | B | Hub presence repository contract (§13.2) |
| `tests/guard_adapter.rs` | 430 | B | `RealmsHuntSessions` and `SqlGuardStore` over `RealmsDb` (§13.2) |
| `tests/stubbed.rs` | 74 | B | Every rifts crate stubbed: 60 routes, 501 (§13.2) |

**Totals:** 21 files, 9,864 lines. A 0, B 21, D 0, G 0.

The migrations are B but their DDL is reproduced in §4 as a contract: a server that ran the AGPL crate must open
its database with the MIT one, so the MIT migrations have the same versions, names, tables, columns, types,
constraints and indexes. Only the SQL comments are prose and are written fresh.

No test in this half depends on a D oracle. All six test files are rewritten from §13; none stays in restricted.

---

## 2. The crate root

### 2.1 Modules

The crate has public modules `capabilities`, `control`, `db`, `directory`, `host_policy`, `hub`, `hunt`, `ids`,
`input`, `leases`, `lifecycle`, `ports`, `realm`, `repo`, `selection`, `service`, `tiers`, `time`; private modules
`hosting`, `routes`, `stub`. `repo` has public submodules `hub`, `hunts`, `realms` and re-exports
`rifts_guard::SqlGuardStore`.

### 2.2 Re-exports at the root (public API)

`RealmsHuntSessions` (from `capabilities`), `HostBootstrap`, `HuntLifecycle`, `HuntStatus`, `Observation`,
`StartHub`, `resolve_manifest_for_hunt` (from the private `hosting`), `HostingRuntime` (from `leases`),
`RealmsRecovery` (from `lifecycle`), `CoreAccounts`, `CoreGuardSessions`, `CoreOperationsView` (from `ports`),
`SqlGuardStore` (from `rifts_guard`), `ROUTES`, `RealmsRoutes`, `RealmsRoutesParts`, `stub_routes` (from `routes`),
`RealmsService`, `RealmsServiceParts`, `realms_enabled_or_default` (from `service`), `Stub`.

### 2.3 `RealmsError`

A public enum, `Debug, Clone, PartialEq, Eq`, `thiserror::Error`, `#[non_exhaustive]`:

| Variant | Display | Meaning |
| --- | --- | --- |
| `Unimplemented(rifts_seam::Unimplemented)` | transparent | The service behind the route is absent (`From` impl) |
| `Refused(rifts_seam::Refusal)` | transparent | A launcher error code (`REALM_*`, `P2P_*`, `AUTH_*`) (`From` impl) |
| `Database(String)` | the string | The database failed; the text is never shown to a client (§9.5) |

Conversions into `RealmsError::Database`: from `RepoError` (its display text), from `store::Error` (its display
text), from `diesel::result::Error` (`"database: "` followed by the error's display text).

### 2.4 `TABLES`

`pub const TABLES: &[rifts_seam::TableSpec]`, every entry with `owner: Component::Realms`. Order matters: it is the
`CREATE TABLE` order of §4 (a unit test checks both migration sets against it). `expires_by` is `None` unless given.

| `name` | `node_collection` | `keys` | `expires_by` |
| --- | --- | --- | --- |
| `rifts_realms` | `realms` | `realm; name unique; host unique` | — |
| `rifts_realm_members` | `realms.members` | `realm + account` | — |
| `rifts_realm_bans` | `realms.bans` | `realm + account` | — |
| `rifts_realm_join_requests` | `realms.joinRequests` | `realm + account` | — |
| `rifts_realm_player_invites` | `realms.playerInvites` | `realm + account` | — |
| `rifts_realm_hub_rooms` | `realms.hubRoomIds` | `room unique` | — |
| `rifts_realm_invites` | `realmInvites` | `code; realm + created_at` | `purge_at` |
| `rifts_realm_presence` | `realmPresence` | `account; realm + expires_at` | `expires_at` |
| `rifts_hunt_sessions` | `p2pHuntSessions` | `session; global id unique; active party key unique; state` | — |
| `rifts_hunt_roster` | `p2pHuntSessions.roster` | `session + account` | — |
| `rifts_hunt_transitions` | `p2pHuntTransitions` | `transition; session + at + state_version` | — |
| `rifts_host_capabilities` | `p2pHostCapabilities` | `capability; secret digest unique; session + generation` | — |
| `rifts_host_presences` | `p2pHostPresences` | `account` | `expires_at` |
| `rifts_host_offers` | `p2pHostOffers` | `offer; session + generation + rank; one accepted` | — |
| `rifts_host_reliability` | `p2pHostReliability` | `account` | — |
| `rifts_participant_observations` | `p2pParticipantObservations` | `observation; session + generation + at` | — |
| `rifts_control_sequences` | `p2pControlSequences` | `account` | — |
| `rifts_control_messages` | `p2pControlMessages` | `message; account + sequence unique` | `expires_at` |
| `rifts_hub_presence` | `realmHubPresence` | `session + account; account / realm + state` | `purge_at` |

Node's `realmHubGates` collection has no table: it serialised hub entries on MongoDB, and `store` serialises writes.

Unit tests in the root: every collection in `realms, realmInvites, realmPresence, p2pHuntSessions,
p2pHostCapabilities, p2pControlSequences, p2pControlMessages, p2pHostPresences, p2pHostOffers, p2pHostReliability,
p2pParticipantObservations, p2pHuntTransitions, realmHubPresence` has an entry; and the table names created by each
`up.sql`, in file order, equal `TABLES`' names in order.

---

## 3. The database handle (`db`)

### 3.1 What `store` gives (dainty-lass `crates/store`)

The crate uses only this much of `store`:

| Item | Use |
| --- | --- |
| `Database::open(&DatabaseConfig, &DatabaseOptions, &DatabaseSchema) -> Result<Database, store::Error>` | Blocking. Opens or creates the database, applies the schema's pending migrations, puts the change-log triggers on. Refuses when the SQLite and Postgres sets' versions differ |
| `DatabaseSchema { sqlite: fn() -> EmbeddedMigrations, postgres: fn() -> EmbeddedMigrations, soft_delete: &'static [&'static str] }` | One per component |
| `Database::read(f)` | `f(&mut AnyConnection)` in a read-only transaction on the read pool (one snapshot on SQLite; read committed per statement on Postgres) |
| `Database::write(actor: store::Actor, f)` | `f(&mut Tx)` as one write transaction. Writes are serial on both engines: SQLite has one writer connection; Postgres takes a transaction-level advisory write lock (`pg_advisory_xact_lock`) at the start of every write. Every changed row is logged under the actor (DD23). An `Err` rolls the transaction back, log entries included |
| `Tx::conn() -> &mut AnyConnection` | The connection inside the write |
| `store::Actor::{Player(String), Admin(String), Host(String), System(String)}` | The change log's actor, stored as `player:<id>`, `admin:<key>`, `host:<cmd>`, `system:<task>` |
| `store::portable::numbered(sql)` | Turns `?` placeholders into `$1..$n` for both engines |
| `store::portable::upsert(table, columns, conflict, update)` | `INSERT .. ON CONFLICT (conflict) DO UPDATE SET c = excluded.c`, binds in `columns` order |
| `store::portable::quote_ident` | Tests: quoting a database name |
| `AnyConnection`, `MultiBackend`, `DatabaseConfig`, `DatabaseOptions`, `PostgresConfig`, `store::Error` | Types |

Closures passed to `read` and `write` must be `Send + 'static` and return `Result<T, E>` with
`E: From<store::Error> + From<diesel::result::Error>`.

### 3.2 Migrations and the schema

- `pub const SQLITE_MIGRATIONS: EmbeddedMigrations`, embedding `migrations/sqlite`.
- `pub const POSTGRES_MIGRATIONS: EmbeddedMigrations`, embedding `migrations/postgres`.
- `pub const SCHEMA: DatabaseSchema` with those two and `soft_delete: &[]` (no realms table is player-facing in
  DD23's sense, so none is soft-deleted).
- One migration per engine, version and folder name `2026-10-26-000001_rifts_realms`. Guard's is
  `2026-10-26-000002_rifts_guard` (spec-rd27-rifts-guard.md). Both live in one database and one
  `__diesel_schema_migrations` table; dated versions keep them from clashing with isles' own.

### 3.3 `RealmsDb`

`pub struct RealmsDb`, `Clone` (cheap; shares the pools). Holds a `store::Database` and a `module_api::NodeId`.

| Item | Contract |
| --- | --- |
| `fn open(config: &DatabaseConfig, options: &DatabaseOptions, node: NodeId) -> Result<RealmsDb, RepoError>` | Blocking. First opens the database with `rifts_guard::SCHEMA` (applying Guard's migration) and drops that handle, propagating its error; then opens it with `SCHEMA` and keeps that handle. Order matters only in that both sets end up applied; opening a database where either set already ran succeeds and keeps the data |
| `fn new(db: Database, node: NodeId) -> RealmsDb` | Over a database whose schema already holds realms' migration |
| `fn database(&self) -> &Database` | The handle (isles builds `SqlGuardStore::new(db.database().clone())`) |
| `fn node(&self) -> &NodeId` | The node that owns sessions this server creates (module-api global ids, DD18) |
| `fn guard_store(&self) -> rifts_guard::SqlGuardStore` | `SqlGuardStore::new` over a clone of the handle |

Internally the crate needs a read helper and a write helper over the handle; the write helper takes a
`rifts_seam::Actor` and converts it with `store_actor`.

`pub fn store_actor(actor: &rifts_seam::Actor) -> store::Actor`:

| `rifts_seam::Actor` | `store::Actor` |
| --- | --- |
| `Account(account)` | `Player(account id)` |
| `HostRuntime { host, .. }` | `Player(host account id)`: a host runtime acts with its host's capability, so its writes are that player's to revert (`revert --actor player:<host>`) |
| `Admin { key_id }` | `Admin(key_id)` |
| `System(task)` | `System(task)` |

Repositories write SQL with `?` placeholders and pass it through `portable::numbered` before binding, so one text
serves both engines. A numbering failure becomes `RepoError::Corrupt` with the message `numbered` returned.

### 3.4 `RepoError`

`pub enum RepoError`, `Debug`, `thiserror::Error`:

| Variant | Display |
| --- | --- |
| `Store(store::Error)` | transparent (`From`) |
| `Query(diesel::result::Error)` | `database: {error}` (`From`) |
| `Duplicate(&'static str)` | `duplicate key: {key}`: a unique key is taken (Node's E11000) |
| `Corrupt(String)` | `realms table: {text}`: a row the code can't read back (unknown enum text, bad JSON) |

A corrupt enum value is reported as `Corrupt("<what> <value in Rust Debug quoting>")`, for example
`realms table: hub state "BOGUS"`. The `<what>` labels used: `hub state`, `member role`, `join policy`,
`hub presence state`, `admission state`, `state`, `capability scope`, `offer status`. A JSON failure on a hunt's
`detail` reads `hunt <session_id>: <serde error>`; other JSON failures carry serde's text alone.

The `Duplicate` keys are exactly `session_id`, `active_party_key`, `secret_digest`, `accepted offer`.

---

## 4. The schema (both engines)

### 4.1 Conventions

- Every time is Unix milliseconds in `BIGINT`. Every enum is its upper-case wire name in `TEXT` (the names are the
  domain spec's: `INVITE_ONLY`, `PUBLIC`, `SOLO`; `HOST`, `MEMBER`; `NONE`, `STARTING`, `RUNNING`, `STOPPING`; the
  hunt states; `PENDING`, `ISSUED`, `ADMITTED`, `LOST` for roster admission; `PENDING`, `TICKETED`, `ADMITTED`,
  `LOST`, `LEFT`, `REMOVED`, `EXPIRED` for hub presence; `OFFERED`, `ACCEPTED`, `DECLINED`, `EXPIRED`, `CANCELLED`).
- No column has a `DEFAULT`; the code writes every column on insert.
- Node's arrays inside a realm or a hunt are child tables with a `seq` column holding their order. A hunt's fields
  nothing queries by stay in its `detail` JSON.
- Foreign keys exist only from the realm child tables (members, bans, join requests, player invites, hub rooms) and
  from the roster. Invites, presence and every p2p table take rows for any realm or session id (Node stores them so).
- The two `up.sql` files are identical but for one line: on Postgres, `rifts_realms.name` is declared
  `TEXT COLLATE "C" NOT NULL UNIQUE`, so the name's uniqueness and every `ORDER BY name` compare bytes, as SQLite's
  default `BINARY` collation does.

### 4.2 DDL (SQLite; Postgres differs only where §4.1 says)

```sql
CREATE TABLE rifts_realms (
    realm_id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    host_account_id TEXT NOT NULL UNIQUE,
    host_public_key TEXT NOT NULL,
    join_policy TEXT NOT NULL,
    listed BOOLEAN NOT NULL,
    created_at BIGINT NOT NULL,
    last_online_at BIGINT,
    online_until BIGINT,
    version TEXT,
    offline_notified_for BIGINT,
    hosting_max_servers BIGINT,
    hosting_max_players BIGINT,
    hosting_afk_seconds BIGINT,
    hosting_hub_session_id TEXT,
    hosting_hub_state TEXT,
    hosting_training_state TEXT,
    hosting_active_servers BIGINT,
    hosting_updated_at BIGINT
);
CREATE INDEX rifts_realms_listed ON rifts_realms (listed, online_until);

CREATE TABLE rifts_realm_members (
    realm_id TEXT NOT NULL REFERENCES rifts_realms (realm_id),
    account_id TEXT NOT NULL,
    role TEXT NOT NULL,
    joined_at BIGINT NOT NULL,
    seq BIGINT NOT NULL,
    PRIMARY KEY (realm_id, account_id)
);
CREATE INDEX rifts_realm_members_account ON rifts_realm_members (account_id);

CREATE TABLE rifts_realm_bans (
    realm_id TEXT NOT NULL REFERENCES rifts_realms (realm_id),
    account_id TEXT NOT NULL,
    reason TEXT,
    at BIGINT NOT NULL,
    seq BIGINT NOT NULL,
    PRIMARY KEY (realm_id, account_id)
);

CREATE TABLE rifts_realm_join_requests (
    realm_id TEXT NOT NULL REFERENCES rifts_realms (realm_id),
    account_id TEXT NOT NULL,
    at BIGINT NOT NULL,
    seq BIGINT NOT NULL,
    PRIMARY KEY (realm_id, account_id)
);

CREATE TABLE rifts_realm_player_invites (
    realm_id TEXT NOT NULL REFERENCES rifts_realms (realm_id),
    account_id TEXT NOT NULL,
    at BIGINT NOT NULL,
    seq BIGINT NOT NULL,
    PRIMARY KEY (realm_id, account_id)
);
CREATE INDEX rifts_realm_player_invites_account ON rifts_realm_player_invites (account_id);

CREATE TABLE rifts_realm_hub_rooms (
    room_id TEXT PRIMARY KEY,
    realm_id TEXT NOT NULL REFERENCES rifts_realms (realm_id),
    seq BIGINT NOT NULL
);
CREATE INDEX rifts_realm_hub_rooms_realm ON rifts_realm_hub_rooms (realm_id);

CREATE TABLE rifts_realm_invites (
    code TEXT PRIMARY KEY,
    realm_id TEXT NOT NULL,
    created_by TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    max_uses BIGINT NOT NULL,
    uses BIGINT NOT NULL,
    revoked_at BIGINT,
    purge_at BIGINT NOT NULL
);
CREATE INDEX rifts_realm_invites_realm ON rifts_realm_invites (realm_id, created_at);
CREATE INDEX rifts_realm_invites_purge ON rifts_realm_invites (purge_at);

CREATE TABLE rifts_realm_presence (
    account_id TEXT PRIMARY KEY,
    realm_id TEXT NOT NULL,
    bound_at BIGINT NOT NULL,
    last_seen_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL
);
CREATE INDEX rifts_realm_presence_realm ON rifts_realm_presence (realm_id, expires_at);
CREATE INDEX rifts_realm_presence_expires ON rifts_realm_presence (expires_at);

CREATE TABLE rifts_hunt_sessions (
    session_id TEXT PRIMARY KEY,
    global_id TEXT UNIQUE,
    generation BIGINT NOT NULL,
    state TEXT NOT NULL,
    state_version BIGINT NOT NULL,
    party_id TEXT NOT NULL,
    active_party_key TEXT UNIQUE,
    host_account_id TEXT NOT NULL,
    session_kind TEXT,
    hub_kind TEXT,
    realm_id TEXT,
    lease_expires_at BIGINT,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    detail TEXT NOT NULL
);
CREATE INDEX rifts_hunt_sessions_state ON rifts_hunt_sessions (state, created_at);
CREATE INDEX rifts_hunt_sessions_realm ON rifts_hunt_sessions (realm_id, state);
CREATE INDEX rifts_hunt_sessions_host ON rifts_hunt_sessions (host_account_id, state);

CREATE TABLE rifts_hunt_roster (
    session_id TEXT NOT NULL REFERENCES rifts_hunt_sessions (session_id),
    account_id TEXT NOT NULL,
    seq BIGINT NOT NULL,
    steam_id TEXT NOT NULL,
    candidate_id TEXT NOT NULL,
    reconnect_epoch BIGINT NOT NULL,
    admission_state TEXT NOT NULL,
    local_bridge_instance_id TEXT,
    local_bridge_port BIGINT,
    local_bridge_ready_at BIGINT,
    admitted_at BIGINT,
    disconnected_at BIGINT,
    rejoin_deadline BIGINT,
    returned_to_hub_at BIGINT,
    PRIMARY KEY (session_id, account_id)
);
CREATE INDEX rifts_hunt_roster_account ON rifts_hunt_roster (account_id);

CREATE TABLE rifts_hunt_transitions (
    transition_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    generation BIGINT NOT NULL,
    from_state TEXT NOT NULL,
    to_state TEXT NOT NULL,
    state_version BIGINT NOT NULL,
    reason TEXT NOT NULL,
    actor_type TEXT NOT NULL,
    actor_account_id TEXT,
    at BIGINT NOT NULL
);
CREATE INDEX rifts_hunt_transitions_session ON rifts_hunt_transitions (session_id, at, state_version);

CREATE TABLE rifts_host_capabilities (
    capability_id TEXT PRIMARY KEY,
    secret_digest TEXT NOT NULL UNIQUE,
    session_id TEXT NOT NULL,
    generation BIGINT NOT NULL,
    host_account_id TEXT NOT NULL,
    host_steam_id TEXT NOT NULL,
    bridge_instance_id TEXT NOT NULL,
    scopes TEXT NOT NULL,
    issued_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    revoked_at BIGINT,
    last_sequence BIGINT NOT NULL
);
CREATE INDEX rifts_host_capabilities_session ON rifts_host_capabilities (session_id, generation);

CREATE TABLE rifts_host_presences (
    account_id TEXT PRIMARY KEY,
    revision BIGINT NOT NULL,
    consent_to_host BOOLEAN NOT NULL,
    observed_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    detail TEXT NOT NULL
);
CREATE INDEX rifts_host_presences_expires ON rifts_host_presences (expires_at);

CREATE TABLE rifts_host_offers (
    offer_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    generation BIGINT NOT NULL,
    account_id TEXT NOT NULL,
    steam_id TEXT NOT NULL,
    rank BIGINT NOT NULL,
    score DOUBLE PRECISION NOT NULL,
    score_breakdown TEXT NOT NULL,
    status TEXT NOT NULL,
    offered_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    responded_at BIGINT,
    response_reason TEXT
);
CREATE INDEX rifts_host_offers_session ON rifts_host_offers (session_id, generation, rank);
CREATE UNIQUE INDEX rifts_host_offers_accepted ON rifts_host_offers (session_id, generation) WHERE status = 'ACCEPTED';

CREATE TABLE rifts_host_reliability (
    account_id TEXT PRIMARY KEY,
    offers_accepted BIGINT NOT NULL,
    offers_declined BIGINT NOT NULL,
    offers_expired BIGINT NOT NULL,
    starts_succeeded BIGINT NOT NULL,
    starts_failed BIGINT NOT NULL,
    clean_completions BIGINT NOT NULL,
    unexpected_failures BIGINT NOT NULL,
    cooldown_until BIGINT,
    updated_at BIGINT NOT NULL
);

CREATE TABLE rifts_participant_observations (
    observation_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    generation BIGINT NOT NULL,
    account_id TEXT NOT NULL,
    steam_id TEXT NOT NULL,
    host_reachable BOOLEAN NOT NULL,
    local_bridge_alive BOOLEAN NOT NULL,
    metrics_available BOOLEAN NOT NULL,
    last_packet_age_ms BIGINT,
    route_kind TEXT NOT NULL,
    ping_ms BIGINT,
    jitter_ms BIGINT,
    packet_loss_percent DOUBLE PRECISION,
    observed_at BIGINT NOT NULL
);
CREATE INDEX rifts_participant_observations_session ON rifts_participant_observations (session_id, generation, observed_at);
CREATE INDEX rifts_participant_observations_account ON rifts_participant_observations (account_id, observed_at);

CREATE TABLE rifts_control_sequences (
    account_id TEXT PRIMARY KEY,
    next_sequence BIGINT NOT NULL
);

CREATE TABLE rifts_control_messages (
    message_id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    sequence BIGINT NOT NULL,
    message_type TEXT NOT NULL,
    session_id TEXT,
    generation BIGINT,
    payload TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    acknowledged_at BIGINT,
    UNIQUE (account_id, sequence)
);
CREATE INDEX rifts_control_messages_expires ON rifts_control_messages (expires_at);

CREATE TABLE rifts_hub_presence (
    session_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    generation BIGINT NOT NULL,
    realm_id TEXT NOT NULL,
    peer_id TEXT NOT NULL,
    state TEXT NOT NULL,
    candidate_id TEXT NOT NULL,
    entry_epoch BIGINT NOT NULL,
    local_bridge_port BIGINT,
    local_bridge_ready_at BIGINT,
    admission_id TEXT,
    route TEXT,
    requested_at BIGINT NOT NULL,
    ticketed_at BIGINT,
    admitted_at BIGINT,
    lost_at BIGINT,
    left_at BIGINT,
    left_reason TEXT,
    expires_at BIGINT NOT NULL,
    purge_at BIGINT,
    PRIMARY KEY (session_id, account_id)
);
CREATE INDEX rifts_hub_presence_generation ON rifts_hub_presence (session_id, generation, state);
CREATE INDEX rifts_hub_presence_account ON rifts_hub_presence (account_id, state);
CREATE INDEX rifts_hub_presence_realm ON rifts_hub_presence (realm_id, state);
CREATE INDEX rifts_hub_presence_purge ON rifts_hub_presence (purge_at);
```

Each `CREATE TABLE` must start a line with exactly `CREATE TABLE <name> ` (the §2.4 test reads it that way).

### 4.3 Columns that hold encoded values

| Column | Content |
| --- | --- |
| `rifts_hunt_sessions.detail` | The hunt record serialised with serde as the domain spec defines `HuntSession`'s JSON (camelCase, absent optionals omitted). The roster is not in it: the roster is the `rifts_hunt_roster` rows |
| `rifts_hunt_sessions.global_id` | The module-api global id (`<kind>:<ULID>@<node>`, kind hub instance or hunt) the domain's id rules derive from the wire session id, whether it is a realm hub and the node; `NULL` when the wire id does not convert |
| `rifts_hunt_sessions.session_kind` | `REALM_HUB` for a realm hub session, otherwise `NULL` |
| `rifts_hunt_sessions.hub_kind` | `RAMSGATE` or `TRAINING`, otherwise `NULL` |
| `rifts_host_capabilities.scopes` | The scope names (`HOST_READY`, `LEASE_HEARTBEAT`, `PEER_ADMITTED`, `GAME_SERVER_RUNTIME`, `EVENTS_WRITE`, `COMPLETE_REQUEST`, `ABORT`) joined with `,` in the record's order; read back by splitting on `,` and skipping empty pieces |
| `rifts_host_presences.detail` | The whole host presence record as serde JSON (domain spec); the other columns are copies for querying |
| `rifts_host_offers.score_breakdown` | The offer's score breakdown as serde JSON (domain spec) |
| `rifts_control_messages.payload` | The message payload, compact JSON text, keys in their insertion order |
| `rifts_control_sequences.next_sequence` | Despite its name, the last sequence handed out for the account |
| `rifts_realms.hosting_*` | The realm's hosting record, present only when `hosting_updated_at` is not `NULL` (§5.2) |

### 4.4 `down.sql` (both engines, same text)

Drops the tables in reverse creation order: `rifts_hub_presence`, `rifts_control_messages`,
`rifts_control_sequences`, `rifts_participant_observations`, `rifts_host_reliability`, `rifts_host_offers`,
`rifts_host_presences`, `rifts_host_capabilities`, `rifts_hunt_transitions`, `rifts_hunt_roster`,
`rifts_hunt_sessions`, `rifts_realm_presence`, `rifts_realm_invites`, `rifts_realm_hub_rooms`,
`rifts_realm_player_invites`, `rifts_realm_join_requests`, `rifts_realm_bans`, `rifts_realm_members`, `rifts_realms`;
one `DROP TABLE <name>;` per line.

---

## 5. Race semantics: what Node had and how it is kept

Node ran on MongoDB and kept its invariants with unique indexes and guarded `updateOne` calls (filter carries the
precondition, `matchedCount` decides), plus `realmHubGates` documents to serialise hub entries, and
`WithP2PTransaction` for multi-step lifecycle changes. The Rust port keeps every one of those by this rule:

1. **Every method that changes data is exactly one `Database::write`**, and its precondition is read and checked
   inside that same closure. `store` serialises writes on both engines (§3.1), so a check-then-write inside one
   closure cannot interleave with another write. This replaces every guarded `updateOne` and the hub gate.
2. **A refusal is a typed `Ok` outcome**, not an error, so the transaction still commits whatever it wrote before
   (nothing, in every refusal path below). Only `HuntRepository::transaction` callers use `Err` to roll back several
   steps.
3. **Unique indexes still exist** (§4.2) as the last line. The repositories pre-check them and answer
   `Duplicate` or a typed outcome; a constraint violation that slips past a pre-check surfaces as `RepoError::Query`.
4. **Reads** go to the read pool and see committed data only; nothing relies on a read followed by a separate write.

The races the contract tests fire at once (spawned tasks on one database, both engines) and what must come out:

| Race | Outcome |
| --- | --- |
| Many `insert_realm` with one name | Exactly one `Inserted`, the rest `NameTaken` |
| Many `add_member` past the ceiling | Members never exceed `max_members` |
| One account's double `add_member` | Added once; the other `AlreadyMember` |
| Many `consume_invite` on one code | Uses never exceed `max_uses` |
| Many `add_join_request` past the bound | Requests never exceed the bound |
| Disjoint-field `patch_hosting` calls | Every field lands (each reads the current record inside its write) |
| Many `bind` for one account | Exactly one presence row |
| Many `request_entry` for one hub seat | Exactly one row |
| Many `request_entries` batches | Live rows never exceed `max_peers` |

---

## 6. `RealmsRepository` and `RealmPresenceRepository` (`repo::realms`)

Both are `Clone`, built with `new(db: RealmsDb)`. Records (`RealmRecord`, `RealmMember`, `RealmBan`, `AccountAt`,
`RealmHosting`, `HostingPatch`, `HeartbeatUpdate`, `RealmInviteRecord`, `RealmPresenceRecord`, `JoinPolicy`,
`MemberRole`, `HubState`) and the constants `INVITE_PURGE_GRACE_MS` (7 days, 604,800,000 ms) and
`apply_hosting_patch` belong to the domain spec's `realm` module.

### 6.1 Outcome enums (public, `Debug, Clone, Copy, PartialEq, Eq`)

| Enum | Variants |
| --- | --- |
| `InsertRealmResult` | `Inserted`, `NameTaken`, `HostHasRealm` |
| `AddMemberResult` | `Added`, `AlreadyMember`, `Banned`, `Full`, `NotFound` |
| `AddBanResult` | `Banned`, `AlreadyBanned`, `Full`, `NotFound` |
| `AddEntryResult` | `Added`, `Already` (Node's `ALREADY_REQUESTED` or `ALREADY_INVITED`), `Banned`, `AlreadyMember`, `Full`, `NotFound` |
| `InsertInviteResult` | `Inserted`, `Duplicate` |

### 6.2 Reading a realm

A realm record is the `rifts_realms` row plus its children, each list in ascending `seq`: members (account, role,
joined at), bans (account, reason, at), join requests and player invites (account, at), hub room ids.

`hosting` is `None` when `hosting_updated_at` is `NULL`. Otherwise it is built with these defaults for `NULL`
columns: `max_servers` 1, `max_players` 16, `active_servers` 0; `afk_seconds` and `hub_session_id` stay absent;
`hub_state` and `training_state` read `NULL` as `NONE`. An unknown role, join policy or hub state is `Corrupt`.

Child `seq`: a new child row takes one more than the largest `seq` of that table for that realm (1 for the first).
Hub rooms written at insert take 1, 2, 3... in list order.

### 6.3 `RealmsRepository` methods

All `async`, all returning `Result<_, RepoError>`. "Write" means one write transaction as `actor`; "read" a read.

| Method | Kind | Contract |
| --- | --- | --- |
| `find_by_id(&self, realm_id: &str) -> Option<RealmRecord>` | read | By primary key |
| `find_by_name(&self, name: &str) -> Option<RealmRecord>` | read | Exact, byte-wise match |
| `find_by_host(&self, host: &str) -> Option<RealmRecord>` | read | By `host_account_id` |
| `find_by_member(&self, account_id: &str, limit: u32) -> Vec<RealmRecord>` | read | Realms with a member row for the account (its own, where it is `HOST`, included), ordered by name, at most `limit` |
| `find_by_player_invite(&self, account_id: &str, limit: u32) -> Vec<RealmRecord>` | read | Realms holding a player invite for the account, by name, at most `limit` |
| `list_listed(&self, limit: u32, offset: u32) -> Vec<RealmRecord>` | read | `listed` realms; rows with `online_until` set first, those by `online_until` descending; then by name; `LIMIT`/`OFFSET` |
| `insert_realm(&self, actor, realm: &RealmRecord) -> InsertRealmResult` | write | Name taken → `NameTaken`; else the host has a realm → `HostHasRealm` (name checked first). Else inserts the row (identity fields, `listed`, `created_at`, `last_online_at`, `online_until`, `version`, `offline_notified_for`), then the hosting columns if `hosting` is present (as `patch_hosting` writes them), then members, bans, join requests, player invites in list order, then hub rooms with `seq` 1.. |
| `delete_realm_if_last_online(&self, actor, realm_id, expected_last_online_at: Option<UnixMillis>) -> bool` | write | Name release. False when the realm is gone or its `last_online_at` (as an option) differs from `expected`. Else deletes the realm (§6.4) and answers true |
| `delete_realm_of_host(&self, actor, realm_id, host) -> bool` | write | False unless a realm with that id has that host; else deletes it (§6.4) |
| `set_settings(&self, actor, realm_id, host, join_policy: Option<JoinPolicy>, listed: Option<bool>) -> bool` | write | Both `None` → true **without touching the database** (Node's). Else false unless the realm has that host; then sets each given field; true |
| `record_heartbeat(&self, actor, realm_id, host, update: &HeartbeatUpdate) -> bool` | write | On the realm with that host: sets `last_online_at` and `online_until` from the update, and `version` to the update's when present (an absent version keeps the old). True iff one row changed |
| `patch_hosting(&self, actor, realm_id, patch: &HostingPatch, at: UnixMillis) -> bool` | write | False when the realm is gone. Else reads the current hosting (§6.2), applies the domain's `apply_hosting_patch(current, patch, at)` and writes every hosting column from the result (`hub_state`/`training_state` as their names, `NONE` included, never `NULL`) |
| `mark_offline(&self, actor, realm_id, at) -> bool` | write | Only when `online_until > at`: sets `online_until` and `offline_notified_for` to `at` (ends a lasting online state without the sweeper noticing it). True iff it changed |
| `claim_lapsed_hosts(&self, actor, now, since, limit: u32) -> Vec<RealmRecord>` | write | Realms with `since < online_until <= now` and `offline_notified_for` `NULL` or different from `online_until`, ordered by `online_until` then `realm_id`, at most `limit`. Sets each one's `offline_notified_for = online_until`, then returns the records as they are after that. A lapse is claimed once |
| `find_by_hub_room_id(&self, room_id) -> Option<RealmRecord>` | read | Exact match on a hub room id |
| `add_member(&self, actor, realm_id, member: &RealmMember, max_members: u32) -> AddMemberResult` | write | In order: realm missing → `NotFound`; banned → `Banned`; already a member → `AlreadyMember`; member count ≥ `max_members` → `Full`. Else inserts the member and, in the same write, deletes the account's player invite for that realm |
| `remove_member(&self, actor, realm_id, account_id) -> bool` | write | Deletes the member row only if its role is `MEMBER` (never the host). True iff deleted |
| `add_ban(&self, actor, realm_id, ban: &RealmBan, max_bans: u32) -> AddBanResult` | write | Realm missing → `NotFound`; already banned → `AlreadyBanned`; ban count ≥ `max_bans` → `Full`. Else inserts the ban and deletes the account's `MEMBER`-role membership (a `HOST` row stays), join request and player invite |
| `remove_ban(&self, actor, realm_id, account_id) -> bool` | write | Deletes the ban row; true iff deleted |
| `add_join_request(&self, actor, realm_id, request: &AccountAt, max_requests: u32) -> AddEntryResult` | write | Order: `NotFound`, `Banned`, `AlreadyMember`, `Already` (row exists), `Full` (count ≥ max). Else inserts |
| `remove_join_request(&self, actor, realm_id, account_id) -> bool` | write | Deletes; true iff deleted |
| `add_player_invite(&self, actor, realm_id, invite: &AccountAt, max_invites: u32) -> AddEntryResult` | write | As `add_join_request`, on player invites |
| `remove_player_invite(&self, actor, realm_id, account_id) -> bool` | write | Deletes; true iff deleted |
| `insert_invite(&self, actor, invite: &RealmInviteRecord) -> InsertInviteResult` | write | Code exists → `Duplicate`. Else inserts with `purge_at = expires_at + INVITE_PURGE_GRACE_MS` (saturating) |
| `find_invite(&self, code) -> Option<RealmInviteRecord>` | read | By code |
| `list_invites(&self, realm_id, limit: u32) -> Vec<RealmInviteRecord>` | read | The realm's invites, `created_at` descending then `code` descending, at most `limit` |
| `count_active_invites(&self, realm_id, now) -> i64` | read | Not revoked, `expires_at > now`, `uses < max_uses` |
| `consume_invite(&self, actor, code, now) -> Option<RealmInviteRecord>` | write | Adds one use only if not revoked, `expires_at > now` and `uses < max_uses`; returns the invite after the change, else `None` |
| `refund_invite(&self, actor, code) -> ()` | write | Takes one use back if `uses > 0` (a join that consumed it failed afterwards). Never below zero; silent when absent |
| `revoke_invite(&self, actor, realm_id, code, at) -> Option<RealmInviteRecord>` | write | `None` when the code is absent or belongs to another realm. Already revoked → the invite unchanged (first time kept). Else sets `revoked_at = at` and returns it |
| `purge_expired(&self, now) -> usize` | write as `System("realms-purge")` | Deletes invites with `purge_at <= now` and presence rows with `expires_at <= now`; returns the total |

### 6.4 Deleting a realm

Deletes, in one write, the realm's rows in `rifts_realm_members`, `rifts_realm_bans`, `rifts_realm_join_requests`,
`rifts_realm_player_invites`, `rifts_realm_hub_rooms`, `rifts_realm_invites` (Node deletes invites with the realm),
then `rifts_realms`. Children first, for the foreign keys. Presence rows are not touched here (the directory calls
`RealmPresenceRepository::remove_for_realm`).

### 6.5 `RealmPresenceRepository` methods

One row per account: which realm its current game login belongs to.

| Method | Kind | Contract |
| --- | --- | --- |
| `bind(&self, actor, row: &RealmPresenceRecord) -> ()` | write | Upsert on `account_id`: replaces realm, bound at, last seen, expires at |
| `find_active(&self, account_id, now) -> Option<RealmPresenceRecord>` | read | The row only while `expires_at > now` (strictly before) |
| `touch(&self, actor, account_id, now, expires_at) -> Option<RealmPresenceRecord>` | write | Only a row with `expires_at > now`: sets `last_seen_at = now`, `expires_at`; returns the row (as `find_active` at `now`). An expired or absent row is never brought back: `None` |
| `remove(&self, actor, account_id, realm_id: Option<&str>) -> bool` | write | Deletes the account's row; with a realm, only if bound to it. True iff deleted |
| `remove_for_realm(&self, actor, realm_id) -> usize` | write | Deletes every row of the realm; the count |
| `count_active_by_realm(&self, realm_ids: &[String], now) -> BTreeMap<String, i64>` | read | Rows with `expires_at > now` per realm among the ids; realms with none are absent. An empty id list answers an empty map without a query |

---

## 7. `RealmHubRepository` (`repo::hub`)

Who is in a realm hub: one row per (hub session, account), written without touching the session row. Every method is
one write that applies the domain spec's hub presence rules: the entry decision for a request (create, reuse,
re-enter), the liveness and expiry tests of a row at a time, the list of live states, and the transition table
(state × event → next state or none). Events used: ticket issued, admitted, disconnected, travelled, removed,
expired, session ended.

### 7.1 Public types

`HubPresenceRecord` (`Debug, Clone, PartialEq, Eq`), one field per column of `rifts_hub_presence` with times as
`UnixMillis`: `session_id`, `generation: i64`, `realm_id`, `account_id`, `peer_id` (the transport peer id the ticket
and the link carry), `state: PresenceState`, `candidate_id` (the matchmaking candidate of the current entry),
`entry_epoch: i64` (1 for the first entry, +1 per re-entry), `local_bridge_port: Option<i64>`,
`local_bridge_ready_at`, `admission_id: Option<String>`, `route: Option<String>` (`relay` or `direct`),
`requested_at`, `ticketed_at`, `admitted_at`, `lost_at`, `left_at`, `left_reason: Option<String>`, `expires_at`
(waiting rows expire then), `purge_at: Option<UnixMillis>` (set once the row stops being live).

`EntryOutcome` (`Copy`): `Created`, `Reused`, `Reentered`.
`HubEntryRequest { account_id, peer_id, candidate_id }`.
`EntryAt { session_id, generation: i64, realm_id, now: UnixMillis, expires_at: UnixMillis }`.
`HubEntryBatch`: `Placed(Vec<(String, HubPresenceRecord, EntryOutcome)>)` (account id, row, outcome) or
`Full { live: i64, max_peers: i64 }` (nothing written).

Rows are written whole (upsert on `session_id, account_id`, every other column replaced).

### 7.2 One entry (the entry rule)

Given the existing row for (session, account), if any, and `now`, the domain's entry decision says:

- **Reuse** (a waiting row still valid): the row stays; if the request's `expires_at` is later than the row's, the
  row's `expires_at` grows to it and the row is written (expiry only ever grows). Outcome `Reused`, the row returned.
- **Re-enter**: the row keeps its session and account; it takes the request's generation, realm, peer and candidate,
  becomes `PENDING`, `requested_at = now`, `expires_at` from the request, `entry_epoch` one more than before, and
  every connection field cleared (`local_bridge_port`, `local_bridge_ready_at`, `admission_id`, `route`,
  `ticketed_at`, `admitted_at`, `lost_at`, `left_at`, `left_reason`, `purge_at`). Outcome `Reentered`.
- **Anything else**: a fresh `PENDING` row with `entry_epoch` 1 and nothing but the identity, `requested_at = now`
  and `expires_at` set; it replaces whatever row was there. Outcome `Created`.

### 7.3 Methods

| Method | Contract |
| --- | --- |
| `request_entry(&self, actor, at: &EntryAt, request: &HubEntryRequest) -> (HubPresenceRecord, EntryOutcome)` | One entry (§7.2), no capacity check (tests only; production uses `request_entries`) |
| `request_entries(&self, actor, at: &EntryAt, max_peers: i64, entries: &[HubEntryRequest]) -> HubEntryBatch` | First de-duplicates by account: an account listed twice keeps its first position and its last value. Then, in one write: `live` = rows of (session, generation) that are live at `now`; `newcomers` = requests whose own row for (session, account), whatever its generation, is absent or not live. If `newcomers > 0` and `live + newcomers > max_peers` → `Full { live, max_peers }`, nothing written. Else applies §7.2 to each request in order and answers `Placed` in that order. A member whose own row is live takes no seat and is never refused; an expired waiting row takes no seat |
| `admit_host(&self, actor, session_id, generation, realm_id, account_id, peer_id, local_bridge_port: i64, now) -> HubPresenceRecord` | The host's own row at hub start, created or replaced `ADMITTED`. Starting from the existing row, or from a new one with `candidate_id = "host:<session_id>"`, `requested_at = now`, `expires_at = now`: sets generation, realm, peer, `ADMITTED`, `local_bridge_port`, `local_bridge_ready_at = now`, `admitted_at = now`, `entry_epoch` = previous + 1 (1 when new), and clears `ticketed_at`, `lost_at`, `left_at`, `left_reason`, `purge_at`, `admission_id`, `route`. An existing row keeps its `candidate_id`, `requested_at` and `expires_at` |
| `find_presence(&self, session_id, account_id) -> Option<HubPresenceRecord>` | read |
| `find_for_account(&self, account_id, states: Option<&[PresenceState]>) -> Option<HubPresenceRecord>` | read. The account's row in any hub whose state is in `states` (the live states when `None`), newest `requested_at` first, then `session_id` ascending; the first |
| `list_by_state(&self, session_id, generation, states: &[PresenceState], limit: u32) -> Vec<HubPresenceRecord>` | read. Rows of the generation in `states`, by `requested_at` then `account_id`, at most `limit` |
| `count_by_state(&self, session_id, generation, states) -> i64` | read |
| `mark_ticketed(&self, actor, session_id, generation, account_id, now, ticket_expires_at) -> Option<..>` | Single-row transition (§7.4), event ticket issued, guard: not expired at `now`. Sets `TICKETED`, `ticketed_at = now`, `expires_at = max(expires_at, ticket_expires_at)` (the row lives at least as long as the ticket) |
| `mark_route_ready(&self, actor, session_id, generation, account_id, local_bridge_port, ready_at, now) -> Option<..>` | Single-row transition, applicability checked with the admitted event, guard: not expired at `now`. Sets only `local_bridge_port` and `local_bridge_ready_at`; the state is unchanged (a waiting row not expired, or a connected one) |
| `mark_admitted(&self, actor, session_id, generation, account_id, admission_id, admitted_at, now, route: Option<&str>) -> Option<..>` | Event admitted, guard: not expired at `now` (`PENDING`, unexpired `TICKETED`, or `ADMITTED` again, never `LOST` or `EXPIRED` as the table says). Sets `ADMITTED`, `admission_id`, `admitted_at`, `route` only when given, clears `lost_at`, `left_at`, `left_reason`, `purge_at` |
| `mark_lost(&self, actor, session_id, generation, account_id, admission_id: Option<&str>, lost_at, purge_at) -> Option<..>` | Event disconnected; guard: no `admission_id` given, or it equals the row's (only the current connection's report ends it). Sets `LOST`, `lost_at`, `purge_at` |
| `mark_left(&self, actor, session_id, generation, account_id, reason, left_at, purge_at) -> Option<..>` | Event travelled, no guard. Sets `LEFT`, `left_at`, `left_reason = reason` (`TRAVEL` when the member went into a realm hunt), `purge_at` |
| `remove_account_from_realm(&self, actor, realm_id, account_id, at, purge_at) -> Vec<HubPresenceRecord>` | Bulk (§7.5) over the account's rows in the realm (any hub), event removed: `left_at = at`, `left_reason = "REMOVED"`, `purge_at` |
| `remove_realm(&self, actor, realm_id, at, purge_at) -> Vec<..>` | Bulk over every row of the realm, event removed: `left_at`, `left_reason = "REALM_RELEASED"`, `purge_at` |
| `expire_stale(&self, actor, session_id: Option<&str>, now, purge_at) -> Vec<..>` | Bulk over rows with `expires_at <= now` (in one hub, or all), event expired: `purge_at` |
| `end_session(&self, actor, session_id, generation, at, purge_at) -> Vec<..>` | Bulk over the generation's rows ordered by `account_id`, event session ended: `lost_at = at`, `purge_at` |
| `purge(&self, now) -> usize` | Write as `System("realms-purge")`: deletes rows with `purge_at` set and `<= now` (Node's TTL index, a week after a row stopped being live) |

### 7.4 Single-row transition

Load the (session, account) row. Answer `None`, writing nothing, when it is absent, its generation differs from the
argument, the transition table has no move for its state and the method's event, or the guard refuses. Otherwise
apply the method's changes, write the row and answer it as written.

### 7.5 Bulk transition

Select the rows (ordered by `session_id`, `account_id` unless the method says otherwise). For each row in order whose
state has a move for the event: set the state to the table's target, apply the method's changes, write it. Answer
the changed rows **as they were before the change**, in order; rows without a move are skipped and not returned.

The ending of a hub generation is also available inside a hunt transaction (`HuntTx::end_hub_presence`, §8.3) so it
commits with the session's own change.

---

## 8. `HuntRepository` and `HuntTx` (`repo::hunts`)

Realms' part of Node's `P2PHuntRepository`: sessions with their roster, transitions, host capabilities, the control
channel, host presences, offers, reliability and observations. Records (`HuntSession`, `Participant`,
`AdmissionState`, `HuntState`, `HuntTransition`, `HostCapabilityRecord`, `ControlMessageRecord`,
`HostPresenceRecord`, `HostOfferRecord`, `OfferStatus`, `ScoreBreakdown`, `HostReliabilityRecord`,
`ReliabilityIncrements`, `ObservationRecord`) are the domain spec's `hunt` module.

### 8.1 State sets

| Name | States |
| --- | --- |
| `pub const LIVE_HOST_STATES: &[HuntState]` | `HOST_OFFERED`, `STARTING`, `HOST_READY`, `ADMITTING`, `RUNNING`, `SUSPECT` |
| realm server states | `HOST_SELECTING`, `HOST_OFFERED`, `STARTING`, `HOST_READY`, `ADMITTING`, `RUNNING`, `SUSPECT` |
| roster live states | `HOST_READY`, `ADMITTING`, `RUNNING` |
| renewable / disconnectable states | `HOST_READY`, `ADMITTING`, `RUNNING`, `DRAINING`, `SUSPECT` |

### 8.2 Loading and saving a session

**Load:** read `detail`, deserialize it as `HuntSession` (failure → `Corrupt("hunt <id>: <err>")`), then set its
roster from `rifts_hunt_roster` in `seq` order (unknown admission state → `Corrupt`).

**Save** (one session, inside a write): upsert the session row on `session_id`. On insert every column is written;
on conflict every column except `session_id` and **`global_id`** is replaced, so a session's global id is fixed by
its first save. Column values: `generation`, `state` name, `state_version`, `party_id`, `active_party_key`,
`host_account_id`, `session_kind`/`hub_kind`/`global_id` as §4.3, `realm_id`, `lease_expires_at`, `created_at`,
`expires_at`, `detail`. Then sync the roster: delete roster rows of the session whose account is not in the
record's roster (all of them when it is empty), and upsert each participant on (`session_id`, `account_id`) with
`seq` = its 1-based position and every other column replaced.

### 8.3 `HuntTx`

`pub struct HuntTx<'a>`: one write transaction over the hunt tables (Node's `WithP2PTransaction`). Every step a
lifecycle change makes (the compare-and-set, its transition, the offer, the control messages, the hub presence)
commits together or not at all. Methods are synchronous and return `Result<_, RepoError>`.

| Method | Contract |
| --- | --- |
| `find_hunt(&mut self, session_id) -> Option<HuntSession>` | Load |
| `create_hunt(&mut self, session: &HuntSession) -> ()` | `Err(Duplicate("session_id"))` when the id exists; `Err(Duplicate("active_party_key"))` when its key is set and any session holds it. Else save |
| `update(&mut self, session_id, bump: bool, change: impl FnOnce(&mut HuntSession) -> bool) -> Option<HuntSession>` | Load (`None` if absent); `change` decides (false → `None`, nothing written) and edits; `bump` adds 1 to `state_version`; if the key is set and another session holds it → `Err(Duplicate("active_party_key"))`; save; answer the saved record |
| `compare_and_set(&mut self, session_id, generation, expected_states: &[HuntState], expected_state_version, change: impl FnOnce(&mut HuntSession)) -> Option<HuntSession>` | Node's `compareAndSetHunt`: an `update` with bump that applies `change` only while the generation, a state in `expected_states` and the state version all still match |
| `append_transition(&mut self, t: &HuntTransition) -> ()` | Insert; an empty `transition_id` gets a fresh random UUID v4 |
| `begin_host_startup(&mut self, s: &HostStartup, now) -> Option<HuntSession>` | Bumped update. Only for `host_account_id == s.host_account_id`, state `HOST_OFFERED` and `expires_at > now` (no generation check). Sets `STARTING`, `bridge_instance_id`, `metadata_nonce`, `startup_deadline`, `server_udp_port` when given (else kept); clears `lobby_id`, `lease_expires_at`, `suspect_since`, `suspect_deadline`, `pre_suspect_state` |
| `renew_host_lease(&mut self, r: &LeaseRenewal) -> Option<HuntSession>` | Bumped update. Only when generation, `host_steam_id` and `bridge_instance_id` match, the state is renewable (§8.1) and the last lease sequence is absent or strictly lower than `r.lease_sequence`. Sets `last_lease_sequence`, `last_heartbeat_at`, `lease_expires_at` |
| `mark_host_ready(&mut self, m: &HostReadyMark, now) -> Option<HuntSession>` | Bumped update. Only when generation, steam id, bridge instance and `virtual_port` match and the state is `STARTING` or `HOST_READY`. Sets `HOST_READY`, `lobby_id`, `lease_expires_at`, `last_heartbeat_at = now`, `last_lease_sequence = 0` |
| `mark_participant_route_ready(&mut self, session_id, generation, account_id, steam_id, bridge_instance_id, local_bridge_port, ready_at) -> Option<..>` | Participant change (below), roster live states, matching account **and** steam id, **no** bump. Sets the participant's bridge instance, port, ready at |
| `mark_participant_admitted(&mut self, session_id, generation, account_id, admitted_at) -> Option<..>` | Roster live states, by account, bumped. Sets `ADMITTED`, `admitted_at`; clears `disconnected_at`, `rejoin_deadline` |
| `reserve_participant_ticket(&mut self, session_id, generation, account_id, now, rejoin_deadline, max_reconnects: i64) -> Option<..>` | Roster live states, by account, no bump. Refused when `reconnect_epoch >= max_reconnects`, or when the participant is disconnected and its `rejoin_deadline` is absent or `<= now`. Else `reconnect_epoch += 1`, `ISSUED`, `rejoin_deadline` set |
| `mark_participant_disconnected(&mut self, session_id, generation, account_id, disconnected_at, rejoin_deadline) -> Option<..>` | Disconnectable states, by account, no bump. Sets `LOST`, `disconnected_at`, `rejoin_deadline` |
| `create_capability(&mut self, c: &HostCapabilityRecord) -> ()` | `Err(Duplicate("secret_digest"))` when the digest exists; else insert |
| `advance_capability_sequence(&mut self, capability_id, sequence) -> bool` | Sets `last_sequence` only on an unrevoked capability whose `last_sequence < sequence`; true iff changed |
| `revoke_generation(&mut self, session_id, generation, at) -> ()` | Sets `revoked_at = at` on every unrevoked capability of the generation (an earlier revocation time stays) |
| `enqueue_control(&mut self, m: &NewControlMessage) -> ControlMessageRecord` | Next sequence = the account's stored last sequence + 1 (1 when none); stores it back (upsert). Inserts the message with a fresh random UUID v4 id, `acknowledged_at` absent. Sequences survive the messages' purge, so they never repeat |
| `acknowledge_control(&mut self, account_id, message_id, at) -> bool` | Sets `acknowledged_at = at` on the message of that id and account (again on a second call); true iff a row matched |
| `upsert_host_presence(&mut self, p: &HostPresenceRecord) -> ()` | Upsert on account: revision, consent, observed at, expires at, detail |
| `find_reliability(&mut self, account_id) -> Option<HostReliabilityRecord>` | Read in the transaction |
| `increment_reliability(&mut self, account_id, add: ReliabilityIncrements, now, cooldown_until: Option<UnixMillis>) -> ()` | Starts from the row, or a fresh one (the domain's zeroed record at `now`); adds each counter; `updated_at = now`; a given cooldown replaces the old one, an absent one keeps it; upsert |
| `create_offer(&mut self, o: &HostOfferRecord) -> ()` | When the offer is `ACCEPTED` and an accepted offer exists for its (session, generation) → `Err(Duplicate("accepted offer"))`. Else upsert on `offer_id` |
| `find_offer(&mut self, offer_id) -> Option<HostOfferRecord>` | Read in the transaction |
| `update_offer(&mut self, offer_id, account_id, expected: OfferStatus, status, responded_at, reason: Option<&str>) -> Option<HostOfferRecord>` | Sets status, `responded_at`, `response_reason` (cleared when `None`) only where the id, the account and `status = expected` match; the offer after the change, or `None` |
| `end_hub_presence(&mut self, session_id, generation, at, purge_at) -> Vec<HubPresenceRecord>` | §7.3's `end_session` inside this transaction |
| `record_observation(&mut self, o: &ObservationRecord) -> ()` | Insert (a repeated id is a database error) |

A participant change loads the session; refuses (`None`) unless the generation matches and the state is in the
method's set; finds the first roster entry the method picks; refuses when none or when the method's own check
refuses; else edits it and saves (bumping only where said).

Input structs (public, `Debug, Clone, PartialEq, Eq` unless said):

| Struct | Fields |
| --- | --- |
| `HostStartup` | `session_id`, `host_account_id`, `bridge_instance_id`, `metadata_nonce: String`, `startup_deadline: UnixMillis`, `server_udp_port: Option<i64>` |
| `LeaseRenewal` | `session_id`, `generation: i64`, `host_steam_id`, `bridge_instance_id`, `lease_sequence: i64`, `heartbeat_at`, `lease_expires_at` |
| `HostReadyMark` | `session_id`, `generation: i64`, `host_steam_id`, `bridge_instance_id`, `lobby_id`, `virtual_port: i64`, `lease_expires_at` |
| `NewControlMessage` (`Debug, Clone, PartialEq`) | `account_id`, `message_type`, `session_id: Option<String>`, `generation: Option<i64>`, `payload: serde_json::Value`, `created_at`, `expires_at` |

### 8.4 `HuntRepository`

`Clone`, `new(db: RealmsDb)`.

`transaction<T, E, F>(&self, actor: &Actor, f: F) -> Result<T, E>` where `F: FnOnce(&mut HuntTx<'_>) -> Result<T, E>
+ Send + 'static`, `T: Send + 'static`, `E: From<store::Error> + From<diesel::result::Error> + Send + 'static`: runs
`f` as one write transaction (with the database's node available to the saves). An `Err`, a refusal included,
rolls every step back. The domain modules (hosting, leases, lifecycle, selection) do all multi-step changes this way.

Each of these is the `HuntTx` method of the same meaning run as a transaction of its own, with the same arguments plus
`actor` first: `create_hunt`, `compare_and_set_hunt` (= `compare_and_set`, `change: FnOnce(&mut HuntSession) + Send
+ 'static`), `append_hunt_transition`, `begin_host_startup`, `renew_host_lease`, `mark_host_ready`,
`mark_participant_route_ready`, `mark_participant_admitted`, `reserve_participant_ticket`,
`mark_participant_disconnected`, `create_host_capability`, `advance_host_capability_sequence`, `revoke_generation`,
`enqueue_control_message`, `acknowledge_control_message`, `upsert_host_presence`, `increment_host_reliability`,
`create_host_offer`, `update_host_offer`, `record_participant_observation`.

Reads:

| Method | Contract |
| --- | --- |
| `find_hunt(&self, session_id) -> Option<HuntSession>` | Load |
| `find_active_hunt_for_party(&self, party_key) -> Option<HuntSession>` | By `active_party_key` (a party's, or a realm hub kind's) |
| `find_active_hunt_for_account(&self, account_id) -> Option<HuntSession>` | Sessions with a roster row for the account, `active_party_key` set and `session_kind` `NULL` (never a hub); newest `created_at`, then `session_id` ascending; the first |
| `find_recoverable_hunt_for_account(&self, account_id, now) -> Option<HuntSession>` | Rostered sessions in `RECOVERING` with `expires_at > now`; same order |
| `list_lifecycle_hunts(&self, states: &[HuntState], limit: u32) -> Vec<HuntSession>` | State in `states`, oldest `created_at` first then `session_id`, at most `limit` |
| `list_transitions(&self, session_id) -> Vec<HuntTransition>` | By `at`, then `state_version` |
| `find_host_capability_by_digest(&self, digest) -> Option<HostCapabilityRecord>` | By `secret_digest` |
| `list_control_messages(&self, account_id, after_sequence: i64, now, limit: u32, unacknowledged_only: bool) -> Vec<ControlMessageRecord>` | The account's messages with `sequence > after_sequence` and `expires_at > now` (and unacknowledged when asked: the stale-cursor answer), by sequence, at most `limit` |
| `latest_control_sequence(&self, account_id) -> i64` | The stored last sequence, 0 when the account never had one |
| `list_host_presences(&self, account_ids: &[String], observed_after, now) -> Vec<HostPresenceRecord>` | Rows for those accounts with `observed_at >= observed_after` and `expires_at > now`, read from `detail`, in the order the accounts were given. An empty list answers empty without a query |
| `count_active_host_sessions(&self, account_id) -> i64` | Sessions of that host with `active_party_key` set and state in `LIVE_HOST_STATES` (a draining one gave its key up) |
| `count_active_realm_servers(&self, realm_id) -> i64` | Sessions of that realm with the key set and state in the realm server states (hub, Training and hunts) |
| `count_running_realm_servers(&self, realm_id) -> i64` | Sessions of that realm with (key set and a realm server state) or state `DRAINING` (still running on the host's PC) |
| `list_live_realm_hunts(&self, realm_id, host) -> Vec<HuntSession>` | That realm and host, `session_kind` `NULL`, key set, a realm server state; by `created_at`, at most 64 |
| `find_host_reliability(&self, account_id) -> Option<HostReliabilityRecord>` | |
| `find_host_offer(&self, offer_id) -> Option<HostOfferRecord>` | |
| `list_host_offers(&self, session_id, generation) -> Vec<HostOfferRecord>` | By `rank` |
| `list_recent_participant_observations(&self, session_id, generation, observed_after) -> Vec<ObservationRecord>` | `observed_at >= observed_after`; newest first, then `observation_id` ascending |
| `purge_expired(&self, now) -> usize` | Write as `System("realms-purge")`: control messages and host presences with `expires_at <= now`; the total |

---

## 9. Routes (`ROUTES`, `RealmsRoutes`)

### 9.1 The table, in mount order

`pub const ROUTES: &[rifts_seam::Route]`, 46 entries. `handler` is a stable string: it names the Node function the
route ports, it is the `operation` in the `Unimplemented` a stub answers (and so in the 501 body, §12.2), and it
must equal RF1's inventory (`modules/rustic-rifts/docs/inventory.json`, rows with `component: rifts-realms`, in the
same order by method and path). Mount: E = `Mount::Early` (before isles' shared JSON parser), N = `Normal`,
A = `Admin`, D = `Director`. Auth: what isles' middleware requires before the call (from the inventory) and what this
crate checks itself.

| # | Method | Path | M | `handler` | Auth |
| ---: | --- | --- | :-: | --- | --- |
| 1 | POST | `/mystic/v1/hunts/:sessionId/host-ready` | E | `p2pAdmission.ts:MarkHostReady` | capability `HOST_READY` (checked here) |
| 2 | POST | `/mystic/v1/hunts/:sessionId/abort` | E | `p2pHuntStatus.ts:AbortHunt` | capability `ABORT` |
| 3 | POST | `/mystic/v1/hunts/:sessionId/end` | E | `p2pHuntStatus.ts:EndHunt` | capability `COMPLETE_REQUEST`, else `GAME_SERVER_RUNTIME`, else `ABORT` |
| 4 | GET | `/mystic/v1/hunts/:sessionId/status` | E | `p2pHuntStatus.ts:GetHuntStatus` | launcher, host runtime, admin, or anonymous with capability `LEASE_HEARTBEAT` |
| 5 | POST | `/mystic/v1/hunts/:sessionId/lease` | E | `p2pHostLifecycle.ts:RenewHostLease` | capability `LEASE_HEARTBEAT` |
| 6 | PATCH | `/admin/v1/operations/p2p-emergency-stop` | A | `p2pAdminOperations.ts:UpdateP2PEmergencyStop` | admin |
| 7 | POST | `/launcher/v1/p2p/hunts/:sessionId/host-bootstrap` | N | `p2pLauncher.ts:CreateHostBootstrap` | launcher; isles: active account, consent false |
| 8 | POST | `/launcher/v1/p2p/hunts/:sessionId/route-ready` | N | `p2pLauncher.ts:MarkLocalRouteReady` | launcher |
| 9 | GET | `/launcher/v1/p2p/control` | N | `p2pControl.ts:PollP2PControl` | launcher |
| 10 | POST | `/launcher/v1/p2p/control/:messageId/ack` | N | `p2pControl.ts:AcknowledgeP2PControl` | launcher |
| 11 | POST | `/launcher/v1/p2p/host-presence` | N | `p2pHostLifecycle.ts:SubmitHostPresence` | launcher; active, consent false |
| 12 | POST | `/launcher/v1/p2p/host-offers/:offerId/respond` | N | `p2pHostLifecycle.ts:RespondToHostOffer` | launcher; active, consent false |
| 13 | POST | `/launcher/v1/p2p/hunts/:sessionId/observation` | N | `p2pHostLifecycle.ts:RecordParticipantObservation` | launcher |
| 14 | POST | `/launcher/v1/realm-hosting/hubs/start` | N | `realmHubLifecycle.ts:CreateRealmHubSession` | launcher; active, consent true |
| 15 | POST | `/launcher/v1/realm-hosting/hubs/stop` | N | `realmHubLifecycle.ts:StopRealmHubSession` | launcher |
| 16 | GET | `/launcher/v1/realm-hosting/status` | N | `realmHostingStatus.ts:GetRealmHostingStatus` | launcher |
| 17 | POST | `/launcher/v1/realms` | N | `realmsService.ts:RealmsService.createRealm` | launcher; active, consent true |
| 18 | GET | `/launcher/v1/realms/mine` | N | `realmsService.ts:RealmsService.getMine` | launcher |
| 19 | GET | `/launcher/v1/realms/listed` | N | `realmsService.ts:RealmsService.listListed` | launcher |
| 20 | GET | `/launcher/v1/realms/by-name/:name` | N | `realmsService.ts:RealmsService.getByName` | launcher |
| 21 | GET | `/launcher/v1/realms/invite/:code` | N | `realmsService.ts:RealmsService.previewInvite` | launcher |
| 22 | POST | `/launcher/v1/realms/join` | N | `realmsService.ts:RealmsService.join` | launcher; active, consent false |
| 23 | POST | `/launcher/v1/realms/by-name/:name/leave` | N | `realmsService.ts:RealmsService.leave` | launcher |
| 24 | POST | `/launcher/v1/realms/mine/settings` | N | `realmsService.ts:RealmsService.updateSettings` | launcher; active, consent true |
| 25 | POST | `/launcher/v1/realms/mine/invites` | N | `realmsService.ts:RealmsService.createInvite` | launcher; active, consent true |
| 26 | GET | `/launcher/v1/realms/mine/invites` | N | `realmsService.ts:RealmsService.listInvites` | launcher |
| 27 | POST | `/launcher/v1/realms/mine/invites/:code/revoke` | N | `realmsService.ts:RealmsService.revokeInvite` | launcher; active, consent true |
| 28 | POST | `/launcher/v1/realms/mine/members/:accountId/kick` | N | `realmsService.ts:RealmsService.kick` | launcher; active, consent true |
| 29 | POST | `/launcher/v1/realms/mine/members/:accountId/ban` | N | `realmsService.ts:RealmsService.ban` | launcher; active, consent true |
| 30 | POST | `/launcher/v1/realms/mine/members/:accountId/unban` | N | `realmsService.ts:RealmsService.unban` | launcher; active, consent true |
| 31 | POST | `/launcher/v1/realms/mine/heartbeat` | N | `realmsService.ts:RealmsService.heartbeat` | launcher; active, consent true |
| 32 | POST | `/launcher/v1/realms/mine/release` | N | `realmsService.ts:RealmsService.release` | launcher; active, consent true |
| 33 | GET | `/launcher/v1/realms/mine/requests` | N | `realmsService.ts:RealmsService.listJoinRequests` | launcher |
| 34 | POST | `/launcher/v1/realms/mine/requests/:accountId/approve` | N | `realmsService.ts:RealmsService.approveJoinRequest` | launcher; active, consent true |
| 35 | POST | `/launcher/v1/realms/mine/requests/:accountId/deny` | N | `realmsService.ts:RealmsService.denyJoinRequest` | launcher; active, consent true |
| 36 | POST | `/launcher/v1/realms/mine/members/add` | N | `realmsService.ts:RealmsService.addMemberByUsername` | launcher; active, consent true |
| 37 | POST | `/launcher/v1/realms/mine/player-invites` | N | `realmsService.ts:RealmsService.invitePlayerByUsername` | launcher; active, consent true |
| 38 | POST | `/launcher/v1/realms/mine/player-invites/:accountId/cancel` | N | `realmsService.ts:RealmsService.cancelPlayerInvite` | launcher; active, consent true |
| 39 | GET | `/launcher/v1/realms/player-invites` | N | `realmsService.ts:RealmsService.listMyPlayerInvites` | launcher |
| 40 | POST | `/launcher/v1/realms/player-invites/:name/accept` | N | `realmsService.ts:RealmsService.acceptPlayerInvite` | launcher; active, consent false |
| 41 | POST | `/launcher/v1/realms/player-invites/:name/decline` | N | `realmsService.ts:RealmsService.declinePlayerInvite` | launcher |
| 42 | POST | `/launcher/v1/realms/presence/heartbeat` | N | `realmsService.ts:RealmsService.presenceHeartbeat` | launcher; active, consent false |
| 43 | POST | `/launcher/v1/realms/presence/leave` | N | `realmsService.ts:RealmsService.presenceLeave` | launcher |
| 44 | GET | `/launcher/v1/realms/presence` | N | `realmsService.ts:RealmsService.presenceStatus` | launcher |
| 45 | ALL | `/launcher/v1/realms/*` | N | `realmsLauncher.ts:CreateRealmsRouter (catch-all)` | launcher |
| 46 | POST | `/api/matchmaker/resolve-hunt-launch-manifest` | D | `huntLaunchManifest.ts:ResolvePlayerHostedLaunchManifest` | none (Director) |

Isles mounts the realms routes (17-45) with `Cache-Control: no-store`, only when realms are on; the catch-all (45)
must stay after every other realms route. Isles parses route 1-5 bodies itself as described in rifts-seam's `Mount`.

`pub fn stub_routes() -> rifts_seam::StubRoutes`: `StubRoutes { component: Component::Realms, routes: ROUTES }`
(every route answers `Unimplemented(Realms, handler)`).

### 9.2 `RealmsRoutes`

`pub struct RealmsRoutesParts { directory: Option<Arc<RealmDirectory>>, control: Option<Arc<ControlChannel>>,
hosting: Option<Arc<Hosting>>, capabilities: Option<Arc<HostCapabilities>>, clock: Arc<dyn Clock>,
realms_enabled: bool }` (all fields public). `RealmsRoutes::new(parts)`; `RealmsRoutes` is `Clone`. It implements
`rifts_seam::RouteHandler`: `routes()` answers `ROUTES`; `handle(call)` dispatches (below) and maps errors (§9.5).

Request helpers the dispatch relies on:

- **Path parameter** `name`: the first pair in `call.params` with that key; absent reads as empty text where a
  string is needed (`sessionId`, `offerId`, `messageId`, `code`, `accountId`), as `None` for `name`.
- **Query parameter**: split `call.query` on `&`, skip empty pieces, split each at its first `=` (no `=` → empty
  value), the first piece whose key matches. Values are passed on **as written, not percent-decoded**.
- **Launcher account**: `Caller::Launcher(account)` gives the account id; any other caller is refused
  `AUTH_UNAUTHORIZED` / `Sign in required.`.
- **Realms body**: empty body → no body; otherwise JSON, and invalid JSON is refused `REALM_VALIDATION_FAILED` /
  `The request body is not valid JSON.`.
- **P2P body**: empty body → an empty JSON object; a JSON object → itself; anything else (invalid JSON, an array, a
  scalar) is refused `P2P_TICKET_INVALID` / `The request body must be a JSON object.`.
- **Input parse failure** from the domain's `input` parsers: refused with the failure's code and message.
- **`accountId` parameter**: the domain's account id parser; refused `REALM_VALIDATION_FAILED` /
  `accountId is malformed.` when it rejects.
- **Success**: status 200, the handler's value serialised with serde (`JSON.stringify`'s field order is the struct's
  field order; `preserve_order` for maps, §14). A serialisation failure becomes `RealmsError::Database`.

### 9.3 Dispatch of the realms routes (path starts with `/launcher/v1/realms`)

Checks, in this order, before any handler: no directory → `Unimplemented(Realms, handler)`; realms not enabled →
`REALMS_DISABLED` / `Realms are not available on this server.`; caller not a launcher → `AUTH_UNAUTHORIZED`. Then,
by route (all calls on `RealmDirectory`, domain spec; `account` = the caller):

| # | Steps |
| ---: | --- |
| 17 | realms body → `parse_create_realm_body` → `create_realm(account, &input)` |
| 18 | `get_mine(account)` |
| 19 | `parse_list_query(limit, offset)` from the query → `list_listed(account, limit, offset)` |
| 20 | `parse_name_param(name)`, then `parse_known_host_key_query(knownHostKey)` → `get_by_name(account, &name, known)` |
| 21 | `parse_invite_code_param(code)` → `preview_invite(account, &code)` |
| 22 | body → `parse_join_body` → `join` |
| 23 | name → `leave` |
| 24 | body → `parse_settings_body` → `update_settings` |
| 25 | body → `parse_invite_body` → `create_invite` |
| 26 | `list_invites` |
| 27 | code → `revoke_invite` |
| 28 | accountId → `kick` |
| 29 | accountId first, then body → `parse_ban_body` (an optional reason) → `ban(account, &target, reason)` |
| 30 | accountId → `unban` |
| 31 | body → `parse_heartbeat_body` → `heartbeat` |
| 32 | `release` |
| 33 | `list_join_requests` |
| 34, 35 | accountId → `approve_join_request` / `deny_join_request` |
| 36, 37 | body → `parse_username_body` → `add_member_by_username` / `invite_player_by_username` |
| 38 | accountId → `cancel_player_invite` |
| 39 | `list_my_player_invites` |
| 40, 41 | name → `accept_player_invite` / `decline_player_invite` |
| 42, 43, 44 | `presence_heartbeat` / `presence_leave` / `presence_status` |
| 45 | refused `REALM_ROUTE_NOT_FOUND` / `Unknown realms route.` |

`/launcher/v1/realm-hosting/...` does not start with `/launcher/v1/realms`, so 14-16 go through §9.4.

### 9.4 Dispatch of the other routes

No rate limit applies on these (Node applied none on `/launcher/v1/p2p/*` or the lease route). Each needs its
service: `hosting` (and `capabilities` for routes 1, 2, 3, 5) or `control` (9, 10); a missing one →
`Unimplemented(Realms, handler)`. A capability check is `HostCapabilities::check(header, session_id, scope, now)`
with the raw `x-mystic-host-capability` header (may be absent), the `sessionId` parameter and the clock's now; its
refusal is the route's answer. Capability checks run before body parsing.

| # | Steps |
| ---: | --- |
| 1 | check `HOST_READY` → p2p body → `Hosting::mark_host_ready(&capability, &body)` |
| 2 | check `ABORT` → p2p body → `abort_hunt` |
| 3 | check `COMPLETE_REQUEST`; on refusal check `GAME_SERVER_RUNTIME`; on refusal check `ABORT`, whose refusal is the answer → p2p body → `finish_hunt` |
| 4 | caller → account: launcher → its account; host runtime → its host account; admin → none; anonymous → when `capabilities` exists and the header is present, check `LEASE_HEARTBEAT` (refusal is the answer) and use none, else `AUTH_UNAUTHORIZED` / `Sign in required.` → `get_hunt_status(account, session_id)` |
| 5 | check `LEASE_HEARTBEAT` → p2p body → `renew` |
| 6 | caller must be admin, else `AUTH_UNAUTHORIZED` / `Admin authentication required.` → the raw body as UTF-8 text (invalid UTF-8 reads as empty) → `HuntLifecycle::set_emergency_stop(hosting, key_id, Json(text))` → 200 with the returned JSON text as is |
| 7 | launcher → `create_host_bootstrap(account, session_id)` |
| 8 | launcher → p2p body → `mark_local_route_ready(account, session_id, &body)` |
| 9 | launcher → `ControlChannel::poll(account, afterSequence, waitMs)` with the raw query values (`Option<&str>`) |
| 10 | launcher → `ack(account, message_id)` |
| 11 | launcher → p2p body → `submit_presence` |
| 12 | launcher → p2p body → `respond_to_offer(account, offer_id, &body)` |
| 13 | launcher → p2p body → `observe(account, session_id, &body)` |
| 14 | launcher → p2p body → `start_realm_hub` |
| 15 | launcher → p2p body → `stop_realm_hub` |
| 16 | launcher → `get_realm_hosting_status` |
| 46 | no caller check → raw body as UTF-8 text → `HuntLifecycle::resolve_launch_manifest(hosting, Json(text))` → 200 with the JSON as is |

The MIT dispatcher may key on (method, path) or on the handler string; the observable result is the same.

### 9.5 Errors to the seam

`RealmsError::Unimplemented(u)` → `SeamError::Unimplemented(u)`; `Refused(r)` → `SeamError::Refused(r)`;
`Database(_)` → `SeamError::Refused(Refusal { code: "INTERNAL", message: "The realms request failed." })`. The
database text never reaches a client. Isles writes refusals as `{"error":{"code","message","requestId"}}` with the
status its registry gives the code.

---

## 10. `RealmsService`: the seam traits

### 10.1 Construction

`pub struct RealmsServiceParts { directory: Option<Arc<RealmDirectory>>, capabilities:
Option<Arc<HostCapabilities>>, hosting: Option<Arc<Hosting>>, clock: Arc<dyn Clock>, core: Arc<dyn IslesCore>,
policy: HostingPolicy, realms_enabled: bool, node: NodeId }` (public fields; `NodeId` is module-api's).
`RealmsService::new(parts)`. "The dedicated default" below means `rifts_seam::DedicatedOnly`'s answer for the same
call (spec-rd27-rifts-admission-relay.md).

### 10.2 `Lifecycle`

| Method | Answer |
| --- | --- |
| `runtime_safety_issues(settings)` | The domain's relay-only warning for these settings as one string, when it applies; otherwise empty |
| `startup_summary(_)` | `Some(Json)` exactly `{"p2pHuntsEnabled":<policy.enabled>,"realmsEnabled":<realms_enabled>,"trustTier":<policy.trust_tier>}` |
| `start(_)` | `Ok(())`; starts nothing (no purge sweeper runs, §15) |
| `stop()` | `Ok(())` |
| `tables()` | `TABLES` |
| `account_deletion_steps()` | Seven `Delete` steps, in order: `rifts_realms`.`host_account_id`; `rifts_realm_members`.`account_id`; `rifts_realm_bans`.`account_id`; `rifts_realm_join_requests`.`account_id`; `rifts_realm_player_invites`.`account_id`; `rifts_realm_invites`.`created_by`; `rifts_realm_presence`.`account_id` |

### 10.3 `RealmsHooks`

| Method | Answer |
| --- | --- |
| `metric_class(path)` | `Some("realms")` for paths starting `/launcher/v1/realms` or `/launcher/v1/realm-hosting`; `Some("p2p")` for `/launcher/v1/p2p` or `/mystic/v1/hunts`; else `None` |
| `request_log_tags(_)` | empty string |
| `authenticate(request)` | No `x-mystic-host-capability` header → `NotMine`. No `capabilities` → `Refused(401)`. Else `HostCapabilities::resolve_runtime(Some(header), now)`: an error maps per §9.5 and is returned as `Err`; `None` → `Refused(401)`; then the domain's runtime identity with no signed user and no override (`None` → `Refused(401)`); then `runtime_auth` (`None` → `Refused(401)`) → `Authenticated(auth)` |
| `dedicated_hunts_disabled()` | `policy.dedicated_hunts_disabled` |
| `filters_offline_party_members(mode, hunt)` | The dedicated default |
| `restore_candidate(player)` | No hosting → the dedicated default; else `Hosting::restore_matchmaking_candidate(player)` |
| `candidate_status(player, current)` | No hosting → the default; else `poll_candidate_status(player, &current)` |
| `party_instance`, `cancel_candidate`, `party_member_left`, `operations_policy_fields_valid`, `operations_overview`, `release_gates` | The dedicated default |
| `route_hunt(context)` | No hosting → the default. Else the realm id is the player's active realm presence from the directory (any error or absence reads as none), then `route_matchmaking_hunt(&context, realm_id)` |
| `launcher_policy_fields()` | Two pairs, in order: `("p2pEmergencyStop", Json("false"))` and `("coopHunts", Json)` exactly `{"enabled":<policy.enabled>,"maxPartySize":<policy.max_party_size>,"forceSdr":<policy.force_sdr>,"trustTier":<policy.trust_tier>}` |

The 401 reply of `authenticate`: status 401, body exactly
`{"code":"unauthorized","message":"Invalid host capability.","payload":null}`.

### 10.4 `RealmsEntry`

| Method | Answer |
| --- | --- |
| `authorize_game_session_realm(account, realm: Option<Json>)` | No directory → the default. A given realm that is not valid JSON → `Refused(REALM_VALIDATION_FAILED, "realm must be a JSON object.")`. Else `RealmDirectory::authorize_game_session_realm(account, parsed, realms_enabled)`; `None` → `None`. A `(wire_id, is_host)` answer becomes `RealmEntry { realm, is_host }` where `realm` is the domain's seam realm id of the wire id on this node, or else the wire id parsed as a `RealmId`, or else `Unimplemented(Realms, "authorize_game_session_realm")` |
| `bind_realm_presence_at_login(account, realm)` | No directory → `Ok(())`. Else `bind_presence_at_login(account, realm's wire id, realms_enabled)` |
| `touch_presence(account)` | No directory → `Ok(())`. Else `touch_presence(account)`, its value dropped |
| `find_by_hub_room_id(room)` | No directory → `None`. Else the found realm's id converted as in the first row (an unconvertible id → `None`) |
| `is_member(realm, account)`, `is_banned(realm, account)` | No directory → `false`. Else the directory's check on the realm's wire id |
| `hub_room_id(realm, kind)` | When realms are off **and** there is no directory → the default. Else the domain's hub room id of the realm's wire id and kind |

### 10.5 `realms_enabled_or_default`

`pub fn realms_enabled_or_default(settings: &dyn rifts_seam::Settings) -> bool`: when `MYSTICPARADOX_REALMS_MODULE`
is set to non-blank text, the domain's realms-module switch decides (it treats `0`, `false`, `off`, `no` as off);
when unset or blank, `true`.

---

## 11. Ports (`ports`)

What realms take from the server around them. Isles supplies the real ones; tests use the plain ones here.

| Item | Contract |
| --- | --- |
| `trait Clock: Send + Sync { fn now(&self) -> UnixMillis; }` | |
| `SystemClock` (`Debug, Clone, Copy, Default`) | The domain's `time::now()` |
| `ManualClock` (`Debug`) | `at(now) -> Self`, `set(now)`, `advance(millis: i64)`; `now()` reads the stored value. Interior mutability, shareable across threads |
| `trait RateLimiter: Send + Sync { fn is_limited(&self, key: &str, max: u32, window_ms: i64) -> bool; }` | Isles' per-key budget |
| `trait IdSource: Send + Sync { fn realm_id(&self) -> String; fn invite_code(&self) -> String; }` | |
| `RandomIds` (`Debug, Clone, Copy, Default`) | `realm_id`: a random lowercase UUID v4; `invite_code`: the domain's invite code generator |
| `trait Accounts: Send + Sync` | `display_names(&self, account_ids: Vec<String>) -> BoxFuture<'_, Result<BTreeMap<String, String>, String>>` (best effort, unknown accounts absent); `resolve_username(&self, username: String) -> BoxFuture<'_, Result<Option<String>, String>>` (the account whose launcher username this is, trimmed and case-insensitive) |
| `FixedAccounts { pub names: BTreeMap<String, String>, pub usernames: BTreeMap<String, String> }` (`Debug, Clone, Default`) | `display_names`: the known subset. `resolve_username`: looks up the lowercased input in `usernames` (keys are lowercase) |
| `trait RealmEventSink: Send + Sync { fn emit(&self, event: RealmEvent) -> BoxFuture<'_, Result<(), String>>; }` | A failure is logged by the caller, never the request's |
| `RecordedEvents` (`Debug, Default`) | Keeps every event; `take()` drains them, `all()` clones them; a poisoned lock is recovered |
| `trait RealmsLog: Send + Sync { fn info(&self, line: &str); fn warn(&self, line: &str); }` | Nothing logged carries a secret or an address |
| `StderrLog` | Writes `info <line>` / `warn <line>` to stderr |
| `QuietLog` | Discards |
| `HealthyGuard { role: lifecycle::GuardRole, guard_session_id: String, manifest_sequence: i64, last_sequence: i64, risk_score: f64 }` (`Debug, Clone, PartialEq`) | A healthy Launcher Guard session (isles keeps Launcher Guard) |
| `trait GuardSessions: Send + Sync { fn find_healthy(&self, account: String, guard_session_id: String, role: GuardRole) -> BoxFuture<'_, Result<Option<HealthyGuard>, String>>; }` | |
| `NoGuardSessions` | Always `Ok(None)` |
| `trait OperationsView: Send + Sync` | `emergency_stop(&self) -> BoxFuture<'_, Result<bool, String>>`; `ice_transport(&self) -> bool` (the `ICE` co-op transport, the default); `guard_refuses(&self) -> bool` (enforce mode with the Guard rule on) |
| `DefaultOperations { pub stopped: AtomicBool }` (`Debug, Default`) | `emergency_stop` reads `stopped`; ICE true; refuses false |
| `CoreAccounts(pub Arc<dyn IslesCore>)` | `display_names`: one `IslesCore::display_name` call per id, keeping only `Ok(Some(name))`; never errs. `resolve_username`: `IslesCore::find_by_display_name(username)`, the account id; an error becomes its display text |
| `CoreGuardSessions(pub Arc<dyn IslesCore>)` | `IslesCore::healthy_guard_session(account)`; anything but `Ok(Some(json))` or unparseable JSON → `Ok(None)`. Reads `guardSessionId` (string, else empty): non-empty and different from the asked id → `Ok(None)`. `manifestSequence` (integer, else 0), `lastSequence` or else `sequence` (integer, else 0), `riskScore` (number, else 0.0). The answer carries the asked role, the stored id (or the asked one when the stored is empty) |
| `CoreOperationsView(pub Arc<dyn IslesCore>)` | `emergency_stop`: `IslesCore::operations_policy()` JSON's `p2pEmergencyStop` boolean; any failure or absence → `false`. `ice_transport`: true. `guard_refuses`: `IslesCore::guard_mode() == GuardMode::Enforce` |

---

## 12. The stub (`Stub`) and the 501 envelope

### 12.1 `Stub`

`pub struct Stub` (`Debug, Clone, Copy, Default`). Every operation answers `Unimplemented(Component::Realms,
"<operation>")`, the operation being the trait method's own name (`runtime_safety_issues`, `startup_summary`,
`start`, `stop`, `authenticate`, `dedicated_hunts_disabled`, `filters_offline_party_members`, `restore_candidate`,
`candidate_status`, `party_instance`, `route_hunt`, `cancel_candidate`, `party_member_left`,
`launcher_policy_fields`, `operations_policy_fields_valid`, `operations_overview`, `release_gates`,
`authorize_game_session_realm`, `bind_realm_presence_at_login`, `touch_presence`, `find_by_hub_room_id`, `is_member`,
`is_banned`, `hub_room_id`; for `HuntLifecycle`: `host_bootstrap`, `route_ready`, `host_ready`, `abort`, `end`,
`status`, `observe`, `start_hub`, `stop_hub`, `hosting_status`, `set_emergency_stop`, `resolve_launch_manifest`),
with these exceptions: `tables()` answers `TABLES`; `account_deletion_steps()` answers empty (a stub stores nothing);
`metric_class` answers `None`; `request_log_tags` answers empty text. It implements `Lifecycle`, `RealmsHooks`,
`RealmsEntry` and `HuntLifecycle` (the last is the domain spec's trait; its error type is `RealmsError`).

### 12.2 The envelope

The 501 is written by `rifts_seam::Reply::not_implemented(&unimplemented, request_id)` (seam spec), which isles calls
for every route a stub holds: status 501, body exactly

```text
{"error":{"code":"NOT_IMPLEMENTED","message":"rifts-realms: <operation> is not implemented yet","requestId":"<id>"}}
```

with the message and request id JSON-escaped. For a route the operation is its `handler` string, so for example
`rifts-realms: realmsService.ts:RealmsService.join is not implemented yet`.

---

## 13. Tests that define the behaviour

### 13.1 Harness (`tests/common`)

- **Targets.** Each repository test runs once per target: SQLite always (a fresh temporary directory with
  `realms.db`), and Postgres when `REFORGED_TEST_POSTGRES_URL` is set. Without it the test prints
  `REFORGED_TEST_POSTGRES_URL is not set: the Postgres half is skipped` to stderr and runs SQLite only.
- **The Postgres switch.** `REFORGED_TEST_POSTGRES_URL` is a `postgres://user:password@host:port/database` URL of a
  server where the tests may create databases (CI: `postgres://postgres:postgres@127.0.0.1:5432/postgres`). It is
  parsed as a URL (an unparseable value fails the test); host default `127.0.0.1`, port default 5432, `allow_remote`
  false. Each target creates a database named `rifts_test_<pid>_<counter>_<subsec nanos>` through an admin
  connection (retrying the connection up to 100 times, 100 ms apart, because parallel tests can fill a small server's
  slots; still failing is a test failure), and drops it with `DROP DATABASE IF EXISTS .. WITH (FORCE)` when the
  target is dropped. Same convention as `store`'s own tests.
- **Open.** `RealmsDb::open(&config, &DatabaseOptions::default(), NodeId "local")`.
- **Fixtures.** `t(iso)` parses a Node ISO time; `actor()` is `Actor::System("test")`; a hunt session builder with
  Node's test defaults (generation 1, version 1, party `party-1`, hunt `ShatteredIsles_IslandA`, build 647472,
  bridge protocol 1, virtual port 1, host `host` with steam id `76561198000000101`, bridge `bridge`, created at 0,
  expires far in the future, everything optional absent); a participant builder (steam id `76561198000000101` for
  `host`, `76561198000000102` otherwise, candidate `<account>-candidate`, optional route ready: bridge
  `<account>-bridge`, port 55000, ready at 1787587208000).
- **The directory kit.** A `RealmDirectory` over the database with: a no-limit rate limiter; `T0 = 1791892800000`
  (2026-10-13T12:00:00.000Z); counting ids (`realm-1`, `realm-2`...; invite codes `ABCDEFGHJK`, `ABCDEFGHJL`... valid
  and distinct); a listed-codes id source (then `ZZZZZZZZZZ`); a key-recording limiter; failing names and a failing
  event sink; fixed display names (`host-1` Hosty, `host-2` Hosty Two, `friend-1` Friendly, `friend-2` Pal) and
  usernames (`pal`, `friendly`, `hosty`); `serve(host, realm)` heartbeats and sets the hub `RUNNING` with session
  `hub-<realm>`; `code(result)` gives the refusal code or `ok`. The kit belongs to the domain spec's directory tests
  but lives here; rewrite it with them.

### 13.2 Suites to rewrite (all on both engines unless said)

`tests/realms_repository.rs` (Node's `realmsRepositoryContract.ts`, `realmPresenceRepositoryContract.ts`): round trip
with optional fields absent until written; name and host each unique; racing name claims make one realm; settings
and heartbeats only on the host's own realm; members added once, never past the ceiling, never when banned; racing
joins never overfill; double join adds once; removing a member never removes the host; a ban removes the member, is
recorded once, has a ceiling, can be lifted; `find_by_member` and the listing order; name release only when not
online since read, with invites; the host releases its own realm with invites, nobody else can; invite round trip,
unique code, newest first; consumption until used up, expired or revoked; racing joins never over-use an invite;
revoke once, first time kept, only for its realm; open invites counted at a time; `patch_hosting` creates with
defaults, merges, removes, leaves the rest; racing disjoint patches all land; `mark_offline` once and only when
online; `claim_lapsed_hosts` returns each recent lapse once and honours its limit; find by either hub room id, exact
match only; join requests once, refused in order; a ban withdraws only the banned account's request; racing
requests never exceed the bound; player invites once, refused in order; a membership and a ban each remove the player
invite; presence round trip and replacing bind; active strictly before expiry; touch extends and never revives;
remove with and without realm; remove for realm; active counts per realm at a time; racing binds leave one row.

`tests/hub_repository.rs` (Node's `realmHubRepositoryContract.ts`): first entry `PENDING`, epoch 1, round trip;
repeated waiting request reuses and only extends expiry; after expiry or from any non-waiting state a new entry
starts; racing requests for one entry make one row; host row admitted and replaced by a later call; ticket only for
a waiting unexpired entry, keeping the row alive; admission from pending, ticketed or admitted, never from lost or
expired; loopback port recorded for waiting or connected rows only; a lost link ends only the current connection and
dates the row; travelling leaves the hub; list and count by state oldest first, bounded, per generation; newest live
entry across hubs; kick or ban removes only that account's live rows in the realm; release removes every live row;
expired waiting rows become `EXPIRED`, connected and not-yet-due rows stay; hub end turns live rows `LOST`; a batch
takes its seats in one step or nobody is written; a placed batch reports as `request_entry` would; racing batches
never exceed `max_peers`; a present member is never refused and takes no seat; an expired waiting row takes no seat;
seats are per hub and a twice-listed account takes one.

`tests/guard_adapter.rs`: `RealmsHuntSessions` maps a hunt (roster admitted flags, `SUSPECT` with pre-suspect
`RUNNING`, realm, `frozenLoadouts` from the detail's extra fields) and a hub session (no host when the host account is
empty, hub kind) to Guard's `HuntView`; hub present accounts are the `ADMITTED` rows, cached one second and evicted by
`invalidate_hub_presence`; capability by digest answers active then revoked views; Guard over `RealmsHuntSessions` and
`SqlGuardStore` authorises and records a host write, authorises inventory for a frozen loadout and refuses a
stranger, resolves a rate-limit bucket `host:<capability id>` from the header; a database migrated by Guard alone,
then by `RealmsDb::open`, then reopened by both keeps its data.

`tests/stubbed.rs`: with every rifts crate stubbed (`stub_routes()` of admission, relay, realms, guard; `Stub`s),
`Rifts::loaded()` is true, every route is mounted once (60 = RF1's 68 less the 8 Launcher Guard and manifest routes
isles keeps) and answers `Unimplemented`, which `Reply::not_implemented` turns into 501; the realms and Guard hooks
and `touch_presence` answer unimplemented.

Unit tests in the crate: §2.4's two; the route table equals the inventory's realms rows in order; every route of
`stub_routes()` and of `RealmsRoutes` with no services answers `Unimplemented(Realms, handler)` for an anonymous
empty call; `Stub`'s hooks and `HuntLifecycle` answer unimplemented and its `tables()` is `TABLES`.

The MIT crate embeds the route table's expected (method, path) list from §9.1 in its test instead of reading
`inventory.json`, which stays in restricted.

---

## 14. Target MIT crate

- `crates/rifts-realms`, package `rifts-realms`, `license = "MIT"`, lints from the workspace.
- Dependencies, all `workspace = true`: `diesel`, `diesel_migrations`, `module-api`, `rand`, `serde`, `serde_json`
  **with `features = ["preserve_order"]`** (restricted's workspace turns it on globally, dainty-lass's does not;
  control payloads, `detail` and every reply rely on insertion order), `sha2`, `store`, `thiserror`, `tokio`
  (`rt`, `rt-multi-thread`, `time`), and the MIT `rifts-seam`, `rifts-admission`, `rifts-relay`, `rifts-guard`.
  Dev: `tempfile`, `tokio` (`rt-multi-thread`, `macros`), `url`.
- **Seam change:** `[workspace.dependencies]` entries for the five `rifts-*` crates (umbrella D4). Nothing else is
  new; restricted's `diesel` adds `32-column-tables`, which nothing here needs (no row type exceeds 20 columns and
  all queries are by name).
- Files: `src/db.rs`, `src/repo/{mod,realms,hub,hunts}.rs`, `src/routes.rs`, `src/service.rs`, `src/ports.rs`,
  `src/stub.rs`, the root, and the migrations at the same relative paths with the same folder names.

---

## 15. Acceptance

1. Both `up.sql` files create exactly §4.2's objects (names, columns, types, nullability, keys, uniques, the partial
   index, the `COLLATE "C"`), and the folder names are `2026-10-26-000001_rifts_realms`. A database created by the
   AGPL crate opens with the MIT one with no migration pending, on both engines.
2. Every repository method keeps §6-§8's preconditions, order of refusals, ordering of results and returned values,
   and every write is one `Database::write` with its checks inside.
3. The §13.2 suites pass on SQLite and, with `REFORGED_TEST_POSTGRES_URL`, on Postgres.
4. `ROUTES` equals §9.1; a dispatcher answer for each route matches §9.2-§9.5; the JSON of §10 is byte-exact.
5. The public API of §16 exists with the same paths and signatures; restricted's `isles-1-14-7` (`rifts_with_core`,
   `tests/module.rs`, `tests/inventory_reforge_host.rs`) builds against the MIT crate by changing dependencies only.
6. Similarity under 30% per file (`clean-room/similarity.md` §1).

---

## 16. Public API used outside the crate

From `isles-1-14-7` (`src/lib.rs` `rifts_with_core`, `tests/module.rs`, `tests/inventory_reforge_host.rs`) and
rifts' own tests. Isles-core, the launcher and the other rifts crates use none. Items owned by the domain spec are
marked (dom); their signatures are there.

| Path | Used as |
| --- | --- |
| `db::RealmsDb` | `open(&config, &options, NodeId) -> Result<_, RepoError>` (result `.ok()`), `Clone`, `database()`, `node()`, `guard_store()` |
| `db::{SCHEMA, SQLITE_MIGRATIONS, POSTGRES_MIGRATIONS, store_actor, RepoError}` | Public; kept |
| `repo::hunts::HuntRepository` | `new(db)`, `create_hunt(&Actor, &HuntSession).await`, `find_hunt(&str).await` |
| `repo::hub::{RealmHubRepository, EntryAt, HubEntryRequest, ...}`, `repo::realms::*` | Tests |
| `RealmsHuntSessions` (dom: capabilities) | `new(db, Arc<dyn Clock>)`, `with_guard_clock(db, Arc<dyn rifts_guard::Clock>)`, `Arc<dyn rifts_guard::HuntSessions>` |
| `SqlGuardStore` (re-export of `rifts_guard`) | `SqlGuardStore::new(db.database().clone())` |
| `RealmsRoutes`, `RealmsRoutesParts` | §9.2, as `Arc<dyn RouteHandler>` |
| `RealmsService`, `RealmsServiceParts` | §10.1, as `Arc<dyn RealmsHooks>` and `Arc<dyn RealmsEntry>` |
| `realms_enabled_or_default(&Settings)` | §10.5 |
| `stub_routes()`, `Stub`, `ROUTES` | §9.1, §12 |
| `RealmsError::Refused(Refusal)` | Matched on by isles' capability resolver |
| `ports::{Clock, SystemClock, IdSource, RandomIds, RateLimiter, RealmEventSink, StderrLog, CoreAccounts, CoreGuardSessions, CoreOperationsView}` | §11 (`RandomIds.realm_id()` makes isles' fallback transport secret) |
| `capabilities::HostCapabilities` (dom) | `new(db)`, `check(Option<&str>, &str, CapabilityScope, UnixMillis)`, `mint(...)` |
| `control::ControlChannel` (dom) | `new(db, clock)` |
| `directory::{DirectoryParts, RealmDirectory}` (dom) | Built by isles |
| `leases::{Hosting, HostingParts, HostingSettings}`, `HostingRuntime` (dom) | Built by isles |
| `lifecycle::RealmsRecovery` (dom) | `new(db, core, clock, log)` |
| `host_policy::{ArtifactSources, read_artifact_policy}`, `hunt::{ArtifactHashes, HuntSession, HuntState, Participant, AdmissionState}`, `ids::seam_session_id`, `realm::RealmLimits`, `selection::derive_ice_peer_id`, `tiers::HostingPolicy`, `time::{now, iso}` (dom) | Isles and its tests |

Public size of this half: 3 repositories plus `HuntTx` (≈100 methods), 9 public record/outcome types, `RealmsDb`,
`RepoError`, `RealmsError`, 4 constants (`TABLES`, `ROUTES`, `LIVE_HOST_STATES`, the migration triple counted once),
`RealmsRoutes`/`Parts`, `RealmsService`/`Parts`, `Stub`, 2 functions, and §11's 9 traits and 13 types.

---

## 17. Risks and ambiguities

1. **Wire names carrying Mystic Paradox's name** (umbrella D3): the paths `/mystic/v1/hunts/:sessionId/...`, the
   header `x-mystic-host-capability`, and the setting `MYSTICPARADOX_REALMS_MODULE`. Keep byte for byte unless the
   umbrella decides otherwise.
2. **Handler strings name Node files** (`p2pAdmission.ts:MarkHostReady`, `realmsService.ts:RealmsService.join`). They
   reach clients inside the 501 message and must equal restricted's `inventory.json`. Recommendation: keep them while
   any route can still answer from a stub; the umbrella may later replace them with neutral operation names together
   with the inventory.
3. **`preserve_order`** must be enabled on `serde_json` in the MIT crate; without it payloads and maps reorder.
4. **No purge runs.** `purge_expired` (realms and hunts) and `purge` (hub) have no caller and `start()` does nothing,
   so expired invites, presence, control messages, host presences and hub rows accumulate. This is the source's
   behaviour; keep it, and leave scheduling to a later row.
5. **Route 3 (`end`)** accepts three scopes in turn while RF1's inventory lists only `ABORT`. The Rust is the
   contract here.
6. **Query values are not percent-decoded** (Express decodes). Clients send plain integers and keys today; keep it,
   flag it.
7. **Order ties.** `list_live_realm_hunts` orders by `created_at` alone and `list_transitions` by `at, state_version`;
   equal values come back in engine order. Adding a final `session_id`/`transition_id` key is safe and recommended.
8. **`update_offer` can collide** with the partial unique index if two offers of one generation are both moved to
   `ACCEPTED`; the second surfaces as `RepoError::Query`, not `Duplicate` (Node's E11000 likewise).
9. **The `db` doc says isles folds `SCHEMA` into its own**; isles actually opens the database three times
   (`rifts_guard::SCHEMA`, `RealmsDb::open`, `isles_core::db::SCHEMA`). Keep `SCHEMA` public either way.
10. **`RealmsError::Database` becomes code `INTERNAL`**; its HTTP status depends on isles' registry (the source's
    root comment says 500).
11. **Race guarantees depend on `store` serialising writes**: on Postgres, only writers that go through
    `Database::write` with the same `postgres_schema` share the advisory lock. Realms must open its handle with the
    default (`None`) schema, as today.
12. **`hub_room_id`** falls back to the default only when realms are off and no directory exists; with realms off but
    a directory present it still answers a room id. Preserved as observed.
