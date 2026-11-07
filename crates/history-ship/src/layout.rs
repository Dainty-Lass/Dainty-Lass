//! Object names and contents. Names carry everything a restore picks by, so the server never reads the
//! bucket back, and each ends with the SHA-256 of its body, so a body replaced under its name shows:
//!
//! - `{prefix}log/{after:019}-{through:019}-{sha256}.jsonl.gz`: the log entries with ids in
//!   `(after, through]`, `through` being the last one's. Each segment's `after` is the one before's
//!   `through`, so a missing segment is a gap a restore names.
//! - `{prefix}snapshot/{taken_at:013}-{last_id:019}-{sha256}.{sqlite|logical}.gz`: the database when
//!   `taken_at` (unix milliseconds) passed, holding the log through `last_id`.
//!
//! Numbers are zero-padded, so key order is id or time order and a listing can start after a given one.
//! A segment is gzipped JSON Lines: a [`SegmentHeader`], then one `history::record` line per entry (DH5), each
//! carrying its hash and the hash before it. The header repeats the last entry's hash as the segment's head.

use std::io::{Read, Write};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use store::ChangeEntry;
use store::history::{Chain, Head, record};
use store::recovery::SnapshotKind;

use crate::Error;

/// The folder under the prefix that holds log segments.
pub const LOG_DIR: &str = "log/";
/// The folder under the prefix that holds snapshots.
pub const SNAPSHOT_DIR: &str = "snapshot/";
/// [`SegmentHeader::format`] in every segment this crate writes.
pub const SEGMENT_FORMAT: &str = "dainty-lass-log";
const SEGMENT_VERSION: u32 = 1;
/// The most a segment or a snapshot may inflate to: a bomb from the bucket stops here.
pub const MAX_SEGMENT_BYTES: u64 = 256 << 20;
/// The most a snapshot may inflate to.
pub const MAX_SNAPSHOT_BYTES: u64 = 16 << 30;

/// A prefix is empty or ends in `/`, of letters, digits and `-_./`, with no empty or dot segment.
pub fn check_prefix(prefix: &str) -> Result<(), Error> {
    let ok = prefix.is_empty()
        || (prefix.ends_with('/')
            && prefix.len() <= 200
            && prefix
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
            && prefix
                .strip_suffix('/')
                .unwrap_or(prefix)
                .split('/')
                .all(|s| !s.is_empty() && s != "." && s != ".."));
    if ok {
        Ok(())
    } else {
        Err(Error::Config(format!(
            "the bucket prefix {prefix:?} must be empty or end in /, with letters, digits, - _ . and /"
        )))
    }
}

/// Lowercase hex SHA-256 of `data`, as object names carry it.
pub fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    ::hex::encode(bytes)
}

fn digits(text: &str, width: usize) -> Option<i64> {
    if text.len() == width && text.bytes().all(|b| b.is_ascii_digit()) {
        text.parse().ok()
    } else {
        None
    }
}

fn is_sha(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A log segment's object name, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentName {
    /// The entry id the segment starts after (exclusive).
    pub after: i64,
    /// The last entry id it holds (inclusive).
    pub through: i64,
    /// Lowercase hex SHA-256 of the gzipped body.
    pub sha256: String,
}

impl SegmentName {
    /// The object key under `prefix`.
    pub fn key(&self, prefix: &str) -> String {
        format!(
            "{prefix}{LOG_DIR}{:019}-{:019}-{}.jsonl.gz",
            self.after, self.through, self.sha256
        )
    }

    /// The name in `key`, or `None` for anything else under the prefix.
    pub fn parse(prefix: &str, key: &str) -> Option<Self> {
        let rest = key
            .strip_prefix(prefix)?
            .strip_prefix(LOG_DIR)?
            .strip_suffix(".jsonl.gz")?;
        let mut parts = rest.split('-');
        let after = digits(parts.next()?, 19)?;
        let through = digits(parts.next()?, 19)?;
        let sha256 = parts.next()?;
        if parts.next().is_some() || !is_sha(sha256) || through <= after {
            return None;
        }
        Some(Self {
            after,
            through,
            sha256: sha256.to_owned(),
        })
    }

    /// The key a listing starts after to find the segments that begin at `after` or later.
    pub fn start_after(prefix: &str, after: i64) -> String {
        format!("{prefix}{LOG_DIR}{after:019}")
    }
}

/// A snapshot's object name, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotName {
    /// When it was taken, unix milliseconds.
    pub taken_at: i64,
    /// The newest log entry id it holds.
    pub last_id: i64,
    /// SQLite file or logical dump.
    pub kind: SnapshotKind,
    /// Lowercase hex SHA-256 of the gzipped body.
    pub sha256: String,
}

impl SnapshotName {
    /// The object key under `prefix`.
    pub fn key(&self, prefix: &str) -> String {
        format!(
            "{prefix}{SNAPSHOT_DIR}{:013}-{:019}-{}.{}.gz",
            self.taken_at,
            self.last_id,
            self.sha256,
            self.kind.as_str()
        )
    }

    /// The name in `key`, or `None` for anything else under the prefix.
    pub fn parse(prefix: &str, key: &str) -> Option<Self> {
        let rest = key
            .strip_prefix(prefix)?
            .strip_prefix(SNAPSHOT_DIR)?
            .strip_suffix(".gz")?;
        let (stem, kind) = rest.rsplit_once('.')?;
        let kind = SnapshotKind::parse(kind)?;
        let mut parts = stem.split('-');
        let taken_at = digits(parts.next()?, 13)?;
        let last_id = digits(parts.next()?, 19)?;
        let sha256 = parts.next()?;
        if parts.next().is_some() || !is_sha(sha256) {
            return None;
        }
        Some(Self {
            taken_at,
            last_id,
            kind,
            sha256: sha256.to_owned(),
        })
    }
}

/// A segment's first line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentHeader {
    /// Always [`SEGMENT_FORMAT`].
    pub format: String,
    /// The segment layout version, 1 so far.
    pub version: u32,
    /// The entry id the segment starts after (exclusive).
    pub after: i64,
    /// The last entry id it holds (inclusive).
    pub through: i64,
    /// How many entries follow the header.
    pub count: usize,
    /// The last entry's hash: the head this segment ships (DH5).
    pub head_hash: String,
}

/// `data` gzipped at the default level.
pub fn gzip(data: &[u8]) -> Result<Vec<u8>, Error> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).map_err(|e| Error::Bucket(e.to_string()))?;
    encoder.finish().map_err(|e| Error::Bucket(e.to_string()))
}

/// `data` inflated; errors when it is not gzip or inflates past `limit` bytes.
pub fn gunzip(data: &[u8], limit: u64) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    GzDecoder::new(data)
        .take(limit.saturating_add(1))
        .read_to_end(&mut out)
        .map_err(|e| format!("not gzip: {e}"))?;
    if out.len() as u64 > limit {
        return Err(format!("inflates past {limit} bytes"));
    }
    Ok(out)
}

/// A segment's body, uncompressed, for entries after `after`. `entries` must be non-empty and oldest first.
pub fn encode_segment(after: i64, entries: &[ChangeEntry]) -> Result<(SegmentName, Vec<u8>), Error> {
    let through = entries
        .last()
        .map(|e| e.id)
        .ok_or_else(|| Error::Bucket("an empty segment".into()))?;
    let head_hash = entries
        .last()
        .and_then(|e| e.hash.clone())
        .ok_or_else(|| Error::Bucket("an unsealed entry can't be shipped".into()))?;
    let header = SegmentHeader {
        format: SEGMENT_FORMAT.into(),
        version: SEGMENT_VERSION,
        after,
        through,
        count: entries.len(),
        head_hash,
    };
    let mut text = serde_json::to_vec(&header).map_err(|e| Error::Bucket(e.to_string()))?;
    text.push(b'\n');
    for entry in entries {
        text.extend_from_slice(
            record::encode(entry)
                .map_err(|e| Error::Bucket(e.to_string()))?
                .as_bytes(),
        );
        text.push(b'\n');
    }
    let body = gzip(&text)?;
    let name = SegmentName {
        after,
        through,
        sha256: sha256_hex(&body),
    };
    Ok((name, body))
}

/// A segment's entries, checked against its name: the body's hash, the header, the ids in `(after, through]`,
/// increasing, the last one `through`, and every entry unchanged and chained onto the one before (DH5), so an
/// edited entry is named. Whether the first one follows the segment before is for the restore to check.
pub fn decode_segment(name: &SegmentName, body: &[u8]) -> Result<Vec<ChangeEntry>, String> {
    if sha256_hex(body) != name.sha256 {
        return Err("its body doesn't match the hash in its name".into());
    }
    let text = gunzip(body, MAX_SEGMENT_BYTES)?;
    let mut lines = text.split(|b| *b == b'\n').filter(|l| !l.is_empty());
    let header: SegmentHeader = lines
        .next()
        .and_then(|l| serde_json::from_slice(l).ok())
        .ok_or("no segment header")?;
    if header.format != SEGMENT_FORMAT || header.version != SEGMENT_VERSION {
        return Err(format!(
            "{} version {} is not a known format",
            header.format, header.version
        ));
    }
    if header.after != name.after || header.through != name.through {
        return Err("its header names other ids than its key".into());
    }
    let mut entries = Vec::with_capacity(header.count.min(100_000));
    let mut previous = name.after;
    let mut chain: Option<Chain> = None;
    for line in lines {
        let entry = record::decode(line).map_err(|e| e.to_string())?;
        if entry.id <= previous || entry.id > name.through {
            return Err(format!("entry {} is out of order or out of range", entry.id));
        }
        // The first entry names what came before it; the rest must name each other.
        let chain = chain.get_or_insert_with(|| {
            Chain::after(&Head {
                id: name.after,
                hash: entry.prev_hash.clone().unwrap_or_default(),
            })
        });
        chain.check(&entry).map_err(|b| b.to_string())?;
        previous = entry.id;
        entries.push(entry);
    }
    if entries.len() != header.count || previous != name.through {
        return Err("it holds fewer entries than its header says".into());
    }
    if entries.last().and_then(|e| e.hash.as_deref()) != Some(header.head_hash.as_str()) {
        return Err("its head isn't its last entry's hash".into());
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;

    use store::history::{GENESIS, entry_hash};

    /// Entries with these ids, chained from genesis.
    fn chained(ids: &[i64]) -> Vec<ChangeEntry> {
        let mut prev = GENESIS.to_owned();
        ids.iter()
            .map(|&id| {
                let mut entry = ChangeEntry {
                    id,
                    at: 1_700_000_000_000 + id,
                    actor: "player:1".into(),
                    table_name: "accounts".into(),
                    row_key: r#"{"id":1}"#.into(),
                    op: "insert".into(),
                    before: None,
                    after: Some(r#"{"id":1}"#.into()),
                    prev_hash: Some(prev.clone()),
                    hash: None,
                };
                entry.hash = Some(entry_hash(&prev, &entry));
                prev = entry.hash.clone().unwrap();
                entry
            })
            .collect()
    }

    #[test]
    fn names_round_trip_and_sort_by_number() {
        let segment = SegmentName {
            after: 9,
            through: 10,
            sha256: "a".repeat(64),
        };
        let key = segment.key("srv/");
        assert_eq!(
            key,
            format!(
                "srv/log/0000000000000000009-0000000000000000010-{}.jsonl.gz",
                "a".repeat(64)
            )
        );
        assert_eq!(SegmentName::parse("srv/", &key), Some(segment.clone()));
        assert!(SegmentName::start_after("srv/", 9) < key);
        assert!(SegmentName::start_after("srv/", 10) > key);
        let later = SegmentName {
            after: 10,
            through: 100,
            ..segment
        };
        assert!(later.key("srv/") > key);

        let snapshot = SnapshotName {
            taken_at: 1_761_480_000_000,
            last_id: 42,
            kind: SnapshotKind::Sqlite,
            sha256: "0f".repeat(32),
        };
        let key = snapshot.key("");
        assert_eq!(SnapshotName::parse("", &key), Some(snapshot));
    }

    #[test]
    fn other_keys_are_not_names() {
        let sha = "a".repeat(64);
        for key in [
            format!("log/1-2-{sha}.jsonl.gz"),
            format!("log/0000000000000000002-0000000000000000002-{sha}.jsonl.gz"),
            format!(
                "log/0000000000000000001-0000000000000000002-{}.jsonl.gz",
                "A".repeat(64)
            ),
            format!("log/0000000000000000001-0000000000000000002-{sha}-x.jsonl.gz"),
            format!("other/0000000000000000001-0000000000000000002-{sha}.jsonl.gz"),
            "log/".into(),
        ] {
            assert_eq!(SegmentName::parse("", &key), None, "{key}");
        }
        assert_eq!(
            SnapshotName::parse("", &format!("snapshot/1761480000000-0000000000000000001-{sha}.tar.gz")),
            None
        );
        assert_eq!(
            SnapshotName::parse(
                "p/",
                &format!("snapshot/1761480000000-0000000000000000001-{sha}.sqlite.gz")
            ),
            None
        );
    }

    #[test]
    fn prefixes() {
        for ok in ["", "a/", "srv-1/eu.west/"] {
            assert!(check_prefix(ok).is_ok(), "{ok}");
        }
        for bad in ["a", "/", "a//", "../", "a/../", "a b/", "ü/"] {
            assert!(check_prefix(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn segments_round_trip_and_refuse_what_their_name_does_not_say() {
        let entries = chained(&[3, 4, 7]);
        let (name, body) = encode_segment(2, &entries).unwrap();
        assert_eq!((name.after, name.through), (2, 7));
        assert_eq!(decode_segment(&name, &body).unwrap(), entries);

        // An entry edited in place, its hash left as it was, is named.
        let mut edited = entries.clone();
        edited[1].after = Some(r#"{"id":2}"#.into());
        let (name_e, body_e) = encode_segment(2, &edited).unwrap();
        assert_eq!(
            decode_segment(&name_e, &body_e).unwrap_err(),
            "entry 4 was changed after it was written"
        );
        // One dropped from the middle leaves the next not following the one before.
        let (name_d, body_d) = encode_segment(2, &[entries[0].clone(), entries[2].clone()]).unwrap();
        assert!(
            decode_segment(&name_d, &body_d)
                .unwrap_err()
                .contains("entry 7 doesn't follow entry 3")
        );

        let mut swapped = name.clone();
        swapped.through = 8;
        assert!(decode_segment(&swapped, &body).unwrap_err().contains("header"));
        let mut damaged = body.clone();
        damaged[20] ^= 1;
        assert!(decode_segment(&name, &damaged).unwrap_err().contains("hash"));

        // A body made to match a lying name is still refused by its header.
        let (other, body2) = encode_segment(3, &entries[1..]).unwrap();
        let lying = SegmentName {
            after: 2,
            sha256: other.sha256.clone(),
            ..other
        };
        assert!(decode_segment(&lying, &body2).unwrap_err().contains("header"));
    }

    #[test]
    fn a_bomb_stops_at_the_limit() {
        let zeros = gzip(&vec![0u8; 4096]).unwrap();
        assert!(gunzip(&zeros, 1024).unwrap_err().contains("inflates"));
        assert_eq!(gunzip(&zeros, 4096).unwrap().len(), 4096);
    }
}
