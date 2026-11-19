// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Online playtime per account (`playtime`): chat presence opens and closes it, a periodic tick credits the time
//! of the accounts still online, so a crash loses at most the time since the last tick.

use diesel::QueryableByName;
use diesel::sql_types::{BigInt, Nullable, Text};

use crate::sql::{account_id, count, sql};
use crate::{AccountId, AnyConnection, RepoResult, Tx};

/// How often the module credits the accounts online.
pub const TICK_MS: i64 = 5 * 60_000;

/// A gap longer than this between two credits means the session died unseen (a crash, a missed offline): it is
/// not counted.
pub const STALE_MS: i64 = 2 * TICK_MS + 60_000;

/// One account's playtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaytimeRecord {
    /// The account.
    pub account: AccountId,
    /// Online time credited so far.
    pub online_ms: i64,
    /// When the open session was last credited; none while offline.
    pub last_seen_ms: Option<i64>,
}

impl PlaytimeRecord {
    /// An account with no time yet.
    pub fn new(account: AccountId) -> Self {
        Self {
            account,
            online_ms: 0,
            last_seen_ms: None,
        }
    }

    /// Whether a session is open.
    pub fn is_open(&self) -> bool {
        self.last_seen_ms.is_some()
    }

    /// Adds the time since the last credit, unless it is stale or runs backwards.
    fn credit(&mut self, now_ms: i64) {
        if let Some(seen) = self.last_seen_ms {
            let gap = now_ms.saturating_sub(seen);
            if (0..=STALE_MS).contains(&gap) {
                self.online_ms = self.online_ms.saturating_add(gap);
            }
        }
    }

    /// The player came online. A second connection while one is open counts the same wall time once.
    pub fn open(&mut self, now_ms: i64) {
        self.credit(now_ms);
        self.last_seen_ms = Some(now_ms);
    }

    /// The player went offline.
    pub fn close(&mut self, now_ms: i64) {
        self.credit(now_ms);
        self.last_seen_ms = None;
    }

    /// The periodic credit of an open session; a stale one is closed uncounted.
    pub fn tick(&mut self, now_ms: i64) {
        let Some(seen) = self.last_seen_ms else {
            return;
        };
        if now_ms.saturating_sub(seen) > STALE_MS {
            self.last_seen_ms = None;
            return;
        }
        self.open(now_ms);
    }

    /// The total, with the open session's time not yet credited.
    pub fn total_ms(&self, now_ms: i64) -> i64 {
        let mut live = self.clone();
        live.credit(now_ms);
        live.online_ms
    }
}

#[derive(QueryableByName)]
struct Row {
    #[diesel(sql_type = Text)]
    account: String,
    #[diesel(sql_type = BigInt)]
    online_ms: i64,
    #[diesel(sql_type = Nullable<BigInt>)]
    last_seen_ms: Option<i64>,
}

impl Row {
    fn decode(self) -> RepoResult<PlaytimeRecord> {
        Ok(PlaytimeRecord {
            account: account_id(&self.account, "playtime.account")?,
            online_ms: self.online_ms,
            last_seen_ms: self.last_seen_ms,
        })
    }
}

/// The account's playtime; none before its first session.
pub fn find(conn: &mut AnyConnection, account: &AccountId) -> RepoResult<Option<PlaytimeRecord>> {
    sql("SELECT account, online_ms, last_seen_ms FROM playtime WHERE account = ?")
        .t(account.to_string())
        .first::<Row>(conn)?
        .map(Row::decode)
        .transpose()
}

/// Every account with a session open.
pub fn find_open(conn: &mut AnyConnection) -> RepoResult<Vec<PlaytimeRecord>> {
    sql("SELECT account, online_ms, last_seen_ms FROM playtime WHERE last_seen_ms IS NOT NULL ORDER BY account")
        .load::<Row>(conn)?
        .into_iter()
        .map(Row::decode)
        .collect()
}

/// Writes the record, replacing the account's row.
pub fn save(tx: &mut Tx<'_>, record: &PlaytimeRecord) -> RepoResult<()> {
    let conn = tx.conn();
    let updated = sql("UPDATE playtime SET online_ms = ?, last_seen_ms = ? WHERE account = ?")
        .i(record.online_ms)
        .oi(record.last_seen_ms)
        .t(record.account.to_string())
        .run(conn)?;
    if updated == 0 {
        sql("INSERT INTO playtime (account, online_ms, last_seen_ms) VALUES (?, ?, ?)")
            .t(record.account.to_string())
            .i(record.online_ms)
            .oi(record.last_seen_ms)
            .run(conn)?;
    }
    Ok(())
}

/// Closes every open session uncounted: at start no one is online yet, whatever a crash left behind.
pub fn close_all(tx: &mut Tx<'_>) -> RepoResult<u64> {
    Ok(count(
        sql("UPDATE playtime SET last_seen_ms = NULL WHERE last_seen_ms IS NOT NULL").run(tx.conn())?,
    ))
}

pub(crate) fn erase(conn: &mut AnyConnection, account: &AccountId) -> RepoResult<u64> {
    Ok(count(
        sql("DELETE FROM playtime WHERE account = ?")
            .t(account.to_string())
            .run(conn)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> PlaytimeRecord {
        PlaytimeRecord::new(AccountId::new(&dainty_module_api::NodeId::new("n1").unwrap()))
    }

    #[test]
    fn open_then_close_counts_the_session() {
        let mut r = record();
        r.open(1_000);
        assert!(r.is_open());
        r.close(61_000);
        assert!(!r.is_open());
        assert_eq!(r.online_ms, 60_000);
        r.close(90_000);
        assert_eq!(r.online_ms, 60_000, "a second close adds nothing");
    }

    #[test]
    fn ticks_credit_a_long_session() {
        let mut r = record();
        r.open(0);
        for n in 1..=12 {
            r.tick(n * TICK_MS);
        }
        assert_eq!(r.online_ms, 12 * TICK_MS);
        r.close(12 * TICK_MS + 30_000);
        assert_eq!(r.online_ms, 12 * TICK_MS + 30_000);
    }

    #[test]
    fn a_crash_without_close_loses_only_the_time_since_the_last_tick() {
        let mut r = record();
        r.open(0);
        r.tick(TICK_MS);
        // The server died here; the next sign-in comes a day later
        r.open(86_400_000);
        assert_eq!(r.online_ms, TICK_MS);
        r.close(86_400_000 + 1_000);
        assert_eq!(r.online_ms, TICK_MS + 1_000);
        let mut s = record();
        s.open(0);
        s.tick(STALE_MS + 1);
        assert!(!s.is_open(), "a stale session is closed");
        assert_eq!(s.online_ms, 0);
    }

    #[test]
    fn overlapping_sessions_count_wall_time_once() {
        let mut r = record();
        r.open(0);
        r.open(10_000);
        r.tick(TICK_MS);
        r.close(TICK_MS + 5_000);
        assert_eq!(r.online_ms, TICK_MS + 5_000);
    }

    #[test]
    fn the_total_includes_the_open_session_and_never_runs_backwards() {
        let mut r = record();
        r.open(100_000);
        assert_eq!(r.total_ms(160_000), 60_000);
        assert_eq!(r.total_ms(50_000), 0);
        r.close(50_000);
        assert_eq!(r.online_ms, 0);
    }
}
