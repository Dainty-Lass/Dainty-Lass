// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use std::fmt;

use crate::error::{Component, Unimplemented};
use crate::id::{AccountId, SessionId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnixMillis(pub i64);

/// A value that must never reach a log, so its Debug hides it.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// Who made a stored change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    Account(AccountId),
    HostRuntime { session: SessionId, host: AccountId },
    Admin { key_id: String },
    System(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableSpec {
    pub name: &'static str,
    /// The collection the table stands for in the original Node service; documentation only.
    pub node_collection: &'static str,
    pub owner: Component,
    pub keys: &'static str,
    pub expires_by: Option<&'static str>,
}

/// What account deletion does to a row: remove it, or clear the account from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeletionAction {
    Delete,
    Detach,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeletionStep {
    pub table: &'static str,
    pub account_column: &'static str,
    pub action: DeletionAction,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    #[error(transparent)]
    Unimplemented(#[from] Unimplemented),
    #[error("{0}")]
    Database(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_debug_is_redacted() {
        let secret = Secret::new("hunter2".to_owned());
        assert_eq!(format!("{secret:?}"), "Secret(<redacted>)");
        assert_eq!(format!("{:?}", Some(secret.clone())), "Some(Secret(<redacted>))");
        assert_eq!(secret.expose(), "hunter2");
    }

    #[test]
    fn store_error_texts() {
        assert_eq!(StoreError::Database("disk full".into()).to_string(), "disk full");
        let missing = StoreError::from(Unimplemented::new(Component::Guard, "store.flush"));
        assert_eq!(missing.to_string(), "rifts-guard: store.flush is not implemented yet");
    }
}
