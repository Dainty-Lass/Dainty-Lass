//! Weapon reforge (weapon prestige), 1.14.7: `controllers/reforge.ts`.
//!
//! `UPlayerExperienceComponent::ServerPrestigeExperience_Implementation`, in order: pays the cost (`POST /inventory`),
//! `DELETE /progression/{acct}/ExperienceTrack_Weapon_X` (the reset: the game stops unless it answers 200-206 with a
//! JSON object), reads the track back, `POST .../PrestigeTrack_Weapon_X/1` once per reforge, 0.5 s later
//! `POST /progression/purchase/{acct}/ExperienceTrack_Weapon_X/{banked}` (progress minus the cap), then the
//! Aetherheart and the rank confirm. The game guards none of that against a duplicate RPC, so the backend does: the
//! reset stores a reforge record on the weapon's ExperienceTrack row, and the +1 and the banked restore are each
//! accepted once against it, marked inside the grant's own transaction. A repeated DELETE while the record is open
//! replays without resetting again. `MYSTICPARADOX_REFORGE=off` answers the DELETE 409 and `=log` does the same and
//! logs the request; the `reforge_rules` hook is false on 1.12.0, which has none of this.
//!
//! The classification, the mark and the stale repair are the progression area's (`areas::progression::reforge`), which
//! its XP route and `GET /progression/:userId` call; this file has the reset, the banked restore and the grant that
//! carries it.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use service_kit::js;

use super::clock::{iso, iso_now};
use super::progression_config::ProgressionConfig;
use super::transaction::{TxContext, TxError, player_actor};
pub use crate::areas::progression::reforge::{ReforgeConflict, mark_reforge_use};
use crate::areas::progression::reforge::{
    ReforgeMode, ReforgePlan, STALE_REFORGE_MS, classify_grant, finish_stale_reforge, reforge_mode,
};
use crate::areas::progression::tracks::{
    MAX_PROGRESSION_GRANT_AMOUNT, grant_request_hash, is_reforgeable_track, is_valid_grant_id, track_cap_xp,
};
use crate::http::Shared;
use crate::repo::progression::{ProgressionGrantRecord, ProgressionTrackRecord, WeaponReforge};
use crate::repo::{AccountId, RepoError, RepoResult};

const PROGRESSION_FILE: &str = "progression_config.json";

/// `nowMs - Date.parse(openedAt) < window`, false for a date JavaScript can't read (NaN).
fn opened_within(now_ms: i64, opened_at: &str, window_ms: i64) -> bool {
    js::date_parse(opened_at).is_some_and(|opened| now_ms - opened < window_ms)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetOutcome {
    Reset,
    Replay,
    BelowCap,
    NoTrack,
    Disabled,
    Contended,
    StaleFinished,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResetResult {
    pub status: u16,
    pub outcome: ResetOutcome,
    pub record: Option<ProgressionTrackRecord>,
}

fn refused(outcome: ResetOutcome, record: Option<ProgressionTrackRecord>) -> ResetResult {
    ResetResult {
        status: 409,
        outcome,
        record,
    }
}

enum Attempt {
    Done(Box<ResetResult>),
    Retry,
}

/// `IsPrestigeGatedReward` for the module's game data; a missing file means nothing is gated.
pub fn is_prestige_gated_reward(shared: &Shared, catalog_id: &str) -> bool {
    shared
        .game_data
        .backend_file(PROGRESSION_FILE)
        .and_then(|path| ProgressionConfig::load(&path).ok())
        .is_some_and(|config| config.is_prestige_gated_reward(catalog_id))
}

/// `ReforgeReset`: the game's DELETE on a weapon's ExperienceTrack, resetting it for a reforge. Compare-and-set on the
/// row, so a retried or duplicate DELETE never resets twice: while the reforge is open it replays (200, unchanged).
pub async fn reforge_reset(
    ctx: &TxContext<'_>,
    account: &AccountId,
    user_id: &str,
    track: &str,
    at_ms: i64,
    request_id: Option<&str>,
) -> Result<ResetResult, TxError> {
    let mode = reforge_mode(ctx.shared);
    if mode != ReforgeMode::On {
        tracing::warn!("[Reforge] DELETE-REQ user={account} track={track} refused: MYSTICPARADOX_REFORGE={mode:?}");
        return Ok(refused(ResetOutcome::Disabled, None));
    }
    let cap = track_cap_xp(ctx.shared, track)
        .filter(|cap| *cap > 0.0)
        .map(|cap| cap as i64);
    let Some(cap) = cap else {
        return Ok(refused(ResetOutcome::NoTrack, None));
    };
    let repos = ctx.repos();
    let actor = player_actor(account, request_id);
    for _ in 0..4 {
        let repos = repos.clone();
        let account_in = account.clone();
        let track_in = track.to_owned();
        let user_in = user_id.to_owned();
        let attempt = ctx
            .store
            .account_write(account, &actor, move |tx, _| -> RepoResult<Attempt> {
                let tracks = &repos.progression_tracks;
                let Some(row) = tracks.get(tx.conn(), &account_in, &track_in)? else {
                    return Ok(Attempt::Done(Box::new(refused(ResetOutcome::NoTrack, None))));
                };
                if let Some(reforge) = row.reforge.as_ref().filter(|r| r.open) {
                    if opened_within(at_ms, &reforge.opened_at, STALE_REFORGE_MS) {
                        tracing::info!(
                            "[Reforge] REPLAY user={account_in} track={track_in} reforge={} (open; not reset again)",
                            reforge.reforge_id
                        );
                        return Ok(Attempt::Done(Box::new(ResetResult {
                            status: 200,
                            outcome: ResetOutcome::Replay,
                            record: Some(row),
                        })));
                    }
                    finish_stale_reforge(tx, &repos, &account_in, &user_in, &track_in, reforge)?;
                    let record = repos.progression_tracks.get(tx.conn(), &account_in, &track_in)?;
                    return Ok(Attempt::Done(Box::new(refused(ResetOutcome::StaleFinished, record))));
                }
                if row.progress < cap {
                    tracing::warn!(
                        "[Reforge] REFUSED user={account_in} track={track_in} progress={} below the level-20 cap {cap}",
                        row.progress
                    );
                    return Ok(Attempt::Done(Box::new(refused(ResetOutcome::BelowCap, Some(row)))));
                }
                let reforge = WeaponReforge {
                    reforge_id: uuid::Uuid::new_v4().to_string(),
                    open: true,
                    prior_progress: row.progress,
                    cap,
                    banked: row.progress - cap,
                    banked_restored: 0,
                    prestige_granted: false,
                    opened_at: iso(at_ms),
                    closed_at: None,
                };
                match tracks.begin_reforge(tx, &account_in, &track_in, row.progress, row.update_version, &reforge)? {
                    Some(after) => {
                        tracing::info!(
                            "[Reforge] RESET user={account_in} track={track_in} reforge={} prior={} cap={cap} banked={}",
                            reforge.reforge_id,
                            reforge.prior_progress,
                            reforge.banked
                        );
                        Ok(Attempt::Done(Box::new(ResetResult {
                            status: 200,
                            outcome: ResetOutcome::Reset,
                            record: Some(after),
                        })))
                    }
                    None => Ok(Attempt::Retry),
                }
            })
            .await?;
        match attempt {
            Attempt::Done(result) => {
                let result = *result;
                if result.outcome == ResetOutcome::Reset {
                    ctx.ports.observer.reforge_reset(ctx.shared, account, at_ms);
                }
                return Ok(result);
            }
            Attempt::Retry => {}
        }
    }
    tracing::warn!("[Reforge] REFUSED user={account} track={track}: the row kept changing (contended)");
    Ok(refused(ResetOutcome::Contended, None))
}

#[derive(Debug, Clone, PartialEq)]
pub struct PurchasePlan {
    pub plan: ReforgePlan,
    pub grant_id: Option<String>,
}

/// `PlanPurchaseRestore`: the banked re-grant arrives as `POST /progression/purchase/{acct}/{track}/{amount}` with no
/// grant id. It is accepted only as the open reforge's banked restore, with a grant id derived from the reforge so a
/// retry replays through the grant ledger.
pub fn plan_purchase_restore(
    progression_id: &str,
    amount: i64,
    experience_row: Option<&ProgressionTrackRecord>,
    now_ms: i64,
    mode: ReforgeMode,
) -> PurchasePlan {
    let refuse = |reason: String| PurchasePlan {
        plan: ReforgePlan::Refuse(reason),
        grant_id: None,
    };
    if mode != ReforgeMode::On || !is_reforgeable_track(progression_id) {
        return refuse("not a reforge banked restore".into());
    }
    let Some(reforge) = experience_row.and_then(|row| row.reforge.as_ref()) else {
        return refuse("no reforge for this weapon".into());
    };
    let grant_id = format!("reforge-{}-banked", reforge.reforge_id);
    let plan = classify_grant(mode, progression_id, amount, experience_row, now_ms);
    if matches!(plan, ReforgePlan::Restore { .. }) {
        return PurchasePlan {
            plan,
            grant_id: Some(grant_id),
        };
    }
    // Already restored by this same reforge: let the ledger replay it rather than refusing a retry
    if reforge.banked_restored >= reforge.banked && amount == reforge.banked {
        return PurchasePlan {
            plan: ReforgePlan::Normal,
            grant_id: Some(grant_id),
        };
    }
    PurchasePlan {
        plan: ReforgePlan::Refuse(match plan {
            ReforgePlan::Refuse(reason) => reason,
            _ => format!(
                "amount {} is not the banked {}",
                amount,
                reforge.banked - reforge.banked_restored
            ),
        }),
        grant_id: None,
    }
}

/// Why a banked restore's grant didn't apply.
#[derive(Debug, thiserror::Error)]
pub enum GrantFailure {
    /// `ReforgeConflictError`
    #[error(transparent)]
    Reforge(#[from] ReforgeConflict),
    /// `ProgressionGrantConflictError`: the grant id was used for another request
    #[error("Progression grant {0} was already used with a different request.")]
    GrantConflict(String),
    /// An error Node doesn't catch (500)
    #[error(transparent)]
    Tx(#[from] TxError),
}

impl From<RepoError> for GrantFailure {
    fn from(error: RepoError) -> Self {
        Self::Tx(error.into())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GrantResult {
    pub record: ProgressionTrackRecord,
    pub replayed: bool,
}

enum GrantStep {
    Done(Box<GrantResult>),
    Failed(GrantFailure),
}

/// One banked restore to apply.
pub struct RestoreGrant<'a> {
    pub account: &'a AccountId,
    pub user_id: &'a str,
    pub progression_id: &'a str,
    pub amount: i64,
    pub grant_id: &'a str,
    pub plan: ReforgePlan,
}

/// `GrantProgressionXp` for the banked restore: applies the grant once, with `plan`'s one-time mark inside the same
/// transaction as the XP and its ledger row. The progression area's XP route has its own; this is that function's
/// `beforeApply` path, kept here so the reforge doesn't depend on it.
pub async fn grant_banked_restore(
    ctx: &TxContext<'_>,
    grant: RestoreGrant<'_>,
    request_id: Option<&str>,
) -> Result<GrantResult, GrantFailure> {
    let RestoreGrant {
        account,
        user_id,
        progression_id,
        amount,
        grant_id,
        plan,
    } = grant;
    // Only an approved track with a rank curve takes a grant; a reforge's weapon track always was
    if !is_reforgeable_track(progression_id) || track_cap_xp(ctx.shared, progression_id).is_none() {
        return Err(TxError::Repo(RepoError::Data(format!(
            "Unsupported progression track '{progression_id}'."
        )))
        .into());
    }
    if amount <= 0 || amount > MAX_PROGRESSION_GRANT_AMOUNT || !is_valid_grant_id(grant_id) {
        return Err(TxError::Repo(RepoError::Data("Invalid authoritative progression grant.".into())).into());
    }
    let request_hash = grant_request_hash(user_id, progression_id, amount);
    let repos = ctx.repos();
    let (account_in, track, grant, hash) = (
        account.clone(),
        progression_id.to_owned(),
        grant_id.to_owned(),
        request_hash,
    );
    let actor = player_actor(account, request_id);
    let failure: Arc<Mutex<Option<GrantFailure>>> = Arc::default();
    let failure_in = failure.clone();
    let stored = ctx
        .store
        .account_write(account, &actor, move |tx, _| -> RepoResult<GrantStep> {
            let fail = |error: GrantFailure| {
                if let Ok(mut slot) = failure_in.lock() {
                    slot.get_or_insert(error);
                }
                RepoError::Data("the grant was rolled back".into())
            };
            if let Some(existing) = repos.progression_grants.find_by_grant_id(tx.conn(), &grant)? {
                if existing.request_hash != hash
                    || existing.account != account_in
                    || existing.progression_id != track
                    || existing.amount != amount
                {
                    return Ok(GrantStep::Failed(GrantFailure::GrantConflict(grant.clone())));
                }
                return Ok(GrantStep::Done(Box::new(GrantResult {
                    record: existing.result,
                    replayed: true,
                })));
            }
            if let Err(conflict) = mark_reforge_use(tx, &repos, &account_in, &plan)? {
                return Err(fail(conflict.into()));
            }
            let before = repos.progression_tracks.get(tx.conn(), &account_in, &track)?;
            let after = repos.progression_tracks.increment(tx, &account_in, &track, amount)?;
            repos.progression_grants.insert_applied(
                tx,
                &ProgressionGrantRecord {
                    grant_id: grant.clone(),
                    account: account_in.clone(),
                    progression_id: track.clone(),
                    amount,
                    request_hash: hash.clone(),
                    result: after.clone(),
                    created_at: iso_now(),
                },
            )?;
            tracing::info!(
                "[ProgressionGrant] grant={grant} user={account_in} track={track} delta={amount} old={} new={} source=host-or-gameserver applied=1",
                before.map_or(0, |b| b.progress),
                after.progress
            );
            Ok(GrantStep::Done(Box::new(GrantResult {
                record: after,
                replayed: false,
            })))
        })
        .await;
    match stored {
        Ok(GrantStep::Done(result)) => Ok(*result),
        Ok(GrantStep::Failed(error)) => Err(error),
        Err(error) => Err(failure
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
            .unwrap_or_else(|| error.into())),
    }
}

/// `SingleTrackPayload`.
pub fn single_track_payload(user_id: &str, progression_id: &str, row: Option<&ProgressionTrackRecord>) -> Value {
    json!({
        "phx_account_id": user_id,
        "progression_id": progression_id,
        "progress": row.map_or(0, |r| r.progress),
        "confirmed_fremium_rank": row.map_or(0, |r| r.confirmed_fremium_rank),
        "confirmed_premium_rank": row.map_or(0, |r| r.confirmed_premium_rank),
        "confirmed_date": row.map_or_else(iso_now, |r| r.updated_at.clone()),
    })
}
