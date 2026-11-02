// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Realms with their members, bans, join requests, player invites and hub rooms; invite codes; and which realm an
//! account's game login belongs to.

use std::collections::BTreeMap;

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Bool, Nullable, Text};
use rifts_seam::{Actor, UnixMillis};
use store::AnyConnection;

use super::{Boxed, Count, corrupt, limit, placeholders, query};
use crate::db::{RealmsDb, RepoError};
use crate::realm::{
    AccountAt, HeartbeatUpdate, HostingPatch, HubState, INVITE_PURGE_GRACE_MS, JoinPolicy, MemberRole, RealmBan,
    RealmHosting, RealmInviteRecord, RealmMember, RealmPresenceRecord, RealmRecord, apply_hosting_patch,
};
use crate::time::Later as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertRealmResult {
    Inserted,
    NameTaken,
    HostHasRealm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddMemberResult {
    Added,
    AlreadyMember,
    Banned,
    Full,
    NotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddBanResult {
    Banned,
    AlreadyBanned,
    Full,
    NotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddEntryResult {
    Added,
    /// Already requested, or already invited.
    Already,
    Banned,
    AlreadyMember,
    Full,
    NotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertInviteResult {
    Inserted,
    Duplicate,
}

const REALM_COLUMNS: &str = "realm_id, name, host_account_id, host_public_key, join_policy, listed, created_at, \
     last_online_at, online_until, version, offline_notified_for, hosting_max_servers, hosting_max_players, \
     hosting_afk_seconds, hosting_hub_session_id, hosting_hub_state, hosting_training_state, \
     hosting_active_servers, hosting_updated_at";

#[derive(QueryableByName)]
struct RealmRow {
    #[diesel(sql_type = Text)]
    realm_id: String,
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = Text)]
    host_account_id: String,
    #[diesel(sql_type = Text)]
    host_public_key: String,
    #[diesel(sql_type = Text)]
    join_policy: String,
    #[diesel(sql_type = Bool)]
    listed: bool,
    #[diesel(sql_type = BigInt)]
    created_at: i64,
    #[diesel(sql_type = Nullable<BigInt>)]
    last_online_at: Option<i64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    online_until: Option<i64>,
    #[diesel(sql_type = Nullable<Text>)]
    version: Option<String>,
    #[diesel(sql_type = Nullable<BigInt>)]
    offline_notified_for: Option<i64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    hosting_max_servers: Option<i64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    hosting_max_players: Option<i64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    hosting_afk_seconds: Option<i64>,
    #[diesel(sql_type = Nullable<Text>)]
    hosting_hub_session_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    hosting_hub_state: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    hosting_training_state: Option<String>,
    #[diesel(sql_type = Nullable<BigInt>)]
    hosting_active_servers: Option<i64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    hosting_updated_at: Option<i64>,
}

fn hub_state(text: Option<&str>) -> Result<HubState, RepoError> {
    match text {
        None => Ok(HubState::None),
        Some(text) => HubState::parse(text).ok_or_else(|| corrupt("hub state", text)),
    }
}

impl RealmRow {
    /// Absent until written; a `NULL` column of a written record reads as its default.
    fn hosting(&self) -> Result<Option<RealmHosting>, RepoError> {
        let Some(updated_at) = self.hosting_updated_at else {
            return Ok(None);
        };
        Ok(Some(RealmHosting {
            max_servers: self.hosting_max_servers.unwrap_or(1),
            max_players: self.hosting_max_players.unwrap_or(16),
            afk_seconds: self.hosting_afk_seconds,
            hub_session_id: self.hosting_hub_session_id.clone(),
            hub_state: hub_state(self.hosting_hub_state.as_deref())?,
            training_state: hub_state(self.hosting_training_state.as_deref())?,
            active_servers: self.hosting_active_servers.unwrap_or(0),
            updated_at: UnixMillis(updated_at),
        }))
    }
}

#[derive(QueryableByName)]
struct MemberRow {
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = Text)]
    role: String,
    #[diesel(sql_type = BigInt)]
    joined_at: i64,
}

#[derive(QueryableByName)]
struct BanRow {
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = Nullable<Text>)]
    reason: Option<String>,
    #[diesel(sql_type = BigInt)]
    at: i64,
}

#[derive(QueryableByName)]
struct AccountAtRow {
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = BigInt)]
    at: i64,
}

#[derive(QueryableByName)]
struct RoomRow {
    #[diesel(sql_type = Text)]
    room_id: String,
}

#[derive(QueryableByName)]
struct IdRow {
    #[diesel(sql_type = Text)]
    realm_id: String,
}

fn children<R: QueryableByName<store::MultiBackend> + 'static>(
    conn: &mut AnyConnection,
    columns: &str,
    table: &str,
    realm_id: &str,
) -> Result<Vec<R>, RepoError> {
    Ok(query(&format!(
        "SELECT {columns} FROM {table} WHERE realm_id = ? ORDER BY seq"
    ))?
    .bind::<Text, _>(realm_id.to_owned())
    .load::<R>(conn)?)
}

fn assemble(conn: &mut AnyConnection, row: RealmRow) -> Result<RealmRecord, RepoError> {
    let id = row.realm_id.clone();
    let members = children::<MemberRow>(conn, "account_id, role, joined_at", "rifts_realm_members", &id)?
        .into_iter()
        .map(|m| {
            Ok(RealmMember {
                role: MemberRole::parse(&m.role).ok_or_else(|| corrupt("member role", &m.role))?,
                account_id: m.account_id,
                joined_at: UnixMillis(m.joined_at),
            })
        })
        .collect::<Result<Vec<_>, RepoError>>()?;
    let bans = children::<BanRow>(conn, "account_id, reason, at", "rifts_realm_bans", &id)?
        .into_iter()
        .map(|b| RealmBan {
            account_id: b.account_id,
            reason: b.reason,
            at: UnixMillis(b.at),
        })
        .collect();
    let account_at = |rows: Vec<AccountAtRow>| {
        rows.into_iter()
            .map(|r| AccountAt {
                account_id: r.account_id,
                at: UnixMillis(r.at),
            })
            .collect::<Vec<_>>()
    };
    let join_requests = account_at(children(conn, "account_id, at", "rifts_realm_join_requests", &id)?);
    let player_invites = account_at(children(conn, "account_id, at", "rifts_realm_player_invites", &id)?);
    let hub_room_ids = children::<RoomRow>(conn, "room_id", "rifts_realm_hub_rooms", &id)?
        .into_iter()
        .map(|r| r.room_id)
        .collect();
    let hosting = row.hosting()?;
    Ok(RealmRecord {
        join_policy: JoinPolicy::parse(&row.join_policy).ok_or_else(|| corrupt("join policy", &row.join_policy))?,
        realm_id: row.realm_id,
        name: row.name,
        host_account_id: row.host_account_id,
        host_public_key: row.host_public_key,
        listed: row.listed,
        members,
        bans,
        created_at: UnixMillis(row.created_at),
        last_online_at: row.last_online_at.map(UnixMillis),
        online_until: row.online_until.map(UnixMillis),
        version: row.version,
        hosting,
        hub_room_ids,
        offline_notified_for: row.offline_notified_for.map(UnixMillis),
        join_requests,
        player_invites,
    })
}

/// Realms matching `tail` (a `WHERE`/`ORDER BY`/`LIMIT` clause) with their children.
fn select_realms(
    conn: &mut AnyConnection,
    tail: &str,
    bind: impl FnOnce(Boxed) -> Boxed,
) -> Result<Vec<RealmRecord>, RepoError> {
    let rows = bind(query(&format!("SELECT {REALM_COLUMNS} FROM rifts_realms {tail}"))?).load::<RealmRow>(conn)?;
    rows.into_iter().map(|row| assemble(conn, row)).collect()
}

fn realm_by_id(conn: &mut AnyConnection, realm_id: &str) -> Result<Option<RealmRecord>, RepoError> {
    let id = realm_id.to_owned();
    Ok(select_realms(conn, "WHERE realm_id = ?", |q| q.bind::<Text, _>(id))?
        .into_iter()
        .next())
}

fn count(conn: &mut AnyConnection, text: &str, bind: impl FnOnce(Boxed) -> Boxed) -> Result<i64, RepoError> {
    Ok(bind(query(text)?)
        .load::<Count>(conn)?
        .into_iter()
        .next()
        .map_or(0, |c| c.n))
}

fn realm_exists(conn: &mut AnyConnection, realm_id: &str) -> Result<bool, RepoError> {
    let id = realm_id.to_owned();
    Ok(
        count(conn, "SELECT COUNT(*) AS n FROM rifts_realms WHERE realm_id = ?", |q| {
            q.bind::<Text, _>(id)
        })? > 0,
    )
}

fn has_child(conn: &mut AnyConnection, table: &str, realm_id: &str, account_id: &str) -> Result<bool, RepoError> {
    let (realm, account) = (realm_id.to_owned(), account_id.to_owned());
    Ok(count(
        conn,
        &format!("SELECT COUNT(*) AS n FROM {table} WHERE realm_id = ? AND account_id = ?"),
        |q| q.bind::<Text, _>(realm).bind::<Text, _>(account),
    )? > 0)
}

fn child_count(conn: &mut AnyConnection, table: &str, realm_id: &str) -> Result<i64, RepoError> {
    let realm = realm_id.to_owned();
    count(
        conn,
        &format!("SELECT COUNT(*) AS n FROM {table} WHERE realm_id = ?"),
        |q| q.bind::<Text, _>(realm),
    )
}

fn next_seq(conn: &mut AnyConnection, table: &str, realm_id: &str) -> Result<i64, RepoError> {
    let realm = realm_id.to_owned();
    count(
        conn,
        &format!("SELECT COALESCE(MAX(seq), 0) + 1 AS n FROM {table} WHERE realm_id = ?"),
        |q| q.bind::<Text, _>(realm),
    )
}

fn delete_child(conn: &mut AnyConnection, table: &str, realm_id: &str, account_id: &str) -> Result<bool, RepoError> {
    Ok(
        query(&format!("DELETE FROM {table} WHERE realm_id = ? AND account_id = ?"))?
            .bind::<Text, _>(realm_id.to_owned())
            .bind::<Text, _>(account_id.to_owned())
            .execute(conn)?
            > 0,
    )
}

fn insert_member(conn: &mut AnyConnection, realm_id: &str, member: &RealmMember) -> Result<(), RepoError> {
    let seq = next_seq(conn, "rifts_realm_members", realm_id)?;
    query("INSERT INTO rifts_realm_members (realm_id, account_id, role, joined_at, seq) VALUES (?, ?, ?, ?, ?)")?
        .bind::<Text, _>(realm_id.to_owned())
        .bind::<Text, _>(member.account_id.clone())
        .bind::<Text, _>(member.role.as_str())
        .bind::<BigInt, _>(member.joined_at.0)
        .bind::<BigInt, _>(seq)
        .execute(conn)?;
    Ok(())
}

fn insert_ban(conn: &mut AnyConnection, realm_id: &str, ban: &RealmBan) -> Result<(), RepoError> {
    let seq = next_seq(conn, "rifts_realm_bans", realm_id)?;
    query("INSERT INTO rifts_realm_bans (realm_id, account_id, reason, at, seq) VALUES (?, ?, ?, ?, ?)")?
        .bind::<Text, _>(realm_id.to_owned())
        .bind::<Text, _>(ban.account_id.clone())
        .bind::<Nullable<Text>, _>(ban.reason.clone())
        .bind::<BigInt, _>(ban.at.0)
        .bind::<BigInt, _>(seq)
        .execute(conn)?;
    Ok(())
}

fn insert_account_at(
    conn: &mut AnyConnection,
    table: &str,
    realm_id: &str,
    entry: &AccountAt,
) -> Result<(), RepoError> {
    let seq = next_seq(conn, table, realm_id)?;
    query(&format!(
        "INSERT INTO {table} (realm_id, account_id, at, seq) VALUES (?, ?, ?, ?)"
    ))?
    .bind::<Text, _>(realm_id.to_owned())
    .bind::<Text, _>(entry.account_id.clone())
    .bind::<BigInt, _>(entry.at.0)
    .bind::<BigInt, _>(seq)
    .execute(conn)?;
    Ok(())
}

fn write_hosting(conn: &mut AnyConnection, realm_id: &str, hosting: &RealmHosting) -> Result<(), RepoError> {
    query(
        "UPDATE rifts_realms SET hosting_max_servers = ?, hosting_max_players = ?, hosting_afk_seconds = ?, \
         hosting_hub_session_id = ?, hosting_hub_state = ?, hosting_training_state = ?, \
         hosting_active_servers = ?, hosting_updated_at = ? WHERE realm_id = ?",
    )?
    .bind::<Nullable<BigInt>, _>(Some(hosting.max_servers))
    .bind::<Nullable<BigInt>, _>(Some(hosting.max_players))
    .bind::<Nullable<BigInt>, _>(hosting.afk_seconds)
    .bind::<Nullable<Text>, _>(hosting.hub_session_id.clone())
    .bind::<Nullable<Text>, _>(Some(hosting.hub_state.as_str()))
    .bind::<Nullable<Text>, _>(Some(hosting.training_state.as_str()))
    .bind::<Nullable<BigInt>, _>(Some(hosting.active_servers))
    .bind::<Nullable<BigInt>, _>((Some(hosting.updated_at)).map(|at| at.0))
    .bind::<Text, _>(realm_id.to_owned())
    .execute(conn)?;
    Ok(())
}

/// Children first, for the foreign keys; invites go with their realm, presence rows do not.
fn delete_realm(conn: &mut AnyConnection, realm_id: &str) -> Result<(), RepoError> {
    for table in [
        "rifts_realm_members",
        "rifts_realm_bans",
        "rifts_realm_join_requests",
        "rifts_realm_player_invites",
        "rifts_realm_hub_rooms",
        "rifts_realm_invites",
        "rifts_realms",
    ] {
        query(&format!("DELETE FROM {table} WHERE realm_id = ?"))?
            .bind::<Text, _>(realm_id.to_owned())
            .execute(conn)?;
    }
    Ok(())
}

fn insert_realm_rows(conn: &mut AnyConnection, realm: &RealmRecord) -> Result<(), RepoError> {
    query(
        "INSERT INTO rifts_realms (realm_id, name, host_account_id, host_public_key, join_policy, listed, \
         created_at, last_online_at, online_until, version, offline_notified_for) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )?
    .bind::<Text, _>(realm.realm_id.clone())
    .bind::<Text, _>(realm.name.clone())
    .bind::<Text, _>(realm.host_account_id.clone())
    .bind::<Text, _>(realm.host_public_key.clone())
    .bind::<Text, _>(realm.join_policy.as_str())
    .bind::<Bool, _>(realm.listed)
    .bind::<BigInt, _>(realm.created_at.0)
    .bind::<Nullable<BigInt>, _>(realm.last_online_at.map(|at| at.0))
    .bind::<Nullable<BigInt>, _>(realm.online_until.map(|at| at.0))
    .bind::<Nullable<Text>, _>(realm.version.clone())
    .bind::<Nullable<BigInt>, _>(realm.offline_notified_for.map(|at| at.0))
    .execute(conn)?;
    if let Some(hosting) = &realm.hosting {
        write_hosting(conn, &realm.realm_id, hosting)?;
    }
    for member in &realm.members {
        insert_member(conn, &realm.realm_id, member)?;
    }
    for ban in &realm.bans {
        insert_ban(conn, &realm.realm_id, ban)?;
    }
    for request in &realm.join_requests {
        insert_account_at(conn, "rifts_realm_join_requests", &realm.realm_id, request)?;
    }
    for invite in &realm.player_invites {
        insert_account_at(conn, "rifts_realm_player_invites", &realm.realm_id, invite)?;
    }
    for (seq, room) in (1i64..).zip(&realm.hub_room_ids) {
        query("INSERT INTO rifts_realm_hub_rooms (room_id, realm_id, seq) VALUES (?, ?, ?)")?
            .bind::<Text, _>(room.clone())
            .bind::<Text, _>(realm.realm_id.clone())
            .bind::<BigInt, _>(seq)
            .execute(conn)?;
    }
    Ok(())
}

const INVITE_COLUMNS: &str = "code, realm_id, created_by, created_at, expires_at, max_uses, uses, revoked_at";

#[derive(QueryableByName)]
struct InviteRow {
    #[diesel(sql_type = Text)]
    code: String,
    #[diesel(sql_type = Text)]
    realm_id: String,
    #[diesel(sql_type = Text)]
    created_by: String,
    #[diesel(sql_type = BigInt)]
    created_at: i64,
    #[diesel(sql_type = BigInt)]
    expires_at: i64,
    #[diesel(sql_type = BigInt)]
    max_uses: i64,
    #[diesel(sql_type = BigInt)]
    uses: i64,
    #[diesel(sql_type = Nullable<BigInt>)]
    revoked_at: Option<i64>,
}

impl From<InviteRow> for RealmInviteRecord {
    fn from(row: InviteRow) -> Self {
        Self {
            code: row.code,
            realm_id: row.realm_id,
            created_by: row.created_by,
            created_at: UnixMillis(row.created_at),
            expires_at: UnixMillis(row.expires_at),
            max_uses: row.max_uses,
            uses: row.uses,
            revoked_at: row.revoked_at.map(UnixMillis),
        }
    }
}

fn invite_by_code(conn: &mut AnyConnection, code: &str) -> Result<Option<RealmInviteRecord>, RepoError> {
    Ok(query(&format!(
        "SELECT {INVITE_COLUMNS} FROM rifts_realm_invites WHERE code = ?"
    ))?
    .bind::<Text, _>(code.to_owned())
    .load::<InviteRow>(conn)?
    .into_iter()
    .next()
    .map(RealmInviteRecord::from))
}

/// One realm's membership, bans, requests and invites, and its invite codes.
#[derive(Clone)]
pub struct RealmsRepository {
    db: RealmsDb,
}

impl RealmsRepository {
    pub fn new(db: RealmsDb) -> Self {
        Self { db }
    }

    async fn one(&self, tail: &'static str, value: &str) -> Result<Option<RealmRecord>, RepoError> {
        let value = value.to_owned();
        self.db
            .read(move |conn| {
                Ok(select_realms(conn, tail, |q| q.bind::<Text, _>(value))?
                    .into_iter()
                    .next())
            })
            .await
    }

    pub async fn find_by_id(&self, realm_id: &str) -> Result<Option<RealmRecord>, RepoError> {
        self.one("WHERE realm_id = ?", realm_id).await
    }

    pub async fn find_by_name(&self, name: &str) -> Result<Option<RealmRecord>, RepoError> {
        self.one("WHERE name = ?", name).await
    }

    pub async fn find_by_host(&self, host: &str) -> Result<Option<RealmRecord>, RepoError> {
        self.one("WHERE host_account_id = ?", host).await
    }

    pub async fn find_by_hub_room_id(&self, room_id: &str) -> Result<Option<RealmRecord>, RepoError> {
        self.one(
            "WHERE realm_id IN (SELECT realm_id FROM rifts_realm_hub_rooms WHERE room_id = ?)",
            room_id,
        )
        .await
    }

    async fn by_child(&self, table: &'static str, account_id: &str, max: u32) -> Result<Vec<RealmRecord>, RepoError> {
        let account = account_id.to_owned();
        self.db
            .read(move |conn| {
                select_realms(
                    conn,
                    &format!(
                        "WHERE realm_id IN (SELECT realm_id FROM {table} WHERE account_id = ?) ORDER BY name LIMIT ?"
                    ),
                    |q| q.bind::<Text, _>(account).bind::<BigInt, _>(limit(max)),
                )
            })
            .await
    }

    /// The host's own realm included.
    pub async fn find_by_member(&self, account_id: &str, limit: u32) -> Result<Vec<RealmRecord>, RepoError> {
        self.by_child("rifts_realm_members", account_id, limit).await
    }

    pub async fn find_by_player_invite(&self, account_id: &str, limit: u32) -> Result<Vec<RealmRecord>, RepoError> {
        self.by_child("rifts_realm_player_invites", account_id, limit).await
    }

    /// Realms online most recently first, never-online ones last, then by name.
    pub async fn list_listed(&self, max: u32, offset: u32) -> Result<Vec<RealmRecord>, RepoError> {
        self.db
            .read(move |conn| {
                select_realms(
                    conn,
                    "WHERE listed = ? ORDER BY CASE WHEN online_until IS NULL THEN 1 ELSE 0 END, \
                     online_until DESC, name LIMIT ? OFFSET ?",
                    |q| {
                        q.bind::<Bool, _>(true)
                            .bind::<BigInt, _>(limit(max))
                            .bind::<BigInt, _>(limit(offset))
                    },
                )
            })
            .await
    }

    /// The name is checked before the host.
    pub async fn insert_realm(&self, actor: &Actor, realm: &RealmRecord) -> Result<InsertRealmResult, RepoError> {
        let realm = realm.clone();
        self.db
            .write(actor, move |conn| {
                let (name, host) = (realm.name.clone(), realm.host_account_id.clone());
                if count(conn, "SELECT COUNT(*) AS n FROM rifts_realms WHERE name = ?", |q| {
                    q.bind::<Text, _>(name)
                })? > 0
                {
                    return Ok(InsertRealmResult::NameTaken);
                }
                if count(
                    conn,
                    "SELECT COUNT(*) AS n FROM rifts_realms WHERE host_account_id = ?",
                    |q| q.bind::<Text, _>(host),
                )? > 0
                {
                    return Ok(InsertRealmResult::HostHasRealm);
                }
                insert_realm_rows(conn, &realm)?;
                Ok(InsertRealmResult::Inserted)
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
            .write(actor, move |conn| match realm_by_id(conn, &realm_id)? {
                Some(realm) if realm.last_online_at == expected_last_online_at => {
                    delete_realm(conn, &realm_id)?;
                    Ok(true)
                }
                _ => Ok(false),
            })
            .await
    }

    pub async fn delete_realm_of_host(&self, actor: &Actor, realm_id: &str, host: &str) -> Result<bool, RepoError> {
        let (realm_id, host) = (realm_id.to_owned(), host.to_owned());
        self.db
            .write(actor, move |conn| {
                let (id, owner) = (realm_id.clone(), host);
                let owned = count(
                    conn,
                    "SELECT COUNT(*) AS n FROM rifts_realms WHERE realm_id = ? AND host_account_id = ?",
                    |q| q.bind::<Text, _>(id).bind::<Text, _>(owner),
                )? > 0;
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
        if join_policy.is_none() && listed.is_none() {
            return Ok(true);
        }
        let (realm_id, host) = (realm_id.to_owned(), host.to_owned());
        self.db
            .write(actor, move |conn| {
                let (id, owner) = (realm_id.clone(), host);
                let owned = count(
                    conn,
                    "SELECT COUNT(*) AS n FROM rifts_realms WHERE realm_id = ? AND host_account_id = ?",
                    |q| q.bind::<Text, _>(id).bind::<Text, _>(owner),
                )? > 0;
                if !owned {
                    return Ok(false);
                }
                if let Some(policy) = join_policy {
                    query("UPDATE rifts_realms SET join_policy = ? WHERE realm_id = ?")?
                        .bind::<Text, _>(policy.as_str())
                        .bind::<Text, _>(realm_id.clone())
                        .execute(conn)?;
                }
                if let Some(listed) = listed {
                    query("UPDATE rifts_realms SET listed = ? WHERE realm_id = ?")?
                        .bind::<Bool, _>(listed)
                        .bind::<Text, _>(realm_id.clone())
                        .execute(conn)?;
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
        let (realm_id, host, update) = (realm_id.to_owned(), host.to_owned(), update.clone());
        self.db
            .write(actor, move |conn| {
                let changed = query(
                    "UPDATE rifts_realms SET last_online_at = ?, online_until = ?, version = COALESCE(?, version) \
                     WHERE realm_id = ? AND host_account_id = ?",
                )?
                .bind::<Nullable<BigInt>, _>((Some(update.last_online_at)).map(|at| at.0))
                .bind::<Nullable<BigInt>, _>((Some(update.online_until)).map(|at| at.0))
                .bind::<Nullable<Text>, _>(update.version)
                .bind::<Text, _>(realm_id)
                .bind::<Text, _>(host)
                .execute(conn)?;
                Ok(changed == 1)
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
                let id = realm_id.clone();
                let Some(row) = query(&format!("SELECT {REALM_COLUMNS} FROM rifts_realms WHERE realm_id = ?"))?
                    .bind::<Text, _>(id)
                    .load::<RealmRow>(conn)?
                    .into_iter()
                    .next()
                else {
                    return Ok(false);
                };
                let hosting = apply_hosting_patch(row.hosting()?, &patch, at);
                write_hosting(conn, &realm_id, &hosting)?;
                Ok(true)
            })
            .await
    }

    /// Ends a lasting online state now, marked as notified so the sweeper never announces it.
    pub async fn mark_offline(&self, actor: &Actor, realm_id: &str, at: UnixMillis) -> Result<bool, RepoError> {
        let realm_id = realm_id.to_owned();
        self.db
            .write(actor, move |conn| {
                let changed = query(
                    "UPDATE rifts_realms SET online_until = ?, offline_notified_for = ? \
                     WHERE realm_id = ? AND online_until > ?",
                )?
                .bind::<Nullable<BigInt>, _>((Some(at)).map(|at| at.0))
                .bind::<Nullable<BigInt>, _>((Some(at)).map(|at| at.0))
                .bind::<Text, _>(realm_id)
                .bind::<BigInt, _>(at.0)
                .execute(conn)?;
                Ok(changed == 1)
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
        self.db
            .write(actor, move |conn| {
                let ids = query(
                    "SELECT realm_id FROM rifts_realms WHERE online_until > ? AND online_until <= ? \
                     AND (offline_notified_for IS NULL OR offline_notified_for <> online_until) \
                     ORDER BY online_until, realm_id LIMIT ?",
                )?
                .bind::<BigInt, _>(since.0)
                .bind::<BigInt, _>(now.0)
                .bind::<BigInt, _>(limit(max))
                .load::<IdRow>(conn)?;
                let mut claimed = Vec::with_capacity(ids.len());
                for IdRow { realm_id } in ids {
                    query("UPDATE rifts_realms SET offline_notified_for = online_until WHERE realm_id = ?")?
                        .bind::<Text, _>(realm_id.clone())
                        .execute(conn)?;
                    claimed.extend(realm_by_id(conn, &realm_id)?);
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
                if !realm_exists(conn, &realm_id)? {
                    return Ok(AddMemberResult::NotFound);
                }
                if has_child(conn, "rifts_realm_bans", &realm_id, &member.account_id)? {
                    return Ok(AddMemberResult::Banned);
                }
                if has_child(conn, "rifts_realm_members", &realm_id, &member.account_id)? {
                    return Ok(AddMemberResult::AlreadyMember);
                }
                if child_count(conn, "rifts_realm_members", &realm_id)? >= i64::from(max_members) {
                    return Ok(AddMemberResult::Full);
                }
                insert_member(conn, &realm_id, &member)?;
                delete_child(conn, "rifts_realm_player_invites", &realm_id, &member.account_id)?;
                Ok(AddMemberResult::Added)
            })
            .await
    }

    /// Never the host.
    pub async fn remove_member(&self, actor: &Actor, realm_id: &str, account_id: &str) -> Result<bool, RepoError> {
        let (realm_id, account_id) = (realm_id.to_owned(), account_id.to_owned());
        self.db
            .write(actor, move |conn| {
                Ok(
                    query("DELETE FROM rifts_realm_members WHERE realm_id = ? AND account_id = ? AND role = ?")?
                        .bind::<Text, _>(realm_id)
                        .bind::<Text, _>(account_id)
                        .bind::<Text, _>(MemberRole::Member.as_str())
                        .execute(conn)?
                        > 0,
                )
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
                if !realm_exists(conn, &realm_id)? {
                    return Ok(AddBanResult::NotFound);
                }
                if has_child(conn, "rifts_realm_bans", &realm_id, &ban.account_id)? {
                    return Ok(AddBanResult::AlreadyBanned);
                }
                if child_count(conn, "rifts_realm_bans", &realm_id)? >= i64::from(max_bans) {
                    return Ok(AddBanResult::Full);
                }
                insert_ban(conn, &realm_id, &ban)?;
                query("DELETE FROM rifts_realm_members WHERE realm_id = ? AND account_id = ? AND role = ?")?
                    .bind::<Text, _>(realm_id.clone())
                    .bind::<Text, _>(ban.account_id.clone())
                    .bind::<Text, _>(MemberRole::Member.as_str())
                    .execute(conn)?;
                delete_child(conn, "rifts_realm_join_requests", &realm_id, &ban.account_id)?;
                delete_child(conn, "rifts_realm_player_invites", &realm_id, &ban.account_id)?;
                Ok(AddBanResult::Banned)
            })
            .await
    }

    async fn remove_child(
        &self,
        actor: &Actor,
        table: &'static str,
        realm_id: &str,
        account_id: &str,
    ) -> Result<bool, RepoError> {
        let (realm_id, account_id) = (realm_id.to_owned(), account_id.to_owned());
        self.db
            .write(actor, move |conn| delete_child(conn, table, &realm_id, &account_id))
            .await
    }

    pub async fn remove_ban(&self, actor: &Actor, realm_id: &str, account_id: &str) -> Result<bool, RepoError> {
        self.remove_child(actor, "rifts_realm_bans", realm_id, account_id).await
    }

    /// Refusals in order: no realm, banned, already a member, already listed, full.
    async fn add_entry(
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
                if !realm_exists(conn, &realm_id)? {
                    return Ok(AddEntryResult::NotFound);
                }
                if has_child(conn, "rifts_realm_bans", &realm_id, &entry.account_id)? {
                    return Ok(AddEntryResult::Banned);
                }
                if has_child(conn, "rifts_realm_members", &realm_id, &entry.account_id)? {
                    return Ok(AddEntryResult::AlreadyMember);
                }
                if has_child(conn, table, &realm_id, &entry.account_id)? {
                    return Ok(AddEntryResult::Already);
                }
                if child_count(conn, table, &realm_id)? >= i64::from(max) {
                    return Ok(AddEntryResult::Full);
                }
                insert_account_at(conn, table, &realm_id, &entry)?;
                Ok(AddEntryResult::Added)
            })
            .await
    }

    pub async fn add_join_request(
        &self,
        actor: &Actor,
        realm_id: &str,
        request: &AccountAt,
        max_requests: u32,
    ) -> Result<AddEntryResult, RepoError> {
        self.add_entry(actor, "rifts_realm_join_requests", realm_id, request, max_requests)
            .await
    }

    pub async fn remove_join_request(
        &self,
        actor: &Actor,
        realm_id: &str,
        account_id: &str,
    ) -> Result<bool, RepoError> {
        self.remove_child(actor, "rifts_realm_join_requests", realm_id, account_id)
            .await
    }

    pub async fn add_player_invite(
        &self,
        actor: &Actor,
        realm_id: &str,
        invite: &AccountAt,
        max_invites: u32,
    ) -> Result<AddEntryResult, RepoError> {
        self.add_entry(actor, "rifts_realm_player_invites", realm_id, invite, max_invites)
            .await
    }

    pub async fn remove_player_invite(
        &self,
        actor: &Actor,
        realm_id: &str,
        account_id: &str,
    ) -> Result<bool, RepoError> {
        self.remove_child(actor, "rifts_realm_player_invites", realm_id, account_id)
            .await
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
                query(&format!(
                    "INSERT INTO rifts_realm_invites ({INVITE_COLUMNS}, purge_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
                ))?
                .bind::<Text, _>(invite.code)
                .bind::<Text, _>(invite.realm_id)
                .bind::<Text, _>(invite.created_by)
                .bind::<BigInt, _>(invite.created_at.0)
                .bind::<BigInt, _>(invite.expires_at.0)
                .bind::<BigInt, _>(invite.max_uses)
                .bind::<BigInt, _>(invite.uses)
                .bind::<Nullable<BigInt>, _>(invite.revoked_at.map(|at| at.0))
                .bind::<BigInt, _>(invite.expires_at.saturating_add(INVITE_PURGE_GRACE_MS).0)
                .execute(conn)?;
                Ok(InsertInviteResult::Inserted)
            })
            .await
    }

    pub async fn find_invite(&self, code: &str) -> Result<Option<RealmInviteRecord>, RepoError> {
        let code = code.to_owned();
        self.db.read(move |conn| invite_by_code(conn, &code)).await
    }

    /// Newest first.
    pub async fn list_invites(&self, realm_id: &str, max: u32) -> Result<Vec<RealmInviteRecord>, RepoError> {
        let realm_id = realm_id.to_owned();
        self.db
            .read(move |conn| {
                Ok(query(&format!(
                    "SELECT {INVITE_COLUMNS} FROM rifts_realm_invites WHERE realm_id = ? \
                     ORDER BY created_at DESC, code DESC LIMIT ?"
                ))?
                .bind::<Text, _>(realm_id)
                .bind::<BigInt, _>(limit(max))
                .load::<InviteRow>(conn)?
                .into_iter()
                .map(RealmInviteRecord::from)
                .collect())
            })
            .await
    }

    /// Not revoked, not expired, not used up.
    pub async fn count_active_invites(&self, realm_id: &str, now: UnixMillis) -> Result<i64, RepoError> {
        let realm_id = realm_id.to_owned();
        self.db
            .read(move |conn| {
                count(
                    conn,
                    "SELECT COUNT(*) AS n FROM rifts_realm_invites WHERE realm_id = ? AND revoked_at IS NULL \
                     AND expires_at > ? AND uses < max_uses",
                    |q| q.bind::<Text, _>(realm_id).bind::<BigInt, _>(now.0),
                )
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
                let changed = query(
                    "UPDATE rifts_realm_invites SET uses = uses + 1 WHERE code = ? AND revoked_at IS NULL \
                     AND expires_at > ? AND uses < max_uses",
                )?
                .bind::<Text, _>(code.clone())
                .bind::<BigInt, _>(now.0)
                .execute(conn)?;
                if changed == 0 {
                    return Ok(None);
                }
                invite_by_code(conn, &code)
            })
            .await
    }

    /// A join that consumed a use failed afterwards; never below zero.
    pub async fn refund_invite(&self, actor: &Actor, code: &str) -> Result<(), RepoError> {
        let code = code.to_owned();
        self.db
            .write(actor, move |conn| {
                query("UPDATE rifts_realm_invites SET uses = uses - 1 WHERE code = ? AND uses > 0")?
                    .bind::<Text, _>(code)
                    .execute(conn)?;
                Ok(())
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
                let Some(mut invite) = invite_by_code(conn, &code)? else {
                    return Ok(None);
                };
                if invite.realm_id != realm_id {
                    return Ok(None);
                }
                if invite.revoked_at.is_none() {
                    query("UPDATE rifts_realm_invites SET revoked_at = ? WHERE code = ?")?
                        .bind::<Nullable<BigInt>, _>((Some(at)).map(|at| at.0))
                        .bind::<Text, _>(code)
                        .execute(conn)?;
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
                let invites = query("DELETE FROM rifts_realm_invites WHERE purge_at <= ?")?
                    .bind::<BigInt, _>(now.0)
                    .execute(conn)?;
                let presence = query("DELETE FROM rifts_realm_presence WHERE expires_at <= ?")?
                    .bind::<BigInt, _>(now.0)
                    .execute(conn)?;
                Ok(invites + presence)
            })
            .await
    }
}

const PRESENCE_COLUMNS: &str = "account_id, realm_id, bound_at, last_seen_at, expires_at";

#[derive(QueryableByName)]
struct PresenceRow {
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = Text)]
    realm_id: String,
    #[diesel(sql_type = BigInt)]
    bound_at: i64,
    #[diesel(sql_type = BigInt)]
    last_seen_at: i64,
    #[diesel(sql_type = BigInt)]
    expires_at: i64,
}

impl From<PresenceRow> for RealmPresenceRecord {
    fn from(row: PresenceRow) -> Self {
        Self {
            account_id: row.account_id,
            realm_id: row.realm_id,
            bound_at: UnixMillis(row.bound_at),
            last_seen_at: UnixMillis(row.last_seen_at),
            expires_at: UnixMillis(row.expires_at),
        }
    }
}

#[derive(QueryableByName)]
struct RealmCount {
    #[diesel(sql_type = Text)]
    realm_id: String,
    #[diesel(sql_type = BigInt)]
    n: i64,
}

fn active_presence(
    conn: &mut AnyConnection,
    account_id: &str,
    now: UnixMillis,
) -> Result<Option<RealmPresenceRecord>, RepoError> {
    Ok(query(&format!(
        "SELECT {PRESENCE_COLUMNS} FROM rifts_realm_presence WHERE account_id = ? AND expires_at > ?"
    ))?
    .bind::<Text, _>(account_id.to_owned())
    .bind::<BigInt, _>(now.0)
    .load::<PresenceRow>(conn)?
    .into_iter()
    .next()
    .map(RealmPresenceRecord::from))
}

/// One row per account: which realm its current game login belongs to.
#[derive(Clone)]
pub struct RealmPresenceRepository {
    db: RealmsDb,
}

impl RealmPresenceRepository {
    pub fn new(db: RealmsDb) -> Self {
        Self { db }
    }

    /// Replaces the account's row.
    pub async fn bind(&self, actor: &Actor, row: &RealmPresenceRecord) -> Result<(), RepoError> {
        let row = row.clone();
        self.db
            .write(actor, move |conn| {
                let text = store::portable::upsert(
                    "rifts_realm_presence",
                    &["account_id", "realm_id", "bound_at", "last_seen_at", "expires_at"],
                    &["account_id"],
                    &["realm_id", "bound_at", "last_seen_at", "expires_at"],
                );
                diesel::sql_query(text)
                    .bind::<Text, _>(row.account_id)
                    .bind::<Text, _>(row.realm_id)
                    .bind::<BigInt, _>(row.bound_at.0)
                    .bind::<BigInt, _>(row.last_seen_at.0)
                    .bind::<BigInt, _>(row.expires_at.0)
                    .execute(conn)?;
                Ok(())
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
                let changed = query(
                    "UPDATE rifts_realm_presence SET last_seen_at = ?, expires_at = ? \
                     WHERE account_id = ? AND expires_at > ?",
                )?
                .bind::<BigInt, _>(now.0)
                .bind::<BigInt, _>(expires_at.0)
                .bind::<Text, _>(account_id.clone())
                .bind::<BigInt, _>(now.0)
                .execute(conn)?;
                if changed == 0 {
                    return Ok(None);
                }
                active_presence(conn, &account_id, now)
            })
            .await
    }

    /// With a realm, only a row bound to it.
    pub async fn remove(&self, actor: &Actor, account_id: &str, realm_id: Option<&str>) -> Result<bool, RepoError> {
        let (account_id, realm_id) = (account_id.to_owned(), realm_id.map(str::to_owned));
        self.db
            .write(actor, move |conn| {
                let deleted = match realm_id {
                    None => query("DELETE FROM rifts_realm_presence WHERE account_id = ?")?
                        .bind::<Text, _>(account_id)
                        .execute(conn)?,
                    Some(realm) => query("DELETE FROM rifts_realm_presence WHERE account_id = ? AND realm_id = ?")?
                        .bind::<Text, _>(account_id)
                        .bind::<Text, _>(realm)
                        .execute(conn)?,
                };
                Ok(deleted > 0)
            })
            .await
    }

    pub async fn remove_for_realm(&self, actor: &Actor, realm_id: &str) -> Result<usize, RepoError> {
        let realm_id = realm_id.to_owned();
        self.db
            .write(actor, move |conn| {
                Ok(query("DELETE FROM rifts_realm_presence WHERE realm_id = ?")?
                    .bind::<Text, _>(realm_id)
                    .execute(conn)?)
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
        let realm_ids = realm_ids.to_vec();
        self.db
            .read(move |conn| {
                let mut q = query(&format!(
                    "SELECT realm_id, COUNT(*) AS n FROM rifts_realm_presence WHERE expires_at > ? \
                     AND realm_id IN ({}) GROUP BY realm_id",
                    placeholders(realm_ids.len())
                ))?
                .bind::<BigInt, _>(now.0);
                for id in realm_ids {
                    q = q.bind::<Text, _>(id);
                }
                Ok(q.load::<RealmCount>(conn)?
                    .into_iter()
                    .map(|row| (row.realm_id, row.n))
                    .collect())
            })
            .await
    }
}
