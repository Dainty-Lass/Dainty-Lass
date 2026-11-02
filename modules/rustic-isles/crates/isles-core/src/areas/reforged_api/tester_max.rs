//! `controllers/testerMaxCore.ts` and `controllers/testerMax.ts`: `POST /admin/v1/players/:userId/tester-max`.
//!
//! Operator-triggered test funding for accounts holding the `tester` role. Validates the confirmation string
//! (`MAX <displayName>`), the idempotent `runId`, and `tester_max_kit.json`, then applies the kit's progression
//! tracks, inventory stacks, and currencies through the authoritative progression and inventory ledgers with economy
//! source `OPERATOR_TEST`.

use std::collections::{BTreeMap, HashSet};
use std::sync::{LazyLock, Mutex, PoisonError};

use axum::response::Response;
use serde_json::{Map, Value, json};

use super::admin::{Ctx, TESTER_ROLE, actor, error, find_target, internal, not_found, route};
use super::util::{self, sha256_hex};
use crate::areas::bounties_store::platinum::PLATINUM_AGGREGATE_ID;
use crate::areas::inventory::ports::Ports as InventoryPorts;
use crate::areas::inventory::storage_policy::StoragePolicy;
use crate::areas::inventory::transaction::{
    Economy, TxContext, TxOptions, TxOutcome, TxRequest, player_actor, run_inventory_transaction,
};
use crate::areas::progression::tracks::{
    MAX_PROGRESSION_GRANT_AMOUNT, curves, grant_request_hash, is_approved_xp_track, track_cap_xp,
};
use crate::db::IslesStore;
use crate::http::{Handlers, Shared};
use crate::ids::AccountId;
use crate::repo::RepoError;
use crate::repo::progression::{ProgressionGrantRecord, now_iso};
use crate::repo::reforged::AdminAuditRecord;
use crate::routes::{Method, Service};

pub const TESTER_MAX_ECONOMY_SOURCE: &str = "OPERATOR_TEST";
pub const TESTER_MAX_KIT_FILE: &str = "tester_max_kit.json";

const MAX_PROGRESSION_ENTRIES: usize = 50;
const MAX_TARGET_XP: i64 = 10_000_000;
const MAX_INVENTORY_ENTRIES: usize = 200;
const MAX_ITEM_QUANTITY: i64 = 1_000;
const MAX_CURRENCY_ENTRIES: usize = 50;
const MAX_CURRENCY_QUANTITY: i64 = 10_000_000;
const KIT_SECTIONS: [&str; 4] = ["comment", "progression", "inventory", "currencies"];

static RUNS_IN_PROGRESS: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetProgress {
    Max,
    Amount(i64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KitProgressionEntry {
    pub progression_id: String,
    pub target_progress: TargetProgress,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KitStackEntry {
    pub catalog_id: String,
    pub quantity: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TesterMaxKit {
    pub comment: Option<String>,
    pub progression: Vec<KitProgressionEntry>,
    pub inventory: Vec<KitStackEntry>,
    pub currencies: Vec<KitStackEntry>,
}

pub trait TesterMaxKitChecks: Send + Sync {
    fn is_approved_xp_track(&self, progression_id: &str) -> bool;
    fn track_cap_xp(&self, progression_id: &str) -> Option<i64>;
    fn rank_for_progress(&self, progression_id: &str, progress: i64) -> Option<i64>;
    fn is_known_catalog_id(&self, catalog_id: &str) -> bool;
    fn is_platinum_aggregate(&self, catalog_id: &str) -> bool {
        catalog_id == PLATINUM_AGGREGATE_ID
    }
    fn max_xp_grant(&self) -> i64 {
        MAX_PROGRESSION_GRANT_AMOUNT
    }
}

pub struct DefaultTesterMaxChecks<'a> {
    shared: &'a Shared,
    policy: Option<std::sync::Arc<StoragePolicy>>,
}

impl<'a> DefaultTesterMaxChecks<'a> {
    pub fn new(shared: &'a Shared) -> Self {
        let policy = shared
            .game_data
            .backend_file("inventory_storage_policy.json")
            .and_then(|path| StoragePolicy::load(&path).ok());
        Self { shared, policy }
    }
}

impl TesterMaxKitChecks for DefaultTesterMaxChecks<'_> {
    fn is_approved_xp_track(&self, progression_id: &str) -> bool {
        is_approved_xp_track(self.shared, progression_id)
    }

    fn track_cap_xp(&self, progression_id: &str) -> Option<i64> {
        track_cap_xp(self.shared, progression_id).map(|cap| cap as i64)
    }

    fn rank_for_progress(&self, progression_id: &str, progress: i64) -> Option<i64> {
        curves(self.shared).rank_for_progress(progression_id, progress as f64)
    }

    fn is_known_catalog_id(&self, catalog_id: &str) -> bool {
        self.policy
            .as_ref()
            .and_then(|p| p.entry(Some(&Value::String(catalog_id.to_owned()))))
            .is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XpGrantStep {
    pub index: i64,
    pub amount: i64,
}

/// `^[A-Za-z0-9][A-Za-z0-9._-]{7,63}$`
pub fn is_valid_tester_max_run_id(run_id: &str) -> bool {
    let bytes = run_id.as_bytes();
    (8..=64).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes[1..]
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// `DeriveTesterMaxId`: `tmax-` and 40 hex characters.
pub fn derive_tester_max_id(run_id: &str, part: &str) -> String {
    let digest = sha256_hex(format!("{run_id}\0{part}").as_bytes());
    format!("tmax-{}", &digest[..40])
}

/// `PlanXpGrants`: splits `[current, target)` into blocks of `max_grant`.
pub fn plan_xp_grants(current: i64, target: i64, max_grant: i64) -> Result<Vec<XpGrantStep>, &'static str> {
    if max_grant < 1 {
        return Err("maxGrant must be a positive integer.");
    }
    let mut steps = Vec::new();
    let mut start = current.max(0);
    let end = target;
    while start < end {
        let index = start / max_grant;
        let stop = end.min((index + 1) * max_grant);
        steps.push(XpGrantStep {
            index,
            amount: stop - start,
        });
        start = stop;
    }
    Ok(steps)
}

/// `ValidateTesterMaxKit`.
pub fn validate_tester_max_kit(kit: &Value, checks: &dyn TesterMaxKitChecks) -> Vec<String> {
    let Some(obj) = kit.as_object() else {
        return vec!["the kit must be a JSON object".into()];
    };
    let mut problems = Vec::new();
    for key in obj.keys() {
        if key == "slayersPath" {
            problems.push("slayersPath was removed (16 Oct): a tester unlocks the Slayer's Path in game".into());
        } else if !KIT_SECTIONS.contains(&key.as_str()) {
            problems.push(format!("unknown section \"{key}\""));
        }
    }
    if let Some(comment) = obj.get("comment")
        && !comment.is_null()
        && !comment.is_string()
    {
        problems.push("comment must be a string".into());
    }

    if let Some(prog) = obj.get("progression")
        && !prog.is_null()
    {
        match prog.as_array() {
            Some(list) if list.len() <= MAX_PROGRESSION_ENTRIES => {
                let mut seen = HashSet::new();
                for (index, entry) in list.iter().enumerate() {
                    let at = format!("progression[{index}]");
                    let row = entry.as_object();
                    let id_val = row.and_then(|r| r.get("progressionId"));
                    let id_str = id_val.and_then(Value::as_str);
                    match id_str {
                        Some(id) if checks.is_approved_xp_track(id) => {
                            if !seen.insert(id.to_owned()) {
                                problems.push(format!("{at}: {id} is listed twice"));
                            }
                        }
                        _ => {
                            let disp = service_kit::js::string(id_val);
                            problems.push(format!(
                                "{at}: \"{disp}\" is not an approved XP track (progression config)"
                            ));
                        }
                    }
                    let target = row.and_then(|r| r.get("targetProgress"));
                    if target.and_then(Value::as_str) == Some("max") {
                        if let Some(id) = id_str
                            && checks.track_cap_xp(id).is_none()
                        {
                            problems.push(format!(
                                "{at}: {id} has no top rank, so \"max\" is not defined for it; give a number"
                            ));
                        }
                    } else if !service_kit::js::safe_integer(target).is_some_and(|n| (1..=MAX_TARGET_XP).contains(&n)) {
                        problems.push(format!(
                            "{at}: targetProgress must be \"max\" or an integer from 1 to {MAX_TARGET_XP}"
                        ));
                    }
                }
            }
            _ => {
                problems.push(format!(
                    "progression must be an array of at most {MAX_PROGRESSION_ENTRIES} entries"
                ));
            }
        }
    }

    if let Some(inv) = obj.get("inventory")
        && !inv.is_null()
    {
        match inv.as_array() {
            Some(list) if list.len() <= MAX_INVENTORY_ENTRIES => {
                for (index, entry) in list.iter().enumerate() {
                    let at = format!("inventory[{index}]");
                    let row = entry.as_object();
                    let id_val = row.and_then(|r| r.get("catalogId"));
                    let id_str = id_val.and_then(Value::as_str);
                    match id_str {
                        Some(id) if checks.is_known_catalog_id(id) => {
                            if id.starts_with("CURRENCY_") {
                                problems.push(format!("{at}: {id} is a currency; list it under currencies"));
                            }
                        }
                        _ => {
                            let disp = service_kit::js::string(id_val);
                            problems.push(format!(
                                "{at}: \"{disp}\" is not in the inventory storage policy (do not invent catalog ids)"
                            ));
                        }
                    }
                    let qty = row.and_then(|r| r.get("quantity"));
                    if !service_kit::js::safe_integer(qty).is_some_and(|q| (1..=MAX_ITEM_QUANTITY).contains(&q)) {
                        problems.push(format!(
                            "{at}: quantity must be an integer from 1 to {MAX_ITEM_QUANTITY}"
                        ));
                    }
                }
            }
            _ => {
                problems.push(format!(
                    "inventory must be an array of at most {MAX_INVENTORY_ENTRIES} entries"
                ));
            }
        }
    }

    if let Some(cur) = obj.get("currencies")
        && !cur.is_null()
    {
        match cur.as_array() {
            Some(list) if list.len() <= MAX_CURRENCY_ENTRIES => {
                for (index, entry) in list.iter().enumerate() {
                    let at = format!("currencies[{index}]");
                    let row = entry.as_object();
                    let id_val = row.and_then(|r| r.get("catalogId"));
                    let id_str = id_val.and_then(Value::as_str);
                    match id_str {
                        Some(id) if id.starts_with("CURRENCY_") && checks.is_known_catalog_id(id) => {
                            if checks.is_platinum_aggregate(id) {
                                problems.push(format!(
                                    "{at}: {id} is the Platinum aggregate; grant a storefront bucket (CURRENCY_PLATINUM_*) instead"
                                ));
                            }
                        }
                        _ => {
                            let disp = service_kit::js::string(id_val);
                            problems.push(format!("{at}: \"{disp}\" is not a known CURRENCY_* id"));
                        }
                    }
                    let qty = row.and_then(|r| r.get("quantity"));
                    if !service_kit::js::safe_integer(qty).is_some_and(|q| (1..=MAX_CURRENCY_QUANTITY).contains(&q)) {
                        problems.push(format!(
                            "{at}: quantity must be an integer from 1 to {MAX_CURRENCY_QUANTITY}"
                        ));
                    }
                }
            }
            _ => {
                problems.push(format!(
                    "currencies must be an array of at most {MAX_CURRENCY_ENTRIES} entries"
                ));
            }
        }
    }

    problems
}

pub fn parse_tester_max_kit(raw: &Value) -> TesterMaxKit {
    let Some(obj) = raw.as_object() else {
        return TesterMaxKit::default();
    };
    let comment = obj.get("comment").and_then(Value::as_str).map(str::to_owned);
    let progression = obj
        .get("progression")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|item| {
                    let row = item.as_object()?;
                    let progression_id = row.get("progressionId")?.as_str()?.to_owned();
                    let target_progress = match row.get("targetProgress") {
                        Some(Value::String(s)) if s == "max" => TargetProgress::Max,
                        other => TargetProgress::Amount(service_kit::js::safe_integer(other)?),
                    };
                    Some(KitProgressionEntry {
                        progression_id,
                        target_progress,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let parse_stacks = |key: &str| -> Vec<KitStackEntry> {
        obj.get(key)
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|item| {
                        let row = item.as_object()?;
                        Some(KitStackEntry {
                            catalog_id: row.get("catalogId")?.as_str()?.to_owned(),
                            quantity: service_kit::js::safe_integer(row.get("quantity"))?,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    TesterMaxKit {
        comment,
        progression,
        inventory: parse_stacks("inventory"),
        currencies: parse_stacks("currencies"),
    }
}

enum LoadedKit {
    Missing,
    Problem(String),
    Kit(Value),
}

fn load_tester_max_kit(shared: &Shared) -> LoadedKit {
    let Some(path) = shared.game_data.backend_file(TESTER_MAX_KIT_FILE) else {
        return LoadedKit::Missing;
    };
    if !path.is_file() {
        return LoadedKit::Missing;
    }
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return LoadedKit::Missing;
    };
    match serde_json::from_str::<Value>(raw.trim_start_matches('\u{feff}')) {
        Ok(kit) => LoadedKit::Kit(kit),
        Err(_) => LoadedKit::Problem(format!("{TESTER_MAX_KIT_FILE} is not valid JSON")),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TesterMaxRefusal {
    pub code: &'static str,
    pub message: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TesterMaxProgressionReport {
    pub progression_id: String,
    pub target: i64,
    pub start_progress: i64,
    pub granted: i64,
    pub grants: usize,
    pub confirmed_rank: Option<i64>,
    pub status: &'static str,
    pub error: Option<String>,
}

impl TesterMaxProgressionReport {
    pub fn to_json(&self) -> Value {
        let mut map = Map::new();
        map.insert("progressionId".into(), json!(self.progression_id));
        map.insert("target".into(), json!(self.target));
        map.insert("startProgress".into(), json!(self.start_progress));
        map.insert("granted".into(), json!(self.granted));
        map.insert("grants".into(), json!(self.grants));
        if let Some(rank) = self.confirmed_rank {
            map.insert("confirmedRank".into(), json!(rank));
        }
        map.insert("status".into(), json!(self.status));
        if let Some(err) = &self.error {
            map.insert("error".into(), json!(err));
        }
        Value::Object(map)
    }

    pub fn to_summary_json(&self) -> Value {
        json!({
            "progressionId": self.progression_id,
            "target": self.target,
            "startProgress": self.start_progress,
            "granted": self.granted,
            "grants": self.grants,
            "confirmedRank": self.confirmed_rank,
            "status": self.status,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TesterMaxItemsReport {
    pub status: &'static str,
    pub error: Option<String>,
    pub stacks: usize,
}

impl TesterMaxItemsReport {
    pub fn to_json(&self) -> Value {
        let mut map = Map::new();
        map.insert("status".into(), json!(self.status));
        if let Some(err) = &self.error {
            map.insert("error".into(), json!(err));
        }
        map.insert("stacks".into(), json!(self.stacks));
        Value::Object(map)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TesterMaxReport {
    pub run_id: String,
    pub user_id: String,
    pub character_id: String,
    pub status: &'static str,
    pub progression: Vec<TesterMaxProgressionReport>,
    pub items: Option<TesterMaxItemsReport>,
}

impl TesterMaxReport {
    pub fn to_json(&self) -> Value {
        let mut map = Map::new();
        map.insert("runId".into(), json!(self.run_id));
        map.insert("userId".into(), json!(self.user_id));
        map.insert("characterId".into(), json!(self.character_id));
        map.insert("status".into(), json!(self.status));
        map.insert(
            "progression".into(),
            Value::Array(
                self.progression
                    .iter()
                    .map(TesterMaxProgressionReport::to_json)
                    .collect(),
            ),
        );
        if let Some(items) = &self.items {
            map.insert("items".into(), items.to_json());
        }
        Value::Object(map)
    }

    pub fn to_summary_json(&self) -> Value {
        let mut map = Map::new();
        map.insert("runId".into(), json!(self.run_id));
        map.insert("status".into(), json!(self.status));
        map.insert(
            "progression".into(),
            Value::Array(
                self.progression
                    .iter()
                    .map(TesterMaxProgressionReport::to_summary_json)
                    .collect(),
            ),
        );
        if let Some(items) = &self.items {
            map.insert(
                "items".into(),
                json!({
                    "status": items.status,
                    "stacks": items.stacks,
                }),
            );
        }
        Value::Object(map)
    }
}

fn describe_err(text: impl std::fmt::Display) -> String {
    text.to_string().chars().take(200).collect()
}

async fn grant_xp_step(
    shared: &Shared,
    store: &IslesStore,
    account: &AccountId,
    user_id: &str,
    progression_id: &str,
    amount: i64,
    grant_id: &str,
) -> Result<(), String> {
    if !is_approved_xp_track(shared, progression_id) {
        return Err(format!("Unsupported progression track '{progression_id}'."));
    }
    let hash = grant_request_hash(user_id, progression_id, amount);
    let owner = account.clone();
    let track = progression_id.to_owned();
    let gid = grant_id.to_owned();
    let actor = player_actor(account, None);
    store
        .account_write(account, &actor, move |tx, repos| {
            if let Some(existing) = repos.progression_grants.find_by_grant_id(tx.conn(), &gid)? {
                if existing.request_hash != hash
                    || existing.account != owner
                    || existing.progression_id != track
                    || existing.amount != amount
                {
                    return Err(RepoError::Data(format!(
                        "Progression grant {gid} was already used with a different request."
                    )));
                }
                return Ok(());
            }
            let after = repos.progression_tracks.increment(tx, &owner, &track, amount)?;
            repos.progression_grants.insert_applied(
                tx,
                &ProgressionGrantRecord {
                    grant_id: gid,
                    account: owner,
                    progression_id: track,
                    amount,
                    request_hash: hash,
                    result: after,
                    created_at: now_iso(),
                },
            )?;
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())
}

async fn confirm_rank_step(
    shared: &Shared,
    store: &IslesStore,
    account: &AccountId,
    progression_id: &str,
    rank: i64,
) -> Result<(), String> {
    let curves = curves(shared);
    let Some(curve) = curves.get(progression_id) else {
        return Err(format!("rank {rank} confirm refused: unknown_track"));
    };
    let Some(required) = curve.required_xp_for_rank(rank) else {
        return Err(format!("rank {rank} confirm refused: rank_out_of_range"));
    };
    let owner = account.clone();
    let track = progression_id.to_owned();
    let actor = player_actor(account, None);
    store
        .account_write(account, &actor, move |tx, repos| {
            if required > 0.0 {
                let persisted = repos
                    .progression_tracks
                    .get(tx.conn(), &owner, &track)?
                    .map_or(0, |t| t.progress);
                if (persisted as f64) < required {
                    return Err(RepoError::Data(format!("rank {rank} confirm refused: insufficient_xp")));
                }
            }
            let _ = repos
                .progression_tracks
                .set_confirmed_fremium_rank(tx, &owner, &track, rank)?;
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())
}

pub async fn run_tester_max(
    shared: &std::sync::Arc<Shared>,
    store: &IslesStore,
    account: &AccountId,
    user_id: &str,
    run_id: &str,
    kit: &TesterMaxKit,
    checks: &dyn TesterMaxKitChecks,
) -> Result<TesterMaxReport, TesterMaxRefusal> {
    if !is_valid_tester_max_run_id(run_id) {
        return Err(TesterMaxRefusal {
            code: "ADMIN_TESTER_MAX_RUN_ID",
            message: "runId must be 8 to 64 characters: letters, digits, dot, underscore or hyphen.",
        });
    }
    let owner = account.clone();
    let characters = store
        .read(move |conn, repos| repos.characters.find_many_by_account(conn, &owner))
        .await
        .unwrap_or_default();
    let Some(character_id) = characters
        .into_iter()
        .find(|c| !c.character_id.trim().is_empty())
        .map(|c| c.character_id)
    else {
        return Err(TesterMaxRefusal {
            code: "ADMIN_TESTER_MAX_NO_CHARACTER",
            message: "The tester has no character yet; the account must sign in to the game once first.",
        });
    };

    let mut report = TesterMaxReport {
        run_id: run_id.to_owned(),
        user_id: user_id.to_owned(),
        character_id: character_id.clone(),
        status: "done",
        progression: Vec::new(),
        items: None,
    };

    for entry in &kit.progression {
        let mut row = TesterMaxProgressionReport {
            progression_id: entry.progression_id.clone(),
            target: 0,
            start_progress: 0,
            granted: 0,
            grants: 0,
            confirmed_rank: None,
            status: "done",
            error: None,
        };
        let step_result: Result<(), String> = async {
            let owner = account.clone();
            let pid = entry.progression_id.clone();
            let start = store
                .read(move |conn, repos| repos.progression_tracks.get(conn, &owner, &pid))
                .await
                .map_err(|e| e.to_string())?
                .map_or(0, |r| r.progress);
            let target = match entry.target_progress {
                TargetProgress::Max => checks
                    .track_cap_xp(&entry.progression_id)
                    .ok_or_else(|| "the track has no top rank".to_owned())?,
                TargetProgress::Amount(amount) => amount,
            };
            row.start_progress = start;
            row.target = target;
            for step in plan_xp_grants(start, target, checks.max_xp_grant()).map_err(str::to_owned)? {
                let grant_id = derive_tester_max_id(run_id, &format!("xp:{}:{}", entry.progression_id, step.index));
                grant_xp_step(
                    shared,
                    store,
                    account,
                    user_id,
                    &entry.progression_id,
                    step.amount,
                    &grant_id,
                )
                .await?;
                row.granted += step.amount;
                row.grants += 1;
            }
            if let Some(reached) = checks
                .rank_for_progress(&entry.progression_id, start.max(target))
                .filter(|r| *r > 0)
            {
                confirm_rank_step(shared, store, account, &entry.progression_id, reached).await?;
                row.confirmed_rank = Some(reached);
            }
            Ok(())
        }
        .await;

        if let Err(err) = step_result {
            row.status = "error";
            row.error = Some(describe_err(err));
            report.status = "partial";
        }
        report.progression.push(row);
    }

    let mut stacks: BTreeMap<String, i64> = BTreeMap::new();
    for entry in kit.inventory.iter().chain(kit.currencies.iter()) {
        *stacks.entry(entry.catalog_id.clone()).or_insert(0) += entry.quantity;
    }
    if !stacks.is_empty() {
        let adds_count = stacks.len();
        let add_stacked: Vec<Value> = stacks
            .into_iter()
            .map(|(catalog_id, quantity)| json!({ "catalogId": catalog_id, "quantity": quantity }))
            .collect();
        let ports = InventoryPorts::default();
        let tx_ctx = TxContext {
            shared,
            store,
            ports: &ports,
        };
        let tx_id = derive_tester_max_id(run_id, "items");
        let outcome = run_inventory_transaction(
            &tx_ctx,
            TxRequest {
                user_id: user_id.to_owned(),
                character_id: character_id.clone(),
                transaction_id: tx_id,
                add_instanced: Value::Array(Vec::new()),
                add_stacked: Value::Array(add_stacked),
                remove_instanced: Value::Array(Vec::new()),
                remove_stacked: Value::Array(Vec::new()),
                save_instanced: Value::Array(Vec::new()),
            },
            TxOptions {
                ownership_already_validated: true,
                operator_test_funding: true,
                economy: Some(Economy {
                    source: TESTER_MAX_ECONOMY_SOURCE.into(),
                    p2p_session_id: None,
                    store_sku_id: None,
                }),
                ..TxOptions::default()
            },
            None,
        )
        .await;
        match outcome {
            Ok(TxOutcome::Done(_)) => {
                report.items = Some(TesterMaxItemsReport {
                    status: "done",
                    error: None,
                    stacks: adds_count,
                });
            }
            Ok(TxOutcome::NotOwned) => {
                report.items = Some(TesterMaxItemsReport {
                    status: "error",
                    error: Some("the inventory transaction did not apply".into()),
                    stacks: adds_count,
                });
                report.status = "partial";
            }
            Err(err) => {
                report.items = Some(TesterMaxItemsReport {
                    status: "error",
                    error: Some(describe_err(err)),
                    stacks: adds_count,
                });
                report.status = "partial";
            }
        }
    }

    Ok(report)
}

async fn post_tester_max(ctx: Ctx) -> Response {
    let caller = ctx.caller().clone();
    let target_user = ctx.params.get("userId").unwrap_or_default().to_owned();
    let lookup = target_user.clone();
    let found = match ctx
        .store
        .read(move |conn, repos| find_target(conn, repos, &lookup))
        .await
    {
        Ok(found) => found,
        Err(e) => return internal(&e),
    };
    let Some((account_id, target)) = found else {
        return not_found();
    };
    if !target.roles.iter().any(|r| r == TESTER_ROLE) {
        return error(
            409,
            "ADMIN_TESTER_REQUIRED",
            "Only an account with the tester role can be maxed.",
        );
    }
    let fields = ctx.fields();
    let expected = format!("MAX {}", target.display_name);
    if fields.get("confirm").and_then(Value::as_str) != Some(expected.as_str()) {
        return error(
            400,
            "ADMIN_TESTER_MAX_CONFIRMATION",
            &format!("Type \"{expected}\" to confirm."),
        );
    }
    let Some(run_id) = fields
        .get("runId")
        .and_then(Value::as_str)
        .filter(|id| is_valid_tester_max_run_id(id))
        .map(str::to_owned)
    else {
        return error(
            400,
            "ADMIN_TESTER_MAX_RUN_ID",
            "runId must be 8 to 64 characters: letters, digits, dot, underscore or hyphen.",
        );
    };

    let raw_kit = match load_tester_max_kit(&ctx.shared) {
        LoadedKit::Problem(problem) => {
            return util::json(
                422,
                &json!({
                    "error": {
                        "code": "ADMIN_TESTER_MAX_KIT_INVALID",
                        "message": "The tester kit cannot run.",
                        "problems": [problem],
                    }
                }),
            );
        }
        LoadedKit::Missing => {
            return error(
                503,
                "ADMIN_TESTER_MAX_KIT_MISSING",
                "Copy game-data/tester_max_kit.example.json to tester_max_kit.json and fill it in.",
            );
        }
        LoadedKit::Kit(kit) => kit,
    };

    let checks = DefaultTesterMaxChecks::new(&ctx.shared);
    let problems = validate_tester_max_kit(&raw_kit, &checks);
    if !problems.is_empty() {
        return util::json(
            422,
            &json!({
                "error": {
                    "code": "ADMIN_TESTER_MAX_KIT_INVALID",
                    "message": "The tester kit cannot run.",
                    "problems": problems,
                }
            }),
        );
    }

    {
        let mut active = RUNS_IN_PROGRESS.lock().unwrap_or_else(PoisonError::into_inner);
        if !active.insert(target_user.clone()) {
            return error(
                409,
                "ADMIN_TESTER_MAX_BUSY",
                "A max run for this account is already in progress.",
            );
        }
    }

    let parsed_kit = parse_tester_max_kit(&raw_kit);
    let request_id = uuid::Uuid::new_v4().to_string();
    let outcome = run_tester_max(
        &ctx.shared,
        &ctx.store,
        &account_id,
        &target_user,
        &run_id,
        &parsed_kit,
        &checks,
    )
    .await;

    RUNS_IN_PROGRESS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&target_user);

    let (action, new_state) = match &outcome {
        Ok(report) => ("player.tester_max", report.to_summary_json()),
        Err(refusal) => ("player.tester_max.failed", json!({ "refused": refusal.code })),
    };
    let audit = AdminAuditRecord {
        id: uuid::Uuid::new_v4().to_string(),
        actor_user_id: caller.user_id.clone(),
        target_user_id: Some(target_user.clone()),
        action: action.to_owned(),
        old_state: Some(json!({ "roles": target.roles })),
        new_state: Some(new_state),
        reason: Some(format!("tester max run {run_id}")),
        ip: ctx.req.ip(),
        request_id: request_id.clone(),
        created_at: util::now_iso(),
    };
    let _ = ctx
        .store
        .write(&actor(&caller), move |tx, repos| repos.admin.append_audit(tx, &audit))
        .await;

    match outcome {
        Ok(report) => {
            tracing::info!(
                "[TesterMax] run={run_id} user={target_user} status={} actor={}",
                report.status,
                caller.user_id
            );
            util::json(
                200,
                &json!({
                    "requestId": request_id,
                    "report": report.to_json(),
                }),
            )
        }
        Err(refusal) => {
            tracing::warn!("[TesterMax] run={run_id} user={target_user} refused: {}", refusal.code);
            util::json(
                409,
                &json!({
                    "error": {
                        "code": refusal.code,
                        "message": refusal.message,
                    },
                    "requestId": request_id,
                }),
            )
        }
    }
}

pub fn register(handlers: &mut Handlers) {
    handlers.on(
        Service::Backend,
        Method::Post,
        "/admin/v1/players/:userId/tester-max",
        |call| Box::pin(route(call, true, post_tester_max)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeChecks;

    impl TesterMaxKitChecks for FakeChecks {
        fn is_approved_xp_track(&self, id: &str) -> bool {
            id.starts_with("ExperienceTrack_")
        }

        fn track_cap_xp(&self, id: &str) -> Option<i64> {
            (id != "ExperienceTrack_PrestigeLess").then_some(27_775)
        }

        fn rank_for_progress(&self, _id: &str, progress: i64) -> Option<i64> {
            Some(if progress >= 27_775 { 20 } else { progress / 10_000 })
        }

        fn is_known_catalog_id(&self, id: &str) -> bool {
            matches!(
                id,
                "AC_HEAD_ARMSTRONG_00"
                    | "CURRENCY_NOTES"
                    | "CURRENCY_PLATINUM"
                    | "CURRENCY_PLATINUM_CS"
                    | "CURRENCY_SEASONAL_COIN"
            )
        }
    }

    #[test]
    fn xp_grants_plan_in_absolute_blocks_and_resume_on_the_same_ids() {
        let first = plan_xp_grants(0, 250_000, 100_000).unwrap();
        assert_eq!(
            first,
            vec![
                XpGrantStep {
                    index: 0,
                    amount: 100_000
                },
                XpGrantStep {
                    index: 1,
                    amount: 100_000
                },
                XpGrantStep {
                    index: 2,
                    amount: 50_000
                },
            ]
        );
        let resumed = plan_xp_grants(200_000, 250_000, 100_000).unwrap();
        assert_eq!(resumed, vec![first[2].clone()]);
        assert_eq!(plan_xp_grants(250_000, 250_000, 100_000).unwrap(), vec![]);
        assert_eq!(
            plan_xp_grants(1_234, 100_000, 100_000).unwrap(),
            vec![XpGrantStep {
                index: 0,
                amount: 98_766
            }]
        );
    }

    #[test]
    fn derived_ids_and_run_ids_match_node_rules() {
        let a = derive_tester_max_id("run-2026-10-14-a", "xp:ExperienceTrack_PlayerLevel:0");
        assert_eq!(
            a,
            derive_tester_max_id("run-2026-10-14-a", "xp:ExperienceTrack_PlayerLevel:0")
        );
        assert_eq!(a.len(), 45);
        assert!(a.starts_with("tmax-"));
        assert!(is_valid_tester_max_run_id("abcdefgh"));
        assert!(!is_valid_tester_max_run_id("abcdefg"));
        assert!(!is_valid_tester_max_run_id("-leading-hyphen"));
    }

    #[test]
    fn kit_validation_reports_unknown_sections_slayers_path_and_invalid_entries() {
        let good = json!({
            "comment": "test kit",
            "progression": [
                { "progressionId": "ExperienceTrack_PlayerLevel", "targetProgress": "max" },
                { "progressionId": "ExperienceTrack_Weapon_Sword", "targetProgress": 5000 }
            ],
            "inventory": [{ "catalogId": "AC_HEAD_ARMSTRONG_00", "quantity": 1 }],
            "currencies": [
                { "catalogId": "CURRENCY_NOTES", "quantity": 1_000_000 },
                { "catalogId": "CURRENCY_PLATINUM_CS", "quantity": 500 }
            ]
        });
        assert!(validate_tester_max_kit(&good, &FakeChecks).is_empty());
        assert_eq!(
            validate_tester_max_kit(&json!({ "slayersPath": { "unlockAll": true } }), &FakeChecks),
            vec!["slayersPath was removed (16 Oct): a tester unlocks the Slayer's Path in game"]
        );
        let currency_problems = validate_tester_max_kit(
            &json!({
                "currencies": [
                    { "catalogId": "CURRENCY_PLATINUM", "quantity": 10 },
                    { "catalogId": "CURRENCY_SEASONAL_COIN", "quantity": 10 }
                ]
            }),
            &FakeChecks,
        );
        assert_eq!(currency_problems.len(), 1);
        assert!(currency_problems[0].contains("Platinum aggregate"));
    }
}
