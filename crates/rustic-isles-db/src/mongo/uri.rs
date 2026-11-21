// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! `mongodb://` URIs (spec §4.1). Every option but `authSource` is ignored, TLS and replica sets included.

use std::fmt;

use percent_encoding::percent_decode_str;

use super::MongoError;

/// A parsed `mongodb://` URI. `Debug` leaves out the password.
#[derive(Clone, PartialEq, Eq)]
pub struct MongoUri {
    /// Seed hosts in URI order, port 27017 when none is given.
    pub hosts: Vec<(String, u16)>,
    /// The database in the path, `dauntless` when it names none.
    pub database: String,
    /// The percent-decoded user name, if credentials were given.
    pub user: Option<String>,
    /// The percent-decoded password, if credentials were given.
    pub password: Option<String>,
    /// The database to authenticate against (`authSource`, default `admin`).
    pub auth_source: String,
}

// By hand: the password never reaches a log line (the decision "secrets never in Debug")
impl fmt::Debug for MongoUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MongoUri")
            .field("hosts", &self.hosts)
            .field("database", &self.database)
            .field("user", &self.user)
            .field("auth_source", &self.auth_source)
            .finish_non_exhaustive()
    }
}

fn bad(detail: impl Into<String>) -> MongoError {
    MongoError::InvalidUri(detail.into())
}

fn decode(text: &str, what: &str) -> Result<String, MongoError> {
    percent_decode_str(text)
        .decode_utf8()
        .map(|s| s.into_owned())
        .map_err(|e| bad(format!("{what}: {e}")))
}

impl MongoUri {
    /// Parses a `mongodb://` URI; SRV URIs and unknown forms are refused with [`MongoError::InvalidUri`].
    pub fn parse(text: &str) -> Result<Self, MongoError> {
        let rest = text
            .strip_prefix("mongodb://")
            .ok_or_else(|| bad("URI must start with mongodb://"))?;
        let (before, query) = rest.split_once('?').unwrap_or((rest, ""));
        let (authority, path) = before.split_once('/').unwrap_or((before, ""));
        let (credentials, host_list) = match authority.rsplit_once('@') {
            Some((c, h)) => (Some(c), h),
            None => (None, authority),
        };
        let (user, password) = match credentials {
            None => (None, None),
            Some(c) => {
                let (u, p) = c
                    .split_once(':')
                    .ok_or_else(|| bad("credentials must be username:password"))?;
                (
                    Some(decode(u, "invalid username encoding")?),
                    Some(decode(p, "invalid password encoding")?),
                )
            }
        };
        let mut hosts = Vec::new();
        for entry in host_list.split(',').map(str::trim).filter(|e| !e.is_empty()) {
            let (host, port) = match entry.rsplit_once(':') {
                Some((h, p)) => (
                    h,
                    p.parse::<u16>()
                        .map_err(|_| bad(format!("invalid port in {entry:?}")))?,
                ),
                None => (entry, 27017),
            };
            if host.is_empty() {
                return Err(bad("empty host in MongoDB URI"));
            }
            hosts.push((host.to_owned(), port));
        }
        if hosts.is_empty() {
            return Err(bad("no hosts in MongoDB URI"));
        }
        let database = decode(path.trim_matches('/'), "invalid database name")?;
        let mut auth_source = String::from("admin");
        for pair in query.split('&') {
            if let Some((k, v)) = pair.split_once('=')
                && k.eq_ignore_ascii_case("authSource")
                && !v.is_empty()
            {
                auth_source = decode(v, "invalid authSource")?;
            }
        }
        Ok(Self {
            hosts,
            database: if database.is_empty() {
                "dauntless".into()
            } else {
                database
            },
            user,
            password,
            auth_source,
        })
    }

    /// Both a user and a password: only then is SCRAM run.
    pub fn credentials(&self) -> Option<(&str, &str)> {
        Some((self.user.as_deref()?, self.password.as_deref()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_uri() {
        let uri = MongoUri::parse("mongodb://myuser:p%40ss@127.0.0.1:27017,127.0.0.2:27018/dauntless?authSource=admin")
            .unwrap();
        assert_eq!(
            uri.hosts,
            vec![("127.0.0.1".to_owned(), 27017), ("127.0.0.2".to_owned(), 27018)]
        );
        assert_eq!(uri.database, "dauntless");
        assert_eq!(uri.credentials(), Some(("myuser", "p@ss")));
        assert_eq!(uri.auth_source, "admin");
        assert!(!format!("{uri:?}").contains("p@ss"));
    }

    #[test]
    fn defaults_and_refusals() {
        let uri = MongoUri::parse("mongodb://db.local?AUTHSOURCE=users&tls=true").unwrap();
        assert_eq!(uri.database, "dauntless");
        assert_eq!(uri.hosts, vec![("db.local".to_owned(), 27017)]);
        assert_eq!(uri.auth_source, "users");
        assert_eq!(uri.credentials(), None);
        let err = |t: &str| MongoUri::parse(t).unwrap_err().to_string();
        assert_eq!(
            err("mongodb://user@host"),
            "invalid MongoDB URI: credentials must be username:password"
        );
        assert_eq!(
            err("mongodb+srv://h"),
            "invalid MongoDB URI: URI must start with mongodb://"
        );
        assert_eq!(
            err("mongodb://h:99999"),
            "invalid MongoDB URI: invalid port in \"h:99999\""
        );
        assert_eq!(err("mongodb://:1"), "invalid MongoDB URI: empty host in MongoDB URI");
        assert_eq!(err("mongodb:// , /x"), "invalid MongoDB URI: no hosts in MongoDB URI");
        assert_eq!(
            MongoUri::parse("mongodb://u:@h").unwrap().credentials(),
            Some(("u", ""))
        );
    }
}
