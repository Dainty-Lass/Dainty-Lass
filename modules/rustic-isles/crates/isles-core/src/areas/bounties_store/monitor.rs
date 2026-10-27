//! The always-on economy monitors of the routes in this area (`diagnostics/economyMonitor.ts`): greppable server log
//! lines that never fail a request. `MYSTICPARADOX_ECONOMY_MONITOR=off` silences them. English, since they are a host's
//! log, not launcher text.

use serde_json::Value;
use service_kit::js;

use crate::settings::Settings;

pub fn enabled(settings: &Settings) -> bool {
    settings.on_unless_off("MYSTICPARADOX_ECONOMY_MONITOR")
}

fn text(value: Option<&Value>, default: &str) -> String {
    value.map_or_else(|| default.to_owned(), |v| js::string(Some(v)))
}

fn draft_summary(value: Option<&Value>) -> String {
    let Some(value @ Value::Object(_)) = value else {
        return "-".into();
    };
    let len = |key: &str| value.get(key).and_then(Value::as_array).map_or(0, Vec::len);
    format!(
        "choices={} previous={} b/s/g={}/{}/{}",
        len("current_draft_choices"),
        len("previous_draft_selections"),
        text(value.get("bronze_count"), "?"),
        text(value.get("silver_count"), "?"),
        text(value.get("gold_count"), "?")
    )
}

fn bounty_summary(bounty: &Value) -> String {
    let progress = bounty
        .get("objectives")
        .and_then(Value::as_array)
        .map(|o| {
            o.iter()
                .map(|x| text(x.get("progress"), "?"))
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_default();
    format!(
        "{}{{p={progress} claimed={}}}",
        text(bounty.get("bounty_id"), "?"),
        i32::from(bounty.get("claimed") == Some(&Value::Bool(true)))
    )
}

fn keys_of(body: Option<&Value>) -> String {
    body.and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect::<Vec<_>>().join(", "))
        .unwrap_or_default()
}

/// `MonitorBountyLoad`.
pub fn bounty_load(on: bool, user: &str, source: &str, served: &str) {
    if on {
        tracing::info!("[DraftMon] BOUNTY-LOAD user={user} src={source} served={served}");
    }
}

/// `MonitorBountySave`: what the server saved and what the store holds afterwards.
pub fn bounty_save(on: bool, user: &str, source: &str, body: Option<&Value>, outcome: &str) {
    if !on {
        return;
    }
    let bounties: &[Value] = body
        .and_then(|b| b.get("bounties"))
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    let claimed = bounties
        .iter()
        .filter(|b| b.get("claimed") == Some(&Value::Bool(true)))
        .count();
    let ids: Vec<String> = bounties.iter().take(6).map(bounty_summary).collect();
    tracing::info!(
        "[DraftMon] BOUNTY-SAVE user={user} src={source} keys=[{}] bounties={}(claimed {claimed})[{}{}] daily{{{}}} weekly{{{}}} base{{{}}} => {outcome}",
        keys_of(body),
        bounties.len(),
        ids.join(", "),
        if bounties.len() > 6 { ", ..." } else { "" },
        draft_summary(body.and_then(|b| b.get("draft_data_daily"))),
        draft_summary(body.and_then(|b| b.get("draft_data_weekly"))),
        draft_summary(body.and_then(|b| b.get("draft_data"))),
    );
}

fn cooldown_list(entries: &[(String, String)]) -> String {
    format!(
        "[{}]",
        entries
            .iter()
            .map(|(id, date)| format!("{id}@{date}"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// `MonitorCooldownLoad`.
pub fn cooldown_load(on: bool, user: &str, source: &str, served: &[(String, String)], detail: &str) {
    if on {
        tracing::info!(
            "[CooldownMon] LOAD user={user} src={source} served={} ({detail})",
            cooldown_list(served)
        );
    }
}

/// `MonitorCooldownSave`.
pub fn cooldown_save(on: bool, user: &str, source: &str, body: Option<&Value>, outcome: &str) {
    if !on {
        return;
    }
    let list = match body {
        Some(Value::Array(list)) => Some(list),
        Some(Value::Object(map)) => map.get("cooldowns").and_then(Value::as_array),
        _ => None,
    };
    let entries: Vec<(String, String)> = list
        .map(|l| {
            l.iter()
                .map(|e| {
                    (
                        text(e.get("cooldown_id"), "?"),
                        text(e.get("cooldown_started_date"), "?"),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    tracing::info!(
        "[CooldownMon] SAVE user={user} src={source} keys=[{}] cooldowns={} => {outcome}",
        keys_of(body),
        cooldown_list(&entries)
    );
}

/// A path value printed bare only when plain; anything else quoted so it cannot forge a log line (Express decodes
/// `%0D%0A` in params, and the line is written before the 403 for an unauthorised caller).
fn plain(value: &str) -> String {
    if super::cooldown_state::is_valid_cooldown_id(value) {
        value.to_owned()
    } else {
        Value::from(value.chars().take(64).collect::<String>()).to_string()
    }
}

/// `MonitorCooldownStart`.
pub fn cooldown_start(on: bool, user: &str, source: &str, cooldown_id: &str, outcome: &str) {
    if on {
        tracing::info!(
            "[CooldownMon] START user={} src={source} id={} => {outcome}",
            plain(user),
            plain(cooldown_id)
        );
    }
}
