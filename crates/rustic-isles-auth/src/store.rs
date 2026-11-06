// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The rows the area reads and writes (spec §9) and the store trait isles implements over its tables.
//! Times are ISO text (§2.1); every read sees live rows only.

use module_api::{ActorContext, BoxFuture};
use serde::Serialize;

/// A store failure: logged, and answered as `INTERNAL` (or a 500 page on a game route).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct StoreError {
    /// What failed, for the log; never shown to a player.
    pub message: String,
}

impl StoreError {
    /// A failure described by `message`.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// An auth store operation's result.
pub type StoreResult<T> = Result<T, StoreError>;

/// An `accounts` row: the game-facing account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameAccount {
    /// The network-wide id, `account:<ULID>@<node>`
    pub id: String,
    /// The userId the game and its tokens carry.
    pub user_id: String,
    /// The display name the game shows.
    pub name: String,
    /// The `accounts.notes` counter; 0 for a new account.
    pub notes: i64,
}

/// Whether an account may sign in at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// In good standing.
    Active,
    /// Banned by staff.
    Banned,
    /// Disabled by staff.
    Disabled,
}

impl Status {
    /// The stored text.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Banned => "banned",
            Self::Disabled => "disabled",
        }
    }
}

/// A closed-test access request's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approval {
    /// Waiting for staff.
    Pending,
    /// Let in.
    Approved,
    /// Turned down.
    Rejected,
}

impl Approval {
    /// The stored text.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
        }
    }
}

/// The Terms and Privacy versions an account accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Consent {
    /// The Terms of Use version accepted.
    pub terms_version: String,
    /// The Privacy Policy version accepted.
    pub privacy_version: String,
    /// When, as ISO text.
    pub accepted_at: String,
}

/// A scheduled account deletion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deletion {
    /// When it was asked for, as ISO text.
    pub requested_at: String,
    /// When the erasure is due, as ISO text.
    pub scheduled_for: String,
}

/// A staff timeout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timeout {
    /// When it ends, as ISO text; in force while this sorts after now.
    pub until: String,
    /// When it began, as ISO text.
    pub since: String,
    /// The reason shown to the player, if staff gave one.
    pub reason: Option<String>,
}

/// A `launcher_accounts` row joined to its `accounts` row.
#[derive(Clone, PartialEq, Eq)]
pub struct LauncherAccount {
    /// The network-wide account id, `account:<ULID>@<node>`.
    pub account_id: String,
    /// The userId the game and its tokens carry.
    pub user_id: String,
    /// The address, normalized; None for a Discord-only account.
    pub email: Option<String>,
    /// The username as chosen.
    pub display_name: String,
    /// The username normalized for uniqueness.
    pub display_name_normalized: String,
    /// The Argon2id hash; None for a Discord-only account. Never logged.
    pub password_hash: Option<String>,
    /// Whether the account may sign in.
    pub status: Status,
    /// `None` is a legacy row, read as approved
    pub approval: Option<Approval>,
    /// Staff roles, such as `admin`.
    pub roles: Vec<String>,
    /// When the roles last changed, as ISO text.
    pub roles_updated_at: Option<String>,
    /// When the account was registered, as ISO text.
    pub created_at: String,
    /// The last launcher sign-in, as ISO text.
    pub last_login_at: Option<String>,
    /// `None` is a legacy row, read as set
    pub username_set: Option<bool>,
    /// The accepted consent, if any.
    pub consent: Option<Consent>,
    /// The scheduled deletion, if any.
    pub deletion: Option<Deletion>,
    /// When the address was verified, as ISO text.
    pub email_verified_at: Option<String>,
    /// When a required verification began; the unverified registration is removed once it is stale.
    pub email_verification_pending_since: Option<String>,
    /// The staff timeout, if any.
    pub timeout: Option<Timeout>,
}

// The password hash stays out of logs (§13)
impl std::fmt::Debug for LauncherAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LauncherAccount")
            .field("account_id", &self.account_id)
            .field("user_id", &self.user_id)
            .field("display_name", &self.display_name)
            .field("status", &self.status)
            .field("approval", &self.approval)
            .field("roles", &self.roles)
            .field("has_password", &self.password_hash.is_some())
            .finish_non_exhaustive()
    }
}

impl LauncherAccount {
    /// The approval; a legacy row without one reads as approved.
    pub fn effective_approval(&self) -> Approval {
        self.approval.unwrap_or(Approval::Approved)
    }
}

/// An `auth_identities` row (provider `discord`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// The Discord user id.
    pub provider_subject: String,
    /// The linked account.
    pub account_id: String,
    /// The Discord username when linked.
    pub provider_username: String,
    /// The avatar URL when linked.
    pub provider_avatar_url: Option<String>,
    /// When it was linked, as ISO text.
    pub linked_at: String,
}

/// A `refresh_sessions` row: one launcher refresh token; Debug leaves out the hash.
#[derive(Clone, PartialEq, Eq)]
pub struct RefreshSession {
    /// The session's UUID.
    pub id: String,
    /// SHA-256 of the refresh token; the token itself is never stored.
    pub token_hash: String,
    /// The account it signs in.
    pub account_id: String,
    /// The rotation family; signing out revokes the family.
    pub family_id: String,
    /// The device id the launcher sent, or a fresh one.
    pub device_id: String,
    /// The device name the launcher sent.
    pub device_name: String,
    /// When it was issued, as ISO text.
    pub created_at: String,
    /// When it lapses, as ISO text.
    pub expires_at: String,
    /// When it was revoked, as ISO text.
    pub revoked_at: Option<String>,
}

impl std::fmt::Debug for RefreshSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RefreshSession")
            .field("id", &self.id)
            .field("account_id", &self.account_id)
            .field("family_id", &self.family_id)
            .field("expires_at", &self.expires_at)
            .field("revoked_at", &self.revoked_at)
            .finish_non_exhaustive()
    }
}

/// A runtime release channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// The released runtime.
    Stable,
    /// The beta runtime.
    Beta,
    /// The development runtime.
    Dev,
}

impl Channel {
    /// The channel a text names; None for any other text.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "stable" => Some(Self::Stable),
            "beta" => Some(Self::Beta),
            "dev" => Some(Self::Dev),
            _ => None,
        }
    }

    /// The stored text.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Beta => "beta",
            Self::Dev => "dev",
        }
    }
}

/// A `game_exchange_codes` row: the one-time code the launcher hands the game; Debug leaves out the hash.
#[derive(Clone, PartialEq, Eq)]
pub struct ExchangeCode {
    /// SHA-256 of the code.
    pub code_hash: String,
    /// The account it signs in.
    pub account_id: String,
    /// The launcher refresh family that asked for it.
    pub launcher_session_id: String,
    /// The game build's changelist.
    pub build_changelist: i64,
    /// The game executable's SHA-256.
    pub executable_sha256: String,
    /// The runtime's channel.
    pub runtime_channel: Channel,
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

impl std::fmt::Debug for ExchangeCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExchangeCode")
            .field("account_id", &self.account_id)
            .field("build_changelist", &self.build_changelist)
            .field("runtime_channel", &self.runtime_channel)
            .field("realm_id", &self.realm_id)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// A `discord_oauth_transactions` row: one Discord sign-in or link; Debug leaves out the secrets.
#[derive(Clone, PartialEq, Eq)]
pub struct DiscordTransaction {
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
    /// The account a sign-in completed for
    pub account_id: Option<String>,
    /// The signed-in account a link transaction links
    pub link_account_id: Option<String>,
}

impl std::fmt::Debug for DiscordTransaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscordTransaction")
            .field("expires_at", &self.expires_at)
            .field("account_id", &self.account_id)
            .field("link_account_id", &self.link_account_id)
            .finish_non_exhaustive()
    }
}

/// What an emailed code is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Purpose {
    /// Verifying the address.
    Verify,
    /// Resetting the password.
    Reset,
    /// Unlinking Discord.
    Unlink,
}

impl Purpose {
    /// The stored text, also bound into the code's hash.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Verify => "verify",
            Self::Reset => "reset",
            Self::Unlink => "unlink",
        }
    }
}

/// An `email_codes` row; Debug leaves out the hash.
#[derive(Clone, PartialEq, Eq)]
pub struct EmailCode {
    /// The account it was sent for.
    pub account_id: String,
    /// What it is for.
    pub purpose: Purpose,
    /// The code's keyed hash.
    pub code_hash: String,
    /// Wrong entries so far.
    pub attempts: u32,
    /// When it was sent, as ISO text.
    pub created_at: String,
    /// When it lapses, as ISO text.
    pub expires_at: String,
}

impl std::fmt::Debug for EmailCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmailCode")
            .field("account_id", &self.account_id)
            .field("purpose", &self.purpose)
            .field("attempts", &self.attempts)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// A new launcher account (registration).
#[derive(Clone)]
pub struct NewLauncherAccount {
    /// The launcher account to write.
    pub account: LauncherAccount,
    /// The game account's name (the display name)
    pub game_name: String,
}

/// What creating a launcher account did.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
    clippy::large_enum_variant,
    reason = "made once per account creation; the size is no concern"
)]
pub enum CreateOutcome {
    /// Created; the account as written.
    Created(LauncherAccount),
    /// Another live account has the address.
    EmailTaken,
    /// Another live account has the username.
    NameTaken,
}

/// What an erasure removed (§8.2), in the answer's order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErasureCounts {
    /// Emailed codes.
    pub email_codes: u64,
    /// Game exchange codes.
    pub game_exchange_codes: u64,
    /// Discord sign-in transactions.
    #[serde(rename = "discordOAuthTransactions")]
    pub discord_oauth_transactions: u64,
    /// Linked identities.
    pub auth_identities: u64,
    /// User account keys.
    pub user_api_keys: u64,
    /// Friendships.
    pub friendships: u64,
    /// Rows in other areas' tables.
    pub other_areas: u64,
    /// The launcher and game accounts.
    pub accounts: u64,
}

/// §9.3's operations. Each write takes its actor (DD23); the atomic ones say so.
pub trait AuthStore: Send + Sync + 'static {
    // Game accounts
    /// The game account with that userId.
    fn game_account_by_user_id<'a>(&'a self, user_id: &'a str) -> BoxFuture<'a, StoreResult<Option<GameAccount>>>;
    /// The game account with that id.
    fn game_account_by_id<'a>(&'a self, account_id: &'a str) -> BoxFuture<'a, StoreResult<Option<GameAccount>>>;
    /// Creates the game account unless one exists for its userId; whether it did
    fn ensure_game_account<'a>(
        &'a self,
        actor: &'a ActorContext,
        account: &'a GameAccount,
    ) -> BoxFuture<'a, StoreResult<bool>>;

    // Launcher accounts
    /// The live launcher account with that userId.
    fn launcher_account_by_user_id<'a>(
        &'a self,
        user_id: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<LauncherAccount>>>;
    /// The live launcher account with that id.
    fn launcher_account_by_id<'a>(&'a self, account_id: &'a str)
    -> BoxFuture<'a, StoreResult<Option<LauncherAccount>>>;
    /// The live launcher account with that normalized address.
    fn launcher_account_by_email<'a>(
        &'a self,
        normalized_email: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<LauncherAccount>>>;
    /// The live launcher account with that normalized username.
    fn launcher_account_by_name<'a>(
        &'a self,
        normalized_name: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<LauncherAccount>>>;
    /// Refuses a taken email, then a taken name; writes the game account unless it exists
    fn create_launcher_account<'a>(
        &'a self,
        actor: &'a ActorContext,
        account: &'a NewLauncherAccount,
    ) -> BoxFuture<'a, StoreResult<CreateOutcome>>;
    /// Records a launcher sign-in at `at`.
    fn set_last_login<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
        at: &'a str,
    ) -> BoxFuture<'a, StoreResult<()>>;
    /// Also sets `accounts.name` and `username_set`; false when another live account holds the name
    fn set_username<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
        display_name: &'a str,
        normalized: &'a str,
    ) -> BoxFuture<'a, StoreResult<bool>>;
    /// Records the account's accepted consent.
    fn set_consent<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
        consent: &'a Consent,
    ) -> BoxFuture<'a, StoreResult<()>>;
    /// Sets or (with None) clears the scheduled deletion.
    fn set_deletion<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
        deletion: Option<&'a Deletion>,
    ) -> BoxFuture<'a, StoreResult<()>>;
    /// Also clears the pending mark
    fn set_email_verified<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
        at: &'a str,
    ) -> BoxFuture<'a, StoreResult<()>>;
    /// One write: the new hash, `email_verified_at`, every session revoked
    fn reset_password<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
        password_hash: &'a str,
        at: &'a str,
    ) -> BoxFuture<'a, StoreResult<()>>;
    /// Live accounts whose deletion is due, oldest scheduled first; `limit` 0 is none
    fn due_deletions<'a>(&'a self, now: &'a str, limit: usize) -> BoxFuture<'a, StoreResult<Vec<LauncherAccount>>>;
    /// Unverified accounts pending since before `before`, oldest pending first
    fn stale_unverified<'a>(
        &'a self,
        before: &'a str,
        limit: usize,
    ) -> BoxFuture<'a, StoreResult<Vec<LauncherAccount>>>;
    /// One write: soft-deletes the launcher and game account if still pending and unverified, with its codes and
    /// sessions; whether it did
    fn delete_unverified_registration<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
    ) -> BoxFuture<'a, StoreResult<bool>>;
    /// One write, the other areas' rows included (§8.2)
    fn erase_account<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
    ) -> BoxFuture<'a, StoreResult<ErasureCounts>>;

    // Identities
    /// The identity linked to that Discord user id.
    fn identity_by_subject<'a>(&'a self, subject: &'a str) -> BoxFuture<'a, StoreResult<Option<Identity>>>;
    /// The identity linked to that account.
    fn identity_by_account<'a>(&'a self, account_id: &'a str) -> BoxFuture<'a, StoreResult<Option<Identity>>>;
    /// False when the subject or the account is linked already
    fn create_identity<'a>(
        &'a self,
        actor: &'a ActorContext,
        identity: &'a Identity,
    ) -> BoxFuture<'a, StoreResult<bool>>;
    /// Unlinks every identity of the account; returns how many.
    fn delete_identities_for_account<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
    ) -> BoxFuture<'a, StoreResult<u64>>;

    // Refresh sessions
    /// Writes a new refresh session.
    fn create_session<'a>(
        &'a self,
        actor: &'a ActorContext,
        session: &'a RefreshSession,
    ) -> BoxFuture<'a, StoreResult<()>>;
    /// Revoked sessions included
    fn session_by_token_hash<'a>(&'a self, token_hash: &'a str) -> BoxFuture<'a, StoreResult<Option<RefreshSession>>>;
    /// One write: revokes `revoke_id` at `at` and creates `next`
    fn rotate_session<'a>(
        &'a self,
        actor: &'a ActorContext,
        revoke_id: &'a str,
        at: &'a str,
        next: &'a RefreshSession,
    ) -> BoxFuture<'a, StoreResult<()>>;
    /// Unrevoked sessions only
    fn revoke_family<'a>(
        &'a self,
        actor: &'a ActorContext,
        family_id: &'a str,
        at: &'a str,
    ) -> BoxFuture<'a, StoreResult<u64>>;
    /// Revokes every unrevoked session of the account at `at`; returns how many.
    fn revoke_all_sessions<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
        at: &'a str,
    ) -> BoxFuture<'a, StoreResult<u64>>;
    /// An unrevoked session of the family and account whose `expires_at` is after `now`
    fn is_family_active<'a>(
        &'a self,
        family_id: &'a str,
        account_id: &'a str,
        now: &'a str,
    ) -> BoxFuture<'a, StoreResult<bool>>;

    // Exchange codes
    /// Writes a new exchange code.
    fn create_exchange_code<'a>(
        &'a self,
        actor: &'a ActorContext,
        code: &'a ExchangeCode,
    ) -> BoxFuture<'a, StoreResult<()>>;
    /// Atomic: unconsumed and `expires_at` after `now`; sets `consumed_at`
    fn consume_exchange_code<'a>(
        &'a self,
        actor: &'a ActorContext,
        code_hash: &'a str,
        now: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<ExchangeCode>>>;

    // Discord transactions
    /// Writes a new Discord transaction.
    fn create_discord_transaction<'a>(
        &'a self,
        actor: &'a ActorContext,
        transaction: &'a DiscordTransaction,
    ) -> BoxFuture<'a, StoreResult<()>>;
    /// Atomic: unconsumed and unexpired
    fn consume_discord_state<'a>(
        &'a self,
        actor: &'a ActorContext,
        state: &'a str,
        now: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<DiscordTransaction>>>;
    /// Attaches the completion code and the signed-in account to the `state`'s transaction.
    fn attach_discord_completion<'a>(
        &'a self,
        actor: &'a ActorContext,
        state: &'a str,
        completion_hash: &'a str,
        expires_at: &'a str,
        account_id: &'a str,
    ) -> BoxFuture<'a, StoreResult<()>>;
    /// Atomic: completion unconsumed and unexpired
    fn consume_discord_completion<'a>(
        &'a self,
        actor: &'a ActorContext,
        completion_hash: &'a str,
        now: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<DiscordTransaction>>>;

    // Email codes
    /// One live code per account and purpose: a new one replaces it
    fn replace_email_code<'a>(&'a self, actor: &'a ActorContext, code: &'a EmailCode)
    -> BoxFuture<'a, StoreResult<()>>;
    /// Unexpired, tries left
    fn live_email_code<'a>(
        &'a self,
        account_id: &'a str,
        purpose: Purpose,
        now: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<EmailCode>>>;
    /// Deletes it if still this hash, unexpired and with tries left; whether this call did
    fn consume_email_code<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
        purpose: Purpose,
        code_hash: &'a str,
        now: &'a str,
    ) -> BoxFuture<'a, StoreResult<bool>>;
    /// One write: counts a try on a live code (removing it when that uses it up) and returns the tries; an expired or
    /// used-up row is removed and `None` returned
    fn count_email_code_try<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
        purpose: Purpose,
        now: &'a str,
        max_attempts: u32,
    ) -> BoxFuture<'a, StoreResult<Option<u32>>>;

    // Keys
    /// The hashes of every game server key.
    fn gameserver_key_hashes(&self) -> BoxFuture<'_, StoreResult<Vec<String>>>;
    /// The complete set (an empty list empties it)
    fn replace_gameserver_keys<'a>(
        &'a self,
        actor: &'a ActorContext,
        hashes: &'a [String],
    ) -> BoxFuture<'a, StoreResult<()>>;
    /// `(key_hash, account_id)` of every user key
    fn user_key_hashes(&self) -> BoxFuture<'_, StoreResult<Vec<(String, String)>>>;

    /// A trivial query
    fn health(&self) -> BoxFuture<'_, StoreResult<()>>;
}

/// The core holds `Auth<Arc<dyn AuthStore>>`.
impl<T: AuthStore + ?Sized> AuthStore for std::sync::Arc<T> {
    fn game_account_by_user_id<'a>(&'a self, user_id: &'a str) -> BoxFuture<'a, StoreResult<Option<GameAccount>>> {
        (**self).game_account_by_user_id(user_id)
    }
    fn game_account_by_id<'a>(&'a self, account_id: &'a str) -> BoxFuture<'a, StoreResult<Option<GameAccount>>> {
        (**self).game_account_by_id(account_id)
    }
    fn ensure_game_account<'a>(
        &'a self,
        actor: &'a ActorContext,
        account: &'a GameAccount,
    ) -> BoxFuture<'a, StoreResult<bool>> {
        (**self).ensure_game_account(actor, account)
    }
    fn launcher_account_by_user_id<'a>(
        &'a self,
        user_id: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<LauncherAccount>>> {
        (**self).launcher_account_by_user_id(user_id)
    }
    fn launcher_account_by_id<'a>(
        &'a self,
        account_id: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<LauncherAccount>>> {
        (**self).launcher_account_by_id(account_id)
    }
    fn launcher_account_by_email<'a>(
        &'a self,
        normalized_email: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<LauncherAccount>>> {
        (**self).launcher_account_by_email(normalized_email)
    }
    fn launcher_account_by_name<'a>(
        &'a self,
        normalized_name: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<LauncherAccount>>> {
        (**self).launcher_account_by_name(normalized_name)
    }
    fn create_launcher_account<'a>(
        &'a self,
        actor: &'a ActorContext,
        account: &'a NewLauncherAccount,
    ) -> BoxFuture<'a, StoreResult<CreateOutcome>> {
        (**self).create_launcher_account(actor, account)
    }
    fn set_last_login<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
        at: &'a str,
    ) -> BoxFuture<'a, StoreResult<()>> {
        (**self).set_last_login(actor, account_id, at)
    }
    fn set_username<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
        display_name: &'a str,
        normalized: &'a str,
    ) -> BoxFuture<'a, StoreResult<bool>> {
        (**self).set_username(actor, account_id, display_name, normalized)
    }
    fn set_consent<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
        consent: &'a Consent,
    ) -> BoxFuture<'a, StoreResult<()>> {
        (**self).set_consent(actor, account_id, consent)
    }
    fn set_deletion<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
        deletion: Option<&'a Deletion>,
    ) -> BoxFuture<'a, StoreResult<()>> {
        (**self).set_deletion(actor, account_id, deletion)
    }
    fn set_email_verified<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
        at: &'a str,
    ) -> BoxFuture<'a, StoreResult<()>> {
        (**self).set_email_verified(actor, account_id, at)
    }
    fn reset_password<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
        password_hash: &'a str,
        at: &'a str,
    ) -> BoxFuture<'a, StoreResult<()>> {
        (**self).reset_password(actor, account_id, password_hash, at)
    }
    fn due_deletions<'a>(&'a self, now: &'a str, limit: usize) -> BoxFuture<'a, StoreResult<Vec<LauncherAccount>>> {
        (**self).due_deletions(now, limit)
    }
    fn stale_unverified<'a>(
        &'a self,
        before: &'a str,
        limit: usize,
    ) -> BoxFuture<'a, StoreResult<Vec<LauncherAccount>>> {
        (**self).stale_unverified(before, limit)
    }
    fn delete_unverified_registration<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
    ) -> BoxFuture<'a, StoreResult<bool>> {
        (**self).delete_unverified_registration(actor, account_id)
    }
    fn erase_account<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
    ) -> BoxFuture<'a, StoreResult<ErasureCounts>> {
        (**self).erase_account(actor, account_id)
    }
    fn identity_by_subject<'a>(&'a self, subject: &'a str) -> BoxFuture<'a, StoreResult<Option<Identity>>> {
        (**self).identity_by_subject(subject)
    }
    fn identity_by_account<'a>(&'a self, account_id: &'a str) -> BoxFuture<'a, StoreResult<Option<Identity>>> {
        (**self).identity_by_account(account_id)
    }
    fn create_identity<'a>(
        &'a self,
        actor: &'a ActorContext,
        identity: &'a Identity,
    ) -> BoxFuture<'a, StoreResult<bool>> {
        (**self).create_identity(actor, identity)
    }
    fn delete_identities_for_account<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
    ) -> BoxFuture<'a, StoreResult<u64>> {
        (**self).delete_identities_for_account(actor, account_id)
    }
    fn create_session<'a>(
        &'a self,
        actor: &'a ActorContext,
        session: &'a RefreshSession,
    ) -> BoxFuture<'a, StoreResult<()>> {
        (**self).create_session(actor, session)
    }
    fn session_by_token_hash<'a>(&'a self, token_hash: &'a str) -> BoxFuture<'a, StoreResult<Option<RefreshSession>>> {
        (**self).session_by_token_hash(token_hash)
    }
    fn rotate_session<'a>(
        &'a self,
        actor: &'a ActorContext,
        revoke_id: &'a str,
        at: &'a str,
        next: &'a RefreshSession,
    ) -> BoxFuture<'a, StoreResult<()>> {
        (**self).rotate_session(actor, revoke_id, at, next)
    }
    fn revoke_family<'a>(
        &'a self,
        actor: &'a ActorContext,
        family_id: &'a str,
        at: &'a str,
    ) -> BoxFuture<'a, StoreResult<u64>> {
        (**self).revoke_family(actor, family_id, at)
    }
    fn revoke_all_sessions<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
        at: &'a str,
    ) -> BoxFuture<'a, StoreResult<u64>> {
        (**self).revoke_all_sessions(actor, account_id, at)
    }
    fn is_family_active<'a>(
        &'a self,
        family_id: &'a str,
        account_id: &'a str,
        now: &'a str,
    ) -> BoxFuture<'a, StoreResult<bool>> {
        (**self).is_family_active(family_id, account_id, now)
    }
    fn create_exchange_code<'a>(
        &'a self,
        actor: &'a ActorContext,
        code: &'a ExchangeCode,
    ) -> BoxFuture<'a, StoreResult<()>> {
        (**self).create_exchange_code(actor, code)
    }
    fn consume_exchange_code<'a>(
        &'a self,
        actor: &'a ActorContext,
        code_hash: &'a str,
        now: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<ExchangeCode>>> {
        (**self).consume_exchange_code(actor, code_hash, now)
    }
    fn create_discord_transaction<'a>(
        &'a self,
        actor: &'a ActorContext,
        transaction: &'a DiscordTransaction,
    ) -> BoxFuture<'a, StoreResult<()>> {
        (**self).create_discord_transaction(actor, transaction)
    }
    fn consume_discord_state<'a>(
        &'a self,
        actor: &'a ActorContext,
        state: &'a str,
        now: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<DiscordTransaction>>> {
        (**self).consume_discord_state(actor, state, now)
    }
    fn attach_discord_completion<'a>(
        &'a self,
        actor: &'a ActorContext,
        state: &'a str,
        completion_hash: &'a str,
        expires_at: &'a str,
        account_id: &'a str,
    ) -> BoxFuture<'a, StoreResult<()>> {
        (**self).attach_discord_completion(actor, state, completion_hash, expires_at, account_id)
    }
    fn consume_discord_completion<'a>(
        &'a self,
        actor: &'a ActorContext,
        completion_hash: &'a str,
        now: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<DiscordTransaction>>> {
        (**self).consume_discord_completion(actor, completion_hash, now)
    }
    fn replace_email_code<'a>(
        &'a self,
        actor: &'a ActorContext,
        code: &'a EmailCode,
    ) -> BoxFuture<'a, StoreResult<()>> {
        (**self).replace_email_code(actor, code)
    }
    fn live_email_code<'a>(
        &'a self,
        account_id: &'a str,
        purpose: Purpose,
        now: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<EmailCode>>> {
        (**self).live_email_code(account_id, purpose, now)
    }
    fn consume_email_code<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
        purpose: Purpose,
        code_hash: &'a str,
        now: &'a str,
    ) -> BoxFuture<'a, StoreResult<bool>> {
        (**self).consume_email_code(actor, account_id, purpose, code_hash, now)
    }
    fn count_email_code_try<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
        purpose: Purpose,
        now: &'a str,
        max_attempts: u32,
    ) -> BoxFuture<'a, StoreResult<Option<u32>>> {
        (**self).count_email_code_try(actor, account_id, purpose, now, max_attempts)
    }
    fn gameserver_key_hashes(&self) -> BoxFuture<'_, StoreResult<Vec<String>>> {
        (**self).gameserver_key_hashes()
    }
    fn replace_gameserver_keys<'a>(
        &'a self,
        actor: &'a ActorContext,
        hashes: &'a [String],
    ) -> BoxFuture<'a, StoreResult<()>> {
        (**self).replace_gameserver_keys(actor, hashes)
    }
    fn user_key_hashes(&self) -> BoxFuture<'_, StoreResult<Vec<(String, String)>>> {
        (**self).user_key_hashes()
    }
    fn health(&self) -> BoxFuture<'_, StoreResult<()>> {
        (**self).health()
    }
}
