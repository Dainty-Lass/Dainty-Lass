// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Cooldowns (bounties spec §4.6): the game's once-per-period gates (the daily bounty, the patrol bonus, a harvested
//! sprout). Dates are stored and served exactly as written.

use std::collections::HashMap;

use dainty_service_kit::js;
use rustic_isles::season;
use rustic_isles_store::AccountId;
use rustic_isles_store::repo::bounties::{CooldownEntry, CooldownStateRecord};
use serde_json::{Map, Value};

use crate::util::{iso, obj, utf16_len};

/// The most cooldowns one save may carry.
pub const MAX_SAVED: usize = 256;
/// The most cooldowns kept per account; the oldest beyond it are dropped.
pub const MAX_STORED: usize = 1024;
const AHEAD_MS: i64 = 48 * 3_600_000;
const LIST_CAP: usize = 20;

/// A cooldown save the validation refused; the text says why.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct CooldownSaveError(pub String);

/// `^[A-Za-z0-9_.:-]{1,200}$` (ASCII bytes).
pub fn is_valid_cooldown_id(id: &str) -> bool {
    (1..=200).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))
}

/// `season` in any case, one or more digits, then at most one ASCII letter (`season43`, `season10a`).
pub fn is_season_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    if bytes.len() < 7 || !bytes[..6].eq_ignore_ascii_case(b"season") {
        return false;
    }
    let rest = &bytes[6..];
    let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
    digits > 0 && rest.len() - digits <= 1 && rest[digits..].iter().all(u8::is_ascii_alphabetic)
}

/// An id of exactly 32 ASCII hex digits (a harvested gatherable).
pub fn is_gatherable(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `ValidateCooldownSave`: `(id, date)` in order.
pub fn validate_cooldown_save(body: Option<&Value>) -> Result<Vec<(String, String)>, CooldownSaveError> {
    let list = match body {
        Some(Value::Array(list)) => list,
        Some(Value::Object(map)) => match map.get("cooldowns") {
            Some(Value::Array(list)) => list,
            _ => return Err(CooldownSaveError("cooldowns must be an array".into())),
        },
        _ => return Err(CooldownSaveError("cooldowns must be an array".into())),
    };
    if list.len() > MAX_SAVED {
        return Err(CooldownSaveError(format!("too many cooldowns ({})", list.len())));
    }
    let mut out = Vec::new();
    for entry in list {
        let Some(id) = entry
            .get("cooldown_id")
            .and_then(Value::as_str)
            .filter(|id| (1..=200).contains(&utf16_len(id)))
        else {
            return Err(CooldownSaveError("every cooldown needs a cooldown_id string".into()));
        };
        let Some(date) = entry
            .get("cooldown_started_date")
            .and_then(Value::as_str)
            .filter(|d| (1..=64).contains(&utf16_len(d)) && js::date_parse(d).is_some())
        else {
            return Err(CooldownSaveError(format!(
                "cooldown {id} needs a cooldown_started_date date string"
            )));
        };
        out.push((id.to_owned(), date.to_owned()));
    }
    Ok(out)
}

/// `MergeCooldownState`: the dedicated server's merge (no future limit).
pub fn merge_cooldowns(
    stored: Option<&CooldownStateRecord>,
    account: &AccountId,
    incoming: &[(String, String)],
    now_ms: i64,
) -> CooldownStateRecord {
    let now = iso(now_ms);
    let mut entries: Vec<CooldownEntry> = stored.map(|r| r.entries.clone()).unwrap_or_default();
    entries.retain(|e| {
        if !is_gatherable(&e.id) || incoming.iter().any(|(id, _)| *id == e.id) {
            return true;
        }
        js::date_parse(&e.started_date).is_none_or(|started| now_ms - started <= AHEAD_MS)
    });
    for (id, date) in incoming {
        let entry = CooldownEntry {
            id: id.clone(),
            started_date: date.clone(),
            updated_at: now.clone(),
        };
        match entries.iter_mut().find(|e| e.id == *id) {
            Some(existing) => *existing = entry,
            None => entries.push(entry),
        }
    }
    if entries.len() > MAX_STORED {
        entries.sort_by(|a, b| dainty_service_kit::collate::locale_compare(&b.updated_at, &a.updated_at));
        entries.truncate(MAX_STORED);
    }
    CooldownStateRecord {
        account: account.clone(),
        entries,
        update_version: stored.map_or(1, |r| r.update_version + 1),
        created_at: stored.map_or_else(|| now.clone(), |r| r.created_at.clone()),
        updated_at: now,
    }
}

/// What a host's batch did (§4.6.3); the lists keep at most 20 entries, the counts are exact.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HostCooldownChanges {
    /// Cooldowns in the batch.
    pub received: usize,
    /// Cooldowns already stored with the same date.
    pub unchanged: usize,
    /// The new cooldowns stored: `{id, to}`.
    pub added: Vec<Value>,
    /// How many new cooldowns were stored.
    pub added_count: usize,
    /// Cooldowns moved to a later date: `{id, from, to}`.
    pub raised: Vec<Value>,
    /// How many cooldowns moved later.
    pub raised_count: usize,
    /// Cooldowns whose stored date was later and stays: `{id, stored, attempted}`.
    pub kept_later: Vec<Value>,
    /// How many stored dates were kept.
    pub kept_later_count: usize,
    /// New cooldowns dropped because the account's list was full.
    pub dropped_new: usize,
    /// Dates too far ahead, refused: `{id, attempted, latest, stored?}`.
    pub refused_future: Vec<Value>,
    /// How many dates were refused.
    pub refused_future_count: usize,
}

fn push(list: &mut Vec<Value>, count: &mut usize, value: Value) {
    *count += 1;
    if list.len() < LIST_CAP {
        list.push(value);
    }
}

/// The latest start a host may write for `id` at `now_ms`.
pub fn latest_allowed(id: &str, now_ms: i64) -> i64 {
    if is_season_id(id) {
        season::active_season_window_ms(now_ms).1
    } else {
        now_ms + AHEAD_MS
    }
}

/// `MergeHostCooldownState`: a start may move only later, and not far ahead. The accepted entries (for the dedicated
/// merge) and the changes.
pub fn plan_host_cooldowns(
    stored: Option<&CooldownStateRecord>,
    incoming: &[(String, String)],
    now_ms: i64,
) -> (Vec<(String, String)>, HostCooldownChanges) {
    let mut dates: HashMap<String, String> = stored
        .map(|r| {
            r.entries
                .iter()
                .map(|e| (e.id.clone(), e.started_date.clone()))
                .collect()
        })
        .unwrap_or_default();
    let mut room = MAX_STORED.saturating_sub(dates.len());
    let mut changes = HostCooldownChanges {
        received: incoming.len(),
        ..HostCooldownChanges::default()
    };
    let mut accepted = Vec::new();
    for (id, date) in incoming {
        let attempted = js::date_parse(date);
        let latest = latest_allowed(id, now_ms);
        let stored_date = dates.get(id).cloned();
        let stored_ms = stored_date.as_deref().and_then(js::date_parse);
        let same = attempted.is_some() && attempted == stored_ms;
        if attempted.is_some_and(|a| a > latest) && !same {
            let mut refused = obj(vec![
                ("id", Value::from(id.as_str())),
                ("attempted", Value::from(date.as_str())),
                ("latest", Value::from(iso(latest))),
            ]);
            if let (Some(stored), Some(map)) = (&stored_date, refused.as_object_mut()) {
                map.insert("stored".into(), Value::from(stored.as_str()));
            }
            push(&mut changes.refused_future, &mut changes.refused_future_count, refused);
            continue;
        }
        let Some(from) = stored_date else {
            if room == 0 {
                changes.dropped_new += 1;
                continue;
            }
            room -= 1;
            push(
                &mut changes.added,
                &mut changes.added_count,
                obj(vec![
                    ("id", Value::from(id.as_str())),
                    ("to", Value::from(date.as_str())),
                ]),
            );
            dates.insert(id.clone(), date.clone());
            accepted.push((id.clone(), date.clone()));
            continue;
        };
        if let (Some(s), Some(a)) = (stored_ms, attempted)
            && s > a
        {
            push(
                &mut changes.kept_later,
                &mut changes.kept_later_count,
                obj(vec![
                    ("id", Value::from(id.as_str())),
                    ("stored", Value::from(from.as_str())),
                    ("attempted", Value::from(date.as_str())),
                ]),
            );
            continue;
        }
        if same {
            changes.unchanged += 1;
        } else {
            push(
                &mut changes.raised,
                &mut changes.raised_count,
                obj(vec![
                    ("id", Value::from(id.as_str())),
                    ("from", Value::from(from.as_str())),
                    ("to", Value::from(date.as_str())),
                ]),
            );
        }
        dates.insert(id.clone(), date.clone());
        accepted.push((id.clone(), date.clone()));
    }
    (accepted, changes)
}

impl HostCooldownChanges {
    /// The host-write tag (§4.6.3); `start` is `(id, startedAt)` for a start, `version` a batch's.
    pub fn tag(&self, start: Option<(&str, &str)>, version: Option<i64>) -> Value {
        let mut map = Map::new();
        map.insert(
            "route".into(),
            Value::from(if start.is_some() { "start" } else { "batch" }),
        );
        if let Some((id, at)) = start {
            map.insert("id".into(), Value::from(id));
            map.insert("startedAt".into(), Value::from(at));
        }
        map.insert("received".into(), Value::from(self.received));
        map.insert("unchanged".into(), Value::from(self.unchanged));
        map.insert("added".into(), Value::Array(self.added.clone()));
        map.insert("addedCount".into(), Value::from(self.added_count));
        map.insert("raised".into(), Value::Array(self.raised.clone()));
        map.insert("raisedCount".into(), Value::from(self.raised_count));
        map.insert("keptLater".into(), Value::Array(self.kept_later.clone()));
        map.insert("keptLaterCount".into(), Value::from(self.kept_later_count));
        map.insert("droppedNew".into(), Value::from(self.dropped_new));
        map.insert("refusedFuture".into(), Value::Array(self.refused_future.clone()));
        map.insert("refusedFutureCount".into(), Value::from(self.refused_future_count));
        if start.is_none()
            && let Some(version) = version
        {
            map.insert("version".into(), Value::from(version));
        }
        Value::Object(map)
    }
}

/// `CooldownStatePayload`: each id to its stored date (integer-like ids written first, as JavaScript objects do).
pub fn cooldown_payload(record: Option<&CooldownStateRecord>) -> Value {
    let mut map = Map::new();
    for entry in record.map_or(&[][..], |r| r.entries.as_slice()) {
        map.insert(entry.id.clone(), Value::from(entry.started_date.as_str()));
    }
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn account() -> AccountId {
        "account:01ARZ3NDEKTSV4RRFFQ69G5FAV@n".parse().unwrap()
    }

    #[test]
    fn validation() {
        assert_eq!(
            validate_cooldown_save(Some(&json!({}))).unwrap_err().0,
            "cooldowns must be an array"
        );
        assert_eq!(
            validate_cooldown_save(Some(&json!([{"cooldown_id": ""}])))
                .unwrap_err()
                .0,
            "every cooldown needs a cooldown_id string"
        );
        assert_eq!(
            validate_cooldown_save(Some(&json!([{"cooldown_id": "a", "cooldown_started_date": "x"}])))
                .unwrap_err()
                .0,
            "cooldown a needs a cooldown_started_date date string"
        );
        let ok = validate_cooldown_save(Some(
            &json!([{"cooldown_id": "a", "cooldown_started_date": "2026-01-01T00:00:00.000Z"}]),
        ))
        .unwrap();
        assert_eq!(ok, vec![("a".to_owned(), "2026-01-01T00:00:00.000Z".to_owned())]);
        let many: Vec<Value> = (0..257)
            .map(|i| json!({"cooldown_id": format!("c{i}"), "cooldown_started_date": "2026-01-01"}))
            .collect();
        assert_eq!(
            validate_cooldown_save(Some(&Value::Array(many))).unwrap_err().0,
            "too many cooldowns (257)"
        );
    }

    #[test]
    fn ids() {
        assert!(is_season_id("season43") && is_season_id("SEASON10a"));
        assert!(!is_season_id("season") && !is_season_id("season4ab") && !is_season_id("season_4"));
        assert!(is_valid_cooldown_id("Sprout_1.x:y-z") && !is_valid_cooldown_id("bad id"));
        assert!(is_gatherable("0123456789abcdefABCDEF0123456789"));
    }

    #[test]
    fn dedicated_merge() {
        let a = account();
        let now = dainty_service_kit::js::date_parse("2026-11-01T00:00:00.000Z").unwrap();
        let old = "2026-10-01T00:00:00.000Z";
        let gather = "0123456789abcdef0123456789abcdef";
        let first = merge_cooldowns(
            None,
            &a,
            &[
                (gather.into(), old.into()),
                ("__proto__".into(), old.into()),
                ("a".into(), old.into()),
            ],
            now,
        );
        let next = merge_cooldowns(
            Some(&first),
            &a,
            &[("a".into(), "2026-10-31T00:00:00.000Z".into())],
            now,
        );
        assert_eq!(
            next.entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["__proto__", "a"]
        );
        assert_eq!(cooldown_payload(Some(&next))["__proto__"], old);
        let kept = merge_cooldowns(Some(&first), &a, &[(gather.into(), old.into())], now);
        assert!(kept.entries.iter().any(|e| e.id == gather));
    }

    #[test]
    fn host_merge() {
        let a = account();
        let now = dainty_service_kit::js::date_parse("2026-11-01T00:00:00.000Z").unwrap();
        let stored = merge_cooldowns(
            None,
            &a,
            &[
                ("x".into(), "2026-10-31T00:00:00.000Z".into()),
                ("bad".into(), "nope".into()),
            ],
            now,
        );
        let incoming = vec![
            ("x".to_owned(), "2026-10-30T00:00:00.000Z".to_owned()),
            ("x".to_owned(), "2026-10-31T00:00:00.000Z".to_owned()),
            ("y".to_owned(), "2026-11-01T00:00:00.000Z".to_owned()),
            ("bad".to_owned(), "2026-10-01T00:00:00.000Z".to_owned()),
            ("far".to_owned(), "2026-11-10T00:00:00.000Z".to_owned()),
        ];
        let (accepted, changes) = plan_host_cooldowns(Some(&stored), &incoming, now);
        assert_eq!(accepted.len(), 3);
        assert_eq!(
            (
                changes.kept_later_count,
                changes.unchanged,
                changes.added_count,
                changes.raised_count,
                changes.refused_future_count
            ),
            (1, 1, 1, 1, 1)
        );
        let tag = changes.tag(None, Some(2));
        let keys: Vec<_> = tag.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            [
                "route",
                "received",
                "unchanged",
                "added",
                "addedCount",
                "raised",
                "raisedCount",
                "keptLater",
                "keptLaterCount",
                "droppedNew",
                "refusedFuture",
                "refusedFutureCount",
                "version"
            ]
        );
        let full: Vec<(String, String)> = (0..MAX_STORED)
            .map(|i| (format!("c{i}"), "2026-10-31T00:00:00.000Z".to_owned()))
            .collect();
        let crowded = merge_cooldowns(None, &a, &full, now);
        let (_, changes) = plan_host_cooldowns(
            Some(&crowded),
            &[
                ("new".into(), "2026-10-31T00:00:00.000Z".into()),
                ("far".into(), "2027-01-01T00:00:00.000Z".into()),
            ],
            now,
        );
        assert_eq!((changes.dropped_new, changes.refused_future_count), (1, 1));
    }
}
