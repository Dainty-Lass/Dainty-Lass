//! The fleet fields on a remote controller (MS6): `installs` from each install's last report, and a
//! moderation mutation given `node` queued for that install instead of run here.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use dainty_admin::config::{Config, ServerMode};
use dainty_admin::fleet::{FleetAction, FleetInstall, FleetModeration, FleetRefusal};
use dainty_admin::keys::Role;
use dainty_admin::module_control::{Module, ModuleControl, Modules, Pending};
use dainty_admin::modules::AdminPlugin;
use dainty_module_api::{Availability, GameServer, GameServerKind, UnavailableReason};
use http_body_util::{BodyExt, Full};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use serde_json::{Value, json};
use tokio::net::TcpListener;

const API_KEY: &str = "api-key-0123456789abcdef0123456789";
const START: i64 = 1_800_000_000_000;

fn hub() -> GameServer {
    GameServer {
        id: "city-1".into(),
        kind: GameServerKind::Hub,
        title: "Ramsgate".into(),
        map: "Ramsgate".into(),
        host: "203.0.113.10".into(),
        port: 7777,
        players: 3,
        max_players: 100,
        ready: false,
        started_at: None,
    }
}

/// A controller with one install that reported and one enrolled that never did.
#[derive(Default)]
struct Fleet {
    queued: Mutex<Vec<(String, FleetModeration)>>,
}

impl ModuleControl for Fleet {
    fn list(&self) -> Vec<Module> {
        Vec::new()
    }

    fn get(&self, id: &str) -> Result<Module, String> {
        Err(format!("no module {id}"))
    }

    fn load<'a>(&'a self, id: &'a str) -> Pending<'a, Module> {
        Box::pin(async move { Err(format!("no module {id}")) })
    }

    fn unload<'a>(&'a self, id: &'a str) -> Pending<'a, Module> {
        self.load(id)
    }

    fn restart<'a>(&'a self, id: &'a str) -> Pending<'a, Module> {
        self.load(id)
    }

    fn installs(&self) -> Option<Vec<FleetInstall>> {
        Some(vec![
            FleetInstall {
                node: "eu-de-1".into(),
                region: "eu-central".into(),
                label: "Frankfurt".into(),
                state: "online".into(),
                last_report_ms: Some(START),
                version: Some("2.0.0".into()),
                players_online: Some(3),
                game_servers: vec![hub()],
                availability: Some(Availability {
                    up: false,
                    reason: Some(UnavailableReason::HubNotReady),
                    hub: Some(hub()),
                    free_hunts: Some(2),
                }),
            },
            FleetInstall {
                node: "us-east-1".into(),
                region: "us-east".into(),
                label: "Virginia".into(),
                state: "offline".into(),
                last_report_ms: None,
                version: None,
                players_online: None,
                game_servers: Vec::new(),
                availability: None,
            },
        ])
    }

    fn queue_moderation(&self, node: &str, moderation: FleetModeration) -> Result<String, FleetRefusal> {
        if node != "eu-de-1" {
            return Err(FleetRefusal::UnknownNode(format!("unknown managed install: {node}")));
        }
        let mut queued = self.queued.lock().unwrap();
        queued.push((node.to_owned(), moderation));
        Ok(format!("cmd-{}", queued.len()))
    }
}

/// A controller's registry without remote management: the trait's defaults.
struct Plain;

impl ModuleControl for Plain {
    fn list(&self) -> Vec<Module> {
        Vec::new()
    }

    fn get(&self, id: &str) -> Result<Module, String> {
        Err(format!("no module {id}"))
    }

    fn load<'a>(&'a self, id: &'a str) -> Pending<'a, Module> {
        Box::pin(async move { Err(format!("no module {id}")) })
    }

    fn unload<'a>(&'a self, id: &'a str) -> Pending<'a, Module> {
        self.load(id)
    }

    fn restart<'a>(&'a self, id: &'a str) -> Pending<'a, Module> {
        self.load(id)
    }
}

struct World {
    url: String,
    admin: Arc<dainty_admin::schema::Admin>,
    _dir: tempfile::TempDir,
}

async fn world(control: Option<Arc<dyn ModuleControl>>) -> World {
    let dir = tempfile::tempdir().unwrap();
    let timeouts_file = dir.path().join("admin").join("timeouts.json");
    // Nothing listens there: the fleet path never reaches this server's backend
    let unused = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap();
    let config = Config {
        port: 0,
        bind_hosts: vec!["127.0.0.1".into()],
        backend_url: format!("http://{unused}"),
        director_url: format!("http://{unused}"),
        api_key: API_KEY.into(),
        admin_key: "admin-key-0123456789abcdef01234567".into(),
        operator_secret: "operator-secret-0123456789abcdef01".into(),
        timeouts_file: timeouts_file.clone(),
        max_hunt_servers: 3,
        server_mode: ServerMode::Public,
        database: dainty_store::DatabaseConfig::Sqlite {
            file: timeouts_file.with_file_name("dainty-lass.db"),
        },
        guard: dainty_admin::guard::GuardSettings::default(),
        metrics_token: None,
    };
    let modules = Modules(control);
    let (app, admin) =
        dainty_admin::server::prepare_with_control(&config, Arc::new(|| START), Arc::new(AdminPlugin::none()), modules)
            .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(dainty_service_kit::serve::serve(
        listener,
        dainty_admin::app::router(app),
        std::future::pending(),
        Duration::from_secs(1),
    ));
    World {
        url: format!("http://{address}"),
        admin,
        _dir: dir,
    }
}

impl World {
    async fn gql_as(&self, key: &str, query: &str) -> Value {
        let client: Client<HttpConnector, Full<Bytes>> =
            Client::builder(TokioExecutor::new()).build(HttpConnector::new());
        let request = hyper::Request::builder()
            .method("POST")
            .uri(format!("{}/graphql", self.url))
            .header("authorization", format!("Bearer {key}"))
            .header("content-type", "application/json")
            .header("x-dainty-operator-id", "42")
            .body(Full::new(Bytes::from(json!({ "query": query }).to_string())))
            .unwrap();
        let response = client.request(request).await.unwrap();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn gql(&self, query: &str) -> Value {
        self.gql_as(API_KEY, query).await
    }
}

fn code(body: &Value) -> &str {
    body["errors"][0]["extensions"]["code"].as_str().unwrap_or("")
}

#[tokio::test]
async fn installs_lists_each_node_with_its_last_report() {
    let world = world(Some(Arc::new(Fleet::default()))).await;
    let body = world
        .gql(
            "{ installs { node region label state lastReportAt version playersOnline
               availability { game up reason freeHunts hub { id } } gameServers { game id kind port } } }",
        )
        .await;
    assert_eq!(
        body["data"]["installs"],
        json!([
            {"node": "eu-de-1", "region": "eu-central", "label": "Frankfurt", "state": "online",
             "lastReportAt": "2027-01-15T08:00:00Z", "version": "2.0.0", "playersOnline": 3,
             "availability": {"game": "1.14.7", "up": false, "reason": "hub_not_ready", "freeHunts": 2,
                              "hub": {"id": "city-1"}},
             "gameServers": [{"game": "1.14.7", "id": "city-1", "kind": "HUB", "port": 7777}]},
            {"node": "us-east-1", "region": "us-east", "label": "Virginia", "state": "offline",
             "lastReportAt": null, "version": null, "playersOnline": null, "availability": null, "gameServers": []}
        ]),
        "{body}"
    );
}

#[tokio::test]
async fn a_server_that_is_not_a_controller_has_no_fleet() {
    let alone = world(None).await;
    assert_eq!(code(&alone.gql("{ installs { node } }").await), "MODULES_UNAVAILABLE");
    let plain = world(Some(Arc::new(Plain))).await;
    assert_eq!(code(&plain.gql("{ installs { node } }").await), "NO_FLEET");
    let body = plain
        .gql(r#"mutation { kick(username: "Bo_Slayer", node: "eu-de-1") { username } }"#)
        .await;
    assert_eq!(code(&body), "NO_FLEET");
}

#[tokio::test]
async fn moderation_with_a_node_is_queued_for_that_install() {
    let fleet = Arc::new(Fleet::default());
    let world = world(Some(fleet.clone())).await;
    let body = world
        .gql(r#"mutation { ban(username: "Bo_Slayer", reason: "cheating in hunts", node: "eu-de-1") { userId } }"#)
        .await;
    assert_eq!(code(&body), "QUEUED", "{body}");
    assert_eq!(body["errors"][0]["extensions"]["id"], "cmd-1");
    assert_eq!(body["errors"][0]["extensions"]["node"], "eu-de-1");

    let body = world
        .gql(r#"mutation { timeout(username: "Bo_Slayer", minutes: 30, reason: "spam", node: "eu-de-1") { userId } }"#)
        .await;
    assert_eq!(code(&body), "QUEUED", "{body}");
    let body = world
        .gql(r#"mutation { kick(username: "Bo_Slayer", node: "eu-de-1") { username } }"#)
        .await;
    assert_eq!(code(&body), "QUEUED", "{body}");

    {
        let queued = fleet.queued.lock().unwrap();
        let actions: Vec<_> = queued.iter().map(|(_, m)| m.action).collect();
        assert_eq!(actions, [FleetAction::Ban, FleetAction::Timeout, FleetAction::Kick]);
        let ban = &queued[0].1;
        assert_eq!(ban.target, "Bo_Slayer");
        assert_eq!(ban.reason.as_deref(), Some("cheating in hunts"));
        assert_eq!(ban.key, "owner");
        assert_eq!(ban.role, Role::Owner);
        assert_eq!(queued[1].1.duration_secs, Some(1800));
    }

    // Checked here as the local mutation checks them, before anything is queued
    let bad = world
        .gql(r#"mutation { ban(username: "no", reason: "cheating", node: "eu-de-1") { userId } }"#)
        .await;
    assert_eq!(code(&bad), "BAD_INPUT");
    let bad = world
        .gql(r#"mutation { ban(username: "Bo_Slayer", reason: "x", node: "eu-de-1") { userId } }"#)
        .await;
    assert_eq!(code(&bad), "BAD_INPUT");
    let bad = world
        .gql(r#"mutation { kick(username: "Bo_Slayer", node: "not a node!") { username } }"#)
        .await;
    assert_eq!(code(&bad), "BAD_INPUT");
    let unknown = world
        .gql(r#"mutation { kick(username: "Bo_Slayer", node: "ap-sg-1") { username } }"#)
        .await;
    assert_eq!(code(&unknown), "NOT_FOUND");
    assert_eq!(fleet.queued.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn a_queued_action_needs_the_role_the_local_one_needs() {
    let fleet = Arc::new(Fleet::default());
    let world = world(Some(fleet.clone())).await;
    let by = dainty_store::Actor::Host("test".into());
    let (_, moderator) = world.admin.keys.issue("Mo", Role::Moderator, by, START).await.unwrap();
    let body = world
        .gql_as(
            &moderator,
            r#"mutation { unban(username: "Bo_Slayer", node: "eu-de-1") { userId } }"#,
        )
        .await;
    assert_eq!(code(&body), "FORBIDDEN", "{body}");
    let body = world
        .gql_as(
            &moderator,
            r#"mutation { kick(username: "Bo_Slayer", node: "eu-de-1") { username } }"#,
        )
        .await;
    assert_eq!(code(&body), "QUEUED", "{body}");
    let queued = fleet.queued.lock().unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].1.role, Role::Moderator);
}
