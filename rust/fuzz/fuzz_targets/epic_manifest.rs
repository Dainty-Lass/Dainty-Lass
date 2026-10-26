//! What the launcher and the host kit read from outside before installing from Epic (docs/dainty-lass-plan.md DE1,
//! DX2; docs/security/epic.md): a download manifest and a CDN chunk. A manifest is pinned by SHA-1 before it is
//! parsed, so the parser only ever sees a file someone chose; the target pins whatever it is given, and also wraps
//! the bytes in a valid header so the sections are reached. Whatever the bytes: no panic, no allocation a count
//! asks for beyond what the data can hold, and an accepted manifest keeps every file inside the install folder, each once, with parts inside
//! listed chunks. An accepted chunk is exactly the size and SHA-1 its entry says.
#![no_main]

use std::path::Path;

use arbitrary::Arbitrary;
use epic::chunk;
use epic::manifest::{ChunkInfo, Guid, InstallPath, Manifest, ManifestError, Sha1Digest};
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
enum Input<'a> {
    /// A whole manifest file
    File(&'a [u8]),
    /// A manifest's unpacked sections, given a valid header
    Sections(&'a [u8]),
    /// A chunk file, checked against an entry made from its own payload so the fuzzer can reach success
    Chunk { guid: [u32; 4], header: u8, raw: &'a [u8] },
}

fn header_around(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(41 + data.len());
    out.extend(0x44BE_C00Cu32.to_le_bytes());
    out.extend(41u32.to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(Sha1Digest::of(data).0);
    out.push(0);
    out.extend(18u32.to_le_bytes());
    out.extend(data);
    out
}

fn check_manifest(bytes: &[u8]) {
    let pin = Sha1Digest::of(bytes);
    let mut wrong = pin;
    wrong.0[0] ^= 1;
    assert!(matches!(Manifest::parse(bytes, &wrong), Err(ManifestError::PinMismatch { .. })));
    let Ok(manifest) = Manifest::parse(bytes, &pin) else {
        return;
    };
    let root = Path::new("install");
    let mut seen = std::collections::HashSet::new();
    for file in manifest.files() {
        let name = file.path.as_str();
        assert_eq!(InstallPath::new(name).as_ref(), Ok(&file.path));
        assert!(!name.starts_with('/') && !name.contains('\\') && !name.contains(':'));
        assert!(file.path.components().all(|c| !c.is_empty() && c != "." && c != ".."));
        assert!(file.path.under(root).starts_with(root));
        assert!(seen.insert(name.to_lowercase()), "{name:?} twice");
        let mut size = 0u64;
        for part in &file.parts {
            let chunk = manifest.chunk(&part.guid).expect("a listed chunk");
            assert!(u64::from(part.offset) + u64::from(part.size) <= u64::from(chunk.window_size));
            size += u64::from(part.size);
        }
        assert_eq!(size, file.size);
    }
}

fuzz_target!(|input: Input<'_>| match input {
    Input::File(bytes) => check_manifest(bytes),
    Input::Sections(data) => check_manifest(&header_around(data)),
    Input::Chunk { guid, header, raw } => {
        let payload = raw.get(usize::from(header)..).unwrap_or_default();
        let info = ChunkInfo {
            guid: Guid(guid),
            rolling_hash: 0,
            sha1: Sha1Digest::of(payload),
            group: 0,
            window_size: payload.len() as u32,
            download_size: raw.len() as u64,
        };
        if let Ok(data) = chunk::decode(raw, &info) {
            assert_eq!(data.len(), info.window_size as usize);
            assert_eq!(Sha1Digest::of(&data), info.sha1);
        }
    }
});
