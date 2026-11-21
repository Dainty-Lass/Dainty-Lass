// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Gauntlet progress per account and season (`gauntlet_progress`): the best level a player has finished and the
//! time left on it, from the hunt server's `PUT /process_run`. Dainty Lass's own: the official service's store is
//! unknown.

use diesel::QueryableByName;
use diesel::sql_types::{BigInt, Text};

use crate::sql::{account_id, count, sql};
use crate::{AccountId, AnyConnection, RepoResult, Tx};

/// One account's progress in one season.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GauntletProgress {
    /// The account.
    pub account: AccountId,
    /// The season, such as `ESC_SEASON_1`.
    pub season_id: String,
    /// The highest level finished.
    pub best_level: i64,
    /// Seconds left when the best level was finished, its best run.
    pub best_remaining_sec: i64,
    /// Finished runs recorded.
    pub runs: i64,
}

impl GauntletProgress {
    /// No level finished yet.
    pub fn new(account: AccountId, season_id: &str) -> Self {
        Self {
            account,
            season_id: season_id.to_owned(),
            best_level: 0,
            best_remaining_sec: 0,
            runs: 0,
        }
    }

    /// Counts a finished run; a higher level, or the same level with more time left, becomes the best.
    pub fn record(&mut self, level: i64, remaining_sec: i64) {
        self.runs = self.runs.saturating_add(1);
        let remaining_sec = remaining_sec.max(0);
        if level > self.best_level || (level == self.best_level && remaining_sec > self.best_remaining_sec) {
            self.best_level = level;
            self.best_remaining_sec = remaining_sec;
        }
    }
}

/// A leaderboard line: the player's name and best run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GauntletBoardEntry {
    /// The account's display name.
    pub name: String,
    /// The highest level finished.
    pub level: i64,
    /// Seconds left on it.
    pub remaining_sec: i64,
}

#[derive(QueryableByName)]
struct Row {
    #[diesel(sql_type = Text)]
    account: String,
    #[diesel(sql_type = Text)]
    season_id: String,
    #[diesel(sql_type = BigInt)]
    best_level: i64,
    #[diesel(sql_type = BigInt)]
    best_remaining_sec: i64,
    #[diesel(sql_type = BigInt)]
    runs: i64,
}

impl Row {
    fn decode(self) -> RepoResult<GauntletProgress> {
        Ok(GauntletProgress {
            account: account_id(&self.account, "gauntlet_progress.account")?,
            season_id: self.season_id,
            best_level: self.best_level,
            best_remaining_sec: self.best_remaining_sec,
            runs: self.runs,
        })
    }
}

#[derive(QueryableByName)]
struct BoardRow {
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = BigInt)]
    best_level: i64,
    #[diesel(sql_type = BigInt)]
    best_remaining_sec: i64,
}

/// The account's progress in the season; none before its first finished run.
pub fn find(conn: &mut AnyConnection, account: &AccountId, season_id: &str) -> RepoResult<Option<GauntletProgress>> {
    sql(
        "SELECT account, season_id, best_level, best_remaining_sec, runs FROM gauntlet_progress \
         WHERE account = ? AND season_id = ?",
    )
    .t(account.to_string())
    .t(season_id)
    .first::<Row>(conn)?
    .map(Row::decode)
    .transpose()
}

/// Writes the record, replacing the account's row for its season.
pub fn save(tx: &mut Tx<'_>, record: &GauntletProgress, updated_at: &str) -> RepoResult<()> {
    let conn = tx.conn();
    let updated = sql(
        "UPDATE gauntlet_progress SET best_level = ?, best_remaining_sec = ?, runs = ?, updated_at = ? \
         WHERE account = ? AND season_id = ?",
    )
    .i(record.best_level)
    .i(record.best_remaining_sec)
    .i(record.runs)
    .t(updated_at)
    .t(record.account.to_string())
    .t(record.season_id.clone())
    .run(conn)?;
    if updated == 0 {
        sql(
            "INSERT INTO gauntlet_progress (account, season_id, best_level, best_remaining_sec, runs, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .t(record.account.to_string())
        .t(record.season_id.clone())
        .i(record.best_level)
        .i(record.best_remaining_sec)
        .i(record.runs)
        .t(updated_at)
        .run(conn)?;
    }
    Ok(())
}

/// The season's best runs, highest level first, then most time left; accounts that are gone are left out.
pub fn leaderboard(conn: &mut AnyConnection, season_id: &str, limit: i64) -> RepoResult<Vec<GauntletBoardEntry>> {
    Ok(sql(
        "SELECT a.name AS name, g.best_level AS best_level, g.best_remaining_sec AS best_remaining_sec \
         FROM gauntlet_progress g JOIN accounts a ON a.id = g.account \
         WHERE g.season_id = ? AND g.best_level > 0 AND a.deleted_at IS NULL \
         ORDER BY g.best_level DESC, g.best_remaining_sec DESC, a.name ASC LIMIT ?",
    )
    .t(season_id)
    .i(limit)
    .load::<BoardRow>(conn)?
    .into_iter()
    .map(|r| GauntletBoardEntry {
        name: r.name,
        level: r.best_level,
        remaining_sec: r.best_remaining_sec,
    })
    .collect())
}

pub(crate) fn erase(conn: &mut AnyConnection, account: &AccountId) -> RepoResult<u64> {
    Ok(count(
        sql("DELETE FROM gauntlet_progress WHERE account = ?")
            .t(account.to_string())
            .run(conn)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> GauntletProgress {
        GauntletProgress::new(
            AccountId::new(&dainty_module_api::NodeId::new("n1").unwrap()),
            "ESC_SEASON_1",
        )
    }

    #[test]
    fn the_best_run_is_the_highest_level_then_the_most_time_left() {
        let mut r = record();
        r.record(1, 526);
        assert_eq!((r.best_level, r.best_remaining_sec, r.runs), (1, 526, 1));
        r.record(1, 300);
        assert_eq!((r.best_level, r.best_remaining_sec), (1, 526));
        r.record(1, 600);
        assert_eq!((r.best_level, r.best_remaining_sec), (1, 600));
        r.record(2, 10);
        assert_eq!((r.best_level, r.best_remaining_sec), (2, 10));
        r.record(1, 900);
        assert_eq!((r.best_level, r.best_remaining_sec, r.runs), (2, 10, 5));
        r.record(3, -5);
        assert_eq!(r.best_remaining_sec, 0, "a negative time left counts as none");
    }
}
