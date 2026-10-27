//! Synthetic manifests, written the way Epic's writer lays them out. The archived ones are Epic's files and stay out
//! of the repository; `tests/archive.rs` fetches them by pinned URL and SHA-1 when asked to.

use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;

use super::*;

pub(crate) struct TestChunk {
    pub guid: Guid,
    pub data: Vec<u8>,
    pub group: u8,
    pub rolling_hash: u64,
}

pub(crate) struct TestFile {
    pub name: String,
    pub symlink: String,
    /// (chunk index, offset, size)
    pub parts: Vec<(usize, u32, u32)>,
}

pub(crate) struct Build {
    pub chunks: Vec<TestChunk>,
    pub files: Vec<TestFile>,
    pub custom: Vec<(String, String)>,
    pub utf16_names: bool,
    /// Bytes a newer writer might add at the end of each section
    pub section_padding: usize,
    pub chunk_count: Option<u32>,
    pub compress: bool,
    /// This file's listed SHA-1 matches no data (the installer's second-failure case)
    pub wrong_sha1: Option<usize>,
}

pub(crate) fn chunk(n: u8, len: usize) -> TestChunk {
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

pub(crate) fn file(name: &str, parts: &[(usize, u32, u32)]) -> TestFile {
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

fn fstring(out: &mut Vec<u8>, s: &str, utf16: bool) {
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

fn guid(out: &mut Vec<u8>, g: Guid) {
    g.0.iter().for_each(|v| out.extend(v.to_le_bytes()));
}

fn section(out: &mut Vec<u8>, version: u8, body: &[u8], padding: usize) {
    out.extend((4 + 1 + body.len() as u32 + padding as u32).to_le_bytes());
    out.push(version);
    out.extend(body);
    out.extend(std::iter::repeat_n(0xEE, padding));
}

impl Build {
    pub(crate) fn file_data(&self, file: &TestFile) -> Vec<u8> {
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
    pub(crate) fn data(&self) -> Vec<u8> {
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

    pub(crate) fn bytes(&self) -> Vec<u8> {
        pack(&self.data(), self.compress)
    }
}

/// The installer checks a listed MD5 (2.1.1's manifests carry them).
pub(crate) fn md5_of(data: &[u8]) -> [u8; 16] {
    let mut md5 = crate::download::md5::Md5::new();
    md5.update(data);
    md5.finish()
}

/// The header around `data`.
pub(crate) fn pack(data: &[u8], compress: bool) -> Vec<u8> {
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

pub(crate) fn parse(bytes: &[u8]) -> Result<Manifest, ManifestError> {
    Manifest::parse(bytes, &Sha1Digest::of(bytes))
}

fn parse_build(build: &Build) -> Result<Manifest, ManifestError> {
    parse(&build.bytes())
}

#[test]
fn a_synthetic_manifest_parses_in_full() {
    let build = Build::default();
    let m = parse_build(&build).unwrap();
    assert_eq!(m.feature_level, 18);
    assert_eq!(m.app_id, 7);
    assert_eq!(m.app_name, "Jackal");
    assert_eq!(m.build_version, "1.14.7.647472");
    assert_eq!(m.build_id.as_deref(), Some("build-id"));
    assert_eq!(m.launch_exe, "Dauntless.exe");
    assert_eq!(m.launch_command, "-epicportal");
    assert_eq!(m.prereq_ids, ["prereq-id"]);
    assert_eq!(m.prereq_args, "/quiet");
    assert_eq!(m.custom_fields, [("BuildLabel".to_owned(), "Live".to_owned())]);

    assert_eq!(m.chunks().len(), 2);
    let c = &m.chunks()[0];
    assert_eq!(c.guid, build.chunks[0].guid);
    assert_eq!(c.sha1, Sha1Digest::of(&build.chunks[0].data));
    assert_eq!(c.window_size, 64);
    assert_eq!(c.download_size, 64 / 2 + 41);
    assert_eq!(m.chunk(&build.chunks[1].guid).map(|c| c.window_size), Some(48));
    assert_eq!(
        c.cdn_path(m.feature_level),
        "ChunksV4/01/0123456789ABCDEE_0000000100001001ABCD0001FFFFFFF1.chunk"
    );

    assert_eq!(m.files().len(), 2);
    let exe = m.file("Archon/Binaries/Win64/Game.exe").unwrap();
    assert_eq!(exe.size, 96);
    assert_eq!(exe.parts.len(), 2);
    assert_eq!(exe.sha1, Sha1Digest::of(&build.file_data(&build.files[1])));
    assert!(exe.flags.read_only());
    assert_eq!(exe.install_tags, [""]);
    assert_eq!(exe.md5, Some(md5_of(&build.file_data(&build.files[1]))));
    assert_eq!(exe.mime_type, "application/octet-stream");
    assert_eq!(
        exe.sha256,
        Some(sha2::Sha256::digest(build.file_data(&build.files[1])).into())
    );
    assert_eq!(m.install_size(), 16 + 96);
    assert_eq!(m.download_size(), (64 / 2 + 41) + (48 / 2 + 41));
}

#[test]
fn chunk_dirs_follow_the_feature_level() {
    assert_eq!(chunk_dir(2), "Chunks");
    assert_eq!(chunk_dir(3), "ChunksV2");
    assert_eq!(chunk_dir(6), "ChunksV3");
    assert_eq!(chunk_dir(15), "ChunksV4");
    assert_eq!(chunk_dir(17), "ChunksV4");
}

#[test]
fn utf16_names_uncompressed_data_and_padded_sections_parse() {
    let mut build = Build {
        utf16_names: true,
        compress: false,
        section_padding: 9,
        ..Build::default()
    };
    build
        .files
        .push(file("Archon/Content/Localization/ファイル.locres", &[(1, 0, 1)]));
    let m = parse_build(&build).unwrap();
    assert_eq!(m.files().len(), 3);
    assert_eq!(
        m.files()[2].path.as_str(),
        "Archon/Content/Localization/ファイル.locres"
    );
    assert_eq!(m.custom_fields.len(), 1);
}

#[test]
fn nothing_is_read_before_the_pin_matches() {
    let bytes = Build::default().bytes();
    let pin: Sha1Digest = "aa70748e4c0a776a991f2bac10ad342adaa31476".parse().unwrap();
    assert_eq!(
        Manifest::parse(&bytes, &pin).unwrap_err(),
        ManifestError::PinMismatch {
            expected: pin,
            actual: Sha1Digest::of(&bytes)
        }
    );
    // Garbage under the wrong pin is a pin mismatch, not a parse error: the parser never saw it
    assert!(matches!(
        Manifest::parse(b"garbage", &pin),
        Err(ManifestError::PinMismatch { .. })
    ));
    assert_eq!(parse(b"garbage").unwrap_err(), ManifestError::NotAManifest);
    assert_eq!(
        parse(b"  {\"ManifestFileVersion\":\"\"}").unwrap_err(),
        ManifestError::Json
    );
}

#[test]
fn sha1_text_round_trips() {
    let text = "AA70748E4C0A776A991F2BAC10AD342ADAA31476";
    let digest: Sha1Digest = text.parse().unwrap();
    assert_eq!(digest.to_string(), text.to_lowercase());
    assert_eq!("aa70".parse::<Sha1Digest>(), Err(BadSha1));
    assert_eq!("zz".repeat(20).parse::<Sha1Digest>(), Err(BadSha1));
    assert_eq!("+a".repeat(20).parse::<Sha1Digest>(), Err(BadSha1));
    assert_eq!("é".repeat(20).parse::<Sha1Digest>(), Err(BadSha1));
}

#[test]
fn a_header_that_lies_is_refused() {
    let data = Build::default().data();
    let mut bytes = pack(&data, true);
    // The header's SHA-1 of the data
    bytes[16] ^= 1;
    assert_eq!(parse(&bytes).unwrap_err(), ManifestError::DataHash);

    let mut bytes = pack(&data, true);
    bytes[36] |= STORED_ENCRYPTED;
    assert_eq!(parse(&bytes).unwrap_err(), ManifestError::Encrypted);

    let mut bytes = pack(&data, true);
    bytes[8..12].copy_from_slice(&(MAX_DATA as u32 + 1).to_le_bytes());
    assert_eq!(parse(&bytes).unwrap_err(), ManifestError::TooLarge);

    let mut bytes = pack(&data, true);
    bytes[4..8].copy_from_slice(&40u32.to_le_bytes());
    assert_eq!(parse(&bytes).unwrap_err(), ManifestError::BadHeader);

    let mut bytes = pack(&data, true);
    bytes.pop();
    assert_eq!(parse(&bytes).unwrap_err(), ManifestError::BadHeader);

    // The unpacked size understated: the stream gives more than promised
    let mut bytes = pack(&data, true);
    bytes[8..12].copy_from_slice(&(data.len() as u32 - 1).to_le_bytes());
    assert_eq!(parse(&bytes).unwrap_err(), ManifestError::Inflate);
}

#[test]
fn names_that_leave_the_install_folder_are_refused() {
    for name in [
        "../outside.dll",
        "Archon/../../outside.dll",
        "/etc/cron.d/x",
        "C:/Windows/System32/x.dll",
        "\\\\server\\share\\x.dll",
        "Archon\\..\\..\\x.dll",
        "Engine/NUL.txt",
        "Dauntless.exe:hidden",
    ] {
        let mut build = Build::default();
        build.files[0].name = name.to_owned();
        assert!(
            matches!(parse_build(&build), Err(ManifestError::UnsafePath { name: n, .. }) if n == name),
            "{name:?}"
        );
    }
}

#[test]
fn symlinks_and_names_given_twice_are_refused() {
    let mut build = Build::default();
    build.files[1].symlink = "../../outside".to_owned();
    assert!(matches!(parse_build(&build), Err(ManifestError::Symlink { .. })));

    let mut build = Build::default();
    build.files.push(file("version.TXT", &[(0, 0, 1)]));
    assert_eq!(
        parse_build(&build).unwrap_err(),
        ManifestError::DuplicatePath {
            name: "version.TXT".to_owned()
        }
    );
}

#[test]
fn parts_must_name_a_listed_chunk_and_stay_inside_it() {
    let mut build = Build::default();
    build.files[0].parts = vec![(0, 60, 5)];
    assert!(matches!(parse_build(&build), Err(ManifestError::PartOutOfRange { .. })));

    let mut build = Build::default();
    build.files[0].parts = vec![(0, u32::MAX, 2)];
    assert!(matches!(parse_build(&build), Err(ManifestError::PartOutOfRange { .. })));

    // A file pointing at a chunk the list does not hold: drop the chunk from the list only
    let build = Build::default();
    let mut data = build.data();
    let other = chunk(9, 64).guid;
    let at = data
        .windows(16)
        .rposition(|w| w == guid_bytes(build.chunks[1].guid))
        .unwrap();
    data[at..at + 16].copy_from_slice(&guid_bytes(other));
    assert!(matches!(
        parse(&pack(&data, true)),
        Err(ManifestError::UnknownChunk { name }) if name == "Archon/Binaries/Win64/Game.exe"
    ));

    let mut build = Build::default();
    build.chunks.push(chunk(1, 8));
    assert_eq!(
        parse_build(&build).unwrap_err(),
        ManifestError::DuplicateChunk(build.chunks[0].guid)
    );
}

fn guid_bytes(g: Guid) -> Vec<u8> {
    let mut out = Vec::new();
    guid(&mut out, g);
    out
}

#[test]
fn counts_larger_than_the_data_are_refused_before_allocating() {
    let build = Build {
        chunk_count: Some(u32::MAX),
        ..Build::default()
    };
    assert_eq!(
        parse_build(&build).unwrap_err(),
        ManifestError::CountTooLarge { what: "chunks" }
    );
}

#[test]
fn truncated_and_padded_data_is_refused() {
    let data = Build::default().data();
    for cut in [1, 5, 17, data.len() / 2, data.len() - 1] {
        let err = parse(&pack(&data[..cut], true)).unwrap_err();
        assert!(
            matches!(
                err,
                ManifestError::Truncated { .. } | ManifestError::SectionSize { .. } | ManifestError::BadString { .. }
            ),
            "cut at {cut}: {err:?}"
        );
    }
    let mut data = data;
    data.push(0);
    assert!(parse(&pack(&data, true)).is_err());
}

#[test]
fn every_prefix_and_flipped_byte_fails_cleanly() {
    let data = Build::default().data();
    for cut in 0..data.len() {
        let _ = parse(&pack(&data[..cut], false));
    }
    for i in 0..data.len() {
        let mut bad = data.clone();
        bad[i] ^= 0xFF;
        let _ = parse(&pack(&bad, false));
    }
}

#[test]
fn bad_strings_are_refused() {
    let build = Build::default();
    let data = build.data();
    // "Jackal" with its NUL replaced
    let at = data.windows(7).position(|w| w == b"Jackal\0").unwrap();
    let mut bad = data.clone();
    bad[at + 6] = b'!';
    assert!(matches!(parse(&pack(&bad, true)), Err(ManifestError::BadString { .. })));
    let mut bad = data;
    bad[at] = 0xFF;
    assert!(matches!(parse(&pack(&bad, true)), Err(ManifestError::BadString { .. })));
}
