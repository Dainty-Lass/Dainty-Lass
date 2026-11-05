//! Reforged patch 0025 (`realtime/cityRooms.ts`): who may join a City (Ramsgate or dojo) chat room. The client names
//! a hub's room `City-<gameSessionId>`, from the `gameSessionId` `/candidate/status` served when it sent the player
//! to that hub, so the backend knows which players were sent to which hub. DD21 turns City rooms into one room per
//! region, and the plugin's question carries the region, not the session id: the rule becomes "sent to a hub in this
//! region within the last week". The exact rule, by session id, is kept as [`HubSessions::may_join_city_room`] for
//! the day the room's name reaches the module.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock, PoisonError};

use module_api::{GlobalId, Region};

/// `HUB_SESSION_TTL_MS`: seven days.
pub const HUB_SESSION_TTL_MS: u64 = 7 * 24 * 60 * 60 * 1000;
/// `MAX_ACCOUNTS`: the oldest records go first.
pub const MAX_ACCOUNTS: usize = 50_000;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Served {
    game_session_id: String,
    region: Region,
    at_ms: u64,
}

#[derive(Debug, Default)]
struct State {
    by_account: HashMap<GlobalId, (u64, Served)>,
    /// Insertion order, as a JavaScript `Map` keeps it
    order: BTreeMap<u64, GlobalId>,
    next: u64,
}

/// The hub each account was last sent to, in memory (matchmaking tickets and presence stay in memory too, DD15).
#[derive(Debug, Default)]
pub struct HubSessions {
    state: Mutex<State>,
}

impl HubSessions {
    pub fn new() -> Self {
        Self::default()
    }

    /// `RecordHubSessionServed`: called when matchmaking tells an account to travel to a hub of `region` with this game
    /// session id. The matchmaking area calls it from `/candidate/status`.
    pub fn record(&self, account: &GlobalId, game_session_id: &str, region: &Region, now_ms: u64) {
        if game_session_id.is_empty() {
            return;
        }
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((seq, _)) = state.by_account.remove(account) {
            state.order.remove(&seq);
        }
        let seq = state.next;
        state.next += 1;
        state.order.insert(seq, account.clone());
        state.by_account.insert(
            account.clone(),
            (
                seq,
                Served {
                    game_session_id: game_session_id.to_owned(),
                    region: region.clone(),
                    at_ms: now_ms,
                },
            ),
        );
        while state.by_account.len() > MAX_ACCOUNTS {
            let Some((_, oldest)) = state.order.pop_first() else {
                break;
            };
            state.by_account.remove(&oldest);
        }
    }

    fn live(&self, account: &GlobalId, now_ms: u64) -> Option<Served> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let (_, served) = state.by_account.get(account)?;
        // `now - at > TTL` is expired
        (now_ms.saturating_sub(served.at_ms) <= HUB_SESSION_TTL_MS).then(|| served.clone())
    }

    /// `MayJoinCityRoom`: `City-<id>` where `<id>` is the hub session the account was last sent to.
    pub fn may_join_city_room(&self, account: &GlobalId, game_session_id: &str, now_ms: u64) -> bool {
        self.live(account, now_ms)
            .is_some_and(|served| served.game_session_id == game_session_id)
    }

    /// The region room: the account was last sent to a hub of `region`.
    pub fn may_join_region(&self, account: &GlobalId, region: &Region, now_ms: u64) -> bool {
        self.live(account, now_ms)
            .is_some_and(|served| &served.region == region)
    }

    /// The region a live hub session belongs to, when any player has been sent to `game_session_id`.
    pub fn region_of_hub(&self, game_session_id: &str, now_ms: u64) -> Option<Region> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.by_account.values().find_map(|(_, served)| {
            (now_ms.saturating_sub(served.at_ms) <= HUB_SESSION_TTL_MS && served.game_session_id == game_session_id)
                .then(|| served.region.clone())
        })
    }
}

/// The node's one registry, which matchmaking records into and the chat rules read.
pub fn hub_sessions() -> &'static HubSessions {
    static SESSIONS: OnceLock<HubSessions> = OnceLock::new();
    SESSIONS.get_or_init(HubSessions::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use module_api::{IdKind, NodeId};

    const HUB: &str = "179cf2a8-0716-49f5-9027-6e4876ba0d76";
    const OTHER: &str = "c069ba56-fe67-40b7-8e19-cb4a4813dc22";

    fn account() -> GlobalId {
        GlobalId::new(IdKind::Account, &"oregon-1".parse::<NodeId>().unwrap())
    }

    fn region() -> Region {
        "oregon".parse().unwrap()
    }

    /// cityRooms.test.ts: "only the hub an account was last sent to opens its City room"
    #[test]
    fn only_the_hub_an_account_was_last_sent_to_opens_its_city_room() {
        let hubs = HubSessions::new();
        let (alice, bob) = (account(), account());
        assert!(!hubs.may_join_city_room(&alice, HUB, 0));
        hubs.record(&alice, HUB, &region(), 0);
        assert!(hubs.may_join_city_room(&alice, HUB, 0));
        assert!(!hubs.may_join_city_room(&alice, OTHER, 0));
        assert!(!hubs.may_join_city_room(&bob, HUB, 0));
        hubs.record(&alice, OTHER, &region(), 0);
        assert!(!hubs.may_join_city_room(&alice, HUB, 0));
        assert!(hubs.may_join_city_room(&alice, OTHER, 0));
        let eight_days = 8 * 24 * 3600 * 1000;
        assert!(!hubs.may_join_city_room(&alice, OTHER, eight_days));
        assert!(
            hubs.may_join_city_room(&alice, OTHER, HUB_SESSION_TTL_MS),
            "the seventh day is still in"
        );
    }

    #[test]
    fn the_region_room_is_open_to_the_players_sent_to_a_hub_of_that_region() {
        let hubs = HubSessions::new();
        let (alice, bob) = (account(), account());
        let germany: Region = "germany".parse().unwrap();
        hubs.record(&alice, HUB, &region(), 1000);
        assert!(hubs.may_join_region(&alice, &region(), 2000));
        assert!(
            !hubs.may_join_region(&alice, &germany, 2000),
            "another region's room stays shut"
        );
        assert!(!hubs.may_join_region(&bob, &region(), 2000));
        hubs.record(&alice, OTHER, &germany, 3000);
        assert!(hubs.may_join_region(&alice, &germany, 3000));
        assert!(
            !hubs.may_join_region(&alice, &region(), 3000),
            "only the last hub counts"
        );
        assert!(!hubs.may_join_region(&alice, &germany, 3000 + HUB_SESSION_TTL_MS + 1));
    }

    #[test]
    fn an_empty_session_id_records_nothing_and_the_oldest_records_go_first() {
        let hubs = HubSessions::new();
        let first = account();
        hubs.record(&first, "", &region(), 0);
        assert!(!hubs.may_join_region(&first, &region(), 0));
        hubs.record(&first, HUB, &region(), 0);
        for _ in 0..MAX_ACCOUNTS {
            hubs.record(&account(), HUB, &region(), 0);
        }
        assert!(
            !hubs.may_join_region(&first, &region(), 0),
            "evicted by the 50 001st account"
        );
    }
}
