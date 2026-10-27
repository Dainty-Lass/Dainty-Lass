//! Host capabilities (`p2p/security/hostCapabilities.ts`): the 43-character secret a hosted game server and its
//! transport send as `x-mystic-host-capability`. Only its SHA-256 is stored; a capability is good for its own session,
//! until it expires or its generation is revoked, and for the scopes it was minted with.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use module_api::NodeId;
use rifts_seam::{AccountId, CapabilityScope, Refusal, RuntimeAuth, Secret, UnixMillis};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::RealmsError;
use crate::db::RealmsDb;
use crate::hub::PresenceState;
use crate::hunt::{AdmissionState, HostCapabilityRecord, HuntSession, HuntState};
use crate::ids::{random_uuid, seam_session_id};
use crate::repo::hub::RealmHubRepository;
use crate::repo::hunts::HuntRepository;

/// `OPAQUE_CAPABILITY`: 43 characters of base64url, the encoding of 32 random bytes.
pub fn is_opaque_capability(raw: &str) -> bool {
    raw.len() == 43 && raw.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().fold(0u32, |acc, b| (acc << 8) | u32::from(*b)) << (8 * (3 - chunk.len()));
        for i in 0..=chunk.len() {
            out.push(char::from(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize]));
        }
    }
    out
}

/// The SHA-256 hex of a secret; `None` when it is not one.
pub fn hash_capability_secret(secret: &str) -> Option<String> {
    is_opaque_capability(secret).then(|| crate::ids::hex(&Sha256::digest(secret.as_bytes())))
}

/// `GenerateHostCapabilitySecret`: the secret and its digest.
pub fn generate_capability_secret() -> (Secret, String) {
    let bytes: [u8; 32] = rand::random();
    let secret = base64url(&bytes);
    let digest = crate::ids::hex(&Sha256::digest(secret.as_bytes()));
    (Secret::new(secret), digest)
}

fn invalid(message: &str) -> RealmsError {
    RealmsError::Refused(Refusal {
        code: "P2P_CAPABILITY_INVALID".to_owned(),
        message: message.to_owned(),
    })
}

/// `ParseOptionalHostRuntimeBearerToken`: the token of a `Bearer` header; a malformed header is an error.
pub fn parse_optional_bearer(raw: Option<&str>) -> Result<Option<&str>, &'static str> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let (scheme, token) = raw.split_once(' ').ok_or("malformed Bearer authorization")?;
    if !scheme.eq_ignore_ascii_case("bearer") || token.is_empty() || token.chars().any(char::is_whitespace) {
        return Err("malformed Bearer authorization");
    }
    Ok(Some(token))
}

/// `ResolvePlayerHostRuntimeRequestIdentity`: the account a per-player route names (params first, then the
/// `accountId` of the three bodies that carry one).
pub fn request_identity(params: &[(String, String)], path: &str, body: Option<&Value>) -> Option<String> {
    for key in ["userId", "accountId", "phxAccountId"] {
        if let Some((_, value)) = params.iter().find(|(name, _)| name == key) {
            let trimmed = crate::realm::js_trim(value);
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }
    }
    let path = path.to_lowercase();
    if matches!(path.as_str(), "/inventory" | "/inventory/instanceditem" | "/character")
        && let Some(Value::Object(body)) = body
        && let Some(Value::String(account)) = body.get("accountId")
    {
        let trimmed = crate::realm::js_trim(account);
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
    }
    None
}

/// A drained hunt's server keeps serving its players; no new admission needs that state.
const HUNT_RUNTIME_STATES: &[HuntState] = &[
    HuntState::Starting,
    HuntState::HostReady,
    HuntState::Admitting,
    HuntState::Running,
    HuntState::Draining,
];

/// `ActiveHostRuntimeContext`: a live runtime capability, its session and, for a hub, who is present.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeContext {
    pub capability: HostCapabilityRecord,
    pub hunt: HuntSession,
    pub present: Option<Vec<String>>,
}

/// Who a runtime request acts for, and the accounts the runtime may act for at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeIdentity {
    pub user_id: String,
    pub roster: Vec<String>,
}

/// The signed identity a request carried: none, a JWT with `userId`, or a JWT without one (refused).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignedUser<'a> {
    Absent,
    User(&'a str),
    Missing,
}

/// `BuildPlayerHostRuntimeAuthData`: the signed identity wins, then a route's account, then the host. A hub's runtime
/// acts for the host and the members present; a hunt's for its admitted roster (and the host during startup).
pub fn runtime_identity(context: &RuntimeContext, signed: SignedUser<'_>, requested: Option<&str>) -> Option<RuntimeIdentity> {
    let host = &context.capability.host_account_id;
    let user_id = match signed {
        SignedUser::Missing => return None,
        SignedUser::User(user) => user.to_owned(),
        SignedUser::Absent => requested.map_or_else(|| host.clone(), |r| crate::realm::js_trim(r).to_owned()),
    };
    if let Some(present) = &context.present {
        let mut roster = vec![host.clone()];
        for account in present {
            if !roster.contains(account) {
                roster.push(account.clone());
            }
        }
        return roster.contains(&user_id).then_some(RuntimeIdentity { user_id, roster });
    }
    let roster: Vec<String> = context.hunt.roster.iter().map(|p| p.account_id.clone()).collect();
    match context.hunt.participant(&user_id) {
        // A realm host that doesn't play its hunt resolves as itself only; its writes stay refused
        None if context.hunt.host_participates == Some(false) && user_id == *host => {
            let mut with_host = roster;
            with_host.push(user_id.clone());
            Some(RuntimeIdentity {
                user_id,
                roster: with_host,
            })
        }
        None => None,
        Some(p) if user_id != *host && p.admission_state != AdmissionState::Admitted => None,
        Some(_) => Some(RuntimeIdentity { user_id, roster }),
    }
}

/// A hub generation, and when its present accounts were read.
type PresenceEntries = HashMap<(String, i64), (i64, Vec<String>)>;

/// The present accounts of a hub generation, cached a second per process; a write here invalidates the hub.
#[derive(Debug, Default)]
pub struct HubPresenceCache {
    entries: Mutex<PresenceEntries>,
}

impl HubPresenceCache {
    const TTL_MS: i64 = 1000;
    const MAX_ENTRIES: usize = 512;

    pub fn invalidate(&self, session_id: &str) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(session, _), _| session != session_id);
    }

    fn get(&self, key: &(String, i64), now: i64) -> Option<Vec<String>> {
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries
            .get(key)
            .filter(|(at, _)| now - at < Self::TTL_MS)
            .map(|(_, value)| value.clone())
    }

    fn put(&self, key: (String, i64), now: i64, value: Vec<String>) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if entries.len() >= Self::MAX_ENTRIES {
            entries.retain(|_, (at, _)| now - *at < Self::TTL_MS);
        }
        entries.insert(key, (now, value));
    }
}

/// Minting, checking and revoking host capabilities.
pub struct HostCapabilities {
    hunts: HuntRepository,
    hub: RealmHubRepository,
    node: NodeId,
    presence: HubPresenceCache,
}

impl HostCapabilities {
    pub fn new(db: RealmsDb) -> Self {
        Self {
            hunts: HuntRepository::new(db.clone()),
            hub: RealmHubRepository::new(db.clone()),
            node: db.node().clone(),
            presence: HubPresenceCache::default(),
        }
    }

    pub fn hub_presence(&self) -> &HubPresenceCache {
        &self.presence
    }

    /// A new capability for the session's host: the secret goes to the host's launcher once, the digest stays.
    #[allow(clippy::too_many_arguments)]
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
        let actor = rifts_seam::Actor::Account(AccountId(hunt.host_account_id.clone()));
        self.hunts.create_host_capability(&actor, &record).await?;
        Ok((secret, record))
    }

    /// `RequireHostCapability(scope)` for a route's `:sessionId`.
    pub async fn check(
        &self,
        header: Option<&str>,
        session_id: &str,
        scope: CapabilityScope,
        now: UnixMillis,
    ) -> Result<HostCapabilityRecord, RealmsError> {
        let Some(digest) = header.and_then(hash_capability_secret) else {
            return Err(invalid("A valid hunt host capability is required."));
        };
        let capability = self
            .hunts
            .find_host_capability_by_digest(&digest)
            .await
            .map_err(|_| invalid("The hunt host capability could not be verified."))?;
        match capability {
            Some(c)
                if c.revoked_at.is_none()
                    && c.expires_at > now
                    && c.session_id == session_id
                    && c.scopes.contains(&scope) =>
            {
                Ok(c)
            }
            _ => Err(invalid("The hunt host capability is invalid or expired.")),
        }
    }

    /// The accounts ADMITTED in a hub generation, through the second-long cache.
    pub async fn hub_present_accounts(&self, session_id: &str, generation: i64, now: UnixMillis) -> Result<Vec<String>, RealmsError> {
        let key = (session_id.to_owned(), generation);
        if let Some(present) = self.presence.get(&key, now.0) {
            return Ok(present);
        }
        let present: Vec<String> = self
            .hub
            .list_by_state(session_id, generation, &[PresenceState::Admitted], 256)
            .await?
            .into_iter()
            .map(|row| row.account_id)
            .collect();
        self.presence.put(key, now.0, present.clone());
        Ok(present)
    }

    /// `ResolveActiveHostRuntimeContext`: a `GAME_SERVER_RUNTIME` capability while its exact generation is live.
    pub async fn resolve_runtime(&self, header: Option<&str>, now: UnixMillis) -> Result<Option<RuntimeContext>, RealmsError> {
        let Some(digest) = header.and_then(hash_capability_secret) else {
            return Ok(None);
        };
        let Some(capability) = self.hunts.find_host_capability_by_digest(&digest).await? else {
            return Ok(None);
        };
        if capability.revoked_at.is_some()
            || capability.expires_at <= now
            || !capability.scopes.contains(&CapabilityScope::GameServerRuntime)
        {
            return Ok(None);
        }
        let Some(hunt) = self.hunts.find_hunt(&capability.session_id).await? else {
            return Ok(None);
        };
        if hunt.generation != capability.generation
            || hunt.host_account_id != capability.host_account_id
            || hunt.host_steam_id != capability.host_steam_id
        {
            return Ok(None);
        }
        if hunt.is_realm_hub() {
            if !crate::hub::HUB_RUNTIME_STATES.contains(&hunt.state) {
                return Ok(None);
            }
            let present = self.hub_present_accounts(&hunt.session_id, hunt.generation, now).await?;
            return Ok(Some(RuntimeContext {
                capability,
                hunt,
                present: Some(present),
            }));
        }
        if !HUNT_RUNTIME_STATES.contains(&hunt.state) {
            return Ok(None);
        }
        Ok(Some(RuntimeContext {
            capability,
            hunt,
            present: None,
        }))
    }

    /// The seam's `RuntimeAuth` of a resolved runtime request (session ids in RF2's spelling).
    pub fn runtime_auth(&self, context: &RuntimeContext, identity: &RuntimeIdentity) -> Option<RuntimeAuth> {
        Some(RuntimeAuth {
            capability_id: context.capability.capability_id.clone(),
            session: seam_session_id(&context.capability.session_id, &self.node)?,
            generation: u64::try_from(context.capability.generation).ok()?,
            host_account: AccountId(context.capability.host_account_id.clone()),
            scopes: context.capability.scopes.clone(),
            roster: identity.roster.iter().cloned().map(AccountId).collect(),
        })
    }

    pub async fn revoke(&self, session_id: &str, generation: i64, at: UnixMillis) -> Result<(), RealmsError> {
        self.hunts
            .revoke_generation(&rifts_seam::Actor::System("realms"), session_id, generation, at)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_is_43_characters_of_base64url_and_hashes_to_its_digest() {
        let (secret, digest) = generate_capability_secret();
        assert!(is_opaque_capability(secret.expose()));
        assert_eq!(hash_capability_secret(secret.expose()).as_deref(), Some(digest.as_str()));
        assert_eq!(digest.len(), 64);
        assert_eq!(hash_capability_secret("short"), None);
        assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
    }
}
