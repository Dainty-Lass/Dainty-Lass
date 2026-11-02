// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! What admission keeps: an account's verified peer link, each issued ticket (by its jti digest, never its text) and
//! each peer admission. The in-memory repository is the reference for the semantics a database store must keep.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkStatus {
    Verified,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerLinkRecord {
    pub account: AccountId,
    pub peer: PeerId,
    pub status: LinkStatus,
    pub revision: u64,
    pub verified_at: UnixMillis,
    pub verification_method: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TicketStatus {
    Issued,
    Consumed,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketRecord {
    pub jti_digest: String,
    pub key_id: String,
    /// The session's wire text.
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsumeOutcome {
    Consumed(TicketRecord),
    /// The same bridge retried the same request.
    Idempotent(TicketRecord),
    Expired,
    Replay,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerAdmissionRecord {
    pub admission_id: String,
    /// `<wire session>:<generation>:<peer>` while live; at most one live admission holds it.
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisconnectRequest {
    pub admission_id: String,
    pub session: String,
    pub generation: u64,
    pub bridge_instance_id: String,
    pub at: UnixMillis,
}

pub trait AdmissionStore: Send + Sync {
    fn peer_link(&self, account: &AccountId) -> Result<Option<PeerLinkRecord>, StoreError>;
    fn put_peer_link(&self, actor: &Actor, link: &PeerLinkRecord) -> Result<(), StoreError>;
    /// `false` when the digest is already stored.
    fn insert_ticket(&self, actor: &Actor, ticket: &TicketRecord) -> Result<bool, StoreError>;
    fn consume_ticket(&self, actor: &Actor, request: &ConsumeRequest) -> Result<ConsumeOutcome, StoreError>;
    /// `false` when the id or the live active key is already held.
    fn insert_admission(&self, actor: &Actor, admission: &PeerAdmissionRecord) -> Result<bool, StoreError>;
    fn find_admission(&self, admission_id: &str) -> Result<Option<PeerAdmissionRecord>, StoreError>;
    fn supersede_active(&self, actor: &Actor, active_key: &str, at: UnixMillis) -> Result<bool, StoreError>;
    fn disconnect(&self, actor: &Actor, request: &DisconnectRequest) -> Result<bool, StoreError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StubAdmissionStore;

fn missing(operation: &'static str) -> StoreError {
    StoreError::Unimplemented(Unimplemented::new(Component::Admission, operation))
}

impl AdmissionStore for StubAdmissionStore {
    fn peer_link(&self, _: &AccountId) -> Result<Option<PeerLinkRecord>, StoreError> {
        Err(missing("store.peer_link"))
    }

    fn put_peer_link(&self, _: &Actor, _: &PeerLinkRecord) -> Result<(), StoreError> {
        Err(missing("store.put_peer_link"))
    }

    fn insert_ticket(&self, _: &Actor, _: &TicketRecord) -> Result<bool, StoreError> {
        Err(missing("store.insert_ticket"))
    }

    fn consume_ticket(&self, _: &Actor, _: &ConsumeRequest) -> Result<ConsumeOutcome, StoreError> {
        Err(missing("store.consume_ticket"))
    }

    fn insert_admission(&self, _: &Actor, _: &PeerAdmissionRecord) -> Result<bool, StoreError> {
        Err(missing("store.insert_admission"))
    }

    fn find_admission(&self, _: &str) -> Result<Option<PeerAdmissionRecord>, StoreError> {
        Err(missing("store.find_admission"))
    }

    fn supersede_active(&self, _: &Actor, _: &str, _: UnixMillis) -> Result<bool, StoreError> {
        Err(missing("store.supersede_active"))
    }

    fn disconnect(&self, _: &Actor, _: &DisconnectRequest) -> Result<bool, StoreError> {
        Err(missing("store.disconnect"))
    }
}

#[derive(Debug, Default)]
struct Tables {
    links: HashMap<AccountId, PeerLinkRecord>,
    tickets: HashMap<String, TicketRecord>,
    admissions: HashMap<String, PeerAdmissionRecord>,
}

/// One process's admission rows behind one lock. The actor is not recorded.
#[derive(Debug, Default)]
pub struct MemoryAdmissionStore {
    tables: Mutex<Tables>,
}

impl MemoryAdmissionStore {
    fn lock(&self) -> MutexGuard<'_, Tables> {
        // Every write is a whole-row swap, so a panic elsewhere leaves the maps consistent.
        self.tables.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn ticket(&self, jti_digest: &str) -> Option<TicketRecord> {
        self.lock().tickets.get(jti_digest).cloned()
    }
}

impl AdmissionStore for MemoryAdmissionStore {
    fn peer_link(&self, account: &AccountId) -> Result<Option<PeerLinkRecord>, StoreError> {
        Ok(self.lock().links.get(account).cloned())
    }

    fn put_peer_link(&self, _: &Actor, link: &PeerLinkRecord) -> Result<(), StoreError> {
        self.lock().links.insert(link.account.clone(), link.clone());
        Ok(())
    }

    fn insert_ticket(&self, _: &Actor, ticket: &TicketRecord) -> Result<bool, StoreError> {
        let mut tables = self.lock();
        if tables.tickets.contains_key(&ticket.jti_digest) {
            return Ok(false);
        }
        tables.tickets.insert(ticket.jti_digest.clone(), ticket.clone());
        Ok(true)
    }

    fn consume_ticket(&self, _: &Actor, request: &ConsumeRequest) -> Result<ConsumeOutcome, StoreError> {
        let mut tables = self.lock();
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
            let same_consumer = ticket.consumer_bridge_instance_id.as_deref() == Some(&request.bridge_instance_id)
                && ticket.consumer_request_id.as_deref() == Some(&request.request_id);
            return Ok(if same_consumer && matches {
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
        let mut tables = self.lock();
        let key_held = admission.active_key.as_ref().is_some_and(|key| {
            tables
                .admissions
                .values()
                .any(|stored| stored.active_key.as_ref() == Some(key))
        });
        if key_held || tables.admissions.contains_key(&admission.admission_id) {
            return Ok(false);
        }
        tables
            .admissions
            .insert(admission.admission_id.clone(), admission.clone());
        Ok(true)
    }

    fn find_admission(&self, admission_id: &str) -> Result<Option<PeerAdmissionRecord>, StoreError> {
        Ok(self.lock().admissions.get(admission_id).cloned())
    }

    fn supersede_active(&self, _: &Actor, active_key: &str, at: UnixMillis) -> Result<bool, StoreError> {
        let mut tables = self.lock();
        let holder = tables
            .admissions
            .values_mut()
            .find(|stored| stored.active_key.as_deref() == Some(active_key));
        let Some(holder) = holder else {
            return Ok(false);
        };
        holder.disconnected_at = Some(at);
        holder.superseded_at = Some(at);
        holder.active_key = None;
        Ok(true)
    }

    fn disconnect(&self, _: &Actor, request: &DisconnectRequest) -> Result<bool, StoreError> {
        let mut tables = self.lock();
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

    const DIGEST: &str = "d1";

    fn actor() -> Actor {
        Actor::System("test")
    }

    fn ticket() -> TicketRecord {
        TicketRecord {
            jti_digest: DIGEST.to_owned(),
            key_id: "k".to_owned(),
            session: "wire".to_owned(),
            generation: 3,
            account: AccountId("joiner-1".to_owned()),
            joiner_peer: PeerId(11),
            host_peer: PeerId(22),
            bridge_protocol: 1,
            virtual_port: 3,
            issued_at: UnixMillis(1_000),
            expires_at: UnixMillis(10_000),
            status: TicketStatus::Issued,
            consumed_at: None,
            consumer_bridge_instance_id: None,
            consumer_request_id: None,
        }
    }

    fn consume(request_id: &str, bridge: &str, now: i64) -> ConsumeRequest {
        ConsumeRequest {
            jti_digest: DIGEST.to_owned(),
            session: "wire".to_owned(),
            generation: 3,
            account: AccountId("joiner-1".to_owned()),
            joiner_peer: PeerId(11),
            host_peer: PeerId(22),
            bridge_instance_id: bridge.to_owned(),
            request_id: request_id.to_owned(),
            now: UnixMillis(now),
        }
    }

    fn admission(id: &str, key: &str) -> PeerAdmissionRecord {
        PeerAdmissionRecord {
            admission_id: id.to_owned(),
            active_key: Some(key.to_owned()),
            session: "wire".to_owned(),
            generation: 3,
            account: AccountId("joiner-1".to_owned()),
            peer: PeerId(11),
            bridge_instance_id: "bridge".to_owned(),
            connection_handle: "h1".to_owned(),
            admitted_at: UnixMillis(2_000),
            disconnected_at: None,
            superseded_at: None,
        }
    }

    fn stored() -> MemoryAdmissionStore {
        let store = MemoryAdmissionStore::default();
        assert!(store.insert_ticket(&actor(), &ticket()).unwrap());
        store
    }

    #[test]
    fn consumed_once_then_idempotent_or_replay() {
        let store = stored();
        assert!(!store.insert_ticket(&actor(), &ticket()).unwrap());
        let ConsumeOutcome::Consumed(record) = store.consume_ticket(&actor(), &consume("r1", "bridge", 2_000)).unwrap()
        else {
            panic!("first consume");
        };
        assert_eq!(record.status, TicketStatus::Consumed);
        assert_eq!(record.consumed_at, Some(UnixMillis(2_000)));
        assert_eq!(record.consumer_bridge_instance_id.as_deref(), Some("bridge"));
        assert_eq!(record.consumer_request_id.as_deref(), Some("r1"));
        assert_eq!(store.ticket(DIGEST), Some(record.clone()));
        assert_eq!(
            store.consume_ticket(&actor(), &consume("r1", "bridge", 3_000)).unwrap(),
            ConsumeOutcome::Idempotent(record)
        );
        assert_eq!(
            store.consume_ticket(&actor(), &consume("r2", "bridge", 3_000)).unwrap(),
            ConsumeOutcome::Replay
        );
        assert_eq!(
            store.consume_ticket(&actor(), &consume("r1", "other", 3_000)).unwrap(),
            ConsumeOutcome::Replay
        );
        let mut other_peer = consume("r1", "bridge", 3_000);
        other_peer.joiner_peer = PeerId(12);
        assert_eq!(
            store.consume_ticket(&actor(), &other_peer).unwrap(),
            ConsumeOutcome::Replay
        );
    }

    #[test]
    fn refusals_change_nothing() {
        let store = stored();
        let mut unknown = consume("r1", "bridge", 2_000);
        unknown.jti_digest = "nope".to_owned();
        assert_eq!(
            store.consume_ticket(&actor(), &unknown).unwrap(),
            ConsumeOutcome::Invalid
        );
        assert_eq!(
            store
                .consume_ticket(&actor(), &consume("r1", "bridge", 10_000))
                .unwrap(),
            ConsumeOutcome::Expired
        );
        let mut other_host = consume("r1", "bridge", 2_000);
        other_host.host_peer = PeerId(23);
        assert_eq!(
            store.consume_ticket(&actor(), &other_host).unwrap(),
            ConsumeOutcome::Invalid
        );
        let mut other_session = consume("r1", "bridge", 2_000);
        other_session.session = "elsewhere".to_owned();
        assert_eq!(
            store.consume_ticket(&actor(), &other_session).unwrap(),
            ConsumeOutcome::Invalid
        );
        assert_eq!(store.ticket(DIGEST), Some(ticket()));

        let revoked = MemoryAdmissionStore::default();
        let ticket = TicketRecord {
            status: TicketStatus::Revoked,
            ..ticket()
        };
        assert!(revoked.insert_ticket(&actor(), &ticket).unwrap());
        assert_eq!(
            revoked
                .consume_ticket(&actor(), &consume("r1", "bridge", 2_000))
                .unwrap(),
            ConsumeOutcome::Invalid
        );
    }

    #[test]
    fn one_live_admission_per_key() {
        let store = MemoryAdmissionStore::default();
        assert!(store.insert_admission(&actor(), &admission("a1", "wire:3:11")).unwrap());
        assert!(!store.insert_admission(&actor(), &admission("a2", "wire:3:11")).unwrap());
        assert!(!store.insert_admission(&actor(), &admission("a1", "wire:3:12")).unwrap());

        let mut request = DisconnectRequest {
            admission_id: "a1".to_owned(),
            session: "wire".to_owned(),
            generation: 3,
            bridge_instance_id: "other".to_owned(),
            at: UnixMillis(5_000),
        };
        assert!(
            !store.disconnect(&actor(), &request).unwrap(),
            "only the admitting bridge"
        );
        request.bridge_instance_id = "bridge".to_owned();
        request.generation = 4;
        assert!(!store.disconnect(&actor(), &request).unwrap());
        request.generation = 3;
        assert!(store.disconnect(&actor(), &request).unwrap());
        assert!(!store.disconnect(&actor(), &request).unwrap(), "once");
        request.admission_id = "missing".to_owned();
        assert!(!store.disconnect(&actor(), &request).unwrap());

        let found = store.find_admission("a1").unwrap().unwrap();
        assert_eq!(found.active_key, None);
        assert_eq!(found.disconnected_at, Some(UnixMillis(5_000)));
        assert_eq!(found.superseded_at, None);
        assert!(store.insert_admission(&actor(), &admission("a2", "wire:3:11")).unwrap());
    }

    #[test]
    fn supersede_clears_the_key() {
        let store = MemoryAdmissionStore::default();
        assert!(!store.supersede_active(&actor(), "wire:3:11", UnixMillis(1)).unwrap());
        assert!(store.insert_admission(&actor(), &admission("a1", "wire:3:11")).unwrap());
        assert!(store.supersede_active(&actor(), "wire:3:11", UnixMillis(9)).unwrap());
        let found = store.find_admission("a1").unwrap().unwrap();
        assert_eq!(found.active_key, None);
        assert_eq!(found.superseded_at, Some(UnixMillis(9)));
        assert_eq!(found.disconnected_at, Some(UnixMillis(9)));
        assert!(store.insert_admission(&actor(), &admission("a2", "wire:3:11")).unwrap());
        assert_eq!(store.find_admission("zz").unwrap(), None);
    }

    #[test]
    fn peer_links() {
        let store = MemoryAdmissionStore::default();
        let account = AccountId("joiner-1".to_owned());
        assert_eq!(store.peer_link(&account).unwrap(), None);
        let mut link = PeerLinkRecord {
            account: account.clone(),
            peer: PeerId(11),
            status: LinkStatus::Verified,
            revision: 1,
            verified_at: UnixMillis(1),
            verification_method: "steam".to_owned(),
        };
        store.put_peer_link(&actor(), &link).unwrap();
        link.revision = 2;
        store.put_peer_link(&actor(), &link).unwrap();
        assert_eq!(store.peer_link(&account).unwrap(), Some(link));
    }

    #[test]
    fn stub_store_and_tables() {
        assert_eq!(
            StubAdmissionStore.peer_link(&AccountId("a".to_owned())),
            Err(missing("store.peer_link"))
        );
        assert_eq!(
            StubAdmissionStore.find_admission("x"),
            Err(missing("store.find_admission"))
        );
        assert_eq!(
            TABLES.iter().map(|table| table.name).collect::<Vec<_>>(),
            ["rifts_peer_links", "rifts_admission_tickets", "rifts_peer_admissions"]
        );
        assert!(
            TABLES
                .iter()
                .all(|table| table.owner == Component::Admission && table.expires_by.is_none())
        );
    }
}
