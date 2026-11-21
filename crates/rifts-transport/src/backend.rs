// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The host's calls to the backend: ticket consumes, host-ready, the lease, Guard batches and completion, peer
//! disconnects. Each call is one POST on a fresh connection with no retry; answers are matched by substring on the raw
//! body, never parsed, so a reply that differs only in spacing is read as before (umbrella spec §7 item 6).

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::{BodyExt as _, Full, Limited};
use hyper_rustls::ConfigBuilderExt as _;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};

use crate::text::{is_hex64, safe_str};

/// Umbrella spec D3: the Rustic header names, user agent and route prefix.
pub const CAPABILITY_HEADER: &str = "x-rustic-host-capability";
pub const REQUEST_ID_HEADER: &str = "X-Rustic-Request-Id";
pub const USER_AGENT: &str = "RusticTransport/1";
pub const ROUTE_PREFIX: &str = "/rustic/v1/hunts/";
const TIMEOUT: Duration = Duration::from_secs(5);
const MAX_RESPONSE: usize = 16 * 1024;
const MAX_BODY: usize = 8192;
pub const MAX_BATCH_BODY: usize = 57344;

#[derive(Debug)]
struct AnyCertificate(Arc<rustls::crypto::CryptoProvider>);

/// Loopback backends only (a developer's own server with a self-signed certificate); the signature still has to
/// match the certificate.
impl ServerCertVerifier for AnyCertificate {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BaseUrl {
    https: bool,
    authority: String,
    host: String,
    path: String,
}

fn parse_base(url: &str) -> Result<BaseUrl, String> {
    let invalid = || "invalid admission backend URL".to_owned();
    let not_https = || "admission backend must be an HTTPS origin without credentials, query, or fragment".to_owned();
    let (https, rest) = if let Some(rest) = url.strip_prefix("https://") {
        (true, rest)
    } else if let Some(rest) = url.strip_prefix("http://") {
        (false, rest)
    } else if url.contains("://") {
        return Err(not_https());
    } else {
        return Err(invalid());
    };
    if rest.contains(['@', '?', '#']) {
        return Err(not_https());
    }
    let (authority, path) = rest.find('/').map_or((rest, ""), |at| (&rest[..at], &rest[at..]));
    let host = match authority.rfind(':') {
        Some(at) => {
            let port = &authority[at + 1..];
            if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) || port.parse::<u16>().is_err() {
                return Err(invalid());
            }
            &authority[..at]
        }
        None => authority,
    };
    if host.is_empty() {
        return Err(invalid());
    }
    let loopback = host == "127.0.0.1" || host == "localhost";
    if !https && !loopback {
        return Err(not_https());
    }
    // A hosts file or resolver could send `localhost` off the machine, where no certificate is checked (below).
    let authority = if host == "localhost" {
        authority.replacen("localhost", "127.0.0.1", 1)
    } else {
        authority.to_owned()
    };
    Ok(BaseUrl {
        https,
        authority,
        host: host.to_owned(),
        path: path.trim_end_matches('/').to_owned(),
    })
}

/// A backend answer: the status and up to 16 KiB of body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub status: u16,
    pub body: String,
}

/// `"name":"<64 lowercase hex>"` somewhere in the body.
pub fn find_hex64(body: &str, name: &str) -> Option<String> {
    let marker = format!("\"{name}\":\"");
    body.match_indices(&marker).find_map(|(at, _)| {
        let value = body.get(at + marker.len()..at + marker.len() + 64)?;
        let closed = body[at + marker.len() + 64..].starts_with('"');
        (closed && is_hex64(value)).then(|| value.to_owned())
    })
}

type Https = hyper_rustls::HttpsConnector<HttpConnector>;

pub struct BackendClient {
    base: BaseUrl,
    capability: String,
    session_id: String,
    runtime: tokio::runtime::Handle,
    client: Client<Https, Full<Bytes>>,
}

impl std::fmt::Debug for BackendClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackendClient")
            .field("authority", &self.base.authority)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumeAnswer {
    pub code: String,
    pub admission_id: String,
    pub idempotent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseOutcome {
    Renewed,
    Rejected,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseAnswer {
    pub outcome: LeaseOutcome,
    pub error: String,
}

/// A Guard batch's answer: taken with the backend's hash, retryable, or refused for good.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchAnswer {
    Accepted(String),
    Retry(String),
    Refused { status: u16, error: String },
}

pub fn classify_lease(answer: Result<Answer, String>) -> LeaseAnswer {
    let (outcome, error) = match answer {
        Err(error) => (LeaseOutcome::Unavailable, error),
        Ok(Answer { status: 200, body }) if body.contains("\"renewed\":true") => (LeaseOutcome::Renewed, String::new()),
        Ok(Answer { status: 503, body }) if body.contains("P2P_EMERGENCY_STOP") => (
            LeaseOutcome::Rejected,
            "host heartbeat rejected active generation (HTTP 503)".to_owned(),
        ),
        Ok(Answer { status, .. }) if matches!(status, 500 | 502 | 503 | 504 | 408 | 429) => (
            LeaseOutcome::Unavailable,
            format!("host heartbeat: the backend is unavailable (HTTP {status})"),
        ),
        Ok(Answer { status, .. }) => (
            LeaseOutcome::Rejected,
            format!("host heartbeat rejected active generation (HTTP {status})"),
        ),
    };
    LeaseAnswer { outcome, error }
}

pub fn classify_batch(answer: Result<Answer, String>) -> BatchAnswer {
    match answer {
        Err(error) => BatchAnswer::Retry(error),
        Ok(Answer { status: 200, body }) => match find_hex64(&body, "batchHash") {
            Some(hash) if body.contains("\"accepted\":true") => BatchAnswer::Accepted(hash),
            _ => BatchAnswer::Refused {
                status: 200,
                error: "malformed Guard event response".to_owned(),
            },
        },
        Ok(Answer { status, .. }) if status == 408 || status == 429 || (500..=599).contains(&status) => {
            BatchAnswer::Retry(format!(
                "Guard event backend rejected active generation (HTTP {status})"
            ))
        }
        Ok(Answer { status, .. }) => BatchAnswer::Refused {
            status,
            error: format!("Guard event backend rejected active generation (HTTP {status})"),
        },
    }
}

impl BackendClient {
    pub fn new(url: &str, capability: &str, session_id: &str, runtime: tokio::runtime::Handle) -> Result<Self, String> {
        if capability.len() != 43 || !safe_str(capability, 43, b"_-") {
            return Err("host capability must be a 32-byte base64url secret".to_owned());
        }
        let base = parse_base(url)?;
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .map_err(|_| "admission backend connection setup failed".to_owned())?;
        let tls = if base.host == "127.0.0.1" || base.host == "localhost" {
            builder
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(AnyCertificate(provider)))
                .with_no_client_auth()
        } else {
            builder.with_webpki_roots().with_no_client_auth()
        };
        let mut http = HttpConnector::new();
        http.enforce_http(false);
        http.set_connect_timeout(Some(TIMEOUT));
        let connector = hyper_rustls::HttpsConnectorBuilder::new()
            .with_tls_config(tls)
            .https_or_http()
            .enable_http1()
            .wrap_connector(http);
        let client = Client::builder(TokioExecutor::new())
            .pool_max_idle_per_host(0)
            .build(connector);
        Ok(Self {
            base,
            capability: capability.to_owned(),
            session_id: session_id.to_owned(),
            runtime,
            client,
        })
    }

    fn post(&self, route: &str, request_id: &str, body: String, max_body: usize) -> Result<Answer, String> {
        if !safe_str(route, 512, b"._:/-") || !safe_str(request_id, 96, b"._:-") || body.len() > max_body {
            return Err("invalid admission HTTP request".to_owned());
        }
        let uri = format!(
            "{}://{}{}{route}",
            if self.base.https { "https" } else { "http" },
            self.base.authority,
            self.base.path
        );
        let request = http::Request::post(uri)
            .header(http::header::CONTENT_TYPE, "application/json")
            .header(http::header::USER_AGENT, USER_AGENT)
            .header(CAPABILITY_HEADER, &self.capability)
            .header(REQUEST_ID_HEADER, request_id)
            .body(Full::new(Bytes::from(body)))
            .map_err(|_| "admission backend connection setup failed".to_owned())?;
        let call = async {
            let response = tokio::time::timeout(TIMEOUT * 2, self.client.request(request))
                .await
                .map_err(|_| "admission backend request failed".to_owned())?
                .map_err(|_| "admission backend request failed".to_owned())?;
            let status = response.status().as_u16();
            let collected = tokio::time::timeout(TIMEOUT, Limited::new(response.into_body(), MAX_RESPONSE).collect())
                .await
                .map_err(|_| "admission backend response read failed".to_owned())?;
            let bytes = match collected {
                Ok(collected) => collected.to_bytes(),
                Err(error) if error.is::<http_body_util::LengthLimitError>() => {
                    return Err("admission backend response exceeded limit".to_owned());
                }
                Err(_) => return Err("admission backend response read failed".to_owned()),
            };
            Ok(Answer {
                status,
                body: String::from_utf8_lossy(&bytes).into_owned(),
            })
        };
        self.runtime.block_on(call)
    }

    fn route(&self, tail: &str) -> String {
        format!("{ROUTE_PREFIX}{}/{tail}", self.session_id)
    }

    pub fn session_ok(&self) -> bool {
        safe_str(&self.session_id, 64, b"-")
    }

    pub fn consume(
        &self,
        ticket: &str,
        peer_id: u64,
        bridge_instance_id: &str,
        connection_handle: &str,
        request_id: &str,
    ) -> ConsumeAnswer {
        let refused = |code: &str| ConsumeAnswer {
            code: code.to_owned(),
            admission_id: String::new(),
            idempotent: false,
        };
        let valid = self.session_ok()
            && safe_str(ticket, 4096, b"._-")
            && safe_str(bridge_instance_id, 96, b"._:-")
            && safe_str(connection_handle, 32, b"")
            && peer_id != 0;
        if !valid {
            return refused("invalid_request");
        }
        let body = format!(
            "{{\"ticket\":\"{ticket}\",\"authenticatedSteamId\":\"{peer_id}\",\"bridgeInstanceId\":\"{bridge_instance_id}\",\
             \"connectionHandle\":\"{connection_handle}\"}}"
        );
        match self.post(&self.route("tickets/consume"), request_id, body, MAX_BODY) {
            Err(_) => refused("backend_unavailable"),
            Ok(Answer { status, .. }) if status != 200 => refused(&format!("backend_http_{status}")),
            Ok(Answer { body, .. }) => match find_hex64(&body, "admissionId") {
                Some(id) if body.contains("\"admitted\":true") => ConsumeAnswer {
                    code: "ok".to_owned(),
                    admission_id: id,
                    idempotent: body.contains("\"idempotent\":true"),
                },
                _ => refused("malformed_backend_response"),
            },
        }
    }

    pub fn host_ready(&self, virtual_port: u16, bridge_instance_id: &str, request_id: &str) -> Result<(), String> {
        if !self.session_ok() || !safe_str(bridge_instance_id, 96, b"._:-") || virtual_port == 0 {
            return Err("invalid host-ready request".to_owned());
        }
        let body = format!(
            "{{\"transport\":\"ICE\",\"virtualPort\":{virtual_port},\"bridgeInstanceId\":\"{bridge_instance_id}\"}}"
        );
        let answer = self.post(&self.route("host-ready"), request_id, body, MAX_BODY)?;
        if answer.status == 200 && answer.body.contains("\"ready\":true") {
            Ok(())
        } else {
            Err(format!(
                "host-ready backend rejected the active generation (HTTP {})",
                answer.status
            ))
        }
    }

    pub fn lease(&self, sequence: u64, admitted: u32, queue_pressure: u32, request_id: &str) -> LeaseAnswer {
        if sequence == 0 || admitted > 64 || queue_pressure > 100 || !self.session_ok() {
            return LeaseAnswer {
                outcome: LeaseOutcome::Rejected,
                error: "invalid host heartbeat".to_owned(),
            };
        }
        let body = format!(
            "{{\"leaseSequence\":{sequence},\"bridgeAlive\":true,\"serverReady\":true,\"admittedPeerCount\":{admitted},\
             \"queuePressure\":{queue_pressure}}}"
        );
        classify_lease(self.post(&self.route("lease"), request_id, body, MAX_BODY))
    }

    pub fn event_batch(&self, body: String, request_id: &str) -> BatchAnswer {
        if body.len() > MAX_BATCH_BODY {
            return BatchAnswer::Refused {
                status: 0,
                error: "Guard event batch exceeds its size limit".to_owned(),
            };
        }
        classify_batch(self.post(&self.route("event-batches"), request_id, body, MAX_BATCH_BODY))
    }

    pub fn completion(&self, final_sequence: u64, final_hash: &str, request_id: &str) -> Result<(), String> {
        if !self.session_ok() || final_sequence == 0 || !is_hex64(final_hash) {
            return Err("invalid Guard completion request".to_owned());
        }
        let body = format!("{{\"finalSequence\":{final_sequence},\"finalHash\":\"{final_hash}\"}}");
        let answer = self.post(&self.route("completion"), request_id, body, MAX_BODY)?;
        if answer.status == 200
            && answer.body.contains("\"accepted\":true")
            && answer.body.contains("\"progressionApplied\":false")
        {
            Ok(())
        } else {
            Err(format!(
                "Guard completion backend rejected active generation (HTTP {})",
                answer.status
            ))
        }
    }

    /// Fire and forget; nothing is sent for an id that is not 64 alphanumerics.
    pub fn disconnect(&self, admission_id: &str, request_id: &str) {
        let valid =
            self.session_ok() && admission_id.len() == 64 && admission_id.bytes().all(|b| b.is_ascii_alphanumeric());
        if valid {
            let _ = self.post(
                &self.route(&format!("peers/{admission_id}/disconnect")),
                request_id,
                "{}".to_owned(),
                MAX_BODY,
            );
        }
    }
}

type Job = Box<dyn FnOnce(&BackendClient) + Send>;

#[derive(Default)]
struct LaneState {
    queue: VecDeque<Job>,
    running: bool,
    stopping: bool,
}

/// One worker thread running posted jobs in order. The session thread never waits on the network: results come back
/// through whatever the job captured.
pub struct Lane {
    shared: Arc<(Mutex<LaneState>, Condvar)>,
    capacity: usize,
    worker: Option<thread::JoinHandle<()>>,
}

impl Lane {
    pub fn start(client: Arc<BackendClient>, capacity: usize) -> Self {
        let shared: Arc<(Mutex<LaneState>, Condvar)> = Arc::default();
        let worker_shared = shared.clone();
        let worker = thread::Builder::new()
            .name("rifts-backend-lane".to_owned())
            .spawn(move || {
                let (lock, signal) = &*worker_shared;
                loop {
                    let job = {
                        let Ok(mut state) = lock.lock() else {
                            return;
                        };
                        loop {
                            if state.stopping {
                                return;
                            }
                            if let Some(job) = state.queue.pop_front() {
                                state.running = true;
                                break job;
                            }
                            state = match signal.wait(state) {
                                Ok(state) => state,
                                Err(_) => return,
                            };
                        }
                    };
                    // A job that panics reports nothing; the next one still runs.
                    let _ = catch_unwind(AssertUnwindSafe(|| job(&client)));
                    if let Ok(mut state) = lock.lock() {
                        state.running = false;
                    }
                    signal.notify_all();
                }
            })
            .ok();
        Self {
            shared,
            capacity,
            worker,
        }
    }

    /// Refused when stopping or when `capacity` jobs are held, the running one included.
    pub fn post(&self, job: impl FnOnce(&BackendClient) + Send + 'static) -> bool {
        let (lock, signal) = &*self.shared;
        let Ok(mut state) = lock.lock() else {
            return false;
        };
        let held = state.queue.len() + usize::from(state.running);
        if state.stopping || self.worker.is_none() || held >= self.capacity {
            return false;
        }
        state.queue.push_back(Box::new(job));
        signal.notify_all();
        true
    }

    /// Waits up to `limit` for the queue to empty, then stops; jobs not started are dropped.
    pub fn drain_and_stop(&mut self, limit: Duration) {
        let deadline = Instant::now() + limit;
        let (lock, signal) = &*self.shared;
        if let Ok(mut state) = lock.lock() {
            while !state.queue.is_empty() || state.running {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                state = match signal.wait_timeout(state, left) {
                    Ok((state, _)) => state,
                    Err(_) => return,
                };
            }
        }
        self.stop();
    }

    /// Stops after the running job, if any.
    pub fn stop(&mut self) {
        let (lock, signal) = &*self.shared;
        if let Ok(mut state) = lock.lock() {
            state.stopping = true;
            state.queue.clear();
        }
        signal.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for Lane {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The lease rule: two refusals in a row fail it, and so does 30 s without a renewal while unavailable.
#[derive(Debug, Clone, Copy)]
pub struct LeaseTracker {
    pub last_renewal: Instant,
    refusals: u32,
}

pub const LEASE_GRACE: Duration = Duration::from_secs(30);

impl LeaseTracker {
    pub fn new(now: Instant) -> Self {
        Self {
            last_renewal: now,
            refusals: 0,
        }
    }

    /// Whether the lease has failed.
    pub fn record(&mut self, outcome: LeaseOutcome, now: Instant) -> bool {
        match outcome {
            LeaseOutcome::Renewed => {
                self.refusals = 0;
                self.last_renewal = now;
                false
            }
            LeaseOutcome::Rejected => {
                self.refusals += 1;
                self.refusals >= 2
            }
            LeaseOutcome::Unavailable => {
                self.refusals = 0;
                now.saturating_duration_since(self.last_renewal) >= LEASE_GRACE
            }
        }
    }

    pub fn seconds_since_renewal(&self, now: Instant) -> u64 {
        now.saturating_duration_since(self.last_renewal).as_secs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_urls() {
        assert!(parse_base("https://api.example.com/base/").is_ok_and(|b| b.path == "/base"));
        assert!(parse_base("http://127.0.0.1:8080").is_ok());
        assert!(parse_base("http://localhost:8080/x").is_ok_and(|b| b.authority == "127.0.0.1:8080"));
        assert_eq!(
            parse_base("http://api.example.com"),
            Err("admission backend must be an HTTPS origin without credentials, query, or fragment".to_owned())
        );
        assert_eq!(
            parse_base("api.example.com"),
            Err("invalid admission backend URL".to_owned())
        );
        assert_eq!(
            parse_base("https://:443"),
            Err("invalid admission backend URL".to_owned())
        );
    }

    #[test]
    fn hex_matches() {
        let id = "a".repeat(64);
        assert_eq!(
            find_hex64(&format!("{{\"admissionId\":\"{id}\"}}"), "admissionId"),
            Some(id.clone())
        );
        assert_eq!(
            find_hex64(&format!("{{\"admissionId\": \"{id}\"}}"), "admissionId"),
            None
        );
        assert_eq!(
            find_hex64(&format!("{{\"admissionId\":\"{id}a\"}}"), "admissionId"),
            None
        );
        assert_eq!(
            find_hex64(&format!("{{\"admissionId\":\"{}\"}}", "A".repeat(64)), "admissionId"),
            None
        );
    }
}
