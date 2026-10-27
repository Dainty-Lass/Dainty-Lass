//! `routes/inventory.ts` and the two reforge routes of `routes/progression.ts`, as Express runs them: the body parsers,
//! `HasParadoxBackendAuth`, then the handler.

// An early answer is the whole Response; boxing it would cost every caller for no gain
#![allow(clippy::result_large_err)]

use std::sync::Arc;

use axum::response::Response;
use rifts_seam::{
    AccountId as RiftsAccount, HostWrite, HostWriteKind, HostWriteOptions, InventoryDecision, InventoryFence,
    InventoryTransaction, Json as RiftsJson, Reply, SeamError, TransactionPhase,
};
use serde_json::{Map, Value, json};
use service_kit::{js, jsjson};

use super::body;
use super::items::number_value;
use super::multipliers::{multiply_reward_items, reward_multiplier};
use super::ports::{AuthData, AuthRequest, Authentication, Ports};
use super::reforge::{
    GrantFailure, ReforgeGrantPlan, ResetOutcome, RestoreGrant, grant_banked_restore, is_prestige_gated_reward,
    is_reforgeable_track, plan_purchase_restore, reforge_mode, reforge_reset, single_track_payload,
};
use super::respond;
use super::transaction::{
    Economy, GrantHook, ItemUpdate, TxContext, TxError, TxOptions, TxOutcome, TxRequest, get_inventory,
    run_inventory_transaction, system_actor, update_instanced_item,
};
use super::validation::{
    inventory_body_has_grant_or_spend, validate_instanced_item_update_body, validate_inventory_transaction_body,
};
use super::weapon_level::serve_weapons_at_base_level;
use crate::db::IslesStore;
use crate::hooks::NoPlayerPolicy;
use crate::http::{Call, Shared};
use crate::repo::{AccountId, Repositories, Tx, UserId};
use crate::routes::Params;

/// "INVALID" is UE4/Phoenix's own sentinel for "no logged-in player" (a standalone game server with no client
/// connected yet). Never auto-create, never persist, never write for it.
const NO_PLAYER_SENTINEL: &str = "INVALID";

/// A request after the body parsers and the authentication middleware.
pub struct Prepared {
    pub shared: Arc<Shared>,
    pub params: Params,
    pub body: Option<Value>,
    pub auth: AuthData,
    pub request_id: String,
}

impl Prepared {
    pub fn param(&self, name: &str) -> String {
        self.params.get(name).unwrap_or_default().to_owned()
    }

    fn store(&self) -> Result<&IslesStore, Response> {
        self.shared.store.as_ref().ok_or_else(|| {
            respond::not_implemented(
                "isles: no database is configured (REFORGED_DB_FILE or REFORGED_DB_ENGINE)",
                &self.request_id,
            )
        })
    }
}

/// Express runs its body parsers, then the route's `HasParadoxBackendAuth`.
pub async fn prepare(call: Call, ports: &Ports) -> Result<Prepared, Response> {
    let (parts, parsed) = body::read(call.request).await?;
    let authentication = ports
        .auth
        .authenticate(
            &call.shared,
            AuthRequest {
                method: &parts.method,
                path: parts.uri.path(),
                headers: &parts.headers,
                extensions: &parts.extensions,
                request_id: &call.request_id,
            },
        )
        .await;
    match authentication {
        Authentication::Authenticated(auth) => Ok(Prepared {
            shared: call.shared,
            params: call.params,
            body: parsed,
            auth,
            request_id: call.request_id,
        }),
        Authentication::Answer(response) => Err(response),
    }
}

fn error_response(error: &TxError, request_id: &str) -> Response {
    match error {
        TxError::Repo(error) => respond::repo_error(error, request_id),
        TxError::InsufficientBalance(message) => {
            tracing::warn!("{message}");
            respond::refusal(409, "Insufficient balance")
        }
        TxError::InsufficientPrestige(message) => {
            tracing::warn!("{message}");
            respond::refusal(409, "Insufficient banked prestige progress")
        }
        TxError::InsufficientStackedItem(error) => {
            tracing::warn!("{error}");
            respond::refusal(409, "Insufficient stacked item")
        }
        TxError::Mismatch(_) => respond::refusal(409, "Transaction id reused with a different request body"),
        TxError::Conflict(_) => respond::refusal(409, "Transaction already in progress"),
        TxError::Denied { status, body } => respond::json(*status, body.clone()),
        TxError::Policy(error) => {
            tracing::error!("{error}");
            respond::server_error()
        }
    }
}

fn reply_response(reply: Reply) -> Response {
    respond::raw_json(reply.status, reply.body.0)
}

fn seam_error_response(error: SeamError, request_id: &str) -> Response {
    match error {
        SeamError::Unimplemented(unimplemented) => reply_response(Reply::not_implemented(&unimplemented, request_id)),
        SeamError::Refused(refusal) => respond::json(
            400,
            json!({ "error": { "code": refusal.code, "message": refusal.message, "requestId": request_id } }),
        ),
        other => {
            tracing::error!("rifts: {other}");
            respond::server_error()
        }
    }
}

fn serve(prepared: &Prepared, value: Value) -> Value {
    let enabled = prepared
        .shared
        .version
        .hooks
        .weapons_at_base_level(&prepared.shared.settings);
    serve_weapons_at_base_level(value, enabled)
}

/// `POST /^\/inventory\/([^/]*)\/(.+)$/` and `POST /inventory/:characterId/:changeList`: the migration stub.
pub async fn migration_stub(call: Call, ports: Ports, named: bool) -> Response {
    let prepared = match prepare(call, &ports).await {
        Ok(prepared) => prepared,
        Err(response) => return response,
    };
    if named {
        tracing::info!("Inventory migration (stubbed)");
    } else {
        tracing::info!(
            "Inventory migration (stubbed) characterId='{}' changeList='{}'",
            prepared.param("0"),
            prepared.param("1")
        );
    }
    respond::json(200, json!({ "code": "NONE", "message": "" }))
}

fn inventory_ctx<'a>(prepared: &'a Prepared, store: &'a IslesStore, ports: &'a Ports) -> TxContext<'a> {
    TxContext {
        shared: &prepared.shared,
        store,
        ports,
    }
}

/// `GET /inventory/:userId/:characterId`.
pub async fn get_by_character(call: Call, ports: Ports) -> Response {
    let prepared = match prepare(call, &ports).await {
        Ok(prepared) => prepared,
        Err(response) => return response,
    };
    let user_id = if prepared.auth.is_gameserver {
        Some(prepared.param("userId"))
    } else {
        prepared.auth.user_id.clone()
    };
    let character_id = prepared.param("characterId");
    if user_id.as_deref() == Some(NO_PLAYER_SENTINEL) {
        tracing::debug!(
            "GET /inventory called with no-player sentinel for characterId {character_id} - returning empty inventory"
        );
        return respond::json(
            200,
            json!({ "characterId": character_id, "instancedItems": [], "stackedItems": [] }),
        );
    }
    let store = match prepared.store() {
        Ok(store) => store,
        Err(response) => return response,
    };
    tracing::info!(
        "UserId {} requested inventory for CharacterId {character_id}",
        user_id.as_deref().unwrap_or("undefined")
    );
    match get_inventory(
        &inventory_ctx(&prepared, store, &ports),
        user_id.as_deref(),
        &character_id,
        Some(&prepared.request_id),
    )
    .await
    {
        Ok(Some(inventory)) => respond::json(200, serve(&prepared, inventory)),
        Ok(None) => respond::empty(400),
        Err(error) => error_response(&error, &prepared.request_id),
    }
}

/// `GET /inventory/:userId/`.
pub async fn get_without_character(call: Call, ports: Ports) -> Response {
    let prepared = match prepare(call, &ports).await {
        Ok(prepared) => prepared,
        Err(response) => return response,
    };
    let user_id = if prepared.auth.is_gameserver {
        Some(prepared.param("userId"))
    } else {
        prepared.auth.user_id.clone()
    };
    if user_id.as_deref() == Some(NO_PLAYER_SENTINEL) {
        tracing::debug!("GET /inventory called with no-player sentinel and no characterId - returning empty inventory");
        return respond::json(
            200,
            json!({ "characterId": "INVALID", "instancedItems": [], "stackedItems": [] }),
        );
    }
    // The hub's phantom player fetches inventory without a characterId. Resolve the account's character, the same
    // single-account rule as the INVALID sentinel and the POST path, and return its real inventory.
    if prepared.shared.version.hooks.no_player_policy() == NoPlayerPolicy::ConfiguredAccount
        && prepared.auth.is_gameserver
        && let Some(user) = user_id.as_deref().filter(|user| !user.is_empty())
    {
        let store = match prepared.store() {
            Ok(store) => store,
            Err(response) => return response,
        };
        let ctx = inventory_ctx(&prepared, store, &ports);
        let first = ports
            .characters
            .first_character_id(&prepared.shared, store, &system_actor(&prepared.request_id), user)
            .await;
        match first {
            Err(error) => return respond::repo_error(&error, &prepared.request_id),
            Ok(Some(character_id)) if !character_id.is_empty() => {
                match get_inventory(&ctx, Some(user), &character_id, Some(&prepared.request_id)).await {
                    Ok(Some(inventory)) => {
                        tracing::info!(
                            "GET /inventory (no characterId): gameserver request resolved to '{character_id}' for {user}"
                        );
                        return respond::json(200, serve(&prepared, inventory));
                    }
                    Ok(None) => {}
                    Err(error) => return error_response(&error, &prepared.request_id),
                }
            }
            Ok(_) => {}
        }
    }
    respond::empty(400)
}

/// The fence a realm host's runtime gets for its transaction, shared with the closure that runs inside it.
struct Fence(Arc<dyn InventoryFence>);

/// `options.onGrant` of a module-authorized transaction: the fence sees the request's grant inside the transaction and
/// its refusal answers as it says (`errorResponse`).
fn fence_hook(fence: Arc<dyn InventoryFence>, grant: RiftsJson) -> GrantHook {
    Box::new(move |_: &mut Tx<'_>, _: &Repositories| match fence.on_grant(&grant) {
        Ok(()) => Ok(None),
        Err(reply) => Err(TxError::Denied {
            status: reply.status,
            body: jsjson::parse(&reply.body.0).unwrap_or(Value::Null),
        }),
    })
}

fn json_field<'a>(body: &'a Map<String, Value>, name: &str) -> &'a Value {
    body.get(name).unwrap_or(&Value::Null)
}

/// `x ?? []`.
fn or_empty_array(value: &Value) -> Value {
    if value.is_null() {
        Value::Array(Vec::new())
    } else {
        value.clone()
    }
}

/// `IsChallengeRewardGrant`: a season challenge's coin, at most the table's amount a claim.
fn is_challenge_reward_grant(shared: &Shared, catalog_id: &str, quantity: f64) -> bool {
    shared.version.tables.seasonal_coin.is_some_and(|coin| {
        coin.catalog_id == catalog_id
            && quantity.is_finite()
            && quantity > 0.0
            && quantity <= f64::from(coin.max_per_claim)
    })
}

/// `POST /inventory`.
pub async fn post_transaction(call: Call, ports: Ports) -> Response {
    let mut prepared = match prepare(call, &ports).await {
        Ok(prepared) => prepared,
        Err(response) => return response,
    };
    let hooks = prepared.shared.version.hooks.clone();
    let settings = &prepared.shared.settings;
    let Some(Value::Object(mut body)) = prepared.body.take() else {
        tracing::warn!("POST /inventory rejected: request body is not an object");
        return respond::json(
            400,
            json!({ "code": null, "message": "Invalid inventory request: body must be an object", "payload": {} }),
        );
    };
    let auth = prepared.auth.clone();

    // A gameserver forwarding a client-initiated transaction carries both its key and the client's JWT: prefer the
    // accountId it names, else the JWT's user
    let mut user_id: Option<String> = if auth.is_gameserver {
        match body.get("accountId") {
            None | Some(Value::Null) => auth.user_id.clone(),
            Some(Value::String(id)) => Some(id.clone()),
            Some(_) => None,
        }
    } else {
        auth.user_id.clone()
    };
    let configured = hooks.no_player_policy() == NoPlayerPolicy::ConfiguredAccount;

    // The hub's phantom player posts inventory with an empty accountId and no bearer
    if configured
        && auth.is_gameserver
        && user_id
            .as_deref()
            .is_none_or(|id| id.is_empty() || id == NO_PLAYER_SENTINEL)
        && let Some(configured_user) = hooks.no_player_policy().bearerless_account(settings)
    {
        tracing::info!("POST /inventory: gameserver request with no accountId resolved to '{configured_user}'");
        user_id = Some(configured_user);
    }

    // ...and with an empty characterId, since it never went through character selection
    let mut character_id = body.get("characterId").cloned().unwrap_or(Value::Null);
    let character_missing = !matches!(&character_id, Value::String(id) if !id.is_empty());
    if configured
        && auth.is_gameserver
        && character_missing
        && let Some(user) = user_id
            .as_deref()
            .filter(|id| !id.is_empty() && *id != NO_PLAYER_SENTINEL)
    {
        let store = match prepared.shared.store.as_ref() {
            Some(store) => store,
            None => {
                return respond::not_implemented(
                    "isles: no database is configured (REFORGED_DB_FILE or REFORGED_DB_ENGINE)",
                    &prepared.request_id,
                );
            }
        };
        match ports
            .characters
            .first_character_id(&prepared.shared, store, &system_actor(&prepared.request_id), user)
            .await
        {
            Err(error) => return respond::repo_error(&error, &prepared.request_id),
            Ok(Some(resolved)) if !resolved.is_empty() => {
                tracing::info!(
                    "POST /inventory: gameserver request with no characterId resolved to '{resolved}' for {user}"
                );
                // The body validator reads body.characterId, so the resolution is written back
                body.insert("characterId".into(), Value::String(resolved.clone()));
                character_id = Value::String(resolved);
            }
            Ok(_) => {}
        }
    }

    let transaction_id = json_field(&body, "transactionId").clone();
    let add_instanced = json_field(&body, "addInstancedItems").clone();
    let add_stacked = json_field(&body, "addStackedItems").clone();
    let remove_instanced = json_field(&body, "removeInstancedItems").clone();
    let remove_stacked = json_field(&body, "removeStackedItems").clone();
    let save_instanced = json_field(&body, "saveInstancedItems").clone();

    if user_id.as_deref() == Some(NO_PLAYER_SENTINEL) {
        tracing::debug!(
            "POST /inventory called with no-player sentinel for transactionId {} - no-op transaction",
            js::string(Some(&transaction_id))
        );
        return respond::json(
            200,
            json!({
                "createdInstancedItems": [],
                "updatedInstancedItems": or_empty_array(&save_instanced),
                "updatedStackedItems": or_empty_array(&add_stacked),
                "removedInstancedItems": or_empty_array(&remove_instanced),
            }),
        );
    }

    let body_value = Value::Object(body.clone());
    let user_label = user_id.clone().unwrap_or_else(|| "undefined".into());
    if let Some(error) = validate_inventory_transaction_body(&body_value) {
        tracing::warn!("POST /inventory rejected (bad payload) for userId {user_label}: {error}");
        return respond::json(
            400,
            json!({ "code": null, "message": format!("Invalid inventory request: {error}"), "payload": {} }),
        );
    }
    // Validation passed: both ids are non-empty strings
    let (Some(character_id), Some(transaction_id)) = (
        character_id.as_str().map(str::to_owned),
        transaction_id.as_str().map(str::to_owned),
    ) else {
        return respond::server_error();
    };

    // A runtime authenticated by an optional module is neither a player nor a gameserver: the module authorizes its
    // transaction or rejects it
    let mut fence: Option<Fence> = None;
    if let Some(runtime) = auth.host_runtime.clone() {
        let decision = prepared
            .shared
            .rifts
            .guard()
            .authorize_inventory(
                runtime,
                InventoryTransaction {
                    user: RiftsAccount(user_id.clone().unwrap_or_default()),
                    character_id: character_id.clone(),
                    transaction_id: transaction_id.clone(),
                },
            )
            .await;
        match decision {
            Ok(InventoryDecision::Authorized(authorized)) => fence = Some(Fence(Arc::from(authorized))),
            Ok(InventoryDecision::Refused(reply)) => return reply_response(reply),
            Err(error) => return seam_error_response(error, &prepared.request_id),
        }
    }

    // Authoritative mutations take dedicated gameserver authority or the module-authorized transaction above. A raw
    // player bearer may only save already-owned instances.
    if !auth.is_gameserver && fence.is_none() && inventory_body_has_grant_or_spend(&body_value) {
        tracing::error!(
            "POST /inventory grant/spend rejected: player bearer for userId {user_label} attempted an authoritative mutation without gameserver auth"
        );
        return respond::refusal(403, "Authoritative inventory mutations require gameserver authority");
    }

    let store = match prepared.shared.store.as_ref() {
        Some(store) => store,
        None => {
            return respond::not_implemented(
                "isles: no database is configured (REFORGED_DB_FILE or REFORGED_DB_ENGINE)",
                &prepared.request_id,
            );
        }
    };
    let ctx = inventory_ctx(&prepared, store, &ports);
    let user = user_id.clone().unwrap_or_default();

    // A season challenge's coin reward may arrive ~50 ms before the bounty save that claims the challenge; give that
    // claim a moment to land so it can fund the grant
    let is_dedicated_server = auth.is_gameserver && fence.is_none();
    let funds_challenge_rewards = is_dedicated_server || fence.is_some();
    let shared = prepared.shared.clone();
    let challenge_grant = |catalog_id: &str, quantity: f64| is_challenge_reward_grant(&shared, catalog_id, quantity);
    if funds_challenge_rewards
        && items_of(&add_stacked).iter().any(|item| {
            item.get("catalogId")
                .and_then(Value::as_str)
                .is_some_and(|id| challenge_grant(id, js::number(item.get("quantity"))))
        })
    {
        let account = match ctx.owner_of(&user, &character_id, false).await {
            Ok(account) => account,
            Err(error) => return respond::repo_error(&error, &prepared.request_id),
        };
        if let Some(account) = account
            && !ports.wallet.wait_for_fundable_season_claim(store, &account).await
        {
            tracing::warn!(
                "transactionId {transaction_id}: challenge-reward currency with no claimed, uncredited season challenge after 1.5 s"
            );
        }
    }

    // A private test server's reward multiplier: only a dedicated server's reward transactions, only currencies and
    // materials. The same body always multiplies the same way, so a retried transaction id still matches its ledger row.
    let effective_add_stacked = if is_dedicated_server && add_stacked.is_array() {
        let excluded = |id: &str, quantity: f64| is_prestige_gated_reward(&shared, id) || challenge_grant(id, quantity);
        multiply_reward_items(
            reward_multiplier(settings),
            body.get("source"),
            items_of(&add_stacked),
            &excluded,
        )
        .map(Value::Array)
    } else {
        None
    };
    if let Some(multiplied) = &effective_add_stacked {
        let source = js::string(body.get("source"));
        tracing::info!(
            "[TestMultiplier] transactionId {transaction_id} source=\"{}\" rewards multiplied for userId {user}",
            source.chars().take(80).collect::<String>()
        );
        let _ = multiplied;
    }

    let on_grant: Option<GrantHook> = fence
        .as_ref()
        .map(|Fence(fence)| fence_hook(fence.clone(), RiftsJson(jsjson::stringify(&body_value))));
    if let Some(Fence(fence)) = &fence {
        fence.report(TransactionPhase::Applying);
    }
    let request = TxRequest {
        user_id: user.clone(),
        character_id: character_id.clone(),
        transaction_id: transaction_id.clone(),
        add_instanced,
        add_stacked: effective_add_stacked.unwrap_or(add_stacked),
        remove_instanced,
        remove_stacked,
        save_instanced,
    };
    let options = TxOptions {
        allow_challenge_reward_funding: funds_challenge_rewards,
        economy: Some(Economy {
            source: if is_dedicated_server {
                "INVENTORY_GAMESERVER"
            } else {
                "OTHER"
            }
            .into(),
            p2p_session_id: None,
            store_sku_id: None,
        }),
        on_grant,
        ..TxOptions::default()
    };
    let outcome = run_inventory_transaction(&ctx, request, options, Some(&prepared.request_id)).await;
    match outcome {
        Ok(TxOutcome::Done(result)) => {
            if let Some(Fence(fence)) = &fence {
                fence.report(TransactionPhase::Accepted);
            }
            tracing::info!("Ran transactionId {transaction_id} for userId {user} and characterId {character_id}");
            respond::json(200, serve(&prepared, result))
        }
        Ok(TxOutcome::NotOwned) => {
            if let Some(Fence(fence)) = &fence {
                fence.report(TransactionPhase::Failed);
            }
            tracing::error!("transactionId {transaction_id} for userId {user} and characterId {character_id} FAILED!");
            respond::empty(400)
        }
        Err(error) => error_response(&error, &prepared.request_id),
    }
}

fn items_of(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

/// `POST /inventory/instanceditem`.
pub async fn post_instanced_item(call: Call, ports: Ports) -> Response {
    let prepared = match prepare(call, &ports).await {
        Ok(prepared) => prepared,
        Err(response) => return response,
    };
    let Some(Value::Object(body)) = prepared.body.as_ref() else {
        tracing::warn!("POST /inventory/instanceditem rejected: request body is not an object");
        return respond::json(
            400,
            json!({ "code": null, "message": "Invalid item update: body must be an object", "payload": {} }),
        );
    };
    let auth = &prepared.auth;

    // This endpoint persists item state, so a runtime authenticated by an optional module writes through it only
    // when that module's hostWrites hook accepts it (kind ITEM_SAVE): the item must belong to that player's
    // character (UpdateInstancedItem), and the save is tagged. Without the module the hook refuses.
    let host = auth.host_runtime.clone();
    if let Some(runtime) = &host {
        let target = auth.user_id.clone().filter(|id| !id.is_empty());
        let denial = match &target {
            Some(target) => {
                match prepared
                    .shared
                    .rifts
                    .guard()
                    .authorize_host_write(
                        runtime.clone(),
                        HostWriteKind::ItemSave,
                        RiftsAccount(target.clone()),
                        HostWriteOptions::default(),
                    )
                    .await
                {
                    Ok(denial) => denial.map(reply_response),
                    Err(error) => Some(seam_error_response(error, &prepared.request_id)),
                }
            }
            None => Some(respond::json(
                403,
                json!({ "code": "forbidden", "message": "Player-host runtime cannot mutate inventory.", "payload": null }),
            )),
        };
        if let Some(denial) = denial {
            tracing::warn!("POST /inventory/instanceditem rejected: the host write was refused");
            return denial;
        }
    }

    let character_id = body.get("characterId");
    let user_id: Option<String> = if auth.is_gameserver {
        match body.get("accountId") {
            None | Some(Value::Null) => auth.user_id.clone(),
            Some(Value::String(id)) => Some(id.clone()),
            Some(_) => None,
        }
    } else {
        auth.user_id.clone()
    };
    let instance_id = body.get("instanceId");
    let catalog_id = body.get("catalogId");
    let item_data = body.get("itemData");
    let update_version = body.get("updateVersion");

    if user_id.as_deref() == Some(NO_PLAYER_SENTINEL) {
        tracing::debug!("POST /inventory/instanceditem called with no-player sentinel - no-op update");
        let mut echo = Map::new();
        for (name, value) in [
            ("characterId", character_id),
            ("instanceId", instance_id),
            ("catalogId", catalog_id),
            ("itemData", item_data),
            ("updateVersion", update_version),
        ] {
            if let Some(value) = value {
                echo.insert(name.into(), value.clone());
            }
        }
        return respond::json(200, Value::Object(echo));
    }

    if let Some(error) = validate_instanced_item_update_body(&Value::Object(body.clone())) {
        tracing::warn!(
            "POST /inventory/instanceditem rejected (bad payload) for userId {}: {error}",
            user_id.as_deref().unwrap_or("undefined")
        );
        return respond::json(
            400,
            json!({ "code": null, "message": format!("Invalid item update: {error}"), "payload": {} }),
        );
    }
    let store = match prepared.store() {
        Ok(store) => store,
        Err(response) => return response,
    };
    let (Some(character_id), Some(instance_id), Some(catalog_id)) =
        (character_id.and_then(Value::as_str), instance_id, catalog_id)
    else {
        return respond::server_error();
    };
    let ctx = inventory_ctx(&prepared, store, &ports);
    let item = match update_instanced_item(
        &ctx,
        &ItemUpdate {
            character_id,
            user_id: user_id.as_deref(),
            instance_id,
            catalog_id,
            item_data,
            update_version,
        },
        Some(&prepared.request_id),
    )
    .await
    {
        Ok(item) => item,
        Err(error) => return error_response(&error, &prepared.request_id),
    };
    if let (Some(runtime), Some(_), Some(user)) = (&host, &item, &user_id) {
        let mut summary = Map::new();
        summary.insert(
            "instanceId".into(),
            Value::String(js::string(Some(instance_id)).chars().take(64).collect()),
        );
        summary.insert(
            "catalogId".into(),
            Value::String(js::string(Some(catalog_id)).chars().take(128).collect()),
        );
        if let Some(version) = update_version {
            summary.insert("updateVersion".into(), version.clone());
        }
        let bytes = match item_data {
            Some(Value::String(data)) => data.encode_utf16().count(),
            _ => 0,
        };
        summary.insert("itemDataBytes".into(), Value::from(bytes));
        tag_host_write(
            &prepared,
            runtime,
            HostWriteKind::ItemSave,
            user,
            Value::Object(summary),
        )
        .await;
    }
    match item {
        Some(item) => respond::json(200, serve(&prepared, item)),
        None => respond::json_undefined(),
    }
}

/// `TagHostWrite`: never throws, the write is already stored.
async fn tag_host_write(
    prepared: &Prepared,
    runtime: &rifts_seam::RuntimeAuth,
    kind: HostWriteKind,
    user: &str,
    summary: Value,
) {
    let write = HostWrite {
        kind,
        target: RiftsAccount(user.to_owned()),
        summary: RiftsJson(jsjson::stringify(&summary)),
    };
    if let Err(error) = prepared
        .shared
        .rifts
        .guard()
        .record_host_write(runtime.clone(), write)
        .await
    {
        tracing::error!("[HostWrite] tag failed kind={kind:?} target={user}: {error}");
    }
}

/// The reforge routes admit a dedicated server, or a realm host's runtime whose `hostWrites` hook accepts `REFORGE`.
async fn authorize_reforge(prepared: &Prepared, user: &str) -> Result<bool, Response> {
    if let Some(runtime) = &prepared.auth.host_runtime {
        let denial = prepared
            .shared
            .rifts
            .guard()
            .authorize_host_write(
                runtime.clone(),
                HostWriteKind::Reforge,
                RiftsAccount(user.to_owned()),
                HostWriteOptions::default(),
            )
            .await;
        return match denial {
            Ok(None) => Ok(true),
            Ok(Some(reply)) => Err(reply_response(reply)),
            Err(error) => Err(seam_error_response(error, &prepared.request_id)),
        };
    }
    if !prepared.auth.is_gameserver {
        return Err(respond::refusal(403, "Forbidden"));
    }
    Ok(false)
}

async fn account_of(ctx: &TxContext<'_>, user: &str) -> Result<Option<AccountId>, Response> {
    let repos = ctx.repos();
    let user = UserId(user.to_owned());
    ctx.store
        .database()
        .read(move |conn| repos.accounts.find_by_user_id(conn, &user))
        .await
        .map(|record| record.map(|account| account.id))
        .map_err(|error| respond::repo_error(&error, ""))
}

/// `DELETE /progression/:userId/:progressionId`: the weapon reforge reset (`DeleteProgressionEndpoint`).
pub async fn reforge_delete(call: Call, ports: Ports) -> Response {
    let prepared = match prepare(call, &ports).await {
        Ok(prepared) => prepared,
        Err(response) => return response,
    };
    let user = prepared.param("userId");
    let track = prepared.param("progressionId");
    let is_host = prepared.auth.host_runtime.is_some();
    tracing::info!(
        "[Reforge] DELETE-REQ user={user} track={track} gs={}{}",
        u8::from(prepared.auth.is_gameserver),
        if is_host { " src=host" } else { "" }
    );
    if let Err(response) = authorize_reforge(&prepared, &user).await {
        return response;
    }
    if user == NO_PLAYER_SENTINEL || !is_reforgeable_track(&track) {
        tracing::warn!("[Reforge] REFUSED user={user} track={track}: not a reforgeable weapon track");
        return respond::refusal(409, "Conflict");
    }
    let store = match prepared.store() {
        Ok(store) => store,
        Err(response) => return response,
    };
    let ctx = inventory_ctx(&prepared, store, &ports);
    let account = match account_of(&ctx, &user).await {
        Ok(Some(account)) => account,
        Ok(None) => return respond::refusal(409, "Conflict"),
        Err(response) => return response,
    };
    let result = match reforge_reset(
        &ctx,
        &account,
        &track,
        super::clock::now_ms(),
        Some(&prepared.request_id),
    )
    .await
    {
        Ok(result) => result,
        Err(error) => return error_response(&error, &prepared.request_id),
    };
    // An applied host reset is tagged; a replay of an open reforge belongs to the request that opened it
    if let (Some(runtime), 200, ResetOutcome::Reset, Some(record)) = (
        &prepared.auth.host_runtime,
        result.status,
        result.outcome,
        &result.record,
    ) {
        let reforge = record.reforge.as_ref();
        let mut summary = Map::new();
        summary.insert("route".into(), "delete".into());
        summary.insert("track".into(), Value::String(track.clone()));
        if let Some(reforge) = reforge {
            summary.insert("reforgeId".into(), Value::String(reforge.reforge_id.clone()));
            summary.insert("priorProgress".into(), Value::from(reforge.prior_progress));
            summary.insert("cap".into(), Value::from(reforge.cap));
            summary.insert("banked".into(), Value::from(reforge.banked));
        }
        tag_host_write(
            &prepared,
            runtime,
            HostWriteKind::Reforge,
            &user,
            Value::Object(summary),
        )
        .await;
    }
    let ok = result.status == 200;
    respond::json(
        result.status,
        json!({
            "code": null,
            "message": if ok { "OK" } else { "Conflict" },
            "payload": if ok { single_track_payload(&user, &track, result.record.as_ref()) } else { json!({}) },
        }),
    )
}

/// `POST /progression/purchase/:userId/:progressionId/:amount`: the reforge's banked re-grant
/// (`GrantProgressionPurchaseEndPoint`).
pub async fn reforge_purchase(call: Call, ports: Ports) -> Response {
    let prepared = match prepare(call, &ports).await {
        Ok(prepared) => prepared,
        Err(response) => return response,
    };
    let user = prepared.param("userId");
    let track = prepared.param("progressionId");
    let raw_amount = prepared.param("amount");
    let amount = js::number(Some(&Value::String(raw_amount.clone())));
    let is_host = prepared.auth.host_runtime.is_some();
    tracing::info!(
        "[Reforge] PURCHASE-REQ user={user} track={track} amount={raw_amount} gs={}{}",
        u8::from(prepared.auth.is_gameserver),
        if is_host { " src=host" } else { "" }
    );
    if let Err(response) = authorize_reforge(&prepared, &user).await {
        return response;
    }
    if user == NO_PLAYER_SENTINEL
        || !(amount.fract() == 0.0 && amount.is_finite())
        || amount <= 0.0
        || amount > super::reforge::MAX_GRANT_AMOUNT
    {
        return respond::refusal(400, "Bad Request");
    }
    let store = match prepared.store() {
        Ok(store) => store,
        Err(response) => return response,
    };
    let ctx = inventory_ctx(&prepared, store, &ports);
    let account = match account_of(&ctx, &user).await {
        Ok(account) => account,
        Err(response) => return response,
    };
    let repos = ctx.repos();
    let row = match &account {
        Some(account) => {
            let (repos, account, track) = (repos.clone(), account.clone(), track.clone());
            match store
                .database()
                .read(move |conn| repos.progression_tracks.get(conn, &account, &track))
                .await
            {
                Ok(row) => row,
                Err(error) => return respond::repo_error(&error, &prepared.request_id),
            }
        }
        None => None,
    };
    let refuse = |reason: &str| {
        tracing::warn!("[Reforge] DUP/REFUSED purchase user={user} track={track} amount={raw_amount}: {reason}");
        respond::refusal(409, "Conflict")
    };
    let purchase = plan_purchase_restore(
        &track,
        amount,
        row.as_ref(),
        super::clock::now_ms(),
        reforge_mode(&prepared.shared),
    );
    let (ReforgeGrantPlan::Normal | ReforgeGrantPlan::Restore { .. }, Some(grant_id), Some(account)) =
        (&purchase.plan, &purchase.grant_id, &account)
    else {
        return refuse(match &purchase.plan {
            ReforgeGrantPlan::Refuse(reason) => reason,
            _ => "no grant id",
        });
    };
    if matches!(purchase.plan, ReforgeGrantPlan::Normal) {
        // A request that matches a reforge already restored replays only if the ledger holds that grant; a stale
        // reforge finished on arrival has no ledger row, and paying it again would double the banked XP
        let (repos, grant) = (repos.clone(), grant_id.clone());
        let known = store
            .database()
            .read(move |conn| repos.progression_grants.find_by_grant_id(conn, &grant))
            .await;
        match known {
            Ok(Some(_)) => {}
            Ok(None) => return refuse("the banked XP was already restored (stale-reforge recovery)"),
            Err(error) => return respond::repo_error(&error, &prepared.request_id),
        }
    }
    let granted = grant_banked_restore(
        &ctx,
        RestoreGrant {
            account,
            user_id: &user,
            progression_id: &track,
            amount: amount as i64,
            grant_id,
            plan: purchase.plan.clone(),
        },
        Some(&prepared.request_id),
    )
    .await;
    let granted = match granted {
        Ok(granted) => granted,
        Err(GrantFailure::Reforge(conflict)) => return refuse(&conflict.0),
        Err(error @ GrantFailure::GrantConflict(_)) => return refuse(&error.to_string()),
        Err(GrantFailure::Tx(error)) => return error_response(&error, &prepared.request_id),
    };
    if let (Some(runtime), false) = (&prepared.auth.host_runtime, granted.replayed) {
        let mut summary = Map::new();
        summary.insert("route".into(), "purchase".into());
        summary.insert("track".into(), Value::String(track.clone()));
        summary.insert("amount".into(), number_value(amount));
        summary.insert("grantId".into(), Value::String(grant_id.clone()));
        if let Some(reforge) = row.as_ref().and_then(|r| r.reforge.as_ref()) {
            summary.insert("reforgeId".into(), Value::String(reforge.reforge_id.clone()));
        }
        tag_host_write(
            &prepared,
            runtime,
            HostWriteKind::Reforge,
            &user,
            Value::Object(summary),
        )
        .await;
    }
    let updated = &granted.record;
    respond::json(
        200,
        json!({
            "code": null,
            "message": "OK",
            "payload": {
                "phx_account_id": user,
                "progression_id": track,
                "progress": updated.progress,
                "confirmed_fremium_rank": updated.confirmed_fremium_rank,
                "confirmed_premium_rank": updated.confirmed_premium_rank,
                "confirmed_date": updated.updated_at,
                "grant_id": grant_id,
                "replayed": granted.replayed,
            }
        }),
    )
}
