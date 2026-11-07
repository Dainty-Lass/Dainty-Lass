// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The 17 bounties-store handlers (bounties spec §3).

use std::sync::Arc;

use axum::response::Response;
use dainty_module_api::network::HuntRewards;
use dainty_module_api::{Actor, ActorContext, GlobalId, IdKind};
use dainty_service_kit::jsjson;
use http::StatusCode;
use rustic_isles::{Call, Handlers, Method, Params, RouteDef, Service, Shared, not_implemented, read_body};
use rustic_isles_auth::StorePurchase;
use rustic_isles_store::repo::bounties::{BountyStateRecord, CooldownStateRecord};
use rustic_isles_store::{AccountId, IslesStore, RepoError, Repositories, Tx, UserId};
use rustic_rifts_seam::{HostWrite, HostWriteKind, HostWriteOptions, RuntimeAuth};
use serde_json::{Map, Value};

use crate::bounties::cooldowns::{
    self, cooldown_payload, is_valid_cooldown_id, merge_cooldowns, plan_host_cooldowns, validate_cooldown_save,
};
use crate::bounties::entitlements::{grant_entitlements_in_transaction, list_entitlements};
use crate::bounties::state::{
    self, bounty_state_payload, delete_bounties, merge_bounty_state, merge_host_bounty_state, names_nothing,
    parse_delete, validate_bounty_save,
};
use crate::bounties::store::{
    Catalog, PurchaseReason, Sku, is_daily_section, listing, plan_purchase, single_query_value,
};
use crate::bounties::{BountiesSeams, SkuGrants, find_claim, wallet};
use crate::caller;
use crate::inventory::{
    Economy, GrantHook, InventoryPorts, TxContext, TxError, TxOptions, TxOutcome, TxRequest, run_inventory_transaction,
};
use crate::tx::guarded;
use crate::util::{empty, envelope, json, log_token, now_iso, number_value, obj, ok, page500};

const INVALID: &str = "INVALID";

const ROUTES: [(Method, &str); 17] = [
    (Method::Get, "/entitlementsv2"),
    (Method::Post, "/entitlementv2/:userId"),
    (Method::Get, "/cooldown/:userId"),
    (Method::Put, "/cooldown/batch/:userId"),
    (Method::Put, "/cooldown/:userId/:cooldownId"),
    (Method::Post, "/cooldown/:userId/:cooldownId"),
    (Method::Get, "/bounty/game-data"),
    (Method::Get, "/bounty/:userId"),
    (Method::Post, "/bounty/:userId"),
    (Method::Post, "/bounty/delete/:userId"),
    (Method::Delete, "/bounty/delete/:userId"),
    (Method::Post, "/reconcile"),
    (Method::Get, "/creator"),
    (Method::Get, "/balance"),
    (Method::Get, "/product/skus/public"),
    (Method::Get, "/token/:currency/:catalogId"),
    (Method::Post, "/notification/:currency"),
];

/// The bounties part's 17 routes, method and Express path.
pub fn routes() -> &'static [(Method, &'static str)] {
    &ROUTES
}

/// Mounts the 17 handlers.
pub fn register(handlers: &mut Handlers) {
    let b = Service::Backend;
    macro_rules! on {
        ($i:expr, $handler:ident) => {{
            let (method, path) = ROUTES[$i];
            handlers.on(b, method, path, $handler);
        }};
    }
    on!(0, get_entitlements);
    on!(1, post_entitlement);
    on!(2, get_cooldowns);
    on!(3, put_cooldown_batch);
    on!(4, start_cooldown);
    on!(5, start_cooldown);
    on!(6, get_bounty_game_data);
    on!(7, get_bounties);
    on!(8, post_bounties);
    on!(9, delete_bounty);
    on!(10, delete_bounty);
    on!(11, post_reconcile);
    on!(12, get_creator);
    on!(13, get_balance);
    on!(14, get_skus);
    on!(15, get_token);
    on!(16, post_notification);
}

macro_rules! answer {
    ($e:expr) => {
        match $e {
            Ok(value) => value,
            Err(response) => return response,
        }
    };
}

fn bad() -> Response {
    crate::util::refusal(StatusCode::BAD_REQUEST, "Bad Request")
}

fn store_error(route: &RouteDef, error: &dyn std::fmt::Display) -> Response {
    tracing::error!("{} {}: {error}", route.method.as_str(), route.path);
    page500()
}

fn store_body(status: u16, message: &str) -> Response {
    json(
        crate::util::status(status),
        &obj(vec![
            ("code", Value::from(status.to_string())),
            ("message", Value::from(message)),
        ]),
    )
}

struct Entry {
    shared: Arc<Shared>,
    route: &'static RouteDef,
    params: Params,
    request_id: String,
    body: Option<Value>,
    query: String,
    user: Option<String>,
    gs: bool,
    host: bool,
    roster: Vec<String>,
    capability: Option<RuntimeAuth>,
}

async fn enter(call: Call) -> Result<Entry, Response> {
    let Call {
        shared,
        route,
        params,
        mut request,
        request_id,
        ..
    } = call;
    let body = read_body(&mut request).await?;
    let data = caller::game_auth(&shared, &mut request).await?;
    let runtime = data.runtime.as_ref().filter(|_| !data.is_gameserver);
    Ok(Entry {
        query: request.uri().query().unwrap_or("").to_owned(),
        shared,
        route,
        params,
        request_id,
        body,
        user: data.user_id.clone(),
        gs: data.is_gameserver,
        host: runtime.is_some(),
        roster: runtime.map(|r| r.roster().to_vec()).unwrap_or_default(),
        capability: runtime.and_then(caller::capability),
    })
}

impl Entry {
    fn param(&self, name: &str) -> String {
        self.params.get(name).unwrap_or("").to_owned()
    }

    fn source(&self) -> &'static str {
        if self.gs {
            "gs"
        } else if self.host {
            "host"
        } else {
            "player"
        }
    }

    fn store(&self) -> Option<Arc<IslesStore>> {
        self.shared.slot::<IslesStore>()
    }

    fn no_database(&self) -> Response {
        not_implemented(self.route, &self.request_id)
    }

    fn actor(&self, account: &AccountId) -> ActorContext {
        let actor = if self.gs || self.host {
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
            .map_err(|e| store_error(self.route, &e))
    }

    /// The reader check of §4.1.
    fn may_read(&self, target: &str, account: Option<&AccountId>) -> bool {
        if self.gs {
            return true;
        }
        if self.host {
            let global = account.map(ToString::to_string);
            return !self.roster.is_empty()
                && self
                    .roster
                    .iter()
                    .any(|r| r == target || global.as_deref() == Some(r.as_str()));
        }
        self.user.as_deref() == Some(target)
    }

    fn may_write(&self) -> bool {
        self.gs || self.host
    }

    /// The host-write check (§6.4): allowed, or the answer.
    async fn host_write(&self, kind: HostWriteKind, account: &AccountId) -> Result<(), Response> {
        let Some(capability) = self.capability.clone() else {
            return Err(empty(StatusCode::FORBIDDEN));
        };
        let asked = self.shared.rifts().guard().authorize_host_write(
            capability,
            kind,
            rustic_rifts_seam::AccountId(account.to_string()),
            HostWriteOptions::default(),
        );
        match asked.await {
            Ok(None) => Ok(()),
            Ok(Some(reply)) => Err(crate::util::reply(&reply)),
            Err(e) => {
                tracing::error!("hostWrites.authorize: {e}");
                Err(page500())
            }
        }
    }

    async fn record(&self, kind: HostWriteKind, account: &AccountId, summary: &Value) {
        let Some(capability) = self.capability.clone() else {
            return;
        };
        let write = HostWrite {
            kind,
            target: rustic_rifts_seam::AccountId(account.to_string()),
            summary: rustic_rifts_seam::Json(jsjson::stringify(summary)),
        };
        if let Err(e) = self.shared.rifts().guard().record_host_write(capability, write).await {
            tracing::warn!("hostWrites.record: {e}");
        }
    }
}

/// `0`, `false`, `off` or `no` (any case, not trimmed) turns a persistence switch off; unset is the version's default.
fn persistence(shared: &Shared, setting: &str, switch: &str) -> bool {
    let value = shared
        .settings()
        .raw(setting)
        .unwrap_or_else(|| shared.version().tables.switch_default(switch));
    !["0", "false", "off", "no"]
        .iter()
        .any(|v| value.eq_ignore_ascii_case(v))
}

fn bounty_persistence(shared: &Shared) -> bool {
    persistence(shared, "RUSTIC_ISLES_BOUNTY_PERSISTENCE", "bounty_persistence")
}

fn cooldown_persistence(shared: &Shared) -> bool {
    persistence(shared, "RUSTIC_ISLES_COOLDOWN_PERSISTENCE", "cooldown_persistence")
}

fn ok_envelope(payload: Value) -> Response {
    envelope(StatusCode::OK, "OK", payload)
}

fn empty_object() -> Value {
    Value::Object(Map::new())
}

// ------------------------------------------------------------------ entitlements and the stubs

async fn get_entitlements(call: Call) -> Response {
    let entry = answer!(enter(call).await);
    let user = entry.user.clone().unwrap_or_else(|| INVALID.to_owned());
    let mut list = Vec::new();
    if !user.is_empty()
        && user != INVALID
        && let Some(store) = entry.store()
        && let Some(account) = answer!(entry.account(&store, &user).await)
    {
        list = answer!(
            store
                .read(move |conn, repos| list_entitlements(conn, repos, &account))
                .await
                .map_err(|e| store_error(entry.route, &e))
        );
    }
    ok(&entry.shared.version().hooks.entitlements_payload(&list))
}

async fn post_entitlement(call: Call) -> Response {
    let _entry = answer!(enter(call).await);
    ok_envelope(Value::Array(Vec::new()))
}

async fn get_bounty_game_data(call: Call) -> Response {
    let _entry = answer!(enter(call).await);
    let payload = serde_json::json!({
        "max_slots": 4, "num_draft_options": 3, "num_spicy_options": 1, "bounty_token_id": "TOKEN_BOUNTY_DRAFT",
        "premium_bounty_token_id": "TOKEN_BOUNTY_DRAFT_PREMIUM", "num_tokens_hp_start": 4, "num_tokens_per_day": 0,
        "bounty_token_grant_hour": 0, "history_length": 10, "bronze_count": 9, "silver_count": 3, "gold_count": 1,
        "new_season_reset_bounties": false, "bounty_data": [], "item_grant_data": [], "token_rollover_warning_days": 1000,
        "automatic_draft": false, "automatic_claim": false, "delete_claimed_bounties": false,
    });
    ok_envelope(payload)
}

async fn get_creator(call: Call) -> Response {
    let _entry = answer!(enter(call).await);
    ok(&obj(vec![
        ("expirationDate", Value::from("2099-01-01T01:00:00.041Z")),
        ("slug", Value::from("MROWMROW")),
        ("success", Value::Bool(true)),
    ]))
}

// ------------------------------------------------------------------ cooldowns

fn cooldown_list(record: Option<&CooldownStateRecord>) -> String {
    record
        .map(|r| {
            r.entries
                .iter()
                .map(|e| format!("{}@{}", log_token(&e.id), log_token(&e.started_date)))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

async fn get_cooldowns(call: Call) -> Response {
    let entry = answer!(enter(call).await);
    let target = entry.param("userId");
    let on = cooldown_persistence(&entry.shared);
    let store = entry.store();
    let mut account = None;
    if (on && target != INVALID || entry.host)
        && let Some(store) = &store
    {
        account = answer!(entry.account(store, &target).await);
    }
    if !entry.may_read(&target, account.as_ref()) {
        tracing::warn!(
            "Rejected cross-account cooldown read actor={} target={}",
            log_token(entry.user.as_deref().unwrap_or("unknown")),
            log_token(&target)
        );
        return empty(StatusCode::FORBIDDEN);
    }
    if !on || target == INVALID {
        tracing::info!(
            "[CooldownMon] LOAD user={} src={} served=[] (persistence off or sentinel)",
            log_token(&target),
            entry.source()
        );
        return ok_envelope(empty_object());
    }
    let Some(store) = store else { return entry.no_database() };
    let Some(account) = account else {
        return ok_envelope(empty_object());
    };
    let record = answer!(
        store
            .read(move |conn, repos| repos.cooldown_states.get(conn, &account))
            .await
            .map_err(|e| store_error(entry.route, &e))
    );
    tracing::info!(
        "[CooldownMon] LOAD user={} src={} served=[{}] (stored)",
        log_token(&target),
        entry.source(),
        cooldown_list(record.as_ref())
    );
    ok_envelope(cooldown_payload(record.as_ref()))
}

/// The batch save under the dedicated or host rules, compare-and-retry up to 8 times in one account write.
async fn save_cooldowns(
    entry: &Entry,
    store: &IslesStore,
    account: &AccountId,
    incoming: Vec<(String, String)>,
) -> Result<(CooldownStateRecord, Option<cooldowns::HostCooldownChanges>), RepoError> {
    let (id, host) = (account.clone(), entry.host);
    guarded(
        store,
        Some(account),
        &entry.actor(account),
        move |tx: &mut Tx<'_>, repos: &Repositories| {
            for _ in 0..8 {
                let stored = repos.cooldown_states.get(tx.conn(), &id)?;
                let now = rustic_isles::replay::now_ms();
                let (accepted, changes) = if host {
                    let (accepted, changes) = plan_host_cooldowns(stored.as_ref(), &incoming, now);
                    (accepted, Some(changes))
                } else {
                    (incoming.clone(), None)
                };
                let record = merge_cooldowns(stored.as_ref(), &id, &accepted, now);
                if let Some(saved) =
                    repos
                        .cooldown_states
                        .save_if_version(tx, &record, stored.as_ref().map(|r| r.update_version))?
                {
                    return Ok((saved, changes));
                }
            }
            Err(RepoError::Data(format!("Cooldown save remained contended for {id}")))
        },
    )
    .await
}

fn host_suffix(changes: Option<&cooldowns::HostCooldownChanges>) -> String {
    changes.map_or_else(String::new, |c| {
        format!(
            " host: added={} raised={} unchanged={} keptLater={} droppedNew={}",
            c.added_count, c.raised_count, c.unchanged, c.kept_later_count, c.dropped_new
        )
    })
}

async fn put_cooldown_batch(call: Call) -> Response {
    let entry = answer!(enter(call).await);
    let target = entry.param("userId");
    if !entry.may_write() {
        tracing::warn!(
            "Rejected cooldown write without gameserver authority actor={} target={}",
            log_token(entry.user.as_deref().unwrap_or("unknown")),
            log_token(&target)
        );
        return empty(StatusCode::FORBIDDEN);
    }
    if !cooldown_persistence(&entry.shared) || target == INVALID {
        return ok_envelope(empty_object());
    }
    let Some(store) = entry.store() else {
        return entry.no_database();
    };
    let Some(account) = answer!(entry.account(&store, &target).await) else {
        return ok_envelope(empty_object());
    };
    if entry.host {
        answer!(entry.host_write(HostWriteKind::Cooldown, &account).await);
    }
    let incoming = match validate_cooldown_save(entry.body.as_ref()) {
        Ok(incoming) => incoming,
        Err(e) => {
            tracing::warn!(
                "[CooldownMon] SAVE user={} src={} => REJECTED {e}",
                log_token(&target),
                entry.source()
            );
            return bad();
        }
    };
    match save_cooldowns(&entry, &store, &account, incoming).await {
        Ok((record, changes)) => {
            tracing::info!(
                "[CooldownMon] SAVE user={} src={} cooldowns=[{}] => stored -> {} cooldown(s) version={}{}",
                log_token(&target),
                entry.source(),
                cooldown_list(Some(&record)),
                record.entries.len(),
                record.update_version,
                host_suffix(changes.as_ref())
            );
            if let Some(changes) = &changes {
                entry
                    .record(
                        HostWriteKind::Cooldown,
                        &account,
                        &changes.tag(None, Some(record.update_version)),
                    )
                    .await;
            }
            ok_envelope(cooldown_payload(Some(&record)))
        }
        Err(e) => store_error(entry.route, &e),
    }
}

async fn start_cooldown(call: Call) -> Response {
    // `POST /cooldown/batch/<x>` is the batch route's 404, before the body and auth (spec §14 item 12)
    if call.params.get("userId") == Some("batch") {
        return empty(StatusCode::NOT_FOUND);
    }
    let entry = answer!(enter(call).await);
    let (target, id) = (entry.param("userId"), entry.param("cooldownId"));
    let shown = (log_token(&target), log_token(&id));
    let refuse = |status: StatusCode, message: &str, why: &str| {
        tracing::warn!(
            "[CooldownMon] START user={} src={} id={} => REFUSED {why}",
            shown.0,
            entry.source(),
            shown.1
        );
        crate::util::refusal(status, message)
    };
    if !entry.may_write() {
        tracing::warn!(
            "Rejected cooldown write without gameserver authority actor=.. target={}",
            shown.0
        );
        return empty(StatusCode::FORBIDDEN);
    }
    if !is_valid_cooldown_id(&id) {
        return refuse(StatusCode::BAD_REQUEST, "Bad Request", "invalid cooldown id");
    }
    if target == INVALID {
        return refuse(StatusCode::BAD_REQUEST, "Bad Request", "no-player sentinel");
    }
    if !cooldown_persistence(&entry.shared) {
        return refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            "Service Unavailable",
            "persistence off",
        );
    }
    let Some(store) = entry.store() else {
        return entry.no_database();
    };
    let Some(account) = answer!(entry.account(&store, &target).await) else {
        return refuse(StatusCode::NOT_FOUND, "Not Found", "unknown account");
    };
    if entry.host {
        answer!(entry.host_write(HostWriteKind::Cooldown, &account).await);
    }
    let started = now_iso();
    match save_cooldowns(&entry, &store, &account, vec![(id.clone(), started.clone())]).await {
        Ok((record, changes)) => {
            let note = match &changes {
                Some(c) if c.kept_later_count > 0 => {
                    " (kept the stored later date: a host cannot move a cooldown earlier)"
                }
                Some(c) if c.refused_future_count > 0 => " (refused: a host cannot start a cooldown that far ahead)",
                _ => "",
            };
            tracing::info!(
                "[CooldownMon] START user={} src={} id={} => stored @{started} -> {} cooldown(s) version={}{note}",
                shown.0,
                entry.source(),
                shown.1,
                record.entries.len(),
                record.update_version
            );
            if let Some(changes) = &changes {
                entry
                    .record(
                        HostWriteKind::Cooldown,
                        &account,
                        &changes.tag(Some((&id, &started)), None),
                    )
                    .await;
            }
            ok_envelope(cooldown_payload(Some(&record)))
        }
        Err(e) => {
            tracing::error!("[CooldownMon] START user={} id={} => ERROR {e}", shown.0, shown.1);
            store_error(entry.route, &e)
        }
    }
}

// ------------------------------------------------------------------ bounties

async fn get_bounties(call: Call) -> Response {
    let entry = answer!(enter(call).await);
    let target = entry.param("userId");
    let on = bounty_persistence(&entry.shared);
    let store = entry.store();
    let mut account = None;
    if (on && target != INVALID || entry.host)
        && let Some(store) = &store
    {
        account = answer!(entry.account(store, &target).await);
    }
    if !entry.may_read(&target, account.as_ref()) {
        tracing::warn!(
            "Rejected cross-account bounty read actor={} target={}",
            log_token(entry.user.as_deref().unwrap_or("unknown")),
            log_token(&target)
        );
        return empty(StatusCode::FORBIDDEN);
    }
    if !on || target == INVALID {
        let what = if on {
            "no-player sentinel"
        } else {
            "persistence off (stub)"
        };
        tracing::info!(
            "[DraftMon] BOUNTY-LOAD user={} src={} served={what}",
            log_token(&target),
            entry.source()
        );
        return ok_envelope(bounty_state_payload(None));
    }
    let Some(store) = store else { return entry.no_database() };
    let Some(account) = account else {
        return ok_envelope(bounty_state_payload(None));
    };
    let record = answer!(
        store
            .read(move |conn, repos| repos.bounty_states.get(conn, &account))
            .await
            .map_err(|e| store_error(entry.route, &e))
    );
    tracing::info!(
        "[DraftMon] BOUNTY-LOAD user={} src={} served=stored {} version={}",
        log_token(&target),
        entry.source(),
        state::counts(record.as_ref()),
        record.as_ref().map_or(0, |r| r.update_version)
    );
    ok_envelope(bounty_state_payload(record.as_ref()))
}

fn save_line(entry: &Entry, target: &str, outcome: &str) {
    let keys: Vec<String> = entry
        .body
        .as_ref()
        .and_then(Value::as_object)
        .map(|m| m.keys().map(|k| log_token(k)).collect())
        .unwrap_or_default();
    let bounties = entry
        .body
        .as_ref()
        .and_then(|b| b.get("bounties"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    tracing::info!(
        "[DraftMon] BOUNTY-SAVE user={} src={} keys=[{}] bounties={bounties} => {outcome}",
        log_token(target),
        entry.source(),
        keys.join(", ")
    );
}

async fn forward_bounty(entry: &Entry, account: GlobalId) -> Response {
    let body = entry.body.clone();
    let progress = match &body {
        Some(Value::Object(_)) => body.clone().unwrap_or(Value::Null),
        other => obj(vec![("bounty", other.clone().unwrap_or(Value::Null))]),
    };
    let hunt = crate::inventory::ring_instance(&entry.shared, &InventoryPorts::default(), &account)
        .unwrap_or_else(|| GlobalId::new(IdKind::Hunt, &entry.shared.node().node));
    let Some(module) = crate::inventory::module_id(&entry.shared) else {
        return page500();
    };
    let rewards = HuntRewards {
        hunt,
        module,
        account: account.clone(),
        xp_grant: None,
        loot_write: None,
        objective_write: Some(obj(vec![("bounty_progress", progress)])),
    };
    if let Err(e) = entry.shared.home().ring_forward_hunt_rewards(&rewards).await {
        tracing::error!("ring_forward_hunt_rewards (bounty) for {account}: {e}");
        return page500();
    }
    ok_envelope(bounty_state_payload(None))
}

async fn post_bounties(call: Call) -> Response {
    let entry = answer!(enter(call).await);
    let target = entry.param("userId");
    if !entry.may_write() {
        tracing::warn!(
            "Rejected bounty write without gameserver authority actor={} target={}",
            log_token(entry.user.as_deref().unwrap_or("unknown")),
            log_token(&target)
        );
        return empty(StatusCode::FORBIDDEN);
    }
    if !bounty_persistence(&entry.shared) || target == INVALID {
        let outcome = if target == INVALID {
            "no-player sentinel, not stored"
        } else {
            "persistence off (stub), not stored"
        };
        save_line(&entry, &target, outcome);
        return ok_envelope(bounty_state_payload(None));
    }
    let Some(store) = entry.store() else {
        return entry.no_database();
    };
    if entry.gs {
        let seams = entry
            .shared
            .slot::<BountiesSeams>()
            .map(|s| (*s).clone())
            .unwrap_or_default();
        if let Some(foreign) = seams.foreign.foreign_account(&entry.shared, &target).await {
            return forward_bounty(&entry, foreign).await;
        }
    }
    let Some(account) = answer!(entry.account(&store, &target).await) else {
        save_line(&entry, &target, "unknown account, not stored");
        return ok_envelope(bounty_state_payload(None));
    };
    if entry.host {
        answer!(entry.host_write(HostWriteKind::Bounty, &account).await);
    }
    let save = match validate_bounty_save(entry.body.as_ref()) {
        Ok(save) => save,
        Err(e) => {
            save_line(&entry, &target, &format!("REJECTED {e}"));
            return bad();
        }
    };
    let (id, host) = (account.clone(), entry.host);
    let merged = guarded(
        &store,
        Some(&account),
        &entry.actor(&account),
        move |tx: &mut Tx<'_>, repos: &Repositories| {
            for _ in 0..8 {
                let stored = repos.bounty_states.get(tx.conn(), &id)?;
                let now_ms = rustic_isles::replay::now_ms();
                let now = crate::util::iso(now_ms);
                let merged = if host {
                    merge_host_bounty_state(stored.as_ref(), &id, &save, &now, now_ms)
                } else {
                    merge_bounty_state(stored.as_ref(), &id, &save, &now)
                };
                if repos
                    .bounty_states
                    .save_if_version(tx, &merged.record, stored.as_ref().map(|r| r.update_version))?
                    .is_some()
                {
                    return Ok(merged);
                }
            }
            Err(RepoError::Data(format!(
                "Bounty state save remained contended for {id}"
            )))
        },
    )
    .await;
    let merged = match merged {
        Ok(merged) => merged,
        Err(e) => return store_error(entry.route, &e),
    };
    let host_part = merged.host.as_ref().map_or_else(String::new, |h| {
        format!(
            " host: keptClaimedWeekly={} ignoredWeekly={} clampedProgress={}",
            h.kept_claimed_weekly, h.ignored_weekly, h.clamped_progress
        )
    });
    save_line(
        &entry,
        &target,
        &format!(
            "stored {}{} incoming={} removed={} -> {} version={}{host_part}",
            merged.mode,
            merged.group.map(|g| format!(":{}", g.as_str())).unwrap_or_default(),
            merged.incoming,
            merged.removed,
            state::counts(Some(&merged.record)),
            merged.record.update_version
        ),
    );
    if !merged.newly_claimed.is_empty() && !target.trim().is_empty() {
        let seams = entry
            .shared
            .slot::<BountiesSeams>()
            .map(|s| (*s).clone())
            .unwrap_or_default();
        seams.activity.challenges_claimed(&target, merged.newly_claimed.len());
    }
    if entry.host {
        entry
            .record(HostWriteKind::Bounty, &account, &state::host_summary(&merged))
            .await;
    }
    ok_envelope(bounty_state_payload(Some(&merged.record)))
}

async fn delete_bounty(call: Call) -> Response {
    let entry = answer!(enter(call).await);
    let target = entry.param("userId");
    if !entry.may_write() {
        tracing::warn!(
            "Rejected bounty delete without gameserver authority actor={} target={}",
            log_token(entry.user.as_deref().unwrap_or("unknown")),
            log_token(&target)
        );
        return empty(StatusCode::FORBIDDEN);
    }
    let on = bounty_persistence(&entry.shared);
    let mut account = None;
    if entry.host || (on && target != INVALID) {
        let Some(store) = entry.store() else {
            return entry.no_database();
        };
        account = answer!(entry.account(&store, &target).await);
    }
    if entry.host {
        let Some(account) = &account else {
            return empty(StatusCode::FORBIDDEN);
        };
        answer!(entry.host_write(HostWriteKind::Bounty, account).await);
    }
    let body_text = entry.body.as_ref().map_or_else(
        || String::from("undefined"),
        |b| crate::util::first_chars(&jsjson::stringify(b), 2000),
    );
    if names_nothing(entry.body.as_ref()) {
        tracing::info!(
            "Bounty reset request acknowledged, nothing changed (source={})",
            entry.source()
        );
        return ok_envelope(empty_object());
    }
    let (Some(account), true) = (account, on && target != INVALID) else {
        return ok_envelope(bounty_state_payload(None));
    };
    let Some(store) = entry.store() else {
        return entry.no_database();
    };
    let request = match parse_delete(entry.body.as_ref()) {
        Ok(request) => request,
        Err(e) => {
            tracing::warn!(
                "[DraftMon] BOUNTY-DELETE user={} src={} body={} => REJECTED {e}",
                log_token(&target),
                entry.source(),
                jsjson::stringify(&serde_json::Value::String(body_text.clone()))
            );
            return bad();
        }
    };
    let id = account.clone();
    let deleted = guarded(
        &store,
        Some(&account),
        &entry.actor(&account),
        move |tx: &mut Tx<'_>, repos: &Repositories| {
            for _ in 0..8 {
                let stored: Option<BountyStateRecord> = repos.bounty_states.get(tx.conn(), &id)?;
                let done = delete_bounties(stored.as_ref(), &id, &request, &now_iso());
                if repos
                    .bounty_states
                    .save_if_version(tx, &done.record, stored.as_ref().map(|r| r.update_version))?
                    .is_some()
                {
                    return Ok(done);
                }
            }
            Err(RepoError::Data(format!("Bounty delete remained contended for {id}")))
        },
    )
    .await;
    match deleted {
        Ok(done) => {
            tracing::info!(
                "[DraftMon] BOUNTY-DELETE user={} src={} body={} deleted=[{}] kept=[{}] -> {} version={}",
                log_token(&target),
                entry.source(),
                jsjson::stringify(&serde_json::Value::String(body_text.clone())),
                done.deleted.iter().map(|d| log_token(d)).collect::<Vec<_>>().join(", "),
                done.kept.iter().map(|d| log_token(d)).collect::<Vec<_>>().join(", "),
                state::counts(Some(&done.record)),
                done.record.update_version
            );
            ok_envelope(bounty_state_payload(Some(&done.record)))
        }
        Err(e) => store_error(entry.route, &e),
    }
}

// ------------------------------------------------------------------ the wallet

/// The caller's balances: an empty wallet for no player; else the stored one (created now with the starter grant).
async fn caller_wallet(entry: &Entry) -> Result<Vec<(String, f64)>, Response> {
    let Some(user) = entry.user.clone().filter(|u| !u.is_empty() && u != INVALID) else {
        return Ok(Vec::new());
    };
    let Some(store) = entry.store() else {
        return Err(entry.no_database());
    };
    let Some(account) = entry.account(&store, &user).await? else {
        return Ok(Vec::new());
    };
    let starter = wallet::starter_balances(&entry.shared);
    let id = account.clone();
    store
        .account_write(&account, &entry.actor(&account), move |tx, repos| {
            wallet::get_wallet(tx, repos, &id, &starter)
        })
        .await
        .map_err(|e| store_error(entry.route, &e))
}

fn balance_of(balances: &[(String, f64)], id: &str) -> f64 {
    balances.iter().find(|(c, _)| c == id).map_or(0.0, |(_, b)| *b)
}

const BALANCE_BASE: [&str; 52] = [
    "id_currency_s20_coin",
    "CURRENCY_GAUNTLET_COIN_FADED",
    "CURRENCY_S20_COIN",
    "CURRENCY_S18_COIN",
    "id_currency_seasonal_coin",
    "id_currency_s18_coin",
    "id_currency_weapon_token",
    "id_currency_celldust",
    "id_currency_event_ramsgiving",
    "CURRENCY_NOTES",
    "id_currency_event_frostfall",
    "CURRENCY_EVENT_DARKHARVEST",
    "CURRENCY_S19_COIN",
    "id_currency_s16_coin",
    "CURRENCY_S16_COIN",
    "id_currency_gauntlet_coin",
    "id_currency_s13_coin",
    "CURRENCY_MARKS_STEEL",
    "CURRENCY_S13_COIN",
    "CURRENCY_EVENT_FROSTFALL",
    "CURRENCY_GAUNTLET_COIN",
    "id_currency_marks_steel",
    "id_currency_rewardcache",
    "CURRENCY_PRESTIGE",
    "CURRENCY_SEASONAL_COIN",
    "CURRENCY_REWARDCACHE",
    "id_currency_token_exchange_speed_up",
    "id_currency_event_springtide",
    "CURRENCY_TOKEN_EXCHANGE_SPEED_UP",
    "id_currency_gauntlet_coin_faded",
    "CURRENCY_S15_COIN",
    "CURRENCY_PLATINUM",
    "id_currency_platinum",
    "id_currency_s15_coin",
    "id_currency_marks_gilded",
    "id_currency_event_darkharvest",
    "id_currency_event_saintsbond",
    "CURRENCY_EVENT_SPRINGTIDE",
    "id_currency_s19_coin",
    "id_currency_notes",
    "id_currency_prestige",
    "id_currency_s13_daily",
    "CURRENCY_WEAPON_TOKEN",
    "CURRENCY_MARKS_GILDED",
    "CURRENCY_S13_DAILY",
    "CURRENCY_CELLDUST",
    "CURRENCY_S14_COIN",
    "CURRENCY_EVENT_SAINTSBOND",
    "CURRENCY_S17_COIN",
    "id_currency_s14_coin",
    "CURRENCY_EVENT_RAMSGIVING",
    "id_currency_s17_coin",
];

async fn get_balance(call: Call) -> Response {
    let entry = answer!(enter(call).await);
    let balances = answer!(caller_wallet(&entry).await);
    let notes = balance_of(&balances, "CURRENCY_NOTES");
    let mut base = Map::new();
    for key in BALANCE_BASE {
        let value = match key {
            "id_currency_weapon_token" | "CURRENCY_WEAPON_TOKEN" => Value::from(25),
            "CURRENCY_NOTES" | "id_currency_notes" => number_value(notes),
            _ => Value::from(0),
        };
        base.insert(key.into(), value);
    }
    tracing::info!(
        "Fetched balance for userId {}: Rams(NOTES)={} CombatMerit(PJM_WEAPON)={}",
        log_token(entry.user.as_deref().unwrap_or("undefined")),
        jsjson::number(notes),
        jsjson::number(balance_of(&balances, "CURRENCY_PJM_WEAPON"))
    );
    ok(&Value::Object(wallet::project_balances(&balances, base)))
}

async fn post_reconcile(call: Call) -> Response {
    let entry = answer!(enter(call).await);
    let balances = answer!(caller_wallet(&entry).await);
    tracing::info!(
        "Reconcile for {}: Rams={} CombatMerit={}",
        log_token(entry.user.as_deref().unwrap_or("undefined")),
        jsjson::number(balance_of(&balances, "CURRENCY_NOTES")),
        jsjson::number(balance_of(&balances, "CURRENCY_PJM_WEAPON"))
    );
    ok(&obj(vec![
        (
            "balances",
            Value::Object(wallet::project_balances(&balances, Map::new())),
        ),
        ("refreshInventory", Value::Bool(true)),
    ]))
}

// ------------------------------------------------------------------ the store

/// A daily section's SKUs for the caller: hidden whenever the claim cannot be checked (§4.13.2).
async fn daily_skus(entry: &Entry, tag: &str, offers: &[Sku]) -> Result<Vec<Sku>, Response> {
    if offers.is_empty() {
        tracing::info!(
            "[Store:{tag}] no daily offer loaded (fountain_daily_bundle.json missing or rejected) - serving none"
        );
        return Ok(Vec::new());
    }
    let Some(user) = entry.user.clone().filter(|u| !u.is_empty() && u != INVALID) else {
        tracing::warn!("[Store:{tag}] daily offer requested without a player id - serving none");
        return Ok(Vec::new());
    };
    let shown = log_token(&user);
    let Some(store) = entry.store() else {
        tracing::warn!("[Store:{tag}] {shown} claim lookup unavailable - hiding the offer");
        return Ok(Vec::new());
    };
    let Some(account) = entry.account(&store, &user).await? else {
        tracing::warn!("[Store:{tag}] {shown} claim lookup unavailable - hiding the offer");
        return Ok(Vec::new());
    };
    let key = crate::bounties::store::daily_window(rustic_isles::replay::now_ms()).2;
    let mut served = Vec::new();
    for sku in offers {
        let id = crate::bounties::store::daily_claim_transaction_id(tag, &user, &sku.id, &key);
        match find_claim(&store, &account, &id).await {
            Ok(None) => served.push(sku.clone()),
            Ok(Some(_)) => {}
            Err(e) => tracing::warn!(
                "[Store:{tag}] {shown} claim lookup failed for {} - hiding the offer: {e}",
                sku.id
            ),
        }
    }
    let ids: Vec<&str> = served.iter().map(|s| s.id.as_str()).collect();
    tracing::info!(
        "[Store:{tag}] {shown} window={key} -> {}",
        if ids.is_empty() {
            String::from("claimed or unavailable")
        } else {
            format!("open ({})", ids.join(", "))
        }
    );
    Ok(served)
}

async fn get_skus(call: Call) -> Response {
    let entry = answer!(enter(call).await);
    let Some(tag) = single_query_value(&entry.query, "requiredTags") else {
        return store_body(400, "missing requiredTags query parameter");
    };
    let catalog = Catalog::cached(&entry.shared);
    let offers = catalog.skus(&tag).to_vec();
    let served = if is_daily_section(&tag) {
        answer!(daily_skus(&entry, &tag, &offers).await)
    } else {
        offers
    };
    if served.is_empty() && !is_daily_section(&tag) {
        tracing::info!(
            "Store SKUs requested for unimplemented tag '{}' - returning empty",
            log_token(&tag)
        );
    }
    let settings = entry.shared.settings();
    ok(&Value::Array(served.iter().map(|s| listing(settings, s)).collect()))
}

async fn get_token(call: Call) -> Response {
    let entry = answer!(enter(call).await);
    let (currency, wanted) = (entry.param("currency"), entry.param("catalogId"));
    let catalog = Catalog::cached(&entry.shared);
    let actor = log_token(entry.user.as_deref().unwrap_or("anonymous"));
    let Some((tag, sku)) = catalog.lookup(&wanted) else {
        tracing::warn!(
            "[Store] GetPurchaseToken: unknown or ambiguous sku/catalogId '{}' (currency={}) for {actor}",
            log_token(&wanted),
            log_token(&currency)
        );
        return store_body(404, "unknown_sku");
    };
    let Some(store) = entry.store() else {
        return entry.no_database();
    };
    let mut character = None;
    if let Some(user) = entry.user.clone().filter(|u| !u.is_empty())
        && let Some(account) = answer!(entry.account(&store, &user).await)
    {
        let id = account.clone();
        let stored_user = store
            .read(move |conn, repos| repos.accounts.find(conn, &id))
            .await
            .ok()
            .flatten()
            .map_or_else(|| account.to_string(), |a| a.user_id.as_str().to_owned());
        let actor = ActorContext::new(Actor::Player {
            account: account.global().clone(),
        })
        .with_request(entry.request_id.as_str());
        let ports = InventoryPorts::default();
        character = answer!(
            ports
                .characters
                .first_character_id(&entry.shared, &store, &actor, &stored_user)
                .await
                .map_err(|e| store_error(entry.route, &e))
        );
    }
    let Some(character) = character else {
        return store_body(500, "no_character");
    };
    let purchase = StorePurchase {
        user_id: entry.user.clone().unwrap_or_default(),
        character_id: character,
        sku_id: sku.id.clone(),
        store_tag: Some(tag.to_owned()),
    };
    match entry.shared.auth().sign_store_token(&purchase) {
        Ok(token) => ok(&obj(vec![("purchaseToken", Value::from(token))])),
        Err(e) => store_error(entry.route, &e),
    }
}

/// The SKU's grant hook (§6.3): its entitlements, then its progression, in the purchase's transaction.
fn grant_hook(shared: &Arc<Shared>, account: &AccountId, sku_id: &str, grants: SkuGrants) -> Option<GrantHook> {
    if grants.is_empty() {
        return None;
    }
    let progression = grants.progression.as_ref().and_then(|p| {
        let id = p.get("progressionId").and_then(Value::as_str).unwrap_or("");
        let track = crate::progression::resolve_track_id(shared, id)?;
        let trunc = |key: &str| match p.get(key) {
            Some(Value::Number(n)) => crate::util::trunc_i64(n.as_f64().unwrap_or(0.0)),
            _ => 0,
        };
        Some((track, trunc("xp"), trunc("ranks")))
    });
    let curves = crate::progression::curves(shared);
    let (id, sku_id) = (account.clone(), sku_id.to_owned());
    Some(Box::new(move |tx: &mut Tx<'_>, repos: &Repositories| {
        grant_entitlements_in_transaction(tx, repos, &id, &grants.entitlements, Some(&sku_id))?;
        if let Some((track, xp, ranks)) = progression {
            if xp > 0 {
                repos.progression_tracks.increment(tx, &id, &track, xp)?;
            } else if ranks > 0
                && let Some(curve) = curves.get(&track)
            {
                let progress = repos
                    .progression_tracks
                    .get(tx.conn(), &id, &track)?
                    .map_or(0, |r| r.progress);
                #[allow(clippy::cast_precision_loss, reason = "track progress is far below 2^53")]
                let progress_f = progress as f64;
                let rank = curve.rank_for_progress(progress_f).unwrap_or(0);
                if rank < curve.max_rank() {
                    let target = (rank + ranks).min(curve.max_rank());
                    if let Some(required) = curve.required_xp_for_rank(target) {
                        let increment = crate::util::trunc_i64(required - progress_f);
                        if increment > 0 {
                            repos.progression_tracks.increment(tx, &id, &track, increment)?;
                        }
                    }
                }
            }
        }
        Ok::<_, TxError>(None)
    }))
}

/// `PurchaseStoreSku` (§4.12).
async fn purchase(entry: &Entry, store: &IslesStore, claims: &StorePurchase, tag: &str) -> Result<(), PurchaseReason> {
    let catalog = Catalog::cached(&entry.shared);
    let sku = catalog
        .from_section(tag, &claims.sku_id)
        .ok_or(PurchaseReason::UnknownSku)?;
    let fresh = || rustic_isles::replay::random_uuid().replace('-', "").to_uppercase();
    let plan = plan_purchase(
        entry.shared.settings(),
        sku,
        tag,
        &claims.user_id,
        rustic_isles::replay::now_ms(),
        &fresh,
    )?;
    let account = entry
        .account(store, &claims.user_id)
        .await
        .ok()
        .flatten()
        .ok_or(PurchaseReason::TransactionFailed)?;
    let user = log_token(&claims.user_id);
    let mut claim_note = "new";
    if is_daily_section(tag) {
        match find_claim(store, &account, &plan.transaction_id).await {
            Err(_) => return Err(PurchaseReason::TransactionFailed),
            Ok(Some(found)) if found.character_id.as_deref() != Some(claims.character_id.as_str()) => {
                return Err(PurchaseReason::AlreadyPurchasedDifferentRequest);
            }
            Ok(Some(_)) => claim_note = "replay (no new grant)",
            Ok(None) => {}
        }
    }
    let request = TxRequest {
        user_id: claims.user_id.clone(),
        character_id: claims.character_id.clone(),
        transaction_id: plan.transaction_id.clone(),
        add_instanced: Value::Array(plan.add_instanced.clone()),
        add_stacked: Value::Array(plan.add_stacked.clone()),
        remove_instanced: Value::Array(Vec::new()),
        remove_stacked: Value::Array(plan.remove_stacked.clone()),
        save_instanced: Value::Array(Vec::new()),
    };
    let options = TxOptions {
        ownership_already_validated: true,
        diagnostic_label: Some(format!("store:{tag}:{}", sku.id)),
        on_grant: grant_hook(&entry.shared, &account, &sku.id, plan.grants.clone()),
        economy: Some(Economy {
            source: "STORE".into(),
            p2p_session_id: None,
            store_sku_id: Some(sku.id.clone()),
        }),
        ..TxOptions::default()
    };
    let ports = InventoryPorts::default();
    let ctx = TxContext::new(&entry.shared, store, &ports);
    match run_inventory_transaction(ctx, request, options, Some(&entry.request_id)).await {
        Ok(TxOutcome::Done(_)) => {
            tracing::info!(
                "[Store:{tag}] {user} purchased {} (transactionId={})",
                sku.id,
                plan.transaction_id
            );
            if let Some(key) = &plan.window_key
                && entry.shared.settings().on_unless_off("RUSTIC_ISLES_ECONOMY_MONITOR")
            {
                let list = |items: &[Value]| {
                    items
                        .iter()
                        .map(|i| {
                            format!(
                                "{} x{}",
                                crate::util::log_token(&dainty_service_kit::js::string(i.get("catalogId"))),
                                dainty_service_kit::js::string(i.get("quantity"))
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                tracing::info!(
                    "[GrantMon] txn={} user={user} src=store source=\"store:{tag}\" sku={} window={key} add=[{}] \
                     remove=[{}] claim={claim_note}",
                    plan.transaction_id,
                    sku.id,
                    list(&plan.add_stacked),
                    list(&plan.remove_stacked)
                );
            }
            Ok(())
        }
        Ok(TxOutcome::NotOwned) => {
            tracing::warn!("[Store:{tag}] {user} purchase of {} failed: not owned", sku.id);
            Err(PurchaseReason::TransactionFailed)
        }
        Err(e) => {
            tracing::warn!("[Store:{tag}] {user} purchase of {} failed: {e}", sku.id);
            Err(match e {
                TxError::InsufficientBalance(_) => PurchaseReason::InsufficientBalance,
                TxError::Conflict(_) => PurchaseReason::Conflict,
                TxError::Mismatch(_) => PurchaseReason::AlreadyPurchasedDifferentRequest,
                _ => PurchaseReason::TransactionFailed,
            })
        }
    }
}

async fn post_notification(call: Call) -> Response {
    let entry = answer!(enter(call).await);
    let Some(token) = single_query_value(&entry.query, "token") else {
        return store_body(400, "missing token");
    };
    let Some(store) = entry.store() else {
        return entry.no_database();
    };
    let Ok(claims) = entry.shared.auth().validate_store_token(&token) else {
        return store_body(401, "invalid_token");
    };
    if entry.user.as_deref() != Some(claims.user_id.as_str()) {
        return store_body(403, "token_user_mismatch");
    }
    let tag = match &claims.store_tag {
        Some(tag) => tag.clone(),
        None => match Catalog::cached(&entry.shared).lookup(&claims.sku_id) {
            Some((tag, _)) => tag.to_owned(),
            None => return store_body(404, "unknown_sku"),
        },
    };
    let actor = log_token(&claims.user_id);
    match purchase(&entry, &store, &claims, &tag).await {
        Ok(()) => {
            tracing::info!(
                "[Store:{tag}] purchase completed for {actor} sku={}",
                log_token(&claims.sku_id)
            );
            empty(StatusCode::NO_CONTENT)
        }
        Err(reason) => {
            tracing::warn!(
                "[Store:{tag}] purchase denied for {actor} sku={}: {}",
                log_token(&claims.sku_id),
                reason.as_str()
            );
            store_body(reason.status(), reason.as_str())
        }
    }
}
