// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The Slayer's Path (player journey map) and its rewards (progression spec §4.11, §4.12). Node maps keep the game's
//! key order; `__proto__` and `constructor` are ordinary keys.

use std::collections::{HashMap, HashSet};

use dainty_service_kit::jsjson;
use serde_json::{Map, Value};

use crate::util::{obj, sha_upper};

/// `node_status` 1 (claimable) or 2 (unlocked).
pub fn is_unlocked(node: &Value) -> bool {
    node.get("node_status")
        .and_then(Value::as_f64)
        .is_some_and(|s| s == 1.0 || s == 2.0)
}

/// Each unlocked entry's `node_id` when a non-empty string, else its key.
pub fn unlocked_node_ids(map: &Map<String, Value>) -> Vec<String> {
    map.iter()
        .filter(|(_, n)| is_unlocked(n))
        .map(|(k, n)| {
            n.get("node_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map_or_else(|| k.clone(), str::to_owned)
        })
        .collect()
}

/// The keys unlocked in `next` and not in `stored`.
pub fn newly_unlocked(stored: &Map<String, Value>, next: &Map<String, Value>) -> Vec<String> {
    next.iter()
        .filter(|(k, n)| is_unlocked(n) && !stored.get(*k).is_some_and(is_unlocked))
        .map(|(k, _)| k.clone())
        .collect()
}

/// The object's members, or none for anything else.
pub fn as_map(value: &Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

/// A stale save (incoming version at most the stored one) keeps the stored values: `(map, version, stale)`.
pub fn merge_nodes(
    stored: &Map<String, Value>,
    stored_version: f64,
    incoming: &Map<String, Value>,
    incoming_version: f64,
) -> (Map<String, Value>, f64, bool) {
    if incoming_version <= stored_version {
        let mut out = incoming.clone();
        for (k, v) in stored {
            out.insert(k.clone(), v.clone());
        }
        (out, stored_version, true)
    } else {
        let mut out = stored.clone();
        for (k, v) in incoming {
            out.insert(k.clone(), v.clone());
        }
        (out, incoming_version, false)
    }
}

/// A copy with each masked node that is an object shown at `node_status` 0.
pub fn mask_nodes(map: &Map<String, Value>, masked: &[String]) -> Map<String, Value> {
    let mut out = map.clone();
    for id in masked {
        if let Some(Value::Object(node)) = out.get_mut(id) {
            node.insert("node_status".into(), Value::from(0));
        }
    }
    out
}

/// Puts back a masked node's stored entry when it is unlocked there (the client's masked echo never overwrites it).
pub fn restore_masked(merged: &mut Map<String, Value>, stored: &Map<String, Value>, masked: &[String]) {
    for id in masked {
        if let Some(node) = stored.get(id).filter(|n| is_unlocked(n)) {
            merged.insert(id.clone(), node.clone());
        }
    }
}

/// `slayers_path_definitions.json` and the served node keys.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Graph {
    /// Node id to the nodes that list it as a child.
    pub parents: HashMap<String, Vec<String>>,
    /// Every node id the definitions or the served keys name.
    pub known: HashSet<String>,
    /// False when the definitions failed to load; a host's unlocks are then refused.
    pub usable: bool,
    /// Node id to its reward catalog ids
    pub rewards: HashMap<String, Vec<String>>,
}

fn names(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|s| !s.is_empty() && *s != "None")
        .map(str::to_owned)
        .collect()
}

impl Graph {
    /// From the definitions (an object keyed by node id; none when they failed) and the served node keys.
    pub fn build(definitions: Option<&Map<String, Value>>, served: &[String]) -> Self {
        let mut graph = Graph::default();
        if let Some(defs) = definitions {
            graph.usable = true;
            for (id, def) in defs {
                graph.known.insert(id.clone());
                for child in names(def.get("childNodes")) {
                    graph.known.insert(child.clone());
                    let parents = graph.parents.entry(child).or_default();
                    if !parents.contains(id) {
                        parents.push(id.clone());
                    }
                }
                let rewards = names(def.get("rewards"));
                if !rewards.is_empty() {
                    graph.rewards.insert(id.clone(), rewards);
                }
            }
        }
        graph.known.extend(served.iter().cloned());
        graph
    }
}

/// What a host's journey save may store.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HostJourneyPlan {
    /// The nodes to store.
    pub nodes: Map<String, Value>,
    /// Nodes newly unlocked and accepted.
    pub unlocked: Vec<String>,
    /// `(id, reason)`
    pub stripped: Vec<(String, &'static str)>,
    /// Stored unlocks the save would have lost, put back.
    pub relocked: Vec<String>,
}

fn put_back(nodes: &mut Map<String, Value>, stored: &Map<String, Value>, key: &str, fallback: Option<Value>) {
    match stored.get(key) {
        Some(entry) => {
            nodes.insert(key.to_owned(), entry.clone());
        }
        None => match fallback {
            Some(value) => {
                nodes.insert(key.to_owned(), value);
            }
            None => {
                nodes.shift_remove(key);
            }
        },
    }
}

/// A host's save (§4.11 "Host plan"): only nodes the graph allows are unlocked; a stored unlock is never lost.
pub fn plan_host_journey(stored: &Map<String, Value>, merged: &Map<String, Value>, graph: &Graph) -> HostJourneyPlan {
    let mut nodes = merged.clone();
    let mut stripped = Vec::new();
    let keys: Vec<String> = nodes.keys().cloned().collect();
    for key in &keys {
        let node = &nodes[key];
        let mismatch = is_unlocked(node)
            && node.get("node_id").and_then(Value::as_str).is_some_and(|id| id != key)
            && stored.get(key).map(jsjson::stringify) != Some(jsjson::stringify(node));
        if mismatch {
            stripped.push((key.clone(), "node-id-mismatch"));
            put_back(&mut nodes, stored, key, None);
        }
    }
    let mut relocked = Vec::new();
    for (key, entry) in stored {
        if is_unlocked(entry) && !nodes.get(key).is_some_and(is_unlocked) {
            nodes.insert(key.clone(), entry.clone());
            relocked.push(key.clone());
        }
    }
    let pending: Vec<String> = nodes
        .iter()
        .filter(|(k, n)| is_unlocked(n) && !stored.get(*k).is_some_and(is_unlocked))
        .map(|(k, _)| k.clone())
        .collect();
    let mut refused: Vec<(String, &'static str)> = Vec::new();
    let mut candidates = Vec::new();
    for key in &pending {
        if !graph.usable {
            refused.push((key.clone(), "definitions-unavailable"));
        } else if !graph.known.contains(key) {
            refused.push((key.clone(), "unknown-node"));
        } else {
            candidates.push(key.clone());
        }
    }
    let mut accepted: HashSet<String> = HashSet::new();
    loop {
        let before = accepted.len();
        for key in &candidates {
            if accepted.contains(key) {
                continue;
            }
            let parents = graph.parents.get(key);
            let open = parents.is_none_or(|ps| {
                ps.is_empty()
                    || ps
                        .iter()
                        .any(|p| accepted.contains(p) || stored.get(p).is_some_and(is_unlocked))
            });
            if open {
                accepted.insert(key.clone());
            }
        }
        if accepted.len() == before {
            break;
        }
    }
    for key in &candidates {
        if !accepted.contains(key) {
            refused.push((key.clone(), "prerequisite"));
        }
    }
    for (key, _) in &refused {
        let locked = nodes.get(key).cloned().map(|mut n| {
            if let Some(map) = n.as_object_mut() {
                map.insert("node_status".into(), Value::from(0));
            }
            n
        });
        put_back(&mut nodes, stored, key, locked);
    }
    stripped.extend(refused);
    HostJourneyPlan {
        unlocked: pending.into_iter().filter(|k| accepted.contains(k)).collect(),
        nodes,
        stripped,
        relocked,
    }
}

/// One reward item to grant (§4.12).
#[derive(Debug, Clone, PartialEq)]
pub struct RewardGrant {
    /// The reward item's catalog id.
    pub catalog_id: String,
    /// The grant's transaction id, derived from the user, character and item.
    pub transaction_id: String,
    /// The instanced item to add, with an id derived the same way.
    pub item: Value,
    /// `{catalogId, instanceId}` of the legacy items it repairs
    pub removes: Vec<Value>,
}

fn is_legacy(item: &Value) -> bool {
    let catalog = item.get("catalogId").and_then(Value::as_str).unwrap_or("");
    catalog.starts_with("PART_") && item.get("instanceId").and_then(Value::as_str) == Some(catalog)
}

/// The rewards of `nodes` the character does not own yet, sorted by catalog id.
pub fn plan_rewards(
    user: &str,
    character: &str,
    nodes: &[String],
    rewards: &HashMap<String, Vec<String>>,
    instanced: &[Value],
) -> Vec<RewardGrant> {
    let mut wanted: Vec<String> = Vec::new();
    for node in nodes {
        for item in rewards.get(node).into_iter().flatten() {
            if !wanted.contains(item) {
                wanted.push(item.clone());
            }
        }
    }
    let owned: HashSet<&str> = instanced
        .iter()
        .filter(|i| !is_legacy(i))
        .filter_map(|i| i.get("catalogId").and_then(Value::as_str))
        .collect();
    let mut grant: Vec<String> = wanted.into_iter().filter(|w| !owned.contains(w.as_str())).collect();
    grant.sort();
    grant
        .into_iter()
        .map(|catalog| {
            let stable = format!("{user}:{character}:{catalog}");
            let legacy: Vec<&Value> = instanced
                .iter()
                .filter(|i| is_legacy(i) && i.get("catalogId").and_then(Value::as_str) == Some(catalog.as_str()))
                .collect();
            let scope = if legacy.is_empty() {
                "pjm-grant:"
            } else {
                "pjm-repair-instance:"
            };
            let first = legacy.first();
            let item_data = first
                .and_then(|i| i.get("itemData"))
                .filter(|d| !d.is_null())
                .cloned()
                .unwrap_or(Value::Null);
            let version = first
                .and_then(|i| i.get("updateVersion"))
                .filter(|v| v.is_number())
                .cloned()
                .unwrap_or(Value::from(0));
            RewardGrant {
                transaction_id: sha_upper(&format!("{scope}{stable}"), 32),
                item: obj(vec![
                    ("catalogId", Value::from(catalog.as_str())),
                    (
                        "instanceId",
                        Value::from(sha_upper(&format!("pjm-instance:{stable}"), 26)),
                    ),
                    ("itemData", item_data),
                    ("updateVersion", version),
                ]),
                removes: legacy
                    .iter()
                    .map(|i| {
                        obj(vec![
                            ("catalogId", Value::from(catalog.as_str())),
                            ("instanceId", i.get("instanceId").cloned().unwrap_or(Value::Null)),
                        ])
                    })
                    .collect(),
                catalog_id: catalog,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn map(value: Value) -> Map<String, Value> {
        as_map(&value)
    }

    #[test]
    fn merges_and_masks() {
        let stored = map(json!({"a": {"node_status": 2}, "b": {"node_status": 0}}));
        let incoming = map(json!({"c": {"node_status": 2}, "a": {"node_status": 0}}));
        let (newer, v, stale) = merge_nodes(&stored, 1.0, &incoming, 2.0);
        assert_eq!((v, stale), (2.0, false));
        assert_eq!(
            Value::Object(newer),
            json!({"a": {"node_status": 0}, "b": {"node_status": 0}, "c": {"node_status": 2}})
        );
        let (old, v, stale) = merge_nodes(&stored, 3.0, &incoming, 2.0);
        assert_eq!((v, stale), (3.0, true));
        assert_eq!(old.keys().collect::<Vec<_>>(), ["c", "a", "b"]);
        assert_eq!(old["a"], json!({"node_status": 2}));
        let masked = mask_nodes(
            &map(json!({"Gauntlet_Unlock": {"node_status": 2}})),
            &["Gauntlet_Unlock".into()],
        );
        assert_eq!(masked["Gauntlet_Unlock"]["node_status"], 0);
        assert_eq!(
            newly_unlocked(&stored, &map(json!({"a": {"node_status": 2}, "c": {"node_status": 1}}))),
            ["c"]
        );
    }

    #[test]
    fn host_plan() {
        let defs = map(
            json!({"root": {"childNodes": ["mid", "None"], "rewards": ["R1"]}, "mid": {"childNodes": ["leaf"]}, "other": {"childNodes": ["orphan"]}}),
        );
        let graph = Graph::build(Some(&defs), &["served".to_owned()]);
        let stored = map(json!({"root": {"node_status": 2}, "kept": {"node_status": 2}}));
        let merged = map(json!({
            "root": {"node_status": 2}, "mid": {"node_status": 2}, "leaf": {"node_status": 2},
            "orphan": {"node_status": 2}, "ghost": {"node_status": 1}, "x": {"node_status": 2, "node_id": "y"},
        }));
        let plan = plan_host_journey(&stored, &merged, &graph);
        assert_eq!(plan.unlocked, ["mid", "leaf"]);
        assert_eq!(plan.relocked, ["kept"]);
        assert_eq!(
            plan.stripped,
            vec![
                ("x".into(), "node-id-mismatch"),
                ("ghost".into(), "unknown-node"),
                ("orphan".into(), "prerequisite")
            ]
        );
        assert_eq!(plan.nodes["orphan"]["node_status"], 0);
        assert!(!plan.nodes.contains_key("x"));
        assert_eq!(plan.nodes["kept"]["node_status"], 2);
        let none = plan_host_journey(&stored, &merged, &Graph::build(None, &[]));
        assert!(none.stripped.iter().any(|(_, r)| *r == "definitions-unavailable"));
        assert!(none.unlocked.is_empty());
    }

    #[test]
    fn rewards() {
        let rewards = HashMap::from([
            ("a".to_owned(), vec!["PART_B".to_owned(), "PART_A".to_owned()]),
            ("b".to_owned(), vec!["PART_A".to_owned()]),
        ]);
        let instanced =
            vec![json!({"catalogId": "PART_A", "instanceId": "PART_A", "itemData": "d", "updateVersion": 3})];
        let plan = plan_rewards("u", "c", &["a".into(), "b".into()], &rewards, &instanced);
        assert_eq!(
            plan.iter().map(|g| g.catalog_id.as_str()).collect::<Vec<_>>(),
            ["PART_A", "PART_B"]
        );
        assert_eq!(plan[0].transaction_id, sha_upper("pjm-repair-instance:u:c:PART_A", 32));
        assert_eq!(plan[0].item["itemData"], "d");
        assert_eq!(
            plan[0].removes,
            vec![json!({"catalogId": "PART_A", "instanceId": "PART_A"})]
        );
        assert_eq!(plan[1].transaction_id, sha_upper("pjm-grant:u:c:PART_B", 32));
        assert_eq!(plan[1].item["instanceId"], sha_upper("pjm-instance:u:c:PART_B", 26));
        let owned = vec![json!({"catalogId": "PART_A", "instanceId": "real"})];
        assert_eq!(plan_rewards("u", "c", &["b".into()], &rewards, &owned), vec![]);
    }
}
