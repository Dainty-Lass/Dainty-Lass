//! The replay rig of the golden tests (RU5): a scenario of requests (`tests/golden/characters/scenario.json`) is run
//! against this area's handlers on a real database, and what they answer is compared with what the Node reference
//! answered to the same requests (`node-<version>.golden.json`, recorded by `record.sh`). The module crates run it,
//! since only they hold a version's hooks and tables. Not for production code.
//!
//! Inventory, wallet and Slayer's Path are other areas' repositories: in-memory stand-ins are used until they land.

#![allow(clippy::expect_used, clippy::unwrap_used)] // a test rig

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::extract::Request;
use module_api::network::StandaloneHome;
use module_api::{Actor, ActorContext, NodeIdentity};
use regex::Regex;
use serde_json::Value;
use service_kit::jsjson;
use store::{DatabaseConfig, DatabaseOptions, PostgresConfig};

use super::{AuthData, controller};
use crate::db::IslesStore;
use crate::game_data::GameDataRoot;
use crate::http::{Call, Rifts, Shared};
use crate::repo::bounties::{WalletRecord, WalletRepository};
use crate::repo::inventory::{InventoryRecord, InventoryRepository};
use crate::repo::progression::{PlayerJourneyRecord, PlayerJourneyRepository};
use crate::repo::{AccountId, AnyConnection, RepoError, RepoResult, Repositories, Tx};
use crate::routes::{Pattern, Service};
use crate::{IslesVersion, Settings};

/// One database for one replay: SQLite in a temporary folder, or a throwaway Postgres database on the server
/// `REFORGED_TEST_POSTGRES_URL` names (`postgres://user:pw@127.0.0.1:5432/postgres`).
pub struct TestDb {
    config: DatabaseConfig,
    folder: Option<PathBuf>,
    postgres: Option<(PostgresConfig, String)>,
}

fn postgres_url(url: &str) -> Option<PostgresConfig> {
    let rest = url
        .strip_prefix("postgres://")
        .or_else(|| url.strip_prefix("postgresql://"))?;
    let (login, location) = rest.rsplit_once('@')?;
    let (user, password) = login.split_once(':').unwrap_or((login, ""));
    let (host_port, database) = location.split_once('/')?;
    let (host, port) = host_port.rsplit_once(':').unwrap_or((host_port, "5432"));
    Some(PostgresConfig {
        host: host.to_owned(),
        port: port.parse().ok()?,
        user: user.to_owned(),
        database: database.to_owned(),
        password: password.to_owned(),
        allow_remote: false,
    })
}

impl TestDb {
    pub fn sqlite() -> Self {
        let folder = std::env::temp_dir().join(format!("isles-test-{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&folder);
        Self {
            config: DatabaseConfig::Sqlite {
                file: folder.join("isles.db"),
            },
            folder: Some(folder),
            postgres: None,
        }
    }

    /// `None` unless `REFORGED_TEST_POSTGRES_URL` is set.
    pub fn postgres() -> Option<Self> {
        use diesel::Connection;
        use diesel::connection::SimpleConnection;
        let url = std::env::var("REFORGED_TEST_POSTGRES_URL").ok()?;
        let server = postgres_url(&url)?;
        let name = format!("isles_test_{}", uuid::Uuid::new_v4().simple());
        let admin_url = format!(
            "postgres://{}:{}@{}:{}/{}",
            server.user, server.password, server.host, server.port, server.database
        );
        diesel::PgConnection::establish(&admin_url)
            .ok()?
            .batch_execute(&format!("CREATE DATABASE {name}"))
            .ok()?;
        let config = PostgresConfig {
            database: name.clone(),
            ..server
        };
        Some(Self {
            config: DatabaseConfig::Postgres(config.clone()),
            folder: None,
            postgres: Some((config, admin_url)),
        })
    }

    pub fn config(&self) -> &DatabaseConfig {
        &self.config
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        if let Some(folder) = &self.folder {
            let _ = std::fs::remove_dir_all(folder);
        }
        if let Some((config, admin_url)) = &self.postgres {
            use diesel::Connection;
            use diesel::connection::SimpleConnection;
            if let Ok(mut admin) = diesel::PgConnection::establish(admin_url) {
                let _ = admin.batch_execute(&format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", config.database));
            }
        }
    }
}

fn exists(what: &str, key: &str) -> RepoError {
    RepoError::Data(format!("{what} {key} exists"))
}

/// In-memory inventories (the inventory-reforge area's table is not here yet).
#[derive(Default)]
pub struct MemInventories(Mutex<HashMap<String, InventoryRecord>>);

impl InventoryRepository for MemInventories {
    fn find_by_character_id(&self, _: &mut AnyConnection, id: &str) -> RepoResult<Option<InventoryRecord>> {
        Ok(self
            .0
            .lock()
            .map_err(|_| RepoError::Data("poisoned".into()))?
            .get(id)
            .cloned())
    }

    fn create(&self, _: &mut Tx<'_>, inventory: &InventoryRecord) -> RepoResult<()> {
        let mut map = self.0.lock().map_err(|_| RepoError::Data("poisoned".into()))?;
        if map.contains_key(&inventory.character_id) {
            return Err(exists("inventory", &inventory.character_id));
        }
        map.insert(inventory.character_id.clone(), inventory.clone());
        Ok(())
    }

    fn update_both_if_revision_matches(
        &self,
        _: &mut Tx<'_>,
        id: &str,
        instanced_items_json: &str,
        stacked_items_json: &str,
        expected_revision: i64,
    ) -> RepoResult<Option<InventoryRecord>> {
        let mut map = self.0.lock().map_err(|_| RepoError::Data("poisoned".into()))?;
        Ok(map
            .get_mut(id)
            .filter(|r| r.revision.unwrap_or(0) == expected_revision)
            .map(|record| {
                record.instanced_items = instanced_items_json.to_owned();
                record.stacked_items = stacked_items_json.to_owned();
                record.revision = Some(expected_revision + 1);
                record.clone()
            }))
    }
}

/// In-memory wallets.
#[derive(Default)]
pub struct MemWallets(Mutex<HashMap<AccountId, WalletRecord>>);

impl WalletRepository for MemWallets {
    fn find(&self, _: &mut AnyConnection, account: &AccountId) -> RepoResult<Option<WalletRecord>> {
        Ok(self
            .0
            .lock()
            .map_err(|_| RepoError::Data("poisoned".into()))?
            .get(account)
            .cloned())
    }

    fn create(&self, _: &mut Tx<'_>, wallet: &WalletRecord) -> RepoResult<()> {
        self.0
            .lock()
            .map_err(|_| RepoError::Data("poisoned".into()))?
            .insert(wallet.account.clone(), wallet.clone());
        Ok(())
    }

    fn create_if_missing(&self, _: &mut Tx<'_>, wallet: &WalletRecord) -> RepoResult<WalletRecord> {
        let mut map = self.0.lock().map_err(|_| RepoError::Data("poisoned".into()))?;
        Ok(map
            .entry(wallet.account.clone())
            .or_insert_with(|| wallet.clone())
            .clone())
    }

    fn increment_balance(&self, _: &mut Tx<'_>, _: &AccountId, _: &str, _: f64) -> RepoResult<Option<WalletRecord>> {
        Ok(None)
    }
}

/// In-memory Slayer's Path rows.
#[derive(Default)]
pub struct MemJourneys(Mutex<HashMap<AccountId, PlayerJourneyRecord>>);

impl PlayerJourneyRepository for MemJourneys {
    fn find(&self, _: &mut AnyConnection, account: &AccountId) -> RepoResult<Option<PlayerJourneyRecord>> {
        Ok(self
            .0
            .lock()
            .map_err(|_| RepoError::Data("poisoned".into()))?
            .get(account)
            .cloned())
    }

    fn create_if_absent(&self, _: &mut Tx<'_>, record: &PlayerJourneyRecord) -> RepoResult<bool> {
        let mut map = self.0.lock().map_err(|_| RepoError::Data("poisoned".into()))?;
        if map.contains_key(&record.account) {
            return Ok(false);
        }
        map.insert(record.account.clone(), record.clone());
        Ok(true)
    }

    fn update_if_revision(&self, _: &mut Tx<'_>, _: &AccountId, _: &str, _: i64, _: i64) -> RepoResult<bool> {
        Ok(false)
    }
}

/// The repositories a replay runs on: this area's on `store`, the others in memory.
#[derive(Default, Clone)]
pub struct Fakes {
    pub inventories: Arc<MemInventories>,
    pub wallets: Arc<MemWallets>,
    pub journeys: Arc<MemJourneys>,
}

impl Fakes {
    /// Loses a character's inventory, as a damaged database would.
    pub fn inventories_clear(&self, character_id: &str) {
        if let Ok(mut map) = self.inventories.0.lock() {
            map.remove(character_id);
        }
    }

    pub fn repositories(&self) -> Repositories {
        Repositories {
            inventories: self.inventories.clone(),
            wallets: self.wallets.clone(),
            player_journey: self.journeys.clone(),
            ..Repositories::on_store()
        }
    }
}

/// An open store on `db` with `fakes` in place of the repositories that are not ported.
pub fn open_store(db: &TestDb, fakes: &Fakes) -> IslesStore {
    let node = NodeIdentity::standalone();
    let home = Arc::new(StandaloneHome { identity: node.clone() });
    IslesStore::open(db.config(), &DatabaseOptions::default(), home, &node)
        .expect("the test database opens")
        .with_repos(fakes.repositories())
}

/// What a request answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub name: String,
    pub status: u16,
    pub content_type: Option<String>,
    pub body: String,
}

/// `req.AuthData` for a scenario's `as`: `player:<user>`, `gameserver` (no bearer) or `gameserver:<user>`.
fn auth_for(kind: &str, version: &IslesVersion, settings: &Settings) -> AuthData {
    let (role, user) = kind
        .split_once(':')
        .map_or((kind, None), |(role, user)| (role, Some(user)));
    match role {
        "gameserver" => AuthData {
            // The middleware names a bearer-less gameserver call after the configured account (1.14.7)
            user_id: user
                .map(str::to_owned)
                .or_else(|| version.hooks.no_player_policy().bearerless_account(settings)),
            is_gameserver: true,
            host_runtime: None,
        },
        _ => AuthData {
            user_id: user.map(str::to_owned),
            is_gameserver: false,
            host_runtime: None,
        },
    }
}

fn replace_tokens(text: &str, saved: &BTreeMap<String, String>) -> String {
    let mut out = text.to_owned();
    for (name, value) in saved {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

fn pick(value: &Value, path: &str) -> String {
    let found = path.split('.').try_fold(value, |v, key| match v {
        Value::Array(items) => key.parse::<usize>().ok().and_then(|i| items.get(i)),
        other => other.get(key),
    });
    match found {
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => "undefined".into(),
    }
}

/// One request to the backend as `auth` made it: the route Express would match, this area's handler, what it answered.
pub async fn call_once(
    shared: &Arc<Shared>,
    method: &str,
    path: &str,
    auth: AuthData,
    body: Option<&str>,
    content_type: Option<&str>,
) -> Reply {
    let handlers = crate::areas::backend();
    let http_method = axum::http::Method::from_bytes(method.as_bytes()).unwrap_or_default();
    let (route, params) = crate::routes::all()
        .iter()
        .filter(|r| r.service == Service::Backend && shared.version.routes.serves(r) && r.method.accepts(&http_method))
        .find_map(|r| {
            let matched = Pattern::parse(r.path).ok()?.matches(path)?.ok()?;
            Some((r, matched))
        })
        .expect("a route matches the request");
    let mut builder = Request::builder().method(http_method).uri(path);
    if let Some(content_type) = content_type {
        builder = builder.header("content-type", content_type);
    }
    let mut request = builder
        .body(body.map_or_else(Body::empty, |text| Body::from(text.to_owned())))
        .expect("a request");
    request.extensions_mut().insert(auth);
    let handler = handlers.get(route).expect("the area handles the route").clone();
    let call = Call {
        shared: shared.clone(),
        route,
        params,
        request,
        origin_host: None,
        request_id: "golden-request".into(),
    };
    let (parts, body) = handler(call).await.into_parts();
    let bytes = to_bytes(body, usize::MAX).await.expect("a response body");
    Reply {
        name: String::new(),
        status: parts.status.as_u16(),
        content_type: parts
            .headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned),
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

/// Runs a scenario: `setEnv` steps change the settings, `setup` steps make Slayer's Path rows, the rest are requests.
pub async fn replay(version: IslesVersion, store: IslesStore, scenario: &Value) -> Vec<Reply> {
    let mut env: BTreeMap<String, String> = BTreeMap::new();
    let mut saved = BTreeMap::new();
    let mut replies = Vec::new();
    let actor = ActorContext::new(Actor::System);
    let steps = scenario["steps"].as_array().cloned().unwrap_or_default();
    for step in &steps {
        if let Some(set) = step.get("setEnv").and_then(Value::as_object) {
            for (name, value) in set {
                env.insert(name.clone(), value.as_str().unwrap_or_default().to_owned());
            }
            continue;
        }
        if step.get("setup").and_then(Value::as_str) == Some("journey") {
            let user = step["user"].as_str().unwrap_or_default();
            if let Ok(Some(account)) = controller::find_account(&store, user).await {
                let record = PlayerJourneyRecord {
                    account: account.clone(),
                    nodes: step["nodes"].to_string(),
                    update_version: 1,
                    revision: 0,
                };
                let _ = store
                    .account_write(&account, &actor, move |tx, repos| {
                        repos.player_journey.create_if_absent(tx, &record)
                    })
                    .await;
            }
            continue;
        }
        let settings = Settings::from_pairs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        let shared = Arc::new(Shared {
            version: version.clone(),
            settings: settings.clone(),
            node: NodeIdentity::standalone(),
            game_data: GameDataRoot::default(),
            store: Some(store.clone()),
            rifts: Rifts::dedicated_only(),
            admin_api: false,
            matchmaking: Arc::default(),
            auth: crate::areas::auth::AuthRuntime::default(),
        });
        let method = step["method"].as_str().unwrap_or("GET");
        let path = replace_tokens(step["path"].as_str().unwrap_or("/"), &saved);
        let (body, content_type) = if let Some(raw) = step.get("rawBody").and_then(Value::as_str) {
            (
                Some(raw.to_owned()),
                Some(step["contentType"].as_str().unwrap_or("application/json").to_owned()),
            )
        } else if let Some(json) = step.get("body") {
            (
                Some(replace_tokens(&json.to_string(), &saved)),
                Some("application/json".to_owned()),
            )
        } else {
            (None, None)
        };
        let auth = auth_for(step["as"].as_str().unwrap_or("player"), &version, &settings);
        let reply = call_once(&shared, method, &path, auth, body.as_deref(), content_type.as_deref()).await;
        let text = reply.body.clone();
        if let Some(save) = step.get("save").and_then(Value::as_object)
            && let Ok(parsed) = serde_json::from_str::<Value>(&text)
        {
            for (name, path) in save {
                saved.insert(name.clone(), pick(&parsed, path.as_str().unwrap_or_default()));
            }
        }
        replies.push(Reply {
            name: step["name"].as_str().unwrap_or_default().to_owned(),
            ..reply
        });
    }
    replies
}

/// Makes the volatile parts of a run comparable: every uuid and instance id becomes a number by first appearance
/// (so which ones are the same still matters), and the clock-bound dates a fixed word.
struct Normalizer {
    ids: HashMap<String, usize>,
    patterns: [(Regex, &'static str); 4],
}

impl Normalizer {
    fn new() -> Self {
        let regex = |p: &str| Regex::new(p).expect("a valid pattern");
        Self {
            ids: HashMap::new(),
            patterns: [
                (regex(r"\b\d{4}\.\d{2}\.\d{2}-\d{2}\.\d{2}\.\d{2}\b"), "<time>"),
                (
                    regex(r"\b(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) \d{1,2}, \d{4}\b"),
                    "<date>",
                ),
                (
                    regex(r"\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b"),
                    "uuid",
                ),
                (regex(r"\b[A-Z0-9]{26}\b"), "iid"),
            ],
        }
    }

    /// Numbers the ids of a golden's whole run first.
    fn seeded(golden: &Value, mask: fn(&str) -> String) -> Self {
        let mut norm = Self::new();
        for step in golden["steps"].as_array().into_iter().flatten() {
            norm.apply(&mask(step["body"].as_str().unwrap_or_default()));
        }
        norm
    }

    /// Keeps the numbering, for a second pass over the same run.
    fn restart(self) -> Self {
        self
    }

    fn apply(&mut self, text: &str) -> String {
        let mut out = text.to_owned();
        let ids = &mut self.ids;
        for (pattern, label) in &self.patterns {
            out = pattern
                .replace_all(&out, |caps: &regex::Captures<'_>| {
                    if label.starts_with('<') {
                        return (*label).to_owned();
                    }
                    let next = ids.len();
                    let n = *ids.entry(caps[0].to_owned()).or_insert(next);
                    format!("<{label}{n}>")
                })
                .into_owned();
        }
        out
    }
}

/// How `replies` differ from the Node `golden` (`{"steps": [{name, status, contentType, body}]}`): one line each.
pub fn differences(golden: &Value, replies: &[Reply]) -> Vec<String> {
    compare(golden, replies, |body| body.to_owned())
}

/// What RU3c unifies on 1.12 (the 1.14.7 behaviour on both versions) and a reply therefore doesn't show the same
/// way: the Dojo series is reconciled on 1.12 too, the wire slot counts and the active slot are the 1.14.7 ones, the
/// default chat wheels are 1.14.7's only, and each version stamps its own changelist. Applied to both sides of a
/// comparison with 1.12.0's Node answers; what it hides is covered by unit tests of the rules themselves.
pub fn mask_unified(body: &str) -> String {
    fn mask(value: &mut Value) {
        match value {
            Value::Array(items) => items.iter_mut().for_each(mask),
            Value::Object(map) => {
                if let Some(Value::String(data)) = map.get("data")
                    && let Ok(Value::Object(mut inner)) = jsjson::parse(data)
                    && inner.contains_key("LastChangelist")
                {
                    inner.remove("SERIE_dojo");
                    inner.insert("LastChangelist".into(), Value::String("<changelist>".into()));
                    let text = jsjson::stringify(&Value::Object(inner));
                    map.insert("data".into(), Value::String(text));
                }
                if let Some(Value::Object(payload)) = map.get_mut("payload") {
                    for key in [
                        "num_account_slots",
                        "max_account_slots",
                        "num_character_slots",
                        "max_character_slots",
                        "active_index",
                    ] {
                        payload.remove(key);
                    }
                    if let Some(Value::Object(persistent)) = payload.get_mut("persistent") {
                        persistent.remove("quick_chats");
                        persistent.remove("emojis");
                        // 1.12.0 seeded the banner and glider as catalog ids; the instance references that
                        // 1.14.7's 2026-10-14 fix introduced resolve to an owned item
                        persistent.insert("banner".into(), Value::Null);
                        persistent.insert("glider".into(), Value::Null);
                    }
                    for slot in payload
                        .get_mut("loadouts")
                        .and_then(Value::as_array_mut)
                        .into_iter()
                        .flatten()
                    {
                        if let Some(Value::Object(slot)) = Some(slot).filter(|s| s.get("flask").is_some()) {
                            slot.insert("flask".into(), Value::Null);
                            if let Some(Value::Object(persistent)) = slot.get_mut("persistent") {
                                persistent.insert("banner".into(), Value::Null);
                                persistent.insert("glider".into(), Value::Null);
                                persistent.remove("quick_chats");
                                persistent.remove("emojis");
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    match jsjson::parse(body) {
        Ok(mut value) => {
            mask(&mut value);
            jsjson::stringify(&value)
        }
        Err(_) => body.to_owned(),
    }
}

/// [`differences`] for 1.12.0: both sides masked ([`mask_unified`]), and the steps named in `unified` compared with
/// the 1.14.7 answers instead, because RU3c has the port answer them as 1.14.7 does (the route 1.12.0 lacked, the unlock
/// read as a delta from the Slayer's Path). `port_answers` are the steps where 1.12.0's Node threw (500) and the port
/// answers a status of its own: only that status is checked.
pub fn differences_unified(
    golden: &Value,
    golden_1_14_7: &Value,
    unified: &[&str],
    port_answers: &[(&str, u16)],
    replies: &[Reply],
) -> Vec<String> {
    let mut found = Vec::new();
    for source in [golden, golden_1_14_7] {
        if source["steps"].as_array().map(Vec::len) != Some(replies.len()) {
            found.push(format!(
                "a golden has {:?} answers, {} replayed",
                source["steps"].as_array().map(Vec::len),
                replies.len()
            ));
        }
    }
    let pick = |name: &str| if unified.contains(&name) { 1 } else { 0 };
    let goldens = [golden, golden_1_14_7];
    // Each answer set numbers its ids by first appearance over the whole run, so the sets compare step by step
    let mut want_norm = [
        Normalizer::seeded(golden, mask_unified),
        Normalizer::seeded(golden_1_14_7, mask_unified),
    ];
    let mut got_norm = Normalizer::new();
    for reply in replies {
        got_norm.apply(&mask_unified(&reply.body));
    }
    let mut got_norm = got_norm.restart();
    for (i, got) in replies.iter().enumerate() {
        if let Some((_, status)) = port_answers.iter().find(|(name, _)| *name == got.name) {
            if got.status != *status {
                found.push(format!(
                    "{}: status {} (the port answers {status})",
                    got.name, got.status
                ));
            }
            continue;
        }
        let which = pick(&got.name);
        let want = &goldens[which]["steps"][i];
        found.extend(compare_step(
            want,
            got,
            &mut want_norm[which],
            &mut got_norm,
            mask_unified,
        ));
    }
    found
}

fn compare(golden: &Value, replies: &[Reply], mask: fn(&str) -> String) -> Vec<String> {
    let steps = golden["steps"].as_array().cloned().unwrap_or_default();
    let mut found = Vec::new();
    if steps.len() != replies.len() {
        found.push(format!("{} answers recorded, {} replayed", steps.len(), replies.len()));
    }
    let (mut want_norm, mut got_norm) = (Normalizer::seeded(golden, mask), Normalizer::new());
    for reply in replies {
        got_norm.apply(&mask(&reply.body));
    }
    let mut got_norm = got_norm.restart();
    for (want, got) in steps.iter().zip(replies) {
        found.extend(compare_step(want, got, &mut want_norm, &mut got_norm, mask));
    }
    found
}

fn compare_step(
    want: &Value,
    got: &Reply,
    want_norm: &mut Normalizer,
    got_norm: &mut Normalizer,
    mask: fn(&str) -> String,
) -> Vec<String> {
    let mut found = Vec::new();
    {
        let name = &got.name;
        let want_body = want_norm.apply(&mask(want["body"].as_str().unwrap_or_default()));
        let got_body = got_norm.apply(&mask(&got.body));
        if want["status"].as_u64() != Some(u64::from(got.status)) {
            found.push(format!("{name}: status {} (Node {})", got.status, want["status"]));
        }
        let want_type = want["contentType"].as_str();
        if want_type != got.content_type.as_deref() {
            found.push(format!(
                "{name}: content-type {:?} (Node {want_type:?})",
                got.content_type
            ));
        }
        if want_body != got_body {
            found.push(format!("{name}:\n  rust {got_body}\n  node {want_body}"));
        }
    }
    found
}
