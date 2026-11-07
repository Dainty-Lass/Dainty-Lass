//! Usernames as the directory claims them: the protocol's registration rule (`dainty_protocol::account`, the
//! shared `username.json` table), unique network-wide ignoring case.

use std::fmt;

use dainty_protocol::account::{UsernameCheck, check_username};
use serde::{Deserialize, Serialize};

/// A username that passes [`check_username`]. Its [`Username::key`] is what is unique.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Username(String);

/// A name that fails [`check_username`]; holds the rule it fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not a username ({})", .0.as_str())]
pub struct InvalidUsername(pub UsernameCheck);

impl Username {
    /// Refuses a name that fails [`check_username`].
    pub fn new(text: impl Into<String>) -> Result<Self, InvalidUsername> {
        let text = text.into();
        match check_username(&text) {
            UsernameCheck::Ok => Ok(Self(text)),
            check => Err(InvalidUsername(check)),
        }
    }

    /// As the player typed it at registration.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The claim: ASCII lower case, which folds every name the rule accepts (A-Z, a-z, 0-9, `_`), as
    /// Paradox's `displayNameNormalized` does.
    pub fn key(&self) -> String {
        self.0.to_ascii_lowercase()
    }

    /// A temporary username for a join clash (DN13): appends `_{suffix}` (`Fox_2`), shortening the base first
    /// so the result stays within [`dainty_protocol::account::USERNAME_MAX`] (16 characters).
    #[must_use]
    pub fn with_suffix(&self, suffix: u32) -> Self {
        let suffix = format!("_{}", suffix.max(2));
        let keep = dainty_protocol::account::USERNAME_MAX
            .saturating_sub(suffix.len())
            .min(self.0.len());
        Self(format!("{}{suffix}", &self.0[..keep]))
    }
}

impl TryFrom<String> for Username {
    type Error = InvalidUsername;

    fn try_from(text: String) -> Result<Self, InvalidUsername> {
        Self::new(text)
    }
}

impl From<Username> for String {
    fn from(name: Username) -> String {
        name.0
    }
}

impl fmt::Display for Username {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;

    #[test]
    fn claims_ignore_case_only() {
        let fox = Username::new("Fox_01").unwrap();
        assert_eq!(fox.as_str(), "Fox_01");
        assert_eq!(fox.key(), "fox_01");
        assert_eq!(Username::new("FOX_01").unwrap().key(), fox.key());
        assert_ne!(Username::new("Fox_1").unwrap().key(), fox.key());
        assert_eq!(Username::new("ab"), Err(InvalidUsername(UsernameCheck::TooShort)));
        assert!(serde_json::from_str::<Username>(r#""bad name""#).is_err());
        assert_eq!(serde_json::to_string(&fox).unwrap(), r#""Fox_01""#);
    }

    #[test]
    fn clash_suffixes_fit_within_sixteen_characters() {
        let fox = Username::new("Fox").unwrap();
        assert_eq!(fox.with_suffix(2).as_str(), "Fox_2");
        assert_eq!(fox.with_suffix(15).as_str(), "Fox_15");

        let sixteen = Username::new("SixteenCharName1").unwrap();
        assert_eq!(sixteen.as_str().len(), 16);
        let s2 = sixteen.with_suffix(2);
        assert_eq!(s2.as_str(), "SixteenCharNam_2");
        assert_eq!(s2.as_str().len(), 16);
        assert_eq!(check_username(s2.as_str()), UsernameCheck::Ok);

        let s12 = sixteen.with_suffix(12);
        assert_eq!(s12.as_str(), "SixteenCharNa_12");
        assert_eq!(s12.as_str().len(), 16);
        assert_eq!(check_username(s12.as_str()), UsernameCheck::Ok);
    }
}
