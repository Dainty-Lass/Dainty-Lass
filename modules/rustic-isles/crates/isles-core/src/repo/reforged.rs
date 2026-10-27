//! RU5 Reforged API: the admin panel's sessions and audit, launcher notices, the activity monitor and Launcher Guard
//! (isles' since RF1's decision 1).

use diesel::prelude::*;

use super::{AccountId, AnyConnection, Json, RepoResult, Tx, repository};

/// `AdminSessionRecord`: the admin dashboard's cookie session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminSessionRecord {
    pub id: String,
    pub token_hash: String,
    pub account: AccountId,
    pub csrf_token: String,
    pub created_at: String,
    pub expires_at: String,
    pub ip: String,
    pub user_agent: String,
    pub revoked_at: Option<String>,
}

/// `AdminAuditRecord`. The actor and the target are the userIds the operator saw (or `manager:<id>`, `host`), as Node
/// kept them: the audit says what was done to whom in the words of the dashboard.
#[derive(Debug, Clone, PartialEq)]
pub struct AdminAuditRecord {
    pub id: String,
    pub actor_user_id: String,
    pub target_user_id: Option<String>,
    pub action: String,
    pub old_state: Option<Json>,
    pub new_state: Option<Json>,
    pub reason: Option<String>,
    pub ip: String,
    pub request_id: String,
    pub created_at: String,
}

/// `PlayerDeletionResult`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerDeletion {
    pub deleted_counts: Vec<(String, u64)>,
    pub character_count: u64,
}

repository! {
    pub trait AdminRepository("admin") {
        fn create_session(&self, tx: &mut Tx<'_>, session: &AdminSessionRecord) -> ();
        /// Not revoked and not expired at `now`
        fn find_active_session_by_token_hash(&self, conn: &mut AnyConnection, token_hash: &str, now: &str)
            -> Option<AdminSessionRecord>;
        fn revoke_session(&self, tx: &mut Tx<'_>, id: &str, at: &str) -> ();
        fn append_audit(&self, tx: &mut Tx<'_>, record: &AdminAuditRecord) -> ();
        /// Newest first (an empty target lists everything); the records and the total
        fn list_audit(&self, conn: &mut AnyConnection, target_user_id: Option<&str>, skip: u32, limit: u32)
            -> (Vec<AdminAuditRecord>, u64);
        /// Player-facing rows are soft-deleted (DD23), so `revert` can bring them back
        fn delete_player_data(&self, tx: &mut Tx<'_>, account: &AccountId, user_id: &str, audit: &AdminAuditRecord)
            -> PlayerDeletion;
    }
}

/// `LauncherNoticesRecord`: one document, replaced by version. The announcements and tags are the validated JSON, so
/// their key order reaches the dashboard as Node wrote it.
#[derive(Debug, Clone, PartialEq)]
pub struct LauncherNoticesRecord {
    pub version: u64,
    pub updated_at: Option<String>,
    pub updated_by: Option<String>,
    pub announcements: Vec<Json>,
    pub tags: Vec<Json>,
}

repository! {
    pub trait LauncherNoticesRepository("launcherNotices") {
        fn get(&self, conn: &mut AnyConnection) -> Option<LauncherNoticesRecord>;
        /// Replaces the document when its version is `expected_version` (0 when there is none yet); `false` otherwise
        fn replace_if_version(&self, tx: &mut Tx<'_>, record: &LauncherNoticesRecord, expected_version: u64) -> bool;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HuntKind {
    Dedicated,
    Realm,
    PlayerHosted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HuntResult {
    Completed,
    Abandoned,
}

/// `ActivityHuntRecord`. `ttl_at` is unix milliseconds; a sweeper purges past it on both engines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityHuntRecord {
    pub id: String,
    pub account: AccountId,
    pub host: bool,
    pub kind: HuntKind,
    pub hunt_id: String,
    pub hunt_ref: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub end_reason: Option<String>,
    pub result: Option<HuntResult>,
    pub kills: u64,
    pub ttl_at: i64,
}

/// `SlayersPathUnlockRecord`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlayersPathUnlockRecord {
    pub id: String,
    pub account: AccountId,
    pub node_id: String,
    pub unlocked_at: String,
    /// `host` or `gameserver`
    pub source: String,
    pub ttl_at: i64,
}

/// `ActivityFlagRecord`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityFlagRecord {
    pub id: String,
    pub kind: String,
    /// `review` or `high`
    pub severity: String,
    pub account: AccountId,
    pub day: String,
    pub detail: String,
    pub acknowledged: bool,
    pub created_at: String,
    pub acknowledged_at: Option<String>,
    pub acknowledged_by: Option<String>,
    pub note: Option<String>,
    pub ttl_at: i64,
}

repository! {
    pub trait ActivityRepository("activity") {
        /// The counters after the increments
        fn add_to_daily(&self, tx: &mut Tx<'_>, account: &AccountId, day: &str, increments: &[(String, f64)],
            ttl_at: i64) -> Json;
        fn find_daily_rows(&self, conn: &mut AnyConnection, account: &AccountId, from_day: &str, to_day: &str,
            limit: u32) -> Vec<Json>;
        fn find_daily_rows_in_range(&self, conn: &mut AnyConnection, from_day: &str, to_day: &str, limit: u32)
            -> Vec<Json>;
        fn insert_hunt_if_absent(&self, tx: &mut Tx<'_>, record: &ActivityHuntRecord) -> bool;
        fn find_hunt(&self, conn: &mut AnyConnection, id: &str) -> Option<ActivityHuntRecord>;
        fn find_open_dedicated_hunts(&self, conn: &mut AnyConnection, account: &AccountId) -> Vec<ActivityHuntRecord>;
        fn find_dedicated_hunts_covering(&self, conn: &mut AnyConnection, account: &AccountId, at: &str,
            not_before: &str, grace_floor: &str) -> Vec<ActivityHuntRecord>;
        fn close_hunt_if_open(&self, tx: &mut Tx<'_>, id: &str, ended_at: &str, end_reason: &str, result: HuntResult)
            -> bool;
        fn add_hunt_kills(&self, tx: &mut Tx<'_>, id: &str, kills: u64) -> ();
        fn find_hunts_for_account(&self, conn: &mut AnyConnection, account: &AccountId, limit: u32)
            -> Vec<ActivityHuntRecord>;
        fn insert_unlock_if_absent(&self, tx: &mut Tx<'_>, record: &SlayersPathUnlockRecord) -> bool;
        fn find_unlocks_for_account(&self, conn: &mut AnyConnection, account: &AccountId, limit: u32)
            -> Vec<SlayersPathUnlockRecord>;
        fn insert_flag_if_absent(&self, tx: &mut Tx<'_>, record: &ActivityFlagRecord) -> bool;
        fn find_flag(&self, conn: &mut AnyConnection, id: &str) -> Option<ActivityFlagRecord>;
        /// `None` is every status; the records and the total
        fn list_flags(&self, conn: &mut AnyConnection, acknowledged: Option<bool>, skip: u32, limit: u32)
            -> (Vec<ActivityFlagRecord>, u64);
        fn find_flags_for_account(&self, conn: &mut AnyConnection, account: &AccountId, limit: u32)
            -> Vec<ActivityFlagRecord>;
        fn acknowledge_flag(&self, tx: &mut Tx<'_>, id: &str, by: &AccountId, at: &str, note: Option<&str>)
            -> Option<ActivityFlagRecord>;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardRole {
    Client,
    Host,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardStatus {
    Challenged,
    Healthy,
    Degraded,
    Revoked,
    Expired,
}

/// `LauncherGuardSessionRecord`. The heartbeat signals and history stay JSON until the area types them.
#[derive(Debug, Clone, PartialEq)]
pub struct LauncherGuardSessionRecord {
    pub guard_session_id: String,
    pub account: AccountId,
    pub role: GuardRole,
    pub public_key_base64: String,
    pub manifest_id: String,
    pub manifest_sequence: u64,
    pub channel: String,
    pub launcher_version: String,
    pub challenge: String,
    pub last_sequence: u64,
    pub status: GuardStatus,
    pub risk_score: f64,
    pub created_at: String,
    pub expires_at: String,
    pub last_heartbeat_at: Option<String>,
    pub last_signals: Option<Json>,
    pub reasons: Vec<String>,
    pub history: Vec<Json>,
}

/// `LauncherGuardRepository.advance`'s input.
#[derive(Debug, Clone, PartialEq)]
pub struct GuardHeartbeat {
    pub guard_session_id: String,
    pub account: AccountId,
    pub expected_challenge: String,
    pub sequence: u64,
    pub next_challenge: String,
    pub status: GuardStatus,
    pub risk_score: f64,
    pub heartbeat_at: String,
    pub expires_at: String,
    pub signals: Json,
    pub reasons: Vec<String>,
    pub history_entry: Json,
}

/// `GuardFindingRecord`: an unexpected module seen in players' games.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardFinding {
    pub file_name: String,
    pub sha256: String,
    pub signer: String,
    pub size: u64,
    pub first_seen_at: String,
    pub last_seen_at: String,
    pub session_count: u64,
    pub account_count: u64,
}

/// `GuardFindingSighting`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardSighting {
    pub file_name: String,
    pub sha256: String,
    pub signer: String,
    pub size: u64,
    pub account: AccountId,
    pub seen_at: String,
    pub new_in_session: bool,
}

repository! {
    pub trait LauncherGuardRepository("launcherGuard") {
        fn create(&self, tx: &mut Tx<'_>, record: &LauncherGuardSessionRecord) -> ();
        fn find(&self, conn: &mut AnyConnection, guard_session_id: &str) -> Option<LauncherGuardSessionRecord>;
        fn find_healthy_for_account(&self, conn: &mut AnyConnection, account: &AccountId, guard_session_id: &str,
            role: GuardRole, now: &str) -> Option<LauncherGuardSessionRecord>;
        fn advance(&self, tx: &mut Tx<'_>, heartbeat: &GuardHeartbeat) -> Option<LauncherGuardSessionRecord>;
        fn revoke_for_account(&self, tx: &mut Tx<'_>, account: &AccountId, role: GuardRole, revoked_at: &str) -> ();
        /// The records and the total
        fn list_sessions(&self, conn: &mut AnyConnection, status: Option<GuardStatus>, min_risk: Option<f64>,
            skip: u32, limit: u32) -> (Vec<LauncherGuardSessionRecord>, u64);
        fn list_findings(&self, conn: &mut AnyConnection, signer: Option<&str>, skip: u32, limit: u32)
            -> (Vec<GuardFinding>, u64);
        fn record_finding_sighting(&self, tx: &mut Tx<'_>, sighting: &GuardSighting) -> ();
    }
}

/// `InviteRecord` (`reforgedApi.ts`): a code a host made for registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteRecord {
    pub code: String,
    pub uses_remaining: i64,
    pub infinite_uses: bool,
    pub note: Option<String>,
    pub created_at: String,
}

repository! {
    pub trait InviteRepository("reforgedInvites") {
        /// `false` when the code exists (Mongo's duplicate key)
        fn insert_if_absent(&self, tx: &mut Tx<'_>, record: &InviteRecord) -> bool;
        /// Takes one use of a code that has one (an unlimited code stays as it is); `false` when it has none
        fn spend(&self, tx: &mut Tx<'_>, code: &str) -> bool;
        /// Newest first
        fn list(&self, conn: &mut AnyConnection, limit: u32) -> Vec<InviteRecord>;
        fn delete(&self, tx: &mut Tx<'_>, code: &str) -> bool;
    }
}

diesel::table! {
    reforged_invites (code) {
        code -> Text,
        uses_remaining -> BigInt,
        infinite_uses -> Bool,
        note -> Nullable<Text>,
        created_at -> Text,
    }
}

type InviteRow = (String, i64, bool, Option<String>, String);

fn invite((code, uses_remaining, infinite_uses, note, created_at): InviteRow) -> InviteRecord {
    InviteRecord {
        code,
        uses_remaining,
        infinite_uses,
        note,
        created_at,
    }
}

/// [`InviteRepository`] on `store`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SqlInvites;

impl InviteRepository for SqlInvites {
    fn insert_if_absent(&self, tx: &mut Tx<'_>, record: &InviteRecord) -> RepoResult<bool> {
        let inserted = diesel::insert_into(reforged_invites::table)
            .values((
                reforged_invites::code.eq(&record.code),
                reforged_invites::uses_remaining.eq(record.uses_remaining),
                reforged_invites::infinite_uses.eq(record.infinite_uses),
                reforged_invites::note.eq(&record.note),
                reforged_invites::created_at.eq(&record.created_at),
            ))
            .execute(tx.conn());
        match inserted {
            Ok(_) => Ok(true),
            // Mongo's duplicate key; a Postgres transaction is spoiled by it, so the caller retries in a new one
            Err(diesel::result::Error::DatabaseError(diesel::result::DatabaseErrorKind::UniqueViolation, _)) => {
                Ok(false)
            }
            Err(error) => Err(error.into()),
        }
    }

    fn spend(&self, tx: &mut Tx<'_>, code: &str) -> RepoResult<bool> {
        let unlimited = reforged_invites::table
            .filter(reforged_invites::code.eq(code))
            .filter(reforged_invites::infinite_uses.eq(true))
            .count()
            .get_result::<i64>(tx.conn())?;
        if unlimited > 0 {
            return Ok(true);
        }
        // One statement, so two registrations can't both take the last use
        let taken = diesel::update(
            reforged_invites::table
                .filter(reforged_invites::code.eq(code))
                .filter(reforged_invites::uses_remaining.gt(0)),
        )
        .set(reforged_invites::uses_remaining.eq(reforged_invites::uses_remaining - 1))
        .execute(tx.conn())?;
        Ok(taken == 1)
    }

    fn list(&self, conn: &mut AnyConnection, limit: u32) -> RepoResult<Vec<InviteRecord>> {
        Ok(reforged_invites::table
            .order((reforged_invites::created_at.desc(), reforged_invites::code.asc()))
            .limit(i64::from(limit))
            .select((
                reforged_invites::code,
                reforged_invites::uses_remaining,
                reforged_invites::infinite_uses,
                reforged_invites::note,
                reforged_invites::created_at,
            ))
            .load::<InviteRow>(conn)?
            .into_iter()
            .map(invite)
            .collect())
    }

    fn delete(&self, tx: &mut Tx<'_>, code: &str) -> RepoResult<bool> {
        let gone =
            diesel::delete(reforged_invites::table.filter(reforged_invites::code.eq(code))).execute(tx.conn())?;
        Ok(gone == 1)
    }
}

// ---------------------------------------------------------------- Launcher Guard on store

/// The last heartbeats a session keeps, newest first.
pub const GUARD_HISTORY_LIMIT: usize = 20;
/// Distinct accounts a finding counts exactly.
pub const GUARD_FINDING_ACCOUNT_LIMIT: usize = 1000;
/// `GUARD_SESSION_RETENTION_DAYS` and `GUARD_FINDING_RETENTION_DAYS` (`legal/retentionPolicy.ts`).
pub const GUARD_RETENTION_DAYS: i64 = 90;
const DAY_MS: i64 = 86_400_000;

diesel::table! {
    launcher_guard_sessions (guard_session_id) {
        guard_session_id -> Text,
        account -> Text,
        role -> Text,
        public_key_base64 -> Text,
        manifest_id -> Text,
        manifest_sequence -> BigInt,
        channel -> Text,
        challenge -> Text,
        last_sequence -> BigInt,
        status -> Text,
        risk_score -> Double,
        created_at -> Text,
        expires_at -> Text,
        last_heartbeat_at -> Nullable<Text>,
        detail -> Text,
        ttl_at -> BigInt,
    }
}

diesel::table! {
    guard_findings (id) {
        id -> BigInt,
        file_name -> Text,
        sha256 -> Text,
        signer -> Text,
        size -> BigInt,
        first_seen_at -> Text,
        last_seen_at -> Text,
        session_count -> BigInt,
        account_count -> BigInt,
        account_ids -> Text,
        ttl_at -> BigInt,
    }
}

diesel::define_sql_function!(fn lower(x: diesel::sql_types::Text) -> diesel::sql_types::Text);

type SessionRow = (
    String,
    String,
    String,
    String,
    String,
    i64,
    String,
    String,
    i64,
    String,
    f64,
    String,
    String,
    Option<String>,
    String,
    i64,
);

impl GuardRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Host => "host",
        }
    }
}

impl GuardStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Challenged => "CHALLENGED",
            Self::Healthy => "HEALTHY",
            Self::Degraded => "DEGRADED",
            Self::Revoked => "REVOKED",
            Self::Expired => "EXPIRED",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "CHALLENGED" => Self::Challenged,
            "HEALTHY" => Self::Healthy,
            "DEGRADED" => Self::Degraded,
            "REVOKED" => Self::Revoked,
            "EXPIRED" => Self::Expired,
            _ => return None,
        })
    }
}

const LIVE: [&str; 3] = ["CHALLENGED", "HEALTHY", "DEGRADED"];

fn data(what: &str, detail: impl std::fmt::Display) -> super::RepoError {
    super::RepoError::Data(format!("{what}: {detail}"))
}

/// The JSON column: what Node kept as separate document fields.
fn detail_json(record: &LauncherGuardSessionRecord) -> String {
    serde_json::json!({
        "launcherVersion": record.launcher_version,
        "lastSignals": record.last_signals,
        "reasons": record.reasons,
        "history": record.history,
    })
    .to_string()
}

fn session(row: SessionRow) -> RepoResult<LauncherGuardSessionRecord> {
    let (
        guard_session_id,
        account,
        role,
        public_key_base64,
        manifest_id,
        manifest_sequence,
        channel,
        challenge,
        last_sequence,
        status,
        risk_score,
        created_at,
        expires_at,
        last_heartbeat_at,
        detail,
        _ttl,
    ) = row;
    let detail = serde_json::from_str::<Json>(&detail).map_err(|e| data("launcher_guard_sessions.detail", e))?;
    let list = |name: &str| match detail.get(name) {
        Some(Json::Array(items)) => items.clone(),
        _ => Vec::new(),
    };
    Ok(LauncherGuardSessionRecord {
        guard_session_id,
        account: account
            .parse()
            .map_err(|_| data("launcher_guard_sessions.account", &account))?,
        role: if role == "host" {
            GuardRole::Host
        } else {
            GuardRole::Client
        },
        public_key_base64,
        manifest_id,
        manifest_sequence: u64::try_from(manifest_sequence).unwrap_or(0),
        channel,
        launcher_version: detail
            .get("launcherVersion")
            .and_then(Json::as_str)
            .unwrap_or_default()
            .to_owned(),
        challenge,
        last_sequence: u64::try_from(last_sequence).unwrap_or(0),
        status: GuardStatus::parse(&status).ok_or_else(|| data("launcher_guard_sessions.status", &status))?,
        risk_score,
        created_at,
        expires_at,
        last_heartbeat_at,
        last_signals: detail.get("lastSignals").filter(|v| !v.is_null()).cloned(),
        reasons: list("reasons")
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        history: list("history"),
    })
}

fn ttl_after(iso: &str) -> i64 {
    service_kit::js::date_parse(iso).unwrap_or(0) + GUARD_RETENTION_DAYS * DAY_MS
}

macro_rules! session_columns {
    () => {{
        use launcher_guard_sessions::*;
        (
            guard_session_id,
            account,
            role,
            public_key_base64,
            manifest_id,
            manifest_sequence,
            channel,
            challenge,
            last_sequence,
            status,
            risk_score,
            created_at,
            expires_at,
            last_heartbeat_at,
            detail,
            ttl_at,
        )
    }};
}

/// [`LauncherGuardRepository`] on `store`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SqlLauncherGuard;

impl LauncherGuardRepository for SqlLauncherGuard {
    fn create(&self, tx: &mut Tx<'_>, record: &LauncherGuardSessionRecord) -> RepoResult<()> {
        use launcher_guard_sessions as t;
        diesel::insert_into(t::table)
            .values((
                t::guard_session_id.eq(&record.guard_session_id),
                t::account.eq(record.account.to_string()),
                t::role.eq(record.role.as_str()),
                t::public_key_base64.eq(&record.public_key_base64),
                t::manifest_id.eq(&record.manifest_id),
                t::manifest_sequence.eq(i64::try_from(record.manifest_sequence).unwrap_or(i64::MAX)),
                t::channel.eq(&record.channel),
                t::challenge.eq(&record.challenge),
                t::last_sequence.eq(i64::try_from(record.last_sequence).unwrap_or(i64::MAX)),
                t::status.eq(record.status.as_str()),
                t::risk_score.eq(record.risk_score),
                t::created_at.eq(&record.created_at),
                t::expires_at.eq(&record.expires_at),
                t::last_heartbeat_at.eq(&record.last_heartbeat_at),
                t::detail.eq(detail_json(record)),
                t::ttl_at.eq(ttl_after(&record.created_at)),
            ))
            .execute(tx.conn())?;
        Ok(())
    }

    fn find(&self, conn: &mut AnyConnection, guard_session_id: &str) -> RepoResult<Option<LauncherGuardSessionRecord>> {
        use launcher_guard_sessions as t;
        t::table
            .filter(t::guard_session_id.eq(guard_session_id))
            .select(session_columns!())
            .first::<SessionRow>(conn)
            .optional()?
            .map(session)
            .transpose()
    }

    fn find_healthy_for_account(
        &self,
        conn: &mut AnyConnection,
        account: &AccountId,
        guard_session_id: &str,
        role: GuardRole,
        now: &str,
    ) -> RepoResult<Option<LauncherGuardSessionRecord>> {
        use launcher_guard_sessions as t;
        let now_ms = service_kit::js::date_parse(now).ok_or_else(|| data("now", now))?;
        // The heartbeat must be within 45 seconds of `now`
        let floor = crate::areas::reforged_api::util::iso(now_ms - 45_000);
        t::table
            .filter(t::guard_session_id.eq(guard_session_id))
            .filter(t::account.eq(account.to_string()))
            .filter(t::role.eq(role.as_str()))
            .filter(t::status.eq("HEALTHY"))
            .filter(t::expires_at.gt(now))
            .filter(t::last_heartbeat_at.gt(floor))
            .select(session_columns!())
            .first::<SessionRow>(conn)
            .optional()?
            .map(session)
            .transpose()
    }

    fn advance(&self, tx: &mut Tx<'_>, heartbeat: &GuardHeartbeat) -> RepoResult<Option<LauncherGuardSessionRecord>> {
        use launcher_guard_sessions as t;
        let sequence = i64::try_from(heartbeat.sequence).unwrap_or(i64::MAX);
        let current = t::table
            .filter(t::guard_session_id.eq(&heartbeat.guard_session_id))
            .filter(t::account.eq(heartbeat.account.to_string()))
            .filter(t::challenge.eq(&heartbeat.expected_challenge))
            .filter(t::last_sequence.lt(sequence))
            .filter(t::status.eq_any(LIVE))
            .filter(t::expires_at.gt(&heartbeat.heartbeat_at))
            .select(t::detail)
            .first::<String>(tx.conn())
            .optional()?;
        let Some(detail) = current else {
            return Ok(None);
        };
        let mut detail =
            serde_json::from_str::<Json>(&detail).map_err(|e| data("launcher_guard_sessions.detail", e))?;
        let mut entries = vec![heartbeat.history_entry.clone()];
        if let Some(Json::Array(old)) = detail.get("history") {
            entries.extend(old.iter().cloned());
        }
        entries.truncate(GUARD_HISTORY_LIMIT);
        if let Json::Object(map) = &mut detail {
            map.insert("lastSignals".into(), heartbeat.signals.clone());
            map.insert("reasons".into(), serde_json::json!(heartbeat.reasons));
            map.insert("history".into(), Json::Array(entries));
        }
        // The condition is repeated in the update, so a raced heartbeat changes nothing
        let changed = diesel::update(
            t::table
                .filter(t::guard_session_id.eq(&heartbeat.guard_session_id))
                .filter(t::challenge.eq(&heartbeat.expected_challenge))
                .filter(t::last_sequence.lt(sequence)),
        )
        .set((
            t::challenge.eq(&heartbeat.next_challenge),
            t::last_sequence.eq(sequence),
            t::status.eq(heartbeat.status.as_str()),
            t::risk_score.eq(heartbeat.risk_score),
            t::last_heartbeat_at.eq(Some(&heartbeat.heartbeat_at)),
            t::expires_at.eq(&heartbeat.expires_at),
            t::detail.eq(detail.to_string()),
            t::ttl_at.eq(ttl_after(&heartbeat.heartbeat_at)),
        ))
        .execute(tx.conn())?;
        if changed == 0 {
            return Ok(None);
        }
        t::table
            .filter(t::guard_session_id.eq(&heartbeat.guard_session_id))
            .select(session_columns!())
            .first::<SessionRow>(tx.conn())
            .optional()?
            .map(session)
            .transpose()
    }

    fn revoke_for_account(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        role: GuardRole,
        revoked_at: &str,
    ) -> RepoResult<()> {
        use launcher_guard_sessions as t;
        diesel::update(
            t::table
                .filter(t::account.eq(account.to_string()))
                .filter(t::role.eq(role.as_str()))
                .filter(t::status.eq_any(LIVE)),
        )
        .set((
            t::status.eq("REVOKED"),
            t::expires_at.eq(revoked_at),
            t::ttl_at.eq(ttl_after(revoked_at)),
        ))
        .execute(tx.conn())?;
        Ok(())
    }

    fn list_sessions(
        &self,
        conn: &mut AnyConnection,
        status: Option<GuardStatus>,
        min_risk: Option<f64>,
        skip: u32,
        limit: u32,
    ) -> RepoResult<(Vec<LauncherGuardSessionRecord>, u64)> {
        use launcher_guard_sessions as t;
        let filtered = || {
            let mut query = t::table.into_boxed();
            if let Some(status) = status {
                query = query.filter(t::status.eq(status.as_str()));
            }
            if let Some(min) = min_risk {
                query = query.filter(t::risk_score.ge(min));
            }
            query
        };
        let total = filtered().count().get_result::<i64>(conn)?;
        let rows = filtered()
            .order(t::created_at.desc())
            .limit(i64::from(limit))
            .offset(i64::from(skip))
            .select(session_columns!())
            .load::<SessionRow>(conn)?;
        // The list leaves the history out: it is read per session
        let records = rows
            .into_iter()
            .map(|row| {
                session(row).map(|mut record| {
                    record.history.clear();
                    record
                })
            })
            .collect::<RepoResult<Vec<_>>>()?;
        Ok((records, u64::try_from(total).unwrap_or(0)))
    }

    fn list_findings(
        &self,
        conn: &mut AnyConnection,
        signer: Option<&str>,
        skip: u32,
        limit: u32,
    ) -> RepoResult<(Vec<GuardFinding>, u64)> {
        use guard_findings as t;
        let pattern = signer.filter(|s| !s.is_empty()).map(|s| {
            format!(
                "%{}%",
                s.to_lowercase()
                    .replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_")
            )
        });
        let filtered = || {
            let mut query = t::table.into_boxed();
            if let Some(pattern) = &pattern {
                query = query.filter(lower(t::signer).like(pattern.clone()).escape('\\'));
            }
            query
        };
        let total = filtered().count().get_result::<i64>(conn)?;
        let rows = filtered()
            .order((t::session_count.desc(), t::last_seen_at.desc(), t::id.desc()))
            .limit(i64::from(limit))
            .offset(i64::from(skip))
            .select((
                t::file_name,
                t::sha256,
                t::signer,
                t::size,
                t::first_seen_at,
                t::last_seen_at,
                t::session_count,
                t::account_count,
            ))
            .load::<(String, String, String, i64, String, String, i64, i64)>(conn)?;
        let n = |v: i64| u64::try_from(v).unwrap_or(0);
        Ok((
            rows.into_iter()
                .map(
                    |(file_name, sha256, signer, size, first_seen_at, last_seen_at, session_count, account_count)| {
                        GuardFinding {
                            file_name,
                            sha256,
                            signer,
                            size: n(size),
                            first_seen_at,
                            last_seen_at,
                            session_count: n(session_count),
                            account_count: n(account_count),
                        }
                    },
                )
                .collect(),
            n(total),
        ))
    }

    fn record_finding_sighting(&self, tx: &mut Tx<'_>, sighting: &GuardSighting) -> RepoResult<()> {
        use guard_findings as t;
        let ttl = ttl_after(&sighting.seen_at);
        let account = sighting.account.to_string();
        let size = i64::try_from(sighting.size).unwrap_or(i64::MAX);
        let seen = i64::from(sighting.new_in_session);
        let existing = t::table
            .filter(t::sha256.eq(&sighting.sha256))
            .filter(t::file_name.eq(&sighting.file_name))
            .select((t::id, t::session_count, t::account_ids))
            .first::<(i64, i64, String)>(tx.conn())
            .optional()?;
        let Some((id, sessions, accounts)) = existing else {
            diesel::insert_into(t::table)
                .values((
                    t::file_name.eq(&sighting.file_name),
                    t::sha256.eq(&sighting.sha256),
                    t::signer.eq(&sighting.signer),
                    t::size.eq(size),
                    t::first_seen_at.eq(&sighting.seen_at),
                    t::last_seen_at.eq(&sighting.seen_at),
                    t::session_count.eq(seen),
                    t::account_count.eq(1),
                    t::account_ids.eq(serde_json::to_string(&[&account]).unwrap_or_else(|_| "[]".into())),
                    t::ttl_at.eq(ttl),
                ))
                .execute(tx.conn())?;
            return Ok(());
        };
        let mut accounts: Vec<String> = serde_json::from_str(&accounts).unwrap_or_default();
        if !accounts.contains(&account) && accounts.len() < GUARD_FINDING_ACCOUNT_LIMIT {
            accounts.push(account);
        }
        diesel::update(t::table.filter(t::id.eq(id)))
            .set((
                t::signer.eq(&sighting.signer),
                t::size.eq(size),
                t::last_seen_at.eq(&sighting.seen_at),
                t::ttl_at.eq(ttl),
                t::session_count.eq(sessions + seen),
                t::account_count.eq(i64::try_from(accounts.len()).unwrap_or(i64::MAX)),
                t::account_ids.eq(serde_json::to_string(&accounts).unwrap_or_else(|_| "[]".into())),
            ))
            .execute(tx.conn())?;
        Ok(())
    }
}

// ---------------------------------------------------------------- the admin panel and the notices on store

diesel::table! {
    admin_sessions (id) {
        id -> Text,
        token_hash -> Text,
        account -> Text,
        csrf_token -> Text,
        created_at -> Text,
        expires_at -> Text,
        ip -> Text,
        user_agent -> Text,
        revoked_at -> Nullable<Text>,
        ttl_at -> BigInt,
    }
}

diesel::table! {
    admin_audit (seq) {
        seq -> BigInt,
        id -> Text,
        actor_user_id -> Text,
        target_user_id -> Nullable<Text>,
        action -> Text,
        old_state -> Nullable<Text>,
        new_state -> Nullable<Text>,
        reason -> Nullable<Text>,
        ip -> Text,
        request_id -> Text,
        created_at -> Text,
    }
}

diesel::table! {
    launcher_notices (id) {
        id -> BigInt,
        version -> BigInt,
        updated_at -> Nullable<Text>,
        updated_by -> Nullable<Text>,
        announcements -> Text,
        tags -> Text,
    }
}

type AuditRow = (
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
    String,
    String,
);

fn audit(row: AuditRow) -> RepoResult<AdminAuditRecord> {
    let (id, actor_user_id, target_user_id, action, old_state, new_state, reason, ip, request_id, created_at) = row;
    let state = |text: Option<String>, what: &str| {
        text.map(|t| serde_json::from_str::<Json>(&t).map_err(|e| data(what, e)))
            .transpose()
    };
    Ok(AdminAuditRecord {
        id,
        actor_user_id,
        target_user_id,
        action,
        old_state: state(old_state, "admin_audit.old_state")?,
        new_state: state(new_state, "admin_audit.new_state")?,
        reason,
        ip,
        request_id,
        created_at,
    })
}

/// [`AdminRepository`] on `store`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SqlAdmin;

/// A session lives until it expires, and its row is purged a day after (the Mongo TTL index of `ttlAt`).
fn session_ttl(expires_at: &str) -> i64 {
    service_kit::js::date_parse(expires_at).unwrap_or(0) + DAY_MS
}

fn insert_audit(tx: &mut Tx<'_>, record: &AdminAuditRecord) -> RepoResult<()> {
    use admin_audit as t;
    diesel::insert_into(t::table)
        .values((
            t::id.eq(&record.id),
            t::actor_user_id.eq(&record.actor_user_id),
            t::target_user_id.eq(&record.target_user_id),
            t::action.eq(&record.action),
            t::old_state.eq(record.old_state.as_ref().map(Json::to_string)),
            t::new_state.eq(record.new_state.as_ref().map(Json::to_string)),
            t::reason.eq(&record.reason),
            t::ip.eq(&record.ip),
            t::request_id.eq(&record.request_id),
            t::created_at.eq(&record.created_at),
        ))
        .execute(tx.conn())?;
    Ok(())
}

impl AdminRepository for SqlAdmin {
    fn create_session(&self, tx: &mut Tx<'_>, session: &AdminSessionRecord) -> RepoResult<()> {
        use admin_sessions as t;
        diesel::insert_into(t::table)
            .values((
                t::id.eq(&session.id),
                t::token_hash.eq(&session.token_hash),
                t::account.eq(session.account.to_string()),
                t::csrf_token.eq(&session.csrf_token),
                t::created_at.eq(&session.created_at),
                t::expires_at.eq(&session.expires_at),
                t::ip.eq(&session.ip),
                t::user_agent.eq(&session.user_agent),
                t::revoked_at.eq(&session.revoked_at),
                t::ttl_at.eq(session_ttl(&session.expires_at)),
            ))
            .execute(tx.conn())?;
        Ok(())
    }

    fn find_active_session_by_token_hash(
        &self,
        conn: &mut AnyConnection,
        token_hash: &str,
        now: &str,
    ) -> RepoResult<Option<AdminSessionRecord>> {
        use admin_sessions as t;
        let row = t::table
            .filter(t::token_hash.eq(token_hash))
            .filter(t::revoked_at.is_null())
            .filter(t::expires_at.gt(now))
            .select((
                t::id,
                t::token_hash,
                t::account,
                t::csrf_token,
                t::created_at,
                t::expires_at,
                t::ip,
                t::user_agent,
            ))
            .first::<(String, String, String, String, String, String, String, String)>(conn)
            .optional()?;
        row.map(
            |(id, token_hash, account, csrf_token, created_at, expires_at, ip, user_agent)| {
                Ok(AdminSessionRecord {
                    id,
                    token_hash,
                    account: account.parse().map_err(|_| data("admin_sessions.account", &account))?,
                    csrf_token,
                    created_at,
                    expires_at,
                    ip,
                    user_agent,
                    revoked_at: None,
                })
            },
        )
        .transpose()
    }

    fn revoke_session(&self, tx: &mut Tx<'_>, id: &str, at: &str) -> RepoResult<()> {
        use admin_sessions as t;
        diesel::update(t::table.filter(t::id.eq(id)))
            .set(t::revoked_at.eq(Some(at)))
            .execute(tx.conn())?;
        Ok(())
    }

    fn append_audit(&self, tx: &mut Tx<'_>, record: &AdminAuditRecord) -> RepoResult<()> {
        insert_audit(tx, record)
    }

    fn list_audit(
        &self,
        conn: &mut AnyConnection,
        target_user_id: Option<&str>,
        skip: u32,
        limit: u32,
    ) -> RepoResult<(Vec<AdminAuditRecord>, u64)> {
        use admin_audit as t;
        let target = target_user_id.filter(|t| !t.is_empty());
        let filtered = || {
            let mut query = t::table.into_boxed();
            if let Some(target) = target {
                query = query.filter(t::target_user_id.eq(target));
            }
            query
        };
        let total = filtered().count().get_result::<i64>(conn)?;
        let rows = filtered()
            .order((t::created_at.desc(), t::seq.desc()))
            .limit(i64::from(limit))
            .offset(i64::from(skip))
            .select((
                t::id,
                t::actor_user_id,
                t::target_user_id,
                t::action,
                t::old_state,
                t::new_state,
                t::reason,
                t::ip,
                t::request_id,
                t::created_at,
            ))
            .load::<AuditRow>(conn)?;
        Ok((
            rows.into_iter().map(audit).collect::<RepoResult<Vec<_>>>()?,
            u64::try_from(total).unwrap_or(0),
        ))
    }

    fn delete_player_data(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        user_id: &str,
        record: &AdminAuditRecord,
    ) -> RepoResult<PlayerDeletion> {
        use super::reforged::{
            admin_sessions as sessions, guard_findings as findings, launcher_guard_sessions as guard,
        };
        let key = account.to_string();
        let mut counts = Vec::new();
        let n = |changed: usize| u64::try_from(changed).unwrap_or(0);
        // The account itself is soft-deleted (DD23), so a revert brings the player back
        let soft = tx.soft_delete("accounts", &[("id", ::store::KeyValue::Text(key.clone()))])?;
        counts.push(("accounts".to_owned(), u64::from(soft)));
        counts.push((
            "adminSessions".to_owned(),
            n(diesel::delete(sessions::table.filter(sessions::account.eq(&key))).execute(tx.conn())?),
        ));
        counts.push((
            "launcherGuardSessions".to_owned(),
            n(diesel::delete(guard::table.filter(guard::account.eq(&key))).execute(tx.conn())?),
        ));
        // The account leaves every Guard module finding that lists it, and the count follows
        let listed = findings::table
            .select((findings::id, findings::account_ids))
            .load::<(i64, String)>(tx.conn())?;
        let mut touched = 0u64;
        for (id, accounts) in listed {
            let accounts: Vec<String> = serde_json::from_str(&accounts).unwrap_or_default();
            if !accounts.contains(&key) {
                continue;
            }
            let rest: Vec<String> = accounts.into_iter().filter(|a| a != &key).collect();
            diesel::update(findings::table.filter(findings::id.eq(id)))
                .set((
                    findings::account_ids.eq(serde_json::to_string(&rest).unwrap_or_else(|_| "[]".into())),
                    findings::account_count.eq(i64::try_from(rest.len()).unwrap_or(i64::MAX)),
                ))
                .execute(tx.conn())?;
            touched += 1;
        }
        counts.push(("guardFindings".to_owned(), touched));
        let _ = user_id;
        insert_audit(tx, record)?;
        Ok(PlayerDeletion {
            deleted_counts: counts,
            character_count: 0,
        })
    }
}

/// [`LauncherNoticesRepository`] on `store`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SqlLauncherNotices;

fn list_of(text: &str) -> Vec<Json> {
    match serde_json::from_str::<Json>(text) {
        Ok(Json::Array(items)) => items,
        _ => Vec::new(),
    }
}

impl LauncherNoticesRepository for SqlLauncherNotices {
    fn get(&self, conn: &mut AnyConnection) -> RepoResult<Option<LauncherNoticesRecord>> {
        use launcher_notices as t;
        let row = t::table
            .filter(t::id.eq(1))
            .select((t::version, t::updated_at, t::updated_by, t::announcements, t::tags))
            .first::<(i64, Option<String>, Option<String>, String, String)>(conn)
            .optional()?;
        Ok(row.map(
            |(version, updated_at, updated_by, announcements, tags)| LauncherNoticesRecord {
                version: u64::try_from(version).unwrap_or(0),
                updated_at,
                updated_by,
                announcements: list_of(&announcements),
                tags: list_of(&tags),
            },
        ))
    }

    fn replace_if_version(
        &self,
        tx: &mut Tx<'_>,
        record: &LauncherNoticesRecord,
        expected_version: u64,
    ) -> RepoResult<bool> {
        use launcher_notices as t;
        let expected = i64::try_from(expected_version).unwrap_or(i64::MAX);
        let version = i64::try_from(record.version).unwrap_or(i64::MAX);
        let announcements = Json::Array(record.announcements.clone()).to_string();
        let tags = Json::Array(record.tags.clone()).to_string();
        let exists = t::table.filter(t::id.eq(1)).count().get_result::<i64>(tx.conn())? > 0;
        if !exists {
            if expected != 0 {
                return Ok(false);
            }
            diesel::insert_into(t::table)
                .values((
                    t::id.eq(1),
                    t::version.eq(version),
                    t::updated_at.eq(&record.updated_at),
                    t::updated_by.eq(&record.updated_by),
                    t::announcements.eq(announcements),
                    t::tags.eq(tags),
                ))
                .execute(tx.conn())?;
            return Ok(true);
        }
        let changed = diesel::update(t::table.filter(t::id.eq(1)).filter(t::version.eq(expected)))
            .set((
                t::version.eq(version),
                t::updated_at.eq(&record.updated_at),
                t::updated_by.eq(&record.updated_by),
                t::announcements.eq(announcements),
                t::tags.eq(tags),
            ))
            .execute(tx.conn())?;
        Ok(changed == 1)
    }
}
