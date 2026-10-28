//! Storage shapes for RF5 and RF6, before DaintyLass's `store` is published.
//!
//! Node keeps realms in MongoDB only (RF1, section 7). Here each component lists its tables ([`TableSpec`]) and
//! declares a repository trait whose methods are plain SQL-shaped reads and writes: no Mongo operators, no document
//! updates by path, so one implementation fits DF7's two engines (DD15). Every write takes the [`Actor`] that DH1's
//! change log records. Once `store` publishes, the repositories are implemented on it, migrations replace
//! `TableSpec`, and `Actor` becomes its actor context if it carries one (then the parameter goes). Multi-table writes
//! (Node's `GetUnitOfWork`, spanning core and module collections) wait on store's transaction API.

use std::fmt;

use crate::error::{Component, Unimplemented};
use crate::id::{AccountId, SessionId};

/// Unix time in milliseconds (Node's `Date`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnixMillis(pub i64);

/// A secret value: never in `Debug`, so never in a log (AGENTS.md).
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

/// Who made a change, for the change log (DD23, DH1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    Account(AccountId),
    /// A hosted game server or transport acting through its host capability.
    HostRuntime {
        session: SessionId,
        host: AccountId,
    },
    Admin {
        key_id: String,
    },
    /// A sweeper or startup task, by name.
    System(&'static str),
}

/// A table a component owns, with the Node collection it replaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableSpec {
    pub name: &'static str,
    pub node_collection: &'static str,
    pub owner: Component,
    /// The key and unique indexes, as RF1 lists them.
    pub keys: &'static str,
    /// Rows past this time are purged (a Mongo TTL index today); a sweeper does it on both engines.
    pub expires_by: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeletionAction {
    Delete,
    /// The row stays for the other accounts in it; the account's fields are cleared.
    Detach,
}

/// One step of the account deletion plan (`accountDeletionPlan.ts`).
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
    fn secrets_stay_out_of_debug() {
        let secret = Secret::new("RRK_do-not-log".to_owned());
        assert_eq!(format!("{secret:?}"), "Secret(<redacted>)");
        assert_eq!(secret.expose(), "RRK_do-not-log");
    }
}
