//! Who is online: the chat sessions (`sessionRegistry`, DN10a fills it in) and the accounts whose game reported in
//! recently (`RecentlyActiveAccountIds`, fed by the activity monitor). Process-wide, as Node's registries are.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use super::util::now_ms;

/// The chat core's view of live sockets, once DN10a registers it.
pub trait ChatSessions: Send + Sync {
    /// Node's `onlineAccountIds()`: one entry per account, whatever its resources.
    fn online_user_ids(&self) -> Vec<String>;

    /// Closes every connection of `user_id` with this code and reason (`close(1008, "kicked by the host")`) and
    /// says how many there were.
    fn close_connections(&self, user_id: &str, code: u16, reason: &str) -> usize;
}

static CHAT: RwLock<Option<Arc<dyn ChatSessions>>> = RwLock::new(None);

/// Registers the chat sessions; the chat plugin calls this when it starts.
pub fn set_chat_sessions(sessions: Option<Arc<dyn ChatSessions>>) {
    *CHAT.write().unwrap_or_else(PoisonError::into_inner) = sessions;
}

pub fn chat_sessions() -> Option<Arc<dyn ChatSessions>> {
    CHAT.read().unwrap_or_else(PoisonError::into_inner).clone()
}

pub fn online_user_ids() -> Vec<String> {
    chat_sessions().map(|chat| chat.online_user_ids()).unwrap_or_default()
}

static ACTIVE: Mutex<Option<HashMap<String, i64>>> = Mutex::new(None);

/// The activity monitor saw this account act.
pub fn touch(user_id: &str) {
    touch_at(user_id, now_ms());
}

pub fn touch_at(user_id: &str, at: i64) {
    let mut guard = ACTIVE.lock().unwrap_or_else(PoisonError::into_inner);
    let map = guard.get_or_insert_with(HashMap::new);
    map.insert(user_id.to_owned(), at);
    // Bounded like a monitor's table: forget what is long past
    if map.len() > 50_000 {
        let floor = at - 3_600_000;
        map.retain(|_, seen| *seen >= floor);
    }
}

/// `RecentlyActiveAccountIds(windowMs)`.
pub fn recently_active(window_ms: i64) -> Vec<String> {
    recently_active_at(window_ms, now_ms())
}

pub fn recently_active_at(window_ms: i64, now: i64) -> Vec<String> {
    let guard = ACTIVE.lock().unwrap_or_else(PoisonError::into_inner);
    guard.as_ref().map_or_else(Vec::new, |map| {
        map.iter()
            .filter(|(_, seen)| now - **seen <= window_ms)
            .map(|(id, _)| id.clone())
            .collect()
    })
}

/// Chat sessions and recent activity, each account once.
pub fn active_players(window_ms: i64) -> Vec<String> {
    let mut all = online_user_ids();
    all.extend(recently_active(window_ms));
    let mut seen = std::collections::HashSet::new();
    all.retain(|id| seen.insert(id.clone()));
    all
}
