// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Trials results per account (`trials_runs`): each player's best finished run of a Trial at a difficulty, solo per
//! weapon and in a group, from the hunt server's `POST /trials/leaderboards`. Dainty Lass's own: the official
//! service's store is unknown.

use diesel::QueryableByName;
use diesel::sql_types::{BigInt, Double, Text};

use crate::sql::{account_id, count, sql};
use crate::{AccountId, AnyConnection, RepoResult, Tx};

/// Whether a run was played alone or in a group; each has its own board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrialMode {
    /// One player.
    Solo,
    /// Two or more.
    Group,
}

impl TrialMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Solo => "solo",
            Self::Group => "group",
        }
    }
}

/// One account's best run of a Trial at a difficulty, in one mode (and, solo, with one weapon).
#[derive(Debug, Clone, PartialEq)]
pub struct TrialRun {
    /// The account.
    pub account: AccountId,
    /// The Trial, as the hunt server names it.
    pub trial_id: String,
    /// The difficulty.
    pub difficulty: i64,
    /// Solo or group.
    pub mode: TrialMode,
    /// The weapon it was played with; 0 for a group run, whose weapons are in `party`.
    pub weapon: i64,
    /// The run's time, lower is better.
    pub completion_time: f64,
    /// Objectives completed.
    pub objectives_completed: i64,
    /// The player's role.
    pub player_role_id: String,
    /// The player's platform.
    pub platform: String,
    /// The group's players, sorted, so one group's rows share it; the userId for a solo run.
    pub party_key: String,
    /// The run's players as the board shows them (JSON, written by the caller).
    pub party: String,
    /// Finished runs recorded.
    pub runs: i64,
}

impl TrialRun {
    /// Counts a finished run; a lower time becomes the best, with what came with it.
    pub fn record(&mut self, run: &Self) {
        self.runs = self.runs.saturating_add(1);
        if run.completion_time < self.completion_time {
            let runs = self.runs;
            *self = run.clone();
            self.runs = runs;
        }
    }
}

/// A board line: the run and its player's userId and name.
#[derive(Debug, Clone, PartialEq)]
pub struct TrialBoardEntry {
    /// The account's userId.
    pub user_id: String,
    /// The account's display name.
    pub name: String,
    /// The run.
    pub run: TrialRun,
}

const COLUMNS: &str = "r.account AS account, r.trial_id AS trial_id, r.difficulty AS difficulty, r.mode AS mode, \
     r.weapon AS weapon, r.completion_time AS completion_time, r.objectives_completed AS objectives_completed, \
     r.player_role_id AS player_role_id, r.platform AS platform, r.party_key AS party_key, r.party AS party, \
     r.runs AS runs";

#[derive(QueryableByName)]
struct Row {
    #[diesel(sql_type = Text)]
    account: String,
    #[diesel(sql_type = Text)]
    trial_id: String,
    #[diesel(sql_type = BigInt)]
    difficulty: i64,
    #[diesel(sql_type = Text)]
    mode: String,
    #[diesel(sql_type = BigInt)]
    weapon: i64,
    #[diesel(sql_type = Double)]
    completion_time: f64,
    #[diesel(sql_type = BigInt)]
    objectives_completed: i64,
    #[diesel(sql_type = Text)]
    player_role_id: String,
    #[diesel(sql_type = Text)]
    platform: String,
    #[diesel(sql_type = Text)]
    party_key: String,
    #[diesel(sql_type = Text)]
    party: String,
    #[diesel(sql_type = BigInt)]
    runs: i64,
}

impl Row {
    fn decode(self) -> RepoResult<TrialRun> {
        Ok(TrialRun {
            account: account_id(&self.account, "trials_runs.account")?,
            trial_id: self.trial_id,
            difficulty: self.difficulty,
            mode: if self.mode == "group" {
                TrialMode::Group
            } else {
                TrialMode::Solo
            },
            weapon: self.weapon,
            completion_time: self.completion_time,
            objectives_completed: self.objectives_completed,
            player_role_id: self.player_role_id,
            platform: self.platform,
            party_key: self.party_key,
            party: self.party,
            runs: self.runs,
        })
    }
}

#[derive(QueryableByName)]
struct BoardRow {
    #[diesel(sql_type = Text)]
    user_id: String,
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(embed)]
    row: Row,
}

/// The account's best run in that mode (and weapon); none before its first.
pub fn find(
    conn: &mut AnyConnection,
    account: &AccountId,
    trial_id: &str,
    difficulty: i64,
    mode: TrialMode,
    weapon: i64,
) -> RepoResult<Option<TrialRun>> {
    sql(format!(
        "SELECT {COLUMNS} FROM trials_runs r \
         WHERE r.account = ? AND r.trial_id = ? AND r.difficulty = ? AND r.mode = ? AND r.weapon = ?"
    ))
    .t(account.to_string())
    .t(trial_id)
    .i(difficulty)
    .t(mode.as_str())
    .i(weapon)
    .first::<Row>(conn)?
    .map(Row::decode)
    .transpose()
}

/// Writes the record, replacing the account's row for its Trial, difficulty, mode and weapon.
pub fn save(tx: &mut Tx<'_>, record: &TrialRun, updated_at: &str) -> RepoResult<()> {
    let conn = tx.conn();
    let updated = sql(
        "UPDATE trials_runs SET completion_time = ?, objectives_completed = ?, player_role_id = ?, platform = ?, \
         party_key = ?, party = ?, runs = ?, updated_at = ? \
         WHERE account = ? AND trial_id = ? AND difficulty = ? AND mode = ? AND weapon = ?",
    )
    .f(record.completion_time)
    .i(record.objectives_completed)
    .t(record.player_role_id.clone())
    .t(record.platform.clone())
    .t(record.party_key.clone())
    .t(record.party.clone())
    .i(record.runs)
    .t(updated_at)
    .t(record.account.to_string())
    .t(record.trial_id.clone())
    .i(record.difficulty)
    .t(record.mode.as_str())
    .i(record.weapon)
    .run(conn)?;
    if updated == 0 {
        sql(
            "INSERT INTO trials_runs (account, trial_id, difficulty, mode, weapon, completion_time, \
             objectives_completed, player_role_id, platform, party_key, party, runs, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .t(record.account.to_string())
        .t(record.trial_id.clone())
        .i(record.difficulty)
        .t(record.mode.as_str())
        .i(record.weapon)
        .f(record.completion_time)
        .i(record.objectives_completed)
        .t(record.player_role_id.clone())
        .t(record.platform.clone())
        .t(record.party_key.clone())
        .t(record.party.clone())
        .i(record.runs)
        .t(updated_at)
        .run(conn)?;
    }
    Ok(())
}

/// Every best run of the Trial at the difficulty in the mode, fastest first; accounts that are gone are left out.
/// The caller picks the lines a board shows (one per player, or one per group).
pub fn runs(
    conn: &mut AnyConnection,
    trial_id: &str,
    difficulty: i64,
    mode: TrialMode,
) -> RepoResult<Vec<TrialBoardEntry>> {
    sql(format!(
        "SELECT a.user_id AS user_id, a.name AS name, {COLUMNS} \
         FROM trials_runs r JOIN accounts a ON a.id = r.account \
         WHERE r.trial_id = ? AND r.difficulty = ? AND r.mode = ? AND a.deleted_at IS NULL \
         ORDER BY r.completion_time ASC, r.updated_at ASC, a.name ASC"
    ))
    .t(trial_id)
    .i(difficulty)
    .t(mode.as_str())
    .load::<BoardRow>(conn)?
    .into_iter()
    .map(|b| {
        Ok(TrialBoardEntry {
            user_id: b.user_id,
            name: b.name,
            run: b.row.decode()?,
        })
    })
    .collect()
}

pub(crate) fn erase(conn: &mut AnyConnection, account: &AccountId) -> RepoResult<u64> {
    Ok(count(
        sql("DELETE FROM trials_runs WHERE account = ?")
            .t(account.to_string())
            .run(conn)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(time: f64, objectives: i64) -> TrialRun {
        TrialRun {
            account: AccountId::new(&dainty_module_api::NodeId::new("n1").unwrap()),
            trial_id: "CR19_PlayerHunt_Arena_Elite".into(),
            difficulty: 1,
            mode: TrialMode::Solo,
            weapon: 1,
            completion_time: time,
            objectives_completed: objectives,
            player_role_id: String::new(),
            platform: "WIN".into(),
            party_key: "u1".into(),
            party: "[]".into(),
            runs: 0,
        }
    }

    #[test]
    fn the_best_run_is_the_fastest() {
        let mut best = run(f64::INFINITY, 0);
        best.record(&run(300.0, 1));
        assert_eq!(
            (best.completion_time, best.objectives_completed, best.runs),
            (300.0, 1, 1)
        );
        best.record(&run(400.0, 5));
        assert_eq!(
            (best.completion_time, best.objectives_completed, best.runs),
            (300.0, 1, 2)
        );
        best.record(&run(250.5, 2));
        assert_eq!(
            (best.completion_time, best.objectives_completed, best.runs),
            (250.5, 2, 3)
        );
    }
}
