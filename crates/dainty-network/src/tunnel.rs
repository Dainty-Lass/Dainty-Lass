// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! HTTP between the two halves of a split server (`docs/split-roles.md`, `docs/security/tunnel.md`). The half that
//! does not run a service listens on that service's loopback port and carries each request over the node link to its
//! other half, which replays it on its own loopback port. Nothing listens off loopback but the gateway: the link is
//! the gateway's `/__node/tunnel` route under the cluster's mutual TLS.
//!
//! A [`TunnelService`] answers its one configured peer only, for the ports it exposes, and on the Director's port
//! only the Director's routes.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use dainty_module_api::BoxFuture;
use dainty_module_api::network::NodeId;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::header::{HeaderMap, HeaderName, HeaderValue};
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use crate::link::{LinkError, NodeService, PeerLink, Service};

/// One tunnelled call, the link included: the Director's own client waits 300 s for a hunt's game server.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(310);
/// The replay on the answering half, inside [`CALL_TIMEOUT`].
const REPLAY_TIMEOUT: Duration = Duration::from_secs(300);
/// A body either way: base64 and the JSON around it keep the message under the link's `MAX_MESSAGE`.
pub const MAX_BODY: usize = 512 * 1024;
const MAX_TARGET: usize = 8 * 1024;
const MAX_HEADERS: usize = 64;
const MAX_HEADER_BYTES: usize = 16 * 1024;
const METHODS: &[&str] = &["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"];
/// The Director's routes (director spec §3): its port carries nothing else, so the link starts no other work there.
pub const DIRECTOR_ROUTES: &[(&str, &str)] = &[
    ("GET", "/health/ready"),
    ("GET", "/api/hub-status"),
    ("GET", "/api/instances"),
    ("POST", "/api/matchmaker/handle-matchmaking-for-player"),
];
/// Per connection, or recomputed by the half that replays the request.
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-connection",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "host",
    "content-length",
];

/// The loopback ports a tunnel carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Port {
    /// The game backend's port: every route.
    Backend,
    /// The Director's port: only the routes the other half calls.
    Director,
}

impl Port {
    /// The name the request carries.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Backend => "backend",
            Self::Director => "director",
        }
    }
}

mod b64 {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let text = <&str>::deserialize(deserializer)?;
        STANDARD.decode(text).map_err(serde::de::Error::custom)
    }
}

/// A request as it crosses the link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TunnelRequest {
    /// Which loopback port it goes to.
    pub port: Port,
    /// The HTTP method, one the tunnel carries.
    pub method: String,
    /// Origin form: path and query
    pub target: String,
    /// Headers in order, hop-by-hop ones left out.
    pub headers: Vec<(String, String)>,
    /// The body, base64 on the wire.
    #[serde(with = "b64")]
    pub body: Vec<u8>,
}

/// An answer as it crosses the link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TunnelResponse {
    /// The HTTP status.
    pub status: u16,
    /// Headers in order, hop-by-hop ones left out.
    pub headers: Vec<(String, String)>,
    /// The body, base64 on the wire.
    #[serde(with = "b64")]
    pub body: Vec<u8>,
}

fn check_headers(headers: &[(String, String)]) -> Result<(), String> {
    if headers.len() > MAX_HEADERS {
        return Err(format!("more than {MAX_HEADERS} headers"));
    }
    let total: usize = headers.iter().map(|(n, v)| n.len() + v.len()).sum();
    if total > MAX_HEADER_BYTES {
        return Err(format!("headers over {MAX_HEADER_BYTES} bytes"));
    }
    for (name, value) in headers {
        let parsed = HeaderName::from_bytes(name.as_bytes()).map_err(|_| "a header name that is not a token")?;
        if HOP_BY_HOP.contains(&parsed.as_str()) {
            return Err(format!("a hop-by-hop header ({parsed})"));
        }
        HeaderValue::from_str(value).map_err(|_| format!("a bad value for {parsed}"))?;
    }
    Ok(())
}

/// Visible ASCII only: no space, control byte or fragment, so the target cannot split the request line.
fn check_target(target: &str) -> Result<(), String> {
    if !target.starts_with('/') || target.starts_with("//") {
        return Err("a target that is not origin form".into());
    }
    if target.len() > MAX_TARGET {
        return Err(format!("a target over {MAX_TARGET} bytes"));
    }
    if !target.bytes().all(|b| (0x21..=0x7e).contains(&b) && b != b'#') {
        return Err("a target with a byte a request line cannot hold".into());
    }
    Ok(())
}

fn path_of(target: &str) -> &str {
    target.split_once('?').map_or(target, |(path, _)| path)
}

/// Whether the tunnel carries `method target` to `port`.
pub fn carries(port: Port, method: &str, target: &str) -> bool {
    match port {
        Port::Backend => true,
        Port::Director => DIRECTOR_ROUTES.contains(&(method, path_of(target))),
    }
}

impl TunnelRequest {
    /// Refuses a method, target, header set or body the tunnel does not carry.
    pub fn check(&self) -> Result<(), String> {
        if !METHODS.contains(&self.method.as_str()) {
            return Err("a method the tunnel does not carry".into());
        }
        check_target(&self.target)?;
        check_headers(&self.headers)?;
        if self.body.len() > MAX_BODY {
            return Err(format!("a body over {MAX_BODY} bytes"));
        }
        Ok(())
    }

    /// The request as JSON.
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec(self).map_err(|e| e.to_string())
    }

    /// Parsed and checked: a request that fails [`check`](Self::check) is refused.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let request: Self = serde_json::from_slice(bytes).map_err(|e| format!("not a tunnel request: {e}"))?;
        request.check()?;
        Ok(request)
    }
}

impl TunnelResponse {
    /// Refuses a status outside 100 to 599 and an oversized header set or body.
    pub fn check(&self) -> Result<(), String> {
        if !(100..=599).contains(&self.status) {
            return Err(format!("status {}", self.status));
        }
        check_headers(&self.headers)?;
        if self.body.len() > MAX_BODY {
            return Err(format!("a body over {MAX_BODY} bytes"));
        }
        Ok(())
    }

    /// The answer as JSON.
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec(self).map_err(|e| e.to_string())
    }

    /// Parsed and checked: an answer that fails [`check`](Self::check) is refused.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let response: Self = serde_json::from_slice(bytes).map_err(|e| format!("not a tunnel answer: {e}"))?;
        response.check()?;
        Ok(response)
    }

    /// `{"error":"<code>"}`, as the gateway refuses.
    pub fn refusal(status: u16, code: &str) -> Self {
        Self {
            status,
            headers: vec![("content-type".into(), "application/json".into())],
            body: format!("{{\"error\":\"{code}\"}}").into_bytes(),
        }
    }

    fn into_response(self) -> Response<Full<Bytes>> {
        let mut response = Response::new(Full::new(Bytes::from(self.body)));
        *response.status_mut() = StatusCode::from_u16(self.status).unwrap_or(StatusCode::BAD_GATEWAY);
        let headers = response.headers_mut();
        for (name, value) in self.headers {
            if let (Ok(name), Ok(value)) = (HeaderName::from_bytes(name.as_bytes()), HeaderValue::from_str(&value)) {
                headers.append(name, value);
            }
        }
        response
    }
}

/// The end-to-end headers, as text: a value that is not UTF-8 is dropped, and so is any header `Connection` names.
pub fn end_to_end(headers: &HeaderMap) -> Vec<(String, String)> {
    let named: Vec<String> = headers
        .get_all(hyper::header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|token| token.trim().to_ascii_lowercase())
        .collect();
    headers
        .iter()
        .filter(|(name, _)| !HOP_BY_HOP.contains(&name.as_str()) && !named.iter().any(|n| n == name.as_str()))
        .filter_map(|(name, value)| Some((name.as_str().to_owned(), value.to_str().ok()?.to_owned())))
        .take(MAX_HEADERS)
        .collect()
}

/// The answering half: the request replayed on this node's own loopback port.
pub struct TunnelService {
    peer: NodeId,
    exposed: Vec<(Port, u16)>,
}

impl TunnelService {
    /// `peer` is the only node answered; `exposed` the ports it may reach here, with their loopback port numbers.
    pub fn new(peer: NodeId, exposed: Vec<(Port, u16)>) -> Self {
        Self { peer, exposed }
    }

    async fn answer(&self, from: &NodeId, body: &[u8]) -> Result<TunnelResponse, LinkError> {
        let refused = |reason: String| LinkError::Refused {
            node: from.clone(),
            reason,
        };
        if *from != self.peer {
            return Err(refused("it is not this server's other half".into()));
        }
        let request = TunnelRequest::decode(body).map_err(|reason| LinkError::Protocol {
            node: from.clone(),
            reason,
        })?;
        let Some((_, number)) = self.exposed.iter().find(|(port, _)| *port == request.port) else {
            return Err(refused(format!("this half does not run the {}", request.port.as_str())));
        };
        if !carries(request.port, &request.method, &request.target) {
            return Ok(TunnelResponse::refusal(404, "not_found"));
        }
        let port = request.port;
        match tokio::time::timeout(REPLAY_TIMEOUT, replay(*number, request)).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(error)) => {
                tracing::warn!(peer = %from, port = port.as_str(), %error, "a tunnelled request failed here");
                Ok(TunnelResponse::refusal(502, "bad_gateway"))
            }
            Err(_) => Ok(TunnelResponse::refusal(504, "upstream_timeout")),
        }
    }
}

impl NodeService for TunnelService {
    fn serve<'a>(
        &'a self,
        from: &'a NodeId,
        service: Service,
        body: Vec<u8>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LinkError>> {
        Box::pin(async move {
            if service != Service::Tunnel {
                return Err(LinkError::NoService {
                    node: from.clone(),
                    service,
                });
            }
            let response = self.answer(from, &body).await?;
            response.encode().map_err(|reason| LinkError::Protocol {
                node: from.clone(),
                reason,
            })
        })
    }
}

/// One plain HTTP/1.1 exchange with `127.0.0.1:<port>`.
async fn replay(port: u16, request: TunnelRequest) -> Result<TunnelResponse, String> {
    let stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .map_err(|e| format!("127.0.0.1:{port}: {e}"))?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .map_err(|e| e.to_string())?;
    let driver = tokio::spawn(connection);
    let mut builder = Request::builder()
        .method(request.method.as_str())
        .uri(request.target.as_str())
        .header(hyper::header::HOST, format!("127.0.0.1:{port}"));
    for (name, value) in &request.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    let outgoing = builder
        .body(Full::new(Bytes::from(request.body)))
        .map_err(|e| e.to_string())?;
    let answer = sender.send_request(outgoing).await.map_err(|e| e.to_string());
    let result = match answer {
        Ok(answer) => {
            let status = answer.status().as_u16();
            let headers = end_to_end(answer.headers());
            match Limited::new(answer.into_body(), MAX_BODY).collect().await {
                Ok(body) => Ok(TunnelResponse {
                    status,
                    headers,
                    body: body.to_bytes().to_vec(),
                }),
                Err(_) => Ok(TunnelResponse::refusal(502, "body_too_large")),
            }
        }
        Err(e) => Err(e),
    };
    driver.abort();
    result
}

/// How the listening half finds its link: the gateway sets it once it starts, so it is looked up per request.
pub type LinkSource = Arc<dyn Fn() -> Option<Arc<dyn PeerLink>> + Send + Sync>;

/// The listening half's side of one port.
pub struct TunnelClient {
    /// Finds the link to the other half.
    pub link: LinkSource,
    /// The node running the other half.
    pub peer: NodeId,
    /// The port requests go to there.
    pub port: Port,
}

impl TunnelClient {
    /// One request over the link, every failure answered as the gateway answers one.
    pub async fn forward(&self, request: Request<Incoming>) -> Response<Full<Bytes>> {
        let (parts, body) = request.into_parts();
        let target = parts.uri.path_and_query().map_or("/", |p| p.as_str()).to_owned();
        let method = parts.method.as_str().to_owned();
        if !carries(self.port, &method, &target) {
            return TunnelResponse::refusal(404, "not_found").into_response();
        }
        let body = match Limited::new(body, MAX_BODY).collect().await {
            Ok(body) => body.to_bytes().to_vec(),
            Err(_) => return TunnelResponse::refusal(413, "body_too_large").into_response(),
        };
        let request = TunnelRequest {
            port: self.port,
            method,
            target,
            headers: end_to_end(&parts.headers),
            body,
        };
        if request.check().is_err() {
            return TunnelResponse::refusal(400, "bad_request").into_response();
        }
        let Some(link) = (self.link)() else {
            return TunnelResponse::refusal(502, "no_node_link").into_response();
        };
        let encoded = match request.encode() {
            Ok(encoded) => encoded,
            Err(_) => return TunnelResponse::refusal(400, "bad_request").into_response(),
        };
        match link.call(&self.peer, Service::Tunnel, encoded).await {
            Ok(bytes) => match TunnelResponse::decode(&bytes) {
                Ok(answer) => answer.into_response(),
                Err(reason) => {
                    tracing::warn!(peer = %self.peer, port = self.port.as_str(), %reason, "the other half answered oddly");
                    TunnelResponse::refusal(502, "bad_gateway").into_response()
                }
            },
            Err(error) => {
                tracing::warn!(peer = %self.peer, port = self.port.as_str(), %error, "the other half did not answer");
                TunnelResponse::refusal(502, "tunnel_unreachable").into_response()
            }
        }
    }
}

/// Serves `listener` (a loopback port) until `stop`, every request carried by `client`.
pub async fn serve(listener: TcpListener, client: Arc<TunnelClient>, stop: impl Future<Output = ()>) {
    let mut connections = JoinSet::new();
    tokio::pin!(stop);
    loop {
        tokio::select! {
            () = &mut stop => break,
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else { continue };
                let client = Arc::clone(&client);
                connections.spawn(async move {
                    let service = hyper::service::service_fn(move |request| {
                        let client = Arc::clone(&client);
                        async move { Ok::<_, std::convert::Infallible>(client.forward(request).await) }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
    connections.abort_all();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::MAX_MESSAGE;

    fn encoded_len(bytes: usize) -> usize {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(vec![0u8; bytes]).len()
    }

    fn node(name: &str) -> NodeId {
        name.parse().unwrap()
    }

    fn request(port: Port, method: &str, target: &str) -> TunnelRequest {
        TunnelRequest {
            port,
            method: method.into(),
            target: target.into(),
            headers: vec![("content-type".into(), "application/json".into())],
            body: b"{}".to_vec(),
        }
    }

    #[test]
    fn round_trip_and_limits() {
        let sent = request(Port::Backend, "POST", "/a?b=c");
        let back = TunnelRequest::decode(&sent.encode().unwrap()).unwrap();
        assert_eq!(back, sent);
        // A full body still fits one link message
        assert!(encoded_len(MAX_BODY) + MAX_HEADER_BYTES * 2 + MAX_TARGET * 2 < MAX_MESSAGE);
    }

    #[test]
    fn refuses_what_could_split_a_request() {
        for target in ["a", "//evil/x", "/a b", "/a\r\nX: y", "/a#b", "/\u{e9}"] {
            assert!(request(Port::Backend, "GET", target).check().is_err(), "{target:?}");
        }
        let mut bad = request(Port::Backend, "CONNECT", "/");
        assert!(bad.check().is_err());
        bad.method = "GET".into();
        bad.headers = vec![("transfer-encoding".into(), "chunked".into())];
        assert!(bad.check().is_err());
        bad.headers = vec![("x".into(), "a\r\nb".into())];
        assert!(bad.check().is_err());
        bad.headers = vec![("bad name".into(), "v".into())];
        assert!(bad.check().is_err());
        assert!(
            TunnelRequest::decode(br#"{"port":"backend","method":"GET","target":"/","headers":[],"body":"","x":1}"#)
                .is_err()
        );
        assert!(
            TunnelRequest::decode(br#"{"port":"admin","method":"GET","target":"/","headers":[],"body":""}"#).is_err()
        );
    }

    #[test]
    fn the_director_port_carries_its_routes_only() {
        assert!(carries(
            Port::Director,
            "POST",
            "/api/matchmaker/handle-matchmaking-for-player"
        ));
        assert!(carries(Port::Director, "GET", "/api/hub-status?x=1"));
        assert!(!carries(
            Port::Director,
            "GET",
            "/api/matchmaker/handle-matchmaking-for-player"
        ));
        assert!(!carries(Port::Director, "POST", "/realms/v1/anything"));
        assert!(carries(Port::Backend, "POST", "/realms/v1/anything"));
    }

    #[test]
    fn hop_by_hop_headers_stay_on_their_connection() {
        let mut headers = HeaderMap::new();
        headers.insert("connection", HeaderValue::from_static("close, x-private"));
        headers.insert("x-private", HeaderValue::from_static("1"));
        headers.insert("host", HeaderValue::from_static("a"));
        headers.insert("x-kept", HeaderValue::from_static("2"));
        assert_eq!(end_to_end(&headers), vec![("x-kept".to_owned(), "2".to_owned())]);
    }

    #[tokio::test]
    async fn answers_its_peer_only_and_its_ports_only() {
        let service = TunnelService::new(node("game-1"), vec![(Port::Director, 1)]);
        let body = request(Port::Director, "GET", "/api/instances").encode().unwrap();
        let other = service.serve(&node("someone"), Service::Tunnel, body.clone()).await;
        assert!(matches!(other, Err(LinkError::Refused { .. })));
        let wrong = service.serve(&node("game-1"), Service::Ring, body).await;
        assert!(matches!(wrong, Err(LinkError::NoService { .. })));
        let backend = request(Port::Backend, "GET", "/").encode().unwrap();
        let refused = service.serve(&node("game-1"), Service::Tunnel, backend).await;
        assert!(matches!(refused, Err(LinkError::Refused { .. })));
        let outside = request(Port::Director, "POST", "/api/instances").encode().unwrap();
        let answer = service.serve(&node("game-1"), Service::Tunnel, outside).await.unwrap();
        assert_eq!(TunnelResponse::decode(&answer).unwrap().status, 404);
    }

    #[tokio::test]
    async fn replays_on_loopback_and_back_over_a_link() {
        use crate::inprocess::InProcessNetwork;

        // The Director's stand-in on the game half: echoes what reached it
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_port = upstream.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (stream, _) = upstream.accept().await.unwrap();
            let service = hyper::service::service_fn(move |request: Request<Incoming>| async move {
                let seen = format!(
                    "{} {} {} {}",
                    request.method(),
                    request.uri(),
                    request.headers()["host"].to_str().unwrap(),
                    request.headers().get("x-private").is_some()
                );
                let mut answer = Response::new(Full::new(Bytes::from(seen)));
                answer.headers_mut().insert("x-answer", HeaderValue::from_static("1"));
                Ok::<_, std::convert::Infallible>(answer)
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });

        let (metagame, game) = (node("meta-1"), node("game-1"));
        let network = InProcessNetwork::new();
        let answering: Arc<dyn NodeService> = Arc::new(TunnelService::new(
            metagame.clone(),
            vec![(Port::Director, upstream_port)],
        ));
        network.attach(&game, &answering);
        let link: Arc<dyn PeerLink> = Arc::new(network.link(&metagame));
        let client = Arc::new(TunnelClient {
            link: Arc::new(move || Some(Arc::clone(&link))),
            peer: game,
            port: Port::Director,
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(serve(listener, client, async {
            let _ = stop_rx.await;
        }));

        let stream = TcpStream::connect(address).await.unwrap();
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .unwrap();
        tokio::spawn(connection);
        let asked = Request::builder()
            .uri("/api/instances?x=1")
            .header("host", "elsewhere")
            .header("connection", "x-private")
            .header("x-private", "1")
            .body(Full::new(Bytes::new()))
            .unwrap();
        let answer = sender.send_request(asked).await.unwrap();
        assert_eq!(answer.status(), 200);
        assert_eq!(answer.headers()["x-answer"], "1");
        let body = answer.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            body,
            format!("GET /api/instances?x=1 127.0.0.1:{upstream_port} false").as_bytes()
        );

        let refused = Request::builder()
            .method("DELETE")
            .uri("/api/instances")
            .body(Full::new(Bytes::new()))
            .unwrap();
        assert_eq!(sender.send_request(refused).await.unwrap().status(), 404);
        let _ = stop_tx.send(());
        server.await.unwrap();
    }
}
