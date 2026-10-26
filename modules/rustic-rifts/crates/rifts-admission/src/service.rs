//! The admission routes' operations, ported from `p2p/controllers/p2pAdmission.ts` (self ticket, consume,
//! disconnect), `realmHubAdmission.ts` (the hub's ticket and peer list) and `p2pLauncher.ts:CreateJoinBootstrap`.
//! Tickets and admissions are admission's ([`AdmissionStore`]); the hunt, its roster and the hub's presence rows are
//! realms' ([`AdmissionHunts`], RF5's). Node runs each operation in one Mongo transaction; here the writes are in the
//! same order, and the multi-table transaction waits on `store`'s API (see `rifts_seam`'s store notes).

use std::sync::Arc;

use rifts_relay::rules::{HuntSnapshot, PresenceRow, derive_pair_key};
use rifts_relay::wire::{JsonObject, hex, iso_time, json_array};
use rifts_relay::{HourlyCounter, P2PGate};
use rifts_seam::{AccountId, Actor, BoxFuture, Json, PeerId, Refusal, Secret, SessionId, UnixMillis};
use serde_json::Value;

use crate::store::{
    AdmissionStore, ConsumeOutcome, ConsumeRequest, DisconnectRequest, LinkStatus, PeerAdmissionRecord, PeerLinkRecord,
    TicketRecord, TicketStatus,
};
use crate::ticket::{MintedTicket, SessionUuid, TicketBindings, TicketError};
use crate::{AdmissionError, AdmissionService, TicketAuthority};

/// The protocol's maximum: a joiner may need to reach a cold relay route before the host consumes the ticket, and
/// generation fencing and single use stay authoritative.
pub const JOIN_TICKET_LIFETIME_SECONDS: u32 = 180;
/// The most peers a hub host's list carries (the runtime's hub ceiling).
pub const MAXIMUM_PEER_LIST: usize = 64;
const PEER_LISTS_PER_HOUR: u32 = 3_600;

pub fn refused(code: &str, message: &str) -> AdmissionError {
    AdmissionError::Refused(Refusal {
        code: code.to_owned(),
        message: message.to_owned(),
    })
}

/// The hunt as admission reads it: relay's snapshot, plus what a ticket binds and the host transport reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionHunt {
    pub snapshot: HuntSnapshot,
    pub session: SessionUuid,
    pub party_id: String,
    pub party_revision: u64,
    pub hunt_definition_id: String,
    pub build: u32,
    pub bridge_protocol: u16,
    pub virtual_port: u16,
    /// `ICE` for an ICE hunt, the Steam lobby otherwise; `None` before host-ready.
    pub lobby_id: Option<String>,
    /// The host transport's `bridgeInstanceId`, set at host-ready.
    pub bridge_instance_id: Option<String>,
}

impl AdmissionHunt {
    fn bindings(&self, account: &AccountId, joiner: PeerId) -> Result<TicketBindings, AdmissionError> {
        Ok(TicketBindings {
            account: account.clone(),
            joiner_peer: joiner,
            host_peer: self.snapshot.host_peer,
            session: self.session,
            generation: u32::try_from(self.snapshot.generation)
                .map_err(|_| AdmissionError::Ticket(TicketError::OutOfRange("generation")))?,
            party_id: self.party_id.clone(),
            party_revision: self.party_revision,
            hunt_definition_id: self.hunt_definition_id.clone(),
            build: self.build,
            bridge_protocol: self.bridge_protocol,
            virtual_port: self.virtual_port,
        })
    }
}

/// The host capability a transport presented (`P2PHostCapabilityRecord`), with the fields admission compares. The
/// seam's `RuntimeAuth` lacks the host peer and bridge instance; isles builds this from the capability record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostCapability {
    pub session: SessionId,
    pub wire_session: String,
    pub generation: u64,
    pub host_account: AccountId,
    pub host_peer: PeerId,
    pub bridge_instance_id: String,
}

/// What admission reads and writes in realms (RF5): the hunt and its roster, a hub's presence rows and the realm's
/// membership. Each write is a guarded update that answers whether it applied, as Node's repository does.
pub trait AdmissionHunts: Send + Sync {
    fn gate(&self) -> Result<P2PGate, AdmissionError>;
    fn find_hunt(&self, session: &SessionId) -> Result<Option<AdmissionHunt>, AdmissionError>;
    /// `reserveParticipantTicket`: within the rejoin window and the reconnect limit, the participant's next attempt.
    /// The hunt as it is after the reservation, or `None` when refused.
    fn reserve_participant(
        &self,
        hunt: &AdmissionHunt,
        account: &AccountId,
        now: UnixMillis,
    ) -> Result<Option<AdmissionHunt>, AdmissionError>;
    /// `HubTicketDenialReason` with the realm's ban and membership: the member's presence peer id, or the refusal
    /// `HubTicketDenialError` names. `link` is the account's peer link from admission's table.
    fn check_hub_entry(
        &self,
        hub: &AdmissionHunt,
        account: &AccountId,
        link: Option<&PeerLinkRecord>,
        now: UnixMillis,
    ) -> Result<PeerId, AdmissionError>;
    /// `markTicketed`: the presence row lives as long as the ticket.
    fn mark_hub_ticketed(
        &self,
        hub: &AdmissionHunt,
        account: &AccountId,
        now: UnixMillis,
        ticket_expires_at: UnixMillis,
    ) -> Result<bool, AdmissionError>;
    /// `markParticipantAdmitted`, then `ADMITTING` to `RUNNING` and the realm travel, as Node does after it.
    fn mark_participant_admitted(
        &self,
        hunt: &AdmissionHunt,
        account: &AccountId,
        admitted_at: UnixMillis,
    ) -> Result<bool, AdmissionError>;
    /// `AdmitHubPeer`: the member's presence row becomes `ADMITTED` (nothing for the host's own client).
    fn admit_hub_peer(
        &self,
        hub: &AdmissionHunt,
        account: &AccountId,
        admission_id: &str,
        admitted_at: UnixMillis,
    ) -> Result<(), AdmissionError>;
    /// After a disconnect applied: `markParticipantDisconnected`, or a hub member's presence row becomes `LOST`.
    fn peer_disconnected(&self, admission: &PeerAdmissionRecord, at: UnixMillis) -> Result<(), AdmissionError>;
    fn hub_presence(&self, hub: &AdmissionHunt, limit: usize) -> Result<Vec<PresenceRow>, AdmissionError>;
    /// `Common(hunt, bridgeInstanceId)` of `p2pLauncher.ts`: realms' bootstrap fields as a JSON object, or `None`
    /// when the rendezvous is incomplete.
    fn join_bootstrap_common(
        &self,
        hunt: &AdmissionHunt,
        key_id: &str,
        bridge_instance_id: &str,
    ) -> Result<Option<Json>, AdmissionError>;
}

/// `IssueSelfAdmissionTicket`'s answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedTicket {
    pub ticket: MintedTicket,
    pub hunt: AdmissionHunt,
}

impl IssuedTicket {
    pub fn to_json(&self) -> Json {
        let hunt = &self.hunt;
        let rendezvous = JsonObject::new()
            .string("sessionId", &hunt.snapshot.wire_session)
            .number("generation", hunt.snapshot.generation)
            .optional_string("lobbyId", hunt.lobby_id.as_deref())
            .string("hostSteamId", &hunt.snapshot.host_peer.0.to_string())
            .number("virtualPort", hunt.virtual_port)
            .number("bridgeProtocol", hunt.bridge_protocol)
            .number("build", hunt.build)
            .finish();
        Json(
            JsonObject::new()
                .string("ticket", self.ticket.text.expose())
                .string("expiresAt", &iso_seconds(self.ticket.claims.expires_at))
                .raw("rendezvous", &rendezvous)
                .finish(),
        )
    }
}

/// `CreateJoinBootstrap`'s answer: realms' common fields, then the ticket and the lobby.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinBootstrap {
    pub common: Json,
    pub ticket: Secret,
    pub lobby_id: String,
}

impl JoinBootstrap {
    /// `{ ...Common(hunt), ticket, lobbyId }`. Common has neither key, so the spread appends them.
    pub fn to_json(&self) -> Json {
        let common = self.common.0.trim();
        let open = common.strip_suffix('}').unwrap_or(common);
        let separator = if open.trim_end().ends_with('{') { "" } else { "," };
        Json(format!(
            "{open}{separator}\"ticket\":{},\"lobbyId\":{}}}",
            rifts_relay::wire::json_string(self.ticket.expose()),
            rifts_relay::wire::json_string(&self.lobby_id)
        ))
    }
}

/// `POST .../tickets/consume`, from the host's transport: Node's body checks done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumeTicket {
    pub ticket: Secret,
    /// The joiner's peer id as the host's transport authenticated it, as text.
    pub authenticated_peer: String,
    pub bridge_instance_id: String,
    pub connection_handle: String,
    /// The request's id, which makes a retry of the same consume idempotent.
    pub request_id: String,
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

impl ConsumeTicket {
    /// The body as `ConsumeAdmissionTicket` checks it: strings, the instance id 16 to 96 long, the handle 1 to 32.
    pub fn from_body(body: &Value, request_id: &str) -> Result<Self, AdmissionError> {
        let text = |key: &str| body.get(key).and_then(Value::as_str);
        let (Some(ticket), Some(peer), Some(bridge), Some(handle)) = (
            text("ticket"),
            text("authenticatedSteamId"),
            text("bridgeInstanceId"),
            text("connectionHandle"),
        ) else {
            return Err(malformed_consume());
        };
        if !(16..=96).contains(&utf16_len(bridge)) || !(1..=32).contains(&utf16_len(handle)) {
            return Err(malformed_consume());
        }
        Ok(Self {
            ticket: Secret::new(ticket.to_owned()),
            authenticated_peer: peer.to_owned(),
            bridge_instance_id: bridge.to_owned(),
            connection_handle: handle.to_owned(),
            request_id: request_id.to_owned(),
        })
    }
}

fn malformed_consume() -> AdmissionError {
    refused("P2P_TICKET_INVALID", "The admission request is malformed.")
}

/// `ConsumeAdmissionTicket`'s answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumedTicket {
    pub idempotent: bool,
    pub admission_id: String,
    pub account: AccountId,
    pub peer: PeerId,
    pub wire_session: String,
    pub generation: u32,
}

impl ConsumedTicket {
    pub fn to_json(&self) -> Json {
        Json(
            JsonObject::new()
                .boolean("admitted", true)
                .boolean("idempotent", self.idempotent)
                .string("admissionId", &self.admission_id)
                .string("accountId", &self.account.0)
                .string("steamId", &self.peer.0.to_string())
                .string("sessionId", &self.wire_session)
                .number("generation", self.generation)
                .finish(),
        )
    }
}

/// One peer a hub host should hold a link with (`AddPeerH`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubPeer {
    pub account: AccountId,
    pub peer: PeerId,
    /// 43 base64url characters.
    pub pair_key: Secret,
    pub state: &'static str,
}

/// `ListHubPeers`' answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubPeers {
    pub generation: u64,
    pub peers: Vec<HubPeer>,
}

impl HubPeers {
    pub fn to_json(&self) -> Json {
        let peers = json_array(self.peers.iter().map(|peer| {
            JsonObject::new()
                .string("accountId", &peer.account.0)
                .string("peerId", &peer.peer.0.to_string())
                .string("transportKey", peer.pair_key.expose())
                .string("state", peer.state)
                .finish()
        }));
        Json(
            JsonObject::new()
                .number("generation", self.generation)
                .raw("peers", &peers)
                .finish(),
        )
    }
}

fn iso_seconds(seconds: u64) -> String {
    iso_time(UnixMillis(
        i64::try_from(seconds).unwrap_or(i64::MAX / 1000).saturating_mul(1000),
    ))
}

/// The real [`AdmissionService`].
pub struct Admission {
    pub tickets: Arc<dyn TicketAuthority>,
    pub store: Arc<dyn AdmissionStore>,
    pub hunts: Arc<dyn AdmissionHunts>,
    /// `P2P_TRANSPORT_KEY_SECRET`, for the hub's pair keys.
    pub transport_key_secret: Option<Secret>,
    pub clock: Arc<dyn Fn() -> UnixMillis + Send + Sync>,
    peer_lists: HourlyCounter,
}

impl Admission {
    pub fn new(
        tickets: Arc<dyn TicketAuthority>,
        store: Arc<dyn AdmissionStore>,
        hunts: Arc<dyn AdmissionHunts>,
        transport_key_secret: Option<Secret>,
        clock: Arc<dyn Fn() -> UnixMillis + Send + Sync>,
    ) -> Self {
        Self {
            tickets,
            store,
            hunts,
            transport_key_secret,
            clock,
            peer_lists: HourlyCounter::default(),
        }
    }

    /// `AssertP2PAdmissionNotStopped`.
    fn not_stopped(&self, gate: P2PGate) -> Result<(), AdmissionError> {
        if gate.emergency_stop {
            return Err(refused(
                "P2P_DISABLED",
                "Player-hosted hunts are currently unavailable.",
            ));
        }
        Ok(())
    }

    fn record_ticket(&self, actor: &Actor, minted: &MintedTicket, hunt: &AdmissionHunt) -> Result<(), AdmissionError> {
        let claims = &minted.claims;
        let record = TicketRecord {
            jti_digest: minted.jti_digest.clone(),
            key_id: claims.key_id.0.clone(),
            session: hunt.snapshot.wire_session.clone(),
            generation: hunt.snapshot.generation,
            account: claims.bindings.account.clone(),
            joiner_peer: claims.bindings.joiner_peer,
            host_peer: claims.bindings.host_peer,
            bridge_protocol: claims.bindings.bridge_protocol,
            virtual_port: claims.bindings.virtual_port,
            issued_at: UnixMillis(i64::try_from(claims.issued_at).unwrap_or_default() * 1000),
            expires_at: UnixMillis(i64::try_from(claims.expires_at).unwrap_or_default() * 1000),
            status: TicketStatus::Issued,
            consumed_at: None,
            consumer_bridge_instance_id: None,
            consumer_request_id: None,
        };
        if self.store.insert_ticket(actor, &record)? {
            Ok(())
        } else {
            Err(AdmissionError::Conflict)
        }
    }

    /// `IssueHubAdmissionTicket`: one fresh ticket per entry, for an expected, still-eligible member.
    fn issue_hub_ticket(&self, hub: AdmissionHunt, account: &AccountId) -> Result<IssuedTicket, AdmissionError> {
        let now = (self.clock)();
        let link = self.store.peer_link(account)?;
        let peer = self.hunts.check_hub_entry(&hub, account, link.as_ref(), now)?;
        let minted = self
            .tickets
            .mint(hub.bindings(account, peer)?, now, Some(JOIN_TICKET_LIFETIME_SECONDS))?;
        let expires_at = UnixMillis(i64::try_from(minted.claims.expires_at).unwrap_or_default() * 1000);
        if !self.hunts.mark_hub_ticketed(&hub, account, now, expires_at)? {
            return Err(refused(
                "P2P_NOT_ROSTERED",
                "This account has no open entry into the hub.",
            ));
        }
        self.record_ticket(&Actor::Account(account.clone()), &minted, &hub)?;
        Ok(IssuedTicket {
            ticket: minted,
            hunt: hub,
        })
    }

    fn issue_self(&self, account: &AccountId, session: &SessionId) -> Result<IssuedTicket, AdmissionError> {
        let gate = self.hunts.gate()?;
        if !gate.enabled {
            return Err(refused("P2P_DISABLED", "Player-hosted hunts are currently disabled."));
        }
        self.not_stopped(gate)?;
        let hunt = self
            .hunts
            .find_hunt(session)?
            .ok_or_else(|| refused("P2P_SESSION_NOT_FOUND", "Hunt session not found."))?;
        if hunt.snapshot.is_hub() {
            return self.issue_hub_ticket(hunt, account);
        }
        let now = (self.clock)();
        if !hunt.snapshot.state.is_ticketable() || hunt.snapshot.expires_at <= now {
            return Err(refused(
                "P2P_SESSION_NOT_READY",
                "The hunt is not accepting participants.",
            ));
        }
        let participant = hunt
            .snapshot
            .roster
            .iter()
            .find(|entry| &entry.account == account)
            .ok_or_else(|| refused("P2P_NOT_ROSTERED", "This account is not in the frozen hunt roster."))?;
        let link_ok = self
            .store
            .peer_link(account)?
            .is_some_and(|link| link.status == LinkStatus::Verified && link.peer == participant.peer);
        if !link_ok {
            return Err(refused(
                "P2P_STEAM_LINK_REQUIRED",
                "The current Steam co-op transport no longer matches this hunt.",
            ));
        }
        let reserved = self.hunts.reserve_participant(&hunt, account, now)?.ok_or_else(|| {
            refused(
                "P2P_SESSION_NOT_READY",
                "The rejoin window or attempt limit has been reached.",
            )
        })?;
        let joiner = reserved
            .snapshot
            .roster
            .iter()
            .find(|entry| &entry.account == account)
            .map(|entry| entry.peer)
            .ok_or_else(|| refused("P2P_NOT_ROSTERED", "This account is not in the frozen hunt roster."))?;
        let minted = self.tickets.mint(
            reserved.bindings(account, joiner)?,
            now,
            Some(JOIN_TICKET_LIFETIME_SECONDS),
        )?;
        self.record_ticket(&Actor::Account(account.clone()), &minted, &reserved)?;
        Ok(IssuedTicket {
            ticket: minted,
            hunt: reserved,
        })
    }

    fn join_bootstrap(&self, account: &AccountId, session: &SessionId) -> Result<JoinBootstrap, AdmissionError> {
        if !self.hunts.gate()?.enabled {
            return Err(refused("P2P_DISABLED", "Player-hosted hunts are disabled."));
        }
        let issued = self.issue_self(account, session)?;
        let not_ready = || refused("P2P_SESSION_NOT_READY", "The host lobby is not ready.");
        let hunt = self.hunts.find_hunt(session)?.ok_or_else(not_ready)?;
        let lobby_id = hunt.lobby_id.clone().ok_or_else(not_ready)?;
        let mut instance = [0u8; 16];
        getrandom::fill(&mut instance).map_err(|_| AdmissionError::Ticket(TicketError::Random))?;
        let common = self
            .hunts
            .join_bootstrap_common(&hunt, &self.tickets.key_id().0, &hex(&instance))?
            .ok_or_else(not_ready)?;
        Ok(JoinBootstrap {
            common,
            ticket: issued.ticket.text,
            lobby_id,
        })
    }

    fn consume(&self, capability: &HostCapability, request: &ConsumeTicket) -> Result<ConsumedTicket, AdmissionError> {
        let gate = self.hunts.gate()?;
        if !gate.enabled {
            return Err(refused("P2P_DISABLED", "Player-hosted hunts are currently disabled."));
        }
        let now = (self.clock)();
        let claims = self.tickets.verify(request.ticket.expose(), now).map_err(|error| {
            let expired = matches!(error, AdmissionError::Ticket(ref ticket) if ticket.is_expired());
            let code = if expired {
                "P2P_TICKET_EXPIRED"
            } else {
                "P2P_TICKET_INVALID"
            };
            refused(code, "The admission ticket is invalid or expired.")
        })?;
        let bindings = &claims.bindings;
        if bindings.session.to_string() != capability.wire_session
            || bindings.joiner_peer.0.to_string() != request.authenticated_peer
        {
            return Err(refused(
                "P2P_TICKET_INVALID",
                "The ticket does not match the authenticated Steam connection.",
            ));
        }
        // AssertCapabilityMatchesTicket
        if capability.generation != u64::from(bindings.generation)
            || capability.host_peer != bindings.host_peer
            || capability.bridge_instance_id != request.bridge_instance_id
        {
            return Err(refused(
                "P2P_GENERATION_FENCED",
                "The admission ticket does not match the active host generation.",
            ));
        }
        self.not_stopped(gate)?;
        let hunt = self
            .hunts
            .find_hunt(&capability.session)?
            .ok_or_else(|| refused("P2P_SESSION_NOT_FOUND", "Hunt session not found."))?;
        let fenced = hunt.snapshot.generation != u64::from(bindings.generation)
            || hunt.snapshot.host_peer != bindings.host_peer
            || hunt.bridge_instance_id.as_deref() != Some(request.bridge_instance_id.as_str())
            || !hunt.snapshot.state.is_ticketable();
        if fenced {
            return Err(refused(
                "P2P_GENERATION_FENCED",
                "This hunt generation no longer accepts admission.",
            ));
        }
        let actor = Actor::HostRuntime {
            session: capability.session.clone(),
            host: capability.host_account.clone(),
        };
        let admission_id = claims.jti_digest();
        let outcome = self.store.consume_ticket(
            &actor,
            &ConsumeRequest {
                jti_digest: admission_id.clone(),
                session: capability.wire_session.clone(),
                generation: u64::from(bindings.generation),
                account: bindings.account.clone(),
                joiner_peer: bindings.joiner_peer,
                host_peer: bindings.host_peer,
                bridge_instance_id: request.bridge_instance_id.clone(),
                request_id: request.request_id.clone(),
                now,
            },
        )?;
        let idempotent = match outcome {
            ConsumeOutcome::Consumed(_) => false,
            ConsumeOutcome::Idempotent(_) => true,
            ConsumeOutcome::Expired => return Err(refused("P2P_TICKET_EXPIRED", "The admission ticket expired.")),
            ConsumeOutcome::Replay => {
                return Err(refused(
                    "P2P_TICKET_REPLAYED",
                    "The admission ticket was already consumed.",
                ));
            }
            ConsumeOutcome::Invalid => {
                return Err(refused(
                    "P2P_TICKET_INVALID",
                    "The admission ticket is not valid for this hunt.",
                ));
            }
        };
        let hub = hunt.snapshot.is_hub();
        if !idempotent {
            let active_key = format!(
                "{}:{}:{}",
                capability.wire_session, bindings.generation, bindings.joiner_peer.0
            );
            // A hub member back before its old link was reported closed would hit the one-live-connection key
            if hub {
                self.store.supersede_active(&actor, &active_key, now)?;
            }
            let recorded = self.store.insert_admission(
                &actor,
                &PeerAdmissionRecord {
                    admission_id: admission_id.clone(),
                    active_key: Some(active_key),
                    session: capability.wire_session.clone(),
                    generation: u64::from(bindings.generation),
                    account: bindings.account.clone(),
                    peer: bindings.joiner_peer,
                    bridge_instance_id: request.bridge_instance_id.clone(),
                    connection_handle: request.connection_handle.clone(),
                    admitted_at: now,
                    disconnected_at: None,
                    superseded_at: None,
                },
            )?;
            if !recorded {
                return Err(AdmissionError::Conflict);
            }
        }
        if hub {
            self.hunts
                .admit_hub_peer(&hunt, &bindings.account, &admission_id, now)?;
        } else if !self.hunts.mark_participant_admitted(&hunt, &bindings.account, now)? {
            return Err(refused(
                "P2P_GENERATION_FENCED",
                "The participant admission raced a fenced generation.",
            ));
        }
        Ok(ConsumedTicket {
            idempotent,
            admission_id,
            account: bindings.account.clone(),
            peer: bindings.joiner_peer,
            wire_session: capability.wire_session.clone(),
            generation: bindings.generation,
        })
    }

    fn disconnected(
        &self,
        capability: &HostCapability,
        admission_id: &str,
        disconnected_at: Option<UnixMillis>,
    ) -> Result<bool, AdmissionError> {
        let valid = admission_id.len() == 64 && admission_id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if !valid {
            return Err(refused("P2P_TICKET_INVALID", "Invalid peer admission identifier."));
        }
        let at = disconnected_at.unwrap_or_else(|| (self.clock)());
        self.not_stopped(self.hunts.gate()?)?;
        let Some(admission) = self.store.find_admission(admission_id)? else {
            return Ok(false);
        };
        if admission.session != capability.wire_session || admission.generation != capability.generation {
            return Ok(false);
        }
        let actor = Actor::HostRuntime {
            session: capability.session.clone(),
            host: capability.host_account.clone(),
        };
        let changed = self.store.disconnect(
            &actor,
            &DisconnectRequest {
                admission_id: admission_id.to_owned(),
                session: capability.wire_session.clone(),
                generation: capability.generation,
                bridge_instance_id: capability.bridge_instance_id.clone(),
                at,
            },
        )?;
        if changed {
            self.hunts.peer_disconnected(&admission, at)?;
        }
        Ok(changed)
    }

    fn hub_peers(
        &self,
        account: &AccountId,
        session: &SessionId,
        generation: Option<i64>,
    ) -> Result<HubPeers, AdmissionError> {
        let gate = self.hunts.gate()?;
        if !gate.enabled {
            return Err(refused("P2P_DISABLED", "Player-hosted hunts are disabled."));
        }
        if gate.emergency_stop {
            return Err(refused(
                "P2P_EMERGENCY_STOP",
                "Player-hosted hunts are currently stopped by operations policy.",
            ));
        }
        let generation = generation.ok_or_else(|| refused("P2P_TICKET_INVALID", "The hub generation is missing."))?;
        let secret = self.transport_key_secret.clone().ok_or_else(|| {
            refused(
                "P2P_ICE_UNAVAILABLE",
                "The co-op transport is not configured on this server.",
            )
        })?;
        let hub = self
            .hunts
            .find_hunt(session)?
            .filter(|hunt| hunt.snapshot.is_hub())
            .ok_or_else(|| refused("P2P_SESSION_NOT_FOUND", "Hub session not found."))?;
        let snapshot = &hub.snapshot;
        if &snapshot.host_account != account {
            return Err(refused("P2P_NOT_ROSTERED", "Only the hub's host may list its peers."));
        }
        if i64::try_from(snapshot.generation).ok() != Some(generation) {
            return Err(refused(
                "P2P_GENERATION_FENCED",
                "This request belongs to a stale hub generation.",
            ));
        }
        if !snapshot.state.is_ice_active() {
            return Err(refused("P2P_SESSION_NOT_READY", "The hub is not running."));
        }
        let key = format!("{}:{}", snapshot.wire_session, snapshot.generation);
        if !self.peer_lists.allow(&key, PEER_LISTS_PER_HOUR, (self.clock)()) {
            return Err(refused("P2P_SIGNAL_LIMIT", "Too many peer list requests for this hub."));
        }
        let rows = self.hunts.hub_presence(&hub, MAXIMUM_PEER_LIST + 1)?;
        let peers = rows
            .into_iter()
            .filter(|row| row.account != snapshot.host_account && row.state.is_live())
            .take(MAXIMUM_PEER_LIST)
            .map(|row| HubPeer {
                pair_key: derive_pair_key(
                    &secret,
                    &snapshot.wire_session,
                    snapshot.generation,
                    &snapshot.host_account,
                    &row.account,
                ),
                state: row.state.as_str(),
                account: row.account,
                peer: row.peer,
            })
            .collect();
        Ok(HubPeers {
            generation: snapshot.generation,
            peers,
        })
    }
}

impl AdmissionService for Admission {
    fn issue_self_ticket(
        &self,
        account: AccountId,
        session: SessionId,
    ) -> BoxFuture<'_, Result<IssuedTicket, AdmissionError>> {
        let result = self.issue_self(&account, &session);
        Box::pin(async move { result })
    }

    fn create_join_bootstrap(
        &self,
        account: AccountId,
        session: SessionId,
    ) -> BoxFuture<'_, Result<JoinBootstrap, AdmissionError>> {
        let result = self.join_bootstrap(&account, &session);
        Box::pin(async move { result })
    }

    fn consume_ticket(
        &self,
        capability: HostCapability,
        request: ConsumeTicket,
    ) -> BoxFuture<'_, Result<ConsumedTicket, AdmissionError>> {
        let result = self.consume(&capability, &request);
        Box::pin(async move { result })
    }

    fn record_peer_disconnected(
        &self,
        capability: HostCapability,
        admission_id: String,
        disconnected_at: Option<UnixMillis>,
    ) -> BoxFuture<'_, Result<bool, AdmissionError>> {
        let result = self.disconnected(&capability, &admission_id, disconnected_at);
        Box::pin(async move { result })
    }

    fn list_hub_peers(
        &self,
        account: AccountId,
        session: SessionId,
        generation: Option<i64>,
    ) -> BoxFuture<'_, Result<HubPeers, AdmissionError>> {
        let result = self.hub_peers(&account, &session, generation);
        Box::pin(async move { result })
    }
}

#[cfg(test)]
mod tests;
