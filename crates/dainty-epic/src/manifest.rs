//! Epic's binary download manifest (Unreal's BuildPatchServices format, as Legendary documents it): a header, then
//! zlib-packed sections for the build's metadata, its chunks, its files and custom fields. Written here rather than
//! taken from `egs-api` (DE1's kill criterion, `docs/port/spike-de1-epic.md`).
//!
//! [`Manifest::parse`] checks the whole file's SHA-1 against the pin before it reads a byte of it (DD22), and
//! refuses a file name that would leave the install folder ([`InstallPath`]), a symlink, a name given twice and a
//! file part that names a chunk the manifest lacks or reaches past one's end.

mod path;
mod reader;

pub use path::{InstallPath, PathProblem};

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::Read;
use std::str::FromStr;

use flate2::read::ZlibDecoder;
use sha1::{Digest, Sha1};

use reader::Reader;

const MAGIC: u32 = 0x44BE_C00C;
const HEADER_SIZE: usize = 41;
/// 1.14.7's unpacks to 1.4 MB; a game manifest is nowhere near this
pub(crate) const MAX_DATA: usize = 64 << 20;
const STORED_COMPRESSED: u8 = 0x01;
const STORED_ENCRYPTED: u8 = 0x02;

/// A SHA-1 digest: a manifest's pin, a chunk's or a file's hash.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sha1Digest(pub [u8; 20]);

impl Sha1Digest {
    /// The SHA-1 of `data`.
    pub fn of(data: &[u8]) -> Self {
        Self(Sha1::digest(data).into())
    }
}

impl fmt::Display for Sha1Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl fmt::Debug for Sha1Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sha1Digest({self})")
    }
}

/// Not 40 hex digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a SHA-1 is 40 hex digits")]
pub struct BadSha1;

impl FromStr for Sha1Digest {
    type Err = BadSha1;

    fn from_str(s: &str) -> Result<Self, BadSha1> {
        // from_str_radix alone would take a sign
        if s.len() != 40 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(BadSha1);
        }
        let mut out = [0u8; 20];
        for (byte, pair) in out.iter_mut().zip(s.as_bytes().as_chunks::<2>().0) {
            let pair = std::str::from_utf8(pair).map_err(|_| BadSha1)?;
            *byte = u8::from_str_radix(pair, 16).map_err(|_| BadSha1)?;
        }
        Ok(Self(out))
    }
}

/// A chunk's GUID, four little-endian `u32`s; shown as the CDN writes it, 32 upper-case hex digits.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Guid(pub [u32; 4]);

impl fmt::Display for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [a, b, c, d] = self.0;
        write!(f, "{a:08X}{b:08X}{c:08X}{d:08X}")
    }
}

impl fmt::Debug for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Guid({self})")
    }
}

/// Why a manifest was refused. File names are shown escaped (`{:?}`), never raw.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ManifestError {
    /// The file's SHA-1 is not the pin.
    #[error("the manifest's SHA-1 is {actual}, not the pinned {expected}")]
    PinMismatch {
        /// The pinned SHA-1.
        expected: Sha1Digest,
        /// The file's SHA-1.
        actual: Sha1Digest,
    },
    /// The manifest is JSON, not binary.
    #[error("JSON manifests are not supported, only binary ones")]
    Json,
    /// The file does not start with the manifest magic.
    #[error("not an Epic manifest")]
    NotAManifest,
    /// The manifest is stored encrypted.
    #[error("the manifest is encrypted")]
    Encrypted,
    /// The header's sizes or fields disagree.
    #[error("the manifest's header is inconsistent")]
    BadHeader,
    /// The data is over the largest size accepted.
    #[error("the manifest is larger than any game's")]
    TooLarge,
    /// The data does not inflate.
    #[error("the manifest's data does not unpack")]
    Inflate,
    /// The unpacked data does not match the SHA-1 in the header.
    #[error("the manifest's data does not match the SHA-1 in its header")]
    DataHash,
    /// The data ends before a field.
    #[error("the manifest ends early at byte {offset}")]
    Truncated {
        /// Byte offset of the field.
        offset: usize,
    },
    /// A string's length or text is invalid.
    #[error("bad text at byte {offset}")]
    BadString {
        /// Byte offset of the string.
        offset: usize,
    },
    /// A section's size disagrees with what it holds.
    #[error("the {section} section's size is wrong")]
    SectionSize {
        /// The section's name.
        section: &'static str,
    },
    /// A count is more than the remaining bytes could hold.
    #[error("more {what} than the manifest has room for")]
    CountTooLarge {
        /// What was counted.
        what: &'static str,
    },
    /// Bytes left after the last section.
    #[error("bytes left after the last section")]
    TrailingData,
    /// A chunk GUID is listed twice.
    #[error("chunk {0} is listed twice")]
    DuplicateChunk(Guid),
    /// A chunk's download size is negative.
    #[error("a chunk's download size is negative")]
    NegativeSize,
    /// A file part's header is shorter than its fields.
    #[error("a file part's header is shorter than its fields")]
    BadPart,
    /// A file name failed [`InstallPath::new`].
    #[error("file {name:?}: {problem}")]
    UnsafePath {
        /// The file name.
        name: String,
        /// What is wrong with it.
        problem: PathProblem,
    },
    /// A file is a symlink.
    #[error("file {name:?} is a symlink")]
    Symlink {
        /// The file's name in the manifest.
        name: String,
    },
    /// A file name is listed twice.
    #[error("file {name:?} is listed twice")]
    DuplicatePath {
        /// The file's name in the manifest.
        name: String,
    },
    /// A file uses a chunk the manifest does not list.
    #[error("file {name:?} uses a chunk the manifest does not list")]
    UnknownChunk {
        /// The file's name in the manifest.
        name: String,
    },
    /// A file part reads past the end of its chunk.
    #[error("file {name:?} reads past the end of a chunk")]
    PartOutOfRange {
        /// The file's name in the manifest.
        name: String,
    },
}

/// A file's flags, as Epic sets them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FileFlags(pub u8);

impl FileFlags {
    /// The file is installed read-only.
    pub const READ_ONLY: u8 = 0x01;
    /// The file is stored compressed.
    pub const COMPRESSED: u8 = 0x02;
    /// The file is executable on Unix.
    pub const UNIX_EXECUTABLE: u8 = 0x04;

    /// Whether [`FileFlags::READ_ONLY`] is set.
    pub fn read_only(self) -> bool {
        self.0 & Self::READ_ONLY != 0
    }

    /// Whether [`FileFlags::UNIX_EXECUTABLE`] is set.
    pub fn unix_executable(self) -> bool {
        self.0 & Self::UNIX_EXECUTABLE != 0
    }
}

/// One chunk the CDN serves, as the manifest describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkInfo {
    /// The chunk's GUID; part of its CDN name.
    pub guid: Guid,
    /// Epic's rolling hash; part of the chunk's CDN name
    pub rolling_hash: u64,
    /// Of the chunk's data once unpacked: what [`crate::chunk::decode`] checks
    pub sha1: Sha1Digest,
    /// CDN group folder, 0 to 99.
    pub group: u8,
    /// The unpacked size
    pub window_size: u32,
    /// The size of the `.chunk` file on the CDN
    pub download_size: u64,
}

impl ChunkInfo {
    /// The chunk's path under a CDN base, e.g. `ChunksV4/75/9B3E…_ACCF….chunk`.
    pub fn cdn_path(&self, feature_level: u32) -> String {
        format!(
            "{}/{:02}/{:016X}_{}.chunk",
            chunk_dir(feature_level),
            self.group,
            self.rolling_hash,
            self.guid
        )
    }
}

/// The CDN folder for a manifest's feature level.
pub fn chunk_dir(feature_level: u32) -> &'static str {
    match feature_level {
        15.. => "ChunksV4",
        6.. => "ChunksV3",
        3.. => "ChunksV2",
        _ => "Chunks",
    }
}

/// Bytes `offset..offset + size` of chunk `guid`, the next piece of a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkPart {
    /// The chunk's GUID.
    pub guid: Guid,
    /// Where the piece starts in the chunk's unpacked data.
    pub offset: u32,
    /// Its length in bytes.
    pub size: u32,
}

/// One file of the build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Where the file goes in the install folder.
    pub path: InstallPath,
    /// SHA-1 of the whole file.
    pub sha1: Sha1Digest,
    /// Epic's flags.
    pub flags: FileFlags,
    /// Epic's install tags.
    pub install_tags: Vec<String>,
    /// In order; the file is their concatenation
    pub parts: Vec<ChunkPart>,
    /// Size in bytes: the parts' sizes added up.
    pub size: u64,
    /// MD5 of the file, when the manifest lists one.
    pub md5: Option<[u8; 16]>,
    /// MIME type, often empty.
    pub mime_type: String,
    /// SHA-256 of the file, when the manifest lists one.
    pub sha256: Option<[u8; 32]>,
}

/// A parsed, pinned manifest.
#[derive(Debug, Clone)]
pub struct Manifest {
    /// Epic's manifest feature level; it picks the CDN's chunk folder.
    pub feature_level: u32,
    /// Whether the manifest describes file data rather than chunks.
    pub is_file_data: bool,
    /// Epic's app id.
    pub app_id: u32,
    /// `Jackal` for Dauntless
    pub app_name: String,
    /// Epic's build version.
    pub build_version: String,
    /// Epic's build id, when the meta section is version 1 or later.
    pub build_id: Option<String>,
    /// Executable the launcher would start, relative to the install folder.
    pub launch_exe: String,
    /// Command line for it.
    pub launch_command: String,
    /// Prerequisite ids.
    pub prereq_ids: Vec<String>,
    /// Prerequisite name.
    pub prereq_name: String,
    /// Prerequisite installer path.
    pub prereq_path: String,
    /// Prerequisite installer arguments.
    pub prereq_args: String,
    /// Custom fields, in file order.
    pub custom_fields: Vec<(String, String)>,
    chunks: Vec<ChunkInfo>,
    files: Vec<FileEntry>,
    chunk_index: HashMap<Guid, usize>,
}

struct Meta {
    feature_level: u32,
    is_file_data: bool,
    app_id: u32,
    app_name: String,
    build_version: String,
    build_id: Option<String>,
    launch_exe: String,
    launch_command: String,
    prereq_ids: Vec<String>,
    prereq_name: String,
    prereq_path: String,
    prereq_args: String,
}

impl Manifest {
    /// Parses `bytes` once their SHA-1 is `pin`; nothing is read from a file that doesn't match.
    pub fn parse(bytes: &[u8], pin: &Sha1Digest) -> Result<Self, ManifestError> {
        let actual = Sha1Digest::of(bytes);
        if actual != *pin {
            return Err(ManifestError::PinMismatch { expected: *pin, actual });
        }
        let data = unpack(bytes)?;
        let mut r = Reader::new(&data);
        let meta = r.section("meta", read_meta)?;
        let chunks = r.section("chunk", read_chunks)?;
        let mut chunk_index = HashMap::with_capacity(chunks.len());
        for (i, chunk) in chunks.iter().enumerate() {
            if chunk_index.insert(chunk.guid, i).is_some() {
                return Err(ManifestError::DuplicateChunk(chunk.guid));
            }
        }
        let files = r.section("file", |r, version| read_files(r, version, &chunks, &chunk_index))?;
        // Older writers stop after the file list
        let custom_fields = if r.remaining() > 0 {
            r.section("custom field", read_custom_fields)?
        } else {
            Vec::new()
        };
        if r.remaining() > 0 {
            return Err(ManifestError::TrailingData);
        }
        Ok(Self {
            feature_level: meta.feature_level,
            is_file_data: meta.is_file_data,
            app_id: meta.app_id,
            app_name: meta.app_name,
            build_version: meta.build_version,
            build_id: meta.build_id,
            launch_exe: meta.launch_exe,
            launch_command: meta.launch_command,
            prereq_ids: meta.prereq_ids,
            prereq_name: meta.prereq_name,
            prereq_path: meta.prereq_path,
            prereq_args: meta.prereq_args,
            custom_fields,
            chunks,
            files,
            chunk_index,
        })
    }

    /// Every chunk, in manifest order.
    pub fn chunks(&self) -> &[ChunkInfo] {
        &self.chunks
    }

    /// Every file, in manifest order.
    pub fn files(&self) -> &[FileEntry] {
        &self.files
    }

    /// The chunk with `guid`.
    pub fn chunk(&self, guid: &Guid) -> Option<&ChunkInfo> {
        self.chunk_index.get(guid).and_then(|i| self.chunks.get(*i))
    }

    /// The file whose manifest name is `path`.
    pub fn file(&self, path: &str) -> Option<&FileEntry> {
        self.files.iter().find(|f| f.path.as_str() == path)
    }

    /// The installed size: every file's bytes.
    pub fn install_size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// What a full download fetches: every chunk's `.chunk` file.
    pub fn download_size(&self) -> u64 {
        self.chunks.iter().map(|c| c.download_size).sum()
    }
}

/// Unpacks `input`, which must give exactly `len` bytes; the caller bounds `len`.
pub(crate) fn inflate(input: &[u8], len: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(len);
    ZlibDecoder::new(input)
        .take(len as u64 + 1)
        .read_to_end(&mut out)
        .ok()?;
    (out.len() == len).then_some(out)
}

/// The header, then the data it describes, unpacked and checked against the header's SHA-1.
fn unpack(bytes: &[u8]) -> Result<Vec<u8>, ManifestError> {
    if bytes.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{') {
        return Err(ManifestError::Json);
    }
    let mut r = Reader::new(bytes);
    if r.u32().map_err(|_| ManifestError::NotAManifest)? != MAGIC {
        return Err(ManifestError::NotAManifest);
    }
    let header_size = r.u32()? as usize;
    let size_unpacked = r.u32()? as usize;
    let size_packed = r.u32()? as usize;
    let sha1 = Sha1Digest(r.array()?);
    let stored_as = r.u8()?;
    let _version = r.u32()?;
    if header_size < HEADER_SIZE || header_size > bytes.len() {
        return Err(ManifestError::BadHeader);
    }
    if stored_as & STORED_ENCRYPTED != 0 {
        return Err(ManifestError::Encrypted);
    }
    if size_unpacked > MAX_DATA {
        return Err(ManifestError::TooLarge);
    }
    let body = &bytes[header_size..];
    let data = if stored_as & STORED_COMPRESSED != 0 {
        if body.len() != size_packed {
            return Err(ManifestError::BadHeader);
        }
        inflate(body, size_unpacked).ok_or(ManifestError::Inflate)?
    } else {
        if body.len() != size_unpacked {
            return Err(ManifestError::BadHeader);
        }
        body.to_vec()
    };
    if Sha1Digest::of(&data) != sha1 {
        return Err(ManifestError::DataHash);
    }
    Ok(data)
}

fn read_meta(r: &mut Reader<'_>, data_version: u8) -> Result<Meta, ManifestError> {
    let feature_level = r.u32()?;
    let is_file_data = r.u8()? != 0;
    let app_id = r.u32()?;
    let app_name = r.fstring()?;
    let build_version = r.fstring()?;
    let launch_exe = r.fstring()?;
    let launch_command = r.fstring()?;
    let prereq_ids = (0..r.count(4, "prerequisites")?)
        .map(|_| r.fstring())
        .collect::<Result<_, _>>()?;
    let prereq_name = r.fstring()?;
    let prereq_path = r.fstring()?;
    let prereq_args = r.fstring()?;
    let build_id = if data_version >= 1 { Some(r.fstring()?) } else { None };
    // Data version 2's uninstall action is left unread, as the section skip allows
    Ok(Meta {
        feature_level,
        is_file_data,
        app_id,
        app_name,
        build_version,
        build_id,
        launch_exe,
        launch_command,
        prereq_ids,
        prereq_name,
        prereq_path,
        prereq_args,
    })
}

/// The chunk list, stored column by column.
fn read_chunks(r: &mut Reader<'_>, _version: u8) -> Result<Vec<ChunkInfo>, ManifestError> {
    // GUID, rolling hash, SHA-1, group, window size, download size
    let n = r.count(16 + 8 + 20 + 1 + 4 + 8, "chunks")?;
    let guids = (0..n).map(|_| r.guid()).collect::<Result<Vec<_>, _>>()?;
    let hashes = (0..n).map(|_| r.u64()).collect::<Result<Vec<_>, _>>()?;
    let shas = (0..n)
        .map(|_| r.array().map(Sha1Digest))
        .collect::<Result<Vec<_>, _>>()?;
    let groups = (0..n).map(|_| r.u8()).collect::<Result<Vec<_>, _>>()?;
    let windows = (0..n).map(|_| r.u32()).collect::<Result<Vec<_>, _>>()?;
    let sizes = (0..n).map(|_| r.i64()).collect::<Result<Vec<_>, _>>()?;
    let mut chunks = Vec::with_capacity(n);
    for i in 0..n {
        chunks.push(ChunkInfo {
            guid: guids[i],
            rolling_hash: hashes[i],
            sha1: shas[i],
            group: groups[i],
            window_size: windows[i],
            download_size: u64::try_from(sizes[i]).map_err(|_| ManifestError::NegativeSize)?,
        });
    }
    Ok(chunks)
}

/// The file list, column by column like the chunks, then checked against the chunks and the folder rules.
fn read_files(
    r: &mut Reader<'_>,
    version: u8,
    chunks: &[ChunkInfo],
    chunk_index: &HashMap<Guid, usize>,
) -> Result<Vec<FileEntry>, ManifestError> {
    // Name and symlink lengths, SHA-1, flags, tag and part counts
    let n = r.count(4 + 4 + 20 + 1 + 4 + 4, "files")?;
    let names = (0..n).map(|_| r.fstring()).collect::<Result<Vec<_>, _>>()?;
    let symlinks = (0..n).map(|_| r.fstring()).collect::<Result<Vec<_>, _>>()?;
    let shas = (0..n)
        .map(|_| r.array().map(Sha1Digest))
        .collect::<Result<Vec<_>, _>>()?;
    let flags = (0..n).map(|_| r.u8().map(FileFlags)).collect::<Result<Vec<_>, _>>()?;
    let mut tags = Vec::with_capacity(n);
    for _ in 0..n {
        let k = r.count(4, "install tags")?;
        tags.push((0..k).map(|_| r.fstring()).collect::<Result<Vec<_>, _>>()?);
    }
    let mut parts = Vec::with_capacity(n);
    for _ in 0..n {
        let k = r.count(28, "file parts")?;
        let mut list = Vec::with_capacity(k);
        for _ in 0..k {
            let start = r.pos();
            let size = r.u32()? as usize;
            if size < 28 {
                return Err(ManifestError::BadPart);
            }
            list.push(ChunkPart {
                guid: r.guid()?,
                offset: r.u32()?,
                size: r.u32()?,
            });
            r.skip_to(start.checked_add(size).ok_or(ManifestError::BadPart)?)?;
        }
        parts.push(list);
    }
    let mut md5s = vec![None; n];
    let mut mimes = vec![String::new(); n];
    if version >= 1 {
        for md5 in &mut md5s {
            if r.u32()? != 0 {
                *md5 = Some(r.array()?);
            }
        }
        for mime in &mut mimes {
            *mime = r.fstring()?;
        }
    }
    let mut sha256s = vec![None; n];
    if version >= 2 {
        for sha256 in &mut sha256s {
            *sha256 = Some(r.array()?);
        }
    }

    let mut seen = HashSet::with_capacity(n);
    let mut files = Vec::with_capacity(n);
    for (i, ((((name, symlink), parts), install_tags), (md5, mime_type))) in names
        .into_iter()
        .zip(symlinks)
        .zip(parts)
        .zip(tags)
        .zip(md5s.into_iter().zip(mimes))
        .enumerate()
    {
        let path = InstallPath::new(&name).map_err(|problem| ManifestError::UnsafePath {
            name: name.clone(),
            problem,
        })?;
        if !symlink.is_empty() {
            return Err(ManifestError::Symlink { name });
        }
        // Windows and macOS fold case, so two names that differ only in case are one file there
        if !seen.insert(name.to_lowercase()) {
            return Err(ManifestError::DuplicatePath { name });
        }
        for part in &parts {
            let Some(chunk) = chunk_index.get(&part.guid).and_then(|c| chunks.get(*c)) else {
                return Err(ManifestError::UnknownChunk { name });
            };
            if u64::from(part.offset) + u64::from(part.size) > u64::from(chunk.window_size) {
                return Err(ManifestError::PartOutOfRange { name });
            }
        }
        files.push(FileEntry {
            path,
            sha1: shas[i],
            flags: flags[i],
            install_tags,
            size: parts.iter().map(|p| u64::from(p.size)).sum(),
            parts,
            md5,
            mime_type,
            sha256: sha256s[i],
        });
    }
    Ok(files)
}

fn read_custom_fields(r: &mut Reader<'_>, _version: u8) -> Result<Vec<(String, String)>, ManifestError> {
    let n = r.count(8, "custom fields")?;
    let keys = (0..n).map(|_| r.fstring()).collect::<Result<Vec<_>, _>>()?;
    let values = (0..n).map(|_| r.fstring()).collect::<Result<Vec<_>, _>>()?;
    Ok(keys.into_iter().zip(values).collect())
}

#[cfg(any(test, feature = "test-support"))]
pub mod synthetic;
#[cfg(test)]
mod tests;
