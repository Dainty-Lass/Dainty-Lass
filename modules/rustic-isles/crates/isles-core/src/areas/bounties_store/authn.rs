//! What this area takes from the pipeline in place of Express's `req.AuthData`, and the seams to the areas it leans on.
//!
//! `HasParadoxBackendAuth` is RU5 auth's: its middleware puts an [`AuthData`] (and, where the store runs, the
//! [`StoreSeams`]) into the request's extensions before a handler runs. Without an `AuthData` a handler answers what
//! Node's middleware does for a call with no bearer: 401, empty.
// The `Err` is the whole answer, a `Response`
#![allow(clippy::result_large_err)]

use std::sync::Arc;

use axum::response::{IntoResponse, Response};
use module_api::BoxFuture;
use rifts_seam::RuntimeAuth;
use serde_json::Value;

use crate::http::Call;
use crate::ids::AccountId;

/// `req.AuthData`: the verified bearer's claims, or a game server's identity.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthData {
    /// `AuthData.userId`: a player's, or the configured account a game server without a bearer is attributed to
    pub user_id: Option<String>,
    /// `IsGameserver`
    pub is_gameserver: bool,
    /// `IsPlayerHostRuntime` and `RuntimeRosterAccountIds`: a player-host runtime rifts authenticated
    pub host_runtime: Option<RuntimeAuth>,
}

impl AuthData {
    pub fn player(user_id: &str) -> Self {
        Self {
            user_id: Some(user_id.to_owned()),
            ..Self::default()
        }
    }

    pub fn gameserver() -> Self {
        Self {
            is_gameserver: true,
            ..Self::default()
        }
    }

    pub fn is_host_runtime(&self) -> bool {
        self.host_runtime.is_some()
    }

    /// `MonitorSourceOf`.
    pub fn monitor_source(&self) -> &'static str {
        if self.is_gameserver {
            "gs"
        } else if self.is_host_runtime() {
            "host"
        } else {
            "player"
        }
    }

    /// `IsRuntimeRosterTarget`: the accounts a runtime may act for, by the client's `userId` or the global id.
    pub fn roster_has(&self, user_id: &str, account: Option<&AccountId>) -> bool {
        let Some(runtime) = &self.host_runtime else {
            return false;
        };
        let global = account.map(ToString::to_string);
        runtime
            .roster
            .iter()
            .any(|id| id.0 == user_id || global.as_deref() == Some(id.0.as_str()))
    }

    /// `AuthData.userId` as a log prints it: `actor=${userId ?? "unknown"}`.
    pub fn actor(&self) -> &str {
        self.user_id.as_deref().unwrap_or("unknown")
    }
}

impl From<&crate::areas::auth::AuthData> for AuthData {
    fn from(auth: &crate::areas::auth::AuthData) -> Self {
        Self {
            user_id: auth.user_id.clone(),
            is_gameserver: auth.is_gameserver,
            host_runtime: auth.runtime.clone(),
        }
    }
}

/// Reads `AuthData` from the request's extensions, adapting from [`crate::areas::auth::AuthData`] when present.
pub fn auth_of_request(request: &axum::extract::Request) -> Option<AuthData> {
    request
        .extensions()
        .get::<crate::areas::auth::AuthData>()
        .map(AuthData::from)
        .or_else(|| request.extensions().get::<AuthData>().cloned())
}

/// A call with no `AuthData`: `HasParadoxBackendAuth` answers 401 with an empty body.
pub fn auth_of(call: &Call) -> Result<AuthData, Response> {
    auth_of_request(&call.request).ok_or_else(|| axum::http::StatusCode::UNAUTHORIZED.into_response())
}

/// `StorePurchaseTokenPayload`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PurchaseTokenClaims {
    pub user_id: String,
    pub character_id: String,
    pub sku_id: String,
    /// Absent only on a token minted before the store registry
    pub store_tag: Option<String>,
}

/// `SignStorePurchaseToken` and `ValidateStorePurchaseToken` (auth's key store: RS256, issuer `mysticparadox-store`,
/// audience `mysticparadox-store-purchase`, 10 minutes).
pub trait PurchaseTokens: Send + Sync {
    fn sign(&self, claims: &PurchaseTokenClaims) -> Result<String, String>;
    /// `Err` for a bad signature, an expired token or the wrong issuer or audience
    fn validate(&self, token: &str) -> Result<PurchaseTokenClaims, String>;
}

/// `GetCharactersForUid(userId)[0]?.id`: the account's first character (characters-loadouts).
pub trait FirstCharacter: Send + Sync {
    fn first_character_id(&self, account: AccountId) -> BoxFuture<'_, Result<Option<String>, String>>;
}

/// One store purchase as `RunInventoryTransaction` takes it (inventory-reforge): the currency debit and the item
/// grants commit together or not at all, behind an idempotency ledger keyed by `transaction_id`.
#[derive(Debug, Clone, PartialEq)]
pub struct StoreTransaction {
    pub account: AccountId,
    pub user_id: String,
    pub character_id: String,
    pub transaction_id: String,
    pub instanced_to_add: Vec<Value>,
    pub stacked_to_add: Vec<Value>,
    pub stacked_to_remove: Vec<Value>,
    pub ownership_already_validated: bool,
    /// `store:<storeTag>:<skuId>`
    pub diagnostic_label: String,
    /// The SKU's entitlement and progression payloads, applied in the same transaction (`onGrant`)
    pub grants: SkuGrants,
    /// `economy: {source: "STORE", storeSkuId}`
    pub sku_id: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SkuGrants {
    /// `{name, duration}`
    pub entitlements: Vec<(String, f64)>,
    /// `{progressionId, xp, ranks}`
    pub progression: Option<Value>,
}

/// `RunInventoryTransaction`'s refusals that `PurchaseStoreSku` maps to a reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionFailure {
    InsufficientBalance,
    Conflict,
    Mismatch,
    Failed(String),
}

/// A row of the inventory transaction ledger (`FindClaimInInventoryLedger`): the character whose transaction it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyClaim {
    pub character_id: Option<String>,
}

pub trait InventoryTransactions: Send + Sync {
    /// The transaction's result as Node returns it; `Err(Failed)` where Node returns a falsy result
    fn run(&self, transaction: StoreTransaction) -> BoxFuture<'_, Result<Value, TransactionFailure>>;

    /// Whether the ledger holds this (user, transaction) pair, for the once-per-window daily claims
    fn find_claim(
        &self,
        account: AccountId,
        transaction_id: String,
    ) -> BoxFuture<'_, Result<Option<DailyClaim>, String>>;
}

/// What the store routes lean on, put in the request's extensions with the [`AuthData`].
#[derive(Clone)]
pub struct StoreSeams {
    pub tokens: Arc<dyn PurchaseTokens>,
    pub characters: Arc<dyn FirstCharacter>,
    pub inventory: Arc<dyn InventoryTransactions>,
}

struct ProductionCharacters(Arc<crate::http::Shared>);

impl FirstCharacter for ProductionCharacters {
    fn first_character_id(&self, account: AccountId) -> BoxFuture<'_, Result<Option<String>, String>> {
        let shared = self.0.clone();
        Box::pin(async move {
            let Some(store) = shared.store.as_ref() else {
                return Ok(None);
            };
            let acc_lookup = account.clone();
            let user_id = store
                .read(move |conn, repos| {
                    Ok(repos
                        .accounts
                        .find(conn, &acc_lookup)?
                        .map(|a| a.user_id.0)
                        .unwrap_or_else(|| acc_lookup.to_string()))
                })
                .await
                .map_err(|e| e.to_string())?;
            let actor = module_api::ActorContext::new(module_api::Actor::Player {
                account: account.global().clone(),
            });
            match crate::areas::characters::controller::characters_for_user(
                store,
                &shared.version.tables,
                &actor,
                &user_id,
            )
            .await
            {
                Ok(list) => Ok(list
                    .first()
                    .and_then(|c| c.get("id"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)),
                Err(error) => Err(error.to_string()),
            }
        })
    }
}

struct ProductionInventory {
    shared: Arc<crate::http::Shared>,
    request_id: String,
}

impl InventoryTransactions for ProductionInventory {
    fn run(&self, transaction: StoreTransaction) -> BoxFuture<'_, Result<Value, TransactionFailure>> {
        let shared = self.shared.clone();
        let request_id = self.request_id.clone();
        Box::pin(async move {
            use crate::areas::inventory::ports::Ports as InventoryPorts;
            use crate::areas::inventory::transaction::{
                Economy, GrantHook, TxContext, TxError, TxOptions, TxOutcome, TxRequest, run_inventory_transaction,
            };

            let Some(store) = shared.store.as_ref() else {
                return Err(TransactionFailure::Failed("store unavailable".into()));
            };
            let has_non_inventory_grants =
                !transaction.grants.entitlements.is_empty() || transaction.grants.progression.is_some();
            let on_grant: Option<GrantHook> = if has_non_inventory_grants {
                let tx_clone = transaction.clone();
                let shared_for_hook = shared.clone();
                Some(Box::new(move |tx, repos| {
                    super::handlers::grant_sku_entitlements(repos, tx, &tx_clone).map_err(TxError::Repo)?;
                    if let Some(prog) = &tx_clone.grants.progression {
                        let raw_track_id = prog.get("progressionId").and_then(Value::as_str).unwrap_or_default();
                        if let Some(track_id) =
                            crate::areas::progression::tracks::resolve_track_id(&shared_for_hook, raw_track_id)
                        {
                            let current = repos
                                .progression_tracks
                                .get(tx.conn(), &tx_clone.account, &track_id)
                                .map_err(TxError::Repo)?
                                .map_or(0, |r| r.progress);
                            let xp = prog.get("xp").and_then(Value::as_f64).map_or(0, |v| v.trunc() as i64);
                            let ranks = prog
                                .get("ranks")
                                .and_then(Value::as_f64)
                                .map_or(0, |v| v.trunc() as i64);
                            if xp > 0 {
                                repos
                                    .progression_tracks
                                    .increment(tx, &tx_clone.account, &track_id, xp)
                                    .map_err(TxError::Repo)?;
                            } else if ranks > 0 {
                                let curves = crate::areas::progression::tracks::curves(&shared_for_hook);
                                if let Some(curve) = curves.get(&track_id) {
                                    let from_rank = curves.rank_for_progress(&track_id, current as f64).unwrap_or(0);
                                    if from_rank < curve.max_rank {
                                        let to_rank = (from_rank + ranks).min(curve.max_rank);
                                        if let Some(threshold) = curve.required_xp_for_rank(to_rank) {
                                            let delta = (threshold as i64) - current;
                                            if delta > 0 {
                                                repos
                                                    .progression_tracks
                                                    .increment(tx, &tx_clone.account, &track_id, delta)
                                                    .map_err(TxError::Repo)?;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Ok(None)
                }))
            } else {
                None
            };

            let ports = InventoryPorts::default();
            let ctx = TxContext {
                shared: &shared,
                store,
                ports: &ports,
            };
            let request = TxRequest {
                user_id: transaction.user_id,
                character_id: transaction.character_id,
                transaction_id: transaction.transaction_id,
                add_instanced: Value::Array(transaction.instanced_to_add),
                add_stacked: Value::Array(transaction.stacked_to_add),
                remove_instanced: Value::Array(Vec::new()),
                remove_stacked: Value::Array(transaction.stacked_to_remove),
                save_instanced: Value::Array(Vec::new()),
            };
            let options = TxOptions {
                ownership_already_validated: transaction.ownership_already_validated,
                diagnostic_label: Some(transaction.diagnostic_label),
                on_grant,
                economy: Some(Economy {
                    source: "STORE".into(),
                    p2p_session_id: None,
                    store_sku_id: Some(transaction.sku_id),
                }),
                ..TxOptions::default()
            };
            match run_inventory_transaction(&ctx, request, options, Some(&request_id)).await {
                Ok(TxOutcome::Done(result)) => Ok(result),
                Ok(TxOutcome::NotOwned) => Err(TransactionFailure::Failed("not_owned".into())),
                Err(TxError::InsufficientBalance(_)) => Err(TransactionFailure::InsufficientBalance),
                Err(TxError::Conflict(_)) => Err(TransactionFailure::Conflict),
                Err(TxError::Mismatch(_)) => Err(TransactionFailure::Mismatch),
                Err(other) => Err(TransactionFailure::Failed(other.to_string())),
            }
        })
    }

    fn find_claim(
        &self,
        account: AccountId,
        transaction_id: String,
    ) -> BoxFuture<'_, Result<Option<DailyClaim>, String>> {
        let shared = self.shared.clone();
        Box::pin(async move {
            let Some(store) = shared.store.as_ref() else {
                return Err("store unavailable".into());
            };
            store
                .read(move |conn, repos| {
                    let characters = repos.characters.find_many_by_account(conn, &account)?;
                    for ch in &characters {
                        let key = crate::repo::inventory::LedgerKey {
                            account: &account,
                            character_id: &ch.character_id,
                            transaction_id: &transaction_id,
                        };
                        if let Some(found) = repos.inventory_transactions.find_record(conn, key)? {
                            return Ok(Some(DailyClaim {
                                character_id: Some(found.character_id),
                            }));
                        }
                    }
                    let recent = repos
                        .inventory_transactions
                        .list_recent_for_account(conn, &account, 500)?;
                    for row in recent {
                        if row.get("transactionId").and_then(Value::as_str) == Some(transaction_id.as_str()) {
                            return Ok(Some(DailyClaim {
                                character_id: row.get("characterId").and_then(Value::as_str).map(str::to_owned),
                            }));
                        }
                    }
                    Ok(None)
                })
                .await
                .map_err(|e| e.to_string())
        })
    }
}

impl StoreSeams {
    pub fn production(shared: &Arc<crate::http::Shared>, request_id: &str) -> Option<Self> {
        shared.store.as_ref()?;
        Some(Self {
            tokens: Arc::new(crate::areas::auth::tokens::StoreTokens(shared.settings.clone())),
            characters: Arc::new(ProductionCharacters(shared.clone())),
            inventory: Arc::new(ProductionInventory {
                shared: shared.clone(),
                request_id: request_id.to_owned(),
            }),
        })
    }
}

pub fn seams_of(call: &Call) -> Option<StoreSeams> {
    call.request
        .extensions()
        .get::<StoreSeams>()
        .cloned()
        .or_else(|| StoreSeams::production(&call.shared, &call.request_id))
}
