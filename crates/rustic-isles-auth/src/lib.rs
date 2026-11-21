// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Rustic Isles' sign-in area (RD27, written clean-room from `docs/port/spec-rustic-isles-auth.md`): the game
//! client's sign-in stubs and Epic stand-in, the hub's encryption keys, and the launcher API (accounts, sessions,
//! email codes, Discord, consent, deletion, game launches). The crate owns the routes and rules; isles owns the
//! pipeline, the tables ([`AuthStore`]) and the neighbours ([`Seams`]).

#![cfg_attr(
    test,
    allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")
)]
#![allow(
    clippy::result_large_err,
    reason = "a refusal is the answer itself (§11.2 hands isles a `Response`), so it is not boxed"
)]

use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use axum::response::Response;
use dainty_module_api::{Actor, ActorContext, GameVersion, NodeIdentity};
use serde_json::{Map, Value};

mod client;
pub mod discord;
pub mod email;
pub mod enckeys;
mod errors;
mod game;
pub mod game_tickets;
pub mod hashes;
mod jobs;
mod launcher;
pub mod launcher_build;
mod limiter;
pub mod mail;
pub mod oauth;
pub mod password;
pub mod policy;
mod routes;
pub mod runtime;
mod seams;
mod store;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
mod tokens;
pub mod totp;
mod util;

pub use client::{client_ip, client_ip_from, rate_limit_key};
pub use discord::{DiscordApi, DiscordConfig, DiscordFailure, DiscordUser, HttpsDiscord};
pub use enckeys::{EncryptionKeys, IssuedKey, KeyLookup};
pub use errors::{LauncherError, status_for};
pub use hashes::pseudonymous_account_id;
pub use jobs::{ErasureOutcome, StartError};
pub use launcher::{Cors, LauncherAuth};
pub use launcher_build::{LauncherReport, LauncherVerdict};
pub use limiter::{BudgetConfig, BudgetVerdict, RateLimiter};
pub use mail::{MailError, MailMessage, MailTransport, SmtpConfig, SmtpTransport};
pub use password::{hash_password, verify_password};
pub use policy::{assert_account_admitted, is_account_eligible};
pub use routes::{Route, RouteId, routes};
pub use seams::{
    Activity, Defaults, Erasure, GameLogin, Notices, Operations, OperationsPolicy, RealmError, Realms, RuntimeAuth,
    RuntimeCaller, RuntimeRequest, Seams, empty_notices,
};
pub use store::{
    Approval, AuthStore, Channel, Consent, CreateOutcome, Deletion, DiscordTransaction, EmailCode, ErasureCounts,
    ExchangeCode, GameAccount, Identity, LauncherAccount, NewLauncherAccount, Purpose, RefreshSession, Status,
    StoreError, StoreResult, Timeout,
};
pub use tokens::{StorePurchase, TokenError};
pub use totp::verify_totp;
pub use util::{iso, query_pairs};

/// The module's settings, read at each use (Node read `process.env`).
pub trait AuthSettings: Send + Sync {
    /// The value as set, empty included.
    fn raw(&self, name: &str) -> Option<&str>;
}

impl AuthSettings for std::collections::HashMap<String, String> {
    fn raw(&self, name: &str) -> Option<&str> {
        self.get(name).map(String::as_str)
    }
}

/// Unix milliseconds; isles passes its replay-aware clock.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;
/// A UUID v4 text; isles passes its replay-seeded source.
pub type IdSource = Arc<dyn Fn() -> String + Send + Sync>;

/// Who a bearer-less game server call is, and what the `INVALID` sentinel becomes (§4.1, §4.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoPlayerPolicy {
    /// `INVALID` stays, and a bearer-less game server has no user
    Off,
    /// `GAMESERVER_NO_PLAYER_USER_ID`, else `DEV_USER_ID` (1.14.7)
    ConfiguredAccount,
}

/// What the version and the host give the auth area.
pub struct AuthConfig {
    /// The game build's changelist the client reports.
    pub target_changelist: u32,
    /// The game version this module serves.
    pub game_version: GameVersion,
    /// The node this module runs on.
    pub node: NodeIdentity,
    /// Which account a bearer-less game server call acts as.
    pub no_player: NoPlayerPolicy,
    /// The `/gamesession/epic` payload for a session token
    pub session_payload: fn(&str) -> Value,
    /// For `RUSTIC_ISLES_CONSENT_REQUIRED`'s blank default
    pub engine_is_postgres: bool,
    /// The clock for limits, codes and token signing; token validation uses the system clock.
    pub clock: Clock,
    /// Where new UUIDs come from.
    pub ids: IdSource,
}

impl AuthConfig {
    /// Dauntless 1.14.7 (changelist 647472) on `node`, with the system clock and random ids.
    pub fn for_1_14_7(node: NodeIdentity) -> Self {
        Self {
            target_changelist: 647_472,
            game_version: GameVersion::release(1, 14, 7),
            node,
            no_player: NoPlayerPolicy::ConfiguredAccount,
            session_payload: session_payload_1_14_7,
            engine_is_postgres: false,
            clock: Arc::new(|| chrono::Utc::now().timestamp_millis()),
            ids: Arc::new(util::random_uuid),
        }
    }
}

/// 1.14.7's session payload.
pub fn session_payload_1_14_7(token: &str) -> Value {
    util::object(vec![
        ("error_code", Value::Null),
        ("sessionid", Value::String(token.to_owned())),
        ("sessiontoken", Value::String(token.to_owned())),
        ("sessionToken", Value::String(token.to_owned())),
    ])
}

/// The game caller (§4.1), in the request's extensions for other areas to read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AuthData {
    /// The player's userId; None for a runtime or a bearer-less game server with no configured account.
    pub user_id: Option<String>,
    /// True when the call carried the game server key.
    pub is_gameserver: bool,
    /// The game token's claims; empty without a token.
    pub claims: Map<String, Value>,
    /// The player host's runtime, when the realms seam authenticated one.
    pub runtime: Option<RuntimeCaller>,
}

/// Which auth stands in front of a relayed rustic-rifts route (§4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerKind {
    /// Anonymous
    None,
    /// Launcher auth; then the active-account check with (`Some(true)`) or without (`Some(false)`) consent
    Launcher {
        /// Which active-account check runs, as above; None skips it.
        active: Option<bool>,
    },
    /// Anonymous here; rifts checks the host capability
    HostCapability,
    /// The Reforged API's admin auth
    Admin,
}

/// The auth a relayed rustic-rifts route takes, by its method and path.
pub fn rifts_caller_kind(method: &str, route_path: &str) -> CallerKind {
    let post = method.eq_ignore_ascii_case("POST");
    let p = route_path;
    if p.starts_with("/admin/") {
        return CallerKind::Admin;
    }
    if p.starts_with("/launcher/v1/realms") {
        let with_consent = post && (p == "/launcher/v1/realms" || p.starts_with("/launcher/v1/realms/mine/"));
        let without = post
            && (p == "/launcher/v1/realms/join"
                || p == "/launcher/v1/realms/presence/heartbeat"
                || (p.starts_with("/launcher/v1/realms/player-invites/") && p.ends_with("/accept")));
        let active = if with_consent {
            Some(true)
        } else if without {
            Some(false)
        } else {
            None
        };
        return CallerKind::Launcher { active };
    }
    if p.starts_with("/launcher/v1/realm-hosting/hubs/start") {
        return CallerKind::Launcher { active: Some(true) };
    }
    if p.starts_with("/launcher/v1/p2p/") || p.starts_with("/launcher/v1/realm-hosting/") {
        let without = p == "/launcher/v1/p2p/host-presence"
            || (p.starts_with("/launcher/v1/p2p/host-offers/") && p.ends_with("/respond"))
            || p.ends_with("/host-bootstrap")
            || p.ends_with("/join-bootstrap")
            || p.ends_with("/peers");
        return CallerKind::Launcher {
            active: without.then_some(false),
        };
    }
    if p.starts_with("/launcher/v1/guard/") {
        return CallerKind::Launcher { active: None };
    }
    // D3: rifts' hunt routes are /rustic/v1/hunts, no alias
    if p.starts_with("/rustic/v1/hunts/") {
        if p.ends_with("/tickets/self") || p.ends_with("/status") || p.ends_with("/reward/self") {
            return CallerKind::Launcher { active: None };
        }
        return CallerKind::HostCapability;
    }
    CallerKind::None
}

/// One module's sign-in area.
pub struct Auth<S: AuthStore> {
    config: AuthConfig,
    settings: Arc<dyn AuthSettings>,
    store: Option<S>,
    seams: Seams,
    limiter: RateLimiter,
    keys: EncryptionKeys,
    token_keys: tokens::KeyCache,
    warned_official: Mutex<HashSet<String>>,
    notices_failing: AtomicBool,
    /// Staff kicks being carried out: userId to when the kick lifts by itself (ms)
    kicked: Mutex<HashMap<String, i64>>,
    /// One-time logins the launcher hands the game in place of the account key
    tickets: game_tickets::GameTickets,
    this: Weak<Self>,
}

impl<S: AuthStore> Auth<S> {
    /// `None` store: no database (the skeleton module).
    pub fn new(config: AuthConfig, settings: Arc<dyn AuthSettings>, store: Option<S>, seams: Seams) -> Arc<Self> {
        Arc::new_cyclic(|this| Self {
            config,
            settings,
            store,
            seams,
            limiter: RateLimiter::new(),
            keys: EncryptionKeys::default(),
            token_keys: tokens::KeyCache::default(),
            warned_official: Mutex::new(HashSet::new()),
            notices_failing: AtomicBool::new(false),
            kicked: Mutex::new(HashMap::new()),
            tickets: game_tickets::GameTickets::default(),
            this: this.clone(),
        })
    }

    /// The version's and host's config.
    pub fn config(&self) -> &AuthConfig {
        &self.config
    }

    /// A one-time game login for the account, redeemable for [`game_tickets::TTL_MS`].
    pub fn mint_game_ticket(&self, holder: game_tickets::TicketHolder) -> String {
        self.tickets.mint(holder, self.now())
    }

    /// Drops the account's unredeemed game tickets.
    pub fn revoke_game_tickets(&self, account_id: &str) {
        self.tickets.revoke_account(account_id);
    }

    /// The module's attempt and request limiter.
    pub fn limiter(&self) -> &RateLimiter {
        &self.limiter
    }

    /// The hub's network encryption keys.
    pub fn encryption_keys(&self) -> &EncryptionKeys {
        &self.keys
    }

    /// The store; None without a database.
    pub fn store_ref(&self) -> Option<&S> {
        self.store.as_ref()
    }

    fn now(&self) -> i64 {
        (self.config.clock)()
    }

    fn now_iso(&self) -> String {
        util::iso(self.now())
    }

    fn uuid(&self) -> String {
        (self.config.ids)()
    }

    fn raw(&self, name: &str) -> Option<&str> {
        self.settings.raw(name)
    }

    fn store(&self) -> StoreResult<&S> {
        self.store
            .as_ref()
            .ok_or_else(|| StoreError::new("no database is configured"))
    }

    fn secret(&self) -> Result<&str, hashes::SecretError> {
        hashes::read_secret(self.raw("API_KEY_HASH_SECRET"))
    }

    fn system_actor(&self) -> ActorContext {
        ActorContext::new(Actor::System)
    }

    /// DD17: a player's write is refused unless the id is a network id (and the store checks the home).
    fn player_actor(&self, account_id: &str) -> StoreResult<ActorContext> {
        let account = account_id
            .parse()
            .map_err(|_| StoreError::new("the account id is not a network id"))?;
        Ok(ActorContext::new(Actor::Player { account }))
    }

    fn error_response(&self, error: &LauncherError) -> Response {
        error.response(&self.uuid())
    }

    fn token_keys(&self) -> Result<Arc<tokens::TokenKeys>, TokenError> {
        self.token_keys.get(
            self.raw("AUTH_SIGNING_PRIVKEY_B64").unwrap_or(""),
            self.raw("AUTH_SIGNING_PUBKEY_B64").unwrap_or(""),
        )
    }

    fn sign_seconds(&self) -> i64 {
        self.now().div_euclid(1000)
    }

    // Validation runs on the system clock (spec §14 item 13)
    fn system_seconds() -> i64 {
        chrono::Utc::now().timestamp()
    }

    /// A staff kick in progress: until `until_ms` (or [`Self::lift_kick`]) the user's game tokens and their game
    /// servers' writes for them are refused, so nothing is saved between the kick and their removal. It lifts by
    /// itself: a kick doesn't outlast the removal, and the player signs back in from the menu with the session they
    /// have. Bans and timeouts last through the launcher account's eligibility, not this.
    pub fn kick(&self, user_id: &str, window_ms: i64) {
        let until = self.now().saturating_add(window_ms.max(0));
        self.kicked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(user_id.to_owned(), until);
    }

    /// Ends a kick in progress: the player was removed.
    pub fn lift_kick(&self, user_id: &str) {
        self.kicked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(user_id);
    }

    fn kick_active(&self, user_id: &str) -> bool {
        let now = self.now();
        let mut kicked = self.kicked.lock().unwrap_or_else(PoisonError::into_inner);
        kicked.retain(|_, until| *until > now);
        kicked.contains_key(user_id)
    }

    /// Why a game server may not act for the player now: `kicked` while a kick is carried out, `restricted` while a
    /// ban or timeout lasts. A store that can't answer refuses nothing (the route's own checks still run).
    pub async fn server_refusal(&self, user_id: &str) -> Option<&'static str> {
        if self.kick_active(user_id) {
            return Some("kicked");
        }
        match self.store().ok()?.launcher_account_by_user_id(user_id).await {
            Ok(Some(account)) if !policy::is_account_eligible(&account, &self.now_iso()) => Some("restricted"),
            _ => None,
        }
    }

    /// Whether a game token's user is being kicked now.
    pub fn is_kicked(&self, claims: &Map<String, Value>) -> bool {
        matches!(claims.get("userId"), Some(Value::String(user_id)) if self.kick_active(user_id))
    }

    /// A game token for `userId`; fails when the signing keys cannot load.
    pub fn sign_game_token(&self, user_id: &str) -> Result<String, TokenError> {
        tokens::sign_game(&*self.token_keys()?, user_id, self.sign_seconds())
    }

    /// A game token's claims, checked against the system clock.
    pub fn validate_game_token(&self, token: &str) -> Result<Map<String, Value>, TokenError> {
        tokens::validate_game(&*self.token_keys()?, token, Self::system_seconds())
    }

    /// A launcher access token for `userId` in refresh family `sid`.
    pub fn sign_launcher_token(&self, user_id: &str, sid: &str) -> Result<String, TokenError> {
        tokens::sign_launcher(&*self.token_keys()?, user_id, sid, self.sign_seconds())
    }

    /// A launcher token's `(userId, sid)`, checked against the system clock.
    pub fn validate_launcher_token(&self, token: &str) -> Result<(String, String), TokenError> {
        tokens::validate_launcher(&*self.token_keys()?, token, Self::system_seconds())
    }

    /// A store token carrying `purchase`.
    pub fn sign_store_token(&self, purchase: &StorePurchase) -> Result<String, TokenError> {
        tokens::sign_store(&*self.token_keys()?, purchase, self.sign_seconds())
    }

    /// The purchase a store token carries, checked against the system clock.
    pub fn validate_store_token(&self, token: &str) -> Result<StorePurchase, TokenError> {
        tokens::validate_store(&*self.token_keys()?, token, Self::system_seconds())
    }

    /// `(available, message)` of official play, warning once per distinct unrecognised value.
    fn official_play(&self) -> bool {
        let (available, unrecognised) =
            policy::official_play(self.raw("RUSTIC_ISLES_OFFICIAL_PLAY"), self.raw("DEPLOYSERVER_URL"));
        if let Some(value) = unrecognised {
            let mut warned = self.warned_official.lock().unwrap_or_else(PoisonError::into_inner);
            if warned.insert(value.clone()) {
                tracing::warn!(
                    "RUSTIC_ISLES_OFFICIAL_PLAY must be on or off (got \"{value}\"); official play is treated as off"
                );
            }
        }
        available
    }

    fn verification_required(&self) -> bool {
        policy::email_verification(self.raw("RUSTIC_ISLES_EMAIL_VERIFICATION")).0
    }

    fn smtp(&self) -> Result<SmtpConfig, Vec<&'static str>> {
        SmtpConfig::from_settings(self.settings.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rifts_callers() {
        let launcher = |active| CallerKind::Launcher { active };
        for (method, path, kind) in [
            ("GET", "/admin/realms", CallerKind::Admin),
            ("POST", "/launcher/v1/realms", launcher(Some(true))),
            ("POST", "/launcher/v1/realms/mine/invites", launcher(Some(true))),
            ("GET", "/launcher/v1/realms", launcher(None)),
            ("POST", "/launcher/v1/realms/join", launcher(Some(false))),
            ("POST", "/launcher/v1/realms/presence/heartbeat", launcher(Some(false))),
            (
                "POST",
                "/launcher/v1/realms/player-invites/:id/accept",
                launcher(Some(false)),
            ),
            ("POST", "/launcher/v1/realm-hosting/hubs/start", launcher(Some(true))),
            ("POST", "/launcher/v1/p2p/host-presence", launcher(Some(false))),
            (
                "POST",
                "/launcher/v1/p2p/host-offers/:id/respond",
                launcher(Some(false)),
            ),
            ("POST", "/launcher/v1/p2p/sessions/:id/peers", launcher(Some(false))),
            ("POST", "/launcher/v1/realm-hosting/hubs/:id/stop", launcher(None)),
            ("POST", "/launcher/v1/guard/batches", launcher(None)),
            ("GET", "/rustic/v1/hunts/:id/status", launcher(None)),
            ("POST", "/rustic/v1/hunts/:id/evidence", CallerKind::HostCapability),
            ("GET", "/launcher/v1/legal", CallerKind::None),
            ("GET", "/mystic/v1/hunts/:id/status", CallerKind::None),
        ] {
            assert_eq!(rifts_caller_kind(method, path), kind, "{method} {path}");
        }
    }

    #[test]
    fn the_session_payload_repeats_the_token() {
        assert_eq!(
            dainty_service_kit::jsjson::stringify(&session_payload_1_14_7("t")),
            r#"{"error_code":null,"sessionid":"t","sessiontoken":"t","sessionToken":"t"}"#
        );
    }
}
