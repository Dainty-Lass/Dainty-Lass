//! [`GameVersion`], the key a game module is loaded under (DD4), and [`ApiVersion`], the contract a module
//! was built against.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// The most dot-separated parts a game version may have.
pub const MAX_VERSION_PARTS: usize = 8;

/// A Dauntless version: dot-separated integers, compared exactly (no ranges, `1.14` is not `1.14.0`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct GameVersion(Vec<u32>);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VersionError {
    #[error("a game version is dot-separated integers, such as 1.14.7")]
    Malformed,
    #[error("a game version has at most {MAX_VERSION_PARTS} parts")]
    TooLong,
}

impl GameVersion {
    /// A three-part version, such as 1.14.7: the form every Dauntless release has.
    pub fn release(major: u32, minor: u32, patch: u32) -> Self {
        Self(vec![major, minor, patch])
    }

    pub fn parts(&self) -> &[u32] {
        &self.0
    }
}

impl FromStr for GameVersion {
    type Err = VersionError;

    fn from_str(text: &str) -> Result<Self, VersionError> {
        let mut parts = Vec::new();
        for part in text.split('.') {
            if parts.len() == MAX_VERSION_PARTS {
                return Err(VersionError::TooLong);
            }
            // Leading zeros refused, so the text a version is shown as is the only text that names it
            let canonical =
                !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()) && (part == "0" || !part.starts_with('0'));
            if !canonical {
                return Err(VersionError::Malformed);
            }
            parts.push(part.parse().map_err(|_| VersionError::Malformed)?);
        }
        Ok(Self(parts))
    }
}

impl TryFrom<String> for GameVersion {
    type Error = VersionError;

    fn try_from(text: String) -> Result<Self, VersionError> {
        text.parse()
    }
}

impl From<GameVersion> for String {
    fn from(version: GameVersion) -> String {
        version.to_string()
    }
}

impl fmt::Display for GameVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, part) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(".")?;
            }
            write!(f, "{part}")?;
        }
        Ok(())
    }
}

/// The module API contract a module was built against, independent of the crate's own version: a change a
/// built module can't follow raises `major`, an addition raises `minor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiVersion {
    pub major: u16,
    pub minor: u16,
}

/// The contract this build of the crate speaks.
pub const API_VERSION: ApiVersion = ApiVersion { major: 1, minor: 0 };

impl ApiVersion {
    /// Whether a host speaking `self` can run a module built against `module`: same major, and nothing
    /// newer than the host knows.
    pub fn accepts(self, module: ApiVersion) -> bool {
        self.major == module.major && module.minor <= self.minor
    }
}

impl fmt::Display for ApiVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_display_and_compare_exactly() {
        for text in ["1.14.7", "1.12.0", "1.4.4", "0", "2.1.1", "10.0.30"] {
            let version: GameVersion = text.parse().unwrap();
            assert_eq!(version.to_string(), text);
        }
        let a: GameVersion = "1.14".parse().unwrap();
        let b: GameVersion = "1.14.0".parse().unwrap();
        assert_ne!(a, b);
        assert_eq!("1.14.7".parse::<GameVersion>().unwrap().parts(), &[1, 14, 7]);
        for bad in [
            "",
            ".",
            "1.",
            ".1",
            "1..2",
            "1.04",
            "v1",
            "1.14.7-beta",
            "+1",
            " 1",
            "1.x",
            "99999999999",
        ] {
            assert_eq!(bad.parse::<GameVersion>(), Err(VersionError::Malformed), "{bad:?}");
        }
        assert_eq!("1.2.3.4.5.6.7.8.9".parse::<GameVersion>(), Err(VersionError::TooLong));
    }

    #[test]
    fn versions_serialise_as_their_text() {
        let version: GameVersion = "1.14.7".parse().unwrap();
        assert_eq!(serde_json::to_string(&version).unwrap(), r#""1.14.7""#);
        assert_eq!(serde_json::from_str::<GameVersion>(r#""1.14.7""#).unwrap(), version);
        assert!(serde_json::from_str::<GameVersion>(r#""1.014""#).is_err());
        assert!(serde_json::from_str::<GameVersion>("1").is_err());
    }

    #[test]
    fn api_versions_accept_older_minors_of_the_same_major() {
        let host = ApiVersion { major: 1, minor: 2 };
        assert!(host.accepts(ApiVersion { major: 1, minor: 0 }));
        assert!(host.accepts(ApiVersion { major: 1, minor: 2 }));
        assert!(!host.accepts(ApiVersion { major: 1, minor: 3 }));
        assert!(!host.accepts(ApiVersion { major: 2, minor: 0 }));
        assert!(!host.accepts(ApiVersion { major: 0, minor: 2 }));
        assert!(API_VERSION.accepts(API_VERSION));
    }
}
