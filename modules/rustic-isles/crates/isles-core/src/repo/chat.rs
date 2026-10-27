//! RU5 chat: the friendship graph (`FriendshipRepository`; the XMPP side is DaintyLass's chat plugin, DD21).

use diesel::prelude::*;

use super::{AccountId, AnyConnection, RepoError, RepoResult, Tx, UserId, repository};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FriendshipStatus {
    Accepted,
    Pending,
    Blocked,
}

impl FriendshipStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "ACCEPTED",
            Self::Pending => "PENDING",
            Self::Blocked => "BLOCKED",
        }
    }

    fn parse(text: &str) -> RepoResult<Self> {
        match text {
            "ACCEPTED" => Ok(Self::Accepted),
            "PENDING" => Ok(Self::Pending),
            "BLOCKED" => Ok(Self::Blocked),
            other => Err(RepoError::Data(format!("friend_edges.status {other:?}"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FriendshipDirection {
    Inbound,
    Outbound,
}

impl FriendshipDirection {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Inbound => "INBOUND",
            Self::Outbound => "OUTBOUND",
        }
    }

    fn parse(text: &str) -> RepoResult<Self> {
        match text {
            "INBOUND" => Ok(Self::Inbound),
            "OUTBOUND" => Ok(Self::Outbound),
            other => Err(RepoError::Data(format!("friend_edges.direction {other:?}"))),
        }
    }
}

/// `FriendEdgeRecord`: one direction of a friendship. `other_user_id` is the friend's `userId`, which the client
/// sees as `accountId`; it is kept on the edge because the friend's account may be homed on another node (DD17).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FriendEdgeRecord {
    pub owner: AccountId,
    pub other: AccountId,
    pub other_user_id: UserId,
    pub status: FriendshipStatus,
    /// Only meaningful for `Pending`; an accepted edge has none
    pub direction: Option<FriendshipDirection>,
    /// ISO-8601 as `Date#toISOString` writes it
    pub created: String,
    pub favorite: bool,
}

repository! {
    pub trait FriendshipRepository("friendships") {
        /// Every edge `owner` has (friends, pending, blocked), in the order they were first written.
        fn list_for_owner(&self, conn: &mut AnyConnection, owner: &AccountId) -> Vec<FriendEdgeRecord>;
        fn find(&self, conn: &mut AnyConnection, owner: &AccountId, other: &AccountId) -> Option<FriendEdgeRecord>;
        /// Replaces the edge, keeping its place in the owner's list.
        fn upsert(&self, tx: &mut Tx<'_>, edge: &FriendEdgeRecord) -> ();
        /// One side only: the other direction stays.
        fn remove(&self, tx: &mut Tx<'_>, owner: &AccountId, other: &AccountId) -> ();
        /// Every edge `account` owns or is the other end of (an account's deletion, `accountDeletionPlan.ts`).
        fn remove_all_for_account(&self, tx: &mut Tx<'_>, account: &AccountId) -> u64;
    }
}

diesel::table! {
    friend_edges (id) {
        id -> BigInt,
        owner -> Text,
        other -> Text,
        other_user_id -> Text,
        status -> Text,
        direction -> Nullable<Text>,
        created -> Text,
        favorite -> Bool,
    }
}

type EdgeRow = (String, String, String, String, Option<String>, String, bool);

macro_rules! edge_columns {
    () => {
        (
            friend_edges::owner,
            friend_edges::other,
            friend_edges::other_user_id,
            friend_edges::status,
            friend_edges::direction,
            friend_edges::created,
            friend_edges::favorite,
        )
    };
}

fn account(text: &str, column: &str) -> RepoResult<AccountId> {
    text.parse()
        .map_err(|_| RepoError::Data(format!("friend_edges.{column} {text:?} is not an account id")))
}

fn edge(row: EdgeRow) -> RepoResult<FriendEdgeRecord> {
    let (owner, other, other_user_id, status, direction, created, favorite) = row;
    Ok(FriendEdgeRecord {
        owner: account(&owner, "owner")?,
        other: account(&other, "other")?,
        other_user_id: UserId(other_user_id),
        status: FriendshipStatus::parse(&status)?,
        direction: direction.as_deref().map(FriendshipDirection::parse).transpose()?,
        created,
        favorite,
    })
}

/// [`FriendshipRepository`] on `store`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SqlFriendships;

impl FriendshipRepository for SqlFriendships {
    fn list_for_owner(&self, conn: &mut AnyConnection, owner: &AccountId) -> RepoResult<Vec<FriendEdgeRecord>> {
        friend_edges::table
            .filter(friend_edges::owner.eq(owner.to_string()))
            .order(friend_edges::id.asc())
            .select(edge_columns!())
            .load::<EdgeRow>(conn)?
            .into_iter()
            .map(edge)
            .collect()
    }

    fn find(
        &self,
        conn: &mut AnyConnection,
        owner: &AccountId,
        other: &AccountId,
    ) -> RepoResult<Option<FriendEdgeRecord>> {
        friend_edges::table
            .filter(friend_edges::owner.eq(owner.to_string()))
            .filter(friend_edges::other.eq(other.to_string()))
            .select(edge_columns!())
            .first::<EdgeRow>(conn)
            .optional()?
            .map(edge)
            .transpose()
    }

    fn upsert(&self, tx: &mut Tx<'_>, record: &FriendEdgeRecord) -> RepoResult<()> {
        let (owner, other) = (record.owner.to_string(), record.other.to_string());
        let direction = record.direction.map(FriendshipDirection::as_str);
        let target = friend_edges::table
            .filter(friend_edges::owner.eq(&owner))
            .filter(friend_edges::other.eq(&other));
        let updated = diesel::update(target)
            .set((
                friend_edges::other_user_id.eq(record.other_user_id.as_str()),
                friend_edges::status.eq(record.status.as_str()),
                friend_edges::direction.eq(direction),
                friend_edges::created.eq(&record.created),
                friend_edges::favorite.eq(record.favorite),
            ))
            .execute(tx.conn())?;
        if updated == 0 {
            diesel::insert_into(friend_edges::table)
                .values((
                    friend_edges::owner.eq(&owner),
                    friend_edges::other.eq(&other),
                    friend_edges::other_user_id.eq(record.other_user_id.as_str()),
                    friend_edges::status.eq(record.status.as_str()),
                    friend_edges::direction.eq(direction),
                    friend_edges::created.eq(&record.created),
                    friend_edges::favorite.eq(record.favorite),
                ))
                .execute(tx.conn())?;
        }
        Ok(())
    }

    fn remove(&self, tx: &mut Tx<'_>, owner: &AccountId, other: &AccountId) -> RepoResult<()> {
        diesel::delete(
            friend_edges::table
                .filter(friend_edges::owner.eq(owner.to_string()))
                .filter(friend_edges::other.eq(other.to_string())),
        )
        .execute(tx.conn())?;
        Ok(())
    }

    fn remove_all_for_account(&self, tx: &mut Tx<'_>, account: &AccountId) -> RepoResult<u64> {
        let id = account.to_string();
        let removed =
            diesel::delete(friend_edges::table.filter(friend_edges::owner.eq(&id).or(friend_edges::other.eq(&id))))
                .execute(tx.conn())?;
        Ok(removed as u64)
    }
}
