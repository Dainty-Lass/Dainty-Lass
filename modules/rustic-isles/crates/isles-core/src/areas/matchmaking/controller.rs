//! `controllers/matchmaking.ts`: queues, party instances, the candidate poll and the client's Cancel. In memory
//! (DD15); the Director is reached through [`super::deploy::DedicatedServers`], realms through `rifts_seam`'s hooks.
//!
//! Node's functions interleave only at their `await`s. Here each stretch between two awaits is one lock of the state,
//! so the same interleavings are the only ones possible.

use module_api::BoxFuture;
use rifts_seam::{
    AccountId as RiftsAccount, Candidate, CandidateLeave, CandidateLeaveOutcome, HuntRouteContext, HuntRouteOutcome,
};
use serde_json::Value;

use super::deploy::{Launch, LaunchRequest};
use super::resolve::{GAUNTLET_DISABLED_REASON, refuses_gauntlet_request};
use super::state::{MatchmakingResult, Queue, cancel_hunt};
use super::util::{delay_ms, random_int};
use crate::http::Shared;

const PARTY_INSTANCE_REUSE_MS: i64 = 120_000;
/// The CR19 queue waits this long for a fourth player (existing queue behaviour)
const QUEUE_WAIT_MS: i64 = 20_000;
const QUEUE_SIZE: usize = 4;
pub const NO_DEDICATED_SERVERS_REASON: &str = "NO_DEDICATED_SERVERS";
pub const RAMSGATE_HUNT: &str = "ShatteredIsles_ReturnToRamsgate";

/// A failure Express answers with its 500 page: a throw in Node.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct MatchError(pub String);

pub type MatchResult<T> = Result<T, MatchError>;

fn seam<T>(result: Result<T, rifts_seam::SeamError>) -> MatchResult<T> {
    result.map_err(|e| MatchError(e.to_string()))
}

/// A join as `HandlePlayerMatchmaking` takes it.
#[derive(Debug, Clone, Default)]
pub struct JoinRequest {
    pub game_mode: Option<String>,
    pub game_args: Option<String>,
    pub hunt_id: Option<String>,
    pub player: String,
    /// Node's `partyId`, the key of the party's shared instance
    pub party_id: Option<String>,
    /// The party's global id, which the realms side names it by
    pub party_global_id: Option<String>,
    pub party_members: Option<Vec<String>>,
    pub party_revision: Option<i64>,
    pub build_id: Option<String>,
}

/// `LeavePartyLookup`'s answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaveParty {
    pub party_id: String,
    pub members: Vec<String>,
}

pub type LeavePartyLookup<'a> = &'a (dyn Fn(String) -> BoxFuture<'a, MatchResult<Option<LeaveParty>>> + Send + Sync);

/// `LeaveAnswer.outcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaveOutcome {
    Canceled,
    AlreadyCanceled,
    AlreadyEnded,
    TooLate,
    HubNotCancelled,
    NoCandidate,
}

impl LeaveOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Canceled => "canceled",
            Self::AlreadyCanceled => "already-canceled",
            Self::AlreadyEnded => "already-ended",
            Self::TooLate => "too-late",
            Self::HubNotCancelled => "hub-not-cancelled",
            Self::NoCandidate => "no-candidate",
        }
    }
}

impl From<CandidateLeaveOutcome> for LeaveOutcome {
    fn from(outcome: CandidateLeaveOutcome) -> Self {
        match outcome {
            CandidateLeaveOutcome::Canceled => Self::Canceled,
            CandidateLeaveOutcome::AlreadyCanceled => Self::AlreadyCanceled,
            CandidateLeaveOutcome::AlreadyEnded => Self::AlreadyEnded,
            CandidateLeaveOutcome::TooLate => Self::TooLate,
            CandidateLeaveOutcome::HubNotCancelled => Self::HubNotCancelled,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaveAnswer {
    pub outcome: LeaveOutcome,
    pub state: String,
    pub candidate_id: Option<String>,
}

/// `GetFallbackHuntId`.
fn fallback_hunt_id(game_mode: Option<&str>, game_args: Option<&str>, hunt_id: Option<&str>) -> String {
    if let Some(hunt) = hunt_id.filter(|h| !h.trim().is_empty()) {
        return hunt.to_owned();
    }
    if game_mode == Some("CITY") {
        return RAMSGATE_HUNT.to_owned();
    }
    if game_args.is_some_and(|args| args.contains("/Game/Maps/islands/1705/dia_moss_triforce")) {
        return "ShatteredIsles_IslandA".to_owned();
    }
    hunt_id.unwrap_or_default().to_owned()
}

fn hunt_id_requires_matchmaking(hunt_id: &str) -> bool {
    hunt_id.contains("CR19")
}

/// `IsHubHuntId`: a hub by its hunt id alone, which is all a candidate carries.
pub fn is_hub_hunt_id(hunt_id: Option<&str>) -> bool {
    hunt_id.is_some_and(|id| id.contains("ReturnToRamsgate") || id.contains("TrainingDojo"))
}

/// Hub destinations are shared social spaces that never party-matchmake: the party is already together there.
fn is_hub_destination(game_mode: Option<&str>, hunt_id: Option<&str>) -> bool {
    game_mode == Some("CITY") || is_hub_hunt_id(hunt_id)
}

pub struct Matchmaker<'a> {
    pub shared: &'a Shared,
    /// Forwarded to the Director as `x-mysticparadox-request-id`
    pub request_id: Option<String>,
}

impl Matchmaker<'_> {
    fn state(&self) -> &super::state::MatchmakingState {
        &self.shared.matchmaking
    }

    fn now(&self) -> i64 {
        self.state().clock.now_ms()
    }

    /// `CreateGoNowReleaseAt`: a synchronised, load-aware countdown. Hubs are already-running servers, so theirs is
    /// short and exists only to spread a burst; a hunt's masks the dedicated server's spin-up.
    fn go_now_release_at(&self, inner: &super::state::Inner, is_hub: bool) -> i64 {
        let setting = |name: &str| self.shared.settings.raw(name);
        let now = self.now();
        let per_joiner = delay_ms(setting("GO_NOW_LOAD_PADDING_PER_JOINER_MS"), 1000);
        let padding_max = delay_ms(setting("GO_NOW_LOAD_PADDING_MAX_MS"), 20_000);
        let in_flight = i64::try_from(inner.results.values().filter(|r| r.ready_at > now).count()).unwrap_or(i64::MAX);
        let padding = in_flight.saturating_mul(per_joiner).min(padding_max);
        let (base_min, base_max) = if is_hub {
            (
                delay_ms(setting("HUB_GO_NOW_MIN_DELAY_MS"), 0),
                delay_ms(setting("HUB_GO_NOW_MAX_DELAY_MS"), 1000),
            )
        } else {
            (
                delay_ms(setting("HUNT_GO_NOW_MIN_DELAY_MS"), 10_000),
                delay_ms(setting("HUNT_GO_NOW_MAX_DELAY_MS"), 20_000),
            )
        };
        let min = base_min.min(base_max);
        let max = min.max(base_max) + padding;
        now + random_int(min, max + 1)
    }

    /// `LaunchGameOnDeployserver`.
    async fn launch_game(
        &self,
        game_mode: Option<&str>,
        game_args: Option<&str>,
        hunt_id: &str,
        expected_players: &[String],
    ) -> MatchResult<Launch> {
        let deploy = self.state().deploy.get();
        let hub = is_hub_destination(game_mode, Some(hunt_id));
        // No DeployServer: no request is made, for a hub or a hunt alike; the caller cancels with the reason.
        if !deploy.configured() {
            tracing::warn!(
                "[HuntRoute] no dedicated server (DEPLOYSERVER_URL is unset): refused {} huntId={hunt_id}",
                if hub { "hub" } else { "hunt" }
            );
            return Ok(Launch::Refused {
                reason: Some(NO_DEDICATED_SERVERS_REASON.to_owned()),
            });
        }
        if !hub && seam(self.shared.rifts.realms().dedicated_hunts_disabled())? {
            tracing::error!(
                "[HuntRoute] blocked dedicated hunt launch (dedicated hunts are disabled) huntId={hunt_id}"
            );
            return Ok(Launch::Refused { reason: None });
        }
        tracing::info!(
            "Querying DeployServer for GameMode: {} HuntId {hunt_id} with {} Expected Players!",
            game_mode.unwrap_or("undefined"),
            expected_players.len()
        );
        let launch = deploy
            .launch(LaunchRequest {
                game_mode: game_mode.map(str::to_owned),
                game_args: game_args.map(str::to_owned),
                hunt_id: hunt_id.to_owned(),
                expected_players: expected_players.to_vec(),
                request_id: self.request_id.clone(),
            })
            .await
            .map_err(MatchError)?;
        if let Launch::Online { host, port } = &launch
            && !hub
        {
            // The activity monitor's participation rows; the launch never waits for them
            let hunt_ref = crate::game_data::season_window::next_uuid().to_string();
            self.state()
                .lock()
                .hunt_ref_of_instance
                .insert(format!("{host}:{port}"), hunt_ref.clone());
            self.state()
                .activity
                .get()
                .dedicated_hunt_starts(expected_players, hunt_id, &hunt_ref);
        }
        Ok(launch)
    }

    /// `PopQueue`: launches the hunt for the players waiting on it.
    async fn pop_queue(&self, hunt_id: &str) -> MatchResult<()> {
        let players = {
            let mut inner = self.state().lock();
            let Some(queue) = inner.queues.get_mut(hunt_id) else {
                return Ok(());
            };
            if queue.resolved {
                return Ok(());
            }
            queue.resolved = true;
            queue.players.clone()
        };
        let launch = self.launch_game(Some("ISLAND"), Some(""), hunt_id, &players).await?;
        let Launch::Online { host, port } = launch else {
            let Launch::Refused { reason } = launch else {
                return Ok(());
            };
            let mut inner = self.state().lock();
            cancel_hunt(
                &mut inner,
                hunt_id,
                &players,
                reason.as_deref().unwrap_or("DEDICATED_HUNT_DISABLED"),
            );
            inner.queues.remove(hunt_id);
            return Ok(());
        };
        let mut inner = self.state().lock();
        let release_at = self.go_now_release_at(&inner, false);
        tracing::info!(
            "Hunt '{hunt_id}' is online; synchronized Go Now release in {}ms for {} queued player(s)",
            release_at - self.now(),
            players.len()
        );
        for player in &players {
            // A player who cancelled while this queue was launching is not told to travel
            if let Some(result) = inner.results.get_mut(player)
                && !result.canceled
            {
                result.host.clone_from(&host);
                result.port = port;
                result.ready = true;
                result.ready_at = release_at;
            }
        }
        inner.queues.remove(hunt_id);
        Ok(())
    }

    /// `CheckAndUpdateQueueStatus`.
    pub async fn check_and_update_queue_status(&self, player: &str) -> MatchResult<Option<MatchmakingResult>> {
        let account = RiftsAccount(player.to_owned());
        let held = self.state().candidate(player);
        let current = match held {
            Some(current) => current,
            None => {
                // A candidate owned by an optional module may outlive this process (a player-hosted hunt after a
                // restart)
                let restored = seam(self.shared.rifts.realms().restore_candidate(account.clone()).await)?;
                let Some(restored) = restored.and_then(|c| parse_candidate(&c)) else {
                    return Ok(None);
                };
                self.state().set_candidate(player, restored.clone());
                restored
            }
        };
        let extension = seam(
            self.shared
                .rifts
                .realms()
                .candidate_status(account, to_candidate(&current))
                .await,
        )?;
        if let Some(status) = extension.and_then(|c| parse_candidate(&c)) {
            return Ok(Some(status));
        }
        if !current.ready {
            let due = {
                let inner = self.state().lock();
                inner
                    .queues
                    .get(&current.hunt_id)
                    .is_some_and(|queue| self.now() - queue.last_added_ms > QUEUE_WAIT_MS)
            };
            // A candidate cancelled with no queue behind it has nothing to pop
            if due {
                self.pop_queue(&current.hunt_id).await?;
            }
        }
        let now = self.now();
        Ok(self.state().candidate(player).map(|r| r.client_visible(now)))
    }

    /// `QueuePlayer`: false when the hunt's queue is already launching.
    async fn queue_player(&self, hunt_id: &str, player: &str) -> MatchResult<bool> {
        let now = self.now();
        let full = {
            let mut inner = self.state().lock();
            match inner.queues.get_mut(hunt_id) {
                Some(queue) if !queue.resolved => {
                    queue.players.push(player.to_owned());
                    queue.last_added_ms = now;
                    queue.players.len() >= QUEUE_SIZE
                }
                Some(_) => return Ok(false),
                None => {
                    inner.queues.insert(
                        hunt_id.to_owned(),
                        Queue {
                            players: vec![player.to_owned()],
                            last_added_ms: now,
                            resolved: false,
                        },
                    );
                    false
                }
            }
        };
        // Node pops a full queue before it gives the fourth player a candidate, so that player's own candidate is made
        // after the launch and waits on a queue that no longer exists: kept as it is (RU5 report)
        if full {
            self.pop_queue(hunt_id).await?;
        }
        let candidate_id = crate::game_data::season_window::next_uuid().to_string();
        self.state()
            .set_candidate(player, MatchmakingResult::matching(hunt_id, candidate_id));
        Ok(true)
    }

    /// `HandlePartyMatchmaking`: a whole party joining one hunt lands in ONE instance. The first member creates it
    /// with the full party as the expected players; the others reuse its host and port.
    async fn handle_party_matchmaking(
        &self,
        hunt_id: &str,
        player: &str,
        party_id: &str,
        members: &[String],
    ) -> MatchResult<bool> {
        let reuse = {
            let now = self.now();
            let mut inner = self.state().lock();
            let created = inner.party_instance_created.get(party_id).copied().unwrap_or(0);
            match inner.party_instances.get(party_id).cloned() {
                // Only for the SAME hunt, and while the members are still following the leader in: a different hunt, or
                // a stale instance whose hunt has ended, launches a fresh one
                Some(existing) if existing.hunt_id == hunt_id && now - created < PARTY_INSTANCE_REUSE_MS => {
                    // The destination is party-wide; a candidate is one client's travel attempt
                    let mut mine = existing.clone();
                    mine.candidate_id = crate::game_data::season_window::next_uuid().to_string();
                    inner.results.insert(player.to_owned(), mine);
                    let key = format!("{}:{}", existing.host, existing.port);
                    Some(inner.hunt_ref_of_instance.get(&key).cloned().unwrap_or(key))
                }
                _ => None,
            }
        };
        if let Some(hunt_ref) = reuse {
            // A late follower of the shared instance is a participant too
            self.state()
                .activity
                .get()
                .dedicated_hunt_starts(&[player.to_owned()], hunt_id, &hunt_ref);
            return Ok(true);
        }
        let launch = self.launch_game(Some("ISLAND"), Some(""), hunt_id, members).await?;
        let Launch::Online { host, port } = launch else {
            let Launch::Refused { reason } = launch else {
                return Ok(false);
            };
            // Without a DeployServer the members are cancelled with the reason, not left waiting
            return Ok(match reason {
                Some(reason) => self.state().cancel_hunt(hunt_id, members, &reason),
                None => false,
            });
        };
        let mut inner = self.state().lock();
        let candidate_id = crate::game_data::season_window::next_uuid().to_string();
        let mut shared = MatchmakingResult::matching(hunt_id, candidate_id);
        shared.ready = true;
        shared.host = host;
        shared.port = port;
        shared.ready_at = self.go_now_release_at(&inner, false);
        tracing::info!(
            "Party {party_id} hunt '{hunt_id}' is online; synchronized Go Now release in {}ms for {} member(s)",
            shared.ready_at - self.now(),
            members.len()
        );
        let now = self.now();
        inner.party_instances.insert(party_id.to_owned(), shared.clone());
        inner.party_instance_created.insert(party_id.to_owned(), now);
        for member in members {
            let mut mine = shared.clone();
            mine.candidate_id = crate::game_data::season_window::next_uuid().to_string();
            inner.results.insert(member.clone(), mine);
        }
        Ok(true)
    }

    /// `GetPartyInstance`: the party's shared matchmaking state, which `POST /party` reflects.
    pub async fn party_instance(
        &self,
        party_id: &str,
        party_global_id: Option<&str>,
    ) -> MatchResult<Option<MatchmakingResult>> {
        let now = self.now();
        if let Some(result) = self.state().lock().party_instances.get(party_id) {
            return Ok(Some(result.client_visible(now)));
        }
        let Some(party) = party_global_id.and_then(rifts_party_id) else {
            return Ok(None);
        };
        let ext = seam(self.shared.rifts.realms().party_instance(party).await)?;
        Ok(ext.and_then(|c| parse_candidate(&c)))
    }

    /// `GetPlayerCandidate`.
    pub async fn player_candidate(&self, player: &str) -> MatchResult<Option<MatchmakingResult>> {
        let now = self.now();
        if let Some(result) = self.state().candidate(player) {
            return Ok(Some(result.client_visible(now)));
        }
        self.check_and_update_queue_status(player).await
    }

    /// `HandlePlayerMatchmaking`: true when the join was taken (a cancelled search counts), false for a 400.
    pub async fn handle_player_matchmaking(&self, join: JoinRequest) -> MatchResult<bool> {
        let mode = self.shared.settings.raw("MATCHMAKING_MODE").unwrap_or_default();
        match mode {
            "DISABLED" => {
                tracing::warn!("Matchmaking is disabled, refusing MM!");
                return Ok(false);
            }
            "DEPLOYSERVER" => {}
            _ => {
                tracing::error!("Unsupported MATCHMAKING_MODE!");
                return Ok(false);
            }
        }
        let effective = fallback_hunt_id(
            join.game_mode.as_deref(),
            join.game_args.as_deref(),
            join.hunt_id.as_deref(),
        );
        let everyone = || join.party_members.clone().unwrap_or_else(|| vec![join.player.clone()]);
        // Gauntlet is refused on every route until MYSTICPARADOX_GAUNTLET=on; the cancel reaches the requester's and
        // the party's candidate status like any other refusal
        if refuses_gauntlet_request(Some(&effective), &self.shared.settings) {
            tracing::warn!(
                "[HuntRoute] Gauntlet hunt {effective} refused reason={GAUNTLET_DISABLED_REASON} (MYSTICPARADOX_GAUNTLET is not on)"
            );
            return Ok(self
                .state()
                .cancel_hunt(&effective, &everyone(), GAUNTLET_DISABLED_REASON));
        }
        let is_hub = is_hub_destination(join.game_mode.as_deref(), Some(&effective));
        // An optional module may route the hunt itself (for example to a player host)
        let context = HuntRouteContext {
            game_mode: join.game_mode.clone().unwrap_or_default(),
            hunt_id: effective.clone(),
            player: RiftsAccount(join.player.clone()),
            party: join.party_global_id.as_deref().and_then(rifts_party_id),
            party_members: join.party_members.iter().flatten().cloned().map(RiftsAccount).collect(),
            party_revision: join.party_revision.and_then(|r| u64::try_from(r).ok()),
            build_id: join.build_id.clone(),
            is_hub,
        };
        if let HuntRouteOutcome::Handled { result } = seam(self.shared.rifts.realms().route_hunt(context).await)? {
            return Ok(result);
        }
        let party = match (&join.party_id, &join.party_members) {
            (Some(id), Some(members)) if members.len() > 1 => Some((id.as_str(), members.as_slice())),
            _ => None,
        };
        // [2026-07-17 party-follow fix] A party travelling to a NON-HUB hunt lands in ONE instance, routed before the
        // immediate-return path, which would register only the leader and strand the members
        if !is_hub
            && !effective.trim().is_empty()
            && let Some((party_id, members)) = party
        {
            return self
                .handle_party_matchmaking(&effective, &join.player, party_id, members)
                .await;
        }
        if effective.trim().is_empty() || !hunt_id_requires_matchmaking(&effective) {
            return self.immediate_instance(&join, &effective, party, is_hub).await;
        }
        if let Some((party_id, members)) = party {
            return self
                .handle_party_matchmaking(&effective, &join.player, party_id, members)
                .await;
        }
        self.queue_player(&effective, &join.player).await
    }

    /// The shared, already-running instance path (Ramsgate, the city hub, the dojo) and a non-CR19 hunt.
    async fn immediate_instance(
        &self,
        join: &JoinRequest,
        effective: &str,
        party: Option<(&str, &[String])>,
        is_hub: bool,
    ) -> MatchResult<bool> {
        let everyone = join.party_members.clone().unwrap_or_else(|| vec![join.player.clone()]);
        let launch = self
            .launch_game(
                join.game_mode.as_deref(),
                join.game_args.as_deref(),
                effective,
                &everyone,
            )
            .await?;
        let Launch::Online { host, port } = launch else {
            let Launch::Refused { reason } = launch else {
                return Ok(false);
            };
            return Ok(match reason {
                Some(reason) => self.state().cancel_hunt(effective, &everyone, &reason),
                None => false,
            });
        };
        // Travelling to a hub ends the party's dedicated hunts (completed or abandoned, once each)
        if is_hub {
            self.state().activity.get().dedicated_hunt_ends(&everyone, "RETURNED");
        }
        let mut inner = self.state().lock();
        let candidate_id = crate::game_data::season_window::next_uuid().to_string();
        let mut shared = MatchmakingResult::matching(effective, candidate_id);
        if is_hub {
            // The hub's room id is stable for the life of the persistent server; CandidateId is not reused for it
            shared.game_session_id = inner
                .hub_game_session_ids
                .entry(format!("{host}:{port}"))
                .or_insert_with(|| crate::game_data::season_window::next_uuid().to_string())
                .clone();
        }
        shared.ready = true;
        shared.host = host;
        shared.port = port;
        // Hubs get the same synchronised, load-aware countdown as hunts: consistent, and a burst of returning players
        // is spread across the release window instead of every client hitting the persistent server at once
        shared.ready_at = self.go_now_release_at(&inner, is_hub);
        tracing::info!(
            "{} '{effective}' is online; synchronized Go Now release in {}ms candidateId={} gameSessionId={}",
            if is_hub { "Hub" } else { "Hunt" },
            shared.ready_at - self.now(),
            shared.candidate_id,
            shared.game_session_id
        );
        let now = self.now();
        match party {
            // [2026-07-17 hub party-follow fix] A party returning to a shared hub fans out to EVERY member like a party
            // hunt, so all members follow the leader
            Some((party_id, members)) => {
                inner.party_instances.insert(party_id.to_owned(), shared.clone());
                inner.party_instance_created.insert(party_id.to_owned(), now);
                for member in members {
                    // Each client needs a fresh candidate to notice and consume this travel
                    let mut mine = shared.clone();
                    mine.candidate_id = crate::game_data::season_window::next_uuid().to_string();
                    inner.results.insert(member.clone(), mine);
                }
            }
            None => {
                if let Some(party_id) = &join.party_id {
                    inner.party_instances.remove(party_id);
                    inner.party_instance_created.remove(party_id);
                }
                inner.results.insert(join.player.clone(), shared);
            }
        }
        Ok(true)
    }

    /// `FormingHuntAccounts`: the requester and each party member whose candidate is the same shared instance.
    async fn forming_hunt_accounts(
        &self,
        player: &str,
        current: &MatchmakingResult,
        party_of: LeavePartyLookup<'_>,
    ) -> MatchResult<(Vec<String>, Option<String>)> {
        let Some(party) = party_of(player.to_owned()).await?.filter(|p| p.members.len() >= 2) else {
            return Ok((vec![player.to_owned()], None));
        };
        let inner = self.state().lock();
        let same_instance = inner
            .party_instances
            .get(&party.party_id)
            .is_some_and(|instance| instance.game_session_id == current.game_session_id);
        if !same_instance {
            return Ok((vec![player.to_owned()], None));
        }
        let mut accounts = vec![player.to_owned()];
        accounts.extend(
            party
                .members
                .iter()
                .filter(|member| {
                    *member != player
                        && inner
                            .results
                            .get(*member)
                            .is_some_and(|r| !r.canceled && r.game_session_id == current.game_session_id)
                })
                .cloned(),
        );
        Ok((accounts, Some(party.party_id)))
    }

    /// `CancelMatchmakingForLeave`: the client's Cancel (`DELETE /candidate/leave`). The client sends it only from the
    /// party leader, and the party's forming hunt is cancelled for every member. A search already told to travel is
    /// left alone (too late); a hub candidate is never cancelled; a second leave answers already-canceled.
    pub async fn cancel_matchmaking_for_leave(
        &self,
        player: &str,
        party_of: LeavePartyLookup<'_>,
    ) -> MatchResult<LeaveAnswer> {
        let account = RiftsAccount(player.to_owned());
        let current = match self.state().candidate(player) {
            Some(current) => current,
            None => {
                // A candidate the module owns may outlive this process
                let restored = seam(self.shared.rifts.realms().restore_candidate(account.clone()).await)?;
                let Some(restored) = restored.and_then(|c| parse_candidate(&c)) else {
                    return Ok(LeaveAnswer {
                        outcome: LeaveOutcome::NoCandidate,
                        state: "none".into(),
                        candidate_id: None,
                    });
                };
                self.state().set_candidate(player, restored.clone());
                restored
            }
        };
        let candidate_id = Some(current.candidate_id.clone());
        let answer = |outcome, state: &str| LeaveAnswer {
            outcome,
            state: state.to_owned(),
            candidate_id: candidate_id.clone(),
        };
        if let CandidateLeave::Handled { outcome, state } = seam(
            self.shared
                .rifts
                .realms()
                .cancel_candidate(account, to_candidate(&current))
                .await,
        )? {
            return Ok(answer(outcome.into(), &state));
        }
        if is_hub_hunt_id(Some(&current.hunt_id)) {
            return Ok(answer(LeaveOutcome::HubNotCancelled, "hub"));
        }
        if current.canceled {
            return Ok(answer(LeaveOutcome::AlreadyCanceled, "canceled"));
        }
        let now = self.now();
        let queued = self
            .state()
            .lock()
            .queues
            .get(&current.hunt_id)
            .is_some_and(|queue| queue.players.iter().any(|p| p == player));
        let state = if current.canceled {
            "canceled"
        } else if current.is_client_ready(now) {
            "ready"
        } else if current.ready {
            "go-now"
        } else if queued {
            "queued"
        } else {
            "matching"
        };
        let (accounts, party_id) = self.forming_hunt_accounts(player, &current, party_of).await?;
        let mut inner = self.state().lock();
        let told = accounts
            .iter()
            .any(|a| inner.results.get(a).is_some_and(|r| r.is_client_ready(now)));
        if told {
            return Ok(answer(LeaveOutcome::TooLate, state));
        }
        for member in &accounts {
            if let Some(result) = inner.results.get_mut(member) {
                result.ready = false;
                result.canceled = true;
                result.status_reason = Some(
                    if member == player {
                        "PLAYER_CANCELED"
                    } else {
                        "PARTY_MEMBER_CANCELED"
                    }
                    .to_owned(),
                );
                result.host.clear();
                result.port = 0;
                result.ready_at = 0;
            }
        }
        // A queue that is still collecting drops these players; a resolved one is already launching (PopQueue skips
        // cancelled players)
        if let Some(queue) = inner.queues.get_mut(&current.hunt_id)
            && !queue.resolved
        {
            queue.players.retain(|member| !accounts.contains(member));
            if queue.players.is_empty() {
                inner.queues.remove(&current.hunt_id);
            }
        }
        if let Some(party_id) = party_id {
            inner.party_instances.remove(&party_id);
            inner.party_instance_created.remove(&party_id);
        }
        tracing::info!(
            "Search for {} cancelled by its player: {} account(s), state={state}",
            current.hunt_id,
            accounts.len()
        );
        Ok(answer(LeaveOutcome::Canceled, state))
    }
}

/// A candidate as the realms hooks take it.
fn to_candidate(result: &MatchmakingResult) -> Candidate {
    Candidate(rifts_seam::Json(result.to_json().to_string()))
}

fn parse_candidate(candidate: &Candidate) -> Option<MatchmakingResult> {
    MatchmakingResult::from_json(&serde_json::from_str::<Value>(&candidate.0.0).ok()?)
}

/// The realms side names a party by the ULID and node of its global id (`<ULID>@<node>`).
pub fn rifts_party_id(global: &str) -> Option<rifts_seam::PartyId> {
    global.split_once(':').map_or(global, |(_, rest)| rest).parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fallback_hunt_id_follows_nodes_rules() {
        assert_eq!(fallback_hunt_id(Some("CITY"), None, None), RAMSGATE_HUNT);
        assert_eq!(fallback_hunt_id(Some("CITY"), None, Some("  ")), RAMSGATE_HUNT);
        assert_eq!(fallback_hunt_id(Some("ISLAND"), None, Some("X")), "X");
        assert_eq!(
            fallback_hunt_id(
                Some("ISLAND"),
                Some("/Game/Maps/islands/1705/dia_moss_triforce?x"),
                None
            ),
            "ShatteredIsles_IslandA"
        );
        assert_eq!(fallback_hunt_id(Some("ISLAND"), None, Some("  ")), "  ");
        assert_eq!(fallback_hunt_id(None, None, None), "");
    }

    #[test]
    fn hubs_are_known_by_mode_or_hunt_id() {
        assert!(is_hub_destination(Some("CITY"), Some("anything")));
        assert!(is_hub_destination(Some("ISLAND"), Some("ShatteredIsles_TrainingDojo")));
        assert!(is_hub_hunt_id(Some(RAMSGATE_HUNT)));
        assert!(!is_hub_destination(Some("ISLAND"), Some("ShatteredIsles_IslandA")));
        assert!(!is_hub_hunt_id(None));
        assert!(hunt_id_requires_matchmaking("CR19_PlayerHunt_X"));
        assert!(!hunt_id_requires_matchmaking("ShatteredIsles_IslandA"));
    }
}
