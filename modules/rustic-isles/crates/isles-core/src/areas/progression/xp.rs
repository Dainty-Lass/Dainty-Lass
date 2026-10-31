//! The game servers' posts: `POST /progression/:userId/:progressionId/:amount` (an XP grant, exactly once by its
//! grant id), `POST /progression/:userId` (the objective report and mastery) and the rank confirm.

use axum::response::Response;
use rifts_seam::{HostWriteKind, HostWriteOptions};
use serde_json::{Value, json};
use service_kit::js;

use crate::AccountId;
use crate::http::Call;
use crate::repo::progression::{ProgressionGrantRecord, ProgressionTrackRecord, now_iso};
use crate::repo::{RepoError, RepoResult};

use super::auth::{
    host_policy_refuses, limit_only_logs, note_would_refuse, refuse_host_write, refuse_plain, tag_host_write,
};
use super::mastery::{
    HostMasteryLimits, MasteryPlan, StoredObjective, describe_limits, plan_mastery_report, summarize_plan, track_row,
};
use super::ports::Ports;
use super::reforge::{ReforgePlan, apply_grant_hook, classify_grant, grant_hook, reforge_mode};
use super::req::{Req, internal, no_account};
use super::tracks::{
    HostGrantRefusal, MAX_PROGRESSION_GRANT_AMOUNT, NO_PLAYER, PROGRESSION_ID_MAX_LENGTH, active_huntpass_id,
    check_host_track_total, check_host_xp_grant, curves, describe_host_refusal, describe_unapproved_xp_track,
    experience_track_for_prestige, grant_request_hash, host_limit_denial, host_track_total_cap, is_approved_xp_track,
    is_huntpass_xp_track, is_reforgeable_track, is_valid_grant_id, multiply_xp_grant, test_multipliers,
};
use super::util::{as_integer, bad_request, conflict, forbidden, js_number, log_safe, ok, reply};

/// `MasteryPersistence`: `MYSTICPARADOX_MASTERY_PERSISTENCE` (the env file, then the version's default switch).
pub fn mastery_persistence(shared: &crate::http::Shared) -> bool {
    match shared.settings.raw(crate::settings::MASTERY_PERSISTENCE.env) {
        Some(_) => shared.settings.on_unless_off(crate::settings::MASTERY_PERSISTENCE.env),
        None => shared.version.hooks.mastery_persistence(),
    }
}

/// The first of the values that is not null or undefined, else the last one (`a ?? b ?? c`).
fn nullish_chain<'a>(values: &[Option<&'a Value>]) -> Option<&'a Value> {
    values
        .iter()
        .copied()
        .flatten()
        .find(|v| !v.is_null())
        .or_else(|| values.last().copied().flatten())
}

/// `req.get("x-mysticparadox-grant-id") ?? req.get("idempotency-key")`.
fn header_grant_id(req: &Req) -> Option<String> {
    req.header("x-mysticparadox-grant-id")
        .or_else(|| req.header("idempotency-key"))
        .map(str::to_owned)
}

fn body_grant_id(req: &Req) -> Option<&Value> {
    nullish_chain(&[req.body.get(&["grant_id"]), req.body.get(&["grantId"])])
}

/// What was sent as a grant id, whatever it is: header, then body (`Sent` in `GrantIdFor`).
fn sent_grant_id(req: &Req) -> Option<Value> {
    match header_grant_id(req) {
        Some(header) => Some(Value::String(header)),
        None => body_grant_id(req).cloned(),
    }
}

/// `ReadAuthoritativeGrantId`.
fn read_grant_id(req: &Req) -> Option<String> {
    let header = header_grant_id(req);
    let body = body_grant_id(req);
    if let (Some(h), Some(b)) = (&header, body)
        && b.as_str() != Some(h.as_str())
    {
        return None;
    }
    let candidate: Option<Value> = header.map(Value::String).or_else(|| body.cloned());
    candidate
        .as_ref()
        .and_then(Value::as_str)
        .filter(|id| is_valid_grant_id(id))
        .map(str::to_owned)
}

/// `GrantIdFor`: an authenticated game server that sent none gets one minted (the 1.14.7 server sends neither).
fn grant_id_for(req: &Req) -> Option<String> {
    if sent_grant_id(req).is_none() && req.auth.is_gameserver {
        return Some(format!("gs-{}", crate::game_data::season_window::next_uuid()));
    }
    read_grant_id(req)
}

/// `DescribeGrantIdPresence`.
fn describe_grant_id(req: &Req, grant_id: Option<&str>) -> &'static str {
    if grant_id.is_some() {
        "present"
    } else if sent_grant_id(req).is_none() {
        "absent"
    } else {
        "invalid"
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

enum Granted {
    Done {
        record: ProgressionTrackRecord,
        replayed: bool,
    },
    Conflict,
    ReforgeConflict(String),
    HostLimit(HostGrantRefusal),
}

pub async fn grant(call: Call, ports: Ports) -> Response {
    let req = match Req::new(call, ports).await {
        Ok(req) => req,
        Err(response) => return response,
    };
    let shared = req.shared.clone();
    let grant_id = grant_id_for(&req);
    let is_host = req.auth.is_host();
    let user_id = req.param("userId").to_owned();
    let progression_id = req.param("progressionId").to_owned();
    let amount_raw = req.param("amount").to_owned();
    let src = if is_host {
        "host"
    } else if req.auth.is_gameserver {
        "gs"
    } else {
        "other"
    };
    tracing::info!(
        "[ProgressionGrant] REQ user={} track={} amount={} grantId={} src={src}",
        log_safe(&user_id, PROGRESSION_ID_MAX_LENGTH),
        log_safe(&progression_id, PROGRESSION_ID_MAX_LENGTH),
        log_safe(&amount_raw, PROGRESSION_ID_MAX_LENGTH),
        describe_grant_id(&req, grant_id.as_deref())
    );

    if !is_host && !req.auth.is_gameserver {
        tracing::warn!("Rejected XP grant attempt without gameserver auth (userId={user_id}, track={progression_id})");
        return forbidden();
    }
    if user_id.is_empty() || user_id == NO_PLAYER {
        tracing::warn!("Rejected XP grant: invalid/sentinel userId ({user_id})");
        return bad_request();
    }
    if progression_id.is_empty() || progression_id.chars().count() > PROGRESSION_ID_MAX_LENGTH {
        tracing::warn!(
            "Rejected XP grant: invalid progressionId (len={})",
            progression_id.chars().count()
        );
        return bad_request();
    }
    if !is_approved_xp_track(&shared, &progression_id) {
        tracing::warn!(
            "Rejected XP grant: progression track is not approved (user={user_id}, track={progression_id}): {}",
            describe_unapproved_xp_track(&shared, &progression_id)
        );
        return bad_request();
    }
    let Some(grant_id) = grant_id else {
        tracing::warn!(
            "Rejected XP grant: missing or invalid authoritative grant id (user={user_id}, track={progression_id})"
        );
        return bad_request();
    };
    let amount = as_integer(js_number(&amount_raw)).filter(|a| *a > 0 && *a <= MAX_PROGRESSION_GRANT_AMOUNT);
    let Some(amount) = amount else {
        tracing::warn!("Rejected XP grant: invalid amount ({amount_raw}) for user={user_id} track={progression_id}");
        return bad_request();
    };

    // A host's grant is checked once it is known to be well formed, before any read of that player's track. The
    // kinds are tried quietly (probe) in order, so a grant the session takes as REFORGE or HUNT_PASS_XP is tagged so.
    let mut write_as = HostWriteKind::Xp;
    if is_host {
        let probe = HostWriteOptions {
            probe: true,
            ..HostWriteOptions::default()
        };
        let mut denial = match refuse_host_write(&shared, &req.auth, HostWriteKind::Xp, &user_id, probe).await {
            Ok(denial) => denial,
            Err(response) => return response,
        };
        if denial.is_some() && experience_track_for_prestige(&progression_id).is_some() {
            match refuse_host_write(&shared, &req.auth, HostWriteKind::Reforge, &user_id, probe).await {
                Ok(None) => {
                    write_as = HostWriteKind::Reforge;
                    denial = None;
                }
                Ok(Some(_)) => {}
                Err(response) => return response,
            }
        }
        if denial.is_some() && progression_id == active_huntpass_id(&shared) {
            match refuse_host_write(&shared, &req.auth, HostWriteKind::HuntPassXp, &user_id, probe).await {
                Ok(None) => {
                    write_as = HostWriteKind::HuntPassXp;
                    denial = None;
                }
                Ok(Some(_)) => {}
                Err(response) => return response,
            }
        }
        if denial.is_some() {
            denial = match refuse_plain(&shared, &req.auth, HostWriteKind::Xp, &user_id).await {
                Ok(denial) => denial,
                Err(response) => return response,
            };
        }
        if let Some(denial) = denial {
            return denial;
        }
        // Only the impossible is refused, after the authorization check so a host that may not write this player
        // learns nothing about the limits; never clamped
        if let Some(refusal) = check_host_xp_grant(&shared, &progression_id, amount)
            && !limit_only_logs(&shared, refusal.reason)
        {
            tracing::warn!(
                "[HostWrite] refused kind={write_as:?} target={} {} grant={}",
                log_safe(&user_id, 128),
                describe_host_refusal(&refusal),
                log_safe(&grant_id, 128)
            );
            return host_limit_denial();
        }
    }

    if req.auth.is_gameserver
        && let Some(foreign_account) = crate::areas::matchmaking::resolve_foreign_account(&shared, &user_id).await
    {
        let granted_amount = multiply_xp_grant(amount, MAX_PROGRESSION_GRANT_AMOUNT, test_multipliers(&shared));
        let instance_id = req
            .header("x-reforged-instance-id")
            .and_then(|s| s.parse::<module_api::GlobalId>().ok())
            .or_else(|| shared.matchmaking.ring_instance_for_player(&foreign_account))
            .unwrap_or_else(|| module_api::GlobalId::new(module_api::network::IdKind::Hunt, &shared.node.node));
        let module_name = format!("isles-{}", shared.version.tables.game_version.replace('.', "-"));
        let Ok(module) = module_api::ModuleId::new(module_name) else {
            return internal(crate::repo::RepoError::Home("invalid module id".into()));
        };
        let rewards = module_api::network::HuntRewards {
            hunt: instance_id,
            module,
            account: foreign_account.clone(),
            xp_grant: Some(json!({
                "track": progression_id,
                "xp": granted_amount,
            })),
            loot_write: None,
            objective_write: None,
        };
        if let Err(error) = shared.home().ring_forward_hunt_rewards(&rewards).await {
            tracing::error!("ring_forward_hunt_rewards (xp) for {foreign_account}: {error}");
            return internal(crate::repo::RepoError::Home(error.to_string()));
        }
        return ok(json!({
            "progression_id": progression_id,
            "xp": granted_amount,
            "level": 1,
            "forwarded_to_home": true,
        }));
    }

    let Some(account) = (match req.account(&user_id).await {
        Ok(account) => account,
        Err(response) => return response,
    }) else {
        return no_account(&user_id);
    };

    // Reforge follow-ups: the +1 on PrestigeTrack_Weapon_* and the banked re-grant are accepted once per reforge. A
    // grant id the ledger already holds is a transport retry and replays as before.
    let mut plan = ReforgePlan::Normal;
    if is_reforgeable_track(&progression_id) || experience_track_for_prestige(&progression_id).is_some() {
        let experience_track = experience_track_for_prestige(&progression_id).unwrap_or_else(|| progression_id.clone());
        let (ledger, row) = {
            let (grant_id, owner) = (grant_id.clone(), account.clone());
            let read = req.store().map(|store| {
                store.read(move |conn, repos| {
                    Ok((
                        repos.progression_grants.find_by_grant_id(conn, &grant_id)?,
                        repos.progression_tracks.get(conn, &owner, &experience_track)?,
                    ))
                })
            });
            match read {
                Ok(read) => match read.await {
                    Ok(found) => found,
                    Err(error) => return internal(error),
                },
                Err(response) => return response,
            }
        };
        if ledger.is_none() {
            plan = classify_grant(reforge_mode(&shared), &progression_id, amount, row.as_ref(), now_ms());
            if let ReforgePlan::Refuse(reason) = &plan {
                tracing::warn!(
                    "[Reforge] DUP/REFUSED grant user={user_id} track={progression_id} amount={amount} grant={grant_id}: {reason}"
                );
                return conflict();
            }
            // A REFORGE-authorized grant is the reforge's +1 and nothing else, switch or not
            if is_host && experience_track_for_prestige(&progression_id).is_some() && plan.kind() != "prestige" {
                tracing::warn!(
                    "[Reforge] REFUSED host prestige grant user={user_id} track={progression_id} grant={grant_id}: not a reforge +1 ({})",
                    plan.kind()
                );
                return conflict();
            }
        }
    }

    // A private test server's XP multiplier, capped at the route's maximum
    let granted_amount = multiply_xp_grant(amount, MAX_PROGRESSION_GRANT_AMOUNT, test_multipliers(&shared));
    if granted_amount != amount {
        tracing::info!(
            "[TestMultiplier] XP grant {amount} -> {granted_amount} (user={user_id}, track={progression_id})"
        );
    }

    let observe = !host_policy_refuses(&shared);
    let outcome = grant_progression_xp(
        &req,
        &account,
        GrantInput {
            user_id: user_id.clone(),
            progression_id: progression_id.clone(),
            amount: granted_amount,
            grant_id: grant_id.clone(),
            plan: plan.clone(),
            host: is_host,
            observe,
        },
    )
    .await;
    let (outcome, notes) = match outcome {
        Ok(found) => found,
        Err(response) => return response,
    };
    for (what, detail) in notes {
        note_would_refuse(&shared, &what, &detail);
    }
    let (updated, replayed) = match outcome {
        Granted::Done { record, replayed } => (record, replayed),
        Granted::Conflict => {
            tracing::warn!("Rejected XP grant reuse with different request (grant={grant_id}, user={user_id})");
            return conflict();
        }
        Granted::ReforgeConflict(message) => {
            tracing::warn!(
                "[Reforge] DUP/REFUSED grant user={user_id} track={progression_id} amount={amount}: {message}"
            );
            return conflict();
        }
        Granted::HostLimit(refusal) => {
            tracing::warn!(
                "[HostWrite] refused kind=XP target={} {} grant={}",
                log_safe(&user_id, 128),
                describe_host_refusal(&refusal),
                log_safe(&grant_id, 128)
            );
            return host_limit_denial();
        }
    };

    // Tag the host's grant. A replay changed nothing and belongs to the request that stored it.
    if is_host && !replayed {
        let mut summary = json!({ "track": progression_id, "amount": amount, "grantId": grant_id });
        if write_as == HostWriteKind::Reforge
            && let ReforgePlan::Prestige { reforge, .. } = &plan
            && let Some(object) = summary.as_object_mut()
        {
            let mut tagged = serde_json::Map::new();
            tagged.insert("route".into(), "xp".into());
            tagged.extend(std::mem::take(object));
            tagged.insert("reforgeId".into(), reforge.reforge_id.clone().into());
            summary = Value::Object(tagged);
        }
        tag_host_write(&shared, &req.auth, write_as, &user_id, summary).await;
    }

    if is_huntpass_xp_track(&shared, &progression_id) {
        monitor_huntpass_grant(
            &shared,
            &user_id,
            &progression_id,
            amount,
            &updated,
            replayed,
            &grant_id,
        );
    }

    ok(json!({
        "phx_account_id": user_id,
        "progression_id": progression_id,
        "progress": updated.progress,
        "confirmed_fremium_rank": updated.confirmed_fremium_rank,
        "confirmed_premium_rank": updated.confirmed_premium_rank,
        "confirmed_date": updated.updated_at,
        "grant_id": grant_id,
        "replayed": replayed,
    }))
}

/// `MonitorHuntPassGrant`: one line per stored hunt pass grant.
fn monitor_huntpass_grant(
    shared: &crate::http::Shared,
    user_id: &str,
    progression_id: &str,
    amount: i64,
    updated: &ProgressionTrackRecord,
    replayed: bool,
    grant_id: &str,
) {
    if !shared.settings.on_unless_off("MYSTICPARADOX_ECONOMY_MONITOR") {
        return;
    }
    let curves = curves(shared);
    let rank = |progress: i64| {
        curves
            .rank_for_progress(progression_id, progress as f64)
            .map_or_else(|| "?".to_owned(), |r| r.to_string())
    };
    let old = updated.progress - amount;
    tracing::info!(
        "[HuntPassMon] GRANT user={user_id} track={progression_id} amount={amount} progress={old}->{} rank={}->{} confirmed={}/{} replayed={} grant={grant_id}",
        updated.progress,
        rank(old),
        rank(updated.progress),
        updated.confirmed_fremium_rank,
        updated.confirmed_premium_rank,
        u8::from(replayed)
    );
}

struct GrantInput {
    user_id: String,
    progression_id: String,
    amount: i64,
    grant_id: String,
    plan: ReforgePlan,
    host: bool,
    /// The host policy is in observe mode: the two limits a dedicated server never had only log
    observe: bool,
}

type Notes = Vec<(String, String)>;

/// `GrantProgressionXp`: applies one authoritative grant exactly once. The reforge mark, the track change and the
/// ledger row commit in one transaction.
async fn grant_progression_xp(req: &Req, account: &AccountId, input: GrantInput) -> Result<(Granted, Notes), Response> {
    let shared = req.shared.clone();
    let mut notes: Notes = Vec::new();
    let only_logs = |reason: &str, observe: bool| matches!(reason, "amount_over_ceiling" | "track_full") && observe;
    if input.host
        && let Some(refusal) = check_host_xp_grant(&shared, &input.progression_id, input.amount)
    {
        if !only_logs(refusal.reason, input.observe) {
            return Ok((Granted::HostLimit(refusal), notes));
        }
        notes.push((
            "xp".into(),
            format!(
                "user={} {} grant={}",
                input.user_id,
                describe_host_refusal(&refusal),
                input.grant_id.chars().take(96).collect::<String>()
            ),
        ));
    }
    let hash = grant_request_hash(&input.user_id, &input.progression_id, input.amount);
    let total_cap = host_track_total_cap(&shared, &input.progression_id);
    let hook = grant_hook(&input.plan);
    let owner = account.clone();
    let (user_id, progression_id, amount) = (input.user_id.clone(), input.progression_id.clone(), input.amount);
    let (shared_in, input_in) = (shared.clone(), input);
    let written = req
        .write(account, move |tx, repos| {
            let mut notes: Notes = Vec::new();
            let input = input_in;
            if let Some(existing) = repos.progression_grants.find_by_grant_id(tx.conn(), &input.grant_id)? {
                if existing.request_hash != hash
                    || existing.account != owner
                    || existing.progression_id != input.progression_id
                    || existing.amount != input.amount
                {
                    return Ok((Granted::Conflict, notes));
                }
                return Ok((
                    Granted::Done {
                        record: existing.result,
                        replayed: true,
                    },
                    notes,
                ));
            }
            // A host may not add to a player level track that is already full, decided after the replay branch so a
            // retry of the grant that filled the track still replays
            if input.host && total_cap.is_some() {
                let current = repos.progression_tracks.get(tx.conn(), &owner, &input.progression_id)?;
                let stored = current.map_or(0, |c| c.progress);
                if let Some(full) = check_host_track_total(&shared_in, &input.progression_id, stored, input.amount) {
                    if !only_logs(full.reason, input.observe) {
                        return Ok((Granted::HostLimit(full), notes));
                    }
                    notes.push((
                        "xp".into(),
                        format!(
                            "user={} {} grant={}",
                            input.user_id,
                            describe_host_refusal(&full),
                            input.grant_id.chars().take(96).collect::<String>()
                        ),
                    ));
                }
            }
            if let Some(hook) = &hook
                && let Err(conflict) = apply_grant_hook(tx, repos, &owner, &input.user_id, hook)?
            {
                return Ok((Granted::ReforgeConflict(conflict.0), notes));
            }
            let before = repos.progression_tracks.get(tx.conn(), &owner, &input.progression_id)?;
            let after = repos
                .progression_tracks
                .increment(tx, &owner, &input.progression_id, input.amount)?;
            repos.progression_grants.insert_applied(
                tx,
                &ProgressionGrantRecord {
                    grant_id: input.grant_id.clone(),
                    account: owner.clone(),
                    progression_id: input.progression_id.clone(),
                    amount: input.amount,
                    request_hash: hash,
                    result: after.clone(),
                    created_at: now_iso(),
                },
            )?;
            tracing::info!(
                "[ProgressionGrant] grant={} user={} track={} delta={} old={} new={} source={} applied=1",
                input.grant_id,
                input.user_id,
                input.progression_id,
                input.amount,
                before.map_or(0, |b| b.progress),
                after.progress,
                if input.host { "host" } else { "gameserver" }
            );
            Ok((
                Granted::Done {
                    record: after,
                    replayed: false,
                },
                notes,
            ))
        })
        .await?;
    let (granted, more) = written;
    notes.extend(more);
    if let Granted::Done { replayed: false, .. } = &granted {
        // The activity monitor counts an applied grant's XP by track (a replay is not counted again)
        req.ports.activity.xp(&shared, &user_id, &progression_id, amount);
    }
    Ok((granted, notes))
}

// ---------------------------------------------------------------------------------------------------------------
// POST /progression/:userId

/// `CaptureProgressionObjectiveEvent`'s validation: `false` is `ProgressionEventValidationError`.
pub fn report_is_valid(body: &Value) -> bool {
    let valid_id = |value: Option<&Value>| {
        value.and_then(Value::as_str).is_some_and(|id| {
            let length = id.chars().count();
            (1..=128).contains(&length)
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))
        })
    };
    let bounded = |value: Option<&Value>, maximum: f64| {
        let number = js::number(value);
        number.is_finite() && number >= 0.0 && number <= maximum
    };
    let list = |key: &str| match body.get(key) {
        None | Some(Value::Null) => Some(Vec::new()),
        Some(Value::Array(items)) => Some(items.clone()),
        Some(_) => None,
    };
    if super::util::stringify(body).encode_utf16().count() > 128 * 1024 {
        return false;
    }
    let (Some(tracks), Some(objectives)) = (list("progress_tracks"), list("objectives")) else {
        return false;
    };
    if tracks.len() > 256 || objectives.len() > 512 {
        return false;
    }
    let tracks_ok = tracks
        .iter()
        .all(|t| valid_id(t.get("progression_id")) && bounded(t.get("progress"), 1_000_000_000.0));
    let objectives_ok = objectives.iter().all(|o| {
        let count_ok = match o.get("completed_count") {
            None | Some(Value::Null) => true,
            other => bounded(other, 1_000_000.0),
        };
        valid_id(o.get("objective_id")) && bounded(o.get("value"), 1_000_000_000.0) && count_ok
    });
    tracks_ok && objectives_ok
}

/// `SummarizeObjectiveReport`: what a report carried, for a host's tag.
fn summarize_report(body: &Value) -> Value {
    let list = |key: &str| body.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
    let (tracks, objectives) = (list("progress_tracks"), list("objectives"));
    let progress_tracks: Vec<Value> = tracks
        .iter()
        .take(20)
        .map(|t| {
            let id: String = js::string(t.get("progression_id")).chars().take(128).collect();
            json!({ "id": id, "progress": js::number(t.get("progress")) })
        })
        .collect();
    json!({
        "progressTracks": progress_tracks,
        "progressTrackCount": tracks.len(),
        "objectiveCount": objectives.len(),
        "completedObjectiveCount": objectives.iter().filter(|o| js::number(o.get("completed_count")) >= 1.0).count(),
    })
}

pub async fn report(call: Call, ports: Ports) -> Response {
    let req = match Req::new(call, ports).await {
        Ok(req) => req,
        Err(response) => return response,
    };
    let shared = req.shared.clone();
    let user_id = req.resolve_user_id();
    let is_host = req.auth.is_host();

    // A player bearer may only report its own state. A player-hosted runtime reports for the players it hosts, once
    // the module's hostWrites hook accepts it.
    if is_host {
        match refuse_plain(&shared, &req.auth, HostWriteKind::Objectives, &user_id).await {
            Ok(None) => {}
            Ok(Some(denial)) => {
                tracing::warn!(
                    "Rejected host progression report actor={} target={user_id}",
                    req.auth.user_id.as_deref().unwrap_or("unknown")
                );
                return denial;
            }
            Err(response) => return response,
        }
    } else if !req.auth.may_report(&user_id) {
        tracing::warn!(
            "Rejected cross-account progression report actor={} target={user_id}",
            req.auth.user_id.as_deref().unwrap_or("unknown")
        );
        return forbidden();
    }

    let mut payload = json!({});
    if user_id == NO_PLAYER {
        tracing::debug!("Progression/objective event received for no-player sentinel - discarding, not persisting");
    } else {
        // `req.body` is undefined for a request express did not parse; the capture reads `rawBody ?? {}`
        let raw = req.body.json().cloned().unwrap_or_else(|| json!({}));
        if !report_is_valid(&raw) {
            return bad_request();
        }
        let Some(account) = (match req.account(&user_id).await {
            Ok(account) => account,
            Err(response) => return response,
        }) else {
            return no_account(&user_id);
        };
        let body_json = super::util::stringify(&raw);
        {
            let (text, owner) = (body_json.clone(), account.clone());
            let received = now_iso();
            if let Err(response) = req
                .write(&account, move |tx, repos| {
                    repos
                        .progression_tracks
                        .append_objective_event(tx, &owner, &text, &received)
                })
                .await
            {
                return response;
            }
        }
        log_unrecognized_keys(&user_id, &raw, &body_json);

        if is_host {
            tag_host_write(
                &shared,
                &req.auth,
                HostWriteKind::Objectives,
                &user_id,
                summarize_report(&raw),
            )
            .await;
        }

        // Mastery (1.14.7): only a dedicated game server's report changes stored state, a player's own token stays
        // diagnostics-only. A host's report persists once MASTERY is accepted for that player.
        let mut persist = false;
        if mastery_persistence(&shared) {
            if is_host {
                match refuse_plain(&shared, &req.auth, HostWriteKind::Mastery, &user_id).await {
                    Ok(None) => persist = true,
                    Ok(Some(_)) => {
                        tracing::warn!("[MasteryMon] user={user_id} host report left diagnostics-only: write refused")
                    }
                    Err(response) => return response,
                }
            } else {
                persist = req.auth.is_gameserver;
            }
        }
        if persist {
            match apply_mastery_report(&req, &account, &user_id, &raw, is_host).await {
                Ok((applied, plan)) => {
                    payload = applied;
                    if is_host {
                        tag_host_write(
                            &shared,
                            &req.auth,
                            HostWriteKind::Mastery,
                            &user_id,
                            summarize_plan(&plan),
                        )
                        .await;
                    }
                }
                Err(message) => {
                    tracing::error!("[MasteryMon] user={user_id} report NOT applied: {message}");
                    payload = json!({});
                }
            }
        }
    }
    ok(payload)
}

fn log_unrecognized_keys(user_id: &str, body: &Value, body_json: &str) {
    let Some(object) = body.as_object() else { return };
    let unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|k| !matches!(*k, "progress_tracks" | "objectives"))
        .collect();
    if !unknown.is_empty() {
        tracing::warn!(
            "[ProgressionObjective] user={user_id} UNRECOGNIZED top-level key(s) [{}] in a progression report - possible unhandled reward/grant payload, not applied. Raw: {}",
            unknown.join(", "),
            body_json.chars().take(2000).collect::<String>()
        );
    }
    let tracks = body
        .get("progress_tracks")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let objectives = body.get("objectives").and_then(Value::as_array).map_or(0, Vec::len);
    tracing::info!(
        "[ProgressionObjective] user={user_id} mode=diagnostics-only tracksReceived={tracks} objectivesReceived={objectives} authoritativeMutation=0 rawBytes={}",
        body_json.len()
    );
}

/// `ApplyMasteryReportWithPlan`: the stored objectives, the plan, the merge and the credits in one transaction.
async fn apply_mastery_report(
    req: &Req,
    account: &AccountId,
    user_id: &str,
    body: &Value,
    host: bool,
) -> Result<(Value, MasteryPlan), String> {
    let ids: Vec<String> = {
        let mut ids: Vec<String> = Vec::new();
        for objective in body.get("objectives").and_then(Value::as_array).into_iter().flatten() {
            let id = js::string(objective.get("objective_id"));
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        ids
    };
    let limits = host.then(|| HostMasteryLimits::new(&req.shared));
    let (body_in, owner) = (body.clone(), account.clone());
    let outcome = req
        .write(account, move |tx, repos| -> RepoResult<_> {
            let stored: std::collections::HashMap<String, StoredObjective> = repos
                .progression_tracks
                .get_objectives(tx.conn(), &owner, &ids)?
                .into_iter()
                .map(|row| {
                    (
                        row.objective_id,
                        StoredObjective {
                            value: row.value,
                            completed_count: row.completed_count,
                        },
                    )
                })
                .collect();
            let limits = match limits {
                Some(mut limits) => {
                    let mut totals = std::collections::HashMap::new();
                    for id in limits.capped_claims(&body_in) {
                        let total = repos
                            .progression_tracks
                            .get(tx.conn(), &owner, &id)?
                            .map_or(0, |t| t.progress);
                        totals.insert(id, total);
                    }
                    limits.track_totals = Some(totals);
                    Some(limits)
                }
                None => None,
            };
            let plan = plan_mastery_report(&body_in, &stored, limits.as_ref());
            let merges: Vec<(String, i64, i64)> = plan
                .objectives
                .iter()
                .map(|(id, s)| (id.clone(), s.value, s.completed_count))
                .collect();
            repos.progression_tracks.merge_objectives(tx, &owner, &merges)?;
            let mut totals: Vec<(String, Option<ProgressionTrackRecord>)> = Vec::new();
            for (track, amount) in plan.credits.iter() {
                totals.push((
                    track.clone(),
                    Some(repos.progression_tracks.increment(tx, &owner, track, *amount)?),
                ));
            }
            for (track, _) in plan.claimed.iter() {
                if !totals.iter().any(|(id, _)| id == track) {
                    totals.push((track.clone(), repos.progression_tracks.get(tx.conn(), &owner, track)?));
                }
            }
            Ok((plan, totals))
        })
        .await
        .map_err(|_| "the database write failed".to_owned())?;
    let (plan, totals) = outcome;

    if !plan.newly_completed.is_empty() || !plan.claimed.is_empty() {
        let join = |items: Vec<String>| items.join(", ");
        tracing::info!(
            "[MasteryMon] user={user_id} newly=[{}] credit={{{}}} claimed={{{}}} totals={{{}}}{}{} objectives={}",
            plan.newly_completed.join(", "),
            join(plan.credits.iter().map(|(t, a)| format!("{t}+{a}")).collect()),
            join(plan.claimed.iter().map(|(t, a)| format!("{t}+{a}")).collect()),
            join(
                totals
                    .iter()
                    .map(|(t, r)| format!("{t}={}", r.as_ref().map_or(0, |r| r.progress)))
                    .collect()
            ),
            if plan.capped.is_empty() {
                String::new()
            } else {
                format!(" capped=[{}]", plan.capped.join("; "))
            },
            if plan.unmapped.is_empty() {
                String::new()
            } else {
                format!(" UNMAPPED=[{}]", plan.unmapped.join(", "))
            },
            plan.objectives.len()
        );
    }
    if let Some(limited) = describe_limits(&plan) {
        tracing::warn!("[HostWrite] limited kind=MASTERY target={user_id} {limited}");
    }

    let now = now_iso();
    let payload = json!({
        "progress_tracks": totals.iter().map(|(track, row)| track_row(user_id, track, row.as_ref(), &now)).collect::<Vec<_>>(),
        "objectives": plan.merged.iter().map(|(objective_id, state)| json!({
            "phx_account_id": user_id,
            "objective_id": objective_id,
            "progress": state.value,
            "completed_count": state.completed_count,
            "created_date": now,
            "last_modified_date": now,
        })).collect::<Vec<_>>(),
    });
    Ok((payload, plan))
}

// ---------------------------------------------------------------------------------------------------------------
// POST /progression/:userId/:progressionId/:rank/confirm/public

enum Confirmed {
    Ok(ProgressionTrackRecord),
    Rejected(&'static str),
}

pub async fn confirm(call: Call, ports: Ports) -> Response {
    let req = match Req::new(call, ports).await {
        Ok(req) => req,
        Err(response) => return response,
    };
    let shared = req.shared.clone();
    let is_host = req.auth.is_host();
    let user_id = req.param("userId").to_owned();
    let progression_id = req.param("progressionId").to_owned();
    let rank_raw = req.param("rank").to_owned();
    if !is_host && !req.auth.is_gameserver {
        tracing::warn!(
            "Rejected rank confirm attempt without gameserver auth (userId={user_id}, track={progression_id})"
        );
        return forbidden();
    }
    if user_id.is_empty() || user_id == NO_PLAYER {
        tracing::warn!("Rejected rank confirm: invalid/sentinel userId ({user_id})");
        return bad_request();
    }
    let rank = as_integer(js_number(&rank_raw)).filter(|rank| *rank >= 0);
    let Some(rank) = rank else {
        tracing::warn!("Rejected rank confirm: invalid rank ({rank_raw}) for user={user_id} track={progression_id}");
        return bad_request();
    };
    // A host's confirm is checked once it is well formed, before any read of the player's track
    if is_host {
        match refuse_plain(&shared, &req.auth, HostWriteKind::RankConfirm, &user_id).await {
            Ok(None) => {}
            Ok(Some(denial)) => return denial,
            Err(response) => return response,
        }
    }
    let Some(account) = (match req.account(&user_id).await {
        Ok(account) => account,
        Err(response) => return response,
    }) else {
        return no_account(&user_id);
    };

    let (track, owner, host) = (progression_id.clone(), account.clone(), is_host);
    let curves = curves(&shared);
    let outcome = req
        .write(&account, move |tx, repos| {
            // The confirmed rank before this confirm, so a host's confirm is tagged only when it raised it
            let previous = if host {
                Some(repos.progression_tracks.get(tx.conn(), &owner, &track)?.map_or(0, |t| t.confirmed_fremium_rank))
            } else {
                None
            };
            let Some(curve) = curves.get(&track) else {
                tracing::warn!("Rejected rank confirm: unknown track '{track}' has no rank curve (user={})", owner);
                return Ok((Confirmed::Rejected("unknown_track"), previous));
            };
            let Some(required) = curve.required_xp_for_rank(rank) else {
                tracing::warn!("Rejected rank confirm: rank {rank} out of range [0,{}] for track {track}", curve.max_rank);
                return Ok((Confirmed::Rejected("rank_out_of_range"), previous));
            };
            // Rank 0/1 (before the first requirement) never needs an XP check
            if required > 0.0 {
                let persisted = repos.progression_tracks.get(tx.conn(), &owner, &track)?.map_or(0, |t| t.progress);
                if (persisted as f64) < required {
                    let gap = required - persisted as f64;
                    // A host's or a dedicated server's mastery confirm may run ahead by one objective's credit
                    if super::tracks::is_mastery_track(&track) && gap <= super::tracks::MASTERY_CREDIT_PER_OBJECTIVE as f64 {
                        repos.progression_tracks.set_progress_if_greater(tx, &owner, &track, required as i64)?;
                        tracing::info!("[MasteryMon] SYNC user={owner} track={track} from={persisted} to={}", required as i64);
                    } else {
                        tracing::warn!(
                            "Rejected rank confirm: user={owner} track={track} claims rank {rank} (needs {required} cumulative XP) but only has {persisted} persisted"
                        );
                        return Ok((Confirmed::Rejected("insufficient_xp"), previous));
                    }
                }
            }
            let record = repos.progression_tracks.set_confirmed_fremium_rank(tx, &owner, &track, rank)?;
            Ok::<_, RepoError>((Confirmed::Ok(record), previous))
        })
        .await;
    let (confirmed, previous) = match outcome {
        Ok(found) => found,
        Err(response) => return response,
    };
    let updated = match confirmed {
        Confirmed::Rejected(reason) => {
            let status = if reason == "insufficient_xp" { 409 } else { 400 };
            return reply(
                status,
                json!({ "code": null, "message": format!("Rejected: {reason}"), "payload": {} }),
            );
        }
        Confirmed::Ok(record) => record,
    };
    tracing::info!(
        "Confirm public progression rank {rank} for userId {user_id} track {progression_id} - progress preserved at {}",
        updated.progress
    );
    if is_host
        && let Some(previous) = previous
        && updated.confirmed_fremium_rank > previous
    {
        tag_host_write(
            &shared,
            &req.auth,
            HostWriteKind::RankConfirm,
            &user_id,
            json!({ "track": progression_id, "rank": rank, "previousRank": previous, "confirmedRank": updated.confirmed_fremium_rank }),
        )
        .await;
    }
    ok(json!({
        "phx_account_id": user_id,
        "progression_id": progression_id,
        "progress": updated.progress,
        "confirmed_fremium_rank": updated.confirmed_fremium_rank,
        "confirmed_premium_rank": updated.confirmed_premium_rank,
        "confirmed_date": updated.updated_at,
    }))
}
