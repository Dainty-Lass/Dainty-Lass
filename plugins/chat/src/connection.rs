// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The listener: WebSocket connections handed to the chat task, the gateway check, and the loopback control API
//! (`/__chat/*`) kept for parity.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::Router;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, middleware};
use futures_util::{SinkExt as _, StreamExt as _};
use module_api::GlobalId;
use serde::Deserialize;
use serde_json::{Value, json};
use service_kit::serve::ConnectionInfo;
use sha2::{Digest as _, Sha256};
use store::Actor;
use subtle::ConstantTimeEq as _;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::{Notify, mpsc, oneshot};
use tracing::warn;

use crate::federation::Federation;
use crate::history::{History, Mute};
use crate::hooks::ChatHost;
use crate::server::{Event, FRAME_LIMIT, Out, Writer, now_ms};
use crate::settings::WS_PATHS;
use crate::xmpp::{DEFAULT_DOMAIN, NS_BIND, NS_FRAMING, NS_MUC, NS_SASL, escape};

pub const GATEWAY_HEADER: &str = "x-reforged-gateway";
/// How long a connection waits for its other half, or for the client to finish closing
const LINGER: Duration = Duration::from_secs(30);
const FLUSH: Duration = Duration::from_millis(200);

fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(IpAddr::V6(v6), IpAddr::V4),
        v4 => v4,
    }
}

fn same_secret(given: &[u8], secret: &str) -> bool {
    // Digests first, so the comparison leaks no length
    let (a, b) = (Sha256::digest(given), Sha256::digest(secret.as_bytes()));
    a.ct_eq(&b).into()
}

/// The player's address and whether the gateway named it: only a loopback peer holding the gateway's secret may.
pub fn client_address(peer: SocketAddr, headers: &HeaderMap, secret: Option<&str>) -> (IpAddr, &'static str) {
    let peer_ip = canonical(peer.ip());
    let from_gateway = secret.is_some_and(|secret| {
        peer_ip.is_loopback()
            && headers
                .get(GATEWAY_HEADER)
                .is_some_and(|value| same_secret(value.as_bytes(), secret))
    });
    if !from_gateway {
        return (peer_ip, "direct");
    }
    let forwarded = headers
        .get_all("x-forwarded-for")
        .iter()
        .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
        .collect::<Vec<_>>()
        .join(",");
    let player = forwarded
        .rsplit(',')
        .next()
        .and_then(|last| last.trim().parse::<IpAddr>().ok())
        .map_or(peer_ip, canonical);
    (player, "gateway")
}

/// What the listener's handlers share with `Running`.
pub(crate) struct Shared {
    pub(crate) events: mpsc::Sender<Event>,
    pub(crate) history: History,
    pub(crate) host: Arc<dyn ChatHost>,
    pub(crate) federation: Arc<dyn Federation>,
    pub(crate) secret: Option<String>,
    pub(crate) port: u16,
    clients: Mutex<HashMap<String, Arc<TestClient>>>,
}

impl Shared {
    pub(crate) fn new(
        events: mpsc::Sender<Event>,
        history: History,
        host: Arc<dyn ChatHost>,
        federation: Arc<dyn Federation>,
        secret: Option<String>,
        port: u16,
    ) -> Self {
        Self {
            events,
            history,
            host,
            federation,
            secret,
            port,
            clients: Mutex::default(),
        }
    }

    /// Stores the mute, tells the task, then the network. `at` is the real clock's.
    pub(crate) async fn mute(
        &self,
        account: &GlobalId,
        until: Option<i64>,
        reason: Option<String>,
        actor: Actor,
    ) -> Result<(), store::Error> {
        let mute = Mute {
            account: account.to_string(),
            until,
            reason,
            by: actor.as_text(),
            at: now_ms(),
        };
        self.history.set_mute(mute.clone(), actor.clone()).await?;
        _ = self
            .events
            .send(Event::Muted {
                account: account.clone(),
                mute: Some(mute.clone()),
            })
            .await;
        self.federation.set_mute(account, Some(mute), actor).await;
        Ok(())
    }

    pub(crate) async fn unmute(&self, account: &GlobalId, actor: Actor) -> Result<bool, store::Error> {
        let had = self.history.clear_mute(&account.to_string(), actor.clone()).await?;
        _ = self
            .events
            .send(Event::Muted {
                account: account.clone(),
                mute: None,
            })
            .await;
        self.federation.set_mute(account, None, actor).await;
        Ok(had)
    }

    pub(crate) async fn tick(&self) {
        let (done, wait) = oneshot::channel();
        if self.events.send(Event::Tick { done }).await.is_ok() {
            _ = wait.await;
        }
    }

    async fn resolve(&self, text: &str) -> Option<GlobalId> {
        if let Ok(id) = text.parse() {
            return Some(id);
        }
        if let Ok(Some(account)) = self.history.account_of(text).await
            && let Ok(id) = account.parse()
        {
            return Some(id);
        }
        if let Some(id) = self.host.account_of(text).await {
            return Some(id);
        }
        self.federation.resolve(text).await
    }

    fn client(&self, handle: &str) -> Option<Arc<TestClient>> {
        self.clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(handle)
            .cloned()
    }
}

pub(crate) fn router(shared: Arc<Shared>) -> Router {
    let control = Router::new()
        .route("/__chat/mute", post(mute))
        .route("/__chat/unmute", post(unmute))
        .route("/__chat/history/direct", get(history_direct))
        .route("/__chat/history/waiting", get(history_waiting))
        .route("/__chat/tick", post(tick))
        .route("/__chat/client/login", post(client_login))
        .route("/__chat/client/send", post(client_send))
        .route("/__chat/client/drain", get(client_drain))
        .route("/__chat/client/close", post(client_close))
        .route_layer(middleware::from_fn(loopback_only));
    control.fallback(upgrade).with_state(shared)
}

/// Nothing the gateway forwards reaches the control API.
async fn loopback_only(request: Request, next: Next) -> Response {
    let loopback = request
        .extensions()
        .get::<ConnectInfo<ConnectionInfo>>()
        .is_some_and(|info| canonical(info.0.peer.ip()).is_loopback());
    if !loopback || request.headers().contains_key(GATEWAY_HEADER) {
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(request).await
}

fn answer(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

fn error(status: StatusCode, text: impl Into<String>) -> Response {
    answer(status, json!({ "error": text.into() }))
}

async fn upgrade(
    State(shared): State<Arc<Shared>>,
    ConnectInfo(info): ConnectInfo<ConnectionInfo>,
    uri: Uri,
    headers: HeaderMap,
    ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    let Ok(ws) = ws else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !WS_PATHS.contains(&uri.path()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let (address, via) = client_address(info.peer, &headers, shared.secret.as_deref());
    let (reply, admitted) = oneshot::channel();
    let asked = shared.events.send(Event::Admit { address, reply }).await.is_ok();
    if !asked || !admitted.await.unwrap_or(false) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::CONNECTION, "close"), (header::CONTENT_LENGTH, "0")],
        )
            .into_response();
    }
    let offered = headers
        .get(header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').map(str::trim).find(|p| !p.is_empty()))
        .map(str::to_owned);
    let ws = match offered {
        Some(protocol) => ws.protocols([protocol]),
        None => ws,
    };
    ws.max_message_size(FRAME_LIMIT)
        .max_frame_size(FRAME_LIMIT)
        .on_failed_upgrade(|e| warn!("chat: WebSocket upgrade failed: {e}"))
        .on_upgrade(move |socket| connection(shared, socket, address, via))
}

fn too_large(error: &axum::Error) -> bool {
    let text = error.to_string();
    text.contains("too long") || text.contains("Space limit")
}

/// One upgraded connection: its frames to the task in order, the task's stanzas to the socket.
async fn connection(shared: Arc<Shared>, socket: WebSocket, address: IpAddr, via: &'static str) {
    let (writer, mut end) = Writer::new();
    let (reply, opened) = oneshot::channel();
    let open = Event::Open {
        address,
        via,
        writer,
        reply,
    };
    if shared.events.send(open).await.is_err() {
        end.mark_closed();
        return;
    }
    let Ok(id) = opened.await else {
        end.mark_closed();
        return;
    };
    let (mut sink, mut stream) = socket.split();
    let mut reading = true;
    let mut writing = true;
    let mut writer_done = false;
    let mut deadline: Option<tokio::time::Instant> = None;
    loop {
        if !reading && writer_done {
            break;
        }
        let linger = async {
            match deadline {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            message = stream.next(), if reading => {
                let gone = match message {
                    Some(Ok(Message::Text(text))) => {
                        let frame = Event::Frame { session: id, text: text.as_str().to_owned() };
                        shared.events.send(frame).await.is_err().then_some(false)
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        let text = String::from_utf8_lossy(&bytes).into_owned();
                        shared.events.send(Event::Frame { session: id, text }).await.is_err().then_some(false)
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => None,
                    Some(Ok(Message::Close(_))) | None => Some(false),
                    Some(Err(e)) => Some(too_large(&e)),
                };
                if let Some(too_large) = gone {
                    reading = false;
                    _ = shared.events.send(Event::Gone { session: id, too_large }).await;
                    deadline.get_or_insert_with(|| tokio::time::Instant::now() + LINGER);
                }
            }
            out = end.rx.recv(), if !writer_done => match out {
                Some(Out::Text(text)) => {
                    let length = text.len();
                    let sent = !writing || sink.send(Message::Text(text.into())).await.is_ok();
                    end.written(length);
                    if !sent {
                        break;
                    }
                }
                Some(Out::Close(code)) => {
                    if writing {
                        writing = false;
                        let frame = CloseFrame { code, reason: "".into() };
                        _ = sink.send(Message::Close(Some(frame))).await;
                        deadline.get_or_insert_with(|| tokio::time::Instant::now() + LINGER);
                    }
                }
                Some(Out::Cut) => break,
                None => {
                    writer_done = true;
                    // Nothing more will come; a connection that wasn't closing is done
                    if writing && reading {
                        break;
                    }
                }
            },
            () = linger => break,
        }
    }
    if reading {
        _ = shared
            .events
            .send(Event::Gone {
                session: id,
                too_large: false,
            })
            .await;
    }
    _ = tokio::time::timeout(FLUSH, sink.flush()).await;
    drop(sink);
    drop(stream);
    end.mark_closed();
}

#[derive(Deserialize)]
struct MuteBody {
    account: Option<String>,
    local: Option<String>,
    until: Option<i64>,
    reason: Option<String>,
    actor: Option<String>,
}

impl MuteBody {
    fn key(&self) -> &str {
        self.account.as_deref().or(self.local.as_deref()).unwrap_or_default()
    }

    fn actor(&self) -> Actor {
        Actor::Admin(self.actor.clone().unwrap_or_else(|| String::from("host")))
    }
}

async fn mute(State(shared): State<Arc<Shared>>, Json(body): Json<MuteBody>) -> Response {
    let Some(account) = shared.resolve(body.key()).await else {
        return error(StatusCode::NOT_FOUND, "account not found");
    };
    match shared
        .mute(&account, body.until, body.reason.clone(), body.actor())
        .await
    {
        Ok(()) => answer(StatusCode::OK, json!({ "ok": true, "account": account.to_string() })),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn unmute(State(shared): State<Arc<Shared>>, Json(body): Json<MuteBody>) -> Response {
    let Some(account) = shared.resolve(body.key()).await else {
        return error(StatusCode::NOT_FOUND, "account not found");
    };
    match shared.unmute(&account, body.actor()).await {
        Ok(had) => answer(
            StatusCode::OK,
            json!({ "ok": true, "account": account.to_string(), "had": had }),
        ),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

type Params = Query<HashMap<String, String>>;

fn param<'a>(params: &'a HashMap<String, String>, name: &str) -> Option<&'a str> {
    params.get(name).map(String::as_str).filter(|v| !v.is_empty())
}

/// An `i64` parameter, `default` when absent or unparsable, clamped.
fn number(params: &HashMap<String, String>, name: &str, default: i64, min: i64, max: i64) -> i64 {
    param(params, name)
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(default)
        .clamp(min, max)
}

async fn history_direct(State(shared): State<Arc<Shared>>, Query(params): Params) -> Response {
    let Some(a) = param(&params, "a") else {
        return error(StatusCode::BAD_REQUEST, "missing a");
    };
    let Some(b) = param(&params, "b") else {
        return error(StatusCode::BAD_REQUEST, "missing b");
    };
    let limit = number(&params, "limit", 50, 1, 200);
    let Some(a) = shared.resolve(a).await else {
        return error(StatusCode::NOT_FOUND, "account a not found");
    };
    let Some(b) = shared.resolve(b).await else {
        return error(StatusCode::NOT_FOUND, "account b not found");
    };
    match shared.history.direct_between(&a, &b, limit).await {
        Ok(messages) => answer(
            StatusCode::OK,
            json!({ "a": a.to_string(), "b": b.to_string(), "messages": messages }),
        ),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn history_waiting(State(shared): State<Arc<Shared>>, Query(params): Params) -> Response {
    let Some(key) = param(&params, "account").or_else(|| param(&params, "local")) else {
        return error(StatusCode::BAD_REQUEST, "missing account");
    };
    let Some(account) = shared.resolve(key).await else {
        return error(StatusCode::NOT_FOUND, "account not found");
    };
    match shared.history.pending_for(&account, 50).await {
        Ok(messages) => answer(
            StatusCode::OK,
            json!({ "account": account.to_string(), "messages": messages }),
        ),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn tick(State(shared): State<Arc<Shared>>) -> Response {
    shared.tick().await;
    answer(StatusCode::OK, json!({ "ok": true }))
}

#[derive(Deserialize)]
struct LoginBody {
    handle: String,
    authcid: String,
    token: String,
    resource: Option<String>,
}

async fn client_login(State(shared): State<Arc<Shared>>, Json(body): Json<LoginBody>) -> Response {
    let resource = body.resource.as_deref().unwrap_or("V0:Win:dauntless:0.1");
    match TestClient::login(shared.port, &body.authcid, &body.token, resource).await {
        Ok((client, frames)) => {
            let old = shared
                .clients
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(body.handle.clone(), Arc::new(client));
            if let Some(old) = old {
                old.stop();
            }
            answer(
                StatusCode::OK,
                json!({ "ok": true, "handle": body.handle, "frames": frames }),
            )
        }
        Err(e) => answer(StatusCode::BAD_REQUEST, json!({ "ok": false, "error": e })),
    }
}

#[derive(Deserialize)]
struct SendBody {
    handle: String,
    xml: Option<String>,
    whisper_to: Option<String>,
    join_room: Option<String>,
    name: Option<String>,
    say_room: Option<String>,
    body: Option<String>,
    wait_ms: Option<i64>,
}

async fn client_send(State(shared): State<Arc<Shared>>, Json(body): Json<SendBody>) -> Response {
    let Some(client) = shared.client(&body.handle) else {
        return error(StatusCode::NOT_FOUND, "unknown client handle");
    };
    let text = escape(body.body.as_deref().unwrap_or_default());
    // Room and target go in as given: a test may want any of them
    let stanza = if let Some(xml) = &body.xml {
        xml.clone()
    } else if let Some(to) = &body.whisper_to {
        format!(r#"<message type="chat" to="{to}@{DEFAULT_DOMAIN}" id="w1"><body>{text}</body></message>"#)
    } else if let Some(room) = &body.join_room {
        let name = body.name.as_deref().unwrap_or(&client.authcid);
        let nick = escape(&format!("{name}:{}:{}", client.authcid, client.resource));
        format!(r#"<presence to="{room}@muc.{DEFAULT_DOMAIN}/{nick}"><x xmlns="{NS_MUC}"/></presence>"#)
    } else if let Some(room) = &body.say_room {
        format!(r#"<message type="groupchat" to="{room}@muc.{DEFAULT_DOMAIN}" id="m1"><body>{text}</body></message>"#)
    } else {
        return error(StatusCode::BAD_REQUEST, "missing stanza action");
    };
    if let Err(e) = client.send(&stanza).await {
        return error(StatusCode::INTERNAL_SERVER_ERROR, e);
    }
    let wait = body.wait_ms.unwrap_or(200).clamp(10, 5000);
    tokio::time::sleep(Duration::from_millis(wait.unsigned_abs())).await;
    answer(StatusCode::OK, json!({ "ok": true, "frames": client.take() }))
}

async fn client_drain(State(shared): State<Arc<Shared>>, Query(params): Params) -> Response {
    let Some(handle) = param(&params, "handle") else {
        return error(StatusCode::BAD_REQUEST, "missing handle");
    };
    let Some(client) = shared.client(handle) else {
        return error(StatusCode::NOT_FOUND, "unknown client handle");
    };
    if let Some(expect) = param(&params, "expect") {
        let timeout = number(&params, "timeout_ms", 5000, 50, 15000);
        let found = client
            .wait_for(&[expect], Duration::from_millis(timeout.unsigned_abs()))
            .await;
        let frames = client.take();
        return match found {
            Ok(_) => answer(StatusCode::OK, json!({ "ok": true, "matched": true, "frames": frames })),
            Err(e) => answer(
                StatusCode::OK,
                json!({ "ok": false, "matched": false, "error": e, "frames": frames }),
            ),
        };
    }
    let wait = number(&params, "wait_ms", 200, 10, 5000);
    tokio::time::sleep(Duration::from_millis(wait.unsigned_abs())).await;
    answer(StatusCode::OK, json!({ "ok": true, "frames": client.take() }))
}

#[derive(Deserialize)]
struct CloseBody {
    handle: String,
}

async fn client_close(State(shared): State<Arc<Shared>>, Json(body): Json<CloseBody>) -> Response {
    let client = shared
        .clients
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&body.handle);
    if let Some(client) = client {
        _ = client.send(&format!(r#"<close xmlns="{NS_FRAMING}"/>"#)).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        client.stop();
    }
    answer(StatusCode::OK, json!({ "ok": true }))
}

/// A WebSocket XMPP client on this chat, for the control API.
struct TestClient {
    authcid: String,
    resource: String,
    write: tokio::sync::Mutex<OwnedWriteHalf>,
    frames: Arc<Mutex<Vec<String>>>,
    arrived: Arc<Notify>,
    reader: tokio::task::JoinHandle<()>,
}

const KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";

impl TestClient {
    async fn login(port: u16, authcid: &str, token: &str, resource: &str) -> Result<(Self, Vec<String>), String> {
        let stream = TcpStream::connect(("127.0.0.1", port))
            .await
            .map_err(|e| format!("connect: {e}"))?;
        _ = stream.set_nodelay(true);
        let (mut read, mut write) = stream.into_split();
        let request = format!(
            "GET // HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Key: {KEY}\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: xmpp\r\n\r\n"
        );
        write
            .write_all(request.as_bytes())
            .await
            .map_err(|e| format!("write: {e}"))?;
        let mut head = Vec::new();
        let mut byte = [0_u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            if head.len() > 16 * 1024 || read.read(&mut byte).await.map_err(|e| format!("read: {e}"))? == 0 {
                return Err(String::from("upgrade refused: connection closed"));
            }
            head.push(byte[0]);
        }
        let head = String::from_utf8_lossy(&head);
        let status = head.lines().next().unwrap_or_default();
        if !status.starts_with("HTTP/1.1 101") {
            return Err(format!("upgrade refused: {status}"));
        }
        let frames: Arc<Mutex<Vec<String>>> = Arc::default();
        let arrived = Arc::new(Notify::new());
        let reader = tokio::spawn(read_frames(read, frames.clone(), arrived.clone()));
        let client = Self {
            authcid: authcid.to_owned(),
            resource: resource.to_owned(),
            write: tokio::sync::Mutex::new(write),
            frames,
            arrived,
            reader,
        };
        let open = format!(r#"<open xmlns="{NS_FRAMING}" to="{DEFAULT_DOMAIN}" version="1.0"/>"#);
        client.send(&open).await?;
        client.wait_for(&["<stream:features"], Duration::from_secs(5)).await?;
        use base64::Engine as _;
        let payload = base64::engine::general_purpose::STANDARD.encode(format!("\0{authcid}\0{token}"));
        client
            .send(&format!(
                r#"<auth xmlns="{NS_SASL}" mechanism="PLAIN">{payload}</auth>"#
            ))
            .await?;
        let at = client
            .wait_for(&["<success", "<failure"], Duration::from_secs(10))
            .await?;
        if client.frame(at).contains("<failure") {
            return Err(format!("SASL authentication failed: {:?}", client.all()));
        }
        client.send(&open).await?;
        let bind = format!(
            r#"<iq type="set" id="b1"><bind xmlns="{NS_BIND}"><resource>{}</resource></bind></iq>"#,
            escape(resource)
        );
        client.send(&bind).await?;
        let at = client
            .wait_for(&["<jid>", r#"type="error""#], Duration::from_secs(10))
            .await?;
        if !client.frame(at).contains("<jid>") {
            return Err(format!("bind failed: {:?}", client.all()));
        }
        client.send("<presence/>").await?;
        tokio::time::sleep(Duration::from_millis(250)).await;
        let frames = {
            let mut all = client.frames.lock().unwrap_or_else(PoisonError::into_inner);
            let at = at.min(all.len());
            let rest = all.split_off(at);
            all.clear();
            rest
        };
        Ok((client, frames))
    }

    fn frame(&self, at: usize) -> String {
        self.all().get(at).cloned().unwrap_or_default()
    }

    fn all(&self) -> Vec<String> {
        self.frames.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.frames.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// The index of the first received frame holding any of `needles`.
    async fn wait_for(&self, needles: &[&str], timeout: Duration) -> Result<usize, String> {
        let until = tokio::time::Instant::now() + timeout;
        loop {
            let notified = self.arrived.notified();
            if let Some(at) = self.all().iter().position(|f| needles.iter().any(|n| f.contains(n))) {
                return Ok(at);
            }
            if tokio::time::timeout_at(until, notified).await.is_err() {
                return Err(format!("timed out waiting for {needles:?}; got {:?}", self.all()));
            }
        }
    }

    /// One masked text frame.
    async fn send(&self, text: &str) -> Result<(), String> {
        let payload = text.as_bytes();
        let mut frame = vec![0x81_u8];
        match payload.len() {
            n if n < 126 => frame.push(0x80 | u8::try_from(n).unwrap_or(0)),
            n if n <= usize::from(u16::MAX) => {
                frame.push(0x80 | 126);
                frame.extend_from_slice(&u16::try_from(n).unwrap_or(0).to_be_bytes());
            }
            n => {
                frame.push(0x80 | 127);
                frame.extend_from_slice(&(n as u64).to_be_bytes());
            }
        }
        let mask: [u8; 4] = rand::random();
        frame.extend_from_slice(&mask);
        frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        self.write
            .lock()
            .await
            .write_all(&frame)
            .await
            .map_err(|e| format!("write: {e}"))
    }

    fn stop(&self) {
        self.reader.abort();
    }
}

impl Drop for TestClient {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

/// Text and binary frames as lossy UTF-8, until a close frame or the end.
async fn read_frames(mut read: tokio::net::tcp::OwnedReadHalf, frames: Arc<Mutex<Vec<String>>>, arrived: Arc<Notify>) {
    let mut message = Vec::new();
    loop {
        let mut head = [0_u8; 2];
        if read.read_exact(&mut head).await.is_err() {
            break;
        }
        let (fin, opcode) = (head[0] & 0x80 != 0, head[0] & 0x0f);
        let length = match head[1] & 0x7f {
            126 => {
                let mut n = [0_u8; 2];
                if read.read_exact(&mut n).await.is_err() {
                    break;
                }
                u64::from(u16::from_be_bytes(n))
            }
            127 => {
                let mut n = [0_u8; 8];
                if read.read_exact(&mut n).await.is_err() {
                    break;
                }
                u64::from_be_bytes(n)
            }
            n => u64::from(n),
        };
        let mut mask = [0_u8; 4];
        if head[1] & 0x80 != 0 && read.read_exact(&mut mask).await.is_err() {
            break;
        }
        let Ok(length) = usize::try_from(length) else { break };
        if length > 4 * FRAME_LIMIT {
            break;
        }
        let mut payload = vec![0_u8; length];
        if read.read_exact(&mut payload).await.is_err() {
            break;
        }
        if head[1] & 0x80 != 0 {
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= mask[i % 4];
            }
        }
        match opcode {
            0x8 => break,
            0x0..=0x2 => {
                message.extend_from_slice(&payload);
                if fin {
                    let text = String::from_utf8_lossy(&message).into_owned();
                    message.clear();
                    frames.lock().unwrap_or_else(PoisonError::into_inner).push(text);
                    arrived.notify_waiters();
                }
            }
            _ => {}
        }
    }
    arrived.notify_waiters();
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn only_the_gateway_names_the_player() {
        let loopback: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        let mapped: SocketAddr = "[::ffff:127.0.0.1]:5000".parse().unwrap();
        let lan: SocketAddr = "192.168.1.9:5000".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(GATEWAY_HEADER, "s3cret".parse().unwrap());
        headers.insert("x-forwarded-for", "1.2.3.4, 203.0.113.7".parse().unwrap());
        let player: IpAddr = "203.0.113.7".parse().unwrap();
        assert_eq!(client_address(loopback, &headers, Some("s3cret")), (player, "gateway"));
        assert_eq!(client_address(mapped, &headers, Some("s3cret")), (player, "gateway"));
        assert_eq!(
            client_address(loopback, &headers, Some("other")),
            (loopback.ip(), "direct")
        );
        assert_eq!(client_address(lan, &headers, Some("s3cret")), (lan.ip(), "direct"));
        assert_eq!(client_address(loopback, &headers, None), (loopback.ip(), "direct"));
        let mut bare = headers.clone();
        bare.remove(GATEWAY_HEADER);
        assert_eq!(
            client_address(loopback, &bare, Some("s3cret")),
            (loopback.ip(), "direct")
        );

        let mut two = headers.clone();
        two.append("x-forwarded-for", "::ffff:198.51.100.2".parse().unwrap());
        let last: IpAddr = "198.51.100.2".parse().unwrap();
        assert_eq!(client_address(loopback, &two, Some("s3cret")), (last, "gateway"));
        let mut junk = headers;
        junk.insert("x-forwarded-for", "nonsense".parse().unwrap());
        assert_eq!(
            client_address(loopback, &junk, Some("s3cret")),
            (loopback.ip(), "gateway")
        );
        assert_eq!(canonical(mapped.ip()), loopback.ip());
    }
}
