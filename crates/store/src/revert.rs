// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! History and revert (DD23, DH3a): the change log queried by account, actor or time, and a selection of
//! it undone by compensating changes.
//!
//! A revert is planned, then applied. [`dry_run`] works out, for every row the selection touched, the change
//! that puts it back as it was before the selection's first entry on it: an inserted row is deleted
//! (soft-deleted on a soft-delete table), a changed one gets its before row back, a deleted one comes back.
//! A row changed since by an entry outside the selection is a conflict: reported, never overwritten (plan
//! section 6). [`apply`] runs the plan as one write of the reverting actor, so the revert is logged and can
//! be reverted in turn. It plans again under the write lock and refuses a plan whose id no longer matches:
//! what is applied is exactly what the dry run showed.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Text};
use serde::{Serialize, Serializer};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::connection::{AnyConnection, MultiBackend};
use crate::database::Tx;
use crate::history::{self, Actor, CHANGE_LOG, ChangeEntry, Column, DELETED_AT};
use crate::portable::{quote_ident, quote_literal};
use crate::transfer::{is_blob, ordered};
use crate::{Database, Engine, Error};

/// The most entries one history page holds.
pub const MAX_PAGE: i64 = 1000;

/// A column naming the account a table's row belongs to: the accounts table's own key, a child's
/// `account_id`. [`Filter::account`] matches it in the row before or after each change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountColumn {
    /// Table the column is in.
    pub table: &'static str,
    /// Column holding the account id.
    pub column: &'static str,
}

/// Which entries a history page or a revert covers. Every field set must match.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Filter {
    /// Rows whose [`AccountColumn`] holds this id, before or after the change.
    pub account: Option<String>,
    /// Who made the change.
    pub actor: Option<Actor>,
    /// Unix milliseconds, inclusive.
    pub since: Option<i64>,
    /// Unix milliseconds, exclusive.
    pub until: Option<i64>,
}

impl Serialize for Actor {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.as_text())
    }
}

/// Where the next history page starts. Opaque to callers: text out, text back in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor(i64);

impl fmt::Display for Cursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Cursor {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self, Error> {
        match text.parse::<i64>() {
            Ok(id) if id > 0 && text.bytes().all(|b| b.is_ascii_digit()) => Ok(Self(id)),
            _ => Err(Error::Revert(format!("not a history cursor: {text:?}"))),
        }
    }
}

/// Entries newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryPage {
    /// The page's entries.
    pub entries: Vec<ChangeEntry>,
    /// Set when the page is full: there may be older entries.
    pub next: Option<Cursor>,
}

/// What a compensating change does to a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RevertKind {
    /// A row the selection deleted, inserted again.
    Insert,
    /// A row the selection changed (soft deletes and restores included), given its before row.
    Update,
    /// A row the selection inserted, deleted.
    Delete,
    /// A row the selection inserted into a soft-delete table, soft-deleted.
    SoftDelete,
}

/// One compensating change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RevertAction {
    /// Table the row is in.
    pub table: String,
    /// The primary key as a JSON object, as the log names the row.
    pub key: Value,
    /// What the compensating change does.
    pub kind: RevertKind,
    /// The row now, and as the revert leaves it; null when absent (a soft delete only sets `deleted_at`).
    pub current: Option<Value>,
    /// The row as the revert leaves it; null when absent.
    pub target: Option<Value>,
    /// The selection's entries on this row.
    pub entries: Vec<i64>,
}

/// Why a row is left as it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Conflict {
    /// Changed since the selection's first entry on it by an entry outside the selection: the first one.
    ChangedSince {
        /// The entry that changed it.
        entry: i64,
        /// Who made that change.
        actor: String,
        /// When, in unix milliseconds.
        at: i64,
    },
    /// Not as its last entry left it: a migration changed it, or a write around the log.
    NotAsLogged,
    /// A change in its history moved its primary key, so the log names it under two keys.
    KeyChanged,
    /// Its table, a column or its key isn't what the log recorded.
    SchemaChanged {
        /// What differs.
        detail: String,
    },
}

/// A row the revert leaves as it is, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RowConflict {
    /// Table the row is in.
    pub table: String,
    /// The primary key as a JSON object.
    pub key: Value,
    /// The selection's entries on this row.
    pub entries: Vec<i64>,
    /// The row now; null when absent.
    pub current: Option<Value>,
    /// Why it is left.
    #[serde(flatten)]
    pub conflict: Conflict,
}

/// A row already as the revert would leave it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RowRef {
    /// Table the row is in.
    pub table: String,
    /// The primary key as a JSON object.
    pub key: Value,
    /// The selection's entries on this row.
    pub entries: Vec<i64>,
}

/// What a revert would do, from [`dry_run`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RevertPlan {
    /// SHA-256 of everything below, in hex: [`apply`] runs the plan only while it is still this one.
    pub id: String,
    /// The entries the plan reverts.
    pub selection: Filter,
    /// How many log entries the selection holds.
    pub entries: usize,
    /// In the order [`apply`] runs them: rows back parents first, changed rows, rows out children first.
    pub actions: Vec<RevertAction>,
    /// Rows left as they are, with why.
    pub conflicts: Vec<RowConflict>,
    /// Rows already as the revert would leave them.
    pub unchanged: Vec<RowRef>,
}

/// What [`Database::revert_apply`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RevertOutcome {
    /// The revert's write time: its entries carry it, with the reverting actor.
    pub at: i64,
    /// The plan that ran.
    pub plan: RevertPlan,
}

// ------------------------------------------------------------------ queries

enum Bind {
    Int(i64),
    Text(String),
}

/// Conditions with `$n` numbered as they are pushed, which is their order in the text (portable.rs).
#[derive(Default)]
struct Where {
    sql: Vec<String>,
    binds: Vec<Bind>,
}

impl Where {
    fn param(&mut self, bind: Bind) -> String {
        self.binds.push(bind);
        format!("${}", self.binds.len())
    }

    fn clause(&self) -> String {
        if self.sql.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", self.sql.join(" AND "))
        }
    }
}

fn boxed(
    sql: String,
    binds: Vec<Bind>,
) -> diesel::query_builder::BoxedSqlQuery<'static, MultiBackend, diesel::query_builder::SqlQuery> {
    let mut query = diesel::sql_query(sql).into_boxed::<MultiBackend>();
    for bind in binds {
        query = match bind {
            Bind::Int(n) => query.bind::<BigInt, _>(n),
            Bind::Text(t) => query.bind::<Text, _>(t),
        };
    }
    query
}

// `SELECT *`: `ChangeEntry` takes its columns by name, so a column added to the log (DH5) leaves this as it is.
fn load(conn: &mut AnyConnection, filter: Where, tail: &str) -> Result<Vec<ChangeEntry>, Error> {
    let sql = format!("SELECT * FROM {}{}{tail}", quote_ident(CHANGE_LOG), filter.clause());
    Ok(boxed(sql, filter.binds).load(conn)?)
}

fn check_accounts(conn: &mut AnyConnection, accounts: &[AccountColumn]) -> Result<(), Error> {
    if accounts.is_empty() {
        return Err(Error::Schema(
            "no account columns declared (Database::with_account_columns)".into(),
        ));
    }
    for account in accounts {
        if !history::columns(conn, account.table)?
            .iter()
            .any(|c| c.name == account.column)
        {
            return Err(Error::Schema(format!(
                "the account column {}.{} doesn't exist",
                account.table, account.column
            )));
        }
    }
    Ok(())
}

fn conditions(
    conn: &mut AnyConnection,
    filter: &Filter,
    accounts: &[AccountColumn],
    w: &mut Where,
) -> Result<(), Error> {
    if let Some(actor) = &filter.actor {
        let p = w.param(Bind::Text(actor.as_text()));
        w.sql.push(format!("\"actor\" = {p}"));
    }
    if let Some(since) = filter.since {
        let p = w.param(Bind::Int(since));
        w.sql.push(format!("\"at\" >= {p}"));
    }
    if let Some(until) = filter.until {
        let p = w.param(Bind::Int(until));
        w.sql.push(format!("\"at\" < {p}"));
    }
    if let Some(account) = &filter.account {
        check_accounts(conn, accounts)?;
        let engine = conn.engine();
        let p = w.param(Bind::Text(account.clone()));
        let field = |side: &str, column: &str| match engine {
            Engine::Sqlite => format!(
                "CAST(json_extract(\"{side}\", {}) AS TEXT)",
                quote_literal(&format!("$.{}", quote_ident(column)))
            ),
            Engine::Postgres => format!("(\"{side}\"::jsonb ->> {})", quote_literal(column)),
        };
        let any: Vec<String> = accounts
            .iter()
            .map(|a| {
                format!(
                    "(\"table_name\" = {} AND ({} = {p} OR {} = {p}))",
                    quote_literal(a.table),
                    field("before", a.column),
                    field("after", a.column)
                )
            })
            .collect();
        w.sql.push(format!("({})", any.join(" OR ")));
    }
    Ok(())
}

/// One page of the entries `filter` matches, newest first, starting after `cursor`.
pub fn history(
    conn: &mut AnyConnection,
    filter: &Filter,
    accounts: &[AccountColumn],
    cursor: Option<Cursor>,
    limit: i64,
) -> Result<HistoryPage, Error> {
    let limit = limit.clamp(1, MAX_PAGE);
    let mut w = Where::default();
    conditions(conn, filter, accounts, &mut w)?;
    if let Some(Cursor(id)) = cursor {
        let p = w.param(Bind::Int(id));
        w.sql.push(format!("\"id\" < {p}"));
    }
    // LIMIT's placeholder comes after every condition's, as its number does.
    let p = format!("${}", w.binds.len() + 1);
    w.binds.push(Bind::Int(limit));
    let entries = load(conn, w, &format!(" ORDER BY \"id\" DESC LIMIT {p}"))?;
    let full = i64::try_from(entries.len()).unwrap_or(i64::MAX) >= limit;
    let next = entries.last().filter(|_| full).map(|e| Cursor(e.id));
    Ok(HistoryPage { entries, next })
}

// ------------------------------------------------------------------ rows

#[derive(QueryableByName)]
struct RowText {
    #[diesel(sql_type = Text)]
    row: String,
}

struct TableInfo {
    columns: Vec<Column>,
    key: Vec<String>,
    soft: bool,
}

impl TableInfo {
    fn read(conn: &mut AnyConnection, table: &str, soft_delete: &[&str]) -> Result<Self, Error> {
        Ok(Self {
            columns: history::columns(conn, table)?,
            key: history::primary_key(conn, table)?,
            soft: soft_delete.contains(&table),
        })
    }

    fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name)
    }
}

/// A SQLite value out of the JSON in `$param`, a blob back from the `\x` hex the log writes.
fn sqlite_value(column: &Column, param: usize) -> String {
    let path = quote_literal(&format!("$.{}", quote_ident(&column.name)));
    if is_blob(column) {
        format!("unhex(substr(json_extract(${param}, {path}), 3))")
    } else {
        format!("json_extract(${param}, {path})")
    }
}

fn idents<'a>(names: impl IntoIterator<Item = &'a str>) -> String {
    names.into_iter().map(quote_ident).collect::<Vec<_>>().join(", ")
}

/// Postgres reads a row out of JSON with the table's own types; the log's JSON is what `to_jsonb` wrote.
fn pg_record(table: &str, param: usize) -> String {
    format!("jsonb_populate_record(NULL::{}, ${param}::jsonb)", quote_ident(table))
}

/// `WHERE` text matching the row whose key is the JSON object in `$param`.
fn key_match(engine: Engine, table: &str, info: &TableInfo, param: usize) -> String {
    match engine {
        Engine::Sqlite => info
            .key
            .iter()
            .filter_map(|k| info.column(k))
            .map(|c| format!("{} = {}", quote_ident(&c.name), sqlite_value(c, param)))
            .collect::<Vec<_>>()
            .join(" AND "),
        Engine::Postgres => {
            let key = idents(info.key.iter().map(String::as_str));
            format!("({key}) = (SELECT {key} FROM {})", pg_record(table, param))
        }
    }
}

/// The row as the triggers write it, or `None` when there is none.
fn read_row(conn: &mut AnyConnection, table: &str, info: &TableInfo, key: &Value) -> Result<Option<Value>, Error> {
    let engine = conn.engine();
    let t = quote_ident(table);
    let sql = match engine {
        Engine::Sqlite => format!(
            "SELECT {} AS row FROM {t} WHERE {}",
            history::sqlite_json_object("", info.columns.iter().map(|c| c.name.as_str())),
            key_match(engine, table, info, 1)
        ),
        Engine::Postgres => format!(
            "SELECT to_jsonb(r)::text AS row FROM {t} AS r WHERE {}",
            key_match(engine, table, info, 1)
        ),
    };
    let rows = boxed(sql, vec![Bind::Text(key.to_string())]).load::<RowText>(conn)?;
    match rows.as_slice() {
        [] => Ok(None),
        [one] => parse(&one.row, table).map(Some),
        _ => Err(Error::Revert(format!(
            "{table} {key}: the key matches more than one row"
        ))),
    }
}

fn parse(text: &str, what: &str) -> Result<Value, Error> {
    serde_json::from_str(text).map_err(|e| Error::Revert(format!("{what}: {e}")))
}

fn parse_opt(text: Option<&str>, what: &str) -> Result<Option<Value>, Error> {
    text.map(|t| parse(t, what)).transpose()
}

/// For comparing: booleans as SQLite stores them, whole floats as integers (a row that moved engines
/// writes `1.0` on one and `1` on the other), keys sorted.
fn normalized(value: &Value) -> Value {
    match value {
        Value::Bool(b) => Value::from(i64::from(*b)),
        Value::Number(n) if n.is_f64() => match n.as_f64() {
            #[allow(
                clippy::cast_possible_truncation,
                reason = "the guard keeps it whole and well inside i64"
            )]
            Some(f) if f.fract() == 0.0 && f.abs() < 9.0e15 => Value::from(f as i64),
            _ => value.clone(),
        },
        Value::Array(items) => Value::Array(items.iter().map(normalized).collect()),
        Value::Object(map) => {
            let sorted: BTreeMap<&String, Value> = map.iter().map(|(k, v)| (k, normalized(v))).collect();
            Value::Object(sorted.into_iter().map(|(k, v)| (k.clone(), v)).collect())
        }
        other => other.clone(),
    }
}

/// Whether `row` holds every column of `wanted` with the same value. Columns only `row` has (one a
/// migration added since) don't count: the revert leaves them alone.
fn holds(row: &Value, wanted: &Value) -> bool {
    match wanted.as_object() {
        Some(fields) => fields
            .iter()
            .all(|(k, v)| row.get(k).is_some_and(|r| normalized(r) == normalized(v))),
        None => false,
    }
}

fn is_gone(row: &Value, soft: bool) -> bool {
    soft && row.get(DELETED_AT).is_some_and(|d| !d.is_null())
}

fn key_of(row: &Value, key: &[String]) -> Vec<Value> {
    key.iter().map(|k| row.get(k).map_or(Value::Null, normalized)).collect()
}

// ------------------------------------------------------------------ the plan

/// A row the selection touched: its selected entries, oldest first.
struct Touched {
    table: String,
    key: Value,
    canon: String,
    entries: Vec<ChangeEntry>,
}

enum Verdict {
    Act(RevertKind),
    Conflict(Conflict),
    Unchanged,
}

fn verdict(
    conn: &mut AnyConnection,
    row: &Touched,
    info: &TableInfo,
    later: &[ChangeEntry],
    selected: &HashSet<i64>,
) -> Result<(Verdict, Option<Value>, Option<Value>), Error> {
    let table = &row.table;
    let Some(first) = row.entries.first() else {
        return Ok((Verdict::Unchanged, None, None));
    };
    if info.columns.is_empty() {
        let detail = format!("the table {table} is gone");
        return Ok((Verdict::Conflict(Conflict::SchemaChanged { detail }), None, None));
    }
    let logged: BTreeMap<&String, &Value> = row.key.as_object().map(|m| m.iter().collect()).unwrap_or_default();
    if logged.len() != info.key.len() || !info.key.iter().all(|k| logged.contains_key(k)) {
        let detail = format!("{table}'s primary key is now ({})", info.key.join(", "));
        return Ok((Verdict::Conflict(Conflict::SchemaChanged { detail }), None, None));
    }
    // The log names an updated row by its new key: a moved key splits the row's history in two.
    let history = row.entries.iter().chain(later.iter());
    for entry in history {
        if let (Some(before), Some(after)) = (entry.before.as_deref(), entry.after.as_deref()) {
            let what = format!("entry {}", entry.id);
            if key_of(&parse(before, &what)?, &info.key) != key_of(&parse(after, &what)?, &info.key) {
                return Ok((Verdict::Conflict(Conflict::KeyChanged), None, None));
            }
        }
    }

    let target = parse_opt(first.before.as_deref(), &format!("entry {}", first.id))?;
    let current = read_row(conn, table, info, &row.key)?;
    let already = match (&current, &target) {
        (None, None) => true,
        (Some(now), None) => is_gone(now, info.soft),
        (None, Some(_)) => false,
        (Some(now), Some(then)) => holds(now, then),
    };
    if already {
        return Ok((Verdict::Unchanged, current, target));
    }
    if let Some(outside) = later.iter().find(|e| !selected.contains(&e.id)) {
        let conflict = Conflict::ChangedSince {
            entry: outside.id,
            actor: outside.actor.clone(),
            at: outside.at,
        };
        return Ok((Verdict::Conflict(conflict), current, target));
    }
    let last = later.last().unwrap_or(first);
    let last_after = parse_opt(last.after.as_deref(), &format!("entry {}", last.id))?;
    let as_logged = match (&current, &last_after) {
        (None, None) => true,
        (Some(now), Some(after)) => holds(now, after),
        _ => false,
    };
    if !as_logged {
        return Ok((Verdict::Conflict(Conflict::NotAsLogged), current, target));
    }
    if let Some(gone) = target
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|fields| fields.keys().find(|k| info.column(k).is_none()))
    {
        let detail = format!("the column {table}.{gone} is gone");
        return Ok((Verdict::Conflict(Conflict::SchemaChanged { detail }), current, target));
    }
    let kind = match (&current, &target) {
        (Some(_), None) if info.soft => RevertKind::SoftDelete,
        (Some(_), None) => RevertKind::Delete,
        (None, _) => RevertKind::Insert,
        (Some(_), Some(_)) => RevertKind::Update,
    };
    Ok((Verdict::Act(kind), current, target))
}

/// Works out what reverting `selection` does, without writing. Reads in one snapshot when the caller's
/// transaction gives one ([`Database::revert_dry_run`] does).
pub fn dry_run(
    conn: &mut AnyConnection,
    selection: &Filter,
    soft_delete: &[&str],
    accounts: &[AccountColumn],
) -> Result<RevertPlan, Error> {
    if selection.actor.is_none() && selection.account.is_none() && selection.since.is_none() {
        return Err(Error::Revert(
            "a revert needs an actor, an account or a start time".into(),
        ));
    }
    let mut w = Where::default();
    w.sql.push("\"op\" <> 'migrate'".into());
    conditions(conn, selection, accounts, &mut w)?;
    let entries = load(conn, w, " ORDER BY \"id\"")?;
    let count = entries.len();
    let selected: HashSet<i64> = entries.iter().map(|e| e.id).collect();

    let mut rows: Vec<Touched> = Vec::new();
    let mut index: HashMap<(String, String), usize> = HashMap::new();
    for entry in entries {
        let key = parse(&entry.row_key, &format!("entry {}", entry.id))?;
        // A row's key text differs by engine ({"id":1} against {"id": 1}) once a database has moved.
        let canon = normalized(&key).to_string();
        match index.get(&(entry.table_name.clone(), canon.clone())) {
            Some(&at) => rows[at].entries.push(entry),
            None => {
                index.insert((entry.table_name.clone(), canon.clone()), rows.len());
                rows.push(Touched {
                    table: entry.table_name.clone(),
                    key,
                    canon,
                    entries: vec![entry],
                });
            }
        }
    }

    // Every entry since the selection's first on each table, by row.
    let mut first_by_table: BTreeMap<String, i64> = BTreeMap::new();
    for row in &rows {
        if let Some(first) = row.entries.first() {
            let id = first_by_table.entry(row.table.clone()).or_insert(first.id);
            *id = (*id).min(first.id);
        }
    }
    let mut later: HashMap<(String, String), Vec<ChangeEntry>> = HashMap::new();
    let mut tables: HashMap<String, TableInfo> = HashMap::new();
    for (table, from) in &first_by_table {
        let mut w = Where::default();
        let p = w.param(Bind::Text(table.clone()));
        w.sql.push(format!("\"table_name\" = {p}"));
        let p = w.param(Bind::Int(*from));
        w.sql.push(format!("\"id\" > {p}"));
        for entry in load(conn, w, " ORDER BY \"id\"")? {
            let canon = normalized(&parse(&entry.row_key, &format!("entry {}", entry.id))?).to_string();
            later.entry((table.clone(), canon)).or_default().push(entry);
        }
        tables.insert(table.clone(), TableInfo::read(conn, table, soft_delete)?);
    }
    let rank: HashMap<String, usize> = ordered(conn, first_by_table.keys().cloned().collect())?
        .into_iter()
        .enumerate()
        .map(|(i, t)| (t, i))
        .collect();

    let mut actions = Vec::new();
    let mut conflicts = Vec::new();
    let mut unchanged = Vec::new();
    for row in &rows {
        let first = row.entries.first().map_or(0, |e| e.id);
        let row_later: Vec<ChangeEntry> = later
            .get(&(row.table.clone(), row.canon.clone()))
            .map(|all| all.iter().filter(|e| e.id > first).cloned().collect())
            .unwrap_or_default();
        let Some(info) = tables.get(&row.table) else { continue };
        let ids: Vec<i64> = row.entries.iter().map(|e| e.id).collect();
        let (verdict, current, target) = verdict(conn, row, info, &row_later, &selected)?;
        match verdict {
            Verdict::Act(kind) => {
                let table_rank = rank.get(&row.table).copied().unwrap_or(0);
                // Rows back parents first, changes, rows out children first; later entries first in each.
                let order = match kind {
                    RevertKind::Insert => (0, table_rank, std::cmp::Reverse(first)),
                    RevertKind::Update | RevertKind::SoftDelete => (1, 0, std::cmp::Reverse(first)),
                    RevertKind::Delete => (2, usize::MAX - table_rank, std::cmp::Reverse(first)),
                };
                let action = RevertAction {
                    table: row.table.clone(),
                    key: row.key.clone(),
                    kind,
                    current,
                    target: if kind == RevertKind::SoftDelete { None } else { target },
                    entries: ids,
                };
                actions.push((order, action));
            }
            Verdict::Conflict(conflict) => conflicts.push(RowConflict {
                table: row.table.clone(),
                key: row.key.clone(),
                entries: ids,
                current,
                conflict,
            }),
            Verdict::Unchanged => unchanged.push(RowRef {
                table: row.table.clone(),
                key: row.key.clone(),
                entries: ids,
            }),
        }
    }
    actions.sort_by_key(|a| a.0);
    let mut plan = RevertPlan {
        id: String::new(),
        selection: selection.clone(),
        entries: count,
        actions: actions.into_iter().map(|(_, a)| a).collect(),
        conflicts,
        unchanged,
    };
    plan.id = plan_id(&plan)?;
    Ok(plan)
}

fn plan_id(plan: &RevertPlan) -> Result<String, Error> {
    let bytes = serde_json::to_vec(plan).map_err(|e| Error::Revert(e.to_string()))?;
    Ok(Sha256::digest(&bytes)
        .iter()
        .flat_map(|&b| [b >> 4, b & 15])
        .filter_map(|n| char::from_digit(u32::from(n), 16))
        .collect())
}

// ------------------------------------------------------------------ apply

fn execute(tx: &mut Tx<'_>, info: &TableInfo, action: &RevertAction) -> Result<(), Error> {
    let engine = tx.engine();
    let table = action.table.as_str();
    let t = quote_ident(table);
    let key = Bind::Text(action.key.to_string());
    let target_columns = || -> Vec<&Column> {
        let fields = action.target.as_ref().and_then(Value::as_object);
        info.columns
            .iter()
            .filter(|c| fields.is_some_and(|f| f.contains_key(&c.name)))
            .collect()
    };
    let target = || Bind::Text(action.target.as_ref().map_or_else(String::new, Value::to_string));
    let (sql, binds) = match action.kind {
        RevertKind::Delete => (
            format!("DELETE FROM {t} WHERE {}", key_match(engine, table, info, 1)),
            vec![key],
        ),
        RevertKind::SoftDelete => {
            let deleted = quote_ident(DELETED_AT);
            (
                format!(
                    "UPDATE {t} SET {deleted} = $1 WHERE {} AND {deleted} IS NULL",
                    key_match(engine, table, info, 2)
                ),
                vec![Bind::Int(tx.at()), key],
            )
        }
        RevertKind::Insert => {
            let columns = target_columns();
            let names = idents(columns.iter().map(|c| c.name.as_str()));
            let sql = match engine {
                Engine::Sqlite => format!(
                    "INSERT INTO {t} ({names}) VALUES ({})",
                    columns
                        .iter()
                        .map(|c| sqlite_value(c, 1))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                Engine::Postgres => format!("INSERT INTO {t} ({names}) SELECT {names} FROM {}", pg_record(table, 1)),
            };
            (sql, vec![target()])
        }
        RevertKind::Update => {
            let columns: Vec<&Column> = target_columns()
                .into_iter()
                .filter(|c| !info.key.contains(&c.name))
                .collect();
            if columns.is_empty() {
                return Ok(());
            }
            let set = match engine {
                Engine::Sqlite => columns
                    .iter()
                    .map(|c| format!("{} = {}", quote_ident(&c.name), sqlite_value(c, 1)))
                    .collect::<Vec<_>>()
                    .join(", "),
                Engine::Postgres => {
                    let names = idents(columns.iter().map(|c| c.name.as_str()));
                    format!("({names}) = (SELECT {names} FROM {})", pg_record(table, 1))
                }
            };
            (
                format!("UPDATE {t} SET {set} WHERE {}", key_match(engine, table, info, 2)),
                vec![target(), key],
            )
        }
    };
    let changed = boxed(sql, binds).execute(tx.conn())?;
    if changed != 1 {
        return Err(Error::Revert(format!(
            "{table} {}: the revert changed {changed} rows, not one",
            action.key
        )));
    }
    Ok(())
}

/// Applies `plan` inside `tx`, the reverting actor's write: plans again under the write lock, refuses with
/// [`Error::PlanChanged`] unless that gives the same id, then runs every action. Returns the plan applied.
/// Any failure rolls the whole revert back with the write.
pub fn apply(tx: &mut Tx<'_>, plan: &RevertPlan, accounts: &[AccountColumn]) -> Result<RevertPlan, Error> {
    let soft_delete: Vec<&'static str> = tx.soft_delete_tables().to_vec();
    let fresh = dry_run(tx.conn(), &plan.selection, &soft_delete, accounts)?;
    if fresh.id != plan.id {
        return Err(Error::PlanChanged);
    }
    let mut tables: HashMap<String, TableInfo> = HashMap::new();
    for action in &fresh.actions {
        if !tables.contains_key(&action.table) {
            let info = TableInfo::read(tx.conn(), &action.table, &soft_delete)?;
            tables.insert(action.table.clone(), info);
        }
        if let Some(info) = tables.get(&action.table) {
            execute(tx, info, action)?;
        }
    }
    Ok(fresh)
}

impl Database {
    /// One page of history, newest first: [`history()`] in a read.
    pub async fn history(&self, filter: Filter, cursor: Option<Cursor>, limit: i64) -> Result<HistoryPage, Error> {
        let accounts = self.account_columns();
        self.read(move |conn| history(conn, &filter, &accounts, cursor, limit))
            .await
    }

    /// What reverting `selection` would do: [`dry_run`] in one snapshot.
    pub async fn revert_dry_run(&self, selection: Filter) -> Result<RevertPlan, Error> {
        let accounts = self.account_columns();
        let soft_delete = self.soft_delete_tables();
        self.read(move |conn| {
            if conn.engine() == Engine::Postgres {
                conn.batch_execute("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")?;
            }
            dry_run(conn, &selection, &soft_delete, &accounts)
        })
        .await
    }

    /// Applies a dry run's plan as `actor`'s write, so the revert is logged and revertible.
    pub async fn revert_apply(&self, actor: Actor, plan: RevertPlan) -> Result<RevertOutcome, Error> {
        let accounts: Arc<[AccountColumn]> = self.account_columns();
        self.write(actor, move |tx| {
            let plan = apply(tx, &plan, &accounts)?;
            Ok(RevertOutcome { at: tx.at(), plan })
        })
        .await
    }
}
