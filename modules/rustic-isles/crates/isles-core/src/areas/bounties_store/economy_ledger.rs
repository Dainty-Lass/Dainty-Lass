//! The economy ledger monitor (`diagnostics/economyLedgerMonitor.ts`): every Platinum credit and spend and every Hunt Pass
//! reward of a transaction, logged and rolled up per account, day and source. Monitoring only: nothing here refuses,
//! limits or changes a grant. The inventory area collects one [`EconomyLedger`] per `POST /inventory` and store
//! transaction and commits it once the transaction has committed; a retried transaction starts a fresh ledger.
//!
//! ```text
//! [PlatMon] credit user= amount= balanceAfter= source= ref=
//! [PlatMon] spend  user= amount= balanceAfter= source= for= session= host= ref=
//! [PlatMon] FLAG   a day's credited or spent Platinum crossed MYSTICPARADOX_PLAT_DAILY_FLAG / _SPEND_FLAG
//! [HuntPassMon] reward user= item= amount= source= ref=
//! [HuntPassMon] FLAG a day's CURRENCY_SEASONAL_COIN crossed MYSTICPARADOX_SEASONAL_COIN_DAILY_FLAG
//! ```

use std::collections::HashSet;

use serde_json::Value;

use super::platinum::{PLATINUM_AGGREGATE_ID, is_platinum_catalog_id};
use super::util::iso;
use crate::ids::AccountId;
use crate::settings::Settings;

/// Storage keeps a rollup about this long.
pub const ECONOMY_ROLLUP_TTL_MS: i64 = 400 * 24 * 60 * 60 * 1000;
pub const SEASONAL_COIN_CURRENCY: &str = "CURRENCY_SEASONAL_COIN";
const MAX_FOR_ITEMS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EconomySource {
    Store,
    Operator,
    OperatorTest,
    Quest,
    HuntPass,
    InventoryGameserver,
    InventoryHost,
    Other,
}

impl EconomySource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Store => "STORE",
            Self::Operator => "OPERATOR",
            Self::OperatorTest => "OPERATOR_TEST",
            Self::Quest => "QUEST",
            Self::HuntPass => "HUNT_PASS",
            Self::InventoryGameserver => "INVENTORY_GAMESERVER",
            Self::InventoryHost => "INVENTORY_HOST",
            Self::Other => "OTHER",
        }
    }
}

/// Where a transaction came from. `store_sku_id` is the purchase's SKU (named as `for=`); the P2P fields name a
/// player-host transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EconomyScope {
    pub source: EconomySource,
    pub store_sku_id: Option<String>,
    pub p2p_session_id: Option<String>,
    pub host_account_id: Option<String>,
}

impl EconomyScope {
    pub fn new(source: EconomySource) -> Self {
        Self {
            source,
            store_sku_id: None,
            p2p_session_id: None,
            host_account_id: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EconomyMonitorSettings {
    pub enabled: bool,
    pub platinum_daily_credit_flag: f64,
    pub platinum_daily_spend_flag: f64,
    pub seasonal_coin_daily_flag: f64,
    pub xp_per_minute_flag: f64,
}

impl EconomyMonitorSettings {
    /// The owner's defaults: the XP one is about ten times a normal pace, only a flag for review.
    pub const DEFAULTS: Self = Self {
        enabled: true,
        platinum_daily_credit_flag: 5000.0,
        platinum_daily_spend_flag: 5000.0,
        seasonal_coin_daily_flag: 1000.0,
        xp_per_minute_flag: 2000.0,
    };

    /// `ReadEconomyMonitorSettings`: a threshold that isn't a non-negative number falls back to its default and says so.
    pub fn read(settings: &Settings) -> Self {
        let threshold = |name: &str, fallback: f64| {
            let Some(raw) = settings.raw(name).map(service_kit::js::trim).filter(|r| !r.is_empty()) else {
                return fallback;
            };
            let value = service_kit::js::number(Some(&Value::from(raw)));
            if value.is_finite() && value >= 0.0 {
                value
            } else {
                let shown: String = raw.chars().take(40).collect();
                let quoted = Value::from(shown).to_string();
                tracing::warn!("[PlatMon] {name}={quoted} is not a non-negative number; using {fallback}");
                fallback
            }
        };
        let d = Self::DEFAULTS;
        Self {
            enabled: settings.on_unless_off("MYSTICPARADOX_ECONOMY_MONITOR"),
            platinum_daily_credit_flag: threshold("MYSTICPARADOX_PLAT_DAILY_FLAG", d.platinum_daily_credit_flag),
            platinum_daily_spend_flag: threshold("MYSTICPARADOX_PLAT_DAILY_SPEND_FLAG", d.platinum_daily_spend_flag),
            seasonal_coin_daily_flag: threshold("MYSTICPARADOX_SEASONAL_COIN_DAILY_FLAG", d.seasonal_coin_daily_flag),
            xp_per_minute_flag: threshold("MYSTICPARADOX_XP_PER_MINUTE_FLAG", d.xp_per_minute_flag),
        }
    }
}

// ---- classification

/// Platinum in any of its buckets, or the aggregate id a store price resolves to.
pub fn is_platinum_movement(catalog_id: &str) -> bool {
    catalog_id == PLATINUM_AGGREGATE_ID || is_platinum_catalog_id(catalog_id)
}

const HUNT_PASS_IDS: [&str; 4] = [
    "CURRENCY_SEASONAL_COIN",
    "CURRENCY_REWARDCACHE",
    "CURRENCY_PRESTIGE",
    "GRANTPROGRESSITEM_HUNTPASS",
];

/// `^CURRENCY_S\d+_(COIN|DAILY)$`, `^TOKEN_HUNT_?PASS`, `^CONTAINER_CORE_PRESTIGE_`.
fn hunt_pass_pattern(id: &str) -> bool {
    if let Some(rest) = id.strip_prefix("CURRENCY_S") {
        let digits = rest.trim_start_matches(|c: char| c.is_ascii_digit());
        if digits.len() < rest.len() && matches!(digits, "_COIN" | "_DAILY") {
            return true;
        }
    }
    id.starts_with("TOKEN_HUNTPASS") || id.starts_with("TOKEN_HUNT_PASS") || id.starts_with("CONTAINER_CORE_PRESTIGE_")
}

/// Season and minipass tracks are the passes; weapon tracks have prestige rewards but are not passes.
fn is_pass_track(progression_id: &str) -> bool {
    let lower = progression_id.to_ascii_lowercase();
    lower.starts_with("season") || lower.starts_with("minipass") || lower.starts_with("test_minipass")
}

fn add_reward_groups(into: &mut HashSet<String>, groups: Option<&Value>) {
    let list: Vec<&Value> = match groups {
        Some(Value::Array(list)) => list.iter().collect(),
        Some(Value::Null) | None => Vec::new(),
        Some(other) => vec![other],
    };
    for group in list {
        for item in group
            .get("stacked_items")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice)
        {
            if let Some(id) = item
                .get("catalog_id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
            {
                into.insert(id.to_owned());
            }
        }
    }
}

/// `CollectHuntPassRewardIds`: the rank rewards (free and premium) of every pass track, and the prestige rewards of
/// every track, from `progression_config.json`.
pub fn collect_hunt_pass_reward_ids(progression: Option<&Value>) -> HashSet<String> {
    let mut ids = HashSet::new();
    let paths = progression
        .and_then(|p| p.pointer("/payload/paths"))
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    for path in paths {
        let id = path
            .get("progression_id")
            .map_or_else(String::new, |v| service_kit::js::string(Some(v)));
        if is_pass_track(&id) {
            add_reward_groups(&mut ids, path.get("free_rewards"));
            add_reward_groups(&mut ids, path.get("premium_rewards"));
        }
        add_reward_groups(&mut ids, path.pointer("/prestige/free_rewards"));
        add_reward_groups(&mut ids, path.pointer("/prestige/premium_rewards"));
    }
    ids
}

/// The catalogue's Hunt Pass rewards from the module's game data; a missing file leaves the fixed ids and patterns.
pub fn default_hunt_pass_reward_ids(root: &crate::game_data::GameDataRoot) -> HashSet<String> {
    let file = root
        .backend_file("progression_config.json")
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| service_kit::jsjson::parse(text.trim_start_matches('\u{feff}')).ok());
    collect_hunt_pass_reward_ids(file.as_ref())
}

pub fn is_hunt_pass_reward_id(catalog_id: &str, reward_ids: &HashSet<String>) -> bool {
    HUNT_PASS_IDS.contains(&catalog_id) || hunt_pass_pattern(catalog_id) || reward_ids.contains(catalog_id)
}

/// The source a credit is logged and rolled up under: a challenge-funded coin is a quest, a store or operator
/// transaction keeps its own source, any other Hunt Pass reward is Hunt Pass, the rest keep the transaction's.
pub fn credit_source_of(
    catalog_id: &str,
    scope: &EconomyScope,
    challenge_funded: bool,
    reward_ids: &HashSet<String>,
) -> EconomySource {
    if challenge_funded {
        return EconomySource::Quest;
    }
    if matches!(scope.source, EconomySource::Store | EconomySource::Operator) {
        return scope.source;
    }
    if is_hunt_pass_reward_id(catalog_id, reward_ids) {
        return EconomySource::HuntPass;
    }
    scope.source
}

// ---- log text

/// A catalog id or session/account id as a log line may carry it: plain when simple, quoted otherwise (no forged lines).
pub fn log_token(value: Option<&str>) -> String {
    let text = value.unwrap_or("-");
    if !text.is_empty()
        && text.len() <= 128
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))
    {
        text.to_owned()
    } else {
        Value::from(text.chars().take(64).collect::<String>()).to_string()
    }
}

/// `for=` for a transaction's grants: the first eight `id:quantity` pairs, then `+n more`.
pub fn format_granted_items(items: &[(String, f64)]) -> String {
    if items.is_empty() {
        return "-".into();
    }
    let shown: Vec<String> = items
        .iter()
        .take(MAX_FOR_ITEMS)
        .map(|(id, q)| format!("{}:{}", log_token(Some(id)), service_kit::jsjson::number(*q)))
        .collect();
    let more = if items.len() > MAX_FOR_ITEMS {
        format!(",+{} more", items.len() - MAX_FOR_ITEMS)
    } else {
        String::new()
    };
    shown.join(",") + &more
}

/// Crossed in this write: the day total was at or under the threshold before it and is over it after.
pub fn flag_crossed(total_after: f64, delta: f64, threshold: f64) -> bool {
    let before = total_after - delta;
    before <= threshold && total_after > threshold
}

/// A catalog id as a rollup field name.
fn safe_key(catalog_id: &str) -> &str {
    if (1..=80).contains(&catalog_id.len()) && catalog_id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        catalog_id
    } else {
        "OTHER"
    }
}

// ---- the ledger

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
}

/// One rollup write the commit asks for: the day's document of one currency, and what to add to it.
#[derive(Debug, Clone, PartialEq)]
pub struct RollupWrite {
    pub day: String,
    pub currency: String,
    /// Dotted paths in the order they were first added
    pub increments: Vec<(String, f64)>,
    pub ttl_at: i64,
    flag: Flag,
}

#[derive(Debug, Clone, PartialEq)]
enum Flag {
    PlatinumDay { credited: f64, debited: f64 },
    SeasonalCoin { added: f64 },
}

fn add(into: &mut Vec<(String, f64)>, path: &str, amount: f64) {
    match into.iter_mut().find(|(p, _)| p == path) {
        Some(slot) => slot.1 += amount,
        None => into.push((path.to_owned(), amount)),
    }
}

#[derive(Debug, Clone)]
pub struct EconomyLedger {
    user_id: String,
    transaction_id: Option<String>,
    scope: EconomyScope,
    /// What the transaction granted (the `for=` list for everything but a store purchase)
    granted: Vec<(String, f64)>,
    settings: EconomyMonitorSettings,
    reward_ids: HashSet<String>,
    credits: Vec<(f64, f64, EconomySource)>,
    rewards: Vec<(String, f64, EconomySource)>,
    spent: f64,
    spent_balance_after: f64,
}

/// `NewEconomyLedger`: `None` while the monitor is switched off (the caller then records nothing).
pub fn new_economy_ledger(
    user_id: &str,
    transaction_id: Option<&str>,
    scope: EconomyScope,
    granted: Vec<(String, f64)>,
    settings: EconomyMonitorSettings,
    reward_ids: HashSet<String>,
) -> Option<EconomyLedger> {
    settings.enabled.then(|| EconomyLedger {
        user_id: user_id.to_owned(),
        transaction_id: transaction_id.map(str::to_owned),
        scope,
        granted,
        settings,
        reward_ids,
        credits: Vec::new(),
        rewards: Vec::new(),
        spent: 0.0,
        spent_balance_after: 0.0,
    })
}

/// What a commit does: the lines to log and the rollups to write.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CommitPlan {
    pub lines: Vec<(Level, String)>,
    pub rollups: Vec<RollupWrite>,
}

impl EconomyLedger {
    /// A Platinum credit of one bucket; `balance_after` is the account's Platinum across all buckets after it.
    pub fn record_platinum_credit(
        &mut self,
        catalog_id: &str,
        amount: f64,
        balance_after: f64,
        challenge_funded: bool,
    ) {
        if amount.is_nan() || amount <= 0.0 {
            return;
        }
        let source = credit_source_of(catalog_id, &self.scope, challenge_funded, &self.reward_ids);
        self.credits.push((amount, balance_after, source));
    }

    /// Platinum spent by this transaction (all its debits together); `balance_after` is the account's total after.
    pub fn record_platinum_spend(&mut self, amount: f64, balance_after: f64) {
        if amount.is_nan() || amount <= 0.0 {
            return;
        }
        self.spent += amount;
        self.spent_balance_after = balance_after;
    }

    /// A credit of a Hunt Pass reward that is not Platinum. Other catalog ids are ignored.
    pub fn record_reward(&mut self, catalog_id: &str, amount: f64, challenge_funded: bool) {
        if amount.is_nan() || amount <= 0.0 || is_platinum_movement(catalog_id) {
            return;
        }
        if !challenge_funded && !is_hunt_pass_reward_id(catalog_id, &self.reward_ids) {
            return;
        }
        let source = credit_source_of(catalog_id, &self.scope, challenge_funded, &self.reward_ids);
        self.rewards.push((catalog_id.to_owned(), amount, source));
    }

    pub fn is_empty(&self) -> bool {
        self.credits.is_empty() && self.spent == 0.0 && self.rewards.is_empty()
    }

    /// Every entry's log line and the rollup writes, for `now_ms`. Nothing is logged or stored until the caller does,
    /// after the transaction committed.
    pub fn commit(&self, now_ms: i64) -> CommitPlan {
        let mut plan = CommitPlan::default();
        if self.scope.source == EconomySource::OperatorTest || !self.settings.enabled || self.is_empty() {
            return plan;
        }
        let day = iso(now_ms)[..10].to_owned();
        let ttl_at = now_ms + ECONOMY_ROLLUP_TTL_MS;
        let user = log_token(Some(&self.user_id));
        let reference = log_token(self.transaction_id.as_deref());
        let num = service_kit::jsjson::number;

        let mut platinum: Vec<(String, f64)> = Vec::new();
        let mut credit_total = 0.0;
        for (amount, balance_after, source) in &self.credits {
            plan.lines.push((
                Level::Info,
                format!(
                    "[PlatMon] credit user={user} amount={} balanceAfter={} source={} ref={reference}",
                    num(*amount),
                    num(*balance_after),
                    source.as_str()
                ),
            ));
            credit_total += amount;
            add(&mut platinum, "credited", *amount);
            add(&mut platinum, "count", 1.0);
            add(
                &mut platinum,
                &format!("bySource.{}.credited", source.as_str()),
                *amount,
            );
            add(&mut platinum, &format!("bySource.{}.count", source.as_str()), 1.0);
        }

        let mut spend_total = 0.0;
        if self.spent > 0.0 {
            let named_sku = (self.scope.source == EconomySource::Store)
                .then_some(self.scope.store_sku_id.as_deref())
                .flatten();
            let (items, for_text): (Vec<String>, String) = match named_sku {
                Some(sku) => (vec![sku.to_owned()], log_token(Some(sku))),
                None => (
                    self.granted.iter().map(|(id, _)| id.clone()).collect(),
                    format_granted_items(&self.granted),
                ),
            };
            let source = self.scope.source.as_str();
            plan.lines.push((
                Level::Info,
                format!(
                    "[PlatMon] spend user={user} amount={} balanceAfter={} source={source} for={for_text} session={} host={} ref={reference}",
                    num(self.spent),
                    num(self.spent_balance_after),
                    log_token(Some(self.scope.p2p_session_id.as_deref().unwrap_or("-"))),
                    log_token(Some(self.scope.host_account_id.as_deref().unwrap_or("-"))),
                ),
            ));
            spend_total = self.spent;
            add(&mut platinum, "debited", self.spent);
            add(&mut platinum, "count", 1.0);
            add(&mut platinum, &format!("bySource.{source}.debited"), self.spent);
            add(&mut platinum, &format!("bySource.{source}.count"), 1.0);
            // By item: the transaction counts once for each item it granted (the first eight, as in `for=`) and carries
            // its whole spend against each, so an item total is not a share of the spend
            let mut seen: Vec<&str> = Vec::new();
            for item in items.iter().take(MAX_FOR_ITEMS).map(|id| safe_key(id)) {
                if seen.contains(&item) {
                    continue;
                }
                seen.push(item);
                add(&mut platinum, &format!("byItem.{item}.count"), 1.0);
                add(&mut platinum, &format!("byItem.{item}.debited"), self.spent);
            }
        }
        if !platinum.is_empty() {
            plan.rollups.push(RollupWrite {
                day: day.clone(),
                currency: PLATINUM_AGGREGATE_ID.to_owned(),
                increments: platinum,
                ttl_at,
                flag: Flag::PlatinumDay {
                    credited: credit_total,
                    debited: spend_total,
                },
            });
        }

        let mut totals: Vec<(String, Vec<(String, f64)>)> = Vec::new();
        for (catalog_id, amount, source) in &self.rewards {
            plan.lines.push((
                Level::Info,
                format!(
                    "[HuntPassMon] reward user={user} item={} amount={} source={} ref={reference}",
                    log_token(Some(catalog_id)),
                    num(*amount),
                    source.as_str()
                ),
            ));
            let index = totals.iter().position(|(id, _)| id == catalog_id).unwrap_or_else(|| {
                totals.push((catalog_id.clone(), Vec::new()));
                totals.len() - 1
            });
            let into = &mut totals[index].1;
            add(into, "credited", *amount);
            add(into, "count", 1.0);
            add(into, &format!("bySource.{}.credited", source.as_str()), *amount);
            add(into, &format!("bySource.{}.count", source.as_str()), 1.0);
        }
        for (currency, increments) in totals {
            let added = increments
                .iter()
                .find(|(p, _)| p == "credited")
                .map_or(0.0, |(_, v)| *v);
            plan.rollups.push(RollupWrite {
                day: day.clone(),
                currency,
                increments,
                ttl_at,
                flag: Flag::SeasonalCoin { added },
            });
        }
        plan
    }

    /// The flag warnings a rollup's document after its write calls for.
    pub fn flags_for(&self, write: &RollupWrite, after: &Value, day: &str) -> Vec<String> {
        let total = |key: &str| service_kit::js::number(after.get(key));
        let total = |key: &str| {
            let n = total(key);
            if n.is_nan() { 0.0 } else { n }
        };
        let user = log_token(Some(&self.user_id));
        let mut lines = Vec::new();
        match write.flag {
            Flag::PlatinumDay { credited, debited } => {
                if credited > 0.0 && flag_crossed(total("credited"), credited, self.settings.platinum_daily_credit_flag)
                {
                    lines.push(format!(
                        "[PlatMon] FLAG credit user={user} day={day} credited={} threshold={} (this transaction +{})",
                        service_kit::js::string(after.get("credited")),
                        service_kit::jsjson::number(self.settings.platinum_daily_credit_flag),
                        service_kit::jsjson::number(credited)
                    ));
                }
                if debited > 0.0 && flag_crossed(total("debited"), debited, self.settings.platinum_daily_spend_flag) {
                    lines.push(format!(
                        "[PlatMon] FLAG spend user={user} day={day} debited={} threshold={} (this transaction -{})",
                        service_kit::js::string(after.get("debited")),
                        service_kit::jsjson::number(self.settings.platinum_daily_spend_flag),
                        service_kit::jsjson::number(debited)
                    ));
                }
            }
            Flag::SeasonalCoin { added } => {
                if write.currency == SEASONAL_COIN_CURRENCY
                    && flag_crossed(total("credited"), added, self.settings.seasonal_coin_daily_flag)
                {
                    lines.push(format!(
                        "[HuntPassMon] FLAG user={user} day={day} item={} credited={} threshold={} (this transaction +{})",
                        write.currency,
                        service_kit::js::string(after.get("credited")),
                        service_kit::jsjson::number(self.settings.seasonal_coin_daily_flag),
                        service_kit::jsjson::number(added)
                    ));
                }
            }
        }
        lines
    }

    /// Logs everything and writes the rollups to `store`; never fails the grant that already committed (a failure is a
    /// warning). `account` keys the rollup rows.
    pub async fn commit_to_store(&self, store: &crate::db::IslesStore, account: &AccountId, now_ms: i64) {
        let plan = self.commit(now_ms);
        for (level, line) in &plan.lines {
            match level {
                Level::Info => tracing::info!("{line}"),
                Level::Warn => tracing::warn!("{line}"),
            }
        }
        for write in &plan.rollups {
            let (w, id) = (write.clone(), account.clone());
            let actor = module_api::ActorContext::new(module_api::Actor::System);
            let after = store
                .write(&actor, move |tx, repos| {
                    repos
                        .economy_rollups
                        .add_to_rollup(tx, &id, &w.day, &w.currency, &w.increments, w.ttl_at)
                })
                .await;
            match after {
                Ok(after) => {
                    for line in self.flags_for(write, &after, &write.day) {
                        tracing::warn!("{line}");
                    }
                }
                Err(error) => tracing::warn!(
                    "{} rollup not stored user={} day={} currency={}: {error}",
                    if write.currency == PLATINUM_AGGREGATE_ID {
                        "[PlatMon]"
                    } else {
                        "[HuntPassMon]"
                    },
                    log_token(Some(&self.user_id)),
                    write.day,
                    write.currency
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn settings() -> EconomyMonitorSettings {
        EconomyMonitorSettings::DEFAULTS
    }

    fn progression() -> Value {
        json!({ "payload": { "paths": [
            { "progression_id": "season43",
              "free_rewards": [{ "rank_id": 1, "stacked_items": [{ "catalog_id": "CURRENCY_PLATINUM_UNIV", "quantity": 4 }] }],
              "premium_rewards": [{ "rank_id": 2, "stacked_items": [{ "catalog_id": "TOKEN_HUNTPASS_EXTRA", "quantity": 1 }] }],
              "prestige": { "free_rewards": { "stacked_items": [{ "catalog_id": "CURRENCY_PJM_PRESTIGE_FILLED", "quantity": 1 }] } } },
            { "progression_id": "ExperienceTrack_Weapon_Axe",
              "free_rewards": [{ "rank_id": 1, "stacked_items": [{ "catalog_id": "EJ_AXE_EMOTE", "quantity": 1 }] }],
              "prestige": { "premium_rewards": { "stacked_items": [{ "catalog_id": "CURRENCY_PRESTIGE_AXE", "quantity": 1 }] } } },
        ] } })
    }

    fn reward_ids() -> HashSet<String> {
        collect_hunt_pass_reward_ids(Some(&progression()))
    }

    fn ledger(user: &str, txn: &str, scope: EconomyScope, granted: Vec<(String, f64)>) -> EconomyLedger {
        new_economy_ledger(user, Some(txn), scope, granted, settings(), reward_ids()).unwrap()
    }

    fn at(text: &str) -> i64 {
        super::super::util::parse_ms(text).unwrap()
    }

    fn map(increments: &[(String, f64)]) -> std::collections::BTreeMap<String, f64> {
        increments.iter().cloned().collect()
    }

    #[test]
    fn the_monitor_reads_its_switch_and_thresholds_with_the_owners_defaults() {
        assert_eq!(EconomyMonitorSettings::read(&Settings::default()), settings());
        for off in ["off", "OFF", "0", "false", "no"] {
            assert!(
                !EconomyMonitorSettings::read(&Settings::from_pairs([("MYSTICPARADOX_ECONOMY_MONITOR", off)])).enabled
            );
        }
        assert!(EconomyMonitorSettings::read(&Settings::from_pairs([("MYSTICPARADOX_ECONOMY_MONITOR", "on")])).enabled);
        let custom = EconomyMonitorSettings::read(&Settings::from_pairs([
            ("MYSTICPARADOX_PLAT_DAILY_FLAG", "120"),
            ("MYSTICPARADOX_PLAT_DAILY_SPEND_FLAG", "340"),
            ("MYSTICPARADOX_SEASONAL_COIN_DAILY_FLAG", "0"),
            ("MYSTICPARADOX_XP_PER_MINUTE_FLAG", "900"),
        ]));
        assert_eq!(
            custom,
            EconomyMonitorSettings {
                enabled: true,
                platinum_daily_credit_flag: 120.0,
                platinum_daily_spend_flag: 340.0,
                seasonal_coin_daily_flag: 0.0,
                xp_per_minute_flag: 900.0
            }
        );
        for bad in ["lots", "-3"] {
            let read = EconomyMonitorSettings::read(&Settings::from_pairs([("MYSTICPARADOX_PLAT_DAILY_FLAG", bad)]));
            assert_eq!(read.platinum_daily_credit_flag, 5000.0);
        }
    }

    #[test]
    fn hunt_pass_rewards_come_from_pass_ranks_and_every_tracks_prestige() {
        let ids = reward_ids();
        for id in [
            "CURRENCY_PLATINUM_UNIV",
            "TOKEN_HUNTPASS_EXTRA",
            "CURRENCY_PJM_PRESTIGE_FILLED",
            "CURRENCY_PRESTIGE_AXE",
        ] {
            assert!(ids.contains(id), "{id}");
        }
        assert!(!ids.contains("EJ_AXE_EMOTE"));
        assert!(collect_hunt_pass_reward_ids(None).is_empty());
    }

    #[test]
    fn the_fixed_hunt_pass_ids_and_the_catalogues_rewards_are_hunt_pass_rewards() {
        let ids = reward_ids();
        for id in [
            "CURRENCY_SEASONAL_COIN",
            "CURRENCY_S13_COIN",
            "CURRENCY_S20_DAILY",
            "TOKEN_HUNTPASS_PRESTIGE_S08A",
            "CONTAINER_CORE_PRESTIGE_01",
            "CURRENCY_PLATINUM_UNIV",
        ] {
            assert!(is_hunt_pass_reward_id(id, &ids), "{id}");
        }
        for id in [
            "CURRENCY_NOTES",
            "BREAK_LERAWR_HIDE",
            "DYE_BABYBLUE_DURABLE",
            "EJ_AXE_EMOTE",
            "CURRENCY_S_COIN",
        ] {
            assert!(!is_hunt_pass_reward_id(id, &ids), "{id}");
        }
    }

    #[test]
    fn a_platinum_id_is_any_bucket_or_the_aggregate() {
        assert!(is_platinum_movement("CURRENCY_PLATINUM"));
        assert!(is_platinum_movement("CURRENCY_PLATINUM_UNIV"));
        assert!(!is_platinum_movement("CURRENCY_PLATINUM_NOT_A_BUCKET"));
        assert!(!is_platinum_movement("CURRENCY_NOTES"));
    }

    #[test]
    fn the_source_of_a_credit() {
        let ids = reward_ids();
        let gs = EconomyScope::new(EconomySource::InventoryGameserver);
        let host = EconomyScope::new(EconomySource::InventoryHost);
        assert_eq!(
            credit_source_of("CURRENCY_SEASONAL_COIN", &gs, true, &ids),
            EconomySource::Quest
        );
        assert_eq!(
            credit_source_of(
                "CURRENCY_PLATINUM_UNIV",
                &EconomyScope::new(EconomySource::Store),
                false,
                &ids
            ),
            EconomySource::Store
        );
        assert_eq!(
            credit_source_of(
                "CURRENCY_PLATINUM_UNIV",
                &EconomyScope::new(EconomySource::Operator),
                false,
                &ids
            ),
            EconomySource::Operator
        );
        assert_eq!(
            credit_source_of("CURRENCY_PLATINUM_UNIV", &gs, false, &ids),
            EconomySource::HuntPass
        );
        assert_eq!(
            credit_source_of("CURRENCY_NOTES", &gs, false, &ids),
            EconomySource::InventoryGameserver
        );
        assert_eq!(
            credit_source_of("CURRENCY_NOTES", &host, false, &ids),
            EconomySource::InventoryHost
        );
    }

    #[test]
    fn the_for_list_names_eight_items_then_counts_the_rest() {
        assert_eq!(format_granted_items(&[]), "-");
        assert_eq!(
            format_granted_items(&[("COSMETIC_X".into(), 1.0), ("CURRENCY_Y".into(), 500.0)]),
            "COSMETIC_X:1,CURRENCY_Y:500"
        );
        let ten: Vec<(String, f64)> = (0..10).map(|i| (format!("ITEM_{i}"), f64::from(i + 1))).collect();
        assert_eq!(
            format_granted_items(&ten),
            "ITEM_0:1,ITEM_1:2,ITEM_2:3,ITEM_3:4,ITEM_4:5,ITEM_5:6,ITEM_6:7,ITEM_7:8,+2 more"
        );
    }

    #[test]
    fn an_id_that_could_forge_a_log_line_is_quoted() {
        assert_eq!(log_token(Some("player-1")), "player-1");
        assert_eq!(log_token(None), "-");
        let forged = log_token(Some("bad\n[PlatMon] credit user=victim"));
        assert!(!forged.contains('\n') && forged.starts_with('"'));
    }

    #[test]
    fn a_flag_fires_on_the_write_that_crosses_the_threshold() {
        assert!(flag_crossed(6000.0, 1500.0, 5000.0));
        assert!(!flag_crossed(7000.0, 1000.0, 5000.0));
        assert!(!flag_crossed(5000.0, 1000.0, 5000.0));
        assert!(flag_crossed(120.0, 120.0, 0.0));
    }

    #[test]
    fn a_switched_off_monitor_makes_no_ledger() {
        let off = EconomyMonitorSettings {
            enabled: false,
            ..settings()
        };
        assert!(
            new_economy_ledger(
                "p",
                None,
                EconomyScope::new(EconomySource::Other),
                vec![],
                off,
                HashSet::new()
            )
            .is_none()
        );
    }

    #[test]
    fn credit_spend_and_reward_are_logged_in_the_specs_formats_and_rolled_up() {
        let mut l = ledger(
            "player-1",
            "txn-1",
            EconomyScope::new(EconomySource::InventoryGameserver),
            vec![("BREAK_X".into(), 2.0)],
        );
        l.record_platinum_credit("CURRENCY_PLATINUM_UNIV", 500.0, 500.0, false);
        l.record_platinum_spend(120.0, 380.0);
        l.record_reward("TOKEN_HUNTPASS_EXTRA", 1.0, false);
        let plan = l.commit(at("2026-10-14T12:00:00Z"));
        let lines: Vec<&str> = plan.lines.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(
            lines,
            [
                "[PlatMon] credit user=player-1 amount=500 balanceAfter=500 source=HUNT_PASS ref=txn-1",
                "[PlatMon] spend user=player-1 amount=120 balanceAfter=380 source=INVENTORY_GAMESERVER for=BREAK_X:2 session=- host=- ref=txn-1",
                "[HuntPassMon] reward user=player-1 item=TOKEN_HUNTPASS_EXTRA amount=1 source=HUNT_PASS ref=txn-1",
            ]
        );
        let platinum = plan.rollups.iter().find(|r| r.currency == "CURRENCY_PLATINUM").unwrap();
        assert_eq!(platinum.day, "2026-10-14");
        let want: std::collections::BTreeMap<String, f64> = [
            ("credited", 500.0),
            ("count", 2.0),
            ("bySource.HUNT_PASS.credited", 500.0),
            ("bySource.HUNT_PASS.count", 1.0),
            ("debited", 120.0),
            ("bySource.INVENTORY_GAMESERVER.debited", 120.0),
            ("bySource.INVENTORY_GAMESERVER.count", 1.0),
            ("byItem.BREAK_X.count", 1.0),
            ("byItem.BREAK_X.debited", 120.0),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        assert_eq!(map(&platinum.increments), want);
        let reward = plan
            .rollups
            .iter()
            .find(|r| r.currency == "TOKEN_HUNTPASS_EXTRA")
            .unwrap();
        assert_eq!(reward.increments.len(), 4);
        assert_eq!(
            platinum.ttl_at - at("2026-10-14T12:00:00Z"),
            400 * 24 * 60 * 60 * 1000,
            "kept 400 days"
        );
    }

    #[test]
    fn a_store_purchases_spend_names_its_sku() {
        let mut scope = EconomyScope::new(EconomySource::Store);
        scope.store_sku_id = Some("SKU_BUNDLE_9".into());
        let mut l = ledger("player-2", "txn-store", scope, vec![("COSMETIC_X".into(), 1.0)]);
        l.record_platinum_spend(1000.0, 250.0);
        let plan = l.commit(at("2026-10-14T09:00:00Z"));
        assert_eq!(
            plan.lines[0].1,
            "[PlatMon] spend user=player-2 amount=1000 balanceAfter=250 source=STORE for=SKU_BUNDLE_9 session=- host=- ref=txn-store"
        );
        let inc = map(&plan.rollups[0].increments);
        assert_eq!(inc["byItem.SKU_BUNDLE_9.debited"], 1000.0);
        assert_eq!(inc["bySource.STORE.debited"], 1000.0);
    }

    #[test]
    fn a_hosts_spend_names_its_session_and_the_day_total_flags_once() {
        let mut scope = EconomyScope::new(EconomySource::InventoryHost);
        scope.p2p_session_id = Some("hunt-7".into());
        scope.host_account_id = Some("host-9".into());
        let small = EconomyMonitorSettings {
            platinum_daily_spend_flag: 100.0,
            ..settings()
        };
        let mut total = 0.0;
        let mut flags = Vec::new();
        let mut first_line = String::new();
        for txn in ["t1", "t2", "t3"] {
            let mut l = new_economy_ledger("player-3", Some(txn), scope.clone(), vec![], small, reward_ids()).unwrap();
            l.record_platinum_spend(60.0, 0.0);
            let plan = l.commit(at("2026-10-14T20:00:00Z"));
            if txn == "t1" {
                first_line = plan.lines[0].1.clone();
            }
            total += 60.0;
            let after = json!({ "debited": total });
            flags.extend(l.flags_for(&plan.rollups[0], &after, &plan.rollups[0].day));
        }
        assert_eq!(
            first_line,
            "[PlatMon] spend user=player-3 amount=60 balanceAfter=0 source=INVENTORY_HOST for=- session=hunt-7 host=host-9 ref=t1"
        );
        assert_eq!(flags.len(), 1, "120 crosses 100 once; the third write is still over");
        assert!(flags[0].contains("debited=120 threshold=100"), "{}", flags[0]);
    }

    #[test]
    fn a_season_coin_day_total_over_its_threshold_is_flagged() {
        let mut l = ledger(
            "player-4",
            "t-coin",
            EconomyScope::new(EconomySource::InventoryGameserver),
            vec![],
        );
        l.record_reward("CURRENCY_SEASONAL_COIN", 1200.0, false);
        let plan = l.commit(at("2026-10-14T10:00:00Z"));
        let flags = l.flags_for(&plan.rollups[0], &json!({ "credited": 1200 }), "2026-10-14");
        assert!(
            flags[0].starts_with(
                "[HuntPassMon] FLAG user=player-4 day=2026-10-14 item=CURRENCY_SEASONAL_COIN credited=1200"
            )
        );
    }

    #[test]
    fn an_empty_ledger_and_a_test_grant_log_nothing() {
        let empty = ledger("player-6", "t", EconomyScope::new(EconomySource::Other), vec![]);
        assert_eq!(empty.commit(0), CommitPlan::default());
        let mut test_grant = ledger("player-7", "t", EconomyScope::new(EconomySource::OperatorTest), vec![]);
        test_grant.record_platinum_credit("CURRENCY_PLATINUM_UNIV", 10.0, 10.0, false);
        assert_eq!(test_grant.commit(0), CommitPlan::default());
    }
}
