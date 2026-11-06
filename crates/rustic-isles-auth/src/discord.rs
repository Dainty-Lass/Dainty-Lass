// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Discord sign-in (spec §7.5): the settings, the authorize URL, and the two Discord calls behind a trait.

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use bytes::Bytes;
use http_body_util::{BodyExt as _, Full};
use module_api::BoxFuture;
use serde_json::Value;

use crate::AuthSettings;
use crate::util;

const AUTHORIZE_URL: &str = "https://discord.com/api/oauth2/authorize";
const TOKEN_URL: &str = "https://discord.com/api/oauth2/token";
const ME_URL: &str = "https://discord.com/api/users/@me";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_ANSWER: usize = 64 * 1024;

#[derive(Clone, PartialEq, Eq)]
pub struct DiscordConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
}

// The client secret stays out of logs (§13)
impl std::fmt::Debug for DiscordConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscordConfig")
            .field("client_id", &self.client_id)
            .field("redirect_uri", &self.redirect_uri)
            .finish_non_exhaustive()
    }
}

impl DiscordConfig {
    /// Configured when all three are non-empty.
    pub fn from_settings(settings: &dyn AuthSettings) -> Option<Self> {
        let get = |name: &str| settings.raw(name).filter(|v| !v.is_empty()).map(str::to_owned);
        Some(Self {
            client_id: get("DISCORD_CLIENT_ID")?,
            client_secret: get("DISCORD_CLIENT_SECRET")?,
            redirect_uri: get("DISCORD_REDIRECT_URI")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscordUser {
    pub id: String,
    pub username: String,
    pub global_name: Option<String>,
    pub avatar: Option<String>,
}

/// A failed call: Discord's status when it answered non-2xx (logged, never shown).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the Discord call failed")]
pub struct DiscordFailure {
    pub status: Option<u16>,
}

pub trait DiscordApi: Send + Sync {
    fn exchange_code<'a>(
        &'a self,
        config: &'a DiscordConfig,
        code: &'a str,
        verifier: &'a str,
    ) -> BoxFuture<'a, Result<DiscordUser, DiscordFailure>>;
}

/// `URLSearchParams`: `A-Za-z0-9*-._` kept, space `+`, the rest `%XX`.
pub fn search_params_encode(text: &str) -> String {
    util::percent_encode(text, |b| b.is_ascii_alphanumeric() || b"*-._".contains(&b), true)
}

/// The token form: every non-alphanumeric byte `%XX` (space `%20`).
pub fn form_encode(pairs: &[(&str, &str)]) -> String {
    let enc = |t: &str| util::percent_encode(t, |b| b.is_ascii_alphanumeric(), false);
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// The PKCE S256 challenge of a verifier.
pub fn code_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(util::sha256(verifier.as_bytes()))
}

pub fn authorize_url(config: &DiscordConfig, state: &str, verifier: &str) -> String {
    let challenge = code_challenge(verifier);
    let pairs = [
        ("client_id", config.client_id.as_str()),
        ("redirect_uri", config.redirect_uri.as_str()),
        ("response_type", "code"),
        ("scope", "identify"),
        ("state", state),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
    ];
    let query: Vec<String> = pairs
        .iter()
        .map(|(k, v)| format!("{}={}", search_params_encode(k), search_params_encode(v)))
        .collect();
    format!("{AUTHORIZE_URL}?{}", query.join("&"))
}

/// A user object as Discord answers it.
pub fn parse_user(value: &Value) -> Option<DiscordUser> {
    let text = |name: &str| value.get(name).and_then(Value::as_str).map(str::to_owned);
    Some(DiscordUser {
        id: text("id")?,
        username: text("username")?,
        global_name: text("global_name"),
        avatar: text("avatar"),
    })
}

type Client = hyper_util::client::legacy::Client<
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
    Full<Bytes>,
>;

/// The calls over HTTPS only (webpki roots, HTTP/1.1, 15 s each).
pub struct HttpsDiscord {
    client: Option<Client>,
}

impl std::fmt::Debug for HttpsDiscord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HttpsDiscord")
    }
}

impl Default for HttpsDiscord {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpsDiscord {
    pub fn new() -> Self {
        let tls = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .ok()
        .map(|builder| {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            builder.with_root_certificates(roots).with_no_client_auth()
        });
        let client = tls.map(|tls| {
            let connector = hyper_rustls::HttpsConnectorBuilder::new()
                .with_tls_config(tls)
                .https_only()
                .enable_http1()
                .build();
            hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new()).build(connector)
        });
        Self { client }
    }

    async fn call(&self, request: http::Request<Full<Bytes>>) -> Result<Value, DiscordFailure> {
        let client = self.client.as_ref().ok_or(DiscordFailure { status: None })?;
        let answer = tokio::time::timeout(REQUEST_TIMEOUT, async {
            let response = client
                .request(request)
                .await
                .map_err(|_| DiscordFailure { status: None })?;
            let status = response.status();
            let body = http_body_util::Limited::new(response.into_body(), MAX_ANSWER)
                .collect()
                .await
                .map_err(|_| DiscordFailure { status: None })?
                .to_bytes();
            if !status.is_success() {
                return Err(DiscordFailure {
                    status: Some(status.as_u16()),
                });
            }
            serde_json::from_slice::<Value>(&body).map_err(|_| DiscordFailure { status: None })
        })
        .await;
        answer.unwrap_or(Err(DiscordFailure { status: None }))
    }
}

impl DiscordApi for HttpsDiscord {
    fn exchange_code<'a>(
        &'a self,
        config: &'a DiscordConfig,
        code: &'a str,
        verifier: &'a str,
    ) -> BoxFuture<'a, Result<DiscordUser, DiscordFailure>> {
        Box::pin(async move {
            let form = form_encode(&[
                ("client_id", &config.client_id),
                ("client_secret", &config.client_secret),
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", &config.redirect_uri),
                ("code_verifier", verifier),
            ]);
            let failure = DiscordFailure { status: None };
            let request = http::Request::post(TOKEN_URL)
                .header(http::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Full::new(Bytes::from(form)))
                .map_err(|_| failure.clone())?;
            let token = self.call(request).await?;
            let (Some(access), Some(kind)) = (
                token.get("access_token").and_then(Value::as_str),
                token.get("token_type").and_then(Value::as_str),
            ) else {
                return Err(failure);
            };
            let request = http::Request::get(ME_URL)
                .header(http::header::AUTHORIZATION, format!("{kind} {access}"))
                .body(Full::new(Bytes::new()))
                .map_err(|_| failure.clone())?;
            parse_user(&self.call(request).await?).ok_or(failure)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn encodings() {
        assert_eq!(search_params_encode("a b*c-d.e_f"), "a+b*c-d.e_f");
        assert_eq!(search_params_encode("https://x/y?z=1"), "https%3A%2F%2Fx%2Fy%3Fz%3D1");
        assert_eq!(form_encode(&[("a", "b c"), ("d&", "e=f")]), "a=b%20c&d%26=e%3Df");
        assert_eq!(
            code_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn users_and_secrets() {
        let full = parse_user(&json!({"id": "1", "username": "fox", "global_name": "Fox", "avatar": "ab"})).unwrap();
        assert_eq!(full.avatar.as_deref(), Some("ab"));
        let bare = parse_user(&json!({"id": "1", "username": "fox", "avatar": null})).unwrap();
        assert_eq!((bare.global_name, bare.avatar), (None, None));
        assert!(parse_user(&json!({"id": 1, "username": "fox"})).is_none());
        let config = DiscordConfig {
            client_id: "id".into(),
            client_secret: "hunter2".into(),
            redirect_uri: "https://x/cb".into(),
        };
        assert!(!format!("{config:?}").contains("hunter2"));
        let url = authorize_url(&config, "st", "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        assert_eq!(
            url,
            "https://discord.com/api/oauth2/authorize?client_id=id&redirect_uri=https%3A%2F%2Fx%2Fcb&response_type=code\
             &scope=identify&state=st&code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM\
             &code_challenge_method=S256"
        );
    }
}
