// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The `.drsave` container (spec §7.1): a header, JSON metadata, the SQLite database and a SHA-256 trailer.
//! Every read is bounded by the header's lengths, which are bounded first; nothing inside names a path.

use std::fmt;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::error::ImportError;
use crate::source::TIME_FORMAT;

/// The first 8 bytes of a container.
pub const MAGIC: &[u8; 8] = b"DRSAVE\r\n";
const HEADER: u64 = 24;
const TRAILER: u64 = 32;
const MIN_META: u32 = 2;
const MAX_META: u32 = 65536;
const MIN_DB: u64 = 512;
const MAX_DB: u64 = 536_870_912;
const CHUNK: usize = 64 * 1024;

const DAMAGED: &str = "not a Dauntless Revived offline save (.drsave), or a damaged one";

fn damaged() -> ImportError {
    ImportError::Invalid(DAMAGED.into())
}

/// The save's credential key, kept out of every `Debug`, log line and error.
pub(crate) struct Key(String);

impl Key {
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Key(..)")
    }
}

/// Whose save it is, as its metadata says.
#[derive(Debug)]
pub(crate) struct Save {
    pub user_id: String,
    pub username: String,
    pub key: Key,
    /// The extracted database.
    pub database: PathBuf,
}

#[derive(Deserialize)]
struct Meta {
    format: String,
    version: u64,
    #[serde(rename = "createdAt")]
    created_at: String,
    account: MetaAccount,
    database: MetaDatabase,
    credential: MetaCredential,
}

#[derive(Deserialize)]
struct MetaAccount {
    #[serde(rename = "userId")]
    user_id: String,
    username: String,
}

#[derive(Deserialize)]
struct MetaDatabase {
    bytes: u64,
    sha256: String,
}

#[derive(Deserialize)]
struct MetaCredential {
    version: u64,
    kind: String,
    key: String,
}

fn lower_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl Meta {
    fn valid(&self, database_bytes: u64) -> bool {
        let created = chrono::DateTime::parse_from_rfc3339(&self.created_at)
            .is_ok_and(|t| t.with_timezone(&chrono::Utc).format(TIME_FORMAT).to_string() == self.created_at);
        let user_id = &self.account.user_id;
        let username_units = self.account.username.encode_utf16().count();
        let key = &self.credential.key;
        self.format == "dauntless-revived-offline-save"
            && self.version == 1
            && created
            && (1..=128).contains(&user_id.len())
            && user_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            && (1..=128).contains(&username_units)
            && !self.account.username.contains(['\r', '\n', '\0'])
            && self.database.bytes == database_bytes
            && lower_hex(&self.database.sha256, 64)
            && self.credential.version == 1
            && self.credential.kind == "uuk-sha256"
            && key.strip_prefix("UUK_").is_some_and(|k| lower_hex(k, 48))
    }
}

/// Whether the file at `path` starts as a container does; `Ok(false)` for a shorter file.
pub(crate) fn is_container(path: &Path) -> Result<bool, ImportError> {
    let mut file = File::open(path).map_err(|e| ImportError::io(path, e))?;
    let mut head = [0u8; 8];
    let mut read = 0;
    while read < head.len() {
        match file.read(&mut head[read..]) {
            Ok(0) => break,
            Ok(n) => read += n,
            Err(e) => return Err(ImportError::io(path, e)),
        }
    }
    Ok(read == head.len() && &head == MAGIC)
}

fn be_u32(bytes: &[u8]) -> u32 {
    bytes.iter().take(4).fold(0u32, |n, b| (n << 8) | u32::from(*b))
}

fn be_u64(bytes: &[u8]) -> u64 {
    bytes.iter().take(8).fold(0u64, |n, b| (n << 8) | u64::from(*b))
}

/// Checks the container at `path` and streams its database into `dir/offline.db`, created new (0600 on Unix).
pub(crate) fn extract(path: &Path, dir: &Path) -> Result<Save, ImportError> {
    let mut file = File::open(path).map_err(|_| damaged())?;
    let meta = file.metadata().map_err(|_| damaged())?;
    if !meta.is_file() {
        return Err(damaged());
    }
    let mut header = [0u8; HEADER as usize];
    file.read_exact(&mut header).map_err(|_| damaged())?;
    if &header[..8] != MAGIC || be_u32(&header[8..12]) != 1 {
        return Err(damaged());
    }
    let meta_len = be_u32(&header[12..16]);
    let db_len = be_u64(&header[16..24]);
    if !(MIN_META..=MAX_META).contains(&meta_len) || !(MIN_DB..=MAX_DB).contains(&db_len) {
        return Err(damaged());
    }
    if meta.len() != HEADER + u64::from(meta_len) + db_len + TRAILER {
        return Err(damaged());
    }
    let mut running = Sha256::new();
    running.update(header);
    let mut json = vec![0u8; meta_len as usize];
    file.read_exact(&mut json).map_err(|_| damaged())?;
    running.update(&json);
    let parsed: Meta = serde_json::from_slice(&json).map_err(|_| damaged())?;
    if !parsed.valid(db_len) {
        return Err(damaged());
    }

    let database = dir.join("offline.db");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut out = options.open(&database).map_err(|e| ImportError::io(&database, e))?;
    let mut own = Sha256::new();
    let mut left = db_len;
    let mut buf = vec![0u8; CHUNK];
    while left > 0 {
        let take = usize::try_from(left).map_or(CHUNK, |l| l.min(CHUNK));
        let chunk = &mut buf[..take];
        file.read_exact(chunk).map_err(|_| damaged())?;
        running.update(&*chunk);
        own.update(&*chunk);
        out.write_all(chunk).map_err(|e| ImportError::io(&database, e))?;
        left -= take as u64;
    }
    out.flush().map_err(|e| ImportError::io(&database, e))?;
    drop(out);
    let mut trailer = [0u8; TRAILER as usize];
    file.read_exact(&mut trailer).map_err(|_| damaged())?;
    if running.finalize().as_slice() != trailer || hex::encode(own.finalize()) != parsed.database.sha256 {
        return Err(damaged());
    }
    Ok(Save {
        user_id: parsed.account.user_id,
        username: parsed.account.username,
        key: Key(parsed.credential.key),
        database,
    })
}

/// A container around `database`, for tests and the fuzz target: what a 1.4.4 client writes.
pub fn build(database: &[u8], user_id: &str, username: &str, key: &str, created_at: &str) -> Vec<u8> {
    let meta = serde_json::json!({
        "format": "dauntless-revived-offline-save",
        "version": 1,
        "createdAt": created_at,
        "account": {"userId": user_id, "username": username},
        "database": {"bytes": database.len(), "sha256": hex::encode(Sha256::digest(database))},
        "credential": {"version": 1, "kind": "uuk-sha256", "key": key},
    })
    .to_string();
    let mut out = Vec::with_capacity(database.len() + meta.len() + 56);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&1u32.to_be_bytes());
    out.extend_from_slice(&u32::try_from(meta.len()).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(&(database.len() as u64).to_be_bytes());
    out.extend_from_slice(meta.as_bytes());
    out.extend_from_slice(database);
    let trailer = Sha256::digest(&out);
    out.extend_from_slice(&trailer);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "UUK_0123456789abcdef0123456789abcdef0123456789abcdef";

    fn write(dir: &Path, bytes: &[u8]) -> PathBuf {
        let path = dir.join("save.drsave");
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn extracts_a_good_container() {
        let dir = tempfile::tempdir().unwrap();
        let db = vec![7u8; 1024];
        let path = write(dir.path(), &build(&db, "UID-1", "Sam", KEY, "2026-10-21T22:00:00.000Z"));
        assert!(is_container(&path).unwrap());
        let out = tempfile::tempdir().unwrap();
        let save = extract(&path, out.path()).unwrap();
        assert_eq!((save.user_id.as_str(), save.username.as_str()), ("UID-1", "Sam"));
        assert_eq!(save.key.expose(), KEY);
        assert!(!format!("{save:?}").contains(KEY));
        assert_eq!(std::fs::read(save.database).unwrap(), db);
    }

    #[test]
    fn refuses_every_damage() {
        let db = vec![7u8; 1024];
        let good = build(&db, "UID-1", "Sam", KEY, "2026-10-21T22:00:00.000Z");
        let meta_len = be_u32(&good[12..16]) as usize;
        let mut cases: Vec<Vec<u8>> = Vec::new();
        for at in [
            0,
            11,
            15,
            23,
            24,
            24 + meta_len / 2,
            24 + meta_len + 100,
            good.len() - 1,
        ] {
            let mut bad = good.clone();
            bad[at] ^= 0x01;
            cases.push(bad);
        }
        let mut short = good.clone();
        short.pop();
        cases.push(short);
        let mut long = good.clone();
        long.push(0);
        cases.push(long);
        for (created, user, name, key) in [
            ("2026-10-21T22:00:00Z", "UID-1", "Sam", KEY),
            ("2026-10-21T22:00:00.000Z", "UID 1", "Sam", KEY),
            ("2026-10-21T22:00:00.000Z", "UID-1", "Sa\nm", KEY),
            ("2026-10-21T22:00:00.000Z", "UID-1", "", KEY),
            ("2026-10-21T22:00:00.000Z", "UID-1", "Sam", "UUK_00"),
        ] {
            cases.push(build(&db, user, name, key, created));
        }
        cases.push(build(&[7u8; 100], "UID-1", "Sam", KEY, "2026-10-21T22:00:00.000Z"));
        for (i, bytes) in cases.iter().enumerate() {
            let dir = tempfile::tempdir().unwrap();
            let path = write(dir.path(), bytes);
            let out = tempfile::tempdir().unwrap();
            let err = extract(&path, out.path()).unwrap_err();
            assert_eq!(err.to_string(), DAMAGED, "case {i}");
        }
    }
}
