// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Chat's settings from the env file the host kit wrote (`gateway.env`): read here for the host's in-process chat
//! and for the chat library alike (RD27), as the host's `compose.rs` read them.

use dainty_service_kit::env::Env;

use crate::moderation::DashboardSettings;
use crate::settings::{NickCheck, Settings};

/// `GATEWAY_SECRET`, `CHAT_PORT`, `CHAT_NICK_CHECK` and `CHAT_TRACE`.
pub fn settings(env: &Env) -> Result<Settings, String> {
    let mut settings = Settings::default();
    if let Some(secret) = env.get("GATEWAY_SECRET").map(str::trim).filter(|s| !s.is_empty()) {
        settings.gateway_secret = Some(secret.to_owned());
    }
    if let Some(port) = env.get("CHAT_PORT").and_then(|s| s.trim().parse::<u16>().ok()) {
        settings.port = port;
    }
    if env
        .get("CHAT_NICK_CHECK")
        .is_some_and(|s| s.trim().eq_ignore_ascii_case("log"))
    {
        settings.nick_check = NickCheck::Log;
    }
    if env
        .get("CHAT_TRACE")
        .is_some_and(|s| matches!(s.trim(), "1" | "true" | "TRUE"))
    {
        settings.trace = true;
    }
    let problems = settings.problems();
    if problems.is_empty() {
        Ok(settings)
    } else {
        Err(format!("chat settings: {}", problems.join("; ")))
    }
}

/// The staff dashboard chat reports to (`CHAT_DASHBOARD_URL`, `CHAT_DASHBOARD_INGEST_KEY`); none without a URL.
pub fn dashboard(env: &Env) -> Result<Option<DashboardSettings>, String> {
    let get = |key: &str| env.get(key).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
    let Some(url) = get("CHAT_DASHBOARD_URL") else {
        return Ok(None);
    };
    let dashboard = DashboardSettings {
        url,
        key: get("CHAT_DASHBOARD_INGEST_KEY").unwrap_or_default(),
    };
    let problems = dashboard.problems();
    if problems.is_empty() {
        Ok(Some(dashboard))
    } else {
        Err(format!("chat settings: {}", problems.join("; ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_env_file_sets_what_it_names_and_nothing_else() {
        let env = Env::from_pairs([
            ("CHAT_PORT", "0"),
            ("CHAT_NICK_CHECK", "LOG"),
            ("GATEWAY_SECRET", " s "),
        ]);
        let settings = settings(&env).unwrap();
        assert_eq!(settings.port, 0);
        assert_eq!(settings.nick_check, NickCheck::Log);
        assert_eq!(settings.gateway_secret.as_deref(), Some("s"));
        assert!(!settings.trace);
        assert_eq!(dashboard(&env).unwrap(), None);
        let short = Env::from_pairs([("CHAT_DASHBOARD_URL", "https://d"), ("CHAT_DASHBOARD_INGEST_KEY", "k")]);
        assert!(dashboard(&short).is_err());
    }
}
