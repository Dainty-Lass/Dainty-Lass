//! Fixtures for the tests here and in the module crate (`test-support`): databases built at test time, so no real
//! player data is ever committed.

use std::path::Path;

use dainty_store::{Connection, DatabaseConfig, DatabaseOptions, drizzle};
use diesel::Connection as _;
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Bool, Nullable, Text};
use sha2::{Digest, Sha256};

use crate::revived::schema::{SCHEMA, drizzle_migrations};

/// A made-up offline-save key (DR's launcher test key)
pub const KEY: &str = concat!("UUK_", "0123456789abcdef", "0123456789abcdef", "0123456789abcdef");
pub const NOW_MS: i64 = 1_793_000_000_000;

fn open(file: &Path) -> Connection {
    #[allow(clippy::expect_used)]
    Connection::establish(&file.to_string_lossy()).expect("a fixture database opens")
}

fn run(conn: &mut Connection, sql: &str) {
    #[allow(clippy::expect_used)]
    conn.batch_execute(sql).expect("fixture SQL runs");
}

/// A 1.14.7 server's database with the account tables and isles' migration history, timeouts (0610) included.
pub fn isles_database(file: &Path) {
    isles_database_before_timeouts(file);
    let mut conn = open(file);
    run(
        &mut conn,
        "ALTER TABLE launcher_accounts ADD COLUMN timeout_until TEXT;
         ALTER TABLE launcher_accounts ADD COLUMN timeout_since TEXT;
         ALTER TABLE launcher_accounts ADD COLUMN timeout_reason TEXT;
         INSERT INTO __diesel_schema_migrations (version) VALUES ('0610');",
    );
}

/// The same, as rustic-isles made it before it kept timeouts.
pub fn isles_database_before_timeouts(file: &Path) {
    let mut conn = open(file);
    run(&mut conn, include_str!("testing/isles-1-14-7.sql"));
    // The other soft-deleted tables, which `store` puts triggers on: stand-ins with a key and `deleted_at`
    for table in [
        "characters",
        "loadouts",
        "inventories",
        "progression_tracks",
        "progression_objectives",
        "escalation_progress",
        "player_journey",
        "breadcrumbs",
        "encountered_content",
        "bounty_states",
        "cooldown_states",
        "entitlements",
        "wallets",
        "wallet_balances",
    ] {
        run(
            &mut conn,
            &format!("CREATE TABLE {table} (id TEXT PRIMARY KEY, deleted_at BIGINT);"),
        );
    }
    run(
        &mut conn,
        "CREATE TABLE __diesel_schema_migrations (version VARCHAR(50) PRIMARY KEY NOT NULL, \
         run_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP);
         INSERT INTO __diesel_schema_migrations (version) VALUES ('0001'), ('0020'), ('0200');",
    );
}

/// A 1.4.4 database the Node metagame made: the first `migrations` frozen Drizzle migrations.
pub fn drizzle_database(file: &Path, migrations: usize) -> Connection {
    let mut conn = open(file);
    let frozen = drizzle_migrations();
    #[allow(clippy::expect_used)]
    drizzle::replay(&mut conn, &frozen[..migrations]).expect("the frozen history replays");
    conn
}

/// A 1.4.4 database the Rust metagame made: the Diesel baseline.
pub fn diesel_database(file: &Path) -> Connection {
    let mut conn = open(file);
    #[allow(clippy::expect_used)]
    dainty_store::migrate(&mut conn, &SCHEMA).expect("the baseline runs");
    conn
}

/// Puts Dainty Lass's change log and triggers on a database, as `dainty_store::Database` does when it opens one.
pub fn add_change_log(file: &Path) {
    #[allow(clippy::expect_used)]
    let db = dainty_store::Database::attach(
        &DatabaseConfig::Sqlite {
            file: file.to_path_buf(),
        },
        &DatabaseOptions::default(),
        &[],
    )
    .expect("store attaches");
    drop(db);
}

pub struct Player<'a> {
    pub user_id: &'a str,
    pub name: &'a str,
    pub admin: bool,
    /// (active, expires_at in Unix ms)
    pub ban: Option<(bool, Option<i64>)>,
    pub discord: Option<&'a str>,
    pub key: Option<&'a str>,
    pub characters: usize,
}

impl<'a> Player<'a> {
    pub fn new(user_id: &'a str, name: &'a str) -> Self {
        Self {
            user_id,
            name,
            admin: false,
            ban: None,
            discord: None,
            key: None,
            characters: 0,
        }
    }
}

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A player of a database at the final schema (or late enough to have the tables used).
#[allow(clippy::expect_used)]
pub fn insert_player(conn: &mut Connection, player: &Player<'_>) {
    diesel::sql_query("INSERT INTO users (userId, name, notes, isAdmin) VALUES (?, ?, 3, ?)")
        .bind::<Text, _>(player.user_id)
        .bind::<Text, _>(player.name)
        .bind::<Bool, _>(player.admin)
        .execute(conn)
        .expect("a user");
    if let Some((active, expires_at)) = player.ban {
        diesel::sql_query(
            "INSERT INTO account_bans (account_id, reason, active, updated_at, actor, expires_at) \
             VALUES (?, 'griefing', ?, 1, 'admin', ?)",
        )
        .bind::<Text, _>(player.user_id)
        .bind::<BigInt, _>(i64::from(active))
        .bind::<Nullable<BigInt>, _>(expires_at)
        .execute(conn)
        .expect("a ban");
    }
    if let Some(discord) = player.discord {
        diesel::sql_query("INSERT INTO discordlinks (discordId, userId, linkedAt) VALUES (?, ?, ?)")
            .bind::<Text, _>(discord)
            .bind::<Text, _>(player.user_id)
            .bind::<Text, _>("2026-10-01T12:00:00.000Z")
            .execute(conn)
            .expect("a Discord link");
    }
    if let Some(key) = player.key {
        diesel::sql_query("INSERT INTO userapikeys (userId, keyHash) VALUES (?, ?)")
            .bind::<Text, _>(player.user_id)
            .bind::<Text, _>(sha256_hex(key))
            .execute(conn)
            .expect("a key");
    }
    for n in 0..player.characters {
        diesel::sql_query(
            "INSERT INTO characters (characterId, userId, createdDate, lastModifiedDate, name, updateVersion, data) \
             VALUES (?, ?, 'x', 'x', 'Slayer', 1, '{}')",
        )
        .bind::<Text, _>(format!("{}-c{n}", player.user_id))
        .bind::<Text, _>(player.user_id)
        .execute(conn)
        .expect("a character");
    }
}

#[allow(clippy::expect_used)]
pub fn insert_invite(conn: &mut Connection, code: &str, uses: i64, infinite: bool) {
    diesel::sql_query("INSERT INTO invitecodes (invitecode, usesRemaining, infiniteUses) VALUES (?, ?, ?)")
        .bind::<Text, _>(code)
        .bind::<BigInt, _>(uses)
        .bind::<Bool, _>(infinite)
        .execute(conn)
        .expect("an invite");
}

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = BigInt)]
    n: i64,
}

/// Rows of `table` matching `filter` in a database file.
#[allow(clippy::expect_used)]
pub fn count(file: &Path, table: &str, filter: &str) -> i64 {
    let mut conn = open(file);
    diesel::sql_query(format!("SELECT count(*) AS n FROM {table} WHERE {filter}"))
        .get_result::<Count>(&mut conn)
        .expect("a count")
        .n
}
