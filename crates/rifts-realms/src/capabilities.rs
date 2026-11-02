// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Host capabilities: the opaque secret a hosted server and its transport present, what it may do, the runtime it
//! resolves to, the hub presence cache, and `RealmsHuntSessions`, Guard's view of the hunts.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use rand::RngExt as _;
use rifts_guard::{CapabilityView, CompletionMode, HuntSessions, HuntView, RosterEntry, block_on_store};
use rifts_seam::{
    AccountId, Actor, CapabilityScope, PeerId, RealmId, Refusal, RuntimeAuth, Secret, SessionId, StoreError, UnixMillis,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::RealmsError;
use crate::db::{RealmsDb, RepoError};
use crate::hub::{HUB_RUNTIME_STATES, PresenceState};
use crate::hunt::{AdmissionState, HostCapabilityRecord, HuntSession, HuntState};
use crate::ids::{hex, random_uuid, seam_realm_id, seam_session_id, wire_session_id};
use crate::ports::Clock;
use crate::realm::js_trim;
use crate::repo::hub::RealmHubRepository;
use crate::repo::hunts::HuntRepository;

const SECRET_LENGTH: usize = 43;
const BASE64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn base64url(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..=chunk.len() {
            out.push(char::from(BASE64URL[((n >> (18 - 6 * i)) & 63) as usize]));
        }
    }
    out
}

/// 43 characters of unpadded base64url: 32 random bytes.
pub fn is_opaque_capability(raw: &str) -> bool {
    raw.len() == SECRET_LENGTH && raw.bytes().all(|b| BASE64URL.contains(&b))
}

/// Only the digest is stored.
pub fn hash_capability_secret(secret: &str) -> Option<String> {
    is_opaque_capability(secret).then(|| hex(&Sha256::digest(secret.as_bytes())))
}

/// The secret (never logged) and its digest.
pub fn generate_capability_secret() -> (Secret, String) {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes[..]);
    let secret = base64url(&bytes);
    let digest = hex(&Sha256::digest(secret.as_bytes()));
    (Secret::new(secret), digest)
}

/// `Authorization` for a host runtime's per-player call: absent, or `Bearer <token>` with one space.
pub fn parse_optional_bearer(header: Option<&str>) -> Result<Option<&str>, &'static str> {
    let Some(header) = header else {
        return Ok(None);
    };
    const MALFORMED: &str = "malformed Bearer authorization";
    let (scheme, token) = header.split_once(' ').ok_or(MALFORMED)?;
    if !scheme.eq_ignore_ascii_case("bearer") || token.is_empty() || token.chars().any(char::is_whitespace) {
        return Err(MALFORMED);
    }
    Ok(Some(token))
}

const BODY_IDENTITY_PATHS: [&str; 3] = ["/inventory", "/inventory/instanceditem", "/character"];

/// The player a host runtime's request is for: a query parameter, else the body of an inventory or character call.
pub fn request_identity(params: &[(String, String)], path: &str, body: Option<&Value>) -> Option<String> {
    for name in ["userId", "accountId", "phxAccountId"] {
        if let Some((_, value)) = params
            .iter()
            .find(|(key, value)| key == name && !js_trim(value).is_empty())
        {
            return Some(js_trim(value).to_owned());
        }
    }
    if !BODY_IDENTITY_PATHS.contains(&path.to_lowercase().as_str()) {
        return None;
    }
    let account = body?.as_object()?.get("accountId")?.as_str()?;
    let account = js_trim(account);
    (!account.is_empty()).then(|| account.to_owned())
}

/// A capability with its hunt; `present` only for a hub.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeContext {
    pub capability: HostCapabilityRecord,
    pub hunt: HuntSession,
    pub present: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeIdentity {
    pub user_id: String,
    pub roster: Vec<String>,
}

/// The user a signed per-player token names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignedUser<'a> {
    Absent,
    User(&'a str),
    /// A token without `userId`: refused.
    Missing,
}

/// Who a host runtime's request acts for, and the roster it may act within.
pub fn runtime_identity(
    context: &RuntimeContext,
    signed: SignedUser<'_>,
    requested: Option<&str>,
) -> Option<RuntimeIdentity> {
    let hunt = &context.hunt;
    let host = hunt.host_account_id.as_str();
    let user = match signed {
        SignedUser::Missing => return None,
        SignedUser::User(user) => user.to_owned(),
        SignedUser::Absent => requested
            .map(js_trim)
            .filter(|r| !r.is_empty())
            .unwrap_or(host)
            .to_owned(),
    };
    if hunt.is_realm_hub() {
        let mut roster = vec![host.to_owned()];
        for account in context.present.iter().flatten() {
            if !roster.contains(account) {
                roster.push(account.clone());
            }
        }
        return roster
            .contains(&user)
            .then_some(RuntimeIdentity { user_id: user, roster });
    }
    let mut roster: Vec<String> = hunt.roster.iter().map(|p| p.account_id.clone()).collect();
    match hunt.participant(&user) {
        // The host of a hunt always resolves; one that does not play joins the roster here, and its writes stay
        // refused elsewhere.
        None if user == host => {
            if hunt.host_participates == Some(false) {
                roster.push(host.to_owned());
            }
            Some(RuntimeIdentity { user_id: user, roster })
        }
        None => None,
        Some(p) if user != host && p.admission_state != AdmissionState::Admitted => None,
        Some(_) => Some(RuntimeIdentity { user_id: user, roster }),
    }
}

const RUNTIME_HUNT_STATES: [HuntState; 5] = [
    HuntState::Starting,
    HuntState::HostReady,
    HuntState::Admitting,
    HuntState::Running,
    // A drained hunt's server keeps serving its players.
    HuntState::Draining,
];

fn capability_refused(message: &str) -> RealmsError {
    RealmsError::Refused(Refusal {
        code: "P2P_CAPABILITY_INVALID".to_owned(),
        message: message.to_owned(),
    })
}

/// The part after the first `:`, when there is one.
fn route_session(session_id: &str) -> &str {
    session_id.split_once(':').map_or(session_id, |(_, rest)| rest)
}

const CACHE_HIT_MS: i64 = 1000;
const CACHE_PRUNE_AT: usize = 512;
const PRESENT_READ_LIMIT: u32 = 256;

/// (session id, generation) to (read at, accounts).
type Entries = HashMap<(String, i64), (UnixMillis, Vec<String>)>;

/// A hub generation's admitted accounts, trusted for a second.
#[derive(Debug, Default)]
pub struct HubPresenceCache {
    entries: Mutex<Entries>,
}

impl HubPresenceCache {
    pub fn get(&self, session_id: &str, generation: i64, now: UnixMillis) -> Option<Vec<String>> {
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries
            .get(&(session_id.to_owned(), generation))
            .filter(|(read_at, _)| now.0 - read_at.0 < CACHE_HIT_MS)
            .map(|(_, accounts)| accounts.clone())
    }

    pub fn put(&self, session_id: &str, generation: i64, now: UnixMillis, accounts: Vec<String>) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if entries.len() >= CACHE_PRUNE_AT {
            entries.retain(|_, (read_at, _)| now.0 - read_at.0 < CACHE_HIT_MS);
        }
        entries.insert((session_id.to_owned(), generation), (now, accounts));
    }

    /// Every generation of the session.
    pub fn invalidate(&self, session_id: &str) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(session, _), _| session != session_id);
    }

    pub fn len(&self) -> usize {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The host capabilities of the hunts this server runs.
pub struct HostCapabilities {
    db: RealmsDb,
    presence: Arc<HubPresenceCache>,
}

impl HostCapabilities {
    pub fn new(db: RealmsDb) -> Self {
        Self {
            db,
            presence: Arc::new(HubPresenceCache::default()),
        }
    }

    pub fn hub_presence(&self) -> Arc<HubPresenceCache> {
        self.presence.clone()
    }

    /// Guard's view, sharing this cache so route-ready and admission invalidate what Guard reads.
    pub fn hunt_sessions(&self, clock: Arc<dyn Clock>) -> RealmsHuntSessions {
        RealmsHuntSessions::with_cache(self.db.clone(), self.presence.clone(), clock)
    }

    fn hunts(&self) -> HuntRepository {
        HuntRepository::new(self.db.clone())
    }

    /// The secret goes to the host's launcher once; only its digest is stored.
    pub async fn mint(
        &self,
        hunt: &HuntSession,
        bridge_instance_id: &str,
        scopes: Vec<CapabilityScope>,
        issued_at: UnixMillis,
        expires_at: UnixMillis,
    ) -> Result<(Secret, HostCapabilityRecord), RealmsError> {
        let (secret, digest) = generate_capability_secret();
        let record = HostCapabilityRecord {
            capability_id: random_uuid(),
            secret_digest: digest,
            session_id: hunt.session_id.clone(),
            generation: hunt.generation,
            host_account_id: hunt.host_account_id.clone(),
            host_steam_id: hunt.host_steam_id.clone(),
            bridge_instance_id: bridge_instance_id.to_owned(),
            scopes,
            issued_at,
            expires_at,
            revoked_at: None,
            last_sequence: 0,
        };
        let actor = Actor::Account(AccountId(hunt.host_account_id.clone()));
        self.hunts().create_host_capability(&actor, &record).await?;
        Ok((secret, record))
    }

    fn matches_session(&self, record: &HostCapabilityRecord, session_id: &str) -> bool {
        record.session_id == session_id
            || seam_session_id(&record.session_id, self.db.node()).is_some_and(|seam| seam.to_string() == session_id)
    }

    /// Node's `RequireHostCapability`.
    pub async fn check(
        &self,
        header: Option<&str>,
        session_id: &str,
        scope: CapabilityScope,
        now: UnixMillis,
    ) -> Result<HostCapabilityRecord, RealmsError> {
        let Some(digest) = header.and_then(hash_capability_secret) else {
            return Err(capability_refused("A valid hunt host capability is required."));
        };
        let found = self
            .hunts()
            .find_host_capability_by_digest(&digest)
            .await
            .map_err(|_| capability_refused("The hunt host capability could not be verified."))?;
        let session_id = route_session(session_id);
        match found {
            Some(record)
                if record.revoked_at.is_none()
                    && record.expires_at > now
                    && self.matches_session(&record, session_id)
                    && record.scopes.contains(&scope) =>
            {
                Ok(record)
            }
            _ => Err(capability_refused("The hunt host capability is invalid or expired.")),
        }
    }

    /// Every live capability of the generation.
    pub async fn revoke(&self, session_id: &str, generation: i64, at: UnixMillis) -> Result<(), RealmsError> {
        self.hunts()
            .revoke_generation(&Actor::System("realms"), session_id, generation, at)
            .await?;
        Ok(())
    }

    /// The ADMITTED accounts of a hub generation, through the cache.
    pub async fn hub_present_accounts(
        &self,
        session_id: &str,
        generation: i64,
        now: UnixMillis,
    ) -> Result<Vec<String>, RealmsError> {
        if let Some(accounts) = self.presence.get(session_id, generation, now) {
            return Ok(accounts);
        }
        let rows = RealmHubRepository::new(self.db.clone())
            .list_by_state(session_id, generation, &[PresenceState::Admitted], PRESENT_READ_LIMIT)
            .await?;
        let accounts: Vec<String> = rows.into_iter().map(|row| row.account_id).collect();
        self.presence.put(session_id, generation, now, accounts.clone());
        Ok(accounts)
    }

    /// Node's `ResolveActiveHostRuntimeContext`: `None` for anything but a live runtime credential of its hunt.
    pub async fn resolve_runtime(
        &self,
        header: Option<&str>,
        now: UnixMillis,
    ) -> Result<Option<RuntimeContext>, RealmsError> {
        let Some(digest) = header.and_then(hash_capability_secret) else {
            return Ok(None);
        };
        let hunts = self.hunts();
        let Some(capability) = hunts.find_host_capability_by_digest(&digest).await? else {
            return Ok(None);
        };
        let usable = capability.revoked_at.is_none()
            && capability.expires_at > now
            && capability.scopes.contains(&CapabilityScope::GameServerRuntime);
        if !usable {
            return Ok(None);
        }
        let Some(hunt) = hunts.find_hunt(&capability.session_id).await? else {
            return Ok(None);
        };
        let same = hunt.generation == capability.generation
            && hunt.host_account_id == capability.host_account_id
            && hunt.host_steam_id == capability.host_steam_id;
        if !same {
            return Ok(None);
        }
        if hunt.is_realm_hub() {
            if !HUB_RUNTIME_STATES.contains(&hunt.state) {
                return Ok(None);
            }
            let present = self
                .hub_present_accounts(&hunt.session_id, hunt.generation, now)
                .await?;
            return Ok(Some(RuntimeContext {
                capability,
                hunt,
                present: Some(present),
            }));
        }
        if !RUNTIME_HUNT_STATES.contains(&hunt.state) {
            return Ok(None);
        }
        Ok(Some(RuntimeContext {
            capability,
            hunt,
            present: None,
        }))
    }

    pub fn runtime_auth(&self, context: &RuntimeContext, identity: &RuntimeIdentity) -> Option<RuntimeAuth> {
        let capability = &context.capability;
        let session = seam_session_id(&capability.session_id, self.db.node())?;
        Some(RuntimeAuth {
            capability_id: capability.capability_id.clone(),
            session,
            generation: u64::try_from(capability.generation).ok()?,
            host_account: AccountId(capability.host_account_id.clone()),
            scopes: capability.scopes.clone(),
            roster: identity.roster.iter().cloned().map(AccountId).collect(),
            host_peer: capability.host_steam_id.parse().ok().map(PeerId),
            bridge_instance: Some(capability.bridge_instance_id.clone()),
            wire_session: Some(capability.session_id.clone()),
        })
    }
}

#[derive(Clone)]
enum TimeSource {
    Realms(Arc<dyn Clock>),
    Guard(Arc<dyn rifts_guard::Clock>),
}

impl TimeSource {
    fn now(&self) -> UnixMillis {
        match self {
            Self::Realms(clock) => clock.now(),
            Self::Guard(clock) => clock.now(),
        }
    }
}

fn store_error(error: RepoError) -> StoreError {
    StoreError::Database(error.to_string())
}

fn non_blank(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// `frozenLoadouts` (or `frozen_loadouts`) entries with both an account and a character.
fn frozen_characters(hunt: &HuntSession) -> Vec<(AccountId, String)> {
    let list = hunt
        .extra
        .get("frozenLoadouts")
        .or_else(|| hunt.extra.get("frozen_loadouts"))
        .and_then(Value::as_array);
    list.into_iter()
        .flatten()
        .filter_map(|entry| {
            let account = non_blank(entry.get("accountId").or_else(|| entry.get("account_id")))?;
            let character = non_blank(entry.get("characterId").or_else(|| entry.get("character_id")))?;
            Some((AccountId(account), character))
        })
        .collect()
}

/// Realms' hunts as rifts-guard reads them.
#[derive(Clone)]
pub struct RealmsHuntSessions {
    db: RealmsDb,
    cache: Arc<HubPresenceCache>,
    time: TimeSource,
}

impl RealmsHuntSessions {
    pub fn new(db: RealmsDb, clock: Arc<dyn Clock>) -> Self {
        Self::with_cache(db, Arc::new(HubPresenceCache::default()), clock)
    }

    pub fn with_cache(db: RealmsDb, cache: Arc<HubPresenceCache>, clock: Arc<dyn Clock>) -> Self {
        Self {
            db,
            cache,
            time: TimeSource::Realms(clock),
        }
    }

    pub fn with_guard_clock(db: RealmsDb, clock: Arc<dyn rifts_guard::Clock>) -> Self {
        Self::with_cache_and_guard_clock(db, Arc::new(HubPresenceCache::default()), clock)
    }

    pub fn with_cache_and_guard_clock(
        db: RealmsDb,
        cache: Arc<HubPresenceCache>,
        clock: Arc<dyn rifts_guard::Clock>,
    ) -> Self {
        Self {
            db,
            cache,
            time: TimeSource::Guard(clock),
        }
    }

    pub fn presence_cache(&self) -> Arc<HubPresenceCache> {
        self.cache.clone()
    }

    /// Under both of the session's spellings.
    pub fn invalidate_hub_presence(&self, session: &SessionId) {
        self.cache.invalidate(&wire_session_id(session));
        self.cache.invalidate(&session.to_string());
    }

    fn view(&self, hunt: HuntSession, asked: &SessionId) -> HuntView {
        let node = self.db.node();
        let session = seam_session_id(&hunt.session_id, node)
            .or_else(|| hunt.session_id.parse().ok())
            .unwrap_or_else(|| asked.clone());
        let realm = hunt
            .realm_id
            .as_deref()
            .and_then(|id| seam_realm_id(id, node).or_else(|| id.parse::<RealmId>().ok()));
        HuntView {
            session,
            generation: u64::try_from(hunt.generation).unwrap_or(0),
            state: hunt.state.as_str().to_owned(),
            pre_suspect_state: hunt.pre_suspect_state.map(|s| s.as_str().to_owned()),
            host: (!hunt.host_account_id.is_empty()).then(|| AccountId(hunt.host_account_id.clone())),
            realm,
            hub: hunt.hub_kind,
            roster: hunt
                .roster
                .iter()
                .map(|p| RosterEntry {
                    account: AccountId(p.account_id.clone()),
                    admitted: p.admission_state == AdmissionState::Admitted,
                })
                .collect(),
            frozen_characters: frozen_characters(&hunt),
            completion_mode: hunt.guard_completion_mode.as_deref().and_then(CompletionMode::parse),
        }
    }
}

impl HuntSessions for RealmsHuntSessions {
    fn hunt(&self, session: &SessionId) -> Result<Option<HuntView>, StoreError> {
        let hunts = HuntRepository::new(self.db.clone());
        let (wire, seam) = (wire_session_id(session), session.to_string());
        let found = block_on_store(async move {
            match hunts.find_hunt(&wire).await.map_err(store_error)? {
                Some(hunt) => Ok(Some(hunt)),
                None => hunts.find_hunt(&seam).await.map_err(store_error),
            }
        })?;
        Ok(found.map(|hunt| self.view(hunt, session)))
    }

    fn hub_present_accounts(&self, session: &SessionId, generation: u64) -> Result<Vec<AccountId>, StoreError> {
        let Ok(generation) = i64::try_from(generation) else {
            return Ok(Vec::new());
        };
        let (wire, seam) = (wire_session_id(session), session.to_string());
        let now = self.time.now();
        if let Some(accounts) = self.cache.get(&wire, generation, now) {
            return Ok(accounts.into_iter().map(AccountId).collect());
        }
        let hub = RealmHubRepository::new(self.db.clone());
        let key = wire.clone();
        let rows = block_on_store(async move {
            let present = [PresenceState::Admitted];
            let rows = hub
                .list_by_state(&wire, generation, &present, PRESENT_READ_LIMIT)
                .await
                .map_err(store_error)?;
            if !rows.is_empty() {
                return Ok(rows);
            }
            hub.list_by_state(&seam, generation, &present, PRESENT_READ_LIMIT)
                .await
                .map_err(store_error)
        })?;
        let accounts: Vec<String> = rows.into_iter().map(|row| row.account_id).collect();
        self.cache.put(&key, generation, now, accounts.clone());
        Ok(accounts.into_iter().map(AccountId).collect())
    }

    fn capability_by_digest(&self, digest: &str) -> Result<Option<CapabilityView>, StoreError> {
        let hunts = HuntRepository::new(self.db.clone());
        let digest = digest.to_owned();
        let found =
            block_on_store(async move { hunts.find_host_capability_by_digest(&digest).await.map_err(store_error) })?;
        Ok(found.map(|c| CapabilityView {
            capability_id: c.capability_id,
            expires_at: c.expires_at,
            revoked: c.revoked_at.is_some(),
        }))
    }

    fn resolve_capability(
        &self,
        digest: &str,
        session_id: &str,
        scope: CapabilityScope,
        now: rifts_seam::UnixMillis,
    ) -> Result<Option<RuntimeAuth>, StoreError> {
        let hunts = HuntRepository::new(self.db.clone());
        let digest = digest.to_owned();
        let found =
            block_on_store(async move { hunts.find_host_capability_by_digest(&digest).await.map_err(store_error) })?;
        let Some(capability) = found else {
            return Ok(None);
        };
        if capability.revoked_at.is_some() || capability.expires_at <= now || !capability.scopes.contains(&scope) {
            return Ok(None);
        }
        let Some(session) = seam_session_id(&capability.session_id, self.db.node()) else {
            return Ok(None);
        };
        if session_id != capability.session_id && session_id != session.to_string() {
            return Ok(None);
        }
        let Ok(generation) = u64::try_from(capability.generation) else {
            return Ok(None);
        };
        Ok(Some(RuntimeAuth {
            capability_id: capability.capability_id,
            session,
            generation,
            host_account: AccountId(capability.host_account_id),
            scopes: capability.scopes,
            roster: Vec::new(),
            host_peer: capability.host_steam_id.parse().ok().map(PeerId),
            bridge_instance: Some(capability.bridge_instance_id),
            wire_session: Some(capability.session_id),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_hits_within_a_second_and_invalidates_by_session() {
        let cache = HubPresenceCache::default();
        cache.put("s", 1, UnixMillis(1000), vec!["a".into()]);
        cache.put("s", 2, UnixMillis(1000), vec!["b".into()]);
        cache.put("t", 1, UnixMillis(1000), vec!["c".into()]);
        assert_eq!(cache.get("s", 1, UnixMillis(1999)), Some(vec!["a".to_owned()]));
        assert_eq!(cache.get("s", 1, UnixMillis(2000)), None);
        cache.invalidate("s");
        assert_eq!(cache.get("s", 2, UnixMillis(1000)), None);
        assert_eq!(cache.get("t", 1, UnixMillis(1000)), Some(vec!["c".to_owned()]));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn a_full_cache_drops_old_entries_first() {
        let cache = HubPresenceCache::default();
        for i in 0..CACHE_PRUNE_AT {
            cache.put(&format!("s{i}"), 1, UnixMillis(0), Vec::new());
        }
        cache.put("fresh", 1, UnixMillis(5000), Vec::new());
        assert_eq!(cache.len(), 1);
    }
}
