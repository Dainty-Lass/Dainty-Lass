//! The two module traits (DD3): [`GameModule`], written once and run compiled in or as its own process
//! (DD5), and [`CorePlugin`], the compiled-in plugins that serve what modules register. A module reaches
//! the host only through its [`ModuleContext`].

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::actor::ActorContext;
use crate::chat::{ChatAnswer, ChatQuery};
use crate::game::{GameAnswer, GameCall};
use crate::ids::{ModuleId, check_name};
use crate::manifest::{ModuleManifest, PortRange};
use crate::network::{
    GlobalId, HomeAnswer, HomeCall, HomeError, HomeForward, HuntRewards, IdKind, InstanceRecord, JoinDecision, NodeId,
    NodeIdentity, OperationName, PublishedTicket, SessionTicket, TicketState,
};
use crate::registrar::{AdminAction, Registrar, Registration};
use crate::version::GameVersion;

/// A boxed `Send` future, the return type of the module traits' async calls.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What kind of failure a [`ModuleError`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The module doesn't do that (a command it doesn't have, a capability it didn't declare)
    Unsupported,
    /// The request was malformed or broke a rule.
    Invalid,
    /// What the request names does not exist.
    NotFound,
    /// The module tried and failed.
    Failed,
    /// The other side of the pipe broke the protocol; the connection is dropped
    Protocol,
    /// The module or host is gone or not running
    Unavailable,
    /// The call took longer than its deadline.
    Timeout,
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unsupported => "unsupported",
            Self::Invalid => "invalid",
            Self::NotFound => "not found",
            Self::Failed => "failed",
            Self::Protocol => "protocol",
            Self::Unavailable => "unavailable",
            Self::Timeout => "timeout",
        })
    }
}

/// An error that crosses the pipe as it is, so a module's answer is the same in and out of process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(deny_unknown_fields)]
#[error("{kind}: {message}")]
pub struct ModuleError {
    /// What kind of failure.
    pub kind: ErrorKind,
    /// Words for the log or the console.
    pub message: String,
}

impl ModuleError {
    /// An error of `kind` with `message`.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// An [`ErrorKind::Unsupported`] error naming `what`.
    pub fn unsupported(what: impl fmt::Display) -> Self {
        Self::new(ErrorKind::Unsupported, format!("{what} is not supported"))
    }

    /// An [`ErrorKind::Failed`] error.
    pub fn failed(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Failed, message)
    }
}

/// The most bytes an event name may hold.
pub const MAX_EVENT_NAME: usize = 128;

/// An event's name: dot-separated lowercase names, such as `player.joined`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct EventName(String);

/// Text that is not an [`EventName`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("an event name is dot-separated lowercase names, such as player.joined, at most {MAX_EVENT_NAME} bytes")]
pub struct EventNameError;

impl EventName {
    /// `text` as an event name, when every part follows the naming rule and it fits.
    pub fn new(text: impl Into<String>) -> Result<Self, EventNameError> {
        let text = text.into();
        let ok = text.len() <= MAX_EVENT_NAME && text.split('.').all(|part| check_name(part, "").is_ok());
        if ok { Ok(Self(text)) } else { Err(EventNameError) }
    }

    /// The name's text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for EventName {
    type Err = EventNameError;

    fn from_str(text: &str) -> Result<Self, EventNameError> {
        Self::new(text)
    }
}

impl TryFrom<String> for EventName {
    type Error = EventNameError;

    fn try_from(text: String) -> Result<Self, EventNameError> {
        Self::new(text)
    }
}

impl From<EventName> for String {
    fn from(name: EventName) -> String {
        name.0
    }
}

impl fmt::Display for EventName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Something that happened, on the controller's event bus (DF3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    /// Its name.
    pub name: EventName,
    /// Its data, as the event's emitter defines it.
    #[serde(default)]
    pub payload: serde_json::Value,
}

/// How serious a [`LogRecord`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    /// Something failed.
    Error,
    /// Something looks wrong.
    Warn,
    /// Normal operation.
    Info,
    /// Detail for debugging.
    Debug,
}

/// A line a module logs through the host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogRecord {
    /// How serious it is.
    pub level: LogLevel,
    /// The line, one line of text.
    pub message: String,
}

/// A console, TUI, server GUI or admin API command for one module (DF3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandRequest {
    /// Who asked.
    pub actor: ActorContext,
    /// The command's name, as the module registered it.
    pub name: String,
    /// Its arguments, in order.
    #[serde(default)]
    pub args: Vec<String>,
}

/// What a command printed and returned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandOutput {
    /// What the console prints
    pub text: String,
    /// What the admin API and the UI read, when there is more than text
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl CommandOutput {
    /// Output with text and no data.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            data: None,
        }
    }
}

/// The admin actions every game answers (DD3), sent to the module of the player's game (DF4c).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum AdminRequest {
    /// Disconnect the player.
    Kick {
        /// The player, as the module names them.
        player: String,
        /// Words given to the player and the log.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// Ban the player.
    Ban {
        /// The player, as the module names them.
        player: String,
        /// Words given to the player and the log.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        /// RFC 3339; none is permanent
        #[serde(default, skip_serializing_if = "Option::is_none")]
        until: Option<String>,
    },
    /// Lift the player's ban.
    Unban {
        /// The player, as the module names them.
        player: String,
        /// Words for the log.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// End the player's timeout early.
    LiftTimeout {
        /// The player, as the module names them.
        player: String,
        /// Words for the log.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// Look the player up.
    Lookup {
        /// The player, as the module names them.
        player: String,
    },
}

impl AdminRequest {
    /// The request's [`AdminAction`].
    pub fn action(&self) -> AdminAction {
        match self {
            Self::Kick { .. } => AdminAction::Kick,
            Self::Ban { .. } => AdminAction::Ban,
            Self::Unban { .. } => AdminAction::Unban,
            Self::LiftTimeout { .. } => AdminAction::LiftTimeout,
            Self::Lookup { .. } => AdminAction::Lookup,
        }
    }
}

/// An admin request and who made it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminCall {
    /// Who asked.
    pub actor: ActorContext,
    /// What was asked.
    pub request: AdminRequest,
}

/// What kind of restriction a [`PlayerRestriction`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestrictionKind {
    /// Banned until someone lifts it
    Ban,
    /// Kept out of play until `until`, then let back in by the module itself
    Timeout,
    /// Disabled by the game's own tools
    Disabled,
}

/// Why a player can't play right now, as the module keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerRestriction {
    /// What kind.
    pub kind: RestrictionKind,
    /// Words given when it was set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// RFC 3339
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// RFC 3339: a timeout's end
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
}

/// A player as an admin lookup shows them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerInfo {
    /// The player's network account, when the module knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<GlobalId>,
    /// Name shown to players.
    pub name: String,
    /// Connected now.
    pub online: bool,
    /// None for a player who may play, or a game whose module doesn't say
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restriction: Option<PlayerRestriction>,
}

/// A module's answer to an [`AdminCall`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum AdminResponse {
    /// The action was carried out.
    Done,
    /// No such player.
    NotFound,
    /// The player looked up, or as the action left them.
    Player(PlayerInfo),
}

/// Where a module is in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Registered, not started
    Loaded,
    /// Starting its services.
    Starting,
    /// Serving.
    Ready,
    /// Stopping its services.
    Stopping,
    /// Stopped after running.
    Stopped,
    /// Failed to start, or stopped serving on an error.
    Failed,
}

/// A module's lifecycle state, as `status` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleStatus {
    /// Where it is.
    pub phase: Phase,
    /// Words on the state, such as a failure's reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl ModuleStatus {
    /// A status with no detail.
    pub fn new(phase: Phase) -> Self {
        Self { phase, detail: None }
    }
}

/// What a module starts with: the ports the controller gave it and its `host.toml` sections.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleConfig {
    /// The port range each named port got.
    #[serde(default)]
    pub ports: BTreeMap<String, PortRange>,
    /// Each section's keys and values.
    #[serde(default)]
    pub sections: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
}

impl ModuleConfig {
    /// The manifest's default ports and the registration's config defaults: what a host with nothing in
    /// `host.toml` for the module runs.
    pub fn defaults(manifest: &ModuleManifest, registration: &Registration) -> Self {
        let ports = manifest
            .ports
            .iter()
            .map(|port| (port.name.clone(), port.range))
            .collect();
        let sections = registration
            .config
            .iter()
            .map(|section| {
                let keys = section
                    .keys
                    .iter()
                    .map(|key| (key.name.clone(), key.default.clone()))
                    .collect();
                (section.name.clone(), keys)
            })
            .collect();
        Self { ports, sections }
    }

    /// The value of `key` in `section`, if set.
    pub fn get(&self, section: &str, key: &str) -> Option<&serde_json::Value> {
        self.sections.get(section)?.get(key)
    }
}

/// A module's calls into the host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum HostCall {
    /// Puts an event on the controller's bus.
    Emit(Event),
    /// Writes a log line.
    Log(LogRecord),
    /// A call to the network's home API.
    Home(HomeCall),
}

/// The host's answer to a [`HostCall`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum HostAnswer {
    /// The call was carried out.
    Done,
    /// The home API's answer or error.
    Home(Result<HomeAnswer, HomeError>),
}

/// The host as a module sees it. The controller implements it (DF3); out of process, the child adapter
/// carries each call over the pipe to the host's implementation, so a module can't tell the difference.
pub trait HostLink: Send + Sync {
    /// Carries `call` from `module` to the host.
    fn call<'a>(&'a self, module: &'a ModuleId, call: HostCall) -> BoxFuture<'a, Result<HostAnswer, ModuleError>>;
}

/// What a running module holds: who it is, which node it runs on, and its link to the host.
#[derive(Clone)]
pub struct ModuleContext {
    module: ModuleId,
    node: NodeIdentity,
    link: Arc<dyn HostLink>,
}

impl fmt::Debug for ModuleContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModuleContext")
            .field("module", &self.module)
            .field("node", &self.node)
            .finish()
    }
}

fn unexpected(answer: &HostAnswer) -> ModuleError {
    ModuleError::new(ErrorKind::Protocol, format!("the host answered {answer:?}"))
}

impl ModuleContext {
    /// The context for `module` on `node`, calling the host through `link`.
    pub fn new(module: ModuleId, node: NodeIdentity, link: Arc<dyn HostLink>) -> Self {
        Self { module, node, link }
    }

    /// This module's id.
    pub fn module_id(&self) -> &ModuleId {
        &self.module
    }

    /// The node the module runs on.
    pub fn node(&self) -> &NodeIdentity {
        &self.node
    }

    /// A network-wide id owned by this node (DD18).
    pub fn new_id(&self, kind: IdKind) -> GlobalId {
        GlobalId::new(kind, &self.node.node)
    }

    async fn call(&self, call: HostCall) -> Result<HostAnswer, ModuleError> {
        self.link.call(&self.module, call).await
    }

    async fn done(&self, call: HostCall) -> Result<(), ModuleError> {
        match self.call(call).await? {
            HostAnswer::Done => Ok(()),
            other => Err(unexpected(&other)),
        }
    }

    /// Puts `event` on the controller's bus.
    pub async fn emit(&self, event: Event) -> Result<(), ModuleError> {
        self.done(HostCall::Emit(event)).await
    }

    /// Logs through the host's tracing layers. Out of process, stdout is the pipe: use this or stderr,
    /// never `println!`.
    pub async fn log(&self, level: LogLevel, message: impl Into<String>) -> Result<(), ModuleError> {
        self.done(HostCall::Log(LogRecord {
            level,
            message: message.into(),
        }))
        .await
    }

    async fn home(&self, call: HomeCall) -> Result<Result<HomeAnswer, HomeError>, ModuleError> {
        match self.call(HostCall::Home(call)).await? {
            HostAnswer::Home(answer) => Ok(answer),
            other => Err(unexpected(&other)),
        }
    }

    /// The node an account is homed on (DD17).
    pub async fn resolve_home(&self, account: &GlobalId) -> Result<Result<NodeIdentity, HomeError>, ModuleError> {
        Ok(
            match self
                .home(HomeCall::ResolveHome {
                    account: account.clone(),
                })
                .await?
            {
                Ok(HomeAnswer::Home(identity)) => Ok(identity),
                Ok(other) => {
                    return Err(ModuleError::new(
                        ErrorKind::Protocol,
                        format!("home answered {other:?}"),
                    ));
                }
                Err(error) => Err(error),
            },
        )
    }

    /// A ticket from the account's home for `audience` (DD17).
    pub async fn session_ticket(
        &self,
        account: &GlobalId,
        audience: &NodeId,
    ) -> Result<Result<SessionTicket, HomeError>, ModuleError> {
        let call = HomeCall::SessionTicket {
            account: account.clone(),
            audience: audience.clone(),
        };
        Ok(match self.home(call).await? {
            Ok(HomeAnswer::Ticket(ticket)) => Ok(ticket),
            Ok(other) => {
                return Err(ModuleError::new(
                    ErrorKind::Protocol,
                    format!("home answered {other:?}"),
                ));
            }
            Err(error) => Err(error),
        })
    }

    /// Runs `operation` at the account's home, in `module`'s backend there, and gives its answer (DN5). A
    /// repeat of the same `key` is answered with the first answer instead of running again; none makes the
    /// host choose one.
    pub async fn forward_home(
        &self,
        account: &GlobalId,
        actor: &ActorContext,
        operation: &OperationName,
        body: serde_json::Value,
        key: Option<String>,
    ) -> Result<Result<serde_json::Value, HomeError>, ModuleError> {
        let call = HomeCall::Forward(HomeForward {
            module: self.module.clone(),
            account: account.clone(),
            actor: actor.clone(),
            operation: operation.clone(),
            body,
            key,
        });
        Ok(match self.home(call).await? {
            Ok(HomeAnswer::Forwarded(answer)) => Ok(answer),
            Ok(other) => {
                return Err(ModuleError::new(
                    ErrorKind::Protocol,
                    format!("home answered {other:?}"),
                ));
            }
            Err(error) => Err(error),
        })
    }

    /// Checks a ticket a player arrived with, as the node it was made for (DN5): the account it proves is
    /// logged in at its home, or why not.
    pub async fn admit_ticket(
        &self,
        ticket: &SessionTicket,
        game: &GameVersion,
    ) -> Result<Result<GlobalId, HomeError>, ModuleError> {
        let call = HomeCall::Admit {
            ticket: ticket.clone(),
            game: game.clone(),
        };
        Ok(match self.home(call).await? {
            Ok(HomeAnswer::Admitted(account)) => Ok(account),
            Ok(other) => {
                return Err(ModuleError::new(
                    ErrorKind::Protocol,
                    format!("home answered {other:?}"),
                ));
            }
            Err(error) => Err(error),
        })
    }

    /// Verifies a ticket for an ongoing HTTP session on this node, admitting on first use (DN6, DN8).
    pub async fn verify_session_ticket(
        &self,
        ticket: &SessionTicket,
        game: &GameVersion,
    ) -> Result<Result<GlobalId, HomeError>, ModuleError> {
        let call = HomeCall::VerifySessionTicket {
            ticket: ticket.clone(),
            game: game.clone(),
        };
        Ok(match self.home(call).await? {
            Ok(HomeAnswer::Admitted(account)) => Ok(account),
            Ok(other) => {
                return Err(ModuleError::new(
                    ErrorKind::Protocol,
                    format!("home answered {other:?}"),
                ));
            }
            Err(error) => Err(error),
        })
    }

    /// Registers the SHA-256 of a local account's API key, so the network can check the caller without
    /// the key (DN8).
    pub async fn register_account_key_hash(
        &self,
        key_sha256: &str,
        account: &GlobalId,
    ) -> Result<Result<(), HomeError>, ModuleError> {
        let call = HomeCall::RegisterAccountKeyHash {
            key_sha256: key_sha256.to_owned(),
            account: account.clone(),
        };
        Ok(match self.home(call).await? {
            Ok(HomeAnswer::Ack) => Ok(()),
            Ok(other) => {
                return Err(ModuleError::new(
                    ErrorKind::Protocol,
                    format!("home answered {other:?}"),
                ));
            }
            Err(error) => Err(error),
        })
    }

    /// Registers a new account in the cluster directory and returns the username it got there (DN4).
    pub async fn register_directory_account(
        &self,
        account: &GlobalId,
        username: &str,
    ) -> Result<Result<String, HomeError>, ModuleError> {
        let call = HomeCall::RegisterDirectoryAccount {
            account: account.clone(),
            username: username.to_owned(),
        };
        Ok(match self.home(call).await? {
            Ok(HomeAnswer::DirectoryUsername(name)) => Ok(name),
            Ok(other) => {
                return Err(ModuleError::new(
                    ErrorKind::Protocol,
                    format!("home answered {other:?}"),
                ));
            }
            Err(error) => Err(error),
        })
    }

    /// Submits a home-owned matchmaking ticket into the regional ring (DN8).
    pub async fn ring_submit_ticket(
        &self,
        account: &GlobalId,
        queue: &str,
        game: &GameVersion,
        party_size: u16,
        client_ip: &str,
    ) -> Result<Result<PublishedTicket, HomeError>, ModuleError> {
        let call = HomeCall::RingSubmitTicket {
            account: account.clone(),
            queue: queue.to_owned(),
            game: game.clone(),
            party_size,
            client_ip: client_ip.to_owned(),
        };
        Ok(match self.home(call).await? {
            Ok(HomeAnswer::PublishedTicket(ticket)) => Ok(*ticket),
            Ok(other) => {
                return Err(ModuleError::new(
                    ErrorKind::Protocol,
                    format!("home answered {other:?}"),
                ));
            }
            Err(error) => Err(error),
        })
    }

    /// Tries to form matches in this node's regional pool for `(queue, game)` (DN8).
    pub async fn ring_try_form_matches(
        &self,
        queue: &str,
        game: &GameVersion,
    ) -> Result<Result<Vec<InstanceRecord>, HomeError>, ModuleError> {
        let call = HomeCall::RingTryFormMatches {
            queue: queue.to_owned(),
            game: game.clone(),
        };
        Ok(match self.home(call).await? {
            Ok(HomeAnswer::FormedMatches(matches)) => Ok(matches),
            Ok(other) => {
                return Err(ModuleError::new(
                    ErrorKind::Protocol,
                    format!("home answered {other:?}"),
                ));
            }
            Err(error) => Err(error),
        })
    }

    /// The state of a matchmaking ticket this home owns, none when unknown (DN8).
    pub async fn ring_ticket_state(
        &self,
        ticket: &GlobalId,
    ) -> Result<Result<Option<TicketState>, HomeError>, ModuleError> {
        let call = HomeCall::RingTicketState { ticket: ticket.clone() };
        Ok(match self.home(call).await? {
            Ok(HomeAnswer::TicketState(state)) => Ok(state.map(|boxed| *boxed)),
            Ok(other) => {
                return Err(ModuleError::new(
                    ErrorKind::Protocol,
                    format!("home answered {other:?}"),
                ));
            }
            Err(error) => Err(error),
        })
    }

    /// Cancels a matchmaking ticket this home owns; whether one was cancelled (DN8).
    pub async fn ring_cancel_ticket(&self, ticket: &GlobalId) -> Result<Result<bool, HomeError>, ModuleError> {
        let call = HomeCall::RingCancelTicket { ticket: ticket.clone() };
        Ok(match self.home(call).await? {
            Ok(HomeAnswer::Cancelled(cancelled)) => Ok(cancelled),
            Ok(other) => {
                return Err(ModuleError::new(
                    ErrorKind::Protocol,
                    format!("home answered {other:?}"),
                ));
            }
            Err(error) => Err(error),
        })
    }

    /// An instance record by its id, from across the network (DN6, DN8).
    pub async fn ring_lookup_instance(
        &self,
        instance: &GlobalId,
    ) -> Result<Result<Option<InstanceRecord>, HomeError>, ModuleError> {
        let call = HomeCall::RingLookupInstance {
            instance: instance.clone(),
        };
        Ok(match self.home(call).await? {
            Ok(HomeAnswer::Instance(record)) => Ok(record.map(|boxed| *boxed)),
            Ok(other) => {
                return Err(ModuleError::new(
                    ErrorKind::Protocol,
                    format!("home answered {other:?}"),
                ));
            }
            Err(error) => Err(error),
        })
    }

    /// Registers a hunt, hub shard or party instance this node owns (DN6).
    pub async fn ring_register_instance(&self, record: &InstanceRecord) -> Result<Result<(), HomeError>, ModuleError> {
        let call = HomeCall::RingRegisterInstance {
            record: Box::new(record.clone()),
        };
        Ok(match self.home(call).await? {
            Ok(HomeAnswer::Ack) => Ok(()),
            Ok(other) => {
                return Err(ModuleError::new(
                    ErrorKind::Protocol,
                    format!("home answered {other:?}"),
                ));
            }
            Err(error) => Err(error),
        })
    }

    /// Decides a player's join to an instance arriving at this node (DN6).
    pub async fn ring_resolve_join(
        &self,
        account: &GlobalId,
        client_ip: &str,
        instance: &GlobalId,
    ) -> Result<Result<JoinDecision, HomeError>, ModuleError> {
        let call = HomeCall::RingResolveJoin {
            account: account.clone(),
            client_ip: client_ip.to_owned(),
            instance: instance.clone(),
        };
        Ok(match self.home(call).await? {
            Ok(HomeAnswer::JoinDecision(decision)) => Ok(*decision),
            Ok(other) => {
                return Err(ModuleError::new(
                    ErrorKind::Protocol,
                    format!("home answered {other:?}"),
                ));
            }
            Err(error) => Err(error),
        })
    }

    /// Puts a player in the shared Ramsgate hub of their lowest-ping region, with a ticket for its node
    /// when one was issued (DN9).
    pub async fn ring_assign_ramsgate(
        &self,
        account: &GlobalId,
        client_ip: &str,
        game: &GameVersion,
    ) -> Result<Result<(InstanceRecord, Option<SessionTicket>), HomeError>, ModuleError> {
        let call = HomeCall::RingAssignRamsgate {
            account: account.clone(),
            client_ip: client_ip.to_owned(),
            game: game.clone(),
        };
        Ok(match self.home(call).await? {
            Ok(HomeAnswer::RamsgateAssigned { instance, ticket }) => Ok((*instance, ticket)),
            Ok(other) => {
                return Err(ModuleError::new(
                    ErrorKind::Protocol,
                    format!("home answered {other:?}"),
                ));
            }
            Err(error) => Err(error),
        })
    }

    /// Sends post-hunt XP, loot and objective progress to the player's home node (DN8).
    pub async fn ring_forward_hunt_rewards(&self, rewards: &HuntRewards) -> Result<Result<(), HomeError>, ModuleError> {
        let call = HomeCall::RingForwardHuntRewards {
            rewards: Box::new(rewards.clone()),
        };
        Ok(match self.home(call).await? {
            Ok(HomeAnswer::Ack) => Ok(()),
            Ok(other) => {
                return Err(ModuleError::new(
                    ErrorKind::Protocol,
                    format!("home answered {other:?}"),
                ));
            }
            Err(error) => Err(error),
        })
    }
}

/// A game module, or a plugin loaded like one (DD3, DD25). Written once: the controller runs it compiled
/// in, or as an executable through `child::main` (DD5), and every call gives the same answer both ways.
pub trait GameModule: Send + Sync + 'static {
    /// The module's manifest.
    fn manifest(&self) -> ModuleManifest;

    /// Declares what the module needs from the core plugins. Runs before `start`, once per load.
    fn register(&self, registrar: &mut Registrar) -> Result<(), ModuleError>;

    /// Starts the module's services on the ports in `config`, and returns once they are ready. The
    /// services keep running on the module's own tasks until `stop`.
    fn start(&self, ctx: ModuleContext, config: ModuleConfig) -> BoxFuture<'_, Result<(), ModuleError>>;

    /// Stops the module's services; it may be started again.
    fn stop(&self, ctx: ModuleContext) -> BoxFuture<'_, Result<(), ModuleError>>;

    /// The module's current status.
    fn status(&self) -> ModuleStatus;

    /// Runs a console command the module registered; unsupported unless overridden.
    fn command(
        &self,
        _ctx: ModuleContext,
        request: CommandRequest,
    ) -> BoxFuture<'_, Result<CommandOutput, ModuleError>> {
        Box::pin(async move { Err(ModuleError::unsupported(format!("command {}", request.name))) })
    }

    /// Runs an admin action the module registered; unsupported unless overridden.
    fn admin(&self, _ctx: ModuleContext, call: AdminCall) -> BoxFuture<'_, Result<AdminResponse, ModuleError>> {
        Box::pin(async move { Err(ModuleError::unsupported(format!("{:?}", call.request.action()))) })
    }

    /// An event the module subscribed to.
    fn event(&self, _ctx: ModuleContext, _event: Event) -> BoxFuture<'_, Result<(), ModuleError>> {
        Box::pin(async { Ok(()) })
    }

    /// The chat plugin's hooks (DN10), for a module with the chat capability.
    fn chat(&self, _ctx: ModuleContext, _query: ChatQuery) -> BoxFuture<'_, Result<ChatAnswer, ModuleError>> {
        Box::pin(async { Err(ModuleError::unsupported("chat")) })
    }

    /// What the module knows about its game (MS1), for the calls it registered with `game_call`.
    fn game(&self, _ctx: ModuleContext, call: GameCall) -> BoxFuture<'_, Result<GameAnswer, ModuleError>> {
        Box::pin(async move { Err(ModuleError::unsupported(format!("game call {:?}", call.kind()))) })
    }
}

/// What a core plugin serves for the modules (DD3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreCapability {
    /// The gateway: a module's routes behind it (DF4a)
    Routes,
    /// The allowlist: a module's UDP ranges (DF4d)
    UdpPorts,
    /// Content: a module's files, client profile and manifests (DF4b)
    ContentFiles,
    /// The console, TUI, server GUI and admin API (DF3)
    Commands,
    /// The event bus: what a module emits and subscribes to (DF3).
    Events,
    /// The admin API's kick, ban and lookup (DF4c)
    AdminActions,
    /// `host.toml` (DF3)
    ConfigSections,
    /// The chat plugin (DN10)
    Chat,
    /// The network plugin (DN2-DN9)
    Network,
}

/// A plugin compiled into Dainty Lass (gateway, content, admin API, host kit, chat, network). The
/// controller hands it each module's registration as the module loads, and takes it back on unload.
pub trait CorePlugin: Send + Sync {
    /// The plugin's name, for logs.
    fn name(&self) -> &'static str;

    /// What it serves.
    fn serves(&self) -> &'static [CoreCapability];

    /// Takes the parts of a registration this plugin serves. An error refuses the module's load.
    fn attach(&self, module: &ModuleManifest, registration: &Registration) -> Result<(), ModuleError>;

    /// Drops what it took for `module` when the module unloads.
    fn detach(&self, module: &ModuleId);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_names_are_dotted_names() {
        for good in ["player.joined", "stub", "hunt.ended.v2"] {
            assert_eq!(good.parse::<EventName>().unwrap().as_str(), good);
        }
        for bad in [
            "",
            ".",
            "player.",
            ".joined",
            "Player.joined",
            "player..joined",
            "player joined",
        ] {
            assert!(bad.parse::<EventName>().is_err(), "{bad:?}");
        }
        assert!("a".repeat(MAX_EVENT_NAME + 1).parse::<EventName>().is_err());
    }

    #[test]
    fn errors_and_answers_serialise() {
        let error = ModuleError::unsupported("chat");
        assert_eq!(error.to_string(), "unsupported: chat is not supported");
        let json = serde_json::to_string(&error).unwrap();
        assert_eq!(json, r#"{"kind":"unsupported","message":"chat is not supported"}"#);
        let answer = HostAnswer::Home(Err(HomeError::NotInNetwork));
        let json = serde_json::to_string(&answer).unwrap();
        assert_eq!(serde_json::from_str::<HostAnswer>(&json).unwrap(), answer);
        let kick = AdminRequest::Kick {
            player: String::from("p"),
            reason: None,
        };
        assert_eq!(serde_json::to_string(&kick).unwrap(), r#"{"kick":{"player":"p"}}"#);
        assert_eq!(kick.action(), AdminAction::Kick);
    }
}
