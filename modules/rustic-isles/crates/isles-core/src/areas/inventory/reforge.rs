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
//! The classification and the repair are what the progression area's XP routes and `GET /progression/:userId` call
//! (`classify_reforge_grant`, `mark_reforge_use`, `finish_stale_reforges_for_user`).

use std::sync::{Arc, Mutex};

use serde_json::{Map, Value, json};
use service_kit::{js, jsjson};

use super::clock::{iso, iso_now};
use super::progression_config::ProgressionConfig;
use super::sha256::sha256_hex;
use super::storage_policy::PolicyError;
use super::transaction::{TxContext, TxError, player_actor};
use crate::http::Shared;
use crate::repo::progression::{ProgressionGrantRecord, ProgressionTrackRecord, WeaponReforge};
use crate::repo::{AccountId, RepoError, RepoResult, Repositories, Tx};

const PROGRESSION_FILE: &str = "progression_config.json";
const WEAPONS: [&str; 7] = [
    "Axe",
    "ChainBlades",
    "Hammer",
    "Repeaters",
    "Spear",
    "Strikers",
    "Sword",
];
/// A banked restore that arrives again after the record closed is a duplicate reforge RPC; well past the game's 0.5 s.
const DUPLICATE_RESTORE_WINDOW_MS: i64 = 15 * 60 * 1000;
/// An open record older than this was abandoned (the read-back or a follow-up never arrived): the follow-ups come
/// within about a second, so the next DELETE or the next arrival's progression load finishes it.
const STALE_REFORGE_MS: i64 = 2 * 60 * 1000;
/// `MAX_PROGRESSION_GRANT_AMOUNT`
pub const MAX_GRANT_AMOUNT: f64 = 100_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReforgeMode {
    On,
    Log,
    Off,
}

/// `REFORGE_MODE`: `MYSTICPARADOX_REFORGE` (`on` unless `off` or `log`), and `off` altogether where the version's
/// `reforge_rules` hook says it has no reforge.
pub fn reforge_mode(shared: &Shared) -> ReforgeMode {
    if !shared.version.hooks.reforge_rules() {
        return ReforgeMode::Off;
    }
    match js::trim(shared.settings.raw("MYSTICPARADOX_REFORGE").unwrap_or("on"))
        .to_lowercase()
        .as_str()
    {
        "off" => ReforgeMode::Off,
        "log" => ReforgeMode::Log,
        _ => ReforgeMode::On,
    }
}

fn weapon_of(progression_id: &str, prefix: &str) -> Option<&'static str> {
    let weapon = progression_id.strip_prefix(prefix)?;
    WEAPONS.iter().copied().find(|known| *known == weapon)
}

pub fn is_reforgeable_track(progression_id: &str) -> bool {
    weapon_of(progression_id, "ExperienceTrack_Weapon_").is_some()
}

pub fn experience_track_for_prestige(progression_id: &str) -> Option<String> {
    weapon_of(progression_id, "PrestigeTrack_Weapon_").map(|weapon| format!("ExperienceTrack_Weapon_{weapon}"))
}

fn prestige_track_for(experience_track: &str) -> String {
    experience_track.replacen("ExperienceTrack_", "PrestigeTrack_", 1)
}

/// `nowMs - Date.parse(openedAt) < window`, false for a date JavaScript can't read (NaN).
fn opened_within(now_ms: i64, opened_at: &str, window_ms: i64) -> bool {
    js::date_parse(opened_at).is_some_and(|opened| now_ms - opened < window_ms)
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReforgeGrantPlan {
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

/// `ClassifyReforgeGrant`: how an XP grant relates to the weapon's reforge. `experience_row` is the weapon's
/// ExperienceTrack row (for a PrestigeTrack grant, the same weapon's). With the feature off nothing changes.
pub fn classify_reforge_grant(
    progression_id: &str,
    amount: f64,
    experience_row: Option<&ProgressionTrackRecord>,
    now_ms: i64,
    mode: ReforgeMode,
) -> ReforgeGrantPlan {
    if mode != ReforgeMode::On {
        return ReforgeGrantPlan::Normal;
    }
    let reforge = experience_row.and_then(|row| row.reforge.as_ref());

    if let Some(experience_track) = experience_track_for_prestige(progression_id) {
        // The game grants a weapon's prestige track only from a reforge (step 4), always +1
        if let Some(reforge) = reforge.filter(|r| r.open && !r.prestige_granted && amount == 1.0) {
            return ReforgeGrantPlan::Prestige {
                experience_track,
                reforge: reforge.clone(),
            };
        }
        return ReforgeGrantPlan::Refuse(match reforge {
            Some(r) if r.open => {
                if amount == 1.0 {
                    "this reforge's +1 was already granted".to_owned()
                } else {
                    format!("prestige amount {} is not 1", jsjson::number(amount))
                }
            }
            _ => "no open reforge for this weapon".to_owned(),
        });
    }

    if is_reforgeable_track(progression_id)
        && let Some(reforge) = reforge.filter(|r| r.banked > 0)
    {
        let remaining = reforge.banked - reforge.banked_restored;
        if reforge.open && remaining > 0 && amount == remaining as f64 {
            return ReforgeGrantPlan::Restore {
                experience_track: progression_id.to_owned(),
                reforge: reforge.clone(),
            };
        }
        if remaining == 0
            && amount == reforge.banked as f64
            && opened_within(now_ms, &reforge.opened_at, DUPLICATE_RESTORE_WINDOW_MS)
        {
            return ReforgeGrantPlan::Refuse(format!(
                "banked {} of reforge {} was already restored",
                reforge.banked, reforge.reforge_id
            ));
        }
    }
    ReforgeGrantPlan::Normal
}

/// `ReforgeConflictError`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ReforgeConflict(pub String);

/// `ReforgeGrantHook`, run inside the grant's transaction for a prestige or restore plan: marks the one-time use, or
/// refuses with a [`ReforgeConflict`]. Any other plan is a no-op.
pub fn mark_reforge_use(
    tx: &mut Tx<'_>,
    repos: &Repositories,
    account: &AccountId,
    plan: &ReforgeGrantPlan,
) -> RepoResult<Result<(), ReforgeConflict>> {
    let (kind, experience_track, reforge) = match plan {
        ReforgeGrantPlan::Prestige {
            experience_track,
            reforge,
        } => ("prestige", experience_track, reforge),
        ReforgeGrantPlan::Restore {
            experience_track,
            reforge,
        } => ("restore", experience_track, reforge),
        _ => return Ok(Ok(())),
    };
    let expected = json!({"status": "open", "prestigeGranted": reforge.prestige_granted, "bankedRestored": reforge.banked_restored});
    let mut set = Map::new();
    if kind == "prestige" {
        set.insert("prestigeGranted".into(), Value::Bool(true));
    } else {
        set.insert("bankedRestored".into(), Value::from(reforge.banked));
    }
    let prestige_done = kind == "prestige" || reforge.prestige_granted;
    let banked_done = kind == "restore" || reforge.banked_restored >= reforge.banked;
    let closing = prestige_done && banked_done;
    if closing {
        set.insert("status".into(), Value::String("closed".into()));
        set.insert("closedAt".into(), Value::String(iso_now()));
    }
    let applied = repos.progression_tracks.update_reforge(
        tx,
        account,
        experience_track,
        &reforge.reforge_id,
        &expected,
        &Value::Object(set),
    )?;
    if !applied {
        return Ok(Err(ReforgeConflict(format!(
            "reforge {} changed concurrently ({kind})",
            reforge.reforge_id
        ))));
    }
    tracing::info!(
        "[Reforge] {} user={account} track={experience_track} reforge={}{}{}",
        if kind == "prestige" { "PRESTIGE" } else { "BANKED" },
        reforge.reforge_id,
        if kind == "restore" {
            format!(" restored={}", reforge.banked)
        } else {
            String::new()
        },
        if closing { " CLOSE" } else { "" }
    );
    Ok(Ok(()))
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

/// A reforge whose follow-ups never arrived (the read-back failed, the hub went away): gives the player what the
/// reforge owed, the banked XP back and the +1, exactly once, then closes it. True when this call closed it.
fn finish_stale_reforge(
    tx: &mut Tx<'_>,
    repos: &Repositories,
    account: &AccountId,
    experience_track: &str,
    row: &ProgressionTrackRecord,
) -> RepoResult<bool> {
    let Some(reforge) = row.reforge.as_ref() else {
        return Ok(false);
    };
    let closed = repos.progression_tracks.update_reforge(
        tx,
        account,
        experience_track,
        &reforge.reforge_id,
        &json!({"status": "open", "prestigeGranted": reforge.prestige_granted, "bankedRestored": reforge.banked_restored}),
        &json!({"status": "closed", "closedAt": iso_now(), "prestigeGranted": true, "bankedRestored": reforge.banked}),
    )?;
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
        "[Reforge] RECOVER user={account} track={experience_track} reforge={} stale since {}: restored {owed}, prestige {}",
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

enum Attempt {
    Done(Box<ResetResult>),
    Retry,
}

fn progression_config(ctx: &TxContext<'_>) -> Result<Arc<ProgressionConfig>, PolicyError> {
    let path =
        ctx.shared.game_data.backend_file(PROGRESSION_FILE).ok_or_else(|| {
            PolicyError::GameData(format!("this module has no game data folder for {PROGRESSION_FILE}"))
        })?;
    ProgressionConfig::load(&path)
}

/// `TrackCapXp` for the module's game data.
pub fn track_cap_xp(ctx: &TxContext<'_>, progression_id: &str) -> Result<Option<i64>, PolicyError> {
    Ok(progression_config(ctx)?.track_cap_xp(progression_id))
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
    track: &str,
    at_ms: i64,
    request_id: Option<&str>,
) -> Result<ResetResult, TxError> {
    let mode = reforge_mode(ctx.shared);
    if mode != ReforgeMode::On {
        tracing::warn!("[Reforge] DELETE-REQ user={account} track={track} refused: MYSTICPARADOX_REFORGE={mode:?}");
        return Ok(refused(ResetOutcome::Disabled, None));
    }
    let cap = progression_config(ctx)?.track_cap_xp(track);
    let Some(cap) = cap.filter(|cap| *cap > 0) else {
        return Ok(refused(ResetOutcome::NoTrack, None));
    };
    let repos = ctx.repos();
    let actor = player_actor(account, request_id);
    for _ in 0..4 {
        let repos = repos.clone();
        let account_in = account.clone();
        let track_in = track.to_owned();
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
                    finish_stale_reforge(tx, &repos, &account_in, &track_in, &row)?;
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
                    ctx.ports.observer.reforge_reset(account, at_ms);
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
    pub plan: ReforgeGrantPlan,
    pub grant_id: Option<String>,
}

/// `PlanPurchaseRestore`: the banked re-grant arrives as `POST /progression/purchase/{acct}/{track}/{amount}` with no
/// grant id. It is accepted only as the open reforge's banked restore, with a grant id derived from the reforge so a
/// retry replays through the grant ledger.
pub fn plan_purchase_restore(
    progression_id: &str,
    amount: f64,
    experience_row: Option<&ProgressionTrackRecord>,
    now_ms: i64,
    mode: ReforgeMode,
) -> PurchasePlan {
    let refuse = |reason: String| PurchasePlan {
        plan: ReforgeGrantPlan::Refuse(reason),
        grant_id: None,
    };
    if mode != ReforgeMode::On || !is_reforgeable_track(progression_id) {
        return refuse("not a reforge banked restore".into());
    }
    let Some(reforge) = experience_row.and_then(|row| row.reforge.as_ref()) else {
        return refuse("no reforge for this weapon".into());
    };
    let grant_id = format!("reforge-{}-banked", reforge.reforge_id);
    let plan = classify_reforge_grant(progression_id, amount, experience_row, now_ms, mode);
    if matches!(plan, ReforgeGrantPlan::Restore { .. }) {
        return PurchasePlan {
            plan,
            grant_id: Some(grant_id),
        };
    }
    // Already restored by this same reforge: let the ledger replay it rather than refusing a retry
    if reforge.banked_restored >= reforge.banked && amount == reforge.banked as f64 {
        return PurchasePlan {
            plan: ReforgeGrantPlan::Normal,
            grant_id: Some(grant_id),
        };
    }
    PurchasePlan {
        plan: ReforgeGrantPlan::Refuse(match plan {
            ReforgeGrantPlan::Refuse(reason) => reason,
            _ => format!(
                "amount {} is not the banked {}",
                jsjson::number(amount),
                reforge.banked - reforge.banked_restored
            ),
        }),
        grant_id: None,
    }
}

/// `FinishStaleReforgesForUser`, called on each arrival's progression load: finishes this player's abandoned weapon
/// reforges and returns the weapon tracks it finished.
pub async fn finish_stale_reforges_for_user(
    ctx: &TxContext<'_>,
    account: &AccountId,
    at_ms: i64,
    request_id: Option<&str>,
) -> Result<Vec<String>, TxError> {
    if reforge_mode(ctx.shared) != ReforgeMode::On {
        return Ok(Vec::new());
    }
    let repos = ctx.repos();
    let account_in = account.clone();
    let actor = player_actor(account, request_id);
    let finished = ctx
        .store
        .account_write(account, &actor, move |tx, _| -> RepoResult<Vec<String>> {
            let rows = repos.progression_tracks.get_all_for_account(tx.conn(), &account_in)?;
            let mut finished = Vec::new();
            for row in rows {
                let stale = row
                    .reforge
                    .as_ref()
                    .is_some_and(|r| r.open && !opened_within(at_ms, &r.opened_at, STALE_REFORGE_MS));
                if !is_reforgeable_track(&row.progression_id) || !stale {
                    continue;
                }
                if finish_stale_reforge(tx, &repos, &account_in, &row.progression_id, &row)? {
                    finished.push(row.progression_id.clone());
                }
            }
            Ok(finished)
        })
        .await?;
    Ok(finished)
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

/// `BuildProgressionGrantRequestHash`.
pub fn progression_grant_request_hash(user_id: &str, progression_id: &str, amount: f64) -> String {
    let mut body = Map::new();
    body.insert("version".into(), Value::from(1));
    body.insert("userId".into(), Value::String(user_id.to_owned()));
    body.insert("progressionId".into(), Value::String(progression_id.to_owned()));
    body.insert("amount".into(), super::items::number_value(amount));
    sha256_hex(&jsjson::stringify(&Value::Object(body)))
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
    pub plan: ReforgeGrantPlan,
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
    if !is_reforgeable_track(progression_id)
        || progression_config(ctx)
            .map_err(TxError::from)?
            .track_cap_xp(progression_id)
            .is_none()
    {
        return Err(TxError::Repo(RepoError::Data(format!(
            "Unsupported progression track '{progression_id}'."
        )))
        .into());
    }
    if amount <= 0 || amount as f64 > MAX_GRANT_AMOUNT || !is_valid_grant_id(grant_id) {
        return Err(TxError::Repo(RepoError::Data("Invalid authoritative progression grant.".into())).into());
    }
    let request_hash = progression_grant_request_hash(user_id, progression_id, amount as f64);
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

/// `IsValidProgressionGrantId`: `^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$`.
pub fn is_valid_grant_id(grant_id: &str) -> bool {
    let mut chars = grant_id.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && grant_id.len() <= 128
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'))
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

#[cfg(test)]
mod tests {
    use super::*;

    // controllers/reforge.gamedata.test.ts: the classification half (it reads the real progression_config.json for
    // the cap; the cap is an input here, tested against a config in progression_config)
    const NOW: i64 = 1_791_702_000_000; // 2026-10-11T07:00:00.000Z

    fn open(extra: impl FnOnce(&mut WeaponReforge)) -> WeaponReforge {
        let mut reforge = WeaponReforge {
            reforge_id: "r1".into(),
            open: true,
            prior_progress: 31275,
            cap: 27775,
            banked: 3500,
            banked_restored: 0,
            prestige_granted: false,
            opened_at: iso(NOW - 1000),
            closed_at: None,
        };
        extra(&mut reforge);
        reforge
    }

    fn row(reforge: Option<WeaponReforge>) -> ProgressionTrackRecord {
        ProgressionTrackRecord {
            account: AccountId::new(&"node-a".parse().unwrap()),
            progression_id: "ExperienceTrack_Weapon_Axe".into(),
            progress: 0,
            confirmed_fremium_rank: 0,
            confirmed_premium_rank: 0,
            update_version: 1,
            created_at: String::new(),
            updated_at: String::new(),
            reforge,
            prestige_claimed: None,
        }
    }

    fn kind(plan: &ReforgeGrantPlan) -> &'static str {
        match plan {
            ReforgeGrantPlan::Normal => "normal",
            ReforgeGrantPlan::Prestige { .. } => "prestige",
            ReforgeGrantPlan::Restore { .. } => "restore",
            ReforgeGrantPlan::Refuse(_) => "refuse",
        }
    }

    fn classify(id: &str, amount: f64, reforge: Option<WeaponReforge>) -> &'static str {
        kind(&classify_reforge_grant(
            id,
            amount,
            Some(&row(reforge)),
            NOW,
            ReforgeMode::On,
        ))
    }

    #[test]
    fn weapon_tracks_and_their_prestige_tracks() {
        assert!(is_reforgeable_track("ExperienceTrack_Weapon_Axe"));
        assert!(is_reforgeable_track("ExperienceTrack_Weapon_ChainBlades"));
        assert!(!is_reforgeable_track("ExperienceTrack_PlayerLevel"));
        assert!(!is_reforgeable_track("ExperienceTrack_Weapon_Axe2"));
        assert!(!is_reforgeable_track("xExperienceTrack_Weapon_Axe"));
        assert_eq!(
            experience_track_for_prestige("PrestigeTrack_Weapon_Spear").as_deref(),
            Some("ExperienceTrack_Weapon_Spear")
        );
        assert_eq!(experience_track_for_prestige("season43"), None);
        assert_eq!(
            prestige_track_for("ExperienceTrack_Weapon_Sword"),
            "PrestigeTrack_Weapon_Sword"
        );
    }

    #[test]
    fn one_plus_one_and_one_banked_restore_per_reforge_and_duplicates_are_refused() {
        // +1 on the prestige track only with an open reforge that has not had it
        assert_eq!(
            classify("PrestigeTrack_Weapon_Axe", 1.0, Some(open(|_| {}))),
            "prestige"
        );
        assert_eq!(
            classify(
                "PrestigeTrack_Weapon_Axe",
                1.0,
                Some(open(|r| r.prestige_granted = true))
            ),
            "refuse"
        );
        assert_eq!(classify("PrestigeTrack_Weapon_Axe", 2.0, Some(open(|_| {}))), "refuse");
        assert_eq!(classify("PrestigeTrack_Weapon_Axe", 1.0, None), "refuse");

        // the banked re-grant: exactly the remaining banked amount while open
        assert_eq!(
            classify("ExperienceTrack_Weapon_Axe", 3500.0, Some(open(|_| {}))),
            "restore"
        );
        // a kill grant during the open window is an ordinary grant
        assert_eq!(
            classify("ExperienceTrack_Weapon_Axe", 98.0, Some(open(|_| {}))),
            "normal"
        );
        // the same restore again after it was applied (a duplicate reforge RPC) is refused for a while...
        let restored = |r: &mut WeaponReforge| {
            r.open = false;
            r.banked_restored = 3500;
            r.prestige_granted = true;
        };
        assert_eq!(
            classify("ExperienceTrack_Weapon_Axe", 3500.0, Some(open(restored))),
            "refuse"
        );
        // ...and long after, a grant of the same size is just a grant
        let long_ago = |r: &mut WeaponReforge| {
            restored(r);
            r.opened_at = iso(NOW - 3_600_000);
        };
        assert_eq!(
            classify("ExperienceTrack_Weapon_Axe", 3500.0, Some(open(long_ago))),
            "normal"
        );

        // with the feature off nothing is classified
        let plan = classify_reforge_grant("PrestigeTrack_Weapon_Axe", 1.0, Some(&row(None)), NOW, ReforgeMode::Off);
        assert_eq!(kind(&plan), "normal");
    }

    #[test]
    fn the_refusal_names_why() {
        let reason = |id: &str, amount: f64, reforge: Option<WeaponReforge>| match classify_reforge_grant(
            id,
            amount,
            Some(&row(reforge)),
            NOW,
            ReforgeMode::On,
        ) {
            ReforgeGrantPlan::Refuse(reason) => reason,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            reason("PrestigeTrack_Weapon_Axe", 1.0, None),
            "no open reforge for this weapon"
        );
        assert_eq!(
            reason("PrestigeTrack_Weapon_Axe", 3.0, Some(open(|_| {}))),
            "prestige amount 3 is not 1"
        );
        assert_eq!(
            reason(
                "PrestigeTrack_Weapon_Axe",
                1.0,
                Some(open(|r| r.prestige_granted = true))
            ),
            "this reforge's +1 was already granted"
        );
        let restored = open(|r| {
            r.open = false;
            r.banked_restored = 3500;
        });
        assert_eq!(
            reason("ExperienceTrack_Weapon_Axe", 3500.0, Some(restored)),
            "banked 3500 of reforge r1 was already restored"
        );
    }

    #[test]
    fn the_banked_restore_through_the_purchase_endpoint() {
        let plan = |id: &str, amount: f64, reforge: Option<WeaponReforge>| {
            plan_purchase_restore(id, amount, Some(&row(reforge)), NOW, ReforgeMode::On)
        };
        // only an open reforge's remaining banked amount, with a reforge-derived grant id
        let opened = plan("ExperienceTrack_Weapon_Axe", 3500.0, Some(open(|_| {})));
        assert_eq!(kind(&opened.plan), "restore");
        assert_eq!(opened.grant_id.as_deref(), Some("reforge-r1-banked"));
        assert_eq!(
            kind(&plan("ExperienceTrack_Weapon_Axe", 3499.0, Some(open(|_| {}))).plan),
            "refuse"
        );
        assert_eq!(kind(&plan("ExperienceTrack_Weapon_Axe", 3500.0, None).plan), "refuse");
        assert_eq!(
            kind(&plan("ExperienceTrack_PlayerLevel", 3500.0, Some(open(|_| {}))).plan),
            "refuse"
        );
        // already restored by this reforge: a replay candidate (the route replays it only if the ledger holds the grant)
        let again = plan(
            "ExperienceTrack_Weapon_Axe",
            3500.0,
            Some(open(|r| {
                r.open = false;
                r.banked_restored = 3500;
                r.prestige_granted = true;
            })),
        );
        assert_eq!(kind(&again.plan), "normal");
        assert_eq!(again.grant_id.as_deref(), Some("reforge-r1-banked"));
        // off
        let off = plan_purchase_restore(
            "ExperienceTrack_Weapon_Axe",
            3500.0,
            Some(&row(Some(open(|_| {})))),
            NOW,
            ReforgeMode::Off,
        );
        assert_eq!(
            off.plan,
            ReforgeGrantPlan::Refuse("not a reforge banked restore".into())
        );
    }

    #[test]
    fn grant_ids_and_the_request_hash_follow_node() {
        assert!(is_valid_grant_id("reforge-3f2a-banked"));
        assert!(is_valid_grant_id("gs-0b1c2d"));
        assert!(!is_valid_grant_id("-lead"));
        assert!(!is_valid_grant_id(""));
        assert!(!is_valid_grant_id(&"a".repeat(129)));
        assert_eq!(
            progression_grant_request_hash("u", "ExperienceTrack_Weapon_Axe", 3500.0),
            sha256_hex(r#"{"version":1,"userId":"u","progressionId":"ExperienceTrack_Weapon_Axe","amount":3500}"#)
        );
    }

    #[test]
    fn a_single_track_payload_reads_zero_without_a_row() {
        let payload = single_track_payload("u", "ExperienceTrack_Weapon_Axe", None);
        assert_eq!(payload["progress"], 0);
        assert_eq!(payload["confirmed_fremium_rank"], 0);
        assert_eq!(payload["phx_account_id"], "u");
        let keys: Vec<&String> = payload.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            [
                "phx_account_id",
                "progression_id",
                "progress",
                "confirmed_fremium_rank",
                "confirmed_premium_rank",
                "confirmed_date"
            ]
        );
    }
}
