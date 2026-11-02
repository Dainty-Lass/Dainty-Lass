//! Who is online as matchmaking sees it, in memory (DD15): when each account's running game last touched the backend
//! (`activityMonitor.ts`'s `GameLastSeen`). Which hub each account was last sent to is the chat crate's
//! (`isles_chat::hub_sessions`).

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

const SESSION_GAP_MS: i64 = 2 * 60 * 60 * 1000;
const MAX_TRACKED: usize = 100_000;

/// `TouchGameSession` and `IsGameRecentlyActive`: a game session's start, the matchmaking heartbeat (about every 5 s)
/// and the realm presence heartbeat (every 30 s) each touch an account. The activity monitor's playtime counting is
/// the reforged-api area's; it only needs the touches this records.
#[derive(Debug, Default)]
pub struct GamePresence {
    last_seen: Mutex<HashMap<String, i64>>,
}

impl GamePresence {
    pub fn touch(&self, user_id: &str, now_ms: i64) {
        if user_id.is_empty() {
            return;
        }
        let mut seen = locked(&self.last_seen);
        let last = seen.entry(user_id.to_owned()).or_insert(now_ms);
        *last = (*last).max(now_ms);
        if seen.len() > MAX_TRACKED {
            seen.retain(|_, last| *last >= now_ms - SESSION_GAP_MS);
        }
    }

    pub fn is_recently_active(&self, user_id: &str, within_ms: i64, now_ms: i64) -> bool {
        locked(&self.last_seen)
            .get(user_id)
            .is_some_and(|last| now_ms - last <= within_ms)
    }

    /// `RecentlyActiveAccountIds`: the operator dashboard's online count.
    pub fn recently_active(&self, within_ms: i64, now_ms: i64) -> Vec<String> {
        locked(&self.last_seen)
            .iter()
            .filter(|(_, last)| now_ms - **last <= within_ms)
            .map(|(user, _)| user.clone())
            .collect()
    }

    /// The flush's forgetting: touches older than the session gap can no longer extend a session.
    pub fn forget_stale(&self, now_ms: i64) {
        locked(&self.last_seen).retain(|_, last| *last >= now_ms - SESSION_GAP_MS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_game_is_recent_for_the_window_after_its_last_touch() {
        let presence = GamePresence::default();
        assert!(!presence.is_recently_active("a", 75_000, 1_000));
        presence.touch("a", 1_000);
        presence.touch("a", 500); // an older touch never moves it back
        assert!(presence.is_recently_active("a", 75_000, 76_000));
        assert!(!presence.is_recently_active("a", 75_000, 76_001));
        assert_eq!(presence.recently_active(10, 1_005), vec!["a".to_owned()]);
        presence.touch("", 1);
        presence.forget_stale(1_000 + SESSION_GAP_MS + 1);
        assert!(
            presence
                .recently_active(i64::MAX / 2, 1_000 + SESSION_GAP_MS + 1)
                .is_empty()
        );
    }
}
