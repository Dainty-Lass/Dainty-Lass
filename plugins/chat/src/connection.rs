//! From Dauntless Revived's metagame chat at `e1c5fec` (`services/metagame/src/chat/connection.rs`), with
//! DaintyLass's changes: a WebSocket upgrade is a chat connection only on the paths the clients ask for
//! (`settings::WS_PATHS`, as Mystic Paradox's gateway allows them; DR took any path on its own port), and the
//! gateway is recognised by its secret here rather than by the metagame's `origin.rs`, which stays DR's.
//!
//! The chat listener (`ChatServer` in `realtime/chat.ts`): HTTP on its own loopback port, where an upgrade
//! becomes a chat connection and any other request is a 404. Each connection has a reader, which hands its
//! frames to the chat task in order, and a writer, which sends what the chat task gives it and counts what
//! still waits (Node's `bufferedAmount`).

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade, rejection::WebSocketUpgradeRejection};
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use service_kit::serve::ConnectionInfo;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::sync::{Notify, mpsc, oneshot};

use crate::server::{Event, FRAME_LIMIT, Out, Outbox};
use crate::settings::WS_PATHS;

/// The gateway's header on everything it forwards (DR's `X-Reforged-Gateway`).
pub const GATEWAY_HEADER: &str = "x-reforged-gateway";

/// How long a close we sent may wait for the client's answer before the connection is cut (ws's
/// closeTimeout).
const CLOSE_WAIT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct Listener {
    pub events: mpsc::UnboundedSender<Event>,
    pub gateway_secret: Option<Arc<str>>,
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
