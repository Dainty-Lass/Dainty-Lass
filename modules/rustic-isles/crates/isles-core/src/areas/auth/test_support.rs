//! A whole auth area for tests: a module over a real SQLite file, a recording code mail and a stand-in Discord, served
//! on 127.0.0.1 through the pipeline the way a module serves it. Not for production code.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use module_api::BoxFuture;
use module_api::NodeIdentity;
use module_api::network::StandaloneHome;
use serde_json::{Value, json};
use service_kit::serve::serve;
use store::{DatabaseConfig, DatabaseOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::crypto::{KeyScope, hash_api_key, hash_password};
use super::discord::{DiscordApi, DiscordConfig, DiscordFailure, DiscordUser};
use super::launcher::create_session_for_user;
use super::mail::{MailError, MailMessage, MailTransport};
use super::tokens::{sign_game_token, test_key_settings};
use crate::db::IslesStore;
use crate::hooks::{
    Destination, Entitlement, GameMode, NoPlayerPolicy, ObjectiveRow, ProgressionConfigRule, StubReply, VersionHooks,
};
use crate::http::{Shared, app};
use crate::repo::auth::{ApprovalStatus, LauncherAccountRecord, OperationalStatus};
use crate::routes::{RouteSet, Service};
use crate::settings::Settings;
use crate::tables::{StarterManifest, VersionTables};
use crate::{AccountId, IslesVersion, UserId};

/// Which module's hooks a test serves: the two sides of RU3c's `game_session_payload`, `network_encryption_keys` and
/// `no_player_policy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestVersion {
    V1147,
    V1120,
}

struct Hooks(TestVersion);

impl VersionHooks for Hooks {
    fn no_player_policy(&self) -> NoPlayerPolicy {
        match self.0 {
            TestVersion::V1147 => NoPlayerPolicy::ConfiguredAccount,
            TestVersion::V1120 => NoPlayerPolicy::Off,
        }
    }
    fn game_session_payload(&self, token: &str) -> Value {
        match self.0 {
            TestVersion::V1147 => {
                json!({ "error_code": null, "sessionid": token, "sessiontoken": token, "sessionToken": token })
            }
            TestVersion::V1120 => json!({ "error_code": null, "sessionid": "SESSION_ID_LOL", "sessionToken": token }),
        }
    }
    fn network_encryption_keys(&self) -> bool {
        self.0 == TestVersion::V1147
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
    fn candidate_destination(&self, hunt_id: &str, _: bool) -> Destination {
        Destination {
            game_mode: GameMode::Island,
            hunt_id: Some(hunt_id.to_owned()),
        }
    }
}

/// The changelist the tests' launcher requests carry.
pub const TEST_CHANGELIST: u32 = 647_472;

const TABLES: VersionTables = VersionTables {
    target_changelist: TEST_CHANGELIST,
    game_version: "1.14.7",
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
    switch_defaults: &[],
};

pub fn test_version(version: TestVersion) -> IslesVersion {
    IslesVersion {
        hooks: Arc::new(Hooks(version)),
        tables: TABLES,
        routes: match version {
            TestVersion::V1147 => RouteSet::V1147,
            TestVersion::V1120 => RouteSet::V1120,
        },
    }
}

/// The code mail, recorded instead of sent. `fail` makes every send fail.
#[derive(Default)]
pub struct RecordingMail {
    pub sent: Mutex<Vec<MailMessage>>,
    pub fail: std::sync::atomic::AtomicBool,
}

impl RecordingMail {
    /// The six-digit code of the latest message to `address`.
    pub fn code_for(&self, address: &str) -> Option<String> {
        let sent = self.sent.lock().unwrap_or_else(PoisonError::into_inner);
        let message = sent.iter().rev().find(|m| m.to == address)?;
        let marker = message.subject.rsplit(": ").next()?;
        (marker.len() == 6 && marker.bytes().all(|b| b.is_ascii_digit())).then(|| marker.to_owned())
    }

    pub fn count_to(&self, address: &str) -> usize {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|m| m.to == address)
            .count()
    }
}

impl MailTransport for RecordingMail {
    fn send<'a>(&'a self, _: &'a Settings, message: MailMessage) -> BoxFuture<'a, Result<(), MailError>> {
        Box::pin(async move {
            if self.fail.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(MailError("ECONNECTION".into()));
            }
            self.sent.lock().unwrap_or_else(PoisonError::into_inner).push(message);
            Ok(())
        })
    }
}

/// Discord's token and user endpoints, answered by the code itself: `code-<id>` is the Discord user `<id>`.
#[derive(Default)]
pub struct FakeDiscord;

impl DiscordApi for FakeDiscord {
    fn exchange_code<'a>(
        &'a self,
        _: &'a DiscordConfig,
        code: &'a str,
        _: &'a str,
    ) -> BoxFuture<'a, Result<DiscordUser, DiscordFailure>> {
        Box::pin(async move {
            let Some(id) = code.strip_prefix("code-") else {
                return Err(DiscordFailure { status: Some(400) });
            };
            Ok(DiscordUser {
                id: id.to_owned(),
                username: format!("user-{}", &id[id.len().saturating_sub(4)..]),
                global_name: None,
                avatar: None,
            })
        })
    }
}

/// What a request got back.
#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Reply {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
}

pub struct TestServer {
    pub shared: Arc<Shared>,
    pub mail: Arc<RecordingMail>,
    pub dir: Option<Arc<tempfile::TempDir>>,
    store: IslesStore,
    address: Mutex<Option<SocketAddr>>,
    next_ip: std::sync::atomic::AtomicU32,
    version: TestVersion,
}

fn open_store(dir: &tempfile::TempDir) -> IslesStore {
    let node = NodeIdentity::standalone();
    let home = Arc::new(StandaloneHome { identity: node.clone() });
    let config = DatabaseConfig::Sqlite {
        file: dir.path().join("isles.db"),
    };
    IslesStore::open(&config, &DatabaseOptions::default(), home, &node).unwrap()
}

fn build(version: TestVersion, store: &IslesStore, extra: &[(&str, &str)]) -> (Arc<Shared>, Arc<RecordingMail>) {
    let mut shared = Shared::bare(test_version(version));
    shared.settings = test_key_settings(extra);
    shared.store = Some(store.clone());
    let mail = Arc::new(RecordingMail::default());
    shared.auth.mail = mail.clone();
    shared.auth.discord = Arc::new(FakeDiscord);
    (Arc::new(shared), mail)
}

impl TestServer {
    pub fn new(extra: &[(&str, &str)]) -> Self {
        Self::for_version(TestVersion::V1147, extra)
    }

    pub fn for_version(version: TestVersion, extra: &[(&str, &str)]) -> Self {
        let dir = Arc::new(tempfile::tempdir().unwrap());
        let store = open_store(&dir);
        let (shared, mail) = build(version, &store, extra);
        Self {
            shared,
            mail,
            dir: Some(dir),
            store,
            address: Mutex::new(None),
            next_ip: std::sync::atomic::AtomicU32::new(1),
            version,
        }
    }

    /// The same database under other settings, as a restart with a changed env file would be.
    pub fn with_store_of(other: &TestServer, extra: &[(&str, &str)]) -> Self {
        let (shared, mail) = build(other.version, &other.store, extra);
        Self {
            shared,
            mail,
            dir: other.dir.clone(),
            store: other.store.clone(),
            address: Mutex::new(None),
            next_ip: std::sync::atomic::AtomicU32::new(1),
            version: other.version,
        }
    }

    /// A launcher account, as registration makes it but with an approved state and a chosen password.
    pub async fn add_launcher_account(&self, name: &str, email: &str, password: &str) -> LauncherAccountRecord {
        self.add_launcher_account_with(name, Some(email), Some(password), |_| {})
            .await
    }

    pub async fn add_launcher_account_with(
        &self,
        name: &str,
        email: Option<&str>,
        password: Option<&str>,
        change: impl FnOnce(&mut LauncherAccountRecord),
    ) -> LauncherAccountRecord {
        let user_id = UserId::random();
        let mut record = LauncherAccountRecord {
            account: AccountId::new(&self.shared.node.node),
            user_id,
            email: email.map(str::to_owned),
            display_name_normalized: name.to_lowercase(),
            display_name: name.to_owned(),
            password_hash: password.map(|p| hash_password(p).unwrap()),
            status: OperationalStatus::Active,
            approval_status: Some(ApprovalStatus::Approved),
            approval_updated_at: None,
            approval_updated_by: None,
            approval_reason: None,
            roles: vec!["player".into()],
            roles_updated_at: None,
            roles_updated_by: None,
            created_at: "2026-07-01T00:00:00.000Z".into(),
            last_login_at: None,
            username_set: Some(true),
            consent: None,
            deletion: None,
            email_verified_at: None,
            email_verification_pending_since: None,
            timeout: None,
        };
        change(&mut record);
        let stored = record.clone();
        self.store
            .write(&super::data::system_actor(), move |tx, repos| {
                repos.launcher_accounts.create(tx, &stored)
            })
            .await
            .unwrap();
        record
    }

    /// A launcher access token and refresh token for the account.
    pub async fn launcher_session(&self, account: &LauncherAccountRecord) -> (String, String) {
        create_session_for_user(&self.shared, account, "test-device", "Test PC")
            .await
            .unwrap()
    }

    /// The game token of a user id.
    pub fn game_token(&self, user_id: &str) -> String {
        sign_game_token(&self.shared.settings, user_id).unwrap()
    }

    /// A key for the game servers; the header value it is sent as.
    pub async fn gameserver_key(&self, key: &str) -> String {
        super::apikeys::synchronize_configured_gameserver_api_keys(&self.shared, key)
            .await
            .unwrap();
        key.to_owned()
    }

    /// A key an account owns, as the Reforged API's register would store it.
    pub async fn user_key(&self, account: &LauncherAccountRecord, key: &str) {
        let hash = hash_api_key(&self.shared.settings, key, KeyScope::User).unwrap();
        let id = account.account.clone();
        self.store
            .account_write(&id.clone(), &super::data::player_actor(&id), move |tx, repos| {
                repos.api_keys.insert_user_key_hash(tx, &id, &hash)
            })
            .await
            .unwrap();
    }

    pub fn store(&self) -> &IslesStore {
        &self.store
    }

    /// Starts the backend on 127.0.0.1 through the real pipeline, once.
    pub async fn address(&self) -> SocketAddr {
        if let Some(address) = *self.address.lock().unwrap_or_else(PoisonError::into_inner) {
            return address;
        }
        let router = app(self.shared.clone(), Service::Backend, &crate::areas::backend()).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(serve(
            listener,
            router,
            std::future::pending::<()>(),
            Duration::from_secs(1),
        ));
        *self.address.lock().unwrap_or_else(PoisonError::into_inner) = Some(address);
        address
    }

    /// A client address no earlier request of this server used, so the per-address limits do not meet.
    pub fn fresh_ip(&self) -> String {
        let n = self.next_ip.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        format!("198.51.{}.{}", (n / 250) % 250, n % 250 + 1)
    }

    /// One request with a JSON body (or none), under a fresh client address unless `headers` names one.
    pub async fn call(&self, method: &str, path: &str, body: Option<Value>, headers: &[(&str, &str)]) -> Reply {
        let address = self.address().await;
        let mut all: Vec<(String, String)> = headers
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        if !all.iter().any(|(k, _)| k.eq_ignore_ascii_case("x-forwarded-for")) {
            all.push(("x-forwarded-for".into(), self.fresh_ip()));
        }
        send(
            address,
            method,
            path,
            body.map(|b| ("application/json", b.to_string())),
            &all,
        )
        .await
    }

    pub async fn call_as(&self, method: &str, path: &str, body: Option<Value>, bearer: &str) -> Reply {
        let header = format!("Bearer {bearer}");
        self.call(method, path, body, &[("authorization", header.as_str())])
            .await
    }
}

/// One HTTP/1.1 request on a fresh connection.
pub async fn send(
    address: SocketAddr,
    method: &str,
    path: &str,
    body: Option<(&str, String)>,
    headers: &[(String, String)],
) -> Reply {
    let mut stream = TcpStream::connect(address).await.unwrap();
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n");
    for (name, value) in headers {
        head += &format!("{name}: {value}\r\n");
    }
    let payload = body.as_ref().map_or("", |(_, text)| text.as_str());
    if let Some((content_type, _)) = &body {
        head += &format!("content-type: {content_type}\r\n");
    }
    head += &format!("content-length: {}\r\n\r\n", payload.len());
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(payload.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
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
    Reply {
        status,
        headers,
        body: dechunk(body),
    }
}

/// The body of a chunked answer (axum writes chunked when it does not know the length).
fn dechunk(body: &str) -> String {
    let mut rest = body;
    let mut out = String::new();
    loop {
        let Some((size, after)) = rest.split_once("\r\n") else {
            return body.to_owned();
        };
        let Ok(size) = usize::from_str_radix(size.trim(), 16) else {
            return body.to_owned();
        };
        if size == 0 {
            return out;
        }
        if after.len() < size {
            return body.to_owned();
        }
        out.push_str(&after[..size]);
        rest = after[size..].trim_start_matches("\r\n");
    }
}

/// The values of a JSON object's keys, in order.
pub fn keys(value: &Value) -> Vec<String> {
    value
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default()
}

/// A flat map of the object, for assertions that ignore order.
pub fn flat(value: &Value) -> BTreeMap<String, Value> {
    value
        .as_object()
        .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}
