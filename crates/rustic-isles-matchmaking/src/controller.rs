// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The controller (spec §6, `controllers/matchmaking.ts`): the join, the poll, the party instance, the cancel, the
//! party hunt, the immediate instance, the queue, its pop, the launch and the ring instance registration.

use std::sync::Arc;

use module_api::network::{HomeError, InstanceRecord, TicketState};
use module_api::{GameVersion, GlobalId, IdKind};
use rifts_seam::{
    AccountId as RealmAccount, CandidateLeave, CandidateLeaveOutcome, HuntRouteContext, HuntRouteOutcome,
};
use rustic_isles::{Launch, LaunchRequest, Shared};
use rustic_isles_store::{IslesStore, UserId};

use crate::candidate::MatchCandidate;
use crate::party::{Parties, PartyError};
use crate::rules::{self, GoNowWindow, ReleaseKind};
use crate::state::{MatchmakingState, PartyInstance, Queue};

pub const NO_DEDICATED_SERVERS_REASON: &str = "NO_DEDICATED_SERVERS";
pub const GAUNTLET_DISABLED_REASON: &str = "GAUNTLET_DISABLED";
pub const DEDICATED_HUNT_DISABLED_REASON: &str = "DEDICATED_HUNT_DISABLED";
pub const PLAYER_CANCELED_REASON: &str = "PLAYER_CANCELED";
pub const PARTY_MEMBER_CANCELED_REASON: &str = "PARTY_MEMBER_CANCELED";
/// A full queue launches at once.
pub const QUEUE_SIZE: usize = 4;
/// After the last add, checked by the next poll that finds the queue.
pub const QUEUE_WAIT_MS: i64 = 20_000;
pub const PARTY_INSTANCE_REUSE_MS: i64 = 120_000;
/// 2.5 realm presence heartbeats of 30 s, shorter than the 2 min release gate, so a member who just closed the game
/// is not frozen into a roster.
pub const PARTY_ONLINE_WINDOW_MS: i64 = 75_000;
const DEFAULT_GAME_PORT: u16 = 7777;

/// A controller failure: the request's 500.
pub type Failure = String;

/// The party a join travels with, as the resolution left it.
#[derive(Debug, Clone, Default)]
pub struct JoinParty {
    pub party_id: Option<String>,
    pub members: Option<Vec<String>>,
    pub revision: Option<i64>,
    /// Only when the resolution kept the party
    pub global_id: Option<GlobalId>,
}

/// One join's request fields.
#[derive(Debug, Clone, Default)]
pub struct JoinRequest {
    pub player: String,
    pub mode: Option<String>,
    pub args: Option<String>,
    pub hunt: Option<String>,
    pub build_id: Option<String>,
    pub party: JoinParty,
}

/// What a leave did (`CancelMatchmakingForLeave`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaveOutcome {
    pub outcome: String,
    pub state: String,
    pub candidate_id: Option<String>,
}

/// One request's view of the module.
pub struct Ctx {
    pub shared: Arc<Shared>,
    pub state: Arc<MatchmakingState>,
    pub request_id: Option<String>,
    pub client: Option<String>,
}

fn failed(error: impl std::fmt::Display) -> Failure {
    error.to_string()
}

fn party_failure(error: PartyError) -> Failure {
    error.to_string()
}

/// Isles' `party:<ULID>@<node>` as rifts' `<ULID>@<node>`.
pub fn rifts_party_id(id: &GlobalId) -> Option<rifts_seam::PartyId> {
    let text = id.to_string();
    text.split_once(':')?.1.parse().ok()
}

fn launch_kind(is_hub: bool) -> &'static str {
    if is_hub { "hub" } else { "hunt" }
}

impl Ctx {
    pub fn now(&self) -> i64 {
        self.state.now_ms()
    }

    pub fn store(&self) -> Option<Arc<IslesStore>> {
        self.shared.slot::<IslesStore>()
    }

    pub fn parties<'a>(&'a self, store: Option<&'a IslesStore>) -> Parties<'a> {
        Parties {
            store,
            node: &self.shared.node().node,
            request_id: self.request_id.as_deref(),
            now: self.now(),
        }
    }

    fn game_version(&self) -> Option<GameVersion> {
        self.shared.version().tables.game_version.parse().ok()
    }

    /// An account's global id: the text itself when it is one, else the store's account with that userId.
    pub async fn resolve_account(&self, user: &str) -> Option<GlobalId> {
        resolve_account(self.store().as_deref(), user).await
    }

    fn release(&self, kind: ReleaseKind) -> i64 {
        let window = GoNowWindow::from_settings(self.shared.settings(), kind);
        let now = self.now();
        let in_flight = self.state.tables().in_flight(now);
        window.release_at(now, in_flight)
    }

    fn realm(user: &str) -> RealmAccount {
        RealmAccount(user.to_owned())
    }

    // ---------------------------------------------------------------- the join (`HandlePlayerMatchmaking`)

    /// True when taken, false for a bare 400.
    pub async fn join(&self, request: &JoinRequest) -> Result<bool, Failure> {
        match self.shared.settings().raw("MATCHMAKING_MODE") {
            Some("DISABLED") => {
                tracing::warn!("Matchmaking is disabled, refusing MM!");
                return Ok(false);
            }
            Some("DEPLOYSERVER") => {}
            _ => {
                tracing::error!("Unsupported MATCHMAKING_MODE!");
                return Ok(false);
            }
        }
        let mode = request.mode.as_deref();
        let hunt = rules::effective_hunt_id(mode, request.args.as_deref(), request.hunt.as_deref());
        let everyone = request
            .party
            .members
            .clone()
            .unwrap_or_else(|| vec![request.player.clone()]);
        // Before the realms route, so it holds on every route
        if rules::is_gauntlet_hunt_id(&hunt) && !rules::gauntlet_enabled(self.shared.settings()) {
            tracing::warn!(
                "[HuntRoute] Gauntlet hunt {hunt} refused reason={GAUNTLET_DISABLED_REASON} (RUSTIC_ISLES_GAUNTLET is not on)"
            );
            return Ok(self.state.cancel_hunt(&hunt, &everyone, GAUNTLET_DISABLED_REASON));
        }
        let is_hub = rules::is_hub_destination(mode, &hunt);
        let context = HuntRouteContext {
            game_mode: mode.unwrap_or("").to_owned(),
            hunt_id: hunt.clone(),
            player: Self::realm(&request.player),
            party: request.party.global_id.as_ref().and_then(rifts_party_id),
            party_members: everyone.iter().map(|m| Self::realm(m)).collect(),
            party_revision: request.party.revision.and_then(|r| u64::try_from(r).ok()),
            build_id: request.build_id.clone(),
            is_hub,
        };
        if let HuntRouteOutcome::Handled { result } =
            self.shared.rifts().realms().route_hunt(context).await.map_err(failed)?
        {
            return Ok(result);
        }
        let party = match (&request.party.party_id, everyone.len() > 1) {
            (Some(id), true) => Some((id.clone(), everyone.clone())),
            _ => None,
        };
        let blank = service_kit::js::trim(&hunt).is_empty();
        if let (false, false, Some((party_id, members))) = (is_hub, blank, &party) {
            return self.party_hunt(&request.player, &hunt, party_id, members).await;
        }
        if blank || !rules::needs_queue(&hunt) {
            return self.immediate(request, &hunt, &everyone, party.is_some()).await;
        }
        if let Some((party_id, members)) = &party {
            return self.party_hunt(&request.player, &hunt, party_id, members).await;
        }
        self.queue(&request.player, &hunt).await
    }

    // ---------------------------------------------------------------- the poll (`CheckAndUpdateQueueStatus`)

    /// A candidate owned by an optional module may outlive the process, so the realms side may restore one.
    async fn current_or_restored(&self, player: &str) -> Result<Option<MatchCandidate>, Failure> {
        if let Some(candidate) = self.state.candidate(player) {
            return Ok(Some(candidate));
        }
        let restored = self
            .shared
            .rifts()
            .realms()
            .restore_candidate(Self::realm(player))
            .await
            .map_err(failed)?;
        let Some(candidate) = restored.and_then(|c| MatchCandidate::from_json_text(&c.0.0)) else {
            return Ok(None);
        };
        self.state.set_candidate(player, candidate.clone());
        Ok(Some(candidate))
    }

    pub async fn poll(&self, player: &str) -> Result<Option<MatchCandidate>, Failure> {
        let Some(current) = self.current_or_restored(player).await? else {
            return Ok(None);
        };
        let seam = rifts_seam::Candidate(rifts_seam::Json(current.to_json_text()));
        let realm = self
            .shared
            .rifts()
            .realms()
            .candidate_status(Self::realm(player), seam)
            .await
            .map_err(failed)?;
        // The realms side owns its own release (spec §15 item 15)
        if let Some(answer) = realm.and_then(|c| MatchCandidate::from_json_text(&c.0.0)) {
            return Ok(Some(answer));
        }
        if !current.ready && !current.canceled {
            match current.candidate_id.parse::<GlobalId>() {
                Ok(ticket) if ticket.kind() == IdKind::Ticket => self.poll_ticket(player, &current, &ticket).await,
                _ => {
                    let due = {
                        let tables = self.state.tables();
                        let now = self.now();
                        tables
                            .queues
                            .get(&current.hunt_id)
                            .is_some_and(|q| now.saturating_sub(q.last_add) > QUEUE_WAIT_MS)
                    };
                    if due {
                        self.pop_queue(&current.hunt_id).await?;
                    }
                }
            }
        }
        let now = self.now();
        Ok(self.state.candidate(player).map(|c| c.client_visible(now)))
    }

    async fn poll_ticket(&self, player: &str, current: &MatchCandidate, ticket: &GlobalId) {
        let home = self.state.home(&self.shared);
        if let Some(game) = self.game_version() {
            let _ = home.ring_try_form_matches(&current.hunt_id, &game).await;
        }
        match home.ring_ticket_state(ticket).await {
            Ok(Some(TicketState::Claimed { match_id, .. })) => {
                if let Ok(Some(record)) = home.ring_lookup_instance(&match_id).await {
                    let release = self.release(ReleaseKind::Hunt);
                    let mut tables = self.state.tables();
                    if let Some(stored) = tables.candidates.get_mut(player).filter(|c| !c.canceled) {
                        stored.host = record.server_host.clone();
                        stored.port = i64::from(record.server_port);
                        stored.game_session_id = match_id.to_string();
                        stored.ready = true;
                        stored.ready_at = release;
                    }
                }
            }
            Ok(Some(TicketState::Cancelled)) => {
                if let Some(stored) = self.state.tables().candidates.get_mut(player) {
                    stored.ready = false;
                    stored.canceled = true;
                    stored.status_reason = Some(PLAYER_CANCELED_REASON.to_owned());
                }
            }
            _ => {}
        }
    }

    // ---------------------------------------------------------------- party instances

    /// `GetPartyInstance`: the local one client-visible, else the realms side's as it is.
    pub async fn party_instance(
        &self,
        party_id: &str,
        global: Option<&GlobalId>,
    ) -> Result<Option<MatchCandidate>, Failure> {
        let now = self.now();
        let local = self
            .state
            .tables()
            .party_instances
            .get(party_id)
            .map(|i| i.candidate.client_visible(now));
        if local.is_some() {
            return Ok(local);
        }
        let Some(party) = global.and_then(rifts_party_id) else {
            return Ok(None);
        };
        let realm = self
            .shared
            .rifts()
            .realms()
            .party_instance(party)
            .await
            .map_err(failed)?;
        Ok(realm.and_then(|c| MatchCandidate::from_json_text(&c.0.0)))
    }

    /// `GetPlayerCandidate`.
    pub async fn player_candidate(&self, player: &str) -> Result<Option<MatchCandidate>, Failure> {
        let now = self.now();
        if let Some(stored) = self.state.candidate(player) {
            return Ok(Some(stored.client_visible(now)));
        }
        self.poll(player).await
    }

    // ---------------------------------------------------------------- the leave (`CancelMatchmakingForLeave`)

    pub async fn cancel_for_leave(&self, player: &str) -> Result<LeaveOutcome, Failure> {
        let outcome = |outcome: &str, state: &str, id: Option<&str>| LeaveOutcome {
            outcome: outcome.to_owned(),
            state: state.to_owned(),
            candidate_id: id.map(str::to_owned),
        };
        let Some(current) = self.current_or_restored(player).await? else {
            return Ok(outcome("no-candidate", "none", None));
        };
        let id = Some(current.candidate_id.as_str());
        let seam = rifts_seam::Candidate(rifts_seam::Json(current.to_json_text()));
        let realm = self
            .shared
            .rifts()
            .realms()
            .cancel_candidate(Self::realm(player), seam)
            .await
            .map_err(failed)?;
        if let CandidateLeave::Handled { outcome: done, state } = realm {
            let text = match done {
                CandidateLeaveOutcome::Canceled => "canceled",
                CandidateLeaveOutcome::AlreadyCanceled => "already-canceled",
                CandidateLeaveOutcome::AlreadyEnded => "already-ended",
                CandidateLeaveOutcome::TooLate => "too-late",
                CandidateLeaveOutcome::HubNotCancelled => "hub-not-cancelled",
            };
            return Ok(outcome(text, &state, id));
        }
        // A hub candidate is never cancelled; a second press is a no-op
        if rules::is_hub_hunt_id(&current.hunt_id) {
            return Ok(outcome("hub-not-cancelled", "hub", id));
        }
        if current.canceled {
            return Ok(outcome("already-canceled", "canceled", id));
        }
        if let Ok(ticket) = current.candidate_id.parse::<GlobalId>()
            && ticket.kind() == IdKind::Ticket
        {
            let _ = self.state.home(&self.shared).ring_cancel_ticket(&ticket).await;
        }
        let now = self.now();
        let state = if current.is_client_ready(now) {
            "ready"
        } else if current.ready {
            "go-now"
        } else if self
            .state
            .tables()
            .queues
            .get(&current.hunt_id)
            .is_some_and(|q| q.players.iter().any(|p| p == player))
        {
            "queued"
        } else {
            "matching"
        };
        let store = self.store();
        // The routes refuse a leave without a database first; a module without one has no parties
        let party = match self.parties(store.as_deref()).for_player(player).await {
            Err(PartyError::NoDatabase) => None,
            found => found.map_err(party_failure)?,
        };
        let mut tables = self.state.tables();
        let mut accounts = vec![player.to_owned()];
        let mut used_party = None;
        if let Some(party) = party.filter(|p| p.members.len() >= 2) {
            let same = tables
                .party_instances
                .get(&party.party_id)
                .is_some_and(|i| i.candidate.game_session_id == current.game_session_id);
            if same {
                for member in &party.members {
                    let user = member.user_id.as_str();
                    let follows = tables
                        .candidates
                        .get(user)
                        .is_some_and(|c| !c.canceled && c.game_session_id == current.game_session_id);
                    if user != player && follows {
                        accounts.push(user.to_owned());
                    }
                }
                used_party = Some(party.party_id);
            }
        }
        // A search already told to travel is left alone
        let told = accounts
            .iter()
            .any(|a| tables.candidates.get(a).is_some_and(|c| c.is_client_ready(now)));
        if told {
            return Ok(outcome("too-late", state, id));
        }
        for account in &accounts {
            if let Some(candidate) = tables.candidates.get_mut(account) {
                let reason = if account == player {
                    PLAYER_CANCELED_REASON
                } else {
                    PARTY_MEMBER_CANCELED_REASON
                };
                candidate.ready = false;
                candidate.canceled = true;
                candidate.status_reason = Some(reason.to_owned());
                candidate.host = String::new();
                candidate.port = 0;
                candidate.ready_at = 0;
            }
        }
        // A resolved queue is already launching, and its pop skips cancelled players
        let hunt = current.hunt_id.clone();
        if let Some(queue) = tables.queues.get_mut(&hunt).filter(|q| !q.resolved) {
            queue.players.retain(|p| !accounts.contains(p));
            if queue.players.is_empty() {
                tables.queues.remove(&hunt);
            }
        }
        if let Some(party_id) = used_party {
            tables.party_instances.remove(&party_id);
        }
        drop(tables);
        tracing::info!(
            "Search for {hunt} cancelled by its player: {} account(s), state={state}",
            accounts.len()
        );
        Ok(outcome("canceled", state, id))
    }

    // ---------------------------------------------------------------- the party hunt (`HandlePartyMatchmaking`)

    /// A whole party joining one hunt lands in one instance.
    async fn party_hunt(&self, player: &str, hunt: &str, party_id: &str, members: &[String]) -> Result<bool, Failure> {
        let now = self.now();
        let reuse = {
            let mut tables = self.state.tables();
            let fresh = tables
                .party_instances
                .get(party_id)
                .filter(|i| i.candidate.hunt_id == hunt && now.saturating_sub(i.made_at) < PARTY_INSTANCE_REUSE_MS)
                .map(|i| i.candidate.clone());
            fresh.map(|shared| {
                // The destination is party-wide; a candidate is one client's travel attempt
                let copy = MatchCandidate {
                    candidate_id: rustic_isles::replay::next_uuid(),
                    ..shared.clone()
                };
                tables.candidates.insert(player.to_owned(), copy);
                let address = format!("{}:{}", shared.host, shared.port);
                tables.launch_refs.get(&address).cloned().unwrap_or(address)
            })
        };
        if let Some(hunt_ref) = reuse {
            self.state
                .hunt_activity()
                .dedicated_hunt_starts(&[player.to_owned()], hunt, &hunt_ref);
            return Ok(true);
        }
        let (host, port) = match self.launch(Some("ISLAND"), Some(""), hunt, members).await? {
            Launch::Refused { reason: Some(reason) } => return Ok(self.state.cancel_hunt(hunt, members, &reason)),
            Launch::Refused { reason: None } => return Ok(false),
            Launch::Online { host, port } => (host, port),
        };
        let ring = self.register_ring_instance(hunt, &host, port, members).await;
        let id = rustic_isles::replay::next_uuid();
        let release = self.release(ReleaseKind::Hunt);
        let shared = MatchCandidate {
            ready: true,
            host,
            port,
            ready_at: release,
            game_session_id: ring.map_or_else(|| id.clone(), |r| r.to_string()),
            ..MatchCandidate::matching(hunt, &id)
        };
        tracing::info!(
            "Party {party_id} hunt '{hunt}' is online; synchronized Go Now release in {}ms for {} member(s)",
            release.saturating_sub(self.now()),
            members.len()
        );
        self.share(Some(party_id), shared, members);
        Ok(true)
    }

    /// The party instance (made now) and a copy per member under its own candidate id.
    fn share(&self, party_id: Option<&str>, shared: MatchCandidate, members: &[String]) {
        let now = self.now();
        let mut tables = self.state.tables();
        if let Some(party_id) = party_id {
            let instance = PartyInstance {
                candidate: shared.clone(),
                made_at: now,
            };
            tables.party_instances.insert(party_id.to_owned(), instance);
        }
        for member in members {
            let copy = MatchCandidate {
                candidate_id: rustic_isles::replay::next_uuid(),
                ..shared.clone()
            };
            tables.candidates.insert(member.clone(), copy);
        }
    }

    // ---------------------------------------------------------------- the immediate instance

    async fn immediate(
        &self,
        request: &JoinRequest,
        hunt: &str,
        everyone: &[String],
        with_party: bool,
    ) -> Result<bool, Failure> {
        let mode = request.mode.as_deref();
        let is_hub = rules::is_hub_destination(mode, hunt);
        let ramsgate = match (is_hub && hunt == rules::RAMSGATE_HUNT, &self.client) {
            (true, Some(address)) => self.ring_ramsgate(&request.player, address).await,
            _ => None,
        };
        let (host, port, ring) = match ramsgate {
            Some(found) => found,
            None => {
                let launched = self.launch(mode, request.args.as_deref(), hunt, everyone).await?;
                let (host, port) = match launched {
                    Launch::Refused { reason: Some(reason) } => {
                        return Ok(self.state.cancel_hunt(hunt, everyone, &reason));
                    }
                    Launch::Refused { reason: None } => return Ok(false),
                    Launch::Online { host, port } => (host, port),
                };
                let ring = if is_hub {
                    None
                } else {
                    self.register_ring_instance(hunt, &host, port, everyone).await
                };
                (host, port, ring)
            }
        };
        if is_hub {
            // Travelling to a hub ends the party's dedicated hunts, once each
            self.state.hunt_activity().dedicated_hunt_ends(everyone, "RETURNED");
        }
        let id = rustic_isles::replay::next_uuid();
        let game_session_id = match ring {
            Some(ring) => ring.to_string(),
            None if is_hub => self.state.hub_game_session_id(&host, port),
            None => id.clone(),
        };
        let kind = if is_hub { ReleaseKind::Hub } else { ReleaseKind::Hunt };
        let release = self.release(kind);
        let shared = MatchCandidate {
            ready: true,
            host,
            port,
            ready_at: release,
            game_session_id: game_session_id.clone(),
            ..MatchCandidate::matching(hunt, &id)
        };
        tracing::info!(
            "{} '{hunt}' is online; synchronized Go Now release in {}ms candidateId={id} gameSessionId={game_session_id}",
            if is_hub { "Hub" } else { "Hunt" },
            release.saturating_sub(self.now())
        );
        if with_party {
            self.share(request.party.party_id.as_deref(), shared, everyone);
        } else {
            let mut tables = self.state.tables();
            if let Some(party_id) = &request.party.party_id {
                tables.party_instances.remove(party_id);
            }
            tables.candidates.insert(request.player.clone(), shared);
        }
        Ok(true)
    }

    /// DN9: the ring's Ramsgate for this player, as host, port and instance.
    async fn ring_ramsgate(&self, player: &str, address: &str) -> Option<(String, i64, Option<GlobalId>)> {
        let account = self.resolve_account(player).await?;
        let game = self.game_version()?;
        let home = self.state.home(&self.shared);
        let (record, _) = home.ring_assign_ramsgate(&account, address, &game).await.ok()?;
        Some((record.server_host, i64::from(record.server_port), Some(record.id)))
    }

    // ---------------------------------------------------------------- the queue (`QueuePlayer`)

    async fn queue(&self, player: &str, hunt: &str) -> Result<bool, Failure> {
        // Without a client address the hosting node could not admit the player, so the hunt stays local
        if let Some(address) = self.client.clone()
            && let Some(taken) = self.ring_ticket(player, hunt, &address).await
        {
            return Ok(taken);
        }
        let now = self.now();
        let full = {
            let mut tables = self.state.tables();
            match tables.queues.get_mut(hunt) {
                None => {
                    let queue = Queue {
                        players: vec![player.to_owned()],
                        last_add: now,
                        resolved: false,
                    };
                    tables.queues.insert(hunt.to_owned(), queue);
                    false
                }
                Some(queue) if queue.resolved => return Ok(false),
                Some(queue) => {
                    queue.players.push(player.to_owned());
                    queue.last_add = now;
                    queue.players.len() >= QUEUE_SIZE
                }
            }
        };
        // Popped before the player's candidate exists, so the fourth waits on nothing (Node's order, §15 item 3)
        if full {
            self.pop_queue(hunt).await?;
        }
        let id = rustic_isles::replay::next_uuid();
        self.state.set_candidate(player, MatchCandidate::matching(hunt, &id));
        Ok(true)
    }

    /// DN8: a ring ticket; none goes on to the local queue.
    async fn ring_ticket(&self, player: &str, hunt: &str, address: &str) -> Option<bool> {
        let account = self.resolve_account(player).await?;
        let game = self.game_version()?;
        let home = self.state.home(&self.shared);
        let ticket = match home.ring_submit_ticket(&account, hunt, &game, 1, address).await {
            Ok(ticket) => ticket.ticket,
            Err(HomeError::NotInNetwork) => return None,
            Err(e) => {
                tracing::warn!("ring_submit_ticket for {player} failed ({e}); using local queue");
                return None;
            }
        };
        let _ = home.ring_try_form_matches(hunt, &game).await;
        let mut candidate = MatchCandidate::matching(hunt, &ticket.to_string());
        if let Ok(Some(TicketState::Claimed { match_id, .. })) = home.ring_ticket_state(&ticket).await
            && let Ok(Some(record)) = home.ring_lookup_instance(&match_id).await
        {
            candidate.host = record.server_host;
            candidate.port = i64::from(record.server_port);
            candidate.game_session_id = match_id.to_string();
            candidate.ready = true;
            candidate.ready_at = self.release(ReleaseKind::Hunt);
        }
        self.state.set_candidate(player, candidate);
        Some(true)
    }

    /// `PopQueue`. A launch that fails clears the launching mark (a departure, spec §15 item 4).
    async fn pop_queue(&self, hunt: &str) -> Result<(), Failure> {
        let players = {
            let mut tables = self.state.tables();
            match tables.queues.get_mut(hunt) {
                Some(queue) if !queue.resolved => {
                    queue.resolved = true;
                    queue.players.clone()
                }
                _ => return Ok(()),
            }
        };
        let launched = match self.launch(Some("ISLAND"), Some(""), hunt, &players).await {
            Ok(launched) => launched,
            Err(e) => {
                if let Some(queue) = self.state.tables().queues.get_mut(hunt) {
                    queue.resolved = false;
                }
                return Err(e);
            }
        };
        match launched {
            Launch::Online { host, port } => {
                let release = self.release(ReleaseKind::Hunt);
                tracing::info!(
                    "Hunt '{hunt}' is online; synchronized Go Now release in {}ms for {} queued player(s)",
                    release.saturating_sub(self.now()),
                    players.len()
                );
                let mut tables = self.state.tables();
                for player in &players {
                    if let Some(candidate) = tables.candidates.get_mut(player).filter(|c| !c.canceled) {
                        candidate.host = host.clone();
                        candidate.port = port;
                        candidate.ready = true;
                        candidate.ready_at = release;
                    }
                }
                tables.queues.remove(hunt);
            }
            Launch::Refused { reason } => {
                let reason = reason.unwrap_or_else(|| DEDICATED_HUNT_DISABLED_REASON.to_owned());
                let mut tables = self.state.tables();
                tables.cancel_hunt(hunt, &players, &reason);
                tables.queues.remove(hunt);
            }
        }
        Ok(())
    }

    // ---------------------------------------------------------------- launching (`LaunchGameOnDeployserver`)

    async fn launch(
        &self,
        mode: Option<&str>,
        args: Option<&str>,
        hunt: &str,
        players: &[String],
    ) -> Result<Launch, Failure> {
        let is_hub = rules::is_hub_destination(mode, hunt);
        let servers = self.shared.dedicated_servers();
        if !servers.configured() {
            tracing::warn!(
                "[HuntRoute] no dedicated server (DEPLOYSERVER_URL is unset): refused {} huntId={hunt}",
                launch_kind(is_hub)
            );
            return Ok(Launch::Refused {
                reason: Some(NO_DEDICATED_SERVERS_REASON.to_owned()),
            });
        }
        if !is_hub
            && self
                .shared
                .rifts()
                .realms()
                .dedicated_hunts_disabled()
                .map_err(failed)?
        {
            tracing::error!("[HuntRoute] blocked dedicated hunt launch (dedicated hunts are disabled) huntId={hunt}");
            return Ok(Launch::Refused { reason: None });
        }
        tracing::info!(
            "Querying DeployServer for GameMode: {} HuntId {hunt} with {} Expected Players!",
            mode.unwrap_or("undefined"),
            players.len()
        );
        let request = LaunchRequest {
            game_mode: mode.map(str::to_owned),
            game_args: args.map(str::to_owned),
            hunt_id: hunt.to_owned(),
            expected_players: players.to_vec(),
            request_id: self.request_id.clone(),
        };
        let launched = servers.launch(request).await?;
        if let (false, Launch::Online { host, port }) = (is_hub, &launched) {
            let hunt_ref = rustic_isles::replay::next_uuid();
            self.state
                .tables()
                .launch_refs
                .insert(format!("{host}:{port}"), hunt_ref.clone());
            self.state
                .hunt_activity()
                .dedicated_hunt_starts(players, hunt, &hunt_ref);
        }
        Ok(launched)
    }

    /// The ring's record of a dedicated hunt; any error (not in a network included) gives none, silently.
    async fn register_ring_instance(&self, hunt: &str, host: &str, port: i64, players: &[String]) -> Option<GlobalId> {
        let game = self.game_version()?;
        let mut accounts = Vec::new();
        for player in players {
            if let Some(account) = self.resolve_account(player).await {
                accounts.push(account);
            }
        }
        let node = self.shared.node();
        let record = InstanceRecord {
            id: GlobalId::new(IdKind::Hunt, &node.node),
            region: node.region.clone(),
            game,
            queue: hunt.to_owned(),
            server_host: host.to_owned(),
            server_port: u16::try_from(port).unwrap_or(DEFAULT_GAME_PORT),
            capacity: 4,
            players: accounts.clone(),
            shard: None,
            created_at_ms: self.now(),
        };
        self.state
            .home(&self.shared)
            .ring_register_instance(&record)
            .await
            .ok()?;
        let texts: Vec<String> = accounts.iter().map(ToString::to_string).collect();
        self.state.record_ring_instance(&record.id, &texts);
        Some(record.id)
    }
}

/// An account's global id from a userId or the id's own text.
pub async fn resolve_account(store: Option<&IslesStore>, user: &str) -> Option<GlobalId> {
    if let Ok(id) = user.parse::<GlobalId>()
        && id.kind() == IdKind::Account
    {
        return Some(id);
    }
    let store = store?;
    let key = UserId::new(user);
    let found = store
        .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &key))
        .await
        .ok()??;
    Some(found.id.global().clone())
}
