//! Admission (RF4): who may join a hosted hunt or hub.
//!
//! Ported from `p2p/security/admissionTickets.ts`, `p2p/controllers/p2pAdmission.ts`, `realmHubAdmission.ts`, the
//! join half of `p2pLauncher.ts`, `p2p/security/transportPeerIds.ts` and `scripts/p2p/generate-admission-key.mjs`
//! (RF1). The ticket is the transport's MPT2 ([`ticket`], `src/admission_ticket.cpp`), signed with a P-256 key the
//! backend holds ([`keys`]); the host's transport checks it against `p2p-admission-public.xy`, reads it from the
//! joiner's MPC1 control frame ([`control`]) and consumes it through the backend ([`Admission`]).

pub mod control;
pub mod keys;
mod routes;
mod service;
mod store;
pub mod ticket;

pub use keys::{KeyError, P256KeyGenerator, SECRET_SETTINGS, write_key_set};
pub use routes::{AdmissionRoutes, CapabilityResolver, ROUTES, stub_routes};
pub use service::{
    Admission, AdmissionHunt, AdmissionHunts, ConsumeTicket, ConsumedTicket, HostCapability, HubPeer, HubPeers,
    IssuedTicket, JOIN_TICKET_LIFETIME_SECONDS, JoinBootstrap, MAXIMUM_PEER_LIST, refused,
};
pub use store::{
    AdmissionStore, ConsumeOutcome, ConsumeRequest, DisconnectRequest, LinkStatus, MemoryAdmissionStore,
    PeerAdmissionRecord, PeerLinkRecord, StubAdmissionStore, TABLES, TicketRecord, TicketStatus,
};
pub use ticket::{Es256Tickets, Jti, MintedTicket, SessionUuid, TicketBindings, TicketClaims, TicketError};

use rifts_seam::{
    AccountId, BoxFuture, Component, PeerId, Refusal, Secret, SessionId, StoreError, Unimplemented, UnixMillis, stub,
};
use sha2::Digest;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AdmissionError {
    #[error(transparent)]
    Unimplemented(#[from] Unimplemented),
    /// A launcher error code (`P2P_*`, `REALM_*`); isles answers it in the launcher envelope with its status.
    #[error(transparent)]
    Refused(#[from] Refusal),
    #[error(transparent)]
    Ticket(#[from] TicketError),
    #[error(transparent)]
    Key(#[from] KeyError),
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Node's duplicate key: a ticket or a live admission that already exists.
    #[error("the record already exists")]
    Conflict,
}

/// `P2P_ADMISSION_KEY_ID`: a safe id of at most 32 characters, written into every ticket.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyId(pub String);

/// The uncompressed P-256 point the transport reads from `p2p-admission-public.xy` (Guard role `ADMISSION_KEY`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicXy(pub [u8; 64]);

/// What the key generator makes: the env lines and the game folder's public key file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedKeyPair {
    pub key_id: KeyId,
    /// `P2P_ADMISSION_SIGNING_PRIVKEY_B64`, base64 PEM (PKCS#8); goes to `secrets.env`.
    pub signing_key: Secret,
    /// `P2P_ADMISSION_VERIFY_PUBKEY_B64`, base64 PEM (SPKI).
    pub verify_key: String,
    pub public_xy: PublicXy,
}

/// `generate-admission-key.mjs`, which the 1.14.7 module exposes as a command ([`write_key_set`] writes its folder).
pub trait KeyGenerator: Send + Sync {
    fn generate(&self, key_id: KeyId) -> Result<GeneratedKeyPair, AdmissionError>;
}

/// Signs and checks MPT2 tickets. Sync: it is local crypto, no I/O.
pub trait TicketAuthority: Send + Sync {
    fn key_id(&self) -> KeyId;
    /// `IssueAdmissionTicket`: the authority sets the key, the jti and the times; `lifetime_seconds` is the caller's
    /// (15 to 180), else `P2P_ADMISSION_TICKET_TTL_SECONDS`.
    fn mint(
        &self,
        bindings: TicketBindings,
        now: UnixMillis,
        lifetime_seconds: Option<u32>,
    ) -> Result<MintedTicket, AdmissionError>;
    /// `VerifyAdmissionTicket`: signature, key id and expiry; single use is [`AdmissionService::consume_ticket`]'s.
    fn verify(&self, ticket: &str, now: UnixMillis) -> Result<TicketClaims, AdmissionError>;
}

fn seconds(now: UnixMillis) -> u64 {
    u64::try_from(now.0.div_euclid(1000)).unwrap_or_default()
}

impl TicketAuthority for Es256Tickets {
    fn key_id(&self) -> KeyId {
        self.key_id.clone()
    }
    fn mint(
        &self,
        bindings: TicketBindings,
        now: UnixMillis,
        lifetime_seconds: Option<u32>,
    ) -> Result<MintedTicket, AdmissionError> {
        Ok(Es256Tickets::mint(self, bindings, seconds(now), lifetime_seconds)?)
    }
    fn verify(&self, ticket: &str, now: UnixMillis) -> Result<TicketClaims, AdmissionError> {
        Ok(Es256Tickets::verify(self, ticket, seconds(now))?)
    }
}

/// `DeriveIcePeerId`: an ICE peer's 64-bit id from its account, nonzero with the top bit clear
/// (`ICE_ACCOUNT_DERIVED` peer links).
pub fn derive_ice_peer_id(account: &AccountId) -> PeerId {
    let digest = sha2::Sha256::digest(format!("mysticparadox-ice-peer-v1\n{account}"));
    let mut head = [0u8; 8];
    head.copy_from_slice(&digest[..8]);
    let value = u64::from_be_bytes(head) & 0x7fff_ffff_ffff_ffff;
    PeerId(if value == 0 { 1 } else { value })
}

/// The admission routes' operations; the route layer parses bodies and answers each result's JSON.
pub trait AdmissionService: Send + Sync {
    /// `IssueSelfAdmissionTicket`.
    fn issue_self_ticket(
        &self,
        account: AccountId,
        session: SessionId,
    ) -> BoxFuture<'_, Result<IssuedTicket, AdmissionError>>;
    /// `CreateJoinBootstrap`.
    fn create_join_bootstrap(
        &self,
        account: AccountId,
        session: SessionId,
    ) -> BoxFuture<'_, Result<JoinBootstrap, AdmissionError>>;
    /// `ConsumeAdmissionTicket`: single use, against the session's roster.
    fn consume_ticket(
        &self,
        capability: HostCapability,
        request: ConsumeTicket,
    ) -> BoxFuture<'_, Result<ConsumedTicket, AdmissionError>>;
    /// `RecordPeerDisconnected`: `{ disconnected }`. `admission_id` is the route's 64-hex parameter.
    fn record_peer_disconnected(
        &self,
        capability: HostCapability,
        admission_id: String,
        disconnected_at: Option<UnixMillis>,
    ) -> BoxFuture<'_, Result<bool, AdmissionError>>;
    /// `ListHubPeers`: a hub host's poll for peers and their pair keys; `generation` is the body's when it is a safe
    /// integer.
    fn list_hub_peers(
        &self,
        account: AccountId,
        session: SessionId,
        generation: Option<i64>,
    ) -> BoxFuture<'_, Result<HubPeers, AdmissionError>>;
}

/// Every operation answers [`Unimplemented`]: what isles links before it wires [`Admission`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Stub;

const COMPONENT: Component = Component::Admission;

impl KeyGenerator for Stub {
    fn generate(&self, _: KeyId) -> Result<GeneratedKeyPair, AdmissionError> {
        Err(Unimplemented::new(COMPONENT, "generate").into())
    }
}

impl TicketAuthority for Stub {
    fn key_id(&self) -> KeyId {
        KeyId(String::new())
    }
    fn mint(&self, _: TicketBindings, _: UnixMillis, _: Option<u32>) -> Result<MintedTicket, AdmissionError> {
        Err(Unimplemented::new(COMPONENT, "mint").into())
    }
    fn verify(&self, _: &str, _: UnixMillis) -> Result<TicketClaims, AdmissionError> {
        Err(Unimplemented::new(COMPONENT, "verify").into())
    }
}

impl AdmissionService for Stub {
    fn issue_self_ticket(&self, _: AccountId, _: SessionId) -> BoxFuture<'_, Result<IssuedTicket, AdmissionError>> {
        stub(COMPONENT, "issue_self_ticket")
    }
    fn create_join_bootstrap(
        &self,
        _: AccountId,
        _: SessionId,
    ) -> BoxFuture<'_, Result<JoinBootstrap, AdmissionError>> {
        stub(COMPONENT, "create_join_bootstrap")
    }
    fn consume_ticket(
        &self,
        _: HostCapability,
        _: ConsumeTicket,
    ) -> BoxFuture<'_, Result<ConsumedTicket, AdmissionError>> {
        stub(COMPONENT, "consume_ticket")
    }
    fn record_peer_disconnected(
        &self,
        _: HostCapability,
        _: String,
        _: Option<UnixMillis>,
    ) -> BoxFuture<'_, Result<bool, AdmissionError>> {
        stub(COMPONENT, "record_peer_disconnected")
    }
    fn list_hub_peers(
        &self,
        _: AccountId,
        _: SessionId,
        _: Option<i64>,
    ) -> BoxFuture<'_, Result<HubPeers, AdmissionError>> {
        stub(COMPONENT, "list_hub_peers")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rifts_seam::poll_once;

    fn unimplemented(operation: &'static str) -> AdmissionError {
        Unimplemented::new(COMPONENT, operation).into()
    }

    fn session() -> SessionId {
        "01ARZ3NDEKTSV4RRFFQ69G5FAV@local".parse().unwrap()
    }

    #[test]
    fn stub_refuses_keys_and_tickets() {
        assert_eq!(Stub.generate(KeyId("k1".to_owned())), Err(unimplemented("generate")));
        assert_eq!(Stub.verify("MPT2.x", UnixMillis(0)), Err(unimplemented("verify")));
    }

    #[test]
    fn stub_refuses_the_service() {
        let account = AccountId("a".to_owned());
        assert_eq!(
            poll_once(Stub.issue_self_ticket(account.clone(), session())).unwrap(),
            Err(unimplemented("issue_self_ticket"))
        );
        assert_eq!(
            poll_once(Stub.list_hub_peers(account, session(), Some(1))).unwrap(),
            Err(unimplemented("list_hub_peers"))
        );
    }

    // transportPeerIds.ts: SHA-256 of the label and the account, the first 8 bytes, the top bit cleared
    #[test]
    fn ice_peer_ids_are_derived_from_the_account() {
        let peer = derive_ice_peer_id(&AccountId("account-1".to_owned()));
        assert_ne!(peer.0, 0);
        assert_eq!(peer.0 >> 63, 0);
        assert_eq!(peer, derive_ice_peer_id(&AccountId("account-1".to_owned())));
        assert_ne!(peer, derive_ice_peer_id(&AccountId("account-2".to_owned())));
    }
}
