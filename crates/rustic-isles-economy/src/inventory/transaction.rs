// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The inventory transaction and its idempotency ledger, the inventory read and the single item save (spec §6).

use std::sync::Arc;
use std::time::Duration;

use module_api::{Actor, ActorContext};
use rustic_isles::Shared;
use rustic_isles_store::repo::inventory::{InventoryRecord, LedgerKey, LedgerStatus, TryBegin};
use rustic_isles_store::{AccountId, IslesStore, RepoError, RepoResult, Repositories, Tx, UserId};
use serde_json::{Map, Value};
use service_kit::{js, jsjson};

use crate::inventory::items::{self, Delta};
use crate::inventory::ports::{CurrencyChange, InventoryPorts, WalletError};
use crate::inventory::prestige::ProgressionConfig;
use crate::inventory::storage_policy::{
    self, PolicyError, StoragePolicy, migrate_inventory_storage_arrays, normalize_inventory_grant_collections,
};
use crate::tx::guarded;
use crate::util::{now_iso, obj};

pub fn player_actor(account: &AccountId, request_id: Option<&str>) -> ActorContext {
    let actor = ActorContext::new(Actor::Player {
        account: account.global().clone(),
    });
    match request_id {
        Some(id) => actor.with_request(id),
        None => actor,
    }
}

pub fn system_actor(request_id: Option<&str>) -> ActorContext {
    let actor = ActorContext::new(Actor::System);
    match request_id {
        Some(id) => actor.with_request(id),
        None => actor,
    }
}

/// A new record at revision 0 (seeding callers).
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

/// What a transaction call runs against: the module, its store and the ports.
#[derive(Clone, Copy)]
pub struct TxContext<'a> {
    pub shared: &'a Arc<Shared>,
    pub store: &'a IslesStore,
    pub ports: &'a InventoryPorts,
}

impl<'a> TxContext<'a> {
    pub fn new(shared: &'a Arc<Shared>, store: &'a IslesStore, ports: &'a InventoryPorts) -> Self {
        Self { shared, store, ports }
    }

    pub fn repos(&self) -> Repositories {
        self.ports.repos.clone().unwrap_or_else(|| self.store.repos().clone())
    }

    /// The store with the ports' repositories, when the ports override them.
    pub(crate) fn effective_store(&self) -> IslesStore {
        match &self.ports.repos {
            Some(repos) => self.store.clone().with_repos(repos.clone()),
            None => self.store.clone(),
        }
    }

    /// The live account of `user`, and (unless told not to) its live character `character` (§6.1 step 1).
    pub async fn owner_of(&self, user: &str, character: &str, check_character: bool) -> RepoResult<Option<AccountId>> {
        let user = UserId::new(user);
        let character = character.to_owned();
        self.effective_store()
            .read(move |conn, repos| {
                let Some(account) = repos.accounts.find_by_user_id(conn, &user)? else {
                    return Ok(None);
                };
                if check_character && repos.characters.find(conn, &character, &account.id)?.is_none() {
                    return Ok(None);
                }
                Ok(Some(account.id))
            })
            .await
    }
}

#[derive(Debug, Clone, PartialEq)]
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

/// Monitoring only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Economy {
    pub source: String,
    pub p2p_session_id: Option<String>,
    pub store_sku_id: Option<String>,
}

/// Given the created instanced items and the accepted stacked adds: optional fields for the ledger row.
pub type Annotator = Box<dyn FnOnce(&[Value], &[Value]) -> Option<Map<String, Value>> + Send>;

/// Runs inside the transaction for a new transaction id; may refuse (rolling everything back) or annotate.
pub type GrantHook = Box<dyn FnOnce(&mut Tx<'_>, &Repositories) -> Result<Option<Annotator>, TxError> + Send>;

#[derive(Default)]
pub struct TxOptions {
    pub ownership_already_validated: bool,
    pub diagnostic_label: Option<String>,
    pub on_grant: Option<GrantHook>,
    pub allow_challenge_reward_funding: bool,
    pub operator_test_funding: bool,
    pub economy: Option<Economy>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TxOutcome {
    NotOwned,
    Done(Value),
}

#[derive(Debug, thiserror::Error)]
pub enum TxError {
    #[error("{0}")]
    InsufficientBalance(String),
    #[error("{0}")]
    InsufficientPrestige(String),
    #[error(
        "Insufficient stacked item: user {user} has {} of {catalog_id}, cannot remove {}",
        jsjson::number(*owned),
        jsjson::number(*requested)
    )]
    InsufficientStackedItem {
        user: String,
        catalog_id: String,
        owned: f64,
        requested: f64,
    },
    #[error("{0}")]
    Mismatch(String),
    #[error("{0}")]
    Conflict(String),
    #[error("denied ({status})")]
    Denied { status: u16, body: Value },
    #[error(transparent)]
    Policy(#[from] PolicyError),
    #[error(transparent)]
    Repo(#[from] RepoError),
}

impl From<WalletError> for TxError {
    fn from(error: WalletError) -> Self {
        match error {
            WalletError::InsufficientBalance(text) => Self::InsufficientBalance(text),
            WalletError::InsufficientPrestige(text) => Self::InsufficientPrestige(text),
            WalletError::Repo(e) => Self::Repo(e),
        }
    }
}

/// What the observer is told after a commit (§6.1).
#[derive(Debug, Clone, PartialEq)]
pub struct CommittedTransaction {
    pub account: AccountId,
    pub user_id: String,
    pub transaction_id: String,
    pub source: String,
    /// `(catalogId, quantity)`: instanced adds as 1 each, then stacked adds; no currency
    pub granted: Vec<(String, f64)>,
    /// `(catalogId, amount)`
    pub currency_credits: Vec<(String, f64)>,
    pub currency_debits: Vec<(String, f64)>,
    pub result: Value,
}

fn list(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

fn currency_moves(entries: &[Value]) -> Vec<(String, f64)> {
    entries
        .iter()
        .filter_map(|e| {
            let id = e.get("catalogId").and_then(Value::as_str)?;
            let q = js::safe_integer(e.get("quantity"))?;
            // A safe integer is exact as a float
            #[allow(clippy::cast_precision_loss)]
            (id.starts_with("CURRENCY_") && q > 0).then(|| (id.to_owned(), q as f64))
        })
        .collect()
}

fn granted(instanced: &[Value], stacked: &[Value]) -> Vec<(String, f64)> {
    let item = |e: &Value, quantity: Option<i64>| {
        let id = e.get("catalogId").and_then(Value::as_str)?;
        let q = quantity?;
        // A safe integer is exact as a float
        #[allow(clippy::cast_precision_loss)]
        (!id.starts_with("CURRENCY_")).then(|| (id.to_owned(), q as f64))
    };
    instanced
        .iter()
        .filter_map(|e| item(e, Some(1)))
        .chain(
            stacked
                .iter()
                .filter_map(|e| item(e, js::safe_integer(e.get("quantity")))),
        )
        .collect()
}

/// Parses one stored item text as `JSON.parse`, which must give an array.
pub(crate) fn parse_items(text: &str, name: &str) -> RepoResult<Vec<Value>> {
    match jsjson::parse(text) {
        Ok(Value::Array(items)) => Ok(items),
        Ok(_) => Err(RepoError::Data(format!("inventory {name} are not an array"))),
        Err(e) => Err(RepoError::Data(format!("inventory {name}: {e}"))),
    }
}

fn text(items: &[Value]) -> String {
    jsjson::stringify(&Value::Array(items.to_vec()))
}

enum Ran {
    Begun(Value),
    Existing(Box<rustic_isles_store::repo::inventory::InventoryTransactionRecord>),
}

/// `RunInventoryTransaction` (§6.1).
pub async fn run_inventory_transaction(
    ctx: TxContext<'_>,
    request: TxRequest,
    options: TxOptions,
    request_id: Option<&str>,
) -> Result<TxOutcome, TxError> {
    let started = std::time::Instant::now();
    let TxRequest {
        user_id,
        character_id,
        transaction_id,
        add_instanced,
        add_stacked,
        remove_instanced,
        remove_stacked,
        save_instanced,
    } = request;
    let Some(account) = ctx
        .owner_of(&user_id, &character_id, !options.ownership_already_validated)
        .await?
    else {
        tracing::error!(
            "Inventory transaction {transaction_id}: characterId {character_id} is not owned by userId {user_id}"
        );
        return Ok(TxOutcome::NotOwned);
    };
    let hash = items::compute_inventory_request_hash(
        &add_instanced,
        &add_stacked,
        &remove_instanced,
        &remove_stacked,
        &save_instanced,
    );
    let policy = StoragePolicy::load(ctx.shared)?;
    let normalized = normalize_inventory_grant_collections(&policy, &transaction_id, &add_instanced, &add_stacked)?;
    if !normalized.corrections.is_empty() {
        let n = normalized.corrections.len();
        tracing::warn!(
            "[InventoryStorage] transactionId {transaction_id} normalized {n} {}: {}",
            if n == 1 { "entry" } else { "entries" },
            normalized
                .corrections
                .iter()
                .map(storage_policy::Correction::describe)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let economy = options.economy.clone().unwrap_or(Economy {
        source: String::from("OTHER"),
        p2p_session_id: None,
        store_sku_id: None,
    });
    let committed_granted = granted(&normalized.add_instanced, &normalized.add_stacked);
    let credits = currency_moves(&normalized.add_stacked);
    let debits = currency_moves(list(&remove_stacked));
    let prestige = ProgressionConfig::load(ctx.shared);
    let starter = crate::bounties::wallet::starter_balances(ctx.shared);
    let shared = Arc::clone(ctx.shared);
    let ports = ctx.ports.clone();
    let actor = player_actor(&account, request_id);
    let closure_account = account.clone();
    let (tx_id, character, user) = (transaction_id.clone(), character_id.clone(), user_id.clone());
    let source = economy.source.clone();
    let hash_in = hash.clone();
    let TxOptions {
        on_grant,
        allow_challenge_reward_funding,
        operator_test_funding,
        ..
    } = options;
    let ran = guarded(
        &ctx.effective_store(),
        Some(&account),
        &actor,
        move |tx: &mut Tx<'_>, repos: &Repositories| -> Result<Ran, TxError> {
            let account = closure_account;
            let key = LedgerKey {
                account: &account,
                character_id: &character,
                transaction_id: &tx_id,
            };
            if let TryBegin::Existing(row) =
                repos
                    .inventory_transactions
                    .try_begin(tx, key, Some(&hash_in), &now_iso())?
            {
                return Ok(Ran::Existing(row));
            }
            let touched = ports.wallet.apply_currency_deltas(
                tx,
                repos,
                &CurrencyChange {
                    shared: &shared,
                    account: &account,
                    user_id: &user,
                    add_stacked: &normalized.add_stacked,
                    remove_stacked: &remove_stacked,
                    transaction_id: &tx_id,
                    allow_challenge_reward_funding,
                    operator_test_funding,
                    prestige: &prestige,
                    starter: &starter,
                },
            )?;
            let annotator = match on_grant {
                Some(hook) => hook(tx, repos)?,
                None => None,
            };
            let record = match repos.inventories.find_by_character_id(tx.conn(), &character)? {
                Some(record) => record,
                None => {
                    tracing::info!("Recovering bootstrap records for characterId {character}");
                    ports
                        .starter
                        .ensure(tx, repos, &shared, &account, &user, &character)?
                        .map_err(|e| RepoError::Data(e.to_string()))?
                }
            };
            let revision = record.revision.unwrap_or(0);
            let instanced = parse_items(&record.instanced_items, "instancedItems")?;
            let stacked = parse_items(&record.stacked_items, "stackedItems")?;
            let mut minter = rustic_isles::replay::next_uuid;
            let applied = items::apply_transaction(Delta {
                user_id: &user,
                transaction_id: &tx_id,
                instanced,
                stacked,
                add_instanced: &normalized.add_instanced,
                add_stacked: &normalized.add_stacked,
                remove_instanced: &remove_instanced,
                remove_stacked: &remove_stacked,
                save_instanced: &save_instanced,
                touched: &touched,
                minter: &mut minter,
            })
            .map_err(|e| TxError::InsufficientStackedItem {
                user: user.clone(),
                catalog_id: e.catalog_id,
                owned: e.owned,
                requested: e.requested,
            })?;
            let updated = repos.inventories.update_both_if_revision_matches(
                tx,
                &character,
                &text(&applied.instanced),
                &text(&applied.stacked),
                revision,
            )?;
            if updated.is_none() {
                tracing::warn!("Inventory transaction {tx_id} for characterId {character}: another write landed first");
                return Err(TxError::Conflict(format!(
                    "Inventory transaction {tx_id} is already in progress (concurrent duplicate request)"
                )));
            }
            let removed = match &remove_instanced {
                Value::Null => Value::Array(Vec::new()),
                other => other.clone(),
            };
            let result = obj(vec![
                ("createdInstancedItems", Value::Array(applied.created.clone())),
                ("updatedInstancedItems", Value::Array(applied.updated_instanced)),
                ("updatedStackedItems", Value::Array(applied.updated_stacked)),
                ("removedInstancedItems", removed),
            ]);
            let mut extra = Map::new();
            extra.insert("economySource".into(), Value::from(economy.source.as_str()));
            if let Some(session) = &economy.p2p_session_id {
                extra.insert("economySessionId".into(), Value::from(session.as_str()));
            }
            if let Some(annotate) = annotator
                && let Some(fields) = annotate(&applied.created, &applied.accepted)
            {
                extra.extend(fields);
            }
            repos
                .inventory_transactions
                .complete(tx, key, &result, Some(&Value::Object(extra)), &now_iso())?;
            Ok(Ran::Begun(result))
        },
    )
    .await?;
    match ran {
        Ran::Begun(result) => {
            ctx.ports.observer.committed(
                ctx.shared,
                &CommittedTransaction {
                    account,
                    user_id: user_id.clone(),
                    transaction_id: transaction_id.clone(),
                    source,
                    granted: committed_granted,
                    currency_credits: credits,
                    currency_debits: debits,
                    result: result.clone(),
                },
            );
            let ms = started.elapsed().as_millis();
            if options.diagnostic_label.is_some() || ms >= 1000 {
                tracing::info!(
                    "[InventoryTiming] label={} total={ms}ms",
                    options.diagnostic_label.as_deref().unwrap_or("inventory")
                );
            }
            Ok(TxOutcome::Done(result))
        }
        Ran::Existing(row) => {
            match row.request_hash.as_deref() {
                Some(stored) if stored != hash => {
                    tracing::error!("Inventory transaction {transaction_id} was reused with a different request body");
                    return Err(TxError::Mismatch(format!(
                        "Inventory transaction {transaction_id} was reused with a different request body"
                    )));
                }
                Some(_) => {}
                None => tracing::warn!(
                    "Inventory transaction {transaction_id} has no stored request hash; replaying it on the old rules"
                ),
            }
            match (row.status, row.result) {
                (LedgerStatus::Completed, Some(result)) => {
                    tracing::info!(
                        "[Inventory] Replayed transactionId {transaction_id} for userId {user_id} and characterId \
                         {character_id}"
                    );
                    Ok(TxOutcome::Done(jsjson::to_js(&result)))
                }
                (LedgerStatus::Completed, None) => Ok(TxOutcome::Done(Value::Null)),
                (LedgerStatus::Pending, _) => Err(TxError::Conflict(format!(
                    "Inventory transaction {transaction_id} is already in progress (concurrent duplicate request)"
                ))),
            }
        }
    }
}

/// `GetInventoryForUserIdAndCharacterId` (§6.3): a read that may write (the starter records, the storage repair and
/// the starter wallet).
pub async fn get_inventory(
    ctx: TxContext<'_>,
    user_id: Option<&str>,
    character_id: &str,
    request_id: Option<&str>,
) -> Result<Option<Value>, TxError> {
    let Some(user) = user_id else { return Ok(None) };
    let Some(account) = ctx.owner_of(user, character_id, true).await? else {
        return Ok(None);
    };
    let policy = StoragePolicy::load(ctx.shared)?;
    let shared = Arc::clone(ctx.shared);
    let ports = ctx.ports.clone();
    let (user, character) = (user.to_owned(), character_id.to_owned());
    let closure_account = account.clone();
    let actor = player_actor(&account, request_id);
    let value = guarded(
        &ctx.effective_store(),
        Some(&account),
        &actor,
        move |tx: &mut Tx<'_>, repos: &Repositories| -> Result<Value, TxError> {
            let account = closure_account;
            let record = ports
                .starter
                .ensure(tx, repos, &shared, &account, &user, &character)?
                .map_err(|e| RepoError::Data(e.to_string()))?;
            let mut instanced = parse_items(&record.instanced_items, "instancedItems")?;
            let mut stacked = parse_items(&record.stacked_items, "stackedItems")?;
            let migration = migrate_inventory_storage_arrays(&policy, instanced.clone(), stacked.clone());
            if migration.changed() {
                let updated = repos.inventories.update_both_if_revision_matches(
                    tx,
                    &character,
                    &text(&migration.instanced),
                    &text(&migration.stacked),
                    record.revision.unwrap_or(0),
                )?;
                let Some(updated) = updated else {
                    return Err(RepoError::Data(format!(
                        "Inventory storage reconciliation for characterId {character} failed"
                    ))
                    .into());
                };
                tracing::warn!(
                    "[InventoryStorage] repaired legacy storage for userId {user} characterId {character}: {}",
                    migration.describe_changes()
                );
                if !migration.issues.is_empty() {
                    tracing::warn!(
                        "[InventoryStorage] userId {user} characterId {character} still has {} stacked non-stackable \
                         entries requiring manual review",
                        migration.issues.len()
                    );
                }
                instanced = parse_items(&updated.instanced_items, "instancedItems")?;
                stacked = parse_items(&updated.stacked_items, "stackedItems")?;
            }
            let stacked = ports
                .wallet
                .merge_into_stacked(tx, repos, &shared, &account, &user, stacked)?;
            Ok(obj(vec![
                ("characterId", Value::from(character.as_str())),
                ("instancedItems", Value::Array(instanced)),
                ("stackedItems", Value::Array(stacked)),
            ]))
        },
    )
    .await?;
    Ok(Some(value))
}

/// The single item save's inputs (§6.4).
#[derive(Debug, Clone, PartialEq)]
pub struct ItemUpdate {
    pub character_id: String,
    pub user_id: Option<String>,
    pub instance_id: Value,
    pub catalog_id: Value,
    /// Absent is none
    pub item_data: Option<Value>,
    pub update_version: Option<Value>,
}

enum SaveAttempt {
    Answer(Option<Value>),
    Retry,
}

/// `UpdateInstancedItem` (§6.4): up to five attempts, each its own write.
pub async fn update_instanced_item(
    ctx: TxContext<'_>,
    update: &ItemUpdate,
    request_id: Option<&str>,
) -> Result<Option<Value>, TxError> {
    let Some(user) = update.user_id.as_deref() else {
        tracing::error!(
            "Instanced item save for characterId {} without a userId",
            update.character_id
        );
        return Ok(None);
    };
    let Some(account) = ctx.owner_of(user, &update.character_id, true).await? else {
        tracing::error!(
            "Instanced item save: characterId {} is not owned by userId {user}",
            update.character_id
        );
        return Ok(None);
    };
    let store = ctx.effective_store();
    let actor = player_actor(&account, request_id);
    for attempt in 1..=5 {
        if attempt > 1 {
            let wait = 5 + rustic_isles::replay::now_ms().rem_euclid(20);
            tokio::time::sleep(Duration::from_millis(u64::try_from(wait).unwrap_or(5))).await;
        }
        let update_in = update.clone();
        let user = user.to_owned();
        let outcome = guarded(
            &store,
            Some(&account),
            &actor,
            move |tx, repos| -> Result<SaveAttempt, TxError> {
                let update = update_in;
                let Some(record) = repos
                    .inventories
                    .find_by_character_id(tx.conn(), &update.character_id)?
                else {
                    tracing::error!("No inventory for characterId {}", update.character_id);
                    return Ok(SaveAttempt::Answer(None));
                };
                let mut instanced = parse_items(&record.instanced_items, "instancedItems")?;
                let Some(at) = instanced
                    .iter()
                    .position(|i| js::strict_eq(i.get("instanceId"), Some(&update.instance_id)))
                else {
                    tracing::warn!(
                        "Ignoring item save for unknown instanceId {} for characterId {}",
                        js::string(Some(&update.instance_id)),
                        update.character_id
                    );
                    return Ok(SaveAttempt::Answer(None));
                };
                if !js::strict_eq(instanced[at].get("catalogId"), Some(&update.catalog_id)) {
                    tracing::warn!(
                        "Rejecting item save that would change catalogId of instanceId {}",
                        js::string(Some(&update.instance_id))
                    );
                    return Ok(SaveAttempt::Answer(None));
                }
                let Some(next) = items::apply_item_data_save(
                    &instanced[at],
                    update.item_data.as_ref(),
                    update.update_version.as_ref(),
                ) else {
                    return Ok(SaveAttempt::Answer(Some(instanced[at].clone())));
                };
                instanced[at] = next.clone();
                let written = repos.inventories.update_both_if_revision_matches(
                    tx,
                    &update.character_id,
                    &text(&instanced),
                    &record.stacked_items,
                    record.revision.unwrap_or(0),
                )?;
                if written.is_none() {
                    return Ok(SaveAttempt::Retry);
                }
                tracing::info!(
                    "Updated instanced item {} for characterId {} and userId {user}",
                    js::string(Some(&update.catalog_id)),
                    update.character_id
                );
                Ok(SaveAttempt::Answer(Some(next)))
            },
        )
        .await?;
        match outcome {
            SaveAttempt::Answer(answer) => return Ok(answer),
            SaveAttempt::Retry => tracing::warn!(
                "Instanced item save for characterId {} lost a revision race, attempt {attempt}/5 - retrying",
                update.character_id
            ),
        }
    }
    tracing::error!(
        "Instanced item save for characterId {} still conflicted after 5 attempts",
        update.character_id
    );
    Ok(None)
}
