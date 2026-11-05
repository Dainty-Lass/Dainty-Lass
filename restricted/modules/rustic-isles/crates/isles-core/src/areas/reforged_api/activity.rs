//! Activity monitor (`src/activity/activityRules.ts`, `src/activity/activityMonitor.ts`,
//! `src/controllers/adminActivity.ts`).

use axum::response::Response;
use module_api::{Actor, ActorContext};
use serde_json::{Map, Value, json};

use super::admin::{Ctx, error, limit_of, page_of, route, user_id_of};
use super::util::{self, Req};
use crate::db::IslesStore;
use crate::http::Handlers;
use crate::ids::{AccountId, UserId};
use crate::repo::AnyConnection;
use crate::repo::progression::ProgressionGrantRecord;
use crate::repo::reforged::{
    ActivityFlagRecord, ActivityHuntRecord, AdminAuditRecord, HuntKind, HuntResult, SlayersPathUnlockRecord,
};
use crate::repo::{RepoResult, Repositories};
use crate::routes::{Method, Service};
use crate::settings::Settings;

pub const ACTIVITY_RETENTION_DAYS: i64 = 400;
pub const ACTIVITY_SESSION_GAP_MS: i64 = 3 * 60 * 1000;
pub const ACTIVITY_HUNT_GRACE_MS: i64 = 10 * 60 * 1000;
pub const ACTIVITY_HUNT_MAX_MS: i64 = 6 * 60 * 60 * 1000;
pub const ACTIVITY_MIN_RATE_MINUTES: f64 = 15.0;
pub const ACTIVITY_MAX_ITEMS_PER_TRANSACTION: usize = 8;
pub const ACTIVITY_IGNORED_SOURCE: &str = "OPERATOR_TEST";

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const DEFAULT_RANGE_DAYS: i64 = 30;
const OVERVIEW_ROW_LIMIT: u32 = 50_000;
const PLAYER_DAY_LIMIT: u32 = 400;
const TIMELINE_LIMIT: usize = 100;
const LEDGER_LIMIT: u32 = 50;
const HUNT_LIMIT: u32 = 50;
const UNLOCK_LIMIT: u32 = 200;
const FLAG_LIMIT: u32 = 50;
const HUNT_KINDS: [&str; 3] = ["dedicated", "realm", "playerHosted"];
const COUNTER_FIELDS: [&str; 7] = [
    "playtimeMinutes",
    "sessions",
    "hostingMinutes",
    "huntsHosted",
    "challengesClaimed",
    "slayersPathUnlocks",
    "reforges",
];

#[derive(Debug, Clone, PartialEq)]
pub struct ActivitySettings {
    pub enabled: bool,
    pub kills_per_hour_flag: f64,
    pub rewards_per_hour_flag: f64,
    pub platinum_daily_credit_flag: f64,
    pub platinum_daily_spend_flag: f64,
    pub seasonal_coin_daily_flag: f64,
    pub xp_per_minute_flag: f64,
    pub expected_launcher_version: Option<String>,
    pub expected_launcher_build_id: Option<String>,
}

fn read_non_negative(settings: &Settings, name: &str, fallback: f64) -> f64 {
    let Some(raw) = settings.raw(name).map(str::trim).filter(|s| !s.is_empty()) else {
        return fallback;
    };
    raw.parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
        .unwrap_or(fallback)
}

fn clean_build_token(value: Option<&str>, max_len: usize) -> Option<String> {
    let text = value.map(service_kit::js::trim).unwrap_or_default();
    if text.is_empty() {
        return None;
    }
    let cleaned: String = text
        .chars()
        .take(max_len)
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_owned();
    (!cleaned.is_empty()).then_some(cleaned)
}

pub fn read_activity_settings(settings: &Settings) -> ActivitySettings {
    let raw = settings
        .raw("RUSTIC_ISLES_ACTIVITY_MONITOR")
        .map(str::trim)
        .unwrap_or_default();
    let enabled = !matches!(raw.to_ascii_lowercase().as_str(), "0" | "false" | "off" | "no");
    ActivitySettings {
        enabled,
        kills_per_hour_flag: read_non_negative(settings, "RUSTIC_ISLES_KILLS_PER_HOUR_FLAG", 90.0),
        rewards_per_hour_flag: read_non_negative(settings, "RUSTIC_ISLES_REWARDS_PER_HOUR_FLAG", 600.0),
        platinum_daily_credit_flag: read_non_negative(settings, "RUSTIC_ISLES_PLAT_DAILY_FLAG", 5_000.0),
        platinum_daily_spend_flag: read_non_negative(settings, "RUSTIC_ISLES_PLAT_SPEND_FLAG", 10_000.0),
        seasonal_coin_daily_flag: read_non_negative(settings, "RUSTIC_ISLES_SEASONAL_COIN_DAILY_FLAG", 20_000.0),
        xp_per_minute_flag: read_non_negative(settings, "RUSTIC_ISLES_XP_PER_MINUTE_FLAG", 2_500.0),
        expected_launcher_version: clean_build_token(settings.raw("RUSTIC_ISLES_EXPECTED_LAUNCHER_VERSION"), 64),
        expected_launcher_build_id: clean_build_token(settings.raw("RUSTIC_ISLES_EXPECTED_LAUNCHER_BUILD_ID"), 96),
    }
}

pub fn day_of(ms: i64) -> String {
    util::iso(ms).chars().take(10).collect()
}

pub fn day_start_ms(day: &str) -> Option<i64> {
    util::date_parse(&format!("{day}T00:00:00.000Z"))
}

pub fn activity_ttl_at(day: &str) -> i64 {
    day_start_ms(day).unwrap_or(0) + ACTIVITY_RETENTION_DAYS * DAY_MS
}

pub fn is_valid_day(text: &str) -> bool {
    if text.len() != 10 {
        return false;
    }
    let bytes = text.as_bytes();
    let shape = bytes[0..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(u8::is_ascii_digit);
    shape && day_start_ms(text).is_some_and(|ms| day_of(ms) == text)
}

pub fn safe_key(text: &str) -> String {
    let valid = (1..=80).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    if valid { text.to_owned() } else { "OTHER".to_owned() }
}

pub fn kill_token_name(catalog_id: &str) -> Option<String> {
    catalog_id
        .strip_prefix("TOKEN_KILL_COUNT_")
        .filter(|rest| !rest.is_empty())
        .map(safe_key)
}

pub fn is_ignored_activity_source(source: Option<&str>) -> bool {
    source == Some(ACTIVITY_IGNORED_SOURCE)
}

#[derive(Debug, Clone, PartialEq)]
pub struct DailyFlagCandidate {
    pub kind: &'static str,
    pub severity: &'static str,
    pub detail: String,
}

fn is_platinum_movement(catalog_id: &str) -> bool {
    catalog_id == "CURRENCY_PLATINUM" || catalog_id.starts_with("CURRENCY_PLATINUM_")
}

fn sum_values(value: Option<&Value>) -> f64 {
    let Some(Value::Object(map)) = value else {
        return 0.0;
    };
    map.values().filter_map(Value::as_f64).sum()
}

pub fn daily_flag_candidates(row: &Value, settings: &ActivitySettings) -> Vec<DailyFlagCandidate> {
    let mut flags = Vec::new();
    let playtime = row.get("playtimeMinutes").and_then(Value::as_f64).unwrap_or(0.0);
    let mut plat_credited = 0.0;
    let mut plat_debited = 0.0;
    if let Some(Value::Object(currency)) = row.get("currency") {
        for (id, totals) in currency {
            if !is_platinum_movement(id) {
                continue;
            }
            plat_credited += totals.get("credited").and_then(Value::as_f64).unwrap_or(0.0);
            plat_debited += totals.get("debited").and_then(Value::as_f64).unwrap_or(0.0);
        }
    }
    if plat_credited > settings.platinum_daily_credit_flag {
        flags.push(DailyFlagCandidate {
            kind: "PLATINUM_CREDIT_DAY",
            severity: "review",
            detail: format!(
                "credited={plat_credited} threshold={}",
                settings.platinum_daily_credit_flag
            ),
        });
    }
    if plat_debited > settings.platinum_daily_spend_flag {
        flags.push(DailyFlagCandidate {
            kind: "PLATINUM_SPEND_DAY",
            severity: "review",
            detail: format!(
                "debited={plat_debited} threshold={}",
                settings.platinum_daily_spend_flag
            ),
        });
    }
    let seasonal = row
        .get("currency")
        .and_then(|c| c.get("CURRENCY_SEASONAL_COIN"))
        .and_then(|c| c.get("credited"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    if seasonal > settings.seasonal_coin_daily_flag {
        flags.push(DailyFlagCandidate {
            kind: "SEASONAL_COIN_DAY",
            severity: "review",
            detail: format!("credited={seasonal} threshold={}", settings.seasonal_coin_daily_flag),
        });
    }
    if playtime >= ACTIVITY_MIN_RATE_MINUTES {
        let hours = playtime / 60.0;
        let xp = sum_values(row.get("xp"));
        let xp_per_minute = xp / playtime;
        if xp_per_minute > settings.xp_per_minute_flag {
            flags.push(DailyFlagCandidate {
                kind: "XP_PER_MINUTE",
                severity: "review",
                detail: format!(
                    "xp={xp} playtimeMinutes={playtime} perMinute={xp_per_minute:.1} threshold={}",
                    settings.xp_per_minute_flag
                ),
            });
        }
        let kills = sum_values(row.get("kills"));
        if kills / hours > settings.kills_per_hour_flag {
            flags.push(DailyFlagCandidate {
                kind: "KILLS_PER_HOUR",
                severity: "review",
                detail: format!(
                    "kills={kills} playtimeMinutes={playtime} threshold={}",
                    settings.kills_per_hour_flag
                ),
            });
        }
        let rewards = sum_values(row.get("itemsGained"));
        if rewards / hours > settings.rewards_per_hour_flag {
            flags.push(DailyFlagCandidate {
                kind: "REWARDS_PER_HOUR",
                severity: "review",
                detail: format!(
                    "items={rewards} playtimeMinutes={playtime} threshold={}",
                    settings.rewards_per_hour_flag
                ),
            });
        }
    }
    flags
}

/// Records increments on an account's daily row and evaluates any resulting review flags.
pub async fn record_daily_increments(
    store: &IslesStore,
    settings: &Settings,
    account: &AccountId,
    at_ms: i64,
    increments: Vec<(String, f64)>,
) {
    let activity = read_activity_settings(settings);
    if !activity.enabled || increments.is_empty() {
        return;
    }
    let day = day_of(at_ms);
    let ttl = activity_ttl_at(&day);
    let target = account.clone();
    let target_in = target.clone();
    let now_iso = util::iso(at_ms);
    let _ = store
        .account_write(&target, &ActorContext::new(Actor::System), move |tx, repos| {
            let after = repos.activity.add_to_daily(tx, &target_in, &day, &increments, ttl)?;
            for candidate in daily_flag_candidates(&after, &activity) {
                let user_id = user_id_of(tx.conn(), repos, &target_in)?;
                let flag = ActivityFlagRecord {
                    id: format!("{}:{user_id}:{day}", candidate.kind),
                    kind: candidate.kind.to_owned(),
                    severity: candidate.severity.to_owned(),
                    account: target_in.clone(),
                    day: day.clone(),
                    detail: candidate.detail,
                    acknowledged: false,
                    created_at: now_iso.clone(),
                    acknowledged_at: None,
                    acknowledged_by: None,
                    note: None,
                    ttl_at: ttl,
                };
                let _ = repos.activity.insert_flag_if_absent(tx, &flag)?;
            }
            Ok(())
        })
        .await;
}

// ---------------------------------------------------------------- pure summaries

fn num(value: Option<&Value>) -> f64 {
    value.and_then(Value::as_f64).filter(|n| n.is_finite()).unwrap_or(0.0)
}

fn num_json(value: f64) -> Value {
    if value.fract() == 0.0 && value >= (i64::MIN as f64) && value <= (i64::MAX as f64) {
        json!(value as i64)
    } else {
        json!(value)
    }
}

pub fn empty_totals() -> Map<String, Value> {
    let mut into = Map::new();
    add_daily_counters(&mut into, &Value::Object(Map::new()));
    into
}

pub fn add_daily_counters(into: &mut Map<String, Value>, row: &Value) {
    for field in COUNTER_FIELDS {
        let next = num(into.get(field)) + num(row.get(field));
        into.insert(field.to_owned(), num_json(next));
    }
    for group in ["huntsStarted", "huntsCompleted", "huntsAbandoned"] {
        let target = into
            .entry(group.to_owned())
            .or_insert_with(|| json!({ "dedicated": 0, "realm": 0, "playerHosted": 0 }));
        if let Some(target_map) = target.as_object_mut() {
            let source = row.get(group);
            for kind in HUNT_KINDS {
                let next = num(target_map.get(kind)) + num(source.and_then(|s| s.get(kind)));
                target_map.insert(kind.to_owned(), num_json(next));
            }
        }
    }
    for group in ["kills", "xp", "itemsGained", "platinumSpendsByItem"] {
        let target = into.entry(group.to_owned()).or_insert_with(|| json!({}));
        if let (Some(target_map), Some(Value::Object(source_map))) = (target.as_object_mut(), row.get(group)) {
            for (k, v) in source_map {
                let next = num(target_map.get(k)) + num(Some(v));
                target_map.insert(k.clone(), num_json(next));
            }
        }
    }
    let currency = into.entry("currency".to_owned()).or_insert_with(|| json!({}));
    if let (Some(currency_map), Some(Value::Object(row_currency))) = (currency.as_object_mut(), row.get("currency")) {
        for (id, entry) in row_currency {
            let target = currency_map
                .entry(id.clone())
                .or_insert_with(|| json!({ "credited": 0, "debited": 0 }));
            if let Some(target_map) = target.as_object_mut() {
                let credited = num(target_map.get("credited")) + num(entry.get("credited"));
                let debited = num(target_map.get("debited")) + num(entry.get("debited"));
                target_map.insert("credited".into(), num_json(credited));
                target_map.insert("debited".into(), num_json(debited));
            }
        }
    }
}

pub fn is_active_row(row: &Value) -> bool {
    num(row.get("playtimeMinutes")) > 0.0 || num(row.get("sessions")) > 0.0
}

pub fn overview_days(from: &str, to: &str, rows: &[Value]) -> Vec<Value> {
    let Some(start_ms) = day_start_ms(from) else {
        return Vec::new();
    };
    let Some(end_ms) = day_start_ms(to) else {
        return Vec::new();
    };
    let mut order = Vec::new();
    let mut by_day: std::collections::HashMap<String, (Map<String, Value>, u64)> = std::collections::HashMap::new();
    let mut cursor = start_ms;
    while cursor <= end_ms {
        let key = day_of(cursor);
        order.push(key.clone());
        by_day.insert(key, (empty_totals(), 0));
        cursor += DAY_MS;
    }
    for row in rows {
        let Some(key) = row.get("day").and_then(Value::as_str) else {
            continue;
        };
        let Some((totals, active)) = by_day.get_mut(key) else {
            continue;
        };
        add_daily_counters(totals, row);
        if is_active_row(row) {
            *active += 1;
        }
    }
    order
        .into_iter()
        .filter_map(|day| {
            let (totals, active) = by_day.remove(&day)?;
            let mut out = Map::new();
            out.insert("day".into(), json!(day));
            out.insert("activePlayers".into(), json!(active));
            for (k, v) in totals {
                out.insert(k, v);
            }
            Some(Value::Object(out))
        })
        .collect()
}

pub fn player_totals(rows: &[Value]) -> Value {
    let mut totals = empty_totals();
    for row in rows {
        add_daily_counters(&mut totals, row);
    }
    Value::Object(totals)
}

fn safe_day_row(row: &Value) -> Value {
    let Some(map) = row.as_object() else {
        return json!({});
    };
    let mut out = Map::new();
    for (k, v) in map {
        if !matches!(k.as_str(), "_id" | "userId" | "ttlAt") {
            out.insert(k.clone(), v.clone());
        }
    }
    Value::Object(out)
}

fn public_flag(conn: &mut AnyConnection, repos: &Repositories, record: &ActivityFlagRecord) -> RepoResult<Value> {
    let user_id = user_id_of(conn, repos, &record.account)?;
    let acknowledged_by = match &record.acknowledged_by {
        Some(by) => match by.parse::<AccountId>() {
            Ok(acc) => Some(user_id_of(conn, repos, &acc)?),
            Err(_) => Some(by.clone()),
        },
        None => None,
    };
    Ok(json!({
        "id": record.id,
        "kind": record.kind,
        "severity": record.severity,
        "userId": user_id,
        "day": record.day,
        "detail": record.detail,
        "status": if record.acknowledged { "acknowledged" } else { "open" },
        "createdAt": record.created_at,
        "acknowledgedAt": record.acknowledged_at,
        "acknowledgedBy": acknowledged_by,
        "note": record.note,
    }))
}

fn hunt_of_timeline(hunt: &ActivityHuntRecord) -> Value {
    json!({
        "role": if hunt.host { "host" } else { "participant" },
        "kind": hunt.kind.as_str(),
        "huntId": hunt.hunt_id,
        "huntRef": hunt.hunt_ref,
        "startedAt": hunt.started_at,
        "endedAt": hunt.ended_at,
        "endReason": hunt.end_reason,
        "result": hunt.result.map(HuntResult::as_str),
        "kills": hunt.kills,
    })
}

fn unlock_of(unlock: &SlayersPathUnlockRecord) -> Value {
    json!({
        "nodeId": unlock.node_id,
        "unlockedAt": unlock.unlocked_at,
        "source": unlock.source,
    })
}

fn player_timeline(
    hunts: &[ActivityHuntRecord],
    unlocks: &[SlayersPathUnlockRecord],
    flags: &[ActivityFlagRecord],
    ledger: &[Value],
    grants: &[ProgressionGrantRecord],
) -> Vec<Value> {
    let mut entries: Vec<(String, Value)> = Vec::new();
    for hunt in hunts {
        if !hunt.started_at.is_empty() {
            entries.push((
                hunt.started_at.clone(),
                json!({
                    "at": hunt.started_at,
                    "type": "hunt",
                    "detail": hunt_of_timeline(hunt),
                }),
            ));
        }
    }
    for unlock in unlocks {
        if !unlock.unlocked_at.is_empty() {
            entries.push((
                unlock.unlocked_at.clone(),
                json!({
                    "at": unlock.unlocked_at,
                    "type": "slayersPathUnlock",
                    "detail": unlock_of(unlock),
                }),
            ));
        }
    }
    for flag in flags {
        if !flag.created_at.is_empty() {
            entries.push((
                flag.created_at.clone(),
                json!({
                    "at": flag.created_at,
                    "type": "flag",
                    "detail": {
                        "id": flag.id,
                        "kind": flag.kind,
                        "severity": flag.severity,
                        "status": if flag.acknowledged { "acknowledged" } else { "open" },
                    },
                }),
            ));
        }
    }
    for row in ledger {
        let at = row
            .get("completedAt")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .or_else(|| row.get("createdAt").and_then(Value::as_str))
            .unwrap_or_default();
        if !at.is_empty() {
            entries.push((
                at.to_owned(),
                json!({
                    "at": at,
                    "type": "inventory",
                    "detail": {
                        "status": row.get("status").cloned().unwrap_or(Value::Null),
                        "economySource": row.get("economySource").cloned().unwrap_or(Value::Null),
                        "economySessionId": row.get("economySessionId").cloned().unwrap_or(Value::Null),
                        "characterId": row.get("characterId").cloned().unwrap_or(Value::Null),
                    },
                }),
            ));
        }
    }
    for grant in grants {
        if !grant.created_at.is_empty() {
            entries.push((
                grant.created_at.clone(),
                json!({
                    "at": grant.created_at,
                    "type": "xp",
                    "detail": {
                        "progressionId": grant.progression_id,
                        "amount": grant.amount,
                        "grantId": grant.grant_id,
                    },
                }),
            ));
        }
    }
    entries.sort_by(|a, b| b.0.cmp(&a.0));
    entries.into_iter().take(TIMELINE_LIMIT).map(|(_, v)| v).collect()
}

// ---------------------------------------------------------------- handlers

fn read_range(req: &Req) -> Result<(String, String), Response> {
    let to_text = req.query_string("to").map(str::trim).unwrap_or_default();
    let from_text = req.query_string("from").map(str::trim).unwrap_or_default();
    let to = if to_text.is_empty() {
        day_of(util::now_ms())
    } else {
        to_text.to_owned()
    };
    let from = if from_text.is_empty() {
        let Some(to_ms) = day_start_ms(&to) else {
            return Err(error(
                400,
                "ACTIVITY_RANGE_INVALID",
                "from and to must be UTC days, YYYY-MM-DD.",
            ));
        };
        day_of(to_ms - (DEFAULT_RANGE_DAYS - 1) * DAY_MS)
    } else {
        from_text.to_owned()
    };
    if !is_valid_day(&to) || !is_valid_day(&from) {
        return Err(error(
            400,
            "ACTIVITY_RANGE_INVALID",
            "from and to must be UTC days, YYYY-MM-DD.",
        ));
    }
    if from > to {
        return Err(error(400, "ACTIVITY_RANGE_INVALID", "from must not be after to."));
    }
    let span = (day_start_ms(&to).unwrap_or(0) - day_start_ms(&from).unwrap_or(0)) / DAY_MS + 1;
    if span > ACTIVITY_RETENTION_DAYS {
        return Err(error(
            400,
            "ACTIVITY_RANGE_TOO_LONG",
            &format!("The range may be at most {ACTIVITY_RETENTION_DAYS} days (the retention)."),
        ));
    }
    Ok((from, to))
}

fn resolve_account(conn: &mut AnyConnection, repos: &Repositories, user_id: &str) -> RepoResult<Option<AccountId>> {
    if let Some(game) = repos.accounts.find_by_user_id(conn, &UserId(user_id.to_owned()))? {
        return Ok(Some(game.id));
    }
    Ok(user_id.parse::<AccountId>().ok())
}

async fn get_player(ctx: Ctx) -> Response {
    let user_id = ctx.params.get("userId").map(str::trim).unwrap_or_default().to_owned();
    if user_id.is_empty() || user_id.len() > 200 {
        return error(400, "ACTIVITY_USER_INVALID", "The account id is missing or too long.");
    }
    let (from, to) = match read_range(&ctx.req) {
        Ok(range) => range,
        Err(response) => return response,
    };
    let monitor_enabled = read_activity_settings(&ctx.shared.settings).enabled;
    let lookup = (user_id.clone(), from.clone(), to.clone());
    let result = ctx
        .store
        .read(move |conn, repos| {
            let Some(account) = resolve_account(conn, repos, &lookup.0)? else {
                return Ok((Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()));
            };
            let rows = repos
                .activity
                .find_daily_rows(conn, &account, &lookup.1, &lookup.2, PLAYER_DAY_LIMIT)?;
            let hunts = repos.activity.find_hunts_for_account(conn, &account, HUNT_LIMIT)?;
            let unlocks = repos.activity.find_unlocks_for_account(conn, &account, UNLOCK_LIMIT)?;
            let flags_raw = repos.activity.find_flags_for_account(conn, &account, FLAG_LIMIT)?;
            let mut flags = Vec::with_capacity(flags_raw.len());
            for f in &flags_raw {
                flags.push(public_flag(conn, repos, f)?);
            }
            let ledger = repos
                .inventory_transactions
                .list_recent_for_account(conn, &account, LEDGER_LIMIT)?;
            let grants = repos
                .progression_grants
                .list_recent_for_account(conn, &account, LEDGER_LIMIT)?;
            let timeline = player_timeline(&hunts, &unlocks, &flags_raw, &ledger, &grants);
            Ok((rows, hunts, unlocks, flags, timeline, flags_raw))
        })
        .await;
    match result {
        Ok((rows, hunts, unlocks, flags, timeline, _)) => util::json(
            200,
            &json!({
                "userId": user_id,
                "from": from,
                "to": to,
                "monitorEnabled": monitor_enabled,
                "days": rows.iter().map(safe_day_row).collect::<Vec<_>>(),
                "totals": player_totals(&rows),
                "unlocks": unlocks.iter().map(unlock_of).collect::<Vec<_>>(),
                "hunts": hunts.iter().map(hunt_of_timeline).collect::<Vec<_>>(),
                "flags": flags,
                "timeline": timeline,
            }),
        ),
        Err(e) => {
            tracing::error!("[ActivityMon] player read failed: {e}");
            error(500, "ACTIVITY_READ_FAILED", "The activity record could not be read.")
        }
    }
}

async fn get_overview(ctx: Ctx) -> Response {
    let (from, to) = match read_range(&ctx.req) {
        Ok(range) => range,
        Err(response) => return response,
    };
    let monitor_enabled = read_activity_settings(&ctx.shared.settings).enabled;
    let lookup = (from.clone(), to.clone());
    let result = ctx
        .store
        .read(move |conn, repos| {
            repos
                .activity
                .find_daily_rows_in_range(conn, &lookup.0, &lookup.1, OVERVIEW_ROW_LIMIT)
        })
        .await;
    match result {
        Ok(rows) => {
            let truncated = rows.len() >= OVERVIEW_ROW_LIMIT as usize;
            let days = overview_days(&from, &to, &rows);
            util::json(
                200,
                &json!({
                    "from": from,
                    "to": to,
                    "monitorEnabled": monitor_enabled,
                    "truncated": truncated,
                    "days": days,
                }),
            )
        }
        Err(e) => {
            tracing::error!("[ActivityMon] overview read failed: {e}");
            error(500, "ACTIVITY_READ_FAILED", "The activity overview could not be read.")
        }
    }
}

async fn list_flags(ctx: Ctx) -> Response {
    let status_text = ctx
        .req
        .query_string("status")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("open")
        .to_owned();
    let acknowledged = match status_text.as_str() {
        "open" => Some(false),
        "acknowledged" => Some(true),
        "all" => None,
        _ => {
            return error(
                400,
                "ACTIVITY_FLAG_QUERY_INVALID",
                "status must be open, acknowledged or all.",
            );
        }
    };
    let page = page_of(&ctx.req);
    let limit = limit_of(&ctx.req, 25, 100);
    let skip = u32::try_from(((page - 1) * limit).clamp(0, i64::from(u32::MAX))).unwrap_or(u32::MAX);
    let take = u32::try_from(limit).unwrap_or(25);
    let result = ctx
        .store
        .read(move |conn, repos| {
            let (records, total) = repos.activity.list_flags(conn, acknowledged, skip, take)?;
            let mut flags = Vec::with_capacity(records.len());
            for r in &records {
                flags.push(public_flag(conn, repos, r)?);
            }
            Ok((flags, total))
        })
        .await;
    match result {
        Ok((flags, total)) => util::json(
            200,
            &json!({
                "flags": flags,
                "total": total,
                "page": page,
                "limit": limit,
                "status": status_text,
            }),
        ),
        Err(e) => {
            tracing::error!("[ActivityMon] flag list failed: {e}");
            error(500, "ACTIVITY_READ_FAILED", "The flags could not be read.")
        }
    }
}

async fn acknowledge_flag(ctx: Ctx) -> Response {
    let caller = ctx.caller().clone();
    let id = ctx.params.get("id").unwrap_or_default().to_owned();
    if id.is_empty() || id.len() > 300 {
        return error(400, "ACTIVITY_FLAG_INVALID", "The flag id is missing.");
    }
    let fields = ctx.fields();
    if fields.get("acknowledged") != Some(&Value::Bool(true)) {
        return error(
            400,
            "ACTIVITY_FLAG_INVALID",
            "Send {\"acknowledged\": true} (an optional note of up to 500 characters).",
        );
    }
    let note = fields
        .get("note")
        .and_then(Value::as_str)
        .map(|s| service_kit::js::trim(s).chars().take(500).collect::<String>())
        .filter(|s| !s.is_empty());
    let at = util::now_iso();
    let ip = ctx.req.ip();
    let actor = ActorContext::new(Actor::Admin {
        key: caller.user_id.clone(),
    });
    let result = ctx
        .store
        .write(&actor, move |tx, repos| {
            let Some(before) = repos.activity.find_flag(tx.conn(), &id)? else {
                return Ok(None);
            };
            let after = repos
                .activity
                .acknowledge_flag(tx, &id, &caller.account.account, &at, note.as_deref())?;
            if !before.acknowledged && after.as_ref().is_some_and(|a| a.acknowledged) {
                let target_user = user_id_of(tx.conn(), repos, &before.account)?;
                repos.admin.append_audit(
                    tx,
                    &AdminAuditRecord {
                        id: uuid::Uuid::new_v4().to_string(),
                        actor_user_id: caller.user_id.clone(),
                        target_user_id: Some(target_user),
                        action: "activity.flag.acknowledge".into(),
                        old_state: Some(json!({ "status": "open" })),
                        new_state: Some(json!({ "status": "acknowledged" })),
                        reason: note,
                        ip,
                        request_id: uuid::Uuid::new_v4().to_string(),
                        created_at: at,
                    },
                )?;
            }
            let record = after.unwrap_or(before);
            Ok(Some(public_flag(tx.conn(), repos, &record)?))
        })
        .await;
    match result {
        Ok(Some(flag)) => util::json(200, &json!({ "flag": flag })),
        Ok(None) => error(404, "ACTIVITY_FLAG_NOT_FOUND", "No activity flag has that id."),
        Err(e) => {
            tracing::error!("[ActivityMon] flag acknowledge failed: {e}");
            error(500, "ACTIVITY_WRITE_FAILED", "The flag could not be acknowledged.")
        }
    }
}

pub fn register(handlers: &mut Handlers) {
    let b = Service::Backend;
    handlers.on(b, Method::Get, "/admin/v1/activity/players/:userId", |call| {
        Box::pin(route(call, true, get_player))
    });
    handlers.on(b, Method::Get, "/admin/v1/activity/overview", |call| {
        Box::pin(route(call, true, get_overview))
    });
    handlers.on(b, Method::Get, "/admin/v1/activity/flags", |call| {
        Box::pin(route(call, true, list_flags))
    });
    handlers.on(b, Method::Patch, "/admin/v1/activity/flags/:id", |call| {
        Box::pin(route(call, true, acknowledge_flag))
    });
}

// ---------------------------------------------------------------- activity monitor recorders

pub fn advance_playtime(previous_last_seen_ms: Option<i64>, now_ms: i64) -> (f64, f64, i64) {
    let Some(prev) = previous_last_seen_ms.filter(|v| *v > 0) else {
        return (0.0, 1.0, now_ms);
    };
    let gap_ms = now_ms - prev;
    if gap_ms <= 0 {
        return (0.0, 0.0, prev);
    }
    if gap_ms > ACTIVITY_SESSION_GAP_MS {
        return (0.0, 1.0, now_ms);
    }
    ((gap_ms as f64) / 60_000.0, 0.0, now_ms)
}

pub fn classify_reported_launcher_build(
    shared: &crate::http::Shared,
    channel: &str,
    report: Option<&crate::areas::auth::policy::LauncherBuildReport>,
) -> crate::areas::auth::policy::LauncherBuildVerdict {
    let Some(report) = report else {
        return crate::areas::auth::policy::LauncherBuildVerdict::Unknown;
    };
    let pinned = super::updates::published_guard_artifacts(shared, channel, "win64").and_then(|artifacts| {
        artifacts
            .into_iter()
            .find(|(role, _, _)| role == "LAUNCHER")
            .map(|(_, _, sha)| sha)
    });
    let served = super::updates::served_launcher_version(shared, "windows", "x86_64");
    crate::areas::auth::policy::classify_launcher_build(Some(report), pinned.as_deref(), served.as_deref())
}

fn spawn_activity<F>(future: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(future);
    }
}

pub fn touch_game_session(shared: &crate::http::Shared, user_id: &str) {
    let user = user_id.trim();
    if user.is_empty() {
        return;
    }
    super::presence::touch(user);
    let Some(store) = shared.store.clone() else {
        return;
    };
    let settings = shared.settings.clone();
    let activity = read_activity_settings(&settings);
    if !activity.enabled {
        return;
    }
    let user = user.to_owned();
    let now_ms = util::now_ms();
    spawn_activity(async move {
        let day = day_of(now_ms);
        let ttl = activity_ttl_at(&day);
        let user_lookup = user.clone();
        let Ok(Some(account)) = store
            .read(move |conn, repos| resolve_account(conn, repos, &user_lookup))
            .await
        else {
            return;
        };
        let target = account.clone();
        let target_in = target.clone();
        let day_in = day.clone();
        let Ok(deltas) = store
            .account_write(&target, &ActorContext::new(Actor::System), move |tx, repos| {
                let rows = repos
                    .activity
                    .find_daily_rows(tx.conn(), &target_in, &day_in, &day_in, 1)?;
                let prev_seen = rows.first().and_then(|r| r.get("lastSeenMs")).and_then(Value::as_i64);
                let (minutes_delta, sessions_delta, next_seen) = advance_playtime(prev_seen, now_ms);
                let fields = vec![("lastSeenMs".to_owned(), json!(next_seen))];
                let _ = repos.activity.set_daily_fields(tx, &target_in, &day_in, &fields, ttl)?;
                let mut inc = Vec::new();
                if minutes_delta > 0.0 {
                    inc.push(("playtimeMinutes".to_owned(), minutes_delta));
                }
                if sessions_delta > 0.0 {
                    inc.push(("sessions".to_owned(), sessions_delta));
                }
                Ok(inc)
            })
            .await
        else {
            return;
        };
        if !deltas.is_empty() {
            record_daily_increments(&store, &settings, &account, now_ms, deltas).await;
        }
    });
}

pub fn record_launcher_build(
    shared: &crate::http::Shared,
    user_id: &str,
    channel: &str,
    report: Option<&crate::areas::auth::policy::LauncherBuildReport>,
) {
    let Some(report) = report.cloned() else {
        return;
    };
    let Some(store) = shared.store.clone() else {
        return;
    };
    let settings = shared.settings.clone();
    if !read_activity_settings(&settings).enabled || user_id.trim().is_empty() {
        return;
    }
    let pinned = super::updates::published_guard_artifacts(shared, channel, "win64").and_then(|artifacts| {
        artifacts
            .into_iter()
            .find(|(role, _, _)| role == "LAUNCHER")
            .map(|(_, _, sha)| sha)
    });
    let served = super::updates::served_launcher_version(shared, "windows", "x86_64");
    let verdict =
        crate::areas::auth::policy::classify_launcher_build(Some(&report), pinned.as_deref(), served.as_deref());
    if verdict != crate::areas::auth::policy::LauncherBuildVerdict::Unofficial {
        return;
    }
    let Some(pinned) = pinned else {
        return;
    };
    let user = user_id.trim().to_owned();
    let chan = channel.to_owned();
    let now_ms = util::now_ms();
    let now_iso = util::iso(now_ms);
    spawn_activity(async move {
        let day = day_of(now_ms);
        let ttl = activity_ttl_at(&day);
        let user_lookup = user.clone();
        let Ok(Some(account)) = store
            .read(move |conn, repos| resolve_account(conn, repos, &user_lookup))
            .await
        else {
            return;
        };
        let target = account.clone();
        let target_in = target.clone();
        let rep_sha = &report.sha256[..report.sha256.len().min(16)];
        let pin_sha = &pinned[..pinned.len().min(16)];
        let detail = format!(
            "launcher {} sha256 {rep_sha}... is not the official build (Guard {chan} pins {pin_sha}...; served version {})",
            report.version,
            served.as_deref().unwrap_or("none")
        );
        let _ = store
            .account_write(&target, &ActorContext::new(Actor::System), move |tx, repos| {
                let flag = ActivityFlagRecord {
                    id: format!("UNOFFICIAL_LAUNCHER:{user}:{day}"),
                    kind: "UNOFFICIAL_LAUNCHER".into(),
                    severity: "high".into(),
                    account: target_in,
                    day,
                    detail,
                    acknowledged: false,
                    created_at: now_iso,
                    acknowledged_at: None,
                    acknowledged_by: None,
                    note: None,
                    ttl_at: ttl,
                };
                let _ = repos.activity.insert_flag_if_absent(tx, &flag)?;
                Ok(())
            })
            .await;
    });
}

fn add_inc(into: &mut Vec<(String, f64)>, path: String, amount: f64) {
    if !amount.is_finite() || amount <= 0.0 {
        return;
    }
    if let Some(existing) = into.iter_mut().find(|(p, _)| *p == path) {
        existing.1 += amount;
    } else {
        into.push((path, amount));
    }
}

pub fn inventory_increments(
    transaction: &crate::areas::inventory::ports::CommittedTransaction,
) -> Option<(Vec<(String, f64)>, u64)> {
    if is_ignored_activity_source(Some(&transaction.source)) {
        return None;
    }
    let mut increments = Vec::new();
    let mut kill_tokens = 0u64;
    let mut item_total = 0.0;
    let mut granted_item_keys = Vec::new();

    for item in &transaction.granted {
        let qty = service_kit::js::number(item.get("quantity"));
        if !qty.is_finite() || qty <= 0.0 {
            continue;
        }
        let catalog_id = item.get("catalogId").and_then(Value::as_str).unwrap_or_default();
        if let Some(kill_name) = kill_token_name(catalog_id) {
            add_inc(&mut increments, format!("kills.{kill_name}"), qty);
            kill_tokens = kill_tokens.saturating_add(qty as u64);
        } else {
            item_total += qty;
            let key = safe_key(catalog_id);
            if granted_item_keys.len() < ACTIVITY_MAX_ITEMS_PER_TRANSACTION && !granted_item_keys.contains(&key) {
                granted_item_keys.push(key);
            }
        }
    }
    if item_total > 0.0 {
        let source_key = safe_key(&transaction.source);
        add_inc(&mut increments, format!("itemsGained.{source_key}"), item_total);
    }
    for entry in &transaction.currency_credits {
        let Some(id) = entry.get("catalogId").and_then(Value::as_str) else {
            continue;
        };
        let amount = service_kit::js::number(entry.get("amount"));
        add_inc(&mut increments, format!("currency.{}.credited", safe_key(id)), amount);
    }
    let mut plat_debited = 0.0;
    for entry in &transaction.currency_debits {
        let Some(id) = entry.get("catalogId").and_then(Value::as_str) else {
            continue;
        };
        let amount = service_kit::js::number(entry.get("amount"));
        if !amount.is_finite() || amount <= 0.0 {
            continue;
        }
        add_inc(&mut increments, format!("currency.{}.debited", safe_key(id)), amount);
        if is_platinum_movement(id) {
            plat_debited += amount;
        }
    }
    if plat_debited > 0.0 {
        for key in granted_item_keys {
            add_inc(&mut increments, format!("platinumSpendsByItem.{key}"), plat_debited);
        }
    }
    Some((increments, kill_tokens))
}

pub fn record_inventory_transaction(
    shared: &crate::http::Shared,
    transaction: &crate::areas::inventory::ports::CommittedTransaction,
) {
    let Some(store) = shared.store.clone() else {
        return;
    };
    let settings = shared.settings.clone();
    if !read_activity_settings(&settings).enabled {
        return;
    }
    let Some((increments, kill_tokens)) = inventory_increments(transaction) else {
        return;
    };
    let account = transaction.account.clone();
    let user_id = transaction.user_id.clone();
    let source = transaction.source.clone();
    let tx_id = transaction.transaction_id.clone();
    let now_ms = util::now_ms();
    spawn_activity(async move {
        if !increments.is_empty() {
            record_daily_increments(&store, &settings, &account, now_ms, increments).await;
        }
        if kill_tokens > 0 && source == "INVENTORY_GAMESERVER" {
            let at_iso = util::iso(now_ms);
            let not_before_iso = util::iso(now_ms - ACTIVITY_HUNT_MAX_MS);
            let grace_floor_iso = util::iso(now_ms - ACTIVITY_HUNT_GRACE_MS);
            let day = day_of(now_ms);
            let ttl = activity_ttl_at(&day);
            let target = account.clone();
            let target_in = target.clone();
            let _ = store
                .account_write(&target, &ActorContext::new(Actor::System), move |tx, repos| {
                    let covering = repos.activity.find_dedicated_hunts_covering(
                        tx.conn(),
                        &target_in,
                        &at_iso,
                        &not_before_iso,
                        &grace_floor_iso,
                    )?;
                    if let Some(hunt) = covering.first() {
                        repos.activity.add_hunt_kills(tx, &hunt.id, kill_tokens)?;
                    } else {
                        let flag = ActivityFlagRecord {
                            id: format!("LOOT_WITHOUT_HUNT:{user_id}:{day}"),
                            kind: "LOOT_WITHOUT_HUNT".into(),
                            severity: "high".into(),
                            account: target_in,
                            day,
                            detail: format!("transactionId={tx_id} killTokens={kill_tokens} source={source}"),
                            acknowledged: false,
                            created_at: at_iso,
                            acknowledged_at: None,
                            acknowledged_by: None,
                            note: None,
                            ttl_at: ttl,
                        };
                        let _ = repos.activity.insert_flag_if_absent(tx, &flag)?;
                    }
                    Ok(())
                })
                .await;
        }
    });
}

pub fn record_xp_grant(shared: &crate::http::Shared, user_id: &str, progression_id: &str, amount: i64) {
    if amount <= 0 || user_id.trim().is_empty() {
        return;
    }
    let Some(store) = shared.store.clone() else {
        return;
    };
    let settings = shared.settings.clone();
    if !read_activity_settings(&settings).enabled {
        return;
    }
    let user = user_id.trim().to_owned();
    let track = safe_key(progression_id);
    let now_ms = util::now_ms();
    spawn_activity(async move {
        let Ok(Some(account)) = store.read(move |conn, repos| resolve_account(conn, repos, &user)).await else {
            return;
        };
        record_daily_increments(
            &store,
            &settings,
            &account,
            now_ms,
            vec![(format!("xp.{track}"), amount as f64)],
        )
        .await;
    });
}

pub fn record_challenge_claims_activity(shared: &crate::http::Shared, user_id: &str, count: usize) {
    if count == 0 || user_id.trim().is_empty() {
        return;
    }
    let Some(store) = shared.store.clone() else {
        return;
    };
    let settings = shared.settings.clone();
    if !read_activity_settings(&settings).enabled {
        return;
    }
    let user = user_id.trim().to_owned();
    let now_ms = util::now_ms();
    spawn_activity(async move {
        let Ok(Some(account)) = store.read(move |conn, repos| resolve_account(conn, repos, &user)).await else {
            return;
        };
        record_daily_increments(
            &store,
            &settings,
            &account,
            now_ms,
            vec![("challengesClaimed".to_owned(), count as f64)],
        )
        .await;
    });
}

pub fn record_slayers_path_unlocks(shared: &crate::http::Shared, user_id: &str, node_ids: &[String], source: &str) {
    if is_ignored_activity_source(Some(source)) || node_ids.is_empty() || user_id.trim().is_empty() {
        return;
    }
    let Some(store) = shared.store.clone() else {
        return;
    };
    let settings = shared.settings.clone();
    if !read_activity_settings(&settings).enabled {
        return;
    }
    let user = user_id.trim().to_owned();
    let nodes: Vec<String> = node_ids
        .iter()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    if nodes.is_empty() {
        return;
    }
    let source_name = source.trim().to_owned();
    let now_ms = util::now_ms();
    let now_iso = util::iso(now_ms);
    spawn_activity(async move {
        let user_lookup = user.clone();
        let Ok(Some(account)) = store
            .read(move |conn, repos| resolve_account(conn, repos, &user_lookup))
            .await
        else {
            return;
        };
        let day = day_of(now_ms);
        let ttl = activity_ttl_at(&day);
        let target = account.clone();
        let target_in = target.clone();
        let Ok(inserted) = store
            .account_write(&target, &ActorContext::new(Actor::System), move |tx, repos| {
                let mut count = 0usize;
                for node_id in &nodes {
                    let record = SlayersPathUnlockRecord {
                        id: format!("{user}:{node_id}"),
                        account: target_in.clone(),
                        node_id: node_id.clone(),
                        unlocked_at: now_iso.clone(),
                        source: source_name.clone(),
                        ttl_at: ttl,
                    };
                    if repos.activity.insert_unlock_if_absent(tx, &record)? {
                        count += 1;
                    }
                }
                Ok(count)
            })
            .await
        else {
            return;
        };
        if inserted > 0 {
            record_daily_increments(
                &store,
                &settings,
                &account,
                now_ms,
                vec![("slayersPathUnlocks".to_owned(), inserted as f64)],
            )
            .await;
        }
    });
}

pub fn record_reforge(shared: &crate::http::Shared, user_id: &str) {
    if user_id.trim().is_empty() {
        return;
    }
    let Some(store) = shared.store.clone() else {
        return;
    };
    let settings = shared.settings.clone();
    if !read_activity_settings(&settings).enabled {
        return;
    }
    let user = user_id.trim().to_owned();
    let now_ms = util::now_ms();
    spawn_activity(async move {
        let Ok(Some(account)) = store.read(move |conn, repos| resolve_account(conn, repos, &user)).await else {
            return;
        };
        record_daily_increments(&store, &settings, &account, now_ms, vec![("reforges".to_owned(), 1.0)]).await;
    });
}

pub fn classify_dedicated_end_reason(end_reason: &str) -> HuntResult {
    if end_reason.trim().eq_ignore_ascii_case("ALL_GONE") {
        HuntResult::Completed
    } else {
        HuntResult::Abandoned
    }
}

pub struct StoreHuntActivity {
    pub store: IslesStore,
    pub settings: Settings,
}

impl crate::areas::matchmaking::state::HuntActivity for StoreHuntActivity {
    fn dedicated_hunt_starts(&self, users: &[String], hunt_id: &str, hunt_ref: &str) {
        if !read_activity_settings(&self.settings).enabled || users.is_empty() {
            return;
        }
        let store = self.store.clone();
        let settings = self.settings.clone();
        let mut unique_users = Vec::new();
        for u in users {
            let trimmed = u.trim().to_owned();
            if !trimmed.is_empty() && !unique_users.contains(&trimmed) {
                unique_users.push(trimmed);
            }
        }
        let hunt_id = hunt_id.trim().to_owned();
        let hunt_ref = hunt_ref.trim().to_owned();
        let now_ms = util::now_ms();
        let now_iso = util::iso(now_ms);
        spawn_activity(async move {
            let day = day_of(now_ms);
            let ttl = activity_ttl_at(&day);
            for user in unique_users {
                let user_lookup = user.clone();
                let Ok(Some(account)) = store
                    .read(move |conn, repos| resolve_account(conn, repos, &user_lookup))
                    .await
                else {
                    continue;
                };
                let target = account.clone();
                let target_in = target.clone();
                let hid = hunt_id.clone();
                let href = hunt_ref.clone();
                let started = now_iso.clone();
                let Ok((inserted, closed)) = store
                    .account_write(&target, &ActorContext::new(Actor::System), move |tx, repos| {
                        let open = repos.activity.find_open_dedicated_hunts(tx.conn(), &target_in)?;
                        let mut closed = 0usize;
                        for prev in open {
                            if repos.activity.close_hunt_if_open(
                                tx,
                                &prev.id,
                                &started,
                                "SUPERSEDED",
                                HuntResult::Abandoned,
                            )? {
                                closed += 1;
                            }
                        }
                        let record = ActivityHuntRecord {
                            id: format!("dedicated:{href}:{user}"),
                            account: target_in,
                            host: false,
                            kind: HuntKind::Dedicated,
                            hunt_id: hid,
                            hunt_ref: href,
                            started_at: started,
                            ended_at: None,
                            end_reason: None,
                            result: None,
                            kills: 0,
                            ttl_at: ttl,
                        };
                        let inserted = repos.activity.insert_hunt_if_absent(tx, &record)?;
                        Ok((inserted, closed))
                    })
                    .await
                else {
                    continue;
                };
                let mut increments = Vec::new();
                if inserted {
                    increments.push(("huntsStarted.dedicated".to_owned(), 1.0));
                }
                if closed > 0 {
                    increments.push(("huntsAbandoned.dedicated".to_owned(), closed as f64));
                }
                if !increments.is_empty() {
                    record_daily_increments(&store, &settings, &account, now_ms, increments).await;
                }
            }
        });
    }

    fn dedicated_hunt_ends(&self, users: &[String], end_reason: &str) {
        if !read_activity_settings(&self.settings).enabled || users.is_empty() {
            return;
        }
        let store = self.store.clone();
        let settings = self.settings.clone();
        let mut unique_users = Vec::new();
        for u in users {
            let trimmed = u.trim().to_owned();
            if !trimmed.is_empty() && !unique_users.contains(&trimmed) {
                unique_users.push(trimmed);
            }
        }
        let reason = end_reason.trim().to_owned();
        let result = classify_dedicated_end_reason(&reason);
        let now_ms = util::now_ms();
        let now_iso = util::iso(now_ms);
        spawn_activity(async move {
            for user in unique_users {
                let Ok(Some(account)) = store.read(move |conn, repos| resolve_account(conn, repos, &user)).await else {
                    continue;
                };
                let target = account.clone();
                let target_in = target.clone();
                let ended = now_iso.clone();
                let reason_in = reason.clone();
                let Ok(closed) = store
                    .account_write(&target, &ActorContext::new(Actor::System), move |tx, repos| {
                        let open = repos.activity.find_open_dedicated_hunts(tx.conn(), &target_in)?;
                        let Some(latest) = open.first() else {
                            return Ok(false);
                        };
                        repos
                            .activity
                            .close_hunt_if_open(tx, &latest.id, &ended, &reason_in, result)
                    })
                    .await
                else {
                    continue;
                };
                if closed {
                    let metric = match result {
                        HuntResult::Completed => "huntsCompleted.dedicated",
                        HuntResult::Abandoned => "huntsAbandoned.dedicated",
                    };
                    record_daily_increments(&store, &settings, &account, now_ms, vec![(metric.to_owned(), 1.0)]).await;
                }
            }
        });
    }
}
