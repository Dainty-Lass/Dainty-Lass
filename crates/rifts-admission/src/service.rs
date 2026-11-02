// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The admission operations: a ticket for one's own seat, the launcher's join bootstrap, the host's one-time consume,
//! a peer's disconnect, and a hub host's list of peers with their pair keys.

use std::sync::Arc;

use rand::TryRng as _;
use rifts_relay::rules::{HuntSnapshot, PresenceRow, derive_pair_key};
use rifts_relay::wire::{JsonObject, hex, iso_time, json_array, json_string};
use rifts_relay::{HourlyCounter, P2PGate};
use rifts_seam::{AccountId, Actor, BoxFuture, Json, PeerId, Refusal, RuntimeAuth, Secret, SessionId, UnixMillis};
use serde_json::Value;

use crate::store::{
    AdmissionStore, ConsumeOutcome, ConsumeRequest, DisconnectRequest, LinkStatus, PeerAdmissionRecord, PeerLinkRecord,
    TicketRecord, TicketStatus,
};
use crate::ticket::{MintedTicket, SessionUuid, TicketBindings, TicketError};
use crate::{AdmissionError, AdmissionService, TicketAuthority};

#[cfg(test)]
mod tests;

pub const JOIN_TICKET_LIFETIME_SECONDS: u32 = 180;
pub const MAXIMUM_PEER_LIST: usize = 64;
const PEER_LISTS_PER_HOUR: u32 = 3600;

const DISABLED: &str = "P2P_DISABLED";
const EMERGENCY_STOP: &str = "P2P_EMERGENCY_STOP";
const SESSION_NOT_FOUND: &str = "P2P_SESSION_NOT_FOUND";
const SESSION_NOT_READY: &str = "P2P_SESSION_NOT_READY";
const NOT_ROSTERED: &str = "P2P_NOT_ROSTERED";
const LINK_REQUIRED: &str = "P2P_STEAM_LINK_REQUIRED";
const TICKET_INVALID: &str = "P2P_TICKET_INVALID";
const TICKET_EXPIRED: &str = "P2P_TICKET_EXPIRED";
const TICKET_REPLAYED: &str = "P2P_TICKET_REPLAYED";
const GENERATION_FENCED: &str = "P2P_GENERATION_FENCED";
const ICE_UNAVAILABLE: &str = "P2P_ICE_UNAVAILABLE";
const SIGNAL_LIMIT: &str = "P2P_SIGNAL_LIMIT";

const CURRENTLY_DISABLED: &str = "Player-hosted hunts are currently disabled.";
const DISABLED_PLAIN: &str = "Player-hosted hunts are disabled.";
const CURRENTLY_UNAVAILABLE: &str = "Player-hosted hunts are currently unavailable.";
const INVALID_OR_EXPIRED: &str = "The admission ticket is invalid or expired.";
const HOST_LOBBY_NOT_READY: &str = "The host lobby is not ready.";
const NOT_IN_ROSTER: &str = "This account is not in the frozen hunt roster.";

pub fn refused(code: &str, message: &str) -> AdmissionError {
    AdmissionError::Refused(Refusal {
        code: code.to_owned(),
        message: message.to_owned(),
    })
}

/// A hunt or hub as admission sees it.
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
    pub lobby_id: Option<String>,
    pub bridge_instance_id: Option<String>,
}

impl AdmissionHunt {
    fn bindings(&self, account: &AccountId, joiner: PeerId) -> Result<TicketBindings, AdmissionError> {
        let generation = u32::try_from(self.snapshot.generation).map_err(|_| TicketError::OutOfRange("generation"))?;
        Ok(TicketBindings {
            account: account.clone(),
            joiner_peer: joiner,
            host_peer: self.snapshot.host_peer,
            session: self.session,
            generation,
            party_id: self.party_id.clone(),
            party_revision: self.party_revision,
            hunt_definition_id: self.hunt_definition_id.clone(),
            build: self.build,
            bridge_protocol: self.bridge_protocol,
            virtual_port: self.virtual_port,
        })
    }

    fn rostered_peer(&self, account: &AccountId) -> Option<PeerId> {
        self.snapshot
            .roster
            .iter()
            .find(|entry| &entry.account == account)
            .map(|entry| entry.peer)
    }
}

/// What a host runtime proved about itself: its hunt, generation, peer and bridge.
#[derive(Debug, Clone, PartialEq, Eq)]
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
        Some(Self {
            session: runtime.session.clone(),
            wire_session: runtime
                .wire_session
                .clone()
                .unwrap_or_else(|| runtime.session.0.ulid.to_string()),
            generation: runtime.generation,
            host_account: runtime.host_account.clone(),
            host_peer: runtime.host_peer?,
            bridge_instance_id: runtime.bridge_instance.clone()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedTicket {
    pub ticket: MintedTicket,
    pub hunt: AdmissionHunt,
}

fn millis(seconds: u64) -> UnixMillis {
    UnixMillis(i64::try_from(seconds).unwrap_or(i64::MAX).saturating_mul(1000))
}

impl IssuedTicket {
    pub fn to_json(&self) -> String {
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
        JsonObject::new()
            .string("ticket", self.ticket.text.expose())
            .string("expiresAt", &iso_time(millis(self.ticket.claims.expires_at)))
            .raw("rendezvous", &rendezvous)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinBootstrap {
    /// Realms' fields, as one JSON object.
    pub common: Json,
    pub ticket: Secret,
    pub lobby_id: String,
}

impl JoinBootstrap {
    /// Realms' object with `ticket` and `lobbyId` appended.
    pub fn to_json(&self) -> String {
        let common = self.common.0.trim();
        let mut out = common.strip_suffix('}').unwrap_or(common).to_owned();
        if !out.trim_end().ends_with('{') {
            out.push(',');
        }
        out.push_str("\"ticket\":");
        out.push_str(&json_string(self.ticket.expose()));
        out.push_str(",\"lobbyId\":");
        out.push_str(&json_string(&self.lobby_id));
        out.push('}');
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumeTicket {
    pub ticket: Secret,
    pub authenticated_peer: String,
    pub bridge_instance_id: String,
    pub connection_handle: String,
    pub request_id: String,
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

impl ConsumeTicket {
    pub fn from_body(body: &Value, request_id: &str) -> Result<Self, AdmissionError> {
        let text = |name: &str| body.get(name).and_then(Value::as_str);
        let malformed = || refused(TICKET_INVALID, "The admission request is malformed.");
        let (Some(ticket), Some(peer), Some(bridge), Some(handle)) = (
            text("ticket"),
            text("authenticatedSteamId"),
            text("bridgeInstanceId"),
            text("connectionHandle"),
        ) else {
            return Err(malformed());
        };
        if !(16..=96).contains(&utf16_len(bridge)) || !(1..=32).contains(&utf16_len(handle)) {
            return Err(malformed());
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
    pub fn to_json(&self) -> String {
        JsonObject::new()
            .boolean("admitted", true)
            .boolean("idempotent", self.idempotent)
            .string("admissionId", &self.admission_id)
            .string("accountId", &self.account.0)
            .string("steamId", &self.peer.0.to_string())
            .string("sessionId", &self.wire_session)
            .number("generation", self.generation)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubPeer {
    pub account: AccountId,
    pub peer: PeerId,
    pub pair_key: Secret,
    pub state: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubPeers {
    pub generation: u64,
    pub peers: Vec<HubPeer>,
}

impl HubPeers {
    pub fn to_json(&self) -> String {
        let peers = self.peers.iter().map(|peer| {
            JsonObject::new()
                .string("accountId", &peer.account.0)
                .string("peerId", &peer.peer.0.to_string())
                .string("transportKey", peer.pair_key.expose())
                .string("state", peer.state)
                .finish()
        });
        JsonObject::new()
            .number("generation", self.generation)
            .raw("peers", &json_array(peers))
            .finish()
    }
}

/// What admission needs from realms.
pub trait AdmissionHunts: Send + Sync {
    fn gate(&self) -> Result<P2PGate, AdmissionError>;
    fn find_hunt(&self, session: &SessionId) -> Result<Option<AdmissionHunt>, AdmissionError>;
    /// The participant's next attempt within the rejoin window and limit; `None` when refused.
    fn reserve_participant(
        &self,
        hunt: &AdmissionHunt,
        account: &AccountId,
        now: UnixMillis,
    ) -> Result<Option<AdmissionHunt>, AdmissionError>;
    /// The member's peer id, or the refusal.
    fn check_hub_entry(
        &self,
        hub: &AdmissionHunt,
        account: &AccountId,
        link: Option<&PeerLinkRecord>,
        now: UnixMillis,
    ) -> Result<PeerId, AdmissionError>;
    fn mark_hub_ticketed(
        &self,
        hub: &AdmissionHunt,
        account: &AccountId,
        now: UnixMillis,
        ticket_expires_at: UnixMillis,
    ) -> Result<bool, AdmissionError>;
    fn mark_participant_admitted(
        &self,
        hunt: &AdmissionHunt,
        account: &AccountId,
        admitted_at: UnixMillis,
    ) -> Result<bool, AdmissionError>;
    fn admit_hub_peer(
        &self,
        hub: &AdmissionHunt,
        account: &AccountId,
        admission_id: &str,
        admitted_at: UnixMillis,
    ) -> Result<(), AdmissionError>;
    fn peer_disconnected(&self, admission: &PeerAdmissionRecord, at: UnixMillis) -> Result<(), AdmissionError>;
    fn hub_presence(&self, hub: &AdmissionHunt, limit: usize) -> Result<Vec<PresenceRow>, AdmissionError>;
    /// Realms' bootstrap fields as one JSON object, or `None`.
    fn join_bootstrap_common(
        &self,
        hunt: &AdmissionHunt,
        key_id: &str,
        bridge_instance_id: &str,
    ) -> Result<Option<Json>, AdmissionError>;
}

pub struct Admission {
    pub tickets: Arc<dyn TicketAuthority>,
    pub store: Arc<dyn AdmissionStore>,
    pub hunts: Arc<dyn AdmissionHunts>,
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

    /// Mints and records a ticket for `account` on `hunt`.
    fn mint_for(
        &self,
        hunt: &AdmissionHunt,
        account: &AccountId,
        peer: PeerId,
        now: UnixMillis,
    ) -> Result<MintedTicket, AdmissionError> {
        self.tickets
            .mint(hunt.bindings(account, peer)?, now, Some(JOIN_TICKET_LIFETIME_SECONDS))
    }

    fn record(&self, hunt: &AdmissionHunt, minted: &MintedTicket) -> Result<(), AdmissionError> {
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
            issued_at: millis(claims.issued_at),
            expires_at: millis(claims.expires_at),
            status: TicketStatus::Issued,
            consumed_at: None,
            consumer_bridge_instance_id: None,
            consumer_request_id: None,
        };
        let actor = Actor::Account(claims.bindings.account.clone());
        if self.store.insert_ticket(&actor, &record)? {
            Ok(())
        } else {
            Err(AdmissionError::Conflict)
        }
    }

    fn issue_self(&self, account: &AccountId, session: &SessionId) -> Result<IssuedTicket, AdmissionError> {
        let gate = self.hunts.gate()?;
        if !gate.enabled {
            return Err(refused(DISABLED, CURRENTLY_DISABLED));
        }
        if gate.emergency_stop {
            return Err(refused(DISABLED, CURRENTLY_UNAVAILABLE));
        }
        let hunt = self
            .hunts
            .find_hunt(session)?
            .ok_or_else(|| refused(SESSION_NOT_FOUND, "Hunt session not found."))?;
        let now = (self.clock)();

        if hunt.snapshot.is_hub() {
            let link = self.store.peer_link(account)?;
            let peer = self.hunts.check_hub_entry(&hunt, account, link.as_ref(), now)?;
            let ticket = self.mint_for(&hunt, account, peer, now)?;
            let expires_at = millis(ticket.claims.expires_at);
            if !self.hunts.mark_hub_ticketed(&hunt, account, now, expires_at)? {
                return Err(refused(NOT_ROSTERED, "This account has no open entry into the hub."));
            }
            self.record(&hunt, &ticket)?;
            return Ok(IssuedTicket { ticket, hunt });
        }

        if !hunt.snapshot.state.is_ticketable() || hunt.snapshot.expires_at <= now {
            return Err(refused(SESSION_NOT_READY, "The hunt is not accepting participants."));
        }
        let peer = hunt
            .rostered_peer(account)
            .ok_or_else(|| refused(NOT_ROSTERED, NOT_IN_ROSTER))?;
        let linked = self
            .store
            .peer_link(account)?
            .is_some_and(|link| link.status == LinkStatus::Verified && link.peer == peer);
        if !linked {
            return Err(refused(
                LINK_REQUIRED,
                "The current Steam co-op transport no longer matches this hunt.",
            ));
        }
        let reserved = self.hunts.reserve_participant(&hunt, account, now)?.ok_or_else(|| {
            refused(
                SESSION_NOT_READY,
                "The rejoin window or attempt limit has been reached.",
            )
        })?;
        let peer = reserved
            .rostered_peer(account)
            .ok_or_else(|| refused(NOT_ROSTERED, NOT_IN_ROSTER))?;
        let ticket = self.mint_for(&reserved, account, peer, now)?;
        self.record(&reserved, &ticket)?;
        Ok(IssuedTicket { ticket, hunt: reserved })
    }

    fn join_bootstrap(&self, account: &AccountId, session: &SessionId) -> Result<JoinBootstrap, AdmissionError> {
        if !self.hunts.gate()?.enabled {
            return Err(refused(DISABLED, DISABLED_PLAIN));
        }
        let issued = self.issue_self(account, session)?;
        let hunt = self
            .hunts
            .find_hunt(session)?
            .ok_or_else(|| refused(SESSION_NOT_READY, HOST_LOBBY_NOT_READY))?;
        let lobby_id = hunt
            .lobby_id
            .clone()
            .ok_or_else(|| refused(SESSION_NOT_READY, HOST_LOBBY_NOT_READY))?;
        let mut instance = [0_u8; 16];
        rand::rngs::SysRng
            .try_fill_bytes(&mut instance)
            .map_err(|_| TicketError::Random)?;
        let common = self
            .hunts
            .join_bootstrap_common(&hunt, &self.tickets.key_id().0, &hex(&instance))?
            .ok_or_else(|| refused(SESSION_NOT_READY, HOST_LOBBY_NOT_READY))?;
        Ok(JoinBootstrap {
            common,
            ticket: issued.ticket.text,
            lobby_id,
        })
    }

    fn consume(&self, capability: &HostCapability, request: &ConsumeTicket) -> Result<ConsumedTicket, AdmissionError> {
        let gate = self.hunts.gate()?;
        if !gate.enabled {
            return Err(refused(DISABLED, CURRENTLY_DISABLED));
        }
        let now = (self.clock)();
        let claims = match self.tickets.verify(request.ticket.expose(), now) {
            Ok(claims) => claims,
            Err(AdmissionError::Ticket(error)) if error.is_expired() => {
                return Err(refused(TICKET_EXPIRED, INVALID_OR_EXPIRED));
            }
            Err(_) => return Err(refused(TICKET_INVALID, INVALID_OR_EXPIRED)),
        };
        let bindings = &claims.bindings;
        if bindings.session.to_string() != capability.wire_session
            || bindings.joiner_peer.0.to_string() != request.authenticated_peer
        {
            return Err(refused(
                TICKET_INVALID,
                "The ticket does not match the authenticated Steam connection.",
            ));
        }
        let generation = u64::from(bindings.generation);
        if capability.generation != generation
            || capability.host_peer != bindings.host_peer
            || capability.bridge_instance_id != request.bridge_instance_id
        {
            return Err(refused(
                GENERATION_FENCED,
                "The admission ticket does not match the active host generation.",
            ));
        }
        if gate.emergency_stop {
            return Err(refused(DISABLED, CURRENTLY_UNAVAILABLE));
        }
        let hunt = self
            .hunts
            .find_hunt(&capability.session)?
            .ok_or_else(|| refused(SESSION_NOT_FOUND, "Hunt session not found."))?;
        if hunt.snapshot.generation != generation
            || hunt.snapshot.host_peer != bindings.host_peer
            || hunt.bridge_instance_id.as_deref() != Some(request.bridge_instance_id.as_str())
            || !hunt.snapshot.state.is_ticketable()
        {
            return Err(refused(
                GENERATION_FENCED,
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
                generation,
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
            ConsumeOutcome::Expired => return Err(refused(TICKET_EXPIRED, "The admission ticket expired.")),
            ConsumeOutcome::Replay => {
                return Err(refused(TICKET_REPLAYED, "The admission ticket was already consumed."));
            }
            ConsumeOutcome::Invalid => {
                return Err(refused(
                    TICKET_INVALID,
                    "The admission ticket is not valid for this hunt.",
                ));
            }
        };

        if !idempotent {
            let active_key = format!("{}:{generation}:{}", capability.wire_session, bindings.joiner_peer.0);
            // A hub member who re-enters replaces its old admission.
            if hunt.snapshot.is_hub() {
                self.store.supersede_active(&actor, &active_key, now)?;
            }
            let admission = PeerAdmissionRecord {
                admission_id: admission_id.clone(),
                active_key: Some(active_key),
                session: capability.wire_session.clone(),
                generation,
                account: bindings.account.clone(),
                peer: bindings.joiner_peer,
                bridge_instance_id: request.bridge_instance_id.clone(),
                connection_handle: request.connection_handle.clone(),
                admitted_at: now,
                disconnected_at: None,
                superseded_at: None,
            };
            if !self.store.insert_admission(&actor, &admission)? {
                return Err(AdmissionError::Conflict);
            }
        }

        // Runs again on an idempotent retry; realms' step must tolerate that (spec §9.6).
        if hunt.snapshot.is_hub() {
            self.hunts
                .admit_hub_peer(&hunt, &bindings.account, &admission_id, now)?;
        } else if !self.hunts.mark_participant_admitted(&hunt, &bindings.account, now)? {
            return Err(refused(
                GENERATION_FENCED,
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
        let well_formed = admission_id.len() == 64
            && admission_id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if !well_formed {
            return Err(refused(TICKET_INVALID, "Invalid peer admission identifier."));
        }
        let at = disconnected_at.unwrap_or_else(|| (self.clock)());
        // Only the emergency stop: a disconnect is still recorded while hosting is switched off.
        if self.hunts.gate()?.emergency_stop {
            return Err(refused(DISABLED, CURRENTLY_UNAVAILABLE));
        }
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
        let request = DisconnectRequest {
            admission_id: admission_id.to_owned(),
            session: capability.wire_session.clone(),
            generation: capability.generation,
            bridge_instance_id: capability.bridge_instance_id.clone(),
            at,
        };
        let disconnected = self.store.disconnect(&actor, &request)?;
        if disconnected {
            self.hunts.peer_disconnected(&admission, at)?;
        }
        Ok(disconnected)
    }

    fn hub_peers(
        &self,
        account: &AccountId,
        session: &SessionId,
        generation: Option<i64>,
    ) -> Result<HubPeers, AdmissionError> {
        let gate = self.hunts.gate()?;
        if !gate.enabled {
            return Err(refused(DISABLED, DISABLED_PLAIN));
        }
        if gate.emergency_stop {
            return Err(refused(
                EMERGENCY_STOP,
                "Player-hosted hunts are currently stopped by operations policy.",
            ));
        }
        let generation = generation.ok_or_else(|| refused(TICKET_INVALID, "The hub generation is missing."))?;
        let secret = self
            .transport_key_secret
            .as_ref()
            .ok_or_else(|| refused(ICE_UNAVAILABLE, "The co-op transport is not configured on this server."))?;
        let hub = self
            .hunts
            .find_hunt(session)?
            .filter(|hunt| hunt.snapshot.is_hub())
            .ok_or_else(|| refused(SESSION_NOT_FOUND, "Hub session not found."))?;
        let snapshot = &hub.snapshot;
        if account != &snapshot.host_account {
            return Err(refused(NOT_ROSTERED, "Only the hub's host may list its peers."));
        }
        if i64::try_from(snapshot.generation).ok() != Some(generation) {
            return Err(refused(
                GENERATION_FENCED,
                "This request belongs to a stale hub generation.",
            ));
        }
        if !snapshot.state.is_ice_active() {
            return Err(refused(SESSION_NOT_READY, "The hub is not running."));
        }
        let key = format!("{}:{generation}", snapshot.wire_session);
        if !self.peer_lists.allow(&key, PEER_LISTS_PER_HOUR, (self.clock)()) {
            return Err(refused(SIGNAL_LIMIT, "Too many peer list requests for this hub."));
        }
        let peers = self
            .hunts
            .hub_presence(&hub, MAXIMUM_PEER_LIST + 1)?
            .into_iter()
            .filter(|row| row.account != snapshot.host_account && row.state.is_live())
            .take(MAXIMUM_PEER_LIST)
            .map(|row| HubPeer {
                pair_key: derive_pair_key(
                    secret,
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
        Box::pin(std::future::ready(self.issue_self(&account, &session)))
    }

    fn create_join_bootstrap(
        &self,
        account: AccountId,
        session: SessionId,
    ) -> BoxFuture<'_, Result<JoinBootstrap, AdmissionError>> {
        Box::pin(std::future::ready(self.join_bootstrap(&account, &session)))
    }

    fn consume_ticket(
        &self,
        capability: HostCapability,
        request: ConsumeTicket,
    ) -> BoxFuture<'_, Result<ConsumedTicket, AdmissionError>> {
        Box::pin(std::future::ready(self.consume(&capability, &request)))
    }

    fn record_peer_disconnected(
        &self,
        capability: HostCapability,
        admission_id: String,
        disconnected_at: Option<UnixMillis>,
    ) -> BoxFuture<'_, Result<bool, AdmissionError>> {
        Box::pin(std::future::ready(self.disconnected(
            &capability,
            &admission_id,
            disconnected_at,
        )))
    }

    fn list_hub_peers(
        &self,
        account: AccountId,
        session: SessionId,
        generation: Option<i64>,
    ) -> BoxFuture<'_, Result<HubPeers, AdmissionError>> {
        Box::pin(std::future::ready(self.hub_peers(&account, &session, generation)))
    }
}
