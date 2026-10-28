//! The Slayer's Path (Player Journey Map): `controllers/slayersPath.ts` and the journey half of
//! `controllers/progression.ts`. Node ids are JSON object keys in the order the game sent them, so the maps here are
//! `serde_json::Map` (insertion ordered), and the merge mirrors JavaScript's object spread.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use module_api::BoxFuture;
use serde_json::{Map, Value, json};

use crate::AccountId;
use crate::http::Shared;

use super::tracks::{read_game_data, sha256_hex};

pub const GAUNTLET_UNLOCK_NODE: &str = "Gauntlet_Unlock";

fn as_record(value: &Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

/// `IsNodeUnlocked`: `node_status` 1 (LockedButClaimable) or 2 (Unlocked), compared strictly.
pub fn is_node_unlocked(node: Option<&Value>) -> bool {
    node.and_then(|n| n.get("node_status"))
        .and_then(Value::as_f64)
        .is_some_and(|s| s == 1.0 || s == 2.0)
}

/// `GetUnlockedNodeIds`: the node's own `node_id` when it has one, else its key.
pub fn unlocked_node_ids(nodes: &Value) -> Vec<String> {
    let Some(nodes) = nodes.as_object() else {
        return Vec::new();
    };
    nodes
        .iter()
        .filter(|(_, node)| is_node_unlocked(Some(node)))
        .map(|(key, node)| match node.get("node_id").and_then(Value::as_str) {
            Some(id) if !id.is_empty() => id.to_owned(),
            _ => key.clone(),
        })
        .collect()
}

/// `ComputeNewlyUnlockedNodeIds`.
pub fn newly_unlocked_node_ids(stored: &Value, next: &Value) -> Vec<String> {
    let stored = stored.as_object();
    next.as_object()
        .into_iter()
        .flatten()
        .filter(|(key, node)| is_node_unlocked(Some(node)) && !is_node_unlocked(stored.and_then(|s| s.get(*key))))
        .map(|(key, _)| key.clone())
        .collect()
}

/// `MergePlayerJourneyNodes`: a stale save may only add nodes, a newer one wins per node, and neither drops one.
pub fn merge_nodes(
    stored: &Value,
    incoming: &Value,
    incoming_version: f64,
    stored_version: f64,
) -> (Map<String, Value>, f64) {
    let stored = as_record(stored);
    let incoming = as_record(incoming);
    let stale = incoming_version <= stored_version;
    // `{...a, ...b}`: a's keys in a's order, then b's new keys; b's values win
    let spread = |first: &Map<String, Value>, second: &Map<String, Value>| {
        let mut out = first.clone();
        for (key, value) in second {
            out.insert(key.clone(), value.clone());
        }
        out
    };
    let nodes = if stale {
        spread(&incoming, &stored)
    } else {
        spread(&stored, &incoming)
    };
    (nodes, if stale { stored_version } else { incoming_version })
}

// ---------------------------------------------------------------------------------------------------------------
// Definitions and the graph

pub type Definitions = Map<String, Value>;

type Cache<T> = Mutex<HashMap<PathBuf, Arc<T>>>;

/// `slayers_path_definitions.json`, read once; a missing file gives no definitions (no rewards, no host unlocks).
pub fn definitions(shared: &Shared) -> Arc<Definitions> {
    static CACHE: OnceLock<Cache<Definitions>> = OnceLock::new();
    let Some(path) = shared.game_data.backend_file("slayers_path_definitions.json") else {
        return Arc::new(Definitions::new());
    };
    let Ok(mut held) = CACHE.get_or_init(Cache::default).lock() else {
        return Arc::new(Definitions::new());
    };
    if let Some(found) = held.get(&path) {
        return found.clone();
    }
    let loaded = match read_game_data(&path) {
        Ok(Value::Object(object)) => object,
        Ok(_) => Definitions::new(),
        Err(error) => {
            tracing::error!(
                "[slayers-path] FAILED to load slayers_path_definitions.json - node rewards will NOT be granted: {error}"
            );
            Definitions::new()
        }
    };
    let loaded = Arc::new(loaded);
    held.insert(path, loaded.clone());
    loaded
}

/// The served graph's node ids (`slayers_path.json`'s `payload.nodes`).
pub fn served_node_ids(shared: &Shared) -> Vec<String> {
    shared
        .game_data
        .backend_file("slayers_path.json")
        .and_then(|path| read_game_data(&path).ok())
        .and_then(|file| {
            file.pointer("/payload/nodes")?
                .as_object()
                .map(|n| n.keys().cloned().collect())
        })
        .unwrap_or_default()
}

/// `GetNodeRewardItemIds`: the export writes a literal "None" for an empty reward slot.
pub fn node_reward_item_ids(definitions: &Definitions, node_id: &str) -> Vec<String> {
    definitions
        .get(node_id)
        .and_then(|d| d.get("rewards")?.as_array())
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|id| !id.is_empty() && *id != "None")
        .map(str::to_owned)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SlayersPathGraph {
    pub parents: HashMap<String, Vec<String>>,
    pub known: HashSet<String>,
    /// False when no definitions loaded: nothing can be verified, so a host's unlocks are all refused
    pub usable: bool,
}

/// `BuildSlayersPathGraph`.
pub fn build_graph(definitions: &Definitions, served: &[String]) -> SlayersPathGraph {
    let mut parents: HashMap<String, Vec<String>> = HashMap::new();
    let mut known = HashSet::new();
    for (node_id, definition) in definitions {
        known.insert(node_id.clone());
        for child in definition
            .get("childNodes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(child) = child.as_str().filter(|c| !c.is_empty() && *c != "None") else {
                continue;
            };
            known.insert(child.to_owned());
            let held = parents.entry(child.to_owned()).or_default();
            if !held.contains(node_id) {
                held.push(node_id.clone());
            }
        }
    }
    for node_id in served.iter().filter(|id| !id.is_empty()) {
        known.insert(node_id.clone());
    }
    SlayersPathGraph {
        parents,
        known,
        usable: !definitions.is_empty(),
    }
}

/// The graph of this module's game data.
pub fn graph(shared: &Shared) -> SlayersPathGraph {
    build_graph(&definitions(shared), &served_node_ids(shared))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StripReason {
    UnknownNode,
    Prerequisite,
    NodeIdMismatch,
    DefinitionsUnavailable,
}

impl StripReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnknownNode => "unknown-node",
            Self::Prerequisite => "prerequisite",
            Self::NodeIdMismatch => "node-id-mismatch",
            Self::DefinitionsUnavailable => "definitions-unavailable",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HostJourneyPlan {
    /// The node map to store
    pub nodes: Map<String, Value>,
    /// Newly unlocked nodes accepted
    pub unlocked: Vec<String>,
    /// Newly unlocked nodes refused; the stored status is kept
    pub stripped: Vec<(String, StripReason)>,
    /// Nodes the host tried to lock again; the stored entry is kept
    pub relocked: Vec<String>,
}

/// `PlanHostPlayerJourney`: checks the journey map a player-hosted server sends for one of its players, after the
/// merge with the stored map, and returns the map to store.
pub fn plan_host_journey(stored: &Value, merged: &Value, graph: &SlayersPathGraph) -> HostJourneyPlan {
    let stored = as_record(stored);
    let mut merged = as_record(merged);
    let mut stripped: Vec<(String, StripReason)> = Vec::new();
    let mut relocked: Vec<String> = Vec::new();

    let snapshot: Vec<(String, Value)> = merged.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    for (node_id, node) in &snapshot {
        let own_id = node.get("node_id").and_then(Value::as_str);
        if !is_node_unlocked(Some(node)) || own_id.is_none() || own_id == Some(node_id.as_str()) {
            continue;
        }
        let before = stored.get(node_id);
        if before.is_some_and(|b| super::util::stringify(b) == super::util::stringify(node)) {
            continue;
        }
        stripped.push((node_id.clone(), StripReason::NodeIdMismatch));
        match before {
            Some(before) => {
                merged.insert(node_id.clone(), before.clone());
            }
            None => {
                merged.shift_remove(node_id);
            }
        }
    }

    for (node_id, node) in &stored {
        if is_node_unlocked(Some(node)) && !is_node_unlocked(merged.get(node_id)) {
            merged.insert(node_id.clone(), node.clone());
            relocked.push(node_id.clone());
        }
    }

    let mut pending: Vec<String> = Vec::new();
    let mut unlocked: HashSet<String> = HashSet::new();
    for (node_id, node) in &merged {
        if !is_node_unlocked(Some(node)) {
            continue;
        }
        if is_node_unlocked(stored.get(node_id)) {
            unlocked.insert(node_id.clone());
        } else {
            pending.push(node_id.clone());
        }
    }

    let mut refuse: Vec<(String, StripReason)> = Vec::new();
    let mut remaining: Vec<String> = Vec::new();
    for node_id in &pending {
        if !graph.usable {
            refuse.push((node_id.clone(), StripReason::DefinitionsUnavailable));
        } else if !graph.known.contains(node_id) {
            refuse.push((node_id.clone(), StripReason::UnknownNode));
        } else {
            remaining.push(node_id.clone());
        }
    }

    let mut accepted: Vec<String> = Vec::new();
    let mut progress = true;
    while progress && !remaining.is_empty() {
        progress = false;
        let mut next = Vec::new();
        for node_id in remaining {
            let parents = graph.parents.get(&node_id);
            if parents.is_none_or(|ps| ps.iter().any(|p| unlocked.contains(p))) {
                unlocked.insert(node_id.clone());
                accepted.push(node_id);
                progress = true;
            } else {
                next.push(node_id);
            }
        }
        remaining = next;
    }
    refuse.extend(remaining.into_iter().map(|id| (id, StripReason::Prerequisite)));

    for (node_id, _) in &refuse {
        let replacement = match stored.get(node_id) {
            Some(before) => before.clone(),
            None => {
                // `{ ...Merged.get(id), node_status: 0 }`
                let mut copy = merged.get(node_id).map(as_record).unwrap_or_default();
                copy.insert("node_status".into(), json!(0));
                Value::Object(copy)
            }
        };
        merged.insert(node_id.clone(), replacement);
    }

    stripped.extend(refuse);
    HostJourneyPlan {
        nodes: merged,
        unlocked: pending.into_iter().filter(|id| accepted.contains(id)).collect(),
        stripped,
        relocked,
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The masked node

/// `JourneyNodesShownLocked`: the table's masked nodes whose switch is not on (`MYSTICPARADOX_GAUNTLET`).
pub fn nodes_shown_locked(shared: &Shared) -> Vec<&'static str> {
    shared
        .version
        .tables
        .masked_journey_nodes
        .iter()
        .filter(|masked| {
            let on = shared
                .settings
                .raw(masked.unless)
                .is_some_and(|v| service_kit::js::trim(v).eq_ignore_ascii_case("on"));
            !on
        })
        .map(|masked| masked.node)
        .collect()
}

/// `MaskJourneyNodesForPlayer`: a copy for an answer, every shown-locked node that is present set to Locked (0).
pub fn mask_nodes(nodes: &Value, shown_locked: &[&str]) -> Map<String, Value> {
    let mut out = as_record(nodes);
    for node_id in shown_locked {
        if let Some(Value::Object(node)) = out.get(*node_id) {
            let mut node = node.clone();
            node.insert("node_status".into(), json!(0));
            out.insert((*node_id).to_owned(), Value::Object(node));
        }
    }
    out
}

/// `RestoreJourneyNodesShownLocked`: a shown-locked node unlocked in the stored map keeps its stored entry.
pub fn restore_shown_locked(stored: &Value, merged: &Value, shown_locked: &[&str]) -> Map<String, Value> {
    let mut out = as_record(merged);
    let stored = as_record(stored);
    for node_id in shown_locked {
        if let Some(held) = stored.get(*node_id).filter(|n| is_node_unlocked(Some(n))) {
            out.insert((*node_id).to_owned(), held.clone());
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------------------------
// Rewards

/// What the reward grant needs from the inventory and character areas (`GetCharactersForUid`,
/// `GetInventoryForUserIdAndCharacterId`, `RunInventoryTransaction`). inventory-reforge and characters-loadouts own
/// those; the integration pass wires them in with [`super::Ports`].
pub trait JourneyInventory: Send + Sync {
    /// The account's first character and its instanced items; `None` when it has no character.
    fn first_character<'a>(
        &'a self,
        shared: &'a std::sync::Arc<Shared>,
        account: &'a AccountId,
    ) -> BoxFuture<'a, FirstCharacter>;

    /// One idempotent inventory transaction: adds the instanced items and removes the given
    /// `(catalogId, instanceId)` ones. `true` when it applied (or already had).
    fn run_transaction<'a>(
        &'a self,
        shared: &'a std::sync::Arc<Shared>,
        account: &'a AccountId,
        transaction: RewardTransaction<'a>,
    ) -> BoxFuture<'a, Result<bool, String>>;
}

/// `Ok(None)` when the account has no character.
pub type FirstCharacter = Result<Option<(String, Vec<Value>)>, String>;

/// The arguments of `RunInventoryTransaction(userId, characterId, transactionId, add, [], remove, [], [])`.
#[derive(Debug, Clone, PartialEq)]
pub struct RewardTransaction<'a> {
    pub user_id: &'a str,
    pub character_id: &'a str,
    pub transaction_id: &'a str,
    pub add_instanced: Vec<Value>,
    pub remove_instanced: Vec<(String, String)>,
}

/// The stand-in until the inventory area is wired: rewards are not granted, and the next journey save retries.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnwiredInventory;

impl JourneyInventory for UnwiredInventory {
    fn first_character<'a>(&'a self, _: &'a std::sync::Arc<Shared>, _: &'a AccountId) -> BoxFuture<'a, FirstCharacter> {
        Box::pin(async { Err("the inventory area is not wired to progression yet".to_owned()) })
    }

    fn run_transaction<'a>(
        &'a self,
        _: &'a std::sync::Arc<Shared>,
        _: &'a AccountId,
        _: RewardTransaction<'a>,
    ) -> BoxFuture<'a, Result<bool, String>> {
        Box::pin(async { Err("the inventory area is not wired to progression yet".to_owned()) })
    }
}

/// The production [`JourneyInventory`] over `characters::controller::characters_for_user` and
/// `inventory::transaction::{get_inventory, run_inventory_transaction}`.
#[derive(Debug, Clone, Copy, Default)]
pub struct RepoJourneyInventory;

impl JourneyInventory for RepoJourneyInventory {
    fn first_character<'a>(
        &'a self,
        shared: &'a std::sync::Arc<Shared>,
        account: &'a AccountId,
    ) -> BoxFuture<'a, FirstCharacter> {
        Box::pin(async move {
            let Some(store) = shared.store.as_ref() else {
                return Ok(None);
            };
            let acc_lookup = account.clone();
            let user_id = store
                .read(move |conn, repos| {
                    Ok(repos
                        .accounts
                        .find(conn, &acc_lookup)?
                        .map(|a| a.user_id.0)
                        .unwrap_or_else(|| acc_lookup.to_string()))
                })
                .await
                .map_err(|e| e.to_string())?;
            let actor = module_api::ActorContext::new(module_api::Actor::Player {
                account: account.global().clone(),
            });
            let list = crate::areas::characters::controller::characters_for_user(
                store,
                &shared.version.tables,
                &actor,
                &user_id,
            )
            .await
            .map_err(|e| e.to_string())?;
            let Some(character_id) = list
                .first()
                .and_then(|c| c.get("id"))
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
            else {
                return Ok(None);
            };
            let ports = crate::areas::inventory::Ports::default();
            let ctx = crate::areas::inventory::transaction::TxContext {
                shared,
                store,
                ports: &ports,
            };
            let inv = crate::areas::inventory::transaction::get_inventory(&ctx, Some(&user_id), &character_id, None)
                .await
                .map_err(|e| e.to_string())?;
            let items = inv
                .and_then(|v| v.get("instancedItems").and_then(Value::as_array).cloned())
                .unwrap_or_default();
            Ok(Some((character_id, items)))
        })
    }

    fn run_transaction<'a>(
        &'a self,
        shared: &'a std::sync::Arc<Shared>,
        _account: &'a AccountId,
        transaction: RewardTransaction<'a>,
    ) -> BoxFuture<'a, Result<bool, String>> {
        Box::pin(async move {
            use crate::areas::inventory::transaction::{
                Economy, TxContext, TxOptions, TxOutcome, TxRequest, run_inventory_transaction,
            };
            let Some(store) = shared.store.as_ref() else {
                return Ok(false);
            };
            let ports = crate::areas::inventory::Ports::default();
            let ctx = TxContext {
                shared,
                store,
                ports: &ports,
            };
            let remove_instanced: Vec<Value> = transaction
                .remove_instanced
                .iter()
                .map(|(cid, iid)| json!({ "catalogId": cid, "instanceId": iid }))
                .collect();
            let req = TxRequest {
                user_id: transaction.user_id.to_owned(),
                character_id: transaction.character_id.to_owned(),
                transaction_id: transaction.transaction_id.to_owned(),
                add_instanced: Value::Array(transaction.add_instanced),
                add_stacked: Value::Array(Vec::new()),
                remove_instanced: Value::Array(remove_instanced),
                remove_stacked: Value::Array(Vec::new()),
                save_instanced: Value::Array(Vec::new()),
            };
            let options = TxOptions {
                ownership_already_validated: true,
                diagnostic_label: Some("slayers-path-reward".into()),
                economy: Some(Economy {
                    source: "OTHER".into(),
                    p2p_session_id: None,
                    store_sku_id: None,
                }),
                ..TxOptions::default()
            };
            match run_inventory_transaction(&ctx, req, options, None).await {
                Ok(TxOutcome::Done(_)) => Ok(true),
                Ok(TxOutcome::NotOwned) => Ok(false),
                Err(error) => Err(error.to_string()),
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RewardGrant {
    pub catalog_id: String,
    pub transaction_id: String,
    pub replacement: Value,
    pub remove: Vec<(String, String)>,
    pub is_repair: bool,
}

fn digest_slice(text: &str, length: usize) -> String {
    sha256_hex(text.as_bytes())[..length].to_ascii_uppercase()
}

/// The plan of `GrantSlayersPathRewards`: which items are missing, and the stable ids that make a retry safe.
pub fn plan_rewards(
    definitions: &Definitions,
    node_ids: &[String],
    user_id: &str,
    character_id: &str,
    instanced_items: &[Value],
) -> Vec<RewardGrant> {
    let mut wanted: Vec<String> = Vec::new();
    for node_id in node_ids {
        for item in node_reward_item_ids(definitions, node_id) {
            if !wanted.contains(&item) {
                wanted.push(item);
            }
        }
    }
    let mut owned: HashSet<String> = HashSet::new();
    let mut legacy: HashMap<String, Vec<&Value>> = HashMap::new();
    for item in instanced_items {
        let Some(catalog_id) = item.get("catalogId").and_then(Value::as_str) else {
            continue;
        };
        // Older builds wrote the catalog id as the instance id for weapon parts: not a real instance identity
        if catalog_id.starts_with("PART_") && item.get("instanceId").and_then(Value::as_str) == Some(catalog_id) {
            legacy.entry(catalog_id.to_owned()).or_default().push(item);
            continue;
        }
        owned.insert(catalog_id.to_owned());
    }
    let mut to_grant: Vec<String> = wanted.into_iter().filter(|id| !owned.contains(id)).collect();
    to_grant.sort();
    to_grant
        .into_iter()
        .map(|catalog_id| {
            let stable = format!("{user_id}:{character_id}:{catalog_id}");
            let legacy_items = legacy.get(&catalog_id).cloned().unwrap_or_default();
            let is_repair = !legacy_items.is_empty();
            let instance_id = digest_slice(&format!("pjm-instance:{stable}"), 26);
            let transaction_id =
                digest_slice(&format!("{}:{stable}", if is_repair { "pjm-repair-instance" } else { "pjm-grant" }), 32);
            let preserved = legacy_items.first();
            let replacement = json!({
                "catalogId": catalog_id,
                "instanceId": instance_id,
                "itemData": preserved.and_then(|p| p.get("itemData")).filter(|d| !d.is_null()).cloned().unwrap_or(Value::Null),
                "updateVersion": preserved.and_then(|p| p.get("updateVersion")).filter(|v| v.is_number()).cloned().unwrap_or(json!(0)),
            });
            let remove = legacy_items
                .iter()
                .map(|item| {
                    (catalog_id.clone(), item.get("instanceId").and_then(Value::as_str).unwrap_or_default().to_owned())
                })
                .collect();
            RewardGrant { catalog_id, transaction_id, replacement, remove, is_repair }
        })
        .collect()
}

/// `GrantSlayersPathRewards`: reconciles reward items for the unlocked nodes. Returns the catalog ids whose
/// transaction completed during this call. Never fails: a failed reward is retried on the next journey save.
pub async fn grant_rewards(
    shared: &std::sync::Arc<Shared>,
    inventory: &dyn JourneyInventory,
    account: &AccountId,
    user_id: &str,
    node_ids: &[String],
) -> Vec<String> {
    if node_ids.is_empty() {
        return Vec::new();
    }
    let definitions = definitions(shared);
    let any_item = node_ids
        .iter()
        .any(|id| !node_reward_item_ids(&definitions, id).is_empty());
    if !any_item {
        tracing::info!(
            "[slayers-path] {user_id}: {} reward-eligible node(s), but none map to an item reward",
            node_ids.len()
        );
        return Vec::new();
    }
    let (character_id, items) = match inventory.first_character(shared, account).await {
        Ok(Some(found)) => found,
        Ok(None) => {
            tracing::warn!("[slayers-path] cannot grant rewards for {user_id}: no character found");
            return Vec::new();
        }
        Err(error) => {
            tracing::error!(
                "[slayers-path] reward reconciliation failed for {user_id}; will retry on next journey save: {error}"
            );
            return Vec::new();
        }
    };
    let mut granted = Vec::new();
    for grant in plan_rewards(&definitions, node_ids, user_id, &character_id, &items) {
        // One transaction per item, so one bad reward cannot stop the others
        let result = inventory
            .run_transaction(
                shared,
                account,
                RewardTransaction {
                    user_id,
                    character_id: &character_id,
                    transaction_id: &grant.transaction_id,
                    add_instanced: vec![grant.replacement.clone()],
                    remove_instanced: grant.remove.clone(),
                },
            )
            .await;
        match result {
            Ok(true) => {
                tracing::info!("[slayers-path] {user_id}: granted {}", grant.catalog_id);
                granted.push(grant.catalog_id);
            }
            Ok(false) => tracing::warn!(
                "[slayers-path] {user_id}: inventory transaction did not apply for {}; will retry on next journey save",
                grant.catalog_id
            ),
            Err(error) => tracing::error!(
                "[slayers-path] {user_id}: grant failed for {}; will retry on next journey save: {error}",
                grant.catalog_id
            ),
        }
    }
    granted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defs() -> Definitions {
        let value = json!({
            "A": { "childNodes": ["B", "C"], "currencyCosts": [{ "currency": "CURRENCY_PJM_WEAPON", "amount": 25 }] },
            "B": { "childNodes": ["D"] },
            "C": { "childNodes": ["D"] },
            "D": { "childNodes": ["E"], "rewards": ["AC_SOMETHING"] },
            "X": { "currencyCosts": [{ "currency": "CURRENCY_NOTES", "amount": 10 }] },
        });
        as_record(&value)
    }

    fn graph() -> SlayersPathGraph {
        build_graph(&defs(), &["E", "A", "B", "C", "D", "X", "Loose"].map(String::from))
    }

    fn node(id: &str, status: i64, extra: Value) -> Value {
        let mut base = json!({ "node_id": id, "node_status": status, "objectives": [] });
        for (k, v) in extra.as_object().into_iter().flatten() {
            base[k] = v.clone();
        }
        base
    }

    fn map_of(entries: &[(&str, i64)]) -> Value {
        Value::Object(
            entries
                .iter()
                .map(|(id, s)| ((*id).to_owned(), node(id, *s, json!({}))))
                .collect(),
        )
    }

    fn plan(stored: &Value, merged: &Value) -> HostJourneyPlan {
        plan_host_journey(stored, merged, &graph())
    }

    fn reasons(plan: &HostJourneyPlan) -> Vec<(&str, &str)> {
        plan.stripped
            .iter()
            .map(|(id, why)| (id.as_str(), why.as_str()))
            .collect()
    }

    fn with(base: &Value, more: Value) -> Value {
        let mut out = as_record(base);
        out.extend(as_record(&more));
        Value::Object(out)
    }

    #[test]
    fn the_graph_keeps_every_parent_ignores_empty_children_and_knows_served_nodes() {
        let built = build_graph(
            &as_record(
                &json!({ "P1": { "childNodes": ["K", "K", "None", ""] }, "P2": { "childNodes": ["K", "L"] }, "Leaf": {} }),
            ),
            &["Served".to_owned(), String::new()],
        );
        assert_eq!(built.parents["K"], ["P1", "P2"]);
        assert_eq!(built.parents["L"], ["P2"]);
        assert!(!built.parents.contains_key("P1"));
        let mut known: Vec<_> = built.known.iter().cloned().collect();
        known.sort();
        assert_eq!(known, ["K", "L", "Leaf", "P1", "P2", "Served"]);
        assert!(built.usable);
        assert!(!build_graph(&Definitions::new(), &[]).usable);
    }

    #[test]
    fn a_root_is_accepted_and_a_child_of_a_locked_parent_keeps_its_stored_status() {
        let merged = with(
            &map_of(&[("A", 0), ("X", 2)]),
            json!({ "B": node("B", 2, json!({ "objectives": [{ "id": "kills", "progress": 3 }] })) }),
        );
        let result = plan(&map_of(&[("A", 0), ("B", 0)]), &merged);
        assert_eq!(result.unlocked, ["X"]);
        assert_eq!(reasons(&result), [("B", "prerequisite")]);
        assert_eq!(result.nodes["B"], node("B", 0, json!({})));
        assert_eq!(result.nodes["X"]["node_status"], 2);
    }

    #[test]
    fn a_stripped_node_the_store_did_not_have_stays_locked_with_what_the_host_sent() {
        let result = plan(
            &json!({}),
            &json!({ "D": node("D", 1, json!({ "objectives": [{ "id": "x", "progress": 2 }] })) }),
        );
        assert!(result.unlocked.is_empty());
        assert_eq!(
            result.nodes["D"],
            json!({ "node_id": "D", "node_status": 0, "objectives": [{ "id": "x", "progress": 2 }] })
        );
    }

    #[test]
    fn a_child_is_accepted_when_a_parent_is_unlocked_in_either_status() {
        assert_eq!(
            plan(&map_of(&[("A", 2)]), &map_of(&[("A", 2), ("B", 2)])).unlocked,
            ["B"]
        );
        assert_eq!(
            plan(&map_of(&[("A", 1)]), &map_of(&[("A", 1), ("C", 1)])).unlocked,
            ["C"]
        );
        assert_eq!(
            plan(&map_of(&[("B", 2)]), &map_of(&[("B", 2), ("D", 2)])).unlocked,
            ["D"]
        );
        assert_eq!(
            plan(&map_of(&[("C", 2)]), &map_of(&[("C", 2), ("D", 2)])).unlocked,
            ["D"]
        );
        let result = plan(&map_of(&[("A", 2)]), &map_of(&[("A", 2), ("D", 2)]));
        assert_eq!(
            result.stripped.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
            ["D"]
        );
    }

    #[test]
    fn a_purchase_and_its_children_in_one_save_are_accepted_in_any_key_order() {
        let reverse = map_of(&[("E", 2), ("D", 2), ("B", 2), ("A", 2)]);
        let result = plan(&json!({}), &reverse);
        let mut unlocked = result.unlocked.clone();
        unlocked.sort();
        assert_eq!(unlocked, ["A", "B", "D", "E"]);
        assert!(result.stripped.is_empty());
        assert_eq!(result.nodes.keys().collect::<Vec<_>>(), ["E", "D", "B", "A"]);
    }

    #[test]
    fn a_broken_link_strips_everything_below_it_and_nothing_above() {
        let result = plan(
            &map_of(&[("A", 0)]),
            &map_of(&[("A", 0), ("B", 2), ("D", 2), ("E", 2), ("X", 2)]),
        );
        assert_eq!(result.unlocked, ["X"]);
        assert_eq!(
            reasons(&result),
            [("B", "prerequisite"), ("D", "prerequisite"), ("E", "prerequisite")]
        );
        for id in ["B", "D", "E"] {
            assert_eq!(result.nodes[id]["node_status"], 0);
        }
    }

    #[test]
    fn a_node_the_game_does_not_have_is_stripped() {
        let result = plan(&json!({}), &map_of(&[("Ghost", 2), ("Loose", 2)]));
        assert_eq!(reasons(&result), [("Ghost", "unknown-node")]);
        assert_eq!(result.unlocked, ["Loose"]);
        assert_eq!(result.nodes["Ghost"]["node_status"], 0);
    }

    #[test]
    fn an_unlocked_node_cannot_be_locked_again() {
        let result = plan(&map_of(&[("A", 2), ("X", 1)]), &map_of(&[("A", 0), ("X", 0), ("B", 0)]));
        let mut relocked = result.relocked.clone();
        relocked.sort();
        assert_eq!(relocked, ["A", "X"]);
        assert_eq!(result.nodes["A"]["node_status"], 2);
        assert_eq!(result.nodes["X"]["node_status"], 1);
        assert!(result.unlocked.is_empty() && result.stripped.is_empty());
    }

    #[test]
    fn an_unlocked_entry_filed_under_another_nodes_id_is_refused() {
        let disguised = plan(&json!({}), &json!({ "X": node("A", 2, json!({})) }));
        assert_eq!(reasons(&disguised), [("X", "node-id-mismatch")]);
        assert!(!disguised.nodes.contains_key("X"));
        assert!(disguised.unlocked.is_empty());

        let overwrite = plan(&map_of(&[("X", 2)]), &json!({ "X": node("D", 2, json!({})) }));
        assert_eq!(reasons(&overwrite), [("X", "node-id-mismatch")]);
        assert_eq!(overwrite.nodes["X"], node("X", 2, json!({})));

        let legacy = plan(
            &json!({ "X": node("A", 2, json!({})) }),
            &json!({ "X": node("A", 2, json!({})) }),
        );
        assert!(legacy.stripped.is_empty());
        assert_eq!(legacy.nodes["X"], node("A", 2, json!({})));

        let keyed = plan(&json!({}), &json!({ "X": { "node_status": 2, "objectives": [] } }));
        assert_eq!(keyed.unlocked, ["X"]);
    }

    #[test]
    fn nodes_already_unlocked_or_not_unlocked_pass_through_unreported() {
        let stored = map_of(&[("A", 2), ("B", 2), ("X", 0)]);
        let merged = with(
            &stored,
            json!({ "B": node("B", 2, json!({ "objectives": [{ "id": "more", "progress": 9 }] })), "E": node("E", 0, json!({})) }),
        );
        let result = plan(&stored, &merged);
        assert!(result.unlocked.is_empty() && result.stripped.is_empty() && result.relocked.is_empty());
        assert_eq!(Value::Object(result.nodes), merged);
    }

    #[test]
    fn without_usable_definitions_every_new_unlock_is_refused() {
        let unusable = build_graph(&Definitions::new(), &["A", "B", "X"].map(String::from));
        let result = plan_host_journey(&map_of(&[("A", 2)]), &map_of(&[("A", 2), ("X", 2)]), &unusable);
        assert_eq!(reasons(&result), [("X", "definitions-unavailable")]);
        assert_eq!(result.nodes["A"]["node_status"], 2);
        assert_eq!(result.nodes["X"]["node_status"], 0);
    }

    #[test]
    fn a_prototype_key_is_an_ordinary_unknown_node() {
        let merged: Value = serde_json::from_str(
            r#"{"__proto__":{"node_id":"__proto__","node_status":2},"constructor":{"node_status":2},"A":{"node_id":"A","node_status":2}}"#,
        )
        .unwrap();
        let result = plan(&json!({}), &merged);
        assert_eq!(result.unlocked, ["A"]);
        assert_eq!(
            reasons(&result),
            [("__proto__", "unknown-node"), ("constructor", "unknown-node")]
        );
        assert!(result.nodes.contains_key("__proto__"));
    }

    #[test]
    fn malformed_inputs_give_an_empty_plan_and_the_inputs_are_untouched() {
        let stored = map_of(&[("A", 2)]);
        let merged = map_of(&[("A", 0), ("B", 2), ("D", 2), ("X", 2)]);
        let (before_s, before_m) = (stored.to_string(), merged.to_string());
        plan(&stored, &merged);
        assert_eq!((stored.to_string(), merged.to_string()), (before_s, before_m));
        for bad in [json!(null), json!("text"), json!(7), json!([])] {
            let result = plan(&bad, &bad);
            assert!(result.nodes.is_empty() && result.unlocked.is_empty() && result.stripped.is_empty());
        }
    }

    #[test]
    fn status_zero_absent_and_sentinel_are_not_unlocks() {
        let merged = json!({ "B": node("B", 0, json!({})), "D": node("D", 3, json!({})), "E": { "node_id": "E" }, "X": node("X", 255, json!({})) });
        let result = plan(&json!({}), &merged);
        assert!(result.unlocked.is_empty() && result.stripped.is_empty());
    }

    // The 2026-10-16 mask (slayersPathGauntlet.test.ts)

    fn gauntlet_graph() -> SlayersPathGraph {
        build_graph(
            &as_record(
                &json!({ "Slayer_11": { "childNodes": ["Prestige.Unlock"] }, "Prestige.Unlock": { "childNodes": [GAUNTLET_UNLOCK_NODE] } }),
            ),
            &[],
        )
    }

    fn unlocked_chain() -> Value {
        json!({ "Slayer_11": node("Slayer_11", 2, json!({})), "Prestige.Unlock": node("Prestige.Unlock", 2, json!({})),
                GAUNTLET_UNLOCK_NODE: node(GAUNTLET_UNLOCK_NODE, 2, json!({})) })
    }

    #[test]
    fn the_answer_is_a_masked_copy() {
        let stored = unlocked_chain();
        let served = mask_nodes(&stored, &[GAUNTLET_UNLOCK_NODE]);
        assert_eq!(served[GAUNTLET_UNLOCK_NODE]["node_status"], 0);
        assert_eq!(served[GAUNTLET_UNLOCK_NODE]["node_id"], GAUNTLET_UNLOCK_NODE);
        assert_eq!(served["Prestige.Unlock"]["node_status"], 2);
        assert_eq!(stored[GAUNTLET_UNLOCK_NODE]["node_status"], 2);
        assert!(
            !mask_nodes(
                &json!({ "Prestige.Unlock": node("Prestige.Unlock", 2, json!({})) }),
                &[GAUNTLET_UNLOCK_NODE]
            )
            .contains_key(GAUNTLET_UNLOCK_NODE)
        );
        assert_eq!(mask_nodes(&stored, &[])[GAUNTLET_UNLOCK_NODE]["node_status"], 2);
    }

    #[test]
    fn a_host_save_with_the_masked_status_keeps_the_stored_unlock() {
        let stored = unlocked_chain();
        let sent = Value::Object(mask_nodes(&stored, &[GAUNTLET_UNLOCK_NODE]));
        let merged = Value::Object(merge_nodes(&stored, &sent, 594.0, 593.0).0);
        let restored = Value::Object(restore_shown_locked(&stored, &merged, &[GAUNTLET_UNLOCK_NODE]));
        let result = plan_host_journey(&stored, &restored, &gauntlet_graph());
        assert_eq!(result.nodes[GAUNTLET_UNLOCK_NODE]["node_status"], 2);
        assert!(result.relocked.is_empty() && result.stripped.is_empty() && result.unlocked.is_empty());
    }

    #[test]
    fn a_real_unlock_while_gauntlet_is_off_is_checked_stored_and_served_locked() {
        let stored = json!({ "Slayer_11": node("Slayer_11", 2, json!({})), "Prestige.Unlock": node("Prestige.Unlock", 2, json!({})) });
        let sent = with(
            &stored,
            json!({ GAUNTLET_UNLOCK_NODE: node(GAUNTLET_UNLOCK_NODE, 2, json!({})) }),
        );
        let merged = Value::Object(merge_nodes(&stored, &sent, 10.0, 9.0).0);
        let restored = Value::Object(restore_shown_locked(&stored, &merged, &[GAUNTLET_UNLOCK_NODE]));
        let result = plan_host_journey(&stored, &restored, &gauntlet_graph());
        assert_eq!(result.unlocked, [GAUNTLET_UNLOCK_NODE]);
        let served = mask_nodes(&Value::Object(result.nodes), &[GAUNTLET_UNLOCK_NODE]);
        assert_eq!(served[GAUNTLET_UNLOCK_NODE]["node_status"], 0);
        // without its parent the host check still refuses it
        let no_parent = json!({ "Slayer_11": node("Slayer_11", 2, json!({})) });
        let sent = with(
            &no_parent,
            json!({ GAUNTLET_UNLOCK_NODE: node(GAUNTLET_UNLOCK_NODE, 2, json!({})) }),
        );
        let merged = Value::Object(merge_nodes(&no_parent, &sent, 10.0, 9.0).0);
        let refused = plan_host_journey(
            &no_parent,
            &Value::Object(restore_shown_locked(&no_parent, &merged, &[GAUNTLET_UNLOCK_NODE])),
            &gauntlet_graph(),
        );
        assert_eq!(refused.nodes[GAUNTLET_UNLOCK_NODE]["node_status"], 0);
    }

    #[test]
    fn the_game_server_path_does_not_overwrite_the_unlock_with_the_masked_status() {
        let stored = json!({ "Prestige.Unlock": node("Prestige.Unlock", 2, json!({})), GAUNTLET_UNLOCK_NODE: node(GAUNTLET_UNLOCK_NODE, 2, json!({})) });
        let sent = Value::Object(mask_nodes(&stored, &[GAUNTLET_UNLOCK_NODE]));
        let merged = Value::Object(merge_nodes(&stored, &sent, 594.0, 593.0).0);
        assert_eq!(merged[GAUNTLET_UNLOCK_NODE]["node_status"], 0);
        assert_eq!(
            restore_shown_locked(&stored, &merged, &[GAUNTLET_UNLOCK_NODE])[GAUNTLET_UNLOCK_NODE]["node_status"],
            2
        );
        assert_eq!(
            restore_shown_locked(&stored, &merged, &[])[GAUNTLET_UNLOCK_NODE]["node_status"],
            0
        );
    }

    #[test]
    fn the_merge_keeps_nodes_and_follows_the_spread_order() {
        let stored = json!({ "a": 1, "b": 2 });
        let incoming = json!({ "b": 20, "c": 30 });
        let (stale, version) = merge_nodes(&stored, &incoming, 3.0, 3.0);
        assert_eq!(Value::Object(stale.clone()).to_string(), r#"{"b":2,"c":30,"a":1}"#);
        assert_eq!(version, 3.0);
        let (fresh, version) = merge_nodes(&stored, &incoming, 4.0, 3.0);
        assert_eq!(Value::Object(fresh).to_string(), r#"{"a":1,"b":20,"c":30}"#);
        assert_eq!(version, 4.0);
        // not an object: treated as empty
        assert_eq!(merge_nodes(&json!([1]), &json!("x"), 1.0, 0.0).0.len(), 0);
    }

    #[test]
    fn unlocked_ids_read_node_id_then_the_key() {
        let nodes = json!({ "k1": { "node_id": "N1", "node_status": 2 }, "k2": { "node_status": 1 }, "k3": { "node_status": 0 }, "k4": { "node_id": "", "node_status": 2 } });
        assert_eq!(unlocked_node_ids(&nodes), ["N1", "k2", "k4"]);
        assert_eq!(
            newly_unlocked_node_ids(&json!({ "k1": { "node_status": 2 } }), &nodes),
            ["k2", "k4"]
        );
    }

    #[test]
    fn rewards_are_planned_once_with_stable_ids_and_repair_legacy_instances() {
        let definitions = as_record(&json!({
            "N1": { "rewards": ["PART_GA_SPECIAL_SKILLSHOT", "None", ""] },
            "N2": { "rewards": ["AC_OWNED"] },
            "N3": { "rewards": ["PART_LEGACY"] },
        }));
        let items = vec![
            json!({ "catalogId": "AC_OWNED", "instanceId": "I1" }),
            json!({ "catalogId": "PART_LEGACY", "instanceId": "PART_LEGACY", "itemData": { "x": 1 }, "updateVersion": 4 }),
        ];
        let nodes = ["N1", "N2", "N3"].map(String::from);
        let plan = plan_rewards(&definitions, &nodes, "u1", "c1", &items);
        assert_eq!(
            plan.iter().map(|g| g.catalog_id.as_str()).collect::<Vec<_>>(),
            ["PART_GA_SPECIAL_SKILLSHOT", "PART_LEGACY"]
        );
        let fresh = &plan[0];
        assert!(!fresh.is_repair);
        assert_eq!(
            fresh.transaction_id,
            digest_slice("pjm-grant:u1:c1:PART_GA_SPECIAL_SKILLSHOT", 32)
        );
        assert_eq!(
            fresh.replacement["instanceId"],
            digest_slice("pjm-instance:u1:c1:PART_GA_SPECIAL_SKILLSHOT", 26)
        );
        assert_eq!(fresh.replacement["itemData"], Value::Null);
        assert_eq!(fresh.replacement["updateVersion"], 0);
        let repair = &plan[1];
        assert!(repair.is_repair);
        assert_eq!(
            repair.transaction_id,
            digest_slice("pjm-repair-instance:u1:c1:PART_LEGACY", 32)
        );
        assert_eq!(repair.replacement["itemData"], json!({ "x": 1 }));
        assert_eq!(repair.replacement["updateVersion"], 4);
        assert_eq!(repair.remove, [("PART_LEGACY".to_owned(), "PART_LEGACY".to_owned())]);
        assert_eq!(fresh.transaction_id.len(), 32);
        assert!(
            fresh
                .transaction_id
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_lowercase())
        );
    }
}
