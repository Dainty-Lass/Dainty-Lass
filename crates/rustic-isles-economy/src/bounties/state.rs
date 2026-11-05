// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Bounty state (bounties spec §4.2 to §4.4): validation, the dedicated and host merges, deletes and the payload.
//! Pure: the routes read and save the record around these.

use std::collections::{HashMap, HashSet};

use rustic_isles::season;
use rustic_isles_store::AccountId;
use rustic_isles_store::repo::bounties::{BountyGroup, BountyStateRecord, DraftData, StoredBounty};
use serde_json::{Map, Value};
use service_kit::js;

use crate::util::{num0, number_value, obj, utf16_len};

pub const MAX_SAVED_BOUNTIES: usize = 512;
pub const MAX_STORED_BOUNTIES: usize = 1000;
pub const MAX_WEEKLY_PER_WINDOW: usize = 110;
pub const MAX_HOST_PROGRESS: f64 = 100_000_000.0;
const WEEK_MS: i64 = 7 * 86_400_000;
const ROLL_GRACE_MS: i64 = 2 * 3_600_000;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct BountySaveError(pub String);

fn fail<T>(text: impl Into<String>) -> Result<T, BountySaveError> {
    Err(BountySaveError(text.into()))
}

/// `Challenge_Daily_*` is daily, `*Challenge_Season*` weekly, anything else base.
pub fn infer_group(id: &str) -> BountyGroup {
    if id.starts_with("Challenge_Daily_") {
        BountyGroup::Daily
    } else if id.contains("Challenge_Season") {
        BountyGroup::Weekly
    } else {
        BountyGroup::Base
    }
}

pub fn bounty_id(entry: &StoredBounty) -> Option<&str> {
    entry.bounty.get("bounty_id").and_then(Value::as_str)
}

/// The stored group, or the inferred one for an imported `unassigned` row.
pub fn effective_group(entry: &StoredBounty) -> BountyGroup {
    match entry.group {
        BountyGroup::Unassigned => infer_group(bounty_id(entry).unwrap_or("")),
        group => group,
    }
}

pub fn is_claimed(entry: &StoredBounty) -> bool {
    claimed(&entry.bounty)
}

fn claimed(bounty: &Value) -> bool {
    bounty.get("claimed") == Some(&Value::Bool(true))
}

const DRAFT_KEYS: [(&str, BountyGroup); 3] = [
    ("draft_data", BountyGroup::Base),
    ("draft_data_daily", BountyGroup::Daily),
    ("draft_data_weekly", BountyGroup::Weekly),
];

/// A save that passed [`validate_bounty_save`].
#[derive(Debug, Clone, PartialEq)]
pub struct BountySave {
    pub bounties: Vec<Value>,
    /// The draft keys present, in key order
    pub drafts: Vec<(BountyGroup, Value)>,
}

/// `ValidateBountySave` (§4.2.2).
pub fn validate_bounty_save(body: Option<&Value>) -> Result<BountySave, BountySaveError> {
    let Some(Value::Object(body)) = body else {
        return fail("body must be an object");
    };
    let bounties = match body.get("bounties") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(list)) => list.clone(),
        Some(_) => return fail("bounties must be an array"),
    };
    if bounties.len() > MAX_SAVED_BOUNTIES {
        return fail(format!("too many bounties ({})", bounties.len()));
    }
    for bounty in &bounties {
        let Some(map) = bounty.as_object() else {
            return fail("every bounty must be an object");
        };
        let Some(id) = map
            .get("bounty_id")
            .and_then(Value::as_str)
            .filter(|id| (1..=200).contains(&utf16_len(id)))
        else {
            return fail("every bounty needs a bounty_id string");
        };
        if map.contains_key("objectives") && !map.get("objectives").is_some_and(Value::is_array) {
            return fail(format!("objectives of {id} must be an array"));
        }
    }
    let mut drafts = Vec::new();
    for (key, group) in DRAFT_KEYS {
        if let Some(value) = body.get(key) {
            if !value.is_object() {
                return fail(format!("{key} must be an object"));
            }
            drafts.push((group, value.clone()));
        }
    }
    Ok(BountySave { bounties, drafts })
}

fn same_stamp(a: &Value, b: &Value) -> bool {
    let norm = |v: &Value| v.get("drafted_timestamp").cloned().unwrap_or(Value::Null);
    js::strict_eq(Some(&norm(a)), Some(&norm(b)))
}

/// Sets `key` in place when present, else appends it.
fn set(map: &mut Map<String, Value>, key: &str, value: Value) {
    map.insert(key.to_owned(), value);
}

/// `MergeBountyInstance`: the incoming copy, never losing progress, a claim or a newer version.
pub fn merge_bounty_instance(stored: &Value, incoming: &Value) -> Value {
    let Some(mut out) = incoming.as_object().cloned() else {
        return incoming.clone();
    };
    let stored_objectives = stored.get("objectives").and_then(Value::as_array);
    let incoming_objectives = incoming.get("objectives").and_then(Value::as_array);
    if stored_objectives.is_some() || incoming_objectives.is_some() {
        let stored_list = stored_objectives.map_or(&[][..], Vec::as_slice);
        let id_of = |o: &Value| o.get("objective_id").and_then(Value::as_str).map(str::to_owned);
        let mut merged = Vec::new();
        let mut seen = HashSet::new();
        for objective in incoming_objectives.map_or(&[][..], Vec::as_slice) {
            let mut objective = objective.clone();
            if let Some(id) = id_of(&objective) {
                seen.insert(id.clone());
                let better = stored_list.iter().find(|s| id_of(s).as_deref() == Some(id.as_str()));
                if let Some(stored_one) = better {
                    let kept = num0(stored_one.get("progress"));
                    if kept > num0(objective.get("progress"))
                        && let Some(map) = objective.as_object_mut()
                    {
                        set(map, "progress", number_value(kept));
                    }
                }
            }
            merged.push(objective);
        }
        for stored_one in stored_list {
            if let Some(id) = id_of(stored_one)
                && seen.insert(id)
            {
                merged.push(stored_one.clone());
            }
        }
        set(&mut out, "objectives", Value::Array(merged));
    }
    if claimed(stored) {
        set(&mut out, "claimed", Value::Bool(true));
    }
    let stored_version = stored
        .get("update_version")
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite());
    if let Some(stored_version) = stored_version {
        let incoming_version = incoming
            .get("update_version")
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite());
        if incoming_version.is_none_or(|v| stored_version > v)
            && let Some(original) = stored.get("update_version")
        {
            set(&mut out, "update_version", original.clone());
        }
    }
    Value::Object(out)
}

/// One bounty upserted into `list`, filed under `filing` when it is new.
fn upsert(list: &mut Vec<StoredBounty>, bounty: Value, filing: BountyGroup, now: &str) {
    let id = bounty
        .get("bounty_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let incoming_claimed = claimed(&bounty);
    let Some(at) = list.iter().position(|e| bounty_id(e) == Some(id.as_str())) else {
        list.push(StoredBounty {
            group: filing,
            bounty,
            claimed_at: incoming_claimed.then(|| now.to_owned()),
            reward_credit: None,
        });
        return;
    };
    let stored = &list[at];
    let same = same_stamp(&stored.bounty, &bounty);
    let group = if stored.group == BountyGroup::Unassigned {
        filing
    } else {
        stored.group
    };
    let next = if same {
        let claimed_at = match stored.claimed_at.as_deref().filter(|c| !c.is_empty()) {
            Some(kept) => Some(kept.to_owned()),
            None if incoming_claimed && !claimed(&stored.bounty) => Some(now.to_owned()),
            None => stored.claimed_at.clone(),
        };
        StoredBounty {
            group,
            bounty: merge_bounty_instance(&stored.bounty, &bounty),
            claimed_at,
            reward_credit: stored.reward_credit.clone(),
        }
    } else {
        StoredBounty {
            group,
            bounty,
            claimed_at: incoming_claimed.then(|| now.to_owned()),
            reward_credit: None,
        }
    };
    list[at] = next;
}

fn stamp_ms(bounty: &Value) -> i64 {
    match bounty.get("drafted_timestamp") {
        None | Some(Value::Null) => 0,
        other => js::date_parse(&js::string(other)).unwrap_or(0),
    }
}

/// At most 1000: protected last, claimed before unclaimed, oldest stamp first, ties in list order; the first
/// (count - 1000) go and the rest keep their order.
fn cap(list: Vec<StoredBounty>, protected: &dyn Fn(&StoredBounty) -> bool) -> Vec<StoredBounty> {
    if list.len() <= MAX_STORED_BOUNTIES {
        return list;
    }
    let mut ranked: Vec<usize> = (0..list.len()).collect();
    ranked.sort_by_key(|&i| (protected(&list[i]), !is_claimed(&list[i]), stamp_ms(&list[i].bounty), i));
    let drop: HashSet<usize> = ranked[..list.len() - MAX_STORED_BOUNTIES].iter().copied().collect();
    list.into_iter()
        .enumerate()
        .filter(|(i, _)| !drop.contains(i))
        .map(|(_, e)| e)
        .collect()
}

fn set_draft(drafts: &mut DraftData, group: BountyGroup, value: Value) {
    match group {
        BountyGroup::Base => drafts.base = Some(value),
        BountyGroup::Daily => drafts.daily = Some(value),
        BountyGroup::Weekly => drafts.weekly = Some(value),
        BountyGroup::Unassigned => {}
    }
}

/// What a save did, for the answer, the logs and a host's tag.
#[derive(Debug, Clone, PartialEq)]
pub struct Merged {
    pub record: BountyStateRecord,
    pub mode: &'static str,
    pub group: Option<BountyGroup>,
    pub incoming: usize,
    pub removed: usize,
    pub newly_claimed: Vec<String>,
    pub host: Option<HostChanges>,
}

/// A host save's refused entries and clamps (§4.3).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HostChanges {
    pub kept_claimed_weekly: usize,
    pub ignored_weekly: usize,
    pub clamped_progress: usize,
    pub kept_claimed_weekly_ids: Vec<String>,
    pub ignored_weekly_ids: Vec<String>,
}

fn new_record(stored: Option<&BountyStateRecord>, account: &AccountId, now: &str) -> BountyStateRecord {
    match stored {
        Some(record) => record.clone(),
        None => BountyStateRecord {
            account: account.clone(),
            bounties: Vec::new(),
            draft_data: DraftData::default(),
            daily_reward_credited_on: None,
            daily_premium_draft_granted_on: None,
            update_version: 0,
            created_at: now.to_owned(),
            updated_at: now.to_owned(),
        },
    }
}

/// `MergeBountyState` (§4.2.3) for a dedicated server.
pub fn merge_bounty_state(
    stored: Option<&BountyStateRecord>,
    account: &AccountId,
    save: &BountySave,
    now: &str,
) -> Merged {
    merge(stored, account, save, now, None)
}

/// A host's save (§4.3): judged entry by entry against the weekly schedule at `now_ms`, then merged.
pub fn merge_host_bounty_state(
    stored: Option<&BountyStateRecord>,
    account: &AccountId,
    save: &BountySave,
    now: &str,
    now_ms: i64,
) -> Merged {
    merge(stored, account, save, now, Some(now_ms))
}

fn merge(
    stored: Option<&BountyStateRecord>,
    account: &AccountId,
    save: &BountySave,
    now: &str,
    host_now: Option<i64>,
) -> Merged {
    let mut record = new_record(stored, account, now);
    let protected_ids: HashSet<String> = match host_now {
        Some(now_ms) => record
            .bounties
            .iter()
            .filter(|e| is_protected(e, now_ms))
            .filter_map(|e| bounty_id(e).map(str::to_owned))
            .collect(),
        None => HashSet::new(),
    };
    let sent_ids: Vec<String> = save
        .bounties
        .iter()
        .filter_map(|b| b.get("bounty_id").and_then(Value::as_str).map(str::to_owned))
        .collect();
    let (passing, mut host) = match host_now {
        Some(now_ms) => {
            let (passing, changes) = judge(&record.bounties, &save.bounties, now_ms);
            (passing, Some(changes))
        }
        None => (save.bounties.clone(), None),
    };
    let single = (save.drafts.len() == 1).then(|| save.drafts[0].0);
    let mut removed = 0;
    let mode;
    match single {
        Some(group) => {
            set_draft(&mut record.draft_data, group, save.drafts[0].1.clone());
            let replace = matches!(group, BountyGroup::Daily | BountyGroup::Weekly);
            mode = if replace { "replace" } else { "upsert" };
            if replace && !save.bounties.is_empty() {
                let before = record.bounties.len();
                let mut kept_protected = Vec::new();
                record.bounties.retain(|e| {
                    let id = bounty_id(e).unwrap_or_default();
                    if effective_group(e) != group || sent_ids.iter().any(|s| s == id) {
                        return true;
                    }
                    if protected_ids.contains(id) {
                        kept_protected.push(id.to_owned());
                        return true;
                    }
                    false
                });
                removed = before - record.bounties.len();
                if let Some(changes) = host.as_mut() {
                    for id in kept_protected {
                        changes.kept_claimed_weekly += 1;
                        sample(&mut changes.kept_claimed_weekly_ids, id);
                    }
                }
            }
            for bounty in passing {
                let filing = if host_now.is_some() {
                    infer_group(bounty.get("bounty_id").and_then(Value::as_str).unwrap_or(""))
                } else {
                    group
                };
                upsert(&mut record.bounties, bounty, filing, now);
            }
        }
        None => {
            mode = "upsert";
            for (group, value) in &save.drafts {
                set_draft(&mut record.draft_data, *group, value.clone());
            }
            for bounty in passing {
                let filing = infer_group(bounty.get("bounty_id").and_then(Value::as_str).unwrap_or(""));
                upsert(&mut record.bounties, bounty, filing, now);
            }
        }
    }
    let bounties = std::mem::take(&mut record.bounties);
    record.bounties = cap(bounties, &|e| bounty_id(e).is_some_and(|id| protected_ids.contains(id)));
    record.update_version = stored.map_or(1, |r| r.update_version + 1);
    record.updated_at = now.to_owned();
    let newly_claimed = newly_claimed(&record);
    Merged {
        record,
        mode,
        group: single,
        incoming: save.bounties.len(),
        removed,
        newly_claimed,
        host,
    }
}

fn sample(list: &mut Vec<String>, id: String) {
    if list.len() < 10 {
        list.push(id);
    }
}

/// The ids claimed in this very save (`claimedAt` equals the record's `updated_at`).
pub fn newly_claimed(record: &BountyStateRecord) -> Vec<String> {
    record
        .bounties
        .iter()
        .filter(|e| is_claimed(e) && e.claimed_at.as_deref() == Some(record.updated_at.as_str()))
        .filter_map(|e| bounty_id(e).map(str::to_owned))
        .collect()
}

// ------------------------------------------------------------------ weekly ids and the host plan

/// `<season>_<week>_<slot>_Challenge_Season_<name>-season<season2>-<index>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeeklyId {
    pub season: String,
    pub week: u32,
    pub slot: u32,
    pub name: String,
    pub season2: String,
}

impl WeeklyId {
    pub fn text(&self) -> String {
        format!(
            "{}_{}_{}_Challenge_Season_{}-season{}-{}",
            self.season,
            self.week,
            self.slot,
            self.name,
            self.season2,
            self.week - 1
        )
    }
}

fn canonical(text: &str, range: std::ops::RangeInclusive<u32>) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) || (text.len() > 1 && text.starts_with('0')) {
        return None;
    }
    text.parse::<u32>().ok().filter(|n| range.contains(n))
}

pub fn parse_weekly_id(id: &str) -> Option<WeeklyId> {
    let mut parts = id.splitn(4, '_');
    let (season, week, slot, rest) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    if season.is_empty() || !season.bytes().all(|b| b.is_ascii_digit()) || season.parse::<i64>().is_err() {
        return None;
    }
    let week = canonical(week, 1..=11)?;
    let slot = canonical(slot, 1..=10)?;
    let rest = rest.strip_prefix("Challenge_Season_")?;
    let (head, index) = rest.rsplit_once('-')?;
    let (name, season2) = head.rsplit_once("-season")?;
    let name_ok = !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    let season2_ok = !season2.is_empty() && season2.bytes().all(|b| b.is_ascii_digit());
    if !name_ok || !season2_ok || index != (week - 1).to_string() {
        return None;
    }
    Some(WeeklyId {
        season: season.to_owned(),
        week,
        slot,
        name: name.to_owned(),
        season2: season2.to_owned(),
    })
}

/// The stamps a week's instance may carry at `now_ms`: the current schedule, and the previous one for the first two
/// hours of a window.
pub fn accepted_stamps(week: u32, now_ms: i64) -> Vec<i64> {
    let (start, _) = season::active_season_window_ms(now_ms);
    let offset = i64::from(week - 1) * WEEK_MS;
    let mut out = vec![start + offset];
    if now_ms - start < ROLL_GRACE_MS {
        out.push(start - season::LENGTH_MS + offset);
    }
    out
}

fn stamp_of(bounty: &Value) -> Option<i64> {
    bounty
        .get("drafted_timestamp")
        .and_then(Value::as_str)
        .and_then(js::date_parse)
}

fn is_accepted(id: &WeeklyId, stamp: Option<i64>, now_ms: i64) -> bool {
    stamp.is_some_and(|s| accepted_stamps(id.week, now_ms).contains(&s))
}

/// A stored, claimed weekly instance on schedule: a host's save can neither drop nor evict it.
pub fn is_protected(entry: &StoredBounty, now_ms: i64) -> bool {
    let Some(id) = bounty_id(entry) else { return false };
    is_claimed(entry)
        && infer_group(id) == BountyGroup::Weekly
        && parse_weekly_id(id).is_some_and(|w| is_accepted(&w, stamp_of(&entry.bounty), now_ms))
}

struct Instance {
    stamp: Option<i64>,
    stamp_text: Option<Value>,
    claimed: bool,
}

/// `PlanHostBountySave`: the entries that pass, in order, and what was refused or clamped.
fn judge(stored: &[StoredBounty], sent: &[Value], now_ms: i64) -> (Vec<Value>, HostChanges) {
    let mut changes = HostChanges::default();
    let mut instances: HashMap<String, Instance> = HashMap::new();
    let mut windows: HashMap<i64, HashSet<String>> = HashMap::new();
    for entry in stored {
        let Some(id) = bounty_id(entry) else { continue };
        if infer_group(id) != BountyGroup::Weekly || instances.contains_key(id) {
            continue;
        }
        let stamp = stamp_of(&entry.bounty);
        if let Some(s) = stamp {
            windows
                .entry(season::active_season_window_ms(s).0)
                .or_default()
                .insert(id.to_owned());
        }
        instances.insert(
            id.to_owned(),
            Instance {
                stamp,
                stamp_text: entry.bounty.get("drafted_timestamp").cloned(),
                claimed: is_claimed(entry),
            },
        );
    }
    let mut passing = Vec::new();
    for bounty in sent {
        let mut bounty = bounty.clone();
        if let Some(objectives) = bounty.get_mut("objectives").and_then(Value::as_array_mut) {
            for objective in objectives.iter_mut() {
                if let Some(map) = objective.as_object_mut()
                    && crate::util::num(map.get("progress")) > MAX_HOST_PROGRESS
                {
                    map.insert("progress".into(), number_value(MAX_HOST_PROGRESS));
                    changes.clamped_progress += 1;
                }
            }
        }
        let id = bounty
            .get("bounty_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if infer_group(&id) != BountyGroup::Weekly {
            passing.push(bounty);
            continue;
        }
        let stamp = stamp_of(&bounty);
        let stored_claimed = instances.get(&id).is_some_and(|i| i.claimed);
        let refuse = |changes: &mut HostChanges| {
            if stored_claimed {
                changes.kept_claimed_weekly += 1;
                sample(&mut changes.kept_claimed_weekly_ids, id.clone());
            } else {
                changes.ignored_weekly += 1;
                sample(&mut changes.ignored_weekly_ids, id.clone());
            }
        };
        let Some(weekly) = parse_weekly_id(&id).filter(|w| is_accepted(w, stamp, now_ms)) else {
            refuse(&mut changes);
            continue;
        };
        let Some(stamp) = stamp else {
            refuse(&mut changes);
            continue;
        };
        let _ = weekly;
        if let Some(instance) = instances.get_mut(&id) {
            if instance.stamp == Some(stamp) {
                if let (Some(text), Some(map)) = (instance.stamp_text.clone(), bounty.as_object_mut()) {
                    map.insert("drafted_timestamp".into(), text);
                }
                if claimed(&bounty) {
                    instance.claimed = true;
                }
                passing.push(bounty);
                continue;
            }
            if instance.stamp.is_some_and(|s| s > stamp) {
                refuse(&mut changes);
                continue;
            }
        }
        let window = season::active_season_window_ms(stamp).0;
        let full = windows
            .get(&window)
            .is_some_and(|ids| ids.len() >= MAX_WEEKLY_PER_WINDOW && !ids.contains(&id));
        if full {
            refuse(&mut changes);
            continue;
        }
        for ids in windows.values_mut() {
            ids.remove(&id);
        }
        windows.entry(window).or_default().insert(id.clone());
        instances.insert(
            id,
            Instance {
                stamp: Some(stamp),
                stamp_text: bounty.get("drafted_timestamp").cloned(),
                claimed: claimed(&bounty),
            },
        );
        passing.push(bounty);
    }
    (passing, changes)
}

/// The host-write tag's summary (§4.3.3).
pub fn host_summary(merged: &Merged) -> Value {
    let mut out = Map::new();
    out.insert("mode".into(), Value::from(merged.mode));
    if let Some(group) = merged.group {
        out.insert("group".into(), Value::from(group.as_str()));
    }
    out.insert("incoming".into(), Value::from(merged.incoming));
    out.insert("removed".into(), Value::from(merged.removed));
    out.insert("newlyClaimedCount".into(), Value::from(merged.newly_claimed.len()));
    out.insert(
        "newlyClaimed".into(),
        Value::from(merged.newly_claimed.iter().take(24).cloned().collect::<Vec<_>>()),
    );
    let host = merged.host.clone().unwrap_or_default();
    out.insert("keptClaimedWeekly".into(), Value::from(host.kept_claimed_weekly));
    out.insert("ignoredWeekly".into(), Value::from(host.ignored_weekly));
    out.insert("clampedProgress".into(), Value::from(host.clamped_progress));
    if !host.kept_claimed_weekly_ids.is_empty() {
        out.insert("keptClaimedWeeklyIds".into(), Value::from(host.kept_claimed_weekly_ids));
    }
    if !host.ignored_weekly_ids.is_empty() {
        out.insert("ignoredWeeklyIds".into(), Value::from(host.ignored_weekly_ids));
    }
    out.insert("version".into(), Value::from(merged.record.update_version));
    Value::Object(out)
}

// ------------------------------------------------------------------ deletes

#[derive(Debug, Clone, PartialEq)]
pub struct DeleteRequest {
    pub ids: Vec<String>,
    pub delete_claimed: bool,
}

const DELETE_KEYS: [&str; 3] = ["bounty_ids", "bountyIds", "bounties"];

/// No body, `null`, or an object naming none of the delete keys: the game's ResetBounties.
pub fn names_nothing(body: Option<&Value>) -> bool {
    match body {
        None | Some(Value::Null) => true,
        Some(Value::Object(map)) => !DELETE_KEYS
            .iter()
            .chain(["delete_claimed_bounties"].iter())
            .any(|k| map.contains_key(*k)),
        Some(_) => false,
    }
}

pub fn parse_delete(body: Option<&Value>) -> Result<DeleteRequest, BountySaveError> {
    let Some(Value::Object(map)) = body else {
        return fail("body must be an object");
    };
    let mut ids: Vec<String> = Vec::new();
    for key in DELETE_KEYS {
        let Some(value) = map.get(key) else { continue };
        let Some(list) = value.as_array() else {
            return fail(format!("{key} must be an array"));
        };
        if list.len() > MAX_SAVED_BOUNTIES {
            return fail(format!("too many bounties ({})", list.len()));
        }
        for item in list {
            let id = match item {
                Value::String(s) => Some(s.as_str()),
                Value::Object(o) => o.get("bounty_id").and_then(Value::as_str),
                _ => None,
            };
            let Some(id) = id.filter(|id| (1..=200).contains(&utf16_len(id))) else {
                return fail(format!("every entry of {key} needs a bounty id string"));
            };
            if !ids.iter().any(|seen| seen == id) {
                ids.push(id.to_owned());
            }
        }
    }
    let delete_claimed = map.get("delete_claimed_bounties") == Some(&Value::Bool(true));
    if ids.is_empty() && !delete_claimed {
        return fail("no bounty to delete");
    }
    Ok(DeleteRequest { ids, delete_claimed })
}

#[derive(Debug, Clone, PartialEq)]
pub struct Deleted {
    pub record: BountyStateRecord,
    pub deleted: Vec<String>,
    pub kept: Vec<String>,
}

/// `DeleteBounties` (§4.4): season challenges are never deleted; both day fields are cleared (§14 item 3).
pub fn delete_bounties(
    stored: Option<&BountyStateRecord>,
    account: &AccountId,
    request: &DeleteRequest,
    now: &str,
) -> Deleted {
    let mut record = new_record(stored, account, now);
    let mut deleted = Vec::new();
    record.bounties.retain(|e| {
        if effective_group(e) == BountyGroup::Weekly {
            return true;
        }
        let id = bounty_id(e).unwrap_or_default();
        let named = request.ids.iter().any(|n| n == id);
        if named || (request.delete_claimed && is_claimed(e)) {
            deleted.push(id.to_owned());
            false
        } else {
            true
        }
    });
    let kept = request.ids.iter().filter(|id| !deleted.contains(id)).cloned().collect();
    record.update_version = stored.map_or(1, |r| r.update_version + 1);
    record.updated_at = now.to_owned();
    record.daily_reward_credited_on = None;
    record.daily_premium_draft_granted_on = None;
    Deleted { record, deleted, kept }
}

// ------------------------------------------------------------------ payload

fn default_draft() -> Value {
    obj(vec![
        ("current_draft_choices", Value::Array(Vec::new())),
        ("previous_draft_selections", Value::Array(Vec::new())),
        ("bronze_count", Value::from(0)),
        ("silver_count", Value::from(0)),
        ("gold_count", Value::from(0)),
    ])
}

/// `BountyStatePayload` (§4.2.6); none is the empty state payload.
pub fn bounty_state_payload(record: Option<&BountyStateRecord>) -> Value {
    let drafts = record.map(|r| &r.draft_data);
    let draft =
        |pick: fn(&DraftData) -> &Option<Value>| drafts.and_then(|d| pick(d).clone()).unwrap_or_else(default_draft);
    obj(vec![
        ("season_start_date", Value::from("2020-08-23T00:00:00.000Z")),
        ("season_end_date", Value::from("2099-01-01T00:00:00.000Z")),
        (
            "bounties",
            Value::Array(
                record
                    .map(|r| r.bounties.iter().map(|e| e.bounty.clone()).collect())
                    .unwrap_or_default(),
            ),
        ),
        ("draft_data", draft(|d| &d.base)),
        ("draft_data_daily", draft(|d| &d.daily)),
        ("draft_data_weekly", draft(|d| &d.weekly)),
    ])
}

/// The log's `{counts}`.
pub fn counts(record: Option<&BountyStateRecord>) -> String {
    let mut n = [0usize; 6];
    for entry in record.map_or(&[][..], |r| r.bounties.as_slice()) {
        let slot = match entry.group {
            BountyGroup::Base => 0,
            BountyGroup::Daily => 1,
            BountyGroup::Weekly => 2,
            BountyGroup::Unassigned => 3,
        };
        n[slot] += 1;
        if is_claimed(entry) {
            n[4] += 1;
        }
        if entry.reward_credit.is_some() {
            n[5] += 1;
        }
    }
    format!(
        "{{\"base\":{},\"daily\":{},\"weekly\":{},\"unassigned\":{},\"claimed\":{},\"credited\":{}}}",
        n[0], n[1], n[2], n[3], n[4], n[5]
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn account() -> AccountId {
        "account:01ARZ3NDEKTSV4RRFFQ69G5FAV@n".parse().unwrap()
    }

    fn save(body: Value) -> BountySave {
        validate_bounty_save(Some(&body)).unwrap()
    }

    #[test]
    fn validation() {
        assert_eq!(
            validate_bounty_save(Some(&json!([]))).unwrap_err().0,
            "body must be an object"
        );
        assert_eq!(validate_bounty_save(None).unwrap_err().0, "body must be an object");
        assert_eq!(
            validate_bounty_save(Some(&json!({"bounties": 5}))).unwrap_err().0,
            "bounties must be an array"
        );
        assert_eq!(
            validate_bounty_save(Some(&json!({"bounties": [1]}))).unwrap_err().0,
            "every bounty must be an object"
        );
        assert_eq!(
            validate_bounty_save(Some(&json!({"bounties": [{"bounty_id": ""}]})))
                .unwrap_err()
                .0,
            "every bounty needs a bounty_id string"
        );
        assert_eq!(
            validate_bounty_save(Some(&json!({"bounties": [{"bounty_id": "b", "objectives": null}]})))
                .unwrap_err()
                .0,
            "objectives of b must be an array"
        );
        assert_eq!(
            validate_bounty_save(Some(&json!({"draft_data_daily": null})))
                .unwrap_err()
                .0,
            "draft_data_daily must be an object"
        );
        assert!(validate_bounty_save(Some(&json!({}))).is_ok());
    }

    #[test]
    fn merges() {
        let a = account();
        let first = merge_bounty_state(
            None,
            &a,
            &save(json!({"bounties": [
            {"bounty_id": "Challenge_Daily_A", "objectives": [{"objective_id": "o", "progress": 5}]},
            {"bounty_id": "Base_1"}
        ], "draft_data_daily": {"x": 1}})),
            "T1",
        );
        assert_eq!(first.mode, "replace");
        assert_eq!(first.record.update_version, 1);
        assert_eq!(
            first.record.bounties.iter().map(|e| e.group).collect::<Vec<_>>(),
            [BountyGroup::Daily, BountyGroup::Daily]
        );
        let second = merge_bounty_state(
            Some(&first.record),
            &a,
            &save(json!({"bounties": [
                {"bounty_id": "Challenge_Daily_A", "objectives": [{"objective_id": "o", "progress": 2}], "claimed": true}
            ]})),
            "T2",
        );
        let entry = &second.record.bounties[0];
        assert_eq!(entry.bounty["objectives"][0]["progress"], 5);
        assert_eq!(entry.claimed_at.as_deref(), Some("T2"));
        assert_eq!(second.newly_claimed, ["Challenge_Daily_A"]);
        let third = merge_bounty_state(
            Some(&second.record),
            &a,
            &save(json!({"bounties": [{"bounty_id": "Challenge_Daily_A", "claimed": false}]})),
            "T3",
        );
        assert_eq!(third.record.bounties[0].bounty["claimed"], true);
        assert_eq!(third.record.bounties[0].claimed_at.as_deref(), Some("T2"));
        assert!(third.newly_claimed.is_empty());
        // A new instance (another stamp) clears the claim
        let redraft = merge_bounty_state(
            Some(&third.record),
            &a,
            &save(json!({"bounties": [{"bounty_id": "Challenge_Daily_A", "drafted_timestamp": "x"}]})),
            "T4",
        );
        assert_eq!(redraft.record.bounties[0].claimed_at, None);
        // A daily replace removes the other daily entries, never a weekly one
        let replace = merge_bounty_state(
            Some(&redraft.record),
            &a,
            &save(json!({"bounties": [{"bounty_id": "Challenge_Daily_B"}], "draft_data_daily": {}})),
            "T5",
        );
        assert_eq!(replace.removed, 2);
        let ids: Vec<_> = replace.record.bounties.iter().filter_map(bounty_id).collect();
        assert_eq!(ids, ["Challenge_Daily_B"]);
    }

    #[test]
    fn weekly_ids() {
        let id = "43_2_10_Challenge_Season_Kill_Things-season43-1";
        let parsed = parse_weekly_id(id).unwrap();
        assert_eq!((parsed.week, parsed.slot), (2, 10));
        assert_eq!(parsed.text(), id);
        for bad in [
            "43_02_1_Challenge_Season_A-season43-1",
            "43_12_1_Challenge_Season_A-season43-11",
            "43_2_11_Challenge_Season_A-season43-1",
            "43_2_1_Challenge_Season_A-season43-2",
            "x_2_1_Challenge_Season_A-season43-1",
            "43_2_1_Challenge_Season_-season43-1",
            "43_2_1_Challenge_Season_A-seasonx-1",
        ] {
            assert!(parse_weekly_id(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn stamps() {
        let (start, _) = season::active_season_window_ms(season::ANCHOR_MS + 10 * WEEK_MS);
        assert_eq!(
            accepted_stamps(1, start + ROLL_GRACE_MS - 1),
            vec![start, start - season::LENGTH_MS]
        );
        assert_eq!(accepted_stamps(1, start + ROLL_GRACE_MS), vec![start]);
        assert_eq!(accepted_stamps(3, start + 5)[0], start + 2 * WEEK_MS);
    }

    #[test]
    fn deletes() {
        assert!(names_nothing(None));
        assert!(names_nothing(Some(&json!({"other": 1}))));
        assert!(!names_nothing(Some(&json!({"bounties": []}))));
        assert_eq!(
            parse_delete(Some(&json!({"bounties": []}))).unwrap_err().0,
            "no bounty to delete"
        );
        assert_eq!(
            parse_delete(Some(&json!({"bountyIds": 1}))).unwrap_err().0,
            "bountyIds must be an array"
        );
        let r = parse_delete(Some(&json!({"bounty_ids": ["a", {"bounty_id": "a"}, "b"]}))).unwrap();
        assert_eq!(r.ids, ["a", "b"]);
    }

    #[test]
    fn payload_keys() {
        let p = bounty_state_payload(None);
        let keys: Vec<_> = p.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            [
                "season_start_date",
                "season_end_date",
                "bounties",
                "draft_data",
                "draft_data_daily",
                "draft_data_weekly"
            ]
        );
    }
}
