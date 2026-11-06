// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Host capabilities: the opaque secret a hosted server and its transport present, what it may do, the runtime it
//! resolves to, the hub presence cache, and `RealmsHuntSessions`, Guard's view of the hunts.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rand::RngExt as _;
use rifts_guard::{CapabilityView, CompletionMode, HuntSessions, HuntView, RosterEntry, block_on_store};
use rifts_seam::{AccountId, PeerId, RealmId, SessionId};
use rifts_seam::{Actor, CapabilityScope, Refusal, RuntimeAuth, Secret, StoreError, UnixMillis};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::{
    RealmsError,
    db::{RealmsDb, RepoError},
    hub::{HUB_RUNTIME_STATES, PresenceState},
    hunt::{AdmissionState, HostCapabilityRecord, HuntSession, HuntState},
    ids::{hex, random_uuid, seam_realm_id, seam_session_id, wire_session_id},
    ports::Clock,
    realm::js_trim,
    repo::{hub::RealmHubRepository, hunts::HuntRepository},
};

/// The URL-safe alphabet, unpadded: 32 random bytes are 43 characters.
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn base64url(bytes: &[u8]) -> String {
    let mut text = String::new();
    let (mut pending, mut bits) = (0u32, 0u32);
    for &byte in bytes {
        pending = (pending << 8) | u32::from(byte);
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            text.push(char::from(ALPHABET[((pending >> bits) & 0x3f) as usize]));
        }
    }
    if bits > 0 {
        text.push(char::from(ALPHABET[((pending << (6 - bits)) & 0x3f) as usize]));
    }
    text
}

fn digest_hex(secret: &str) -> String {
    hex(Sha256::digest(secret).as_slice())
}

/// 43 characters of unpadded base64url: 32 random bytes.
pub fn is_opaque_capability(raw: &str) -> bool {
    raw.len() == 43 && raw.bytes().all(|b| ALPHABET.contains(&b))
}

/// Only the digest is stored.
pub fn hash_capability_secret(secret: &str) -> Option<String> {
    if is_opaque_capability(secret) {
        Some(digest_hex(secret))
    } else {
        None
    }
}

/// The secret (never logged) and its digest.
pub fn generate_capability_secret() -> (Secret, String) {
    let mut random = [0u8; 32];
    rand::rng().fill(&mut random[..]);
    let text = base64url(&random);
    let digest = digest_hex(&text);
    (Secret::new(text), digest)
}

/// `Authorization` for a host runtime's per-player call: absent, or `Bearer <token>` with one space.
pub fn parse_optional_bearer(header: Option<&str>) -> Result<Option<&str>, &'static str> {
    let Some(value) = header else {
        return Ok(None);
    };
    match value.split_once(' ') {
        Some((scheme, token))
            if scheme.eq_ignore_ascii_case("bearer") && !token.is_empty() && !token.contains(char::is_whitespace) =>
        {
            Ok(Some(token))
        }
        _ => Err("malformed Bearer authorization"),
    }
}

/// The player a host runtime's request is for: a query parameter, else the body of an inventory or character call.
pub fn request_identity(params: &[(String, String)], path: &str, body: Option<&Value>) -> Option<String> {
    let from_query = ["userId", "accountId", "phxAccountId"].iter().find_map(|wanted| {
        params
            .iter()
            .map(|(key, value)| (key, js_trim(value)))
            .find(|(key, value)| key == wanted && !value.is_empty())
    });
    if let Some((_, value)) = from_query {
        return Some(value.to_owned());
    }
    let lowered = path.to_lowercase();
    if !matches!(
        lowered.as_str(),
        "/inventory" | "/inventory/instanceditem" | "/character"
    ) {
        return None;
    }
    let account = js_trim(body?.as_object()?.get("accountId")?.as_str()?);
    if account.is_empty() {
        None
    } else {
        Some(account.to_owned())
    }
}

/// A capability with its hunt; `present` only for a hub.
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeContext {
    /// The capability the request carried.
    pub capability: HostCapabilityRecord,
    /// The session the capability was issued for.
    pub hunt: HuntSession,
    /// For a hub, the accounts admitted to its generation.
    pub present: Option<Vec<String>>,
}

/// Who a host runtime's request acts for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeIdentity {
    /// The account it acts for.
    pub user_id: String,
    /// The session's roster, the accounts it may act for.
    pub roster: Vec<String>,
}

/// The user a signed per-player token names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignedUser<'a> {
    /// No signed token: the requested user, or the host.
    Absent,
    /// The token names this user.
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
    let RuntimeContext { hunt, present, .. } = context;
    let host = hunt.host_account_id.as_str();
    let user_id = match signed {
        SignedUser::Missing => return None,
        SignedUser::User(named) => named,
        SignedUser::Absent => requested.map(js_trim).filter(|asked| !asked.is_empty()).unwrap_or(host),
    }
    .to_owned();
    let mut roster: Vec<String>;
    if hunt.is_realm_hub() {
        // A hub's roster is its host and whoever is present now.
        roster = vec![host.to_owned()];
        present.iter().flatten().for_each(|account| {
            if !roster.contains(account) {
                roster.push(account.to_owned());
            }
        });
        return roster.contains(&user_id).then_some(RuntimeIdentity { user_id, roster });
    }
    roster = hunt.roster.iter().map(|player| player.account_id.clone()).collect();
    let is_host = user_id == host;
    let allowed = match hunt.participant(&user_id) {
        Some(player) => is_host || player.admission_state == AdmissionState::Admitted,
        // The host of a hunt always resolves; one that does not play joins the roster here, and its writes stay
        // refused elsewhere.
        None if is_host => {
            if hunt.host_participates == Some(false) {
                roster.push(host.to_owned());
            }
            true
        }
        None => false,
    };
    allowed.then_some(RuntimeIdentity { user_id, roster })
}

/// A hunt's runtime credential works while its server runs, including a drained hunt's, which keeps serving its
/// players.
fn runtime_hunt_state(state: HuntState) -> bool {
    use HuntState::*;
    matches!(state, Starting | HostReady | Admitting | Running | Draining)
}

fn capability_refused(message: &str) -> RealmsError {
    let refusal = Refusal {
        code: String::from("P2P_CAPABILITY_INVALID"),
        message: String::from(message),
    };
    RealmsError::Refused(refusal)
}

/// The part after the first `:`, when there is one.
fn route_session(session_id: &str) -> &str {
    match session_id.find(':') {
        Some(colon) => &session_id[colon + 1..],
        None => session_id,
    }
}

fn live_for(capability: &HostCapabilityRecord, scope: &CapabilityScope, now: UnixMillis) -> bool {
    capability.revoked_at.is_none() && now < capability.expires_at && capability.scopes.contains(scope)
}

/// The seam's view of a capability, for a session already resolved; `None` for a generation it cannot hold.
fn auth_for(capability: &HostCapabilityRecord, session: SessionId, roster: Vec<AccountId>) -> Option<RuntimeAuth> {
    let generation = u64::try_from(capability.generation).ok()?;
    Some(RuntimeAuth {
        host_account: AccountId(capability.host_account_id.clone()),
        host_peer: capability.host_steam_id.parse().ok().map(PeerId),
        bridge_instance: Some(capability.bridge_instance_id.clone()),
        wire_session: Some(capability.session_id.clone()),
        capability_id: capability.capability_id.clone(),
        scopes: capability.scopes.clone(),
        session,
        generation,
        roster,
    })
}

/// A cached read of a hub's present accounts is trusted this long.
const CACHE_HIT_MS: i64 = 1000;
/// At this many entries, a write first drops the stale ones.
const CACHE_PRUNE_AT: usize = 512;
const PRESENT_READ_LIMIT: u32 = 256;

/// (session id, generation) to (read at, accounts).
type Entries = HashMap<(String, i64), (UnixMillis, Vec<String>)>;

/// A hub generation's admitted accounts, trusted for a second.
#[derive(Debug, Default)]
pub struct HubPresenceCache {
    entries: Mutex<Entries>,
}

fn fresh(read_at: UnixMillis, now: UnixMillis) -> bool {
    now.0 - read_at.0 < CACHE_HIT_MS
}

impl HubPresenceCache {
    fn locked(&self) -> MutexGuard<'_, Entries> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The generation's cached accounts, while still fresh.
    pub fn get(&self, session_id: &str, generation: i64, now: UnixMillis) -> Option<Vec<String>> {
        let key = (session_id.to_owned(), generation);
        let entries = self.locked();
        let (read_at, accounts) = entries.get(&key)?;
        fresh(*read_at, now).then(|| accounts.clone())
    }

    /// Caches the generation's accounts as read at `now`, pruning stale entries when the cache is large.
    pub fn put(&self, session_id: &str, generation: i64, now: UnixMillis, accounts: Vec<String>) {
        let mut entries = self.locked();
        if entries.len() >= CACHE_PRUNE_AT {
            entries.retain(|_, entry| fresh(entry.0, now));
        }
        entries.insert((session_id.to_owned(), generation), (now, accounts));
    }

    /// Every generation of the session.
    pub fn invalidate(&self, session_id: &str) {
        self.locked().retain(|key, _| key.0 != session_id);
    }

    /// How many generations are cached.
    pub fn len(&self) -> usize {
        self.locked().len()
    }

    /// Whether nothing is cached.
    pub fn is_empty(&self) -> bool {
        self.locked().is_empty()
    }
}

/// The host capabilities of the hunts this server runs.
pub struct HostCapabilities {
    presence: Arc<HubPresenceCache>,
    db: RealmsDb,
}

impl HostCapabilities {
    /// Capabilities over `db`, with an empty hub presence cache.
    pub fn new(db: RealmsDb) -> Self {
        Self {
            presence: Arc::new(HubPresenceCache::default()),
            db,
        }
    }

    /// The hub presence cache, shared.
    pub fn hub_presence(&self) -> Arc<HubPresenceCache> {
        Arc::clone(&self.presence)
    }

    /// Guard's view, sharing this cache so route-ready and admission invalidate what Guard reads.
    pub fn hunt_sessions(&self, clock: Arc<dyn Clock>) -> RealmsHuntSessions {
        RealmsHuntSessions::with_cache(self.db.clone(), self.hub_presence(), clock)
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
        let (secret, secret_digest) = generate_capability_secret();
        let host = hunt.host_account_id.clone();
        let record = HostCapabilityRecord {
            secret_digest,
            scopes,
            issued_at,
            expires_at,
            capability_id: random_uuid(),
            bridge_instance_id: String::from(bridge_instance_id),
            session_id: hunt.session_id.clone(),
            generation: hunt.generation,
            host_steam_id: hunt.host_steam_id.clone(),
            host_account_id: host.clone(),
            revoked_at: None,
            last_sequence: 0,
        };
        self.hunts()
            .create_host_capability(&Actor::Account(AccountId(host)), &record)
            .await?;
        Ok((secret, record))
    }

    /// A route may name the session by its stored id or by the seam's spelling of it.
    fn matches_session(&self, record: &HostCapabilityRecord, session_id: &str) -> bool {
        let seam_spelling = || seam_session_id(&record.session_id, self.db.node()).map(|seam| seam.to_string());
        record.session_id == session_id || seam_spelling().as_deref() == Some(session_id)
    }

    /// Node's `RequireHostCapability`.
    pub async fn check(
        &self,
        header: Option<&str>,
        session_id: &str,
        scope: CapabilityScope,
        now: UnixMillis,
    ) -> Result<HostCapabilityRecord, RealmsError> {
        let digest = header
            .and_then(hash_capability_secret)
            .ok_or_else(|| capability_refused("A valid hunt host capability is required."))?;
        let lookup = self.hunts().find_host_capability_by_digest(&digest).await;
        let found = lookup.map_err(|_| capability_refused("The hunt host capability could not be verified."))?;
        let session_id = route_session(session_id);
        found
            .filter(|record| live_for(record, &scope, now) && self.matches_session(record, session_id))
            .ok_or_else(|| capability_refused("The hunt host capability is invalid or expired."))
    }

    /// Every live capability of the generation.
    pub async fn revoke(&self, session_id: &str, generation: i64, at: UnixMillis) -> Result<(), RealmsError> {
        let system = Actor::System("realms");
        self.hunts()
            .revoke_generation(&system, session_id, generation, at)
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
        if let Some(cached) = self.presence.get(session_id, generation, now) {
            return Ok(cached);
        }
        let hub = RealmHubRepository::new(self.db.clone());
        let admitted = [PresenceState::Admitted];
        let rows = hub
            .list_by_state(session_id, generation, &admitted, PRESENT_READ_LIMIT)
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
        let capability = match hunts.find_host_capability_by_digest(&digest).await? {
            Some(found) if live_for(&found, &CapabilityScope::GameServerRuntime, now) => found,
            _ => return Ok(None),
        };
        let Some(hunt) = hunts.find_hunt(&capability.session_id).await? else {
            return Ok(None);
        };
        // The credential belongs to this generation and this host.
        let owned = (hunt.generation, &hunt.host_account_id, &hunt.host_steam_id)
            == (
                capability.generation,
                &capability.host_account_id,
                &capability.host_steam_id,
            );
        let running = if hunt.is_realm_hub() {
            HUB_RUNTIME_STATES.contains(&hunt.state)
        } else {
            runtime_hunt_state(hunt.state)
        };
        if !owned || !running {
            return Ok(None);
        }
        let present = if hunt.is_realm_hub() {
            Some(
                self.hub_present_accounts(&hunt.session_id, hunt.generation, now)
                    .await?,
            )
        } else {
            None
        };
        Ok(Some(RuntimeContext {
            capability,
            hunt,
            present,
        }))
    }

    /// The seam's view of a resolved capability; `None` when its session id does not parse.
    pub fn runtime_auth(&self, context: &RuntimeContext, identity: &RuntimeIdentity) -> Option<RuntimeAuth> {
        let capability = &context.capability;
        let session = seam_session_id(&capability.session_id, self.db.node())?;
        let roster = identity
            .roster
            .iter()
            .map(|account| AccountId(account.clone()))
            .collect();
        auth_for(capability, session, roster)
    }
}

/// Guard's and realms' clocks are separate traits; the view reads whichever it was built with.
type Now = Arc<dyn Fn() -> UnixMillis + Send + Sync>;

fn realms_now(clock: Arc<dyn Clock>) -> Now {
    Arc::new(move || clock.now())
}

fn guard_now(clock: Arc<dyn rifts_guard::Clock>) -> Now {
    Arc::new(move || clock.now())
}

fn store_error(error: RepoError) -> StoreError {
    StoreError::Database(format!("{error}"))
}

/// A field under either spelling, trimmed, when it is a non-empty string.
fn text_field(entry: &Value, camel: &str, snake: &str) -> Option<String> {
    let text = entry.get(camel).or_else(|| entry.get(snake))?.as_str()?.trim();
    if text.is_empty() { None } else { Some(text.to_owned()) }
}

/// `frozenLoadouts` (or `frozen_loadouts`) entries with both an account and a character.
fn frozen_characters(hunt: &HuntSession) -> Vec<(AccountId, String)> {
    let extra = &hunt.extra;
    let Some(entries) = extra
        .get("frozenLoadouts")
        .or_else(|| extra.get("frozen_loadouts"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let mut frozen = Vec::new();
    for entry in entries {
        let account = text_field(entry, "accountId", "account_id");
        let character = text_field(entry, "characterId", "character_id");
        if let (Some(account), Some(character)) = (account, character) {
            frozen.push((AccountId(account), character));
        }
    }
    frozen
}

/// Realms' hunts as rifts-guard reads them.
#[derive(Clone)]
pub struct RealmsHuntSessions {
    cache: Arc<HubPresenceCache>,
    now: Now,
    db: RealmsDb,
}

impl RealmsHuntSessions {
    fn assemble(db: RealmsDb, cache: Option<Arc<HubPresenceCache>>, now: Now) -> Self {
        Self {
            cache: cache.unwrap_or_default(),
            now,
            db,
        }
    }

    /// Guard's view with its own cache, on a realms clock.
    pub fn new(db: RealmsDb, clock: Arc<dyn Clock>) -> Self {
        Self::assemble(db, None, realms_now(clock))
    }

    /// Guard's view sharing `cache`, on a realms clock.
    pub fn with_cache(db: RealmsDb, cache: Arc<HubPresenceCache>, clock: Arc<dyn Clock>) -> Self {
        Self::assemble(db, Some(cache), realms_now(clock))
    }

    /// Guard's view with its own cache, on Guard's clock.
    pub fn with_guard_clock(db: RealmsDb, clock: Arc<dyn rifts_guard::Clock>) -> Self {
        Self::assemble(db, None, guard_now(clock))
    }

    /// Guard's view sharing `cache`, on Guard's clock.
    pub fn with_cache_and_guard_clock(
        db: RealmsDb,
        cache: Arc<HubPresenceCache>,
        clock: Arc<dyn rifts_guard::Clock>,
    ) -> Self {
        Self::assemble(db, Some(cache), guard_now(clock))
    }

    /// The hub presence cache, shared.
    pub fn presence_cache(&self) -> Arc<HubPresenceCache> {
        Arc::clone(&self.cache)
    }

    /// Under both of the session's spellings.
    pub fn invalidate_hub_presence(&self, session: &SessionId) {
        for spelling in [wire_session_id(session), session.to_string()] {
            self.cache.invalidate(&spelling);
        }
    }

    fn hunts(&self) -> HuntRepository {
        HuntRepository::new(self.db.clone())
    }

    fn capability(&self, digest: &str) -> Result<Option<HostCapabilityRecord>, StoreError> {
        let hunts = self.hunts();
        let digest = digest.to_owned();
        block_on_store(async move { hunts.find_host_capability_by_digest(&digest).await.map_err(store_error) })
    }

    fn view(&self, hunt: HuntSession, asked: &SessionId) -> HuntView {
        let node = self.db.node();
        let session = seam_session_id(&hunt.session_id, node)
            .or_else(|| hunt.session_id.parse().ok())
            .unwrap_or_else(|| asked.clone());
        let realm = match hunt.realm_id.as_deref() {
            Some(id) => seam_realm_id(id, node).or_else(|| id.parse::<RealmId>().ok()),
            None => None,
        };
        let roster = hunt
            .roster
            .iter()
            .map(|player| RosterEntry {
                admitted: player.admission_state == AdmissionState::Admitted,
                account: AccountId(player.account_id.clone()),
            })
            .collect();
        let host = if hunt.host_account_id.is_empty() {
            None
        } else {
            Some(AccountId(hunt.host_account_id.clone()))
        };
        HuntView {
            frozen_characters: frozen_characters(&hunt),
            completion_mode: hunt.guard_completion_mode.as_deref().and_then(CompletionMode::parse),
            generation: u64::try_from(hunt.generation).unwrap_or_default(),
            state: hunt.state.as_str().into(),
            pre_suspect_state: hunt.pre_suspect_state.map(|before| before.as_str().into()),
            hub: hunt.hub_kind,
            session,
            host,
            realm,
            roster,
        }
    }
}

impl HuntSessions for RealmsHuntSessions {
    fn hunt(&self, session: &SessionId) -> Result<Option<HuntView>, StoreError> {
        let hunts = self.hunts();
        let spellings = [wire_session_id(session), session.to_string()];
        let found = block_on_store(async move {
            // The wire spelling first, then the seam's.
            for spelling in &spellings {
                if let Some(hunt) = hunts.find_hunt(spelling).await.map_err(store_error)? {
                    return Ok(Some(hunt));
                }
            }
            Ok(None)
        })?;
        Ok(found.map(|hunt| self.view(hunt, session)))
    }

    fn hub_present_accounts(&self, session: &SessionId, generation: u64) -> Result<Vec<AccountId>, StoreError> {
        let Ok(generation) = i64::try_from(generation) else {
            return Ok(Vec::new());
        };
        let wire = wire_session_id(session);
        let now = (self.now)();
        let accounts = match self.cache.get(&wire, generation, now) {
            Some(cached) => cached,
            None => {
                let hub = RealmHubRepository::new(self.db.clone());
                let spellings = [wire.clone(), session.to_string()];
                let rows = block_on_store(async move {
                    let admitted = [PresenceState::Admitted];
                    let mut rows = Vec::new();
                    // The seam's spelling only when the wire one has nobody.
                    for spelling in &spellings {
                        rows = hub
                            .list_by_state(spelling, generation, &admitted, PRESENT_READ_LIMIT)
                            .await
                            .map_err(store_error)?;
                        if !rows.is_empty() {
                            break;
                        }
                    }
                    Ok(rows)
                })?;
                let read: Vec<String> = rows.into_iter().map(|row| row.account_id).collect();
                self.cache.put(&wire, generation, now, read.clone());
                read
            }
        };
        Ok(accounts.into_iter().map(AccountId).collect())
    }

    fn capability_by_digest(&self, digest: &str) -> Result<Option<CapabilityView>, StoreError> {
        let view = |record: HostCapabilityRecord| CapabilityView {
            revoked: record.revoked_at.is_some(),
            expires_at: record.expires_at,
            capability_id: record.capability_id,
        };
        Ok(self.capability(digest)?.map(view))
    }

    fn resolve_capability(
        &self,
        digest: &str,
        session_id: &str,
        scope: CapabilityScope,
        now: rifts_seam::UnixMillis,
    ) -> Result<Option<RuntimeAuth>, StoreError> {
        let Some(capability) = self.capability(digest)?.filter(|found| live_for(found, &scope, now)) else {
            return Ok(None);
        };
        let session = seam_session_id(&capability.session_id, self.db.node())
            .filter(|seam| session_id == capability.session_id || session_id == seam.to_string());
        Ok(session.and_then(|session| auth_for(&capability, session, Vec::new())))
    }
}

#[cfg(test)]
mod tests {
    use super::{CACHE_PRUNE_AT, HubPresenceCache, UnixMillis as At, base64url, is_opaque_capability};

    fn accounts(name: &str) -> Vec<String> {
        vec![name.to_owned()]
    }

    #[test]
    fn cache_hits_within_a_second_and_invalidates_by_session() {
        let cache = HubPresenceCache::default();
        for (session, generation, who) in [("s", 1, "a"), ("s", 2, "b"), ("t", 1, "c")] {
            cache.put(session, generation, At(1000), accounts(who));
        }
        assert_eq!(cache.get("s", 1, At(1999)), Some(accounts("a")));
        assert_eq!(cache.get("s", 1, At(2000)), None);
        cache.invalidate("s");
        assert_eq!(cache.get("s", 2, At(1000)), None);
        assert_eq!(cache.get("t", 1, At(1000)), Some(accounts("c")));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn a_full_cache_drops_old_entries_first() {
        let cache = HubPresenceCache::default();
        (0..CACHE_PRUNE_AT).for_each(|n| cache.put(&format!("s{n}"), 1, At(0), Vec::new()));
        cache.put("fresh", 1, At(5000), Vec::new());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn secrets_are_unpadded_base64url() {
        assert_eq!(base64url(b""), "");
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
        assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
        assert!(is_opaque_capability(&base64url(&[7; 32])));
    }
}
