// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The clean-room rewrite's own black-box tests (RD27), over real sockets: ghost replacement and the loopback
//! `/__chat/*` API, which `tests/chat.rs` doesn't reach.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use chat::hooks::{ChatHost, Login, LoginRefusal, Player};
use chat::{ChatPlugin, History, LocalOnly, Options, Settings};
use futures_util::{SinkExt as _, StreamExt as _};
use module_api::chat::{ChatAnswer, ChatQuery};
use module_api::network::{IdKind, NodeId};
use module_api::{BoxFuture, GlobalId, ModuleError, ModuleId, NodeIdentity};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

const RESOURCE: &str = "V2:app:WIN::0123";

struct Host {
    accounts: HashMap<String, (GlobalId, String)>,
}

impl ChatHost for Host {
    fn login<'a>(&'a self, login: Login<'a>) -> BoxFuture<'a, Result<Player, LoginRefusal>> {
        Box::pin(async move {
            match self.accounts.get(login.authcid) {
                Some((account, name)) if login.token == "good" => Ok(Player {
                    account: account.clone(),
                    module: "stub".parse().unwrap(),
                    name: name.clone(),
                }),
                Some(_) => Err(LoginRefusal::BadToken),
                None => Err(LoginRefusal::NoAccount),
            }
        })
    }

    fn ask<'a>(&'a self, _: &'a ModuleId, _: ChatQuery) -> BoxFuture<'a, Result<ChatAnswer, ModuleError>> {
        Box::pin(async { Err(ModuleError::unsupported("chat")) })
    }

    fn account_of<'a>(&'a self, local: &'a str) -> BoxFuture<'a, Option<GlobalId>> {
        Box::pin(async move { self.accounts.get(local).map(|(a, _)| a.clone()) })
    }
}

struct Server {
    running: chat::Running,
    accounts: HashMap<String, GlobalId>,
    _dir: tempfile::TempDir,
}

async fn server() -> Server {
    let node = NodeIdentity::standalone();
    let mut accounts = HashMap::new();
    let mut known = HashMap::new();
    for (uid, name) in [("UID-ada", "Ada"), ("UID-bo", "Bo"), ("UID-cy", "Cy")] {
        let id = GlobalId::new(IdKind::Account, &node.node);
        accounts.insert(uid.to_owned(), (id.clone(), name.to_owned()));
        known.insert(uid.to_owned(), id);
    }
    let settings = Settings {
        port: 0,
        ..Settings::default()
    };
    let options = Options {
        auto_tick: false,
        ..Options::default()
    };
    let dir = tempfile::tempdir().unwrap();
    let database = History::beside(&store::DatabaseConfig::Sqlite {
        file: dir.path().join("game.db"),
    });
    let history = tokio::task::spawn_blocking(move || History::open(&database))
        .await
        .unwrap()
        .unwrap();
    let running = ChatPlugin::new()
        .start(
            &settings,
            node,
            Arc::new(Host { accounts }),
            Arc::new(LocalOnly),
            history,
            options,
        )
        .await
        .unwrap();
    Server {
        running,
        accounts: known,
        _dir: dir,
    }
}

struct Client {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    seen: Vec<String>,
}

impl Client {
    async fn connect(port: u16, path: &str) -> Result<Self, String> {
        let mut request = format!("ws://127.0.0.1:{port}{path}").into_client_request().unwrap();
        request
            .headers_mut()
            .insert("sec-websocket-protocol", "xmpp".parse().unwrap());
        let (ws, response) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| e.to_string())?;
        assert_eq!(response.headers()["sec-websocket-protocol"], "xmpp");
        Ok(Self { ws, seen: Vec::new() })
    }

    async fn send(&mut self, text: &str) {
        self.ws.send(Message::text(text)).await.unwrap();
    }

    /// The next frame holding `needle`, skipping others.
    async fn expect(&mut self, needle: &str) -> String {
        let found = tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(Ok(message)) = self.ws.next().await {
                if let Message::Text(text) = message {
                    let text = text.as_str().to_owned();
                    self.seen.push(text.clone());
                    if text.contains(needle) {
                        return Some(text);
                    }
                }
            }
            None
        })
        .await;
        match found {
            Ok(Some(text)) => text,
            _ => panic!("no frame with {needle:?}; saw {:#?}", self.seen),
        }
    }

    /// Every frame arriving within `ms`.
    async fn quiet(&mut self, ms: u64) -> Vec<String> {
        let mut frames = Vec::new();
        while let Ok(Some(Ok(message))) = tokio::time::timeout(Duration::from_millis(ms), self.ws.next()).await {
            if let Message::Text(text) = message {
                frames.push(text.as_str().to_owned());
            }
        }
        frames
    }

    async fn open(&mut self) -> String {
        self.send(r#"<open xmlns="urn:ietf:params:xml:ns:xmpp-framing" to="prod.ol.epicgames.com" version="1.0"/>"#)
            .await;
        self.expect("<stream:features").await
    }

    async fn auth(&mut self, uid: &str, token: &str) -> String {
        let payload = base64::engine::general_purpose::STANDARD.encode(format!("\0{uid}\0{token}"));
        self.send(&format!(
            r#"<auth xmlns="urn:ietf:params:xml:ns:xmpp-sasl" mechanism="PLAIN">{payload}</auth>"#
        ))
        .await;
        self.expect("xmpp-sasl").await
    }
}

async fn login(port: u16, uid: &str) -> Client {
    let mut client = Client::connect(port, "/").await.unwrap();
    client.open().await;
    let answer = client.auth(uid, "good").await;
    assert_eq!(answer, r#"<success xmlns="urn:ietf:params:xml:ns:xmpp-sasl"/>"#);
    let features = client.open().await;
    assert!(features.contains("xmpp-bind"));
    client
        .send(&format!(
            r#"<iq type="set" id="b1"><bind xmlns="urn:ietf:params:xml:ns:xmpp-bind"><resource>{RESOURCE}</resource></bind></iq>"#
        ))
        .await;
    let bound = client.expect("<jid>").await;
    assert_eq!(
        bound,
        format!(
            r#"<iq xmlns="jabber:client" type="result" id="b1"><bind xmlns="urn:ietf:params:xml:ns:xmpp-bind"><jid>{uid}@prod.ol.epicgames.com/{RESOURCE}</jid></bind></iq>"#
        )
    );
    client.send("<presence/>").await;
    client
}

fn nick(name: &str, uid: &str) -> String {
    format!("{name}:{uid}:{RESOURCE}")
}

fn join(room: &str, nick: &str) -> String {
    format!(
        r#"<presence to="{room}@muc.prod.ol.epicgames.com/{nick}"><x xmlns="http://jabber.org/protocol/muc"/></presence>"#
    )
}

fn say(room: &str, id: &str, body: &str) -> String {
    format!(
        r#"<message type="groupchat" to="{room}@muc.prod.ol.epicgames.com" id="{id}"><body>{body}</body></message>"#
    )
}

async fn http(port: u16, request: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    let status = response[9..12].parse().unwrap();
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_owned())
        .unwrap_or_default();
    (status, body)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn logins_rooms_and_the_city_merge() {
    let server = server().await;
    let port = server.running.port;

    let mut wrong = Client::connect(port, "//").await.unwrap();
    wrong.open().await;
    assert_eq!(
        wrong.auth("UID-ada", "bad").await,
        r#"<failure xmlns="urn:ietf:params:xml:ns:xmpp-sasl"><not-authorized/></failure>"#
    );
    let mut unknown = Client::connect(port, "/__ws/xmpp").await.unwrap();
    unknown.open().await;
    assert!(unknown.auth("UID-nobody", "good").await.contains("<not-authorized/>"));
    let mut mechanism = Client::connect(port, "/").await.unwrap();
    mechanism.open().await;
    mechanism
        .send(r#"<auth xmlns="urn:ietf:params:xml:ns:xmpp-sasl" mechanism="X">AA==</auth>"#)
        .await;
    assert!(mechanism.expect("failure").await.contains("<invalid-mechanism/>"));
    assert!(Client::connect(port, "/other").await.err().unwrap().contains("404"));

    let mut ada = login(port, "UID-ada").await;
    let mut bo = login(port, "UID-bo").await;
    ada.send(&join("City-a", &nick("Ada", "UID-ada"))).await;
    let own = ada.expect("110").await;
    assert_eq!(
        own,
        format!(
            r#"<presence xmlns="jabber:client" from="City-a@muc.prod.ol.epicgames.com/{n}" to="UID-ada@prod.ol.epicgames.com/{RESOURCE}"><x xmlns="http://jabber.org/protocol/muc#user"><item affiliation="none" role="participant" jid="UID-ada@prod.ol.epicgames.com/{RESOURCE}"/><status code="110"/></x></presence>"#,
            n = nick("Ada", "UID-ada")
        )
    );
    bo.send(&join("City-b", &nick("Bo", "UID-bo"))).await;
    let seen_ada = bo.expect("UID-ada").await;
    assert!(seen_ada.contains(&format!(
        r#"from="City-b@muc.prod.ol.epicgames.com/{}""#,
        nick("Ada", "UID-ada")
    )));
    bo.expect("110").await;
    let seen_bo = ada.expect("UID-bo").await;
    assert!(seen_bo.contains(r#"from="City-a@muc.prod.ol.epicgames.com/Bo:UID-bo"#));

    ada.send(&say("City-a", "m1", "a &amp; b")).await;
    let heard = bo.expect("groupchat").await;
    assert_eq!(
        heard,
        format!(
            r#"<message xmlns="jabber:client" type="groupchat" id="m1" from="City-b@muc.prod.ol.epicgames.com/{}" to="UID-bo@prod.ol.epicgames.com/{RESOURCE}"><body>a &amp; b</body></message>"#,
            nick("Ada", "UID-ada")
        )
    );
    let echo = ada.expect("groupchat").await;
    assert!(echo.contains(r#"from="City-a@muc.prod.ol.epicgames.com/Ada:UID-ada"#));
    let stored = server.running.history().room_history("region:local", 10).await.unwrap();
    assert!(stored.is_empty(), "region rooms keep no history by default");

    // Someone else's nick, an unknown room, a message to a room not held
    let mut cy = login(port, "UID-cy").await;
    cy.send(&join("City-c", &nick("Ada", "UID-ada"))).await;
    let refused = cy.expect("type=\"error\"").await;
    assert!(refused.contains(r#"<error type="auth"><forbidden xmlns="urn:ietf:params:xml:ns:xmpp-stanzas"/>"#));
    cy.send(&join("Guild-x", &nick("Cy", "UID-cy"))).await;
    assert!(cy.expect("type=\"error\"").await.contains("<not-allowed"));
    cy.send(&say("City-c", "m2", "hi")).await;
    assert!(
        cy.expect("type=\"error\"")
            .await
            .contains(r#"<error type="modify"><not-acceptable"#)
    );

    // Leaving tells the others, and the leaver gets its own unavailable presence
    bo.send(r#"<presence type="unavailable" to="City-b@muc.prod.ol.epicgames.com/x"/>"#)
        .await;
    let gone = ada.expect("unavailable").await;
    assert!(gone.contains(r#"role="none""#) && !gone.contains("110"));
    assert!(bo.expect("unavailable").await.contains(r#"<status code="110"/>"#));

    // The loopback control API: a tick, and nothing the gateway forwards
    let (status, body) = http(
        port,
        "POST /__chat/tick HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
    )
    .await;
    assert_eq!((status, body.as_str()), (200, r#"{"ok":true}"#));
    let (status, _) = http(
        port,
        "POST /__chat/tick HTTP/1.1\r\nHost: x\r\nx-reforged-gateway: s\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
    )
    .await;
    assert_eq!(status, 403);
    let (status, body) = http(
        port,
        "GET /__chat/history/direct?b=x HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert_eq!((status, body.as_str()), (400, r#"{"error":"missing a"}"#));

    server.running.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn whispers_wait_for_the_next_login_and_mutes_hold() {
    let server = server().await;
    let port = server.running.port;
    let mut ada = login(port, "UID-ada").await;
    ada.send(r#"<message type="chat" to="UID-cy@prod.ol.epicgames.com" id="w1"><body>later &lt;3</body></message>"#)
        .await;
    assert!(ada.quiet(300).await.is_empty(), "nothing comes back to the sender");

    let mut cy = login(port, "UID-cy").await;
    let waiting = cy.expect("UID-ada@").await;
    assert!(waiting.starts_with(
        r#"<message xmlns="jabber:client" type="chat" from="UID-ada@prod.ol.epicgames.com" to="UID-cy@prod.ol.epicgames.com/V2:app:WIN::0123" id="w1"><body>later &lt;3</body><delay xmlns="urn:xmpp:delay" from="prod.ol.epicgames.com" stamp=""#
    ));
    let cy_account = &server.accounts["UID-cy"];
    let ada_account = &server.accounts["UID-ada"];
    // The bind is marked delivered after its stanzas go out; a tick queues behind it
    server.running.tick().await;
    assert!(
        server
            .running
            .history()
            .pending_for(&cy_account.to_string(), 10)
            .await
            .unwrap()
            .is_empty()
    );

    ada.send(r#"<message to="UID-cy@prod.ol.epicgames.com/V2:app:WIN::0123"><body>now</body></message>"#)
        .await;
    let live = cy.expect("now").await;
    assert!(live.contains(&format!(r#"from="UID-ada@prod.ol.epicgames.com/{RESOURCE}""#)));
    assert!(!live.contains("delay"));
    let both = server
        .running
        .history()
        .direct_between(&ada_account.to_string(), &cy_account.to_string(), 10)
        .await
        .unwrap();
    assert_eq!(both.len(), 2);
    assert!(both.iter().all(|m| m.delivered_at.is_some()));

    // A report without a dashboard is answered to the sender alone
    ada.send(r#"<message type="chat" to="UID-cy@prod.ol.epicgames.com"><body>/report UID-cy spam it was bad</body></message>"#)
        .await;
    assert!(
        ada.expect("Reports are not set up on this server.")
            .await
            .contains(r#"type="chat""#)
    );

    // Party rooms keep history; a muted player is not heard
    let party = GlobalId::new(IdKind::Party, &NodeId::new("local").unwrap());
    let room = format!("Party-{}", chat::naming::room_id_text(&party));
    ada.send(&join(&room, &nick("Ada", "UID-ada"))).await;
    ada.expect("110").await;
    ada.send(&say(&room, "p1", "one")).await;
    ada.send(&say(&room, "p2", "two")).await;
    ada.expect(">two<").await;
    cy.send(&join(&room, &nick("Cy", "UID-cy"))).await;
    cy.expect("110").await;
    assert!(cy.expect(">one<").await.contains("urn:xmpp:delay"));
    assert!(cy.expect(">two<").await.contains(r#"id="p2" from=""#));

    let actor = store::Actor::Admin(String::from("mod"));
    server
        .running
        .mute(ada_account, None, None, actor.clone())
        .await
        .unwrap();
    ada.send(&say(&room, "p3", "three")).await;
    assert!(ada.expect("type=\"error\"").await.contains("<forbidden"));
    let mute = server
        .running
        .history()
        .mute_of(&ada_account.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(mute.by, "admin:mod");
    assert!(server.running.unmute(ada_account, actor).await.unwrap());
    ada.send(&say(&room, "p4", "four")).await;
    cy.expect(">four<").await;

    server.running.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_bind_of_one_resource_replaces_the_first() {
    let server = server().await;
    let port = server.running.port;
    let mut first = login(port, "UID-bo").await;
    let _second = login(port, "UID-bo").await;
    assert!(
        first
            .expect("<close")
            .await
            .contains("urn:ietf:params:xml:ns:xmpp-framing")
    );
    let mut early = Client::connect(port, "/").await.unwrap();
    early.send("<?xml version='1.0'?>").await;
    assert!(early.expect("restricted-xml").await.contains("stream:error"));
    server.running.stop().await;
}
