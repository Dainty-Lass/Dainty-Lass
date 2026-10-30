// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Builds `progression_config.source.json` from the 1.14.7 CatalogExporter JSONL progression export.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::error::HostError;

pub const PROGRESSION_EXPORT_DIR: &str = "progression_1_14_7";
pub const TRACK_EXPORTS: [&str; 4] = [
    "player-experience-tracks.jsonl",
    "weapon-experience-tracks.jsonl",
    "experience-tracks.jsonl",
    "mastery-tracks.jsonl",
];
pub const HUNT_PASS_EXPORT: &str = "huntpass-season-table.jsonl";
pub const ACTIVE_HUNT_PASS: &str = "season43";
pub const GENERATED_BY: &str = "DaintyLass host kit, from the CatalogExporter progression export";

const START_DATE: &str = "2019-01-01T14:00:00+00:00";
const END_DATE: &str = "2099-01-01T14:00:00+00:00";
const MAX_SAFE_INT: i64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, PartialEq)]
pub struct ProgressionBuild {
    pub config: Value,
    pub paths: usize,
    pub tracks: Vec<String>,
    pub skipped: Vec<String>,
    pub derived: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceWritten {
    Supplied,
    NoExport,
    Written { written: usize, derived: usize },
}

pub fn read_jsonl(file: &Path) -> Result<Vec<Map<String, Value>>, HostError> {
    let text = std::fs::read_to_string(file).map_err(|e| HostError::io(file, e))?;
    Ok(parse_jsonl(&text))
}

pub fn parse_jsonl(text: &str) -> Vec<Map<String, Value>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = Vec::new();
    for line in text.split('\n') {
        let trimmed = service_kit::js::trim(line);
        if trimmed.is_empty() || trimmed.starts_with("status:") {
            continue;
        }
        if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(trimmed) {
            rows.push(map);
        }
    }
    rows
}

fn clean_id(val: Option<&Value>) -> Option<String> {
    let s = service_kit::js::trim(val?.as_str()?);
    if s.is_empty() || s.eq_ignore_ascii_case("none") {
        None
    } else {
        Some(s.to_owned())
    }
}

fn non_neg_safe_int(val: Option<&Value>) -> Option<i64> {
    let v = val?;
    if let Some(i) = v.as_i64() {
        return (0..=MAX_SAFE_INT).contains(&i).then_some(i);
    }
    if let Some(u) = v.as_u64() {
        let i = i64::try_from(u).ok()?;
        return (0..=MAX_SAFE_INT).contains(&i).then_some(i);
    }
    if let Some(f) = v.as_f64()
        && f.is_finite()
        && f.fract() == 0.0
        && f >= 0.0
        && f <= (MAX_SAFE_INT as f64)
    {
        return Some(f as i64);
    }
    None
}

fn extract_requirements(row: &Map<String, Value>, base: &Map<String, Value>) -> Vec<Value> {
    let Some(arr) = base
        .get("requirements")
        .or_else(|| row.get("requirements"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let mut pairs: Vec<(i64, i64)> = Vec::with_capacity(arr.len());
    for item in arr {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let rank = non_neg_safe_int(obj.get("rankId").or_else(|| obj.get("rank_id")));
        let xp = non_neg_safe_int(
            obj.get("requiredXP")
                .or_else(|| obj.get("xp_required"))
                .or_else(|| obj.get("xpRequired")),
        );
        if let (Some(rank), Some(xp)) = (rank, xp) {
            pairs.push((rank, xp));
        }
    }
    pairs.sort_by_key(|(rank, _)| *rank);
    pairs.dedup_by_key(|(rank, _)| *rank);
    pairs
        .into_iter()
        .map(|(rank_id, xp_required)| json!({ "rank_id": rank_id, "xp_required": xp_required }))
        .collect()
}

fn extract_rewards(raw_rewards: Option<&Value>) -> Vec<Value> {
    let Some(arr) = raw_rewards.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let rank_id = non_neg_safe_int(obj.get("rankId").or_else(|| obj.get("rank_id"))).unwrap_or(0);
        let custom_str = match obj.get("customRewardString").and_then(Value::as_str) {
            Some(s) if !s.is_empty() => Value::String(s.to_owned()),
            _ => Value::Null,
        };

        let mut stacked_items = Vec::new();
        if let Some(stacked) = obj.get("stackedItems").and_then(Value::as_array) {
            for s_item in stacked {
                let Some(sobj) = s_item.as_object() else {
                    continue;
                };
                let Some(cat_id) = sobj
                    .get("catalogId")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                else {
                    continue;
                };
                let hidden = sobj.get("hidden").and_then(Value::as_bool).unwrap_or(false);
                let priority = non_neg_safe_int(sobj.get("priority")).unwrap_or(0);
                let quantity = non_neg_safe_int(sobj.get("quantity"))
                    .filter(|&q| q >= 1)
                    .unwrap_or(1);
                stacked_items.push(json!({
                    "catalog_id": cat_id,
                    "hidden": hidden,
                    "priority": priority,
                    "quantity": quantity,
                }));
            }
        }

        let mut instanced_items = Vec::new();
        if let Some(inst) = obj.get("instancedItems").and_then(Value::as_array) {
            for i_val in inst {
                if let Some(s) = i_val.as_str().filter(|s| !s.is_empty()) {
                    instanced_items.push(Value::String(s.to_owned()));
                }
            }
        }

        let mut ordered_instanced_items = Vec::new();
        if let Some(ord) = obj.get("orderedInstancedItems").and_then(Value::as_array) {
            for o_item in ord {
                let Some(oobj) = o_item.as_object() else {
                    continue;
                };
                let Some(cat_id) = oobj
                    .get("catalogId")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                else {
                    continue;
                };
                let priority = non_neg_safe_int(oobj.get("priority")).unwrap_or(0);
                ordered_instanced_items.push(json!({
                    "catalog_id": cat_id,
                    "priority": priority,
                }));
            }
        }

        let mut entitlements = Vec::new();
        if let Some(ents) = obj.get("entitlements").and_then(Value::as_array) {
            for e_item in ents {
                let Some(eobj) = e_item.as_object() else {
                    continue;
                };
                let Some(ent_id) = eobj
                    .get("entitlementId")
                    .or_else(|| eobj.get("entitlement"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                else {
                    continue;
                };
                let duration = non_neg_safe_int(eobj.get("duration")).unwrap_or(0);
                entitlements.push(json!({
                    "duration": duration,
                    "entitlement": ent_id,
                }));
            }
        }

        out.push(json!({
            "rank_id": rank_id,
            "custom_reward_string": {
                "culture_invariant_string": custom_str,
            },
            "stacked_items": stacked_items,
            "instanced_items": instanced_items,
            "ordered_instanced_items": ordered_instanced_items,
            "entitlements": entitlements,
            "buffs": [],
        }));
    }
    out
}

fn multiplier_value(base: &Map<String, Value>) -> Value {
    if let Some(v) = base.get("progressionMultiplier")
        && let Some(f) = v.as_f64()
        && f.is_finite()
        && f > 0.0
    {
        if f.fract() == 0.0 && f <= (MAX_SAFE_INT as f64) {
            return json!(f as i64);
        }
        return json!(f);
    }
    json!(1)
}

pub fn build_from_rows(
    tracks: &[Vec<Map<String, Value>>],
    seasons: &[Map<String, Value>],
    active_hunt_pass: &str,
) -> Result<ProgressionBuild, HostError> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut track_ids: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut derived: Vec<String> = Vec::new();
    let mut path_entries: Vec<Value> = Vec::new();
    let mut pending_prestige: Vec<(String, Vec<Value>)> = Vec::new();

    for group in tracks {
        for row in group {
            let Some(base) = row.get("base").and_then(Value::as_object) else {
                if let Some(id) = clean_id(row.get("progressionTrack"))
                    .or_else(|| clean_id(row.get("rowName")))
                    .or_else(|| clean_id(row.get("id")))
                    && !seen.contains(&id)
                    && !skipped.contains(&id)
                {
                    skipped.push(id);
                }
                continue;
            };
            let Some(id) = clean_id(base.get("progressionTrack"))
                .or_else(|| clean_id(row.get("progressionTrack")))
                .or_else(|| clean_id(row.get("rowName")))
                .or_else(|| clean_id(row.get("id")))
            else {
                continue;
            };
            if seen.contains(&id) {
                continue;
            }
            let reqs = extract_requirements(row, base);
            if reqs.is_empty() {
                if !skipped.contains(&id) {
                    skipped.push(id);
                }
                continue;
            }
            seen.insert(id.clone());
            skipped.retain(|s| s != &id);
            track_ids.push(id.clone());

            let explicit_prestige = clean_id(row.get("prestigeTrack"))
                .or_else(|| clean_id(base.get("prestigeTrack")));
            let prestige_id = match explicit_prestige {
                Some(p) => Some(p),
                None => id
                    .strip_prefix("ExperienceTrack_Weapon_")
                    .filter(|w| !w.is_empty())
                    .map(|w| format!("PrestigeTrack_Weapon_{w}")),
            };
            if let Some(pid) = prestige_id {
                pending_prestige.push((pid, reqs.clone()));
            }

            let gating = base
                .get("premiumGatingEntitlement")
                .and_then(Value::as_str)
                .unwrap_or("");
            let mult = multiplier_value(base);
            let free_rewards = extract_rewards(base.get("freeRewards"));
            let premium_rewards = extract_rewards(base.get("premiumRewards"));

            path_entries.push(json!({
                "progression_id": id,
                "premium_gating_entitlement": gating,
                "progression_multiplier": mult,
                "start_date": START_DATE,
                "end_date": END_DATE,
                "requirements": reqs,
                "free_rewards": free_rewards,
                "premium_rewards": premium_rewards,
            }));
        }
    }

    if !seen.contains("ExperienceTrack_PlayerLevel") {
        return Err(HostError::message(
            "the progression export has no ExperienceTrack_PlayerLevel track",
        ));
    }

    for (pid, reqs) in pending_prestige {
        if seen.insert(pid.clone()) {
            derived.push(pid.clone());
            track_ids.push(pid.clone());
            path_entries.push(json!({
                "progression_id": pid,
                "premium_gating_entitlement": "",
                "progression_multiplier": 1,
                "start_date": START_DATE,
                "end_date": END_DATE,
                "requirements": reqs,
                "free_rewards": [],
                "premium_rewards": [],
            }));
        }
    }

    let hunt_pass = service_kit::js::trim(active_hunt_pass);
    if !hunt_pass.is_empty() && seen.insert(hunt_pass.to_owned()) {
        track_ids.push(hunt_pass.to_owned());
        let gating = seasons
            .iter()
            .find_map(|row| {
                let base = row.get("base").and_then(Value::as_object)?;
                let id = clean_id(base.get("progressionTrack"))
                    .or_else(|| clean_id(row.get("rowName")))?;
                if id.eq_ignore_ascii_case(hunt_pass) {
                    base.get("premiumGatingEntitlement")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| format!("{hunt_pass}_premium"));

        let mut hp_reqs = Vec::with_capacity(51);
        let mut hp_rewards = Vec::with_capacity(51);
        for rank in 0..=50_i64 {
            let xp = if rank == 0 { 0_i64 } else { 100_i64 };
            hp_reqs.push(json!({ "rank_id": rank, "xp_required": xp }));
            hp_rewards.push(json!({
                "rank_id": rank,
                "stacked_items": [],
                "instanced_items": [],
                "ordered_instanced_items": [],
                "entitlements": [],
            }));
        }
        path_entries.push(json!({
            "progression_id": hunt_pass,
            "premium_gating_entitlement": gating,
            "progression_multiplier": 1,
            "start_date": START_DATE,
            "end_date": END_DATE,
            "requirements": hp_reqs,
            "free_rewards": hp_rewards.clone(),
            "premium_rewards": hp_rewards,
            "prestige": Value::Null,
        }));
    }

    let paths_count = path_entries.len();
    let config = json!({
        "generated_by": GENERATED_BY,
        "code": Value::Null,
        "message": "OK",
        "payload": {
            "paths": path_entries,
        },
    });

    Ok(ProgressionBuild {
        config,
        paths: paths_count,
        tracks: track_ids,
        skipped,
        derived,
    })
}

fn resolve_export_dir(exports_dir: &Path) -> PathBuf {
    let sub = exports_dir.join(PROGRESSION_EXPORT_DIR);
    if sub.is_dir() {
        sub
    } else {
        exports_dir.to_path_buf()
    }
}

pub fn build_progression_config(
    exports_dir: &Path,
    active_hunt_pass: &str,
) -> Result<ProgressionBuild, HostError> {
    let dir = resolve_export_dir(exports_dir);
    let mut track_groups = Vec::with_capacity(TRACK_EXPORTS.len());
    let mut any_found = false;
    for name in TRACK_EXPORTS {
        let file = dir.join(name);
        if file.is_file() {
            any_found = true;
            track_groups.push(read_jsonl(&file)?);
        }
    }
    if !any_found {
        return Err(HostError::message(format!(
            "no progression export found in {} (run CatalogExporter with EXPORT_PROGRESSION=1)",
            dir.display()
        )));
    }
    let seasons_file = dir.join(HUNT_PASS_EXPORT);
    let seasons = if seasons_file.is_file() {
        read_jsonl(&seasons_file)?
    } else {
        Vec::new()
    };
    build_from_rows(&track_groups, &seasons, active_hunt_pass)
}

pub fn progression_text(build: &ProgressionBuild) -> String {
    let mut text = crate::hostconfig::stringify_pretty(&build.config);
    text.push('\n');
    text
}

pub fn write_progression_source(
    exports_dir: &Path,
    source_file: &Path,
) -> Result<SourceWritten, HostError> {
    if source_file.is_file()
        && let Ok(existing_text) = std::fs::read_to_string(source_file)
    {
        let existing_text = existing_text.strip_prefix('\u{feff}').unwrap_or(&existing_text);
        match serde_json::from_str::<Value>(existing_text) {
            Ok(Value::Object(obj)) => {
                let gen_by = obj.get("generated_by").and_then(Value::as_str);
                if gen_by != Some(GENERATED_BY) {
                    return Ok(SourceWritten::Supplied);
                }
            }
            _ => {
                return Ok(SourceWritten::Supplied);
            }
        }
    }

    let dir = resolve_export_dir(exports_dir);
    if !TRACK_EXPORTS.iter().any(|name| dir.join(name).is_file()) {
        return Ok(SourceWritten::NoExport);
    }

    let build = build_progression_config(exports_dir, ACTIVE_HUNT_PASS)?;
    if let Some(parent) = source_file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| HostError::io(parent, e))?;
    }
    let text = progression_text(&build);
    std::fs::write(source_file, text).map_err(|e| HostError::io(source_file, e))?;
    Ok(SourceWritten::Written {
        written: build.paths,
        derived: build.derived.len(),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn builds_tracks_derives_weapon_prestige_and_adds_hunt_pass() {
        let player_rows = parse_jsonl(
            r#"{"rowName":"ExperienceTrack_PlayerLevel","base":{"progressionTrack":"ExperienceTrack_PlayerLevel","requirements":[{"rankId":2,"requiredXP":200},{"rankId":1,"requiredXP":100}]}}
{"rowName":"EmptyTrack","base":{"progressionTrack":"EmptyTrack","requirements":[]}}"#,
        );
        let weapon_rows = parse_jsonl(
            r#"{"rowName":"Sword","base":{"progressionTrack":"ExperienceTrack_Weapon_Sword","requirements":[{"rankId":1,"requiredXP":50}]}}"#,
        );
        let build = build_from_rows(&[player_rows, weapon_rows], &[], ACTIVE_HUNT_PASS).unwrap();
        assert_eq!(
            build.tracks,
            vec![
                "ExperienceTrack_PlayerLevel".to_owned(),
                "ExperienceTrack_Weapon_Sword".to_owned(),
                "PrestigeTrack_Weapon_Sword".to_owned(),
                "season43".to_owned(),
            ]
        );
        assert_eq!(build.skipped, vec!["EmptyTrack".to_owned()]);
        assert_eq!(build.derived, vec!["PrestigeTrack_Weapon_Sword".to_owned()]);
        assert_eq!(build.paths, 4);

        let paths = build.config["payload"]["paths"].as_array().unwrap();
        assert_eq!(paths[0]["progression_id"], "ExperienceTrack_PlayerLevel");
        assert_eq!(
            paths[0]["requirements"],
            json!([{"rank_id": 1, "xp_required": 100}, {"rank_id": 2, "xp_required": 200}])
        );
        assert_eq!(paths[1]["progression_id"], "ExperienceTrack_Weapon_Sword");
        assert_eq!(paths[2]["progression_id"], "PrestigeTrack_Weapon_Sword");
        assert_eq!(paths[3]["progression_id"], "season43");
        assert_eq!(paths[3]["requirements"].as_array().unwrap().len(), 51);
    }

    #[test]
    fn preserves_supplied_source_file_and_writes_when_generated() {
        let tmp = tempfile::tempdir().unwrap();
        let exports = tmp.path().join("Items_Analysis");
        let prog_dir = exports.join(PROGRESSION_EXPORT_DIR);
        std::fs::create_dir_all(&prog_dir).unwrap();
        let source_file = tmp.path().join("raw").join("progression_config.source.json");

        assert_eq!(
            write_progression_source(&exports, &source_file).unwrap(),
            SourceWritten::NoExport
        );

        std::fs::write(
            prog_dir.join("player-experience-tracks.jsonl"),
            r#"{"rowName":"ExperienceTrack_PlayerLevel","base":{"progressionTrack":"ExperienceTrack_PlayerLevel","requirements":[{"rankId":1,"requiredXP":100}]}}"#,
        )
        .unwrap();

        let res = write_progression_source(&exports, &source_file).unwrap();
        assert_eq!(res, SourceWritten::Written { written: 2, derived: 0 });
        assert!(source_file.is_file());

        // Overwrite with a user-supplied file (no generated_by matching GENERATED_BY)
        std::fs::write(&source_file, r#"{"payload":{"paths":[]}}"#).unwrap();
        assert_eq!(
            write_progression_source(&exports, &source_file).unwrap(),
            SourceWritten::Supplied
        );
    }
}
