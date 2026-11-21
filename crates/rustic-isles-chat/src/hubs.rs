// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The hub each account was last sent to (spec §5.10, Reforged patch 0025 recast for DD21): one registry per
//! module, in memory, which matchmaking records into and the region-room rule reads.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard, PoisonError};

use dainty_module_api::{GlobalId, Region};

/// Seven days: a record exactly this old is still live.
pub const HUB_SESSION_TTL_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// Past this many accounts the oldest recorded go first.
pub const MAX_ACCOUNTS: usize = 50_000;

/// Where an account was last sent, and its place in arrival order.
#[derive(Debug, Clone)]
struct Visit {
    hub: String,
    region: Region,
    sent_at: i64,
    serial: u64,
}

impl Visit {
    fn fresh_at(&self, now: i64) -> bool {
        // i128: a replay-pinned extreme cannot overflow; a visit from the future is fresh
        i128::from(now) - i128::from(self.sent_at) <= i128::from(HUB_SESSION_TTL_MS)
    }
}

/// Arrivals are queued oldest first; re-recording an account leaves its earlier arrival behind as a stale entry,
/// skipped when it reaches the front and swept out when stale ones outnumber the live.
#[derive(Debug, Default)]
struct Ledger {
    visits: HashMap<GlobalId, Visit>,
    arrivals: VecDeque<(u64, GlobalId)>,
    serials: u64,
}

impl Ledger {
    fn is_current(&self, serial: u64, account: &GlobalId) -> bool {
        self.visits.get(account).is_some_and(|v| v.serial == serial)
    }

    fn drop_oldest(&mut self) {
        while let Some((serial, account)) = self.arrivals.pop_front() {
            if self.is_current(serial, &account) {
                self.visits.remove(&account);
                return;
            }
        }
    }

    fn sweep_stale(&mut self) {
        if self.arrivals.len() <= 2 * self.visits.len().max(1) {
            return;
        }
        let visits = &self.visits;
        self.arrivals
            .retain(|(serial, account)| visits.get(account).is_some_and(|v| v.serial == *serial));
    }
}

/// Every account's last hub, shared by matchmaking (which records) and the room rules (which read).
#[derive(Debug, Default)]
pub struct HubSessions(Mutex<Ledger>);

impl HubSessions {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    fn ledger(&self) -> MutexGuard<'_, Ledger> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The account's hub is now this one; an empty game session id keeps the old record.
    pub fn record(&self, account: &GlobalId, game_session_id: &str, region: &Region, now: i64) {
        if game_session_id.is_empty() {
            return;
        }
        let mut ledger = self.ledger();
        ledger.serials += 1;
        let serial = ledger.serials;
        let visit = Visit {
            hub: game_session_id.to_owned(),
            region: region.clone(),
            sent_at: now,
            serial,
        };
        ledger.visits.insert(account.clone(), visit);
        ledger.arrivals.push_back((serial, account.clone()));
        while ledger.visits.len() > MAX_ACCOUNTS {
            ledger.drop_oldest();
        }
        ledger.sweep_stale();
    }

    fn fresh_visit(&self, account: &GlobalId, now: i64) -> Option<Visit> {
        let ledger = self.ledger();
        ledger.visits.get(account).filter(|v| v.fresh_at(now)).cloned()
    }

    /// Whether the account's live hub record is in this region (the region-room rule).
    pub fn may_join_region(&self, account: &GlobalId, region: &Region, now: i64) -> bool {
        self.fresh_visit(account, now).is_some_and(|v| &v.region == region)
    }

    /// The region of a live hub with that game session id.
    pub fn region_of_hub(&self, game_session_id: &str, now: i64) -> Option<Region> {
        let ledger = self.ledger();
        let mut fresh = ledger.visits.values().filter(|v| v.fresh_at(now));
        fresh.find(|v| v.hub == game_session_id).map(|v| v.region.clone())
    }

    /// Kept for the day a `City-` name reaches the room query; nothing asks it today.
    pub fn may_join_city_room(&self, account: &GlobalId, game_session_id: &str, now: i64) -> bool {
        self.fresh_visit(account, now).is_some_and(|v| v.hub == game_session_id)
    }

    /// How many accounts have a record, live or stale.
    pub fn len(&self) -> usize {
        self.ledger().visits.len()
    }

    /// True when no account has a record.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dainty_module_api::{IdKind, NodeId};

    fn someone() -> GlobalId {
        GlobalId::new(IdKind::Account, &NodeId::new("local").unwrap())
    }

    fn region(name: &str) -> Region {
        Region::new(name).unwrap()
    }

    #[test]
    fn only_the_last_hub_counts_and_the_seventh_day_is_in() {
        let hubs = HubSessions::new();
        let a = someone();
        hubs.record(&a, "hub-1", &region("local"), 0);
        hubs.record(&a, "hub-2", &region("germany"), 10);
        assert!(!hubs.may_join_region(&a, &region("local"), 11));
        assert!(hubs.may_join_region(&a, &region("germany"), 11));
        assert!(hubs.may_join_city_room(&a, "hub-2", 11));
        assert!(!hubs.may_join_city_room(&a, "hub-1", 11));
        assert_eq!(hubs.region_of_hub("hub-1", 11), None);
        assert_eq!(hubs.region_of_hub("hub-2", 11), Some(region("germany")));
        assert!(hubs.may_join_region(&a, &region("germany"), 10 + HUB_SESSION_TTL_MS));
        assert!(!hubs.may_join_region(&a, &region("germany"), 11 + HUB_SESSION_TTL_MS));
        assert_eq!(hubs.region_of_hub("hub-2", 11 + HUB_SESSION_TTL_MS), None);
        assert!(hubs.may_join_region(&a, &region("germany"), -5));
        hubs.record(&a, "", &region("local"), 20);
        assert!(hubs.may_join_region(&a, &region("germany"), 21));
        assert!(!hubs.may_join_region(&someone(), &region("germany"), 21));
    }

    #[test]
    fn the_cap_drops_the_earliest_arrival() {
        let hubs = HubSessions::new();
        let everyone: Vec<GlobalId> = (0..=MAX_ACCOUNTS).map(|_| someone()).collect();
        let local = region("local");
        for a in &everyone {
            hubs.record(a, "hub", &local, 0);
        }
        assert_eq!(hubs.len(), MAX_ACCOUNTS);
        assert!(!hubs.may_join_region(&everyone[0], &local, 0));
        assert!(hubs.may_join_region(&everyone[1], &local, 0));
        assert!(hubs.may_join_region(&everyone[MAX_ACCOUNTS], &local, 0));
        // A second record moves an account to the back; its old arrival is skipped
        hubs.record(&everyone[1], "hub", &local, 0);
        hubs.record(&someone(), "hub", &local, 0);
        assert!(hubs.may_join_region(&everyone[1], &local, 0));
        assert!(!hubs.may_join_region(&everyone[2], &local, 0));
    }

    #[test]
    fn re_recording_one_account_keeps_the_queue_small() {
        let hubs = HubSessions::new();
        let a = someone();
        for t in 0..1000 {
            hubs.record(&a, "hub", &region("local"), t);
        }
        assert_eq!(hubs.len(), 1);
        assert!(hubs.ledger().arrivals.len() <= 3);
    }
}
