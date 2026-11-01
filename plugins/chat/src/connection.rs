//! From Dauntless Revived's metagame chat at `e1c5fec` (`services/metagame/src/chat/connection.rs`), with
//! DaintyLass's changes: a WebSocket upgrade is a chat connection only on the paths the clients ask for
//! (`settings::WS_PATHS`, as Mystic Paradox's gateway allows them; DR took any path on its own port), and the
//! gateway is recognised by its secret here rather than by the metagame's `origin.rs`, which stays DR's.
//!
//! The chat listener (`ChatServer` in `realtime/chat.ts`): HTTP on its own loopback port, where an upgrade
//! becomes a chat connection and any other request is a 404. Each connection has a reader, which hands its
//! frames to the chat task in order, and a writer, which sends what the chat task gives it and counts what
//! still waits (Node's `bufferedAmount`).

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::Json;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade, rejection::WebSocketUpgradeRejection};
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use module_api::GlobalId;
use serde::Deserialize;
use serde_json::json;
use service_kit::serve::ConnectionInfo;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{Mutex as AsyncMutex, Notify, mpsc, oneshot};

use crate::federation::Federation;
use crate::history::{History, Mute};
use crate::hooks::ChatHost;
use crate::server::{Event, FRAME_LIMIT, Out, Outbox, now_ms};
use crate::settings::WS_PATHS;
use crate::xmpp::{DEFAULT_DOMAIN, escape};

/// The gateway's header on everything it forwards (DR's `X-Reforged-Gateway`).
pub const GATEWAY_HEADER: &str = "x-reforged-gateway";

/// How long a close we sent may wait for the client's answer before the connection is cut (ws's
/// closeTimeout).
const CLOSE_WAIT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct Listener {
    pub port: u16,
    pub events: mpsc::UnboundedSender<Event>,
    pub gateway_secret: Option<Arc<str>>,
    pub history: History,
    pub federation: Arc<dyn Federation>,
    pub host: Arc<dyn ChatHost>,
    pub clients: Arc<Mutex<BTreeMap<String, Arc<LoopbackWsClient>>>>,
}

/// The player's address and how they came: through the gateway (from loopback with its secret, the address
/// it wrote in `X-Forwarded-For`) or straight to this port (the peer's).
pub fn client_address(peer: SocketAddr, headers: &HeaderMap, secret: Option<&str>) -> (IpAddr, &'static str) {
    let peer_ip = peer.ip().to_canonical();
    let trusted = secret.is_some_and(|secret| {
        let presented = headers.get(GATEWAY_HEADER).map(|v| v.as_bytes()).unwrap_or_default();
        let (a, b): ([u8; 32], [u8; 32]) = (Sha256::digest(presented).into(), Sha256::digest(secret).into());
        peer_ip.is_loopback() && headers.contains_key(GATEWAY_HEADER) && bool::from(a.ct_eq(&b))
    });
    if !trusted {
        return (peer_ip, "direct");
    }
    // The gateway overwrites the header with the one address it trusts: the right-most entry
    let forwarded = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .next_back()
        .and_then(|v| v.trim().parse::<IpAddr>().ok())
        .map(|ip| ip.to_canonical());
    (forwarded.unwrap_or(peer_ip), "gateway")
}

pub async fn upgrade(
    State(listener): State<Listener>,
    ConnectInfo(connection): ConnectInfo<ConnectionInfo>,
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
    let (address, via) = client_address(connection.peer, &headers, listener.gateway_secret.as_deref());
    let address = address.to_string();
    let (reply, admitted) = oneshot::channel();
    let asked = listener.events.send(Event::Admit {
        address: address.clone(),
        via,
        reply,
    });
    if asked.is_err() || !admitted.await.unwrap_or(false) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::CONNECTION, "close"), (header::CONTENT_LENGTH, "0")],
        )
            .into_response();
    }
    // ws answers with the first subprotocol the client offers (the game offers "xmpp")
    let offered = headers
        .get(header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').map(str::trim).find(|p| !p.is_empty()))
        .map(str::to_owned);
    let ws = ws.max_message_size(FRAME_LIMIT).max_frame_size(FRAME_LIMIT);
    let ws = match offered {
        Some(protocol) => ws.protocols([protocol]),
        None => ws,
    };
    let events = listener.events.clone();
    ws.on_failed_upgrade(|error| tracing::warn!("chat: upgrade failed ({error})"))
        .on_upgrade(move |socket| connection_task(socket, events, address, via))
}

async fn connection_task(socket: WebSocket, events: mpsc::UnboundedSender<Event>, address: String, via: &'static str) {
    let (sender, mut outgoing) = mpsc::unbounded_channel::<Out>();
    let queued = Arc::new(AtomicUsize::new(0));
    let closed = Arc::new(AtomicBool::new(false));
    let terminate = Arc::new(Notify::new());
    let out = Outbox {
        sender,
        queued: queued.clone(),
        closed: closed.clone(),
        terminate: terminate.clone(),
    };
    let (reply, id) = oneshot::channel();
    if events
        .send(Event::Connected {
            address,
            via,
            out,
            reply,
        })
        .is_err()
    {
        return;
    }
    let Ok(id) = id.await else {
        return;
    };
    let (mut sink, mut stream) = socket.split();
    let cut = terminate.clone();
    let writer = tokio::spawn(async move {
        while let Some(out) = outgoing.recv().await {
            match out {
                Out::Text(text) => {
                    let length = text.len();
                    let sent = sink.send(Message::Text(text.into())).await;
                    queued.fetch_sub(length, Ordering::Relaxed);
                    if sent.is_err() {
                        break;
                    }
                }
                Out::Close(code) => {
                    let _ = sink
                        .send(Message::Close(Some(CloseFrame {
                            code,
                            reason: "".into(),
                        })))
                        .await;
                    // The client answers, or the connection is cut after a while
                    let later = cut.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(CLOSE_WAIT).await;
                        later.notify_one();
                    });
                    break;
                }
                Out::Terminate => {
                    cut.notify_one();
                    break;
                }
            }
        }
        // A close the client sent is answered before the socket goes (tungstenite queued the
        // reply), as ws answers one
        let _ = tokio::time::timeout(Duration::from_millis(200), sink.flush()).await;
        sink
    });
    loop {
        tokio::select! {
            message = stream.next() => match message {
                Some(Ok(Message::Text(text))) => {
                    let _ = events.send(Event::Frame { id, text: text.to_string() });
                }
                // Text and binary frames alike are read as UTF-8 (the game sends text)
                Some(Ok(Message::Binary(bytes))) => {
                    let _ = events.send(Event::Frame { id, text: String::from_utf8_lossy(&bytes).into_owned() });
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                Some(Ok(Message::Close(_))) | None => {
                    let _ = events.send(Event::Gone { id, size: false });
                    break;
                }
                Some(Err(error)) => {
                    let size = is_too_long(&error);
                    let _ = events.send(Event::Gone { id, size });
                    break;
                }
            },
            () = terminate.notified() => break,
        }
    }
    // The writer goes once the chat task has ended the session; then the connection closes
    if let Ok(Ok(sink)) = tokio::time::timeout(CLOSE_WAIT, writer).await {
        drop(sink);
    }
    closed.store(true, Ordering::Relaxed);
}

/// An oversized frame or message (ws's `WS_ERR_UNSUPPORTED_MESSAGE_LENGTH`).
fn is_too_long(error: &axum::Error) -> bool {
    let text = error.to_string().to_lowercase();
    text.contains("too long") || text.contains("too big") || text.contains("capacity")
}

fn is_direct_loopback(peer: SocketAddr, headers: &HeaderMap) -> bool {
    peer.ip().to_canonical().is_loopback() && !headers.contains_key(GATEWAY_HEADER)
}

async fn resolve_target(listener: &Listener, raw: &str) -> Option<GlobalId> {
    if let Ok(id) = raw.parse::<GlobalId>() {
        return Some(id);
    }
    if let Ok(Some(record)) = listener.history.account_of(raw).await
        && let Ok(id) = record.parse::<GlobalId>()
    {
        return Some(id);
    }
    if let Some(id) = listener.host.account_of(raw).await {
        return Some(id);
    }
    listener.federation.resolve(raw).await
}

#[derive(Deserialize)]
pub struct MuteBody {
    pub account: Option<String>,
    pub local: Option<String>,
    pub until: Option<i64>,
    pub reason: Option<String>,
    pub actor: Option<String>,
}

pub async fn mute_account(
    State(listener): State<Listener>,
    ConnectInfo(connection): ConnectInfo<ConnectionInfo>,
    headers: HeaderMap,
    Json(body): Json<MuteBody>,
) -> Response {
    if !is_direct_loopback(connection.peer, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let key = body.account.as_deref().or(body.local.as_deref()).unwrap_or("");
    let Some(account) = resolve_target(&listener, key).await else {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "account not found" }))).into_response();
    };
    let actor = store::Actor::Admin(body.actor.unwrap_or_else(|| "host".into()));
    let mute = Mute {
        account: account.to_string(),
        until: body.until,
        reason: body.reason,
        by: actor.as_text(),
        at: now_ms(),
    };
    if let Err(error) = listener.history.set_mute(mute.clone(), actor.clone()).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error.to_string() })),
        )
            .into_response();
    }
    let _ = listener.events.send(Event::Muted {
        account: account.clone(),
        mute: Some(mute.clone()),
    });
    listener.federation.set_mute(&account, Some(mute), actor).await;
    (
        StatusCode::OK,
        Json(json!({ "ok": true, "account": account.to_string() })),
    )
        .into_response()
}

pub async fn unmute_account(
    State(listener): State<Listener>,
    ConnectInfo(connection): ConnectInfo<ConnectionInfo>,
    headers: HeaderMap,
    Json(body): Json<MuteBody>,
) -> Response {
    if !is_direct_loopback(connection.peer, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let key = body.account.as_deref().or(body.local.as_deref()).unwrap_or("");
    let Some(account) = resolve_target(&listener, key).await else {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "account not found" }))).into_response();
    };
    let actor = store::Actor::Admin(body.actor.unwrap_or_else(|| "host".into()));
    let had = match listener.history.clear_mute(&account.to_string(), actor.clone()).await {
        Ok(had) => had,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": error.to_string() })),
            )
                .into_response();
        }
    };
    let _ = listener.events.send(Event::Muted {
        account: account.clone(),
        mute: None,
    });
    listener.federation.set_mute(&account, None, actor).await;
    (
        StatusCode::OK,
        Json(json!({ "ok": true, "account": account.to_string(), "had": had })),
    )
        .into_response()
}

pub async fn direct_history(
    State(listener): State<Listener>,
    ConnectInfo(connection): ConnectInfo<ConnectionInfo>,
    headers: HeaderMap,
    Query(params): Query<BTreeMap<String, String>>,
) -> Response {
    if !is_direct_loopback(connection.peer, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(raw_a) = params.get("a") else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "missing a" }))).into_response();
    };
    let Some(raw_b) = params.get("b") else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "missing b" }))).into_response();
    };
    let limit = params
        .get("limit")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(50)
        .clamp(1, 200);
    let Some(a) = resolve_target(&listener, raw_a).await else {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "account a not found" }))).into_response();
    };
    let Some(b) = resolve_target(&listener, raw_b).await else {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "account b not found" }))).into_response();
    };
    match listener
        .history
        .direct_between(&a.to_string(), &b.to_string(), limit)
        .await
    {
        Ok(messages) => (
            StatusCode::OK,
            Json(json!({
                "a": a.to_string(),
                "b": b.to_string(),
                "messages": messages,
            })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

pub async fn waiting_history(
    State(listener): State<Listener>,
    ConnectInfo(connection): ConnectInfo<ConnectionInfo>,
    headers: HeaderMap,
    Query(params): Query<BTreeMap<String, String>>,
) -> Response {
    if !is_direct_loopback(connection.peer, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(raw) = params.get("account").or_else(|| params.get("local")) else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "missing account" }))).into_response();
    };
    let Some(account) = resolve_target(&listener, raw).await else {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "account not found" }))).into_response();
    };
    match listener.history.pending_for(&account.to_string(), 50).await {
        Ok(messages) => (
            StatusCode::OK,
            Json(json!({
                "account": account.to_string(),
                "messages": messages,
            })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

pub async fn tick_chat(
    State(listener): State<Listener>,
    ConnectInfo(connection): ConnectInfo<ConnectionInfo>,
    headers: HeaderMap,
) -> Response {
    if !is_direct_loopback(connection.peer, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let (done, finished) = oneshot::channel();
    if listener.events.send(Event::Tick(done)).is_ok() {
        let _ = finished.await;
    }
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}

const DEFAULT_RESOURCE: &str = "V0:Win:dauntless:0.1";

pub struct LoopbackWsClient {
    authcid: String,
    resource: String,
    writer: AsyncMutex<OwnedWriteHalf>,
    inbox: Arc<Mutex<Vec<String>>>,
    arrived: Arc<Notify>,
    reader_task: tokio::task::JoinHandle<()>,
}

fn base64_encode(bytes: &[u8]) -> String {
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

async fn read_exact_buffered(reader: &mut OwnedReadHalf, buf: &mut Vec<u8>, need: usize) -> std::io::Result<Vec<u8>> {
    while buf.len() < need {
        let mut chunk = [0u8; 4096];
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "websocket closed",
            ));
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Ok(buf.drain(..need).collect())
}

async fn read_ws_loop(
    mut reader: OwnedReadHalf,
    mut buf: Vec<u8>,
    inbox: Arc<Mutex<Vec<String>>>,
    arrived: Arc<Notify>,
) {
    loop {
        let Ok(hdr) = read_exact_buffered(&mut reader, &mut buf, 2).await else {
            break;
        };
        let opcode = hdr[0] & 0x0f;
        let masked = (hdr[1] & 0x80) != 0;
        let mut len = usize::from(hdr[1] & 0x7f);
        if len == 126 {
            let Ok(ext) = read_exact_buffered(&mut reader, &mut buf, 2).await else {
                break;
            };
            len = usize::from(u16::from_be_bytes([ext[0], ext[1]]));
        } else if len == 127 {
            let Ok(ext) = read_exact_buffered(&mut reader, &mut buf, 8).await else {
                break;
            };
            let Ok(val) = usize::try_from(u64::from_be_bytes([
                ext[0], ext[1], ext[2], ext[3], ext[4], ext[5], ext[6], ext[7],
            ])) else {
                break;
            };
            len = val;
        }
        let mask = if masked {
            let Ok(m) = read_exact_buffered(&mut reader, &mut buf, 4).await else {
                break;
            };
            Some([m[0], m[1], m[2], m[3]])
        } else {
            None
        };
        let Ok(mut payload) = read_exact_buffered(&mut reader, &mut buf, len).await else {
            break;
        };
        if let Some(m) = mask {
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= m[i & 3];
            }
        }
        match opcode {
            0x1 | 0x2 => {
                let text = String::from_utf8_lossy(&payload).into_owned();
                inbox.lock().unwrap_or_else(PoisonError::into_inner).push(text);
                arrived.notify_waiters();
            }
            0x8 => break,
            _ => {}
        }
    }
    arrived.notify_waiters();
}

async fn send_ws_text(writer: &mut OwnedWriteHalf, text: &str) -> Result<(), String> {
    let bytes = text.as_bytes();
    let mut frame = Vec::with_capacity(14 + bytes.len());
    frame.push(0x81);
    if bytes.len() < 126 {
        frame.push(0x80 | u8::try_from(bytes.len()).unwrap_or(0));
    } else if let Ok(len16) = u16::try_from(bytes.len()) {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&len16.to_be_bytes());
    } else {
        frame.push(0x80 | 127);
        frame.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    }
    frame.extend_from_slice(&[0, 0, 0, 0]);
    frame.extend_from_slice(bytes);
    writer.write_all(&frame).await.map_err(|e| e.to_string())?;
    writer.flush().await.map_err(|e| e.to_string())
}

impl LoopbackWsClient {
    async fn connect_and_login(
        port: u16,
        authcid: &str,
        token: &str,
        resource: &str,
    ) -> Result<(Arc<Self>, Vec<String>), String> {
        let stream = TcpStream::connect(SocketAddr::from(([127, 0, 0, 1], port)))
            .await
            .map_err(|e| format!("connect 127.0.0.1:{port}: {e}"))?;
        let _ = stream.set_nodelay(true);
        let (mut reader, mut writer) = stream.into_split();
        let req = concat!(
            "GET // HTTP/1.1\r\n",
            "Host: 127.0.0.1\r\n",
            "Upgrade: websocket\r\n",
            "Connection: Upgrade\r\n",
            "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n",
            "Sec-WebSocket-Version: 13\r\n",
            "Sec-WebSocket-Protocol: xmpp\r\n\r\n"
        );
        writer
            .write_all(req.as_bytes())
            .await
            .map_err(|e| format!("write upgrade: {e}"))?;
        writer.flush().await.map_err(|e| format!("flush upgrade: {e}"))?;
        let mut buf = Vec::new();
        let header_end = loop {
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
            let mut chunk = [0u8; 1024];
            let n = reader
                .read(&mut chunk)
                .await
                .map_err(|e| format!("read upgrade: {e}"))?;
            if n == 0 {
                return Err("closed during upgrade".into());
            }
            buf.extend_from_slice(&chunk[..n]);
        };
        let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
        if !head.starts_with("HTTP/1.1 101") {
            return Err(format!("upgrade refused: {}", head.lines().next().unwrap_or("")));
        }
        let leftover = buf.split_off(header_end);
        let inbox = Arc::new(Mutex::new(Vec::new()));
        let arrived = Arc::new(Notify::new());
        let reader_task = tokio::spawn(read_ws_loop(reader, leftover, inbox.clone(), arrived.clone()));
        let client = Arc::new(Self {
            authcid: authcid.to_owned(),
            resource: resource.to_owned(),
            writer: AsyncMutex::new(writer),
            inbox,
            arrived,
            reader_task,
        });
        client
            .send_raw(&format!(
                "<open xmlns=\"urn:ietf:params:xml:ns:xmpp-framing\" to=\"{DEFAULT_DOMAIN}\" version=\"1.0\"/>"
            ))
            .await?;
        client.wait_for("<stream:features", Duration::from_secs(5)).await?;
        let _ = client.take_all();
        let plain = base64_encode(format!("\0{authcid}\0{token}").as_bytes());
        client
            .send_raw(&format!(
                "<auth xmlns=\"urn:ietf:params:xml:ns:xmpp-sasl\" mechanism=\"PLAIN\">{plain}</auth>"
            ))
            .await?;
        let auth_frames = client
            .wait_for_any(&["<success", "<failure"], Duration::from_secs(10))
            .await?;
        if !auth_frames.iter().any(|f| f.contains("<success")) {
            return Err(format!("SASL authentication failed: {auth_frames:?}"));
        }
        client
            .send_raw(&format!(
                "<open xmlns=\"urn:ietf:params:xml:ns:xmpp-framing\" to=\"{DEFAULT_DOMAIN}\" version=\"1.0\"/>"
            ))
            .await?;
        client.wait_for("<stream:features", Duration::from_secs(5)).await?;
        let _ = client.take_all();
        client
            .send_raw(&format!(
                "<iq type=\"set\" id=\"b1\"><bind xmlns=\"urn:ietf:params:xml:ns:xmpp-bind\"><resource>{}</resource></bind></iq>",
                escape(resource)
            ))
            .await?;
        let bind_frames = client
            .wait_for_any(&["<jid>", "type=\"error\""], Duration::from_secs(10))
            .await?;
        if !bind_frames.iter().any(|f| f.contains("<jid>")) {
            return Err(format!("bind failed: {bind_frames:?}"));
        }
        client.send_raw("<presence/>").await?;
        tokio::time::sleep(Duration::from_millis(250)).await;
        let mut initial = bind_frames;
        initial.extend(client.take_all());
        Ok((client, initial))
    }

    async fn send_raw(&self, xml: &str) -> Result<(), String> {
        let mut writer = self.writer.lock().await;
        send_ws_text(&mut writer, xml).await
    }

    fn take_all(&self) -> Vec<String> {
        std::mem::take(&mut *self.inbox.lock().unwrap_or_else(PoisonError::into_inner))
    }

    async fn wait_for(&self, needle: &str, timeout: Duration) -> Result<Vec<String>, String> {
        self.wait_for_any(&[needle], timeout).await
    }

    async fn wait_for_any(&self, needles: &[&str], timeout: Duration) -> Result<Vec<String>, String> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let matched = {
                let guard = self.inbox.lock().unwrap_or_else(PoisonError::into_inner);
                guard.iter().any(|f| needles.iter().any(|n| f.contains(n)))
            };
            if matched {
                return Ok(self.take_all());
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                let got = self.take_all();
                return Err(format!("timed out waiting for {needles:?}; got {got:?}"));
            }
            let _ = tokio::time::timeout(deadline - now, self.arrived.notified()).await;
        }
    }
}

impl Drop for LoopbackWsClient {
    fn drop(&mut self) {
        self.reader_task.abort();
    }
}

#[derive(Deserialize)]
pub struct ClientLoginBody {
    pub handle: String,
    pub authcid: String,
    pub token: String,
    pub resource: Option<String>,
}

pub async fn client_login(
    State(listener): State<Listener>,
    ConnectInfo(connection): ConnectInfo<ConnectionInfo>,
    headers: HeaderMap,
    Json(body): Json<ClientLoginBody>,
) -> Response {
    if !is_direct_loopback(connection.peer, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let resource = body.resource.as_deref().unwrap_or(DEFAULT_RESOURCE);
    match LoopbackWsClient::connect_and_login(listener.port, &body.authcid, &body.token, resource).await {
        Ok((client, frames)) => {
            listener
                .clients
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(body.handle.clone(), client);
            (
                StatusCode::OK,
                Json(json!({
                    "ok": true,
                    "handle": body.handle,
                    "frames": frames,
                })),
            )
                .into_response()
        }
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({ "ok": false, "error": error }))).into_response(),
    }
}

#[derive(Deserialize)]
pub struct ClientSendBody {
    pub handle: String,
    pub xml: Option<String>,
    pub whisper_to: Option<String>,
    pub join_room: Option<String>,
    pub name: Option<String>,
    pub say_room: Option<String>,
    pub body: Option<String>,
    pub wait_ms: Option<u64>,
}

pub async fn client_send(
    State(listener): State<Listener>,
    ConnectInfo(connection): ConnectInfo<ConnectionInfo>,
    headers: HeaderMap,
    Json(req): Json<ClientSendBody>,
) -> Response {
    if !is_direct_loopback(connection.peer, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let client = listener
        .clients
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&req.handle)
        .cloned();
    let Some(client) = client else {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "unknown client handle" }))).into_response();
    };
    let stanza = if let Some(xml) = req.xml {
        xml
    } else if let Some(to) = req.whisper_to {
        let text = escape(req.body.as_deref().unwrap_or(""));
        format!("<message type=\"chat\" to=\"{to}@{DEFAULT_DOMAIN}\" id=\"w1\"><body>{text}</body></message>")
    } else if let Some(room) = req.join_room {
        let name = req.name.as_deref().unwrap_or(&client.authcid);
        let nick = format!("{name}:{}:{}", client.authcid, client.resource);
        format!(
            "<presence to=\"{room}@muc.{DEFAULT_DOMAIN}/{}\"><x xmlns=\"http://jabber.org/protocol/muc\"/></presence>",
            escape(&nick)
        )
    } else if let Some(room) = req.say_room {
        let text = escape(req.body.as_deref().unwrap_or(""));
        format!(
            "<message type=\"groupchat\" to=\"{room}@muc.{DEFAULT_DOMAIN}\" id=\"m1\"><body>{text}</body></message>"
        )
    } else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "missing stanza action" })),
        )
            .into_response();
    };
    if let Err(error) = client.send_raw(&stanza).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": error }))).into_response();
    }
    let wait_ms = req.wait_ms.unwrap_or(200).clamp(10, 5000);
    tokio::time::sleep(Duration::from_millis(wait_ms)).await;
    let frames = client.take_all();
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "frames": frames,
        })),
    )
        .into_response()
}

pub async fn client_drain(
    State(listener): State<Listener>,
    ConnectInfo(connection): ConnectInfo<ConnectionInfo>,
    headers: HeaderMap,
    Query(params): Query<BTreeMap<String, String>>,
) -> Response {
    if !is_direct_loopback(connection.peer, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(handle) = params.get("handle") else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "missing handle" }))).into_response();
    };
    let client = listener
        .clients
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(handle)
        .cloned();
    let Some(client) = client else {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "unknown client handle" }))).into_response();
    };
    if let Some(expect) = params.get("expect").filter(|s| !s.is_empty()) {
        let timeout_ms = params
            .get("timeout_ms")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(5000)
            .clamp(50, 15000);
        match client.wait_for(expect, Duration::from_millis(timeout_ms)).await {
            Ok(frames) => (
                StatusCode::OK,
                Json(json!({ "ok": true, "matched": true, "frames": frames })),
            )
                .into_response(),
            Err(error) => (
                StatusCode::OK,
                Json(json!({ "ok": false, "matched": false, "error": error, "frames": client.take_all() })),
            )
                .into_response(),
        }
    } else {
        let wait_ms = params
            .get("wait_ms")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(200)
            .clamp(10, 5000);
        tokio::time::sleep(Duration::from_millis(wait_ms)).await;
        let frames = client.take_all();
        (StatusCode::OK, Json(json!({ "ok": true, "frames": frames }))).into_response()
    }
}

#[derive(Deserialize)]
pub struct ClientCloseBody {
    pub handle: String,
}

pub async fn client_close(
    State(listener): State<Listener>,
    ConnectInfo(connection): ConnectInfo<ConnectionInfo>,
    headers: HeaderMap,
    Json(req): Json<ClientCloseBody>,
) -> Response {
    if !is_direct_loopback(connection.peer, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let removed = listener
        .clients
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&req.handle);
    if let Some(client) = removed {
        let _ = client
            .send_raw("<close xmlns=\"urn:ietf:params:xml:ns:xmpp-framing\"/>")
            .await;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(*name, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn only_the_gateway_names_the_player() {
        let loopback: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        let lan: SocketAddr = "192.168.1.9:5000".parse().unwrap();
        let forwarded = headers(&[(GATEWAY_HEADER, "s3cret"), ("x-forwarded-for", "1.2.3.4, 203.0.113.7")]);
        assert_eq!(
            client_address(loopback, &forwarded, Some("s3cret")),
            ("203.0.113.7".parse().unwrap(), "gateway")
        );
        let wrong = headers(&[(GATEWAY_HEADER, "guess"), ("x-forwarded-for", "203.0.113.7")]);
        assert_eq!(
            client_address(loopback, &wrong, Some("s3cret")),
            ("127.0.0.1".parse().unwrap(), "direct")
        );
        assert_eq!(client_address(lan, &forwarded, Some("s3cret")).1, "direct");
        assert_eq!(client_address(loopback, &forwarded, None).1, "direct");
        let missing = headers(&[("x-forwarded-for", "203.0.113.7")]);
        assert_eq!(client_address(loopback, &missing, Some("s3cret")).1, "direct");
    }
}
