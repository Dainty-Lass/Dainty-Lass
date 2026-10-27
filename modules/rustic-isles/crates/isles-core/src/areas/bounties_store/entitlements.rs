//! Entitlements (`controllers/entitlements.ts`): the third grant a store SKU can carry, and how Hunt Pass Premium and the
//! Middleman's extra slots are owned. The wire shape is the captured one (`{entitlements: [{activatedDate, duration,
//! name}]}`, sorted by name) on 1.14.7 and 1.12.0's stub on 1.12, which is the `entitlements_payload` hook.

use super::util::now_iso;
use crate::hooks::Entitlement;
use crate::ids::AccountId;
use crate::repo::bounties::{EntitlementEntry, is_valid_entitlement_name};
use crate::repo::{AnyConnection, RepoError, RepoResult, Repositories, Tx};

/// `GetEntitlementsForUser`: sorted by name as `localeCompare` sorts, the audit fields left out.
pub fn entitlements_for_account(
    repos: &Repositories,
    conn: &mut AnyConnection,
    account: &AccountId,
) -> RepoResult<Vec<Entitlement>> {
    let mut list: Vec<Entitlement> = repos
        .entitlements
        .find(conn, account)?
        .map(|record| {
            record
                .entitlements
                .into_iter()
                .map(|e| Entitlement {
                    name: e.name,
                    duration: e.duration,
                    activated_date: e.activated_date,
                })
                .collect()
        })
        .unwrap_or_default();
    list.sort_by(|a, b| service_kit::collate::locale_compare(&a.name, &b.name));
    Ok(list)
}

/// `DoesUserHoldEntitlement`.
pub fn holds_entitlement(
    repos: &Repositories,
    conn: &mut AnyConnection,
    account: &AccountId,
    name: &str,
) -> RepoResult<bool> {
    Ok(repos
        .entitlements
        .find(conn, account)?
        .is_some_and(|record| record.entitlements.iter().any(|e| e.name == name)))
}

/// `GrantEntitlementsInTransaction`: inside the caller's transaction, so the grant commits with the currency debit or
/// not at all. Each grant is idempotent; returns the names this call added. An unsafe name is refused rather than
/// skipped: it is a data defect, and aborting surfaces it instead of selling a player something that grants nothing.
pub fn grant_entitlements_in_transaction(
    repos: &Repositories,
    tx: &mut Tx<'_>,
    account: &AccountId,
    grants: &[(String, f64)],
    source_sku_id: Option<&str>,
) -> RepoResult<Vec<String>> {
    let mut added = Vec::new();
    for (name, duration) in grants {
        if !is_valid_entitlement_name(name) {
            return Err(RepoError::Data(format!(
                "Refusing entitlement grant for {account}: unsafe or missing entitlement name {name:?}"
            )));
        }
        let duration = if duration.is_finite() { *duration } else { 0.0 };
        // Every store entitlement captured is permanent (duration 0, no activation); a timed one is stamped
        let granted_at = now_iso();
        let entry = EntitlementEntry {
            name: name.clone(),
            duration,
            activated_date: (duration > 0.0).then(|| granted_at.clone()),
            granted_at: Some(granted_at),
            source_sku_id: source_sku_id.map(str::to_owned),
        };
        if repos.entitlements.grant_if_missing(tx, account, &entry)? {
            added.push(name.clone());
        }
    }
    if !added.is_empty() {
        tracing::info!(
            "[Entitlement] {account} granted {}{}",
            added.join(", "),
            source_sku_id.map_or_else(String::new, |sku| format!(" (sku={sku})"))
        );
    }
    Ok(added)
}
