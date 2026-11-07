// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Bound SQL both engines run alike: written with `?`, numbered to `$1..$n` by `dainty_store::portable` (its portable
//! rule), every value bound, never spliced. No table declarations, so `launcher_accounts`' 27 columns need no
//! Diesel column-count feature.

use dainty_store::{AnyConnection, MultiBackend};
use diesel::QueryableByName;
use diesel::prelude::*;
use diesel::query_builder::{BoxedSqlQuery, SqlQuery};
use diesel::sql_types::{BigInt, Bool, Double, Nullable, Text};

use crate::{RepoError, RepoResult};

enum Param {
    Text(Option<String>),
    Int(Option<i64>),
    Float(Option<f64>),
    Bool(Option<bool>),
}

pub(crate) struct Sql {
    text: String,
    params: Vec<Param>,
}

pub(crate) fn sql(text: impl Into<String>) -> Sql {
    Sql {
        text: text.into(),
        params: Vec::new(),
    }
}

impl Sql {
    pub fn t(mut self, value: impl Into<String>) -> Self {
        self.params.push(Param::Text(Some(value.into())));
        self
    }

    pub fn ot(mut self, value: Option<impl Into<String>>) -> Self {
        self.params.push(Param::Text(value.map(Into::into)));
        self
    }

    pub fn i(mut self, value: i64) -> Self {
        self.params.push(Param::Int(Some(value)));
        self
    }

    pub fn oi(mut self, value: Option<i64>) -> Self {
        self.params.push(Param::Int(value));
        self
    }

    pub fn f(mut self, value: f64) -> Self {
        self.params.push(Param::Float(Some(value)));
        self
    }

    pub fn b(mut self, value: bool) -> Self {
        self.params.push(Param::Bool(Some(value)));
        self
    }

    /// Appends SQL text (and no binds); for clauses built from fixed pieces.
    pub fn push(mut self, text: &str) -> Self {
        self.text.push_str(text);
        self
    }

    fn query(self) -> RepoResult<BoxedSqlQuery<'static, MultiBackend, SqlQuery>> {
        let text = dainty_store::portable::numbered(&self.text).map_err(RepoError::Data)?;
        let mut query = diesel::sql_query(text).into_boxed::<MultiBackend>();
        for param in self.params {
            query = match param {
                Param::Text(v) => query.bind::<Nullable<Text>, _>(v),
                Param::Int(v) => query.bind::<Nullable<BigInt>, _>(v),
                Param::Float(v) => query.bind::<Nullable<Double>, _>(v),
                Param::Bool(v) => query.bind::<Nullable<Bool>, _>(v),
            };
        }
        Ok(query)
    }

    /// Runs a statement; how many rows it changed.
    pub fn run(self, conn: &mut AnyConnection) -> RepoResult<usize> {
        Ok(self.query()?.execute(conn)?)
    }

    pub fn load<T>(self, conn: &mut AnyConnection) -> RepoResult<Vec<T>>
    where
        T: QueryableByName<MultiBackend> + 'static,
    {
        Ok(self.query()?.load::<T>(conn)?)
    }

    pub fn first<T>(self, conn: &mut AnyConnection) -> RepoResult<Option<T>>
    where
        T: QueryableByName<MultiBackend> + 'static,
    {
        Ok(self.load::<T>(conn)?.into_iter().next())
    }

    /// A `SELECT .. AS n` of one integer.
    pub fn int(self, conn: &mut AnyConnection) -> RepoResult<i64> {
        Ok(self.first::<IntRow>(conn)?.and_then(|r| r.n).unwrap_or(0))
    }

    /// Whether the query answers a row.
    pub fn exists(self, conn: &mut AnyConnection) -> RepoResult<bool> {
        Ok(!self.load::<IntRow>(conn)?.is_empty())
    }
}

#[derive(QueryableByName)]
pub(crate) struct IntRow {
    #[diesel(sql_type = Nullable<BigInt>)]
    pub n: Option<i64>,
}

#[derive(QueryableByName)]
pub(crate) struct TextRow {
    #[diesel(sql_type = Text)]
    pub t: String,
}

/// `u64` from a row count.
pub(crate) fn count(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// ` LIMIT n` for a non-zero limit, nothing for 0 (no limit).
pub(crate) fn limit_clause(limit: u64) -> String {
    if limit == 0 {
        String::new()
    } else {
        format!(" LIMIT {}", i64::try_from(limit).unwrap_or(i64::MAX))
    }
}

/// ` LIMIT n OFFSET m` for a page; a zero limit takes every row after the skip.
pub(crate) fn page_clause(skip: u64, limit: u64) -> String {
    let skip = i64::try_from(skip).unwrap_or(i64::MAX);
    match limit {
        // Postgres takes LIMIT ALL, SQLite LIMIT -1; an OFFSET alone is valid on Postgres only
        0 if skip == 0 => String::new(),
        0 => format!(" LIMIT {} OFFSET {skip}", i64::MAX),
        n => format!(" LIMIT {} OFFSET {skip}", i64::try_from(n).unwrap_or(i64::MAX)),
    }
}

/// The text of a stored account id, or the data error `{what} "<text>" is not an account id`.
pub(crate) fn account_id(text: &str, what: &str) -> RepoResult<crate::AccountId> {
    text.parse()
        .map_err(|_| RepoError::data(format!("{what} \"{text}\" is not an account id")))
}

pub(crate) fn opt_account_id(text: Option<&str>, what: &str) -> RepoResult<Option<crate::AccountId>> {
    text.map(|t| account_id(t, what)).transpose()
}
