// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The repositories. Every method that changes data is one `Database::write` with its precondition read inside it:
//! `store` serialises writes on both engines, which stands in for Node's guarded updates and hub gates.

pub mod hub;
pub mod hunts;
pub mod realms;

pub use rifts_guard::SqlGuardStore;

use diesel::QueryableByName;
use diesel::query_builder::{BoxedSqlQuery, SqlQuery};
use diesel::sql_types::BigInt;
use store::MultiBackend;

use crate::db::RepoError;

pub(crate) type Boxed = BoxedSqlQuery<'static, MultiBackend, SqlQuery>;

pub(crate) fn query(text: &str) -> Result<Boxed, RepoError> {
    Ok(diesel::sql_query(crate::db::sql(text)?).into_boxed::<MultiBackend>())
}

#[derive(QueryableByName)]
pub(crate) struct Count {
    #[diesel(sql_type = BigInt)]
    pub n: i64,
}

/// `what "value"`, as the Corrupt text names a bad enum.
pub(crate) fn corrupt(what: &str, value: &str) -> RepoError {
    RepoError::Corrupt(format!("{what} {value:?}"))
}

pub(crate) fn placeholders(count: usize) -> String {
    vec!["?"; count].join(", ")
}

pub(crate) fn limit(value: u32) -> i64 {
    i64::from(value)
}
