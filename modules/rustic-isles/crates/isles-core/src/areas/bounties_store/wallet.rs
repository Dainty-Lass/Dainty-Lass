//! The wallet (`controllers/wallet.ts`): the balance map of an account, with a starter grant on first access, atomic
//! credits and debits that never go below zero, the platinum debit split across its buckets, and the projections
//! `GET /balance`, `POST /reconcile` and `GET /inventory` serve.
//!
//! Called inside a request's closure on `store`, so a spend commits with the inventory change it belongs to. What the
//! inventory area calls (`POST /inventory`'s currency deltas, store purchases): [`get_wallet`], [`add_currency`],
//! [`spend_platinum`], [`merge_wallet_into_stacked`]. The prestige-gated reward rules of `ApplyCurrencyDeltas` stay
//! with the inventory and progression areas, which own the tracks they spend.

use serde_json::{Map, Value};

use super::platinum::{PLATINUM_AGGREGATE_ID, PlatinumSpend, project_balances, resolve_platinum_spend, sum_platinum};
use super::util::num;
use crate::ids::AccountId;
use crate::repo::bounties::{WalletRecord, is_valid_balance_catalog_id};
use crate::repo::{AnyConnection, RepoError, Repositories, Tx};

/// Granted once, when a wallet is first created (first login, first balance query).
pub const STARTER_WALLET: [(&str, f64); 6] = [
    ("CURRENCY_NOTES", 5000.0),
    ("CURRENCY_PJM_WEAPON", 50.0),
    ("CURRENCY_PJM_ARMOR", 50.0),
    ("CURRENCY_PJM_ALCHEMY", 50.0),
    ("CURRENCY_PJM_AIRSHIP", 50.0),
    ("CURRENCY_PJM_FISHING", 50.0),
];

pub fn starter_wallet(account: &AccountId) -> WalletRecord {
    WalletRecord {
        account: account.clone(),
        balances: STARTER_WALLET.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect(),
        bootstrap_version: None,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WalletError {
    /// `InsufficientBalanceError`: a debit would take the balance below zero, never clamped
    #[error("Insufficient balance: user {account} has {current} of {catalog_id}, cannot apply delta {delta}")]
    Insufficient {
        account: AccountId,
        catalog_id: String,
        delta: f64,
        current: f64,
    },
    #[error("Refusing currency change: unsafe balance catalogId {0:?}")]
    UnsafeCatalogId(String),
    #[error(transparent)]
    Repo(#[from] RepoError),
}

/// `GetWallet` on a read connection: the balances, or the starter wallet's when none is stored yet (nothing is written;
/// the first write path creates it). Pair with [`get_wallet`] where a write is open.
pub fn read_wallet(
    repos: &Repositories,
    conn: &mut AnyConnection,
    account: &AccountId,
) -> Result<Vec<(String, f64)>, RepoError> {
    Ok(repos
        .wallets
        .find(conn, account)?
        .map_or_else(|| starter_wallet(account).balances, |wallet| wallet.balances))
}

/// `GetWallet`: the full balance map, auto-creating the wallet with the starter grant on first access (one
/// create-if-missing, so an existing balance row is adopted unchanged).
pub fn get_wallet(repos: &Repositories, tx: &mut Tx<'_>, account: &AccountId) -> Result<Vec<(String, f64)>, RepoError> {
    Ok(repos.wallets.create_if_missing(tx, &starter_wallet(account))?.balances)
}

pub fn get_balance(
    repos: &Repositories,
    tx: &mut Tx<'_>,
    account: &AccountId,
    catalog_id: &str,
) -> Result<f64, RepoError> {
    Ok(get_wallet(repos, tx, account)?
        .iter()
        .find(|(id, _)| id == catalog_id)
        .map_or(0.0, |(_, v)| *v))
}

/// `AddCurrency`: a signed delta applied atomically; a debit that would go negative is rejected. Returns the new
/// balance.
pub fn add_currency(
    repos: &Repositories,
    tx: &mut Tx<'_>,
    account: &AccountId,
    catalog_id: &str,
    delta: f64,
) -> Result<f64, WalletError> {
    if catalog_id.is_empty() || !delta.is_finite() || delta == 0.0 {
        return Ok(get_balance(repos, tx, account, catalog_id)?);
    }
    // The id becomes a key of a stored row: anything but a strict identifier is refused before any wallet is created
    if !is_valid_balance_catalog_id(catalog_id) {
        return Err(WalletError::UnsafeCatalogId(catalog_id.to_owned()));
    }
    let updated = match repos.wallets.increment_balance(tx, account, catalog_id, delta)? {
        Some(wallet) => Some(wallet),
        None => {
            if repos.wallets.find(tx.conn(), account)?.is_none() {
                repos.wallets.create_if_missing(tx, &starter_wallet(account))?;
                repos.wallets.increment_balance(tx, account, catalog_id, delta)?
            } else {
                None
            }
        }
    };
    if let Some(wallet) = updated {
        let next = wallet.balance(catalog_id).unwrap_or(0.0);
        tracing::info!(
            "[Wallet] {account} {catalog_id} {}{delta} -> {next}",
            if delta >= 0.0 { "+" } else { "" }
        );
        return Ok(next);
    }
    // For a negative delta the guarded update reaches here because the balance is insufficient
    let current = repos
        .wallets
        .find(tx.conn(), account)?
        .and_then(|w| w.balance(catalog_id))
        .unwrap_or(0.0);
    Err(WalletError::Insufficient {
        account: account.clone(),
        catalog_id: catalog_id.to_owned(),
        delta,
        current,
    })
}

/// `SpendPlatinum`: debits `amount` across the buckets that hold it and reports each touched bucket and the new
/// aggregate. The same `Insufficient` error as every other overspend.
pub fn spend_platinum(
    repos: &Repositories,
    tx: &mut Tx<'_>,
    account: &AccountId,
    amount: f64,
) -> Result<Vec<(String, f64)>, WalletError> {
    let mut touched: Vec<(String, f64)> = Vec::new();
    let requested = amount.trunc();
    if !requested.is_finite() || requested <= 0.0 {
        return Ok(touched);
    }
    let wallet = get_wallet(repos, tx, account)?;
    let plan = match resolve_platinum_spend(&wallet, requested) {
        PlatinumSpend::Ok(deltas) => deltas,
        PlatinumSpend::Short { available, requested } => {
            tracing::warn!(
                "[Wallet] {account} platinum debit rejected: requested {requested}, holds {available} across all buckets"
            );
            return Err(WalletError::Insufficient {
                account: account.clone(),
                catalog_id: PLATINUM_AGGREGATE_ID.to_owned(),
                delta: -requested,
                current: available,
            });
        }
    };
    for (catalog_id, delta) in &plan {
        let next = add_currency(repos, tx, account, catalog_id, *delta)?;
        touched.push((catalog_id.clone(), next));
    }
    let mut after = wallet;
    for (id, next) in &touched {
        match after.iter_mut().find(|(k, _)| k == id) {
            Some(slot) => slot.1 = *next,
            None => after.push((id.clone(), *next)),
        }
    }
    let remaining = sum_platinum(&after);
    touched.push((PLATINUM_AGGREGATE_ID.to_owned(), remaining));
    Ok(touched)
}

/// `MergeWalletIntoStacked`: the wallet's currencies in an inventory's `stackedItems`, replacing the quantity of an
/// existing entry and appending the rest. A projected response only: nothing is written back.
pub fn merge_wallet_into_stacked(wallet: &[(String, f64)], stacked_items: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = stacked_items.to_vec();
    for (catalog_id, amount) in wallet {
        let found = out
            .iter()
            .position(|item| item.get("catalogId").and_then(Value::as_str) == Some(catalog_id.as_str()));
        match found {
            Some(index) => {
                let mut item = out[index].as_object().cloned().unwrap_or_default();
                item.insert("catalogId".into(), catalog_id.clone().into());
                item.insert("quantity".into(), num(*amount));
                out[index] = Value::Object(item);
            }
            None => out.push(serde_json::json!({ "catalogId": catalog_id, "quantity": num(*amount) })),
        }
    }
    out
}

/// `BuildBalanceDict`: the `/balance`-style currency dict, both naming forms.
pub fn build_balance_dict(wallet: &[(String, f64)], base: Map<String, Value>) -> Map<String, Value> {
    project_balances(wallet, base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merging_the_wallet_replaces_and_appends() {
        let stacked = vec![
            serde_json::json!({ "catalogId": "CURRENCY_NOTES", "quantity": 1, "extra": true }),
            serde_json::json!({ "catalogId": "TOKEN_X", "quantity": 3 }),
        ];
        let wallet = vec![
            ("CURRENCY_NOTES".to_owned(), 5000.0),
            ("CURRENCY_PJM_WEAPON".to_owned(), 50.0),
        ];
        let merged = merge_wallet_into_stacked(&wallet, &stacked);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0]["quantity"], 5000);
        assert_eq!(merged[0]["extra"], true);
        assert_eq!(merged[2]["catalogId"], "CURRENCY_PJM_WEAPON");
    }
}
