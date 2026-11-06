//! Leader election for a network's nodes (`docs/distributed-mode.md`): who leads the directory, chosen by
//! the nodes themselves so the network keeps registering accounts when the leader goes down.
//!
//! Raft's election (Ongaro, sections 3.4 and 9.6) over [`Service::Election`]: numbered terms, one vote per
//! node per term kept on disk before it is given, a majority of every voter to win, and a vote only for a
//! candidate whose log is at least as new as the voter's ([`LogPosition`]), so a new leader holds every
//! entry a majority had. Three guards from the thesis keep a flapping link from deposing a healthy leader:
//!
//! - **Pre-vote:** a node first asks whether it would win, without raising its term, so a node cut off by a
//!   partition can't come back with a term that unseats the leader.
//! - **Leader stickiness:** a node that heard from its leader within the shortest election timeout ignores
//!   candidates, pre-vote or not.
//! - **Check-quorum:** a leader that hasn't heard back from a majority within the longest election timeout
//!   steps down, so the side of a partition without a majority never keeps one.
//!
//! The caller is the node the link authenticated (DN3), never a name in the body, and only a listed voter
//! is answered. What the election decides is published on a [`tokio::sync::watch`] channel
//! ([`Election::subscribe`]); the directory follows it ([`crate::Directory::follow`]).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use module_api::BoxFuture;
use module_api::network::NodeId;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::link::{LinkError, MAX_MESSAGE, NodeService, PeerLink, Service};

/// The election protocol's version; another is refused before its body is read.
pub const ELECTION_VERSION: u32 = 1;

/// A node's log as a vote compares it: the term of its newest entry, then that entry's number.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogPosition {
    /// The term the newest entry was written in.
    pub term: u64,
    /// The newest entry's number.
    pub index: u64,
}

/// Where a voter's log stands, asked before every vote.
pub trait LogSource: Send + Sync {
    /// This node's newest entry.
    fn position(&self) -> BoxFuture<'_, Result<LogPosition, String>>;
}

/// A log that stays empty: a node with nothing to replicate.
#[derive(Debug, Clone, Copy, Default)]
pub struct EmptyLog;

impl LogSource for EmptyLog {
    fn position(&self) -> BoxFuture<'_, Result<LogPosition, String>> {
        Box::pin(async { Ok(LogPosition::default()) })
    }
}

/// What a node must remember across restarts: its term and whom it voted for in it (Raft 3.8).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HardState {
    /// The newest term this node knows.
    pub term: u64,
    /// The node it voted for in `term`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voted_for: Option<NodeId>,
}

/// Where [`HardState`] is kept. A save returns only once the state would survive a crash.
pub trait TermStore: Send + Sync {
    /// The state saved last; the default before the first save.
    fn load(&self) -> Result<HardState, String>;
    /// Saves `state`.
    fn save(&self, state: &HardState) -> Result<(), String>;
}

/// [`HardState`] in a JSON file, replaced whole through a synced temporary file.
#[derive(Debug, Clone)]
pub struct FileTermStore {
    path: PathBuf,
}

impl FileTermStore {
    /// The state at `path`; its folder is made on the first save.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl TermStore for FileTermStore {
    fn load(&self) -> Result<HardState, String> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", self.path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HardState::default()),
            Err(e) => Err(format!("{}: {e}", self.path.display())),
        }
    }

    fn save(&self, state: &HardState) -> Result<(), String> {
        use std::io::Write;
        let fail = |e: std::io::Error| format!("{}: {e}", self.path.display());
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(fail)?;
        }
        let body = serde_json::to_vec(state).map_err(|e| e.to_string())?;
        let temp = self.path.with_extension("tmp");
        let mut file = std::fs::File::create(&temp).map_err(fail)?;
        file.write_all(&body).map_err(fail)?;
        // A vote must not be forgotten by a crash right after it was given (Raft 3.8)
        file.sync_all().map_err(fail)?;
        drop(file);
        std::fs::rename(&temp, &self.path).map_err(fail)
    }
}

/// [`HardState`] in memory, for tests: a restart that keeps the store keeps the state.
#[derive(Debug, Default)]
pub struct MemoryTermStore(Mutex<HardState>);

impl TermStore for MemoryTermStore {
    fn load(&self) -> Result<HardState, String> {
        Ok(self.0.lock().unwrap_or_else(PoisonError::into_inner).clone())
    }

    fn save(&self, state: &HardState) -> Result<(), String> {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = state.clone();
        Ok(())
    }
}

/// How an [`Election`] is run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElectionConfig {
    /// This node.
    pub node: NodeId,
    /// The other voters: every node of `[[network.nodes]]`.
    pub peers: Vec<NodeId>,
    /// A follower that hears nothing from a leader for a random time in this range stands.
    pub election_timeout: (Duration, Duration),
    /// How often a leader tells the others it still leads.
    pub heartbeat: Duration,
    /// How long one election call may take, connecting included.
    pub call_timeout: Duration,
}

impl ElectionConfig {
    /// Timeouts fit for nodes in different regions over Tailscale or the internet: a heartbeat a second, an
    /// election after 5 to 10 s of silence, so a failover takes about 10 s and a 200 ms link never triggers one.
    pub fn new(node: NodeId, peers: Vec<NodeId>) -> Self {
        Self {
            node,
            peers,
            election_timeout: (Duration::from_secs(5), Duration::from_secs(10)),
            heartbeat: Duration::from_secs(1),
            call_timeout: Duration::from_secs(2),
        }
    }

    /// Votes a candidate needs: a majority of every voter, this node included.
    pub fn quorum(&self) -> usize {
        self.peers.len().div_ceil(2) + 1
    }

    /// Whether the timeouts can work: a heartbeat well inside the shortest timeout, the range in order.
    pub fn check(&self) -> Result<(), String> {
        let (low, high) = self.election_timeout;
        if self.heartbeat.is_zero() || low < self.heartbeat * 3 {
            return Err("the shortest election timeout must be at least three heartbeats".into());
        }
        if high < low {
            return Err("the election timeout range is backwards".into());
        }
        if self.call_timeout.is_zero() || self.call_timeout >= low {
            return Err("an election call must time out before an election does".into());
        }
        if self.peers.contains(&self.node) {
            return Err(format!("{} lists itself as a voter", self.node));
        }
        Ok(())
    }
}

/// Who leads, as this node knows it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Leadership {
    /// The term it leads in.
    pub term: u64,
    /// The leader; none while an election runs.
    pub leader: Option<NodeId>,
}

/// What this node is doing in the election.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ElectionRole {
    /// Following a leader, or waiting for one.
    Follower,
    /// Asking for votes.
    Candidate,
    /// Leading.
    Leader,
}

/// One election call as sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElectionRequest {
    /// [`ELECTION_VERSION`].
    pub version: u32,
    /// The call.
    pub call: ElectionCall,
}

/// What one node asks another. The candidate or leader is the caller the link authenticated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ElectionCall {
    /// Would you vote for me in `term`? Changes nothing at the voter.
    PreVote {
        /// The term the caller would stand in.
        term: u64,
        /// The caller's log.
        last: LogPosition,
    },
    /// Vote for me in `term`.
    Vote {
        /// The caller's new term.
        term: u64,
        /// The caller's log.
        last: LogPosition,
    },
    /// I lead in `term`.
    Heartbeat {
        /// The leader's term.
        term: u64,
    },
}

/// One election answer as sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElectionResponse {
    /// [`ELECTION_VERSION`].
    pub version: u32,
    /// The answer.
    pub reply: ElectionReply,
}

/// The answer to an [`ElectionCall`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ElectionReply {
    /// A vote or pre-vote, and the voter's term.
    Vote {
        /// The voter's term after the call.
        term: u64,
        /// Whether it voted for the caller.
        granted: bool,
    },
    /// A heartbeat's answer: `ok` when the caller is this node's leader for `term`.
    Heartbeat {
        /// The follower's term after the call.
        term: u64,
        /// Whether it follows the caller.
        ok: bool,
    },
    /// Not answered: the caller isn't a voter, the version or the body is wrong, or the state can't be saved.
    Refused {
        /// Why, for the caller's log.
        reason: String,
    },
}

/// What a node reports of the election, for status pages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ElectionStatus {
    /// This node's term.
    pub term: u64,
    /// What it is doing.
    pub role: ElectionRole,
    /// The leader it knows of.
    pub leader: Option<NodeId>,
    /// Votes a leader needs.
    pub quorum: usize,
    /// Every voter.
    pub voters: usize,
}

struct State {
    hard: HardState,
    role: ElectionRole,
    leader: Option<NodeId>,
    /// When this node last heard from a leader it accepted, or gave a vote
    heard: Instant,
    /// When this follower stands if it hears nothing more
    deadline: Instant,
    /// A leader's last answer from each follower
    acks: BTreeMap<NodeId, Instant>,
}

/// One node's side of the election.
pub struct Election {
    config: ElectionConfig,
    link: Arc<dyn PeerLink>,
    log: Arc<dyn LogSource>,
    store: Arc<dyn TermStore>,
    state: Mutex<State>,
    published: watch::Sender<Leadership>,
}

fn random_between(low: Duration, high: Duration) -> Duration {
    if high <= low {
        return low;
    }
    let mut bytes = [0u8; 8];
    // Without randomness every node would stand at once, every time; the low end still works
    if rustls::crypto::aws_lc_rs::default_provider()
        .secure_random
        .fill(&mut bytes)
        .is_err()
    {
        return low;
    }
    let span = u64::try_from((high - low).as_millis()).unwrap_or(u64::MAX).max(1);
    low + Duration::from_millis(u64::from_le_bytes(bytes) % span)
}

impl Election {
    /// The election of `config.node`, from the term and vote `store` holds. Nothing runs until
    /// [`spawn`](Self::spawn).
    pub fn new(
        config: ElectionConfig,
        link: Arc<dyn PeerLink>,
        log: Arc<dyn LogSource>,
        store: Arc<dyn TermStore>,
    ) -> Result<Arc<Self>, String> {
        config.check()?;
        let hard = store.load()?;
        let now = Instant::now();
        let (published, _) = watch::channel(Leadership {
            term: hard.term,
            leader: None,
        });
        let deadline = now + random_between(config.election_timeout.0, config.election_timeout.1);
        Ok(Arc::new(Self {
            state: Mutex::new(State {
                hard,
                role: ElectionRole::Follower,
                leader: None,
                // A restarted node waits a full timeout before standing, as if it had just heard a leader
                heard: now,
                deadline,
                acks: BTreeMap::new(),
            }),
            config,
            link,
            log,
            store,
            published,
        }))
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// This node.
    pub fn node(&self) -> &NodeId {
        &self.config.node
    }

    /// Who leads now, as this node knows it.
    pub fn leadership(&self) -> Leadership {
        self.published.borrow().clone()
    }

    /// Every change of leader or term from now on.
    pub fn subscribe(&self) -> watch::Receiver<Leadership> {
        self.published.subscribe()
    }

    /// The election as this node sees it.
    pub fn status(&self) -> ElectionStatus {
        let state = self.state();
        ElectionStatus {
            term: state.hard.term,
            role: state.role,
            leader: state.leader.clone(),
            quorum: self.config.quorum(),
            voters: self.config.peers.len() + 1,
        }
    }

    fn publish(&self, state: &State) {
        let next = Leadership {
            term: state.hard.term,
            leader: state.leader.clone(),
        };
        self.published.send_if_modified(|current| {
            if *current == next {
                return false;
            }
            tracing::info!(term = next.term, leader = ?next.leader.as_ref().map(NodeId::as_str), "election: leadership changed");
            *current = next;
            true
        });
    }

    fn reset_deadline(&self, state: &mut State, now: Instant) {
        let (low, high) = self.config.election_timeout;
        state.deadline = now + random_between(low, high);
    }

    /// Moves to `term` as a follower, forgetting the vote; saved before anything else is answered.
    fn adopt_term(&self, state: &mut State, term: u64) -> Result<(), String> {
        if term > state.hard.term {
            let next = HardState { term, voted_for: None };
            self.store.save(&next)?;
            state.hard = next;
            state.leader = None;
        }
        if state.role != ElectionRole::Follower {
            state.role = ElectionRole::Follower;
            state.acks.clear();
        }
        Ok(())
    }

    /// Runs the election for as long as it lives: heartbeats while leading, a vote when the leader goes quiet.
    pub fn spawn(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let election = Arc::downgrade(self);
        let tick = (self.config.heartbeat / 4).max(Duration::from_millis(10));
        tokio::spawn(async move {
            let mut next_beat = Instant::now();
            loop {
                let Some(election) = election.upgrade() else {
                    break;
                };
                let now = Instant::now();
                let (role, deadline) = {
                    let state = election.state();
                    (state.role, state.deadline)
                };
                match role {
                    ElectionRole::Leader if now >= next_beat => {
                        next_beat = now + election.config.heartbeat;
                        election.beat().await;
                    }
                    ElectionRole::Leader => {}
                    ElectionRole::Follower | ElectionRole::Candidate if now >= deadline => {
                        election.stand().await;
                        next_beat = Instant::now();
                    }
                    ElectionRole::Follower | ElectionRole::Candidate => {}
                }
                drop(election);
                tokio::time::sleep(tick).await;
            }
        })
    }

    async fn ask(&self, to: NodeId, call: ElectionCall) -> Option<(NodeId, ElectionReply)> {
        let body = serde_json::to_vec(&ElectionRequest {
            version: ELECTION_VERSION,
            call,
        })
        .ok()?;
        let answer = tokio::time::timeout(self.config.call_timeout, self.link.call(&to, Service::Election, body))
            .await
            .ok()?;
        let answer = match answer {
            Ok(answer) => answer,
            Err(error) => {
                tracing::debug!(peer = %to, %error, "election: no answer");
                return None;
            }
        };
        let response: ElectionResponse = serde_json::from_slice(&answer).ok()?;
        (response.version == ELECTION_VERSION).then_some((to, response.reply))
    }

    /// Asks every peer at once and gathers whatever comes back within the call timeout.
    async fn ask_all(&self, call: &ElectionCall) -> Vec<(NodeId, ElectionReply)> {
        let calls = self
            .config
            .peers
            .iter()
            .map(|peer| self.ask(peer.clone(), call.clone()));
        let mut answers = Vec::new();
        for answer in join_all(calls).await.into_iter().flatten() {
            answers.push(answer);
        }
        answers
    }

    /// One heartbeat round, and the check-quorum that may end this node's leadership.
    async fn beat(&self) {
        let term = self.state().hard.term;
        let answers = self.ask_all(&ElectionCall::Heartbeat { term }).await;
        let now = Instant::now();
        let mut state = self.state();
        if state.role != ElectionRole::Leader || state.hard.term != term {
            return;
        }
        for (peer, reply) in answers {
            match reply {
                ElectionReply::Heartbeat { term: theirs, .. } if theirs > term => {
                    tracing::info!(peer = %peer, term = theirs, "election: a newer term; stepping down");
                    if let Err(error) = self.adopt_term(&mut state, theirs) {
                        tracing::error!(%error, "election: can't save the term");
                    }
                    self.reset_deadline(&mut state, now);
                    self.publish(&state);
                    return;
                }
                ElectionReply::Heartbeat { ok: true, .. } => {
                    state.acks.insert(peer, now);
                }
                _ => {}
            }
        }
        let window = self.config.election_timeout.1;
        let heard = 1 + state
            .acks
            .values()
            .filter(|at| now.duration_since(**at) <= window)
            .count();
        if heard < self.config.quorum() && now.duration_since(state.heard) > window {
            tracing::warn!(
                term,
                heard,
                quorum = self.config.quorum(),
                "election: lost touch with a majority; stepping down"
            );
            state.role = ElectionRole::Follower;
            state.leader = None;
            state.acks.clear();
            self.reset_deadline(&mut state, now);
            self.publish(&state);
        }
    }

    /// A pre-vote, then, if it would win, the vote.
    async fn stand(&self) {
        let position = match self.log.position().await {
            Ok(position) => position,
            Err(error) => {
                tracing::error!(%error, "election: can't read this node's log; not standing");
                let mut state = self.state();
                self.reset_deadline(&mut state, Instant::now());
                return;
            }
        };
        let term = {
            let mut state = self.state();
            self.reset_deadline(&mut state, Instant::now());
            // Silent for a whole timeout: whoever led is gone as far as this node can tell
            if state.role == ElectionRole::Follower && state.leader.take().is_some() {
                self.publish(&state);
            }
            state.hard.term
        };
        if !self.config.peers.is_empty() {
            let answers = self
                .ask_all(&ElectionCall::PreVote {
                    term: term + 1,
                    last: position,
                })
                .await;
            let granted = 1 + answers
                .iter()
                .filter(|(_, reply)| matches!(reply, ElectionReply::Vote { granted: true, .. }))
                .count();
            if granted < self.config.quorum() {
                tracing::debug!(term = term + 1, granted, "election: pre-vote lost");
                return;
            }
        }
        let term = {
            let mut state = self.state();
            if state.hard.term != term || state.role == ElectionRole::Leader {
                return;
            }
            let next = HardState {
                term: term + 1,
                voted_for: Some(self.config.node.clone()),
            };
            if let Err(error) = self.store.save(&next) {
                tracing::error!(%error, "election: can't save the term; not standing");
                return;
            }
            state.hard = next;
            state.role = ElectionRole::Candidate;
            state.leader = None;
            self.publish(&state);
            term + 1
        };
        let answers = self.ask_all(&ElectionCall::Vote { term, last: position }).await;
        let now = Instant::now();
        let mut state = self.state();
        if state.hard.term != term || state.role != ElectionRole::Candidate {
            return;
        }
        let mut granted = 1;
        for (peer, reply) in answers {
            match reply {
                ElectionReply::Vote { term: theirs, .. } if theirs > term => {
                    if let Err(error) = self.adopt_term(&mut state, theirs) {
                        tracing::error!(%error, "election: can't save the term");
                    }
                    self.publish(&state);
                    return;
                }
                ElectionReply::Vote { granted: true, .. } => {
                    granted += 1;
                    state.acks.insert(peer, now);
                }
                _ => {}
            }
        }
        if granted >= self.config.quorum() {
            tracing::info!(term, granted, "election: won; leading");
            state.role = ElectionRole::Leader;
            state.leader = Some(self.config.node.clone());
            state.heard = now;
        } else {
            tracing::debug!(term, granted, "election: lost");
            state.role = ElectionRole::Follower;
            state.acks.clear();
        }
        self.publish(&state);
    }

    /// Answers a peer's call. `from` is the caller as its link authenticated it.
    pub async fn handle(&self, from: &NodeId, call: ElectionCall) -> ElectionReply {
        if !self.config.peers.contains(from) {
            return ElectionReply::Refused {
                reason: format!("{from} is not a voter here"),
            };
        }
        let position = match &call {
            ElectionCall::PreVote { .. } | ElectionCall::Vote { .. } => match self.log.position().await {
                Ok(position) => Some(position),
                Err(error) => {
                    tracing::error!(%error, "election: can't read this node's log");
                    return ElectionReply::Refused {
                        reason: "unavailable".into(),
                    };
                }
            },
            ElectionCall::Heartbeat { .. } => None,
        };
        let now = Instant::now();
        let mut state = self.state();
        let sticky = state.leader.is_some()
            && state.leader.as_ref() != Some(from)
            && now.duration_since(state.heard) < self.config.election_timeout.0;
        let result = match call {
            ElectionCall::PreVote { term, last } => {
                let granted = !sticky
                    && state.role != ElectionRole::Leader
                    && term > state.hard.term
                    && position.is_some_and(|mine| last >= mine);
                Ok(ElectionReply::Vote {
                    term: state.hard.term,
                    granted,
                })
            }
            ElectionCall::Vote { term, last } => self.vote(&mut state, from, term, last, position, sticky, now),
            ElectionCall::Heartbeat { term } => self.follow(&mut state, from, term, now),
        };
        self.publish(&state);
        result.unwrap_or_else(|error| {
            tracing::error!(%error, "election: can't save the term");
            ElectionReply::Refused {
                reason: "unavailable".into(),
            }
        })
    }

    #[allow(clippy::too_many_arguments, reason = "one vote's inputs, all from handle")]
    fn vote(
        &self,
        state: &mut State,
        from: &NodeId,
        term: u64,
        last: LogPosition,
        position: Option<LogPosition>,
        sticky: bool,
        now: Instant,
    ) -> Result<ElectionReply, String> {
        // Raft 4.2.3: a node that hears its leader ignores candidates, without even taking their term
        if sticky || term < state.hard.term {
            return Ok(ElectionReply::Vote {
                term: state.hard.term,
                granted: false,
            });
        }
        if term > state.hard.term {
            self.adopt_term(state, term)?;
        }
        let free = state.hard.voted_for.as_ref().is_none_or(|voted| voted == from);
        let granted = free && position.is_some_and(|mine| last >= mine);
        if granted && state.hard.voted_for.is_none() {
            let next = HardState {
                term,
                voted_for: Some(from.clone()),
            };
            self.store.save(&next)?;
            state.hard = next;
        }
        if granted {
            state.heard = now;
            self.reset_deadline(state, now);
        }
        Ok(ElectionReply::Vote {
            term: state.hard.term,
            granted,
        })
    }

    fn follow(&self, state: &mut State, from: &NodeId, term: u64, now: Instant) -> Result<ElectionReply, String> {
        if term < state.hard.term {
            return Ok(ElectionReply::Heartbeat {
                term: state.hard.term,
                ok: false,
            });
        }
        if term > state.hard.term || state.role != ElectionRole::Follower {
            self.adopt_term(state, term)?;
        }
        state.leader = Some(from.clone());
        state.heard = now;
        self.reset_deadline(state, now);
        Ok(ElectionReply::Heartbeat { term, ok: true })
    }
}

impl NodeService for Election {
    fn serve<'a>(
        &'a self,
        from: &'a NodeId,
        service: Service,
        body: Vec<u8>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LinkError>> {
        Box::pin(async move {
            if service != Service::Election {
                return Err(LinkError::NoService {
                    node: self.config.node.clone(),
                    service,
                });
            }
            let reply = if body.len() > MAX_MESSAGE {
                ElectionReply::Refused {
                    reason: "too large".into(),
                }
            } else {
                match serde_json::from_slice::<ElectionRequest>(&body) {
                    Ok(request) if request.version == ELECTION_VERSION => self.handle(from, request.call).await,
                    Ok(request) => ElectionReply::Refused {
                        reason: format!("election protocol {ELECTION_VERSION} only, not {}", request.version),
                    },
                    Err(_) => ElectionReply::Refused {
                        reason: "not an election message".into(),
                    },
                }
            };
            serde_json::to_vec(&ElectionResponse {
                version: ELECTION_VERSION,
                reply,
            })
            .map_err(|_| LinkError::TooLarge)
        })
    }
}

/// Polls every future to the end together; the crate has no `futures` dependency.
pub(crate) async fn join_all<F: std::future::Future>(futures: impl IntoIterator<Item = F>) -> Vec<F::Output> {
    let mut pending: Vec<std::pin::Pin<Box<F>>> = futures.into_iter().map(Box::pin).collect();
    let mut out: Vec<Option<F::Output>> = pending.iter().map(|_| None).collect();
    std::future::poll_fn(|cx| {
        let mut busy = false;
        for (slot, future) in out.iter_mut().zip(pending.iter_mut()) {
            if slot.is_none() {
                match future.as_mut().poll(cx) {
                    std::task::Poll::Ready(value) => *slot = Some(value),
                    std::task::Poll::Pending => busy = true,
                }
            }
        }
        if busy {
            std::task::Poll::Pending
        } else {
            std::task::Poll::Ready(())
        }
    })
    .await;
    pending.clear();
    out.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;
    use crate::inprocess::InProcessNetwork;

    fn id(name: &str) -> NodeId {
        NodeId::new(name).unwrap()
    }

    fn fast(node: &str, all: &[&str]) -> ElectionConfig {
        ElectionConfig {
            node: id(node),
            peers: all.iter().filter(|n| **n != node).map(|n| id(n)).collect(),
            election_timeout: (Duration::from_millis(150), Duration::from_millis(300)),
            heartbeat: Duration::from_millis(30),
            call_timeout: Duration::from_millis(100),
        }
    }

    struct Node {
        name: &'static str,
        election: Arc<Election>,
        store: Arc<MemoryTermStore>,
        task: tokio::task::JoinHandle<()>,
    }

    fn start(net: &InProcessNetwork, name: &'static str, all: &[&'static str], store: Arc<MemoryTermStore>) -> Node {
        let election = Election::new(
            fast(name, all),
            Arc::new(net.link(&id(name))),
            Arc::new(EmptyLog),
            store.clone(),
        )
        .unwrap();
        let service: Arc<dyn NodeService> = election.clone();
        net.attach(&id(name), &service);
        let task = election.spawn();
        Node {
            name,
            election,
            store,
            task,
        }
    }

    async fn settled(nodes: &[&Node]) -> Leadership {
        settled_without(nodes, None).await
    }

    async fn settled_without(nodes: &[&Node], gone: Option<&NodeId>) -> Leadership {
        for _ in 0..200 {
            let views: Vec<Leadership> = nodes.iter().map(|n| n.election.leadership()).collect();
            if let Some(first) = views.first()
                && first.leader.is_some()
                && first.leader.as_ref() != gone
                && views.iter().all(|v| v == first)
            {
                return first.clone();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("no leader agreed on");
    }

    const THREE: [&str; 3] = ["a-1", "b-1", "c-1"];

    #[tokio::test]
    async fn three_nodes_agree_on_one_leader_and_replace_it_when_it_goes() {
        let net = InProcessNetwork::new();
        let nodes: Vec<Node> = THREE.iter().map(|n| start(&net, n, &THREE, Arc::default())).collect();
        let first = settled(&nodes.iter().collect::<Vec<_>>()).await;
        let leader = first.leader.clone().unwrap();
        assert_eq!(
            nodes
                .iter()
                .filter(|n| n.election.status().role == ElectionRole::Leader)
                .count(),
            1
        );

        net.isolate(&leader);
        let rest: Vec<&Node> = nodes.iter().filter(|n| id(n.name) != leader).collect();
        let second = settled_without(&rest, Some(&leader)).await;
        assert!(second.term > first.term);
        assert_ne!(second.leader, Some(leader.clone()));

        // Check-quorum: the cut-off leader gives up on its own
        let old = nodes.iter().find(|n| id(n.name) == leader).unwrap();
        for _ in 0..100 {
            if old.election.status().role != ElectionRole::Leader {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_ne!(old.election.status().role, ElectionRole::Leader);

        // Pre-vote: back on the network, it follows instead of unseating the new leader
        net.heal();
        let third = settled(&nodes.iter().collect::<Vec<_>>()).await;
        assert_eq!(third.leader, second.leader);
        assert_eq!(third.term, second.term);
    }

    #[tokio::test]
    async fn a_minority_never_elects() {
        let net = InProcessNetwork::new();
        let nodes: Vec<Node> = THREE.iter().map(|n| start(&net, n, &THREE, Arc::default())).collect();
        net.isolate(&id("a-1"));
        net.isolate(&id("b-1"));
        net.isolate(&id("c-1"));
        tokio::time::sleep(Duration::from_millis(900)).await;
        for node in &nodes {
            assert_eq!(node.election.leadership().leader, None, "{}", node.name);
            // Pre-vote keeps an isolated node's term from climbing
            assert_eq!(node.election.status().term, 0, "{}", node.name);
        }
    }

    #[tokio::test]
    async fn a_vote_is_kept_across_a_restart() {
        let net = InProcessNetwork::new();
        let store = Arc::new(MemoryTermStore::default());
        let node = start(&net, "a-1", &THREE, store.clone());
        let candidate = id("b-1");
        let granted = node
            .election
            .handle(
                &candidate,
                ElectionCall::Vote {
                    term: 4,
                    last: LogPosition::default(),
                },
            )
            .await;
        assert_eq!(granted, ElectionReply::Vote { term: 4, granted: true });
        node.task.abort();
        drop(node);

        let again = start(&net, "a-1", &THREE, store);
        let other = again
            .election
            .handle(
                &id("c-1"),
                ElectionCall::Vote {
                    term: 4,
                    last: LogPosition::default(),
                },
            )
            .await;
        assert_eq!(
            other,
            ElectionReply::Vote {
                term: 4,
                granted: false
            }
        );
        assert_eq!(again.store.load().unwrap().voted_for, Some(candidate));
    }

    #[tokio::test]
    async fn a_voter_refuses_an_older_log_and_a_stranger() {
        struct At(LogPosition);
        impl LogSource for At {
            fn position(&self) -> BoxFuture<'_, Result<LogPosition, String>> {
                let position = self.0;
                Box::pin(async move { Ok(position) })
            }
        }
        let net = InProcessNetwork::new();
        let mine = LogPosition { term: 3, index: 10 };
        let election = Election::new(
            fast("a-1", &THREE),
            Arc::new(net.link(&id("a-1"))),
            Arc::new(At(mine)),
            Arc::new(MemoryTermStore::default()),
        )
        .unwrap();
        let older = LogPosition { term: 2, index: 50 };
        let reply = election
            .handle(&id("b-1"), ElectionCall::Vote { term: 5, last: older })
            .await;
        assert_eq!(
            reply,
            ElectionReply::Vote {
                term: 5,
                granted: false
            }
        );
        let reply = election
            .handle(&id("c-1"), ElectionCall::Vote { term: 5, last: mine })
            .await;
        assert_eq!(reply, ElectionReply::Vote { term: 5, granted: true });
        let reply = election.handle(&id("z-9"), ElectionCall::Heartbeat { term: 9 }).await;
        assert!(matches!(reply, ElectionReply::Refused { .. }));
        assert_eq!(election.status().term, 5);
    }

    #[tokio::test]
    async fn a_lone_node_leads_itself() {
        let net = InProcessNetwork::new();
        let node = start(&net, "a-1", &["a-1"], Arc::default());
        let leadership = settled(&[&node]).await;
        assert_eq!(leadership.leader, Some(id("a-1")));
        assert_eq!(leadership.term, 1);
    }

    #[test]
    fn the_file_store_round_trips_and_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileTermStore::new(dir.path().join("state").join("election.json"));
        assert_eq!(store.load().unwrap(), HardState::default());
        let state = HardState {
            term: 7,
            voted_for: Some(id("b-1")),
        };
        store.save(&state).unwrap();
        assert_eq!(store.load().unwrap(), state);
    }

    #[test]
    fn bad_timeouts_are_refused() {
        let mut config = fast("a-1", &THREE);
        config.heartbeat = Duration::from_millis(100);
        assert!(config.check().is_err());
        let mut config = fast("a-1", &THREE);
        config.call_timeout = Duration::from_secs(1);
        assert!(config.check().is_err());
        assert_eq!(fast("a-1", &THREE).quorum(), 2);
        assert_eq!(ElectionConfig::new(id("a-1"), vec![]).quorum(), 1);
        assert!(ElectionConfig::new(id("a-1"), vec![id("b-1")]).check().is_ok());
    }
}
