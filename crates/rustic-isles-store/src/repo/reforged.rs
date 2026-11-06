// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The Reforged API's rows (spec §11.3): admin sessions and audit, the operations policy, launcher notices,
//! activity, Launcher Guard, invites; and the erasure and TTL purge every area's rows go through.

use std::fmt;

use diesel::QueryableByName;
use diesel::result::{DatabaseErrorKind, Error as DieselError};
use diesel::sql_types::{BigInt, Bool, Double, Nullable, Text};
use serde_json::{Map, Value};
use service_kit::js::date_parse;

use crate::repo::{bounties, chat, parties};
use crate::sql::{account_id, count, page_clause, sql};
use crate::{AccountId, AnyConnection, Json, RepoError, RepoResult, Tx};

const DAY_MS: i64 = 86_400_000;

// ------------------------------------------------------------------ admin

/// An `admin_sessions` row: one signed-in admin dashboard session; Debug leaves out the secrets.
#[derive(Clone, PartialEq, Eq)]
pub struct AdminSessionRecord {
    /// The session's id.
    pub id: String,
    /// SHA-256 of the session token.
    pub token_hash: String,
    /// The admin's account.
    pub account: AccountId,
    /// The CSRF token the dashboard must echo.
    pub csrf_token: String,
    /// When it began, as ISO text.
    pub created_at: String,
    /// When it lapses, as ISO text.
    pub expires_at: String,
    /// The client address it began from.
    pub ip: String,
    /// The browser's user agent.
    pub user_agent: String,
    /// When it was revoked, as ISO text.
    pub revoked_at: Option<String>,
}

impl fmt::Debug for AdminSessionRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AdminSessionRecord")
            .field("id", &self.id)
            .field("account", &self.account)
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .field("ip", &self.ip)
            .field("user_agent", &self.user_agent)
            .field("revoked_at", &self.revoked_at)
            .finish_non_exhaustive()
    }
}

/// An `admin_audit` row: one admin action.
#[derive(Debug, Clone, PartialEq)]
pub struct AdminAuditRecord {
    /// The record's id.
    pub id: String,
    /// The admin's userId.
    pub actor_user_id: String,
    /// The player acted on, if any.
    pub target_user_id: Option<String>,
    /// What was done.
    pub action: String,
    /// The state before, if recorded.
    pub old_state: Option<Json>,
    /// The state after, if recorded.
    pub new_state: Option<Json>,
    /// The reason given.
    pub reason: Option<String>,
    /// The admin's client address.
    pub ip: String,
    /// The request's id.
    pub request_id: String,
    /// When, as ISO text.
    pub created_at: String,
}

/// The operations policy, one row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationsPolicyRecord {
    /// The guard's mode, such as `ENFORCE`.
    pub guard_enforcement: String,
    /// `PRODUCTION`, or `DEVELOPMENT` for admins' diagnostics.
    pub diagnostics_profile: String,
    /// Bumped on every change; a change names the version it read.
    pub version: u64,
    /// When it last changed, as ISO text.
    pub updated_at: Option<String>,
    /// Who last changed it.
    pub updated_by: Option<String>,
    /// When development diagnostics lapse, as ISO text.
    pub diagnostics_expires_at: Option<String>,
}

/// What deleting a player's data removed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlayerDeletion {
    /// `(area, rows)` for every table touched, in order.
    pub deleted_counts: Vec<(String, u64)>,
    /// How many characters were deleted.
    pub character_count: u64,
}

store_trait! {
    /// The Reforged API's admin data.
    pub trait AdminRepository = "admin" {
        /// Writes a new admin session.
        fn create_session(&self, tx: &mut Tx<'_>, record: &AdminSessionRecord) -> ();
        /// The unrevoked, unexpired session with that token hash.
        fn find_active_session_by_token_hash(
            &self,
            conn: &mut AnyConnection,
            hash: &str,
            now: &str,
        ) -> Option<AdminSessionRecord>;
        /// Revokes the session at `at`.
        fn revoke_session(&self, tx: &mut Tx<'_>, id: &str, at: &str) -> ();
        /// Writes an audit record.
        fn append_audit(&self, tx: &mut Tx<'_>, record: &AdminAuditRecord) -> ();
        /// The audit page, newest first, of every record or those for `target`, and the total that matched.
        fn list_audit(
            &self,
            conn: &mut AnyConnection,
            target: Option<&str>,
            skip: u32,
            limit: u32,
        ) -> (Vec<AdminAuditRecord>, u64);
        /// The operations policy, if one was ever written.
        fn read_operations_policy(&self, conn: &mut AnyConnection) -> Option<OperationsPolicyRecord>;
        /// Writes the policy if the stored version is still `expected`; returns whether it did.
        fn update_operations_policy(&self, tx: &mut Tx<'_>, expected: u64, record: &OperationsPolicyRecord) -> bool;
        /// One write: the player's rows of every area, then the audit record (`user_id` is unused)
        fn delete_player_data(
            &self,
            tx: &mut Tx<'_>,
            account: &AccountId,
            user_id: &str,
            audit: &AdminAuditRecord,
        ) -> PlayerDeletion;
    }
}

/// The launcher's notices, one row.
#[derive(Debug, Clone, PartialEq)]
pub struct LauncherNoticesRecord {
    /// Bumped on every change; a change names the version it read.
    pub version: u64,
    /// When they last changed, as ISO text.
    pub updated_at: Option<String>,
    /// Who last changed them.
    pub updated_by: Option<String>,
    /// The announcement objects.
    pub announcements: Vec<Json>,
    /// The tag objects.
    pub tags: Vec<Json>,
}

store_trait! {
    /// The launcher's notices (`launcher_notices`).
    pub trait LauncherNoticesRepository = "launcherNotices" {
        /// The notices, if ever written.
        fn get(&self, conn: &mut AnyConnection) -> Option<LauncherNoticesRecord>;
        /// Writes the notices if the stored version is still `expected` (0 when none were written); returns whether it did.
        fn replace_if_version(&self, tx: &mut Tx<'_>, record: &LauncherNoticesRecord, expected: u64) -> bool;
    }
}

// ------------------------------------------------------------------ activity

/// Where a hunt ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HuntKind {
    /// On the dedicated server.
    Dedicated,
    /// In a realm.
    Realm,
    /// On a player's own host.
    PlayerHosted,
}

impl HuntKind {
    /// The stored text.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dedicated => "dedicated",
            Self::Realm => "realm",
            Self::PlayerHosted => "playerHosted",
        }
    }

    /// The kind a stored text names; None for any other text.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "dedicated" => Some(Self::Dedicated),
            "realm" => Some(Self::Realm),
            "playerHosted" => Some(Self::PlayerHosted),
            _ => None,
        }
    }
}

/// How a hunt ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HuntResult {
    /// The hunt was finished.
    Completed,
    /// The hunt was left before the end.
    Abandoned,
}

impl HuntResult {
    /// The stored text.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Abandoned => "abandoned",
        }
    }

    /// The result a stored text names; None for any other text.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "completed" => Some(Self::Completed),
            "abandoned" => Some(Self::Abandoned),
            _ => None,
        }
    }
}

/// An `activity_hunts` row: one account's part in one hunt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityHuntRecord {
    /// The record's id.
    pub id: String,
    /// The account.
    pub account: AccountId,
    /// Whether the account hosted it.
    pub host: bool,
    /// Where it ran.
    pub kind: HuntKind,
    /// The hunt's id.
    pub hunt_id: String,
    /// The id the hunt is known by where it ran.
    pub hunt_ref: String,
    /// When it started, as ISO text.
    pub started_at: String,
    /// When it ended, as ISO text.
    pub ended_at: Option<String>,
    /// Why it ended.
    pub end_reason: Option<String>,
    /// How it ended.
    pub result: Option<HuntResult>,
    /// Kills counted so far.
    pub kills: u64,
    /// When the record is purged, in Unix milliseconds.
    pub ttl_at: i64,
}

/// A `slayers_path_unlocks` row: one node an account unlocked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlayersPathUnlockRecord {
    /// The record's id.
    pub id: String,
    /// The account.
    pub account: AccountId,
    /// The node.
    pub node_id: String,
    /// When, as ISO text.
    pub unlocked_at: String,
    /// The route that saved it.
    pub source: String,
    /// When the record is purged, in Unix milliseconds.
    pub ttl_at: i64,
}

/// An `activity_flags` row: an economy or activity threshold an account crossed.
#[derive(Debug, Clone, PartialEq)]
pub struct ActivityFlagRecord {
    /// The flag's id.
    pub id: String,
    /// Which threshold.
    pub kind: String,
    /// How serious it is.
    pub severity: String,
    /// The account.
    pub account: AccountId,
    /// The UTC day, `YYYY-MM-DD`.
    pub day: String,
    /// Stored as compact JSON
    pub detail: Json,
    /// Whether staff looked at it.
    pub acknowledged: bool,
    /// When it was raised, as ISO text.
    pub created_at: String,
    /// When it was acknowledged, as ISO text.
    pub acknowledged_at: Option<String>,
    /// The userId of the staff member who acknowledged it.
    pub acknowledged_by: Option<String>,
    /// Their note.
    pub note: Option<String>,
    /// When the record is purged, in Unix milliseconds.
    pub ttl_at: i64,
}

store_trait! {
    /// Activity records: daily counters, hunts, Slayer's Path unlocks and flags.
    pub trait ActivityRepository = "activity" {
        /// Adds each `(dotted path, amount)` to the account's day document; returns the document after.
        fn add_to_daily(
            &self,
            tx: &mut Tx<'_>,
            account: &AccountId,
            day: &str,
            increments: &[(String, f64)],
            ttl_at: i64,
        ) -> Json;
        /// `fields`' top-level keys set on the day's document
        fn set_daily_fields(&self, tx: &mut Tx<'_>, account: &AccountId, day: &str, fields: &Json, ttl_at: i64) -> Json;
        /// The account's day documents from `from` to `to`, oldest first.
        fn find_daily_rows(&self, conn: &mut AnyConnection, account: &AccountId, from: &str, to: &str, limit: u32)
            -> Vec<Json>;
        /// Every account's documents by day, then account
        fn find_daily_rows_in_range(&self, conn: &mut AnyConnection, from: &str, to: &str, limit: u32)
            -> Vec<(AccountId, Json)>;
        /// Writes the hunt unless its id exists; returns whether it did.
        fn insert_hunt_if_absent(&self, tx: &mut Tx<'_>, record: &ActivityHuntRecord) -> bool;
        /// Writes the unlock unless its id exists; returns whether it did.
        fn insert_unlock_if_absent(&self, tx: &mut Tx<'_>, record: &SlayersPathUnlockRecord) -> bool;
        /// Writes the flag unless its id exists; returns whether it did.
        fn insert_flag_if_absent(&self, tx: &mut Tx<'_>, record: &ActivityFlagRecord) -> bool;
        /// The hunt with that id.
        fn find_hunt(&self, conn: &mut AnyConnection, id: &str) -> Option<ActivityHuntRecord>;
        /// The flag with that id.
        fn find_flag(&self, conn: &mut AnyConnection, id: &str) -> Option<ActivityFlagRecord>;
        /// The account's dedicated hunts still open, newest first (at most 50).
        fn find_open_dedicated_hunts(&self, conn: &mut AnyConnection, account: &AccountId) -> Vec<ActivityHuntRecord>;
        /// The account's dedicated hunts that started between `not_before` and `at` and were open at `grace_floor`, newest first (at most 10).
        fn find_dedicated_hunts_covering(
            &self,
            conn: &mut AnyConnection,
            account: &AccountId,
            at: &str,
            not_before: &str,
            grace_floor: &str,
        ) -> Vec<ActivityHuntRecord>;
        /// Ends the hunt if it is still open; returns whether it did.
        fn close_hunt_if_open(
            &self,
            tx: &mut Tx<'_>,
            id: &str,
            ended_at: &str,
            reason: &str,
            result: Option<HuntResult>,
        ) -> bool;
        /// Adds kills to the hunt.
        fn add_hunt_kills(&self, tx: &mut Tx<'_>, id: &str, kills: u64) -> ();
        /// The account's hunts, newest first.
        fn find_hunts_for_account(&self, conn: &mut AnyConnection, account: &AccountId, limit: u32)
            -> Vec<ActivityHuntRecord>;
        /// The account's unlocks, newest first.
        fn find_unlocks_for_account(&self, conn: &mut AnyConnection, account: &AccountId, limit: u32)
            -> Vec<SlayersPathUnlockRecord>;
        /// The flags page, newest first, all or by acknowledged state, and the total that matched.
        fn list_flags(
            &self,
            conn: &mut AnyConnection,
            acknowledged: Option<bool>,
            skip: u32,
            limit: u32,
        ) -> (Vec<ActivityFlagRecord>, u64);
        /// The account's flags, newest first.
        fn find_flags_for_account(&self, conn: &mut AnyConnection, account: &AccountId, limit: u32)
            -> Vec<ActivityFlagRecord>;
        /// Acknowledges the flag once, naming the operator by userId; returns the flag after, or None when there is none.
        fn acknowledge_flag(
            &self,
            tx: &mut Tx<'_>,
            id: &str,
            by: &AccountId,
            at: &str,
            note: Option<&str>,
        ) -> Option<ActivityFlagRecord>;
    }
}

// ------------------------------------------------------------------ Launcher Guard

/// Heartbeat history entries kept per guard session.
pub const GUARD_HISTORY_LIMIT: usize = 20;
/// Accounts listed per guard finding.
pub const GUARD_FINDING_ACCOUNT_LIMIT: usize = 1000;
/// Days guard rows are kept after they lapse.
pub const GUARD_RETENTION_DAYS: i64 = 90;

/// Which launcher a guard session watches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardRole {
    /// A player's launcher.
    Client,
    /// A host's launcher.
    Host,
}

impl GuardRole {
    /// The stored text.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Host => "host",
        }
    }

    /// Anything but `host` reads as a client.
    pub fn parse(text: &str) -> Self {
        if text == "host" { Self::Host } else { Self::Client }
    }
}

/// A guard session's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardStatus {
    /// Opened, waiting for its first heartbeat.
    Challenged,
    /// The last heartbeat reported healthy.
    Healthy,
    /// The last heartbeat reported degraded.
    Degraded,
    /// Revoked; no heartbeat is accepted.
    Revoked,
    /// Lapsed; no heartbeat is accepted.
    Expired,
}

impl GuardStatus {
    /// The stored text.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Challenged => "CHALLENGED",
            Self::Healthy => "HEALTHY",
            Self::Degraded => "DEGRADED",
            Self::Revoked => "REVOKED",
            Self::Expired => "EXPIRED",
        }
    }

    /// The status a stored text names; None for any other text.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "CHALLENGED" => Some(Self::Challenged),
            "HEALTHY" => Some(Self::Healthy),
            "DEGRADED" => Some(Self::Degraded),
            "REVOKED" => Some(Self::Revoked),
            "EXPIRED" => Some(Self::Expired),
            _ => None,
        }
    }

    /// Whether heartbeats may still advance the session.
    pub fn is_live(self) -> bool {
        matches!(self, Self::Challenged | Self::Healthy | Self::Degraded)
    }
}

const LIVE_GUARD_STATUSES: &str = "('CHALLENGED', 'HEALTHY', 'DEGRADED')";

/// A `launcher_guard_sessions` row: a launcher's signed heartbeat session; Debug leaves out the key and challenge.
#[derive(Clone, PartialEq)]
pub struct LauncherGuardSessionRecord {
    /// The session's id.
    pub guard_session_id: String,
    /// The account.
    pub account: AccountId,
    /// Which launcher.
    pub role: GuardRole,
    /// The launcher's public signing key, base64.
    pub public_key_base64: String,
    /// The guard manifest the launcher runs.
    pub manifest_id: String,
    /// The manifest's sequence number.
    pub manifest_sequence: u64,
    /// The launcher's release channel.
    pub channel: String,
    /// The launcher's version.
    pub launcher_version: String,
    /// The challenge the next heartbeat must sign.
    pub challenge: String,
    /// The last heartbeat's sequence number.
    pub last_sequence: u64,
    /// The session's state.
    pub status: GuardStatus,
    /// The last heartbeat's risk score.
    pub risk_score: f64,
    /// When it was opened, as ISO text.
    pub created_at: String,
    /// When it lapses without a heartbeat, as ISO text.
    pub expires_at: String,
    /// When the last heartbeat came, as ISO text.
    pub last_heartbeat_at: Option<String>,
    /// The last heartbeat's signals.
    pub last_signals: Option<Json>,
    /// The last heartbeat's reasons.
    pub reasons: Vec<String>,
    /// Recent heartbeat entries, newest first.
    pub history: Vec<Json>,
}

impl fmt::Debug for LauncherGuardSessionRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LauncherGuardSessionRecord")
            .field("guard_session_id", &self.guard_session_id)
            .field("account", &self.account)
            .field("role", &self.role)
            .field("manifest_id", &self.manifest_id)
            .field("manifest_sequence", &self.manifest_sequence)
            .field("channel", &self.channel)
            .field("launcher_version", &self.launcher_version)
            .field("last_sequence", &self.last_sequence)
            .field("status", &self.status)
            .field("risk_score", &self.risk_score)
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .field("last_heartbeat_at", &self.last_heartbeat_at)
            .field("reasons", &self.reasons)
            .finish_non_exhaustive()
    }
}

/// One accepted heartbeat to apply; Debug leaves out the challenges.
#[derive(Clone, PartialEq)]
pub struct GuardHeartbeat {
    /// The session's id.
    pub guard_session_id: String,
    /// The account it must belong to.
    pub account: AccountId,
    /// The challenge the session must still hold.
    pub expected_challenge: String,
    /// The heartbeat's sequence; it must be above the last.
    pub sequence: u64,
    /// The challenge for the next heartbeat.
    pub next_challenge: String,
    /// The session's new state.
    pub status: GuardStatus,
    /// The new risk score.
    pub risk_score: f64,
    /// When the heartbeat came, as ISO text.
    pub heartbeat_at: String,
    /// When the session now lapses, as ISO text.
    pub expires_at: String,
    /// The heartbeat's signals.
    pub signals: Json,
    /// The heartbeat's reasons.
    pub reasons: Vec<String>,
    /// The history entry it adds.
    pub history_entry: Json,
}

impl fmt::Debug for GuardHeartbeat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GuardHeartbeat")
            .field("guard_session_id", &self.guard_session_id)
            .field("account", &self.account)
            .field("sequence", &self.sequence)
            .field("status", &self.status)
            .field("risk_score", &self.risk_score)
            .field("heartbeat_at", &self.heartbeat_at)
            .field("expires_at", &self.expires_at)
            .field("reasons", &self.reasons)
            .finish_non_exhaustive()
    }
}

/// One file guard sessions have reported, summed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardFinding {
    /// The file's name.
    pub file_name: String,
    /// The file's SHA-256.
    pub sha256: String,
    /// The signer the launcher read.
    pub signer: String,
    /// The file's size in bytes.
    pub size: u64,
    /// When it was first reported, as ISO text.
    pub first_seen_at: String,
    /// When it was last reported, as ISO text.
    pub last_seen_at: String,
    /// How many sessions reported it.
    pub session_count: u64,
    /// How many accounts reported it.
    pub account_count: u64,
}

/// One report of a file by a guard session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardSighting {
    /// The file's name.
    pub file_name: String,
    /// The file's SHA-256.
    pub sha256: String,
    /// The signer the launcher read.
    pub signer: String,
    /// The file's size in bytes.
    pub size: u64,
    /// The account that reported it.
    pub account: AccountId,
    /// When, as ISO text.
    pub seen_at: String,
    /// Whether this session reports it for the first time.
    pub new_in_session: bool,
}

store_trait! {
    /// Launcher Guard sessions and findings.
    pub trait LauncherGuardRepository = "launcherGuard" {
        /// Writes a new session.
        fn create(&self, tx: &mut Tx<'_>, record: &LauncherGuardSessionRecord) -> ();
        /// The session with that id.
        fn find(&self, conn: &mut AnyConnection, id: &str) -> Option<LauncherGuardSessionRecord>;
        /// The session if it belongs to the account and role, is healthy, unexpired, and had a heartbeat in the last 45 seconds.
        fn find_healthy_for_account(
            &self,
            conn: &mut AnyConnection,
            account: &AccountId,
            id: &str,
            role: GuardRole,
            now: &str,
        ) -> Option<LauncherGuardSessionRecord>;
        /// Applies the heartbeat if the session is live, belongs to the account, holds the expected challenge and its last sequence is below the heartbeat's; returns the session after, or None otherwise.
        fn advance(&self, tx: &mut Tx<'_>, heartbeat: &GuardHeartbeat) -> Option<LauncherGuardSessionRecord>;
        /// Revokes every live session of the account in that role.
        fn revoke_for_account(&self, tx: &mut Tx<'_>, account: &AccountId, role: GuardRole, at: &str) -> ();
        /// The sessions page, newest first, by status and minimum risk, and the total that matched.
        fn list_sessions(
            &self,
            conn: &mut AnyConnection,
            status: Option<GuardStatus>,
            min_risk: Option<f64>,
            skip: u32,
            limit: u32,
        ) -> (Vec<LauncherGuardSessionRecord>, u64);
        /// The findings page, most reported first, by signer text, and the total that matched.
        fn list_findings(
            &self,
            conn: &mut AnyConnection,
            signer: Option<&str>,
            skip: u32,
            limit: u32,
        ) -> (Vec<GuardFinding>, u64);
        /// Adds a sighting to its file's finding, creating it on the first.
        fn record_finding_sighting(&self, tx: &mut Tx<'_>, sighting: &GuardSighting) -> ();
    }
}

// ------------------------------------------------------------------ invites

/// A `reforged_invites` row: a code that lets a player register.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteRecord {
    /// The code.
    pub code: String,
    /// Registrations left.
    pub uses_remaining: i64,
    /// Whether the code never runs out.
    pub infinite_uses: bool,
    /// The note staff left on it.
    pub note: Option<String>,
    /// When it was made, as ISO text.
    pub created_at: String,
}

store_trait! {
    /// Registration invite codes (`reforged_invites`).
    pub trait InviteRepository = "reforgedInvites" {
        /// A taken code answers `false`; on Postgres that spoils the transaction, so the caller retries in a new
        /// one (spec §16 item 14)
        fn insert_if_absent(&self, tx: &mut Tx<'_>, record: &InviteRecord) -> bool;
        /// Uses the code once; false when it does not exist or is used up.
        fn spend(&self, tx: &mut Tx<'_>, code: &str) -> bool;
        /// The newest codes, at most `limit`.
        fn list(&self, conn: &mut AnyConnection, limit: u32) -> Vec<InviteRecord>;
        /// Deletes the code; returns whether it existed.
        fn delete(&self, tx: &mut Tx<'_>, code: &str) -> bool;
    }
}

// ------------------------------------------------------------------ SQL: admin

/// [`AdminRepository`] in SQL.
pub struct SqlAdmin;

#[derive(QueryableByName)]
struct AdminSessionRow {
    #[diesel(sql_type = Text)]
    id: String,
    #[diesel(sql_type = Text)]
    token_hash: String,
    #[diesel(sql_type = Text)]
    account: String,
    #[diesel(sql_type = Text)]
    csrf_token: String,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    expires_at: String,
    #[diesel(sql_type = Text)]
    ip: String,
    #[diesel(sql_type = Text)]
    user_agent: String,
}

#[derive(QueryableByName)]
struct AuditRow {
    #[diesel(sql_type = Text)]
    id: String,
    #[diesel(sql_type = Text)]
    actor_user_id: String,
    #[diesel(sql_type = Nullable<Text>)]
    target_user_id: Option<String>,
    #[diesel(sql_type = Text)]
    action: String,
    #[diesel(sql_type = Nullable<Text>)]
    old_state: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    new_state: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    reason: Option<String>,
    #[diesel(sql_type = Text)]
    ip: String,
    #[diesel(sql_type = Text)]
    request_id: String,
    #[diesel(sql_type = Text)]
    created_at: String,
}

fn parse_json(text: Option<&str>, column: &str) -> RepoResult<Option<Json>> {
    text.map(|t| serde_json::from_str(t).map_err(|e| RepoError::data(format!("{column}: {e}"))))
        .transpose()
}

impl AuditRow {
    fn decode(self) -> RepoResult<AdminAuditRecord> {
        Ok(AdminAuditRecord {
            old_state: parse_json(self.old_state.as_deref(), "admin_audit.old_state")?,
            new_state: parse_json(self.new_state.as_deref(), "admin_audit.new_state")?,
            id: self.id,
            actor_user_id: self.actor_user_id,
            target_user_id: self.target_user_id,
            action: self.action,
            reason: self.reason,
            ip: self.ip,
            request_id: self.request_id,
            created_at: self.created_at,
        })
    }
}

#[derive(QueryableByName)]
struct PolicyRow {
    #[diesel(sql_type = Text)]
    guard_enforcement: String,
    #[diesel(sql_type = Text)]
    diagnostics_profile: String,
    #[diesel(sql_type = BigInt)]
    version: i64,
    #[diesel(sql_type = Nullable<Text>)]
    updated_at: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    updated_by: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    diagnostics_expires_at: Option<String>,
}

fn stored_u64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn read_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn insert_audit(conn: &mut AnyConnection, r: &AdminAuditRecord) -> RepoResult<()> {
    sql(
        "INSERT INTO admin_audit (id, actor_user_id, target_user_id, action, old_state, new_state, reason, ip, \
         request_id, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .t(&*r.id)
    .t(&*r.actor_user_id)
    .ot(r.target_user_id.clone())
    .t(&*r.action)
    .ot(r.old_state.as_ref().map(ToString::to_string))
    .ot(r.new_state.as_ref().map(ToString::to_string))
    .ot(r.reason.clone())
    .t(&*r.ip)
    .t(&*r.request_id)
    .t(&*r.created_at)
    .run(conn)?;
    Ok(())
}

impl AdminRepository for SqlAdmin {
    fn create_session(&self, tx: &mut Tx<'_>, r: &AdminSessionRecord) -> RepoResult<()> {
        sql(
            "INSERT INTO admin_sessions (id, token_hash, account, csrf_token, created_at, expires_at, ip, user_agent, \
             revoked_at, ttl_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .t(&*r.id)
        .t(&*r.token_hash)
        .t(r.account.to_string())
        .t(&*r.csrf_token)
        .t(&*r.created_at)
        .t(&*r.expires_at)
        .t(&*r.ip)
        .t(&*r.user_agent)
        .ot(r.revoked_at.clone())
        .i(date_parse(&r.expires_at).unwrap_or(0).saturating_add(DAY_MS))
        .run(tx.conn())?;
        Ok(())
    }

    fn find_active_session_by_token_hash(
        &self,
        conn: &mut AnyConnection,
        hash: &str,
        now: &str,
    ) -> RepoResult<Option<AdminSessionRecord>> {
        let Some(row) =
            sql("SELECT * FROM admin_sessions WHERE token_hash = ? AND revoked_at IS NULL AND expires_at > ?")
                .t(hash)
                .t(now)
                .first::<AdminSessionRow>(conn)?
        else {
            return Ok(None);
        };
        Ok(Some(AdminSessionRecord {
            account: account_id(&row.account, "admin_sessions.account")?,
            id: row.id,
            token_hash: row.token_hash,
            csrf_token: row.csrf_token,
            created_at: row.created_at,
            expires_at: row.expires_at,
            ip: row.ip,
            user_agent: row.user_agent,
            revoked_at: None,
        }))
    }

    fn revoke_session(&self, tx: &mut Tx<'_>, id: &str, at: &str) -> RepoResult<()> {
        sql("UPDATE admin_sessions SET revoked_at = ? WHERE id = ?")
            .t(at)
            .t(id)
            .run(tx.conn())?;
        Ok(())
    }

    fn append_audit(&self, tx: &mut Tx<'_>, r: &AdminAuditRecord) -> RepoResult<()> {
        insert_audit(tx.conn(), r)
    }

    fn list_audit(
        &self,
        conn: &mut AnyConnection,
        target: Option<&str>,
        skip: u32,
        limit: u32,
    ) -> RepoResult<(Vec<AdminAuditRecord>, u64)> {
        let target = target.filter(|t| !t.is_empty());
        let filter = if target.is_some() {
            " WHERE target_user_id = ?"
        } else {
            ""
        };
        let with_target = |q: crate::sql::Sql| match target {
            Some(t) => q.t(t),
            None => q,
        };
        let total = with_target(sql(format!("SELECT COUNT(*) AS n FROM admin_audit{filter}"))).int(conn)?;
        let rows = with_target(sql(format!(
            "SELECT * FROM admin_audit{filter} ORDER BY created_at DESC, seq DESC{}",
            page_clause(skip.into(), limit.into())
        )))
        .load::<AuditRow>(conn)?;
        let records = rows.into_iter().map(AuditRow::decode).collect::<RepoResult<Vec<_>>>()?;
        Ok((records, read_u64(total)))
    }

    fn read_operations_policy(&self, conn: &mut AnyConnection) -> RepoResult<Option<OperationsPolicyRecord>> {
        Ok(sql("SELECT * FROM operations_policy WHERE id = 1")
            .first::<PolicyRow>(conn)?
            .map(|r| OperationsPolicyRecord {
                guard_enforcement: r.guard_enforcement,
                diagnostics_profile: r.diagnostics_profile,
                version: read_u64(r.version),
                updated_at: r.updated_at,
                updated_by: r.updated_by,
                diagnostics_expires_at: r.diagnostics_expires_at,
            }))
    }

    fn update_operations_policy(&self, tx: &mut Tx<'_>, expected: u64, r: &OperationsPolicyRecord) -> RepoResult<bool> {
        let changed = sql(
            "UPDATE operations_policy SET guard_enforcement = ?, diagnostics_profile = ?, updated_at = ?, \
             updated_by = ?, diagnostics_expires_at = ?, version = ? WHERE id = 1 AND version = ?",
        )
        .t(&*r.guard_enforcement)
        .t(&*r.diagnostics_profile)
        .ot(r.updated_at.clone())
        .ot(r.updated_by.clone())
        .ot(r.diagnostics_expires_at.clone())
        .i(stored_u64(expected).saturating_add(1))
        .i(stored_u64(expected))
        .run(tx.conn())?;
        Ok(changed == 1)
    }

    fn delete_player_data(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        _user_id: &str,
        audit: &AdminAuditRecord,
    ) -> RepoResult<PlayerDeletion> {
        let id = account.to_string();
        let accounts = u64::from(crate::repo::auth::soft_delete_game_account(tx, account)?);
        let launcher = u64::from(crate::repo::auth::soft_delete_launcher_account(tx, account)?);
        let conn = tx.conn();
        let mut counts = vec![
            ("accounts".to_owned(), accounts),
            ("launcherAccounts".to_owned(), launcher),
        ];
        let delete = |conn: &mut AnyConnection, table: &str| -> RepoResult<u64> {
            Ok(count(
                sql(format!("DELETE FROM {table} WHERE account = ?"))
                    .t(id.clone())
                    .run(conn)?,
            ))
        };
        counts.push(("adminSessions".into(), delete(conn, "admin_sessions")?));
        counts.push(("launcherGuardSessions".into(), delete(conn, "launcher_guard_sessions")?));
        counts.push(("guardFindings".into(), forget_finding_account(conn, account)?));
        counts.push(("playerActivityDaily".into(), delete(conn, "player_activity_daily")?));
        counts.push(("activityHunts".into(), delete(conn, "activity_hunts")?));
        counts.push(("slayersPathUnlocks".into(), delete(conn, "slayers_path_unlocks")?));
        counts.push(("activityFlags".into(), delete(conn, "activity_flags")?));
        let (other, character_count) = erase_other_areas(tx, account)?;
        counts.extend(other);
        insert_audit(tx.conn(), audit)?;
        Ok(PlayerDeletion {
            deleted_counts: counts,
            character_count,
        })
    }
}

#[derive(QueryableByName)]
struct FindingAccountsRow {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Text)]
    account_ids: String,
}

fn account_list(text: &str) -> Vec<String> {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Array(items)) => items
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

/// Removes the account from every finding that lists it; how many findings changed.
fn forget_finding_account(conn: &mut AnyConnection, account: &AccountId) -> RepoResult<u64> {
    let id = account.to_string();
    let rows = sql("SELECT id, account_ids FROM guard_findings WHERE account_ids LIKE ? ESCAPE '\\'")
        .t(format!("%{}%", like_escape(&id)))
        .load::<FindingAccountsRow>(conn)?;
    let mut changed = 0;
    for row in rows {
        let mut list = account_list(&row.account_ids);
        let before = list.len();
        list.retain(|a| a != &id);
        if list.len() == before {
            continue;
        }
        sql("UPDATE guard_findings SET account_ids = ?, account_count = ? WHERE id = ?")
            .t(Value::from(list.clone()).to_string())
            .i(i64::try_from(list.len()).unwrap_or(i64::MAX))
            .i(row.id)
            .run(conn)?;
        changed += 1;
    }
    Ok(changed)
}

/// `\`, `%` and `_` escaped for a `LIKE .. ESCAPE '\'`.
fn like_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

// ------------------------------------------------------------------ SQL: notices

/// [`LauncherNoticesRepository`] in SQL.
pub struct SqlLauncherNotices;

#[derive(QueryableByName)]
struct NoticesRow {
    #[diesel(sql_type = BigInt)]
    version: i64,
    #[diesel(sql_type = Nullable<Text>)]
    updated_at: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    updated_by: Option<String>,
    #[diesel(sql_type = Text)]
    announcements: String,
    #[diesel(sql_type = Text)]
    tags: String,
}

/// A stored list: anything but a JSON array reads empty.
pub fn decode_notice_list(text: &str) -> Vec<Json> {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Array(items)) => items,
        _ => Vec::new(),
    }
}

impl LauncherNoticesRepository for SqlLauncherNotices {
    fn get(&self, conn: &mut AnyConnection) -> RepoResult<Option<LauncherNoticesRecord>> {
        Ok(sql("SELECT * FROM launcher_notices WHERE id = 1")
            .first::<NoticesRow>(conn)?
            .map(|r| LauncherNoticesRecord {
                version: read_u64(r.version),
                updated_at: r.updated_at,
                updated_by: r.updated_by,
                announcements: decode_notice_list(&r.announcements),
                tags: decode_notice_list(&r.tags),
            }))
    }

    fn replace_if_version(&self, tx: &mut Tx<'_>, r: &LauncherNoticesRecord, expected: u64) -> RepoResult<bool> {
        let conn = tx.conn();
        let announcements = Value::Array(r.announcements.clone()).to_string();
        let tags = Value::Array(r.tags.clone()).to_string();
        let exists = sql("SELECT CAST(1 AS BIGINT) AS n FROM launcher_notices WHERE id = 1").exists(conn)?;
        if !exists {
            if expected != 0 {
                return Ok(false);
            }
            sql(
                "INSERT INTO launcher_notices (id, version, updated_at, updated_by, announcements, tags) \
                 VALUES (1, ?, ?, ?, ?, ?)",
            )
            .i(stored_u64(r.version))
            .ot(r.updated_at.clone())
            .ot(r.updated_by.clone())
            .t(announcements)
            .t(tags)
            .run(conn)?;
            return Ok(true);
        }
        let changed = sql(
            "UPDATE launcher_notices SET version = ?, updated_at = ?, updated_by = ?, announcements = ?, \
             tags = ? WHERE id = 1 AND version = ?",
        )
        .i(stored_u64(r.version))
        .ot(r.updated_at.clone())
        .ot(r.updated_by.clone())
        .t(announcements)
        .t(tags)
        .i(stored_u64(expected))
        .run(conn)?;
        Ok(changed == 1)
    }
}

// ------------------------------------------------------------------ SQL: activity

/// [`ActivityRepository`] in SQL.
pub struct SqlActivity;

#[derive(QueryableByName)]
struct DailyRow {
    #[diesel(sql_type = Text)]
    account: String,
    #[diesel(sql_type = Text)]
    day: String,
    #[diesel(sql_type = Text)]
    counters: String,
}

/// The stored counters document; a text that is no object is a data error.
pub fn decode_daily(text: &str) -> RepoResult<Map<String, Value>> {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(doc)) => Ok(doc),
        Ok(_) => Err(RepoError::data("player_activity_daily.counters: not an object")),
        Err(e) => Err(RepoError::data(format!("player_activity_daily.counters: {e}"))),
    }
}

/// Adds `amount` at the dotted `path`: a missing or non-numeric leaf reads 0, a non-object on the way becomes an
/// object. Non-zero amounts only.
pub fn add_daily(doc: &mut Map<String, Value>, path: &str, amount: f64) {
    if amount == 0.0 {
        return;
    }
    let mut parts = path.split('.').peekable();
    let mut node = doc;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            let current = node.get(part).and_then(Value::as_f64).unwrap_or(0.0);
            node.insert(part.to_owned(), Value::from(current + amount));
            return;
        }
        let entry = node.entry(part.to_owned()).or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        let Value::Object(next) = entry else { return };
        node = next;
    }
}

fn daily_doc(conn: &mut AnyConnection, account: &AccountId, day: &str) -> RepoResult<Map<String, Value>> {
    let stored = sql("SELECT counters AS t FROM player_activity_daily WHERE account = ? AND day = ?")
        .t(account.to_string())
        .t(day)
        .first::<crate::sql::TextRow>(conn)?;
    let mut doc = match stored {
        Some(row) => decode_daily(&row.t)?,
        None => Map::new(),
    };
    doc.insert("day".into(), day.into());
    Ok(doc)
}

fn store_daily(
    conn: &mut AnyConnection,
    account: &AccountId,
    day: &str,
    doc: Map<String, Value>,
    ttl_at: i64,
) -> RepoResult<Json> {
    let doc = Value::Object(doc);
    sql(
        "INSERT INTO player_activity_daily (account, day, counters, ttl_at) VALUES (?, ?, ?, ?) \
         ON CONFLICT (account, day) DO UPDATE SET counters = excluded.counters, ttl_at = excluded.ttl_at",
    )
    .t(account.to_string())
    .t(day)
    .t(doc.to_string())
    .i(ttl_at)
    .run(conn)?;
    Ok(doc)
}

fn daily_with_day(row: &DailyRow) -> RepoResult<Json> {
    let mut doc = decode_daily(&row.counters)?;
    doc.insert("day".into(), row.day.clone().into());
    Ok(Value::Object(doc))
}

#[derive(QueryableByName)]
struct HuntRow {
    #[diesel(sql_type = Text)]
    id: String,
    #[diesel(sql_type = Text)]
    account: String,
    #[diesel(sql_type = Bool)]
    host: bool,
    #[diesel(sql_type = Text)]
    kind: String,
    #[diesel(sql_type = Text)]
    hunt_id: String,
    #[diesel(sql_type = Text)]
    hunt_ref: String,
    #[diesel(sql_type = Text)]
    started_at: String,
    #[diesel(sql_type = Nullable<Text>)]
    ended_at: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    end_reason: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    result: Option<String>,
    #[diesel(sql_type = BigInt)]
    kills: i64,
    #[diesel(sql_type = BigInt)]
    ttl_at: i64,
}

impl HuntRow {
    fn decode(self) -> RepoResult<ActivityHuntRecord> {
        let kind = HuntKind::parse(&self.kind)
            .ok_or_else(|| RepoError::data(format!("activity_hunts.kind: {}", self.kind)))?;
        let result = match &self.result {
            None => None,
            Some(text) => {
                Some(HuntResult::parse(text).ok_or_else(|| RepoError::data(format!("activity_hunts.result: {text}")))?)
            }
        };
        let account = self
            .account
            .parse()
            .map_err(|_| RepoError::data(format!("activity_hunts.account: {}", self.account)))?;
        Ok(ActivityHuntRecord {
            id: self.id,
            account,
            host: self.host,
            kind,
            hunt_id: self.hunt_id,
            hunt_ref: self.hunt_ref,
            started_at: self.started_at,
            ended_at: self.ended_at,
            end_reason: self.end_reason,
            result,
            kills: read_u64(self.kills),
            ttl_at: self.ttl_at,
        })
    }
}

fn hunts(rows: Vec<HuntRow>) -> RepoResult<Vec<ActivityHuntRecord>> {
    rows.into_iter().map(HuntRow::decode).collect()
}

#[derive(QueryableByName)]
struct UnlockRow {
    #[diesel(sql_type = Text)]
    id: String,
    #[diesel(sql_type = Text)]
    account: String,
    #[diesel(sql_type = Text)]
    node_id: String,
    #[diesel(sql_type = Text)]
    unlocked_at: String,
    #[diesel(sql_type = Text)]
    source: String,
    #[diesel(sql_type = BigInt)]
    ttl_at: i64,
}

#[derive(QueryableByName)]
struct FlagRow {
    #[diesel(sql_type = Text)]
    id: String,
    #[diesel(sql_type = Text)]
    kind: String,
    #[diesel(sql_type = Text)]
    severity: String,
    #[diesel(sql_type = Text)]
    account: String,
    #[diesel(sql_type = Text)]
    day: String,
    #[diesel(sql_type = Text)]
    detail: String,
    #[diesel(sql_type = Bool)]
    acknowledged: bool,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Nullable<Text>)]
    acknowledged_at: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    acknowledged_by: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    note: Option<String>,
    #[diesel(sql_type = BigInt)]
    ttl_at: i64,
}

impl FlagRow {
    fn decode(self) -> RepoResult<ActivityFlagRecord> {
        let account = self
            .account
            .parse()
            .map_err(|_| RepoError::data(format!("activity_flags.account: {}", self.account)))?;
        let detail =
            serde_json::from_str(&self.detail).map_err(|e| RepoError::data(format!("activity_flags.detail: {e}")))?;
        Ok(ActivityFlagRecord {
            id: self.id,
            kind: self.kind,
            severity: self.severity,
            account,
            day: self.day,
            detail,
            acknowledged: self.acknowledged,
            created_at: self.created_at,
            acknowledged_at: self.acknowledged_at,
            acknowledged_by: self.acknowledged_by,
            note: self.note,
            ttl_at: self.ttl_at,
        })
    }
}

fn flags(rows: Vec<FlagRow>) -> RepoResult<Vec<ActivityFlagRecord>> {
    rows.into_iter().map(FlagRow::decode).collect()
}

fn find_flag(conn: &mut AnyConnection, id: &str) -> RepoResult<Option<ActivityFlagRecord>> {
    sql("SELECT * FROM activity_flags WHERE id = ?")
        .t(id)
        .first::<FlagRow>(conn)?
        .map(FlagRow::decode)
        .transpose()
}

fn insert_if_absent(conn: &mut AnyConnection, table: &str, id: &str, insert: crate::sql::Sql) -> RepoResult<bool> {
    if sql(format!("SELECT CAST(1 AS BIGINT) AS n FROM {table} WHERE id = ?"))
        .t(id)
        .exists(conn)?
    {
        return Ok(false);
    }
    insert.run(conn)?;
    Ok(true)
}

impl ActivityRepository for SqlActivity {
    fn add_to_daily(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        day: &str,
        increments: &[(String, f64)],
        ttl_at: i64,
    ) -> RepoResult<Json> {
        let conn = tx.conn();
        let mut doc = daily_doc(conn, account, day)?;
        for (path, amount) in increments {
            add_daily(&mut doc, path, *amount);
        }
        store_daily(conn, account, day, doc, ttl_at)
    }

    fn set_daily_fields(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        day: &str,
        fields: &Json,
        ttl_at: i64,
    ) -> RepoResult<Json> {
        let conn = tx.conn();
        let mut doc = daily_doc(conn, account, day)?;
        if let Value::Object(fields) = fields {
            for (key, value) in fields {
                doc.insert(key.clone(), value.clone());
            }
        }
        store_daily(conn, account, day, doc, ttl_at)
    }

    fn find_daily_rows(
        &self,
        conn: &mut AnyConnection,
        account: &AccountId,
        from: &str,
        to: &str,
        limit: u32,
    ) -> RepoResult<Vec<Json>> {
        sql(format!(
            "SELECT account, day, counters FROM player_activity_daily WHERE account = ? AND day >= ? AND day <= ? \
             ORDER BY day ASC LIMIT {limit}"
        ))
        .t(account.to_string())
        .t(from)
        .t(to)
        .load::<DailyRow>(conn)?
        .iter()
        .map(daily_with_day)
        .collect()
    }

    fn find_daily_rows_in_range(
        &self,
        conn: &mut AnyConnection,
        from: &str,
        to: &str,
        limit: u32,
    ) -> RepoResult<Vec<(AccountId, Json)>> {
        sql(format!(
            "SELECT account, day, counters FROM player_activity_daily WHERE day >= ? AND day <= ? \
             ORDER BY day ASC, account ASC LIMIT {limit}"
        ))
        .t(from)
        .t(to)
        .load::<DailyRow>(conn)?
        .iter()
        .map(|row| {
            Ok((
                account_id(&row.account, "player_activity_daily.account")?,
                daily_with_day(row)?,
            ))
        })
        .collect()
    }

    fn insert_hunt_if_absent(&self, tx: &mut Tx<'_>, r: &ActivityHuntRecord) -> RepoResult<bool> {
        let insert = sql(
            "INSERT INTO activity_hunts (id, account, host, kind, hunt_id, hunt_ref, started_at, ended_at, \
             end_reason, result, kills, ttl_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .t(&*r.id)
        .t(r.account.to_string())
        .b(r.host)
        .t(r.kind.as_str())
        .t(&*r.hunt_id)
        .t(&*r.hunt_ref)
        .t(&*r.started_at)
        .ot(r.ended_at.clone())
        .ot(r.end_reason.clone())
        .ot(r.result.map(HuntResult::as_str))
        .i(stored_u64(r.kills))
        .i(r.ttl_at);
        insert_if_absent(tx.conn(), "activity_hunts", &r.id, insert)
    }

    fn insert_unlock_if_absent(&self, tx: &mut Tx<'_>, r: &SlayersPathUnlockRecord) -> RepoResult<bool> {
        let insert = sql(
            "INSERT INTO slayers_path_unlocks (id, account, node_id, unlocked_at, source, ttl_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .t(&*r.id)
        .t(r.account.to_string())
        .t(&*r.node_id)
        .t(&*r.unlocked_at)
        .t(&*r.source)
        .i(r.ttl_at);
        insert_if_absent(tx.conn(), "slayers_path_unlocks", &r.id, insert)
    }

    fn insert_flag_if_absent(&self, tx: &mut Tx<'_>, r: &ActivityFlagRecord) -> RepoResult<bool> {
        let insert = sql(
            "INSERT INTO activity_flags (id, kind, severity, account, day, detail, acknowledged, \
             created_at, acknowledged_at, acknowledged_by, note, ttl_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .t(&*r.id)
        .t(&*r.kind)
        .t(&*r.severity)
        .t(r.account.to_string())
        .t(&*r.day)
        .t(r.detail.to_string())
        .b(r.acknowledged)
        .t(&*r.created_at)
        .ot(r.acknowledged_at.clone())
        .ot(r.acknowledged_by.clone())
        .ot(r.note.clone())
        .i(r.ttl_at);
        insert_if_absent(tx.conn(), "activity_flags", &r.id, insert)
    }

    fn find_hunt(&self, conn: &mut AnyConnection, id: &str) -> RepoResult<Option<ActivityHuntRecord>> {
        sql("SELECT * FROM activity_hunts WHERE id = ?")
            .t(id)
            .first::<HuntRow>(conn)?
            .map(HuntRow::decode)
            .transpose()
    }

    fn find_flag(&self, conn: &mut AnyConnection, id: &str) -> RepoResult<Option<ActivityFlagRecord>> {
        find_flag(conn, id)
    }

    fn find_open_dedicated_hunts(
        &self,
        conn: &mut AnyConnection,
        account: &AccountId,
    ) -> RepoResult<Vec<ActivityHuntRecord>> {
        hunts(
            sql(
                "SELECT * FROM activity_hunts WHERE account = ? AND kind = 'dedicated' AND ended_at IS NULL \
                 ORDER BY started_at DESC LIMIT 50",
            )
            .t(account.to_string())
            .load::<HuntRow>(conn)?,
        )
    }

    fn find_dedicated_hunts_covering(
        &self,
        conn: &mut AnyConnection,
        account: &AccountId,
        at: &str,
        not_before: &str,
        grace_floor: &str,
    ) -> RepoResult<Vec<ActivityHuntRecord>> {
        hunts(
            sql(
                "SELECT * FROM activity_hunts WHERE account = ? AND kind = 'dedicated' AND started_at >= ? \
                 AND started_at <= ? AND (ended_at IS NULL OR ended_at >= ?) ORDER BY started_at DESC LIMIT 10",
            )
            .t(account.to_string())
            .t(not_before)
            .t(at)
            .t(grace_floor)
            .load::<HuntRow>(conn)?,
        )
    }

    fn close_hunt_if_open(
        &self,
        tx: &mut Tx<'_>,
        id: &str,
        ended_at: &str,
        reason: &str,
        result: Option<HuntResult>,
    ) -> RepoResult<bool> {
        let closed = sql(
            "UPDATE activity_hunts SET ended_at = ?, end_reason = ?, result = ? WHERE id = ? \
             AND ended_at IS NULL",
        )
        .t(ended_at)
        .t(reason)
        .ot(result.map(HuntResult::as_str))
        .t(id)
        .run(tx.conn())?;
        Ok(closed == 1)
    }

    fn add_hunt_kills(&self, tx: &mut Tx<'_>, id: &str, kills: u64) -> RepoResult<()> {
        sql("UPDATE activity_hunts SET kills = kills + ? WHERE id = ?")
            .i(stored_u64(kills))
            .t(id)
            .run(tx.conn())?;
        Ok(())
    }

    fn find_hunts_for_account(
        &self,
        conn: &mut AnyConnection,
        account: &AccountId,
        limit: u32,
    ) -> RepoResult<Vec<ActivityHuntRecord>> {
        hunts(
            sql(format!(
                "SELECT * FROM activity_hunts WHERE account = ? ORDER BY started_at DESC, id ASC LIMIT {limit}"
            ))
            .t(account.to_string())
            .load::<HuntRow>(conn)?,
        )
    }

    fn find_unlocks_for_account(
        &self,
        conn: &mut AnyConnection,
        account: &AccountId,
        limit: u32,
    ) -> RepoResult<Vec<SlayersPathUnlockRecord>> {
        sql(format!(
            "SELECT * FROM slayers_path_unlocks WHERE account = ? ORDER BY unlocked_at DESC, id ASC LIMIT {limit}"
        ))
        .t(account.to_string())
        .load::<UnlockRow>(conn)?
        .into_iter()
        .map(|r| {
            Ok(SlayersPathUnlockRecord {
                account: r
                    .account
                    .parse()
                    .map_err(|_| RepoError::data(format!("slayers_path_unlocks.account: {}", r.account)))?,
                id: r.id,
                node_id: r.node_id,
                unlocked_at: r.unlocked_at,
                source: r.source,
                ttl_at: r.ttl_at,
            })
        })
        .collect()
    }

    fn list_flags(
        &self,
        conn: &mut AnyConnection,
        acknowledged: Option<bool>,
        skip: u32,
        limit: u32,
    ) -> RepoResult<(Vec<ActivityFlagRecord>, u64)> {
        let filter = if acknowledged.is_some() {
            " WHERE acknowledged = ?"
        } else {
            ""
        };
        let bind = |q: crate::sql::Sql| match acknowledged {
            Some(a) => q.b(a),
            None => q,
        };
        let total = bind(sql(format!("SELECT COUNT(*) AS n FROM activity_flags{filter}"))).int(conn)?;
        let rows = bind(sql(format!(
            "SELECT * FROM activity_flags{filter} ORDER BY created_at DESC, id ASC{}",
            page_clause(skip.into(), limit.into())
        )))
        .load::<FlagRow>(conn)?;
        Ok((flags(rows)?, read_u64(total)))
    }

    fn find_flags_for_account(
        &self,
        conn: &mut AnyConnection,
        account: &AccountId,
        limit: u32,
    ) -> RepoResult<Vec<ActivityFlagRecord>> {
        flags(
            sql(format!(
                "SELECT * FROM activity_flags WHERE account = ? ORDER BY created_at DESC, id ASC LIMIT {limit}"
            ))
            .t(account.to_string())
            .load::<FlagRow>(conn)?,
        )
    }

    fn acknowledge_flag(
        &self,
        tx: &mut Tx<'_>,
        id: &str,
        by: &AccountId,
        at: &str,
        note: Option<&str>,
    ) -> RepoResult<Option<ActivityFlagRecord>> {
        let conn = tx.conn();
        let Some(flag) = find_flag(conn, id)? else {
            return Ok(None);
        };
        if flag.acknowledged {
            return Ok(Some(flag));
        }
        let operator = sql("SELECT user_id AS t FROM accounts WHERE id = ? AND deleted_at IS NULL")
            .t(by.to_string())
            .first::<crate::sql::TextRow>(conn)?
            .map_or_else(|| by.to_string(), |r| r.t);
        sql("UPDATE activity_flags SET acknowledged = ?, acknowledged_at = ?, acknowledged_by = ?, note = ? WHERE id = ?")
            .b(true)
            .t(at)
            .t(operator)
            .ot(note)
            .t(id)
            .run(conn)?;
        find_flag(conn, id)
    }
}

// ------------------------------------------------------------------ SQL: Launcher Guard

/// [`LauncherGuardRepository`] in SQL.
pub struct SqlLauncherGuard;

/// `Date.parse(time)` (unparseable 0) + the retention.
fn guard_ttl(time: &str) -> i64 {
    date_parse(time)
        .unwrap_or(0)
        .saturating_add(GUARD_RETENTION_DAYS * DAY_MS)
}

#[derive(QueryableByName)]
struct GuardRow {
    #[diesel(sql_type = Text)]
    guard_session_id: String,
    #[diesel(sql_type = Text)]
    account: String,
    #[diesel(sql_type = Text)]
    role: String,
    #[diesel(sql_type = Text)]
    public_key_base64: String,
    #[diesel(sql_type = Text)]
    manifest_id: String,
    #[diesel(sql_type = BigInt)]
    manifest_sequence: i64,
    #[diesel(sql_type = Text)]
    channel: String,
    #[diesel(sql_type = Text)]
    challenge: String,
    #[diesel(sql_type = BigInt)]
    last_sequence: i64,
    #[diesel(sql_type = Text)]
    status: String,
    #[diesel(sql_type = Double)]
    risk_score: f64,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    expires_at: String,
    #[diesel(sql_type = Nullable<Text>)]
    last_heartbeat_at: Option<String>,
    #[diesel(sql_type = Text)]
    detail: String,
}

/// The `detail` column's parts: launcher version, last signals, reasons, history.
pub type GuardDetail = (String, Option<Json>, Vec<String>, Vec<Json>);

/// Reads a stored `detail`: missing parts read empty, only string reasons are kept.
pub fn decode_guard_detail(text: &str) -> GuardDetail {
    let value = serde_json::from_str::<Value>(text).unwrap_or(Value::Null);
    let launcher_version = value
        .get("launcherVersion")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let last_signals = value.get("lastSignals").filter(|v| !v.is_null()).cloned();
    let reasons = match value.get("reasons") {
        Some(Value::Array(items)) => items.iter().filter_map(|r| r.as_str().map(str::to_owned)).collect(),
        _ => Vec::new(),
    };
    let history = match value.get("history") {
        Some(Value::Array(items)) => items.clone(),
        _ => Vec::new(),
    };
    (launcher_version, last_signals, reasons, history)
}

fn encode_guard_detail(
    launcher_version: &str,
    last_signals: Option<&Json>,
    reasons: &[String],
    history: &[Json],
) -> String {
    let mut out = Map::new();
    out.insert("launcherVersion".into(), launcher_version.into());
    out.insert("lastSignals".into(), last_signals.cloned().unwrap_or(Value::Null));
    out.insert("reasons".into(), Value::from(reasons.to_vec()));
    out.insert("history".into(), Value::Array(history.to_vec()));
    Value::Object(out).to_string()
}

impl GuardRow {
    fn decode(self) -> RepoResult<LauncherGuardSessionRecord> {
        let status = GuardStatus::parse(&self.status)
            .ok_or_else(|| RepoError::data(format!("launcher_guard_sessions.status: {}", self.status)))?;
        let (launcher_version, last_signals, reasons, history) = decode_guard_detail(&self.detail);
        Ok(LauncherGuardSessionRecord {
            account: account_id(&self.account, "launcher_guard_sessions.account")?,
            guard_session_id: self.guard_session_id,
            role: GuardRole::parse(&self.role),
            public_key_base64: self.public_key_base64,
            manifest_id: self.manifest_id,
            manifest_sequence: read_u64(self.manifest_sequence),
            channel: self.channel,
            launcher_version,
            challenge: self.challenge,
            last_sequence: read_u64(self.last_sequence),
            status,
            risk_score: self.risk_score,
            created_at: self.created_at,
            expires_at: self.expires_at,
            last_heartbeat_at: self.last_heartbeat_at,
            last_signals,
            reasons,
            history,
        })
    }
}

fn find_guard(conn: &mut AnyConnection, id: &str) -> RepoResult<Option<LauncherGuardSessionRecord>> {
    sql("SELECT * FROM launcher_guard_sessions WHERE guard_session_id = ?")
        .t(id)
        .first::<GuardRow>(conn)?
        .map(GuardRow::decode)
        .transpose()
}

#[derive(QueryableByName)]
struct FindingRow {
    #[diesel(sql_type = Text)]
    file_name: String,
    #[diesel(sql_type = Text)]
    sha256: String,
    #[diesel(sql_type = Text)]
    signer: String,
    #[diesel(sql_type = BigInt)]
    size: i64,
    #[diesel(sql_type = Text)]
    first_seen_at: String,
    #[diesel(sql_type = Text)]
    last_seen_at: String,
    #[diesel(sql_type = BigInt)]
    session_count: i64,
    #[diesel(sql_type = BigInt)]
    account_count: i64,
}

#[derive(QueryableByName)]
struct SightingRow {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = BigInt)]
    session_count: i64,
    #[diesel(sql_type = Text)]
    account_ids: String,
}

impl LauncherGuardRepository for SqlLauncherGuard {
    fn create(&self, tx: &mut Tx<'_>, r: &LauncherGuardSessionRecord) -> RepoResult<()> {
        sql(
            "INSERT INTO launcher_guard_sessions (guard_session_id, account, role, public_key_base64, manifest_id, \
             manifest_sequence, channel, challenge, last_sequence, status, risk_score, created_at, expires_at, \
             last_heartbeat_at, detail, ttl_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .t(&*r.guard_session_id)
        .t(r.account.to_string())
        .t(r.role.as_str())
        .t(&*r.public_key_base64)
        .t(&*r.manifest_id)
        .i(stored_u64(r.manifest_sequence))
        .t(&*r.channel)
        .t(&*r.challenge)
        .i(stored_u64(r.last_sequence))
        .t(r.status.as_str())
        .f(r.risk_score)
        .t(&*r.created_at)
        .t(&*r.expires_at)
        .ot(r.last_heartbeat_at.clone())
        .t(encode_guard_detail(
            &r.launcher_version,
            r.last_signals.as_ref(),
            &r.reasons,
            &r.history,
        ))
        .i(guard_ttl(&r.created_at))
        .run(tx.conn())?;
        Ok(())
    }

    fn find(&self, conn: &mut AnyConnection, id: &str) -> RepoResult<Option<LauncherGuardSessionRecord>> {
        find_guard(conn, id)
    }

    fn find_healthy_for_account(
        &self,
        conn: &mut AnyConnection,
        account: &AccountId,
        id: &str,
        role: GuardRole,
        now: &str,
    ) -> RepoResult<Option<LauncherGuardSessionRecord>> {
        let now_ms = date_parse(now).ok_or_else(|| RepoError::data(format!("now: {now}")))?;
        let fresh_after = rustic_isles::season::to_iso_string(now_ms - 45_000);
        sql(
            "SELECT * FROM launcher_guard_sessions WHERE guard_session_id = ? AND account = ? AND role = ? \
             AND status = 'HEALTHY' AND expires_at > ? AND last_heartbeat_at > ?",
        )
        .t(id)
        .t(account.to_string())
        .t(role.as_str())
        .t(now)
        .t(fresh_after)
        .first::<GuardRow>(conn)?
        .map(GuardRow::decode)
        .transpose()
    }

    fn advance(&self, tx: &mut Tx<'_>, hb: &GuardHeartbeat) -> RepoResult<Option<LauncherGuardSessionRecord>> {
        let conn = tx.conn();
        let Some(session) = find_guard(conn, &hb.guard_session_id)? else {
            return Ok(None);
        };
        if session.account != hb.account
            || session.challenge != hb.expected_challenge
            || session.last_sequence >= hb.sequence
            || !session.status.is_live()
            || session.expires_at.as_str() <= hb.heartbeat_at.as_str()
        {
            return Ok(None);
        }
        let mut history = vec![hb.history_entry.clone()];
        history.extend(session.history);
        history.truncate(GUARD_HISTORY_LIMIT);
        let detail = encode_guard_detail(&session.launcher_version, Some(&hb.signals), &hb.reasons, &history);
        let sequence = stored_u64(hb.sequence);
        let moved = sql(
            "UPDATE launcher_guard_sessions SET challenge = ?, last_sequence = ?, status = ?, \
             risk_score = ?, last_heartbeat_at = ?, expires_at = ?, detail = ?, ttl_at = ? \
             WHERE guard_session_id = ? AND challenge = ? AND last_sequence < ?",
        )
        .t(&*hb.next_challenge)
        .i(sequence)
        .t(hb.status.as_str())
        .f(hb.risk_score)
        .t(&*hb.heartbeat_at)
        .t(&*hb.expires_at)
        .t(detail)
        .i(guard_ttl(&hb.heartbeat_at))
        .t(&*hb.guard_session_id)
        .t(&*hb.expected_challenge)
        .i(sequence)
        .run(conn)?;
        if moved == 0 {
            return Ok(None);
        }
        find_guard(conn, &hb.guard_session_id)
    }

    fn revoke_for_account(&self, tx: &mut Tx<'_>, account: &AccountId, role: GuardRole, at: &str) -> RepoResult<()> {
        sql(format!(
            "UPDATE launcher_guard_sessions SET status = 'REVOKED', expires_at = ?, ttl_at = ? \
             WHERE account = ? AND role = ? AND status IN {LIVE_GUARD_STATUSES}"
        ))
        .t(at)
        .i(guard_ttl(at))
        .t(account.to_string())
        .t(role.as_str())
        .run(tx.conn())?;
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
        let mut clauses = Vec::new();
        if status.is_some() {
            clauses.push("status = ?");
        }
        if min_risk.is_some() {
            clauses.push("risk_score >= ?");
        }
        let filter = if clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", clauses.join(" AND "))
        };
        let bind = |mut q: crate::sql::Sql| {
            if let Some(s) = status {
                q = q.t(s.as_str());
            }
            if let Some(r) = min_risk {
                q = q.f(r);
            }
            q
        };
        let total = bind(sql(format!(
            "SELECT COUNT(*) AS n FROM launcher_guard_sessions{filter}"
        )))
        .int(conn)?;
        let rows = bind(sql(format!(
            "SELECT * FROM launcher_guard_sessions{filter} ORDER BY created_at DESC{}",
            page_clause(skip.into(), limit.into())
        )))
        .load::<GuardRow>(conn)?;
        let sessions = rows
            .into_iter()
            .map(|r| {
                r.decode().map(|mut s| {
                    s.history.clear();
                    s
                })
            })
            .collect::<RepoResult<Vec<_>>>()?;
        Ok((sessions, read_u64(total)))
    }

    fn list_findings(
        &self,
        conn: &mut AnyConnection,
        signer: Option<&str>,
        skip: u32,
        limit: u32,
    ) -> RepoResult<(Vec<GuardFinding>, u64)> {
        let needle = signer
            .filter(|s| !s.is_empty())
            .map(|s| format!("%{}%", like_escape(&s.to_lowercase())));
        let filter = if needle.is_some() {
            " WHERE LOWER(signer) LIKE ? ESCAPE '\\'"
        } else {
            ""
        };
        let bind = |q: crate::sql::Sql| match &needle {
            Some(n) => q.t(n.clone()),
            None => q,
        };
        let total = bind(sql(format!("SELECT COUNT(*) AS n FROM guard_findings{filter}"))).int(conn)?;
        let rows = bind(sql(format!(
            "SELECT * FROM guard_findings{filter} ORDER BY session_count DESC, last_seen_at DESC, id DESC{}",
            page_clause(skip.into(), limit.into())
        )))
        .load::<FindingRow>(conn)?;
        let findings = rows
            .into_iter()
            .map(|r| GuardFinding {
                file_name: r.file_name,
                sha256: r.sha256,
                signer: r.signer,
                size: read_u64(r.size),
                first_seen_at: r.first_seen_at,
                last_seen_at: r.last_seen_at,
                session_count: read_u64(r.session_count),
                account_count: read_u64(r.account_count),
            })
            .collect();
        Ok((findings, read_u64(total)))
    }

    fn record_finding_sighting(&self, tx: &mut Tx<'_>, s: &GuardSighting) -> RepoResult<()> {
        let conn = tx.conn();
        let account = s.account.to_string();
        let existing =
            sql("SELECT id, session_count, account_ids FROM guard_findings WHERE sha256 = ? AND file_name = ?")
                .t(&*s.sha256)
                .t(&*s.file_name)
                .first::<SightingRow>(conn)?;
        let new_session = i64::from(s.new_in_session);
        let Some(row) = existing else {
            sql(
                "INSERT INTO guard_findings (file_name, sha256, signer, size, first_seen_at, last_seen_at, \
                 session_count, account_count, account_ids, ttl_at) VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?, ?)",
            )
            .t(&*s.file_name)
            .t(&*s.sha256)
            .t(&*s.signer)
            .i(stored_u64(s.size))
            .t(&*s.seen_at)
            .t(&*s.seen_at)
            .i(new_session)
            .t(Value::from(vec![account]).to_string())
            .i(guard_ttl(&s.seen_at))
            .run(conn)?;
            return Ok(());
        };
        let mut accounts = account_list(&row.account_ids);
        if !accounts.contains(&account) && accounts.len() < GUARD_FINDING_ACCOUNT_LIMIT {
            accounts.push(account);
        }
        sql(
            "UPDATE guard_findings SET signer = ?, size = ?, last_seen_at = ?, ttl_at = ?, session_count = ?, \
             account_ids = ?, account_count = ? WHERE id = ?",
        )
        .t(&*s.signer)
        .i(stored_u64(s.size))
        .t(&*s.seen_at)
        .i(guard_ttl(&s.seen_at))
        .i(row.session_count.saturating_add(new_session))
        .t(Value::from(accounts.clone()).to_string())
        .i(i64::try_from(accounts.len()).unwrap_or(i64::MAX))
        .i(row.id)
        .run(conn)?;
        Ok(())
    }
}

// ------------------------------------------------------------------ SQL: invites

/// [`InviteRepository`] in SQL.
pub struct SqlInvites;

#[derive(QueryableByName)]
struct InviteRow {
    #[diesel(sql_type = Text)]
    code: String,
    #[diesel(sql_type = BigInt)]
    uses_remaining: i64,
    #[diesel(sql_type = Bool)]
    infinite_uses: bool,
    #[diesel(sql_type = Nullable<Text>)]
    note: Option<String>,
    #[diesel(sql_type = Text)]
    created_at: String,
}

impl InviteRepository for SqlInvites {
    fn insert_if_absent(&self, tx: &mut Tx<'_>, r: &InviteRecord) -> RepoResult<bool> {
        let inserted = sql(
            "INSERT INTO reforged_invites (code, uses_remaining, infinite_uses, note, created_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .t(&*r.code)
        .i(r.uses_remaining)
        .b(r.infinite_uses)
        .ot(r.note.clone())
        .t(&*r.created_at)
        .run(tx.conn());
        match inserted {
            Ok(_) => Ok(true),
            Err(RepoError::Query(DieselError::DatabaseError(DatabaseErrorKind::UniqueViolation, _))) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn spend(&self, tx: &mut Tx<'_>, code: &str) -> RepoResult<bool> {
        let conn = tx.conn();
        let Some(invite) = sql("SELECT * FROM reforged_invites WHERE code = ?")
            .t(code)
            .first::<InviteRow>(conn)?
        else {
            return Ok(false);
        };
        if invite.infinite_uses {
            return Ok(true);
        }
        let spent = sql(
            "UPDATE reforged_invites SET uses_remaining = uses_remaining - 1 WHERE code = ? \
             AND uses_remaining > 0",
        )
        .t(code)
        .run(conn)?;
        Ok(spent == 1)
    }

    fn list(&self, conn: &mut AnyConnection, limit: u32) -> RepoResult<Vec<InviteRecord>> {
        Ok(sql(format!(
            "SELECT * FROM reforged_invites ORDER BY created_at DESC, code ASC LIMIT {limit}"
        ))
        .load::<InviteRow>(conn)?
        .into_iter()
        .map(|r| InviteRecord {
            code: r.code,
            uses_remaining: r.uses_remaining,
            infinite_uses: r.infinite_uses,
            note: r.note,
            created_at: r.created_at,
        })
        .collect())
    }

    fn delete(&self, tx: &mut Tx<'_>, code: &str) -> RepoResult<bool> {
        Ok(sql("DELETE FROM reforged_invites WHERE code = ?")
            .t(code)
            .run(tx.conn())?
            == 1)
    }
}

// ------------------------------------------------------------------ erasure and TTL purge

/// Every non-auth area's rows of the account in one write (spec §11.3.6): the counts in their order, and how many
/// live characters were soft-deleted.
pub fn erase_other_areas(tx: &mut Tx<'_>, account: &AccountId) -> RepoResult<(Vec<(String, u64)>, u64)> {
    let at = tx.at();
    let id = account.to_string();
    let conn = tx.conn();
    // Every character of the account, deleted ones too
    let characters: Vec<String> = sql("SELECT character_id AS t FROM characters WHERE account_id = ?")
        .t(id.clone())
        .load::<crate::sql::TextRow>(conn)?
        .into_iter()
        .map(|r| r.t)
        .collect();
    // `<account column> = ? OR character_id IN (..)`, bound in that order
    let by_account_or_character = |column: &str| {
        if characters.is_empty() {
            format!("{column} = ?")
        } else {
            format!(
                "({column} = ? OR character_id IN ({}))",
                vec!["?"; characters.len()].join(", ")
            )
        }
    };
    let bind_characters = |mut q: crate::sql::Sql| {
        q = q.t(id.clone());
        for c in &characters {
            q = q.t(&**c);
        }
        q
    };
    let soft = |conn: &mut AnyConnection, table: &str, clause: String, bind_all: bool| -> RepoResult<u64> {
        let q = sql(format!(
            "UPDATE {table} SET deleted_at = ? WHERE deleted_at IS NULL AND {clause}"
        ))
        .i(at);
        let q = if bind_all { bind_characters(q) } else { q.t(id.clone()) };
        Ok(count(q.run(conn)?))
    };
    let hard = |conn: &mut AnyConnection, table: &str, clause: String, bind_all: bool| -> RepoResult<u64> {
        let q = sql(format!("DELETE FROM {table} WHERE {clause}"));
        let q = if bind_all { bind_characters(q) } else { q.t(id.clone()) };
        Ok(count(q.run(conn)?))
    };
    let mut out = Vec::new();
    let character_count = soft(conn, "characters", "account_id = ?".into(), false)?;
    out.push(("characters".to_owned(), character_count));
    out.push((
        "loadouts".into(),
        soft(conn, "loadouts", by_account_or_character("account_id"), true)?,
    ));
    out.push((
        "inventories".into(),
        soft(conn, "inventories", by_account_or_character("account_id"), true)?,
    ));
    out.push((
        "inventoryTransactions".into(),
        hard(
            conn,
            "inventory_transactions",
            by_account_or_character("account_id"),
            true,
        )?,
    ));
    out.push((
        "breadcrumbs".into(),
        soft(conn, "breadcrumbs", by_account_or_character("account"), true)?,
    ));
    out.push((
        "encounteredContent".into(),
        soft(conn, "encountered_content", by_account_or_character("account"), true)?,
    ));
    out.push((
        "progressionTracks".into(),
        soft(conn, "progression_tracks", "account = ?".into(), false)?,
    ));
    out.push((
        "progressionObjectives".into(),
        soft(conn, "progression_objectives", "account = ?".into(), false)?,
    ));
    out.push((
        "progressionObjectiveEvents".into(),
        hard(conn, "progression_objective_events", "account = ?".into(), false)?,
    ));
    out.push((
        "progressionGrants".into(),
        hard(conn, "progression_grants", "account = ?".into(), false)?,
    ));
    out.push((
        "escalationProgress".into(),
        soft(conn, "escalation_progress", "account = ?".into(), false)?,
    ));
    out.push((
        "playerJourney".into(),
        soft(conn, "player_journey", "account = ?".into(), false)?,
    ));
    for (name, table) in [
        ("bountyStates", "bounty_states"),
        ("cooldownStates", "cooldown_states"),
        ("entitlements", "entitlements"),
        ("walletBalances", "wallet_balances"),
        ("wallets", "wallets"),
    ] {
        debug_assert!(bounties::SOFT_DELETE_TABLES.contains(&table));
        out.push((name.into(), soft(conn, table, "account_id = ?".into(), false)?));
    }
    out.push((
        "economyRollups".into(),
        hard(conn, "economy_rollup_counters", "account_id = ?".into(), false)?,
    ));
    out.push(("friends".into(), chat::remove_all_edges(conn, account)?));
    out.push((
        "partyInvites".into(),
        count(
            sql("DELETE FROM party_invites WHERE sender = ? OR recipient = ?")
                .t(id.clone())
                .t(id.clone())
                .run(conn)?,
        ),
    ));
    out.push(("parties".into(), parties::erase_memberships(conn, account)?));
    Ok((out, character_count))
}

/// The tables whose rows carry a `ttl_at`.
pub const TTL_TABLES: &[&str] = &[
    "admin_sessions",
    "launcher_guard_sessions",
    "guard_findings",
    "player_activity_daily",
    "activity_hunts",
    "slayers_path_unlocks",
    "activity_flags",
    "economy_rollup_counters",
];

/// Deletes every row whose `ttl_at` has passed; how many went.
pub fn purge_expired_ttl_rows(tx: &mut Tx<'_>, now_ms: i64) -> RepoResult<u64> {
    let conn = tx.conn();
    let mut total = 0;
    for table in TTL_TABLES {
        total += count(
            sql(format!("DELETE FROM {table} WHERE ttl_at <= ?"))
                .i(now_ms)
                .run(conn)?,
        );
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daily_paths_add_and_replace_non_objects() {
        let mut doc = Map::new();
        add_daily(&mut doc, "hunts.completed", 1.0);
        add_daily(&mut doc, "hunts.completed", 2.0);
        add_daily(&mut doc, "skipped", 0.0);
        doc.insert("kills".into(), "x".into());
        add_daily(&mut doc, "kills", 4.0);
        doc.insert("deep".into(), 3.into());
        add_daily(&mut doc, "deep.er", 1.0);
        assert_eq!(
            Value::Object(doc).to_string(),
            r#"{"hunts":{"completed":3.0},"kills":4.0,"deep":{"er":1.0}}"#
        );
    }

    #[test]
    fn guard_detail_reads_leniently() {
        let (version, signals, reasons, history) = decode_guard_detail(r#"{"reasons":["a",1],"history":{}}"#);
        assert_eq!(
            (version.as_str(), signals, reasons, history.len()),
            ("", None, vec!["a".to_owned()], 0)
        );
        let text = encode_guard_detail("1.2", None, &["r".into()], &[]);
        assert_eq!(
            text,
            r#"{"launcherVersion":"1.2","lastSignals":null,"reasons":["r"],"history":[]}"#
        );
    }

    #[test]
    fn like_escapes() {
        assert_eq!(like_escape(r"a_b%c\d"), r"a\_b\%c\\d");
    }
}
