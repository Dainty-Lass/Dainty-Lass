// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Account ids and the client-visible userId (core spec §2.2).

use std::fmt;
use std::str::FromStr;

use module_api::{GlobalId, IdKind, NodeId};

/// `account:<ULID>@<node>`, owned by (homed on) its node.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AccountId(GlobalId);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("an account id is account:<ULID>@<node>")]
pub struct AccountIdError;

impl AccountId {
    /// A new id minted on `node`.
    pub fn new(node: &NodeId) -> Self {
        Self(GlobalId::new(IdKind::Account, node))
    }

    pub fn global(&self) -> &GlobalId {
        &self.0
    }

    pub fn home(&self) -> &NodeId {
        self.0.owner()
    }
}

impl TryFrom<GlobalId> for AccountId {
    type Error = AccountIdError;

    fn try_from(id: GlobalId) -> Result<Self, AccountIdError> {
        if id.kind() == IdKind::Account {
            Ok(Self(id))
        } else {
            Err(AccountIdError)
        }
    }
}

impl FromStr for AccountId {
    type Err = AccountIdError;

    fn from_str(text: &str) -> Result<Self, AccountIdError> {
        text.parse::<GlobalId>().map_err(|_| AccountIdError)?.try_into()
    }
}

impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The client-visible id: any text; a new one is a UUID v4 from the replay-aware source.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UserId(pub String);

impl UserId {
    pub fn random() -> Self {
        Self(crate::replay::next_uuid())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let node = NodeId::new("n1").unwrap();
        let id = AccountId::new(&node);
        let back: AccountId = id.to_string().parse().unwrap();
        assert_eq!(back, id);
        assert_eq!(back.home(), &node);
        let hunt = GlobalId::new(IdKind::Hunt, &node);
        assert_eq!(hunt.to_string().parse::<AccountId>(), Err(AccountIdError));
        assert_eq!(AccountId::try_from(hunt), Err(AccountIdError));
        assert_eq!(UserId::random().as_str().len(), 36);
    }
}
