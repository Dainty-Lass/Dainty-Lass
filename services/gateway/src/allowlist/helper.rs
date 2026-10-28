//! The helper itself (`src/allowlist/helper.ts`): the loopback HTTP endpoint the gateway reports
//! logged-in addresses to, the expiry of those addresses, the audit log and the saved state.

use std::convert::Infallible;
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bytes::Bytes;
use http::{Request, Response};
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper_util::rt::{TokioIo, TokioTimer};
use serde_json::{Map, Value, json};
use service_kit::linelog::{self, Throttle};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio::sync::{Notify, Semaphore, watch};

use super::config::AllowlistConfig;
use super::firewall::{Backend, plan};
use super::guard::Guard;
use super::state::{AllowlistState, Refresh};
use super::sync::{Apply, ApplyResult, FirewallSync, OnResult};
use crate::cert::iso;
use crate::feed::Clock;
use crate::ip::{AllowlistCheck, IpClass, check_allowlist_ip, parse_ip};

const MAX_BODY: usize = 1024;
const MAX_CONNECTIONS: usize = 64;

fn iso_ms(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(iso).unwrap_or_default()
}

/// Append-only JSON lines: every address added or expired, every refusal, every rule change (with
/// what a dry run would have done) and every failed secret. Never the secret itself.
pub struct AuditLog {
    file: Option<PathBuf>,
    lock: Mutex<()>,
}

impl AuditLog {
    pub fn new(file: Option<PathBuf>) -> Self {
        if let Some(dir) = file.as_deref().and_then(Path::parent) {
            let _ = std::fs::create_dir_all(dir);
        }
        Self {
            file,
            lock: Mutex::new(()),
        }
    }

    pub fn write(&self, event: &str, fields: Value) {
        let mut line = Map::new();
        line.insert("t".into(), json!(iso(chrono::Utc::now())));
        line.insert("event".into(), json!(event));
        if let Value::Object(fields) = &fields {
            line.extend(fields.clone());
        }
        if let Some(file) = &self.file {
            let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
            let written = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(file)
                .and_then(|mut f| writeln!(f, "{}", Value::Object(line)));
            if let Err(error) = written {
                linelog::error(
                    "audit log write failed",
                    json!({ "file": file.display().to_string(), "error": error.to_string() }),
                );
            }
        }
        linelog::info(&format!("allowlist {event}"), fields);
    }
}

/// What applies one desired set: a dry run writes it to the audit log, a real backend changes the
/// firewall.
pub fn applier(backend: Arc<Backend>, audit: Arc<AuditLog>, ports: String, dry_run: bool) -> Apply {
    applier_for(backend, audit, Arc::new(move || ports.clone()), dry_run)
}

/// The same, for ports that change while it runs: read afresh for every run (the game modules' ranges, DF4d).
pub fn applier_for(
    backend: Arc<Backend>,
    audit: Arc<AuditLog>,
    ports: Arc<dyn Fn() -> String + Send + Sync>,
    dry_run: bool,
) -> Apply {
    Arc::new(move |ips| {
        let (backend, audit, ports) = (backend.clone(), audit.clone(), ports());
        Box::pin(async move {
            let plan = plan(&ips, &ports)?;
            if dry_run {
                let mut fields = backend.describe(&plan)?;
                fields["backend"] = json!(backend.name());
                audit.write("dry_run", fields);
                return Ok(());
            }
            backend.apply(&plan).await
        })
    })
}

fn secret_matches(given: Option<&[u8]>, expected: &str) -> bool {
    let Some(given) = given else {
        return false;
    };
    // Hashing first gives equal lengths, so the comparison time says nothing about the secret
    let a = Sha256::digest(given);
    let b = Sha256::digest(expected.as_bytes());
    a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn reply(status: u16, body: &Value, close: bool) -> Response<Full<Bytes>> {
    let text = Bytes::from(body.to_string());
    let mut response = Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .header("content-length", text.len())
        .header("cache-control", "no-store");
    if close {
        response = response.header("connection", "close");
    }
    response
        .body(Full::new(text))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())))
}

/// What a refused value looks like in the audit log: a string's first 64 characters, printable
/// ASCII only, or JavaScript's `typeof` of anything else.
fn shown(given: Option<&Value>) -> String {
    match given {
        Some(Value::String(text)) => text
            .chars()
            .take(64)
            .map(|c| if (' '..='~').contains(&c) { c } else { '?' })
            .collect(),
        Some(Value::Number(_)) => "number".into(),
        Some(Value::Bool(_)) => "boolean".into(),
        Some(Value::Null | Value::Object(_) | Value::Array(_)) => "object".into(),
        None => "undefined".into(),
    }
}

pub struct Helper {
    config: AllowlistConfig,
    backend: Arc<Backend>,
    audit: Arc<AuditLog>,
    state: Mutex<AllowlistState>,
    sync: FirewallSync,
    now: Clock,
    guard: Arc<Guard>,
    throttle: Throttle,
    dirty: AtomicBool,
    expiry: Notify,
    stop: watch::Sender<bool>,
}

pub struct Running {
    pub port: u16,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Helper {
    pub fn new(
        config: AllowlistConfig,
        backend: Backend,
        audit: Arc<AuditLog>,
        now: Clock,
    ) -> Result<Arc<Self>, String> {
        let guard = Arc::new(Guard::new(&config.ports)?);
        Ok(Self::with_guard(config, backend, audit, now, guard))
    }

    /// A helper that guards what `guard` holds: `ALLOWLIST_PORTS` and the ranges game modules registered (DF4d).
    /// The guard outlives the helper, so a restarted allowlist module keeps the ranges of the modules loaded.
    pub fn with_guard(
        config: AllowlistConfig,
        backend: Backend,
        audit: Arc<AuditLog>,
        now: Clock,
        guard: Arc<Guard>,
    ) -> Arc<Self> {
        let backend = Arc::new(backend);
        Arc::new_cyclic(|this: &std::sync::Weak<Self>| {
            let ports: Arc<dyn Fn() -> String + Send + Sync> = {
                let guard = guard.clone();
                Arc::new(move || guard.text())
            };
            let apply = applier_for(backend.clone(), audit.clone(), ports, config.dry_run);
            let on_result: OnResult = {
                let this = this.clone();
                Arc::new(move |result: &ApplyResult| {
                    if let Some(helper) = this.upgrade() {
                        helper.applied(result);
                    }
                })
            };
            // The interval between rule changes is real time, whatever clock the entries use
            let sync = FirewallSync::new(apply, config.min_interval, on_result);
            guard.on_change({
                let this = this.clone();
                Arc::new(move || {
                    if let Some(helper) = this.upgrade() {
                        helper.ports_changed();
                    }
                })
            });
            Self {
                state: Mutex::new(AllowlistState::new(config.ttl.as_millis() as i64, config.max_entries)),
                config,
                backend,
                audit,
                sync,
                now,
                guard,
                throttle: Throttle::new(60_000),
                dirty: AtomicBool::new(false),
                expiry: Notify::new(),
                stop: watch::Sender::new(false),
            }
        })
    }

    /// The ports this helper guards: its own, and the game modules' (DF4d).
    pub fn guard(&self) -> &Arc<Guard> {
        &self.guard
    }

    pub fn sync(&self) -> &FirewallSync {
        &self.sync
    }

    pub fn list(&self) -> Vec<String> {
        self.lock().list()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, AllowlistState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A module's ranges came or went: the rule is applied again with the same addresses.
    fn ports_changed(&self) {
        self.audit.write(
            "ports_changed",
            json!({ "ports": self.guard.text(), "modules": self.guard.modules() }),
        );
        self.sync.reapply();
    }

    fn applied(&self, result: &ApplyResult) {
        if result.ok {
            self.audit.write(
                "rule_applied",
                json!({ "enabled": !result.ips.is_empty(), "ips": result.ips, "dryRun": self.config.dry_run, "ms": result.ms }),
            );
        } else {
            self.audit.write(
                "rule_failed",
                json!({ "enabled": !result.ips.is_empty(), "ips": result.ips, "error": result.error }),
            );
        }
    }

    /// Restores unexpired entries from the state file, then brings the rule to that set (an empty
    /// set closes the ports): the rule never keeps addresses from before a crash. Then listens.
    pub async fn start(self: &Arc<Self>) -> Result<Running, String> {
        let mut restored = 0;
        match std::fs::read_to_string(&self.config.state_file) {
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(saved) => {
                    let allow_private = self.config.allow_private;
                    restored = self.lock().load(&saved["entries"], (self.now)(), |ip| {
                        matches!(
                            check_allowlist_ip(Some(&ip.canonical), allow_private),
                            AllowlistCheck::Ok(_)
                        )
                    });
                }
                Err(error) => self.state_unreadable(&error.to_string()),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => self.state_unreadable(&error.to_string()),
        }
        self.audit.write(
            "start",
            json!({
                "dryRun": self.config.dry_run,
                "restored": restored,
                "ttlSeconds": self.config.ttl.as_secs(),
                "ports": self.guard.text(),
                "allowPrivate": self.config.allow_private,
                "backend": self.backend.name(),
            }),
        );
        self.sync.request(self.list());

        let address = SocketAddr::new(
            self.config.bind_host.parse::<IpAddr>().map_err(|e| e.to_string())?,
            self.config.port,
        );
        let listener = TcpListener::bind(address).await.map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let tasks = vec![
            tokio::spawn(self.clone().expire_loop()),
            tokio::spawn(self.clone().save_loop()),
            tokio::spawn(self.clone().serve(listener)),
        ];
        Ok(Running { port, tasks })
    }

    fn state_unreadable(&self, error: &str) {
        linelog::warn(
            "allowlist state file unreadable; starting empty",
            json!({ "file": self.config.state_file.display().to_string(), "error": error }),
        );
    }

    /// Stops listening, closes the ports (fail closed) and saves the live set, so a restart opens
    /// them again for the same players right away.
    pub async fn close(&self, running: Running) {
        self.stop.send_replace(true);
        for task in running.tasks {
            task.abort();
            let _ = task.await;
        }
        self.dirty.store(true, Ordering::Relaxed);
        self.save_if_dirty();
        self.sync.finish(Vec::new()).await;
        self.audit.write("stop", json!({ "saved": self.lock().len() }));
    }

    fn save_if_dirty(&self) {
        if !self.dirty.swap(false, Ordering::Relaxed) {
            return;
        }
        let entries: Vec<Value> = self
            .lock()
            .snapshot()
            .into_iter()
            .map(|(ip, at)| json!({ "ip": ip, "expiresAt": at }))
            .collect();
        let text = json!({ "version": 1, "entries": entries }).to_string();
        let file = &self.config.state_file;
        let mut temp = file.clone().into_os_string();
        temp.push(".tmp");
        let temp = PathBuf::from(temp);
        let saved = write_private(&temp, text.as_bytes()).and_then(|()| std::fs::rename(&temp, file));
        if let Err(error) = saved {
            linelog::warn(
                "allowlist state file not saved",
                json!({ "file": file.display().to_string(), "error": error.to_string() }),
            );
        }
    }

    async fn save_loop(self: Arc<Self>) {
        let mut every = tokio::time::interval(Duration::from_secs(30));
        every.tick().await;
        loop {
            every.tick().await;
            self.save_if_dirty();
        }
    }

    async fn expire_loop(self: Arc<Self>) {
        loop {
            let woken = self.expiry.notified();
            let next = self.lock().next_expiry();
            match next {
                None => woken.await,
                Some(at) => {
                    let wait = Duration::from_millis((at - (self.now)()).max(0) as u64 + 5);
                    tokio::select! {
                        () = tokio::time::sleep(wait) => self.expire_now(),
                        () = woken => {}
                    }
                }
            }
        }
    }

    /// Drops the addresses whose time is up (also called by tests with a moved clock).
    pub fn expire_now(&self) {
        let gone = self.lock().expire((self.now)());
        for ip in &gone {
            self.audit.write("expired", json!({ "ip": ip }));
        }
        if !gone.is_empty() {
            self.dirty.store(true, Ordering::Relaxed);
            self.save_if_dirty();
            self.sync.request(self.list());
        }
        self.expiry.notify_one();
    }

    pub fn status(&self) -> Value {
        let entries: Vec<Value> = self
            .lock()
            .snapshot()
            .into_iter()
            .map(|(ip, at)| json!({ "ip": ip, "expiresAt": iso_ms(at) }))
            .collect();
        let last = self.sync.last_result().map(|result| {
            let mut out = json!({
                "ok": result.ok,
                "enabled": !result.ips.is_empty(),
                "ips": result.ips,
                "at": iso_ms(result.at),
            });
            if let Some(error) = result.error {
                out["error"] = json!(error);
            }
            out
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
        // Only a server running game modules has any
        let modules = self.guard.modules();
        if !modules.is_empty() {
            status["modulePorts"] = json!(modules);
        }
        status
    }

    async fn serve(self: Arc<Self>, listener: TcpListener) {
        let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
        loop {
            let Ok((stream, peer)) = listener.accept().await else {
                continue;
            };
            let Ok(slot) = slots.clone().try_acquire_owned() else {
                continue;
            };
            let helper = self.clone();
            tokio::spawn(async move {
                let _slot = slot;
                let service = hyper::service::service_fn(move |request: Request<Incoming>| {
                    let helper = helper.clone();
                    async move { Ok::<_, Infallible>(helper.handle(request, peer).await) }
                });
                let connection = hyper::server::conn::http1::Builder::new()
                    .timer(TokioTimer::new())
                    .header_read_timeout(Duration::from_secs(5))
                    .serve_connection(TokioIo::new(stream), service);
                let _ = tokio::time::timeout(Duration::from_secs(30), connection).await;
            });
        }
    }

    async fn handle(&self, request: Request<Incoming>, peer: SocketAddr) -> Response<Full<Bytes>> {
        // Defence in depth: the bind address is loopback already
        let loopback = parse_ip(&peer.ip().to_string()).is_some_and(|ip| ip.class == IpClass::Loopback);
        if !loopback {
            return reply(403, &json!({ "error": "forbidden" }), true);
        }
        let path = request.uri().path().to_owned();
        let method = match path.as_str() {
            "/allow" => "POST",
            "/status" => "GET",
            _ => return reply(404, &json!({ "error": "not_found" }), false),
        };
        if request.method().as_str() != method {
            let mut response = reply(405, &json!({ "error": "method_not_allowed" }), false);
            response
                .headers_mut()
                .insert("allow", http::HeaderValue::from_static(method));
            return response;
        }
        let given = request.headers().get("x-allowlist-secret").map(|v| v.as_bytes());
        if !secret_matches(given, &self.config.secret) {
            if let Some(suppressed) = self.throttle.allow("unauthorized", crate::now_ms()) {
                self.audit
                    .write("unauthorized", json!({ "path": path, "suppressed": suppressed }));
            }
            return reply(401, &json!({ "error": "unauthorized" }), true);
        }
        if path == "/status" {
            return reply(200, &self.status(), false);
        }
        let declared = declared_length(
            request
                .headers()
                .get(http::header::CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok()),
        );
        if declared > MAX_BODY as f64 {
            return reply(413, &json!({ "error": "body_too_large" }), true);
        }
        let body = match Limited::new(request.into_body(), MAX_BODY).collect().await {
            Ok(body) => body.to_bytes(),
            Err(_) => return reply(413, &json!({ "error": "body_too_large" }), true),
        };
        let Some(ip) = allow_body(&body) else {
            return reply(400, &json!({ "error": "bad_request" }), false);
        };
        self.allow(ip.as_ref())
    }

    fn allow(&self, given: Option<&Value>) -> Response<Full<Bytes>> {
        let ip = match check_allowlist_ip(given.and_then(Value::as_str), self.config.allow_private) {
            AllowlistCheck::Ok(ip) => ip,
            AllowlistCheck::Refused(reason) => {
                let value = shown(given);
                if let Some(suppressed) = self
                    .throttle
                    .allow(&format!("refused:{reason}:{value}"), crate::now_ms())
                {
                    self.audit.write(
                        "refused",
                        json!({ "value": value, "reason": reason, "suppressed": suppressed }),
                    );
                }
                return reply(400, &json!({ "error": "invalid_ip", "reason": reason }), false);
            }
        };
        let result = self.lock().refresh(&ip, (self.now)());
        if result == Refresh::Full {
            if let Some(suppressed) = self.throttle.allow("full", crate::now_ms()) {
                self.audit.write(
                    "full",
                    json!({ "ip": ip.canonical, "max": self.config.max_entries, "suppressed": suppressed }),
                );
            }
            return reply(503, &json!({ "error": "allowlist_full" }), false);
        }
        self.dirty.store(true, Ordering::Relaxed);
        if result == Refresh::Added {
            self.audit.write("added", json!({ "ip": ip.canonical }));
            self.save_if_dirty();
            self.sync.request(self.list());
        }
        self.expiry.notify_one();
        reply(
            200,
            &json!({ "ip": ip.canonical, "added": result == Refresh::Added, "ttlSeconds": self.config.ttl.as_secs() }),
            false,
        )
    }

    /// Direct in-process allowlist entry addition (used by `ServerController` without HTTP overhead).
    pub fn allow_ip(&self, ip_text: &str) -> bool {
        let resp = self.allow(Some(&json!(ip_text)));
        resp.status().as_u16() == 200
    }

    /// Admits `ip_text` into the allowlist when `udp_port` is covered by [`Self::guard`] (DF4d, DN8, DN9).
    pub fn admit_udp(&self, ip_text: &str, udp_port: u16) -> bool {
        self.guard.covers_port(udp_port) && self.allow_ip(ip_text)
    }

    /// Whether `ip_text` is currently admitted in the allowlist and `udp_port` is covered by [`Self::guard`].
    pub fn is_admitted(&self, ip_text: &str, udp_port: u16) -> bool {
        let AllowlistCheck::Ok(ip) = check_allowlist_ip(Some(ip_text), self.config.allow_private) else {
            return false;
        };
        self.guard.covers_port(udp_port) && self.list().contains(&ip.canonical)
    }
}

/// Closes the firewall game ports once and returns (`--close-ports`).
pub async fn close_ports_once(config: &AllowlistConfig) -> Result<(), String> {
    let audit = Arc::new(AuditLog::new(Some(config.audit_log.clone())));
    let backend = if config.dry_run {
        Backend::dry_run(config)
    } else {
        Backend::real(config).await?
    };
    let backend = Arc::new(backend);
    let apply = applier(backend, audit.clone(), config.ports.clone(), config.dry_run);
    plan(&[], &config.ports)?;
    apply(Vec::new())
        .await
        .map_err(|e| format!("Could not disable the game-port rule: {e}"))?;
    audit.write("closed_ports", json!({ "dryRun": config.dry_run }));
    Ok(())
}

/// Runs the allowlist helper in-process with a `ServiceContext`.
pub async fn run_with_context(
    config: AllowlistConfig,
    ctx: service_kit::context::ServiceContext,
    helper_slot: Option<Arc<Mutex<Option<Arc<Helper>>>>>,
) -> Result<(), String> {
    let guard = Arc::new(Guard::new(&config.ports)?);
    run_with_guard(config, ctx, helper_slot, guard).await
}

/// The same, guarding what `guard` holds (the controller shares it with the core plugin, DF4d).
pub async fn run_with_guard(
    config: AllowlistConfig,
    mut ctx: service_kit::context::ServiceContext,
    helper_slot: Option<Arc<Mutex<Option<Arc<Helper>>>>>,
    guard: Arc<Guard>,
) -> Result<(), String> {
    ctx.status.starting();
    let audit = Arc::new(AuditLog::new(Some(config.audit_log.clone())));
    let backend = if config.dry_run {
        linelog::warn(
            "DRY RUN: the firewall is never changed; every change is only written to the audit log",
            json!({}),
        );
        Backend::dry_run(&config)
    } else {
        match Backend::real(&config).await {
            Ok(backend) => backend,
            Err(error) => {
                ctx.status.failed(error.clone());
                return Err(error);
            }
        }
    };
    let helper = Helper::with_guard(config.clone(), backend, audit, Arc::new(crate::now_ms), guard);
    if let Some(slot) = helper_slot {
        *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(helper.clone());
    }
    let running = match helper.start().await {
        Ok(running) => running,
        Err(error) => {
            let msg = format!("Could not listen on {}:{}: {error}", config.bind_host, config.port);
            ctx.status.failed(msg.clone());
            return Err(msg);
        }
    };
    let listen_addr = format!("{}:{}", config.bind_host, running.port);
    ctx.status
        .ready(Some(listen_addr.clone()), Some(format!("ports {}", config.ports)));
    linelog::info(
        "allowlist helper ready",
        json!({
            "listen": listen_addr,
            "dryRun": config.dry_run,
            "firewall": config.firewall.as_str(),
            "ports": config.ports,
            "ttlSeconds": config.ttl.as_secs(),
            "minIntervalMs": config.min_interval.as_millis() as u64,
            "maxEntries": config.max_entries,
            "allowPrivate": config.allow_private,
            "auditLog": config.audit_log.display().to_string(),
            "stateFile": config.state_file.display().to_string(),
        }),
    );
    ctx.wait_stop().await;
    ctx.status.stopping();
    linelog::info("stopping: closing the game ports", json!({ "signal": "stop" }));
    if tokio::time::timeout(Duration::from_secs(90), helper.close(running))
        .await
        .is_err()
    {
        let msg = "Shutdown took longer than 90 s".to_owned();
        ctx.status.failed(msg.clone());
        return Err(msg);
    }
    ctx.status.stopped();
    Ok(())
}

/// The `Content-Length` a request declares, read as Node reads it (`Number(header)`); none is 0.
pub fn declared_length(header: Option<&str>) -> f64 {
    header.map_or(0.0, |v| service_kit::js::number(Some(&json!(v))))
}

/// An `/allow` body: `None` when it is not JSON, else the `ip` member of an object (the gateway's
/// `{"ip": "203.0.113.7"}`), which [`check_allowlist_ip`] then judges.
pub fn allow_body(body: &[u8]) -> Option<Option<Value>> {
    let body = serde_json::from_slice::<Value>(body).ok()?;
    Some(body.as_object().and_then(|o| o.get("ip")).cloned())
}

/// Mode 0600 where the file system has modes.
fn write_private(file: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(file)?.write_all(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_compare_by_hash() {
        assert!(secret_matches(Some(b"abc"), "abc"));
        assert!(!secret_matches(Some(b"abd"), "abc"));
        assert!(!secret_matches(None, "abc"));
    }

    #[test]
    fn refused_values_as_the_audit_shows_them() {
        assert_eq!(shown(Some(&json!("a\u{1}b"))), "a?b");
        assert_eq!(shown(Some(&json!("x".repeat(100)))).len(), 64);
        assert_eq!(shown(Some(&json!(7))), "number");
        assert_eq!(shown(Some(&json!(null))), "object");
        assert_eq!(shown(None), "undefined");
    }
}
