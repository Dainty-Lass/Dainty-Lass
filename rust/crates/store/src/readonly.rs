// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Read-only SQLite connection opening and applied Diesel migration version lookup.

use std::path::Path;
use std::time::Duration;

use diesel::connection::{Connection as _, SimpleConnection};
use diesel::prelude::*;
use diesel::sql_types::Text;

use crate::{Connection, Error};

#[derive(QueryableByName)]
struct TextCell {
    #[diesel(sql_type = Text)]
    value: String,
}

/// Opens an existing SQLite database at `path` in read-only mode (`mode=ro`).
pub fn open_read_only(path: &Path, busy_timeout: Duration) -> Result<Connection, Error> {
    let escaped = path
        .to_string_lossy()
        .replace('%', "%25")
        .replace('?', "%3f")
        .replace('#', "%23");
    let uri = format!("file:{escaped}?mode=ro");

    let mut conn = Connection::establish(&uri)?;
    conn.batch_execute(&format!(
        "PRAGMA busy_timeout = {}; PRAGMA query_only = ON;",
        busy_timeout.as_millis()
    ))?;
    Ok(conn)
}

/// Returns the highest applied Diesel migration version string, or `Ok(None)` if unmigrated.
pub fn schema_version(conn: &mut Connection) -> Result<Option<String>, Error> {
    let exists = diesel::sql_query(
        "SELECT name AS value FROM sqlite_schema
         WHERE type = 'table' AND name = '__diesel_schema_migrations'
         LIMIT 1",
    )
    .get_result::<TextCell>(conn)
    .optional()?;

    if exists.is_none() {
        return Ok(None);
    }

    let latest =
        diesel::sql_query("SELECT version AS value FROM __diesel_schema_migrations ORDER BY version DESC LIMIT 1")
            .get_result::<TextCell>(conn)
            .optional()?;

    Ok(latest.map(|r| r.value))
}
