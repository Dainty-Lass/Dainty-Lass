// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The repositories' statement kit: values bound in the order the text names its columns, and rows declared once
//! as (column, type) lists that give the row struct, its column list and its record conversion.

use diesel::QueryableByName;
use diesel::query_dsl::RunQueryDsl;
use diesel::sql_types::{BigInt, Bool, Double, Nullable, Text};
use rifts_seam::{CapabilityScope, UnixMillis};
use serde_json::Value;
use store::{AnyConnection, MultiBackend};

use super::corrupt;
use crate::db::RepoError;
use crate::hub::PresenceState;
use crate::hunt::{AdmissionState, HuntState, OfferStatus, ScoreBreakdown, parse_scope, scope_name};
use crate::realm::{HubState, JoinPolicy, MemberRole};

/// One bound value. Every kind binds as nullable: the same parameter on both engines.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Arg {
    Text(Option<String>),
    Int(Option<i64>),
    Real(Option<f64>),
    Flag(Option<bool>),
}

/// A value a statement can bind.
pub(crate) trait Bind {
    /// What `None` of this kind binds as.
    const NULL: Arg;
    fn arg(&self) -> Arg;
}

impl<T: Bind + ?Sized> Bind for &T {
    const NULL: Arg = T::NULL;
    fn arg(&self) -> Arg {
        (**self).arg()
    }
}

impl<T: Bind> Bind for Option<T> {
    const NULL: Arg = T::NULL;
    fn arg(&self) -> Arg {
        self.as_ref().map_or(T::NULL, Bind::arg)
    }
}

impl Bind for str {
    const NULL: Arg = Arg::Text(None);
    fn arg(&self) -> Arg {
        Arg::Text(Some(self.to_owned()))
    }
}

impl Bind for String {
    const NULL: Arg = Arg::Text(None);
    fn arg(&self) -> Arg {
        self.as_str().arg()
    }
}

impl Bind for i64 {
    const NULL: Arg = Arg::Int(None);
    fn arg(&self) -> Arg {
        Arg::Int(Some(*self))
    }
}

impl Bind for UnixMillis {
    const NULL: Arg = Arg::Int(None);
    fn arg(&self) -> Arg {
        self.0.arg()
    }
}

impl Bind for bool {
    const NULL: Arg = Arg::Flag(None);
    fn arg(&self) -> Arg {
        Arg::Flag(Some(*self))
    }
}

impl Bind for f64 {
    const NULL: Arg = Arg::Real(None);
    fn arg(&self) -> Arg {
        Arg::Real(Some(*self))
    }
}

/// Stored as `,`-joined names.
impl Bind for Vec<CapabilityScope> {
    const NULL: Arg = Arg::Text(None);
    fn arg(&self) -> Arg {
        let names: Vec<&str> = self.iter().copied().map(scope_name).collect();
        names.join(",").arg()
    }
}

impl Bind for Value {
    const NULL: Arg = Arg::Text(None);
    fn arg(&self) -> Arg {
        self.to_string().arg()
    }
}

impl Bind for ScoreBreakdown {
    const NULL: Arg = Arg::Text(None);
    fn arg(&self) -> Arg {
        // Seven plain numbers: serde_json has no failure to report here.
        serde_json::to_string(self).unwrap_or_default().arg()
    }
}

/// A column's record type: how it is read (`Raw` under `Sql`) and checked back into the record.
pub(crate) trait Cell: Bind + Sized {
    type Sql;
    type Raw;
    fn load(raw: Self::Raw) -> Result<Self, RepoError>;
}

macro_rules! plain_cells {
    ($($ty:ty => $sql:ty),+ $(,)?) => {$(
        impl Cell for $ty {
            type Sql = $sql;
            type Raw = $ty;
            fn load(raw: $ty) -> Result<Self, RepoError> {
                Ok(raw)
            }
        }
    )+};
}

plain_cells!(String => Text, i64 => BigInt, bool => Bool, f64 => Double);

impl<T: Cell> Cell for Option<T> {
    type Sql = Nullable<T::Sql>;
    type Raw = Option<T::Raw>;
    fn load(raw: Self::Raw) -> Result<Self, RepoError> {
        raw.map(T::load).transpose()
    }
}

impl Cell for UnixMillis {
    type Sql = BigInt;
    type Raw = i64;
    fn load(raw: i64) -> Result<Self, RepoError> {
        Ok(UnixMillis(raw))
    }
}

impl Cell for Vec<CapabilityScope> {
    type Sql = Text;
    type Raw = String;
    fn load(raw: String) -> Result<Self, RepoError> {
        let named = raw.split(',').filter(|name| !name.is_empty());
        named
            .map(|name| parse_scope(name).ok_or_else(|| corrupt("capability scope", name)))
            .collect()
    }
}

pub(crate) fn json_error(error: serde_json::Error) -> RepoError {
    RepoError::Corrupt(error.to_string())
}

impl Cell for Value {
    type Sql = Text;
    type Raw = String;
    fn load(raw: String) -> Result<Self, RepoError> {
        serde_json::from_str(&raw).map_err(json_error)
    }
}

impl Cell for ScoreBreakdown {
    type Sql = Text;
    type Raw = String;
    fn load(raw: String) -> Result<Self, RepoError> {
        serde_json::from_str(&raw).map_err(json_error)
    }
}

/// Enums stored as their wire text; an unknown text names the column kind in the Corrupt error.
macro_rules! text_cells {
    ($($ty:ty => $what:literal),+ $(,)?) => {$(
        impl Bind for $ty {
            const NULL: Arg = Arg::Text(None);
            fn arg(&self) -> Arg {
                self.as_str().arg()
            }
        }

        impl Cell for $ty {
            type Sql = Text;
            type Raw = String;
            fn load(raw: String) -> Result<Self, RepoError> {
                <$ty>::parse(&raw).ok_or_else(|| corrupt($what, &raw))
            }
        }
    )+};
}

text_cells!(
    HuntState => "state",
    AdmissionState => "admission state",
    OfferStatus => "offer status",
    PresenceState => "hub presence state",
    MemberRole => "member role",
    JoinPolicy => "join policy",
    HubState => "hub state",
);

/// `binds![a, b]`: the values in column order.
macro_rules! binds {
    ($($value:expr),* $(,)?) => {
        vec![$($crate::repo::sql::Bind::arg(&$value)),*]
    };
}

pub(crate) use binds;

/// `table_row!(Row { column: Type, .. })` declares the row in column order, `Row::COLUMNS` with it;
/// `table_row!(Row for Record { .. })` also converts: the columns are then exactly the record's fields.
macro_rules! table_row {
    ($row:ident { $($column:ident: $ty:ty),+ $(,)? }) => {
        #[derive(diesel::QueryableByName)]
        struct $row {
            $(
                #[diesel(sql_type = <$ty as $crate::repo::sql::Cell>::Sql)]
                $column: <$ty as $crate::repo::sql::Cell>::Raw,
            )+
        }

        impl $row {
            #[allow(dead_code)]
            const COLUMNS: &'static [&'static str] = &[$(stringify!($column)),+];
        }
    };
    ($row:ident for $record:ident { $($column:ident: $ty:ty),+ $(,)? }) => {
        $crate::repo::sql::table_row!($row { $($column: $ty),+ });

        impl $row {
            /// Checked in column order, so the first bad column names the error.
            fn record(self) -> Result<$record, $crate::db::RepoError> {
                Ok($record {
                    $($column: <$ty as $crate::repo::sql::Cell>::load(self.$column)?,)+
                })
            }

            fn values(record: &$record) -> Vec<$crate::repo::sql::Arg> {
                $crate::repo::sql::binds![$(record.$column),+]
            }
        }
    };
}

pub(crate) use table_row;

/// `?, ?, ..` for `count` values.
pub(crate) fn marks(count: usize) -> String {
    vec!["?"; count].join(", ")
}

/// A statement and its values; `?` marks are numbered for the engine when it is made.
pub(crate) struct Statement {
    text: String,
    args: Vec<Arg>,
}

impl Statement {
    pub(crate) fn new(marked: &str, args: Vec<Arg>) -> Result<Self, RepoError> {
        Ok(Self::numbered(crate::db::sql(marked)?, args))
    }

    /// Text already numbered, as `store::portable` writes it.
    pub(crate) fn numbered(text: String, args: Vec<Arg>) -> Self {
        Self { text, args }
    }

    fn query(self) -> diesel::query_builder::BoxedSqlQuery<'static, MultiBackend, diesel::query_builder::SqlQuery> {
        let mut query = diesel::sql_query(self.text).into_boxed::<MultiBackend>();
        for arg in self.args {
            query = match arg {
                Arg::Text(value) => query.bind::<Nullable<Text>, _>(value),
                Arg::Int(value) => query.bind::<Nullable<BigInt>, _>(value),
                Arg::Real(value) => query.bind::<Nullable<Double>, _>(value),
                Arg::Flag(value) => query.bind::<Nullable<Bool>, _>(value),
            };
        }
        query
    }

    /// Rows changed.
    pub(crate) fn run(self, conn: &mut AnyConnection) -> Result<usize, RepoError> {
        Ok(self.query().execute(conn)?)
    }

    pub(crate) fn rows<R: QueryableByName<MultiBackend> + 'static>(
        self,
        conn: &mut AnyConnection,
    ) -> Result<Vec<R>, RepoError> {
        Ok(self.query().load::<R>(conn)?)
    }

    pub(crate) fn first<R: QueryableByName<MultiBackend> + 'static>(
        self,
        conn: &mut AnyConnection,
    ) -> Result<Option<R>, RepoError> {
        Ok(self.rows::<R>(conn)?.into_iter().next())
    }

    /// The `n` of the first row; 0 when there is none.
    pub(crate) fn number(self, conn: &mut AnyConnection) -> Result<i64, RepoError> {
        Ok(self.first::<Number>(conn)?.map_or(0, |row| row.n))
    }
}

table_row!(Number { n: i64 });

/// `store::portable::upsert` with `args` in `columns` order; a conflict rewrites every column but the key and
/// `kept`.
pub(crate) fn upsert(
    conn: &mut AnyConnection,
    table: &str,
    (columns, key, kept): (&[&str], &[&str], &[&str]),
    args: Vec<Arg>,
) -> Result<(), RepoError> {
    let rewritten: Vec<&str> = columns
        .iter()
        .copied()
        .filter(|column| !key.contains(column) && !kept.contains(column))
        .collect();
    let text = store::portable::upsert(table, columns, key, &rewritten);
    Statement::numbered(text, args).run(conn)?;
    Ok(())
}
