//! A `.chunk` file from Epic's CDN: a header, then the data, zlib-packed or stored as is (about a quarter of
//! 1.14.7's chunks are stored, which is what broke the archive's first download tool). [`decode`] checks it against
//! the pinned manifest's entry, never against the chunk's own header: the GUID, the unpacked size and the SHA-1.

use crate::manifest::{ChunkInfo, Guid, MAX_DATA, Sha1Digest, inflate};

const MAGIC: u32 = 0xB1FE_3AA2;
/// Magic, version, header size, packed size, GUID, rolling hash, stored-as
const V1_HEADER: usize = 4 + 4 + 4 + 4 + 16 + 8 + 1;
/// Version 2 adds the SHA-1 and the hash type, version 3 the unpacked size
const V2_HEADER: usize = V1_HEADER + 20 + 1;
const V3_HEADER: usize = V2_HEADER + 4;
const STORED_COMPRESSED: u8 = 0x01;
const STORED_ENCRYPTED: u8 = 0x02;

/// Why a chunk was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ChunkError {
    #[error("not an Epic chunk")]
    NotAChunk,
    #[error("the chunk's header is inconsistent")]
    BadHeader,
    #[error("the chunk is {actual}, not {expected}")]
    WrongChunk { expected: Guid, actual: Guid },
    #[error("the chunk is encrypted")]
    Encrypted,
    #[error("the chunk is larger than any game's")]
    TooLarge,
    #[error("the chunk's data does not unpack to its {expected} bytes")]
    Size { expected: u32 },
    #[error("the chunk's SHA-1 is {actual}, not the manifest's {expected}")]
    Hash { expected: Sha1Digest, actual: Sha1Digest },
}

/// The chunk's data, once it is the one `expected` describes, whole and unchanged.
pub fn decode(raw: &[u8], expected: &ChunkInfo) -> Result<Vec<u8>, ChunkError> {
    if raw.len() < V1_HEADER || u32_at(raw, 0) != MAGIC {
        return Err(ChunkError::NotAChunk);
    }
    let version = u32_at(raw, 4);
    let header_size = u32_at(raw, 8) as usize;
    let packed_size = u32_at(raw, 12) as usize;
    let guid = Guid([u32_at(raw, 16), u32_at(raw, 20), u32_at(raw, 24), u32_at(raw, 28)]);
    let stored_as = raw[40];
    let minimum = match version {
        0 => return Err(ChunkError::BadHeader),
        1 => V1_HEADER,
        2 => V2_HEADER,
        _ => V3_HEADER,
    };
    if header_size < minimum || header_size > raw.len() || raw.len() - header_size != packed_size {
        return Err(ChunkError::BadHeader);
    }
    if guid != expected.guid {
        return Err(ChunkError::WrongChunk {
            expected: expected.guid,
            actual: guid,
        });
    }
    if stored_as & STORED_ENCRYPTED != 0 {
        return Err(ChunkError::Encrypted);
    }
    let window = expected.window_size as usize;
    if window > MAX_DATA {
        return Err(ChunkError::TooLarge);
    }
    let payload = &raw[header_size..];
    let data = if stored_as & STORED_COMPRESSED != 0 {
        inflate(payload, window)
    } else {
        (payload.len() == window).then(|| payload.to_vec())
    }
    .ok_or(ChunkError::Size {
        expected: expected.window_size,
    })?;
    let actual = Sha1Digest::of(&data);
    if actual != expected.sha1 {
        return Err(ChunkError::Hash {
            expected: expected.sha1,
            actual,
        });
    }
    Ok(data)
}

/// Callers check the length first.
fn u32_at(raw: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([raw[at], raw[at + 1], raw[at + 2], raw[at + 3]])
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::Compression;
    use flate2::write::ZlibEncoder;

    use super::*;
    use crate::manifest::tests::{Build, parse};

    fn encode(guid: Guid, data: &[u8], compress: bool, version: u32) -> Vec<u8> {
        let payload = if compress {
            let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
            z.write_all(data).unwrap();
            z.finish().unwrap()
        } else {
            data.to_vec()
        };
        let header_size = match version {
            1 => V1_HEADER,
            2 => V2_HEADER,
            _ => V3_HEADER,
        };
        let mut out = Vec::new();
        out.extend(MAGIC.to_le_bytes());
        out.extend(version.to_le_bytes());
        out.extend((header_size as u32).to_le_bytes());
        out.extend((payload.len() as u32).to_le_bytes());
        guid.0.iter().for_each(|v| out.extend(v.to_le_bytes()));
        out.extend(0u64.to_le_bytes());
        out.push(u8::from(compress));
        if version >= 2 {
            out.extend(Sha1Digest::of(data).0);
            out.push(2);
        }
        if version >= 3 {
            out.extend((data.len() as u32).to_le_bytes());
        }
        out.extend(payload);
        out
    }

    #[test]
    fn packed_and_stored_chunks_of_every_header_version_decode_and_assemble() {
        let build = Build::default();
        let manifest = parse(&build.bytes()).unwrap();
        for version in 1..=3 {
            for compress in [true, false] {
                let decoded: Vec<Vec<u8>> = build
                    .chunks
                    .iter()
                    .zip(manifest.chunks())
                    .map(|(c, info)| decode(&encode(c.guid, &c.data, compress, version), info).unwrap())
                    .collect();
                for (test_file, file) in build.files.iter().zip(manifest.files()) {
                    let assembled: Vec<u8> = file
                        .parts
                        .iter()
                        .flat_map(|p| {
                            let i = manifest.chunks().iter().position(|c| c.guid == p.guid).unwrap();
                            decoded[i][p.offset as usize..(p.offset + p.size) as usize].to_vec()
                        })
                        .collect();
                    assert_eq!(assembled, build.file_data(test_file));
                    assert_eq!(Sha1Digest::of(&assembled), file.sha1);
                }
            }
        }
    }

    #[test]
    fn a_chunk_that_is_not_the_manifests_is_refused() {
        let build = Build::default();
        let manifest = parse(&build.bytes()).unwrap();
        let info = &manifest.chunks()[0];
        let data = &build.chunks[0].data;

        let other = encode(build.chunks[1].guid, data, true, 3);
        assert!(matches!(decode(&other, info), Err(ChunkError::WrongChunk { .. })));

        let mut changed = data.clone();
        changed[3] ^= 1;
        assert!(matches!(
            decode(&encode(info.guid, &changed, true, 3), info),
            Err(ChunkError::Hash { .. })
        ));
        assert!(matches!(
            decode(&encode(info.guid, &changed, false, 1), info),
            Err(ChunkError::Hash { .. })
        ));

        let longer = [data.as_slice(), b"x"].concat();
        for compress in [true, false] {
            assert_eq!(
                decode(&encode(info.guid, &longer, compress, 3), info),
                Err(ChunkError::Size { expected: 64 })
            );
            assert_eq!(
                decode(&encode(info.guid, &data[1..], compress, 3), info),
                Err(ChunkError::Size { expected: 64 })
            );
        }

        let mut encrypted = encode(info.guid, data, false, 3);
        encrypted[40] |= STORED_ENCRYPTED;
        assert_eq!(decode(&encrypted, info), Err(ChunkError::Encrypted));
    }

    #[test]
    fn malformed_chunks_fail_cleanly() {
        let build = Build::default();
        let manifest = parse(&build.bytes()).unwrap();
        let info = &manifest.chunks()[0];
        let good = encode(info.guid, &build.chunks[0].data, true, 3);
        assert_eq!(decode(&good[..40], info), Err(ChunkError::NotAChunk));
        assert_eq!(decode(&[0; 80], info), Err(ChunkError::NotAChunk));
        assert_eq!(decode(&good[..good.len() - 1], info), Err(ChunkError::BadHeader));

        let mut bad = good.clone();
        bad[8..12].copy_from_slice(&(V2_HEADER as u32).to_le_bytes());
        assert_eq!(decode(&bad, info), Err(ChunkError::BadHeader));
        let mut bad = good.clone();
        bad[4..8].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(decode(&bad, info), Err(ChunkError::BadHeader));

        for cut in 0..good.len() {
            let _ = decode(&good[..cut], info);
        }
        // Only a later header version and the header's own hashes and sizes go unnoticed: the manifest's entry counts
        let unchecked = |i: usize| (4..8).contains(&i) || (32..40).contains(&i) || (41..V3_HEADER).contains(&i);
        for i in 0..good.len() {
            let mut bad = good.clone();
            bad[i] ^= 0xFF;
            let result = decode(&bad, info);
            assert_eq!(result.is_ok(), unchecked(i), "byte {i}: {result:?}");
        }
    }
}
