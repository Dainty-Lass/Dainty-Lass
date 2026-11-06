//! How far shipping has fallen behind (DD23: the status page and the admin API alert). Worked out from the
//! database and the shipper's state file, not from anything the shipper reports, so a shipper that was
//! stopped still shows: its heartbeat ages and the oldest unshipped entry does too.

use std::path::Path;
use std::time::Duration;

use serde::Serialize;
use store::Database;

use crate::Error;
use crate::ship::ShipState;

/// How far behind shipping may fall before [`assess`] raises an alert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LagLimits {
    /// The oldest entry not shipped yet may be this old.
    pub max_unshipped: Duration,
    /// The shipper's last round may be this long ago.
    pub max_silence: Duration,
    /// The last snapshot may be this old.
    pub max_snapshot_age: Duration,
}

impl Default for LagLimits {
    /// Five minutes for the log and the heartbeat, two days for a daily snapshot.
    fn default() -> Self {
        Self {
            max_unshipped: Duration::from_secs(5 * 60),
            max_silence: Duration::from_secs(5 * 60),
            max_snapshot_age: Duration::from_secs(48 * 60 * 60),
        }
    }
}

/// Why shipping counts as behind or broken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LagAlert {
    /// Entries since `since` (the oldest one's time) are only on this server.
    Unshipped {
        /// The oldest unshipped entry's time.
        since: i64,
        /// Its id.
        oldest_id: i64,
    },
    /// The shipper hasn't run since `since`, or never has.
    Silent {
        /// The last heartbeat, `None` if there never was one.
        since: Option<i64>,
    },
    /// No snapshot since `since`, or never.
    SnapshotStale {
        /// When the last snapshot was taken, `None` if never.
        since: Option<i64>,
    },
    /// The local log fails the hash chain (DH5): the shipper refused to ship it on. Always an alert.
    HistoryBroken {
        /// Where the chain breaks.
        error: String,
    },
    /// The last round failed (shown, not counted as behind by itself).
    Failing {
        /// The last round's error.
        error: String,
        /// When it failed.
        at: Option<i64>,
    },
}

/// What the status page and the admin API show. Times are unix milliseconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Lag {
    /// The id of the newest entry in the bucket.
    pub last_shipped_id: i64,
    /// When that entry was written.
    pub last_shipped_at: Option<i64>,
    /// The id of the newest entry in the local log.
    pub newest_id: i64,
    /// When the oldest entry not shipped yet was written.
    pub oldest_unshipped_at: Option<i64>,
    /// The shipper's last tick, whether or not it succeeded.
    pub heartbeat_at: Option<i64>,
    /// When the last snapshot uploaded was taken.
    pub last_snapshot_at: Option<i64>,
    /// The hash of the newest entry shipped (DH5): the head to compare with the owner's own copy.
    pub shipped_head_hash: Option<String>,
    /// What is wrong, empty when shipping is on time.
    pub alerts: Vec<LagAlert>,
}

impl Lag {
    /// Whether to alert: anything but a failure that hasn't made shipping late yet.
    pub fn behind(&self) -> bool {
        self.alerts.iter().any(|a| !matches!(a, LagAlert::Failing { .. }))
    }
}

fn millis(d: Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
}

/// The alerts for a state, the newest entry and the oldest unshipped one (`(id, at)`), at `now`.
pub fn assess(
    state: &ShipState,
    newest: Option<(i64, i64)>,
    oldest_unshipped: Option<(i64, i64)>,
    limits: &LagLimits,
    now: i64,
) -> Lag {
    let mut alerts = Vec::new();
    if let Some((id, at)) = oldest_unshipped
        && now.saturating_sub(at) > millis(limits.max_unshipped)
    {
        alerts.push(LagAlert::Unshipped {
            since: at,
            oldest_id: id,
        });
    }
    if state
        .heartbeat_at
        .is_none_or(|at| now.saturating_sub(at) > millis(limits.max_silence))
    {
        alerts.push(LagAlert::Silent {
            since: state.heartbeat_at,
        });
    }
    if state
        .last_snapshot_at
        .is_none_or(|at| now.saturating_sub(at) > millis(limits.max_snapshot_age))
    {
        alerts.push(LagAlert::SnapshotStale {
            since: state.last_snapshot_at,
        });
    }
    if let Some(error) = &state.chain_broken {
        alerts.push(LagAlert::HistoryBroken { error: error.clone() });
    }
    if let Some(error) = &state.last_error {
        alerts.push(LagAlert::Failing {
            error: error.clone(),
            at: state.last_error_at,
        });
    }
    Lag {
        last_shipped_id: state.last_shipped_id,
        last_shipped_at: state.last_upload_at,
        newest_id: newest.map_or(0, |(id, _)| id),
        oldest_unshipped_at: oldest_unshipped.map(|(_, at)| at),
        heartbeat_at: state.heartbeat_at,
        last_snapshot_at: state.last_snapshot_at,
        shipped_head_hash: state.last_shipped_hash.clone(),
        alerts,
    }
}

/// The lag of the shipper whose state is in `state_file`, against `db`, now by the database's clock.
pub async fn lag(db: &Database, state_file: &Path, limits: &LagLimits) -> Result<Lag, Error> {
    let state = ShipState::load(state_file)?;
    let newest = db.last_change().await?;
    let oldest = db
        .changes(state.last_shipped_id, 1)
        .await?
        .first()
        .map(|e| (e.id, e.at));
    Ok(assess(&state, newest, oldest, limits, db.now()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stopped_shipper_alerts_once_its_entries_age() {
        let limits = LagLimits::default();
        let state = ShipState {
            last_shipped_id: 10,
            heartbeat_at: Some(1_000_000),
            last_snapshot_at: Some(1_000_000),
            ..ShipState::default()
        };
        let fresh = assess(&state, Some((10, 999_000)), None, &limits, 1_010_000);
        assert!(!fresh.behind(), "{fresh:?}");

        // Shipping stopped at 1_000_000, a write at 1_005_000, then six minutes passed.
        let later = 1_005_000 + 6 * 60 * 1000;
        let lag = assess(&state, Some((11, 1_005_000)), Some((11, 1_005_000)), &limits, later);
        assert!(lag.behind());
        assert!(lag.alerts.contains(&LagAlert::Unshipped {
            since: 1_005_000,
            oldest_id: 11
        }));
        assert!(lag.alerts.contains(&LagAlert::Silent { since: Some(1_000_000) }));
    }

    #[test]
    fn a_shipper_that_never_ran_alerts_and_a_failure_alone_does_not() {
        let lag = assess(&ShipState::default(), None, None, &LagLimits::default(), 5);
        assert!(lag.behind());
        let state = ShipState {
            heartbeat_at: Some(5),
            last_snapshot_at: Some(5),
            last_error: Some("bucket: GET: 503".into()),
            ..ShipState::default()
        };
        let lag = assess(&state, None, None, &LagLimits::default(), 6);
        assert!(!lag.behind());
        assert_eq!(lag.alerts.len(), 1);
    }
}
