//! The Epic builds DaintyLass can install (DD22, DE4): each version's manifest pin, where its manifest is archived
//! and the executable's SHA-256. The launcher's compiled-in client profiles and the host kit's `install-game` both
//! read this table, so a pin is changed in one place. It is **a stand-in, to be replaced**: the 1.14.7 module
//! serves its own profile through content (DD12), and once it does this table goes.

use crate::manifest::Sha1Digest;

/// The community archive's raw files, pinned to the commit that added each manifest (DE1's `tests/archive.rs`).
pub const ARCHIVE: &str = "https://raw.githubusercontent.com/addison2k/DauntlessManifestArchive";

/// The executable, relative to the game folder (the one with `Archon`); the same in every build listed.
pub const EXECUTABLE: &str = "Archon/Binaries/Win64/Dauntless-Win64-Shipping.exe";

/// One installable build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Build {
    /// The game version as modules and invites name it
    pub version: &'static str,
    /// DD4's display name
    pub display_name: &'static str,
    /// Epic's build version, which the manifest carries
    pub build_version: &'static str,
    /// The SHA-1 of the build's manifest, 40 hex digits: what every source's copy must match.
    pub manifest_sha1: &'static str,
    /// The archive's commit and file
    pub archive: (&'static str, &'static str),
    /// The executable's SHA-256: the ones Mystic's guide pins (rustic-isles `docs/INSTALLING_DAUNTLESS.md`)
    pub executable_sha256: &'static str,
}

impl Build {
    /// The pin every source's manifest is checked against.
    pub fn pin(&self) -> Option<Sha1Digest> {
        self.manifest_sha1.parse().ok()
    }

    /// [`Build::executable_sha256`] as bytes; `None` if it is not 64 hex digits.
    pub fn executable_sha256_bytes(&self) -> Option<[u8; 32]> {
        hex32(self.executable_sha256)
    }

    /// The archive's copy of the manifest.
    pub fn manifest_source(&self) -> String {
        format!("{ARCHIVE}/{}/manifests/Windows/{}", self.archive.0, self.archive.1)
    }
}

/// Newest first.
pub const BUILDS: [Build; 1] = [Build {
    version: "1.14.7",
    display_name: "Rustic Isles 1.14.7",
    build_version: "1.14.7.647472",
    manifest_sha1: "aa70748e4c0a776a991f2bac10ad342adaa31476",
    archive: (
        "92e6082335b1c2234c88e9aec7fe2b704fc24494",
        "Dauntless_1.14.7.647472.manifest",
    ),
    executable_sha256: "f3f95fbf22709ef077a11632ed95d675b7209b7e0172d9ecb9ba7d02a1fbb66b",
}];

/// The build for a game version, as typed (`1.14.7`).
pub fn find(version: &str) -> Option<&'static Build> {
    BUILDS.iter().find(|build| build.version == version)
}

fn hex32(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (byte, pair) in out.iter_mut().zip(text.as_bytes().as_chunks::<2>().0) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_build_is_whole() {
        for build in BUILDS {
            assert!(build.pin().is_some(), "{}", build.version);
            assert!(build.executable_sha256_bytes().is_some(), "{}", build.version);
            assert!(build.build_version.starts_with(build.version), "{}", build.version);
            assert!(build.manifest_source().starts_with("https://"), "{}", build.version);
        }
        assert_eq!(find("1.14.7").map(|b| b.build_version), Some("1.14.7.647472"));
        assert!(find("1.4.4").is_none() && find("1.14").is_none());
    }

    #[test]
    fn the_hash_is_read_as_hex() {
        assert_eq!(hex32(&"ab".repeat(32)), Some([0xab; 32]));
        assert_eq!(hex32(&"ab".repeat(31)), None);
        assert_eq!(hex32(&"+b".repeat(32)), None);
    }
}
