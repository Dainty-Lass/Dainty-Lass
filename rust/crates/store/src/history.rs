//! The change log (DD23, DH1): every insert, update and delete on a two-engine database leaves an entry in
//! `__store_change_log`, in the same transaction, with the time, the actor, the table, the row's key and
//! the row before and after as JSON.
//!
//! The entries are written by triggers that [`crate::Database::open`] puts on every table, so a repository
//! can't forget one. A trigger refuses a write made outside [`crate::Database::write`]: that call puts the
//! actor where the trigger finds it (a row of `__store_context` on SQLite, a transaction-local setting on
//! Postgres), and nothing else does. The log itself refuses updates and deletes.
//!
//! JSON per engine: SQLite stores booleans as 0 and 1, and blobs are written as `\x` and lower-case hex on
//! both, as Postgres writes `bytea`. Postgres's `jsonb` text orders keys its own way: parse, don't compare.

use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Nullable, Text};

use crate::connection::{AnyConnection, MultiBackend};
use crate::portable::{quote_ident, quote_literal};
use crate::{Engine, Error};

pub const CHANGE_LOG: &str = "__store_change_log";
pub(crate) const CONTEXT: &str = "__store_context";
pub(crate) const MIGRATIONS: &str = "__diesel_schema_migrations";
/// Tables the triggers leave alone: the log, its context and Diesel's own.
pub(crate) const INTERNAL: [&str; 3] = [CHANGE_LOG, CONTEXT, MIGRATIONS];
/// The column a soft-deleted row has set (unix milliseconds); null while the row is live.
pub const DELETED_AT: &str = "deleted_at";

/// Who made a change: what the log records and what DH3's `revert --actor` names. Kept small on purpose:
/// DF2 defines the request context in `module-api`, which carries one of these.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Actor {
    /// A player's request: their account id.
    Player(String),
    /// An admin API caller: the key's name (one per person, DH2).
    Admin(String),
    /// A host kit command or the console: which one.
    Host(String),
    /// The server itself: the task (`retention`, `migration`).
    System(String),
}

impl Actor {
    /// `player:<id>`, `admin:<key>`, `host:<command>` or `system:<task>`, as the log stores it.
    pub fn as_text(&self) -> String {
        let (kind, id) = match self {
            Self::Player(id) => ("player", id),
            Self::Admin(id) => ("admin", id),
            Self::Host(id) => ("host", id),
            Self::System(id) => ("system", id),
        };
        format!("{kind}:{id}")
    }

    pub fn parse(text: &str) -> Option<Self> {
        let (kind, id) = text.split_once(':')?;
        if id.is_empty() || id.chars().any(char::is_control) {
            return None;
        }
        let id = id.to_owned();
        match kind {
            "player" => Some(Self::Player(id)),
            "admin" => Some(Self::Admin(id)),
            "host" => Some(Self::Host(id)),
            "system" => Some(Self::System(id)),
            _ => None,
        }
    }

    fn check(&self) -> Result<(), Error> {
        let text = self.as_text();
        if Self::parse(&text).as_ref() == Some(self) {
            Ok(())
        } else {
            Err(Error::Actor(text))
        }
    }
}

/// One entry of the log.
#[derive(Debug, Clone, PartialEq, Eq, QueryableByName)]
pub struct ChangeEntry {
    #[diesel(sql_type = BigInt)]
    pub id: i64,
    /// Unix milliseconds: the start of the write it belongs to, the same for every entry of it.
    #[diesel(sql_type = BigInt)]
    pub at: i64,
    #[diesel(sql_type = Text)]
    pub actor: String,
    #[diesel(sql_type = Text)]
    pub table_name: String,
    /// The primary key as a JSON object.
    #[diesel(sql_type = Text)]
    pub row_key: String,
    /// `insert`, `update`, `delete`, `soft_delete`, `restore`, or `migrate` for a migration applied at open.
    #[diesel(sql_type = Text)]
    pub op: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub before: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub after: Option<String>,
}

/// Entries after `after_id`, oldest first, at most `limit`.
pub fn changes(conn: &mut AnyConnection, after_id: i64, limit: i64) -> Result<Vec<ChangeEntry>, Error> {
    Ok(diesel::sql_query(format!(
        "SELECT id, at, actor, table_name, row_key, op, before, after FROM {} WHERE id > $1 ORDER BY id LIMIT $2",
        quote_ident(CHANGE_LOG)
    ))
    .bind::<BigInt, _>(after_id)
    .bind::<BigInt, _>(limit)
    .load(conn)?)
}

// ------------------------------------------------------------------ the tables

/// `id` is never reused (AUTOINCREMENT, an identity), so DH5's hash chain can name an entry by it.
/// Postgres's is `BY DEFAULT`, not `ALWAYS`, so `db move` can copy the ids.
fn ensure_sql(engine: Engine) -> String {
    let log = quote_ident(CHANGE_LOG);
    let columns = "\"at\" BIGINT NOT NULL, \"actor\" TEXT NOT NULL, \"table_name\" TEXT NOT NULL, \
                   \"row_key\" TEXT NOT NULL, \"op\" TEXT NOT NULL, \"before\" TEXT, \"after\" TEXT";
    let indexes = format!(
        "CREATE INDEX IF NOT EXISTS \"__store_change_log_at\" ON {log} (\"at\");
         CREATE INDEX IF NOT EXISTS \"__store_change_log_actor\" ON {log} (\"actor\", \"at\");
         CREATE INDEX IF NOT EXISTS \"__store_change_log_row\" ON {log} (\"table_name\", \"row_key\");"
    );
    match engine {
        Engine::Sqlite => format!(
            "CREATE TABLE IF NOT EXISTS {log} (\"id\" INTEGER PRIMARY KEY AUTOINCREMENT, {columns});
             {indexes}
             CREATE TABLE IF NOT EXISTS {ctx} (\"actor\" TEXT NOT NULL, \"at\" BIGINT NOT NULL);
             CREATE TRIGGER IF NOT EXISTS \"__store_change_log_no_update\" BEFORE UPDATE ON {log}
               BEGIN SELECT RAISE(ABORT, 'store: the change log is append-only'); END;
             CREATE TRIGGER IF NOT EXISTS \"__store_change_log_no_delete\" BEFORE DELETE ON {log}
               BEGIN SELECT RAISE(ABORT, 'store: the change log is append-only'); END;",
            ctx = quote_ident(CONTEXT),
        ),
        Engine::Postgres => format!(
            "CREATE TABLE IF NOT EXISTS {log} (\"id\" BIGINT GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY, {columns});
             {indexes}
             CREATE OR REPLACE FUNCTION __store_append_only() RETURNS trigger LANGUAGE plpgsql AS $f$
             BEGIN
               RAISE EXCEPTION 'store: the change log is append-only';
             END $f$;
             DROP TRIGGER IF EXISTS \"__store_append_only\" ON {log};
             CREATE TRIGGER \"__store_append_only\" BEFORE UPDATE OR DELETE OR TRUNCATE ON {log}
               FOR EACH STATEMENT EXECUTE FUNCTION __store_append_only();
             CREATE OR REPLACE FUNCTION __store_log_change() RETURNS trigger LANGUAGE plpgsql AS $f$
             DECLARE
               who text := current_setting('dainty.actor', true);
               old_row jsonb;
               new_row jsonb;
               src jsonb;
               row_key jsonb := '{{}}'::jsonb;
               k text;
               op text := lower(TG_OP);
             BEGIN
               -- Empty, not null, once a transaction-local setting has ended in this session.
               IF who IS NULL OR who = '' THEN
                 RAISE EXCEPTION 'store: a write to % outside store''s write path', TG_TABLE_NAME;
               END IF;
               IF TG_OP <> 'INSERT' THEN old_row := to_jsonb(OLD); END IF;
               IF TG_OP <> 'DELETE' THEN new_row := to_jsonb(NEW); END IF;
               src := coalesce(new_row, old_row);
               FOREACH k IN ARRAY TG_ARGV LOOP
                 row_key := row_key || jsonb_build_object(k, src -> k);
               END LOOP;
               IF TG_OP = 'UPDATE' AND old_row ? '{DELETED_AT}' THEN
                 IF old_row -> '{DELETED_AT}' = 'null'::jsonb AND new_row -> '{DELETED_AT}' <> 'null'::jsonb THEN
                   op := 'soft_delete';
                 ELSIF old_row -> '{DELETED_AT}' <> 'null'::jsonb AND new_row -> '{DELETED_AT}' = 'null'::jsonb THEN
                   op := 'restore';
                 END IF;
               END IF;
               INSERT INTO {log} (\"at\", \"actor\", \"table_name\", \"row_key\", \"op\", \"before\", \"after\")
                 VALUES (current_setting('dainty.at')::bigint, who, TG_TABLE_NAME, row_key::text, op,
                         old_row::text, new_row::text);
               RETURN NULL;
             END $f$;"
        ),
    }
}

pub(crate) fn ensure_tables(conn: &mut AnyConnection) -> Result<(), Error> {
    conn.batch_execute(&ensure_sql(conn.engine()))?;
    Ok(())
}

// ------------------------------------------------------------------ the catalog

#[derive(QueryableByName)]
struct Name {
    #[diesel(sql_type = Text)]
    name: String,
}

#[derive(QueryableByName)]
pub(crate) struct Column {
    #[diesel(sql_type = Text)]
    pub name: String,
    /// As declared on SQLite, `data_type` on Postgres; lower case.
    #[diesel(sql_type = Text)]
    pub kind: String,
}

fn names(conn: &mut AnyConnection, sql: &str, bind: Option<&str>) -> Result<Vec<String>, Error> {
    let query = diesel::sql_query(sql).into_boxed::<MultiBackend>();
    let query = match bind {
        Some(value) => query.bind::<Text, _>(value.to_owned()),
        None => query,
    };
    Ok(query.load::<Name>(conn)?.into_iter().map(|n| n.name).collect())
}

/// The tables a service's migrations made, by name: everything but [`INTERNAL`].
pub(crate) fn user_tables(conn: &mut AnyConnection) -> Result<Vec<String>, Error> {
    let internal = INTERNAL.map(quote_literal).join(", ");
    let sql = match conn.engine() {
        Engine::Sqlite => format!(
            "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite!_%' ESCAPE '!'
             AND name NOT IN ({internal}) ORDER BY name"
        ),
        Engine::Postgres => format!(
            "SELECT table_name::text AS name FROM information_schema.tables
             WHERE table_schema = current_schema() AND table_type = 'BASE TABLE' AND table_name NOT IN ({internal})
             ORDER BY table_name"
        ),
    };
    names(conn, &sql, None)
}

pub(crate) fn columns(conn: &mut AnyConnection, table: &str) -> Result<Vec<Column>, Error> {
    let sql = match conn.engine() {
        Engine::Sqlite => "SELECT name, lower(type) AS kind FROM pragma_table_info($1) ORDER BY cid",
        Engine::Postgres => {
            "SELECT column_name::text AS name, lower(data_type::text) AS kind FROM information_schema.columns
             WHERE table_schema = current_schema() AND table_name = $1 ORDER BY ordinal_position"
        }
    };
    Ok(diesel::sql_query(sql)
        .bind::<Text, _>(table.to_owned())
        .load::<Column>(conn)?)
}

pub(crate) fn primary_key(conn: &mut AnyConnection, table: &str) -> Result<Vec<String>, Error> {
    let sql = match conn.engine() {
        Engine::Sqlite => "SELECT name FROM pragma_table_info($1) WHERE pk > 0 ORDER BY pk",
        Engine::Postgres => {
            "SELECT a.attname::text AS name FROM pg_index i
             JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY(i.indkey)
             WHERE i.indisprimary AND i.indrelid = (quote_ident(current_schema()) || '.' || quote_ident($1))::regclass
             ORDER BY array_position(i.indkey::int2[], a.attnum)"
        }
    };
    names(conn, sql, Some(table))
}

/// The tables `table`'s foreign keys point at.
pub(crate) fn parents(conn: &mut AnyConnection, table: &str) -> Result<Vec<String>, Error> {
    let sql = match conn.engine() {
        Engine::Sqlite => "SELECT DISTINCT \"table\" AS name FROM pragma_foreign_key_list($1)",
        Engine::Postgres => {
            "SELECT DISTINCT ccu.table_name::text AS name FROM information_schema.table_constraints tc
             JOIN information_schema.constraint_column_usage ccu
               ON ccu.constraint_name = tc.constraint_name AND ccu.constraint_schema = tc.constraint_schema
             WHERE tc.constraint_type = 'FOREIGN KEY' AND tc.table_schema = current_schema() AND tc.table_name = $1"
        }
    };
    names(conn, sql, Some(table))
}

// ------------------------------------------------------------------ the triggers

/// A SQLite column value for JSON: a blob as Postgres writes `bytea` (`json_object` refuses blobs).
pub(crate) fn sqlite_json_value(row: &str, column: &str) -> String {
    let c = if row.is_empty() {
        quote_ident(column)
    } else {
        format!("{row}.{}", quote_ident(column))
    };
    format!("CASE WHEN typeof({c}) = 'blob' THEN '\\x' || lower(hex({c})) ELSE {c} END")
}

pub(crate) fn sqlite_json_object<'a>(row: &str, columns: impl IntoIterator<Item = &'a str>) -> String {
    let pairs: Vec<String> = columns
        .into_iter()
        .map(|c| format!("{}, {}", quote_literal(c), sqlite_json_value(row, c)))
        .collect();
    format!("json_object({})", pairs.join(", "))
}

fn sqlite_triggers(table: &str, key: &[String], columns: &[Column]) -> String {
    let t = quote_ident(table);
    let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    let soft = names.contains(&DELETED_AT);
    let refuse = quote_literal(&format!("store: a write to {table} outside store's write path"));
    let deleted = quote_ident(DELETED_AT);
    let op_update = if soft {
        format!(
            "CASE WHEN OLD.{deleted} IS NULL AND NEW.{deleted} IS NOT NULL THEN 'soft_delete' \
             WHEN OLD.{deleted} IS NOT NULL AND NEW.{deleted} IS NULL THEN 'restore' ELSE 'update' END"
        )
    } else {
        "'update'".into()
    };
    let mut out = String::new();
    for (event, op, row_for_key, before, after) in [
        ("INSERT", "'insert'".to_owned(), "NEW", None, Some("NEW")),
        ("UPDATE", op_update, "NEW", Some("OLD"), Some("NEW")),
        ("DELETE", "'delete'".to_owned(), "OLD", Some("OLD"), None),
    ] {
        let object =
            |row: Option<&str>| row.map_or("NULL".to_owned(), |r| sqlite_json_object(r, names.iter().copied()));
        out += &format!(
            "CREATE TRIGGER {trigger} AFTER {event} ON {t} BEGIN
               SELECT RAISE(ABORT, {refuse}) WHERE NOT EXISTS (SELECT 1 FROM {ctx});
               INSERT INTO {log} (\"at\", \"actor\", \"table_name\", \"row_key\", \"op\", \"before\", \"after\")
                 SELECT \"at\", \"actor\", {name}, {row_key}, {op}, {before}, {after} FROM {ctx} LIMIT 1;
             END;\n",
            trigger = quote_ident(&format!("__store_log_{table}_{}", event.to_lowercase())),
            ctx = quote_ident(CONTEXT),
            log = quote_ident(CHANGE_LOG),
            name = quote_literal(table),
            row_key = sqlite_json_object(row_for_key, key.iter().map(String::as_str)),
            before = object(before),
            after = object(after),
        );
    }
    out
}

/// Takes every change-log trigger off, so a migration can change the tables under them.
pub(crate) fn drop_triggers(conn: &mut AnyConnection) -> Result<(), Error> {
    match conn.engine() {
        Engine::Sqlite => {
            let triggers = names(
                conn,
                "SELECT name FROM sqlite_schema WHERE type = 'trigger' AND substr(name, 1, 12) = '__store_log_'",
                None,
            )?;
            for trigger in triggers {
                conn.batch_execute(&format!("DROP TRIGGER IF EXISTS {}", quote_ident(&trigger)))?;
            }
        }
        Engine::Postgres => {
            let tables = names(
                conn,
                "SELECT DISTINCT event_object_table::text AS name FROM information_schema.triggers
                 WHERE trigger_schema = current_schema() AND trigger_name = '__store_log'",
                None,
            )?;
            for table in tables {
                conn.batch_execute(&format!(
                    "DROP TRIGGER IF EXISTS \"__store_log\" ON {}",
                    quote_ident(&table)
                ))?;
            }
        }
    }
    Ok(())
}

/// Puts the change-log triggers on every table. Each table needs a primary key, which names its rows in
/// the log; a soft-delete table needs a `deleted_at` column.
pub(crate) fn install_triggers(conn: &mut AnyConnection, soft_delete: &[&str]) -> Result<(), Error> {
    let tables = user_tables(conn)?;
    for wanted in soft_delete {
        if !tables.iter().any(|t| t == wanted) {
            return Err(Error::Schema(format!("the soft-delete table {wanted} doesn't exist")));
        }
    }
    for table in tables {
        let key = primary_key(conn, &table)?;
        if key.is_empty() {
            return Err(Error::Schema(format!(
                "{table} has no primary key: the change log names every row by it (DH1)"
            )));
        }
        let columns = columns(conn, &table)?;
        if soft_delete.contains(&table.as_str()) && !columns.iter().any(|c| c.name == DELETED_AT) {
            return Err(Error::Schema(format!(
                "{table} is soft-deleted, so it needs a nullable BIGINT {DELETED_AT} column"
            )));
        }
        let sql = match conn.engine() {
            Engine::Sqlite => sqlite_triggers(&table, &key, &columns),
            Engine::Postgres => format!(
                "CREATE TRIGGER \"__store_log\" AFTER INSERT OR UPDATE OR DELETE ON {} FOR EACH ROW
                 EXECUTE FUNCTION __store_log_change({});",
                quote_ident(&table),
                key.iter().map(|k| quote_literal(k)).collect::<Vec<_>>().join(", ")
            ),
        };
        conn.batch_execute(&sql)?;
    }
    Ok(())
}

// ------------------------------------------------------------------ the write path

/// Advisory lock every Postgres write takes, so writes are serial as on SQLite: the log's order is the
/// order the changes were made in, which DH3's conflict check and DH5's hash chain rely on.
const WRITE_LOCK: i64 = 0x6461_696e_7479; // "dainty"
/// Held while a database is opened, so two services opening one Postgres database don't migrate it twice.
pub(crate) const OPEN_LOCK: i64 = 0x6461_696e_7475;

/// Makes `actor` the author of what the transaction writes next, and returns the write's time, taken once
/// the write lock is held so the log's times follow its order. SQLite's first statement is a write, so
/// the transaction takes the lock at once, as `BEGIN IMMEDIATE` does (its one writer is the pool's).
pub(crate) fn begin(conn: &mut AnyConnection, actor: &Actor, now: fn() -> i64) -> Result<i64, Error> {
    actor.check()?;
    match conn.engine() {
        Engine::Sqlite => {
            let at = now();
            diesel::sql_query(format!(
                "INSERT INTO {} (\"actor\", \"at\") VALUES ($1, $2)",
                quote_ident(CONTEXT)
            ))
            .bind::<Text, _>(actor.as_text())
            .bind::<BigInt, _>(at)
            .execute(conn)?;
            Ok(at)
        }
        Engine::Postgres => {
            diesel::sql_query("SELECT pg_advisory_xact_lock($1)")
                .bind::<BigInt, _>(WRITE_LOCK)
                .execute(conn)?;
            let at = now();
            diesel::sql_query("SELECT set_config('dainty.actor', $1, true), set_config('dainty.at', $2, true)")
                .bind::<Text, _>(actor.as_text())
                .bind::<Text, _>(at.to_string())
                .execute(conn)?;
            Ok(at)
        }
    }
}

pub(crate) fn end(conn: &mut AnyConnection) -> Result<(), Error> {
    if conn.engine() == Engine::Sqlite {
        conn.batch_execute(&format!("DELETE FROM {}", quote_ident(CONTEXT)))?;
    }
    Ok(())
}

/// Records the migrations [`crate::Database::open`] applied: what they changed isn't logged row by row,
/// since the triggers are off while they run.
pub(crate) fn log_migrations(conn: &mut AnyConnection, versions: &[String], at: i64) -> Result<(), Error> {
    for version in versions {
        let row = serde_json::json!({ "version": version }).to_string();
        diesel::sql_query(format!(
            "INSERT INTO {} (\"at\", \"actor\", \"table_name\", \"row_key\", \"op\", \"before\", \"after\")
             VALUES ($1, $2, $3, $4, 'migrate', NULL, $5)",
            quote_ident(CHANGE_LOG)
        ))
        .bind::<BigInt, _>(at)
        .bind::<Text, _>(Actor::System("migration".into()).as_text())
        .bind::<Text, _>(MIGRATIONS)
        .bind::<Text, _>(row.clone())
        .bind::<Text, _>(row)
        .execute(conn)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actors_round_trip_as_text() {
        for actor in [
            Actor::Player("42".into()),
            Actor::Admin("owner".into()),
            Actor::Host("console".into()),
            Actor::System("retention".into()),
        ] {
            assert_eq!(Actor::parse(&actor.as_text()), Some(actor));
        }
        assert_eq!(Actor::parse("player:a:b"), Some(Actor::Player("a:b".into())));
        for bad in ["", "player", "player:", "robot:x", "admin:a\nb"] {
            assert_eq!(Actor::parse(bad), None, "{bad:?}");
        }
        assert!(Actor::Player(String::new()).check().is_err());
    }
}
