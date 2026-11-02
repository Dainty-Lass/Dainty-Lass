// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The helpers the moved tests (`modules`, `network`, `nodelink`, `home`, gateway-plugin's `library`) call by these
//! names (spec-rd27-gateway.md section 5.3), on top of this module's own. Written at integration from the spec and
//! those tests' calls (docs/port/clean-room/gateway.md section 7).

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use gateway::config::Config;
use gateway::feed::AllowlistFeed;
use gateway::gateway::Gateway;
use serde_json::Value;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpSocket, TcpStream};
use tokio_rustls::client::TlsStream;

use super::{Running, SECRET, TestCert, Upstream, closed_port, upstream};

pub const GATEWAY_SECRET: &str = SECRET;

pub fn make_cert() -> TestCert {
    TestCert::new()
}

/// The loopback ports a test gateway proxies to.
pub struct Ports {
    pub backend: u16,
    pub content: u16,
    pub ws: u16,
}

pub async fn dead_port() -> u16 {
    closed_port().await
}

/// A public gateway's settings on `cert`, the feed off, `extra` on top.
pub fn test_config(cert: &TestCert, ports: &Ports, extra: &[(&str, &str)]) -> Config {
    let cert_file = cert.cert_file.display().to_string();
    let key_file = cert.key_file.display().to_string();
    let backend = format!("http://127.0.0.1:{}", ports.backend);
    let content = format!("http://127.0.0.1:{}", ports.content);
    let ws = format!("http://127.0.0.1:{}", ports.ws);
    let mut pairs: Vec<(&str, &str)> = vec![
        ("GATEWAY_BIND", "127.0.0.1"),
        ("GATEWAY_PORT", "1"),
        ("GATEWAY_SECRET", SECRET),
        ("GATEWAY_CERT", &cert_file),
        ("GATEWAY_KEY", &key_file),
        ("GATEWAY_BACKEND_URL", &backend),
        ("GATEWAY_CONTENT_URL", &content),
        ("GATEWAY_WS_URL", &ws),
        ("GATEWAY_ALLOWLIST", "0"),
    ];
    pairs.extend_from_slice(extra);
    Config::from_env(&service_kit::env::Env::from_pairs(pairs)).unwrap()
}

/// `config` served on a free loopback port with `cert`.
pub async fn start_gateway(cert: &TestCert, config: Config, feed: Option<AllowlistFeed>) -> Running {
    let loaded = gateway::cert::load(&cert.cert_file, &cert.key_file).unwrap();
    let tls = gateway::gateway::Tls {
        config: gateway::cert::server_config(&loaded).unwrap(),
        info: loaded.info,
    };
    let gateway = Gateway::new(config, Some(tls), feed, None).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let serving = gateway.clone();
    let task = tokio::spawn(async move {
        serving
            .serve_until(listener, async move {
                _ = rx.await;
            })
            .await;
    });
    Running {
        port,
        gateway,
        stop: Some(tx),
        task,
    }
}

/// An upstream that echoes what reached it; the moved tests pass no options.
pub async fn start_upstream(name: &'static str, _options: Option<()>) -> Upstream {
    upstream(name).await
}

/// The fake upstreams take WebSocket upgrades too.
pub async fn start_ws_upstream() -> Upstream {
    upstream("ws").await
}

/// A request for [`https_request`].
#[derive(Debug, Clone, Default)]
pub struct Options<'a> {
    pub method: Option<&'a str>,
    pub headers: Vec<(&'a str, String)>,
    pub body: Option<Vec<u8>>,
    /// The source address (127.0.0.1 when `None`)
    pub local: Option<Ipv4Addr>,
}

#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    /// Lower-case names; a repeated header keeps its last value
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// A TCP connection to the gateway from `local` (127.0.0.1 when `None`).
pub async fn tcp_connect(port: u16, local: Option<Ipv4Addr>) -> std::io::Result<TcpStream> {
    let socket = TcpSocket::new_v4()?;
    socket.bind(SocketAddr::from((local.unwrap_or(Ipv4Addr::LOCALHOST), 0)))?;
    socket.connect(SocketAddr::from(([127, 0, 0, 1], port))).await
}

/// Takes any server certificate: these tests check routing, not the pin.
#[derive(Debug)]
struct AnyCert;

impl rustls::client::danger::ServerCertVerifier for AnyCert {
    fn verify_server_cert(
        &self,
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::aws_lc_rs::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// A TLS connection to the public listener, offering no ALPN name (a player, not a node).
pub async fn tls_connect(port: u16, local: Option<Ipv4Addr>) -> std::io::Result<TlsStream<TcpStream>> {
    let tcp = tcp_connect(port, local).await?;
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(std::io::Error::other)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyCert))
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from("localhost").map_err(std::io::Error::other)?;
    tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect(name, tcp)
        .await
}

/// One request with `Connection: close` over TLS, read to the end (10 s at most).
pub async fn https_request(port: u16, target: &str, options: Options<'_>) -> Result<Reply, String> {
    let method = options.method.unwrap_or("GET");
    let mut head = format!("{method} {target} HTTP/1.1\r\nhost: 127.0.0.1\r\nconnection: close\r\n");
    for (name, value) in &options.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    let body = options.body.unwrap_or_default();
    if !body.is_empty() || method == "POST" {
        head.push_str(&format!("content-length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");
    let mut tls = tls_connect(port, options.local).await.map_err(|e| e.to_string())?;
    tls.write_all(head.as_bytes()).await.map_err(|e| e.to_string())?;
    // The gateway may answer before the body (413) and close
    _ = tls.write_all(&body).await;
    let mut raw = Vec::new();
    _ = tokio::time::timeout(Duration::from_secs(10), tls.read_to_end(&mut raw)).await;
    let (answer, _) = super::parse(&raw).ok_or_else(|| format!("no answer: {:?}", String::from_utf8_lossy(&raw)))?;
    Ok(Reply {
        status: answer.status,
        headers: answer.headers.into_iter().collect(),
        body: answer.body,
    })
}
