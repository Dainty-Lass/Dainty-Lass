// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The XP route's half of the weapon reforge (progression spec §4.5, §4.6; inventory's P1 to P5): which grants a
//! stored reforge record allows, the compare-and-set marks, and the stale-reforge repair.

use dainty_service_kit::js;
use rustic_isles::Shared;
use rustic_isles_store::repo::progression::{ProgressionTrackRecord, WeaponReforge};
use rustic_isles_store::{AccountId, RepoResult, Repositories, Tx};
use serde_json::{Map, Value};

use crate::progression::tracks::{
    experience_track_for_prestige, is_prestige_track, is_reforgeable_track, prestige_track_for,
};
use crate::util::now_iso;

/// P5
pub const STALE_REFORGE_MS: i64 = 120_000;
const DUPLICATE_WINDOW_MS: i64 = 15 * 60_000;

/// `RUSTIC_ISLES_REFORGE`: whether the reforge rules run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReforgeMode {
    /// The rules run.
    On,
    /// The rules do not run, as with `off`, but the log names the mode `log`.
    Log,
    /// The rules do not run: grants pass as normal and a reset is refused.
    Off,
}

impl ReforgeMode {
    /// The setting's text.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Log => "log",
            Self::Off => "off",
        }
    }
}

/// P1: `RUSTIC_ISLES_REFORGE` JS-trimmed and ASCII lower-cased (`off`, `log`, anything else `on`); unset follows the
/// version's `reforge_rules`.
pub fn reforge_mode(shared: &Shared) -> ReforgeMode {
    match shared.settings().raw("RUSTIC_ISLES_REFORGE") {
        None => {
            if shared.version().hooks.reforge_rules() {
                ReforgeMode::On
            } else {
                ReforgeMode::Off
            }
        }
        Some(raw) => match js::trim(raw).to_ascii_lowercase().as_str() {
            "off" => ReforgeMode::Off,
            "log" => ReforgeMode::Log,
            _ => ReforgeMode::On,
        },
    }
}

/// What an XP grant may do against the weapon's reforge record (P2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReforgePlan {
    /// An ordinary grant.
    Normal,
    /// The reforge's one +1 on the prestige track.
    Prestige {
        /// The weapon's experience track.
        experience_track: String,
        /// The open reforge, as read.
        reforge: WeaponReforge,
    },
    /// The banked XP returned to the experience track.
    Restore {
        /// The weapon's experience track.
        experience_track: String,
        /// The open reforge, as read.
        reforge: WeaponReforge,
    },
    /// Refused; the text says why.
    Refuse(String),
}

impl ReforgePlan {
    /// The plan's name, for the log.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Prestige { .. } => "prestige",
            Self::Restore { .. } => "restore",
            Self::Refuse(_) => "refuse",
        }
    }

    /// The reforge a prestige or restore plan marks.
    pub fn reforge(&self) -> Option<&WeaponReforge> {
        match self {
            Self::Prestige { reforge, .. } | Self::Restore { reforge, .. } => Some(reforge),
            _ => None,
        }
    }
}

/// An unparseable `openedAt` reads as long ago.
fn opened_ms(reforge: &WeaponReforge) -> Option<i64> {
    js::date_parse(&reforge.opened_at)
}

/// Whether the reforge was opened at least [`STALE_REFORGE_MS`] ago, or its time is unreadable.
pub fn is_stale(reforge: &WeaponReforge, now_ms: i64) -> bool {
    opened_ms(reforge).is_none_or(|opened| now_ms - opened >= STALE_REFORGE_MS)
}

/// P2: what a grant of `amount` to `progression_id` may do against the weapon's ExperienceTrack row.
pub fn classify_grant(
    mode: ReforgeMode,
    progression_id: &str,
    amount: f64,
    row: Option<&ProgressionTrackRecord>,
    now_ms: i64,
) -> ReforgePlan {
    if mode != ReforgeMode::On {
        return ReforgePlan::Normal;
    }
    let reforge = row.and_then(|r| r.reforge.as_ref());
    if let Some(experience_track) = experience_track_for_prestige(progression_id) {
        return match reforge.filter(|r| r.open) {
            Some(r) if amount == 1.0 && !r.prestige_granted => ReforgePlan::Prestige {
                experience_track,
                reforge: r.clone(),
            },
            Some(_) if amount == 1.0 => ReforgePlan::Refuse("this reforge's +1 was already granted".into()),
            Some(_) => ReforgePlan::Refuse(format!(
                "prestige amount {} is not 1",
                dainty_service_kit::jsjson::number(amount)
            )),
            None => ReforgePlan::Refuse("no open reforge for this weapon".into()),
        };
    }
    if is_reforgeable_track(progression_id)
        && let Some(r) = reforge.filter(|r| r.banked > 0)
    {
        let remaining = r.banked - r.banked_restored;
        #[allow(clippy::cast_precision_loss, reason = "reforge counters are far below 2^53")]
        let (remaining_f, banked_f) = (remaining as f64, r.banked as f64);
        if r.open && remaining > 0 && amount == remaining_f {
            return ReforgePlan::Restore {
                experience_track: progression_id.to_owned(),
                reforge: r.clone(),
            };
        }
        let recent = opened_ms(r).is_some_and(|opened| now_ms - opened < DUPLICATE_WINDOW_MS);
        if remaining <= 0 && amount == banked_f && recent {
            return ReforgePlan::Refuse(format!(
                "banked {} of reforge {} was already restored",
                r.banked, r.reforge_id
            ));
        }
    }
    ReforgePlan::Normal
}

/// The mark lost a compare-and-set race; its display is the message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ReforgeConflict(pub String);

fn expected(reforge: &WeaponReforge) -> Value {
    crate::util::obj(vec![
        ("status", Value::from("open")),
        ("prestigeGranted", Value::Bool(reforge.prestige_granted)),
        ("bankedRestored", Value::from(reforge.banked_restored)),
    ])
}

/// P3: records the plan's follow-up on the reforge (nothing for `Normal` or `Refuse`); `label` names the user in the
/// log line.
pub fn mark_reforge_use(
    tx: &mut Tx<'_>,
    repos: &Repositories,
    account: &AccountId,
    plan: &ReforgePlan,
) -> RepoResult<Result<(), ReforgeConflict>> {
    mark_reforge_use_as(tx, repos, account, &account.to_string(), plan)
}

pub(crate) fn mark_reforge_use_as(
    tx: &mut Tx<'_>,
    repos: &Repositories,
    account: &AccountId,
    label: &str,
    plan: &ReforgePlan,
) -> RepoResult<Result<(), ReforgeConflict>> {
    let (experience_track, reforge, prestige) = match plan {
        ReforgePlan::Prestige {
            experience_track,
            reforge,
        } => (experience_track, reforge, true),
        ReforgePlan::Restore {
            experience_track,
            reforge,
        } => (experience_track, reforge, false),
        _ => return Ok(Ok(())),
    };
    let mut set = Map::new();
    let closes = if prestige {
        set.insert("prestigeGranted".into(), Value::Bool(true));
        reforge.banked_restored >= reforge.banked
    } else {
        set.insert("bankedRestored".into(), Value::from(reforge.banked));
        reforge.prestige_granted
    };
    if closes {
        set.insert("status".into(), Value::from("closed"));
        set.insert("closedAt".into(), Value::from(now_iso()));
    }
    let applied = repos.progression_tracks.update_reforge(
        tx,
        account,
        experience_track,
        &reforge.reforge_id,
        &expected(reforge),
        &Value::Object(set),
    )?;
    let word = if prestige { "prestige" } else { "restore" };
    if !applied {
        return Ok(Err(ReforgeConflict(format!(
            "reforge {} changed concurrently ({word})",
            reforge.reforge_id
        ))));
    }
    tracing::info!(
        "[Reforge] {} user={label} track={experience_track} reforge={}{}{}",
        if prestige { "PRESTIGE" } else { "BANKED" },
        reforge.reforge_id,
        if prestige {
            String::new()
        } else {
            format!(" restored={}", reforge.banked)
        },
        if closes { " CLOSE" } else { "" }
    );
    Ok(Ok(()))
}

/// P4: closes an open reforge with every follow-up marked done, then applies what the game never sent.
pub fn finish_stale_reforge(
    tx: &mut Tx<'_>,
    repos: &Repositories,
    account: &AccountId,
    user_id: &str,
    experience_track: &str,
    reforge: &WeaponReforge,
) -> RepoResult<bool> {
    let set = crate::util::obj(vec![
        ("status", Value::from("closed")),
        ("closedAt", Value::from(now_iso())),
        ("prestigeGranted", Value::Bool(true)),
        ("bankedRestored", Value::from(reforge.banked)),
    ]);
    let applied = repos.progression_tracks.update_reforge(
        tx,
        account,
        experience_track,
        &reforge.reforge_id,
        &expected(reforge),
        &set,
    )?;
    if !applied {
        return Ok(false);
    }
    let restored = reforge.banked - reforge.banked_restored;
    if restored > 0 {
        repos
            .progression_tracks
            .increment(tx, account, experience_track, restored)?;
    }
    if !reforge.prestige_granted {
        repos
            .progression_tracks
            .increment(tx, account, &prestige_track_for(experience_track), 1)?;
    }
    tracing::warn!(
        "[Reforge] RECOVER user={user_id} track={experience_track} reforge={} stale since {}: restored {}, prestige {}",
        reforge.reforge_id,
        reforge.opened_at,
        restored.max(0),
        if reforge.prestige_granted {
            "already granted"
        } else {
            "+1"
        }
    );
    Ok(true)
}

/// The list form: every stale open reforge of the account, in creation order; the tracks it closed.
pub fn finish_stale_reforges(
    tx: &mut Tx<'_>,
    repos: &Repositories,
    account: &AccountId,
    user_id: &str,
    now_ms: i64,
) -> RepoResult<Vec<String>> {
    let mut closed = Vec::new();
    for track in repos.progression_tracks.get_all_for_account(tx.conn(), account)? {
        let Some(reforge) = track.reforge.as_ref().filter(|r| r.open) else {
            continue;
        };
        if !is_reforgeable_track(&track.progression_id) || !is_stale(reforge, now_ms) {
            continue;
        }
        if finish_stale_reforge(tx, repos, account, user_id, &track.progression_id, reforge)? {
            closed.push(track.progression_id.clone());
        }
    }
    Ok(closed)
}

/// Re-exported for the XP route's checks.
pub fn is_reforge_track(id: &str) -> bool {
    is_reforgeable_track(id) || is_prestige_track(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reforge(open: bool, banked: i64, restored: i64, granted: bool) -> WeaponReforge {
        WeaponReforge {
            reforge_id: "r1".into(),
            open,
            prior_progress: 1000 + banked,
            cap: 1000,
            banked,
            banked_restored: restored,
            prestige_granted: granted,
            opened_at: "2026-11-01T00:00:00.000Z".into(),
            closed_at: None,
        }
    }

    fn row(r: WeaponReforge) -> ProgressionTrackRecord {
        ProgressionTrackRecord {
            account: "account:01ARZ3NDEKTSV4RRFFQ69G5FAV@n".parse().unwrap(),
            progression_id: "ExperienceTrack_Weapon_Axe".into(),
            progress: 0,
            confirmed_fremium_rank: 0,
            confirmed_premium_rank: 0,
            update_version: 1,
            created_at: String::new(),
            updated_at: String::new(),
            reforge: Some(r),
            prestige_claimed: None,
        }
    }

    #[test]
    fn plans() {
        let opened = js::date_parse("2026-11-01T00:00:00.000Z").unwrap();
        let on = ReforgeMode::On;
        let p = "PrestigeTrack_Weapon_Axe";
        let e = "ExperienceTrack_Weapon_Axe";
        assert_eq!(
            classify_grant(on, p, 1.0, Some(&row(reforge(true, 50, 0, false))), opened).kind(),
            "prestige"
        );
        assert_eq!(
            classify_grant(on, p, 1.0, Some(&row(reforge(true, 50, 0, true))), opened),
            ReforgePlan::Refuse("this reforge's +1 was already granted".into())
        );
        assert_eq!(
            classify_grant(on, p, 2.0, Some(&row(reforge(true, 50, 0, false))), opened),
            ReforgePlan::Refuse("prestige amount 2 is not 1".into())
        );
        assert_eq!(
            classify_grant(on, p, 1.0, None, opened),
            ReforgePlan::Refuse("no open reforge for this weapon".into())
        );
        assert_eq!(
            classify_grant(on, e, 50.0, Some(&row(reforge(true, 50, 0, false))), opened).kind(),
            "restore"
        );
        assert_eq!(
            classify_grant(on, e, 49.0, Some(&row(reforge(true, 50, 0, false))), opened).kind(),
            "normal"
        );
        assert_eq!(
            classify_grant(on, e, 50.0, Some(&row(reforge(false, 50, 50, true))), opened + 60_000),
            ReforgePlan::Refuse("banked 50 of reforge r1 was already restored".into())
        );
        assert_eq!(
            classify_grant(
                on,
                e,
                50.0,
                Some(&row(reforge(false, 50, 50, true))),
                opened + DUPLICATE_WINDOW_MS
            )
            .kind(),
            "normal"
        );
        assert_eq!(classify_grant(ReforgeMode::Off, p, 1.0, None, opened).kind(), "normal");
        assert!(is_stale(&reforge(true, 0, 0, false), opened + STALE_REFORGE_MS));
        assert!(!is_stale(&reforge(true, 0, 0, false), opened + STALE_REFORGE_MS - 1));
    }
}
