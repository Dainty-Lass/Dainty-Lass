// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The helper: a loopback endpoint that takes logged-in players' addresses with a shared secret, the live set
//! with its expiry and state file, the audit log, and the sync that brings the firewall rule to the set.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use bytes::Bytes;
use dainty_service_kit::context::ServiceContext;
use dainty_service_kit::js;
use dainty_service_kit::linelog::{self, Throttle};
use http::header::{CONNECTION, CONTENT_LENGTH, CONTENT_TYPE, HeaderValue};
use http::{HeaderMap, Method, Request, Response, StatusCode};
use http_body_util::{BodyExt as _, Full};
use hyper::body::Incoming;
use hyper_util::rt::{TokioIo, TokioTimer};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio::sync::{Notify, Semaphore, watch};
use tokio::task::JoinHandle;

use super::config::{AllowlistConfig, FirewallKind};
use super::firewall::{Backend, plan};
use super::guard::Guard;
use super::nft::{Nft, NftSettings, can_manage_nft};
use super::state::{AllowlistState, Refresh};
use super::sync::{Apply, ApplyResult, FirewallSync, OnResult};
use crate::feed::InProcessSink;
use crate::ip::{AllowlistCheck, check_allowlist_ip};

const MAX_CONNECTIONS: usize = 64;
const HEAD_TIME: Duration = Duration::from_secs(5);
const CONNECTION_TIME: Duration = Duration::from_secs(30);
const MAX_BODY: usize = 1024;
const SAVE_EVERY: Duration = Duration::from_secs(30);
const STOP_LIMIT: Duration = Duration::from_secs(90);

/// Append-only JSON lines, each also an info line. Never the secret.
#[derive(Debug)]
pub struct AuditLog {
    file: PathBuf,
    lock: Mutex<()>,
}

impl AuditLog {
    /// An audit log appending to `file`.
    pub fn new(file: PathBuf) -> Self {
        Self {
            file,
            lock: Mutex::new(()),
        }
    }

    /// The log file.
    pub fn file(&self) -> &std::path::Path {
        &self.file
    }

    /// Appends `event` with `fields` and a timestamp, and logs it at info.
    pub fn write(&self, event: &str, fields: Value) {
        let mut line = Map::new();
        line.insert("t".into(), json!(dainty_service_kit::js::iso_string(crate::now_ms())));
        line.insert("event".into(), json!(event));
        if let Value::Object(fields) = &fields {
            for (k, v) in fields {
                line.insert(k.clone(), v.clone());
            }
        }
        let mut text = Value::Object(line).to_string();
        text.push('\n');
        let written = {
            let _held = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
            std::fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(&self.file)
                .and_then(|mut f| std::io::Write::write_all(&mut f, text.as_bytes()))
        };
        linelog::info(&format!("allowlist {event}"), fields);
        if let Err(error) = written {
            linelog::error(
                "audit log write failed",
                json!({ "file": self.file.display().to_string(), "error": error.to_string() }),
            );
        }
    }
}

/// The backend as configured, without the real backends' start checks.
pub fn applier_for(config: &AllowlistConfig) -> Backend {
    let settings = NftSettings {
        table: config.nft_table.clone(),
        interface: config.interface.clone(),
    };
    match (config.dry_run, config.firewall) {
        (true, firewall) => Backend::DryRun {
            firewall,
            nft: settings,
            rule_name: config.rule_name.clone(),
        },
        (false, FirewallKind::Nftables) => Backend::Nftables {
            nft: Nft::new(config.nft.clone()),
            settings,
        },
        (false, FirewallKind::Windows) => Backend::Windows {
            rule_name: config.rule_name.clone(),
        },
    }
}

#[cfg(windows)]
fn elevated() -> bool {
    super::windows::is_elevated()
}

#[cfg(not(windows))]
fn elevated() -> bool {
    false
}

/// The backend as configured; the real firewall must be this platform's and manageable.
pub async fn applier(config: &AllowlistConfig) -> Result<Backend, String> {
    if !config.dry_run {
        match config.firewall {
            FirewallKind::Nftables => {
                if !cfg!(target_os = "linux") {
                    return Err("ALLOWLIST_FIREWALL=nftables needs Linux".to_owned());
                }
                if !can_manage_nft(&config.nft).await {
                    return Err(format!(
                        "{} could not list the firewall: install nftables (apt install nftables) and run the helper as root or with CAP_NET_ADMIN (the reforged-allowlist systemd unit does)",
                        config.nft
                    ));
                }
            }
            FirewallKind::Windows => {
                if !cfg!(windows) {
                    return Err("ALLOWLIST_FIREWALL=windows needs Windows (Windows Firewall); on Linux use ALLOWLIST_FIREWALL=nftables".to_owned());
                }
                if !elevated() {
                    return Err("ALLOWLIST_DRY_RUN=0 needs administrator rights: run it as a scheduled task with \"Run with highest privileges\"".to_owned());
                }
            }
        }
    }
    Ok(applier_for(config))
}

/// A `Content-Length` read as a number.
pub fn declared_length(headers: &HeaderMap) -> Option<f64> {
    headers
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .map(|v| js::number(Some(&Value::String(v.to_owned()))))
}

/// `POST /allow`'s body: `None` when it is not JSON, else its `ip` member (`None` inside when there is none).
pub fn allow_body(body: &[u8]) -> Option<Option<Value>> {
    let value: Value = serde_json::from_slice(body).ok()?;
    Some(value.as_object().and_then(|o| o.get("ip")).cloned())
}

/// What the audit shows of a refused value.
fn shown(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_owned(),
        Some(Value::String(s)) => s
            .chars()
            .take(64)
            .map(|c| if (' '..='~').contains(&c) { c } else { '?' })
            .collect(),
        Some(Value::Number(_)) => "number".to_owned(),
        Some(Value::Bool(_)) => "boolean".to_owned(),
        Some(_) => "object".to_owned(),
    }
}

/// The helper's run: what stops it.
pub struct Running {
    /// Address it listens on.
    pub addr: SocketAddr,
    stop: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
}

/// The allowlist helper: the admitted set, the firewall sync and its HTTP API.
pub struct Helper {
    config: AllowlistConfig,
    backend: Arc<Backend>,
    guard: Arc<Guard>,
    state: Mutex<AllowlistState>,
    sync: FirewallSync,
    audit: Arc<AuditLog>,
    changed: AtomicBool,
    wake: Notify,
    throttle: Throttle,
    secret_digest: [u8; 32],
}

/// Where the started helper is left, for the gateway's feed and the core's admits.
pub type HelperSlot = Arc<Mutex<Option<Arc<Helper>>>>;

fn digest(text: &[u8]) -> [u8; 32] {
    Sha256::digest(text).into()
}

impl Helper {
    /// A helper on a fresh guard of `ALLOWLIST_PORTS`. Needs a runtime (the sync's scheduler starts here).
    pub fn new(config: AllowlistConfig, backend: Backend) -> Result<Arc<Self>, String> {
        let guard = Arc::new(Guard::new(&config.ports)?);
        Ok(Self::with_guard(config, backend, guard))
    }

    /// A helper on a guard shared with the core plugin.
    pub fn with_guard(config: AllowlistConfig, backend: Backend, guard: Arc<Guard>) -> Arc<Self> {
        let backend = Arc::new(backend);
        let audit = Arc::new(AuditLog::new(config.audit_log.clone()));
        let apply: Apply = {
            let (backend, guard, audit) = (backend.clone(), guard.clone(), audit.clone());
            Arc::new(move |ips| {
                let (backend, guard, audit) = (backend.clone(), guard.clone(), audit.clone());
                Box::pin(async move {
                    // The ports are read afresh for every run (the guard's text)
                    let rule = plan(&ips, &guard.text())?;
                    if let Some(described) = backend.apply(&rule).await? {
                        audit.write("dry_run", described);
                    }
                    Ok(())
                })
            })
        };
        let on_result: OnResult = {
            let (audit, dry_run) = (audit.clone(), config.dry_run);
            Arc::new(move |r: &ApplyResult| match &r.error {
                None => audit.write(
                    "rule_applied",
                    json!({ "enabled": !r.ips.is_empty(), "ips": r.ips, "dryRun": dry_run, "ms": r.ms }),
                ),
                Some(error) => audit.write(
                    "rule_failed",
                    json!({ "enabled": !r.ips.is_empty(), "ips": r.ips, "error": error }),
                ),
            })
        };
        let sync = FirewallSync::new(apply, on_result, config.min_interval);
        {
            let (audit, sync, weak) = (audit.clone(), sync.clone(), Arc::downgrade(&guard));
            guard.on_change(Arc::new(move || {
                if let Some(guard) = weak.upgrade() {
                    audit.write(
                        "ports_changed",
                        json!({ "ports": guard.text(), "modules": guard.modules() }),
                    );
                }
                sync.reapply();
            }));
        }
        let state = AllowlistState::new(config.ttl.as_millis() as i64, config.max_entries);
        Arc::new(Self {
            secret_digest: digest(config.secret.as_bytes()),
            config,
            backend,
            guard,
            state: Mutex::new(state),
            sync,
            audit,
            changed: AtomicBool::new(false),
            wake: Notify::new(),
            throttle: Throttle::new(60_000),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, AllowlistState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Its settings.
    pub fn config(&self) -> &AllowlistConfig {
        &self.config
    }

    /// The port guard.
    pub fn guard(&self) -> &Arc<Guard> {
        &self.guard
    }

    /// The firewall sync.
    pub fn sync(&self) -> &FirewallSync {
        &self.sync
    }

    /// The audit log.
    pub fn audit(&self) -> &AuditLog {
        &self.audit
    }

    /// Admitted addresses, canonical.
    pub fn list(&self) -> Vec<String> {
        self.lock().list()
    }

    /// `GET /status`: nothing about the secret or the settings files.
    pub fn status(&self) -> Value {
        let entries: Vec<Value> = self
            .lock()
            .entries()
            .into_iter()
            .map(|(ip, at)| json!({ "ip": ip, "expiresAt": dainty_service_kit::js::iso_string(at) }))
            .collect();
        let last = self.sync.last_result().map(|r| {
            let mut last = json!({ "ok": r.ok, "enabled": !r.ips.is_empty(), "ips": r.ips, "at": dainty_service_kit::js::iso_string(r.at) });
            if let (Some(error), Value::Object(map)) = (&r.error, &mut last) {
                map.insert("error".into(), json!(error));
            }
            last
        });
        let mut status = json!({
            "backend": self.backend.name(),
            "dryRun": self.config.dry_run,
            "allowPrivate": self.config.allow_private,
            "ttlSeconds": self.config.ttl.as_secs(),
            "ports": self.guard.text(),
            "entries": entries,
            "pending": self.sync.pending(),
            "lastApply": last,
        });
        let modules = self.guard.modules();
        if modules.values().any(|r| !r.is_empty())
            && let Value::Object(map) = &mut status
        {
            map.insert("modulePorts".into(), json!(modules));
        }
        status
    }

    fn save(&self) -> usize {
        self.changed.store(false, Ordering::Relaxed);
        let (saved, result) = {
            let state = self.lock();
            (state.len(), state.save(&self.config.state_file))
        };
        if let Err(error) = result {
            linelog::warn(
                "allowlist state file not saved",
                json!({ "file": self.config.state_file.display().to_string(), "error": error.to_string() }),
            );
        }
        saved
    }

    /// Expires what is due at `now`: audited, saved, and the firewall asked for the new list.
    pub fn expire_now(&self, now: i64) -> Vec<String> {
        let gone = self.lock().expire(now);
        if !gone.is_empty() {
            for ip in &gone {
                self.audit.write("expired", json!({ "ip": ip }));
            }
            self.save();
            self.sync.request(self.list());
        }
        gone
    }

    fn audit_throttled(&self, key: &str, event: &str, mut fields: Value) {
        if let Some(suppressed) = self.throttle.allow(key, crate::now_ms())
            && let Value::Object(map) = &mut fields
        {
            map.insert("suppressed".into(), json!(suppressed));
            self.audit.write(event, fields);
        }
    }

    /// Sections 4.6 to 4.8: the answer `POST /allow` gives for this `ip` member.
    pub fn allow_value(&self, value: Option<&Value>) -> (u16, Value) {
        let ip = match check_allowlist_ip(value.and_then(Value::as_str), self.config.allow_private) {
            AllowlistCheck::Allowed(ip) => ip,
            AllowlistCheck::Refused(reason) => {
                let shown = shown(value);
                self.audit_throttled(
                    &format!("refused:{reason}:{shown}"),
                    "refused",
                    json!({ "value": shown, "reason": reason }),
                );
                return (400, json!({ "error": "invalid_ip", "reason": reason }));
            }
        };
        let refreshed = self.lock().refresh(&ip, crate::now_ms());
        self.wake.notify_one();
        match refreshed {
            Refresh::Full => {
                self.audit_throttled(
                    "full",
                    "full",
                    json!({ "ip": ip.canonical, "max": self.config.max_entries }),
                );
                (503, json!({ "error": "allowlist_full" }))
            }
            Refresh::Added | Refresh::Refreshed => {
                let added = refreshed == Refresh::Added;
                if added {
                    self.audit.write("added", json!({ "ip": ip.canonical }));
                    self.save();
                    self.sync.request(self.list());
                } else {
                    self.changed.store(true, Ordering::Relaxed);
                }
                (
                    200,
                    json!({ "ip": ip.canonical, "added": added, "ttlSeconds": self.config.ttl.as_secs() }),
                )
            }
        }
    }

    /// The same as a `POST /allow` with this text; true on 200.
    pub fn allow_ip(&self, ip: &str) -> bool {
        self.allow_value(Some(&Value::String(ip.to_owned()))).0 == 200
    }

    /// The guard covers the port and the address is admitted (DN8, DN9).
    pub fn admit_udp(&self, ip: &str, port: u16) -> bool {
        self.guard.covers_port(port) && self.allow_ip(ip)
    }

    /// Whether `ip` is admitted and `port` is a guarded game port; does not admit anyone.
    pub fn is_admitted(&self, ip: &str, port: u16) -> bool {
        match check_allowlist_ip(Some(ip), self.config.allow_private) {
            AllowlistCheck::Allowed(parsed) => self.guard.covers_port(port) && self.lock().contains(&parsed.canonical),
            AllowlistCheck::Refused(_) => false,
        }
    }

    fn restore(&self) -> usize {
        let file = &self.config.state_file;
        let text = match std::fs::read_to_string(file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return 0,
            Err(e) => {
                linelog::warn(
                    "allowlist state file unreadable; starting empty",
                    json!({ "file": file.display().to_string(), "error": e.to_string() }),
                );
                return 0;
            }
        };
        match serde_json::from_str::<Value>(&text) {
            Ok(saved) => self.lock().restore(&saved, crate::now_ms(), self.config.allow_private),
            Err(e) => {
                linelog::warn(
                    "allowlist state file unreadable; starting empty",
                    json!({ "file": file.display().to_string(), "error": e.to_string() }),
                );
                0
            }
        }
    }

    /// Section 13: restore, audit, ask for the restored list (an empty one closes the ports), bind, serve.
    pub async fn start(self: &Arc<Self>) -> Result<Running, String> {
        if let Some(dir) = self.config.audit_log.parent() {
            _ = std::fs::create_dir_all(dir);
        }
        let restored = self.restore();
        if self.config.dry_run {
            linelog::warn(
                "DRY RUN: the firewall is never changed; every change is only written to the audit log",
                json!({}),
            );
        }
        self.audit.write(
            "start",
            json!({ "dryRun": self.config.dry_run, "restored": restored, "ttlSeconds": self.config.ttl.as_secs(),
                "ports": self.guard.text(), "allowPrivate": self.config.allow_private, "backend": self.backend.name() }),
        );
        self.sync.request(self.list());
        let listen = self.config.listen_address();
        let listener = TcpListener::bind(&listen)
            .await
            .map_err(|e| format!("Could not listen on {listen}: {e}"))?;
        let addr = listener
            .local_addr()
            .map_err(|e| format!("Could not listen on {listen}: {e}"))?;
        let (stop, stopped) = watch::channel(false);
        let tasks = vec![
            tokio::spawn(self.clone().expiry_loop(stopped.clone())),
            tokio::spawn(self.clone().save_loop(stopped.clone())),
            tokio::spawn(self.clone().endpoint(listener, stopped)),
        ];
        linelog::info(
            "allowlist helper ready",
            json!({ "listen": addr.to_string(), "dryRun": self.config.dry_run, "firewall": self.config.firewall.as_str(),
                "ports": self.guard.text(), "ttlSeconds": self.config.ttl.as_secs(),
                "minIntervalMs": self.config.min_interval.as_millis() as u64, "maxEntries": self.config.max_entries,
                "allowPrivate": self.config.allow_private, "auditLog": self.config.audit_log.display().to_string(),
                "stateFile": self.config.state_file.display().to_string() }),
        );
        Ok(Running { addr, stop, tasks })
    }

    /// Stops listening and the loops, saves the set and closes the ports; the count saved.
    pub async fn close(&self, running: Running) -> Result<usize, String> {
        linelog::info("stopping: closing the game ports", json!({ "signal": "stop" }));
        let work = async {
            running.stop.send_replace(true);
            for task in running.tasks {
                _ = task.await;
            }
            let saved = self.save();
            self.sync.finish(Vec::new()).await;
            self.audit.write("stop", json!({ "saved": saved }));
            saved
        };
        tokio::time::timeout(STOP_LIMIT, work)
            .await
            .map_err(|_| "Shutdown took longer than 90 s".to_owned())
    }

    async fn expiry_loop(self: Arc<Self>, mut stop: watch::Receiver<bool>) {
        loop {
            let wake = self.wake.notified();
            let next = self.lock().next_expiry();
            let sleep = next.map_or(Duration::from_secs(3600), |at| {
                Duration::from_millis((at + 5 - crate::now_ms()).max(0) as u64)
            });
            tokio::select! {
                _ = stop.wait_for(|s| *s) => return,
                () = wake => {}
                () = tokio::time::sleep(sleep) => {
                    self.expire_now(crate::now_ms());
                }
            }
        }
    }

    async fn save_loop(self: Arc<Self>, mut stop: watch::Receiver<bool>) {
        let mut every = tokio::time::interval(SAVE_EVERY);
        every.tick().await;
        loop {
            tokio::select! {
                _ = stop.wait_for(|s| *s) => return,
                _ = every.tick() => {
                    if self.changed.load(Ordering::Relaxed) {
                        self.save();
                    }
                }
            }
        }
    }

    async fn endpoint(self: Arc<Self>, listener: TcpListener, mut stop: watch::Receiver<bool>) {
        let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
        loop {
            let accepted = tokio::select! {
                _ = stop.wait_for(|s| *s) => return,
                accepted = listener.accept() => accepted,
            };
            let Ok((stream, peer)) = accepted else {
                continue;
            };
            let Ok(slot) = slots.clone().try_acquire_owned() else {
                continue;
            };
            let helper = self.clone();
            tokio::spawn(async move {
                let service = hyper::service::service_fn(move |req| {
                    let helper = helper.clone();
                    async move { Ok::<_, Infallible>(helper.answer(req, peer).await) }
                });
                let mut builder = hyper::server::conn::http1::Builder::new();
                builder.timer(TokioTimer::new()).header_read_timeout(HEAD_TIME);
                let serving = builder.serve_connection(TokioIo::new(stream), service);
                _ = tokio::time::timeout(CONNECTION_TIME, serving).await;
                drop(slot);
            });
        }
    }

    fn reply(status: u16, body: &Value, close: bool, allow: Option<&'static str>) -> Response<Full<Bytes>> {
        let text = body.to_string();
        let len = text.len();
        let mut r = Response::new(Full::new(Bytes::from(text)));
        *r.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let h = r.headers_mut();
        h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        h.insert(CONTENT_LENGTH, HeaderValue::from(len));
        h.insert(http::header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        if let Some(allow) = allow {
            h.insert(http::header::ALLOW, HeaderValue::from_static(allow));
        }
        if close {
            h.insert(CONNECTION, HeaderValue::from_static("close"));
        }
        r
    }

    /// Section 4: one request on the loopback endpoint.
    async fn answer(&self, req: Request<Incoming>, peer: SocketAddr) -> Response<Full<Bytes>> {
        let loopback = match peer.ip() {
            std::net::IpAddr::V4(v4) => v4.is_loopback(),
            std::net::IpAddr::V6(v6) => v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback()),
        };
        if !loopback {
            return Self::reply(403, &json!({ "error": "forbidden" }), true, None);
        }
        let path = req.uri().path().to_owned();
        let (wanted, allow) = match path.as_str() {
            "/allow" => (Method::POST, "POST"),
            "/status" => (Method::GET, "GET"),
            _ => return Self::reply(404, &json!({ "error": "not_found" }), false, None),
        };
        if req.method() != wanted {
            return Self::reply(405, &json!({ "error": "method_not_allowed" }), false, Some(allow));
        }
        let given = req.headers().get("x-allowlist-secret").map(|v| digest(v.as_bytes()));
        // Both sides hashed and compared without an early exit: the time says nothing about the secret
        let matches = given.is_some_and(|g| {
            g.iter()
                .zip(self.secret_digest.iter())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
        });
        if !matches {
            self.audit_throttled("unauthorized", "unauthorized", json!({ "path": path }));
            return Self::reply(401, &json!({ "error": "unauthorized" }), true, None);
        }
        if wanted == Method::GET {
            return Self::reply(200, &self.status(), false, None);
        }
        if declared_length(req.headers()).is_some_and(|n| n > MAX_BODY as f64) {
            return Self::reply(413, &json!({ "error": "body_too_large" }), true, None);
        }
        let mut body = req.into_body();
        let mut bytes = Vec::new();
        while let Some(frame) = body.frame().await {
            let Ok(frame) = frame else {
                return Self::reply(400, &json!({ "error": "bad_request" }), true, None);
            };
            if let Some(data) = frame.data_ref() {
                if bytes.len() + data.len() > MAX_BODY {
                    return Self::reply(413, &json!({ "error": "body_too_large" }), true, None);
                }
                bytes.extend_from_slice(data);
            }
        }
        let Some(ip) = allow_body(&bytes) else {
            return Self::reply(400, &json!({ "error": "bad_request" }), false, None);
        };
        let (status, body) = self.allow_value(ip.as_ref());
        Self::reply(status, &body, false, None)
    }
}

/// The gateway's feed straight into an in-process helper (DN8).
pub struct HelperSink(pub Weak<Helper>);

impl HelperSink {
    /// The sink the gateway's feed calls; admits nothing once the helper is gone.
    pub fn into_sink(self) -> InProcessSink {
        Arc::new(move |ip: &str| self.0.upgrade().is_some_and(|helper| helper.allow_ip(ip)))
    }
}

/// `allowlist --close-ports`: one apply of the empty list, for stop scripts (a supervisor that kills the helper
/// never lets it close the ports itself).
pub async fn close_ports_once(config: &AllowlistConfig) -> Result<(), String> {
    let backend = applier(config).await?;
    let audit = AuditLog::new(config.audit_log.clone());
    if let Some(dir) = config.audit_log.parent() {
        _ = std::fs::create_dir_all(dir);
    }
    let rule = plan(&[], &config.ports).map_err(|e| format!("Could not disable the game-port rule: {e}"))?;
    let described = backend
        .apply(&rule)
        .await
        .map_err(|e| format!("Could not disable the game-port rule: {e}"))?;
    if let Some(described) = described {
        audit.write("dry_run", described);
    }
    audit.write("closed_ports", json!({ "dryRun": config.dry_run }));
    Ok(())
}

/// The helper under a service context, on a fresh guard from `ALLOWLIST_PORTS`.
pub async fn run_with_context(config: AllowlistConfig, ctx: ServiceContext) -> Result<(), String> {
    let guard = Arc::new(Guard::new(&config.ports)?);
    run_with_guard(config, ctx, None, guard).await
}

/// The same start and stop under a service context, the helper left in `slot` so the gateway's feed and the
/// core's admits reach it directly, on a guard shared with the core plugin.
pub async fn run_with_guard(
    config: AllowlistConfig,
    mut ctx: ServiceContext,
    slot: Option<HelperSlot>,
    guard: Arc<Guard>,
) -> Result<(), String> {
    let status = ctx.status().clone();
    status.starting();
    let backend = match applier(&config).await {
        Ok(backend) => backend,
        Err(e) => {
            status.failed(e.clone());
            return Err(e);
        }
    };
    let helper = Helper::with_guard(config, backend, guard);
    let running = match helper.start().await {
        Ok(running) => running,
        Err(e) => {
            status.failed(e.clone());
            return Err(e);
        }
    };
    if let Some(slot) = &slot {
        *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(helper.clone());
    }
    status.ready(
        Some(running.addr.to_string()),
        Some(format!("ports {}", helper.guard.text())),
    );
    ctx.wait_stop().await;
    status.stopping();
    if let Some(slot) = &slot {
        slot.lock().unwrap_or_else(PoisonError::into_inner).take();
    }
    let closed = helper.close(running).await;
    status.stopped();
    closed.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shown_values() {
        assert_eq!(shown(None), "undefined");
        assert_eq!(shown(Some(&json!(5))), "number");
        assert_eq!(shown(Some(&json!(true))), "boolean");
        assert_eq!(shown(Some(&Value::Null)), "object");
        assert_eq!(shown(Some(&json!([1]))), "object");
        assert_eq!(shown(Some(&json!("a\u{e9}\n"))), "a??");
        assert_eq!(shown(Some(&json!("x".repeat(70)))).len(), 64);
    }

    #[test]
    fn bodies() {
        assert_eq!(allow_body(b"{\"ip\":\"1.2.3.4\"}"), Some(Some(json!("1.2.3.4"))));
        assert_eq!(allow_body(b"[]"), Some(None));
        assert_eq!(allow_body(b"nope"), None);
        let mut h = HeaderMap::new();
        h.insert(CONTENT_LENGTH, HeaderValue::from_static("1025"));
        assert_eq!(declared_length(&h), Some(1025.0));
    }
}
