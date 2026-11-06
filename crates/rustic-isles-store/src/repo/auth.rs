// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Accounts and sign-in (spec §6). Records holding a hash, code, verifier or password hash keep it out of
//! `Debug` (§14).

use std::fmt;

use diesel::QueryableByName;
use diesel::sql_types::{BigInt, Nullable, Text};

use crate::sql::{account_id, count, limit_clause, opt_account_id, sql};
use crate::{AccountId, AnyConnection, RepoError, RepoResult, Tx, UserId};

// ------------------------------------------------------------------ records

/// An `accounts` row: the game-facing account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRecord {
    /// The network-wide account id.
    pub id: AccountId,
    /// The userId the game and its tokens carry.
    pub user_id: UserId,
    /// The display name the game shows.
    pub name: String,
    /// The `notes` counter; 0 for a new account.
    pub notes: i64,
}

/// `launcher_accounts.approval_status`: a closed-test access request's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalStatus {
    /// Waiting for staff.
    Pending,
    /// Let in.
    Approved,
    /// Turned down.
    Rejected,
}

impl ApprovalStatus {
    /// The stored text.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
        }
    }

    /// The state a stored text names; None for any other text.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "pending" => Some(Self::Pending),
            "approved" => Some(Self::Approved),
            "rejected" => Some(Self::Rejected),
            _ => None,
        }
    }
}

/// `launcher_accounts.status`: whether an account may sign in at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationalStatus {
    /// In good standing.
    Active,
    /// Banned by staff.
    Banned,
    /// Disabled by staff.
    Disabled,
}

impl OperationalStatus {
    /// The stored text.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Banned => "banned",
            Self::Disabled => "disabled",
        }
    }

    /// The status a stored text names; None for any other text.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "active" => Some(Self::Active),
            "banned" => Some(Self::Banned),
            "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }
}

/// The Terms and Privacy versions an account accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherConsent {
    /// The Terms of Use version accepted.
    pub terms_version: String,
    /// The Privacy Policy version accepted.
    pub privacy_version: String,
    /// When, as ISO text.
    pub accepted_at: String,
}

/// A scheduled account deletion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherDeletion {
    /// When it was asked for, as ISO text.
    pub requested_at: String,
    /// When the erasure is due, as ISO text.
    pub scheduled_for: String,
}

/// A staff timeout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountTimeout {
    /// When it ends, as ISO text.
    pub until: String,
    /// When it began, as ISO text.
    pub since: String,
    /// The reason shown to the player, if staff gave one.
    pub reason: Option<String>,
}

/// A live `launcher_accounts` row joined to its `accounts` row; Debug leaves out the password hash.
#[derive(Clone, PartialEq, Eq)]
pub struct LauncherAccountRecord {
    /// The account.
    pub account: AccountId,
    /// The userId, from `accounts`.
    pub user_id: UserId,
    /// The address, normalized; None for a Discord-only account.
    pub email: Option<String>,
    /// The username normalized for uniqueness.
    pub display_name_normalized: String,
    /// The username as chosen.
    pub display_name: String,
    /// The Argon2id hash; None for a Discord-only account.
    pub password_hash: Option<String>,
    /// Whether the account may sign in.
    pub status: OperationalStatus,
    /// The access request's state; None is a legacy row, read as approved.
    pub approval_status: Option<ApprovalStatus>,
    /// When staff last changed the access state, as ISO text.
    pub approval_updated_at: Option<String>,
    /// Who last changed the access state.
    pub approval_updated_by: Option<String>,
    /// The reason staff gave.
    pub approval_reason: Option<String>,
    /// Staff roles, such as `admin`.
    pub roles: Vec<String>,
    /// When the roles last changed, as ISO text.
    pub roles_updated_at: Option<String>,
    /// Who last changed the roles.
    pub roles_updated_by: Option<String>,
    /// When the account was registered, as ISO text.
    pub created_at: String,
    /// The last launcher sign-in, as ISO text.
    pub last_login_at: Option<String>,
    /// Whether a username was chosen; None is a legacy row, read as set.
    pub username_set: Option<bool>,
    /// The accepted consent, if any.
    pub consent: Option<LauncherConsent>,
    /// The scheduled deletion, if any.
    pub deletion: Option<LauncherDeletion>,
    /// When the address was verified, as ISO text.
    pub email_verified_at: Option<String>,
    /// When a required verification began, as ISO text.
    pub email_verification_pending_since: Option<String>,
    /// The staff timeout, if any; it may have ended.
    pub timeout: Option<AccountTimeout>,
}

impl LauncherAccountRecord {
    /// The timeout while it runs at `now` (ISO text order is time order).
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

/// The admin list's filters and page; an unset filter matches every account.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdminAccountFilter {
    /// Only accounts in this access state (a legacy row reads as approved).
    pub approval_status: Option<ApprovalStatus>,
    /// Only accounts with this status.
    pub status: Option<OperationalStatus>,
    /// Only accounts timed out at this ISO time.
    pub timed_out_at: Option<String>,
    /// Text matched case-insensitively against the username, address and userId.
    pub search: Option<String>,
    /// Matching accounts to skip, newest first.
    pub skip: u32,
    /// The page's size; 0 is every match.
    pub limit: u32,
}

/// A staff change of an account's access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessChange {
    /// The new access state; None keeps it.
    pub approval_status: Option<ApprovalStatus>,
    /// The new status; None keeps it.
    pub status: Option<OperationalStatus>,
    /// When, as ISO text.
    pub approval_updated_at: String,
    /// The staff member who made it.
    pub approval_updated_by: String,
    /// The reason given.
    pub approval_reason: Option<String>,
}

/// What creating or renaming a launcher account did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateOutcome {
    /// The account was written.
    Created,
    /// Another live account has the address.
    EmailTaken,
    /// Another live account has the username.
    DisplayNameTaken,
}

/// An `auth_identities` row: a linked Discord account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthIdentityRecord {
    /// The Discord user id.
    pub provider_subject: String,
    /// The linked account.
    pub account: AccountId,
    /// The Discord username when linked.
    pub provider_username: String,
    /// The avatar URL when linked.
    pub provider_avatar_url: Option<String>,
    /// When it was linked, as ISO text.
    pub linked_at: String,
}

/// A `refresh_sessions` row: one launcher refresh token; Debug leaves out the hash.
#[derive(Clone, PartialEq, Eq)]
pub struct RefreshSessionRecord {
    /// The session's UUID.
    pub id: String,
    /// SHA-256 of the refresh token.
    pub token_hash: String,
    /// The account it signs in.
    pub account: AccountId,
    /// The rotation family.
    pub family_id: String,
    /// The device id.
    pub device_id: String,
    /// The device name.
    pub device_name: String,
    /// When it was issued, as ISO text.
    pub created_at: String,
    /// When it lapses, as ISO text.
    pub expires_at: String,
    /// When it was revoked, as ISO text.
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

/// A runtime release channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeChannel {
    /// The released runtime.
    Stable,
    /// The beta runtime.
    Beta,
    /// The development runtime.
    Dev,
}

impl RuntimeChannel {
    /// The stored text.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Beta => "beta",
            Self::Dev => "dev",
        }
    }

    /// The channel a stored text names; None for any other text.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "stable" => Some(Self::Stable),
            "beta" => Some(Self::Beta),
            "dev" => Some(Self::Dev),
            _ => None,
        }
    }
}

/// A `game_exchange_codes` row: the one-time code the launcher hands the game; Debug leaves out the hash.
#[derive(Clone, PartialEq, Eq)]
pub struct GameExchangeCodeRecord {
    /// SHA-256 of the code.
    pub code_hash: String,
    /// The account it signs in.
    pub account: AccountId,
    /// The launcher refresh family that asked for it.
    pub launcher_session_id: String,
    /// The game build's changelist.
    pub build_changelist: u32,
    /// The game executable's SHA-256.
    pub executable_sha256: String,
    /// The runtime's channel.
    pub runtime_channel: RuntimeChannel,
    /// The runtime's main DLL hash.
    pub runtime_sha256: String,
    /// The runtime manifest's version.
    pub runtime_manifest_version: String,
    /// The approved artifact set's digest.
    pub runtime_artifact_set_sha256: String,
    /// The realm the game joins, if one was named.
    pub realm_id: Option<String>,
    /// When it was issued, as ISO text.
    pub created_at: String,
    /// When it lapses, as ISO text.
    pub expires_at: String,
    /// When it was used, as ISO text.
    pub consumed_at: Option<String>,
}

impl fmt::Debug for GameExchangeCodeRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GameExchangeCodeRecord")
            .field("account", &self.account)
            .field("launcher_session_id", &self.launcher_session_id)
            .field("build_changelist", &self.build_changelist)
            .field("runtime_channel", &self.runtime_channel)
            .field("realm_id", &self.realm_id)
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .field("consumed_at", &self.consumed_at)
            .finish_non_exhaustive()
    }
}

/// A `discord_oauth_transactions` row: one Discord sign-in or link; Debug leaves out the secrets.
#[derive(Clone, PartialEq, Eq)]
pub struct DiscordOAuthTransactionRecord {
    /// The OAuth `state`.
    pub state: String,
    /// The PKCE verifier.
    pub code_verifier: String,
    /// When it began, as ISO text.
    pub created_at: String,
    /// When the `state` lapses, as ISO text.
    pub expires_at: String,
    /// When the callback used the `state`, as ISO text.
    pub consumed_at: Option<String>,
    /// SHA-256 of the completion code the launcher redeems.
    pub completion_code_hash: Option<String>,
    /// When the completion code lapses, as ISO text.
    pub completion_expires_at: Option<String>,
    /// When the completion code was used, as ISO text.
    pub completion_consumed_at: Option<String>,
    /// The account a sign-in completed for.
    pub account: Option<AccountId>,
    /// The signed-in account a link transaction links.
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

/// What an emailed code is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EmailCodePurpose {
    /// Verifying the address.
    Verify,
    /// Resetting the password.
    Reset,
    /// Unlinking Discord.
    Unlink,
}

impl EmailCodePurpose {
    /// The stored text.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Verify => "verify",
            Self::Reset => "reset",
            Self::Unlink => "unlink",
        }
    }

    /// The purpose a stored text names; None for any other text.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "verify" => Some(Self::Verify),
            "reset" => Some(Self::Reset),
            "unlink" => Some(Self::Unlink),
            _ => None,
        }
    }
}

/// An `email_codes` row; Debug leaves out the hash.
#[derive(Clone, PartialEq, Eq)]
pub struct EmailCodeRecord {
    /// The account it was sent for.
    pub account: AccountId,
    /// What it is for.
    pub purpose: EmailCodePurpose,
    /// The code's keyed hash.
    pub code_hash: String,
    /// Wrong entries so far.
    pub attempts: u32,
    /// When it was sent, as ISO text.
    pub created_at: String,
    /// When it lapses, as ISO text.
    pub expires_at: String,
}

impl fmt::Debug for EmailCodeRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EmailCodeRecord")
            .field("account", &self.account)
            .field("purpose", &self.purpose)
            .field("attempts", &self.attempts)
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// Node's record shape: `id` is the row's 0-based position in the read, `key_hash` always some.
#[derive(Clone, PartialEq, Eq)]
pub struct GameServerKeyHash {
    /// The row's 0-based position in the read.
    pub id: i64,
    /// The key's hash.
    pub key_hash: Option<String>,
}

impl fmt::Debug for GameServerKeyHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GameServerKeyHash")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// A `user_api_keys` row; Debug leaves out the hash.
#[derive(Clone, PartialEq, Eq)]
pub struct UserKeyHash {
    /// The account the key signs in.
    pub account: AccountId,
    /// The key's hash.
    pub key_hash: String,
}

impl fmt::Debug for UserKeyHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UserKeyHash")
            .field("account", &self.account)
            .finish_non_exhaustive()
    }
}

// ------------------------------------------------------------------ traits

store_trait! {
    /// Game accounts (`accounts`).
    pub trait AccountRepository = "accounts" {
        /// The live account with that userId.
        fn find_by_user_id(&self, conn: &mut AnyConnection, user_id: &UserId) -> Option<AccountRecord>;
        /// The live account with that id.
        fn find(&self, conn: &mut AnyConnection, id: &AccountId) -> Option<AccountRecord>;
        /// Writes a new account.
        fn create(&self, tx: &mut Tx<'_>, record: &AccountRecord) -> ();
    }
}

store_trait! {
    /// Launcher accounts (`launcher_accounts`); every read sees live rows only.
    pub trait LauncherAccountRepository = "launcherAccounts" {
        /// The account with that userId.
        fn find_by_user_id(&self, conn: &mut AnyConnection, user_id: &UserId) -> Option<LauncherAccountRecord>;
        /// The account's launcher row.
        fn find_by_account(&self, conn: &mut AnyConnection, account: &AccountId) -> Option<LauncherAccountRecord>;
        /// The account with that normalized address.
        fn find_by_email(&self, conn: &mut AnyConnection, normalized: &str) -> Option<LauncherAccountRecord>;
        /// The account with that normalized username.
        fn find_by_display_name_normalized(
            &self,
            conn: &mut AnyConnection,
            normalized: &str,
        ) -> Option<LauncherAccountRecord>;
        /// Writes the account, and its game account unless one exists; refuses a taken address, then a taken username.
        fn create(&self, tx: &mut Tx<'_>, record: &LauncherAccountRecord) -> CreateOutcome;
        /// Records a launcher sign-in at `when`.
        fn update_last_login(&self, tx: &mut Tx<'_>, account: &AccountId, when: &str) -> ();
        /// Sets the username and the game account's name, and marks it chosen; refuses a name another live account holds.
        fn set_username(
            &self,
            tx: &mut Tx<'_>,
            account: &AccountId,
            display_name: &str,
            normalized: &str,
        ) -> CreateOutcome;
        /// The admin bootstrap's rename: refuses an address another live account holds
        fn set_email(&self, tx: &mut Tx<'_>, account: &AccountId, email: &str) -> CreateOutcome;
        /// Records the accepted consent.
        fn set_consent(&self, tx: &mut Tx<'_>, account: &AccountId, consent: &LauncherConsent) -> ();
        /// Sets or (with None) clears the scheduled deletion.
        fn set_deletion(&self, tx: &mut Tx<'_>, account: &AccountId, deletion: Option<&LauncherDeletion>) -> ();
        /// Accounts whose deletion is due at `now`, oldest scheduled first; `limit` 0 is no limit.
        fn list_deletions_due(&self, conn: &mut AnyConnection, now: &str, limit: u32) -> Vec<LauncherAccountRecord>;
        /// Marks the address verified and clears the pending mark.
        fn set_email_verified(&self, tx: &mut Tx<'_>, account: &AccountId, verified_at: &str) -> ();
        /// Replaces the password hash.
        fn set_password_hash(&self, tx: &mut Tx<'_>, account: &AccountId, hash: &str) -> ();
        /// Unverified accounts pending since before `pending_before`, oldest first; `limit` 0 is no limit.
        fn list_unverified_registrations(
            &self,
            conn: &mut AnyConnection,
            pending_before: &str,
            limit: u32,
        ) -> Vec<LauncherAccountRecord>;
        /// Soft-deletes the account and its game account if still unverified and pending since before `pending_before`; returns whether it did.
        fn delete_unverified_registration(&self, tx: &mut Tx<'_>, account: &AccountId, pending_before: &str) -> bool;
        /// The admin list's page, newest first, and the total that matched.
        fn list_for_admin(
            &self,
            conn: &mut AnyConnection,
            filter: &AdminAccountFilter,
        ) -> (Vec<LauncherAccountRecord>, u64);
        /// Applies a staff access change; returns the account after it, or None when there is no live account.
        fn set_access_state(
            &self,
            tx: &mut Tx<'_>,
            account: &AccountId,
            change: &AccessChange,
        ) -> Option<LauncherAccountRecord>;
        /// Sets or (with None) clears the timeout; returns the account after it, or None when there is no live account.
        fn set_timeout(
            &self,
            tx: &mut Tx<'_>,
            account: &AccountId,
            timeout: Option<&AccountTimeout>,
        ) -> Option<LauncherAccountRecord>;
        /// Replaces the roles; returns the account after it, or None when there is no live account.
        fn update_roles(
            &self,
            tx: &mut Tx<'_>,
            account: &AccountId,
            roles: &[String],
            at: &str,
            by: &str,
        ) -> Option<LauncherAccountRecord>;
        /// Soft-deletes the launcher and game accounts; returns whether either was live.
        fn erase(&self, tx: &mut Tx<'_>, account: &AccountId) -> bool;
    }
}

store_trait! {
    /// Linked Discord identities (`auth_identities`).
    pub trait AuthIdentityRepository = "authIdentities" {
        /// The identity with that Discord user id.
        fn find_by_provider_subject(&self, conn: &mut AnyConnection, subject: &str) -> Option<AuthIdentityRecord>;
        /// The account's identity.
        fn find_by_account(&self, conn: &mut AnyConnection, account: &AccountId) -> Option<AuthIdentityRecord>;
        /// Links the identity; false when the subject or the account is linked already.
        fn create(&self, tx: &mut Tx<'_>, record: &AuthIdentityRecord) -> bool;
        /// Unlinks the account's identities; returns how many.
        fn delete_for_account(&self, tx: &mut Tx<'_>, account: &AccountId) -> u64;
    }
}

store_trait! {
    /// Launcher refresh sessions (`refresh_sessions`).
    pub trait RefreshSessionRepository = "refreshSessions" {
        /// Writes a new session.
        fn create(&self, tx: &mut Tx<'_>, record: &RefreshSessionRecord) -> ();
        /// The session with that token hash, revoked or not.
        fn find_by_token_hash(&self, conn: &mut AnyConnection, hash: &str) -> Option<RefreshSessionRecord>;
        /// Revokes the session at `now`.
        fn revoke_by_id(&self, tx: &mut Tx<'_>, id: &str, now: &str) -> ();
        /// How many rows it revoked (the count is the adapter's, spec §16 item 16)
        fn revoke_family(&self, tx: &mut Tx<'_>, family_id: &str, now: &str) -> u64;
        /// Revokes the account's unrevoked sessions at `now`; returns how many.
        fn revoke_all_for_account(&self, tx: &mut Tx<'_>, account: &AccountId, now: &str) -> u64;
        /// Deletes every session of the account; returns how many.
        fn delete_all_for_account(&self, tx: &mut Tx<'_>, account: &AccountId) -> u64;
        /// Whether the family has an unrevoked session of the account that lapses after `now`.
        fn is_family_active(&self, conn: &mut AnyConnection, family_id: &str, account: &AccountId, now: &str) -> bool;
        /// Deletes sessions that lapsed before `now`; returns how many.
        fn purge_expired(&self, tx: &mut Tx<'_>, now: &str) -> u64;
    }
}

store_trait! {
    /// One-time codes from the launcher to the game (`game_exchange_codes`).
    pub trait GameExchangeCodeRepository = "gameExchangeCodes" {
        /// Writes a new code.
        fn create(&self, tx: &mut Tx<'_>, record: &GameExchangeCodeRecord) -> ();
        /// Marks the code used if unused and unexpired; returns the code, or None when it was not.
        fn consume_by_code_hash(&self, tx: &mut Tx<'_>, hash: &str, now: &str) -> Option<GameExchangeCodeRecord>;
        /// Marks every unused code of the account used at `now`.
        fn revoke_unused_for_account(&self, tx: &mut Tx<'_>, account: &AccountId, now: &str) -> ();
        /// Deletes every code of the account; returns how many.
        fn delete_for_account(&self, tx: &mut Tx<'_>, account: &AccountId) -> u64;
        /// Deletes codes that lapsed before `now`; returns how many.
        fn purge_expired(&self, tx: &mut Tx<'_>, now: &str) -> u64;
    }
}

store_trait! {
    /// Discord sign-in transactions (`discord_oauth_transactions`).
    pub trait DiscordOAuthTransactionRepository = "discordOAuthTransactions" {
        /// Writes a new transaction.
        fn create(&self, tx: &mut Tx<'_>, record: &DiscordOAuthTransactionRecord) -> ();
        /// Marks the `state` used if unused and unexpired; returns the transaction, or None when it was not.
        fn consume_by_state(&self, tx: &mut Tx<'_>, state: &str, now: &str) -> Option<DiscordOAuthTransactionRecord>;
        /// Attaches the completion code and the signed-in account to the `state`'s transaction.
        fn attach_completion_code(
            &self,
            tx: &mut Tx<'_>,
            state: &str,
            code_hash: &str,
            expires_at: &str,
            account: &AccountId,
        ) -> ();
        /// Marks the completion code used if unused and unexpired; returns the transaction, or None when it was not.
        fn consume_by_completion_code_hash(
            &self,
            tx: &mut Tx<'_>,
            code_hash: &str,
            now: &str,
        ) -> Option<DiscordOAuthTransactionRecord>;
        /// Deletes transactions whose `state` and completion code both lapsed before `before`; returns how many.
        fn purge_expired(&self, tx: &mut Tx<'_>, before: &str) -> u64;
        /// Deletes every transaction of the account, signed in or linked; returns how many.
        fn delete_for_account(&self, tx: &mut Tx<'_>, account: &AccountId) -> u64;
    }
}

store_trait! {
    /// Emailed one-time codes (`email_codes`), one per account and purpose.
    pub trait EmailCodeRepository = "emailCodes" {
        /// Writes the code, replacing the account's code for that purpose.
        fn replace(&self, tx: &mut Tx<'_>, record: &EmailCodeRecord) -> ();
        /// The account's code for the purpose while unexpired and with tries left.
        fn peek_live(
            &self,
            conn: &mut AnyConnection,
            account: &AccountId,
            purpose: EmailCodePurpose,
            now: &str,
            max_tries: u32,
        ) -> Option<EmailCodeRecord>;
        /// Deletes the code if it has this hash, is unexpired and has tries left; returns whether it did.
        fn consume(
            &self,
            tx: &mut Tx<'_>,
            account: &AccountId,
            purpose: EmailCodePurpose,
            code_hash: &str,
            now: &str,
            max_tries: u32,
        ) -> bool;
        /// Counts a try on a live code and returns the tries so far; a dead code is deleted and None returned.
        fn reserve_try(
            &self,
            tx: &mut Tx<'_>,
            account: &AccountId,
            purpose: EmailCodePurpose,
            now: &str,
            max_tries: u32,
        ) -> Option<u32>;
        /// Deletes the code if it has this hash; returns whether it did.
        fn remove(&self, tx: &mut Tx<'_>, account: &AccountId, purpose: EmailCodePurpose, code_hash: &str) -> bool;
        /// Deletes every code of the account; returns how many.
        fn delete_for_account(&self, tx: &mut Tx<'_>, account: &AccountId) -> u64;
        /// Deletes codes that lapsed before `now`; returns how many.
        fn purge_expired(&self, tx: &mut Tx<'_>, now: &str) -> u64;
    }
}

store_trait! {
    /// Game server and user API key hashes.
    pub trait ApiKeyRepository = "apiKeys" {
        /// Every game server key hash.
        fn find_all_game_server_key_hashes(&self, conn: &mut AnyConnection) -> Vec<GameServerKeyHash>;
        /// Adds a game server key hash, if new.
        fn insert_game_server_key_hash(&self, tx: &mut Tx<'_>, hash: &str) -> ();
        /// Makes the stored game server key hashes exactly `hashes`.
        fn replace_game_server_key_hashes(&self, tx: &mut Tx<'_>, hashes: &[String]) -> ();
        /// Every user key hash.
        fn find_all_user_key_hashes(&self, conn: &mut AnyConnection) -> Vec<UserKeyHash>;
        /// Sets the account's one user key hash, replacing any.
        fn insert_user_key_hash(&self, tx: &mut Tx<'_>, account: &AccountId, hash: &str) -> ();
        /// Deletes the account's user key hash; returns whether there was one.
        fn delete_user_key_hash(&self, tx: &mut Tx<'_>, account: &AccountId) -> bool;
    }
}

// ------------------------------------------------------------------ accounts

/// [`AccountRepository`] in SQL.
pub struct SqlAccounts;

#[derive(QueryableByName)]
struct AccountRow {
    #[diesel(sql_type = Text)]
    id: String,
    #[diesel(sql_type = Text)]
    user_id: String,
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = BigInt)]
    notes: i64,
}

impl AccountRow {
    fn decode(self) -> RepoResult<AccountRecord> {
        Ok(AccountRecord {
            id: account_id(&self.id, "accounts.id")?,
            user_id: UserId::new(self.user_id),
            name: self.name,
            notes: self.notes,
        })
    }
}

const ACCOUNT_COLUMNS: &str = "SELECT id, user_id, name, notes FROM accounts WHERE deleted_at IS NULL";

impl AccountRepository for SqlAccounts {
    fn find_by_user_id(&self, conn: &mut AnyConnection, user_id: &UserId) -> RepoResult<Option<AccountRecord>> {
        sql(format!("{ACCOUNT_COLUMNS} AND user_id = ?"))
            .t(user_id.as_str())
            .first::<AccountRow>(conn)?
            .map(AccountRow::decode)
            .transpose()
    }

    fn find(&self, conn: &mut AnyConnection, id: &AccountId) -> RepoResult<Option<AccountRecord>> {
        sql(format!("{ACCOUNT_COLUMNS} AND id = ?"))
            .t(id.to_string())
            .first::<AccountRow>(conn)?
            .map(AccountRow::decode)
            .transpose()
    }

    fn create(&self, tx: &mut Tx<'_>, record: &AccountRecord) -> RepoResult<()> {
        insert_account(tx.conn(), &record.id, &record.user_id, &record.name, record.notes)
    }
}

pub(crate) fn insert_account(
    conn: &mut AnyConnection,
    id: &AccountId,
    user_id: &UserId,
    name: &str,
    notes: i64,
) -> RepoResult<()> {
    sql("INSERT INTO accounts (id, user_id, name, notes) VALUES (?, ?, ?, ?)")
        .t(id.to_string())
        .t(user_id.as_str())
        .t(name)
        .i(notes)
        .run(conn)?;
    Ok(())
}

/// Whether an `accounts` row with this id exists, soft-deleted or not.
pub(crate) fn account_row_exists(conn: &mut AnyConnection, id: &AccountId) -> RepoResult<bool> {
    sql("SELECT CAST(1 AS BIGINT) AS n FROM accounts WHERE id = ?")
        .t(id.to_string())
        .exists(conn)
}

// ------------------------------------------------------------------ launcher accounts

/// [`LauncherAccountRepository`] in SQL.
pub struct SqlLauncherAccounts;

#[derive(QueryableByName)]
struct LauncherRow {
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = Text)]
    user_id: String,
    #[diesel(sql_type = Nullable<Text>)]
    email: Option<String>,
    #[diesel(sql_type = Text)]
    display_name_normalized: String,
    #[diesel(sql_type = Text)]
    display_name: String,
    #[diesel(sql_type = Nullable<Text>)]
    password_hash: Option<String>,
    #[diesel(sql_type = Text)]
    status: String,
    #[diesel(sql_type = Nullable<Text>)]
    approval_status: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    approval_updated_at: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    approval_updated_by: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    approval_reason: Option<String>,
    #[diesel(sql_type = Text)]
    roles: String,
    #[diesel(sql_type = Nullable<Text>)]
    roles_updated_at: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    roles_updated_by: Option<String>,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Nullable<Text>)]
    last_login_at: Option<String>,
    #[diesel(sql_type = Nullable<BigInt>)]
    username_set: Option<i64>,
    #[diesel(sql_type = Nullable<Text>)]
    consent_terms_version: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    consent_privacy_version: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    consent_accepted_at: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    deletion_requested_at: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    deletion_scheduled_for: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    email_verified_at: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    email_verification_pending_since: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    timeout_until: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    timeout_since: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    timeout_reason: Option<String>,
}

impl LauncherRow {
    fn decode(self) -> RepoResult<LauncherAccountRecord> {
        let roles = decode_roles(&self.roles)?;
        let status = OperationalStatus::parse(&self.status)
            .ok_or_else(|| RepoError::data(format!("launcher_accounts.status \"{}\"", self.status)))?;
        let approval_status = match &self.approval_status {
            None => None,
            Some(text) => Some(
                ApprovalStatus::parse(text)
                    .ok_or_else(|| RepoError::data(format!("launcher_accounts.approval_status \"{text}\"")))?,
            ),
        };
        let consent = match (
            self.consent_terms_version,
            self.consent_privacy_version,
            self.consent_accepted_at,
        ) {
            (Some(terms_version), Some(privacy_version), Some(accepted_at)) => Some(LauncherConsent {
                terms_version,
                privacy_version,
                accepted_at,
            }),
            _ => None,
        };
        let deletion = match (self.deletion_requested_at, self.deletion_scheduled_for) {
            (Some(requested_at), Some(scheduled_for)) => Some(LauncherDeletion {
                requested_at,
                scheduled_for,
            }),
            _ => None,
        };
        let timeout = match (self.timeout_until, self.timeout_since) {
            (Some(until), Some(since)) => Some(AccountTimeout {
                until,
                since,
                reason: self.timeout_reason,
            }),
            _ => None,
        };
        Ok(LauncherAccountRecord {
            account: account_id(&self.account_id, "launcher_accounts.account_id")?,
            user_id: UserId::new(self.user_id),
            email: self.email,
            display_name_normalized: self.display_name_normalized,
            display_name: self.display_name,
            password_hash: self.password_hash,
            status,
            approval_status,
            approval_updated_at: self.approval_updated_at,
            approval_updated_by: self.approval_updated_by,
            approval_reason: self.approval_reason,
            roles,
            roles_updated_at: self.roles_updated_at,
            roles_updated_by: self.roles_updated_by,
            created_at: self.created_at,
            last_login_at: self.last_login_at,
            username_set: self.username_set.map(|v| v != 0),
            consent,
            deletion,
            email_verified_at: self.email_verified_at,
            email_verification_pending_since: self.email_verification_pending_since,
            timeout,
        })
    }
}

/// The stored `roles`: a JSON array of strings.
pub(crate) fn decode_roles(text: &str) -> RepoResult<Vec<String>> {
    serde_json::from_str::<Vec<String>>(text)
        .map_err(|e| RepoError::data(format!("launcher_accounts.roles \"{text}\": {e}")))
}

/// A live launcher account: the launcher row and its game account both live.
const LIVE_LAUNCHER: &str = "SELECT l.*, a.user_id AS user_id FROM launcher_accounts l \
     JOIN accounts a ON a.id = l.account_id WHERE l.deleted_at IS NULL AND a.deleted_at IS NULL";

fn launcher_one(conn: &mut AnyConnection, clause: &str, value: String) -> RepoResult<Option<LauncherAccountRecord>> {
    sql(format!("{LIVE_LAUNCHER} AND {clause}"))
        .t(value)
        .first::<LauncherRow>(conn)?
        .map(LauncherRow::decode)
        .transpose()
}

fn launcher_many(rows: Vec<LauncherRow>) -> RepoResult<Vec<LauncherAccountRecord>> {
    rows.into_iter().map(LauncherRow::decode).collect()
}

fn find_launcher(conn: &mut AnyConnection, account: &AccountId) -> RepoResult<Option<LauncherAccountRecord>> {
    launcher_one(conn, "l.account_id = ?", account.to_string())
}

/// Whether a live launcher row other than `except`'s holds `column = value`.
fn launcher_taken(conn: &mut AnyConnection, column: &str, value: &str, except: Option<&AccountId>) -> RepoResult<bool> {
    let query = sql(format!(
        "SELECT CAST(1 AS BIGINT) AS n FROM launcher_accounts WHERE {column} = ? AND deleted_at IS NULL"
    ))
    .t(value);
    match except {
        None => query.exists(conn),
        Some(account) => query.push(" AND account_id <> ?").t(account.to_string()).exists(conn),
    }
}

fn roles_json(roles: &[String]) -> String {
    serde_json::Value::from(roles.to_vec()).to_string()
}

impl LauncherAccountRepository for SqlLauncherAccounts {
    fn find_by_user_id(&self, conn: &mut AnyConnection, user_id: &UserId) -> RepoResult<Option<LauncherAccountRecord>> {
        launcher_one(conn, "a.user_id = ?", user_id.as_str().to_owned())
    }

    fn find_by_account(
        &self,
        conn: &mut AnyConnection,
        account: &AccountId,
    ) -> RepoResult<Option<LauncherAccountRecord>> {
        find_launcher(conn, account)
    }

    fn find_by_email(&self, conn: &mut AnyConnection, normalized: &str) -> RepoResult<Option<LauncherAccountRecord>> {
        launcher_one(conn, "l.email = ?", normalized.to_owned())
    }

    fn find_by_display_name_normalized(
        &self,
        conn: &mut AnyConnection,
        normalized: &str,
    ) -> RepoResult<Option<LauncherAccountRecord>> {
        launcher_one(conn, "l.display_name_normalized = ?", normalized.to_owned())
    }

    fn create(&self, tx: &mut Tx<'_>, record: &LauncherAccountRecord) -> RepoResult<CreateOutcome> {
        let conn = tx.conn();
        if let Some(email) = &record.email
            && launcher_taken(conn, "email", email, None)?
        {
            return Ok(CreateOutcome::EmailTaken);
        }
        if launcher_taken(conn, "display_name_normalized", &record.display_name_normalized, None)? {
            return Ok(CreateOutcome::DisplayNameTaken);
        }
        if !account_row_exists(conn, &record.account)? {
            insert_account(conn, &record.account, &record.user_id, &record.display_name, 0)?;
        }
        let consent = record.consent.as_ref();
        let deletion = record.deletion.as_ref();
        sql(
            "INSERT INTO launcher_accounts (account_id, email, display_name_normalized, display_name, \
             password_hash, status, approval_status, approval_updated_at, approval_updated_by, approval_reason, \
             roles, roles_updated_at, roles_updated_by, created_at, last_login_at, username_set, \
             consent_terms_version, consent_privacy_version, consent_accepted_at, deletion_requested_at, \
             deletion_scheduled_for, email_verified_at, email_verification_pending_since) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .t(record.account.to_string())
        .ot(record.email.clone())
        .t(&*record.display_name_normalized)
        .t(&*record.display_name)
        .ot(record.password_hash.clone())
        .t(record.status.as_str())
        .ot(record.approval_status.map(ApprovalStatus::as_str))
        .ot(record.approval_updated_at.clone())
        .ot(record.approval_updated_by.clone())
        .ot(record.approval_reason.clone())
        .t(roles_json(&record.roles))
        .ot(record.roles_updated_at.clone())
        .ot(record.roles_updated_by.clone())
        .t(&*record.created_at)
        .ot(record.last_login_at.clone())
        .oi(record.username_set.map(i64::from))
        .ot(consent.map(|c| c.terms_version.clone()))
        .ot(consent.map(|c| c.privacy_version.clone()))
        .ot(consent.map(|c| c.accepted_at.clone()))
        .ot(deletion.map(|d| d.requested_at.clone()))
        .ot(deletion.map(|d| d.scheduled_for.clone()))
        .ot(record.email_verified_at.clone())
        .ot(record.email_verification_pending_since.clone())
        .run(conn)?;
        Ok(CreateOutcome::Created)
    }

    fn update_last_login(&self, tx: &mut Tx<'_>, account: &AccountId, when: &str) -> RepoResult<()> {
        sql("UPDATE launcher_accounts SET last_login_at = ? WHERE account_id = ?")
            .t(when)
            .t(account.to_string())
            .run(tx.conn())?;
        Ok(())
    }

    fn set_username(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        display_name: &str,
        normalized: &str,
    ) -> RepoResult<CreateOutcome> {
        let conn = tx.conn();
        if launcher_taken(conn, "display_name_normalized", normalized, Some(account))? {
            return Ok(CreateOutcome::DisplayNameTaken);
        }
        sql(
            "UPDATE launcher_accounts SET display_name = ?, display_name_normalized = ?, username_set = 1 \
             WHERE account_id = ?",
        )
        .t(display_name)
        .t(normalized)
        .t(account.to_string())
        .run(conn)?;
        sql("UPDATE accounts SET name = ? WHERE id = ?")
            .t(display_name)
            .t(account.to_string())
            .run(conn)?;
        Ok(CreateOutcome::Created)
    }

    fn set_email(&self, tx: &mut Tx<'_>, account: &AccountId, email: &str) -> RepoResult<CreateOutcome> {
        let conn = tx.conn();
        if launcher_taken(conn, "email", email, Some(account))? {
            return Ok(CreateOutcome::EmailTaken);
        }
        sql("UPDATE launcher_accounts SET email = ? WHERE account_id = ?")
            .t(email)
            .t(account.to_string())
            .run(conn)?;
        Ok(CreateOutcome::Created)
    }

    fn set_consent(&self, tx: &mut Tx<'_>, account: &AccountId, consent: &LauncherConsent) -> RepoResult<()> {
        sql(
            "UPDATE launcher_accounts SET consent_terms_version = ?, consent_privacy_version = ?, \
             consent_accepted_at = ? WHERE account_id = ?",
        )
        .t(&*consent.terms_version)
        .t(&*consent.privacy_version)
        .t(&*consent.accepted_at)
        .t(account.to_string())
        .run(tx.conn())?;
        Ok(())
    }

    fn set_deletion(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        deletion: Option<&LauncherDeletion>,
    ) -> RepoResult<()> {
        sql("UPDATE launcher_accounts SET deletion_requested_at = ?, deletion_scheduled_for = ? WHERE account_id = ?")
            .ot(deletion.map(|d| d.requested_at.clone()))
            .ot(deletion.map(|d| d.scheduled_for.clone()))
            .t(account.to_string())
            .run(tx.conn())?;
        Ok(())
    }

    fn list_deletions_due(
        &self,
        conn: &mut AnyConnection,
        now: &str,
        limit: u32,
    ) -> RepoResult<Vec<LauncherAccountRecord>> {
        let rows = sql(format!(
            "{LIVE_LAUNCHER} AND l.deletion_scheduled_for <= ? ORDER BY l.deletion_scheduled_for ASC{}",
            limit_clause(limit.into())
        ))
        .t(now)
        .load::<LauncherRow>(conn)?;
        launcher_many(rows)
    }

    fn set_email_verified(&self, tx: &mut Tx<'_>, account: &AccountId, verified_at: &str) -> RepoResult<()> {
        sql(
            "UPDATE launcher_accounts SET email_verified_at = ?, email_verification_pending_since = NULL \
             WHERE account_id = ?",
        )
        .t(verified_at)
        .t(account.to_string())
        .run(tx.conn())?;
        Ok(())
    }

    fn set_password_hash(&self, tx: &mut Tx<'_>, account: &AccountId, hash: &str) -> RepoResult<()> {
        sql("UPDATE launcher_accounts SET password_hash = ? WHERE account_id = ?")
            .t(hash)
            .t(account.to_string())
            .run(tx.conn())?;
        Ok(())
    }

    fn list_unverified_registrations(
        &self,
        conn: &mut AnyConnection,
        pending_before: &str,
        limit: u32,
    ) -> RepoResult<Vec<LauncherAccountRecord>> {
        let rows = sql(format!(
            "{LIVE_LAUNCHER} AND l.email_verification_pending_since < ? AND l.email_verified_at IS NULL \
             ORDER BY l.email_verification_pending_since ASC{}",
            limit_clause(limit.into())
        ))
        .t(pending_before)
        .load::<LauncherRow>(conn)?;
        launcher_many(rows)
    }

    fn delete_unverified_registration(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        pending_before: &str,
    ) -> RepoResult<bool> {
        let at = tx.at();
        let deleted = sql(
            "UPDATE launcher_accounts SET deleted_at = ? WHERE account_id = ? AND deleted_at IS NULL \
             AND email_verification_pending_since < ? AND email_verified_at IS NULL",
        )
        .i(at)
        .t(account.to_string())
        .t(pending_before)
        .run(tx.conn())?;
        if deleted == 0 {
            return Ok(false);
        }
        soft_delete_game_account(tx, account)?;
        Ok(true)
    }

    fn list_for_admin(
        &self,
        conn: &mut AnyConnection,
        filter: &AdminAccountFilter,
    ) -> RepoResult<(Vec<LauncherAccountRecord>, u64)> {
        let rows =
            sql(format!("{LIVE_LAUNCHER} ORDER BY l.created_at DESC, l.account_id DESC")).load::<LauncherRow>(conn)?;
        let search = filter
            .search
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(str::to_lowercase);
        let mut kept = Vec::new();
        for record in launcher_many(rows)? {
            if let Some(wanted) = filter.approval_status
                && record.approval_status.unwrap_or(ApprovalStatus::Approved) != wanted
            {
                continue;
            }
            if filter.status.is_some_and(|s| s != record.status) {
                continue;
            }
            if let Some(at) = &filter.timed_out_at
                && record.timeout_at(at).is_none()
            {
                continue;
            }
            if let Some(needle) = &search {
                let hit = record.display_name.to_lowercase().contains(needle.as_str())
                    || record
                        .email
                        .as_deref()
                        .is_some_and(|e| e.to_lowercase().contains(needle.as_str()))
                    || record.user_id.as_str().to_lowercase().contains(needle.as_str());
                if !hit {
                    continue;
                }
            }
            kept.push(record);
        }
        let total = count(kept.len());
        let page = kept.into_iter().skip(filter.skip as usize);
        let page: Vec<_> = if filter.limit == 0 {
            page.collect()
        } else {
            page.take(filter.limit as usize).collect()
        };
        Ok((page, total))
    }

    fn set_access_state(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        change: &AccessChange,
    ) -> RepoResult<Option<LauncherAccountRecord>> {
        let conn = tx.conn();
        if find_launcher(conn, account)?.is_none() {
            return Ok(None);
        }
        // Node writes "" for no reason
        sql(
            "UPDATE launcher_accounts SET approval_updated_at = ?, approval_updated_by = ?, approval_reason = ? \
             WHERE account_id = ?",
        )
        .t(&*change.approval_updated_at)
        .t(&*change.approval_updated_by)
        .t(change.approval_reason.clone().unwrap_or_default())
        .t(account.to_string())
        .run(conn)?;
        if let Some(approval) = change.approval_status {
            sql("UPDATE launcher_accounts SET approval_status = ? WHERE account_id = ?")
                .t(approval.as_str())
                .t(account.to_string())
                .run(conn)?;
        }
        if let Some(status) = change.status {
            sql("UPDATE launcher_accounts SET status = ? WHERE account_id = ?")
                .t(status.as_str())
                .t(account.to_string())
                .run(conn)?;
        }
        find_launcher(conn, account)
    }

    fn set_timeout(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        timeout: Option<&AccountTimeout>,
    ) -> RepoResult<Option<LauncherAccountRecord>> {
        let conn = tx.conn();
        if find_launcher(conn, account)?.is_none() {
            return Ok(None);
        }
        sql(
            "UPDATE launcher_accounts SET timeout_until = ?, timeout_since = ?, timeout_reason = ? \
             WHERE account_id = ?",
        )
        .ot(timeout.map(|t| t.until.clone()))
        .ot(timeout.map(|t| t.since.clone()))
        .ot(timeout.and_then(|t| t.reason.clone()))
        .t(account.to_string())
        .run(conn)?;
        find_launcher(conn, account)
    }

    fn update_roles(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        roles: &[String],
        at: &str,
        by: &str,
    ) -> RepoResult<Option<LauncherAccountRecord>> {
        let conn = tx.conn();
        if find_launcher(conn, account)?.is_none() {
            return Ok(None);
        }
        sql("UPDATE launcher_accounts SET roles = ?, roles_updated_at = ?, roles_updated_by = ? WHERE account_id = ?")
            .t(roles_json(roles))
            .t(at)
            .t(by)
            .t(account.to_string())
            .run(conn)?;
        find_launcher(conn, account)
    }

    fn erase(&self, tx: &mut Tx<'_>, account: &AccountId) -> RepoResult<bool> {
        let at = tx.at();
        let launcher = sql("UPDATE launcher_accounts SET deleted_at = ? WHERE account_id = ? AND deleted_at IS NULL")
            .i(at)
            .t(account.to_string())
            .run(tx.conn())?;
        let game = soft_delete_game_account(tx, account)?;
        Ok(launcher > 0 || game)
    }
}

/// Soft-deletes the live `accounts` row; whether there was one.
pub(crate) fn soft_delete_game_account(tx: &mut Tx<'_>, account: &AccountId) -> RepoResult<bool> {
    let at = tx.at();
    Ok(
        sql("UPDATE accounts SET deleted_at = ? WHERE id = ? AND deleted_at IS NULL")
            .i(at)
            .t(account.to_string())
            .run(tx.conn())?
            > 0,
    )
}

/// Soft-deletes the live launcher row; whether there was one.
pub(crate) fn soft_delete_launcher_account(tx: &mut Tx<'_>, account: &AccountId) -> RepoResult<bool> {
    let at = tx.at();
    Ok(
        sql("UPDATE launcher_accounts SET deleted_at = ? WHERE account_id = ? AND deleted_at IS NULL")
            .i(at)
            .t(account.to_string())
            .run(tx.conn())?
            > 0,
    )
}

// ------------------------------------------------------------------ identities

/// [`AuthIdentityRepository`] in SQL.
pub struct SqlAuthIdentities;

#[derive(QueryableByName)]
struct IdentityRow {
    #[diesel(sql_type = Text)]
    provider_subject: String,
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = Text)]
    provider_username: String,
    #[diesel(sql_type = Nullable<Text>)]
    provider_avatar_url: Option<String>,
    #[diesel(sql_type = Text)]
    linked_at: String,
}

impl IdentityRow {
    fn decode(self) -> RepoResult<AuthIdentityRecord> {
        Ok(AuthIdentityRecord {
            provider_subject: self.provider_subject,
            account: account_id(&self.account_id, "auth_identities.account_id")?,
            provider_username: self.provider_username,
            provider_avatar_url: self.provider_avatar_url,
            linked_at: self.linked_at,
        })
    }
}

const IDENTITIES: &str = "SELECT * FROM auth_identities WHERE provider = 'discord'";

impl AuthIdentityRepository for SqlAuthIdentities {
    fn find_by_provider_subject(
        &self,
        conn: &mut AnyConnection,
        subject: &str,
    ) -> RepoResult<Option<AuthIdentityRecord>> {
        sql(format!("{IDENTITIES} AND provider_subject = ?"))
            .t(subject)
            .first::<IdentityRow>(conn)?
            .map(IdentityRow::decode)
            .transpose()
    }

    fn find_by_account(&self, conn: &mut AnyConnection, account: &AccountId) -> RepoResult<Option<AuthIdentityRecord>> {
        sql(format!("{IDENTITIES} AND account_id = ?"))
            .t(account.to_string())
            .first::<IdentityRow>(conn)?
            .map(IdentityRow::decode)
            .transpose()
    }

    fn create(&self, tx: &mut Tx<'_>, record: &AuthIdentityRecord) -> RepoResult<bool> {
        let conn = tx.conn();
        let linked = sql(
            "SELECT CAST(1 AS BIGINT) AS n FROM auth_identities WHERE provider = 'discord' AND (provider_subject = ? OR account_id = ?)",
        )
        .t(&*record.provider_subject)
        .t(record.account.to_string())
        .exists(conn)?;
        if linked {
            return Ok(false);
        }
        sql(
            "INSERT INTO auth_identities (provider, provider_subject, account_id, provider_username, \
             provider_avatar_url, linked_at) VALUES ('discord', ?, ?, ?, ?, ?)",
        )
        .t(&*record.provider_subject)
        .t(record.account.to_string())
        .t(&*record.provider_username)
        .ot(record.provider_avatar_url.clone())
        .t(&*record.linked_at)
        .run(conn)?;
        Ok(true)
    }

    fn delete_for_account(&self, tx: &mut Tx<'_>, account: &AccountId) -> RepoResult<u64> {
        Ok(count(
            sql("DELETE FROM auth_identities WHERE provider = 'discord' AND account_id = ?")
                .t(account.to_string())
                .run(tx.conn())?,
        ))
    }
}

// ------------------------------------------------------------------ refresh sessions

/// [`RefreshSessionRepository`] in SQL.
pub struct SqlRefreshSessions;

#[derive(QueryableByName)]
struct SessionRow {
    #[diesel(sql_type = Text)]
    id: String,
    #[diesel(sql_type = Text)]
    token_hash: String,
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = Text)]
    family_id: String,
    #[diesel(sql_type = Text)]
    device_id: String,
    #[diesel(sql_type = Text)]
    device_name: String,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    expires_at: String,
    #[diesel(sql_type = Nullable<Text>)]
    revoked_at: Option<String>,
}

impl RefreshSessionRepository for SqlRefreshSessions {
    fn create(&self, tx: &mut Tx<'_>, r: &RefreshSessionRecord) -> RepoResult<()> {
        sql(
            "INSERT INTO refresh_sessions (id, token_hash, account_id, family_id, device_id, device_name, \
             created_at, expires_at, revoked_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .t(&*r.id)
        .t(&*r.token_hash)
        .t(r.account.to_string())
        .t(&*r.family_id)
        .t(&*r.device_id)
        .t(&*r.device_name)
        .t(&*r.created_at)
        .t(&*r.expires_at)
        .ot(r.revoked_at.clone())
        .run(tx.conn())?;
        Ok(())
    }

    fn find_by_token_hash(&self, conn: &mut AnyConnection, hash: &str) -> RepoResult<Option<RefreshSessionRecord>> {
        let Some(row) = sql("SELECT * FROM refresh_sessions WHERE token_hash = ?")
            .t(hash)
            .first::<SessionRow>(conn)?
        else {
            return Ok(None);
        };
        Ok(Some(RefreshSessionRecord {
            account: account_id(&row.account_id, "refresh_sessions.account_id")?,
            id: row.id,
            token_hash: row.token_hash,
            family_id: row.family_id,
            device_id: row.device_id,
            device_name: row.device_name,
            created_at: row.created_at,
            expires_at: row.expires_at,
            revoked_at: row.revoked_at,
        }))
    }

    fn revoke_by_id(&self, tx: &mut Tx<'_>, id: &str, now: &str) -> RepoResult<()> {
        sql("UPDATE refresh_sessions SET revoked_at = ? WHERE id = ?")
            .t(now)
            .t(id)
            .run(tx.conn())?;
        Ok(())
    }

    fn revoke_family(&self, tx: &mut Tx<'_>, family_id: &str, now: &str) -> RepoResult<u64> {
        Ok(count(
            sql("UPDATE refresh_sessions SET revoked_at = ? WHERE family_id = ? AND revoked_at IS NULL")
                .t(now)
                .t(family_id)
                .run(tx.conn())?,
        ))
    }

    fn revoke_all_for_account(&self, tx: &mut Tx<'_>, account: &AccountId, now: &str) -> RepoResult<u64> {
        Ok(count(
            sql("UPDATE refresh_sessions SET revoked_at = ? WHERE account_id = ? AND revoked_at IS NULL")
                .t(now)
                .t(account.to_string())
                .run(tx.conn())?,
        ))
    }

    fn delete_all_for_account(&self, tx: &mut Tx<'_>, account: &AccountId) -> RepoResult<u64> {
        Ok(count(
            sql("DELETE FROM refresh_sessions WHERE account_id = ?")
                .t(account.to_string())
                .run(tx.conn())?,
        ))
    }

    fn is_family_active(
        &self,
        conn: &mut AnyConnection,
        family_id: &str,
        account: &AccountId,
        now: &str,
    ) -> RepoResult<bool> {
        sql("SELECT CAST(1 AS BIGINT) AS n FROM refresh_sessions WHERE family_id = ? AND account_id = ? AND revoked_at IS NULL \
             AND expires_at > ?")
        .t(family_id)
        .t(account.to_string())
        .t(now)
        .exists(conn)
    }

    fn purge_expired(&self, tx: &mut Tx<'_>, now: &str) -> RepoResult<u64> {
        Ok(count(
            sql("DELETE FROM refresh_sessions WHERE expires_at < ?")
                .t(now)
                .run(tx.conn())?,
        ))
    }
}

// ------------------------------------------------------------------ exchange codes

/// [`GameExchangeCodeRepository`] in SQL.
pub struct SqlGameExchangeCodes;

#[derive(QueryableByName)]
struct ExchangeRow {
    #[diesel(sql_type = Text)]
    code_hash: String,
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = Text)]
    launcher_session_id: String,
    #[diesel(sql_type = BigInt)]
    build_changelist: i64,
    #[diesel(sql_type = Text)]
    executable_sha256: String,
    #[diesel(sql_type = Text)]
    runtime_channel: String,
    #[diesel(sql_type = Text)]
    runtime_sha256: String,
    #[diesel(sql_type = Text)]
    runtime_manifest_version: String,
    #[diesel(sql_type = Text)]
    runtime_artifact_set_sha256: String,
    #[diesel(sql_type = Nullable<Text>)]
    realm_id: Option<String>,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    expires_at: String,
    #[diesel(sql_type = Nullable<Text>)]
    consumed_at: Option<String>,
}

impl ExchangeRow {
    fn decode(self) -> RepoResult<GameExchangeCodeRecord> {
        let runtime_channel = RuntimeChannel::parse(&self.runtime_channel).ok_or_else(|| {
            RepoError::data(format!(
                "game_exchange_codes.runtime_channel \"{}\"",
                self.runtime_channel
            ))
        })?;
        let build_changelist = u32::try_from(self.build_changelist)
            .map_err(|_| RepoError::data("game_exchange_codes.build_changelist"))?;
        Ok(GameExchangeCodeRecord {
            account: account_id(&self.account_id, "game_exchange_codes.account_id")?,
            code_hash: self.code_hash,
            launcher_session_id: self.launcher_session_id,
            build_changelist,
            executable_sha256: self.executable_sha256,
            runtime_channel,
            runtime_sha256: self.runtime_sha256,
            runtime_manifest_version: self.runtime_manifest_version,
            runtime_artifact_set_sha256: self.runtime_artifact_set_sha256,
            realm_id: self.realm_id,
            created_at: self.created_at,
            expires_at: self.expires_at,
            consumed_at: self.consumed_at,
        })
    }
}

impl GameExchangeCodeRepository for SqlGameExchangeCodes {
    fn create(&self, tx: &mut Tx<'_>, r: &GameExchangeCodeRecord) -> RepoResult<()> {
        sql(
            "INSERT INTO game_exchange_codes (code_hash, account_id, launcher_session_id, build_changelist, \
             executable_sha256, runtime_channel, runtime_sha256, runtime_manifest_version, \
             runtime_artifact_set_sha256, realm_id, created_at, expires_at, consumed_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .t(&*r.code_hash)
        .t(r.account.to_string())
        .t(&*r.launcher_session_id)
        .i(r.build_changelist.into())
        .t(&*r.executable_sha256)
        .t(r.runtime_channel.as_str())
        .t(&*r.runtime_sha256)
        .t(&*r.runtime_manifest_version)
        .t(&*r.runtime_artifact_set_sha256)
        .ot(r.realm_id.clone())
        .t(&*r.created_at)
        .t(&*r.expires_at)
        .ot(r.consumed_at.clone())
        .run(tx.conn())?;
        Ok(())
    }

    fn consume_by_code_hash(
        &self,
        tx: &mut Tx<'_>,
        hash: &str,
        now: &str,
    ) -> RepoResult<Option<GameExchangeCodeRecord>> {
        let conn = tx.conn();
        let taken = sql(
            "UPDATE game_exchange_codes SET consumed_at = ? WHERE code_hash = ? AND consumed_at IS NULL \
             AND expires_at > ?",
        )
        .t(now)
        .t(hash)
        .t(now)
        .run(conn)?;
        if taken == 0 {
            return Ok(None);
        }
        sql("SELECT * FROM game_exchange_codes WHERE code_hash = ?")
            .t(hash)
            .first::<ExchangeRow>(conn)?
            .map(ExchangeRow::decode)
            .transpose()
    }

    fn revoke_unused_for_account(&self, tx: &mut Tx<'_>, account: &AccountId, now: &str) -> RepoResult<()> {
        sql("UPDATE game_exchange_codes SET consumed_at = ? WHERE account_id = ? AND consumed_at IS NULL")
            .t(now)
            .t(account.to_string())
            .run(tx.conn())?;
        Ok(())
    }

    fn delete_for_account(&self, tx: &mut Tx<'_>, account: &AccountId) -> RepoResult<u64> {
        Ok(count(
            sql("DELETE FROM game_exchange_codes WHERE account_id = ?")
                .t(account.to_string())
                .run(tx.conn())?,
        ))
    }

    fn purge_expired(&self, tx: &mut Tx<'_>, now: &str) -> RepoResult<u64> {
        Ok(count(
            sql("DELETE FROM game_exchange_codes WHERE expires_at < ?")
                .t(now)
                .run(tx.conn())?,
        ))
    }
}

// ------------------------------------------------------------------ Discord transactions

/// [`DiscordOAuthTransactionRepository`] in SQL.
pub struct SqlDiscordOAuthTransactions;

#[derive(QueryableByName)]
struct DiscordRow {
    #[diesel(sql_type = Text)]
    state: String,
    #[diesel(sql_type = Text)]
    code_verifier: String,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    expires_at: String,
    #[diesel(sql_type = Nullable<Text>)]
    consumed_at: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    completion_code_hash: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    completion_expires_at: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    completion_consumed_at: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    account_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    link_account_id: Option<String>,
}

impl DiscordRow {
    fn decode(self) -> RepoResult<DiscordOAuthTransactionRecord> {
        Ok(DiscordOAuthTransactionRecord {
            account: opt_account_id(self.account_id.as_deref(), "discord_oauth_transactions.account_id")?,
            link_account: opt_account_id(
                self.link_account_id.as_deref(),
                "discord_oauth_transactions.link_account_id",
            )?,
            state: self.state,
            code_verifier: self.code_verifier,
            created_at: self.created_at,
            expires_at: self.expires_at,
            consumed_at: self.consumed_at,
            completion_code_hash: self.completion_code_hash,
            completion_expires_at: self.completion_expires_at,
            completion_consumed_at: self.completion_consumed_at,
        })
    }
}

fn discord_by(
    conn: &mut AnyConnection,
    column: &str,
    value: &str,
) -> RepoResult<Option<DiscordOAuthTransactionRecord>> {
    sql(format!("SELECT * FROM discord_oauth_transactions WHERE {column} = ?"))
        .t(value)
        .first::<DiscordRow>(conn)?
        .map(DiscordRow::decode)
        .transpose()
}

impl DiscordOAuthTransactionRepository for SqlDiscordOAuthTransactions {
    fn create(&self, tx: &mut Tx<'_>, r: &DiscordOAuthTransactionRecord) -> RepoResult<()> {
        sql(
            "INSERT INTO discord_oauth_transactions (state, code_verifier, created_at, expires_at, consumed_at, \
             completion_code_hash, completion_expires_at, completion_consumed_at, account_id, link_account_id) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .t(&*r.state)
        .t(&*r.code_verifier)
        .t(&*r.created_at)
        .t(&*r.expires_at)
        .ot(r.consumed_at.clone())
        .ot(r.completion_code_hash.clone())
        .ot(r.completion_expires_at.clone())
        .ot(r.completion_consumed_at.clone())
        .ot(r.account.as_ref().map(ToString::to_string))
        .ot(r.link_account.as_ref().map(ToString::to_string))
        .run(tx.conn())?;
        Ok(())
    }

    fn consume_by_state(
        &self,
        tx: &mut Tx<'_>,
        state: &str,
        now: &str,
    ) -> RepoResult<Option<DiscordOAuthTransactionRecord>> {
        let conn = tx.conn();
        let taken = sql("UPDATE discord_oauth_transactions SET consumed_at = ? WHERE state = ? \
             AND consumed_at IS NULL AND expires_at > ?")
        .t(now)
        .t(state)
        .t(now)
        .run(conn)?;
        if taken == 0 {
            return Ok(None);
        }
        discord_by(conn, "state", state)
    }

    fn attach_completion_code(
        &self,
        tx: &mut Tx<'_>,
        state: &str,
        code_hash: &str,
        expires_at: &str,
        account: &AccountId,
    ) -> RepoResult<()> {
        sql(
            "UPDATE discord_oauth_transactions SET completion_code_hash = ?, completion_expires_at = ?, \
             account_id = ? WHERE state = ?",
        )
        .t(code_hash)
        .t(expires_at)
        .t(account.to_string())
        .t(state)
        .run(tx.conn())?;
        Ok(())
    }

    fn consume_by_completion_code_hash(
        &self,
        tx: &mut Tx<'_>,
        code_hash: &str,
        now: &str,
    ) -> RepoResult<Option<DiscordOAuthTransactionRecord>> {
        let conn = tx.conn();
        let Some(row) = sql(
            "SELECT * FROM discord_oauth_transactions WHERE completion_code_hash = ? \
             AND completion_expires_at > ? AND completion_consumed_at IS NULL",
        )
        .t(code_hash)
        .t(now)
        .first::<DiscordRow>(conn)?
        else {
            return Ok(None);
        };
        sql("UPDATE discord_oauth_transactions SET completion_consumed_at = ? WHERE state = ?")
            .t(now)
            .t(&*row.state)
            .run(conn)?;
        discord_by(conn, "state", &row.state)
    }

    fn purge_expired(&self, tx: &mut Tx<'_>, before: &str) -> RepoResult<u64> {
        Ok(count(
            sql("DELETE FROM discord_oauth_transactions WHERE expires_at < ? \
                 AND (completion_expires_at IS NULL OR completion_expires_at < ?)")
            .t(before)
            .t(before)
            .run(tx.conn())?,
        ))
    }

    fn delete_for_account(&self, tx: &mut Tx<'_>, account: &AccountId) -> RepoResult<u64> {
        let id = account.to_string();
        Ok(count(
            sql("DELETE FROM discord_oauth_transactions WHERE account_id = ? OR link_account_id = ?")
                .t(id.clone())
                .t(id)
                .run(tx.conn())?,
        ))
    }
}

// ------------------------------------------------------------------ email codes

/// [`EmailCodeRepository`] in SQL.
pub struct SqlEmailCodes;

#[derive(QueryableByName)]
struct EmailCodeRow {
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = Text)]
    purpose: String,
    #[diesel(sql_type = Text)]
    code_hash: String,
    #[diesel(sql_type = BigInt)]
    attempts: i64,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    expires_at: String,
}

impl EmailCodeRow {
    fn decode(self) -> RepoResult<EmailCodeRecord> {
        let purpose = EmailCodePurpose::parse(&self.purpose)
            .ok_or_else(|| RepoError::data(format!("email_codes.purpose \"{}\"", self.purpose)))?;
        Ok(EmailCodeRecord {
            account: account_id(&self.account_id, "email_codes.account_id")?,
            purpose,
            code_hash: self.code_hash,
            attempts: attempts(self.attempts),
            created_at: self.created_at,
            expires_at: self.expires_at,
        })
    }

    fn is_live(&self, now: &str, max_tries: u32) -> bool {
        self.expires_at.as_str() > now && attempts(self.attempts) < max_tries
    }
}

/// A stored count beyond `u32` reads as `u32::MAX`.
fn attempts(stored: i64) -> u32 {
    u32::try_from(stored.max(0)).unwrap_or(u32::MAX)
}

fn email_code(
    conn: &mut AnyConnection,
    account: &AccountId,
    purpose: EmailCodePurpose,
) -> RepoResult<Option<EmailCodeRow>> {
    sql("SELECT * FROM email_codes WHERE account_id = ? AND purpose = ?")
        .t(account.to_string())
        .t(purpose.as_str())
        .first::<EmailCodeRow>(conn)
}

impl EmailCodeRepository for SqlEmailCodes {
    fn replace(&self, tx: &mut Tx<'_>, r: &EmailCodeRecord) -> RepoResult<()> {
        let conn = tx.conn();
        sql("DELETE FROM email_codes WHERE account_id = ? AND purpose = ?")
            .t(r.account.to_string())
            .t(r.purpose.as_str())
            .run(conn)?;
        sql(
            "INSERT INTO email_codes (account_id, purpose, code_hash, attempts, created_at, expires_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .t(r.account.to_string())
        .t(r.purpose.as_str())
        .t(&*r.code_hash)
        .i(r.attempts.into())
        .t(&*r.created_at)
        .t(&*r.expires_at)
        .run(conn)?;
        Ok(())
    }

    fn peek_live(
        &self,
        conn: &mut AnyConnection,
        account: &AccountId,
        purpose: EmailCodePurpose,
        now: &str,
        max_tries: u32,
    ) -> RepoResult<Option<EmailCodeRecord>> {
        match email_code(conn, account, purpose)? {
            Some(row) if row.is_live(now, max_tries) => row.decode().map(Some),
            _ => Ok(None),
        }
    }

    fn consume(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        purpose: EmailCodePurpose,
        code_hash: &str,
        now: &str,
        max_tries: u32,
    ) -> RepoResult<bool> {
        let gone = sql(
            "DELETE FROM email_codes WHERE account_id = ? AND purpose = ? AND code_hash = ? \
             AND expires_at > ? AND attempts < ?",
        )
        .t(account.to_string())
        .t(purpose.as_str())
        .t(code_hash)
        .t(now)
        .i(max_tries.into())
        .run(tx.conn())?;
        Ok(gone == 1)
    }

    fn reserve_try(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        purpose: EmailCodePurpose,
        now: &str,
        max_tries: u32,
    ) -> RepoResult<Option<u32>> {
        let conn = tx.conn();
        let Some(row) = email_code(conn, account, purpose)? else {
            return Ok(None);
        };
        if row.is_live(now, max_tries) {
            sql("UPDATE email_codes SET attempts = attempts + 1 WHERE account_id = ? AND purpose = ?")
                .t(account.to_string())
                .t(purpose.as_str())
                .run(conn)?;
            return Ok(Some(attempts(row.attempts).saturating_add(1)));
        }
        sql("DELETE FROM email_codes WHERE account_id = ? AND purpose = ?")
            .t(account.to_string())
            .t(purpose.as_str())
            .run(conn)?;
        Ok(None)
    }

    fn remove(
        &self,
        tx: &mut Tx<'_>,
        account: &AccountId,
        purpose: EmailCodePurpose,
        code_hash: &str,
    ) -> RepoResult<bool> {
        let gone = sql("DELETE FROM email_codes WHERE account_id = ? AND purpose = ? AND code_hash = ?")
            .t(account.to_string())
            .t(purpose.as_str())
            .t(code_hash)
            .run(tx.conn())?;
        Ok(gone == 1)
    }

    fn delete_for_account(&self, tx: &mut Tx<'_>, account: &AccountId) -> RepoResult<u64> {
        Ok(count(
            sql("DELETE FROM email_codes WHERE account_id = ?")
                .t(account.to_string())
                .run(tx.conn())?,
        ))
    }

    fn purge_expired(&self, tx: &mut Tx<'_>, now: &str) -> RepoResult<u64> {
        Ok(count(
            sql("DELETE FROM email_codes WHERE expires_at < ?")
                .t(now)
                .run(tx.conn())?,
        ))
    }
}

// ------------------------------------------------------------------ keys

/// [`ApiKeyRepository`] in SQL.
pub struct SqlApiKeys;

#[derive(QueryableByName)]
struct UserKeyRow {
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = Text)]
    key_hash: String,
}

fn game_server_hashes(conn: &mut AnyConnection) -> RepoResult<Vec<String>> {
    Ok(sql("SELECT key_hash AS t FROM game_server_api_keys")
        .load::<crate::sql::TextRow>(conn)?
        .into_iter()
        .map(|r| r.t)
        .collect())
}

fn insert_game_server_hash(conn: &mut AnyConnection, hash: &str) -> RepoResult<()> {
    sql("INSERT INTO game_server_api_keys (key_hash) VALUES (?) ON CONFLICT DO NOTHING")
        .t(hash)
        .run(conn)?;
    Ok(())
}

impl ApiKeyRepository for SqlApiKeys {
    fn find_all_game_server_key_hashes(&self, conn: &mut AnyConnection) -> RepoResult<Vec<GameServerKeyHash>> {
        Ok(game_server_hashes(conn)?
            .into_iter()
            .enumerate()
            .map(|(i, key_hash)| GameServerKeyHash {
                id: i64::try_from(i).unwrap_or(i64::MAX),
                key_hash: Some(key_hash),
            })
            .collect())
    }

    fn insert_game_server_key_hash(&self, tx: &mut Tx<'_>, hash: &str) -> RepoResult<()> {
        insert_game_server_hash(tx.conn(), hash)
    }

    fn replace_game_server_key_hashes(&self, tx: &mut Tx<'_>, hashes: &[String]) -> RepoResult<()> {
        let conn = tx.conn();
        if hashes.is_empty() {
            sql("DELETE FROM game_server_api_keys").run(conn)?;
            return Ok(());
        }
        let stored = game_server_hashes(conn)?;
        for hash in hashes {
            if !stored.contains(hash) {
                insert_game_server_hash(conn, hash)?;
            }
        }
        for hash in stored.iter().filter(|h| !hashes.contains(h)) {
            sql("DELETE FROM game_server_api_keys WHERE key_hash = ?")
                .t(&**hash)
                .run(conn)?;
        }
        Ok(())
    }

    fn find_all_user_key_hashes(&self, conn: &mut AnyConnection) -> RepoResult<Vec<UserKeyHash>> {
        sql("SELECT account_id, key_hash FROM user_api_keys")
            .load::<UserKeyRow>(conn)?
            .into_iter()
            .map(|r| {
                Ok(UserKeyHash {
                    account: account_id(&r.account_id, "user_api_keys.account_id")?,
                    key_hash: r.key_hash,
                })
            })
            .collect()
    }

    fn insert_user_key_hash(&self, tx: &mut Tx<'_>, account: &AccountId, hash: &str) -> RepoResult<()> {
        let conn = tx.conn();
        sql("DELETE FROM user_api_keys WHERE account_id = ?")
            .t(account.to_string())
            .run(conn)?;
        sql("INSERT INTO user_api_keys (account_id, key_hash) VALUES (?, ?)")
            .t(account.to_string())
            .t(hash)
            .run(conn)?;
        Ok(())
    }

    fn delete_user_key_hash(&self, tx: &mut Tx<'_>, account: &AccountId) -> RepoResult<bool> {
        Ok(sql("DELETE FROM user_api_keys WHERE account_id = ?")
            .t(account.to_string())
            .run(tx.conn())?
            > 0)
    }
}

/// Sets the game account's name (the adapter's registration with its own game name).
pub(crate) fn set_account_name(conn: &mut AnyConnection, account: &AccountId, name: &str) -> RepoResult<()> {
    sql("UPDATE accounts SET name = ? WHERE id = ?")
        .t(name)
        .t(account.to_string())
        .run(conn)?;
    Ok(())
}

/// Soft-deletes the launcher row while it is live, pending and unverified, then its game account; whether it did.
/// The pending mark is written once at registration and only ever cleared, so no cut-off is needed (spec §16 item 2).
pub(crate) fn delete_pending_registration(tx: &mut Tx<'_>, account: &AccountId) -> RepoResult<bool> {
    let at = tx.at();
    let deleted = sql(
        "UPDATE launcher_accounts SET deleted_at = ? WHERE account_id = ? AND deleted_at IS NULL \
         AND email_verification_pending_since IS NOT NULL AND email_verified_at IS NULL",
    )
    .i(at)
    .t(account.to_string())
    .run(tx.conn())?;
    if deleted == 0 {
        return Ok(false);
    }
    soft_delete_game_account(tx, account)?;
    Ok(true)
}
