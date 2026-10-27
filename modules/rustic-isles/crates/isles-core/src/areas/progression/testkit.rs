//! A progression server for tests: the real handlers over a real SQLite database, the body parser the pipeline will
//! have, and an auth layer driven by request headers in place of auth's middleware. Not for production code.
//!
//! `x-test-role` is `gs`, `player` or `host` (none: the request carries no `AuthData`); `x-test-user` is the token's
//! `userId`; `x-test-roster` is a host's comma-separated roster.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use module_api::network::StandaloneHome;
use module_api::{BoxFuture, NodeIdentity};
use rifts_seam::{
    AccountId as RiftsAccount, CapabilityScope, DedicatedOnly, GlobalId, GuardHooks, HostWrite, HostWriteKind,
    HostWriteOptions, InventoryDecision, InventoryTransaction, Json as RiftsJson, Lifecycle, NodeId, Reply,
    RequestInfo, Rifts, RiftsParts, RuntimeAuth, SeamError, SessionId, Ulid,
};
use store::DatabaseOptions;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::db::IslesStore;
use crate::game_data::GameDataRoot;
use crate::http::{Handlers, Shared, app};
use crate::ids::{AccountId, UserId};
use crate::repo::auth::AccountRecord;
use crate::routes::Service;
use crate::settings::Settings;
use crate::version::IslesVersion;

use super::{AuthData, Ports, register_with};

pub mod golden;

pub use store::DatabaseConfig;

/// A host guard that records what it was asked and answers as told: allow (the default), or refuse with a reply.
#[derive(Default)]
pub struct RecordingGuard {
    pub authorized: Mutex<Vec<(HostWriteKind, String, HostWriteOptions)>>,
    pub records: Mutex<Vec<HostWrite>>,
    pub denial: Mutex<Option<Reply>>,
    /// Kinds a probe call refuses even when `denial` is empty (a session that takes the grant as another kind)
    pub probe_refuses: Mutex<Vec<HostWriteKind>>,
    pub policy_refuses: AtomicBool,
    pub notes: Mutex<Vec<String>>,
}

impl RecordingGuard {
    pub fn new(policy_refuses: bool) -> Arc<Self> {
        let guard = Self::default();
        guard.policy_refuses.store(policy_refuses, Ordering::SeqCst);
        Arc::new(guard)
    }

    pub fn tagged(&self) -> Vec<(HostWriteKind, String, serde_json::Value)> {
        self.records
            .lock()
            .map(|records| {
                records
                    .iter()
                    .map(|w| {
                        let summary = serde_json::from_str(&w.summary.0).unwrap_or(serde_json::Value::Null);
                        (w.kind, w.target.0.clone(), summary)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn asked(&self) -> Vec<(HostWriteKind, String)> {
        self.authorized
            .lock()
            .map(|asked| asked.iter().map(|(kind, target, _)| (*kind, target.clone())).collect())
            .unwrap_or_default()
    }
}

impl Lifecycle for RecordingGuard {
    fn runtime_safety_issues(&self, settings: &dyn rifts_seam::Settings) -> Result<Vec<String>, SeamError> {
        DedicatedOnly.runtime_safety_issues(settings)
    }
    fn startup_summary(&self, role: &str) -> Result<Option<RiftsJson>, SeamError> {
        DedicatedOnly.startup_summary(role)
    }
    fn start(&self, role: &str) -> Result<(), SeamError> {
        DedicatedOnly.start(role)
    }
    fn stop(&self) -> BoxFuture<'_, Result<(), SeamError>> {
        Box::pin(async { Ok(()) })
    }
    fn tables(&self) -> &'static [rifts_seam::TableSpec] {
        &[]
    }
    fn account_deletion_steps(&self) -> Vec<rifts_seam::DeletionStep> {
        Vec::new()
    }
}

impl GuardHooks for RecordingGuard {
    fn service_roles(&self) -> &'static [&'static str] {
        &[]
    }
    fn guard_manifest_roles(&self) -> &'static [&'static str] {
        &[]
    }
    fn census(&self, _: &RequestInfo) -> Result<(), SeamError> {
        Ok(())
    }
    fn rate_limit_bucket(&self, _: RequestInfo) -> BoxFuture<'_, Result<Option<String>, SeamError>> {
        Box::pin(async { Ok(None) })
    }
    fn authorize_inventory(
        &self,
        runtime: RuntimeAuth,
        transaction: InventoryTransaction,
    ) -> BoxFuture<'_, Result<InventoryDecision, SeamError>> {
        DedicatedOnly.authorize_inventory(runtime, transaction)
    }
    fn authorize_host_write(
        &self,
        _: RuntimeAuth,
        kind: HostWriteKind,
        target: RiftsAccount,
        options: HostWriteOptions,
    ) -> BoxFuture<'_, Result<Option<Reply>, SeamError>> {
        if let Ok(mut asked) = self.authorized.lock() {
            asked.push((kind, target.0, options));
        }
        let refused = options.probe && self.probe_refuses.lock().is_ok_and(|kinds| kinds.contains(&kind));
        let denial = self.denial.lock().ok().and_then(|d| d.clone());
        Box::pin(async move {
            if refused && denial.is_none() {
                return Ok(Some(Reply {
                    status: 403,
                    body: RiftsJson(r#"{"code":"P2P_HOST_WRITE_DENIED","message":"probe","payload":null}"#.into()),
                }));
            }
            Ok(denial)
        })
    }
    fn record_host_write(&self, _: RuntimeAuth, write: HostWrite) -> BoxFuture<'_, Result<(), SeamError>> {
        if let Ok(mut records) = self.records.lock() {
            records.push(write);
        }
        Box::pin(async { Ok(()) })
    }
    fn host_policy_refuses(&self) -> Result<bool, SeamError> {
        Ok(self.policy_refuses.load(Ordering::SeqCst))
    }
    fn note_would_refuse(&self, what: &str, detail: &str) -> Result<(), SeamError> {
        if let Ok(mut notes) = self.notes.lock() {
            notes.push(format!("{what} {detail}"));
        }
        Ok(())
    }
}

/// `x-test-role` and friends as `AuthData`.
pub async fn auth_layer(mut request: Request, next: Next) -> Response {
    let data = auth_data(&request);
    if let Some(data) = data {
        request.extensions_mut().insert(data);
    }
    next.run(request).await
}

fn auth_data(request: &Request) -> Option<AuthData> {
    let header = |name: &str| header_of(request, name);
    let user = header("x-test-user");
    match header("x-test-role").as_deref() {
        Some("gs") => Some(AuthData {
            is_gameserver: true,
            user_id: user,
            ..AuthData::default()
        }),
        Some("player") => Some(AuthData {
            user_id: user,
            ..AuthData::default()
        }),
        Some("host") => {
            let roster: Vec<String> = header("x-test-roster")
                .map(|r| r.split(',').filter(|s| !s.is_empty()).map(str::to_owned).collect())
                .unwrap_or_default();
            let node = NodeId::new("local").ok();
            let runtime = node.map(|node| RuntimeAuth {
                capability_id: "cap-1".into(),
                session: SessionId(GlobalId {
                    ulid: Ulid::from_parts(1_790_000_000_000, 1),
                    node,
                }),
                generation: 1,
                host_account: RiftsAccount(user.clone().unwrap_or_default()),
                scopes: vec![CapabilityScope::GameServerRuntime],
                roster: roster.iter().map(|id| RiftsAccount(id.clone())).collect(),
            });
            Some(AuthData {
                is_gameserver: false,
                is_player_host_runtime: true,
                user_id: user,
                roster,
                runtime,
            })
        }
        _ => None,
    }
}

fn header_of(request: &Request, name: &str) -> Option<String> {
    request
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

pub struct Options {
    pub version: IslesVersion,
    pub game_data: Option<PathBuf>,
    pub database: DatabaseConfig,
    pub settings: Vec<(String, String)>,
    pub ports: Ports,
    pub guard: Option<Arc<RecordingGuard>>,
}

pub struct Harness {
    pub address: SocketAddr,
    pub shared: Arc<Shared>,
    pub store: IslesStore,
    pub guard: Option<Arc<RecordingGuard>>,
}

impl Harness {
    pub async fn start(options: Options) -> Result<Self, String> {
        let node = NodeIdentity::standalone();
        let home = Arc::new(StandaloneHome { identity: node.clone() });
        let config = options.database;
        let store =
            IslesStore::open(&config, &DatabaseOptions::default(), home, &node).map_err(|error| error.to_string())?;
        let pairs: Vec<(&str, &str)> = options.settings.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let rifts = match &options.guard {
            Some(guard) => Rifts::new(RiftsParts {
                realms: Arc::new(DedicatedOnly),
                entry: Arc::new(DedicatedOnly),
                guard: guard.clone(),
                routes: vec![Arc::new(AllRiftsRoutes::new())],
            }),
            None => Rifts::dedicated_only(),
        };
        let mut shared = Shared::bare(options.version);
        shared.settings = Settings::from_pairs(pairs);
        shared.node = node;
        shared.game_data = GameDataRoot {
            backend: options.game_data,
            director: None,
        };
        shared.store = Some(store.clone());
        shared.rifts = rifts;
        let shared = Arc::new(shared);
        let mut handlers = Handlers::default();
        register_with(&mut handlers, options.ports);
        let router = app(shared.clone(), Service::Backend, &handlers)
            .map_err(|error| error.to_string())?
            .layer(axum::middleware::from_fn(auth_layer))
            .layer(axum::middleware::from_fn_with_state(
                service_kit::body::BodyConfig {
                    limit: 100 * 1024,
                    ..service_kit::body::BodyConfig::default()
                },
                service_kit::body::parse_bodies,
            ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| error.to_string())?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        tokio::spawn(async move { axum::serve(listener, router).await });
        Ok(Self {
            address,
            shared,
            store,
            guard: options.guard,
        })
    }

    /// A game account with this Node `userId`.
    pub async fn account(&self, user_id: &str) -> Result<AccountId, String> {
        let id = AccountId::new(&self.shared.node.node);
        let record = AccountRecord {
            id: id.clone(),
            user_id: UserId(user_id.to_owned()),
            name: user_id.to_owned(),
            notes: 0,
        };
        let actor = module_api::ActorContext::new(module_api::Actor::System);
        self.store
            .account_write(&id, &actor, move |tx, repos| repos.accounts.create(tx, &record))
            .await
            .map_err(|error| error.to_string())?;
        Ok(id)
    }

    pub async fn send(&self, request: &TestRequest) -> Result<TestResponse, String> {
        send(self.address, request).await.map_err(|error| error.to_string())
    }
}

#[derive(Debug, Clone, Default)]
pub struct TestRequest {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

impl TestRequest {
    pub fn new(method: &str, path: &str) -> Self {
        Self {
            method: method.into(),
            path: path.into(),
            ..Self::default()
        }
    }

    pub fn as_role(mut self, role: &str, user: Option<&str>) -> Self {
        self.headers.push(("x-test-role".into(), role.into()));
        if let Some(user) = user {
            self.headers.push(("x-test-user".into(), user.into()));
        }
        self
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn json(mut self, body: &serde_json::Value) -> Self {
        self.headers.push(("content-type".into(), "application/json".into()));
        self.body = Some(body.to_string());
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TestResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl TestResponse {
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).unwrap_or(serde_json::Value::Null)
    }

    pub fn payload(&self) -> serde_json::Value {
        self.json().get("payload").cloned().unwrap_or(serde_json::Value::Null)
    }
}

/// One HTTP/1.1 request on a fresh connection.
pub async fn send(address: SocketAddr, request: &TestRequest) -> std::io::Result<TestResponse> {
    let mut stream = TcpStream::connect(address).await?;
    let body = request.body.clone().unwrap_or_default();
    let mut head = format!(
        "{} {} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Length: {}\r\n",
        request.method,
        request.path,
        body.len()
    );
    for (name, value) in &request.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await?;
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text.as_str(), ""));
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    Ok(TestResponse {
        status,
        headers,
        body: body.to_owned(),
    })
}

/// A scratch game data folder with these files.
pub fn write_game_data(dir: &Path, files: &[(&str, &serde_json::Value)]) -> std::io::Result<PathBuf> {
    let folder = dir.join("game-data");
    std::fs::create_dir_all(&folder)?;
    for (name, content) in files {
        std::fs::write(folder.join(name), content.to_string())?;
    }
    Ok(folder)
}

/// Answers every rifts row of the route table as unimplemented, so a loaded guard doesn't unmount the pipeline.
struct AllRiftsRoutes {
    routes: &'static [rifts_seam::Route],
}

impl AllRiftsRoutes {
    fn new() -> Self {
        use crate::routes::{Method, When};
        let routes: Vec<rifts_seam::Route> = crate::routes::all()
            .iter()
            .filter(|route| route.when == When::Rifts)
            .filter_map(|route| {
                let method = match route.method {
                    Method::Get => rifts_seam::Method::Get,
                    Method::Post => rifts_seam::Method::Post,
                    Method::Patch => rifts_seam::Method::Patch,
                    _ => return None,
                };
                Some(rifts_seam::Route {
                    method,
                    path: route.path,
                    mount: rifts_seam::Mount::Normal,
                    handler: "stub",
                })
            })
            .collect();
        Self {
            routes: Box::leak(routes.into_boxed_slice()),
        }
    }
}

impl rifts_seam::RouteHandler for AllRiftsRoutes {
    fn routes(&self) -> &'static [rifts_seam::Route] {
        self.routes
    }

    fn handle(&self, call: rifts_seam::RouteCall) -> BoxFuture<'_, Result<Reply, SeamError>> {
        let _ = call;
        Box::pin(async { Err(SeamError::unimplemented(rifts_seam::Component::Guard, "test route")) })
    }
}

/// A throwaway Postgres database on the server `REFORGED_TEST_POSTGRES_URL` names (`postgres://user:pw@host:port/db`),
/// dropped when it goes out of scope. `None` when the variable is not set.
pub struct Throwaway {
    server: store::PostgresConfig,
    name: String,
}

impl Throwaway {
    pub fn create() -> Option<Self> {
        use diesel::Connection as _;
        use diesel::connection::SimpleConnection as _;
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let url = std::env::var("REFORGED_TEST_POSTGRES_URL").ok()?;
        let rest = url.strip_prefix("postgres://")?;
        let (login, location) = rest.rsplit_once('@')?;
        let (user, password) = login.split_once(':').unwrap_or((login, ""));
        let (hostport, database) = location.split_once('/').unwrap_or((location, "postgres"));
        let (host, port) = hostport.split_once(':').unwrap_or((hostport, "5432"));
        let server = store::PostgresConfig {
            host: host.to_owned(),
            port: port.parse().ok()?,
            user: user.to_owned(),
            database: database.to_owned(),
            password: password.to_owned(),
            allow_remote: false,
        };
        let name = format!(
            "isles_kit_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        diesel::PgConnection::establish(&server.conninfo())
            .ok()?
            .batch_execute(&format!("CREATE DATABASE \"{name}\""))
            .ok()?;
        Some(Self { server, name })
    }

    pub fn config(&self) -> DatabaseConfig {
        DatabaseConfig::Postgres(store::PostgresConfig {
            database: self.name.clone(),
            ..self.server.clone()
        })
    }
}

impl Drop for Throwaway {
    fn drop(&mut self) {
        use diesel::Connection as _;
        use diesel::connection::SimpleConnection as _;
        if let Ok(mut admin) = diesel::PgConnection::establish(&self.server.conninfo()) {
            let _ = admin.batch_execute(&format!("DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)", self.name));
        }
    }
}
