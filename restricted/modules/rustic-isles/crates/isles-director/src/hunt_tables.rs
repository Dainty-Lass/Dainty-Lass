//! RB11 Rust port of `ParadoxDirector/scripts/import_hunt_tables.cjs`: merges a CatalogExporter
//! `EXPORT_HUNTS=1` capture (`Items_Analysis/hunts_<ver>/`) into the Director's hunt tables.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde_json::{Map, Value, json};
use service_kit::jsjson;

pub const MAIN_PLAYER_TABLE: &str = "player_hunts_table";
pub const MAIN_MATCHMAKER_TABLE: &str = "matchmaker_hunts_table";
pub const DEFAULT_EXPORT: &str = "hunts_1_14_7";
pub const SUPPORTED_VERSIONS: [&str; 2] = ["1.12.0", "1.14.7"];

const UNEXPORTED_FIELDS: [&str; 3] = ["HuntName", "HuntDescription", "AdditionalBehemoths"];
const PROVENANCE_FIELDS: [&str; 2] = ["_importedFrom", "_updatedFrom"];
const FNAME_FIELDS: [&str; 10] = [
    "RowName",
    "HuntTags",
    "TagName",
    "Modifiers",
    "AssetPathName",
    "HuntSuccessReward",
    "FirstCompletionSuccessReward",
    "TargetedHuntSuccessReward",
    "HuntFailureReward",
    "TargetedHuntFailureReward",
];

const PLAYER_ROW_ORDER: &[&str] = &[
    "HuntName",
    "HuntDescription",
    "Region",
    "MatchmakerHuntIDs",
    "MatchmakerHuntsByTag",
    "MatchmakingType",
    "MatchmakingGameType",
    "HuntTags",
    "EscalationModeSpecification",
    "EscalationPatrolInitialChallengeLevel",
    "EscalationPatrolMaxChallengeLevel",
    "RecommendedEscalationLevel",
    "bHasPortals",
    "bHasGlitterEvent",
    "bHasPhaelanxEvent",
    "HuntSuccessReward",
    "FirstCompletionSuccessReward",
    "TargetedHuntSuccessReward",
    "HuntFailureReward",
    "TargetedHuntFailureReward",
    "MinWeaponSkillLevel",
    "RecomendedWeaponSkillLevel",
    "IsVisibleWhileLocked",
    "_importedFrom",
];

const MATCHMAKER_ROW_ORDER: &[&str] = &[
    "Region",
    "HuntTags",
    "HuntThreatLevel",
    "GameModeOverride",
    "DangerPerSecOverride",
    "SpecificBehemoth",
    "AdditionalBehemoths",
    "AdditionalSpecificBehemoths",
    "MapMetaData",
    "MapList",
    "Modifiers",
    "GatherableTier",
    "MaxPlayers",
    "bIsGeneratedEncounter",
    "GameModeSpecificData",
    "_importedFrom",
];

const MAP_ENTRY_ORDER: &[&str] = &["MapName", "MapAssetName", "Biome", "Weighting"];
const BEHEMOTH_ORDER: &[&str] = &["BehemothName", "BehemothAsset", "PowerOverride", "Weighting"];

#[derive(Debug, Clone)]
pub struct ImportHuntTablesOptions {
    pub export_dir: PathBuf,
    pub vendor_dir: PathBuf,
    pub repo_root: Option<PathBuf>,
    pub apply: bool,
    pub update_existing: bool,
    pub verbose: bool,
    pub backup_stamp: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TableMergeSummary {
    pub table_name: String,
    pub table_path: PathBuf,
    pub is_new_file: bool,
    pub added: Vec<String>,
    pub updated: Vec<(String, Vec<String>)>,
    pub normalised: Vec<String>,
    pub rejected: Vec<String>,
    pub vendor_only: Vec<String>,
    pub will_change: bool,
}

#[derive(Debug, Clone)]
pub struct ImportHuntTablesReport {
    pub export_version: String,
    pub tables: Vec<TableMergeSummary>,
    pub conversion_problems: Vec<String>,
    pub output: String,
}

struct EnumSpec {
    prefix: &'static str,
    always_name: bool,
    values: Option<&'static [&'static str]>,
}

const MATCHMAKING_GAME_TYPE_1_12_0: &[&str] = &[
    "None",
    "LegacyHunt",
    "City",
    "HuntingGround",
    "Escalation",
    "Trial",
    "TrainingGround",
    "Mission",
    "Event",
    "FTUE",
];

const MATCHMAKING_GAME_TYPE_1_14_7: &[&str] = &[
    "None",
    "LegacyHunt",
    "City",
    "HuntingGround",
    "Escalation",
    "Trial",
    "TrainingGround",
    "Mission",
    "Event",
    "Gauntlet",
    "FTUE",
];

const MATCHMAKING_TYPE_VALUES: &[&str] = &["Reserved", "Open", "City"];
const BIOME_VALUES: &[&str] = &["Moss", "Ice", "Arid"];
const GATHERABLE_TIER_VALUES: &[&str] = &["Tier0", "Tier1", "Tier2", "Tier3", "Tier4"];

fn enum_of(field: &str) -> Option<EnumSpec> {
    match field {
        "MatchmakingGameType" => Some(EnumSpec {
            prefix: "EMatchmakingGameType",
            always_name: true,
            values: None,
        }),
        "MatchmakingType" => Some(EnumSpec {
            prefix: "EMatchmakingType",
            always_name: false,
            values: Some(MATCHMAKING_TYPE_VALUES),
        }),
        "Biome" => Some(EnumSpec {
            prefix: "EHuntBiome",
            always_name: false,
            values: Some(BIOME_VALUES),
        }),
        "GatherableTier" => Some(EnumSpec {
            prefix: "EGatherableTier",
            always_name: false,
            values: Some(GATHERABLE_TIER_VALUES),
        }),
        _ => None,
    }
}

fn enum_values_for<'a>(spec: &EnumSpec, version: &str) -> Option<&'a [&'static str]> {
    if let Some(values) = spec.values {
        return Some(values);
    }
    match version {
        "1.12.0" => Some(MATCHMAKING_GAME_TYPE_1_12_0),
        "1.14.7" => Some(MATCHMAKING_GAME_TYPE_1_14_7),
        _ => None,
    }
}

fn enum_name(field: &str, value: Option<&Value>, version: &str) -> Option<&'static str> {
    let spec = enum_of(field)?;
    let value = value?;
    match value {
        Value::String(s) => {
            let name = s.rsplit_once("::").map_or(s.as_str(), |(_, n)| n);
            if let Some(values) = spec.values {
                values.iter().copied().find(|v| *v == name)
            } else {
                MATCHMAKING_GAME_TYPE_1_14_7
                    .iter()
                    .chain(MATCHMAKING_GAME_TYPE_1_12_0.iter())
                    .copied()
                    .find(|v| *v == name)
            }
        }
        Value::Number(n) => {
            let f = n.as_f64()?;
            if f < 0.0 || f.fract() != 0.0 {
                return None;
            }
            let idx = f as usize;
            let values = enum_values_for(&spec, version)?;
            values.get(idx).copied()
        }
        _ => None,
    }
}

fn row_version(row: &Map<String, Value>) -> String {
    let marker = row
        .get("_importedFrom")
        .or_else(|| row.get("_updatedFrom"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if let Some(rest) = marker.split("EXPORT_HUNTS ").nth(1) {
        let ver: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
        if !ver.is_empty() {
            return ver;
        }
    }
    "1.12.0".to_owned()
}

fn read_jsonl(dir: &Path, file_name: &str) -> Result<Option<Vec<Value>>, String> {
    let full_path = dir.join(file_name);
    if !full_path.exists() {
        return Ok(None);
    }
    let text =
        std::fs::read_to_string(&full_path).map_err(|e| format!("failed to read {}: {e}", full_path.display()))?;
    let trimmed = text.strip_prefix('\u{feff}').unwrap_or(&text).trim();
    if trimmed.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let mut rows = Vec::new();
    for (idx, line) in trimmed.split('\n').enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parsed = jsjson::parse(line)
            .map_err(|e| format!("malformed JSON on line {} of {}: {e}", idx + 1, full_path.display()))?;
        rows.push(parsed);
    }
    Ok(Some(rows))
}

fn read_json(full_path: &Path) -> Result<Value, String> {
    let text =
        std::fs::read_to_string(full_path).map_err(|e| format!("failed to read {}: {e}", full_path.display()))?;
    let stripped = text.strip_prefix('\u{feff}').unwrap_or(&text);
    jsjson::parse(stripped).map_err(|e| format!("invalid JSON in {}: {e}", full_path.display()))
}

fn table_from_object_name(object_name: Option<&Value>) -> Option<String> {
    let s = object_name?.as_str()?;
    let inner = s.strip_prefix("DataTable'")?.strip_suffix('\'')?;
    Some(inner.to_owned())
}

fn index_object_paths(value: &Value, known: &mut HashMap<String, String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                index_object_paths(item, known);
            }
        }
        Value::Object(map) => {
            if let Some(table) = table_from_object_name(map.get("ObjectName"))
                && let Some(Value::String(obj_path)) = map.get("ObjectPath")
                && !known.contains_key(&table)
            {
                known.insert(table, obj_path.clone());
            }
            for child in map.values() {
                index_object_paths(child, known);
            }
        }
        _ => {}
    }
}

fn table_ref(table_name: &str, known: &HashMap<String, String>) -> Value {
    let obj_path = known
        .get(table_name)
        .cloned()
        .unwrap_or_else(|| format!("Archon/Content/Gameplay/hunts/{table_name}.0"));
    json!({
        "ObjectName": format!("DataTable'{table_name}'"),
        "ObjectPath": obj_path
    })
}

fn handle(table_name: Option<&str>, row_name: Option<&str>, known: &HashMap<String, String>) -> Value {
    let Some(row_name) = row_name.filter(|r| !r.is_empty() && *r != "None") else {
        return json!({ "DataTable": null, "RowName": "None" });
    };
    let table_name = table_name.unwrap_or("undefined");
    json!({
        "DataTable": table_ref(table_name, known),
        "RowName": row_name
    })
}

fn handle_from_export(exported: Option<&Value>, known: &HashMap<String, String>) -> Value {
    let Some(Value::Object(map)) = exported else {
        return json!({ "DataTable": null, "RowName": "None" });
    };
    handle(
        map.get("table").and_then(Value::as_str),
        map.get("rowName").and_then(Value::as_str),
        known,
    )
}

fn encode_enum(
    field: &str,
    row_name: &str,
    value: Option<&Value>,
    export_version: &str,
    problems: &mut Vec<String>,
) -> Option<Value> {
    let spec = enum_of(field)?;
    let Some(name) = enum_name(field, value, export_version) else {
        let val_str = value.map(jsjson::stringify).unwrap_or_else(|| "undefined".into());
        problems.push(format!(
            "{row_name}: {field}={val_str} is not a {} value in {export_version}",
            spec.prefix
        ));
        return value.cloned();
    };
    if spec.always_name {
        Some(Value::String(format!("{}::{name}", spec.prefix)))
    } else {
        value.cloned()
    }
}

fn to_vendor_tag_routed_list(list: &Value, known: &HashMap<String, String>) -> Value {
    let obj = list.as_object();
    let table_name = obj
        .and_then(|m| m.get("matchmakerTable"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let matchmaker_table = if !table_name.is_empty() {
        table_ref(table_name, known)
    } else {
        Value::Null
    };
    let queries = obj
        .and_then(|m| m.get("queries"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let hunt_tags: Vec<Value> = queries
        .into_iter()
        .map(|q| {
            let qm = q.as_object();
            let token_stream_version = qm
                .and_then(|m| m.get("tokenStreamVersion"))
                .filter(|v| !v.is_null())
                .cloned()
                .unwrap_or_else(|| json!(0));
            let tag_dict = qm
                .and_then(|m| m.get("tagDictionary"))
                .and_then(Value::as_array)
                .map(|arr| arr.iter().map(|t| json!({ "TagName": t })).collect::<Vec<_>>())
                .unwrap_or_default();
            let query_token_stream = qm
                .and_then(|m| m.get("tokenStream"))
                .filter(|v| !v.is_null())
                .cloned()
                .unwrap_or_else(|| json!([]));
            let user_desc = qm
                .and_then(|m| m.get("userDescription"))
                .filter(|v| !v.is_null())
                .cloned()
                .unwrap_or_else(|| json!(""));
            let auto_desc = qm
                .and_then(|m| m.get("autoDescription"))
                .filter(|v| !v.is_null())
                .cloned()
                .unwrap_or_else(|| json!(""));
            json!({
                "TokenStreamVersion": token_stream_version,
                "TagDictionary": tag_dict,
                "QueryTokenStream": query_token_stream,
                "UserDescription": user_desc,
                "AutoDescription": auto_desc
            })
        })
        .collect();
    json!({
        "MatchmakerTable": matchmaker_table,
        "HuntTags": hunt_tags
    })
}

fn insert_opt(out: &mut Map<String, Value>, key: &str, val: Option<Value>) {
    if let Some(v) = val {
        out.insert(key.to_owned(), v);
    }
}

fn to_vendor_player_row(
    row: &Map<String, Value>,
    export_version: &str,
    provenance_marker: &str,
    known: &HashMap<String, String>,
    problems: &mut Vec<String>,
) -> Map<String, Value> {
    let row_name = row.get("rowName").and_then(Value::as_str).unwrap_or("");
    let mut out = Map::new();
    out.insert("HuntName".into(), json!({ "CultureInvariantString": "" }));
    out.insert("HuntDescription".into(), json!({ "CultureInvariantString": "" }));
    out.insert("Region".into(), handle_from_export(row.get("region"), known));
    let mm_ids = row
        .get("matchmakerHuntIDs")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .map(|e| handle_from_export(Some(e), known))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    out.insert("MatchmakerHuntIDs".into(), Value::Array(mm_ids));
    let mm_by_tag = row
        .get("matchmakerHuntsByTag")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .map(|l| to_vendor_tag_routed_list(l, known))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    out.insert("MatchmakerHuntsByTag".into(), Value::Array(mm_by_tag));
    insert_opt(
        &mut out,
        "MatchmakingType",
        encode_enum(
            "MatchmakingType",
            row_name,
            row.get("matchmakingType"),
            export_version,
            problems,
        ),
    );
    insert_opt(
        &mut out,
        "MatchmakingGameType",
        encode_enum(
            "MatchmakingGameType",
            row_name,
            row.get("matchmakingGameType"),
            export_version,
            problems,
        ),
    );
    out.insert(
        "HuntTags".into(),
        row.get("huntTags")
            .filter(|v| !v.is_null())
            .cloned()
            .unwrap_or_else(|| json!([])),
    );
    out.insert(
        "EscalationModeSpecification".into(),
        handle_from_export(row.get("escalationModeSpecification"), known),
    );
    insert_opt(
        &mut out,
        "EscalationPatrolInitialChallengeLevel",
        row.get("escalationPatrolInitialChallengeLevel").cloned(),
    );
    insert_opt(
        &mut out,
        "EscalationPatrolMaxChallengeLevel",
        row.get("escalationPatrolMaxChallengeLevel").cloned(),
    );
    insert_opt(
        &mut out,
        "RecommendedEscalationLevel",
        row.get("recommendedEscalationLevel").cloned(),
    );
    insert_opt(&mut out, "bHasPortals", row.get("hasPortals").cloned());
    insert_opt(&mut out, "bHasGlitterEvent", row.get("hasGlitterEvent").cloned());
    insert_opt(&mut out, "bHasPhaelanxEvent", row.get("hasPhaelanxEvent").cloned());
    insert_opt(&mut out, "HuntSuccessReward", row.get("huntSuccessReward").cloned());
    insert_opt(
        &mut out,
        "FirstCompletionSuccessReward",
        row.get("firstCompletionSuccessReward").cloned(),
    );
    insert_opt(
        &mut out,
        "TargetedHuntSuccessReward",
        row.get("targetedHuntSuccessReward").cloned(),
    );
    insert_opt(&mut out, "HuntFailureReward", row.get("huntFailureReward").cloned());
    insert_opt(
        &mut out,
        "TargetedHuntFailureReward",
        row.get("targetedHuntFailureReward").cloned(),
    );
    insert_opt(&mut out, "MinWeaponSkillLevel", row.get("minWeaponSkillLevel").cloned());
    insert_opt(
        &mut out,
        "RecomendedWeaponSkillLevel",
        row.get("recommendedWeaponSkillLevel").cloned(),
    );
    insert_opt(&mut out, "IsVisibleWhileLocked", row.get("visibleWhileLocked").cloned());
    out.insert("_importedFrom".into(), Value::String(provenance_marker.to_owned()));
    out
}

fn to_vendor_map_entry(map_val: &Value, row_name: &str, export_version: &str, problems: &mut Vec<String>) -> Value {
    let map = map_val.as_object();
    let asset_path = map
        .and_then(|m| m.get("mapAssetPath"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| map.and_then(|m| m.get("mapAssetName")).and_then(Value::as_str))
        .unwrap_or("");
    let map_name = map
        .and_then(|m| m.get("mapName"))
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or_else(|| json!(""));
    let mut out = Map::new();
    out.insert("MapName".into(), map_name);
    out.insert("MapAssetName".into(), Value::String(asset_path.to_owned()));
    insert_opt(
        &mut out,
        "Biome",
        encode_enum(
            "Biome",
            row_name,
            map.and_then(|m| m.get("biome")),
            export_version,
            problems,
        ),
    );
    insert_opt(&mut out, "Weighting", map.and_then(|m| m.get("weighting")).cloned());
    Value::Object(out)
}

fn to_vendor_behemoth(behemoth_val: Option<&Value>) -> Value {
    let b = behemoth_val.and_then(Value::as_object);
    let behemoth_name = b
        .and_then(|m| m.get("behemothName"))
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or_else(|| json!(""));
    let asset_path = b
        .and_then(|m| m.get("behemothAssetPath"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("None");
    let mut out = Map::new();
    out.insert("BehemothName".into(), behemoth_name);
    out.insert(
        "BehemothAsset".into(),
        json!({
            "AssetPathName": asset_path,
            "SubPathString": ""
        }),
    );
    insert_opt(
        &mut out,
        "PowerOverride",
        b.and_then(|m| m.get("powerOverride")).cloned(),
    );
    insert_opt(&mut out, "Weighting", b.and_then(|m| m.get("weighting")).cloned());
    Value::Object(out)
}

fn to_vendor_matchmaker_row(
    row: &Map<String, Value>,
    export_version: &str,
    provenance_marker: &str,
    known: &HashMap<String, String>,
    problems: &mut Vec<String>,
) -> Map<String, Value> {
    let row_name = row.get("rowName").and_then(Value::as_str).unwrap_or("");
    let mut out = Map::new();
    out.insert("Region".into(), handle_from_export(row.get("region"), known));
    out.insert(
        "HuntTags".into(),
        row.get("huntTags")
            .filter(|v| !v.is_null())
            .cloned()
            .unwrap_or_else(|| json!([])),
    );
    insert_opt(&mut out, "HuntThreatLevel", row.get("huntThreatLevel").cloned());
    out.insert(
        "GameModeOverride".into(),
        row.get("gameModeOverride")
            .filter(|v| !v.is_null())
            .cloned()
            .unwrap_or_else(|| json!("")),
    );
    insert_opt(
        &mut out,
        "DangerPerSecOverride",
        row.get("dangerPerSecOverride").cloned(),
    );
    out.insert(
        "SpecificBehemoth".into(),
        to_vendor_behemoth(row.get("specificBehemoth")),
    );
    out.insert("AdditionalBehemoths".into(), json!([]));
    let add_spec = row
        .get("additionalSpecificBehemoths")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().map(|b| to_vendor_behemoth(Some(b))).collect::<Vec<_>>())
        .unwrap_or_default();
    out.insert("AdditionalSpecificBehemoths".into(), Value::Array(add_spec));
    out.insert("MapMetaData".into(), handle_from_export(row.get("mapMetaData"), known));
    let map_list = row
        .get("mapList")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .map(|m| to_vendor_map_entry(m, row_name, export_version, problems))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    out.insert("MapList".into(), Value::Array(map_list));
    out.insert(
        "Modifiers".into(),
        row.get("modifiers")
            .filter(|v| !v.is_null())
            .cloned()
            .unwrap_or_else(|| json!([])),
    );
    insert_opt(
        &mut out,
        "GatherableTier",
        encode_enum(
            "GatherableTier",
            row_name,
            row.get("gatherableTier"),
            export_version,
            problems,
        ),
    );
    insert_opt(&mut out, "MaxPlayers", row.get("maxPlayers").cloned());
    insert_opt(
        &mut out,
        "bIsGeneratedEncounter",
        row.get("isGeneratedEncounter").cloned(),
    );
    out.insert(
        "GameModeSpecificData".into(),
        handle_from_export(row.get("gameModeSpecificData"), known),
    );
    out.insert("_importedFrom".into(), Value::String(provenance_marker.to_owned()));
    out
}

fn is_handle(value: &Value) -> bool {
    matches!(value, Value::Object(m) if m.contains_key("RowName") && m.contains_key("DataTable"))
}

fn is_object_ref(value: &Value) -> bool {
    matches!(value, Value::Object(m) if m.contains_key("ObjectName") && m.contains_key("ObjectPath"))
}

fn handle_key(value: &Value) -> String {
    let Some(m) = value.as_object() else {
        return "None".into();
    };
    let row_name = m.get("RowName").and_then(Value::as_str).unwrap_or("");
    if row_name.is_empty() || row_name == "None" {
        return "None".into();
    }
    let dt_obj_name = m
        .get("DataTable")
        .and_then(Value::as_object)
        .and_then(|dt| dt.get("ObjectName"));
    let table = table_from_object_name(dt_obj_name).unwrap_or_else(|| "?".into());
    format!("{table}/{row_name}").to_lowercase()
}

struct MergeCtx<'a> {
    existing_version: String,
    export_version: &'a str,
}

fn same(a: &Value, b: &Value, field: &str, ctx: &MergeCtx<'_>) -> bool {
    if enum_of(field).is_some() {
        let left = enum_name(field, Some(a), &ctx.existing_version);
        let right = enum_name(field, Some(b), ctx.export_version);
        return left.is_some() && left == right;
    }
    if a == b {
        return true;
    }
    if let (Value::String(sa), Value::String(sb)) = (a, b)
        && FNAME_FIELDS.contains(&field)
    {
        return sa.to_lowercase() == sb.to_lowercase();
    }
    if let (Value::Number(na), Value::Number(nb)) = (a, b)
        && let (Some(fa), Some(fb)) = (na.as_f64(), nb.as_f64())
    {
        return (fa - fb).abs() <= 1e-6 * 1.0_f64.max(fa.abs()).max(fb.abs());
    }
    if let (Value::Array(aa), Value::Array(ab)) = (a, b) {
        return aa.len() == ab.len() && aa.iter().zip(ab.iter()).all(|(ia, ib)| same(ia, ib, field, ctx));
    }
    if let (Value::Object(ma), Value::Object(mb)) = (a, b) {
        if is_handle(a) || is_handle(b) {
            return is_handle(a) && is_handle(b) && handle_key(a) == handle_key(b);
        }
        if is_object_ref(a) || is_object_ref(b) {
            return ma.get("ObjectName") == mb.get("ObjectName");
        }
        return mb.iter().all(|(k, vb)| {
            UNEXPORTED_FIELDS.contains(&k.as_str()) || ma.get(k).is_some_and(|va| same(va, vb, k, ctx))
        });
    }
    false
}

fn identity(value: &Value) -> Option<String> {
    let m = value.as_object()?;
    if let Some(Value::String(map_asset_name)) = m.get("MapAssetName") {
        let pkg = match map_asset_name.rfind('.') {
            Some(dot) if dot > 0 => &map_asset_name[..dot],
            _ => map_asset_name.as_str(),
        };
        return Some(format!("map:{}", pkg.to_lowercase()));
    }
    if let Some(Value::Object(ba)) = m.get("BehemothAsset") {
        let path = ba
            .get("AssetPathName")
            .map(|v| service_kit::js::string(Some(v)))
            .unwrap_or_else(|| "undefined".into());
        return Some(format!("behemoth:{}", path.to_lowercase()));
    }
    if let Some(Value::Object(mt)) = m.get("MatchmakerTable")
        && let Some(Value::String(obj_name)) = mt.get("ObjectName")
    {
        return Some(format!("table:{obj_name}"));
    }
    None
}

fn ordered_insert(keys: &mut Vec<String>, new_key: &str, reference_order: &[&str]) {
    if let Some(ref_index) = reference_order.iter().position(|k| *k == new_key) {
        let positions: HashMap<&str, usize> = keys.iter().enumerate().map(|(i, k)| (k.as_str(), i)).collect();
        for i in (0..ref_index).rev() {
            if let Some(&at) = positions.get(reference_order[i]) {
                keys.insert(at + 1, new_key.to_owned());
                return;
            }
        }
        for item in reference_order.iter().skip(ref_index + 1) {
            if let Some(&at) = positions.get(*item) {
                keys.insert(at, new_key.to_owned());
                return;
            }
        }
    }
    if let Some(marker) = keys.iter().position(|k| PROVENANCE_FIELDS.contains(&k.as_str())) {
        keys.insert(marker, new_key.to_owned());
    } else {
        keys.push(new_key.to_owned());
    }
}

fn ordered_insert_owned(keys: &mut Vec<String>, new_key: &str, reference_order: &[String]) {
    let refs: Vec<&str> = reference_order.iter().map(String::as_str).collect();
    ordered_insert(keys, new_key, &refs);
}

fn rebuild(keys: &[String], values: &mut Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for key in keys {
        if let Some(v) = values.remove(key) {
            out.insert(key.clone(), v);
        }
    }
    out
}

fn reference_order_for_object<'a>(field: &str, b: &'a Map<String, Value>) -> Vec<&'a str> {
    match field {
        "MapList" => MAP_ENTRY_ORDER.to_vec(),
        "SpecificBehemoth" | "AdditionalSpecificBehemoths" => BEHEMOTH_ORDER.to_vec(),
        _ => b.keys().map(String::as_str).collect(),
    }
}

fn merge_value(a: &Value, b: &Value, field: &str, ctx: &MergeCtx<'_>) -> Value {
    if let Some(spec) = enum_of(field) {
        if spec.always_name || !a.is_string() {
            return b.clone();
        }
        let name = enum_name(field, Some(b), ctx.export_version).unwrap_or("undefined");
        return Value::String(format!("{}::{name}", spec.prefix));
    }
    if is_handle(a) && is_handle(b) {
        let ka = handle_key(a);
        let kb = handle_key(b);
        let dt_a = a
            .as_object()
            .and_then(|m| m.get("DataTable"))
            .and_then(Value::as_object)
            .and_then(|dt| dt.get("ObjectName"));
        let dt_b = b
            .as_object()
            .and_then(|m| m.get("DataTable"))
            .and_then(Value::as_object)
            .and_then(|dt| dt.get("ObjectName"));
        if ka != "None" && kb != "None" && dt_a.is_some() && dt_a == dt_b {
            let mut merged = a.as_object().cloned().unwrap_or_default();
            if let Some(rn) = b.as_object().and_then(|m| m.get("RowName")).cloned() {
                merged.insert("RowName".into(), rn);
            }
            return Value::Object(merged);
        }
        return b.clone();
    }
    if let (Value::Array(aa), Value::Array(ab)) = (a, b) {
        let mut pool = aa.clone();
        let mut out = Vec::with_capacity(ab.len());
        for item in ab {
            if let Some(at) = pool.iter().position(|existing| same(existing, item, field, ctx)) {
                out.push(pool.remove(at));
                continue;
            }
            if let Some(id) = identity(item)
                && let Some(at) = pool
                    .iter()
                    .position(|existing| identity(existing).as_deref() == Some(id.as_str()))
            {
                let existing = pool.remove(at);
                out.push(merge_value(&existing, item, field, ctx));
                continue;
            }
            out.push(item.clone());
        }
        return Value::Array(out);
    }
    if let (Value::Object(ma), Value::Object(mb)) = (a, b)
        && !is_handle(a)
        && !is_handle(b)
        && !is_object_ref(b)
    {
        let mut keys: Vec<String> = ma.keys().cloned().collect();
        let mut values = ma.clone();
        let order = reference_order_for_object(field, mb);
        for (key, value) in mb {
            if UNEXPORTED_FIELDS.contains(&key.as_str()) {
                continue;
            }
            let Some(existing_val) = ma.get(key) else {
                values.insert(key.clone(), value.clone());
                ordered_insert(&mut keys, key, &order);
                continue;
            };
            if !same(existing_val, value, key, ctx) {
                values.insert(key.clone(), merge_value(existing_val, value, key, ctx));
            }
        }
        return Value::Object(rebuild(&keys, &mut values));
    }
    b.clone()
}

fn merge_row(
    existing: &Map<String, Value>,
    incoming: &Map<String, Value>,
    reference_order: &[&str],
    export_version: &str,
    provenance_marker: &str,
) -> (Map<String, Value>, Vec<String>) {
    let ctx = MergeCtx {
        existing_version: row_version(existing),
        export_version,
    };
    let mut keys: Vec<String> = existing.keys().cloned().collect();
    let mut values = existing.clone();
    let mut changes = Vec::new();

    for (key, value) in incoming {
        if UNEXPORTED_FIELDS.contains(&key.as_str()) || PROVENANCE_FIELDS.contains(&key.as_str()) {
            continue;
        }
        let Some(existing_val) = existing.get(key) else {
            values.insert(key.clone(), value.clone());
            ordered_insert(&mut keys, key, reference_order);
            changes.push(format!("+{key}"));
            continue;
        };
        if same(existing_val, value, key, &ctx) {
            if enum_of(key).is_some_and(|e| e.always_name) && existing_val != value {
                values.insert(key.clone(), value.clone());
                changes.push(format!("{key}(number->name)"));
            }
            continue;
        }
        values.insert(key.clone(), merge_value(existing_val, value, key, &ctx));
        changes.push(key.clone());
    }

    if !changes.is_empty() {
        if existing.contains_key("_importedFrom") {
            values.insert("_importedFrom".into(), Value::String(provenance_marker.to_owned()));
        } else {
            if !existing.contains_key("_updatedFrom") {
                keys.push("_updatedFrom".into());
            }
            values.insert("_updatedFrom".into(), Value::String(provenance_marker.to_owned()));
        }
    }

    (rebuild(&keys, &mut values), changes)
}

fn validate_matchmaker_row(row_name: &str, row: &Map<String, Value>) -> Option<String> {
    let maps = row
        .get("mapList")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if maps.is_empty() {
        return Some(format!("{row_name}: no mapList"));
    }
    let usable = maps
        .iter()
        .filter_map(|m| {
            let obj = m.as_object()?;
            obj.get("mapAssetPath")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .or_else(|| obj.get("mapAssetName").and_then(Value::as_str))
        })
        .any(|v| v.starts_with("/Game/"));
    if !usable {
        let first = maps.first().and_then(Value::as_object);
        let sample = first
            .and_then(|m| m.get("mapAssetPath"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                first
                    .and_then(|m| m.get("mapAssetName"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or("(empty)");
        return Some(format!("{row_name}: no /Game/ map path (got \"{sample}\")"));
    }
    None
}

fn summarise(names: &[String], limit: usize) -> String {
    if names.is_empty() {
        return String::new();
    }
    let shown = names
        .iter()
        .take(limit)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    if names.len() > limit {
        format!(" -> {shown}, ...(+{})", names.len() - limit)
    } else {
        format!(" -> {shown}")
    }
}

struct MergedTable {
    summary: TableMergeSummary,
    table: Value,
}

#[allow(clippy::too_many_arguments)]
fn merge_table(
    table_name: &str,
    vendor_dir: &Path,
    exported_rows: &[Value],
    is_player: bool,
    reference_order: &[&str],
    export_version: &str,
    provenance_marker: &str,
    update_existing: bool,
    verbose: bool,
    known: &HashMap<String, String>,
    problems: &mut Vec<String>,
    out_lines: &mut Vec<String>,
) -> Result<MergedTable, String> {
    let table_path = vendor_dir.join(format!("{table_name}.json"));
    let is_new_file = !table_path.exists();
    let mut table = if is_new_file {
        json!([{ "Name": table_name, "Type": "DataTable", "Rows": {} }])
    } else {
        read_json(&table_path)?
    };
    let existing_rows = table
        .get(0)
        .and_then(|t| t.get("Rows"))
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(|| format!("{} has no [0].Rows object", table_path.display()))?;
    let before_count = existing_rows.len();

    let mut added = Vec::new();
    let mut updated: Vec<(String, Vec<String>)> = Vec::new();
    let mut rejected = Vec::new();
    let mut unchanged = 0usize;

    let mut row_keys: Vec<String> = existing_rows.keys().cloned().collect();
    let mut row_values = existing_rows.clone();
    let export_order: Vec<String> = exported_rows
        .iter()
        .map(|r| r.get("rowName").and_then(Value::as_str).unwrap_or_default().to_owned())
        .collect();
    let exported_set: HashSet<String> = export_order.iter().cloned().collect();

    for row_val in exported_rows {
        let Some(row) = row_val.as_object() else {
            continue;
        };
        let row_name = row
            .get("rowName")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if !is_player && let Some(problem) = validate_matchmaker_row(&row_name, row) {
            rejected.push(problem);
            continue;
        }
        let incoming = if is_player {
            to_vendor_player_row(row, export_version, provenance_marker, known, problems)
        } else {
            to_vendor_matchmaker_row(row, export_version, provenance_marker, known, problems)
        };

        let Some(existing_row_val) = existing_rows.get(&row_name) else {
            row_values.insert(row_name.clone(), Value::Object(incoming));
            ordered_insert_owned(&mut row_keys, &row_name, &export_order);
            added.push(row_name);
            continue;
        };
        let Some(existing_row) = existing_row_val.as_object() else {
            continue;
        };
        let (merged_row, changes) = merge_row(
            existing_row,
            &incoming,
            reference_order,
            export_version,
            provenance_marker,
        );
        if changes.is_empty() {
            unchanged += 1;
            continue;
        }
        if update_existing {
            row_values.insert(row_name.clone(), Value::Object(merged_row));
        }
        updated.push((row_name, changes));
    }

    let vendor_only: Vec<String> = row_keys
        .iter()
        .filter(|rn| !exported_set.contains(*rn))
        .cloned()
        .collect();

    let mut normalised = Vec::new();
    for row_name in &vendor_only {
        let Some(Value::Object(row_obj)) = row_values.get(row_name).cloned() else {
            continue;
        };
        let Some(mgt) = row_obj.get("MatchmakingGameType") else {
            continue;
        };
        if !mgt.is_number() {
            continue;
        }
        let ver = row_version(&row_obj);
        let Some(name) = enum_name("MatchmakingGameType", Some(mgt), &ver) else {
            problems.push(format!(
                "{row_name}: vendored MatchmakingGameType={} does not decode for {ver}",
                jsjson::stringify(mgt)
            ));
            continue;
        };
        normalised.push(row_name.clone());
        if update_existing {
            let mut updated_obj = row_obj;
            updated_obj.insert(
                "MatchmakingGameType".into(),
                Value::String(format!("EMatchmakingGameType::{name}")),
            );
            row_values.insert(row_name.clone(), Value::Object(updated_obj));
        }
    }

    let final_rows = rebuild(&row_keys, &mut row_values);
    if let Some(first) = table.get_mut(0).and_then(Value::as_object_mut) {
        first.insert("Rows".into(), Value::Object(final_rows));
    }
    let will_change = !added.is_empty() || (update_existing && (!updated.is_empty() || !normalised.is_empty()));

    let mut field_counts: Vec<(String, usize)> = Vec::new();
    for (_, changes) in &updated {
        for change in changes {
            if let Some(entry) = field_counts.iter_mut().find(|(f, _)| f == change) {
                entry.1 += 1;
            } else {
                field_counts.push((change.clone(), 1));
            }
        }
    }
    field_counts.sort_by_key(|a| std::cmp::Reverse(a.1));
    let field_summary = field_counts
        .iter()
        .map(|(f, c)| format!("{f} x{c}"))
        .collect::<Vec<_>>()
        .join(", ");

    out_lines.push(format!(
        "\n=== {table_name}{} ===",
        if is_new_file { "  (NEW vendor file)" } else { "" }
    ));
    out_lines.push(format!("  vendored rows   : {before_count}"));
    out_lines.push(format!("  export rows     : {}", exported_rows.len()));
    out_lines.push(format!("  added           : {}{}", added.len(), summarise(&added, 12)));
    let updated_names: Vec<String> = updated.iter().map(|(n, _)| n.clone()).collect();
    out_lines.push(format!(
        "  changed         : {}{}{}",
        updated.len(),
        if update_existing {
            ""
        } else {
            " (not written: pass --update-existing)"
        },
        summarise(&updated_names, 8)
    ));
    if !field_summary.is_empty() {
        out_lines.push(format!("    fields        : {field_summary}"));
    }
    if verbose {
        for (row_name, changes) in &updated {
            out_lines.push(format!("      {row_name}: {}", changes.join(", ")));
        }
    }
    out_lines.push(format!("  unchanged       : {unchanged}"));
    out_lines.push(format!(
        "  vendor-only     : {} (kept; absent from this export){}",
        vendor_only.len(),
        summarise(&vendor_only, 12)
    ));
    if !normalised.is_empty() {
        out_lines.push(format!(
            "    game type     : {} vendor-only row(s) with a numeric MatchmakingGameType -> name{}{}",
            normalised.len(),
            if update_existing {
                ""
            } else {
                " (not written: pass --update-existing)"
            },
            summarise(&normalised, 12)
        ));
    }
    if !rejected.is_empty() {
        out_lines.push(format!("  REJECTED        : {}", rejected.len()));
        for problem in rejected.iter().take(10) {
            out_lines.push(format!("      {problem}"));
        }
        if rejected.len() > 10 {
            out_lines.push(format!("      ...and {} more", rejected.len() - 10));
        }
    }
    out_lines.push(format!("  rows after      : {}", row_keys.len()));

    Ok(MergedTable {
        summary: TableMergeSummary {
            table_name: table_name.to_owned(),
            table_path,
            is_new_file,
            added,
            updated,
            normalised,
            rejected,
            vendor_only,
            will_change,
        },
        table,
    })
}

pub fn import_hunt_tables(opts: &ImportHuntTablesOptions) -> Result<ImportHuntTablesReport, String> {
    if !opts.export_dir.exists() {
        return Err(format!(
            "No export found at {}\nRun CatalogExporter with EXPORT_HUNTS=1 and inject into the CLIENT first.",
            opts.export_dir.display()
        ));
    }

    let manifest_path = opts.export_dir.join("hunt_export_manifest.json");
    let manifest = if manifest_path.exists() {
        Some(read_json(&manifest_path)?)
    } else {
        None
    };
    let export_version = manifest
        .as_ref()
        .and_then(|m| m.get("gameVersion"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    if !SUPPORTED_VERSIONS.contains(&export_version.as_str()) {
        return Err(format!(
            "Export {} declares gameVersion '{}'.\nEnum numbers can only be decoded for: {}. Add the version's EMatchmakingGameType table first.",
            opts.export_dir.display(),
            if export_version.is_empty() {
                "(missing)"
            } else {
                &export_version
            },
            SUPPORTED_VERSIONS.join(", ")
        ));
    }
    let provenance_marker = format!("CatalogExporter EXPORT_HUNTS {export_version}");

    let player_rows = read_jsonl(&opts.export_dir, "player_hunts.jsonl")?;
    let matchmaker_rows = read_jsonl(&opts.export_dir, "matchmaker_hunts.jsonl")?;
    let (Some(player_rows), Some(matchmaker_rows)) = (player_rows, matchmaker_rows) else {
        return Err("Export is missing player_hunts.jsonl or matchmaker_hunts.jsonl.".into());
    };

    let mut out_lines = Vec::new();
    let rel_export = opts
        .repo_root
        .as_deref()
        .and_then(|root| opts.export_dir.strip_prefix(root).ok())
        .unwrap_or(&opts.export_dir);
    let changelist = manifest
        .as_ref()
        .and_then(|m| m.get("changelist"))
        .filter(|v| !v.is_null())
        .map(|v| service_kit::js::string(Some(v)))
        .unwrap_or_else(|| "?".into());
    out_lines.push(format!(
        "Export: {} ({export_version}, CL {changelist}): {} player rows, {} matchmaker rows",
        rel_export.display(),
        player_rows.len(),
        matchmaker_rows.len()
    ));
    out_lines.push(format!(
        "Mode  : {}{}",
        if opts.apply {
            "APPLY"
        } else {
            "DRY RUN (pass --apply to write)"
        },
        if opts.update_existing { " +update-existing" } else { "" }
    ));

    let mut known_object_paths = HashMap::new();
    for table_name in [MAIN_PLAYER_TABLE, MAIN_MATCHMAKER_TABLE] {
        let vendor_path = opts.vendor_dir.join(format!("{table_name}.json"));
        if !vendor_path.exists() {
            continue;
        }
        let doc = read_json(&vendor_path)?;
        if let Some(rows) = doc.get(0).and_then(|t| t.get("Rows")).and_then(Value::as_object) {
            for row in rows.values() {
                if row
                    .as_object()
                    .and_then(|m| m.get("_importedFrom"))
                    .is_none_or(Value::is_null)
                {
                    index_object_paths(row, &mut known_object_paths);
                }
            }
        }
    }

    let mut matchmaker_by_table: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for row in &matchmaker_rows {
        let table = row
            .get("sourceTable")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(MAIN_MATCHMAKER_TABLE)
            .to_owned();
        matchmaker_by_table.entry(table).or_default().push(row.clone());
    }
    let mut matchmaker_tables = vec![MAIN_MATCHMAKER_TABLE.to_owned()];
    for name in matchmaker_by_table.keys() {
        if name != MAIN_MATCHMAKER_TABLE {
            matchmaker_tables.push(name.clone());
        }
    }

    let mut conversion_problems = Vec::new();
    let mut results = Vec::new();
    results.push(merge_table(
        MAIN_PLAYER_TABLE,
        &opts.vendor_dir,
        &player_rows,
        true,
        PLAYER_ROW_ORDER,
        &export_version,
        &provenance_marker,
        opts.update_existing,
        opts.verbose,
        &known_object_paths,
        &mut conversion_problems,
        &mut out_lines,
    )?);
    for table_name in &matchmaker_tables {
        let rows = matchmaker_by_table.get(table_name).map(Vec::as_slice).unwrap_or(&[]);
        results.push(merge_table(
            table_name,
            &opts.vendor_dir,
            rows,
            false,
            MATCHMAKER_ROW_ORDER,
            &export_version,
            &provenance_marker,
            opts.update_existing,
            opts.verbose,
            &known_object_paths,
            &mut conversion_problems,
            &mut out_lines,
        )?);
    }

    // Game-type census in first-seen order (as JS Map preserves insertion order).
    let mut game_types: Vec<(String, usize)> = Vec::new();
    for row in &player_rows {
        let mgt = row.get("matchmakingGameType");
        let name = enum_name("MatchmakingGameType", mgt, &export_version)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("#{}", service_kit::js::string(mgt)));
        if let Some(entry) = game_types.iter_mut().find(|(n, _)| *n == name) {
            entry.1 += 1;
        } else {
            game_types.push((name, 1));
        }
    }
    out_lines.push(format!("\n=== MatchmakingGameType ({export_version} enum) ==="));
    out_lines.push(format!(
        "  {}",
        game_types
            .iter()
            .map(|(n, c)| format!("{n} x{c}"))
            .collect::<Vec<_>>()
            .join(", ")
    ));

    // Referential integrity check.
    let final_mm_rows: HashMap<String, &Map<String, Value>> = results
        .iter()
        .skip(1)
        .filter_map(|r| {
            let rows = r.table.get(0)?.get("Rows")?.as_object()?;
            Some((r.summary.table_name.clone(), rows))
        })
        .collect();
    let find_row = |row_name: &str| final_mm_rows.values().any(|rows| rows.contains_key(row_name));
    let mut dangling = Vec::new();
    let mut unknown_tag_tables = Vec::new();
    if let Some(player_table_rows) = results[0]
        .table
        .get(0)
        .and_then(|t| t.get("Rows"))
        .and_then(Value::as_object)
    {
        for (row_name, row) in player_table_rows {
            if let Some(entries) = row.get("MatchmakerHuntIDs").and_then(Value::as_array) {
                for entry in entries {
                    let target = entry.get("RowName").and_then(Value::as_str).unwrap_or("");
                    if target.is_empty() || target == "None" {
                        continue;
                    }
                    if !find_row(target) {
                        dangling.push(format!("{row_name} -> {target}"));
                    }
                }
            }
            if let Some(lists) = row.get("MatchmakerHuntsByTag").and_then(Value::as_array) {
                for list in lists {
                    let obj_name = list
                        .get("MatchmakerTable")
                        .and_then(Value::as_object)
                        .and_then(|mt| mt.get("ObjectName"));
                    if let Some(table) = table_from_object_name(obj_name)
                        && !final_mm_rows.contains_key(&table)
                        && !opts.vendor_dir.join(format!("{table}.json")).exists()
                    {
                        let item = format!("{row_name} -> {table}");
                        if !unknown_tag_tables.contains(&item) {
                            unknown_tag_tables.push(item);
                        }
                    }
                }
            }
        }
    }

    out_lines.push("\n=== referential check ===".into());
    if dangling.is_empty() {
        out_lines.push("  all direct MatchmakerHuntIDs resolve".into());
    } else {
        out_lines.push(format!("  {} dangling direct reference(s):", dangling.len()));
        for d in dangling.iter().take(15) {
            out_lines.push(format!("      {d}"));
        }
        if dangling.len() > 15 {
            out_lines.push(format!("      ...and {} more", dangling.len() - 15));
        }
    }
    if unknown_tag_tables.is_empty() {
        out_lines.push("  every tag-routed MatchmakerTable has a vendored file".into());
    } else {
        for entry in &unknown_tag_tables {
            out_lines.push(format!("  tag-routed table without a vendored file: {entry}"));
        }
    }

    let rejected: Vec<String> = results
        .iter()
        .flat_map(|r| r.summary.rejected.iter().cloned())
        .collect();
    if !conversion_problems.is_empty() {
        out_lines.push(format!("\n=== enum problems ({}) ===", conversion_problems.len()));
        for problem in conversion_problems.iter().take(15) {
            out_lines.push(format!("  {problem}"));
        }
    }

    if !opts.apply {
        out_lines.push(format!(
            "\nDry run complete. Nothing written. Re-run with --apply{} to commit.",
            if opts.update_existing { " --update-existing" } else { "" }
        ));
        return Ok(ImportHuntTablesReport {
            export_version,
            tables: results.into_iter().map(|r| r.summary).collect(),
            conversion_problems,
            output: out_lines.join("\n"),
        });
    }

    if !rejected.is_empty() {
        return Err(format!(
            "REFUSING TO WRITE: {} matchmaker row(s) have no /Game/ map path.\nGetMapPathFromMatchmakerHuntId requires it, so these rows would resolve and then fail at launch.\nRe-export with the current HuntExporter (it reads the soft-pointer path via FName::GetRawString).",
            rejected.len()
        ));
    }
    if !conversion_problems.is_empty() {
        return Err(format!(
            "REFUSING TO WRITE: {} enum value(s) do not decode for {export_version}.",
            conversion_problems.len()
        ));
    }

    out_lines.push(String::new());
    std::fs::create_dir_all(&opts.vendor_dir)
        .map_err(|e| format!("failed to create {}: {e}", opts.vendor_dir.display()))?;
    let mut new_files = Vec::new();
    let stamp = opts.backup_stamp.clone().unwrap_or_else(|| {
        Utc::now()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
            .replace([':', '.'], "-")
    });

    for result in &results {
        if !result.summary.will_change {
            continue;
        }
        if !result.summary.is_new_file {
            let bak_name = format!(
                "{}.{stamp}.bak",
                result
                    .summary
                    .table_path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("table.json")
            );
            let bak_path = result.summary.table_path.with_file_name(&bak_name);
            std::fs::copy(&result.summary.table_path, &bak_path).map_err(|e| {
                format!(
                    "failed to back up {} to {}: {e}",
                    result.summary.table_path.display(),
                    bak_path.display()
                )
            })?;
            out_lines.push(format!("Backed up: {bak_name}"));
        }
        let text = isles_core::game_data::generators::stringify_pretty(&result.table, 2);
        std::fs::write(&result.summary.table_path, text.as_bytes())
            .map_err(|e| format!("failed to write {}: {e}", result.summary.table_path.display()))?;
        let row_count = result
            .table
            .get(0)
            .and_then(|t| t.get("Rows"))
            .and_then(Value::as_object)
            .map_or(0, Map::len);
        let file_name = result
            .summary
            .table_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&result.summary.table_name);
        out_lines.push(format!(
            "Wrote {file_name} ({row_count} rows: +{}{})",
            result.summary.added.len(),
            if opts.update_existing {
                format!(
                    ", ~{}, game-type names {}",
                    result.summary.updated.len(),
                    result.summary.normalised.len()
                )
            } else {
                String::new()
            }
        ));
        if result.summary.is_new_file {
            new_files.push(result.summary.table_name.clone());
        }
    }

    if !new_files.is_empty() {
        out_lines.push(format!("\nNEW vendor table(s): {}", new_files.join(", ")));
        out_lines
            .push("  DeployServer and the Metagame only read them once they are listed in HUNT_TABLE_FILES".into());
        out_lines.push("  (shared/hunt-manifest/index.js).".into());
    }

    out_lines.push("\nDone. Rebuild and restart DeployServer, then check the boot log:".into());
    out_lines.push("  [HuntTableAudit]    problems=0".into());
    out_lines.push("  [HuntLaunchCoverage] resolvable=N/N".into());

    Ok(ImportHuntTablesReport {
        export_version,
        tables: results.into_iter().map(|r| r.summary).collect(),
        conversion_problems,
        output: out_lines.join("\n"),
    })
}
