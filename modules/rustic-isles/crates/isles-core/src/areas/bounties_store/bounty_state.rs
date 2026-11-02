//! Per-account bounty state for `GET/POST /bounty/:userId` (`bountyState.ts`): the wire shape, the monotonic merge of
//! the game server's saves, claim deletes, a player-host runtime's limited saves and the season-challenge rewards.
//! Pure: the persistence is [`crate::repo::bounties::BountyStateRepository`], the retry loop the handler's.

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value, json};
use service_kit::js;
use service_kit::jsjson::to_js;

use super::authn::AuthData;
use super::util::{DAY_MS, is_plain_object, iso, js_len, num, number_or_zero, parse_ms, parse_ms_f64};
use crate::game_data::season_window::{ACTIVE_SEASON_LENGTH_MS, active_season_window_ms};
use crate::ids::AccountId;
use crate::repo::bounties::{BountyGroup, BountyStateRecord, DraftData, RewardCredit, StoredBounty};

pub const BOUNTY_SEASON_START_DATE: &str = "2020-08-23T00:00:00.000Z";
pub const BOUNTY_SEASON_END_DATE: &str = "2099-01-01T00:00:00.000Z";
pub const MAX_STORED_BOUNTIES: usize = 1000;
const MAX_BOUNTIES_PER_SAVE: usize = 512;
const MAX_BOUNTY_ID_LENGTH: usize = 200;

/// The draft key a component's save carries, and the group it files.
const DRAFT_KEYS: [(&str, BountyGroup); 3] = [
    ("draft_data", BountyGroup::Base),
    ("draft_data_daily", BountyGroup::Daily),
    ("draft_data_weekly", BountyGroup::Weekly),
];

pub fn default_draft_data() -> Value {
    json!({
        "current_draft_choices": [],
        "previous_draft_selections": [],
        "bronze_count": 0,
        "silver_count": 0,
        "gold_count": 0,
    })
}

/// Writes come from the authoritative dedicated server only: a player bearer could otherwise unclaim or re-date a
/// claimed challenge and collect its reward again.
pub fn is_authorized_bounty_writer(auth: &AuthData) -> bool {
    auth.is_gameserver
}

/// A player-host runtime (never a game server); the route then asks rifts' host-write hooks.
pub fn is_host_bounty_writer(auth: &AuthData) -> bool {
    auth.is_host_runtime() && !auth.is_gameserver
}

/// A player-host runtime reads the bounty state of its own hunt's roster only.
pub fn is_authorized_bounty_reader(auth: &AuthData, target_user_id: &str, target: Option<&AccountId>) -> bool {
    if auth.is_gameserver {
        return true;
    }
    if auth.is_host_runtime() {
        return auth.roster_has(target_user_id, target);
    }
    auth.user_id.as_deref() == Some(target_user_id)
}

pub fn infer_bounty_group(bounty_id: &str) -> BountyGroup {
    if bounty_id.starts_with("Challenge_Daily_") {
        BountyGroup::Daily
    } else if bounty_id.contains("Challenge_Season") {
        BountyGroup::Weekly
    } else {
        BountyGroup::Base
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct BountySaveValidationError(pub String);

fn invalid(message: impl Into<String>) -> BountySaveValidationError {
    BountySaveValidationError(message.into())
}

fn bounty_id(bounty: &Value) -> &str {
    bounty.get("bounty_id").and_then(Value::as_str).unwrap_or_default()
}

fn is_true(value: &Value, key: &str) -> bool {
    value.get(key) == Some(&Value::Bool(true))
}

type DraftKey = (&'static str, BountyGroup);

/// `ValidateBountySave`: the save's bounties verbatim and the draft keys it carries.
pub fn validate_bounty_save(body: Option<&Value>) -> Result<(Vec<Value>, Vec<DraftKey>), BountySaveValidationError> {
    let Some(Value::Object(body)) = body else {
        return Err(invalid("body must be an object"));
    };
    let bounties = match body.get("bounties") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(list)) => list.clone(),
        Some(_) => return Err(invalid("bounties must be an array")),
    };
    if bounties.len() > MAX_BOUNTIES_PER_SAVE {
        return Err(invalid(format!("too many bounties ({})", bounties.len())));
    }
    for bounty in &bounties {
        if !bounty.is_object() {
            return Err(invalid("every bounty must be an object"));
        }
        let id = bounty.get("bounty_id").and_then(Value::as_str);
        if id.is_none_or(|id| id.is_empty() || js_len(id) > MAX_BOUNTY_ID_LENGTH) {
            return Err(invalid("every bounty needs a bounty_id string"));
        }
        if bounty.get("objectives").is_some_and(|o| !o.is_array()) {
            return Err(invalid(format!("objectives of {} must be an array", bounty_id(bounty))));
        }
    }
    let draft_keys: Vec<DraftKey> = DRAFT_KEYS
        .iter()
        .filter(|(key, _)| body.get(*key).is_some())
        .copied()
        .collect();
    for (key, _) in &draft_keys {
        if !body[*key].is_object() {
            return Err(invalid(format!("{key} must be an object")));
        }
    }
    Ok((bounties, draft_keys))
}

fn same_instance(a: &Value, b: &Value) -> bool {
    let stamp = |v: &Value| match v.get("drafted_timestamp") {
        None | Some(Value::Null) => Value::Null,
        Some(other) => other.clone(),
    };
    js::strict_eq(a.get("bounty_id"), b.get("bounty_id")) && js::strict_eq(Some(&stamp(a)), Some(&stamp(b)))
}

fn objective_id(objective: &Value) -> Option<&str> {
    objective.as_object()?.get("objective_id")?.as_str()
}

/// `MergeBountyInstance`: each objective's progress keeps the maximum, `claimed` never goes back, `update_version` keeps
/// the maximum.
pub fn merge_bounty_instance(stored: &Value, incoming: &Value) -> Value {
    let stored_objectives: &[Value] = stored
        .get("objectives")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    let mut stored_progress: HashMap<&str, f64> = HashMap::new();
    for objective in stored_objectives {
        if let Some(id) = objective_id(objective) {
            stored_progress.insert(id, number_or_zero(objective.get("progress")));
        }
    }
    let incoming_objectives: &[Value] = incoming
        .get("objectives")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    let mut seen: HashSet<&str> = HashSet::new();
    let mut objectives: Vec<Value> = incoming_objectives
        .iter()
        .map(|objective| {
            let Some(id) = objective_id(objective) else {
                return objective.clone();
            };
            seen.insert(id);
            let progress = number_or_zero(objective.get("progress"));
            match stored_progress.get(id) {
                Some(&previous) if previous > progress => {
                    let mut copy = objective.clone();
                    if let Some(map) = copy.as_object_mut() {
                        map.insert("progress".into(), num(previous));
                    }
                    copy
                }
                _ => objective.clone(),
            }
        })
        .collect();
    // An objective the stored copy has and the incoming one omits is kept rather than dropped
    for objective in stored_objectives {
        if objective_id(objective).is_some_and(|id| !seen.contains(id)) {
            objectives.push(objective.clone());
        }
    }
    let mut merged: Map<String, Value> = incoming.as_object().cloned().unwrap_or_default();
    merged.insert("objectives".into(), Value::Array(objectives));
    if is_true(stored, "claimed") {
        merged.insert("claimed".into(), Value::Bool(true));
    }
    let stored_version = js::number(stored.get("update_version"));
    let incoming_version = js::number(incoming.get("update_version"));
    if stored_version.is_finite() && (!incoming_version.is_finite() || stored_version > incoming_version) {
        merged.insert("update_version".into(), num(stored_version));
    }
    Value::Object(merged)
}

/// The claim metadata (claimedAt, rewardCredit) belongs to one drafted instance: kept while the instance is the same,
/// dropped when a new draft replaces it. `claimedAt` is set the first time the backend sees the instance claimed.
fn with_claim_metadata(group: BountyGroup, bounty: Value, previous: Option<&StoredBounty>, now: &str) -> StoredBounty {
    let same = previous.filter(|p| same_instance(&p.bounty, &bounty));
    let mut next = StoredBounty {
        group,
        bounty,
        claimed_at: None,
        reward_credit: None,
    };
    match same {
        Some(previous) => {
            if previous.claimed_at.as_deref().is_some_and(|at| !at.is_empty()) {
                next.claimed_at.clone_from(&previous.claimed_at);
            } else if is_true(&next.bounty, "claimed") && !is_true(&previous.bounty, "claimed") {
                next.claimed_at = Some(now.to_owned());
            }
            next.reward_credit.clone_from(&previous.reward_credit);
        }
        None if is_true(&next.bounty, "claimed") => next.claimed_at = Some(now.to_owned()),
        None => {}
    }
    next
}

fn upsert(mut list: Vec<StoredBounty>, incoming: &Value, group: BountyGroup, now: &str) -> Vec<StoredBounty> {
    let index = list
        .iter()
        .position(|e| js::strict_eq(e.bounty.get("bounty_id"), incoming.get("bounty_id")));
    let Some(index) = index else {
        list.push(with_claim_metadata(group, incoming.clone(), None, now));
        return list;
    };
    let existing = list[index].clone();
    let next = if same_instance(&existing.bounty, incoming) {
        merge_bounty_instance(&existing.bounty, incoming)
    } else {
        incoming.clone()
    };
    let filed = if existing.group == BountyGroup::Unassigned {
        group
    } else {
        existing.group
    };
    list[index] = with_claim_metadata(filed, next, Some(&existing), now);
    list
}

type IsKept<'a> = &'a dyn Fn(&StoredBounty) -> bool;

/// `Cap`: over the limit the oldest claimed bounties go first, then the oldest of the rest. `is_kept` (host saves)
/// ranks the entries a host must not lose after everything else.
fn cap(list: Vec<StoredBounty>, is_kept: Option<IsKept<'_>>) -> Vec<StoredBounty> {
    if list.len() <= MAX_STORED_BOUNTIES {
        return list;
    }
    let mut ranked: Vec<(usize, bool, bool, f64)> = list
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let stamp = match entry.bounty.get("drafted_timestamp") {
                None | Some(Value::Null) => String::new(),
                other => js::string(other),
            };
            let time = parse_ms_f64(&stamp);
            (
                index,
                is_kept.is_some_and(|f| f(entry)),
                is_true(&entry.bounty, "claimed"),
                if time.is_nan() { 0.0 } else { time },
            )
        })
        .collect();
    ranked.sort_by(|a, b| {
        a.1.cmp(&b.1)
            .then(b.2.cmp(&a.2))
            .then(a.3.partial_cmp(&b.3).unwrap_or(std::cmp::Ordering::Equal))
    });
    let drop: HashSet<usize> = ranked
        .iter()
        .take(list.len() - MAX_STORED_BOUNTIES)
        .map(|r| r.0)
        .collect();
    list.into_iter()
        .enumerate()
        .filter(|(index, _)| !drop.contains(index))
        .map(|(_, entry)| entry)
        .collect()
}

fn group_of(entry: &StoredBounty) -> BountyGroup {
    if entry.group == BountyGroup::Unassigned {
        infer_bounty_group(bounty_id(&entry.bounty))
    } else {
        entry.group
    }
}

// ---- POST /bounty/delete/:userId

/// `BountyDeleteNamesNothing`: no body, or none of the fields `parse_bounty_delete` reads. The game's ResetBounties sends
/// no body and is answered 200 with no change.
pub fn bounty_delete_names_nothing(body: Option<&Value>) -> bool {
    match body {
        None | Some(Value::Null) => true,
        Some(Value::Object(map)) => {
            ["bounty_ids", "bountyIds", "bounties"]
                .iter()
                .all(|k| map.get(*k).is_none())
                && map.get("delete_claimed_bounties").is_none()
        }
        Some(_) => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BountyDeleteRequest {
    pub bounty_ids: Vec<String>,
    pub delete_claimed: bool,
}

/// `ParseBountyDelete`: `bounty_ids`, `bountyIds` and `bounties` (ids or bounty objects) are all read.
pub fn parse_bounty_delete(body: Option<&Value>) -> Result<BountyDeleteRequest, BountySaveValidationError> {
    let Some(Value::Object(body)) = body else {
        return Err(invalid("body must be an object"));
    };
    let mut ids: Vec<String> = Vec::new();
    for key in ["bounty_ids", "bountyIds", "bounties"] {
        let Some(list) = body.get(key) else { continue };
        let Some(list) = list.as_array() else {
            return Err(invalid(format!("{key} must be an array")));
        };
        if list.len() > MAX_BOUNTIES_PER_SAVE {
            return Err(invalid(format!("too many bounties ({})", list.len())));
        }
        for item in list {
            let id = match item {
                Value::String(id) => Some(id.as_str()),
                Value::Object(map) => map.get("bounty_id").and_then(Value::as_str),
                _ => None,
            };
            match id {
                Some(id) if !id.is_empty() && js_len(id) <= MAX_BOUNTY_ID_LENGTH => {
                    if !ids.iter().any(|known| known == id) {
                        ids.push(id.to_owned());
                    }
                }
                _ => return Err(invalid(format!("every entry of {key} needs a bounty id string"))),
            }
        }
    }
    let delete_claimed = body.get("delete_claimed_bounties") == Some(&Value::Bool(true));
    if ids.is_empty() && !delete_claimed {
        return Err(invalid("no bounty to delete"));
    }
    Ok(BountyDeleteRequest {
        bounty_ids: ids,
        delete_claimed,
    })
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BountyDeleteSummary {
    pub deleted: Vec<String>,
    /// Named ids that were not deleted: season challenges, or ids that are not stored
    pub kept: Vec<String>,
}

/// `DeleteBounties`: season challenges are never deleted.
pub fn delete_bounties(
    account: &AccountId,
    previous: Option<&BountyStateRecord>,
    body: Option<&Value>,
    now: &str,
) -> Result<(BountyStateRecord, BountyDeleteSummary), BountySaveValidationError> {
    let request = parse_bounty_delete(body)?;
    let named: HashSet<&str> = request.bounty_ids.iter().map(String::as_str).collect();
    let mut deleted: Vec<String> = Vec::new();
    let list: Vec<StoredBounty> = previous
        .map(|p| p.bounties.clone())
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| {
            let id = js::string(entry.bounty.get("bounty_id"));
            if group_of(entry) == BountyGroup::Weekly {
                return true;
            }
            if named.contains(id.as_str()) || (request.delete_claimed && is_true(&entry.bounty, "claimed")) {
                deleted.push(id);
                return false;
            }
            true
        })
        .collect();
    let kept = request
        .bounty_ids
        .iter()
        .filter(|id| !deleted.contains(id))
        .cloned()
        .collect();
    let record = BountyStateRecord {
        account: account.clone(),
        bounties: list,
        draft_data: previous.map(|p| p.draft_data.clone()).unwrap_or_default(),
        // Node's delete record doesn't carry the account's reward days over, unlike a save's (kept as it is)
        daily_reward_credited_on: None,
        daily_premium_draft_granted_on: None,
        update_version: previous.map_or(0, |p| p.update_version) + 1,
        created_at: previous.map_or_else(|| now.to_owned(), |p| p.created_at.clone()),
        updated_at: now.to_owned(),
    };
    Ok((record, BountyDeleteSummary { deleted, kept }))
}

// ---- POST /bounty/:userId

/// Several bounty tokens can be active at once (one board slot each): drafting one saves only the new bounty, so the
/// base group is upserted. The daily group holds one bounty and the weekly one the season's full list.
pub fn replaces_group_on_save(group: BountyGroup) -> bool {
    group != BountyGroup::Base
}

#[derive(Debug, Clone, PartialEq)]
pub struct BountyMergeSummary {
    /// "replace" or "upsert"
    pub mode: &'static str,
    pub group: Option<BountyGroup>,
    pub incoming: usize,
    pub removed: usize,
    pub host: Option<HostBountyAdjustments>,
}

/// What a host save carries into the shared merge: the plan's sanitised bounties are merged, the original ids still
/// decide what a replace-mode save names (so an ignored entry stays as stored).
struct HostMerge {
    now_ms: i64,
    sent_ids: HashSet<String>,
    sent_count: usize,
    adjustments: HostBountyAdjustments,
}

fn set_draft(draft: &mut DraftData, group: BountyGroup, value: Value) {
    match group {
        BountyGroup::Base => draft.base = Some(value),
        BountyGroup::Daily => draft.daily = Some(value),
        BountyGroup::Weekly => draft.weekly = Some(value),
        BountyGroup::Unassigned => {}
    }
}

fn apply_bounty_save(
    account: &AccountId,
    previous: Option<&BountyStateRecord>,
    body: &Value,
    bounties: &[Value],
    draft_keys: &[DraftKey],
    now: &str,
    mut host: Option<HostMerge>,
) -> (BountyStateRecord, BountyMergeSummary) {
    let mut list: Vec<StoredBounty> = previous.map(|p| p.bounties.clone()).unwrap_or_default();
    let mut draft: DraftData = previous.map(|p| p.draft_data.clone()).unwrap_or_default();
    // A dedicated save names what it carries; a host save names what the host sent (the plan may have ignored some)
    let sent_ids: HashSet<String> = host.as_ref().map_or_else(
        || bounties.iter().map(|b| js::string(b.get("bounty_id"))).collect(),
        |h| h.sent_ids.clone(),
    );
    let sent_count = host.as_ref().map_or(bounties.len(), |h| h.sent_count);
    let mut summary = BountyMergeSummary {
        mode: "upsert",
        group: None,
        incoming: sent_count,
        removed: 0,
        host: None,
    };
    if let [(key, group)] = draft_keys {
        set_draft(&mut draft, *group, body[*key].clone());
        // Drafting a bounty token saves only the new bounty with its draft data, so the base group is never replaced
        if sent_count > 0 && replaces_group_on_save(*group) {
            let before = list.len();
            list.retain(|entry| {
                if group_of(entry) != *group || sent_ids.contains(&js::string(entry.bounty.get("bounty_id"))) {
                    return true;
                }
                if let Some(host) = host.as_mut()
                    && is_protected_claimed_weekly(entry, host.now_ms)
                {
                    // A claimed weekly instance the host's list leaves out stays: dropping it would let a later save
                    // add it back unclaimed
                    host.adjustments.kept_claimed_weekly += 1;
                    note_adjusted_id(&mut host.adjustments.kept_claimed_weekly_ids, bounty_id(&entry.bounty));
                    return true;
                }
                false
            });
            summary.removed = before - list.len();
        }
        for bounty in bounties {
            // A host's entry is filed by its id: a Challenge_Daily_* id sent under the weekly key must not become a
            // fundable weekly claim
            let filed = if host.is_some() {
                infer_bounty_group(bounty_id(bounty))
            } else {
                *group
            };
            list = upsert(list, bounty, filed, now);
        }
        summary.mode = if replaces_group_on_save(*group) {
            "replace"
        } else {
            "upsert"
        };
        summary.group = Some(*group);
    } else {
        for (key, group) in draft_keys {
            set_draft(&mut draft, *group, body[*key].clone());
        }
        for bounty in bounties {
            list = upsert(list, bounty, infer_bounty_group(bounty_id(bounty)), now);
        }
    }
    let host_now = host.as_ref().map(|h| h.now_ms);
    summary.host = host.map(|h| h.adjustments);
    let protected = host_now.map(|ms| move |entry: &StoredBounty| is_protected_claimed_weekly(entry, ms));
    let record = BountyStateRecord {
        account: account.clone(),
        bounties: cap(list, protected.as_ref().map(|f| f as IsKept<'_>)),
        draft_data: draft,
        // The once-a-day daily reward key and the host's premium draft day belong to the account: every save keeps them
        daily_reward_credited_on: previous.and_then(|p| p.daily_reward_credited_on.clone()),
        daily_premium_draft_granted_on: previous.and_then(|p| p.daily_premium_draft_granted_on.clone()),
        update_version: previous.map_or(0, |p| p.update_version) + 1,
        created_at: previous.map_or_else(|| now.to_owned(), |p| p.created_at.clone()),
        updated_at: now.to_owned(),
    };
    (record, summary)
}

/// `MergeBountyState`.
pub fn merge_bounty_state(
    account: &AccountId,
    previous: Option<&BountyStateRecord>,
    body: Option<&Value>,
    now: &str,
) -> Result<(BountyStateRecord, BountyMergeSummary), BountySaveValidationError> {
    let (bounties, draft_keys) = validate_bounty_save(body)?;
    let body = body.cloned().unwrap_or(Value::Null);
    let bounties: Vec<Value> = bounties.iter().map(to_js).collect();
    Ok(apply_bounty_save(
        account,
        previous,
        &body,
        &bounties,
        &draft_keys,
        now,
        None,
    ))
}

/// `BountyStatePayload`.
pub fn bounty_state_payload(record: Option<&BountyStateRecord>) -> Value {
    let draft = |pick: fn(&DraftData) -> &Option<Value>| {
        record
            .and_then(|r| pick(&r.draft_data).clone())
            .unwrap_or_else(default_draft_data)
    };
    let bounties: Vec<Value> = record
        .map(|r| r.bounties.iter().map(|e| e.bounty.clone()).collect())
        .unwrap_or_default();
    json!({
        "season_start_date": BOUNTY_SEASON_START_DATE,
        "season_end_date": BOUNTY_SEASON_END_DATE,
        "bounties": bounties,
        "draft_data": draft(|d| &d.base),
        "draft_data_daily": draft(|d| &d.daily),
        "draft_data_weekly": draft(|d| &d.weekly),
    })
}

const MAX_RECORDED_BOUNTY_IDS: usize = 24;

/// `NewlyClaimedBountyIds`: claimed in this very save, whose `now` is also the record's `updatedAt`.
pub fn newly_claimed_bounty_ids(record: &BountyStateRecord) -> Vec<String> {
    record
        .bounties
        .iter()
        .filter(|e| is_true(&e.bounty, "claimed") && e.claimed_at.as_deref() == Some(record.updated_at.as_str()))
        .map(|e| js::string(e.bounty.get("bounty_id")))
        .collect()
}

/// `HostBountyWriteSummary`: what an accepted host save tags for behaviour analysis.
pub fn host_bounty_write_summary(summary: &BountyMergeSummary, record: &BountyStateRecord) -> Value {
    let claimed = newly_claimed_bounty_ids(record);
    let mut out = Map::new();
    out.insert("mode".into(), summary.mode.into());
    if let Some(group) = summary.group {
        out.insert("group".into(), group.as_str().into());
    }
    out.insert("incoming".into(), summary.incoming.into());
    out.insert("removed".into(), summary.removed.into());
    out.insert("newlyClaimedCount".into(), claimed.len().into());
    out.insert(
        "newlyClaimed".into(),
        claimed
            .iter()
            .take(MAX_RECORDED_BOUNTY_IDS)
            .cloned()
            .collect::<Vec<_>>()
            .into(),
    );
    if let Some(host) = &summary.host {
        out.insert("keptClaimedWeekly".into(), host.kept_claimed_weekly.into());
        out.insert("ignoredWeekly".into(), host.ignored_weekly.into());
        out.insert("clampedProgress".into(), host.clamped_progress.into());
        if !host.kept_claimed_weekly_ids.is_empty() {
            out.insert(
                "keptClaimedWeeklyIds".into(),
                host.kept_claimed_weekly_ids.clone().into(),
            );
        }
        if !host.ignored_weekly_ids.is_empty() {
            out.insert("ignoredWeeklyIds".into(), host.ignored_weekly_ids.clone().into());
        }
    }
    out.insert("version".into(), record.update_version.into());
    Value::Object(out)
}

/// `BountyGroupCounts`, for the `[DraftMon]` lines; `credited` counts claims that already funded their reward.
pub fn bounty_group_counts(record: Option<&BountyStateRecord>) -> Value {
    let mut counts = [0usize; 4];
    let (mut claimed, mut credited) = (0usize, 0usize);
    for entry in record.map(|r| r.bounties.as_slice()).unwrap_or_default() {
        counts[match entry.group {
            BountyGroup::Base => 0,
            BountyGroup::Daily => 1,
            BountyGroup::Weekly => 2,
            BountyGroup::Unassigned => 3,
        }] += 1;
        if is_true(&entry.bounty, "claimed") {
            claimed += 1;
        }
        if entry.reward_credit.is_some() {
            credited += 1;
        }
    }
    json!({
        "base": counts[0], "daily": counts[1], "weekly": counts[2], "unassigned": counts[3],
        "claimed": claimed, "credited": credited,
    })
}

// ---- Host saves: refuse only the impossible, proven by evidence.
//
// Weekly season challenges are a fixed schedule: 11 weeks x 10 per pass window, each stamped window start + 7 days x
// (week - 1). A claimed instance gets a new stamp only when the window rolls. A host save is held to exactly that.

pub const WEEKLY_CHALLENGE_WEEKS: i64 = 11;
pub const WEEKLY_CHALLENGES_PER_WEEK: i64 = 10;
/// The most weekly instances one pass window holds (11 weeks x 10)
pub const WEEKLY_INSTANCES_PER_WINDOW: usize = 110;
/// A server that fetched the pass window before it rolled keeps stamping the previous window's schedule for a while
pub const WEEKLY_PREVIOUS_WINDOW_GRACE_MS: i64 = 2 * 60 * 60 * 1000;
/// Clamp, not refusal: refusing the entry would also drop its claim and the other objectives riding on it
pub const MAX_HOST_OBJECTIVE_PROGRESS: f64 = 100_000_000.0;
const MAX_RECORDED_ADJUSTED_IDS: usize = 10;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostBountyAdjustments {
    /// Claimed weekly instances that stayed as stored: a re-stamp or a dropped entry a host tried
    pub kept_claimed_weekly: usize,
    /// Weekly entries not stored: bad id, off-schedule stamp, a stamp older than the stored one, or a full window
    pub ignored_weekly: usize,
    /// Objectives whose progress was clamped to `MAX_HOST_OBJECTIVE_PROGRESS`
    pub clamped_progress: usize,
    pub kept_claimed_weekly_ids: Vec<String>,
    pub ignored_weekly_ids: Vec<String>,
}

fn note_adjusted_id(list: &mut Vec<String>, id: &str) {
    if list.len() < MAX_RECORDED_ADJUSTED_IDS {
        list.push(id.to_owned());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeeklyBountyId {
    pub season: i64,
    pub week: i64,
    pub slot: i64,
}

/// `ParseWeeklyBountyId`: `<season>_<week>_<n>_Challenge_Season_<name>-season<season>-<week-1>`, week 1..11, n 1..10, the
/// trailing index equal to week - 1, numbers written canonically.
pub fn parse_weekly_bounty_id(id: &str) -> Option<WeeklyBountyId> {
    let (season, rest) = id.split_once('_')?;
    let (week, rest) = rest.split_once('_')?;
    let (slot, rest) = rest.split_once('_')?;
    let rest = rest.strip_prefix("Challenge_Season_")?;
    let (name_and_season, index) = rest.rsplit_once('-')?;
    let (name, season_again) = name_and_season.rsplit_once("-season")?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(season) || !digits(week) || !digits(slot) || !digits(season_again) || !digits(index) {
        return None;
    }
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return None;
    }
    let week_n: i64 = week.parse().ok()?;
    let slot_n: i64 = slot.parse().ok()?;
    if !(1..=WEEKLY_CHALLENGE_WEEKS).contains(&week_n) || week_n.to_string() != week {
        return None;
    }
    if !(1..=WEEKLY_CHALLENGES_PER_WEEK).contains(&slot_n) || slot_n.to_string() != slot {
        return None;
    }
    if index != (week_n - 1).to_string() {
        return None;
    }
    Some(WeeklyBountyId {
        season: season.parse().ok()?,
        week: week_n,
        slot: slot_n,
    })
}

pub fn weekly_schedule_stamp_ms(window_start_ms: i64, week: i64) -> i64 {
    window_start_ms + (week - 1) * 7 * DAY_MS
}

/// The stamps a host may give week `week` at `now_ms`: the current window's schedule, and the previous window's for
/// `WEEKLY_PREVIOUS_WINDOW_GRACE_MS` after a roll.
pub fn accepted_weekly_stamps_ms(week: i64, now_ms: i64) -> Vec<i64> {
    let (start, _) = active_season_window_ms(now_ms);
    let mut stamps = vec![weekly_schedule_stamp_ms(start, week)];
    if now_ms - start < WEEKLY_PREVIOUS_WINDOW_GRACE_MS {
        stamps.push(weekly_schedule_stamp_ms(start - ACTIVE_SEASON_LENGTH_MS, week));
    }
    stamps
}

fn bounty_stamp_ms(bounty: &Value) -> f64 {
    match bounty.get("drafted_timestamp") {
        Some(Value::String(text)) => parse_ms_f64(text),
        _ => f64::NAN,
    }
}

/// `IsProtectedClaimedWeekly`: a stored, claimed weekly instance whose stamp is one a host may still send. Dropping it
/// would let a host add it back unclaimed and claim it again.
pub fn is_protected_claimed_weekly(entry: &StoredBounty, now_ms: i64) -> bool {
    if !is_true(&entry.bounty, "claimed") {
        return false;
    }
    let id = js::string(entry.bounty.get("bounty_id"));
    if infer_bounty_group(&id) != BountyGroup::Weekly {
        return false;
    }
    let Some(parsed) = parse_weekly_bounty_id(&id) else {
        return false;
    };
    let stamp = bounty_stamp_ms(&entry.bounty);
    accepted_weekly_stamps_ms(parsed.week, now_ms)
        .iter()
        .any(|accepted| *accepted as f64 == stamp)
}

/// `ClampHostObjectiveProgress`: progress above the maximum is clamped, read the way the merge reads it.
pub fn clamp_host_objective_progress(bounty: &Value) -> (Value, usize) {
    let Some(objectives) = bounty.get("objectives").and_then(Value::as_array) else {
        return (bounty.clone(), 0);
    };
    let mut clamped = 0;
    let objectives: Vec<Value> = objectives
        .iter()
        .map(|objective| {
            if !is_plain_object(Some(objective))
                || js::number(objective.get("progress")).partial_cmp(&MAX_HOST_OBJECTIVE_PROGRESS)
                    != Some(std::cmp::Ordering::Greater)
            {
                return objective.clone();
            }
            clamped += 1;
            let mut copy = objective.clone();
            if let Some(map) = copy.as_object_mut() {
                map.insert("progress".into(), num(MAX_HOST_OBJECTIVE_PROGRESS));
            }
            copy
        })
        .collect();
    if clamped == 0 {
        return (bounty.clone(), 0);
    }
    let mut copy = bounty.clone();
    if let Some(map) = copy.as_object_mut() {
        map.insert("objectives".into(), Value::Array(objectives));
    }
    (copy, clamped)
}

struct WeeklyState {
    stamp_ms: f64,
    stamp: Option<Value>,
    claimed: bool,
}

fn place(windows: &mut HashMap<i64, HashSet<String>>, id: &str, stamp_ms: f64) {
    for ids in windows.values_mut() {
        ids.remove(id);
    }
    if !stamp_ms.is_finite() {
        return;
    }
    let key = active_season_window_ms(stamp_ms as i64).0;
    windows.entry(key).or_default().insert(id.to_owned());
}

/// `PlanHostBountySave`: the sanitised bounty list a host's save is merged from, and what was changed. Per weekly entry
/// (an id with Challenge_Season), judged in order against the stored instances as earlier entries of the same save
/// left them:
/// - the id must be a `parse_weekly_bounty_id` id and the stamp an `accepted_weekly_stamps_ms` stamp, else not stored;
/// - an entry for an instance stored with the same stamp (compared as instants) is the same instance and merges as always;
/// - a different stamp is a replacement, allowed only forward (the window rolled), never back;
/// - a new id, or a move into another window, is refused once the window already holds 110.
pub fn plan_host_bounty_save(
    previous: Option<&BountyStateRecord>,
    bounties: &[Value],
    now_ms: i64,
) -> (Vec<Value>, HostBountyAdjustments) {
    let mut adjustments = HostBountyAdjustments::default();
    let mut weekly: HashMap<String, WeeklyState> = HashMap::new();
    // window start -> the weekly ids whose stamp lies in it
    let mut windows: HashMap<i64, HashSet<String>> = HashMap::new();
    for entry in previous.map(|p| p.bounties.as_slice()).unwrap_or_default() {
        let id = js::string(entry.bounty.get("bounty_id"));
        // The merge finds the first entry of an id too
        if infer_bounty_group(&id) != BountyGroup::Weekly || weekly.contains_key(&id) {
            continue;
        }
        let stamp_ms = bounty_stamp_ms(&entry.bounty);
        weekly.insert(
            id.clone(),
            WeeklyState {
                stamp_ms,
                stamp: entry.bounty.get("drafted_timestamp").cloned(),
                claimed: is_true(&entry.bounty, "claimed"),
            },
        );
        place(&mut windows, &id, stamp_ms);
    }

    let mut planned: Vec<Value> = Vec::new();
    for sent in bounties {
        let id = js::string(sent.get("bounty_id"));
        let (bounty, clamped) = clamp_host_objective_progress(sent);
        adjustments.clamped_progress += clamped;
        if infer_bounty_group(&id) != BountyGroup::Weekly {
            planned.push(bounty);
            continue;
        }
        let stored_claimed = weekly.get(&id).map(|s| s.claimed);
        let refuse = |adjustments: &mut HostBountyAdjustments| {
            if stored_claimed == Some(true) {
                adjustments.kept_claimed_weekly += 1;
                note_adjusted_id(&mut adjustments.kept_claimed_weekly_ids, &id);
            } else {
                adjustments.ignored_weekly += 1;
                note_adjusted_id(&mut adjustments.ignored_weekly_ids, &id);
            }
        };
        let stamp_ms = bounty_stamp_ms(&bounty);
        let accepted = parse_weekly_bounty_id(&id).is_some_and(|p| {
            accepted_weekly_stamps_ms(p.week, now_ms)
                .iter()
                .any(|a| *a as f64 == stamp_ms)
        });
        if !accepted {
            refuse(&mut adjustments);
            continue;
        }
        if let Some(stored) = weekly.get_mut(&id) {
            if stored.stamp_ms == stamp_ms {
                // The same instance; a different notation of the same instant must not look like a re-stamp
                let mut kept = bounty.clone();
                if !js::strict_eq(stored.stamp.as_ref(), bounty.get("drafted_timestamp"))
                    && let (Some(map), Some(stamp)) = (kept.as_object_mut(), stored.stamp.clone())
                {
                    map.insert("drafted_timestamp".into(), stamp);
                }
                planned.push(kept);
                stored.claimed = stored.claimed || is_true(&bounty, "claimed");
                continue;
            }
            if stored.stamp_ms.is_finite() && stamp_ms < stored.stamp_ms {
                refuse(&mut adjustments);
                continue;
            }
        }
        let in_window = windows.get(&active_season_window_ms(stamp_ms as i64).0);
        let present = in_window.is_some_and(|ids| ids.contains(&id));
        if !present && in_window.map_or(0, HashSet::len) >= WEEKLY_INSTANCES_PER_WINDOW {
            refuse(&mut adjustments);
            continue;
        }
        place(&mut windows, &id, stamp_ms);
        weekly.insert(
            id.clone(),
            WeeklyState {
                stamp_ms,
                stamp: bounty.get("drafted_timestamp").cloned(),
                claimed: is_true(&bounty, "claimed"),
            },
        );
        planned.push(bounty);
    }
    (planned, adjustments)
}

/// `MergeHostBountyState`: `plan_host_bounty_save` first, then the shared merge with the claimed weekly instances
/// protected from omission and from the list cap.
pub fn merge_host_bounty_state(
    account: &AccountId,
    previous: Option<&BountyStateRecord>,
    body: Option<&Value>,
    now: &str,
) -> Result<(BountyStateRecord, BountyMergeSummary), BountySaveValidationError> {
    let (bounties, draft_keys) = validate_bounty_save(body)?;
    let now_ms = parse_ms(now).unwrap_or_else(super::util::now_ms);
    let bounties: Vec<Value> = bounties.iter().map(to_js).collect();
    let (planned, adjustments) = plan_host_bounty_save(previous, &bounties, now_ms);
    let host = HostMerge {
        now_ms,
        sent_ids: bounties.iter().map(|b| js::string(b.get("bounty_id"))).collect(),
        sent_count: bounties.len(),
        adjustments,
    };
    let body = body.cloned().unwrap_or(Value::Null);
    Ok(apply_bounty_save(
        account,
        previous,
        &body,
        &planned,
        &draft_keys,
        now,
        Some(host),
    ))
}

// ---- Season-challenge rewards. Completing a season challenge makes the game server claim it and post +N coins; the
// wallet funds such a grant from exactly one claimed, not-yet-credited challenge and records the credit on that
// instance in the same transaction, so each completed challenge pays once. A daily claim funds at most one reward per
// game day (from 17:00 UTC), recorded on the account.

pub const MAX_CHALLENGE_REWARD_PER_CLAIM: f64 = 1000.0;
pub const CHALLENGE_REWARD_CURRENCIES: [&str; 1] = ["CURRENCY_SEASONAL_COIN"];

/// The UTC day of an instant, as YYYY-MM-DD.
pub fn utc_day(ms: i64) -> String {
    iso(ms)[..10].to_owned()
}

const GAME_DAY_OFFSET_MS: i64 = 17 * 60 * 60 * 1000;

/// `GameDailyWindowKey`: the game day's 17:00 UTC start as an ISO string.
pub fn game_daily_window_key(ms: i64) -> String {
    iso((ms - GAME_DAY_OFFSET_MS).div_euclid(DAY_MS) * DAY_MS + GAME_DAY_OFFSET_MS)
}

pub fn daily_challenge_reward_paid(record: Option<&BountyStateRecord>, now_ms: i64) -> bool {
    record.and_then(|r| r.daily_reward_credited_on.as_deref()) == Some(game_daily_window_key(now_ms).as_str())
}

/// True when a claimed daily challenge waits for its reward but this game day's daily reward was already paid: the one
/// case in which the wallet skips the coins instead of refusing the whole transaction.
pub fn daily_challenge_reward_limited(record: Option<&BountyStateRecord>, now_ms: i64) -> bool {
    if !daily_challenge_reward_paid(record, now_ms) {
        return false;
    }
    record.is_some_and(|r| {
        r.bounties
            .iter()
            .any(|e| group_of(e) == BountyGroup::Daily && is_true(&e.bounty, "claimed") && e.reward_credit.is_none())
    })
}

/// `FindFundableSeasonClaim`: the most recently claimed, uncredited weekly or daily challenge (claims stored before
/// `claimedAt` existed count as the oldest); a daily one only while no daily reward was paid this game day.
pub fn find_fundable_season_claim(record: Option<&BountyStateRecord>, now_ms: i64) -> Option<&StoredBounty> {
    let daily_allowed = !daily_challenge_reward_paid(record, now_ms);
    let mut best: Option<&StoredBounty> = None;
    for entry in record.map(|r| r.bounties.as_slice()).unwrap_or_default() {
        let group = group_of(entry);
        if !is_true(&entry.bounty, "claimed") || entry.reward_credit.is_some() {
            continue;
        }
        if group != BountyGroup::Weekly && !(group == BountyGroup::Daily && daily_allowed) {
            continue;
        }
        if best.is_none_or(|b| entry.claimed_at.as_deref().unwrap_or("") > b.claimed_at.as_deref().unwrap_or("")) {
            best = Some(entry);
        }
    }
    best
}

/// `WithRewardCredit`: a copy of the record with the credit recorded on `bounty_id` (the caller saves it with a
/// version check).
pub fn with_reward_credit(
    record: &BountyStateRecord,
    bounty_id: &str,
    credit: RewardCredit,
    now: &str,
) -> Result<BountyStateRecord, String> {
    let mut found = false;
    let mut daily = false;
    let mut bounties = Vec::with_capacity(record.bounties.len());
    for entry in &record.bounties {
        if entry.bounty.get("bounty_id").and_then(Value::as_str) != Some(bounty_id) {
            bounties.push(entry.clone());
            continue;
        }
        if !is_true(&entry.bounty, "claimed") || entry.reward_credit.is_some() {
            return Err(format!("bounty {bounty_id} is not a fundable claim"));
        }
        found = true;
        daily = group_of(entry) == BountyGroup::Daily;
        bounties.push(StoredBounty {
            reward_credit: Some(credit.clone()),
            ..entry.clone()
        });
    }
    if !found {
        return Err(format!("bounty {bounty_id} is not stored"));
    }
    let mut next = record.clone();
    next.bounties = bounties;
    if daily {
        next.daily_reward_credited_on = Some(game_daily_window_key(parse_ms(&credit.at).unwrap_or_default()));
    }
    next.update_version += 1;
    next.updated_at = now.to_owned();
    Ok(next)
}
