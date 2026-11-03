// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use std::fmt;

use crate::{
    error::{Component, Unimplemented},
    id::{AccountId, SessionId},
};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct UnixMillis(pub i64);

/// A value that must never reach a log, so its Debug hides it.
#[derive(Clone, Eq, PartialEq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(value)
    }

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
    Account(AccountId),
    HostRuntime { session: SessionId, host: AccountId },
    Admin { key_id: String },
    System(&'static str),
}

/// One table a component owns, as its schema check and the operators' docs list it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TableSpec {
    pub name: &'static str,
    /// The collection the table stands for in the original Node service; documentation only.
    pub node_collection: &'static str,
    pub owner: Component,
    pub keys: &'static str,
    pub expires_by: Option<&'static str>,
}

/// One step of account deletion: which table, which column names the account, and what happens to the row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeletionStep {
    pub table: &'static str,
    pub account_column: &'static str,
    pub action: DeletionAction,
}

/// Remove the row, or clear the account from it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeletionAction {
    Delete,
    Detach,
}

/// A store's failure: a backend not written yet, or the database's own text.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum StoreError {
    Unimplemented(Unimplemented),
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
