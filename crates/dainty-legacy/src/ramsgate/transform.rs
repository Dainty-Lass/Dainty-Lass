// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! One Ramsgate Reborn player's documents into the 1.14.7 collections `rustic-isles-db` maps
//! (`docs/port/spec-legacy-ramsgate.md` sections 2, 4, 5 and 7): the user id re-keyed everywhere, the RR-only
//! collections folded, and what 1.14.7 has no home for left behind and counted.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde_json::{Map, Value};

/// A MongoDB document.
pub type Doc = Map<String, Value>;

/// One player's documents as RR keeps them, by collection (section 4's plan).
pub type RrSave = BTreeMap<String, Vec<Doc>>;

/// The collections carried as they are (same shape as 1.14.7's, section 2.1).
const SAME: [&str; 9] = [
    "characters",
    "inventories",
    "loadouts",
    "wallets",
    "playerJourney",
    "breadcrumbs",
    "encounteredContent",
    "progressionTracks",
    "progressionObjectives",
];
/// RR-only collections with no 1.14.7 home (section 2.15).
const LEFT: [&str; 2] = ["trackedObjectives", "playtime"];
/// A gatherable's cooldown id: 32 upper-case hex characters, worthless after a day (section 2.14).
const GATHERABLE_MS: i64 = 86_400_000;
/// Ids shorter than this are never re-keyed: too likely to occur by chance (section 6.4 step 3).
const MIN_ID: usize = 8;

/// The 1.14.7 documents, and what was left behind.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Target {
    /// By 1.14.7 collection name.
    pub collections: BTreeMap<String, Vec<Doc>>,
    /// What was not carried, by what it was: `item <catalog id>`, `currency <id>`, a collection name.
    pub dropped: BTreeMap<String, usize>,
}

/// Why a save can't be carried.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refused {
    /// The old user id still appears after re-keying: some document hides it in a form the re-keying missed.
    #[error("the old user id is still in the {0} documents after re-keying")]
    Leftover(String),
    /// No character to carry.
    #[error("the old account has no character")]
    NoCharacter,
}

fn forms(id: &str) -> Vec<String> {
    let mut out: Vec<String> = [id.to_owned(), id.to_lowercase(), id.to_uppercase(), id.replace('-', "")]
        .into_iter()
        .filter(|f| f.len() >= MIN_ID)
        .collect();
    out.sort_by_key(|f| std::cmp::Reverse(f.len()));
    out.dedup();
    out
}

fn replace_all(text: &str, from: &[(String, String)]) -> String {
    let mut out = text.to_owned();
    for (old, new) in from {
        if out.contains(old.as_str()) {
            out = out.replace(old.as_str(), new);
        }
    }
    out
}

fn rekey(value: &mut Value, from: &[(String, String)]) {
    match value {
        Value::String(s) => *s = replace_all(s, from),
        Value::Array(items) => items.iter_mut().for_each(|v| rekey(v, from)),
        Value::Object(map) => {
            let old = std::mem::take(map);
            for (k, mut v) in old {
                rekey(&mut v, from);
                map.insert(replace_all(&k, from), v);
            }
        }
        _ => {}
    }
}

fn squash(text: &str) -> String {
    text.to_lowercase().replace('-', "")
}

fn mentions(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(s) => squash(s).contains(needle),
        Value::Array(items) => items.iter().any(|v| mentions(v, needle)),
        Value::Object(map) => map
            .iter()
            .any(|(k, v)| squash(k).contains(needle) || mentions(v, needle)),
        _ => false,
    }
}

fn text(doc: &Doc, key: &str) -> Option<String> {
    doc.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// A JSON-text field parsed; RR reads one that fails to parse as empty (section 7.2).
fn parsed(doc: &Doc, key: &str, empty: Value) -> Value {
    match doc.get(key) {
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(empty),
        Some(v @ (Value::Array(_) | Value::Object(_))) => v.clone(),
        _ => empty,
    }
}

/// Section 4's ownership rule, after the characters are known.
fn owned(doc: &Doc, user: &str, characters: &BTreeSet<String>) -> bool {
    match doc.get("userId").and_then(Value::as_str) {
        Some(u) => u == user,
        None => text(doc, "_id")
            .is_some_and(|id| id == user || id.starts_with(&format!("{user}::")) || characters.contains(&id)),
    }
}

fn known(catalog: Option<&HashSet<String>>, id: &str) -> bool {
    catalog.is_none_or(|c| c.contains(id))
}

fn count(dropped: &mut BTreeMap<String, usize>, what: String) {
    *dropped.entry(what).or_default() += 1;
}

/// Items whose catalog id 1.14.7 doesn't know, out of one JSON-text list.
fn filter_items(doc: &mut Doc, key: &str, catalog: Option<&HashSet<String>>, dropped: &mut BTreeMap<String, usize>) {
    let Value::Array(items) = parsed(doc, key, Value::Array(Vec::new())) else {
        return;
    };
    let kept: Vec<Value> = items
        .into_iter()
        .filter(|item| {
            let id = item.get("catalogId").and_then(Value::as_str).unwrap_or_default();
            let keep = !id.is_empty() && known(catalog, id);
            if !keep {
                count(dropped, format!("item {id}"));
            }
            keep
        })
        .collect();
    doc.insert(key.into(), Value::String(Value::Array(kept).to_string()));
}

/// Platinum is 1.14.7's paid currency, which RR never shows or charges (section 5.4): never carried.
fn filter_wallet(doc: &mut Doc, catalog: Option<&HashSet<String>>, dropped: &mut BTreeMap<String, usize>) {
    let Some(Value::Object(balances)) = doc.get_mut("balances") else {
        return;
    };
    balances.retain(|id, _| {
        let keep = !id.to_uppercase().contains("PLATINUM") && known(catalog, id);
        if !keep {
            count(dropped, format!("currency {id}"));
        }
        keep
    });
}

/// RR's `escalationProgress` nests the values under `payload` and names the season `season` (section 2.12).
fn escalation(doc: &Doc) -> Option<Doc> {
    let mut out = Doc::new();
    out.insert("userId".into(), doc.get("userId")?.clone());
    out.insert("seasonId".into(), doc.get("season")?.clone());
    if let Some(Value::Object(payload)) = doc.get("payload") {
        for key in [
            "escalation_level",
            "next_level_xp",
            "update_version",
            "talents_progress",
            "unlock_progress",
        ] {
            if let Some(v) = payload.get(key) {
                out.insert(key.into(), v.clone());
            }
        }
    }
    if let Some(at) = doc.get("updatedAt") {
        out.insert("createdAt".into(), at.clone());
        out.insert("updatedAt".into(), at.clone());
    }
    Some(out)
}

/// RR's one document per bounty and its drafts, as 1.14.7's one `bountyStates` (section 2.13).
fn bounty_state(user: &str, bounties: &[Doc], drafts: Option<&Doc>) -> Option<Doc> {
    if bounties.is_empty() && drafts.is_none() {
        return None;
    }
    let mut ordered: Vec<&Doc> = bounties.iter().collect();
    ordered.sort_by(|a, b| {
        let slot = |d: &Doc| d.get("slotIndex").and_then(Value::as_i64).unwrap_or(i64::MAX);
        (slot(a), text(a, "bountyId")).cmp(&(slot(b), text(b, "bountyId")))
    });
    let elements: Vec<Value> = ordered
        .into_iter()
        .filter_map(|d| {
            let mut data = d.get("data")?.as_object()?.clone();
            if !data.get("drafted_timestamp").is_some_and(Value::is_string)
                && let Some(at) = d.get("updatedAt")
            {
                data.insert("drafted_timestamp".into(), at.clone());
            }
            Some(Value::Object(data))
        })
        .collect();
    let mut out = Doc::new();
    out.insert("userId".into(), Value::from(user));
    out.insert("bounties".into(), Value::Array(elements));
    if let Some(Value::Object(draft)) = drafts.and_then(|d| d.get("data")) {
        out.insert("draftData".into(), Value::Object(draft.clone()));
    }
    Some(out)
}

fn is_gatherable(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b))
}

/// RR's one document per cooldown, as 1.14.7's one `cooldownStates` map; spent gatherables dropped (2.14).
fn cooldown_state(user: &str, cooldowns: &[Doc], now_ms: i64, dropped: &mut BTreeMap<String, usize>) -> Option<Doc> {
    let mut entries = Map::new();
    for doc in cooldowns {
        let (Some(id), Some(started)) = (text(doc, "cooldownId"), text(doc, "startedDate")) else {
            continue;
        };
        let spent = is_gatherable(&id)
            && chrono::DateTime::parse_from_rfc3339(&started)
                .map(|t| now_ms.saturating_sub(t.timestamp_millis()) > GATHERABLE_MS)
                .unwrap_or(true);
        if spent {
            count(dropped, "spent gatherable cooldowns".into());
            continue;
        }
        entries.insert(id, Value::String(started));
    }
    if entries.is_empty() {
        return None;
    }
    let mut out = Doc::new();
    out.insert("userId".into(), Value::from(user));
    out.insert("entries".into(), Value::Object(entries));
    Some(out)
}

/// `save` (RR user `old`) as the 1.14.7 documents of user `new`. `catalog` is 1.14.7's item ids when the host has
/// the catalog export; without it, every item and currency is carried.
pub fn to_target(
    mut save: RrSave,
    old: &str,
    new: &str,
    catalog: Option<&HashSet<String>>,
    now_ms: i64,
) -> Result<Target, Refused> {
    let mut target = Target::default();
    let characters: BTreeSet<String> = save
        .get("characters")
        .into_iter()
        .flatten()
        .filter(|d| text(d, "userId").as_deref() == Some(old))
        .filter_map(|d| text(d, "_id").or_else(|| text(d, "characterId")))
        .collect();
    if characters.is_empty() {
        return Err(Refused::NoCharacter);
    }
    for docs in save.values_mut() {
        docs.retain(|d| owned(d, old, &characters));
    }
    for name in LEFT {
        if let Some(n) = save.remove(name).map(|d| d.len()).filter(|n| *n > 0) {
            target.dropped.insert(name.to_owned(), n);
        }
    }

    let pairs: Vec<(String, String)> = forms(old)
        .into_iter()
        .map(|f| {
            let to = if f.contains('-') || !old.contains('-') {
                new.to_owned()
            } else {
                new.replace('-', "")
            };
            (f, to)
        })
        .collect();
    for docs in save.values_mut() {
        for doc in docs.iter_mut() {
            let mut value = Value::Object(std::mem::take(doc));
            rekey(&mut value, &pairs);
            if let Value::Object(map) = value {
                *doc = map;
            }
        }
    }
    let needle = squash(old);
    if needle.len() >= MIN_ID {
        for (name, docs) in &save {
            if docs.iter().any(|d| {
                d.iter()
                    .any(|(k, v)| squash(k).contains(&needle) || mentions(v, &needle))
            }) {
                return Err(Refused::Leftover(name.clone()));
            }
        }
    }

    for name in SAME {
        let mut docs = save.remove(name).unwrap_or_default();
        for doc in &mut docs {
            match name {
                "inventories" => {
                    filter_items(doc, "instancedItems", catalog, &mut target.dropped);
                    filter_items(doc, "stackedItems", catalog, &mut target.dropped);
                }
                "wallets" => filter_wallet(doc, catalog, &mut target.dropped),
                _ => {}
            }
        }
        if !docs.is_empty() {
            target.collections.insert(name.to_owned(), docs);
        }
    }
    let seasons: Vec<Doc> = save
        .remove("escalationProgress")
        .unwrap_or_default()
        .iter()
        .filter_map(escalation)
        .collect();
    if !seasons.is_empty() {
        target.collections.insert("escalationProgress".into(), seasons);
    }
    let bounties = save.remove("bounties").unwrap_or_default();
    let drafts = save.remove("bountyDrafts").unwrap_or_default();
    if let Some(state) = bounty_state(new, &bounties, drafts.first()) {
        target.collections.insert("bountyStates".into(), vec![state]);
    }
    let cooldowns = save.remove("cooldowns").unwrap_or_default();
    if let Some(state) = cooldown_state(new, &cooldowns, now_ms, &mut target.dropped) {
        target.collections.insert("cooldownStates".into(), vec![state]);
    }
    for (name, docs) in save {
        if !docs.is_empty() {
            target.dropped.insert(name, docs.len());
        }
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const OLD: &str = "3f2b1c4d-aaaa-bbbb-cccc-000000000001";
    const NEW: &str = "dl-user-42";
    const CHAR: &str = "9d0e8f7a-1111-2222-3333-444444444444";
    const NOW: i64 = 1_795_000_000_000;

    fn doc(v: Value) -> Doc {
        v.as_object().cloned().unwrap()
    }

    fn save() -> RrSave {
        let mut s = RrSave::new();
        s.insert(
            "characters".into(),
            vec![
                doc(json!({"_id": CHAR, "characterId": CHAR, "userId": OLD, "name": "Ash",
                    "createdDate": "Nov 19, 2026", "lastModifiedDate": "Nov 19, 2026", "updateVersion": 3,
                    "data": format!("{{\"owner\":\"{OLD}\",\"LastChangelist\":392819}}")})),
                doc(json!({"_id": "someone-elses-char", "userId": "other-user-0000", "name": "X"})),
            ],
        );
        s.insert(
            "inventories".into(),
            vec![doc(json!({"_id": CHAR, "characterId": CHAR, "userId": OLD,
                "instancedItems": "[{\"catalogId\":\"WP_EB_BEGINNER\",\"instanceId\":\"A\"},{\"catalogId\":\"WP_GONE\",\"instanceId\":\"B\"}]",
                "stackedItems": "[{\"catalogId\":\"DYE_RED\",\"quantity\":2}]"}))],
        );
        s.insert(
            "wallets".into(),
            vec![doc(json!({"_id": OLD, "userId": OLD,
                "balances": {"CURRENCY_NOTES": 900, "CURRENCY_PLATINUM": 10_000, "CURRENCY_S19_COIN": 4}}))],
        );
        s.insert(
            "escalationProgress".into(),
            vec![doc(
                json!({"_id": format!("{OLD}::esc1"), "userId": OLD, "season": "esc1",
                "payload": {"escalation_level": 4, "next_level_xp": 9, "talents_progress": [1], "extra": true},
                "updatedAt": "2026-11-19T01:16:10.501Z"}),
            )],
        );
        s.insert(
            "bounties".into(),
            vec![
                doc(
                    json!({"_id": format!("{OLD}::b2"), "userId": OLD, "bountyId": "b2", "slotIndex": null,
                    "data": {"bounty_id": "b2"}, "updatedAt": "2026-11-18T00:00:00.000Z"}),
                ),
                doc(
                    json!({"_id": format!("{OLD}::b1"), "userId": OLD, "bountyId": "b1", "slotIndex": 0,
                    "data": {"bounty_id": "b1", "drafted_timestamp": "t"}}),
                ),
            ],
        );
        s.insert(
            "bountyDrafts".into(),
            vec![doc(json!({"_id": OLD, "data": {"bronze_count": 2}}))],
        );
        s.insert(
            "cooldowns".into(),
            vec![
                doc(json!({"userId": OLD, "cooldownId": "daily", "startedDate": "2026-11-18T00:00:00.000Z"})),
                doc(json!({"userId": OLD, "cooldownId": "0123456789ABCDEF0123456789ABCDEF",
                    "startedDate": "2020-01-01T00:00:00.000Z"})),
            ],
        );
        s.insert("playtime".into(), vec![doc(json!({"_id": OLD, "seconds": 5}))]);
        s
    }

    #[test]
    fn carries_what_maps_and_rekeys_the_user() {
        let catalog: HashSet<String> = ["WP_EB_BEGINNER", "DYE_RED", "CURRENCY_NOTES"].map(String::from).into();
        let t = to_target(save(), OLD, NEW, Some(&catalog), NOW).unwrap();
        let c = &t.collections;

        assert_eq!(c["characters"].len(), 1, "the other player's character is not carried");
        assert_eq!(c["characters"][0]["userId"], NEW);
        assert!(c["characters"][0]["data"].as_str().unwrap().contains(NEW));

        let items: Value = serde_json::from_str(c["inventories"][0]["instancedItems"].as_str().unwrap()).unwrap();
        assert_eq!(items, json!([{"catalogId": "WP_EB_BEGINNER", "instanceId": "A"}]));
        assert_eq!(c["wallets"][0]["balances"], json!({"CURRENCY_NOTES": 900}));

        let season = &c["escalationProgress"][0];
        assert_eq!(
            (&season["seasonId"], &season["escalation_level"]),
            (&json!("esc1"), &json!(4))
        );
        assert!(season.get("extra").is_none() && season.get("payload").is_none());

        let bounties = &c["bountyStates"][0];
        assert_eq!(bounties["userId"], NEW);
        assert_eq!(
            bounties["bounties"],
            json!([{"bounty_id": "b1", "drafted_timestamp": "t"},
                   {"bounty_id": "b2", "drafted_timestamp": "2026-11-18T00:00:00.000Z"}])
        );
        assert_eq!(bounties["draftData"], json!({"bronze_count": 2}));
        assert_eq!(
            c["cooldownStates"][0]["entries"],
            json!({"daily": "2026-11-18T00:00:00.000Z"})
        );

        let dropped: Vec<(&str, usize)> = t.dropped.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        assert_eq!(
            dropped,
            [
                ("currency CURRENCY_PLATINUM", 1),
                ("currency CURRENCY_S19_COIN", 1),
                ("item WP_GONE", 1),
                ("playtime", 1),
                ("spent gatherable cooldowns", 1)
            ]
        );
    }

    #[test]
    fn without_a_catalog_everything_but_platinum_carries() {
        let t = to_target(save(), OLD, NEW, None, NOW).unwrap();
        let items: Value =
            serde_json::from_str(t.collections["inventories"][0]["instancedItems"].as_str().unwrap()).unwrap();
        assert_eq!(items.as_array().unwrap().len(), 2);
        assert_eq!(t.collections["wallets"][0]["balances"].as_object().unwrap().len(), 2);
    }

    #[test]
    fn rekeys_every_form_and_refuses_a_leftover() {
        let mut s = save();
        let upper = OLD.to_uppercase();
        let bare = OLD.replace('-', "");
        s.get_mut("characters").unwrap()[0].insert("note".into(), json!(format!("{upper} {bare}")));
        let t = to_target(s, OLD, "6c1a-new-user-id", None, NOW).unwrap();
        assert_eq!(t.collections["characters"][0]["note"], "6c1a-new-user-id 6c1anewuserid");

        let mut s = save();
        // One dash dropped: no form the re-keying knows, but the same id
        s.get_mut("characters").unwrap()[0].insert("note".into(), json!(OLD.replacen('-', "", 1)));
        assert_eq!(
            to_target(s, OLD, NEW, None, NOW),
            Err(Refused::Leftover("characters".into()))
        );
    }

    #[test]
    fn no_character_is_no_save() {
        let mut s = save();
        s.remove("characters");
        assert_eq!(to_target(s, OLD, NEW, None, NOW), Err(Refused::NoCharacter));
    }
}
