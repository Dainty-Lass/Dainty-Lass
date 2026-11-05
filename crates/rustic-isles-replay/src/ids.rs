// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Learned ids (spec §5): recorded id → the id this target minted, applied to every later request of its side.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::pattern;

/// Recorded id → target id, one per side.
pub type IdMap = BTreeMap<String, String>;

// The id shapes of §5.1 and §7.4, shared with the normalizer
pub(crate) static GRANT: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"(?i)gs-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}"));
pub(crate) static PARTY: LazyLock<Regex> = LazyLock::new(|| {
    pattern(r"\b([a-z0-9-]+-[0-9a-f]{16,32}|[0-9a-f]{32}|[0-9A-HJKMNP-TV-Z]{26}\.[a-z0-9-]+)_([A-Za-z0-9+/=-]+)")
});
pub(crate) static UUID: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b"));
static HEX32: LazyLock<Regex> = LazyLock::new(|| pattern(r"\b[0-9a-f]{32}\b"));
pub(crate) static INSTANCE: LazyLock<Regex> = LazyLock::new(|| pattern(r"\b(?:[A-Z0-9]{26}|[0-9a-f]{16,24})\b"));

const ID_HINTS: [&str; 7] = [
    "id",
    "characterId",
    "instanceId",
    "partyId",
    "candidateId",
    "gameSessionId",
    "grantId",
];

fn looks_like_dynamic_id(text: &str, hint: Option<&str>) -> bool {
    if hint.is_some_and(|h| ID_HINTS.contains(&h)) && !text.is_empty() && text != "INVALID" {
        return true;
    }
    if GRANT.is_match(text) || PARTY.is_match(text) || UUID.is_match(text) || HEX32.is_match(text) {
        return true;
    }
    INSTANCE.is_match(text) && text.bytes().any(|b| b.is_ascii_digit())
}

/// Walks a recorded answer and a live one together, learning recorded → live ids (first mapping wins).
pub fn learn_ids(recorded: &Value, actual: &Value, map: &mut IdMap) {
    learn(recorded, actual, None, map);
}

fn learn(recorded: &Value, actual: &Value, hint: Option<&str>, map: &mut IdMap) {
    match (recorded, actual) {
        (Value::Object(r), Value::Object(a)) => {
            for (key, rv) in r {
                if let Some(av) = a.get(key) {
                    learn(rv, av, Some(key), map);
                }
            }
        }
        (Value::Array(r), Value::Array(a)) => {
            for (rv, av) in r.iter().zip(a) {
                learn(rv, av, hint, map);
            }
        }
        (Value::String(r), Value::String(a)) => {
            if r != a && !a.is_empty() && looks_like_dynamic_id(r, hint) {
                map.entry(r.clone()).or_insert_with(|| a.clone());
            } else if r.starts_with('{')
                && a.starts_with('{')
                && let (Ok(rv), Ok(av)) = (serde_json::from_str::<Value>(r), serde_json::from_str::<Value>(a))
            {
                // Double-encoded data, as a character's or loadout's `data`
                learn(&rv, &av, None, map);
            }
        }
        _ => {}
    }
}

fn ordered(map: &IdMap) -> Vec<(&String, &String)> {
    let mut pairs: Vec<_> = map.iter().filter(|(k, _)| !k.is_empty()).collect();
    // Longest recorded id first, then byte order; each replacement works on the previous result (§15 item 9)
    pairs.sort_by(|(a, _), (b, _)| b.len().cmp(&a.len()).then_with(|| a.as_bytes().cmp(b.as_bytes())));
    pairs
}

fn apply_ordered(text: &str, pairs: &[(&String, &String)]) -> String {
    let mut out = text.to_owned();
    for (recorded, live) in pairs {
        if out.contains(recorded.as_str()) {
            out = out.replace(recorded.as_str(), live);
        }
    }
    out
}

/// Every mapped recorded id replaced as a plain substring.
pub fn apply_ids_str(text: &str, map: &IdMap) -> String {
    if map.is_empty() {
        return text.to_owned();
    }
    apply_ordered(text, &ordered(map))
}

/// Recursively, object keys included.
pub fn apply_ids_value(value: &Value, map: &IdMap) -> Value {
    if map.is_empty() {
        return value.clone();
    }
    apply_value(value, &ordered(map))
}

fn apply_value(value: &Value, pairs: &[(&String, &String)]) -> Value {
    match value {
        Value::String(s) => Value::String(apply_ordered(s, pairs)),
        Value::Array(items) => Value::Array(items.iter().map(|v| apply_value(v, pairs)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (apply_ordered(k, pairs), apply_value(v, pairs)))
                .collect(),
        ),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn l1_learns_both_pairs() {
        let recorded = json!({"payload": {"id": "65f1a2b3c4d5e6f7a8b9c0d1", "instanceId": "aaaa1111bbbb2222"}});
        let live = json!({"payload": {"id": "77aa00bb11cc22dd33ee44ff", "instanceId": "cccc3333dddd4444"}});
        let mut map = IdMap::new();
        learn_ids(&recorded, &live, &mut map);
        assert_eq!(map.len(), 2);
        assert_eq!(map["65f1a2b3c4d5e6f7a8b9c0d1"], "77aa00bb11cc22dd33ee44ff");
        assert_eq!(map["aaaa1111bbbb2222"], "cccc3333dddd4444");
    }

    #[test]
    fn l2_first_mapping_wins() {
        let mut map = IdMap::new();
        learn_ids(&json!({"id": "c-1"}), &json!({"id": "x-1"}), &mut map);
        learn_ids(&json!({"id": "c-1"}), &json!({"id": "x-2"}), &mut map);
        assert_eq!(map["c-1"], "x-1");
    }

    #[test]
    fn l3_invalid_is_not_learned() {
        let mut map = IdMap::new();
        learn_ids(&json!({"id": "INVALID"}), &json!({"id": "x"}), &mut map);
        learn_ids(&json!({"id": ""}), &json!({"id": "x"}), &mut map);
        learn_ids(&json!({"id": "a"}), &json!({"id": ""}), &mut map);
        assert!(map.is_empty());
    }

    #[test]
    fn l4_longest_first() {
        let map: IdMap = [("ab".into(), "X".into()), ("abc".into(), "Y".into())].into();
        assert_eq!(apply_ids_str("abcd", &map), "Yd");
    }

    #[test]
    fn shapes_without_a_hint() {
        let mut map = IdMap::new();
        let uuid = "0b9e8a56-3c3f-4d0b-9a3e-1f2d3c4b5a69";
        let recorded = json!({"name": uuid, "label": "plainword", "word": "ABCDEFGHIJKLMNOPQRSTUVWXYZ"});
        let live = json!({"name": "other", "label": "x", "word": "y"});
        learn_ids(&recorded, &live, &mut map);
        assert_eq!(map.len(), 1);
        assert_eq!(map[uuid], "other");
    }

    #[test]
    fn double_encoded_and_keys() {
        let mut map = IdMap::new();
        learn_ids(
            &json!({"data": "{\"characterId\":\"c1\"}"}),
            &json!({"data": "{\"characterId\":\"c9\"}"}),
            &mut map,
        );
        assert_eq!(map["c1"], "c9");
        let applied = apply_ids_value(&json!({"c1": ["c1x"]}), &map);
        assert_eq!(applied, json!({"c9": ["c9x"]}));
    }
}
