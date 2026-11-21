// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The store side's address (spec §9.1): a SQLite path, `sqlite://<path>`, or a Postgres URL.

use std::path::PathBuf;

use dainty_store::{Database, DatabaseConfig, DatabaseOptions, PostgresConfig};

use crate::DbError;

/// A Postgres URL, not percent-decoded (P11): a password holding `@`, `:` or `/` must be written as it is.
pub fn parse_postgres_url(text: &str, allow_remote: bool) -> Result<PostgresConfig, DbError> {
    let rest = text
        .strip_prefix("postgresql://")
        .or_else(|| text.strip_prefix("postgres://"))
        .ok_or_else(|| DbError::Config("Postgres URL must start with postgres://".into()))?;
    let rest = rest.split_once('?').map_or(rest, |(r, _)| r);
    let (authority, database) = rest
        .split_once('/')
        .ok_or_else(|| DbError::Config("Postgres URL must include /<database>".into()))?;
    let database = database.trim_matches('/');
    let (credentials, host_port) = match authority.rsplit_once('@') {
        Some((c, h)) => (Some(c), h),
        None => (None, authority),
    };
    let (user, password) = match credentials {
        None => ("postgres", ""),
        Some(c) => c.split_once(':').unwrap_or((c, "")),
    };
    let (host, port) = match host_port.rsplit_once(':') {
        Some((h, p)) => (
            h,
            p.parse::<u16>()
                .map_err(|_| DbError::Config(format!("invalid Postgres port in {host_port:?}")))?,
        ),
        None => (host_port, 5432),
    };
    let config = PostgresConfig {
        host: host.to_owned(),
        port,
        user: user.to_owned(),
        database: if database.is_empty() {
            "postgres".into()
        } else {
            database.to_owned()
        },
        password: password.to_owned(),
        allow_remote,
    };
    config.check_address().map_err(DbError::Config)?;
    Ok(config)
}

pub(crate) fn config(target: &str, allow_remote: bool) -> Result<DatabaseConfig, DbError> {
    let target = target.trim();
    if target.is_empty() {
        return Err(DbError::Config("--to target cannot be empty".into()));
    }
    if target.starts_with("postgres://") || target.starts_with("postgresql://") {
        return Ok(DatabaseConfig::Postgres(parse_postgres_url(target, allow_remote)?));
    }
    let path = target.strip_prefix("sqlite://").unwrap_or(target);
    Ok(DatabaseConfig::Sqlite {
        file: PathBuf::from(path),
    })
}

/// verify and export refuse a SQLite file that is not there rather than create an empty one (coordinator item 1:
/// a mistyped export source would empty MongoDB).
pub(crate) fn must_exist(config: &DatabaseConfig) -> Result<(), DbError> {
    match config {
        DatabaseConfig::Sqlite { file } if !file.exists() => Err(DbError::Config(format!(
            "the database {} does not exist",
            file.display()
        ))),
        _ => Ok(()),
    }
}

/// Opens the store at the latest migration (a missing SQLite file is created unless `existing`).
pub(crate) async fn open(target: &str, allow_remote: bool, existing: bool) -> Result<Database, DbError> {
    let config = config(target, allow_remote)?;
    if existing {
        must_exist(&config)?;
    }
    tokio::task::spawn_blocking(move || {
        Database::open(&config, &DatabaseOptions::default(), &rustic_isles_store::SCHEMA).map_err(DbError::Store)
    })
    .await
    .map_err(|_| DbError::Store(dainty_store::Error::Panicked))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_urls() {
        let c = parse_postgres_url("postgres://dainty:p@ss:w@127.0.0.1:5433/isles?sslmode=disable", false).unwrap();
        assert_eq!((c.user.as_str(), c.password.as_str()), ("dainty", "p@ss:w"));
        assert_eq!(
            (c.host.as_str(), c.port, c.database.as_str()),
            ("127.0.0.1", 5433, "isles")
        );
        let c = parse_postgres_url("postgresql://localhost/", false).unwrap();
        assert_eq!(
            (c.user.as_str(), c.database.as_str(), c.port),
            ("postgres", "postgres", 5432)
        );
        let err = |t: &str| parse_postgres_url(t, false).unwrap_err().to_string();
        assert_eq!(
            err("postgres://localhost"),
            "invalid argument: Postgres URL must include /<database>"
        );
        assert_eq!(
            err("postgres://h:x/d"),
            "invalid argument: invalid Postgres port in \"h:x\""
        );
        assert!(err("postgres://db.example.com/d").contains("not on this machine"));
        assert!(parse_postgres_url("postgres://db.example.com/d", true).is_ok());
        assert!(matches!(
            config(" sqlite://a.db ", false).unwrap(),
            DatabaseConfig::Sqlite { .. }
        ));
        assert_eq!(
            config("  ", false).unwrap_err().to_string(),
            "invalid argument: --to target cannot be empty"
        );
    }
}
