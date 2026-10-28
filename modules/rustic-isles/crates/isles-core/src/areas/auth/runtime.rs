//! What the auth area keeps in memory and what it asks of its neighbours, one per module (`Shared::auth`): the
//! `IsRateLimited` windows and the `EncryptionKeyStore` Node kept as module-level state, the two outgoing clients
//! (the code mail and Discord), and the seams to the other RU5 areas.
//!
//! The seams are the Node functions auth calls that another area owns: the activity monitor's `TouchGameSession`
//! and launcher-build classification (Reforged API), `GetOperationsPolicy` (Reforged API) and the starter
//! bootstrap and storage policy (characters, inventory). Each defaults to the answer that changes nothing, so
//! the area works before the others land; a neighbour installs its own by replacing the field when it builds
//! `Shared`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use module_api::BoxFuture;
use serde_json::Value;

use super::discord::{DiscordApi, HttpsDiscord};
use super::mail::{MailTransport, SmtpTransport};
use super::policy::{LauncherBuildReport, LauncherBuildVerdict};
use super::util::now_ms;
use crate::repo::AccountId;

/// `security/rateLimit.ts`'s `IsRateLimited` (sliding window per key) and `CreateRequestRateLimiter`
/// (`express-rate-limit` 8.7.0 `MemoryStore`, fixed 60-second window per key), in memory and reset on restart.
// Deliberate improvement over Node: sweep expired keys and cap tracked keys so an address flood cannot grow memory without bound.
pub const MAX_RATE_LIMIT_KEYS: usize = 16_384;
pub const REQUEST_BUDGET_WINDOW_MS: i64 = 60_000;
pub const DEFAULT_CLIENT_LIMIT_PER_MINUTE: u32 = 600;
pub const DEFAULT_GAMESERVER_LIMIT_PER_MINUTE: u32 = 6000;
pub const DEFAULT_HOST_LIMIT_PER_MINUTE: u32 = 6000;
const SWEEP_INTERVAL_MS: i64 = 60_000;

#[derive(Debug, Default)]
struct SlidingWindow {
    attempts: Vec<i64>,
    expires_at: i64,
}

#[derive(Debug, Default)]
struct SlidingState {
    entries: HashMap<String, SlidingWindow>,
    next_sweep_at: i64,
    last_now: i64,
}

#[derive(Debug, Clone, Copy)]
struct BudgetWindow {
    total_hits: u32,
    reset_time: i64,
}

#[derive(Debug, Default)]
struct BudgetState {
    entries: HashMap<String, BudgetWindow>,
    next_sweep_at: i64,
    last_now: i64,
}

/// One hit against `CreateRequestRateLimiter` (`security/rateLimit.ts`) and its `draft-8` response headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestBudgetVerdict {
    pub limited: bool,
    pub rate_limit: String,
    pub rate_limit_policy: String,
    pub retry_after: Option<String>,
}

/// `RequestRateLimitOptions` (`security/rateLimit.ts`), read from [`crate::settings::Settings`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestBudgetConfig {
    pub enabled: bool,
    pub client_limit: u32,
    pub gameserver_limit: u32,
    pub host_limit: u32,
}

impl RequestBudgetConfig {
    pub fn from_settings(settings: &crate::settings::Settings) -> Self {
        Self {
            enabled: settings.rate_limit_enabled(),
            client_limit: settings.rate_limit_per_minute(),
            gameserver_limit: settings.gameserver_rate_limit_per_minute(),
            host_limit: settings.host_rate_limit_per_minute(),
        }
    }
}

fn draft8_partition_key(key: &str) -> String {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let hex = super::crypto::hex(&Sha256::digest(key.as_bytes()));
    base64::engine::general_purpose::STANDARD.encode(&hex.as_bytes()[..12])
}

#[derive(Debug, Default)]
pub struct RateLimiter {
    windows: Mutex<SlidingState>,
    budgets: Mutex<BudgetState>,
}

impl RateLimiter {
    /// True when the key already made `max_attempts` in the window; an attempt that is not refused counts.
    pub fn is_limited(&self, key: &str, max_attempts: usize, window_ms: i64) -> bool {
        self.is_limited_at(key, max_attempts, window_ms, now_ms())
    }

    pub fn is_limited_at(&self, key: &str, max_attempts: usize, window_ms: i64, now: i64) -> bool {
        let mut state = self.windows.lock().unwrap_or_else(PoisonError::into_inner);
        if now >= state.next_sweep_at || now < state.last_now || state.entries.len() >= MAX_RATE_LIMIT_KEYS {
            state.entries.retain(|_, window| window.expires_at > now);
            state.next_sweep_at = now.saturating_add(SWEEP_INTERVAL_MS);
        }
        state.last_now = now;
        if !state.entries.contains_key(key)
            && state.entries.len() >= MAX_RATE_LIMIT_KEYS
            && let Some(oldest_key) = state
                .entries
                .iter()
                .min_by_key(|(_, window)| window.expires_at)
                .map(|(k, _)| k.clone())
        {
            state.entries.remove(&oldest_key);
        }
        let window = state.entries.entry(key.to_owned()).or_default();
        window.attempts.retain(|t| now - t < window_ms);
        if window.attempts.len() >= max_attempts {
            if window.attempts.is_empty() {
                state.entries.remove(key);
            }
            return true;
        }
        window.attempts.push(now);
        window.expires_at = window.expires_at.max(now.saturating_add(window_ms));
        false
    }

    /// Counts one request in `key`'s fixed 60-second window (`express-rate-limit` 8.7.0 `MemoryStore`).
    pub fn check_request_budget(&self, key: &str, limit: u32) -> RequestBudgetVerdict {
        self.check_request_budget_at(key, limit, now_ms())
    }

    pub fn check_request_budget_at(&self, key: &str, limit: u32, now: i64) -> RequestBudgetVerdict {
        let mut state = self.budgets.lock().unwrap_or_else(PoisonError::into_inner);
        if now >= state.next_sweep_at || now < state.last_now || state.entries.len() >= MAX_RATE_LIMIT_KEYS {
            state.entries.retain(|_, window| window.reset_time > now);
            state.next_sweep_at = now.saturating_add(SWEEP_INTERVAL_MS);
        }
        state.last_now = now;
        if !state.entries.contains_key(key)
            && state.entries.len() >= MAX_RATE_LIMIT_KEYS
            && let Some(oldest_key) = state
                .entries
                .iter()
                .min_by_key(|(_, window)| window.reset_time)
                .map(|(k, _)| k.clone())
        {
            state.entries.remove(&oldest_key);
        }
        let window = match state.entries.get_mut(key) {
            Some(existing) if existing.reset_time > now => {
                existing.total_hits = existing.total_hits.saturating_add(1);
                *existing
            }
            _ => {
                let fresh = BudgetWindow {
                    total_hits: 1,
                    reset_time: now.saturating_add(REQUEST_BUDGET_WINDOW_MS),
                };
                state.entries.insert(key.to_owned(), fresh);
                fresh
            }
        };
        let remaining = limit.saturating_sub(window.total_hits);
        let delta_ms = window.reset_time.saturating_sub(now);
        let reset_seconds = if delta_ms <= 0 {
            0
        } else {
            ((delta_ms + 999) / 1000) as u64
        };
        let name = format!("{limit}-in-1min");
        let partition_key = draft8_partition_key(key);
        let rate_limit = format!("\"{name}\"; r={remaining}; t={reset_seconds}");
        let rate_limit_policy = format!("\"{name}\"; q={limit}; w=60; pk=:{partition_key}:");
        let limited = window.total_hits > limit;
        let retry_after = limited.then(|| reset_seconds.to_string());
        RequestBudgetVerdict {
            limited,
            rate_limit,
            rate_limit_policy,
            retry_after,
        }
    }

    #[cfg(test)]
    pub fn tracked_sliding_keys(&self) -> usize {
        self.windows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entries
            .len()
    }

    #[cfg(test)]
    pub fn tracked_budget_keys(&self) -> usize {
        self.budgets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entries
            .len()
    }
}

// ------------------------------------------------------------------ encryption keys

/// `IssuedEncryptionKey`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedEncryptionKey {
    pub key: String,
    pub nonce: String,
    pub token: String,
    pub at: i64,
    /// The account `/key/generate` was called by, when known
    pub account: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncryptionKeyMatch {
    CandidateId,
    Token,
    MostRecent,
    None,
}

impl EncryptionKeyMatch {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CandidateId => "candidateId",
            Self::Token => "token",
            Self::MostRecent => "most-recent",
            Self::None => "none",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptionKeyLookup {
    pub issued: Option<IssuedEncryptionKey>,
    pub matched_by: EncryptionKeyMatch,
    /// A token was supplied but no key was issued under it
    pub unknown_token: bool,
    /// A key matched by candidate or token belongs to an account outside the caller's roster
    pub roster_rejected: bool,
}

pub const ENCRYPTION_KEY_TTL_MS: i64 = 30 * 60 * 1000;

/// The same token may arrive with or without its "BEARER " prefix and with stray whitespace.
pub fn normalize_encryption_key_token(raw: &str) -> String {
    let trimmed = service_kit::js::trim(raw);
    let lower = trimmed.to_ascii_lowercase();
    let stripped = match lower.strip_prefix("bearer") {
        Some(rest) if rest.starts_with(service_kit::js::is_space) => &trimmed[trimmed.len() - rest.len()..],
        _ => trimmed,
    };
    service_kit::js::trim(stripped).to_owned()
}

/// Identifies a token in the log without printing it.
pub fn encryption_key_token_fingerprint(raw: &str) -> String {
    use sha2::{Digest, Sha256};
    let normalized = normalize_encryption_key_token(raw);
    if normalized.is_empty() {
        return "none".to_owned();
    }
    super::crypto::hex(&Sha256::digest(normalized.as_bytes()))[..12].to_owned()
}

/// The UDP encryption keys `/key/generate` hands out and the game server fetches back on `/key/consume`
/// (`encryptionKeyStore.ts`). The game server never names a candidate, so each key is also kept under the caller's own
/// bearer token, which keeps two players who generate keys moments apart from receiving each other's.
#[derive(Debug)]
pub struct EncryptionKeyStore {
    ttl_ms: i64,
    by_candidate: HashMap<String, IssuedEncryptionKey>,
    by_token: HashMap<String, IssuedEncryptionKey>,
    most_recent: Option<IssuedEncryptionKey>,
}

impl Default for EncryptionKeyStore {
    fn default() -> Self {
        Self::new(ENCRYPTION_KEY_TTL_MS)
    }
}

impl EncryptionKeyStore {
    pub fn new(ttl_ms: i64) -> Self {
        Self {
            ttl_ms,
            by_candidate: HashMap::new(),
            by_token: HashMap::new(),
            most_recent: None,
        }
    }

    pub fn issue(&mut self, candidate_id: &str, issued: IssuedEncryptionKey) {
        if !candidate_id.is_empty() {
            self.by_candidate.insert(candidate_id.to_owned(), issued.clone());
        }
        let token_key = normalize_encryption_key_token(&issued.token);
        if !token_key.is_empty() {
            self.by_token.insert(token_key, issued.clone());
        }
        self.most_recent = Some(issued);
    }

    pub fn lookup(
        &mut self,
        candidate_id: &str,
        token: &str,
        now: i64,
        roster: Option<&std::collections::BTreeSet<String>>,
    ) -> EncryptionKeyLookup {
        let ttl = self.ttl_ms;
        self.by_candidate.retain(|_, issued| now - issued.at <= ttl);
        self.by_token.retain(|_, issued| now - issued.at <= ttl);
        let token_key = normalize_encryption_key_token(token);
        let none = |unknown_token: bool, roster_rejected: bool| EncryptionKeyLookup {
            issued: None,
            matched_by: EncryptionKeyMatch::None,
            unknown_token,
            roster_rejected,
        };
        if let Some(roster) = roster {
            let mut rejected = false;
            let candidates = [
                (
                    EncryptionKeyMatch::CandidateId,
                    (!candidate_id.is_empty())
                        .then(|| self.by_candidate.get(candidate_id))
                        .flatten(),
                ),
                (
                    EncryptionKeyMatch::Token,
                    (!token_key.is_empty()).then(|| self.by_token.get(&token_key)).flatten(),
                ),
            ];
            for (matched_by, issued) in candidates {
                let Some(issued) = issued else { continue };
                if issued.account.as_ref().is_some_and(|account| roster.contains(account)) {
                    return EncryptionKeyLookup {
                        issued: Some(issued.clone()),
                        matched_by,
                        unknown_token: false,
                        roster_rejected: false,
                    };
                }
                rejected = true;
            }
            return none(
                !token_key.is_empty() && !self.by_token.contains_key(&token_key),
                rejected,
            );
        }
        if !candidate_id.is_empty()
            && let Some(issued) = self.by_candidate.get(candidate_id)
        {
            return EncryptionKeyLookup {
                issued: Some(issued.clone()),
                matched_by: EncryptionKeyMatch::CandidateId,
                unknown_token: false,
                roster_rejected: false,
            };
        }
        if !token_key.is_empty()
            && let Some(issued) = self.by_token.get(&token_key)
        {
            return EncryptionKeyLookup {
                issued: Some(issued.clone()),
                matched_by: EncryptionKeyMatch::Token,
                unknown_token: false,
                roster_rejected: false,
            };
        }
        // Kept for request shapes that name neither a candidate nor a known token
        if let Some(recent) = self.most_recent.as_ref().filter(|recent| now - recent.at <= ttl) {
            return EncryptionKeyLookup {
                issued: Some(recent.clone()),
                matched_by: EncryptionKeyMatch::MostRecent,
                unknown_token: !token_key.is_empty(),
                roster_rejected: false,
            };
        }
        none(!token_key.is_empty(), false)
    }
}

// ------------------------------------------------------------------ the seams

/// The activity monitor (Reforged API): game sessions and launcher builds. Nothing here refuses a request.
pub trait ActivitySeam: Send + Sync {
    /// `TouchGameSession`
    fn touch_game_session(&self, _shared: &crate::http::Shared, _account: &AccountId) {}

    /// `ClassifyReportedLauncherBuild`: against the pinned launcher hash and the served version; unknown without them
    fn classify_launcher_build(
        &self,
        _shared: &crate::http::Shared,
        _channel: &str,
        _report: Option<&LauncherBuildReport>,
    ) -> LauncherBuildVerdict {
        LauncherBuildVerdict::Unknown
    }

    /// `RecordLauncherBuild`: raises the UNOFFICIAL_LAUNCHER flag; never awaited, never throws
    fn record_launcher_build(
        &self,
        _shared: &crate::http::Shared,
        _account: &AccountId,
        _channel: &str,
        _report: Option<&LauncherBuildReport>,
    ) {
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoActivity;

impl ActivitySeam for NoActivity {}

#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultActivity;

impl ActivitySeam for DefaultActivity {
    fn touch_game_session(&self, shared: &crate::http::Shared, account: &AccountId) {
        crate::areas::reforged_api::activity::touch_game_session(shared, &account.to_string());
    }

    fn classify_launcher_build(
        &self,
        shared: &crate::http::Shared,
        channel: &str,
        report: Option<&LauncherBuildReport>,
    ) -> LauncherBuildVerdict {
        crate::areas::reforged_api::activity::classify_reported_launcher_build(shared, channel, report)
    }

    fn record_launcher_build(
        &self,
        shared: &crate::http::Shared,
        account: &AccountId,
        channel: &str,
        report: Option<&LauncherBuildReport>,
    ) {
        crate::areas::reforged_api::activity::record_launcher_build(shared, &account.to_string(), channel, report);
    }
}

/// `OperationsPolicy`, the part the launcher policy reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationsPolicy {
    /// `ENFORCE` or `OBSERVE`
    pub guard_enforcement: String,
    /// `PRODUCTION` or `DEVELOPMENT`
    pub diagnostics_profile: String,
    pub version: u64,
}

impl Default for OperationsPolicy {
    /// `DEFAULT_OPERATIONS_POLICY`: a missing singleton is a deployment fault, and these cover the interval
    fn default() -> Self {
        Self {
            guard_enforcement: "ENFORCE".into(),
            diagnostics_profile: "PRODUCTION".into(),
            version: 0,
        }
    }
}

/// `GetOperationsPolicy` (Reforged API).
pub trait OperationsSeam: Send + Sync {
    fn policy<'a>(&'a self, _shared: &'a crate::http::Shared) -> BoxFuture<'a, OperationsPolicy> {
        Box::pin(async { OperationsPolicy::default() })
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultOperations;

impl OperationsSeam for DefaultOperations {
    fn policy<'a>(&'a self, shared: &'a crate::http::Shared) -> BoxFuture<'a, OperationsPolicy> {
        Box::pin(async move {
            match crate::areas::reforged_api::operations::current_policy(shared).await {
                Some(record) => OperationsPolicy {
                    guard_enforcement: record.guard_enforcement.to_owned(),
                    diagnostics_profile: record.diagnostics_profile.to_owned(),
                    version: record.version,
                },
                None => OperationsPolicy::default(),
            }
        })
    }
}

/// What the session supplies need of the inventory area: `GetInventoryStoragePolicy(catalogId)?.storage === "instanced"`.
pub trait GameDataSeam: Send + Sync {
    fn is_instanced(&self, _shared: &crate::http::Shared, _catalog_id: &str) -> bool {
        false
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoGameData;

impl GameDataSeam for NoGameData {}

#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultGameData;

impl GameDataSeam for DefaultGameData {
    fn is_instanced(&self, shared: &crate::http::Shared, catalog_id: &str) -> bool {
        let Some(path) = shared.game_data.backend_file("inventory_storage_policy.json") else {
            return false;
        };
        let Ok(policy) = crate::areas::inventory::storage_policy::StoragePolicy::load(&path) else {
            return false;
        };
        policy
            .entry(Some(&Value::String(catalog_id.to_owned())))
            .is_some_and(|e| e.storage == crate::areas::inventory::storage_policy::Storage::Instanced)
    }
}

/// Everything the auth area holds for one module.
pub struct AuthRuntime {
    pub limiter: RateLimiter,
    pub encryption_keys: Mutex<EncryptionKeyStore>,
    pub mail: Arc<dyn MailTransport>,
    pub discord: Arc<dyn DiscordApi>,
    pub activity: Arc<dyn ActivitySeam>,
    pub operations: Arc<dyn OperationsSeam>,
    pub game_data: Arc<dyn GameDataSeam>,
    /// `EnsureStarterBootstrapRecords` for the session supplies (characters' starter manifest)
    pub starter: Arc<dyn crate::areas::progression::StarterBootstrap>,
    pub erasure: Arc<dyn super::erasure::ErasureSeam>,
    /// One log line per streak of failed notices reads (`PublicReadFailureLogged`)
    pub notices_failure_logged: std::sync::atomic::AtomicBool,
}

impl Default for AuthRuntime {
    fn default() -> Self {
        Self {
            limiter: RateLimiter::default(),
            encryption_keys: Mutex::default(),
            mail: Arc::new(SmtpTransport),
            discord: Arc::new(HttpsDiscord),
            activity: Arc::new(DefaultActivity),
            operations: Arc::new(DefaultOperations),
            game_data: Arc::new(DefaultGameData),
            starter: Arc::new(crate::areas::progression::CharactersStarterBootstrap),
            erasure: Arc::new(super::erasure::AllAreasErasure),
            notices_failure_logged: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl std::fmt::Debug for AuthRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthRuntime").finish_non_exhaustive()
    }
}

/// `typeof value === "string"` and non-empty, as the bearer checks ask.
pub fn non_empty_str(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str).filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    const LEADER: &str = "BEARER eyJleader.payload.sig";
    const MEMBER: &str = "BEARER eyJmember.payload.sig";

    fn issued(key: &str, token: &str, at: i64, account: Option<&str>) -> IssuedEncryptionKey {
        IssuedEncryptionKey {
            key: key.into(),
            nonce: "n".into(),
            token: token.into(),
            at,
            account: account.map(str::to_owned),
        }
    }

    #[test]
    fn a_window_counts_only_attempts_it_allowed() {
        let limiter = RateLimiter::default();
        assert!(!limiter.is_limited_at("k", 2, 1000, 0));
        assert!(!limiter.is_limited_at("k", 2, 1000, 10));
        assert!(limiter.is_limited_at("k", 2, 1000, 20));
        assert!(limiter.is_limited_at("k", 2, 1000, 30));
        // the first two age out
        assert!(!limiter.is_limited_at("k", 2, 1000, 1011));
        assert!(!limiter.is_limited_at("other", 2, 1000, 20));
    }

    #[test]
    fn rate_limiter_sweeps_expired_keys_and_bounds_memory() {
        let limiter = RateLimiter::default();
        for i in 0..(MAX_RATE_LIMIT_KEYS + 128) {
            let key = format!("addr-{i}");
            assert!(!limiter.is_limited_at(&key, 5, 1000, 100));
            let verdict = limiter.check_request_budget_at(&key, 5, 100);
            assert!(!verdict.limited);
        }
        assert_eq!(limiter.tracked_sliding_keys(), MAX_RATE_LIMIT_KEYS);
        assert_eq!(limiter.tracked_budget_keys(), MAX_RATE_LIMIT_KEYS);

        // Advancing past the window sweeps all expired keys
        assert!(!limiter.is_limited_at("fresh", 5, 1000, 70_000));
        let verdict = limiter.check_request_budget_at("fresh", 5, 70_000);
        assert!(!verdict.limited);
        assert_eq!(limiter.tracked_sliding_keys(), 1);
        assert_eq!(limiter.tracked_budget_keys(), 1);
    }

    #[test]
    fn each_joining_player_gets_the_key_issued_to_their_own_token() {
        let mut store = EncryptionKeyStore::default();
        store.issue("leader-candidate", issued("leader-key", LEADER, 1000, None));
        store.issue("member-candidate", issued("member-key", MEMBER, 1170, None));
        let leader = store.lookup("", LEADER, 9000, None);
        assert_eq!(leader.matched_by, EncryptionKeyMatch::Token);
        assert_eq!(leader.issued.unwrap().key, "leader-key");
        let member = store.lookup("", MEMBER, 21000, None);
        assert_eq!(member.matched_by, EncryptionKeyMatch::Token);
        assert_eq!(member.issued.unwrap().key, "member-key");
    }

    #[test]
    fn token_matching_ignores_the_bearer_prefix_its_case_and_whitespace() {
        let mut store = EncryptionKeyStore::default();
        store.issue("", issued("k", LEADER, 0, None));
        assert_eq!(
            store.lookup("", "eyJleader.payload.sig", 1, None).issued.unwrap().key,
            "k"
        );
        assert_eq!(
            store
                .lookup("", "  bearer eyJleader.payload.sig ", 1, None)
                .issued
                .unwrap()
                .key,
            "k"
        );
        assert_eq!(normalize_encryption_key_token("Bearer  abc "), "abc");
        // A token that merely starts with the letters is not a prefix
        assert_eq!(normalize_encryption_key_token("bearerxyz"), "bearerxyz");
    }

    #[test]
    fn a_candidate_id_still_wins_and_a_reissue_replaces_the_key() {
        let mut store = EncryptionKeyStore::default();
        store.issue("c1", issued("first", LEADER, 0, None));
        store.issue("c2", issued("second", LEADER, 10, None));
        assert_eq!(store.lookup("c1", LEADER, 20, None).issued.unwrap().key, "first");
        assert_eq!(store.lookup("", LEADER, 20, None).issued.unwrap().key, "second");
    }

    #[test]
    fn an_unknown_token_falls_back_to_the_most_recent_key_and_expired_keys_are_dropped() {
        let mut store = EncryptionKeyStore::new(1000);
        store.issue("", issued("recent", MEMBER, 0, None));
        let unknown = store.lookup("", "BEARER somebody-else", 500, None);
        assert_eq!(unknown.matched_by, EncryptionKeyMatch::MostRecent);
        assert!(unknown.unknown_token);
        assert_eq!(unknown.issued.unwrap().key, "recent");
        let expired = store.lookup("", MEMBER, 5000, None);
        assert_eq!(expired.issued, None);
        assert_eq!(expired.matched_by, EncryptionKeyMatch::None);
    }

    #[test]
    fn the_log_fingerprint_never_contains_the_token() {
        let fingerprint = encryption_key_token_fingerprint(LEADER);
        assert_eq!(fingerprint.len(), 12);
        assert!(!LEADER.contains(&fingerprint));
        assert_eq!(encryption_key_token_fingerprint(""), "none");
    }

    #[test]
    fn a_player_host_lookup_serves_only_its_rosters_keys() {
        let mut store = EncryptionKeyStore::default();
        store.issue("leader-candidate", issued("leader-key", LEADER, 1000, Some("leader")));
        store.issue("", issued("stranger-key", "BEARER stranger", 1100, Some("stranger")));
        let roster: BTreeSet<String> = ["leader", "member"].into_iter().map(str::to_owned).collect();
        let leader = store.lookup("", LEADER, 2000, Some(&roster));
        assert_eq!(leader.matched_by, EncryptionKeyMatch::Token);
        assert_eq!(leader.issued.unwrap().key, "leader-key");
        assert_eq!(
            store
                .lookup("leader-candidate", "", 2000, Some(&roster))
                .issued
                .unwrap()
                .key,
            "leader-key"
        );
        let stranger = store.lookup("", "BEARER stranger", 2000, Some(&roster));
        assert_eq!(stranger.issued, None);
        assert!(stranger.roster_rejected);
        let unknown = store.lookup("", "BEARER nobody", 2000, Some(&roster));
        assert_eq!(unknown.issued, None);
        assert!(unknown.unknown_token);
        assert_eq!(store.lookup("", "", 2000, Some(&roster)).issued, None);
        assert_eq!(
            store.lookup("", "", 2000, None).matched_by,
            EncryptionKeyMatch::MostRecent
        );
        store.issue("", issued("anonymous-key", "BEARER anonymous", 1200, None));
        assert_eq!(store.lookup("", "BEARER anonymous", 2000, Some(&roster)).issued, None);
        assert_eq!(store.lookup("", LEADER, 2000, Some(&BTreeSet::new())).issued, None);
    }
}
