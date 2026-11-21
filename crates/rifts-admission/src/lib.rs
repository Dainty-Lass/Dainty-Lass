// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Who may join a hosted hunt or hub. A joiner's launcher gets a signed ticket; the joiner's transport hands it to the
//! host's transport in a control frame; the host's transport checks the signature with the public key and asks the
//! backend to consume the ticket once.

use std::error::Error as StdError;
use std::fmt;

use rifts_seam as seam;
use seam::{AccountId, PeerId, Secret, SessionId, UnixMillis};
use sha2::Digest as _;

pub mod control;
pub mod keys;
pub mod ticket;

mod routes;
mod service;
mod store;

pub use crate::{
    keys::{KeyError, P256KeyGenerator, SECRET_SETTINGS, write_key_set},
    routes::{AdmissionRoutes, CapabilityResolver, ROUTES, stub_routes},
    service::{
        Admission, AdmissionHunt, AdmissionHunts, ConsumeTicket, ConsumedTicket, HostCapability, HubPeer, HubPeers,
        IssuedTicket, JOIN_TICKET_LIFETIME_SECONDS, JoinBootstrap, MAXIMUM_PEER_LIST, refused,
    },
    store::{
        AdmissionStore, ConsumeOutcome, ConsumeRequest, DisconnectRequest, LinkStatus, MemoryAdmissionStore,
        PeerAdmissionRecord, PeerLinkRecord, StubAdmissionStore, TABLES, TicketRecord, TicketStatus,
    },
    ticket::{Es256Tickets, Jti, MintedTicket, SessionUuid, TicketBindings, TicketClaims, TicketError},
};

/// What every service operation resolves to.
type Answer<'a, T> = seam::BoxFuture<'a, Result<T, AdmissionError>>;

#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum AdmissionError {
    Unimplemented(seam::Unimplemented),
    Refused(seam::Refusal),
    Ticket(TicketError),
    Key(KeyError),
    Store(seam::StoreError),
    Conflict,
}

impl AdmissionError {
    /// The wrapped error every variant but `Conflict` shows through unchanged.
    fn wrapped(&self) -> Option<&(dyn StdError + 'static)> {
        Some(match self {
            Self::Unimplemented(inner) => inner,
            Self::Refused(inner) => inner,
            Self::Ticket(inner) => inner,
            Self::Key(inner) => inner,
            Self::Store(inner) => inner,
            Self::Conflict => return None,
        })
    }
}

impl fmt::Display for AdmissionError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.wrapped() {
            Some(inner) => fmt::Display::fmt(inner, out),
            None => out.write_str("the record already exists"),
        }
    }
}

impl StdError for AdmissionError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.wrapped()?.source()
    }
}

macro_rules! admission_error_from {
    ($($variant:ident <- $inner:ty),* $(,)?) => {$(
        impl From<$inner> for AdmissionError {
            fn from(inner: $inner) -> Self {
                Self::$variant(inner)
            }
        }
    )*};
}

admission_error_from! {
    Unimplemented <- seam::Unimplemented,
    Refused <- seam::Refusal,
    Ticket <- TicketError,
    Key <- KeyError,
    Store <- seam::StoreError,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct KeyId(pub String);

/// The public point without SEC1's `0x04` tag: X then Y, 32 bytes each, big-endian.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PublicXy(pub [u8; 64]);

#[derive(Clone, Debug, Eq, PartialEq)]
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
        let lines = [
            (keys::KEY_ID_SETTING, self.key_id.0.as_str()),
            (keys::SIGNING_KEY_SETTING, self.signing_key.expose()),
            (keys::VERIFY_KEY_SETTING, self.verify_key.as_str()),
        ];
        Secret::new(lines.iter().map(|(name, value)| format!("{name}={value}\n")).collect())
    }
}

pub trait KeyGenerator: Send + Sync {
    fn generate(&self, id: KeyId) -> Result<GeneratedKeyPair, AdmissionError>;
}

pub trait TicketAuthority: Send + Sync {
    fn key_id(&self) -> KeyId;
    fn mint(
        &self,
        to: TicketBindings,
        at: UnixMillis,
        lifetime_seconds: Option<u32>,
    ) -> Result<MintedTicket, AdmissionError>;
    fn verify(&self, text: &str, at: UnixMillis) -> Result<TicketClaims, AdmissionError>;
}

// Umbrella spec D3: the Rustic label. Every peer id derived from it changes with it.
const ICE_PEER_LABEL: &str = "rustic-ice-peer-v1";

/// The transport's peer id for an account: never 0, top bit clear.
pub fn derive_ice_peer_id(account: &AccountId) -> PeerId {
    let digest = sha2::Sha256::new()
        .chain_update(ICE_PEER_LABEL)
        .chain_update(b"\n")
        .chain_update(&account.0)
        .finalize();
    let leading = digest.first_chunk::<8>().copied().unwrap_or_default();
    PeerId((u64::from_be_bytes(leading) & (u64::MAX >> 1)).max(1))
}

pub trait AdmissionService: Send + Sync {
    fn issue_self_ticket(&self, who: AccountId, hunt: SessionId) -> Answer<'_, IssuedTicket>;
    fn create_join_bootstrap(&self, who: AccountId, hunt: SessionId) -> Answer<'_, JoinBootstrap>;
    fn consume_ticket(&self, host: HostCapability, request: ConsumeTicket) -> Answer<'_, ConsumedTicket>;
    fn record_peer_disconnected(
        &self,
        host: HostCapability,
        admission: String,
        at: Option<UnixMillis>,
    ) -> Answer<'_, bool>;
    fn list_hub_peers(&self, who: AccountId, hub: SessionId, generation: Option<i64>) -> Answer<'_, HubPeers>;
}

/// Answers every operation as not written yet.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stub;

fn unimplemented(operation: &'static str) -> AdmissionError {
    seam::Unimplemented::new(seam::Component::Admission, operation).into()
}

fn not_yet<'a, T: Send + 'static>(operation: &'static str) -> Answer<'a, T> {
    seam::stub(seam::Component::Admission, operation)
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
    fn issue_self_ticket(&self, _: AccountId, _: SessionId) -> Answer<'_, IssuedTicket> {
        not_yet("issue_self_ticket")
    }

    fn create_join_bootstrap(&self, _: AccountId, _: SessionId) -> Answer<'_, JoinBootstrap> {
        not_yet("create_join_bootstrap")
    }

    fn consume_ticket(&self, _: HostCapability, _: ConsumeTicket) -> Answer<'_, ConsumedTicket> {
        not_yet("consume_ticket")
    }

    fn record_peer_disconnected(&self, _: HostCapability, _: String, _: Option<UnixMillis>) -> Answer<'_, bool> {
        not_yet("record_peer_disconnected")
    }

    fn list_hub_peers(&self, _: AccountId, _: SessionId, _: Option<i64>) -> Answer<'_, HubPeers> {
        not_yet("list_hub_peers")
    }
}

#[cfg(test)]
mod tests {
    use seam::{GlobalId, NodeId, Ulid, poll_once};
    use sha2::Sha256;

    use super::*;

    fn account(name: &str) -> AccountId {
        AccountId(name.to_owned())
    }

    #[test]
    fn ice_peer_ids() {
        let [one, two] = ["account-1", "account-2"].map(|name| derive_ice_peer_id(&account(name)));
        assert_ne!(one.0, 0);
        assert_eq!(one.0 >> 63, 0);
        assert_eq!(derive_ice_peer_id(&account("account-1")), one);
        assert_ne!(two, one);
        let digest = Sha256::digest(b"rustic-ice-peer-v1\naccount-1");
        let expected = u64::from_be_bytes(digest[..8].try_into().unwrap()) & 0x7fff_ffff_ffff_ffff;
        assert_eq!(one.0, expected);
    }

    #[test]
    fn stubs_answer_unimplemented() {
        let who = account("account-1");
        let hunt = SessionId(GlobalId {
            node: NodeId::new("node").unwrap(),
            ulid: Ulid(1),
        });
        let host = HostCapability {
            host_account: who.clone(),
            host_peer: PeerId(1),
            bridge_instance_id: "b".to_owned(),
            session: hunt.clone(),
            wire_session: "w".to_owned(),
            generation: 1,
        };
        let request = ConsumeTicket {
            connection_handle: "h".to_owned(),
            request_id: "r".to_owned(),
            ticket: Secret::new("t".to_owned()),
            authenticated_peer: "1".to_owned(),
            bridge_instance_id: "b".to_owned(),
        };
        assert_eq!(Stub.generate(KeyId("k".to_owned())), Err(unimplemented("generate")));
        assert_eq!(TicketAuthority::key_id(&Stub), KeyId(String::new()));
        assert_eq!(
            TicketAuthority::verify(&Stub, "t", UnixMillis(0)),
            Err(unimplemented("verify"))
        );

        let answers = [
            (
                "issue_self_ticket",
                poll_once(Stub.issue_self_ticket(who.clone(), hunt.clone()))
                    .unwrap()
                    .map(drop),
            ),
            (
                "create_join_bootstrap",
                poll_once(Stub.create_join_bootstrap(who.clone(), hunt.clone()))
                    .unwrap()
                    .map(drop),
            ),
            (
                "consume_ticket",
                poll_once(Stub.consume_ticket(host.clone(), request)).unwrap().map(drop),
            ),
            (
                "record_peer_disconnected",
                poll_once(Stub.record_peer_disconnected(host, "x".to_owned(), None))
                    .unwrap()
                    .map(drop),
            ),
            (
                "list_hub_peers",
                poll_once(Stub.list_hub_peers(who, hunt, Some(1))).unwrap().map(drop),
            ),
        ];
        for (operation, answer) in answers {
            assert_eq!(answer, Err(unimplemented(operation)), "{operation}");
        }
        assert_eq!(AdmissionError::Conflict.to_string(), "the record already exists");
    }
}
