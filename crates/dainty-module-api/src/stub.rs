//! Stand-ins that exercise every seam (RD18): [`StubModule`] registers one of everything and answers every
//! call, [`RecordingHost`] answers a module's calls and keeps them, [`RecordingPlugin`] keeps what it was
//! handed. The crate's tests run the stub compiled in and out of process (`module-api-stub`) and compare;
//! DF3's controller tests can load the same one.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::json;

use crate::actor::ActorContext;
use crate::chat::{ChatAnswer, ChatQuery, Jid, RoomKind, RoomRules};
use crate::game::{
    AVAILABILITY_CHANGED_EVENT, Availability, GameAnswer, GameCall, GameCallKind, GameServer, GameServerKind,
    UnavailableReason,
};
use crate::ids::ModuleId;
use crate::manifest::{
    Capabilities, ClientProfile, EpicBuild, ModuleKind, ModuleManifest, PatchSetCapability, PortDefault, PortRange,
    Transport,
};
use crate::module::{
    AdminCall, AdminRequest, AdminResponse, BoxFuture, CommandOutput, CommandRequest, CoreCapability, CorePlugin,
    ErrorKind, Event, EventName, GameModule, HostAnswer, HostCall, HostLink, LogLevel, ModuleConfig, ModuleContext,
    ModuleError, ModuleStatus, Phase, PlayerInfo,
};
use crate::network::{HomeApi, NodeIdentity, StandaloneHome, dispatch_home};
use crate::registrar::{AdminAction, CommandSpec, ConfigSection, Registrar, Registration};
use crate::version::API_VERSION;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn event(name: &str, payload: serde_json::Value) -> Result<Event, ModuleError> {
    let name = EventName::new(name).map_err(|error| ModuleError::new(ErrorKind::Invalid, error.to_string()))?;
    Ok(Event { name, payload })
}

#[derive(Debug, Default)]
struct StubState {
    phase: Option<Phase>,
    greeting: String,
    invites: BTreeSet<String>,
    next_invite: u32,
}

/// The stub's one game server, its hub.
fn stub_hub() -> GameServer {
    GameServer {
        id: String::from("stub-hub"),
        kind: GameServerKind::Hub,
        title: String::from("Stub hub"),
        map: String::from("stub"),
        host: String::from("127.0.0.1"),
        port: 65010,
        players: 0,
        max_players: 10,
        ready: true,
        started_at: None,
    }
}

/// A game module for game version 0.0.1 that serves nothing and answers everything.
#[derive(Debug, Default)]
pub struct StubModule {
    state: Mutex<StubState>,
}

impl StubModule {
    /// A stub with nothing recorded.
    pub fn new() -> Self {
        Self::default()
    }
}

impl GameModule for StubModule {
    fn manifest(&self) -> ModuleManifest {
        let (Ok(id), Ok(game_version), Ok(manifest_sha1), Ok(executable_sha256)) = (
            "stub".parse(),
            "0.0.1".parse(),
            "0".repeat(40).parse(),
            "0".repeat(64).parse(),
        ) else {
            unreachable!("the stub's constants parse")
        };
        ModuleManifest {
            id,
            kind: ModuleKind::Game,
            display_name: String::from("Stub"),
            game_version,
            invite_aliases: vec![String::from("stub")],
            api: API_VERSION,
            ports: vec![
                PortDefault {
                    name: String::from("http"),
                    transport: Transport::Tcp,
                    range: PortRange::single(65000),
                },
                PortDefault {
                    name: String::from("game"),
                    transport: Transport::Udp,
                    range: PortRange {
                        first: 65010,
                        last: 65019,
                    },
                },
            ],
            capabilities: Capabilities {
                chat: true,
                patch_set: Some(PatchSetCapability::Accepts),
                ..Capabilities::default()
            },
            unverified: true,
            client: Some(ClientProfile {
                executable: String::from("Stub/stub.exe"),
                executable_sha256: Some(executable_sha256),
                files: vec![],
                arguments: vec![],
                environment: BTreeMap::new(),
                ini_edits: vec![],
                epic: Some(EpicBuild {
                    version: String::from("0.0.1"),
                    manifest_sha1,
                    manifest_sources: vec![String::from("https://example.invalid/stub.manifest")],
                    executable_sha256,
                }),
            }),
        }
    }

    fn register(&self, registrar: &mut Registrar) -> Result<(), ModuleError> {
        let named = |name: &str| EventName::new(name).map_err(|e| ModuleError::new(ErrorKind::Invalid, e.to_string()));
        registrar
            .route("/stub", "http")
            .host_only_route("/stub/admin", "http")
            .udp_ports("game")
            .content_file("client/profile.json", "data/profile.json", "application/json")
            .command(CommandSpec::new("echo", "Prints the greeting and the arguments").arg(
                "words",
                false,
                "What to say",
            ))
            .command(CommandSpec::new("home", "Resolves an account's home").arg("account", true, "A global id"))
            .command(CommandSpec::new("fail", "Always fails"))
            .emits(named("stub.started")?)
            .emits(named("stub.saw")?)
            .subscribes(named("server.ready")?)
            .admin_action(AdminAction::Kick)
            .admin_action(AdminAction::Lookup)
            .emits(named(AVAILABILITY_CHANGED_EVENT)?)
            .game_call(GameCallKind::Availability)
            .game_call(GameCallKind::Servers)
            .game_call(GameCallKind::CreateInvite)
            .game_call(GameCallKind::RevokeInvite)
            .game_call(GameCallKind::AccountForKey)
            .config_section(ConfigSection::new("stub").key("greeting", json!("hello"), "What echo starts with"));
        Ok(())
    }

    fn start(&self, ctx: ModuleContext, config: ModuleConfig) -> BoxFuture<'_, Result<(), ModuleError>> {
        Box::pin(async move {
            let greeting = config
                .get("stub", "greeting")
                .and_then(|v| v.as_str())
                .unwrap_or("hello")
                .to_owned();
            {
                let mut state = lock(&self.state);
                state.phase = Some(Phase::Ready);
                state.greeting = greeting.clone();
            }
            ctx.log(LogLevel::Info, format!("stub started on {}", ctx.node().node))
                .await?;
            ctx.emit(event(
                "stub.started",
                json!({ "greeting": greeting, "ports": config.ports.len() }),
            )?)
            .await?;
            let availability = serde_json::to_value(Availability::up(stub_hub()))
                .map_err(|error| ModuleError::failed(error.to_string()))?;
            ctx.emit(event(AVAILABILITY_CHANGED_EVENT, availability)?).await
        })
    }

    fn stop(&self, ctx: ModuleContext) -> BoxFuture<'_, Result<(), ModuleError>> {
        Box::pin(async move {
            lock(&self.state).phase = Some(Phase::Stopped);
            ctx.log(LogLevel::Info, "stub stopped").await
        })
    }

    fn status(&self) -> ModuleStatus {
        ModuleStatus::new(lock(&self.state).phase.unwrap_or(Phase::Loaded))
    }

    fn command(
        &self,
        ctx: ModuleContext,
        request: CommandRequest,
    ) -> BoxFuture<'_, Result<CommandOutput, ModuleError>> {
        Box::pin(async move {
            // The stub keeps no accounts: a home invocation (DN5) is answered with what it asked
            if let Some(invocation) = crate::network::HomeInvocation::from_command(&request) {
                let invocation = invocation?;
                return Ok(CommandOutput {
                    text: String::new(),
                    data: Some(json!({
                        "from": invocation.from,
                        "account": invocation.account,
                        "operation": invocation.operation,
                        "body": invocation.body,
                    })),
                });
            }
            match request.name.as_str() {
                "echo" => {
                    let greeting = lock(&self.state).greeting.clone();
                    let mut words = vec![greeting];
                    words.extend(request.args);
                    Ok(CommandOutput {
                        text: words.join(" "),
                        data: Some(json!({ "actor": request.actor })),
                    })
                }
                "home" => {
                    let account = request
                        .args
                        .first()
                        .and_then(|arg| arg.parse().ok())
                        .ok_or_else(|| ModuleError::new(ErrorKind::Invalid, "home takes a global id"))?;
                    Ok(match ctx.resolve_home(&account).await? {
                        Ok(identity) => CommandOutput::text(format!("{} in {}", identity.node, identity.region)),
                        Err(error) => CommandOutput::text(error.to_string()),
                    })
                }
                "fail" => Err(ModuleError::failed("as asked")),
                other => Err(ModuleError::unsupported(format!("command {other}"))),
            }
        })
    }

    fn admin(&self, _ctx: ModuleContext, call: AdminCall) -> BoxFuture<'_, Result<AdminResponse, ModuleError>> {
        Box::pin(async move {
            match call.request {
                AdminRequest::Lookup { player } if player == "nobody" => Ok(AdminResponse::NotFound),
                AdminRequest::Lookup { player } => Ok(AdminResponse::Player(PlayerInfo {
                    account: None,
                    name: player,
                    online: false,
                    restriction: None,
                    ..PlayerInfo::default()
                })),
                AdminRequest::Kick { .. } => Ok(AdminResponse::Done),
                other => Err(ModuleError::unsupported(format!("{:?}", other.action()))),
            }
        })
    }

    fn event(&self, ctx: ModuleContext, event_in: Event) -> BoxFuture<'_, Result<(), ModuleError>> {
        Box::pin(async move { ctx.emit(event("stub.saw", json!({ "name": event_in.name }))?).await })
    }

    fn chat(&self, _ctx: ModuleContext, query: ChatQuery) -> BoxFuture<'_, Result<ChatAnswer, ModuleError>> {
        Box::pin(async move {
            Ok(match query {
                ChatQuery::Jid { account } => {
                    let local = account.ulid().to_string().to_lowercase();
                    let jid = Jid::new(&local, "stub.local", None)
                        .map_err(|error| ModuleError::new(ErrorKind::Failed, error.to_string()))?;
                    ChatAnswer::Jid(jid)
                }
                ChatQuery::Room { room, .. } => ChatAnswer::Room(RoomRules {
                    may_join: true,
                    may_speak: !matches!(room, RoomKind::Network),
                    history: matches!(room, RoomKind::Party(_) | RoomKind::Hunt(_)),
                }),
                other => return Err(ModuleError::unsupported(format!("chat query {other:?}"))),
            })
        })
    }

    fn game(&self, _ctx: ModuleContext, call: GameCall) -> BoxFuture<'_, Result<GameAnswer, ModuleError>> {
        Box::pin(async move {
            Ok(match call {
                GameCall::Availability => GameAnswer::Availability(self.availability()),
                GameCall::Servers => GameAnswer::Servers(self.availability().hub.into_iter().collect()),
                GameCall::CreateInvite { uses, require_up, .. } => {
                    if uses == 0 {
                        return Err(ModuleError::new(
                            ErrorKind::Invalid,
                            "an invite admits at least one login",
                        ));
                    }
                    // Checked under the same lock as the write, as a real module checks inside its own
                    let mut state = lock(&self.state);
                    if require_up && state.phase != Some(Phase::Ready) {
                        return Err(UnavailableReason::ModuleNotReady.error());
                    }
                    state.next_invite += 1;
                    let code = format!("STUB{:04}", state.next_invite);
                    state.invites.insert(code.clone());
                    GameAnswer::Invite { code }
                }
                GameCall::RevokeInvite { code } => {
                    if !lock(&self.state).invites.remove(&code) {
                        return Err(ModuleError::new(ErrorKind::NotFound, "no such invite"));
                    }
                    GameAnswer::Done
                }
                GameCall::AccountForKey { key } => GameAnswer::Account(
                    (key == STUB_ACCOUNT_KEY)
                        .then(|| STUB_ACCOUNT.parse())
                        .transpose()
                        .map_err(|_| ModuleError::failed("the stub's account id"))?,
                ),
            })
        })
    }
}

impl StubModule {
    fn availability(&self) -> Availability {
        if lock(&self.state).phase == Some(Phase::Ready) {
            Availability::up(stub_hub())
        } else {
            Availability::down(UnavailableReason::ModuleNotReady)
        }
    }
}

/// The one account key the stub knows, answered by [`GameCall::AccountForKey`].
pub const STUB_ACCOUNT_KEY: &str = "DLK_stubstubstubstubstubstubstubstub";

/// The account holding [`STUB_ACCOUNT_KEY`].
pub const STUB_ACCOUNT: &str = "account:01J9Z3N5C8Q0V2X4Y6W8T0R2P4@local";

/// A host that answers a module's calls (home calls through a [`StandaloneHome`]) and keeps every call.
pub struct RecordingHost {
    home: StandaloneHome,
    calls: Mutex<Vec<(ModuleId, HostCall)>>,
}

impl RecordingHost {
    /// A host for the node `identity`, with nothing recorded.
    pub fn new(identity: NodeIdentity) -> Self {
        Self {
            home: StandaloneHome { identity },
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Every call so far, oldest first, with the module that made it.
    pub fn calls(&self) -> Vec<(ModuleId, HostCall)> {
        lock(&self.calls).clone()
    }

    /// The home API the host answers home calls with.
    pub fn home(&self) -> &dyn HomeApi {
        &self.home
    }
}

impl HostLink for RecordingHost {
    fn call<'a>(&'a self, module: &'a ModuleId, call: HostCall) -> BoxFuture<'a, Result<HostAnswer, ModuleError>> {
        Box::pin(async move {
            lock(&self.calls).push((module.clone(), call.clone()));
            Ok(match call {
                HostCall::Emit(_) | HostCall::Log(_) => HostAnswer::Done,
                HostCall::Home(home) => HostAnswer::Home(dispatch_home(&self.home, home).await),
            })
        })
    }
}

/// A core plugin that serves every capability and keeps each module's registration.
#[derive(Default)]
pub struct RecordingPlugin {
    attached: Mutex<BTreeMap<ModuleId, Registration>>,
}

impl RecordingPlugin {
    /// Each attached module's registration.
    pub fn attached(&self) -> BTreeMap<ModuleId, Registration> {
        lock(&self.attached).clone()
    }
}

impl CorePlugin for RecordingPlugin {
    fn name(&self) -> &'static str {
        "recording"
    }

    fn serves(&self) -> &'static [CoreCapability] {
        &[
            CoreCapability::Routes,
            CoreCapability::UdpPorts,
            CoreCapability::ContentFiles,
            CoreCapability::Commands,
            CoreCapability::Events,
            CoreCapability::AdminActions,
            CoreCapability::ConfigSections,
            CoreCapability::Chat,
            CoreCapability::Network,
        ]
    }

    fn attach(&self, module: &ModuleManifest, registration: &Registration) -> Result<(), ModuleError> {
        let mut attached = lock(&self.attached);
        if attached.contains_key(&module.id) {
            return Err(ModuleError::new(
                ErrorKind::Invalid,
                format!("{} is already attached", module.id),
            ));
        }
        attached.insert(module.id.clone(), registration.clone());
        Ok(())
    }

    fn detach(&self, module: &ModuleId) {
        lock(&self.attached).remove(module);
    }
}

/// The actor the stub tests send commands as.
pub fn console() -> ActorContext {
    ActorContext::new(crate::actor::Actor::Console)
}

/// A shared [`RecordingHost`] for a standalone node, ready to hand to a driver.
pub fn recording_host() -> Arc<RecordingHost> {
    Arc::new(RecordingHost::new(NodeIdentity::standalone()))
}
