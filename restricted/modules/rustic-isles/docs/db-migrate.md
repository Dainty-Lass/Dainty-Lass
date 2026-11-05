# Database Migration (`reforged-host db migrate`, `reforged-host db verify` & `reforged-host db export-mongo`)

`crates/isles-db` (rows **RU7**, **RB11**, and **RB13**, exposed in the shipped host kit as `reforged-host db migrate`, `reforged-host db verify`, and `reforged-host db export-mongo`) migrates an existing Dauntless 1.14.7 MongoDB deployment into Rustic Isles' two-engine `store` database (SQLite or Postgres, DD15), verifies per-collection counts and canonical SHA-256 content hashes, and exports `store` back to MongoDB for the one-release rollback path. It replaces all Node `reference/1.14.7/ParadoxBackend/scripts/migrate_*.ts` scripts.

---

## Quick Start

On a shipped server, `reforged-host db` defaults `--from`, `--against`, `--to`, and `--node` from `host.toml` (`[mongo]`, `[database]`, and `[network]`):

1. **Dry-run against the source MongoDB database** (reads and transforms every collection without touching `--to`):

   ```bash
   reforged-host db migrate --from "mongodb://127.0.0.1:27017/reforged" --dry-run
   ```

2. **Migrate into SQLite or Postgres**:

   ```bash
   # Uses host.toml defaults when --from / --to are omitted
   reforged-host db migrate --from "mongodb://127.0.0.1:27017/reforged"
   ```

3. **Verify counts and canonical SHA-256 content hashes**:

   ```bash
   reforged-host db verify --against "mongodb://127.0.0.1:27017/reforged"
   ```

4. **Export back to MongoDB for one-release rollback (RB13)**:

   ```bash
   reforged-host db export-mongo --to "mongodb://127.0.0.1:27017/reforged"
   ```

---

## CLI Options

| Flag | Description |
| --- | --- |
| `--from <mongodb://...\|store>` | Source MongoDB connection URI for `migrate` (or SQLite/Postgres source for `export-mongo`). Defaults from `host.toml` in `reforged-host db`. Strictly read-only. |
| `--against <mongodb://...>` | Source MongoDB connection URI for `verify` (`--from` is also accepted). Defaults from `host.toml` in `reforged-host db`. |
| `--to <path\|postgres://...\|mongodb://...>` | Destination SQLite file path or `postgres://` URL for `migrate`/`verify`, or destination `mongodb://` URI for `export-mongo`. Defaults from `host.toml` in `reforged-host db`. |
| `--node <node-id>` | Node identifier used when deriving deterministic DD18 global IDs (`account:<ULID>@<node>`, `party:<ULID>@<node>`). Defaults to `local`. |
| `--area <area>` | Restrict migration or verification to one or more areas (`auth`, `characters`, `inventory`, `progression`, `bounties`, `parties`, `reforged`). Repeatable or comma-separated. |
| `--dry-run` | Extract and run all RB11 transforms in memory, printing the summary without opening or writing to `--to`. |
| `--skip-invalid` | Log a warning and skip malformed MongoDB documents instead of aborting. |
| `--game-data-dir <dir>` | Path to the directory containing `inventory_storage_policy.json` for RB11 `migrate_inventory_storage_arrays`. |
| `--allow-remote-postgres` | Permit migrating into a non-loopback Postgres address. |
| `--json` | Emit the migration or verification report as pretty-printed JSON. |

---

## Collection-to-Table Mapping by Area

| Area (`--area`) | MongoDB Collections | Target `store` Tables |
| --- | --- | --- |
| `auth` | `accounts`, `authIdentities`, `refreshSessions`, `gameExchangeCodes`, `discordOAuthTransactions`, `emailCodes`, `gameServerApiKeys`, `userApiKeys` | `accounts`, `launcher_accounts`, `auth_identities`, `refresh_sessions`, `game_exchange_codes`, `discord_oauth_transactions`, `email_codes`, `game_server_api_keys`, `user_api_keys` |
| `characters` | `characters`, `loadouts` | `characters`, `loadouts` |
| `inventory` | `inventories`, `inventoryTransactions` | `inventories`, `inventory_transactions` |
| `progression` | `progressionTracks`, `progressionObjectives`, `progressionObjectiveEvents`, `progressionTransactions`, `escalationProgress`, `playerJourney`, `breadcrumbs`, `encounteredContent` | `progression_tracks`, `progression_objectives`, `progression_objective_events`, `progression_grants`, `escalation_progress`, `player_journey`, `breadcrumbs`, `encountered_content` |
| `bounties` | `bountyStates`, `cooldownStates`, `entitlements`, `wallets`, `economyDaily` | `bounty_states`, `cooldown_states`, `entitlements`, `wallets`, `wallet_balances`, `economy_rollup_counters` |
| `parties` | `parties`, `partyMemberships`, `partyInvites`, `partyOutbox`, `friendships` | `parties`, `party_members`, `party_invites`, `party_outbox`, `friend_edges` |
| `reforged` | `reforgedInvites`, `adminSessions`, `adminAudit`, `launcherNotices`, `operationsPolicy`, `launcherGuardSessions`, `guardFindings` | `reforged_invites`, `admin_sessions`, `admin_audit`, `launcher_notices`, `operations_policy`, `launcher_guard_sessions`, `guard_findings` |

---

## DD18 global IDs and idempotency

- Every Node `userId` (`accounts._id` / `userId`) is preserved in `accounts.user_id` (what the game client sees) and mapped to a DD18 `AccountId` (`account:<26-char Crockford Base32 ULID>@<node>`).
- If the destination database already contains a row in `accounts` for `user_id`, its existing `AccountId` is reused; otherwise a deterministic ULID is derived from `SHA-256("isles-db:account:<node>:<userId>")`.
- Party records preserve Node's `partyId` in `parties.client_id` and map to a deterministic `party:<ULID>@<node>` global ID.
- Re-running `isles-db migrate` against an already-migrated database updates rows in place (`UPDATE ... WHERE ...`, falling back to `INSERT` only when absent) without duplicating rows or disturbing sequence numbers.

---

## RB11 In-Flight Data Transforms

`isles-db migrate` applies all five legacy Node `scripts/migrate_*.ts` data repairs during extraction:

1. **`migrate_launcher_approval`**: Launcher accounts without an `approvalStatus` are backfilled to `approval_status = "approved"`, `approval_updated_by = "migration:0.1.73"`, and `approval_updated_at = created_at`.
2. **`migrate_inventory_storage_arrays`**: Normalizes `instancedItems` and `stackedItems` (whether stored in MongoDB as BSON arrays or legacy JSON strings) and relocates items between instanced and stacked arrays when `inventory_storage_policy.json` is provided via `--game-data-dir`.
3. **`migrate_starter_instances`**: Ensures starter equipment items (`STARTER_INSTANCED_CATALOG_IDS`) carry valid, unique 26-character uppercase alphanumeric `instanceId`s and rewrites loadout equipment slots (`weapon`, `helmet`, `chest`, `arms`, `legs`, `lantern`, `flask`) to reference the character's owned starter instances.
4. **`migrate_banner_instances`**: Seeds/repairs starter banner (`BN_BEGINNER_00`) and glider (`GD_FRAME_STARTER_BASE`) instances and rewrites `persistent.banner` (`fabric_id`, `pole_id`, default dyes/sigil) and `persistent.glider` (`instance_id`, default dyes).
5. **`migrate_loadout_slots`**: Reconciles each character's `unlocked_total_slots`, `loadouts` array length (cloning slot 0 with `custom_name = ""` and `update_version = 0`), and `active_index` against the account's unlocked Slayer's Path loadout slot nodes in `player_journey`.
