//! What this area's tests share: a module's `Shared` for either version, a Director that answers as scripted, and real
//! HTTP to the backend app with the authenticated user named by a header (standing in for `HasParadoxBackendAuth`,
//! which is RU5 auth's). Not for production code.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use module_api::BoxFuture;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::AuthData;
use super::deploy::{DedicatedServers, Launch, LaunchRequest, LiveHub, LiveHubStatus};
use super::state::MatchmakingState;
use crate::hooks::{
    Destination, Entitlement, GameMode, NoPlayerPolicy, ObjectiveRow, ProgressionConfigRule, StubReply, VersionHooks,
};
use crate::http::{Shared, app};
use crate::routes::{RouteSet, Service};
use crate::settings::{QOS_PORT, Settings};
use crate::tables::{StarterManifest, SwitchDefault, VersionTables};
use crate::version::IslesVersion;

pub const AUTH_HEADER: &str = "x-test-user";

const fn tables(
    target_changelist: u32,
    game_version: &'static str,
    switches: &'static [SwitchDefault],
) -> VersionTables {
    VersionTables {
        target_changelist,
        game_version,
        active_huntpass_id: "",
        recent_huntpass_seasons: &[],
        patchnotes_release_version: "",
        bounty_item_grant_data: &[],
        store_sections: &[],
        seasonal_coin: None,
        masked_journey_nodes: &[],
        starter_manifest: StarterManifest {
            bootstrap_version: "",
            stacked: &[],
            quick_emojis: None,
        },
        switch_defaults: switches,
    }
}

/// Tables shaped like 1.14.7's (the pong on 80 by default).
pub const TABLES: VersionTables = tables(
    647_472,
    "1.14.7",
    &[SwitchDefault {
        switch: QOS_PORT,
        value: "80",
    }],
);
/// 1.14.7's `candidate_destination` hook; the rest of the hooks are not this area's.
struct Hooks;

impl VersionHooks for Hooks {
    fn no_player_policy(&self) -> NoPlayerPolicy {
        NoPlayerPolicy::Off
    }
    fn game_session_payload(&self, _: &str) -> Value {
        Value::Null
    }
    fn weapons_at_base_level(&self, _: &Settings) -> bool {
        false
    }
    fn reforge_rules(&self) -> bool {
        false
    }
    fn mastery_persistence(&self) -> bool {
        false
    }
    fn huntpass_payload(&self, _: &str) -> Value {
        Value::Null
    }
    fn objective_row(&self, _: &str, _: &ObjectiveRow) -> Value {
        Value::Null
    }
    fn progression_config(&self) -> ProgressionConfigRule {
        ProgressionConfigRule::AsIs
    }
    fn survey_config(&self) -> StubReply {
        StubReply {
            status: 400,
            body: None,
        }
    }
    fn entitlements_payload(&self, _: &[Entitlement]) -> Value {
        Value::Null
    }
    fn candidate_destination(&self, hunt_id: &str, realm_hub: bool) -> Destination {
        let island = Destination {
            game_mode: GameMode::Island,
            hunt_id: Some(hunt_id.to_owned()),
        };
        if hunt_id == "ShatteredIsles_ReturnToRamsgate" {
            Destination {
                game_mode: GameMode::City,
                hunt_id: None,
            }
        } else if realm_hub && hunt_id.contains("TrainingDojo") {
            Destination {
                game_mode: GameMode::Shared,
                hunt_id: Some(hunt_id.to_owned()),
            }
        } else {
            island
        }
    }
}

pub fn version() -> IslesVersion {
    IslesVersion {
        hooks: Arc::new(Hooks),
        tables: TABLES,
        routes: RouteSet::V1147,
    }
}

/// A Director that answers as told and remembers what it was asked.
pub struct FakeDeploy {
    configured: bool,
    answers: Mutex<VecDeque<Result<Launch, String>>>,
    pub launches: Mutex<Vec<LaunchRequest>>,
    pub hubs: Mutex<Option<LiveHubStatus>>,
    pub hub_asks: Mutex<u32>,
}

impl FakeDeploy {
    /// Every launch lands on `host:port`.
    pub fn online(host: &str, port: i64) -> Arc<Self> {
        let launch = Launch::Online {
            host: host.to_owned(),
            port,
        };
        Arc::new(Self {
            configured: true,
            answers: Mutex::new(VecDeque::from(vec![Ok(launch); 64])),
            launches: Mutex::default(),
            hubs: Mutex::default(),
            hub_asks: Mutex::default(),
        })
    }

    pub fn failing() -> Arc<Self> {
        Arc::new(Self {
            configured: true,
            answers: Mutex::new(VecDeque::from(vec![Ok(Launch::Refused { reason: None }); 64])),
            launches: Mutex::default(),
            hubs: Mutex::default(),
            hub_asks: Mutex::default(),
        })
    }

    pub fn throwing() -> Arc<Self> {
        Arc::new(Self {
            configured: true,
            answers: Mutex::new(VecDeque::from(vec![Err("fetch failed".to_owned()); 64])),
            launches: Mutex::default(),
            hubs: Mutex::default(),
            hub_asks: Mutex::default(),
        })
    }

    pub fn launch_count(&self) -> usize {
        self.launches.lock().unwrap().len()
    }

    pub fn set_ramsgate(&self, ready: bool, host: &str, port: i64) {
        *self.hubs.lock().unwrap() = Some(LiveHubStatus {
            ramsgate: Some(LiveHub {
                ready,
                host: host.to_owned(),
                port,
            }),
            training_dojo: None,
        });
    }
}

impl DedicatedServers for FakeDeploy {
    fn configured(&self) -> bool {
        self.configured
    }

    fn launch(&self, request: LaunchRequest) -> BoxFuture<'_, Result<Launch, String>> {
        self.launches.lock().unwrap().push(request);
        let answer = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Ok(Launch::Refused { reason: None }));
        Box::pin(async move { answer })
    }

    fn hub_status(&self) -> BoxFuture<'_, Option<LiveHubStatus>> {
        *self.hub_asks.lock().unwrap() += 1;
        let hubs = self.hubs.lock().unwrap().clone();
        Box::pin(async move { hubs })
    }
}

/// A module matchmaking in `DEPLOYSERVER` mode with no Go Now delay, talking to `deploy`.
pub fn shared(pairs: &[(&str, &str)], deploy: Arc<dyn DedicatedServers>) -> Shared {
    let mut shared = Shared::bare(version());
    let mut settings = vec![
        ("MATCHMAKING_MODE", "DEPLOYSERVER"),
        ("HUNT_GO_NOW_MIN_DELAY_MS", "10000"),
        ("HUNT_GO_NOW_MAX_DELAY_MS", "10000"),
        ("HUB_GO_NOW_MIN_DELAY_MS", "5000"),
        ("HUB_GO_NOW_MAX_DELAY_MS", "5000"),
        ("GO_NOW_LOAD_PADDING_PER_JOINER_MS", "1"),
    ];
    settings.extend_from_slice(pairs);
    shared.settings = Settings::from_pairs(settings);
    shared.matchmaking = Arc::new(MatchmakingState::with_deploy(deploy));
    shared
}

async fn as_user(mut request: Request, next: Next) -> Response {
    let user = request
        .headers()
        .get(AUTH_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    if let Some(user) = user {
        request.extensions_mut().insert(AuthData {
            user_id: Some(user),
            ..AuthData::default()
        });
    }
    next.run(request).await
}

/// The backend app of `shared` on an ephemeral loopback port; `x-test-user` names the authenticated user.
pub async fn serve(shared: Arc<Shared>) -> SocketAddr {
    let router = app(shared, Service::Backend, &crate::areas::backend())
        .unwrap()
        .layer(axum::middleware::from_fn(as_user));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        service_kit::serve::serve(listener, router, std::future::pending::<()>(), Duration::from_secs(1))
            .await
            .unwrap();
    });
    address
}

#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Reply {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {:?}", self.body))
    }
}

/// One HTTP/1.1 request: `user` authenticates it, `body` is JSON.
pub async fn send(address: SocketAddr, method: &str, path: &str, user: Option<&str>, body: Option<Value>) -> Reply {
    let mut stream = TcpStream::connect(address).await.unwrap();
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n");
    if let Some(user) = user {
        head.push_str(&format!("{AUTH_HEADER}: {user}\r\n"));
    }
    let payload = body.map(|b| b.to_string()).unwrap_or_default();
    if !payload.is_empty() {
        head.push_str("Content-Type: application/json\r\n");
    }
    head.push_str(&format!("Content-Length: {}\r\n\r\n{payload}", payload.len()));
    stream.write_all(head.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, rest) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let mut lines = head.lines();
    let status = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    let chunked = headers
        .iter()
        .any(|(n, v)| n == "transfer-encoding" && v.contains("chunked"));
    let body = if chunked { dechunk(rest) } else { rest.to_owned() };
    Reply { status, headers, body }
}

fn dechunk(mut rest: &str) -> String {
    let mut out = String::new();
    while let Some((size, tail)) = rest.split_once("\r\n") {
        let size = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        if size == 0 || tail.len() < size {
            break;
        }
        out.push_str(&tail[..size]);
        rest = tail[size..].trim_start_matches("\r\n");
    }
    out
}

pub fn join_body(mode: &str, hunt: Option<&str>, party: Option<&str>) -> Value {
    let mut body = json!({ "gameMode": mode });
    if let Some(hunt) = hunt {
        body["playerHuntId"] = json!(hunt);
    }
    if let Some(party) = party {
        body["partyId"] = json!(party);
    }
    body
}
