// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Who may join a hosted hunt or hub. A joiner's launcher gets a signed ticket; the joiner's transport hands it to the
//! host's transport in a control frame; the host's transport checks the signature with the public key and asks the
//! backend to consume the ticket once.

pub mod control;
pub mod keys;
mod routes;
mod service;
mod store;
pub mod ticket;

use rifts_seam::{
    AccountId, BoxFuture, Component, PeerId, Refusal, Secret, SessionId, StoreError, Unimplemented, UnixMillis, stub,
};
use sha2::{Digest as _, Sha256};

pub use crate::keys::{KeyError, P256KeyGenerator, SECRET_SETTINGS, write_key_set};
pub use crate::routes::{AdmissionRoutes, CapabilityResolver, ROUTES, stub_routes};
pub use crate::service::{
    Admission, AdmissionHunt, AdmissionHunts, ConsumeTicket, ConsumedTicket, HostCapability, HubPeer, HubPeers,
    IssuedTicket, JOIN_TICKET_LIFETIME_SECONDS, JoinBootstrap, MAXIMUM_PEER_LIST, refused,
};
pub use crate::store::{
    AdmissionStore, ConsumeOutcome, ConsumeRequest, DisconnectRequest, LinkStatus, MemoryAdmissionStore,
    PeerAdmissionRecord, PeerLinkRecord, StubAdmissionStore, TABLES, TicketRecord, TicketStatus,
};
pub use crate::ticket::{Es256Tickets, Jti, MintedTicket, SessionUuid, TicketBindings, TicketClaims, TicketError};

// Umbrella spec D3: the Rustic label. Every peer id derived from it changes with it.
const ICE_PEER_LABEL: &str = "rustic-ice-peer-v1";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AdmissionError {
    #[error(transparent)]
    Unimplemented(#[from] Unimplemented),
    #[error(transparent)]
    Refused(#[from] Refusal),
    #[error(transparent)]
    Ticket(#[from] TicketError),
    #[error(transparent)]
    Key(#[from] KeyError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("the record already exists")]
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyId(pub String);

/// The public point without SEC1's `0x04` tag: X then Y, 32 bytes each, big-endian.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicXy(pub [u8; 64]);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedKeyPair {
    pub key_id: KeyId,
    /// Standard base64 of the PKCS#8 PEM.
    pub signing_key: Secret,
    /// Standard base64 of the SPKI PEM.
    pub verify_key: String,
    pub public_xy: PublicXy,
}

impl GeneratedKeyPair {
    /// The three settings lines a server's env file takes.
    pub fn env_lines(&self) -> Secret {
        Secret::new(format!(
            "{}={}\n{}={}\n{}={}\n",
            keys::KEY_ID_SETTING,
            self.key_id.0,
            keys::SIGNING_KEY_SETTING,
            self.signing_key.expose(),
            keys::VERIFY_KEY_SETTING,
            self.verify_key,
        ))
    }
}

pub trait KeyGenerator: Send + Sync {
    fn generate(&self, key_id: KeyId) -> Result<GeneratedKeyPair, AdmissionError>;
}

pub trait TicketAuthority: Send + Sync {
    fn key_id(&self) -> KeyId;
    fn mint(
        &self,
        bindings: TicketBindings,
        now: UnixMillis,
        lifetime_seconds: Option<u32>,
    ) -> Result<MintedTicket, AdmissionError>;
    fn verify(&self, ticket: &str, now: UnixMillis) -> Result<TicketClaims, AdmissionError>;
}

/// The transport's peer id for an account: never 0, top bit clear.
pub fn derive_ice_peer_id(account: &AccountId) -> PeerId {
    let digest = Sha256::digest(format!("{ICE_PEER_LABEL}\n{}", account.0).as_bytes());
    let mut first = [0_u8; 8];
    first.copy_from_slice(&digest[..8]);
    let id = u64::from_be_bytes(first) & 0x7fff_ffff_ffff_ffff;
    PeerId(id.max(1))
}

pub trait AdmissionService: Send + Sync {
    fn issue_self_ticket(
        &self,
        account: AccountId,
        session: SessionId,
    ) -> BoxFuture<'_, Result<IssuedTicket, AdmissionError>>;
    fn create_join_bootstrap(
        &self,
        account: AccountId,
        session: SessionId,
    ) -> BoxFuture<'_, Result<JoinBootstrap, AdmissionError>>;
    fn consume_ticket(
        &self,
        capability: HostCapability,
        request: ConsumeTicket,
    ) -> BoxFuture<'_, Result<ConsumedTicket, AdmissionError>>;
    fn record_peer_disconnected(
        &self,
        capability: HostCapability,
        admission_id: String,
        disconnected_at: Option<UnixMillis>,
    ) -> BoxFuture<'_, Result<bool, AdmissionError>>;
    fn list_hub_peers(
        &self,
        account: AccountId,
        session: SessionId,
        generation: Option<i64>,
    ) -> BoxFuture<'_, Result<HubPeers, AdmissionError>>;
}

/// Answers every operation as not written yet.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stub;

fn unimplemented(operation: &'static str) -> AdmissionError {
    AdmissionError::Unimplemented(Unimplemented::new(Component::Admission, operation))
}

impl KeyGenerator for Stub {
    fn generate(&self, _: KeyId) -> Result<GeneratedKeyPair, AdmissionError> {
        Err(unimplemented("generate"))
    }
}

impl TicketAuthority for Stub {
    fn key_id(&self) -> KeyId {
        KeyId(String::new())
    }

    fn mint(&self, _: TicketBindings, _: UnixMillis, _: Option<u32>) -> Result<MintedTicket, AdmissionError> {
        Err(unimplemented("mint"))
    }

    fn verify(&self, _: &str, _: UnixMillis) -> Result<TicketClaims, AdmissionError> {
        Err(unimplemented("verify"))
    }
}

impl AdmissionService for Stub {
    fn issue_self_ticket(&self, _: AccountId, _: SessionId) -> BoxFuture<'_, Result<IssuedTicket, AdmissionError>> {
        stub(Component::Admission, "issue_self_ticket")
    }

    fn create_join_bootstrap(
        &self,
        _: AccountId,
        _: SessionId,
    ) -> BoxFuture<'_, Result<JoinBootstrap, AdmissionError>> {
        stub(Component::Admission, "create_join_bootstrap")
    }

    fn consume_ticket(
        &self,
        _: HostCapability,
        _: ConsumeTicket,
    ) -> BoxFuture<'_, Result<ConsumedTicket, AdmissionError>> {
        stub(Component::Admission, "consume_ticket")
    }

    fn record_peer_disconnected(
        &self,
        _: HostCapability,
        _: String,
        _: Option<UnixMillis>,
    ) -> BoxFuture<'_, Result<bool, AdmissionError>> {
        stub(Component::Admission, "record_peer_disconnected")
    }

    fn list_hub_peers(
        &self,
        _: AccountId,
        _: SessionId,
        _: Option<i64>,
    ) -> BoxFuture<'_, Result<HubPeers, AdmissionError>> {
        stub(Component::Admission, "list_hub_peers")
    }
}

#[cfg(test)]
mod tests {
    use rifts_seam::{GlobalId, NodeId, Ulid, poll_once};

    use super::*;

    #[test]
    fn ice_peer_ids() {
        let one = derive_ice_peer_id(&AccountId("account-1".to_owned()));
        let two = derive_ice_peer_id(&AccountId("account-2".to_owned()));
        assert_ne!(one.0, 0);
        assert_eq!(one.0 >> 63, 0);
        assert_eq!(one, derive_ice_peer_id(&AccountId("account-1".to_owned())));
        assert_ne!(one, two);
        let digest = Sha256::digest(b"rustic-ice-peer-v1\naccount-1");
        let mut first = [0_u8; 8];
        first.copy_from_slice(&digest[..8]);
        assert_eq!(one.0, u64::from_be_bytes(first) & (u64::MAX >> 1));
    }

    #[test]
    fn stubs_answer_unimplemented() {
        let account = AccountId("account-1".to_owned());
        let session = SessionId(GlobalId {
            ulid: Ulid(1),
            node: NodeId::new("node").unwrap(),
        });
        let capability = HostCapability {
            session: session.clone(),
            wire_session: "w".to_owned(),
            generation: 1,
            host_account: account.clone(),
            host_peer: PeerId(1),
            bridge_instance_id: "b".to_owned(),
        };
        let consume = ConsumeTicket {
            ticket: Secret::new("t".to_owned()),
            authenticated_peer: "1".to_owned(),
            bridge_instance_id: "b".to_owned(),
            connection_handle: "h".to_owned(),
            request_id: "r".to_owned(),
        };
        assert_eq!(Stub.generate(KeyId("k".to_owned())), Err(unimplemented("generate")));
        assert_eq!(TicketAuthority::key_id(&Stub), KeyId(String::new()));
        assert_eq!(
            TicketAuthority::verify(&Stub, "t", UnixMillis(0)),
            Err(unimplemented("verify"))
        );
        assert_eq!(
            poll_once(Stub.issue_self_ticket(account.clone(), session.clone())).unwrap(),
            Err(unimplemented("issue_self_ticket"))
        );
        assert_eq!(
            poll_once(Stub.create_join_bootstrap(account.clone(), session.clone())).unwrap(),
            Err(unimplemented("create_join_bootstrap"))
        );
        assert_eq!(
            poll_once(Stub.consume_ticket(capability.clone(), consume)).unwrap(),
            Err(unimplemented("consume_ticket"))
        );
        assert_eq!(
            poll_once(Stub.record_peer_disconnected(capability, "x".to_owned(), None)).unwrap(),
            Err(unimplemented("record_peer_disconnected"))
        );
        assert_eq!(
            poll_once(Stub.list_hub_peers(account, session, Some(1))).unwrap(),
            Err(unimplemented("list_hub_peers"))
        );
        assert_eq!(AdmissionError::Conflict.to_string(), "the record already exists");
    }
}
