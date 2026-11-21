// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

use diesel::prelude::*;
use diesel::sql_types::Text;

use crate::{Connection, Error};

#[derive(QueryableByName)]
struct CatalogObject {
    #[diesel(sql_type = Text)]
    sql: String,
}

/// Returns a deterministic text snapshot of user-defined schema objects in `conn`.
pub fn schema_snapshot(conn: &mut Connection) -> Result<String, Error> {
    let objects = diesel::sql_query(
        "SELECT sql FROM sqlite_schema
         WHERE sql IS NOT NULL
           AND name NOT LIKE 'sqlite_%'
           AND name NOT IN ('__drizzle_migrations', '__diesel_schema_migrations')
           AND tbl_name NOT IN ('__drizzle_migrations', '__diesel_schema_migrations')
         ORDER BY
           CASE type
             WHEN 'table' THEN 1
             WHEN 'index' THEN 2
             WHEN 'view' THEN 3
             ELSE 4
           END,
           name",
    )
    .load::<CatalogObject>(conn)?;

    let mut out = String::new();
    for obj in objects {
        out.push_str(&obj.sql.replace("\r\n", "\n"));
        out.push_str(";\n");
    }
    Ok(out)
}
