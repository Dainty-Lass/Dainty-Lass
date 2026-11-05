//! What Rustic Isles' characters area reads from players and its own rows (spec-rustic-isles-characters §16):
//! arbitrary text as character data, the stored data it is guarded against, a trigger series, a loadout write and
//! its index, journey nodes and a stored item list, with a synthetic `CharacterData` and `StarterKit`. Nothing
//! panics; a normalise that succeeds is an object and a fixed point; the guard never moves the step back or more
//! than one forward; a loadout write is accepted only as an object at an allowed index; a repair gives every
//! instanced catalog id a distinct id, valid wherever it chose one.
#![no_main]

use std::collections::HashSet;

use libfuzzer_sys::fuzz_target;
use rustic_isles_characters::data::{
    dojo_unlocked, guard_progression, is_fresh, normalize, process_triggers, progress_ordinal, read_form,
    validate_loadout_write,
};
use rustic_isles_characters::starter::{is_instance_id, repair};
use rustic_isles_characters::{CharacterData, StarterKit};
use serde_json::{Map, Value, json};
use service_kit::jsjson;

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

fn data() -> CharacterData {
    let look = json!({"CreationState": "None", "Data": [1], "StringData": [{"Key": "K", "Value": "V"}]});
    CharacterData {
        fill: vec![
            ("RecentPlayers".into(), json!("[]")),
            ("AppearanceData".into(), Value::from(look.to_string())),
        ],
        default_appearance: object(look),
        forced_character_flags: vec![("Flag_A".into(), json!("True")), ("Flag_B".into(), json!("True"))],
        forced_login_flag: ("Login_Seen".into(), json!("true")),
        dojo_series_key: "SERIE_dojo".into(),
        dojo_series: object(json!({"Series": "S"})),
        dojo_node_id: "Dojo".into(),
        trigger_series_key: "SERIE_trigger".into(),
        trigger_node: "Node_T".into(),
        trigger_status: 3.0,
    }
}

fn kit() -> StarterKit {
    let ids = ["WP", "PART_A", "HELM", "FLASK", "BANNER", "GLIDER"];
    StarterKit {
        instanced: ids.iter().map(|s| (*s).to_owned()).collect(),
        default_weapon_parts: vec!["PART_A".into()],
        equipment: vec![("weapon".into(), "WP".into()), ("helmet".into(), "HELM".into())],
        instance_data: vec![],
        flask: "FLASK".into(),
        banner: "BANNER".into(),
        glider: "GLIDER".into(),
        banner_customization: Map::new(),
        intro_emote: "E".into(),
        flare: "F".into(),
        default_quick_chats: vec![String::new(); 9],
        wallet: vec![],
        default_appearance: Map::new(),
    }
}

fn as_object(text: &str) -> Option<Map<String, Value>> {
    match jsjson::parse(if text.is_empty() { "{}" } else { text }) {
        Ok(Value::Object(map)) => Some(map),
        _ => None,
    }
}

fuzz_target!(|bytes: &[u8]| {
    let text = String::from_utf8_lossy(bytes);
    let mut parts = text.splitn(5, '\u{1}');
    let blob = parts.next().unwrap_or("");
    let existing = parts.next().unwrap_or("");
    let write = parts.next().unwrap_or("");
    let index = parts.next().unwrap_or("");
    let items = parts.next().unwrap_or("");
    let data = data();
    let now = i64::from(bytes.first().copied().unwrap_or(0)) * 86_400;

    if let Ok(once) = normalize(blob, &data, 647_472, now) {
        assert!(as_object(&once).is_some());
        assert_eq!(normalize(&once, &data, 647_472, now + 3_600).ok().as_deref(), Some(once.as_str()));
    }

    let guarded = guard_progression(blob, existing);
    if let (Some(_), Some(current)) = (as_object(blob), as_object(existing)) {
        let after = as_object(&guarded).expect("the guard answers an object");
        let after = progress_ordinal(after.get("PlayerAccountProgressStep"));
        let from = progress_ordinal(current.get("PlayerAccountProgressStep"));
        assert!(after >= from && after <= from + 1, "{from} -> {after}");
    }

    let _ = read_form(blob, &data, 647_472, now, true);
    let _ = process_triggers(blob, &data, 647_472, now);
    let _ = process_triggers(&json!({"SERIE_trigger": existing}).to_string(), &data, 647_472, now);
    let _ = dojo_unlocked(existing, &data);
    let _ = is_fresh([blob, existing]);

    let sent = Value::from(write);
    if validate_loadout_write(index, Some(&sent)).is_none() {
        assert!(index == "persistent" || matches!(index, "0" | "1" | "2" | "3" | "4" | "5"));
        assert!(matches!(jsjson::parse(write), Ok(Value::Object(_))));
    }

    let kit = kit();
    let list = match jsjson::parse(items) {
        Ok(Value::Array(list)) => list,
        Ok(other) => vec![other],
        Err(_) => Vec::new(),
    };
    let known: HashSet<String> = list
        .iter()
        .filter_map(|i| i.get("catalogId").and_then(Value::as_str))
        .filter(|c| {
            list.iter().any(|i| {
                i.get("catalogId").and_then(Value::as_str) == Some(*c)
                    && i.get("instanceId").and_then(Value::as_str).is_some_and(|id| !id.is_empty())
            })
        })
        .map(str::to_owned)
        .collect();
    let source = jsjson::parse(existing).ok();
    let repaired = repair(list, source.as_ref(), &kit);
    let mut seen = HashSet::new();
    for catalog in &kit.instanced {
        let id = repaired.ids.get(catalog).expect("every instanced catalog id has an id");
        assert!(seen.insert(id.clone()), "{id} given twice");
        if !known.contains(catalog) {
            assert!(is_instance_id(id), "{id}");
        }
    }
    assert!(repaired.items.iter().all(Value::is_object));
});
