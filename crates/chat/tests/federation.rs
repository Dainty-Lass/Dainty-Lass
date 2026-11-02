// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Chat federation across nodes (DD21, DN10b): three in-process nodes with real WebSocket clients on
//! 127.0.0.1:0 (DM across two nodes arriving once and stored in both histories, offline DM, region and
//! network rooms across nodes, party room history replication, mutes across nodes, partition catch-up
//! via `chat_outbox`) plus one real mutual-TLS end-to-end test over `network`'s node link.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chat::{
    ChatFederation, ChatFederationConfig, ChatHost, ChatPlugin, Drivers, History, Login, LoginRefusal, Options, Player,
    Running, Settings,
};
use futures_util::{SinkExt, StreamExt};
use module_api::chat::{ChatAnswer, ChatQuery};
use module_api::network::{NodeId, Region};
use module_api::{BoxFuture, GlobalId, IdKind, ModuleError, ModuleId, NodeIdentity};
use network::cluster::peer_identity;
use network::cluster::{ClusterCa, NodeCertRequest, NodeCredentials, Peer};
use network::endpoint::Head;
use network::inprocess::InProcessNetwork;
use network::wire::TIME_HEADER;
use network::{NodeEndpoint, NodeService, Service, ServiceMux, TlsLinkConfig, TlsPeerLink};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

const DOMAIN: &str = "dom.test";
const RESOURCE: &str = "V2:Jackal:WIN::1";
const NODES: [(&str, &str); 3] = [
    ("frankfurt-1", "frankfurt"),
    ("oregon-1", "oregon"),
    ("sydney-1", "sydney"),
];

#[derive(Clone)]
struct Account {
    id: GlobalId,
    module: ModuleId,
    name: String,
    home: NodeId,
}

#[derive(Clone)]
struct ClusterHost {
    node: NodeId,
    accounts: HashMap<String, Account>,
    drivers: Arc<Drivers>,
}

impl ClusterHost {
    fn new(node: NodeId, accounts: HashMap<String, Account>) -> Self {
        Self {
            node,
            accounts,
            drivers: Arc::new(Drivers::default()),
        }
    }
}

impl ChatHost for ClusterHost {
    fn login<'a>(&'a self, login: Login<'a>) -> BoxFuture<'a, Result<Player, LoginRefusal>> {
        Box::pin(async move {
            let account = self.accounts.get(login.authcid).ok_or(LoginRefusal::NoAccount)?;
            if login.token != format!("token-{}", login.authcid) {
                return Err(LoginRefusal::BadToken);
            }
            Ok(Player {
                account: account.id.clone(),
                module: account.module.clone(),
                name: account.name.clone(),
            })
        })
    }

    fn ask<'a>(&'a self, module: &'a ModuleId, query: ChatQuery) -> BoxFuture<'a, Result<ChatAnswer, ModuleError>> {
        Box::pin(self.drivers.ask(module, query))
    }

    fn account_of<'a>(&'a self, local: &'a str) -> BoxFuture<'a, Option<GlobalId>> {
        Box::pin(async move {
            self.accounts
                .get(local)
                .filter(|a| a.home == self.node)
                .map(|a| a.id.clone())
        })
    }
}

struct NodeServer {
    identity: NodeIdentity,
    running: Running,
    _mux: Arc<dyn NodeService>,
    _dir: tempfile::TempDir,
}

struct Cluster {
    net: InProcessNetwork,
    accounts: HashMap<String, Account>,
    nodes: HashMap<&'static str, NodeServer>,
}

impl Cluster {
    async fn start() -> Self {
        let net = InProcessNetwork::new();
        let oregon: NodeId = "oregon-1".parse().unwrap();
        let frankfurt: NodeId = "frankfurt-1".parse().unwrap();
        let sydney: NodeId = "sydney-1".parse().unwrap();
        let module: ModuleId = "isles".parse().unwrap();

        let mut accounts = HashMap::new();
        for (local, name, home) in [
            ("ada", "Ada", oregon.clone()),
            ("bo", "Bo", frankfurt.clone()),
            ("cy", "Cy", frankfurt.clone()),
            ("dee", "Dee", sydney.clone()),
        ] {
            accounts.insert(
                local.to_owned(),
                Account {
                    id: GlobalId::new(IdKind::Account, &home),
                    module: module.clone(),
                    name: name.to_owned(),
                    home,
                },
            );
        }

        let all_peers: Vec<(NodeId, Region)> = NODES
            .iter()
            .map(|(n, r)| (n.parse().unwrap(), r.parse().unwrap()))
            .collect();

        let mut nodes = HashMap::new();
        for (node_str, region_str) in NODES {
            let identity = NodeIdentity {
                node: node_str.parse().unwrap(),
                region: region_str.parse().unwrap(),
            };
            let dir = tempfile::tempdir().unwrap();
            let db_cfg = History::beside(&store::DatabaseConfig::Sqlite {
                file: dir.path().join("game.db"),
            });
            let history = tokio::task::spawn_blocking(move || History::open(&db_cfg))
                .await
                .unwrap()
                .unwrap();
            let host = Arc::new(ClusterHost::new(identity.node.clone(), accounts.clone()));
            let link = Arc::new(net.link(&identity.node));
            let fed_cfg = ChatFederationConfig::new(
                identity.clone(),
                all_peers.iter().filter(|(n, _)| *n != identity.node).cloned(),
            );
            let federation = Arc::new(ChatFederation::new(fed_cfg, link));
            let mux: Arc<dyn NodeService> =
                Arc::new(ServiceMux::new(identity.node.clone()).with(Service::Chat, federation.clone()));
            net.attach(&identity.node, &mux);

            let plugin = ChatPlugin::new();
            let settings = Settings {
                port: 0,
                ..Settings::default()
            };
            let running = plugin
                .start(
                    &settings,
                    identity.clone(),
                    host,
                    federation,
                    history,
                    Options {
                        auto_tick: false,
                        ..Options::default()
                    },
                )
                .await
                .unwrap();
            nodes.insert(
                node_str,
                NodeServer {
                    identity,
                    running,
                    _mux: mux,
                    _dir: dir,
                },
            );
        }

        Self { net, accounts, nodes }
    }

    fn node(&self, name: &str) -> &NodeServer {
        &self.nodes[name]
    }

    async fn stop(self) {
        for (_, node) in self.nodes {
            node.running.stop().await;
        }
    }
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn nick(name: &str, local: &str) -> String {
    format!("{name}:{local}:{RESOURCE}")
}

struct Client {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    local: String,
}

async fn connect(port: u16, path: &str) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>, String> {
    let mut request = format!("ws://127.0.0.1:{port}{path}").into_client_request().unwrap();
    request
        .headers_mut()
        .insert("sec-websocket-protocol", "xmpp".parse().unwrap());
    tokio_tungstenite::connect_async(request)
        .await
        .map(|(socket, _)| socket)
        .map_err(|e| e.to_string())
}

impl Client {
    async fn send(&mut self, text: &str) {
        self.socket.send(Message::Text(text.into())).await.unwrap();
    }

    async fn next(&mut self) -> Option<String> {
        loop {
            match tokio::time::timeout(Duration::from_millis(300), self.socket.next()).await {
                Ok(Some(Ok(Message::Text(text)))) => return Some(text.to_string()),
                Ok(Some(Ok(Message::Close(_)) | Err(_)) | None) | Err(_) => return None,
                Ok(Some(Ok(_))) => {}
            }
        }
    }

    async fn expect(&mut self, needle: &str) -> String {
        let mut seen = Vec::new();
        while let Some(frame) = self.next().await {
            if frame.contains(needle) {
                return frame;
            }
            seen.push(frame);
        }
        panic!("{} never got {needle:?}; got {seen:#?}", self.local);
    }

    async fn drain(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        while let Some(frame) = self.next().await {
            out.push(frame);
        }
        out
    }

    async fn open(&mut self) {
        self.send(&format!(
            "<open xmlns=\"urn:ietf:params:xml:ns:xmpp-framing\" to=\"{DOMAIN}\" version=\"1.0\"/>"
        ))
        .await;
        self.expect("<stream:features").await;
    }

    async fn join(&mut self, room: &str, name: &str) {
        let nick = nick(name, &self.local);
        self.send(&format!(
            "<presence to=\"{room}@muc.{DOMAIN}/{nick}\"><x xmlns=\"http://jabber.org/protocol/muc\"/></presence>"
        ))
        .await;
    }

    async fn say(&mut self, room: &str, body: &str) {
        self.send(&format!(
            "<message type=\"groupchat\" to=\"{room}@muc.{DOMAIN}\" id=\"m1\"><body>{body}</body></message>"
        ))
        .await;
    }

    async fn whisper(&mut self, to: &str, body: &str) {
        self.send(&format!(
            "<message type=\"chat\" to=\"{to}@{DOMAIN}\" id=\"w1\"><body>{body}</body></message>"
        ))
        .await;
    }
}

async fn login(port: u16, local: &str) -> Client {
    let token = format!("token-{local}");
    let socket = connect(port, "//").await.unwrap();
    let mut client = Client {
        socket,
        local: local.to_owned(),
    };
    client.open().await;
    let plain = base64(format!("\0{local}\0{token}").as_bytes());
    client
        .send(&format!(
            "<auth xmlns=\"urn:ietf:params:xml:ns:xmpp-sasl\" mechanism=\"PLAIN\">{plain}</auth>"
        ))
        .await;
    let answer = client.next().await.unwrap_or_default();
    assert!(answer.contains("<success"), "login failed: {answer}");
    client.open().await;
    client
        .send(&format!(
            "<iq type=\"set\" id=\"b1\"><bind xmlns=\"urn:ietf:params:xml:ns:xmpp-bind\"><resource>{RESOURCE}</resource></bind></iq>"
        ))
        .await;
    client.expect(&format!("<jid>{local}@{DOMAIN}/{RESOURCE}</jid>")).await;
    client.send("<presence/>").await;
    client
}

#[tokio::test]
async fn a_dm_across_two_nodes_arrives_once_and_both_histories_hold_it() {
    let cluster = Cluster::start().await;
    let oregon = cluster.node("oregon-1");
    let frankfurt = cluster.node("frankfurt-1");
    let sydney = cluster.node("sydney-1");

    let mut ada = login(oregon.running.port, "ada").await;
    let mut bo = login(frankfurt.running.port, "bo").await;
    // Cy is homed on frankfurt-1, but connects to sydney-1
    let mut cy = login(sydney.running.port, "cy").await;

    // 1. Ada (on oregon-1) whispers to Bo (on frankfurt-1)
    ada.whisper("bo", "hello from oregon").await;
    let received = bo.expect("hello from oregon").await;
    assert!(received.contains(&format!("from=\"ada@{DOMAIN}\"")), "{received}");
    assert!(!received.contains("<delay"), "{received}");
    assert!(
        bo.drain().await.iter().all(|f| !f.contains("hello from oregon")),
        "arrives once only"
    );

    let (ada_id, bo_id) = (
        cluster.accounts["ada"].id.to_string(),
        cluster.accounts["bo"].id.to_string(),
    );
    let oregon_hist = oregon
        .running
        .history()
        .direct_between(&ada_id, &bo_id, 10)
        .await
        .unwrap();
    let frankfurt_hist = frankfurt
        .running
        .history()
        .direct_between(&ada_id, &bo_id, 10)
        .await
        .unwrap();
    assert_eq!(oregon_hist.len(), 1);
    assert_eq!(frankfurt_hist.len(), 1);
    assert_eq!(oregon_hist[0].body, "hello from oregon");
    assert_eq!(frankfurt_hist[0].body, "hello from oregon");
    assert_eq!(oregon_hist[0].mid, frankfurt_hist[0].mid);
    assert!(frankfurt_hist[0].delivered_at.is_some());

    // 2. Ada (homed/online on oregon-1) whispers to Cy (homed on frankfurt-1, online on sydney-1)
    ada.whisper("cy", "hello to sydney via frankfurt").await;
    let cy_got = cy.expect("hello to sydney via frankfurt").await;
    assert!(cy_got.contains(&format!("from=\"ada@{DOMAIN}\"")), "{cy_got}");
    oregon.running.tick().await;

    let cy_id = cluster.accounts["cy"].id.to_string();
    let ada_home_cy = oregon
        .running
        .history()
        .direct_between(&ada_id, &cy_id, 10)
        .await
        .unwrap();
    let cy_home_cy = frankfurt
        .running
        .history()
        .direct_between(&ada_id, &cy_id, 10)
        .await
        .unwrap();
    assert_eq!(ada_home_cy.len(), 1);
    assert_eq!(cy_home_cy.len(), 1);
    assert_eq!(ada_home_cy[0].mid, cy_home_cy[0].mid);
    assert!(cy_home_cy[0].delivered_at.is_some());

    cluster.stop().await;
}

#[tokio::test]
async fn an_offline_dm_across_nodes_waits_at_the_recipients_home_and_delivers_once_at_login() {
    let cluster = Cluster::start().await;
    let oregon = cluster.node("oregon-1");
    let frankfurt = cluster.node("frankfurt-1");
    let sydney = cluster.node("sydney-1");

    let mut ada = login(oregon.running.port, "ada").await;
    // Bo and Cy (both homed on frankfurt-1) are offline
    ada.whisper("bo", "waiting at frankfurt").await;
    ada.whisper("cy", "waiting for cy on sydney").await;
    ada.drain().await;

    let bo_id = cluster.accounts["bo"].id.to_string();
    let cy_id = cluster.accounts["cy"].id.to_string();
    assert_eq!(
        frankfurt.running.history().pending_for(&bo_id, 10).await.unwrap().len(),
        1
    );
    assert_eq!(
        frankfurt.running.history().pending_for(&cy_id, 10).await.unwrap().len(),
        1
    );

    // Bo logs into home node (frankfurt-1) and gets the message once with <delay/>
    let mut bo = login(frankfurt.running.port, "bo").await;
    let delayed = bo.expect("waiting at frankfurt").await;
    assert!(delayed.contains("<delay xmlns=\"urn:xmpp:delay\""), "{delayed}");
    bo.send("<close xmlns=\"urn:ietf:params:xml:ns:xmpp-framing\"/>").await;
    bo.drain().await;
    let mut bo = login(frankfurt.running.port, "bo").await;
    assert!(bo.drain().await.iter().all(|f| !f.contains("waiting at frankfurt")));

    // Cy logs into a foreign node (sydney-1), which pulls waiting messages from frankfurt-1
    let mut cy = login(sydney.running.port, "cy").await;
    let cy_delayed = cy.expect("waiting for cy on sydney").await;
    assert!(cy_delayed.contains("<delay xmlns=\"urn:xmpp:delay\""), "{cy_delayed}");
    assert!(
        frankfurt
            .running
            .history()
            .pending_for(&cy_id, 10)
            .await
            .unwrap()
            .is_empty(),
        "frankfurt-1 marked Cy's waiting message delivered"
    );

    cluster.stop().await;
}

#[tokio::test]
async fn rooms_and_room_history_federate_across_nodes() {
    let cluster = Cluster::start().await;
    let oregon = cluster.node("oregon-1");
    let frankfurt = cluster.node("frankfurt-1");
    let sydney = cluster.node("sydney-1");

    let mut ada = login(oregon.running.port, "ada").await;
    let mut bo = login(frankfurt.running.port, "bo").await;
    let mut dee = login(sydney.running.port, "dee").await;

    // 1. Region room across two nodes: Ada joins City-1 on oregon-1, Bo joins Region-oregon on frankfurt-1
    ada.join("City-1", "Ada").await;
    ada.expect("status code=\"110\"").await;
    bo.join("Region-oregon", "Bo").await;
    bo.expect("status code=\"110\"").await;

    ada.say("City-1", "welcome to oregon").await;
    let bo_heard = bo.expect("welcome to oregon").await;
    assert!(
        bo_heard.contains(&format!("from=\"Region-oregon@muc.{DOMAIN}/{}\"", nick("Ada", "ada"))),
        "{bo_heard}"
    );
    let _ = ada.expect("welcome to oregon").await;

    bo.say("Region-oregon", "greetings from frankfurt").await;
    let ada_heard = ada.expect("greetings from frankfurt").await;
    assert!(
        ada_heard.contains(&format!("from=\"City-1@muc.{DOMAIN}/{}\"", nick("Bo", "bo"))),
        "{ada_heard}"
    );
    let _ = bo.expect("greetings from frankfurt").await;

    // 2. Network-wide General room across three nodes
    ada.join("General", "Ada").await;
    ada.expect("status code=\"110\"").await;
    dee.join("General", "Dee").await;
    dee.expect("status code=\"110\"").await;
    dee.say("General", "network broadcast").await;
    ada.expect("network broadcast").await;
    dee.expect("network broadcast").await;

    // 3. Party room owned by oregon-1 replicates bounded history to a reader joining on frankfurt-1
    let party = GlobalId::new(IdKind::Party, &oregon.identity.node);
    let party_room = format!("Party-{}", chat::naming::room_id_text(&party));
    ada.join(&party_room, "Ada").await;
    ada.expect("status code=\"110\"").await;
    ada.say(&party_room, "party-one").await;
    ada.say(&party_room, "party-two").await;
    ada.drain().await;

    bo.join(&party_room, "Bo").await;
    let frames = bo.drain().await;
    let delayed: Vec<&String> = frames.iter().filter(|f| f.contains("<delay")).collect();
    assert_eq!(delayed.len(), 2, "{frames:#?}");
    assert!(delayed[0].contains("<body>party-one</body>"));
    assert!(delayed[1].contains("<body>party-two</body>"));

    cluster.stop().await;
}

#[tokio::test]
async fn a_mute_at_one_node_holds_at_another() {
    let cluster = Cluster::start().await;
    let oregon = cluster.node("oregon-1");
    let frankfurt = cluster.node("frankfurt-1");
    let sydney = cluster.node("sydney-1");

    let mut ada = login(oregon.running.port, "ada").await;
    // Bo (homed on frankfurt-1) is connected on sydney-1
    let mut bo = login(sydney.running.port, "bo").await;
    ada.join("General", "Ada").await;
    bo.join("General", "Bo").await;
    ada.drain().await;
    bo.drain().await;

    // Mute Bo at Bo's home (frankfurt-1) -> immediately enforced on sydney-1
    let bo_id = cluster.accounts["bo"].id.clone();
    let admin = store::Actor::Admin("mod".into());
    frankfurt
        .running
        .mute(&bo_id, None, Some("federated mute".into()), admin.clone())
        .await
        .unwrap();

    bo.say("General", "muted on sydney").await;
    let err = bo.expect("type=\"error\"").await;
    assert!(err.contains("<forbidden"), "{err}");
    bo.whisper("ada", "muted whisper").await;
    assert!(ada.drain().await.is_empty());

    // Bo reconnects on oregon-1 -> oregon-1 consults frankfurt-1 and keeps Bo muted
    bo.send("<close xmlns=\"urn:ietf:params:xml:ns:xmpp-framing\"/>").await;
    bo.drain().await;
    let mut bo_on_oregon = login(oregon.running.port, "bo").await;
    bo_on_oregon.join("General", "Bo").await;
    bo_on_oregon.drain().await;
    ada.drain().await;
    bo_on_oregon.say("General", "still muted on oregon").await;
    bo_on_oregon.expect("<forbidden").await;

    // Unmute at frankfurt-1 -> Bo can speak on oregon-1
    assert!(frankfurt.running.unmute(&bo_id, admin).await.unwrap());
    bo_on_oregon.say("General", "unmuted now").await;
    ada.expect("<body>unmuted now</body>").await;

    cluster.stop().await;
}

#[tokio::test]
async fn a_partitioned_node_catches_up_from_the_outbox_when_healed() {
    let cluster = Cluster::start().await;
    let oregon = cluster.node("oregon-1");
    let frankfurt = cluster.node("frankfurt-1");

    let mut ada = login(oregon.running.port, "ada").await;
    let mut bo = login(frankfurt.running.port, "bo").await;
    ada.join("City-1", "Ada").await;
    ada.expect("status code=\"110\"").await;
    bo.join("Region-oregon", "Bo").await;
    bo.expect("status code=\"110\"").await;
    ada.drain().await;
    bo.drain().await;

    // Partition oregon-1 and frankfurt-1
    cluster.net.cut(&oregon.identity.node, &frankfurt.identity.node);

    ada.whisper("bo", "dm while cut").await;
    ada.say("City-1", "room while cut").await;
    let _ = ada.expect("room while cut").await;
    assert!(bo.drain().await.is_empty(), "nothing crosses while cut");
    assert!(oregon.running.history().outbox_len().await.unwrap() >= 2);

    // Heal the partition and tick oregon-1's outbox
    cluster.net.heal();
    oregon.running.tick().await;

    let frames = bo.drain().await;
    assert!(
        frames.iter().any(|f| f.contains("dm while cut")),
        "expected DM after heal, got {frames:#?}"
    );
    assert!(
        frames.iter().any(|f| f.contains("room while cut")),
        "expected room message after heal, got {frames:#?}"
    );
    assert_eq!(oregon.running.history().outbox_len().await.unwrap(), 0);

    cluster.stop().await;
}

/// A node's `/__node` listener over mutual TLS, built from `network`'s parts: the gateway (AGPL) did this before
/// chat moved to MIT (spec-rd27-chat §6.3).
fn node_link_server(
    listener: tokio::net::TcpListener,
    tls: Arc<rustls::ServerConfig>,
    endpoint: Arc<NodeEndpoint>,
) -> tokio::task::JoinHandle<()> {
    let acceptor = tokio_rustls::TlsAcceptor::from(tls);
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let (acceptor, endpoint) = (acceptor.clone(), endpoint.clone());
            tokio::spawn(async move {
                let Ok(stream) = acceptor.accept(tcp).await else { return };
                let Some(from) = stream
                    .get_ref()
                    .1
                    .peer_certificates()
                    .and_then(|c| peer_identity(c).ok())
                else {
                    return;
                };
                let service = hyper::service::service_fn(move |request| {
                    let (endpoint, from) = (endpoint.clone(), from.node.clone());
                    async move { Ok::<_, std::convert::Infallible>(node_link_answer(&endpoint, &from, request).await) }
                });
                let io = hyper_util::rt::TokioIo::new(stream);
                _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, service)
                    .await;
            });
        }
    })
}

async fn node_link_answer(
    endpoint: &NodeEndpoint,
    from: &NodeId,
    request: hyper::Request<hyper::body::Incoming>,
) -> hyper::Response<http_body_util::Full<hyper::body::Bytes>> {
    use http_body_util::BodyExt as _;
    let (parts, body) = request.into_parts();
    let header = |name: &str| parts.headers.get(name).and_then(|v| v.to_str().ok());
    let head = Head {
        method: parts.method.as_str(),
        target: parts.uri.path_and_query().map_or("", |p| p.as_str()),
        content_length: header("content-length").and_then(|v| v.parse().ok()),
        time: header(TIME_HEADER),
    };
    let reply = match endpoint.admit(from, &head) {
        Err(refused) => refused,
        Ok(target) => {
            let body = body.collect().await.map(|b| b.to_bytes().to_vec()).unwrap_or_default();
            endpoint.serve(from, target, body).await
        }
    };
    hyper::Response::builder()
        .status(reply.status)
        .header(TIME_HEADER, reply.time_ms.to_string())
        .header("content-type", "application/json")
        .body(http_body_util::Full::new(reply.body.into()))
        .unwrap()
}

#[tokio::test]
async fn federation_works_end_to_end_over_mutual_tls_node_links() {
    let ca = ClusterCa::create(30).unwrap();
    let oregon_id = NodeIdentity {
        node: "oregon-1".parse().unwrap(),
        region: "oregon".parse().unwrap(),
    };
    let frankfurt_id = NodeIdentity {
        node: "frankfurt-1".parse().unwrap(),
        region: "frankfurt".parse().unwrap(),
    };

    let temp_or = tempfile::tempdir().unwrap();
    let temp_de = tempfile::tempdir().unwrap();

    let write_files = |dir: &std::path::Path, id: &NodeIdentity| {
        let issued = ca
            .issue(&NodeCertRequest {
                node: id.node.clone(),
                region: id.region.clone(),
                hosts: vec!["127.0.0.1".into()],
                days: 30,
            })
            .unwrap();
        let cert = dir.join("node-cert.pem");
        let key = dir.join("node-key.pem");
        let ca_path = dir.join("ca-cert.pem");
        std::fs::write(&cert, &issued.cert_pem).unwrap();
        std::fs::write(&key, &issued.key_pem).unwrap();
        std::fs::write(&ca_path, ca.certificate_pem()).unwrap();
        NodeCredentials::load(&cert, &key, &ca_path).unwrap()
    };
    let or_creds = write_files(temp_or.path(), &oregon_id);
    let de_creds = write_files(temp_de.path(), &frankfurt_id);
    let (or_tls, de_tls) = (or_creds.server_config().unwrap(), de_creds.server_config().unwrap());

    let or_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let de_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let or_port = or_listener.local_addr().unwrap().port();
    let de_port = de_listener.local_addr().unwrap().port();

    let or_peer = Peer {
        id: oregon_id.node.clone(),
        region: oregon_id.region.clone(),
        host: "127.0.0.1".into(),
        port: or_port,
        fingerprint: None,
    };
    let de_peer = Peer {
        id: frankfurt_id.node.clone(),
        region: frankfurt_id.region.clone(),
        host: "127.0.0.1".into(),
        port: de_port,
        fingerprint: None,
    };

    let or_link = Arc::new(TlsPeerLink::new(TlsLinkConfig::new(or_creds, vec![de_peer.clone()])).unwrap());
    let de_link = Arc::new(TlsPeerLink::new(TlsLinkConfig::new(de_creds, vec![or_peer.clone()])).unwrap());

    let module: ModuleId = "isles".parse().unwrap();
    let mut accounts = HashMap::new();
    accounts.insert(
        "ada".into(),
        Account {
            id: GlobalId::new(IdKind::Account, &oregon_id.node),
            module: module.clone(),
            name: "Ada".into(),
            home: oregon_id.node.clone(),
        },
    );
    accounts.insert(
        "bo".into(),
        Account {
            id: GlobalId::new(IdKind::Account, &frankfurt_id.node),
            module,
            name: "Bo".into(),
            home: frankfurt_id.node.clone(),
        },
    );

    let or_fed = Arc::new(ChatFederation::new(
        ChatFederationConfig::new(
            oregon_id.clone(),
            vec![(frankfurt_id.node.clone(), frankfurt_id.region.clone())],
        ),
        or_link,
    ));
    let de_fed = Arc::new(ChatFederation::new(
        ChatFederationConfig::new(
            frankfurt_id.clone(),
            vec![(oregon_id.node.clone(), oregon_id.region.clone())],
        ),
        de_link,
    ));

    let or_mux: Arc<dyn NodeService> =
        Arc::new(ServiceMux::new(oregon_id.node.clone()).with(Service::Chat, or_fed.clone()));
    let de_mux: Arc<dyn NodeService> =
        Arc::new(ServiceMux::new(frankfurt_id.node.clone()).with(Service::Chat, de_fed.clone()));

    let serve = |me: &NodeIdentity, peer: &NodeId, listener, tls, service| {
        let endpoint = Arc::new(NodeEndpoint::new(me.clone(), [peer.clone()]));
        endpoint.attach(service);
        node_link_server(listener, tls, endpoint)
    };
    let or_server = serve(&oregon_id, &frankfurt_id.node, or_listener, or_tls, or_mux);
    let de_server = serve(&frankfurt_id, &oregon_id.node, de_listener, de_tls, de_mux);

    let or_db_cfg = History::beside(&store::DatabaseConfig::Sqlite {
        file: temp_or.path().join("game.db"),
    });
    let de_db_cfg = History::beside(&store::DatabaseConfig::Sqlite {
        file: temp_de.path().join("game.db"),
    });
    let or_hist = tokio::task::spawn_blocking(move || History::open(&or_db_cfg))
        .await
        .unwrap()
        .unwrap();
    let de_hist = tokio::task::spawn_blocking(move || History::open(&de_db_cfg))
        .await
        .unwrap()
        .unwrap();

    let plugin_or = ChatPlugin::new();
    let plugin_de = ChatPlugin::new();
    let settings = Settings {
        port: 0,
        ..Settings::default()
    };
    let running_or = plugin_or
        .start(
            &settings,
            oregon_id.clone(),
            Arc::new(ClusterHost::new(oregon_id.node.clone(), accounts.clone())),
            or_fed,
            or_hist,
            Options {
                auto_tick: false,
                ..Options::default()
            },
        )
        .await
        .unwrap();
    let running_de = plugin_de
        .start(
            &settings,
            frankfurt_id.clone(),
            Arc::new(ClusterHost::new(frankfurt_id.node.clone(), accounts.clone())),
            de_fed,
            de_hist,
            Options {
                auto_tick: false,
                ..Options::default()
            },
        )
        .await
        .unwrap();

    let mut ada = login(running_or.port, "ada").await;
    let mut bo = login(running_de.port, "bo").await;

    ada.whisper("bo", "over mutual tls").await;
    let received = bo.expect("over mutual tls").await;
    assert!(received.contains(&format!("from=\"ada@{DOMAIN}\"")), "{received}");

    let (ada_id, bo_id) = (accounts["ada"].id.to_string(), accounts["bo"].id.to_string());
    assert_eq!(
        running_or
            .history()
            .direct_between(&ada_id, &bo_id, 10)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        running_de
            .history()
            .direct_between(&ada_id, &bo_id, 10)
            .await
            .unwrap()
            .len(),
        1
    );

    running_or.stop().await;
    running_de.stop().await;
    or_server.abort();
    de_server.abort();
}
