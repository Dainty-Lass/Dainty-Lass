//! Escalation season progress (`escalationProgress.ts`): the dedicated merge, a host's never-lower merge and the
//! season limits of `escalation_tuning.json`.

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value, json};

use crate::AccountId;
use crate::repo::progression::EscalationProgressRecord;

const MAX_INT32: i64 = 2_147_483_647;
pub const MAX_HOST_ESCALATION_ENTRIES: usize = 512;
const ENTRY_KEY_FIELDS: [&str; 3] = ["talent_id", "reward_id", "unlock_id"];
const MAX_TUNING_LEVEL: i64 = 1000;
const MAX_TUNING_RANKS: usize = 100;

/// `IsNonNegativeInt32`.
fn non_negative_int32(value: Option<&Value>) -> Option<i64> {
    let number = value?.as_f64()?;
    (number.fract() == 0.0 && (0.0..=MAX_INT32 as f64).contains(&number)).then_some(number as i64)
}

/// `RecordToEscalationPayload`: the five fields the 1.12 serializer confirms, in the order Node writes them.
pub fn record_to_payload(record: Option<&EscalationProgressRecord>) -> Value {
    match record {
        None => json!({
            "escalation_level": 0, "next_level_xp": 0, "talents_progress": [], "unlock_progress": [], "update_version": 0
        }),
        Some(r) => json!({
            "escalation_level": r.escalation_level,
            "next_level_xp": r.next_level_xp,
            "talents_progress": r.talents_progress,
            "unlock_progress": r.unlock_progress,
            "update_version": r.update_version,
        }),
    }
}

/// `MergeEscalationProgress`: omitted fields keep their value, the level never goes down, the arrays are replaceable.
pub fn merge_escalation(
    account: &AccountId,
    season_id: &str,
    previous: Option<&EscalationProgressRecord>,
    update: Option<&Value>,
    now: &str,
) -> EscalationProgressRecord {
    let field = |key: &str| update.and_then(|u| u.as_object()).and_then(|o| o.get(key));
    let held_level = previous.map_or(0, |p| p.escalation_level);
    let requested = non_negative_int32(field("escalation_level")).unwrap_or(held_level);
    let array = |key: &str, held: Vec<Value>| match field(key) {
        Some(Value::Array(items)) => items.clone(),
        _ => held,
    };
    EscalationProgressRecord {
        account: account.clone(),
        season_id: season_id.to_owned(),
        escalation_level: held_level.max(requested),
        next_level_xp: non_negative_int32(field("next_level_xp")).unwrap_or(previous.map_or(0, |p| p.next_level_xp)),
        talents_progress: array(
            "talents_progress",
            previous.map_or_else(Vec::new, |p| p.talents_progress.clone()),
        ),
        unlock_progress: array(
            "unlock_progress",
            previous.map_or_else(Vec::new, |p| p.unlock_progress.clone()),
        ),
        update_version: previous.map_or(0, |p| p.update_version) + 1,
        created_at: previous.map_or_else(|| now.to_owned(), |p| p.created_at.clone()),
        updated_at: now.to_owned(),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Season limits

#[derive(Debug, Clone, PartialEq)]
pub struct TalentTuning {
    pub points_to_unlock: i64,
    pub point_costs: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SeasonTuning {
    pub max_level: i64,
    pub xp_to_reach_level: Vec<i64>,
    pub unlock_levels: HashMap<String, i64>,
    pub talents: HashMap<String, TalentTuning>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct EscalationTuning {
    pub seasons: HashMap<String, SeasonTuning>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TuningLoad {
    Loaded(EscalationTuning),
    Missing,
    Invalid(String),
}

impl TuningLoad {
    pub fn tuning(&self) -> Option<&EscalationTuning> {
        match self {
            Self::Loaded(tuning) => Some(tuning),
            _ => None,
        }
    }
}

fn int_in(value: Option<&Value>, min: i64, max: i64) -> Option<i64> {
    let number = value?.as_f64()?;
    (number.fract() == 0.0 && number >= min as f64 && number <= max as f64).then_some(number as i64)
}

fn non_empty_object(value: Option<&Value>) -> Option<&Map<String, Value>> {
    value?.as_object().filter(|o| !o.is_empty())
}

fn parse_season(at: &str, raw: &Value) -> Result<SeasonTuning, String> {
    let Some(object) = raw.as_object() else {
        return Err(format!("{at} is not an object"));
    };
    let max_level = int_in(object.get("maxLevel"), 1, MAX_TUNING_LEVEL)
        .ok_or_else(|| format!("{at}.maxLevel must be an integer 1..{MAX_TUNING_LEVEL}"))?;
    let xp = object
        .get("xpToReachLevel")
        .and_then(Value::as_array)
        .filter(|xp| xp.len() as i64 == max_level)
        .ok_or_else(|| format!("{at}.xpToReachLevel must hold one value per level ({max_level})"))?;
    let mut xp_to_reach_level = Vec::new();
    for (index, value) in xp.iter().enumerate() {
        xp_to_reach_level.push(
            int_in(Some(value), 1, MAX_INT32)
                .ok_or_else(|| format!("{at}.xpToReachLevel[{index}] must be a positive integer"))?,
        );
    }
    let unlocks =
        non_empty_object(object.get("unlocks")).ok_or_else(|| format!("{at}.unlocks must be a non-empty object"))?;
    let mut unlock_levels = HashMap::new();
    for (reward_id, entry) in unlocks {
        let level = entry.as_object().and_then(|e| int_in(e.get("level"), 1, max_level));
        match level {
            Some(level) if !reward_id.is_empty() => {
                unlock_levels.insert(reward_id.clone(), level);
            }
            _ => {
                return Err(format!(
                    "{at}.unlocks.{reward_id}.level must be an integer 1..{max_level}"
                ));
            }
        }
    }
    let talents_raw =
        non_empty_object(object.get("talents")).ok_or_else(|| format!("{at}.talents must be a non-empty object"))?;
    let mut talents = HashMap::new();
    for (talent_id, entry) in talents_raw {
        let Some(entry) = entry.as_object().filter(|_| !talent_id.is_empty()) else {
            return Err(format!("{at}.talents.{talent_id} must be an object"));
        };
        let points_to_unlock = int_in(entry.get("pointsToUnlock"), 0, MAX_TUNING_LEVEL)
            .ok_or_else(|| format!("{at}.talents.{talent_id}.pointsToUnlock must be a non-negative integer"))?;
        let costs = entry
            .get("pointCosts")
            .and_then(Value::as_array)
            .filter(|c| (1..=MAX_TUNING_RANKS).contains(&c.len()))
            .ok_or_else(|| {
                format!("{at}.talents.{talent_id}.pointCosts must list the cost of each rank (1..{MAX_TUNING_RANKS})")
            })?;
        let mut point_costs = Vec::new();
        for (index, cost) in costs.iter().enumerate() {
            point_costs.push(int_in(Some(cost), 0, MAX_TUNING_LEVEL).ok_or_else(|| {
                format!("{at}.talents.{talent_id}.pointCosts[{index}] must be a non-negative integer")
            })?);
        }
        talents.insert(
            talent_id.clone(),
            TalentTuning {
                points_to_unlock,
                point_costs,
            },
        );
    }
    Ok(SeasonTuning {
        max_level,
        xp_to_reach_level,
        unlock_levels,
        talents,
    })
}

/// `ParseEscalationTuning`: one bad value rejects the whole file, with the first problem found.
pub fn parse_tuning(raw: &Value) -> Result<EscalationTuning, String> {
    let Some(object) = raw.as_object() else {
        return Err("the file is not a JSON object".into());
    };
    let seasons_raw = non_empty_object(object.get("seasons")).ok_or("\"seasons\" must be a non-empty object")?;
    let mut seasons = HashMap::new();
    for (season_id, season) in seasons_raw {
        if season_id.is_empty() {
            return Err("a season id is empty".into());
        }
        seasons.insert(
            season_id.clone(),
            parse_season(&format!("seasons.{season_id}"), season)?,
        );
    }
    Ok(EscalationTuning { seasons })
}

/// `LoadEscalationTuning` over the game data root.
pub fn load_tuning(shared: &crate::http::Shared) -> TuningLoad {
    let Some(path) = shared.game_data.backend_file("escalation_tuning.json") else {
        return TuningLoad::Missing;
    };
    if !path.exists() {
        return TuningLoad::Missing;
    }
    match super::tracks::read_game_data(&path) {
        Err(error) => TuningLoad::Invalid(format!("unreadable: {error}")),
        Ok(raw) => match parse_tuning(&raw) {
            Ok(tuning) => TuningLoad::Loaded(tuning),
            Err(problem) => TuningLoad::Invalid(problem),
        },
    }
}

/// `Quote`: request text in a reason, bounded and escaped.
fn quote(value: &str) -> String {
    let bounded: String = if value.chars().count() > 80 {
        format!("{}...", value.chars().take(80).collect::<String>())
    } else {
        value.to_owned()
    };
    Value::String(bounded).to_string()
}

/// `CheckEscalationLevelXp`.
pub fn check_level_xp(season: &SeasonTuning, level: i64, xp: i64) -> Option<String> {
    if level > season.max_level {
        return Some(format!(
            "level {level} is above the season maximum {}",
            season.max_level
        ));
    }
    if level < season.max_level {
        let needed = *season.xp_to_reach_level.get(usize::try_from(level).ok()?)?;
        if xp >= needed {
            return Some(format!(
                "next_level_xp {xp} is not below the {needed} XP level {} takes",
                level + 1
            ));
        }
    }
    None
}

fn rank_cost(talent: &TalentTuning, rank: i64) -> i64 {
    talent.point_costs.iter().take(usize::try_from(rank).unwrap_or(0)).sum()
}

/// `CheckEscalationTalents`: whether a whole `talents_progress` is a build the game could have produced.
pub fn check_talents(season: &SeasonTuning, entries: &Value, level: i64) -> Result<i64, String> {
    let Some(entries) = entries.as_array() else {
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
    let mut purchases: Vec<(i64, i64)> = Vec::new();
    let mut points = 0;
    for (index, entry) in entries.iter().enumerate() {
        let Some(entry) = entry.as_object() else {
            return Err(format!("entry {index} is not an object"));
        };
        let Some(id) = entry.get("talent_id").and_then(Value::as_str) else {
            return Err(format!("entry {index} has no talent_id"));
        };
        let Some(talent) = season.talents.get(id) else {
            return Err(format!("talent {} is not in this season's table", quote(id)));
        };
        if !seen.insert(id.to_owned()) {
            return Err(format!("talent {} appears twice", quote(id)));
        }
        let Some(rank) = int_in(entry.get("rank"), 1, talent.point_costs.len() as i64) else {
            return Err(format!(
                "talent {} needs a rank of 1..{}",
                quote(id),
                talent.point_costs.len()
            ));
        };
        for cost in talent.point_costs.iter().take(rank as usize) {
            purchases.push((talent.points_to_unlock, *cost));
            points += cost;
        }
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

/// `EscalationTalentPoints`: points spent by a stored build; entries that do not resolve count for nothing.
pub fn talent_points(season: &SeasonTuning, entries: &[Value]) -> i64 {
    let mut seen = HashSet::new();
    let mut points = 0;
    for entry in entries {
        let Some(object) = entry.as_object() else { continue };
        let Some(id) = object.get("talent_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(talent) = season.talents.get(id) else { continue };
        if seen.contains(id) {
            continue;
        }
        let Some(rank) = int_in(object.get("rank"), 1, MAX_INT32) else {
            continue;
        };
        seen.insert(id.to_owned());
        points += rank_cost(talent, rank);
    }
    points
}

/// `FilterEscalationUnlocks`: the unlock entries a host may submit at `level`.
pub fn filter_unlocks(season: &SeasonTuning, entries: &[Value], level: i64) -> (Vec<Value>, usize) {
    let mut accepted = Vec::new();
    let mut rejected = 0;
    for entry in entries {
        let unlock_level = entry
            .as_object()
            .and_then(|o| o.get("reward_id")?.as_str())
            .and_then(|id| season.unlock_levels.get(id));
        let (Some(object), Some(unlock_level)) = (entry.as_object(), unlock_level) else {
            rejected += 1;
            continue;
        };
        match object.get("collected") {
            None => {}
            Some(Value::Bool(true)) if level < *unlock_level => {
                rejected += 1;
                continue;
            }
            Some(Value::Bool(_)) => {}
            Some(_) => {
                rejected += 1;
                continue;
            }
        }
        accepted.push(entry.clone());
    }
    (accepted, rejected)
}

// ---------------------------------------------------------------------------------------------------------------
// A host's never-lower merge

fn entry_key(entry: &Value) -> Option<String> {
    let object = entry.as_object()?;
    ENTRY_KEY_FIELDS.iter().find_map(|field| {
        object
            .get(*field)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .map(|v| format!("{field}\u{0}{v}"))
    })
}

fn reward_key(entry: &Value) -> Option<String> {
    entry
        .as_object()
        .and_then(|o| o.get("reward_id")?.as_str())
        .filter(|v| !v.is_empty())
        .map(|v| format!("reward_id\u{0}{v}"))
}

/// `MergeEntryNeverLower`: numbers keep the larger value, a true flag stays true.
fn merge_entry(stored: &Map<String, Value>, incoming: &Map<String, Value>) -> (Map<String, Value>, bool) {
    let mut fields = stored.clone();
    let mut restored = false;
    for (field, value) in incoming {
        let old = fields.get(field);
        if let Some(Value::Number(old)) = old {
            let kept = value.as_f64().zip(old.as_f64()).is_some_and(|(new, old)| new >= old);
            if !kept {
                restored = true;
                continue;
            }
        }
        if old == Some(&Value::Bool(true)) && value != &Value::Bool(true) {
            restored = true;
            continue;
        }
        fields.insert(field.clone(), value.clone());
    }
    (fields, restored)
}

fn as_map(value: &Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

/// `MergeEntriesNeverLower`; `ignored` counts what the host tried to take away or could not be compared.
pub fn merge_entries_never_lower(
    stored: &[Value],
    incoming: &[Value],
    key_of: fn(&Value) -> Option<String>,
) -> (Vec<Value>, usize) {
    let mut order: Vec<String> = Vec::new();
    let mut by_key: HashMap<String, Map<String, Value>> = HashMap::new();
    let mut keyless: Vec<Value> = Vec::new();
    for entry in stored {
        let Some(key) = key_of(entry) else {
            keyless.push(entry.clone());
            continue;
        };
        match by_key.get(&key) {
            None => {
                order.push(key.clone());
                by_key.insert(key, as_map(entry));
            }
            Some(existing) => {
                let merged = merge_entry(existing, &as_map(entry)).0;
                by_key.insert(key, merged);
            }
        }
    }
    let stored_keys: HashSet<String> = order.iter().cloned().collect();
    let mut offered: HashSet<String> = HashSet::new();
    let mut ignored = 0;
    for entry in incoming {
        let Some(key) = key_of(entry) else {
            ignored += 1;
            continue;
        };
        offered.insert(key.clone());
        match by_key.get(&key) {
            None => {
                if by_key.len() >= MAX_HOST_ESCALATION_ENTRIES {
                    ignored += 1;
                    continue;
                }
                order.push(key.clone());
                by_key.insert(key, as_map(entry));
            }
            Some(existing) => {
                let (merged, restored) = merge_entry(existing, &as_map(entry));
                if restored {
                    ignored += 1;
                }
                by_key.insert(key, merged);
            }
        }
    }
    ignored += stored_keys.iter().filter(|key| !offered.contains(*key)).count();
    let mut merged: Vec<Value> = order
        .iter()
        .filter_map(|key| by_key.remove(key))
        .map(Value::Object)
        .collect();
    merged.extend(keyless);
    (merged, ignored)
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct HostMergeSummary {
    pub level_before: i64,
    pub level_after: i64,
    pub xp_before: i64,
    pub xp_after: i64,
    pub level_xp_regression_ignored: bool,
    pub talents: (usize, usize, usize),
    pub unlocks: (usize, usize, usize),
    pub version: i64,
    pub level_xp_rejected: Option<String>,
    pub talent_points: Option<(i64, i64)>,
    pub talents_reset: bool,
    pub talents_rejected: Option<String>,
}

impl HostMergeSummary {
    /// The summary as the host-write tag carries it (`{ season, ...summary }` is built by the caller).
    pub fn to_json(&self) -> Map<String, Value> {
        let mut summary = Map::new();
        summary.insert("levelBefore".into(), self.level_before.into());
        summary.insert("levelAfter".into(), self.level_after.into());
        summary.insert("xpBefore".into(), self.xp_before.into());
        summary.insert("xpAfter".into(), self.xp_after.into());
        summary.insert(
            "levelXpRegressionIgnored".into(),
            self.level_xp_regression_ignored.into(),
        );
        summary.insert(
            "talents".into(),
            json!({ "before": self.talents.0, "after": self.talents.1, "ignored": self.talents.2 }),
        );
        summary.insert(
            "unlocks".into(),
            json!({ "before": self.unlocks.0, "after": self.unlocks.1, "ignored": self.unlocks.2 }),
        );
        summary.insert("version".into(), self.version.into());
        if let Some(reason) = &self.level_xp_rejected {
            summary.insert("levelXpRejected".into(), reason.clone().into());
        }
        if let Some((before, after)) = self.talent_points {
            summary.insert("talentPoints".into(), json!({ "before": before, "after": after }));
        }
        if self.talents_reset {
            summary.insert("talentsReset".into(), true.into());
        }
        if let Some(reason) = &self.talents_rejected {
            summary.insert("talentsRejected".into(), reason.clone().into());
        }
        summary
    }
}

/// `MergeHostEscalationProgress`. With a tuning table the season limits apply; the caller checked that the season is
/// in it (`None` for an unlisted season of a loaded table).
pub fn merge_host_escalation(
    account: &AccountId,
    season_id: &str,
    previous: Option<&EscalationProgressRecord>,
    update: Option<&Value>,
    now: &str,
    tuning: Option<&EscalationTuning>,
) -> Option<(EscalationProgressRecord, HostMergeSummary)> {
    let previous_payload = (
        previous.map_or(0, |p| p.escalation_level),
        previous.map_or(0, |p| p.next_level_xp),
        previous.map_or_else(Vec::new, |p| p.talents_progress.clone()),
        previous.map_or_else(Vec::new, |p| p.unlock_progress.clone()),
        previous.map_or(0, |p| p.update_version),
    );
    let season = match tuning {
        Some(tuning) => Some(tuning.seasons.get(season_id)?),
        None => None,
    };
    let base = merge_escalation(account, season_id, previous, update, now);
    let field = |key: &str| update.and_then(|u| u.as_object()).and_then(|o| o.get(key));
    let requested_level = non_negative_int32(field("escalation_level")).unwrap_or(previous_payload.0);
    let sent_level_xp =
        non_negative_int32(field("escalation_level")).is_some() || non_negative_int32(field("next_level_xp")).is_some();
    let level_xp_rejected = match season {
        Some(season) if sent_level_xp => check_level_xp(season, requested_level, base.next_level_xp),
        _ => None,
    };

    let mut level = previous_payload.0;
    let mut xp = previous_payload.1;
    let mut regression = false;
    if level_xp_rejected.is_some() {
        // The stored level and XP stay
    } else if requested_level > level {
        level = requested_level;
        xp = base.next_level_xp;
    } else if requested_level == level {
        regression = base.next_level_xp < xp;
        xp = xp.max(base.next_level_xp);
    } else {
        regression = true;
    }

    let points_before = season.map_or(0, |s| talent_points(s, &previous_payload.2));
    let mut points_after = points_before;
    let mut talents_rejected = None;
    let mut talents_reset = false;
    let talents: (Vec<Value>, usize) = if !matches!(field("talents_progress"), Some(Value::Array(_))) {
        (base.talents_progress.clone(), 0)
    } else if let Some(season) = season {
        match check_talents(season, &Value::Array(base.talents_progress.clone()), level) {
            Ok(points) => {
                points_after = points;
                talents_reset = points < points_before;
                (base.talents_progress.clone(), 0)
            }
            Err(reason) => {
                talents_rejected = Some(reason);
                (previous_payload.2.clone(), base.talents_progress.len())
            }
        }
    } else {
        merge_entries_never_lower(&previous_payload.2, &base.talents_progress, entry_key)
    };

    let unlocks: (Vec<Value>, usize) = if !matches!(field("unlock_progress"), Some(Value::Array(_))) {
        (base.unlock_progress.clone(), 0)
    } else if let Some(season) = season {
        let (accepted, rejected) = filter_unlocks(season, &base.unlock_progress, level);
        let (merged, ignored) = merge_entries_never_lower(&previous_payload.3, &accepted, reward_key);
        (merged, ignored + rejected)
    } else {
        merge_entries_never_lower(&previous_payload.3, &base.unlock_progress, entry_key)
    };

    let record = EscalationProgressRecord {
        escalation_level: level,
        next_level_xp: xp,
        talents_progress: talents.0.clone(),
        unlock_progress: unlocks.0.clone(),
        ..base
    };
    let mut summary = HostMergeSummary {
        level_before: previous_payload.0,
        level_after: level,
        xp_before: previous_payload.1,
        xp_after: xp,
        level_xp_regression_ignored: regression,
        talents: (previous_payload.2.len(), talents.0.len(), talents.1),
        unlocks: (previous_payload.3.len(), unlocks.0.len(), unlocks.1),
        version: record.update_version,
        ..HostMergeSummary::default()
    };
    if season.is_some() {
        summary.level_xp_rejected = level_xp_rejected;
        summary.talent_points = Some((points_before, points_after));
        summary.talents_reset = talents_reset;
        summary.talents_rejected = talents_rejected;
    }
    Some((record, summary))
}

#[cfg(test)]
mod tests {
    use super::*;
    use module_api::NodeId;

    fn account() -> AccountId {
        AccountId::new(&"n".parse::<NodeId>().unwrap())
    }

    fn tuning() -> EscalationTuning {
        parse_tuning(&json!({ "seasons": { "ESC_SEASON_6": {
            "maxLevel": 25,
            "xpToReachLevel": (1..=25).map(|n| 100 * n).collect::<Vec<_>>(),
            "unlocks": { "Reward_A": { "level": 5 } },
            "talents": {
                "Talent_A": { "pointsToUnlock": 0, "pointCosts": [1, 1, 1, 1] },
                "Talent_B": { "pointsToUnlock": 0, "pointCosts": [1] },
                "Talent_C": { "pointsToUnlock": 4, "pointCosts": [1] },
            }
        } } }))
        .unwrap()
    }

    fn record(level: i64, xp: i64, talents: Value, unlocks: Value, version: i64) -> EscalationProgressRecord {
        EscalationProgressRecord {
            account: account(),
            season_id: "ESC_SEASON_6".into(),
            escalation_level: level,
            next_level_xp: xp,
            talents_progress: talents.as_array().cloned().unwrap_or_default(),
            unlock_progress: unlocks.as_array().cloned().unwrap_or_default(),
            update_version: version,
            created_at: "t0".into(),
            updated_at: "t0".into(),
        }
    }

    #[test]
    fn the_dedicated_merge_keeps_omitted_fields_and_never_lowers_the_level() {
        let held = record(9, 50, json!([{ "talent_id": "Talent_A", "rank": 2 }]), json!([]), 3);
        let update = json!({ "escalation_level": 4, "talents_progress": [] });
        let next = merge_escalation(&account(), "ESC_SEASON_6", Some(&held), Some(&update), "t1");
        assert_eq!(
            (next.escalation_level, next.next_level_xp, next.update_version),
            (9, 50, 4)
        );
        assert!(next.talents_progress.is_empty());
        assert_eq!((next.created_at.as_str(), next.updated_at.as_str()), ("t0", "t1"));
        // a bad value is ignored like an omitted one
        let bad = json!({ "escalation_level": -1, "next_level_xp": 1.5, "unlock_progress": "x" });
        let next = merge_escalation(&account(), "s", None, Some(&bad), "t1");
        assert_eq!(
            (next.escalation_level, next.next_level_xp, next.update_version),
            (0, 0, 1)
        );
    }

    #[test]
    fn a_host_cannot_lower_the_pair_or_take_a_collected_flag_back() {
        let held = record(
            9,
            50,
            json!([{ "talent_id": "Talent_A", "rank": 2 }]),
            json!([{ "reward_id": "Reward_A", "collected": true }]),
            3,
        );
        let update = json!({
            "escalation_level": 9, "next_level_xp": 10,
            "talents_progress": [{ "talent_id": "Talent_A", "rank": 1 }],
            "unlock_progress": [{ "reward_id": "Reward_A", "collected": false }],
        });
        let (next, summary) =
            merge_host_escalation(&account(), "ESC_SEASON_6", Some(&held), Some(&update), "t1", None).unwrap();
        assert_eq!((next.escalation_level, next.next_level_xp), (9, 50));
        assert!(summary.level_xp_regression_ignored);
        assert_eq!(
            next.talents_progress,
            vec![json!({ "talent_id": "Talent_A", "rank": 2 })]
        );
        assert_eq!(
            next.unlock_progress,
            vec![json!({ "reward_id": "Reward_A", "collected": true })]
        );
        assert_eq!((summary.talents.2, summary.unlocks.2), (1, 1));
    }

    #[test]
    fn season_limits_replace_a_valid_build_and_refuse_an_impossible_one() {
        let table = tuning();
        let held = record(9, 50, json!([{ "talent_id": "Talent_A", "rank": 4 }]), json!([]), 3);
        // a respec to a different valid build is accepted
        let update = json!({ "talents_progress": [{ "talent_id": "Talent_B", "rank": 1 }] });
        let (next, summary) = merge_host_escalation(
            &account(),
            "ESC_SEASON_6",
            Some(&held),
            Some(&update),
            "t1",
            Some(&table),
        )
        .unwrap();
        assert_eq!(
            next.talents_progress,
            vec![json!({ "talent_id": "Talent_B", "rank": 1 })]
        );
        assert!(summary.talents_reset);
        assert_eq!(summary.talent_points, Some((4, 1)));
        // 6 points at level 5 are impossible: the stored build stays
        let held = record(5, 50, json!([{ "talent_id": "Talent_A", "rank": 4 }]), json!([]), 3);
        let update = json!({ "talents_progress": [{ "talent_id": "Talent_A", "rank": 4 }, { "talent_id": "Talent_B", "rank": 1 }, { "talent_id": "Talent_C", "rank": 1 }] });
        let (next, summary) = merge_host_escalation(
            &account(),
            "ESC_SEASON_6",
            Some(&held),
            Some(&update),
            "t1",
            Some(&table),
        )
        .unwrap();
        assert_eq!(next.talents_progress, held.talents_progress);
        assert_eq!(
            summary.talents_rejected.as_deref(),
            Some("6 points spent with 5 earned (level 5)")
        );
        // a season the table lacks is the caller's to refuse
        assert!(merge_host_escalation(&account(), "ESC_SEASON_9", None, None, "t1", Some(&table)).is_none());
    }

    #[test]
    fn impossible_level_and_xp_pairs_keep_the_stored_pair() {
        let table = tuning();
        let held = record(3, 10, json!([]), json!([]), 1);
        let update = json!({ "escalation_level": 3, "next_level_xp": 400 });
        let (next, summary) = merge_host_escalation(
            &account(),
            "ESC_SEASON_6",
            Some(&held),
            Some(&update),
            "t1",
            Some(&table),
        )
        .unwrap();
        assert_eq!((next.escalation_level, next.next_level_xp), (3, 10));
        assert!(
            summary
                .level_xp_rejected
                .unwrap()
                .contains("not below the 400 XP level 4 takes")
        );
        let update = json!({ "escalation_level": 26 });
        let (_, summary) = merge_host_escalation(
            &account(),
            "ESC_SEASON_6",
            Some(&held),
            Some(&update),
            "t1",
            Some(&table),
        )
        .unwrap();
        assert_eq!(
            summary.level_xp_rejected.as_deref(),
            Some("level 26 is above the season maximum 25")
        );
    }

    #[test]
    fn an_unlock_collected_before_its_level_is_ignored() {
        let table = tuning();
        let season = &table.seasons["ESC_SEASON_6"];
        let entries = [
            json!({ "reward_id": "Reward_A", "collected": true }),
            json!({ "reward_id": "Reward_Z" }),
            json!({ "reward_id": "Reward_A", "collected": "yes" }),
        ];
        let (accepted, rejected) = filter_unlocks(season, &entries, 4);
        assert_eq!((accepted.len(), rejected), (0, 3));
        let (accepted, rejected) = filter_unlocks(season, &entries, 5);
        assert_eq!((accepted.len(), rejected), (1, 2));
    }

    #[test]
    fn tuning_files_are_validated() {
        assert!(parse_tuning(&json!([])).unwrap_err().contains("not a JSON object"));
        assert!(parse_tuning(&json!({})).unwrap_err().contains("seasons"));
        let bad = json!({ "seasons": { "S": { "maxLevel": 2, "xpToReachLevel": [1] } } });
        assert_eq!(
            parse_tuning(&bad).unwrap_err(),
            "seasons.S.xpToReachLevel must hold one value per level (2)"
        );
    }
}
