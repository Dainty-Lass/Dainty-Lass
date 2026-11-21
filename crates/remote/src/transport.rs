// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `NodeService` adapters (`Service::Remote`) and pinned-TLS enrollment client
//! (`POST /__node/enroll`, `docs/remote-management.md` §5, `docs/security/remote-management.md`).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bytes::Bytes;
use http::Request;
use http_body_util::{BodyExt, Full, Limited};
use hyper_util::rt::TokioIo;
use module_api::BoxFuture;
use module_api::network::NodeId;
use network::link::{LinkError, NodeService, PeerLink, Service};
use rustls::DigitallySignedStruct;
use rustls::SignatureScheme;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use url::Url;

use crate::RemoteError;
use crate::enroll::{EnrollmentInvite, verify_cert_fingerprint};
use crate::state::{RemoteControllerState, RemoteHostHandler, RemoteManagedAgent, generate_node_keypair_and_spki_pem};
use crate::wire::{
    CommandOutcome, EnrollAnswer, EnrollRequest, MAX_REMOTE_WIRE, RemoteRequest, decode_enroll_answer, decode_request,
    decode_response, encode_enroll_request, encode_request, encode_response,
};

pub const ENROLL_PATH: &str = "/__node/enroll";

fn append_wire_log(direction: &str, bytes: &[u8]) {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    if let Ok(path) = std::env::var("REFORGED_REMOTE_WIRE_LOG")
        && !path.is_empty()
        && let Ok(mut file) = options.open(path)
    {
        use std::io::Write as _;
        let _ = writeln!(file, "{direction}: {}", String::from_utf8_lossy(bytes));
    }
}

/// `NodeService` attached to `Service::Remote` on a controller install.
pub struct RemoteControllerService {
    state: Arc<Mutex<RemoteControllerState>>,
    state_file: Option<PathBuf>,
}

impl RemoteControllerService {
    pub fn new(state: Arc<Mutex<RemoteControllerState>>, state_file: Option<PathBuf>) -> Self {
        Self { state, state_file }
    }
}

impl NodeService for RemoteControllerService {
    fn serve<'a>(
        &'a self,
        from: &'a NodeId,
        service: Service,
        body: Vec<u8>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LinkError>> {
        Box::pin(async move {
            if service != Service::Remote {
                return Err(LinkError::NoService {
                    node: from.clone(),
                    service,
                });
            }
            append_wire_log("controller_recv", &body);
            let req = decode_request(&body).map_err(|e| LinkError::Protocol {
                node: from.clone(),
                reason: e.to_string(),
            })?;
            let now_ms = chrono::Utc::now().timestamp_millis();
            let resp = {
                let mut guard = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                if let Some(path) = &self.state_file
                    && path.is_file()
                    && let Ok(fresh) = RemoteControllerState::load(path)
                {
                    *guard = fresh;
                }
                let r = guard.handle_remote_request(from, req, now_ms).map_err(|e| match e {
                    RemoteError::UnknownInstall(_) | RemoteError::RevokedPeer(_) => LinkError::Refused {
                        node: from.clone(),
                        reason: e.to_string(),
                    },
                    RemoteError::PeerIdentityMismatch { .. } => LinkError::Untrusted {
                        node: from.clone(),
                        reason: e.to_string(),
                    },
                    other => LinkError::Protocol {
                        node: from.clone(),
                        reason: other.to_string(),
                    },
                })?;
                if let Some(path) = &self.state_file {
                    let _ = guard.save(path);
                }
                r
            };
            let out = encode_response(&resp).map_err(|e| LinkError::Protocol {
                node: from.clone(),
                reason: e.to_string(),
            })?;
            append_wire_log("controller_send", &out);
            Ok(out)
        })
    }
}

/// `NodeService` attached to `Service::Remote` on a managed install so the controller can push
/// config revisions, execute immediate commands, or fetch log pages over mTLS.
pub struct RemoteManagedService {
    agent: Arc<Mutex<RemoteManagedAgent>>,
    handler: Arc<dyn RemoteHostHandler>,
}

impl RemoteManagedService {
    pub fn new(agent: Arc<Mutex<RemoteManagedAgent>>, handler: Arc<dyn RemoteHostHandler>) -> Self {
        Self { agent, handler }
    }
}

impl NodeService for RemoteManagedService {
    fn serve<'a>(
        &'a self,
        from: &'a NodeId,
        service: Service,
        body: Vec<u8>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LinkError>> {
        Box::pin(async move {
            if service != Service::Remote {
                return Err(LinkError::NoService {
                    node: from.clone(),
                    service,
                });
            }
            append_wire_log("controller_recv", &body);
            let req = decode_request(&body).map_err(|e| LinkError::Protocol {
                node: from.clone(),
                reason: e.to_string(),
            })?;
            let resp = {
                let mut guard = self.agent.lock().unwrap_or_else(PoisonError::into_inner);
                guard
                    .handle_controller_push(from, self.handler.as_ref(), req)
                    .map_err(|e| match e {
                        RemoteError::RevokedPeer(_) => LinkError::Refused {
                            node: from.clone(),
                            reason: e.to_string(),
                        },
                        RemoteError::PeerIdentityMismatch { .. } => LinkError::Untrusted {
                            node: from.clone(),
                            reason: e.to_string(),
                        },
                        other => LinkError::Protocol {
                            node: from.clone(),
                            reason: other.to_string(),
                        },
                    })?
            };
            let out = encode_response(&resp).map_err(|e| LinkError::Protocol {
                node: from.clone(),
                reason: e.to_string(),
            })?;
            append_wire_log("controller_send", &out);
            Ok(out)
        })
    }
}

/// Perform one outbound heartbeat `Sync` pull from a managed install to its controller over mTLS
/// (`PeerLink`).
pub async fn sync_once_with_controller(
    link: &dyn PeerLink,
    agent: &Mutex<RemoteManagedAgent>,
    handler: &dyn RemoteHostHandler,
    pending_outcomes: &Mutex<Vec<CommandOutcome>>,
) -> Result<u64, RemoteError> {
    let (node, controller_node, applied_rev, detached) = {
        let g = agent.lock().unwrap_or_else(PoisonError::into_inner);
        (
            g.node.clone(),
            g.controller_node.clone(),
            g.applied_revision,
            g.detached,
        )
    };
    if detached {
        return Err(RemoteError::RevokedPeer(node.as_str().to_string()));
    }
    let report = handler.build_report(applied_rev)?;
    let completed = {
        let mut g = pending_outcomes.lock().unwrap_or_else(PoisonError::into_inner);
        std::mem::take(&mut *g)
    };
    let req = RemoteRequest::Sync {
        node: node.clone(),
        applied_revision: applied_rev,
        report: Box::new(report),
        completed_commands: completed.clone(),
    };
    let req_bytes = encode_request(&req)?;
    append_wire_log("managed_send", &req_bytes);
    match link.call(&controller_node, Service::Remote, req_bytes).await {
        Ok(resp_bytes) => {
            append_wire_log("managed_recv", &resp_bytes);
            let ack = decode_response(&resp_bytes)?;
            let (new_outcomes, rev) = {
                let mut g = agent.lock().unwrap_or_else(PoisonError::into_inner);
                let outcomes = g.process_sync_ack(handler, ack)?;
                (outcomes, g.applied_revision)
            };
            if !new_outcomes.is_empty() {
                let follow_report = handler.build_report(rev)?;
                let follow_req = RemoteRequest::Sync {
                    node,
                    applied_revision: rev,
                    report: Box::new(follow_report),
                    completed_commands: new_outcomes.clone(),
                };
                if let Ok(follow_bytes) = encode_request(&follow_req) {
                    append_wire_log("managed_send", &follow_bytes);
                    match link.call(&controller_node, Service::Remote, follow_bytes).await {
                        Ok(follow_resp) => {
                            append_wire_log("managed_recv", &follow_resp);
                        }
                        Err(_) => {
                            let mut out_guard = pending_outcomes.lock().unwrap_or_else(PoisonError::into_inner);
                            out_guard.extend(new_outcomes);
                        }
                    }
                }
            }
            Ok(rev)
        }
        Err(err) => {
            if !completed.is_empty() {
                let mut out_guard = pending_outcomes.lock().unwrap_or_else(PoisonError::into_inner);
                let mut restored = completed;
                restored.extend(std::mem::take(&mut *out_guard));
                *out_guard = restored;
            }
            if matches!(err, LinkError::Refused { .. }) {
                let mut g = agent.lock().unwrap_or_else(PoisonError::into_inner);
                g.detached = true;
                return Err(RemoteError::RevokedPeer(err.to_string()));
            }
            let mut g = agent.lock().unwrap_or_else(PoisonError::into_inner);
            let _backoff_ms = g.record_controller_unreachable();
            Err(RemoteError::Io(format!("controller sync failed: {err}")))
        }
    }
}

/// TLS certificate verifier that pins the server's leaf certificate SHA-256 fingerprint to
/// `expected_fp` from the `dainty-lass-remote://enroll?...` URI.
#[derive(Debug)]
struct PinnedControllerCertVerifier {
    expected_fp: String,
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl ServerCertVerifier for PinnedControllerCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        verify_cert_fingerprint(end_entity.as_ref(), &self.expected_fp)
            .map_err(|_| rustls::Error::General("controller certificate fingerprint mismatch".to_string()))?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

fn write_secret_file(path: &Path, content: &str) -> Result<(), RemoteError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| RemoteError::Io(format!("{}: {e}", parent.display())))?;
    }
    std::fs::write(path, content).map_err(|e| RemoteError::Io(format!("{}: {e}", path.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Redeem a `dainty-lass-remote://enroll?...` invitation against the controller's gateway
/// (`POST /__node/enroll`), pinning the controller gateway certificate's SHA-256 fingerprint to
/// `invite.controller_fp`, and write `node.key` (`0600`), `node.crt`, and `cluster-ca.crt` to
/// `tls_dir`.
pub async fn enroll_with_controller(
    invite: &EnrollmentInvite,
    label_override: Option<String>,
    gateway_url: Option<String>,
    gateway_cert_fp: Option<String>,
    tls_dir: &Path,
) -> Result<EnrollAnswer, RemoteError> {
    invite.validate()?;
    let (priv_pem, pub_pem) = generate_node_keypair_and_spki_pem()?;
    let req = EnrollRequest {
        token: invite.token.clone(),
        node: invite.node.clone(),
        region: invite.region.clone(),
        label: label_override.or_else(|| invite.label.clone()),
        public_key_pem: pub_pem,
        gateway_url,
        gateway_cert_fp,
    };
    let req_bytes = encode_enroll_request(&req)?;
    // The one-time token is a secret: the debug log gets the request without it
    if let Ok(logged) = serde_json::to_vec(&EnrollRequest {
        token: "[redacted]".to_owned(),
        ..req.clone()
    }) {
        append_wire_log("enroll_send", &logged);
    }

    let parsed =
        Url::parse(&invite.url).map_err(|e| RemoteError::InvalidInput(format!("invalid controller URL: {e}")))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| RemoteError::InvalidInput("controller URL missing host".to_string()))?
        .to_string();
    let port = parsed.port_or_known_default().unwrap_or(61000);
    let addr = format!("{host}:{port}");

    let tcp = tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(&addr))
        .await
        .map_err(|_| RemoteError::Io(format!("timed out connecting to {addr}")))?
        .map_err(|e| RemoteError::Io(format!("connect {addr}: {e}")))?;

    let http_req = Request::builder()
        .method("POST")
        .uri(ENROLL_PATH)
        .header("host", &addr)
        .header("content-type", "application/json")
        .header("content-length", req_bytes.len())
        .body(Full::new(Bytes::from(req_bytes)))
        .map_err(|e| RemoteError::InvalidInput(format!("build enroll HTTP request: {e}")))?;

    let (status, resp_bytes) = if parsed.scheme() == "https" {
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let verifier = Arc::new(PinnedControllerCertVerifier {
            expected_fp: invite.controller_fp.clone(),
            provider: provider.clone(),
        });
        let tls_cfg = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| RemoteError::Crypto(format!("TLS protocol versions: {e}")))?
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth();
        let connector = TlsConnector::from(Arc::new(tls_cfg));
        let clean_host = host.trim_start_matches('[').trim_end_matches(']');
        let server_name = ServerName::try_from(clean_host.to_string())
            .map_err(|e| RemoteError::InvalidInput(format!("invalid server name {clean_host}: {e}")))?;
        let tls_stream = tokio::time::timeout(Duration::from_secs(10), connector.connect(server_name, tcp))
            .await
            .map_err(|_| RemoteError::Io("TLS handshake timed out".to_string()))?
            .map_err(|e| {
                if e.to_string().contains("fingerprint mismatch") {
                    RemoteError::CertificateMismatch {
                        expected: invite.controller_fp.clone(),
                        actual: "untrusted".to_string(),
                    }
                } else {
                    RemoteError::Io(format!("TLS handshake failed: {e}"))
                }
            })?;
        send_http1_request(tls_stream, http_req).await?
    } else {
        send_http1_request(tcp, http_req).await?
    };
    append_wire_log("enroll_recv", &resp_bytes);

    if status != 200 {
        let err_msg = String::from_utf8_lossy(&resp_bytes).into_owned();
        if err_msg.contains("used") {
            return Err(RemoteError::EnrollmentTokenUsed);
        }
        if err_msg.contains("expired") {
            return Err(RemoteError::EnrollmentTokenExpired);
        }
        if status == 401 || status == 403 {
            return Err(RemoteError::InvalidEnrollmentToken);
        }
        return Err(RemoteError::Io(format!("controller returned HTTP {status}: {err_msg}")));
    }

    let answer = decode_enroll_answer(&resp_bytes)?;
    std::fs::create_dir_all(tls_dir).map_err(|e| RemoteError::Io(format!("{}: {e}", tls_dir.display())))?;
    write_secret_file(&tls_dir.join("node.key"), &priv_pem)?;
    write_secret_file(&tls_dir.join("node-key.pem"), &priv_pem)?;
    std::fs::write(tls_dir.join("node.crt"), &answer.node_cert_pem)
        .map_err(|e| RemoteError::Io(format!("write node.crt: {e}")))?;
    std::fs::write(tls_dir.join("node-cert.pem"), &answer.node_cert_pem)
        .map_err(|e| RemoteError::Io(format!("write node-cert.pem: {e}")))?;
    std::fs::write(tls_dir.join("cluster-ca.crt"), &answer.ca_cert_pem)
        .map_err(|e| RemoteError::Io(format!("write cluster-ca.crt: {e}")))?;
    std::fs::write(tls_dir.join("cluster-ca-cert.pem"), &answer.ca_cert_pem)
        .map_err(|e| RemoteError::Io(format!("write cluster-ca-cert.pem: {e}")))?;
    Ok(answer)
}

async fn send_http1_request<S>(stream: S, req: Request<Full<Bytes>>) -> Result<(u16, Vec<u8>), RemoteError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .map_err(|e| RemoteError::Io(format!("HTTP handshake: {e}")))?;
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let resp = tokio::time::timeout(Duration::from_secs(15), sender.send_request(req))
        .await
        .map_err(|_| RemoteError::Io("HTTP request timed out".to_string()))?
        .map_err(|e| RemoteError::Io(format!("HTTP request failed: {e}")))?;
    let status = resp.status().as_u16();
    let limited = Limited::new(resp.into_body(), MAX_REMOTE_WIRE);
    let collected = tokio::time::timeout(Duration::from_secs(15), limited.collect())
        .await
        .map_err(|_| RemoteError::Io("reading HTTP response timed out".to_string()))?
        .map_err(|e| RemoteError::Io(format!("read HTTP response body: {e}")))?;
    Ok((status, collected.to_bytes().to_vec()))
}
