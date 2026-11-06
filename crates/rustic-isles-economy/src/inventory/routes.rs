// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The eight `inventory-reforge` handlers (spec §3 to §5).

use std::sync::Arc;

use axum::response::Response;
use http::StatusCode;
use module_api::network::HuntRewards;
use module_api::{GlobalId, IdKind, ModuleId};
use rifts_seam::{
    HostWrite, HostWriteKind, HostWriteOptions, InventoryDecision, InventoryFence, InventoryTransaction, RuntimeAuth,
    SeamError, TransactionPhase,
};
use rustic_isles::{Call, Handlers, Method, NoPlayerPolicy, Params, Service, Shared, read_body};
use rustic_isles_store::{AccountId, IslesStore, RepoError, UserId};
use serde_json::{Map, Value};
use service_kit::{js, jsjson};

use crate::bounties::challenge_rewards::is_challenge_reward_grant;
use crate::inventory::ports::{AuthRequest, Authentication, Caller, InventoryPorts};
use crate::inventory::prestige::ProgressionConfig;
use crate::inventory::reforge::{
    self as reforge, GrantFailure, PurchasePlan, ResetOutcome, RestoreGrant, plan_purchase_restore,
    single_track_payload,
};
use crate::inventory::transaction::{
    ItemUpdate, TxContext, TxError, TxOptions, TxOutcome, TxRequest, get_inventory, run_inventory_transaction,
    system_actor, update_instanced_item,
};
use crate::inventory::validation::{
    inventory_body_has_grant_or_spend, validate_instanced_item_update_body, validate_inventory_transaction_body,
};
use crate::inventory::weapon_level::serve_weapons_at_base_level;
use crate::multipliers::{multiply_reward_items, read_test_multipliers};
use crate::progression::reforge::reforge_mode;
use crate::progression::tracks::{MAX_PROGRESSION_GRANT_AMOUNT, is_reforgeable_track};
use crate::util::{
    coded, empty, empty_json, envelope, first_units, js_number_text, json, log_token, not_implemented, num, obj, ok,
    page500, refusal, utf16_len,
};

const INVALID: &str = "INVALID";
const NO_DATABASE: &str = "isles: no database is configured (DAINTY_DB_FILE or DAINTY_DB_ENGINE)";
const WIRE_CREDENTIALS: [&str; 3] = [
    "authorization",
    "x-rustic-isles-gameserver-apikey",
    "x-rustic-isles-gameserver-key",
];

const ROUTES: [(Method, &str); 8] = [
    (Method::Post, r"/^\/inventory\/([^/]*)\/(.+)$/"),
    (Method::Post, "/inventory/:characterId/:changeList"),
    (Method::Get, "/inventory/:userId/:characterId"),
    (Method::Get, "/inventory/:userId/"),
    (Method::Post, "/inventory"),
    (Method::Post, "/inventory/instanceditem"),
    (Method::Delete, "/progression/:userId/:progressionId"),
    (Method::Post, "/progression/purchase/:userId/:progressionId/:amount"),
];

/// The (method, path) rows this part answers, in the table's order.
pub fn routes() -> &'static [(Method, &'static str)] {
    &ROUTES
}

/// Adds the eight handlers (core §5.7); the regex row goes first so `POST /inventory//<build>` reaches it.
pub fn register(handlers: &mut Handlers, ports: InventoryPorts) {
    let ports = Arc::new(ports);
    let b = Service::Backend;
    macro_rules! on {
        ($method:expr, $path:expr, $handler:ident) => {{
            let ports = Arc::clone(&ports);
            handlers.on(b, $method, $path, move |call| $handler(call, Arc::clone(&ports)));
        }};
    }
    on!(Method::Post, ROUTES[0].1, migration_regex);
    on!(Method::Post, ROUTES[1].1, migration);
    on!(Method::Get, ROUTES[2].1, get_character_inventory);
    on!(Method::Get, ROUTES[3].1, get_first_inventory);
    on!(Method::Post, ROUTES[4].1, post_inventory);
    on!(Method::Post, ROUTES[5].1, post_instanced_item);
    on!(Method::Delete, ROUTES[6].1, delete_progression);
    on!(Method::Post, ROUTES[7].1, purchase_progression);
}

macro_rules! answer {
    ($e:expr) => {
        match $e {
            Ok(value) => value,
            Err(response) => return response,
        }
    };
}

struct Entry {
    shared: Arc<Shared>,
    params: Params,
    request_id: String,
    body: Option<Value>,
    caller: Caller,
    ports: Arc<InventoryPorts>,
}

impl Entry {
    fn param(&self, name: &str) -> String {
        self.params.get(name).unwrap_or("").to_owned()
    }

    fn store(&self) -> Result<Arc<IslesStore>, Response> {
        self.shared
            .slot::<IslesStore>()
            .ok_or_else(|| not_implemented(NO_DATABASE, &self.request_id))
    }

    fn ctx<'a>(&'a self, store: &'a IslesStore) -> TxContext<'a> {
        TxContext::new(&self.shared, store, &self.ports)
    }

    fn policy(&self) -> NoPlayerPolicy {
        self.shared.version().hooks.no_player_policy()
    }

    fn base_level(&self, value: &Value) -> Value {
        let enabled = self
            .shared
            .version()
            .hooks
            .weapons_at_base_level(self.shared.settings());
        serve_weapons_at_base_level(value, enabled)
    }

    /// §3.3
    fn repo_error(&self, error: &RepoError, request_id: &str) -> Response {
        match error {
            RepoError::Unimplemented { .. } => not_implemented(&format!("isles: {error}"), request_id),
            other => {
                tracing::error!("inventory: {other}");
                page500()
            }
        }
    }

    /// §6.5
    fn tx_error(&self, error: &TxError) -> Response {
        let conflict = |message: &str| refusal(StatusCode::CONFLICT, message);
        match error {
            TxError::InsufficientBalance(text) => {
                tracing::warn!("{text}");
                conflict("Insufficient balance")
            }
            TxError::InsufficientPrestige(text) => {
                tracing::warn!("{text}");
                conflict("Insufficient banked prestige progress")
            }
            TxError::InsufficientStackedItem { .. } => {
                tracing::warn!("{error}");
                conflict("Insufficient stacked item")
            }
            TxError::Mismatch(_) => conflict("Transaction id reused with a different request body"),
            TxError::Conflict(_) => conflict("Transaction already in progress"),
            TxError::Denied { status, body } => json(crate::util::status(*status), body),
            TxError::Policy(e) => {
                tracing::error!("inventory: {e}");
                page500()
            }
            TxError::Repo(e) => self.repo_error(e, &self.request_id),
        }
    }

    /// The first character of `user` (§11.3), as the system.
    async fn first_character(&self, store: &IslesStore, user: &str) -> Result<Option<String>, Response> {
        let actor = system_actor(Some(&self.request_id));
        self.ports
            .characters
            .first_character_id(&self.shared, store, &actor, user)
            .await
            .map_err(|e| self.repo_error(&e, &self.request_id))
    }
}

/// Body, then game auth only on wire credentials, then the authenticator port (§3.1).
async fn enter(call: Call, ports: Arc<InventoryPorts>) -> Result<Entry, Response> {
    let Call {
        shared,
        params,
        mut request,
        request_id,
        ..
    } = call;
    let body = read_body(&mut request).await?;
    let wire = WIRE_CREDENTIALS
        .iter()
        .any(|name| request.headers().contains_key(*name));
    if request.extensions().get::<rustic_isles_auth::AuthData>().is_none() && wire {
        shared.auth().game_auth(&mut request).await?;
    }
    let (parts, _) = request.into_parts();
    let caller = authenticate(&shared, &ports, &parts, &request_id).await?;
    Ok(Entry {
        shared,
        params,
        request_id,
        body,
        caller,
        ports,
    })
}

async fn authenticate(
    shared: &Shared,
    ports: &InventoryPorts,
    parts: &http::request::Parts,
    request_id: &str,
) -> Result<Caller, Response> {
    let outcome = ports
        .auth
        .authenticate(
            shared,
            AuthRequest {
                method: &parts.method,
                path: parts.uri.path(),
                headers: &parts.headers,
                extensions: &parts.extensions,
                request_id,
            },
        )
        .await;
    match outcome {
        Authentication::Authenticated(caller) => Ok(caller),
        Authentication::Answer(response) => Err(response),
    }
}

/// §4.7: always 400 for a refusal (spec §14 item 10).
fn seam_error(error: &SeamError, request_id: &str) -> Response {
    match error {
        SeamError::Unimplemented(missing) => {
            crate::util::reply(&rifts_seam::Reply::not_implemented(missing, request_id))
        }
        SeamError::Refused(refusal) => json(
            StatusCode::BAD_REQUEST,
            &obj(vec![(
                "error",
                obj(vec![
                    ("code", Value::from(refusal.code.as_str())),
                    ("message", Value::from(refusal.message.as_str())),
                    ("requestId", Value::from(request_id)),
                ]),
            )]),
        ),
        other => {
            tracing::error!("inventory: {other}");
            page500()
        }
    }
}

fn no_capability(message: &str) -> Response {
    coded(StatusCode::FORBIDDEN, "forbidden", message)
}

// ------------------------------------------------------------------ B68, B69

async fn migration_regex(call: Call, ports: Arc<InventoryPorts>) -> Response {
    let entry = answer!(enter(call, ports).await);
    tracing::info!(
        "Inventory migration (stubbed) characterId='{}' changeList='{}'",
        log_token(&entry.param("0")),
        log_token(&entry.param("1"))
    );
    ok(&obj(vec![("code", Value::from("NONE")), ("message", Value::from(""))]))
}

async fn migration(call: Call, ports: Arc<InventoryPorts>) -> Response {
    let _entry = answer!(enter(call, ports).await);
    tracing::info!("Inventory migration (stubbed)");
    ok(&obj(vec![("code", Value::from("NONE")), ("message", Value::from(""))]))
}

// ------------------------------------------------------------------ B70, B71

fn empty_inventory(character: &str) -> Value {
    obj(vec![
        ("characterId", Value::from(character)),
        ("instancedItems", Value::Array(Vec::new())),
        ("stackedItems", Value::Array(Vec::new())),
    ])
}

fn read_user(entry: &Entry) -> Option<String> {
    if entry.caller.is_gameserver {
        Some(entry.param("userId"))
    } else {
        entry.caller.user_id.clone()
    }
}

async fn get_character_inventory(call: Call, ports: Arc<InventoryPorts>) -> Response {
    let entry = answer!(enter(call, ports).await);
    let user = read_user(&entry);
    let character = entry.param("characterId");
    if user.as_deref() == Some(INVALID) {
        return ok(&empty_inventory(&character));
    }
    let store = answer!(entry.store());
    tracing::info!(
        "UserId {} requested inventory for CharacterId {}",
        user.as_deref().map_or_else(|| String::from("undefined"), log_token),
        log_token(&character)
    );
    match get_inventory(entry.ctx(&store), user.as_deref(), &character, Some(&entry.request_id)).await {
        Ok(Some(inventory)) => ok(&entry.base_level(&inventory)),
        Ok(None) => empty(StatusCode::BAD_REQUEST),
        Err(e) => entry.tx_error(&e),
    }
}

async fn get_first_inventory(call: Call, ports: Arc<InventoryPorts>) -> Response {
    let entry = answer!(enter(call, ports).await);
    let user = read_user(&entry);
    if user.as_deref() == Some(INVALID) {
        return ok(&empty_inventory(INVALID));
    }
    if let Some(user) = user.as_deref().filter(|u| !u.is_empty())
        && entry.policy() == NoPlayerPolicy::ConfiguredAccount
        && entry.caller.is_gameserver
    {
        let store = answer!(entry.store());
        if let Some(character) = answer!(entry.first_character(&store, user).await).filter(|c| !c.is_empty()) {
            match get_inventory(entry.ctx(&store), Some(user), &character, Some(&entry.request_id)).await {
                Ok(Some(inventory)) => {
                    tracing::info!(
                        "GET /inventory (no characterId): gameserver request resolved to '{character}' for {}",
                        log_token(user)
                    );
                    return ok(&entry.base_level(&inventory));
                }
                Ok(None) => {}
                Err(e) => return entry.tx_error(&e),
            }
        }
    }
    empty(StatusCode::BAD_REQUEST)
}

// ------------------------------------------------------------------ B72

fn echo(body: &Map<String, Value>, key: &str) -> Value {
    match body.get(key) {
        None | Some(Value::Null) => Value::Array(Vec::new()),
        Some(other) => other.clone(),
    }
}

fn collection(body: &Map<String, Value>, key: &str) -> Value {
    body.get(key).cloned().unwrap_or(Value::Null)
}

fn body_user(entry: &Entry, body: &Map<String, Value>) -> Option<String> {
    if !entry.caller.is_gameserver {
        return entry.caller.user_id.clone();
    }
    match body.get("accountId") {
        Some(Value::String(account)) => Some(account.clone()),
        None | Some(Value::Null) => entry.caller.user_id.clone(),
        Some(_) => None,
    }
}

/// The account global id `user` names, when its home is another node (§4.4 step 10).
async fn foreign_account(shared: &Shared, store: &IslesStore, user: &str) -> Result<Option<GlobalId>, RepoError> {
    let here = shared.node().node.clone();
    let global = match user.parse::<GlobalId>() {
        Ok(id) if id.kind() == IdKind::Account => Some(id),
        _ => {
            let user_id = UserId::new(user);
            store
                .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &user_id))
                .await?
                .map(|a| a.id.global().clone())
        }
    };
    Ok(global.filter(|id| id.owner() != &here))
}

pub(crate) fn module_id(shared: &Shared) -> Option<ModuleId> {
    ModuleId::new(format!(
        "isles-{}",
        shared.version().tables.game_version.replace('.', "-")
    ))
    .ok()
}

/// §4.4.1: removals, saves and item data are not forwarded (spec §14 item 8).
async fn forward_home(entry: &Entry, body: &Map<String, Value>, account: GlobalId) -> Response {
    let mut loot = Vec::new();
    for add in body
        .get("addStackedItems")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(id) = add.get("catalogId").and_then(Value::as_str) {
            let q = match add.get("quantity") {
                Some(Value::Number(n)) if n.is_i64() || n.is_u64() => n.as_f64().unwrap_or(1.0),
                _ => 1.0,
            };
            loot.push(Value::from(format!("{id}:{}", jsjson::number(q.max(1.0)))));
        }
    }
    for add in body
        .get("addInstancedItems")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(id) = add.get("catalogId").and_then(Value::as_str) {
            loot.push(Value::from(format!("{id}:1")));
        }
    }
    let hunt = crate::inventory::ring_instance(&entry.shared, &entry.ports, &account)
        .unwrap_or_else(|| GlobalId::new(IdKind::Hunt, &entry.shared.node().node));
    let Some(module) = module_id(&entry.shared) else {
        return page500();
    };
    let rewards = HuntRewards {
        hunt,
        module,
        account: account.clone(),
        xp_grant: None,
        loot_write: Some(obj(vec![("loot", Value::Array(loot))])),
        objective_write: None,
    };
    if let Err(e) = entry.shared.home().ring_forward_hunt_rewards(&rewards).await {
        tracing::error!("ring_forward_hunt_rewards (loot) for {account}: {e}");
        return page500();
    }
    ok(&obj(vec![
        ("createdInstancedItems", Value::Array(Vec::new())),
        ("updatedInstancedItems", echo(body, "saveInstancedItems")),
        ("updatedStackedItems", echo(body, "addStackedItems")),
        ("removedInstancedItems", echo(body, "removeInstancedItems")),
        ("forwarded_to_home", Value::Bool(true)),
    ]))
}

async fn post_inventory(call: Call, ports: Arc<InventoryPorts>) -> Response {
    let entry = answer!(enter(call, ports).await);
    let Some(Value::Object(mut body)) = entry.body.clone() else {
        tracing::warn!("Invalid inventory request: body must be an object");
        return refusal(
            StatusCode::BAD_REQUEST,
            "Invalid inventory request: body must be an object",
        );
    };
    let gs = entry.caller.is_gameserver;
    let configured = entry.policy() == NoPlayerPolicy::ConfiguredAccount;
    let mut user = body_user(&entry, &body);
    if configured
        && gs
        && user.as_deref().is_none_or(|u| u.is_empty() || u == INVALID)
        && let Some(account) = entry.policy().bearerless_account(entry.shared.settings())
    {
        tracing::info!(
            "POST /inventory: gameserver request with no accountId resolved to '{}'",
            log_token(&account)
        );
        user = Some(account);
    }
    let character_missing = !body
        .get("characterId")
        .and_then(Value::as_str)
        .is_some_and(|c| !c.is_empty());
    if let Some(name) = user.clone().filter(|u| !u.is_empty() && u != INVALID)
        && configured
        && gs
        && character_missing
    {
        let store = answer!(entry.store());
        if let Some(character) = answer!(entry.first_character(&store, &name).await).filter(|c| !c.is_empty()) {
            tracing::info!(
                "POST /inventory: gameserver request with no characterId resolved to '{character}' for {}",
                log_token(&name)
            );
            body.insert("characterId".into(), Value::from(character));
        }
    }
    if user.as_deref() == Some(INVALID) {
        return ok(&obj(vec![
            ("createdInstancedItems", Value::Array(Vec::new())),
            ("updatedInstancedItems", echo(&body, "saveInstancedItems")),
            ("updatedStackedItems", echo(&body, "addStackedItems")),
            ("removedInstancedItems", echo(&body, "removeInstancedItems")),
        ]));
    }
    let body_value = Value::Object(body.clone());
    if let Some(message) = validate_inventory_transaction_body(&body_value) {
        tracing::warn!("Invalid inventory request: {message}");
        return refusal(
            StatusCode::BAD_REQUEST,
            &format!("Invalid inventory request: {message}"),
        );
    }
    let character = body
        .get("characterId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let transaction_id = body
        .get("transactionId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let mut fence: Option<Arc<dyn InventoryFence>> = None;
    if let Some(runtime) = &entry.caller.runtime {
        let Some(capability) = runtime.capability.clone() else {
            return no_capability("This runtime cannot mutate inventory.");
        };
        let transaction = InventoryTransaction {
            user: rifts_seam::AccountId(user.clone().unwrap_or_default()),
            character_id: character.clone(),
            transaction_id: transaction_id.clone(),
            body: Some(rifts_seam::Json(body_value.to_string())),
        };
        match entry
            .shared
            .rifts()
            .guard()
            .authorize_inventory(capability, transaction)
            .await
        {
            Ok(InventoryDecision::Authorized(granted)) => fence = Some(Arc::from(granted)),
            Ok(InventoryDecision::Refused(reply)) => return crate::util::reply(&reply),
            Err(e) => return seam_error(&e, &entry.request_id),
        }
    }
    if !gs && fence.is_none() && inventory_body_has_grant_or_spend(&body_value) {
        tracing::error!(
            "Rejected inventory grant or spend without gameserver authority for userId {}",
            user.as_deref().map_or_else(|| String::from("undefined"), log_token)
        );
        return refusal(
            StatusCode::FORBIDDEN,
            "Authoritative inventory mutations require gameserver authority",
        );
    }
    let store = answer!(entry.store());
    let dedicated = gs && fence.is_none();
    if dedicated && let Some(name) = user.as_deref() {
        match foreign_account(&entry.shared, &store, name).await {
            Ok(Some(account)) => return forward_home(&entry, &body, account).await,
            Ok(None) => {}
            Err(e) => return entry.repo_error(&e, &entry.request_id),
        }
    }
    let coin = entry.shared.version().tables.seasonal_coin;
    let challenge = body
        .get("addStackedItems")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|a| {
            a.get("catalogId")
                .and_then(Value::as_str)
                .is_some_and(|id| is_challenge_reward_grant(coin, id, num(a.get("quantity"))))
        });
    if (dedicated || fence.is_some()) && challenge {
        let user_id = UserId::new(user.clone().unwrap_or_default());
        let account = store
            .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &user_id))
            .await;
        match account {
            Ok(Some(account)) => {
                if !entry
                    .ports
                    .wallet
                    .wait_for_fundable_season_claim(&store, &account.id)
                    .await
                {
                    tracing::warn!(
                        "transactionId {}: challenge-reward currency with no claimed, uncredited season challenge \
                         after 1.5 s",
                        log_token(&transaction_id)
                    );
                }
            }
            Ok(None) => {}
            Err(e) => return entry.repo_error(&e, &entry.request_id),
        }
    }
    if dedicated && let Some(adds) = body.get("addStackedItems").and_then(Value::as_array).cloned() {
        let multipliers = read_test_multipliers(entry.shared.settings());
        let prestige = ProgressionConfig::load(&entry.shared);
        let excluded =
            |id: &str, q: f64| prestige.is_prestige_gated_reward(id) || is_challenge_reward_grant(coin, id, q);
        if let Some(multiplied) = multiply_reward_items(&adds, body.get("source"), multipliers.rewards, &excluded) {
            tracing::info!(
                "[TestMultiplier] transactionId {} source=\"{}\" rewards multiplied for userId {}",
                log_token(&transaction_id),
                first_units(&js::string(body.get("source")), 80).escape_debug(),
                user.as_deref().map_or_else(|| String::from("undefined"), log_token)
            );
            body.insert("addStackedItems".into(), Value::Array(multiplied));
        }
    }
    let grant_hook: Option<crate::inventory::GrantHook> = fence.clone().map(|fence| {
        let grant = jsjson::stringify(&Value::Object(body.clone()));
        let hook: crate::inventory::GrantHook =
            Box::new(move |_tx, _repos| match fence.on_grant(&rifts_seam::Json(grant)) {
                Ok(()) => Ok(None),
                Err(reply) => Err(TxError::Denied {
                    status: reply.status,
                    body: jsjson::parse(&reply.body.0).unwrap_or(Value::Null),
                }),
            });
        hook
    });
    let request = TxRequest {
        user_id: user.clone().unwrap_or_default(),
        character_id: character.clone(),
        transaction_id: transaction_id.clone(),
        add_instanced: collection(&body, "addInstancedItems"),
        add_stacked: collection(&body, "addStackedItems"),
        remove_instanced: collection(&body, "removeInstancedItems"),
        remove_stacked: collection(&body, "removeStackedItems"),
        save_instanced: collection(&body, "saveInstancedItems"),
    };
    let options = TxOptions {
        on_grant: grant_hook,
        allow_challenge_reward_funding: dedicated || fence.is_some(),
        economy: Some(crate::inventory::Economy {
            source: String::from(if dedicated { "INVENTORY_GAMESERVER" } else { "OTHER" }),
            p2p_session_id: None,
            store_sku_id: None,
        }),
        ..TxOptions::default()
    };
    if let Some(fence) = &fence {
        fence.report(TransactionPhase::Applying);
    }
    match run_inventory_transaction(entry.ctx(&store), request, options, Some(&entry.request_id)).await {
        Ok(TxOutcome::Done(result)) => {
            if let Some(fence) = &fence {
                fence.report(TransactionPhase::Accepted);
            }
            tracing::info!(
                "Ran transactionId {} for userId {} and characterId {}",
                log_token(&transaction_id),
                log_token(user.as_deref().unwrap_or("")),
                log_token(&character)
            );
            ok(&entry.base_level(&result))
        }
        Ok(TxOutcome::NotOwned) => {
            if let Some(fence) = &fence {
                fence.report(TransactionPhase::Failed);
            }
            tracing::error!(
                "transactionId {}: characterId {} is not owned by userId {}",
                log_token(&transaction_id),
                log_token(&character),
                log_token(user.as_deref().unwrap_or(""))
            );
            empty(StatusCode::BAD_REQUEST)
        }
        Err(e) => entry.tx_error(&e),
    }
}

// ------------------------------------------------------------------ B73

async fn post_instanced_item(call: Call, ports: Arc<InventoryPorts>) -> Response {
    let entry = answer!(enter(call, ports).await);
    let Some(Value::Object(body)) = entry.body.clone() else {
        return refusal(StatusCode::BAD_REQUEST, "Invalid item update: body must be an object");
    };
    let mut capability: Option<RuntimeAuth> = None;
    if let Some(runtime) = &entry.caller.runtime {
        let Some(target) = entry.caller.user_id.clone().filter(|u| !u.is_empty()) else {
            return no_capability("Player-host runtime cannot mutate inventory.");
        };
        let Some(cap) = runtime.capability.clone() else {
            return no_capability("This runtime cannot write player state.");
        };
        let asked = entry.shared.rifts().guard().authorize_host_write(
            cap.clone(),
            HostWriteKind::ItemSave,
            rifts_seam::AccountId(target),
            HostWriteOptions::default(),
        );
        match asked.await {
            Ok(None) => capability = Some(cap),
            Ok(Some(reply)) => {
                tracing::warn!("POST /inventory/instanceditem rejected: the host write was refused");
                return crate::util::reply(&reply);
            }
            Err(e) => return seam_error(&e, &entry.request_id),
        }
    }
    let user = body_user(&entry, &body);
    if user.as_deref() == Some(INVALID) {
        let mut echo = Map::new();
        for key in ["characterId", "instanceId", "catalogId", "itemData", "updateVersion"] {
            if let Some(value) = body.get(key) {
                echo.insert(key.into(), value.clone());
            }
        }
        return ok(&Value::Object(echo));
    }
    let body_value = Value::Object(body.clone());
    if let Some(message) = validate_instanced_item_update_body(&body_value) {
        return refusal(StatusCode::BAD_REQUEST, &format!("Invalid item update: {message}"));
    }
    let store = answer!(entry.store());
    let update = ItemUpdate {
        character_id: body
            .get("characterId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        user_id: user.clone(),
        instance_id: body.get("instanceId").cloned().unwrap_or(Value::Null),
        catalog_id: body.get("catalogId").cloned().unwrap_or(Value::Null),
        item_data: body.get("itemData").cloned(),
        update_version: body.get("updateVersion").cloned(),
    };
    let item = match update_instanced_item(entry.ctx(&store), &update, Some(&entry.request_id)).await {
        Ok(item) => item,
        Err(e) => return entry.tx_error(&e),
    };
    if let (Some(cap), Some(_), Some(target)) = (capability, item.as_ref(), user.as_deref()) {
        let mut summary = Map::new();
        summary.insert(
            "instanceId".into(),
            Value::from(first_units(&js::string(body.get("instanceId")), 64)),
        );
        summary.insert(
            "catalogId".into(),
            Value::from(first_units(&js::string(body.get("catalogId")), 128)),
        );
        if let Some(version) = body.get("updateVersion") {
            summary.insert("updateVersion".into(), version.clone());
        }
        let bytes = body.get("itemData").and_then(Value::as_str).map_or(0, utf16_len);
        summary.insert("itemDataBytes".into(), Value::from(bytes));
        record(
            &entry.shared,
            cap,
            HostWriteKind::ItemSave,
            target,
            &Value::Object(summary),
        )
        .await;
    }
    match item {
        Some(item) => ok(&entry.base_level(&item)),
        None => empty_json(),
    }
}

async fn record(shared: &Shared, capability: RuntimeAuth, kind: HostWriteKind, target: &str, summary: &Value) {
    let write = HostWrite {
        kind,
        target: rifts_seam::AccountId(target.to_owned()),
        summary: rifts_seam::Json(jsjson::stringify(summary)),
    };
    if let Err(e) = shared.rifts().guard().record_host_write(capability, write).await {
        tracing::warn!(
            "[HostWrite] tag failed kind={} target={}: {e}",
            crate::caller::kind_name(kind),
            log_token(target)
        );
    }
}

// ------------------------------------------------------------------ B130, B131

/// §5.4: the capability to tag with (none for a game server), or the answer.
async fn authorize_reforge(entry: &Entry, user: &str) -> Result<Option<RuntimeAuth>, Response> {
    if let Some(runtime) = &entry.caller.runtime {
        let Some(cap) = runtime.capability.clone() else {
            return Err(no_capability("This runtime cannot write player state."));
        };
        let asked = entry.shared.rifts().guard().authorize_host_write(
            cap.clone(),
            HostWriteKind::Reforge,
            rifts_seam::AccountId(user.to_owned()),
            HostWriteOptions::default(),
        );
        return match asked.await {
            Ok(None) => Ok(Some(cap)),
            Ok(Some(reply)) => Err(crate::util::reply(&reply)),
            Err(e) => Err(seam_error(&e, &entry.request_id)),
        };
    }
    if !entry.caller.is_gameserver {
        return Err(refusal(StatusCode::FORBIDDEN, "Forbidden"));
    }
    Ok(None)
}

fn conflict() -> Response {
    refusal(StatusCode::CONFLICT, "Conflict")
}

async fn account_of(entry: &Entry, store: &IslesStore, user: &str) -> Result<Option<AccountId>, Response> {
    let user_id = UserId::new(user);
    store
        .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &user_id))
        .await
        .map(|a| a.map(|a| a.id))
        // Parity: this lookup's 501 carries an empty request id (spec §14 item 7)
        .map_err(|e| entry.repo_error(&e, ""))
}

async fn delete_progression(call: Call, ports: Arc<InventoryPorts>) -> Response {
    let entry = answer!(enter(call, ports).await);
    let (user, track) = (entry.param("userId"), entry.param("progressionId"));
    tracing::info!(
        "[Reforge] DELETE-REQ user={} track={} gs={}{}",
        log_token(&user),
        log_token(&track),
        u8::from(entry.caller.is_gameserver),
        if entry.caller.runtime.is_some() {
            " src=host"
        } else {
            ""
        }
    );
    let capability = answer!(authorize_reforge(&entry, &user).await);
    if user == INVALID || !is_reforgeable_track(&track) {
        tracing::warn!(
            "[Reforge] reset refused for {} {}: not a reforgeable track",
            log_token(&user),
            log_token(&track)
        );
        return conflict();
    }
    let store = answer!(entry.store());
    let Some(account) = answer!(account_of(&entry, &store, &user).await) else {
        return conflict();
    };
    let mode = reforge_mode(&entry.shared);
    let now = rustic_isles::replay::now_ms();
    let reset = reforge::reforge_reset(
        &entry.shared,
        &store,
        mode,
        &account,
        &user,
        &track,
        now,
        Some(&entry.request_id),
    )
    .await;
    let reset = match reset {
        Ok(reset) => reset,
        Err(e) => return entry.tx_error(&TxError::Repo(e)),
    };
    if reset.outcome == ResetOutcome::Reset {
        entry.ports.observer.reforge_reset(&entry.shared, &account, now);
        if let (Some(cap), Some(stored)) = (capability, reset.row.as_ref().and_then(|r| r.reforge.as_ref())) {
            let summary = obj(vec![
                ("route", Value::from("delete")),
                ("track", Value::from(track.as_str())),
                ("reforgeId", Value::from(stored.reforge_id.as_str())),
                ("priorProgress", Value::from(stored.prior_progress)),
                ("cap", Value::from(stored.cap)),
                ("banked", Value::from(stored.banked)),
            ]);
            record(&entry.shared, cap, HostWriteKind::Reforge, &user, &summary).await;
        }
    }
    if reset.status == 200 {
        envelope(
            StatusCode::OK,
            "OK",
            single_track_payload(&user, &track, reset.row.as_ref()),
        )
    } else {
        conflict()
    }
}

async fn purchase_progression(call: Call, ports: Arc<InventoryPorts>) -> Response {
    let entry = answer!(enter(call, ports).await);
    let (user, track, raw) = (
        entry.param("userId"),
        entry.param("progressionId"),
        entry.param("amount"),
    );
    let amount = js_number_text(&raw);
    tracing::info!(
        "[Reforge] PURCHASE-REQ user={} track={} amount={} gs={}",
        log_token(&user),
        log_token(&track),
        log_token(&raw),
        u8::from(entry.caller.is_gameserver)
    );
    let capability = answer!(authorize_reforge(&entry, &user).await);
    let whole = amount.is_finite() && amount.fract() == 0.0;
    if user == INVALID || !whole || amount <= 0.0 || amount > MAX_PROGRESSION_GRANT_AMOUNT {
        return refusal(StatusCode::BAD_REQUEST, "Bad Request");
    }
    let store = answer!(entry.store());
    let account = answer!(account_of(&entry, &store, &user).await);
    let row = match &account {
        Some(account) => {
            let (id, t) = (account.clone(), track.clone());
            match store
                .read(move |conn, repos| repos.progression_tracks.get(conn, &id, &t))
                .await
            {
                Ok(row) => row,
                Err(e) => return entry.repo_error(&e, &entry.request_id),
            }
        }
        None => None,
    };
    let mode = reforge_mode(&entry.shared);
    let plan = plan_purchase_restore(mode, &track, amount, row.as_ref(), rustic_isles::replay::now_ms());
    let refuse = |reason: &str| {
        tracing::info!(
            "[Reforge] DUP/REFUSED purchase user={} track={} amount={}: {reason}",
            log_token(&user),
            log_token(&track),
            log_token(&raw)
        );
        conflict()
    };
    let (grant_id, reforge_plan) = match plan {
        PurchasePlan::Refuse(reason) => return refuse(&reason),
        PurchasePlan::Restore { grant_id, plan } => (grant_id, plan),
        PurchasePlan::Normal { grant_id } => {
            let id = grant_id.clone();
            match store
                .read(move |conn, repos| repos.progression_grants.find_by_grant_id(conn, &id))
                .await
            {
                Ok(Some(_)) => (grant_id, crate::progression::reforge::ReforgePlan::Normal),
                Ok(None) => return refuse("the banked XP was already restored (stale-reforge recovery)"),
                Err(e) => return entry.repo_error(&e, &entry.request_id),
            }
        }
    };
    let Some(account) = account else {
        return refuse("no account");
    };
    let grant = RestoreGrant {
        grant_id: grant_id.clone(),
        user_id: user.clone(),
        track: track.clone(),
        amount: crate::util::whole(amount),
        plan: reforge_plan,
    };
    let granted =
        match reforge::grant_banked_restore(&entry.shared, &store, &account, grant, Some(&entry.request_id)).await {
            Ok(granted) => granted,
            Err(GrantFailure::ReforgeConflict(reason) | GrantFailure::GrantConflict(reason)) => return refuse(&reason),
            Err(GrantFailure::Data(text)) => {
                tracing::error!("inventory: {text}");
                return page500();
            }
            Err(GrantFailure::Repo(e)) => return entry.repo_error(&e, &entry.request_id),
        };
    if let Some(cap) = capability
        && !granted.replayed
    {
        let mut summary = obj(vec![
            ("route", Value::from("purchase")),
            ("track", Value::from(track.as_str())),
            ("amount", reforge::amount_value(amount)),
            ("grantId", Value::from(grant_id.as_str())),
        ]);
        if let (Some(map), Some(stored)) = (summary.as_object_mut(), row.as_ref().and_then(|r| r.reforge.as_ref())) {
            map.insert("reforgeId".into(), Value::from(stored.reforge_id.as_str()));
        }
        record(&entry.shared, cap, HostWriteKind::Reforge, &user, &summary).await;
    }
    let mut payload = single_track_payload(&user, &track, Some(&granted.record));
    if let Some(map) = payload.as_object_mut() {
        map.insert("grant_id".into(), Value::from(grant_id));
        map.insert("replayed".into(), Value::Bool(granted.replayed));
    }
    envelope(StatusCode::OK, "OK", payload)
}
