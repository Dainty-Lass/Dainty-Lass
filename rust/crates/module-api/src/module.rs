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
use crate::ids::{ModuleId, check_name};
use crate::manifest::{ModuleManifest, PortRange};
use crate::network::{
    GlobalId, HomeAnswer, HomeCall, HomeError, HomeForward, IdKind, NodeId, NodeIdentity, OperationName, SessionTicket,
};
use crate::registrar::{AdminAction, Registrar, Registration};
use crate::version::GameVersion;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The module doesn't do that (a command it doesn't have, a capability it didn't declare)
    Unsupported,
    Invalid,
    NotFound,
    Failed,
    /// The other side of the pipe broke the protocol; the connection is dropped
    Protocol,
    /// The module or host is gone or not running
    Unavailable,
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
    pub kind: ErrorKind,
    pub message: String,
}

impl ModuleError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn unsupported(what: impl fmt::Display) -> Self {
        Self::new(ErrorKind::Unsupported, format!("{what} is not supported"))
    }

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

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("an event name is dot-separated lowercase names, such as player.joined, at most {MAX_EVENT_NAME} bytes")]
pub struct EventNameError;

impl EventName {
    pub fn new(text: impl Into<String>) -> Result<Self, EventNameError> {
        let text = text.into();
        let ok = text.len() <= MAX_EVENT_NAME && text.split('.').all(|part| check_name(part, "").is_ok());
        if ok { Ok(Self(text)) } else { Err(EventNameError) }
    }

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
    pub name: EventName,
    #[serde(default)]
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogRecord {
    pub level: LogLevel,
    pub message: String,
}

/// A console, TUI, server GUI or admin API command for one module (DF3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandRequest {
    pub actor: ActorContext,
    pub name: String,
    #[serde(default)]
    pub args: Vec<String>,
}

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
    Kick {
        player: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Ban {
        player: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        /// RFC 3339; none is permanent
        #[serde(default, skip_serializing_if = "Option::is_none")]
        until: Option<String>,
    },
    Lookup {
        player: String,
    },
}

impl AdminRequest {
    pub fn action(&self) -> AdminAction {
        match self {
            Self::Kick { .. } => AdminAction::Kick,
            Self::Ban { .. } => AdminAction::Ban,
            Self::Lookup { .. } => AdminAction::Lookup,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminCall {
    pub actor: ActorContext,
    pub request: AdminRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<GlobalId>,
    pub name: String,
    pub online: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum AdminResponse {
    Done,
    NotFound,
    Player(PlayerInfo),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Registered, not started
    Loaded,
    Starting,
    Ready,
    Stopping,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleStatus {
    pub phase: Phase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl ModuleStatus {
    pub fn new(phase: Phase) -> Self {
        Self { phase, detail: None }
    }
}

/// What a module starts with: the ports the controller gave it and its `host.toml` sections.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleConfig {
    #[serde(default)]
    pub ports: BTreeMap<String, PortRange>,
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

    pub fn get(&self, section: &str, key: &str) -> Option<&serde_json::Value> {
        self.sections.get(section)?.get(key)
    }
}

/// A module's calls into the host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum HostCall {
    Emit(Event),
    Log(LogRecord),
    Home(HomeCall),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum HostAnswer {
    Done,
    Home(Result<HomeAnswer, HomeError>),
}

/// The host as a module sees it. The controller implements it (DF3); out of process, the child adapter
/// carries each call over the pipe to the host's implementation, so a module can't tell the difference.
pub trait HostLink: Send + Sync {
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
    pub fn new(module: ModuleId, node: NodeIdentity, link: Arc<dyn HostLink>) -> Self {
        Self { module, node, link }
    }

    pub fn module_id(&self) -> &ModuleId {
        &self.module
    }

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
}

/// A game module, or a plugin loaded like one (DD3, DD25). Written once: the controller runs it compiled
/// in, or as an executable through `child::main` (DD5), and every call gives the same answer both ways.
pub trait GameModule: Send + Sync + 'static {
    fn manifest(&self) -> ModuleManifest;

    /// Declares what the module needs from the core plugins. Runs before `start`, once per load.
    fn register(&self, registrar: &mut Registrar) -> Result<(), ModuleError>;

    /// Starts the module's services on the ports in `config`, and returns once they are ready. The
    /// services keep running on the module's own tasks until `stop`.
    fn start(&self, ctx: ModuleContext, config: ModuleConfig) -> BoxFuture<'_, Result<(), ModuleError>>;

    fn stop(&self, ctx: ModuleContext) -> BoxFuture<'_, Result<(), ModuleError>>;

    fn status(&self) -> ModuleStatus;

    fn command(
        &self,
        _ctx: ModuleContext,
        request: CommandRequest,
    ) -> BoxFuture<'_, Result<CommandOutput, ModuleError>> {
        Box::pin(async move { Err(ModuleError::unsupported(format!("command {}", request.name))) })
    }

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

/// A plugin compiled into DaintyLass (gateway, content, admin API, host kit, chat, network). The
/// controller hands it each module's registration as the module loads, and takes it back on unload.
pub trait CorePlugin: Send + Sync {
    fn name(&self) -> &'static str;

    fn serves(&self) -> &'static [CoreCapability];

    /// Takes the parts of a registration this plugin serves. An error refuses the module's load.
    fn attach(&self, module: &ModuleManifest, registration: &Registration) -> Result<(), ModuleError>;

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
