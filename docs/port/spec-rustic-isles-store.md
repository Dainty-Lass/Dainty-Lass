<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false, "code_blocks": false } } -->

# Clean-Room Behaviour Specification: `crates/rustic-isles-store` (RD27)

> **Scope and purpose (RD27):** this is the clean-room behaviour specification for an MIT crate, `rustic-isles-store`,
> the Rustic Isles database layer: the database handle on dainty-lass's `store` (both engines, the change log, soft
> deletes, the actor mapping, the home check), every migration of both engines, every repository trait with its SQL
> behaviour, and the adapter that implements `rustic_isles_auth::AuthStore`. It is unit 2 of
> [`spec-rustic-isles-core.md`](spec-rustic-isles-core.md) §16 and the store that core's composer (§11.4) and auth's
> `Auth` (auth spec §9, §11.2) are given. Its source is dainty-restricted's `modules/rustic-isles/crates/isles-core`
> files `src/db.rs`, `src/repo/*` and `migrations/*`, which are AGPL-3.0-only with Mystic Paradox's section 7 terms;
> an implementer who never sees them writes the MIT crate from this document alone.
>
> Everything below is stated from observable contracts: the schema an existing database already has (a fact the
> crate must match exactly, given as tables, not as SQL files), each operation's inputs, outputs, atomicity, ordering
> and errors, the JSON text it stores, the public names other crates are written against, and the tests. No
> implementation code is reproduced and no private helper is named. Node names appear only as provenance.
>
> Written against dainty-restricted `e0e238b4434316bb210677d88bc59e3448688794` (worktree
> `splashloader-integration-90130d`; the files of this unit last changed in `2ae16c9`, 2026-11-01) and dainty-lass
> `f9cb7b4b26309c7cd2d9ca9ea03bc93a65417978` (branch `feat/isles-auth-clean-room`, carrying the in-progress
> `rustic-isles-auth` whose `AuthStore` trait §12 implements).

---

## 1. Provenance and licensing classification

### 1.1 Evidence used

| Source | What it says |
| --- | --- |
| `modules/rustic-isles/NOTICE.md` | The module is a modified Mystic Paradox; Paradox's own list includes "MongoDB persistence layer (migration off SQLite) and repository contracts". Every file is AGPL-3.0-only with the section 7 terms |
| File headers (`//!`) | `repo/mod.rs`: "One trait per Node repository (`persistence/contracts/*.ts`)"; `parties/sql.rs`: "a port of `SqlitePartyRepository`"; `characters/slots.rs`: `loadoutSlots.ts`; `reforged.rs`: `admin.ts`, `launcherNotices.ts`, `operationsPolicy.ts`, `launcherGuard.ts`, `reforgedApi.ts`, `src/activity`; `db.rs`: DD15, DD23, DN2 |
| `git log` of the files | 27 commits, one author (the maintainer): `19ee75d` (RU4 seams), `7ab8e4e` .. `0c6747b` (RU5 areas), `0a29728` (RU8 replay scope), `66601f2`, `e33dff5`, `b909a45` (DN5, timeouts), `2ae16c9` (merge of timeouts) |
| `crates/isles-core/tests/{store,auth_repos,characters,friendships,inventory_store,progression_repo,bounties_repos,parties}.rs` | The authored tests (described in §15, not copied) |
| `crates/isles-core/src/module.rs` (`open_store`), `areas/auth/*` (how each store call is made), `areas/auth/erasure.rs` (the erasure transaction) | Read for the seam only: how the store is opened and which actor and write path each auth call uses (§12) |
| `legacy/crates/legacy-import/src/target.rs` | Requires the migration versions `0001`, `0020`, `0200` and `0610` in `__diesel_schema_migrations` (§4.1) |

### 1.2 Classification and what is rewritten

Category A is code written for DaintyLass with no Paradox code in it; Category B ports Paradox TypeScript. The schema
is an interface fact (existing databases have it) and is restated here as facts; everything else is rewritten.

| Restricted file (`crates/isles-core/`) | Lines | Cat. | Source | Rewritten as |
| --- | ---: | :-: | --- | --- |
| `src/db.rs` | 341 | A (the home-API adapter part is core's) | DD15, DD17, DD23, DN2, RU8 | §3 (handle), §2.3 (actors); the context home adapter is core §5.4, not here |
| `src/repo/mod.rs` | 203 | mixed | RU4 shape; the repository list is Paradox's `RepositoryProvider` | §5 |
| `src/repo/auth.rs` | 1869 | B | `launcherAccounts`, `authIdentities`, `refreshSessions`, `gameExchangeCodes`, `discordOAuthTransactions`, `emailCodes`, `apiKeys` contracts | §6 |
| `src/repo/characters.rs`, `characters/slots.rs` | 410 + 177 | B | `CharacterRepository`, `LoadoutRepository`, `loadoutSlots.ts` | §7 |
| `src/repo/chat.rs` | 212 | B | `FriendshipRepository` | §8 |
| `src/repo/inventory.rs` | 424 | B | `InventoryRepository`, `InventoryTransactionRepository` | §9 |
| `src/repo/progression.rs`, `progression/sql.rs` | 177 + 1039 | B | progression, grant, escalation, journey, breadcrumb, encountered-content contracts | §10 |
| `src/repo/bounties.rs` | 832 | B | bounty/cooldown state, entitlements, wallets, economy rollups | §11.1 |
| `src/repo/parties.rs`, `parties/sql.rs` | 271 + 921 | B | `PartyRepository.ts`, `SqlitePartyRepository` | §11.2 |
| `src/repo/reforged.rs` | 2423 | B | admin, notices, operations policy, activity, Launcher Guard, invites | §11.3 |
| `migrations/{sqlite,postgres}/*/{up,down}.sql` | 1196 (30 files) | schema facts | DaintyLass tables shaped after Node's collections | §4 (restated as tables) |
| (none: new) | | | auth spec §11.3 | §12, the `AuthStore` adapter, specified fresh (restricted has none yet) |

### 1.3 What stays in dainty-restricted

- The areas that call the repositories (`src/areas/*`), the composer that puts the store in core's slots (core
  §11.4), the context home adapter (core §5.4), `AuthReforgedCore` and the other auth seams (auth §1.3), the erasure
  seam's blockers (presence and matchmaking: auth's `Erasure` seam), `isles-db` (migrate, verify, export, replay),
  the legacy importer, and every golden.
- Until the areas are ported, restricted's areas call this crate's repositories in place of `src/repo/*`; then
  `src/db.rs`, `src/repo/*` and `migrations/*` are deleted there.

---

## 2. Conventions

### 2.1 Text, times, JSON

- **Times** stored as text are `Date#toISOString`'s form (`2026-10-27T00:00:00.000Z`): fixed width, so text order is
  time order and every "before", "after", "due" comparison below is a text comparison on the stored column. Times
  stored as integers (`deleted_at`, `ttl_at`) are Unix milliseconds.
- **`now` inside the store**: where an operation stamps a time itself (progression's `updated_at`), it is
  `to_iso_string(now_ms())` of core §9.1, so the request's replay scope applies (§3.3). Everywhere else the caller
  passes the time.
- **JSON columns** hold compact `serde_json` text (no spaces) with key insertion order kept (the crate enables
  `serde_json`'s `preserve_order` and `float_roundtrip` in its own manifest, core §17 item 8). Where a row is
  re-serialized the JavaScript way it says so (`service_kit::jsjson::stringify`, loadouts and the inventory ledger).
  Reading parses with `serde_json` unless stated.
- **Numbers written from `f64`** keep `serde_json`'s float text: an activity counter or rollup value of 1 is stored
  and returned as `1.0` (§16 item 9).
- **Ids**: an account is keyed by its global id text `account:<ULID>@<node>` (DD18; core §2.2 `AccountId`); the
  client's `userId` lives only in `accounts.user_id`. A stored account id that does not parse is a data error
  (§2.4). Party ids are global ids of their own kind (`module_api::GlobalId`).

### 2.2 Live rows and soft deletes (DD23)

Sixteen tables are **soft-delete tables**: `accounts`, `launcher_accounts`, `characters`, `loadouts`, `inventories`,
`progression_tracks`, `progression_objectives`, `escalation_progress`, `player_journey`, `breadcrumbs`,
`encountered_content`, `bounty_states`, `cooldown_states`, `entitlements`, `wallets`, `wallet_balances` (this list, in
this order, is the schema's `soft_delete` list given to `store`). A row is **live** while `deleted_at` is null. Unless
an operation says otherwise, reads see live rows only; a soft-deleted row keeps its primary key, so an insert with the
same key collides with it (§16 items 3 to 6 list where that shows). Soft deletes go through `store`'s `Tx::soft_delete`
or set `deleted_at` to the write's time (`Tx::at()`); `store`'s retention purge removes them for good later.

### 2.3 Actors (the change log's names)

Every write names a `module_api::ActorContext`; the change log records `store::Actor`, mapped as:

| `module_api::Actor` | `store::Actor` | Logged as |
| --- | --- | --- |
| `System` | `System("isles")` | `system:isles` |
| `Console` | `Host("console")` | `host:console` |
| `Admin { key }` | `Admin(key)` | `admin:<key>` |
| `Player { account }` | `Player(<account global id text>)` | `player:account:<ULID>@<node>` |
| `Module { module }` | `System(<module id text>)` | `system:<module id>` |

### 2.4 Errors (`RepoError`)

Display texts are observable (logs; the inventory area answers `Unimplemented` as a 501 whose message is
`isles: {display}`), so they are kept exactly:

| Variant | Display |
| --- | --- |
| `Unimplemented { repository: &'static str, operation: &'static str }` | `{repository}: {operation} is not implemented yet` |
| `NotHome { account: AccountId, home: String }` | `{account} is homed on {home}, not here` |
| `Home(String)` | `the account's home could not be resolved: {0}` |
| `Store(store::Error)` | `store::Error`'s own text (transparent) |
| `Query(diesel::result::Error)` | `database: {0}` |
| `Data(String)` | `stored data: {0}` |

`RepoResult<T> = Result<T, RepoError>`. A **data error** below is `Data` with the text given; a stored row that does
not decode (an enum text the crate doesn't know, an account id that does not parse, JSON that does not parse where
JSON is required) is one. Database failures (a constraint violated, a lost connection) are `Query` or `Store`.

---

## 3. The database handle (`IslesStore`)

### 3.1 Opening

`IslesStore::open(config: &store::DatabaseConfig, options: &store::DatabaseOptions, home: Arc<dyn HomeApi>, node:
&NodeIdentity) -> RepoResult<IslesStore>` opens `store::Database` with the schema of §4 (`DatabaseSchema { sqlite,
postgres, soft_delete }`), which brings the database to the latest migration of its engine and puts the change-log
triggers on. It is blocking (call it at start, before serving). The store keeps `node.node` as **this node** and starts
with every repository on SQL (`Repositories::on_store()`, §5).

**Opening from settings** (what restricted's module does today, kept as a helper so the composer has one call):
`IslesStore::open_configured(lookup, home, node) -> RepoResult<Option<IslesStore>>`, where `lookup(name)` is the
module's settings `get` (none when empty, core §6.1):

1. Neither `REFORGED_DB_FILE` nor `REFORGED_DB_ENGINE` set: `None` (no database; core §5.1 "database wanted").
2. `store::DatabaseConfig::from_lookup(lookup, <REFORGED_DB_FILE or empty>)`: its error text is the error.
3. SQLite with a file whose parent folder is not empty: the folder is created (with its parents).
4. Opened with `DatabaseOptions::default()`.

### 3.2 Calls

| Call | Contract |
| --- | --- |
| `with_repos(self, Repositories) -> Self` | The same database with other repositories (tests, and an area's fakes) |
| `database() -> &store::Database`, `repos() -> &Repositories`, `node() -> &NodeId`, `home() -> &Arc<dyn HomeApi>` | Accessors |
| `read(f)` | `f(&mut AnyConnection, &Repositories) -> RepoResult<T>` on `store`'s read pool, in a read-only transaction |
| `write(actor, f)` | `f(&mut Tx<'_>, &Repositories) -> RepoResult<T>` through `store::Database::write` under `store_actor(actor)`; one closure is one transaction (committed on `Ok`, rolled back on `Err`); writes are serial on both engines, so a select-then-update inside one closure is atomic |
| `account_write(account: &AccountId, actor, f)` | The home check (§3.4), then `write(actor, f)` |
| `store_actor(&ActorContext) -> store::Actor` | §2.3 (public: the Reforged API and the auth adapter use it) |

Every closure runs with `f`'s `Send + 'static` bounds; the repositories are cloned into it.

### 3.3 The replay scope inside closures (RU8)

`read`, `write` and `account_write` capture core's current replay scope (`rustic_isles::replay::current_replay_scope()`)
on the calling task and run the closure inside it (`ReplayScope::enter`), so a closure on `store`'s blocking thread
sees the request's pinned clock and seeded ids. Nothing else of the replay crosses: `Tx::at()` (the change log's time
and soft-delete stamps) is `store`'s own clock.

### 3.4 The home check (DD17, DN2)

Before an account write, `home.resolve_home(account.global())`:

| Answer | Result |
| --- | --- |
| `Ok(identity)` and `identity.node` is this node | proceed |
| `Ok(identity)` of another node | `NotHome { account, home: <that node id text> }` (DN5 forwards on that error) |
| `Err(HomeError::UnknownAccount)` and the account id's owner is this node | proceed |
| any other `Err(e)` | `Home(<e's text>)` (for example `this server is not in a network` from a standalone home asked about another node's account) |

The actor is not checked against the account: an admin may make an account write (§15 row S1).

---

## 4. The schema (both engines)

### 4.1 Migrations

Diesel migrations, one folder per migration in each engine's set (`sqlite`, `postgres`), each with `up.sql` and
`down.sql`. The version is the folder's prefix before the first `_`; both sets have the same versions (DF7; `store`
refuses to open otherwise). The versions and folder names are facts an existing database records in
`__diesel_schema_migrations` and must not change:

| Version | Folder | Creates |
| --- | --- | --- |
| `0001` | `0001_accounts` | `accounts` |
| `0020` | `0020_auth` | `launcher_accounts`, `auth_identities`, `refresh_sessions`, `game_exchange_codes`, `discord_oauth_transactions`, `email_codes`, `game_server_api_keys`, `user_api_keys` (in this order) |
| `0030` | `0030_characters_loadouts` | `characters`, `loadouts` |
| `0050` | `0050_inventory` | `inventories`, `inventory_transactions` |
| `0055` | `0055_bounties_store` | `bounty_states`, `cooldown_states`, `entitlements`, `wallets`, `wallet_balances`, `economy_rollup_counters` |
| `0070` | `0070_chat_friend_edges` | `friend_edges` |
| `0200` | `0200_reforged_invites` | `reforged_invites` |
| `0201` | `0201_admin` | `admin_sessions`, `admin_audit` |
| `0202` | `0202_launcher_notices` | `launcher_notices` |
| `0203` | `0203_operations_policy` | `operations_policy` and its one seed row |
| `0204` | `0204_launcher_guard` | `launcher_guard_sessions`, `guard_findings` |
| `0205` | `0205_activity` | `player_activity_daily`, `activity_hunts`, `slayers_path_unlocks`, `activity_flags` |
| `0500` | `0500_progression` | `progression_tracks`, `progression_objectives`, `progression_objective_events`, `progression_grants`, `escalation_progress`, `player_journey`, `breadcrumbs`, `encountered_content` |
| `0600` | `0600_parties` | `parties`, `party_members`, `party_invites`, `party_outbox` |
| `0610` | `0610_timeouts` | three columns added to `launcher_accounts` |

`legacy-import` (restricted) requires `0001`, `0020`, `0200` and checks for `0610`. A new table is a new migration in
both sets with one version, never an edit of a shipped one.

**Down migrations** drop what their `up` created, in reverse order of creation: `0020` drops `user_api_keys`,
`game_server_api_keys`, `email_codes`, `discord_oauth_transactions`, `game_exchange_codes`, `refresh_sessions`,
`auth_identities`, `launcher_accounts`; `0030` `loadouts`, `characters`; `0050` `inventory_transactions`,
`inventories`; `0055` `economy_rollup_counters`, `wallet_balances`, `wallets`, `entitlements`, `cooldown_states`,
`bounty_states`; `0201` `admin_audit`, `admin_sessions`; `0204` `guard_findings`, `launcher_guard_sessions`; `0205`
`activity_flags`, `slayers_path_unlocks`, `activity_hunts`, `player_activity_daily`; `0500` `encountered_content`,
`breadcrumbs`, `player_journey`, `escalation_progress`, `progression_grants`, `progression_objective_events`,
`progression_objectives`, `progression_tracks`; `0600` `party_outbox`, `party_invites`, `party_members`, `parties`;
`0610` drops `timeout_reason`, `timeout_since`, `timeout_until` (column drops); the one-table migrations drop their
table. The two engines' down files are identical.

### 4.2 How to read the tables

- Columns are listed **in table order**; the order is part of the contract (SQLite's `PRAGMA table_info`, the change
  log's row JSON and `db move` see it).
- **Type** is the declared type, spelled as here on both engines: `TEXT`, `BIGINT`, `DOUBLE PRECISION`, `BOOLEAN`
  (SQLite stores it as 0/1). The declared spelling decides SQLite's affinity (`BIGINT` integer, `DOUBLE PRECISION`
  real, `BOOLEAN` numeric), so it is kept as given.
- **Null**: `NN` is `NOT NULL`; blank is nullable.
- **Constraint** is written in the order the column clause carries it: `PK` (inline primary key), `UNIQUE`,
  `DEFAULT x`, `FK accounts(id)` (an inline `REFERENCES accounts (id)`), `ON DELETE CASCADE`.
- **Identity key** (three tables): SQLite `INTEGER PRIMARY KEY AUTOINCREMENT`; Postgres `BIGINT GENERATED ALWAYS AS
  IDENTITY PRIMARY KEY`. This is the only difference between the two engines' `up` files.
- A composite primary key or a multi-column `UNIQUE` is a table constraint after the last column.
- Indexes are separate statements after their table, named as given; "where" is a partial index's predicate.
- The SQL files carry comments (some inside a `CREATE TABLE`, which SQLite keeps in its stored schema text). Nothing
  reads that text; whether the rewrite keeps comments is §16 item 13.

### 4.3 `0001_accounts`

**`accounts`** (the game account; soft-delete)

| Column | Type | Null | Constraint |
| --- | --- | --- | --- |
| `id` | TEXT | | PK |
| `user_id` | TEXT | NN | UNIQUE |
| `name` | TEXT | NN | |
| `notes` | BIGINT | NN | DEFAULT 0 |
| `deleted_at` | BIGINT | | |

### 4.4 `0020_auth` and `0610_timeouts`

**`launcher_accounts`** (soft-delete; the last three columns are added by `0610` with `ALTER TABLE ... ADD COLUMN`,
so they follow `deleted_at`)

| Column | Type | Null | Constraint |
| --- | --- | --- | --- |
| `account_id` | TEXT | | PK, FK accounts(id) |
| `email` | TEXT | | |
| `display_name_normalized` | TEXT | NN | |
| `display_name` | TEXT | NN | |
| `password_hash` | TEXT | | |
| `status` | TEXT | NN | |
| `approval_status` | TEXT | | |
| `approval_updated_at` | TEXT | | |
| `approval_updated_by` | TEXT | | |
| `approval_reason` | TEXT | | |
| `roles` | TEXT | NN | |
| `roles_updated_at` | TEXT | | |
| `roles_updated_by` | TEXT | | |
| `created_at` | TEXT | NN | |
| `last_login_at` | TEXT | | |
| `username_set` | BIGINT | | |
| `consent_terms_version` | TEXT | | |
| `consent_privacy_version` | TEXT | | |
| `consent_accepted_at` | TEXT | | |
| `deletion_requested_at` | TEXT | | |
| `deletion_scheduled_for` | TEXT | | |
| `email_verified_at` | TEXT | | |
| `email_verification_pending_since` | TEXT | | |
| `deleted_at` | BIGINT | | |
| `timeout_until` | TEXT | | (`0610`) |
| `timeout_since` | TEXT | | (`0610`) |
| `timeout_reason` | TEXT | | (`0610`) |

Indexes: `launcher_accounts_email` UNIQUE on (`email`) where `email IS NOT NULL AND deleted_at IS NULL`;
`launcher_accounts_display_name` UNIQUE on (`display_name_normalized`) where `deleted_at IS NULL`. Unique among live
rows only, so a soft-deleted registration frees its address and name.

**`auth_identities`**: `provider` TEXT NN; `provider_subject` TEXT NN; `account_id` TEXT NN FK accounts(id);
`provider_username` TEXT NN; `provider_avatar_url` TEXT; `linked_at` TEXT NN; primary key (`provider`,
`provider_subject`). Index `auth_identities_account` on (`provider`, `account_id`) (not unique).

**`refresh_sessions`**: `id` TEXT PK; `token_hash` TEXT NN; `account_id` TEXT NN FK accounts(id); `family_id` TEXT NN;
`device_id` TEXT NN; `device_name` TEXT NN; `created_at` TEXT NN; `expires_at` TEXT NN; `revoked_at` TEXT. Indexes
`refresh_sessions_token_hash` (`token_hash`), `refresh_sessions_family` (`family_id`, `account_id`),
`refresh_sessions_account` (`account_id`), none unique.

**`game_exchange_codes`**: `code_hash` TEXT PK; `account_id` TEXT NN FK accounts(id); `launcher_session_id` TEXT NN;
`build_changelist` BIGINT NN; `executable_sha256` TEXT NN; `runtime_channel` TEXT NN; `runtime_sha256` TEXT NN;
`runtime_manifest_version` TEXT NN; `runtime_artifact_set_sha256` TEXT NN; `realm_id` TEXT; `created_at` TEXT NN;
`expires_at` TEXT NN; `consumed_at` TEXT. Index `game_exchange_codes_account` (`account_id`).

**`discord_oauth_transactions`**: `state` TEXT PK; `code_verifier` TEXT NN; `created_at` TEXT NN; `expires_at` TEXT
NN; `consumed_at` TEXT; `completion_code_hash` TEXT; `completion_expires_at` TEXT; `completion_consumed_at` TEXT;
`account_id` TEXT FK accounts(id); `link_account_id` TEXT FK accounts(id). Index `discord_oauth_completion`
(`completion_code_hash`).

**`email_codes`**: `account_id` TEXT NN FK accounts(id); `purpose` TEXT NN; `code_hash` TEXT NN; `attempts` BIGINT NN;
`created_at` TEXT NN; `expires_at` TEXT NN; primary key (`account_id`, `purpose`).

**`game_server_api_keys`**: `key_hash` TEXT PK.

**`user_api_keys`**: `account_id` TEXT PK FK accounts(id); `key_hash` TEXT NN. Index `user_api_keys_hash`
(`key_hash`).

### 4.5 `0030_characters_loadouts`

**`characters`** (soft-delete): `character_id` TEXT PK; `account_id` TEXT NN; `ord` BIGINT NN; `created_date` TEXT
NN; `last_modified_date` TEXT NN; `name` TEXT NN; `update_version` BIGINT NN; `data` TEXT NN; `deleted_at` BIGINT.
Index `characters_account` (`account_id`, `ord`). No foreign key.

**`loadouts`** (soft-delete): `character_id` TEXT PK; `account_id` TEXT NN; `loadouts` TEXT NN; `persistent` TEXT NN;
`active_index` BIGINT NN DEFAULT 0; `unlocked_total_slots` BIGINT; `revision` BIGINT NN DEFAULT 0;
`bootstrap_version` TEXT; `deleted_at` BIGINT. Index `loadouts_account` (`account_id`).

### 4.6 `0050_inventory`

**`inventories`** (soft-delete): `character_id` TEXT PK; `account_id` TEXT (nullable: rows from before it was
tracked); `instanced_items` TEXT NN; `stacked_items` TEXT NN; `revision` BIGINT NN DEFAULT 0; `bootstrap_version`
TEXT; `deleted_at` BIGINT. Index `inventories_account` (`account_id`).

**`inventory_transactions`**: `account_id` TEXT NN; `character_id` TEXT NN; `transaction_id` TEXT NN; `request_hash`
TEXT; `status` TEXT NN; `result` TEXT; `created_at` TEXT NN; `completed_at` TEXT; `economy_source` TEXT;
`economy_session_id` TEXT; `extra` TEXT; primary key (`account_id`, `character_id`, `transaction_id`). Index
`inventory_transactions_account` (`account_id`, `created_at`).

### 4.7 `0055_bounties_store`

| Table | Columns in order | Key |
| --- | --- | --- |
| `bounty_states` (soft-delete) | `account_id` TEXT PK; `bounties` TEXT NN; `draft_data` TEXT NN; `daily_reward_credited_on` TEXT; `daily_premium_draft_granted_on` TEXT; `update_version` BIGINT NN; `created_at` TEXT NN; `updated_at` TEXT NN; `deleted_at` BIGINT | inline |
| `cooldown_states` (soft-delete) | `account_id` TEXT PK; `entries` TEXT NN; `update_version` BIGINT NN; `created_at` TEXT NN; `updated_at` TEXT NN; `deleted_at` BIGINT | inline |
| `entitlements` (soft-delete) | `account_id` TEXT NN; `name` TEXT NN; `duration` DOUBLE PRECISION NN; `activated_date` TEXT; `granted_at` TEXT; `source_sku_id` TEXT; `deleted_at` BIGINT | (`account_id`, `name`) |
| `wallets` (soft-delete) | `account_id` TEXT PK; `bootstrap_version` TEXT; `deleted_at` BIGINT | inline |
| `wallet_balances` (soft-delete) | `account_id` TEXT NN; `catalog_id` TEXT NN; `balance` DOUBLE PRECISION NN; `ordinal` BIGINT NN; `deleted_at` BIGINT | (`account_id`, `catalog_id`) |
| `economy_rollup_counters` | `account_id` TEXT NN; `day` TEXT NN; `currency` TEXT NN; `path` TEXT NN; `value` DOUBLE PRECISION NN; `ttl_at` BIGINT NN | (`account_id`, `day`, `currency`, `path`) |

No indexes beyond the keys; no foreign keys.

### 4.8 `0070_chat_friend_edges`

**`friend_edges`**: `id` identity key; `owner` TEXT NN FK accounts(id) ON DELETE CASCADE; `other` TEXT NN (no foreign
key: the friend may live on another node, DD17); `other_user_id` TEXT NN; `status` TEXT NN; `direction` TEXT;
`created` TEXT NN; `favorite` BOOLEAN NN DEFAULT FALSE; `UNIQUE (owner, other)`. Index `friend_edges_other` (`other`).

### 4.9 `0200` to `0205` (the Reforged API)

**`reforged_invites`**: `code` TEXT PK; `uses_remaining` BIGINT NN; `infinite_uses` BOOLEAN NN; `note` TEXT;
`created_at` TEXT NN.

**`admin_sessions`**: `id` TEXT PK; `token_hash` TEXT NN UNIQUE; `account` TEXT NN; `csrf_token` TEXT NN;
`created_at` TEXT NN; `expires_at` TEXT NN; `ip` TEXT NN; `user_agent` TEXT NN; `revoked_at` TEXT; `ttl_at` BIGINT NN.

**`admin_audit`**: `seq` identity key; `id` TEXT NN UNIQUE; `actor_user_id` TEXT NN; `target_user_id` TEXT; `action`
TEXT NN; `old_state` TEXT; `new_state` TEXT; `reason` TEXT; `ip` TEXT NN; `request_id` TEXT NN; `created_at` TEXT NN.
Index `admin_audit_target` (`target_user_id`).

**`launcher_notices`**: `id` BIGINT PK; `version` BIGINT NN; `updated_at` TEXT; `updated_by` TEXT; `announcements`
TEXT NN; `tags` TEXT NN. (One row, `id` 1, made by the first save.)

**`operations_policy`**: `id` BIGINT PK; `guard_enforcement` TEXT NN; `diagnostics_profile` TEXT NN; `version` BIGINT
NN; `updated_at` TEXT; `updated_by` TEXT; `diagnostics_expires_at` TEXT. The migration inserts the one row: `id` 1,
`guard_enforcement` `ENFORCE`, `diagnostics_profile` `PRODUCTION`, `version` 0, the rest null.

**`launcher_guard_sessions`**: `guard_session_id` TEXT PK; `account` TEXT NN; `role` TEXT NN; `public_key_base64` TEXT
NN; `manifest_id` TEXT NN; `manifest_sequence` BIGINT NN; `channel` TEXT NN; `challenge` TEXT NN; `last_sequence`
BIGINT NN; `status` TEXT NN; `risk_score` DOUBLE PRECISION NN; `created_at` TEXT NN; `expires_at` TEXT NN;
`last_heartbeat_at` TEXT; `detail` TEXT NN; `ttl_at` BIGINT NN. Indexes `launcher_guard_sessions_account` (`account`,
`role`, `status`), `launcher_guard_sessions_created` (`created_at`).

**`guard_findings`**: `id` identity key; `file_name` TEXT NN; `sha256` TEXT NN; `signer` TEXT NN; `size` BIGINT NN;
`first_seen_at` TEXT NN; `last_seen_at` TEXT NN; `session_count` BIGINT NN; `account_count` BIGINT NN; `account_ids`
TEXT NN; `ttl_at` BIGINT NN; `UNIQUE (sha256, file_name)`.

**`player_activity_daily`**: `account` TEXT NN; `day` TEXT NN; `counters` TEXT NN; `ttl_at` BIGINT NN; primary key
(`account`, `day`). Index `player_activity_daily_day` (`day`).

**`activity_hunts`**: `id` TEXT PK; `account` TEXT NN; `host` BOOLEAN NN; `kind` TEXT NN; `hunt_id` TEXT NN; `hunt_ref`
TEXT NN; `started_at` TEXT NN; `ended_at` TEXT; `end_reason` TEXT; `result` TEXT; `kills` BIGINT NN; `ttl_at` BIGINT NN.
Index `activity_hunts_account_started` (`account`, `started_at`).

**`slayers_path_unlocks`**: `id` TEXT PK; `account` TEXT NN; `node_id` TEXT NN; `unlocked_at` TEXT NN; `source` TEXT
NN; `ttl_at` BIGINT NN. Index `slayers_path_unlocks_account_unlocked` (`account`, `unlocked_at`).

**`activity_flags`**: `id` TEXT PK; `kind` TEXT NN; `severity` TEXT NN; `account` TEXT NN; `day` TEXT NN; `detail` TEXT
NN; `acknowledged` BOOLEAN NN; `created_at` TEXT NN; `acknowledged_at` TEXT; `acknowledged_by` TEXT; `note` TEXT;
`ttl_at` BIGINT NN. Indexes `activity_flags_created` (`acknowledged`, `created_at`), `activity_flags_account_created`
(`account`, `created_at`).

### 4.10 `0500_progression`

| Table | Columns in order | Key, indexes |
| --- | --- | --- |
| `progression_tracks` (soft-delete) | `account` TEXT NN; `progression_id` TEXT NN; `progress` BIGINT NN DEFAULT 0; `confirmed_fremium_rank` BIGINT NN DEFAULT 0; `confirmed_premium_rank` BIGINT NN DEFAULT 0; `update_version` BIGINT NN DEFAULT 0; `seq` BIGINT NN DEFAULT 0; `created_at` TEXT NN; `updated_at` TEXT NN; `reforge` TEXT; `prestige_claimed` BIGINT; `deleted_at` BIGINT | (`account`, `progression_id`) |
| `progression_objectives` (soft-delete) | `account` TEXT NN; `objective_id` TEXT NN; `value` BIGINT NN DEFAULT 0; `completed_count` BIGINT NN DEFAULT 0; `seq` BIGINT NN DEFAULT 0; `created_at` TEXT NN; `updated_at` TEXT NN; `deleted_at` BIGINT | (`account`, `objective_id`) |
| `progression_objective_events` | `id` TEXT PK; `account` TEXT NN; `raw_body` TEXT NN; `received_at` TEXT NN | inline |
| `progression_grants` | `grant_id` TEXT PK; `account` TEXT NN; `progression_id` TEXT NN; `amount` BIGINT NN; `request_hash` TEXT NN; `status` TEXT NN; `result` TEXT NN; `created_at` TEXT NN | inline; index `progression_grants_account` (`account`, `created_at`) |
| `escalation_progress` (soft-delete) | `account` TEXT NN; `season_id` TEXT NN; `escalation_level` BIGINT NN; `next_level_xp` BIGINT NN; `talents_progress` TEXT NN; `unlock_progress` TEXT NN; `update_version` BIGINT NN; `created_at` TEXT NN; `updated_at` TEXT NN; `deleted_at` BIGINT | (`account`, `season_id`) |
| `player_journey` (soft-delete) | `account` TEXT PK; `nodes` TEXT NN; `update_version` BIGINT NN; `revision` BIGINT NN DEFAULT 0; `deleted_at` BIGINT | inline |
| `breadcrumbs` (soft-delete) | `character_id` TEXT PK; `account` TEXT NN; `items` TEXT NN; `update_version` BIGINT NN; `deleted_at` BIGINT | inline |
| `encountered_content` (soft-delete) | `character_id` TEXT PK; `account` TEXT NN; `content` TEXT NN; `deleted_at` BIGINT | inline |

### 4.11 `0600_parties`

| Table | Columns in order | Key, indexes |
| --- | --- | --- |
| `parties` | `id` TEXT PK; `client_id` TEXT NN UNIQUE; `revision` BIGINT NN; `leader` TEXT NN; `build_id` TEXT NN; `activity` TEXT NN; `active_hunt_session_id` TEXT; `created_at` TEXT NN; `updated_at` TEXT NN | inline |
| `party_members` | `account` TEXT PK; `party` TEXT NN; `position` BIGINT NN; `joined_at` TEXT NN | inline (one party per account); index `party_members_party` (`party`, `position`) |
| `party_invites` | `recipient` TEXT NN; `sender` TEXT NN; `party` TEXT NN; `party_revision` BIGINT NN; `sending_display_name` TEXT NN; `sending_platform` TEXT NN; `status` TEXT NN; `created_at` TEXT NN; `expires_at` TEXT NN; `consumed_at` TEXT; `seq` BIGINT NN | (`recipient`, `sender`) |
| `party_outbox` | `id` TEXT PK; `party` TEXT NN; `party_revision` BIGINT NN; `event_type` TEXT NN; `recipients` TEXT NN; `payload` TEXT NN; `state` TEXT NN; `created_at` TEXT NN; `claimed_at` TEXT; `claim_owner` TEXT; `published_at` TEXT; `seq` BIGINT NN | inline; index `party_outbox_state` (`state`, `created_at`, `seq`) |

### 4.12 Column meanings shared with other crates

The meanings of `accounts` and the auth tables are auth spec §9.1's; the stored enum texts are:

| Column | Values |
| --- | --- |
| `launcher_accounts.status` | `active`, `banned`, `disabled` |
| `launcher_accounts.approval_status` | `pending`, `approved`, `rejected`; null = legacy, read as approved where a filter asks |
| `launcher_accounts.roles` | a JSON array of strings |
| `launcher_accounts.username_set` | 1, 0 or null (legacy) |
| `auth_identities.provider` | always `discord` |
| `game_exchange_codes.runtime_channel` | `stable`, `beta`, `dev` |
| `email_codes.purpose` | `verify`, `reset`, `unlink` |
| `friend_edges.status`, `.direction` | `ACCEPTED`, `PENDING`, `BLOCKED`; `INBOUND`, `OUTBOUND` or null |
| `inventory_transactions.status` | `pending`, `completed` |
| `progression_grants.status` | `applied` |
| `parties.activity` | `IDLE`, `MATCHMAKING`, `HOST_SELECTING`, `STARTING_HUNT`, `IN_HUNT`, `RECOVERING` |
| `party_invites.status` | `PENDING`, `ACCEPTED`, `REVOKED`, `EXPIRED` |
| `party_outbox.event_type`, `.state` | `PARTY_CREATED`, `PARTY_INVITE_CREATED`, `PARTY_MEMBERSHIP_CHANGED`, `PARTY_LEADER_CHANGED`, `PARTY_ACTIVITY_CHANGED`, `PARTY_PRESENCE_CHANGED`, `PARTY_HUNT_STATUS_CHANGED`; `PENDING`, `PROCESSING`, `PUBLISHED` |
| `launcher_guard_sessions.role`, `.status` | `client`, `host`; `CHALLENGED`, `HEALTHY`, `DEGRADED`, `REVOKED`, `EXPIRED` (the first three are **live**) |
| `activity_hunts.kind`, `.result` | `dedicated`, `realm`, `playerHosted`; `completed`, `abandoned` |
| `operations_policy` | `OBSERVE`/`ENFORCE`; `PRODUCTION`/`DEVELOPMENT` (stored as given, not checked here) |

---

## 5. Repositories

### 5.1 Shape

One trait per repository, every method synchronous and returning `RepoResult<_>`: a read takes `&mut AnyConnection`
(the closure's), a write takes `&mut Tx<'_>` (the write's; `Tx::conn()` reaches the connection, and only through a
`Tx` can a row be changed, so the change log sees it). A method's work is part of the caller's transaction; "one
write" below means it needs no other statement to be atomic. Each trait is `Send + Sync`.

`Repositories` (`Clone`) holds one `Arc<dyn Trait>` per field; the field names are the contract the area crates use:

| Field | Trait (§) | Label |
| --- | --- | --- |
| `accounts` | `AccountRepository` (6.1) | `accounts` |
| `launcher_accounts` | `LauncherAccountRepository` (6.2) | `launcherAccounts` |
| `auth_identities` | `AuthIdentityRepository` (6.3) | `authIdentities` |
| `refresh_sessions` | `RefreshSessionRepository` (6.4) | `refreshSessions` |
| `game_exchange_codes` | `GameExchangeCodeRepository` (6.5) | `gameExchangeCodes` |
| `discord_oauth_transactions` | `DiscordOAuthTransactionRepository` (6.6) | `discordOAuthTransactions` |
| `email_codes` | `EmailCodeRepository` (6.7) | `emailCodes` |
| `api_keys` | `ApiKeyRepository` (6.8) | `apiKeys` |
| `admin` | `AdminRepository` (11.3.1) | `admin` |
| `invites` | `InviteRepository` (11.3.5) | `reforgedInvites` |
| `launcher_notices` | `LauncherNoticesRepository` (11.3.2) | `launcherNotices` |
| `activity` | `ActivityRepository` (11.3.3) | `activity` |
| `launcher_guard` | `LauncherGuardRepository` (11.3.4) | `launcherGuard` |
| `characters` | `CharacterRepository` (7.1) | `characters` |
| `loadouts` | `LoadoutRepository` (7.2) | `loadouts` |
| `inventories` | `InventoryRepository` (9.1) | `inventories` |
| `inventory_transactions` | `InventoryTransactionRepository` (9.2) | `inventoryTransactions` |
| `progression_tracks` | `ProgressionTrackRepository` (10.2) | `progressionTracks` |
| `progression_grants` | `ProgressionGrantRepository` (10.3) | `progressionGrants` |
| `escalation_progress` | `EscalationProgressRepository` (10.4) | `escalationProgress` |
| `player_journey` | `PlayerJourneyRepository` (10.5) | `playerJourney` |
| `breadcrumbs` | `BreadcrumbRepository` (10.6) | `breadcrumbs` |
| `encountered_content` | `EncounteredContentRepository` (10.6) | `encounteredContent` |
| `bounty_states` | `BountyStateRepository` (11.1) | `bountyStates` |
| `cooldown_states` | `CooldownStateRepository` (11.1) | `cooldownStates` |
| `entitlements` | `EntitlementRepository` (11.1) | `entitlements` |
| `wallets` | `WalletRepository` (11.1) | `wallets` |
| `economy_rollups` | `EconomyRollupRepository` (11.1) | `economyRollups` |
| `parties` | `PartyRepository` (11.2) | `parties` |
| `friendships` | `FriendshipRepository` (8) | `friendships` |

- `Repositories::on_store()`: every field the SQL implementation of this crate (one public unit type per
  implementation; the progression ones are one type implementing six traits).
- `Repositories::stubs()`: every field `Stub`.
- **`Stub`** (public, `Default`, `Copy`) implements every trait; each method answers
  `Unimplemented { repository: <label>, operation: <method name> }`. Tests and area fakes start from it
  (`stubs()` with some fields replaced, then `IslesStore::with_repos`).

### 5.2 Order without an `ORDER BY`

Where a read below says "engine order", the rewrite adds no ordering and the caller must not rely on one. Everywhere
else the order given is exact, with its tie-breakers.

---

## 6. Accounts and sign-in (`repo::auth`)

Records use core's `AccountId` and `UserId`. `Debug` of any record holding a hash, a code, a verifier or a password
hash prints the other fields only (§14).

### 6.1 `AccountRepository`

`AccountRecord { id: AccountId, user_id: UserId, name: String, notes: i64 }`.

| Method | Contract |
| --- | --- |
| `find_by_user_id(conn, &UserId) -> Option<AccountRecord>` | The live row with that `user_id` |
| `find(conn, &AccountId) -> Option<AccountRecord>` | The live row with that id |
| `create(tx, &AccountRecord) -> ()` | Plain insert of the four fields (`deleted_at` null); a taken id or `user_id` (live or not) is a database error |

### 6.2 `LauncherAccountRepository`

Records:

- `ApprovalStatus { Pending, Approved, Rejected }`, `OperationalStatus { Active, Banned, Disabled }` with `as_str` and
  `parse` (the texts of §4.12; `parse` of anything else is none).
- `LauncherConsent { terms_version, privacy_version, accepted_at }`, `LauncherDeletion { requested_at, scheduled_for }`,
  `AccountTimeout { until, since, reason: Option<String> }`.
- `LauncherAccountRecord { account: AccountId, user_id: UserId, email: Option, display_name_normalized,
  display_name, password_hash: Option, status: OperationalStatus, approval_status: Option<ApprovalStatus>,
  approval_updated_at, approval_updated_by, approval_reason (Options), roles: Vec<String>, roles_updated_at,
  roles_updated_by (Options), created_at, last_login_at: Option, username_set: Option<bool>, consent:
  Option<LauncherConsent>, deletion: Option<LauncherDeletion>, email_verified_at, email_verification_pending_since
  (Options), timeout: Option<AccountTimeout> }`, with `timeout_at(now) -> Option<&AccountTimeout>`: the timeout when
  its `until` text is greater than `now`. `Debug` shows `account`, `user_id`, `display_name`, `status`,
  `approval_status`, `roles`, `timeout`, and the password hash only as `<set>` or none.
- `AdminAccountFilter { approval_status: Option, status: Option, timed_out_at: Option<String>, search: Option<String>,
  skip: u32, limit: u32 }`; `AccessChange { approval_status: Option, status: Option, approval_updated_at,
  approval_updated_by, approval_reason: Option }`; `CreateOutcome { Created, EmailTaken, DisplayNameTaken }`.

**Decoding a row** (joined to its game account for `user_id`): `roles` must parse as a JSON array of strings, else data
error `launcher_accounts.roles "<text>": <parse error>`; `status` must be known, else `launcher_accounts.status
"<text>"`; a non-null `approval_status` must be known, else `launcher_accounts.approval_status "<text>"`. `consent` is
present only when all three consent columns are set, `deletion` only when both deletion columns are, `timeout` only
when `until` and `since` are (the reason may be null). `username_set` is `stored != 0`.

**A live launcher account** is a launcher row with `deleted_at` null joined to its `accounts` row with `deleted_at`
null; the finders and lists see only those.

| Method | Contract |
| --- | --- |
| `find_by_user_id(conn, &UserId)` | Live account whose game account has that `user_id` |
| `find_by_account(conn, &AccountId)` | Live account by id |
| `find_by_email(conn, normalized)` | Live account with exactly that `email` text (the caller normalizes) |
| `find_by_display_name_normalized(conn, normalized)` | Live account with that normalized name |
| `create(tx, &record) -> CreateOutcome` | One write. (1) The record has an email and a live launcher row has that email: `EmailTaken`, nothing written. (2) A live launcher row has the normalized name: `DisplayNameTaken`, nothing written. (3) Unless an `accounts` row with the id exists (live or not), insert it with the record's `user_id`, `name` = the display name, `notes` 0. (4) Insert the launcher row with every field of the record (roles as a JSON array; `username_set` 1/0/null; consent and deletion split into their columns; the timeout columns are not written, so null). `Created` |
| `update_last_login(tx, &AccountId, when)` | Sets `last_login_at`; no live check; a missing row changes nothing |
| `set_username(tx, &AccountId, display_name, normalized) -> CreateOutcome` | A live launcher row of **another** account holds the normalized name: `DisplayNameTaken`, nothing written. Else sets `display_name`, `display_name_normalized`, `username_set` 1 on the launcher row and `name` on the game account; `Created` |
| `set_consent(tx, &AccountId, &LauncherConsent)` | Sets the three consent columns |
| `set_deletion(tx, &AccountId, Option<&LauncherDeletion>)` | Sets both deletion columns, or clears both |
| `list_deletions_due(conn, now, limit: u32) -> Vec` | Live accounts with `deletion_scheduled_for <= now`, ascending by it; `limit` 0 is no limit |
| `set_email_verified(tx, &AccountId, verified_at)` | Sets `email_verified_at` and clears `email_verification_pending_since` |
| `set_password_hash(tx, &AccountId, hash)` | Sets `password_hash` |
| `list_unverified_registrations(conn, pending_before, limit: u32) -> Vec` | Live accounts with `email_verification_pending_since < pending_before` and `email_verified_at` null, ascending by the pending time; `limit` 0 is no limit |
| `delete_unverified_registration(tx, &AccountId, pending_before) -> bool` | Only while the launcher row is not soft-deleted, its pending time is before `pending_before` and it is unverified (the game account's state is not checked): soft-deletes the launcher row and the game account; answers whether the launcher row was soft-deleted. Otherwise `false`, nothing changed |
| `list_for_admin(conn, &AdminAccountFilter) -> (Vec, u64)` | Every live account, newest first (`created_at` descending, then `account_id` descending), then filtered in order: approval (a null approval counts as `Approved`), status, `timed_out_at` (only accounts whose `timeout_at(that time)` is some), `search` (when non-empty: the lower-cased search text is a substring of the lower-cased display name, email or `userId`; plain substring, Unicode lower-casing). The total is the count after filtering; the page skips `skip` and takes `limit` (0 is no limit) |
| `set_access_state(tx, &AccountId, &AccessChange) -> Option<record>` | No live account: none, nothing written. Else sets `approval_updated_at`, `approval_updated_by`, `approval_reason` (the reason, or `""` when none: Node's), then the approval status when given, then the status when given; answers the re-read account |
| `set_timeout(tx, &AccountId, Option<&AccountTimeout>) -> Option<record>` | No live account: none. Else sets (or, with none, clears) all three timeout columns; the re-read account |
| `update_roles(tx, &AccountId, roles, at, by) -> Option<record>` | No live account: none. Else sets `roles` (JSON array), `roles_updated_at`, `roles_updated_by`; the re-read account |
| `erase(tx, &AccountId) -> bool` | Soft-deletes the launcher row and the game account; whether either was live |

### 6.3 `AuthIdentityRepository`

`AuthIdentityRecord { provider_subject, account: AccountId, provider_username, provider_avatar_url: Option, linked_at }`.
Every method acts on `provider = discord` rows only.

| Method | Contract |
| --- | --- |
| `find_by_provider_subject(conn, subject)` | The row with that subject |
| `find_by_account(conn, &AccountId)` | The account's row (engine order if more than one) |
| `create(tx, &record) -> bool` | One write: a row with that subject **or** that account exists: `false`, nothing written; else inserted, `true` |
| `delete_for_account(tx, &AccountId) -> u64` | Deletes the account's rows; how many |

### 6.4 `RefreshSessionRepository`

`RefreshSessionRecord { id, token_hash, account: AccountId, family_id, device_id, device_name, created_at, expires_at,
revoked_at: Option }`; `Debug` shows `id`, `account`, `family_id`, `revoked_at` only.

| Method | Contract |
| --- | --- |
| `create(tx, &record)` | Insert as given |
| `find_by_token_hash(conn, hash)` | The row with that hash, revoked or not (engine order if more than one) |
| `revoke_by_id(tx, id, now)` | Sets `revoked_at` = now on that row, even if already revoked |
| `revoke_family(tx, family_id, now)` | Sets `revoked_at` on the family's unrevoked rows (any account) |
| `revoke_all_for_account(tx, &AccountId, now)` | Sets `revoked_at` on the account's unrevoked rows |
| `delete_all_for_account(tx, &AccountId)` | Deletes the account's rows |
| `is_family_active(conn, family_id, &AccountId, now) -> bool` | Whether a row of that family and account is unrevoked with `expires_at > now` |
| `purge_expired(tx, now) -> u64` | Deletes rows with `expires_at < now`; how many (no job calls it today, §16 item 15) |

### 6.5 `GameExchangeCodeRepository`

`RuntimeChannel { Stable, Beta, Dev }` (`as_str`, `parse`). `GameExchangeCodeRecord { code_hash, account: AccountId,
launcher_session_id, build_changelist: u32, executable_sha256, runtime_channel, runtime_sha256,
runtime_manifest_version, runtime_artifact_set_sha256, realm_id: Option, created_at, expires_at, consumed_at: Option }`;
`Debug` without the code hash.

| Method | Contract |
| --- | --- |
| `create(tx, &record)` | Insert as given (the changelist as an integer) |
| `consume_by_code_hash(tx, hash, now) -> Option<record>` | One write: the row with that hash, unconsumed and `expires_at > now`, gets `consumed_at` = now and is answered with `consumed_at` set; otherwise none. Decoding: unknown channel, data error `game_exchange_codes.runtime_channel "<text>"`; a changelist outside `u32`, `game_exchange_codes.build_changelist` |
| `revoke_unused_for_account(tx, &AccountId, now)` | Sets `consumed_at` = now on the account's unconsumed rows |
| `delete_for_account(tx, &AccountId) -> u64` | Deletes the account's rows; how many |
| `purge_expired(tx, now) -> u64` | Deletes rows with `expires_at < now` |

### 6.6 `DiscordOAuthTransactionRepository`

`DiscordOAuthTransactionRecord { state, code_verifier, created_at, expires_at, consumed_at, completion_code_hash,
completion_expires_at, completion_consumed_at (Options), account: Option<AccountId>, link_account: Option<AccountId> }`;
`Debug` shows `expires_at`, `account`, `link_account` only.

| Method | Contract |
| --- | --- |
| `create(tx, &record)` | Insert as given |
| `consume_by_state(tx, state, now) -> Option` | One write: the row with that state, unconsumed and `expires_at > now`, gets `consumed_at` = now; answered with it set |
| `attach_completion_code(tx, state, code_hash, expires_at, &AccountId)` | Sets `completion_code_hash`, `completion_expires_at` and `account_id` on that row (no condition) |
| `consume_by_completion_code_hash(tx, code_hash, now) -> Option` | One write: a row with that completion hash, `completion_expires_at > now` and completion unconsumed, gets `completion_consumed_at` = now; answered with it set |
| `purge_expired(tx, before) -> u64` | Deletes rows with `expires_at < before` and (no completion expiry, or `completion_expires_at < before`) |
| `delete_for_account(tx, &AccountId) -> u64` | Deletes rows whose `account_id` or `link_account_id` is the account |

### 6.7 `EmailCodeRepository`

`EmailCodePurpose { Verify, Reset, Unlink }` (`as_str`). `EmailCodeRecord { account, purpose, code_hash, attempts: u32,
created_at, expires_at }` (a stored attempt count beyond `u32` reads as `u32::MAX`); `Debug` without the hash. A code
is **live** at `now` with `max_tries` when `expires_at > now` and `attempts < max_tries`.

| Method | Contract |
| --- | --- |
| `replace(tx, &record)` | One write: deletes the (account, purpose) row, inserts this one |
| `peek_live(conn, &AccountId, purpose, now, max_tries) -> Option` | The row when live; a read only |
| `consume(tx, &AccountId, purpose, code_hash, now, max_tries) -> bool` | One write: the row holds this hash and is live: deleted, `true` when exactly one row went; otherwise `false`, nothing changed |
| `reserve_try(tx, &AccountId, purpose, now, max_tries) -> Option` | One write: no row: none. A live row: `attempts` + 1, answered with the new count. An expired or used-up row: deleted, none |
| `remove(tx, &AccountId, purpose, code_hash) -> bool` | Deletes the row only while it holds this hash; whether exactly one went |
| `delete_for_account(tx, &AccountId) -> u64` | Every purpose's row of the account |
| `purge_expired(tx, now) -> u64` | Rows with `expires_at < now` |

### 6.8 `ApiKeyRepository`

`GameServerKeyHash { id: i64, key_hash: Option<String> }` (Node's record shape: `id` is the row's 0-based position in
the read, `key_hash` always some); `UserKeyHash { account: AccountId, key_hash }`; neither shows its hash in `Debug`.

| Method | Contract |
| --- | --- |
| `find_all_game_server_key_hashes(conn) -> Vec` | Every stored hash, engine order |
| `insert_game_server_key_hash(tx, hash)` | Adds it unless stored |
| `replace_game_server_key_hashes(tx, &[String])` | One write: empty list empties the table; otherwise each missing hash is added and every stored hash not in the list deleted |
| `find_all_user_key_hashes(conn) -> Vec` | Every user key with its account (engine order) |
| `insert_user_key_hash(tx, &AccountId, hash)` | One key per account: the account's row is replaced |
| `delete_user_key_hash(tx, &AccountId) -> bool` | Whether a row went |

---

## 7. Characters and loadouts (`repo::characters`)

### 7.1 `CharacterRepository`

`CharacterRecord { character_id, account: AccountId, created_date, last_modified_date, name, update_version: i64,
data: String }` (`data` is the character JSON as the client sent it).

| Method | Contract |
| --- | --- |
| `find_many_by_account(conn, &AccountId) -> Vec` | The account's live characters by `ord` ascending (insertion order) |
| `find(conn, character_id, &AccountId) -> Option` | Live, and only the owner's: another account's character is none |
| `create(tx, &record) -> record` | Inserts with `ord` = (the largest `ord` in the table, any account, deleted rows included, else 0) + 1; answers the record as given |
| `update_data_conditional(tx, character_id, &AccountId, data, effective_version)` | Only a live row of that account whose `update_version < effective_version`: sets `data` and `update_version` = effective. A refused write is not an error and says nothing (Node's `void`) |

### 7.2 `LoadoutRepository`

`LoadoutRecord { character_id, account, loadouts: String, persistent: String, active_index: Option<u32>,
unlocked_total_slots: Option<u32>, revision: Option<i64>, bootstrap_version: Option }` (`loadouts` and `persistent` are
JSON text). A read fills `active_index` and `revision` (an index outside `u32` reads 0) and `unlocked_total_slots`
(null, negative or too large reads none). Every write needs the live row of that character **and** account.

| Method | Contract |
| --- | --- |
| `find(conn, character_id, &AccountId)` | The live row |
| `create(tx, &record)` | Insert: `active_index` default 0, `unlocked_total_slots` default 1, `revision` default 0 |
| `replace_slot_if_revision_matches(tx, character, &AccountId, slot_index, data_json, expected) -> bool` | `false` (nothing written) unless: the row exists; its `loadouts` parses (JavaScript `JSON.parse`, `service_kit::jsjson::parse`) as an array; the visible total (§7.3, from the array length and the stored entitlement) resolves; `slot_index` is below it; `data_json` parses. Then the slot is replaced, the array is written back as `jsjson::stringify` text and `revision` = expected + 1, only if the stored revision is `expected`; whether a row changed |
| `replace_all_and_entitlement_if_revision_matches(tx, character, &AccountId, loadouts_json, unlocked, expected) -> bool` | `false` unless `loadouts_json` parses as an array whose length is 1 to 6, `unlocked` is 1 to 6 and not above the length. Then writes the array (re-stringified), `unlocked_total_slots` and `revision` + 1 at the expected revision |
| `update_persistent_if_revision_matches(tx, character, &AccountId, persistent_json, expected) -> bool` | Writes the text as given and `revision` + 1 at the expected revision |
| `update_active_index_if_revision_matches(tx, character, &AccountId, index, expected) -> bool` | `false` unless the row, the array and the visible total resolve and `index` is below it. If the stored index already is `index`: `true` with nothing written, **whatever the revision** (§16 item 8). Else writes it and `revision` + 1 at the expected revision |

### 7.3 Slot rules (`repo::characters::slots`, `loadoutSlots.ts`)

Constants: `DEFAULT_ACCOUNT_LOADOUT_SLOTS` 1, `MAX_CHARACTER_LOADOUT_SLOTS` 5, `MAX_TOTAL_LOADOUT_SLOTS` 6,
`PLAYER_JOURNEY_LOADOUT_NODE_IDS` `LoadoutSlot_01` .. `LoadoutSlot_05`. `SlotRangeError(String)` displays its text.
A total is **valid** in 1 to 6; an invalid one is `SlotRangeError("{label} must be between 1 and 6")`.

| Function | Contract |
| --- | --- |
| `resolve_visible_total(stored: i64, persisted: Option<i64>) -> Result<u32, SlotRangeError>` | `stored` must be valid (label `Stored loadout slot count`). No persisted value: `max(1, stored - 1)` (a legacy row has one excess slot). Else `persisted` must be valid (label `Persisted unlocked loadout slot count`) and not above `stored` (else `Persisted unlocked loadout slot count {p} exceeds stored count {s}`); answers `persisted` |
| `resolve_requested_total(persisted, requested) -> Result<u32>` | `requested` valid (label `Requested total loadout slot count`); no persisted: `requested`; else `max(persisted (valid, label as above), requested)` (unlocks are monotonic) |
| `resolve_player_journey_total(nodes: Option<&Json>) -> u32` | 1 + the number of the five node ids whose `node_status` reads as the number 2 (`Unlocked`); 1 (`LockedButClaimable`) does not count |
| `resolve_active_index(stored: Option<i64>, visible: u32) -> Result<u32>` | `visible` valid (label `Visible loadout slot count`); the stored index when in `0..visible`, else 0 |
| `wire_counts(entitled: f64) -> (u32, u32, u32, u32)` | `(0, 0, total, 6)`, `total` = `trunc(entitled)` clamped to 1..6 (non-finite reads 0, so 1) |

---

## 8. Friendships (`repo::chat`)

`FriendshipStatus { Accepted, Pending, Blocked }`, `FriendshipDirection { Inbound, Outbound }` (texts of §4.12; an
unknown stored text is a data error `friend_edges.status "<text>"` / `friend_edges.direction "<text>"`).
`FriendEdgeRecord { owner: AccountId, other: AccountId, other_user_id: UserId, status, direction: Option, created,
favorite: bool }`. A stored owner or other that is no account id: `friend_edges.{owner|other} "<text>" is not an
account id`.

| Method | Contract |
| --- | --- |
| `list_for_owner(conn, &owner) -> Vec` | Every edge of the owner by `id` ascending (first-written order) |
| `find(conn, &owner, &other) -> Option` | That one direction |
| `upsert(tx, &record)` | Updates the (owner, other) edge's `other_user_id`, `status`, `direction`, `created`, `favorite` in place (it keeps its list position); none there: inserted at the end |
| `remove(tx, &owner, &other)` | That direction only |
| `remove_all_for_account(tx, &account) -> u64` | Every edge the account owns or is the other end of; how many |

---

## 9. Inventory (`repo::inventory`)

### 9.1 `InventoryRepository`

`InventoryRecord { character_id, account: Option<AccountId>, instanced_items: String, stacked_items: String, revision:
Option<i64>, bootstrap_version: Option }`. A stored account id that does not parse: `inventories.account_id "<text>"
is not an account id`.

| Method | Contract |
| --- | --- |
| `find_by_character_id(conn, character)` | The live row (any account; the caller checks ownership) |
| `create(tx, &record)` | Insert; `revision` default 0 |
| `update_both_if_revision_matches(tx, character, instanced, stacked, expected) -> Option` | The only mutation: on the live row whose revision is `expected`, sets both texts as given and `revision` = expected + 1, then answers the re-read row; no row changed: none |

### 9.2 `InventoryTransactionRepository` (POST /inventory's idempotency ledger)

`LedgerStatus { Pending, Completed }` (`pending`/`completed`; any stored text other than `completed` reads `Pending`).
`InventoryTransactionRecord { transaction_id, account, character_id, request_hash: Option, status, result:
Option<Json>, created_at, completed_at: Option, economy_source: Option, economy_session_id: Option, extra:
Option<Json> }`. `TryBegin { Begun, Existing(Box<record>) }`. `LedgerKey<'a> { account: &AccountId, character_id:
&str, transaction_id: &str }` (`Copy`). `result` and `extra` parse with `serde_json`; a failure is
`inventory_transactions.{result|extra}: <error>`.

| Method | Contract |
| --- | --- |
| `try_begin(tx, key, request_hash, created_at) -> TryBegin` | One write: the key's row exists: `Existing(it)`, nothing written. Else a `pending` row with the hash and time: `Begun` |
| `complete(tx, key, &result, extra_fields: Option<&Json>, completed_at)` | Sets `status` `completed`, `result` (`jsjson::stringify` of it), `completed_at`; from `extra_fields` (when an object): `economySource` and `economySessionId` go to their columns when strings (a non-string is dropped), `status`, `result`, `completedAt` are dropped, and the rest, when any is left, is stored in `extra` as `jsjson::stringify` text (else `extra` null). Every one of the six columns is overwritten |
| `find_record(conn, key) -> Option` | The row |
| `list_recent_for_account(conn, &AccountId, limit) -> Vec<Json>` | The account's rows by `created_at` descending then `transaction_id` descending, at most `limit` (0 answers none): each an object with keys in this order: `transactionId`, `characterId`, `status`, `createdAt`, then `economySource`, `economySessionId`, `completedAt` when set; never the result |

---

## 10. Progression (`repo::progression`)

### 10.1 Records and the stored reforge

- `WeaponReforge { reforge_id, open: bool, prior_progress, cap, banked, banked_restored: i64, prestige_granted: bool,
  opened_at, closed_at: Option }`, with `to_json` and `from_json` (public). Its JSON is an object with keys in order
  `reforgeId`, `status` (`open`/`closed`), `priorProgress`, `cap`, `banked`, `bankedRestored`, `prestigeGranted`,
  `openedAt`, and `closedAt` only when set. `from_json` needs every key but `closedAt` with the right type (integers as
  `i64`), else none.
- `ProgressionTrackRecord { account, progression_id, progress, confirmed_fremium_rank, confirmed_premium_rank,
  update_version, created_at, updated_at, reforge: Option<WeaponReforge>, prestige_claimed: Option<i64> }`. Reading:
  `reforge` text that is not JSON is a data error `reforge: <error>`; JSON that `from_json` refuses reads as **no
  reforge** (§16 item 10).
- `ProgressionObjectiveRecord { account, objective_id, value, completed_count, updated_at }`.
- `now_iso()` (public): the store's `now` of §2.1.

### 10.2 `ProgressionTrackRepository`

**Upsert base** (used by `increment`, `set_progress_if_greater`, `set_confirmed_fremium_rank`): the live track, or a
new row inserted with `progress` 0, ranks 0, `update_version` 0, `seq` = (the account's largest `seq`, deleted rows
included, else 0) + 1, `created_at` = `updated_at` = now. A soft-deleted track with that key makes the insert fail
(§16 item 3). Each of these three then updates the row (whether new or not) and answers the re-read live row (gone
meanwhile: data error `progression track {id} vanished inside its write`).

| Method | Contract |
| --- | --- |
| `get_all_for_account(conn, &AccountId)` | Live tracks by `seq` ascending (creation order) |
| `get(conn, &AccountId, id)` | The live track |
| `increment(tx, &AccountId, id, amount)` | Upsert base; `progress` += amount, `update_version` + 1, `updated_at` now |
| `set_progress_if_greater(tx, ..., value)` | Upsert base; `progress` = max(stored, value); `update_version` + 1 even when unchanged; `updated_at` now |
| `set_confirmed_fremium_rank(tx, ..., rank)` | Upsert base; that rank = max(stored, rank); `update_version` + 1; `updated_at` now |
| `append_objective_event(tx, &AccountId, raw_body, received_at)` | Inserts an event with a fresh random UUID v4 id (system randomness, never the replay id source: §16 item 11) |
| `get_all_objectives_for_account(conn, &AccountId)` | Live objectives by `seq` ascending |
| `set_objective_if_greater(tx, &AccountId, objective, value, completed)` | Merge (below) at now; answers the merged record |
| `merge_objectives(tx, &AccountId, &[(id, value, completed)])` | Merge each row in the order given, one `now` for all |
| `get_objectives(conn, &AccountId, ids)` | Live objectives among `ids` (an empty list answers none without a query), engine order |
| `begin_reforge(tx, &AccountId, id, expected_progress, expected_version, &reforge) -> Option` | None unless the live track has exactly that progress and version and no open reforge. Then: `progress` 0, both ranks 0, `reforge` = its JSON, `updated_at` = `reforge.opened_at`, `update_version` + 1; the re-read track |
| `update_reforge(tx, &AccountId, id, reforge_id, expected: &Json, set: &Json) -> bool` | `false` unless the live track has a stored reforge whose `reforgeId` is `reforge_id` and, for every key of `expected`, the stored value equals it (`serde_json` value equality; an absent key equals `null`: Mongo's `$eq: undefined`). Then every key of `set` is put on the stored object; if none changed a value: `false`, nothing written (Mongo's `modifiedCount` 0). Else the object is written back; `update_version` and `updated_at` are **not** touched |
| `claim_prestige_levels(tx, &AccountId, id, claimed_before, levels, min_progress) -> bool` | `true` only if the live track has `progress >= min_progress` and `prestige_claimed` (null reads 0) equals `claimed_before`; sets it to that + `levels` and `updated_at` now (no version change) |
| `spend(tx, &AccountId, id, amount) -> Option` | None when no live track or `progress < amount`; else `progress` − amount, `update_version` + 1, `updated_at` now; the re-read track |

**Merge of an objective**: the live (account, objective) row: `value` = max(stored, value) and `completed_count` =
max(stored, completed), each on its own, `updated_at` = now. None: inserted with the given values, `seq` = (the
account's largest objective `seq`, deleted included, else 0) + 1, `created_at` = `updated_at` = now.

### 10.3 `ProgressionGrantRepository`

`ProgressionGrantRecord { grant_id, account, progression_id, amount, request_hash, result: ProgressionTrackRecord,
created_at }`. `result` is stored as JSON with keys in order `userId` (the account's **global id** text, under Node's
name), `progressionId`, `progress`, `confirmedFremiumRank`, `confirmedPremiumRank`, `updateVersion`, `createdAt`,
`updatedAt`; read back with `reforge` and `prestige_claimed` none. A result missing any of those (or wrongly typed) is
data error `progression_grants.result is not a track`.

| Method | Contract |
| --- | --- |
| `find_by_grant_id(conn, grant_id)` | The row |
| `insert_applied(tx, &record)` | Insert with `status` `applied`; a replayed grant id is a database error (the caller then serves the stored result) |
| `list_recent_for_account(conn, &AccountId, limit)` | By `created_at` descending, at most `limit` |

### 10.4 `EscalationProgressRepository`

`EscalationProgressRecord { account, season_id, escalation_level, next_level_xp, talents_progress: Vec<Json>,
unlock_progress: Vec<Json>, update_version, created_at, updated_at }`; the two lists are stored as JSON arrays (a
stored text that is not an array is data error `talents_progress: ...` / `unlock_progress: ...`).

| Method | Contract |
| --- | --- |
| `get(conn, &AccountId, season)` | The live row |
| `save_if_version(tx, &record, expected: Option<i64>) -> Option<record>` | `expected` none: inserts only when there is no live row (answers the record), else none. `expected` some: none when no live row or its version differs; else every column (`created_at` included) is replaced by the record's, answering the record |

### 10.5 `PlayerJourneyRepository`

`PlayerJourneyRecord { account, nodes: String, update_version, revision }`.

| Method | Contract |
| --- | --- |
| `find(conn, &AccountId)` | The live row |
| `create_if_absent(tx, &record) -> bool` | `false` when any row of the account exists (**deleted included**); else inserted |
| `update_if_revision(tx, &AccountId, nodes, update_version, expected) -> bool` | On the live row at `expected`: sets `nodes`, `update_version`, `revision` + 1; whether exactly one row changed |

### 10.6 `BreadcrumbRepository`, `EncounteredContentRepository`

`BreadcrumbsRecord { character_id, account, breadcrumbs: String, update_version }` (column `items`);
`EncounteredContentRecord { character_id, account, encountered_content: String }` (column `content`). Both: `find`
answers the live row of that character **and** account; `create` inserts; `update` (`update(tx, character, &account,
json, update_version)`) and `update_content(tx, character, &account, json)` set the text by character and account,
with no live check and no version guard.

---

## 11. Economy, parties and the Reforged API

### 11.1 Bounties, cooldowns, entitlements, wallets, rollups (`repo::bounties`)

- `BountyGroup { Base, Daily, Weekly, Unassigned }` (`base`, `daily`, `weekly`, `unassigned`). `RewardCredit {
  catalog_id, amount: f64, transaction_id, at }`. `StoredBounty { group, bounty: Json, claimed_at: Option,
  reward_credit: Option }`, stored as an object with keys `group`, `bounty`, then `claimedAt` and `rewardCredit`
  (`{catalogId, amount, transactionId, at}`) when set. Reading an entry: no known `group` is data error `bountyStates:
  an entry has no group`, no `bounty` is `bountyStates: an entry has no bounty`; a `rewardCredit` object with missing
  fields reads `""` and 0.
- `DraftData { base, daily, weekly: Option<Json> }`, stored as an object with only the set keys in that order.
- `BountyStateRecord { account, bounties: Vec<StoredBounty>, draft_data, daily_reward_credited_on,
  daily_premium_draft_granted_on (Options), update_version, created_at, updated_at }`. A stored `bounties` that is not
  an array: `bountyStates.bounties is not a list`.
- `CooldownEntry { id, started_date, updated_at }`, stored as `[{"id","startedDate","updatedAt"}]` in write order; an
  entry without `id` is `a cooldown has no id`, the other two read `""` when absent. `CooldownStateRecord { account,
  entries, update_version, created_at, updated_at }`.
- `EntitlementEntry { name, duration: f64, activated_date, granted_at, source_sku_id (Options) }`,
  `EntitlementRecord { account, entitlements }`. `is_valid_entitlement_name`: 1 to 128 bytes of ASCII letters, digits,
  `_`, `.`, `-`. `is_valid_balance_catalog_id`: 1 to 128 bytes of ASCII letters, digits, `_`.
- `WalletRecord { account, balances: Vec<(String, f64)>, bootstrap_version }` with `balance(id)`.
- `SOFT_DELETE_TABLES` (public): `bounty_states`, `cooldown_states`, `entitlements`, `wallets`, `wallet_balances`.

| Method | Contract |
| --- | --- |
| `BountyStateRepository::get(conn, &AccountId)` | The live row, decoded |
| `BountyStateRepository::save_if_version(tx, &record, expected) -> Option<record>` | `expected` none: an insert that does nothing on any key conflict (a soft-deleted row included); `expected` some: on the live row at that version, sets the lists, both dates, `update_version` (the record's) and `updated_at` (not `created_at`). Answers the record as given when a row was written, else none |
| `CooldownStateRepository::get`, `save_if_version` | The same rules over `entries` |
| `EntitlementRepository::find(conn, &AccountId)` | The live entries by `name` ascending; none when there are no live entries |
| `EntitlementRepository::grant_if_missing(tx, &AccountId, &entry) -> bool` | An invalid name is data error `unsafe entitlement name "<name>"`; else an insert that does nothing on conflict (a held name, live or deleted, stays as it is); whether it inserted |
| `WalletRepository::find(conn, &AccountId)` | The live wallet with its live balances by `ordinal` ascending (first-written order) |
| `WalletRepository::create(tx, &wallet)` | Plain insert (an existing wallet is a database error), then each balance in order with `ordinal` 1, 2, ...; an invalid catalog id is data error `unsafe balance catalogId "<id>"` (the caller's transaction rolls back) |
| `WalletRepository::create_if_missing(tx, &wallet) -> WalletRecord` | Inserts the wallet unless the key exists (then its balances as `create` does); answers the stored live wallet, never resetting an existing one. A soft-deleted wallet with that key: data error `a wallet vanished` (§16 item 5) |
| `WalletRepository::increment_balance(tx, &AccountId, catalog, delta) -> Option<WalletRecord>` | Invalid catalog id: data error as above. No live wallet: none. One write, guarded in the statement: a debit (`delta < 0`) applies only where the live balance is at least `-delta`; a credit always applies. Nothing applied: a debit answers none; a credit creates the balance with `ordinal` = (the account's largest ordinal, deleted included, else 0) + 1, or adds to a conflicting row (§16 item 6). Answers the re-read wallet |
| `EconomyRollupRepository::add_to_rollup(tx, &AccountId, day, currency, increments: &[(path, f64)], ttl_at) -> Json` | For each dotted path: the counter is created with the amount or the amount is added to it, and its `ttl_at` set. Answers the day's document for that currency: every counter by `path` ascending, nested by `.` (`bySource.STORE.debited` is `{"bySource":{"STORE":{"debited":v}}}`), a non-object met on the way replaced by an object; values as `f64` |
| `rollup_document(&[(path, f64)]) -> Json` (public) | That nesting on its own |

### 11.2 Parties (`repo::parties`)

Records: `PartyActivity`, `InviteStatus`, `PartyEventType`, `OutboxState` (texts of §4.12, `as_str`/`parse`);
`PartyMember { account: AccountId, user_id: UserId }`; `PartyRecord { id: GlobalId, party_id (Node's `partyId`, the
`client_id` column), revision, leader: PartyMember, members: Vec<PartyMember> (join order), build_id, activity,
active_hunt_session_id: Option, created_at, updated_at }`; `PartyInviteRecord { invite_id, party_id, party_revision,
sender, recipient, sending_display_name, sending_platform, status, created_at, expires_at }`; `PartyOutboxRecord {
event_id, party_id, party_revision, event_type, recipients: Vec<UserId>, payload: Json, state, created_at, claimed_at,
claim_owner, published_at }`; inputs `NewParty { id: GlobalId, party_id }`, `NewInvite { sender, recipient, build_id,
sender_display_name, sender_platform, new_party, now, expires_at }`, `AcceptInvite { recipient, inviter, build_id,
new_party, now, max_party_size: u32 }`, `PartyEvent { party_id, party_revision, event_type, recipients, payload, now }`;
`REVISION_CONFLICT` = `PARTY_REVISION_CONFLICT`.

**Reading a party**: members by `position` ascending; every member's and the leader's `userId` comes from `accounts`
(soft-deleted accounts included); a member with no `accounts` row is data error `party member <id> has no account`.

**The snapshot** (an event payload): an object with keys `partyId`, `revision`, `leaderPlayerId` (userId), `members`
(userIds), `activity`, `activeHuntSessionId` (null when none).

**Writing an event**: recipients de-duplicated keeping first occurrences, stored as a JSON array; `seq` = (largest
`seq` in the outbox, else 0) + 1; a random UUID v4 id (system randomness, §16 item 11); `state` `PENDING`;
`created_at` the event's now; the payload as compact JSON.

**Bump**: moving a party from the revision it was read at to + 1, setting `leader` and `updated_at`; when no row is at
that revision, the write fails with data error `PARTY_REVISION_CONFLICT` (the area retries; its check is "the error
text contains it").

| Method | Contract |
| --- | --- |
| `find_by_id(conn, &GlobalId)`, `find_by_party_id(conn, client_id)`, `find_by_member(conn, &AccountId)` | The party |
| `get_or_create_solo(tx, &AccountId, build_id, &new_party, now) -> PartyRecord` | The account's party if any (no event). Else (the account must have an `accounts` row, else data error `<id> has no account`, before anything is written): a party with `new_party`'s ids, revision 1, the account as leader and only member (position 0), activity `IDLE`, both times now; event `PARTY_CREATED` to the account at revision 1 with the snapshot |
| `create_invite(tx, &NewInvite)` | Sender = recipient: nothing. Else the sender's party (made as above if needed); the recipient already in it: nothing more. Else the (recipient, sender) invite is set (inserted, or updated keeping its old `consumed_at`): party global id, its revision, name, platform, `PENDING`, `created_at` now, `expires_at`, `seq` = global largest + 1. Event `PARTY_INVITE_CREATED` to the recipient at the party's revision, payload keys `partyId`, `partyRevision`, `sendingPlayerId`, `recipientPlayerId`, `sendingDisplayName`, `sendingPlatform`, `status` (`PENDING`), `createdAt`, `expiresAt`, `inviteId` (`<recipient userId>` U+001F `<sender userId>`) |
| `list_pending_invites(conn, &recipient, now) -> Vec` | The recipient's `PENDING` invites with `expires_at > now`, by `created_at` descending then `seq` descending; `party_id` is the party's `client_id`, or its stored global id text when the party is gone |
| `accept_invite(tx, &AcceptInvite) -> Option<PartyRecord>` | (1) No `PENDING` unexpired (recipient, inviter) invite: the recipient's current party, nothing written. (2) The inviter's party (`get_or_create_solo`, which may create one with its event). (3) It is not the invite's party: the current party, nothing more (an invite belongs to the issuing party). (4) The recipient is already in it: invites consumed (below); that party. (5) It has `max_party_size` members or more: the current party. (6) Else the recipient leaves their current party (removal `MOVE`, below), the inviter's party is bumped (leader unchanged), the recipient joins at position (largest + 1, or 0), invites are consumed, event `PARTY_MEMBERSHIP_CHANGED` to every member (the joiner included) with the snapshot plus `action` `JOIN` and `accountId` (the joiner's userId); the new party |
| `leave(tx, &AccountId, now)` | No party: nothing. Else removal `LEAVE` |
| `kick(tx, &actor, &target, now)` | Nothing unless the actor leads a party the target is in and the target is not the actor; removal `KICK` with the leader as actor |
| `promote(tx, &actor, &target, now)` | Nothing unless the actor leads and the target is another member; bump with the target as leader; event `PARTY_LEADER_CHANGED` to every member with the snapshot |
| `set_activity(tx, &GlobalId, expected, activity, hunt_session: Option, now) -> Option` | None when the party is gone or not at `expected`; else revision + 1, activity, hunt session (cleared with none), `updated_at`; a lost race is the conflict error; event `PARTY_ACTIVITY_CHANGED` to every member with the snapshot; the new party |
| `append_event(tx, &PartyEvent)` | Writes it as given |
| `claim_next_outbox(tx, owner, now, stale_before) -> Option` | The oldest (`created_at`, then `seq`) event that is `PENDING`, or `PROCESSING` with `claimed_at < stale_before`; marked `PROCESSING` by `owner` at now; the re-read event (recipients and payload parse with `serde_json`; a failure is data error `outbox recipients: ...` / `outbox payload: ...`) |
| `mark_outbox_published(tx, event_id, owner, at) -> bool` | Only a `PROCESSING` event claimed by `owner`: `PUBLISHED`, `published_at`, claim cleared; whether one changed |
| `release_outbox(tx, event_id, owner)` | Only a `PROCESSING` event claimed by `owner`: back to `PENDING`, claim cleared |

**Consuming invites** (recipient, inviter, now): the recipient's `PENDING` invite from the inviter becomes `ACCEPTED`,
every other `PENDING` invite of the recipient `REVOKED`, both with `consumed_at` now.

**Removing a member** (party, target, now, removal, actor: Option): the target's membership is deleted. No one left:
the party row is deleted (at its read revision, unchecked), event `PARTY_MEMBERSHIP_CHANGED` at revision + 1 to the
target only, payload keys `action` `DISBAND`, `accountId` (target userId), `actorAccountId` (when an actor), `members`
`[]`. Members left: the leader stays, or becomes the first remaining member when the target led; bump; event
`PARTY_MEMBERSHIP_CHANGED` to the remaining members then the target, payload = the new snapshot plus `action`
(`LEAVE`, `KICK`, `MOVE`), `accountId`, `actorAccountId` when an actor. Invites are not touched.

### 11.3 The Reforged API (`repo::reforged`)

#### 11.3.1 `AdminRepository`

`AdminSessionRecord { id, token_hash, account, csrf_token, created_at, expires_at, ip, user_agent, revoked_at }`;
`AdminAuditRecord { id, actor_user_id, target_user_id: Option, action, old_state: Option<Json>, new_state:
Option<Json>, reason: Option, ip, request_id, created_at }`; `OperationsPolicyRecord { guard_enforcement,
diagnostics_profile, version: u64, updated_at, updated_by, diagnostics_expires_at }`; `PlayerDeletion {
deleted_counts: Vec<(String, u64)>, character_count: u64 }`.

| Method | Contract |
| --- | --- |
| `create_session(tx, &record)` | Insert; `ttl_at` = `Date.parse(expires_at)` (`service_kit::js::date_parse`; unparseable 0) + one day |
| `find_active_session_by_token_hash(conn, hash, now)` | Unrevoked with `expires_at > now`; answered with `revoked_at` none |
| `revoke_session(tx, id, at)` | Sets `revoked_at` |
| `append_audit(tx, &record)` | Insert; the states as compact JSON |
| `list_audit(conn, target: Option, skip, limit) -> (Vec, u64)` | A non-empty target filters `target_user_id`; by `created_at` descending then `seq` descending; total is the filtered count |
| `read_operations_policy(conn)` | Row 1 (none only if emptied by hand) |
| `update_operations_policy(tx, expected, &record) -> bool` | Row 1 at `version` = expected: sets the five fields and `version` = expected + 1 (the record's `version` is ignored); whether it changed |
| `delete_player_data(tx, &AccountId, user_id, &audit) -> PlayerDeletion` | One write: soft-deletes the game and launcher accounts, deletes the account's admin sessions and Guard sessions, removes the account from every Guard finding that lists it (its `account_ids` and `account_count` follow; each such finding counts 1), deletes its daily activity, hunts, unlocks and flags, erases the other areas (§11.3.6), then appends the audit record. Counts in order: `accounts`, `launcherAccounts`, `adminSessions`, `launcherGuardSessions`, `guardFindings`, `playerActivityDaily`, `activityHunts`, `slayersPathUnlocks`, `activityFlags`, then §11.3.6's; `character_count` is §11.3.6's. `user_id` is unused |

#### 11.3.2 `LauncherNoticesRepository`

`LauncherNoticesRecord { version: u64, updated_at, updated_by, announcements: Vec<Json>, tags: Vec<Json> }`.
`get`: row 1, with each list parsed as a JSON array (anything else reads empty). `replace_if_version(tx, &record,
expected) -> bool`: no row: `expected` must be 0, then the row is inserted with the record's version; a row: replaced
only at `version` = expected, with the record's version. The lists are stored as compact JSON.

#### 11.3.3 `ActivityRepository`

`HuntKind`, `HuntResult` (texts of §4.12); `ActivityHuntRecord { id, account, host, kind, hunt_id, hunt_ref,
started_at, ended_at, end_reason, result: Option, kills: u64, ttl_at }`; `SlayersPathUnlockRecord { id, account,
node_id, unlocked_at, source, ttl_at }`; `ActivityFlagRecord { id, kind, severity, account, day, detail, acknowledged,
created_at, acknowledged_at, acknowledged_by, note, ttl_at }`.

| Method | Contract |
| --- | --- |
| `add_to_daily(tx, &AccountId, day, increments: &[(path, f64)], ttl_at) -> Json` | One write: the (account, day) document (an object; a stored text that is not one is data error `player_activity_daily.counters: ...`) or `{}`; `day` set to the day (a new document has it first); each non-zero amount added at its dotted path (missing or non-numeric leaves read 0; a non-object on the way becomes an object); stored with `ttl_at`; answered |
| `set_daily_fields(tx, &AccountId, day, fields, ttl_at) -> Json` | The same document; `day`, then each top-level field set; stored; answered |
| `find_daily_rows(conn, &AccountId, from, to, limit)` | Days `from..=to` ascending, at most `limit`; each document with `day` set |
| `find_daily_rows_in_range(conn, from, to, limit)` | Every account; by day then account ascending |
| `insert_hunt_if_absent`, `insert_unlock_if_absent`, `insert_flag_if_absent` `(tx, &record) -> bool` | `false` when the id exists; else inserted |
| `find_hunt(conn, id)`, `find_flag(conn, id)` | By id |
| `find_open_dedicated_hunts(conn, &AccountId)` | The account's `dedicated` hunts with no end, by `started_at` descending, at most 50 |
| `find_dedicated_hunts_covering(conn, &AccountId, at, not_before, grace_floor)` | `dedicated`, `not_before <= started_at <= at`, and no end or `ended_at >= grace_floor`; by `started_at` descending, at most 10 |
| `close_hunt_if_open(tx, id, ended_at, reason, result) -> bool` | Only while no end: sets the three; whether it did |
| `add_hunt_kills(tx, id, kills)` | `kills` += kills |
| `find_hunts_for_account(conn, &AccountId, limit)` | By `started_at` descending then `id` ascending |
| `find_unlocks_for_account(conn, &AccountId, limit)` | By `unlocked_at` descending then `id` ascending |
| `list_flags(conn, acknowledged: Option<bool>, skip, limit) -> (Vec, u64)` | Filtered when some; by `created_at` descending then `id` ascending; total is the filtered count |
| `find_flags_for_account(conn, &AccountId, limit)` | By `created_at` descending then `id` ascending |
| `acknowledge_flag(tx, id, by: &AccountId, at, note: Option) -> Option` | None when missing; an acknowledged flag answers as it is; else sets acknowledged, `acknowledged_at`, `acknowledged_by` = the operator's live `userId` (else the account id text) and `note` (cleared with none); the re-read flag |

Decoding a hunt: unknown kind or result, account not an id: data errors `activity_hunts.{kind|result|account}: <text>`.

#### 11.3.4 `LauncherGuardRepository`

`GuardRole { Client, Host }`, `GuardStatus` (§4.12); `LauncherGuardSessionRecord { guard_session_id, account, role,
public_key_base64, manifest_id, manifest_sequence: u64, channel, launcher_version, challenge, last_sequence: u64,
status, risk_score: f64, created_at, expires_at, last_heartbeat_at: Option, last_signals: Option<Json>, reasons:
Vec<String>, history: Vec<Json> }`; `GuardHeartbeat { guard_session_id, account, expected_challenge, sequence: u64,
next_challenge, status, risk_score, heartbeat_at, expires_at, signals: Json, reasons, history_entry: Json }`;
`GuardFinding { file_name, sha256, signer, size, first_seen_at, last_seen_at, session_count, account_count }`;
`GuardSighting { file_name, sha256, signer, size, account, seen_at, new_in_session: bool }`. Constants
`GUARD_HISTORY_LIMIT` 20, `GUARD_FINDING_ACCOUNT_LIMIT` 1000, `GUARD_RETENTION_DAYS` 90.

- The `detail` column is a JSON object with keys `launcherVersion`, `lastSignals` (null when none), `reasons`,
  `history`. Reading: a role other than `host` reads `Client`; `launcherVersion` missing reads `""`; `lastSignals` null
  reads none; only string reasons are kept; non-array lists read empty; unknown status is data error
  `launcher_guard_sessions.status: <text>`. A `u64` above `i64::MAX` is stored as `i64::MAX`; a negative stored
  sequence reads 0.
- **TTL** of a session or finding: `Date.parse(time) + 90 days` (unparseable 0 + 90 days).

| Method | Contract |
| --- | --- |
| `create(tx, &record)` | Insert, `ttl_at` from `created_at` |
| `find(conn, id)` | By id |
| `find_healthy_for_account(conn, &AccountId, id, role, now)` | `now` must `Date.parse` (else data error `now: <text>`); the session of that id, account and role, `HEALTHY`, `expires_at > now` and `last_heartbeat_at` after the ISO text of now − 45 s |
| `advance(tx, &GuardHeartbeat) -> Option` | One write: a session of that id and account, holding `expected_challenge`, `last_sequence < sequence`, live status, `expires_at > heartbeat_at`; else none. The history becomes the new entry followed by the old, cut to 20; `lastSignals` and `reasons` replaced. The update repeats the id, challenge and sequence conditions (none if a race lost) and sets the next challenge, sequence, status, risk, `last_heartbeat_at`, `expires_at`, detail and `ttl_at` from the heartbeat time; the re-read session |
| `revoke_for_account(tx, &AccountId, role, at)` | The account's live sessions of that role: `REVOKED`, `expires_at` = at, `ttl_at` from it |
| `list_sessions(conn, status: Option, min_risk: Option<f64>, skip, limit) -> (Vec, u64)` | Filtered (`risk_score >= min`), by `created_at` descending; histories emptied in the list; total filtered |
| `list_findings(conn, signer: Option, skip, limit) -> (Vec, u64)` | A non-empty signer filters to findings whose lower-cased signer contains it lower-cased (a `LIKE` with `\`, `%` and `_` escaped); by `session_count` descending, `last_seen_at` descending, `id` descending |
| `record_finding_sighting(tx, &GuardSighting)` | One write. A new (sha256, file name): inserted with first and last seen = seen, `session_count` 1 if new in session else 0, `account_count` 1, `account_ids` `["<account>"]`, `ttl_at`. Existing: signer, size, last seen, `ttl_at` updated; `session_count` + (1 if new in session); the account added to `account_ids` unless listed or the list holds 1000; `account_count` = the list's length (a stored list that does not parse reads empty) |

#### 11.3.5 `InviteRepository`

`InviteRecord { code, uses_remaining: i64, infinite_uses: bool, note: Option, created_at }`.

| Method | Contract |
| --- | --- |
| `insert_if_absent(tx, &record) -> bool` | Insert; a unique violation answers `false` (on Postgres it spoils the transaction: the caller retries in a new one, §16 item 14) |
| `spend(tx, code) -> bool` | An unlimited code: `true`, unchanged. Else one statement takes one use where `uses_remaining > 0`; whether it did (an unknown code: `false`) |
| `list(conn, limit)` | By `created_at` descending then `code` ascending |
| `delete(tx, code) -> bool` | Whether exactly one went |

#### 11.3.6 `erase_other_areas` and `purge_expired_ttl_rows` (public functions)

`erase_other_areas(tx, &AccountId) -> RepoResult<(Vec<(String, u64)>, u64)>`: every non-auth area's rows of the
account, in one write. The account's characters are every `characters` row of the account, deleted ones included;
"by account or character" below means rows whose account column is the account or whose character id is one of them.
Soft deletes set `deleted_at` = `Tx::at()` on live rows only. Counts in this order:

| Name | What |
| --- | --- |
| `characters` | soft-deletes the account's live characters (this count is also the second value) |
| `loadouts`, `inventories` | soft-delete by account or character |
| `inventoryTransactions` | deletes by account or character |
| `breadcrumbs`, `encounteredContent` | soft-delete by account or character |
| `progressionTracks`, `progressionObjectives` | soft-delete by account |
| `progressionObjectiveEvents`, `progressionGrants` | delete by account |
| `escalationProgress`, `playerJourney`, `bountyStates`, `cooldownStates`, `entitlements`, `walletBalances`, `wallets` | soft-delete by account |
| `economyRollups` | deletes by account |
| `friends` | deletes edges the account owns or is the other end of |
| `partyInvites` | deletes invites it sent or received |
| `parties` | deletes its memberships (each counts 1); for each party it was in, if members remain and it led, the first remaining member by position becomes leader (no revision bump, no event); if none remain, the party's outbox rows and the party are deleted (the party counts 1) |

`purge_expired_ttl_rows(tx, now_ms) -> RepoResult<u64>`: deletes rows with `ttl_at <= now_ms` from `admin_sessions`,
`launcher_guard_sessions`, `guard_findings`, `player_activity_daily`, `activity_hunts`, `slayers_path_unlocks`,
`activity_flags`, `economy_rollup_counters`; the total.

---

## 12. The `AuthStore` adapter

`IslesAuthStore` (public), built from an `IslesStore`, implements `rustic_isles_auth::AuthStore` (the trait as it is
in dainty-lass `crates/rustic-isles-auth/src/store.rs`; auth spec §9.3). It is what core's composer hands `Auth` as
`Arc<dyn AuthStore>`.

### 12.1 Common rules

- **Reads** run in `IslesStore::read`. **Writes** use the actor given: a `Player { account }` actor makes the write an
  `account_write` for **that** account (the home check of §3.4); any other actor is a plain `write`. This is restricted
  behaviour: the auth area's player writes are always `account_write`s for the player's own account, its system writes
  plain `write`s.
- **Account id texts**: the trait passes `&str`. A read whose id does not parse as an account id answers none (no
  stored row could match); a write whose id does not parse is a `StoreError`.
- **Errors**: any `RepoError` becomes `StoreError { message: <its display> }` (so `NotHome` is answered `INTERNAL` by
  auth, auth spec §9.2).
- **Records**: `GameAccount { id: <global id text>, user_id, name, notes }`. `LauncherAccount` from §6.2's record:
  `account_id` = the id text, `approval` = `approval_status`, `roles_updated_at`, `consent`, `deletion`, `timeout`,
  and the rest by name; `approval_updated_*`, `approval_reason` and `roles_updated_by` are not carried. `Identity`,
  `RefreshSession`, `ExchangeCode` (`Channel` = `RuntimeChannel`, `build_changelist` widened to `i64`),
  `DiscordTransaction`, `EmailCode` (`Purpose`) map field by field with ids as text. Going the other way, an
  `ExchangeCode` changelist outside `u32` is a `StoreError`.
- **The code-tries limit** where the trait passes none (`live_email_code`, `consume_email_code`) is the auth crate's
  (`rustic_isles_auth::email::CODE_MAX_ATTEMPTS`, 5; auth §7.3).

### 12.2 Operations

| Trait method | What the adapter does |
| --- | --- |
| `game_account_by_user_id`, `game_account_by_id` | §6.1 finds |
| `ensure_game_account(actor, &account) -> bool` | One write: a live game account with that `user_id` exists: `false`. Else §6.1 `create` with the given id (must parse), user id, name, notes: `true` |
| `launcher_account_by_user_id`, `_by_id`, `_by_email`, `_by_name` | §6.2 finds |
| `create_launcher_account(actor, &new) -> CreateOutcome` | §6.2 `create` of the account (timeout ignored); the game account it writes when missing takes `new.game_name` as its name. `Created(the account as given)`, `EmailTaken`, `NameTaken` |
| `set_last_login`, `set_consent`, `set_deletion`, `set_email_verified` | §6.2, one write each |
| `set_username(actor, id, display, normalized) -> bool` | §6.2 `set_username`; `false` for `DisplayNameTaken` |
| `reset_password(actor, id, hash, at)` | One write: `set_password_hash`, `set_email_verified(at)`, `revoke_all_for_account(at)` |
| `due_deletions(now, limit)` | `list_deletions_due` (`limit` clamped to `u32`) |
| `stale_unverified(before, limit)` | `list_unverified_registrations` |
| `delete_unverified_registration(actor, id) -> bool` | One write: only while the launcher row is not soft-deleted, its pending mark is set and it is unverified (the trait gives no cut-off; the pending mark is written once at registration and only ever cleared, so this equals restricted's re-check: §16 item 2), soft-delete the launcher and game accounts, then delete the account's email codes and refresh sessions; whether the launcher row was soft-deleted |
| `erase_account(actor, id) -> ErasureCounts` | One write, in this order: delete the refresh sessions (not counted); `email_codes` = §6.7 delete; `game_exchange_codes` = §6.5 delete; `discord_oauth_transactions` = §6.6 delete; `auth_identities` = §6.3 delete; `user_api_keys` = 1 if §6.8 delete removed one else 0; `friendships` = §8 `remove_all_for_account`; `other_areas` = the sum of §11.3.6's counts (whose `friends` is then 0); `accounts` = 1 if §6.2 `erase` found a live row else 0 |
| `identity_by_subject`, `identity_by_account`, `create_identity`, `delete_identities_for_account` | §6.3 |
| `create_session`, `session_by_token_hash` | §6.4 |
| `rotate_session(actor, revoke_id, at, &next)` | One write: `revoke_by_id`, then `create(next)` |
| `revoke_family(actor, family, at) -> u64`, `revoke_all_sessions(actor, id, at) -> u64` | §6.4's revocations, answering how many rows they changed (restricted's repository returns nothing; the count is new, §16 item 16) |
| `is_family_active` | §6.4 |
| `create_exchange_code`, `consume_exchange_code` | §6.5 |
| `create_discord_transaction`, `consume_discord_state`, `attach_discord_completion`, `consume_discord_completion` | §6.6 (`attach` needs a parsing account id) |
| `replace_email_code`, `live_email_code`, `consume_email_code` | §6.7 `replace`, `peek_live`, `consume` with the limit of §12.1 |
| `count_email_code_try(actor, id, purpose, now, max) -> Option<u32>` | One write: §6.7 `reserve_try`; when it answers a count that reached `max`, the row is removed (only while it still holds that hash); answers the count (some even when that try removed the row), or none |
| `gameserver_key_hashes`, `replace_gameserver_keys`, `user_key_hashes` | §6.8 (user keys as `(key_hash, account id text)`) |
| `health` | A read running a trivial query (`SELECT 1`) |

---

## 13. The public seam

### 13.1 Shape

One crate, `rustic-isles-store` (lib), MIT, in dainty-lass `crates/rustic-isles-store`. Depends on dainty-lass crates
`store` (`Database`, `DatabaseConfig`, `DatabaseOptions`, `DatabaseSchema`, `AnyConnection`, `Tx`, `KeyValue`,
`MultiBackend`, `Actor`, `Error`), `module-api` (`ActorContext`, `Actor`, `GlobalId`, `NodeId`, `NodeIdentity`,
`HomeApi`, `HomeError`, `BoxFuture`), `rustic-isles` (core: `AccountId`, `UserId`, the replay scope, `to_iso_string`,
`now_ms`), `rustic-isles-auth` (`AuthStore`, its records, `CODE_MAX_ATTEMPTS`), `service-kit` (`jsjson`, `js::
date_parse`), and workspace `diesel`, `diesel_migrations`, `serde`, `serde_json` (own `preserve_order`,
`float_roundtrip`), `rand` (UUID v4 bytes), `thiserror`, `tracing`. Core does not depend on it (the composer does),
so there is no cycle.

**Seam changes for the integration pass** (not this row's):

- `launcher_accounts` has 27 columns and dainty-lass's workspace `diesel` has no `32-column-tables` feature (16 is
  the limit with default features off). Either the feature is added to `[workspace.dependencies]`, or the crate reads
  and writes that table without a 27-column table declaration. §16 item 1.
- `crates/rustic-isles-store` and `rustic-isles-store` in `[workspace.dependencies]`.
- The migrations move with the crate (`crates/rustic-isles-store/migrations/{sqlite,postgres}`), byte-for-byte the
  same versions and folder names.

### 13.2 Interface (names are the contract the composer, the auth adapter and the area crates are written against)

| Item | Contract |
| --- | --- |
| `pub const SCHEMA: store::DatabaseSchema` | Both migration sets and §2.2's soft-delete list |
| `pub struct IslesStore` (`Clone`) | §3: `open`, `open_configured`, `with_repos`, `database`, `repos`, `node`, `home`, `read`, `write`, `account_write` |
| `pub fn store_actor(&ActorContext) -> store::Actor` | §2.3 |
| `pub enum RepoError`, `pub type RepoResult<T>`, `RepoError::unimplemented(repository, operation)` | §2.4 |
| `pub type Json = serde_json::Value`; re-exports `AnyConnection`, `Tx`, `AccountId`, `UserId` | |
| `pub struct Repositories` (fields of §5.1), `on_store()`, `stubs()`; `pub struct Stub` | §5.1 |
| `pub mod repo::{auth, characters (with slots), chat, inventory, progression, bounties, parties, reforged}` | Each module's traits, records, enums, constants and public functions as named in §6 to §11, and one public SQL type per implementation |
| `pub struct IslesAuthStore` | `new(IslesStore)`; implements `rustic_isles_auth::AuthStore` (§12) |
| feature `test-support` | A store on a temporary SQLite file for tests here and in the area crates: `TempStore { store, dir }` opened with a standalone home (§15.1) |

### 13.3 Logs

None of its own: the crate returns errors and the callers log them (English, never a row's secret column).

---

## 14. Security behaviour (for `docs/security/rustic-isles-store.md` and RX3)

The crate opens no listener and parses no request, so it is not on RX3's trust-boundary list; what it guards is the
data at rest and the home rule. A short checklist page is still worth having (the integration pass decides).

- **Home rule (DD17).** An account write runs only on the account's home node; another node's account is refused
  (`NotHome`, which DN5 forwards) and an unresolvable home is refused (`Home`). Plain `write`s are for rows that
  belong to no one account; the adapter picks the path from the actor (§12.1), so a player actor can never write
  another node's account.
- **Every write is logged (DD23).** No connection that can write exists outside `store::Database::write`; the change
  log records the actor of §2.3. Player-facing rows are soft-deleted, so an erasure or an admin deletion can be
  reverted until the retention purge, and an erasure removes rows from every area in one transaction.
- **Secrets stay out of `Debug`** (a deliberate improvement on restricted, as core's settings `Debug`): password
  hashes, refresh token hashes, exchange code hashes, Discord states, verifiers and completion hashes, email code
  hashes, game server and user key hashes, admin session token hashes and CSRF tokens, Guard challenges and next
  challenges are never printed by a record's `Debug` (restricted's derived `Debug` prints the exchange code hash, the
  admin token hash and CSRF token, the Guard challenge and the game server key hashes). Hashes only are stored; no
  plaintext key, code or token reaches the database.
- **SQL**: every value is bound, never spliced. The few hand-written statements (`ON CONFLICT` upserts, which Diesel's
  two-engine connection has no DSL for) use `$1`.. placeholders as `store`'s portable rule requires. The signer search
  escapes `\`, `%`, `_` for `LIKE`.
- **Name hygiene**: entitlement names and balance catalog ids are validated before they are stored (a name is a value,
  never a path); a rollup or activity path is split on `.` only to build a document, never used as SQL.
- **Replay**: closures run in the caller's replay scope (§3.3); the store adds no way to set one. Core §17 item 1
  gates the headers.
- **Fuzz target** (`fuzz/fuzz_targets/rustic_isles_store_decode.rs`): arbitrary text as a stored `roles`, Guard
  `detail`, bounty `bounties`/`draft_data`, cooldown `entries`, reforge JSON, grant `result`, notices lists, and
  arbitrary dotted paths and amounts into the rollup document and the daily counters never panic, answer a data
  error or a value, and keep `rollup_document`'s output an object; `resolve_visible_total`,
  `resolve_requested_total`, `resolve_active_index` and `wire_counts` never panic for any `i64`/`f64` input.

---

## 15. Test cases

### 15.1 What may be copied

Nothing recorded exists for this unit; the tests listed are authored test code in restricted and are described here
only. Each runs on SQLite (a temporary file) and, when `REFORGED_TEST_POSTGRES_URL` is set
(`postgres://user:password@host:port/database`), again on a throwaway Postgres database the test creates and drops;
unset, the Postgres half prints that it is skipped and passes. The restricted goldens for the areas replay against
this crate once the composer uses it.

### 15.2 Behaviour the authored tests fix

| # | Area | Case |
| --- | --- | --- |
| S1 | Handle | An account created through `account_write` by an admin actor on its home node is found by `user_id`; the change log has an `accounts` `insert` by `admin:owner` whose key contains the id |
| S2 | Handle | `account_write` for an account of node `germany-1` on a standalone store answers `Home(_)` |
| S3 | Handle | `Stub`'s `find_many_by_account` answers `Unimplemented { repository: "characters", operation: "find_many_by_account" }` |
| S4 | Handle | (new; restricted checks it in core's tests) A closure inside a replay scope with a pinned time sees that time in `now_iso()` |
| A1 | Launcher | Create round-trips the record and makes the game account (name = display name, notes 0); found by user id, account, email and normalized name; an unknown email is none |
| A2 | Launcher | A second account with the same email: `EmailTaken`; with the same normalized name (`SLAYER` vs `slayer`) and another email: `DisplayNameTaken`, and neither the launcher nor the game account was written; two accounts with no email both `Created` |
| A3 | Launcher | Last login, consent, deletion, verification (clears the pending mark) and password writes read back; `list_deletions_due` at a time after the scheduled one lists it; clearing the deletion removes it |
| A4 | Launcher | `set_username` sets the name, the normalized name, `username_set` and the game account's name (`Slayer Prime`); a name another live account holds: `DisplayNameTaken` |
| A5 | Launcher | Admin list of five accounts: newest first; approval `approved` counts a legacy (null) row (4 of 5); a search finds one (total 1); skip/limit pages 2 of 5 |
| A6 | Launcher | Access state and roles write and answer the re-read row; an unknown account answers none |
| A7 | Launcher | An unverified registration pending since T0 is listed before T1; `delete_unverified_registration(.., T0)` is `false` (not past the cut-off), `(.., T1)` is `true`; it is gone; a new registration with the same address and name is `Created`; the log has the insert by the actor and the soft delete as an `update` |
| A8 | Sessions | Rotation, family revocation (only unrevoked), revoke-all, `is_family_active` (false once revoked or expired), `find_by_token_hash` answers a revoked session, `purge_expired` removes only expired rows |
| A9 | Codes | Exchange code consumed once, refused after expiry; `revoke_unused_for_account` consumes unused codes; Discord state and completion consumed once each, expired refused; email code replace keeps one per purpose, `consume` works once, `reserve_try` counts and removes an expired or used-up row |
| A10 | Identities | Second link of the same subject `false`; another subject for a linked account `false` ("one link per account"); by subject equals by account; delete for account removes 1 |
| A11 | Keys | Replace `[aa, bb]` then `[bb, cc]` leaves `bb, cc`; replace `[]` empties; two user keys for one account leave one, the second (`second`) |
| C1 | Characters | Insertion order kept across accounts; another account's character is none; `update_data_conditional` writes only for a higher version |
| C2 | Loadouts | Revision-guarded writes advance the revision by 1 and a stale revision changes nothing; a slot beyond the visible total is refused; `replace_all` refuses lengths outside 1..6 and an entitlement above the length; re-applying the current active index is `true` without a revision change |
| C3 | Loadouts | A legacy row (no entitlement, revision 0) stays writable at revision 0 |
| C4 | Characters | A character write is in the change log; a soft-deleted character is gone from reads until restored with `Tx::restore` |
| C5 | Slots | Visible: (3, none) 2, (2, none) 1, (1, none) 1, (3, 2) 2, (2, 3) error. Requested: (none, 2) 2, (3, 2) 3, (5, 6) 6, (none, 0) error. Active: (3, 4) 3, (none, 4) 0, (4, 4) 0, (−1, 4) 0. Journey: five unlocked 6; statuses 2, 1, 0 give 2; none 1. Wire: 6 → (0, 0, 6, 6), 1 → (0, 0, 1, 6), 0 / −3 / NaN / ∞ → total 1, 7 and 6.9 → 6; for 0..5 unlocked nodes the character count equals 1 + unlocked |
| F1 | Friends | Upsert keeps list position; one-sided remove leaves the other direction; `remove_all_for_account` removes both directions; on both engines |
| I1 | Inventory | Create, revision-guarded update (re-read answered, stale none), logged with the player actor |
| I2 | Inventory | A soft-deleted inventory is hidden and comes back after `restore` |
| I3 | Ledger | `try_begin` claims once, then answers `Existing`; `complete` stores the result and extras (economy columns split out, core fields dropped); `list_recent_for_account` newest first with no result |
| P1 | Progression | Tracks accumulate, count versions, keep creation order |
| P2 | Progression | Max guards never regress; `spend` never goes negative |
| P3 | Progression | A write closure that fails rolls back everything it did |
| P4 | Progression | `begin_reforge` needs the exact progress and version and no open reforge; `update_reforge` applies once (a repeat is `false`) |
| P5 | Progression | Prestige levels claimed once with enough progress |
| P6 | Progression | Objectives merge by max, in arrival order |
| P7 | Progression | The grant ledger is exactly once (a replayed id fails; the stored result reads back) |
| P8 | Progression | Escalation: insert-only with none, replace only at the expected version |
| P9 | Progression | The journey updates only at its revision; breadcrumbs and encountered content of another account are none |
| B1 | Bounties | A versioned save inserts once (a second insert none) and updates only at the expected version |
| B2 | Wallet | `increment_balance` on a missing wallet is none and makes none; `create_if_missing` keeps an existing wallet's 5000 over a starter of 1; a debit of 4000 leaves 1000; an over-debit refused; first-written balance order kept |
| B3 | Entitlements | Granted once (`exchange_slot_2` twice: `true`, `false`); `bad name` is a data error; two held |
| B4 | Rollups | Two calls on one day add up: `credited` 105, `bySource.STORE.debited` 42, `bySource.QUEST.credited` 1, as one document |
| Y1 | Parties | A solo party is created once per account with one `PARTY_CREATED` |
| Y2 | Parties | Invite, list (newest first), accept with the size cap, invites consumed (`ACCEPTED`, others `REVOKED`) |
| Y3 | Parties | Accepting moves a member out of the old party (`MOVE` event); an invite from a party the inviter left is dead |
| Y4 | Parties | Leave, kick and promote follow the leader rules; the last member's leave disbands (`DISBAND`, `members: []`) |
| Y5 | Parties | Membership event payloads have the keys of §11.2 in order |
| Y6 | Parties | `set_activity` is revision-guarded; a failed unit of work leaves neither the change nor its event |
| Y7 | Outbox | Oldest first; claimed by one worker; released or reclaimed when stale; published once |
| Y8 | Parties | Every party write is in the change log under its actor |
| U1 | Adapter | (new: restricted has no adapter) Each `AuthStore` method against §12 on SQLite: player writes of a node-local account succeed; a player write for another node's account is a `StoreError`; `erase_account` answers the eight counts in order; `revoke_family` and `revoke_all_sessions` answer their counts; `count_email_code_try` reaching 5 removes the code and answers 5 |

---

## 16. Open questions and behaviour that looks wrong but is kept

1. **Diesel's 16-column limit** (§13.1): a seam decision for the integration pass. Without the feature the crate
   must not declare `launcher_accounts` with all 27 columns at once.
2. **`delete_unverified_registration` has no cut-off in the trait.** Equivalent today (the pending mark never
   moves); if a future change re-arms the mark on resend, the trait needs a `pending_before` argument. A request to
   the auth implementer if wanted.
3. **Soft-deleted keys block inserts.** An upsert base (§10.2), an objective merge, a character create with a reused
   id, `ensure_game_account` for a `user_id` whose account was soft-deleted (the live check misses it, the unique
   `user_id` refuses it): each is a database error, not a fresh row. Kept for parity; the retention purge clears it.
4. **`ON CONFLICT DO NOTHING` sees deleted rows**: a bounty, cooldown or entitlement grant over a soft-deleted row is
   refused (`None` / `false`).
5. **`create_if_missing` over a soft-deleted wallet** answers `a wallet vanished`. Kept.
6. **A credit to a soft-deleted balance row** adds to the deleted row and leaves it deleted; the wallet then reads
   without it. Kept.
7. **Sequence counters include deleted rows and, for characters, outbox and invites, every account.** Kept: only
   order matters.
8. **`update_active_index` answers `true` for the current index even at a stale revision.** Node's behaviour. Kept.
9. **Float text in stored counters** (`1.0`), activity documents and rollups: what `serde_json` writes for an `f64`.
   Node wrote `1`. Monitoring reads only; kept.
10. **A malformed reforge object reads as no reforge**, silently, which `begin_reforge` then treats as "none open".
    Kept.
11. **Outbox and objective-event ids are system-random**, not drawn from the replay id source: drawing from it would
    shift every later seeded id of a replayed request and break the goldens. Kept.
12. **Engine differences the code accepts**: text ordering of user strings (entitlement names, rollup paths, invite
    codes, ids as tie-breakers) follows SQLite's binary order but Postgres's database collation; `lower()` is
    ASCII-only on SQLite and Unicode on Postgres for the signer search; Postgres identity keys are `GENERATED ALWAYS`
    and refuse explicit values (a copy that keeps ids must use `OVERRIDING SYSTEM VALUE`). ISO time comparisons are
    safe on both (fixed width). Kept; worth a DF row if it ever matters.
13. **SQLite schema text.** Comments inside `CREATE TABLE` (in `0055`, `0204`, `0500`) live in an existing database's
    `sqlite_master`. Nothing compares that text, and `store`'s snapshot adoption is for Drizzle databases only, so the
    rewrite may leave comments out; the tables, columns, types, constraints, indexes and versions must match. Proposed:
    write the DDL fresh to §4, comments optional.
14. **`insert_if_absent` on Postgres** spoils the transaction it fails in; callers retry in a new one. Kept.
15. **Expired auth rows are never purged.** `purge_expired` exists for refresh sessions, exchange codes, Discord
    transactions and email codes, but the only purge job (`purge_expired_ttl_rows`, restricted's auth wiring) covers
    the Reforged tables. Proposed: a decision row to add them to the TTL job; parity until then.
16. **New answers in the adapter**: `revoke_family` and `revoke_all_sessions` counts, `count_email_code_try`'s count
    and the read-answers-none rule for an unparseable id have no restricted original (there is no adapter there yet).
    They follow auth's trait docs.
17. **Account columns for `history --account`** (DH3b): restricted never declares isles' account columns, so the host's
    history and revert tools cannot select isles rows by account. Proposed: the crate declares them at open
    (`accounts.id`, `launcher_accounts.account_id`, `characters.account_id`, ... and the `account` columns) with
    `with_account_columns` and `store::declare_accounts`; a behaviour addition, so a decision row.
18. **`delete_player_data` ignores its `user_id`** and the party leader handover in an erasure sends no event and
    bumps no revision (members see the old revision until the next write). Kept.
19. **Debug departures** (§14) are deliberate, as core §17's settings `Debug`.


---

## Coordinator decisions (2026-11-05, before implementation)

These settle the open questions and override anything above that disagrees.

- **Diesel's column limit:** the crate enables `diesel`'s `32-column-tables` feature in its own manifest (features are
  additive, as `serde_json`'s `preserve_order` is per crate); no workspace change.
- **No cut-off on the unverified delete; the try limit:** as proposed (`CODE_MAX_ATTEMPTS`).
- **The adapter's new answers** (revoke counts, an unparseable id reads as none): accepted.
- **Unpurged auth rows, `history --account`:** parity; both are candidate decision rows for the maintainer.
- **SQLite schema comments:** optional, as proposed.
- **Everything listed as parity:** parity.
- **`Debug`:** secrets stay out, as proposed.
- **Security:** the crate gets `docs/security/rustic-isles-store.md` and the `rustic_isles_store_decode` fuzz target.
- **Added with the Reforged admin spec:** `LauncherAccountRepository::set_email(account, normalized email)` (refuses
  an address another live account holds, as registration's uniqueness does), for the admin bootstrap's email rename.
