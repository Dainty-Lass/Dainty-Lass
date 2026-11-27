// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! One player's documents out of a Ramsgate Reborn MongoDB (`docs/port/spec-legacy-ramsgate.md` sections 3 and 4),
//! read-only: the client sends `hello`, authentication, `find` and `getMore` and nothing else.

use rustic_isles_db::mongo::{MongoClient, MongoError};
use serde_json::{Map, Value, json};

use super::transform::{Doc, RrSave};

/// Discord's `provider` in RR's `authIdentities` (spec Q1: what the live sign-in writes, unconfirmed).
pub const DISCORD: &str = "discord";

/// Who to read: an old account named by a claim, or a Discord user id to look up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Who {
    /// The RR user id a redeemed claim names.
    User(String),
    /// A Discord user id (snowflake).
    Discord(String),
}

/// The account's state (section 3), and the save.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// The RR user id.
    pub user: String,
    /// Why the account may not be carried: a ban in force, `disabled`, or approval refused.
    pub barred: Option<&'static str>,
    /// Section 4's collections.
    pub save: RrSave,
}

fn filter(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

fn text(doc: &Doc, key: &str) -> Option<String> {
    doc.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// Section 3's bans and approval: a ban is in force while `banUntil` is absent, unparsable or ahead of `now_ms`.
pub fn barred(account: &Doc, now_ms: i64) -> Option<&'static str> {
    match account.get("status").and_then(Value::as_str) {
        Some("banned") => {
            let lifted = text(account, "banUntil")
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(&t).ok())
                .is_some_and(|t| t.timestamp_millis() <= now_ms);
            if !lifted {
                return Some("banned on Ramsgate Reborn");
            }
        }
        Some("disabled") => return Some("disabled on Ramsgate Reborn"),
        _ => {}
    }
    match account.get("approvalStatus").and_then(Value::as_str) {
        Some("pending" | "rejected") => Some("not approved on Ramsgate Reborn"),
        _ => None,
    }
}

/// The player's save, or none when RR has no such account. Blocking.
pub fn fetch(client: &mut MongoClient, who: &Who, now_ms: i64) -> Result<Option<Found>, MongoError> {
    let user = match who {
        Who::User(user) => user.clone(),
        Who::Discord(subject) => {
            let links = client.find_matching(
                "authIdentities",
                &filter(json!({"provider": DISCORD, "providerSubject": subject})),
            )?;
            // Unique by index; more than one would be a broken database, so none is guessed
            let [link] = links.as_slice() else {
                return Ok(None);
            };
            let Some(user) = text(link, "userId") else {
                return Ok(None);
            };
            user
        }
    };
    let accounts = client.find_matching("accounts", &filter(json!({"_id": user})))?;
    let Some(account) = accounts.first() else {
        return Ok(None);
    };
    let by_user = filter(json!({"userId": user}));
    let by_id = filter(json!({"_id": user}));
    let mut save = RrSave::new();
    let characters = client.find_matching("characters", &by_user)?;
    let ids: Vec<Value> = characters
        .iter()
        .filter_map(|d| text(d, "_id"))
        .map(Value::String)
        .collect();
    save.insert("characters".into(), characters);
    if !ids.is_empty() {
        let in_k = filter(json!({"_id": {"$in": ids}}));
        for name in ["inventories", "loadouts", "breadcrumbs", "encounteredContent"] {
            save.insert(name.into(), client.find_matching(name, &in_k)?);
        }
    }
    for name in ["wallets", "playerJourney", "bountyDrafts", "playtime"] {
        save.insert(name.into(), client.find_matching(name, &by_id)?);
    }
    for name in [
        "progressionTracks",
        "progressionObjectives",
        "escalationProgress",
        "bounties",
        "cooldowns",
        "trackedObjectives",
    ] {
        save.insert(name.into(), client.find_matching(name, &by_user)?);
    }
    Ok(Some(Found {
        barred: barred(account, now_ms),
        user,
        save,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bans_bar_until_they_run_out() {
        let now = 1_795_000_000_000;
        let account = |v: Value| filter(v);
        assert_eq!(barred(&account(json!({"status": "active"})), now), None);
        assert_eq!(barred(&account(json!({})), now), None, "absent approval is approved");
        assert!(barred(&account(json!({"status": "banned"})), now).is_some());
        assert!(
            barred(
                &account(json!({"status": "banned", "banUntil": "2099-01-01T00:00:00.000Z"})),
                now
            )
            .is_some()
        );
        assert!(barred(&account(json!({"status": "banned", "banUntil": "junk"})), now).is_some());
        assert_eq!(
            barred(
                &account(json!({"status": "banned", "banUntil": "2020-01-01T00:00:00.000Z"})),
                now
            ),
            None
        );
        assert!(barred(&account(json!({"status": "disabled"})), now).is_some());
        assert!(barred(&account(json!({"approvalStatus": "pending"})), now).is_some());
    }
}
