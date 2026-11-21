// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The repositories. Every method that changes data is one `Database::write` with its precondition read inside it:
//! `store` serialises writes on both engines, which stands in for Node's guarded updates and hub gates.

pub mod hub;
pub mod hunts;
pub mod realms;
pub(crate) mod sql;

pub use rustic_rifts_guard::SqlGuardStore;

use crate::db::RepoError;

/// `what "value"`, as the Corrupt text names a bad enum.
pub(crate) fn corrupt(what: &str, value: &str) -> RepoError {
    RepoError::Corrupt(format!("{what} {value:?}"))
}
