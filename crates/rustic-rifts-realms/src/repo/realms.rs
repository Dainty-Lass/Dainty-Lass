// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Realms with their members, bans, join requests, player invites and hub rooms; invite codes; and which realm an
//! account's game login belongs to.

use std::collections::BTreeMap;

use dainty_store::AnyConnection;
use rustic_rifts_seam::{Actor, UnixMillis};

use super::sql::{Arg, Bind, Cell as _, Statement, binds, marks, table_row, upsert};
use crate::db::{RealmsDb, RepoError};
use crate::realm::{
    AccountAt, HeartbeatUpdate, HostingPatch, HubState, INVITE_PURGE_GRACE_MS, JoinPolicy, MemberRole, RealmBan,
    RealmHosting, RealmInviteRecord, RealmMember, RealmPresenceRecord, RealmRecord, apply_hosting_patch,
};
use crate::time::Later as _;

/// How storing a new realm went.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InsertRealmResult {
    /// Stored.
    Inserted,
    /// Another realm has the name.
    NameTaken,
    /// The host already has a realm.
    HostHasRealm,
}

/// How adding a member went.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddMemberResult {
    /// Added.
    Added,
    /// The account is a member already.
    AlreadyMember,
    /// The account is banned from the realm.
    Banned,
    /// The realm is at its member limit.
    Full,
    /// No such realm.
    NotFound,
}

/// How adding a ban went.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddBanResult {
    /// Banned.
    Banned,
    /// The account was banned already.
    AlreadyBanned,
    /// The realm is at its ban limit.
    Full,
    /// No such realm.
    NotFound,
}

/// How adding a join request or player invite went.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddEntryResult {
    /// Added.
    Added,
    /// Already requested, or already invited.
    Already,
    /// The account is banned from the realm.
    Banned,
    /// The account is a member already.
    AlreadyMember,
    /// The realm's list is at its limit.
    Full,
    /// No such realm.
    NotFound,
}

/// How storing an invite code went.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InsertInviteResult {
    /// Stored.
    Inserted,
    /// Another invite has the code.
    Duplicate,
}

const REALMS: &str = "rifts_realms";
const MEMBERS: &str = "rifts_realm_members";
const BANS: &str = "rifts_realm_bans";
const JOIN_REQUESTS: &str = "rifts_realm_join_requests";
const PLAYER_INVITES: &str = "rifts_realm_player_invites";
const HUB_ROOMS: &str = "rifts_realm_hub_rooms";
const INVITES: &str = "rifts_realm_invites";
const PRESENCE: &str = "rifts_realm_presence";

// The realm's own columns first (the insert writes these), then the hosting block `write_hosting` sets.
table_row!(RealmRow {
    realm_id: String,
    name: String,
    host_account_id: String,
    host_public_key: String,
    join_policy: String,
    listed: bool,
    created_at: i64,
    last_online_at: Option<i64>,
    online_until: Option<i64>,
    version: Option<String>,
    offline_notified_for: Option<i64>,
    hosting_max_servers: Option<i64>,
    hosting_max_players: Option<i64>,
    hosting_afk_seconds: Option<i64>,
    hosting_hub_session_id: Option<String>,
    hosting_hub_state: Option<String>,
    hosting_training_state: Option<String>,
    hosting_active_servers: Option<i64>,
    hosting_updated_at: Option<i64>,
});

/// Columns of `RealmRow` the insert writes; the rest belong to hosting.
const OWN_COLUMNS: usize = 11;

fn hub_state(text: Option<String>) -> Result<HubState, RepoError> {
    Ok(text.map(HubState::load).transpose()?.unwrap_or(HubState::None))
}

impl RealmRow {
    /// Absent until written; a `NULL` column of a written record reads as its default.
    fn hosting(&self) -> Result<Option<RealmHosting>, RepoError> {
        self.hosting_updated_at
            .map(|updated_at| {
                Ok(RealmHosting {
                    hub_state: hub_state(self.hosting_hub_state.clone())?,
                    training_state: hub_state(self.hosting_training_state.clone())?,
                    updated_at: UnixMillis(updated_at),
                    hub_session_id: self.hosting_hub_session_id.clone(),
                    afk_seconds: self.hosting_afk_seconds,
                    active_servers: self.hosting_active_servers.unwrap_or(0),
                    max_players: self.hosting_max_players.unwrap_or(16),
                    max_servers: self.hosting_max_servers.unwrap_or(1),
                })
            })
            .transpose()
    }
}

table_row!(MemberRow for RealmMember {
    account_id: String,
    role: MemberRole,
    joined_at: UnixMillis,
});

table_row!(BanRow for RealmBan {
    account_id: String,
    reason: Option<String>,
    at: UnixMillis,
});

table_row!(EntryRow for AccountAt {
    account_id: String,
    at: UnixMillis,
});

table_row!(RoomRow { room_id: String });

table_row!(RealmIdRow { realm_id: String });

table_row!(InviteRow for RealmInviteRecord {
    code: String,
    realm_id: String,
    created_by: String,
    created_at: UnixMillis,
    expires_at: UnixMillis,
    max_uses: i64,
    uses: i64,
    revoked_at: Option<UnixMillis>,
});

table_row!(PresenceRow for RealmPresenceRecord {
    account_id: String,
    realm_id: String,
    bound_at: UnixMillis,
    last_seen_at: UnixMillis,
    expires_at: UnixMillis,
});

table_row!(RealmCountRow {
    realm_id: String,
    n: i64,
});

fn list(columns: &[&str]) -> String {
    columns.join(", ")
}

/// One child table's rows of a realm, in the order they were added.
fn child_rows<R: diesel::QueryableByName<dainty_store::MultiBackend> + 'static>(
    conn: &mut AnyConnection,
    columns: &[&str],
    table: &str,
    realm_id: &str,
) -> Result<Vec<R>, RepoError> {
    let text = format!("SELECT {} FROM {table} WHERE realm_id = ? ORDER BY seq", list(columns));
    Statement::new(&text, binds![realm_id])?.rows(conn)
}

fn assemble(conn: &mut AnyConnection, row: RealmRow) -> Result<RealmRecord, RepoError> {
    let id = row.realm_id.as_str();
    let members = child_rows::<MemberRow>(conn, MemberRow::COLUMNS, MEMBERS, id)?
        .into_iter()
        .map(MemberRow::record)
        .collect::<Result<_, _>>()?;
    let bans = child_rows::<BanRow>(conn, BanRow::COLUMNS, BANS, id)?
        .into_iter()
        .map(BanRow::record)
        .collect::<Result<_, _>>()?;
    let mut entries = |table| -> Result<Vec<AccountAt>, RepoError> {
        child_rows::<EntryRow>(conn, EntryRow::COLUMNS, table, id)?
            .into_iter()
            .map(EntryRow::record)
            .collect()
    };
    let join_requests = entries(JOIN_REQUESTS)?;
    let player_invites = entries(PLAYER_INVITES)?;
    let rooms: Vec<RoomRow> = child_rows(conn, RoomRow::COLUMNS, HUB_ROOMS, id)?;
    let hosting = row.hosting()?;
    let join_policy = JoinPolicy::load(row.join_policy)?;
    Ok(RealmRecord {
        join_policy,
        hosting,
        members,
        bans,
        join_requests,
        player_invites,
        hub_room_ids: rooms.into_iter().map(|room| room.room_id).collect(),
        created_at: UnixMillis(row.created_at),
        last_online_at: row.last_online_at.map(UnixMillis),
        online_until: row.online_until.map(UnixMillis),
        offline_notified_for: row.offline_notified_for.map(UnixMillis),
        listed: row.listed,
        version: row.version,
        host_public_key: row.host_public_key,
        host_account_id: row.host_account_id,
        name: row.name,
        realm_id: row.realm_id,
    })
}

/// Realms matching `tail` (a `WHERE`/`ORDER BY`/`LIMIT` clause) with their children.
fn realms_where(conn: &mut AnyConnection, tail: &str, args: Vec<Arg>) -> Result<Vec<RealmRecord>, RepoError> {
    let text = format!("SELECT {} FROM {REALMS} {tail}", list(RealmRow::COLUMNS));
    let rows: Vec<RealmRow> = Statement::new(&text, args)?.rows(conn)?;
    rows.into_iter().map(|row| assemble(conn, row)).collect()
}

fn first_realm(conn: &mut AnyConnection, tail: &str, args: Vec<Arg>) -> Result<Option<RealmRecord>, RepoError> {
    Ok(realms_where(conn, tail, args)?.into_iter().next())
}

fn any(conn: &mut AnyConnection, text: &str, args: Vec<Arg>) -> Result<bool, RepoError> {
    Ok(Statement::new(text, args)?.number(conn)? > 0)
}

fn realm_exists(conn: &mut AnyConnection, realm_id: &str) -> Result<bool, RepoError> {
    any(
        conn,
        "SELECT COUNT(*) AS n FROM rifts_realms WHERE realm_id = ?",
        binds![realm_id],
    )
}

fn owned_by(conn: &mut AnyConnection, realm_id: &str, host: &str) -> Result<bool, RepoError> {
    let text = "SELECT COUNT(*) AS n FROM rifts_realms WHERE realm_id = ? AND host_account_id = ?";
    any(conn, text, binds![realm_id, host])
}

/// A realm's child table, keyed by realm and account.
#[derive(Clone, Copy)]
struct Child(&'static str);

impl Child {
    fn has(self, conn: &mut AnyConnection, realm_id: &str, account_id: &str) -> Result<bool, RepoError> {
        let text = format!(
            "SELECT COUNT(*) AS n FROM {} WHERE realm_id = ? AND account_id = ?",
            self.0
        );
        any(conn, &text, binds![realm_id, account_id])
    }

    fn count(self, conn: &mut AnyConnection, realm_id: &str) -> Result<i64, RepoError> {
        let text = format!("SELECT COUNT(*) AS n FROM {} WHERE realm_id = ?", self.0);
        Statement::new(&text, binds![realm_id])?.number(conn)
    }

    fn at_least(self, conn: &mut AnyConnection, realm_id: &str, max: u32) -> Result<bool, RepoError> {
        Ok(self.count(conn, realm_id)? >= i64::from(max))
    }

    fn delete(self, conn: &mut AnyConnection, realm_id: &str, account_id: &str) -> Result<bool, RepoError> {
        let text = format!("DELETE FROM {} WHERE realm_id = ? AND account_id = ?", self.0);
        Ok(Statement::new(&text, binds![realm_id, account_id])?.run(conn)? > 0)
    }

    /// Appends after the realm's last row: `(realm_id, <columns>, seq)`.
    fn append(
        self,
        conn: &mut AnyConnection,
        realm_id: &str,
        columns: &[&str],
        values: Vec<Arg>,
    ) -> Result<(), RepoError> {
        let table = self.0;
        let next = format!("SELECT COALESCE(MAX(seq), 0) + 1 AS n FROM {table} WHERE realm_id = ?");
        let seq = Statement::new(&next, binds![realm_id])?.number(conn)?;
        let text = format!(
            "INSERT INTO {table} (realm_id, {}, seq) VALUES ({})",
            list(columns),
            marks(columns.len() + 2)
        );
        let mut args = binds![realm_id];
        args.extend(values);
        args.extend(binds![seq]);
        Statement::new(&text, args)?.run(conn)?;
        Ok(())
    }
}

fn add_member_row(conn: &mut AnyConnection, realm_id: &str, member: &RealmMember) -> Result<(), RepoError> {
    Child(MEMBERS).append(conn, realm_id, MemberRow::COLUMNS, MemberRow::values(member))
}

fn add_ban_row(conn: &mut AnyConnection, realm_id: &str, ban: &RealmBan) -> Result<(), RepoError> {
    Child(BANS).append(conn, realm_id, BanRow::COLUMNS, BanRow::values(ban))
}

fn add_entry_row(
    conn: &mut AnyConnection,
    table: &'static str,
    realm_id: &str,
    entry: &AccountAt,
) -> Result<(), RepoError> {
    Child(table).append(conn, realm_id, EntryRow::COLUMNS, EntryRow::values(entry))
}

/// A member's row, never the host's.
fn delete_plain_member(conn: &mut AnyConnection, realm_id: &str, account_id: &str) -> Result<usize, RepoError> {
    let text = "DELETE FROM rifts_realm_members WHERE realm_id = ? AND account_id = ? AND role = ?";
    Statement::new(text, binds![realm_id, account_id, MemberRole::Member])?.run(conn)
}

fn write_hosting(conn: &mut AnyConnection, realm_id: &str, hosting: &RealmHosting) -> Result<(), RepoError> {
    let sets: Vec<String> = RealmRow::COLUMNS[OWN_COLUMNS..]
        .iter()
        .map(|column| format!("{column} = ?"))
        .collect();
    let text = format!("UPDATE rifts_realms SET {} WHERE realm_id = ?", sets.join(", "));
    let args = binds![
        hosting.max_servers,
        hosting.max_players,
        hosting.afk_seconds,
        hosting.hub_session_id,
        hosting.hub_state,
        hosting.training_state,
        hosting.active_servers,
        hosting.updated_at,
        realm_id,
    ];
    Statement::new(&text, args)?.run(conn)?;
    Ok(())
}

/// Children first, for the foreign keys; invites go with their realm, presence rows do not.
const REALM_TABLES: [&str; 7] = [MEMBERS, BANS, JOIN_REQUESTS, PLAYER_INVITES, HUB_ROOMS, INVITES, REALMS];

fn delete_realm(conn: &mut AnyConnection, realm_id: &str) -> Result<(), RepoError> {
    for table in REALM_TABLES {
        Statement::new(&format!("DELETE FROM {table} WHERE realm_id = ?"), binds![realm_id])?.run(conn)?;
    }
    Ok(())
}

fn insert_realm_rows(conn: &mut AnyConnection, realm: &RealmRecord) -> Result<(), RepoError> {
    let own = &RealmRow::COLUMNS[..OWN_COLUMNS];
    let text = format!("INSERT INTO rifts_realms ({}) VALUES ({})", list(own), marks(own.len()));
    let args = binds![
        realm.realm_id,
        realm.name,
        realm.host_account_id,
        realm.host_public_key,
        realm.join_policy,
        realm.listed,
        realm.created_at,
        realm.last_online_at,
        realm.online_until,
        realm.version,
        realm.offline_notified_for,
    ];
    Statement::new(&text, args)?.run(conn)?;
    let id = realm.realm_id.as_str();
    if let Some(hosting) = &realm.hosting {
        write_hosting(conn, id, hosting)?;
    }
    realm
        .members
        .iter()
        .try_for_each(|member| add_member_row(conn, id, member))?;
    realm.bans.iter().try_for_each(|ban| add_ban_row(conn, id, ban))?;
    for (table, entries) in [
        (JOIN_REQUESTS, &realm.join_requests),
        (PLAYER_INVITES, &realm.player_invites),
    ] {
        entries
            .iter()
            .try_for_each(|entry| add_entry_row(conn, table, id, entry))?;
    }
    for (room, seq) in realm.hub_room_ids.iter().zip(1i64..) {
        let text = "INSERT INTO rifts_realm_hub_rooms (room_id, realm_id, seq) VALUES (?, ?, ?)";
        Statement::new(text, binds![room, id, seq])?.run(conn)?;
    }
    Ok(())
}

fn invite_by_code(conn: &mut AnyConnection, code: &str) -> Result<Option<RealmInviteRecord>, RepoError> {
    let text = format!("SELECT {} FROM {INVITES} WHERE code = ?", list(InviteRow::COLUMNS));
    Statement::new(&text, binds![code])?
        .first::<InviteRow>(conn)?
        .map(InviteRow::record)
        .transpose()
}

/// One realm's membership, bans, requests and invites, and its invite codes.
#[derive(Clone)]
pub struct RealmsRepository {
    db: RealmsDb,
}

impl RealmsRepository {
    /// The realm tables in `db`.
    pub fn new(db: RealmsDb) -> Self {
        Self { db }
    }

    async fn first(&self, tail: &'static str, value: &str) -> Result<Option<RealmRecord>, RepoError> {
        let value = value.to_owned();
        self.db.read(move |conn| first_realm(conn, tail, binds![value])).await
    }

    /// The realm with this id.
    pub async fn find_by_id(&self, realm_id: &str) -> Result<Option<RealmRecord>, RepoError> {
        self.first("WHERE realm_id = ?", realm_id).await
    }

    /// The realm with this canonical name.
    pub async fn find_by_name(&self, name: &str) -> Result<Option<RealmRecord>, RepoError> {
        self.first("WHERE name = ?", name).await
    }

    /// The realm this account hosts.
    pub async fn find_by_host(&self, host: &str) -> Result<Option<RealmRecord>, RepoError> {
        self.first("WHERE host_account_id = ?", host).await
    }

    /// The realm whose hub has this room id.
    pub async fn find_by_hub_room_id(&self, room_id: &str) -> Result<Option<RealmRecord>, RepoError> {
        let tail = "WHERE realm_id IN (SELECT realm_id FROM rifts_realm_hub_rooms WHERE room_id = ?)";
        self.first(tail, room_id).await
    }

    async fn having_child(
        &self,
        table: &'static str,
        account_id: &str,
        max: u32,
    ) -> Result<Vec<RealmRecord>, RepoError> {
        let tail =
            format!("WHERE realm_id IN (SELECT realm_id FROM {table} WHERE account_id = ?) ORDER BY name LIMIT ?");
        let args = binds![account_id, i64::from(max)];
        self.db.read(move |conn| realms_where(conn, &tail, args)).await
    }

    /// The host's own realm included.
    pub async fn find_by_member(&self, account_id: &str, limit: u32) -> Result<Vec<RealmRecord>, RepoError> {
        self.having_child(MEMBERS, account_id, limit).await
    }

    /// Realms that invited the account, by name, at most `limit`.
    pub async fn find_by_player_invite(&self, account_id: &str, limit: u32) -> Result<Vec<RealmRecord>, RepoError> {
        self.having_child(PLAYER_INVITES, account_id, limit).await
    }

    /// Realms online most recently first, never-online ones last, then by name.
    pub async fn list_listed(&self, max: u32, offset: u32) -> Result<Vec<RealmRecord>, RepoError> {
        const TAIL: &str = "WHERE listed = ? ORDER BY CASE WHEN online_until IS NULL THEN 1 ELSE 0 END, \
                            online_until DESC, name LIMIT ? OFFSET ?";
        let args = binds![true, i64::from(max), i64::from(offset)];
        self.db.read(move |conn| realms_where(conn, TAIL, args)).await
    }

    /// The name is checked before the host.
    pub async fn insert_realm(&self, actor: &Actor, realm: &RealmRecord) -> Result<InsertRealmResult, RepoError> {
        let realm = realm.clone();
        self.db
            .write(actor, move |conn| {
                let name_text = "SELECT COUNT(*) AS n FROM rifts_realms WHERE name = ?";
                let host_text = "SELECT COUNT(*) AS n FROM rifts_realms WHERE host_account_id = ?";
                if any(conn, name_text, binds![realm.name])? {
                    Ok(InsertRealmResult::NameTaken)
                } else if any(conn, host_text, binds![realm.host_account_id])? {
                    Ok(InsertRealmResult::HostHasRealm)
                } else {
                    insert_realm_rows(conn, &realm).map(|()| InsertRealmResult::Inserted)
                }
            })
            .await
    }

    /// Releases a name only if the realm has not been online since `expected_last_online_at` was read.
    pub async fn delete_realm_if_last_online(
        &self,
        actor: &Actor,
        realm_id: &str,
        expected_last_online_at: Option<UnixMillis>,
    ) -> Result<bool, RepoError> {
        let realm_id = realm_id.to_owned();
        self.db
            .write(actor, move |conn| {
                let current = first_realm(conn, "WHERE realm_id = ?", binds![realm_id])?;
                let unchanged = current.is_some_and(|realm| realm.last_online_at == expected_last_online_at);
                if unchanged {
                    delete_realm(conn, &realm_id)?;
                }
                Ok(unchanged)
            })
            .await
    }

    /// Deletes the realm and its rows if `host` hosts it; returns whether it did.
    pub async fn delete_realm_of_host(&self, actor: &Actor, realm_id: &str, host: &str) -> Result<bool, RepoError> {
        let (realm_id, host) = (realm_id.to_owned(), host.to_owned());
        self.db
            .write(actor, move |conn| {
                let owned = owned_by(conn, &realm_id, &host)?;
                if owned {
                    delete_realm(conn, &realm_id)?;
                }
                Ok(owned)
            })
            .await
    }

    /// Nothing to change answers true without reading, as Node did.
    pub async fn set_settings(
        &self,
        actor: &Actor,
        realm_id: &str,
        host: &str,
        join_policy: Option<JoinPolicy>,
        listed: Option<bool>,
    ) -> Result<bool, RepoError> {
        if (join_policy, listed) == (None, None) {
            return Ok(true);
        }
        let (realm_id, host) = (realm_id.to_owned(), host.to_owned());
        self.db
            .write(actor, move |conn| {
                if !owned_by(conn, &realm_id, &host)? {
                    return Ok(false);
                }
                let updates = [
                    join_policy.map(|policy| ("join_policy", binds![policy, realm_id])),
                    listed.map(|listed| ("listed", binds![listed, realm_id])),
                ];
                for (column, args) in updates.into_iter().flatten() {
                    let text = format!("UPDATE rifts_realms SET {column} = ? WHERE realm_id = ?");
                    Statement::new(&text, args)?.run(conn)?;
                }
                Ok(true)
            })
            .await
    }

    /// An absent version keeps the stored one.
    pub async fn record_heartbeat(
        &self,
        actor: &Actor,
        realm_id: &str,
        host: &str,
        update: &HeartbeatUpdate,
    ) -> Result<bool, RepoError> {
        let args = binds![
            update.last_online_at,
            update.online_until,
            update.version,
            realm_id,
            host
        ];
        self.db
            .write(actor, move |conn| {
                let text = "UPDATE rifts_realms SET last_online_at = ?, online_until = ?, \
                            version = COALESCE(?, version) WHERE realm_id = ? AND host_account_id = ?";
                Ok(Statement::new(text, args)?.run(conn)? == 1)
            })
            .await
    }

    /// Reads the current record inside the write, so disjoint patches never undo each other.
    pub async fn patch_hosting(
        &self,
        actor: &Actor,
        realm_id: &str,
        patch: &HostingPatch,
        at: UnixMillis,
    ) -> Result<bool, RepoError> {
        let (realm_id, patch) = (realm_id.to_owned(), patch.clone());
        self.db
            .write(actor, move |conn| {
                let text = format!(
                    "SELECT {} FROM rifts_realms WHERE realm_id = ?",
                    list(RealmRow::COLUMNS)
                );
                let Some(row) = Statement::new(&text, binds![realm_id])?.first::<RealmRow>(conn)? else {
                    return Ok(false);
                };
                write_hosting(conn, &realm_id, &apply_hosting_patch(row.hosting()?, &patch, at))?;
                Ok(true)
            })
            .await
    }

    /// Ends a lasting online state now, marked as notified so the sweeper never announces it.
    pub async fn mark_offline(&self, actor: &Actor, realm_id: &str, at: UnixMillis) -> Result<bool, RepoError> {
        let args = binds![at, at, realm_id, at];
        self.db
            .write(actor, move |conn| {
                let text = "UPDATE rifts_realms SET online_until = ?, offline_notified_for = ? \
                            WHERE realm_id = ? AND online_until > ?";
                Ok(Statement::new(text, args)?.run(conn)? == 1)
            })
            .await
    }

    /// Each lapse in `(since, now]` is claimed once: the claim records the `online_until` it was for.
    pub async fn claim_lapsed_hosts(
        &self,
        actor: &Actor,
        now: UnixMillis,
        since: UnixMillis,
        max: u32,
    ) -> Result<Vec<RealmRecord>, RepoError> {
        let args = binds![since, now, i64::from(max)];
        self.db
            .write(actor, move |conn| {
                let text = "SELECT realm_id FROM rifts_realms WHERE online_until > ? AND online_until <= ? \
                            AND (offline_notified_for IS NULL OR offline_notified_for <> online_until) \
                            ORDER BY online_until, realm_id LIMIT ?";
                let lapsed: Vec<RealmIdRow> = Statement::new(text, args)?.rows(conn)?;
                let mut claimed = Vec::with_capacity(lapsed.len());
                for RealmIdRow { realm_id } in lapsed {
                    let claim = "UPDATE rifts_realms SET offline_notified_for = online_until WHERE realm_id = ?";
                    Statement::new(claim, binds![realm_id])?.run(conn)?;
                    claimed.extend(first_realm(conn, "WHERE realm_id = ?", binds![realm_id])?);
                }
                Ok(claimed)
            })
            .await
    }

    /// In the same write, a new member's player invite to that realm goes.
    pub async fn add_member(
        &self,
        actor: &Actor,
        realm_id: &str,
        member: &RealmMember,
        max_members: u32,
    ) -> Result<AddMemberResult, RepoError> {
        let (realm_id, member) = (realm_id.to_owned(), member.clone());
        self.db
            .write(actor, move |conn| {
                let (realm, account) = (realm_id.as_str(), member.account_id.as_str());
                let refusal = if !realm_exists(conn, realm)? {
                    Some(AddMemberResult::NotFound)
                } else if Child(BANS).has(conn, realm, account)? {
                    Some(AddMemberResult::Banned)
                } else if Child(MEMBERS).has(conn, realm, account)? {
                    Some(AddMemberResult::AlreadyMember)
                } else if Child(MEMBERS).at_least(conn, realm, max_members)? {
                    Some(AddMemberResult::Full)
                } else {
                    None
                };
                if let Some(refusal) = refusal {
                    return Ok(refusal);
                }
                add_member_row(conn, realm, &member)?;
                Child(PLAYER_INVITES).delete(conn, realm, account)?;
                Ok(AddMemberResult::Added)
            })
            .await
    }

    /// Never the host.
    pub async fn remove_member(&self, actor: &Actor, realm_id: &str, account_id: &str) -> Result<bool, RepoError> {
        let (realm_id, account_id) = (realm_id.to_owned(), account_id.to_owned());
        self.db
            .write(actor, move |conn| {
                Ok(delete_plain_member(conn, &realm_id, &account_id)? > 0)
            })
            .await
    }

    /// Also withdraws the account's membership (never a host's), join request and player invite.
    pub async fn add_ban(
        &self,
        actor: &Actor,
        realm_id: &str,
        ban: &RealmBan,
        max_bans: u32,
    ) -> Result<AddBanResult, RepoError> {
        let (realm_id, ban) = (realm_id.to_owned(), ban.clone());
        self.db
            .write(actor, move |conn| {
                let (realm, account) = (realm_id.as_str(), ban.account_id.as_str());
                let refusal = if !realm_exists(conn, realm)? {
                    Some(AddBanResult::NotFound)
                } else if Child(BANS).has(conn, realm, account)? {
                    Some(AddBanResult::AlreadyBanned)
                } else if Child(BANS).at_least(conn, realm, max_bans)? {
                    Some(AddBanResult::Full)
                } else {
                    None
                };
                if let Some(refusal) = refusal {
                    return Ok(refusal);
                }
                add_ban_row(conn, realm, &ban)?;
                delete_plain_member(conn, realm, account)?;
                for table in [JOIN_REQUESTS, PLAYER_INVITES] {
                    Child(table).delete(conn, realm, account)?;
                }
                Ok(AddBanResult::Banned)
            })
            .await
    }

    async fn drop_child(
        &self,
        actor: &Actor,
        table: &'static str,
        realm_id: &str,
        account_id: &str,
    ) -> Result<bool, RepoError> {
        let (realm_id, account_id) = (realm_id.to_owned(), account_id.to_owned());
        self.db
            .write(actor, move |conn| Child(table).delete(conn, &realm_id, &account_id))
            .await
    }

    /// Lifts a ban; returns whether there was one.
    pub async fn remove_ban(&self, actor: &Actor, realm_id: &str, account_id: &str) -> Result<bool, RepoError> {
        self.drop_child(actor, BANS, realm_id, account_id).await
    }

    /// Refusals in order: no realm, banned, already a member, already listed, full.
    async fn add_listed(
        &self,
        actor: &Actor,
        table: &'static str,
        realm_id: &str,
        entry: &AccountAt,
        max: u32,
    ) -> Result<AddEntryResult, RepoError> {
        let (realm_id, entry) = (realm_id.to_owned(), entry.clone());
        self.db
            .write(actor, move |conn| {
                let (realm, account) = (realm_id.as_str(), entry.account_id.as_str());
                let refusal = if !realm_exists(conn, realm)? {
                    Some(AddEntryResult::NotFound)
                } else if Child(BANS).has(conn, realm, account)? {
                    Some(AddEntryResult::Banned)
                } else if Child(MEMBERS).has(conn, realm, account)? {
                    Some(AddEntryResult::AlreadyMember)
                } else if Child(table).has(conn, realm, account)? {
                    Some(AddEntryResult::Already)
                } else if Child(table).at_least(conn, realm, max)? {
                    Some(AddEntryResult::Full)
                } else {
                    None
                };
                if let Some(refusal) = refusal {
                    return Ok(refusal);
                }
                add_entry_row(conn, table, realm, &entry)?;
                Ok(AddEntryResult::Added)
            })
            .await
    }

    /// Adds a join request, refusing past `max_requests`.
    pub async fn add_join_request(
        &self,
        actor: &Actor,
        realm_id: &str,
        request: &AccountAt,
        max_requests: u32,
    ) -> Result<AddEntryResult, RepoError> {
        self.add_listed(actor, JOIN_REQUESTS, realm_id, request, max_requests)
            .await
    }

    /// Removes a join request; returns whether there was one.
    pub async fn remove_join_request(
        &self,
        actor: &Actor,
        realm_id: &str,
        account_id: &str,
    ) -> Result<bool, RepoError> {
        self.drop_child(actor, JOIN_REQUESTS, realm_id, account_id).await
    }

    /// Adds a player invite, refusing past `max_invites`.
    pub async fn add_player_invite(
        &self,
        actor: &Actor,
        realm_id: &str,
        invite: &AccountAt,
        max_invites: u32,
    ) -> Result<AddEntryResult, RepoError> {
        self.add_listed(actor, PLAYER_INVITES, realm_id, invite, max_invites)
            .await
    }

    /// Removes a player invite; returns whether there was one.
    pub async fn remove_player_invite(
        &self,
        actor: &Actor,
        realm_id: &str,
        account_id: &str,
    ) -> Result<bool, RepoError> {
        self.drop_child(actor, PLAYER_INVITES, realm_id, account_id).await
    }

    /// Purged a week after it expires.
    pub async fn insert_invite(
        &self,
        actor: &Actor,
        invite: &RealmInviteRecord,
    ) -> Result<InsertInviteResult, RepoError> {
        let invite = invite.clone();
        self.db
            .write(actor, move |conn| {
                if invite_by_code(conn, &invite.code)?.is_some() {
                    return Ok(InsertInviteResult::Duplicate);
                }
                let columns = InviteRow::COLUMNS;
                let text = format!(
                    "INSERT INTO {INVITES} ({}, purge_at) VALUES ({})",
                    list(columns),
                    marks(columns.len() + 1)
                );
                let mut args = InviteRow::values(&invite);
                args.extend(binds![invite.expires_at.saturating_add(INVITE_PURGE_GRACE_MS)]);
                Statement::new(&text, args)?.run(conn)?;
                Ok(InsertInviteResult::Inserted)
            })
            .await
    }

    /// The invite with this code, whatever its state.
    pub async fn find_invite(&self, code: &str) -> Result<Option<RealmInviteRecord>, RepoError> {
        let code = code.to_owned();
        self.db.read(move |conn| invite_by_code(conn, &code)).await
    }

    /// Newest first.
    pub async fn list_invites(&self, realm_id: &str, max: u32) -> Result<Vec<RealmInviteRecord>, RepoError> {
        let args = binds![realm_id, i64::from(max)];
        self.db
            .read(move |conn| {
                let text = format!(
                    "SELECT {} FROM {INVITES} WHERE realm_id = ? ORDER BY created_at DESC, code DESC LIMIT ?",
                    list(InviteRow::COLUMNS)
                );
                let rows: Vec<InviteRow> = Statement::new(&text, args)?.rows(conn)?;
                rows.into_iter().map(InviteRow::record).collect()
            })
            .await
    }

    /// Not revoked, not expired, not used up.
    pub async fn count_active_invites(&self, realm_id: &str, now: UnixMillis) -> Result<i64, RepoError> {
        let args = binds![realm_id, now];
        self.db
            .read(move |conn| {
                let text = "SELECT COUNT(*) AS n FROM rifts_realm_invites WHERE realm_id = ? AND revoked_at IS NULL \
                            AND expires_at > ? AND uses < max_uses";
                Statement::new(text, args)?.number(conn)
            })
            .await
    }

    /// One use, only while usable at `now`: two joins never share the last one.
    pub async fn consume_invite(
        &self,
        actor: &Actor,
        code: &str,
        now: UnixMillis,
    ) -> Result<Option<RealmInviteRecord>, RepoError> {
        let code = code.to_owned();
        self.db
            .write(actor, move |conn| {
                let text = "UPDATE rifts_realm_invites SET uses = uses + 1 WHERE code = ? AND revoked_at IS NULL \
                            AND expires_at > ? AND uses < max_uses";
                match Statement::new(text, binds![code, now])?.run(conn)? {
                    0 => Ok(None),
                    _ => invite_by_code(conn, &code),
                }
            })
            .await
    }

    /// A join that consumed a use failed afterwards; never below zero.
    pub async fn refund_invite(&self, actor: &Actor, code: &str) -> Result<(), RepoError> {
        let args = binds![code];
        self.db
            .write(actor, move |conn| {
                let text = "UPDATE rifts_realm_invites SET uses = uses - 1 WHERE code = ? AND uses > 0";
                Statement::new(text, args)?.run(conn).map(drop)
            })
            .await
    }

    /// The first revocation time stays.
    pub async fn revoke_invite(
        &self,
        actor: &Actor,
        realm_id: &str,
        code: &str,
        at: UnixMillis,
    ) -> Result<Option<RealmInviteRecord>, RepoError> {
        let (realm_id, code) = (realm_id.to_owned(), code.to_owned());
        self.db
            .write(actor, move |conn| {
                let Some(mut invite) = invite_by_code(conn, &code)?.filter(|found| found.realm_id == realm_id) else {
                    return Ok(None);
                };
                if invite.revoked_at.is_none() {
                    let text = "UPDATE rifts_realm_invites SET revoked_at = ? WHERE code = ?";
                    Statement::new(text, binds![at, code])?.run(conn)?;
                    invite.revoked_at = Some(at);
                }
                Ok(Some(invite))
            })
            .await
    }

    /// Nothing calls this yet (spec-rd27-rifts §7 risk 3).
    pub async fn purge_expired(&self, now: UnixMillis) -> Result<usize, RepoError> {
        self.db
            .write(&Actor::System("realms-purge"), move |conn| {
                let mut purged = 0;
                for (table, column) in [(INVITES, "purge_at"), (PRESENCE, "expires_at")] {
                    let text = format!("DELETE FROM {table} WHERE {column} <= ?");
                    purged += Statement::new(&text, binds![now])?.run(conn)?;
                }
                Ok(purged)
            })
            .await
    }
}

fn active_presence(
    conn: &mut AnyConnection,
    account_id: &str,
    now: UnixMillis,
) -> Result<Option<RealmPresenceRecord>, RepoError> {
    let text = format!(
        "SELECT {} FROM {PRESENCE} WHERE account_id = ? AND expires_at > ?",
        list(PresenceRow::COLUMNS)
    );
    Statement::new(&text, binds![account_id, now])?
        .first::<PresenceRow>(conn)?
        .map(PresenceRow::record)
        .transpose()
}

/// One row per account: which realm its current game login belongs to.
#[derive(Clone)]
pub struct RealmPresenceRepository {
    db: RealmsDb,
}

impl RealmPresenceRepository {
    /// The realm presence table in `db`.
    pub fn new(db: RealmsDb) -> Self {
        Self { db }
    }

    /// Replaces the account's row.
    pub async fn bind(&self, actor: &Actor, row: &RealmPresenceRecord) -> Result<(), RepoError> {
        let args = PresenceRow::values(row);
        self.db
            .write(actor, move |conn| {
                upsert(conn, PRESENCE, (PresenceRow::COLUMNS, &["account_id"], &[]), args)
            })
            .await
    }

    /// Strictly before expiry.
    pub async fn find_active(
        &self,
        account_id: &str,
        now: UnixMillis,
    ) -> Result<Option<RealmPresenceRecord>, RepoError> {
        let account_id = account_id.to_owned();
        self.db.read(move |conn| active_presence(conn, &account_id, now)).await
    }

    /// Extends a row still active; an expired or absent one is never brought back.
    pub async fn touch(
        &self,
        actor: &Actor,
        account_id: &str,
        now: UnixMillis,
        expires_at: UnixMillis,
    ) -> Result<Option<RealmPresenceRecord>, RepoError> {
        let account_id = account_id.to_owned();
        self.db
            .write(actor, move |conn| {
                let text = "UPDATE rifts_realm_presence SET last_seen_at = ?, expires_at = ? \
                            WHERE account_id = ? AND expires_at > ?";
                match Statement::new(text, binds![now, expires_at, account_id, now])?.run(conn)? {
                    0 => Ok(None),
                    _ => active_presence(conn, &account_id, now),
                }
            })
            .await
    }

    /// With a realm, only a row bound to it.
    pub async fn remove(&self, actor: &Actor, account_id: &str, realm_id: Option<&str>) -> Result<bool, RepoError> {
        let (text, args) = match realm_id {
            None => (
                "DELETE FROM rifts_realm_presence WHERE account_id = ?",
                binds![account_id],
            ),
            Some(realm) => (
                "DELETE FROM rifts_realm_presence WHERE account_id = ? AND realm_id = ?",
                binds![account_id, realm],
            ),
        };
        self.db
            .write(actor, move |conn| Ok(Statement::new(text, args)?.run(conn)? > 0))
            .await
    }

    /// Removes every presence row in the realm; returns how many it removed.
    pub async fn remove_for_realm(&self, actor: &Actor, realm_id: &str) -> Result<usize, RepoError> {
        let args = binds![realm_id];
        self.db
            .write(actor, move |conn| {
                Statement::new("DELETE FROM rifts_realm_presence WHERE realm_id = ?", args)?.run(conn)
            })
            .await
    }

    /// Realms with no active row are absent.
    pub async fn count_active_by_realm(
        &self,
        realm_ids: &[String],
        now: UnixMillis,
    ) -> Result<BTreeMap<String, i64>, RepoError> {
        if realm_ids.is_empty() {
            return Ok(BTreeMap::new());
        }
        let text = format!(
            "SELECT realm_id, COUNT(*) AS n FROM rifts_realm_presence WHERE expires_at > ? \
             AND realm_id IN ({}) GROUP BY realm_id",
            marks(realm_ids.len())
        );
        let mut args = binds![now];
        args.extend(realm_ids.iter().map(Bind::arg));
        self.db
            .read(move |conn| {
                let counts: Vec<RealmCountRow> = Statement::new(&text, args)?.rows(conn)?;
                Ok(counts.into_iter().map(|row| (row.realm_id, row.n)).collect())
            })
            .await
    }
}
