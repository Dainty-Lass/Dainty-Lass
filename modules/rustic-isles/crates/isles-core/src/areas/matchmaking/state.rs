//! What matchmaking keeps in memory (DD15): the queues, each player's candidate, the parties' shared instances and the
//! hubs' stable game session ids, plus the collaborators the area reaches through seams.
//!
//! Every map is keyed as Node keys it: by the client's `userId` (the key a hunt's roster and the candidate poll use),
//! so the realms side, which names accounts the same way (`rifts_seam::AccountId` carries the `userId`), shares them.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};

use serde_json::{Map, Value};

use super::deploy::{DedicatedServers, HttpDeployServer};
use super::presence::GamePresence;
use super::util::Clock;
use crate::settings::Settings;

/// `MatchmakingResult.Source`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A realm's Ramsgate or Training hub on a member's PC (the P2P module's candidate)
    RealmHub,
}

/// `MatchmakingResult`: one player's candidate, a travel attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchmakingResult {
    pub ready: bool,
    pub canceled: bool,
    pub status_reason: Option<String>,
    pub hunt_id: String,
    pub candidate_id: String,
    pub game_session_id: String,
    pub host: String,
    pub port: i64,
    /// Epoch ms. `ready` stays internal server readiness; this controls the client-visible Go Now release after the
    /// instance is genuinely online.
    pub ready_at: i64,
    pub source: Option<Source>,
    /// The hub session the candidate's status is read from, for a realm hub
    pub source_session_id: Option<String>,
}

impl MatchmakingResult {
    /// A search that has begun: matching, nothing to travel to yet.
    pub fn matching(hunt_id: &str, candidate_id: String) -> Self {
        Self {
            ready: false,
            canceled: false,
            status_reason: None,
            hunt_id: hunt_id.to_owned(),
            game_session_id: candidate_id.clone(),
            candidate_id,
            host: String::new(),
            port: 0,
            ready_at: 0,
            source: None,
            source_session_id: None,
        }
    }

    /// `IsClientReady`.
    pub fn is_client_ready(&self, now_ms: i64) -> bool {
        self.ready && (self.ready_at == 0 || now_ms >= self.ready_at)
    }

    /// `ClientVisibleResult`.
    pub fn client_visible(&self, now_ms: i64) -> Self {
        Self {
            ready: self.is_client_ready(now_ms),
            ..self.clone()
        }
    }

    /// `GetCandidateStatusPeriodMillis`: once per second while matching and during the release window, then every 10 s.
    pub fn status_period_millis(&self, now_ms: i64) -> i64 {
        if !self.ready || self.ready_at > now_ms {
            1000
        } else {
            10_000
        }
    }

    /// Node's object, as the realms crate receives it (`Ready`, `HuntId`...; absent fields left out).
    pub fn to_json(&self) -> Value {
        let mut map = Map::new();
        map.insert("Ready".into(), Value::Bool(self.ready));
        if self.canceled {
            map.insert("Canceled".into(), Value::Bool(true));
        }
        if let Some(reason) = &self.status_reason {
            map.insert("StatusReason".into(), Value::String(reason.clone()));
        }
        map.insert("HuntId".into(), Value::String(self.hunt_id.clone()));
        map.insert("CandidateId".into(), Value::String(self.candidate_id.clone()));
        map.insert("GameSessionId".into(), Value::String(self.game_session_id.clone()));
        map.insert("Host".into(), Value::String(self.host.clone()));
        map.insert("Port".into(), Value::from(self.port));
        map.insert("ReadyAt".into(), Value::from(self.ready_at));
        if self.source == Some(Source::RealmHub) {
            map.insert("Source".into(), Value::String("REALM_HUB".into()));
        }
        if let Some(session) = &self.source_session_id {
            map.insert("SourceSessionId".into(), Value::String(session.clone()));
        }
        Value::Object(map)
    }

    pub fn from_json(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        let text = |key: &str| object.get(key).and_then(Value::as_str).map(str::to_owned);
        Some(Self {
            ready: object.get("Ready").and_then(Value::as_bool).unwrap_or(false),
            canceled: object.get("Canceled").and_then(Value::as_bool).unwrap_or(false),
            status_reason: text("StatusReason"),
            hunt_id: text("HuntId")?,
            candidate_id: text("CandidateId")?,
            game_session_id: text("GameSessionId")?,
            host: text("Host").unwrap_or_default(),
            port: object.get("Port").and_then(Value::as_i64).unwrap_or(0),
            ready_at: object.get("ReadyAt").and_then(Value::as_i64).unwrap_or(0),
            source: (text("Source").as_deref() == Some("REALM_HUB")).then_some(Source::RealmHub),
            source_session_id: text("SourceSessionId"),
        })
    }
}

/// `MatchmakingQueueData`: the players waiting for one CR19 hunt id.
#[derive(Debug, Clone)]
pub(super) struct Queue {
    pub players: Vec<String>,
    pub last_added_ms: i64,
    pub resolved: bool,
}

#[derive(Debug, Default)]
pub(super) struct Inner {
    /// By hunt id
    pub queues: HashMap<String, Queue>,
    /// By player
    pub results: HashMap<String, MatchmakingResult>,
    /// By party: the party's shared hunt instance, and when it was created (the fan-out window)
    pub party_instances: HashMap<String, MatchmakingResult>,
    pub party_instance_created: HashMap<String, i64>,
    /// The launch id of each dedicated instance, by `host:port`
    pub hunt_ref_of_instance: HashMap<String, String>,
    /// A shared hub's stable game session id (the `City-<id>` chat room), by `host:port`
    pub hub_game_session_ids: HashMap<String, String>,
}

/// The activity monitor's hunt rows (`RecordDedicatedHuntStarts`, `RecordDedicatedHuntEnds`), the reforged-api
/// area's; the launch never waits for them.
pub trait HuntActivity: Send + Sync {
    fn dedicated_hunt_starts(&self, users: &[String], hunt_id: &str, hunt_ref: &str);
    fn dedicated_hunt_ends(&self, users: &[String], end_reason: &str);
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoHuntActivity;

impl HuntActivity for NoHuntActivity {
    fn dedicated_hunt_starts(&self, _: &[String], _: &str, _: &str) {}
    fn dedicated_hunt_ends(&self, _: &[String], _: &str) {}
}

/// `sessionRegistry.isOnline`: whether a game holds a chat connection, the chat plugin's (DN10a).
pub trait ChatPresence: Send + Sync {
    fn is_online(&self, user_id: &str) -> bool;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NobodyOnline;

impl ChatPresence for NobodyOnline {
    fn is_online(&self, _: &str) -> bool {
        false
    }
}

/// A replaceable collaborator.
pub(super) struct Slot<T: ?Sized>(RwLock<Arc<T>>);

impl<T: ?Sized> Slot<T> {
    fn new(value: Arc<T>) -> Self {
        Self(RwLock::new(value))
    }

    pub fn get(&self) -> Arc<T> {
        self.0.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn set(&self, value: Arc<T>) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = value;
    }
}

/// Everything the area keeps per module (a node holding both versions keeps one each).
pub struct MatchmakingState {
    inner: Mutex<Inner>,
    pub clock: Clock,
    pub presence: GamePresence,
    pub(super) deploy: Slot<dyn DedicatedServers>,
    pub(super) activity: Slot<dyn HuntActivity>,
    pub(super) chat: Slot<dyn ChatPresence>,
}

impl Default for MatchmakingState {
    fn default() -> Self {
        Self::with_deploy(Arc::new(super::deploy::Unconfigured))
    }
}

impl MatchmakingState {
    pub fn with_deploy(deploy: Arc<dyn DedicatedServers>) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            clock: Clock::default(),
            presence: GamePresence::default(),
            deploy: Slot::new(deploy),
            activity: Slot::new(Arc::new(NoHuntActivity)),
            chat: Slot::new(Arc::new(NobodyOnline)),
        }
    }

    /// A module's state, with the Director address its settings name (`DEPLOYSERVER_URL`).
    pub fn from_settings(settings: &Settings) -> Self {
        Self::with_deploy(HttpDeployServer::from_settings(settings))
    }

    pub fn set_dedicated_servers(&self, deploy: Arc<dyn DedicatedServers>) {
        self.deploy.set(deploy);
    }

    pub fn dedicated_servers(&self) -> Arc<dyn DedicatedServers> {
        self.deploy.get()
    }

    pub fn set_hunt_activity(&self, activity: Arc<dyn HuntActivity>) {
        self.activity.set(activity);
    }

    pub fn set_chat_presence(&self, chat: Arc<dyn ChatPresence>) {
        self.chat.set(chat);
    }

    pub(super) fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `IsHubGameSessionId`: true for a hub's game session id (the `City-<id>` chat room), false for a hunt's.
    pub fn is_hub_game_session_id(&self, id: &str) -> bool {
        self.lock().hub_game_session_ids.values().any(|hub| hub == id)
    }

    /// `GetHubGameSessionId`: stable for the life of the persistent hub, and shared with the realms' recovery to
    /// Ramsgate so returning players land in the room everyone in the hub is in.
    pub fn hub_game_session_id(&self, host: &str, port: i64) -> String {
        self.lock()
            .hub_game_session_ids
            .entry(format!("{host}:{port}"))
            .or_insert_with(|| crate::game_data::season_window::next_uuid().to_string())
            .clone()
    }

    /// `MatchmakingResultMap.set`: publishes one player's candidate.
    pub fn set_candidate(&self, player: &str, result: MatchmakingResult) {
        self.lock().results.insert(player.to_owned(), result);
    }

    /// The stored candidate as it is, not as the client sees it.
    pub fn candidate(&self, player: &str) -> Option<MatchmakingResult> {
        self.lock().results.get(player).cloned()
    }

    /// `CancelHuntMatchmaking`: every distinct player gets a cancelled candidate with `reason`.
    pub fn cancel_hunt(&self, hunt_id: &str, players: &[String], reason: &str) -> bool {
        cancel_hunt(&mut self.lock(), hunt_id, players, reason)
    }
}

pub(super) fn cancel_hunt(inner: &mut Inner, hunt_id: &str, players: &[String], reason: &str) -> bool {
    let mut seen: Vec<&String> = Vec::new();
    for player in players {
        if seen.contains(&player) {
            continue;
        }
        seen.push(player);
        let candidate_id = crate::game_data::season_window::next_uuid().to_string();
        let mut result = MatchmakingResult::matching(hunt_id, candidate_id);
        result.canceled = true;
        result.status_reason = Some(reason.to_owned());
        inner.results.insert(player.clone(), result);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_candidate_round_trips_through_nodes_object() {
        let mut result = MatchmakingResult::matching("ShatteredIsles_IslandA", "c1".into());
        result.ready = true;
        result.host = "203.0.113.10".into();
        result.port = 7777;
        result.ready_at = 5;
        result.source = Some(Source::RealmHub);
        result.source_session_id = Some("s".into());
        assert_eq!(
            result.to_json().to_string(),
            r#"{"Ready":true,"HuntId":"ShatteredIsles_IslandA","CandidateId":"c1","GameSessionId":"c1","Host":"203.0.113.10","Port":7777,"ReadyAt":5,"Source":"REALM_HUB","SourceSessionId":"s"}"#
        );
        assert_eq!(MatchmakingResult::from_json(&result.to_json()), Some(result));
        assert_eq!(MatchmakingResult::from_json(&serde_json::json!({"Ready": true})), None);
    }

    #[test]
    fn the_poll_period_is_one_second_until_the_release_has_passed() {
        let mut result = MatchmakingResult::matching("h", "c".into());
        assert_eq!(result.status_period_millis(100), 1000);
        result.ready = true;
        result.ready_at = 200;
        assert_eq!(result.status_period_millis(100), 1000);
        assert_eq!(result.status_period_millis(200), 10_000);
        result.ready_at = 0;
        assert_eq!(result.status_period_millis(0), 10_000);
        assert!(result.is_client_ready(0));
        assert!(!result.client_visible(0).canceled);
    }

    #[test]
    fn a_hub_keeps_one_game_session_id() {
        let state = MatchmakingState::default();
        let first = state.hub_game_session_id("203.0.113.9", 8790);
        assert_eq!(state.hub_game_session_id("203.0.113.9", 8790), first);
        assert_ne!(state.hub_game_session_id("203.0.113.9", 8791), first);
        assert!(state.is_hub_game_session_id(&first));
        assert!(!state.is_hub_game_session_id("a-hunt"));
    }

    #[test]
    fn a_cancelled_hunt_names_each_player_once() {
        let state = MatchmakingState::default();
        let players = vec!["a".to_owned(), "b".to_owned(), "a".to_owned()];
        assert!(state.cancel_hunt("h", &players, "WHY"));
        let a = state.candidate("a").unwrap();
        assert!(a.canceled && !a.ready);
        assert_eq!(a.status_reason.as_deref(), Some("WHY"));
        assert_eq!(a.game_session_id, a.candidate_id);
        assert_ne!(a.candidate_id, state.candidate("b").unwrap().candidate_id);
    }
}
