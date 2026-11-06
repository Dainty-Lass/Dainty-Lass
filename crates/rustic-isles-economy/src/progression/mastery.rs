// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The objective report's checks and the mastery plan (progression spec §4.8, §4.9): credit comes from the stored
//! objective state, never from the increments a report claims.

use std::collections::{HashMap, HashSet};

use serde_json::Value;
use service_kit::jsjson;

use crate::util::{num, utf16_len};

/// Mastery credit a newly completed objective earns on each of its tracks.
pub const MASTERY_CREDIT_PER_OBJECTIVE: i64 = 1;
const MAX_REPORT_UNITS: usize = 131_072;
const PLAYER_TRACK: &str = "MasteryTrack_PlayerLevel";

fn id_ok(value: Option<&Value>) -> bool {
    value.and_then(Value::as_str).is_some_and(|id| {
        (1..=128).contains(&id.chars().count())
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))
    })
}

fn in_range(value: Option<&Value>, max: f64) -> bool {
    let n = num(value);
    n.is_finite() && (0.0..=max).contains(&n)
}

fn list<'a>(report: &'a Value, key: &str) -> Option<&'a [Value]> {
    match report.get(key) {
        None | Some(Value::Null) => Some(&[]),
        Some(Value::Array(items)) => Some(items),
        Some(_) => None,
    }
}

/// §4.8: a JSON array body is an object without the keys, and valid (spec §15 item 12).
pub fn report_is_valid(report: &Value) -> bool {
    if utf16_len(&jsjson::stringify(report)) > MAX_REPORT_UNITS {
        return false;
    }
    let (Some(tracks), Some(objectives)) = (list(report, "progress_tracks"), list(report, "objectives")) else {
        return false;
    };
    if tracks.len() > 256 || objectives.len() > 512 {
        return false;
    }
    tracks
        .iter()
        .all(|t| id_ok(t.get("progression_id")) && in_range(t.get("progress"), 1e9))
        && objectives.iter().all(|o| {
            id_ok(o.get("objective_id"))
                && in_range(o.get("value"), 1e9)
                && (matches!(o.get("completed_count"), None | Some(Value::Null))
                    || in_range(o.get("completed_count"), 1e6))
        })
}

/// An objective id's mastery card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Card {
    /// Not a `MasteryObjective_` id.
    NotMastery,
    /// A mastery objective with no known track.
    Unmapped,
    /// The mastery track the objective credits.
    Track(String),
}

/// The mastery card an objective id credits; weapon codes are mapped to the game's track names.
pub fn card_track(objective_id: &str) -> Card {
    let Some(rest) = objective_id.strip_prefix("MasteryObjective_") else {
        return Card::NotMastery;
    };
    if rest.starts_with("Behemoth_") {
        return Card::Track("MasteryTrack_Behemoth".into());
    }
    let Some(weapon) = rest.strip_prefix("Weapon_") else {
        return Card::Unmapped;
    };
    let code = weapon.split('_').next().unwrap_or("");
    if code.is_empty() || !code.bytes().all(|b| b.is_ascii_alphabetic()) {
        return Card::Unmapped;
    }
    let track = match code {
        "Axe" | "Sword" | "Strikers" | "Hammer" => code,
        "Pike" | "Spear" => "Spear",
        "DP" | "Repeaters" => "Repeaters",
        "CB" | "ChainBlades" | "Chainblades" => "ChainBlades",
        _ => return Card::Unmapped,
    };
    Card::Track(format!("MasteryTrack_Weapon_{track}"))
}

fn runs(text: &str, at_least: usize) -> bool {
    let parts: Vec<&str> = text.split('_').collect();
    parts.len() >= at_least
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// The id grammar used when `mastery_objectives.json` gives no list.
pub fn matches_id_grammar(id: &str) -> bool {
    if id.len() > 128 {
        return false;
    }
    for prefix in ["MasteryObjective_Behemoth_", "MasteryObjective_Weapon_"] {
        if let Some(rest) = id.strip_prefix(prefix) {
            return runs(rest, 2);
        }
    }
    id.strip_prefix("AchievementObjective_")
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
}

/// A host's limits on one report.
#[derive(Debug, Clone, PartialEq)]
pub struct HostLimits {
    /// The most new objectives one report may complete.
    pub max_new: usize,
    /// The largest objective value kept; more is clamped.
    pub max_value: i64,
    /// The largest objective count kept; more is clamped.
    pub max_count: i64,
    /// `mastery_objectives.json`'s ids; none falls back to the grammar
    pub known: Option<HashSet<String>>,
    /// Capped mastery tracks: `(cap, stored total)`
    pub caps: HashMap<String, (i64, i64)>,
}

impl HostLimits {
    /// The standard limits: 64 new objectives, values to 100000000, counts to 10.
    pub fn standard(known: Option<HashSet<String>>, caps: HashMap<String, (i64, i64)>) -> Self {
        Self {
            max_new: 64,
            max_value: 100_000_000,
            max_count: 10,
            known,
            caps,
        }
    }

    fn refusal(&self, id: &str) -> Option<&'static str> {
        if id.is_empty() || id.len() > 128 {
            return Some("bad_id");
        }
        let known = match &self.known {
            Some(list) => list.contains(id),
            None => matches_id_grammar(id),
        };
        if known {
            return None;
        }
        Some(if id.starts_with("MasteryObjective_") {
            "unknown_mastery_id"
        } else if id.starts_with("AchievementObjective_") {
            "unknown_achievement_id"
        } else {
            "unrecognised_id_family"
        })
    }
}

/// `floor(Number(x))`, absent, `null` or not finite as 0.
fn floor_of(value: Option<&Value>) -> i64 {
    let n = num(value);
    if n.is_finite() {
        crate::util::trunc_i64(n.floor())
    } else {
        0
    }
}

/// What one objective report does.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MasteryPlan {
    /// `(id, value, count)` merged with the stored state, in report order (over-limit ones only when stored)
    pub merged: Vec<(String, i64, i64)>,
    /// What `merge_objectives` gets
    pub writes: Vec<(String, i64, i64)>,
    /// `(track, amount)` of mastery credit to grant.
    pub credits: Vec<(String, i64)>,
    /// `(track, progress)` the report claims for each mastery track; credit never exceeds it.
    pub claimed: Vec<(String, i64)>,
    /// Objectives completed by this report.
    pub newly: Vec<String>,
    /// Credits cut to what the report claimed, as `track amount->allowed`.
    pub capped: Vec<String>,
    /// New mastery objectives with no known track.
    pub unmapped: Vec<String>,
    /// `(id, reason)` of objectives a host's report may not write.
    pub ignored: Vec<(String, &'static str)>,
    /// New objectives past the host's limit, left out.
    pub over_limit: Vec<String>,
    /// Values and counts clamped to the host's limits, as text.
    pub clamped: Vec<String>,
    /// Credits cut to a capped track's room, as text.
    pub at_cap: Vec<String>,
}

fn bump(list: &mut Vec<(String, i64)>, key: &str, by: i64) {
    match list.iter_mut().find(|(k, _)| k == key) {
        Some((_, n)) => *n += by,
        None => list.push((key.to_owned(), by)),
    }
}

/// `masteryReport.ts`'s plan over the stored objectives `(value, count)` by id.
pub fn plan_mastery_report(
    report: &Value,
    stored: &HashMap<String, (i64, i64)>,
    host: Option<&HostLimits>,
) -> MasteryPlan {
    let mut plan = MasteryPlan::default();
    let mut reported: Vec<(String, i64, i64)> = Vec::new();
    for objective in list(report, "objectives").unwrap_or(&[]) {
        let id = objective
            .get("objective_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let mut value = floor_of(objective.get("value"));
        let mut count = floor_of(objective.get("completed_count"));
        if let Some(limits) = host {
            if let Some(reason) = limits.refusal(&id) {
                if !plan.ignored.iter().any(|(i, _)| *i == id) {
                    plan.ignored.push((id, reason));
                }
                continue;
            }
            if value > limits.max_value {
                plan.clamped.push(format!("{id} value {value}->{}", limits.max_value));
                value = limits.max_value;
            }
            if count > limits.max_count {
                plan.clamped.push(format!("{id} count {count}->{}", limits.max_count));
                count = limits.max_count;
            }
        }
        match reported.iter_mut().find(|(i, _, _)| *i == id) {
            Some(entry) => {
                entry.1 = entry.1.max(value);
                entry.2 = entry.2.max(count);
            }
            None => reported.push((id, value, count)),
        }
    }
    for track in list(report, "progress_tracks").unwrap_or(&[]) {
        let Some(id) = track.get("progression_id").and_then(Value::as_str) else {
            continue;
        };
        if id.starts_with("MasteryTrack_") {
            bump(&mut plan.claimed, id, floor_of(track.get("progress")).max(0));
        }
    }
    let mut raw: Vec<(String, i64)> = Vec::new();
    let mut new_count = 0usize;
    for (id, value, count) in reported {
        let before = stored.get(&id).copied();
        let card = card_track(&id);
        let is_new = card != Card::NotMastery && before.map_or(0, |b| b.1) < 1 && count >= 1;
        if is_new && host.is_some_and(|l| new_count >= l.max_new) {
            plan.over_limit.push(id.clone());
            if let Some((v, c)) = before {
                plan.merged.push((id, v, c));
            }
            continue;
        }
        let (v0, c0) = before.unwrap_or((0, 0));
        plan.merged.push((id.clone(), v0.max(value), c0.max(count)));
        plan.writes.push((id.clone(), value, count));
        if is_new {
            new_count += 1;
            plan.newly.push(id.clone());
            bump(&mut raw, PLAYER_TRACK, MASTERY_CREDIT_PER_OBJECTIVE);
            match card {
                Card::Track(track) => bump(&mut raw, &track, MASTERY_CREDIT_PER_OBJECTIVE),
                _ => plan.unmapped.push(id),
            }
        }
    }
    for (track, amount) in raw {
        let claimed = plan.claimed.iter().find(|(t, _)| *t == track).map_or(0, |(_, n)| *n);
        let mut allowed = amount.min(claimed);
        if allowed < amount {
            plan.capped.push(format!("{track} {amount}->{allowed}"));
        }
        if let Some((cap, total)) = host.and_then(|l| l.caps.get(&track)) {
            let room = (cap - total).max(0);
            if allowed > room {
                plan.at_cap
                    .push(format!("{track} {allowed}->{room} (total {total}, cap {cap})"));
                allowed = room;
            }
        }
        if allowed > 0 {
            plan.credits.push((track, allowed));
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn report(objectives: Value, tracks: Value) -> Value {
        json!({"objectives": objectives, "progress_tracks": tracks})
    }

    #[test]
    fn validation() {
        assert!(report_is_valid(&json!({})));
        assert!(report_is_valid(&json!([1, 2])));
        assert!(!report_is_valid(&json!({"progress_tracks": 5})));
        assert!(!report_is_valid(
            &json!({"progress_tracks": [{"progression_id": "a", "progress": -1}]})
        ));
        assert!(!report_is_valid(
            &json!({"objectives": [{"objective_id": "a b", "value": 1}]})
        ));
        assert!(!report_is_valid(
            &json!({"objectives": [{"objective_id": "a", "value": 1, "completed_count": 1e7}]})
        ));
        assert!(report_is_valid(
            &json!({"objectives": [{"objective_id": "a", "value": "3", "completed_count": null}]})
        ));
    }

    #[test]
    fn cards() {
        assert_eq!(
            card_track("MasteryObjective_Weapon_Pike_Kills"),
            Card::Track("MasteryTrack_Weapon_Spear".into())
        );
        assert_eq!(
            card_track("MasteryObjective_Weapon_CB_X"),
            Card::Track("MasteryTrack_Weapon_ChainBlades".into())
        );
        assert_eq!(
            card_track("MasteryObjective_Behemoth_Shrike"),
            Card::Track("MasteryTrack_Behemoth".into())
        );
        assert_eq!(card_track("MasteryObjective_Weapon_Bow_X"), Card::Unmapped);
        assert_eq!(card_track("MasteryObjective_Other"), Card::Unmapped);
        assert_eq!(card_track("AchievementObjective_Win"), Card::NotMastery);
        assert!(matches_id_grammar("MasteryObjective_Weapon_Axe_A1"));
        assert!(!matches_id_grammar("MasteryObjective_Weapon_Axe"));
        assert!(matches_id_grammar("AchievementObjective_Win_2"));
    }

    #[test]
    fn table() {
        let sword = "MasteryObjective_Weapon_Sword_Generic_Kills1";
        let r = report(
            json!([{"objective_id": sword, "value": 3, "completed_count": 1}]),
            json!([{"progression_id": "MasteryTrack_PlayerLevel", "progress": 1}, {"progression_id": "MasteryTrack_Weapon_Sword", "progress": 1}]),
        );
        let plan = plan_mastery_report(&r, &HashMap::new(), None);
        assert_eq!(
            plan.credits,
            vec![
                ("MasteryTrack_PlayerLevel".into(), 1),
                ("MasteryTrack_Weapon_Sword".into(), 1)
            ]
        );
        let stored = HashMap::from([(sword.to_owned(), (3, 1))]);
        assert!(plan_mastery_report(&r, &stored, None).credits.is_empty());
        let axe = report(
            json!([{"objective_id": "MasteryObjective_Weapon_Axe_A_B", "value": 1, "completed_count": 1}]),
            json!([{"progression_id": "MasteryTrack_PlayerLevel", "progress": 1}]),
        );
        let plan = plan_mastery_report(&axe, &HashMap::new(), None);
        assert_eq!(plan.credits, vec![("MasteryTrack_PlayerLevel".into(), 1)]);
        assert_eq!(plan.capped, vec!["MasteryTrack_Weapon_Axe 1->0"]);
        let win = report(
            json!([{"objective_id": "AchievementObjective_Win", "value": 1, "completed_count": 1}]),
            json!([]),
        );
        let plan = plan_mastery_report(&win, &HashMap::new(), None);
        assert!(plan.credits.is_empty());
        assert_eq!(plan.writes.len(), 1);
    }

    #[test]
    fn host_row() {
        let objectives: Vec<Value> = (0..4)
            .map(|i| json!({"objective_id": format!("MasteryObjective_Weapon_Axe_A_{i}"), "value": 500, "completed_count": 50}))
            .chain([json!({"objective_id": "Bogus_Id", "value": 1})])
            .collect();
        let r = report(
            Value::Array(objectives),
            json!([{"progression_id": "MasteryTrack_PlayerLevel", "progress": 9}]),
        );
        let limits = HostLimits {
            max_new: 2,
            max_value: 100,
            max_count: 10,
            known: None,
            caps: HashMap::from([("MasteryTrack_PlayerLevel".to_owned(), (3, 2))]),
        };
        let plan = plan_mastery_report(&r, &HashMap::new(), Some(&limits));
        assert_eq!(plan.ignored, vec![("Bogus_Id".to_owned(), "unrecognised_id_family")]);
        assert_eq!((plan.newly.len(), plan.over_limit.len(), plan.writes.len()), (2, 2, 2));
        assert_eq!(plan.clamped.len(), 8);
        assert_eq!(plan.credits, vec![("MasteryTrack_PlayerLevel".into(), 1)]);
        assert_eq!(plan.at_cap, vec!["MasteryTrack_PlayerLevel 2->1 (total 2, cap 3)"]);
        assert!(
            plan.credits
                .iter()
                .all(|(t, n)| *n <= plan.claimed.iter().find(|(c, _)| c == t).map_or(0, |c| c.1))
        );
    }
}
