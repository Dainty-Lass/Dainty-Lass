// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The launcher API's per-module state (public spec §12): readiness, metrics, the status cache, the recent-activity
//! map, the launcher feed's log dedup and the matchmaking slots. Restricted keeps these process-wide; here each lives
//! in the module's slot (core §17 item 14, extended).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Instant;

use rustic_isles::Shared;
use serde_json::Value;

use crate::metrics::Metrics;
use crate::util::{HOUR_MS, now_ms};

const STATUS_TTL_MS: i64 = 5_000;
const GAME_DATA_TTL_MS: i64 = 60_000;
const RECENT_LIMIT: usize = 50_000;

/// Matchmaking's presence touch (unit 8 fills it).
pub trait MatchmakingPresence: Send + Sync {
    /// Marks the user's game as seen at `at_ms`.
    fn touch(&self, user_id: &str, at_ms: i64);
}

/// Whether matchmaking holds a live candidate for the user (unit 8 fills it).
pub trait MatchmakingCandidates: Send + Sync {
    /// Whether the user has a candidate.
    fn has_candidate(&self, user_id: &str) -> bool;
}

/// Matchmaking's own state fills both slots (the composer passes its `MatchmakingState`).
impl MatchmakingPresence for rustic_isles_matchmaking::MatchmakingState {
    fn touch(&self, user_id: &str, at_ms: i64) {
        self.presence().touch(user_id, at_ms);
    }
}

impl MatchmakingCandidates for rustic_isles_matchmaking::MatchmakingState {
    fn has_candidate(&self, user_id: &str) -> bool {
        rustic_isles_matchmaking::MatchmakingState::has_candidate(self, user_id)
    }
}

/// Whether the backend is ready, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Readiness {
    /// Whether it is ready.
    pub ready: bool,
    /// Why, such as `API_BEHIND_PROXY_RUNNING`.
    pub reason: String,
    /// Unix ms of the last change, taken the first time it is read
    pub changed_ms: i64,
}

struct ReadinessCell {
    ready: bool,
    reason: String,
    changed_ms: Option<i64>,
}

#[derive(Default)]
struct StatusCache {
    full: Option<(i64, Value)>,
    limited: Option<(i64, Value)>,
}

/// The two views of `ServerStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusView {
    /// The view for an eligible key holder.
    Full,
    /// The view for everyone else.
    Limited,
}

/// The Reforged area's per-module state: readiness, metrics, caches and the matchmaking slots.
pub struct LauncherApiState {
    started: Instant,
    readiness: Mutex<ReadinessCell>,
    metrics: Metrics,
    status: Mutex<StatusCache>,
    game_data: Mutex<Option<(i64, String)>>,
    recent: Mutex<HashMap<String, i64>>,
    launcher_warning: Mutex<Option<(String, String)>>,
    presence: RwLock<Option<Arc<dyn MatchmakingPresence>>>,
    candidates: RwLock<Option<Arc<dyn MatchmakingCandidates>>>,
    admin: std::sync::OnceLock<Arc<crate::admin::LauncherApiAdmin>>,
    login_queue: Mutex<crate::queue::LoginQueue>,
}

impl std::fmt::Debug for LauncherApiState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LauncherApiState").finish_non_exhaustive()
    }
}

impl Default for LauncherApiState {
    fn default() -> Self {
        Self::new()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl LauncherApiState {
    /// A ready state with empty caches and no matchmaking slots filled.
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            readiness: Mutex::new(ReadinessCell {
                ready: true,
                reason: "API_BEHIND_PROXY_RUNNING".to_owned(),
                changed_ms: None,
            }),
            metrics: Metrics::default(),
            status: Mutex::new(StatusCache::default()),
            game_data: Mutex::new(None),
            recent: Mutex::new(HashMap::new()),
            launcher_warning: Mutex::new(None),
            presence: RwLock::new(None),
            candidates: RwLock::new(None),
            admin: std::sync::OnceLock::new(),
            login_queue: Mutex::new(crate::queue::LoginQueue::default()),
        }
    }

    /// The admin half's state when the composer put none in the slots (production Discord, the economy's funding).
    pub(crate) fn admin_fallback(&self) -> Arc<crate::admin::LauncherApiAdmin> {
        Arc::clone(self.admin.get_or_init(Default::default))
    }

    /// The module's state, or a fresh one when the composer put none in the slots (each call then starts over).
    pub fn of(shared: &Shared) -> Arc<Self> {
        shared.slot::<Self>().unwrap_or_else(|| Arc::new(Self::new()))
    }

    /// The login queue (`queue.rs`).
    pub fn login_queue(&self) -> std::sync::MutexGuard<'_, crate::queue::LoginQueue> {
        lock(&self.login_queue)
    }

    /// The module's metrics.
    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    /// Seconds since the state was made.
    pub fn uptime_seconds(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    /// The readiness; the change time is fixed the first time it is read.
    pub fn readiness(&self) -> Readiness {
        let mut cell = lock(&self.readiness);
        let changed_ms = *cell.changed_ms.get_or_insert_with(now_ms);
        Readiness {
            ready: cell.ready,
            reason: cell.reason.clone(),
            changed_ms,
        }
    }

    /// For a listener plan; the reason is cut to 96 characters.
    pub fn set_readiness(&self, ready: bool, reason: &str) {
        let mut cell = lock(&self.readiness);
        cell.ready = ready;
        cell.reason = reason.chars().take(96).collect();
        cell.changed_ms = Some(now_ms());
    }

    // -------------------------------------------------------------- status cache

    /// The cached body of a view, unless the clock is pinned.
    pub fn cached_status(&self, view: StatusView) -> Option<Value> {
        if rustic_isles::replay::is_replay_pinned() {
            return None;
        }
        let cache = lock(&self.status);
        let entry = match view {
            StatusView::Full => &cache.full,
            StatusView::Limited => &cache.limited,
        };
        let now = now_ms();
        entry
            .as_ref()
            .filter(|(at, _)| now - at < STATUS_TTL_MS && now >= *at)
            .map(|(_, body)| body.clone())
    }

    /// Caches a view's body, unless the clock is pinned.
    pub fn store_status(&self, view: StatusView, body: &Value) {
        if rustic_isles::replay::is_replay_pinned() {
            return;
        }
        let mut cache = lock(&self.status);
        let entry = Some((now_ms(), body.clone()));
        match view {
            StatusView::Full => cache.full = entry,
            StatusView::Limited => cache.limited = entry,
        }
    }

    /// Drops both cached status bodies.
    pub fn forget_status(&self) {
        *lock(&self.status) = StatusCache::default();
    }

    pub(crate) fn cached_game_data(&self) -> Option<String> {
        let now = now_ms();
        lock(&self.game_data)
            .as_ref()
            .filter(|(at, _)| now - at < GAME_DATA_TTL_MS && now >= *at)
            .map(|(_, kind)| kind.clone())
    }

    pub(crate) fn store_game_data(&self, kind: &str) {
        *lock(&self.game_data) = Some((now_ms(), kind.to_owned()));
    }

    /// Whether `(path, reason)` differs from the last launcher-feed refusal logged (and records it).
    pub(crate) fn launcher_warning_changed(&self, path: &str, reason: &str) -> bool {
        let mut last = lock(&self.launcher_warning);
        let next = (path.to_owned(), reason.to_owned());
        if last.as_ref() == Some(&next) {
            return false;
        }
        *last = Some(next);
        true
    }

    // -------------------------------------------------------------- who is online

    /// Notes the user at `at_ms`; past 50 000 entries those older than an hour before it are dropped.
    pub fn touch(&self, user: &str, at_ms: i64) {
        let mut recent = lock(&self.recent);
        recent.insert(user.to_owned(), at_ms);
        if recent.len() > RECENT_LIMIT {
            let floor = at_ms - HOUR_MS;
            recent.retain(|_, seen| *seen >= floor);
        }
    }

    /// The users seen within `window_ms` of now (inclusive), in text order.
    pub fn recently_active(&self, window_ms: i64) -> Vec<String> {
        let now = now_ms();
        let mut seen: Vec<String> = lock(&self.recent)
            .iter()
            .filter(|(_, at)| now - **at <= window_ms)
            .map(|(user, _)| user.clone())
            .collect();
        seen.sort();
        seen
    }

    /// Core's online user ids, then the recently active, each once.
    pub fn active_players(&self, shared: &Shared, window_ms: i64) -> Vec<String> {
        let mut out = shared.presence().online_user_ids();
        for user in self.recently_active(window_ms) {
            if !out.contains(&user) {
                out.push(user);
            }
        }
        out
    }

    // -------------------------------------------------------------- matchmaking slots

    /// Fills matchmaking's presence slot.
    pub fn set_matchmaking_presence(&self, presence: Arc<dyn MatchmakingPresence>) {
        *self.presence.write().unwrap_or_else(PoisonError::into_inner) = Some(presence);
    }

    /// Fills matchmaking's candidates slot.
    pub fn set_matchmaking_candidates(&self, candidates: Arc<dyn MatchmakingCandidates>) {
        *self.candidates.write().unwrap_or_else(PoisonError::into_inner) = Some(candidates);
    }

    /// Matchmaking's presence, none until filled.
    pub fn matchmaking_presence(&self) -> Option<Arc<dyn MatchmakingPresence>> {
        self.presence.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Whether matchmaking holds a candidate for the user; false until filled.
    pub fn has_matchmaking_candidate(&self, user_id: &str) -> bool {
        let candidates = self.candidates.read().unwrap_or_else(PoisonError::into_inner).clone();
        candidates.is_some_and(|c| c.has_candidate(user_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_and_recent() {
        let state = LauncherApiState::new();
        let first = state.readiness();
        assert!(first.ready);
        assert_eq!(first.reason, "API_BEHIND_PROXY_RUNNING");
        assert_eq!(state.readiness().changed_ms, first.changed_ms);
        state.set_readiness(false, &"x".repeat(200));
        assert_eq!(state.readiness().reason.len(), 96);
        let now = now_ms();
        state.touch("u1", now);
        state.touch("u2", now - 120_000);
        assert_eq!(state.recently_active(60_000), ["u1"]);
        assert_eq!(state.recently_active(180_000), ["u1", "u2"]);
    }
}
