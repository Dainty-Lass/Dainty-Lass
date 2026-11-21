// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! What one module keeps in memory (spec §5.2, §5.8, §5.9, DD15): queues, candidates, party instances, launch refs,
//! hub game session ids, ring instances and game presence.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock, Weak};

use dainty_module_api::{GlobalId, HomeApi};
use rustic_isles::Shared;
use rustic_isles_economy::inventory::{RingInstances, RingInstancesSlot};

use crate::candidate::MatchCandidate;

/// Two hours: a game not seen for this long is forgotten.
pub const PRESENCE_STALE_MS: i64 = 2 * 60 * 60 * 1000;
/// Past this many users a touch prunes the stale.
pub const PRESENCE_PRUNE_AT: usize = 100_000;

/// The activity rows a dedicated hunt leaves (the Reforged unit implements it); never awaited by a launch.
pub trait HuntActivity: Send + Sync {
    /// A dedicated hunt started for `users`.
    fn dedicated_hunt_starts(&self, users: &[String], hunt_id: &str, hunt_ref: &str);
    /// A dedicated hunt ended for `users`, with the reason (such as `RETURNED`).
    fn dedicated_hunt_ends(&self, users: &[String], end_reason: &str);
}

/// A [`HuntActivity`] that records nothing.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoHuntActivity;

impl HuntActivity for NoHuntActivity {
    fn dedicated_hunt_starts(&self, _: &[String], _: &str, _: &str) {}

    fn dedicated_hunt_ends(&self, _: &[String], _: &str) {}
}

/// Core's replay-aware clock plus an offset a test moves forward; never the system clock directly.
#[derive(Debug, Default)]
pub struct MatchClock {
    offset: AtomicI64,
}

impl MatchClock {
    /// The replay-aware time plus the test offset, in Unix ms.
    pub fn now_ms(&self) -> i64 {
        rustic_isles::replay::now_ms().saturating_add(self.offset.load(Ordering::Relaxed))
    }

    /// Moves the clock forward by `ms`.
    #[cfg(feature = "test-support")]
    pub fn advance(&self, ms: i64) {
        self.offset.fetch_add(ms, Ordering::Relaxed);
    }
}

/// `GameLastSeen`: the last time each user's running game touched the backend.
#[derive(Debug, Default)]
pub struct GamePresence(Mutex<HashMap<String, i64>>);

impl GamePresence {
    fn seen(&self) -> MutexGuard<'_, HashMap<String, i64>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Only moves forward; an empty user is ignored.
    pub fn touch(&self, user: &str, now: i64) {
        if user.is_empty() {
            return;
        }
        let mut seen = self.seen();
        let entry = seen.entry(user.to_owned()).or_insert(now);
        *entry = (*entry).max(now);
        if seen.len() > PRESENCE_PRUNE_AT {
            seen.retain(|_, last| now.saturating_sub(*last) <= PRESENCE_STALE_MS);
        }
    }

    /// Whether the user's game touched the backend within `within` ms of `now`.
    pub fn is_recently_active(&self, user: &str, within: i64, now: i64) -> bool {
        self.seen()
            .get(user)
            .is_some_and(|last| now.saturating_sub(*last) <= within)
    }

    /// The operator dashboard's online count.
    pub fn recently_active(&self, within: i64, now: i64) -> Vec<String> {
        let mut users: Vec<String> = self
            .seen()
            .iter()
            .filter(|(_, last)| now.saturating_sub(**last) <= within)
            .map(|(user, _)| user.clone())
            .collect();
        users.sort();
        users
    }

    /// Drops users not seen for the stale window.
    pub fn forget_stale(&self, now: i64) {
        self.seen()
            .retain(|_, last| now.saturating_sub(*last) <= PRESENCE_STALE_MS);
    }

    /// How many users are tracked.
    pub fn len(&self) -> usize {
        self.seen().len()
    }

    /// Whether no user is tracked.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Queue {
    /// In join order, a player twice when he joined twice (spec §15 item 5)
    pub players: Vec<String>,
    pub last_add: i64,
    /// Launching: a join waits for nothing more here
    pub resolved: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct PartyInstance {
    pub candidate: MatchCandidate,
    pub made_at: i64,
}

#[derive(Debug, Default)]
pub(crate) struct Tables {
    pub queues: HashMap<String, Queue>,
    pub candidates: HashMap<String, MatchCandidate>,
    pub party_instances: HashMap<String, PartyInstance>,
    pub launch_refs: HashMap<String, String>,
    pub hub_sessions: HashMap<String, String>,
    pub ring_instances: HashMap<String, GlobalId>,
    pub last_ring_instance: Option<GlobalId>,
    /// Each player's last Gauntlet join's `gauntlet_level`, for the launch that follows
    pub gauntlet_levels: HashMap<String, i64>,
}

impl Tables {
    /// `CancelHuntMatchmaking` for each distinct player.
    pub fn cancel_hunt(&mut self, hunt: &str, players: &[String], reason: &str) {
        let mut seen = HashSet::new();
        for player in players {
            if !seen.insert(player.as_str()) {
                continue;
            }
            let id = rustic_isles::replay::next_uuid();
            let candidate = MatchCandidate {
                canceled: true,
                status_reason: Some(reason.to_owned()),
                ..MatchCandidate::matching(hunt, &id)
            };
            self.candidates.insert(player.clone(), candidate);
        }
    }

    pub fn in_flight(&self, now: i64) -> usize {
        self.candidates.values().filter(|c| c.ready_at > now).count()
    }

    pub fn hub_game_session_id(&mut self, host: &str, port: i64) -> String {
        self.hub_sessions
            .entry(format!("{host}:{port}"))
            .or_insert_with(rustic_isles::replay::next_uuid)
            .clone()
    }
}

/// One per module, in a core slot.
#[derive(Default)]
pub struct MatchmakingState {
    tables: Mutex<Tables>,
    clock: MatchClock,
    presence: GamePresence,
    activity: RwLock<Option<Arc<dyn HuntActivity>>>,
    shared: RwLock<Option<Weak<Shared>>>,
    home: RwLock<Option<Arc<dyn HomeApi>>>,
}

impl std::fmt::Debug for MatchmakingState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let tables = self.tables();
        f.debug_struct("MatchmakingState")
            .field("candidates", &tables.candidates.len())
            .field("queues", &tables.queues.len())
            .field("party_instances", &tables.party_instances.len())
            .finish_non_exhaustive()
    }
}

impl MatchmakingState {
    /// Empty state with no hunt activity bound.
    pub fn new() -> Self {
        Self::default()
    }

    /// Each step between two awaits is one critical section, so the only interleavings are Node's.
    pub(crate) fn tables(&self) -> MutexGuard<'_, Tables> {
        self.tables.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The clock every matchmaking time comes from.
    pub fn clock(&self) -> &MatchClock {
        &self.clock
    }

    /// Matchmaking's current time, Unix ms.
    pub fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }

    /// When each user's game last touched the backend.
    pub fn presence(&self) -> &GamePresence {
        &self.presence
    }

    /// Sets where dedicated hunt starts and ends are recorded.
    pub fn set_hunt_activity(&self, activity: Arc<dyn HuntActivity>) {
        *self.activity.write().unwrap_or_else(PoisonError::into_inner) = Some(activity);
    }

    pub(crate) fn hunt_activity(&self) -> Arc<dyn HuntActivity> {
        let held = self.activity.read().unwrap_or_else(PoisonError::into_inner).clone();
        held.unwrap_or_else(|| Arc::new(NoHuntActivity))
    }

    /// The module state the rifts core asks for the Director slot: rifts is built before `Shared`, so the first
    /// handler or start-up binds it.
    pub fn bind_shared(&self, shared: &Arc<Shared>) {
        let mut held = self.shared.write().unwrap_or_else(PoisonError::into_inner);
        if held.as_ref().and_then(Weak::upgrade).is_none() {
            *held = Some(Arc::downgrade(shared));
        }
    }

    pub(crate) fn shared(&self) -> Option<Arc<Shared>> {
        self.shared
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()?
            .upgrade()
    }

    /// A home for the ring calls in place of the module's (tests).
    #[cfg(feature = "test-support")]
    pub fn set_home(&self, home: Arc<dyn HomeApi>) {
        *self.home.write().unwrap_or_else(PoisonError::into_inner) = Some(home);
    }

    pub(crate) fn home(&self, shared: &Shared) -> Arc<dyn HomeApi> {
        let held = self.home.read().unwrap_or_else(PoisonError::into_inner).clone();
        held.unwrap_or_else(|| shared.home())
    }

    /// The stored candidate (not client-visible).
    pub fn candidate(&self, user: &str) -> Option<MatchCandidate> {
        self.tables().candidates.get(user).cloned()
    }

    /// The erasure blocker; a cancelled candidate counts too.
    pub fn has_candidate(&self, user: &str) -> bool {
        self.tables().candidates.contains_key(user)
    }

    /// Stores or replaces the user's candidate.
    pub fn set_candidate(&self, user: &str, candidate: MatchCandidate) {
        self.tables().candidates.insert(user.to_owned(), candidate);
    }

    /// A cancelled search is a taken join, so this always answers true.
    pub fn cancel_hunt(&self, hunt: &str, players: &[String], reason: &str) -> bool {
        self.tables().cancel_hunt(hunt, players, reason);
        true
    }

    /// Stable for the life of the hub (its `City-<id>` room).
    pub fn hub_game_session_id(&self, host: &str, port: i64) -> String {
        self.tables().hub_game_session_id(host, port)
    }

    /// Whether the id is one of the live hubs' game sessions.
    pub fn is_hub_game_session_id(&self, id: &str) -> bool {
        self.tables().hub_sessions.values().any(|v| v == id)
    }

    /// Each player maps to the instance, which also becomes the last ring instance (an empty list included).
    pub fn record_ring_instance(&self, instance: &GlobalId, players: &[String]) {
        let mut tables = self.tables();
        for player in players {
            tables.ring_instances.insert(player.clone(), instance.clone());
        }
        tables.last_ring_instance = Some(instance.clone());
    }

    /// The player's instance, else the last ring instance.
    pub fn ring_instance_for_player(&self, player: &str) -> Option<GlobalId> {
        let tables = self.tables();
        tables
            .ring_instances
            .get(player)
            .or(tables.last_ring_instance.as_ref())
            .cloned()
    }
}

impl RingInstances for MatchmakingState {
    fn ring_instance_for_player(&self, player: &GlobalId) -> Option<GlobalId> {
        MatchmakingState::ring_instance_for_player(self, &player.to_string())
    }
}

/// Economy's slot over this state.
pub fn ring_instances_slot(state: &Arc<MatchmakingState>) -> RingInstancesSlot {
    RingInstancesSlot(Arc::clone(state) as Arc<dyn RingInstances>)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dainty_module_api::{IdKind, NodeId};

    #[test]
    fn presence_moves_forward_and_forgets() {
        let p = GamePresence::default();
        p.touch("u", 1000);
        p.touch("u", 500);
        assert!(p.is_recently_active("u", 75_000, 76_000));
        assert!(!p.is_recently_active("u", 75_000, 76_001));
        p.touch("", 1);
        assert_eq!(p.len(), 1);
        assert_eq!(p.recently_active(75_000, 76_000), ["u"]);
        p.forget_stale(1000 + PRESENCE_STALE_MS + 1);
        assert!(p.is_empty());
    }

    #[test]
    fn hub_ids_and_cancels() {
        let s = MatchmakingState::new();
        let a = s.hub_game_session_id("h", 1);
        assert_eq!((a.clone(), a.len()), (s.hub_game_session_id("h", 1), 36));
        assert_ne!(a, s.hub_game_session_id("h", 2));
        assert!(s.is_hub_game_session_id(&a) && !s.is_hub_game_session_id("ShatteredIsles_IslandA"));
        let players = ["a", "b", "a"].map(String::from);
        assert!(s.cancel_hunt("h", &players, "WHY"));
        let (ca, cb) = (s.candidate("a").unwrap(), s.candidate("b").unwrap());
        for c in [&ca, &cb] {
            assert!(c.canceled && !c.ready);
            assert_eq!(c.status_reason.as_deref(), Some("WHY"));
            assert_eq!(c.game_session_id, c.candidate_id);
        }
        assert_ne!(ca.candidate_id, cb.candidate_id);
        assert!(s.has_candidate("a"));
    }

    #[test]
    fn ring_instances() {
        let s = Arc::new(MatchmakingState::new());
        let node = NodeId::new("local").unwrap();
        let (hunt, hub) = (
            GlobalId::new(IdKind::Hunt, &node),
            GlobalId::new(IdKind::HubInstance, &node),
        );
        assert_eq!(s.ring_instance_for_player("p"), None);
        s.record_ring_instance(&hunt, &["p".to_owned()]);
        s.record_ring_instance(&hub, &[]);
        assert_eq!(s.ring_instance_for_player("p"), Some(hunt));
        assert_eq!(s.ring_instance_for_player("q"), Some(hub.clone()));
        let slot = ring_instances_slot(&s);
        let foreign = GlobalId::new(IdKind::Account, &NodeId::new("germany-1").unwrap());
        assert_eq!(slot.0.ring_instance_for_player(&foreign), Some(hub));
    }
}
