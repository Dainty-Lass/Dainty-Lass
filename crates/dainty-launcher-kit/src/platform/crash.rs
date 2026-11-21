// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Game crash reports (`docs/security/crash-reports.md`): the game's crash folders, which of them were already
//! offered, and the bundle sent to the joined server's content service.
//!
//! A bundle is gzip over `DLCR1\n`, one line of header JSON, then each file as a big-endian `u16` name length, the
//! name, a big-endian `u32` data length and the data. The content service's `crash` module reads it.

use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use flate2::Compression;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::bridge::types::CrashReports;
use crate::platform::log;

/// The bundle's first bytes.
pub const MAGIC: &[u8] = b"DLCR1\n";
/// The upload route on the content service.
pub const UPLOAD_PATH: &str = "/content/v1/crash-reports";
/// The upload's content type.
pub const CONTENT_TYPE: &str = "application/vnd.dainty.crash+gzip";
/// Largest bundle sent: under the gateway's 128 KiB default request body (`GATEWAY_MAX_BODY_BYTES`).
pub const MAX_BUNDLE: usize = 120 * 1024;
/// Most files in a bundle.
pub const MAX_FILES: usize = 16;
/// Longest file name in a bundle.
pub const MAX_NAME: usize = 80;
/// Longest crash folder name taken as an id.
pub const MAX_ID: usize = 128;
/// The file the offered crash ids are kept in, in `userData`.
pub const LEDGER_FILE: &str = "crash-reports.json";
/// Crash ids remembered; the oldest go first.
pub const MAX_SEEN: usize = 64;
/// The launcher's own log in a bundle.
pub const LAUNCHER_LOG_NAME: &str = "launcher.log";
// A crash folder's time can trail the start by the file system's resolution.
const START_SLACK: Duration = Duration::from_secs(5);
const DUMP_EXTENSION: &str = "dmp";
const TEXT_EXTENSIONS: [&str; 5] = ["log", "xml", "runtime-xml", "txt", "json"];
// Tried in turn until the bundle fits: (bytes kept from the end of each text file, whether the minidump goes).
const BUDGETS: [(usize, bool); 6] = [
    (256 * 1024, true),
    (64 * 1024, true),
    (16 * 1024, true),
    (64 * 1024, false),
    (16 * 1024, false),
    (4 * 1024, false),
];

/// What to do about a crash just found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashAction {
    /// Nothing: reports are off, or it was offered before.
    Ignore,
    /// Ask the player.
    Ask,
    /// Send it now.
    Send,
}

/// The action for a crash: each one is acted on once, the first time it is seen.
#[must_use]
pub const fn decide(choice: CrashReports, first_sight: bool) -> CrashAction {
    if !first_sight {
        return CrashAction::Ignore;
    }
    match choice {
        CrashReports::Never => CrashAction::Ignore,
        CrashReports::Ask => CrashAction::Ask,
        CrashReports::Always => CrashAction::Send,
    }
}

/// Whether `name` can be a crash id: UE names its folders `UECC-Windows-<hex>_0000` and the like.
#[must_use]
pub fn is_crash_id(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_ID
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Where the game may write its crash folders: beside the game, and under the `Saved` folder that holds its
/// config (`%LOCALAPPDATA%`).
#[must_use]
pub fn crash_dirs(game_root: Option<&Path>, config_dir: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(root) = game_root {
        dirs.push(root.join("Archon").join("Saved").join("Crashes"));
    }
    if let Some(saved) = config_dir
        .ancestors()
        .find(|p| p.file_name().is_some_and(|n| n == "Saved"))
    {
        dirs.push(saved.join("Crashes"));
    }
    dirs.dedup();
    dirs
}

/// One crash folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashFolder {
    /// Its name.
    pub id: String,
    /// Where it is.
    pub path: PathBuf,
}

/// The crash folders in `dirs` written since `since`, newest first.
#[must_use]
pub fn new_crashes(dirs: &[PathBuf], since: SystemTime) -> Vec<CrashFolder> {
    let since = since.checked_sub(START_SLACK).unwrap_or(since);
    let mut found: Vec<(SystemTime, CrashFolder)> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        for entry in entries.filter_map(Result::ok) {
            let Ok(meta) = entry.metadata() else { continue };
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let Ok(time) = meta.modified() else { continue };
            if meta.is_dir() && is_crash_id(&name) && time >= since {
                found.push((
                    time,
                    CrashFolder {
                        id: name,
                        path: entry.path(),
                    },
                ));
            }
        }
    }
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    found.into_iter().map(|(_, folder)| folder).collect()
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct LedgerFile {
    seen: Vec<String>,
}

/// The crash ids already acted on, kept in [`LEDGER_FILE`] so a crash is offered once, across restarts.
#[derive(Debug)]
pub struct Ledger {
    path: PathBuf,
    seen: Vec<String>,
}

impl Ledger {
    /// Reads `<user_data>/crash-reports.json`; a missing or unreadable file is empty.
    #[must_use]
    pub fn load(user_data: &Path) -> Self {
        let path = user_data.join(LEDGER_FILE);
        let seen = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<LedgerFile>(&bytes).ok())
            .map(|file| file.seen.into_iter().filter(|id| is_crash_id(id)).collect())
            .unwrap_or_default();
        Self { path, seen }
    }

    /// True the first time `id` is seen, which remembers it. A failed write still remembers it here.
    pub fn first_sight(&mut self, id: &str) -> bool {
        if self.seen.iter().any(|s| s == id) {
            return false;
        }
        self.seen.push(id.to_owned());
        let cut = self.seen.len().saturating_sub(MAX_SEEN);
        self.seen.drain(..cut);
        if let Err(failed) = self.save() {
            log::warn(format!("could not save the crash report list: {failed}"));
        }
        true
    }

    fn save(&self) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = LedgerFile {
            seen: self.seen.clone(),
        };
        std::fs::write(&self.path, serde_json::to_vec(&file).map_err(io::Error::other)?)
    }
}

/// What the bundle's header says about the crash.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Header {
    /// The crash folder's name.
    pub crash_id: String,
    /// The game version played.
    pub game_version: Option<String>,
    /// This launcher's version.
    pub launcher_version: Option<String>,
    /// The game's exit code, as `0x` hex.
    pub exit_code: Option<String>,
}

impl Header {
    fn line(&self) -> Vec<u8> {
        let mut line = json!({
            "crashId": self.crash_id,
            "gameVersion": self.game_version,
            "launcherVersion": self.launcher_version,
            "exitCode": self.exit_code,
        })
        .to_string()
        .into_bytes();
        line.push(b'\n');
        line
    }
}

/// The frame before compression.
#[must_use]
pub fn encode(header: &Header, files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.extend(header.line());
    for (name, data) in files {
        let name_len = u16::try_from(name.len()).unwrap_or(u16::MAX);
        let data_len = u32::try_from(data.len()).unwrap_or(u32::MAX);
        out.extend(name_len.to_be_bytes());
        out.extend(&name.as_bytes()[..usize::from(name_len)]);
        out.extend(data_len.to_be_bytes());
        out.extend(&data[..data_len as usize]);
    }
    out
}

fn gzip(bytes: &[u8]) -> io::Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(bytes)?;
    encoder.finish()
}

/// `text`'s last `max` bytes, from a line start.
#[must_use]
pub fn tail(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let rest = &text[start..];
    rest.find('\n').map_or("", |i| &rest[i + 1..])
}

/// `bytes` with each secret, as UTF-8 and as UTF-16 LE, overwritten by `*`s of the same length, so a minidump
/// keeps its layout.
#[must_use]
pub fn scrub(mut bytes: Vec<u8>, secrets: &[&str]) -> Vec<u8> {
    for secret in secrets.iter().filter(|s| s.len() >= 6) {
        let wide: Vec<u8> = secret.encode_utf16().flat_map(u16::to_le_bytes).collect();
        for needle in [secret.as_bytes(), wide.as_slice()] {
            let mut at = 0;
            while let Some(found) = bytes[at..].windows(needle.len()).position(|w| w == needle) {
                let start = at + found;
                bytes[start..start + needle.len()].fill(b'*');
                at = start + needle.len();
            }
        }
    }
    bytes
}

/// A text file as the bundle carries it: the launcher log's redaction and every secret removed.
#[must_use]
pub fn redact_text(text: &str, secrets: &[&str]) -> String {
    let mut out = log::redact(text);
    for secret in secrets.iter().filter(|s| s.len() >= 6) {
        out = out.replace(*secret, log::HIDDEN);
    }
    out
}

fn extension(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    // Longest first: `runtime-xml` before `xml`
    let mut known: Vec<&'static str> = TEXT_EXTENSIONS.iter().chain([&DUMP_EXTENSION]).copied().collect();
    known.sort_by_key(|e| std::cmp::Reverse(e.len()));
    known.into_iter().find(|e| {
        lower
            .strip_suffix(e)
            .is_some_and(|stem| stem.ends_with('.') && stem.len() > 1)
    })
}

/// Whether a bundle may carry a file named `name`: a plain name with a known extension.
#[must_use]
pub fn is_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && extension(name).is_some()
}

/// The gzip bundle for `folder`, with this session's launcher log, no larger than [`MAX_BUNDLE`]. Text is cut
/// from the start and the minidump left out until it fits. `secrets` (the account key) never leave in any file.
pub fn bundle(folder: &Path, header: &Header, launcher_log: Option<&str>, secrets: &[&str]) -> io::Result<Vec<u8>> {
    let mut texts: Vec<(String, String)> = Vec::new();
    let mut dumps: Vec<(String, Vec<u8>)> = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(folder)?.filter_map(Result::ok).collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if !is_file_name(&name) || name == LAUNCHER_LOG_NAME || !entry.metadata().is_ok_and(|m| m.is_file()) {
            continue;
        }
        if texts.len() + dumps.len() + 1 >= MAX_FILES {
            break;
        }
        let bytes = std::fs::read(entry.path())?;
        if extension(&name) == Some(DUMP_EXTENSION) {
            dumps.push((name, scrub(bytes, secrets)));
        } else {
            texts.push((name, redact_text(&String::from_utf8_lossy(&bytes), secrets)));
        }
    }
    if let Some(text) = launcher_log {
        texts.push((LAUNCHER_LOG_NAME.to_owned(), redact_text(text, secrets)));
    }
    for (keep, with_dumps) in BUDGETS {
        let mut files: Vec<(String, Vec<u8>)> = texts
            .iter()
            .map(|(name, text)| (name.clone(), tail(text, keep).as_bytes().to_vec()))
            .collect();
        if with_dumps {
            files.extend(dumps.iter().cloned());
        }
        let packed = gzip(&encode(header, &files))?;
        if packed.len() <= MAX_BUNDLE {
            return Ok(packed);
        }
    }
    Err(io::Error::other("the crash report does not fit"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

    use std::io::Read as _;

    use flate2::read::GzDecoder;

    use super::*;

    const KEY: &str = "DLK_abcdefghijklmnopqrstuvwxyz012345";

    fn unpack(bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        GzDecoder::new(bytes).read_to_end(&mut out).unwrap();
        out
    }

    fn files(frame: &[u8]) -> Vec<(String, Vec<u8>)> {
        let rest = frame.strip_prefix(MAGIC).unwrap();
        let line = rest.iter().position(|b| *b == b'\n').unwrap();
        let mut rest = &rest[line + 1..];
        let mut out = Vec::new();
        while !rest.is_empty() {
            let n = usize::from(u16::from_be_bytes([rest[0], rest[1]]));
            let name = String::from_utf8(rest[2..2 + n].to_vec()).unwrap();
            rest = &rest[2 + n..];
            let len = u32::from_be_bytes(rest[..4].try_into().unwrap()) as usize;
            out.push((name, rest[4..4 + len].to_vec()));
            rest = &rest[4 + len..];
        }
        out
    }

    #[test]
    fn each_crash_is_acted_on_once() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ledger = Ledger::load(tmp.path());
        assert!(ledger.first_sight("UECC-Windows-1"));
        assert!(!ledger.first_sight("UECC-Windows-1"));
        // Across a restart too.
        let mut again = Ledger::load(tmp.path());
        assert!(!again.first_sight("UECC-Windows-1"));
        assert!(again.first_sight("UECC-Windows-2"));

        assert_eq!(decide(CrashReports::Ask, true), CrashAction::Ask);
        assert_eq!(decide(CrashReports::Always, true), CrashAction::Send);
        assert_eq!(decide(CrashReports::Never, true), CrashAction::Ignore);
        assert_eq!(decide(CrashReports::Ask, false), CrashAction::Ignore);
        assert_eq!(decide(CrashReports::Always, false), CrashAction::Ignore);
    }

    #[test]
    fn the_ledger_keeps_the_newest_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ledger = Ledger::load(tmp.path());
        for n in 0..=MAX_SEEN {
            assert!(ledger.first_sight(&format!("c{n}")));
        }
        let mut again = Ledger::load(tmp.path());
        assert!(again.first_sight("c0"), "the oldest was forgotten");
        assert!(!again.first_sight(&format!("c{MAX_SEEN}")));
        std::fs::write(tmp.path().join(LEDGER_FILE), b"{\"seen\":[\"../x\",\"ok\"]}").unwrap();
        let mut odd = Ledger::load(tmp.path());
        assert!(!odd.first_sight("ok"));
        assert!(odd.first_sight("../x"));
    }

    #[test]
    fn only_new_crash_folders_count() {
        let tmp = tempfile::tempdir().unwrap();
        let crashes = tmp.path().join("Archon").join("Saved").join("Crashes");
        std::fs::create_dir_all(crashes.join("UECC-Windows-AB_0000")).unwrap();
        std::fs::create_dir_all(crashes.join("bad name")).unwrap();
        std::fs::write(crashes.join("file.txt"), "x").unwrap();
        let dirs = crash_dirs(
            Some(tmp.path()),
            Path::new("C:/L/Dauntless/Saved/Config/WindowsNoEditor"),
        );
        assert_eq!(dirs[0], crashes);
        assert_eq!(dirs[1], Path::new("C:/L/Dauntless/Saved").join("Crashes"));
        let found = new_crashes(&dirs, SystemTime::now() - Duration::from_secs(60));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "UECC-Windows-AB_0000");
        assert!(new_crashes(&dirs, SystemTime::now() + Duration::from_secs(60)).is_empty());
    }

    #[test]
    fn names_and_ids() {
        assert!(is_file_name("UEMinidump.dmp") && is_file_name("CrashContext.runtime-xml"));
        assert!(is_file_name("Archon.log") && !is_file_name(".log") && !is_file_name("x.exe"));
        assert!(!is_file_name("../a.log") && !is_file_name("a/b.log") && !is_file_name("log"));
        assert!(is_crash_id("UECC-Windows-0A1B_0000") && !is_crash_id("a b") && !is_crash_id(".."));
        assert_eq!(tail("one\ntwo\nthree\n", 8), "three\n");
        assert_eq!(tail("short", 8), "short");
        assert_eq!(tail("ééééé\nx", 4), "x");
    }

    #[test]
    fn the_key_never_leaves_in_a_bundle() {
        let tmp = tempfile::tempdir().unwrap();
        let wide: Vec<u8> = KEY.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let mut dump = b"MDMP....".to_vec();
        dump.extend(KEY.as_bytes());
        dump.extend(&wide);
        std::fs::write(tmp.path().join("UEMinidump.dmp"), &dump).unwrap();
        std::fs::write(
            tmp.path().join("Archon.log"),
            format!("LogInit: Command Line: -AUTH_PASSWORD={KEY} -USER=1\nother {KEY}\n"),
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("CrashContext.runtime-xml"),
            "<CommandLine>-AUTH_PASSWORD=zz</CommandLine>",
        )
        .unwrap();
        std::fs::write(tmp.path().join("evil.exe"), "x").unwrap();
        let header = Header {
            crash_id: "UECC-1".into(),
            exit_code: Some("0xC0000005".into()),
            ..Header::default()
        };
        let packed = bundle(
            tmp.path(),
            &header,
            Some("launcher said x-dainty-user-api-key: abcdefgh"),
            &[KEY],
        )
        .unwrap();
        let frame = unpack(&packed);
        let rest = &KEY["DLK_".len()..];
        assert!(!frame.windows(rest.len()).any(|w| w == rest.as_bytes()));
        assert!(!frame.windows(wide.len()).any(|w| w == wide.as_slice()));
        let files = files(&frame);
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            [
                "Archon.log",
                "CrashContext.runtime-xml",
                "launcher.log",
                "UEMinidump.dmp"
            ]
        );
        assert_eq!(files[3].1.len(), dump.len(), "the dump keeps its layout");
        let log = String::from_utf8(files[2].1.clone()).unwrap();
        assert!(!log.contains("abcdefgh"));
        assert!(String::from_utf8_lossy(&files[1].1).contains("-AUTH_PASSWORD=<hidden>"));
    }

    #[test]
    fn a_large_crash_is_cut_to_fit() {
        let tmp = tempfile::tempdir().unwrap();
        // Random bytes don't compress: the dump alone is over the cap.
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let noise: Vec<u8> = (0..300_000)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state.to_le_bytes()[0]
            })
            .collect();
        std::fs::write(tmp.path().join("UEMinidump.dmp"), &noise).unwrap();
        let log: String = (0..40_000).map(|n| format!("line {n} {}\n", n * 7919 % 1013)).collect();
        std::fs::write(tmp.path().join("Archon.log"), &log).unwrap();
        let packed = bundle(tmp.path(), &Header::default(), None, &[]).unwrap();
        assert!(packed.len() <= MAX_BUNDLE);
        let files = files(&unpack(&packed));
        assert_eq!(files.len(), 1, "the dump is left out");
        assert!(log.ends_with(std::str::from_utf8(&files[0].1).unwrap()));
    }
}
