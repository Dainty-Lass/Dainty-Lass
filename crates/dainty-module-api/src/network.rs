//! The network's seams (DN2, DD16-DD18): who this node is, ids that are the same on every node, and the
//! home API another node calls for an account it doesn't own. Stubs: a server outside a network is a node
//! with no peers ([`NodeIdentity::standalone`], [`StandaloneHome`]), and DN3-DN5 fill the rest.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::actor::ActorContext;
use crate::ids::{ModuleId, check_name, name_type};
use crate::module::{BoxFuture, CommandRequest, ErrorKind, ModuleError};
use crate::version::GameVersion;

name_type!(
    /// A node's name in the cluster, as its `settings.toml` gives it (`[network]`, DF3).
    NodeId,
    "a node id"
);

name_type!(
    /// The region a node serves, such as `oregon` or `germany`.
    Region,
    "a region"
);

/// Who this server is in the network.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeIdentity {
    /// This node's name.
    pub node: NodeId,
    /// The region it serves.
    pub region: Region,
}

impl NodeIdentity {
    /// A server that isn't in a network: one node named `local` in region `local`, which works exactly as
    /// a node with no peers (section 6).
    pub fn standalone() -> Self {
        Self {
            node: NodeId(String::from("local")),
            region: Region(String::from("local")),
        }
    }
}

/// What a [`GlobalId`] names (DD18).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdKind {
    /// A player's account.
    Account,
    /// A hunt.
    Hunt,
    /// A hub instance.
    HubInstance,
    /// A party.
    Party,
    /// A matchmaking ticket.
    Ticket,
}

impl IdKind {
    /// Every kind, in declaration order.
    pub const ALL: [IdKind; 5] = [Self::Account, Self::Hunt, Self::HubInstance, Self::Party, Self::Ticket];

    /// The kind's tag in an id's text, such as `hub_instance`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Account => "account",
            Self::Hunt => "hunt",
            Self::HubInstance => "hub_instance",
            Self::Party => "party",
            Self::Ticket => "ticket",
        }
    }
}

/// A network-wide id: a ULID tagged with its kind and the node that owns it (DD18), written
/// `hunt:01J...@oregon-1`. Every node resolves one to its owner without asking anyone.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct GlobalId {
    kind: IdKind,
    ulid: Ulid,
    node: NodeId,
}

/// Text that is not a [`GlobalId`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("a global id is <kind>:<ULID>@<node>, such as hunt:01J9Z3N5C8Q0V2X4Y6W8T0R2P4@oregon-1")]
pub struct GlobalIdError;

impl GlobalId {
    /// A new id owned by `node`.
    pub fn new(kind: IdKind, node: &NodeId) -> Self {
        Self {
            kind,
            ulid: Ulid::generate(),
            node: node.clone(),
        }
    }

    /// The id with these parts, as read back from storage.
    pub fn from_parts(kind: IdKind, ulid: Ulid, node: NodeId) -> Self {
        Self { kind, ulid, node }
    }

    /// What it names.
    pub fn kind(&self) -> IdKind {
        self.kind
    }

    /// Its ULID.
    pub fn ulid(&self) -> Ulid {
        self.ulid
    }

    /// The node that owns what this names.
    pub fn owner(&self) -> &NodeId {
        &self.node
    }
}

impl FromStr for GlobalId {
    type Err = GlobalIdError;

    fn from_str(text: &str) -> Result<Self, GlobalIdError> {
        let (kind, rest) = text.split_once(':').ok_or(GlobalIdError)?;
        let (ulid, node) = rest.split_once('@').ok_or(GlobalIdError)?;
        let kind = IdKind::ALL
            .into_iter()
            .find(|k| k.as_str() == kind)
            .ok_or(GlobalIdError)?;
        let parsed = Ulid::from_string(ulid).map_err(|_| GlobalIdError)?;
        // Only the canonical spelling, so an id has one text and compares as text everywhere
        if parsed.to_string() != ulid {
            return Err(GlobalIdError);
        }
        let node = NodeId::new(node).map_err(|_| GlobalIdError)?;
        Ok(Self {
            kind,
            ulid: parsed,
            node,
        })
    }
}

impl TryFrom<String> for GlobalId {
    type Error = GlobalIdError;

    fn try_from(text: String) -> Result<Self, GlobalIdError> {
        text.parse()
    }
}

impl From<GlobalId> for String {
    fn from(id: GlobalId) -> String {
        id.to_string()
    }
}

impl fmt::Display for GlobalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}@{}", self.kind.as_str(), self.ulid, self.node)
    }
}

/// A short-lived ticket an account's home signs so another node can act for the player (DD17); the
/// account key itself never leaves its home. `token` is the home's signed claims (DN5,
/// `network::home::ticket`): `account` and `audience` repeat what it says, and a verifier checks they agree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionTicket {
    /// The account the ticket acts for.
    pub account: GlobalId,
    /// The node it was made for.
    pub audience: NodeId,
    /// The home's signed token (`dlt1.<claims>.<signature>`).
    pub token: String,
}

/// Public gateway endpoint of a peer node in the cluster, used by the launcher to ping regions (DN7) and
/// re-point its local relay (DN6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerGateway {
    /// The peer's node name.
    pub node: NodeId,
    /// The region it serves.
    pub region: Region,
    /// Its gateway's public host.
    pub host: String,
    /// Its gateway's port.
    pub port: u16,
    /// SHA-256 of its gateway's certificate, lowercase hex.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
}

/// One region entry in a player's measured latency report (`POST /network/v1/regions`, DN7/DN8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionPing {
    /// The region.
    pub region: Region,
    /// Round-trip time in milliseconds, or `None` if the region did not answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtt_ms: Option<u32>,
}

/// Metadata for an active hunt, hub shard, or party instance registered in the cluster (DN6, DN8, DN9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceRecord {
    /// The instance's id.
    pub id: GlobalId,
    /// The region it runs in.
    pub region: Region,
    /// The game version it runs.
    pub game: GameVersion,
    /// The matchmaking queue it came from.
    pub queue: String,
    /// The game server's host.
    pub server_host: String,
    /// The game server's port.
    pub server_port: u16,
    /// Most players it takes.
    pub capacity: u16,
    /// The players in it.
    pub players: Vec<GlobalId>,
    /// The hub shard's number, for a hub.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shard: Option<u32>,
    /// When it was registered, in Unix milliseconds.
    pub created_at_ms: i64,
}

/// Outcome of resolving a join for an instance on this node (DN6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum JoinDecision {
    /// This node owns the instance and admitted the player.
    Local(InstanceRecord),
    /// Another node owns the instance; the launcher relay should re-point to `gateway` with `ticket`.
    Redirect {
        /// The instance.
        instance: GlobalId,
        /// The node that owns it.
        owner: NodeId,
        /// That node's gateway.
        gateway: PeerGateway,
        /// A ticket for that node.
        ticket: SessionTicket,
    },
    /// Another node owns the instance and wrong-node joins are refused (`WrongNodePolicy::Refuse`, DN1).
    Refused {
        /// The instance.
        instance: GlobalId,
        /// The node that owns it.
        owner: NodeId,
        /// That node's region.
        region: Region,
        /// Words for the player.
        message: String,
    },
}

/// A matchmaking ticket published into the ring (DN8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishedTicket {
    /// The ticket's id.
    pub ticket: GlobalId,
    /// The account that queued.
    pub account: GlobalId,
    /// The account's home node.
    pub home_node: NodeId,
    /// The matchmaking queue.
    pub queue: String,
    /// The game version.
    pub game: GameVersion,
    /// Players in the party.
    pub party_size: u16,
    /// The player's IP address, for placing them.
    pub client_ip: String,
    /// Regions to search, nearest first.
    pub ordered_regions: Vec<Region>,
    /// When it was published, in Unix milliseconds.
    pub created_at_ms: i64,
}

/// State of a matchmaking ticket at its owning home node (DN8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum TicketState {
    /// Still looking for a match.
    Searching {
        /// How many of its ordered regions it has been published to.
        published_steps: usize,
    },
    /// A node took it into a match.
    Claimed {
        /// The match's id.
        match_id: GlobalId,
        /// The node hosting the match.
        hosting_node: NodeId,
        /// When it was claimed, in Unix milliseconds.
        claimed_at_ms: i64,
    },
    /// Cancelled before a match.
    Cancelled,
}

/// Post-hunt rewards and objective progress to forward from the hosting node to a player's home (DN8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HuntRewards {
    /// The hunt.
    pub hunt: GlobalId,
    /// The module whose backend applies the rewards.
    pub module: ModuleId,
    /// The player.
    pub account: GlobalId,
    /// XP to grant, as JSON the module defines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xp_grant: Option<serde_json::Value>,
    /// Loot to write, as JSON the module defines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loot_write: Option<serde_json::Value>,
    /// Objective progress to write, as JSON the module defines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective_write: Option<serde_json::Value>,
}

/// The command name a module's `GameModule::command` answers for [`DirectorInvocation`]s (DN8, DN9).
pub const DIRECTOR_COMMAND: &str = "__director";

/// What a node's network ring asks its local game module to place on its Director (DN8, DN9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum DirectorInvocation {
    /// Start a hunt for a formed match.
    PlaceHunt {
        /// The match's id.
        match_id: GlobalId,
        /// The queue it came from.
        queue: String,
        /// The game version.
        game: GameVersion,
        /// The players.
        players: Vec<GlobalId>,
    },
    /// Start a hub shard.
    PlaceHub {
        /// The hub's id.
        hub_id: GlobalId,
        /// The region it serves.
        region: Region,
        /// Its shard number.
        shard: u32,
        /// The game version.
        game: GameVersion,
    },
}

/// Endpoint returned by a module's Director when placing a hunt or hub instance (DN8, DN9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlacedInstance {
    /// The game server's host.
    pub server_host: String,
    /// The game server's port.
    pub server_port: u16,
}

impl DirectorInvocation {
    /// The game version the placement is for.
    pub fn game(&self) -> &GameVersion {
        match self {
            Self::PlaceHunt { game, .. } | Self::PlaceHub { game, .. } => game,
        }
    }

    /// The command that carries this invocation to the module, as the system actor.
    pub fn to_command(&self) -> CommandRequest {
        CommandRequest {
            actor: ActorContext::new(crate::actor::Actor::System),
            name: DIRECTOR_COMMAND.to_owned(),
            args: vec![serde_json::to_string(self).unwrap_or_default()],
        }
    }

    /// The invocation a command carries; none for any other command.
    pub fn from_command(request: &CommandRequest) -> Option<Result<Self, ModuleError>> {
        if request.name != DIRECTOR_COMMAND {
            return None;
        }
        Some(match request.args.as_slice() {
            [json] => serde_json::from_str(json)
                .map_err(|error| ModuleError::new(ErrorKind::Invalid, format!("not a director invocation: {error}"))),
            _ => Err(ModuleError::new(
                ErrorKind::Invalid,
                "a director invocation is one JSON argument",
            )),
        })
    }
}

fn decode_base64url_no_pad(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits: u8 = 0;
    for b in input.bytes() {
        let val: u32 = match b {
            b'A'..=b'Z' => u32::from(b - b'A'),
            b'a'..=b'z' => u32::from(b - b'a') + 26,
            b'0'..=b'9' => u32::from(b - b'0') + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        buf = (buf << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    if bits >= 6 || (bits > 0 && (buf & ((1 << bits) - 1)) != 0) {
        return None;
    }
    Some(out)
}

impl SessionTicket {
    /// Parses a wire token (`dlt1.<claims>.<signature>`) into an unverified [`SessionTicket`] and the
    /// [`GameVersion`] its claims name, ready for [`HomeApi::admit`] or [`HomeApi::verify_session_ticket`].
    pub fn from_token(token: &str) -> Result<(Self, GameVersion), String> {
        #[derive(Deserialize)]
        struct EnvelopeClaims {
            aud: NodeId,
            sub: GlobalId,
            game: GameVersion,
        }
        if token.len() > 1024 {
            return Err("not a session ticket".into());
        }
        let mut parts = token.split('.');
        let (Some("dlt1"), Some(payload), Some(sig), None) = (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err("not a session ticket".into());
        };
        if decode_base64url_no_pad(sig).is_none_or(|b| b.len() != 64) {
            return Err("not a session ticket".into());
        }
        let payload_bytes = decode_base64url_no_pad(payload).ok_or_else(|| String::from("not a session ticket"))?;
        let claims: EnvelopeClaims =
            serde_json::from_slice(&payload_bytes).map_err(|_| String::from("not a session ticket"))?;
        if claims.sub.kind() != IdKind::Account {
            return Err("not a session ticket".into());
        }
        Ok((
            Self {
                account: claims.sub,
                audience: claims.aud,
                token: token.to_owned(),
            },
            claims.game,
        ))
    }
}

/// Why a home call failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum HomeError {
    /// This server has no peers, so only accounts it owns resolve
    #[error("this server is not in a network")]
    NotInNetwork,
    /// The account's owner is not a node this one knows.
    #[error("no node owns that account")]
    UnknownAccount,
    /// The home node could not be reached; holds the cause.
    #[error("the home node did not answer: {0}")]
    Unreachable(String),
    /// The home (or the verifying node) refused: a ticket that is expired, replayed or not for this node, a
    /// revoked session, a module the home doesn't run (DN5)
    #[error("refused: {0}")]
    Denied(String),
    /// The home's module ran the operation and it failed, as the module said (DN5)
    #[error("the home's module failed: {0}")]
    Module(ModuleError),
}

/// The most bytes an [`OperationName`] may hold.
pub const MAX_OPERATION_NAME: usize = 64;

/// What a module asks its accounts' home to do: dot-separated lowercase names such as `inventory.write`
/// (DN5). The network plugin carries it without reading it; the home's module defines what it means.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct OperationName(String);

/// Text that is not an [`OperationName`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "an operation name is dot-separated lowercase names, such as inventory.write, at most {MAX_OPERATION_NAME} bytes"
)]
pub struct OperationNameError;

impl OperationName {
    /// `text` as an operation name, when every part follows the naming rule and it fits.
    pub fn new(text: impl Into<String>) -> Result<Self, OperationNameError> {
        let text = text.into();
        let ok = text.len() <= MAX_OPERATION_NAME && text.split('.').all(|part| check_name(part, "").is_ok());
        if ok { Ok(Self(text)) } else { Err(OperationNameError) }
    }

    /// The name's text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for OperationName {
    type Err = OperationNameError;

    fn from_str(text: &str) -> Result<Self, OperationNameError> {
        Self::new(text)
    }
}

impl TryFrom<String> for OperationName {
    type Error = OperationNameError;

    fn try_from(text: String) -> Result<Self, OperationNameError> {
        Self::new(text)
    }
}

impl From<OperationName> for String {
    fn from(name: OperationName) -> String {
        name.0
    }
}

impl fmt::Display for OperationName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The operation names a game module's home backend is expected to answer for the account data other nodes
/// need (DN5): a convention, so DN6, DN8 and the modules agree; the bodies are each module's own.
pub mod operations {
    /// Load a character.
    pub const CHARACTER_LOAD: &str = "character.load";
    /// Read progression.
    pub const PROGRESSION_READ: &str = "progression.read";
    /// Grant progression.
    pub const PROGRESSION_GRANT: &str = "progression.grant";
    /// Read the inventory.
    pub const INVENTORY_READ: &str = "inventory.read";
    /// Write the inventory.
    pub const INVENTORY_WRITE: &str = "inventory.write";
    /// Read bounties.
    pub const BOUNTY_READ: &str = "bounty.read";
    /// Write bounties.
    pub const BOUNTY_WRITE: &str = "bounty.write";
    /// The generic envelope: what `IslesStore::account_write` forwards when the account is homed elsewhere
    pub const ACCOUNT_WRITE: &str = "account.write";
}

/// A module's request to run an operation at an account's home (DN5), as it crosses the module pipe. The
/// host adds who the account's home is, a ticket and the link; a module only names what to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HomeForward {
    /// The module whose backend runs it at the home
    pub module: ModuleId,
    /// The account the operation is for.
    pub account: GlobalId,
    /// Who asked at this node, for the home's change log (DD23)
    pub actor: ActorContext,
    /// What to run.
    pub operation: OperationName,
    /// Its input, as the operation defines it.
    #[serde(default)]
    pub body: serde_json::Value,
    /// Names this request, 8 to 64 characters of `[A-Za-z0-9_-]`: the home runs one key once and answers a
    /// repeat with the first answer, so a request lost on the way can be sent again. The host makes one if
    /// the module doesn't.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

/// A call to an account's home, as it crosses the module pipe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum HomeCall {
    /// Ask which node an account is homed on.
    ResolveHome {
        /// The account.
        account: GlobalId,
    },
    /// Ask the account's home for a ticket for another node.
    SessionTicket {
        /// The account.
        account: GlobalId,
        /// The node the ticket is for.
        audience: NodeId,
    },
    /// Run an operation at the account's home (DN5)
    Forward(HomeForward),
    /// Check a ticket some player arrived with, as the node it was made for (DN5)
    Admit {
        /// The ticket.
        ticket: SessionTicket,
        /// The game version the player is joining.
        game: GameVersion,
    },
    /// Verify a ticket for an ongoing HTTP session on the audience node (admits on first use and allows
    /// subsequent requests on the same held session until expiry or revocation, DN6/DN8)
    VerifySessionTicket {
        /// The ticket.
        ticket: SessionTicket,
        /// The game version of the session.
        game: GameVersion,
    },
    /// Register the SHA-256 hex digest of a local account's API key so `POST /network/v1/regions` can
    /// verify the caller without the raw account key crossing `HostLink` (DN8, RD9)
    RegisterAccountKeyHash {
        /// Lowercase hex SHA-256 of the key.
        key_sha256: String,
        /// The account.
        account: GlobalId,
    },
    /// Register a newly created account in the cluster directory (DN4)
    RegisterDirectoryAccount {
        /// The new account.
        account: GlobalId,
        /// The username it asked for.
        username: String,
    },
    /// Submit a home-owned matchmaking ticket into the regional ring (DN8)
    RingSubmitTicket {
        /// The account queueing.
        account: GlobalId,
        /// The matchmaking queue.
        queue: String,
        /// The game version.
        game: GameVersion,
        /// Players in the party.
        party_size: u16,
        /// The player's IP address.
        client_ip: String,
    },
    /// Try forming matches in this node's regional matchmaking pool for `(queue, game)` (DN8)
    RingTryFormMatches {
        /// The matchmaking queue.
        queue: String,
        /// The game version.
        game: GameVersion,
    },
    /// Query the current state of a matchmaking ticket owned by this home node (DN8)
    RingTicketState {
        /// The ticket.
        ticket: GlobalId,
    },
    /// Cancel a matchmaking ticket owned by this home node (DN8)
    RingCancelTicket {
        /// The ticket.
        ticket: GlobalId,
    },
    /// Look up an instance record by its `GlobalId` across the network (DN6, DN8)
    RingLookupInstance {
        /// The instance.
        instance: GlobalId,
    },
    /// Register an active hunt, hub shard, or party instance owned by this node (DN6)
    RingRegisterInstance {
        /// The instance's record.
        record: Box<InstanceRecord>,
    },
    /// Resolve a player's join for an instance arriving at this node (DN6)
    RingResolveJoin {
        /// The player.
        account: GlobalId,
        /// The player's IP address.
        client_ip: String,
        /// The instance being joined.
        instance: GlobalId,
    },
    /// Assign a player to the shared Ramsgate hub in their lowest-ping region (DN9)
    RingAssignRamsgate {
        /// The player.
        account: GlobalId,
        /// The player's IP address.
        client_ip: String,
        /// The game version.
        game: GameVersion,
    },
    /// Forward post-hunt XP, loot, and objective progress to a player's home node (DN8)
    RingForwardHuntRewards {
        /// The rewards.
        rewards: Box<HuntRewards>,
    },
}

/// The answer to a [`HomeCall`], the variant matching the call's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum HomeAnswer {
    /// The node the account is homed on.
    Home(NodeIdentity),
    /// The ticket made.
    Ticket(SessionTicket),
    /// What the home's module answered
    Forwarded(serde_json::Value),
    /// The account the ticket proved is logged in at its home
    Admitted(GlobalId),
    /// The call was carried out.
    Ack,
    /// The username the directory gave the account.
    DirectoryUsername(String),
    /// The published matchmaking ticket.
    PublishedTicket(Box<PublishedTicket>),
    /// Matches formed.
    FormedMatches(Vec<InstanceRecord>),
    /// The ticket's state, none when unknown.
    TicketState(Option<Box<TicketState>>),
    /// Whether a ticket was cancelled.
    Cancelled(bool),
    /// The instance, none when unknown.
    Instance(Option<Box<InstanceRecord>>),
    /// Where the player's join goes.
    JoinDecision(Box<JoinDecision>),
    /// The Ramsgate instance the player was assigned.
    RamsgateAssigned {
        /// The instance.
        instance: Box<InstanceRecord>,
        /// A ticket for the instance's node, when one was issued.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ticket: Option<SessionTicket>,
    },
}

/// The home API (DD17): what a node asks the home of an account it doesn't own. The network plugin
/// implements it over node links (DN5); modules reach it through `ModuleContext`.
pub trait HomeApi: Send + Sync {
    /// The node an account is homed on.
    fn resolve_home<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<NodeIdentity, HomeError>>;

    /// A ticket from the account's home that lets `audience` act for the player.
    fn session_ticket<'a>(
        &'a self,
        account: &'a GlobalId,
        audience: &'a NodeId,
    ) -> BoxFuture<'a, Result<SessionTicket, HomeError>>;

    /// Runs an operation at the account's home and returns what its module answered (DN5). A server outside a
    /// network has no other home to run it at.
    fn forward<'a>(&'a self, _call: &'a HomeForward) -> BoxFuture<'a, Result<serde_json::Value, HomeError>> {
        Box::pin(async { Err(HomeError::NotInNetwork) })
    }

    /// Verifies a ticket a player arrived with, as the node it was made for: signed by the account's home, for
    /// this node and `game`, fresh and not seen before (DN5). Gives the account it proves is logged in.
    fn admit<'a>(
        &'a self,
        _ticket: &'a SessionTicket,
        _game: &'a GameVersion,
    ) -> BoxFuture<'a, Result<GlobalId, HomeError>> {
        Box::pin(async { Err(HomeError::NotInNetwork) })
    }

    /// Verifies a ticket for an ongoing HTTP session on the audience node: admits the ticket on first use
    /// and accepts subsequent requests presenting the same held ticket while it remains unrevoked and unexpired.
    fn verify_session_ticket<'a>(
        &'a self,
        ticket: &'a SessionTicket,
        game: &'a GameVersion,
    ) -> BoxFuture<'a, Result<GlobalId, HomeError>> {
        self.admit(ticket, game)
    }

    /// Registers the SHA-256 hex digest of a local account's API key (`DLK_...`) for `POST /network/v1/regions`.
    fn register_account_key_hash<'a>(
        &'a self,
        _key_sha256: &'a str,
        _account: &'a GlobalId,
    ) -> BoxFuture<'a, Result<(), HomeError>> {
        Box::pin(async { Err(HomeError::NotInNetwork) })
    }

    /// Registers a newly created account in the cluster directory (DN4).
    fn register_directory_account<'a>(
        &'a self,
        _account: &'a GlobalId,
        _username: &'a str,
    ) -> BoxFuture<'a, Result<String, HomeError>> {
        Box::pin(async { Err(HomeError::NotInNetwork) })
    }

    /// Submits a home-owned matchmaking ticket into the regional ring (DN8).
    fn ring_submit_ticket<'a>(
        &'a self,
        _account: &'a GlobalId,
        _queue: &'a str,
        _game: &'a GameVersion,
        _party_size: u16,
        _client_ip: &'a str,
    ) -> BoxFuture<'a, Result<PublishedTicket, HomeError>> {
        Box::pin(async { Err(HomeError::NotInNetwork) })
    }

    /// Attempts to form matches in this node's regional pool for `(queue, game)` (DN8).
    fn ring_try_form_matches<'a>(
        &'a self,
        _queue: &'a str,
        _game: &'a GameVersion,
    ) -> BoxFuture<'a, Result<Vec<InstanceRecord>, HomeError>> {
        Box::pin(async { Err(HomeError::NotInNetwork) })
    }

    /// Queries the state of a matchmaking ticket owned by this home node (DN8).
    fn ring_ticket_state<'a>(&'a self, _ticket: &'a GlobalId) -> BoxFuture<'a, Result<Option<TicketState>, HomeError>> {
        Box::pin(async { Err(HomeError::NotInNetwork) })
    }

    /// Cancels a matchmaking ticket owned by this home node (DN8).
    fn ring_cancel_ticket<'a>(&'a self, _ticket: &'a GlobalId) -> BoxFuture<'a, Result<bool, HomeError>> {
        Box::pin(async { Err(HomeError::NotInNetwork) })
    }

    /// Looks up any instance in the network by resolving `instance.owner()` (DN6, DN8).
    fn ring_lookup_instance<'a>(
        &'a self,
        _instance: &'a GlobalId,
    ) -> BoxFuture<'a, Result<Option<InstanceRecord>, HomeError>> {
        Box::pin(async { Err(HomeError::NotInNetwork) })
    }

    /// Registers an instance owned by this node in its local registry (DN6).
    fn ring_register_instance<'a>(&'a self, _record: &'a InstanceRecord) -> BoxFuture<'a, Result<(), HomeError>> {
        Box::pin(async { Err(HomeError::NotInNetwork) })
    }

    /// Resolves a player's join for `instance` arriving at this node (DN6).
    fn ring_resolve_join<'a>(
        &'a self,
        _account: &'a GlobalId,
        _client_ip: &'a str,
        _instance: &'a GlobalId,
    ) -> BoxFuture<'a, Result<JoinDecision, HomeError>> {
        Box::pin(async { Err(HomeError::NotInNetwork) })
    }

    /// Assigns `account` to the shared Ramsgate hub in their lowest-ping region (DN9).
    fn ring_assign_ramsgate<'a>(
        &'a self,
        _account: &'a GlobalId,
        _client_ip: &'a str,
        _game: &'a GameVersion,
    ) -> BoxFuture<'a, Result<(InstanceRecord, Option<SessionTicket>), HomeError>> {
        Box::pin(async { Err(HomeError::NotInNetwork) })
    }

    /// Forwards post-hunt XP, loot, and objective progress to a player's home node (DN8).
    fn ring_forward_hunt_rewards<'a>(&'a self, _rewards: &'a HuntRewards) -> BoxFuture<'a, Result<(), HomeError>> {
        Box::pin(async { Err(HomeError::NotInNetwork) })
    }
}

/// Answers a [`HomeCall`] from a [`HomeApi`], for the host side of the pipe.
pub async fn dispatch_home(api: &dyn HomeApi, call: HomeCall) -> Result<HomeAnswer, HomeError> {
    match call {
        HomeCall::ResolveHome { account } => api.resolve_home(&account).await.map(HomeAnswer::Home),
        HomeCall::SessionTicket { account, audience } => {
            api.session_ticket(&account, &audience).await.map(HomeAnswer::Ticket)
        }
        HomeCall::Forward(forward) => api.forward(&forward).await.map(HomeAnswer::Forwarded),
        HomeCall::Admit { ticket, game } => api.admit(&ticket, &game).await.map(HomeAnswer::Admitted),
        HomeCall::VerifySessionTicket { ticket, game } => api
            .verify_session_ticket(&ticket, &game)
            .await
            .map(HomeAnswer::Admitted),
        HomeCall::RegisterAccountKeyHash { key_sha256, account } => api
            .register_account_key_hash(&key_sha256, &account)
            .await
            .map(|()| HomeAnswer::Ack),
        HomeCall::RegisterDirectoryAccount { account, username } => api
            .register_directory_account(&account, &username)
            .await
            .map(HomeAnswer::DirectoryUsername),
        HomeCall::RingSubmitTicket {
            account,
            queue,
            game,
            party_size,
            client_ip,
        } => api
            .ring_submit_ticket(&account, &queue, &game, party_size, &client_ip)
            .await
            .map(|t| HomeAnswer::PublishedTicket(Box::new(t))),
        HomeCall::RingTryFormMatches { queue, game } => api
            .ring_try_form_matches(&queue, &game)
            .await
            .map(HomeAnswer::FormedMatches),
        HomeCall::RingTicketState { ticket } => api
            .ring_ticket_state(&ticket)
            .await
            .map(|opt| HomeAnswer::TicketState(opt.map(Box::new))),
        HomeCall::RingCancelTicket { ticket } => api.ring_cancel_ticket(&ticket).await.map(HomeAnswer::Cancelled),
        HomeCall::RingLookupInstance { instance } => api
            .ring_lookup_instance(&instance)
            .await
            .map(|opt| HomeAnswer::Instance(opt.map(Box::new))),
        HomeCall::RingRegisterInstance { record } => {
            api.ring_register_instance(&record).await.map(|()| HomeAnswer::Ack)
        }
        HomeCall::RingResolveJoin {
            account,
            client_ip,
            instance,
        } => api
            .ring_resolve_join(&account, &client_ip, &instance)
            .await
            .map(|decision| HomeAnswer::JoinDecision(Box::new(decision))),
        HomeCall::RingAssignRamsgate {
            account,
            client_ip,
            game,
        } => api
            .ring_assign_ramsgate(&account, &client_ip, &game)
            .await
            .map(|(instance, ticket)| HomeAnswer::RamsgateAssigned {
                instance: Box::new(instance),
                ticket,
            }),
        HomeCall::RingForwardHuntRewards { rewards } => {
            api.ring_forward_hunt_rewards(&rewards).await.map(|()| HomeAnswer::Ack)
        }
    }
}

/// What an account's home asks its module to do for another node (DN5): the operation, with who asked and
/// where. `from` is the node the link authenticated, never a name the caller wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HomeInvocation {
    /// The node that asked, as its link authenticated it.
    pub from: NodeId,
    /// The account, homed here.
    pub account: GlobalId,
    /// Who asked at the calling node.
    pub actor: ActorContext,
    /// What to run.
    pub operation: OperationName,
    /// Its input, as the operation defines it.
    #[serde(default)]
    pub body: serde_json::Value,
}

/// The command name a module's `GameModule::command` answers for [`HomeInvocation`]s. It is not a name a
/// module can register (names have no underscores), so no console or admin command reaches it.
pub const HOME_COMMAND: &str = "__home";

impl HomeInvocation {
    /// The command that carries this invocation to a module: one argument, the invocation as JSON. The
    /// module answers in the command's `data`.
    pub fn to_command(&self) -> CommandRequest {
        CommandRequest {
            actor: self.actor.clone(),
            name: HOME_COMMAND.to_owned(),
            args: vec![serde_json::to_string(self).unwrap_or_default()],
        }
    }

    /// The invocation a command carries, for a module's `command`: `None` for any other command.
    pub fn from_command(request: &CommandRequest) -> Option<Result<Self, ModuleError>> {
        if request.name != HOME_COMMAND {
            return None;
        }
        Some(match request.args.as_slice() {
            [json] => serde_json::from_str(json)
                .map_err(|error| ModuleError::new(ErrorKind::Invalid, format!("not a home invocation: {error}"))),
            _ => Err(ModuleError::new(
                ErrorKind::Invalid,
                "a home invocation is one JSON argument",
            )),
        })
    }
}

/// What a game module does at an account's home when another node asks (DN5). The network plugin's router
/// holds one per module and calls it only for accounts homed here and for a caller that holds a valid ticket
/// for the account. A compiled-in module implements it directly; a module as its own process answers
/// [`HomeInvocation::to_command`] in `command`, and the host wraps its driver.
pub trait HomeBackend: Send + Sync {
    /// The game version the module serves: a ticket for another is refused before this is called.
    fn game(&self) -> &GameVersion;

    /// Runs `invocation` against the account's data, which the module owns. The answer is the operation's.
    fn run<'a>(&'a self, invocation: HomeInvocation) -> BoxFuture<'a, Result<serde_json::Value, ModuleError>>;
}

/// The home API of a server with no peers: every account it owns is homed here, every other is
/// unknown, and no ticket is issued.
#[derive(Debug, Clone)]
pub struct StandaloneHome {
    /// This node.
    pub identity: NodeIdentity,
}

impl HomeApi for StandaloneHome {
    fn resolve_home<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<NodeIdentity, HomeError>> {
        Box::pin(async move {
            if account.kind() != IdKind::Account {
                return Err(HomeError::UnknownAccount);
            }
            if account.owner() == &self.identity.node {
                Ok(self.identity.clone())
            } else {
                Err(HomeError::NotInNetwork)
            }
        })
    }

    fn session_ticket<'a>(
        &'a self,
        _account: &'a GlobalId,
        _audience: &'a NodeId,
    ) -> BoxFuture<'a, Result<SessionTicket, HomeError>> {
        Box::pin(async { Err(HomeError::NotInNetwork) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_ids_round_trip_and_name_their_owner() {
        let node = NodeId::new("oregon-1").unwrap();
        for kind in IdKind::ALL {
            let id = GlobalId::new(kind, &node);
            let text = id.to_string();
            assert!(text.starts_with(kind.as_str()) && text.ends_with("@oregon-1"), "{text}");
            let back: GlobalId = text.parse().unwrap();
            assert_eq!(back, id);
            assert_eq!(back.owner(), &node);
            assert_eq!(back.kind(), kind);
            assert_eq!(
                serde_json::from_str::<GlobalId>(&serde_json::to_string(&id).unwrap()).unwrap(),
                id
            );
        }
        let a = GlobalId::new(IdKind::Hunt, &node);
        let b = GlobalId::new(IdKind::Hunt, &node);
        assert_ne!(a, b);
    }

    #[test]
    fn global_ids_have_one_spelling() {
        let good = "hunt:01J9Z3N5C8Q0V2X4Y6W8T0R2P4@oregon-1";
        assert_eq!(good.parse::<GlobalId>().unwrap().to_string(), good);
        for bad in [
            "hunt:01j9z3n5c8q0v2x4y6w8t0r2p4@oregon-1",
            "hunt:01J9Z3N5C8Q0V2X4Y6W8T0R2P@oregon-1",
            "hunt:81J9Z3N5C8Q0V2X4Y6W8T0R2P4@oregon-1",
            "raid:01J9Z3N5C8Q0V2X4Y6W8T0R2P4@oregon-1",
            "hunt:01J9Z3N5C8Q0V2X4Y6W8T0R2P4@Oregon",
            "hunt:01J9Z3N5C8Q0V2X4Y6W8T0R2P4",
            "hunt01J9Z3N5C8Q0V2X4Y6W8T0R2P4@oregon-1",
            "hunt:01J9Z3N5C8Q0V2X4Y6W8T0R2P4@oregon-1@x",
            "",
        ] {
            assert!(bad.parse::<GlobalId>().is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn a_standalone_home_owns_its_own_accounts_only() {
        let home = StandaloneHome {
            identity: NodeIdentity::standalone(),
        };
        let ours = GlobalId::new(IdKind::Account, &home.identity.node);
        let theirs = GlobalId::new(IdKind::Account, &NodeId::new("germany-1").unwrap());
        let hunt = GlobalId::new(IdKind::Hunt, &home.identity.node);
        assert_eq!(home.resolve_home(&ours).await, Ok(NodeIdentity::standalone()));
        assert_eq!(home.resolve_home(&theirs).await, Err(HomeError::NotInNetwork));
        assert_eq!(home.resolve_home(&hunt).await, Err(HomeError::UnknownAccount));
        let audience = NodeId::new("germany-1").unwrap();
        assert_eq!(
            home.session_ticket(&ours, &audience).await,
            Err(HomeError::NotInNetwork)
        );
        let answer = dispatch_home(&home, HomeCall::ResolveHome { account: ours }).await;
        assert_eq!(answer, Ok(HomeAnswer::Home(NodeIdentity::standalone())));
    }

    #[test]
    fn operation_names_are_dotted_lowercase_names() {
        for good in ["inventory.write", "account.write", "bounty-board.read", "ping"] {
            assert_eq!(good.parse::<OperationName>().unwrap().as_str(), good);
        }
        for bad in [
            "",
            ".",
            "inventory.",
            ".write",
            "Inventory.write",
            "a..b",
            "a b",
            "a_b",
            "DLK_abcdef",
        ] {
            assert!(bad.parse::<OperationName>().is_err(), "{bad:?}");
        }
        assert!("a".repeat(MAX_OPERATION_NAME + 1).parse::<OperationName>().is_err());
        assert!(serde_json::from_str::<OperationName>("\"Bad.Name\"").is_err());
    }

    #[test]
    fn a_home_invocation_travels_as_a_command_no_module_can_register() {
        use crate::actor::Actor;
        let account: GlobalId = "account:01J9Z3N5C8Q0V2X4Y6W8T0R2P4@germany-1".parse().unwrap();
        let invocation = HomeInvocation {
            from: NodeId::new("oregon-1").unwrap(),
            account: account.clone(),
            actor: ActorContext::new(Actor::Player { account }),
            operation: OperationName::new(operations::INVENTORY_WRITE).unwrap(),
            body: serde_json::json!({ "items": [1, 2] }),
        };
        let command = invocation.to_command();
        assert_eq!(command.name, HOME_COMMAND);
        assert_eq!(command.args.len(), 1);
        assert_eq!(HomeInvocation::from_command(&command).unwrap().unwrap(), invocation);
        // A command with a name a module can register is not one, and this name is not registrable
        let other = CommandRequest {
            name: String::from("echo"),
            ..command.clone()
        };
        assert!(HomeInvocation::from_command(&other).is_none());
        assert!(crate::ids::check_name(HOME_COMMAND, "a command").is_err());
        let bad = CommandRequest {
            args: vec![String::from("{}")],
            ..command.clone()
        };
        assert!(HomeInvocation::from_command(&bad).unwrap().is_err());
        let none = CommandRequest {
            args: Vec::new(),
            ..command
        };
        assert!(HomeInvocation::from_command(&none).unwrap().is_err());
    }

    #[tokio::test]
    async fn a_standalone_home_has_nowhere_to_forward_or_admit() {
        let home = StandaloneHome {
            identity: NodeIdentity::standalone(),
        };
        let account = GlobalId::new(IdKind::Account, &home.identity.node);
        let forward = HomeForward {
            module: ModuleId::new("rustic-isles").unwrap(),
            account: account.clone(),
            actor: ActorContext::new(crate::actor::Actor::System),
            operation: OperationName::new(operations::ACCOUNT_WRITE).unwrap(),
            body: serde_json::Value::Null,
            key: None,
        };
        let call = HomeCall::Forward(forward);
        assert_eq!(
            serde_json::from_str::<HomeCall>(&serde_json::to_string(&call).unwrap()).unwrap(),
            call
        );
        assert_eq!(dispatch_home(&home, call).await, Err(HomeError::NotInNetwork));
        let ticket = SessionTicket {
            account,
            audience: home.identity.node.clone(),
            token: String::new(),
        };
        let admit = HomeCall::Admit {
            ticket,
            game: "1.14.7".parse().unwrap(),
        };
        assert_eq!(dispatch_home(&home, admit).await, Err(HomeError::NotInNetwork));
        // The new refusals cross the pipe as they are
        for error in [
            HomeError::Denied(String::from("expired")),
            HomeError::Module(ModuleError::new(ErrorKind::NotFound, "no such item")),
        ] {
            let json = serde_json::to_string(&error).unwrap();
            assert_eq!(serde_json::from_str::<HomeError>(&json).unwrap(), error);
        }
    }
}
