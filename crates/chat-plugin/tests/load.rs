// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The core loads the built chat library and players chat through it as they did through the compiled-in plugin
//! (`crates/chat/tests/chat.rs`): their logins come from a game module of the core's, and the library's history
//! keeps a whisper for someone offline (RD27).
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use controller::{ModuleEntry, ModuleRegistry, ModuleSource, RegistryOptions, ServerController, ServiceLibrary};
use futures_util::{SinkExt, StreamExt};
use module_api::chat::{ChatAnswer, ChatQuery};
use module_api::host::file_sha256;
use module_api::stub::StubModule;
use module_api::{
    BoxFuture, GameModule, GlobalId, ModuleConfig, ModuleContext, ModuleError, ModuleManifest, ModuleStatus,
    NodeIdentity, Registrar,
};
use service_kit::context::ComponentPhase;
use service_kit::env::Env;
use service_plugin::chat::ChatSetup;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

const DOMAIN: &str = "dom.test";
const RESOURCE: &str = "V2:Jackal:WIN::1";
const PLAYERS: [(&str, &str); 3] = [("ada", "Ada"), ("bo", "Bo"), ("cy", "Cy")];

/// The library cargo built beside this test (target/<profile>/), before running it.
fn library() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let profile = exe.parent().and_then(|deps| deps.parent()).unwrap();
    let name = format!(
        "{}chat_plugin{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    profile.join(name)
}

/// The stub module, answering logins: tokens are `token-<local>`.
struct Talk {
    stub: StubModule,
    accounts: BTreeMap<String, GlobalId>,
}

impl Talk {
    fn new() -> Self {
        let node = NodeIdentity::standalone().node;
        let accounts = PLAYERS
            .iter()
            .map(|(local, _)| ((*local).to_owned(), GlobalId::new(module_api::IdKind::Account, &node)))
            .collect();
        Self {
            stub: StubModule::new(),
            accounts,
        }
    }
}

impl GameModule for Talk {
    fn manifest(&self) -> ModuleManifest {
        self.stub.manifest()
    }

    fn register(&self, registrar: &mut Registrar) -> Result<(), ModuleError> {
        self.stub.register(registrar)
    }

    fn start(&self, ctx: ModuleContext, config: ModuleConfig) -> BoxFuture<'_, Result<(), ModuleError>> {
        self.stub.start(ctx, config)
    }

    fn stop(&self, ctx: ModuleContext) -> BoxFuture<'_, Result<(), ModuleError>> {
        self.stub.stop(ctx)
    }

    fn status(&self) -> ModuleStatus {
        self.stub.status()
    }

    fn chat(&self, ctx: ModuleContext, query: ChatQuery) -> BoxFuture<'_, Result<ChatAnswer, ModuleError>> {
        let refused = || ModuleError::new(module_api::ErrorKind::Invalid, "bad token");
        match query {
            ChatQuery::Login { authcid, token } => {
                let answer = match self.accounts.get(&authcid) {
                    Some(account) if token == format!("token-{authcid}") => Ok(ChatAnswer::Login {
                        account: account.clone(),
                        name: PLAYERS.iter().find(|(l, _)| *l == authcid).unwrap().1.to_owned(),
                    }),
                    _ => Err(refused()),
                };
                Box::pin(async move { answer })
            }
            other => self.stub.chat(ctx, other),
        }
    }
}

struct Client {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    local: String,
}

impl Client {
    async fn send(&mut self, text: &str) {
        self.socket.send(Message::Text(text.into())).await.unwrap();
    }

    async fn next(&mut self) -> Option<String> {
        loop {
            match tokio::time::timeout(Duration::from_millis(500), self.socket.next()).await {
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

    async fn close(&mut self) {
        self.send("<close xmlns=\"urn:ietf:params:xml:ns:xmpp-framing\"/>")
            .await;
        self.drain().await;
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

async fn login_as(port: u16, local: &str, token: &str) -> Result<Client, String> {
    let mut request = format!("ws://127.0.0.1:{port}//").into_client_request().unwrap();
    request
        .headers_mut()
        .insert("sec-websocket-protocol", "xmpp".parse().unwrap());
    let (socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| e.to_string())?;
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

#[tokio::test(flavor = "multi_thread")]
async fn players_chat_through_the_library_the_core_loads() {
    let path = library();
    // A cdylib is not built for its own tests, and a nested cargo would wait on this one's lock
    assert!(
        path.is_file(),
        "{} is missing: run `cargo build -p chat-plugin` before the tests (CI does)",
        path.display()
    );
    exercise(Some(path)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn players_chat_the_same_with_chat_compiled_in() {
    exercise(None).await;
}

/// The whole script, with chat from its library (`Some`) or compiled in: both must pass it unchanged.
async fn exercise(library: Option<PathBuf>) {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("dainty.db");
    let env = Env::from_pairs([
        ("CHAT_PORT", "0"),
        ("REFORGED_DB_ENGINE", "sqlite"),
        ("REFORGED_DB_FILE", database.to_str().unwrap()),
    ]);

    let mut controller = ServerController::new("private", Vec::new());
    let registry = ModuleRegistry::new(RegistryOptions::standalone(), controller.status_registry().clone());
    let mut events = registry.subscribe_events();
    let setup = ChatSetup {
        node: NodeIdentity::standalone(),
        network: None,
    };
    let module = match library {
        Some(path) => {
            let library = ServiceLibrary {
                name: "chat",
                sha256: file_sha256(&path).await.unwrap(),
                path,
                serves: &[],
            };
            controller::service_library::chat_library(library, env, &registry, setup)
        }
        None => {
            controller::service_library::chat_compiled_in(chat::service::ChatService::default, env, &registry, setup)
        }
    };
    controller.add(module);
    registry
        .add_entry(ModuleEntry {
            id: "stub".parse().unwrap(),
            source: ModuleSource::CompiledIn(Arc::new(|| Arc::new(Talk::new()) as Arc<dyn GameModule>)),
            autoload: true,
            settings: BTreeMap::new(),
        })
        .unwrap();
    controller.set_registry(registry.clone());
    controller.start_all().await.unwrap();

    let status = controller.status_registry().clone();
    let port = ready_port(&status, None).await;

    // The module checks the token, through the core
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

    // Bo has logged in before, so chat knows the account behind bo@
    let mut bo = login(port, "bo").await;
    bo.close().await;

    let mut ada = login(port, "ada").await;
    let presence = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let (_, event) = events.recv().await.unwrap();
            if event.name.as_str() == module_api::chat::PRESENCE_EVENT && event.payload["user_id"] == "ada" {
                break event.payload;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(presence["online"], true);

    ada.send(&format!(
        "<message type=\"chat\" to=\"bo@{DOMAIN}\" id=\"w1\"><body>are you there?</body></message>"
    ))
    .await;
    assert!(ada.drain().await.iter().all(|f| !f.contains("<message")));
    let mut bo = login(port, "bo").await;
    let waiting = bo.expect("are you there?").await;
    assert!(waiting.contains(&format!("from=\"ada@{DOMAIN}\"")), "{waiting}");
    assert!(waiting.contains("<delay xmlns=\"urn:xmpp:delay\""), "{waiting}");

    // Two hubs of one region are one room (DD21)
    ada.send(&format!(
        "<presence to=\"City-a@muc.{DOMAIN}/{}\"><x xmlns=\"http://jabber.org/protocol/muc\"/></presence>",
        nick("Ada", "ada")
    ))
    .await;
    ada.expect("status code=\"110\"").await;
    bo.send(&format!(
        "<presence to=\"City-b@muc.{DOMAIN}/{}\"><x xmlns=\"http://jabber.org/protocol/muc\"/></presence>",
        nick("Bo", "bo")
    ))
    .await;
    bo.expect("status code=\"110\"").await;
    ada.send(&format!(
        "<message type=\"groupchat\" to=\"City-a@muc.{DOMAIN}\" id=\"m1\"><body>hello</body></message>"
    ))
    .await;
    let heard = bo.expect("type=\"groupchat\"").await;
    assert!(heard.contains("<body>hello</body>"), "{heard}");
    // The history is the library's own, beside the node database
    assert!(dir.path().join("chat.db").is_file());

    // A restart is a new instance of the library, given the loaded modules again
    controller.restart_component("chat").await.unwrap();
    let port = ready_port(&status, Some(port)).await;
    let mut cy = login(port, "cy").await;
    cy.close().await;
    controller.shutdown().await;
}

/// The port chat listens on once ready, other than `before`.
async fn ready_port(status: &service_kit::context::StatusRegistry, before: Option<u16>) -> u16 {
    for _ in 0..100 {
        if let Some(chat) = status.snapshot().get("chat")
            && chat.phase == ComponentPhase::Ready
            && let Some(port) = chat
                .listen_addr
                .as_deref()
                .and_then(|a| a.rsplit(':').next()?.parse::<u16>().ok())
            && Some(port) != before
        {
            return port;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("chat never ready");
}
