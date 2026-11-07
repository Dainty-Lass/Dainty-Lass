// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Escalation season progress (progression spec §4.10): the dedicated server's merge, the host's never-lower merge
//! and the season table that bounds it.

use std::collections::{HashMap, HashSet};

use dainty_service_kit::jsjson;
use rustic_isles_store::AccountId;
use rustic_isles_store::repo::progression::EscalationProgressRecord;
use serde_json::{Map, Value};

use crate::util::obj;

const MAX_INT32: f64 = 2_147_483_647.0;
const MAX_ENTRY_KEYS: usize = 512;

/// A JSON number that is an integer in `[0, 2^31 - 1]`.
pub fn non_negative_int32(value: Option<&Value>) -> Option<i64> {
    let n = value?.as_f64()?;
    (n.is_finite() && n.fract() == 0.0 && (0.0..=MAX_INT32).contains(&n)).then(|| crate::util::whole(n))
}

/// The escalation answer for a stored record, zeros without one.
pub fn escalation_payload(record: Option<&EscalationProgressRecord>) -> Value {
    let (level, xp, talents, unlocks, version) = match record {
        Some(r) => (
            r.escalation_level,
            r.next_level_xp,
            r.talents_progress.clone(),
            r.unlock_progress.clone(),
            r.update_version,
        ),
        None => (0, 0, Vec::new(), Vec::new(), 0),
    };
    obj(vec![
        ("escalation_level", Value::from(level)),
        ("next_level_xp", Value::from(xp)),
        ("talents_progress", Value::Array(talents)),
        ("unlock_progress", Value::Array(unlocks)),
        ("update_version", Value::from(version)),
    ])
}

/// The dedicated server's merge: the level never goes down; everything else is the update's when it is valid.
pub fn merge_escalation(
    previous: Option<&EscalationProgressRecord>,
    update: &Value,
    now: &str,
    account: &AccountId,
    season: &str,
) -> EscalationProgressRecord {
    let field = |key: &str| update.as_object().and_then(|m| m.get(key));
    let p_level = previous.map_or(0, |p| p.escalation_level);
    let level = match non_negative_int32(field("escalation_level")) {
        Some(sent) => p_level.max(sent),
        None => p_level,
    };
    let xp = non_negative_int32(field("next_level_xp")).unwrap_or_else(|| previous.map_or(0, |p| p.next_level_xp));
    let list = |key: &str, stored: Option<&Vec<Value>>| match field(key) {
        Some(Value::Array(items)) => items.clone(),
        _ => stored.cloned().unwrap_or_default(),
    };
    EscalationProgressRecord {
        account: account.clone(),
        season_id: season.to_owned(),
        escalation_level: level,
        next_level_xp: xp,
        talents_progress: list("talents_progress", previous.map(|p| &p.talents_progress)),
        unlock_progress: list("unlock_progress", previous.map(|p| &p.unlock_progress)),
        update_version: previous.map_or(1, |p| p.update_version + 1),
        created_at: previous.map_or_else(|| now.to_owned(), |p| p.created_at.clone()),
        updated_at: now.to_owned(),
    }
}

// ------------------------------------------------------------------ the season table

/// One talent's rules.
#[derive(Debug, Clone, PartialEq)]
pub struct Talent {
    /// Points spent elsewhere before the talent can be bought.
    pub points_to_unlock: i64,
    /// The cost of each rank, in order.
    pub point_costs: Vec<i64>,
}

/// One season's escalation table.
#[derive(Debug, Clone, PartialEq)]
pub struct SeasonTuning {
    /// The highest level.
    pub max_level: i64,
    /// 0-based: index L is the XP level L + 1 takes
    pub xp_to_reach_level: Vec<i64>,
    /// Unlock name to the level that grants it.
    pub unlocks: HashMap<String, i64>,
    /// Talent name to its rules.
    pub talents: HashMap<String, Talent>,
}

/// `escalation_tuning.json`.
#[derive(Debug, Clone, PartialEq)]
pub struct Tuning {
    /// Season id to its table.
    pub seasons: HashMap<String, SeasonTuning>,
}

fn int_in(value: Option<&Value>, low: f64, high: f64) -> Option<i64> {
    let n = value?.as_f64()?;
    (n.is_finite() && n.fract() == 0.0 && (low..=high).contains(&n)).then(|| crate::util::whole(n))
}

/// `escalation_tuning.json`; one bad value rejects the whole file with the first problem.
pub fn parse_tuning(document: &Value) -> Result<Tuning, String> {
    let Some(root) = document.as_object() else {
        return Err("the file is not a JSON object".into());
    };
    let Some(seasons) = root.get("seasons").and_then(Value::as_object).filter(|s| !s.is_empty()) else {
        return Err("\"seasons\" must be a non-empty object".into());
    };
    let mut out = HashMap::new();
    for (s, season) in seasons {
        if s.is_empty() {
            return Err("a season id is empty".into());
        }
        let Some(season) = season.as_object() else {
            return Err(format!("seasons.{s} is not an object"));
        };
        let Some(max) = int_in(season.get("maxLevel"), 1.0, 1000.0) else {
            return Err(format!("seasons.{s}.maxLevel must be an integer 1..1000"));
        };
        let xp = season.get("xpToReachLevel").and_then(Value::as_array);
        let Some(xp) = xp.filter(|l| i64::try_from(l.len()).is_ok_and(|n| n == max)) else {
            return Err(format!(
                "seasons.{s}.xpToReachLevel must hold one value per level ({max})"
            ));
        };
        let mut xp_to_reach_level = Vec::new();
        for (i, v) in xp.iter().enumerate() {
            let Some(v) = int_in(Some(v), 1.0, MAX_INT32) else {
                return Err(format!("seasons.{s}.xpToReachLevel[{i}] must be a positive integer"));
            };
            xp_to_reach_level.push(v);
        }
        let Some(unlock_table) = season
            .get("unlocks")
            .and_then(Value::as_object)
            .filter(|u| !u.is_empty())
        else {
            return Err(format!("seasons.{s}.unlocks must be a non-empty object"));
        };
        let mut unlocks = HashMap::new();
        for (r, unlock) in unlock_table {
            let level = (!r.is_empty())
                .then(|| int_in(unlock.get("level"), 1.0, f64::from(u32::try_from(max).unwrap_or(1000))))
                .flatten();
            let Some(level) = level else {
                return Err(format!("seasons.{s}.unlocks.{r}.level must be an integer 1..{max}"));
            };
            unlocks.insert(r.clone(), level);
        }
        let Some(talent_table) = season
            .get("talents")
            .and_then(Value::as_object)
            .filter(|t| !t.is_empty())
        else {
            return Err(format!("seasons.{s}.talents must be a non-empty object"));
        };
        let mut talents = HashMap::new();
        for (t, talent) in talent_table {
            let Some(talent) = talent.as_object().filter(|_| !t.is_empty()) else {
                return Err(format!("seasons.{s}.talents.{t} must be an object"));
            };
            let Some(gate) = int_in(talent.get("pointsToUnlock"), 0.0, 1000.0) else {
                return Err(format!(
                    "seasons.{s}.talents.{t}.pointsToUnlock must be a non-negative integer"
                ));
            };
            let costs = talent.get("pointCosts").and_then(Value::as_array);
            let Some(costs) = costs.filter(|c| (1..=100).contains(&c.len())) else {
                return Err(format!(
                    "seasons.{s}.talents.{t}.pointCosts must list the cost of each rank (1..100)"
                ));
            };
            let mut point_costs = Vec::new();
            for (i, c) in costs.iter().enumerate() {
                let Some(c) = int_in(Some(c), 0.0, 1000.0) else {
                    return Err(format!(
                        "seasons.{s}.talents.{t}.pointCosts[{i}] must be a non-negative integer"
                    ));
                };
                point_costs.push(c);
            }
            talents.insert(
                t.clone(),
                Talent {
                    points_to_unlock: gate,
                    point_costs,
                },
            );
        }
        out.insert(
            s.clone(),
            SeasonTuning {
                max_level: max,
                xp_to_reach_level,
                unlocks,
                talents,
            },
        );
    }
    Ok(Tuning { seasons: out })
}

/// The level/XP check: `None` when the pair is possible.
pub fn check_level_xp(season: &SeasonTuning, level: i64, xp: i64) -> Option<String> {
    if level > season.max_level {
        return Some(format!(
            "level {level} is above the season maximum {}",
            season.max_level
        ));
    }
    if level < season.max_level
        && let Some(need) = usize::try_from(level)
            .ok()
            .and_then(|i| season.xp_to_reach_level.get(i))
        && xp >= *need
    {
        return Some(format!(
            "next_level_xp {xp} is not below the {need} XP level {} takes",
            level + 1
        ));
    }
    None
}

fn quoted(id: &str) -> String {
    let cut: String = id.chars().take(80).collect();
    let text = jsjson::stringify(&Value::String(cut));
    if id.chars().count() > 80 {
        format!("{text}...")
    } else {
        text
    }
}

fn cost_of(talent: &Talent, rank: usize) -> i64 {
    talent.point_costs.iter().take(rank).sum()
}

/// A whole build checked at `level`: its points, or the first problem.
pub fn check_build(season: &SeasonTuning, build: &Value, level: i64) -> Result<i64, String> {
    let Some(entries) = build.as_array() else {
        return Err("talents_progress is not an array".into());
    };
    if entries.len() > season.talents.len() {
        return Err(format!(
            "{} entries for a season with {} talents",
            entries.len(),
            season.talents.len()
        ));
    }
    let mut seen = HashSet::new();
    let mut purchases = Vec::new();
    let mut points = 0;
    for (i, entry) in entries.iter().enumerate() {
        let Some(entry) = entry.as_object() else {
            return Err(format!("entry {i} is not an object"));
        };
        let Some(id) = entry.get("talent_id").and_then(Value::as_str) else {
            return Err(format!("entry {i} has no talent_id"));
        };
        let Some(talent) = season.talents.get(id) else {
            return Err(format!("talent {} is not in this season's table", quoted(id)));
        };
        if !seen.insert(id.to_owned()) {
            return Err(format!("talent {} appears twice", quoted(id)));
        }
        let ranks = talent.point_costs.len();
        #[allow(clippy::cast_precision_loss, reason = "`ranks` is at most 100")]
        let Some(rank) = int_in(entry.get("rank"), 1.0, ranks as f64) else {
            return Err(format!("talent {} needs a rank of 1..{ranks}", quoted(id)));
        };
        let cost = cost_of(talent, usize::try_from(rank).unwrap_or(0));
        points += cost;
        purchases.push((talent.points_to_unlock, cost));
    }
    let earned = level.min(season.max_level);
    if points > earned {
        return Err(format!("{points} points spent with {earned} earned (level {level})"));
    }
    purchases.sort_by_key(|(gate, _)| *gate);
    let mut spent = 0;
    for (gate, cost) in purchases {
        if spent < gate {
            return Err(format!("a talent gated at {gate} points is bought with {spent} spent"));
        }
        spent += cost;
    }
    Ok(points)
}

/// The points of the entries that resolve; the others count 0.
pub fn stored_points(season: &SeasonTuning, build: &[Value]) -> i64 {
    let mut seen = HashSet::new();
    let mut points = 0;
    for entry in build {
        let Some(id) = entry.get("talent_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(talent) = season.talents.get(id) else { continue };
        if !seen.insert(id.to_owned()) {
            continue;
        }
        let rank = entry
            .get("rank")
            .and_then(Value::as_f64)
            .filter(|r| r.fract() == 0.0 && *r >= 1.0);
        if let Some(rank) = rank {
            points += cost_of(talent, usize::try_from(crate::util::trunc_i64(rank)).unwrap_or(0));
        }
    }
    points
}

/// The unlock entries a level may hold: `(accepted, rejected count)`.
pub fn filter_unlocks(season: &SeasonTuning, unlocks: &[Value], level: i64) -> (Vec<Value>, usize) {
    let mut accepted = Vec::new();
    let mut rejected = 0;
    for entry in unlocks {
        let ok = entry.as_object().is_some_and(|map| {
            let Some(at) = map
                .get("reward_id")
                .and_then(Value::as_str)
                .and_then(|id| season.unlocks.get(id))
            else {
                return false;
            };
            match map.get("collected") {
                None | Some(Value::Bool(false)) => true,
                Some(Value::Bool(true)) => level >= *at,
                _ => false,
            }
        });
        if ok {
            accepted.push(entry.clone());
        } else {
            rejected += 1;
        }
    }
    (accepted, rejected)
}

fn entry_key(entry: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| entry.get(*k).and_then(Value::as_str).filter(|s| !s.is_empty()))
        .map(str::to_owned)
}

/// Field by field: a stored number keeps unless the incoming is a number at least as big; a stored `true` stays.
fn merge_fields(stored: &mut Map<String, Value>, incoming: &Map<String, Value>) -> bool {
    let mut kept = false;
    for (field, value) in incoming {
        match stored.get(field) {
            Some(Value::Number(old)) => match value.as_f64() {
                Some(new) if new >= old.as_f64().unwrap_or(f64::NAN) => {
                    stored.insert(field.clone(), value.clone());
                }
                _ => kept = true,
            },
            Some(Value::Bool(true)) => {
                if value != &Value::Bool(true) {
                    kept = true;
                }
            }
            _ => {
                stored.insert(field.clone(), value.clone());
            }
        }
    }
    kept
}

/// The never-lower merge of entry lists: `(merged, ignored count)`.
pub fn never_lower_merge(stored: &[Value], incoming: &[Value], keys: &[&str]) -> (Vec<Value>, usize) {
    let mut keyed: Vec<(String, Map<String, Value>)> = Vec::new();
    let mut keyless = Vec::new();
    for entry in stored {
        match (entry_key(entry, keys), entry.as_object()) {
            (Some(key), Some(map)) => match keyed.iter_mut().find(|(k, _)| *k == key) {
                Some((_, existing)) => {
                    merge_fields(existing, map);
                }
                None => keyed.push((key, map.clone())),
            },
            _ => keyless.push(entry.clone()),
        }
    }
    let mut ignored = 0;
    let mut sent = HashSet::new();
    for entry in incoming {
        let (Some(key), Some(map)) = (entry_key(entry, keys), entry.as_object()) else {
            ignored += 1;
            continue;
        };
        sent.insert(key.clone());
        match keyed.iter().position(|(k, _)| *k == key) {
            Some(at) => {
                if merge_fields(&mut keyed[at].1, map) {
                    ignored += 1;
                }
            }
            None if keyed.len() >= MAX_ENTRY_KEYS => ignored += 1,
            None => keyed.push((key, map.clone())),
        }
    }
    ignored += keyed.iter().filter(|(k, _)| !sent.contains(k)).count();
    let mut out: Vec<Value> = keyed.into_iter().map(|(_, m)| Value::Object(m)).collect();
    out.extend(keyless);
    (out, ignored)
}

const ENTRY_KEYS: [&str; 3] = ["talent_id", "reward_id", "unlock_id"];

/// A host's merge (§4.10): the record, the tag's summary (after `season`) and the parts it refused.
pub struct HostMerge {
    /// The record to store.
    pub record: EscalationProgressRecord,
    /// The host tag's summary of the merge.
    pub summary: Map<String, Value>,
    /// Why the level and XP sent were refused, if they were.
    pub level_xp_rejected: Option<String>,
    /// Why the talents sent were refused, if they were.
    pub talents_rejected: Option<String>,
}

/// A host's merge: as the dedicated server's, but the level and talents are checked against the season table and a refused part keeps the stored value.
pub fn merge_host_escalation(
    previous: Option<&EscalationProgressRecord>,
    update: &Value,
    now: &str,
    account: &AccountId,
    season_id: &str,
    season: Option<&SeasonTuning>,
) -> HostMerge {
    let mut record = merge_escalation(previous, update, now, account, season_id);
    let field = |key: &str| update.as_object().and_then(|m| m.get(key));
    let (p_level, p_xp) = previous.map_or((0, 0), |p| (p.escalation_level, p.next_level_xp));
    let sent_level = non_negative_int32(field("escalation_level"));
    let sent = sent_level.is_some() || non_negative_int32(field("next_level_xp")).is_some();
    let requested = sent_level.unwrap_or(p_level);
    let level_xp_rejected = season
        .filter(|_| sent)
        .and_then(|s| check_level_xp(s, requested, record.next_level_xp));
    let mut regression = false;
    let (level, xp) = if level_xp_rejected.is_some() {
        (p_level, p_xp)
    } else if requested > p_level {
        (requested, record.next_level_xp)
    } else if requested == p_level {
        if record.next_level_xp < p_xp {
            regression = true;
        }
        (p_level, p_xp.max(record.next_level_xp))
    } else {
        regression = true;
        (p_level, p_xp)
    };
    record.escalation_level = level;
    record.next_level_xp = xp;
    let p_talents: Vec<Value> = previous.map(|p| p.talents_progress.clone()).unwrap_or_default();
    let p_unlocks: Vec<Value> = previous.map(|p| p.unlock_progress.clone()).unwrap_or_default();
    let mut talents_ignored = 0;
    let mut talents_rejected = None;
    let mut talents_reset = false;
    let mut points = None;
    if let Some(Value::Array(sent_talents)) = field("talents_progress") {
        match season {
            Some(s) => match check_build(s, &Value::Array(sent_talents.clone()), level) {
                Ok(after) => {
                    let before = stored_points(s, &p_talents);
                    talents_reset = after < before;
                    points = Some((before, after));
                    record.talents_progress = sent_talents.clone();
                }
                Err(problem) => {
                    let before = stored_points(s, &p_talents);
                    points = Some((before, before));
                    talents_ignored = sent_talents.len();
                    talents_rejected = Some(problem);
                    record.talents_progress = p_talents.clone();
                }
            },
            None => {
                let (merged, ignored) = never_lower_merge(&p_talents, sent_talents, &ENTRY_KEYS);
                record.talents_progress = merged;
                talents_ignored = ignored;
            }
        }
    } else if let Some(s) = season {
        let before = stored_points(s, &p_talents);
        points = Some((before, before));
    }
    let mut unlocks_ignored = 0;
    if let Some(Value::Array(sent_unlocks)) = field("unlock_progress") {
        match season {
            Some(s) => {
                let (accepted, rejected) = filter_unlocks(s, sent_unlocks, level);
                let (merged, ignored) = never_lower_merge(&p_unlocks, &accepted, &["reward_id"]);
                record.unlock_progress = merged;
                unlocks_ignored = ignored + rejected;
            }
            None => {
                let (merged, ignored) = never_lower_merge(&p_unlocks, sent_unlocks, &ENTRY_KEYS);
                record.unlock_progress = merged;
                unlocks_ignored = ignored;
            }
        }
    }
    let counts = |before: usize, after: usize, ignored: usize| {
        obj(vec![
            ("before", Value::from(before)),
            ("after", Value::from(after)),
            ("ignored", Value::from(ignored)),
        ])
    };
    let mut summary = Map::new();
    summary.insert("levelBefore".into(), Value::from(p_level));
    summary.insert("levelAfter".into(), Value::from(level));
    summary.insert("xpBefore".into(), Value::from(p_xp));
    summary.insert("xpAfter".into(), Value::from(xp));
    summary.insert("levelXpRegressionIgnored".into(), Value::Bool(regression));
    if let Some(problem) = &level_xp_rejected {
        summary.insert("levelXpRejected".into(), Value::from(problem.as_str()));
    }
    summary.insert(
        "talents".into(),
        counts(p_talents.len(), record.talents_progress.len(), talents_ignored),
    );
    if let Some((before, after)) = points {
        summary.insert(
            "talentPoints".into(),
            obj(vec![("before", Value::from(before)), ("after", Value::from(after))]),
        );
    }
    if talents_reset {
        summary.insert("talentsReset".into(), Value::Bool(true));
    }
    if let Some(problem) = &talents_rejected {
        summary.insert("talentsRejected".into(), Value::from(problem.as_str()));
    }
    summary.insert(
        "unlocks".into(),
        counts(p_unlocks.len(), record.unlock_progress.len(), unlocks_ignored),
    );
    summary.insert("version".into(), Value::from(record.update_version));
    HostMerge {
        record,
        summary,
        level_xp_rejected,
        talents_rejected,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn account() -> AccountId {
        "account:01ARZ3NDEKTSV4RRFFQ69G5FAV@n".parse().unwrap()
    }

    fn tuning() -> Value {
        json!({"seasons": {"s1": {
            "maxLevel": 10,
            "xpToReachLevel": [10, 20, 30, 40, 50, 60, 70, 80, 90, 100],
            "unlocks": {"r1": {"level": 2}, "r5": {"level": 5}},
            "talents": {"a": {"pointsToUnlock": 0, "pointCosts": [1, 1]}, "b": {"pointsToUnlock": 2, "pointCosts": [3]}},
        }}})
    }

    #[test]
    fn tuning_errors() {
        assert_eq!(parse_tuning(&json!([])).unwrap_err(), "the file is not a JSON object");
        assert_eq!(
            parse_tuning(&json!({"seasons": {}})).unwrap_err(),
            "\"seasons\" must be a non-empty object"
        );
        let mut bad = tuning();
        bad["seasons"]["s1"]["maxLevel"] = json!(0);
        assert_eq!(
            parse_tuning(&bad).unwrap_err(),
            "seasons.s1.maxLevel must be an integer 1..1000"
        );
        let mut bad = tuning();
        bad["seasons"]["s1"]["xpToReachLevel"] = json!([1]);
        assert_eq!(
            parse_tuning(&bad).unwrap_err(),
            "seasons.s1.xpToReachLevel must hold one value per level (10)"
        );
        let mut bad = tuning();
        bad["seasons"]["s1"]["xpToReachLevel"][3] = json!(0);
        assert_eq!(
            parse_tuning(&bad).unwrap_err(),
            "seasons.s1.xpToReachLevel[3] must be a positive integer"
        );
        let mut bad = tuning();
        bad["seasons"]["s1"]["unlocks"]["r1"]["level"] = json!(11);
        assert_eq!(
            parse_tuning(&bad).unwrap_err(),
            "seasons.s1.unlocks.r1.level must be an integer 1..10"
        );
        let mut bad = tuning();
        bad["seasons"]["s1"]["talents"]["a"]["pointCosts"] = json!([]);
        assert_eq!(
            parse_tuning(&bad).unwrap_err(),
            "seasons.s1.talents.a.pointCosts must list the cost of each rank (1..100)"
        );
        assert!(parse_tuning(&tuning()).is_ok());
    }

    #[test]
    fn dedicated() {
        let a = account();
        let first = merge_escalation(
            None,
            &json!({"escalation_level": 9, "next_level_xp": 50}),
            "T1",
            &a,
            "s1",
        );
        let second = merge_escalation(Some(&first), &json!({"escalation_level": 4}), "T2", &a, "s1");
        assert_eq!(
            (second.escalation_level, second.next_level_xp, second.update_version),
            (9, 50, 2)
        );
        for bad in [json!(-1), json!(1.5), json!("x")] {
            let r = merge_escalation(
                Some(&first),
                &json!({"escalation_level": bad, "next_level_xp": bad}),
                "T",
                &a,
                "s1",
            );
            assert_eq!((r.escalation_level, r.next_level_xp), (9, 50));
        }
    }

    #[test]
    fn host() {
        let a = account();
        let tuning = parse_tuning(&tuning()).unwrap();
        let s = &tuning.seasons["s1"];
        let mut stored = merge_escalation(None, &json!({"escalation_level": 5, "next_level_xp": 5}), "T", &a, "s1");
        stored.unlock_progress = vec![json!({"reward_id": "r1", "collected": true})];
        stored.talents_progress = vec![
            json!({"talent_id": "a", "rank": 2}),
            json!({"talent_id": "b", "rank": 1}),
        ];
        let lower = merge_host_escalation(
            Some(&stored),
            &json!({"escalation_level": 3, "unlock_progress": [{"reward_id": "r1", "collected": false}]}),
            "T2",
            &a,
            "s1",
            Some(s),
        );
        assert_eq!(lower.record.escalation_level, 5);
        assert_eq!(
            lower.record.unlock_progress,
            vec![json!({"reward_id": "r1", "collected": true})]
        );
        assert_eq!(lower.summary["levelXpRegressionIgnored"], true);
        assert_eq!(lower.summary["unlocks"]["ignored"], 1);
        let respec = merge_host_escalation(
            Some(&stored),
            &json!({"talents_progress": [{"talent_id": "a", "rank": 1}]}),
            "T3",
            &a,
            "s1",
            Some(s),
        );
        assert_eq!(respec.summary["talentsReset"], true);
        assert_eq!(
            respec.record.talents_progress,
            vec![json!({"talent_id": "a", "rank": 1})]
        );
        let over = merge_host_escalation(
            Some(&stored),
            &json!({"talents_progress": [{"talent_id": "a", "rank": 2}, {"talent_id": "b", "rank": 1}, ]}),
            "T4",
            &a,
            "s1",
            Some(s),
        );
        assert!(over.talents_rejected.is_none());
        let six = merge_host_escalation(
            Some(&stored),
            &json!({"escalation_level": 5, "talents_progress": [{"talent_id": "b", "rank": 1}, {"talent_id": "a", "rank": 2}, {"talent_id": "zz", "rank": 1}]}),
            "T5",
            &a,
            "s1",
            Some(s),
        );
        assert_eq!(
            six.talents_rejected.as_deref(),
            Some("3 entries for a season with 2 talents")
        );
        let gated = check_build(s, &json!([{"talent_id": "b", "rank": 1}]), 5);
        assert_eq!(gated.unwrap_err(), "a talent gated at 2 points is bought with 0 spent");
        assert_eq!(
            check_build(
                s,
                &json!([{"talent_id": "a", "rank": 2}, {"talent_id": "b", "rank": 1}]),
                4
            )
            .unwrap_err(),
            "5 points spent with 4 earned (level 4)"
        );
        assert_eq!(
            check_level_xp(s, 11, 0).unwrap(),
            "level 11 is above the season maximum 10"
        );
        assert_eq!(
            check_level_xp(s, 2, 30).unwrap(),
            "next_level_xp 30 is not below the 30 XP level 3 takes"
        );
    }

    #[test]
    fn entry_cap() {
        let stored: Vec<Value> = (0..512).map(|i| json!({"reward_id": format!("r{i}")})).collect();
        let (merged, ignored) = never_lower_merge(&stored, &[json!({"reward_id": "new"})], &ENTRY_KEYS);
        assert_eq!(merged.len(), 512);
        assert_eq!(ignored, 1 + 512);
    }
}
