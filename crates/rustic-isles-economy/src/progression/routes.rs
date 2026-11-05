// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The 25 progression handlers (progression spec §3) and the session supplies seam (§4.14).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::response::Response;
use http::StatusCode;
use module_api::network::HuntRewards;
use module_api::{Actor, ActorContext, BoxFuture, GlobalId, IdKind};
use rifts_seam::{HostWrite, HostWriteKind, HostWriteOptions, RuntimeAuth};
use rustic_isles::{Call, Handlers, Method, ObjectiveRow, Params, Service, Shared, read_body};
use rustic_isles_auth::{GameAccount, GameLogin};
use rustic_isles_store::repo::progression::{
    BreadcrumbsRecord, EncounteredContentRecord, EscalationProgressRecord, PlayerJourneyRecord, ProgressionGrantRecord,
    ProgressionTrackRecord,
};
use rustic_isles_store::{AccountId, IslesStore, RepoError, Repositories, Tx, UserId};
use serde_json::{Map, Value};
use service_kit::{js, jsjson};

use crate::caller;
use crate::inventory::{TxContext, TxOptions, TxOutcome, TxRequest, get_inventory, run_inventory_transaction};
use crate::multipliers::{multiply_xp_grant, read_test_multipliers};
use crate::progression::ProgressionPorts;
use crate::progression::escalation::{self, Tuning, escalation_payload, merge_escalation, merge_host_escalation};
use crate::progression::journey::{self, Graph};
use crate::progression::mastery::{self, HostLimits, plan_mastery_report, report_is_valid};
use crate::progression::reforge::{
    ReforgeMode, ReforgePlan, classify_grant, finish_stale_reforges, mark_reforge_use_as, reforge_mode,
};
use crate::progression::supplies::{apply_session_supplies, parse_session_supplies};
use crate::progression::tracks::{
    self, DEFAULT_PROGRESSION_IDS, MAX_PROGRESSION_GRANT_AMOUNT, NO_PLAYER, curves, experience_track_for_prestige,
    grant_request_hash, is_huntpass_xp_track, is_prestige_track, is_reforgeable_track, is_valid_grant_id,
    xp_track_refusal,
};
use crate::tx::guarded;
use crate::util::{
    coded, empty, envelope, first_chars, js_number_text, log_token, now_iso, number_value, obj, ok, printable,
    utf16_len,
};

const ROUTES: [(Method, &str); 25] = [
    (Method::Get, "/escalation/:escalationSeason/:userId"),
    (Method::Post, "/escalation/:escalationSeason/:userId"),
    (Method::Get, "/huntpass/:userId"),
    (Method::Get, "/encountered-content/:characterId/:contentType"),
    (Method::Post, "/encountered-content/query/:characterId"),
    (Method::Post, "/encountered-content/query"),
    (Method::Post, "/encountered-content/:characterId/query"),
    (Method::Post, "/encountered-content/:characterId"),
    (Method::Post, "/encountered-content"),
    (Method::Post, "/encountered-content/"),
    (Method::Get, "/progression/objectives/:userId"),
    (Method::Get, "/breadcrumbs/"),
    (Method::Get, "/breadcrumbs/:characterId"),
    (Method::Post, "/breadcrumbs/:characterId"),
    (Method::Post, "/progression/:userId/:progressionId/:amount"),
    (Method::Post, "/progression/:userId"),
    (Method::Post, "/progression/:userId/:progressionId/:rank/confirm/public"),
    (Method::Get, "/progression/:userId"),
    (Method::Get, "/pjm/:userId"),
    (Method::Get, "/weapon_tracker/:userId/:itemId"),
    (Method::Post, "/pjm/:userId"),
    (Method::Get, "/pjm"),
    (Method::Get, "/progression/tracked_objectives/:phxAccountId"),
    (Method::Post, "/progression/tracked_objectives/:phxAccountId"),
    (Method::Get, "/progression/:userId/:progressionId"),
];

/// The 25 rows in the table's order.
pub fn routes() -> &'static [(Method, &'static str)] {
    &ROUTES
}

pub fn register(handlers: &mut Handlers, ports: ProgressionPorts) {
    let ports = Arc::new(ports);
    let b = Service::Backend;
    macro_rules! on {
        ($i:expr, $handler:ident) => {{
            let ports = Arc::clone(&ports);
            let (method, path) = ROUTES[$i];
            handlers.on(b, method, path, move |call| $handler(call, Arc::clone(&ports)));
        }};
    }
    on!(0, get_escalation);
    on!(1, post_escalation);
    on!(2, get_huntpass);
    on!(3, get_encountered);
    on!(4, query_encountered);
    on!(5, query_encountered_without_character);
    on!(6, query_encountered);
    on!(7, post_encountered);
    on!(8, post_encountered_stub);
    on!(9, post_encountered_stub);
    on!(10, get_objectives);
    on!(11, get_breadcrumbs_stub);
    on!(12, get_breadcrumbs);
    on!(13, post_breadcrumbs);
    on!(14, post_grant);
    on!(15, post_report);
    on!(16, post_confirm);
    on!(17, get_tracks);
    on!(18, get_journey);
    on!(19, get_weapon_tracker);
    on!(20, post_journey);
    on!(21, get_journey_file);
    on!(22, get_tracked_objectives);
    on!(23, post_tracked_objectives);
    on!(24, get_track);
}

macro_rules! answer {
    ($e:expr) => {
        match $e {
            Ok(value) => value,
            Err(response) => return response,
        }
    };
}

/// OK(p): the envelope with `p`.
fn okp(payload: &Value) -> Response {
    envelope(StatusCode::OK, "OK", payload.clone())
}

fn bad() -> Response {
    crate::util::refusal(StatusCode::BAD_REQUEST, "Bad Request")
}

fn forbidden() -> Response {
    crate::util::refusal(StatusCode::FORBIDDEN, "Forbidden")
}

fn conflict() -> Response {
    crate::util::refusal(StatusCode::CONFLICT, "Conflict")
}

fn host_limit() -> Response {
    coded(
        StatusCode::FORBIDDEN,
        "HOST_WRITE_LIMIT",
        "This write is beyond what a hosted server may report.",
    )
}

fn store_error(error: &dyn std::fmt::Display) -> Response {
    tracing::error!("progression: {error}");
    empty(StatusCode::INTERNAL_SERVER_ERROR)
}

/// Who is asking (spec §2.2).
struct Who {
    user: Option<String>,
    gs: bool,
    host: bool,
    roster: Vec<String>,
    capability: Option<RuntimeAuth>,
}

struct Entry {
    shared: Arc<Shared>,
    params: Params,
    request_id: String,
    body: Option<Value>,
    who: Who,
    ports: Arc<ProgressionPorts>,
    headers: http::HeaderMap,
}

async fn enter(call: Call, ports: Arc<ProgressionPorts>) -> Result<Entry, Response> {
    let Call {
        shared,
        params,
        mut request,
        request_id,
        ..
    } = call;
    let body = read_body(&mut request).await?;
    let data = caller::game_auth(&shared, &mut request).await?;
    let runtime = data.runtime.as_ref();
    let who = Who {
        user: data.user_id.clone(),
        gs: data.is_gameserver,
        host: runtime.is_some(),
        roster: runtime.map(|r| r.roster().to_vec()).unwrap_or_default(),
        capability: runtime.and_then(caller::capability),
    };
    Ok(Entry {
        shared,
        params,
        request_id,
        body,
        who,
        ports,
        headers: request.headers().clone(),
    })
}

/// `a.b` on the body: absent for an absent body or a missing key.
fn field<'a>(body: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    body?.as_object()?.get(key)
}

fn non_null(value: Option<&Value>) -> Option<&Value> {
    value.filter(|v| !v.is_null())
}

impl Entry {
    fn param(&self, name: &str) -> String {
        self.params.get(name).unwrap_or("").to_owned()
    }

    fn body(&self) -> Option<&Value> {
        self.body.as_ref()
    }

    fn store(&self) -> Result<Arc<IslesStore>, Response> {
        self.shared.slot::<IslesStore>().ok_or_else(|| {
            tracing::error!("progression: no database is configured");
            empty(StatusCode::INTERNAL_SERVER_ERROR)
        })
    }

    fn user_is(&self, target: &str) -> bool {
        self.who.user.as_deref() == Some(target)
    }

    fn may_read(&self, target: &str) -> bool {
        self.who.gs || self.user_is(target)
    }

    fn may_report(&self, target: &str) -> bool {
        self.who.gs || (!self.who.host && self.user_is(target))
    }

    /// `ResolveRequestUserId`
    fn resolved_user(&self) -> String {
        let param = self.param("userId");
        if !param.is_empty() {
            return param;
        }
        if let Some(account) = field(self.body(), "accountId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            return account.to_owned();
        }
        self.who.user.clone().unwrap_or_else(|| NO_PLAYER.to_owned())
    }

    fn read_gate(&self, target: &str) -> Result<(), Response> {
        if target == NO_PLAYER || self.may_read(target) {
            return Ok(());
        }
        tracing::warn!(
            "Rejected cross-account progression read actor={} target={}",
            log_token(self.who.user.as_deref().unwrap_or("unknown")),
            log_token(target)
        );
        Err(forbidden())
    }

    fn actor(&self, account: &AccountId) -> ActorContext {
        let actor = if self.who.gs || self.who.host {
            Actor::System
        } else {
            Actor::Player {
                account: account.global().clone(),
            }
        };
        ActorContext::new(actor).with_request(self.request_id.as_str())
    }

    async fn account(&self, store: &IslesStore, user: &str) -> Result<Option<AccountId>, Response> {
        let user_id = UserId::new(user);
        store
            .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &user_id))
            .await
            .map(|a| a.map(|a| a.id))
            .map_err(|e| store_error(&e))
    }

    async fn require_account(&self, store: &IslesStore, user: &str) -> Result<AccountId, Response> {
        match self.account(store, user).await? {
            Some(account) => Ok(account),
            None => {
                tracing::warn!("progression: no account for userId {}", printable(user, 128));
                Err(bad())
            }
        }
    }

    /// One host write asked of the guard (§5.1): allowed, or the answer.
    async fn authorize(&self, kind: HostWriteKind, target: &str, options: HostWriteOptions) -> Result<bool, Response> {
        let Some(capability) = self.who.capability.clone() else {
            return Err(coded(
                StatusCode::FORBIDDEN,
                "forbidden",
                "This runtime cannot write player state.",
            ));
        };
        let asked = self.shared.rifts().guard().authorize_host_write(
            capability,
            kind,
            rifts_seam::AccountId(target.to_owned()),
            options,
        );
        match asked.await {
            Ok(None) => Ok(true),
            Ok(Some(reply)) if options.probe => {
                let _ = reply;
                Ok(false)
            }
            Ok(Some(reply)) => Err(crate::util::reply(&reply)),
            Err(e) => {
                tracing::error!(
                    "[HostWrite] authorize {} for {} failed: {e}",
                    caller::kind_name(kind),
                    log_token(target)
                );
                Err(empty(StatusCode::INTERNAL_SERVER_ERROR))
            }
        }
    }

    async fn tag(&self, kind: HostWriteKind, target: &str, summary: &Value) {
        let Some(capability) = self.who.capability.clone() else {
            return;
        };
        let write = HostWrite {
            kind,
            target: rifts_seam::AccountId(target.to_owned()),
            summary: rifts_seam::Json(jsjson::stringify(summary)),
        };
        if let Err(e) = self.shared.rifts().guard().record_host_write(capability, write).await {
            tracing::warn!(
                "[HostWrite] tag failed kind={} target={}: {e}",
                caller::kind_name(kind),
                log_token(target)
            );
        }
    }

    /// Whether the caller may repair or reconcile `target`'s state on a read (§3.2 step 4).
    async fn authoritative(&self, kind: HostWriteKind, target: &str) -> Result<bool, Response> {
        if self.who.gs {
            return Ok(true);
        }
        if !self.who.host || self.who.capability.is_none() {
            return Ok(false);
        }
        let options = HostWriteOptions {
            probe: false,
            parity: true,
        };
        // A refusal here only means "not authoritative"; a guard error is still the empty 500
        match self.authorize(kind, target, options).await {
            Ok(allowed) => Ok(allowed),
            Err(answer) if answer.status() == StatusCode::INTERNAL_SERVER_ERROR => Err(answer),
            Err(_) => Ok(false),
        }
    }

    fn inventory_ctx<'a>(&'a self, store: &'a IslesStore) -> TxContext<'a> {
        TxContext::new(&self.shared, store, &self.ports.inventory)
    }
}

// ------------------------------------------------------------------ rows

fn track_row(user: &str, id: &str, record: Option<&ProgressionTrackRecord>) -> Value {
    crate::inventory::reforge::single_track_payload(user, id, record)
}

fn grant_payload(user: &str, record: &ProgressionTrackRecord, grant: &str, replayed: bool) -> Value {
    let mut row = track_row(user, &record.progression_id, Some(record));
    if let Some(map) = row.as_object_mut() {
        map.insert("grant_id".into(), Value::from(grant));
        map.insert("replayed".into(), Value::Bool(replayed));
    }
    row
}

// ------------------------------------------------------------------ B52 and the stubs

async fn get_huntpass(call: Call, ports: Arc<ProgressionPorts>) -> Response {
    let entry = answer!(enter(call, ports).await);
    let id = tracks::served_pass(&entry.shared);
    tracing::info!("Huntpass (stubbed) -> huntpass_id={}", log_token(&id));
    envelope(StatusCode::OK, "OK", entry.shared.version().hooks.huntpass_payload(&id))
}

async fn get_weapon_tracker(call: Call, ports: Arc<ProgressionPorts>) -> Response {
    let entry = answer!(enter(call, ports).await);
    answer!(entry.read_gate(&entry.param("userId")));
    envelope(
        StatusCode::OK,
        "OK",
        obj(vec![("TrackersInfo", Value::Array(Vec::new()))]),
    )
}

async fn get_tracked_objectives(call: Call, ports: Arc<ProgressionPorts>) -> Response {
    let entry = answer!(enter(call, ports).await);
    let target = entry.param("phxAccountId");
    answer!(entry.read_gate(&target));
    let payload = obj(vec![
        ("current_set", Value::from("quest_slayer_links")),
        ("omitted_quests", Value::Array(Vec::new())),
        ("phx_account_id", Value::from(target)),
        ("tracked_craftables", Value::Array(Vec::new())),
        ("tracked_quests", Value::Array(Vec::new())),
    ]);
    envelope(StatusCode::OK, "OK", payload)
}

async fn post_tracked_objectives(call: Call, ports: Arc<ProgressionPorts>) -> Response {
    let entry = answer!(enter(call, ports).await);
    answer!(entry.read_gate(&entry.param("phxAccountId")));
    envelope(StatusCode::OK, "OK", Value::Null)
}

// ------------------------------------------------------------------ B122, B129, B115

async fn get_tracks(call: Call, ports: Arc<ProgressionPorts>) -> Response {
    let entry = answer!(enter(call, ports).await);
    let target = entry.resolved_user();
    answer!(entry.read_gate(&target));
    let mut known: Vec<String> = DEFAULT_PROGRESSION_IDS.iter().map(|s| (*s).to_owned()).collect();
    known.push(tracks::active_pass(&entry.shared).to_owned());
    if target == NO_PLAYER {
        let rows: Vec<Value> = known.iter().map(|k| track_row(&target, k, None)).collect();
        return envelope(StatusCode::OK, "OK", Value::Array(rows));
    }
    let store = answer!(entry.store());
    let account = answer!(entry.account(&store, &target).await);
    if reforge_mode(&entry.shared) == ReforgeMode::On
        && let Some(account) = &account
        && answer!(entry.authoritative(HostWriteKind::Reforge, &target).await)
    {
        let (id, user, now) = (account.clone(), target.clone(), rustic_isles::replay::now_ms());
        let repaired = store
            .account_write(account, &entry.actor(account), move |tx, repos| {
                finish_stale_reforges(tx, repos, &id, &user, now)
            })
            .await;
        match repaired {
            Err(e) => tracing::error!(
                "[Reforge] stale-reforge recovery failed for {}: {e}",
                log_token(&target)
            ),
            Ok(closed) if entry.who.host && !closed.is_empty() => {
                tracing::info!(
                    "[HostParity] reforge repair at a realm load user={} tracks=[{}]",
                    log_token(&target),
                    closed.join(", ")
                );
                let summary = obj(vec![("staleReforgesFinished", Value::from(closed))]);
                entry.tag(HostWriteKind::Reforge, &target, &summary).await;
            }
            Ok(_) => {}
        }
    }
    let mut stored = match &account {
        Some(account) => {
            let id = account.clone();
            answer!(
                store
                    .read(move |conn, repos| repos.progression_tracks.get_all_for_account(conn, &id))
                    .await
                    .map_err(|e| store_error(&e))
            )
        }
        None => Vec::new(),
    };
    let mut rows = Vec::new();
    for k in &known {
        match stored.iter().position(|r| &r.progression_id == k) {
            Some(at) => {
                let record = stored.remove(at);
                rows.push(track_row(&target, k, Some(&record)));
            }
            None => rows.push(track_row(&target, k, None)),
        }
    }
    for record in &stored {
        rows.push(track_row(&target, &record.progression_id, Some(record)));
    }
    tracing::info!(
        "Progression fetched for userId {} ({} tracks, persisted)",
        log_token(&target),
        rows.len()
    );
    envelope(StatusCode::OK, "OK", Value::Array(rows))
}

async fn get_track(call: Call, ports: Arc<ProgressionPorts>) -> Response {
    // Express's next("route") with nothing after it, before the body and auth
    if matches!(
        call.params.get("userId"),
        Some("objectives" | "tracked_objectives" | "config")
    ) {
        return empty(StatusCode::NOT_FOUND);
    }
    let entry = answer!(enter(call, ports).await);
    let (target, id) = (entry.param("userId"), entry.param("progressionId"));
    answer!(entry.read_gate(&target));
    if target == NO_PLAYER || id.chars().count() > 128 {
        return envelope(StatusCode::OK, "OK", track_row(&target, &id, None));
    }
    let store = answer!(entry.store());
    let record = match answer!(entry.account(&store, &target).await) {
        Some(account) => {
            let t = id.clone();
            answer!(
                store
                    .read(move |conn, repos| repos.progression_tracks.get(conn, &account, &t))
                    .await
                    .map_err(|e| store_error(&e))
            )
        }
        None => None,
    };
    let reforge = match record.as_ref().and_then(|r| r.reforge.as_ref()) {
        Some(r) if r.open => " reforge=open",
        Some(_) => " reforge=closed",
        None => "",
    };
    tracing::info!(
        "Progression track fetched user={} track={} progress={}{reforge}",
        log_token(&target),
        log_token(&id),
        record.as_ref().map_or(0, |r| r.progress)
    );
    envelope(StatusCode::OK, "OK", track_row(&target, &id, record.as_ref()))
}

async fn get_objectives(call: Call, ports: Arc<ProgressionPorts>) -> Response {
    let entry = answer!(enter(call, ports).await);
    let target = entry.resolved_user();
    answer!(entry.read_gate(&target));
    if target == NO_PLAYER {
        return envelope(StatusCode::OK, "OK", Value::Array(Vec::new()));
    }
    let store = answer!(entry.store());
    let records = match answer!(entry.account(&store, &target).await) {
        Some(account) => answer!(
            store
                .read(move |conn, repos| repos.progression_tracks.get_all_objectives_for_account(conn, &account))
                .await
                .map_err(|e| store_error(&e))
        ),
        None => Vec::new(),
    };
    let hooks = &entry.shared.version().hooks;
    // Stored counters are far below 2^53
    #[allow(clippy::cast_precision_loss)]
    let rows: Vec<Value> = records
        .iter()
        .map(|r| {
            hooks.objective_row(
                &target,
                &ObjectiveRow {
                    objective_id: r.objective_id.clone(),
                    value: r.value as f64,
                    completed_count: r.completed_count as f64,
                    created: r.updated_at.clone(),
                    last_modified: r.updated_at.clone(),
                },
            )
        })
        .collect();
    tracing::info!(
        "Objective progression fetched for userId {} ({} objectives, persisted)",
        log_token(&target),
        rows.len()
    );
    envelope(StatusCode::OK, "OK", Value::Array(rows))
}

// ------------------------------------------------------------------ B119: the XP grant

/// A host ceiling refusal (§4.4).
struct Ceiling {
    reason: &'static str,
    limit: i64,
    stored: Option<i64>,
}

impl Ceiling {
    fn observe_only(&self) -> bool {
        matches!(self.reason, "amount_over_ceiling" | "track_full")
    }

    fn describe(&self, track: &str, amount: f64) -> String {
        let mut text = format!(
            "reason={} track={} amount={} limit={}",
            self.reason,
            printable(track, 64),
            jsjson::number(amount),
            self.limit
        );
        if let Some(stored) = self.stored {
            text.push_str(&format!(" stored={stored}"));
        }
        text
    }
}

fn host_ceiling(shared: &Shared, track: &str, amount: f64) -> Option<Ceiling> {
    let (limit, reason) = if track == "ExperienceTrack_PlayerLevel" || track.starts_with("ExperienceTrack_Weapon_") {
        (1000, "amount_over_ceiling")
    } else if track.starts_with("PrestigeTrack_Weapon_") {
        return (amount != 1.0).then_some(Ceiling {
            reason: "amount_not_one",
            limit: 1,
            stored: None,
        });
    } else if track.starts_with("MasteryTrack_") {
        return Some(Ceiling {
            reason: "track_not_writable",
            limit: 0,
            stored: None,
        });
    } else if is_huntpass_xp_track(shared, track) {
        (200, "amount_over_ceiling")
    } else {
        return None;
    };
    // The limits are small integers
    #[allow(clippy::cast_precision_loss)]
    let over = amount > limit as f64;
    over.then_some(Ceiling {
        reason,
        limit,
        stored: None,
    })
}

/// The total cap of a host-written track: the player level's curve cap.
fn total_cap(shared: &Shared, track: &str) -> Option<i64> {
    if track != "ExperienceTrack_PlayerLevel" {
        return None;
    }
    tracks::track_cap_xp(shared, track)
        .map(crate::util::trunc_i64)
        .filter(|c| *c > 0)
}

fn observe_mode(shared: &Shared) -> bool {
    !shared.rifts().guard().host_policy_refuses().unwrap_or(true)
}

/// `(grant id, word for the REQ log)` (§4.3).
fn grant_id(entry: &Entry) -> (Option<String>, &'static str) {
    let header = ["x-rustic-isles-grant-id", "idempotency-key"]
        .iter()
        .find_map(|name| entry.headers.get(*name).and_then(|v| v.to_str().ok()))
        .map(str::to_owned);
    let body = entry.body();
    let body_id: Option<Value> = match non_null(field(body, "grant_id")) {
        Some(v) => Some(v.clone()),
        None => field(body, "grantId").cloned(),
    };
    let chosen: Option<Value> = match (&header, &body_id) {
        (None, None) if entry.who.gs => {
            return (Some(format!("gs-{}", rustic_isles::replay::next_uuid())), "present");
        }
        (None, None) => return (None, "absent"),
        (Some(h), Some(b)) if b.as_str() != Some(h.as_str()) => None,
        (Some(h), _) => Some(Value::from(h.as_str())),
        (None, Some(b)) => Some(b.clone()),
    };
    match chosen.as_ref().and_then(Value::as_str).filter(|g| is_valid_grant_id(g)) {
        Some(g) => (Some(g.to_owned()), "present"),
        None => (None, "invalid"),
    }
}

/// §5.2: the kind a host's grant is written under, or the guard's refusal.
async fn grant_kind(entry: &Entry, track: &str, target: &str) -> Result<HostWriteKind, Response> {
    let probe = HostWriteOptions {
        probe: true,
        parity: false,
    };
    if entry.authorize(HostWriteKind::Xp, target, probe).await? {
        return Ok(HostWriteKind::Xp);
    }
    if is_prestige_track(track) && entry.authorize(HostWriteKind::Reforge, target, probe).await? {
        return Ok(HostWriteKind::Reforge);
    }
    if track == tracks::active_pass(&entry.shared) && entry.authorize(HostWriteKind::HuntPassXp, target, probe).await? {
        return Ok(HostWriteKind::HuntPassXp);
    }
    entry
        .authorize(HostWriteKind::Xp, target, HostWriteOptions::default())
        .await
        .map(|_| HostWriteKind::Xp)
}

/// The account a game server's grant must be forwarded to (§4.15).
async fn foreign_account(entry: &Entry, store: &IslesStore, user: &str) -> Result<Option<GlobalId>, Response> {
    let here = entry.shared.node().node.clone();
    let global = match user.parse::<GlobalId>() {
        Ok(id) if id.kind() == IdKind::Account => Some(id),
        _ => entry.account(store, user).await?.map(|a| a.global().clone()),
    };
    Ok(global.filter(|id| id.owner() != &here))
}

enum Granted {
    Applied(ProgressionTrackRecord),
    Replayed(ProgressionTrackRecord),
}

#[derive(Debug, thiserror::Error)]
enum GrantStop {
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("host limit")]
    HostLimit,
    #[error(transparent)]
    Repo(#[from] RepoError),
}

#[allow(clippy::too_many_lines)]
async fn post_grant(call: Call, ports: Arc<ProgressionPorts>) -> Response {
    let entry = answer!(enter(call, ports).await);
    let (user, track, raw) = (
        entry.param("userId"),
        entry.param("progressionId"),
        entry.param("amount"),
    );
    let (grant, word) = grant_id(&entry);
    let source = if entry.who.host {
        "host"
    } else if entry.who.gs {
        "gs"
    } else {
        "other"
    };
    tracing::info!(
        "[ProgressionGrant] REQ user={} track={} amount={} grantId={word} src={source}",
        printable(&user, 128),
        printable(&track, 128),
        printable(&raw, 128)
    );
    if !entry.who.host && !entry.who.gs {
        tracing::warn!(
            "Rejected XP grant attempt without gameserver auth (userId={}, track={})",
            log_token(&user),
            log_token(&track)
        );
        return forbidden();
    }
    if user.is_empty() || user == NO_PLAYER {
        tracing::warn!("Rejected XP grant: invalid/sentinel userId ({})", log_token(&user));
        return bad();
    }
    if track.is_empty() || track.chars().count() > 128 {
        tracing::warn!("Rejected XP grant: invalid progressionId (len={})", utf16_len(&track));
        return bad();
    }
    if let Some(reason) = xp_track_refusal(&entry.shared, &track) {
        tracing::warn!(
            "Rejected XP grant: progression track is not approved (user={}, track={}): {reason}",
            log_token(&user),
            log_token(&track)
        );
        return bad();
    }
    let Some(grant) = grant else {
        tracing::warn!("Rejected XP grant: missing or invalid authoritative grant id ({word})");
        return bad();
    };
    let amount = js_number_text(&raw);
    if !(amount.is_finite() && amount.fract() == 0.0 && (1.0..=MAX_PROGRESSION_GRANT_AMOUNT).contains(&amount)) {
        tracing::warn!(
            "Rejected XP grant: invalid amount ({}) for user={} track={}",
            log_token(&raw),
            log_token(&user),
            log_token(&track)
        );
        return bad();
    }
    let observe = entry.who.host && observe_mode(&entry.shared);
    let mut notes: Vec<String> = Vec::new();
    let mut kind = HostWriteKind::Xp;
    let refuse_ceiling = |ceiling: &Ceiling, amount: f64, kind: HostWriteKind, notes: &mut Vec<String>| -> bool {
        let description = ceiling.describe(&track, amount);
        if observe && ceiling.observe_only() {
            notes.push(format!("user={} {description} grant={}", user, first_chars(&grant, 96)));
            return false;
        }
        tracing::warn!(
            "[HostWrite] refused kind={} target={} {description} grant={}",
            caller::kind_name(kind),
            printable(&user, 128),
            printable(&grant, 128)
        );
        true
    };
    if entry.who.host {
        kind = answer!(grant_kind(&entry, &track, &user).await);
        if let Some(ceiling) = host_ceiling(&entry.shared, &track, amount)
            && refuse_ceiling(&ceiling, amount, kind, &mut notes)
        {
            return host_limit();
        }
    }
    let store = answer!(entry.store());
    let multipliers = read_test_multipliers(entry.shared.settings());
    let multiplied = multiply_xp_grant(amount, MAX_PROGRESSION_GRANT_AMOUNT, &multipliers);
    if entry.who.gs
        && let Some(account) = answer!(foreign_account(&entry, &store, &user).await)
    {
        return forward_grant(&entry, account, &track, multiplied).await;
    }
    let account = answer!(entry.require_account(&store, &user).await);
    let mut plan = ReforgePlan::Normal;
    if is_reforgeable_track(&track) || is_prestige_track(&track) {
        let weapon = experience_track_for_prestige(&track).unwrap_or_else(|| track.clone());
        let (id, g) = (account.clone(), grant.clone());
        let read = store
            .read(move |conn, repos| {
                let ledger = repos.progression_grants.find_by_grant_id(conn, &g)?;
                let row = repos.progression_tracks.get(conn, &id, &weapon)?;
                Ok((ledger, row))
            })
            .await;
        let (ledger, row) = answer!(read.map_err(|e| store_error(&e)));
        if ledger.is_none() {
            plan = classify_grant(
                reforge_mode(&entry.shared),
                &track,
                amount,
                row.as_ref(),
                crate::util::system_now_ms(),
            );
            if let ReforgePlan::Refuse(reason) = &plan {
                tracing::info!(
                    "[Reforge] DUP/REFUSED grant user={} track={} amount={}: {reason}",
                    log_token(&user),
                    log_token(&track),
                    jsjson::number(amount)
                );
                return conflict();
            }
            if entry.who.host && is_prestige_track(&track) && !matches!(plan, ReforgePlan::Prestige { .. }) {
                return conflict();
            }
        }
    }
    if multiplied != amount {
        tracing::info!(
            "[TestMultiplier] XP grant {} -> {} user={} track={}",
            jsjson::number(amount),
            jsjson::number(multiplied),
            log_token(&user),
            log_token(&track)
        );
    }
    let m = crate::util::whole(multiplied);
    let hash = grant_request_hash(&user, &track, multiplied);
    let host_ceiling_m = if entry.who.host {
        host_ceiling(&entry.shared, &track, multiplied)
    } else {
        None
    };
    if let Some(ceiling) = &host_ceiling_m
        && refuse_ceiling(ceiling, multiplied, kind, &mut notes)
    {
        send_notes(&entry, &notes);
        return host_limit();
    }
    let cap = if entry.who.host {
        total_cap(&entry.shared, &track)
    } else {
        None
    };
    let (id, t, g, u, p) = (
        account.clone(),
        track.clone(),
        grant.clone(),
        user.clone(),
        plan.clone(),
    );
    let source_word = if entry.who.host { "host" } else { "gameserver" };
    let outcome = guarded(
        &store,
        Some(&account),
        &entry.actor(&account),
        move |tx: &mut Tx<'_>, repos: &Repositories| -> Result<(Granted, Option<Ceiling>), GrantStop> {
            if let Some(existing) = repos.progression_grants.find_by_grant_id(tx.conn(), &g)? {
                if existing.request_hash != hash
                    || existing.account != id
                    || existing.progression_id != t
                    || existing.amount != m
                {
                    tracing::warn!(
                        "Rejected XP grant reuse with different request grant={g} user={} track={}",
                        log_token(&u),
                        log_token(&t)
                    );
                    return Err(GrantStop::Conflict(String::new()));
                }
                return Ok((Granted::Replayed(existing.result), None));
            }
            let before = repos
                .progression_tracks
                .get(tx.conn(), &id, &t)?
                .map_or(0, |r| r.progress);
            let mut full = None;
            if let Some(cap) = cap
                && before >= cap
            {
                full = Some(Ceiling {
                    reason: "track_full",
                    limit: cap,
                    stored: Some(before),
                });
                if !observe {
                    return Err(GrantStop::HostLimit);
                }
            }
            if let Err(lost) = mark_reforge_use_as(tx, repos, &id, &u, &p)? {
                return Err(GrantStop::Conflict(lost.0));
            }
            let record = repos.progression_tracks.increment(tx, &id, &t, m)?;
            repos.progression_grants.insert_applied(
                tx,
                &ProgressionGrantRecord {
                    grant_id: g.clone(),
                    account: id.clone(),
                    progression_id: t.clone(),
                    amount: m,
                    request_hash: hash,
                    result: record.clone(),
                    created_at: now_iso(),
                },
            )?;
            tracing::info!(
                "[ProgressionGrant] grant={g} user={} track={t} delta={m} old={before} new={} source={source_word} \
                 applied=1",
                log_token(&u),
                record.progress
            );
            Ok((Granted::Applied(record), full))
        },
    )
    .await;
    let (granted, full) = match outcome {
        Ok(done) => done,
        Err(GrantStop::Conflict(_)) => {
            send_notes(&entry, &notes);
            return conflict();
        }
        Err(GrantStop::HostLimit) => {
            let ceiling = Ceiling {
                reason: "track_full",
                limit: cap.unwrap_or(0),
                stored: None,
            };
            tracing::warn!(
                "[HostWrite] refused kind={} target={} {} grant={}",
                caller::kind_name(kind),
                printable(&user, 128),
                ceiling.describe(&track, multiplied),
                printable(&grant, 128)
            );
            send_notes(&entry, &notes);
            return host_limit();
        }
        Err(GrantStop::Repo(e)) => return store_error(&e),
    };
    if let Some(ceiling) = full {
        notes.push(format!(
            "user={user} {} grant={}",
            ceiling.describe(&track, multiplied),
            first_chars(&grant, 96)
        ));
    }
    send_notes(&entry, &notes);
    let (record, replayed) = match granted {
        Granted::Applied(record) => (record, false),
        Granted::Replayed(record) => (record, true),
    };
    if !replayed {
        entry.ports.activity.xp(&user, &track, multiplied);
        if entry.who.host {
            let summary = match (&plan, kind) {
                (ReforgePlan::Prestige { reforge, .. }, HostWriteKind::Reforge) => obj(vec![
                    ("route", Value::from("xp")),
                    ("track", Value::from(track.as_str())),
                    ("amount", number_value(amount)),
                    ("grantId", Value::from(grant.as_str())),
                    ("reforgeId", Value::from(reforge.reforge_id.as_str())),
                ]),
                _ => obj(vec![
                    ("track", Value::from(track.as_str())),
                    ("amount", number_value(amount)),
                    ("grantId", Value::from(grant.as_str())),
                ]),
            };
            entry.tag(kind, &user, &summary).await;
        }
    }
    if is_huntpass_xp_track(&entry.shared, &track)
        && entry.shared.settings().on_unless_off("RUSTIC_ISLES_ECONOMY_MONITOR")
    {
        let curves = curves(&entry.shared);
        // Track progress is far below 2^53
        #[allow(clippy::cast_precision_loss)]
        let (new, old) = (record.progress as f64, record.progress as f64 - amount);
        let rank = |x: f64| {
            curves
                .rank_for_progress(&track, x)
                .map_or_else(|| String::from("?"), |r| r.to_string())
        };
        tracing::info!(
            "[HuntPassMon] GRANT user={} track={} amount={} progress={}->{} rank={}->{} confirmed={}/{} replayed={} grant={}",
            log_token(&user),
            log_token(&track),
            jsjson::number(amount),
            jsjson::number(old),
            record.progress,
            rank(old),
            rank(new),
            record.confirmed_fremium_rank,
            record.confirmed_premium_rank,
            u8::from(replayed),
            log_token(&grant)
        );
    }
    envelope(StatusCode::OK, "OK", grant_payload(&user, &record, &grant, replayed))
}

fn send_notes(entry: &Entry, notes: &[String]) {
    for detail in notes {
        if let Err(e) = entry.shared.rifts().guard().note_would_refuse("xp", detail) {
            tracing::warn!("[HostWrite] note failed: {e}");
        }
    }
}

async fn forward_grant(entry: &Entry, account: GlobalId, track: &str, multiplied: f64) -> Response {
    let header_hunt = entry
        .headers
        .get("x-reforged-instance-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|t| t.parse::<GlobalId>().ok());
    let hunt = header_hunt
        .or_else(|| entry.ports.ring.ring_instance_for_player(&account))
        .or_else(|| {
            entry
                .shared
                .slot::<crate::inventory::RingInstancesSlot>()?
                .0
                .ring_instance_for_player(&account)
        })
        .unwrap_or_else(|| GlobalId::new(IdKind::Hunt, &entry.shared.node().node));
    let Some(module) = crate::inventory::module_id(&entry.shared) else {
        return empty(StatusCode::INTERNAL_SERVER_ERROR);
    };
    let rewards = HuntRewards {
        hunt,
        module,
        account: account.clone(),
        xp_grant: Some(obj(vec![
            ("track", Value::from(track)),
            ("xp", number_value(multiplied)),
        ])),
        loot_write: None,
        objective_write: None,
    };
    if let Err(e) = entry.shared.home().ring_forward_hunt_rewards(&rewards).await {
        tracing::error!("ring_forward_hunt_rewards (xp) for {account}: {e}");
        return empty(StatusCode::INTERNAL_SERVER_ERROR);
    }
    envelope(
        StatusCode::OK,
        "OK",
        obj(vec![
            ("progression_id", Value::from(track)),
            ("xp", number_value(multiplied)),
            ("level", Value::from(1)),
            ("forwarded_to_home", Value::Bool(true)),
        ]),
    )
}

// ------------------------------------------------------------------ B120: the objective report

fn mastery_persistence(shared: &Shared) -> bool {
    match shared.settings().raw("RUSTIC_ISLES_MASTERY_PERSISTENCE") {
        Some(_) => shared.settings().on_unless_off("RUSTIC_ISLES_MASTERY_PERSISTENCE"),
        None => shared.version().hooks.mastery_persistence(),
    }
}

const MASTERY_TRACKS: [&str; 9] = [
    "MasteryTrack_PlayerLevel",
    "MasteryTrack_Weapon_Axe",
    "MasteryTrack_Weapon_ChainBlades",
    "MasteryTrack_Weapon_Hammer",
    "MasteryTrack_Weapon_Repeaters",
    "MasteryTrack_Weapon_Spear",
    "MasteryTrack_Weapon_Strikers",
    "MasteryTrack_Weapon_Sword",
    "MasteryTrack_Behemoth",
];

fn known_objectives(shared: &Shared) -> Option<HashSet<String>> {
    match shared.game_data().try_load_backend("mastery_objectives.json") {
        Ok(Some(document)) => {
            let ids: HashSet<String> = document
                .get("objectiveIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            (!ids.is_empty()).then_some(ids)
        }
        Ok(None) => None,
        Err(e) => {
            tracing::warn!("mastery_objectives.json: {e}");
            None
        }
    }
}

fn first_twenty(list: &[String]) -> Value {
    Value::from(list.iter().take(20).cloned().collect::<Vec<_>>())
}

fn report_summary(report: &Value) -> Value {
    let tracks = report
        .get("progress_tracks")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let objectives = report
        .get("objectives")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let listed: Vec<Value> = tracks
        .iter()
        .take(20)
        .map(|t| {
            obj(vec![
                (
                    "id",
                    Value::from(crate::util::first_units(&js::string(t.get("progression_id")), 128)),
                ),
                ("progress", number_value(crate::util::num(t.get("progress")))),
            ])
        })
        .collect();
    let completed = objectives
        .iter()
        .filter(|o| crate::util::num(o.get("completed_count")) >= 1.0)
        .count();
    obj(vec![
        ("progressTracks", Value::Array(listed)),
        ("progressTrackCount", Value::from(tracks.len())),
        ("objectiveCount", Value::from(objectives.len())),
        ("completedObjectiveCount", Value::from(completed)),
    ])
}

fn pairs_object(list: &[(String, i64)]) -> Value {
    let mut map = Map::new();
    for (k, v) in list {
        map.insert(k.clone(), Value::from(*v));
    }
    Value::Object(map)
}

fn mastery_summary(plan: &mastery::MasteryPlan) -> Value {
    let mut map = Map::new();
    map.insert("objectives".into(), Value::from(plan.merged.len()));
    map.insert("newlyCompleted".into(), first_twenty(&plan.newly));
    map.insert("newlyCompletedCount".into(), Value::from(plan.newly.len()));
    map.insert("credits".into(), pairs_object(&plan.credits));
    map.insert("claimed".into(), pairs_object(&plan.claimed));
    if !plan.capped.is_empty() {
        map.insert("capped".into(), first_twenty(&plan.capped));
    }
    if !plan.unmapped.is_empty() {
        map.insert("unmapped".into(), first_twenty(&plan.unmapped));
    }
    if !plan.ignored.is_empty() {
        map.insert("ignoredCount".into(), Value::from(plan.ignored.len()));
        let ignored: Vec<String> = plan.ignored.iter().map(|(id, r)| format!("{id}:{r}")).collect();
        map.insert("ignored".into(), first_twenty(&ignored));
    }
    if !plan.over_limit.is_empty() {
        map.insert("overLimitCount".into(), Value::from(plan.over_limit.len()));
        map.insert("overLimit".into(), first_twenty(&plan.over_limit));
    }
    if !plan.clamped.is_empty() {
        map.insert("clamped".into(), first_twenty(&plan.clamped));
    }
    if !plan.at_cap.is_empty() {
        map.insert("atCap".into(), first_twenty(&plan.at_cap));
    }
    Value::Object(map)
}

#[allow(clippy::too_many_lines)]
async fn post_report(call: Call, ports: Arc<ProgressionPorts>) -> Response {
    let entry = answer!(enter(call, ports).await);
    let target = entry.resolved_user();
    if entry.who.host {
        if let Err(answer) = entry
            .authorize(HostWriteKind::Objectives, &target, HostWriteOptions::default())
            .await
        {
            tracing::warn!(
                "Rejected host progression report actor={} target={}",
                log_token(entry.who.user.as_deref().unwrap_or("host")),
                log_token(&target)
            );
            return answer;
        }
    } else if !entry.may_report(&target) {
        return forbidden();
    }
    if target == NO_PLAYER {
        return envelope(StatusCode::OK, "OK", Value::Object(Map::new()));
    }
    let report = entry.body.clone().unwrap_or_else(|| Value::Object(Map::new()));
    if !report_is_valid(&report) {
        return bad();
    }
    let store = answer!(entry.store());
    let account = answer!(entry.require_account(&store, &target).await);
    let raw = jsjson::stringify(&report);
    let (id, text) = (account.clone(), raw.clone());
    let appended = store
        .account_write(&account, &entry.actor(&account), move |tx, repos| {
            repos
                .progression_tracks
                .append_objective_event(tx, &id, &text, &now_iso())
        })
        .await;
    if let Err(e) = appended {
        return store_error(&e);
    }
    if let Some(map) = report.as_object() {
        let unknown: Vec<&String> = map
            .keys()
            .filter(|k| !matches!(k.as_str(), "progress_tracks" | "objectives"))
            .collect();
        if !unknown.is_empty() {
            let keys: Vec<String> = unknown.iter().map(|k| log_token(k)).collect();
            tracing::warn!(
                "[ProgressionObjective] user={} UNRECOGNIZED top-level key(s) [{}] in a progression report - possible \
                 unhandled reward/grant payload, not applied. Raw: {}",
                log_token(&target),
                keys.join(", "),
                jsjson::stringify(&serde_json::Value::String(first_chars(&raw, 2000)))
            );
        }
    }
    let count = |key: &str| report.get(key).and_then(Value::as_array).map_or(0, Vec::len);
    tracing::info!(
        "[ProgressionObjective] user={} mode=diagnostics-only tracksReceived={} objectivesReceived={} \
         authoritativeMutation=0 rawBytes={}",
        log_token(&target),
        count("progress_tracks"),
        count("objectives"),
        raw.len()
    );
    if entry.who.host {
        entry
            .tag(HostWriteKind::Objectives, &target, &report_summary(&report))
            .await;
    }
    let mut persist = false;
    if mastery_persistence(&entry.shared) {
        if entry.who.host {
            match entry
                .authorize(HostWriteKind::Mastery, &target, HostWriteOptions::default())
                .await
            {
                Ok(_) => persist = true,
                Err(_) => tracing::info!(
                    "[MasteryMon] user={} host report left diagnostics-only: write refused",
                    log_token(&target)
                ),
            }
        } else {
            persist = entry.who.gs;
        }
    }
    if !persist {
        return envelope(StatusCode::OK, "OK", Value::Object(Map::new()));
    }
    let host_limits = entry.who.host.then(|| {
        let curves = curves(&entry.shared);
        let caps: HashMap<String, (i64, i64)> = MASTERY_TRACKS
            .iter()
            .filter_map(|t| {
                let cap = crate::util::trunc_i64(curves.get(t)?.cap_xp());
                (cap > 0).then(|| ((*t).to_owned(), (cap, 0)))
            })
            .collect();
        HostLimits::standard(known_objectives(&entry.shared), caps)
    });
    let (id, r, t) = (account.clone(), report.clone(), target.clone());
    let now = now_iso();
    let planned = store
        .account_write(&account, &entry.actor(&account), move |tx, repos| {
            let ids: Vec<String> = r
                .get("objectives")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|o| o.get("objective_id").and_then(Value::as_str).map(str::to_owned))
                .collect();
            let stored: HashMap<String, (i64, i64)> = repos
                .progression_tracks
                .get_objectives(tx.conn(), &id, &ids)?
                .into_iter()
                .map(|o| (o.objective_id, (o.value, o.completed_count)))
                .collect();
            let mut limits = host_limits;
            if let Some(limits) = limits.as_mut() {
                let claimed: Vec<String> = r
                    .get("progress_tracks")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|p| p.get("progression_id").and_then(Value::as_str).map(str::to_owned))
                    .collect();
                for (track, (_, total)) in limits.caps.iter_mut() {
                    if claimed.contains(track) {
                        *total = repos
                            .progression_tracks
                            .get(tx.conn(), &id, track)?
                            .map_or(0, |x| x.progress);
                    }
                }
                limits.caps.retain(|track, _| claimed.contains(track));
            }
            let plan = plan_mastery_report(&r, &stored, limits.as_ref());
            repos.progression_tracks.merge_objectives(tx, &id, &plan.writes)?;
            let mut credited = Vec::new();
            for (track, amount) in &plan.credits {
                credited.push(repos.progression_tracks.increment(tx, &id, track, *amount)?);
            }
            let mut rows: Vec<Value> = credited
                .iter()
                .map(|rec| track_row(&t, &rec.progression_id, Some(rec)))
                .collect();
            for (track, _) in &plan.claimed {
                if plan.credits.iter().any(|(c, _)| c == track) {
                    continue;
                }
                let current = repos.progression_tracks.get(tx.conn(), &id, track)?;
                rows.push(track_row(&t, track, current.as_ref()));
            }
            let totals: Vec<String> = credited
                .iter()
                .map(|rec| format!("{}={}", rec.progression_id, rec.progress))
                .collect();
            Ok((plan, rows, totals))
        })
        .await;
    let (plan, rows, totals) = match planned {
        Ok(done) => done,
        Err(e) => {
            tracing::error!(
                "[MasteryMon] user={} report NOT applied: the database write failed: {e}",
                log_token(&target)
            );
            return envelope(StatusCode::OK, "OK", Value::Object(Map::new()));
        }
    };
    if !plan.newly.is_empty() || !plan.claimed.is_empty() {
        let pairs = |l: &[(String, i64)]| l.iter().map(|(t, n)| format!("{t}+{n}")).collect::<Vec<_>>().join(", ");
        let mut line = format!(
            "[MasteryMon] user={} newly=[{}] credit={{{}}} claimed={{{}}} totals={{{}}}",
            log_token(&target),
            plan.newly.join(", "),
            pairs(&plan.credits),
            pairs(&plan.claimed),
            totals.join(", ")
        );
        if !plan.capped.is_empty() {
            line.push_str(&format!(" capped=[{}]", plan.capped.join(", ")));
        }
        if !plan.unmapped.is_empty() {
            line.push_str(&format!(" UNMAPPED=[{}]", plan.unmapped.join(", ")));
        }
        line.push_str(&format!(" objectives={}", plan.merged.len()));
        tracing::info!("{line}");
    }
    if entry.who.host
        && (!plan.ignored.is_empty()
            || !plan.over_limit.is_empty()
            || !plan.clamped.is_empty()
            || !plan.at_cap.is_empty())
    {
        let ignored: Vec<String> = plan
            .ignored
            .iter()
            .take(10)
            .map(|(id, r)| format!("{id}:{r}"))
            .collect();
        tracing::warn!(
            "[HostWrite] limited kind=MASTERY target={} ignored={}[{}] overLimit={} clamped={} atCap={}",
            log_token(&target),
            plan.ignored.len(),
            ignored.join(", "),
            plan.over_limit.len(),
            plan.clamped.len(),
            plan.at_cap.len()
        );
        entry
            .tag(HostWriteKind::Mastery, &target, &mastery_summary(&plan))
            .await;
    } else if entry.who.host {
        entry
            .tag(HostWriteKind::Mastery, &target, &mastery_summary(&plan))
            .await;
    }
    let objectives: Vec<Value> = plan
        .merged
        .iter()
        .map(|(id, value, count)| {
            obj(vec![
                ("phx_account_id", Value::from(target.as_str())),
                ("objective_id", Value::from(id.as_str())),
                ("progress", Value::from(*value)),
                ("completed_count", Value::from(*count)),
                ("created_date", Value::from(now.as_str())),
                ("last_modified_date", Value::from(now.as_str())),
            ])
        })
        .collect();
    envelope(
        StatusCode::OK,
        "OK",
        obj(vec![
            ("progress_tracks", Value::Array(rows)),
            ("objectives", Value::Array(objectives)),
        ]),
    )
}

// ------------------------------------------------------------------ B121: the rank confirm

enum Confirmed {
    Rejected(&'static str),
    Done(Box<ProgressionTrackRecord>, i64),
}

async fn post_confirm(call: Call, ports: Arc<ProgressionPorts>) -> Response {
    let entry = answer!(enter(call, ports).await);
    let (user, track, raw) = (entry.param("userId"), entry.param("progressionId"), entry.param("rank"));
    if !entry.who.host && !entry.who.gs {
        tracing::warn!(
            "Rejected rank confirm without gameserver auth (userId={})",
            log_token(&user)
        );
        return forbidden();
    }
    if user.is_empty() || user == NO_PLAYER {
        tracing::warn!("Rejected rank confirm: invalid/sentinel userId ({})", log_token(&user));
        return bad();
    }
    let rank = js_number_text(&raw);
    if !(rank.is_finite() && rank.fract() == 0.0 && (0.0..=crate::util::MAX_SAFE).contains(&rank)) {
        tracing::warn!("Rejected rank confirm: invalid rank ({})", log_token(&raw));
        return bad();
    }
    let rank = crate::util::whole(rank);
    if entry.who.host {
        answer!(
            entry
                .authorize(HostWriteKind::RankConfirm, &user, HostWriteOptions::default())
                .await
        );
    }
    let store = answer!(entry.store());
    let account = answer!(entry.require_account(&store, &user).await);
    let curve = curves(&entry.shared).get(&track).cloned();
    let (id, t, u) = (account.clone(), track.clone(), user.clone());
    let outcome = store
        .account_write(&account, &entry.actor(&account), move |tx, repos| {
            let stored = repos.progression_tracks.get(tx.conn(), &id, &t)?;
            let before = stored.as_ref().map_or(0, |r| r.confirmed_fremium_rank);
            let Some(curve) = curve else {
                return Ok(Confirmed::Rejected("unknown_track"));
            };
            let Some(required) = curve.required_xp_for_rank(rank) else {
                return Ok(Confirmed::Rejected("rank_out_of_range"));
            };
            let progress = stored.as_ref().map_or(0, |r| r.progress);
            // Track progress is far below 2^53
            #[allow(clippy::cast_precision_loss)]
            let gap = required - progress as f64;
            if required > 0.0 && gap > 0.0 {
                if t.starts_with("MasteryTrack_") && gap <= 1.0 {
                    let target = crate::util::trunc_i64(required.ceil());
                    repos.progression_tracks.set_progress_if_greater(tx, &id, &t, target)?;
                    tracing::info!(
                        "[MasteryMon] SYNC user={} track={t} progress {progress}->{target}",
                        log_token(&u)
                    );
                } else {
                    return Ok(Confirmed::Rejected("insufficient_xp"));
                }
            }
            let record = repos.progression_tracks.set_confirmed_fremium_rank(tx, &id, &t, rank)?;
            Ok(Confirmed::Done(Box::new(record), before))
        })
        .await;
    let (record, before) = match outcome {
        Ok(Confirmed::Done(record, before)) => (*record, before),
        Ok(Confirmed::Rejected(reason)) => {
            tracing::warn!(
                "Rejected rank confirm user={} track={} rank={rank}: {reason}",
                log_token(&user),
                log_token(&track)
            );
            let status = if reason == "insufficient_xp" {
                StatusCode::CONFLICT
            } else {
                StatusCode::BAD_REQUEST
            };
            return crate::util::refusal(status, &format!("Rejected: {reason}"));
        }
        Err(e) => return store_error(&e),
    };
    if entry.who.host && record.confirmed_fremium_rank > before {
        let summary = obj(vec![
            ("track", Value::from(track.as_str())),
            ("rank", Value::from(rank)),
            ("previousRank", Value::from(before)),
            ("confirmedRank", Value::from(record.confirmed_fremium_rank)),
        ]);
        entry.tag(HostWriteKind::RankConfirm, &user, &summary).await;
    }
    tracing::info!(
        "Confirm public progression rank {rank} for userId {} track {} - progress preserved at {}",
        log_token(&user),
        log_token(&track),
        record.progress
    );
    envelope(StatusCode::OK, "OK", track_row(&user, &track, Some(&record)))
}

// ------------------------------------------------------------------ B43, B44: escalation

fn may_read_escalation(entry: &Entry, target: &str) -> bool {
    if entry.who.gs {
        return true;
    }
    if entry.who.host {
        return entry.who.roster.iter().any(|r| r == target);
    }
    entry.user_is(target)
}

async fn get_escalation(call: Call, ports: Arc<ProgressionPorts>) -> Response {
    let entry = answer!(enter(call, ports).await);
    let (season, target) = (entry.param("escalationSeason"), entry.param("userId"));
    if !may_read_escalation(&entry, &target) {
        tracing::warn!(
            "Rejected cross-account escalation read actor={} target={}",
            log_token(entry.who.user.as_deref().unwrap_or("unknown")),
            log_token(&target)
        );
        return empty(StatusCode::FORBIDDEN);
    }
    let store = answer!(entry.store());
    let record = match answer!(entry.account(&store, &target).await) {
        Some(account) => answer!(
            store
                .read(move |conn, repos| repos.escalation_progress.get(conn, &account, &season))
                .await
                .map_err(|e| store_error(&e))
        ),
        None => None,
    };
    envelope(StatusCode::OK, "OK", escalation_payload(record.as_ref()))
}

/// `escalation_tuning.json`, read on each host write: none when missing, an error text when invalid.
fn load_tuning(shared: &Shared) -> Result<Option<Tuning>, String> {
    match shared.game_data().try_load_backend("escalation_tuning.json") {
        Ok(None) => Ok(None),
        Ok(Some(document)) => escalation::parse_tuning(&document).map(Some),
        Err(e) => Err(format!("unreadable: {e}")),
    }
}

async fn post_escalation(call: Call, ports: Arc<ProgressionPorts>) -> Response {
    let entry = answer!(enter(call, ports).await);
    let (season, target) = (entry.param("escalationSeason"), entry.param("userId"));
    let host_write = !entry.may_report(&target) && entry.who.host && !entry.who.gs;
    if !entry.may_report(&target) && !host_write {
        return empty(StatusCode::FORBIDDEN);
    }
    let mut table = None;
    if host_write {
        answer!(
            entry
                .authorize(HostWriteKind::Escalation, &target, HostWriteOptions::default())
                .await
        );
        match load_tuning(&entry.shared) {
            Ok(Some(tuning)) => match tuning.seasons.get(&season) {
                Some(s) => table = Some(s.clone()),
                None => {
                    tracing::warn!(
                        "[Escalation] host write for an unknown season {}",
                        jsjson::stringify(&serde_json::Value::String(first_chars(&season, 64)))
                    );
                    return coded(
                        StatusCode::FORBIDDEN,
                        "ESCALATION_SEASON_UNKNOWN",
                        "This escalation season is not known.",
                    );
                }
            },
            Ok(None) => {}
            Err(problem) => tracing::warn!(
                "[Escalation] game-data/escalation_tuning.json rejected ({problem}): host escalation writes keep the \
                 never-lower merge"
            ),
        }
    }
    let store = answer!(entry.store());
    let account = answer!(entry.require_account(&store, &target).await);
    let update = entry.body.clone().unwrap_or_else(|| Value::Object(Map::new()));
    let (id, s) = (account.clone(), season.clone());
    let saved = store
        .account_write(&account, &entry.actor(&account), move |tx, repos| {
            for _ in 0..8 {
                let previous = repos.escalation_progress.get(tx.conn(), &id, &s)?;
                let now = now_iso();
                let (record, summary) = if host_write {
                    let merged = merge_host_escalation(previous.as_ref(), &update, &now, &id, &s, table.as_ref());
                    let parts = (merged.level_xp_rejected.clone(), merged.talents_rejected.clone());
                    (merged.record, Some((merged.summary, parts)))
                } else {
                    (merge_escalation(previous.as_ref(), &update, &now, &id, &s), None)
                };
                let expected = previous.as_ref().map(|p| p.update_version);
                if let Some(saved) = repos.escalation_progress.save_if_version(tx, &record, expected)? {
                    return Ok::<(EscalationProgressRecord, _), RepoError>((saved, summary));
                }
            }
            Err(RepoError::Data("escalation progress save remained contended".into()))
        })
        .await;
    let (record, summary) = match saved {
        Ok(saved) => saved,
        Err(e) => return store_error(&e),
    };
    if let Some((summary, (level, talents))) = summary {
        let mut tag = Map::new();
        tag.insert("season".into(), Value::from(season.as_str()));
        tag.extend(summary);
        entry.tag(HostWriteKind::Escalation, &target, &Value::Object(tag)).await;
        if level.is_some() || talents.is_some() {
            tracing::warn!(
                "Host escalation write partly refused user={} season={}: {}",
                log_token(&target),
                log_token(&season),
                [level, talents].into_iter().flatten().collect::<Vec<_>>().join("; ")
            );
        }
    }
    tracing::info!(
        "Escalation progress saved season={} user={} level={} version={}",
        log_token(&season),
        log_token(&target),
        record.escalation_level,
        record.update_version
    );
    envelope(StatusCode::OK, "OK", escalation_payload(Some(&record)))
}

// ------------------------------------------------------------------ encountered content and breadcrumbs

/// §4.13's ownership: blank or sentinel ids are never owned.
async fn owned(entry: &Entry, store: &IslesStore, user: &str, character: &str) -> bool {
    let blank = |s: &str| js::trim(s).is_empty() || s == NO_PLAYER;
    if blank(user) || blank(character) {
        return false;
    }
    let Ok(Some(account)) = entry.account(store, user).await else {
        return false;
    };
    entry.ports.characters.owns(store, &account, character).await
}

fn stub_content(list: Option<&Value>) -> Value {
    let types: Vec<Value> = list
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|x| obj(vec![("content", Value::Array(Vec::new())), ("content_type", x.clone())]))
        .collect();
    obj(vec![
        ("content_types", Value::Array(types)),
        ("success", Value::Bool(true)),
    ])
}

/// The categories of `list` stored for (character, the user's account), ascending.
async fn query_content(
    entry: &Entry,
    store: &IslesStore,
    user: &str,
    character: &str,
    list: &Value,
) -> Result<Value, Response> {
    let Some(list) = list.as_array() else {
        return Err(empty(StatusCode::INTERNAL_SERVER_ERROR));
    };
    let Some(account) = entry.account(store, user).await? else {
        return Ok(Value::Array(Vec::new()));
    };
    let c = character.to_owned();
    let row = store
        .read(move |conn, repos| repos.encountered_content.find(conn, &c, &account))
        .await
        .map_err(|e| store_error(&e))?;
    let Some(row) = row else {
        return Ok(Value::Array(Vec::new()));
    };
    let Ok(Value::Array(stored)) = jsjson::parse(&row.encountered_content) else {
        return Err(empty(StatusCode::INTERNAL_SERVER_ERROR));
    };
    let mut out = Vec::new();
    for n in 0..8u8 {
        let asked = list.iter().any(|x| x.as_f64() == Some(f64::from(n)));
        if !asked {
            continue;
        }
        let content: Vec<Value> = stored
            .iter()
            .filter(|e| e.get("category").and_then(Value::as_f64) == Some(f64::from(n)))
            .filter_map(|e| e.get("content").cloned())
            .collect();
        out.push(obj(vec![
            ("content", Value::Array(content)),
            ("content_type", Value::from(n)),
        ]));
    }
    Ok(Value::Array(out))
}

fn log_query(entry: &Entry, character: &str) {
    tracing::info!(
        "Querying encountered content for userId {} and characterId {}",
        entry
            .who
            .user
            .as_deref()
            .map_or_else(|| String::from("undefined"), log_token),
        log_token(character)
    );
}

async fn character_gate(entry: &Entry, store: &IslesStore, user: &str, character: &str) -> Result<(), Response> {
    if user == NO_PLAYER || owned(entry, store, user, character).await {
        return Ok(());
    }
    Err(coded(
        StatusCode::FORBIDDEN,
        "forbidden",
        "Character is not owned by the authenticated account.",
    ))
}

async fn get_encountered(call: Call, ports: Arc<ProgressionPorts>) -> Response {
    let entry = answer!(enter(call, ports).await);
    let character = entry.param("characterId");
    let k = js_number_text(&entry.param("contentType"));
    let k = if k.is_finite() { number_value(k) } else { Value::Null };
    log_query(&entry, &character);
    let Some(user) = entry.who.user.clone() else {
        return okp(&stub_content(Some(&Value::Array(vec![k]))));
    };
    let store = answer!(entry.store());
    answer!(character_gate(&entry, &store, &user, &character).await);
    let types = answer!(query_content(&entry, &store, &user, &character, &Value::Array(vec![k])).await);
    okp(&obj(vec![("content_types", types), ("success", Value::Bool(true))]))
}

async fn query_encountered(call: Call, ports: Arc<ProgressionPorts>) -> Response {
    let entry = answer!(enter(call, ports).await);
    if entry.body.is_none() {
        return empty(StatusCode::INTERNAL_SERVER_ERROR);
    }
    let character = entry.param("characterId");
    let list = non_null(field(entry.body(), "content_types"))
        .cloned()
        .unwrap_or(Value::Array(Vec::new()));
    log_query(&entry, &character);
    let Some(user) = entry.who.user.clone() else {
        return okp(&stub_content(Some(&list)));
    };
    let store = answer!(entry.store());
    answer!(character_gate(&entry, &store, &user, &character).await);
    let types = answer!(query_content(&entry, &store, &user, &character, &list).await);
    okp(&obj(vec![("content_types", types), ("success", Value::Bool(true))]))
}

async fn query_encountered_without_character(call: Call, ports: Arc<ProgressionPorts>) -> Response {
    let entry = answer!(enter(call, ports).await);
    if entry.body.is_none() {
        return empty(StatusCode::INTERNAL_SERVER_ERROR);
    }
    let list = non_null(field(entry.body(), "content_types"))
        .cloned()
        .unwrap_or(Value::Array(Vec::new()));
    tracing::info!("Querying encountered content without characterId; returning empty content");
    okp(&stub_content(Some(&list)))
}

async fn post_encountered(call: Call, ports: Arc<ProgressionPorts>) -> Response {
    let entry = answer!(enter(call, ports).await);
    if entry.body.is_none() {
        return empty(StatusCode::INTERNAL_SERVER_ERROR);
    }
    let character = entry.param("characterId");
    let (kind, content) = (
        field(entry.body(), "content_type").cloned(),
        field(entry.body(), "content_id").cloned(),
    );
    let (Some(user), Some(kind), Some(content)) = (entry.who.user.clone(), kind, content) else {
        return okp(&Value::Object(Map::new()));
    };
    let store = answer!(entry.store());
    if !owned(&entry, &store, &user, &character).await {
        tracing::warn!("Rejected encountered content write for a character the caller does not own");
        return empty(StatusCode::FORBIDDEN);
    }
    let account = answer!(entry.require_account(&store, &user).await);
    let id = account.clone();
    let saved = store
        .account_write(&account, &entry.actor(&account), move |tx, repos| {
            let row = repos.encountered_content.find(tx.conn(), &character, &id)?;
            let mut list = match &row {
                Some(row) => match jsjson::parse(&row.encountered_content) {
                    Ok(Value::Array(list)) => list,
                    Ok(_) => return Err(RepoError::Data("encountered content: not an array".into())),
                    Err(e) => return Err(RepoError::Data(format!("encountered content: {e}"))),
                },
                None => {
                    repos.encountered_content.create(
                        tx,
                        &EncounteredContentRecord {
                            character_id: character.clone(),
                            account: id.clone(),
                            encountered_content: String::from("[]"),
                        },
                    )?;
                    Vec::new()
                }
            };
            list.push(obj(vec![("content", content), ("category", kind)]));
            repos
                .encountered_content
                .update_content(tx, &character, &id, &jsjson::stringify(&Value::Array(list)))
        })
        .await;
    match saved {
        Ok(()) => okp(&Value::Object(Map::new())),
        Err(e) => store_error(&e),
    }
}

async fn post_encountered_stub(call: Call, ports: Arc<ProgressionPorts>) -> Response {
    let _entry = answer!(enter(call, ports).await);
    okp(&Value::Object(Map::new()))
}

fn empty_breadcrumbs() -> Value {
    obj(vec![
        ("breadcrumbs", Value::Array(Vec::new())),
        ("updateVersion", Value::from(0)),
    ])
}

async fn get_breadcrumbs_stub(call: Call, ports: Arc<ProgressionPorts>) -> Response {
    let _entry = answer!(enter(call, ports).await);
    tracing::info!("Requested breadcrumbs without characterId; returning empty breadcrumbs");
    okp(&empty_breadcrumbs())
}

async fn get_breadcrumbs(call: Call, ports: Arc<ProgressionPorts>) -> Response {
    let entry = answer!(enter(call, ports).await);
    let character = entry.param("characterId");
    let Some(user) = entry.who.user.clone() else {
        return okp(&empty_breadcrumbs());
    };
    let store = answer!(entry.store());
    if !owned(&entry, &store, &user, &character).await {
        return empty(StatusCode::FORBIDDEN);
    }
    let account = answer!(entry.require_account(&store, &user).await);
    let id = account.clone();
    let read = store
        .account_write(&account, &entry.actor(&account), move |tx, repos| {
            match repos.breadcrumbs.find(tx.conn(), &character, &id)? {
                None => {
                    repos.breadcrumbs.create(
                        tx,
                        &BreadcrumbsRecord {
                            character_id: character.clone(),
                            account: id.clone(),
                            breadcrumbs: String::from("[]"),
                            update_version: 0,
                        },
                    )?;
                    Ok(empty_breadcrumbs())
                }
                Some(row) => {
                    let parsed =
                        jsjson::parse(&row.breadcrumbs).map_err(|e| RepoError::Data(format!("breadcrumbs: {e}")))?;
                    Ok(obj(vec![
                        ("breadcrumbs", parsed),
                        ("updateVersion", Value::from(row.update_version)),
                    ]))
                }
            }
        })
        .await;
    match read {
        Ok(answer) => okp(&answer),
        Err(e) => store_error(&e),
    }
}

async fn post_breadcrumbs(call: Call, ports: Arc<ProgressionPorts>) -> Response {
    let entry = answer!(enter(call, ports).await);
    if entry.body.is_none() {
        return empty(StatusCode::INTERNAL_SERVER_ERROR);
    }
    let character = entry.param("characterId");
    let crumbs = field(entry.body(), "breadcrumbs").cloned();
    let version = field(entry.body(), "updateVersion").cloned();
    let user = entry.who.user.clone().unwrap_or_default();
    let store = answer!(entry.store());
    if !owned(&entry, &store, &user, &character).await {
        return empty(StatusCode::FORBIDDEN);
    }
    let account = answer!(entry.require_account(&store, &user).await);
    let text = crumbs.as_ref().map_or_else(|| String::from("[]"), jsjson::stringify);
    let stored_version = version
        .as_ref()
        .and_then(Value::as_f64)
        .map_or(0, crate::util::trunc_i64);
    let id = account.clone();
    let saved = store
        .account_write(&account, &entry.actor(&account), move |tx, repos| {
            if repos.breadcrumbs.find(tx.conn(), &character, &id)?.is_none() {
                repos.breadcrumbs.create(
                    tx,
                    &BreadcrumbsRecord {
                        character_id: character.clone(),
                        account: id.clone(),
                        breadcrumbs: text,
                        update_version: stored_version,
                    },
                )
            } else {
                repos.breadcrumbs.update(tx, &character, &id, &text, stored_version)
            }
        })
        .await;
    if let Err(e) = saved {
        return store_error(&e);
    }
    let mut answer = Map::new();
    if let Some(crumbs) = crumbs {
        answer.insert("breadcrumbs".into(), crumbs);
    }
    if let Some(version) = version {
        answer.insert("updateVersion".into(), version);
    }
    okp(&Value::Object(answer))
}

// ------------------------------------------------------------------ the Slayer's Path

fn masked_nodes(shared: &Shared) -> Vec<String> {
    shared
        .version()
        .tables
        .masked_journey_nodes
        .iter()
        .filter(|m| {
            !shared
                .settings()
                .raw(m.switch)
                .is_some_and(|v| js::trim(v).eq_ignore_ascii_case("on"))
        })
        .map(|m| m.node.to_owned())
        .collect()
}

/// The definitions, read once per module (a missing or bad file is "none" for the module's life).
fn definitions(shared: &Shared) -> Arc<Option<Map<String, Value>>> {
    crate::caches::cached(shared, "progression.journey_definitions", || {
        let root = shared.game_data();
        if root.backend_file("slayers_path_definitions.json").is_none() {
            return (Arc::new(None), false);
        }
        let loaded = match root.try_load_backend("slayers_path_definitions.json") {
            Ok(Some(Value::Object(map))) => Some(map),
            Ok(Some(_)) => {
                tracing::error!(
                    "[slayers-path] FAILED to load slayers_path_definitions.json - node rewards will NOT be granted: not an object"
                );
                None
            }
            Ok(None) => {
                tracing::error!(
                    "[slayers-path] FAILED to load slayers_path_definitions.json - node rewards will NOT be granted: the file is missing"
                );
                None
            }
            Err(e) => {
                tracing::error!(
                    "[slayers-path] FAILED to load slayers_path_definitions.json - node rewards will NOT be granted: {e}"
                );
                None
            }
        };
        (Arc::new(loaded), true)
    })
}

fn served_nodes(shared: &Shared) -> Vec<String> {
    match shared.game_data().try_load_backend("slayers_path.json") {
        Ok(Some(document)) => document
            .get("payload")
            .and_then(|p| p.get("nodes"))
            .and_then(Value::as_object)
            .map(|nodes| nodes.keys().cloned().collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn graph(shared: &Shared) -> Graph {
    let defs = definitions(shared);
    Graph::build((*defs).as_ref(), &served_nodes(shared))
}

async fn saved_journey(store: &IslesStore, account: &AccountId) -> Result<Option<PlayerJourneyRecord>, Response> {
    let id = account.clone();
    store
        .read(move |conn, repos| repos.player_journey.find(conn, &id))
        .await
        .map_err(|e| store_error(&e))
}

fn journey_answer(shared: &Shared, nodes: &Map<String, Value>, version: Value) -> Value {
    obj(vec![
        (
            "nodes",
            Value::Object(journey::mask_nodes(nodes, &masked_nodes(shared))),
        ),
        ("update_version", version),
    ])
}

/// §4.12: grants the rewards of `nodes` the character lacks; never fails the route. The granted catalog ids.
async fn reconcile_rewards(
    entry: &Entry,
    store: &IslesStore,
    account: &AccountId,
    user: &str,
    nodes: &[String],
) -> Vec<String> {
    let graph = graph(&entry.shared);
    if !nodes.iter().any(|n| graph.rewards.contains_key(n)) {
        tracing::info!("[slayers-path] no reward for the unlocked nodes of {}", log_token(user));
        return Vec::new();
    }
    let id = account.clone();
    let user_id = store
        .read(move |conn, repos| repos.accounts.find(conn, &id))
        .await
        .ok()
        .flatten()
        .map_or_else(|| account.to_string(), |a| a.user_id.as_str().to_owned());
    let actor = ActorContext::new(Actor::Player {
        account: account.global().clone(),
    });
    let character = match entry
        .ports
        .inventory
        .characters
        .first_character_id(&entry.shared, store, &actor, &user_id)
        .await
    {
        Ok(Some(c)) => c,
        Ok(None) => {
            tracing::warn!(
                "[slayers-path] cannot grant rewards for {}: no character found",
                log_token(user)
            );
            return Vec::new();
        }
        Err(e) => {
            tracing::warn!("[slayers-path] cannot grant rewards for {}: {e}", log_token(user));
            return Vec::new();
        }
    };
    let ctx = entry.inventory_ctx(store);
    let instanced = match get_inventory(ctx, Some(&user_id), &character, None).await {
        Ok(Some(inventory)) => inventory
            .get("instancedItems")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        Ok(None) => Vec::new(),
        Err(e) => {
            tracing::warn!("[slayers-path] cannot read the inventory of {}: {e}", log_token(user));
            return Vec::new();
        }
    };
    let mut granted = Vec::new();
    for reward in journey::plan_rewards(user, &character, nodes, &graph.rewards, &instanced) {
        let request = TxRequest {
            user_id: user_id.clone(),
            character_id: character.clone(),
            transaction_id: reward.transaction_id.clone(),
            add_instanced: Value::Array(vec![reward.item.clone()]),
            add_stacked: Value::Array(Vec::new()),
            remove_instanced: Value::Array(reward.removes.clone()),
            remove_stacked: Value::Array(Vec::new()),
            save_instanced: Value::Array(Vec::new()),
        };
        let options = TxOptions {
            ownership_already_validated: true,
            diagnostic_label: Some("slayers-path-reward".into()),
            economy: Some(crate::inventory::Economy {
                source: "OTHER".into(),
                p2p_session_id: None,
                store_sku_id: None,
            }),
            ..TxOptions::default()
        };
        match run_inventory_transaction(ctx, request, options, None).await {
            Ok(TxOutcome::Done(_)) => {
                tracing::info!("[slayers-path] granted {} to {}", reward.catalog_id, log_token(user));
                granted.push(reward.catalog_id);
            }
            Ok(TxOutcome::NotOwned) => tracing::warn!(
                "[slayers-path] {} for {} not applied (character not owned); retried on the next save",
                reward.catalog_id,
                log_token(user)
            ),
            Err(e) => tracing::warn!(
                "[slayers-path] {} for {} failed: {e}; retried on the next save",
                reward.catalog_id,
                log_token(user)
            ),
        }
    }
    granted
}

async fn get_journey(call: Call, ports: Arc<ProgressionPorts>) -> Response {
    let entry = answer!(enter(call, ports).await);
    let target = entry.param("userId");
    answer!(entry.read_gate(&target));
    let empty_map = || {
        okp(&obj(vec![
            ("nodes", Value::Object(Map::new())),
            ("update_version", Value::from(1)),
        ]))
    };
    if target == NO_PLAYER {
        return empty_map();
    }
    let store = answer!(entry.store());
    let Some(account) = answer!(entry.account(&store, &target).await) else {
        return empty_map();
    };
    let Some(row) = answer!(saved_journey(&store, &account).await) else {
        return empty_map();
    };
    let nodes = journey::as_map(&jsjson::parse(&row.nodes).unwrap_or(Value::Null));
    if answer!(entry.authoritative(HostWriteKind::SlayersPath, &target).await) {
        let unlocked = journey::unlocked_node_ids(&nodes);
        let granted = reconcile_rewards(&entry, &store, &account, &target, &unlocked).await;
        if !granted.is_empty() {
            tracing::info!(
                "Slayer's Path login reconciliation granted [{}] to {}",
                granted.join(", "),
                log_token(&target)
            );
            if entry.who.host {
                let summary = obj(vec![("loginCatchUp", Value::from(granted))]);
                entry.tag(HostWriteKind::SlayersPath, &target, &summary).await;
            }
        }
    }
    okp(&journey_answer(&entry.shared, &nodes, Value::from(row.update_version)))
}

struct JourneySave {
    nodes: Map<String, Value>,
    version: i64,
    newly: Vec<String>,
    plan: Option<journey::HostJourneyPlan>,
}

#[allow(clippy::too_many_lines)]
async fn post_journey(call: Call, ports: Arc<ProgressionPorts>) -> Response {
    let entry = answer!(enter(call, ports).await);
    let target = entry.param("userId");
    if !entry.who.host && !entry.who.gs {
        return forbidden();
    }
    if entry.who.host {
        answer!(
            entry
                .authorize(HostWriteKind::SlayersPath, &target, HostWriteOptions::default())
                .await
        );
    }
    let sent_version = non_null(field(entry.body(), "update_version")).cloned();
    if target == NO_PLAYER {
        return okp(&obj(vec![
            ("nodes", Value::Object(Map::new())),
            ("update_version", sent_version.unwrap_or(Value::from(1))),
        ]));
    }
    let incoming = journey::as_map(non_null(field(entry.body(), "nodes")).unwrap_or(&Value::Null));
    let v = match &sent_version {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(_) => f64::NAN,
        None => 1.0,
    };
    let v = if v.is_finite() { v } else { 1.0 };
    let store = answer!(entry.store());
    let account = answer!(entry.require_account(&store, &target).await);
    let host_graph = entry.who.host.then(|| graph(&entry.shared));
    let masked = masked_nodes(&entry.shared);
    let (id, user) = (account.clone(), target.clone());
    let saved = store
        .account_write(&account, &entry.actor(&account), move |tx, repos| {
            match repos.player_journey.find(tx.conn(), &id)? {
                None => {
                    let empty = Map::new();
                    let plan = host_graph
                        .as_ref()
                        .map(|g| journey::plan_host_journey(&empty, &incoming, g));
                    let nodes = plan.as_ref().map_or_else(|| incoming.clone(), |p| p.nodes.clone());
                    let version = crate::util::trunc_i64(v);
                    let record = PlayerJourneyRecord {
                        account: id.clone(),
                        nodes: jsjson::stringify(&Value::Object(nodes.clone())),
                        update_version: version,
                        revision: 0,
                    };
                    if !repos.player_journey.create_if_absent(tx, &record)? {
                        return Err(RepoError::Data("player journey row already exists".into()));
                    }
                    let newly = journey::newly_unlocked(&empty, &nodes);
                    Ok(JourneySave {
                        nodes,
                        version,
                        newly,
                        plan,
                    })
                }
                Some(row) => {
                    let stored = journey::as_map(&jsjson::parse(&row.nodes).unwrap_or(Value::Null));
                    // Stored versions are small
                    #[allow(clippy::cast_precision_loss)]
                    let (mut merged, version, stale) =
                        journey::merge_nodes(&stored, row.update_version as f64, &incoming, v);
                    journey::restore_masked(&mut merged, &stored, &masked);
                    let plan = host_graph
                        .as_ref()
                        .map(|g| journey::plan_host_journey(&stored, &merged, g));
                    let nodes = plan.as_ref().map_or(merged, |p| p.nodes.clone());
                    if stale {
                        tracing::warn!(
                            "[pjm-stale] Stale journey save for userId {} (v{} <= stored v{}): stored nodes kept",
                            log_token(&user),
                            jsjson::number(v),
                            row.update_version
                        );
                    }
                    let version = crate::util::trunc_i64(version);
                    let text = jsjson::stringify(&Value::Object(nodes.clone()));
                    if !repos
                        .player_journey
                        .update_if_revision(tx, &id, &text, version, row.revision)?
                    {
                        return Err(RepoError::Data("player journey update conflicted".into()));
                    }
                    let newly = journey::newly_unlocked(&stored, &nodes);
                    Ok(JourneySave {
                        nodes,
                        version,
                        newly,
                        plan,
                    })
                }
            }
        })
        .await;
    let save = match saved {
        Ok(save) => save,
        Err(e) => return store_error(&e),
    };
    tracing::info!(
        "Player journey map SAVED for userId {} ({} nodes, v{})",
        log_token(&target),
        save.nodes.len(),
        save.version
    );
    let source = if entry.who.host { "host" } else { "gameserver" };
    entry.ports.activity.slayers_path_unlocks(&target, &save.newly, source);
    if let Some(plan) = &save.plan
        && (!plan.unlocked.is_empty() || !plan.stripped.is_empty() || !plan.relocked.is_empty())
    {
        let stripped: Vec<String> = plan.stripped.iter().map(|(id, r)| format!("{id}:{r}")).collect();
        let mut summary = Map::new();
        summary.insert("unlocked".into(), Value::from(plan.unlocked.clone()));
        summary.insert("stripped".into(), Value::from(stripped.clone()));
        if !plan.relocked.is_empty() {
            summary.insert("relocked".into(), Value::from(plan.relocked.clone()));
        }
        if !stripped.is_empty() || !plan.relocked.is_empty() {
            tracing::warn!(
                "[slayers-path] host save for {} stripped [{}] relocked [{}]",
                log_token(&target),
                stripped.join(", "),
                plan.relocked.join(", ")
            );
        }
        entry
            .tag(HostWriteKind::SlayersPath, &target, &Value::Object(summary))
            .await;
    }
    let unlocked = journey::unlocked_node_ids(&save.nodes);
    reconcile_rewards(&entry, &store, &account, &target, &unlocked).await;
    okp(&journey_answer(&entry.shared, &save.nodes, Value::from(save.version)))
}

async fn get_journey_file(call: Call, ports: Arc<ProgressionPorts>) -> Response {
    let entry = answer!(enter(call, ports).await);
    if let Some(user) = entry.who.user.clone().filter(|u| u != NO_PLAYER)
        && let Some(store) = entry.shared.slot::<IslesStore>()
        && let Some(account) = answer!(entry.account(&store, &user).await)
        && let Some(row) = answer!(saved_journey(&store, &account).await)
    {
        let nodes = journey::as_map(&jsjson::parse(&row.nodes).unwrap_or(Value::Null));
        return okp(&journey_answer(&entry.shared, &nodes, Value::from(row.update_version)));
    }
    match entry.shared.game_data().try_load_backend("slayers_path.json") {
        Ok(Some(file)) => ok(&file),
        Ok(None) => {
            tracing::error!("slayers_path.json: the file is missing (or there is no game data folder)");
            empty(StatusCode::INTERNAL_SERVER_ERROR)
        }
        Err(e) => {
            tracing::error!("slayers_path.json: {e}");
            empty(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

// ------------------------------------------------------------------ the session supplies seam

/// `GameLogin::session_supplies` (§4.14): never fails the login.
pub struct SessionSupplies {
    shared: Arc<Shared>,
    ports: ProgressionPorts,
}

impl SessionSupplies {
    pub fn new(shared: Arc<Shared>, ports: ProgressionPorts) -> Self {
        Self { shared, ports }
    }

    async fn refill(&self, account_text: &str, user: &str) {
        let Some(path) = self
            .shared
            .settings()
            .get("REFORGED_SESSION_SUPPLIES_FILE")
            .map(str::to_owned)
        else {
            return;
        };
        let Ok(account) = account_text.parse::<AccountId>() else {
            return;
        };
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                tracing::error!(
                    "[SessionSupplies] could not refill supplies for {}: {e}",
                    log_token(user)
                );
                return;
            }
        };
        let text = String::from_utf8_lossy(&bytes);
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        let policy = crate::inventory::storage_policy::StoragePolicy::load(&self.shared).ok();
        let instanced = |id: &str| policy.as_ref().is_some_and(|p| p.is_instanced(id));
        let file = match parse_session_supplies(text, &instanced) {
            Ok(file) => file,
            Err(problem) => {
                tracing::error!(
                    "[SessionSupplies] could not refill supplies for {}: {problem}",
                    log_token(user)
                );
                return;
            }
        };
        let Some((_, targets)) = file.into_iter().find(|(u, _)| u == user).filter(|(_, t)| !t.is_empty()) else {
            return;
        };
        let Some(store) = self.shared.slot::<IslesStore>() else {
            return;
        };
        let id = account.clone();
        let Ok(characters) = store
            .read(move |conn, repos| repos.characters.find_many_by_account(conn, &id))
            .await
        else {
            return;
        };
        for character in characters {
            let c = character.character_id;
            self.ports
                .characters
                .ensure_starter(&self.shared, &store, &self.ports.inventory, &account, user, &c)
                .await;
            for _ in 0..5 {
                let (cid, t) = (c.clone(), targets.clone());
                let attempt = store
                    .write(&ActorContext::new(Actor::System), move |tx, repos| {
                        let Some(inventory) = repos.inventories.find_by_character_id(tx.conn(), &cid)? else {
                            return Ok(Some(false));
                        };
                        let stacked = match jsjson::parse(&inventory.stacked_items) {
                            Ok(Value::Array(list)) => list,
                            _ => return Err(RepoError::Data("inventory stackedItems are not an array".into())),
                        };
                        let Some(next) = apply_session_supplies(&stacked, &t) else {
                            return Ok(Some(false));
                        };
                        let written = repos.inventories.update_both_if_revision_matches(
                            tx,
                            &cid,
                            &inventory.instanced_items,
                            &jsjson::stringify(&Value::Array(next)),
                            inventory.revision.unwrap_or(0),
                        )?;
                        Ok(written.map(|_| true))
                    })
                    .await;
                match attempt {
                    Ok(Some(true)) => {
                        tracing::info!(
                            "[SessionSupplies] refilled {} item(s) for {} (character {c})",
                            targets.len(),
                            log_token(user)
                        );
                        break;
                    }
                    Ok(Some(false)) => break,
                    Ok(None) => continue,
                    Err(e) => {
                        tracing::error!(
                            "[SessionSupplies] could not refill supplies for {}: {e}",
                            log_token(user)
                        );
                        break;
                    }
                }
            }
        }
    }
}

impl GameLogin for SessionSupplies {
    fn session_supplies<'a>(&'a self, account: &'a GameAccount) -> BoxFuture<'a, ()> {
        Box::pin(async move { self.refill(&account.id, &account.user_id).await })
    }
}
