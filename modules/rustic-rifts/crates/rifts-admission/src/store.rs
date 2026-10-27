//! Admission's tables (RF1, section 7), shaped for `store` (see `rifts_seam`'s store notes), and the in-memory
//! repository the service runs on until the `store`-backed one (DF7, RF5).

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use rifts_seam::{AccountId, Actor, Component, PeerId, StoreError, TableSpec, Unimplemented, UnixMillis};

pub const TABLES: &[TableSpec] = &[
    TableSpec {
        name: "rifts_peer_links",
        node_collection: "p2pSteamLinks",
        owner: Component::Admission,
        keys: "account; peer unique",
        expires_by: None,
    },
    TableSpec {
        name: "rifts_admission_tickets",
        node_collection: "p2pAdmissionTickets",
        owner: Component::Admission,
        keys: "jti digest; session + generation + account",
        expires_by: None,
    },
    TableSpec {
        name: "rifts_peer_admissions",
        node_collection: "p2pPeerAdmissions",
        owner: Component::Admission,
        keys: "admission; active admission key unique",
        expires_by: None,
    },
];

/// `p2pSteamLinks.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkStatus {
    Verified,
    Revoked,
}

/// `p2pSteamLinks`: an account's transport peer id. ICE derives it from the account (`ICE_ACCOUNT_DERIVED`,
/// [`crate::derive_ice_peer_id`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerLinkRecord {
    pub account: AccountId,
    pub peer: PeerId,
    pub status: LinkStatus,
    pub revision: u64,
    pub verified_at: UnixMillis,
    pub verification_method: String,
}

/// `p2pAdmissionTickets.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TicketStatus {
    Issued,
    Consumed,
    Revoked,
}

/// `p2pAdmissionTickets`. Only the jti's digest is kept, never the ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketRecord {
    pub jti_digest: String,
    pub key_id: String,
    /// Node's session id text.
    pub session: String,
    pub generation: u64,
    pub account: AccountId,
    pub joiner_peer: PeerId,
    pub host_peer: PeerId,
    pub bridge_protocol: u16,
    pub virtual_port: u16,
    pub issued_at: UnixMillis,
    pub expires_at: UnixMillis,
    pub status: TicketStatus,
    pub consumed_at: Option<UnixMillis>,
    pub consumer_bridge_instance_id: Option<String>,
    pub consumer_request_id: Option<String>,
}

/// `consumeAdmissionTicket`'s input: the claims that must match the stored ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumeRequest {
    pub jti_digest: String,
    pub session: String,
    pub generation: u64,
    pub account: AccountId,
    pub joiner_peer: PeerId,
    pub host_peer: PeerId,
    pub bridge_instance_id: String,
    pub request_id: String,
    pub now: UnixMillis,
}

/// `P2PTicketConsumeResult`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsumeOutcome {
    Consumed(TicketRecord),
    /// The same transport asking again for the same request.
    Idempotent(TicketRecord),
    Expired,
    Replay,
    Invalid,
}

/// `p2pPeerAdmissions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerAdmissionRecord {
    /// The jti's digest: each reconnect gets its own record.
    pub admission_id: String,
    /// `<session>:<generation>:<peer>`, unique while connected, so a peer has one live connection per session.
    pub active_key: Option<String>,
    pub session: String,
    pub generation: u64,
    pub account: AccountId,
    pub peer: PeerId,
    pub bridge_instance_id: String,
    pub connection_handle: String,
    pub admitted_at: UnixMillis,
    pub disconnected_at: Option<UnixMillis>,
    pub superseded_at: Option<UnixMillis>,
}

/// `disconnectPeer`'s input: the admission and the host generation that reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisconnectRequest {
    pub admission_id: String,
    pub session: String,
    pub generation: u64,
    pub bridge_instance_id: String,
    pub at: UnixMillis,
}

/// The writes return whether they took effect: `false` is Node's duplicate key.
pub trait AdmissionStore: Send + Sync {
    fn peer_link(&self, account: &AccountId) -> Result<Option<PeerLinkRecord>, StoreError>;
    fn put_peer_link(&self, actor: &Actor, link: &PeerLinkRecord) -> Result<(), StoreError>;
    fn insert_ticket(&self, actor: &Actor, ticket: &TicketRecord) -> Result<bool, StoreError>;
    /// Compare-and-set on the ticket's status, with Node's outcomes.
    fn consume_ticket(&self, actor: &Actor, request: &ConsumeRequest) -> Result<ConsumeOutcome, StoreError>;
    fn insert_admission(&self, actor: &Actor, admission: &PeerAdmissionRecord) -> Result<bool, StoreError>;
    fn find_admission(&self, admission_id: &str) -> Result<Option<PeerAdmissionRecord>, StoreError>;
    /// Ends the admission holding `active_key`, if any (a hub member back before its old link was reported closed).
    fn supersede_active(&self, actor: &Actor, active_key: &str, at: UnixMillis) -> Result<bool, StoreError>;
    fn disconnect(&self, actor: &Actor, request: &DisconnectRequest) -> Result<bool, StoreError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StubAdmissionStore;

fn unimplemented<T>(operation: &'static str) -> Result<T, StoreError> {
    Err(Unimplemented::new(Component::Admission, operation).into())
}

impl AdmissionStore for StubAdmissionStore {
    fn peer_link(&self, _: &AccountId) -> Result<Option<PeerLinkRecord>, StoreError> {
        unimplemented("store.peer_link")
    }
    fn put_peer_link(&self, _: &Actor, _: &PeerLinkRecord) -> Result<(), StoreError> {
        unimplemented("store.put_peer_link")
    }
    fn insert_ticket(&self, _: &Actor, _: &TicketRecord) -> Result<bool, StoreError> {
        unimplemented("store.insert_ticket")
    }
    fn consume_ticket(&self, _: &Actor, _: &ConsumeRequest) -> Result<ConsumeOutcome, StoreError> {
        unimplemented("store.consume_ticket")
    }
    fn insert_admission(&self, _: &Actor, _: &PeerAdmissionRecord) -> Result<bool, StoreError> {
        unimplemented("store.insert_admission")
    }
    fn find_admission(&self, _: &str) -> Result<Option<PeerAdmissionRecord>, StoreError> {
        unimplemented("store.find_admission")
    }
    fn supersede_active(&self, _: &Actor, _: &str, _: UnixMillis) -> Result<bool, StoreError> {
        unimplemented("store.supersede_active")
    }
    fn disconnect(&self, _: &Actor, _: &DisconnectRequest) -> Result<bool, StoreError> {
        unimplemented("store.disconnect")
    }
}

#[derive(Debug, Default)]
struct Tables {
    links: HashMap<AccountId, PeerLinkRecord>,
    tickets: HashMap<String, TicketRecord>,
    admissions: HashMap<String, PeerAdmissionRecord>,
}

/// `InMemoryP2PHuntRepository`'s admission half: one process, no persistence.
#[derive(Debug, Default)]
pub struct MemoryAdmissionStore {
    tables: Mutex<Tables>,
}

impl MemoryAdmissionStore {
    fn tables(&self) -> std::sync::MutexGuard<'_, Tables> {
        self.tables.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn ticket(&self, jti_digest: &str) -> Option<TicketRecord> {
        self.tables().tickets.get(jti_digest).cloned()
    }
}

impl AdmissionStore for MemoryAdmissionStore {
    fn peer_link(&self, account: &AccountId) -> Result<Option<PeerLinkRecord>, StoreError> {
        Ok(self.tables().links.get(account).cloned())
    }

    fn put_peer_link(&self, _: &Actor, link: &PeerLinkRecord) -> Result<(), StoreError> {
        self.tables().links.insert(link.account.clone(), link.clone());
        Ok(())
    }

    fn insert_ticket(&self, _: &Actor, ticket: &TicketRecord) -> Result<bool, StoreError> {
        let mut tables = self.tables();
        if tables.tickets.contains_key(&ticket.jti_digest) {
            return Ok(false);
        }
        tables.tickets.insert(ticket.jti_digest.clone(), ticket.clone());
        Ok(true)
    }

    fn consume_ticket(&self, _: &Actor, request: &ConsumeRequest) -> Result<ConsumeOutcome, StoreError> {
        let mut tables = self.tables();
        let Some(ticket) = tables.tickets.get_mut(&request.jti_digest) else {
            return Ok(ConsumeOutcome::Invalid);
        };
        let matches = ticket.session == request.session
            && ticket.generation == request.generation
            && ticket.joiner_peer == request.joiner_peer;
        if ticket.status == TicketStatus::Issued
            && matches
            && ticket.account == request.account
            && ticket.host_peer == request.host_peer
            && ticket.expires_at > request.now
        {
            ticket.status = TicketStatus::Consumed;
            ticket.consumed_at = Some(request.now);
            ticket.consumer_bridge_instance_id = Some(request.bridge_instance_id.clone());
            ticket.consumer_request_id = Some(request.request_id.clone());
            return Ok(ConsumeOutcome::Consumed(ticket.clone()));
        }
        if ticket.status == TicketStatus::Consumed {
            let same = ticket.consumer_bridge_instance_id.as_deref() == Some(request.bridge_instance_id.as_str())
                && ticket.consumer_request_id.as_deref() == Some(request.request_id.as_str())
                && matches;
            return Ok(if same {
                ConsumeOutcome::Idempotent(ticket.clone())
            } else {
                ConsumeOutcome::Replay
            });
        }
        Ok(if ticket.expires_at <= request.now {
            ConsumeOutcome::Expired
        } else {
            ConsumeOutcome::Invalid
        })
    }

    fn insert_admission(&self, _: &Actor, admission: &PeerAdmissionRecord) -> Result<bool, StoreError> {
        let mut tables = self.tables();
        let active_taken = admission.active_key.as_ref().is_some_and(|key| {
            tables
                .admissions
                .values()
                .any(|entry| entry.active_key.as_ref() == Some(key))
        });
        if tables.admissions.contains_key(&admission.admission_id) || active_taken {
            return Ok(false);
        }
        tables
            .admissions
            .insert(admission.admission_id.clone(), admission.clone());
        Ok(true)
    }

    fn find_admission(&self, admission_id: &str) -> Result<Option<PeerAdmissionRecord>, StoreError> {
        Ok(self.tables().admissions.get(admission_id).cloned())
    }

    fn supersede_active(&self, _: &Actor, active_key: &str, at: UnixMillis) -> Result<bool, StoreError> {
        let mut tables = self.tables();
        let Some(admission) = tables
            .admissions
            .values_mut()
            .find(|entry| entry.active_key.as_deref() == Some(active_key))
        else {
            return Ok(false);
        };
        admission.disconnected_at = Some(at);
        admission.superseded_at = Some(at);
        admission.active_key = None;
        Ok(true)
    }

    fn disconnect(&self, _: &Actor, request: &DisconnectRequest) -> Result<bool, StoreError> {
        let mut tables = self.tables();
        let Some(admission) = tables.admissions.get_mut(&request.admission_id) else {
            return Ok(false);
        };
        if admission.session != request.session
            || admission.generation != request.generation
            || admission.bridge_instance_id != request.bridge_instance_id
            || admission.disconnected_at.is_some()
        {
            return Ok(false);
        }
        admission.disconnected_at = Some(request.at);
        admission.active_key = None;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor() -> Actor {
        Actor::System("test")
    }

    fn ticket() -> TicketRecord {
        TicketRecord {
            jti_digest: "d".repeat(64),
            key_id: "k1".to_owned(),
            session: "s".to_owned(),
            generation: 7,
            account: AccountId("a".to_owned()),
            joiner_peer: PeerId(2),
            host_peer: PeerId(1),
            bridge_protocol: 1,
            virtual_port: 1,
            issued_at: UnixMillis(0),
            expires_at: UnixMillis(180_000),
            status: TicketStatus::Issued,
            consumed_at: None,
            consumer_bridge_instance_id: None,
            consumer_request_id: None,
        }
    }

    fn request(now: i64, bridge: &str, request_id: &str) -> ConsumeRequest {
        ConsumeRequest {
            jti_digest: "d".repeat(64),
            session: "s".to_owned(),
            generation: 7,
            account: AccountId("a".to_owned()),
            joiner_peer: PeerId(2),
            host_peer: PeerId(1),
            bridge_instance_id: bridge.to_owned(),
            request_id: request_id.to_owned(),
            now: UnixMillis(now),
        }
    }

    #[test]
    fn stub_store_refuses() {
        let error = StubAdmissionStore.peer_link(&AccountId("a".to_owned())).unwrap_err();
        assert_eq!(
            error,
            StoreError::Unimplemented(Unimplemented::new(Component::Admission, "store.peer_link"))
        );
    }

    #[test]
    fn tables_are_admissions() {
        assert!(
            TABLES
                .iter()
                .all(|table| table.owner == Component::Admission && table.name.starts_with("rifts_"))
        );
    }

    #[test]
    fn a_ticket_is_consumed_once_then_idempotent_for_its_request_and_a_replay_for_anyone_else() {
        let store = MemoryAdmissionStore::default();
        assert!(store.insert_ticket(&actor(), &ticket()).unwrap());
        assert!(!store.insert_ticket(&actor(), &ticket()).unwrap());
        assert!(matches!(
            store
                .consume_ticket(&actor(), &request(1_000, "bridge-1", "r1"))
                .unwrap(),
            ConsumeOutcome::Consumed(_)
        ));
        assert!(matches!(
            store
                .consume_ticket(&actor(), &request(2_000, "bridge-1", "r1"))
                .unwrap(),
            ConsumeOutcome::Idempotent(_)
        ));
        assert_eq!(
            store
                .consume_ticket(&actor(), &request(2_000, "bridge-1", "r2"))
                .unwrap(),
            ConsumeOutcome::Replay
        );
        assert_eq!(
            store
                .consume_ticket(&actor(), &request(2_000, "bridge-2", "r1"))
                .unwrap(),
            ConsumeOutcome::Replay
        );
        let mut unknown = request(1_000, "b", "r");
        unknown.jti_digest = "e".repeat(64);
        assert_eq!(
            store.consume_ticket(&actor(), &unknown).unwrap(),
            ConsumeOutcome::Invalid
        );
    }

    #[test]
    fn an_expired_or_mismatched_ticket_is_never_consumed() {
        let store = MemoryAdmissionStore::default();
        store.insert_ticket(&actor(), &ticket()).unwrap();
        assert_eq!(
            store.consume_ticket(&actor(), &request(180_000, "b", "r")).unwrap(),
            ConsumeOutcome::Expired
        );
        let mut other_host = request(1_000, "b", "r");
        other_host.host_peer = PeerId(9);
        assert_eq!(
            store.consume_ticket(&actor(), &other_host).unwrap(),
            ConsumeOutcome::Invalid
        );
        let revoked = TicketRecord {
            status: TicketStatus::Revoked,
            jti_digest: "f".repeat(64),
            ..ticket()
        };
        store.insert_ticket(&actor(), &revoked).unwrap();
        let mut for_revoked = request(1_000, "b", "r");
        for_revoked.jti_digest = "f".repeat(64);
        assert_eq!(
            store.consume_ticket(&actor(), &for_revoked).unwrap(),
            ConsumeOutcome::Invalid
        );
        assert_eq!(store.ticket(&"d".repeat(64)).unwrap().status, TicketStatus::Issued);
    }

    #[test]
    fn one_live_admission_per_peer_and_disconnects_by_the_reporting_host_only() {
        let store = MemoryAdmissionStore::default();
        let admission = PeerAdmissionRecord {
            admission_id: "1".repeat(64),
            active_key: Some("s:7:2".to_owned()),
            session: "s".to_owned(),
            generation: 7,
            account: AccountId("a".to_owned()),
            peer: PeerId(2),
            bridge_instance_id: "bridge-1".to_owned(),
            connection_handle: "h".to_owned(),
            admitted_at: UnixMillis(0),
            disconnected_at: None,
            superseded_at: None,
        };
        assert!(store.insert_admission(&actor(), &admission).unwrap());
        let second = PeerAdmissionRecord {
            admission_id: "2".repeat(64),
            ..admission.clone()
        };
        assert!(!store.insert_admission(&actor(), &second).unwrap());
        let wrong_host = DisconnectRequest {
            admission_id: admission.admission_id.clone(),
            session: "s".to_owned(),
            generation: 7,
            bridge_instance_id: "bridge-2".to_owned(),
            at: UnixMillis(5),
        };
        assert!(!store.disconnect(&actor(), &wrong_host).unwrap());
        let right_host = DisconnectRequest {
            bridge_instance_id: "bridge-1".to_owned(),
            ..wrong_host
        };
        assert!(store.disconnect(&actor(), &right_host).unwrap());
        assert!(!store.disconnect(&actor(), &right_host).unwrap());
        assert!(store.insert_admission(&actor(), &second).unwrap());
        assert!(store.supersede_active(&actor(), "s:7:2", UnixMillis(9)).unwrap());
        let superseded = store.find_admission(&second.admission_id).unwrap().unwrap();
        assert_eq!(
            (superseded.active_key, superseded.superseded_at),
            (None, Some(UnixMillis(9)))
        );
    }
}
