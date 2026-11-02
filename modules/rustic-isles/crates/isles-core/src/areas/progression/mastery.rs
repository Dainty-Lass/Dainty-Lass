//! Mastery from the game server's progression report (`controllers/masteryReport.ts`, 1.14.7): credit comes from the
//! stored objective state, not from the claimed increments, and runs in one transaction. A player-hosted server's
//! report is bounded by [`HostMasteryLimits`].

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value, json};
use service_kit::js;

use crate::http::Shared;
use crate::repo::progression::ProgressionTrackRecord;

use super::tracks::{MASTERY_CREDIT_PER_OBJECTIVE, read_game_data, track_cap_xp};

pub const MASTERY_PLAYER_TRACK: &str = "MasteryTrack_PlayerLevel";
const MASTERY_BEHEMOTH_TRACK: &str = "MasteryTrack_Behemoth";
pub const HOST_MASTERY_MAX_NEWLY_COMPLETED: usize = 64;
pub const HOST_MASTERY_MAX_COMPLETED_COUNT: i64 = 10;
pub const HOST_MASTERY_MAX_VALUE: i64 = 100_000_000;
const OBJECTIVE_ID_MAX_LENGTH: usize = 128;

fn weapon_code_track(code: &str) -> Option<&'static str> {
    Some(match code {
        "Axe" => "MasteryTrack_Weapon_Axe",
        "Sword" => "MasteryTrack_Weapon_Sword",
        "Strikers" => "MasteryTrack_Weapon_Strikers",
        "Pike" | "Spear" => "MasteryTrack_Weapon_Spear",
        "DP" | "Repeaters" => "MasteryTrack_Weapon_Repeaters",
        "CB" | "ChainBlades" | "Chainblades" => "MasteryTrack_Weapon_ChainBlades",
        "Hammer" => "MasteryTrack_Weapon_Hammer",
        _ => return None,
    })
}

/// The card track a mastery objective feeds: `Some(Some(track))`, `Some(None)` for an unknown weapon code, `None`
/// for a non-mastery id (`MasteryCardTrackFor`).
pub fn mastery_card_track(objective_id: &str) -> Option<Option<&'static str>> {
    if !objective_id.starts_with("MasteryObjective_") {
        return None;
    }
    if objective_id.starts_with("MasteryObjective_Behemoth_") {
        return Some(Some(MASTERY_BEHEMOTH_TRACK));
    }
    if let Some(rest) = objective_id.strip_prefix("MasteryObjective_Weapon_")
        && let Some((code, _)) = rest.split_once('_')
        && !code.is_empty()
        && code.bytes().all(|b| b.is_ascii_alphabetic())
    {
        return Some(weapon_code_track(code));
    }
    Some(None)
}

/// `/^MasteryObjective_(?:Behemoth|Weapon)_[A-Za-z0-9]+(?:_[A-Za-z0-9]+)+$/` or `/^AchievementObjective_[A-Za-z0-9_]+$/`.
pub fn is_objective_id_shape(id: &str) -> bool {
    if id.len() > OBJECTIVE_ID_MAX_LENGTH {
        return false;
    }
    let mastery = ["MasteryObjective_Behemoth_", "MasteryObjective_Weapon_"]
        .iter()
        .find_map(|prefix| id.strip_prefix(prefix))
        .is_some_and(|rest| {
            let segments: Vec<&str> = rest.split('_').collect();
            segments.len() >= 2
                && segments
                    .iter()
                    .all(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric()))
        });
    let achievement = id
        .strip_prefix("AchievementObjective_")
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'));
    mastery || achievement
}

/// What a host's report may name: the ids of `mastery_objectives.json` when it loads, else the id grammar.
#[derive(Debug, Clone, PartialEq)]
pub enum KnownObjectives {
    Ids(HashSet<String>),
    Shape,
}

impl KnownObjectives {
    /// `BuildMasteryObjectiveCheck` over the parsed file (`{ objectiveIds: [...] }`).
    pub fn from_file(data: &Value) -> Self {
        let ids: HashSet<String> = data
            .get("objectiveIds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|id| id.as_str().map(str::to_owned))
            .collect();
        if ids.is_empty() { Self::Shape } else { Self::Ids(ids) }
    }

    pub fn load(shared: &Shared) -> Self {
        let Some(path) = shared.game_data.backend_file("mastery_objectives.json") else {
            return Self::Shape;
        };
        if !path.exists() {
            return Self::Shape;
        }
        match read_game_data(&path) {
            Ok(data) => Self::from_file(&data),
            Err(error) => {
                tracing::warn!(
                    "[MasteryMon] game-data/mastery_objectives.json is unreadable ({error}); host mastery reports use the id grammar"
                );
                Self::Shape
            }
        }
    }

    pub fn knows(&self, id: &str) -> bool {
        match self {
            Self::Ids(ids) => ids.contains(id),
            Self::Shape => is_objective_id_shape(id),
        }
    }
}

/// `HostObjectiveIdRefusal`.
pub fn host_objective_id_refusal(id: &str, known: &KnownObjectives) -> Option<&'static str> {
    if id.is_empty() || id.len() > OBJECTIVE_ID_MAX_LENGTH {
        return Some("bad_id");
    }
    if id.starts_with("MasteryObjective_") {
        return (!known.knows(id)).then_some("unknown_mastery_id");
    }
    if id.starts_with("AchievementObjective_") {
        return (!known.knows(id)).then_some("unknown_achievement_id");
    }
    Some("unrecognised_id_family")
}

#[derive(Debug, Clone)]
pub struct HostMasteryLimits {
    pub max_newly_completed: usize,
    pub max_completed_count: i64,
    pub max_value: i64,
    pub known: KnownObjectives,
    /// A mastery track's rank-sum total, by track id
    pub track_caps: HashMap<String, i64>,
    /// The stored totals of the mastery tracks the report claims; filled inside the transaction
    pub track_totals: Option<HashMap<String, i64>>,
}

impl HostMasteryLimits {
    pub fn new(shared: &Shared) -> Self {
        let mut caps = HashMap::new();
        for id in super::tracks::DEFAULT_PROGRESSION_IDS
            .iter()
            .chain(["MasteryTrack_Behemoth"].iter())
            .filter(|id| id.starts_with("MasteryTrack_"))
        {
            if let Some(cap) = track_cap_xp(shared, id).filter(|c| *c > 0.0) {
                caps.insert((*id).to_owned(), cap as i64);
            }
        }
        Self {
            max_newly_completed: HOST_MASTERY_MAX_NEWLY_COMPLETED,
            max_completed_count: HOST_MASTERY_MAX_COMPLETED_COUNT,
            max_value: HOST_MASTERY_MAX_VALUE,
            known: KnownObjectives::load(shared),
            track_caps: caps,
            track_totals: None,
        }
    }

    /// Mastery tracks of the report that have a cap, whose stored total the transaction reads.
    pub fn capped_claims(&self, body: &Value) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for track in body
            .get("progress_tracks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let id = js::string(track.get("progression_id"));
            if id.starts_with("MasteryTrack_") && self.track_caps.contains_key(&id) && !out.contains(&id) {
                out.push(id);
            }
        }
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StoredObjective {
    pub value: i64,
    pub completed_count: i64,
}

/// An insertion-ordered map, as JavaScript's `Map` and object keys are.
#[derive(Debug, Clone, PartialEq)]
pub struct Ordered<V> {
    keys: Vec<String>,
    map: HashMap<String, V>,
}

impl<V> Default for Ordered<V> {
    fn default() -> Self {
        Self {
            keys: Vec::new(),
            map: HashMap::new(),
        }
    }
}

impl<V> Ordered<V> {
    pub fn get(&self, key: &str) -> Option<&V> {
        self.map.get(key)
    }

    pub fn set(&mut self, key: &str, value: V) {
        if self.map.insert(key.to_owned(), value).is_none() {
            self.keys.push(key.to_owned());
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &V)> {
        self.keys.iter().filter_map(|key| Some((key, self.map.get(key)?)))
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn contains(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MasteryPlan {
    /// Reported state, duplicates merged
    pub objectives: Vec<(String, StoredObjective)>,
    /// `max(stored, reported)`, for the response
    pub merged: Ordered<StoredObjective>,
    pub newly_completed: Vec<String>,
    pub credits: Ordered<i64>,
    pub claimed: Ordered<i64>,
    pub unmapped: Vec<String>,
    pub capped: Vec<String>,
    pub ignored: Vec<(String, &'static str)>,
    pub over_limit: Vec<String>,
    pub clamped: Vec<String>,
    pub at_cap: Vec<String>,
}

/// `Math.floor(Number(x ?? 0))` as an integer; a value that is not finite is 0 (the report was validated before).
fn floor_number(value: Option<&Value>) -> i64 {
    let number = match value {
        None | Some(Value::Null) => 0.0,
        other => js::number(other),
    };
    if number.is_finite() { number.floor() as i64 } else { 0 }
}

/// `PlanMasteryReport`: what this report changes, given the stored state of its objectives.
pub fn plan_mastery_report(
    body: &Value,
    stored: &HashMap<String, StoredObjective>,
    limits: Option<&HostMasteryLimits>,
) -> MasteryPlan {
    let mut reported: Ordered<StoredObjective> = Ordered::default();
    let mut ignored: Vec<(String, &'static str)> = Vec::new();
    let mut ignored_ids: HashSet<String> = HashSet::new();
    let mut clamped: Vec<String> = Vec::new();
    for objective in body.get("objectives").and_then(Value::as_array).into_iter().flatten() {
        let id = js::string(objective.get("objective_id"));
        let mut value = floor_number(objective.get("value"));
        let mut count = floor_number(objective.get("completed_count"));
        if let Some(limits) = limits {
            if let Some(reason) = host_objective_id_refusal(&id, &limits.known) {
                if ignored_ids.insert(id.clone()) {
                    ignored.push((id, reason));
                }
                continue;
            }
            if value > limits.max_value {
                clamped.push(format!("{id} value {value}->{}", limits.max_value));
                value = limits.max_value;
            }
            if count > limits.max_completed_count {
                clamped.push(format!("{id} count {count}->{}", limits.max_completed_count));
                count = limits.max_completed_count;
            }
        }
        let previous = reported.get(&id).copied().unwrap_or_default();
        reported.set(
            &id,
            StoredObjective {
                value: previous.value.max(value),
                completed_count: previous.completed_count.max(count),
            },
        );
    }

    let mut claimed: Ordered<i64> = Ordered::default();
    for track in body
        .get("progress_tracks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let id = js::string(track.get("progression_id"));
        if !id.starts_with("MasteryTrack_") {
            continue;
        }
        let amount = floor_number(track.get("progress")).max(0);
        claimed.set(&id, claimed.get(&id).copied().unwrap_or(0) + amount);
    }

    let mut raw: Ordered<i64> = Ordered::default();
    let mut newly_completed: Vec<String> = Vec::new();
    let mut unmapped: Vec<String> = Vec::new();
    let mut merged: Ordered<StoredObjective> = Ordered::default();
    let mut skipped: HashSet<String> = HashSet::new();
    let mut over_limit: Vec<String> = Vec::new();
    let add = |raw: &mut Ordered<i64>, track: &str| {
        let held = raw.get(track).copied().unwrap_or(0);
        raw.set(track, held + MASTERY_CREDIT_PER_OBJECTIVE);
    };
    for (id, state) in reported.iter() {
        let before = stored.get(id).copied();
        let card = mastery_card_track(id);
        let is_new = card.is_some() && before.map_or(0, |b| b.completed_count) < 1 && state.completed_count >= 1;
        if is_new && limits.is_some_and(|l| newly_completed.len() >= l.max_newly_completed) {
            // Past the per-report limit nothing of this row is stored, so a later report can still credit it
            over_limit.push(id.clone());
            skipped.insert(id.clone());
            if let Some(before) = before {
                merged.set(id, before);
            }
            continue;
        }
        merged.set(
            id,
            StoredObjective {
                value: before.map_or(0, |b| b.value).max(state.value),
                completed_count: before.map_or(0, |b| b.completed_count).max(state.completed_count),
            },
        );
        let Some(card) = card else { continue };
        if !is_new {
            continue;
        }
        newly_completed.push(id.clone());
        add(&mut raw, MASTERY_PLAYER_TRACK);
        match card {
            None => unmapped.push(id.clone()),
            Some(track) => add(&mut raw, track),
        }
    }

    let mut credits: Ordered<i64> = Ordered::default();
    let mut capped: Vec<String> = Vec::new();
    let mut at_cap: Vec<String> = Vec::new();
    for (track, amount) in raw.iter() {
        let mut allowed = (*amount).min(claimed.get(track).copied().unwrap_or(0));
        if allowed < *amount {
            capped.push(format!("{track} {amount}->{allowed}"));
        }
        let cap = limits.and_then(|l| l.track_caps.get(track.as_str()).copied());
        let total = limits.and_then(|l| l.track_totals.as_ref()?.get(track.as_str()).copied());
        if let (Some(cap), Some(total)) = (cap, total) {
            let room = (cap - total).max(0);
            if allowed > room {
                at_cap.push(format!("{track} {allowed}->{room} (total {total}, cap {cap})"));
                allowed = room;
            }
        }
        if allowed > 0 {
            credits.set(track, allowed);
        }
    }

    MasteryPlan {
        objectives: reported
            .iter()
            .filter(|(id, _)| !skipped.contains(*id))
            .map(|(id, s)| (id.clone(), *s))
            .collect(),
        merged,
        newly_completed,
        credits,
        claimed,
        unmapped,
        capped,
        ignored,
        over_limit,
        clamped,
        at_cap,
    }
}

/// `SummarizeMasteryPlan`: what a stored report changed, for the host-write tag.
pub fn summarize_plan(plan: &MasteryPlan) -> Value {
    let counts = |ordered: &Ordered<i64>| {
        Value::Object(
            ordered
                .iter()
                .map(|(k, v)| (k.clone(), Value::from(*v)))
                .collect::<Map<_, _>>(),
        )
    };
    let first = |items: &[String]| Value::from(items.iter().take(20).cloned().collect::<Vec<_>>());
    let mut summary = Map::new();
    summary.insert("objectives".into(), plan.objectives.len().into());
    summary.insert("newlyCompleted".into(), first(&plan.newly_completed));
    summary.insert("newlyCompletedCount".into(), plan.newly_completed.len().into());
    summary.insert("credits".into(), counts(&plan.credits));
    summary.insert("claimed".into(), counts(&plan.claimed));
    if !plan.capped.is_empty() {
        summary.insert("capped".into(), first(&plan.capped));
    }
    if !plan.unmapped.is_empty() {
        summary.insert("unmapped".into(), first(&plan.unmapped));
    }
    if !plan.ignored.is_empty() {
        summary.insert("ignoredCount".into(), plan.ignored.len().into());
        let ignored: Vec<String> = plan
            .ignored
            .iter()
            .take(20)
            .map(|(id, why)| format!("{id}:{why}"))
            .collect();
        summary.insert("ignored".into(), ignored.into());
    }
    if !plan.over_limit.is_empty() {
        summary.insert("overLimitCount".into(), plan.over_limit.len().into());
        summary.insert("overLimit".into(), first(&plan.over_limit));
    }
    if !plan.clamped.is_empty() {
        summary.insert("clamped".into(), first(&plan.clamped));
    }
    if !plan.at_cap.is_empty() {
        summary.insert("atCap".into(), first(&plan.at_cap));
    }
    Value::Object(summary)
}

/// `DescribeMasteryLimits`.
pub fn describe_limits(plan: &MasteryPlan) -> Option<String> {
    if plan.ignored.is_empty() && plan.over_limit.is_empty() && plan.clamped.is_empty() && plan.at_cap.is_empty() {
        return None;
    }
    let head = |items: Vec<String>| {
        let more = if items.len() > 10 { ", ..." } else { "" };
        format!(
            "{}[{}{more}]",
            items.len(),
            items.iter().take(10).cloned().collect::<Vec<_>>().join(", ")
        )
    };
    Some(format!(
        "ignored={} overLimit={} clamped={} atCap={}",
        head(plan.ignored.iter().map(|(id, why)| format!("{id}:{why}")).collect()),
        head(plan.over_limit.clone()),
        head(plan.clamped.clone()),
        head(plan.at_cap.clone()),
    ))
}

/// `TrackRow`: a track as the response lists it.
pub fn track_row(user_id: &str, progression_id: &str, row: Option<&ProgressionTrackRecord>, now: &str) -> Value {
    json!({
        "phx_account_id": user_id,
        "progression_id": progression_id,
        "progress": row.map_or(0, |r| r.progress),
        "confirmed_fremium_rank": row.map_or(0, |r| r.confirmed_fremium_rank),
        "confirmed_premium_rank": row.map_or(0, |r| r.confirmed_premium_rank),
        "confirmed_date": row.map_or(now, |r| r.updated_at.as_str()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(rows: &[(&str, i64, i64)]) -> HashMap<String, StoredObjective> {
        rows.iter()
            .map(|(id, value, completed_count)| {
                (
                    (*id).to_owned(),
                    StoredObjective {
                        value: *value,
                        completed_count: *completed_count,
                    },
                )
            })
            .collect()
    }

    fn report(objectives: Value, tracks: Value) -> Value {
        json!({ "objectives": objectives, "progress_tracks": tracks })
    }

    #[test]
    fn card_tracks_follow_the_weapon_codes() {
        assert_eq!(
            mastery_card_track("MasteryObjective_Weapon_Sword_Generic_Kills1"),
            Some(Some("MasteryTrack_Weapon_Sword"))
        );
        assert_eq!(
            mastery_card_track("MasteryObjective_Weapon_Pike_Kill"),
            Some(Some("MasteryTrack_Weapon_Spear"))
        );
        assert_eq!(
            mastery_card_track("MasteryObjective_Weapon_CB_Kill"),
            Some(Some("MasteryTrack_Weapon_ChainBlades"))
        );
        assert_eq!(mastery_card_track("MasteryObjective_Weapon_Bow_Kill"), Some(None));
        assert_eq!(
            mastery_card_track("MasteryObjective_Behemoth_Embermane_Boop"),
            Some(Some("MasteryTrack_Behemoth"))
        );
        assert_eq!(mastery_card_track("AchievementObjective_X"), None);
    }

    #[test]
    fn a_first_completion_credits_the_player_and_the_card_track() {
        let body = report(
            json!([{ "objective_id": "MasteryObjective_Weapon_Sword_Generic_Kills1", "value": 5, "completed_count": 1 }]),
            json!([{ "progression_id": "MasteryTrack_PlayerLevel", "progress": 1 }, { "progression_id": "MasteryTrack_Weapon_Sword", "progress": 1 }]),
        );
        let plan = plan_mastery_report(&body, &HashMap::new(), None);
        assert_eq!(plan.newly_completed, ["MasteryObjective_Weapon_Sword_Generic_Kills1"]);
        let credits: Vec<_> = plan.credits.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        assert_eq!(
            credits,
            [("MasteryTrack_PlayerLevel", 1), ("MasteryTrack_Weapon_Sword", 1)]
        );
        // already complete in the store: a re-sent arrival report credits nothing
        let held = stored(&[("MasteryObjective_Weapon_Sword_Generic_Kills1", 3, 1)]);
        let again = plan_mastery_report(&body, &held, None);
        assert!(again.newly_completed.is_empty() && again.credits.is_empty());
        assert_eq!(
            again
                .merged
                .get("MasteryObjective_Weapon_Sword_Generic_Kills1")
                .map(|s| s.value),
            Some(5)
        );
    }

    #[test]
    fn credit_is_bounded_by_what_the_report_claimed() {
        let body = report(
            json!([{ "objective_id": "MasteryObjective_Weapon_Axe_A_B", "value": 1, "completed_count": 1 }]),
            json!([{ "progression_id": "MasteryTrack_PlayerLevel", "progress": 1 }]),
        );
        let plan = plan_mastery_report(&body, &HashMap::new(), None);
        assert_eq!(plan.capped, ["MasteryTrack_Weapon_Axe 1->0"]);
        let credits: Vec<_> = plan.credits.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(credits, ["MasteryTrack_PlayerLevel"]);
    }

    #[test]
    fn an_achievement_is_stored_and_credits_nothing() {
        let body = report(
            json!([{ "objective_id": "AchievementObjective_Win", "value": 1, "completed_count": 1 }]),
            json!([]),
        );
        let plan = plan_mastery_report(&body, &HashMap::new(), None);
        assert!(plan.credits.is_empty() && plan.newly_completed.is_empty());
        assert_eq!(plan.objectives.len(), 1);
    }

    fn limits(max_new: usize) -> HostMasteryLimits {
        HostMasteryLimits {
            max_newly_completed: max_new,
            max_completed_count: 10,
            max_value: 100,
            known: KnownObjectives::Shape,
            track_caps: HashMap::from([(MASTERY_PLAYER_TRACK.to_owned(), 3)]),
            track_totals: Some(HashMap::from([(MASTERY_PLAYER_TRACK.to_owned(), 2)])),
        }
    }

    #[test]
    fn a_hosts_report_is_bounded() {
        let objectives: Vec<Value> = (0..4)
            .map(|n| json!({ "objective_id": format!("MasteryObjective_Weapon_Axe_Kill{n}_X"), "value": 500, "completed_count": 50 }))
            .chain([json!({ "objective_id": "Bogus_Id", "value": 1, "completed_count": 1 })])
            .collect();
        let body = report(
            json!(objectives),
            json!([{ "progression_id": "MasteryTrack_PlayerLevel", "progress": 9 }]),
        );
        let plan = plan_mastery_report(&body, &HashMap::new(), Some(&limits(2)));
        assert_eq!(plan.ignored, [("Bogus_Id".to_owned(), "unrecognised_id_family")]);
        assert_eq!(plan.newly_completed.len(), 2);
        assert_eq!(plan.over_limit.len(), 2);
        assert!(plan.clamped.iter().any(|c| c.contains("value 500->100")));
        assert!(plan.clamped.iter().any(|c| c.contains("count 50->10")));
        // the player track has room for 1 more only
        assert_eq!(plan.credits.get(MASTERY_PLAYER_TRACK), Some(&1));
        assert_eq!(plan.at_cap.len(), 1);
        assert_eq!(plan.objectives.len(), 2);
        let summary = summarize_plan(&plan);
        assert_eq!(summary["ignoredCount"], 1);
        assert!(describe_limits(&plan).is_some());
    }

    #[test]
    fn the_id_grammar_matches_the_live_ids() {
        for id in [
            "MasteryObjective_Weapon_Sword_Generic_Kills1",
            "MasteryObjective_Behemoth_Embermane_Boop_X_Times",
            "AchievementObjective_Some_Thing",
        ] {
            assert!(is_objective_id_shape(id), "{id}");
        }
        for id in [
            "MasteryObjective_Weapon_Sword",
            "MasteryObjective_Other_A_B",
            "AchievementObjective_",
            "x",
        ] {
            assert!(!is_objective_id_shape(id), "{id}");
        }
    }
}
