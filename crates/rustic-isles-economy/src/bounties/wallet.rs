// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The wallet (bounties spec §4.8.1, inventory's W1 to W4): an account's currency balances, never stored in an
//! inventory blob. Every write is inside the caller's transaction.

use rustic_isles::Shared;
use rustic_isles_store::repo::bounties::{WalletRecord, is_valid_balance_catalog_id};
use rustic_isles_store::{AccountId, AnyConnection, RepoError, Repositories, Tx};
use serde_json::{Map, Value};
use service_kit::jsjson;

use crate::bounties::platinum;
use crate::util::{number_value, obj};

/// The starter grant when the characters area is not composed (the same six balances it writes).
pub const DEFAULT_STARTER_WALLET: [(&str, f64); 6] = [
    ("CURRENCY_NOTES", 5000.0),
    ("CURRENCY_PJM_WEAPON", 50.0),
    ("CURRENCY_PJM_ARMOR", 50.0),
    ("CURRENCY_PJM_ALCHEMY", 50.0),
    ("CURRENCY_PJM_AIRSHIP", 50.0),
    ("CURRENCY_PJM_FISHING", 50.0),
];

/// The one starter wallet: the characters area's `starter_wallet()` (characters decision item 4), else the default.
pub fn starter_balances(shared: &Shared) -> Vec<(String, f64)> {
    match shared.slot::<rustic_isles_characters::Characters>() {
        Some(characters) => characters.starter_wallet().to_vec(),
        None => DEFAULT_STARTER_WALLET
            .iter()
            .map(|(id, n)| ((*id).to_owned(), *n))
            .collect(),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WalletError {
    #[error("Insufficient balance: user {account} has {} of {catalog_id}, cannot apply delta {}", jsjson::number(*current), jsjson::number(*delta))]
    Insufficient {
        account: AccountId,
        catalog_id: String,
        delta: f64,
        current: f64,
    },
    #[error("Refusing currency change: unsafe balance catalogId {}", jsjson::stringify(&Value::String(.0.clone())))]
    UnsafeCatalogId(String),
    #[error(transparent)]
    Repo(#[from] RepoError),
}

fn starter_record(account: &AccountId, starter: &[(String, f64)]) -> WalletRecord {
    WalletRecord {
        account: account.clone(),
        balances: starter.to_vec(),
        bootstrap_version: None,
    }
}

/// W1: the stored balances, the starter wallet created when there is none (never resetting an existing one).
pub fn get_wallet(
    tx: &mut Tx<'_>,
    repos: &Repositories,
    account: &AccountId,
    starter: &[(String, f64)],
) -> Result<Vec<(String, f64)>, RepoError> {
    Ok(repos
        .wallets
        .create_if_missing(tx, &starter_record(account, starter))?
        .balances)
}

/// The stored balances, or the starter's, writing nothing.
pub fn read_wallet(
    conn: &mut AnyConnection,
    repos: &Repositories,
    account: &AccountId,
    starter: &[(String, f64)],
) -> Result<Vec<(String, f64)>, RepoError> {
    Ok(match repos.wallets.find(conn, account)? {
        Some(wallet) => wallet.balances,
        None => starter.to_vec(),
    })
}

fn balance_of(balances: &[(String, f64)], id: &str) -> f64 {
    balances.iter().find(|(c, _)| c == id).map_or(0.0, |(_, b)| *b)
}

/// W2: one signed change; a debit never goes below zero.
pub fn add_currency(
    tx: &mut Tx<'_>,
    repos: &Repositories,
    account: &AccountId,
    starter: &[(String, f64)],
    catalog_id: &str,
    delta: f64,
) -> Result<f64, WalletError> {
    if catalog_id.is_empty() || !delta.is_finite() || delta == 0.0 {
        return Ok(balance_of(&get_wallet(tx, repos, account, starter)?, catalog_id));
    }
    if !is_valid_balance_catalog_id(catalog_id) {
        return Err(WalletError::UnsafeCatalogId(catalog_id.to_owned()));
    }
    let mut applied = repos.wallets.increment_balance(tx, account, catalog_id, delta)?;
    if applied.is_none() && repos.wallets.find(tx.conn(), account)?.is_none() {
        get_wallet(tx, repos, account, starter)?;
        applied = repos.wallets.increment_balance(tx, account, catalog_id, delta)?;
    }
    match applied {
        Some(wallet) => {
            let balance = wallet.balance(catalog_id).unwrap_or(0.0);
            let sign = if delta >= 0.0 { "+" } else { "" };
            tracing::info!(
                "[Wallet] {account} {catalog_id} {sign}{} -> {}",
                jsjson::number(delta),
                jsjson::number(balance)
            );
            Ok(balance)
        }
        None => {
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
    }
}

/// W3: Platinum across its buckets, most disposable first; all or nothing. Each debited bucket with its new
/// balance, then the aggregate with the new total.
pub fn spend_platinum(
    tx: &mut Tx<'_>,
    repos: &Repositories,
    account: &AccountId,
    starter: &[(String, f64)],
    amount: f64,
) -> Result<Vec<(String, f64)>, WalletError> {
    let requested = amount.trunc();
    if !requested.is_finite() || requested <= 0.0 {
        return Ok(Vec::new());
    }
    let balances = get_wallet(tx, repos, account, starter)?;
    let total = platinum::sum_platinum(&balances);
    if total < requested {
        tracing::warn!(
            "[Wallet] {account} platinum debit rejected: requested {}, holds {} across all buckets",
            jsjson::number(requested),
            jsjson::number(total)
        );
        return Err(WalletError::Insufficient {
            account: account.clone(),
            catalog_id: platinum::PLATINUM_AGGREGATE_ID.to_owned(),
            delta: -requested,
            current: total,
        });
    }
    let mut outstanding = requested;
    let mut report = Vec::new();
    for bucket in platinum::SPEND_ORDER {
        if outstanding <= 0.0 {
            break;
        }
        let held = balance_of(&balances, bucket);
        if !held.is_finite() || held <= 0.0 {
            continue;
        }
        let take = held.min(outstanding);
        let after = add_currency(tx, repos, account, starter, bucket, -take)?;
        outstanding -= take;
        report.push((bucket.to_owned(), after));
    }
    let after = get_wallet(tx, repos, account, starter)?;
    report.push((
        platinum::PLATINUM_AGGREGATE_ID.to_owned(),
        platinum::sum_platinum(&after),
    ));
    Ok(report)
}

/// W4: each balance over an inventory's stacked items (a projection; nothing is written back).
pub fn merge_wallet_into_stacked(balances: &[(String, f64)], stacked: Vec<Value>) -> Vec<Value> {
    let mut stacked = stacked;
    for (id, balance) in balances {
        let found = stacked
            .iter_mut()
            .find(|s| s.get("catalogId").and_then(Value::as_str) == Some(id.as_str()));
        match found.and_then(Value::as_object_mut) {
            Some(map) => {
                map.insert("catalogId".into(), Value::from(id.as_str()));
                map.insert("quantity".into(), number_value(*balance));
            }
            None => stacked.push(obj(vec![
                ("catalogId", Value::from(id.as_str())),
                ("quantity", number_value(*balance)),
            ])),
        }
    }
    stacked
}

/// `id_currency_` and the id without a leading `CURRENCY_`, lower-cased.
pub fn id_form(catalog_id: &str) -> String {
    let bare = catalog_id.strip_prefix("CURRENCY_").unwrap_or(catalog_id);
    format!("id_currency_{}", bare.to_lowercase())
}

/// `ProjectBalances` (§4.8.4, Node's `BuildBalanceDict`) over `base`.
pub fn project_balances(balances: &[(String, f64)], base: Map<String, Value>) -> Map<String, Value> {
    let mut out = base;
    for (id, balance) in balances {
        out.insert(id.clone(), number_value(*balance));
        out.insert(id_form(id), number_value(*balance));
    }
    let total = platinum::sum_platinum(balances);
    if total != 0.0 || balances.iter().any(|(id, _)| id == platinum::PLATINUM_AGGREGATE_ID) {
        out.insert(platinum::PLATINUM_AGGREGATE_ID.into(), number_value(total));
        out.insert("id_currency_platinum".into(), number_value(total));
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn merge_and_project() {
        let balances = vec![
            ("CURRENCY_NOTES".to_owned(), 5000.0),
            ("CURRENCY_PLATINUM_UNIV".to_owned(), 25.0),
        ];
        let merged = merge_wallet_into_stacked(
            &balances,
            vec![
                json!({"catalogId": "CURRENCY_NOTES", "quantity": 1, "x": 1}),
                json!({"catalogId": "MAT", "quantity": 2}),
            ],
        );
        assert_eq!(
            merged,
            vec![
                json!({"catalogId": "CURRENCY_NOTES", "quantity": 5000, "x": 1}),
                json!({"catalogId": "MAT", "quantity": 2}),
                json!({"catalogId": "CURRENCY_PLATINUM_UNIV", "quantity": 25}),
            ]
        );
        let mut base = Map::new();
        base.insert("CURRENCY_PLATINUM".into(), json!(0));
        let projected = project_balances(&balances, base);
        assert_eq!(
            Value::Object(projected),
            json!({
                "CURRENCY_PLATINUM": 25, "CURRENCY_NOTES": 5000, "id_currency_notes": 5000,
                "CURRENCY_PLATINUM_UNIV": 25, "id_currency_platinum_univ": 25, "id_currency_platinum": 25,
            })
        );
        let mut base = Map::new();
        base.insert("CURRENCY_PLATINUM".into(), json!(7));
        let none = project_balances(&[("CURRENCY_NOTES".to_owned(), 1.0)], base);
        assert_eq!(none["CURRENCY_PLATINUM"], json!(7));
    }

    #[test]
    fn insufficient_text() {
        let account: AccountId = "account:01ARZ3NDEKTSV4RRFFQ69G5FAV@n".parse().unwrap();
        let e = WalletError::Insufficient {
            account,
            catalog_id: "CURRENCY_NOTES".into(),
            delta: -10.0,
            current: 5.0,
        };
        assert_eq!(
            e.to_string(),
            "Insufficient balance: user account:01ARZ3NDEKTSV4RRFFQ69G5FAV@n has 5 of CURRENCY_NOTES, cannot apply delta -10"
        );
        assert_eq!(
            WalletError::UnsafeCatalogId("a b".into()).to_string(),
            "Refusing currency change: unsafe balance catalogId \"a b\""
        );
    }
}
