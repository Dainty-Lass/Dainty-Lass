// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! `[oauth]`: players sign in to the launcher with an OAuth provider (`docs/oauth.md`), written into the backend's
//! env file with the client secret from secrets.env. The secret is never a settings.toml field
//! (`setup --oauth-client-secret` reads it from stdin).
//!
//! ```toml
//! [oauth]
//! preset = "discord"
//! client_id = "123456789012345678"
//! guild_ids = ["123456789012345678"]
//! ```

use serde_json::{Map, Value};

use super::EnvEntries;
use crate::error::HostError;

/// The variables the backend reads (`rustic_isles_auth::oauth`), every one written so none comes from the shell.
pub const VARIABLES: [&str; 13] = [
    "DAINTY_OAUTH_PRESET",
    "DAINTY_OAUTH_CLIENT_ID",
    "DAINTY_OAUTH_CLIENT_SECRET",
    "DAINTY_OAUTH_GUILD_IDS",
    "DAINTY_OAUTH_REQUIRED",
    "DAINTY_OAUTH_PROVIDER_ID",
    "DAINTY_OAUTH_NAME",
    "DAINTY_OAUTH_AUTHORIZE_URL",
    "DAINTY_OAUTH_TOKEN_URL",
    "DAINTY_OAUTH_USERINFO_URL",
    "DAINTY_OAUTH_SCOPES",
    "DAINTY_OAUTH_SUBJECT_FIELD",
    "DAINTY_OAUTH_USERNAME_FIELD",
];

/// The redirect URI the host registers with the provider (the launcher's loopback listener).
pub const REDIRECT_URI: &str = "http://127.0.0.1:61190/oauth/callback";

/// The provider's addresses and fields, for `preset = "custom"`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CustomProvider {
    /// The id accounts are linked under, a lower-case slug other than `discord`.
    pub provider_id: String,
    /// The name on the launcher's button.
    pub name: String,
    /// The authorization endpoint.
    pub authorize_url: String,
    /// The token endpoint.
    pub token_url: String,
    /// The userinfo endpoint.
    pub userinfo_url: String,
    /// The scopes; empty: `openid profile`.
    pub scopes: Vec<String>,
    /// The userinfo field with the user's id; None: `sub`.
    pub subject_field: Option<String>,
    /// The userinfo field with their name; None: `preferred_username`.
    pub username_field: Option<String>,
}

/// Which provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preset {
    /// Discord, with only the client id and optionally the servers a player must be in.
    Discord {
        /// The servers (guild ids); empty admits anyone with a Discord account.
        guild_ids: Vec<String>,
    },
    /// Any OAuth 2.0 provider with PKCE and a JSON userinfo endpoint.
    Custom(CustomProvider),
}

/// `[oauth]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthConfig {
    /// The provider.
    pub preset: Preset,
    /// The application's client id.
    pub client_id: String,
    /// New accounts are made only by signing in; the name-and-invite form is off.
    pub required: bool,
}

const DISCORD_KEYS: [&str; 4] = ["preset", "clientId", "guildIds", "required"];
const CUSTOM_KEYS: [&str; 11] = [
    "preset",
    "clientId",
    "required",
    "providerId",
    "name",
    "authorizeUrl",
    "tokenUrl",
    "userinfoUrl",
    "scopes",
    "subjectField",
    "usernameField",
];

fn bad(text: impl Into<String>) -> HostError {
    HostError::message(text)
}

fn is_slug(text: &str) -> bool {
    (1..=32).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn is_https(text: &str) -> bool {
    text.len() <= 2048
        && text
            .strip_prefix("https://")
            .and_then(|rest| rest.split(['/', '?', '#']).next())
            .is_some_and(|authority| !authority.is_empty() && !authority.contains('@'))
        && !text.chars().any(|c| c.is_whitespace() || c.is_control())
}

fn snowflake(text: &str) -> bool {
    (1..=20).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit())
}

fn strings(obj: &Map<String, Value>, key: &str, toml: &str) -> Result<Vec<String>, HostError> {
    match obj.get(key).filter(|v| !v.is_null()) {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => {
            let mut out: Vec<String> = Vec::new();
            for item in items {
                let text = item
                    .as_str()
                    .ok_or_else(|| bad(format!("oauth.{toml} must list strings, such as [\"1234\"]")))?
                    .trim()
                    .to_owned();
                if !text.is_empty() && !out.contains(&text) {
                    out.push(text);
                }
            }
            Ok(out)
        }
        Some(_) => Err(bad(format!("oauth.{toml} must be a list"))),
    }
}

impl OAuthConfig {
    /// Reads `[oauth]`; fails on a key it doesn't know or a value it doesn't take.
    pub fn from_json(value: &Value) -> Result<Self, HostError> {
        let obj = value.as_object().ok_or_else(|| bad("oauth must be a table"))?;
        let text = |key: &str, toml: &str| -> Result<Option<String>, HostError> {
            match obj.get(key).filter(|v| !v.is_null()) {
                None => Ok(None),
                Some(v) => v
                    .as_str()
                    .map(|s| Some(s.trim().to_owned()).filter(|s| !s.is_empty()))
                    .ok_or_else(|| bad(format!("oauth.{toml} must be a string"))),
            }
        };
        let preset = text("preset", "preset")?
            .ok_or_else(|| bad("oauth.preset must be \"discord\" or \"custom\""))?
            .to_ascii_lowercase();
        let known: &[&str] = match preset.as_str() {
            "discord" => &DISCORD_KEYS,
            "custom" => &CUSTOM_KEYS,
            _ => return Err(bad("oauth.preset must be \"discord\" or \"custom\"")),
        };
        for key in obj.keys() {
            if !known.contains(&key.as_str()) {
                let toml = crate::settingstoml::snake_case(key);
                return Err(bad(format!("oauth.{toml} is not a setting for preset = \"{preset}\"")));
            }
        }
        let client_id = text("clientId", "client_id")?.ok_or_else(|| {
            bad("oauth.client_id must be set: the application's client id (OAuth2 > Client ID on Discord)")
        })?;
        if client_id.len() > 255 || client_id.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(bad("oauth.client_id must be the client id, without spaces"));
        }
        let required = match obj.get("required").filter(|v| !v.is_null()) {
            None => false,
            Some(v) => v.as_bool().ok_or_else(|| bad("oauth.required must be true or false"))?,
        };
        let preset = if preset == "discord" {
            let guild_ids = strings(obj, "guildIds", "guild_ids")?;
            if !guild_ids.iter().all(|g| snowflake(g)) {
                return Err(bad("oauth.guild_ids must list Discord server ids, digits only"));
            }
            Preset::Discord { guild_ids }
        } else {
            let need = |key: &str, toml: &str| -> Result<String, HostError> {
                text(key, toml)?.ok_or_else(|| bad(format!("oauth.{toml} must be set for preset = \"custom\"")))
            };
            let provider_id = need("providerId", "provider_id")?;
            if !is_slug(&provider_id) || provider_id == "discord" {
                return Err(bad(
                    "oauth.provider_id must be lower-case letters, digits and -, and not discord (use preset = \"discord\")",
                ));
            }
            let name = need("name", "name")?;
            if name.chars().count() > 32 || name.chars().any(char::is_control) {
                return Err(bad("oauth.name must be at most 32 characters"));
            }
            let url = |key: &str, toml: &str| -> Result<String, HostError> {
                let value = need(key, toml)?;
                if !is_https(&value) {
                    return Err(bad(format!("oauth.{toml} must be an https:// address")));
                }
                Ok(value)
            };
            let field = |key: &str, toml: &str| -> Result<Option<String>, HostError> {
                let value = text(key, toml)?;
                if value
                    .as_deref()
                    .is_some_and(|f| f.len() > 64 || !f.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
                {
                    return Err(bad(format!("oauth.{toml} must be a field name: letters, digits and _")));
                }
                Ok(value)
            };
            let scopes = strings(obj, "scopes", "scopes")?;
            if scopes.iter().any(|s| s.contains(char::is_whitespace)) {
                return Err(bad("oauth.scopes must list one scope per string"));
            }
            Preset::Custom(CustomProvider {
                provider_id,
                name,
                authorize_url: url("authorizeUrl", "authorize_url")?,
                token_url: url("tokenUrl", "token_url")?,
                userinfo_url: url("userinfoUrl", "userinfo_url")?,
                scopes,
                subject_field: field("subjectField", "subject_field")?,
                username_field: field("usernameField", "username_field")?,
            })
        };
        Ok(Self {
            preset,
            client_id,
            required,
        })
    }

    /// The table settings.toml writes.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        let list = |items: &[String]| Value::Array(items.iter().cloned().map(Value::from).collect());
        match &self.preset {
            Preset::Discord { guild_ids } => {
                out.insert("preset".into(), Value::from("discord"));
                out.insert("clientId".into(), Value::from(self.client_id.clone()));
                if !guild_ids.is_empty() {
                    out.insert("guildIds".into(), list(guild_ids));
                }
            }
            Preset::Custom(c) => {
                out.insert("preset".into(), Value::from("custom"));
                out.insert("clientId".into(), Value::from(self.client_id.clone()));
                out.insert("providerId".into(), Value::from(c.provider_id.clone()));
                out.insert("name".into(), Value::from(c.name.clone()));
                out.insert("authorizeUrl".into(), Value::from(c.authorize_url.clone()));
                out.insert("tokenUrl".into(), Value::from(c.token_url.clone()));
                out.insert("userinfoUrl".into(), Value::from(c.userinfo_url.clone()));
                if !c.scopes.is_empty() {
                    out.insert("scopes".into(), list(&c.scopes));
                }
                if let Some(f) = &c.subject_field {
                    out.insert("subjectField".into(), Value::from(f.clone()));
                }
                if let Some(f) = &c.username_field {
                    out.insert("usernameField".into(), Value::from(f.clone()));
                }
            }
        }
        out.insert("required".into(), Value::Bool(self.required));
        Value::Object(out)
    }

    /// The backend's `DAINTY_OAUTH_*`, every one of [`VARIABLES`].
    #[must_use]
    pub fn env(&self, secret: &str) -> EnvEntries {
        let mut values: Vec<(&str, String)> = vec![
            ("DAINTY_OAUTH_CLIENT_ID", self.client_id.clone()),
            ("DAINTY_OAUTH_CLIENT_SECRET", secret.to_owned()),
            ("DAINTY_OAUTH_REQUIRED", self.required.to_string()),
        ];
        match &self.preset {
            Preset::Discord { guild_ids } => {
                values.push(("DAINTY_OAUTH_PRESET", "discord".to_owned()));
                values.push(("DAINTY_OAUTH_GUILD_IDS", guild_ids.join(",")));
            }
            Preset::Custom(c) => {
                values.extend([
                    ("DAINTY_OAUTH_PRESET", "custom".to_owned()),
                    ("DAINTY_OAUTH_PROVIDER_ID", c.provider_id.clone()),
                    ("DAINTY_OAUTH_NAME", c.name.clone()),
                    ("DAINTY_OAUTH_AUTHORIZE_URL", c.authorize_url.clone()),
                    ("DAINTY_OAUTH_TOKEN_URL", c.token_url.clone()),
                    ("DAINTY_OAUTH_USERINFO_URL", c.userinfo_url.clone()),
                    ("DAINTY_OAUTH_SCOPES", c.scopes.join(" ")),
                    (
                        "DAINTY_OAUTH_SUBJECT_FIELD",
                        c.subject_field.clone().unwrap_or_default(),
                    ),
                    (
                        "DAINTY_OAUTH_USERNAME_FIELD",
                        c.username_field.clone().unwrap_or_default(),
                    ),
                ]);
            }
        }
        VARIABLES
            .iter()
            .map(|name| {
                let value = values.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
                ((*name).to_owned(), value.unwrap_or_default())
            })
            .collect()
    }
}

/// Every [`VARIABLES`] empty: sign-in off, and nothing left for the shell to fill in.
#[must_use]
pub fn off() -> EnvEntries {
    VARIABLES
        .iter()
        .map(|name| ((*name).to_owned(), String::new()))
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;
    use serde_json::json;

    #[test]
    fn the_discord_preset_round_trips_and_renders_what_the_backend_reads() {
        let value = json!({"preset": " Discord ", "clientId": "77", "guildIds": ["1", " 2 ", "1"], "required": true});
        let oauth = OAuthConfig::from_json(&value).unwrap();
        assert_eq!(
            oauth.preset,
            Preset::Discord {
                guild_ids: vec!["1".into(), "2".into()]
            }
        );
        assert_eq!(OAuthConfig::from_json(&oauth.to_json()).unwrap(), oauth);
        let env = oauth.env("shh");
        assert_eq!(env.len(), VARIABLES.len());
        let get = |k: &str| env.iter().find(|(n, _)| n == k).unwrap().1.clone();
        assert_eq!(get("DAINTY_OAUTH_PRESET"), "discord");
        assert_eq!(get("DAINTY_OAUTH_GUILD_IDS"), "1,2");
        assert_eq!(get("DAINTY_OAUTH_REQUIRED"), "true");
        assert_eq!(get("DAINTY_OAUTH_TOKEN_URL"), "");
        let settings: std::collections::HashMap<String, String> = env.into_iter().collect();
        let provider = rustic_isles_auth::oauth::OAuthProvider::from_settings(&settings, REDIRECT_URI)
            .unwrap()
            .unwrap();
        assert_eq!(
            (provider.id.as_str(), provider.guild_ids.len(), provider.required),
            ("discord", 2, true)
        );
        assert!(off().iter().all(|(_, v)| v.is_empty()));
        assert_eq!(REDIRECT_URI, dainty_protocol::account::OAUTH_REDIRECT_URI);
    }

    #[test]
    fn a_custom_provider_round_trips() {
        let value = json!({
            "preset": "custom", "clientId": "c", "providerId": "github", "name": "GitHub",
            "authorizeUrl": "https://github.com/login/oauth/authorize",
            "tokenUrl": "https://github.com/login/oauth/access_token",
            "userinfoUrl": "https://api.github.com/user", "scopes": ["read:user"],
            "subjectField": "id", "usernameField": "login"
        });
        let oauth = OAuthConfig::from_json(&value).unwrap();
        assert_eq!(OAuthConfig::from_json(&oauth.to_json()).unwrap(), oauth);
        let settings: std::collections::HashMap<String, String> = oauth.env("s").into_iter().collect();
        let provider = rustic_isles_auth::oauth::OAuthProvider::from_settings(&settings, REDIRECT_URI)
            .unwrap()
            .unwrap();
        assert_eq!(
            (provider.id.as_str(), provider.username_field.as_str()),
            ("github", "login")
        );
    }

    #[test]
    fn refuses_what_it_does_not_know() {
        let custom = |k: &str, v: Value| {
            let mut base = json!({
                "preset": "custom", "clientId": "c", "providerId": "sso", "name": "SSO",
                "authorizeUrl": "https://sso.example/a", "tokenUrl": "https://sso.example/t",
                "userinfoUrl": "https://sso.example/u"
            });
            base[k] = v;
            base
        };
        for value in [
            json!({"clientId": "1"}),
            json!({"preset": "google", "clientId": "1"}),
            json!({"preset": "discord"}),
            json!({"preset": "discord", "clientId": "1", "clientSecret": "x"}),
            json!({"preset": "discord", "clientId": "1", "guildIds": ["my-server"]}),
            json!({"preset": "discord", "clientId": "1", "guildIds": "1"}),
            json!({"preset": "discord", "clientId": "1", "required": "yes"}),
            json!({"preset": "discord", "clientId": "1", "tokenUrl": "https://x"}),
            custom("providerId", json!("discord")),
            custom("providerId", json!("My SSO")),
            custom("tokenUrl", json!("http://sso.example/t")),
            custom("guildIds", json!(["1"])),
            custom("subjectField", json!("user.id")),
            json!(true),
        ] {
            assert!(OAuthConfig::from_json(&value).is_err(), "{value}");
        }
    }
}
