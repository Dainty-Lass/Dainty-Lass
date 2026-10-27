//! Usernames as the directory claims them: the protocol's registration rule (`protocol::account`, the
//! shared `username.json` table), unique network-wide ignoring case.

use std::fmt;

use protocol::account::{UsernameCheck, check_username};
use serde::{Deserialize, Serialize};

/// A username that passes [`check_username`]. Its [`Username::key`] is what is unique.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Username(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not a username ({})", .0.as_str())]
pub struct InvalidUsername(pub UsernameCheck);

impl Username {
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
    #![allow(clippy::unwrap_used)]

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
}
