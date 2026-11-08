// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Invite link formatting (`dauntless://join?...`), gateway X.509 certificate SHA-256 fingerprint
//! inspection, and local backend admin API calls (`/dainty/api/CreateInvite` and `/dainty/api/InviteCodes`).

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, Error as TlsError, SignatureScheme};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::HostError;
use crate::hostconfig::{HostConfig, ServerMode, gateway_port};

pub use dainty_protocol::GameVersion;
pub use dainty_protocol::invite::{
    DEFAULT_PUBLIC_PORT, FormatError, INVITE_SCHEME, Invite, InviteError, MAX_INVITE_LENGTH, REVIVED_INVITE_SCHEME,
    ServerMode as InviteServerMode, UNIFIED_INVITE_SCHEME, clean_server_name, dainty_version, format_invite,
    invite_schemes, is_loopback_host, is_magic_dns_name, is_private_mode_host, is_tailscale_ipv4, is_valid_fingerprint,
    is_valid_host, is_valid_invite_code, is_valid_ipv4, is_valid_port, is_valid_share_url, named_version, parse_invite,
    parse_invite_for, played_versions, version_alias,
};

/// Default game version string for standard 1.14.7 invites.
pub const DEFAULT_GAME_VERSION: &str = "1.14.7";

/// Parses a game version string or known alias (`"1.14.7"`, `"reforged"`) this server plays.
#[must_use]
pub fn parse_game_version(text: &str) -> Option<GameVersion> {
    let trimmed = dainty_service_kit::js::trim(text);
    if let Some(v) = named_version(trimmed) {
        return played_versions().contains(&v).then_some(v);
    }
    let parsed = trimmed.parse::<GameVersion>().ok()?;
    played_versions().contains(&parsed).then_some(parsed)
}

/// Components used to build an invite URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteParts {
    /// Private or public: which form the link takes.
    pub mode: ServerMode,
    /// The gateway's host.
    pub host: String,
    /// The gateway's port.
    pub port: u16,
    /// The invite code, or `OPEN`.
    pub code: String,
    /// The server's name, as the launcher shows it.
    pub name: String,
    /// The gateway certificate's SHA-256, hex; a public invite needs it.
    pub fp: Option<String>,
    /// A private server's Tailscale share link.
    pub share: Option<String>,
}

/// Options for creating a new invite code on the backend.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CodeOptions {
    /// How many accounts the code registers.
    pub uses: f64,
    /// The code has no limit on its uses.
    pub infinite: bool,
    /// Who the code is for.
    pub note: Option<String>,
}

/// Plain HTTP/1.1 response from a loopback service endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    /// The status code.
    pub status: u16,
    /// The body, as text.
    pub body: String,
}

fn encode_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')') {
            out.push(b as char);
        } else {
            use std::fmt::Write as _;
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// Formats an invite URL for the default 1.14.7 game version from [`InviteParts`].
pub fn format_invite_parts(parts: &InviteParts) -> Result<String, HostError> {
    format_game_invite(parts, &dainty_version())
}

/// Formats an invite URL for a specific [`GameVersion`] from [`InviteParts`].
pub fn format_game_invite(parts: &InviteParts, game: &GameVersion) -> Result<String, HostError> {
    let scheme = if *game == dainty_version() {
        INVITE_SCHEME
    } else {
        UNIFIED_INVITE_SCHEME
    };

    let mut url = match parts.mode {
        ServerMode::Public => {
            let fp = parts
                .fp
                .as_deref()
                .map(dainty_service_kit::js::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| HostError::message("public invites need the certificate fingerprint"))?;
            format!(
                "{scheme}://join?v=2&mode=public&host={}&port={}&fp={}&code={}&name={}",
                encode_uri_component(&parts.host),
                parts.port,
                encode_uri_component(fp),
                encode_uri_component(&parts.code),
                encode_uri_component(&parts.name),
            )
        }
        ServerMode::Private => {
            let mut s = format!(
                "{scheme}://join?v=1&host={}&port={}&code={}&name={}",
                encode_uri_component(&parts.host),
                parts.port,
                encode_uri_component(&parts.code),
                encode_uri_component(&parts.name),
            );
            if let Some(share) = parts.share.as_deref().filter(|v| !v.is_empty()) {
                s.push_str("&share=");
                s.push_str(&encode_uri_component(share));
            }
            s
        }
    };

    if scheme == UNIFIED_INVITE_SCHEME {
        url.push_str("&version=");
        url.push_str(&encode_uri_component(&game.to_string()));
    }

    Ok(url)
}

/// Formats an invite URL for `config` on 1.14.7.
pub fn invite_for(config: &HostConfig, code: &str, cert_fingerprint: Option<&str>) -> Result<String, HostError> {
    invite_for_game(config, code, cert_fingerprint, &dainty_version())
}

/// Formats an invite URL for `config` and an explicit [`GameVersion`].
pub fn invite_for_game(
    config: &HostConfig,
    code: &str,
    cert_fingerprint: Option<&str>,
    game: &GameVersion,
) -> Result<String, HostError> {
    let parts = InviteParts {
        mode: config.mode,
        host: config.host.clone(),
        port: gateway_port(config),
        code: code.to_owned(),
        name: config.name.clone(),
        fp: cert_fingerprint.map(str::to_owned),
        share: config.share_url.clone(),
    };
    format_game_invite(&parts, game)
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Computes the lowercase 64-character hex SHA-256 fingerprint of the first PEM certificate in `cert_file`.
pub fn certificate_fingerprint(cert_file: &Path) -> Result<String, HostError> {
    let pem = std::fs::read(cert_file).map_err(|e| HostError::io(cert_file, e))?;
    let der = match CertificateDer::from_pem_slice(&pem) {
        Ok(der) => der,
        Err(rustls::pki_types::pem::Error::NoItemsFound) => {
            return Err(HostError::message(format!(
                "no PEM certificate found in {}",
                cert_file.display()
            )));
        }
        Err(e) => {
            return Err(HostError::message(format!(
                "invalid PEM certificate {}: {e}",
                cert_file.display()
            )));
        }
    };
    if der.is_empty() {
        return Err(HostError::message(format!(
            "empty PEM certificate in {}",
            cert_file.display()
        )));
    }
    Ok(sha256_hex(der.as_ref()))
}

/// Takes any leaf, since its fingerprint is what is checked, but still checks the handshake signature: without it
/// whatever holds the port could replay the gateway's public certificate and pass as the gateway.
#[derive(Debug)]
struct LeafCaptureVerifier {
    leaf_der: Arc<Mutex<Option<Vec<u8>>>>,
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl ServerCertVerifier for LeafCaptureVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        if let Ok(mut slot) = self.leaf_der.lock() {
            *slot = Some(end_entity.as_ref().to_vec());
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// Connects to `127.0.0.1:port` over TLS (sending SNI `host`) and returns the leaf certificate's
/// 64-character lowercase hex SHA-256 fingerprint, or `None` if the connection/handshake fails.
pub fn served_fingerprint(port: u16, host: &str, timeout: Duration) -> Option<String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, timeout).ok()?;
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    let captured = Arc::new(Mutex::new(None));
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let verifier = Arc::new(LeafCaptureVerifier {
        leaf_der: Arc::clone(&captured),
        provider: Arc::clone(&provider),
    });
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .ok()?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();

    let server_name = ServerName::try_from(host.to_owned()).unwrap_or_else(|_| {
        let ip = host.parse::<IpAddr>().unwrap_or_else(|_| IpAddr::from([127, 0, 0, 1]));
        ServerName::IpAddress(ip.into())
    });

    let mut conn = ClientConnection::new(Arc::new(config), server_name).ok()?;
    while conn.is_handshaking() {
        if conn.complete_io(&mut stream).is_err() {
            // The leaf is captured before the signature is checked: a failed handshake proves nothing
            return None;
        }
    }
    let der = captured.lock().ok()?.clone()?;
    Some(sha256_hex(&der))
}

fn decode_chunked(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < raw.len() {
        let Some(line_end_rel) = raw[pos..].windows(2).position(|w| w == b"\r\n") else {
            break;
        };
        let line = &raw[pos..pos + line_end_rel];
        let size_str = std::str::from_utf8(line)
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim();
        let Ok(chunk_len) = usize::from_str_radix(size_str, 16) else {
            break;
        };
        pos += line_end_rel + 2;
        if chunk_len == 0 {
            break;
        }
        let available = raw.len().saturating_sub(pos);
        let take = chunk_len.min(available);
        out.extend_from_slice(&raw[pos..pos + take]);
        pos += take;
        if raw[pos..].starts_with(b"\r\n") {
            pos += 2;
        }
    }
    out
}

/// Sends a plain HTTP/1.1 request to `127.0.0.1:port` and returns the status code and body.
pub fn http_request(
    method: &str,
    port: u16,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
    timeout: Duration,
) -> Result<HttpResponse, HostError> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, timeout).map_err(|e| HostError::message(e.to_string()))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| HostError::message(e.to_string()))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|e| HostError::message(e.to_string()))?;

    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n");
    for (k, v) in headers {
        req.push_str(k);
        req.push_str(": ");
        req.push_str(v);
        req.push_str("\r\n");
    }
    if let Some(payload) = body {
        req.push_str(&format!("Content-Length: {}\r\n\r\n", payload.len()));
        req.push_str(payload);
    } else {
        req.push_str("\r\n");
    }

    stream
        .write_all(req.as_bytes())
        .map_err(|e| HostError::message(e.to_string()))?;
    let _ = stream.flush();

    let mut raw = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                raw.extend_from_slice(&buf[..n]);
                if raw.len() > 8 * 1024 * 1024 {
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
                if raw.is_empty() {
                    return Err(HostError::message(e.to_string()));
                }
                break;
            }
            Err(e) => return Err(HostError::message(e.to_string())),
        }
    }

    let (head_bytes, body_bytes) = if let Some(idx) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
        (&raw[..idx], &raw[idx + 4..])
    } else if let Some(idx) = raw.windows(2).position(|w| w == b"\n\n") {
        (&raw[..idx], &raw[idx + 2..])
    } else {
        return Err(HostError::message("malformed HTTP response from server"));
    };

    let head_str = String::from_utf8_lossy(head_bytes);
    let mut lines = head_str.lines();
    let status_line = lines.next().ok_or_else(|| HostError::message("empty HTTP response"))?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| HostError::message(format!("invalid HTTP status line: {status_line}")))?;

    let mut chunked = false;
    for line in lines {
        if let Some((k, v)) = line.split_once(':')
            && k.trim().eq_ignore_ascii_case("transfer-encoding")
            && v.to_ascii_lowercase().contains("chunked")
        {
            chunked = true;
        }
    }

    let decoded = if chunked {
        decode_chunked(body_bytes)
    } else {
        body_bytes.to_vec()
    };
    let body_text = String::from_utf8_lossy(&decoded).into_owned();
    Ok(HttpResponse {
        status,
        body: body_text,
    })
}

fn error_detail_from_body(body: &str) -> String {
    if let Ok(val) = serde_json::from_str::<Value>(body) {
        for key in ["error", "message", "detail"] {
            if let Some(msg) = val.get(key).and_then(Value::as_str)
                && !msg.trim().is_empty()
            {
                return msg.trim().to_owned();
            }
        }
    }
    let trimmed = body.trim();
    if trimmed.is_empty() {
        "unknown error".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Calls `POST /dainty/api/CreateInvite` on `127.0.0.1:port` and returns the created invite code.
pub fn create_invite_code(admin_key: &str, opts: &CodeOptions, port: u16) -> Result<String, HostError> {
    let key = admin_key.trim();
    if key.is_empty() {
        return Err(HostError::message("DAINTY_ADMIN_KEY is missing from secrets.env"));
    }
    let mut payload = serde_json::Map::new();
    payload.insert("uses".into(), Value::Number((opts.uses as i64).into()));
    payload.insert("infinite".into(), Value::Bool(opts.infinite));
    if let Some(name) = opts
        .note
        .as_deref()
        .map(dainty_service_kit::js::trim)
        .filter(|s| !s.is_empty())
    {
        payload.insert("name".into(), Value::String(name.to_owned()));
    }
    let body_str = dainty_service_kit::jsjson::stringify(&Value::Object(payload));
    let resp = http_request(
        "POST",
        port,
        "/dainty/api/CreateInvite",
        &[("x-dainty-admin-key", key), ("Content-Type", "application/json")],
        Some(&body_str),
        Duration::from_millis(5000),
    )
    .map_err(|e| {
        HostError::message(format!(
            "the backend is not running on 127.0.0.1:{port} ({e}): start the server first (dainty-host start)"
        ))
    })?;

    if resp.status == 401 || resp.status == 403 {
        return Err(HostError::message(format!(
            "the backend refused the admin key (HTTP {}): check DAINTY_ADMIN_KEY in secrets.env",
            resp.status
        )));
    }
    if resp.status != 200 {
        return Err(HostError::message(format!(
            "backend refused invite creation (HTTP {}): {}",
            resp.status,
            error_detail_from_body(&resp.body)
        )));
    }

    let parsed: Value = serde_json::from_str(&resp.body)
        .map_err(|e| HostError::message(format!("backend returned invalid JSON for CreateInvite: {e}")))?;
    let code = parsed
        .get("code")
        .or_else(|| parsed.get("payload").and_then(|p| p.get("code")))
        .and_then(Value::as_str)
        .map(dainty_service_kit::js::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| HostError::message("backend response did not include an invite code"))?;
    Ok(code.to_owned())
}

/// Calls `GET /dainty/api/InviteCodes` on `127.0.0.1:port` and returns the invite code objects.
pub fn list_invite_codes(port: u16, admin_key: &str) -> Result<Vec<Value>, HostError> {
    let key = admin_key.trim();
    if key.is_empty() {
        return Err(HostError::message("DAINTY_ADMIN_KEY is missing from secrets.env"));
    }
    let resp = http_request(
        "GET",
        port,
        "/dainty/api/InviteCodes",
        &[("x-dainty-admin-key", key)],
        None,
        Duration::from_millis(5000),
    )
    .map_err(|e| {
        HostError::message(format!(
            "could not reach the backend on 127.0.0.1:{port} ({e}). Is the server running? (npm start)"
        ))
    })?;

    if resp.status != 200 {
        return Err(HostError::message(format!(
            "backend refused invite listing (HTTP {}): {}",
            resp.status,
            error_detail_from_body(&resp.body)
        )));
    }

    let parsed: Value = serde_json::from_str(&resp.body)
        .map_err(|e| HostError::message(format!("backend returned invalid JSON for InviteCodes: {e}")))?;
    if let Some(arr) = parsed.get("InviteCodes").and_then(Value::as_array) {
        return Ok(arr.clone());
    }
    if let Some(arr) = parsed.get("invites").and_then(Value::as_array) {
        return Ok(arr.clone());
    }
    if let Some(arr) = parsed
        .get("payload")
        .and_then(|p| p.get("invites"))
        .and_then(Value::as_array)
    {
        return Ok(arr.clone());
    }
    if let Some(arr) = parsed.as_array() {
        return Ok(arr.clone());
    }
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

    use super::*;
    use std::net::TcpListener;

    #[test]
    fn served_fingerprint_reads_the_leaf_of_a_real_handshake() {
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(vec!["localhost".to_owned()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        let der = cert.der().clone();
        let expected = sha256_hex(der.as_ref());
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let server = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![der],
                rustls::pki_types::PrivateKeyDer::try_from(key.serialize_der()).unwrap(),
            )
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let serve = std::thread::spawn(move || {
            let (mut tcp, _) = listener.accept().unwrap();
            let mut conn = rustls::ServerConnection::new(Arc::new(server)).unwrap();
            while conn.is_handshaking() {
                if conn.complete_io(&mut tcp).is_err() {
                    break;
                }
            }
        });
        let got = served_fingerprint(port, "localhost", Duration::from_secs(5));
        serve.join().unwrap();
        assert_eq!(got, Some(expected));
    }

    #[test]
    fn formats_private_and_public_invites_and_parses_versions() {
        assert_eq!(parse_game_version("1.14.7"), Some(dainty_version()));
        assert_eq!(parse_game_version("reforged"), Some(dainty_version()));
        assert_eq!(parse_game_version("9.9.9"), None);

        let priv_parts = InviteParts {
            mode: ServerMode::Private,
            host: "100.101.102.103".into(),
            port: 61000,
            code: "ABCD-1234".into(),
            name: "Friday Hunts".into(),
            fp: None,
            share: Some("https://login.tailscale.com/admin/invite/abc123".into()),
        };
        let url = format_invite_parts(&priv_parts).unwrap();
        let parsed = parse_invite(&url).unwrap();
        assert_eq!(parsed.mode, InviteServerMode::Private);
        assert_eq!(parsed.host, "100.101.102.103");
        assert_eq!(parsed.port, 61000);
        assert_eq!(parsed.code, "ABCD-1234");
        assert_eq!(parsed.name, "Friday Hunts");

        let fp = "ab".repeat(32);
        let pub_parts = InviteParts {
            mode: ServerMode::Public,
            host: "play.example.org".into(),
            port: 443,
            code: "OPEN-9999".into(),
            name: "Public Server".into(),
            fp: Some(fp.clone()),
            share: None,
        };
        let pub_url = format_invite_parts(&pub_parts).unwrap();
        let pub_parsed = parse_invite(&pub_url).unwrap();
        assert_eq!(pub_parsed.mode, InviteServerMode::Public);
        assert_eq!(pub_parsed.fp.as_deref(), Some(fp.as_str()));
    }

    #[test]
    fn computes_certificate_fingerprint_from_pem() {
        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("cert.pem");
        assert!(certificate_fingerprint(&cert_path).is_err());
        std::fs::write(&cert_path, "not a pem file\n").unwrap();
        assert!(
            certificate_fingerprint(&cert_path)
                .unwrap_err()
                .to_string()
                .contains("no PEM certificate found")
        );
        let pem = crate::commands::setup::secrets::pem("CERTIFICATE", b"test-certificate-der-bytes");
        std::fs::write(&cert_path, pem).unwrap();
        let fp = certificate_fingerprint(&cert_path).unwrap();
        assert_eq!(fp, sha256_hex(b"test-certificate-der-bytes"));
        assert_eq!(fp.len(), 64);
        assert!(is_valid_fingerprint(&fp));
    }

    #[test]
    fn creates_and_lists_invite_codes_over_local_http() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let n = stream.read(&mut buf).unwrap();
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(req.contains("POST /dainty/api/CreateInvite HTTP/1.1"));
            assert!(req.contains("x-dainty-admin-key: secret123"));
            let body = r#"{"code":"HUNT-2026"}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(resp.as_bytes()).unwrap();
        });

        let code = create_invite_code(
            "secret123",
            &CodeOptions {
                uses: 5.0,
                infinite: false,
                note: Some("Alex".into()),
            },
            port,
        )
        .unwrap();
        assert_eq!(code, "HUNT-2026");
        handle.join().unwrap();
    }
}
