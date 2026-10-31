// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use diesel::connection::SimpleConnection;
use diesel::migration::MigrationSource;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Text};
use diesel::sqlite::Sqlite;
use diesel_migrations::{EmbeddedMigrations, MigrationHarness};

use crate::{Connection, Error, drizzle, schema_snapshot};

#[derive(Clone, Copy)]
pub struct Schema {
    pub migrations: fn() -> EmbeddedMigrations,
    pub drizzle: Option<DrizzleBaseline>,
}

#[derive(Debug, Clone, Copy)]
pub struct DrizzleBaseline {
    pub created_at: i64,
    pub sql: &'static str,
    pub snapshot: &'static str,
}

#[derive(QueryableByName)]
struct CountRow {
    #[diesel(sql_type = BigInt)]
    n: i64,
}

fn table_exists(conn: &mut Connection, name: &str) -> Result<bool, Error> {
    let row = diesel::sql_query("SELECT count(*) AS n FROM sqlite_schema WHERE type = 'table' AND name = $1")
        .bind::<Text, _>(name)
        .get_result::<CountRow>(conn)?;
    Ok(row.n > 0)
}

fn has_user_objects(conn: &mut Connection) -> Result<bool, Error> {
    let row = diesel::sql_query("SELECT count(*) AS n FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'")
        .get_result::<CountRow>(conn)?;
    Ok(row.n > 0)
}

pub(crate) fn apply_schema(conn: &mut Connection, schema: &Schema) -> Result<(), Error> {
    if table_exists(conn, "__diesel_schema_migrations")? {
        conn.run_pending_migrations((schema.migrations)())
            .map_err(|e| Error::Migration(e.to_string()))?;
        return Ok(());
    }

    if table_exists(conn, "__drizzle_migrations")? {
        let baseline = schema.drizzle.ok_or_else(|| {
            Error::Adoption("database has __drizzle_migrations but no Drizzle baseline is configured".into())
        })?;
        adopt_drizzle(conn, schema, &baseline)?;
        conn.run_pending_migrations((schema.migrations)())
            .map_err(|e| Error::Migration(e.to_string()))?;
        return Ok(());
    }

    if has_user_objects(conn)? {
        return Err(Error::Adoption(
            "database contains existing schema objects without migration tracking tables".into(),
        ));
    }

    conn.run_pending_migrations((schema.migrations)())
        .map_err(|e| Error::Migration(e.to_string()))?;
    Ok(())
}

fn adopt_drizzle(conn: &mut Connection, schema: &Schema, baseline: &DrizzleBaseline) -> Result<(), Error> {
    let (created_at, recorded_hash) =
        drizzle::last_applied(conn)?.ok_or_else(|| Error::Adoption("__drizzle_migrations table is empty".into()))?;

    if created_at != Some(baseline.created_at) {
        return Err(Error::Adoption(format!(
            "last Drizzle migration created_at is {:?}, expected {}",
            created_at, baseline.created_at
        )));
    }

    if !drizzle::hash_matches(&recorded_hash, baseline.sql) {
        return Err(Error::Adoption(format!(
            "last Drizzle migration hash {} does not match baseline SQL",
            recorded_hash
        )));
    }

    let actual_snapshot = schema_snapshot(conn)?;
    let expected_snapshot = baseline.snapshot.replace("\r\n", "\n");
    if actual_snapshot != expected_snapshot {
        return Err(Error::Adoption(
            "live database schema does not match the Drizzle baseline snapshot".into(),
        ));
    }

    let embedded = (schema.migrations)();
    let migrations = MigrationSource::<Sqlite>::migrations(&embedded).map_err(|e| Error::Migration(e.to_string()))?;
    let first = migrations
        .first()
        .ok_or_else(|| Error::Migration("embedded migration list is empty".into()))?;
    let baseline_version = first.name().version().to_string();

    conn.batch_execute(
        "CREATE TABLE IF NOT EXISTS __diesel_schema_migrations (
            version VARCHAR(50) PRIMARY KEY NOT NULL,
            run_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
        );",
    )?;
    diesel::sql_query("INSERT INTO __diesel_schema_migrations (version) VALUES ($1)")
        .bind::<Text, _>(baseline_version)
        .execute(conn)?;

    Ok(())
}
