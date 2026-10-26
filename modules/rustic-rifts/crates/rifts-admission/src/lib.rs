//! Admission (RF4): who may join a hosted hunt or hub.
//!
//! Ported from `p2p/security/admissionTickets.ts`, `p2p/controllers/p2pAdmission.ts`, `realmHubAdmission.ts`, the
//! join half of `p2pLauncher.ts` and `scripts/p2p/generate-admission-key.mjs` (RF1). The ticket is the transport's
//! MPT2 (`src/admission_ticket.cpp`), signed with a P-256 key the backend holds; the host's transport checks it
//! against `p2p-admission-public.xy` and consumes it through the backend.

mod routes;
mod store;

pub use routes::{ROUTES, stub_routes};
pub use store::{AdmissionStore, PeerAdmissionRecord, PeerLinkRecord, StubAdmissionStore, TABLES, TicketRecord};

use rifts_seam::{
    AccountId, AdmissionId, BoxFuture, Component, PeerId, RuntimeAuth, Secret, SessionId, Unimplemented, UnixMillis,
    stub,
};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AdmissionError {
    #[error(transparent)]
    Unimplemented(#[from] Unimplemented),
}

/// `P2P_ADMISSION_KEY_ID`: a safe id of at most 32 characters, written into every ticket.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyId(pub String);

/// The uncompressed P-256 point the transport reads from `p2p-admission-public.xy` (Guard role `ADMISSION_KEY`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicXy(pub [u8; 64]);

/// What the key generator writes: the env lines and the game folder's public key file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedKeyPair {
    pub key_id: KeyId,
    /// `P2P_ADMISSION_SIGNING_PRIVKEY_B64`, base64 PEM; goes to `secrets.env` (RF4).
    pub signing_key: Secret,
    /// `P2P_ADMISSION_VERIFY_PUBKEY_B64`, base64 PEM.
    pub verify_key: String,
    pub public_xy: PublicXy,
}

/// `generate-admission-key.mjs`, which RF4 makes a host kit command.
pub trait KeyGenerator: Send + Sync {
    fn generate(&self, key_id: KeyId) -> Result<GeneratedKeyPair, AdmissionError>;
}

/// The claims of one MPT2 ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketClaims {
    pub key_id: KeyId,
    pub session: SessionId,
    pub generation: u64,
    pub account: AccountId,
    pub joiner_peer: PeerId,
    pub host_peer: PeerId,
    pub bridge_protocol: u8,
    pub virtual_port: u16,
    pub issued_at: UnixMillis,
    /// `P2P_ADMISSION_TICKET_TTL_SECONDS` (75) after `issued_at`.
    pub expires_at: UnixMillis,
    pub jti: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MintedTicket {
    /// The ticket text written to `ticket.secret`.
    pub text: Secret,
    pub claims: TicketClaims,
}

/// Signs and checks MPT2 tickets. Sync: it is local crypto, no I/O.
pub trait TicketAuthority: Send + Sync {
    /// `issued_at`, `expires_at` and `jti` in `claims` are the authority's to set.
    fn mint(&self, claims: TicketClaims, now: UnixMillis) -> Result<MintedTicket, AdmissionError>;
    /// Signature, key id and expiry; single use is [`AdmissionService::consume_ticket`]'s.
    fn verify(&self, ticket: &str, now: UnixMillis) -> Result<TicketClaims, AdmissionError>;
}

/// `POST .../join-bootstrap`: what a joiner's launcher needs to start its transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinBootstrap {
    pub ticket: MintedTicket,
    pub host_account: AccountId,
}

/// `POST .../tickets/consume`, from the host's transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumeTicket {
    pub ticket: Secret,
    pub bridge_instance_id: String,
    pub connection_handle: String,
}

/// One peer a hub host should add (`AddPeerH`), with the pair key RF4 derives in rifts-relay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubPeer {
    pub account: AccountId,
    pub peer: PeerId,
    /// 43 base64url characters.
    pub pair_key: Secret,
}

/// The admission routes' operations; the route layer parses and answers as Node.
pub trait AdmissionService: Send + Sync {
    /// `IssueSelfAdmissionTicket`.
    fn issue_self_ticket(
        &self,
        account: AccountId,
        session: SessionId,
    ) -> BoxFuture<'_, Result<MintedTicket, AdmissionError>>;
    /// `CreateJoinBootstrap`.
    fn create_join_bootstrap(
        &self,
        account: AccountId,
        session: SessionId,
    ) -> BoxFuture<'_, Result<JoinBootstrap, AdmissionError>>;
    /// `ConsumeAdmissionTicket`: single use, against the session's roster.
    fn consume_ticket(
        &self,
        runtime: RuntimeAuth,
        request: ConsumeTicket,
    ) -> BoxFuture<'_, Result<PeerAdmissionRecord, AdmissionError>>;
    /// `RecordPeerDisconnected`.
    fn record_peer_disconnected(
        &self,
        runtime: RuntimeAuth,
        admission: AdmissionId,
    ) -> BoxFuture<'_, Result<(), AdmissionError>>;
    /// `ListHubPeers`: a hub host's poll for peers and their pair keys.
    fn list_hub_peers(
        &self,
        account: AccountId,
        session: SessionId,
    ) -> BoxFuture<'_, Result<Vec<HubPeer>, AdmissionError>>;
}

/// Every operation answers [`Unimplemented`] until RF4.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stub;

const COMPONENT: Component = Component::Admission;

impl KeyGenerator for Stub {
    fn generate(&self, _: KeyId) -> Result<GeneratedKeyPair, AdmissionError> {
        Err(Unimplemented::new(COMPONENT, "generate").into())
    }
}

impl TicketAuthority for Stub {
    fn mint(&self, _: TicketClaims, _: UnixMillis) -> Result<MintedTicket, AdmissionError> {
        Err(Unimplemented::new(COMPONENT, "mint").into())
    }
    fn verify(&self, _: &str, _: UnixMillis) -> Result<TicketClaims, AdmissionError> {
        Err(Unimplemented::new(COMPONENT, "verify").into())
    }
}

impl AdmissionService for Stub {
    fn issue_self_ticket(&self, _: AccountId, _: SessionId) -> BoxFuture<'_, Result<MintedTicket, AdmissionError>> {
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
        _: RuntimeAuth,
        _: ConsumeTicket,
    ) -> BoxFuture<'_, Result<PeerAdmissionRecord, AdmissionError>> {
        stub(COMPONENT, "consume_ticket")
    }
    fn record_peer_disconnected(&self, _: RuntimeAuth, _: AdmissionId) -> BoxFuture<'_, Result<(), AdmissionError>> {
        stub(COMPONENT, "record_peer_disconnected")
    }
    fn list_hub_peers(&self, _: AccountId, _: SessionId) -> BoxFuture<'_, Result<Vec<HubPeer>, AdmissionError>> {
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
            poll_once(Stub.list_hub_peers(account, session())).unwrap(),
            Err(unimplemented("list_hub_peers"))
        );
    }
}
