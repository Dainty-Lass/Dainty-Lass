use std::sync::Mutex;

use rifts_relay::rules::{CoopTransport, HuntState, PresenceState, RosterEntry, SessionKind};
use rifts_seam::poll_once;

use super::*;
use crate::store::MemoryAdmissionStore;
use crate::{Es256Tickets, KeyId, P256KeyGenerator};

const NOW: UnixMillis = UnixMillis(1_800_000_000_000);
const HOST: &str = "host-0000-account";
const JOIN_A: &str = "join-aaaa-account";
const JOIN_B: &str = "join-bbbb-account";
const BRIDGE: &str = "0123456789abcdef0123456789abcdef";
const WIRE_SESSION: &str = "01234567-89ab-4def-8123-456789abcdef";

fn account(id: &str) -> AccountId {
    AccountId(id.to_owned())
}

fn session() -> SessionId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV@local".parse().unwrap()
}

fn hunt(kind: SessionKind) -> AdmissionHunt {
    let entry = |id: &str, peer: u64| RosterEntry {
        account: account(id),
        peer: PeerId(peer),
        returned_to_hub: false,
    };
    AdmissionHunt {
        snapshot: HuntSnapshot {
            wire_session: WIRE_SESSION.to_owned(),
            generation: 7,
            state: HuntState::Running,
            kind,
            realm: (kind == SessionKind::RealmHub).then(|| "realm-1".to_owned()),
            coop_transport: CoopTransport::Ice,
            host_account: account(HOST),
            host_peer: PeerId(1001),
            expires_at: UnixMillis(NOW.0 + 3_600_000),
            roster: if kind == SessionKind::Hunt {
                vec![entry(HOST, 1001), entry(JOIN_A, 1002), entry(JOIN_B, 1003)]
            } else {
                Vec::new()
            },
        },
        session: WIRE_SESSION.parse().unwrap(),
        party_id: "party_abc==".to_owned(),
        party_revision: 12,
        hunt_definition_id: "hunt/island-1".to_owned(),
        build: 392_819,
        bridge_protocol: 1,
        virtual_port: 1,
        lobby_id: Some("ICE".to_owned()),
        bridge_instance_id: Some(BRIDGE.to_owned()),
    }
}

#[derive(Default)]
struct Calls {
    reserved: Vec<AccountId>,
    ticketed: Vec<(AccountId, UnixMillis)>,
    admitted: Vec<AccountId>,
    hub_admitted: Vec<(AccountId, String)>,
    disconnected: Vec<String>,
}

struct FakeHunts {
    hunt: Mutex<Option<AdmissionHunt>>,
    gate: Mutex<P2PGate>,
    presence: Vec<PresenceRow>,
    reserve_refused: bool,
    calls: Mutex<Calls>,
}

impl FakeHunts {
    fn new(hunt: AdmissionHunt) -> Arc<Self> {
        Arc::new(Self {
            hunt: Mutex::new(Some(hunt)),
            gate: Mutex::new(P2PGate {
                enabled: true,
                emergency_stop: false,
            }),
            presence: Vec::new(),
            reserve_refused: false,
            calls: Mutex::default(),
        })
    }
}

impl AdmissionHunts for FakeHunts {
    fn gate(&self) -> Result<P2PGate, AdmissionError> {
        Ok(*self.gate.lock().unwrap())
    }
    fn find_hunt(&self, _: &SessionId) -> Result<Option<AdmissionHunt>, AdmissionError> {
        Ok(self.hunt.lock().unwrap().clone())
    }
    fn reserve_participant(
        &self,
        hunt: &AdmissionHunt,
        account: &AccountId,
        _: UnixMillis,
    ) -> Result<Option<AdmissionHunt>, AdmissionError> {
        self.calls.lock().unwrap().reserved.push(account.clone());
        Ok((!self.reserve_refused).then(|| hunt.clone()))
    }
    fn check_hub_entry(
        &self,
        _: &AdmissionHunt,
        account: &AccountId,
        link: Option<&PeerLinkRecord>,
        _: UnixMillis,
    ) -> Result<PeerId, AdmissionError> {
        let row = self
            .presence
            .iter()
            .find(|row| {
                &row.account == account && matches!(row.state, PresenceState::Pending | PresenceState::Ticketed)
            })
            .ok_or_else(|| refused("P2P_NOT_ROSTERED", "This account has no open entry into the hub."))?;
        match link {
            Some(link) if link.status == LinkStatus::Verified && link.peer == row.peer => Ok(row.peer),
            _ => Err(refused(
                "P2P_STEAM_LINK_REQUIRED",
                "The current co-op transport no longer matches this hub entry.",
            )),
        }
    }
    fn mark_hub_ticketed(
        &self,
        _: &AdmissionHunt,
        account: &AccountId,
        _: UnixMillis,
        expires_at: UnixMillis,
    ) -> Result<bool, AdmissionError> {
        self.calls.lock().unwrap().ticketed.push((account.clone(), expires_at));
        Ok(true)
    }
    fn mark_participant_admitted(
        &self,
        _: &AdmissionHunt,
        account: &AccountId,
        _: UnixMillis,
    ) -> Result<bool, AdmissionError> {
        self.calls.lock().unwrap().admitted.push(account.clone());
        Ok(true)
    }
    fn admit_hub_peer(
        &self,
        _: &AdmissionHunt,
        account: &AccountId,
        admission_id: &str,
        _: UnixMillis,
    ) -> Result<(), AdmissionError> {
        self.calls
            .lock()
            .unwrap()
            .hub_admitted
            .push((account.clone(), admission_id.to_owned()));
        Ok(())
    }
    fn peer_disconnected(&self, admission: &PeerAdmissionRecord, _: UnixMillis) -> Result<(), AdmissionError> {
        self.calls
            .lock()
            .unwrap()
            .disconnected
            .push(admission.admission_id.clone());
        Ok(())
    }
    fn hub_presence(&self, _: &AdmissionHunt, limit: usize) -> Result<Vec<PresenceRow>, AdmissionError> {
        Ok(self
            .presence
            .iter()
            .filter(|row| row.state.is_live())
            .take(limit)
            .cloned()
            .collect())
    }
    fn join_bootstrap_common(
        &self,
        hunt: &AdmissionHunt,
        key_id: &str,
        bridge_instance_id: &str,
    ) -> Result<Option<Json>, AdmissionError> {
        Ok(Some(Json(format!(
            r#"{{"coopTransport":"ICE","keyId":"{key_id}","sessionId":"{}","bridgeInstanceId":"{bridge_instance_id}"}}"#,
            hunt.snapshot.wire_session
        ))))
    }
}

struct Rig {
    admission: Admission,
    hunts: Arc<FakeHunts>,
    store: Arc<MemoryAdmissionStore>,
    tickets: Arc<Es256Tickets>,
}

fn rig_with(hunts: Arc<FakeHunts>) -> Rig {
    let (_, private_pem, _) = P256KeyGenerator
        .generate_with_pems(KeyId("test-key-1".to_owned()))
        .unwrap();
    let signing = crate::keys::signing_key_from_pem(private_pem.expose()).unwrap();
    let tickets = Arc::new(Es256Tickets {
        key_id: KeyId("test-key-1".to_owned()),
        verifying: *signing.verifying_key(),
        signing: Some(signing),
        default_lifetime: 75.0,
    });
    let store = Arc::new(MemoryAdmissionStore::default());
    for (id, peer) in [(JOIN_A, 1002), (JOIN_B, 1003), (HOST, 1001)] {
        store
            .put_peer_link(
                &Actor::System("test"),
                &PeerLinkRecord {
                    account: account(id),
                    peer: PeerId(peer),
                    status: LinkStatus::Verified,
                    revision: 1,
                    verified_at: NOW,
                    verification_method: "ICE_ACCOUNT_DERIVED".to_owned(),
                },
            )
            .unwrap();
    }
    let admission = Admission::new(
        tickets.clone(),
        store.clone(),
        hunts.clone(),
        Some(Secret::new("s".repeat(40))),
        Arc::new(|| NOW),
    );
    Rig {
        admission,
        hunts,
        store,
        tickets,
    }
}

fn rig() -> Rig {
    rig_with(FakeHunts::new(hunt(SessionKind::Hunt)))
}

fn capability() -> HostCapability {
    HostCapability {
        session: session(),
        wire_session: WIRE_SESSION.to_owned(),
        generation: 7,
        host_account: account(HOST),
        host_peer: PeerId(1001),
        bridge_instance_id: BRIDGE.to_owned(),
    }
}

fn consume_body(ticket: &str, peer: &str) -> Value {
    serde_json::json!({ "ticket": ticket, "authenticatedSteamId": peer, "bridgeInstanceId": BRIDGE,
        "connectionHandle": "link-1" })
}

fn code(error: AdmissionError) -> String {
    match error {
        AdmissionError::Refused(refusal) => refusal.code,
        other => format!("{other:?}"),
    }
}

impl Rig {
    fn issue(&self, who: &str) -> Result<IssuedTicket, AdmissionError> {
        poll_once(self.admission.issue_self_ticket(account(who), session())).unwrap()
    }

    fn consume(&self, ticket: &str, peer: &str, request_id: &str) -> Result<ConsumedTicket, AdmissionError> {
        let request = ConsumeTicket::from_body(&consume_body(ticket, peer), request_id)?;
        poll_once(self.admission.consume_ticket(capability(), request)).unwrap()
    }
}

#[test]
fn a_joiner_is_ticketed_admitted_once_and_disconnected() {
    let rig = rig();
    let issued = rig.issue(JOIN_A).unwrap();
    let json = issued.to_json().0;
    assert!(json.starts_with(r#"{"ticket":"mpt1."#));
    assert!(json.ends_with(concat!(
        r#","expiresAt":"2027-01-15T08:03:00.000Z","rendezvous":{"sessionId":"01234567-89ab-4def-8123-456789abcdef","#,
        r#""generation":7,"lobbyId":"ICE","hostSteamId":"1001","virtualPort":1,"bridgeProtocol":1,"build":392819}}"#
    )));
    let claims = rig
        .tickets
        .verify(issued.ticket.text.expose(), NOW.0 as u64 / 1000)
        .unwrap();
    assert_eq!(claims.bindings.joiner_peer, PeerId(1002));
    assert_eq!(claims.expires_at - claims.issued_at, 180);
    let stored = rig.store.ticket(&issued.ticket.jti_digest).unwrap();
    assert_eq!(
        (stored.status, stored.session.as_str()),
        (TicketStatus::Issued, WIRE_SESSION)
    );
    assert_eq!(rig.hunts.calls.lock().unwrap().reserved, [account(JOIN_A)]);

    let consumed = rig.consume(issued.ticket.text.expose(), "1002", "req-1").unwrap();
    assert_eq!(
        consumed.to_json().0,
        format!(
            r#"{{"admitted":true,"idempotent":false,"admissionId":"{}","accountId":"join-aaaa-account","steamId":"1002","sessionId":"{WIRE_SESSION}","generation":7}}"#,
            issued.ticket.jti_digest
        )
    );
    assert_eq!(rig.hunts.calls.lock().unwrap().admitted, [account(JOIN_A)]);
    let again = rig.consume(issued.ticket.text.expose(), "1002", "req-1").unwrap();
    assert!(again.idempotent);
    let replay = rig.consume(issued.ticket.text.expose(), "1002", "req-2").unwrap_err();
    assert_eq!(code(replay), "P2P_TICKET_REPLAYED");

    let disconnect = |id: &str| {
        poll_once(
            rig.admission
                .record_peer_disconnected(capability(), id.to_owned(), None),
        )
        .unwrap()
        .unwrap()
    };
    assert!(disconnect(&consumed.admission_id));
    assert!(!disconnect(&consumed.admission_id));
    assert!(!disconnect(&"0".repeat(64)));
    assert_eq!(
        rig.hunts.calls.lock().unwrap().disconnected,
        std::slice::from_ref(&consumed.admission_id)
    );
    let bad = poll_once(
        rig.admission
            .record_peer_disconnected(capability(), "XYZ".to_owned(), None),
    )
    .unwrap();
    assert_eq!(code(bad.unwrap_err()), "P2P_TICKET_INVALID");
}

#[test]
fn self_tickets_are_refused_as_node_refuses_them() {
    let refused_with = |hunts: Arc<FakeHunts>, who: &str| code(rig_with(hunts).issue(who).unwrap_err());
    let off = FakeHunts::new(hunt(SessionKind::Hunt));
    off.gate.lock().unwrap().enabled = false;
    assert_eq!(refused_with(off, JOIN_A), "P2P_DISABLED");
    let stopped = FakeHunts::new(hunt(SessionKind::Hunt));
    stopped.gate.lock().unwrap().emergency_stop = true;
    assert_eq!(refused_with(stopped, JOIN_A), "P2P_DISABLED");
    let missing = FakeHunts::new(hunt(SessionKind::Hunt));
    *missing.hunt.lock().unwrap() = None;
    assert_eq!(refused_with(missing, JOIN_A), "P2P_SESSION_NOT_FOUND");
    let mut starting = hunt(SessionKind::Hunt);
    starting.snapshot.state = HuntState::Starting;
    assert_eq!(refused_with(FakeHunts::new(starting), JOIN_A), "P2P_SESSION_NOT_READY");
    let mut over = hunt(SessionKind::Hunt);
    over.snapshot.expires_at = NOW;
    assert_eq!(refused_with(FakeHunts::new(over), JOIN_A), "P2P_SESSION_NOT_READY");
    assert_eq!(
        refused_with(FakeHunts::new(hunt(SessionKind::Hunt)), "someone-else"),
        "P2P_NOT_ROSTERED"
    );
    let mut moved = hunt(SessionKind::Hunt);
    moved.snapshot.roster[1].peer = PeerId(9999);
    assert_eq!(refused_with(FakeHunts::new(moved), JOIN_A), "P2P_STEAM_LINK_REQUIRED");
    let mut full = FakeHunts::new(hunt(SessionKind::Hunt));
    Arc::get_mut(&mut full).unwrap().reserve_refused = true;
    assert_eq!(refused_with(full, JOIN_A), "P2P_SESSION_NOT_READY");
}

#[test]
fn consumes_are_refused_as_node_refuses_them() {
    let rig = rig();
    let issued = rig.issue(JOIN_A).unwrap();
    let ticket = issued.ticket.text.expose();
    let malformed = ConsumeTicket::from_body(
        &serde_json::json!({ "ticket": ticket, "authenticatedSteamId": "1002",
        "bridgeInstanceId": "short", "connectionHandle": "h" }),
        "r",
    )
    .unwrap_err();
    assert_eq!(code(malformed), "P2P_TICKET_INVALID");
    assert!(ConsumeTicket::from_body(&serde_json::json!({ "ticket": 5 }), "r").is_err());
    assert_eq!(
        code(rig.consume("mpt1.x.y", "1002", "r").unwrap_err()),
        "P2P_TICKET_INVALID"
    );
    assert_eq!(
        code(rig.consume(ticket, "1003", "r").unwrap_err()),
        "P2P_TICKET_INVALID"
    );
    let mut other_bridge = capability();
    other_bridge.bridge_instance_id = "ffffffffffffffffffffffffffffffff".to_owned();
    let request = ConsumeTicket::from_body(&consume_body(ticket, "1002"), "r").unwrap();
    let fenced = poll_once(rig.admission.consume_ticket(other_bridge, request))
        .unwrap()
        .unwrap_err();
    assert_eq!(code(fenced), "P2P_GENERATION_FENCED");
    // A ticket signed by this key but never issued here
    let unknown = rig
        .tickets
        .mint(issued.ticket.claims.bindings.clone(), NOW.0 as u64 / 1000, Some(60))
        .unwrap();
    assert_eq!(
        code(rig.consume(unknown.text.expose(), "1002", "r").unwrap_err()),
        "P2P_TICKET_INVALID"
    );
    // The hunt moved on to another host transport
    rig.hunts.hunt.lock().unwrap().as_mut().unwrap().bridge_instance_id = Some("x".repeat(32));
    assert_eq!(
        code(rig.consume(ticket, "1002", "r").unwrap_err()),
        "P2P_GENERATION_FENCED"
    );
    assert!(rig.store.ticket(&issued.ticket.jti_digest).unwrap().status == TicketStatus::Issued);
}

#[test]
fn an_expired_ticket_is_answered_as_expired() {
    let hunts = FakeHunts::new(hunt(SessionKind::Hunt));
    let early = rig_with(hunts.clone());
    let issued = early.issue(JOIN_A).unwrap();
    let late = Admission::new(
        early.tickets.clone(),
        early.store.clone(),
        hunts,
        None,
        Arc::new(|| UnixMillis(NOW.0 + 211_000)),
    );
    let request = ConsumeTicket::from_body(&consume_body(issued.ticket.text.expose(), "1002"), "r").unwrap();
    let error = poll_once(late.consume_ticket(capability(), request))
        .unwrap()
        .unwrap_err();
    assert_eq!(code(error), "P2P_TICKET_EXPIRED");
}

fn hub_rig() -> Rig {
    let mut hunts = FakeHunts::new(hunt(SessionKind::RealmHub));
    let row = |id: &str, peer: u64, state: PresenceState| PresenceRow {
        account: account(id),
        peer: PeerId(peer),
        state,
    };
    Arc::get_mut(&mut hunts).unwrap().presence = vec![
        row(HOST, 1001, PresenceState::Admitted),
        row(JOIN_A, 1002, PresenceState::Pending),
        row(JOIN_B, 1003, PresenceState::Lost),
    ];
    rig_with(hunts)
}

#[test]
fn a_hub_member_gets_a_ticket_per_entry_and_a_new_entry_supersedes_the_old_link() {
    let rig = hub_rig();
    let first = rig.issue(JOIN_A).unwrap();
    assert_eq!(
        rig.hunts.calls.lock().unwrap().ticketed,
        [(account(JOIN_A), UnixMillis(NOW.0 + 180_000))]
    );
    assert!(rig.hunts.calls.lock().unwrap().reserved.is_empty());
    let admitted = rig.consume(first.ticket.text.expose(), "1002", "r1").unwrap();
    let second = rig.issue(JOIN_A).unwrap();
    let readmitted = rig.consume(second.ticket.text.expose(), "1002", "r2").unwrap();
    assert_ne!(admitted.admission_id, readmitted.admission_id);
    let old = rig.store.find_admission(&admitted.admission_id).unwrap().unwrap();
    assert!(old.superseded_at.is_some() && old.active_key.is_none());
    assert_eq!(rig.hunts.calls.lock().unwrap().hub_admitted.len(), 2);
    assert!(rig.hunts.calls.lock().unwrap().admitted.is_empty());
    assert_eq!(code(rig.issue(JOIN_B).unwrap_err()), "P2P_NOT_ROSTERED");
}

#[test]
fn a_hub_host_lists_its_live_peers_with_their_pair_keys() {
    let hub = hub_rig();
    let list = |who: &str, generation: Option<i64>| {
        poll_once(hub.admission.list_hub_peers(account(who), session(), generation)).unwrap()
    };
    let peers = list(HOST, Some(7)).unwrap();
    let key = derive_pair_key(
        &Secret::new("s".repeat(40)),
        WIRE_SESSION,
        7,
        &account(HOST),
        &account(JOIN_A),
    );
    assert_eq!(
        peers.to_json().0,
        format!(
            r#"{{"generation":7,"peers":[{{"accountId":"join-aaaa-account","peerId":"1002","transportKey":"{}","state":"PENDING"}}]}}"#,
            key.expose()
        )
    );
    assert_eq!(code(list(JOIN_A, Some(7)).unwrap_err()), "P2P_NOT_ROSTERED");
    assert_eq!(code(list(HOST, Some(6)).unwrap_err()), "P2P_GENERATION_FENCED");
    assert_eq!(code(list(HOST, None).unwrap_err()), "P2P_TICKET_INVALID");
    let hunt_rig = rig();
    let not_hub = poll_once(hunt_rig.admission.list_hub_peers(account(HOST), session(), Some(7))).unwrap();
    assert_eq!(code(not_hub.unwrap_err()), "P2P_SESSION_NOT_FOUND");
    let no_secret = Admission::new(
        hub.tickets.clone(),
        hub.store.clone(),
        hub.hunts.clone(),
        None,
        Arc::new(|| NOW),
    );
    let refused = poll_once(no_secret.list_hub_peers(account(HOST), session(), Some(7))).unwrap();
    assert_eq!(code(refused.unwrap_err()), "P2P_ICE_UNAVAILABLE");
}

#[test]
fn the_join_bootstrap_appends_the_ticket_and_lobby_to_realms_fields() {
    let rig = rig();
    let bootstrap = poll_once(rig.admission.create_join_bootstrap(account(JOIN_A), session()))
        .unwrap()
        .unwrap();
    let json = bootstrap.to_json().0;
    assert!(json.starts_with(r#"{"coopTransport":"ICE","keyId":"test-key-1","sessionId":"01234567-89ab-4def-8123-456789abcdef","bridgeInstanceId":""#));
    assert!(json.contains(r#"","ticket":"mpt1."#));
    assert!(json.ends_with(r#","lobbyId":"ICE"}"#));
    let parsed: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["bridgeInstanceId"].as_str().unwrap().len(), 32);
    let empty = JoinBootstrap {
        common: Json("{}".to_owned()),
        ticket: Secret::new("t".to_owned()),
        lobby_id: "ICE".to_owned(),
    };
    assert_eq!(empty.to_json().0, r#"{"ticket":"t","lobbyId":"ICE"}"#);
    let no_lobby = rig_with(FakeHunts::new(AdmissionHunt {
        lobby_id: None,
        ..hunt(SessionKind::Hunt)
    }));
    let refused = poll_once(no_lobby.admission.create_join_bootstrap(account(JOIN_A), session())).unwrap();
    assert_eq!(code(refused.unwrap_err()), "P2P_SESSION_NOT_READY");
}
