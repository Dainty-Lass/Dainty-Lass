//! `RealmsRepository` and `RealmPresenceRepository` (`realms/persistence/RealmsRepository.ts`) on `store`. Each
//! method that changes a realm or an invite is one write transaction with its precondition inside it; writes are
//! serial on both engines, so racing requests cannot both win a name, the last member slot or the last invite use.

use std::collections::BTreeMap;

use diesel::RunQueryDsl;
use diesel::sql_types::{BigInt, Bool, Nullable, Text};
use rifts_seam::{Actor, UnixMillis};
use store::AnyConnection;

use crate::db::{Count, Query, RealmsDb, RepoError, corrupt, marks, sql};
use crate::realm::{
    AccountAt, HeartbeatUpdate, HostingPatch, HubState, INVITE_PURGE_GRACE_MS, JoinPolicy, MemberRole, RealmBan,
    RealmHosting, RealmInviteRecord, RealmMember, RealmPresenceRecord, RealmRecord, apply_hosting_patch,
};

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

/// A join request's or a player invite's answer; the refusals are checked in Node's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddEntryResult {
    Added,
    /// `ALREADY_REQUESTED` or `ALREADY_INVITED`.
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

#[derive(diesel::QueryableByName)]
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

const REALM_COLUMNS: &str = "realm_id, name, host_account_id, host_public_key, join_policy, listed, created_at, \
    last_online_at, online_until, version, offline_notified_for, hosting_max_servers, hosting_max_players, \
    hosting_afk_seconds, hosting_hub_session_id, hosting_hub_state, hosting_training_state, hosting_active_servers, \
    hosting_updated_at";

#[derive(diesel::QueryableByName)]
struct MemberRow {
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = Text)]
    role: String,
    #[diesel(sql_type = BigInt)]
    joined_at: i64,
}

#[derive(diesel::QueryableByName)]
struct BanRow {
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = Nullable<Text>)]
    reason: Option<String>,
    #[diesel(sql_type = BigInt)]
    at: i64,
}

#[derive(diesel::QueryableByName)]
struct AtRow {
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = BigInt)]
    at: i64,
}

#[derive(diesel::QueryableByName)]
struct RoomRow {
    #[diesel(sql_type = Text)]
    room_id: String,
}

#[derive(diesel::QueryableByName)]
struct IdRow {
    #[diesel(sql_type = Text)]
    realm_id: String,
}

#[derive(diesel::QueryableByName)]
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

const INVITE_COLUMNS: &str = "code, realm_id, created_by, created_at, expires_at, max_uses, uses, revoked_at";

#[derive(diesel::QueryableByName)]
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

const PRESENCE_COLUMNS: &str = "account_id, realm_id, bound_at, last_seen_at, expires_at";

#[derive(diesel::QueryableByName)]
struct RealmCount {
    #[diesel(sql_type = Text)]
    realm_id: String,
    #[diesel(sql_type = BigInt)]
    n: i64,
}

fn hub_state(text: Option<String>) -> Result<HubState, RepoError> {
    match text {
        None => Ok(HubState::None),
        Some(text) => HubState::parse(&text).ok_or_else(|| corrupt("hub state", &text)),
    }
}

fn hosting_of(row: &RealmRow) -> Result<Option<RealmHosting>, RepoError> {
    let Some(updated_at) = row.hosting_updated_at else {
        return Ok(None);
    };
    Ok(Some(RealmHosting {
        max_servers: row.hosting_max_servers.unwrap_or(1),
        max_players: row.hosting_max_players.unwrap_or(16),
        afk_seconds: row.hosting_afk_seconds,
        hub_session_id: row.hosting_hub_session_id.clone(),
        hub_state: hub_state(row.hosting_hub_state.clone())?,
        training_state: hub_state(row.hosting_training_state.clone())?,
        active_servers: row.hosting_active_servers.unwrap_or(0),
        updated_at: UnixMillis(updated_at),
    }))
}

fn by_realm<T>(conn: &mut AnyConnection, select: &str, table: &str, realm_id: &str) -> Result<Vec<T>, RepoError>
where
    T: diesel::QueryableByName<store::MultiBackend> + 'static,
{
    Ok(sql(&format!("SELECT {select} FROM {table} WHERE realm_id = ? ORDER BY seq"))?
        .bind::<Text, _>(realm_id.to_owned())
        .load::<T>(conn)?)
}

fn complete(conn: &mut AnyConnection, row: RealmRow) -> Result<RealmRecord, RepoError> {
    let id = row.realm_id.clone();
    let members = by_realm::<MemberRow>(conn, "account_id, role, joined_at", "rifts_realm_members", &id)?
        .into_iter()
        .map(|m| {
            Ok(RealmMember {
                role: MemberRole::parse(&m.role).ok_or_else(|| corrupt("member role", &m.role))?,
                account_id: m.account_id,
                joined_at: UnixMillis(m.joined_at),
            })
        })
        .collect::<Result<Vec<_>, RepoError>>()?;
    let bans = by_realm::<BanRow>(conn, "account_id, reason, at", "rifts_realm_bans", &id)?
        .into_iter()
        .map(|b| RealmBan {
            account_id: b.account_id,
            reason: b.reason,
            at: UnixMillis(b.at),
        })
        .collect();
    let entries = |rows: Vec<AtRow>| {
        rows.into_iter()
            .map(|r| AccountAt {
                account_id: r.account_id,
                at: UnixMillis(r.at),
            })
            .collect::<Vec<_>>()
    };
    let join_requests = entries(by_realm(conn, "account_id, at", "rifts_realm_join_requests", &id)?);
    let player_invites = entries(by_realm(conn, "account_id, at", "rifts_realm_player_invites", &id)?);
    let hub_room_ids = by_realm::<RoomRow>(conn, "room_id", "rifts_realm_hub_rooms", &id)?
        .into_iter()
        .map(|r| r.room_id)
        .collect();
    Ok(RealmRecord {
        join_policy: JoinPolicy::parse(&row.join_policy).ok_or_else(|| corrupt("join policy", &row.join_policy))?,
        hosting: hosting_of(&row)?,
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
        hub_room_ids,
        offline_notified_for: row.offline_notified_for.map(UnixMillis),
        join_requests,
        player_invites,
    })
}

fn load_one(conn: &mut AnyConnection, query: Query) -> Result<Option<RealmRecord>, RepoError> {
    match query.load::<RealmRow>(conn)?.into_iter().next() {
        Some(row) => complete(conn, row).map(Some),
        None => Ok(None),
    }
}

fn realm_where(conn: &mut AnyConnection, column: &str, value: &str) -> Result<Option<RealmRecord>, RepoError> {
    let query = sql(&format!("SELECT {REALM_COLUMNS} FROM rifts_realms WHERE {column} = ?"))?;
    load_one(conn, query.bind::<Text, _>(value.to_owned()))
}

fn realms_by_ids(conn: &mut AnyConnection, ids: Vec<IdRow>) -> Result<Vec<RealmRecord>, RepoError> {
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(realm) = realm_where(conn, "realm_id", &id.realm_id)? {
            out.push(realm);
        }
    }
    Ok(out)
}

fn exists(conn: &mut AnyConnection, realm_id: &str) -> Result<bool, RepoError> {
    count(conn, "SELECT COUNT(*) AS n FROM rifts_realms WHERE realm_id = ?", &[realm_id]).map(|n| n > 0)
}

fn count(conn: &mut AnyConnection, text: &str, binds: &[&str]) -> Result<i64, RepoError> {
    let mut query = sql(text)?;
    for bind in binds {
        query = query.bind::<Text, _>((*bind).to_owned());
    }
    Ok(query.load::<Count>(conn)?.into_iter().next().map_or(0, |c| c.n))
}

fn has_row(conn: &mut AnyConnection, table: &str, realm_id: &str, account_id: &str) -> Result<bool, RepoError> {
    let text = format!("SELECT COUNT(*) AS n FROM {table} WHERE realm_id = ? AND account_id = ?");
    count(conn, &text, &[realm_id, account_id]).map(|n| n > 0)
}

fn rows_of(conn: &mut AnyConnection, table: &str, realm_id: &str) -> Result<i64, RepoError> {
    count(conn, &format!("SELECT COUNT(*) AS n FROM {table} WHERE realm_id = ?"), &[realm_id])
}

fn next_seq(conn: &mut AnyConnection, table: &str, realm_id: &str) -> Result<i64, RepoError> {
    count(
        conn,
        &format!("SELECT COALESCE(MAX(seq), 0) + 1 AS n FROM {table} WHERE realm_id = ?"),
        &[realm_id],
    )
}

fn delete_row(conn: &mut AnyConnection, table: &str, realm_id: &str, account_id: &str) -> Result<bool, RepoError> {
    Ok(sql(&format!("DELETE FROM {table} WHERE realm_id = ? AND account_id = ?"))?
        .bind::<Text, _>(realm_id.to_owned())
        .bind::<Text, _>(account_id.to_owned())
        .execute(conn)?
        > 0)
}

fn insert_entry(conn: &mut AnyConnection, table: &str, realm_id: &str, entry: &AccountAt) -> Result<(), RepoError> {
    let seq = next_seq(conn, table, realm_id)?;
    sql(&format!("INSERT INTO {table} (realm_id, account_id, at, seq) VALUES (?, ?, ?, ?)"))?
        .bind::<Text, _>(realm_id.to_owned())
        .bind::<Text, _>(entry.account_id.clone())
        .bind::<BigInt, _>(entry.at.0)
        .bind::<BigInt, _>(seq)
        .execute(conn)?;
    Ok(())
}

fn insert_member(conn: &mut AnyConnection, realm_id: &str, member: &RealmMember) -> Result<(), RepoError> {
    let seq = next_seq(conn, "rifts_realm_members", realm_id)?;
    sql("INSERT INTO rifts_realm_members (realm_id, account_id, role, joined_at, seq) VALUES (?, ?, ?, ?, ?)")?
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
    sql("INSERT INTO rifts_realm_bans (realm_id, account_id, reason, at, seq) VALUES (?, ?, ?, ?, ?)")?
        .bind::<Text, _>(realm_id.to_owned())
        .bind::<Text, _>(ban.account_id.clone())
        .bind::<Nullable<Text>, _>(ban.reason.clone())
        .bind::<BigInt, _>(ban.at.0)
        .bind::<BigInt, _>(seq)
        .execute(conn)?;
    Ok(())
}

/// The realm, its children and its invites (Node deletes the invites with the realm).
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
        sql(&format!("DELETE FROM {table} WHERE realm_id = ?"))?
            .bind::<Text, _>(realm_id.to_owned())
            .execute(conn)?;
    }
    Ok(())
}

fn write_hosting(conn: &mut AnyConnection, realm_id: &str, hosting: &RealmHosting) -> Result<usize, RepoError> {
    Ok(sql("UPDATE rifts_realms SET hosting_max_servers = ?, hosting_max_players = ?, hosting_afk_seconds = ?, \
         hosting_hub_session_id = ?, hosting_hub_state = ?, hosting_training_state = ?, hosting_active_servers = ?, \
         hosting_updated_at = ? WHERE realm_id = ?")?
    .bind::<BigInt, _>(hosting.max_servers)
    .bind::<BigInt, _>(hosting.max_players)
    .bind::<Nullable<BigInt>, _>(hosting.afk_seconds)
    .bind::<Nullable<Text>, _>(hosting.hub_session_id.clone())
    .bind::<Text, _>(hosting.hub_state.as_str())
    .bind::<Text, _>(hosting.training_state.as_str())
    .bind::<BigInt, _>(hosting.active_servers)
    .bind::<BigInt, _>(hosting.updated_at.0)
    .bind::<Text, _>(realm_id.to_owned())
    .execute(conn)?)
}

fn invite_of(row: InviteRow) -> RealmInviteRecord {
    RealmInviteRecord {
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

fn find_invite(conn: &mut AnyConnection, code: &str) -> Result<Option<RealmInviteRecord>, RepoError> {
    Ok(sql(&format!("SELECT {INVITE_COLUMNS} FROM rifts_realm_invites WHERE code = ?"))?
        .bind::<Text, _>(code.to_owned())
        .load::<InviteRow>(conn)?
        .into_iter()
        .next()
        .map(invite_of))
}

fn presence_of(row: PresenceRow) -> RealmPresenceRecord {
    RealmPresenceRecord {
        account_id: row.account_id,
        realm_id: row.realm_id,
        bound_at: UnixMillis(row.bound_at),
        last_seen_at: UnixMillis(row.last_seen_at),
        expires_at: UnixMillis(row.expires_at),
    }
}

fn find_presence(conn: &mut AnyConnection, account_id: &str, now: UnixMillis) -> Result<Option<RealmPresenceRecord>, RepoError> {
    Ok(sql(&format!("SELECT {PRESENCE_COLUMNS} FROM rifts_realm_presence WHERE account_id = ? AND expires_at > ?"))?
        .bind::<Text, _>(account_id.to_owned())
        .bind::<BigInt, _>(now.0)
        .load::<PresenceRow>(conn)?
        .into_iter()
        .next()
        .map(presence_of))
}

/// Realms' directory tables.
#[derive(Clone)]
pub struct RealmsRepository {
    db: RealmsDb,
}

impl RealmsRepository {
    pub fn new(db: RealmsDb) -> Self {
        Self { db }
    }

    pub async fn find_by_id(&self, realm_id: &str) -> Result<Option<RealmRecord>, RepoError> {
        let id = realm_id.to_owned();
        self.db.read(move |conn| realm_where(conn, "realm_id", &id)).await
    }

    pub async fn find_by_name(&self, name: &str) -> Result<Option<RealmRecord>, RepoError> {
        let name = name.to_owned();
        self.db.read(move |conn| realm_where(conn, "name", &name)).await
    }

    pub async fn find_by_host(&self, host: &str) -> Result<Option<RealmRecord>, RepoError> {
        let host = host.to_owned();
        self.db.read(move |conn| realm_where(conn, "host_account_id", &host)).await
    }

    /// The realms the account is in (its own included), by name.
    pub async fn find_by_member(&self, account_id: &str, limit: u32) -> Result<Vec<RealmRecord>, RepoError> {
        self.by_child("rifts_realm_members", account_id, limit).await
    }

    /// Realms holding a player invite for the account, by name.
    pub async fn find_by_player_invite(&self, account_id: &str, limit: u32) -> Result<Vec<RealmRecord>, RepoError> {
        self.by_child("rifts_realm_player_invites", account_id, limit).await
    }

    async fn by_child(&self, table: &'static str, account_id: &str, limit: u32) -> Result<Vec<RealmRecord>, RepoError> {
        let account = account_id.to_owned();
        self.db
            .read(move |conn| {
                let ids = sql(&format!(
                    "SELECT r.realm_id FROM rifts_realms r JOIN {table} c ON c.realm_id = r.realm_id \
                     WHERE c.account_id = ? ORDER BY r.name LIMIT ?"
                ))?
                .bind::<Text, _>(account)
                .bind::<BigInt, _>(i64::from(limit))
                .load::<IdRow>(conn)?;
                realms_by_ids(conn, ids)
            })
            .await
    }

    /// Listed realms, online ones first (Mongo's `onlineUntil` descending, absent last), then by name.
    pub async fn list_listed(&self, limit: u32, offset: u32) -> Result<Vec<RealmRecord>, RepoError> {
        self.db
            .read(move |conn| {
                let ids = sql("SELECT realm_id FROM rifts_realms WHERE listed \
                     ORDER BY (online_until IS NULL), online_until DESC, name LIMIT ? OFFSET ?")?
                .bind::<BigInt, _>(i64::from(limit))
                .bind::<BigInt, _>(i64::from(offset))
                .load::<IdRow>(conn)?;
                realms_by_ids(conn, ids)
            })
            .await
    }

    /// The name and the host account are each unique.
    pub async fn insert_realm(&self, actor: &Actor, realm: &RealmRecord) -> Result<InsertRealmResult, RepoError> {
        let realm = realm.clone();
        self.db
            .write(actor, move |conn| {
                if count(conn, "SELECT COUNT(*) AS n FROM rifts_realms WHERE name = ?", &[&realm.name])? > 0 {
                    return Ok(InsertRealmResult::NameTaken);
                }
                let host = &realm.host_account_id;
                if count(conn, "SELECT COUNT(*) AS n FROM rifts_realms WHERE host_account_id = ?", &[host])? > 0 {
                    return Ok(InsertRealmResult::HostHasRealm);
                }
                sql("INSERT INTO rifts_realms (realm_id, name, host_account_id, host_public_key, join_policy, listed, \
                     created_at, last_online_at, online_until, version, offline_notified_for) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")?
                .bind::<Text, _>(realm.realm_id.clone())
                .bind::<Text, _>(realm.name.clone())
                .bind::<Text, _>(realm.host_account_id.clone())
                .bind::<Text, _>(realm.host_public_key.clone())
                .bind::<Text, _>(realm.join_policy.as_str())
                .bind::<Bool, _>(realm.listed)
                .bind::<BigInt, _>(realm.created_at.0)
                .bind::<Nullable<BigInt>, _>(realm.last_online_at.map(|t| t.0))
                .bind::<Nullable<BigInt>, _>(realm.online_until.map(|t| t.0))
                .bind::<Nullable<Text>, _>(realm.version.clone())
                .bind::<Nullable<BigInt>, _>(realm.offline_notified_for.map(|t| t.0))
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
                for entry in &realm.join_requests {
                    insert_entry(conn, "rifts_realm_join_requests", &realm.realm_id, entry)?;
                }
                for entry in &realm.player_invites {
                    insert_entry(conn, "rifts_realm_player_invites", &realm.realm_id, entry)?;
                }
                for (seq, room) in (1i64..).zip(&realm.hub_room_ids) {
                    sql("INSERT INTO rifts_realm_hub_rooms (room_id, realm_id, seq) VALUES (?, ?, ?)")?
                        .bind::<Text, _>(room.clone())
                        .bind::<Text, _>(realm.realm_id.clone())
                        .bind::<BigInt, _>(seq)
                        .execute(conn)?;
                }
                Ok(InsertRealmResult::Inserted)
            })
            .await
    }

    /// Name release: deletes only a realm not online since `expected_last_online_at` was read, with its invites.
    pub async fn delete_realm_if_last_online(
        &self,
        actor: &Actor,
        realm_id: &str,
        expected_last_online_at: Option<UnixMillis>,
    ) -> Result<bool, RepoError> {
        let id = realm_id.to_owned();
        self.db
            .write(actor, move |conn| {
                let Some(realm) = realm_where(conn, "realm_id", &id)? else {
                    return Ok(false);
                };
                if realm.last_online_at != expected_last_online_at {
                    return Ok(false);
                }
                delete_realm(conn, &id)?;
                Ok(true)
            })
            .await
    }

    /// The host closes its own realm; its invites go with it.
    pub async fn delete_realm_of_host(&self, actor: &Actor, realm_id: &str, host: &str) -> Result<bool, RepoError> {
        let (id, host) = (realm_id.to_owned(), host.to_owned());
        self.db
            .write(actor, move |conn| {
                let text = "SELECT COUNT(*) AS n FROM rifts_realms WHERE realm_id = ? AND host_account_id = ?";
                if count(conn, text, &[&id, &host])? == 0 {
                    return Ok(false);
                }
                delete_realm(conn, &id)?;
                Ok(true)
            })
            .await
    }

    /// Nothing to change answers true without looking, as Node's.
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
        let (id, host) = (realm_id.to_owned(), host.to_owned());
        self.db
            .write(actor, move |conn| {
                let text = "SELECT COUNT(*) AS n FROM rifts_realms WHERE realm_id = ? AND host_account_id = ?";
                if count(conn, text, &[&id, &host])? == 0 {
                    return Ok(false);
                }
                if let Some(policy) = join_policy {
                    sql("UPDATE rifts_realms SET join_policy = ? WHERE realm_id = ?")?
                        .bind::<Text, _>(policy.as_str())
                        .bind::<Text, _>(id.clone())
                        .execute(conn)?;
                }
                if let Some(listed) = listed {
                    sql("UPDATE rifts_realms SET listed = ? WHERE realm_id = ?")?
                        .bind::<Bool, _>(listed)
                        .bind::<Text, _>(id.clone())
                        .execute(conn)?;
                }
                Ok(true)
            })
            .await
    }

    /// A heartbeat without a version keeps the one on record.
    pub async fn record_heartbeat(
        &self,
        actor: &Actor,
        realm_id: &str,
        host: &str,
        update: &HeartbeatUpdate,
    ) -> Result<bool, RepoError> {
        let (id, host, update) = (realm_id.to_owned(), host.to_owned(), update.clone());
        self.db
            .write(actor, move |conn| {
                let changed = sql("UPDATE rifts_realms SET last_online_at = ?, online_until = ?, \
                     version = COALESCE(?, version) WHERE realm_id = ? AND host_account_id = ?")?
                .bind::<BigInt, _>(update.last_online_at.0)
                .bind::<BigInt, _>(update.online_until.0)
                .bind::<Nullable<Text>, _>(update.version)
                .bind::<Text, _>(id)
                .bind::<Text, _>(host)
                .execute(conn)?;
                Ok(changed == 1)
            })
            .await
    }

    /// Merges the patch into the realm's hosting (created with the defaults when absent); false when the realm is gone.
    pub async fn patch_hosting(
        &self,
        actor: &Actor,
        realm_id: &str,
        patch: &HostingPatch,
        at: UnixMillis,
    ) -> Result<bool, RepoError> {
        let (id, patch) = (realm_id.to_owned(), patch.clone());
        self.db
            .write(actor, move |conn| {
                let query = sql(&format!("SELECT {REALM_COLUMNS} FROM rifts_realms WHERE realm_id = ?"))?;
                let Some(row) = query.bind::<Text, _>(id.clone()).load::<RealmRow>(conn)?.into_iter().next() else {
                    return Ok(false);
                };
                let next = apply_hosting_patch(hosting_of(&row)?, &patch, at);
                write_hosting(conn, &id, &next)?;
                Ok(true)
            })
            .await
    }

    /// Ends a lasting online state at `at` without the sweeper noticing it; false when it was not online.
    pub async fn mark_offline(&self, actor: &Actor, realm_id: &str, at: UnixMillis) -> Result<bool, RepoError> {
        let id = realm_id.to_owned();
        self.db
            .write(actor, move |conn| {
                let changed = sql("UPDATE rifts_realms SET online_until = ?, offline_notified_for = ? \
                     WHERE realm_id = ? AND online_until > ?")?
                .bind::<BigInt, _>(at.0)
                .bind::<BigInt, _>(at.0)
                .bind::<Text, _>(id)
                .bind::<BigInt, _>(at.0)
                .execute(conn)?;
                Ok(changed == 1)
            })
            .await
    }

    /// Realms whose online state lapsed within `(since, now]` and are not noticed yet, each claimed once.
    pub async fn claim_lapsed_hosts(
        &self,
        actor: &Actor,
        now: UnixMillis,
        since: UnixMillis,
        limit: u32,
    ) -> Result<Vec<RealmRecord>, RepoError> {
        self.db
            .write(actor, move |conn| {
                let ids = sql("SELECT realm_id FROM rifts_realms WHERE online_until > ? AND online_until <= ? \
                     AND (offline_notified_for IS NULL OR offline_notified_for <> online_until) \
                     ORDER BY online_until, realm_id LIMIT ?")?
                .bind::<BigInt, _>(since.0)
                .bind::<BigInt, _>(now.0)
                .bind::<BigInt, _>(i64::from(limit))
                .load::<IdRow>(conn)?;
                for id in &ids {
                    sql("UPDATE rifts_realms SET offline_notified_for = online_until WHERE realm_id = ?")?
                        .bind::<Text, _>(id.realm_id.clone())
                        .execute(conn)?;
                }
                realms_by_ids(conn, ids)
            })
            .await
    }

    pub async fn find_by_hub_room_id(&self, room_id: &str) -> Result<Option<RealmRecord>, RepoError> {
        let room = room_id.to_owned();
        self.db
            .read(move |conn| {
                let ids = sql("SELECT realm_id FROM rifts_realm_hub_rooms WHERE room_id = ?")?
                    .bind::<Text, _>(room)
                    .load::<IdRow>(conn)?;
                Ok(realms_by_ids(conn, ids)?.into_iter().next())
            })
            .await
    }

    /// Adds a member unless in, banned or the realm is full; the same write drops the account's player invite.
    pub async fn add_member(
        &self,
        actor: &Actor,
        realm_id: &str,
        member: &RealmMember,
        max_members: u32,
    ) -> Result<AddMemberResult, RepoError> {
        let (id, member) = (realm_id.to_owned(), member.clone());
        self.db
            .write(actor, move |conn| {
                if !exists(conn, &id)? {
                    return Ok(AddMemberResult::NotFound);
                }
                if has_row(conn, "rifts_realm_bans", &id, &member.account_id)? {
                    return Ok(AddMemberResult::Banned);
                }
                if has_row(conn, "rifts_realm_members", &id, &member.account_id)? {
                    return Ok(AddMemberResult::AlreadyMember);
                }
                if rows_of(conn, "rifts_realm_members", &id)? >= i64::from(max_members) {
                    return Ok(AddMemberResult::Full);
                }
                insert_member(conn, &id, &member)?;
                delete_row(conn, "rifts_realm_player_invites", &id, &member.account_id)?;
                Ok(AddMemberResult::Added)
            })
            .await
    }

    /// Removes a plain member, never the host.
    pub async fn remove_member(&self, actor: &Actor, realm_id: &str, account_id: &str) -> Result<bool, RepoError> {
        let (id, account) = (realm_id.to_owned(), account_id.to_owned());
        self.db
            .write(actor, move |conn| {
                let removed = sql("DELETE FROM rifts_realm_members WHERE realm_id = ? AND account_id = ? AND role = ?")?
                    .bind::<Text, _>(id)
                    .bind::<Text, _>(account)
                    .bind::<Text, _>(MemberRole::Member.as_str())
                    .execute(conn)?;
                Ok(removed > 0)
            })
            .await
    }

    /// Bans an account and removes its membership (never the host's), join request and player invite.
    pub async fn add_ban(
        &self,
        actor: &Actor,
        realm_id: &str,
        ban: &RealmBan,
        max_bans: u32,
    ) -> Result<AddBanResult, RepoError> {
        let (id, ban) = (realm_id.to_owned(), ban.clone());
        self.db
            .write(actor, move |conn| {
                if !exists(conn, &id)? {
                    return Ok(AddBanResult::NotFound);
                }
                if has_row(conn, "rifts_realm_bans", &id, &ban.account_id)? {
                    return Ok(AddBanResult::AlreadyBanned);
                }
                if rows_of(conn, "rifts_realm_bans", &id)? >= i64::from(max_bans) {
                    return Ok(AddBanResult::Full);
                }
                insert_ban(conn, &id, &ban)?;
                sql("DELETE FROM rifts_realm_members WHERE realm_id = ? AND account_id = ? AND role = ?")?
                    .bind::<Text, _>(id.clone())
                    .bind::<Text, _>(ban.account_id.clone())
                    .bind::<Text, _>(MemberRole::Member.as_str())
                    .execute(conn)?;
                delete_row(conn, "rifts_realm_join_requests", &id, &ban.account_id)?;
                delete_row(conn, "rifts_realm_player_invites", &id, &ban.account_id)?;
                Ok(AddBanResult::Banned)
            })
            .await
    }

    pub async fn remove_ban(&self, actor: &Actor, realm_id: &str, account_id: &str) -> Result<bool, RepoError> {
        self.delete_child(actor, "rifts_realm_bans", realm_id, account_id).await
    }

    /// Checked in Node's order: missing realm, banned, member, already requested, full.
    pub async fn add_join_request(
        &self,
        actor: &Actor,
        realm_id: &str,
        request: &AccountAt,
        max_requests: u32,
    ) -> Result<AddEntryResult, RepoError> {
        self.add_entry(actor, "rifts_realm_join_requests", realm_id, request, max_requests).await
    }

    pub async fn remove_join_request(&self, actor: &Actor, realm_id: &str, account_id: &str) -> Result<bool, RepoError> {
        self.delete_child(actor, "rifts_realm_join_requests", realm_id, account_id).await
    }

    pub async fn add_player_invite(
        &self,
        actor: &Actor,
        realm_id: &str,
        invite: &AccountAt,
        max_invites: u32,
    ) -> Result<AddEntryResult, RepoError> {
        self.add_entry(actor, "rifts_realm_player_invites", realm_id, invite, max_invites).await
    }

    pub async fn remove_player_invite(&self, actor: &Actor, realm_id: &str, account_id: &str) -> Result<bool, RepoError> {
        self.delete_child(actor, "rifts_realm_player_invites", realm_id, account_id).await
    }

    async fn add_entry(
        &self,
        actor: &Actor,
        table: &'static str,
        realm_id: &str,
        entry: &AccountAt,
        max: u32,
    ) -> Result<AddEntryResult, RepoError> {
        let (id, entry) = (realm_id.to_owned(), entry.clone());
        self.db
            .write(actor, move |conn| {
                if !exists(conn, &id)? {
                    return Ok(AddEntryResult::NotFound);
                }
                if has_row(conn, "rifts_realm_bans", &id, &entry.account_id)? {
                    return Ok(AddEntryResult::Banned);
                }
                if has_row(conn, "rifts_realm_members", &id, &entry.account_id)? {
                    return Ok(AddEntryResult::AlreadyMember);
                }
                if has_row(conn, table, &id, &entry.account_id)? {
                    return Ok(AddEntryResult::Already);
                }
                if rows_of(conn, table, &id)? >= i64::from(max) {
                    return Ok(AddEntryResult::Full);
                }
                insert_entry(conn, table, &id, &entry)?;
                Ok(AddEntryResult::Added)
            })
            .await
    }

    async fn delete_child(
        &self,
        actor: &Actor,
        table: &'static str,
        realm_id: &str,
        account_id: &str,
    ) -> Result<bool, RepoError> {
        let (id, account) = (realm_id.to_owned(), account_id.to_owned());
        self.db.write(actor, move |conn| delete_row(conn, table, &id, &account)).await
    }

    pub async fn insert_invite(&self, actor: &Actor, invite: &RealmInviteRecord) -> Result<InsertInviteResult, RepoError> {
        let invite = invite.clone();
        self.db
            .write(actor, move |conn| {
                if find_invite(conn, &invite.code)?.is_some() {
                    return Ok(InsertInviteResult::Duplicate);
                }
                sql(&format!(
                    "INSERT INTO rifts_realm_invites ({INVITE_COLUMNS}, purge_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
                ))?
                .bind::<Text, _>(invite.code)
                .bind::<Text, _>(invite.realm_id)
                .bind::<Text, _>(invite.created_by)
                .bind::<BigInt, _>(invite.created_at.0)
                .bind::<BigInt, _>(invite.expires_at.0)
                .bind::<BigInt, _>(invite.max_uses)
                .bind::<BigInt, _>(invite.uses)
                .bind::<Nullable<BigInt>, _>(invite.revoked_at.map(|t| t.0))
                .bind::<BigInt, _>(invite.expires_at.0.saturating_add(INVITE_PURGE_GRACE_MS))
                .execute(conn)?;
                Ok(InsertInviteResult::Inserted)
            })
            .await
    }

    pub async fn find_invite(&self, code: &str) -> Result<Option<RealmInviteRecord>, RepoError> {
        let code = code.to_owned();
        self.db.read(move |conn| find_invite(conn, &code)).await
    }

    /// Newest first.
    pub async fn list_invites(&self, realm_id: &str, limit: u32) -> Result<Vec<RealmInviteRecord>, RepoError> {
        let id = realm_id.to_owned();
        self.db
            .read(move |conn| {
                Ok(sql(&format!(
                    "SELECT {INVITE_COLUMNS} FROM rifts_realm_invites WHERE realm_id = ? \
                     ORDER BY created_at DESC, code DESC LIMIT ?"
                ))?
                .bind::<Text, _>(id)
                .bind::<BigInt, _>(i64::from(limit))
                .load::<InviteRow>(conn)?
                .into_iter()
                .map(invite_of)
                .collect())
            })
            .await
    }

    /// Invites usable at `now`: not revoked, not expired, not used up.
    pub async fn count_active_invites(&self, realm_id: &str, now: UnixMillis) -> Result<i64, RepoError> {
        let id = realm_id.to_owned();
        self.db
            .read(move |conn| {
                Ok(sql("SELECT COUNT(*) AS n FROM rifts_realm_invites WHERE realm_id = ? AND revoked_at IS NULL \
                     AND expires_at > ? AND uses < max_uses")?
                .bind::<Text, _>(id)
                .bind::<BigInt, _>(now.0)
                .load::<Count>(conn)?
                .into_iter()
                .next()
                .map_or(0, |c| c.n))
            })
            .await
    }

    /// One more use if still usable at `now`; the invite after it, or `None`.
    pub async fn consume_invite(
        &self,
        actor: &Actor,
        code: &str,
        now: UnixMillis,
    ) -> Result<Option<RealmInviteRecord>, RepoError> {
        let code = code.to_owned();
        self.db
            .write(actor, move |conn| {
                let changed = sql("UPDATE rifts_realm_invites SET uses = uses + 1 WHERE code = ? \
                     AND revoked_at IS NULL AND expires_at > ? AND uses < max_uses")?
                .bind::<Text, _>(code.clone())
                .bind::<BigInt, _>(now.0)
                .execute(conn)?;
                if changed == 0 {
                    return Ok(None);
                }
                find_invite(conn, &code)
            })
            .await
    }

    /// Gives one use back (a join that consumed it failed afterwards); never below zero.
    pub async fn refund_invite(&self, actor: &Actor, code: &str) -> Result<(), RepoError> {
        let code = code.to_owned();
        self.db
            .write(actor, move |conn| {
                sql("UPDATE rifts_realm_invites SET uses = uses - 1 WHERE code = ? AND uses > 0")?
                    .bind::<Text, _>(code)
                    .execute(conn)?;
                Ok(())
            })
            .await
    }

    /// Revoked once (the first time stays); `None` when it is not an invite of this realm.
    pub async fn revoke_invite(
        &self,
        actor: &Actor,
        realm_id: &str,
        code: &str,
        at: UnixMillis,
    ) -> Result<Option<RealmInviteRecord>, RepoError> {
        let (id, code) = (realm_id.to_owned(), code.to_owned());
        self.db
            .write(actor, move |conn| {
                let Some(invite) = find_invite(conn, &code)?.filter(|invite| invite.realm_id == id) else {
                    return Ok(None);
                };
                if invite.revoked_at.is_some() {
                    return Ok(Some(invite));
                }
                sql("UPDATE rifts_realm_invites SET revoked_at = ? WHERE code = ?")?
                    .bind::<BigInt, _>(at.0)
                    .bind::<Text, _>(code.clone())
                    .execute(conn)?;
                find_invite(conn, &code)
            })
            .await
    }

    /// Node's TTL indexes: invites a week past expiry, presence rows past theirs.
    pub async fn purge_expired(&self, now: UnixMillis) -> Result<usize, RepoError> {
        self.db
            .write(&Actor::System("realms-purge"), move |conn| {
                let invites = sql("DELETE FROM rifts_realm_invites WHERE purge_at <= ?")?
                    .bind::<BigInt, _>(now.0)
                    .execute(conn)?;
                let presence = sql("DELETE FROM rifts_realm_presence WHERE expires_at <= ?")?
                    .bind::<BigInt, _>(now.0)
                    .execute(conn)?;
                Ok(invites + presence)
            })
            .await
    }
}

/// Which realm an account's current game login belongs to, one row per account.
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
                diesel::sql_query(store::portable::upsert(
                    "rifts_realm_presence",
                    &["account_id", "realm_id", "bound_at", "last_seen_at", "expires_at"],
                    &["account_id"],
                    &["realm_id", "bound_at", "last_seen_at", "expires_at"],
                ))
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

    /// The row while it is active at `now` (strictly before its `expires_at`).
    pub async fn find_active(&self, account_id: &str, now: UnixMillis) -> Result<Option<RealmPresenceRecord>, RepoError> {
        let account = account_id.to_owned();
        self.db.read(move |conn| find_presence(conn, &account, now)).await
    }

    /// Extends a row still active at `now`; an expired or absent one is never brought back.
    pub async fn touch(
        &self,
        actor: &Actor,
        account_id: &str,
        now: UnixMillis,
        expires_at: UnixMillis,
    ) -> Result<Option<RealmPresenceRecord>, RepoError> {
        let account = account_id.to_owned();
        self.db
            .write(actor, move |conn| {
                let changed = sql("UPDATE rifts_realm_presence SET last_seen_at = ?, expires_at = ? \
                     WHERE account_id = ? AND expires_at > ?")?
                .bind::<BigInt, _>(now.0)
                .bind::<BigInt, _>(expires_at.0)
                .bind::<Text, _>(account.clone())
                .bind::<BigInt, _>(now.0)
                .execute(conn)?;
                if changed == 0 {
                    return Ok(None);
                }
                find_presence(conn, &account, now)
            })
            .await
    }

    /// With `realm_id`, only when the row is bound to that realm.
    pub async fn remove(&self, actor: &Actor, account_id: &str, realm_id: Option<&str>) -> Result<bool, RepoError> {
        let (account, realm) = (account_id.to_owned(), realm_id.map(str::to_owned));
        self.db
            .write(actor, move |conn| {
                let removed = match realm {
                    Some(realm) => sql("DELETE FROM rifts_realm_presence WHERE account_id = ? AND realm_id = ?")?
                        .bind::<Text, _>(account)
                        .bind::<Text, _>(realm)
                        .execute(conn)?,
                    None => sql("DELETE FROM rifts_realm_presence WHERE account_id = ?")?
                        .bind::<Text, _>(account)
                        .execute(conn)?,
                };
                Ok(removed > 0)
            })
            .await
    }

    pub async fn remove_for_realm(&self, actor: &Actor, realm_id: &str) -> Result<usize, RepoError> {
        let realm = realm_id.to_owned();
        self.db
            .write(actor, move |conn| {
                Ok(sql("DELETE FROM rifts_realm_presence WHERE realm_id = ?")?
                    .bind::<Text, _>(realm)
                    .execute(conn)?)
            })
            .await
    }

    /// Active rows per realm; realms with none are absent.
    pub async fn count_active_by_realm(
        &self,
        realm_ids: &[String],
        now: UnixMillis,
    ) -> Result<BTreeMap<String, i64>, RepoError> {
        if realm_ids.is_empty() {
            return Ok(BTreeMap::new());
        }
        let ids = realm_ids.to_vec();
        self.db
            .read(move |conn| {
                let mut query = sql(&format!(
                    "SELECT realm_id, COUNT(*) AS n FROM rifts_realm_presence WHERE realm_id IN ({}) AND expires_at > ? \
                     GROUP BY realm_id",
                    marks(ids.len())
                ))?;
                for id in ids {
                    query = query.bind::<Text, _>(id);
                }
                Ok(query
                    .bind::<BigInt, _>(now.0)
                    .load::<RealmCount>(conn)?
                    .into_iter()
                    .map(|row| (row.realm_id, row.n))
                    .collect())
            })
            .await
    }
}
