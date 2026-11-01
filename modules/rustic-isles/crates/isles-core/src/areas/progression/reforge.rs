//! The part of `controllers/reforge.ts` the XP route needs: how a grant relates to the weapon's reforge, and the mark
//! the grant's transaction makes. The reset, the banked restore plan and the stale repair are inventory-reforge's
//! (RU5 `inventory-reforge`); `classify_grant`, `ReforgeMode` and `stale_reforge_ms` are the pieces they share.

use serde_json::{Value, json};

use crate::AccountId;
use crate::http::Shared;
use crate::repo::progression::{ProgressionTrackRecord, WeaponReforge, now_iso};
use crate::repo::{RepoResult, Repositories, Tx};
use crate::settings::REFORGE;

use super::tracks::{experience_track_for_prestige, is_reforgeable_track};

/// A banked restore that arrives again after the record closed is a duplicate reforge RPC; well past the game's 0.5 s.
const DUPLICATE_RESTORE_WINDOW_MS: i64 = 15 * 60 * 1000;
/// An open record older than this was abandoned: the next DELETE or progression load finishes it.
pub const STALE_REFORGE_MS: i64 = 2 * 60 * 1000;

/// `REFORGE_MODE`: `RUSTIC_ISLES_REFORGE` (the env file, then the version's default switch) is `on`, `log` or `off`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReforgeMode {
    On,
    Log,
    Off,
}

pub fn reforge_mode(shared: &Shared) -> ReforgeMode {
    let value = shared
        .settings
        .raw(REFORGE.env)
        .map(|v| service_kit::js::trim(v).to_ascii_lowercase())
        .unwrap_or_else(|| {
            if shared.version.hooks.reforge_rules() {
                "on"
            } else {
                "off"
            }
            .to_owned()
        });
    match value.as_str() {
        "off" => ReforgeMode::Off,
        "log" => ReforgeMode::Log,
        _ => ReforgeMode::On,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReforgePlan {
    Normal,
    Prestige {
        experience_track: String,
        reforge: WeaponReforge,
    },
    Restore {
        experience_track: String,
        reforge: WeaponReforge,
    },
    Refuse(String),
}

impl ReforgePlan {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Prestige { .. } => "prestige",
            Self::Restore { .. } => "restore",
            Self::Refuse(_) => "refuse",
        }
    }
}

/// `ClassifyReforgeGrant`: how a grant relates to the weapon's reforge. `experience_row` is the weapon's
/// ExperienceTrack row (for a PrestigeTrack grant, the same weapon's).
pub fn classify_grant(
    mode: ReforgeMode,
    progression_id: &str,
    amount: i64,
    experience_row: Option<&ProgressionTrackRecord>,
    now_ms: i64,
) -> ReforgePlan {
    if mode != ReforgeMode::On {
        return ReforgePlan::Normal;
    }
    let reforge = experience_row.and_then(|row| row.reforge.as_ref());
    if let Some(experience_track) = experience_track_for_prestige(progression_id) {
        // The game grants a weapon's prestige track only from a reforge (step 4), always +1
        if let Some(open) = reforge.filter(|r| r.open && !r.prestige_granted && amount == 1) {
            return ReforgePlan::Prestige {
                experience_track,
                reforge: open.clone(),
            };
        }
        return ReforgePlan::Refuse(match reforge {
            Some(r) if r.open && amount == 1 => "this reforge's +1 was already granted".into(),
            Some(r) if r.open => format!("prestige amount {amount} is not 1"),
            _ => "no open reforge for this weapon".into(),
        });
    }
    if is_reforgeable_track(progression_id)
        && let Some(r) = reforge.filter(|r| r.banked > 0)
    {
        let remaining = r.banked - r.banked_restored;
        if r.open && remaining > 0 && amount == remaining {
            return ReforgePlan::Restore {
                experience_track: progression_id.to_owned(),
                reforge: r.clone(),
            };
        }
        let opened = service_kit::js::date_parse(&r.opened_at).unwrap_or(i64::MIN);
        if remaining == 0 && amount == r.banked && now_ms.saturating_sub(opened) < DUPLICATE_RESTORE_WINDOW_MS {
            return ReforgePlan::Refuse(format!(
                "banked {} of reforge {} was already restored",
                r.banked, r.reforge_id
            ));
        }
    }
    ReforgePlan::Normal
}

/// What `ReforgeGrantHook` asks of the repository inside the grant's transaction: the reforge and the compare-and-set
/// (`expected`, `set` as `updateReforge` takes them), or `None` for a plan that marks nothing.
pub fn grant_hook(plan: &ReforgePlan) -> Option<GrantHook> {
    let (kind_prestige, experience_track, r) = match plan {
        ReforgePlan::Prestige {
            experience_track,
            reforge,
        } => (true, experience_track, reforge),
        ReforgePlan::Restore {
            experience_track,
            reforge,
        } => (false, experience_track, reforge),
        _ => return None,
    };
    let expected =
        json!({ "status": "open", "prestigeGranted": r.prestige_granted, "bankedRestored": r.banked_restored });
    let mut set = if kind_prestige {
        json!({ "prestigeGranted": true })
    } else {
        json!({ "bankedRestored": r.banked })
    };
    let prestige_done = kind_prestige || r.prestige_granted;
    let banked_done = !kind_prestige || r.banked_restored >= r.banked;
    let closes = prestige_done && banked_done;
    if closes && let Some(object) = set.as_object_mut() {
        object.insert("status".into(), Value::from("closed"));
        object.insert("closedAt".into(), Value::from(now_iso()));
    }
    Some(GrantHook {
        experience_track: experience_track.clone(),
        reforge_id: r.reforge_id.clone(),
        expected,
        set,
        closes,
        prestige: kind_prestige,
        banked: r.banked,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct GrantHook {
    pub experience_track: String,
    pub reforge_id: String,
    pub expected: Value,
    pub set: Value,
    pub closes: bool,
    pub prestige: bool,
    pub banked: i64,
}

/// `ReforgeConflictError`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ReforgeConflict(pub String);

/// Applies a [`GrantHook`] inside the grant's transaction, or returns a [`ReforgeConflict`] if the reforge state
/// changed concurrently.
pub fn apply_grant_hook(
    tx: &mut Tx<'_>,
    repos: &Repositories,
    account: &AccountId,
    user_label: &str,
    hook: &GrantHook,
) -> RepoResult<Result<(), ReforgeConflict>> {
    let applied = repos.progression_tracks.update_reforge(
        tx,
        account,
        &hook.experience_track,
        &hook.reforge_id,
        &hook.expected,
        &hook.set,
    )?;
    let kind = if hook.prestige { "prestige" } else { "restore" };
    if !applied {
        return Ok(Err(ReforgeConflict(format!(
            "reforge {} changed concurrently ({kind})",
            hook.reforge_id
        ))));
    }
    tracing::info!(
        "[Reforge] {} user={user_label} track={} reforge={}{}{}",
        if hook.prestige { "PRESTIGE" } else { "BANKED" },
        hook.experience_track,
        hook.reforge_id,
        if hook.prestige {
            String::new()
        } else {
            format!(" restored={}", hook.banked)
        },
        if hook.closes { " CLOSE" } else { "" }
    );
    Ok(Ok(()))
}

/// `ReforgeGrantHook`, run inside the grant's transaction for a prestige or restore plan: marks the one-time use, or
/// refuses with a [`ReforgeConflict`]. Any other plan is a no-op.
pub fn mark_reforge_use(
    tx: &mut Tx<'_>,
    repos: &Repositories,
    account: &AccountId,
    plan: &ReforgePlan,
) -> RepoResult<Result<(), ReforgeConflict>> {
    let Some(hook) = grant_hook(plan) else {
        return Ok(Ok(()));
    };
    apply_grant_hook(tx, repos, account, &account.to_string(), &hook)
}

/// `PrestigeTrackFor`.
pub fn prestige_track_for(experience_track: &str) -> String {
    experience_track.replacen("ExperienceTrack_", "PrestigeTrack_", 1)
}

/// `FinishStaleReforge`: a reforge whose follow-ups never arrived gives the player what it owed (the banked XP back and
/// the +1) exactly once, then closes. `true` when this call closed it.
pub fn finish_stale_reforge(
    tx: &mut Tx<'_>,
    repos: &Repositories,
    account: &AccountId,
    user_id: &str,
    experience_track: &str,
    reforge: &WeaponReforge,
) -> RepoResult<bool> {
    let expected = json!({ "status": "open", "prestigeGranted": reforge.prestige_granted, "bankedRestored": reforge.banked_restored });
    let set =
        json!({ "status": "closed", "closedAt": now_iso(), "prestigeGranted": true, "bankedRestored": reforge.banked });
    let closed =
        repos
            .progression_tracks
            .update_reforge(tx, account, experience_track, &reforge.reforge_id, &expected, &set)?;
    if !closed {
        return Ok(false);
    }
    let owed = reforge.banked - reforge.banked_restored;
    if owed > 0 {
        repos
            .progression_tracks
            .increment(tx, account, experience_track, owed)?;
    }
    if !reforge.prestige_granted {
        repos
            .progression_tracks
            .increment(tx, account, &prestige_track_for(experience_track), 1)?;
    }
    tracing::warn!(
        "[Reforge] RECOVER user={user_id} track={experience_track} reforge={} stale since {}: restored {owed}, prestige {}",
        reforge.reforge_id,
        reforge.opened_at,
        if reforge.prestige_granted {
            "already granted"
        } else {
            "+1"
        }
    );
    Ok(true)
}

/// `FinishStaleReforgesForUser`: run on a game server's progression load. Returns the weapon tracks it finished.
pub fn finish_stale_reforges(
    tx: &mut Tx<'_>,
    repos: &Repositories,
    account: &AccountId,
    user_id: &str,
    now_ms: i64,
) -> RepoResult<Vec<String>> {
    let mut finished = Vec::new();
    for row in repos.progression_tracks.get_all_for_account(tx.conn(), account)? {
        let Some(reforge) = row.reforge.as_ref().filter(|r| r.open) else {
            continue;
        };
        if !is_reforgeable_track(&row.progression_id) {
            continue;
        }
        let opened = service_kit::js::date_parse(&reforge.opened_at).unwrap_or(i64::MIN);
        if now_ms.saturating_sub(opened) < STALE_REFORGE_MS {
            continue;
        }
        if finish_stale_reforge(tx, repos, account, user_id, &row.progression_id, reforge)? {
            finished.push(row.progression_id.clone());
        }
    }
    Ok(finished)
}

#[cfg(test)]
mod tests {
    use super::*;
    use module_api::NodeId;

    fn row(reforge: Option<WeaponReforge>) -> ProgressionTrackRecord {
        ProgressionTrackRecord {
            account: AccountId::new(&"n".parse::<NodeId>().unwrap()),
            progression_id: "ExperienceTrack_Weapon_Axe".into(),
            progress: 0,
            confirmed_fremium_rank: 0,
            confirmed_premium_rank: 0,
            update_version: 1,
            created_at: "2026-10-11T00:00:00.000Z".into(),
            updated_at: "2026-10-11T00:00:00.000Z".into(),
            reforge,
            prestige_claimed: None,
        }
    }

    fn open(prestige_granted: bool, banked_restored: i64) -> WeaponReforge {
        WeaponReforge {
            reforge_id: "r1".into(),
            open: true,
            prior_progress: 30_000,
            cap: 27_775,
            banked: 2_225,
            banked_restored,
            prestige_granted,
            opened_at: "2026-10-11T12:00:00.000Z".into(),
            closed_at: None,
        }
    }

    const NOW: i64 = 1_791_720_000_000; // 2026-10-11T12:00:00Z

    #[test]
    fn a_prestige_grant_needs_an_open_reforge_and_is_plus_one() {
        let held = row(Some(open(false, 0)));
        let plan = classify_grant(ReforgeMode::On, "PrestigeTrack_Weapon_Axe", 1, Some(&held), NOW);
        assert_eq!(plan.kind(), "prestige");
        let refused = |amount, held: Option<&ProgressionTrackRecord>| match classify_grant(
            ReforgeMode::On,
            "PrestigeTrack_Weapon_Axe",
            amount,
            held,
            NOW,
        ) {
            ReforgePlan::Refuse(reason) => reason,
            other => panic!("{other:?}"),
        };
        assert_eq!(refused(2, Some(&held)), "prestige amount 2 is not 1");
        assert_eq!(refused(1, None), "no open reforge for this weapon");
        let granted = row(Some(open(true, 0)));
        assert_eq!(refused(1, Some(&granted)), "this reforge's +1 was already granted");
        // with the rules off nothing is classified
        let off = classify_grant(ReforgeMode::Off, "PrestigeTrack_Weapon_Axe", 1, None, NOW);
        assert_eq!(off, ReforgePlan::Normal);
    }

    #[test]
    fn the_banked_restore_is_accepted_once_and_a_duplicate_refused() {
        let held = row(Some(open(false, 0)));
        assert_eq!(
            classify_grant(ReforgeMode::On, "ExperienceTrack_Weapon_Axe", 2_225, Some(&held), NOW).kind(),
            "restore"
        );
        assert_eq!(
            classify_grant(ReforgeMode::On, "ExperienceTrack_Weapon_Axe", 100, Some(&held), NOW).kind(),
            "normal"
        );
        let mut closed = open(true, 2_225);
        closed.open = false;
        let held = row(Some(closed));
        assert_eq!(
            classify_grant(ReforgeMode::On, "ExperienceTrack_Weapon_Axe", 2_225, Some(&held), NOW).kind(),
            "refuse"
        );
        // long after the reforge a same-sized grant is ordinary XP again
        let later = NOW + DUPLICATE_RESTORE_WINDOW_MS + 1;
        assert_eq!(
            classify_grant(ReforgeMode::On, "ExperienceTrack_Weapon_Axe", 2_225, Some(&held), later).kind(),
            "normal"
        );
    }

    #[test]
    fn the_hook_closes_the_reforge_when_both_follow_ups_are_done() {
        let plan = classify_grant(
            ReforgeMode::On,
            "PrestigeTrack_Weapon_Axe",
            1,
            Some(&row(Some(open(false, 0)))),
            NOW,
        );
        let hook = grant_hook(&plan).unwrap();
        assert!(!hook.closes);
        assert_eq!(hook.set, json!({ "prestigeGranted": true }));
        let plan = classify_grant(
            ReforgeMode::On,
            "PrestigeTrack_Weapon_Axe",
            1,
            Some(&row(Some(open(false, 2_225)))),
            NOW,
        );
        let hook = grant_hook(&plan).unwrap();
        assert!(hook.closes);
        assert_eq!(hook.set["status"], "closed");
        assert_eq!(
            hook.expected,
            json!({ "status": "open", "prestigeGranted": false, "bankedRestored": 2_225 })
        );
        assert!(grant_hook(&ReforgePlan::Normal).is_none());
    }
}
