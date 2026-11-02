//! The gateway library end to end, as the core runs it (RD27): a peer's node-link call is admitted by the library's
//! endpoint and answered by the core's node services, a body over one frame goes both ways, a peer the core adds is
//! let in, the core's enroll handler answers `POST /__node/enroll`, a loaded module's UDP range joins the in-process
//! allowlist's rule, and a player the ring admits is let through it. `cargo build -p gateway-plugin` first.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

#[path = "../../gateway/tests/common/mod.rs"]
mod common;
mod support;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use controller::service_library::gateway::{AllowlistMode, GatewayOptions, gateway_library};
use controller::{ModuleEntry, ModuleRegistry, ModuleSource, RegistryOptions};
use module_api::network::{GlobalId, IdKind, NodeId, NodeIdentity};
use module_api::stub::StubModule;
use module_api::{BoxFuture, GameModule};
use network_plugin::cluster::{ClusterCa, NodeCertRequest, NodeCredentials, Peer};
use network_plugin::{LinkError, NodeService, PeerLink, Service, TlsLinkConfig, TlsPeerLink};
use serde_json::Value;
use service_kit::context::StatusRegistry;
use service_kit::env::Env;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn id(text: &str) -> NodeId {
    text.parse().unwrap()
}

/// Answers with who called and what they said.
struct Echo;

impl NodeService for Echo {
    fn serve<'a>(&'a self, from: &'a NodeId, _: Service, body: Vec<u8>) -> BoxFuture<'a, Result<Vec<u8>, LinkError>> {
        Box::pin(async move { Ok([format!("{from}:").as_bytes(), &body].concat()) })
    }
}

/// A node's files, issued by `ca`, in `dir`.
fn issue(ca: &ClusterCa, dir: &Path, node: &str, region: &str) -> [String; 3] {
    let issued = ca
        .issue(&NodeCertRequest {
            node: id(node),
            region: region.parse().unwrap(),
            hosts: vec!["127.0.0.1".into()],
            days: 30,
        })
        .unwrap();
    let write = |name: &str, text: &str| {
        let path = dir.join(format!("{node}-{name}.pem"));
        std::fs::write(&path, text).unwrap();
        path.to_string_lossy().into_owned()
    };
    [
        write("cert", &issued.cert_pem),
        write("key", &issued.key_pem),
        write("ca", ca.certificate_pem()),
    ]
}

fn link(files: &[String; 3], to: Peer) -> TlsPeerLink {
    let credentials = NodeCredentials::load(files[0].as_ref(), files[1].as_ref(), files[2].as_ref()).unwrap();
    let mut config = TlsLinkConfig::new(credentials, vec![to]);
    config.connect_timeout = Duration::from_secs(3);
    config.request_timeout = Duration::from_secs(10);
    TlsPeerLink::new(config).unwrap()
}

fn allowlist_pairs(dir: &Path) -> Vec<(String, String)> {
    let file = |name: &str| dir.join(name).to_string_lossy().into_owned();
    [
        ("ALLOWLIST_DRY_RUN", "1".to_owned()),
        ("ALLOWLIST_SECRET", "a".repeat(40)),
        ("ALLOWLIST_PORT", support::free_port().to_string()),
        ("ALLOWLIST_FIREWALL", "nftables".to_owned()),
        ("ALLOWLIST_PORTS", "8770-8777".to_owned()),
        ("ALLOWLIST_AUDIT_LOG", file("audit.log")),
        ("ALLOWLIST_STATE_FILE", file("state.json")),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect()
}

/// Every rule the dry-run helper applied, as (ports, addresses).
fn applied(dir: &Path) -> Vec<(String, Vec<String>)> {
    std::fs::read_to_string(dir.join("audit.log"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|event| event["event"] == "dry_run")
        .map(|event| {
            let ips = event["ips"].as_array().into_iter().flatten();
            (
                event["ports"].as_str().unwrap_or_default().to_owned(),
                ips.filter_map(|ip| ip.as_str().map(str::to_owned)).collect(),
            )
        })
        .collect()
}

/// `POST /__node/enroll` on the public port, as an install redeeming its token does.
async fn enroll(port: u16, body: &str) -> (u16, String) {
    let mut tls = common::tls_connect(port, None).await.unwrap();
    let request = format!(
        "POST /__node/enroll HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    tls.write_all(request.as_bytes()).await.unwrap();
    let mut reply = Vec::new();
    let _ = tls.read_to_end(&mut reply).await;
    let text = String::from_utf8_lossy(&reply).into_owned();
    let status = text.split(' ').nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    (status, text.split("\r\n\r\n").nth(1).unwrap_or("").to_owned())
}

#[tokio::test(flavor = "multi_thread")]
async fn node_links_enrollment_and_allowlist_admits_cross_the_library() {
    let dir = tempfile::tempdir().unwrap();
    let ca = ClusterCa::create(30).unwrap();
    let oregon = issue(&ca, dir.path(), "oregon-1", "oregon");
    let uk = issue(&ca, dir.path(), "uk-1", "uk");
    let de = issue(&ca, dir.path(), "de-1", "de");
    let port = support::free_port();
    let public = common::make_cert();

    let status = StatusRegistry::default();
    let mut options = RegistryOptions::standalone();
    options.node = NodeIdentity {
        node: id("oregon-1"),
        region: "oregon".parse().unwrap(),
    };
    let registry = ModuleRegistry::new(options, status.clone());
    let services = registry.node_services().clone();
    services.attach(Service::Directory, Arc::new(Echo));
    services.set_enroll_handler(Some(Arc::new(|body: Vec<u8>| {
        Box::pin(async move { (200, [b"enrolled:".as_slice(), &body].concat()) })
    })));

    let peers = serde_json::to_string(&[Peer {
        id: id("uk-1"),
        region: "uk".parse().unwrap(),
        host: "127.0.0.1".into(),
        port: 1,
        fingerprint: None,
    }])
    .unwrap();
    let (cert, key) = (
        public.cert_file.to_string_lossy().into_owned(),
        public.key_file.to_string_lossy().into_owned(),
    );
    let port_text = port.to_string();
    let secret = "s".repeat(64);
    let env = Env::from_pairs([
        ("GATEWAY_BIND", "127.0.0.1"),
        ("GATEWAY_PORT", port_text.as_str()),
        ("GATEWAY_CERT", &cert),
        ("GATEWAY_KEY", &key),
        ("GATEWAY_SECRET", &secret),
        ("GATEWAY_ALLOWLIST", "0"),
        ("GATEWAY_NODE_ID", "oregon-1"),
        ("GATEWAY_NODE_REGION", "oregon"),
        ("GATEWAY_NODE_CERT", &oregon[0]),
        ("GATEWAY_NODE_KEY", &oregon[1]),
        ("GATEWAY_NODE_CA", &oregon[2]),
        ("GATEWAY_NETWORK_NODES", &peers),
    ]);
    let gateway_options = GatewayOptions {
        allowlist: AllowlistMode::InProcess {
            env: allowlist_pairs(dir.path()),
        },
        ws_port: None,
    };
    let (module, handle) = gateway_library(support::library().await, env, Some(&registry), gateway_options);
    let (ctx, stop) = status.service_context("gateway");
    let running = tokio::spawn((module.start)(ctx));
    support::ready(&status, "gateway").await;
    support::eventually("the first rule", || {
        applied(dir.path())
            .first()
            .is_some_and(|(ports, _)| ports == "8770-8777")
    })
    .await;

    // A listed peer's call: admitted by the library's endpoint, answered by the core's node services
    let oregon_peer = Peer {
        id: id("oregon-1"),
        region: "oregon".parse().unwrap(),
        host: "127.0.0.1".into(),
        port,
        fingerprint: None,
    };
    let from_uk = link(&uk, oregon_peer.clone());
    let mut answer = None;
    for _ in 0..50 {
        match from_uk
            .call(&id("oregon-1"), Service::Directory, b"hello".to_vec())
            .await
        {
            Ok(body) => {
                answer = Some(body);
                break;
            }
            // Until the core bound its node services to the library's links
            Err(LinkError::NoService { .. }) => tokio::time::sleep(Duration::from_millis(100)).await,
            Err(error) => panic!("{error}"),
        }
    }
    assert_eq!(answer.as_deref(), Some(b"uk-1:hello".as_slice()));
    // One frame can't carry it either way
    let big: Vec<u8> = (0..900 * 1024).map(|i| (i % 251) as u8).collect();
    let answer = from_uk
        .call(&id("oregon-1"), Service::Directory, big.clone())
        .await
        .unwrap();
    assert_eq!(answer, [b"uk-1:".as_slice(), &big].concat());
    // Nothing answers a service the core hasn't attached
    assert!(matches!(
        from_uk.call(&id("oregon-1"), Service::Home, vec![]).await,
        Err(LinkError::NoService { .. })
    ));

    // A peer the core adds (an enrolled install) is let in; before, it is not a peer
    let from_de = link(&de, oregon_peer);
    assert!(
        from_de
            .call(&id("oregon-1"), Service::Directory, b"x".to_vec())
            .await
            .is_err()
    );
    services.add_dynamic_peer(id("de-1"));
    let mut admitted = false;
    for _ in 0..50 {
        if from_de
            .call(&id("oregon-1"), Service::Directory, b"x".to_vec())
            .await
            .ok()
            == Some(b"de-1:x".to_vec())
        {
            admitted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(admitted, "an added peer is let in");

    // The core's enroll handler answers on the public port
    assert_eq!(enroll(port, "{}").await, (200, "enrolled:{}".to_owned()));

    // A loaded module's UDP range joins the rule, as the controller placed it
    registry
        .add_entry(ModuleEntry {
            id: "stub".parse().unwrap(),
            source: ModuleSource::CompiledIn(Arc::new(|| Arc::new(StubModule::new()) as Arc<dyn GameModule>)),
            autoload: false,
            settings: BTreeMap::new(),
        })
        .unwrap();
    registry.load(&"stub".parse().unwrap()).await.unwrap();
    support::eventually("the module's range in the rule", || {
        applied(dir.path())
            .last()
            .is_some_and(|(ports, _)| ports == "8770-8777,65010-65019")
    })
    .await;

    // The ring admits a player on the module's port: through to the library's allowlist
    let node = id("oregon-1");
    let (instance, account) = (
        GlobalId::new(IdKind::Account, &node),
        GlobalId::new(IdKind::Account, &node),
    );
    handle
        .allowlist_sink()
        .admit_player(&instance, &account, "203.0.113.42", 65012);
    support::eventually("the player in the rule", || {
        applied(dir.path())
            .last()
            .is_some_and(|(_, ips)| ips.iter().any(|ip| ip == "203.0.113.42"))
    })
    .await;
    // Not on a port nothing guards
    handle
        .allowlist_sink()
        .admit_player(&instance, &account, "203.0.113.99", 59999);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        applied(dir.path())
            .iter()
            .all(|(_, ips)| !ips.iter().any(|ip| ip == "203.0.113.99"))
    );

    registry.unload_all().await;
    let _ = stop.send(true);
    running.await.unwrap().unwrap();
    // The allowlist closed the game ports on the way out
    assert!(
        std::fs::read_to_string(dir.path().join("audit.log"))
            .unwrap()
            .lines()
            .any(|line| line.contains("\"stop\""))
    );
}
