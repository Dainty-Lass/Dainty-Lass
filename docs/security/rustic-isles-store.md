<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: Rustic Isles store (RX3)

The checklist for `crates/rustic-isles-store`, the MIT database layer of Rustic Isles (RD27,
`docs/port/spec-rustic-isles-store.md` §14). It opens no listener and parses no request, so it is not on RX3's
trust-boundary list; what it guards is the data at rest and the home rule.

How to use it: before a change to this crate merges, and before its cutover is signed (RD10), copy this list into the
change, check each item in the code (not from memory), link the code or test that shows it, and sign at the end with the
date and the commit reviewed.

## What it is and where the boundary is

- **In:** closures from the areas and the auth crate (`IslesStore::read`, `write`, `account_write`), and the auth
  trait calls (`IslesAuthStore`). Every value they pass is bound into SQL.
- **Out:** one `store::Database` (SQLite or Postgres), its change log, and the home API's `resolve_home`.
- **Stored text it reads back** (its own columns, so a damaged or hand-edited database is the threat): roles,
  Guard details, bounty and cooldown lists, reforges, grant results, notices, activity documents.

Code: `crates/rustic-isles-store/src/` (`lib.rs`, `sql.rs`, `adapter.rs`, `repo/*.rs`), the migrations in
`crates/rustic-isles-store/migrations/{sqlite,postgres}`.

Fuzz target in `fuzz`: `rustic_isles_store_decode`: arbitrary text as stored roles, a Guard `detail`, bounty
`bounties`/`draft_data`, cooldown `entries`, a reforge, a grant `result` and notices lists, and arbitrary dotted paths
and amounts into the rollup document and the daily counters never panic and answer a value or a data error; the rollup
stays an object; `resolve_visible_total`, `resolve_requested_total`, `resolve_active_index` and `wire_counts` never
panic for any input and `wire_counts` stays in 1..6.

## Checklist

### The home rule (DD17)

- [ ] `account_write` resolves the account's home before the closure runs: another node's account is `NotHome`, an
  unresolvable home is `Home(..)`; only `UnknownAccount` for an account minted here proceeds (`lib.rs`
  `check_home`; test `s2_another_nodes_account_is_refused_on_a_standalone_store`).
- [ ] The adapter makes every `Player { account }` write an `account_write` for that account, so a player actor can
  never write another node's account (`adapter.rs` `write`; test `u1_the_adapter_follows_the_store_rules`).
- [ ] Plain `write`s are used only for rows that belong to no one account or for system and admin actors.

### Every write is logged (DD23)

- [ ] No connection that can write exists outside `store::Database::write`; repository writes take a `Tx`.
- [ ] The actor mapping is §2.3's (`store_actor`): `system:isles`, `host:console`, `admin:<key>`, `player:<account>`,
  `system:<module>` (test `s1_an_admin_account_write_on_its_home_is_logged`).
- [ ] The sixteen player-facing tables are soft-deleted (`SOFT_DELETE`); an erasure and an admin deletion run in one
  transaction (`erase_account`, `delete_player_data`, `erase_other_areas`), so either everything goes or nothing.

### Secrets

- [ ] `Debug` of every record holding a secret leaves it out: password hashes (shown only as `<set>`), refresh token
  hashes, exchange code hashes, Discord states, verifiers and completion hashes, email code hashes, game server and
  user key hashes, admin session token hashes and CSRF tokens, Guard challenges and next challenges (tests in
  `auth.rs`, `reforged.rs`).
- [ ] Only hashes are stored; no plaintext key, code or token reaches a column.
- [ ] The crate logs nothing of its own; callers log its errors, whose texts carry no secret column.

### SQL

- [ ] Every value is bound, never spliced (`sql.rs`: written with `?`, numbered by `store::portable::numbered`).
  The only text put into SQL is fixed table and column names, limits from `u32`s, and the `?` lists for `IN`.
- [ ] The signer search lower-cases the needle and escapes `\`, `%`, `_` for `LIKE .. ESCAPE '\'` (test
  `launcher_guard_sessions_and_findings`).

### Names and documents

- [ ] Entitlement names (1-128 of `[A-Za-z0-9_.-]`) and balance catalog ids (1-128 of `[A-Za-z0-9_]`) are checked
  before they are stored.
- [ ] Rollup and activity paths are split on `.` only to build a document, never used as SQL.
- [ ] A stored row that does not decode is a `Data` error, never a panic (fuzz target).

### Replay

- [ ] Closures run in the caller's replay scope; the store adds no way to set one. Outbox and objective-event ids
  are system-random, never drawn from the replay source.

### Known weak spots (kept for parity, spec §16)

- Expired refresh sessions, exchange codes, Discord transactions and email codes are never purged by a job.
- A soft-deleted key blocks a new insert (characters, tracks, objectives, wallets, a reused `user_id`).
- `insert_if_absent` on Postgres spoils the transaction it fails in; the caller retries in a new one.
- Postgres orders user strings by the database collation and lower-cases Unicode; SQLite is binary and ASCII.
- The account columns for `history --account` are not declared (a candidate decision row).

## Sign-off

Reviewed by: ______ on ______ at commit ______.
