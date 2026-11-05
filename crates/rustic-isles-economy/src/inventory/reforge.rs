// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The weapon reforge's reset and banked restore (spec §5): the reset stores a reforge record on the weapon's
//! ExperienceTrack row, and the banked restore is accepted once against it, marked inside the grant's transaction.

use std::sync::Arc;

use rustic_isles::Shared;
use rustic_isles_store::repo::progression::{ProgressionGrantRecord, ProgressionTrackRecord, WeaponReforge};
use rustic_isles_store::{AccountId, IslesStore, RepoError, Repositories, Tx};
use serde_json::Value;

use crate::inventory::transaction::player_actor;
use crate::progression::reforge::{
    ReforgeMode, ReforgePlan, classify_grant, finish_stale_reforge, is_stale, mark_reforge_use_as,
};
use crate::progression::tracks::{
    MAX_PROGRESSION_GRANT_AMOUNT, curves, grant_request_hash, is_reforgeable_track, is_valid_grant_id, track_cap_xp,
};
use crate::tx::guarded;
use crate::util::{iso, now_iso, number_value, obj, trunc_i64};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetOutcome {
    Disabled,
    NoTrack,
    Replay,
    StaleFinished,
    BelowCap,
    Reset,
    Contended,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResetResult {
    pub outcome: ResetOutcome,
    /// 200 or 409
    pub status: u16,
    pub row: Option<ProgressionTrackRecord>,
}

fn result(outcome: ResetOutcome, row: Option<ProgressionTrackRecord>) -> ResetResult {
    let status = if matches!(outcome, ResetOutcome::Replay | ResetOutcome::Reset) {
        200
    } else {
        409
    };
    ResetResult { outcome, status, row }
}

/// The reset (§5.2.1) at `now_ms`, for the account of `user_id`.
#[allow(clippy::too_many_arguments)]
pub async fn reforge_reset(
    shared: &Arc<Shared>,
    store: &IslesStore,
    mode: ReforgeMode,
    account: &AccountId,
    user_id: &str,
    track: &str,
    now_ms: i64,
    request_id: Option<&str>,
) -> Result<ResetResult, RepoError> {
    if mode != ReforgeMode::On {
        tracing::warn!(
            "[Reforge] reset of {track} for {user_id} refused: reforge mode is {}",
            mode.as_str()
        );
        return Ok(result(ResetOutcome::Disabled, None));
    }
    let cap = track_cap_xp(shared, track).map_or(0, trunc_i64);
    if cap <= 0 {
        return Ok(result(ResetOutcome::NoTrack, None));
    }
    let actor = player_actor(account, request_id);
    for _ in 0..4 {
        let (account_in, user, track_in) = (account.clone(), user_id.to_owned(), track.to_owned());
        let attempt = guarded(
            &store.clone(),
            Some(account),
            &actor,
            move |tx: &mut Tx<'_>, repos: &Repositories| {
                let account = account_in;
                let Some(row) = repos.progression_tracks.get(tx.conn(), &account, &track_in)? else {
                    return Ok(Some(result(ResetOutcome::NoTrack, None)));
                };
                if let Some(open) = row.reforge.as_ref().filter(|r| r.open) {
                    if !is_stale(open, now_ms) {
                        tracing::info!(
                            "[Reforge] REPLAY user={user} track={track_in} reforge={} (a duplicate reset)",
                            open.reforge_id
                        );
                        return Ok(Some(result(ResetOutcome::Replay, Some(row))));
                    }
                    let open = open.clone();
                    finish_stale_reforge(tx, repos, &account, &user, &track_in, &open)?;
                    let reread = repos.progression_tracks.get(tx.conn(), &account, &track_in)?;
                    return Ok(Some(result(ResetOutcome::StaleFinished, reread)));
                }
                if row.progress < cap {
                    tracing::info!(
                        "[Reforge] user={user} track={track_in} below the cap: progress={} cap={cap}",
                        row.progress
                    );
                    return Ok(Some(result(ResetOutcome::BelowCap, Some(row))));
                }
                let record = WeaponReforge {
                    reforge_id: rustic_isles::replay::random_uuid(),
                    open: true,
                    prior_progress: row.progress,
                    cap,
                    banked: row.progress - cap,
                    banked_restored: 0,
                    prestige_granted: false,
                    opened_at: iso(now_ms),
                    closed_at: None,
                };
                let begun = repos.progression_tracks.begin_reforge(
                    tx,
                    &account,
                    &track_in,
                    row.progress,
                    row.update_version,
                    &record,
                )?;
                Ok::<_, RepoError>(begun.map(|reset| {
                    tracing::info!(
                        "[Reforge] RESET user={user} track={track_in} reforge={} prior={} cap={cap} banked={}",
                        record.reforge_id,
                        record.prior_progress,
                        record.banked
                    );
                    result(ResetOutcome::Reset, Some(reset))
                }))
            },
        )
        .await?;
        if let Some(done) = attempt {
            return Ok(done);
        }
    }
    tracing::warn!("[Reforge] reset of {track} for {user_id} contended");
    Ok(result(ResetOutcome::Contended, None))
}

/// `{"phx_account_id","progression_id","progress","confirmed_fremium_rank","confirmed_premium_rank","confirmed_date"}`
pub fn single_track_payload(user_id: &str, track: &str, row: Option<&ProgressionTrackRecord>) -> Value {
    let (progress, free, premium, date) = match row {
        Some(r) => (
            r.progress,
            r.confirmed_fremium_rank,
            r.confirmed_premium_rank,
            r.updated_at.clone(),
        ),
        None => (0, 0, 0, now_iso()),
    };
    obj(vec![
        ("phx_account_id", Value::from(user_id)),
        ("progression_id", Value::from(track)),
        ("progress", Value::from(progress)),
        ("confirmed_fremium_rank", Value::from(free)),
        ("confirmed_premium_rank", Value::from(premium)),
        ("confirmed_date", Value::from(date)),
    ])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PurchasePlan {
    /// The banked restore, with its grant id and the plan to mark
    Restore {
        grant_id: String,
        plan: ReforgePlan,
    },
    /// A retry of an applied restore: the ledger replays it
    Normal {
        grant_id: String,
    },
    Refuse(String),
}

/// `PlanPurchaseRestore` (§5.3.1).
pub fn plan_purchase_restore(
    mode: ReforgeMode,
    track: &str,
    amount: f64,
    row: Option<&ProgressionTrackRecord>,
    now_ms: i64,
) -> PurchasePlan {
    if mode != ReforgeMode::On || !is_reforgeable_track(track) {
        return PurchasePlan::Refuse("not a reforge banked restore".into());
    }
    let Some(reforge) = row.and_then(|r| r.reforge.as_ref()) else {
        return PurchasePlan::Refuse("no reforge for this weapon".into());
    };
    let grant_id = format!("reforge-{}-banked", reforge.reforge_id);
    let plan = classify_grant(mode, track, amount, row, now_ms);
    if let ReforgePlan::Restore { .. } = plan {
        return PurchasePlan::Restore { grant_id, plan };
    }
    // Banked counters are far below 2^53
    #[allow(clippy::cast_precision_loss)]
    let banked = reforge.banked as f64;
    if reforge.banked_restored >= reforge.banked && amount == banked {
        return PurchasePlan::Normal { grant_id };
    }
    match plan {
        ReforgePlan::Refuse(reason) => PurchasePlan::Refuse(reason),
        _ => PurchasePlan::Refuse(format!(
            "amount {} is not the banked {}",
            service_kit::jsjson::number(amount),
            reforge.banked - reforge.banked_restored
        )),
    }
}

/// The banked restore's inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreGrant {
    pub grant_id: String,
    pub user_id: String,
    pub track: String,
    pub amount: i64,
    /// `Normal` marks nothing
    pub plan: ReforgePlan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantResult {
    pub record: ProgressionTrackRecord,
    pub replayed: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum GrantFailure {
    #[error("{0}")]
    ReforgeConflict(String),
    #[error("{0}")]
    GrantConflict(String),
    #[error("{0}")]
    Data(String),
    #[error(transparent)]
    Repo(#[from] RepoError),
}

/// The `beforeApply` path of `GrantProgressionXp` (§5.3.2): one account write, marked against the reforge.
pub async fn grant_banked_restore(
    shared: &Arc<Shared>,
    store: &IslesStore,
    account: &AccountId,
    grant: RestoreGrant,
    request_id: Option<&str>,
) -> Result<GrantResult, GrantFailure> {
    if !is_reforgeable_track(&grant.track) || !curves(shared).has(&grant.track) {
        return Err(GrantFailure::Data(format!(
            "Unsupported progression track '{}'.",
            grant.track
        )));
    }
    // An amount the route already bounded
    #[allow(clippy::cast_precision_loss)]
    let amount_f = grant.amount as f64;
    if !(grant.amount > 0 && amount_f <= MAX_PROGRESSION_GRANT_AMOUNT && is_valid_grant_id(&grant.grant_id)) {
        return Err(GrantFailure::Data("Invalid authoritative progression grant.".into()));
    }
    let hash = grant_request_hash(&grant.user_id, &grant.track, amount_f);
    let actor = player_actor(account, request_id);
    let account_in = account.clone();
    guarded(
        store,
        Some(account),
        &actor,
        move |tx: &mut Tx<'_>, repos: &Repositories| {
            let account = account_in;
            if let Some(existing) = repos.progression_grants.find_by_grant_id(tx.conn(), &grant.grant_id)? {
                if existing.request_hash != hash
                    || existing.account != account
                    || existing.progression_id != grant.track
                    || existing.amount != grant.amount
                {
                    return Err(GrantFailure::GrantConflict(format!(
                        "Progression grant {} was already used with a different request.",
                        grant.grant_id
                    )));
                }
                return Ok(GrantResult {
                    record: existing.result,
                    replayed: true,
                });
            }
            if let Err(conflict) = mark_reforge_use_as(tx, repos, &account, &grant.user_id, &grant.plan)? {
                return Err(GrantFailure::ReforgeConflict(conflict.0));
            }
            let before = repos
                .progression_tracks
                .get(tx.conn(), &account, &grant.track)?
                .map_or(0, |t| t.progress);
            let record = repos
                .progression_tracks
                .increment(tx, &account, &grant.track, grant.amount)?;
            repos.progression_grants.insert_applied(
                tx,
                &ProgressionGrantRecord {
                    grant_id: grant.grant_id.clone(),
                    account: account.clone(),
                    progression_id: grant.track.clone(),
                    amount: grant.amount,
                    request_hash: hash,
                    result: record.clone(),
                    created_at: now_iso(),
                },
            )?;
            tracing::info!(
                "[ProgressionGrant] grant={} user={} track={} delta={} old={before} new={} source=host-or-gameserver \
             applied=1",
                grant.grant_id,
                grant.user_id,
                grant.track,
                grant.amount,
                record.progress
            );
            Ok(GrantResult {
                record,
                replayed: false,
            })
        },
    )
    .await
}

/// The `amount` of a purchase tag, as a number value.
pub(crate) fn amount_value(amount: f64) -> Value {
    number_value(amount)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(banked: i64, restored: i64, granted: bool, open: bool) -> ProgressionTrackRecord {
        ProgressionTrackRecord {
            account: "account:01ARZ3NDEKTSV4RRFFQ69G5FAV@n".parse().unwrap(),
            progression_id: "ExperienceTrack_Weapon_Axe".into(),
            progress: 0,
            confirmed_fremium_rank: 0,
            confirmed_premium_rank: 0,
            update_version: 1,
            created_at: String::new(),
            updated_at: String::new(),
            reforge: Some(WeaponReforge {
                reforge_id: "abc".into(),
                open,
                prior_progress: 0,
                cap: 0,
                banked,
                banked_restored: restored,
                prestige_granted: granted,
                opened_at: "2026-11-01T00:00:00.000Z".into(),
                closed_at: None,
            }),
            prestige_claimed: None,
        }
    }

    #[test]
    fn purchase_plans() {
        let now = service_kit::js::date_parse("2026-11-01T00:01:00.000Z").unwrap();
        let on = ReforgeMode::On;
        let t = "ExperienceTrack_Weapon_Axe";
        assert_eq!(
            plan_purchase_restore(ReforgeMode::Log, t, 5.0, Some(&row(5, 0, false, true)), now),
            PurchasePlan::Refuse("not a reforge banked restore".into())
        );
        assert_eq!(
            plan_purchase_restore(on, "ExperienceTrack_PlayerLevel", 5.0, None, now),
            PurchasePlan::Refuse("not a reforge banked restore".into())
        );
        assert_eq!(
            plan_purchase_restore(on, t, 5.0, None, now),
            PurchasePlan::Refuse("no reforge for this weapon".into())
        );
        assert!(
            matches!(plan_purchase_restore(on, t, 5.0, Some(&row(5, 0, false, true)), now), PurchasePlan::Restore { ref grant_id, .. } if grant_id == "reforge-abc-banked")
        );
        // A retry after the restore: within 15 minutes the classification refuses, but the ledger replays
        assert_eq!(
            plan_purchase_restore(on, t, 5.0, Some(&row(5, 5, true, false)), now),
            PurchasePlan::Normal {
                grant_id: "reforge-abc-banked".into()
            }
        );
        assert_eq!(
            plan_purchase_restore(on, t, 4.0, Some(&row(5, 0, false, true)), now),
            PurchasePlan::Refuse("amount 4 is not the banked 5".into())
        );
    }
}
