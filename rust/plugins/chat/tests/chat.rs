//! The chat plugin over real WebSockets on 127.0.0.1:0: login, rooms, room messages, direct messages to
//! someone offline delivered at their next login, history, mutes, and a game module's own JID and rules.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chat::{
    ChatHost, ChatPlugin, Drivers, History, LocalOnly, Login, LoginRefusal, Options, Player, Running, Settings,
};
use futures_util::{SinkExt, StreamExt};
use module_api::chat::{ChatAnswer, ChatQuery};
use module_api::stub::{StubModule, recording_host};
use module_api::{
    BoxFuture, CorePlugin, GameModule, GlobalId, IdKind, InProcessModule, ModuleError, ModuleId, NodeIdentity,
    Registrar,
};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

const DOMAIN: &str = "dom.test";
const RESOURCE: &str = "V2:Jackal:WIN::1";

// ---- The host ----

#[derive(Clone)]
struct Account {
    id: GlobalId,
    module: ModuleId,
    name: String,
}

/// Tokens are `token-<authcid>`; every account belongs to `module`.
struct TestHost {
    accounts: HashMap<String, Account>,
    drivers: Drivers,
}

impl TestHost {
    fn new(module: &str, locals: &[(&str, &str)]) -> Self {
        let node = NodeIdentity::standalone().node;
        let accounts = locals
            .iter()
            .map(|(local, name)| {
                let account = Account {
                    id: GlobalId::new(IdKind::Account, &node),
                    module: module.parse().unwrap(),
                    name: (*name).to_owned(),
                };
                ((*local).to_owned(), account)
            })
            .collect();
        Self {
            accounts,
            drivers: Drivers::default(),
        }
    }
}

impl ChatHost for TestHost {
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
}

struct Server {
    running: Running,
    host: Arc<TestHost>,
    _dir: tempfile::TempDir,
}

async fn start(host: TestHost, plugin: &ChatPlugin) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let config = History::beside(&store::DatabaseConfig::Sqlite {
        file: dir.path().join("game.db"),
    });
    let history = tokio::task::spawn_blocking(move || History::open(&config))
        .await
        .unwrap()
        .unwrap();
    let host = Arc::new(host);
    let settings = Settings {
        port: 0,
        ..Settings::default()
    };
    let running = plugin
        .start(
            &settings,
            NodeIdentity::standalone(),
            host.clone(),
            Arc::new(LocalOnly),
            history,
            Options {
                auto_tick: false,
                ..Options::default()
            },
        )
        .await
        .unwrap();
    Server {
        running,
        host,
        _dir: dir,
    }
}

// ---- The client ----

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

    /// The next text frame, or `None` after 300 ms of silence.
    async fn next(&mut self) -> Option<String> {
        loop {
            match tokio::time::timeout(Duration::from_millis(300), self.socket.next()).await {
                Ok(Some(Ok(Message::Text(text)))) => return Some(text.to_string()),
                Ok(Some(Ok(Message::Close(_)) | Err(_)) | None) | Err(_) => return None,
                Ok(Some(Ok(_))) => {}
            }
        }
    }

    /// Frames until one holds `needle`; panics if none comes.
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

    /// Everything until 300 ms of silence.
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

/// Logs `local` in with its token and binds; `Err` holds the SASL failure.
async fn login_as(port: u16, local: &str, token: &str) -> Result<Client, String> {
    let socket = connect(port, "//").await?;
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
    if !answer.contains("<success") {
        return Err(answer);
    }
    client.open().await;
    client
        .send(&format!(
            "<iq type=\"set\" id=\"b1\"><bind xmlns=\"urn:ietf:params:xml:ns:xmpp-bind\"><resource>{RESOURCE}</resource></bind></iq>"
        ))
        .await;
    client.expect(&format!("<jid>{local}@{DOMAIN}/{RESOURCE}</jid>")).await;
    client.send("<presence/>").await;
    Ok(client)
}

async fn login(port: u16, local: &str) -> Client {
    login_as(port, local, &format!("token-{local}")).await.unwrap()
}

fn host() -> TestHost {
    TestHost::new("isles", &[("ada", "Ada"), ("bo", "Bo"), ("cy", "Cy")])
}

// ---- The tests ----

#[tokio::test]
async fn players_log_in_join_a_room_and_talk() {
    let plugin = ChatPlugin::new();
    let server = start(host(), &plugin).await;
    let port = server.running.port;

    assert!(
        login_as(port, "ada", "token-bo")
            .await
            .err()
            .unwrap()
            .contains("not-authorized")
    );
    assert!(
        login_as(port, "nobody", "token-nobody")
            .await
            .err()
            .unwrap()
            .contains("not-authorized")
    );

    let mut ada = login(port, "ada").await;
    let mut bo = login(port, "bo").await;
    // Two hubs of one region are one room (DD21); each client keeps the JID it asked for
    ada.join("City-a", "Ada").await;
    ada.expect("status code=\"110\"").await;
    bo.join("City-b", "Bo").await;
    let seen = bo
        .expect(&format!("from=\"City-b@muc.{DOMAIN}/{}\"", nick("Ada", "ada")))
        .await;
    assert!(seen.contains(&format!("jid=\"ada@{DOMAIN}/{RESOURCE}\"")), "{seen}");
    bo.expect("status code=\"110\"").await;
    ada.expect(&format!("from=\"City-a@muc.{DOMAIN}/{}\"", nick("Bo", "bo")))
        .await;

    ada.say("City-a", "hello &amp; welcome").await;
    let heard = bo.expect("type=\"groupchat\"").await;
    assert!(
        heard.contains(&format!("from=\"City-b@muc.{DOMAIN}/{}\"", nick("Ada", "ada"))),
        "{heard}"
    );
    assert!(heard.contains("<body>hello &amp; welcome</body>"), "{heard}");
    let echo = ada.expect("type=\"groupchat\"").await;
    assert!(
        echo.contains(&format!("from=\"City-a@muc.{DOMAIN}/{}\"", nick("Ada", "ada"))),
        "{echo}"
    );

    // Region rooms keep no history by default
    let key = chat::naming::room_key(&module_api::chat::RoomKind::Region(NodeIdentity::standalone().region));
    assert!(
        server
            .running
            .history()
            .room_history(&key, 10)
            .await
            .unwrap()
            .is_empty()
    );

    // A nickname that isn't the player's own is refused, and a room DD21 doesn't name too
    let mut cy = login(port, "cy").await;
    cy.send(&format!(
        "<presence to=\"City-a@muc.{DOMAIN}/{}\"/>",
        nick("Ada", "ada")
    ))
    .await;
    cy.expect("<forbidden").await;
    cy.join("Guild-x", "Cy").await;
    cy.expect("<not-allowed").await;
    assert!(connect(port, "/elsewhere").await.is_err(), "other paths are a 404");
    server.running.stop().await;
}

#[tokio::test]
async fn a_whisper_to_someone_offline_arrives_at_their_next_login() {
    let plugin = ChatPlugin::new();
    let server = start(host(), &plugin).await;
    let port = server.running.port;
    // Bo has logged in here before, so chat knows the account behind bo@
    let mut bo = login(port, "bo").await;
    bo.send("<close xmlns=\"urn:ietf:params:xml:ns:xmpp-framing\"/>").await;
    bo.drain().await;

    let mut ada = login(port, "ada").await;
    ada.whisper("bo", "are you there?").await;
    ada.whisper("cy", "never logged in").await;
    assert!(
        ada.drain().await.iter().all(|f| !f.contains("<message")),
        "nothing comes back"
    );

    let mut bo = login(port, "bo").await;
    let waiting = bo.expect("are you there?").await;
    assert!(waiting.contains(&format!("from=\"ada@{DOMAIN}\"")), "{waiting}");
    assert!(waiting.contains("<delay xmlns=\"urn:xmpp:delay\""), "{waiting}");
    assert!(waiting.contains(" id=\"w1\""), "{waiting}");
    // Delivered once: the next login gets nothing
    bo.send("<close xmlns=\"urn:ietf:params:xml:ns:xmpp-framing\"/>").await;
    bo.drain().await;
    let mut bo = login(port, "bo").await;
    assert!(bo.drain().await.iter().all(|f| !f.contains("are you there?")));

    // Online now: straight through, from the full JID
    ada.whisper("bo", "hi again").await;
    let live = bo.expect("hi again").await;
    assert!(
        live.contains(&format!("from=\"ada@{DOMAIN}/{RESOURCE}\"")) && !live.contains("<delay"),
        "{live}"
    );

    let (a, b) = (&server.host.accounts["ada"].id, &server.host.accounts["bo"].id);
    let between = server
        .running
        .history()
        .direct_between(&a.to_string(), &b.to_string(), 10)
        .await
        .unwrap();
    let bodies: Vec<&str> = between.iter().map(|m| m.body.as_str()).collect();
    assert_eq!(bodies, ["are you there?", "hi again"]);
    assert!(between.iter().all(|m| m.delivered_at.is_some()));
    server.running.stop().await;
}

#[tokio::test]
async fn party_rooms_keep_history_for_the_next_joiner() {
    let plugin = ChatPlugin::new();
    let server = start(host(), &plugin).await;
    let port = server.running.port;
    let party = GlobalId::new(IdKind::Party, &NodeIdentity::standalone().node);
    let room = format!("Party-{}", chat::naming::room_id_text(&party));

    let mut ada = login(port, "ada").await;
    ada.join(&room, "Ada").await;
    ada.expect("status code=\"110\"").await;
    ada.say(&room, "first").await;
    ada.say(&room, "second").await;
    ada.drain().await;

    let mut bo = login(port, "bo").await;
    bo.join(&room, "Bo").await;
    let frames = bo.drain().await;
    let own = frames.iter().position(|f| f.contains("status code=\"110\"")).unwrap();
    let history: Vec<&String> = frames.iter().filter(|f| f.contains("<delay")).collect();
    assert_eq!(history.len(), 2, "{frames:#?}");
    assert!(history[0].contains("<body>first</body>") && history[1].contains("<body>second</body>"));
    assert!(frames.iter().position(|f| f.contains("<body>first</body>")).unwrap() > own);
    assert!(history[0].contains(&format!("from=\"{room}@muc.{DOMAIN}/{}\"", nick("Ada", "ada"))));

    let stored = server
        .running
        .history()
        .room_history(&party.to_string(), 10)
        .await
        .unwrap();
    assert_eq!(stored.len(), 2);
    assert_eq!(stored[0].sender, server.host.accounts["ada"].id.to_string());
    server.running.stop().await;
}

#[tokio::test]
async fn a_muted_player_is_not_heard() {
    let plugin = ChatPlugin::new();
    let server = start(host(), &plugin).await;
    let port = server.running.port;
    let mut ada = login(port, "ada").await;
    let mut bo = login(port, "bo").await;
    ada.join("General", "Ada").await;
    bo.join("General", "Bo").await;
    ada.drain().await;
    bo.drain().await;

    let account = server.host.accounts["ada"].id.clone();
    let admin = store::Actor::Admin("mod".into());
    server
        .running
        .mute(&account, None, Some("spam".into()), admin.clone())
        .await
        .unwrap();
    ada.say("General", "can anyone hear me").await;
    let refused = ada.expect("type=\"error\"").await;
    assert!(refused.contains("<forbidden"), "{refused}");
    ada.whisper("bo", "psst").await;
    assert!(
        bo.drain().await.is_empty(),
        "neither the room message nor the whisper reached bo"
    );

    let stored = server
        .running
        .history()
        .mute_of(&account.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (stored.reason.as_deref(), stored.by.as_str()),
        (Some("spam"), "admin:mod")
    );
    // A mute holds across logins
    ada.send("<close xmlns=\"urn:ietf:params:xml:ns:xmpp-framing\"/>").await;
    ada.drain().await;
    let mut ada = login(port, "ada").await;
    ada.join("General", "Ada").await;
    ada.drain().await;
    bo.drain().await;
    ada.say("General", "still muted").await;
    ada.expect("<forbidden").await;

    assert!(server.running.unmute(&account, admin).await.unwrap());
    ada.say("General", "back").await;
    bo.expect("<body>back</body>").await;
    server.running.stop().await;
}

#[tokio::test]
async fn a_module_gives_the_jid_and_the_rules() {
    // The stub module answers chat: its JID's local part is the account's ULID, and the network room is
    // read-only
    let stub: Arc<dyn GameModule> = Arc::new(StubModule::new());
    let manifest = stub.manifest();
    let mut registrar = Registrar::new(manifest.clone());
    stub.register(&mut registrar).unwrap();
    let registration = registrar.finish().unwrap();
    let plugin = ChatPlugin::new();
    plugin.attach(&manifest, &registration).unwrap();
    assert!(plugin.modules().contains(&manifest.id));

    let mut host = TestHost::new("stub", &[]);
    let node = NodeIdentity::standalone().node;
    let id = GlobalId::new(IdKind::Account, &node);
    let local = id.ulid().to_string().to_lowercase();
    host.accounts.insert(
        local.clone(),
        Account {
            id: id.clone(),
            module: manifest.id.clone(),
            name: "Stubby".into(),
        },
    );
    host.accounts.insert(
        "impostor".into(),
        Account {
            id,
            module: manifest.id.clone(),
            name: "Impostor".into(),
        },
    );
    let driver = InProcessModule::new(stub, NodeIdentity::standalone(), recording_host()).unwrap();
    host.drivers.insert(Arc::new(driver));
    let server = start(host, &plugin).await;
    let port = server.running.port;

    // The token is the account's, but the name isn't the one its game gives it
    assert!(
        login_as(port, "impostor", "token-impostor")
            .await
            .err()
            .unwrap()
            .contains("not-authorized")
    );
    let mut player = login(port, &local).await;
    player.join("General", "Stubby").await;
    player.expect("status code=\"110\"").await;
    player.say("General", "hello").await;
    player.expect("<forbidden").await;

    plugin.detach(&manifest.id);
    assert!(plugin.modules().is_empty());
    server.running.stop().await;
}

#[test]
fn the_plugin_serves_chat() {
    let plugin = ChatPlugin::new();
    assert_eq!(plugin.name(), "chat");
    assert_eq!(plugin.serves(), &[module_api::CoreCapability::Chat]);
}
