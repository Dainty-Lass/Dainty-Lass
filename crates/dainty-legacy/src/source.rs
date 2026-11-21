// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! A 1.4.4 database (spec §5, §6): copied into a private folder, checked by its history, its objects and the
//! columns read, then read where it stands (L-3). The host's file is never opened for writing.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Nullable, Text};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness};
use sha2::{Digest, Sha256};

use crate::data::{self, Revived};
use crate::error::ImportError;

/// The online backup's busy timeout and deadline (§5).
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const COPY_DEADLINE: Duration = Duration::from_secs(15 * 60);

/// Where a 1.4.4 database comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Origin {
    /// A server's, possibly still running: copied with SQLite's online backup.
    Server,
    /// A stopped player's: copied as a file, and must be in rollback-journal mode.
    Offline,
}

/// Whose accounts are carried (§7.2).
pub(crate) enum Holder<'a> {
    /// A server: everyone, and the invites.
    Server,
    /// A bare offline database: its one player.
    Bare,
    /// A `.drsave`: the player its credential belongs to.
    Container {
        user_id: &'a str,
        username: &'a str,
        key: &'a str,
    },
}

/// A 1.4.4 ban.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ban {
    pub reason: Option<String>,
    /// Unix milliseconds; `None` is permanent.
    pub expires_at: Option<i64>,
}

/// A 1.4.4 Discord link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Link {
    pub discord_id: String,
    /// `YYYY-MM-DDTHH:MM:SS.mmmZ`.
    pub linked_at: String,
}

/// A 1.4.4 account with its ban and link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Account {
    pub user_id: String,
    pub name: String,
    pub notes: i64,
    pub is_admin: bool,
    pub ban: Option<Ban>,
    pub link: Option<Link>,
}

/// A 1.4.4 invite code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Invite {
    pub code: String,
    pub uses: i64,
    pub infinite: bool,
}

/// What a 1.4.4 database carries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Source {
    pub accounts: Vec<Account>,
    pub invites: Vec<Invite>,
    pub dropped: BTreeMap<String, u64>,
}

/// The import's clock: unix milliseconds and `YYYY-MM-DDTHH:MM:SS.mmmZ`.
#[derive(Debug, Clone)]
pub(crate) struct Now {
    pub ms: i64,
    pub text: String,
}

pub(crate) const TIME_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.3fZ";

impl Now {
    pub(crate) fn current() -> Self {
        let now = chrono::Utc::now();
        Self {
            ms: now.timestamp_millis(),
            text: now.format(TIME_FORMAT).to_string(),
        }
    }
}

/// Unix milliseconds as `YYYY-MM-DDTHH:MM:SS.mmmZ`, if it is a time chrono can hold.
pub(crate) fn format_ms(ms: i64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(ms).map(|t| t.format(TIME_FORMAT).to_string())
}

const NO_PLAYER: &str = "not a Dauntless Revived 1.4.4 offline save: it doesn't hold one player";

/// Copies, checks and reads the 1.4.4 database at `path`.
pub(crate) fn load(path: &Path, origin: Origin, holder: &Holder<'_>, now: &Now) -> Result<Source, ImportError> {
    let revived = data::revived()?;
    let dir = tempfile::Builder::new()
        .prefix("dainty-legacy-")
        .tempdir()
        .map_err(|e| ImportError::Io(format!("a private temporary folder: {e}")))?;
    let copy = dir.path().join("source.db");
    copy_source(path, &copy, origin)?;
    let conn = SqliteConnection::establish(&copy.to_string_lossy()).map_err(|e| ImportError::sqlite(path, e))?;
    let mut reader = Reader { conn, path };
    let checked = reader.check(revived, origin)?;
    let mut source = reader.read(revived, &checked, now)?;
    match holder {
        Holder::Server => {}
        Holder::Bare => {
            if source.accounts.len() != 1 {
                return Err(ImportError::Invalid(NO_PLAYER.into()));
            }
            source.invites.clear();
        }
        Holder::Container { user_id, username, key } => {
            reader.check_holder(revived, &checked, user_id, username, key)?;
            source.accounts.retain(|a| a.user_id == *user_id);
            source.invites.clear();
        }
    }
    Ok(source)
}

fn copy_source(path: &Path, copy: &Path, origin: Origin) -> Result<(), ImportError> {
    if !path.is_file() {
        return Err(ImportError::io(path, "there is no file here"));
    }
    match origin {
        Origin::Server => {
            dainty_store::backup::backup_file(path, copy, BUSY_TIMEOUT, COPY_DEADLINE).map_err(|e| {
                let text = e.to_string();
                ImportError::sqlite(path, text.strip_prefix("backup: ").unwrap_or(&text))
            })?;
            let mut conn =
                SqliteConnection::establish(&copy.to_string_lossy()).map_err(|e| ImportError::sqlite(path, e))?;
            conn.batch_execute("PRAGMA journal_mode = DELETE")
                .map_err(|e| ImportError::sqlite(path, e))?;
        }
        Origin::Offline => {
            for suffix in ["-wal", "-shm", "-journal"] {
                let mut beside = path.as_os_str().to_owned();
                beside.push(suffix);
                if Path::new(&beside).exists() {
                    return Err(ImportError::not_1_4_4(
                        "it has journal files beside it, so it was not stopped cleanly",
                    ));
                }
            }
            // A new file rather than fs::copy, which would carry a read-only attribute across
            let mut from = std::fs::File::open(path).map_err(|e| ImportError::io(path, e))?;
            let mut to = std::fs::File::create_new(copy).map_err(|e| ImportError::io(copy, e))?;
            std::io::copy(&mut from, &mut to).map_err(|e| ImportError::io(path, e))?;
        }
    }
    Ok(())
}

#[derive(QueryableByName)]
struct NameRow {
    #[diesel(sql_type = Text)]
    name: String,
}

#[derive(QueryableByName)]
struct ObjectRow {
    #[diesel(sql_type = Text)]
    kind: String,
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = Nullable<Text>)]
    sql: Option<String>,
}

#[derive(QueryableByName)]
struct CountRow {
    #[diesel(sql_type = BigInt)]
    n: i64,
}

#[derive(QueryableByName)]
struct IntegrityRow {
    #[diesel(sql_type = Nullable<Text>)]
    integrity_check: Option<String>,
}

#[derive(QueryableByName)]
struct JournalRow {
    #[diesel(sql_type = Nullable<Text>)]
    journal_mode: Option<String>,
}

#[derive(QueryableByName)]
struct DrizzleRow {
    #[diesel(sql_type = Nullable<BigInt>)]
    created_at: Option<i64>,
    #[diesel(sql_type = Nullable<Text>)]
    hash: Option<String>,
}

#[derive(QueryableByName)]
struct VersionRow {
    #[diesel(sql_type = Nullable<Text>)]
    version: Option<String>,
}

#[derive(QueryableByName)]
struct UserRow {
    #[diesel(sql_type = Nullable<Text>)]
    user_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    name: Option<String>,
    #[diesel(sql_type = Nullable<BigInt>)]
    notes: Option<i64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    is_admin: Option<i64>,
}

#[derive(QueryableByName)]
struct BanRow {
    #[diesel(sql_type = Nullable<Text>)]
    account_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    reason: Option<String>,
    #[diesel(sql_type = Nullable<BigInt>)]
    expires_at: Option<i64>,
}

#[derive(QueryableByName)]
struct LinkRow {
    #[diesel(sql_type = Nullable<Text>)]
    discord_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    user_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    linked_at: Option<String>,
}

#[derive(QueryableByName)]
struct InviteRow {
    #[diesel(sql_type = Nullable<Text>)]
    code: Option<String>,
    #[diesel(sql_type = Nullable<BigInt>)]
    uses: Option<i64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    infinite: Option<i64>,
}

#[derive(QueryableByName)]
struct HolderRow {
    #[diesel(sql_type = Nullable<Text>)]
    user_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    name: Option<String>,
}

fn collapse(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The `CREATE` text of `__drizzle_migrations` and `__diesel_schema_migrations` as the store's Drizzle replay
/// and Diesel's own bookkeeping make them, white space collapsed.
fn bookkeeping() -> Result<&'static (String, String), ImportError> {
    static CELL: OnceLock<Result<(String, String), String>> = OnceLock::new();
    CELL.get_or_init(|| {
        let mut conn = SqliteConnection::establish(":memory:").map_err(|e| e.to_string())?;
        dainty_store::drizzle::replay(&mut conn, &[]).map_err(|e| e.to_string())?;
        conn.run_pending_migrations(EmbeddedMigrations::new(&[]))
            .map_err(|e| e.to_string())?;
        let mut sql = |name: &str| -> Result<String, String> {
            diesel::sql_query("SELECT type AS kind, name, sql FROM sqlite_schema WHERE name = ?")
                .bind::<Text, _>(name)
                .get_result::<ObjectRow>(&mut conn)
                .map_err(|e| e.to_string())
                .map(|row| collapse(row.sql.as_deref().unwrap_or_default()))
        };
        Ok((sql("__drizzle_migrations")?, sql("__diesel_schema_migrations")?))
    })
    .as_ref()
    .map_err(|e| ImportError::Io(format!("the migration tables' reference text: {e}")))
}

/// What the checks learnt.
pub(crate) struct Checked {
    pub prefix: usize,
    pub tables: BTreeSet<String>,
}

struct Reader<'a> {
    conn: SqliteConnection,
    path: &'a Path,
}

fn quote(name: &str) -> String {
    dainty_store::portable::quote_ident(name)
}

impl Reader<'_> {
    fn load<T: QueryableByName<diesel::sqlite::Sqlite> + 'static>(&mut self, sql: &str) -> Result<Vec<T>, ImportError> {
        diesel::sql_query(sql)
            .load::<T>(&mut self.conn)
            .map_err(|e| ImportError::sqlite(self.path, e))
    }

    fn exec(&mut self, sql: &str) -> Result<(), ImportError> {
        self.conn
            .batch_execute(sql)
            .map_err(|e| ImportError::sqlite(self.path, e))
    }

    fn objects(&mut self) -> Result<Vec<ObjectRow>, ImportError> {
        self.load("SELECT type AS kind, name, sql FROM sqlite_schema WHERE type IN ('table', 'trigger', 'view')")
    }

    fn check(&mut self, revived: &Revived, origin: Origin) -> Result<Checked, ImportError> {
        // Before anything reads the schema: no function in a view or trigger may run
        self.exec("PRAGMA trusted_schema = OFF")?;
        // A Dainty Lass-run metagame carried the store's change log; it isn't the metagame's
        for kind in ["trigger", "table"] {
            let names: Vec<NameRow> = self.load(&format!(
                "SELECT name FROM sqlite_schema WHERE type = '{kind}' AND substr(name, 1, 8) = '__store_'"
            ))?;
            for row in names {
                self.exec(&format!("DROP {} IF EXISTS {}", kind.to_uppercase(), quote(&row.name)))?;
            }
        }
        let objects = self.objects()?;
        let (drizzle_sql, diesel_sql) = bookkeeping()?;
        let find = |name: &str| objects.iter().find(|o| o.kind == "table" && o.name == name);
        let drizzle = find("__drizzle_migrations");
        let diesel = find("__diesel_schema_migrations");
        for (table, expected) in [(drizzle, drizzle_sql), (diesel, diesel_sql)] {
            if let Some(table) = table
                && collapse(table.sql.as_deref().unwrap_or_default()) != *expected
            {
                return Err(ImportError::not_1_4_4(
                    "a migration table is not the one its tool makes",
                ));
            }
        }
        let integrity: Vec<IntegrityRow> = self.load("PRAGMA integrity_check")?;
        if integrity.len() != 1 || integrity[0].integrity_check.as_deref() != Some("ok") {
            return Err(ImportError::not_1_4_4("it fails SQLite's integrity check"));
        }
        if origin == Origin::Offline {
            let mode: Vec<JournalRow> = self.load("PRAGMA journal_mode")?;
            if !mode
                .first()
                .and_then(|m| m.journal_mode.as_deref())
                .is_some_and(|m| m.eq_ignore_ascii_case("delete"))
            {
                return Err(ImportError::not_1_4_4(
                    "an offline save is a stopped database in rollback-journal mode",
                ));
            }
        }
        let prefix = if diesel.is_some() {
            let versions: Vec<VersionRow> = self
                .load("SELECT CAST(version AS TEXT) AS version FROM \"__diesel_schema_migrations\" ORDER BY version")?;
            let versions: Vec<Option<String>> = versions.into_iter().map(|v| v.version).collect();
            let expected: Vec<Option<String>> = revived.diesel.versions.iter().cloned().map(Some).collect();
            if versions != expected {
                return Err(ImportError::not_1_4_4(
                    "its Diesel history is not one the 1.4.4 metagame made",
                ));
            }
            revived.full_prefix()
        } else if drizzle.is_some() {
            let limit = revived.drizzle.len() + 1;
            let rows: Vec<DrizzleRow> = self.load(&format!(
                "SELECT CAST(created_at AS INTEGER) AS created_at, CAST(hash AS TEXT) AS hash \
                 FROM \"__drizzle_migrations\" ORDER BY created_at, rowid LIMIT {limit}"
            ))?;
            let fits = !rows.is_empty()
                && rows.len() <= revived.drizzle.len()
                && rows.iter().zip(&revived.drizzle).all(|(row, entry)| {
                    row.created_at == Some(entry.when)
                        && row
                            .hash
                            .as_deref()
                            .is_some_and(|h| h == entry.sha256_lf || h == entry.sha256_crlf)
                });
            if !fits {
                return Err(ImportError::not_1_4_4(
                    "its Drizzle history is not one the Node metagame made",
                ));
            }
            rows.len()
        } else {
            return Err(ImportError::not_1_4_4(
                "it has neither Drizzle's nor Diesel's migration history",
            ));
        };
        let own = ["__drizzle_migrations", "__diesel_schema_migrations"];
        let known = objects.iter().all(|o| match o.kind.as_str() {
            // sqlite_sequence, and the sqlite_stat* tables ANALYZE and PRAGMA optimize leave
            "table" => {
                o.name.starts_with("sqlite_") || own.contains(&o.name.as_str()) || revived.tables.contains(&o.name)
            }
            "trigger" => revived.triggers.contains(&o.name),
            _ => false,
        });
        if !known {
            return Err(ImportError::not_1_4_4("it holds objects the 1.4.4 metagame never made"));
        }
        for column in revived.column.iter().filter(|c| c.since < prefix) {
            let names: Vec<NameRow> = diesel::sql_query("SELECT name FROM pragma_table_info(?)")
                .bind::<Text, _>(&column.table)
                .load(&mut self.conn)
                .map_err(|e| ImportError::sqlite(self.path, e))?;
            if !names.iter().any(|n| n.name == column.name) {
                return Err(ImportError::not_1_4_4(
                    "its schema is not the one its migration history makes",
                ));
            }
        }
        let tables = objects
            .into_iter()
            .filter(|o| o.kind == "table")
            .map(|o| o.name)
            .collect();
        Ok(Checked { prefix, tables })
    }

    fn read(&mut self, revived: &Revived, checked: &Checked, now: &Now) -> Result<Source, ImportError> {
        let prefix = checked.prefix;
        let is_admin = if revived.has_column(prefix, "users", "isAdmin") {
            "CAST(\"isAdmin\" AS INTEGER)"
        } else {
            "0"
        };
        let users: Vec<UserRow> = self.load(&format!(
            "SELECT CAST(\"userId\" AS TEXT) AS user_id, CAST(\"name\" AS TEXT) AS name, \
             CAST(\"notes\" AS INTEGER) AS notes, {is_admin} AS is_admin FROM \"users\" ORDER BY \"userId\""
        ))?;
        let mut accounts: Vec<Account> = users
            .into_iter()
            .map(|u| Account {
                user_id: u.user_id.unwrap_or_default(),
                name: u.name.unwrap_or_default(),
                notes: u.notes.unwrap_or(0),
                is_admin: u.is_admin.unwrap_or(0) != 0,
                ban: None,
                link: None,
            })
            .collect();

        if revived.has_table(prefix, "account_bans") {
            let (expires, live) = if revived.has_column(prefix, "account_bans", "expires_at") {
                (
                    "CAST(\"expires_at\" AS INTEGER)",
                    format!(
                        "(\"expires_at\" IS NULL OR CAST(\"expires_at\" AS INTEGER) > {})",
                        now.ms
                    ),
                )
            } else {
                ("NULL", String::from("1"))
            };
            let bans: Vec<BanRow> = self.load(&format!(
                "SELECT CAST(\"account_id\" AS TEXT) AS account_id, CAST(\"reason\" AS TEXT) AS reason, \
                 {expires} AS expires_at FROM \"account_bans\" WHERE \"active\" <> 0 AND {live}"
            ))?;
            for ban in bans {
                if let Some(account) = accounts
                    .iter_mut()
                    .find(|a| Some(&a.user_id) == ban.account_id.as_ref())
                {
                    account.ban = Some(Ban {
                        reason: ban.reason,
                        expires_at: ban.expires_at,
                    });
                }
            }
        }

        if revived.has_table(prefix, "discordlinks") {
            let links: Vec<LinkRow> = self.load(
                "SELECT CAST(\"discordId\" AS TEXT) AS discord_id, CAST(\"userId\" AS TEXT) AS user_id, \
                 CAST(\"linkedAt\" AS TEXT) AS linked_at FROM \"discordlinks\" ORDER BY \"discordId\"",
            )?;
            for link in links {
                let Some(discord_id) = link.discord_id else {
                    continue;
                };
                let linked_at = link
                    .linked_at
                    .as_deref()
                    .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                    .map_or_else(
                        || now.text.clone(),
                        |t| t.with_timezone(&chrono::Utc).format(TIME_FORMAT).to_string(),
                    );
                if let Some(account) = accounts.iter_mut().find(|a| Some(&a.user_id) == link.user_id.as_ref()) {
                    account.link = Some(Link { discord_id, linked_at });
                }
            }
        }

        let mut invites = Vec::new();
        if revived.has_table(prefix, "invitecodes") {
            let rows: Vec<InviteRow> = self.load(
                "SELECT CAST(\"invitecode\" AS TEXT) AS code, CAST(\"usesRemaining\" AS INTEGER) AS uses, \
                 CAST(\"infiniteUses\" AS INTEGER) AS infinite FROM \"invitecodes\" ORDER BY \"invitecode\"",
            )?;
            invites = rows
                .into_iter()
                .filter_map(|r| {
                    Some(Invite {
                        code: r.code?,
                        uses: r.uses.unwrap_or(0),
                        infinite: r.infinite.unwrap_or(0) != 0,
                    })
                })
                .collect();
        }

        let mut dropped = BTreeMap::new();
        for table in &checked.tables {
            if table.starts_with("sqlite_")
                || table.starts_with("__")
                || revived.carried.contains(table)
                || revived.not_player_data.contains(table)
            {
                continue;
            }
            let rows: Vec<CountRow> = self.load(&format!("SELECT count(*) AS n FROM {}", quote(table)))?;
            let n = rows.first().map_or(0, |r| r.n);
            if n > 0 {
                dropped.insert(table.clone(), u64::try_from(n).unwrap_or(0));
            }
        }
        Ok(Source {
            accounts,
            invites,
            dropped,
        })
    }

    fn check_holder(
        &mut self,
        revived: &Revived,
        checked: &Checked,
        user_id: &str,
        username: &str,
        key: &str,
    ) -> Result<(), ImportError> {
        if !revived.has_table(checked.prefix, "userapikeys") {
            return Err(ImportError::Invalid(NO_PLAYER.into()));
        }
        let hash = hex::encode(Sha256::digest(key.as_bytes()));
        let rows: Vec<HolderRow> = diesel::sql_query(
            "SELECT CAST(u.\"userId\" AS TEXT) AS user_id, CAST(u.\"name\" AS TEXT) AS name \
             FROM \"userapikeys\" k JOIN \"users\" u ON u.\"userId\" = k.\"userId\" WHERE k.\"keyHash\" = ?",
        )
        .bind::<Text, _>(hash)
        .load(&mut self.conn)
        .map_err(|e| ImportError::sqlite(self.path, e))?;
        let one =
            rows.len() == 1 && rows[0].user_id.as_deref() == Some(user_id) && rows[0].name.as_deref() == Some(username);
        if !one {
            return Err(ImportError::Invalid(NO_PLAYER.into()));
        }
        for table in revived.cleared.iter().filter(|t| checked.tables.contains(*t)) {
            let rows: Vec<CountRow> = self.load(&format!("SELECT count(*) AS n FROM {}", quote(table)))?;
            if rows.first().is_none_or(|r| r.n != 0) {
                return Err(ImportError::Invalid(NO_PLAYER.into()));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reference_texts_are_the_tools() {
        let (drizzle, diesel) = bookkeeping().unwrap();
        assert!(
            drizzle.starts_with("CREATE TABLE \"__drizzle_migrations\" ("),
            "{drizzle}"
        );
        assert!(diesel.contains("__diesel_schema_migrations"), "{diesel}");
        assert!(!drizzle.contains("  ") && !diesel.contains('\n'));
    }

    #[test]
    fn times() {
        assert_eq!(format_ms(0).unwrap(), "1970-01-01T00:00:00.000Z");
        assert_eq!(format_ms(1792200000002).unwrap(), "2026-10-17T01:20:00.002Z");
        let now = Now::current();
        assert_eq!(now.text.len(), 24);
    }
}
