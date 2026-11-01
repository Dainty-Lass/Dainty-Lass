//! RU5 auth: game accounts, launcher accounts and their sessions, sign-in codes and API keys (migration
//! `0002_auth`).
//!
//! Node kept all of these as documents of a few collections; here each is a table keyed by the account's global id
//! (DD18), with the client-visible `userId` on `accounts.user_id`. Every conditional update Node did in one document
//! write (`consumeByCodeHash`, `reserveTry`...) is a select and an update inside one [`Tx`], which is atomic because
//! `store` serialises writes (a read-then-write here cannot interleave with another request's).

use std::fmt;

use diesel::prelude::*;

use super::{AccountId, AnyConnection, RepoError, RepoResult, Tx, UserId, repository};

diesel::table! {
    accounts (id) {
        id -> Text,
        user_id -> Text,
        name -> Text,
        notes -> BigInt,
        deleted_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    launcher_accounts (account_id) {
        account_id -> Text,
        email -> Nullable<Text>,
        display_name_normalized -> Text,
        display_name -> Text,
        password_hash -> Nullable<Text>,
        status -> Text,
        approval_status -> Nullable<Text>,
        approval_updated_at -> Nullable<Text>,
        approval_updated_by -> Nullable<Text>,
        approval_reason -> Nullable<Text>,
        roles -> Text,
        roles_updated_at -> Nullable<Text>,
        roles_updated_by -> Nullable<Text>,
        created_at -> Text,
        last_login_at -> Nullable<Text>,
        username_set -> Nullable<BigInt>,
        consent_terms_version -> Nullable<Text>,
        consent_privacy_version -> Nullable<Text>,
        consent_accepted_at -> Nullable<Text>,
        deletion_requested_at -> Nullable<Text>,
        deletion_scheduled_for -> Nullable<Text>,
        email_verified_at -> Nullable<Text>,
        email_verification_pending_since -> Nullable<Text>,
        deleted_at -> Nullable<BigInt>,
        timeout_until -> Nullable<Text>,
        timeout_since -> Nullable<Text>,
        timeout_reason -> Nullable<Text>,
    }
}

diesel::table! {
    auth_identities (provider, provider_subject) {
        provider -> Text,
        provider_subject -> Text,
        account_id -> Text,
        provider_username -> Text,
        provider_avatar_url -> Nullable<Text>,
        linked_at -> Text,
    }
}

diesel::table! {
    refresh_sessions (id) {
        id -> Text,
        token_hash -> Text,
        account_id -> Text,
        family_id -> Text,
        device_id -> Text,
        device_name -> Text,
        created_at -> Text,
        expires_at -> Text,
        revoked_at -> Nullable<Text>,
    }
}

diesel::table! {
    game_exchange_codes (code_hash) {
        code_hash -> Text,
        account_id -> Text,
        launcher_session_id -> Text,
        build_changelist -> BigInt,
        executable_sha256 -> Text,
        runtime_channel -> Text,
        runtime_sha256 -> Text,
        runtime_manifest_version -> Text,
        runtime_artifact_set_sha256 -> Text,
        realm_id -> Nullable<Text>,
        created_at -> Text,
        expires_at -> Text,
        consumed_at -> Nullable<Text>,
    }
}

diesel::table! {
    discord_oauth_transactions (state) {
        state -> Text,
        code_verifier -> Text,
        created_at -> Text,
        expires_at -> Text,
        consumed_at -> Nullable<Text>,
        completion_code_hash -> Nullable<Text>,
        completion_expires_at -> Nullable<Text>,
        completion_consumed_at -> Nullable<Text>,
        account_id -> Nullable<Text>,
        link_account_id -> Nullable<Text>,
    }
}

diesel::table! {
    email_codes (account_id, purpose) {
        account_id -> Text,
        purpose -> Text,
        code_hash -> Text,
        attempts -> BigInt,
        created_at -> Text,
        expires_at -> Text,
    }
}

diesel::table! {
    game_server_api_keys (key_hash) {
        key_hash -> Text,
    }
}

diesel::table! {
    user_api_keys (account_id) {
        account_id -> Text,
        key_hash -> Text,
    }
}

diesel::joinable!(launcher_accounts -> accounts (account_id));
diesel::allow_tables_to_appear_in_same_query!(accounts, launcher_accounts);

fn account_id(text: &str, column: &str) -> RepoResult<AccountId> {
    text.parse()
        .map_err(|_| RepoError::Data(format!("{column} {text:?} is not an account id")))
}

fn optional_account(text: Option<String>, column: &str) -> RepoResult<Option<AccountId>> {
    text.map(|text| account_id(&text, column)).transpose()
}

// ------------------------------------------------------------------ accounts

/// `AccountRecord`: the game account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRecord {
    pub id: AccountId,
    pub user_id: UserId,
    pub name: String,
    pub notes: i64,
}

repository! {
    pub trait AccountRepository("accounts") {
        fn find_by_user_id(&self, conn: &mut AnyConnection, user: &UserId) -> Option<AccountRecord>;
        fn find(&self, conn: &mut AnyConnection, account: &AccountId) -> Option<AccountRecord>;
        fn create(&self, tx: &mut Tx<'_>, account: &AccountRecord) -> ();
    }
}

type AccountRow = (String, String, String, i64);

fn account(row: AccountRow) -> RepoResult<AccountRecord> {
    let (id, user_id, name, notes) = row;
    Ok(AccountRecord {
        id: account_id(&id, "accounts.id")?,
        user_id: UserId(user_id),
        name,
        notes,
    })
}

/// [`AccountRepository`] on `store`: live rows only, a deleted account is soft-deleted (DD23).
#[derive(Debug, Clone, Copy, Default)]
pub struct SqlAccounts;

impl AccountRepository for SqlAccounts {
    fn find_by_user_id(&self, conn: &mut AnyConnection, user: &UserId) -> RepoResult<Option<AccountRecord>> {
        accounts::table
            .filter(accounts::user_id.eq(user.as_str()))
            .filter(accounts::deleted_at.is_null())
            .select((accounts::id, accounts::user_id, accounts::name, accounts::notes))
            .first::<AccountRow>(conn)
            .optional()?
            .map(account)
            .transpose()
    }

    fn find(&self, conn: &mut AnyConnection, id: &AccountId) -> RepoResult<Option<AccountRecord>> {
        accounts::table
            .filter(accounts::id.eq(id.to_string()))
            .filter(accounts::deleted_at.is_null())
            .select((accounts::id, accounts::user_id, accounts::name, accounts::notes))
            .first::<AccountRow>(conn)
            .optional()?
            .map(account)
            .transpose()
    }

    fn create(&self, tx: &mut Tx<'_>, record: &AccountRecord) -> RepoResult<()> {
        diesel::insert_into(accounts::table)
            .values((
                accounts::id.eq(record.id.to_string()),
                accounts::user_id.eq(record.user_id.as_str()),
                accounts::name.eq(&record.name),
                accounts::notes.eq(record.notes),
            ))
            .execute(tx.conn())?;
        Ok(())
    }
}

// ------------------------------------------------------------------ launcher accounts

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Rejected,
}

impl ApprovalStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "pending" => Some(Self::Pending),
            "approved" => Some(Self::Approved),
            "rejected" => Some(Self::Rejected),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationalStatus {
    Active,
    Banned,
    Disabled,
}

impl OperationalStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Banned => "banned",
            Self::Disabled => "disabled",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "active" => Some(Self::Active),
            "banned" => Some(Self::Banned),
            "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }
}

/// `LauncherConsentRecord`; `ageConfirmed` is always true when a record exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherConsent {
    pub terms_version: String,
    pub privacy_version: String,
    pub accepted_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherDeletion {
    pub requested_at: String,
    pub scheduled_for: String,
}

/// A timeout (migration 0610): the account may sign in but not play until `until`. Times are `iso_from_ms`'s text,
/// so they compare as the times do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountTimeout {
    pub until: String,
    pub since: String,
    pub reason: Option<String>,
}

/// `LauncherAccountRecord`. The password hash stays out of `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct LauncherAccountRecord {
    pub account: AccountId,
    /// Node's `userId`, the id the launcher and the game client see
    pub user_id: UserId,
    pub email: Option<String>,
    pub display_name_normalized: String,
    pub display_name: String,
    pub password_hash: Option<String>,
    pub status: OperationalStatus,
    pub approval_status: Option<ApprovalStatus>,
    pub approval_updated_at: Option<String>,
    pub approval_updated_by: Option<String>,
    pub approval_reason: Option<String>,
    pub roles: Vec<String>,
    pub roles_updated_at: Option<String>,
    pub roles_updated_by: Option<String>,
    pub created_at: String,
    pub last_login_at: Option<String>,
    pub username_set: Option<bool>,
    pub consent: Option<LauncherConsent>,
    pub deletion: Option<LauncherDeletion>,
    pub email_verified_at: Option<String>,
    pub email_verification_pending_since: Option<String>,
    /// The last timeout set, which may have run out: [`Self::timeout_at`] says whether it is in force
    pub timeout: Option<AccountTimeout>,
}

impl LauncherAccountRecord {
    /// The timeout in force at `now` (`iso_from_ms`'s text).
    pub fn timeout_at(&self, now: &str) -> Option<&AccountTimeout> {
        self.timeout.as_ref().filter(|t| t.until.as_str() > now)
    }
}

impl fmt::Debug for LauncherAccountRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LauncherAccountRecord")
            .field("account", &self.account)
            .field("user_id", &self.user_id)
            .field("display_name", &self.display_name)
            .field("status", &self.status)
            .field("approval_status", &self.approval_status)
            .field("roles", &self.roles)
            .field("timeout", &self.timeout)
            .field("password_hash", &self.password_hash.as_ref().map(|_| "<set>"))
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminAccountFilter {
    pub approval_status: Option<ApprovalStatus>,
    pub status: Option<OperationalStatus>,
    /// Only the accounts with a timeout in force at this time
    pub timed_out_at: Option<String>,
    pub search: Option<String>,
    pub skip: u32,
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessChange {
    pub approval_status: Option<ApprovalStatus>,
    pub status: Option<OperationalStatus>,
    pub approval_updated_at: String,
    pub approval_updated_by: String,
    pub approval_reason: Option<String>,
}

/// What `LauncherAccountRepository::create` did: Node threw the unique index's E11000 and the controller read which
/// index it was; the write is atomic, so the check comes first here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateOutcome {
    Created,
    EmailTaken,
    DisplayNameTaken,
}

repository! {
    pub trait LauncherAccountRepository("launcherAccounts") {
        fn find_by_user_id(&self, conn: &mut AnyConnection, user: &UserId) -> Option<LauncherAccountRecord>;
        fn find_by_account(&self, conn: &mut AnyConnection, account: &AccountId) -> Option<LauncherAccountRecord>;
        fn find_by_email(&self, conn: &mut AnyConnection, normalized_email: &str) -> Option<LauncherAccountRecord>;
        fn find_by_display_name_normalized(&self, conn: &mut AnyConnection, normalized: &str)
            -> Option<LauncherAccountRecord>;
        /// Writes the game account too unless it exists (`name` mirrors the display name, `notes` is 0), as Node's `create` did
        fn create(&self, tx: &mut Tx<'_>, account: &LauncherAccountRecord) -> CreateOutcome;
        fn update_last_login(&self, tx: &mut Tx<'_>, account: &AccountId, when: &str) -> ();
        /// Also the game account's `name`; marks the username set
        fn set_username(&self, tx: &mut Tx<'_>, account: &AccountId, display_name: &str, normalized: &str)
            -> CreateOutcome;
        fn set_consent(&self, tx: &mut Tx<'_>, account: &AccountId, consent: &LauncherConsent) -> ();
        fn set_deletion(&self, tx: &mut Tx<'_>, account: &AccountId, deletion: Option<&LauncherDeletion>) -> ();
        fn list_deletions_due(&self, conn: &mut AnyConnection, now: &str, limit: u32) -> Vec<LauncherAccountRecord>;
        /// Also clears the pending mark
        fn set_email_verified(&self, tx: &mut Tx<'_>, account: &AccountId, verified_at: &str) -> ();
        fn set_password_hash(&self, tx: &mut Tx<'_>, account: &AccountId, password_hash: &str) -> ();
        fn list_unverified_registrations(&self, conn: &mut AnyConnection, pending_before: &str, limit: u32)
            -> Vec<LauncherAccountRecord>;
        /// Soft-deletes (DD23) the launcher and game account of an unverified registration; whether this call did
        fn delete_unverified_registration(&self, tx: &mut Tx<'_>, account: &AccountId, pending_before: &str) -> bool;
        /// The records and the total
        fn list_for_admin(&self, conn: &mut AnyConnection, filter: &AdminAccountFilter)
            -> (Vec<LauncherAccountRecord>, u64);
        fn set_access_state(&self, tx: &mut Tx<'_>, account: &AccountId, change: &AccessChange)
            -> Option<LauncherAccountRecord>;
        /// Sets or (with `None`) lifts the account's timeout
        fn set_timeout(&self, tx: &mut Tx<'_>, account: &AccountId, timeout: Option<&AccountTimeout>)
            -> Option<LauncherAccountRecord>;
        fn update_roles(&self, tx: &mut Tx<'_>, account: &AccountId, roles: &[String], at: &str, by: &str)
            -> Option<LauncherAccountRecord>;
        /// An account's erasure (the 30-day deletion): soft-deletes the launcher and the game account (DD23); whether
        /// there was a live one
        fn erase(&self, tx: &mut Tx<'_>, account: &AccountId) -> bool;
    }
}

#[derive(Queryable, Selectable)]
#[diesel(table_name = launcher_accounts)]
struct LauncherRow {
    account_id: String,
    email: Option<String>,
    display_name_normalized: String,
    display_name: String,
    password_hash: Option<String>,
    status: String,
    approval_status: Option<String>,
    approval_updated_at: Option<String>,
    approval_updated_by: Option<String>,
    approval_reason: Option<String>,
    roles: String,
    roles_updated_at: Option<String>,
    roles_updated_by: Option<String>,
    created_at: String,
    last_login_at: Option<String>,
    username_set: Option<i64>,
    consent_terms_version: Option<String>,
    consent_privacy_version: Option<String>,
    consent_accepted_at: Option<String>,
    deletion_requested_at: Option<String>,
    deletion_scheduled_for: Option<String>,
    email_verified_at: Option<String>,
    email_verification_pending_since: Option<String>,
    timeout_until: Option<String>,
    timeout_since: Option<String>,
    timeout_reason: Option<String>,
}

fn launcher_record(row: LauncherRow, user_id: String) -> RepoResult<LauncherAccountRecord> {
    let roles: Vec<String> = serde_json::from_str(&row.roles)
        .map_err(|e| RepoError::Data(format!("launcher_accounts.roles {:?}: {e}", row.roles)))?;
    let status = OperationalStatus::parse(&row.status)
        .ok_or_else(|| RepoError::Data(format!("launcher_accounts.status {:?}", row.status)))?;
    let approval_status = match row.approval_status.as_deref() {
        None => None,
        Some(text) => Some(
            ApprovalStatus::parse(text)
                .ok_or_else(|| RepoError::Data(format!("launcher_accounts.approval_status {text:?}")))?,
        ),
    };
    let consent = match (
        row.consent_terms_version,
        row.consent_privacy_version,
        row.consent_accepted_at,
    ) {
        (Some(terms_version), Some(privacy_version), Some(accepted_at)) => Some(LauncherConsent {
            terms_version,
            privacy_version,
            accepted_at,
        }),
        _ => None,
    };
    let timeout = match (row.timeout_until, row.timeout_since) {
        (Some(until), Some(since)) => Some(AccountTimeout {
            until,
            since,
            reason: row.timeout_reason,
        }),
        _ => None,
    };
    let deletion = match (row.deletion_requested_at, row.deletion_scheduled_for) {
        (Some(requested_at), Some(scheduled_for)) => Some(LauncherDeletion {
            requested_at,
            scheduled_for,
        }),
        _ => None,
    };
    Ok(LauncherAccountRecord {
        account: account_id(&row.account_id, "launcher_accounts.account_id")?,
        user_id: UserId(user_id),
        email: row.email,
        display_name_normalized: row.display_name_normalized,
        display_name: row.display_name,
        password_hash: row.password_hash,
        status,
        approval_status,
        approval_updated_at: row.approval_updated_at,
        approval_updated_by: row.approval_updated_by,
        approval_reason: row.approval_reason,
        roles,
        roles_updated_at: row.roles_updated_at,
        roles_updated_by: row.roles_updated_by,
        created_at: row.created_at,
        last_login_at: row.last_login_at,
        username_set: row.username_set.map(|flag| flag != 0),
        consent,
        deletion,
        email_verified_at: row.email_verified_at,
        email_verification_pending_since: row.email_verification_pending_since,
        timeout,
    })
}

/// A live launcher account joined to its game account (the `userId`).
fn live_launchers() -> diesel::helper_types::Filter<
    diesel::helper_types::InnerJoin<launcher_accounts::table, accounts::table>,
    diesel::helper_types::And<
        diesel::helper_types::IsNull<launcher_accounts::deleted_at>,
        diesel::helper_types::IsNull<accounts::deleted_at>,
    >,
> {
    launcher_accounts::table.inner_join(accounts::table).filter(
        launcher_accounts::deleted_at
            .is_null()
            .and(accounts::deleted_at.is_null()),
    )
}

fn load_launchers(rows: Vec<(LauncherRow, String)>) -> RepoResult<Vec<LauncherAccountRecord>> {
    rows.into_iter().map(|(row, user)| launcher_record(row, user)).collect()
}

fn find_launcher_by_account(conn: &mut AnyConnection, id: &AccountId) -> RepoResult<Option<LauncherAccountRecord>> {
    live_launchers()
        .filter(launcher_accounts::account_id.eq(id.to_string()))
        .select((LauncherRow::as_select(), accounts::user_id))
        .first::<(LauncherRow, String)>(conn)
        .optional()?
        .map(|(row, user)| launcher_record(row, user))
        .transpose()
}

fn name_taken(conn: &mut AnyConnection, normalized: &str, except: Option<&AccountId>) -> RepoResult<bool> {
    let mut query = launcher_accounts::table
        .filter(launcher_accounts::deleted_at.is_null())
        .filter(launcher_accounts::display_name_normalized.eq(normalized))
        .select(launcher_accounts::account_id)
        .into_boxed();
    if let Some(except) = except {
        query = query.filter(launcher_accounts::account_id.ne(except.to_string()));
    }
    Ok(query.first::<String>(conn).optional()?.is_some())
}

/// [`LauncherAccountRepository`] on `store`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SqlLauncherAccounts;

impl LauncherAccountRepository for SqlLauncherAccounts {
    fn find_by_user_id(&self, conn: &mut AnyConnection, user: &UserId) -> RepoResult<Option<LauncherAccountRecord>> {
        live_launchers()
            .filter(accounts::user_id.eq(user.as_str()))
            .select((LauncherRow::as_select(), accounts::user_id))
            .first::<(LauncherRow, String)>(conn)
            .optional()?
            .map(|(row, user)| launcher_record(row, user))
            .transpose()
    }

    fn find_by_account(&self, conn: &mut AnyConnection, id: &AccountId) -> RepoResult<Option<LauncherAccountRecord>> {
        find_launcher_by_account(conn, id)
    }

    fn find_by_email(&self, conn: &mut AnyConnection, email: &str) -> RepoResult<Option<LauncherAccountRecord>> {
        live_launchers()
            .filter(launcher_accounts::email.eq(email))
            .select((LauncherRow::as_select(), accounts::user_id))
            .first::<(LauncherRow, String)>(conn)
            .optional()?
            .map(|(row, user)| launcher_record(row, user))
            .transpose()
    }

    fn find_by_display_name_normalized(
        &self,
        conn: &mut AnyConnection,
        normalized: &str,
    ) -> RepoResult<Option<LauncherAccountRecord>> {
        live_launchers()
            .filter(launcher_accounts::display_name_normalized.eq(normalized))
            .select((LauncherRow::as_select(), accounts::user_id))
            .first::<(LauncherRow, String)>(conn)
            .optional()?
            .map(|(row, user)| launcher_record(row, user))
            .transpose()
    }

    fn create(&self, tx: &mut Tx<'_>, record: &LauncherAccountRecord) -> RepoResult<CreateOutcome> {
        if let Some(email) = &record.email {
            let taken = launcher_accounts::table
                .filter(launcher_accounts::deleted_at.is_null())
                .filter(launcher_accounts::email.eq(email))
                .select(launcher_accounts::account_id)
                .first::<String>(tx.conn())
                .optional()?;
            if taken.is_some() {
                return Ok(CreateOutcome::EmailTaken);
            }
        }
        if name_taken(tx.conn(), &record.display_name_normalized, None)? {
            return Ok(CreateOutcome::DisplayNameTaken);
        }
        // The game account is this call's to write unless the caller made it first (reforged-api's registration does)
        let has_account = accounts::table
            .filter(accounts::id.eq(record.account.to_string()))
            .select(accounts::id)
            .first::<String>(tx.conn())
            .optional()?
            .is_some();
        if !has_account {
            diesel::insert_into(accounts::table)
                .values((
                    accounts::id.eq(record.account.to_string()),
                    accounts::user_id.eq(record.user_id.as_str()),
                    accounts::name.eq(&record.display_name),
                    accounts::notes.eq(0_i64),
                ))
                .execute(tx.conn())?;
        }
        let roles = serde_json::to_string(&record.roles).map_err(|e| RepoError::Data(e.to_string()))?;
        diesel::insert_into(launcher_accounts::table)
            .values((
                launcher_accounts::account_id.eq(record.account.to_string()),
                launcher_accounts::email.eq(&record.email),
                launcher_accounts::display_name_normalized.eq(&record.display_name_normalized),
                launcher_accounts::display_name.eq(&record.display_name),
                launcher_accounts::password_hash.eq(&record.password_hash),
                launcher_accounts::status.eq(record.status.as_str()),
                launcher_accounts::approval_status.eq(record.approval_status.map(ApprovalStatus::as_str)),
                launcher_accounts::approval_updated_at.eq(&record.approval_updated_at),
                launcher_accounts::approval_updated_by.eq(&record.approval_updated_by),
                launcher_accounts::approval_reason.eq(&record.approval_reason),
                launcher_accounts::roles.eq(roles),
                launcher_accounts::roles_updated_at.eq(&record.roles_updated_at),
                launcher_accounts::roles_updated_by.eq(&record.roles_updated_by),
                launcher_accounts::created_at.eq(&record.created_at),
                launcher_accounts::last_login_at.eq(&record.last_login_at),
                launcher_accounts::username_set.eq(record.username_set.map(i64::from)),
                launcher_accounts::consent_terms_version.eq(record.consent.as_ref().map(|c| c.terms_version.clone())),
                launcher_accounts::consent_privacy_version
                    .eq(record.consent.as_ref().map(|c| c.privacy_version.clone())),
                launcher_accounts::consent_accepted_at.eq(record.consent.as_ref().map(|c| c.accepted_at.clone())),
                launcher_accounts::deletion_requested_at.eq(record.deletion.as_ref().map(|d| d.requested_at.clone())),
                launcher_accounts::deletion_scheduled_for.eq(record.deletion.as_ref().map(|d| d.scheduled_for.clone())),
                launcher_accounts::email_verified_at.eq(&record.email_verified_at),
                launcher_accounts::email_verification_pending_since.eq(&record.email_verification_pending_since),
            ))
            .execute(tx.conn())?;
        Ok(CreateOutcome::Created)
    }

    fn update_last_login(&self, tx: &mut Tx<'_>, id: &AccountId, when: &str) -> RepoResult<()> {
        diesel::update(launcher_accounts::table.filter(launcher_accounts::account_id.eq(id.to_string())))
            .set(launcher_accounts::last_login_at.eq(when))
            .execute(tx.conn())?;
        Ok(())
    }

    fn set_username(
        &self,
        tx: &mut Tx<'_>,
        id: &AccountId,
        display_name: &str,
        normalized: &str,
    ) -> RepoResult<CreateOutcome> {
        if name_taken(tx.conn(), normalized, Some(id))? {
            return Ok(CreateOutcome::DisplayNameTaken);
        }
        diesel::update(launcher_accounts::table.filter(launcher_accounts::account_id.eq(id.to_string())))
            .set((
                launcher_accounts::display_name.eq(display_name),
                launcher_accounts::display_name_normalized.eq(normalized),
                launcher_accounts::username_set.eq(Some(1_i64)),
            ))
            .execute(tx.conn())?;
        diesel::update(accounts::table.filter(accounts::id.eq(id.to_string())))
            .set(accounts::name.eq(display_name))
            .execute(tx.conn())?;
        Ok(CreateOutcome::Created)
    }

    fn set_consent(&self, tx: &mut Tx<'_>, id: &AccountId, consent: &LauncherConsent) -> RepoResult<()> {
        diesel::update(launcher_accounts::table.filter(launcher_accounts::account_id.eq(id.to_string())))
            .set((
                launcher_accounts::consent_terms_version.eq(Some(&consent.terms_version)),
                launcher_accounts::consent_privacy_version.eq(Some(&consent.privacy_version)),
                launcher_accounts::consent_accepted_at.eq(Some(&consent.accepted_at)),
            ))
            .execute(tx.conn())?;
        Ok(())
    }

    fn set_deletion(&self, tx: &mut Tx<'_>, id: &AccountId, deletion: Option<&LauncherDeletion>) -> RepoResult<()> {
        diesel::update(launcher_accounts::table.filter(launcher_accounts::account_id.eq(id.to_string())))
            .set((
                launcher_accounts::deletion_requested_at.eq(deletion.map(|d| d.requested_at.clone())),
                launcher_accounts::deletion_scheduled_for.eq(deletion.map(|d| d.scheduled_for.clone())),
            ))
            .execute(tx.conn())?;
        Ok(())
    }

    fn list_deletions_due(
        &self,
        conn: &mut AnyConnection,
        now: &str,
        limit: u32,
    ) -> RepoResult<Vec<LauncherAccountRecord>> {
        let mut query = live_launchers()
            .filter(launcher_accounts::deletion_scheduled_for.le(now))
            .order(launcher_accounts::deletion_scheduled_for.asc())
            .select((LauncherRow::as_select(), accounts::user_id))
            .into_boxed();
        if limit > 0 {
            query = query.limit(i64::from(limit));
        }
        load_launchers(query.load(conn)?)
    }

    fn set_email_verified(&self, tx: &mut Tx<'_>, id: &AccountId, verified_at: &str) -> RepoResult<()> {
        diesel::update(launcher_accounts::table.filter(launcher_accounts::account_id.eq(id.to_string())))
            .set((
                launcher_accounts::email_verified_at.eq(Some(verified_at)),
                launcher_accounts::email_verification_pending_since.eq(None::<String>),
            ))
            .execute(tx.conn())?;
        Ok(())
    }

    fn set_password_hash(&self, tx: &mut Tx<'_>, id: &AccountId, password_hash: &str) -> RepoResult<()> {
        diesel::update(launcher_accounts::table.filter(launcher_accounts::account_id.eq(id.to_string())))
            .set(launcher_accounts::password_hash.eq(Some(password_hash)))
            .execute(tx.conn())?;
        Ok(())
    }

    fn list_unverified_registrations(
        &self,
        conn: &mut AnyConnection,
        pending_before: &str,
        limit: u32,
    ) -> RepoResult<Vec<LauncherAccountRecord>> {
        let mut query = live_launchers()
            .filter(launcher_accounts::email_verification_pending_since.lt(pending_before))
            .filter(launcher_accounts::email_verified_at.is_null())
            .order(launcher_accounts::email_verification_pending_since.asc())
            .select((LauncherRow::as_select(), accounts::user_id))
            .into_boxed();
        if limit > 0 {
            query = query.limit(i64::from(limit));
        }
        load_launchers(query.load(conn)?)
    }

    fn delete_unverified_registration(
        &self,
        tx: &mut Tx<'_>,
        id: &AccountId,
        pending_before: &str,
    ) -> RepoResult<bool> {
        let still = launcher_accounts::table
            .filter(launcher_accounts::deleted_at.is_null())
            .filter(launcher_accounts::account_id.eq(id.to_string()))
            .filter(launcher_accounts::email_verification_pending_since.lt(pending_before))
            .filter(launcher_accounts::email_verified_at.is_null())
            .select(launcher_accounts::account_id)
            .first::<String>(tx.conn())
            .optional()?;
        if still.is_none() {
            return Ok(false);
        }
        let key = [("account_id", store::KeyValue::Text(id.to_string()))];
        let gone = tx.soft_delete("launcher_accounts", &key)?;
        tx.soft_delete("accounts", &[("id", store::KeyValue::Text(id.to_string()))])?;
        Ok(gone)
    }

    fn list_for_admin(
        &self,
        conn: &mut AnyConnection,
        filter: &AdminAccountFilter,
    ) -> RepoResult<(Vec<LauncherAccountRecord>, u64)> {
        // Newest first, as Node ordered (createdAt, then insertion order); the search is a case-insensitive
        // substring over display name, email and userId, evaluated here like Node's RegExp on SQLite
        let rows = live_launchers()
            .order((
                launcher_accounts::created_at.desc(),
                launcher_accounts::account_id.desc(),
            ))
            .select((LauncherRow::as_select(), accounts::user_id))
            .load::<(LauncherRow, String)>(conn)?;
        let needle = filter
            .search
            .as_ref()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_lowercase());
        let mut matching = Vec::new();
        for record in load_launchers(rows)? {
            let approval = record.approval_status.unwrap_or(ApprovalStatus::Approved);
            // Legacy rows without the field count as approved
            if filter.approval_status.is_some_and(|wanted| wanted != approval) {
                continue;
            }
            if filter.status.is_some_and(|wanted| wanted != record.status) {
                continue;
            }
            if filter
                .timed_out_at
                .as_deref()
                .is_some_and(|now| record.timeout_at(now).is_none())
            {
                continue;
            }
            if let Some(needle) = &needle {
                let hit = [
                    Some(record.display_name.as_str()),
                    record.email.as_deref(),
                    Some(record.user_id.as_str()),
                ]
                .into_iter()
                .flatten()
                .any(|value| value.to_lowercase().contains(needle.as_str()));
                if !hit {
                    continue;
                }
            }
            matching.push(record);
        }
        let total = matching.len() as u64;
        let page = matching.into_iter().skip(filter.skip as usize);
        let page: Vec<_> = if filter.limit > 0 {
            page.take(filter.limit as usize).collect()
        } else {
            page.collect()
        };
        Ok((page, total))
    }

    fn set_access_state(
        &self,
        tx: &mut Tx<'_>,
        id: &AccountId,
        change: &AccessChange,
    ) -> RepoResult<Option<LauncherAccountRecord>> {
        if find_launcher_by_account(tx.conn(), id)?.is_none() {
            return Ok(None);
        }
        let target = launcher_accounts::table.filter(launcher_accounts::account_id.eq(id.to_string()));
        diesel::update(target)
            .set((
                launcher_accounts::approval_updated_at.eq(Some(&change.approval_updated_at)),
                launcher_accounts::approval_updated_by.eq(Some(&change.approval_updated_by)),
                // Node writes "" when no reason is given
                launcher_accounts::approval_reason.eq(Some(change.approval_reason.clone().unwrap_or_default())),
            ))
            .execute(tx.conn())?;
        if let Some(approval) = change.approval_status {
            diesel::update(launcher_accounts::table.filter(launcher_accounts::account_id.eq(id.to_string())))
                .set(launcher_accounts::approval_status.eq(Some(approval.as_str())))
                .execute(tx.conn())?;
        }
        if let Some(status) = change.status {
            diesel::update(launcher_accounts::table.filter(launcher_accounts::account_id.eq(id.to_string())))
                .set(launcher_accounts::status.eq(status.as_str()))
                .execute(tx.conn())?;
        }
        find_launcher_by_account(tx.conn(), id)
    }

    fn set_timeout(
        &self,
        tx: &mut Tx<'_>,
        id: &AccountId,
        timeout: Option<&AccountTimeout>,
    ) -> RepoResult<Option<LauncherAccountRecord>> {
        if find_launcher_by_account(tx.conn(), id)?.is_none() {
            return Ok(None);
        }
        diesel::update(launcher_accounts::table.filter(launcher_accounts::account_id.eq(id.to_string())))
            .set((
                launcher_accounts::timeout_until.eq(timeout.map(|t| t.until.clone())),
                launcher_accounts::timeout_since.eq(timeout.map(|t| t.since.clone())),
                launcher_accounts::timeout_reason.eq(timeout.and_then(|t| t.reason.clone())),
            ))
            .execute(tx.conn())?;
        find_launcher_by_account(tx.conn(), id)
    }

    fn erase(&self, tx: &mut Tx<'_>, id: &AccountId) -> RepoResult<bool> {
        let launcher = tx.soft_delete(
            "launcher_accounts",
            &[("account_id", store::KeyValue::Text(id.to_string()))],
        )?;
        let game = tx.soft_delete("accounts", &[("id", store::KeyValue::Text(id.to_string()))])?;
        Ok(launcher || game)
    }

    fn update_roles(
        &self,
        tx: &mut Tx<'_>,
        id: &AccountId,
        roles: &[String],
        at: &str,
        by: &str,
    ) -> RepoResult<Option<LauncherAccountRecord>> {
        if find_launcher_by_account(tx.conn(), id)?.is_none() {
            return Ok(None);
        }
        let roles = serde_json::to_string(roles).map_err(|e| RepoError::Data(e.to_string()))?;
        diesel::update(launcher_accounts::table.filter(launcher_accounts::account_id.eq(id.to_string())))
            .set((
                launcher_accounts::roles.eq(roles),
                launcher_accounts::roles_updated_at.eq(Some(at)),
                launcher_accounts::roles_updated_by.eq(Some(by)),
            ))
            .execute(tx.conn())?;
        find_launcher_by_account(tx.conn(), id)
    }
}

// ------------------------------------------------------------------ Discord links

/// `AuthIdentityRecord`: a linked Discord account (the only provider).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthIdentityRecord {
    pub provider_subject: String,
    pub account: AccountId,
    pub provider_username: String,
    pub provider_avatar_url: Option<String>,
    pub linked_at: String,
}

repository! {
    pub trait AuthIdentityRepository("authIdentities") {
        fn find_by_provider_subject(&self, conn: &mut AnyConnection, subject: &str) -> Option<AuthIdentityRecord>;
        fn find_by_account(&self, conn: &mut AnyConnection, account: &AccountId) -> Option<AuthIdentityRecord>;
        /// False when the Discord user (or the account) is linked already: the unique index refused it
        fn create(&self, tx: &mut Tx<'_>, identity: &AuthIdentityRecord) -> bool;
        /// How many went
        fn delete_for_account(&self, tx: &mut Tx<'_>, account: &AccountId) -> u64;
    }
}

const DISCORD: &str = "discord";

type IdentityRow = (String, String, String, Option<String>, String);

fn identity(row: IdentityRow) -> RepoResult<AuthIdentityRecord> {
    let (provider_subject, account, provider_username, provider_avatar_url, linked_at) = row;
    Ok(AuthIdentityRecord {
        provider_subject,
        account: account_id(&account, "auth_identities.account_id")?,
        provider_username,
        provider_avatar_url,
        linked_at,
    })
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SqlAuthIdentities;

impl AuthIdentityRepository for SqlAuthIdentities {
    fn find_by_provider_subject(
        &self,
        conn: &mut AnyConnection,
        subject: &str,
    ) -> RepoResult<Option<AuthIdentityRecord>> {
        auth_identities::table
            .filter(auth_identities::provider.eq(DISCORD))
            .filter(auth_identities::provider_subject.eq(subject))
            .select((
                auth_identities::provider_subject,
                auth_identities::account_id,
                auth_identities::provider_username,
                auth_identities::provider_avatar_url,
                auth_identities::linked_at,
            ))
            .first::<IdentityRow>(conn)
            .optional()?
            .map(identity)
            .transpose()
    }

    fn find_by_account(&self, conn: &mut AnyConnection, id: &AccountId) -> RepoResult<Option<AuthIdentityRecord>> {
        auth_identities::table
            .filter(auth_identities::provider.eq(DISCORD))
            .filter(auth_identities::account_id.eq(id.to_string()))
            .select((
                auth_identities::provider_subject,
                auth_identities::account_id,
                auth_identities::provider_username,
                auth_identities::provider_avatar_url,
                auth_identities::linked_at,
            ))
            .first::<IdentityRow>(conn)
            .optional()?
            .map(identity)
            .transpose()
    }

    fn create(&self, tx: &mut Tx<'_>, record: &AuthIdentityRecord) -> RepoResult<bool> {
        // Node's unique index is (provider, providerSubject); one account has at most one link, which the controller
        // checks first, and the write is serial, so both are checked here as well
        let taken = auth_identities::table
            .filter(auth_identities::provider.eq(DISCORD))
            .filter(
                auth_identities::provider_subject
                    .eq(&record.provider_subject)
                    .or(auth_identities::account_id.eq(record.account.to_string())),
            )
            .select(auth_identities::provider_subject)
            .first::<String>(tx.conn())
            .optional()?;
        if taken.is_some() {
            return Ok(false);
        }
        diesel::insert_into(auth_identities::table)
            .values((
                auth_identities::provider.eq(DISCORD),
                auth_identities::provider_subject.eq(&record.provider_subject),
                auth_identities::account_id.eq(record.account.to_string()),
                auth_identities::provider_username.eq(&record.provider_username),
                auth_identities::provider_avatar_url.eq(&record.provider_avatar_url),
                auth_identities::linked_at.eq(&record.linked_at),
            ))
            .execute(tx.conn())?;
        Ok(true)
    }

    fn delete_for_account(&self, tx: &mut Tx<'_>, id: &AccountId) -> RepoResult<u64> {
        let gone = diesel::delete(
            auth_identities::table
                .filter(auth_identities::provider.eq(DISCORD))
                .filter(auth_identities::account_id.eq(id.to_string())),
        )
        .execute(tx.conn())?;
        Ok(gone as u64)
    }
}

// ------------------------------------------------------------------ refresh sessions

/// `RefreshSessionRecord`: a launcher session's refresh token, by hash.
#[derive(Clone, PartialEq, Eq)]
pub struct RefreshSessionRecord {
    pub id: String,
    pub token_hash: String,
    pub account: AccountId,
    pub family_id: String,
    pub device_id: String,
    pub device_name: String,
    pub created_at: String,
    pub expires_at: String,
    pub revoked_at: Option<String>,
}

impl fmt::Debug for RefreshSessionRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RefreshSessionRecord")
            .field("id", &self.id)
            .field("account", &self.account)
            .field("family_id", &self.family_id)
            .field("revoked_at", &self.revoked_at)
            .finish_non_exhaustive()
    }
}

repository! {
    pub trait RefreshSessionRepository("refreshSessions") {
        fn create(&self, tx: &mut Tx<'_>, session: &RefreshSessionRecord) -> ();
        fn find_by_token_hash(&self, conn: &mut AnyConnection, token_hash: &str) -> Option<RefreshSessionRecord>;
        fn revoke_by_id(&self, tx: &mut Tx<'_>, id: &str, now: &str) -> ();
        fn revoke_family(&self, tx: &mut Tx<'_>, family_id: &str, now: &str) -> ();
        fn revoke_all_for_account(&self, tx: &mut Tx<'_>, account: &AccountId, now: &str) -> ();
        fn delete_all_for_account(&self, tx: &mut Tx<'_>, account: &AccountId) -> ();
        fn is_family_active(&self, conn: &mut AnyConnection, family_id: &str, account: &AccountId, now: &str) -> bool;
        /// Rows past their expiry (Node's TTL purge); how many went
        fn purge_expired(&self, tx: &mut Tx<'_>, now: &str) -> u64;
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SqlRefreshSessions;

impl RefreshSessionRepository for SqlRefreshSessions {
    fn create(&self, tx: &mut Tx<'_>, s: &RefreshSessionRecord) -> RepoResult<()> {
        diesel::insert_into(refresh_sessions::table)
            .values((
                refresh_sessions::id.eq(&s.id),
                refresh_sessions::token_hash.eq(&s.token_hash),
                refresh_sessions::account_id.eq(s.account.to_string()),
                refresh_sessions::family_id.eq(&s.family_id),
                refresh_sessions::device_id.eq(&s.device_id),
                refresh_sessions::device_name.eq(&s.device_name),
                refresh_sessions::created_at.eq(&s.created_at),
                refresh_sessions::expires_at.eq(&s.expires_at),
                refresh_sessions::revoked_at.eq(&s.revoked_at),
            ))
            .execute(tx.conn())?;
        Ok(())
    }

    fn find_by_token_hash(&self, conn: &mut AnyConnection, hash: &str) -> RepoResult<Option<RefreshSessionRecord>> {
        let row = refresh_sessions::table
            .filter(refresh_sessions::token_hash.eq(hash))
            .select((
                refresh_sessions::id,
                refresh_sessions::token_hash,
                refresh_sessions::account_id,
                refresh_sessions::family_id,
                refresh_sessions::device_id,
                refresh_sessions::device_name,
                refresh_sessions::created_at,
                refresh_sessions::expires_at,
                refresh_sessions::revoked_at,
            ))
            .first::<(
                String,
                String,
                String,
                String,
                String,
                String,
                String,
                String,
                Option<String>,
            )>(conn)
            .optional()?;
        row.map(|r| {
            Ok(RefreshSessionRecord {
                id: r.0,
                token_hash: r.1,
                account: account_id(&r.2, "refresh_sessions.account_id")?,
                family_id: r.3,
                device_id: r.4,
                device_name: r.5,
                created_at: r.6,
                expires_at: r.7,
                revoked_at: r.8,
            })
        })
        .transpose()
    }

    fn revoke_by_id(&self, tx: &mut Tx<'_>, id: &str, now: &str) -> RepoResult<()> {
        diesel::update(refresh_sessions::table.filter(refresh_sessions::id.eq(id)))
            .set(refresh_sessions::revoked_at.eq(Some(now)))
            .execute(tx.conn())?;
        Ok(())
    }

    fn revoke_family(&self, tx: &mut Tx<'_>, family: &str, now: &str) -> RepoResult<()> {
        diesel::update(
            refresh_sessions::table
                .filter(refresh_sessions::family_id.eq(family))
                .filter(refresh_sessions::revoked_at.is_null()),
        )
        .set(refresh_sessions::revoked_at.eq(Some(now)))
        .execute(tx.conn())?;
        Ok(())
    }

    fn revoke_all_for_account(&self, tx: &mut Tx<'_>, id: &AccountId, now: &str) -> RepoResult<()> {
        diesel::update(
            refresh_sessions::table
                .filter(refresh_sessions::account_id.eq(id.to_string()))
                .filter(refresh_sessions::revoked_at.is_null()),
        )
        .set(refresh_sessions::revoked_at.eq(Some(now)))
        .execute(tx.conn())?;
        Ok(())
    }

    fn delete_all_for_account(&self, tx: &mut Tx<'_>, id: &AccountId) -> RepoResult<()> {
        diesel::delete(refresh_sessions::table.filter(refresh_sessions::account_id.eq(id.to_string())))
            .execute(tx.conn())?;
        Ok(())
    }

    fn is_family_active(&self, conn: &mut AnyConnection, family: &str, id: &AccountId, now: &str) -> RepoResult<bool> {
        let found = refresh_sessions::table
            .filter(refresh_sessions::family_id.eq(family))
            .filter(refresh_sessions::account_id.eq(id.to_string()))
            .filter(refresh_sessions::revoked_at.is_null())
            .filter(refresh_sessions::expires_at.gt(now))
            .select(refresh_sessions::id)
            .first::<String>(conn)
            .optional()?;
        Ok(found.is_some())
    }

    fn purge_expired(&self, tx: &mut Tx<'_>, now: &str) -> RepoResult<u64> {
        Ok(
            diesel::delete(refresh_sessions::table.filter(refresh_sessions::expires_at.lt(now))).execute(tx.conn())?
                as u64,
        )
    }
}

// ------------------------------------------------------------------ game exchange codes

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeChannel {
    Stable,
    Beta,
    Dev,
}

impl RuntimeChannel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Beta => "beta",
            Self::Dev => "dev",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "stable" => Some(Self::Stable),
            "beta" => Some(Self::Beta),
            "dev" => Some(Self::Dev),
            _ => None,
        }
    }
}

/// `GameExchangeCodeRecord`: the one-time code `POST /launcher/v1/game-sessions` hands the game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameExchangeCodeRecord {
    pub code_hash: String,
    pub account: AccountId,
    pub launcher_session_id: String,
    /// Checked against the module's `TARGET_CHANGELIST`
    pub build_changelist: u32,
    pub executable_sha256: String,
    pub runtime_channel: RuntimeChannel,
    pub runtime_sha256: String,
    pub runtime_manifest_version: String,
    pub runtime_artifact_set_sha256: String,
    pub realm_id: Option<String>,
    pub created_at: String,
    pub expires_at: String,
    pub consumed_at: Option<String>,
}

repository! {
    pub trait GameExchangeCodeRepository("gameExchangeCodes") {
        fn create(&self, tx: &mut Tx<'_>, code: &GameExchangeCodeRecord) -> ();
        /// Find-and-consume: succeeds at most once per code, and only before expiry
        fn consume_by_code_hash(&self, tx: &mut Tx<'_>, code_hash: &str, now: &str) -> Option<GameExchangeCodeRecord>;
        fn revoke_unused_for_account(&self, tx: &mut Tx<'_>, account: &AccountId, now: &str) -> ();
        fn delete_for_account(&self, tx: &mut Tx<'_>, account: &AccountId) -> u64;
        fn purge_expired(&self, tx: &mut Tx<'_>, now: &str) -> u64;
    }
}

#[derive(Queryable, Selectable)]
#[diesel(table_name = game_exchange_codes)]
struct ExchangeRow {
    code_hash: String,
    account_id: String,
    launcher_session_id: String,
    build_changelist: i64,
    executable_sha256: String,
    runtime_channel: String,
    runtime_sha256: String,
    runtime_manifest_version: String,
    runtime_artifact_set_sha256: String,
    realm_id: Option<String>,
    created_at: String,
    expires_at: String,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SqlGameExchangeCodes;

impl GameExchangeCodeRepository for SqlGameExchangeCodes {
    fn create(&self, tx: &mut Tx<'_>, c: &GameExchangeCodeRecord) -> RepoResult<()> {
        diesel::insert_into(game_exchange_codes::table)
            .values((
                game_exchange_codes::code_hash.eq(&c.code_hash),
                game_exchange_codes::account_id.eq(c.account.to_string()),
                game_exchange_codes::launcher_session_id.eq(&c.launcher_session_id),
                game_exchange_codes::build_changelist.eq(i64::from(c.build_changelist)),
                game_exchange_codes::executable_sha256.eq(&c.executable_sha256),
                game_exchange_codes::runtime_channel.eq(c.runtime_channel.as_str()),
                game_exchange_codes::runtime_sha256.eq(&c.runtime_sha256),
                game_exchange_codes::runtime_manifest_version.eq(&c.runtime_manifest_version),
                game_exchange_codes::runtime_artifact_set_sha256.eq(&c.runtime_artifact_set_sha256),
                game_exchange_codes::realm_id.eq(&c.realm_id),
                game_exchange_codes::created_at.eq(&c.created_at),
                game_exchange_codes::expires_at.eq(&c.expires_at),
                game_exchange_codes::consumed_at.eq(&c.consumed_at),
            ))
            .execute(tx.conn())?;
        Ok(())
    }

    fn consume_by_code_hash(
        &self,
        tx: &mut Tx<'_>,
        hash: &str,
        now: &str,
    ) -> RepoResult<Option<GameExchangeCodeRecord>> {
        let row = game_exchange_codes::table
            .filter(game_exchange_codes::code_hash.eq(hash))
            .filter(game_exchange_codes::consumed_at.is_null())
            .filter(game_exchange_codes::expires_at.gt(now))
            .select(ExchangeRow::as_select())
            .first::<ExchangeRow>(tx.conn())
            .optional()?;
        let Some(row) = row else { return Ok(None) };
        diesel::update(game_exchange_codes::table.filter(game_exchange_codes::code_hash.eq(hash)))
            .set(game_exchange_codes::consumed_at.eq(Some(now)))
            .execute(tx.conn())?;
        let channel = RuntimeChannel::parse(&row.runtime_channel)
            .ok_or_else(|| RepoError::Data(format!("game_exchange_codes.runtime_channel {:?}", row.runtime_channel)))?;
        Ok(Some(GameExchangeCodeRecord {
            code_hash: row.code_hash,
            account: account_id(&row.account_id, "game_exchange_codes.account_id")?,
            launcher_session_id: row.launcher_session_id,
            build_changelist: u32::try_from(row.build_changelist)
                .map_err(|_| RepoError::Data("game_exchange_codes.build_changelist".into()))?,
            executable_sha256: row.executable_sha256,
            runtime_channel: channel,
            runtime_sha256: row.runtime_sha256,
            runtime_manifest_version: row.runtime_manifest_version,
            runtime_artifact_set_sha256: row.runtime_artifact_set_sha256,
            realm_id: row.realm_id,
            created_at: row.created_at,
            expires_at: row.expires_at,
            consumed_at: Some(now.to_owned()),
        }))
    }

    fn revoke_unused_for_account(&self, tx: &mut Tx<'_>, id: &AccountId, now: &str) -> RepoResult<()> {
        diesel::update(
            game_exchange_codes::table
                .filter(game_exchange_codes::account_id.eq(id.to_string()))
                .filter(game_exchange_codes::consumed_at.is_null()),
        )
        .set(game_exchange_codes::consumed_at.eq(Some(now)))
        .execute(tx.conn())?;
        Ok(())
    }

    fn delete_for_account(&self, tx: &mut Tx<'_>, id: &AccountId) -> RepoResult<u64> {
        Ok(
            diesel::delete(game_exchange_codes::table.filter(game_exchange_codes::account_id.eq(id.to_string())))
                .execute(tx.conn())? as u64,
        )
    }

    fn purge_expired(&self, tx: &mut Tx<'_>, now: &str) -> RepoResult<u64> {
        Ok(
            diesel::delete(game_exchange_codes::table.filter(game_exchange_codes::expires_at.lt(now)))
                .execute(tx.conn())? as u64,
        )
    }
}

// ------------------------------------------------------------------ Discord OAuth transactions

/// `DiscordOAuthTransactionRecord`.
#[derive(Clone, PartialEq, Eq)]
pub struct DiscordOAuthTransactionRecord {
    pub state: String,
    pub code_verifier: String,
    pub created_at: String,
    pub expires_at: String,
    pub consumed_at: Option<String>,
    pub completion_code_hash: Option<String>,
    pub completion_expires_at: Option<String>,
    pub completion_consumed_at: Option<String>,
    pub account: Option<AccountId>,
    pub link_account: Option<AccountId>,
}

impl fmt::Debug for DiscordOAuthTransactionRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiscordOAuthTransactionRecord")
            .field("expires_at", &self.expires_at)
            .field("account", &self.account)
            .field("link_account", &self.link_account)
            .finish_non_exhaustive()
    }
}

repository! {
    pub trait DiscordOAuthTransactionRepository("discordOAuthTransactions") {
        fn create(&self, tx: &mut Tx<'_>, transaction: &DiscordOAuthTransactionRecord) -> ();
        fn consume_by_state(&self, tx: &mut Tx<'_>, state: &str, now: &str) -> Option<DiscordOAuthTransactionRecord>;
        fn attach_completion_code(&self, tx: &mut Tx<'_>, state: &str, code_hash: &str, expires_at: &str,
            account: &AccountId) -> ();
        fn consume_by_completion_code_hash(&self, tx: &mut Tx<'_>, code_hash: &str, now: &str)
            -> Option<DiscordOAuthTransactionRecord>;
        /// Rows past the later of their two expiries (Node's TTL mirror, with its hour of grace)
        fn purge_expired(&self, tx: &mut Tx<'_>, before: &str) -> u64;
        /// The transactions that signed `account` in or were to link Discord to it
        fn delete_for_account(&self, tx: &mut Tx<'_>, account: &AccountId) -> u64;
    }
}

#[derive(Queryable, Selectable)]
#[diesel(table_name = discord_oauth_transactions)]
struct OAuthRow {
    state: String,
    code_verifier: String,
    created_at: String,
    expires_at: String,
    consumed_at: Option<String>,
    completion_code_hash: Option<String>,
    completion_expires_at: Option<String>,
    completion_consumed_at: Option<String>,
    account_id: Option<String>,
    link_account_id: Option<String>,
}

fn oauth_record(row: OAuthRow) -> RepoResult<DiscordOAuthTransactionRecord> {
    Ok(DiscordOAuthTransactionRecord {
        state: row.state,
        code_verifier: row.code_verifier,
        created_at: row.created_at,
        expires_at: row.expires_at,
        consumed_at: row.consumed_at,
        completion_code_hash: row.completion_code_hash,
        completion_expires_at: row.completion_expires_at,
        completion_consumed_at: row.completion_consumed_at,
        account: optional_account(row.account_id, "discord_oauth_transactions.account_id")?,
        link_account: optional_account(row.link_account_id, "discord_oauth_transactions.link_account_id")?,
    })
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SqlDiscordOAuthTransactions;

impl DiscordOAuthTransactionRepository for SqlDiscordOAuthTransactions {
    fn create(&self, tx: &mut Tx<'_>, t: &DiscordOAuthTransactionRecord) -> RepoResult<()> {
        diesel::insert_into(discord_oauth_transactions::table)
            .values((
                discord_oauth_transactions::state.eq(&t.state),
                discord_oauth_transactions::code_verifier.eq(&t.code_verifier),
                discord_oauth_transactions::created_at.eq(&t.created_at),
                discord_oauth_transactions::expires_at.eq(&t.expires_at),
                discord_oauth_transactions::consumed_at.eq(&t.consumed_at),
                discord_oauth_transactions::completion_code_hash.eq(&t.completion_code_hash),
                discord_oauth_transactions::completion_expires_at.eq(&t.completion_expires_at),
                discord_oauth_transactions::completion_consumed_at.eq(&t.completion_consumed_at),
                discord_oauth_transactions::account_id.eq(t.account.as_ref().map(ToString::to_string)),
                discord_oauth_transactions::link_account_id.eq(t.link_account.as_ref().map(ToString::to_string)),
            ))
            .execute(tx.conn())?;
        Ok(())
    }

    fn consume_by_state(
        &self,
        tx: &mut Tx<'_>,
        state: &str,
        now: &str,
    ) -> RepoResult<Option<DiscordOAuthTransactionRecord>> {
        let row = discord_oauth_transactions::table
            .filter(discord_oauth_transactions::state.eq(state))
            .filter(discord_oauth_transactions::consumed_at.is_null())
            .filter(discord_oauth_transactions::expires_at.gt(now))
            .select(OAuthRow::as_select())
            .first::<OAuthRow>(tx.conn())
            .optional()?;
        let Some(row) = row else { return Ok(None) };
        diesel::update(discord_oauth_transactions::table.filter(discord_oauth_transactions::state.eq(state)))
            .set(discord_oauth_transactions::consumed_at.eq(Some(now)))
            .execute(tx.conn())?;
        let mut record = oauth_record(row)?;
        record.consumed_at = Some(now.to_owned());
        Ok(Some(record))
    }

    fn attach_completion_code(
        &self,
        tx: &mut Tx<'_>,
        state: &str,
        code_hash: &str,
        expires_at: &str,
        id: &AccountId,
    ) -> RepoResult<()> {
        diesel::update(discord_oauth_transactions::table.filter(discord_oauth_transactions::state.eq(state)))
            .set((
                discord_oauth_transactions::completion_code_hash.eq(Some(code_hash)),
                discord_oauth_transactions::completion_expires_at.eq(Some(expires_at)),
                discord_oauth_transactions::account_id.eq(Some(id.to_string())),
            ))
            .execute(tx.conn())?;
        Ok(())
    }

    fn consume_by_completion_code_hash(
        &self,
        tx: &mut Tx<'_>,
        code_hash: &str,
        now: &str,
    ) -> RepoResult<Option<DiscordOAuthTransactionRecord>> {
        let row = discord_oauth_transactions::table
            .filter(discord_oauth_transactions::completion_code_hash.eq(code_hash))
            .filter(discord_oauth_transactions::completion_expires_at.gt(now))
            .filter(discord_oauth_transactions::completion_consumed_at.is_null())
            .select(OAuthRow::as_select())
            .first::<OAuthRow>(tx.conn())
            .optional()?;
        let Some(row) = row else { return Ok(None) };
        diesel::update(discord_oauth_transactions::table.filter(discord_oauth_transactions::state.eq(&row.state)))
            .set(discord_oauth_transactions::completion_consumed_at.eq(Some(now)))
            .execute(tx.conn())?;
        let mut record = oauth_record(row)?;
        record.completion_consumed_at = Some(now.to_owned());
        Ok(Some(record))
    }

    fn delete_for_account(&self, tx: &mut Tx<'_>, id: &AccountId) -> RepoResult<u64> {
        let id = id.to_string();
        Ok(diesel::delete(
            discord_oauth_transactions::table.filter(
                discord_oauth_transactions::account_id
                    .eq(Some(&id))
                    .or(discord_oauth_transactions::link_account_id.eq(Some(&id))),
            ),
        )
        .execute(tx.conn())? as u64)
    }

    fn purge_expired(&self, tx: &mut Tx<'_>, before: &str) -> RepoResult<u64> {
        let gone = diesel::delete(
            discord_oauth_transactions::table
                .filter(discord_oauth_transactions::expires_at.lt(before))
                .filter(
                    discord_oauth_transactions::completion_expires_at
                        .is_null()
                        .or(discord_oauth_transactions::completion_expires_at.lt(before)),
                ),
        )
        .execute(tx.conn())?;
        Ok(gone as u64)
    }
}

// ------------------------------------------------------------------ email codes

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailCodePurpose {
    Verify,
    Reset,
    Unlink,
}

impl EmailCodePurpose {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Verify => "verify",
            Self::Reset => "reset",
            Self::Unlink => "unlink",
        }
    }
}

/// `EmailCodeRecord`: the live one-time code of email verification, password reset and the Discord unlink.
#[derive(Clone, PartialEq, Eq)]
pub struct EmailCodeRecord {
    pub account: AccountId,
    pub purpose: EmailCodePurpose,
    pub code_hash: String,
    pub attempts: u32,
    pub created_at: String,
    pub expires_at: String,
}

impl fmt::Debug for EmailCodeRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EmailCodeRecord")
            .field("account", &self.account)
            .field("purpose", &self.purpose)
            .field("attempts", &self.attempts)
            .finish_non_exhaustive()
    }
}

repository! {
    pub trait EmailCodeRepository("emailCodes") {
        /// Stores the live code of the account and purpose, replacing any issued before it
        fn replace(&self, tx: &mut Tx<'_>, record: &EmailCodeRecord) -> ();
        /// A read only: the code while it has not expired and has tries left
        fn peek_live(&self, conn: &mut AnyConnection, account: &AccountId, purpose: EmailCodePurpose, now: &str,
            max_tries: u32) -> Option<EmailCodeRecord>;
        /// Removes the row only while it still holds this hash, unexpired and with tries left; whether it did
        fn consume(&self, tx: &mut Tx<'_>, account: &AccountId, purpose: EmailCodePurpose, code_hash: &str,
            now: &str, max_tries: u32) -> bool;
        /// Counts one try against the live code; an expired or used-up row goes and answers `None`
        fn reserve_try(&self, tx: &mut Tx<'_>, account: &AccountId, purpose: EmailCodePurpose, now: &str,
            max_tries: u32) -> Option<EmailCodeRecord>;
        fn remove(&self, tx: &mut Tx<'_>, account: &AccountId, purpose: EmailCodePurpose, code_hash: &str) -> bool;
        fn delete_for_account(&self, tx: &mut Tx<'_>, account: &AccountId) -> u64;
        fn purge_expired(&self, tx: &mut Tx<'_>, now: &str) -> u64;
    }
}

type EmailRow = (String, String, String, i64, String, String);

fn email_row(account: &AccountId, purpose: EmailCodePurpose, row: EmailRow) -> EmailCodeRecord {
    EmailCodeRecord {
        account: account.clone(),
        purpose,
        code_hash: row.2,
        attempts: u32::try_from(row.3).unwrap_or(u32::MAX),
        created_at: row.4,
        expires_at: row.5,
    }
}

fn find_email_code(
    conn: &mut AnyConnection,
    id: &AccountId,
    purpose: EmailCodePurpose,
) -> RepoResult<Option<EmailCodeRecord>> {
    let row = email_codes::table
        .filter(email_codes::account_id.eq(id.to_string()))
        .filter(email_codes::purpose.eq(purpose.as_str()))
        .select((
            email_codes::account_id,
            email_codes::purpose,
            email_codes::code_hash,
            email_codes::attempts,
            email_codes::created_at,
            email_codes::expires_at,
        ))
        .first::<EmailRow>(conn)
        .optional()?;
    Ok(row.map(|row| email_row(id, purpose, row)))
}

fn delete_email_code(tx: &mut Tx<'_>, id: &AccountId, purpose: EmailCodePurpose) -> RepoResult<usize> {
    Ok(diesel::delete(
        email_codes::table
            .filter(email_codes::account_id.eq(id.to_string()))
            .filter(email_codes::purpose.eq(purpose.as_str())),
    )
    .execute(tx.conn())?)
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SqlEmailCodes;

impl EmailCodeRepository for SqlEmailCodes {
    fn replace(&self, tx: &mut Tx<'_>, r: &EmailCodeRecord) -> RepoResult<()> {
        delete_email_code(tx, &r.account, r.purpose)?;
        diesel::insert_into(email_codes::table)
            .values((
                email_codes::account_id.eq(r.account.to_string()),
                email_codes::purpose.eq(r.purpose.as_str()),
                email_codes::code_hash.eq(&r.code_hash),
                email_codes::attempts.eq(i64::from(r.attempts)),
                email_codes::created_at.eq(&r.created_at),
                email_codes::expires_at.eq(&r.expires_at),
            ))
            .execute(tx.conn())?;
        Ok(())
    }

    fn peek_live(
        &self,
        conn: &mut AnyConnection,
        id: &AccountId,
        purpose: EmailCodePurpose,
        now: &str,
        max_tries: u32,
    ) -> RepoResult<Option<EmailCodeRecord>> {
        Ok(find_email_code(conn, id, purpose)?
            .filter(|record| record.expires_at.as_str() > now && record.attempts < max_tries))
    }

    fn consume(
        &self,
        tx: &mut Tx<'_>,
        id: &AccountId,
        purpose: EmailCodePurpose,
        code_hash: &str,
        now: &str,
        max_tries: u32,
    ) -> RepoResult<bool> {
        let live = find_email_code(tx.conn(), id, purpose)?.filter(|record| {
            record.code_hash == code_hash && record.expires_at.as_str() > now && record.attempts < max_tries
        });
        if live.is_none() {
            return Ok(false);
        }
        Ok(delete_email_code(tx, id, purpose)? == 1)
    }

    fn reserve_try(
        &self,
        tx: &mut Tx<'_>,
        id: &AccountId,
        purpose: EmailCodePurpose,
        now: &str,
        max_tries: u32,
    ) -> RepoResult<Option<EmailCodeRecord>> {
        let Some(mut record) = find_email_code(tx.conn(), id, purpose)? else {
            return Ok(None);
        };
        if record.expires_at.as_str() > now && record.attempts < max_tries {
            diesel::update(
                email_codes::table
                    .filter(email_codes::account_id.eq(id.to_string()))
                    .filter(email_codes::purpose.eq(purpose.as_str())),
            )
            .set(email_codes::attempts.eq(i64::from(record.attempts) + 1))
            .execute(tx.conn())?;
            record.attempts += 1;
            return Ok(Some(record));
        }
        // Expired or out of tries: the row goes now
        delete_email_code(tx, id, purpose)?;
        Ok(None)
    }

    fn remove(&self, tx: &mut Tx<'_>, id: &AccountId, purpose: EmailCodePurpose, code_hash: &str) -> RepoResult<bool> {
        match find_email_code(tx.conn(), id, purpose)? {
            Some(record) if record.code_hash == code_hash => Ok(delete_email_code(tx, id, purpose)? == 1),
            _ => Ok(false),
        }
    }

    fn delete_for_account(&self, tx: &mut Tx<'_>, id: &AccountId) -> RepoResult<u64> {
        Ok(
            diesel::delete(email_codes::table.filter(email_codes::account_id.eq(id.to_string()))).execute(tx.conn())?
                as u64,
        )
    }

    fn purge_expired(&self, tx: &mut Tx<'_>, now: &str) -> RepoResult<u64> {
        Ok(diesel::delete(email_codes::table.filter(email_codes::expires_at.lt(now))).execute(tx.conn())? as u64)
    }
}

// ------------------------------------------------------------------ API keys

/// `GameServerApiKeyRecord`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameServerKeyHash {
    pub id: i64,
    pub key_hash: Option<String>,
}

/// `UserApiKeyRecord`.
#[derive(Clone, PartialEq, Eq)]
pub struct UserKeyHash {
    pub account: AccountId,
    pub key_hash: String,
}

impl fmt::Debug for UserKeyHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UserKeyHash")
            .field("account", &self.account)
            .finish_non_exhaustive()
    }
}

repository! {
    pub trait ApiKeyRepository("apiKeys") {
        fn find_all_game_server_key_hashes(&self, conn: &mut AnyConnection) -> Vec<GameServerKeyHash>;
        /// Adds the hash unless it is stored already
        fn insert_game_server_key_hash(&self, tx: &mut Tx<'_>, key_hash: &str) -> ();
        /// Makes these the complete gameserver key set: adds the missing, deletes the rest
        fn replace_game_server_key_hashes(&self, tx: &mut Tx<'_>, key_hashes: &[String]) -> ();
        fn find_all_user_key_hashes(&self, conn: &mut AnyConnection) -> Vec<UserKeyHash>;
        /// One key per account: a second replaces the first
        fn insert_user_key_hash(&self, tx: &mut Tx<'_>, account: &AccountId, key_hash: &str) -> ();
        /// The account's key (its erasure); whether there was one
        fn delete_user_key_hash(&self, tx: &mut Tx<'_>, account: &AccountId) -> bool;
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SqlApiKeys;

impl ApiKeyRepository for SqlApiKeys {
    fn find_all_game_server_key_hashes(&self, conn: &mut AnyConnection) -> RepoResult<Vec<GameServerKeyHash>> {
        let hashes = game_server_api_keys::table
            .select(game_server_api_keys::key_hash)
            .load::<String>(conn)?;
        Ok(hashes
            .into_iter()
            .enumerate()
            .map(|(index, hash)| GameServerKeyHash {
                id: index as i64,
                key_hash: Some(hash),
            })
            .collect())
    }

    fn insert_game_server_key_hash(&self, tx: &mut Tx<'_>, key_hash: &str) -> RepoResult<()> {
        let there = game_server_api_keys::table
            .filter(game_server_api_keys::key_hash.eq(key_hash))
            .select(game_server_api_keys::key_hash)
            .first::<String>(tx.conn())
            .optional()?;
        if there.is_none() {
            diesel::insert_into(game_server_api_keys::table)
                .values(game_server_api_keys::key_hash.eq(key_hash))
                .execute(tx.conn())?;
        }
        Ok(())
    }

    fn replace_game_server_key_hashes(&self, tx: &mut Tx<'_>, key_hashes: &[String]) -> RepoResult<()> {
        if key_hashes.is_empty() {
            diesel::delete(game_server_api_keys::table).execute(tx.conn())?;
            return Ok(());
        }
        for hash in key_hashes {
            self.insert_game_server_key_hash(tx, hash)?;
        }
        diesel::delete(game_server_api_keys::table.filter(game_server_api_keys::key_hash.ne_all(key_hashes)))
            .execute(tx.conn())?;
        Ok(())
    }

    fn find_all_user_key_hashes(&self, conn: &mut AnyConnection) -> RepoResult<Vec<UserKeyHash>> {
        let rows = user_api_keys::table
            .select((user_api_keys::account_id, user_api_keys::key_hash))
            .load::<(String, String)>(conn)?;
        rows.into_iter()
            .map(|(account, key_hash)| {
                Ok(UserKeyHash {
                    account: account_id(&account, "user_api_keys.account_id")?,
                    key_hash,
                })
            })
            .collect()
    }

    fn delete_user_key_hash(&self, tx: &mut Tx<'_>, id: &AccountId) -> RepoResult<bool> {
        Ok(
            diesel::delete(user_api_keys::table.filter(user_api_keys::account_id.eq(id.to_string())))
                .execute(tx.conn())?
                > 0,
        )
    }

    fn insert_user_key_hash(&self, tx: &mut Tx<'_>, id: &AccountId, key_hash: &str) -> RepoResult<()> {
        diesel::delete(user_api_keys::table.filter(user_api_keys::account_id.eq(id.to_string()))).execute(tx.conn())?;
        diesel::insert_into(user_api_keys::table)
            .values((
                user_api_keys::account_id.eq(id.to_string()),
                user_api_keys::key_hash.eq(key_hash),
            ))
            .execute(tx.conn())?;
        Ok(())
    }
}
