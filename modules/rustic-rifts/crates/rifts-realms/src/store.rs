//! Realms' tables (RF1, section 7), shaped for `store` (see `rifts_seam`'s store notes).
//!
//! Node's arrays inside documents (`realms.members`, `bans`, `joinRequests`, `playerInvites`, `hubRoomIds`, a hunt's
//! `roster`) become child tables, so both engines query them with plain joins. Records carry the keys and the
//! fields the traits need; RF5 completes them from the Node contracts.

use rifts_seam::{
    AccountId, Actor, CapabilityScope, Component, HubKind, PartyId, RealmId, SessionId, StoreError, TableSpec,
    Unimplemented, UnixMillis,
};

const fn table(name: &'static str, node_collection: &'static str, keys: &'static str) -> TableSpec {
    TableSpec {
        name,
        node_collection,
        owner: Component::Realms,
        keys,
        expires_by: None,
    }
}

const fn expiring(
    name: &'static str,
    node_collection: &'static str,
    keys: &'static str,
    by: &'static str,
) -> TableSpec {
    TableSpec {
        name,
        node_collection,
        owner: Component::Realms,
        keys,
        expires_by: Some(by),
    }
}

pub const TABLES: &[TableSpec] = &[
    table("rifts_realms", "realms", "realm; name unique; host unique"),
    table("rifts_realm_members", "realms.members", "realm + account"),
    table("rifts_realm_bans", "realms.bans", "realm + account"),
    table("rifts_realm_join_requests", "realms.joinRequests", "realm + account"),
    table("rifts_realm_player_invites", "realms.playerInvites", "realm + account"),
    table("rifts_realm_hub_rooms", "realms.hubRoomIds", "room unique"),
    expiring(
        "rifts_realm_invites",
        "realmInvites",
        "code; realm + created_at",
        "purge_at",
    ),
    expiring(
        "rifts_realm_presence",
        "realmPresence",
        "account; realm + expires_at",
        "expires_at",
    ),
    table(
        "rifts_hunt_sessions",
        "p2pHuntSessions",
        "session; active party key unique; state + deadlines",
    ),
    table("rifts_hunt_roster", "p2pHuntSessions.roster", "session + account"),
    table(
        "rifts_hunt_transitions",
        "p2pHuntTransitions",
        "session + at + state_version",
    ),
    table(
        "rifts_host_capabilities",
        "p2pHostCapabilities",
        "secret digest unique; session + generation",
    ),
    expiring("rifts_host_presences", "p2pHostPresences", "account", "expires_at"),
    table("rifts_host_offers", "p2pHostOffers", "session + generation + rank"),
    table("rifts_host_reliability", "p2pHostReliability", "account"),
    table(
        "rifts_participant_observations",
        "p2pParticipantObservations",
        "session + generation + observed_at",
    ),
    table("rifts_control_sequences", "p2pControlSequences", "account"),
    expiring(
        "rifts_control_messages",
        "p2pControlMessages",
        "account + sequence unique",
        "ttl_at",
    ),
    expiring(
        "rifts_hub_presence",
        "realmHubPresence",
        "session / account / realm + state",
        "purge_at",
    ),
    expiring("rifts_hub_gates", "realmHubGates", "session + generation", "purge_at"),
];

/// `joinPolicy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinPolicy {
    InviteOnly,
    Public,
    Solo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmRecord {
    pub realm: RealmId,
    pub name: String,
    pub host: AccountId,
    /// 32 random bytes in hex, made at claim time and pinned by joiners; compared, never used to sign.
    pub host_public_key: String,
    pub join_policy: JoinPolicy,
    pub listed: bool,
    pub created_at: UnixMillis,
    pub online_until: Option<UnixMillis>,
    /// Optimistic concurrency, as Node's `version`.
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmInviteRecord {
    pub code: String,
    pub realm: RealmId,
    pub created_by: AccountId,
    pub created_at: UnixMillis,
    pub expires_at: UnixMillis,
    pub max_uses: Option<u32>,
    pub uses: u32,
    pub revoked_at: Option<UnixMillis>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmPresenceRecord {
    pub account: AccountId,
    pub realm: RealmId,
    pub bound_at: UnixMillis,
    pub last_seen_at: UnixMillis,
    pub expires_at: UnixMillis,
}

/// `P2PHuntState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HuntState {
    Requested,
    HostSelecting,
    HostOffered,
    Starting,
    HostReady,
    Admitting,
    Running,
    Draining,
    Suspect,
    Fenced,
    Aborting,
    Recovering,
    Completed,
    Aborted,
}

/// `sessionKind`: a hunt, or a realm hub of one kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    Hunt,
    RealmHub(HubKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HuntSessionRecord {
    pub session: SessionId,
    pub generation: u64,
    pub kind: SessionKind,
    pub state: HuntState,
    pub state_version: u64,
    pub party: Option<PartyId>,
    pub realm: Option<RealmId>,
    pub host: Option<AccountId>,
    pub hunt_definition_id: Option<String>,
    pub lease_expires_at: Option<UnixMillis>,
    pub created_at: UnixMillis,
    pub expires_at: UnixMillis,
}

/// `p2pHostCapabilities`. Only the secret's SHA-256 is kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostCapabilityRecord {
    pub capability_id: String,
    pub secret_digest: String,
    pub session: SessionId,
    pub generation: u64,
    pub host: AccountId,
    pub scopes: Vec<CapabilityScope>,
    pub issued_at: UnixMillis,
    pub expires_at: UnixMillis,
    pub revoked_at: Option<UnixMillis>,
    pub last_sequence: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferStatus {
    Offered,
    Accepted,
    Declined,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostOfferRecord {
    pub offer_id: String,
    pub session: SessionId,
    pub generation: u64,
    pub account: AccountId,
    pub rank: u32,
    pub score: i64,
    pub status: OfferStatus,
    pub offered_at: UnixMillis,
    pub expires_at: UnixMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlMessageRecord {
    pub message_id: String,
    pub account: AccountId,
    pub sequence: u64,
    /// One of Node's 12 `messageType`s (`ICE_SIGNAL`, `HOST_OFFER`...).
    pub kind: String,
    pub session: Option<SessionId>,
    pub generation: Option<u64>,
    pub payload: rifts_seam::Json,
    pub created_at: UnixMillis,
    pub expires_at: UnixMillis,
    pub acknowledged_at: Option<UnixMillis>,
}

/// The realm directory's tables.
pub trait RealmStore: Send + Sync {
    fn realm(&self, realm: &RealmId) -> Result<Option<RealmRecord>, StoreError>;
    fn realm_by_name(&self, name: &str) -> Result<Option<RealmRecord>, StoreError>;
    fn realm_of_host(&self, host: &AccountId) -> Result<Option<RealmRecord>, StoreError>;
    fn insert_realm(&self, actor: &Actor, realm: &RealmRecord) -> Result<(), StoreError>;
    /// Writes only when the stored version is `expected_version`.
    fn update_realm(&self, actor: &Actor, realm: &RealmRecord, expected_version: u64) -> Result<bool, StoreError>;
    fn is_member(&self, realm: &RealmId, account: &AccountId) -> Result<bool, StoreError>;
    fn is_banned(&self, realm: &RealmId, account: &AccountId) -> Result<bool, StoreError>;
    fn invite(&self, code: &str) -> Result<Option<RealmInviteRecord>, StoreError>;
    fn put_invite(&self, actor: &Actor, invite: &RealmInviteRecord) -> Result<(), StoreError>;
    fn presence(&self, account: &AccountId) -> Result<Option<RealmPresenceRecord>, StoreError>;
    fn put_presence(&self, actor: &Actor, presence: &RealmPresenceRecord) -> Result<(), StoreError>;
    fn clear_presence(&self, actor: &Actor, account: &AccountId) -> Result<(), StoreError>;
}

/// Hunt and hub sessions, capabilities, host selection and the control channel.
pub trait HuntStore: Send + Sync {
    fn session(&self, session: &SessionId) -> Result<Option<HuntSessionRecord>, StoreError>;
    fn insert_session(&self, actor: &Actor, session: &HuntSessionRecord) -> Result<(), StoreError>;
    /// A state change as one compare-and-set on `state_version`, with its transition row.
    fn transition(
        &self,
        actor: &Actor,
        session: &SessionId,
        expected_version: u64,
        to: HuntState,
        reason: &str,
    ) -> Result<bool, StoreError>;
    fn extend_lease(&self, actor: &Actor, session: &SessionId, until: UnixMillis) -> Result<bool, StoreError>;
    fn capability_by_digest(&self, secret_digest: &str) -> Result<Option<HostCapabilityRecord>, StoreError>;
    fn insert_capability(&self, actor: &Actor, capability: &HostCapabilityRecord) -> Result<(), StoreError>;
    fn put_offer(&self, actor: &Actor, offer: &HostOfferRecord) -> Result<(), StoreError>;
    fn offer(&self, offer_id: &str) -> Result<Option<HostOfferRecord>, StoreError>;
    /// Appends with the account's next sequence (`p2pControlSequences`).
    fn push_control(&self, actor: &Actor, message: &ControlMessageRecord) -> Result<u64, StoreError>;
    fn control_after(
        &self,
        account: &AccountId,
        after_sequence: u64,
        limit: u32,
    ) -> Result<Vec<ControlMessageRecord>, StoreError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StubRealmsStore;

fn unimplemented<T>(operation: &'static str) -> Result<T, StoreError> {
    Err(Unimplemented::new(Component::Realms, operation).into())
}

impl RealmStore for StubRealmsStore {
    fn realm(&self, _: &RealmId) -> Result<Option<RealmRecord>, StoreError> {
        unimplemented("store.realm")
    }
    fn realm_by_name(&self, _: &str) -> Result<Option<RealmRecord>, StoreError> {
        unimplemented("store.realm_by_name")
    }
    fn realm_of_host(&self, _: &AccountId) -> Result<Option<RealmRecord>, StoreError> {
        unimplemented("store.realm_of_host")
    }
    fn insert_realm(&self, _: &Actor, _: &RealmRecord) -> Result<(), StoreError> {
        unimplemented("store.insert_realm")
    }
    fn update_realm(&self, _: &Actor, _: &RealmRecord, _: u64) -> Result<bool, StoreError> {
        unimplemented("store.update_realm")
    }
    fn is_member(&self, _: &RealmId, _: &AccountId) -> Result<bool, StoreError> {
        unimplemented("store.is_member")
    }
    fn is_banned(&self, _: &RealmId, _: &AccountId) -> Result<bool, StoreError> {
        unimplemented("store.is_banned")
    }
    fn invite(&self, _: &str) -> Result<Option<RealmInviteRecord>, StoreError> {
        unimplemented("store.invite")
    }
    fn put_invite(&self, _: &Actor, _: &RealmInviteRecord) -> Result<(), StoreError> {
        unimplemented("store.put_invite")
    }
    fn presence(&self, _: &AccountId) -> Result<Option<RealmPresenceRecord>, StoreError> {
        unimplemented("store.presence")
    }
    fn put_presence(&self, _: &Actor, _: &RealmPresenceRecord) -> Result<(), StoreError> {
        unimplemented("store.put_presence")
    }
    fn clear_presence(&self, _: &Actor, _: &AccountId) -> Result<(), StoreError> {
        unimplemented("store.clear_presence")
    }
}

impl HuntStore for StubRealmsStore {
    fn session(&self, _: &SessionId) -> Result<Option<HuntSessionRecord>, StoreError> {
        unimplemented("store.session")
    }
    fn insert_session(&self, _: &Actor, _: &HuntSessionRecord) -> Result<(), StoreError> {
        unimplemented("store.insert_session")
    }
    fn transition(&self, _: &Actor, _: &SessionId, _: u64, _: HuntState, _: &str) -> Result<bool, StoreError> {
        unimplemented("store.transition")
    }
    fn extend_lease(&self, _: &Actor, _: &SessionId, _: UnixMillis) -> Result<bool, StoreError> {
        unimplemented("store.extend_lease")
    }
    fn capability_by_digest(&self, _: &str) -> Result<Option<HostCapabilityRecord>, StoreError> {
        unimplemented("store.capability_by_digest")
    }
    fn insert_capability(&self, _: &Actor, _: &HostCapabilityRecord) -> Result<(), StoreError> {
        unimplemented("store.insert_capability")
    }
    fn put_offer(&self, _: &Actor, _: &HostOfferRecord) -> Result<(), StoreError> {
        unimplemented("store.put_offer")
    }
    fn offer(&self, _: &str) -> Result<Option<HostOfferRecord>, StoreError> {
        unimplemented("store.offer")
    }
    fn push_control(&self, _: &Actor, _: &ControlMessageRecord) -> Result<u64, StoreError> {
        unimplemented("store.push_control")
    }
    fn control_after(&self, _: &AccountId, _: u64, _: u32) -> Result<Vec<ControlMessageRecord>, StoreError> {
        unimplemented("store.control_after")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_store_refuses() {
        let error = RealmStore::realm_by_name(&StubRealmsStore, "x").unwrap_err();
        assert_eq!(
            error,
            StoreError::Unimplemented(Unimplemented::new(Component::Realms, "store.realm_by_name"))
        );
        assert!(HuntStore::offer(&StubRealmsStore, "o").is_err());
    }

    #[test]
    fn every_realm_collection_has_a_table() {
        for collection in [
            "realms",
            "realmInvites",
            "realmPresence",
            "p2pHuntSessions",
            "p2pHostCapabilities",
            "p2pControlSequences",
            "p2pControlMessages",
            "p2pHostPresences",
            "p2pHostOffers",
            "p2pHostReliability",
            "p2pParticipantObservations",
            "p2pHuntTransitions",
            "realmHubPresence",
            "realmHubGates",
        ] {
            assert!(
                TABLES.iter().any(|table| table.node_collection == collection),
                "{collection}"
            );
        }
    }
}
