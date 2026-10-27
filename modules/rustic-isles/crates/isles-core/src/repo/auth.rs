//! RU5 auth: game accounts, launcher accounts and their sessions, sign-in codes and API keys.

use diesel::prelude::*;

use super::{AccountId, AnyConnection, RepoError, RepoResult, Tx, UserId, repository};

/// `AccountRecord`: the game account. The only table RU4 lays down (`migrations/*/0001_accounts`).
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

diesel::table! {
    accounts (id) {
        id -> Text,
        user_id -> Text,
        name -> Text,
        notes -> BigInt,
        deleted_at -> Nullable<BigInt>,
    }
}

type AccountRow = (String, String, String, i64);

fn account(row: AccountRow) -> RepoResult<AccountRecord> {
    let (id, user_id, name, notes) = row;
    let id = id
        .parse()
        .map_err(|_| RepoError::Data(format!("accounts.id {id:?} is not an account id")))?;
    Ok(AccountRecord {
        id,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationalStatus {
    Active,
    Banned,
    Disabled,
}

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

/// `LauncherAccountRecord`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherAccountRecord {
    pub account: AccountId,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminAccountFilter {
    pub approval_status: Option<ApprovalStatus>,
    pub status: Option<OperationalStatus>,
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

repository! {
    pub trait LauncherAccountRepository("launcherAccounts") {
        fn find_by_account(&self, conn: &mut AnyConnection, account: &AccountId) -> Option<LauncherAccountRecord>;
        fn find_by_email(&self, conn: &mut AnyConnection, normalized_email: &str) -> Option<LauncherAccountRecord>;
        fn find_by_display_name_normalized(&self, conn: &mut AnyConnection, normalized: &str)
            -> Option<LauncherAccountRecord>;
        fn create(&self, tx: &mut Tx<'_>, account: &LauncherAccountRecord) -> ();
        fn update_last_login(&self, tx: &mut Tx<'_>, account: &AccountId, when: &str) -> ();
        fn set_username(&self, tx: &mut Tx<'_>, account: &AccountId, display_name: &str, normalized: &str) -> ();
        fn set_consent(&self, tx: &mut Tx<'_>, account: &AccountId, consent: &LauncherConsent) -> ();
        fn set_deletion(&self, tx: &mut Tx<'_>, account: &AccountId, deletion: Option<&LauncherDeletion>) -> ();
        fn list_deletions_due(&self, conn: &mut AnyConnection, now: &str, limit: u32) -> Vec<LauncherAccountRecord>;
        fn set_email_verified(&self, tx: &mut Tx<'_>, account: &AccountId, verified_at: &str) -> ();
        fn set_password_hash(&self, tx: &mut Tx<'_>, account: &AccountId, password_hash: &str) -> ();
        fn list_unverified_registrations(&self, conn: &mut AnyConnection, pending_before: &str, limit: u32)
            -> Vec<LauncherAccountRecord>;
        fn delete_unverified_registration(&self, tx: &mut Tx<'_>, account: &AccountId, pending_before: &str) -> bool;
        /// The records and the total
        fn list_for_admin(&self, conn: &mut AnyConnection, filter: &AdminAccountFilter)
            -> (Vec<LauncherAccountRecord>, u64);
        fn set_access_state(&self, tx: &mut Tx<'_>, account: &AccountId, change: &AccessChange)
            -> Option<LauncherAccountRecord>;
        fn update_roles(&self, tx: &mut Tx<'_>, account: &AccountId, roles: &[String], at: &str, by: &str)
            -> Option<LauncherAccountRecord>;
    }
}

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
        fn create(&self, tx: &mut Tx<'_>, identity: &AuthIdentityRecord) -> ();
        /// How many went
        fn delete_for_account(&self, tx: &mut Tx<'_>, account: &AccountId) -> u64;
    }
}

/// `RefreshSessionRecord`: a launcher session's refresh token, by hash.
#[derive(Debug, Clone, PartialEq, Eq)]
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

repository! {
    pub trait RefreshSessionRepository("refreshSessions") {
        fn create(&self, tx: &mut Tx<'_>, session: &RefreshSessionRecord) -> ();
        fn find_by_token_hash(&self, conn: &mut AnyConnection, token_hash: &str) -> Option<RefreshSessionRecord>;
        fn revoke_by_id(&self, tx: &mut Tx<'_>, id: &str) -> ();
        fn revoke_family(&self, tx: &mut Tx<'_>, family_id: &str) -> ();
        fn revoke_all_for_account(&self, tx: &mut Tx<'_>, account: &AccountId) -> ();
        fn delete_all_for_account(&self, tx: &mut Tx<'_>, account: &AccountId) -> ();
        fn is_family_active(&self, conn: &mut AnyConnection, family_id: &str, account: &AccountId) -> bool;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeChannel {
    Stable,
    Beta,
    Dev,
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
        fn consume_by_code_hash(&self, tx: &mut Tx<'_>, code_hash: &str) -> Option<GameExchangeCodeRecord>;
        fn revoke_unused_for_account(&self, tx: &mut Tx<'_>, account: &AccountId) -> ();
    }
}

/// `DiscordOAuthTransactionRecord`.
#[derive(Debug, Clone, PartialEq, Eq)]
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

repository! {
    pub trait DiscordOAuthTransactionRepository("discordOAuthTransactions") {
        fn create(&self, tx: &mut Tx<'_>, transaction: &DiscordOAuthTransactionRecord) -> ();
        fn consume_by_state(&self, tx: &mut Tx<'_>, state: &str) -> Option<DiscordOAuthTransactionRecord>;
        fn attach_completion_code(&self, tx: &mut Tx<'_>, state: &str, code_hash: &str, expires_at: &str,
            account: &AccountId) -> ();
        fn consume_by_completion_code_hash(&self, tx: &mut Tx<'_>, code_hash: &str)
            -> Option<DiscordOAuthTransactionRecord>;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailCodePurpose {
    Verify,
    Reset,
    Unlink,
}

/// `EmailCodeRecord`: the live one-time code of email verification and password reset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailCodeRecord {
    pub account: AccountId,
    pub purpose: EmailCodePurpose,
    pub code_hash: String,
    pub attempts: u32,
    pub created_at: String,
    pub expires_at: String,
}

repository! {
    pub trait EmailCodeRepository("emailCodes") {
        fn replace(&self, tx: &mut Tx<'_>, record: &EmailCodeRecord) -> ();
        fn peek_live(&self, conn: &mut AnyConnection, account: &AccountId, purpose: EmailCodePurpose, now: &str,
            max_tries: u32) -> Option<EmailCodeRecord>;
        fn consume(&self, tx: &mut Tx<'_>, account: &AccountId, purpose: EmailCodePurpose, code_hash: &str,
            now: &str, max_tries: u32) -> bool;
        fn reserve_try(&self, tx: &mut Tx<'_>, account: &AccountId, purpose: EmailCodePurpose, now: &str,
            max_tries: u32) -> Option<EmailCodeRecord>;
        fn remove(&self, tx: &mut Tx<'_>, account: &AccountId, purpose: EmailCodePurpose, code_hash: &str) -> bool;
        fn delete_for_account(&self, tx: &mut Tx<'_>, account: &AccountId) -> u64;
    }
}

/// `GameServerApiKeyRecord`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameServerKeyHash {
    pub id: i64,
    pub key_hash: Option<String>,
}

/// `UserApiKeyRecord`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserKeyHash {
    pub account: AccountId,
    pub key_hash: String,
}

/// A key the host asked to register; the plain key never lands anywhere but its hash (`*ToRegisterRecord`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyToRegister {
    pub account: Option<AccountId>,
    pub key: String,
}

repository! {
    pub trait ApiKeyRepository("apiKeys") {
        fn find_all_game_server_key_hashes(&self, conn: &mut AnyConnection) -> Vec<GameServerKeyHash>;
        fn insert_game_server_key_hash(&self, tx: &mut Tx<'_>, key_hash: &str) -> ();
        fn replace_game_server_key_hashes(&self, tx: &mut Tx<'_>, key_hashes: &[String]) -> ();
        fn find_all_game_server_keys_to_register(&self, conn: &mut AnyConnection) -> Vec<KeyToRegister>;
        fn clear_game_server_keys_to_register(&self, tx: &mut Tx<'_>) -> ();
        fn find_all_user_key_hashes(&self, conn: &mut AnyConnection) -> Vec<UserKeyHash>;
        fn insert_user_key_hash(&self, tx: &mut Tx<'_>, account: &AccountId, key_hash: &str) -> ();
        fn find_all_user_keys_to_register(&self, conn: &mut AnyConnection) -> Vec<KeyToRegister>;
        fn clear_user_keys_to_register(&self, tx: &mut Tx<'_>) -> ();
    }
}
