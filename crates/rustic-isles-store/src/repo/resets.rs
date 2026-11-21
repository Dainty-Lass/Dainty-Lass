// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Save resets and their snapshots (`save_snapshots`). A reset moves every save row of the account to the snapshot's
//! own owner id (`reset:<id>`) instead of copying it, so both engines run the same plain `UPDATE`s and a restore is
//! the same move back. The account is left with no save rows, which is a new account's state: every area bootstraps
//! its rows on first read. Identity (accounts, launcher account, friends, parties) and staff records (activity,
//! audit, economy day caps) are not save rows and stay.

use diesel::QueryableByName;
use diesel::sql_types::{BigInt, Nullable, Text};

use crate::sql::{count, sql};
use crate::{AccountId, AnyConnection, RepoError, RepoResult, Tx};

/// A snapshot is kept this long, then pruned with its rows.
pub const SNAPSHOT_KEEP_MS: i64 = 30 * 86_400_000;
/// A player may reset once in this long.
pub const PLAYER_RESET_EVERY_MS: i64 = 86_400_000;
/// The snapshot a player's own reset takes.
pub const SOURCE_PLAYER: &str = "player";
/// The snapshot a staff restore takes of the save it replaces, so a restore can be undone too.
pub const SOURCE_RESTORE: &str = "restore";

/// Every table holding save rows and its account column. Character-keyed rows move with their account column;
/// an `inventories` row without one follows its character, whose id never changes.
pub const SAVE_TABLES: &[(&str, &str)] = &[
    ("characters", "account_id"),
    ("loadouts", "account_id"),
    ("inventories", "account_id"),
    ("inventory_transactions", "account_id"),
    ("breadcrumbs", "account"),
    ("encountered_content", "account"),
    ("progression_tracks", "account"),
    ("progression_objectives", "account"),
    ("progression_objective_events", "account"),
    ("progression_grants", "account"),
    ("escalation_progress", "account"),
    ("player_journey", "account"),
    ("bounty_states", "account_id"),
    ("cooldown_states", "account_id"),
    ("entitlements", "account_id"),
    ("wallet_balances", "account_id"),
    ("wallets", "account_id"),
];

/// A `save_snapshots` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveSnapshot {
    /// The snapshot's id.
    pub id: String,
    /// The account whose save it holds.
    pub account: AccountId,
    /// [`SOURCE_PLAYER`] or [`SOURCE_RESTORE`].
    pub source: String,
    /// The staff userId that took it; none for a player's own reset.
    pub actor_user_id: Option<String>,
    /// When it was taken, unix ms.
    pub created_at: i64,
    /// When it is pruned, unix ms.
    pub expires_at: i64,
    /// When staff restored it, unix ms.
    pub restored_at: Option<i64>,
    /// How many save rows it holds.
    pub rows: u64,
}

#[derive(QueryableByName)]
struct SnapshotRow {
    #[diesel(sql_type = Text)]
    id: String,
    #[diesel(sql_type = Text)]
    account: String,
    #[diesel(sql_type = Text)]
    source: String,
    #[diesel(sql_type = Nullable<Text>)]
    actor_user_id: Option<String>,
    #[diesel(sql_type = BigInt)]
    created_at: i64,
    #[diesel(sql_type = BigInt)]
    expires_at: i64,
    #[diesel(sql_type = Nullable<BigInt>)]
    restored_at: Option<i64>,
}

const SNAPSHOT_COLUMNS: &str = "SELECT id, account, source, actor_user_id, created_at, expires_at, restored_at \
                                FROM save_snapshots";

fn owner_of(snapshot: &str) -> String {
    format!("reset:{snapshot}")
}

fn rows_owned(conn: &mut AnyConnection, owner: &str) -> RepoResult<u64> {
    let mut total = 0;
    for (table, column) in SAVE_TABLES {
        let n = sql(format!("SELECT COUNT(*) AS n FROM {table} WHERE {column} = ?"))
            .t(owner)
            .int(conn)?;
        total += u64::try_from(n).unwrap_or(0);
    }
    Ok(total)
}

fn decode(conn: &mut AnyConnection, row: SnapshotRow) -> RepoResult<SaveSnapshot> {
    let rows = rows_owned(conn, &owner_of(&row.id))?;
    Ok(SaveSnapshot {
        account: crate::sql::account_id(&row.account, "save_snapshots.account")?,
        id: row.id,
        source: row.source,
        actor_user_id: row.actor_user_id,
        created_at: row.created_at,
        expires_at: row.expires_at,
        restored_at: row.restored_at,
        rows,
    })
}

/// Moves every save row owned by `from` to `to`; how many moved.
fn move_rows(conn: &mut AnyConnection, from: &str, to: &str) -> RepoResult<u64> {
    let mut moved = 0;
    for (table, column) in SAVE_TABLES {
        moved += count(
            sql(format!("UPDATE {table} SET {column} = ? WHERE {column} = ?"))
                .t(to)
                .t(from)
                .run(conn)?,
        );
    }
    Ok(moved)
}

/// The account's save moved into a new snapshot `id`, kept [`SNAPSHOT_KEEP_MS`] from `now_ms`; the rows moved.
pub fn take_snapshot(
    tx: &mut Tx<'_>,
    account: &AccountId,
    id: &str,
    source: &str,
    actor_user_id: Option<&str>,
    now_ms: i64,
) -> RepoResult<u64> {
    let conn = tx.conn();
    sql(
        "INSERT INTO save_snapshots (id, account, source, actor_user_id, created_at, expires_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .t(id)
    .t(account.to_string())
    .t(source)
    .ot(actor_user_id)
    .i(now_ms)
    .i(now_ms.saturating_add(SNAPSHOT_KEEP_MS))
    .run(conn)?;
    move_rows(conn, &account.to_string(), &owner_of(id))
}

/// When the account's last own reset was, if any is on record (pruned ones are not).
pub fn last_player_reset(conn: &mut AnyConnection, account: &AccountId) -> RepoResult<Option<i64>> {
    let n = sql("SELECT MAX(created_at) AS n FROM save_snapshots WHERE account = ? AND source = ?")
        .t(account.to_string())
        .t(SOURCE_PLAYER)
        .first::<crate::sql::IntRow>(conn)?;
    Ok(n.and_then(|r| r.n))
}

/// The account's snapshots, newest first.
pub fn list(conn: &mut AnyConnection, account: &AccountId) -> RepoResult<Vec<SaveSnapshot>> {
    let rows = sql(format!(
        "{SNAPSHOT_COLUMNS} WHERE account = ? ORDER BY created_at DESC, id ASC"
    ))
    .t(account.to_string())
    .load::<SnapshotRow>(conn)?;
    rows.into_iter().map(|r| decode(conn, r)).collect()
}

/// One snapshot of the account.
pub fn find(conn: &mut AnyConnection, account: &AccountId, id: &str) -> RepoResult<Option<SaveSnapshot>> {
    let row = sql(format!("{SNAPSHOT_COLUMNS} WHERE account = ? AND id = ?"))
        .t(account.to_string())
        .t(id)
        .first::<SnapshotRow>(conn)?;
    row.map(|r| decode(conn, r)).transpose()
}

/// Puts snapshot `id` back as the account's save. The save it replaces becomes snapshot `replaced_id`
/// ([`SOURCE_RESTORE`]); the restored snapshot is marked restored and holds no rows after. The rows moved back.
pub fn restore(
    tx: &mut Tx<'_>,
    account: &AccountId,
    id: &str,
    replaced_id: &str,
    actor_user_id: &str,
    now_ms: i64,
) -> RepoResult<u64> {
    let found = sql("SELECT id AS t FROM save_snapshots WHERE account = ? AND id = ? AND restored_at IS NULL")
        .t(account.to_string())
        .t(id)
        .load::<crate::sql::TextRow>(tx.conn())?;
    if found.is_empty() {
        return Err(RepoError::data(format!("no restorable snapshot {id}")));
    }
    take_snapshot(tx, account, replaced_id, SOURCE_RESTORE, Some(actor_user_id), now_ms)?;
    let conn = tx.conn();
    let moved = move_rows(conn, &owner_of(id), &account.to_string())?;
    sql("UPDATE save_snapshots SET restored_at = ? WHERE id = ?")
        .i(now_ms)
        .t(id)
        .run(conn)?;
    Ok(moved)
}

/// Deletes every snapshot past its `expires_at`, and its rows; how many snapshots went.
pub fn prune(tx: &mut Tx<'_>, now_ms: i64) -> RepoResult<u64> {
    let conn = tx.conn();
    let expired: Vec<String> = sql("SELECT id AS t FROM save_snapshots WHERE expires_at <= ?")
        .i(now_ms)
        .load::<crate::sql::TextRow>(conn)?
        .into_iter()
        .map(|r| r.t)
        .collect();
    for id in &expired {
        let owner = owner_of(id);
        // Before the characters: an account-less inventory is found only through its character
        sql("DELETE FROM inventories WHERE account_id IS NULL AND character_id IN \
             (SELECT character_id FROM characters WHERE account_id = ?)")
        .t(&*owner)
        .run(conn)?;
        for (table, column) in SAVE_TABLES.iter().rev() {
            sql(format!("DELETE FROM {table} WHERE {column} = ?"))
                .t(&*owner)
                .run(conn)?;
        }
        sql("DELETE FROM save_snapshots WHERE id = ?").t(&**id).run(conn)?;
    }
    Ok(count(expired.len()))
}

/// What a player's own reset did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerReset {
    /// The save went into the snapshot; the rows moved.
    Done(u64),
    /// The last reset was under [`PLAYER_RESET_EVERY_MS`] ago; the next is allowed at this unix ms.
    TooSoon(i64),
}

/// A player's own reset: prunes expired snapshots, refuses a second within [`PLAYER_RESET_EVERY_MS`], else moves the
/// save into snapshot `id`.
pub fn player_reset(tx: &mut Tx<'_>, account: &AccountId, id: &str, now_ms: i64) -> RepoResult<PlayerReset> {
    prune(tx, now_ms)?;
    if let Some(next) = next_player_reset(tx.conn(), account, now_ms)? {
        return Ok(PlayerReset::TooSoon(next));
    }
    take_snapshot(tx, account, id, SOURCE_PLAYER, None, now_ms).map(PlayerReset::Done)
}

/// When the account may next reset, if not now.
pub fn next_player_reset(conn: &mut AnyConnection, account: &AccountId, now_ms: i64) -> RepoResult<Option<i64>> {
    Ok(last_player_reset(conn, account)?
        .map(|last| last.saturating_add(PLAYER_RESET_EVERY_MS))
        .filter(|next| *next > now_ms))
}
