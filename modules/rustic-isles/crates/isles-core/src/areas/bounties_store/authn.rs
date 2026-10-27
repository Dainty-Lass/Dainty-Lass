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

/// A call with no `AuthData`: `HasParadoxBackendAuth` answers 401 with an empty body.
pub fn auth_of(call: &Call) -> Result<AuthData, Response> {
    call.request
        .extensions()
        .get::<AuthData>()
        .cloned()
        .ok_or_else(|| axum::http::StatusCode::UNAUTHORIZED.into_response())
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

pub fn seams_of(call: &Call) -> Option<StoreSeams> {
    call.request.extensions().get::<StoreSeams>().cloned()
}
