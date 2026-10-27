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

/// `AdminAuditRecord`.
#[derive(Debug, Clone, PartialEq)]
pub struct AdminAuditRecord {
    pub id: String,
    pub actor: AccountId,
    pub target: Option<AccountId>,
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
        fn find_active_session_by_token_hash(&self, conn: &mut AnyConnection, token_hash: &str)
            -> Option<AdminSessionRecord>;
        fn revoke_session(&self, tx: &mut Tx<'_>, id: &str) -> ();
        fn append_audit(&self, tx: &mut Tx<'_>, record: &AdminAuditRecord) -> ();
        /// The records and the total
        fn list_audit(&self, conn: &mut AnyConnection, target: Option<&AccountId>, skip: u32, limit: u32)
            -> (Vec<AdminAuditRecord>, u64);
        /// Player-facing rows are soft-deleted (DD23), so `revert` can bring them back
        fn delete_player_data(&self, tx: &mut Tx<'_>, account: &AccountId, audit: &AdminAuditRecord)
            -> PlayerDeletion;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeTone {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagTone {
    Info,
    Warning,
    Danger,
    Success,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherNotice {
    pub id: String,
    pub text: String,
    pub tone: NoticeTone,
    pub event_at: Option<String>,
    pub show_from: Option<String>,
    pub show_until: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherNoticeTag {
    pub id: String,
    pub text: String,
    pub tone: TagTone,
    pub show_from: Option<String>,
    pub show_until: Option<String>,
}

/// `LauncherNoticesRecord`: one document, replaced by version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherNoticesRecord {
    pub version: u64,
    pub updated_at: Option<String>,
    pub updated_by: Option<String>,
    pub announcements: Vec<LauncherNotice>,
    pub tags: Vec<LauncherNoticeTag>,
}

repository! {
    pub trait LauncherNoticesRepository("launcherNotices") {
        fn get(&self, conn: &mut AnyConnection) -> Option<LauncherNoticesRecord>;
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
