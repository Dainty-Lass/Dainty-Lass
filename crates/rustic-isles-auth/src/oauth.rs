// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! OAuth 2.0 sign-in for the Dainty launcher (`DAINTY_OAUTH_*`): one provider per server, the Discord preset or
//! the host's own, an authorization code with PKCE (RFC 7636) to a loopback redirect the launcher holds (RFC 8252
//! section 7.3), and the code exchange behind a trait. The server keeps the client secret; the launcher never sees it.

use std::time::Duration;

use bytes::Bytes;
use dainty_module_api::BoxFuture;
use http_body_util::Full;
use serde_json::Value;

use crate::AuthSettings;
use crate::discord::{code_challenge, form_encode, search_params_encode};

const DISCORD_AUTHORIZE: &str = "https://discord.com/oauth2/authorize";
const DISCORD_TOKEN: &str = "https://discord.com/api/oauth2/token";
const DISCORD_ME: &str = "https://discord.com/api/users/@me";
const DISCORD_GUILDS: &str = "https://discord.com/api/users/@me/guilds";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_ANSWER: usize = 256 * 1024;
const MAX_URL: usize = 2048;
/// Longest provider name the launcher's button shows.
pub const NAME_MAX: usize = 32;

/// Why the settings name no provider.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    /// `DAINTY_OAUTH_PRESET` is neither `discord` nor `custom`.
    #[error("DAINTY_OAUTH_PRESET must be discord or custom")]
    Preset,
    /// A setting the provider needs is empty.
    #[error("{0} must be set")]
    Missing(&'static str),
    /// A URL is not `https://` without credentials.
    #[error("{0} must be an https:// address")]
    Url(&'static str),
    /// The custom provider's id is not a lower-case slug, or is `discord`.
    #[error("DAINTY_OAUTH_PROVIDER_ID must be lower-case letters, digits and -, and not discord")]
    Id,
    /// A guild id is not digits.
    #[error("DAINTY_OAUTH_GUILD_IDS must list Discord server ids, digits only")]
    Guild,
}

/// The host's provider; Debug leaves out the secret.
#[derive(Clone, PartialEq, Eq)]
pub struct OAuthProvider {
    /// `discord`, or the host's slug: `auth_identities.provider`.
    pub id: String,
    /// The name the launcher's button shows.
    pub name: String,
    /// The client id.
    pub client_id: String,
    /// The client secret; never logged.
    pub client_secret: String,
    /// The authorization endpoint.
    pub authorize_url: String,
    /// The token endpoint.
    pub token_url: String,
    /// The endpoint that answers the signed-in user as JSON.
    pub userinfo_url: String,
    /// The scopes asked for.
    pub scopes: Vec<String>,
    /// The user's field holding their stable id.
    pub subject_field: String,
    /// The user's field holding their name.
    pub username_field: String,
    /// Discord only: the user must be in one of these servers; empty admits anyone.
    pub guild_ids: Vec<String>,
    /// New accounts are made only this way (`Register` refuses).
    pub required: bool,
    /// The launcher's loopback redirect.
    pub redirect_uri: String,
}

impl std::fmt::Debug for OAuthProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthProvider")
            .field("id", &self.id)
            .field("client_id", &self.client_id)
            .field("authorize_url", &self.authorize_url)
            .finish_non_exhaustive()
    }
}

fn https(text: &str, name: &'static str) -> Result<String, ProviderError> {
    let ok = text.len() <= MAX_URL
        && text
            .strip_prefix("https://")
            .and_then(|rest| rest.split(['/', '?', '#']).next())
            .is_some_and(|authority| !authority.is_empty() && !authority.contains('@'))
        && !text.chars().any(|c| c.is_whitespace() || c.is_control());
    ok.then(|| text.to_owned()).ok_or(ProviderError::Url(name))
}

/// Whether `text` is a provider id: lower-case ASCII letters, digits and `-`, 1 to 32 bytes.
pub fn is_provider_id(text: &str) -> bool {
    (1..=32).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn truthy(text: Option<&str>) -> bool {
    matches!(
        text.map(|t| t.trim().to_ascii_lowercase()).as_deref(),
        Some("1" | "true" | "on" | "yes")
    )
}

impl OAuthProvider {
    /// The provider the settings name; `Ok(None)` when `DAINTY_OAUTH_PRESET` is empty.
    pub fn from_settings(settings: &dyn AuthSettings, redirect_uri: &str) -> Result<Option<Self>, ProviderError> {
        let get = |name: &str| settings.raw(name).map(str::trim).filter(|v| !v.is_empty());
        let need = |name: &'static str| get(name).map(str::to_owned).ok_or(ProviderError::Missing(name));
        let preset = match get("DAINTY_OAUTH_PRESET") {
            None => return Ok(None),
            Some(p) => p.to_ascii_lowercase(),
        };
        if !["discord", "custom"].contains(&preset.as_str()) {
            return Err(ProviderError::Preset);
        }
        let client_id = need("DAINTY_OAUTH_CLIENT_ID")?;
        let client_secret = need("DAINTY_OAUTH_CLIENT_SECRET")?;
        let required = truthy(get("DAINTY_OAUTH_REQUIRED"));
        let redirect_uri = redirect_uri.to_owned();
        match preset.as_str() {
            "discord" => {
                let guild_ids: Vec<String> = get("DAINTY_OAUTH_GUILD_IDS")
                    .unwrap_or("")
                    .split(',')
                    .map(str::trim)
                    .filter(|g| !g.is_empty())
                    .map(str::to_owned)
                    .collect();
                if guild_ids
                    .iter()
                    .any(|g| g.len() > 20 || !g.bytes().all(|b| b.is_ascii_digit()))
                {
                    return Err(ProviderError::Guild);
                }
                // `guilds` lists the user's servers in one call; asked only when membership is checked
                let mut scopes = vec!["identify".to_owned()];
                if !guild_ids.is_empty() {
                    scopes.push("guilds".to_owned());
                }
                Ok(Some(Self {
                    id: "discord".to_owned(),
                    name: "Discord".to_owned(),
                    client_id,
                    client_secret,
                    authorize_url: DISCORD_AUTHORIZE.to_owned(),
                    token_url: DISCORD_TOKEN.to_owned(),
                    userinfo_url: DISCORD_ME.to_owned(),
                    scopes,
                    subject_field: "id".to_owned(),
                    username_field: "username".to_owned(),
                    guild_ids,
                    required,
                    redirect_uri,
                }))
            }
            "custom" => {
                let id = need("DAINTY_OAUTH_PROVIDER_ID")?;
                if !is_provider_id(&id) || id == "discord" {
                    return Err(ProviderError::Id);
                }
                let name: String = need("DAINTY_OAUTH_NAME")?
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(NAME_MAX)
                    .collect();
                let scopes = get("DAINTY_OAUTH_SCOPES")
                    .unwrap_or("openid profile")
                    .split([' ', ','])
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect();
                let field = |name: &str, default: &str| get(name).unwrap_or(default).to_owned();
                Ok(Some(Self {
                    id,
                    name,
                    client_id,
                    client_secret,
                    authorize_url: https(&need("DAINTY_OAUTH_AUTHORIZE_URL")?, "DAINTY_OAUTH_AUTHORIZE_URL")?,
                    token_url: https(&need("DAINTY_OAUTH_TOKEN_URL")?, "DAINTY_OAUTH_TOKEN_URL")?,
                    userinfo_url: https(&need("DAINTY_OAUTH_USERINFO_URL")?, "DAINTY_OAUTH_USERINFO_URL")?,
                    scopes,
                    subject_field: field("DAINTY_OAUTH_SUBJECT_FIELD", "sub"),
                    username_field: field("DAINTY_OAUTH_USERNAME_FIELD", "preferred_username"),
                    guild_ids: Vec::new(),
                    required,
                    redirect_uri,
                }))
            }
            _ => Err(ProviderError::Preset),
        }
    }

    /// The authorize URL for `state`, with the S256 challenge of `verifier`.
    pub fn authorize_url(&self, state: &str, verifier: &str) -> String {
        let challenge = code_challenge(verifier);
        let scope = self.scopes.join(" ");
        let pairs = [
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", self.redirect_uri.as_str()),
            ("response_type", "code"),
            ("scope", scope.as_str()),
            ("state", state),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
        ];
        let query: Vec<String> = pairs
            .iter()
            .map(|(k, v)| format!("{}={}", search_params_encode(k), search_params_encode(v)))
            .collect();
        let joiner = if self.authorize_url.contains('?') { '&' } else { '?' };
        format!("{}{joiner}{}", self.authorize_url, query.join("&"))
    }

    /// Whether the user may sign in: in one of the guilds, when the host listed any.
    pub fn admits(&self, user: &OAuthUser) -> bool {
        self.guild_ids.is_empty()
            || user
                .guilds
                .as_ref()
                .is_some_and(|in_| in_.iter().any(|g| self.guild_ids.contains(g)))
    }

    /// The avatar URL Discord's user carries; None for other providers.
    pub fn avatar_url(&self, user: &OAuthUser) -> Option<String> {
        (self.id == "discord")
            .then_some(user.avatar.as_ref())
            .flatten()
            .map(|a| format!("https://cdn.discordapp.com/avatars/{}/{a}.png", user.subject))
    }
}

/// The user a code exchange signed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthUser {
    /// The provider's stable id for them.
    pub subject: String,
    /// Their name at the provider, empty when it sent none.
    pub username: String,
    /// Discord's avatar hash.
    pub avatar: Option<String>,
    /// Discord: the ids of the servers they are in, when asked.
    pub guilds: Option<Vec<String>>,
}

/// A failed call: the provider's status when it answered non-2xx (logged, never shown).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the OAuth provider call failed")]
pub struct OAuthFailure {
    /// The HTTP status; None when it never answered or the answer did not parse.
    pub status: Option<u16>,
}

/// The provider calls, a trait so tests answer without the network.
pub trait OAuthApi: Send + Sync {
    /// Trades an authorization code and its PKCE verifier for the signed-in user.
    fn sign_in<'a>(
        &'a self,
        provider: &'a OAuthProvider,
        code: &'a str,
        verifier: &'a str,
    ) -> BoxFuture<'a, Result<OAuthUser, OAuthFailure>>;
}

/// A text or whole-number field (GitHub's `id` is a number).
fn text_field(value: &Value, name: &str) -> Option<String> {
    match value.get(name)? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) if n.is_u64() || n.is_i64() => Some(n.to_string()),
        _ => None,
    }
}

/// The user from a userinfo answer.
pub fn parse_user(provider: &OAuthProvider, value: &Value) -> Option<OAuthUser> {
    let subject = text_field(value, &provider.subject_field).filter(|s| s.len() <= 255)?;
    Some(OAuthUser {
        subject,
        username: text_field(value, &provider.username_field).unwrap_or_default(),
        avatar: value.get("avatar").and_then(Value::as_str).map(str::to_owned),
        guilds: None,
    })
}

/// The guild ids from Discord's `/users/@me/guilds`.
pub fn parse_guilds(value: &Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|g| g.get("id").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

/// The calls over HTTPS only (webpki roots, HTTP/1.1, 15 s each).
pub struct HttpsOAuth {
    client: dainty_http::Client,
}

impl std::fmt::Debug for HttpsOAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HttpsOAuth")
    }
}

impl Default for HttpsOAuth {
    fn default() -> Self {
        Self {
            client: dainty_http::client(dainty_http::Schemes::HttpsOnly),
        }
    }
}

impl HttpsOAuth {
    async fn call(&self, request: http::Request<Full<Bytes>>) -> Result<Value, OAuthFailure> {
        let answer = dainty_http::call(&self.client, request, MAX_ANSWER, REQUEST_TIMEOUT)
            .await
            .map_err(|_| OAuthFailure { status: None })?;
        if !answer.status.is_success() {
            return Err(OAuthFailure {
                status: Some(answer.status.as_u16()),
            });
        }
        serde_json::from_slice::<Value>(&answer.body).map_err(|_| OAuthFailure { status: None })
    }

    async fn get(&self, url: &str, bearer: &str) -> Result<Value, OAuthFailure> {
        let request = http::Request::get(url)
            .header(http::header::AUTHORIZATION, format!("Bearer {bearer}"))
            .header(http::header::ACCEPT, "application/json")
            // GitHub refuses a call without one
            .header(http::header::USER_AGENT, "DaintyLass")
            .body(Full::new(Bytes::new()))
            .map_err(|_| OAuthFailure { status: None })?;
        self.call(request).await
    }
}

impl OAuthApi for HttpsOAuth {
    fn sign_in<'a>(
        &'a self,
        provider: &'a OAuthProvider,
        code: &'a str,
        verifier: &'a str,
    ) -> BoxFuture<'a, Result<OAuthUser, OAuthFailure>> {
        Box::pin(async move {
            let failure = OAuthFailure { status: None };
            let form = form_encode(&[
                ("client_id", &provider.client_id),
                ("client_secret", &provider.client_secret),
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", &provider.redirect_uri),
                ("code_verifier", verifier),
            ]);
            let request = http::Request::post(&provider.token_url)
                .header(http::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                // GitHub answers a form unless asked for JSON
                .header(http::header::ACCEPT, "application/json")
                .header(http::header::USER_AGENT, "DaintyLass")
                .body(Full::new(Bytes::from(form)))
                .map_err(|_| failure.clone())?;
            let token = self.call(request).await?;
            let access = token
                .get("access_token")
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty() && t.bytes().all(|b| b.is_ascii_graphic()))
                .ok_or_else(|| failure.clone())?
                .to_owned();
            let mut user =
                parse_user(provider, &self.get(&provider.userinfo_url, &access).await?).ok_or(failure.clone())?;
            if !provider.guild_ids.is_empty() {
                user.guilds = Some(parse_guilds(&self.get(DISCORD_GUILDS, &access).await?).ok_or(failure)?);
            }
            Ok(user)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::*;

    const REDIRECT: &str = "http://127.0.0.1:61190/oauth/callback";

    fn settings(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    #[test]
    fn the_discord_preset_needs_only_the_client() {
        assert_eq!(OAuthProvider::from_settings(&settings(&[]), REDIRECT), Ok(None));
        let base = [
            ("DAINTY_OAUTH_PRESET", " Discord "),
            ("DAINTY_OAUTH_CLIENT_ID", "123"),
            ("DAINTY_OAUTH_CLIENT_SECRET", "hunter2"),
        ];
        let p = OAuthProvider::from_settings(&settings(&base), REDIRECT)
            .unwrap()
            .unwrap();
        assert_eq!(
            (p.id.as_str(), p.name.as_str(), p.scopes.clone()),
            ("discord", "Discord", vec!["identify".to_owned()])
        );
        assert!(!p.required && p.guild_ids.is_empty());
        assert!(!format!("{p:?}").contains("hunter2"));
        assert_eq!(
            p.authorize_url("st", "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "https://discord.com/oauth2/authorize?client_id=123&redirect_uri=http%3A%2F%2F127.0.0.1%3A61190%2Foauth%2Fcallback\
             &response_type=code&scope=identify&state=st&code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM\
             &code_challenge_method=S256"
        );
        let mut guilded = base.to_vec();
        guilded.extend([
            ("DAINTY_OAUTH_GUILD_IDS", "111, 222"),
            ("DAINTY_OAUTH_REQUIRED", "true"),
        ]);
        let g = OAuthProvider::from_settings(&settings(&guilded), REDIRECT)
            .unwrap()
            .unwrap();
        assert_eq!(g.guild_ids, ["111", "222"]);
        assert!(g.required && g.scopes.contains(&"guilds".to_owned()));
        let user = |guilds: Option<Vec<&str>>| OAuthUser {
            subject: "9".into(),
            username: "fox".into(),
            avatar: Some("ab".into()),
            guilds: guilds.map(|g| g.into_iter().map(str::to_owned).collect()),
        };
        assert!(g.admits(&user(Some(vec!["5", "222"]))));
        assert!(!g.admits(&user(Some(vec!["5"]))) && !g.admits(&user(None)));
        assert!(p.admits(&user(None)));
        assert_eq!(
            p.avatar_url(&user(None)).as_deref(),
            Some("https://cdn.discordapp.com/avatars/9/ab.png")
        );
        let mut bad = base.to_vec();
        bad.push(("DAINTY_OAUTH_GUILD_IDS", "my-server"));
        assert_eq!(
            OAuthProvider::from_settings(&settings(&bad), REDIRECT),
            Err(ProviderError::Guild)
        );
        assert_eq!(
            OAuthProvider::from_settings(&settings(&base[..2]), REDIRECT),
            Err(ProviderError::Missing("DAINTY_OAUTH_CLIENT_SECRET"))
        );
        assert_eq!(
            OAuthProvider::from_settings(&settings(&[("DAINTY_OAUTH_PRESET", "google")]), REDIRECT),
            Err(ProviderError::Preset)
        );
    }

    #[test]
    fn a_custom_provider() {
        let base = [
            ("DAINTY_OAUTH_PRESET", "custom"),
            ("DAINTY_OAUTH_CLIENT_ID", "c"),
            ("DAINTY_OAUTH_CLIENT_SECRET", "s"),
            ("DAINTY_OAUTH_PROVIDER_ID", "github"),
            ("DAINTY_OAUTH_NAME", "GitHub"),
            (
                "DAINTY_OAUTH_AUTHORIZE_URL",
                "https://github.com/login/oauth/authorize?allow_signup=false",
            ),
            ("DAINTY_OAUTH_TOKEN_URL", "https://github.com/login/oauth/access_token"),
            ("DAINTY_OAUTH_USERINFO_URL", "https://api.github.com/user"),
            ("DAINTY_OAUTH_SCOPES", "read:user"),
            ("DAINTY_OAUTH_SUBJECT_FIELD", "id"),
            ("DAINTY_OAUTH_USERNAME_FIELD", "login"),
        ];
        let p = OAuthProvider::from_settings(&settings(&base), REDIRECT)
            .unwrap()
            .unwrap();
        assert!(
            p.authorize_url("s", "v")
                .starts_with("https://github.com/login/oauth/authorize?allow_signup=false&client_id=c&")
        );
        let user = parse_user(&p, &json!({"id": 583231, "login": "octocat"})).unwrap();
        assert_eq!((user.subject.as_str(), user.username.as_str()), ("583231", "octocat"));
        assert!(parse_user(&p, &json!({"login": "octocat"})).is_none());
        assert_eq!(p.avatar_url(&user), None);
        for (name, value, error) in [
            ("DAINTY_OAUTH_PROVIDER_ID", "discord", ProviderError::Id),
            ("DAINTY_OAUTH_PROVIDER_ID", "Git Hub", ProviderError::Id),
            (
                "DAINTY_OAUTH_TOKEN_URL",
                "http://github.com/x",
                ProviderError::Url("DAINTY_OAUTH_TOKEN_URL"),
            ),
            (
                "DAINTY_OAUTH_TOKEN_URL",
                "https://u:p@github.com/x",
                ProviderError::Url("DAINTY_OAUTH_TOKEN_URL"),
            ),
        ] {
            let mut changed = settings(&base);
            changed.insert(name.to_owned(), value.to_owned());
            assert_eq!(
                OAuthProvider::from_settings(&changed, REDIRECT),
                Err(error),
                "{name}={value}"
            );
        }
        let defaults: Vec<_> = base
            .iter()
            .filter(|(k, _)| !k.ends_with("_FIELD") && !k.ends_with("SCOPES"))
            .copied()
            .collect();
        let d = OAuthProvider::from_settings(&settings(&defaults), REDIRECT)
            .unwrap()
            .unwrap();
        assert_eq!(
            (d.subject_field.as_str(), d.username_field.as_str()),
            ("sub", "preferred_username")
        );
        assert_eq!(d.scopes, ["openid", "profile"]);
    }

    #[test]
    fn guilds() {
        assert_eq!(
            parse_guilds(&json!([{"id": "1"}, {"id": "2", "name": "x"}])),
            Some(vec!["1".into(), "2".into()])
        );
        assert_eq!(parse_guilds(&json!([{"id": 1}])), None);
        assert_eq!(parse_guilds(&json!({})), None);
    }
}
