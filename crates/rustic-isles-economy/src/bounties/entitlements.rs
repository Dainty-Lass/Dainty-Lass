// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Entitlements (bounties spec §4.7): the third grant a SKU can carry, and how Hunt Pass Premium and the
//! Middleman's extra slots are owned.

use rustic_isles::Entitlement;
use rustic_isles_store::repo::bounties::{EntitlementEntry, is_valid_entitlement_name};
use rustic_isles_store::{AccountId, AnyConnection, RepoError, RepoResult, Repositories, Tx};
use serde_json::Value;

use crate::util::now_iso;

/// The account's live entitlements, sorted by name.
pub fn list_entitlements(
    conn: &mut AnyConnection,
    repos: &Repositories,
    account: &AccountId,
) -> RepoResult<Vec<Entitlement>> {
    let mut list: Vec<Entitlement> = repos
        .entitlements
        .find(conn, account)?
        .map(|r| r.entitlements)
        .unwrap_or_default()
        .into_iter()
        .map(|e| Entitlement {
            name: e.name,
            duration: e.duration,
            activated: e.activated_date,
        })
        .collect();
    list.sort_by(|a, b| service_kit::collate::locale_compare(&a.name, &b.name));
    Ok(list)
}

/// `DoesUserHoldEntitlement` (no caller at the spec's revision; kept for the Hunt Pass Premium check).
pub fn holds_entitlement(
    conn: &mut AnyConnection,
    repos: &Repositories,
    account: &AccountId,
    name: &str,
) -> RepoResult<bool> {
    Ok(repos
        .entitlements
        .find(conn, account)?
        .is_some_and(|r| r.entitlements.iter().any(|e| e.name == name)))
}

/// `GrantEntitlementsInTransaction`: the names it added. An unsafe name refuses the whole grant (a data defect).
pub fn grant_entitlements_in_transaction(
    tx: &mut Tx<'_>,
    repos: &Repositories,
    account: &AccountId,
    grants: &[(String, f64)],
    source_sku_id: Option<&str>,
) -> RepoResult<Vec<String>> {
    if let Some((name, _)) = grants.iter().find(|(name, _)| !is_valid_entitlement_name(name)) {
        return Err(RepoError::Data(format!(
            "Refusing entitlement grant for {account}: unsafe or missing entitlement name {}",
            service_kit::jsjson::stringify(&Value::String(name.clone()))
        )));
    }
    let now = now_iso();
    let mut added = Vec::new();
    for (name, duration) in grants {
        let duration = if duration.is_finite() { *duration } else { 0.0 };
        let entry = EntitlementEntry {
            name: name.clone(),
            duration,
            activated_date: (duration > 0.0).then(|| now.clone()),
            granted_at: Some(now.clone()),
            source_sku_id: source_sku_id.map(str::to_owned),
        };
        if repos.entitlements.grant_if_missing(tx, account, &entry)? {
            added.push(name.clone());
        }
    }
    if !added.is_empty() {
        let sku = source_sku_id.map(|s| format!(" (sku={s})")).unwrap_or_default();
        tracing::info!("[Entitlement] {account} granted {}{sku}", added.join(", "));
    }
    Ok(added)
}
