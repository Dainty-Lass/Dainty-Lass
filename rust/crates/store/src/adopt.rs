// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Schema preparation and Drizzle-to-Diesel baseline adoption.

use diesel::migration::MigrationSource;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Text};
use diesel::sqlite::Sqlite;
use diesel_migrations::{EmbeddedMigrations, MigrationHarness};

use crate::snapshot::schema_snapshot;
use crate::{Connection, Error, drizzle};

pub struct Schema {
    pub migrations: fn() -> EmbeddedMigrations,
    pub drizzle: Option<DrizzleBaseline>,
}

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

fn non_sqlite_object_count(conn: &mut Connection) -> Result<i64, Error> {
    let row = diesel::sql_query("SELECT count(*) AS n FROM sqlite_schema WHERE name NOT LIKE 'sqlite!_%' ESCAPE '!'")
        .get_result::<CountRow>(conn)?;
    Ok(row.n)
}

fn run_pending(conn: &mut Connection, migrations: EmbeddedMigrations) -> Result<(), Error> {
    conn.run_pending_migrations(migrations)
        .map(|_| ())
        .map_err(|e| Error::Migration(e.to_string()))
}

pub(crate) fn prepare(conn: &mut Connection, schema: &Schema) -> Result<(), Error> {
    if table_exists(conn, "__diesel_schema_migrations")? {
        return run_pending(conn, (schema.migrations)());
    }

    if table_exists(conn, "__drizzle_migrations")? {
        let Some(baseline) = &schema.drizzle else {
            return Err(Error::Adoption("no Drizzle baseline configured".into()));
        };
        let migrations = (schema.migrations)();
        let list = MigrationSource::<Sqlite>::migrations(&migrations).map_err(|e| Error::Migration(e.to_string()))?;
        let first = list
            .first()
            .ok_or_else(|| Error::Migration("no Diesel migrations defined".into()))?;
        let first_version = first.name().version().to_string();

        conn.immediate_transaction(|conn| {
            let Some((created_at, hash)) = drizzle::last_applied(conn)? else {
                return Err(Error::Adoption("__drizzle_migrations is empty".into()));
            };
            if created_at != Some(baseline.created_at) {
                return Err(Error::Adoption(format!(
                    "expected baseline created_at {}, got {:?}",
                    baseline.created_at, created_at
                )));
            }
            if !drizzle::hash_matches(&hash, baseline.sql) {
                return Err(Error::Adoption("baseline SQL hash does not match".into()));
            }
            let current_snapshot = schema_snapshot(conn)?;
            let expected_snapshot = baseline.snapshot.replace("\r\n", "\n");
            if current_snapshot != expected_snapshot {
                return Err(Error::Adoption("its schema differs from the baseline's".into()));
            }
            conn.applied_migrations().map_err(|e| Error::Migration(e.to_string()))?;
            diesel::sql_query("INSERT INTO __diesel_schema_migrations (version) VALUES ($1)")
                .bind::<Text, _>(&first_version)
                .execute(conn)?;
            Ok(())
        })?;

        return run_pending(conn, (schema.migrations)());
    }

    if non_sqlite_object_count(conn)? == 0 {
        return run_pending(conn, (schema.migrations)());
    }

    Err(Error::Adoption("it has tables but no migration history".into()))
}
