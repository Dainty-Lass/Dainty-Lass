// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Read-only SQLite connection helper and schema version lookup.

use std::path::Path;
use std::time::Duration;

use diesel::connection::{Connection as _, SimpleConnection};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Text};

use crate::{Connection, Error};

fn format_ro_uri(abs: &Path) -> String {
    let raw = abs.to_string_lossy();
    let stripped = raw.strip_prefix(r"\\?\").unwrap_or(&raw);
    let normalized = stripped.replace('\\', "/");
    let mut encoded = String::with_capacity(normalized.len() + 16);
    for &b in normalized.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/' | b':') {
            encoded.push(char::from(b));
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }
    if encoded.starts_with('/') {
        format!("file://{encoded}?mode=ro")
    } else {
        format!("file:///{encoded}?mode=ro")
    }
}

pub fn open_read_only(path: &Path, busy_timeout: Duration) -> Result<Connection, Error> {
    let abs = std::path::absolute(path)
        .map_err(|e| Error::Connection(diesel::ConnectionError::BadConnection(e.to_string())))?;
    let uri = format_ro_uri(&abs);
    let mut conn = Connection::establish(&uri)?;
    let ms = busy_timeout.as_millis();
    conn.batch_execute(&format!("PRAGMA busy_timeout = {ms}; PRAGMA query_only = ON;"))?;
    Ok(conn)
}

#[derive(QueryableByName)]
struct CountRow {
    #[diesel(sql_type = BigInt)]
    n: i64,
}

#[derive(QueryableByName)]
struct VersionRow {
    #[diesel(sql_type = Text)]
    version: String,
}

pub fn schema_version(conn: &mut Connection) -> Result<Option<String>, Error> {
    let exists = diesel::sql_query(
        "SELECT count(*) AS n FROM sqlite_master WHERE type = 'table' AND name = '__diesel_schema_migrations'",
    )
    .get_result::<CountRow>(conn)?;
    if exists.n == 0 {
        return Ok(None);
    }
    let rows = diesel::sql_query(
        "SELECT version FROM __diesel_schema_migrations ORDER BY version DESC LIMIT 1",
    )
    .load::<VersionRow>(conn)?;
    Ok(rows.into_iter().next().map(|r| r.version))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn ro_uri_formats_unix_and_windows_paths_and_encodes_special_chars() {
        let unix = Path::new("/var/lib/my db#1?.sqlite");
        assert_eq!(
            format_ro_uri(unix),
            "file:///var/lib/my%20db%231%3F.sqlite?mode=ro"
        );
        let win = Path::new(r"C:\data\my db\test.sqlite");
        assert_eq!(
            format_ro_uri(win),
            "file:///C:/data/my%20db/test.sqlite?mode=ro"
        );
    }

    #[test]
    fn missing_file_is_not_created_by_open_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does_not_exist.db");
        assert!(open_read_only(&missing, Duration::from_secs(1)).is_err());
        assert!(!missing.exists());
    }
}
