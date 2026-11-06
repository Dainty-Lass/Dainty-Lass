// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Friendships (spec §8): one row per direction; the other end may live on another node (DD17).

use diesel::QueryableByName;
use diesel::sql_types::{Bool, Nullable, Text};

use crate::sql::{account_id, count, sql};
use crate::{AccountId, AnyConnection, RepoError, RepoResult, Tx, UserId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FriendshipStatus {
    Accepted,
    Pending,
    Blocked,
}

impl FriendshipStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "ACCEPTED",
            Self::Pending => "PENDING",
            Self::Blocked => "BLOCKED",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "ACCEPTED" => Some(Self::Accepted),
            "PENDING" => Some(Self::Pending),
            "BLOCKED" => Some(Self::Blocked),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FriendshipDirection {
    Inbound,
    Outbound,
}

impl FriendshipDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inbound => "INBOUND",
            Self::Outbound => "OUTBOUND",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "INBOUND" => Some(Self::Inbound),
            "OUTBOUND" => Some(Self::Outbound),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FriendEdgeRecord {
    pub owner: AccountId,
    pub other: AccountId,
    pub other_user_id: UserId,
    pub status: FriendshipStatus,
    pub direction: Option<FriendshipDirection>,
    pub created: String,
    pub favorite: bool,
}

store_trait! {
    pub trait FriendshipRepository = "friendships" {
        fn list_for_owner(&self, conn: &mut AnyConnection, owner: &AccountId) -> Vec<FriendEdgeRecord>;
        fn find(&self, conn: &mut AnyConnection, owner: &AccountId, other: &AccountId) -> Option<FriendEdgeRecord>;
        fn upsert(&self, tx: &mut Tx<'_>, record: &FriendEdgeRecord) -> ();
        fn remove(&self, tx: &mut Tx<'_>, owner: &AccountId, other: &AccountId) -> ();
        fn remove_all_for_account(&self, tx: &mut Tx<'_>, account: &AccountId) -> u64;
    }
}

pub struct SqlFriendships;

#[derive(QueryableByName)]
struct EdgeRow {
    #[diesel(sql_type = Text)]
    owner: String,
    #[diesel(sql_type = Text)]
    other: String,
    #[diesel(sql_type = Text)]
    other_user_id: String,
    #[diesel(sql_type = Text)]
    status: String,
    #[diesel(sql_type = Nullable<Text>)]
    direction: Option<String>,
    #[diesel(sql_type = Text)]
    created: String,
    #[diesel(sql_type = Bool)]
    favorite: bool,
}

impl EdgeRow {
    fn decode(self) -> RepoResult<FriendEdgeRecord> {
        let status = FriendshipStatus::parse(&self.status)
            .ok_or_else(|| RepoError::data(format!("friend_edges.status \"{}\"", self.status)))?;
        let direction = match &self.direction {
            None => None,
            Some(text) => Some(
                FriendshipDirection::parse(text)
                    .ok_or_else(|| RepoError::data(format!("friend_edges.direction \"{text}\"")))?,
            ),
        };
        Ok(FriendEdgeRecord {
            owner: account_id(&self.owner, "friend_edges.owner")?,
            other: account_id(&self.other, "friend_edges.other")?,
            other_user_id: UserId::new(self.other_user_id),
            status,
            direction,
            created: self.created,
            favorite: self.favorite,
        })
    }
}

impl FriendshipRepository for SqlFriendships {
    fn list_for_owner(&self, conn: &mut AnyConnection, owner: &AccountId) -> RepoResult<Vec<FriendEdgeRecord>> {
        sql("SELECT * FROM friend_edges WHERE owner = ? ORDER BY id ASC")
            .t(owner.to_string())
            .load::<EdgeRow>(conn)?
            .into_iter()
            .map(EdgeRow::decode)
            .collect()
    }

    fn find(
        &self,
        conn: &mut AnyConnection,
        owner: &AccountId,
        other: &AccountId,
    ) -> RepoResult<Option<FriendEdgeRecord>> {
        sql("SELECT * FROM friend_edges WHERE owner = ? AND other = ?")
            .t(owner.to_string())
            .t(other.to_string())
            .first::<EdgeRow>(conn)?
            .map(EdgeRow::decode)
            .transpose()
    }

    fn upsert(&self, tx: &mut Tx<'_>, r: &FriendEdgeRecord) -> RepoResult<()> {
        let conn = tx.conn();
        // In place, so the edge keeps its list position
        let updated = sql(
            "UPDATE friend_edges SET other_user_id = ?, status = ?, direction = ?, created = ?, \
             favorite = ? WHERE owner = ? AND other = ?",
        )
        .t(r.other_user_id.as_str())
        .t(r.status.as_str())
        .ot(r.direction.map(FriendshipDirection::as_str))
        .t(&*r.created)
        .b(r.favorite)
        .t(r.owner.to_string())
        .t(r.other.to_string())
        .run(conn)?;
        if updated == 0 {
            sql(
                "INSERT INTO friend_edges (owner, other, other_user_id, status, direction, created, favorite) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .t(r.owner.to_string())
            .t(r.other.to_string())
            .t(r.other_user_id.as_str())
            .t(r.status.as_str())
            .ot(r.direction.map(FriendshipDirection::as_str))
            .t(&*r.created)
            .b(r.favorite)
            .run(conn)?;
        }
        Ok(())
    }

    fn remove(&self, tx: &mut Tx<'_>, owner: &AccountId, other: &AccountId) -> RepoResult<()> {
        sql("DELETE FROM friend_edges WHERE owner = ? AND other = ?")
            .t(owner.to_string())
            .t(other.to_string())
            .run(tx.conn())?;
        Ok(())
    }

    fn remove_all_for_account(&self, tx: &mut Tx<'_>, account: &AccountId) -> RepoResult<u64> {
        remove_all_edges(tx.conn(), account)
    }
}

pub(crate) fn remove_all_edges(conn: &mut AnyConnection, account: &AccountId) -> RepoResult<u64> {
    let id = account.to_string();
    Ok(count(
        sql("DELETE FROM friend_edges WHERE owner = ? OR other = ?")
            .t(id.clone())
            .t(id)
            .run(conn)?,
    ))
}
