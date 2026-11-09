// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `discord.env`, which `dainty-host setup` writes from host.toml's `[discord]` and secrets.env's
//! `DISCORD_BOT_TOKEN` (`crates/dainty-hostkit/src/hostconfig/discord.rs`).

use std::fmt;

use dainty_service_kit::env::Env;

use crate::interaction::Snowflake;

/// The bot's token, from secrets.env.
pub const TOKEN: &str = "DISCORD_BOT_TOKEN";
/// The application the token belongs to; checked against Discord's READY when set.
pub const APPLICATION_ID: &str = "DISCORD_APPLICATION_ID";
/// The guilds the commands are registered in and answered from, comma-separated; empty: every guild.
pub const GUILD_IDS: &str = "DISCORD_GUILD_IDS";
/// Where dainty-dash answers `GET /api/public/signup` (A8); empty: signups read as off.
pub const DASHBOARD_URL: &str = "DISCORD_DASHBOARD_URL";

/// Every variable setup writes, in order; each one is written even when empty, so a stale value in the host's
/// shell never reaches the bot (AGENTS.md, "When you add, remove or rename a variable").
pub const VARIABLES: [&str; 4] = [TOKEN, APPLICATION_ID, GUILD_IDS, DASHBOARD_URL];

/// The most guilds `DISCORD_GUILD_IDS` takes.
pub const MAX_GUILDS: usize = 25;
/// The longest token taken; Discord's are about 72 characters.
pub const MAX_TOKEN: usize = 256;

/// The bot token. Never shown: `Debug` and `Display` print a placeholder.
#[derive(Clone, PartialEq, Eq)]
pub struct BotToken(String);

impl BotToken {
    /// `text` as a token: printable ASCII with no spaces, at most [`MAX_TOKEN`] bytes.
    pub fn new(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let ok = !text.is_empty() && text.len() <= MAX_TOKEN && text.bytes().all(|b| b.is_ascii_graphic());
        if ok {
            Ok(Self(text.to_owned()))
        } else {
            Err(format!("{TOKEN} is not a bot token"))
        }
    }

    /// The token, for the Authorization header and Identify only.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for BotToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BotToken(..)")
    }
}

/// What the bot runs with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The bot token.
    pub token: BotToken,
    /// The application, when the host named it.
    pub application_id: Option<Snowflake>,
    /// The guilds it serves; empty: every guild it is in, with global commands.
    pub guild_ids: Vec<Snowflake>,
    /// dainty-dash's base URL, http or https.
    pub dashboard_url: Option<url::Url>,
}

impl Config {
    /// Reads `discord.env`.
    pub fn from_env(env: &Env) -> Result<Self, String> {
        let token = BotToken::new(env.get(TOKEN).ok_or_else(|| format!("{TOKEN} is not set"))?)?;
        let application_id = env
            .get(APPLICATION_ID)
            .map(|text| Snowflake::parse(text.trim()).ok_or_else(|| format!("{APPLICATION_ID} is not an id")))
            .transpose()?;
        let guild_ids = parse_guild_ids(env.get(GUILD_IDS).unwrap_or_default())?;
        let dashboard_url = env.get(DASHBOARD_URL).map(parse_dashboard_url).transpose()?;
        Ok(Self {
            token,
            application_id,
            guild_ids,
            dashboard_url,
        })
    }
}

/// A comma-separated list of guild ids.
pub fn parse_guild_ids(text: &str) -> Result<Vec<Snowflake>, String> {
    let mut out = Vec::new();
    for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let id = Snowflake::parse(part).ok_or_else(|| format!("{GUILD_IDS}: {part} is not a guild id"))?;
        if !out.contains(&id) {
            out.push(id);
        }
    }
    if out.len() > MAX_GUILDS {
        return Err(format!("{GUILD_IDS} names more than {MAX_GUILDS} guilds"));
    }
    Ok(out)
}

/// dainty-dash's base URL: http or https, no credentials, query or fragment.
pub fn parse_dashboard_url(text: &str) -> Result<url::Url, String> {
    let bad = || format!("{DASHBOARD_URL} must be an http:// or https:// address");
    let url = url::Url::parse(text.trim()).map_err(|_| bad())?;
    let ok = matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none();
    if ok { Ok(url) } else { Err(bad()) }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;

    #[test]
    fn reads_the_file_setup_writes() {
        let env = Env::from_pairs([
            (TOKEN, " abc.DEF-123_x "),
            (APPLICATION_ID, "123456789012345678"),
            (GUILD_IDS, "1, 2,2,"),
            (DASHBOARD_URL, "https://dash.example/"),
        ]);
        let config = Config::from_env(&env).unwrap();
        assert_eq!(config.token.expose(), "abc.DEF-123_x");
        assert_eq!(config.application_id, Snowflake::parse("123456789012345678"));
        assert_eq!(config.guild_ids.len(), 2);
        assert_eq!(config.dashboard_url.unwrap().as_str(), "https://dash.example/");
        assert!(!format!("{:?}", config.token).contains("abc"));

        let empty = Env::from_pairs([(TOKEN, "t"), (APPLICATION_ID, ""), (GUILD_IDS, ""), (DASHBOARD_URL, "")]);
        let config = Config::from_env(&empty).unwrap();
        assert_eq!(
            (config.application_id, config.guild_ids.len(), config.dashboard_url),
            (None, 0, None)
        );
    }

    #[test]
    fn refuses_what_it_does_not_take() {
        assert!(Config::from_env(&Env::from_pairs([])).is_err());
        assert!(Config::from_env(&Env::from_pairs([(TOKEN, "a b")])).is_err());
        assert!(Config::from_env(&Env::from_pairs([(TOKEN, "t"), (GUILD_IDS, "1,x")])).is_err());
        assert!(Config::from_env(&Env::from_pairs([(TOKEN, "t"), (APPLICATION_ID, "-1")])).is_err());
        for url in [
            "ftp://dash",
            "https://u:p@dash",
            "https://dash/?a=1",
            "dash.example",
            "javascript:x",
        ] {
            assert!(parse_dashboard_url(url).is_err(), "{url}");
        }
        let many = (1..=26).map(|n| n.to_string()).collect::<Vec<_>>().join(",");
        assert!(parse_guild_ids(&many).is_err());
    }
}
