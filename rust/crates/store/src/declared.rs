// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The account columns a database's owner declares (DD23, DH3b), for the history and revert tools that open
//! the database without the owner's code: `Database::attach` reads them. A module compiled into the server
//! calls [`declare_accounts`] where it opens its [`crate::Database`], with the columns it also gives
//! `with_account_columns`, so `history --account` finds its tables whoever opens the file.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock, PoisonError};

use crate::revert::AccountColumn;

type Registry = Mutex<BTreeMap<String, Vec<AccountColumn>>>;

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(Registry::default)
}

/// Adds `columns` to what `database` (a name such as `node`, the one `history` prints) has declared.
/// Declaring a column again changes nothing.
pub fn declare_accounts(database: &str, columns: &[AccountColumn]) {
    let mut all = registry().lock().unwrap_or_else(PoisonError::into_inner);
    let declared = all.entry(database.to_owned()).or_default();
    for column in columns {
        if !declared.contains(column) {
            declared.push(*column);
        }
    }
}

/// What [`declare_accounts`] was given for `database`, in the order declared.
pub fn declared_accounts(database: &str) -> Vec<AccountColumn> {
    registry()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(database)
        .cloned()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declaring_twice_changes_nothing() {
        let a = AccountColumn {
            table: "declared_a",
            column: "id",
        };
        let b = AccountColumn {
            table: "declared_b",
            column: "owner",
        };
        assert!(declared_accounts("declared-test").is_empty());
        declare_accounts("declared-test", &[a]);
        declare_accounts("declared-test", &[a, b]);
        assert_eq!(declared_accounts("declared-test"), [a, b]);
    }
}
