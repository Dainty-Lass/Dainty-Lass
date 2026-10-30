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
        if trimmed.is_empty() {
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

fn non_neg_int(val: Option<&Value>) -> Option<i64> {
    let v = val?;
    if let Some(i) = v.as_i64() {
        return (i >= 0).then_some(i);
    }
    if let Some(u) = v.as_u64() {
        return i64::try_from(u).ok();
    }
    if let Some(f) = v.as_f64()
        && f.is_finite()
        && f.fract() == 0.0
        && f >= 0.0
        && f <= (i64::MAX as f64)
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
        let rank = non_neg_int(obj.get("rankId").or_else(|| obj.get("rank_id")));
        let xp = non_neg_int(
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

pub fn build_from_rows(
    tracks: &[Vec<Map<String, Value>>],
    seasons: &[Map<String, Value>],
    active_hunt_pass: &str,
) -> Result<ProgressionBuild, HostError> {
    let _ = seasons;
    let mut seen: HashSet<String> = HashSet::new();
    let mut track_ids: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut derived: Vec<String> = Vec::new();
    let mut path_entries: Vec<Value> = Vec::new();
    let mut pending_prestige: Vec<(String, Vec<Value>)> = Vec::new();

    for group in tracks {
        for row in group {
            let base = row.get("base").and_then(Value::as_object).unwrap_or(row);
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
            if let Some(ref pid) = prestige_id {
                pending_prestige.push((pid.clone(), reqs.clone()));
            }

            let prestige_val = prestige_id.map_or(Value::Null, Value::String);
            path_entries.push(json!({
                "id": id,
                "requirements": reqs,
                "rewards": [],
                "prestige": prestige_val,
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
                "id": pid,
                "requirements": reqs,
                "rewards": [],
                "prestige": Value::Null,
            }));
        }
    }

    let hunt_pass = service_kit::js::trim(active_hunt_pass);
    if !hunt_pass.is_empty() && seen.insert(hunt_pass.to_owned()) {
        track_ids.push(hunt_pass.to_owned());
        let mut hp_reqs = Vec::with_capacity(51);
        for rank in 0..=50_i64 {
            let xp = if rank == 0 { 0_i64 } else { 100_i64 };
            hp_reqs.push(json!({ "rank_id": rank, "xp_required": xp }));
        }
        path_entries.push(json!({
            "id": hunt_pass,
            "requirements": hp_reqs,
            "rewards": [],
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
            "no progression export found in {}",
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
        if let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(existing_text) {
            let gen_by = obj.get("generated_by").and_then(Value::as_str);
            if gen_by != Some(GENERATED_BY) {
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
        assert_eq!(paths[0]["id"], "ExperienceTrack_PlayerLevel");
        assert_eq!(
            paths[0]["requirements"],
            json!([{"rank_id": 1, "xp_required": 100}, {"rank_id": 2, "xp_required": 200}])
        );
        assert_eq!(paths[1]["id"], "ExperienceTrack_Weapon_Sword");
        assert_eq!(paths[1]["prestige"], "PrestigeTrack_Weapon_Sword");
        assert_eq!(paths[2]["id"], "PrestigeTrack_Weapon_Sword");
        assert_eq!(paths[3]["id"], "season43");
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
