//! Account ids (DD17, DD18). An account's identity is a network-wide [`GlobalId`] owned by its home node, from the
//! first commit, so nothing is migrated when a server joins a network. The game client never sees it: it sees the
//! [`UserId`], Node's `userId` (a UUID for new accounts, whatever a migrated database held), in paths and bodies.

use std::fmt;
use std::str::FromStr;

use module_api::{GlobalId, IdKind, NodeId};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AccountId(GlobalId);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("an account id is account:<ULID>@<node>")]
pub struct AccountIdError;

impl AccountId {
    /// A new account homed on `node`.
    pub fn new(node: &NodeId) -> Self {
        Self(GlobalId::new(IdKind::Account, node))
    }

    pub fn global(&self) -> &GlobalId {
        &self.0
    }

    /// The node that owns the account (its home).
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

/// Node's `userId`, as the client sends and reads it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UserId(pub String);

impl UserId {
    /// A new account's: `crypto.randomUUID()`, as `controllers/launcherAuth.ts` mints them.
    pub fn random() -> Self {
        Self(crate::game_data::season_window::next_uuid().to_string())
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
    fn account_ids_are_global_account_ids() {
        let node: NodeId = "oregon-1".parse().unwrap();
        let id = AccountId::new(&node);
        assert_eq!(id.home(), &node);
        assert_eq!(id.to_string().parse::<AccountId>().unwrap(), id);
        let hunt = GlobalId::new(IdKind::Hunt, &node);
        assert!(AccountId::try_from(hunt).is_err());
        assert_eq!(UserId::random().as_str().len(), 36);
    }
}
