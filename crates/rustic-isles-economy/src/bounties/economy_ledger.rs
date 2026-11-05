// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The economy ledger monitor (bounties spec §4.14, inventory's W9): Platinum and Hunt Pass reward movements of one
//! committed transaction, logged and summed per day. Monitoring only: nothing here refuses or changes a grant.

use std::collections::HashSet;
use std::sync::Arc;

use module_api::{Actor, ActorContext};
use rustic_isles::{GameDataRoot, Settings, Shared};
use rustic_isles_store::{AccountId, IslesStore};
use serde_json::Value;
use service_kit::{js, jsjson};

use crate::bounties::platinum::{PLATINUM_AGGREGATE_ID, is_platinum_catalog_id};
use crate::inventory::CommittedTransaction;
use crate::util::{first_chars, num, num0};

const DAY_MS: i64 = 86_400_000;
const SEASONAL_COIN: &str = "CURRENCY_SEASONAL_COIN";

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
    pub fn as_str(self) -> &'static str {
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

    /// Unknown text is `OTHER`.
    pub fn parse(text: &str) -> Self {
        match text {
            "STORE" => Self::Store,
            "OPERATOR" => Self::Operator,
            "OPERATOR_TEST" => Self::OperatorTest,
            "QUEST" => Self::Quest,
            "HUNT_PASS" => Self::HuntPass,
            "INVENTORY_GAMESERVER" => Self::InventoryGameserver,
            "INVENTORY_HOST" => Self::InventoryHost,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub source: EconomySource,
    pub store_sku_id: Option<String>,
    pub p2p_session_id: Option<String>,
    pub host_account_id: Option<String>,
}

impl Scope {
    pub fn of(source: EconomySource) -> Self {
        Self {
            source,
            store_sku_id: None,
            p2p_session_id: None,
            host_account_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EconomyMonitorSettings {
    pub enabled: bool,
    pub plat_daily_flag: f64,
    pub plat_daily_spend_flag: f64,
    pub seasonal_coin_daily_flag: f64,
    /// Read and carried; this part does not use it
    pub xp_per_minute_flag: f64,
}

fn threshold(settings: &Settings, name: &str, default: f64) -> f64 {
    let Some(raw) = settings.raw(name) else { return default };
    let text = js::trim(raw);
    if text.is_empty() {
        return default;
    }
    let n = crate::util::js_number_text(text);
    if n.is_finite() && n >= 0.0 {
        return n;
    }
    let shown = jsjson::stringify(&serde_json::Value::String(first_chars(raw, 40)));
    let fallback = jsjson::number(default);
    tracing::warn!("[PlatMon] {name}={shown} is not a non-negative number; using {fallback}");
    default
}

impl EconomyMonitorSettings {
    pub fn read(settings: &Settings) -> Self {
        Self {
            enabled: settings.on_unless_off("RUSTIC_ISLES_ECONOMY_MONITOR"),
            plat_daily_flag: threshold(settings, "RUSTIC_ISLES_PLAT_DAILY_FLAG", 5000.0),
            plat_daily_spend_flag: threshold(settings, "RUSTIC_ISLES_PLAT_DAILY_SPEND_FLAG", 5000.0),
            seasonal_coin_daily_flag: threshold(settings, "RUSTIC_ISLES_SEASONAL_COIN_DAILY_FLAG", 1000.0),
            xp_per_minute_flag: threshold(settings, "RUSTIC_ISLES_XP_PER_MINUTE_FLAG", 2000.0),
        }
    }
}

fn reward_group_ids(group: Option<&Value>, out: &mut HashSet<String>) {
    let groups: Vec<&Value> = match group {
        Some(Value::Array(list)) => list.iter().collect(),
        Some(single @ Value::Object(_)) => vec![single],
        _ => Vec::new(),
    };
    for g in groups {
        for item in g.get("stacked_items").and_then(Value::as_array).into_iter().flatten() {
            if let Some(id) = item.get("catalog_id").and_then(Value::as_str) {
                out.insert(id.to_owned());
            }
        }
    }
}

/// Hunt Pass reward ids from `progression_config.json` (read per ledger; missing gives none).
pub fn hunt_pass_reward_ids(root: &GameDataRoot) -> HashSet<String> {
    let mut out = HashSet::new();
    let Ok(Some(config)) = root.try_load_backend("progression_config.json") else {
        return out;
    };
    let paths = config
        .get("payload")
        .and_then(|p| p.get("paths"))
        .and_then(Value::as_array);
    for path in paths.into_iter().flatten() {
        let id = js::string(path.get("progression_id")).to_lowercase();
        if id.starts_with("season") || id.starts_with("minipass") || id.starts_with("test_minipass") {
            reward_group_ids(path.get("free_rewards"), &mut out);
            reward_group_ids(path.get("premium_rewards"), &mut out);
        }
        let prestige = path.get("prestige");
        reward_group_ids(prestige.and_then(|p| p.get("free_rewards")), &mut out);
        reward_group_ids(prestige.and_then(|p| p.get("premium_rewards")), &mut out);
    }
    out
}

fn is_season_coin_pattern(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("CURRENCY_S") else {
        return false;
    };
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    digits > 0 && matches!(&rest[digits..], "_COIN" | "_DAILY")
}

pub fn is_hunt_pass_reward_id(id: &str, from_config: &HashSet<String>) -> bool {
    matches!(
        id,
        "CURRENCY_SEASONAL_COIN" | "CURRENCY_REWARDCACHE" | "CURRENCY_PRESTIGE" | "GRANTPROGRESSITEM_HUNTPASS"
    ) || is_season_coin_pattern(id)
        || ["TOKEN_HUNTPASS", "TOKEN_HUNT_PASS", "CONTAINER_CORE_PRESTIGE_"]
            .iter()
            .any(|p| id.starts_with(p))
        || from_config.contains(id)
}

#[derive(Debug, Clone, PartialEq)]
struct PlatCredit {
    id: String,
    amount: f64,
    balance_after: f64,
    funded: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct Reward {
    id: String,
    amount: f64,
    funded: bool,
}

/// One transaction's movements.
#[derive(Debug, Clone, PartialEq)]
pub struct EconomyLedger {
    user_id: String,
    transaction_id: String,
    scope: Scope,
    granted: Vec<(String, f64)>,
    hunt_pass_ids: HashSet<String>,
    credits: Vec<PlatCredit>,
    spend: Option<(f64, f64)>,
    rewards: Vec<Reward>,
}

/// One rollup write and the flag it may cross.
#[derive(Debug, Clone, PartialEq)]
pub struct RollupWrite {
    pub currency: String,
    pub increments: Vec<(String, f64)>,
    pub flags: Vec<Flag>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Flag {
    /// The counter whose day total is compared (`credited` or `debited`)
    pub counter: &'static str,
    pub threshold: f64,
    pub amount: f64,
    pub kind: FlagKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlagKind {
    PlatinumCredit,
    PlatinumSpend,
    SeasonalCoin,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommitPlan {
    pub day: String,
    pub ttl_at: i64,
    pub lines: Vec<String>,
    pub writes: Vec<RollupWrite>,
}

/// Plain when 1 to 128 bytes of ASCII letters, digits, `_ . : -`; absent `-`.
fn token(text: Option<&str>) -> String {
    match text {
        None => String::from("-"),
        Some(t) => crate::util::log_token(t),
    }
}

fn item_key(id: &str) -> &str {
    let plain = (1..=80).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    if plain { id } else { "OTHER" }
}

fn add(increments: &mut Vec<(String, f64)>, path: String, value: f64) {
    match increments.iter_mut().find(|(p, _)| *p == path) {
        Some((_, v)) => *v += value,
        None => increments.push((path, value)),
    }
}

impl EconomyLedger {
    pub fn new(
        user_id: &str,
        transaction_id: &str,
        scope: Scope,
        granted: Vec<(String, f64)>,
        hunt_pass_ids: HashSet<String>,
    ) -> Self {
        Self {
            user_id: user_id.to_owned(),
            transaction_id: transaction_id.to_owned(),
            scope,
            granted,
            hunt_pass_ids,
            credits: Vec::new(),
            spend: None,
            rewards: Vec::new(),
        }
    }

    fn credit_source(&self, id: &str, funded: bool) -> EconomySource {
        if funded {
            return EconomySource::Quest;
        }
        match self.scope.source {
            s @ (EconomySource::Store | EconomySource::Operator) => s,
            _ if is_hunt_pass_reward_id(id, &self.hunt_pass_ids) => EconomySource::HuntPass,
            s => s,
        }
    }

    pub fn record_platinum_credit(&mut self, id: &str, amount: f64, balance_after: f64, funded: bool) {
        if amount.is_nan() || amount <= 0.0 {
            return;
        }
        self.credits.push(PlatCredit {
            id: id.to_owned(),
            amount,
            balance_after,
            funded,
        });
    }

    pub fn record_platinum_spend(&mut self, amount: f64, balance_after: f64) {
        if amount.is_nan() || amount <= 0.0 {
            return;
        }
        let total = self.spend.map_or(0.0, |(a, _)| a) + amount;
        self.spend = Some((total, balance_after));
    }

    pub fn record_reward(&mut self, id: &str, amount: f64, funded: bool) {
        if is_platinum_catalog_id(id) || amount.is_nan() || amount <= 0.0 {
            return;
        }
        if funded || is_hunt_pass_reward_id(id, &self.hunt_pass_ids) {
            self.rewards.push(Reward {
                id: id.to_owned(),
                amount,
                funded,
            });
        }
    }

    pub fn is_empty(&self) -> bool {
        self.credits.is_empty() && self.spend.is_none() && self.rewards.is_empty()
    }

    fn spent_for(&self) -> String {
        if self.scope.source == EconomySource::Store
            && let Some(sku) = &self.scope.store_sku_id
        {
            return token(Some(sku));
        }
        if self.granted.is_empty() {
            return String::from("-");
        }
        let parts: Vec<String> = self
            .granted
            .iter()
            .take(8)
            .map(|(id, q)| format!("{id}:{}", jsjson::number(*q)))
            .collect();
        let mut text = parts.join(",");
        if self.granted.len() > 8 {
            text.push_str(&format!(",+{} more", self.granted.len() - 8));
        }
        text
    }

    /// The log lines and rollup writes at `now_ms` (nothing for `OPERATOR_TEST` or an empty ledger).
    pub fn commit(&self, now_ms: i64, settings: &EconomyMonitorSettings) -> Option<CommitPlan> {
        if self.scope.source == EconomySource::OperatorTest || !settings.enabled || self.is_empty() {
            return None;
        }
        let day = crate::util::iso(now_ms)[..10].to_owned();
        let user = token(Some(&self.user_id));
        let reference = token(Some(&self.transaction_id));
        let mut lines = Vec::new();
        let mut platinum: Vec<(String, f64)> = Vec::new();
        let mut flags = Vec::new();
        let mut credited = 0.0;
        for credit in &self.credits {
            let source = self.credit_source(&credit.id, credit.funded).as_str();
            lines.push(format!(
                "[PlatMon] credit user={user} amount={} balanceAfter={} source={source} ref={reference}",
                jsjson::number(credit.amount),
                jsjson::number(credit.balance_after)
            ));
            credited += credit.amount;
            add(&mut platinum, "credited".into(), credit.amount);
            add(&mut platinum, "count".into(), 1.0);
            add(&mut platinum, format!("bySource.{source}.credited"), credit.amount);
            add(&mut platinum, format!("bySource.{source}.count"), 1.0);
        }
        if credited > 0.0 {
            flags.push(Flag {
                counter: "credited",
                threshold: settings.plat_daily_flag,
                amount: credited,
                kind: FlagKind::PlatinumCredit,
            });
        }
        if let Some((amount, after)) = self.spend {
            let source = self.scope.source.as_str();
            lines.push(format!(
                "[PlatMon] spend user={user} amount={} balanceAfter={} source={source} for={} session={} host={} \
                 ref={reference}",
                jsjson::number(amount),
                jsjson::number(after),
                self.spent_for(),
                token(self.scope.p2p_session_id.as_deref()),
                token(self.scope.host_account_id.as_deref())
            ));
            add(&mut platinum, "debited".into(), amount);
            add(&mut platinum, "count".into(), 1.0);
            add(&mut platinum, format!("bySource.{source}.debited"), amount);
            add(&mut platinum, format!("bySource.{source}.count"), 1.0);
            let mut seen = HashSet::new();
            for (id, _) in self.granted.iter().take(8) {
                let key = item_key(id);
                if seen.insert(key.to_owned()) {
                    add(&mut platinum, format!("byItem.{key}.count"), 1.0);
                    add(&mut platinum, format!("byItem.{key}.debited"), amount);
                }
            }
            flags.push(Flag {
                counter: "debited",
                threshold: settings.plat_daily_spend_flag,
                amount,
                kind: FlagKind::PlatinumSpend,
            });
        }
        let mut writes = Vec::new();
        if !platinum.is_empty() {
            writes.push(RollupWrite {
                currency: PLATINUM_AGGREGATE_ID.to_owned(),
                increments: platinum,
                flags,
            });
        }
        let mut reward_writes: Vec<RollupWrite> = Vec::new();
        for reward in &self.rewards {
            let source = self.credit_source(&reward.id, reward.funded).as_str();
            lines.push(format!(
                "[HuntPassMon] reward user={user} item={} amount={} source={source} ref={reference}",
                token(Some(&reward.id)),
                jsjson::number(reward.amount)
            ));
            let write = match reward_writes.iter_mut().find(|w| w.currency == reward.id) {
                Some(w) => w,
                None => {
                    reward_writes.push(RollupWrite {
                        currency: reward.id.clone(),
                        increments: Vec::new(),
                        flags: Vec::new(),
                    });
                    let last = reward_writes.len() - 1;
                    &mut reward_writes[last]
                }
            };
            add(&mut write.increments, "credited".into(), reward.amount);
            add(&mut write.increments, "count".into(), 1.0);
            add(
                &mut write.increments,
                format!("bySource.{source}.credited"),
                reward.amount,
            );
            add(&mut write.increments, format!("bySource.{source}.count"), 1.0);
        }
        for write in &mut reward_writes {
            if write.currency == SEASONAL_COIN {
                let amount = write
                    .increments
                    .iter()
                    .find(|(p, _)| p == "credited")
                    .map_or(0.0, |(_, v)| *v);
                write.flags.push(Flag {
                    counter: "credited",
                    threshold: settings.seasonal_coin_daily_flag,
                    amount,
                    kind: FlagKind::SeasonalCoin,
                });
            }
        }
        writes.extend(reward_writes);
        Some(CommitPlan {
            day,
            ttl_at: now_ms + 400 * DAY_MS,
            lines,
            writes,
        })
    }

    /// Logs the lines and writes the rollups (plain system writes, never failing anything).
    pub async fn commit_to_store(
        &self,
        store: &IslesStore,
        account: &AccountId,
        now_ms: i64,
        settings: &EconomyMonitorSettings,
    ) {
        let Some(plan) = self.commit(now_ms, settings) else {
            return;
        };
        for line in &plan.lines {
            tracing::info!("{line}");
        }
        let user = token(Some(&self.user_id));
        for write in plan.writes {
            let (id, day, currency, increments, ttl) = (
                account.clone(),
                plan.day.clone(),
                write.currency.clone(),
                write.increments.clone(),
                plan.ttl_at,
            );
            let stored = store
                .write(&ActorContext::new(Actor::System), move |tx, repos| {
                    repos
                        .economy_rollups
                        .add_to_rollup(tx, &id, &day, &currency, &increments, ttl)
                })
                .await;
            let tag = if write.currency == PLATINUM_AGGREGATE_ID {
                "[PlatMon]"
            } else {
                "[HuntPassMon]"
            };
            let document = match stored {
                Ok(document) => document,
                Err(e) => {
                    tracing::warn!(
                        "{tag} rollup not stored user={user} day={} currency={}: {e}",
                        plan.day,
                        write.currency
                    );
                    continue;
                }
            };
            for flag in &write.flags {
                let after = num0(document.get(flag.counter));
                let before = after - flag.amount;
                if !(before <= flag.threshold && after > flag.threshold) {
                    continue;
                }
                let (t, a, total) = (
                    jsjson::number(flag.threshold),
                    jsjson::number(flag.amount),
                    jsjson::number(after),
                );
                match flag.kind {
                    FlagKind::PlatinumCredit => tracing::warn!(
                        "[PlatMon] FLAG credit user={user} day={} credited={total} threshold={t} (this transaction +{a})",
                        plan.day
                    ),
                    FlagKind::PlatinumSpend => tracing::warn!(
                        "[PlatMon] FLAG spend user={user} day={} debited={total} threshold={t} (this transaction -{a})",
                        plan.day
                    ),
                    FlagKind::SeasonalCoin => tracing::warn!(
                        "[HuntPassMon] FLAG user={user} day={} item=CURRENCY_SEASONAL_COIN credited={total} \
                         threshold={t} (this transaction +{a})",
                        plan.day
                    ),
                }
            }
        }
    }
}

/// The ledger of one committed inventory transaction (W9's rules).
pub fn ledger_for(transaction: &CommittedTransaction, hunt_pass_ids: HashSet<String>) -> EconomyLedger {
    let source = EconomySource::parse(&transaction.source);
    let granted = transaction
        .granted
        .iter()
        .filter(|(_, q)| q.is_finite() && *q > 0.0)
        .cloned()
        .collect();
    let mut ledger = EconomyLedger::new(
        &transaction.user_id,
        &transaction.transaction_id,
        Scope::of(source),
        granted,
        hunt_pass_ids,
    );
    let updated = transaction
        .result
        .get("updatedStackedItems")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let movement = |e: &&Value| {
        e.get("catalogId")
            .and_then(Value::as_str)
            .is_some_and(is_platinum_catalog_id)
    };
    let balance_after: f64 = updated
        .iter()
        .filter(movement)
        .map(|e| num0(e.get("quantity")).max(0.0))
        .sum();
    let funded = source == EconomySource::Quest;
    for (id, amount) in &transaction.currency_credits {
        if !(amount.is_finite() && *amount > 0.0) {
            continue;
        }
        if is_platinum_catalog_id(id) {
            let own = updated
                .iter()
                .find(|e| e.get("catalogId").and_then(Value::as_str) == Some(id.as_str()))
                .map_or(0.0, |e| num(e.get("quantity")));
            let after = if balance_after == 0.0 {
                if own.is_finite() { own } else { 0.0 }
            } else {
                balance_after
            };
            ledger.record_platinum_credit(id, *amount, after, funded);
        } else {
            ledger.record_reward(id, *amount, funded);
        }
    }
    for (id, amount) in &transaction.currency_debits {
        if is_platinum_catalog_id(id) && amount.is_finite() && *amount > 0.0 {
            ledger.record_platinum_spend(*amount, balance_after);
        }
    }
    ledger
}

/// W9: after the inventory commit, in the background; never fails the request.
pub fn commit_transaction(shared: &Arc<Shared>, transaction: &CommittedTransaction) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let settings = EconomyMonitorSettings::read(shared.settings());
    if !settings.enabled || EconomySource::parse(&transaction.source) == EconomySource::OperatorTest {
        return;
    }
    let Some(store) = shared.slot::<IslesStore>() else {
        return;
    };
    let ledger = ledger_for(transaction, hunt_pass_reward_ids(shared.game_data()));
    if ledger.is_empty() {
        return;
    }
    let account = transaction.account.clone();
    runtime.spawn(async move {
        ledger
            .commit_to_store(&store, &account, crate::util::system_now_ms(), &settings)
            .await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> EconomyMonitorSettings {
        EconomyMonitorSettings::read(&Settings::default())
    }

    #[test]
    fn thresholds() {
        let s = settings();
        assert_eq!(
            (
                s.enabled,
                s.plat_daily_flag,
                s.plat_daily_spend_flag,
                s.seasonal_coin_daily_flag,
                s.xp_per_minute_flag
            ),
            (true, 5000.0, 5000.0, 1000.0, 2000.0)
        );
        let s = EconomyMonitorSettings::read(&Settings::from_pairs([
            ("RUSTIC_ISLES_ECONOMY_MONITOR", "off"),
            ("RUSTIC_ISLES_PLAT_DAILY_FLAG", "-1"),
            ("RUSTIC_ISLES_PLAT_DAILY_SPEND_FLAG", " 12 "),
        ]));
        assert_eq!(
            (s.enabled, s.plat_daily_flag, s.plat_daily_spend_flag),
            (false, 5000.0, 12.0)
        );
    }

    #[test]
    fn reward_ids() {
        let none = HashSet::new();
        for id in [
            "CURRENCY_SEASONAL_COIN",
            "CURRENCY_S19_COIN",
            "CURRENCY_S13_DAILY",
            "TOKEN_HUNTPASS_X",
            "CONTAINER_CORE_PRESTIGE_1",
        ] {
            assert!(is_hunt_pass_reward_id(id, &none), "{id}");
        }
        for id in ["CURRENCY_S_COIN", "CURRENCY_S19_COINS", "CURRENCY_NOTES"] {
            assert!(!is_hunt_pass_reward_id(id, &none), "{id}");
        }
    }

    #[test]
    fn lines() {
        let granted: Vec<(String, f64)> = (0..10).map(|i| (format!("ITEM_{i}"), 1.0)).collect();
        let mut ledger = EconomyLedger::new(
            "u1",
            "tx1",
            Scope::of(EconomySource::InventoryGameserver),
            granted,
            HashSet::new(),
        );
        ledger.record_platinum_spend(100.0, 900.0);
        ledger.record_platinum_credit("CURRENCY_PLATINUM_UNIV", 5.0, 905.0, false);
        ledger.record_reward("CURRENCY_SEASONAL_COIN", 10.0, true);
        ledger.record_reward("CURRENCY_NOTES", 10.0, false);
        let plan = ledger.commit(0, &settings()).unwrap();
        assert_eq!(plan.day, "1970-01-01");
        assert_eq!(
            plan.lines[0],
            "[PlatMon] credit user=u1 amount=5 balanceAfter=905 source=INVENTORY_GAMESERVER ref=tx1"
        );
        assert!(plan.lines[1].contains(
            "for=ITEM_0:1,ITEM_1:1,ITEM_2:1,ITEM_3:1,ITEM_4:1,ITEM_5:1,ITEM_6:1,ITEM_7:1,+2 more session=- host=-"
        ));
        assert_eq!(
            plan.lines[2],
            "[HuntPassMon] reward user=u1 item=CURRENCY_SEASONAL_COIN amount=10 source=QUEST ref=tx1"
        );
        assert_eq!(plan.lines.len(), 3);
        assert_eq!(plan.writes[0].currency, "CURRENCY_PLATINUM");
        assert!(
            plan.writes[0]
                .increments
                .contains(&("byItem.ITEM_7.debited".into(), 100.0))
        );
        let mut store = EconomyLedger::new(
            "u\n1",
            "t",
            Scope {
                store_sku_id: Some("sku_1".into()),
                ..Scope::of(EconomySource::Store)
            },
            vec![],
            HashSet::new(),
        );
        store.record_platinum_spend(10.0, 0.0);
        let plan = store.commit(0, &settings()).unwrap();
        assert!(
            plan.lines[0].starts_with("[PlatMon] spend user=\"u\\n1\" amount=10 balanceAfter=0 source=STORE for=sku_1")
        );
        let test = EconomyLedger::new("u", "t", Scope::of(EconomySource::OperatorTest), vec![], HashSet::new());
        assert!(test.commit(0, &settings()).is_none());
    }
}
