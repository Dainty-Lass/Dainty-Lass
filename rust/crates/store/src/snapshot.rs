// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Canonical SQL text representation of a SQLite database schema.

use diesel::prelude::*;
use diesel::sql_types::Text;

use crate::{Connection, Error};

#[derive(QueryableByName)]
struct SqlRow {
    #[diesel(sql_type = Text)]
    sql: String,
}

pub fn schema_snapshot(conn: &mut Connection) -> Result<String, Error> {
    let rows = diesel::sql_query(
        "SELECT sql FROM sqlite_schema
         WHERE sql IS NOT NULL
           AND name NOT LIKE 'sqlite!_%' ESCAPE '!'
           AND tbl_name NOT IN ('__drizzle_migrations', '__diesel_schema_migrations')
         ORDER BY CASE type WHEN 'table' THEN 0 WHEN 'index' THEN 1 WHEN 'view' THEN 2 ELSE 3 END, name",
    )
    .load::<SqlRow>(conn)?;

    let mut out = String::new();
    for row in rows {
        let normalized = row.sql.replace("\r\n", "\n");
        out.push_str(&normalized);
        out.push_str(";\n");
    }
    Ok(out)
}
