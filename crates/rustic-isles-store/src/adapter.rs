// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `rustic_isles_auth::AuthStore` over [`IslesStore`] (spec §12). A player actor's write is an account write for
//! that player (the home check); any other actor's is a plain write.

use module_api::{Actor, ActorContext, BoxFuture};
use rustic_isles_auth::email::CODE_MAX_ATTEMPTS;
use rustic_isles_auth::{
    Approval, AuthStore, Channel, Consent, CreateOutcome, Deletion, DiscordTransaction, EmailCode, ErasureCounts,
    ExchangeCode, GameAccount, Identity, LauncherAccount, NewLauncherAccount, Purpose, RefreshSession, Status,
    StoreError, StoreResult, Timeout,
};

use crate::repo::Repositories;
use crate::repo::auth::{
    self as repo_auth, AccountRecord, ApprovalStatus, AuthIdentityRecord, CreateOutcome as RepoOutcome,
    DiscordOAuthTransactionRecord, EmailCodePurpose, EmailCodeRecord, GameExchangeCodeRecord, LauncherAccountRecord,
    LauncherConsent, LauncherDeletion, OperationalStatus, RefreshSessionRecord, RuntimeChannel,
};
use crate::repo::reforged::erase_other_areas;
use crate::{AccountId, AnyConnection, IslesStore, RepoError, RepoResult, Tx, UserId};

/// What core's composer hands `Auth` as its store.
#[derive(Clone)]
pub struct IslesAuthStore {
    store: IslesStore,
}

impl IslesAuthStore {
    pub fn new(store: IslesStore) -> Self {
        Self { store }
    }

    async fn read<T, F>(&self, f: F) -> StoreResult<T>
    where
        F: FnOnce(&mut AnyConnection, &Repositories) -> RepoResult<T> + Send + 'static,
        T: Send + 'static,
    {
        self.store.read(f).await.map_err(store_error)
    }

    async fn write<T, F>(&self, actor: &ActorContext, f: F) -> StoreResult<T>
    where
        F: FnOnce(&mut Tx<'_>, &Repositories) -> RepoResult<T> + Send + 'static,
        T: Send + 'static,
    {
        let result = match &actor.actor {
            Actor::Player { account } => match AccountId::try_from(account.clone()) {
                Ok(id) => self.store.account_write(&id, actor, f).await,
                Err(e) => Err(RepoError::data(e.to_string())),
            },
            _ => self.store.write(actor, f).await,
        };
        result.map_err(store_error)
    }
}

fn store_error(e: RepoError) -> StoreError {
    StoreError::new(e.to_string())
}

/// A write's id must parse.
fn require(id: &str) -> StoreResult<AccountId> {
    id.parse()
        .map_err(|_| StoreError::new(format!("\"{id}\" is not an account id")))
}

/// A read's id that does not parse matches no row.
fn parsed(id: &str) -> Option<AccountId> {
    id.parse().ok()
}

fn game_account(r: AccountRecord) -> GameAccount {
    GameAccount {
        id: r.id.to_string(),
        user_id: r.user_id.as_str().to_owned(),
        name: r.name,
        notes: r.notes,
    }
}

fn status_out(s: OperationalStatus) -> Status {
    match s {
        OperationalStatus::Active => Status::Active,
        OperationalStatus::Banned => Status::Banned,
        OperationalStatus::Disabled => Status::Disabled,
    }
}

fn status_in(s: Status) -> OperationalStatus {
    match s {
        Status::Active => OperationalStatus::Active,
        Status::Banned => OperationalStatus::Banned,
        Status::Disabled => OperationalStatus::Disabled,
    }
}

fn approval_out(a: ApprovalStatus) -> Approval {
    match a {
        ApprovalStatus::Pending => Approval::Pending,
        ApprovalStatus::Approved => Approval::Approved,
        ApprovalStatus::Rejected => Approval::Rejected,
    }
}

fn approval_in(a: Approval) -> ApprovalStatus {
    match a {
        Approval::Pending => ApprovalStatus::Pending,
        Approval::Approved => ApprovalStatus::Approved,
        Approval::Rejected => ApprovalStatus::Rejected,
    }
}

fn launcher_account(r: LauncherAccountRecord) -> LauncherAccount {
    LauncherAccount {
        account_id: r.account.to_string(),
        user_id: r.user_id.as_str().to_owned(),
        email: r.email,
        display_name: r.display_name,
        display_name_normalized: r.display_name_normalized,
        password_hash: r.password_hash,
        status: status_out(r.status),
        approval: r.approval_status.map(approval_out),
        roles: r.roles,
        roles_updated_at: r.roles_updated_at,
        created_at: r.created_at,
        last_login_at: r.last_login_at,
        username_set: r.username_set,
        consent: r.consent.map(|c| Consent {
            terms_version: c.terms_version,
            privacy_version: c.privacy_version,
            accepted_at: c.accepted_at,
        }),
        deletion: r.deletion.map(|d| Deletion {
            requested_at: d.requested_at,
            scheduled_for: d.scheduled_for,
        }),
        email_verified_at: r.email_verified_at,
        email_verification_pending_since: r.email_verification_pending_since,
        timeout: r.timeout.map(|t| Timeout {
            until: t.until,
            since: t.since,
            reason: t.reason,
        }),
    }
}

fn launcher_record(a: &LauncherAccount, account: AccountId) -> LauncherAccountRecord {
    LauncherAccountRecord {
        account,
        user_id: UserId::new(a.user_id.clone()),
        email: a.email.clone(),
        display_name_normalized: a.display_name_normalized.clone(),
        display_name: a.display_name.clone(),
        password_hash: a.password_hash.clone(),
        status: status_in(a.status),
        approval_status: a.approval.map(approval_in),
        approval_updated_at: None,
        approval_updated_by: None,
        approval_reason: None,
        roles: a.roles.clone(),
        roles_updated_at: a.roles_updated_at.clone(),
        roles_updated_by: None,
        created_at: a.created_at.clone(),
        last_login_at: a.last_login_at.clone(),
        username_set: a.username_set,
        consent: a.consent.as_ref().map(consent_in),
        deletion: a.deletion.as_ref().map(deletion_in),
        email_verified_at: a.email_verified_at.clone(),
        email_verification_pending_since: a.email_verification_pending_since.clone(),
        // Registration writes no timeout
        timeout: None,
    }
}

fn consent_in(c: &Consent) -> LauncherConsent {
    LauncherConsent {
        terms_version: c.terms_version.clone(),
        privacy_version: c.privacy_version.clone(),
        accepted_at: c.accepted_at.clone(),
    }
}

fn deletion_in(d: &Deletion) -> LauncherDeletion {
    LauncherDeletion {
        requested_at: d.requested_at.clone(),
        scheduled_for: d.scheduled_for.clone(),
    }
}

fn identity(r: AuthIdentityRecord) -> Identity {
    Identity {
        provider_subject: r.provider_subject,
        account_id: r.account.to_string(),
        provider_username: r.provider_username,
        provider_avatar_url: r.provider_avatar_url,
        linked_at: r.linked_at,
    }
}

fn session(r: RefreshSessionRecord) -> RefreshSession {
    RefreshSession {
        id: r.id,
        token_hash: r.token_hash,
        account_id: r.account.to_string(),
        family_id: r.family_id,
        device_id: r.device_id,
        device_name: r.device_name,
        created_at: r.created_at,
        expires_at: r.expires_at,
        revoked_at: r.revoked_at,
    }
}

fn session_record(s: &RefreshSession) -> StoreResult<RefreshSessionRecord> {
    Ok(RefreshSessionRecord {
        id: s.id.clone(),
        token_hash: s.token_hash.clone(),
        account: require(&s.account_id)?,
        family_id: s.family_id.clone(),
        device_id: s.device_id.clone(),
        device_name: s.device_name.clone(),
        created_at: s.created_at.clone(),
        expires_at: s.expires_at.clone(),
        revoked_at: s.revoked_at.clone(),
    })
}

fn channel_out(c: RuntimeChannel) -> Channel {
    match c {
        RuntimeChannel::Stable => Channel::Stable,
        RuntimeChannel::Beta => Channel::Beta,
        RuntimeChannel::Dev => Channel::Dev,
    }
}

fn channel_in(c: Channel) -> RuntimeChannel {
    match c {
        Channel::Stable => RuntimeChannel::Stable,
        Channel::Beta => RuntimeChannel::Beta,
        Channel::Dev => RuntimeChannel::Dev,
    }
}

fn exchange_code(r: GameExchangeCodeRecord) -> ExchangeCode {
    ExchangeCode {
        code_hash: r.code_hash,
        account_id: r.account.to_string(),
        launcher_session_id: r.launcher_session_id,
        build_changelist: r.build_changelist.into(),
        executable_sha256: r.executable_sha256,
        runtime_channel: channel_out(r.runtime_channel),
        runtime_sha256: r.runtime_sha256,
        runtime_manifest_version: r.runtime_manifest_version,
        runtime_artifact_set_sha256: r.runtime_artifact_set_sha256,
        realm_id: r.realm_id,
        created_at: r.created_at,
        expires_at: r.expires_at,
        consumed_at: r.consumed_at,
    }
}

fn exchange_record(c: &ExchangeCode) -> StoreResult<GameExchangeCodeRecord> {
    Ok(GameExchangeCodeRecord {
        code_hash: c.code_hash.clone(),
        account: require(&c.account_id)?,
        launcher_session_id: c.launcher_session_id.clone(),
        build_changelist: u32::try_from(c.build_changelist)
            .map_err(|_| StoreError::new(format!("build changelist {} is out of range", c.build_changelist)))?,
        executable_sha256: c.executable_sha256.clone(),
        runtime_channel: channel_in(c.runtime_channel),
        runtime_sha256: c.runtime_sha256.clone(),
        runtime_manifest_version: c.runtime_manifest_version.clone(),
        runtime_artifact_set_sha256: c.runtime_artifact_set_sha256.clone(),
        realm_id: c.realm_id.clone(),
        created_at: c.created_at.clone(),
        expires_at: c.expires_at.clone(),
        consumed_at: c.consumed_at.clone(),
    })
}

fn discord(r: DiscordOAuthTransactionRecord) -> DiscordTransaction {
    DiscordTransaction {
        state: r.state,
        code_verifier: r.code_verifier,
        created_at: r.created_at,
        expires_at: r.expires_at,
        consumed_at: r.consumed_at,
        completion_code_hash: r.completion_code_hash,
        completion_expires_at: r.completion_expires_at,
        completion_consumed_at: r.completion_consumed_at,
        account_id: r.account.map(|a| a.to_string()),
        link_account_id: r.link_account.map(|a| a.to_string()),
    }
}

fn discord_record(t: &DiscordTransaction) -> StoreResult<DiscordOAuthTransactionRecord> {
    Ok(DiscordOAuthTransactionRecord {
        state: t.state.clone(),
        code_verifier: t.code_verifier.clone(),
        created_at: t.created_at.clone(),
        expires_at: t.expires_at.clone(),
        consumed_at: t.consumed_at.clone(),
        completion_code_hash: t.completion_code_hash.clone(),
        completion_expires_at: t.completion_expires_at.clone(),
        completion_consumed_at: t.completion_consumed_at.clone(),
        account: t.account_id.as_deref().map(require).transpose()?,
        link_account: t.link_account_id.as_deref().map(require).transpose()?,
    })
}

fn purpose_in(p: Purpose) -> EmailCodePurpose {
    match p {
        Purpose::Verify => EmailCodePurpose::Verify,
        Purpose::Reset => EmailCodePurpose::Reset,
        Purpose::Unlink => EmailCodePurpose::Unlink,
    }
}

fn purpose_out(p: EmailCodePurpose) -> Purpose {
    match p {
        EmailCodePurpose::Verify => Purpose::Verify,
        EmailCodePurpose::Reset => Purpose::Reset,
        EmailCodePurpose::Unlink => Purpose::Unlink,
    }
}

fn email_code(r: EmailCodeRecord) -> EmailCode {
    EmailCode {
        account_id: r.account.to_string(),
        purpose: purpose_out(r.purpose),
        code_hash: r.code_hash,
        attempts: r.attempts,
        created_at: r.created_at,
        expires_at: r.expires_at,
    }
}

fn limit(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

impl AuthStore for IslesAuthStore {
    fn game_account_by_user_id<'a>(&'a self, user_id: &'a str) -> BoxFuture<'a, StoreResult<Option<GameAccount>>> {
        let user_id = UserId::new(user_id);
        Box::pin(async move {
            self.read(move |conn, repos| repos.accounts.find_by_user_id(conn, &user_id))
                .await
                .map(|r| r.map(game_account))
        })
    }

    fn game_account_by_id<'a>(&'a self, account_id: &'a str) -> BoxFuture<'a, StoreResult<Option<GameAccount>>> {
        Box::pin(async move {
            let Some(id) = parsed(account_id) else { return Ok(None) };
            self.read(move |conn, repos| repos.accounts.find(conn, &id))
                .await
                .map(|r| r.map(game_account))
        })
    }

    fn ensure_game_account<'a>(
        &'a self,
        actor: &'a ActorContext,
        account: &'a GameAccount,
    ) -> BoxFuture<'a, StoreResult<bool>> {
        Box::pin(async move {
            let record = AccountRecord {
                id: require(&account.id)?,
                user_id: UserId::new(account.user_id.clone()),
                name: account.name.clone(),
                notes: account.notes,
            };
            self.write(actor, move |tx, repos| {
                if repos.accounts.find_by_user_id(tx.conn(), &record.user_id)?.is_some() {
                    return Ok(false);
                }
                repos.accounts.create(tx, &record)?;
                Ok(true)
            })
            .await
        })
    }

    fn launcher_account_by_user_id<'a>(
        &'a self,
        user_id: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<LauncherAccount>>> {
        let user_id = UserId::new(user_id);
        Box::pin(async move {
            self.read(move |conn, repos| repos.launcher_accounts.find_by_user_id(conn, &user_id))
                .await
                .map(|r| r.map(launcher_account))
        })
    }

    fn launcher_account_by_id<'a>(
        &'a self,
        account_id: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<LauncherAccount>>> {
        Box::pin(async move {
            let Some(id) = parsed(account_id) else { return Ok(None) };
            self.read(move |conn, repos| repos.launcher_accounts.find_by_account(conn, &id))
                .await
                .map(|r| r.map(launcher_account))
        })
    }

    fn launcher_account_by_email<'a>(
        &'a self,
        normalized_email: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<LauncherAccount>>> {
        let email = normalized_email.to_owned();
        Box::pin(async move {
            self.read(move |conn, repos| repos.launcher_accounts.find_by_email(conn, &email))
                .await
                .map(|r| r.map(launcher_account))
        })
    }

    fn launcher_account_by_name<'a>(
        &'a self,
        normalized_name: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<LauncherAccount>>> {
        let name = normalized_name.to_owned();
        Box::pin(async move {
            self.read(move |conn, repos| repos.launcher_accounts.find_by_display_name_normalized(conn, &name))
                .await
                .map(|r| r.map(launcher_account))
        })
    }

    fn create_launcher_account<'a>(
        &'a self,
        actor: &'a ActorContext,
        account: &'a NewLauncherAccount,
    ) -> BoxFuture<'a, StoreResult<CreateOutcome>> {
        Box::pin(async move {
            let id = require(&account.account.account_id)?;
            let record = launcher_record(&account.account, id.clone());
            let game_name = account.game_name.clone();
            let outcome = self
                .write(actor, move |tx, repos| {
                    let existed = repo_auth::account_row_exists(tx.conn(), &id)?;
                    let outcome = repos.launcher_accounts.create(tx, &record)?;
                    // The game account it writes takes the registration's game name
                    if outcome == RepoOutcome::Created && !existed && game_name != record.display_name {
                        repo_auth::set_account_name(tx.conn(), &id, &game_name)?;
                    }
                    Ok(outcome)
                })
                .await?;
            Ok(match outcome {
                RepoOutcome::Created => CreateOutcome::Created(account.account.clone()),
                RepoOutcome::EmailTaken => CreateOutcome::EmailTaken,
                RepoOutcome::DisplayNameTaken => CreateOutcome::NameTaken,
            })
        })
    }

    fn set_last_login<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
        at: &'a str,
    ) -> BoxFuture<'a, StoreResult<()>> {
        Box::pin(async move {
            let id = require(account_id)?;
            let at = at.to_owned();
            self.write(actor, move |tx, repos| {
                repos.launcher_accounts.update_last_login(tx, &id, &at)
            })
            .await
        })
    }

    fn set_username<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
        display_name: &'a str,
        normalized: &'a str,
    ) -> BoxFuture<'a, StoreResult<bool>> {
        Box::pin(async move {
            let id = require(account_id)?;
            let (display, normalized) = (display_name.to_owned(), normalized.to_owned());
            self.write(actor, move |tx, repos| {
                Ok(repos.launcher_accounts.set_username(tx, &id, &display, &normalized)? == RepoOutcome::Created)
            })
            .await
        })
    }

    fn set_consent<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
        consent: &'a Consent,
    ) -> BoxFuture<'a, StoreResult<()>> {
        Box::pin(async move {
            let id = require(account_id)?;
            let consent = consent_in(consent);
            self.write(actor, move |tx, repos| {
                repos.launcher_accounts.set_consent(tx, &id, &consent)
            })
            .await
        })
    }

    fn set_deletion<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
        deletion: Option<&'a Deletion>,
    ) -> BoxFuture<'a, StoreResult<()>> {
        Box::pin(async move {
            let id = require(account_id)?;
            let deletion = deletion.map(deletion_in);
            self.write(actor, move |tx, repos| {
                repos.launcher_accounts.set_deletion(tx, &id, deletion.as_ref())
            })
            .await
        })
    }

    fn set_email_verified<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
        at: &'a str,
    ) -> BoxFuture<'a, StoreResult<()>> {
        Box::pin(async move {
            let id = require(account_id)?;
            let at = at.to_owned();
            self.write(actor, move |tx, repos| {
                repos.launcher_accounts.set_email_verified(tx, &id, &at)
            })
            .await
        })
    }

    fn reset_password<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
        password_hash: &'a str,
        at: &'a str,
    ) -> BoxFuture<'a, StoreResult<()>> {
        Box::pin(async move {
            let id = require(account_id)?;
            let (hash, at) = (password_hash.to_owned(), at.to_owned());
            self.write(actor, move |tx, repos| {
                repos.launcher_accounts.set_password_hash(tx, &id, &hash)?;
                repos.launcher_accounts.set_email_verified(tx, &id, &at)?;
                repos.refresh_sessions.revoke_all_for_account(tx, &id, &at)?;
                Ok(())
            })
            .await
        })
    }

    fn due_deletions<'a>(&'a self, now: &'a str, limit_n: usize) -> BoxFuture<'a, StoreResult<Vec<LauncherAccount>>> {
        let now = now.to_owned();
        Box::pin(async move {
            self.read(move |conn, repos| repos.launcher_accounts.list_deletions_due(conn, &now, limit(limit_n)))
                .await
                .map(|v| v.into_iter().map(launcher_account).collect())
        })
    }

    fn stale_unverified<'a>(
        &'a self,
        before: &'a str,
        limit_n: usize,
    ) -> BoxFuture<'a, StoreResult<Vec<LauncherAccount>>> {
        let before = before.to_owned();
        Box::pin(async move {
            self.read(move |conn, repos| {
                repos
                    .launcher_accounts
                    .list_unverified_registrations(conn, &before, limit(limit_n))
            })
            .await
            .map(|v| v.into_iter().map(launcher_account).collect())
        })
    }

    fn delete_unverified_registration<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
    ) -> BoxFuture<'a, StoreResult<bool>> {
        Box::pin(async move {
            let id = require(account_id)?;
            self.write(actor, move |tx, repos| {
                if !repo_auth::delete_pending_registration(tx, &id)? {
                    return Ok(false);
                }
                repos.email_codes.delete_for_account(tx, &id)?;
                repos.refresh_sessions.delete_all_for_account(tx, &id)?;
                Ok(true)
            })
            .await
        })
    }

    fn erase_account<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
    ) -> BoxFuture<'a, StoreResult<ErasureCounts>> {
        Box::pin(async move {
            let id = require(account_id)?;
            self.write(actor, move |tx, repos| {
                repos.refresh_sessions.delete_all_for_account(tx, &id)?;
                let email_codes = repos.email_codes.delete_for_account(tx, &id)?;
                let game_exchange_codes = repos.game_exchange_codes.delete_for_account(tx, &id)?;
                let discord_oauth_transactions = repos.discord_oauth_transactions.delete_for_account(tx, &id)?;
                let auth_identities = repos.auth_identities.delete_for_account(tx, &id)?;
                let user_api_keys = u64::from(repos.api_keys.delete_user_key_hash(tx, &id)?);
                let friendships = repos.friendships.remove_all_for_account(tx, &id)?;
                let (other, _) = erase_other_areas(tx, &id)?;
                let other_areas = other.iter().map(|(_, n)| n).sum();
                let accounts = u64::from(repos.launcher_accounts.erase(tx, &id)?);
                Ok(ErasureCounts {
                    email_codes,
                    game_exchange_codes,
                    discord_oauth_transactions,
                    auth_identities,
                    user_api_keys,
                    friendships,
                    other_areas,
                    accounts,
                })
            })
            .await
        })
    }

    fn identity_by_subject<'a>(&'a self, subject: &'a str) -> BoxFuture<'a, StoreResult<Option<Identity>>> {
        let subject = subject.to_owned();
        Box::pin(async move {
            self.read(move |conn, repos| repos.auth_identities.find_by_provider_subject(conn, &subject))
                .await
                .map(|r| r.map(identity))
        })
    }

    fn identity_by_account<'a>(&'a self, account_id: &'a str) -> BoxFuture<'a, StoreResult<Option<Identity>>> {
        Box::pin(async move {
            let Some(id) = parsed(account_id) else { return Ok(None) };
            self.read(move |conn, repos| repos.auth_identities.find_by_account(conn, &id))
                .await
                .map(|r| r.map(identity))
        })
    }

    fn create_identity<'a>(
        &'a self,
        actor: &'a ActorContext,
        identity: &'a Identity,
    ) -> BoxFuture<'a, StoreResult<bool>> {
        Box::pin(async move {
            let record = AuthIdentityRecord {
                provider_subject: identity.provider_subject.clone(),
                account: require(&identity.account_id)?,
                provider_username: identity.provider_username.clone(),
                provider_avatar_url: identity.provider_avatar_url.clone(),
                linked_at: identity.linked_at.clone(),
            };
            self.write(actor, move |tx, repos| repos.auth_identities.create(tx, &record))
                .await
        })
    }

    fn delete_identities_for_account<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
    ) -> BoxFuture<'a, StoreResult<u64>> {
        Box::pin(async move {
            let id = require(account_id)?;
            self.write(actor, move |tx, repos| {
                repos.auth_identities.delete_for_account(tx, &id)
            })
            .await
        })
    }

    fn create_session<'a>(
        &'a self,
        actor: &'a ActorContext,
        session: &'a RefreshSession,
    ) -> BoxFuture<'a, StoreResult<()>> {
        Box::pin(async move {
            let record = session_record(session)?;
            self.write(actor, move |tx, repos| repos.refresh_sessions.create(tx, &record))
                .await
        })
    }

    fn session_by_token_hash<'a>(&'a self, token_hash: &'a str) -> BoxFuture<'a, StoreResult<Option<RefreshSession>>> {
        let hash = token_hash.to_owned();
        Box::pin(async move {
            self.read(move |conn, repos| repos.refresh_sessions.find_by_token_hash(conn, &hash))
                .await
                .map(|r| r.map(session))
        })
    }

    fn rotate_session<'a>(
        &'a self,
        actor: &'a ActorContext,
        revoke_id: &'a str,
        at: &'a str,
        next: &'a RefreshSession,
    ) -> BoxFuture<'a, StoreResult<()>> {
        Box::pin(async move {
            let record = session_record(next)?;
            let (revoke, at) = (revoke_id.to_owned(), at.to_owned());
            self.write(actor, move |tx, repos| {
                repos.refresh_sessions.revoke_by_id(tx, &revoke, &at)?;
                repos.refresh_sessions.create(tx, &record)
            })
            .await
        })
    }

    fn revoke_family<'a>(
        &'a self,
        actor: &'a ActorContext,
        family_id: &'a str,
        at: &'a str,
    ) -> BoxFuture<'a, StoreResult<u64>> {
        let (family, at) = (family_id.to_owned(), at.to_owned());
        Box::pin(async move {
            self.write(actor, move |tx, repos| {
                repos.refresh_sessions.revoke_family(tx, &family, &at)
            })
            .await
        })
    }

    fn revoke_all_sessions<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
        at: &'a str,
    ) -> BoxFuture<'a, StoreResult<u64>> {
        Box::pin(async move {
            let id = require(account_id)?;
            let at = at.to_owned();
            self.write(actor, move |tx, repos| {
                repos.refresh_sessions.revoke_all_for_account(tx, &id, &at)
            })
            .await
        })
    }

    fn is_family_active<'a>(
        &'a self,
        family_id: &'a str,
        account_id: &'a str,
        now: &'a str,
    ) -> BoxFuture<'a, StoreResult<bool>> {
        Box::pin(async move {
            let Some(id) = parsed(account_id) else { return Ok(false) };
            let (family, now) = (family_id.to_owned(), now.to_owned());
            self.read(move |conn, repos| repos.refresh_sessions.is_family_active(conn, &family, &id, &now))
                .await
        })
    }

    fn create_exchange_code<'a>(
        &'a self,
        actor: &'a ActorContext,
        code: &'a ExchangeCode,
    ) -> BoxFuture<'a, StoreResult<()>> {
        Box::pin(async move {
            let record = exchange_record(code)?;
            self.write(actor, move |tx, repos| repos.game_exchange_codes.create(tx, &record))
                .await
        })
    }

    fn consume_exchange_code<'a>(
        &'a self,
        actor: &'a ActorContext,
        code_hash: &'a str,
        now: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<ExchangeCode>>> {
        let (hash, now) = (code_hash.to_owned(), now.to_owned());
        Box::pin(async move {
            self.write(actor, move |tx, repos| {
                repos.game_exchange_codes.consume_by_code_hash(tx, &hash, &now)
            })
            .await
            .map(|r| r.map(exchange_code))
        })
    }

    fn create_discord_transaction<'a>(
        &'a self,
        actor: &'a ActorContext,
        transaction: &'a DiscordTransaction,
    ) -> BoxFuture<'a, StoreResult<()>> {
        Box::pin(async move {
            let record = discord_record(transaction)?;
            self.write(actor, move |tx, repos| {
                repos.discord_oauth_transactions.create(tx, &record)
            })
            .await
        })
    }

    fn consume_discord_state<'a>(
        &'a self,
        actor: &'a ActorContext,
        state: &'a str,
        now: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<DiscordTransaction>>> {
        let (state, now) = (state.to_owned(), now.to_owned());
        Box::pin(async move {
            self.write(actor, move |tx, repos| {
                repos.discord_oauth_transactions.consume_by_state(tx, &state, &now)
            })
            .await
            .map(|r| r.map(discord))
        })
    }

    fn attach_discord_completion<'a>(
        &'a self,
        actor: &'a ActorContext,
        state: &'a str,
        completion_hash: &'a str,
        expires_at: &'a str,
        account_id: &'a str,
    ) -> BoxFuture<'a, StoreResult<()>> {
        Box::pin(async move {
            let id = require(account_id)?;
            let (state, hash, expires) = (state.to_owned(), completion_hash.to_owned(), expires_at.to_owned());
            self.write(actor, move |tx, repos| {
                repos
                    .discord_oauth_transactions
                    .attach_completion_code(tx, &state, &hash, &expires, &id)
            })
            .await
        })
    }

    fn consume_discord_completion<'a>(
        &'a self,
        actor: &'a ActorContext,
        completion_hash: &'a str,
        now: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<DiscordTransaction>>> {
        let (hash, now) = (completion_hash.to_owned(), now.to_owned());
        Box::pin(async move {
            self.write(actor, move |tx, repos| {
                repos
                    .discord_oauth_transactions
                    .consume_by_completion_code_hash(tx, &hash, &now)
            })
            .await
            .map(|r| r.map(discord))
        })
    }

    fn replace_email_code<'a>(
        &'a self,
        actor: &'a ActorContext,
        code: &'a EmailCode,
    ) -> BoxFuture<'a, StoreResult<()>> {
        Box::pin(async move {
            let record = EmailCodeRecord {
                account: require(&code.account_id)?,
                purpose: purpose_in(code.purpose),
                code_hash: code.code_hash.clone(),
                attempts: code.attempts,
                created_at: code.created_at.clone(),
                expires_at: code.expires_at.clone(),
            };
            self.write(actor, move |tx, repos| repos.email_codes.replace(tx, &record))
                .await
        })
    }

    fn live_email_code<'a>(
        &'a self,
        account_id: &'a str,
        purpose: Purpose,
        now: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<EmailCode>>> {
        Box::pin(async move {
            let Some(id) = parsed(account_id) else { return Ok(None) };
            let now = now.to_owned();
            self.read(move |conn, repos| {
                repos
                    .email_codes
                    .peek_live(conn, &id, purpose_in(purpose), &now, CODE_MAX_ATTEMPTS)
            })
            .await
            .map(|r| r.map(email_code))
        })
    }

    fn consume_email_code<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
        purpose: Purpose,
        code_hash: &'a str,
        now: &'a str,
    ) -> BoxFuture<'a, StoreResult<bool>> {
        Box::pin(async move {
            let id = require(account_id)?;
            let (hash, now) = (code_hash.to_owned(), now.to_owned());
            self.write(actor, move |tx, repos| {
                repos
                    .email_codes
                    .consume(tx, &id, purpose_in(purpose), &hash, &now, CODE_MAX_ATTEMPTS)
            })
            .await
        })
    }

    fn count_email_code_try<'a>(
        &'a self,
        actor: &'a ActorContext,
        account_id: &'a str,
        purpose: Purpose,
        now: &'a str,
        max_attempts: u32,
    ) -> BoxFuture<'a, StoreResult<Option<u32>>> {
        Box::pin(async move {
            let id = require(account_id)?;
            let now = now.to_owned();
            let purpose = purpose_in(purpose);
            self.write(actor, move |tx, repos| {
                let hash = repos
                    .email_codes
                    .peek_live(tx.conn(), &id, purpose, &now, max_attempts)?
                    .map(|c| c.code_hash);
                let tries = repos.email_codes.reserve_try(tx, &id, purpose, &now, max_attempts)?;
                // The try that uses the code up removes it, while it still holds that hash
                if let (Some(n), Some(hash)) = (tries, &hash)
                    && n >= max_attempts
                {
                    repos.email_codes.remove(tx, &id, purpose, hash)?;
                }
                Ok(tries)
            })
            .await
        })
    }

    fn gameserver_key_hashes(&self) -> BoxFuture<'_, StoreResult<Vec<String>>> {
        Box::pin(async move {
            self.read(|conn, repos| repos.api_keys.find_all_game_server_key_hashes(conn))
                .await
                .map(|v| v.into_iter().filter_map(|k| k.key_hash).collect())
        })
    }

    fn replace_gameserver_keys<'a>(
        &'a self,
        actor: &'a ActorContext,
        hashes: &'a [String],
    ) -> BoxFuture<'a, StoreResult<()>> {
        let hashes = hashes.to_vec();
        Box::pin(async move {
            self.write(actor, move |tx, repos| {
                repos.api_keys.replace_game_server_key_hashes(tx, &hashes)
            })
            .await
        })
    }

    fn user_key_hashes(&self) -> BoxFuture<'_, StoreResult<Vec<(String, String)>>> {
        Box::pin(async move {
            self.read(|conn, repos| repos.api_keys.find_all_user_key_hashes(conn))
                .await
                .map(|v| v.into_iter().map(|k| (k.key_hash, k.account.to_string())).collect())
        })
    }

    fn health(&self) -> BoxFuture<'_, StoreResult<()>> {
        Box::pin(async move {
            self.read(|conn, _| {
                crate::sql::sql("SELECT CAST(1 AS BIGINT) AS n").int(conn)?;
                Ok(())
            })
            .await
        })
    }
}
