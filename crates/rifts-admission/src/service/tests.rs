// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

use std::sync::Mutex;

use p256::ecdsa::SigningKey;
use p256::elliptic_curve::Generate as _;
use rifts_relay::rules::{CoopTransport, HuntState, PresenceState, RosterEntry, SessionKind};
use rifts_seam::{CapabilityScope, GlobalId, NodeId, Ulid, poll_once};
use serde_json::json;

use super::*;
use crate::KeyId;
use crate::store::MemoryAdmissionStore;
use crate::ticket::Es256Tickets;

const SESSION: &str = "01234567-89ab-4def-8123-456789abcdef";
const START: i64 = 1_793_000_000_000;
const HOST_PEER: PeerId = PeerId(2);
const JOINER_PEER: PeerId = PeerId(11);

fn host() -> AccountId {
    AccountId("host-0001".to_owned())
}

fn joiner() -> AccountId {
    AccountId("joiner-01".to_owned())
}

fn bridge() -> String {
    "b".repeat(16)
}

fn session_id() -> SessionId {
    SessionId(GlobalId {
        ulid: Ulid(1),
        node: NodeId::new("home").unwrap(),
    })
}

fn hunt(kind: SessionKind) -> AdmissionHunt {
    AdmissionHunt {
        snapshot: HuntSnapshot {
            wire_session: SESSION.to_owned(),
            generation: 3,
            state: HuntState::Admitting,
            kind,
            realm: None,
            coop_transport: CoopTransport::Ice,
            host_account: host(),
            host_peer: HOST_PEER,
            expires_at: UnixMillis(START + 3_600_000),
            roster: vec![
                RosterEntry {
                    account: host(),
                    peer: HOST_PEER,
                    returned_to_hub: false,
                },
                RosterEntry {
                    account: joiner(),
                    peer: JOINER_PEER,
                    returned_to_hub: false,
                },
            ],
        },
        session: SESSION.parse().unwrap(),
        party_id: "party_abc==".to_owned(),
        party_revision: 12,
        hunt_definition_id: "hunt/island-1".to_owned(),
        build: 392_819,
        bridge_protocol: 1,
        virtual_port: 3,
        lobby_id: Some("lobby-1".to_owned()),
        bridge_instance_id: Some(bridge()),
    }
}

/// Realms as a test sets it up, recording what admission asked of it.
struct FakeHunts {
    gate: Mutex<P2PGate>,
    hunt: Mutex<Option<AdmissionHunt>>,
    reserve: Mutex<bool>,
    hub_ticketed: Mutex<bool>,
    admitted: Mutex<bool>,
    presence: Mutex<Vec<PresenceRow>>,
    common: Mutex<Option<Json>>,
    calls: Mutex<Vec<String>>,
}

impl FakeHunts {
    fn new(kind: SessionKind) -> Self {
        Self {
            gate: Mutex::new(P2PGate {
                enabled: true,
                emergency_stop: false,
            }),
            hunt: Mutex::new(Some(hunt(kind))),
            reserve: Mutex::new(true),
            hub_ticketed: Mutex::new(true),
            admitted: Mutex::new(true),
            presence: Mutex::new(Vec::new()),
            common: Mutex::new(Some(Json("{\"realm\":\"r\"}".to_owned()))),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn log(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }

    fn edit(&self, change: impl FnOnce(&mut AdmissionHunt)) {
        change(self.hunt.lock().unwrap().as_mut().unwrap());
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
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
        self.log(format!("reserve {}", account.0));
        Ok((*self.reserve.lock().unwrap()).then(|| hunt.clone()))
    }

    fn check_hub_entry(
        &self,
        _: &AdmissionHunt,
        account: &AccountId,
        link: Option<&PeerLinkRecord>,
        _: UnixMillis,
    ) -> Result<PeerId, AdmissionError> {
        self.log(format!("entry {} linked={}", account.0, link.is_some()));
        if account == &joiner() {
            Ok(JOINER_PEER)
        } else {
            Err(refused("P2P_NOT_ROSTERED", "No entry."))
        }
    }

    fn mark_hub_ticketed(
        &self,
        _: &AdmissionHunt,
        account: &AccountId,
        now: UnixMillis,
        expires_at: UnixMillis,
    ) -> Result<bool, AdmissionError> {
        self.log(format!("ticketed {} {}", account.0, expires_at.0 - now.0));
        Ok(*self.hub_ticketed.lock().unwrap())
    }

    fn mark_participant_admitted(
        &self,
        _: &AdmissionHunt,
        account: &AccountId,
        _: UnixMillis,
    ) -> Result<bool, AdmissionError> {
        self.log(format!("admitted {}", account.0));
        Ok(*self.admitted.lock().unwrap())
    }

    fn admit_hub_peer(
        &self,
        _: &AdmissionHunt,
        account: &AccountId,
        admission_id: &str,
        _: UnixMillis,
    ) -> Result<(), AdmissionError> {
        self.log(format!("hub admitted {} {}", account.0, &admission_id[..8]));
        Ok(())
    }

    fn peer_disconnected(&self, admission: &PeerAdmissionRecord, _: UnixMillis) -> Result<(), AdmissionError> {
        self.log(format!("disconnected {}", admission.account.0));
        Ok(())
    }

    fn hub_presence(&self, _: &AdmissionHunt, limit: usize) -> Result<Vec<PresenceRow>, AdmissionError> {
        self.log(format!("presence {limit}"));
        Ok(self.presence.lock().unwrap().clone())
    }

    fn join_bootstrap_common(
        &self,
        _: &AdmissionHunt,
        key_id: &str,
        bridge_instance_id: &str,
    ) -> Result<Option<Json>, AdmissionError> {
        self.log(format!("common {key_id} {}", bridge_instance_id.len()));
        Ok(self.common.lock().unwrap().clone())
    }
}

struct Fixture {
    hunts: Arc<FakeHunts>,
    store: Arc<MemoryAdmissionStore>,
    tickets: Arc<Es256Tickets>,
    now: Arc<Mutex<i64>>,
    admission: Admission,
}

impl Fixture {
    fn new(kind: SessionKind) -> Self {
        let hunts = Arc::new(FakeHunts::new(kind));
        let store = Arc::new(MemoryAdmissionStore::default());
        store
            .put_peer_link(
                &Actor::System("test"),
                &PeerLinkRecord {
                    account: joiner(),
                    peer: JOINER_PEER,
                    status: LinkStatus::Verified,
                    revision: 1,
                    verified_at: UnixMillis(1),
                    verification_method: "steam".to_owned(),
                },
            )
            .unwrap();
        let signing = SigningKey::try_generate_from_rng(&mut rand::rngs::SysRng).unwrap();
        let tickets = Arc::new(Es256Tickets {
            key_id: KeyId("k1".to_owned()),
            verifying: *signing.verifying_key(),
            signing: Some(signing),
            default_lifetime: 75.0,
        });
        let now = Arc::new(Mutex::new(START));
        let clock_now = now.clone();
        let admission = Admission::new(
            tickets.clone(),
            store.clone(),
            hunts.clone(),
            Some(Secret::new("s".repeat(32))),
            Arc::new(move || UnixMillis(*clock_now.lock().unwrap())),
        );
        Self {
            hunts,
            store,
            tickets,
            now,
            admission,
        }
    }

    fn advance(&self, millis: i64) {
        *self.now.lock().unwrap() += millis;
    }

    fn issue(&self) -> Result<IssuedTicket, AdmissionError> {
        poll_once(self.admission.issue_self_ticket(joiner(), session_id())).unwrap()
    }

    fn capability(&self) -> HostCapability {
        HostCapability {
            session: session_id(),
            wire_session: SESSION.to_owned(),
            generation: 3,
            host_account: host(),
            host_peer: HOST_PEER,
            bridge_instance_id: bridge(),
        }
    }

    fn request(&self, ticket: &str, request_id: &str) -> ConsumeTicket {
        ConsumeTicket {
            ticket: Secret::new(ticket.to_owned()),
            authenticated_peer: JOINER_PEER.0.to_string(),
            bridge_instance_id: bridge(),
            connection_handle: "h1".to_owned(),
            request_id: request_id.to_owned(),
        }
    }

    fn consume_with(
        &self,
        capability: HostCapability,
        request: ConsumeTicket,
    ) -> Result<ConsumedTicket, AdmissionError> {
        poll_once(self.admission.consume_ticket(capability, request)).unwrap()
    }

    fn consume(&self, ticket: &str, request_id: &str) -> Result<ConsumedTicket, AdmissionError> {
        self.consume_with(self.capability(), self.request(ticket, request_id))
    }

    fn disconnect(&self, admission_id: &str) -> Result<bool, AdmissionError> {
        poll_once(
            self.admission
                .record_peer_disconnected(self.capability(), admission_id.to_owned(), None),
        )
        .unwrap()
    }

    fn peers(&self, account: AccountId, generation: Option<i64>) -> Result<HubPeers, AdmissionError> {
        poll_once(self.admission.list_hub_peers(account, session_id(), generation)).unwrap()
    }
}

fn code(result: Result<impl std::fmt::Debug, AdmissionError>) -> String {
    match result {
        Err(AdmissionError::Refused(refusal)) => format!("{}: {}", refusal.code, refusal.message),
        other => format!("{other:?}"),
    }
}

#[test]
fn a_joiner_is_ticketed_admitted_once_and_disconnected() {
    let fixture = Fixture::new(SessionKind::Hunt);
    let issued = fixture.issue().unwrap();
    let text = issued.ticket.text.expose().to_owned();
    assert_eq!(issued.ticket.claims.expires_at * 1000, (START + 180_000) as u64);
    let stored = fixture.store.ticket(&issued.ticket.jti_digest).unwrap();
    assert_eq!(stored.status, TicketStatus::Issued);
    assert_eq!(stored.session, SESSION);
    assert_eq!(stored.generation, 3);
    assert_eq!(stored.key_id, "k1");
    assert_eq!(stored.issued_at, UnixMillis(START));
    assert_eq!(stored.expires_at, UnixMillis(START + 180_000));
    assert_eq!(
        issued.to_json(),
        format!(
            "{{\"ticket\":\"{text}\",\"expiresAt\":\"2026-10-26T07:36:20.000Z\",\"rendezvous\":{{\"sessionId\":\"{SESSION}\",\
             \"generation\":3,\"lobbyId\":\"lobby-1\",\"hostSteamId\":\"2\",\"virtualPort\":3,\"bridgeProtocol\":1,\
             \"build\":392819}}}}"
        )
    );

    fixture.advance(1_000);
    let first = fixture.consume(&text, "r1").unwrap();
    assert!(!first.idempotent);
    assert_eq!(first.admission_id, issued.ticket.jti_digest);
    assert_eq!(
        first.to_json(),
        format!(
            "{{\"admitted\":true,\"idempotent\":false,\"admissionId\":\"{}\",\"accountId\":\"joiner-01\",\
             \"steamId\":\"11\",\"sessionId\":\"{SESSION}\",\"generation\":3}}",
            first.admission_id
        )
    );
    let admission = fixture.store.find_admission(&first.admission_id).unwrap().unwrap();
    assert_eq!(
        admission.active_key.as_deref(),
        Some(format!("{SESSION}:3:11").as_str())
    );
    assert_eq!(admission.connection_handle, "h1");

    let again = fixture.consume(&text, "r1").unwrap();
    assert_eq!(
        again,
        ConsumedTicket {
            idempotent: true,
            ..first.clone()
        }
    );
    assert_eq!(
        code(fixture.consume(&text, "r2")),
        "P2P_TICKET_REPLAYED: The admission ticket was already consumed."
    );

    assert_eq!(fixture.disconnect(&first.admission_id), Ok(true));
    assert_eq!(fixture.disconnect(&first.admission_id), Ok(false));
    assert_eq!(fixture.disconnect(&"0".repeat(64)), Ok(false));
    assert_eq!(
        fixture.hunts.calls(),
        [
            "reserve joiner-01",
            "admitted joiner-01",
            "admitted joiner-01",
            "disconnected joiner-01",
        ]
    );
}

#[test]
fn self_ticket_refusals_in_order() {
    let fixture = Fixture::new(SessionKind::Hunt);
    *fixture.hunts.gate.lock().unwrap() = P2PGate {
        enabled: false,
        emergency_stop: true,
    };
    assert_eq!(
        code(fixture.issue()),
        "P2P_DISABLED: Player-hosted hunts are currently disabled."
    );
    fixture.hunts.gate.lock().unwrap().enabled = true;
    assert_eq!(
        code(fixture.issue()),
        "P2P_DISABLED: Player-hosted hunts are currently unavailable."
    );
    fixture.hunts.gate.lock().unwrap().emergency_stop = false;

    let saved = fixture.hunts.hunt.lock().unwrap().take();
    assert_eq!(code(fixture.issue()), "P2P_SESSION_NOT_FOUND: Hunt session not found.");
    *fixture.hunts.hunt.lock().unwrap() = saved;

    fixture.hunts.edit(|hunt| hunt.snapshot.state = HuntState::Starting);
    let not_accepting = "P2P_SESSION_NOT_READY: The hunt is not accepting participants.";
    assert_eq!(code(fixture.issue()), not_accepting);
    fixture.hunts.edit(|hunt| {
        hunt.snapshot.state = HuntState::Running;
        hunt.snapshot.expires_at = UnixMillis(START);
    });
    assert_eq!(code(fixture.issue()), not_accepting);
    fixture
        .hunts
        .edit(|hunt| hunt.snapshot.expires_at = UnixMillis(START + 1));

    fixture
        .hunts
        .edit(|hunt| hunt.snapshot.roster.retain(|entry| entry.account != joiner()));
    assert_eq!(
        code(fixture.issue()),
        "P2P_NOT_ROSTERED: This account is not in the frozen hunt roster."
    );
    fixture.hunts.edit(|hunt| {
        hunt.snapshot.roster.push(RosterEntry {
            account: joiner(),
            peer: PeerId(12),
            returned_to_hub: false,
        });
    });
    let link_required = "P2P_STEAM_LINK_REQUIRED: The current Steam co-op transport no longer matches this hunt.";
    assert_eq!(code(fixture.issue()), link_required, "linked to another peer");
    fixture
        .hunts
        .edit(|hunt| hunt.snapshot.roster.last_mut().unwrap().peer = JOINER_PEER);
    let mut link = fixture.store.peer_link(&joiner()).unwrap().unwrap();
    link.status = LinkStatus::Revoked;
    fixture.store.put_peer_link(&Actor::System("test"), &link).unwrap();
    assert_eq!(code(fixture.issue()), link_required, "revoked");
    link.status = LinkStatus::Verified;
    fixture.store.put_peer_link(&Actor::System("test"), &link).unwrap();

    *fixture.hunts.reserve.lock().unwrap() = false;
    assert_eq!(
        code(fixture.issue()),
        "P2P_SESSION_NOT_READY: The rejoin window or attempt limit has been reached."
    );
    *fixture.hunts.reserve.lock().unwrap() = true;
    assert!(fixture.issue().is_ok());

    fixture
        .hunts
        .edit(|hunt| hunt.snapshot.generation = u64::from(u32::MAX) + 1);
    assert_eq!(
        fixture.issue(),
        Err(AdmissionError::Ticket(TicketError::OutOfRange("generation")))
    );
}

#[test]
fn consume_refusals_in_order() {
    let fixture = Fixture::new(SessionKind::Hunt);
    let text = fixture.issue().unwrap().ticket.text.expose().to_owned();
    let invalid_or_expired = "The admission ticket is invalid or expired.";

    fixture.hunts.gate.lock().unwrap().enabled = false;
    assert_eq!(
        code(fixture.consume(&text, "r")),
        "P2P_DISABLED: Player-hosted hunts are currently disabled."
    );
    fixture.hunts.gate.lock().unwrap().enabled = true;

    assert_eq!(
        code(fixture.consume("rpt1.x.y", "r")),
        format!("P2P_TICKET_INVALID: {invalid_or_expired}")
    );
    let mut wrong_peer = fixture.request(&text, "r");
    wrong_peer.authenticated_peer = "12".to_owned();
    let mismatch = "P2P_TICKET_INVALID: The ticket does not match the authenticated Steam connection.";
    assert_eq!(code(fixture.consume_with(fixture.capability(), wrong_peer)), mismatch);
    let other_session = HostCapability {
        wire_session: "01234567-89ab-4def-8123-456789abcdee".to_owned(),
        ..fixture.capability()
    };
    assert_eq!(
        code(fixture.consume_with(other_session, fixture.request(&text, "r"))),
        mismatch
    );

    let active = "P2P_GENERATION_FENCED: The admission ticket does not match the active host generation.";
    let stale = HostCapability {
        generation: 4,
        ..fixture.capability()
    };
    assert_eq!(code(fixture.consume_with(stale, fixture.request(&text, "r"))), active);
    let other_host = HostCapability {
        host_peer: PeerId(3),
        ..fixture.capability()
    };
    assert_eq!(
        code(fixture.consume_with(other_host, fixture.request(&text, "r"))),
        active
    );
    let mut other_bridge = fixture.request(&text, "r");
    other_bridge.bridge_instance_id = "c".repeat(16);
    assert_eq!(code(fixture.consume_with(fixture.capability(), other_bridge)), active);

    fixture.hunts.gate.lock().unwrap().emergency_stop = true;
    assert_eq!(
        code(fixture.consume(&text, "r")),
        "P2P_DISABLED: Player-hosted hunts are currently unavailable."
    );
    fixture.hunts.gate.lock().unwrap().emergency_stop = false;

    let saved = fixture.hunts.hunt.lock().unwrap().take();
    assert_eq!(
        code(fixture.consume(&text, "r")),
        "P2P_SESSION_NOT_FOUND: Hunt session not found."
    );
    *fixture.hunts.hunt.lock().unwrap() = saved;

    let no_longer = "P2P_GENERATION_FENCED: This hunt generation no longer accepts admission.";
    fixture.hunts.edit(|hunt| hunt.bridge_instance_id = None);
    assert_eq!(code(fixture.consume(&text, "r")), no_longer);
    fixture.hunts.edit(|hunt| {
        hunt.bridge_instance_id = Some(bridge());
        hunt.snapshot.state = HuntState::Draining;
    });
    assert_eq!(code(fixture.consume(&text, "r")), no_longer);
    fixture.hunts.edit(|hunt| hunt.snapshot.state = HuntState::Running);

    // Signed but never recorded by this server.
    let unrecorded = fixture
        .tickets
        .mint(
            hunt(SessionKind::Hunt).ticket_bindings(&joiner(), JOINER_PEER).unwrap(),
            (START / 1000) as u64,
            None,
        )
        .unwrap();
    assert_eq!(
        code(fixture.consume(unrecorded.text.expose(), "r")),
        "P2P_TICKET_INVALID: The admission ticket is not valid for this hunt."
    );

    *fixture.hunts.admitted.lock().unwrap() = false;
    assert_eq!(
        code(fixture.consume(&text, "r")),
        "P2P_GENERATION_FENCED: The participant admission raced a fenced generation."
    );
    // The ticket was consumed before realms refused; a retry is idempotent and refused again.
    assert_eq!(
        code(fixture.consume(&text, "r")),
        "P2P_GENERATION_FENCED: The participant admission raced a fenced generation."
    );
}

#[test]
fn expired_tickets() {
    let fixture = Fixture::new(SessionKind::Hunt);
    let text = fixture.issue().unwrap().ticket.text.expose().to_owned();
    fixture.advance(211_000);
    assert_eq!(
        code(fixture.consume(&text, "r")),
        "P2P_TICKET_EXPIRED: The admission ticket is invalid or expired."
    );
    // Within the clock skew the signature still verifies, but the stored ticket has expired.
    fixture.advance(-30_000);
    assert_eq!(
        code(fixture.consume(&text, "r")),
        "P2P_TICKET_EXPIRED: The admission ticket expired."
    );
}

#[test]
fn a_hub_member_gets_one_ticket_per_entry() {
    let fixture = Fixture::new(SessionKind::RealmHub);
    fixture.hunts.edit(|hub| hub.snapshot.state = HuntState::Running);
    let first = fixture.issue().unwrap();
    assert_eq!(first.ticket.claims.bindings.joiner_peer, JOINER_PEER);
    let first = fixture.consume(first.ticket.text.expose(), "r1").unwrap();

    fixture.advance(1_000);
    let second = fixture.issue().unwrap();
    let second = fixture.consume(second.ticket.text.expose(), "r2").unwrap();
    assert_ne!(first.admission_id, second.admission_id);
    let old = fixture.store.find_admission(&first.admission_id).unwrap().unwrap();
    assert_eq!(old.superseded_at, Some(UnixMillis(START + 1_000)));
    assert_eq!(old.active_key, None);
    let new = fixture.store.find_admission(&second.admission_id).unwrap().unwrap();
    assert!(new.active_key.is_some());

    let calls = fixture.hunts.calls();
    assert_eq!(calls[0], "entry joiner-01 linked=true");
    assert_eq!(calls[1], "ticketed joiner-01 180000");
    assert!(calls[2].starts_with("hub admitted joiner-01 "));
    assert_eq!(calls.len(), 6);

    *fixture.hunts.hub_ticketed.lock().unwrap() = false;
    assert_eq!(
        code(fixture.issue()),
        "P2P_NOT_ROSTERED: This account has no open entry into the hub."
    );
    let stranger = poll_once(
        fixture
            .admission
            .issue_self_ticket(AccountId("stranger1".to_owned()), session_id()),
    )
    .unwrap();
    assert_eq!(code(stranger), "P2P_NOT_ROSTERED: No entry.");
}

fn row(account: &str, peer: u64, state: PresenceState) -> PresenceRow {
    PresenceRow {
        account: AccountId(account.to_owned()),
        peer: PeerId(peer),
        state,
    }
}

#[test]
fn a_hub_host_lists_live_peers() {
    let fixture = Fixture::new(SessionKind::RealmHub);
    fixture.hunts.edit(|hub| hub.snapshot.state = HuntState::Running);
    *fixture.hunts.presence.lock().unwrap() = vec![
        row("host-0001", 2, PresenceState::Admitted),
        row("joiner-01", 11, PresenceState::Admitted),
        row("joiner-02", 12, PresenceState::Lost),
        row("joiner-03", 13, PresenceState::Pending),
        row("joiner-04", 14, PresenceState::Left),
        row("joiner-05", 15, PresenceState::Ticketed),
    ];
    let peers = fixture.peers(host(), Some(3)).unwrap();
    assert_eq!(peers.generation, 3);
    let listed: Vec<(&str, u64, &str)> = peers
        .peers
        .iter()
        .map(|peer| (peer.account.0.as_str(), peer.peer.0, peer.state))
        .collect();
    assert_eq!(
        listed,
        [
            ("joiner-01", 11, "ADMITTED"),
            ("joiner-03", 13, "PENDING"),
            ("joiner-05", 15, "TICKETED")
        ]
    );
    for peer in &peers.peers {
        assert_eq!(peer.pair_key.expose().len(), 43);
        let expected = derive_pair_key(&Secret::new("s".repeat(32)), SESSION, 3, &host(), &peer.account);
        assert_eq!(peer.pair_key, expected);
    }
    assert_eq!(fixture.hunts.calls(), ["presence 65"]);
    let first_key = peers.peers[0].pair_key.expose();
    assert!(peers.to_json().starts_with(&format!(
        "{{\"generation\":3,\"peers\":[{{\"accountId\":\"joiner-01\",\"peerId\":\"11\",\"transportKey\":\"{first_key}\",\
         \"state\":\"ADMITTED\"}},{{\"accountId\":\"joiner-03\""
    )));

    *fixture.hunts.presence.lock().unwrap() = (0..80)
        .map(|n| row(&format!("member-{n:03}"), 100 + n, PresenceState::Admitted))
        .collect();
    assert_eq!(fixture.peers(host(), Some(3)).unwrap().peers.len(), 64);
}

#[test]
fn hub_peer_refusals_in_order() {
    let fixture = Fixture::new(SessionKind::RealmHub);
    fixture.hunts.edit(|hub| hub.snapshot.state = HuntState::Running);
    *fixture.hunts.gate.lock().unwrap() = P2PGate {
        enabled: false,
        emergency_stop: true,
    };
    assert_eq!(
        code(fixture.peers(host(), Some(3))),
        "P2P_DISABLED: Player-hosted hunts are disabled."
    );
    fixture.hunts.gate.lock().unwrap().enabled = true;
    assert_eq!(
        code(fixture.peers(host(), Some(3))),
        "P2P_EMERGENCY_STOP: Player-hosted hunts are currently stopped by operations policy."
    );
    fixture.hunts.gate.lock().unwrap().emergency_stop = false;
    assert_eq!(
        code(fixture.peers(host(), None)),
        "P2P_TICKET_INVALID: The hub generation is missing."
    );

    let keyless = Admission::new(
        fixture.tickets.clone(),
        fixture.store.clone(),
        fixture.hunts.clone(),
        None,
        Arc::new(|| UnixMillis(START)),
    );
    assert_eq!(
        code(poll_once(keyless.list_hub_peers(host(), session_id(), Some(3))).unwrap()),
        "P2P_ICE_UNAVAILABLE: The co-op transport is not configured on this server."
    );

    fixture.hunts.edit(|hub| hub.snapshot.kind = SessionKind::Hunt);
    assert_eq!(
        code(fixture.peers(host(), Some(3))),
        "P2P_SESSION_NOT_FOUND: Hub session not found."
    );
    fixture.hunts.edit(|hub| hub.snapshot.kind = SessionKind::RealmHub);
    assert_eq!(
        code(fixture.peers(joiner(), Some(3))),
        "P2P_NOT_ROSTERED: Only the hub's host may list its peers."
    );
    assert_eq!(
        code(fixture.peers(host(), Some(2))),
        "P2P_GENERATION_FENCED: This request belongs to a stale hub generation."
    );
    fixture.hunts.edit(|hub| hub.snapshot.state = HuntState::Completed);
    assert_eq!(
        code(fixture.peers(host(), Some(3))),
        "P2P_SESSION_NOT_READY: The hub is not running."
    );
    fixture.hunts.edit(|hub| hub.snapshot.state = HuntState::Starting);
    assert!(fixture.peers(host(), Some(3)).is_ok());
}

#[test]
fn hub_peer_lists_are_rate_limited() {
    let fixture = Fixture::new(SessionKind::RealmHub);
    fixture.hunts.edit(|hub| hub.snapshot.state = HuntState::Running);
    for _ in 0..3600 {
        assert!(fixture.peers(host(), Some(3)).is_ok());
    }
    assert_eq!(
        code(fixture.peers(host(), Some(3))),
        "P2P_SIGNAL_LIMIT: Too many peer list requests for this hub."
    );
    fixture.advance(3_600_000);
    assert!(fixture.peers(host(), Some(3)).is_ok());
}

#[test]
fn join_bootstrap() {
    let fixture = Fixture::new(SessionKind::Hunt);
    let bootstrap = poll_once(fixture.admission.create_join_bootstrap(joiner(), session_id()))
        .unwrap()
        .unwrap();
    assert_eq!(bootstrap.lobby_id, "lobby-1");
    assert_eq!(
        bootstrap.to_json(),
        format!(
            "{{\"realm\":\"r\",\"ticket\":\"{}\",\"lobbyId\":\"lobby-1\"}}",
            bootstrap.ticket.expose()
        )
    );
    assert_eq!(fixture.hunts.calls(), ["reserve joiner-01", "common k1 32"]);

    let bootstrap =
        |fixture: &Fixture| poll_once(fixture.admission.create_join_bootstrap(joiner(), session_id())).unwrap();
    *fixture.hunts.common.lock().unwrap() = None;
    let not_ready = "P2P_SESSION_NOT_READY: The host lobby is not ready.";
    assert_eq!(code(bootstrap(&fixture)), not_ready);
    fixture.hunts.edit(|hunt| hunt.lobby_id = None);
    assert_eq!(code(bootstrap(&fixture)), not_ready);
    fixture.hunts.gate.lock().unwrap().enabled = false;
    assert_eq!(
        code(bootstrap(&fixture)),
        "P2P_DISABLED: Player-hosted hunts are disabled."
    );
}

#[test]
fn join_bootstrap_text() {
    let with = |common: &str| {
        JoinBootstrap {
            common: Json(common.to_owned()),
            ticket: Secret::new("t\"1".to_owned()),
            lobby_id: "L".to_owned(),
        }
        .to_json()
    };
    assert_eq!(with("{\"a\":1}"), "{\"a\":1,\"ticket\":\"t\\\"1\",\"lobbyId\":\"L\"}");
    assert_eq!(with("{}"), "{\"ticket\":\"t\\\"1\",\"lobbyId\":\"L\"}");
    assert_eq!(with("  { \n } \n"), "{ \n \"ticket\":\"t\\\"1\",\"lobbyId\":\"L\"}");
    assert_eq!(with(" {\"a\":1} "), "{\"a\":1,\"ticket\":\"t\\\"1\",\"lobbyId\":\"L\"}");
}

#[test]
fn disconnect_rules() {
    let fixture = Fixture::new(SessionKind::Hunt);
    let text = fixture.issue().unwrap().ticket.text.expose().to_owned();
    let admitted = fixture.consume(&text, "r1").unwrap();
    let invalid = "P2P_TICKET_INVALID: Invalid peer admission identifier.";
    assert_eq!(code(fixture.disconnect(&admitted.admission_id.to_uppercase())), invalid);
    assert_eq!(code(fixture.disconnect("abc")), invalid);

    fixture.hunts.gate.lock().unwrap().emergency_stop = true;
    assert_eq!(
        code(fixture.disconnect(&admitted.admission_id)),
        "P2P_DISABLED: Player-hosted hunts are currently unavailable."
    );
    *fixture.hunts.gate.lock().unwrap() = P2PGate {
        enabled: false,
        emergency_stop: false,
    };
    let stale = HostCapability {
        generation: 4,
        ..fixture.capability()
    };
    let stale = poll_once(
        fixture
            .admission
            .record_peer_disconnected(stale, admitted.admission_id.clone(), None),
    )
    .unwrap();
    assert_eq!(stale, Ok(false));
    let at = poll_once(fixture.admission.record_peer_disconnected(
        fixture.capability(),
        admitted.admission_id.clone(),
        Some(UnixMillis(42)),
    ))
    .unwrap();
    assert_eq!(at, Ok(true), "disabled hosting still records a disconnect");
    let record = fixture.store.find_admission(&admitted.admission_id).unwrap().unwrap();
    assert_eq!(record.disconnected_at, Some(UnixMillis(42)));
}

#[test]
fn consume_bodies() {
    let body = |bridge: &str, handle: &str| {
        json!({
            "ticket": "t",
            "authenticatedSteamId": "11",
            "bridgeInstanceId": bridge,
            "connectionHandle": handle,
        })
    };
    let parsed = ConsumeTicket::from_body(&body(&"b".repeat(16), "h"), "req").unwrap();
    assert_eq!(parsed.request_id, "req");
    assert_eq!(parsed.ticket.expose(), "t");
    assert_eq!(parsed.authenticated_peer, "11");
    let malformed = "P2P_TICKET_INVALID: The admission request is malformed.";
    assert_eq!(
        code(ConsumeTicket::from_body(&body(&"b".repeat(15), "h"), "r")),
        malformed
    );
    assert!(ConsumeTicket::from_body(&body(&"b".repeat(96), "h"), "r").is_ok());
    assert_eq!(
        code(ConsumeTicket::from_body(&body(&"b".repeat(97), "h"), "r")),
        malformed
    );
    assert_eq!(
        code(ConsumeTicket::from_body(&body(&"b".repeat(16), ""), "r")),
        malformed
    );
    assert!(ConsumeTicket::from_body(&body(&"b".repeat(16), &"h".repeat(32)), "r").is_ok());
    assert_eq!(
        code(ConsumeTicket::from_body(&body(&"b".repeat(16), &"h".repeat(33)), "r")),
        malformed
    );
    // Counted in UTF-16 units: 16 astral characters are 32 units.
    assert_eq!(
        code(ConsumeTicket::from_body(
            &body(&"b".repeat(16), &"\u{1F600}".repeat(17)),
            "r"
        )),
        malformed
    );
    let mut numeric = body(&"b".repeat(16), "h");
    numeric["authenticatedSteamId"] = json!(11);
    assert_eq!(code(ConsumeTicket::from_body(&numeric, "r")), malformed);
    assert_eq!(code(ConsumeTicket::from_body(&json!({}), "r")), malformed);
}

#[test]
fn capability_from_runtime() {
    let mut runtime = RuntimeAuth {
        capability_id: "c".to_owned(),
        session: session_id(),
        generation: 3,
        host_account: host(),
        scopes: vec![CapabilityScope::PeerAdmitted],
        roster: Vec::new(),
        host_peer: Some(HOST_PEER),
        bridge_instance: Some(bridge()),
        wire_session: None,
    };
    let capability = HostCapability::from_runtime(&runtime).unwrap();
    assert_eq!(capability.wire_session, Ulid(1).to_string());
    assert_eq!(capability.wire_session.len(), 26);
    assert_eq!(capability.host_peer, HOST_PEER);
    runtime.wire_session = Some(SESSION.to_owned());
    assert_eq!(
        HostCapability::from_runtime(&runtime).unwrap(),
        Fixture::new(SessionKind::Hunt).capability()
    );
    runtime.host_peer = None;
    assert_eq!(HostCapability::from_runtime(&runtime), None);
    runtime.host_peer = Some(HOST_PEER);
    runtime.bridge_instance = None;
    assert_eq!(HostCapability::from_runtime(&runtime), None);
}

#[test]
fn each_issue_is_a_new_ticket_row() {
    let fixture = Fixture::new(SessionKind::Hunt);
    let first = fixture.issue().unwrap();
    let second = fixture.issue().unwrap();
    assert_ne!(first.ticket.jti_digest, second.ticket.jti_digest);
    assert!(fixture.store.ticket(&first.ticket.jti_digest).is_some());
    assert!(fixture.store.ticket(&second.ticket.jti_digest).is_some());
}
