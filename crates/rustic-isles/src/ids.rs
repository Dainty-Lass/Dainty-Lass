// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The two ids areas pass around (core spec §2.2): the network-wide account id and the userId the client sees.

use std::fmt;
use std::str::FromStr;

use dainty_module_api::{GlobalId, IdKind, NodeId};

/// A `GlobalId` known to name an account; the node in it is the account's home.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AccountId {
    id: GlobalId,
}

/// Why a text or id is not an account id.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AccountIdError {
    /// The text does not parse as a network id.
    #[error("{0:?} is not a network id; account ids are written account:<ULID>@<node>")]
    Unreadable(String),
    /// The kind it names; the text is the spec's (§2.2), which callers see
    #[error("an account id is account:<ULID>@<node>")]
    OtherKind(&'static str),
}

impl AccountId {
    /// Minted on `node`, which becomes its home.
    pub fn new(node: &NodeId) -> Self {
        Self {
            id: GlobalId::new(IdKind::Account, node),
        }
    }

    /// The underlying network id.
    pub fn global(&self) -> &GlobalId {
        &self.id
    }

    /// The account's home node.
    pub fn home(&self) -> &NodeId {
        self.id.owner()
    }
}

impl FromStr for AccountId {
    type Err = AccountIdError;

    fn from_str(text: &str) -> Result<Self, AccountIdError> {
        let Ok(id) = text.parse::<GlobalId>() else {
            return Err(AccountIdError::Unreadable(text.to_owned()));
        };
        match id.kind() {
            IdKind::Account => Ok(Self { id }),
            other => Err(AccountIdError::OtherKind(other.as_str())),
        }
    }
}

impl TryFrom<GlobalId> for AccountId {
    type Error = AccountIdError;

    fn try_from(id: GlobalId) -> Result<Self, AccountIdError> {
        if id.kind() != IdKind::Account {
            return Err(AccountIdError::OtherKind(id.kind().as_str()));
        }
        Ok(Self { id })
    }
}

impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.id, f)
    }
}

/// What the game client calls a player: free text; new ones are UUID v4s from the replay-aware source.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UserId {
    text: String,
}

impl UserId {
    /// Wraps a client userId as it is.
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }

    /// A fresh UUID v4 from the replay-aware source.
    pub fn random() -> Self {
        Self::new(crate::replay::next_uuid())
    }

    /// The userId text.
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_ids() {
        let node = NodeId::new("n1").unwrap();
        let minted = AccountId::new(&node);
        let parsed: AccountId = minted.to_string().parse().unwrap();
        assert_eq!((&parsed, parsed.home()), (&minted, &node));
        assert_eq!(AccountId::try_from(minted.global().clone()), Ok(minted));
        let hunt = GlobalId::new(IdKind::Hunt, &node);
        assert!(matches!(
            hunt.to_string().parse::<AccountId>(),
            Err(AccountIdError::OtherKind(_))
        ));
        assert!(matches!(AccountId::try_from(hunt), Err(AccountIdError::OtherKind(_))));
        assert!(matches!(
            "nope".parse::<AccountId>(),
            Err(AccountIdError::Unreadable(_))
        ));
    }

    #[test]
    fn user_ids() {
        assert_eq!(UserId::random().as_str().len(), 36);
        assert_eq!(UserId::new("u1").to_string(), "u1");
    }
}
