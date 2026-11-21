// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! `db move`: a database copied from one engine to the other, for a host that switches (DD15).
//!
//! Each row travels as JSON the source writes and the destination reads back (`json_object` and
//! `json_extract` on SQLite, `to_jsonb` and `jsonb_populate_record` on Postgres), so no Rust type has to
//! name the columns. The change log moves with the data, ids and all; the destination's triggers are off
//! while the rows go in, so the copy adds no entries. The source is read in one snapshot, the destination
//! written in one transaction: a failed move leaves it as empty as it was.

use std::collections::BTreeMap;

use diesel::connection::{Connection as _, SimpleConnection};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Nullable, Text};

use crate::connection::AnyConnection;
use crate::database::applied_versions;
use crate::history::{self, CHANGE_LOG, Column};
use crate::portable::{quote_ident, quote_literal};
use crate::{Database, DatabaseConfig, DatabaseOptions, DatabaseSchema, Engine, Error};

/// Rows read and written per round trip.
pub(crate) const PAGE: i64 = 1000;

#[derive(QueryableByName)]
pub(crate) struct Row {
    #[diesel(sql_type = Text)]
    pub row: String,
}

/// What a move copied: rows per table, the change log last.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MoveReport {
    /// Rows copied per table, in copy order.
    pub tables: Vec<(String, usize)>,
}

pub(crate) fn is_blob(column: &Column) -> bool {
    column.kind.contains("blob") || column.kind == "bytea"
}

pub(crate) fn select_rows(conn: &mut AnyConnection, table: &str, columns: &[Column], key: &[String]) -> String {
    let order = key.iter().map(|k| quote_ident(k)).collect::<Vec<_>>().join(", ");
    let t = quote_ident(table);
    match conn.engine() {
        Engine::Sqlite => format!(
            "SELECT {} AS row FROM {t} ORDER BY {order} LIMIT $1 OFFSET $2",
            history::sqlite_json_object("", columns.iter().map(|c| c.name.as_str()))
        ),
        Engine::Postgres => {
            format!("SELECT to_jsonb(r)::text AS row FROM {t} AS r ORDER BY {order} LIMIT $1 OFFSET $2")
        }
    }
}

pub(crate) fn insert_row(conn: &mut AnyConnection, table: &str, columns: &[Column]) -> String {
    let t = quote_ident(table);
    match conn.engine() {
        Engine::Sqlite => {
            let names = columns
                .iter()
                .map(|c| quote_ident(&c.name))
                .collect::<Vec<_>>()
                .join(", ");
            let values: Vec<String> = columns
                .iter()
                .map(|c| {
                    let path = quote_literal(&format!("$.{}", quote_ident(&c.name)));
                    if is_blob(c) {
                        format!("unhex(substr(json_extract($1, {path}), 3))")
                    } else {
                        format!("json_extract($1, {path})")
                    }
                })
                .collect();
            format!("INSERT INTO {t} ({names}) VALUES ({})", values.join(", "))
        }
        Engine::Postgres => {
            format!("INSERT INTO {t} SELECT * FROM jsonb_populate_record(NULL::{t}, $1::jsonb)",)
        }
    }
}

/// Parents before children, so foreign keys hold row by row; a cycle keeps the order it has.
pub(crate) fn ordered(conn: &mut AnyConnection, tables: Vec<String>) -> Result<Vec<String>, Error> {
    let mut parents = BTreeMap::new();
    for table in &tables {
        let mut of = history::parents(conn, table)?;
        of.retain(|p| p != table && tables.contains(p));
        parents.insert(table.clone(), of);
    }
    let mut out: Vec<String> = Vec::with_capacity(tables.len());
    let mut left = tables;
    while !left.is_empty() {
        let ready: Vec<String> = left
            .iter()
            .filter(|t| parents.get(*t).is_none_or(|ps| ps.iter().all(|p| out.contains(p))))
            .cloned()
            .collect();
        let ready = if ready.is_empty() { left.clone() } else { ready };
        left.retain(|t| !ready.contains(t));
        out.extend(ready);
    }
    Ok(out)
}

#[derive(QueryableByName)]
struct Sequence {
    #[diesel(sql_type = Nullable<Text>)]
    seq: Option<String>,
}

/// Postgres: each serial or identity column's sequence moved past the copied ids.
pub(crate) fn fix_sequences(conn: &mut AnyConnection, table: &str, columns: &[Column]) -> Result<(), Error> {
    for column in columns {
        let found = diesel::sql_query("SELECT pg_get_serial_sequence($1, $2) AS seq")
            .bind::<Text, _>(quote_ident(table))
            .bind::<Text, _>(column.name.clone())
            .get_result::<Sequence>(conn)?;
        if let Some(seq) = found.seq {
            conn.batch_execute(&format!(
                "SELECT setval({}, COALESCE((SELECT MAX({}) FROM {}), 0) + 1, false)",
                quote_literal(&seq),
                quote_ident(&column.name),
                quote_ident(table)
            ))?;
        }
    }
    Ok(())
}

fn copy_table(
    from: &mut AnyConnection,
    to: &mut AnyConnection,
    table: &str,
    sanity: &dyn Fn(&str) -> Result<(), Error>,
) -> Result<usize, Error> {
    sanity(table)?;
    let key = history::primary_key(to, table)?;
    let to_columns = history::columns(to, table)?;
    let from_columns = history::columns(from, table)?;
    let names = |cs: &[Column]| cs.iter().map(|c| c.name.clone()).collect::<Vec<_>>();
    if names(&to_columns) != names(&from_columns) {
        return Err(Error::Move(format!(
            "{table} has the columns {:?} here and {:?} there",
            names(&from_columns),
            names(&to_columns)
        )));
    }
    let select = select_rows(from, table, &from_columns, &key);
    let insert = insert_row(to, table, &to_columns);
    let mut copied = 0usize;
    loop {
        let rows = diesel::sql_query(&select)
            .bind::<BigInt, _>(PAGE)
            .bind::<BigInt, _>(i64::try_from(copied).unwrap_or(i64::MAX))
            .load::<Row>(from)?;
        for row in &rows {
            diesel::sql_query(&insert)
                .bind::<Text, _>(row.row.clone())
                .execute(to)?;
        }
        copied += rows.len();
        if rows.len() < usize::try_from(PAGE).unwrap_or(usize::MAX) {
            break;
        }
    }
    if to.engine() == Engine::Postgres {
        fix_sequences(to, table, &to_columns)?;
    }
    Ok(copied)
}

fn user_rows(conn: &mut AnyConnection) -> Result<i64, Error> {
    let mut total = 0;
    for table in history::user_tables(conn)? {
        #[derive(QueryableByName)]
        struct Count {
            #[diesel(sql_type = BigInt)]
            n: i64,
        }
        total += diesel::sql_query(format!("SELECT count(*) AS n FROM {}", quote_ident(&table)))
            .get_result::<Count>(conn)?
            .n;
    }
    Ok(total)
}

/// Copies the database at `from` into `to`, on the other engine or the same. `to` is created with the
/// schema's migrations and must hold no rows yet; both must end up at the same migration. Blocking.
pub fn move_database(
    from: &DatabaseConfig,
    to: &DatabaseConfig,
    schema: &DatabaseSchema,
    options: &DatabaseOptions,
) -> Result<MoveReport, Error> {
    let source = Database::open(from, options, schema)?;
    let destination = Database::open(to, options, schema)?;
    let (_, source_read) = source.pools();
    let (destination_write, _) = destination.pools();
    let mut src = source_read.get()?;
    let mut dst = destination_write.get()?;
    if applied_versions(&mut src)? != applied_versions(&mut dst)? {
        return Err(Error::Move("the two databases are at different migrations".into()));
    }
    // Rows, or any log entry beyond the destination's own migrations, mean it isn't a fresh database.
    let own_entries = history::changes(&mut dst, 0, i64::MAX)?
        .iter()
        .any(|e| e.op != "migrate");
    if user_rows(&mut dst)? > 0 || own_entries {
        return Err(Error::Move(
            "the destination already holds data: move into an empty database".into(),
        ));
    }
    let src_engine = src.engine();
    let mut report = MoveReport::default();
    (*src).transaction(|src| {
        if src_engine == Engine::Postgres {
            src.batch_execute("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")?;
        }
        (*dst).transaction(|dst| {
            history::drop_triggers(dst)?;
            // The destination's own migration entries give way to the source's history.
            dst.batch_execute(&match dst.engine() {
                Engine::Sqlite => format!(
                    "DROP TRIGGER IF EXISTS \"__store_change_log_no_delete\"; DELETE FROM {log};",
                    log = quote_ident(CHANGE_LOG)
                ),
                Engine::Postgres => format!(
                    "ALTER TABLE {log} DISABLE TRIGGER \"__store_append_only\"; DELETE FROM {log};
                     ALTER TABLE {log} ENABLE TRIGGER \"__store_append_only\";",
                    log = quote_ident(CHANGE_LOG)
                ),
            })?;
            let found = history::user_tables(dst)?;
            let tables = ordered(dst, found)?;
            let present = history::user_tables(src)?;
            let sanity = |table: &str| {
                if present.iter().any(|t| t == table) {
                    Ok(())
                } else {
                    Err(Error::Move(format!("the source has no table {table}")))
                }
            };
            for table in tables.iter().map(String::as_str).chain([CHANGE_LOG]) {
                let check = |t: &str| if t == CHANGE_LOG { Ok(()) } else { sanity(t) };
                let copied = copy_table(src, dst, table, &check)?;
                report.tables.push((table.to_owned(), copied));
            }
            history::ensure_tables(dst)?;
            history::install_triggers(dst, schema.soft_delete)?;
            Ok::<_, Error>(())
        })
    })?;
    Ok(report)
}

/// Every table's rows (the change log's included) as JSON values, sorted, with booleans as 0 and 1 as
/// SQLite stores them: two databases holding the same data dump equal, whatever their engines.
pub fn dump(config: &DatabaseConfig) -> Result<BTreeMap<String, Vec<serde_json::Value>>, Error> {
    let manager_config = config.clone();
    let mut conn = match &manager_config {
        DatabaseConfig::Sqlite { file } => {
            AnyConnection::Sqlite(diesel::SqliteConnection::establish(&file.to_string_lossy())?)
        }
        DatabaseConfig::Postgres(pg) => AnyConnection::Postgres(crate::PgConnection::establish(&pg.conninfo())?),
    };
    let mut out = BTreeMap::new();
    let mut tables = history::user_tables(&mut conn)?;
    tables.push(CHANGE_LOG.to_owned());
    for table in tables {
        let key = history::primary_key(&mut conn, &table)?;
        let columns = history::columns(&mut conn, &table)?;
        let sql = select_rows(&mut conn, &table, &columns, &key);
        let rows = diesel::sql_query(sql)
            .bind::<BigInt, _>(i64::MAX)
            .bind::<BigInt, _>(0)
            .load::<Row>(&mut conn)?;
        let mut values = rows
            .iter()
            .map(|r| serde_json::from_str(&r.row).map(normalized))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| Error::Move(format!("{table}: {e}")))?;
        values.sort_by_key(ToString::to_string);
        out.insert(table, values);
    }
    Ok(out)
}

fn normalized(value: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::Bool(b) => Value::from(i64::from(b)),
        // Keys sorted whether or not serde_json keeps insertion order (preserve_order, from service-kit).
        Value::Object(map) => {
            let sorted: BTreeMap<String, Value> = map.into_iter().map(|(k, v)| (k, normalized(v))).collect();
            Value::Object(sorted.into_iter().collect())
        }
        other => other,
    }
}
