//! The resume file: one per download, in the launcher's (or the host kit's) data folder, never in the game folder.
//! A header names the pinned manifest and the install folder; then one line per event, appended as it happens:
//! `c <GUID>` once a chunk's parts are written, `x <GUID>` when a file that used it failed its check and the chunk
//! is fetched again, `f <index>` once a file has passed its check and been flushed to disk. A download killed
//! halfway leaves at worst a torn last line, which is skipped. Nothing in it is trusted further than that: every
//! file is checked against the manifest before the version counts as installed, so a wrong line costs a refetch.

use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::manifest::{Guid, Sha1Digest};

const MAGIC: &str = "dainty-lass epic resume 1";
/// 1.14.7's 15,749 chunks and 611 files take about 600 KB; anything far larger is not ours
const MAX_SIZE: u64 = 16 << 20;

/// What an earlier run of the same download finished.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResumeState {
    pub chunks: HashSet<Guid>,
    pub files: HashSet<usize>,
}

fn header(manifest: &Sha1Digest, folder: &str) -> String {
    format!("{MAGIC}\nmanifest {manifest}\nfolder {folder}\n")
}

/// The folder as the header writes it: one line, whatever the path holds.
fn folder_line(folder: &Path) -> String {
    folder
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .collect()
}

fn parse_guid(text: &str) -> Option<Guid> {
    if text.len() != 32 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u32; 4];
    for (i, word) in out.iter_mut().enumerate() {
        *word = u32::from_str_radix(text.get(i * 8..i * 8 + 8)?, 16).ok()?;
    }
    Some(Guid(out))
}

/// The state a resume file records, or None when it belongs to another manifest or folder (or is not one):
/// the download then starts over.
pub fn parse(bytes: &[u8], manifest: &Sha1Digest, folder: &str) -> Option<ResumeState> {
    let text = String::from_utf8_lossy(bytes);
    let expected = header(manifest, folder);
    let body = text.strip_prefix(expected.as_str())?;
    let mut state = ResumeState::default();
    // Only whole lines: the last one may have been cut off by a kill
    let whole = body.rfind('\n').map_or("", |end| &body[..end]);
    for line in whole.split('\n') {
        match line.split_once(' ') {
            Some(("c", guid)) => {
                if let Some(guid) = parse_guid(guid) {
                    state.chunks.insert(guid);
                }
            }
            Some(("x", guid)) => {
                if let Some(guid) = parse_guid(guid) {
                    state.chunks.remove(&guid);
                }
            }
            Some(("f", index)) => {
                if let Ok(index) = index.parse::<usize>() {
                    state.files.insert(index);
                }
            }
            _ => {}
        }
    }
    Some(state)
}

/// The open resume file, appended to as the download goes.
pub(crate) struct ResumeFile {
    path: PathBuf,
    file: File,
}

impl ResumeFile {
    /// Reads what an earlier run left, or starts the file afresh when it belongs to another download.
    pub(crate) fn open(path: &Path, manifest: &Sha1Digest, folder: &Path) -> io::Result<(Self, ResumeState)> {
        let folder = folder_line(folder);
        let earlier = match File::open(path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(MAX_SIZE + 1).read_to_end(&mut bytes)?;
                (bytes.len() as u64 <= MAX_SIZE)
                    .then(|| parse(&bytes, manifest, &folder))
                    .flatten()
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let (file, state) = match earlier {
            Some(state) => (OpenOptions::new().append(true).open(path)?, state),
            None => {
                let mut file = File::create(path)?;
                file.write_all(header(manifest, &folder).as_bytes())?;
                (file, ResumeState::default())
            }
        };
        let mut resume = Self {
            path: path.to_path_buf(),
            file,
        };
        // A torn last line from a kill would swallow the next one
        resume.file.write_all(b"\n")?;
        Ok((resume, state))
    }

    fn line(&mut self, line: String) -> io::Result<()> {
        self.file.write_all(line.as_bytes())
    }

    pub(crate) fn chunk_done(&mut self, guid: Guid) -> io::Result<()> {
        self.line(format!("c {guid}\n"))
    }

    pub(crate) fn chunk_undone(&mut self, guid: Guid) -> io::Result<()> {
        self.line(format!("x {guid}\n"))
    }

    pub(crate) fn file_verified(&mut self, index: usize) -> io::Result<()> {
        self.line(format!("f {index}\n"))
    }

    /// The download finished: nothing is left to resume.
    pub(crate) fn remove(self) -> io::Result<()> {
        drop(self.file);
        std::fs::remove_file(&self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUID: Guid = Guid([1, 0xABCD_EF01, 0, u32::MAX]);

    #[test]
    fn a_guid_reads_back_as_it_is_written() {
        assert_eq!(parse_guid(&GUID.to_string()), Some(GUID));
        assert_eq!(parse_guid(&GUID.to_string().to_lowercase()), Some(GUID));
        assert_eq!(parse_guid("+0000001ABCDEF010000000000000000"), None);
        assert_eq!(parse_guid("00000001"), None);
    }

    #[test]
    fn only_this_downloads_file_and_whole_lines_count() {
        let pin = Sha1Digest([7; 20]);
        let other = Sha1Digest([8; 20]);
        let text = format!(
            "{}c {GUID}\nf 3\nf x\nq 1\nc {GUID}\nx {GUID}\nc {GUID}\nf 4",
            header(&pin, "D:/G")
        );
        let state = parse(text.as_bytes(), &pin, "D:/G").unwrap();
        assert!(state.chunks.contains(&GUID));
        // `f 4` has no newline yet: a kill may have cut it
        assert_eq!(state.files, HashSet::from([3]));
        assert_eq!(parse(text.as_bytes(), &other, "D:/G"), None);
        assert_eq!(parse(text.as_bytes(), &pin, "E:/G"), None);
        assert_eq!(parse(b"", &pin, "D:/G"), None);
    }

    #[test]
    fn the_file_resumes_appends_and_starts_over_for_another_download() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data").join("1.14.7.resume");
        let pin = Sha1Digest([7; 20]);
        let folder = dir.path().join("game");
        let (mut file, state) = ResumeFile::open(&path, &pin, &folder).unwrap();
        assert_eq!(state, ResumeState::default());
        file.chunk_done(GUID).unwrap();
        file.file_verified(0).unwrap();
        drop(file);
        // A kill mid-line
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"c 0000")
            .unwrap();
        let (mut file, state) = ResumeFile::open(&path, &pin, &folder).unwrap();
        assert!(state.chunks.contains(&GUID));
        assert_eq!(state.files, HashSet::from([0]));
        file.chunk_undone(GUID).unwrap();
        drop(file);
        let (_, state) = ResumeFile::open(&path, &pin, &folder).unwrap();
        assert!(state.chunks.is_empty());

        let (file, state) = ResumeFile::open(&path, &Sha1Digest([9; 20]), &folder).unwrap();
        assert_eq!(state, ResumeState::default());
        file.remove().unwrap();
        assert!(!path.exists());
    }
}
