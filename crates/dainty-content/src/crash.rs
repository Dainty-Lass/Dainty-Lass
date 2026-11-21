// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Game crash reports a player's launcher sends (`POST /content/v1/crash-reports`,
//! `docs/security/crash-reports.md`): the bundle's parser, the per-account rate limit, the folder they are kept
//! in and the copy sent to the staff dashboard.
//!
//! A bundle is gzip over `DLCR1\n`, one line of header JSON, then each file as a big-endian `u16` name length, the
//! name, a big-endian `u32` data length and the data (the launcher kit's `platform::crash` writes it).

use std::collections::{HashMap, VecDeque};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use flate2::read::GzDecoder;
use serde_json::{Map, Value, json};

use crate::{Clock, sha256_hex};

/// The route.
pub const PATH: &str = "/content/v1/crash-reports";
/// The only content type taken.
pub const CONTENT_TYPE: &str = "application/vnd.dainty.crash+gzip";
/// Largest upload: the gateway's default body limit (`GATEWAY_MAX_BODY_BYTES`).
pub const MAX_UPLOAD: usize = 128 * 1024;
/// Largest bundle once unpacked, so a small upload can't unpack into a large one.
pub const MAX_UNPACKED: usize = 8 * 1024 * 1024;
/// Most files in a bundle.
pub const MAX_FILES: usize = 16;
/// Longest file name.
pub const MAX_NAME: usize = 80;
/// Longest header line.
pub const MAX_HEADER: usize = 2048;
/// Longest header value.
pub const MAX_VALUE: usize = 128;
/// Uploads one account may make per [`WINDOW`].
pub const PER_ACCOUNT: usize = 3;
/// Uploads all accounts together may make per [`WINDOW`].
pub const TOTAL: usize = 60;
/// The rate limit's window.
pub const WINDOW: Duration = Duration::from_secs(3600);
/// Reports kept; the oldest go first.
pub const KEEP_REPORTS: usize = 200;
/// Bytes kept across all reports; the oldest go first.
pub const KEEP_BYTES: u64 = 256 * 1024 * 1024;
/// Longest text file copied to the dashboard, in bytes.
pub const DASHBOARD_TEXT: usize = 256 * 1024;

const MAGIC: &[u8] = b"DLCR1\n";
const HEADER_KEYS: [&str; 4] = ["crashId", "gameVersion", "launcherVersion", "exitCode"];
const TEXT_EXTENSIONS: [&str; 5] = [".runtime-xml", ".log", ".xml", ".txt", ".json"];
const DUMP_EXTENSION: &str = ".dmp";
const META_FILE: &str = "meta.json";

/// Why a bundle was refused; each is the reply's `reason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadBundle {
    /// Not gzip, or truncated.
    Gzip,
    /// Unpacks past [`MAX_UNPACKED`].
    TooLarge,
    /// No `DLCR1` line.
    Magic,
    /// The header line is missing, too long, not JSON or holds anything else.
    Header,
    /// A file's frame is cut short.
    Frame,
    /// A file name isn't a plain name with a known extension.
    Name,
    /// The same name twice.
    Duplicate,
    /// More than [`MAX_FILES`].
    TooMany,
}

impl BadBundle {
    /// The log's reason.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Gzip => "bad_gzip",
            Self::TooLarge => "unpacked_too_large",
            Self::Magic => "bad_magic",
            Self::Header => "bad_header",
            Self::Frame => "bad_frame",
            Self::Name => "bad_name",
            Self::Duplicate => "duplicate_name",
            Self::TooMany => "too_many_files",
        }
    }
}

/// A parsed bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    /// The crash folder's name.
    pub crash_id: String,
    /// The game version played.
    pub game_version: Option<String>,
    /// The launcher's version.
    pub launcher_version: Option<String>,
    /// The game's exit code.
    pub exit_code: Option<String>,
    /// The files, in the bundle's order.
    pub files: Vec<(String, Vec<u8>)>,
}

fn plain(text: &str) -> bool {
    !text.is_empty()
        && !text.starts_with('.')
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Whether a bundle may carry `name`: a plain name of at most [`MAX_NAME`] bytes with a known extension.
pub fn is_file_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    name.len() <= MAX_NAME
        && plain(name)
        && TEXT_EXTENSIONS
            .iter()
            .chain([&DUMP_EXTENSION])
            .any(|e| lower.strip_suffix(e).is_some_and(|stem| !stem.is_empty()))
}

/// Whether `name` is shown to staff as text.
pub fn is_text_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    TEXT_EXTENSIONS.iter().any(|e| lower.ends_with(e))
}

fn header_value(map: &Map<String, Value>, key: &str, required: bool) -> Result<Option<String>, BadBundle> {
    match map.get(key) {
        None | Some(Value::Null) if !required => Ok(None),
        Some(Value::String(text)) if text.len() <= MAX_VALUE && plain(&text.replace('+', "-")) => {
            Ok(Some(text.clone()))
        }
        _ => Err(BadBundle::Header),
    }
}

/// Unpacks and reads an upload. Nothing past [`MAX_UNPACKED`] is unpacked.
pub fn parse(upload: &[u8]) -> Result<Bundle, BadBundle> {
    let mut frame = Vec::new();
    GzDecoder::new(upload)
        .take(MAX_UNPACKED as u64 + 1)
        .read_to_end(&mut frame)
        .map_err(|_| BadBundle::Gzip)?;
    if frame.len() > MAX_UNPACKED {
        return Err(BadBundle::TooLarge);
    }
    parse_frame(&frame)
}

/// Reads an unpacked bundle.
pub fn parse_frame(frame: &[u8]) -> Result<Bundle, BadBundle> {
    let rest = frame.strip_prefix(MAGIC).ok_or(BadBundle::Magic)?;
    let line_end = rest
        .iter()
        .take(MAX_HEADER + 1)
        .position(|b| *b == b'\n')
        .ok_or(BadBundle::Header)?;
    let header: Map<String, Value> = serde_json::from_slice(&rest[..line_end]).map_err(|_| BadBundle::Header)?;
    if header.keys().any(|k| !HEADER_KEYS.contains(&k.as_str())) {
        return Err(BadBundle::Header);
    }
    let crash_id = header_value(&header, "crashId", true)?.ok_or(BadBundle::Header)?;
    let game_version = header_value(&header, "gameVersion", false)?;
    let launcher_version = header_value(&header, "launcherVersion", false)?;
    let exit_code = header_value(&header, "exitCode", false)?;

    let mut rest = &rest[line_end + 1..];
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    while !rest.is_empty() {
        if files.len() == MAX_FILES {
            return Err(BadBundle::TooMany);
        }
        let (len, after) = rest.split_first_chunk::<2>().ok_or(BadBundle::Frame)?;
        let (name, after) = after
            .split_at_checked(usize::from(u16::from_be_bytes(*len)))
            .ok_or(BadBundle::Frame)?;
        let name = std::str::from_utf8(name).map_err(|_| BadBundle::Name)?;
        if !is_file_name(name) {
            return Err(BadBundle::Name);
        }
        if files.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)) {
            return Err(BadBundle::Duplicate);
        }
        let (len, after) = after.split_first_chunk::<4>().ok_or(BadBundle::Frame)?;
        let len = usize::try_from(u32::from_be_bytes(*len)).map_err(|_| BadBundle::Frame)?;
        let (data, after) = after.split_at_checked(len).ok_or(BadBundle::Frame)?;
        files.push((name.to_owned(), data.to_vec()));
        rest = after;
    }
    Ok(Bundle {
        crash_id,
        game_version,
        launcher_version,
        exit_code,
        files,
    })
}

/// Why an upload was refused before its body was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limited {
    /// The account used its [`PER_ACCOUNT`].
    Account,
    /// Everyone together used [`TOTAL`].
    Total,
}

// Upload times per account, and in all
type Windows = (HashMap<String, VecDeque<i64>>, VecDeque<i64>);

/// Uploads per account and in all within a sliding [`WINDOW`].
pub struct RateLimit {
    clock: Clock,
    state: Mutex<Windows>,
}

impl RateLimit {
    /// An empty limit read against `clock`.
    pub fn new(clock: Clock) -> Self {
        Self {
            clock,
            state: Mutex::new((HashMap::new(), VecDeque::new())),
        }
    }

    /// Counts one upload by `account`, or says why it is refused (and counts nothing).
    pub fn take(&self, account: &str) -> Result<(), Limited> {
        let now = (self.clock)();
        let since = now - i64::try_from(WINDOW.as_millis()).unwrap_or(i64::MAX);
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let (accounts, all) = &mut *state;
        while all.front().is_some_and(|t| *t <= since) {
            all.pop_front();
        }
        accounts.retain(|_, times| {
            while times.front().is_some_and(|t| *t <= since) {
                times.pop_front();
            }
            !times.is_empty()
        });
        if accounts.get(account).is_some_and(|t| t.len() >= PER_ACCOUNT) {
            return Err(Limited::Account);
        }
        if all.len() >= TOTAL {
            return Err(Limited::Total);
        }
        accounts.entry(account.to_owned()).or_default().push_back(now);
        all.push_back(now);
        Ok(())
    }
}

/// Who sent a report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sender {
    /// Account id.
    pub id: String,
    /// Display name.
    pub name: String,
}

/// A report as kept: its id, folder and the dashboard's copy.
#[derive(Debug, Clone)]
pub struct Saved {
    /// `<UTC time>-<8 hex>`, which sorts by time.
    pub id: String,
    /// The folder holding it.
    pub folder: PathBuf,
    /// The body for the dashboard's `POST /api/ingest/crash-reports`.
    pub dashboard: Value,
}

/// Writes `bundle` under `dir` as `<id>/meta.json` and its files, then drops the oldest reports past
/// [`KEEP_REPORTS`] or [`KEEP_BYTES`].
pub fn save(dir: &Path, bundle: &Bundle, sender: &Sender, now_ms: i64, upload: &[u8]) -> std::io::Result<Saved> {
    let time = chrono::DateTime::from_timestamp_millis(now_ms).unwrap_or_default();
    let digest = sha256_hex(&[upload, sender.id.as_bytes(), &now_ms.to_be_bytes()].concat());
    let id = format!("{}-{}", time.format("%Y%m%dT%H%M%S%3fZ"), &digest[..8]);
    let folder = dir.join(&id);
    std::fs::create_dir_all(&folder)?;
    let mut files = Vec::new();
    for (name, data) in &bundle.files {
        std::fs::write(folder.join(name), data)?;
        let text = is_text_name(name).then(|| {
            let start = data.len().saturating_sub(DASHBOARD_TEXT);
            String::from_utf8_lossy(&data[start..]).into_owned()
        });
        files.push(json!({
            "name": name,
            "size": data.len(),
            "sha256": sha256_hex(data),
            "text": text,
        }));
    }
    // The dashboard's id is 32 hex; the folder name stays readable
    let dashboard = json!({
        "id": sha256_hex(id.as_bytes())[..32],
        "receivedAt": now_ms,
        "account": {"userId": sender.id, "name": sender.name},
        "crashId": bundle.crash_id,
        "gameVersion": bundle.game_version,
        "launcherVersion": bundle.launcher_version,
        "exitCode": bundle.exit_code,
        "serverPath": folder.display().to_string(),
        "files": files,
    });
    let mut meta = dashboard.clone();
    if let Some(files) = meta.get_mut("files").and_then(Value::as_array_mut) {
        for file in files {
            if let Some(file) = file.as_object_mut() {
                file.remove("text");
            }
        }
    }
    std::fs::write(folder.join(META_FILE), serde_json::to_vec_pretty(&meta)?)?;
    prune(dir, KEEP_REPORTS, KEEP_BYTES)?;
    Ok(Saved { id, folder, dashboard })
}

/// Drops the oldest report folders (by name) until at most `keep` remain within `max_bytes`.
pub fn prune(dir: &Path, keep: usize, max_bytes: u64) -> std::io::Result<()> {
    let mut reports: Vec<(String, PathBuf, u64)> = Vec::new();
    for entry in std::fs::read_dir(dir)?.filter_map(Result::ok) {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if !entry.path().join(META_FILE).is_file() {
            continue;
        }
        let size = std::fs::read_dir(entry.path())?
            .filter_map(Result::ok)
            .filter_map(|f| f.metadata().ok())
            .map(|m| m.len())
            .sum();
        reports.push((name, entry.path(), size));
    }
    reports.sort();
    let mut total: u64 = reports.iter().map(|r| r.2).sum();
    let mut count = reports.len();
    for (_, path, size) in reports {
        if count <= keep && total <= max_bytes {
            break;
        }
        std::fs::remove_dir_all(&path)?;
        count -= 1;
        total -= size;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

    use std::io::Write as _;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering};

    use flate2::Compression;
    use flate2::write::GzEncoder;

    use super::*;

    pub(crate) fn frame(header: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend(header.as_bytes());
        out.push(b'\n');
        for (name, data) in files {
            out.extend(u16::try_from(name.len()).unwrap().to_be_bytes());
            out.extend(name.as_bytes());
            out.extend(u32::try_from(data.len()).unwrap().to_be_bytes());
            out.extend(*data);
        }
        out
    }

    pub(crate) fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut e = GzEncoder::new(Vec::new(), Compression::default());
        e.write_all(bytes).unwrap();
        e.finish().unwrap()
    }

    const HEADER: &str = r#"{"crashId":"UECC-Windows-AB_0000","gameVersion":"1.14.7","exitCode":"0xC0000005"}"#;

    #[test]
    fn a_good_bundle_parses() {
        let up = gzip(&frame(HEADER, &[("Archon.log", b"log"), ("UEMinidump.dmp", b"MDMP")]));
        let bundle = parse(&up).unwrap();
        assert_eq!(bundle.crash_id, "UECC-Windows-AB_0000");
        assert_eq!(bundle.game_version.as_deref(), Some("1.14.7"));
        assert_eq!(bundle.launcher_version, None);
        assert_eq!(bundle.files.len(), 2);
        assert_eq!(bundle.files[1], ("UEMinidump.dmp".to_owned(), b"MDMP".to_vec()));
    }

    #[test]
    fn bad_bundles_are_refused() {
        let bad = |frame: Vec<u8>| parse(&gzip(&frame)).unwrap_err();
        assert_eq!(parse(b"not gzip").unwrap_err(), BadBundle::Gzip);
        assert_eq!(bad(b"DLCR2\n{}\n".to_vec()), BadBundle::Magic);
        assert_eq!(bad(frame(r#"{"crashId":"a","x":1}"#, &[])), BadBundle::Header);
        assert_eq!(bad(frame(r#"{"crashId":"../a"}"#, &[])), BadBundle::Header);
        assert_eq!(bad(frame(r#"{"gameVersion":"1"}"#, &[])), BadBundle::Header);
        assert_eq!(bad(frame(r#"[1]"#, &[])), BadBundle::Header);
        assert_eq!(bad(frame(r#"{"crashId":"a"}"#, &[("../x.log", b"")])), BadBundle::Name);
        assert_eq!(bad(frame(r#"{"crashId":"a"}"#, &[("x.exe", b"")])), BadBundle::Name);
        assert_eq!(
            bad(frame(r#"{"crashId":"a"}"#, &[("a.log", b""), ("A.LOG", b"")])),
            BadBundle::Duplicate
        );
        let many: Vec<(String, Vec<u8>)> = (0..=MAX_FILES).map(|n| (format!("f{n}.txt"), Vec::new())).collect();
        let many: Vec<(&str, &[u8])> = many.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
        assert_eq!(bad(frame(r#"{"crashId":"a"}"#, &many)), BadBundle::TooMany);
        let mut cut = frame(r#"{"crashId":"a"}"#, &[("a.log", b"abcdef")]);
        cut.truncate(cut.len() - 1);
        assert_eq!(bad(cut), BadBundle::Frame);
        let long = format!(r#"{{"crashId":"{}"}}"#, "a".repeat(MAX_HEADER));
        assert_eq!(bad(frame(&long, &[])), BadBundle::Header);
    }

    #[test]
    fn a_small_upload_cannot_unpack_past_the_cap() {
        let zeros = vec![0u8; MAX_UNPACKED + 1];
        let up = gzip(&zeros);
        assert!(up.len() < MAX_UPLOAD);
        assert_eq!(parse(&up).unwrap_err(), BadBundle::TooLarge);
    }

    #[test]
    fn the_rate_limit_slides() {
        let now = Arc::new(AtomicI64::new(1_000_000));
        let clock = now.clone();
        let limit = RateLimit::new(Arc::new(move || clock.load(Ordering::SeqCst)));
        for _ in 0..PER_ACCOUNT {
            limit.take("a").unwrap();
        }
        assert_eq!(limit.take("a"), Err(Limited::Account));
        limit.take("b").unwrap();
        now.fetch_add(i64::try_from(WINDOW.as_millis()).unwrap(), Ordering::SeqCst);
        limit.take("a").unwrap();
        for n in 0..TOTAL - 1 {
            let _ = limit.take(&format!("u{n}"));
        }
        assert_eq!(limit.take("fresh"), Err(Limited::Total));
    }

    #[test]
    fn saved_reports_are_pruned_oldest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let up = gzip(&frame(
            HEADER,
            &[("Archon.log", b"log line"), ("UEMinidump.dmp", b"MDMP")],
        ));
        let bundle = parse(&up).unwrap();
        let sender = Sender {
            id: "u1".into(),
            name: "Slayer".into(),
        };
        let first = save(tmp.path(), &bundle, &sender, 1_000, &up).unwrap();
        assert_eq!(std::fs::read(first.folder.join("UEMinidump.dmp")).unwrap(), b"MDMP");
        let d = &first.dashboard;
        assert_eq!(d["id"].as_str().unwrap().len(), 32);
        assert_eq!(d["files"][0]["text"], "log line");
        assert_eq!(d["files"][1]["text"], Value::Null);
        let meta: Value = serde_json::from_slice(&std::fs::read(first.folder.join(META_FILE)).unwrap()).unwrap();
        assert!(meta["files"][0].get("text").is_none());
        let second = save(tmp.path(), &bundle, &sender, 2_000, &up).unwrap();
        prune(tmp.path(), 1, u64::MAX).unwrap();
        assert!(!first.folder.exists() && second.folder.exists());
        prune(tmp.path(), 10, 1).unwrap();
        assert!(!second.folder.exists());
    }
}
