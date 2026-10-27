//! RU5 chat: friends (the XMPP side is DaintyLass's chat plugin, DD21).

use super::{AccountId, AnyConnection, Tx, repository};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FriendshipStatus {
    Accepted,
    Pending,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FriendshipDirection {
    Inbound,
    Outbound,
}

/// `FriendEdgeRecord`: one direction of a friendship.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FriendEdgeRecord {
    pub owner: AccountId,
    pub other: AccountId,
    pub status: FriendshipStatus,
    pub direction: Option<FriendshipDirection>,
    pub created: String,
    pub favorite: Option<bool>,
}

repository! {
    pub trait FriendshipRepository("friendships") {
        fn list_for_owner(&self, conn: &mut AnyConnection, owner: &AccountId) -> Vec<FriendEdgeRecord>;
        fn find(&self, conn: &mut AnyConnection, owner: &AccountId, other: &AccountId) -> Option<FriendEdgeRecord>;
        fn upsert(&self, tx: &mut Tx<'_>, edge: &FriendEdgeRecord) -> ();
        fn remove(&self, tx: &mut Tx<'_>, owner: &AccountId, other: &AccountId) -> ();
    }
}
