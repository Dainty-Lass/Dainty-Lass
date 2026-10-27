//! The two calls `controllers/discordAuth.ts` makes to Discord, behind a trait so a test never reaches the network:
//! the authorization-code exchange (`POST /api/oauth2/token`, with the PKCE verifier) and `GET /api/users/@me`.

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::{Method, Request};
use hyper_rustls::HttpsConnectorBuilder;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use module_api::BoxFuture;
use serde::Deserialize;

/// `DISCORD_CLIENT_ID`, `DISCORD_CLIENT_SECRET`, `DISCORD_REDIRECT_URI`: all three or Discord sign-in is not set up.
#[derive(Clone, PartialEq, Eq)]
pub struct DiscordConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
}

impl std::fmt::Debug for DiscordConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscordConfig")
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

/// `DiscordUser`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DiscordUser {
    pub id: String,
    pub username: String,
    #[serde(default)]
    pub global_name: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
}

/// The exchange or the lookup failed; `status` is Discord's, logged by the controller, never shown.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Discord answered {status:?}")]
pub struct DiscordFailure {
    pub status: Option<u16>,
}

pub trait DiscordApi: Send + Sync {
    /// Trades the authorization code for the user it was granted by.
    fn exchange_code<'a>(
        &'a self,
        config: &'a DiscordConfig,
        code: &'a str,
        code_verifier: &'a str,
    ) -> BoxFuture<'a, Result<DiscordUser, DiscordFailure>>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct HttpsDiscord;

#[derive(Deserialize)]
struct TokenBody {
    access_token: String,
    token_type: String,
}

fn form(pairs: &[(&str, &str)]) -> String {
    use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
    pairs
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                utf8_percent_encode(k, NON_ALPHANUMERIC),
                utf8_percent_encode(v, NON_ALPHANUMERIC)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

async fn call(
    method: Method,
    url: &str,
    headers: &[(&str, String)],
    body: Option<String>,
) -> Result<Bytes, DiscordFailure> {
    let https = HttpsConnectorBuilder::new()
        .with_webpki_roots()
        .https_only()
        .enable_http1()
        .build();
    let client = Client::builder(TokioExecutor::new()).build::<_, Full<Bytes>>(https);
    let mut request = Request::builder().method(method).uri(url);
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    let request = request
        .body(Full::new(Bytes::from(body.unwrap_or_default())))
        .map_err(|_| DiscordFailure { status: None })?;
    let response = tokio::time::timeout(std::time::Duration::from_secs(15), client.request(request))
        .await
        .map_err(|_| DiscordFailure { status: None })?
        .map_err(|_| DiscordFailure { status: None })?;
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .map_err(|_| DiscordFailure { status: None })?
        .to_bytes();
    if status.is_success() {
        Ok(bytes)
    } else {
        Err(DiscordFailure {
            status: Some(status.as_u16()),
        })
    }
}

impl DiscordApi for HttpsDiscord {
    fn exchange_code<'a>(
        &'a self,
        config: &'a DiscordConfig,
        code: &'a str,
        code_verifier: &'a str,
    ) -> BoxFuture<'a, Result<DiscordUser, DiscordFailure>> {
        Box::pin(async move {
            let body = form(&[
                ("client_id", &config.client_id),
                ("client_secret", &config.client_secret),
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", &config.redirect_uri),
                ("code_verifier", code_verifier),
            ]);
            let token = call(
                Method::POST,
                "https://discord.com/api/oauth2/token",
                &[("content-type", "application/x-www-form-urlencoded".to_owned())],
                Some(body),
            )
            .await?;
            let token: TokenBody = serde_json::from_slice(&token).map_err(|_| DiscordFailure { status: None })?;
            let user = call(
                Method::GET,
                "https://discord.com/api/users/@me",
                &[("authorization", format!("{} {}", token.token_type, token.access_token))],
                None,
            )
            .await?;
            serde_json::from_slice(&user).map_err(|_| DiscordFailure { status: None })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_form_is_percent_encoded() {
        assert_eq!(form(&[("a", "b c"), ("d&", "e=f")]), "a=b%20c&d%26=e%3Df");
    }

    #[test]
    fn a_discord_user_reads_with_or_without_the_optional_fields() {
        let user: DiscordUser =
            serde_json::from_str(r#"{"id":"1","username":"fox","global_name":null,"avatar":"abc"}"#).unwrap();
        assert_eq!(
            (user.id.as_str(), user.avatar.as_deref(), user.global_name),
            ("1", Some("abc"), None)
        );
        assert!(serde_json::from_str::<DiscordUser>(r#"{"id":"1","username":"fox"}"#).is_ok());
    }

    #[test]
    fn the_config_stays_out_of_debug() {
        let config = DiscordConfig {
            client_id: "id".into(),
            client_secret: "SECRET".into(),
            redirect_uri: "https://x".into(),
        };
        assert!(!format!("{config:?}").contains("SECRET"));
    }
}
