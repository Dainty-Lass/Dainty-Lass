//! The `.drsave` container of DR's launcher (`src-tauri/src/offline/save_format.rs` at legacy-revived `fac6efd`,
//! `runtime/OFFLINE_SAVE_FORMAT.md`): an 8-byte magic, the version, the metadata and database sizes (big endian), the
//! metadata JSON, exactly one SQLite database, and the SHA-256 of everything before it. No names or paths are read
//! from the file; one bounded payload is copied out. The digests find damage; they do not prove who wrote the file.

use std::io::{Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SAVE_MAGIC: &[u8; 8] = b"DRSAVE\r\n";
pub const HEADER_BYTES: usize = 24;
pub const MAX_METADATA_BYTES: usize = 65536;
pub const MAX_DATABASE_BYTES: u64 = 512 * 1024 * 1024;
const MIN_DATABASE_BYTES: u64 = 512;

#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("not a Dauntless Revived offline save (.drsave), or a damaged one")]
    Invalid,
    #[error("offline save file: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveAccount {
    pub user_id: String,
    pub username: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveDatabase {
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveCredential {
    pub version: u32,
    pub kind: String,
    pub key: String,
}

// By hand: the save's key stays out of every log line
impl std::fmt::Debug for SaveCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveCredential")
            .field("version", &self.version)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveMetadata {
    pub format: String,
    pub version: u32,
    pub created_at: String,
    pub account: SaveAccount,
    pub database: SaveDatabase,
    pub credential: SaveCredential,
}

/// `text.length` in JavaScript: UTF-16 code units (DR `core/text.rs`)
fn js_length(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// `new Date(createdAt).toISOString() === createdAt`
fn canonical_time(text: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(text).is_ok_and(|time| {
        time.with_timezone(&chrono::Utc)
            .format("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string()
            == text
    })
}

fn validate(metadata: &SaveMetadata, bytes: u64) -> Result<(), FormatError> {
    let user_id_ok = (1..=128).contains(&metadata.account.user_id.len())
        && metadata
            .account
            .user_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    let username_length = js_length(&metadata.account.username);
    let username_ok = (1..=128).contains(&username_length) && !metadata.account.username.contains(['\r', '\n', '\0']);
    let hex = |text: &str, length: usize| {
        text.len() == length && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    };
    let key_ok = metadata
        .credential
        .key
        .strip_prefix("UUK_")
        .is_some_and(|rest| hex(rest, 48));
    let ok = metadata.format == "dauntless-revived-offline-save"
        && metadata.version == 1
        && canonical_time(&metadata.created_at)
        && user_id_ok
        && username_ok
        && metadata.database.bytes == bytes
        && hex(&metadata.database.sha256, 64)
        && metadata.credential.version == 1
        && metadata.credential.kind == "uuk-sha256"
        && key_ok;
    if ok { Ok(()) } else { Err(FormatError::Invalid) }
}

fn hex_of(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)
}

fn database_digest(database: &Path) -> std::io::Result<String> {
    let mut input = std::fs::File::open(database)?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 65536];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            return Ok(hex_of(&hash.finalize()));
        }
        hash.update(&buffer[..read]);
    }
}

/// A save's account and its backup credential.
#[derive(Clone, PartialEq, Eq)]
pub struct PortableIdentity {
    pub user_id: String,
    pub username: String,
    pub key: String,
}

/// Writes a new save file (it must not exist) holding `database` and `identity`, as DR's launcher does. Kept for
/// the tests and the fuzz corpus: this module only ever reads saves.
pub fn write_save(
    file: &Path,
    database: &Path,
    identity: &PortableIdentity,
    created_at: &str,
) -> Result<(), FormatError> {
    let bytes = std::fs::metadata(database)?.len();
    if !(MIN_DATABASE_BYTES..=MAX_DATABASE_BYTES).contains(&bytes) {
        return Err(FormatError::Invalid);
    }
    let metadata = SaveMetadata {
        format: "dauntless-revived-offline-save".into(),
        version: 1,
        created_at: created_at.to_owned(),
        account: SaveAccount {
            user_id: identity.user_id.clone(),
            username: identity.username.clone(),
        },
        database: SaveDatabase {
            bytes,
            sha256: database_digest(database)?,
        },
        credential: SaveCredential {
            version: 1,
            kind: "uuk-sha256".into(),
            key: identity.key.clone(),
        },
    };
    validate(&metadata, bytes)?;
    let json = serde_json::to_vec(&metadata).map_err(|_| FormatError::Invalid)?;
    let json_len = u32::try_from(json.len()).map_err(|_| FormatError::Invalid)?;
    if json.len() > MAX_METADATA_BYTES {
        return Err(FormatError::Invalid);
    }
    let mut header = [0u8; HEADER_BYTES];
    header[..8].copy_from_slice(SAVE_MAGIC);
    header[8..12].copy_from_slice(&1u32.to_be_bytes());
    header[12..16].copy_from_slice(&json_len.to_be_bytes());
    header[16..24].copy_from_slice(&bytes.to_be_bytes());
    let mut output = create_private(file)?;
    let mut digest = Sha256::new();
    for part in [&header[..], &json[..]] {
        output.write_all(part)?;
        digest.update(part);
    }
    let mut input = std::fs::File::open(database)?;
    let mut buffer = vec![0u8; 65536];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read])?;
        digest.update(&buffer[..read]);
    }
    output.write_all(&digest.finalize())?;
    output.sync_all()?;
    Ok(())
}

/// Checks a save and copies its database to `database` (which must not exist); its metadata.
pub fn read_save(file: &Path, database: &Path) -> Result<SaveMetadata, FormatError> {
    let mut input = std::fs::File::open(file)?;
    let stat = input.metadata()?;
    if !stat.is_file() || stat.len() < (HEADER_BYTES + 32) as u64 {
        return Err(FormatError::Invalid);
    }
    let size = stat.len();
    let mut header = [0u8; HEADER_BYTES];
    input.read_exact(&mut header).map_err(|_| FormatError::Invalid)?;
    let word = |at: usize| u32::from_be_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]]);
    let metadata_bytes = word(12) as usize;
    let mut database_bytes = [0u8; 8];
    database_bytes.copy_from_slice(&header[16..24]);
    let database_bytes = u64::from_be_bytes(database_bytes);
    if &header[..8] != SAVE_MAGIC
        || word(8) != 1
        || !(2..=MAX_METADATA_BYTES).contains(&metadata_bytes)
        || !(MIN_DATABASE_BYTES..=MAX_DATABASE_BYTES).contains(&database_bytes)
        || size != (HEADER_BYTES + metadata_bytes + 32) as u64 + database_bytes
    {
        return Err(FormatError::Invalid);
    }
    let mut json = vec![0u8; metadata_bytes];
    input.read_exact(&mut json).map_err(|_| FormatError::Invalid)?;
    let metadata: SaveMetadata = serde_json::from_slice(&json).map_err(|_| FormatError::Invalid)?;
    validate(&metadata, database_bytes)?;

    let mut digest = Sha256::new();
    digest.update(header);
    digest.update(&json);
    let mut database_digest = Sha256::new();
    let mut output = create_private(database)?;
    let mut buffer = vec![0u8; 65536];
    let mut remaining = database_bytes;
    while remaining > 0 {
        let want = buffer.len().min(usize::try_from(remaining).unwrap_or(buffer.len()));
        let read = input.read(&mut buffer[..want])?;
        if read == 0 {
            return Err(FormatError::Invalid);
        }
        digest.update(&buffer[..read]);
        database_digest.update(&buffer[..read]);
        output.write_all(&buffer[..read])?;
        remaining -= read as u64;
    }
    let mut trailer = [0u8; 32];
    input.read_exact(&mut trailer).map_err(|_| FormatError::Invalid)?;
    if trailer[..] != digest.finalize()[..] || hex_of(&database_digest.finalize()) != metadata.database.sha256 {
        return Err(FormatError::Invalid);
    }
    output.sync_all()?;
    Ok(metadata)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = concat!("UUK_", "0123456789abcdef", "0123456789abcdef", "0123456789abcdef");
    const WHEN: &str = "2026-10-21T22:00:00.000Z";

    fn identity() -> PortableIdentity {
        PortableIdentity {
            user_id: "UID-1".into(),
            username: "Sam".into(),
            key: KEY.into(),
        }
    }

    #[test]
    fn a_save_round_trips_and_any_damage_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("db");
        std::fs::write(&database, (0..4096u32).map(|i| (i % 251) as u8).collect::<Vec<_>>()).unwrap();
        let save = dir.path().join("a.drsave");
        write_save(&save, &database, &identity(), WHEN).unwrap();
        assert!(
            write_save(&save, &database, &identity(), WHEN).is_err(),
            "never overwrites"
        );

        let out = dir.path().join("out.db");
        let metadata = read_save(&save, &out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), std::fs::read(&database).unwrap());
        assert_eq!(
            (metadata.account.user_id.as_str(), metadata.credential.key.as_str()),
            ("UID-1", KEY)
        );
        assert!(!format!("{metadata:?}").contains(KEY), "the key stays out of Debug");

        let bytes = std::fs::read(&save).unwrap();
        for at in [0, 9, 13, 30, bytes.len() / 2, bytes.len() - 1] {
            let mut damaged = bytes.clone();
            damaged[at] ^= 1;
            let file = dir.path().join(format!("d{at}.drsave"));
            std::fs::write(&file, &damaged).unwrap();
            assert!(
                matches!(
                    read_save(&file, &dir.path().join(format!("d{at}.db"))),
                    Err(FormatError::Invalid)
                ),
                "byte {at}"
            );
        }
        let short = dir.path().join("short.drsave");
        std::fs::write(&short, &bytes[..bytes.len() - 1]).unwrap();
        assert!(read_save(&short, &dir.path().join("short.db")).is_err());
    }

    #[test]
    fn metadata_rules() {
        let good = SaveMetadata {
            format: "dauntless-revived-offline-save".into(),
            version: 1,
            created_at: WHEN.into(),
            account: SaveAccount {
                user_id: "UID-1".into(),
                username: "Sam".into(),
            },
            database: SaveDatabase {
                bytes: 512,
                sha256: "a".repeat(64),
            },
            credential: SaveCredential {
                version: 1,
                kind: "uuk-sha256".into(),
                key: KEY.into(),
            },
        };
        assert!(validate(&good, 512).is_ok());
        type Change = Box<dyn Fn(&mut SaveMetadata)>;
        let cases: Vec<Change> = vec![
            Box::new(|m| m.created_at = "2026-10-21T22:00:00Z".into()),
            Box::new(|m| m.account.user_id = "a/b".into()),
            Box::new(|m| m.account.username = "a\nb".into()),
            Box::new(|m| m.account.username = String::new()),
            Box::new(|m| m.database.sha256 = "A".repeat(64)),
            Box::new(|m| m.credential.key = "UUK_short".into()),
            Box::new(|m| m.credential.kind = "other".into()),
            Box::new(|m| m.version = 2),
        ];
        for (index, change) in cases.iter().enumerate() {
            let mut metadata = good.clone();
            change(&mut metadata);
            assert!(validate(&metadata, 512).is_err(), "case {index}");
        }
        assert!(validate(&good, 513).is_err());
    }
}
