// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! CLEAN-ROOM STUB (RD27): stands in for the Category A `env.rs` (chat's settings from its env file), which the
//! integration pass copies in unchanged. Behaviour from spec-rd27-chat §3.3; the function names are the rewrite's
//! assumption.

use std::path::PathBuf;

use service_kit::env::Env;

use crate::moderation::DashboardSettings;
use crate::settings::{NickCheck, Settings};

pub fn settings(env: &Env) -> Result<Settings, String> {
    let mut settings = Settings::default();
    if let Some(secret) = env.raw("GATEWAY_SECRET").map(str::trim).filter(|s| !s.is_empty()) {
        settings.gateway_secret = Some(secret.to_owned());
    }
    if let Some(port) = env.raw("CHAT_PORT").and_then(|p| p.trim().parse().ok()) {
        settings.port = port;
    }
    if env
        .raw("CHAT_NICK_CHECK")
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("log"))
    {
        settings.nick_check = NickCheck::Log;
    }
    settings.trace = env
        .raw("CHAT_TRACE")
        .is_some_and(|v| matches!(v.trim(), "1" | "true" | "TRUE"));
    let problems = settings.problems();
    if problems.is_empty() {
        Ok(settings)
    } else {
        Err(format!("chat settings: {}", problems.join("; ")))
    }
}

pub fn dashboard(env: &Env) -> Result<Option<DashboardSettings>, String> {
    let Some(url) = env.get("CHAT_DASHBOARD_URL") else {
        return Ok(None);
    };
    Ok(Some(DashboardSettings {
        url: url.to_owned(),
        ingest_key: env.get("CHAT_DASHBOARD_INGEST_KEY").unwrap_or_default().to_owned(),
    }))
}

/// The node database, which the core hands the library in its env file.
pub fn database(env: &Env) -> Result<store::DatabaseConfig, String> {
    if env.get("REFORGED_DB_ENGINE").is_none() {
        return Err(String::from("chat: REFORGED_DB_ENGINE is not set"));
    }
    let file = env
        .get("REFORGED_DB_FILE")
        .ok_or_else(|| String::from("chat: REFORGED_DB_FILE is not set"))?;
    store::DatabaseConfig::from_lookup(|name| env.raw(name).map(str::to_owned), PathBuf::from(file))
}
