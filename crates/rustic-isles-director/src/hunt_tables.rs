// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The hunt-table importer (spec §10, `dainty-host gamedata`, RB11): merges a CatalogExporter `EXPORT_HUNTS=1`
//! capture of the host's own install into the Director's tables. Its terminal output is English (AGENTS.md).

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use dainty_service_kit::js;
use serde_json::{Map, Value};

use crate::hunt::js_entries;

/// The export folder's name when none is given.
pub const DEFAULT_EXPORT: &str = "hunts_1_14_7";
/// Game versions an export may come from.
pub const SUPPORTED_VERSIONS: [&str; 2] = ["1.12.0", "1.14.7"];

const PLAYER_TABLE: &str = "player_hunts_table";
const MAIN_TABLE: &str = "matchmaker_hunts_table";
const GAME_TYPES_1_12: [&str; 10] = [
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
const GAME_TYPES_1_14: [&str; 11] = [
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
const UNEXPORTED: [&str; 3] = ["HuntName", "HuntDescription", "AdditionalBehemoths"];
const PROVENANCE: [&str; 2] = ["_importedFrom", "_updatedFrom"];
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
const PLAYER_ORDER: [&str; 23] = [
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
];
const MATCHMAKER_ORDER: [&str; 17] = [
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
    "_updatedFrom",
];
const MAP_ORDER: [&str; 4] = ["MapName", "MapAssetName", "Biome", "Weighting"];
const BEHEMOTH_ORDER: [&str; 4] = ["BehemothName", "BehemothAsset", "PowerOverride", "Weighting"];

/// What to import and how.
#[derive(Debug, Clone)]
pub struct ImportHuntTablesOptions {
    /// The CatalogExporter capture's folder.
    pub export_dir: PathBuf,
    /// The Director's vendored tables, written in place.
    pub vendor_dir: PathBuf,
    /// Only to show the export folder relative to it
    pub repo_root: Option<PathBuf>,
    /// Else a dry run
    pub apply: bool,
    /// Rewrite rows the vendor already has; otherwise only new rows are added.
    pub update_existing: bool,
    /// List each updated row's changes.
    pub verbose: bool,
    /// The suffix of the `.bak` copies; the current time when none.
    pub backup_stamp: Option<String>,
}

/// What the merge did to one table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableMergeSummary {
    /// The table's name.
    pub name: String,
    /// The vendored file.
    pub path: PathBuf,
    /// Whether the vendor had no such file.
    pub new_file: bool,
    /// Rows the export adds.
    pub added: Vec<String>,
    /// Each updated row with its changes
    pub updated: Vec<(String, Vec<String>)>,
    /// Vendor-only rows whose numeric `MatchmakingGameType` gets its name.
    pub normalised: Vec<String>,
    /// Rows the export holds but that could not be converted, with the reason.
    pub rejected: Vec<String>,
    /// Vendored rows the export does not hold.
    pub vendor_only: Vec<String>,
    /// Each change with its count, most first
    pub changes: Vec<(String, usize)>,
    /// Rows the export left as they were.
    pub unchanged: usize,
    /// Rows in the vendored file before the merge.
    pub vendored_rows: usize,
    /// Rows in the export.
    pub export_rows: usize,
    /// Rows in the vendored file after the merge.
    pub rows_after: usize,
}

/// What an import did, or would do on a dry run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportHuntTablesReport {
    /// The game version the export's manifest names.
    pub export_version: String,
    /// Each table merged.
    pub tables: Vec<TableMergeSummary>,
    /// Export rows or values that could not be converted.
    pub conversion_problems: Vec<String>,
    /// The terminal text the host is shown.
    pub output: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Enum {
    GameType,
    MatchmakingType,
    Biome,
    Tier,
}

impl Enum {
    fn of(field: &str) -> Option<Self> {
        match field {
            "MatchmakingGameType" => Some(Self::GameType),
            "MatchmakingType" => Some(Self::MatchmakingType),
            "Biome" => Some(Self::Biome),
            "GatherableTier" => Some(Self::Tier),
            _ => None,
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Self::GameType => "EMatchmakingGameType",
            Self::MatchmakingType => "EMatchmakingType",
            Self::Biome => "EHuntBiome",
            Self::Tier => "EGatherableTier",
        }
    }

    fn values(self, version: &str) -> &'static [&'static str] {
        match self {
            Self::GameType if version == "1.12.0" => &GAME_TYPES_1_12,
            Self::GameType => &GAME_TYPES_1_14,
            Self::MatchmakingType => &["Reserved", "Open", "City"],
            Self::Biome => &["Moss", "Ice", "Arid"],
            Self::Tier => &["Tier0", "Tier1", "Tier2", "Tier3", "Tier4"],
        }
    }

    fn decode(self, value: Option<&Value>, version: &str) -> Option<&'static str> {
        match value? {
            Value::String(text) => {
                let name = text.rsplit("::").next().unwrap_or(text);
                let lists: &[&[&str]] = if self == Self::GameType {
                    &[&GAME_TYPES_1_12, &GAME_TYPES_1_14]
                } else {
                    &[self.values(version)]
                };
                lists.iter().flat_map(|l| l.iter()).find(|v| **v == name).copied()
            }
            Value::Number(n) => {
                let f = n.as_f64()?;
                (f >= 0.0 && f.fract() == 0.0).then(|| self.values(version).get(f as usize).copied())?
            }
            _ => None,
        }
    }
}

fn json_text(value: Option<&Value>) -> String {
    value.map_or_else(|| "undefined".to_owned(), dainty_service_kit::jsjson::stringify)
}

fn present(value: Option<&Value>) -> Option<&Value> {
    value.filter(|v| !v.is_null())
}

fn or(value: Option<&Value>, default: Value) -> Value {
    present(value).cloned().unwrap_or(default)
}

/// Converts exported rows, collecting enum problems.
struct Converter<'a> {
    version: &'a str,
    marker: String,
    paths: HashMap<String, String>,
    problems: Vec<String>,
}

impl Converter<'_> {
    fn table_ref(&self, table: &str) -> Value {
        let path = self
            .paths
            .get(table)
            .cloned()
            .unwrap_or_else(|| format!("Archon/Content/Gameplay/hunts/{table}.0"));
        let mut out = Map::new();
        out.insert("ObjectName".into(), Value::String(format!("DataTable'{table}'")));
        out.insert("ObjectPath".into(), Value::String(path));
        Value::Object(out)
    }

    fn handle(&self, value: Option<&Value>) -> Value {
        let row = value
            .and_then(|v| v.get("rowName"))
            .and_then(Value::as_str)
            .filter(|r| !r.is_empty() && *r != "None");
        let mut out = Map::new();
        match (value.filter(|v| v.is_object()), row) {
            (Some(handle), Some(row)) => {
                let table = handle.get("table").and_then(Value::as_str).unwrap_or("undefined");
                out.insert("DataTable".into(), self.table_ref(table));
                out.insert("RowName".into(), Value::String(row.to_owned()));
            }
            _ => {
                out.insert("DataTable".into(), Value::Null);
                out.insert("RowName".into(), Value::from("None"));
            }
        }
        Value::Object(out)
    }

    fn handles(&self, value: Option<&Value>) -> Value {
        let items = value.and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        Value::Array(items.iter().map(|h| self.handle(Some(h))).collect())
    }

    /// Kept as given when it decodes (the game type by name), else kept as it was with a problem.
    fn enum_field(&mut self, out: &mut Map<String, Value>, row: &str, field: &str, value: Option<&Value>) {
        let Some(kind) = Enum::of(field) else {
            return;
        };
        match kind.decode(value, self.version) {
            Some(name) if kind == Enum::GameType => {
                out.insert(field.into(), Value::String(format!("{}::{name}", kind.prefix())));
            }
            Some(_) => {
                out.insert(field.into(), value.cloned().unwrap_or(Value::Null));
            }
            None => {
                self.problems.push(format!(
                    "{row}: {field}={} is not a {} value in {}",
                    json_text(value),
                    kind.prefix(),
                    self.version
                ));
                if let Some(value) = value {
                    out.insert(field.into(), value.clone());
                }
            }
        }
    }

    fn tag_list(&self, list: &Value) -> Value {
        let table = list
            .get("matchmakerTable")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .map_or(Value::Null, |t| self.table_ref(t));
        let queries = list
            .get("queries")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let queries = queries
            .iter()
            .map(|q| {
                let dictionary = present(q.get("tagDictionary"))
                    .and_then(Value::as_array)
                    .map(|tags| {
                        tags.iter()
                            .map(|t| {
                                let mut entry = Map::new();
                                entry.insert("TagName".into(), t.clone());
                                Value::Object(entry)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let mut out = Map::new();
                out.insert(
                    "TokenStreamVersion".into(),
                    or(q.get("tokenStreamVersion"), Value::from(0)),
                );
                out.insert("TagDictionary".into(), Value::Array(dictionary));
                out.insert(
                    "QueryTokenStream".into(),
                    or(q.get("tokenStream"), Value::Array(Vec::new())),
                );
                out.insert("UserDescription".into(), or(q.get("userDescription"), Value::from("")));
                out.insert("AutoDescription".into(), or(q.get("autoDescription"), Value::from("")));
                Value::Object(out)
            })
            .collect();
        let mut out = Map::new();
        out.insert("MatchmakerTable".into(), table);
        out.insert("HuntTags".into(), Value::Array(queries));
        Value::Object(out)
    }

    fn player_row(&mut self, source: &Value) -> Value {
        let row = source.get("rowName").and_then(Value::as_str).unwrap_or("").to_owned();
        let mut out = Map::new();
        let text = || {
            let mut t = Map::new();
            t.insert("CultureInvariantString".into(), Value::from(""));
            Value::Object(t)
        };
        out.insert("HuntName".into(), text());
        out.insert("HuntDescription".into(), text());
        out.insert("Region".into(), self.handle(source.get("region")));
        out.insert(
            "MatchmakerHuntIDs".into(),
            self.handles(source.get("matchmakerHuntIDs")),
        );
        let lists = source
            .get("matchmakerHuntsByTag")
            .and_then(Value::as_array)
            .map(Vec::as_slice);
        let lists = lists.unwrap_or_default().iter().map(|l| self.tag_list(l)).collect();
        out.insert("MatchmakerHuntsByTag".into(), Value::Array(lists));
        self.enum_field(&mut out, &row, "MatchmakingType", source.get("matchmakingType"));
        self.enum_field(&mut out, &row, "MatchmakingGameType", source.get("matchmakingGameType"));
        out.insert("HuntTags".into(), or(source.get("huntTags"), Value::Array(Vec::new())));
        out.insert(
            "EscalationModeSpecification".into(),
            self.handle(source.get("escalationModeSpecification")),
        );
        let optional = [
            (
                "EscalationPatrolInitialChallengeLevel",
                "escalationPatrolInitialChallengeLevel",
            ),
            ("EscalationPatrolMaxChallengeLevel", "escalationPatrolMaxChallengeLevel"),
            ("RecommendedEscalationLevel", "recommendedEscalationLevel"),
            ("bHasPortals", "hasPortals"),
            ("bHasGlitterEvent", "hasGlitterEvent"),
            ("bHasPhaelanxEvent", "hasPhaelanxEvent"),
            ("HuntSuccessReward", "huntSuccessReward"),
            ("FirstCompletionSuccessReward", "firstCompletionSuccessReward"),
            ("TargetedHuntSuccessReward", "targetedHuntSuccessReward"),
            ("HuntFailureReward", "huntFailureReward"),
            ("TargetedHuntFailureReward", "targetedHuntFailureReward"),
            ("MinWeaponSkillLevel", "minWeaponSkillLevel"),
            ("RecomendedWeaponSkillLevel", "recommendedWeaponSkillLevel"),
            ("IsVisibleWhileLocked", "visibleWhileLocked"),
        ];
        for (field, from) in optional {
            if let Some(value) = source.get(from) {
                out.insert(field.into(), value.clone());
            }
        }
        out.insert("_importedFrom".into(), Value::String(self.marker.clone()));
        Value::Object(out)
    }

    fn behemoth(&self, source: Option<&Value>) -> Value {
        let get = |k: &str| source.and_then(|s| s.get(k));
        let asset = get("behemothAssetPath")
            .and_then(Value::as_str)
            .filter(|p| !p.is_empty());
        let mut asset_out = Map::new();
        asset_out.insert("AssetPathName".into(), Value::from(asset.unwrap_or("None")));
        asset_out.insert("SubPathString".into(), Value::from(""));
        let mut out = Map::new();
        out.insert("BehemothName".into(), or(get("behemothName"), Value::from("")));
        out.insert("BehemothAsset".into(), Value::Object(asset_out));
        if let Some(v) = get("powerOverride") {
            out.insert("PowerOverride".into(), v.clone());
        }
        if let Some(v) = get("weighting") {
            out.insert("Weighting".into(), v.clone());
        }
        Value::Object(out)
    }

    fn map_entry(&mut self, row: &str, source: &Value) -> Value {
        let mut out = Map::new();
        out.insert("MapName".into(), or(source.get("mapName"), Value::from("")));
        out.insert(
            "MapAssetName".into(),
            Value::String(map_path(source).unwrap_or("").to_owned()),
        );
        self.enum_field(&mut out, row, "Biome", source.get("biome"));
        if let Some(v) = source.get("weighting") {
            out.insert("Weighting".into(), v.clone());
        }
        Value::Object(out)
    }

    /// The row, or the reason it is rejected.
    fn matchmaker_row(&mut self, source: &Value) -> Result<Value, String> {
        let row = source.get("rowName").and_then(Value::as_str).unwrap_or("").to_owned();
        let list = match source.get("mapList") {
            Some(Value::Array(list)) if !list.is_empty() => list,
            _ => return Err(format!("{row}: no mapList")),
        };
        if !list
            .iter()
            .any(|e| map_path(e).is_some_and(|p| p.starts_with("/Game/")))
        {
            let first = map_path(&list[0]).filter(|p| !p.is_empty()).unwrap_or("(empty)");
            return Err(format!("{row}: no /Game/ map path (got \"{first}\")"));
        }
        let mut out = Map::new();
        out.insert("Region".into(), self.handle(source.get("region")));
        out.insert("HuntTags".into(), or(source.get("huntTags"), Value::Array(Vec::new())));
        if let Some(v) = source.get("huntThreatLevel") {
            out.insert("HuntThreatLevel".into(), v.clone());
        }
        out.insert(
            "GameModeOverride".into(),
            or(source.get("gameModeOverride"), Value::from("")),
        );
        if let Some(v) = source.get("dangerPerSecOverride") {
            out.insert("DangerPerSecOverride".into(), v.clone());
        }
        out.insert("SpecificBehemoth".into(), self.behemoth(source.get("specificBehemoth")));
        out.insert("AdditionalBehemoths".into(), Value::Array(Vec::new()));
        let extra = source.get("additionalSpecificBehemoths").and_then(Value::as_array);
        let extra = extra.map(Vec::as_slice).unwrap_or_default();
        let extra = extra.iter().map(|b| self.behemoth(Some(b))).collect();
        out.insert("AdditionalSpecificBehemoths".into(), Value::Array(extra));
        out.insert("MapMetaData".into(), self.handle(source.get("mapMetaData")));
        let maps = list.iter().map(|e| self.map_entry(&row, e)).collect();
        out.insert("MapList".into(), Value::Array(maps));
        out.insert(
            "Modifiers".into(),
            or(source.get("modifiers"), Value::Array(Vec::new())),
        );
        if source.get("gatherableTier").is_some() {
            self.enum_field(&mut out, &row, "GatherableTier", source.get("gatherableTier"));
        }
        if let Some(v) = source.get("maxPlayers") {
            out.insert("MaxPlayers".into(), v.clone());
        }
        if let Some(v) = source.get("isGeneratedEncounter") {
            out.insert("bIsGeneratedEncounter".into(), v.clone());
        }
        out.insert(
            "GameModeSpecificData".into(),
            self.handle(source.get("gameModeSpecificData")),
        );
        out.insert("_importedFrom".into(), Value::String(self.marker.clone()));
        Ok(Value::Object(out))
    }
}

/// `mapAssetPath` when a non-empty string, else `mapAssetName` when a string.
fn map_path(entry: &Value) -> Option<&str> {
    entry
        .get("mapAssetPath")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
        .or_else(|| entry.get("mapAssetName").and_then(Value::as_str))
}

fn row_version(row: &Value) -> String {
    let marker = PROVENANCE
        .iter()
        .find_map(|k| row.get(*k).and_then(Value::as_str))
        .unwrap_or("");
    marker
        .split_once("EXPORT_HUNTS ")
        .map(|(_, rest)| {
            rest.chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect::<String>()
        })
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "1.12.0".to_owned())
}

fn is_handle(v: &Value) -> bool {
    v.get("RowName").is_some() && v.get("DataTable").is_some()
}

fn is_object_ref(v: &Value) -> bool {
    v.get("ObjectName").is_some() && v.get("ObjectPath").is_some()
}

fn handle_key(v: &Value) -> String {
    let row = v.get("RowName").and_then(Value::as_str).unwrap_or("");
    if row.is_empty() || row == "None" {
        return "None".to_owned();
    }
    let name = v
        .get("DataTable")
        .and_then(|t| t.get("ObjectName"))
        .and_then(Value::as_str);
    let table = name
        .and_then(|n| n.strip_prefix("DataTable'"))
        .and_then(|n| n.strip_suffix('\''))
        .unwrap_or("?");
    format!("{table}/{row}").to_lowercase()
}

/// Whether two values are the same for a merge (§10.4).
fn same(a: &Value, b: &Value, field: &str, versions: (&str, &str)) -> bool {
    if let Some(kind) = Enum::of(field) {
        let (x, y) = (kind.decode(Some(a), versions.0), kind.decode(Some(b), versions.1));
        return x.is_some() && x == y;
    }
    if a == b {
        return true;
    }
    match (a, b) {
        (Value::String(x), Value::String(y)) if FNAME_FIELDS.contains(&field) => x.to_lowercase() == y.to_lowercase(),
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) => (x - y).abs() <= 1e-6 * 1f64.max(x.abs()).max(y.abs()),
            _ => false,
        },
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(i, j)| same(i, j, field, versions))
        }
        (Value::Object(x), Value::Object(y)) => {
            if is_handle(a) || is_handle(b) {
                return is_handle(a) && is_handle(b) && handle_key(a) == handle_key(b);
            }
            if is_object_ref(a) || is_object_ref(b) {
                return a.get("ObjectName") == b.get("ObjectName");
            }
            y.iter()
                .filter(|(k, _)| !UNEXPORTED.contains(&k.as_str()))
                .all(|(k, v)| x.get(k).is_some_and(|w| same(w, v, k, versions)))
        }
        _ => false,
    }
}

fn identity(v: &Value) -> Option<String> {
    if let Some(name) = v.get("MapAssetName").and_then(Value::as_str) {
        let package = match name.rfind('.') {
            Some(at) if at > 0 => &name[..at],
            _ => name,
        };
        return Some(format!("map:{}", package.to_lowercase()));
    }
    if let Some(asset) = v.get("BehemothAsset").filter(|a| a.is_object()) {
        return Some(format!(
            "behemoth:{}",
            js::string(asset.get("AssetPathName")).to_lowercase()
        ));
    }
    let table = v.get("MatchmakerTable").filter(|t| t.is_object());
    if let Some(name) = table.and_then(|t| t.get("ObjectName")).and_then(Value::as_str) {
        return Some(format!("table:{name}"));
    }
    None
}

/// Inserts `key` placed by `order`: after the nearest earlier key the object has, else before the nearest later one,
/// else before `fallback_before` when present, else at the end.
fn place(object: &mut Map<String, Value>, key: &str, value: Value, order: &[&str], fallback_before: &[&str]) {
    let mut entries: Vec<(String, Value)> = std::mem::take(object).into_iter().collect();
    let at = order.iter().position(|k| *k == key);
    let index_of = |entries: &[(String, Value)], name: &str| entries.iter().position(|(k, _)| k == name);
    let mut position = None;
    if let Some(at) = at {
        position = order[..at]
            .iter()
            .rev()
            .find_map(|k| index_of(&entries, k))
            .map(|i| i + 1)
            .or_else(|| order[at + 1..].iter().find_map(|k| index_of(&entries, k)));
    }
    let position = position
        .or_else(|| fallback_before.iter().find_map(|k| index_of(&entries, k)))
        .unwrap_or(entries.len());
    entries.insert(position, (key.to_owned(), value));
    object.extend(entries);
}

fn merged(a: &Value, b: &Value, field: &str, versions: (&str, &str)) -> Value {
    if let Some(kind) = Enum::of(field) {
        if b.is_string() || !a.is_string() {
            return b.clone();
        }
        let name = kind.decode(Some(b), versions.1).unwrap_or("undefined");
        return Value::String(format!("{}::{name}", kind.prefix()));
    }
    if is_handle(a) && is_handle(b) {
        let table = |v: &Value| v.get("DataTable").and_then(|t| t.get("ObjectName")).cloned();
        let keyed = handle_key(a) != "None" && handle_key(b) != "None";
        if keyed && table(a).is_some() && table(a) == table(b) {
            let mut out = a.clone();
            if let (Some(out), Some(row)) = (out.as_object_mut(), b.get("RowName")) {
                out.insert("RowName".into(), row.clone());
            }
            return out;
        }
        return b.clone();
    }
    match (a, b) {
        (Value::Array(x), Value::Array(y)) => {
            let mut used = vec![false; x.len()];
            let items = y
                .iter()
                .map(|item| {
                    if let Some(i) = (0..x.len()).find(|&i| !used[i] && same(&x[i], item, field, versions)) {
                        used[i] = true;
                        return x[i].clone();
                    }
                    let id = identity(item);
                    if let Some(i) = (0..x.len()).find(|&i| !used[i] && id.is_some() && identity(&x[i]) == id) {
                        used[i] = true;
                        return merged(&x[i], item, field, versions);
                    }
                    item.clone()
                })
                .collect();
            Value::Array(items)
        }
        (Value::Object(x), Value::Object(y)) if !is_handle(a) && !is_handle(b) && !is_object_ref(b) => {
            let own: Vec<&str> = y.keys().map(String::as_str).collect();
            let order: &[&str] = match field {
                "MapList" => &MAP_ORDER,
                "SpecificBehemoth" | "AdditionalSpecificBehemoths" => &BEHEMOTH_ORDER,
                _ => &own,
            };
            let mut out = x.clone();
            for (k, v) in y {
                if UNEXPORTED.contains(&k.as_str()) {
                    continue;
                }
                match x.get(k) {
                    None => place(&mut out, k, v.clone(), order, &[]),
                    Some(w) if !same(w, v, k, versions) => {
                        out.insert(k.clone(), merged(w, v, k, versions));
                    }
                    Some(_) => {}
                }
            }
            Value::Object(out)
        }
        _ => b.clone(),
    }
}

/// Merges incoming `i` into existing `e`; the changes.
fn merge_row(
    e: &mut Map<String, Value>,
    i: &Map<String, Value>,
    order: &[&str],
    version: &str,
    marker: &str,
) -> Vec<String> {
    let existing = row_version(&Value::Object(e.clone()));
    let versions = (existing.as_str(), version);
    let mut changes = Vec::new();
    for (field, value) in i {
        if UNEXPORTED.contains(&field.as_str()) || PROVENANCE.contains(&field.as_str()) {
            continue;
        }
        match e.get(field) {
            None => {
                place(e, field, value.clone(), order, &PROVENANCE);
                changes.push(format!("+{field}"));
            }
            Some(old) if same(old, value, field, versions) => {
                if Enum::of(field) == Some(Enum::GameType) && old != value {
                    e.insert(field.clone(), value.clone());
                    changes.push(format!("{field}(number->name)"));
                }
            }
            Some(old) => {
                let new = merged(old, value, field, versions);
                e.insert(field.clone(), new);
                changes.push(field.clone());
            }
        }
    }
    if !changes.is_empty() {
        let key = if present(e.get("_importedFrom")).is_some() {
            "_importedFrom"
        } else {
            "_updatedFrom"
        };
        e.insert(key.to_owned(), Value::String(marker.to_owned()));
    }
    changes
}

fn read_json(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    dainty_service_kit::jsjson::parse(text).map_err(|e| format!("invalid JSON in {}: {e}", path.display()))
}

fn read_jsonl(path: &Path) -> Result<Vec<Value>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text).trim();
    let mut rows = Vec::new();
    for (n, line) in text.split('\n').enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let row = dainty_service_kit::jsjson::parse(line)
            .map_err(|e| format!("malformed JSON on line {} of {}: {e}", n + 1, path.display()))?;
        rows.push(row);
    }
    Ok(rows)
}

/// `JSON.stringify(value, null, 2)`.
pub fn stringify_pretty(value: &Value) -> String {
    fn write(out: &mut String, value: &Value, depth: usize) {
        let pad = |out: &mut String, depth: usize| out.extend(std::iter::repeat_n("  ", depth));
        match value {
            Value::Array(items) if !items.is_empty() => {
                out.push('[');
                for (n, item) in items.iter().enumerate() {
                    out.push_str(if n == 0 { "\n" } else { ",\n" });
                    pad(out, depth + 1);
                    write(out, item, depth + 1);
                }
                out.push('\n');
                pad(out, depth);
                out.push(']');
            }
            Value::Object(map) if !map.is_empty() => {
                out.push('{');
                for (n, (k, v)) in js_entries(map).into_iter().enumerate() {
                    out.push_str(if n == 0 { "\n" } else { ",\n" });
                    pad(out, depth + 1);
                    out.push_str(&dainty_service_kit::jsjson::stringify(&Value::String(k.clone())));
                    out.push_str(": ");
                    write(out, v, depth + 1);
                }
                out.push('\n');
                pad(out, depth);
                out.push('}');
            }
            other => out.push_str(&dainty_service_kit::jsjson::stringify(other)),
        }
    }
    let mut out = String::new();
    write(&mut out, value, 0);
    out
}

fn list(items: &[String], show: usize) -> String {
    if items.is_empty() {
        return String::new();
    }
    let mut out = format!(
        " -> {}",
        items.iter().take(show).cloned().collect::<Vec<_>>().join(", ")
    );
    if items.len() > show {
        let _ = write!(out, ", ...(+{})", items.len() - show);
    }
    out
}

/// `name xN` by count, most first, ties first seen.
fn counted(items: impl IntoIterator<Item = String>, sort: bool) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = Vec::new();
    for item in items {
        match out.iter_mut().find(|(k, _)| *k == item) {
            Some((_, n)) => *n += 1,
            None => out.push((item, 1)),
        }
    }
    if sort {
        out.sort_by_key(|e| std::cmp::Reverse(e.1));
    }
    out
}

fn counted_text(items: &[(String, usize)]) -> String {
    items
        .iter()
        .map(|(k, n)| format!("{k} x{n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The known object paths of the existing player and main matchmaker files: rows not imported, first seen wins.
fn known_paths(vendor: &Path) -> Result<HashMap<String, String>, String> {
    fn walk(value: &Value, out: &mut HashMap<String, String>) {
        match value {
            Value::Object(map) => {
                let name = map.get("ObjectName").and_then(Value::as_str);
                let table = name
                    .and_then(|n| n.strip_prefix("DataTable'"))
                    .and_then(|n| n.strip_suffix('\''));
                if let (Some(table), Some(path)) = (table, map.get("ObjectPath").and_then(Value::as_str)) {
                    out.entry(table.to_owned()).or_insert_with(|| path.to_owned());
                }
                for (_, v) in js_entries(map) {
                    walk(v, out);
                }
            }
            Value::Array(items) => items.iter().for_each(|i| walk(i, out)),
            _ => {}
        }
    }
    let mut out = HashMap::new();
    for table in [PLAYER_TABLE, MAIN_TABLE] {
        let path = vendor.join(format!("{table}.json"));
        if !path.is_file() {
            continue;
        }
        let file = read_json(&path)?;
        let Some(Value::Object(rows)) = file.get(0).and_then(|f| f.get("Rows")) else {
            continue;
        };
        for (_, row) in js_entries(rows) {
            if present(row.get("_importedFrom")).is_none() {
                walk(row, &mut out);
            }
        }
    }
    Ok(out)
}

/// An exported row's name and its converted row, or why it is rejected.
type Incoming = (String, Result<Value, String>);

struct Merged {
    summary: TableMergeSummary,
    file: Value,
    changes_table: bool,
}

fn merge_table(
    options: &ImportHuntTablesOptions,
    name: &str,
    incoming: &[Incoming],
    version: &str,
    marker: &str,
    problems: &mut Vec<String>,
) -> Result<Merged, String> {
    let path = options.vendor_dir.join(format!("{name}.json"));
    let new_file = !path.is_file();
    let mut file = if new_file {
        serde_json::json!([{"Name": name, "Type": "DataTable", "Rows": {}}])
    } else {
        read_json(&path)?
    };
    let Some(Value::Object(rows)) = file.get(0).and_then(|f| f.get("Rows")) else {
        return Err(format!("{} has no [0].Rows object", path.display()));
    };
    let mut rows: Map<String, Value> = rows.clone();
    let order: &[&str] = if name == PLAYER_TABLE {
        &PLAYER_ORDER
    } else {
        &MATCHMAKER_ORDER
    };
    let mut summary = TableMergeSummary {
        name: name.to_owned(),
        path: path.clone(),
        new_file,
        vendored_rows: rows.len(),
        export_rows: incoming.len(),
        ..TableMergeSummary::default()
    };
    let names: Vec<&str> = incoming.iter().map(|(n, _)| n.as_str()).collect();
    for (at, (row_name, converted)) in incoming.iter().enumerate() {
        let row = match converted {
            Ok(row) => row,
            Err(problem) => {
                summary.rejected.push(problem.clone());
                continue;
            }
        };
        match rows.get(row_name) {
            None => {
                let mut entries: Vec<(String, Value)> = std::mem::take(&mut rows).into_iter().collect();
                let index_of = |entries: &[(String, Value)], n: &str| entries.iter().position(|(k, _)| k == n);
                let position = names[..at]
                    .iter()
                    .rev()
                    .find_map(|n| index_of(&entries, n))
                    .map(|i| i + 1)
                    .or_else(|| names[at + 1..].iter().find_map(|n| index_of(&entries, n)))
                    .unwrap_or(entries.len());
                entries.insert(position, (row_name.clone(), row.clone()));
                rows.extend(entries);
                summary.added.push(row_name.clone());
            }
            Some(Value::Object(existing)) => {
                let mut updated = existing.clone();
                let Value::Object(incoming_row) = row else {
                    continue;
                };
                let changes = merge_row(&mut updated, incoming_row, order, version, marker);
                if changes.is_empty() {
                    summary.unchanged += 1;
                } else {
                    if options.update_existing {
                        rows.insert(row_name.clone(), Value::Object(updated));
                    }
                    summary.updated.push((row_name.clone(), changes));
                }
            }
            Some(_) => {}
        }
    }
    let exported: Vec<&String> = incoming.iter().map(|(n, _)| n).collect();
    let vendor_only: Vec<String> = rows
        .keys()
        .filter(|k| !exported.contains(k) && !summary.added.contains(k))
        .cloned()
        .collect();
    for row_name in &vendor_only {
        let Some(row) = rows.get(row_name) else {
            continue;
        };
        let Some(game_type) = row.get("MatchmakingGameType").filter(|v| v.is_number()) else {
            continue;
        };
        let own = row_version(row);
        match Enum::GameType.decode(Some(game_type), &own) {
            Some(decoded) => {
                summary.normalised.push(row_name.clone());
                if options.update_existing
                    && let Some(Value::Object(row)) = rows.get_mut(row_name)
                {
                    row.insert(
                        "MatchmakingGameType".into(),
                        Value::String(format!("EMatchmakingGameType::{decoded}")),
                    );
                }
            }
            None => problems.push(format!(
                "{row_name}: vendored MatchmakingGameType={} does not decode for {own}",
                dainty_service_kit::jsjson::stringify(game_type)
            )),
        }
    }
    summary.vendor_only = vendor_only;
    summary.changes = counted(summary.updated.iter().flat_map(|(_, c)| c.iter().cloned()), true);
    summary.rows_after = rows.len();
    let changes_table = !summary.added.is_empty()
        || (options.update_existing && (!summary.updated.is_empty() || !summary.normalised.is_empty()));
    if let Some(first) = file.get_mut(0).and_then(Value::as_object_mut) {
        first.insert("Rows".into(), Value::Object(rows));
    }
    Ok(Merged {
        summary,
        file,
        changes_table,
    })
}

fn summary_lines(out: &mut Vec<String>, s: &TableMergeSummary, options: &ImportHuntTablesOptions) {
    let not_written = if options.update_existing {
        ""
    } else {
        " (not written: pass --update-existing)"
    };
    let new = if s.new_file { "  (NEW vendor file)" } else { "" };
    out.push(format!("\n=== {}{new} ===", s.name));
    out.push(format!("  vendored rows   : {}", s.vendored_rows));
    out.push(format!("  export rows     : {}", s.export_rows));
    out.push(format!("  added           : {}{}", s.added.len(), list(&s.added, 12)));
    let updated: Vec<String> = s.updated.iter().map(|(n, _)| n.clone()).collect();
    let flag = if updated.is_empty() { "" } else { not_written };
    out.push(format!(
        "  changed         : {}{flag}{}",
        updated.len(),
        list(&updated, 8)
    ));
    if !s.changes.is_empty() {
        out.push(format!("    fields        : {}", counted_text(&s.changes)));
    }
    if options.verbose {
        for (row, changes) in &s.updated {
            out.push(format!("      {row}: {}", changes.join(", ")));
        }
    }
    out.push(format!("  unchanged       : {}", s.unchanged));
    out.push(format!(
        "  vendor-only     : {} (kept; absent from this export){}",
        s.vendor_only.len(),
        list(&s.vendor_only, 12)
    ));
    if !s.normalised.is_empty() {
        out.push(format!(
            "    game type     : {} vendor-only row(s) with a numeric MatchmakingGameType -> name{not_written}{}",
            s.normalised.len(),
            list(&s.normalised, 12)
        ));
    }
    if !s.rejected.is_empty() {
        out.push(format!("  REJECTED        : {}", s.rejected.len()));
        for problem in s.rejected.iter().take(10) {
            out.push(format!("      {problem}"));
        }
        if s.rejected.len() > 10 {
            out.push(format!("      ...and {} more", s.rejected.len() - 10));
        }
    }
    out.push(format!("  rows after      : {}", s.rows_after));
}

/// Merges the capture into the vendor folder; a refusal is the error with its text.
pub fn import_hunt_tables(options: &ImportHuntTablesOptions) -> Result<ImportHuntTablesReport, String> {
    let dir = &options.export_dir;
    if !dir.is_dir() {
        return Err(format!(
            "No export found at {}\nRun CatalogExporter with EXPORT_HUNTS=1 and inject into the CLIENT first.",
            dir.display()
        ));
    }
    let manifest_path = dir.join("hunt_export_manifest.json");
    let manifest = if manifest_path.is_file() {
        read_json(&manifest_path)?
    } else {
        Value::Object(Map::new())
    };
    let version = manifest.get("gameVersion").and_then(Value::as_str).unwrap_or("");
    if !SUPPORTED_VERSIONS.contains(&version) {
        let shown =
            present(manifest.get("gameVersion")).map_or_else(|| "(missing)".to_owned(), |v| js::string(Some(v)));
        return Err(format!(
            "Export {} declares gameVersion '{shown}'.\nEnum numbers can only be decoded for: 1.12.0, 1.14.7. Add the \
             version's EMatchmakingGameType table first.",
            dir.display()
        ));
    }
    let (player_path, matchmaker_path) = (dir.join("player_hunts.jsonl"), dir.join("matchmaker_hunts.jsonl"));
    if !player_path.is_file() || !matchmaker_path.is_file() {
        return Err("Export is missing player_hunts.jsonl or matchmaker_hunts.jsonl.".to_owned());
    }
    let players = read_jsonl(&player_path)?;
    let matchmakers = read_jsonl(&matchmaker_path)?;
    let marker = format!("CatalogExporter EXPORT_HUNTS {version}");
    let mut converter = Converter {
        version,
        marker: marker.clone(),
        paths: known_paths(&options.vendor_dir)?,
        problems: Vec::new(),
    };
    let row_name = |row: &Value| row.get("rowName").and_then(Value::as_str).unwrap_or("").to_owned();
    let player_rows: Vec<Incoming> = players
        .iter()
        .filter(|r| r.is_object())
        .map(|r| (row_name(r), Ok(converter.player_row(r))))
        .collect();
    let mut groups: BTreeMap<String, Vec<Incoming>> = BTreeMap::new();
    groups.entry(MAIN_TABLE.to_owned()).or_default();
    for row in matchmakers.iter().filter(|r| r.is_object()) {
        let table = row
            .get("sourceTable")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .unwrap_or(MAIN_TABLE);
        groups
            .entry(table.to_owned())
            .or_default()
            .push((row_name(row), converter.matchmaker_row(row)));
    }
    let mut problems = std::mem::take(&mut converter.problems);
    let mut order = vec![(PLAYER_TABLE.to_owned(), player_rows)];
    if let Some(main) = groups.remove(MAIN_TABLE) {
        order.push((MAIN_TABLE.to_owned(), main));
    }
    order.extend(groups);
    let mut merged = Vec::new();
    for (name, rows) in &order {
        merged.push(merge_table(options, name, rows, version, &marker, &mut problems)?);
    }

    let mut out = Vec::new();
    let shown_dir = options
        .repo_root
        .as_ref()
        .and_then(|root| dir.strip_prefix(root).ok())
        .unwrap_or(dir);
    let changelist = present(manifest.get("changelist")).map_or_else(|| "?".to_owned(), |v| js::string(Some(v)));
    out.push(format!(
        "Export: {} ({version}, CL {changelist}): {} player rows, {} matchmaker rows",
        shown_dir.display(),
        players.len(),
        matchmakers.len()
    ));
    let update_flag = if options.update_existing {
        " +update-existing"
    } else {
        ""
    };
    let mode = if options.apply {
        "APPLY"
    } else {
        "DRY RUN (pass --apply to write)"
    };
    out.push(format!("Mode  : {mode}{update_flag}"));
    for table in &merged {
        summary_lines(&mut out, &table.summary, options);
    }
    out.push(format!("\n=== MatchmakingGameType ({version} enum) ==="));
    let names = players.iter().filter(|r| r.is_object()).map(|r| {
        let value = r.get("matchmakingGameType");
        Enum::GameType
            .decode(value, version)
            .map_or_else(|| format!("#{}", js::string(value)), str::to_owned)
    });
    out.push(format!("  {}", counted_text(&counted(names, false))));

    out.push("\n=== referential check ===".to_owned());
    let tables: Vec<&Value> = merged.iter().map(|m| &m.file).collect();
    let rows_of = |file: &Value| {
        file.get(0)
            .and_then(|f| f.get("Rows"))
            .and_then(Value::as_object)
            .cloned()
    };
    let matchmaker_rows: Vec<Map<String, Value>> = tables[1..].iter().filter_map(|f| rows_of(f)).collect();
    let player_merged = rows_of(tables[0]).unwrap_or_default();
    let mut dangling = Vec::new();
    let mut unknown_tables = Vec::new();
    let merged_names: Vec<&str> = merged.iter().map(|m| m.summary.name.as_str()).collect();
    for (row, value) in js_entries(&player_merged) {
        let handles = value
            .get("MatchmakerHuntIDs")
            .and_then(Value::as_array)
            .map(Vec::as_slice);
        for handle in handles.unwrap_or_default() {
            let Some(target) = handle.get("RowName").and_then(Value::as_str) else {
                continue;
            };
            if target.is_empty() || target == "None" {
                continue;
            }
            if !matchmaker_rows.iter().any(|rows| rows.contains_key(target)) {
                dangling.push(format!("{row} -> {target}"));
            }
        }
        let lists = value
            .get("MatchmakerHuntsByTag")
            .and_then(Value::as_array)
            .map(Vec::as_slice);
        for list in lists.unwrap_or_default() {
            let name = list
                .get("MatchmakerTable")
                .and_then(|t| t.get("ObjectName"))
                .and_then(Value::as_str);
            let Some(table) = name
                .and_then(|n| n.strip_prefix("DataTable'"))
                .and_then(|n| n.strip_suffix('\''))
            else {
                continue;
            };
            let vendored = options.vendor_dir.join(format!("{table}.json")).is_file();
            if !merged_names.contains(&table) && !vendored {
                unknown_tables.push(format!("{row} -> {table}"));
            }
        }
    }
    if dangling.is_empty() {
        out.push("  all direct MatchmakerHuntIDs resolve".to_owned());
    } else {
        out.push(format!("  {} dangling direct reference(s):", dangling.len()));
        out.extend(dangling.iter().take(15).map(|d| format!("      {d}")));
        if dangling.len() > 15 {
            out.push(format!("      ...and {} more", dangling.len() - 15));
        }
    }
    if unknown_tables.is_empty() {
        out.push("  every tag-routed MatchmakerTable has a vendored file".to_owned());
    } else {
        out.extend(
            unknown_tables
                .iter()
                .map(|t| format!("  tag-routed table without a vendored file: {t}")),
        );
    }
    if !problems.is_empty() {
        out.push(format!("\n=== enum problems ({}) ===", problems.len()));
        out.extend(problems.iter().take(15).map(|p| format!("  {p}")));
    }

    let mut report = ImportHuntTablesReport {
        export_version: version.to_owned(),
        tables: merged.iter().map(|m| m.summary.clone()).collect(),
        conversion_problems: problems.clone(),
        output: String::new(),
    };
    if !options.apply {
        let again = if options.update_existing {
            " --update-existing"
        } else {
            ""
        };
        out.push(format!(
            "\nDry run complete. Nothing written. Re-run with --apply{again} to commit."
        ));
        report.output = out.join("\n");
        return Ok(report);
    }
    let rejected: usize = merged.iter().map(|m| m.summary.rejected.len()).sum();
    if rejected > 0 {
        out.push(format!(
            "REFUSING TO WRITE: {rejected} matchmaker row(s) have no /Game/ map path.\nGetMapPathFromMatchmakerHuntId \
             requires it, so these rows would resolve and then fail at launch.\nRe-export with the current \
             HuntExporter (it reads the soft-pointer path via FName::GetRawString)."
        ));
        return Err(out.join("\n"));
    }
    if !problems.is_empty() {
        out.push(format!(
            "REFUSING TO WRITE: {} enum value(s) do not decode for {version}.",
            problems.len()
        ));
        return Err(out.join("\n"));
    }
    out.push(String::new());
    std::fs::create_dir_all(&options.vendor_dir).map_err(|e| format!("{}: {e}", options.vendor_dir.display()))?;
    let stamp = options.backup_stamp.clone().unwrap_or_else(|| {
        rustic_isles::season::to_iso_string(chrono::Utc::now().timestamp_millis()).replace([':', '.'], "-")
    });
    let mut new_tables = Vec::new();
    for table in merged.iter().filter(|m| m.changes_table) {
        let s = &table.summary;
        let file_name = format!("{}.json", s.name);
        if !s.new_file {
            let backup = format!("{file_name}.{stamp}.bak");
            std::fs::copy(&s.path, options.vendor_dir.join(&backup)).map_err(|e| format!("{backup}: {e}"))?;
            out.push(format!("Backed up: {backup}"));
        } else {
            new_tables.push(s.name.clone());
        }
        std::fs::write(&s.path, stringify_pretty(&table.file)).map_err(|e| format!("{}: {e}", s.path.display()))?;
        let detail = if options.update_existing {
            format!(", ~{}, game-type names {}", s.updated.len(), s.normalised.len())
        } else {
            String::new()
        };
        out.push(format!(
            "Wrote {file_name} ({} rows: +{}{detail})",
            s.rows_after,
            s.added.len()
        ));
    }
    if !new_tables.is_empty() {
        out.push(format!("\nNEW vendor table(s): {}", new_tables.join(", ")));
        out.push("  DeployServer and the Metagame only read them once they are listed in HUNT_TABLE_FILES".to_owned());
        out.push("  (shared/hunt-manifest/index.js).".to_owned());
    }
    out.push("\nDone. Rebuild and restart DeployServer, then check the boot log:".to_owned());
    out.push("  [HuntTableAudit]    problems=0".to_owned());
    out.push("  [HuntLaunchCoverage] resolvable=N/N".to_owned());
    report.output = out.join("\n");
    Ok(report)
}

/// The converters and the merge in memory, for the fuzz target: every converted row keeps its fixed key order.
pub fn convert_rows_for_fuzz(players: &[Value], matchmakers: &[Value], version: &str) -> Vec<Value> {
    let mut converter = Converter {
        version,
        marker: format!("CatalogExporter EXPORT_HUNTS {version}"),
        paths: HashMap::new(),
        problems: Vec::new(),
    };
    let mut out: Vec<Value> = players
        .iter()
        .filter(|r| r.is_object())
        .map(|r| converter.player_row(r))
        .collect();
    out.extend(
        matchmakers
            .iter()
            .filter(|r| r.is_object())
            .filter_map(|r| converter.matchmaker_row(r).ok()),
    );
    let pairs: Vec<(Value, Value)> = out.windows(2).map(|w| (w[0].clone(), w[1].clone())).collect();
    for (a, b) in pairs {
        if let (Value::Object(mut a), Value::Object(b)) = (a, b) {
            let _ = merge_row(&mut a, &b, &PLAYER_ORDER, version, "fuzz");
        }
    }
    out
}

/// The fixed key orders, for the fuzz target's check.
pub fn key_order(player: bool) -> &'static [&'static str] {
    if player { &PLAYER_ORDER } else { &MATCHMAKER_ORDER }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn enums() {
        assert_eq!(Enum::GameType.decode(Some(&json!(9)), "1.14.7"), Some("Gauntlet"));
        assert_eq!(Enum::GameType.decode(Some(&json!(9)), "1.12.0"), Some("FTUE"));
        assert_eq!(
            Enum::GameType.decode(Some(&json!("EMatchmakingGameType::Gauntlet")), "1.12.0"),
            Some("Gauntlet")
        );
        assert_eq!(Enum::Biome.decode(Some(&json!("EHuntBiome::Lava")), "1.14.7"), None);
        assert_eq!(Enum::Tier.decode(Some(&json!(1.5)), "1.14.7"), None);
    }

    #[test]
    fn pretty() {
        let v = json!({"b": [], "a": {"2": 1, "x": [1, {"c": null}]}, "1": {}});
        assert_eq!(
            stringify_pretty(&v),
            "{\n  \"1\": {},\n  \"b\": [],\n  \"a\": {\n    \"2\": 1,\n    \"x\": [\n      1,\n      {\n        \"c\": null\n      }\n    ]\n  }\n}"
        );
    }

    #[test]
    fn sameness_and_merge() {
        let v = ("1.14.7", "1.14.7");
        assert!(same(&json!("hunt.a"), &json!("Hunt.A"), "HuntTags", v));
        assert!(!same(&json!("hunt.a"), &json!("Hunt.A"), "GameModeOverride", v));
        assert!(same(&json!(1.0000001), &json!(1.0), "MaxPlayers", v));
        let a = json!({"DataTable": {"ObjectName": "DataTable'T'", "ObjectPath": "x"}, "RowName": "R"});
        let b = json!({"DataTable": {"ObjectName": "DataTable'T'", "ObjectPath": "y"}, "RowName": "r"});
        assert!(same(&a, &b, "Region", v));
        let c = json!({"DataTable": {"ObjectName": "DataTable'T'", "ObjectPath": "y"}, "RowName": "Q"});
        assert_eq!(merged(&a, &c, "Region", v)["DataTable"]["ObjectPath"], "x");
        assert_eq!(merged(&a, &c, "Region", v)["RowName"], "Q");
        assert_eq!(merged(&json!(1), &json!(9), "MatchmakingGameType", v), json!(9));
        assert_eq!(
            merged(
                &json!("EMatchmakingGameType::City"),
                &json!(9),
                "MatchmakingGameType",
                v
            ),
            json!("EMatchmakingGameType::Gauntlet")
        );
        let maps_a = json!([{"MapName": "a", "MapAssetName": "/Game/A.a", "Biome": "Moss", "Weighting": 1}]);
        let maps_b = json!([{"MapName": "a", "MapAssetName": "/Game/A.a", "Biome": "Ice"}]);
        let out = merged(&maps_a, &maps_b, "MapList", v);
        assert_eq!(
            out,
            json!([{"MapName": "a", "MapAssetName": "/Game/A.a", "Biome": "Ice", "Weighting": 1}])
        );
        let mut e = json!({"Region": a, "HuntTags": [], "_importedFrom": "old"})
            .as_object()
            .unwrap()
            .clone();
        let i = json!({"Region": b, "HuntTags": ["X"], "MaxPlayers": 4, "_importedFrom": "new"});
        let changes = merge_row(&mut e, i.as_object().unwrap(), &MATCHMAKER_ORDER, "1.14.7", "M");
        assert_eq!(changes, ["HuntTags", "+MaxPlayers"]);
        let keys: Vec<&String> = e.keys().collect();
        assert_eq!(keys, ["Region", "HuntTags", "MaxPlayers", "_importedFrom"]);
        assert_eq!(e["_importedFrom"], "M");
        let mut plain = json!({"HuntTags": []}).as_object().unwrap().clone();
        merge_row(
            &mut plain,
            json!({"HuntTags": ["A"]}).as_object().unwrap(),
            &MATCHMAKER_ORDER,
            "1.14.7",
            "M",
        );
        assert_eq!(plain["_updatedFrom"], "M");
        assert_eq!(
            row_version(&json!({"_updatedFrom": "CatalogExporter EXPORT_HUNTS 1.14.7"})),
            "1.14.7"
        );
        assert_eq!(row_version(&json!({})), "1.12.0");
    }
}
