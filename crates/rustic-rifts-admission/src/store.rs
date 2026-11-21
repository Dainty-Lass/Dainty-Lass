// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! What admission keeps: an account's verified peer link, each issued ticket (by its jti digest, never its text) and
//! each peer admission. The in-memory repository is the reference for the semantics a database store must keep.

use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard, PoisonError},
};

use rustic_rifts_seam::{AccountId, Actor, Component, PeerId, StoreError, TableSpec, Unimplemented, UnixMillis};

type Stored<T> = Result<T, StoreError>;

const fn admission_table(name: &'static str, node_collection: &'static str, keys: &'static str) -> TableSpec {
    TableSpec {
        name,
        node_collection,
        keys,
        owner: Component::Admission,
        expires_by: None,
    }
}

/// Admission's tables, with the Node collection each replaces and its key.
pub const TABLES: &[TableSpec] = &[
    admission_table("rifts_peer_links", "p2pSteamLinks", "account; peer unique"),
    admission_table(
        "rifts_admission_tickets",
        "p2pAdmissionTickets",
        "jti digest; session + generation + account",
    ),
    admission_table(
        "rifts_peer_admissions",
        "p2pPeerAdmissions",
        "admission; active admission key unique",
    ),
];

/// Whether an account's peer link holds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinkStatus {
    /// The link was verified.
    Verified,
    /// The link was revoked.
    Revoked,
}

/// An account's verified link to its transport peer id.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeerLinkRecord {
    /// The account.
    pub account: AccountId,
    /// The linked peer id.
    pub peer: PeerId,
    /// Whether the link holds.
    pub status: LinkStatus,
    /// Bumped on every change.
    pub revision: u64,
    /// When the link was verified.
    pub verified_at: UnixMillis,
    /// How it was verified.
    pub verification_method: String,
}

/// Where an issued ticket is in its life.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TicketStatus {
    /// Minted, not consumed yet.
    Issued,
    /// Consumed by a host.
    Consumed,
    /// Revoked before use.
    Revoked,
}

/// An issued ticket, kept by its digest and bindings, never its text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TicketRecord {
    /// The SHA-256 hex digest of the ticket's jti.
    pub jti_digest: String,
    /// The key it was signed with.
    pub key_id: String,
    /// The session's wire text.
    pub session: String,
    /// The hunt generation.
    pub generation: u64,
    /// The joiner's account.
    pub account: AccountId,
    /// The joiner's peer id.
    pub joiner_peer: PeerId,
    /// The host's peer id.
    pub host_peer: PeerId,
    /// The bridge protocol version.
    pub bridge_protocol: u16,
    /// The game's virtual port.
    pub virtual_port: u16,
    /// When it was minted.
    pub issued_at: UnixMillis,
    /// When it expires.
    pub expires_at: UnixMillis,
    /// Where it is in its life.
    pub status: TicketStatus,
    /// When a host consumed it.
    pub consumed_at: Option<UnixMillis>,
    /// The bridge instance that consumed it.
    pub consumer_bridge_instance_id: Option<String>,
    /// The request id that consumed it, so the same request may retry.
    pub consumer_request_id: Option<String>,
}

/// A host's attempt to consume a ticket.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsumeRequest {
    /// The SHA-256 hex digest of the ticket's jti.
    pub jti_digest: String,
    /// The session's wire text.
    pub session: String,
    /// The hunt generation.
    pub generation: u64,
    /// The joiner's account.
    pub account: AccountId,
    /// The joiner's peer id.
    pub joiner_peer: PeerId,
    /// The host's peer id.
    pub host_peer: PeerId,
    /// The consuming bridge's instance id.
    pub bridge_instance_id: String,
    /// The request's id.
    pub request_id: String,
    /// When the attempt was made.
    pub now: UnixMillis,
}

/// What consuming a ticket did.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConsumeOutcome {
    /// Consumed now.
    Consumed(TicketRecord),
    /// The same bridge retried the same request.
    Idempotent(TicketRecord),
    /// The ticket expired.
    Expired,
    /// Another bridge or request already consumed it.
    Replay,
    /// No such ticket, or it does not match the attempt.
    Invalid,
}

/// One joiner's admission to a host's session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeerAdmissionRecord {
    /// The admission's id, 64 hex digits.
    pub admission_id: String,
    /// `<wire session>:<generation>:<peer>` while live; at most one live admission holds it.
    pub active_key: Option<String>,
    /// The session's wire text.
    pub session: String,
    /// The hunt generation.
    pub generation: u64,
    /// The joiner's account.
    pub account: AccountId,
    /// The joiner's peer id.
    pub peer: PeerId,
    /// The host bridge's instance id.
    pub bridge_instance_id: String,
    /// The joiner's connection handle on the host.
    pub connection_handle: String,
    /// When the joiner was admitted.
    pub admitted_at: UnixMillis,
    /// When the joiner disconnected or was superseded.
    pub disconnected_at: Option<UnixMillis>,
    /// When a newer admission for the same seat replaced it.
    pub superseded_at: Option<UnixMillis>,
}

/// A host's report that an admitted peer disconnected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisconnectRequest {
    /// The admission's id.
    pub admission_id: String,
    /// The session's wire text.
    pub session: String,
    /// The hunt generation.
    pub generation: u64,
    /// The reporting bridge's instance id, which must be the admitting one.
    pub bridge_instance_id: String,
    /// When the peer disconnected.
    pub at: UnixMillis,
}

/// Admission's storage; each method is one transaction.
pub trait AdmissionStore: Send + Sync {
    /// The account's peer link, `None` when it has none.
    fn peer_link(&self, of: &AccountId) -> Stored<Option<PeerLinkRecord>>;
    /// Writes the account's peer link, replacing any earlier one.
    fn put_peer_link(&self, by: &Actor, row: &PeerLinkRecord) -> Stored<()>;
    /// `false` when the digest is already stored.
    fn insert_ticket(&self, by: &Actor, row: &TicketRecord) -> Stored<bool>;
    /// Consumes a ticket if it matches the attempt and is live.
    fn consume_ticket(&self, by: &Actor, attempt: &ConsumeRequest) -> Stored<ConsumeOutcome>;
    /// `false` when the id or the live active key is already held.
    fn insert_admission(&self, by: &Actor, row: &PeerAdmissionRecord) -> Stored<bool>;
    /// An admission by id.
    fn find_admission(&self, id: &str) -> Stored<Option<PeerAdmissionRecord>>;
    /// Closes the live admission holding `key`; `false` when none does.
    fn supersede_active(&self, by: &Actor, key: &str, at: UnixMillis) -> Stored<bool>;
    /// Marks the admission disconnected; `false` when it is unknown, already closed or from another bridge.
    fn disconnect(&self, by: &Actor, attempt: &DisconnectRequest) -> Stored<bool>;
}

/// Fails every operation with `Unimplemented`.
#[derive(Clone, Copy, Debug, Default)]
pub struct StubAdmissionStore;

fn missing(operation: &'static str) -> StoreError {
    Unimplemented::new(Component::Admission, operation).into()
}

/// Each method of the stub fails with `store.<method>`.
macro_rules! unwritten {
    ($($method:ident($($arg:ty),*) -> $out:ty;)*) => {$(
        fn $method(&self, $(_: $arg),*) -> Stored<$out> {
            Err(missing(concat!("store.", stringify!($method))))
        }
    )*};
}

impl AdmissionStore for StubAdmissionStore {
    unwritten! {
        peer_link(&AccountId) -> Option<PeerLinkRecord>;
        put_peer_link(&Actor, &PeerLinkRecord) -> ();
        insert_ticket(&Actor, &TicketRecord) -> bool;
        consume_ticket(&Actor, &ConsumeRequest) -> ConsumeOutcome;
        insert_admission(&Actor, &PeerAdmissionRecord) -> bool;
        find_admission(&str) -> Option<PeerAdmissionRecord>;
        supersede_active(&Actor, &str, UnixMillis) -> bool;
        disconnect(&Actor, &DisconnectRequest) -> bool;
    }
}

#[derive(Debug, Default)]
struct Rows {
    links: HashMap<AccountId, PeerLinkRecord>,
    tickets: HashMap<String, TicketRecord>,
    admissions: HashMap<String, PeerAdmissionRecord>,
}

impl Rows {
    fn holder_of(&mut self, key: &str) -> Option<&mut PeerAdmissionRecord> {
        self.admissions
            .values_mut()
            .find(|row| row.active_key.as_deref() == Some(key))
    }
}

/// One process's admission rows behind one lock. The actor is not recorded.
#[derive(Debug, Default)]
pub struct MemoryAdmissionStore {
    tables: Mutex<Rows>,
}

impl MemoryAdmissionStore {
    fn rows(&self) -> MutexGuard<'_, Rows> {
        // Every write is a whole-row swap, so a panic elsewhere leaves the maps consistent.
        self.tables.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The stored ticket with this jti digest.
    pub fn ticket(&self, jti_digest: &str) -> Option<TicketRecord> {
        self.rows().tickets.get(jti_digest).cloned()
    }
}

fn settle(row: &mut TicketRecord, attempt: &ConsumeRequest) -> ConsumeOutcome {
    let same_seat = row.session == attempt.session
        && row.generation == attempt.generation
        && row.joiner_peer == attempt.joiner_peer;
    let live = row.expires_at > attempt.now;
    match row.status {
        TicketStatus::Issued
            if same_seat && live && row.account == attempt.account && row.host_peer == attempt.host_peer =>
        {
            row.status = TicketStatus::Consumed;
            row.consumed_at = Some(attempt.now);
            row.consumer_bridge_instance_id = Some(attempt.bridge_instance_id.clone());
            row.consumer_request_id = Some(attempt.request_id.clone());
            ConsumeOutcome::Consumed(row.clone())
        }
        TicketStatus::Consumed => {
            let retried = row.consumer_bridge_instance_id.as_ref() == Some(&attempt.bridge_instance_id)
                && row.consumer_request_id.as_ref() == Some(&attempt.request_id);
            if retried && same_seat {
                ConsumeOutcome::Idempotent(row.clone())
            } else {
                ConsumeOutcome::Replay
            }
        }
        _ if live => ConsumeOutcome::Invalid,
        _ => ConsumeOutcome::Expired,
    }
}

impl AdmissionStore for MemoryAdmissionStore {
    fn peer_link(&self, of: &AccountId) -> Stored<Option<PeerLinkRecord>> {
        Ok(self.rows().links.get(of).cloned())
    }

    fn put_peer_link(&self, _: &Actor, row: &PeerLinkRecord) -> Stored<()> {
        self.rows().links.insert(row.account.clone(), row.clone());
        Ok(())
    }

    fn insert_ticket(&self, _: &Actor, row: &TicketRecord) -> Stored<bool> {
        let mut rows = self.rows();
        let fresh = !rows.tickets.contains_key(&row.jti_digest);
        if fresh {
            rows.tickets.insert(row.jti_digest.clone(), row.clone());
        }
        Ok(fresh)
    }

    fn consume_ticket(&self, _: &Actor, attempt: &ConsumeRequest) -> Stored<ConsumeOutcome> {
        Ok(self
            .rows()
            .tickets
            .get_mut(&attempt.jti_digest)
            .map_or(ConsumeOutcome::Invalid, |row| settle(row, attempt)))
    }

    fn insert_admission(&self, _: &Actor, row: &PeerAdmissionRecord) -> Stored<bool> {
        let mut rows = self.rows();
        let key_taken = match row.active_key.as_deref() {
            Some(key) => rows.holder_of(key).is_some(),
            None => false,
        };
        let fresh = !key_taken && !rows.admissions.contains_key(&row.admission_id);
        if fresh {
            rows.admissions.insert(row.admission_id.clone(), row.clone());
        }
        Ok(fresh)
    }

    fn find_admission(&self, id: &str) -> Stored<Option<PeerAdmissionRecord>> {
        Ok(self.rows().admissions.get(id).cloned())
    }

    fn supersede_active(&self, _: &Actor, key: &str, at: UnixMillis) -> Stored<bool> {
        let mut rows = self.rows();
        let Some(row) = rows.holder_of(key) else {
            return Ok(false);
        };
        row.active_key = None;
        (row.disconnected_at, row.superseded_at) = (Some(at), Some(at));
        Ok(true)
    }

    fn disconnect(&self, _: &Actor, attempt: &DisconnectRequest) -> Stored<bool> {
        let mut rows = self.rows();
        let row = rows.admissions.get_mut(&attempt.admission_id).filter(|row| {
            row.disconnected_at.is_none()
                && (row.session.as_str(), row.generation, row.bridge_instance_id.as_str())
                    == (
                        attempt.session.as_str(),
                        attempt.generation,
                        attempt.bridge_instance_id.as_str(),
                    )
        });
        Ok(row
            .map(|row| {
                row.active_key = None;
                row.disconnected_at = Some(attempt.at);
            })
            .is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "d1";
    const WIRE: &str = "wire";
    const KEY: &str = "wire:3:11";

    fn by() -> Actor {
        Actor::System("test")
    }

    fn joiner() -> AccountId {
        AccountId("joiner-1".to_owned())
    }

    fn ticket() -> TicketRecord {
        TicketRecord {
            status: TicketStatus::Issued,
            issued_at: UnixMillis(1_000),
            expires_at: UnixMillis(10_000),
            jti_digest: DIGEST.into(),
            key_id: "k".into(),
            session: WIRE.into(),
            generation: 3,
            account: joiner(),
            joiner_peer: PeerId(11),
            host_peer: PeerId(22),
            bridge_protocol: 1,
            virtual_port: 3,
            consumed_at: None,
            consumer_bridge_instance_id: None,
            consumer_request_id: None,
        }
    }

    fn attempt(request: &str, bridge: &str, now: i64) -> ConsumeRequest {
        ConsumeRequest {
            now: UnixMillis(now),
            request_id: request.into(),
            bridge_instance_id: bridge.into(),
            jti_digest: DIGEST.into(),
            session: WIRE.into(),
            account: joiner(),
            generation: 3,
            joiner_peer: PeerId(11),
            host_peer: PeerId(22),
        }
    }

    fn attempt_with(change: impl FnOnce(&mut ConsumeRequest)) -> ConsumeRequest {
        let mut changed = attempt("r1", "bridge", 2_000);
        change(&mut changed);
        changed
    }

    fn admission(id: &str, key: &str) -> PeerAdmissionRecord {
        PeerAdmissionRecord {
            active_key: Some(key.into()),
            admission_id: id.into(),
            account: joiner(),
            session: WIRE.into(),
            peer: PeerId(11),
            generation: 3,
            connection_handle: "h1".into(),
            bridge_instance_id: "bridge".into(),
            admitted_at: UnixMillis(2_000),
            superseded_at: None,
            disconnected_at: None,
        }
    }

    fn with_ticket(row: TicketRecord) -> MemoryAdmissionStore {
        let store = MemoryAdmissionStore::default();
        assert!(store.insert_ticket(&by(), &row).unwrap());
        store
    }

    fn admitted(ids: &[(&str, &str, bool)]) -> MemoryAdmissionStore {
        let store = MemoryAdmissionStore::default();
        for &(id, key, taken) in ids {
            assert_eq!(
                store.insert_admission(&by(), &admission(id, key)).unwrap(),
                taken,
                "{id} {key}"
            );
        }
        store
    }

    #[test]
    fn consumed_once_then_idempotent_or_replay() {
        let store = with_ticket(ticket());
        assert!(!store.insert_ticket(&by(), &ticket()).unwrap());
        let outcome = store.consume_ticket(&by(), &attempt("r1", "bridge", 2_000)).unwrap();
        let ConsumeOutcome::Consumed(row) = outcome else {
            panic!("first consume: {outcome:?}");
        };
        let expected = TicketRecord {
            status: TicketStatus::Consumed,
            consumed_at: Some(UnixMillis(2_000)),
            consumer_bridge_instance_id: Some("bridge".into()),
            consumer_request_id: Some("r1".into()),
            ..ticket()
        };
        assert_eq!(row, expected);
        assert_eq!(store.ticket(DIGEST).as_ref(), Some(&row));

        let later = [
            (attempt("r1", "bridge", 3_000), ConsumeOutcome::Idempotent(row)),
            (attempt("r2", "bridge", 3_000), ConsumeOutcome::Replay),
            (attempt("r1", "other", 3_000), ConsumeOutcome::Replay),
            (
                attempt_with(|a| (a.joiner_peer, a.now) = (PeerId(12), UnixMillis(3_000))),
                ConsumeOutcome::Replay,
            ),
        ];
        for (retry, outcome) in later {
            assert_eq!(store.consume_ticket(&by(), &retry).unwrap(), outcome, "{retry:?}");
        }
    }

    #[test]
    fn refusals_change_nothing() {
        let store = with_ticket(ticket());
        let refused = [
            (attempt_with(|a| a.jti_digest = "nope".into()), ConsumeOutcome::Invalid),
            (attempt("r1", "bridge", 10_000), ConsumeOutcome::Expired),
            (attempt_with(|a| a.host_peer = PeerId(23)), ConsumeOutcome::Invalid),
            (
                attempt_with(|a| a.session = "elsewhere".into()),
                ConsumeOutcome::Invalid,
            ),
        ];
        for (request, outcome) in refused {
            assert_eq!(store.consume_ticket(&by(), &request).unwrap(), outcome, "{request:?}");
        }
        assert_eq!(store.ticket(DIGEST), Some(ticket()));

        let revoked = with_ticket(TicketRecord {
            status: TicketStatus::Revoked,
            ..ticket()
        });
        assert_eq!(
            revoked.consume_ticket(&by(), &attempt("r1", "bridge", 2_000)).unwrap(),
            ConsumeOutcome::Invalid
        );
    }

    #[test]
    fn one_live_admission_per_key() {
        let store = admitted(&[("a1", KEY, true), ("a2", KEY, false), ("a1", "wire:3:12", false)]);
        let mut request = DisconnectRequest {
            at: UnixMillis(5_000),
            admission_id: "a1".into(),
            bridge_instance_id: "other".into(),
            session: WIRE.into(),
            generation: 3,
        };
        let mut disconnect = |change: &dyn Fn(&mut DisconnectRequest)| {
            change(&mut request);
            store.disconnect(&by(), &request).unwrap()
        };
        assert!(!disconnect(&|_| ()), "only the admitting bridge");
        assert!(!disconnect(
            &|r| (r.bridge_instance_id, r.generation) = ("bridge".into(), 4)
        ));
        assert!(disconnect(&|r| r.generation = 3));
        assert!(!disconnect(&|_| ()), "once");
        assert!(!disconnect(&|r| r.admission_id = "missing".into()));

        let row = store.find_admission("a1").unwrap().unwrap();
        assert_eq!(
            (row.active_key, row.disconnected_at, row.superseded_at),
            (None, Some(UnixMillis(5_000)), None)
        );
        assert!(store.insert_admission(&by(), &admission("a2", KEY)).unwrap());
    }

    #[test]
    fn supersede_clears_the_key() {
        let store = MemoryAdmissionStore::default();
        assert!(!store.supersede_active(&by(), KEY, UnixMillis(1)).unwrap());
        assert!(store.insert_admission(&by(), &admission("a1", KEY)).unwrap());
        assert!(store.supersede_active(&by(), KEY, UnixMillis(9)).unwrap());
        let row = store.find_admission("a1").unwrap().unwrap();
        assert_eq!(
            (row.active_key, row.superseded_at, row.disconnected_at),
            (None, Some(UnixMillis(9)), Some(UnixMillis(9)))
        );
        assert!(store.insert_admission(&by(), &admission("a2", KEY)).unwrap());
        assert_eq!(store.find_admission("zz").unwrap(), None);
    }

    #[test]
    fn peer_links() {
        let store = MemoryAdmissionStore::default();
        assert_eq!(store.peer_link(&joiner()).unwrap(), None);
        let first = PeerLinkRecord {
            verification_method: "steam".into(),
            verified_at: UnixMillis(1),
            status: LinkStatus::Verified,
            account: joiner(),
            peer: PeerId(11),
            revision: 1,
        };
        let second = PeerLinkRecord {
            revision: 2,
            ..first.clone()
        };
        for row in [&first, &second] {
            store.put_peer_link(&by(), row).unwrap();
        }
        assert_eq!(store.peer_link(&joiner()).unwrap(), Some(second));
    }

    #[test]
    fn stub_store_and_tables() {
        assert_eq!(
            StubAdmissionStore.peer_link(&AccountId("a".into())),
            Err(missing("store.peer_link"))
        );
        assert_eq!(
            StubAdmissionStore.find_admission("x"),
            Err(missing("store.find_admission"))
        );
        assert_eq!(
            StubAdmissionStore.supersede_active(&by(), "k", UnixMillis(0)),
            Err(missing("store.supersede_active"))
        );
        let names: Vec<&str> = TABLES.iter().map(|table| table.name).collect();
        assert_eq!(
            names,
            ["rifts_peer_links", "rifts_admission_tickets", "rifts_peer_admissions"]
        );
        for table in TABLES {
            assert_eq!((table.owner, table.expires_by), (Component::Admission, None));
        }
    }
}
