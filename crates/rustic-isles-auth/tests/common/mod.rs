// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! A module's sign-in area on a memory store, driven through `Auth::router()` as the launcher and game would.

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "shared by several test binaries, each using only some; a panic is how a test fails"
)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use dainty_service_kit::serve::ConnectionInfo;
use http_body_util::BodyExt as _;
use rustic_isles_auth::test_support::{MapSettings, MemoryStore, RecordingMail, StubDiscord, throwaway_signing_keys};
use rustic_isles_auth::{Auth, AuthConfig, Seams};
use serde_json::{Value, json};
use tower::ServiceExt as _;

pub const SECRET: &str = "test-only-api-key-hash-secret-0123456789abcdef";
pub const GS_KEY: &str = "gs-key-1";

pub fn h(c: char) -> String {
    c.to_string().repeat(64)
}

pub struct Answer {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub text: String,
}

impl Answer {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.text).unwrap_or(Value::Null)
    }

    pub fn code(&self) -> String {
        self.json()["error"]["code"].as_str().unwrap_or("").to_owned()
    }

    pub fn message(&self) -> String {
        self.json()["error"]["message"].as_str().unwrap_or("").to_owned()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
}

pub struct Harness {
    pub auth: Arc<Auth<MemoryStore>>,
    pub settings: Arc<MapSettings>,
    pub mail: Arc<RecordingMail>,
    pub clock: Arc<AtomicI64>,
    step: AtomicUsize,
    _updates: tempfile::TempDir,
}

pub fn manifest() -> Value {
    json!({"version": "0.1.78", "targetChangelist": 647472, "sha256": h('b'), "extraFiles": [{"name": "winmm.dll", "sha256": h('c')}]})
}

impl Harness {
    pub async fn new() -> Self {
        Self::with(&[], false).await
    }

    /// §12.2's base settings plus `extra`.
    pub async fn with(extra: &[(&str, &str)], postgres: bool) -> Self {
        let updates = tempfile::tempdir().unwrap();
        let dir = updates.path().join("runtime/client/stable/windows-x86_64");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("latest.json"), manifest().to_string()).unwrap();
        let (private, public) = throwaway_signing_keys();
        let root = updates.path().to_string_lossy().into_owned();
        let exe = h('a');
        let mut pairs: Vec<(&str, &str)> = vec![
            ("AUTH_MODE", "LAUNCHER"),
            ("API_KEY_HASH_SECRET", SECRET),
            ("AUTH_SIGNING_PRIVKEY_B64", &private),
            ("AUTH_SIGNING_PUBKEY_B64", &public),
            ("GAMESERVER_API_KEYS", GS_KEY),
            ("APPROVED_EXECUTABLE_SHA256", &exe),
            ("LAUNCHER_UPDATE_ROOT", &root),
            ("RUSTIC_ISLES_OFFICIAL_PLAY", "on"),
            ("RUSTIC_ISLES_RATE_LIMIT", "off"),
            ("NODE_ENV", "production"),
        ];
        pairs.extend_from_slice(extra);
        let settings = Arc::new(MapSettings::new(&pairs));
        let clock = Arc::new(AtomicI64::new(chrono::Utc::now().timestamp_millis()));
        let mut config = AuthConfig::for_1_14_7(dainty_module_api::NodeIdentity::standalone());
        let c = Arc::clone(&clock);
        config.clock = Arc::new(move || c.load(Ordering::SeqCst));
        config.engine_is_postgres = postgres;
        let mail = Arc::new(RecordingMail::default());
        let seams = Seams {
            mail: mail.clone(),
            discord: Arc::new(StubDiscord),
            ..Seams::default()
        };
        let auth = Auth::new(config, settings.clone(), Some(MemoryStore::new()), seams);
        let handles = auth.start().await.unwrap();
        for handle in handles {
            handle.abort();
        }
        Self {
            auth,
            settings,
            mail,
            clock,
            step: AtomicUsize::new(0),
            _updates: updates,
        }
    }

    pub fn store(&self) -> &MemoryStore {
        self.auth.store_ref().unwrap()
    }

    pub fn advance(&self, ms: i64) {
        self.clock.fetch_add(ms, Ordering::SeqCst);
    }

    pub fn now(&self) -> i64 {
        self.clock.load(Ordering::SeqCst)
    }

    pub fn smtp_on(&self) {
        for (k, v) in [
            ("RUSTIC_ISLES_SMTP_HOST", "smtp.example.test"),
            ("RUSTIC_ISLES_SMTP_USER", "mailer"),
            ("RUSTIC_ISLES_SMTP_PASSWORD", "secret"),
        ] {
            self.settings.set(k, v);
        }
    }

    pub fn discord_on(&self) {
        for (k, v) in [
            ("DISCORD_CLIENT_ID", "client"),
            ("DISCORD_CLIENT_SECRET", "shh"),
            (
                "DISCORD_REDIRECT_URI",
                "https://isles.example.test/launcher/v1/auth/discord/callback",
            ),
        ] {
            self.settings.set(k, v);
        }
    }

    /// One request from a loopback peer with a per-step `x-forwarded-for` (so per-IP limits don't bite).
    pub async fn raw(
        &self,
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<(&str, String)>,
    ) -> Answer {
        let n = self.step.fetch_add(1, Ordering::SeqCst);
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("x-forwarded-for", format!("198.51.100.{}", n % 250 + 1));
        for (k, v) in headers {
            builder = builder.header(*k, *v);
        }
        let body = match body {
            Some((content_type, text)) => {
                builder = builder
                    .header("content-type", content_type)
                    .header("content-length", text.len());
                Body::from(text)
            }
            None => Body::empty(),
        };
        let mut request = builder.body(body).unwrap();
        let peer: SocketAddr = "127.0.0.1:50000".parse().unwrap();
        request.extensions_mut().insert(ConnectInfo(ConnectionInfo {
            peer,
            local: "127.0.0.1:61000".parse().unwrap(),
        }));
        let response = Arc::clone(&self.auth).router().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Answer {
            status,
            headers,
            text: String::from_utf8_lossy(&bytes).into_owned(),
        }
    }

    pub async fn get(&self, path: &str, headers: &[(&str, &str)]) -> Answer {
        self.raw(Method::GET, path, headers, None).await
    }

    pub async fn post(&self, path: &str, headers: &[(&str, &str)], body: Value) -> Answer {
        self.raw(
            Method::POST,
            path,
            headers,
            Some(("application/json", body.to_string())),
        )
        .await
    }

    pub async fn register(&self, name: Option<&str>, email: &str) -> Answer {
        let mut body = json!({"email": email, "password": "correct horse"});
        if let Some(name) = name {
            body["displayName"] = json!(name);
        }
        self.post("/launcher/v1/auth/register", &[], body).await
    }

    pub async fn login(&self, email: &str, password: &str) -> Answer {
        self.post(
            "/launcher/v1/auth/login",
            &[],
            json!({"email": email, "password": password}),
        )
        .await
    }

    /// Registers and signs in: `(access token, refresh token, userId)`.
    pub async fn account(&self, name: &str, email: &str) -> (String, String, String) {
        assert_eq!(self.register(Some(name), email).await.status, StatusCode::ACCEPTED);
        let answer = self.login(email, "correct horse").await;
        assert_eq!(answer.status, StatusCode::OK, "{}", answer.text);
        let json = answer.json();
        (
            json["accessToken"].as_str().unwrap().to_owned(),
            json["refreshToken"].as_str().unwrap().to_owned(),
            json["account"]["userId"].as_str().unwrap().to_owned(),
        )
    }

    pub fn launch_body() -> Value {
        json!({
            "runtimeChannel": "stable",
            "buildChangelist": 647472,
            "executableSha256": h('a'),
            "runtimeManifestVersion": "0.1.78",
            "runtimeArtifacts": [{"name": "rustic-isles-runtime.dll", "sha256": h('b')}, {"name": "winmm.dll", "sha256": h('c')}]
        })
    }

    /// A game token through a launch and the exchange.
    pub async fn game_token(&self, access: &str) -> String {
        let auth = format!("Bearer {access}");
        let launch = self
            .post(
                "/launcher/v1/game-sessions",
                &[("authorization", &auth)],
                Self::launch_body(),
            )
            .await;
        assert_eq!(launch.status, StatusCode::OK, "{}", launch.text);
        let code = launch.json()["exchangeCode"].as_str().unwrap().to_owned();
        let token = self
            .post(
                "/account/api/oauth/token",
                &[],
                json!({"grant_type": "exchange_code", "exchange_code": code}),
            )
            .await;
        assert_eq!(token.status, StatusCode::OK, "{}", token.text);
        token.json()["access_token"].as_str().unwrap().to_owned()
    }
}

pub fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}
