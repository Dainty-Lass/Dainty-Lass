// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! A module over a temporary store with both halves of the launcher API, driven through core's `app()`. Every value
//! is synthetic; the Guard and update tests sign with their own key pairs.

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "shared by several test binaries, each using only some; a panic is how a test fails"
)]

use std::net::SocketAddr;
use std::sync::Arc;

use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair as _};
use axum::body::Body;
use axum::extract::{ConnectInfo, Request};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use dainty_module_api::{Actor, ActorContext};
use dainty_service_kit::serve::ConnectionInfo;
use http_body_util::BodyExt;
use rustic_isles::handlers::auth_handlers;
use rustic_isles::testing::sample_version;
use rustic_isles::{GameDataRoot, Service, Settings, Shared, SharedParts, Slots};
use rustic_isles_auth::test_support::{StubDiscord, throwaway_signing_keys};
use rustic_isles_launcher_api::admin::LauncherApiAdmin;
use rustic_isles_launcher_api::guard_manifest::Trusted;
use rustic_isles_launcher_api::test_support::FakeFunding;
use rustic_isles_launcher_api::{LauncherApiSeams, LauncherApiState, RequestObserver};
use rustic_isles_store::repo::auth::{
    AccountRecord, ApprovalStatus, LauncherAccountRecord, OperationalStatus, RefreshSessionRecord,
};
use rustic_isles_store::repo::reforged::AdminSessionRecord;
use rustic_isles_store::test_support::TempStore;
use rustic_isles_store::{AccountId, IslesAuthStore, IslesStore, UserId};
use serde_json::Value;
use tower::ServiceExt;

pub const SECRET: &str = "a-test-secret-that-is-long-enough-1234";
pub const OPERATOR_SECRET: &str = "ssssssssssssssssssssssssssssssssssssssss";
pub const ADMIN_KEY: &str = "host-admin-key-that-is-long-enough-1234";
pub const RAW_SESSION: &str = "raw-session-token";
pub const CSRF: &str = "csrf-token-1";
pub const PUBLISHER_KEY: &str = "publisher-key-1";

pub fn iso(ms: i64) -> String {
    rustic_isles_auth::iso(ms)
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub struct Keys {
    pub runtime: Ed25519KeyPair,
    pub guard: Ed25519KeyPair,
}

fn key_pair() -> Ed25519KeyPair {
    let rng = aws_lc_rs::rand::SystemRandom::new();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
    Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
}

fn raw(pair: &Ed25519KeyPair) -> [u8; 32] {
    pair.public_key().as_ref().try_into().unwrap()
}

#[derive(Default)]
pub struct Options {
    pub settings: Vec<(&'static str, String)>,
    pub store: bool,
    pub funding: Option<FakeFunding>,
}

pub fn options(store: bool) -> Options {
    Options {
        store,
        ..Options::default()
    }
}

pub struct Answer {
    pub status: u16,
    pub headers: http::HeaderMap,
    pub body: String,
}

impl Answer {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("{e}: {} {}", self.status, self.body))
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    pub fn cookies(&self) -> Vec<String> {
        self.headers
            .get_all("set-cookie")
            .iter()
            .map(|v| v.to_str().unwrap().to_owned())
            .collect()
    }

    #[track_caller]
    pub fn assert(&self, status: u16, body: &str) {
        assert_eq!((self.status, self.body.as_str()), (status, body));
    }

    #[track_caller]
    pub fn code(&self) -> String {
        let v = self.json();
        v.pointer("/error/code")
            .and_then(Value::as_str)
            .or_else(|| v.get("error").and_then(Value::as_str))
            .unwrap_or("")
            .to_owned()
    }
}

pub struct Rig {
    pub app: axum::Router,
    pub shared: Arc<Shared>,
    pub store: IslesStore,
    pub state: Arc<LauncherApiState>,
    pub admin: Arc<LauncherApiAdmin>,
    pub keys: Keys,
    pub updates: tempfile::TempDir,
    pub logs: tempfile::TempDir,
    pub data: tempfile::TempDir,
    temp: TempStore,
}

pub struct Request1<'a> {
    pub method: &'a str,
    pub uri: &'a str,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub peer: &'a str,
}

impl<'a> Request1<'a> {
    pub fn new(method: &'a str, uri: &'a str) -> Self {
        Self {
            method,
            uri,
            headers: Vec::new(),
            body: None,
            peer: "127.0.0.1:5000",
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    pub fn json(mut self, value: &Value) -> Self {
        self.body = Some(value.to_string().into_bytes());
        self.header("content-type", "application/json")
    }

    pub fn bytes(mut self, bytes: &[u8]) -> Self {
        self.body = Some(bytes.to_vec());
        self.header("content-type", "application/octet-stream")
    }

    pub fn peer(mut self, peer: &'a str) -> Self {
        self.peer = peer;
        self
    }
}

impl Rig {
    pub async fn new(options: Options) -> Self {
        let temp = TempStore::open().unwrap();
        let store = temp.store.clone();
        let updates = tempfile::tempdir().unwrap();
        let logs = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let (private, public) = throwaway_signing_keys();
        let mut pairs: Vec<(String, String)> = vec![
            ("AUTH_SIGNING_PRIVKEY_B64".into(), private),
            ("AUTH_SIGNING_PUBKEY_B64".into(), public),
            ("API_KEY_HASH_SECRET".into(), SECRET.into()),
            ("RUSTIC_ISLES_RATE_LIMIT".into(), "off".into()),
            ("RUSTIC_ISLES_ADMIN_API".into(), "on".into()),
            ("RUSTIC_ISLES_MANAGER_ADMIN".into(), "on".into()),
            ("RUSTIC_ISLES_MANAGER_OPERATOR_SECRET".into(), OPERATOR_SECRET.into()),
            ("DAINTY_ADMIN_KEY".into(), ADMIN_KEY.into()),
            ("LAUNCHER_UPDATE_ROOT".into(), updates.path().display().to_string()),
            ("LAUNCHER_LOG_STORAGE_DIR".into(), logs.path().display().to_string()),
            ("UPDATE_PUBLISHER_ALLOWED_IPS".into(), "127.0.0.1".into()),
            ("UPDATE_PUBLISHER_API_KEY".into(), PUBLISHER_KEY.into()),
            ("UPDATE_PUBLIC_BASE_URL".into(), "https://updates.example".into()),
        ];
        for (k, v) in &options.settings {
            pairs.retain(|(name, _)| name != k);
            pairs.push(((*k).to_owned(), v.clone()));
        }
        let settings = Settings::from_pairs(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        let admin_api = settings.admin_api().unwrap();
        let keys = Keys {
            runtime: key_pair(),
            guard: key_pair(),
        };
        let state = Arc::new(LauncherApiState::new());
        let funding = options.funding.unwrap_or_default();
        let admin = Arc::new(LauncherApiAdmin::new(Arc::new(StubDiscord), Arc::new(funding)));
        let seams = LauncherApiSeams::new();
        let mut parts = SharedParts::bare(sample_version());
        parts.settings = settings;
        parts.admin_api = admin_api;
        parts.observer = Arc::new(RequestObserver::new(Arc::clone(&state)));
        parts.game_data = GameDataRoot::new(Some(data.path().to_path_buf()), None);
        parts.auth_seams.activity = seams.activity();
        parts.auth_seams.notices = seams.notices();
        parts.auth_seams.operations = seams.operations();
        parts.auth_seams.erasure = seams.erasure();
        parts.auth_seams.discord = Arc::new(StubDiscord);
        let mut slots = Slots::default();
        if options.store {
            parts.auth_store = Some(Arc::new(IslesAuthStore::new(store.clone())));
            slots.insert(Arc::new(store.clone()));
        }
        slots.insert(Arc::clone(&state));
        slots.insert(Arc::clone(&admin));
        slots.insert(Arc::new(seams.clone()));
        slots.insert(Arc::new(Trusted::with_keys(raw(&keys.runtime), raw(&keys.guard))));
        parts.slots = slots;
        let mut handlers = auth_handlers().unwrap();
        rustic_isles_launcher_api::register_public(&mut handlers);
        rustic_isles_launcher_api::register_admin(&mut handlers);
        parts.handlers = handlers.clone();
        let shared = Shared::new(parts).unwrap();
        seams.bind(&shared);
        shared.set_admin_authenticator(Arc::new(rustic_isles_launcher_api::admin::AdminAuthenticatorSlot::new(
            &shared,
        )));
        let app = rustic_isles::app(Arc::clone(&shared), Service::Backend, &handlers).unwrap();
        Self {
            app,
            shared,
            store,
            state,
            admin,
            keys,
            updates,
            logs,
            data,
            temp,
        }
    }

    pub async fn send(&self, r: Request1<'_>) -> Answer {
        let mut builder = Request::builder()
            .method(r.method)
            .uri(r.uri)
            .header("host", "localhost");
        for (name, value) in &r.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        let body = r.body.unwrap_or_default();
        let mut request = builder
            .header("content-length", body.len().to_string())
            .body(Body::from(body))
            .unwrap();
        let peer: SocketAddr = r.peer.parse().unwrap();
        request.extensions_mut().insert(ConnectInfo(ConnectionInfo {
            peer,
            local: "127.0.0.1:61000".parse().unwrap(),
        }));
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        Answer {
            status,
            headers,
            body: String::from_utf8_lossy(&body).into_owned(),
        }
    }

    pub async fn get(&self, uri: &str) -> Answer {
        self.send(Request1::new("GET", uri)).await
    }

    // -------------------------------------------------------------- accounts

    /// A game account and a launcher account; the userId is `user`.
    pub async fn player(&self, user: &str, name: &str, email: Option<&str>, roles: &[&str]) -> AccountId {
        let account = AccountId::new(&self.shared.node().node);
        let game = AccountRecord {
            id: account.clone(),
            user_id: UserId::new(user),
            name: name.to_owned(),
            notes: 0,
        };
        let launcher = LauncherAccountRecord {
            account: account.clone(),
            user_id: UserId::new(user),
            email: email.map(str::to_owned),
            display_name_normalized: name.to_lowercase(),
            display_name: name.to_owned(),
            password_hash: Some(rustic_isles_auth::hash_password("correct horse").unwrap()),
            status: OperationalStatus::Active,
            approval_status: Some(ApprovalStatus::Approved),
            approval_updated_at: None,
            approval_updated_by: None,
            approval_reason: None,
            roles: roles.iter().map(|r| (*r).to_owned()).collect(),
            roles_updated_at: None,
            roles_updated_by: None,
            created_at: iso(now_ms()),
            last_login_at: None,
            username_set: Some(true),
            consent: None,
            deletion: None,
            email_verified_at: None,
            email_verification_pending_since: None,
            timeout: None,
        };
        self.store
            .write(&ActorContext::new(Actor::System), move |tx, repos| {
                repos.accounts.create(tx, &game)?;
                repos.launcher_accounts.create(tx, &launcher).map(|_| ())
            })
            .await
            .unwrap();
        account
    }

    /// A launcher bearer for the account (a live refresh family).
    pub async fn bearer(&self, user: &str, account: &AccountId) -> String {
        let family = format!("family-{user}");
        let session = RefreshSessionRecord {
            id: format!("rs-{user}"),
            token_hash: format!("hash-{user}"),
            account: account.clone(),
            family_id: family.clone(),
            device_id: "d".into(),
            device_name: "d".into(),
            created_at: iso(now_ms()),
            expires_at: iso(now_ms() + 86_400_000),
            revoked_at: None,
        };
        self.store
            .write(&ActorContext::new(Actor::System), move |tx, repos| {
                repos.refresh_sessions.create(tx, &session)
            })
            .await
            .unwrap();
        format!(
            "Bearer {}",
            self.shared.auth().sign_launcher_token(user, &family).unwrap()
        )
    }

    /// An admin session with the fixed raw token and CSRF token.
    pub async fn admin_session(&self, account: &AccountId) {
        let record = AdminSessionRecord {
            id: "session-1".into(),
            token_hash: rustic_isles_launcher_api::util::sha256_hex(RAW_SESSION.as_bytes()),
            account: account.clone(),
            csrf_token: CSRF.into(),
            created_at: iso(now_ms()),
            expires_at: iso(now_ms() + 3_600_000),
            ip: "127.0.0.1".into(),
            user_agent: "test".into(),
            revoked_at: None,
        };
        self.store
            .write(&ActorContext::new(Actor::System), move |tx, repos| {
                repos.admin.create_session(tx, &record)
            })
            .await
            .unwrap();
    }

    pub async fn launcher(&self, account: &AccountId) -> LauncherAccountRecord {
        let a = account.clone();
        self.store
            .read(move |conn, repos| repos.launcher_accounts.find_by_account(conn, &a))
            .await
            .unwrap()
            .unwrap()
    }

    pub async fn audits(&self) -> Vec<rustic_isles_store::repo::reforged::AdminAuditRecord> {
        self.store
            .read(|conn, repos| repos.admin.list_audit(conn, None, 0, 100))
            .await
            .unwrap()
            .0
    }

    // -------------------------------------------------------------- signing

    pub fn sign_runtime(&self, bytes: &[u8]) -> String {
        STANDARD.encode(self.keys.runtime.sign(bytes).as_ref())
    }

    /// A Guard envelope signed by the guard key.
    pub fn envelope(&self, payload: &Value) -> Value {
        let text = payload.to_string();
        serde_json::json!({
            "schema": 1,
            "keyId": "guard-2026-10",
            "payloadBase64": STANDARD.encode(text.as_bytes()),
            "signature": STANDARD.encode(self.keys.guard.sign(text.as_bytes()).as_ref()),
        })
    }
}

/// A Guard payload for changelist 647472, channel `stable`, valid for a month.
pub fn guard_payload(id: &str, sequence: i64, minimum: &str) -> Value {
    serde_json::json!({
        "schema": 1,
        "manifestId": id,
        "sequence": sequence,
        "channel": "stable",
        "platform": "windows-x86_64",
        "minimumLauncherVersion": minimum,
        "issuedAt": iso(now_ms() - 86_400_000),
        "expiresAt": iso(now_ms() + 30 * 86_400_000),
        "targetChangelist": 647_472,
        "artifacts": [{"name": "Dauntless.exe", "role": "GAME", "size": 1, "sha256": "a".repeat(64), "required": true}],
        "executableMemoryPolicy": {
            "maximumPrivateExecutableBytes": 1024 * 1024,
            "maximumWritableExecutableBytes": 64 * 1024,
            "allowlistedModuleHashes": []
        }
    })
}

pub fn settings(pairs: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
    pairs.iter().map(|(k, v)| (*k, (*v).to_owned())).collect()
}
