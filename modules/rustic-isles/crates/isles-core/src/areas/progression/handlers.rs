//! The reads and the smaller writes of the progression area: hunt pass, escalation, encountered content, breadcrumbs,
//! the progression and objective lists, the Slayer's Path and the bootstrap stubs.

use axum::response::Response;
use rifts_seam::HostWriteKind;
use serde_json::{Value, json};

use crate::AccountId;
use crate::hooks::ObjectiveRow;
use crate::http::Call;
use crate::repo::progression::{BreadcrumbsRecord, EncounteredContentRecord, now_iso};
use crate::repo::{RepoError, RepoResult};

use super::auth::{is_authoritative_server_for, refuse_plain, tag_host_write};
use super::escalation::{TuningLoad, load_tuning, merge_escalation, merge_host_escalation, record_to_payload};
use super::journey::{
    HostJourneyPlan, SlayersPathGraph, StripReason, grant_rewards, graph, mask_nodes, merge_nodes,
    newly_unlocked_node_ids, nodes_shown_locked, plan_host_journey, restore_shown_locked, unlocked_node_ids,
};
use super::ports::Ports;
use super::reforge::finish_stale_reforges;
use super::req::{Req, no_account};
use super::tracks::{
    DEFAULT_PROGRESSION_IDS, NO_PLAYER, PROGRESSION_ID_MAX_LENGTH, active_huntpass_id, served_huntpass_id,
};
use super::util::{empty, forbidden, js_number, ok, reply};

fn now_ms() -> i64 {
    crate::game_data::season_window::now_ms()
}

macro_rules! prepare {
    ($call:expr, $ports:expr) => {
        match Req::new($call, $ports).await {
            Ok(req) => req,
            Err(response) => return response,
        }
    };
}

macro_rules! tried {
    ($result:expr) => {
        match $result {
            Ok(value) => value,
            Err(response) => return response,
        }
    };
}

// ---------------------------------------------------------------------------------------------------------------
// /huntpass

/// `GET /huntpass/:userId`: `huntpass_payload` is the version's.
pub async fn huntpass(call: Call, ports: Ports) -> Response {
    let req = prepare!(call, ports);
    let id = served_huntpass_id(&req.shared);
    tracing::info!("Huntpass (stubbed) -> huntpass_id={id}");
    ok(req.shared.version.hooks.huntpass_payload(&id))
}

// ---------------------------------------------------------------------------------------------------------------
// /escalation

pub async fn escalation_get(call: Call, ports: Ports) -> Response {
    let req = prepare!(call, ports);
    let season = req.param("escalationSeason").to_owned();
    let user_id = req.param("userId").to_owned();
    if !req.auth.may_read_escalation(&user_id) {
        tracing::warn!(
            "Rejected cross-account escalation read actor={} target={user_id}",
            req.auth.user_id.as_deref().unwrap_or("unknown")
        );
        return empty(403);
    }
    let stored = tried!(
        req.read(&user_id, move |conn, repos, account| match account {
            Some(account) => repos.escalation_progress.get(conn, account, &season),
            None => Ok(None),
        })
        .await
    );
    ok(record_to_payload(stored.as_ref()))
}

const HOST_ESCALATION_SAVE_ATTEMPTS: usize = 8;

pub async fn escalation_post(call: Call, ports: Ports) -> Response {
    let req = prepare!(call, ports);
    let shared = req.shared.clone();
    let season = req.param("escalationSeason").to_owned();
    let user_id = req.param("userId").to_owned();
    // A player-host runtime may write too, through the hostWrites hooks and a merge that never lowers progress
    let is_host_write = !req.auth.may_write_escalation(&user_id) && req.auth.is_host_escalation_writer();
    if !req.auth.may_write_escalation(&user_id) && !is_host_write {
        tracing::warn!(
            "Rejected cross-account or player-host escalation write actor={} target={user_id}",
            req.auth.user_id.as_deref().unwrap_or("unknown")
        );
        return empty(403);
    }
    if is_host_write {
        match refuse_plain(&shared, &req.auth, HostWriteKind::Escalation, &user_id).await {
            Ok(None) => {}
            Ok(Some(denied)) => {
                tracing::warn!(
                    "Rejected host escalation write actor={} target={user_id}",
                    req.auth.user_id.as_deref().unwrap_or("unknown")
                );
                return denied;
            }
            Err(response) => return response,
        }
    }
    // A season the game has no table for cannot be real progress; judged after the hooks so a host that may not
    // write this account learns nothing about seasons
    let loaded = if is_host_write {
        load_tuning(&shared)
    } else {
        TuningLoad::Missing
    };
    if is_host_write {
        match &loaded {
            TuningLoad::Loaded(tuning) if !tuning.seasons.contains_key(&season) => {
                let shown = Value::String(season.chars().take(64).collect()).to_string();
                tracing::warn!(
                    "Rejected host escalation write for unknown season {shown} actor={} target={user_id}",
                    req.auth.user_id.as_deref().unwrap_or("unknown")
                );
                return reply(
                    403,
                    json!({ "code": "ESCALATION_SEASON_UNKNOWN", "message": "This escalation season is not known.", "payload": null }),
                );
            }
            TuningLoad::Invalid(problem) => {
                tracing::warn!(
                    "[Escalation] game-data/escalation_tuning.json rejected ({problem}): host escalation writes keep the never-lower merge"
                );
            }
            _ => {}
        }
    }
    let Some(account) = tried!(req.account(&user_id).await) else {
        return no_account(&user_id);
    };
    // The gameserver posts the season data it wants persisted; omitted fields keep their previous value
    let update = req.body.json().cloned().unwrap_or_else(|| json!({}));
    let (owner, season_in, limits) = (account.clone(), season.clone(), loaded.tuning().cloned());
    let saved = tried!(
        req.write(&account, move |tx, repos| {
            for _ in 0..HOST_ESCALATION_SAVE_ATTEMPTS {
                let previous = repos.escalation_progress.get(tx.conn(), &owner, &season_in)?;
                let now = now_iso();
                let (record, summary) = if is_host_write {
                    let Some((record, summary)) = merge_host_escalation(
                        &owner,
                        &season_in,
                        previous.as_ref(),
                        Some(&update),
                        &now,
                        limits.as_ref(),
                    ) else {
                        return Err(RepoError::Data("escalation season is not in the tuning table".into()));
                    };
                    (record, Some(summary))
                } else {
                    (
                        merge_escalation(&owner, &season_in, previous.as_ref(), Some(&update), &now),
                        None,
                    )
                };
                if let Some(saved) = repos.escalation_progress.save_if_version(
                    tx,
                    &record,
                    previous.as_ref().map(|p| p.update_version),
                )? {
                    return Ok((saved, summary));
                }
            }
            Err(RepoError::Data("escalation progress save remained contended".into()))
        })
        .await
    );
    let (saved, summary) = saved;
    if let Some(summary) = summary {
        let mut tag = serde_json::Map::new();
        tag.insert("season".into(), season.clone().into());
        tag.extend(summary.to_json());
        tag_host_write(
            &shared,
            &req.auth,
            HostWriteKind::Escalation,
            &user_id,
            Value::Object(tag),
        )
        .await;
        if summary.level_xp_rejected.is_some() || summary.talents_rejected.is_some() {
            tracing::warn!(
                "Host escalation write partly refused season={season} user={user_id} levelXp={} talents={}",
                summary.level_xp_rejected.as_deref().unwrap_or("ok"),
                summary.talents_rejected.as_deref().unwrap_or("ok")
            );
        }
    }
    tracing::info!(
        "Escalation progress saved season={season} user={user_id} level={} version={}",
        saved.escalation_level,
        saved.update_version
    );
    ok(record_to_payload(Some(&saved)))
}

// ---------------------------------------------------------------------------------------------------------------
// /encountered-content

fn empty_encountered(content_types: &Value) -> Response {
    let types: Vec<Value> = content_types
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|t| json!({ "content": [], "content_type": t }))
                .collect()
        })
        .unwrap_or_default();
    ok(json!({ "content_types": types, "success": true }))
}

/// `IsCharacterOwnedByUser`.
async fn character_owned(req: &Req, user_id: &str, character_id: &str) -> Result<bool, Response> {
    if user_id.trim().is_empty() || character_id.trim().is_empty() || user_id == NO_PLAYER || character_id == NO_PLAYER
    {
        return Ok(false);
    }
    let Some(account) = req.account(user_id).await? else {
        return Ok(false);
    };
    Ok(req.ports.characters.owns(&req.shared, &account, character_id).await)
}

/// `RequireCharacterOwnership`: `Some(reply)` refuses; no player is let through.
async fn require_character(req: &Req, user_id: &str, character_id: &str) -> Result<Option<Response>, Response> {
    if user_id == NO_PLAYER || character_owned(req, user_id, character_id).await? {
        return Ok(None);
    }
    tracing::warn!(
        "Rejected character-scoped progression read for characterId {character_id}: character is not owned by userId {user_id}"
    );
    Ok(Some(reply(
        403,
        json!({ "code": "forbidden", "message": "Character is not owned by the authenticated account.", "payload": null }),
    )))
}

/// `QueryEncounteredContent`: for each of the eight categories asked for, the content ids stored under it.
async fn query_encountered(
    req: &Req,
    user_id: &str,
    character_id: &str,
    categories: &Value,
) -> Result<Vec<Value>, Response> {
    let Some(categories) = categories.as_array() else {
        // `.includes` on anything else throws in Node: a 500
        return Err(empty(500));
    };
    let character = character_id.to_owned();
    let stored = req
        .read(user_id, move |conn, repos, account| match account {
            Some(account) => repos.encountered_content.find(conn, &character, account),
            None => Ok(None),
        })
        .await?;
    let content: Vec<Value> = match stored {
        None => Vec::new(),
        Some(row) => match serde_json::from_str::<Value>(&row.encountered_content) {
            Ok(Value::Array(items)) => items,
            _ => return Err(empty(500)),
        },
    };
    let wants = |category: f64| categories.iter().any(|c| c.as_f64() == Some(category));
    let mut out = Vec::new();
    for category in 0..8 {
        if !wants(f64::from(category)) {
            continue;
        }
        let items: Vec<Value> = content
            .iter()
            .filter(|c| c.get("category").and_then(Value::as_f64) == Some(f64::from(category)))
            .filter_map(|c| c.get("content").cloned())
            .collect();
        out.push(json!({ "content": items, "content_type": category }));
    }
    Ok(out)
}

fn encountered_reply(content: Vec<Value>) -> Response {
    ok(json!({ "content_types": content, "success": true }))
}

pub async fn encountered_get(call: Call, ports: Ports) -> Response {
    let req = prepare!(call, ports);
    let character_id = req.param("characterId").to_owned();
    let content_type = js_number(req.param("contentType"));
    tracing::info!(
        "Querying encountered content for userId {} and characterId {character_id}",
        req.auth.user_id.as_deref().unwrap_or("undefined")
    );
    let Some(requestor) = req.auth.user_id.clone() else {
        return empty_encountered(&json!([content_type]));
    };
    if let Some(refused) = tried!(require_character(&req, &requestor, &character_id).await) {
        return refused;
    }
    let content = tried!(query_encountered(&req, &requestor, &character_id, &json!([content_type])).await);
    encountered_reply(content)
}

/// `POST /encountered-content/query/:characterId` and `/:characterId/query`.
pub async fn encountered_query(call: Call, ports: Ports) -> Response {
    let req = prepare!(call, ports);
    let character_id = req.param("characterId").to_owned();
    // `req.body.content_types ?? []` throws on an unparsed body
    if req.body.is_undefined() {
        return empty(500);
    }
    let types = req
        .body
        .get(&["content_types"])
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or_else(|| json!([]));
    tracing::info!(
        "Querying encountered content for userId {} and characterId {character_id}",
        req.auth.user_id.as_deref().unwrap_or("undefined")
    );
    let Some(requestor) = req.auth.user_id.clone() else {
        return empty_encountered(&types);
    };
    if let Some(refused) = tried!(require_character(&req, &requestor, &character_id).await) {
        return refused;
    }
    let content = tried!(query_encountered(&req, &requestor, &character_id, &types).await);
    encountered_reply(content)
}

/// `POST /encountered-content/query`: no character, no content.
pub async fn encountered_query_anonymous(call: Call, ports: Ports) -> Response {
    let req = prepare!(call, ports);
    if req.body.is_undefined() {
        return empty(500);
    }
    let types = req
        .body
        .get(&["content_types"])
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or_else(|| json!([]));
    tracing::info!("Querying encountered content without characterId; returning empty content");
    empty_encountered(&types)
}

pub async fn encountered_add(call: Call, ports: Ports) -> Response {
    let req = prepare!(call, ports);
    let character_id = req.param("characterId").to_owned();
    if req.body.is_undefined() {
        return empty(500);
    }
    let content_type = req.body.get(&["content_type"]).cloned();
    let content_id = req.body.get(&["content_id"]).cloned();
    let requestor = req.auth.user_id.clone();
    let (Some(requestor), Some(content_type), Some(content_id)) = (requestor, content_type, content_id) else {
        return ok(json!({}));
    };
    if !tried!(character_owned(&req, &requestor, &character_id).await) {
        tracing::warn!(
            "Rejected encountered-content write for characterId {character_id}: character is not owned by userId {requestor}"
        );
        return empty(403);
    }
    let Some(account) = tried!(req.account(&requestor).await) else {
        return no_account(&requestor);
    };
    let (owner, character) = (account.clone(), character_id.clone());
    tried!(
        req.write(&account, move |tx, repos| {
            let held = repos.encountered_content.find(tx.conn(), &character, &owner)?;
            let mut parsed: Vec<Value> = match &held {
                None => Vec::new(),
                Some(row) => serde_json::from_str(&row.encountered_content)
                    .map_err(|error| RepoError::Data(format!("encountered content: {error}")))?,
            };
            if held.is_none() {
                repos.encountered_content.create(
                    tx,
                    &EncounteredContentRecord {
                        character_id: character.clone(),
                        account: owner.clone(),
                        encountered_content: "[]".into(),
                    },
                )?;
            }
            parsed.push(json!({ "content": content_id, "category": content_type }));
            repos.encountered_content.update_content(
                tx,
                &character,
                &owner,
                &super::util::stringify(&Value::Array(parsed)),
            )
        })
        .await
    );
    ok(json!({}))
}

/// `POST /encountered-content` and `POST /encountered-content/`: the client posts with an empty characterId before
/// its login state settles; acknowledged as a no-op.
pub async fn encountered_noop(call: Call, ports: Ports) -> Response {
    let _req = prepare!(call, ports);
    ok(json!({}))
}

// ---------------------------------------------------------------------------------------------------------------
// /progression/objectives, /progression, /progression/:userId/:progressionId

/// `GET /progression/objectives/:userId`. `GAMESERVER_OBJECTIVES_AS_CONFIG` (an A/B switch) is dropped, RU3c.
pub async fn objectives(call: Call, ports: Ports) -> Response {
    let req = prepare!(call, ports);
    let user_id = req.resolve_user_id();
    if let Some(refused) = req.require_read(&user_id) {
        return refused;
    }
    if user_id == NO_PLAYER {
        tracing::debug!("Objective progression fetched for no-player sentinel - returning empty, not touching DB");
        return ok(json!([]));
    }
    let rows = tried!(
        req.read(&user_id, move |conn, repos, account| match account {
            Some(account) => repos.progression_tracks.get_all_objectives_for_account(conn, account),
            None => Ok(Vec::new()),
        })
        .await
    );
    tracing::info!(
        "Objective progression fetched for userId {user_id} ({} objectives, persisted)",
        rows.len()
    );
    let hooks = req.shared.version.hooks.clone();
    let payload: Vec<Value> = rows
        .iter()
        .map(|row| {
            hooks.objective_row(
                &user_id,
                &ObjectiveRow {
                    objective_id: row.objective_id.clone(),
                    value: row.value,
                    completed_count: row.completed_count,
                    created_date: row.updated_at.clone(),
                    last_modified_date: row.updated_at.clone(),
                },
            )
        })
        .collect();
    ok(Value::Array(payload))
}

fn zero_track(user_id: &str, progression_id: &str, now: &str) -> Value {
    json!({
        "phx_account_id": user_id,
        "progression_id": progression_id,
        "progress": 0,
        "confirmed_fremium_rank": 0,
        "confirmed_premium_rank": 0,
        "confirmed_date": now,
    })
}

/// `GET /progression/:userId`.
pub async fn progression_list(call: Call, ports: Ports) -> Response {
    let req = prepare!(call, ports);
    let shared = req.shared.clone();
    let user_id = req.resolve_user_id();
    if let Some(refused) = req.require_read(&user_id) {
        return refused;
    }
    let mut known: Vec<String> = DEFAULT_PROGRESSION_IDS.iter().map(|id| (*id).to_owned()).collect();
    known.push(active_huntpass_id(&shared).to_owned());
    if user_id == NO_PLAYER {
        tracing::debug!("Progression fetched for no-player sentinel - returning empty zero-baseline, not touching DB");
        let now = now_iso();
        return ok(Value::Array(
            known.iter().map(|id| zero_track(&user_id, id, &now)).collect(),
        ));
    }
    // A weapon reforge whose follow-ups never arrived is finished before the load (banked XP back, +1), so the arriving
    // server starts from the right level. Only on an authoritative server's load; failures never block it.
    if super::reforge::reforge_mode(&shared) == super::reforge::ReforgeMode::On {
        let authoritative =
            tried!(is_authoritative_server_for(&shared, &req.auth, &user_id, HostWriteKind::Reforge).await);
        if authoritative && let Some(account) = tried!(req.account(&user_id).await) {
            let (owner, user) = (account.clone(), user_id.clone());
            let repaired = req
                .write(&account, move |tx, repos| {
                    finish_stale_reforges(tx, repos, &owner, &user, now_ms())
                })
                .await;
            match repaired {
                Ok(finished) if !finished.is_empty() && req.auth.is_host() => {
                    tracing::info!(
                        "[HostParity] reforge repair at a realm load user={user_id} tracks=[{}]",
                        finished.join(", ")
                    );
                    tag_host_write(
                        &shared,
                        &req.auth,
                        HostWriteKind::Reforge,
                        &user_id,
                        json!({ "staleReforgesFinished": finished }),
                    )
                    .await;
                }
                Ok(_) => {}
                Err(_) => tracing::error!("[Reforge] stale-reforge recovery failed for {user_id}"),
            }
        }
    }
    let rows = tried!(
        req.read(&user_id, move |conn, repos, account| match account {
            Some(account) => repos.progression_tracks.get_all_for_account(conn, account),
            None => Ok(Vec::new()),
        })
        .await
    );
    // Known tracks first, in the declared order (zero rows for any never granted), then every other stored track
    let now = now_iso();
    let mut payload: Vec<Value> = Vec::new();
    let mut remaining: Vec<&crate::repo::progression::ProgressionTrackRecord> = rows.iter().collect();
    for track in &known {
        let position = remaining.iter().position(|r| &r.progression_id == track);
        let row = position.map(|p| remaining.remove(p));
        payload.push(match row {
            Some(row) => track_json(&user_id, &row.progression_id, row),
            None => zero_track(&user_id, track, &now),
        });
    }
    for row in remaining {
        payload.push(track_json(&user_id, &row.progression_id, row));
    }
    tracing::info!(
        "Progression fetched for userId {user_id} ({} tracks, persisted)",
        payload.len()
    );
    ok(Value::Array(payload))
}

fn track_json(user_id: &str, progression_id: &str, row: &crate::repo::progression::ProgressionTrackRecord) -> Value {
    json!({
        "phx_account_id": user_id,
        "progression_id": progression_id,
        "progress": row.progress,
        "confirmed_fremium_rank": row.confirmed_fremium_rank,
        "confirmed_premium_rank": row.confirmed_premium_rank,
        "confirmed_date": row.updated_at,
    })
}

/// `GET /progression/:userId/:progressionId` (1.14.7): the single-track read the weapon reforge uses.
pub async fn progression_single(call: Call, ports: Ports) -> Response {
    // `next("route")` before any auth: these first segments belong to the routes registered above, and the path has
    // no other route after this one
    if matches!(
        call.params.get("userId"),
        Some("objectives" | "tracked_objectives" | "config")
    ) {
        return empty(404);
    }
    let req = prepare!(call, ports);
    let user_id = req.param("userId").to_owned();
    let progression_id = req.param("progressionId").to_owned();
    if let Some(refused) = req.require_read(&user_id) {
        return refused;
    }
    let now = now_iso();
    if user_id == NO_PLAYER || progression_id.is_empty() || progression_id.chars().count() > PROGRESSION_ID_MAX_LENGTH {
        return ok(zero_track(&user_id, &progression_id, &now));
    }
    let wanted = progression_id.clone();
    let row = tried!(
        req.read(&user_id, move |conn, repos, account| match account {
            Some(account) => repos.progression_tracks.get(conn, account, &wanted),
            None => Ok(None),
        })
        .await
    );
    tracing::info!(
        "Progression track fetched user={user_id} track={progression_id} progress={}{}",
        row.as_ref().map_or(0, |r| r.progress),
        row.as_ref()
            .and_then(|r| r.reforge.as_ref())
            .map_or_else(String::new, |r| format!(
                " reforge={}",
                if r.open { "open" } else { "closed" }
            ))
    );
    ok(match row {
        Some(row) => track_json(&user_id, &progression_id, &row),
        None => zero_track(&user_id, &progression_id, &now),
    })
}

// ---------------------------------------------------------------------------------------------------------------
// Stubs

pub async fn weapon_tracker(call: Call, ports: Ports) -> Response {
    let req = prepare!(call, ports);
    let user_id = req.param("userId").to_owned();
    if let Some(refused) = req.require_read(&user_id) {
        return refused;
    }
    tracing::info!(
        "Weapon trackers for userId {user_id} item {} (1.14.7 stub)",
        req.param("itemId")
    );
    ok(json!({ "TrackersInfo": [] }))
}

pub async fn tracked_objectives_get(call: Call, ports: Ports) -> Response {
    let req = prepare!(call, ports);
    let account = req.param("phxAccountId").to_owned();
    if let Some(refused) = req.require_read(&account) {
        return refused;
    }
    tracing::info!("GET tracked objectives for phx_account_id={account} (bootstrap)");
    ok(json!({
        "current_set": "quest_slayer_links",
        "omitted_quests": [],
        "phx_account_id": account,
        "tracked_craftables": [],
        "tracked_quests": []
    }))
}

pub async fn tracked_objectives_post(call: Call, ports: Ports) -> Response {
    let req = prepare!(call, ports);
    let account = req.param("phxAccountId").to_owned();
    if let Some(refused) = req.require_read(&account) {
        return refused;
    }
    tracing::info!("POST tracked objectives for phx_account_id={account} (bootstrap, discarding update)");
    ok(Value::Null)
}

// ---------------------------------------------------------------------------------------------------------------
// /breadcrumbs

fn empty_breadcrumbs() -> Response {
    ok(json!({ "breadcrumbs": [], "updateVersion": 0 }))
}

pub async fn breadcrumbs_anonymous(call: Call, ports: Ports) -> Response {
    let _req = prepare!(call, ports);
    tracing::info!("Requested breadcrumbs without characterId; returning empty breadcrumbs");
    empty_breadcrumbs()
}

pub async fn breadcrumbs_get(call: Call, ports: Ports) -> Response {
    let req = prepare!(call, ports);
    let character_id = req.param("characterId").to_owned();
    tracing::info!("Requested breadcrumbs for characterId {character_id}");
    let Some(requestor) = req.auth.user_id.clone() else {
        return empty_breadcrumbs();
    };
    if !tried!(character_owned(&req, &requestor, &character_id).await) {
        tracing::warn!(
            "Rejected breadcrumbs read for characterId {character_id}: character is not owned by userId {requestor}"
        );
        return empty(403);
    }
    let Some(account) = tried!(req.account(&requestor).await) else {
        return no_account(&requestor);
    };
    let (owner, character) = (account.clone(), character_id.clone());
    let payload = tried!(
        req.write(&account, move |tx, repos| {
            match repos.breadcrumbs.find(tx.conn(), &character, &owner)? {
                None => {
                    tracing::info!("Creating new breadcrumbs entry for character {character}");
                    repos.breadcrumbs.create(
                        tx,
                        &BreadcrumbsRecord {
                            character_id: character.clone(),
                            account: owner.clone(),
                            breadcrumbs: "[]".into(),
                            update_version: 0,
                        },
                    )?;
                    Ok(json!({ "breadcrumbs": [], "updateVersion": 0 }))
                }
                Some(row) => {
                    let parsed: Value = serde_json::from_str(&row.breadcrumbs)
                        .map_err(|error| RepoError::Data(format!("breadcrumbs: {error}")))?;
                    Ok(json!({ "breadcrumbs": parsed, "updateVersion": row.update_version }))
                }
            }
        })
        .await
    );
    ok(payload)
}

pub async fn breadcrumbs_post(call: Call, ports: Ports) -> Response {
    let req = prepare!(call, ports);
    let character_id = req.param("characterId").to_owned();
    // `req.body.breadcrumbs` throws on an unparsed body
    if req.body.is_undefined() {
        return empty(500);
    }
    let from_user = req.body.get(&["breadcrumbs"]).cloned();
    let version = req.body.get(&["updateVersion"]).cloned();
    tracing::info!("Setting breadcrumbs for characterId {character_id}");
    let requestor = req.auth.user_id.clone().unwrap_or_default();
    if !tried!(character_owned(&req, &requestor, &character_id).await) {
        tracing::warn!(
            "Rejected breadcrumbs write for characterId {character_id}: character is not owned by userId {requestor}"
        );
        return empty(403);
    }
    let Some(account) = tried!(req.account(&requestor).await) else {
        return no_account(&requestor);
    };
    // A missing breadcrumbs list is stored as [] and a missing version as 0; the answer omits what was not sent
    let stored_json = from_user
        .as_ref()
        .map_or_else(|| "[]".to_owned(), super::util::stringify);
    let stored_version = version.as_ref().and_then(Value::as_f64).map_or(0, |v| v as i64);
    let (owner, character) = (account.clone(), character_id.clone());
    tried!(
        req.write(&account, move |tx, repos| {
            if repos.breadcrumbs.find(tx.conn(), &character, &owner)?.is_none() {
                tracing::info!("Creating new breadcrumbs entry for character {character}");
                repos.breadcrumbs.create(
                    tx,
                    &BreadcrumbsRecord {
                        character_id: character,
                        account: owner,
                        breadcrumbs: stored_json,
                        update_version: stored_version,
                    },
                )
            } else {
                tracing::info!(
                    "Updating breadcrumbs entry for character {character} with updateVersion {stored_version}"
                );
                repos
                    .breadcrumbs
                    .update(tx, &character, &owner, &stored_json, stored_version)
            }
        })
        .await
    );
    let mut payload = serde_json::Map::new();
    if let Some(b) = from_user {
        payload.insert("breadcrumbs".into(), b);
    }
    if let Some(v) = version {
        payload.insert("updateVersion".into(), v);
    }
    ok(Value::Object(payload))
}

// ---------------------------------------------------------------------------------------------------------------
// /pjm

/// `GetPlayerJourney` for a user: `{ nodes, update_version }`, `None` without a save.
async fn player_journey(req: &Req, user_id: &str) -> Result<Option<(Value, i64)>, Response> {
    if user_id.is_empty() || user_id == NO_PLAYER {
        return Ok(None);
    }
    let row = req
        .read(user_id, move |conn, repos, account| match account {
            Some(account) => repos.player_journey.find(conn, account),
            None => Ok(None),
        })
        .await?;
    Ok(row.map(|row| {
        (
            serde_json::from_str::<Value>(&row.nodes).unwrap_or_else(|_| json!({})),
            row.update_version,
        )
    }))
}

fn masked_payload(req: &Req, nodes: &Value, version: i64) -> Value {
    json!({ "nodes": mask_nodes(nodes, &nodes_shown_locked(&req.shared)), "update_version": version })
}

pub async fn pjm_get(call: Call, ports: Ports) -> Response {
    let req = prepare!(call, ports);
    let shared = req.shared.clone();
    let user_id = req.param("userId").to_owned();
    if let Some(refused) = req.require_read(&user_id) {
        return refused;
    }
    if user_id == NO_PLAYER {
        tracing::debug!(
            "GET /pjm called with no-player sentinel (userId=INVALID) - returning empty player journey map, not touching DB"
        );
        return ok(json!({ "nodes": {}, "update_version": 1 }));
    }
    let Some((nodes, version)) = tried!(player_journey(&req, &user_id).await) else {
        tracing::info!("Player journey map fetched for userId {user_id} (no save yet - empty baseline)");
        return ok(json!({ "nodes": {}, "update_version": 1 }));
    };
    // Backfill rewards missed before the server knew how to apply unlock effects: only for an authoritative server's
    // read (a realm server's for a player in its session too), never for a bearer-only client's GET
    if tried!(is_authoritative_server_for(&shared, &req.auth, &user_id, HostWriteKind::SlayersPath).await)
        && let Some(account) = tried!(req.account(&user_id).await)
    {
        let eligible = unlocked_node_ids(&nodes);
        tracing::info!(
            "[slayers-path] login reconciliation evaluating {} reward-eligible nodes for {user_id}",
            eligible.len()
        );
        let granted = grant_rewards(&shared, req.ports.inventory.as_ref(), &account, &user_id, &eligible).await;
        if !granted.is_empty() {
            tracing::info!(
                "Slayer's Path login reconciliation granted [{}] to {user_id}",
                granted.join(", ")
            );
            if req.auth.is_host() {
                tracing::info!(
                    "[HostParity] Slayer's Path catch-up at a realm login user={user_id} granted=[{}]",
                    granted.join(", ")
                );
                tag_host_write(
                    &shared,
                    &req.auth,
                    HostWriteKind::SlayersPath,
                    &user_id,
                    json!({ "loginCatchUp": granted }),
                )
                .await;
            }
        }
    }
    tracing::info!(
        "Player journey map fetched for userId {user_id} (persisted: {} nodes, v{version})",
        nodes.as_object().map_or(0, serde_json::Map::len)
    );
    ok(masked_payload(&req, &nodes, version))
}

pub async fn pjm_anonymous(call: Call, ports: Ports) -> Response {
    let req = prepare!(call, ports);
    if let Some(user_id) = req.auth.user_id.clone().filter(|id| id != NO_PLAYER)
        && let Some((nodes, version)) = tried!(player_journey(&req, &user_id).await)
    {
        tracing::info!("GET /pjm - returning persisted map for {user_id}, v{version}");
        return ok(masked_payload(&req, &nodes, version));
    }
    tracing::info!("GET /pjm - no persisted map; returning captured Slayer's Path bootstrap payload");
    let Some(path) = req.shared.game_data.backend_file("slayers_path.json") else {
        tracing::error!("slayers_path.json: no game data folder");
        return empty(500);
    };
    match super::tracks::read_game_data(&path) {
        Ok(payload) => reply(200, payload),
        Err(error) => {
            tracing::error!("slayers_path.json: {error}");
            empty(500)
        }
    }
}

struct Saved {
    nodes: Value,
    update_version: i64,
    newly_unlocked: Vec<String>,
    plan: Option<HostJourneyPlan>,
}

/// `SavePlayerJourney` with the route's vet: stale saves may only add nodes, the masked node keeps its stored
/// unlock, and a host's unlocks are checked against the graph. One transaction, so no compare-and-swap retry.
fn save_player_journey(
    tx: &mut crate::repo::Tx<'_>,
    repos: &crate::repo::Repositories,
    account: &AccountId,
    nodes: &Value,
    update_version: f64,
    host_graph: Option<&SlayersPathGraph>,
    shown_locked: &[&str],
) -> RepoResult<Saved> {
    let version = if update_version.is_finite() {
        update_version
    } else {
        1.0
    };
    let safe_nodes = if nodes.is_object() { nodes.clone() } else { json!({}) };
    let vet = |stored: &Value, merged: &Value| -> (Value, Option<HostJourneyPlan>) {
        let restored = Value::Object(restore_shown_locked(stored, merged, shown_locked));
        match host_graph {
            None => (restored, None),
            Some(graph) => {
                let planned = plan_host_journey(stored, &restored, graph);
                (Value::Object(planned.nodes.clone()), Some(planned))
            }
        }
    };
    let existing = repos.player_journey.find(tx.conn(), account)?;
    let Some(existing) = existing else {
        let (first, plan) = vet(&json!({}), &safe_nodes);
        let record = crate::repo::progression::PlayerJourneyRecord {
            account: account.clone(),
            nodes: super::util::stringify(&first),
            update_version: version as i64,
            revision: 0,
        };
        if !repos.player_journey.create_if_absent(tx, &record)? {
            return Err(RepoError::Data(
                "player journey row appeared inside its own write".into(),
            ));
        }
        tracing::info!("Creating player journey row for userId {account}");
        let newly_unlocked = newly_unlocked_node_ids(&json!({}), &first);
        return Ok(Saved {
            nodes: first,
            update_version: version as i64,
            newly_unlocked,
            plan,
        });
    };
    let stored: Value = serde_json::from_str(&existing.nodes).unwrap_or_else(|_| json!({}));
    let (merged, effective) = merge_nodes(&stored, &safe_nodes, version, existing.update_version as f64);
    let (merged_nodes, plan) = vet(&stored, &Value::Object(merged));
    if version <= existing.update_version as f64 {
        tracing::warn!(
            "[pjm-stale] Stale journey save for userId {account}: incoming v{version} <= stored v{} - merged without reverting stored unlocks (kept v{effective}, {} nodes)",
            existing.update_version,
            merged_nodes.as_object().map_or(0, serde_json::Map::len)
        );
    }
    let newly_unlocked = newly_unlocked_node_ids(&stored, &merged_nodes);
    let applied = repos.player_journey.update_if_revision(
        tx,
        account,
        &super::util::stringify(&merged_nodes),
        effective as i64,
        existing.revision,
    )?;
    if !applied {
        return Err(RepoError::Data("player journey update conflicted".into()));
    }
    Ok(Saved {
        nodes: merged_nodes,
        update_version: effective as i64,
        newly_unlocked,
        plan,
    })
}

pub async fn pjm_post(call: Call, ports: Ports) -> Response {
    let req = prepare!(call, ports);
    let shared = req.shared.clone();
    let user_id = req.param("userId").to_owned();
    let is_host = req.auth.is_host();
    // This route has inventory-granting side effects: only a game server (or an accepted host runtime) may call it
    if !is_host && !req.auth.is_gameserver {
        tracing::warn!("Rejected player journey mutation without gameserver auth (userId={user_id})");
        return forbidden();
    }
    if is_host {
        match refuse_plain(&shared, &req.auth, HostWriteKind::SlayersPath, &user_id).await {
            Ok(None) => {}
            Ok(Some(denial)) => return denial,
            Err(response) => return response,
        }
    }
    let version_sent = req.body.get(&["update_version"]).filter(|v| !v.is_null());
    if user_id == NO_PLAYER {
        tracing::debug!("POST /pjm called with no-player sentinel (userId=INVALID) - no-op update, not touching DB");
        return ok(json!({ "nodes": {}, "update_version": version_sent.cloned().unwrap_or_else(|| json!(1)) }));
    }
    let nodes = req
        .body
        .get(&["nodes"])
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or_else(|| json!({}));
    // `Number.isFinite(x)` takes no coercion: a version that is not a number is 1
    let version = version_sent.map_or(1.0, |v| v.as_f64().unwrap_or(f64::NAN));
    let Some(account) = tried!(req.account(&user_id).await) else {
        return no_account(&user_id);
    };
    let host_graph = is_host.then(|| graph(&shared));
    let shown: Vec<&'static str> = nodes_shown_locked(&shared);
    let (owner, nodes_in) = (account.clone(), nodes.clone());
    let saved = tried!(
        req.write(&account, move |tx, repos| {
            save_player_journey(tx, repos, &owner, &nodes_in, version, host_graph.as_ref(), &shown)
        })
        .await
    );

    req.ports.activity.slayers_path_unlocks(
        &shared,
        &user_id,
        &saved.newly_unlocked,
        if is_host { "host" } else { "gameserver" },
    );
    tracing::info!(
        "Player journey map SAVED for userId {user_id} ({} nodes, v{})",
        nodes.as_object().map_or(0, serde_json::Map::len),
        if version.is_finite() {
            version.to_string()
        } else {
            "1".into()
        }
    );
    // An arrival save that changes no node status (the usual one) is not a write worth a tag
    if let Some(plan) = &saved.plan
        && (!plan.unlocked.is_empty() || !plan.stripped.is_empty() || !plan.relocked.is_empty())
    {
        if !plan.stripped.is_empty() || !plan.relocked.is_empty() {
            tracing::warn!(
                "[slayers-path] host journey save for {user_id}: kept stored status for stripped=[{}] relocked=[{}]",
                plan.stripped
                    .iter()
                    .map(|(id, why)| format!("{id}({})", why.as_str()))
                    .collect::<Vec<_>>()
                    .join(", "),
                plan.relocked.join(", ")
            );
        }
        let mut tag = serde_json::Map::new();
        tag.insert("unlocked".into(), plan.unlocked.clone().into());
        let stripped: Vec<String> = plan
            .stripped
            .iter()
            .map(|(id, why): &(String, StripReason)| format!("{id}:{}", why.as_str()))
            .collect();
        tag.insert("stripped".into(), stripped.into());
        if !plan.relocked.is_empty() {
            tag.insert("relocked".into(), plan.relocked.clone().into());
        }
        tag_host_write(
            &shared,
            &req.auth,
            HostWriteKind::SlayersPath,
            &user_id,
            Value::Object(tag),
        )
        .await;
    }
    // Reconcile ALL unlocked reward nodes after every save, so a reward that failed after the node save is retried
    let granted = grant_rewards(
        &shared,
        req.ports.inventory.as_ref(),
        &account,
        &user_id,
        &unlocked_node_ids(&saved.nodes),
    )
    .await;
    if !granted.is_empty() {
        tracing::info!(
            "Slayer's Path reconciliation granted [{}] to {user_id}",
            granted.join(", ")
        );
    }
    // newlyUnlocked is internal: the answer is exactly { nodes, update_version }
    ok(masked_payload(&req, &saved.nodes, saved.update_version))
}
