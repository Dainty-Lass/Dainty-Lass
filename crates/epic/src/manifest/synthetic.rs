//! Synthetic manifests, written the way Epic's writer lays them out. The archived ones are Epic's files and stay out
//! of the repository; `tests/archive.rs` fetches them by pinned URL and SHA-1 when asked to. Shared with the host
//! kit's tests through the `test-support` feature (DE6).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test fixtures, built only for tests and test-support; a panic is how a test fails"
)]

use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;

use super::*;

/// A chunk of a synthetic build.
pub struct TestChunk {
    /// Its GUID.
    pub guid: Guid,
    /// Its unpacked data.
    pub data: Vec<u8>,
    /// Its CDN group.
    pub group: u8,
    /// Its rolling hash.
    pub rolling_hash: u64,
}

/// A file of a synthetic build.
pub struct TestFile {
    /// Name in the manifest.
    pub name: String,
    /// Symlink target; empty for a regular file.
    pub symlink: String,
    /// (chunk index, offset, size)
    pub parts: Vec<(usize, u32, u32)>,
}

/// A synthetic build, written as Epic's writer lays a manifest out.
pub struct Build {
    /// Its chunks.
    pub chunks: Vec<TestChunk>,
    /// Its files.
    pub files: Vec<TestFile>,
    /// Custom fields, written after the files.
    pub custom: Vec<(String, String)>,
    /// Writes names as UTF-16 instead of ASCII.
    pub utf16_names: bool,
    /// Bytes a newer writer might add at the end of each section
    pub section_padding: usize,
    /// Chunk count written in the chunk list; the real count when `None`.
    pub chunk_count: Option<u32>,
    /// Whether the manifest data is zlib-packed.
    pub compress: bool,
    /// This file's listed SHA-1 matches no data (the installer's second-failure case)
    pub wrong_sha1: Option<usize>,
}

/// Chunk `n` with `len` bytes of data derived from `n`.
pub fn chunk(n: u8, len: usize) -> TestChunk {
    TestChunk {
        guid: Guid([
            u32::from(n),
            0x1000 + u32::from(n),
            0xABCD_0000 | u32::from(n),
            0xFFFF_FFF0 | u32::from(n),
        ]),
        data: (0..len).map(|i| (i as u8).wrapping_mul(n).wrapping_add(n)).collect(),
        group: n % 100,
        rolling_hash: 0x0123_4567_89AB_CDEF ^ u64::from(n),
    }
}

/// A regular file built from `parts`.
pub fn file(name: &str, parts: &[(usize, u32, u32)]) -> TestFile {
    TestFile {
        name: name.to_owned(),
        symlink: String::new(),
        parts: parts.to_vec(),
    }
}

impl Default for Build {
    fn default() -> Self {
        Self {
            chunks: vec![chunk(1, 64), chunk(2, 48)],
            files: vec![
                file("Version.txt", &[(0, 0, 16)]),
                file("Archon/Binaries/Win64/Game.exe", &[(0, 16, 48), (1, 0, 48)]),
            ],
            custom: vec![("BuildLabel".to_owned(), "Live".to_owned())],
            utf16_names: false,
            section_padding: 0,
            chunk_count: None,
            compress: true,
            wrong_sha1: None,
        }
    }
}

/// Writes `s` as Epic's `FString`: a signed length with the terminator, negative for UTF-16.
pub fn fstring(out: &mut Vec<u8>, s: &str, utf16: bool) {
    if s.is_empty() {
        out.extend(0i32.to_le_bytes());
    } else if utf16 {
        let units: Vec<u16> = s.encode_utf16().chain([0]).collect();
        out.extend((-(units.len() as i32)).to_le_bytes());
        units.iter().for_each(|u| out.extend(u.to_le_bytes()));
    } else {
        out.extend((s.len() as i32 + 1).to_le_bytes());
        out.extend(s.as_bytes());
        out.push(0);
    }
}

/// Writes `g` as four little-endian `u32`s.
pub fn guid(out: &mut Vec<u8>, g: Guid) {
    g.0.iter().for_each(|v| out.extend(v.to_le_bytes()));
}

/// Writes a section: its size, `version`, `body`, then `padding` filler bytes.
pub fn section(out: &mut Vec<u8>, version: u8, body: &[u8], padding: usize) {
    out.extend((4 + 1 + body.len() as u32 + padding as u32).to_le_bytes());
    out.push(version);
    out.extend(body);
    out.extend(std::iter::repeat_n(0xEE, padding));
}

impl Build {
    /// The bytes `file` holds, its parts' slices of the chunks.
    pub fn file_data(&self, file: &TestFile) -> Vec<u8> {
        file.parts
            .iter()
            // Out of range on purpose in some tests; any hash does for those
            .flat_map(|&(c, offset, size)| {
                let (start, end) = (offset as usize, offset as usize + size as usize);
                self.chunks[c].data.get(start..end).unwrap_or_default().to_vec()
            })
            .collect()
    }

    /// The unpacked data: meta, chunks, files, custom fields.
    pub fn data(&self) -> Vec<u8> {
        let pad = self.section_padding;
        let mut out = Vec::new();

        let mut meta = Vec::new();
        meta.extend(18u32.to_le_bytes());
        meta.push(0);
        meta.extend(7u32.to_le_bytes());
        for s in ["Jackal", "1.14.7.647472", "Dauntless.exe", "-epicportal"] {
            fstring(&mut meta, s, false);
        }
        meta.extend(1u32.to_le_bytes());
        fstring(&mut meta, "prereq-id", false);
        for s in ["Prereqs", "Engine/Extras/Redist/en-us/UE4PrereqSetup_x64.exe", "/quiet"] {
            fstring(&mut meta, s, false);
        }
        fstring(&mut meta, "build-id", false);
        section(&mut out, 1, &meta, pad);

        let mut cdl = Vec::new();
        cdl.extend(self.chunk_count.unwrap_or(self.chunks.len() as u32).to_le_bytes());
        self.chunks.iter().for_each(|c| guid(&mut cdl, c.guid));
        self.chunks
            .iter()
            .for_each(|c| cdl.extend(c.rolling_hash.to_le_bytes()));
        self.chunks.iter().for_each(|c| cdl.extend(Sha1Digest::of(&c.data).0));
        self.chunks.iter().for_each(|c| cdl.push(c.group));
        self.chunks
            .iter()
            .for_each(|c| cdl.extend((c.data.len() as u32).to_le_bytes()));
        self.chunks
            .iter()
            .for_each(|c| cdl.extend((c.data.len() as i64 / 2 + 41).to_le_bytes()));
        section(&mut out, 0, &cdl, pad);

        let mut fml = Vec::new();
        fml.extend((self.files.len() as u32).to_le_bytes());
        self.files
            .iter()
            .for_each(|f| fstring(&mut fml, &f.name, self.utf16_names));
        self.files.iter().for_each(|f| fstring(&mut fml, &f.symlink, false));
        for (i, f) in self.files.iter().enumerate() {
            let mut sha1 = Sha1Digest::of(&self.file_data(f)).0;
            if self.wrong_sha1 == Some(i) {
                sha1[0] ^= 1;
            }
            fml.extend(sha1);
        }
        self.files.iter().for_each(|_| fml.push(FileFlags::READ_ONLY));
        for _ in &self.files {
            fml.extend(1u32.to_le_bytes());
            fstring(&mut fml, "", false);
        }
        for f in &self.files {
            fml.extend((f.parts.len() as u32).to_le_bytes());
            for &(c, offset, size) in &f.parts {
                fml.extend(28u32.to_le_bytes());
                guid(&mut fml, self.chunks[c].guid);
                fml.extend(offset.to_le_bytes());
                fml.extend(size.to_le_bytes());
            }
        }
        for f in &self.files {
            fml.extend(1u32.to_le_bytes());
            fml.extend(md5_of(&self.file_data(f)));
        }
        self.files
            .iter()
            .for_each(|_| fstring(&mut fml, "application/octet-stream", false));
        self.files
            .iter()
            .for_each(|f| fml.extend(<[u8; 32]>::from(sha2::Sha256::digest(self.file_data(f)))));
        section(&mut out, 2, &fml, pad);

        let mut custom = Vec::new();
        custom.extend((self.custom.len() as u32).to_le_bytes());
        self.custom.iter().for_each(|(k, _)| fstring(&mut custom, k, false));
        self.custom.iter().for_each(|(_, v)| fstring(&mut custom, v, false));
        section(&mut out, 0, &custom, pad);
        out
    }

    /// The whole manifest file: header and data.
    pub fn bytes(&self) -> Vec<u8> {
        pack(&self.data(), self.compress)
    }
}

/// The installer checks a listed MD5 (2.1.1's manifests carry them).
pub fn md5_of(data: &[u8]) -> [u8; 16] {
    let mut md5 = crate::download::md5::Md5::new();
    md5.update(data);
    md5.finish()
}

/// The header around `data`.
pub fn pack(data: &[u8], compress: bool) -> Vec<u8> {
    let body = if compress {
        let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
        z.write_all(data).unwrap();
        z.finish().unwrap()
    } else {
        data.to_vec()
    };
    let mut out = Vec::new();
    out.extend(MAGIC.to_le_bytes());
    out.extend(41u32.to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(Sha1Digest::of(data).0);
    out.push(u8::from(compress));
    out.extend(18u32.to_le_bytes());
    out.extend(body);
    out
}

/// Parses `bytes` pinned to their own SHA-1.
pub fn parse(bytes: &[u8]) -> Result<Manifest, ManifestError> {
    Manifest::parse(bytes, &Sha1Digest::of(bytes))
}
