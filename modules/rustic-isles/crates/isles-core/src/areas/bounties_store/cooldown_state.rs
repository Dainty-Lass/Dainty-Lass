//! Per-account cooldowns for `GET /cooldown/:userId`, `PUT /cooldown/batch/:userId` and the single-id start
//! (`cooldownState.ts`, `controllers/cooldowns.ts`). The game keeps its once-per-period gates there (the daily bounty,
//! the patrol bonus, a harvested sprout).
//!
//! The wire shapes differ by direction: the PUT carries `{cooldowns: [{cooldown_id, cooldown_started_date}]}` and the GET
//! answers a map `{"<cooldown_id>": "<ISO date>"}`. Dates are stored and served exactly as the game wrote them.

use serde_json::{Map, Value};

use super::authn::AuthData;
use super::util::{iso, js_len, parse_ms, parse_ms_f64};
use crate::game_data::season_window::active_season_window_ms;
use crate::ids::AccountId;
use crate::repo::bounties::{CooldownEntry, CooldownStateRecord};

pub const MAX_COOLDOWNS_PER_SAVE: usize = 256;
pub const MAX_STORED_COOLDOWNS: usize = 1024;
const MAX_ID_LENGTH: usize = 200;
const MAX_DATE_LENGTH: usize = 64;

/// Seasonal city gatherables store one entry per harvested node, keyed by its 32-hex GUID. The game reads an id missing
/// from the map and one older than its daily respawn alike, so dropping an entry after 48 hours changes nothing it sees.
pub const GATHERABLE_RETENTION_MS: i64 = 48 * 60 * 60 * 1000;

fn is_gatherable_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn is_expired_gatherable_cooldown(id: &str, started_date: &str, now: &str) -> bool {
    if !is_gatherable_id(id) {
        return false;
    }
    // An unparsable date keeps the entry: dropping it would report the node available
    match (parse_ms(started_date), parse_ms(now)) {
        (Some(started), Some(now)) => now - started > GATHERABLE_RETENTION_MS,
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct CooldownSaveValidationError(pub String);

fn invalid(message: impl Into<String>) -> CooldownSaveValidationError {
    CooldownSaveValidationError(message.into())
}

/// Writes need dedicated-server authority: a player rewinding a start date could collect a periodic reward again.
pub fn is_authorized_cooldown_writer(auth: &AuthData) -> bool {
    auth.is_gameserver
}

/// A player-host runtime: writes later than the stored date only (`merge_host_cooldown_state`).
pub fn is_host_cooldown_writer(auth: &AuthData) -> bool {
    auth.is_host_runtime() && !auth.is_gameserver
}

/// A player-host runtime may read the cooldowns of its own hunt's roster.
pub fn is_authorized_cooldown_reader(auth: &AuthData, target_user_id: &str, target: Option<&AccountId>) -> bool {
    if auth.is_gameserver {
        return true;
    }
    if auth.is_host_runtime() {
        return auth.roster_has(target_user_id, target);
    }
    auth.user_id.as_deref() == Some(target_user_id)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingCooldown {
    pub cooldown_id: String,
    pub cooldown_started_date: String,
}

/// `ValidateCooldownSave`: accepts `{cooldowns: [...]}` or a bare array.
pub fn validate_cooldown_save(body: Option<&Value>) -> Result<Vec<IncomingCooldown>, CooldownSaveValidationError> {
    let list = match body {
        Some(Value::Array(list)) => Some(list),
        Some(Value::Object(map)) => map.get("cooldowns").and_then(Value::as_array),
        _ => None,
    };
    let Some(list) = list else {
        return Err(invalid("cooldowns must be an array"));
    };
    if list.len() > MAX_COOLDOWNS_PER_SAVE {
        return Err(invalid(format!("too many cooldowns ({})", list.len())));
    }
    list.iter()
        .map(|entry| {
            let id = entry.get("cooldown_id").and_then(Value::as_str);
            let Some(id) = id.filter(|id| !id.is_empty() && js_len(id) <= MAX_ID_LENGTH) else {
                return Err(invalid("every cooldown needs a cooldown_id string"));
            };
            let started = entry.get("cooldown_started_date").and_then(Value::as_str);
            match started {
                Some(started)
                    if !started.is_empty() && js_len(started) <= MAX_DATE_LENGTH && parse_ms(started).is_some() =>
                {
                    Ok(IncomingCooldown {
                        cooldown_id: id.to_owned(),
                        cooldown_started_date: started.to_owned(),
                    })
                }
                _ => Err(invalid(format!(
                    "cooldown {id} needs a cooldown_started_date date string"
                ))),
            }
        })
        .collect()
}

/// `MergeCooldownState`: a save sets the listed cooldowns (the latest write for an id wins) and removes no others, apart
/// from gatherable ids past `GATHERABLE_RETENTION_MS`.
pub fn merge_cooldown_state(
    account: &AccountId,
    previous: Option<&CooldownStateRecord>,
    incoming: &[IncomingCooldown],
    now: &str,
) -> CooldownStateRecord {
    let incoming_ids: std::collections::HashSet<&str> = incoming.iter().map(|e| e.cooldown_id.as_str()).collect();
    // Map insertion order: a rewritten id keeps its place
    let mut entries: Vec<CooldownEntry> = Vec::new();
    for entry in previous.map(|p| p.entries.as_slice()).unwrap_or_default() {
        // Pruned at write time; the ids this save writes are never pruned
        if !incoming_ids.contains(entry.id.as_str())
            && is_expired_gatherable_cooldown(&entry.id, &entry.started_date, now)
        {
            continue;
        }
        entries.push(entry.clone());
    }
    for entry in incoming {
        let written = CooldownEntry {
            id: entry.cooldown_id.clone(),
            started_date: entry.cooldown_started_date.clone(),
            updated_at: now.to_owned(),
        };
        match entries.iter_mut().find(|e| e.id == written.id) {
            Some(slot) => *slot = written,
            None => entries.push(written),
        }
    }
    if entries.len() > MAX_STORED_COOLDOWNS {
        // Keep the most recently written ones (`b.updatedAt.localeCompare(a.updatedAt)`, ISO strings)
        entries.sort_by(|a, b| service_kit::collate::locale_compare(&b.updated_at, &a.updated_at));
        entries.truncate(MAX_STORED_COOLDOWNS);
    }
    CooldownStateRecord {
        account: account.clone(),
        entries,
        update_version: previous.map_or(0, |p| p.update_version) + 1,
        created_at: previous.map_or_else(|| now.to_owned(), |p| p.created_at.clone()),
        updated_at: now.to_owned(),
    }
}

// ---- Host writes: a cooldown may only move later, and not far ahead.
//
// A start date is a once-per-period gate: moving it earlier, or dropping it, re-opens the gate and pays again; a far
// future date would lock a friend's daily gate for good. Per id: not stored -> added (while the store has room); later
// or equal -> accepted; earlier than stored -> the stored date is kept and the rest of the save applies; more than 48 h
// ahead (a season id: past the end of the current pass window) -> refused, the stored date kept.

pub const HOST_COOLDOWN_MAX_FUTURE_MS: i64 = 48 * 60 * 60 * 1000;
const MAX_RECORDED_COOLDOWN_CHANGES: usize = 20;

/// `season43`, `season10a`: the hunt-pass ids, whose cooldown is the pass window's end.
pub fn is_season_cooldown_id(id: &str) -> bool {
    let Some(rest) = id
        .get(..6)
        .filter(|p| p.eq_ignore_ascii_case("season"))
        .map(|_| &id[6..])
    else {
        return false;
    };
    let digits = rest.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let suffix = &rest[digits.len()..];
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) && suffix.len() <= 1
}

/// The latest start date (instant) a host may write for cooldown `id` when the backend's clock reads `now_ms`.
pub fn host_cooldown_latest_ms(id: &str, now_ms: i64) -> i64 {
    if is_season_cooldown_id(id) {
        active_season_window_ms(now_ms).1
    } else {
        now_ms + HOST_COOLDOWN_MAX_FUTURE_MS
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct HostCooldownChanges {
    pub received: usize,
    pub unchanged: usize,
    /// (id, to); capped lists, the counts are the real numbers
    pub added: Vec<(String, String)>,
    pub added_count: usize,
    /// (id, from, to)
    pub raised: Vec<(String, String, String)>,
    pub raised_count: usize,
    /// (id, stored, attempted): the save tried to move it earlier and the stored date was kept
    pub kept_later: Vec<(String, String, String)>,
    pub kept_later_count: usize,
    /// New ids refused because the store was full
    pub dropped_new: usize,
    /// (id, attempted, latest, stored)
    pub refused_future: Vec<(String, String, String, Option<String>)>,
    pub refused_future_count: usize,
}

impl HostCooldownChanges {
    /// The tag `hostWrites.record` gets (Node spreads the object, `JSON.stringify` order).
    pub fn to_json(&self) -> Map<String, Value> {
        let list = |items: Vec<Value>| Value::Array(items);
        let mut out = Map::new();
        out.insert("received".into(), self.received.into());
        out.insert("unchanged".into(), self.unchanged.into());
        out.insert(
            "added".into(),
            list(
                self.added
                    .iter()
                    .map(|(id, to)| serde_json::json!({ "id": id, "to": to }))
                    .collect(),
            ),
        );
        out.insert("addedCount".into(), self.added_count.into());
        out.insert(
            "raised".into(),
            list(
                self.raised
                    .iter()
                    .map(|(id, from, to)| serde_json::json!({ "id": id, "from": from, "to": to }))
                    .collect(),
            ),
        );
        out.insert("raisedCount".into(), self.raised_count.into());
        out.insert(
            "keptLater".into(),
            list(
                self.kept_later
                    .iter()
                    .map(|(id, stored, attempted)| {
                        serde_json::json!({ "id": id, "stored": stored, "attempted": attempted })
                    })
                    .collect(),
            ),
        );
        out.insert("keptLaterCount".into(), self.kept_later_count.into());
        out.insert("droppedNew".into(), self.dropped_new.into());
        out.insert(
            "refusedFuture".into(),
            list(
                self.refused_future
                    .iter()
                    .map(|(id, attempted, latest, stored)| {
                        let mut m = Map::new();
                        m.insert("id".into(), id.clone().into());
                        m.insert("attempted".into(), attempted.clone().into());
                        m.insert("latest".into(), latest.clone().into());
                        if let Some(stored) = stored {
                            m.insert("stored".into(), stored.clone().into());
                        }
                        Value::Object(m)
                    })
                    .collect(),
            ),
        );
        out.insert("refusedFutureCount".into(), self.refused_future_count.into());
        out
    }
}

/// `MergeHostCooldownState`.
pub fn merge_host_cooldown_state(
    account: &AccountId,
    previous: Option<&CooldownStateRecord>,
    body: Option<&Value>,
    now: &str,
) -> Result<(CooldownStateRecord, HostCooldownChanges), CooldownSaveValidationError> {
    let incoming = validate_cooldown_save(body)?;
    let mut stored: Vec<(String, String)> = previous
        .map(|p| {
            p.entries
                .iter()
                .map(|e| (e.id.clone(), e.started_date.clone()))
                .collect()
        })
        .unwrap_or_default();
    let mut room = MAX_STORED_COOLDOWNS as i64 - stored.len() as i64;
    let now_ms = parse_ms(now).unwrap_or_else(super::util::now_ms);

    let mut accepted: Vec<IncomingCooldown> = Vec::new();
    let mut changes = HostCooldownChanges {
        received: incoming.len(),
        ..HostCooldownChanges::default()
    };
    for entry in incoming {
        let id = entry.cooldown_id.clone();
        let current = stored.iter().find(|(k, _)| *k == id).map(|(_, v)| v.clone());
        // Too far ahead: refused, the stored date (if any) stays. The very date already stored is not a refusal. This
        // runs before the room check so a refused new id costs no room.
        let latest = host_cooldown_latest_ms(&id, now_ms);
        let attempted_ms = parse_ms_f64(&entry.cooldown_started_date);
        let same_as_stored = current.as_deref().is_some_and(|c| parse_ms_f64(c) == attempted_ms);
        if attempted_ms > latest as f64 && !same_as_stored {
            changes.refused_future_count += 1;
            if changes.refused_future.len() < MAX_RECORDED_COOLDOWN_CHANGES {
                changes
                    .refused_future
                    .push((id, entry.cooldown_started_date.clone(), iso(latest), current));
            }
            continue;
        }
        match current {
            None => {
                if room <= 0 {
                    changes.dropped_new += 1;
                    continue;
                }
                room -= 1;
                changes.added_count += 1;
                if changes.added.len() < MAX_RECORDED_COOLDOWN_CHANGES {
                    changes.added.push((id.clone(), entry.cooldown_started_date.clone()));
                }
            }
            Some(current) => {
                let current_ms = parse_ms_f64(&current);
                if current_ms.is_finite() && attempted_ms < current_ms {
                    changes.kept_later_count += 1;
                    if changes.kept_later.len() < MAX_RECORDED_COOLDOWN_CHANGES {
                        changes
                            .kept_later
                            .push((id, current, entry.cooldown_started_date.clone()));
                    }
                    continue;
                }
                if attempted_ms == current_ms {
                    changes.unchanged += 1;
                } else {
                    changes.raised_count += 1;
                    if changes.raised.len() < MAX_RECORDED_COOLDOWN_CHANGES {
                        changes
                            .raised
                            .push((id.clone(), current, entry.cooldown_started_date.clone()));
                    }
                }
            }
        }
        // A later duplicate of the same id in this save is judged against what was just accepted
        match stored.iter_mut().find(|(k, _)| *k == entry.cooldown_id) {
            Some(slot) => slot.1.clone_from(&entry.cooldown_started_date),
            None => stored.push((entry.cooldown_id.clone(), entry.cooldown_started_date.clone())),
        }
        accepted.push(entry);
    }
    Ok((merge_cooldown_state(account, previous, &accepted, now), changes))
}

/// `CooldownStatePayload`: the map of cooldown id to start date, the shape `OnQueryCooldownDataRequestComplete` reads.
pub fn cooldown_state_payload(record: Option<&CooldownStateRecord>) -> Value {
    let mut map = Map::new();
    for entry in record.map(|r| r.entries.as_slice()).unwrap_or_default() {
        map.insert(entry.id.clone(), Value::from(entry.started_date.as_str()));
    }
    // JavaScript moves integer-like keys to the front; the reply's stringify does the same
    Value::Object(map)
}

/// The stored cooldowns as the log lines print them: `id@date`.
pub fn cooldown_list(record: Option<&CooldownStateRecord>) -> Vec<(String, String)> {
    record
        .map(|r| {
            r.entries
                .iter()
                .map(|e| (e.id.clone(), e.started_date.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// `IsValidCooldownId`: `^[A-Za-z0-9_.:-]{1,200}$`.
pub fn is_valid_cooldown_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 200
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))
}
