// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

//! The home API over the real node link (DN5): two nodes on 127.0.0.1:0, each a gateway, call each other through
//! `TlsPeerLink` over mutual TLS under one cluster CA. The home hands the player's ticket to the hunt's node, the
//! node saves loot and XP at the home through the link, and the same refusals hold as in process
//! (`crates/network/tests/home.rs` has the cases; this is the end-to-end of the transport).

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use gateway::gateway::{Gateway, Tls as GatewayTls};
use module_api::network::{GlobalId, HomeApi, HomeError, IdKind, NodeId, NodeIdentity};
use module_api::{
    Actor, ActorContext, BoxFuture, GameVersion, HomeBackend, HomeForward, HomeInvocation, ModuleError, ModuleId,
};
use network_plugin::cluster::{ClusterCa, NodeCertRequest, NodeCredentials, Peer};
use network_plugin::home::{
    HomeConfig, HomeLocator, HomeRouter, HomeService, NetworkedConfig, NetworkedHome, SigningKey, TicketConfig, Tickets,
};
use network_plugin::{Clock, NodeService, TlsLinkConfig, TlsPeerLink};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::watch;

fn id(text: &str) -> NodeId {
    text.parse().unwrap()
}

fn game() -> GameVersion {
    "1.14.7".parse().unwrap()
}

/// Every account is homed on the node that owns its id, which is what the directory says once it has registered it.
struct OwnerIsHome;

impl HomeLocator for OwnerIsHome {
    fn home_of<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<Option<NodeId>, String>> {
        Box::pin(async move { Ok(Some(account.owner().clone())) })
    }
}

/// The game module at the home: XP and loot per account, and who wrote them.
#[derive(Default)]
struct Saves {
    xp: Mutex<HashMap<GlobalId, u64>>,
    items: Mutex<HashMap<GlobalId, Vec<Value>>>,
    callers: Mutex<Vec<NodeId>>,
    game: Option<GameVersion>,
}

impl HomeBackend for Saves {
    fn game(&self) -> &GameVersion {
        self.game.as_ref().unwrap()
    }

    fn run<'a>(&'a self, invocation: HomeInvocation) -> BoxFuture<'a, Result<Value, ModuleError>> {
        Box::pin(async move {
            self.callers.lock().unwrap().push(invocation.from.clone());
            match invocation.operation.as_str() {
                "progression.grant" => {
                    let mut xp = self.xp.lock().unwrap();
                    let total = xp.entry(invocation.account).or_default();
                    *total += invocation.body["xp"].as_u64().unwrap_or(0);
                    Ok(json!({ "xp": *total }))
                }
                "inventory.write" => {
                    let mut items = self.items.lock().unwrap();
                    let held = items.entry(invocation.account).or_default();
                    held.extend(invocation.body["items"].as_array().cloned().unwrap_or_default());
                    Ok(json!({ "count": held.len() }))
                }
                other => Err(ModuleError::unsupported(other)),
            }
        })
    }
}

struct Cluster {
    ca: ClusterCa,
    dir: tempfile::TempDir,
}

struct Files {
    cert: String,
    key: String,
    ca: String,
}

impl Cluster {
    fn files(&self, node: &str, region: &str) -> Files {
        let issued = self
            .ca
            .issue(&NodeCertRequest {
                node: id(node),
                region: region.parse().unwrap(),
                hosts: vec!["127.0.0.1".into()],
                days: 30,
            })
            .unwrap();
        let path = |name: &str| self.dir.path().join(format!("{node}-{name}.pem"));
        let write = |name: &str, text: &str| {
            std::fs::write(path(name), text).unwrap();
            path(name).to_string_lossy().into_owned()
        };
        Files {
            cert: write("cert", &issued.cert_pem),
            key: write("key", &issued.key_pem),
            ca: write("ca", self.ca.certificate_pem()),
        }
    }
}

fn peer(id_text: &str, region: &str, port: u16) -> Peer {
    Peer {
        id: id(id_text),
        region: region.parse().unwrap(),
        host: "127.0.0.1".into(),
        port,
        fingerprint: None,
        link: None,
    }
}

struct Running {
    gateway: Arc<Gateway>,
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
    _public: TestCert,
}

impl Running {
    async fn close(self) {
        let _ = self.stop.send(true);
        let _ = self.task.await;
    }
}

async fn start_node(files: &Files, (node, region): (&str, &str), listener: TcpListener, peers: &[Peer]) -> Running {
    let public = make_cert();
    let ports = Ports {
        backend: dead_port().await,
        content: dead_port().await,
        ws: dead_port().await,
    };
    let nodes = serde_json::to_string(peers).unwrap();
    let config = test_config(
        &public,
        &ports,
        &[
            ("GATEWAY_NODE_ID", node),
            ("GATEWAY_NODE_REGION", region),
            ("GATEWAY_NODE_CERT", &files.cert),
            ("GATEWAY_NODE_KEY", &files.key),
            ("GATEWAY_NODE_CA", &files.ca),
            ("GATEWAY_NETWORK_NODES", &nodes),
        ],
    );
    let loaded = gateway::cert::load(&public.cert_file, &public.key_file).unwrap();
    let tls = GatewayTls {
        config: gateway::cert::server_config(&loaded).unwrap(),
        info: loaded.info,
    };
    let gateway = Gateway::new(config, Some(tls), None, None).unwrap();
    let (stop, stopping) = watch::channel(false);
    let task = tokio::spawn(gateway.clone().serve(listener, stopping));
    Running {
        gateway,
        stop,
        task,
        _public: public,
    }
}

fn link_to(creds: NodeCredentials, peers: Vec<Peer>) -> Arc<TlsPeerLink> {
    let mut config = TlsLinkConfig::new(creds, peers);
    config.connect_timeout = Duration::from_secs(3);
    config.request_timeout = Duration::from_secs(5);
    Arc::new(TlsPeerLink::new(config).unwrap())
}

/// One node: its gateway answering the home service, and the home API it asks the other through.
struct Node {
    running: Running,
    home: NetworkedHome,
    saves: Arc<Saves>,
}

fn node_pieces(
    name: &str,
    other: &str,
    link: Arc<TlsPeerLink>,
) -> (NetworkedHome, HomeService, Arc<Saves>, Arc<Tickets>) {
    let me = NodeIdentity {
        node: id(name),
        region: name.split('-').next().unwrap().parse().unwrap(),
    };
    let tickets = Arc::new(Tickets::new(
        &me,
        SigningKey::generate().unwrap(),
        Arc::new(Clock::default()),
        TicketConfig::default(),
    ));
    let saves = Arc::new(Saves {
        game: Some(game()),
        ..Saves::default()
    });
    let router = Arc::new(HomeRouter::default());
    router.register("rustic-isles".parse::<ModuleId>().unwrap(), saves.clone());
    let locator: Arc<dyn HomeLocator> = Arc::new(OwnerIsHome);
    let service = HomeService::new(
        me.clone(),
        tickets.clone(),
        router,
        locator.clone(),
        [id(other)],
        HomeConfig::default(),
    );
    let other_region = other.split('-').next().unwrap().parse().unwrap();
    let home = NetworkedHome::new(
        me,
        [(id(other), other_region)],
        link,
        locator,
        tickets.clone(),
        NetworkedConfig::new(game()),
    );
    (home, service, saves, tickets)
}

#[tokio::test]
async fn a_hunt_in_another_region_saves_at_the_players_home_over_mutual_tls() {
    let cluster = Cluster {
        ca: ClusterCa::create(30).unwrap(),
        dir: tempfile::tempdir().unwrap(),
    };
    let (germany_listener, oregon_listener) = (
        TcpListener::bind("127.0.0.1:0").await.unwrap(),
        TcpListener::bind("127.0.0.1:0").await.unwrap(),
    );
    let ports = (
        germany_listener.local_addr().unwrap().port(),
        oregon_listener.local_addr().unwrap().port(),
    );
    let (germany_files, oregon_files) = (
        cluster.files("germany-1", "germany"),
        cluster.files("oregon-1", "oregon"),
    );
    let (to_oregon, to_germany) = (
        peer("oregon-1", "oregon", ports.1),
        peer("germany-1", "germany", ports.0),
    );
    let load =
        |files: &Files| NodeCredentials::load(files.cert.as_ref(), files.key.as_ref(), files.ca.as_ref()).unwrap();
    let germany_link = link_to(load(&germany_files), vec![to_oregon.clone()]);
    let oregon_link = link_to(load(&oregon_files), vec![to_germany.clone()]);

    let (germany_home, germany_service, germany_saves, _) = node_pieces("germany-1", "oregon-1", germany_link);
    let (oregon_home, oregon_service, oregon_saves, _) = node_pieces("oregon-1", "germany-1", oregon_link);
    let germany_running = start_node(&germany_files, ("germany-1", "germany"), germany_listener, &[to_oregon]).await;
    let oregon_running = start_node(&oregon_files, ("oregon-1", "oregon"), oregon_listener, &[to_germany]).await;
    let attach = |running: &Running, service: HomeService| {
        let service: Arc<dyn NodeService> = Arc::new(service);
        running.gateway.node_links().unwrap().endpoint().attach(service);
    };
    attach(&germany_running, germany_service);
    attach(&oregon_running, oregon_service);
    let germany = Node {
        running: germany_running,
        home: germany_home,
        saves: germany_saves,
    };
    let oregon = Node {
        running: oregon_running,
        home: oregon_home,
        saves: oregon_saves,
    };

    // Fox lives in Germany; the hunt is Oregon's
    let fox = GlobalId::new(IdKind::Account, &id("germany-1"));
    assert_eq!(oregon.home.resolve_home(&fox).await.unwrap().node, id("germany-1"));
    let ticket = germany.home.session_ticket(&fox, &id("oregon-1")).await.unwrap();
    // Oregon checks the ticket against the key Germany tells it over the link
    assert_eq!(oregon.home.admit(&ticket, &game()).await.unwrap(), fox);
    assert!(matches!(
        oregon.home.admit(&ticket, &game()).await,
        Err(HomeError::Denied(why)) if why.contains("already used")
    ));

    let save = |operation: &str, body: Value| HomeForward {
        module: "rustic-isles".parse().unwrap(),
        account: fox.clone(),
        actor: ActorContext::new(Actor::Player { account: fox.clone() }),
        operation: operation.parse().unwrap(),
        body,
        key: None,
    };
    assert_eq!(
        oregon
            .home
            .forward(&save("progression.grant", json!({ "xp": 300 })))
            .await
            .unwrap(),
        json!({ "xp": 300 })
    );
    assert_eq!(
        oregon
            .home
            .forward(&save("inventory.write", json!({ "items": [{ "id": "ember-blade" }] })))
            .await
            .unwrap(),
        json!({ "count": 1 })
    );
    assert_eq!(germany.saves.xp.lock().unwrap().get(&fox), Some(&300));
    assert_eq!(germany.saves.items.lock().unwrap().get(&fox).map(Vec::len), Some(1));
    assert_eq!(*germany.saves.callers.lock().unwrap(), [id("oregon-1"), id("oregon-1")]);
    assert!(oregon.saves.xp.lock().unwrap().is_empty());
    assert!(oregon.saves.items.lock().unwrap().is_empty());

    // The home hands the player on to a node it lists, with the ticket Oregon holds; one it doesn't list gets none
    assert_eq!(
        oregon
            .home
            .session_ticket(&fox, &id("oregon-1"))
            .await
            .map(|t| t.audience),
        Ok(id("oregon-1"))
    );
    assert!(matches!(
        oregon.home.session_ticket(&fox, &id("mars-1")).await,
        Err(HomeError::Denied(_))
    ));
    // A module the home does not run, and an operation it does not know, are the module's answers
    let mut other = save("progression.grant", Value::Null);
    other.module = "other-game".parse().unwrap();
    assert!(matches!(oregon.home.forward(&other).await, Err(HomeError::Denied(_))));
    assert!(matches!(
        oregon.home.forward(&save("nothing.here", Value::Null)).await,
        Err(HomeError::Module(_))
    ));

    germany.running.close().await;
    oregon.running.close().await;
}
