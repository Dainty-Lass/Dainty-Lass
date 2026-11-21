// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The character-data rules (spec §5.4 to §5.8) and the loadout write check (§5.12): pure functions over the JSON
//! text the client and the store hold, for the area, its tests and the fuzz target.

use dainty_service_kit::{js, jsjson};
use serde_json::{Map, Value};

use crate::CharacterData;

/// The appearance's `CreationState` once the creator is done (a client enum text).
pub const FACE_COMPLETE: &str = "EArchonCharacterCreationState::FaceComplete";

const PROGRESS: &str = "PlayerAccountProgressStep";
const APPEARANCE: &str = "AppearanceData";
const CHARACTER_FLAGS: &str = "CharacterFlagData";
const LOGIN_FLAGS: &str = "LoginFlagData";

/// `EPlayerAccountProgressState` (client enum names); `Final` shares 5 with `FinishedSecondHunt`.
const STEPS: [&str; 7] = [
    "New",
    "DefeatedGnasher",
    "SavedCharacter",
    "EnteredRamsgate",
    "FinishedFirstHunt",
    "FinishedSecondHunt",
    "Final",
];
const DEFEATED_GNASHER: u32 = 1;
const SAVED_CHARACTER: u32 = 2;
const ENTERED_RAMSGATE: u32 = 3;

/// 2^20 UTF-16 units (Node's cap, unreachable under the 100 KiB body limit).
pub const MAX_LOADOUT_DATA_UNITS: usize = 1_048_576;

/// Character data that is not a JSON object, or a series that does not parse; the text says which.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct DataError(pub String);

/// A step's ordinal: absent, `null`, a non-string or an unknown name is 0.
pub fn progress_ordinal(value: Option<&Value>) -> u32 {
    let Some(Value::String(name)) = value else {
        return 0;
    };
    match STEPS.iter().position(|s| s == name) {
        Some(6) => 5,
        Some(n) => u32::try_from(n).unwrap_or(0),
        None => 0,
    }
}

/// The name of an ordinal, from the first six.
pub fn progress_name(ordinal: u32) -> &'static str {
    STEPS[usize::try_from(ordinal.min(5)).unwrap_or(5)]
}

fn object_of(text: &str) -> Option<Map<String, Value>> {
    let text = if text.is_empty() { "{}" } else { text };
    match jsjson::parse(text) {
        Ok(Value::Object(map)) => Some(map),
        _ => None,
    }
}

fn parse_data(text: &str) -> Result<Map<String, Value>, DataError> {
    let text = if text.is_empty() { "{}" } else { text };
    match jsjson::parse(text) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(DataError("character data is not a JSON object".into())),
        Err(e) => Err(DataError(format!("character data: {e}"))),
    }
}

/// An object as it is, or a string that parses to one.
fn as_object(value: Option<&Value>) -> Option<Map<String, Value>> {
    match value {
        Some(Value::Object(map)) => Some(map.clone()),
        Some(Value::String(text)) => match jsjson::parse(text) {
            Ok(Value::Object(map)) => Some(map),
            _ => None,
        },
        _ => None,
    }
}

fn nullish(map: &Map<String, Value>, key: &str) -> bool {
    matches!(map.get(key), None | Some(Value::Null))
}

fn non_empty_array(value: Option<&Value>) -> Option<&Vec<Value>> {
    match value {
        Some(Value::Array(items)) if !items.is_empty() => Some(items),
        _ => None,
    }
}

fn text(map: Map<String, Value>) -> Value {
    Value::String(jsjson::stringify(&Value::Object(map)))
}

/// An appearance string entry from before the key/value format; `0` counts (spec §14 item 13).
fn legacy_string_entry(entry: &Value) -> bool {
    match entry {
        Value::Object(map) => {
            map.contains_key("CustomizationName") || map.contains_key("ValueName") || !map.contains_key("Key")
        }
        Value::Null | Value::Bool(false) => false,
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

fn appearance(object: &mut Map<String, Value>, data: &CharacterData) {
    let default = &data.default_appearance;
    let mut look = as_object(object.get(APPEARANCE)).unwrap_or_else(|| default.clone());
    look.insert("CreationState".into(), Value::from(FACE_COMPLETE));
    if non_empty_array(look.get("Data")).is_none() {
        look.insert(
            "Data".into(),
            default.get("Data").cloned().unwrap_or(Value::Array(Vec::new())),
        );
    }
    let strings_ok =
        non_empty_array(look.get("StringData")).is_some_and(|items| !items.iter().any(legacy_string_entry));
    if !strings_ok {
        let strings = default.get("StringData").cloned().unwrap_or(Value::Array(Vec::new()));
        look.insert("StringData".into(), strings);
    }
    if !matches!(look.get("AssetReferences"), Some(Value::Array(_))) {
        look.insert("AssetReferences".into(), Value::Array(Vec::new()));
    }
    object.insert(APPEARANCE.into(), text(look));
}

fn flag_key(entry: &Value) -> Option<&str> {
    entry.get("FlagKey").and_then(Value::as_str)
}

/// Appends each forced flag the blob lacks, keeping the blob's other members and their order.
fn force_flags(object: &mut Map<String, Value>, key: &str, forced: &[(String, Value)]) {
    let mut blob = as_object(object.get(key)).unwrap_or_else(|| {
        let mut empty = Map::new();
        empty.insert("Flags".into(), Value::Array(Vec::new()));
        empty
    });
    let mut flags = match blob.get("Flags") {
        Some(Value::Array(items)) => items.clone(),
        _ => Vec::new(),
    };
    for (name, value) in forced {
        if !flags.iter().any(|entry| flag_key(entry) == Some(name)) {
            let mut entry = Map::new();
            entry.insert("FlagKey".into(), Value::from(name.as_str()));
            entry.insert("FlagValue".into(), value.clone());
            flags.push(Value::Object(entry));
        }
    }
    blob.insert("Flags".into(), Value::Array(flags));
    object.insert(key.into(), text(blob));
}

/// The creator unfinished: no further than `DefeatedGnasher` and no login flag (unreachable after a normalise).
fn progression_invariant(object: &mut Map<String, Value>, data: &CharacterData) {
    let complete = as_object(object.get(APPEARANCE))
        .is_some_and(|look| look.get("CreationState").and_then(Value::as_str) == Some(FACE_COMPLETE));
    if complete {
        return;
    }
    if progress_ordinal(object.get(PROGRESS)) >= SAVED_CHARACTER {
        object.insert(PROGRESS.into(), Value::from(progress_name(DEFEATED_GNASHER)));
    }
    let login = data.forced_login_flag.0.as_str();
    object.shift_remove(login);
    if let Some(mut blob) = as_object(object.get(LOGIN_FLAGS))
        && let Some(Value::Array(flags)) = blob.get_mut("Flags")
    {
        flags.retain(|entry| flag_key(entry) != Some(login));
        object.insert(LOGIN_FLAGS.into(), text(blob));
    }
}

/// `NormalizeCharacterData`: the fills, the changelist, the returning-player state; compact text.
pub fn normalize(text: &str, data: &CharacterData, changelist: u32, now_secs: i64) -> Result<String, DataError> {
    let mut object = parse_data(text)?;
    for (key, value) in &data.fill {
        if nullish(&object, key) {
            object.insert(key.clone(), value.clone());
        }
    }
    let login = login_time(now_secs);
    for key in ["LoginTime", "LastLoginTime"] {
        if nullish(&object, key) {
            object.insert(key.into(), Value::from(login.as_str()));
        }
    }
    object.insert("LastChangelist".into(), Value::from(changelist.to_string()));
    // Every account reads as onboarded, in Ramsgate: 1.14.7's tutorial hunt starves on beginner-loadout data
    if progress_ordinal(object.get(PROGRESS)) < ENTERED_RAMSGATE {
        object.insert(PROGRESS.into(), Value::from(progress_name(ENTERED_RAMSGATE)));
    }
    appearance(&mut object, data);
    force_flags(&mut object, CHARACTER_FLAGS, &data.forced_character_flags);
    force_flags(&mut object, LOGIN_FLAGS, std::slice::from_ref(&data.forced_login_flag));
    progression_invariant(&mut object, data);
    Ok(jsjson::stringify(&Value::Object(object)))
}

/// The step moves forward only, one at a time; text that is not an object passes unchanged.
pub fn guard_progression(incoming: &str, existing: &str) -> String {
    let (Some(mut next), Some(current)) = (object_of(incoming), object_of(existing)) else {
        tracing::warn!("[progression-guard] Non-JSON incoming data or parse error - skipping guard");
        return incoming.to_owned();
    };
    let (from, to) = (
        progress_ordinal(current.get(PROGRESS)),
        progress_ordinal(next.get(PROGRESS)),
    );
    if to < from {
        tracing::warn!("[progression-guard] Rejecting backward progression; keeping existing");
        next.insert(PROGRESS.into(), current.get(PROGRESS).cloned().unwrap_or(Value::Null));
    } else if to > from + 1 {
        let name = progress_name(from + 1);
        tracing::warn!("[progression-guard] Rejecting skip; clamped to {name}");
        next.insert(PROGRESS.into(), Value::from(name));
    }
    jsjson::stringify(&Value::Object(next))
}

fn dojo_text(data: &CharacterData) -> Value {
    text(data.dojo_series.clone())
}

/// `ProcessTriggers`: normalise, then the trigger series' node at the trigger status sets the Dojo series.
pub fn process_triggers(text: &str, data: &CharacterData, changelist: u32, now_secs: i64) -> Result<String, DataError> {
    let normalized = normalize(text, data, changelist, now_secs)?;
    let mut object = parse_data(&normalized)?;
    if let Some(value) = object.get(&data.trigger_series_key).filter(|v| !v.is_null()) {
        let source = match value {
            Value::String(s) => s.clone(),
            other => js::string(Some(other)),
        };
        let series = jsjson::parse(&source).map_err(|e| DataError(format!("{}: {e}", data.trigger_series_key)))?;
        let status = series
            .get(&data.trigger_node)
            .and_then(|node| node.get("Status"))
            .and_then(Value::as_f64);
        if status == Some(data.trigger_status) && nullish(&object, &data.dojo_series_key) {
            tracing::info!("Injecting SERIE_dojo!");
            object.insert(data.dojo_series_key.clone(), dojo_text(data));
        }
    }
    Ok(jsjson::stringify(&Value::Object(object)))
}

/// `EnsureLegacyDojoSeries`: the normalised text, with the Dojo series when the flag is on; and whether it was added.
pub fn read_form(
    text: &str,
    data: &CharacterData,
    changelist: u32,
    now_secs: i64,
    dojo_unlocked: bool,
) -> Result<(String, bool), DataError> {
    let normalized = normalize(text, data, changelist, now_secs)?;
    if !dojo_unlocked {
        return Ok((normalized, false));
    }
    let mut object = parse_data(&normalized)?;
    if !nullish(&object, &data.dojo_series_key) {
        return Ok((normalized, false));
    }
    object.insert(data.dojo_series_key.clone(), dojo_text(data));
    Ok((jsjson::stringify(&Value::Object(object)), true))
}

fn claimed(node: &Value) -> bool {
    matches!(node.get("node_status").and_then(Value::as_f64), Some(s) if s == 1.0 || s == 2.0)
}

/// `IsDojoUnlockedForUser` over the journey's `nodes` text; a capture's map key may differ from the node id.
pub fn dojo_unlocked(nodes: &str, data: &CharacterData) -> bool {
    let Some(nodes) = object_of(nodes) else {
        return false;
    };
    let id = data.dojo_node_id.as_str();
    nodes.get(id).is_some_and(claimed)
        || nodes
            .values()
            .any(|node| node.get("node_id").and_then(Value::as_str) == Some(id) && claimed(node))
}

/// `IsFreshOnboarding` over an account's live characters' `data`: fresh unless one has finished the creator and
/// reached `SavedCharacter`.
pub fn is_fresh<'a>(datas: impl IntoIterator<Item = &'a str>) -> bool {
    !datas.into_iter().any(|text| {
        let Ok(Value::Object(object)) = jsjson::parse(text) else {
            return false;
        };
        let complete = as_object(object.get(APPEARANCE))
            .is_some_and(|look| look.get("CreationState").and_then(Value::as_str) == Some(FACE_COMPLETE));
        complete && progress_ordinal(object.get(PROGRESS)) >= SAVED_CHARACTER
    })
}

fn utc(secs: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(secs, 0).unwrap_or_default()
}

/// `YYYY.MM.DD-HH.MM.SS` in UTC.
pub fn login_time(secs: i64) -> String {
    utc(secs).format("%Y.%m.%d-%H.%M.%S").to_string()
}

/// `toLocaleDateString("en-US", {month: "short", day: "numeric", year: "numeric"})` in UTC: `Oct 14, 2026`.
pub fn display_date(secs: i64) -> String {
    utc(secs).format("%b %-d, %Y").to_string()
}

/// `ValidateLoadoutWriteData`: the first reason the write is refused, or none.
pub fn validate_loadout_write(index: &str, data: Option<&Value>) -> Option<String> {
    if index != "persistent" && !matches!(index, "0" | "1" | "2" | "3" | "4" | "5") {
        return Some(format!("unsupported loadout index {}", index.escape_debug()));
    }
    let Some(Value::String(text)) = data.filter(|d| d.as_str().is_some_and(|s| !s.is_empty())) else {
        return Some("data must be a non-empty JSON string".into());
    };
    if text.encode_utf16().count() > MAX_LOADOUT_DATA_UNITS {
        return Some("data is too large".into());
    }
    match jsjson::parse(text) {
        Err(_) => Some("data is not valid JSON".into()),
        Ok(Value::Object(_)) => None,
        Ok(_) => Some("data must be a JSON object".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(login_time(0), "1970.01.01-00.00.00");
        assert_eq!(login_time(1_792_000_000), "2026.10.14-17.46.40");
        assert_eq!(display_date(1_792_000_000), "Oct 14, 2026");
        assert_eq!(display_date(951_782_400), "Feb 29, 2000");
        assert_eq!(display_date(0), "Jan 1, 1970");
        assert_eq!(login_time(-1), "1969.12.31-23.59.59");
        assert_eq!(login_time(951_868_799), "2000.02.29-23.59.59");
        assert_eq!(display_date(1_704_067_199), "Dec 31, 2023");
        // Extremes stay arithmetic, never a panic
        let _ = (login_time(i64::MAX), login_time(i64::MIN), display_date(i64::MIN));
    }

    #[test]
    fn ordinals() {
        assert_eq!(progress_ordinal(Some(&Value::from("Final"))), 5);
        assert_eq!(progress_ordinal(Some(&Value::from("EnteredRamsgate"))), 3);
        assert_eq!(progress_ordinal(Some(&Value::from(3))), 0);
        assert_eq!(progress_ordinal(Some(&Value::Null)), 0);
        assert_eq!(progress_ordinal(None), 0);
        assert_eq!(progress_name(5), "FinishedSecondHunt");
        assert_eq!(progress_name(9), "FinishedSecondHunt");
    }

    #[test]
    fn loadout_writes() {
        let ok = Value::from("{\"a\":1}");
        assert_eq!(validate_loadout_write("0", Some(&ok)), None);
        assert_eq!(validate_loadout_write("persistent", Some(&ok)), None);
        for index in ["6", "01", "+1", "-0", " 1", "x", ""] {
            assert_eq!(
                validate_loadout_write(index, Some(&ok)),
                Some(format!("unsupported loadout index {index}"))
            );
        }
        let reason = |data: Option<&Value>| validate_loadout_write("1", data).unwrap();
        assert_eq!(reason(None), "data must be a non-empty JSON string");
        assert_eq!(reason(Some(&Value::from(""))), "data must be a non-empty JSON string");
        assert_eq!(reason(Some(&Value::from(5))), "data must be a non-empty JSON string");
        assert_eq!(reason(Some(&Value::from("nope"))), "data is not valid JSON");
        assert_eq!(reason(Some(&Value::from("[1]"))), "data must be a JSON object");
        let big = format!("{{\"a\":\"{}\"}}", "x".repeat(MAX_LOADOUT_DATA_UNITS));
        assert_eq!(reason(Some(&Value::from(big))), "data is too large");
        assert_eq!(
            validate_loadout_write("a\nb", Some(&ok)).unwrap(),
            "unsupported loadout index a\\nb"
        );
    }
}
