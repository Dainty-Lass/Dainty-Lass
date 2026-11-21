// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

use std::fmt;

use crate::{
    error::{Component, Unimplemented},
    id::{AccountId, SessionId},
};

/// Milliseconds since the Unix epoch.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct UnixMillis(pub i64);

/// A value that must never reach a log, so its Debug hides it.
#[derive(Clone, Eq, PartialEq)]
pub struct Secret(String);

impl Secret {
    /// Wraps a value so it never reaches a log.
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// The hidden value, for the one place that needs it.
    pub fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// Who made a stored change.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Actor {
    /// A player through the launcher.
    Account(AccountId),
    /// A hosted runtime acting for its host.
    HostRuntime {
        /// The hosted session.
        session: SessionId,
        /// The player hosting it.
        host: AccountId,
    },
    /// An operator with an admin key.
    Admin {
        /// The admin key's id, never the key.
        key_id: String,
    },
    /// Rifts itself, named by the job that made the change.
    System(&'static str),
}

/// One table a component owns, as its schema check and the operators' docs list it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TableSpec {
    /// The table's name.
    pub name: &'static str,
    /// The collection the table stands for in the original Node service; documentation only.
    pub node_collection: &'static str,
    /// The component that owns it.
    pub owner: Component,
    /// Its keys and indexes in short form for the docs, such as `realm + account; name unique`.
    pub keys: &'static str,
    /// The column whose time expires a row, if rows expire.
    pub expires_by: Option<&'static str>,
}

/// One step of account deletion: which table, which column names the account, and what happens to the row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeletionStep {
    /// The table.
    pub table: &'static str,
    /// The column that holds the account.
    pub account_column: &'static str,
    /// What happens to the account's rows.
    pub action: DeletionAction,
}

/// Remove the row, or clear the account from it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeletionAction {
    /// Remove the row.
    Delete,
    /// Clear the account from the row and keep it.
    Detach,
}

/// A store's failure: a backend not written yet, or the database's own text.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum StoreError {
    /// The backend is not written yet.
    Unimplemented(Unimplemented),
    /// The database failed, with its own text.
    Database(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(text) => f.write_str(text),
            Self::Unimplemented(missing) => fmt::Display::fmt(missing, f),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<Unimplemented> for StoreError {
    fn from(missing: Unimplemented) -> Self {
        Self::Unimplemented(missing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_debug_is_redacted() {
        let hidden = Secret::new(String::from("hunter2"));
        assert_eq!(hidden.expose(), "hunter2");
        for (shown, want) in [
            (format!("{hidden:?}"), "Secret(<redacted>)"),
            (format!("{:?}", Some(hidden.clone())), "Some(Secret(<redacted>))"),
        ] {
            assert_eq!(shown, want);
        }
    }

    #[test]
    fn store_error_texts() {
        let cases = [
            (StoreError::Database("disk full".into()), "disk full"),
            (
                Unimplemented::new(Component::Guard, "store.flush").into(),
                "rifts-guard: store.flush is not implemented yet",
            ),
        ];
        for (error, text) in cases {
            assert_eq!(error.to_string(), text);
        }
    }
}
