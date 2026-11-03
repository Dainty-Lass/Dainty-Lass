// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The admission operations: a ticket for one's own seat, the launcher's join bootstrap, the host's one-time consume,
//! a peer's disconnect, and a hub host's list of peers with their pair keys.

use std::{future::ready, sync::Arc};

use rand::TryRng as _;
use rifts_relay::{
    HourlyCounter, P2PGate,
    rules::{HuntSnapshot, PresenceRow, derive_pair_key},
    wire::{self, JsonObject},
};
use rifts_seam::{AccountId, Actor, Json, PeerId, RuntimeAuth, Secret, SessionId, UnixMillis};
use serde_json::Value;

use crate::{
    AdmissionError, AdmissionService, Answer, TicketAuthority,
    store::{
        AdmissionStore, ConsumeOutcome, ConsumeRequest, DisconnectRequest, LinkStatus, PeerAdmissionRecord,
        PeerLinkRecord, TicketRecord, TicketStatus,
    },
    ticket::{MintedTicket, SessionUuid, TicketBindings, TicketError},
};

#[cfg(test)]
mod tests;

pub const JOIN_TICKET_LIFETIME_SECONDS: u32 = 180;
pub const MAXIMUM_PEER_LIST: usize = 64;
const PEER_LISTS_PER_HOUR: u32 = 3600;

type Outcome<T> = Result<T, AdmissionError>;

pub fn refused(code: &str, message: &str) -> AdmissionError {
    rifts_seam::Refusal {
        code: code.into(),
        message: message.into(),
    }
    .into()
}

/// A refusal admission answers with: its code, then its text.
#[derive(Clone, Copy)]
struct No(&'static str, &'static str);

impl From<No> for AdmissionError {
    fn from(No(code, text): No) -> Self {
        refused(code, text)
    }
}

/// Every refusal, by what it says; several share a code.
macro_rules! refusals {
    ($($name:ident: $code:literal => $text:literal,)*) => {$(
        const $name: No = No($code, $text);
    )*};
}

refusals! {
    SWITCHED_OFF: "P2P_DISABLED" => "Player-hosted hunts are currently disabled.",
    OFF: "P2P_DISABLED" => "Player-hosted hunts are disabled.",
    UNAVAILABLE: "P2P_DISABLED" => "Player-hosted hunts are currently unavailable.",
    STOPPED: "P2P_EMERGENCY_STOP" => "Player-hosted hunts are currently stopped by operations policy.",
    NO_HUNT: "P2P_SESSION_NOT_FOUND" => "Hunt session not found.",
    NO_HUB: "P2P_SESSION_NOT_FOUND" => "Hub session not found.",
    NOT_ACCEPTING: "P2P_SESSION_NOT_READY" => "The hunt is not accepting participants.",
    REJOIN_SPENT: "P2P_SESSION_NOT_READY" => "The rejoin window or attempt limit has been reached.",
    NO_LOBBY: "P2P_SESSION_NOT_READY" => "The host lobby is not ready.",
    HUB_IDLE: "P2P_SESSION_NOT_READY" => "The hub is not running.",
    NO_HUB_ENTRY: "P2P_NOT_ROSTERED" => "This account has no open entry into the hub.",
    NOT_ROSTERED: "P2P_NOT_ROSTERED" => "This account is not in the frozen hunt roster.",
    NOT_HUB_HOST: "P2P_NOT_ROSTERED" => "Only the hub's host may list its peers.",
    LINK_MISMATCH: "P2P_STEAM_LINK_REQUIRED" => "The current Steam co-op transport no longer matches this hunt.",
    MALFORMED: "P2P_TICKET_INVALID" => "The admission request is malformed.",
    UNREADABLE_TICKET: "P2P_TICKET_INVALID" => "The admission ticket is invalid or expired.",
    LAPSED_TICKET: "P2P_TICKET_EXPIRED" => "The admission ticket is invalid or expired.",
    OTHER_CONNECTION: "P2P_TICKET_INVALID" => "The ticket does not match the authenticated Steam connection.",
    WRONG_HUNT: "P2P_TICKET_INVALID" => "The admission ticket is not valid for this hunt.",
    BAD_ADMISSION_ID: "P2P_TICKET_INVALID" => "Invalid peer admission identifier.",
    NO_GENERATION: "P2P_TICKET_INVALID" => "The hub generation is missing.",
    EXPIRED: "P2P_TICKET_EXPIRED" => "The admission ticket expired.",
    REPLAYED: "P2P_TICKET_REPLAYED" => "The admission ticket was already consumed.",
    OTHER_GENERATION: "P2P_GENERATION_FENCED" => "The admission ticket does not match the active host generation.",
    FENCED_HUNT: "P2P_GENERATION_FENCED" => "This hunt generation no longer accepts admission.",
    RACED: "P2P_GENERATION_FENCED" => "The participant admission raced a fenced generation.",
    STALE_HUB: "P2P_GENERATION_FENCED" => "This request belongs to a stale hub generation.",
    NO_TRANSPORT: "P2P_ICE_UNAVAILABLE" => "The co-op transport is not configured on this server.",
    LIST_LIMIT: "P2P_SIGNAL_LIMIT" => "Too many peer list requests for this hub.",
}

/// `Err(no)` unless `holds`.
fn ensure(holds: bool, no: No) -> Outcome<()> {
    if holds { Ok(()) } else { Err(no.into()) }
}

/// A hunt or hub as admission sees it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdmissionHunt {
    pub snapshot: HuntSnapshot,
    pub session: SessionUuid,
    pub party_id: String,
    pub party_revision: u64,
    pub hunt_definition_id: String,
    pub build: u32,
    pub bridge_protocol: u16,
    pub virtual_port: u16,
    pub lobby_id: Option<String>,
    pub bridge_instance_id: Option<String>,
}

impl AdmissionHunt {
    fn seat_for(&self, account: &AccountId) -> Option<PeerId> {
        let seat = self.snapshot.roster.iter().find(|seat| seat.account == *account)?;
        Some(seat.peer)
    }

    /// Everything a ticket for `joiner` on this hunt is bound to.
    fn ticket_bindings(&self, account: &AccountId, joiner: PeerId) -> Outcome<TicketBindings> {
        let Self {
            ref snapshot,
            session,
            ref party_id,
            party_revision,
            ref hunt_definition_id,
            build,
            bridge_protocol,
            virtual_port,
            ..
        } = *self;
        let Ok(generation) = snapshot.generation.try_into() else {
            return Err(TicketError::OutOfRange("generation").into());
        };
        Ok(TicketBindings {
            generation,
            session,
            build,
            bridge_protocol,
            virtual_port,
            party_revision,
            account: account.clone(),
            joiner_peer: joiner,
            host_peer: snapshot.host_peer,
            party_id: party_id.clone(),
            hunt_definition_id: hunt_definition_id.clone(),
        })
    }
}

/// What a host runtime proved about itself: its hunt, generation, peer and bridge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostCapability {
    pub session: SessionId,
    pub wire_session: String,
    pub generation: u64,
    pub host_account: AccountId,
    pub host_peer: PeerId,
    pub bridge_instance_id: String,
}

impl HostCapability {
    pub fn from_runtime(runtime: &RuntimeAuth) -> Option<Self> {
        let (host_peer, bridge) = (runtime.host_peer?, runtime.bridge_instance.as_ref()?);
        let wire_session = match &runtime.wire_session {
            Some(text) => text.clone(),
            None => runtime.session.0.ulid.to_string(),
        };
        Some(Self {
            host_peer,
            bridge_instance_id: bridge.clone(),
            wire_session,
            session: runtime.session.clone(),
            host_account: runtime.host_account.clone(),
            generation: runtime.generation,
        })
    }

    fn actor(&self) -> Actor {
        Actor::HostRuntime {
            host: self.host_account.clone(),
            session: self.session.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssuedTicket {
    pub ticket: MintedTicket,
    pub hunt: AdmissionHunt,
}

/// Unix seconds as milliseconds, saturating.
fn millis(seconds: u64) -> UnixMillis {
    let seconds = i64::try_from(seconds).unwrap_or(i64::MAX);
    UnixMillis(seconds.saturating_mul(1000))
}

impl IssuedTicket {
    pub fn to_json(&self) -> String {
        let IssuedTicket { ticket, hunt } = self;
        let shot = &hunt.snapshot;
        let rendezvous = JsonObject::new()
            .string("sessionId", &shot.wire_session)
            .number("generation", shot.generation)
            .optional_string("lobbyId", hunt.lobby_id.as_deref())
            .string("hostSteamId", &shot.host_peer.0.to_string())
            .number("virtualPort", hunt.virtual_port)
            .number("bridgeProtocol", hunt.bridge_protocol)
            .number("build", hunt.build);
        let expires = wire::iso_time(millis(ticket.claims.expires_at));
        JsonObject::new()
            .string("ticket", ticket.text.expose())
            .string("expiresAt", &expires)
            .raw("rendezvous", &rendezvous.finish())
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JoinBootstrap {
    /// Realms' fields, as one JSON object.
    pub common: Json,
    pub ticket: Secret,
    pub lobby_id: String,
}

impl JoinBootstrap {
    /// Realms' object with `ticket` and `lobbyId` appended.
    pub fn to_json(&self) -> String {
        let whole = self.common.0.trim();
        let open = whole.strip_suffix('}').unwrap_or(whole);
        let comma = if open.trim_end().ends_with('{') { "" } else { "," };
        let (ticket, lobby) = (
            wire::json_string(self.ticket.expose()),
            wire::json_string(&self.lobby_id),
        );
        format!("{open}{comma}\"ticket\":{ticket},\"lobbyId\":{lobby}}}")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsumeTicket {
    pub ticket: Secret,
    pub authenticated_peer: String,
    pub bridge_instance_id: String,
    pub connection_handle: String,
    pub request_id: String,
}

impl ConsumeTicket {
    pub fn from_body(body: &Value, request_id: &str) -> Result<Self, AdmissionError> {
        let field = |name: &str| {
            body.get(name)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or(MALFORMED)
        };
        let request = Self {
            ticket: Secret::new(field("ticket")?),
            authenticated_peer: field("authenticatedSteamId")?,
            bridge_instance_id: field("bridgeInstanceId")?,
            connection_handle: field("connectionHandle")?,
            request_id: request_id.to_owned(),
        };
        // Lengths in UTF-16 units, as JavaScript counts them.
        let units = |text: &String| text.encode_utf16().count();
        let sized = (16..=96).contains(&units(&request.bridge_instance_id))
            && (1..=32).contains(&units(&request.connection_handle));
        ensure(sized, MALFORMED).map(|()| request)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsumedTicket {
    pub idempotent: bool,
    pub admission_id: String,
    pub account: AccountId,
    pub peer: PeerId,
    pub wire_session: String,
    pub generation: u32,
}

impl ConsumedTicket {
    pub fn to_json(&self) -> String {
        let Self {
            idempotent,
            admission_id,
            account,
            peer,
            wire_session,
            generation,
        } = self;
        JsonObject::new()
            .boolean("admitted", true)
            .boolean("idempotent", *idempotent)
            .string("admissionId", admission_id)
            .string("accountId", &account.0)
            .string("steamId", &peer.0.to_string())
            .string("sessionId", wire_session)
            .number("generation", generation)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HubPeer {
    pub account: AccountId,
    pub peer: PeerId,
    pub pair_key: Secret,
    pub state: &'static str,
}

impl HubPeer {
    fn json(&self) -> String {
        let peer = self.peer.0.to_string();
        JsonObject::new()
            .string("accountId", &self.account.0)
            .string("peerId", &peer)
            .string("transportKey", self.pair_key.expose())
            .string("state", self.state)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HubPeers {
    pub generation: u64,
    pub peers: Vec<HubPeer>,
}

impl HubPeers {
    pub fn to_json(&self) -> String {
        let peers = wire::json_array(self.peers.iter().map(HubPeer::json));
        JsonObject::new()
            .number("generation", self.generation)
            .raw("peers", &peers)
            .finish()
    }
}

/// What admission needs from realms.
pub trait AdmissionHunts: Send + Sync {
    fn gate(&self) -> Outcome<P2PGate>;
    fn find_hunt(&self, id: &SessionId) -> Outcome<Option<AdmissionHunt>>;
    /// The participant's next attempt within the rejoin window and limit; `None` when refused.
    fn reserve_participant(
        &self,
        on: &AdmissionHunt,
        who: &AccountId,
        at: UnixMillis,
    ) -> Outcome<Option<AdmissionHunt>>;
    /// The member's peer id, or the refusal.
    fn check_hub_entry(
        &self,
        on: &AdmissionHunt,
        who: &AccountId,
        verified_link: Option<&PeerLinkRecord>,
        at: UnixMillis,
    ) -> Outcome<PeerId>;
    fn mark_hub_ticketed(
        &self,
        on: &AdmissionHunt,
        who: &AccountId,
        at: UnixMillis,
        ticket_expiry: UnixMillis,
    ) -> Outcome<bool>;
    fn mark_participant_admitted(&self, on: &AdmissionHunt, who: &AccountId, at: UnixMillis) -> Outcome<bool>;
    fn admit_hub_peer(&self, on: &AdmissionHunt, who: &AccountId, admission: &str, at: UnixMillis) -> Outcome<()>;
    fn peer_disconnected(&self, row: &PeerAdmissionRecord, at: UnixMillis) -> Outcome<()>;
    fn hub_presence(&self, on: &AdmissionHunt, most: usize) -> Outcome<Vec<PresenceRow>>;
    /// Realms' bootstrap fields as one JSON object, or `None`.
    fn join_bootstrap_common(&self, on: &AdmissionHunt, key: &str, bridge: &str) -> Outcome<Option<Json>>;
}

pub struct Admission {
    pub tickets: Arc<dyn TicketAuthority>,
    pub store: Arc<dyn AdmissionStore>,
    pub hunts: Arc<dyn AdmissionHunts>,
    pub transport_key_secret: Option<Secret>,
    pub clock: Arc<dyn Fn() -> UnixMillis + Send + Sync>,
    list_calls: HourlyCounter,
}

/// The row an issued ticket is remembered by: its digest and bindings, never its text.
fn issued_row(hunt: &AdmissionHunt, minted: &MintedTicket) -> TicketRecord {
    let MintedTicket { claims, jti_digest, .. } = minted;
    let TicketBindings {
        account,
        joiner_peer,
        host_peer,
        bridge_protocol,
        virtual_port,
        ..
    } = &claims.bindings;
    TicketRecord {
        status: TicketStatus::Issued,
        jti_digest: jti_digest.clone(),
        key_id: claims.key_id.0.clone(),
        session: hunt.snapshot.wire_session.clone(),
        generation: hunt.snapshot.generation,
        account: account.clone(),
        joiner_peer: *joiner_peer,
        host_peer: *host_peer,
        bridge_protocol: *bridge_protocol,
        virtual_port: *virtual_port,
        issued_at: millis(claims.issued_at),
        expires_at: millis(claims.expires_at),
        consumed_at: None,
        consumer_bridge_instance_id: None,
        consumer_request_id: None,
    }
}

/// An admission id is the 64 lower-case hex digits of a jti digest.
fn admission_id_shaped(id: &str) -> bool {
    id.len() == 64 && id.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
}

impl Admission {
    pub fn new(
        tickets: Arc<dyn TicketAuthority>,
        store: Arc<dyn AdmissionStore>,
        hunts: Arc<dyn AdmissionHunts>,
        transport_key_secret: Option<Secret>,
        clock: Arc<dyn Fn() -> UnixMillis + Send + Sync>,
    ) -> Self {
        let list_calls = HourlyCounter::default();
        Self {
            clock,
            hunts,
            store,
            tickets,
            transport_key_secret,
            list_calls,
        }
    }

    fn now(&self) -> UnixMillis {
        (self.clock)()
    }

    /// The gate, refused with `off` when hosting is switched off.
    fn switched_on(&self, off: No) -> Outcome<P2PGate> {
        let gate = self.hunts.gate()?;
        ensure(gate.enabled, off).map(|()| gate)
    }

    fn hunt(&self, id: &SessionId, missing: No) -> Outcome<AdmissionHunt> {
        self.hunts.find_hunt(id)?.ok_or_else(|| missing.into())
    }

    /// A ticket for `account` as `peer` on `hunt`, not yet stored.
    fn mint_ticket(&self, on: &AdmissionHunt, who: &AccountId, seat: PeerId, at: UnixMillis) -> Outcome<MintedTicket> {
        let bindings = on.ticket_bindings(who, seat)?;
        let minted = self.tickets.mint(bindings, at, Some(JOIN_TICKET_LIFETIME_SECONDS))?;
        Ok(minted)
    }

    fn remember(&self, hunt: &AdmissionHunt, minted: &MintedTicket) -> Outcome<()> {
        let row = issued_row(hunt, minted);
        let by = Actor::Account(row.account.clone());
        match self.store.insert_ticket(&by, &row)? {
            true => Ok(()),
            false => Err(AdmissionError::Conflict),
        }
    }

    fn issue_self(&self, account: &AccountId, session: &SessionId) -> Outcome<IssuedTicket> {
        let gate = self.switched_on(SWITCHED_OFF)?;
        ensure(!gate.emergency_stop, UNAVAILABLE)?;
        let hunt = self.hunt(session, NO_HUNT)?;
        let at = self.now();
        if hunt.snapshot.is_hub() {
            self.issue_for_hub(hunt, account, at)
        } else {
            self.issue_for_participant(&hunt, account, at)
        }
    }

    fn issue_for_hub(&self, hub: AdmissionHunt, account: &AccountId, at: UnixMillis) -> Outcome<IssuedTicket> {
        let link = self.store.peer_link(account)?;
        let peer = self.hunts.check_hub_entry(&hub, account, link.as_ref(), at)?;
        let ticket = self.mint_ticket(&hub, account, peer, at)?;
        let ticketed = self
            .hunts
            .mark_hub_ticketed(&hub, account, at, millis(ticket.claims.expires_at))?;
        ensure(ticketed, NO_HUB_ENTRY)?;
        self.remember(&hub, &ticket)?;
        Ok(IssuedTicket { hunt: hub, ticket })
    }

    fn issue_for_participant(
        &self,
        hunt: &AdmissionHunt,
        account: &AccountId,
        at: UnixMillis,
    ) -> Outcome<IssuedTicket> {
        let snapshot = &hunt.snapshot;
        ensure(
            snapshot.state.is_ticketable() && snapshot.expires_at > at,
            NOT_ACCEPTING,
        )?;
        let seat = hunt.seat_for(account).ok_or(NOT_ROSTERED)?;
        let link = self.store.peer_link(account)?;
        let verified =
            matches!(&link, Some(PeerLinkRecord { status: LinkStatus::Verified, peer, .. }) if *peer == seat);
        ensure(verified, LINK_MISMATCH)?;
        let reserved = self.hunts.reserve_participant(hunt, account, at)?.ok_or(REJOIN_SPENT)?;
        let seat = reserved.seat_for(account).ok_or(NOT_ROSTERED)?;
        let ticket = self.mint_ticket(&reserved, account, seat, at)?;
        self.remember(&reserved, &ticket)?;
        Ok(IssuedTicket { hunt: reserved, ticket })
    }

    fn join_bootstrap(&self, account: &AccountId, session: &SessionId) -> Outcome<JoinBootstrap> {
        self.switched_on(OFF)?;
        let issued = self.issue_self(account, session)?;
        let hunt = self.hunt(session, NO_LOBBY)?;
        let lobby_id = hunt.lobby_id.clone().ok_or(NO_LOBBY)?;
        let mut instance = [0_u8; 16];
        if rand::rngs::SysRng.try_fill_bytes(&mut instance).is_err() {
            return Err(TicketError::Random.into());
        }
        let key = self.tickets.key_id();
        let common = self
            .hunts
            .join_bootstrap_common(&hunt, &key.0, &wire::hex(&instance))?
            .ok_or(NO_LOBBY)?;
        Ok(JoinBootstrap {
            ticket: issued.ticket.text,
            common,
            lobby_id,
        })
    }

    fn consume(&self, host: &HostCapability, request: &ConsumeTicket) -> Outcome<ConsumedTicket> {
        let gate = self.switched_on(SWITCHED_OFF)?;
        let at = self.now();
        let claims = self
            .tickets
            .verify(request.ticket.expose(), at)
            .map_err(|failure| match failure {
                AdmissionError::Ticket(ticket) if ticket.is_expired() => LAPSED_TICKET.into(),
                _ => AdmissionError::from(UNREADABLE_TICKET),
            })?;
        let to = &claims.bindings;
        let generation = u64::from(to.generation);
        ensure(
            to.session.to_string() == host.wire_session && to.joiner_peer.0.to_string() == request.authenticated_peer,
            OTHER_CONNECTION,
        )?;
        ensure(
            (host.generation, host.host_peer, host.bridge_instance_id.as_str())
                == (generation, to.host_peer, request.bridge_instance_id.as_str()),
            OTHER_GENERATION,
        )?;
        ensure(!gate.emergency_stop, UNAVAILABLE)?;
        let hunt = self.hunt(&host.session, NO_HUNT)?;
        let live_generation = hunt.snapshot.generation == generation
            && hunt.snapshot.host_peer == to.host_peer
            && hunt.bridge_instance_id.as_ref() == Some(&request.bridge_instance_id)
            && hunt.snapshot.state.is_ticketable();
        ensure(live_generation, FENCED_HUNT)?;

        let by = host.actor();
        let admission_id = claims.jti_digest();
        let attempt = ConsumeRequest {
            jti_digest: admission_id.clone(),
            session: host.wire_session.clone(),
            generation,
            account: to.account.clone(),
            joiner_peer: to.joiner_peer,
            host_peer: to.host_peer,
            bridge_instance_id: request.bridge_instance_id.clone(),
            request_id: request.request_id.clone(),
            now: at,
        };
        let idempotent = match self.store.consume_ticket(&by, &attempt)? {
            ConsumeOutcome::Idempotent(_) => true,
            ConsumeOutcome::Consumed(_) => false,
            ConsumeOutcome::Expired => return Err(EXPIRED.into()),
            ConsumeOutcome::Replay => return Err(REPLAYED.into()),
            ConsumeOutcome::Invalid => return Err(WRONG_HUNT.into()),
        };
        let is_hub = hunt.snapshot.is_hub();
        if !idempotent {
            let active_key = [
                host.wire_session.clone(),
                generation.to_string(),
                to.joiner_peer.0.to_string(),
            ]
            .join(":");
            // A hub member who re-enters replaces its old admission.
            if is_hub {
                self.store.supersede_active(&by, &active_key, at)?;
            }
            let row = PeerAdmissionRecord {
                active_key: Some(active_key),
                admission_id: admission_id.clone(),
                session: host.wire_session.clone(),
                generation,
                account: to.account.clone(),
                peer: to.joiner_peer,
                bridge_instance_id: request.bridge_instance_id.clone(),
                connection_handle: request.connection_handle.clone(),
                admitted_at: at,
                disconnected_at: None,
                superseded_at: None,
            };
            if !self.store.insert_admission(&by, &row)? {
                return Err(AdmissionError::Conflict);
            }
        }

        // Runs again on an idempotent retry; realms' step must tolerate that (spec §9.6).
        if is_hub {
            self.hunts.admit_hub_peer(&hunt, &to.account, &admission_id, at)?;
        } else {
            ensure(self.hunts.mark_participant_admitted(&hunt, &to.account, at)?, RACED)?;
        }

        Ok(ConsumedTicket {
            account: to.account.clone(),
            peer: to.joiner_peer,
            generation: to.generation,
            wire_session: host.wire_session.clone(),
            admission_id,
            idempotent,
        })
    }

    fn peer_left(&self, host: &HostCapability, admission_id: &str, reported: Option<UnixMillis>) -> Outcome<bool> {
        ensure(admission_id_shaped(admission_id), BAD_ADMISSION_ID)?;
        let at = reported.unwrap_or_else(|| self.now());
        // Only the emergency stop: a disconnect is still recorded while hosting is switched off.
        ensure(!self.hunts.gate()?.emergency_stop, UNAVAILABLE)?;
        let row = self
            .store
            .find_admission(admission_id)?
            .filter(|row| row.session == host.wire_session && row.generation == host.generation);
        let Some(row) = row else {
            return Ok(false);
        };
        let attempt = DisconnectRequest {
            admission_id: admission_id.to_owned(),
            session: host.wire_session.clone(),
            generation: host.generation,
            bridge_instance_id: host.bridge_instance_id.clone(),
            at,
        };
        let done = self.store.disconnect(&host.actor(), &attempt)?;
        if done {
            self.hunts.peer_disconnected(&row, at)?;
        }
        Ok(done)
    }

    fn hub_peers(&self, account: &AccountId, session: &SessionId, generation: Option<i64>) -> Outcome<HubPeers> {
        let gate = self.switched_on(OFF)?;
        ensure(!gate.emergency_stop, STOPPED)?;
        let asked = generation.ok_or(NO_GENERATION)?;
        let secret = self.transport_key_secret.as_ref().ok_or(NO_TRANSPORT)?;
        let hub = self
            .hunts
            .find_hunt(session)?
            .filter(|found| found.snapshot.is_hub())
            .ok_or(NO_HUB)?;
        let shot = &hub.snapshot;
        ensure(*account == shot.host_account, NOT_HUB_HOST)?;
        ensure(i64::try_from(shot.generation) == Ok(asked), STALE_HUB)?;
        ensure(shot.state.is_ice_active(), HUB_IDLE)?;
        let budget_key = format!("{}:{asked}", shot.wire_session);
        ensure(
            self.list_calls.allow(&budget_key, PEER_LISTS_PER_HOUR, self.now()),
            LIST_LIMIT,
        )?;
        let key_for = |joiner: &AccountId| {
            derive_pair_key(secret, &shot.wire_session, shot.generation, &shot.host_account, joiner)
        };
        let mut peers = Vec::new();
        for row in self.hunts.hub_presence(&hub, MAXIMUM_PEER_LIST + 1)? {
            if peers.len() == MAXIMUM_PEER_LIST {
                break;
            }
            if row.account == shot.host_account || !row.state.is_live() {
                continue;
            }
            peers.push(HubPeer {
                pair_key: key_for(&row.account),
                state: row.state.as_str(),
                peer: row.peer,
                account: row.account,
            });
        }
        Ok(HubPeers {
            generation: shot.generation,
            peers,
        })
    }
}

/// Every operation is synchronous underneath; the trait is async for stores that are not.
fn settled<'a, T: Send + 'a>(outcome: Outcome<T>) -> Answer<'a, T> {
    Box::pin(ready(outcome))
}

impl AdmissionService for Admission {
    fn issue_self_ticket(&self, who: AccountId, hunt: SessionId) -> Answer<'_, IssuedTicket> {
        settled(self.issue_self(&who, &hunt))
    }

    fn create_join_bootstrap(&self, who: AccountId, hunt: SessionId) -> Answer<'_, JoinBootstrap> {
        settled(self.join_bootstrap(&who, &hunt))
    }

    fn consume_ticket(&self, host: HostCapability, request: ConsumeTicket) -> Answer<'_, ConsumedTicket> {
        settled(self.consume(&host, &request))
    }

    fn record_peer_disconnected(&self, host: HostCapability, id: String, at: Option<UnixMillis>) -> Answer<'_, bool> {
        settled(self.peer_left(&host, &id, at))
    }

    fn list_hub_peers(&self, who: AccountId, hub: SessionId, generation: Option<i64>) -> Answer<'_, HubPeers> {
        settled(self.hub_peers(&who, &hub, generation))
    }
}
