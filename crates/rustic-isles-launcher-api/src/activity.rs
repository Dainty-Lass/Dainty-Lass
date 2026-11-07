// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The activity monitor (public spec §10): the recorders other areas call, the daily document and its flags, and
//! the admin's reads.

use std::future::Future;
use std::sync::{Arc, OnceLock, Weak};

use axum::response::Response;
use http::StatusCode;
use module_api::{Actor, ActorContext};
use rustic_isles::{Settings, Shared, UserId};
use rustic_isles_economy::inventory::CommittedTransaction;
use rustic_isles_store::repo::reforged::{
    ActivityFlagRecord, ActivityHuntRecord, AdminAuditRecord, HuntKind, HuntResult, SlayersPathUnlockRecord,
};
use rustic_isles_store::{AccountId, AnyConnection, IslesStore, RepoResult, Repositories, Tx};
use serde_json::{Map, Value};

use crate::admin::{AdminContext, admin_actor, admin_error, fresh_id, limit_of, page_of, skip_of, user_id_of};
use crate::state::LauncherApiState;
use crate::util::{self, DAY_MS, HOUR_MS, MINUTE_MS, iso, now_ms, object};

const RETENTION_DAYS: i64 = 400;
const MAX_GRANTED_KEYS: usize = 8;

// ------------------------------------------------------------------ settings (§10.1)

/// The activity monitor's switch and flag thresholds, read per call.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActivitySettings {
    /// Whether recording is on (`RUSTIC_ISLES_ACTIVITY_MONITOR` not off).
    pub enabled: bool,
    /// Kills per hour that raise a flag (`RUSTIC_ISLES_KILLS_PER_HOUR_FLAG`, default 90).
    pub kills_per_hour: f64,
    /// Rewards per hour that raise a flag (`RUSTIC_ISLES_REWARDS_PER_HOUR_FLAG`, default 600).
    pub rewards_per_hour: f64,
    /// Platinum credited in a day that raises a flag (`RUSTIC_ISLES_PLAT_DAILY_FLAG`, default 5000).
    pub platinum_credit_day: f64,
    /// Platinum spent in a day that raises a flag (`RUSTIC_ISLES_PLAT_SPEND_FLAG`, default 10000).
    pub platinum_spend_day: f64,
    /// Seasonal coins in a day that raise a flag (`RUSTIC_ISLES_SEASONAL_COIN_DAILY_FLAG`, default 20000).
    pub seasonal_coin_day: f64,
    /// XP per minute that raises a flag (`RUSTIC_ISLES_XP_PER_MINUTE_FLAG`, default 2500).
    pub xp_per_minute: f64,
}

fn threshold(settings: &Settings, name: &str, default: f64) -> f64 {
    let text = settings.raw(name).unwrap_or("").trim();
    if text.is_empty() {
        return default;
    }
    text.parse::<f64>()
        .ok()
        .filter(|n| n.is_finite() && *n >= 0.0)
        .unwrap_or(default)
}

impl ActivitySettings {
    /// Reads the settings; a threshold that is not a finite number of 0 or more takes its default.
    pub fn read(settings: &Settings) -> Self {
        let switch = settings
            .raw("RUSTIC_ISLES_ACTIVITY_MONITOR")
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        // Read and unused, as Node (§20 item 16)
        let _expected = (
            settings.raw("RUSTIC_ISLES_EXPECTED_LAUNCHER_VERSION"),
            settings.raw("RUSTIC_ISLES_EXPECTED_LAUNCHER_BUILD_ID"),
        );
        Self {
            enabled: !matches!(switch.as_str(), "0" | "false" | "off" | "no"),
            kills_per_hour: threshold(settings, "RUSTIC_ISLES_KILLS_PER_HOUR_FLAG", 90.0),
            rewards_per_hour: threshold(settings, "RUSTIC_ISLES_REWARDS_PER_HOUR_FLAG", 600.0),
            platinum_credit_day: threshold(settings, "RUSTIC_ISLES_PLAT_DAILY_FLAG", 5000.0),
            platinum_spend_day: threshold(settings, "RUSTIC_ISLES_PLAT_SPEND_FLAG", 10000.0),
            seasonal_coin_day: threshold(settings, "RUSTIC_ISLES_SEASONAL_COIN_DAILY_FLAG", 20000.0),
            xp_per_minute: threshold(settings, "RUSTIC_ISLES_XP_PER_MINUTE_FLAG", 2500.0),
        }
    }
}

// ------------------------------------------------------------------ days and keys (§10.2)

/// The first 10 characters of the time's ISO text.
pub fn day_of(ms: i64) -> String {
    iso(ms)[..10].to_owned()
}

fn midnight(day: &str) -> Option<i64> {
    let b = day.as_bytes();
    let shape = b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && [0, 1, 2, 3, 5, 6, 8, 9].iter().all(|i| b[*i].is_ascii_digit());
    if !shape {
        return None;
    }
    let at = service_kit::js::date_parse(&format!("{day}T00:00:00.000Z"))?;
    (day_of(at) == day).then_some(at)
}

/// Whether the text is a real `YYYY-MM-DD` day.
pub fn is_valid_day(day: &str) -> bool {
    midnight(day).is_some()
}

/// The day's midnight plus the 400-day retention.
pub fn ttl_of_day(day: &str) -> i64 {
    midnight(day).unwrap_or(0) + RETENTION_DAYS * DAY_MS
}

/// 1 to 80 bytes of ASCII letters, digits and `_`; anything else is `OTHER`.
pub fn safe_key(text: &str) -> String {
    let ok = (1..=80).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    if ok { text.to_owned() } else { "OTHER".to_owned() }
}

fn num_at(doc: &Map<String, Value>, path: &[&str]) -> f64 {
    let mut current = doc.get(path[0]);
    for key in &path[1..] {
        current = current.and_then(|v| v.get(*key));
    }
    current.and_then(Value::as_f64).filter(|n| n.is_finite()).unwrap_or(0.0)
}

fn group_sum(doc: &Map<String, Value>, group: &str) -> f64 {
    doc.get(group)
        .and_then(Value::as_object)
        .map(|m| m.values().filter_map(Value::as_f64).filter(|n| n.is_finite()).sum())
        .unwrap_or(0.0)
}

fn platinum_sum(doc: &Map<String, Value>, side: &str) -> f64 {
    doc.get("currency")
        .and_then(Value::as_object)
        .map(|currencies| {
            currencies
                .iter()
                .filter(|(id, _)| is_platinum(id))
                .filter_map(|(_, entry)| entry.get(side).and_then(Value::as_f64))
                .filter(|n| n.is_finite())
                .sum()
        })
        .unwrap_or(0.0)
}

fn is_platinum(id: &str) -> bool {
    id == "CURRENCY_PLATINUM" || id.starts_with("CURRENCY_PLATINUM_")
}

/// The daily flags a document raises: `(kind, detail)` in §10.2's order.
pub fn daily_flag_candidates(doc: &Map<String, Value>, settings: &ActivitySettings) -> Vec<(&'static str, String)> {
    let mut flags = Vec::new();
    let credited = platinum_sum(doc, "credited");
    if credited > settings.platinum_credit_day {
        flags.push((
            "PLATINUM_CREDIT_DAY",
            format!("credited={credited} threshold={}", settings.platinum_credit_day),
        ));
    }
    let debited = platinum_sum(doc, "debited");
    if debited > settings.platinum_spend_day {
        flags.push((
            "PLATINUM_SPEND_DAY",
            format!("debited={debited} threshold={}", settings.platinum_spend_day),
        ));
    }
    let coins = num_at(doc, &["currency", "CURRENCY_SEASONAL_COIN", "credited"]);
    if coins > settings.seasonal_coin_day {
        flags.push((
            "SEASONAL_COIN_DAY",
            format!("credited={coins} threshold={}", settings.seasonal_coin_day),
        ));
    }
    let playtime = num_at(doc, &["playtimeMinutes"]);
    if playtime >= 15.0 {
        let xp = group_sum(doc, "xp");
        let per_minute = xp / playtime;
        if per_minute > settings.xp_per_minute {
            flags.push((
                "XP_PER_MINUTE",
                format!(
                    "xp={xp} playtimeMinutes={playtime} perMinute={per_minute:.1} threshold={}",
                    settings.xp_per_minute
                ),
            ));
        }
        let hours = playtime / 60.0;
        let kills = group_sum(doc, "kills");
        if kills / hours > settings.kills_per_hour {
            flags.push((
                "KILLS_PER_HOUR",
                format!(
                    "kills={kills} playtimeMinutes={playtime} threshold={}",
                    settings.kills_per_hour
                ),
            ));
        }
        let items = group_sum(doc, "itemsGained");
        if items / hours > settings.rewards_per_hour {
            flags.push((
                "REWARDS_PER_HOUR",
                format!(
                    "items={items} playtimeMinutes={playtime} threshold={}",
                    settings.rewards_per_hour
                ),
            ));
        }
    }
    flags
}

/// The playtime step: `(minutes, sessions, seen)`, or none when the clock went backwards.
pub fn advance_playtime(previous: Option<i64>, now: i64) -> Option<(f64, u32, i64)> {
    let Some(previous) = previous.filter(|p| *p > 0) else {
        return Some((0.0, 1, now));
    };
    let gap = now - previous;
    if gap <= 0 {
        return None;
    }
    if gap > 3 * MINUTE_MS {
        return Some((0.0, 1, now));
    }
    #[allow(clippy::cast_precision_loss, reason = "a gap in ms, far below 2^53")]
    Some((gap as f64 / MINUTE_MS as f64, 0, now))
}

/// `completed` for `ALL_GONE` (trimmed, any ASCII case), else `abandoned`.
pub fn classify_dedicated_end_reason(reason: &str) -> HuntResult {
    if util::trim(reason).eq_ignore_ascii_case("ALL_GONE") {
        HuntResult::Completed
    } else {
        HuntResult::Abandoned
    }
}

/// §10.3's transaction fields.
#[derive(Debug, Clone, PartialEq)]
pub struct ActivityInput {
    /// The ledger's transaction id.
    pub transaction_id: String,
    /// What made the transaction, such as `OPERATOR_TEST`.
    pub source: String,
    /// Items granted, `(catalog id, amount)`.
    pub granted: Vec<(String, f64)>,
    /// Currencies credited, `(currency, amount)`.
    pub credits: Vec<(String, f64)>,
    /// Currencies debited, `(currency, amount)`.
    pub debits: Vec<(String, f64)>,
}

impl From<&CommittedTransaction> for ActivityInput {
    fn from(t: &CommittedTransaction) -> Self {
        Self {
            transaction_id: t.transaction_id.clone(),
            source: t.source.clone(),
            granted: t.granted.clone(),
            credits: t.currency_credits.clone(),
            debits: t.currency_debits.clone(),
        }
    }
}

fn add(increments: &mut Vec<(String, f64)>, path: String, amount: f64) {
    match increments.iter_mut().find(|(p, _)| *p == path) {
        Some((_, total)) => *total += amount,
        None => increments.push((path, amount)),
    }
}

/// The daily increments of a transaction and its kill tokens.
pub fn inventory_increments(input: &ActivityInput) -> (Vec<(String, f64)>, u64) {
    let mut increments = Vec::new();
    if input.source == "OPERATOR_TEST" {
        return (increments, 0);
    }
    let mut tokens: u64 = 0;
    let mut items = 0.0;
    let mut keys: Vec<String> = Vec::new();
    for (catalog, quantity) in &input.granted {
        if !quantity.is_finite() || *quantity <= 0.0 {
            continue;
        }
        match catalog.strip_prefix("TOKEN_KILL_COUNT_").filter(|r| !r.is_empty()) {
            Some(rest) => {
                add(&mut increments, format!("kills.{}", safe_key(rest)), *quantity);
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "positive and finite; truncation is the rule"
                )]
                let whole = quantity.trunc() as u64;
                tokens = tokens.saturating_add(whole);
            }
            None => {
                items += quantity;
                let key = safe_key(catalog);
                if !keys.contains(&key) && keys.len() < MAX_GRANTED_KEYS {
                    keys.push(key);
                }
            }
        }
    }
    if items > 0.0 {
        add(
            &mut increments,
            format!("itemsGained.{}", safe_key(&input.source)),
            items,
        );
    }
    for (id, amount) in &input.credits {
        if amount.is_finite() && *amount > 0.0 {
            add(&mut increments, format!("currency.{}.credited", safe_key(id)), *amount);
        }
    }
    let mut platinum = 0.0;
    for (id, amount) in &input.debits {
        if amount.is_finite() && *amount > 0.0 {
            add(&mut increments, format!("currency.{}.debited", safe_key(id)), *amount);
            if is_platinum(id) {
                platinum += amount;
            }
        }
    }
    if platinum > 0.0 {
        for key in &keys {
            add(&mut increments, format!("platinumSpendsByItem.{key}"), platinum);
        }
    }
    (increments, tokens)
}

// ------------------------------------------------------------------ recording (§10.2, §10.3)

fn system() -> ActorContext {
    ActorContext::new(Actor::System)
}

/// The account a user text names and its userId: a game account's userId, else an account id.
fn resolve(conn: &mut AnyConnection, repos: &Repositories, user: &str) -> RepoResult<Option<(AccountId, String)>> {
    if let Some(game) = repos.accounts.find_by_user_id(conn, &UserId::new(user))? {
        return Ok(Some((game.id, game.user_id.as_str().to_owned())));
    }
    match user.parse::<AccountId>() {
        Ok(account) => {
            let user_id = user_id_of(conn, repos, &account);
            Ok(Some((account, user_id)))
        }
        Err(_) => Ok(None),
    }
}

async fn resolve_user(store: &IslesStore, user: &str) -> Option<(AccountId, String)> {
    let user = user.to_owned();
    store
        .read(move |conn, repos| resolve(conn, repos, &user))
        .await
        .ok()
        .flatten()
}

/// Adds the increments to the day's document and inserts the flags it raises, in the caller's write.
fn apply_increments(
    tx: &mut Tx<'_>,
    repos: &Repositories,
    settings: &ActivitySettings,
    account: &AccountId,
    user_id: &str,
    at_ms: i64,
    increments: &[(String, f64)],
) -> RepoResult<()> {
    if increments.is_empty() {
        return Ok(());
    }
    let day = day_of(at_ms);
    let ttl = ttl_of_day(&day);
    let doc = repos.activity.add_to_daily(tx, account, &day, increments, ttl)?;
    let empty = Map::new();
    for (kind, detail) in daily_flag_candidates(doc.as_object().unwrap_or(&empty), settings) {
        repos
            .activity
            .insert_flag_if_absent(tx, &flag(kind, "review", account, user_id, &day, detail, at_ms))?;
    }
    Ok(())
}

fn flag(
    kind: &str,
    severity: &str,
    account: &AccountId,
    user_id: &str,
    day: &str,
    detail: String,
    at: i64,
) -> ActivityFlagRecord {
    ActivityFlagRecord {
        id: format!("{kind}:{user_id}:{day}"),
        kind: kind.to_owned(),
        severity: severity.to_owned(),
        account: account.clone(),
        day: day.to_owned(),
        detail: Value::from(detail),
        acknowledged: false,
        created_at: iso(at),
        acknowledged_at: None,
        acknowledged_by: None,
        note: None,
        ttl_at: ttl_of_day(day),
    }
}

/// What a recorder needs: the store and the monitor's settings, when both are there.
fn ready(shared: &Shared) -> Option<(Arc<IslesStore>, ActivitySettings)> {
    let settings = ActivitySettings::read(shared.settings());
    if !settings.enabled {
        return None;
    }
    Some((util::store_of(shared)?, settings))
}

fn spawn(work: impl Future<Output = ()> + Send + 'static) {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(work);
    }
}

async fn record(store: &IslesStore, settings: ActivitySettings, user: &str, at: i64, increments: Vec<(String, f64)>) {
    let Some((account, user_id)) = resolve_user(store, user).await else {
        return;
    };
    let target = account.clone();
    let _ = store
        .account_write(&target, &system(), move |tx, repos| {
            apply_increments(tx, repos, &settings, &account, &user_id, at, &increments)
        })
        .await;
}

/// A game session touched: always noted as recently active; with a store and the monitor on, the playtime step.
pub fn touch_game_session(shared: &Arc<Shared>, user: &str, at_ms: i64) {
    LauncherApiState::of(shared).touch(user, at_ms);
    let Some((store, settings)) = ready(shared) else {
        return;
    };
    let user = user.to_owned();
    spawn(async move {
        let Some((account, user_id)) = resolve_user(&store, &user).await else {
            return;
        };
        let target = account.clone();
        let _ = store
            .account_write(&target, &system(), move |tx, repos| {
                let day = day_of(at_ms);
                let rows = repos.activity.find_daily_rows(tx.conn(), &account, &day, &day, 1)?;
                let previous = rows.first().and_then(|d| d.get("lastSeenMs")).and_then(Value::as_i64);
                let Some((minutes, sessions, seen)) = advance_playtime(previous, at_ms) else {
                    return Ok(());
                };
                let fields = object(vec![("lastSeenMs", Value::from(seen))]);
                repos
                    .activity
                    .set_daily_fields(tx, &account, &day, &fields, ttl_of_day(&day))?;
                let mut increments = Vec::new();
                if minutes != 0.0 {
                    increments.push(("playtimeMinutes".to_owned(), minutes));
                }
                if sessions != 0 {
                    increments.push(("sessions".to_owned(), f64::from(sessions)));
                }
                apply_increments(tx, repos, &settings, &account, &user_id, at_ms, &increments)
            })
            .await;
    });
}

/// Records a committed inventory transaction in the day's document, in the background; operator test grants are not
/// counted.
pub fn record_inventory_transaction(shared: &Arc<Shared>, transaction: &CommittedTransaction) {
    let input = ActivityInput::from(transaction);
    let (increments, tokens) = inventory_increments(&input);
    if increments.is_empty() && tokens == 0 {
        return;
    }
    let Some((store, settings)) = ready(shared) else {
        return;
    };
    let at = now_ms();
    let user = transaction.user_id.clone();
    spawn(async move {
        record(&store, settings, &user, at, increments).await;
        if tokens == 0 || input.source != "INVENTORY_GAMESERVER" {
            return;
        }
        let Some((account, user_id)) = resolve_user(&store, &user).await else {
            return;
        };
        let target = account.clone();
        let _ = store
            .account_write(&target, &system(), move |tx, repos| {
                let hunts = repos.activity.find_dedicated_hunts_covering(
                    tx.conn(),
                    &account,
                    &iso(at),
                    &iso(at - 6 * HOUR_MS),
                    &iso(at - 10 * MINUTE_MS),
                )?;
                match hunts.first() {
                    Some(hunt) => repos.activity.add_hunt_kills(tx, &hunt.id, tokens),
                    None => {
                        let detail = format!(
                            "transactionId={} killTokens={tokens} source={}",
                            input.transaction_id, input.source
                        );
                        let day = day_of(at);
                        repos
                            .activity
                            .insert_flag_if_absent(
                                tx,
                                &flag("LOOT_WITHOUT_HUNT", "high", &account, &user_id, &day, detail, at),
                            )
                            .map(|_| ())
                    }
                }
            })
            .await;
    });
}

/// Records an XP grant on a progression track, in the background.
pub fn record_xp_grant(shared: &Arc<Shared>, user: &str, progression_id: &str, amount: f64) {
    if !(amount.is_finite() && amount > 0.0) || util::trim(user).is_empty() {
        return;
    }
    let Some((store, settings)) = ready(shared) else {
        return;
    };
    let (user, at) = (user.to_owned(), now_ms());
    let increments = vec![(format!("xp.{}", safe_key(progression_id)), amount)];
    spawn(async move { record(&store, settings, &user, at, increments).await });
}

/// Records claimed challenges, in the background.
pub fn record_challenge_claims(shared: &Arc<Shared>, user: &str, count: usize) {
    if count == 0 {
        return;
    }
    let Some((store, settings)) = ready(shared) else {
        return;
    };
    let (user, at) = (user.to_owned(), now_ms());
    #[allow(clippy::cast_precision_loss, reason = "a claim count, far below 2^53")]
    let increments = vec![("challengesClaimed".to_owned(), count as f64)];
    spawn(async move { record(&store, settings, &user, at, increments).await });
}

/// Records one reforge, in the background.
pub fn record_reforge(shared: &Arc<Shared>, user: &str) {
    let Some((store, settings)) = ready(shared) else {
        return;
    };
    let (user, at) = (user.to_owned(), now_ms());
    spawn(async move { record(&store, settings, &user, at, vec![("reforges".to_owned(), 1.0)]).await });
}

/// Records Slayer's Path node unlocks, in the background; operator test unlocks are not counted.
pub fn record_slayers_path_unlocks(shared: &Arc<Shared>, user: &str, nodes: &[String], source: &str) {
    if source == "OPERATOR_TEST" {
        return;
    }
    let nodes: Vec<String> = nodes
        .iter()
        .map(|n| util::trim(n).to_owned())
        .filter(|n| !n.is_empty())
        .collect();
    if nodes.is_empty() {
        return;
    }
    let Some((store, settings)) = ready(shared) else {
        return;
    };
    let (user, source, at) = (util::trim(user).to_owned(), util::trim(source).to_owned(), now_ms());
    spawn(async move {
        let Some((account, user_id)) = resolve_user(&store, &user).await else {
            return;
        };
        let target = account.clone();
        let _ = store
            .account_write(&target, &system(), move |tx, repos| {
                let day = day_of(at);
                let mut inserted = 0u32;
                for node in &nodes {
                    let record = SlayersPathUnlockRecord {
                        id: format!("{user}:{node}"),
                        account: account.clone(),
                        node_id: node.clone(),
                        unlocked_at: iso(at),
                        source: source.clone(),
                        ttl_at: ttl_of_day(&day),
                    };
                    if repos.activity.insert_unlock_if_absent(tx, &record)? {
                        inserted += 1;
                    }
                }
                if inserted == 0 {
                    return Ok(());
                }
                let increments = [("slayersPathUnlocks".to_owned(), f64::from(inserted))];
                apply_increments(tx, repos, &settings, &account, &user_id, at, &increments)
            })
            .await;
    });
}

fn distinct_users(users: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for user in users.iter().map(|u| util::trim(u).to_owned()).filter(|u| !u.is_empty()) {
        if !out.contains(&user) {
            out.push(user);
        }
    }
    out
}

/// Records a dedicated hunt's start for each player, in the background.
pub fn dedicated_hunt_starts(shared: &Arc<Shared>, users: &[String], hunt_id: &str, hunt_ref: &str) {
    let users = distinct_users(users);
    let Some((store, settings)) = ready(shared) else {
        return;
    };
    let (hunt_id, hunt_ref, at) = (
        util::trim(hunt_id).to_owned(),
        util::trim(hunt_ref).to_owned(),
        now_ms(),
    );
    spawn(async move {
        for user in users {
            let Some((account, user_id)) = resolve_user(&store, &user).await else {
                continue;
            };
            let (hunt_id, hunt_ref) = (hunt_id.clone(), hunt_ref.clone());
            let target = account.clone();
            let _ = store
                .account_write(&target, &system(), move |tx, repos| {
                    let now = iso(at);
                    let mut closed = 0u32;
                    for open in repos.activity.find_open_dedicated_hunts(tx.conn(), &account)? {
                        if repos.activity.close_hunt_if_open(
                            tx,
                            &open.id,
                            &now,
                            "SUPERSEDED",
                            Some(HuntResult::Abandoned),
                        )? {
                            closed += 1;
                        }
                    }
                    let day = day_of(at);
                    let hunt = ActivityHuntRecord {
                        id: format!("dedicated:{hunt_ref}:{user}"),
                        account: account.clone(),
                        host: false,
                        kind: HuntKind::Dedicated,
                        hunt_id,
                        hunt_ref,
                        started_at: now,
                        ended_at: None,
                        end_reason: None,
                        result: None,
                        kills: 0,
                        ttl_at: ttl_of_day(&day),
                    };
                    let inserted = repos.activity.insert_hunt_if_absent(tx, &hunt)?;
                    let mut increments = Vec::new();
                    if inserted {
                        increments.push(("huntsStarted.dedicated".to_owned(), 1.0));
                    }
                    if closed > 0 {
                        increments.push(("huntsAbandoned.dedicated".to_owned(), f64::from(closed)));
                    }
                    apply_increments(tx, repos, &settings, &account, &user_id, at, &increments)
                })
                .await;
        }
    });
}

/// Records a dedicated hunt's end for each player, classed as completed or abandoned from `end_reason`, in the
/// background.
pub fn dedicated_hunt_ends(shared: &Arc<Shared>, users: &[String], end_reason: &str) {
    let users = distinct_users(users);
    let Some((store, settings)) = ready(shared) else {
        return;
    };
    let reason = util::trim(end_reason).to_owned();
    let result = classify_dedicated_end_reason(&reason);
    let at = now_ms();
    spawn(async move {
        for user in users {
            let Some((account, user_id)) = resolve_user(&store, &user).await else {
                continue;
            };
            let reason = reason.clone();
            let target = account.clone();
            let _ = store
                .account_write(&target, &system(), move |tx, repos| {
                    let open = repos.activity.find_open_dedicated_hunts(tx.conn(), &account)?;
                    let Some(latest) = open.first() else {
                        return Ok(());
                    };
                    if !repos
                        .activity
                        .close_hunt_if_open(tx, &latest.id, &iso(at), &reason, Some(result))?
                    {
                        return Ok(());
                    }
                    let path = match result {
                        HuntResult::Completed => "huntsCompleted.dedicated",
                        HuntResult::Abandoned => "huntsAbandoned.dedicated",
                    };
                    apply_increments(tx, repos, &settings, &account, &user_id, at, &[(path.to_owned(), 1.0)])
                })
                .await;
        }
    });
}

/// An unofficial launcher build against Guard's pin (never reached while the pin lookup finds none, §10.4).
pub fn record_unofficial_launcher(
    shared: &Arc<Shared>,
    user: &str,
    report: &rustic_isles_auth::LauncherReport,
    pinned: &str,
    served: Option<&str>,
) {
    let Some((store, _)) = ready(shared) else {
        return;
    };
    let head = |text: &str| text.chars().take(16).collect::<String>();
    let detail = format!(
        "launcher {} sha256 {}... is not the official build (Guard stable pins {}...; served version {})",
        report.version,
        head(&report.sha256),
        head(pinned),
        served.unwrap_or("none")
    );
    let (user, at) = (user.to_owned(), now_ms());
    spawn(async move {
        let Some((account, user_id)) = resolve_user(&store, &user).await else {
            return;
        };
        let target = account.clone();
        let _ = store
            .account_write(&target, &system(), move |tx, repos| {
                let day = day_of(at);
                let record = flag("UNOFFICIAL_LAUNCHER", "high", &account, &user_id, &day, detail, at);
                repos.activity.insert_flag_if_absent(tx, &record).map(|_| ())
            })
            .await;
    });
}

// ------------------------------------------------------------------ the economy's ports

/// The economy's activity ports over this module's recorders; bound to the module once it is built.
#[derive(Default)]
pub struct ActivityHooks {
    shared: OnceLock<Weak<Shared>>,
}

impl std::fmt::Debug for ActivityHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActivityHooks")
            .field("bound", &self.shared.get().is_some())
            .finish()
    }
}

impl ActivityHooks {
    /// Points the hooks at the module; only the first call counts.
    pub fn bind(&self, shared: &Arc<Shared>) {
        let _ = self.shared.set(Arc::downgrade(shared));
    }

    fn shared(&self) -> Option<Arc<Shared>> {
        self.shared.get()?.upgrade()
    }
}

impl rustic_isles_economy::inventory::InventoryActivity for ActivityHooks {
    fn record_inventory_transaction(&self, transaction: &CommittedTransaction) {
        if let Some(shared) = self.shared() {
            record_inventory_transaction(&shared, transaction);
        }
    }

    fn record_reforge(&self, account: &AccountId, _at_ms: i64) {
        if let Some(shared) = self.shared() {
            record_reforge(&shared, &account.to_string());
        }
    }
}

impl rustic_isles_economy::progression::Activity for ActivityHooks {
    fn xp(&self, user_id: &str, track: &str, amount: f64) {
        if let Some(shared) = self.shared() {
            record_xp_grant(&shared, user_id, track, amount);
        }
    }

    fn slayers_path_unlocks(&self, user_id: &str, node_keys: &[String], source: &str) {
        if let Some(shared) = self.shared() {
            record_slayers_path_unlocks(&shared, user_id, node_keys, source);
        }
    }
}

impl rustic_isles_economy::bounties::ClaimActivity for ActivityHooks {
    fn challenges_claimed(&self, user_id: &str, count: usize) {
        if let Some(shared) = self.shared() {
            record_challenge_claims(&shared, user_id, count);
        }
    }
}

impl rustic_isles_matchmaking::HuntActivity for ActivityHooks {
    fn dedicated_hunt_starts(&self, users: &[String], hunt_id: &str, hunt_ref: &str) {
        if let Some(shared) = self.shared() {
            dedicated_hunt_starts(&shared, users, hunt_id, hunt_ref);
        }
    }

    fn dedicated_hunt_ends(&self, users: &[String], end_reason: &str) {
        if let Some(shared) = self.shared() {
            dedicated_hunt_ends(&shared, users, end_reason);
        }
    }
}

// ------------------------------------------------------------------ the admin's reads (§10.5)

fn range_error(code: &str, message: &str) -> Response {
    admin_error(StatusCode::BAD_REQUEST, code, message)
}

/// `(from, to)` from the query, or the 400.
pub fn read_range(query: &[(String, String)], today: &str) -> Result<(String, String), Response> {
    let invalid = || range_error("ACTIVITY_RANGE_INVALID", "from and to must be UTC days, YYYY-MM-DD.");
    let value = |name: &str| {
        util::single(query, name)
            .map(|v| util::trim(v).to_owned())
            .unwrap_or_default()
    };
    let to = Some(value("to"))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| today.to_owned());
    let from = match Some(value("from")).filter(|f| !f.is_empty()) {
        Some(from) => from,
        None => day_of(midnight(&to).ok_or_else(invalid)? - 29 * DAY_MS),
    };
    let (Some(start), Some(end)) = (midnight(&from), midnight(&to)) else {
        return Err(invalid());
    };
    if from > to {
        return Err(range_error("ACTIVITY_RANGE_INVALID", "from must not be after to."));
    }
    if (end - start) / DAY_MS + 1 > RETENTION_DAYS {
        return Err(range_error(
            "ACTIVITY_RANGE_TOO_LONG",
            "The range may be at most 400 days (the retention).",
        ));
    }
    Ok((from, to))
}

/// The documents' totals in §10.5's key order; whole numbers as integers.
pub fn totals<'a>(docs: impl IntoIterator<Item = &'a Map<String, Value>>) -> Value {
    const COUNTERS: [&str; 7] = [
        "playtimeMinutes",
        "sessions",
        "hostingMinutes",
        "huntsHosted",
        "challengesClaimed",
        "slayersPathUnlocks",
        "reforges",
    ];
    let mut flat = [0.0f64; 7];
    let mut hunts: [Vec<(String, f64)>; 3] = Default::default();
    for group in &mut hunts {
        *group = vec![
            ("dedicated".to_owned(), 0.0),
            ("realm".to_owned(), 0.0),
            ("playerHosted".to_owned(), 0.0),
        ];
    }
    let mut open: [Vec<(String, f64)>; 4] = Default::default();
    let mut currency: Vec<(String, f64, f64)> = Vec::new();
    let n = |v: &Value| v.as_f64().filter(|x| x.is_finite()).unwrap_or(0.0);
    let add_to = |list: &mut Vec<(String, f64)>, key: &str, amount: f64| match list.iter_mut().find(|(k, _)| k == key) {
        Some((_, total)) => *total += amount,
        None => list.push((key.to_owned(), amount)),
    };
    for doc in docs {
        for (i, name) in COUNTERS.iter().enumerate() {
            flat[i] += doc.get(*name).map_or(0.0, n);
        }
        for (i, name) in ["huntsStarted", "huntsCompleted", "huntsAbandoned"].iter().enumerate() {
            if let Some(map) = doc.get(*name).and_then(Value::as_object) {
                for (k, v) in map {
                    add_to(&mut hunts[i], k, n(v));
                }
            }
        }
        for (i, name) in ["kills", "xp", "itemsGained", "platinumSpendsByItem"]
            .iter()
            .enumerate()
        {
            if let Some(map) = doc.get(*name).and_then(Value::as_object) {
                for (k, v) in map {
                    add_to(&mut open[i], k, n(v));
                }
            }
        }
        if let Some(map) = doc.get("currency").and_then(Value::as_object) {
            for (id, entry) in map {
                let credited = entry.get("credited").map_or(0.0, n);
                let debited = entry.get("debited").map_or(0.0, n);
                match currency.iter_mut().find(|(k, _, _)| k == id) {
                    Some((_, c, d)) => {
                        *c += credited;
                        *d += debited;
                    }
                    None => currency.push((id.clone(), credited, debited)),
                }
            }
        }
    }
    let group = |list: &[(String, f64)]| {
        let mut map = Map::new();
        for (k, v) in list {
            map.insert(k.clone(), util::number(*v));
        }
        Value::Object(map)
    };
    let mut out = Map::new();
    for (i, name) in COUNTERS.iter().enumerate() {
        out.insert((*name).to_owned(), util::number(flat[i]));
    }
    for (i, name) in ["huntsStarted", "huntsCompleted", "huntsAbandoned"].iter().enumerate() {
        out.insert((*name).to_owned(), group(&hunts[i]));
    }
    for (i, name) in ["kills", "xp", "itemsGained", "platinumSpendsByItem"]
        .iter()
        .enumerate()
    {
        out.insert((*name).to_owned(), group(&open[i]));
    }
    let mut cur = Map::new();
    for (id, c, d) in currency {
        cur.insert(
            id,
            object(vec![("credited", util::number(c)), ("debited", util::number(d))]),
        );
    }
    out.insert("currency".into(), Value::Object(cur));
    Value::Object(out)
}

fn flag_json(conn: &mut AnyConnection, repos: &Repositories, f: &ActivityFlagRecord) -> Value {
    let by = f.acknowledged_by.as_ref().map(|text| match text.parse::<AccountId>() {
        Ok(account) => Value::from(user_id_of(conn, repos, &account)),
        Err(_) => Value::from(text.clone()),
    });
    let opt = |v: &Option<String>| v.clone().map_or(Value::Null, Value::from);
    let detail = match &f.detail {
        Value::String(text) => Value::from(text.clone()),
        other => other.clone(),
    };
    object(vec![
        ("id", Value::from(f.id.clone())),
        ("kind", Value::from(f.kind.clone())),
        ("severity", Value::from(f.severity.clone())),
        ("userId", Value::from(user_id_of(conn, repos, &f.account))),
        ("day", Value::from(f.day.clone())),
        ("detail", detail),
        (
            "status",
            Value::from(if f.acknowledged { "acknowledged" } else { "open" }),
        ),
        ("createdAt", Value::from(f.created_at.clone())),
        ("acknowledgedAt", opt(&f.acknowledged_at)),
        ("acknowledgedBy", by.unwrap_or(Value::Null)),
        ("note", opt(&f.note)),
    ])
}

fn hunt_json(h: &ActivityHuntRecord) -> Value {
    let opt = |v: &Option<String>| v.clone().map_or(Value::Null, Value::from);
    object(vec![
        ("role", Value::from(if h.host { "host" } else { "participant" })),
        ("kind", Value::from(h.kind.as_str())),
        ("huntId", Value::from(h.hunt_id.clone())),
        ("huntRef", Value::from(h.hunt_ref.clone())),
        ("startedAt", Value::from(h.started_at.clone())),
        ("endedAt", opt(&h.ended_at)),
        ("endReason", opt(&h.end_reason)),
        ("result", h.result.map_or(Value::Null, |r| Value::from(r.as_str()))),
        ("kills", Value::from(h.kills)),
    ])
}

fn unlock_json(u: &SlayersPathUnlockRecord) -> Value {
    object(vec![
        ("nodeId", Value::from(u.node_id.clone())),
        ("unlockedAt", Value::from(u.unlocked_at.clone())),
        ("source", Value::from(u.source.clone())),
    ])
}

fn bad_user() -> Response {
    admin_error(
        StatusCode::BAD_REQUEST,
        "ACTIVITY_USER_INVALID",
        "The account id is missing or too long.",
    )
}

/// `GET /admin/v1/activity/players/:userId`: one player's daily documents over a day range.
pub async fn player(ctx: AdminContext) -> Response {
    let user = util::trim(ctx.param("userId")).to_owned();
    if user.is_empty() || user.len() > 200 {
        return bad_user();
    }
    let (from, to) = match read_range(&ctx.request.query, &day_of(now_ms())) {
        Ok(range) => range,
        Err(refused) => return refused,
    };
    let enabled = ActivitySettings::read(ctx.shared.settings()).enabled;
    let (f, t, u) = (from.clone(), to.clone(), user.clone());
    let read = ctx
        .store
        .read(move |conn, repos| {
            let Some((account, _)) = resolve(conn, repos, &u)? else {
                return Ok(None);
            };
            let days = repos.activity.find_daily_rows(conn, &account, &f, &t, 400)?;
            let hunts = repos.activity.find_hunts_for_account(conn, &account, 50)?;
            let unlocks = repos.activity.find_unlocks_for_account(conn, &account, 200)?;
            let flags = repos.activity.find_flags_for_account(conn, &account, 50)?;
            let flags: Vec<(ActivityFlagRecord, Value)> = flags
                .into_iter()
                .map(|fl| {
                    let json = flag_json(conn, repos, &fl);
                    (fl, json)
                })
                .collect();
            let ledger = repos
                .inventory_transactions
                .list_recent_for_account(conn, &account, 50)?;
            let grants = repos.progression_grants.list_recent_for_account(conn, &account, 50)?;
            Ok(Some((days, hunts, unlocks, flags, ledger, grants)))
        })
        .await;
    let found = match read {
        Ok(found) => found,
        Err(e) => {
            tracing::error!("[ActivityMon] player read failed: {e}");
            return admin_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "ACTIVITY_READ_FAILED",
                "The activity record could not be read.",
            );
        }
    };
    let (days, hunts, unlocks, flags, ledger, grants) = found.unwrap_or_default();
    let docs: Vec<Map<String, Value>> = days.iter().filter_map(|d| d.as_object().cloned()).collect();
    let shown_days: Vec<Value> = docs
        .iter()
        .map(|d| {
            let mut d = d.clone();
            for key in ["_id", "userId", "ttlAt"] {
                d.shift_remove(key);
            }
            Value::Object(d)
        })
        .collect();
    let mut timeline: Vec<(String, Value)> = Vec::new();
    for h in &hunts {
        timeline.push((
            h.started_at.clone(),
            object(vec![
                ("at", Value::from(h.started_at.clone())),
                ("type", Value::from("hunt")),
                ("detail", hunt_json(h)),
            ]),
        ));
    }
    for u in &unlocks {
        timeline.push((
            u.unlocked_at.clone(),
            object(vec![
                ("at", Value::from(u.unlocked_at.clone())),
                ("type", Value::from("slayersPathUnlock")),
                ("detail", unlock_json(u)),
            ]),
        ));
    }
    for (f, _) in &flags {
        timeline.push((
            f.created_at.clone(),
            object(vec![
                ("at", Value::from(f.created_at.clone())),
                ("type", Value::from("flag")),
                (
                    "detail",
                    object(vec![
                        ("id", Value::from(f.id.clone())),
                        ("kind", Value::from(f.kind.clone())),
                        ("severity", Value::from(f.severity.clone())),
                        (
                            "status",
                            Value::from(if f.acknowledged { "acknowledged" } else { "open" }),
                        ),
                    ]),
                ),
            ]),
        ));
    }
    for row in &ledger {
        let text = |k: &str| row.get(k).and_then(Value::as_str).map(str::to_owned);
        let at = text("completedAt")
            .filter(|t| !t.is_empty())
            .or_else(|| text("createdAt"))
            .unwrap_or_default();
        let pick = |k: &str| row.get(k).cloned().unwrap_or(Value::Null);
        timeline.push((
            at.clone(),
            object(vec![
                ("at", Value::from(at)),
                ("type", Value::from("inventory")),
                (
                    "detail",
                    object(vec![
                        ("status", pick("status")),
                        ("economySource", pick("economySource")),
                        ("economySessionId", pick("economySessionId")),
                        ("characterId", pick("characterId")),
                    ]),
                ),
            ]),
        ));
    }
    for g in &grants {
        timeline.push((
            g.created_at.clone(),
            object(vec![
                ("at", Value::from(g.created_at.clone())),
                ("type", Value::from("xp")),
                (
                    "detail",
                    object(vec![
                        ("progressionId", Value::from(g.progression_id.clone())),
                        ("amount", Value::from(g.amount)),
                        ("grantId", Value::from(g.grant_id.clone())),
                    ]),
                ),
            ]),
        ));
    }
    timeline.retain(|(at, _)| !at.is_empty());
    timeline.sort_by(|a, b| b.0.cmp(&a.0));
    timeline.truncate(100);
    util::ok(&object(vec![
        ("userId", Value::from(user)),
        ("from", Value::from(from)),
        ("to", Value::from(to)),
        ("monitorEnabled", Value::Bool(enabled)),
        ("days", Value::Array(shown_days)),
        ("totals", totals(docs.iter())),
        ("unlocks", Value::Array(unlocks.iter().map(unlock_json).collect())),
        ("hunts", Value::Array(hunts.iter().map(hunt_json).collect())),
        ("flags", Value::Array(flags.into_iter().map(|(_, j)| j).collect())),
        ("timeline", Value::Array(timeline.into_iter().map(|(_, v)| v).collect())),
    ]))
}

/// `GET /admin/v1/activity/overview`: totals over a day range.
pub async fn overview(ctx: AdminContext) -> Response {
    let (from, to) = match read_range(&ctx.request.query, &day_of(now_ms())) {
        Ok(range) => range,
        Err(refused) => return refused,
    };
    let enabled = ActivitySettings::read(ctx.shared.settings()).enabled;
    let (f, t) = (from.clone(), to.clone());
    let rows = match ctx
        .store
        .read(move |conn, repos| repos.activity.find_daily_rows_in_range(conn, &f, &t, 50_000))
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!("[ActivityMon] overview read failed: {e}");
            return admin_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "ACTIVITY_READ_FAILED",
                "The activity overview could not be read.",
            );
        }
    };
    let truncated = rows.len() == 50_000;
    let mut days = Vec::new();
    let (Some(mut at), Some(end)) = (midnight(&from), midnight(&to)) else {
        return range_error("ACTIVITY_RANGE_INVALID", "from and to must be UTC days, YYYY-MM-DD.");
    };
    while at <= end {
        let day = day_of(at);
        let docs: Vec<&Map<String, Value>> = rows
            .iter()
            .filter_map(|(_, d)| d.as_object())
            .filter(|d| d.get("day").and_then(Value::as_str) == Some(day.as_str()))
            .collect();
        let active = docs
            .iter()
            .filter(|d| {
                let n = |k: &str| d.get(k).and_then(Value::as_f64).unwrap_or(0.0);
                n("playtimeMinutes") > 0.0 || n("sessions") > 0.0
            })
            .count();
        let mut entry = Map::new();
        entry.insert("day".into(), Value::from(day));
        entry.insert("activePlayers".into(), Value::from(active));
        if let Value::Object(t) = totals(docs.iter().copied()) {
            entry.extend(t);
        }
        days.push(Value::Object(entry));
        at += DAY_MS;
    }
    util::ok(&object(vec![
        ("from", Value::from(from)),
        ("to", Value::from(to)),
        ("monitorEnabled", Value::Bool(enabled)),
        ("truncated", Value::Bool(truncated)),
        ("days", Value::Array(days)),
    ]))
}

/// `GET /admin/v1/activity/flags`: flags by status (`open`, `acknowledged` or `all`).
pub async fn flags(ctx: AdminContext) -> Response {
    let status = util::single(&ctx.request.query, "status")
        .map(|s| util::trim(s).to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "open".to_owned());
    let acknowledged = match status.as_str() {
        "open" => Some(false),
        "acknowledged" => Some(true),
        "all" => None,
        _ => {
            return admin_error(
                StatusCode::BAD_REQUEST,
                "ACTIVITY_FLAG_QUERY_INVALID",
                "status must be open, acknowledged or all.",
            );
        }
    };
    let page = page_of(&ctx.request);
    let limit = limit_of(&ctx.request, 25, 100);
    let skip = skip_of(page, limit);
    let read = ctx
        .store
        .read(move |conn, repos| {
            let (list, total) = repos.activity.list_flags(conn, acknowledged, skip, limit)?;
            let shown: Vec<Value> = list.iter().map(|f| flag_json(conn, repos, f)).collect();
            Ok((shown, total))
        })
        .await;
    match read {
        Ok((list, total)) => util::ok(&object(vec![
            ("flags", Value::Array(list)),
            ("total", Value::from(total)),
            ("page", Value::from(page)),
            ("limit", Value::from(limit)),
            ("status", Value::from(status)),
        ])),
        Err(e) => {
            tracing::error!("[ActivityMon] flag list failed: {e}");
            admin_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "ACTIVITY_READ_FAILED",
                "The flags could not be read.",
            )
        }
    }
}

enum Acknowledged {
    Missing,
    Done(Value),
}

/// `PATCH /admin/v1/activity/flags/:id`: acknowledges a flag.
pub async fn acknowledge(ctx: AdminContext) -> Response {
    let id = ctx.param("id").to_owned();
    if id.is_empty() || id.len() > 300 {
        return admin_error(
            StatusCode::BAD_REQUEST,
            "ACTIVITY_FLAG_INVALID",
            "The flag id is missing.",
        );
    }
    if ctx.request.field("acknowledged") != Some(&Value::Bool(true)) {
        return admin_error(
            StatusCode::BAD_REQUEST,
            "ACTIVITY_FLAG_INVALID",
            "Send {\"acknowledged\": true} (an optional note of up to 500 characters).",
        );
    }
    let note = ctx
        .request
        .field("note")
        .and_then(Value::as_str)
        .map(|n| util::take_chars(util::trim(n), 500))
        .filter(|n| !n.is_empty());
    let Some(caller) = ctx.caller.clone() else {
        return crate::admin::admin_internal("the acknowledgement had no caller");
    };
    let operator = caller.account_id().cloned();
    let ip = ctx.request.client_ip.clone();
    let actor_user = caller.user_id.clone();
    let done = ctx
        .store
        .write(&admin_actor(&caller), move |tx, repos| {
            let Some(before) = repos.activity.find_flag(tx.conn(), &id)? else {
                return Ok(Acknowledged::Missing);
            };
            let at = iso(now_ms());
            // The manager has no account: its id stands in, as the store writes the userId it finds
            let by = operator.unwrap_or_else(|| before.account.clone());
            let Some(after) = repos.activity.acknowledge_flag(tx, &id, &by, &at, note.as_deref())? else {
                return Ok(Acknowledged::Missing);
            };
            let shown = flag_json(tx.conn(), repos, &after);
            if !before.acknowledged && after.acknowledged {
                let target = user_id_of(tx.conn(), repos, &after.account);
                repos.admin.append_audit(
                    tx,
                    &AdminAuditRecord {
                        id: fresh_id(),
                        actor_user_id: actor_user,
                        target_user_id: Some(target),
                        action: "activity.flag.acknowledge".to_owned(),
                        old_state: Some(object(vec![("status", Value::from("open"))])),
                        new_state: Some(object(vec![("status", Value::from("acknowledged"))])),
                        reason: note,
                        ip,
                        request_id: fresh_id(),
                        created_at: at,
                    },
                )?;
            }
            Ok(Acknowledged::Done(shown))
        })
        .await;
    match done {
        Ok(Acknowledged::Missing) => admin_error(
            StatusCode::NOT_FOUND,
            "ACTIVITY_FLAG_NOT_FOUND",
            "No activity flag has that id.",
        ),
        Ok(Acknowledged::Done(flag)) => util::ok(&object(vec![("flag", flag)])),
        Err(e) => {
            tracing::error!("[ActivityMon] flag acknowledge failed: {e}");
            admin_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "ACTIVITY_WRITE_FAILED",
                "The flag could not be acknowledged.",
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn settings() -> ActivitySettings {
        ActivitySettings::read(&Settings::default())
    }

    #[test]
    fn playtime_steps() {
        assert_eq!(advance_playtime(None, 1_000_000), Some((0.0, 1, 1_000_000)));
        assert_eq!(advance_playtime(Some(1_000_000), 1_120_000), Some((2.0, 0, 1_120_000)));
        assert_eq!(advance_playtime(Some(1_000_000), 1_240_000), Some((0.0, 1, 1_240_000)));
        assert_eq!(advance_playtime(Some(1_000_000), 900_000), None);
    }

    #[test]
    fn flags_from_documents() {
        let doc = json!({"currency":{"CURRENCY_PLATINUM_STEAM":{"credited":5001}}});
        assert_eq!(
            daily_flag_candidates(doc.as_object().unwrap(), &settings()),
            vec![("PLATINUM_CREDIT_DAY", "credited=5001 threshold=5000".to_owned())]
        );
        let short = json!({"playtimeMinutes":14,"kills":{"a":100000}});
        assert!(daily_flag_candidates(short.as_object().unwrap(), &settings()).is_empty());
        let hour = json!({"playtimeMinutes":60,"kills":{"a":91}});
        assert_eq!(
            daily_flag_candidates(hour.as_object().unwrap(), &settings()),
            vec![("KILLS_PER_HOUR", "kills=91 playtimeMinutes=60 threshold=90".to_owned())]
        );
    }

    #[test]
    fn increments() {
        let input = ActivityInput {
            transaction_id: "t".into(),
            source: "INVENTORY_GAMESERVER".into(),
            granted: vec![("TOKEN_KILL_COUNT_Skarn".into(), 3.0)],
            credits: vec![],
            debits: vec![],
        };
        let (incs, tokens) = inventory_increments(&input);
        assert_eq!(incs, vec![("kills.Skarn".to_owned(), 3.0)]);
        assert_eq!(tokens, 3);
        let test = ActivityInput {
            source: "OPERATOR_TEST".into(),
            ..input
        };
        assert_eq!(inventory_increments(&test), (vec![], 0));
        let granted: Vec<(String, f64)> = (0..10).map(|i| (format!("ITEM_{i}"), 1.0)).collect();
        let spend = ActivityInput {
            transaction_id: "t".into(),
            source: "STORE".into(),
            granted,
            credits: vec![],
            debits: vec![("CURRENCY_PLATINUM_STEAM".into(), 50.0)],
        };
        let (incs, _) = inventory_increments(&spend);
        let spends = incs
            .iter()
            .filter(|(p, _)| p.starts_with("platinumSpendsByItem."))
            .count();
        assert_eq!(spends, 8);
        assert!(incs.contains(&("itemsGained.STORE".to_owned(), 10.0)));
        assert!(incs.contains(&("currency.CURRENCY_PLATINUM_STEAM.debited".to_owned(), 50.0)));
    }

    #[test]
    fn days_and_ranges() {
        assert!(!is_valid_day("2026-02-30") && is_valid_day("2026-02-28"));
        assert_eq!(safe_key("a-b"), "OTHER");
        let q = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
            pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
        };
        assert_eq!(
            read_range(&q(&[]), "2026-10-30").unwrap(),
            ("2026-10-01".to_owned(), "2026-10-30".to_owned())
        );
        assert_eq!(
            read_range(&q(&[("from", "2025-01-01"), ("to", "2026-10-30")]), "2026-10-30")
                .map(|_| ())
                .unwrap_err()
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert!(read_range(&q(&[("from", "2026-10-31"), ("to", "2026-10-30")]), "2026-10-30").is_err());
        assert_eq!(classify_dedicated_end_reason(" all_gone "), HuntResult::Completed);
        assert_eq!(classify_dedicated_end_reason("TIMEOUT"), HuntResult::Abandoned);
        let docs = [
            json!({"playtimeMinutes":1.5,"huntsStarted":{"dedicated":1},"kills":{"a":2},"currency":{"C":{"credited":3}}}),
        ];
        let maps: Vec<Map<String, Value>> = docs.iter().map(|d| d.as_object().unwrap().clone()).collect();
        assert_eq!(
            service_kit::jsjson::stringify(&totals(maps.iter())),
            r#"{"playtimeMinutes":1.5,"sessions":0,"hostingMinutes":0,"huntsHosted":0,"challengesClaimed":0,"slayersPathUnlocks":0,"reforges":0,"huntsStarted":{"dedicated":1,"realm":0,"playerHosted":0},"huntsCompleted":{"dedicated":0,"realm":0,"playerHosted":0},"huntsAbandoned":{"dedicated":0,"realm":0,"playerHosted":0},"kills":{"a":2},"xp":{},"itemsGained":{},"platinumSpendsByItem":{},"currency":{"C":{"credited":3,"debited":0}}}"#
        );
    }
}
