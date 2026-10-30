// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Which engine a database runs on, as `host.toml`'s `[database]` names it and setup writes it into a
//! service's env file (DD15). The variables are read here so every service reads them the same way.

use std::net::IpAddr;
use std::path::PathBuf;

/// `REFORGED_DB_ENGINE`: `sqlite` (the default) or `postgres`.
pub const ENV_ENGINE: &str = "REFORGED_DB_ENGINE";
/// `REFORGED_DB_FILE`: the SQLite file. Absent: the service's default.
pub const ENV_FILE: &str = "REFORGED_DB_FILE";
pub const ENV_HOST: &str = "REFORGED_DB_HOST";
pub const ENV_PORT: &str = "REFORGED_DB_PORT";
pub const ENV_USER: &str = "REFORGED_DB_USER";
pub const ENV_NAME: &str = "REFORGED_DB_NAME";
/// The Postgres password: a secret, made once in `secrets.env`.
pub const ENV_PASSWORD: &str = "REFORGED_DB_PASSWORD";
/// `1`: a Postgres address off this machine is accepted, though the connection is plaintext.
pub const ENV_ALLOW_REMOTE: &str = "REFORGED_DB_ALLOW_REMOTE";

pub const DEFAULT_POSTGRES_HOST: &str = "127.0.0.1";
pub const DEFAULT_POSTGRES_PORT: u16 = 5432;
pub const DEFAULT_POSTGRES_USER: &str = "dainty";
pub const DEFAULT_POSTGRES_DATABASE: &str = "dainty";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Sqlite,
    Postgres,
}

impl Engine {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Postgres => "postgres",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "sqlite" => Some(Self::Sqlite),
            "postgres" => Some(Self::Postgres),
            _ => None,
        }
    }
}

/// Where a Postgres database is and how to log in.
#[derive(Clone, PartialEq, Eq)]
pub struct PostgresConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub database: String,
    pub password: String,
    pub allow_remote: bool,
}

// By hand: the password stays out of every log line.
impl std::fmt::Debug for PostgresConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostgresConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("database", &self.database)
            .field("allow_remote", &self.allow_remote)
            .finish_non_exhaustive()
    }
}

impl PostgresConfig {
    /// libpq's keyword/value form. `sslmode=disable`: libpq is built without TLS (DF7 spike), so say so
    /// rather than let `prefer` fall back quietly.
    pub fn conninfo(&self) -> String {
        format!(
            "host={} port={} user={} dbname={} password={} sslmode=disable connect_timeout=10 application_name=dainty-lass",
            conninfo_value(&self.host),
            self.port,
            conninfo_value(&self.user),
            conninfo_value(&self.database),
            conninfo_value(&self.password),
        )
    }

    /// Refuses an address off this machine unless the host allowed it: the connection is plaintext.
    pub fn check_address(&self) -> Result<(), String> {
        if self.allow_remote || is_local_address(&self.host) {
            return Ok(());
        }
        Err(format!(
            "the Postgres server {} is not on this machine, and connections to it are not encrypted: \
             use 127.0.0.1 (or a Unix socket folder), or set allow_remote in host.toml's [database] \
             if the network between them is private",
            self.host
        ))
    }
}

/// A database as a service opens it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatabaseConfig {
    Sqlite { file: PathBuf },
    Postgres(PostgresConfig),
}

impl DatabaseConfig {
    pub fn engine(&self) -> Engine {
        match self {
            Self::Sqlite { .. } => Engine::Sqlite,
            Self::Postgres(_) => Engine::Postgres,
        }
    }

    /// From a service's environment (`lookup` returns a variable's value). Nothing set: SQLite at
    /// `default_file`, so an env file from before DF7 keeps today's behaviour.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>, default_file: PathBuf) -> Result<Self, String> {
        let get = |name: &str| lookup(name).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
        let engine = match get(ENV_ENGINE) {
            None => Engine::Sqlite,
            Some(text) => {
                Engine::parse(&text).ok_or_else(|| format!("{ENV_ENGINE} must be sqlite or postgres, not {text}"))?
            }
        };
        match engine {
            Engine::Sqlite => Ok(Self::Sqlite {
                file: get(ENV_FILE).map_or(default_file, PathBuf::from),
            }),
            Engine::Postgres => {
                let port = match get(ENV_PORT) {
                    None => DEFAULT_POSTGRES_PORT,
                    Some(text) => text
                        .parse::<u16>()
                        .ok()
                        .filter(|p| *p > 0)
                        .ok_or_else(|| format!("{ENV_PORT} must be a TCP port, not {text}"))?,
                };
                let allow_remote = match get(ENV_ALLOW_REMOTE).as_deref() {
                    None | Some("0") => false,
                    Some("1") => true,
                    Some(other) => return Err(format!("{ENV_ALLOW_REMOTE} must be 0 or 1, not {other}")),
                };
                let config = PostgresConfig {
                    host: get(ENV_HOST).unwrap_or_else(|| DEFAULT_POSTGRES_HOST.into()),
                    port,
                    user: get(ENV_USER).unwrap_or_else(|| DEFAULT_POSTGRES_USER.into()),
                    database: get(ENV_NAME).unwrap_or_else(|| DEFAULT_POSTGRES_DATABASE.into()),
                    password: get(ENV_PASSWORD).ok_or_else(|| format!("{ENV_PASSWORD} is not set"))?,
                    allow_remote,
                };
                config.check_address()?;
                Ok(Self::Postgres(config))
            }
        }
    }
}

/// `localhost`, a loopback address, or a Unix socket folder.
pub fn is_local_address(host: &str) -> bool {
    if host == "localhost" || host.starts_with('/') {
        return true;
    }
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<IpAddr>()
        .is_ok_and(|ip| ip.is_loopback())
}

/// A conninfo value in single quotes, backslash and quote escaped (libpq's rule).
fn conninfo_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for c in value.chars() {
        if c == '\'' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use std::collections::HashMap;

    fn read(pairs: &[(&str, &str)]) -> Result<DatabaseConfig, String> {
        let map: HashMap<String, String> = pairs.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect();
        DatabaseConfig::from_lookup(|name| map.get(name).cloned(), PathBuf::from("default.db"))
    }

    #[test]
    fn nothing_set_is_sqlite_at_the_default_file() {
        assert_eq!(
            read(&[]).unwrap(),
            DatabaseConfig::Sqlite {
                file: "default.db".into()
            }
        );
        assert_eq!(
            read(&[(ENV_ENGINE, "sqlite"), (ENV_FILE, "/srv/x.db")]).unwrap(),
            DatabaseConfig::Sqlite {
                file: "/srv/x.db".into()
            }
        );
    }

    #[test]
    fn postgres_needs_a_password_and_a_local_address() {
        assert!(read(&[(ENV_ENGINE, "postgres")]).unwrap_err().contains(ENV_PASSWORD));
        let ok = read(&[(ENV_ENGINE, "postgres"), (ENV_PASSWORD, "pw")]).unwrap();
        let DatabaseConfig::Postgres(pg) = ok else {
            panic!("postgres")
        };
        assert_eq!((pg.host.as_str(), pg.port), ("127.0.0.1", 5432));
        let far = [
            (ENV_ENGINE, "postgres"),
            (ENV_PASSWORD, "pw"),
            (ENV_HOST, "db.example.org"),
        ];
        assert!(read(&far).unwrap_err().contains("not encrypted"));
        let allowed = [far.as_slice(), &[(ENV_ALLOW_REMOTE, "1")]].concat();
        assert!(read(&allowed).is_ok());
        assert!(read(&[(ENV_ENGINE, "mysql")]).is_err());
    }

    #[test]
    fn local_addresses() {
        for local in [
            "127.0.0.1",
            "127.8.0.1",
            "::1",
            "[::1]",
            "localhost",
            "/var/run/postgresql",
        ] {
            assert!(is_local_address(local), "{local}");
        }
        for far in ["10.0.0.2", "db", "192.168.1.1", "::"] {
            assert!(!is_local_address(far), "{far}");
        }
    }

    #[test]
    fn conninfo_quotes_values_and_debug_hides_the_password() {
        let pg = PostgresConfig {
            host: "127.0.0.1".into(),
            port: 5432,
            user: "o'neil".into(),
            database: "d".into(),
            password: "p\\w d".into(),
            allow_remote: false,
        };
        let info = pg.conninfo();
        assert!(info.contains(r"user='o\'neil'"), "{info}");
        assert!(info.contains(r"password='p\\w d'"), "{info}");
        assert!(!format!("{pg:?}").contains("p\\\\w"));
    }
}
