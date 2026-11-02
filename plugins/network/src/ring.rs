//! Instance registry and redirects (DN6, DD18), the matchmaking ring (DN8, DD19), and regional
//! Ramsgate hubs (DN9, DD20) over [`Service::Ring`].
//!
//! - **DN6 (Instance registry & redirects)**: Every hunt, hub instance, party, and matchmaking ticket carries
//!   a [`GlobalId`] (`<kind>:<ULID>@<node>`) whose [`GlobalId::owner`] names the owning node without asking
//!   anyone. When a player's join reaches a node that does not own the instance, [`RingNode::resolve_join`]
//!   answers according to [`WrongNodePolicy`]:
//!   - [`WrongNodePolicy::Redirect`]: returns the owner node's [`PeerGateway`] and a short-lived
//!     home-signed [`SessionTicket`] for `audience = owner`, allowing the launcher relay on
//!     `127.0.0.1:61100` to re-point to the owner's gateway without exposing the account key.
//!   - [`WrongNodePolicy::Refuse`]: returns a refusal naming the owning node and region (the fallback if
//!     spike DN1 shows the 1.14.7 client drops sessions across a mid-session relay switch).
//! - **DN8 (Matchmaking ring)**: The launcher's `POST /network/v1/regions` report is stored per account after
//!   verifying the account key at its home (or a valid [`SessionTicket`]). A matchmaking ticket is owned by
//!   the player's home node, published to the player's lowest-ping region at step 0, and widened to the next
//!   region after each delay in [`RingSchedule`]. A region that forms a match claims each ticket from its
//!   home via an atomic compare-and-set ([`RingCall::ClaimTicket`]), starts the hunt on its
//!   [`DirectorPlacement`], admits the matched players' addresses through [`AllowlistSink`], and forwards
//!   post-hunt XP, loot, and objective progress to each player's home via [`HomeApi::forward`] using
//!   [`module_api::network::operations`].
//! - **DN9 (Regional Ramsgate)**: One shared Ramsgate hub per region, hosted by that region's node and split
//!   into shards when an instance reaches [`RingConfig::max_hub_players`]. Players from any home node are
//!   assigned to the Ramsgate of their lowest-ping region.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

pub use module_api::network::{
    DIRECTOR_COMMAND, DirectorInvocation, HuntRewards, InstanceRecord, JoinDecision, PeerGateway, PlacedInstance,
    PublishedTicket, RegionPing, TicketState,
};
use module_api::network::{
    GlobalId, HomeApi, HomeError, HomeForward, IdKind, NodeId, NodeIdentity, OperationName, Region, SessionTicket,
    operations,
};
use module_api::{ActorContext, BoxFuture, GameVersion, ModuleDriver, ModuleDriverExt, ModuleId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::home::protocol::{holds_account_key, looks_like_account_key};
use crate::link::{Clock, LinkError, NodeService, PeerLink, Service};

/// Wire protocol version for [`Service::Ring`].
pub const RING_VERSION: u32 = 1;

/// Maximum wire size for a single [`Service::Ring`] message (64 KiB).
pub const MAX_RING_MESSAGE: usize = 64 * 1024;

/// Maximum length of a queue name, host name, or refusal string on the wire.
pub const MAX_RING_TEXT: usize = 256;

/// Default maximum players in a single Ramsgate hub shard before a new shard is started (DD20, DN9).
pub const DEFAULT_MAX_HUB_PLAYERS: u16 = 24;

/// Default widening ring delays in milliseconds (DD19, DN8): step 0 immediate, step 1 at 5s, step 2 at 10s,
/// step 3 at 15s.
pub const DEFAULT_RING_DELAYS_MS: [u64; 4] = [0, 5_000, 10_000, 15_000];

/// Minimum wait time in milliseconds before a partial match batch (`>= 2` players) forms a hunt when fewer
/// than `match_size` tickets are waiting in the regional pool (DN8).
pub const PARTIAL_MATCH_MIN_WAIT_MS: i64 = 1_000;

/// How a node handles a join request for a [`GlobalId`] owned by a different node (DN6, DN1).
///
/// Defaults to [`WrongNodePolicy::Refuse`] (the DN1 kill criterion fallback) until spike DN1 proves the
/// 1.14.7 client survives a mid-session relay switch, or `[network] wrong_node_policy = "redirect"` is
/// explicitly enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WrongNodePolicy {
    /// Return the owning node's gateway endpoint and a home-signed [`SessionTicket`] so the launcher relay
    /// on `127.0.0.1:61100` re-points to the owner (DD18, DN6).
    Redirect,
    /// Refuse the wrong-node join with a message naming the owning node and region (DN1 kill criterion).
    #[default]
    Refuse,
}

/// Widening schedule for matchmaking tickets across regions (DD19, DN8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RingSchedule {
    /// Cumulative delay in milliseconds before a ticket widens to ring index `i` (`0..delays_ms.len()`).
    pub delays_ms: Vec<u64>,
}

impl Default for RingSchedule {
    fn default() -> Self {
        Self {
            delays_ms: DEFAULT_RING_DELAYS_MS.to_vec(),
        }
    }
}

impl RingSchedule {
    /// Cumulative delay in ms before ring index `step` (`0` is always `0`).
    pub fn delay_for_step(&self, step: usize) -> u64 {
        if step == 0 {
            return 0;
        }
        self.delays_ms
            .get(step)
            .copied()
            .or_else(|| self.delays_ms.last().copied())
            .unwrap_or(0)
    }

    /// Maximum wait time in ms before a ticket has widened to every region in `region_count`.
    pub fn max_wait_ms(&self, region_count: usize) -> u64 {
        if region_count <= 1 {
            0
        } else {
            self.delay_for_step(region_count - 1)
        }
    }
}

/// Home-owned record of a player's matchmaking ticket (DN8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeTicket {
    pub published: PublishedTicket,
    pub state: TicketState,
}

/// Answer to an atomic compare-and-set ticket claim at the player's home node (DN8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ClaimOutcome {
    Granted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_ticket: Option<SessionTicket>,
    },
    AlreadyClaimed {
        match_id: GlobalId,
        hosting_node: NodeId,
    },
    Cancelled,
    UnknownTicket,
}

/// Wire requests sent over [`Service::Ring`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RingRequest {
    pub version: u32,
    pub call: RingCall,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum RingCall {
    /// Look up an instance owned by the target node (DN6).
    LookupInstance { instance: GlobalId },
    /// Publish or widen a home-owned matchmaking ticket to a regional node (DN8).
    PublishTicket { ticket: PublishedTicket },
    /// Withdraw a ticket from a regional pool once claimed or cancelled (DN8).
    WithdrawTicket { ticket: GlobalId },
    /// Compare-and-set claim of a ticket at its home node (DN8).
    ClaimTicket {
        ticket: GlobalId,
        claiming_node: NodeId,
        match_id: GlobalId,
    },
    /// Release a previously granted claim if another player in the candidate match could not be claimed (DN8).
    ReleaseTicket {
        ticket: GlobalId,
        claiming_node: NodeId,
        match_id: GlobalId,
    },
    /// Assign a player to a regional Ramsgate shard on the target region's node (DN9).
    AssignHub {
        account: GlobalId,
        client_ip: String,
        game: GameVersion,
    },
}

/// Wire replies returned over [`Service::Ring`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RingReply {
    pub version: u32,
    pub outcome: RingOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum RingOutcome {
    Ack,
    Instance(Option<InstanceRecord>),
    Claim(ClaimOutcome),
    HubAssigned(InstanceRecord),
    Refused(String),
}

/// Error returned when [`DirectorPlacement`] cannot start a hunt or hub server (DN8, DN9).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DirectorPlacementError {
    /// No Director placement seam is attached for `0`.
    #[error("no director placement configured for game {0}")]
    Unconfigured(GameVersion),
    /// The Director refused or failed to place the instance.
    #[error("{0}")]
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RingError {
    #[error("ring message exceeds {MAX_RING_MESSAGE} bytes")]
    TooLarge,
    #[error("ring message holds something shaped like an account key")]
    AccountKey,
    #[error("invalid ring message: {0}")]
    Invalid(String),
    #[error("unknown instance {0}")]
    UnknownInstance(GlobalId),
    #[error("unknown peer node {0}")]
    UnknownPeer(NodeId),
    #[error("unknown region {0}")]
    UnknownRegion(Region),
    #[error("unauthorized region report")]
    Unauthorized,
    #[error("link error: {0}")]
    Link(#[from] LinkError),
    #[error("home error: {0}")]
    Home(#[from] HomeError),
    #[error("director placement failed: {0}")]
    Director(#[from] DirectorPlacementError),
}

fn check_clean_str(s: &str) -> Result<(), RingError> {
    if s.len() > MAX_RING_TEXT {
        return Err(RingError::Invalid(format!("text over {MAX_RING_TEXT} bytes")));
    }
    if looks_like_account_key(s) {
        return Err(RingError::AccountKey);
    }
    Ok(())
}

fn validate_instance(rec: &InstanceRecord) -> Result<(), RingError> {
    check_clean_str(&rec.queue)?;
    check_clean_str(&rec.server_host)?;
    if rec.players.len() > usize::from(rec.capacity.max(1)) * 2 {
        return Err(RingError::Invalid("too many players in instance record".into()));
    }
    Ok(())
}

fn validate_published_ticket(t: &PublishedTicket) -> Result<(), RingError> {
    if t.ticket.kind() != IdKind::Ticket {
        return Err(RingError::Invalid("ticket id must have kind `ticket`".into()));
    }
    if t.account.kind() != IdKind::Account {
        return Err(RingError::Invalid("account id must have kind `account`".into()));
    }
    if t.ticket.owner() != &t.home_node {
        return Err(RingError::Invalid("ticket owner must match home_node".into()));
    }
    if t.party_size == 0 || t.party_size > 4 {
        return Err(RingError::Invalid("party_size must be 1..=4".into()));
    }
    check_clean_str(&t.queue)?;
    check_clean_str(&t.client_ip)?;
    if t.ordered_regions.len() > 32 {
        return Err(RingError::Invalid("too many ordered regions".into()));
    }
    Ok(())
}

pub fn encode_request(call: RingCall) -> Result<Vec<u8>, RingError> {
    let req = RingRequest {
        version: RING_VERSION,
        call,
    };
    validate_request(&req)?;
    let bytes = serde_json::to_vec(&req).map_err(|e| RingError::Invalid(e.to_string()))?;
    if bytes.len() > MAX_RING_MESSAGE {
        return Err(RingError::TooLarge);
    }
    Ok(bytes)
}

pub fn decode_request(bytes: &[u8]) -> Result<RingRequest, RingError> {
    if bytes.len() > MAX_RING_MESSAGE {
        return Err(RingError::TooLarge);
    }
    let req: RingRequest = serde_json::from_slice(bytes).map_err(|e| RingError::Invalid(e.to_string()))?;
    validate_request(&req)?;
    Ok(req)
}

fn validate_request(req: &RingRequest) -> Result<(), RingError> {
    if req.version != RING_VERSION {
        return Err(RingError::Invalid(format!(
            "unsupported ring version {} (expected {RING_VERSION})",
            req.version
        )));
    }
    match &req.call {
        RingCall::LookupInstance { .. } | RingCall::WithdrawTicket { .. } => Ok(()),
        RingCall::PublishTicket { ticket } => validate_published_ticket(ticket),
        RingCall::ClaimTicket { ticket, match_id, .. } | RingCall::ReleaseTicket { ticket, match_id, .. } => {
            if ticket.kind() != IdKind::Ticket {
                return Err(RingError::Invalid("ticket must have kind `ticket`".into()));
            }
            if match_id.kind() != IdKind::Hunt {
                return Err(RingError::Invalid("match_id must have kind `hunt`".into()));
            }
            Ok(())
        }
        RingCall::AssignHub { account, client_ip, .. } => {
            if account.kind() != IdKind::Account {
                return Err(RingError::Invalid("account must have kind `account`".into()));
            }
            check_clean_str(client_ip)
        }
    }
}

pub fn encode_reply(outcome: RingOutcome) -> Result<Vec<u8>, RingError> {
    let reply = RingReply {
        version: RING_VERSION,
        outcome,
    };
    validate_reply(&reply)?;
    let bytes = serde_json::to_vec(&reply).map_err(|e| RingError::Invalid(e.to_string()))?;
    if bytes.len() > MAX_RING_MESSAGE {
        return Err(RingError::TooLarge);
    }
    Ok(bytes)
}

pub fn decode_reply(bytes: &[u8]) -> Result<RingReply, RingError> {
    if bytes.len() > MAX_RING_MESSAGE {
        return Err(RingError::TooLarge);
    }
    let reply: RingReply = serde_json::from_slice(bytes).map_err(|e| RingError::Invalid(e.to_string()))?;
    validate_reply(&reply)?;
    Ok(reply)
}

fn validate_reply(reply: &RingReply) -> Result<(), RingError> {
    if reply.version != RING_VERSION {
        return Err(RingError::Invalid(format!(
            "unsupported ring version {} (expected {RING_VERSION})",
            reply.version
        )));
    }
    match &reply.outcome {
        RingOutcome::Ack | RingOutcome::Claim(_) => Ok(()),
        RingOutcome::Instance(Some(rec)) | RingOutcome::HubAssigned(rec) => validate_instance(rec),
        RingOutcome::Instance(None) => Ok(()),
        RingOutcome::Refused(msg) => check_clean_str(msg),
    }
}

/// Seam for starting a hunt or hub game server on this node's Director (DN8, DN9).
pub trait DirectorPlacement: Send + Sync {
    /// Starts a hunt server for `hunt_id` and returns `(server_host, udp_port)`.
    fn place_hunt<'a>(
        &'a self,
        hunt_id: &'a GlobalId,
        queue: &'a str,
        game: &'a GameVersion,
        players: &'a [GlobalId],
    ) -> BoxFuture<'a, Result<(String, u16), DirectorPlacementError>>;

    /// Starts a regional Ramsgate hub shard for `hub_id` (`shard` index) and returns `(server_host, udp_port)`.
    fn place_hub<'a>(
        &'a self,
        hub_id: &'a GlobalId,
        region: &'a Region,
        shard: u32,
        game: &'a GameVersion,
    ) -> BoxFuture<'a, Result<(String, u16), DirectorPlacementError>>;
}

/// Production [`DirectorPlacement`] used when no module-to-host Director seam is wired: refuses placement cleanly
/// with [`DirectorPlacementError::Unconfigured`] instead of returning a fake loopback port.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnconfiguredDirector;

impl DirectorPlacement for UnconfiguredDirector {
    fn place_hunt<'a>(
        &'a self,
        _hunt_id: &'a GlobalId,
        _queue: &'a str,
        game: &'a GameVersion,
        _players: &'a [GlobalId],
    ) -> BoxFuture<'a, Result<(String, u16), DirectorPlacementError>> {
        let game = game.clone();
        Box::pin(async move { Err(DirectorPlacementError::Unconfigured(game)) })
    }

    fn place_hub<'a>(
        &'a self,
        _hub_id: &'a GlobalId,
        _region: &'a Region,
        _shard: u32,
        game: &'a GameVersion,
    ) -> BoxFuture<'a, Result<(String, u16), DirectorPlacementError>> {
        let game = game.clone();
        Box::pin(async move { Err(DirectorPlacementError::Unconfigured(game)) })
    }
}

/// Routes [`DirectorPlacement`] requests to the loaded game module for `game` via [`DirectorInvocation::to_command`] (DN8, DN9).
#[derive(Default)]
pub struct DriverDirectorRouter {
    drivers: RwLock<BTreeMap<GameVersion, Arc<dyn ModuleDriver>>>,
}

impl DriverDirectorRouter {
    pub fn register(&self, driver: Arc<dyn ModuleDriver>) {
        let game = driver.manifest().game_version.clone();
        self.drivers
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(game, driver);
    }

    pub fn unregister(&self, module: &ModuleId) {
        self.drivers
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|_, d| &d.manifest().id != module);
    }

    fn driver_for(&self, game: &GameVersion) -> Result<Arc<dyn ModuleDriver>, DirectorPlacementError> {
        self.drivers
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(game)
            .cloned()
            .ok_or_else(|| DirectorPlacementError::Unconfigured(game.clone()))
    }

    async fn invoke(&self, invocation: DirectorInvocation) -> Result<(String, u16), DirectorPlacementError> {
        let driver = self.driver_for(invocation.game())?;
        let output = driver
            .command(invocation.to_command())
            .await
            .map_err(|e| DirectorPlacementError::Failed(e.to_string()))?;
        let data = output
            .data
            .ok_or_else(|| DirectorPlacementError::Failed("director returned no instance endpoint".into()))?;
        let placed: PlacedInstance =
            serde_json::from_value(data).map_err(|e| DirectorPlacementError::Failed(e.to_string()))?;
        Ok((placed.server_host, placed.server_port))
    }
}

impl DirectorPlacement for DriverDirectorRouter {
    fn place_hunt<'a>(
        &'a self,
        hunt_id: &'a GlobalId,
        queue: &'a str,
        game: &'a GameVersion,
        players: &'a [GlobalId],
    ) -> BoxFuture<'a, Result<(String, u16), DirectorPlacementError>> {
        let inv = DirectorInvocation::PlaceHunt {
            match_id: hunt_id.clone(),
            queue: queue.to_owned(),
            game: game.clone(),
            players: players.to_vec(),
        };
        Box::pin(async move { self.invoke(inv).await })
    }

    fn place_hub<'a>(
        &'a self,
        hub_id: &'a GlobalId,
        region: &'a Region,
        shard: u32,
        game: &'a GameVersion,
    ) -> BoxFuture<'a, Result<(String, u16), DirectorPlacementError>> {
        let inv = DirectorInvocation::PlaceHub {
            hub_id: hub_id.clone(),
            region: region.clone(),
            shard,
            game: game.clone(),
        };
        Box::pin(async move { self.invoke(inv).await })
    }
}

/// Seam for admitting matched or hub-assigned players' IP addresses into the hosting node's UDP allowlist (DN8, DN9).
pub trait AllowlistSink: Send + Sync {
    fn admit_player(&self, instance: &GlobalId, account: &GlobalId, client_ip: &str, udp_port: u16);
}

/// Configuration for [`RingNode`].
#[derive(Debug, Clone)]
pub struct RingConfig {
    pub identity: NodeIdentity,
    pub wrong_node_policy: WrongNodePolicy,
    pub schedule: RingSchedule,
    pub match_size: usize,
    pub max_hub_players: u16,
    pub peers: Vec<PeerGateway>,
}

impl RingConfig {
    pub fn new(identity: NodeIdentity) -> Self {
        Self {
            identity,
            wrong_node_policy: WrongNodePolicy::Refuse,
            schedule: RingSchedule::default(),
            match_size: 4,
            max_hub_players: DEFAULT_MAX_HUB_PLAYERS,
            peers: Vec::new(),
        }
    }
}

fn sha256_hex_str(s: &str) -> String {
    let digest = Sha256::digest(s.as_bytes());
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

struct State {
    /// Active instances owned by this node (`Hunt`, `HubInstance`, `Party`).
    instances: HashMap<GlobalId, InstanceRecord>,
    /// Per-account ordered region ping reports (`POST /network/v1/regions`, DN7/DN8).
    region_pings: HashMap<GlobalId, Vec<RegionPing>>,
    /// Matchmaking tickets owned by this home node (`IdKind::Ticket`).
    home_tickets: BTreeMap<GlobalId, HomeTicket>,
    /// Matchmaking tickets currently visible in this node's regional matchmaking pool.
    regional_pool: BTreeMap<GlobalId, PublishedTicket>,
    /// Map of account key (`RRK_...`) or its SHA-256 hex digest -> owning `GlobalId` registered for local key verification on `POST /network/v1/regions`.
    local_account_keys: HashMap<String, GlobalId>,
}

/// The per-node service implementing DN6 (instance registry & redirects), DN8 (matchmaking ring), and
/// DN9 (regional Ramsgate sharding).
pub struct RingNode {
    config: RingConfig,
    clock: Arc<Clock>,
    link: Arc<dyn PeerLink>,
    home: Arc<dyn HomeApi>,
    director: Arc<dyn DirectorPlacement>,
    allowlist: Arc<dyn AllowlistSink>,
    state: Mutex<State>,
}

fn is_pinned_fingerprint(fp: Option<&str>) -> bool {
    fp.is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

impl RingNode {
    pub fn new(
        config: RingConfig,
        clock: Arc<Clock>,
        link: Arc<dyn PeerLink>,
        home: Arc<dyn HomeApi>,
        director: Arc<dyn DirectorPlacement>,
        allowlist: Arc<dyn AllowlistSink>,
    ) -> Arc<Self> {
        Arc::new(Self {
            config,
            clock,
            link,
            home,
            director,
            allowlist,
            state: Mutex::new(State {
                instances: HashMap::new(),
                region_pings: HashMap::new(),
                home_tickets: BTreeMap::new(),
                regional_pool: BTreeMap::new(),
                local_account_keys: HashMap::new(),
            }),
        })
    }

    pub fn identity(&self) -> &NodeIdentity {
        &self.config.identity
    }

    pub fn wrong_node_policy(&self) -> WrongNodePolicy {
        self.config.wrong_node_policy
    }

    fn peer_by_node(&self, node: &NodeId) -> Option<PeerGateway> {
        self.config.peers.iter().find(|p| &p.node == node).cloned()
    }

    fn peer_by_region(&self, region: &Region) -> Option<NodeId> {
        if &self.config.identity.region == region {
            return Some(self.config.identity.node.clone());
        }
        self.config
            .peers
            .iter()
            .find(|p| &p.region == region)
            .map(|p| p.node.clone())
    }

    fn all_known_regions(&self) -> Vec<Region> {
        let mut out = vec![self.config.identity.region.clone()];
        for peer in &self.config.peers {
            if !out.contains(&peer.region) {
                out.push(peer.region.clone());
            }
        }
        out
    }

    // ------------------------------------------------------------------ DN6: Instance registry & redirects

    /// Registers an instance owned by this node in its local registry.
    pub fn register_instance(&self, record: InstanceRecord) -> Result<(), RingError> {
        if record.id.owner() != &self.config.identity.node {
            return Err(RingError::Invalid(format!(
                "instance {} is owned by {}, not {}",
                record.id,
                record.id.owner(),
                self.config.identity.node
            )));
        }
        validate_instance(&record)?;
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .instances
            .insert(record.id.clone(), record);
        Ok(())
    }

    /// Removes an instance when a hunt or hub shard stops.
    pub fn unregister_instance(&self, id: &GlobalId) -> Option<InstanceRecord> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .instances
            .remove(id)
    }

    /// Returns a locally owned instance if present.
    pub fn local_instance(&self, id: &GlobalId) -> Option<InstanceRecord> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .instances
            .get(id)
            .cloned()
    }

    /// Looks up any instance in the network by resolving `id.owner()` directly (DD18).
    pub async fn lookup_instance(&self, id: &GlobalId) -> Result<Option<InstanceRecord>, RingError> {
        let owner = id.owner();
        if owner == &self.config.identity.node {
            return Ok(self.local_instance(id));
        }
        let reply = self
            .call_peer(owner, RingCall::LookupInstance { instance: id.clone() })
            .await?;
        match reply.outcome {
            RingOutcome::Instance(record) => Ok(record),
            RingOutcome::Refused(msg) => Err(RingError::Invalid(msg)),
            _ => Err(RingError::Invalid("unexpected reply to LookupInstance".into())),
        }
    }

    /// Resolves a player's join for `instance` arriving at this node (DN6, DD18).
    ///
    /// - If this node owns `instance`, admits `account` and `client_ip` locally and returns
    ///   [`JoinDecision::Local`].
    /// - If another node owns `instance`:
    ///   - under [`WrongNodePolicy::Redirect`], mints a home-signed [`SessionTicket`] for `audience = owner`
    ///     and returns [`JoinDecision::Redirect`] with the owner's [`PeerGateway`];
    ///   - under [`WrongNodePolicy::Refuse`] (DN1 kill criterion), returns [`JoinDecision::Refused`].
    pub async fn resolve_join(
        &self,
        account: &GlobalId,
        client_ip: &str,
        instance: &GlobalId,
    ) -> Result<JoinDecision, RingError> {
        check_clean_str(client_ip)?;
        let owner = instance.owner();
        if owner == &self.config.identity.node {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let record = state
                .instances
                .get_mut(instance)
                .ok_or_else(|| RingError::UnknownInstance(instance.clone()))?;
            if !record.players.contains(account) {
                record.players.push(account.clone());
            }
            let snapshot = record.clone();
            drop(state);
            self.allowlist
                .admit_player(instance, account, client_ip, snapshot.server_port);
            return Ok(JoinDecision::Local(snapshot));
        }

        let peer = self
            .peer_by_node(owner)
            .ok_or_else(|| RingError::UnknownPeer(owner.clone()))?;

        match self.config.wrong_node_policy {
            WrongNodePolicy::Refuse => Ok(JoinDecision::Refused {
                instance: instance.clone(),
                owner: owner.clone(),
                region: peer.region.clone(),
                message: format!(
                    "instance {} is hosted on node {} ({}); wrong-node joins are refused",
                    instance, owner, peer.region
                ),
            }),
            WrongNodePolicy::Redirect => {
                if !is_pinned_fingerprint(peer.fingerprint.as_deref()) {
                    return Ok(JoinDecision::Refused {
                        instance: instance.clone(),
                        owner: owner.clone(),
                        region: peer.region.clone(),
                        message: format!(
                            "instance {} is hosted on node {} ({}) which has no pinned gateway certificate fingerprint",
                            instance, owner, peer.region
                        ),
                    });
                }
                let ticket = self.home.session_ticket(account, owner).await?;
                Ok(JoinDecision::Redirect {
                    instance: instance.clone(),
                    owner: owner.clone(),
                    gateway: peer,
                    ticket,
                })
            }
        }
    }

    // ------------------------------------------------------------------ DN7 / DN8: Region ping report

    /// Registers a local account's API key (`RRK_...`) or its 64-hex SHA-256 digest so
    /// `POST /network/v1/regions` can authenticate home-node requests directly.
    pub fn register_local_account_key(&self, api_key: impl Into<String>, account: GlobalId) {
        let key = api_key.into();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if key.starts_with("RRK_") {
            state.local_account_keys.insert(sha256_hex_str(&key), account.clone());
        }
        state.local_account_keys.insert(key, account);
    }

    /// Registers the 64-hex SHA-256 digest of a local account's API key (`RRK_...`) so
    /// `POST /network/v1/regions` can authenticate home-node requests without the raw key crossing `HostLink`.
    pub fn register_local_account_key_hash(&self, key_sha256: impl Into<String>, account: GlobalId) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .local_account_keys
            .insert(key_sha256.into().to_ascii_lowercase(), account);
    }

    /// Authenticates and records a player's ordered region ping report from `POST /network/v1/regions`
    /// (DN7/DN8), accepting either the account's home API key (`RRK_...`) or a valid [`SessionTicket`].
    pub async fn record_region_pings_authenticated(
        &self,
        api_key: Option<&str>,
        ticket: Option<(&SessionTicket, &GameVersion)>,
        pings: Vec<RegionPing>,
    ) -> Result<GlobalId, RingError> {
        let account = if let Some(key) = api_key {
            let key_hash = sha256_hex_str(key);
            let found = {
                let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                state
                    .local_account_keys
                    .get(key)
                    .or_else(|| state.local_account_keys.get(&key_hash))
                    .cloned()
            };
            found.ok_or(RingError::Unauthorized)?
        } else if let Some((session_ticket, game)) = ticket {
            self.home
                .verify_session_ticket(session_ticket, game)
                .await
                .map_err(|_| RingError::Unauthorized)?
        } else {
            return Err(RingError::Unauthorized);
        };
        self.record_region_pings(account.clone(), pings)?;
        Ok(account)
    }

    /// Records a player's measured region RTTs, sorting reachable regions by ascending RTT.
    pub fn record_region_pings(&self, account: GlobalId, mut pings: Vec<RegionPing>) -> Result<(), RingError> {
        if account.kind() != IdKind::Account {
            return Err(RingError::Invalid("account id must have kind `account`".into()));
        }
        if pings.len() > 32 {
            return Err(RingError::Invalid("too many regions in ping report".into()));
        }
        pings.sort_by(|a, b| match (a.rtt_ms, b.rtt_ms) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .region_pings
            .insert(account, pings);
        Ok(())
    }

    /// Returns the recorded region pings for `account` (sorted by ascending RTT), if any.
    pub fn region_pings_for(&self, account: &GlobalId) -> Option<Vec<RegionPing>> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .region_pings
            .get(account)
            .cloned()
    }

    /// Returns the ordered list of reachable regions for `account` (lowest RTT first), appending any
    /// remaining cluster regions so the widening ring eventually reaches every region (DD19).
    pub fn ordered_regions_for(&self, account: &GlobalId) -> Vec<Region> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let mut ordered: Vec<Region> = state
            .region_pings
            .get(account)
            .map(|list| {
                list.iter()
                    .filter(|p| p.rtt_ms.is_some())
                    .map(|p| p.region.clone())
                    .collect()
            })
            .unwrap_or_default();
        drop(state);

        for region in self.all_known_regions() {
            if !ordered.contains(&region) {
                ordered.push(region);
            }
        }
        ordered
    }

    // ------------------------------------------------------------------ DN8: Matchmaking ring

    /// Submits a new home-owned matchmaking ticket and publishes it immediately to ring step 0 (the
    /// player's lowest-ping region, DD19).
    pub async fn submit_ticket(
        &self,
        account: GlobalId,
        queue: impl Into<String>,
        game: GameVersion,
        party_size: u16,
        client_ip: impl Into<String>,
    ) -> Result<PublishedTicket, RingError> {
        if account.owner() != &self.config.identity.node {
            return Err(RingError::Invalid(format!(
                "ticket must be owned by the account's home node ({}, not {})",
                account.owner(),
                self.config.identity.node
            )));
        }
        let ordered_regions = self.ordered_regions_for(&account);
        let published = PublishedTicket {
            ticket: GlobalId::new(IdKind::Ticket, &self.config.identity.node),
            account,
            home_node: self.config.identity.node.clone(),
            queue: queue.into(),
            game,
            party_size,
            client_ip: client_ip.into(),
            ordered_regions,
            created_at_ms: self.clock.now_ms(),
        };
        validate_published_ticket(&published)?;

        {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.home_tickets.insert(
                published.ticket.clone(),
                HomeTicket {
                    published: published.clone(),
                    state: TicketState::Searching { published_steps: 1 },
                },
            );
        }

        if let Some(first_region) = published.ordered_regions.first() {
            self.publish_to_region(first_region, &published).await?;
        }
        Ok(published)
    }

    /// Widens all searching home-owned tickets whose ring schedule delays have elapsed by current clock time.
    pub async fn advance_ring(&self) -> Result<usize, RingError> {
        let now_ms = self.clock.now_ms();
        let mut to_publish: Vec<(Region, PublishedTicket)> = Vec::new();

        {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            for home_ticket in state.home_tickets.values_mut() {
                let TicketState::Searching { published_steps } = &mut home_ticket.state else {
                    continue;
                };
                let elapsed_ms = u64::try_from(now_ms.saturating_sub(home_ticket.published.created_at_ms)).unwrap_or(0);
                let total_regions = home_ticket.published.ordered_regions.len();
                while *published_steps < total_regions {
                    let needed_ms = self.config.schedule.delay_for_step(*published_steps);
                    if elapsed_ms < needed_ms {
                        break;
                    }
                    let region = home_ticket.published.ordered_regions[*published_steps].clone();
                    to_publish.push((region, home_ticket.published.clone()));
                    *published_steps += 1;
                }
            }
        }

        let count = to_publish.len();
        for (region, ticket) in to_publish {
            self.publish_to_region(&region, &ticket).await?;
        }
        Ok(count)
    }

    /// Spawns a background task that calls [`Self::advance_ring`] and [`Self::try_form_all_regional_matches`]
    /// on `interval` until aborted.
    pub fn spawn_ticker(self: &Arc<Self>, interval: std::time::Duration) -> tokio::task::JoinHandle<()> {
        let this = self.clone();
        tokio::spawn(async move {
            let mut every = tokio::time::interval(interval);
            every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                every.tick().await;
                if let Err(err) = this.advance_ring().await {
                    tracing::warn!(error = %err, "ring ticker advance_ring failed");
                }
                if let Err(err) = this.try_form_all_regional_matches().await {
                    tracing::warn!(error = %err, "ring ticker try_form_all_regional_matches failed");
                }
            }
        })
    }

    /// Attempts to form matches for every `(queue, game)` pair currently waiting in this node's regional pool.
    pub async fn try_form_all_regional_matches(&self) -> Result<Vec<InstanceRecord>, RingError> {
        let pairs: Vec<(String, GameVersion)> = {
            let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let mut seen = Vec::new();
            for t in state.regional_pool.values() {
                let pair = (t.queue.clone(), t.game.clone());
                if !seen.contains(&pair) {
                    seen.push(pair);
                }
            }
            seen
        };
        let mut out = Vec::new();
        for (queue, game) in pairs {
            out.extend(self.try_form_matches(&queue, &game).await?);
        }
        Ok(out)
    }

    /// Cancels a home-owned ticket and withdraws it from every region it was published to.
    pub async fn cancel_ticket(&self, ticket_id: &GlobalId) -> Result<bool, RingError> {
        let regions: Vec<Region> = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(entry) = state.home_tickets.get_mut(ticket_id) else {
                return Ok(false);
            };
            let steps = match entry.state {
                TicketState::Searching { published_steps } => published_steps,
                TicketState::Claimed { .. } | TicketState::Cancelled => return Ok(false),
            };
            entry.state = TicketState::Cancelled;
            entry.published.ordered_regions.iter().take(steps).cloned().collect()
        };

        for region in &regions {
            let _ = self.withdraw_from_region(region, ticket_id).await;
        }
        Ok(true)
    }

    /// Returns the current state of a ticket owned by this home node.
    pub fn ticket_state(&self, ticket_id: &GlobalId) -> Option<TicketState> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .home_tickets
            .get(ticket_id)
            .map(|t| t.state.clone())
    }

    /// Returns the tickets currently visible in this node's regional matchmaking pool.
    pub fn regional_pool(&self) -> Vec<PublishedTicket> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .regional_pool
            .values()
            .cloned()
            .collect()
    }

    /// Attempts to form matches in this node's regional pool for `(queue, game)`, claiming each candidate
    /// ticket from its home node via compare-and-set (`RingCall::ClaimTicket`), placing the hunt on this
    /// node's [`DirectorPlacement`], and feeding matched players' IPs into [`AllowlistSink`] (DN8).
    pub async fn try_form_matches(&self, queue: &str, game: &GameVersion) -> Result<Vec<InstanceRecord>, RingError> {
        let candidates: Vec<PublishedTicket> = {
            let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state
                .regional_pool
                .values()
                .filter(|t| t.queue == queue && &t.game == game)
                .cloned()
                .collect()
        };

        let mut formed = Vec::new();
        let mut batch: Vec<PublishedTicket> = Vec::new();
        let mut batch_players: usize = 0;

        for candidate in candidates {
            let size = usize::from(candidate.party_size);
            if batch_players + size > self.config.match_size {
                continue;
            }
            let match_id = GlobalId::new(IdKind::Hunt, &self.config.identity.node);
            batch.push(candidate);
            batch_players += size;

            if batch_players == self.config.match_size {
                if let Some(instance) = self.claim_and_place_match(match_id, queue, game, &batch).await? {
                    formed.push(instance);
                }
                batch.clear();
                batch_players = 0;
            }
        }

        // If fewer than `match_size` players are waiting (e.g. a 2- or 3-player hunt across nodes) and the
        // oldest waiting ticket has been searching for at least `PARTIAL_MATCH_MIN_WAIT_MS`, form a match for
        // the partial batch so 2-3 players do not wait forever when no 4th player joins.
        let now_ms = self.clock.now_ms();
        let min_players = self.config.match_size.min(2);
        if formed.is_empty()
            && batch_players >= min_players
            && batch
                .iter()
                .any(|t| now_ms.saturating_sub(t.created_at_ms) >= PARTIAL_MATCH_MIN_WAIT_MS)
        {
            let match_id = GlobalId::new(IdKind::Hunt, &self.config.identity.node);
            if let Some(instance) = self.claim_and_place_match(match_id, queue, game, &batch).await? {
                formed.push(instance);
            }
        }

        Ok(formed)
    }

    async fn claim_and_place_match(
        &self,
        match_id: GlobalId,
        queue: &str,
        game: &GameVersion,
        tickets: &[PublishedTicket],
    ) -> Result<Option<InstanceRecord>, RingError> {
        let mut claimed: Vec<PublishedTicket> = Vec::with_capacity(tickets.len());

        for t in tickets {
            let outcome = self
                .claim_ticket_at_home(&t.home_node, &t.ticket, &self.config.identity.node, &match_id)
                .await?;
            match outcome {
                ClaimOutcome::Granted { session_ticket } => {
                    if let Some(st) = session_ticket {
                        let _ = self.home.admit(&st, game).await;
                    }
                    claimed.push(t.clone());
                }
                ClaimOutcome::AlreadyClaimed { .. } | ClaimOutcome::Cancelled | ClaimOutcome::UnknownTicket => {
                    // Remove the unavailable ticket from our regional pool and release any partial claims
                    self.state
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .regional_pool
                        .remove(&t.ticket);
                    for prev in &claimed {
                        let _ = self
                            .release_ticket_at_home(
                                &prev.home_node,
                                &prev.ticket,
                                &self.config.identity.node,
                                &match_id,
                            )
                            .await;
                    }
                    return Ok(None);
                }
            }
        }

        // All tickets claimed! Start on Director first; if placement fails, release claims so tickets can retry.
        let players: Vec<GlobalId> = claimed.iter().map(|t| t.account.clone()).collect();
        let (server_host, server_port) = match self.director.place_hunt(&match_id, queue, game, &players).await {
            Ok(placed) => placed,
            Err(err) => {
                for prev in &claimed {
                    let _ = self
                        .release_ticket_at_home(&prev.home_node, &prev.ticket, &self.config.identity.node, &match_id)
                        .await;
                }
                return Err(RingError::Director(err));
            }
        };

        {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            for t in &claimed {
                state.regional_pool.remove(&t.ticket);
            }
        }
        for t in &claimed {
            for region in &t.ordered_regions {
                if region != &self.config.identity.region {
                    let _ = self.withdraw_from_region(region, &t.ticket).await;
                }
            }
        }

        let record = InstanceRecord {
            id: match_id.clone(),
            region: self.config.identity.region.clone(),
            game: game.clone(),
            queue: queue.to_owned(),
            server_host,
            server_port,
            capacity: u16::try_from(self.config.match_size).unwrap_or(4),
            players,
            shard: None,
            created_at_ms: self.clock.now_ms(),
        };
        self.register_instance(record.clone())?;

        for t in &claimed {
            self.allowlist
                .admit_player(&match_id, &t.account, &t.client_ip, server_port);
        }

        Ok(Some(record))
    }

    /// Forwards post-hunt XP, loot, and objective progress to a player's home node using
    /// [`HomeApi::forward`] and [`module_api::network::operations`] (DN8, DD19).
    pub async fn forward_hunt_rewards(&self, rewards: HuntRewards) -> Result<(), RingError> {
        let actor = ActorContext::new(module_api::Actor::System).with_request(format!("hunt:{}", rewards.hunt.ulid()));
        if let Some(xp_body) = rewards.xp_grant {
            if holds_account_key(&xp_body) {
                return Err(RingError::AccountKey);
            }
            let op =
                OperationName::new(operations::PROGRESSION_GRANT).map_err(|e| RingError::Invalid(e.to_string()))?;
            self.home
                .forward(&HomeForward {
                    module: rewards.module.clone(),
                    account: rewards.account.clone(),
                    actor: actor.clone(),
                    operation: op,
                    body: xp_body,
                    key: Some(format!("xp-{}-{}", rewards.hunt.ulid(), rewards.account.ulid())),
                })
                .await?;
        }
        if let Some(loot_body) = rewards.loot_write {
            if holds_account_key(&loot_body) {
                return Err(RingError::AccountKey);
            }
            let op = OperationName::new(operations::INVENTORY_WRITE).map_err(|e| RingError::Invalid(e.to_string()))?;
            self.home
                .forward(&HomeForward {
                    module: rewards.module.clone(),
                    account: rewards.account.clone(),
                    actor: actor.clone(),
                    operation: op,
                    body: loot_body,
                    key: Some(format!("loot-{}-{}", rewards.hunt.ulid(), rewards.account.ulid())),
                })
                .await?;
        }
        if let Some(obj_body) = rewards.objective_write {
            if holds_account_key(&obj_body) {
                return Err(RingError::AccountKey);
            }
            let op = OperationName::new(operations::BOUNTY_WRITE).map_err(|e| RingError::Invalid(e.to_string()))?;
            let key = Some(format!("bounty-{}-{}", rewards.hunt.ulid(), rewards.account.ulid()));
            self.home
                .forward(&HomeForward {
                    module: rewards.module,
                    account: rewards.account,
                    actor,
                    operation: op,
                    body: obj_body,
                    key,
                })
                .await?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------ DN9: Regional Ramsgate

    /// Assigns `account` to the shared Ramsgate hub in their lowest-ping region (DD20, DN9), sharding when
    /// an instance reaches [`RingConfig::max_hub_players`].
    pub async fn assign_ramsgate(
        &self,
        account: GlobalId,
        client_ip: impl Into<String>,
        game: GameVersion,
    ) -> Result<(InstanceRecord, Option<SessionTicket>), RingError> {
        let client_ip = client_ip.into();
        check_clean_str(&client_ip)?;
        let target_region = self
            .ordered_regions_for(&account)
            .into_iter()
            .next()
            .unwrap_or_else(|| self.config.identity.region.clone());

        let target_node = self
            .peer_by_region(&target_region)
            .ok_or_else(|| RingError::UnknownRegion(target_region.clone()))?;

        let instance = if target_node == self.config.identity.node {
            self.assign_local_hub(&account, &client_ip, &game).await?
        } else {
            let reply = self
                .call_peer(
                    &target_node,
                    RingCall::AssignHub {
                        account: account.clone(),
                        client_ip,
                        game,
                    },
                )
                .await?;
            match reply.outcome {
                RingOutcome::HubAssigned(rec) => rec,
                RingOutcome::Refused(msg) => return Err(RingError::Invalid(msg)),
                _ => return Err(RingError::Invalid("unexpected reply to AssignHub".into())),
            }
        };

        let ticket = if target_node == *account.owner() {
            None
        } else {
            Some(self.home.session_ticket(&account, &target_node).await?)
        };

        Ok((instance, ticket))
    }

    async fn assign_local_hub(
        &self,
        account: &GlobalId,
        client_ip: &str,
        game: &GameVersion,
    ) -> Result<InstanceRecord, RingError> {
        let existing_or_shard = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);

            // If the player is already in an active hub shard in this region for this game, keep them there.
            if let Some(existing) = state
                .instances
                .values()
                .find(|inst| {
                    inst.id.kind() == IdKind::HubInstance && &inst.game == game && inst.players.contains(account)
                })
                .cloned()
            {
                Ok(existing)
            } else {
                // Find the lowest-numbered shard with available capacity.
                let mut candidate_ids: Vec<(u32, GlobalId)> = state
                    .instances
                    .values()
                    .filter(|inst| {
                        inst.id.kind() == IdKind::HubInstance
                            && &inst.game == game
                            && inst.players.len() < usize::from(inst.capacity)
                    })
                    .map(|inst| (inst.shard.unwrap_or(0), inst.id.clone()))
                    .collect();
                candidate_ids.sort_by_key(|(shard, _)| *shard);

                if let Some((_, hub_id)) = candidate_ids.first()
                    && let Some(inst) = state.instances.get_mut(hub_id)
                {
                    inst.players.push(account.clone());
                    Ok(inst.clone())
                } else {
                    // All existing shards are full (or none exist yet): start a new shard on this region's Director.
                    let next_shard = state
                        .instances
                        .values()
                        .filter(|inst| inst.id.kind() == IdKind::HubInstance && &inst.game == game)
                        .filter_map(|inst| inst.shard)
                        .max()
                        .map_or(1, |s| s.saturating_add(1));
                    Err(next_shard)
                }
            }
        };

        let next_shard = match existing_or_shard {
            Ok(record) => {
                self.allowlist
                    .admit_player(&record.id, account, client_ip, record.server_port);
                return Ok(record);
            }
            Err(shard) => shard,
        };

        let hub_id = GlobalId::new(IdKind::HubInstance, &self.config.identity.node);
        let (server_host, server_port) = self
            .director
            .place_hub(&hub_id, &self.config.identity.region, next_shard, game)
            .await
            .map_err(RingError::Director)?;

        let record = InstanceRecord {
            id: hub_id.clone(),
            region: self.config.identity.region.clone(),
            game: game.clone(),
            queue: "ramsgate".into(),
            server_host,
            server_port,
            capacity: self.config.max_hub_players,
            players: vec![account.clone()],
            shard: Some(next_shard),
            created_at_ms: self.clock.now_ms(),
        };
        self.register_instance(record.clone())?;
        self.allowlist
            .admit_player(&hub_id, account, client_ip, record.server_port);
        Ok(record)
    }

    // ------------------------------------------------------------------ Internal peer helpers

    async fn publish_to_region(&self, region: &Region, ticket: &PublishedTicket) -> Result<(), RingError> {
        let Some(node) = self.peer_by_region(region) else {
            return Ok(());
        };
        if node == self.config.identity.node {
            self.state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .regional_pool
                .insert(ticket.ticket.clone(), ticket.clone());
            return Ok(());
        };
        let _ = self
            .call_peer(&node, RingCall::PublishTicket { ticket: ticket.clone() })
            .await?;
        Ok(())
    }

    async fn withdraw_from_region(&self, region: &Region, ticket: &GlobalId) -> Result<(), RingError> {
        let Some(node) = self.peer_by_region(region) else {
            return Ok(());
        };
        if node == self.config.identity.node {
            self.state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .regional_pool
                .remove(ticket);
            return Ok(());
        }
        let _ = self
            .call_peer(&node, RingCall::WithdrawTicket { ticket: ticket.clone() })
            .await?;
        Ok(())
    }

    async fn claim_ticket_at_home(
        &self,
        home_node: &NodeId,
        ticket: &GlobalId,
        claiming_node: &NodeId,
        match_id: &GlobalId,
    ) -> Result<ClaimOutcome, RingError> {
        if home_node == &self.config.identity.node {
            return Ok(self.local_claim_ticket(ticket, claiming_node, match_id).await);
        }
        let reply = self
            .call_peer(
                home_node,
                RingCall::ClaimTicket {
                    ticket: ticket.clone(),
                    claiming_node: claiming_node.clone(),
                    match_id: match_id.clone(),
                },
            )
            .await?;
        match reply.outcome {
            RingOutcome::Claim(outcome) => Ok(outcome),
            RingOutcome::Refused(msg) => Err(RingError::Invalid(msg)),
            _ => Err(RingError::Invalid("unexpected reply to ClaimTicket".into())),
        }
    }

    async fn release_ticket_at_home(
        &self,
        home_node: &NodeId,
        ticket: &GlobalId,
        claiming_node: &NodeId,
        match_id: &GlobalId,
    ) -> Result<(), RingError> {
        if home_node == &self.config.identity.node {
            self.local_release_ticket(ticket, claiming_node, match_id);
            return Ok(());
        }
        let _ = self
            .call_peer(
                home_node,
                RingCall::ReleaseTicket {
                    ticket: ticket.clone(),
                    claiming_node: claiming_node.clone(),
                    match_id: match_id.clone(),
                },
            )
            .await?;
        Ok(())
    }

    async fn local_claim_ticket(&self, ticket: &GlobalId, claiming_node: &NodeId, match_id: &GlobalId) -> ClaimOutcome {
        let now_ms = self.clock.now_ms();
        let (outcome_kind, account_opt) = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(home_ticket) = state.home_tickets.get_mut(ticket) else {
                return ClaimOutcome::UnknownTicket;
            };
            let account = home_ticket.published.account.clone();
            match &home_ticket.state {
                TicketState::Searching { .. } => {
                    home_ticket.state = TicketState::Claimed {
                        match_id: match_id.clone(),
                        hosting_node: claiming_node.clone(),
                        claimed_at_ms: now_ms,
                    };
                    state.regional_pool.remove(ticket);
                    (None, Some(account))
                }
                TicketState::Claimed {
                    match_id: existing_match,
                    hosting_node: existing_host,
                    ..
                } => {
                    if existing_match == match_id && existing_host == claiming_node {
                        (None, Some(account))
                    } else {
                        (
                            Some(ClaimOutcome::AlreadyClaimed {
                                match_id: existing_match.clone(),
                                hosting_node: existing_host.clone(),
                            }),
                            None,
                        )
                    }
                }
                TicketState::Cancelled => (Some(ClaimOutcome::Cancelled), None),
            }
        };
        if let Some(non_granted) = outcome_kind {
            return non_granted;
        }
        let session_ticket = if let Some(account) = account_opt
            && claiming_node != &self.config.identity.node
        {
            self.home.session_ticket(&account, claiming_node).await.ok()
        } else {
            None
        };
        ClaimOutcome::Granted { session_ticket }
    }

    fn local_release_ticket(&self, ticket: &GlobalId, claiming_node: &NodeId, match_id: &GlobalId) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(home_ticket) = state.home_tickets.get_mut(ticket) else {
            return;
        };
        if let TicketState::Claimed {
            match_id: existing_match,
            hosting_node: existing_host,
            ..
        } = &home_ticket.state
            && existing_match == match_id
            && existing_host == claiming_node
        {
            home_ticket.state = TicketState::Searching { published_steps: 1 };
        }
    }

    async fn call_peer(&self, peer: &NodeId, call: RingCall) -> Result<RingReply, RingError> {
        let bytes = encode_request(call)?;
        let raw = self.link.call(peer, Service::Ring, bytes).await?;
        decode_reply(&raw)
    }
}

impl NodeService for RingNode {
    fn serve<'a>(
        &'a self,
        from: &'a NodeId,
        service: Service,
        body: Vec<u8>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LinkError>> {
        Box::pin(async move {
            if service != Service::Ring {
                return Err(LinkError::NoService {
                    node: self.config.identity.node.clone(),
                    service,
                });
            }
            let req = match decode_request(&body) {
                Ok(req) => req,
                Err(RingError::TooLarge) => return Err(LinkError::TooLarge),
                Err(err) => {
                    return encode_reply(RingOutcome::Refused(err.to_string())).map_err(|e| LinkError::Protocol {
                        node: from.clone(),
                        reason: e.to_string(),
                    });
                }
            };

            let outcome = match req.call {
                RingCall::LookupInstance { instance } => RingOutcome::Instance(self.local_instance(&instance)),
                RingCall::PublishTicket { ticket } => {
                    if &ticket.home_node != from {
                        RingOutcome::Refused(format!(
                            "caller {from} cannot publish a ticket owned by {}",
                            ticket.home_node
                        ))
                    } else {
                        self.state
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .regional_pool
                            .insert(ticket.ticket.clone(), ticket);
                        RingOutcome::Ack
                    }
                }
                RingCall::WithdrawTicket { ticket } => {
                    self.state
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .regional_pool
                        .remove(&ticket);
                    RingOutcome::Ack
                }
                RingCall::ClaimTicket {
                    ticket,
                    claiming_node,
                    match_id,
                } => {
                    if &claiming_node != from {
                        RingOutcome::Refused(format!(
                            "caller {from} cannot claim tickets on behalf of {claiming_node}"
                        ))
                    } else {
                        RingOutcome::Claim(self.local_claim_ticket(&ticket, &claiming_node, &match_id).await)
                    }
                }
                RingCall::ReleaseTicket {
                    ticket,
                    claiming_node,
                    match_id,
                } => {
                    if &claiming_node != from {
                        RingOutcome::Refused(format!(
                            "caller {from} cannot release tickets on behalf of {claiming_node}"
                        ))
                    } else {
                        self.local_release_ticket(&ticket, &claiming_node, &match_id);
                        RingOutcome::Ack
                    }
                }
                RingCall::AssignHub {
                    account,
                    client_ip,
                    game,
                } => match self.assign_local_hub(&account, &client_ip, &game).await {
                    Ok(rec) => RingOutcome::HubAssigned(rec),
                    Err(err) => RingOutcome::Refused(err.to_string()),
                },
            };

            encode_reply(outcome).map_err(|e| LinkError::Protocol {
                node: self.config.identity.node.clone(),
                reason: e.to_string(),
            })
        })
    }
}
