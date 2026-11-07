// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! MongoDB discovery (`mongod` on PATH and Windows Program Files), command-line argument construction
//! for a 1-member loopback replica set (`rs0`), and wire-protocol replica-set inspection/initiation.

pub mod wire;

use std::ffi::OsStr;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::error::HostError;
use crate::paths::PORTS;
use wire::{Doc, Value, as_bool, as_f64, as_str, command, find};

/// Default replica set name used by the host kit.
pub const REPLICA_SET: &str = "rs0";
/// Default WiredTiger cache size (in GiB) passed to managed `mongod`.
pub const MONGO_CACHE_GB: u32 = 1;

/// Failure categories when probing a MongoDB instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MongoProblem {
    /// No way to talk to MongoDB in this build.
    NoDriver,
    /// Nothing answered at the URI's host and port.
    Unreachable,
    /// A mongod answered that runs without a replica set.
    Standalone,
    /// MongoDB answered with an error, or the URI was bad.
    Error,
}

/// Outcome of probing or initiating the MongoDB replica set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MongoCheck {
    /// The replica set answers.
    Ok {
        /// Its name.
        set_name: String,
        /// Whether the node is primary.
        primary: bool,
        /// Whether this check initiated it.
        initiated: bool,
    },
    /// The check failed.
    Failed {
        /// Which kind of failure.
        reason: MongoProblem,
        /// What MongoDB or the connection said.
        detail: String,
    },
}

/// Options for [`ensure_replica_set`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaSetOptions {
    /// Runs `replSetInitiate` when the node has no replica set yet.
    pub initiate: bool,
    /// The member `host:port` to initiate with; the URI's host and port when `None`.
    pub host: Option<String>,
    /// How long to wait for the node, in milliseconds; 8000 when `None`.
    pub timeout_ms: Option<u64>,
}

impl Default for ReplicaSetOptions {
    fn default() -> Self {
        Self {
            initiate: true,
            host: None,
            timeout_ms: Some(8000),
        }
    }
}

/// Builds the command-line arguments for starting a managed 1-member `mongod` replica set on loopback.
pub fn mongod_args(data_dir: &Path, log_file: &Path, port: u16) -> Result<Vec<String>, HostError> {
    Ok(vec![
        "--replSet".into(),
        REPLICA_SET.into(),
        "--bind_ip".into(),
        "127.0.0.1".into(),
        "--port".into(),
        port.to_string(),
        "--dbpath".into(),
        data_dir.display().to_string(),
        "--logpath".into(),
        log_file.display().to_string(),
        "--logappend".into(),
        "--wiredTigerCacheSizeGB".into(),
        MONGO_CACHE_GB.to_string(),
    ])
}

/// Returns the primary Windows `Program Files` directory (`ProgramW6432` or `ProgramFiles`).
#[must_use]
pub fn program_files() -> PathBuf {
    std::env::var_os("ProgramW6432")
        .or_else(|| std::env::var_os("ProgramFiles"))
        .map_or_else(|| PathBuf::from(r"C:\Program Files"), PathBuf::from)
}

/// Helper trait allowing [`search_path`] to accept either a single binary name (`&str`) or a slice (`&[&str]`).
pub trait SearchNames<'a> {
    /// The names to look for.
    fn as_names(&self) -> Vec<&'a str>;
}

impl<'a> SearchNames<'a> for &'a str {
    fn as_names(&self) -> Vec<&'a str> {
        vec![*self]
    }
}

impl<'a> SearchNames<'a> for &'a [&'a str] {
    fn as_names(&self) -> Vec<&'a str> {
        self.to_vec()
    }
}

impl<'a, const N: usize> SearchNames<'a> for [&'a str; N] {
    fn as_names(&self) -> Vec<&'a str> {
        self.to_vec()
    }
}

impl<'a, const N: usize> SearchNames<'a> for &'a [&'a str; N] {
    fn as_names(&self) -> Vec<&'a str> {
        self.to_vec()
    }
}

/// Searches `path` (`PATH`) for the first existing executable matching `names`.
#[must_use]
pub fn search_path<'a>(names: impl SearchNames<'a>, path: Option<&OsStr>) -> Option<PathBuf> {
    let path = path?;
    let name_list = names.as_names();
    for dir in std::env::split_paths(path) {
        let cleaned = dir.to_string_lossy();
        let cleaned = cleaned.trim().trim_matches('"');
        if cleaned.is_empty() {
            continue;
        }
        let dir_path = Path::new(cleaned);
        for &name in &name_list {
            let candidate = dir_path.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
            if cfg!(windows) && !name.to_ascii_lowercase().ends_with(".exe") {
                let with_exe = dir_path.join(format!("{name}.exe"));
                if with_exe.is_file() {
                    return Some(with_exe);
                }
            }
        }
    }
    None
}

fn version_sort_key(name: &str) -> (Vec<u64>, String) {
    let numbers: Vec<u64> = name
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<u64>().ok())
        .collect();
    (numbers, name.to_owned())
}

/// Finds `<base>/<version>/bin/<exe>` for the highest version directory under `base`.
#[must_use]
pub fn newest_installed(base: &Path, exe: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(base).ok()?;
    let mut found: Vec<((Vec<u64>, String), PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let ver_name = entry.file_name().to_string_lossy().into_owned();
        let bin = path.join("bin").join(exe);
        if bin.is_file() {
            found.push((version_sort_key(&ver_name), bin));
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found.pop().map(|(_, p)| p)
}

/// Searches `path` (`PATH`) and an optional Windows `Program Files` directory for `mongod` / `mongod.exe`.
#[must_use]
pub fn find_mongod_in(path: Option<&OsStr>, program_files_dir: Option<&Path>) -> Option<String> {
    let exe = if cfg!(windows) { "mongod.exe" } else { "mongod" };
    if let Some(on_path) = search_path([exe, "mongod", "mongod.exe"], path) {
        return Some(on_path.display().to_string());
    }
    if let Some(pf) = program_files_dir {
        let server_dir = pf.join("MongoDB").join("Server");
        if let Some(installed) =
            newest_installed(&server_dir, "mongod.exe").or_else(|| newest_installed(&server_dir, "mongod"))
        {
            return Some(installed.display().to_string());
        }
    }
    None
}

/// Finds `mongod` from an explicit `configured` path, `PATH`, or `%ProgramFiles%\MongoDB\Server\<ver>\bin\mongod.exe`.
pub fn find_mongod(configured: Option<&str>) -> Result<Option<String>, HostError> {
    if let Some(explicit) = configured.map(str::trim).filter(|s| !s.is_empty()) {
        let p = Path::new(explicit);
        if p.is_file() {
            return Ok(Some(explicit.to_owned()));
        }
        return Err(HostError::message(format!(
            "mongod was not found at configured path: {explicit}"
        )));
    }
    let pf = cfg!(windows).then(program_files);
    Ok(find_mongod_in(std::env::var_os("PATH").as_deref(), pf.as_deref()))
}

/// Converts a MongoDB replica-set URI into a `directConnection=true` URI for single-node administration.
#[must_use]
pub fn direct_uri(uri: &str) -> String {
    let trimmed = dainty_service_kit::js::trim(uri);
    let (base, query_opt) = match trimmed.split_once('?') {
        Some((b, q)) => (b, Some(q)),
        None => (trimmed, None),
    };
    if let Some(q) = query_opt {
        let mut has_direct = false;
        let mut replaced = false;
        let mut params: Vec<String> = Vec::new();
        for part in q.split('&') {
            if part.is_empty() {
                continue;
            }
            if part.starts_with("replicaSet=") {
                if !has_direct {
                    params.push("directConnection=true".into());
                    has_direct = true;
                }
                replaced = true;
            } else {
                if part.starts_with("directConnection=") {
                    has_direct = true;
                }
                params.push(part.to_owned());
            }
        }
        if !replaced && !has_direct {
            params.push("directConnection=true".into());
        }
        format!("{base}?{}", params.join("&"))
    } else {
        let after_scheme = base.strip_prefix("mongodb://").unwrap_or(base);
        if after_scheme.contains('/') {
            format!("{base}?directConnection=true")
        } else {
            format!("{base}/?directConnection=true")
        }
    }
}

/// Parses the first `(host, port)` from a `mongodb://` connection URI.
pub fn first_host(uri: &str) -> Result<(String, u16), String> {
    let trimmed = dainty_service_kit::js::trim(uri);
    if trimmed.starts_with("mongodb+srv://") {
        return Err("mongodb+srv:// is not supported for a local replica set".into());
    }
    let Some(rest) = trimmed.strip_prefix("mongodb://") else {
        return Err("URI must start with mongodb://".into());
    };
    let authority = rest.split(['/', '?']).next().unwrap_or("");
    let hosts_part = match authority.rsplit_once('@') {
        Some((_, h)) => h,
        None => authority,
    };
    let first = hosts_part.split(',').next().unwrap_or("").trim();
    if first.is_empty() {
        return Err("URI has no host".into());
    }
    if let Some(after_bracket) = first.strip_prefix('[') {
        let Some((ipv6, port_part)) = after_bracket.split_once(']') else {
            return Err(format!("unclosed IPv6 bracket in URI host: {first}"));
        };
        if ipv6.is_empty() {
            return Err("empty IPv6 address in URI".into());
        }
        let port = if port_part.is_empty() {
            PORTS.mongo
        } else if let Some(p_str) = port_part.strip_prefix(':') {
            p_str
                .parse::<u16>()
                .ok()
                .filter(|&p| p > 0)
                .ok_or_else(|| format!("invalid port in URI: {p_str}"))?
        } else {
            return Err(format!("invalid host:port in URI: {first}"));
        };
        return Ok((ipv6.to_owned(), port));
    }
    if let Some((host, port_str)) = first.rsplit_once(':') {
        if host.is_empty() {
            return Err("URI has no host".into());
        }
        let port = port_str
            .parse::<u16>()
            .ok()
            .filter(|&p| p > 0)
            .ok_or_else(|| format!("invalid port in URI: {port_str}"))?;
        Ok((host.to_owned(), port))
    } else {
        Ok((first.to_owned(), PORTS.mongo))
    }
}

fn resolve_addr(host: &str, port: u16) -> Result<SocketAddr, String> {
    if host.eq_ignore_ascii_case("localhost") {
        return Ok(SocketAddr::from(([127, 0, 0, 1], port)));
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, port));
    }
    (host, port)
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .next()
        .ok_or_else(|| format!("could not resolve {host}:{port}"))
}

fn is_primary(doc: &Doc) -> bool {
    as_bool(find(doc, "isWritablePrimary")).unwrap_or(false) || as_bool(find(doc, "ismaster")).unwrap_or(false)
}

fn hello_doc() -> Doc {
    vec![
        ("hello".into(), Value::Int32(1)),
        ("$db".into(), Value::Str("admin".into())),
    ]
}

/// Connects to the first host in `uri`, inspects its replica-set status via `hello` / `replSetGetStatus`,
/// and (when `opts.initiate` is `true`) runs `replSetInitiate` if the node is not yet initialized.
pub fn ensure_replica_set(uri: &str, opts: &ReplicaSetOptions) -> Result<MongoCheck, HostError> {
    let (host, port) = match first_host(uri) {
        Ok(hp) => hp,
        Err(detail) => {
            return Ok(MongoCheck::Failed {
                reason: MongoProblem::Error,
                detail,
            });
        }
    };
    let addr = match resolve_addr(&host, port) {
        Ok(a) => a,
        Err(detail) => {
            return Ok(MongoCheck::Failed {
                reason: MongoProblem::Unreachable,
                detail,
            });
        }
    };

    let timeout = Duration::from_millis(opts.timeout_ms.unwrap_or(8000));
    let deadline = Instant::now() + timeout;
    let cmd_timeout = timeout.min(Duration::from_millis(3000)).max(Duration::from_millis(200));

    let hello = match command(addr, &hello_doc(), cmd_timeout) {
        Ok(doc) => doc,
        Err(e) => {
            return Ok(MongoCheck::Failed {
                reason: MongoProblem::Unreachable,
                detail: e.to_string(),
            });
        }
    };

    if let Some(set_name) = as_str(find(&hello, "setName")) {
        let set_name = set_name.to_owned();
        if is_primary(&hello) || !opts.initiate {
            return Ok(MongoCheck::Ok {
                set_name,
                primary: is_primary(&hello),
                initiated: false,
            });
        }
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(150));
            if let Ok(doc) = command(addr, &hello_doc(), cmd_timeout)
                && is_primary(&doc)
            {
                return Ok(MongoCheck::Ok {
                    set_name,
                    primary: true,
                    initiated: false,
                });
            }
        }
        return Ok(MongoCheck::Ok {
            set_name,
            primary: false,
            initiated: false,
        });
    }

    // Node responded to hello without `setName`. Check `replSetGetStatus` to distinguish standalone vs uninitiated.
    let status_cmd: Doc = vec![
        ("replSetGetStatus".into(), Value::Int32(1)),
        ("$db".into(), Value::Str("admin".into())),
    ];
    let status_doc = match command(addr, &status_cmd, cmd_timeout) {
        Ok(doc) => doc,
        Err(e) => {
            return Ok(MongoCheck::Failed {
                reason: MongoProblem::Error,
                detail: e.to_string(),
            });
        }
    };

    let code = find(&status_doc, "code").and_then(Value::as_i64);
    let errmsg = as_str(find(&status_doc, "errmsg")).unwrap_or("").to_owned();
    let is_repl_flag = as_bool(find(&hello, "isreplicaset")).unwrap_or(false);

    if code == Some(76)
        || errmsg.contains("--replSet")
        || (!is_repl_flag && code != Some(94) && as_f64(find(&status_doc, "ok")) != Some(1.0))
    {
        return Ok(MongoCheck::Failed {
            reason: MongoProblem::Standalone,
            detail: format!(
                "mongod is running without --replSet {REPLICA_SET} ({})",
                if errmsg.is_empty() { "standalone" } else { &errmsg }
            ),
        });
    }

    if !opts.initiate {
        return Ok(MongoCheck::Failed {
            reason: MongoProblem::Error,
            detail: if code == Some(94) || errmsg.is_empty() {
                "the replica set is not initiated yet".into()
            } else {
                errmsg
            },
        });
    }

    let member_host = opts.host.clone().unwrap_or_else(|| format!("{host}:{port}"));
    let init_cmd: Doc = vec![
        (
            "replSetInitiate".into(),
            Value::Doc(vec![
                ("_id".into(), Value::Str(REPLICA_SET.into())),
                (
                    "members".into(),
                    Value::Array(vec![Value::Doc(vec![
                        ("_id".into(), Value::Int32(0)),
                        ("host".into(), Value::Str(member_host)),
                    ])]),
                ),
            ]),
        ),
        ("$db".into(), Value::Str("admin".into())),
    ];

    let init_res = match command(addr, &init_cmd, cmd_timeout) {
        Ok(doc) => doc,
        Err(e) => {
            return Ok(MongoCheck::Failed {
                reason: MongoProblem::Error,
                detail: e.to_string(),
            });
        }
    };

    let init_ok = as_f64(find(&init_res, "ok")) == Some(1.0);
    let init_code = find(&init_res, "code").and_then(Value::as_i64);
    // Code 23 is AlreadyInitialized
    if !init_ok && init_code != Some(23) {
        let msg = as_str(find(&init_res, "errmsg"))
            .unwrap_or("replSetInitiate failed")
            .to_owned();
        return Ok(MongoCheck::Failed {
            reason: MongoProblem::Error,
            detail: msg,
        });
    }

    let initiated = init_code != Some(23);
    while Instant::now() < deadline {
        if let Ok(doc) = command(addr, &hello_doc(), cmd_timeout)
            && is_primary(&doc)
        {
            let set_name = as_str(find(&doc, "setName")).unwrap_or(REPLICA_SET).to_owned();
            return Ok(MongoCheck::Ok {
                set_name,
                primary: true,
                initiated,
            });
        }
        std::thread::sleep(Duration::from_millis(150));
    }

    Ok(MongoCheck::Ok {
        set_name: REPLICA_SET.into(),
        primary: false,
        initiated,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn builds_mongod_args_and_rewrites_direct_uri() {
        let args = mongod_args(Path::new("/data/mongo"), Path::new("/logs/mongo.log"), 27017).unwrap();
        assert_eq!(
            args,
            [
                "--replSet",
                "rs0",
                "--bind_ip",
                "127.0.0.1",
                "--port",
                "27017",
                "--dbpath",
                "/data/mongo",
                "--logpath",
                "/logs/mongo.log",
                "--logappend",
                "--wiredTigerCacheSizeGB",
                "1",
            ]
        );

        assert_eq!(
            direct_uri("mongodb://127.0.0.1:27017/?replicaSet=rs0"),
            "mongodb://127.0.0.1:27017/?directConnection=true"
        );
        assert_eq!(
            direct_uri("mongodb://127.0.0.1:27017/reforged?replicaSet=rs0&w=majority"),
            "mongodb://127.0.0.1:27017/reforged?directConnection=true&w=majority"
        );
        assert_eq!(
            direct_uri("mongodb://127.0.0.1:27017"),
            "mongodb://127.0.0.1:27017/?directConnection=true"
        );
    }

    #[test]
    fn parses_first_host_and_rejects_srv() {
        assert_eq!(
            first_host("mongodb://127.0.0.1:27017/?replicaSet=rs0").unwrap(),
            ("127.0.0.1".into(), 27017)
        );
        assert_eq!(
            first_host("mongodb://user:pass@localhost:27018,other:27019/db").unwrap(),
            ("localhost".into(), 27018)
        );
        assert_eq!(first_host("mongodb://[::1]:27020/db").unwrap(), ("::1".into(), 27020));
        assert_eq!(
            first_host("mongodb://127.0.0.1/db").unwrap(),
            ("127.0.0.1".into(), 27017)
        );
        assert!(
            first_host("mongodb+srv://cluster.example.net/db")
                .unwrap_err()
                .contains("mongodb+srv://")
        );
    }

    #[test]
    fn picks_newest_installed_mongodb_version_in_program_files() {
        let dir = tempfile::tempdir().unwrap();
        let server = dir.path().join("MongoDB").join("Server");
        for ver in ["6.0", "7.0.2", "7.0.14", "8.0"] {
            let bin = server.join(ver).join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            std::fs::write(bin.join("mongod.exe"), b"bin").unwrap();
        }
        let picked = newest_installed(&server, "mongod.exe").unwrap();
        assert_eq!(picked, server.join("8.0").join("bin").join("mongod.exe"));
    }

    #[test]
    fn initiates_uninitialized_replica_set_over_wire_protocol() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let server = std::thread::spawn(move || {
            for step in 0..3 {
                let (mut sock, _) = listener.accept().unwrap();
                let mut len_buf = [0u8; 4];
                sock.read_exact(&mut len_buf).unwrap();
                let total = i32::from_le_bytes(len_buf) as usize;
                let mut frame = vec![0u8; total];
                frame[..4].copy_from_slice(&len_buf);
                sock.read_exact(&mut frame[4..]).unwrap();
                let req = wire::parse_op_msg(&frame).unwrap();

                let reply: Doc = match step {
                    0 => {
                        assert!(find(&req, "hello").is_some());
                        vec![
                            ("ok".into(), Value::Double(1.0)),
                            ("isreplicaset".into(), Value::Bool(true)),
                        ]
                    }
                    1 => {
                        assert!(find(&req, "replSetGetStatus").is_some());
                        vec![
                            ("ok".into(), Value::Double(0.0)),
                            ("code".into(), Value::Int32(94)),
                            ("errmsg".into(), Value::Str("no replset config".into())),
                        ]
                    }
                    2 => {
                        assert!(find(&req, "replSetInitiate").is_some());
                        vec![("ok".into(), Value::Double(1.0))]
                    }
                    _ => unreachable!(),
                };
                sock.write_all(&wire::op_msg(&reply)).unwrap();
            }
            let (mut sock, _) = listener.accept().unwrap();
            let mut len_buf = [0u8; 4];
            sock.read_exact(&mut len_buf).unwrap();
            let total = i32::from_le_bytes(len_buf) as usize;
            let mut frame = vec![0u8; total];
            frame[..4].copy_from_slice(&len_buf);
            sock.read_exact(&mut frame[4..]).unwrap();
            let reply: Doc = vec![
                ("ok".into(), Value::Double(1.0)),
                ("setName".into(), Value::Str("rs0".into())),
                ("isWritablePrimary".into(), Value::Bool(true)),
            ];
            sock.write_all(&wire::op_msg(&reply)).unwrap();
        });

        let uri = format!("mongodb://127.0.0.1:{port}/?replicaSet=rs0");
        let check = ensure_replica_set(
            &uri,
            &ReplicaSetOptions {
                initiate: true,
                host: None,
                timeout_ms: Some(3000),
            },
        )
        .unwrap();
        assert_eq!(
            check,
            MongoCheck::Ok {
                set_name: "rs0".into(),
                primary: true,
                initiated: true,
            }
        );
        server.join().unwrap();
    }
}
