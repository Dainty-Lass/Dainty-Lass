//! Admission's tables (RF1, section 7), shaped for `store` (see `rifts_seam`'s store notes).

use rifts_seam::{
    AccountId, Actor, AdmissionId, Component, PeerId, SessionId, StoreError, TableSpec, Unimplemented, UnixMillis,
};

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
        keys: "session + generation + account",
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

/// `p2pSteamLinks`: an account's transport peer id. ICE derives it from the account (`ICE_ACCOUNT_DERIVED`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerLinkRecord {
    pub account: AccountId,
    pub peer: PeerId,
    pub revision: u64,
    pub verified_at: UnixMillis,
    pub verification_method: String,
}

/// `p2pAdmissionTickets`. Only the jti's digest is kept, never the ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketRecord {
    pub jti_digest: String,
    pub key_id: String,
    pub session: SessionId,
    pub generation: u64,
    pub account: AccountId,
    pub issued_at: UnixMillis,
    pub expires_at: UnixMillis,
    pub consumed_at: Option<UnixMillis>,
    pub consumer_bridge_instance_id: Option<String>,
}

/// `p2pPeerAdmissions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerAdmissionRecord {
    pub admission: AdmissionId,
    pub session: SessionId,
    pub generation: u64,
    pub account: AccountId,
    pub peer: PeerId,
    pub bridge_instance_id: String,
    pub admitted_at: UnixMillis,
    pub disconnected_at: Option<UnixMillis>,
}

pub trait AdmissionStore: Send + Sync {
    fn peer_link(&self, account: &AccountId) -> Result<Option<PeerLinkRecord>, StoreError>;
    fn put_peer_link(&self, actor: &Actor, link: &PeerLinkRecord) -> Result<(), StoreError>;
    fn insert_ticket(&self, actor: &Actor, ticket: &TicketRecord) -> Result<(), StoreError>;
    /// Compare-and-set on `consumed_at`: `None` when the ticket is unknown or already used.
    fn consume_ticket(
        &self,
        actor: &Actor,
        jti_digest: &str,
        consumer_bridge_instance_id: &str,
        at: UnixMillis,
    ) -> Result<Option<TicketRecord>, StoreError>;
    fn insert_admission(&self, actor: &Actor, admission: &PeerAdmissionRecord) -> Result<(), StoreError>;
    fn mark_disconnected(&self, actor: &Actor, admission: &AdmissionId, at: UnixMillis) -> Result<bool, StoreError>;
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
    fn insert_ticket(&self, _: &Actor, _: &TicketRecord) -> Result<(), StoreError> {
        unimplemented("store.insert_ticket")
    }
    fn consume_ticket(&self, _: &Actor, _: &str, _: &str, _: UnixMillis) -> Result<Option<TicketRecord>, StoreError> {
        unimplemented("store.consume_ticket")
    }
    fn insert_admission(&self, _: &Actor, _: &PeerAdmissionRecord) -> Result<(), StoreError> {
        unimplemented("store.insert_admission")
    }
    fn mark_disconnected(&self, _: &Actor, _: &AdmissionId, _: UnixMillis) -> Result<bool, StoreError> {
        unimplemented("store.mark_disconnected")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
