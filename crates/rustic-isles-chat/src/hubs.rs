// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The hub each account was last sent to (spec §5.10, Reforged patch 0025 recast for DD21): one registry per
//! module, in memory, which matchmaking records into and the region-room rule reads.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, MutexGuard, PoisonError};

use module_api::{GlobalId, Region};

/// Seven days: a record exactly this old is still live.
pub const HUB_SESSION_TTL_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// Past this many accounts the oldest recorded go first.
pub const MAX_ACCOUNTS: usize = 50_000;

#[derive(Debug, Clone)]
struct HubSession {
    game_session_id: String,
    region: Region,
    at: i64,
    order: u64,
}

impl HubSession {
    fn is_live(&self, now: i64) -> bool {
        i128::from(now) - i128::from(self.at) <= i128::from(HUB_SESSION_TTL_MS)
    }
}

#[derive(Debug, Default)]
struct Inner {
    by_account: HashMap<GlobalId, HubSession>,
    /// Record order, oldest first
    order: BTreeMap<u64, GlobalId>,
    next: u64,
}

#[derive(Debug, Default)]
pub struct HubSessions(Mutex<Inner>);

impl HubSessions {
    pub fn new() -> Self {
        Self::default()
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The account's hub is now this one; an empty game session id keeps the old record.
    pub fn record(&self, account: &GlobalId, game_session_id: &str, region: &Region, now: i64) {
        if game_session_id.is_empty() {
            return;
        }
        let mut inner = self.inner();
        let order = inner.next;
        inner.next += 1;
        let session = HubSession {
            game_session_id: game_session_id.to_owned(),
            region: region.clone(),
            at: now,
            order,
        };
        if let Some(old) = inner.by_account.insert(account.clone(), session) {
            inner.order.remove(&old.order);
        }
        inner.order.insert(order, account.clone());
        while inner.by_account.len() > MAX_ACCOUNTS {
            let Some((_, oldest)) = inner.order.pop_first() else {
                break;
            };
            inner.by_account.remove(&oldest);
        }
    }

    fn live(&self, account: &GlobalId, now: i64) -> Option<HubSession> {
        self.inner().by_account.get(account).filter(|s| s.is_live(now)).cloned()
    }

    pub fn may_join_region(&self, account: &GlobalId, region: &Region, now: i64) -> bool {
        self.live(account, now).is_some_and(|s| &s.region == region)
    }

    /// The region of a live hub with that game session id.
    pub fn region_of_hub(&self, game_session_id: &str, now: i64) -> Option<Region> {
        self.inner()
            .by_account
            .values()
            .find(|s| s.game_session_id == game_session_id && s.is_live(now))
            .map(|s| s.region.clone())
    }

    /// Kept for the day a `City-` name reaches the room query; nothing asks it today.
    pub fn may_join_city_room(&self, account: &GlobalId, game_session_id: &str, now: i64) -> bool {
        self.live(account, now)
            .is_some_and(|s| s.game_session_id == game_session_id)
    }

    pub fn len(&self) -> usize {
        self.inner().by_account.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use module_api::{IdKind, NodeId};

    fn account() -> GlobalId {
        GlobalId::new(IdKind::Account, &NodeId::new("local").unwrap())
    }

    fn region(name: &str) -> Region {
        Region::new(name).unwrap()
    }

    #[test]
    fn last_hub_only_and_the_ttl_is_inclusive() {
        let hubs = HubSessions::new();
        let a = account();
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
        // From the future: live
        assert!(hubs.may_join_region(&a, &region("germany"), -5));
        hubs.record(&a, "", &region("local"), 20);
        assert!(hubs.may_join_region(&a, &region("germany"), 21));
        assert!(!hubs.may_join_region(&account(), &region("germany"), 21));
    }

    #[test]
    fn the_oldest_go_first() {
        let hubs = HubSessions::new();
        let accounts: Vec<GlobalId> = (0..=MAX_ACCOUNTS).map(|_| account()).collect();
        let local = region("local");
        for a in &accounts {
            hubs.record(a, "hub", &local, 0);
        }
        assert_eq!(hubs.len(), MAX_ACCOUNTS);
        assert!(!hubs.may_join_region(&accounts[0], &local, 0));
        assert!(hubs.may_join_region(&accounts[1], &local, 0));
        assert!(hubs.may_join_region(&accounts[MAX_ACCOUNTS], &local, 0));
        // Re-recording makes an account the newest
        hubs.record(&accounts[1], "hub", &local, 0);
        hubs.record(&account(), "hub", &local, 0);
        assert!(hubs.may_join_region(&accounts[1], &local, 0));
        assert!(!hubs.may_join_region(&accounts[2], &local, 0));
    }
}
