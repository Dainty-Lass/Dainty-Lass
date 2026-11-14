//! Names checked when they are made and when they are read: [`ModuleId`], and the hex digests a manifest
//! pins files with.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// The longest name a [`ModuleId`] and the other names in this crate may be.
pub const MAX_NAME_LEN: usize = 64;

/// A name broke the naming rule.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "{what} must be 1 to {MAX_NAME_LEN} characters of a-z, 0-9 and '-', starting with a letter and not ending with '-'"
)]
pub struct NameError {
    /// What the name was for, such as "a module id".
    pub what: &'static str,
}

/// The rule every name in this crate follows: they end up in paths, logs and `settings.toml` keys.
pub(crate) fn check_name(text: &str, what: &'static str) -> Result<(), NameError> {
    let bytes = text.as_bytes();
    let ok = !bytes.is_empty()
        && bytes.len() <= MAX_NAME_LEN
        && bytes[0].is_ascii_lowercase()
        && bytes[bytes.len() - 1] != b'-'
        && bytes
            .iter()
            .all(|&b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if ok { Ok(()) } else { Err(NameError { what }) }
}

/// A name type: checked on parse and on deserialisation, serialised as its text.
macro_rules! name_type {
    ($(#[$doc:meta])* $name:ident, $what:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// `text` as a name, when it follows the naming rule.
            pub fn new(text: impl Into<String>) -> Result<Self, $crate::ids::NameError> {
                let text = text.into();
                $crate::ids::check_name(&text, $what)?;
                Ok(Self(text))
            }

            /// The name's text.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::str::FromStr for $name {
            type Err = $crate::ids::NameError;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                Self::new(text)
            }
        }

        impl TryFrom<String> for $name {
            type Error = $crate::ids::NameError;

            fn try_from(text: String) -> Result<Self, Self::Error> {
                Self::new(text)
            }
        }

        impl From<$name> for String {
            fn from(name: $name) -> String {
                name.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}
pub(crate) use name_type;

name_type!(
    /// A module's id, such as `isles-1-14-7`: its key in `settings.toml` and its name in logs and status.
    ModuleId,
    "a module id"
);

/// Text that is not a digest of the expected length in lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("a SHA-{bits} digest is {len} lowercase hex characters")]
pub struct DigestError {
    bits: usize,
    len: usize,
}

/// A digest of `BYTES` bytes as lowercase hex, the form Legendary, Mystic's guide and `settings.toml` write.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct HexDigest<const BYTES: usize>([u8; BYTES]);

/// A SHA-1 digest: Epic download manifests are pinned by one (DD22).
pub type Sha1Hex = HexDigest<20>;
/// A SHA-256 digest: executables and external modules are pinned by one (DD5, DD22).
pub type Sha256Hex = HexDigest<32>;

impl<const BYTES: usize> HexDigest<BYTES> {
    /// The digest of these raw bytes.
    pub fn from_bytes(bytes: [u8; BYTES]) -> Self {
        Self(bytes)
    }

    /// The raw digest bytes.
    pub fn as_bytes(&self) -> &[u8; BYTES] {
        &self.0
    }

    fn error() -> DigestError {
        DigestError {
            bits: if BYTES == 20 { 1 } else { BYTES * 8 },
            len: BYTES * 2,
        }
    }
}

impl<const BYTES: usize> FromStr for HexDigest<BYTES> {
    type Err = DigestError;

    fn from_str(text: &str) -> Result<Self, DigestError> {
        let text = text.as_bytes();
        if text.len() != BYTES * 2 {
            return Err(Self::error());
        }
        let nibble = |b: u8| match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            _ => None,
        };
        let mut out = [0u8; BYTES];
        for (byte, pair) in out.iter_mut().zip(text.as_chunks::<2>().0) {
            let (Some(high), Some(low)) = (nibble(pair[0]), nibble(pair[1])) else {
                return Err(Self::error());
            };
            *byte = high << 4 | low;
        }
        Ok(Self(out))
    }
}

impl<const BYTES: usize> TryFrom<String> for HexDigest<BYTES> {
    type Error = DigestError;

    fn try_from(text: String) -> Result<Self, DigestError> {
        text.parse()
    }
}

impl<const BYTES: usize> From<HexDigest<BYTES>> for String {
    fn from(digest: HexDigest<BYTES>) -> String {
        digest.to_string()
    }
}

impl<const BYTES: usize> fmt::Display for HexDigest<BYTES> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl<const BYTES: usize> fmt::Debug for HexDigest<BYTES> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_ids_are_lowercase_names() {
        for good in ["isles-1-14-7", "revived-1-4-4", "stub", "a", "rustic-freeza"] {
            assert_eq!(good.parse::<ModuleId>().unwrap().as_str(), good);
        }
        let long = "a".repeat(MAX_NAME_LEN + 1);
        for bad in [
            "", "Isles", "1isles", "isles-", "-isles", "isles_1", "isles 1", "isles/1", "é", &long,
        ] {
            assert!(bad.parse::<ModuleId>().is_err(), "{bad:?}");
        }
        assert!(serde_json::from_str::<ModuleId>(r#""../x""#).is_err());
        assert_eq!(
            serde_json::to_string(&ModuleId::new("stub").unwrap()).unwrap(),
            r#""stub""#
        );
    }

    #[test]
    fn digests_are_exact_lowercase_hex() {
        let sha1 = "aa70748e0000000000000000000000000031476f";
        assert_eq!(sha1.parse::<Sha1Hex>().unwrap().to_string(), sha1);
        let sha256 = "0123456789abcdef".repeat(4);
        let digest: Sha256Hex = sha256.parse().unwrap();
        assert_eq!(digest.to_string(), sha256);
        assert_eq!(digest.as_bytes()[..2], [0x01, 0x23]);
        assert!(sha256.to_uppercase().parse::<Sha256Hex>().is_err());
        assert!(sha1.parse::<Sha256Hex>().is_err());
        assert!(format!("{sha1}0").parse::<Sha1Hex>().is_err());
        assert!("zz".repeat(20).parse::<Sha1Hex>().is_err());
        assert_eq!(
            sha1.parse::<Sha256Hex>().unwrap_err().to_string(),
            "a SHA-256 digest is 64 lowercase hex characters"
        );
        assert_eq!(
            "".parse::<Sha1Hex>().unwrap_err().to_string(),
            "a SHA-1 digest is 40 lowercase hex characters"
        );
    }
}
