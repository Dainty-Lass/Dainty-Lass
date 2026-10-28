#![cfg(feature = "network")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! End-to-end wiring of [`controller::modules::network`] and [`controller::modules::gateway`] across two
//! [`controller::ServerController`] nodes over loopback mTLS (`/__node/<service>`):
//! - `NodeServices` is bound to each gateway's `/__node/*` endpoint
//! - `Directory` replicates registrations from the leader (`oregon-1`) to a replica (`uk-1`)
//! - `HomeRouterHook` registers loaded game modules into `HomeRouter`
//! - `NetworkedHome` (`registry.home()`) resolves an account's home, issues and admits session tickets, and
//!   executes a module's home operation on the remote home node

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use controller::modules::{self, NetworkOptions};
use controller::{ModuleEntry, ModuleRegistry, ModuleSource, RegistryOptions, ServerController};
use module_api::network::{GlobalId, IdKind, NodeId};
use module_api::stub::StubModule;
use module_api::{ActorContext, GameModule, GameVersion, HomeForward, NodeIdentity};
use network_plugin::cluster::{ClusterCa, NodeCertRequest, NodeCredentials, Peer};
use network_plugin::home::SigningKey;
use service_kit::env::Env;
use store::{Actor, DatabaseConfig};

fn free_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct TestNode {
    controller: ServerController,
    registry: ModuleRegistry,
    _dir: tempfile::TempDir,
}

fn build_node(ca: &ClusterCa, node: &str, region: &str, leader: &str, port: u16, peers: Vec<Peer>) -> TestNode {
    let dir = tempfile::tempdir().unwrap();
    let node_id: NodeId = node.parse().unwrap();
    let region_id = region.parse().unwrap();
    let identity = NodeIdentity {
        node: node_id.clone(),
        region: region_id,
    };
    let issued = ca
        .issue(&NodeCertRequest {
            node: identity.node.clone(),
            region: identity.region.clone(),
            hosts: vec!["127.0.0.1".into()],
            days: 30,
        })
        .unwrap();
    let cert_path = dir.path().join("node-cert.pem");
    let key_path = dir.path().join("node-key.pem");
    let ca_path = dir.path().join("cluster-ca-cert.pem");
    std::fs::write(&cert_path, &issued.cert_pem).unwrap();
    std::fs::write(&key_path, &issued.key_pem).unwrap();
    std::fs::write(&ca_path, ca.certificate_pem()).unwrap();

    let credentials = NodeCredentials::load(&cert_path, &key_path, &ca_path).unwrap();
    let signing_key = SigningKey::load_or_create(&dir.path().join("session-key")).unwrap();
    let database = DatabaseConfig::Sqlite {
        file: dir.path().join("dainty.sqlite"),
    };

    let mut controller = ServerController::new("public", vec!["0.0.1".into()]);
    let mut reg_options = RegistryOptions::standalone();
    reg_options.node = identity.clone();
    let registry = ModuleRegistry::new(reg_options, controller.status_registry().clone());
    registry
        .add_entry(ModuleEntry {
            id: "stub".parse().unwrap(),
            source: ModuleSource::CompiledIn(Arc::new(|| Arc::new(StubModule::new()) as Arc<dyn GameModule>)),
            autoload: true,
            settings: BTreeMap::new(),
        })
        .unwrap();
    controller.set_registry(registry.clone());

    let peers_json = serde_json::to_string(&peers).unwrap();
    let cert_str = cert_path.to_string_lossy().into_owned();
    let key_str = key_path.to_string_lossy().into_owned();
    let ca_str = ca_path.to_string_lossy().into_owned();
    let port_str = port.to_string();
    let secret = "a".repeat(64);
    let gw_env = Env::from_pairs(vec![
        ("GATEWAY_BIND", "127.0.0.1"),
        ("GATEWAY_PORT", &port_str),
        ("GATEWAY_CERT", &cert_str),
        ("GATEWAY_KEY", &key_str),
        ("GATEWAY_SECRET", &secret),
        ("GATEWAY_ALLOWLIST", "0"),
        ("GATEWAY_VERSION", "0.0.1"),
        ("GATEWAY_NODE_ID", node),
        ("GATEWAY_NODE_REGION", region),
        ("GATEWAY_NODE_CERT", &cert_str),
        ("GATEWAY_NODE_KEY", &key_str),
        ("GATEWAY_NODE_CA", &ca_str),
        ("GATEWAY_NETWORK_NODES", &peers_json),
    ]);

    let peer_log_dir = dir.path().join("peer-logs");
    let ship_state = dir.path().join("history-peer-state.json");
    let ship_to_peer = peers.first().map(|p| (p.id.clone(), ship_state));
    controller.add(
        modules::network(NetworkOptions {
            node: identity,
            leader: leader.parse().unwrap(),
            peers,
            credentials,
            signing_key,
            database,
            game: "0.0.1".parse().unwrap(),
            registry: registry.clone(),
            replication_interval: Duration::from_millis(100),
            peer_log_dir: Some(peer_log_dir),
            ship_to_peer,
        })
        .unwrap(),
    );
    controller.add(modules::gateway(&gw_env, None, Some(registry.clone())).unwrap());

    TestNode {
        controller,
        registry,
        _dir: dir,
    }
}

#[tokio::test]
async fn two_composed_controllers_replicate_directory_and_route_home_calls() {
    let ca = ClusterCa::create(30).unwrap();
    let port1 = free_port();
    let port2 = free_port();

    let peer1 = Peer {
        id: "oregon-1".parse().unwrap(),
        region: "oregon".parse().unwrap(),
        host: "127.0.0.1".into(),
        port: port1,
        fingerprint: None,
    };
    let peer2 = Peer {
        id: "uk-1".parse().unwrap(),
        region: "uk".parse().unwrap(),
        host: "127.0.0.1".into(),
        port: port2,
        fingerprint: None,
    };

    let mut node1 = build_node(&ca, "oregon-1", "oregon", "oregon-1", port1, vec![peer2]);
    let mut node2 = build_node(&ca, "uk-1", "uk", "oregon-1", port2, vec![peer1]);

    node1.controller.start_all().await.unwrap();
    node2.controller.start_all().await.unwrap();

    let dir1 = node1.registry.node_services().directory().unwrap();
    let dir2 = node2.registry.node_services().directory().unwrap();

    let account = GlobalId::new(IdKind::Account, &"oregon-1".parse().unwrap());
    dir1.register(Actor::System("test".into()), &account, "slayer_one")
        .await
        .unwrap();

    // Replica pulls the registration from the leader over the gateway's mTLS node link
    // (or the background 100ms replication loop has already pulled it)
    let pulled = dir2.sync().await.unwrap();
    assert!(pulled <= 1);
    assert_eq!(dir2.home_of(&account).await.unwrap(), Some("oregon-1".parse().unwrap()));

    // Node 2's HomeApi (NetworkedHome) resolves the account's home to oregon-1 in oregon
    let home = node2.registry.home().resolve_home(&account).await.unwrap();
    assert_eq!(home.node.as_str(), "oregon-1");
    assert_eq!(home.region.as_str(), "oregon");

    // Issue a session ticket on node 1 for node 2, admit it on node 2, and forward a home call
    // to the loaded stub game module on node 1
    let ticket = node1
        .registry
        .home()
        .session_ticket(&account, &"uk-1".parse().unwrap())
        .await
        .unwrap();
    let game: GameVersion = "0.0.1".parse().unwrap();
    let admitted = node2.registry.home().admit(&ticket, &game).await.unwrap();
    assert_eq!(admitted, account);

    let answer = node2
        .registry
        .home()
        .forward(&HomeForward {
            module: "stub".parse().unwrap(),
            account: account.clone(),
            actor: ActorContext::new(module_api::Actor::System),
            operation: "ping".parse().unwrap(),
            key: None,
            body: serde_json::json!({ "hello": "world" }),
        })
        .await
        .unwrap();
    assert_eq!(answer["from"], "uk-1");
    assert_eq!(answer["operation"], "ping");
    assert_eq!(answer["body"]["hello"], "world");

    assert!(
        node2
            .registry
            .node_services()
            .is_attached(network_plugin::Service::LogShip)
    );
    // Wait briefly for oregon-1's background peer log shipper to push its snapshot and log segment to uk-1 over mTLS
    let uk_oregon_logs = node2._dir.path().join("peer-logs").join("oregon-1").join("log");
    let uk_oregon_snaps = node2._dir.path().join("peer-logs").join("oregon-1").join("snapshot");
    let mut shipped_over_mtls = false;
    for _ in 0..30 {
        let has_log = std::fs::read_dir(&uk_oregon_logs)
            .map(|mut d| d.any(|e| e.ok().is_some_and(|f| f.path().extension().is_some_and(|x| x == "gz"))))
            .unwrap_or(false);
        let has_snap = std::fs::read_dir(&uk_oregon_snaps)
            .map(|mut d| d.any(|e| e.ok().is_some_and(|f| f.path().extension().is_some_and(|x| x == "gz"))))
            .unwrap_or(false);
        if has_log && has_snap {
            shipped_over_mtls = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        shipped_over_mtls,
        "expected oregon-1 to ship snapshot and log segment to uk-1 over mTLS"
    );

    node2.controller.shutdown().await;
    node1.controller.shutdown().await;

    assert!(
        !node1
            .registry
            .node_services()
            .is_attached(network_plugin::Service::Directory)
    );
    assert!(
        !node1
            .registry
            .node_services()
            .is_attached(network_plugin::Service::Home)
    );
    assert!(
        !node1
            .registry
            .node_services()
            .is_attached(network_plugin::Service::LogShip)
    );
}
