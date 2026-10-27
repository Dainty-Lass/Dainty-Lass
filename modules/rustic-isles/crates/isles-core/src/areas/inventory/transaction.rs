//! `controllers/inventory.ts`: the inventory transaction, the single-item update and the read, on `store`.
//!
//! `RunInventoryTransaction` is one `Database::write` closure: the ledger row, the wallet, the optional grant fence,
//! the revision-guarded inventory write and the ledger completion commit together or not at all (Node's unit of
//! work). Other areas call it too: the store's purchases pass an `on_grant` for the entitlements and progression a
//! SKU also pays, and the admin tester tool sets `operator_test_funding`.

use std::sync::{Arc, Mutex};

use module_api::{Actor, ActorContext};
use serde_json::{Map, Value};
use service_kit::{js, jsjson};

use super::clock::{iso_now, now_ms};
use super::items::{self, Delta, InsufficientStackedItem, apply_item_data_save, compute_inventory_request_hash};
use super::ports::{CommittedTransaction, CurrencyChange, Ports, WalletError};
use super::storage_policy::{
    PolicyError, StoragePolicy, migrate_inventory_storage_arrays, normalize_inventory_grant_collections,
};
use crate::db::IslesStore;
use crate::http::Shared;
use crate::repo::inventory::{InventoryRecord, LedgerKey, LedgerStatus, TryBegin};
use crate::repo::{AccountId, RepoError, RepoResult, Repositories, Tx, UserId};

const POLICY_FILE: &str = "inventory_storage_policy.json";

/// What stops a transaction, and how the route answers it.
#[derive(Debug, thiserror::Error)]
pub enum TxError {
    /// 409 "Insufficient balance"
    #[error("{0}")]
    InsufficientBalance(String),
    /// 409 "Insufficient banked prestige progress"
    #[error("{0}")]
    InsufficientPrestige(String),
    /// 409 "Insufficient stacked item"
    #[error(transparent)]
    InsufficientStackedItem(#[from] InsufficientStackedItem),
    /// 409 "Transaction id reused with a different request body"
    #[error("Inventory transaction {0} was reused with a different request body")]
    Mismatch(String),
    /// 409 "Transaction already in progress"
    #[error("Inventory transaction {0} is already in progress (concurrent duplicate request)")]
    Conflict(String),
    /// A module's own refusal (the grant fence), answered as it says
    #[error("denied ({status})")]
    Denied { status: u16, body: Value },
    /// An error Node lets reach Express's final handler (500)
    #[error(transparent)]
    Policy(#[from] PolicyError),
    #[error(transparent)]
    Repo(#[from] RepoError),
}

impl From<WalletError> for TxError {
    fn from(error: WalletError) -> Self {
        match error {
            WalletError::InsufficientBalance(message) => Self::InsufficientBalance(message),
            WalletError::InsufficientPrestige(message) => Self::InsufficientPrestige(message),
            WalletError::Repo(error) => Self::Repo(error),
        }
    }
}

/// Where a transaction came from, for the economy ledger (`EconomyScope`). Monitoring only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Economy {
    pub source: String,
    pub p2p_session_id: Option<String>,
    pub store_sku_id: Option<String>,
}

/// Fields an `onGrant` annotator stores on the ledger row, from the item adds that survived normalisation.
pub type Annotator = Box<dyn FnOnce(&[Value], &[Value]) -> Option<Value> + Send>;

/// `options.onGrant`: runs inside the transaction after the wallet step and before the inventory write, only for a
/// new transaction id (a replay never re-enters it). An `Err` rolls the whole transaction back.
pub type GrantHook = Box<dyn FnOnce(&mut Tx<'_>, &Repositories) -> Result<Option<Annotator>, TxError> + Send>;

#[derive(Default)]
pub struct TxOptions {
    /// The caller already bound the user to the character through something server-authenticated (the signed store token)
    pub ownership_already_validated: bool,
    pub diagnostic_label: Option<String>,
    pub on_grant: Option<GrantHook>,
    /// A season challenge's claim may fund a `SEASONAL_COIN` grant (`POST /inventory` from a dedicated server only)
    pub allow_challenge_reward_funding: bool,
    /// The admin tester tool credits a prestige-gated currency without banked prestige
    pub operator_test_funding: bool,
    pub economy: Option<Economy>,
}

/// `RunInventoryTransaction`'s arguments: the request's collections as the client sent them (`Null` when absent).
#[derive(Debug, Clone)]
pub struct TxRequest {
    pub user_id: String,
    pub character_id: String,
    pub transaction_id: String,
    pub add_instanced: Value,
    pub add_stacked: Value,
    pub remove_instanced: Value,
    pub remove_stacked: Value,
    pub save_instanced: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TxOutcome {
    /// Node's `false`: the character isn't the user's
    NotOwned,
    /// The result to answer: the new one, or the stored one of a replay
    Done(Value),
}

/// What the functions here need to reach the database and the other areas.
pub struct TxContext<'a> {
    pub shared: &'a Arc<Shared>,
    pub store: &'a IslesStore,
    pub ports: &'a Ports,
}

fn list(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

impl TxContext<'_> {
    pub fn repos(&self) -> Repositories {
        self.ports.repos.clone().unwrap_or_else(|| self.store.repos().clone())
    }

    fn policy(&self) -> Result<Arc<StoragePolicy>, PolicyError> {
        let path =
            self.shared.game_data.backend_file(POLICY_FILE).ok_or_else(|| {
                PolicyError::GameData(format!("this module has no game data folder for {POLICY_FILE}"))
            })?;
        StoragePolicy::load(&path)
    }

    /// `DoesInventoryBelongToUserId`: the user's account, when it has the character.
    pub async fn owner_of(
        &self,
        user_id: &str,
        character_id: &str,
        check_character: bool,
    ) -> RepoResult<Option<AccountId>> {
        let repos = self.repos();
        let user = UserId(user_id.to_owned());
        let character_id = character_id.to_owned();
        self.store
            .database()
            .read(move |conn| -> RepoResult<Option<AccountId>> {
                let Some(account) = repos.accounts.find_by_user_id(conn, &user)? else {
                    return Ok(None);
                };
                if check_character && repos.characters.find(conn, &character_id, &account.id)?.is_none() {
                    return Ok(None);
                }
                Ok(Some(account.id))
            })
            .await
    }
}

pub fn player_actor(account: &AccountId, request_id: Option<&str>) -> ActorContext {
    let mut actor = ActorContext::new(Actor::Player {
        account: account.global().clone(),
    });
    actor.request = request_id.map(str::to_owned);
    actor
}

fn rolled_back(slot: &Mutex<Option<TxError>>, error: impl Into<TxError>) -> RepoError {
    if let Ok(mut slot) = slot.lock() {
        slot.get_or_insert(error.into());
    }
    RepoError::Data("the transaction was rolled back".into())
}

enum Inner {
    Existing(Box<crate::repo::inventory::InventoryTransactionRecord>),
    Done(Value, Box<CommittedTransaction>),
}

fn parse_array(text: &str, what: &str) -> RepoResult<Vec<Value>> {
    match jsjson::parse(text) {
        Ok(Value::Array(items)) => Ok(items),
        Ok(_) => Err(RepoError::Data(format!("inventory {what} are not an array"))),
        Err(error) => Err(RepoError::Data(format!("inventory {what}: {error}"))),
    }
}

fn is_currency_entry(item: &Value) -> bool {
    items::is_wallet_currency_id(item.get("catalogId"))
        && js::safe_integer(item.get("quantity")).is_some()
        && js::number(item.get("quantity")) > 0.0
}

fn currency_entries(list: &[Value]) -> Vec<Value> {
    list.iter()
        .filter(|item| is_currency_entry(item))
        .map(|item| {
            let mut entry = Map::new();
            entry.insert("catalogId".into(), item["catalogId"].clone());
            entry.insert("amount".into(), item["quantity"].clone());
            Value::Object(entry)
        })
        .collect()
}

/// `RunInventoryTransaction`.
pub async fn run_inventory_transaction(
    ctx: &TxContext<'_>,
    request: TxRequest,
    mut options: TxOptions,
    request_id: Option<&str>,
) -> Result<TxOutcome, TxError> {
    let Some(account) = ctx
        .owner_of(
            &request.user_id,
            &request.character_id,
            !options.ownership_already_validated,
        )
        .await?
    else {
        tracing::error!(
            "Specified characterId {} does not belong to user {}",
            request.character_id,
            request.user_id
        );
        return Ok(TxOutcome::NotOwned);
    };

    // The hash of the wire body, before the storage correction, so old ledger rows still match
    let request_hash = compute_inventory_request_hash(
        &request.add_instanced,
        &request.add_stacked,
        &request.remove_instanced,
        &request.remove_stacked,
        &request.save_instanced,
    );
    let policy = ctx.policy()?;
    let grants = normalize_inventory_grant_collections(
        &policy,
        &request.transaction_id,
        list(&request.add_instanced),
        list(&request.add_stacked),
    )?;
    if !grants.corrections.is_empty() {
        let list = grants
            .corrections
            .iter()
            .map(|c| format!("{}:{}->{}x{}", c.catalog_id, c.from, c.to, c.quantity))
            .collect::<Vec<_>>()
            .join(", ");
        tracing::warn!(
            "[InventoryStorage] transactionId {} normalized {} entr{}: {list}",
            request.transaction_id,
            grants.corrections.len(),
            if grants.corrections.len() == 1 { "y" } else { "ies" }
        );
    }

    let economy = options.economy.clone().unwrap_or(Economy {
        source: "OTHER".into(),
        p2p_session_id: None,
        store_sku_id: None,
    });
    let committed_base = CommittedTransaction {
        account: account.clone(),
        user_id: request.user_id.clone(),
        transaction_id: request.transaction_id.clone(),
        source: economy.source.clone(),
        granted: grants
            .instanced_items_to_add
            .iter()
            .map(|item| (item.get("catalogId"), Value::from(1)))
            .chain(
                grants
                    .stacked_items_to_add
                    .iter()
                    .map(|item| (item.get("catalogId"), item["quantity"].clone())),
            )
            .filter(|(catalog_id, quantity)| {
                matches!(catalog_id, Some(Value::String(id)) if !id.starts_with("CURRENCY_"))
                    && js::safe_integer(Some(quantity)).is_some()
            })
            .map(|(catalog_id, quantity)| {
                let mut entry = Map::new();
                entry.insert("catalogId".into(), catalog_id.cloned().unwrap_or(Value::Null));
                entry.insert("quantity".into(), quantity);
                Value::Object(entry)
            })
            .collect(),
        currency_credits: currency_entries(&grants.stacked_items_to_add),
        currency_debits: currency_entries(list(&request.remove_stacked)),
        result: Value::Null,
    };

    let repos = ctx.repos();
    let wallet = ctx.ports.wallet.clone();
    let starter = ctx.ports.starter.clone();
    let shared = ctx.shared.clone();
    let slot: Arc<Mutex<Option<TxError>>> = Arc::default();
    let slot_in = slot.clone();
    let started = now_ms();
    let on_grant = options.on_grant.take();
    let allow_funding = options.allow_challenge_reward_funding;
    let operator_test = options.operator_test_funding;
    let economy_in = economy.clone();
    let account_in = account.clone();
    let req = request.clone();
    let hash = request_hash.clone();
    let actor = player_actor(&account, request_id);

    let outcome = ctx
        .store
        .account_write(&account, &actor, move |tx, _| -> RepoResult<Inner> {
            let key = LedgerKey {
                account: &account_in,
                character_id: &req.character_id,
                transaction_id: &req.transaction_id,
            };
            match repos.inventory_transactions.try_begin(tx, key, &hash, &iso_now())? {
                TryBegin::Existing(existing) => return Ok(Inner::Existing(existing)),
                TryBegin::Begun => {}
            }

            // Currency first, in the same transaction: an overspend or an unbacked gated reward aborts it all
            let touched = wallet
                .apply_currency_deltas(
                    tx,
                    &repos,
                    &CurrencyChange {
                        account: &account_in,
                        user_id: &req.user_id,
                        adds: &grants.stacked_items_to_add,
                        removes: list(&req.remove_stacked),
                        transaction_id: &req.transaction_id,
                        allow_challenge_reward_funding: allow_funding,
                        operator_test_funding: operator_test,
                    },
                )
                .map_err(|e| rolled_back(&slot_in, e))?;

            let annotator = match on_grant {
                Some(hook) => hook(tx, &repos).map_err(|e| rolled_back(&slot_in, e))?,
                None => None,
            };

            let current = match repos.inventories.find_by_character_id(tx.conn(), &req.character_id)? {
                Some(inventory) => inventory,
                None => {
                    tracing::info!("Recovering bootstrap records for characterId {}", req.character_id);
                    starter
                        .ensure(tx, &repos, &shared, &account_in, &req.user_id, &req.character_id)?
                        .map_err(|e| RepoError::Data(e.to_string()))?
                }
            };
            let expected_revision = current.revision.unwrap_or(0);
            let instanced = parse_array(&current.instanced_items, "instancedItems")?;
            let stacked = parse_array(&current.stacked_items, "stackedItems")?;

            let applied = items::apply_transaction(
                &req.user_id,
                instanced,
                stacked,
                &Delta {
                    add_instanced: &grants.instanced_items_to_add,
                    add_stacked: &grants.stacked_items_to_add,
                    remove_instanced: list(&req.remove_instanced),
                    remove_stacked: list(&req.remove_stacked),
                    save_instanced: list(&req.save_instanced),
                },
                &touched,
                &mut items::mint_instance_id,
                &req.transaction_id,
            )
            .map_err(|e| rolled_back(&slot_in, e))?;

            let updated = repos.inventories.update_both_if_revision_matches(
                tx,
                &req.character_id,
                &jsjson::stringify(&Value::Array(applied.instanced.clone())),
                &jsjson::stringify(&Value::Array(applied.stacked.clone())),
                expected_revision,
            )?;
            if updated.is_none() {
                tracing::warn!(
                    "transactionId {} for userId {} hit a revision conflict on characterId {} - another write landed first, caller should retry",
                    req.transaction_id,
                    req.user_id,
                    req.character_id
                );
                return Err(rolled_back(&slot_in, TxError::Conflict(req.transaction_id.clone())));
            }

            let mut result = Map::new();
            result.insert("createdInstancedItems".into(), Value::Array(applied.created_instanced.clone()));
            result.insert("updatedInstancedItems".into(), Value::Array(applied.updated_instanced));
            result.insert("updatedStackedItems".into(), Value::Array(applied.updated_stacked));
            result.insert(
                "removedInstancedItems".into(),
                if req.remove_instanced.is_null() {
                    Value::Array(Vec::new())
                } else {
                    req.remove_instanced.clone()
                },
            );
            let result = Value::Object(result);

            // The annotator sees only what survived normalisation and persistence; its fields ride the completion write
            let mut extra = Map::new();
            extra.insert("economySource".into(), Value::String(economy_in.source.clone()));
            if let Some(session) = &economy_in.p2p_session_id {
                extra.insert("economySessionId".into(), Value::String(session.clone()));
            }
            if let Some(Value::Object(fields)) = annotator.and_then(|f| f(&applied.created_instanced, &applied.accepted_stacked_adds)) {
                extra.extend(fields);
            }
            repos.inventory_transactions.complete(tx, key, &result, Some(&Value::Object(extra)), &iso_now())?;
            Ok(Inner::Done(result.clone(), Box::new(CommittedTransaction { result, ..committed_base })))
        })
        .await;

    let inner = match outcome {
        Ok(inner) => inner,
        Err(error) => {
            let failure = slot.lock().ok().and_then(|mut slot| slot.take());
            return Err(failure.unwrap_or(TxError::Repo(error)));
        }
    };
    match inner {
        Inner::Done(result, committed) => {
            ctx.ports.observer.committed(&committed);
            let total = now_ms() - started;
            if options.diagnostic_label.is_some() || total >= 1000 {
                tracing::info!(
                    "[InventoryTiming] label={} total={total}ms",
                    options.diagnostic_label.as_deref().unwrap_or("inventory")
                );
            }
            Ok(TxOutcome::Done(result))
        }
        Inner::Existing(existing) => {
            // Binding the id to the body: the same id with another body is misuse, not a retry
            match &existing.request_hash {
                Some(stored) if *stored != request_hash => {
                    tracing::error!(
                        "transactionId {} for userId {} reused with a different request body - rejecting (stored {} != incoming {})",
                        request.transaction_id,
                        request.user_id,
                        &stored[..stored.len().min(12)],
                        &request_hash[..12]
                    );
                    return Err(TxError::Mismatch(request.transaction_id));
                }
                None => tracing::warn!(
                    "transactionId {} for userId {} is a legacy ledger record with no requestHash - replaying on original (pre-body-binding) semantics",
                    request.transaction_id,
                    request.user_id
                ),
                Some(_) => {}
            }
            if existing.status == LedgerStatus::Completed {
                tracing::info!(
                    "[Inventory] Replayed transactionId {} for userId {} - returning original stored result, not re-applying",
                    request.transaction_id,
                    request.user_id
                );
                return Ok(TxOutcome::Done(
                    existing.result.map_or(Value::Null, |r| jsjson::to_js(&r)),
                ));
            }
            Err(TxError::Conflict(request.transaction_id))
        }
    }
}

/// `GetInventoryForUserIdAndCharacterId`: the inventory with the wallet over its currencies, after the starter
/// bootstrap and the storage-policy repair. `None` when the character isn't the user's.
pub async fn get_inventory(
    ctx: &TxContext<'_>,
    user_id: Option<&str>,
    character_id: &str,
    request_id: Option<&str>,
) -> Result<Option<Value>, TxError> {
    let Some(user_id) = user_id else {
        return Ok(None);
    };
    let Some(account) = ctx.owner_of(user_id, character_id, true).await? else {
        return Ok(None);
    };
    let policy = ctx.policy()?;
    let repos = ctx.repos();
    let wallet = ctx.ports.wallet.clone();
    let starter = ctx.ports.starter.clone();
    let shared = ctx.shared.clone();
    let user = user_id.to_owned();
    let character = character_id.to_owned();
    let account_in = account.clone();
    let actor = player_actor(&account, request_id);
    let value = ctx
        .store
        .account_write(&account, &actor, move |tx, _| -> RepoResult<Value> {
            // Recovery creates an absent inventory and loadout; with both present it only appends missing starter
            // parts, once, so repeated reads can't reset progression or the loadout
            let mut inventory = starter
                .ensure(tx, &repos, &shared, &account_in, &user, &character)?
                .map_err(|e| RepoError::Data(e.to_string()))?;
            let mut instanced = parse_array(&inventory.instanced_items, "instancedItems")?;
            let mut stacked = parse_array(&inventory.stacked_items, "stackedItems")?;
            let migration = migrate_inventory_storage_arrays(&policy, &instanced, &stacked);
            if !migration.changes.is_empty() {
                let updated = repos
                    .inventories
                    .update_both_if_revision_matches(
                        tx,
                        &character,
                        &jsjson::stringify(&Value::Array(migration.instanced_items.clone())),
                        &jsjson::stringify(&Value::Array(migration.stacked_items.clone())),
                        inventory.revision.unwrap_or(0),
                    )?
                    .ok_or_else(|| RepoError::Data(format!("Inventory storage reconciliation for characterId {character} failed")))?;
                let summary = migration
                    .changes
                    .iter()
                    .map(|c| format!("{}:instanced->stacked x{} ({})", c.catalog_id, c.migrated_instances, c.result))
                    .collect::<Vec<_>>()
                    .join(", ");
                tracing::warn!("[InventoryStorage] repaired legacy storage for userId {user} characterId {character}: {summary}");
                if !migration.issues.is_empty() {
                    tracing::warn!(
                        "[InventoryStorage] characterId {character} still has {} stacked non-stackable entries requiring manual review",
                        migration.issues.len()
                    );
                }
                inventory = updated;
                instanced = parse_array(&inventory.instanced_items, "instancedItems")?;
                stacked = parse_array(&inventory.stacked_items, "stackedItems")?;
            }
            let stacked = wallet.merge_into_stacked(tx, &repos, &account_in, &user, stacked)?;
            let mut out = Map::new();
            out.insert("characterId".into(), Value::String(character.clone()));
            out.insert("instancedItems".into(), Value::Array(instanced));
            out.insert("stackedItems".into(), Value::Array(stacked));
            Ok(Value::Object(out))
        })
        .await?;
    Ok(Some(value))
}

/// `UpdateInstancedItem`'s arguments, as the body carried them (`None` is a missing key).
#[derive(Clone, Copy)]
pub struct ItemUpdate<'a> {
    pub character_id: &'a str,
    pub user_id: Option<&'a str>,
    pub instance_id: &'a Value,
    pub catalog_id: &'a Value,
    pub item_data: Option<&'a Value>,
    pub update_version: Option<&'a Value>,
}

/// `UpdateInstancedItem`: persists one owned item's data. `None` is Node's `undefined`: not the user's character, no
/// such item, a catalogId change, or retries exhausted.
pub async fn update_instanced_item(
    ctx: &TxContext<'_>,
    update: &ItemUpdate<'_>,
    request_id: Option<&str>,
) -> Result<Option<Value>, TxError> {
    let ItemUpdate {
        character_id,
        user_id,
        instance_id,
        catalog_id,
        item_data,
        update_version,
    } = *update;
    let Some(user_id) = user_id else {
        tracing::error!("UpdateInstancedItem: characterId {character_id} does not belong to user undefined");
        return Ok(None);
    };
    let Some(account) = ctx.owner_of(user_id, character_id, true).await? else {
        tracing::error!("UpdateInstancedItem: characterId {character_id} does not belong to user {user_id}");
        return Ok(None);
    };
    let repos = ctx.repos();
    let actor = player_actor(&account, request_id);
    for attempt in 0..5u64 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(5 + (now_ms() as u64 % 20))).await;
        }
        let repos = repos.clone();
        let character = character_id.to_owned();
        let (instance_id, catalog_id) = (instance_id.clone(), catalog_id.clone());
        let (item_data, update_version) = (item_data.cloned(), update_version.cloned());
        let user = user_id.to_owned();
        let step = ctx
            .store
            .account_write(&account, &actor, move |tx, _| -> RepoResult<Step> {
                let Some(current) = repos.inventories.find_by_character_id(tx.conn(), &character)? else {
                    tracing::error!("UpdateInstancedItem: no inventory for characterId {character}");
                    return Ok(Step::Done(None));
                };
                let mut instanced = parse_array(&current.instanced_items, "instancedItems")?;
                let Some(index) = instanced
                    .iter()
                    .position(|item| js::strict_eq(item.get("instanceId"), Some(&instance_id)))
                else {
                    tracing::warn!(
                        "UpdateInstancedItem: instanceId {} not found for characterId {character}",
                        js::string(Some(&instance_id))
                    );
                    return Ok(Step::Done(None));
                };
                let existing = instanced[index].clone();
                // Identity is immutable here: refusing a catalogId change is refusing a transmutation
                if !js::strict_eq(existing.get("catalogId"), Some(&catalog_id)) {
                    tracing::warn!(
                        "UpdateInstancedItem: refusing catalogId change {} -> {} for instanceId {} (userId {user})",
                        js::string(existing.get("catalogId")),
                        js::string(Some(&catalog_id)),
                        js::string(Some(&instance_id))
                    );
                    return Ok(Step::Done(None));
                }
                let Some(next) = apply_item_data_save(&existing, item_data.as_ref(), update_version.as_ref()) else {
                    return Ok(Step::Done(Some(existing)));
                };
                instanced[index] = next.clone();
                let updated = repos.inventories.update_both_if_revision_matches(
                    tx,
                    &character,
                    &jsjson::stringify(&Value::Array(instanced)),
                    &current.stacked_items,
                    current.revision.unwrap_or(0),
                )?;
                if updated.is_some() {
                    tracing::info!(
                        "Updated instanced item {} for characterId {character} and userId {user}",
                        js::string(Some(&catalog_id))
                    );
                    return Ok(Step::Done(Some(next)));
                }
                Ok(Step::Retry)
            })
            .await?;
        match step {
            Step::Done(item) => return Ok(item),
            Step::Retry => tracing::warn!(
                "UpdateInstancedItem: revision conflict for characterId {character_id}, attempt {}/5 - retrying",
                attempt + 1
            ),
        }
    }
    tracing::error!("UpdateInstancedItem for characterId {character_id} failed after 5 revision-conflict retries");
    Ok(None)
}

enum Step {
    Done(Option<Value>),
    Retry,
}

/// The record as the repository holds it, for callers that seed an inventory.
pub fn inventory_record(
    character_id: &str,
    account: &AccountId,
    instanced: &[Value],
    stacked: &[Value],
) -> InventoryRecord {
    InventoryRecord {
        character_id: character_id.to_owned(),
        account: Some(account.clone()),
        instanced_items: jsjson::stringify(&Value::Array(instanced.to_vec())),
        stacked_items: jsjson::stringify(&Value::Array(stacked.to_vec())),
        revision: Some(0),
        bootstrap_version: None,
    }
}
