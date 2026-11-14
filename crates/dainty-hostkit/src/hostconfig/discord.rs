// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `[discord]` (module sharing A7): the Discord bot's settings, written into `discord.env` with the token from
//! secrets.env. The token is never a settings.toml field (`setup --discord-bot-token` reads it from stdin).
//!
//! ```toml
//! [discord]
//! enabled = true
//! guild_ids = ["123456789012345678"]
//! dashboard_url = "https://dash.example.org"
//! ```

use dainty_discord::config as bot;
use serde_json::{Map, Value};

use super::EnvEntries;
use crate::error::HostError;

/// `[discord]`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiscordConfig {
    /// Whether the server runs the bot.
    pub enabled: bool,
    /// The bot's application id; checked against the token's when set.
    pub application_id: Option<String>,
    /// The guilds it answers in; empty: every guild it is in.
    pub guild_ids: Vec<String>,
    /// dainty-dash, for whether signups are on; left out: settings.toml's `dashboard_url`.
    pub dashboard_url: Option<String>,
}

fn bad(text: impl Into<String>) -> HostError {
    HostError::message(text)
}

impl DiscordConfig {
    /// Reads `[discord]`; fails on a key it doesn't know or a value it doesn't take.
    pub fn from_json(value: &Value) -> Result<Self, HostError> {
        let obj = value.as_object().ok_or_else(|| bad("discord must be a table"))?;
        for key in obj.keys() {
            if !["enabled", "applicationId", "guildIds", "dashboardUrl"].contains(&key.as_str()) {
                return Err(bad(format!("discord.{key} is not a setting")));
            }
        }
        let enabled = match obj.get("enabled").filter(|v| !v.is_null()) {
            None => false,
            Some(v) => v
                .as_bool()
                .ok_or_else(|| bad("discord.enabled must be true or false"))?,
        };
        let text = |key: &str, toml: &str| -> Result<Option<String>, HostError> {
            match obj.get(key).filter(|v| !v.is_null()) {
                None => Ok(None),
                Some(v) => v
                    .as_str()
                    .map(|s| Some(s.trim().to_owned()))
                    .ok_or_else(|| bad(format!("discord.{toml} must be a string"))),
            }
        };
        let application_id = text("applicationId", "application_id")?;
        if let Some(id) = &application_id
            && dainty_discord::interaction::Snowflake::parse(id).is_none()
        {
            return Err(bad("discord.application_id must be the application's id, digits only"));
        }
        let guild_ids = match obj.get("guildIds").filter(|v| !v.is_null()) {
            None => Vec::new(),
            Some(Value::Array(items)) => {
                let texts = items
                    .iter()
                    .map(|v| v.as_str().map(str::trim))
                    .collect::<Option<Vec<_>>>()
                    .ok_or_else(|| bad("discord.guild_ids must list ids as strings, such as [\"1234\"]"))?;
                bot::parse_guild_ids(&texts.join(","))
                    .map_err(|_| bad("discord.guild_ids must list guild ids, digits only"))?
                    .iter()
                    .map(ToString::to_string)
                    .collect()
            }
            Some(_) => return Err(bad("discord.guild_ids must be a list")),
        };
        let dashboard_url = text("dashboardUrl", "dashboard_url")?;
        if let Some(url) = &dashboard_url {
            bot::parse_dashboard_url(url)
                .map_err(|_| bad("discord.dashboard_url must be an http:// or https:// address"))?;
        }
        Ok(Self {
            enabled,
            application_id,
            guild_ids,
            dashboard_url,
        })
    }

    /// The table settings.toml writes.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("enabled".into(), Value::Bool(self.enabled));
        if let Some(id) = &self.application_id {
            out.insert("applicationId".into(), Value::from(id.clone()));
        }
        if !self.guild_ids.is_empty() {
            out.insert(
                "guildIds".into(),
                Value::Array(self.guild_ids.iter().cloned().map(Value::from).collect()),
            );
        }
        if let Some(url) = &self.dashboard_url {
            out.insert("dashboardUrl".into(), Value::from(url.clone()));
        }
        Value::Object(out)
    }

    /// `discord.env`: every variable, empty ones too, so none comes from the host's shell instead.
    #[must_use]
    pub fn env(&self, token: &str, dashboard_fallback: Option<&str>) -> EnvEntries {
        let dashboard = self.dashboard_url.as_deref().or(dashboard_fallback).unwrap_or_default();
        vec![
            (bot::TOKEN.to_owned(), token.to_owned()),
            (
                bot::APPLICATION_ID.to_owned(),
                self.application_id.clone().unwrap_or_default(),
            ),
            (bot::GUILD_IDS.to_owned(), self.guild_ids.join(",")),
            (bot::DASHBOARD_URL.to_owned(), dashboard.to_owned()),
        ]
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;
    use serde_json::json;

    #[test]
    fn reads_writes_back_and_renders_what_the_bot_reads() {
        let value = json!({
            "enabled": true, "applicationId": "77", "guildIds": ["1", " 2 ", "1"],
            "dashboardUrl": "https://dash.example"
        });
        let discord = DiscordConfig::from_json(&value).unwrap();
        assert_eq!(discord.guild_ids, ["1", "2"]);
        assert_eq!(DiscordConfig::from_json(&discord.to_json()).unwrap(), discord);
        let env = discord.env("tok", Some("https://ignored.example"));
        let pairs = env.iter().map(|(k, v)| (k.as_str(), v.as_str()));
        let config = dainty_discord::Config::from_env(&dainty_service_kit::env::Env::from_pairs(pairs)).unwrap();
        assert_eq!(config.token.expose(), "tok");
        assert_eq!(config.guild_ids.len(), 2);
        assert_eq!(config.dashboard_url.unwrap().as_str(), "https://dash.example/");

        let bare = DiscordConfig::from_json(&json!({"enabled": true})).unwrap();
        let env = bare.env("tok", Some("https://dash.example"));
        assert_eq!(env.len(), dainty_discord::config::VARIABLES.len());
        assert!(
            env.iter()
                .any(|(k, v)| k == bot::DASHBOARD_URL && v == "https://dash.example")
        );
        assert!(env.iter().any(|(k, v)| k == bot::GUILD_IDS && v.is_empty()));
    }

    #[test]
    fn refuses_what_it_does_not_know() {
        for value in [
            json!({"enabled": "yes"}),
            json!({"token": "x"}),
            json!({"botToken": "x"}),
            json!({"guildIds": [1]}),
            json!({"guildIds": "1"}),
            json!({"guildIds": ["x"]}),
            json!({"applicationId": "app"}),
            json!({"dashboardUrl": "ftp://dash"}),
            json!(true),
        ] {
            assert!(DiscordConfig::from_json(&value).is_err(), "{value}");
        }
    }
}
