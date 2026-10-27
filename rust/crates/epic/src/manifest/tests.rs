//! Manifests read in full and refused piece by piece (`synthetic` writes them).

use super::synthetic::*;
use super::*;

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
