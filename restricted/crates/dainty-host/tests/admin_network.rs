//! Admin actions across nodes (DN11, DD16, DD23): a three-node cluster (`germany-1`, `oregon-1`,
//! `new-york-1`), each with its own directory replica, session-ticket authority, `NetworkedHome`,
//! game module and `Admin` service wired over `Service::Admin`.
//!
//! Verifies:
//! 1. A ban on one node refuses the player everywhere: session tickets are revoked on all three
//!    nodes immediately (`holds` is false, `admit` and `forward` fail with `Denied`), all three
//!    nodes' game modules apply the ban under `Actor::Admin { key: <originating key id> }`, and
//!    `admin_actions` on the peer nodes records `via_node = "oregon-1"`.
//! 2. Guarded actions across nodes (DH6): when a bulk ban is held and approved by a second
//!    operator on `oregon-1`, `approved_by` is propagated over `Service::Admin` and recorded in
//!    `admin_actions` on every node.
//! 3. Down-peer queue and idempotent replay: when `new-york-1` is partitioned, a ban on `oregon-1`
//!    still succeeds on `oregon-1` and `germany-1`, records `"pending on node new-york-1"` in
//!    `admin_actions.detail` and `pendingRelay`, persists across a restart of `AdminRelay`, and
//!    applies on `new-york-1` with the original `request_key` once the link recovers and `sweep()`
//!    runs (with duplicate replays deduplicated).
//! 4. Real mutual-TLS end-to-end: `TlsPeerLink` + `Gateway` across two real HTTPS gateways on
//!    `127.0.0.1:0` carrying `Service::Admin` ban and ticket revocation.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dainty_admin::config::{Config, ServerMode};
use dainty_admin::guard::{GuardSettings, Rule};
use dainty_admin::keys::{Caller, Keys, Role};
use dainty_admin::module_control::Modules;
use dainty_admin::modules::{AdminCalls, AdminPlugin};
use dainty_admin::relay::ConnectedNetwork;
use dainty_admin::schema::Admin;
use dainty_admin::upstream::Operator;
use dainty_module_api::driver::{InProcessModule, ModuleDriver, ModuleDriverExt};
use dainty_module_api::module::PlayerInfo;
use dainty_module_api::network::{GlobalId, HomeApi, HomeError, IdKind, NodeId, NodeIdentity};
use dainty_module_api::registrar::{AdminAction, Registrar};
use dainty_module_api::stub::recording_host;
use dainty_module_api::{
    API_VERSION, Actor, ActorContext, AdminCall, AdminRequest, AdminResponse, BoxFuture, Capabilities, CorePlugin,
    ErrorKind, GameModule, GameVersion, HomeBackend, HomeForward, HomeInvocation, ModuleConfig, ModuleContext,
    ModuleError, ModuleId, ModuleManifest, ModuleStatus, Phase,
};
use dainty_store::{Database, DatabaseConfig, DatabaseOptions};
use network_plugin::directory::{Directory, DirectoryConfig, SCHEMA};
use network_plugin::home::{
    HomeConfig, HomeRouter, HomeService, NetworkedConfig, NetworkedHome, SigningKey, TicketConfig, TicketError, Tickets,
};
use network_plugin::inprocess::InProcessNetwork;
use network_plugin::link::{Clock, Service, ServiceMux};
use serde_json::{Value, json};

const LEADER: &str = "germany-1";
const NODES: [&str; 3] = ["germany-1", "oregon-1", "new-york-1"];
const GAME_MODULE: &str = "isles-1-14-7";
const GAME_VERSION: &str = "1.14.7";
const START_MS: i64 = 1_800_000_000_000;
const OWNER_SECRET: &str = "owner-secret-0123456789abcdef0123456789";

fn node_id(name: &str) -> NodeId {
    name.parse().unwrap()
}

fn game_version() -> GameVersion {
    GAME_VERSION.parse().unwrap()
}

fn node_identity(name: &str) -> NodeIdentity {
    let region = name.rsplit_once('-').map_or(name, |(r, _)| r);
    NodeIdentity {
        node: node_id(name),
        region: region.parse().unwrap(),
    }
}

#[derive(Clone, Debug)]
struct StubPlayer {
    account: GlobalId,
    name: String,
    online: bool,
    admin: bool,
    banned_reason: Option<String>,
}

struct NodeGame {
    players: Mutex<Vec<StubPlayer>>,
    calls: Mutex<Vec<AdminCall>>,
    home_writes: Mutex<Vec<HomeInvocation>>,
    game: GameVersion,
}

impl NodeGame {
    fn new(players: Vec<StubPlayer>) -> Arc<Self> {
        Arc::new(Self {
            players: Mutex::new(players),
            calls: Mutex::new(Vec::new()),
            home_writes: Mutex::new(Vec::new()),
            game: game_version(),
        })
    }

    fn calls(&self) -> Vec<AdminCall> {
        self.calls.lock().unwrap().clone()
    }

    fn mutations(&self) -> Vec<AdminCall> {
        self.calls()
            .into_iter()
            .filter(|c| c.request.action() != AdminAction::Lookup)
            .collect()
    }

    fn is_banned(&self, name: &str) -> bool {
        self.players
            .lock()
            .unwrap()
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
            .and_then(|p| p.banned_reason.as_ref())
            .is_some()
    }
}

impl GameModule for NodeGame {
    fn manifest(&self) -> ModuleManifest {
        ModuleManifest {
            id: GAME_MODULE.parse().unwrap(),
            kind: dainty_module_api::ModuleKind::Game,
            display_name: "Rustic Isles 1.14.7".into(),
            game_version: self.game.clone(),
            invite_aliases: vec![],
            api: API_VERSION,
            ports: vec![],
            capabilities: Capabilities::default(),
            unverified: false,
            client: None,
        }
    }

    fn register(&self, registrar: &mut Registrar) -> Result<(), ModuleError> {
        registrar.admin_action(AdminAction::Lookup);
        registrar.admin_action(AdminAction::Kick);
        registrar.admin_action(AdminAction::Ban);
        Ok(())
    }

    fn start(&self, _ctx: ModuleContext, _config: ModuleConfig) -> BoxFuture<'_, Result<(), ModuleError>> {
        Box::pin(async { Ok(()) })
    }

    fn stop(&self, _ctx: ModuleContext) -> BoxFuture<'_, Result<(), ModuleError>> {
        Box::pin(async { Ok(()) })
    }

    fn status(&self) -> ModuleStatus {
        ModuleStatus::new(Phase::Ready)
    }

    fn admin(&self, _ctx: ModuleContext, call: AdminCall) -> BoxFuture<'_, Result<AdminResponse, ModuleError>> {
        Box::pin(async move {
            let mut players = self.players.lock().unwrap();
            let (AdminRequest::Kick { player, .. }
            | AdminRequest::Ban { player, .. }
            | AdminRequest::Unban { player, .. }
            | AdminRequest::LiftTimeout { player, .. }
            | AdminRequest::Lookup { player }) = &call.request;
            let Some(found) = players.iter_mut().find(|p| p.name.eq_ignore_ascii_case(player)) else {
                self.calls.lock().unwrap().push(call);
                return Ok(AdminResponse::NotFound);
            };
            match &call.request {
                AdminRequest::Lookup { .. } => {
                    let info = PlayerInfo {
                        account: Some(found.account.clone()),
                        name: found.name.clone(),
                        online: found.online,
                        restriction: None,
                    };
                    drop(players);
                    self.calls.lock().unwrap().push(call);
                    Ok(AdminResponse::Player(info))
                }
                _ if found.admin => Err(ModuleError::failed("this player is a server admin")),
                AdminRequest::Kick { .. } => {
                    found.online = false;
                    drop(players);
                    self.calls.lock().unwrap().push(call);
                    Ok(AdminResponse::Done)
                }
                AdminRequest::Ban { reason, .. } => {
                    found.online = false;
                    found.banned_reason = Some(reason.clone().unwrap_or_else(|| "Banned".into()));
                    drop(players);
                    self.calls.lock().unwrap().push(call);
                    Ok(AdminResponse::Done)
                }
                AdminRequest::Unban { .. } | AdminRequest::LiftTimeout { .. } => {
                    found.banned_reason = None;
                    drop(players);
                    self.calls.lock().unwrap().push(call);
                    Ok(AdminResponse::Done)
                }
            }
        })
    }
}

impl HomeBackend for NodeGame {
    fn game(&self) -> &GameVersion {
        &self.game
    }

    fn run<'a>(&'a self, invocation: HomeInvocation) -> BoxFuture<'a, Result<Value, ModuleError>> {
        Box::pin(async move {
            self.home_writes.lock().unwrap().push(invocation);
            Ok(json!({ "saved": true }))
        })
    }
}

struct Drivers(BTreeMap<ModuleId, InProcessModule>);

impl AdminCalls for Drivers {
    fn admin<'a>(&'a self, module: &'a ModuleId, call: AdminCall) -> BoxFuture<'a, Result<AdminResponse, ModuleError>> {
        Box::pin(async move {
            match self.0.get(module) {
                Some(driver) => driver.admin(call).await,
                None => Err(ModuleError::new(ErrorKind::Unavailable, "not loaded")),
            }
        })
    }
}

async fn build_admin_plugin(game: Arc<NodeGame>, node: NodeIdentity) -> Arc<AdminPlugin> {
    let module: Arc<dyn GameModule> = game;
    let driver = InProcessModule::new(module, node, recording_host()).unwrap();
    let manifest = driver.manifest().clone();
    let registration = driver.register().await.unwrap();
    let mut drivers = BTreeMap::new();
    drivers.insert(manifest.id.clone(), driver);
    let plugin = Arc::new(AdminPlugin::new(Arc::new(Drivers(drivers))));
    plugin.attach(&manifest, &registration).unwrap();
    plugin
}

struct ClusterNode {
    dir: Arc<Directory>,
    tickets: Arc<Tickets>,
    home: Arc<NetworkedHome>,
    game: Arc<NodeGame>,
    admin: Arc<Admin>,
    config: Config,
    _mux: Arc<ServiceMux>,
    _temp: tempfile::TempDir,
}

impl ClusterNode {
    /// Reads `admin_actions` rows from the change log, newest last.
    async fn recorded_actions(&self) -> Vec<Value> {
        let entries = self.admin.keys.database().changes(0, 200).await.unwrap();
        entries
            .into_iter()
            .filter(|e| e.table_name == "admin_actions" && e.op == "insert")
            .filter_map(|e| serde_json::from_str::<Value>(e.after.as_deref()?).ok())
            .collect()
    }
}

struct AdminCluster {
    net: InProcessNetwork,
    nodes: HashMap<&'static str, ClusterNode>,
}

impl AdminCluster {
    async fn new(players: Vec<StubPlayer>, guard: GuardSettings) -> Self {
        let net = InProcessNetwork::new();
        let net_clock = Arc::new(Clock::default());
        let admin_clock = Arc::new(AtomicI64::new(START_MS));
        let mut nodes = HashMap::new();

        for name in NODES {
            let temp = tempfile::tempdir().unwrap();
            let me = node_identity(name);
            let link = Arc::new(net.link(&me.node));
            let dir_db = Database::open(
                &DatabaseConfig::Sqlite {
                    file: temp.path().join("directory.db"),
                },
                &DatabaseOptions::default(),
                &SCHEMA,
            )
            .unwrap();
            let dir = Arc::new(
                Directory::new(
                    dir_db,
                    DirectoryConfig::in_network(me.node.clone(), node_id(LEADER)).with_page(16),
                    link.clone(),
                )
                .await
                .unwrap(),
            );
            let tickets = Arc::new(Tickets::new(
                &me,
                SigningKey::generate().unwrap(),
                net_clock.clone(),
                TicketConfig::default(),
            ));
            let game = NodeGame::new(players.clone());
            let router = Arc::new(HomeRouter::default());
            router.register(GAME_MODULE.parse().unwrap(), game.clone());
            let peers: Vec<NodeId> = NODES.iter().filter(|p| **p != name).map(|p| node_id(p)).collect();
            let home_service = Arc::new(HomeService::new(
                me.clone(),
                tickets.clone(),
                router,
                dir.clone(),
                peers.clone(),
                HomeConfig::default(),
            ));
            let mut home_cfg = NetworkedConfig::new(game_version());
            home_cfg.retry_after = Duration::from_millis(10);
            let home = Arc::new(NetworkedHome::new(
                me.clone(),
                NODES
                    .iter()
                    .filter(|p| **p != name)
                    .map(|p| (node_id(p), node_identity(p).region)),
                link.clone(),
                dir.clone(),
                tickets.clone(),
                home_cfg,
            ));

            let plugin = build_admin_plugin(game.clone(), me.clone()).await;
            let timeouts_file = temp.path().join("timeouts.json");
            let config = Config {
                port: 0,
                bind_hosts: vec!["127.0.0.1".into()],
                backend_url: "http://127.0.0.1:9".into(),
                director_url: "http://127.0.0.1:9".into(),
                api_key: OWNER_SECRET.into(),
                admin_key: "admin-key-0123456789abcdef01234567".into(),
                operator_secret: "operator-secret-0123456789abcdef01".into(),
                timeouts_file: timeouts_file.clone(),
                max_hunt_servers: 3,
                server_mode: ServerMode::Public,
                database: DatabaseConfig::Sqlite {
                    file: temp.path().join("dainty-lass.db"),
                },
                guard: guard.clone(),
                metrics_token: None,
            };
            let network =
                ConnectedNetwork::new(me.node.clone(), link.clone(), Some(dir.clone()), Some(tickets.clone()));
            let clock_read = admin_clock.clone();
            let (_, admin) = dainty_admin::server::prepare_with_network(
                &config,
                Arc::new(move || clock_read.load(Ordering::SeqCst)),
                plugin,
                Modules::default(),
                network,
            )
            .unwrap();

            let mux = Arc::new(
                ServiceMux::new(me.node.clone())
                    .with(Service::Directory, dir.clone())
                    .with(Service::Home, home_service)
                    .with(Service::Admin, admin.relay_service()),
            );
            let as_service: Arc<dyn network_plugin::NodeService> = mux.clone();
            net.attach(&me.node, &as_service);

            nodes.insert(
                name,
                ClusterNode {
                    dir,
                    tickets,
                    home,
                    game,
                    admin,
                    config,
                    _mux: mux,
                    _temp: temp,
                },
            );
        }

        Self { net, nodes }
    }

    fn node(&self, name: &str) -> &ClusterNode {
        self.nodes.get(name).unwrap()
    }

    async fn register_directory(&self, home: &str, account: &GlobalId, username: &str) {
        self.node(home)
            .dir
            .register(dainty_store::Actor::Player(account.to_string()), account, username)
            .await
            .unwrap();
        for name in NODES {
            self.node(name).dir.sync().await.unwrap();
        }
    }
}

fn caller(key: &str, role: Role, operator: &str) -> Caller {
    Caller {
        key: key.to_owned(),
        role,
        operator: Operator::parse(operator).unwrap(),
    }
}

#[tokio::test]
async fn ban_on_one_node_revokes_tickets_and_refuses_player_everywhere() {
    let germany = node_id("germany-1");
    let oregon = node_id("oregon-1");
    let new_york = node_id("new-york-1");

    let fox_account = GlobalId::new(IdKind::Account, &germany);
    let admin_account = GlobalId::new(IdKind::Account, &germany);
    let players = vec![
        StubPlayer {
            account: fox_account.clone(),
            name: "Fox".into(),
            online: true,
            admin: false,
            banned_reason: None,
        },
        StubPlayer {
            account: admin_account.clone(),
            name: "Cyra".into(),
            online: true,
            admin: true,
            banned_reason: None,
        },
    ];

    let cluster = AdminCluster::new(players, GuardSettings::default()).await;
    cluster.register_directory("germany-1", &fox_account, "Fox").await;
    cluster.register_directory("germany-1", &admin_account, "Cyra").await;

    // Issue a moderator key on oregon-1 so we can verify Actor::Admin { key } across all 3 nodes
    let owner = caller("owner", Role::Owner, "100");
    let issued = cluster
        .node("oregon-1")
        .admin
        .issue_key(&owner, "Oregon Mod", Role::Moderator)
        .await
        .unwrap();
    let mod_caller = caller(&issued.info.id, Role::Moderator, "4242");

    // Mint session tickets for Fox at Fox's home (germany-1) for oregon-1 and new-york-1, and admit them
    let ticket_oregon = cluster
        .node("germany-1")
        .home
        .session_ticket(&fox_account, &oregon)
        .await
        .unwrap();
    let ticket_ny = cluster
        .node("germany-1")
        .home
        .session_ticket(&fox_account, &new_york)
        .await
        .unwrap();
    let fresh_ticket_ny = cluster
        .node("germany-1")
        .home
        .session_ticket(&fox_account, &new_york)
        .await
        .unwrap();

    let gv = game_version();
    cluster.node("oregon-1").home.admit(&ticket_oregon, &gv).await.unwrap();
    cluster.node("new-york-1").home.admit(&ticket_ny, &gv).await.unwrap();
    assert!(cluster.node("oregon-1").home.holds(&fox_account));
    assert!(cluster.node("new-york-1").home.holds(&fox_account));

    // Protected account at the home (germany-1) refuses a ban initiated on oregon-1
    let err = cluster
        .node("oregon-1")
        .admin
        .ban(&mod_caller, "Cyra", "testing protected", Some(GAME_VERSION))
        .await
        .unwrap_err();
    assert_eq!(
        err.extensions.as_ref().and_then(|e| e.get("code")),
        Some(&async_graphql::Value::String("BACKEND_REFUSED".into()))
    );
    assert!(err.message.contains("this player is a server admin"));

    // Ban Fox on oregon-1
    let banned = cluster
        .node("oregon-1")
        .admin
        .ban(&mod_caller, "Fox", "griefing hunts across regions", Some(GAME_VERSION))
        .await
        .unwrap();
    assert_eq!(banned.username, "Fox");
    assert_eq!(banned.user_id, fox_account.to_string());

    // 1. All 3 nodes' game modules applied the ban with Actor::Admin { key: issued.info.id }
    for node_name in NODES {
        let game = &cluster.node(node_name).game;
        assert!(game.is_banned("Fox"), "Fox must be banned on {node_name}");
        let mutations = game.mutations();
        assert_eq!(mutations.len(), 1, "expected 1 mutation on {node_name}");
        assert_eq!(
            mutations[0].actor.actor,
            Actor::Admin {
                key: issued.info.id.clone(),
            }
        );
        assert_eq!(
            mutations[0].request,
            AdminRequest::Ban {
                player: "Fox".into(),
                reason: Some("griefing hunts across regions".into()),
                until: None,
            }
        );
    }

    // 2. Session tickets for Fox are revoked on every node: held sessions stop working and new tickets fail
    for node_name in NODES {
        assert!(
            cluster.node(node_name).tickets.is_revoked_at(&fox_account, 0),
            "tickets for Fox must be revoked on {node_name}"
        );
    }
    assert!(!cluster.node("oregon-1").home.holds(&fox_account));
    assert!(!cluster.node("new-york-1").home.holds(&fox_account));

    // Forwarding a home operation from oregon-1 or admitting a previously minted ticket on new-york-1 is refused
    let forward_err = cluster
        .node("oregon-1")
        .home
        .forward(&HomeForward {
            module: GAME_MODULE.parse().unwrap(),
            operation: "progression.grant".parse().unwrap(),
            account: fox_account.clone(),
            actor: ActorContext::new(Actor::Player {
                account: fox_account.clone(),
            }),
            key: None,
            body: json!({ "xp": 100 }),
        })
        .await
        .unwrap_err();
    assert!(matches!(forward_err, HomeError::Denied(_)), "got {forward_err:?}");

    let admit_err = cluster
        .node("new-york-1")
        .home
        .admit(&fresh_ticket_ny, &gv)
        .await
        .unwrap_err();
    assert!(matches!(admit_err, HomeError::Denied(_)), "got {admit_err:?}");

    // Any ticket issued before or during the ban fails verification at germany-1 with TicketError::Revoked
    let verify_err = cluster
        .node("germany-1")
        .tickets
        .verify(
            &fresh_ticket_ny,
            &network_plugin::home::Expect {
                audience: &new_york,
                game: Some(&gv),
                account: Some(&fox_account),
                issuer: &germany,
            },
        )
        .unwrap_err();
    assert_eq!(verify_err, TicketError::Revoked);

    // 3. Audit log on germany-1 and new-york-1 records via_node = "oregon-1" and the originating key id
    let oregon_actions = cluster.node("oregon-1").recorded_actions().await;
    let oregon_ban = oregon_actions
        .iter()
        .find(|a| a["key_id"] == issued.info.id && a["action"] == "ban" && a["target"] == "Fox")
        .expect("oregon ban row");
    assert!(oregon_ban["via_node"].is_null());
    let req_key = oregon_ban["request_key"].as_str().expect("request_key present");

    for peer in ["germany-1", "new-york-1"] {
        let peer_actions = cluster.node(peer).recorded_actions().await;
        let peer_ban = peer_actions
            .iter()
            .find(|a| a["key_id"] == issued.info.id && a["action"] == "ban" && a["target"] == "Fox")
            .expect("peer ban row");
        assert_eq!(peer_ban["via_node"], "oregon-1");
        assert_eq!(peer_ban["request_key"], req_key);
    }
}

#[tokio::test]
async fn guarded_approval_propagates_second_operator_across_nodes() {
    let germany = node_id("germany-1");
    let ada = GlobalId::new(IdKind::Account, &germany);
    let bo = GlobalId::new(IdKind::Account, &germany);
    let players = vec![
        StubPlayer {
            account: ada.clone(),
            name: "Ada".into(),
            online: true,
            admin: false,
            banned_reason: None,
        },
        StubPlayer {
            account: bo.clone(),
            name: "Bo_Slayer".into(),
            online: true,
            admin: false,
            banned_reason: None,
        },
    ];
    let guard = GuardSettings {
        ban_limit: 1,
        ban_window_minutes: 10,
        ban_guard: Rule::Approval,
        ..GuardSettings::default()
    };
    let cluster = AdminCluster::new(players, guard).await;
    cluster.register_directory("germany-1", &ada, "Ada").await;
    cluster.register_directory("germany-1", &bo, "Bo_Slayer").await;

    let owner = caller("owner", Role::Owner, "1");
    let k1 = cluster
        .node("oregon-1")
        .admin
        .issue_key(&owner, "Mod One", Role::Moderator)
        .await
        .unwrap();
    let k2 = cluster
        .node("oregon-1")
        .admin
        .issue_key(&owner, "Mod Two", Role::Moderator)
        .await
        .unwrap();
    let mod1 = caller(&k1.info.id, Role::Moderator, "101");
    let mod2 = caller(&k2.info.id, Role::Moderator, "202");

    // First ban runs immediately
    cluster
        .node("oregon-1")
        .admin
        .ban(&mod1, "Ada", "first ban under limit", Some(GAME_VERSION))
        .await
        .unwrap();

    // Second ban exceeds ban_limit = 1 and is held for approval
    let held_err = cluster
        .node("oregon-1")
        .admin
        .ban(&mod1, "Bo_Slayer", "second ban held for approval", Some(GAME_VERSION))
        .await
        .unwrap_err();
    let guarded_id = match held_err.extensions.as_ref().and_then(|e| e.get("guardedId")) {
        Some(async_graphql::Value::String(id)) => id.clone(),
        other => panic!("expected GUARDED with guardedId, got {other:?}"),
    };
    assert!(!cluster.node("germany-1").game.is_banned("Bo_Slayer"));

    // Second operator (mod2) approves on oregon-1
    let approved = cluster
        .node("oregon-1")
        .admin
        .approve(&mod2, &guarded_id)
        .await
        .unwrap();
    assert_eq!(approved.approved_by.as_deref(), Some(k2.info.id.as_str()));

    // All 3 nodes applied the ban and recorded both initiated_by (k1) and approved_by (k2)
    for node_name in NODES {
        assert!(cluster.node(node_name).game.is_banned("Bo_Slayer"));
        let actions = cluster.node(node_name).recorded_actions().await;
        let bo_ban = actions
            .iter()
            .find(|a| a["key_id"] == k1.info.id && a["action"] == "ban" && a["target"] == "Bo_Slayer")
            .expect("Bo_Slayer ban recorded");
        assert_eq!(bo_ban["approved_by"], k2.info.id);
        if node_name != "oregon-1" {
            assert_eq!(bo_ban["via_node"], "oregon-1");
        }
    }
}

#[tokio::test]
async fn down_peer_queues_pending_survives_restart_and_replays_idempotently() {
    let germany = node_id("germany-1");
    let new_york = node_id("new-york-1");
    let wren = GlobalId::new(IdKind::Account, &germany);
    let players = vec![StubPlayer {
        account: wren.clone(),
        name: "Wren".into(),
        online: true,
        admin: false,
        banned_reason: None,
    }];

    let cluster = AdminCluster::new(players, GuardSettings::default()).await;
    cluster.register_directory("germany-1", &wren, "Wren").await;

    // Partition new-york-1 before banning Wren on oregon-1
    cluster.net.isolate(&new_york);

    let owner = caller("owner", Role::Owner, "77");
    let banned = cluster
        .node("oregon-1")
        .admin
        .ban(&owner, "Wren", "exploiting geometry", Some(GAME_VERSION))
        .await
        .unwrap();
    assert_eq!(banned.username, "Wren");

    // Applied on oregon-1 and germany-1 immediately; not yet on new-york-1
    assert!(cluster.node("oregon-1").game.is_banned("Wren"));
    assert!(cluster.node("germany-1").game.is_banned("Wren"));
    assert!(!cluster.node("new-york-1").game.is_banned("Wren"));

    // Audit detail on oregon-1 notes "pending on node new-york-1"
    let oregon_actions = cluster.node("oregon-1").recorded_actions().await;
    assert_eq!(oregon_actions.len(), 1);
    assert_eq!(
        oregon_actions[0]["detail"],
        "exploiting geometry (pending on node new-york-1)"
    );

    // pendingRelay query lists the queued row with status "pending on node new-york-1"
    let pending = cluster.node("oregon-1").admin.pending_relay(&owner).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].node, "new-york-1");
    assert_eq!(pending[0].operation, "admin.ban");
    assert_eq!(pending[0].target.as_deref(), Some("Wren"));
    assert_eq!(pending[0].status, "pending on node new-york-1");

    // Re-open the database from disk (simulating a restart of oregon-1) and verify the queue persisted
    let reopened_keys = Keys::open(
        &Keys::database_for(&cluster.node("oregon-1").config.database),
        OWNER_SECRET,
    )
    .unwrap();
    let reopened_relay = dainty_admin::relay::AdminRelay::new(
        reopened_keys.database().clone(),
        cluster.node("oregon-1").admin.relay.network().clone(),
    );
    let persisted = reopened_relay.pending().await.unwrap();
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].status, "pending on node new-york-1");
    let saved_key = persisted[0].request_key.clone();

    // Heal the network partition and run sweep()
    cluster.net.heal();
    cluster.node("oregon-1").admin.sweep().await;

    // Now new-york-1 has banned Wren, revoked Wren's tickets, and the queue on oregon-1 is empty
    assert!(cluster.node("new-york-1").game.is_banned("Wren"));
    assert!(cluster.node("new-york-1").tickets.is_revoked_at(&wren, 0));
    assert!(
        cluster
            .node("oregon-1")
            .admin
            .pending_relay(&owner)
            .await
            .unwrap()
            .is_empty()
    );

    // Replaying the exact same RelayAction with the same request_key to new-york-1 is idempotent
    let wire = network_plugin::relay::RelayAction {
        key: network_plugin::home::protocol::RequestKey::new(saved_key).unwrap(),
        operation: "admin.ban".parse().unwrap(),
        account: Some(wren.clone()),
        revoke_tickets: true,
        actor_key: network_plugin::home::protocol::CleanText::new("owner").unwrap(),
        operator: network_plugin::home::protocol::CleanText::new("77").unwrap(),
        approved_by: None,
        body: network_plugin::home::protocol::HomeBody::new(json!({
            "username": "Wren",
            "reason": "exploiting geometry",
            "game": GAME_VERSION,
        }))
        .unwrap(),
    };
    let _ = cluster
        .node("oregon-1")
        .admin
        .relay
        .call_peer(&new_york, &wire)
        .await
        .unwrap();

    // new-york-1 only mutated its game module once and only recorded one audit row
    assert_eq!(cluster.node("new-york-1").game.mutations().len(), 1);
    let ny_actions = cluster.node("new-york-1").recorded_actions().await;
    assert_eq!(ny_actions.len(), 1);
    assert_eq!(ny_actions[0]["via_node"], "oregon-1");
}

#[tokio::test]
async fn real_mtls_gateway_node_links_carry_admin_relay() {
    use dainty_gateway::gateway::{Gateway, Tls as GatewayTls};
    use dainty_service_kit::env::Env;
    use network_plugin::cluster::{ClusterCa, NodeCertRequest, NodeCredentials, Peer};
    use network_plugin::{TlsLinkConfig, TlsPeerLink};
    use tokio::sync::watch;

    let ca = ClusterCa::create(30).unwrap();
    let oregon_id = node_identity("oregon-1");
    let germany_id = node_identity("germany-1");

    let oregon_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let germany_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let oregon_port = oregon_listener.local_addr().unwrap().port();
    let germany_port = germany_listener.local_addr().unwrap().port();

    let oregon_peer = Peer {
        id: oregon_id.node.clone(),
        region: oregon_id.region.clone(),
        host: "127.0.0.1".into(),
        port: oregon_port,
        fingerprint: None,
        link: None,
    };
    let germany_peer = Peer {
        id: germany_id.node.clone(),
        region: germany_id.region.clone(),
        host: "127.0.0.1".into(),
        port: germany_port,
        fingerprint: None,
        link: None,
    };

    let cert_dir = tempfile::tempdir().unwrap();
    let write_node_files = |me: &NodeIdentity| {
        let issued = ca
            .issue(&NodeCertRequest {
                node: me.node.clone(),
                region: me.region.clone(),
                hosts: vec!["127.0.0.1".into()],
                days: 30,
            })
            .unwrap();
        let cert_path = cert_dir.path().join(format!("{}-cert.pem", me.node));
        let key_path = cert_dir.path().join(format!("{}-key.pem", me.node));
        let ca_path = cert_dir.path().join(format!("{}-ca.pem", me.node));
        std::fs::write(&cert_path, &issued.cert_pem).unwrap();
        std::fs::write(&key_path, &issued.key_pem).unwrap();
        std::fs::write(&ca_path, ca.certificate_pem()).unwrap();
        let creds = NodeCredentials::load(&cert_path, &key_path, &ca_path).unwrap();
        (cert_path, key_path, ca_path, creds)
    };

    let (or_cert, or_key, or_ca, oregon_creds) = write_node_files(&oregon_id);
    let (de_cert, de_key, de_ca, germany_creds) = write_node_files(&germany_id);

    let oregon_link = Arc::new(TlsPeerLink::new(TlsLinkConfig::new(oregon_creds, vec![germany_peer.clone()])).unwrap());
    let germany_link =
        Arc::new(TlsPeerLink::new(TlsLinkConfig::new(germany_creds, vec![oregon_peer.clone()])).unwrap());

    let net_clock = Arc::new(Clock::default());
    let admin_clock = Arc::new(AtomicI64::new(START_MS));

    let fox = GlobalId::new(IdKind::Account, &germany_id.node);
    let players = vec![StubPlayer {
        account: fox.clone(),
        name: "Fox".into(),
        online: true,
        admin: false,
        banned_reason: None,
    }];

    let temp_de = tempfile::tempdir().unwrap();
    let temp_or = tempfile::tempdir().unwrap();

    let dir_de = Arc::new(
        Directory::new(
            Database::open(
                &DatabaseConfig::Sqlite {
                    file: temp_de.path().join("dir.db"),
                },
                &DatabaseOptions::default(),
                &SCHEMA,
            )
            .unwrap(),
            DirectoryConfig::in_network(germany_id.node.clone(), germany_id.node.clone()),
            germany_link.clone(),
        )
        .await
        .unwrap(),
    );
    let dir_or = Arc::new(
        Directory::new(
            Database::open(
                &DatabaseConfig::Sqlite {
                    file: temp_or.path().join("dir.db"),
                },
                &DatabaseOptions::default(),
                &SCHEMA,
            )
            .unwrap(),
            DirectoryConfig::in_network(oregon_id.node.clone(), germany_id.node.clone()),
            oregon_link.clone(),
        )
        .await
        .unwrap(),
    );

    let tickets_de = Arc::new(Tickets::new(
        &germany_id,
        SigningKey::generate().unwrap(),
        net_clock.clone(),
        TicketConfig::default(),
    ));
    let tickets_or = Arc::new(Tickets::new(
        &oregon_id,
        SigningKey::generate().unwrap(),
        net_clock.clone(),
        TicketConfig::default(),
    ));

    let game_de = NodeGame::new(players.clone());
    let game_or = NodeGame::new(players);

    let make_admin = |dir: &tempfile::TempDir,
                      me: &NodeIdentity,
                      link: Arc<TlsPeerLink>,
                      directory: Arc<Directory>,
                      tickets: Arc<Tickets>,
                      plugin: Arc<AdminPlugin>| {
        let cfg = Config {
            port: 0,
            bind_hosts: vec!["127.0.0.1".into()],
            backend_url: "http://127.0.0.1:9".into(),
            director_url: "http://127.0.0.1:9".into(),
            api_key: OWNER_SECRET.into(),
            admin_key: "admin-key-0123456789abcdef01234567".into(),
            operator_secret: "operator-secret-0123456789abcdef01".into(),
            timeouts_file: dir.path().join("timeouts.json"),
            max_hunt_servers: 3,
            server_mode: ServerMode::Public,
            database: DatabaseConfig::Sqlite {
                file: dir.path().join("dainty-lass.db"),
            },
            guard: GuardSettings::default(),
            metrics_token: None,
        };
        let net = ConnectedNetwork::new(me.node.clone(), link, Some(directory), Some(tickets));
        let clk = admin_clock.clone();
        dainty_admin::server::prepare_with_network(
            &cfg,
            Arc::new(move || clk.load(Ordering::SeqCst)),
            plugin,
            Modules::default(),
            net,
        )
        .unwrap()
        .1
    };

    let plugin_de = build_admin_plugin(game_de.clone(), germany_id.clone()).await;
    let plugin_or = build_admin_plugin(game_or.clone(), oregon_id.clone()).await;
    let admin_de = make_admin(
        &temp_de,
        &germany_id,
        germany_link.clone(),
        dir_de.clone(),
        tickets_de.clone(),
        plugin_de,
    );
    let admin_or = make_admin(
        &temp_or,
        &oregon_id,
        oregon_link.clone(),
        dir_or.clone(),
        tickets_or.clone(),
        plugin_or,
    );

    let mux_de: Arc<dyn network_plugin::NodeService> = Arc::new(
        ServiceMux::new(germany_id.node.clone())
            .with(Service::Directory, dir_de.clone())
            .with(Service::Admin, admin_de.relay_service()),
    );
    let mux_or: Arc<dyn network_plugin::NodeService> = Arc::new(
        ServiceMux::new(oregon_id.node.clone())
            .with(Service::Directory, dir_or.clone())
            .with(Service::Admin, admin_or.relay_service()),
    );

    let start_gateway = |me: &NodeIdentity,
                         port: u16,
                         cert: &std::path::Path,
                         key: &std::path::Path,
                         ca_file: &std::path::Path,
                         peers: &[Peer],
                         listener: tokio::net::TcpListener,
                         service: Arc<dyn network_plugin::NodeService>| {
        let cert_s = cert.to_string_lossy().into_owned();
        let key_s = key.to_string_lossy().into_owned();
        let ca_s = ca_file.to_string_lossy().into_owned();
        let port_s = port.to_string();
        let peers_json = serde_json::to_string(peers).unwrap();
        let secret = "a".repeat(64);
        let env = Env::from_pairs(vec![
            ("GATEWAY_BIND", "127.0.0.1"),
            ("GATEWAY_PORT", &port_s),
            ("GATEWAY_CERT", &cert_s),
            ("GATEWAY_KEY", &key_s),
            ("GATEWAY_SECRET", &secret),
            ("GATEWAY_ALLOWLIST", "0"),
            ("GATEWAY_VERSION", GAME_VERSION),
            ("GATEWAY_NODE_ID", me.node.as_str()),
            ("GATEWAY_NODE_REGION", me.region.as_str()),
            ("GATEWAY_NODE_CERT", &cert_s),
            ("GATEWAY_NODE_KEY", &key_s),
            ("GATEWAY_NODE_CA", &ca_s),
            ("GATEWAY_NETWORK_NODES", &peers_json),
        ]);
        let gw_cfg = dainty_gateway::config::Config::from_env(&env).unwrap();
        let loaded = dainty_gateway::cert::load(cert, key).unwrap();
        let tls = GatewayTls {
            config: dainty_gateway::cert::server_config(&loaded).unwrap(),
            info: loaded.info,
        };
        let gw = Gateway::new(gw_cfg, Some(tls), None, None).unwrap();
        gw.node_links().unwrap().endpoint().attach(service);
        let (stop, stopping) = watch::channel(false);
        let task = tokio::spawn(gw.serve(listener, stopping));
        (stop, task)
    };

    let (stop_de, task_de) = start_gateway(
        &germany_id,
        germany_port,
        &de_cert,
        &de_key,
        &de_ca,
        std::slice::from_ref(&oregon_peer),
        germany_listener,
        mux_de,
    );
    let (stop_or, task_or) = start_gateway(
        &oregon_id,
        oregon_port,
        &or_cert,
        &or_key,
        &or_ca,
        std::slice::from_ref(&germany_peer),
        oregon_listener,
        mux_or,
    );

    dir_de
        .register(dainty_store::Actor::Player(fox.to_string()), &fox, "Fox")
        .await
        .unwrap();
    dir_or.sync().await.unwrap();

    // Ban Fox on Oregon -> travels over real mTLS to Germany
    let owner = caller("owner", Role::Owner, "99");
    let banned = admin_or
        .ban(&owner, "Fox", "mTLS cluster ban", Some(GAME_VERSION))
        .await
        .unwrap();
    assert_eq!(banned.username, "Fox");
    assert!(game_de.is_banned("Fox"));
    assert!(game_or.is_banned("Fox"));
    assert!(tickets_de.is_revoked_at(&fox, 0));
    assert!(tickets_or.is_revoked_at(&fox, 0));

    let _ = stop_de.send(true);
    let _ = stop_or.send(true);
    let _ = task_de.await;
    let _ = task_or.await;
}
