//! The home API across nodes in one process (DN5): three regions on an in-process network, each with its own
//! database and directory, a stub game module recording what is written to an account, and the row's Verify: a
//! hunt hosted in another region saves loot and XP at the player's home. SQLite always; Postgres too when
//! `REFORGED_TEST_POSTGRES_URL` is set.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{Engine, Target, engines};
use dainty_module_api::network::{GlobalId, HomeApi, HomeError, IdKind, NodeId, NodeIdentity, StandaloneHome};
use dainty_module_api::{
    Actor, ActorContext, BoxFuture, ErrorKind, GameVersion, HomeBackend, HomeForward, HomeInvocation, ModuleError,
    ModuleId, OperationName,
};
use dainty_network::home::protocol::{
    HomeCall, HomeRefusal, HomeReply, HomeRequest, HomeRequestCall, RequestKey, WireActor, decode_response,
    encode_request,
};
use dainty_network::home::{
    HomeConfig, HomeRouter, HomeService, NetworkedConfig, NetworkedHome, SigningKey, TicketConfig, TicketError, Tickets,
};
use dainty_network::inprocess::{InProcessLink, InProcessNetwork};
use dainty_network::{
    Clock, Directory, DirectoryConfig, LinkError, NoPeers, NodeService, PeerLink, Service, ServiceMux,
};
use serde_json::{Value, json};

const LEADER: &str = "germany-1";
const REGIONS: [&str; 3] = ["germany-1", "oregon-1", "uk-1"];
const MODULE: &str = "rustic-isles";

fn id(name: &str) -> NodeId {
    name.parse().unwrap()
}

fn game() -> GameVersion {
    "1.14.7".parse().unwrap()
}

fn identity(name: &str) -> NodeIdentity {
    let region = name.split('-').next().unwrap();
    NodeIdentity {
        node: id(name),
        region: region.parse().unwrap(),
    }
}

fn op(name: &str) -> OperationName {
    name.parse().unwrap()
}

// ------------------------------------------------------------------ the stub game module

#[derive(Default)]
struct Account {
    xp: u64,
    items: Vec<Value>,
}

#[derive(Debug, Clone)]
struct Write {
    from: NodeId,
    operation: String,
    body: Value,
    actor: ActorContext,
}

/// What the game module does at an account's home: it owns the data, and records every write made to it.
struct Recording {
    accounts: Mutex<HashMap<GlobalId, Account>>,
    writes: Mutex<Vec<Write>>,
    runs: AtomicUsize,
    // Runs once, after the operation was applied and before it is answered
    after_apply: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    game: GameVersion,
}

impl Recording {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            accounts: Mutex::new(HashMap::new()),
            writes: Mutex::new(Vec::new()),
            runs: AtomicUsize::new(0),
            after_apply: Mutex::new(None),
            game: game(),
        })
    }

    fn xp(&self, account: &GlobalId) -> u64 {
        self.accounts.lock().unwrap().get(account).map_or(0, |a| a.xp)
    }

    fn items(&self, account: &GlobalId) -> Vec<Value> {
        self.accounts
            .lock()
            .unwrap()
            .get(account)
            .map(|a| a.items.clone())
            .unwrap_or_default()
    }

    fn writes(&self) -> Vec<Write> {
        self.writes.lock().unwrap().clone()
    }
}

impl HomeBackend for Recording {
    fn game(&self) -> &GameVersion {
        &self.game
    }

    fn run<'a>(&'a self, invocation: HomeInvocation) -> BoxFuture<'a, Result<Value, ModuleError>> {
        Box::pin(async move {
            self.runs.fetch_add(1, Ordering::SeqCst);
            let mut accounts = self.accounts.lock().unwrap();
            let account = accounts.entry(invocation.account.clone()).or_default();
            let record = |this: &Self| {
                this.writes.lock().unwrap().push(Write {
                    from: invocation.from.clone(),
                    operation: invocation.operation.to_string(),
                    body: invocation.body.clone(),
                    actor: invocation.actor.clone(),
                });
            };
            let answer = match invocation.operation.as_str() {
                "progression.grant" => {
                    account.xp += invocation.body["xp"].as_u64().unwrap_or(0);
                    record(self);
                    json!({ "xp": account.xp })
                }
                "inventory.write" => {
                    if let Some(items) = invocation.body["items"].as_array() {
                        account.items.extend(items.iter().cloned());
                    }
                    record(self);
                    json!({ "count": account.items.len() })
                }
                "account.write" => {
                    record(self);
                    json!({ "applied": true })
                }
                "progression.read" => json!({ "xp": account.xp }),
                "inventory.read" => json!({ "items": account.items }),
                "character.load" => json!({ "name": "Fox", "level": 1 + account.xp / 100 }),
                "bounty.read" => json!({ "bounties": [] }),
                "fail.me" => return Err(ModuleError::new(ErrorKind::NotFound, "no such item")),
                other => return Err(ModuleError::unsupported(format!("operation {other}"))),
            };
            drop(accounts);
            if let Some(hook) = self.after_apply.lock().unwrap().take() {
                hook();
            }
            Ok(answer)
        })
    }
}

// ------------------------------------------------------------------ the nodes

/// Every home message crossing a link, request and answer, so a test can read what was on the wire.
type Wire = Arc<Mutex<Vec<Vec<u8>>>>;

struct Tap {
    inner: InProcessLink,
    wire: Wire,
}

impl PeerLink for Tap {
    fn call<'a>(
        &'a self,
        to: &'a NodeId,
        service: Service,
        body: Vec<u8>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LinkError>> {
        Box::pin(async move {
            if service == Service::Home {
                self.wire.lock().unwrap().push(body.clone());
            }
            let answer = self.inner.call(to, service, body).await?;
            if service == Service::Home {
                self.wire.lock().unwrap().push(answer.clone());
            }
            Ok(answer)
        })
    }
}

struct Node {
    name: &'static str,
    dir: Arc<Directory>,
    tickets: Arc<Tickets>,
    home: Arc<NetworkedHome>,
    backend: Arc<Recording>,
    _mux: Arc<ServiceMux>,
    _target: Target,
}

struct Cluster {
    net: InProcessNetwork,
    clock: Arc<Clock>,
    wire: Wire,
    nodes: Vec<Node>,
}

fn locator(dir: &Arc<Directory>) -> Arc<dyn dainty_network::home::HomeLocator> {
    dir.clone()
}

impl Cluster {
    async fn new(engine: Engine) -> Self {
        let net = InProcessNetwork::new();
        let clock = Arc::new(Clock::default());
        let wire: Wire = Arc::default();
        let mut nodes = Vec::new();
        for name in REGIONS {
            let target = Target::new(engine);
            let me = identity(name);
            let dir = Arc::new(
                Directory::new(
                    target.open(),
                    DirectoryConfig::in_network(id(name), id(LEADER)).with_page(16),
                    Arc::new(net.link(&id(name))),
                )
                .await
                .unwrap(),
            );
            let tickets = Arc::new(Tickets::new(
                &me,
                SigningKey::generate().unwrap(),
                clock.clone(),
                TicketConfig::default(),
            ));
            let router = Arc::new(HomeRouter::default());
            let backend = Recording::new();
            router.register(MODULE.parse::<ModuleId>().unwrap(), backend.clone());
            let peers: Vec<NodeId> = REGIONS.iter().filter(|p| **p != name).map(|p| id(p)).collect();
            let service = HomeService::new(
                me.clone(),
                tickets.clone(),
                router.clone(),
                locator(&dir),
                peers,
                HomeConfig::default(),
            );
            let mux = Arc::new(
                ServiceMux::new(id(name))
                    .with(Service::Directory, dir.clone())
                    .with(Service::Home, Arc::new(service)),
            );
            let as_service: Arc<dyn NodeService> = mux.clone();
            net.attach(&id(name), &as_service);
            let mut config = NetworkedConfig::new(game());
            config.retry_after = Duration::from_millis(10);
            let home = Arc::new(NetworkedHome::new(
                me,
                REGIONS
                    .iter()
                    .filter(|p| **p != name)
                    .map(|p| (id(p), identity(p).region)),
                Arc::new(Tap {
                    inner: net.link(&id(name)),
                    wire: wire.clone(),
                }),
                locator(&dir),
                tickets.clone(),
                config,
            ));
            nodes.push(Node {
                name,
                dir,
                tickets,
                home,
                backend,
                _mux: mux,
                _target: target,
            });
        }
        Self {
            net,
            clock,
            wire,
            nodes,
        }
    }

    fn node(&self, name: &str) -> &Node {
        self.nodes.iter().find(|n| n.name == name).unwrap()
    }

    /// A player registered at `home`, known to every node.
    async fn player(&self, home: &str, username: &str) -> GlobalId {
        let account = GlobalId::new(IdKind::Account, &id(home));
        self.node(home)
            .dir
            .register(dainty_store::Actor::Player(account.to_string()), &account, username)
            .await
            .unwrap();
        for node in &self.nodes {
            node.dir.sync().await.unwrap();
        }
        account
    }

    /// A call from `from` to `to`'s home service as the link would deliver it: `from` is the name the link
    /// authenticated, whatever the message says.
    async fn raw(&self, from: &str, to: &str, call: HomeRequestCall) -> HomeReply {
        let bytes = encode_request(&HomeRequest::new(call)).unwrap();
        let answer = self
            .net
            .link(&id(from))
            .call(&id(to), Service::Home, bytes)
            .await
            .unwrap();
        decode_response(&answer).unwrap().reply
    }

    fn forward(&self, account: &GlobalId, operation: &str, body: Value) -> HomeForward {
        HomeForward {
            module: MODULE.parse().unwrap(),
            account: account.clone(),
            actor: ActorContext::new(Actor::Player {
                account: account.clone(),
            }),
            operation: op(operation),
            body,
            key: None,
        }
    }
}

fn denied(result: Result<impl std::fmt::Debug, HomeError>) -> String {
    match result {
        Err(HomeError::Denied(why)) => why,
        other => panic!("not a refusal: {other:?}"),
    }
}

/// Fox, homed in Germany, is admitted to a hunt hosted in Oregon: the ticket and the admission.
async fn admitted(cluster: &Cluster, fox: &GlobalId) {
    let ticket = cluster
        .node("germany-1")
        .home
        .session_ticket(fox, &id("oregon-1"))
        .await
        .unwrap();
    assert_eq!(
        cluster.node("oregon-1").home.admit(&ticket, &game()).await.unwrap(),
        *fox
    );
}

// ------------------------------------------------------------------ the row's Verify

#[tokio::test(flavor = "multi_thread")]
async fn a_hunt_hosted_in_another_region_saves_loot_and_xp_at_the_players_home() {
    for engine in engines() {
        let cluster = Cluster::new(engine).await;
        let fox = cluster.player("germany-1", "Fox").await;
        let (germany, oregon) = (cluster.node("germany-1"), cluster.node("oregon-1"));

        // Where Fox lives, as Oregon sees it
        let home = oregon.home.resolve_home(&fox).await.unwrap();
        assert_eq!(home, identity("germany-1"), "{engine:?}");

        // The hunt is Oregon's. Germany, Fox's home, hands Fox over with a ticket for Oregon
        admitted(&cluster, &fox).await;
        assert!(oregon.home.holds(&fox));

        // Oregon loads the character from Fox's home, plays, and saves the XP and the loot there
        let hunt = |operation, body| cluster.forward(&fox, operation, body);
        let loaded = oregon.home.forward(&hunt("character.load", Value::Null)).await.unwrap();
        assert_eq!(loaded, json!({ "name": "Fox", "level": 1 }));
        oregon
            .home
            .forward(&hunt("progression.grant", json!({ "xp": 250 })))
            .await
            .unwrap();
        let saved = oregon
            .home
            .forward(&hunt(
                "inventory.write",
                json!({ "items": [{ "id": "iron-sword", "count": 1 }, { "id": "gold", "count": 40 }] }),
            ))
            .await
            .unwrap();
        assert_eq!(saved, json!({ "count": 2 }));
        let envelope = oregon
            .home
            .forward(&hunt("account.write", json!({ "write": "bounty.claim", "id": 7 })))
            .await
            .unwrap();
        assert_eq!(envelope, json!({ "applied": true }));

        // It is at Fox's home, and only there
        assert_eq!(germany.backend.xp(&fox), 250, "{engine:?}");
        assert_eq!(germany.backend.items(&fox).len(), 2);
        assert_eq!(oregon.backend.xp(&fox), 0);
        assert!(oregon.backend.items(&fox).is_empty());
        assert_eq!(oregon.backend.runs.load(Ordering::SeqCst), 0);
        assert_eq!(cluster.node("uk-1").backend.runs.load(Ordering::SeqCst), 0);

        // Reads see it from either side, and the level follows the XP
        let read = oregon
            .home
            .forward(&hunt("progression.read", Value::Null))
            .await
            .unwrap();
        assert_eq!(read, json!({ "xp": 250 }));
        let level = oregon.home.forward(&hunt("character.load", Value::Null)).await.unwrap();
        assert_eq!(level["level"], 3);

        // The home's module knew who wrote, for its change log: the hunt's node and the player
        let writes = germany.backend.writes();
        assert_eq!(writes.len(), 3);
        assert!(writes.iter().all(|w| w.from == id("oregon-1")));
        assert_eq!(writes[0].actor.actor, Actor::Player { account: fox.clone() });
        assert_eq!(writes[0].operation, "progression.grant");
        assert_eq!(writes[0].body, json!({ "xp": 250 }));

        // The module's own failures come back as the module said them
        let missing = oregon.home.forward(&hunt("fail.me", Value::Null)).await;
        assert_eq!(
            missing,
            Err(HomeError::Module(ModuleError::new(ErrorKind::NotFound, "no such item")))
        );
        let unsupported = oregon.home.forward(&hunt("nothing.here", Value::Null)).await;
        assert!(matches!(unsupported, Err(HomeError::Module(e)) if e.kind == ErrorKind::Unsupported));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_node_acts_only_for_players_whose_home_gave_it_a_ticket() {
    let cluster = Cluster::new(Engine::Sqlite).await;
    let fox = cluster.player("germany-1", "Fox").await;
    let wolf = cluster.player("uk-1", "Wolf").await;
    let oregon = cluster.node("oregon-1");

    // No ticket held, no write: nothing is asked of the home
    let before = cluster.wire.lock().unwrap().len();
    let why = denied(
        oregon
            .home
            .forward(&cluster.forward(&fox, "progression.grant", json!({ "xp": 1_000_000 })))
            .await,
    );
    assert!(why.contains("no session"), "{why}");
    assert_eq!(cluster.wire.lock().unwrap().len(), before);

    // A ticket for Fox buys nothing for Wolf
    admitted(&cluster, &fox).await;
    let why = denied(
        oregon
            .home
            .forward(&cluster.forward(&wolf, "progression.grant", json!({ "xp": 1 })))
            .await,
    );
    assert!(why.contains("no session"), "{why}");
    assert_eq!(cluster.node("uk-1").backend.xp(&wolf), 0);
    assert_eq!(cluster.node("germany-1").backend.xp(&fox), 0);

    // A module the home doesn't run, and an operation on an account that isn't homed there
    let mut other = cluster.forward(&fox, "progression.grant", json!({ "xp": 1 }));
    other.module = "other-game".parse().unwrap();
    assert!(denied(oregon.home.forward(&other).await).contains("no such module"));
    // The account's home is the forwarding node itself: nowhere to forward to
    assert!(
        denied(
            cluster
                .node("germany-1")
                .home
                .forward(&cluster.forward(&fox, "progression.read", Value::Null))
                .await
        )
        .contains("account's home")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tickets_that_are_foreign_expired_replayed_or_misaddressed_are_refused() {
    let cluster = Cluster::new(Engine::Sqlite).await;
    let fox = cluster.player("germany-1", "Fox").await;
    let (germany, oregon, uk) = (
        cluster.node("germany-1"),
        cluster.node("oregon-1"),
        cluster.node("uk-1"),
    );
    let ticket = || async { germany.home.session_ticket(&fox, &id("oregon-1")).await.unwrap() };

    // Replayed: the second presentation of one ticket
    let first = ticket().await;
    oregon.home.admit(&first, &game()).await.unwrap();
    assert!(denied(oregon.home.admit(&first, &game()).await).contains("already used"));

    // Made for another node
    let for_uk = germany.home.session_ticket(&fox, &id("uk-1")).await.unwrap();
    assert!(denied(oregon.home.admit(&for_uk, &game()).await).contains("another node"));
    // ...and its audience can't be relabelled: the outer field must match the signed claims
    let relabelled = dainty_module_api::network::SessionTicket {
        audience: id("oregon-1"),
        ..for_uk.clone()
    };
    assert!(denied(oregon.home.admit(&relabelled, &game()).await).contains("not a session ticket"));
    uk.home.admit(&for_uk, &game()).await.unwrap();

    // Made for another game
    assert!(denied(oregon.home.admit(&ticket().await, &"1.12.0".parse().unwrap()).await).contains("game"));

    // Issued by a node that is not the account's home, although it is a node of the cluster
    let by_uk = uk.tickets.issue(&fox, &id("oregon-1"), &game(), None).unwrap();
    assert!(denied(oregon.home.admit(&by_uk, &game()).await).contains("home"));

    // Signed by nobody the home vouches for, naming the home as issuer
    let rogue = Tickets::new(
        &identity("germany-1"),
        SigningKey::generate().unwrap(),
        cluster.clock.clone(),
        TicketConfig::default(),
    );
    let forged = rogue.issue(&fox, &id("oregon-1"), &game(), None).unwrap();
    assert!(denied(oregon.home.admit(&forged, &game()).await).contains("key"));

    // Not a ticket at all
    let junk = dainty_module_api::network::SessionTicket {
        token: String::from("dlt1.x.y"),
        ..first.clone()
    };
    assert!(denied(oregon.home.admit(&junk, &game()).await).contains("not a session ticket"));

    // Expired
    let old = ticket().await;
    cluster.clock.skew_by(6 * 60 * 1000);
    assert!(denied(oregon.home.admit(&old, &game()).await).contains("expired"));
    cluster.clock.skew_by(0);

    // What the home itself refuses, whoever is calling (a node that took a ticket that isn't its own)
    let held = ticket().await;
    match cluster
        .raw("uk-1", "germany-1", HomeRequestCall::Check { ticket: held.clone() })
        .await
    {
        HomeReply::Refused(HomeRefusal::Ticket(TicketError::WrongAudience)) => {}
        other => panic!("{other:?}"),
    }
    let run = |ticket: &dainty_module_api::network::SessionTicket, account: &GlobalId| {
        HomeRequestCall::Run(Box::new(HomeCall {
            ticket: ticket.clone(),
            module: MODULE.parse().unwrap(),
            operation: op("progression.grant"),
            account: account.clone(),
            actor: WireActor::new(ActorContext::new(Actor::System)).unwrap(),
            key: RequestKey::generate().unwrap(),
            body: dainty_network::home::HomeBody::new(json!({ "xp": 5 })).unwrap(),
        }))
    };
    assert!(matches!(
        cluster.raw("uk-1", "germany-1", run(&held, &fox)).await,
        HomeReply::Refused(HomeRefusal::Ticket(TicketError::WrongAudience))
    ));
    // An account that is not the ticket's
    let wolf = cluster.player("uk-1", "Wolf").await;
    assert!(matches!(
        cluster.raw("oregon-1", "germany-1", run(&held, &wolf)).await,
        HomeReply::Refused(HomeRefusal::BadRequest)
    ));
    // A ticket for another game version
    let other_game = germany
        .tickets
        .issue(&fox, &id("oregon-1"), &"1.12.0".parse().unwrap(), None)
        .unwrap();
    assert!(matches!(
        cluster.raw("oregon-1", "germany-1", run(&other_game, &fox)).await,
        HomeReply::Refused(HomeRefusal::Ticket(TicketError::WrongGame))
    ));
    // A ticket the home signed for an account it is not the home of
    let foreign = germany.tickets.issue(&wolf, &id("oregon-1"), &game(), None).unwrap();
    assert!(matches!(
        cluster.raw("oregon-1", "germany-1", run(&foreign, &wolf)).await,
        HomeReply::Refused(HomeRefusal::NotHome)
    ));
    // Nothing of that reached the module
    assert_eq!(germany.backend.runs.load(Ordering::SeqCst), 0);
    assert_eq!(germany.backend.xp(&fox), 0);
    // An exchange for a node that isn't in the cluster
    assert!(matches!(
        cluster
            .raw(
                "oregon-1",
                "germany-1",
                HomeRequestCall::Exchange {
                    ticket: held.clone(),
                    audience: id("mars-1")
                }
            )
            .await,
        HomeReply::Refused(HomeRefusal::BadRequest)
    ));
    // The real thing still works
    assert!(matches!(
        cluster.raw("oregon-1", "germany-1", run(&held, &fox)).await,
        HomeReply::Done { .. }
    ));
    assert_eq!(germany.backend.xp(&fox), 5);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_the_home_ends_stops_working_everywhere() {
    let cluster = Cluster::new(Engine::Sqlite).await;
    let fox = cluster.player("germany-1", "Fox").await;
    let (germany, oregon) = (cluster.node("germany-1"), cluster.node("oregon-1"));
    admitted(&cluster, &fox).await;
    let grant = cluster.forward(&fox, "progression.grant", json!({ "xp": 10 }));
    oregon.home.forward(&grant).await.unwrap();
    oregon.home.check_session(&fox).await.unwrap();

    // Banned or logged out at home: the ticket Oregon holds is dead, and so is its renewal
    cluster.clock.skew_by(1_000);
    germany.tickets.revoke(&fox).await.unwrap();
    let why = denied(oregon.home.forward(&grant).await);
    assert!(why.contains("ended"), "{why}");
    assert_eq!(germany.backend.xp(&fox), 10);
    // Oregon dropped what it held, so a later write doesn't even ask
    assert!(!oregon.home.holds(&fox));
    let why = denied(oregon.home.check_session(&fox).await);
    assert!(why.contains("no session"), "{why}");

    // A login after that is a new session
    cluster.clock.skew_by(2_000);
    admitted(&cluster, &fox).await;
    oregon.home.forward(&grant).await.unwrap();
    assert_eq!(germany.backend.xp(&fox), 20);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_held_ticket_is_renewed_for_as_long_as_the_login_lasts() {
    let cluster = Cluster::new(Engine::Sqlite).await;
    let fox = cluster.player("germany-1", "Fox").await;
    let (germany, oregon) = (cluster.node("germany-1"), cluster.node("oregon-1"));
    admitted(&cluster, &fox).await;
    let grant = cluster.forward(&fox, "progression.grant", json!({ "xp": 1 }));
    let step = 4 * 60 * 1000;
    let mut now = 0;
    // A hunt of six hours, saving every four minutes: a ticket lives five
    while now + step <= 6 * 60 * 60 * 1000 {
        now += step;
        cluster.clock.skew_by(now);
        oregon.home.forward(&grant).await.unwrap();
    }
    assert_eq!(germany.backend.xp(&fox), 90);

    // The login is now as old as a session may be: the home renews no more, the ticket in hand lives out its five
    // minutes (the last save above used it), and then the player has to log in again
    now += step;
    cluster.clock.skew_by(now);
    let why = denied(oregon.home.forward(&grant).await);
    assert!(why.contains("expired") || why.contains("older"), "{why}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_lost_on_the_way_back_is_applied_once() {
    let cluster = Cluster::new(Engine::Sqlite).await;
    let fox = cluster.player("germany-1", "Fox").await;
    let (germany, oregon) = (cluster.node("germany-1"), cluster.node("oregon-1"));
    admitted(&cluster, &fox).await;

    // The link breaks after the home applied the grant and before the answer reached Oregon
    let net = cluster.net.clone();
    *germany.backend.after_apply.lock().unwrap() = Some(Box::new(move || {
        net.isolate(&id("germany-1"));
        let net = net.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(5)).await;
            net.heal();
        });
    }));
    let grant = cluster.forward(&fox, "progression.grant", json!({ "xp": 100 }));
    let answer = oregon.home.forward(&grant).await.unwrap();
    assert_eq!(answer, json!({ "xp": 100 }));
    assert_eq!(germany.backend.runs.load(Ordering::SeqCst), 1);
    assert_eq!(germany.backend.xp(&fox), 100);

    // The same key from the module is the same request; another key is another
    let mut keyed = cluster.forward(&fox, "progression.grant", json!({ "xp": 7 }));
    keyed.key = Some(String::from("hunt-end-0001"));
    oregon.home.forward(&keyed).await.unwrap();
    assert_eq!(oregon.home.forward(&keyed).await.unwrap(), json!({ "xp": 107 }));
    assert_eq!(germany.backend.xp(&fox), 107);
    let mut again = keyed.clone();
    again.body = json!({ "xp": 9999 });
    let why = denied(oregon.home.forward(&again).await);
    assert!(why.contains("not a request"), "{why}");
    keyed.key = Some(String::from("hunt-end-0002"));
    oregon.home.forward(&keyed).await.unwrap();
    assert_eq!(germany.backend.xp(&fox), 114);

    // A home that can't be reached is unreachable, and nothing was written
    cluster.net.cut(&id("germany-1"), &id("oregon-1"));
    let grant = cluster.forward(&fox, "progression.grant", json!({ "xp": 1 }));
    assert!(matches!(
        oregon.home.forward(&grant).await,
        Err(HomeError::Unreachable(_))
    ));
    assert_eq!(germany.backend.xp(&fox), 114);
    // Admission is offline: a ticket in hand is checked with the keys the node already has
    cluster.net.heal();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_account_key_never_crosses_a_node_link() {
    let cluster = Cluster::new(Engine::Sqlite).await;
    let fox = cluster.player("germany-1", "Fox").await;
    let oregon = cluster.node("oregon-1");
    admitted(&cluster, &fox).await;
    for operation in ["character.load", "progression.read", "inventory.read", "bounty.read"] {
        oregon
            .home
            .forward(&cluster.forward(&fox, operation, Value::Null))
            .await
            .unwrap();
    }
    oregon
        .home
        .forward(&cluster.forward(
            &fox,
            "inventory.write",
            json!({ "items": [{ "id": "gold", "count": 3 }] }),
        ))
        .await
        .unwrap();
    oregon.home.check_session(&fox).await.unwrap();
    let ticket = oregon.home.session_ticket(&fox, &id("uk-1")).await.unwrap();
    cluster.node("uk-1").home.admit(&ticket, &game()).await.unwrap();

    let sent = cluster.wire.lock().unwrap().len();
    assert!(sent >= 12, "{sent} messages on the wire");
    for message in cluster.wire.lock().unwrap().iter() {
        let text = String::from_utf8_lossy(message).to_ascii_lowercase();
        assert!(!text.contains("rrk_"), "{text}");
    }

    // A module that tries to write one is refused before anything is sent, wherever it hides it
    let key = format!("RRK_{}", "ab".repeat(24));
    for body in [
        json!({ "note": key }),
        json!({ "nested": [{ "deep": [format!("my key is {key}")] }] }),
        json!({ key.clone(): 1 }),
    ] {
        let before = cluster.wire.lock().unwrap().len();
        let result = oregon.home.forward(&cluster.forward(&fox, "account.write", body)).await;
        assert!(matches!(result, Err(HomeError::Denied(_))), "{result:?}");
        assert_eq!(cluster.wire.lock().unwrap().len(), before);
    }
    let mut keyed = cluster.forward(&fox, "account.write", Value::Null);
    keyed.key = Some(key.clone());
    assert!(matches!(oregon.home.forward(&keyed).await, Err(HomeError::Denied(_))));
    let mut actor = cluster.forward(&fox, "account.write", Value::Null);
    actor.actor = ActorContext::new(Actor::System).with_request(key);
    assert!(matches!(oregon.home.forward(&actor).await, Err(HomeError::Denied(_))));
    assert_eq!(cluster.node("germany-1").backend.writes().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn keys_are_fetched_from_the_home_and_a_rotation_is_followed() {
    let cluster = Cluster::new(Engine::Sqlite).await;
    let fox = cluster.player("germany-1", "Fox").await;
    let (germany, oregon) = (cluster.node("germany-1"), cluster.node("oregon-1"));
    assert!(!oregon.tickets.knows_keys_of(&id("germany-1")));
    admitted(&cluster, &fox).await;
    assert!(oregon.tickets.knows_keys_of(&id("germany-1")));

    // Germany rotates its key. Oregon sees a key id it doesn't know and asks, but not more often than it may
    germany.tickets.rotate(SigningKey::generate().unwrap());
    let ticket = germany.home.session_ticket(&fox, &id("oregon-1")).await.unwrap();
    let why = denied(oregon.home.admit(&ticket, &game()).await);
    assert!(why.contains("key"), "{why}");
    cluster.clock.skew_by(11_000);
    let ticket = germany.home.session_ticket(&fox, &id("oregon-1")).await.unwrap();
    assert_eq!(oregon.home.admit(&ticket, &game()).await.unwrap(), fox);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_driver_backend_runs_operations_through_a_modules_command() {
    use dainty_module_api::stub::{RecordingHost, StubModule};
    use dainty_module_api::{InProcessModule, ModuleDriver};

    let host = Arc::new(RecordingHost::new(NodeIdentity::standalone()));
    let module = InProcessModule::new(Arc::new(StubModule::new()), NodeIdentity::standalone(), host).unwrap();
    let driver: Arc<dyn ModuleDriver> = Arc::new(module);
    let backend = dainty_network::home::DriverBackend::new(driver);
    assert_eq!(backend.game().to_string(), "0.0.1");
    let fox = GlobalId::new(IdKind::Account, &id("germany-1"));
    let answer = backend
        .run(HomeInvocation {
            from: id("oregon-1"),
            account: fox.clone(),
            actor: ActorContext::new(Actor::System),
            operation: op("inventory.read"),
            body: json!({ "a": 1 }),
        })
        .await
        .unwrap();
    assert_eq!(answer["from"], "oregon-1");
    assert_eq!(answer["operation"], "inventory.read");
    assert_eq!(answer["account"], fox.to_string());
    assert_eq!(answer["body"], json!({ "a": 1 }));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mux_answers_each_service_and_refuses_the_others() {
    let cluster = Cluster::new(Engine::Sqlite).await;
    // A node with a directory and no home service
    let bare = ServiceMux::new(id("uk-1"));
    let bare: Arc<dyn NodeService> = Arc::new(bare);
    let alone = InProcessNetwork::new();
    alone.attach(&id("uk-1"), &bare);
    let bytes = encode_request(&HomeRequest::new(HomeRequestCall::PublicKeys)).unwrap();
    let result = alone
        .link(&id("oregon-1"))
        .call(&id("uk-1"), Service::Home, bytes.clone())
        .await;
    assert!(matches!(
        result,
        Err(LinkError::NoService {
            service: Service::Home,
            ..
        })
    ));
    // The cluster's nodes answer it
    assert!(matches!(
        cluster.raw("oregon-1", "germany-1", HomeRequestCall::PublicKeys).await,
        HomeReply::PublicKeys { .. }
    ));
    // ...and a message that isn't one is refused as such, a newer version as that
    let net = cluster.net.link(&id("oregon-1"));
    let answer = net.call(&id("germany-1"), Service::Home, b"{}".to_vec()).await.unwrap();
    assert_eq!(
        decode_response(&answer).unwrap().reply,
        HomeReply::Refused(HomeRefusal::Malformed)
    );
    let answer = net
        .call(
            &id("germany-1"),
            Service::Home,
            br#"{"version":9,"call":"public_keys"}"#.to_vec(),
        )
        .await
        .unwrap();
    assert_eq!(
        decode_response(&answer).unwrap().reply,
        HomeReply::Refused(HomeRefusal::UnsupportedVersion { supported: 1 })
    );
    let huge = vec![b' '; dainty_network::link::MAX_MESSAGE + 1];
    assert_eq!(
        net.call(&id("germany-1"), Service::Home, huge).await,
        Err(LinkError::TooLarge)
    );
}

// ------------------------------------------------------------------ outside a network

#[tokio::test(flavor = "multi_thread")]
async fn a_server_outside_a_network_has_no_other_home() {
    // The standalone home refuses the new calls the way it refuses tickets
    let standalone = StandaloneHome {
        identity: NodeIdentity::standalone(),
    };
    let local = GlobalId::new(IdKind::Account, &standalone.identity.node);
    let call = HomeForward {
        module: MODULE.parse().unwrap(),
        account: local.clone(),
        actor: ActorContext::new(Actor::System),
        operation: op("account.write"),
        body: Value::Null,
        key: None,
    };
    assert_eq!(standalone.forward(&call).await, Err(HomeError::NotInNetwork));
    let ticket = dainty_module_api::network::SessionTicket {
        account: local.clone(),
        audience: standalone.identity.node.clone(),
        token: String::new(),
    };
    assert_eq!(standalone.admit(&ticket, &game()).await, Err(HomeError::NotInNetwork));

    // And a networked home with no peers works as the node it is: its own accounts, its own tickets
    let target = Target::new(Engine::Sqlite);
    let me = NodeIdentity::standalone();
    let dir = Arc::new(
        Directory::new(
            target.open(),
            DirectoryConfig::standalone(me.node.clone()),
            Arc::new(NoPeers),
        )
        .await
        .unwrap(),
    );
    dir.register(dainty_store::Actor::Player(local.to_string()), &local, "Fox")
        .await
        .unwrap();
    let tickets = Arc::new(Tickets::new(
        &me,
        SigningKey::generate().unwrap(),
        Arc::new(Clock::default()),
        TicketConfig::default(),
    ));
    let home = NetworkedHome::new(
        me.clone(),
        std::iter::empty(),
        Arc::new(NoPeers),
        locator(&dir),
        tickets,
        NetworkedConfig::new(game()),
    );
    assert_eq!(home.resolve_home(&local).await.unwrap(), me);
    let own = home.session_ticket(&local, &me.node).await.unwrap();
    assert_eq!(home.admit(&own, &game()).await.unwrap(), local);
    assert!(denied(home.admit(&own, &game()).await).contains("already used"));
    // An account the directory has never heard of has no home; a foreign one has nothing to reach
    let stranger = GlobalId::new(IdKind::Account, &id("oregon-1"));
    assert_eq!(home.resolve_home(&stranger).await, Err(HomeError::UnknownAccount));
    let mut foreign = call.clone();
    foreign.account = stranger;
    assert_eq!(home.forward(&foreign).await, Err(HomeError::UnknownAccount));
    // Its own account is its own home
    assert!(denied(home.forward(&call).await).contains("account's home"));
}

#[tokio::test(flavor = "multi_thread")]
async fn ban_revocation_survives_restart_on_sqlite_and_postgres() {
    for engine in engines() {
        let target = Target::new(engine);
        let db = target.open();
        let key_dir = tempfile::tempdir().unwrap();
        let key_path = key_dir.path().join("ticket.key");

        let home_identity = NodeIdentity {
            node: id("germany-1"),
            region: "eu-central".parse().unwrap(),
        };
        let audience_node = id("oregon-1");
        let clock = Arc::new(Clock::default());
        let account = GlobalId::new(IdKind::Account, &home_identity.node);
        let game_version = game();

        let tickets_before = Tickets::with_database(
            &home_identity,
            SigningKey::load_or_create(&key_path).unwrap(),
            clock.clone(),
            TicketConfig::default(),
            db.clone(),
        )
        .await
        .unwrap();

        let expect = dainty_network::home::Expect {
            issuer: &home_identity.node,
            audience: &audience_node,
            game: Some(&game_version),
            account: Some(&account),
        };

        // Issue a ticket before the ban
        let old_ticket = tickets_before
            .issue(&account, &audience_node, &game_version, None)
            .unwrap();
        let old_claims = tickets_before.verify(&old_ticket, &expect).unwrap();

        // Advance clock by 1s and ban (revoke) the account
        clock.skew_by(1_000);
        tickets_before.revoke(&account).await.unwrap();
        assert_eq!(tickets_before.verify(&old_ticket, &expect), Err(TicketError::Revoked));
        drop(tickets_before);

        // Restart Tickets on the same database with the same signing key
        clock.skew_by(2_000);
        let tickets_after = Tickets::with_database(
            &home_identity,
            SigningKey::load_or_create(&key_path).unwrap(),
            clock.clone(),
            TicketConfig::default(),
            db.clone(),
        )
        .await
        .unwrap();

        // The old ticket issued before the ban is still refused after restart, and its session cannot be renewed
        assert_eq!(tickets_after.verify(&old_ticket, &expect), Err(TicketError::Revoked));
        assert_eq!(
            tickets_after.issue(&account, &audience_node, &game_version, Some(old_claims.since)),
            Err(TicketError::Revoked)
        );

        // Even 20 minutes later (past MAX_TTL, within DEFAULT_MAX_SESSION), renewing the pre-ban session is refused
        clock.skew_by(20 * 60_000);
        assert_eq!(
            tickets_after.issue(&account, &audience_node, &game_version, Some(old_claims.since)),
            Err(TicketError::Revoked)
        );

        // A fresh login after the ban succeeds
        let fresh_ticket = tickets_after
            .issue(&account, &audience_node, &game_version, None)
            .unwrap();
        tickets_after.verify(&fresh_ticket, &expect).unwrap();

        // Unbanning (`allow`) clears the persisted floor across another restart
        tickets_after.allow(&account).await.unwrap();
        drop(tickets_after);

        let tickets_unbanned = Tickets::with_database(
            &home_identity,
            SigningKey::load_or_create(&key_path).unwrap(),
            clock.clone(),
            TicketConfig::default(),
            db,
        )
        .await
        .unwrap();
        assert!(!tickets_unbanned.is_revoked_at(&account, old_claims.since));
    }
}

#[tokio::test]
async fn revocation_write_failure_is_surfaced_when_database_table_is_missing() {
    use dainty_store::Actor;
    use diesel::RunQueryDsl;

    let clock = Arc::new(Clock::default());
    let home_identity = NodeIdentity {
        node: id("germany-1"),
        region: "eu-central".parse().unwrap(),
    };
    let account = GlobalId::new(IdKind::Account, &home_identity.node);
    let key_dir = tempfile::tempdir().unwrap();
    let key_path = key_dir.path().join("ticket.key");
    let target = Target::new(Engine::Sqlite);
    let db = target.open();

    let tickets = Tickets::with_database(
        &home_identity,
        SigningKey::load_or_create(&key_path).unwrap(),
        clock,
        TicketConfig::default(),
        db.clone(),
    )
    .await
    .unwrap();

    // Drop the revocation table to force the persistence write to fail
    db.write::<_, dainty_store::Error, _>(Actor::System("test-break-table".to_string()), |tx| {
        diesel::sql_query("DROP TABLE network_ticket_revocations")
            .execute(tx.conn())
            .map(|_| ())
            .map_err(dainty_store::Error::from)
    })
    .await
    .unwrap();

    let err = tickets
        .revoke(&account)
        .await
        .expect_err("revoke must return an error when database persistence fails");
    assert_eq!(err.account, account);
    assert!(!err.to_string().is_empty());

    let allow_err = tickets
        .allow(&account)
        .await
        .expect_err("allow must return an error when database persistence fails");
    assert_eq!(allow_err.account, account);
}

#[tokio::test(flavor = "current_thread")]
async fn revocation_inside_current_thread_runtime_completes_without_deadlock() {
    let target = Target::new(Engine::Sqlite);
    let db = target.open();
    let key_dir = tempfile::tempdir().unwrap();
    let key_path = key_dir.path().join("ticket.key");
    let home_identity = NodeIdentity {
        node: id("germany-1"),
        region: "eu-central".parse().unwrap(),
    };
    let audience_node = id("oregon-1");
    let clock = Arc::new(Clock::default());
    let account = GlobalId::new(IdKind::Account, &home_identity.node);
    let game_version = game();

    let tickets = Tickets::with_database(
        &home_identity,
        SigningKey::load_or_create(&key_path).unwrap(),
        clock.clone(),
        TicketConfig::default(),
        db.clone(),
    )
    .await
    .unwrap();

    let expect = dainty_network::home::Expect {
        issuer: &home_identity.node,
        audience: &audience_node,
        game: Some(&game_version),
        account: Some(&account),
    };

    let ticket = tickets.issue(&account, &audience_node, &game_version, None).unwrap();
    let claims = tickets.verify(&ticket, &expect).unwrap();

    clock.skew_by(1_000);
    tickets.revoke(&account).await.unwrap();
    assert_eq!(tickets.verify(&ticket, &expect), Err(TicketError::Revoked));

    let reloaded = Tickets::with_database(
        &home_identity,
        SigningKey::load_or_create(&key_path).unwrap(),
        clock.clone(),
        TicketConfig::default(),
        db,
    )
    .await
    .unwrap();
    assert!(reloaded.is_revoked_at(&account, claims.since));

    reloaded.allow(&account).await.unwrap();
    assert!(!reloaded.is_revoked_at(&account, claims.since));
}
