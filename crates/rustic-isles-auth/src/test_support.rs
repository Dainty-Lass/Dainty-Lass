// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Test doubles (feature `test-support`): an in-memory store with §9.3's atomic rules, a recording mail transport,
//! a stub Discord, settings from a map, and a throwaway signing key pair.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test fixtures; a panic is how a test fails"
)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError, RwLock};

use aws_lc_rs::encoding::AsDer as _;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use dainty_module_api::{ActorContext, BoxFuture};

use crate::AuthSettings;
use crate::discord::{DiscordApi, DiscordConfig, DiscordFailure, DiscordUser};
use crate::mail::{MailError, MailMessage, MailTransport, SmtpConfig};
use crate::store::*;

/// Settings from a map; values set later are leaked so `raw` can lend them (tests only).
#[derive(Default)]
pub struct MapSettings {
    values: RwLock<HashMap<String, &'static str>>,
}

impl MapSettings {
    /// Settings holding `pairs`.
    pub fn new(pairs: &[(&str, &str)]) -> Self {
        let settings = Self::default();
        for (k, v) in pairs {
            settings.set(k, v);
        }
        settings
    }

    /// Sets one setting, replacing its value.
    pub fn set(&self, name: &str, value: &str) {
        let leaked: &'static str = Box::leak(value.to_owned().into_boxed_str());
        self.values
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.to_owned(), leaked);
    }

    /// Removes one setting.
    pub fn unset(&self, name: &str) {
        self.values.write().unwrap_or_else(PoisonError::into_inner).remove(name);
    }
}

impl AuthSettings for MapSettings {
    fn raw(&self, name: &str) -> Option<&str> {
        self.values
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .copied()
    }
}

/// `(AUTH_SIGNING_PRIVKEY_B64, AUTH_SIGNING_PUBKEY_B64)` of an RSA-2048 pair made once per process.
pub fn throwaway_signing_keys() -> (String, String) {
    static KEYS: OnceLock<(String, String)> = OnceLock::new();
    KEYS.get_or_init(|| {
        let pair = aws_lc_rs::rsa::KeyPair::generate(aws_lc_rs::rsa::KeySize::Rsa2048).unwrap();
        let private: aws_lc_rs::encoding::Pkcs8V1Der<'static> = pair.as_der().unwrap();
        let public = aws_lc_rs::signature::KeyPair::public_key(&pair).as_der().unwrap();
        let pem = |label: &str, der: &[u8]| {
            let body = STANDARD.encode(der);
            let lines: Vec<&str> = body
                .as_bytes()
                .chunks(64)
                .map(|c| std::str::from_utf8(c).unwrap())
                .collect();
            STANDARD.encode(format!(
                "-----BEGIN {label}-----\n{}\n-----END {label}-----\n",
                lines.join("\n")
            ))
        };
        (pem("PRIVATE KEY", private.as_ref()), pem("PUBLIC KEY", public.as_ref()))
    })
    .clone()
}

/// Keeps every message; fails on demand.
#[derive(Default)]
pub struct RecordingMail {
    sent: Mutex<Vec<MailMessage>>,
    fail: AtomicBool,
}

impl RecordingMail {
    /// Makes every later send fail (`true`) or succeed.
    pub fn fail(&self, fail: bool) {
        self.fail.store(fail, Ordering::SeqCst);
    }

    /// Every message sent so far, oldest first.
    pub fn messages(&self) -> Vec<MailMessage> {
        self.sent.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// The six-digit code in the last message to `to`.
    pub fn last_code(&self, to: &str) -> Option<String> {
        let messages = self.messages();
        let message = messages.iter().rev().find(|m| m.to == to)?;
        message
            .subject
            .rsplit(": ")
            .next()
            .filter(|c| crate::email::is_code_shape(c))
            .map(str::to_owned)
    }

    /// Waits up to two seconds for `count` messages (background mail).
    pub async fn wait_for(&self, count: usize) -> bool {
        for _ in 0..200 {
            if self.messages().len() >= count {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        false
    }
}

impl MailTransport for RecordingMail {
    fn send<'a>(&'a self, _settings: &'a SmtpConfig, message: MailMessage) -> BoxFuture<'a, Result<(), MailError>> {
        Box::pin(async move {
            if self.fail.load(Ordering::SeqCst) {
                return Err(MailError::new("ECONNECTION"));
            }
            self.sent.lock().unwrap_or_else(PoisonError::into_inner).push(message);
            Ok(())
        })
    }
}

/// `code-<id>` is Discord user `<id>`, username `user-<last 4 of id>`; anything else fails.
#[derive(Debug, Default)]
pub struct StubDiscord;

impl DiscordApi for StubDiscord {
    fn exchange_code<'a>(
        &'a self,
        _config: &'a DiscordConfig,
        code: &'a str,
        _verifier: &'a str,
    ) -> BoxFuture<'a, Result<DiscordUser, DiscordFailure>> {
        Box::pin(async move {
            let id = code
                .strip_prefix("code-")
                .filter(|id| !id.is_empty())
                .ok_or(DiscordFailure { status: Some(400) })?;
            let tail: String = id.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
            Ok(DiscordUser {
                id: id.to_owned(),
                username: format!("user-{tail}"),
                global_name: None,
                avatar: None,
            })
        })
    }
}

#[derive(Default)]
struct Tables {
    games: Vec<(GameAccount, bool)>,
    launchers: Vec<(LauncherAccount, bool)>,
    identities: Vec<Identity>,
    sessions: Vec<RefreshSession>,
    exchanges: Vec<ExchangeCode>,
    transactions: Vec<DiscordTransaction>,
    codes: Vec<EmailCode>,
    gameserver_keys: Vec<String>,
    user_keys: Vec<(String, String)>,
}

impl Tables {
    fn launcher(&self, pred: impl Fn(&LauncherAccount) -> bool) -> Option<LauncherAccount> {
        self.launchers
            .iter()
            .find(|(a, deleted)| !deleted && pred(a))
            .map(|(a, _)| a.clone())
    }

    fn launcher_mut(&mut self, account_id: &str) -> Option<&mut LauncherAccount> {
        self.launchers
            .iter_mut()
            .find(|(a, deleted)| !deleted && a.account_id == account_id)
            .map(|(a, _)| a)
    }

    fn game_mut(&mut self, account_id: &str) -> Option<&mut GameAccount> {
        self.games
            .iter_mut()
            .find(|(g, deleted)| !deleted && g.id == account_id)
            .map(|(g, _)| g)
    }

    fn soft_delete(&mut self, account_id: &str) -> bool {
        let mut found = false;
        for (account, deleted) in &mut self.launchers {
            if account.account_id == account_id && !*deleted {
                *deleted = true;
                found = true;
            }
        }
        for (game, deleted) in &mut self.games {
            if game.id == account_id {
                *deleted = true;
            }
        }
        found
    }
}

/// An [`AuthStore`] in memory, one lock per operation so each is atomic.
#[derive(Default)]
pub struct MemoryStore {
    tables: Mutex<Tables>,
    offline: AtomicBool,
}

impl MemoryStore {
    /// An empty, online store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every operation fails while offline.
    pub fn set_offline(&self, offline: bool) {
        self.offline.store(offline, Ordering::SeqCst);
    }

    /// A user key row as the Reforged API writes it.
    pub fn insert_user_key(&self, account_id: &str, key_hash: &str) {
        let mut t = self.lock();
        t.user_keys.retain(|(_, a)| a != account_id);
        t.user_keys.push((key_hash.to_owned(), account_id.to_owned()));
    }

    /// An admin write to an account's row.
    pub fn update_account(&self, user_id: &str, change: impl FnOnce(&mut LauncherAccount)) {
        let mut t = self.lock();
        if let Some((account, _)) = t.launchers.iter_mut().find(|(a, d)| !d && a.user_id == user_id) {
            change(account);
        }
    }

    /// Every stored email code.
    pub fn email_codes(&self) -> Vec<EmailCode> {
        self.lock().codes.clone()
    }

    /// Every stored refresh session.
    pub fn sessions(&self) -> Vec<RefreshSession> {
        self.lock().sessions.clone()
    }

    /// The stored game server key hashes.
    pub fn gameserver_hashes(&self) -> Vec<String> {
        self.lock().gameserver_keys.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Tables> {
        self.tables.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn run<'a, T: Send + 'a>(&'a self, f: impl FnOnce(&mut Tables) -> T + Send + 'a) -> BoxFuture<'a, StoreResult<T>> {
        Box::pin(async move {
            if self.offline.load(Ordering::SeqCst) {
                return Err(StoreError::new("the store is offline"));
            }
            Ok(f(&mut self.lock()))
        })
    }
}

fn live_code(code: &EmailCode, now: &str, max: u32) -> bool {
    code.expires_at.as_str() > now && code.attempts < max
}

impl AuthStore for MemoryStore {
    fn game_account_by_user_id<'a>(&'a self, user_id: &'a str) -> BoxFuture<'a, StoreResult<Option<GameAccount>>> {
        self.run(move |t| {
            t.games
                .iter()
                .find(|(g, d)| !d && g.user_id == user_id)
                .map(|(g, _)| g.clone())
        })
    }

    fn game_account_by_id<'a>(&'a self, account_id: &'a str) -> BoxFuture<'a, StoreResult<Option<GameAccount>>> {
        self.run(move |t| {
            t.games
                .iter()
                .find(|(g, d)| !d && g.id == account_id)
                .map(|(g, _)| g.clone())
        })
    }

    fn ensure_game_account<'a>(
        &'a self,
        _: &'a ActorContext,
        account: &'a GameAccount,
    ) -> BoxFuture<'a, StoreResult<bool>> {
        self.run(move |t| {
            if t.games.iter().any(|(g, d)| !d && g.user_id == account.user_id) {
                return false;
            }
            t.games.push((account.clone(), false));
            true
        })
    }

    fn launcher_account_by_user_id<'a>(
        &'a self,
        user_id: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<LauncherAccount>>> {
        self.run(move |t| t.launcher(|a| a.user_id == user_id))
    }

    fn launcher_account_by_id<'a>(
        &'a self,
        account_id: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<LauncherAccount>>> {
        self.run(move |t| t.launcher(|a| a.account_id == account_id))
    }

    fn launcher_account_by_email<'a>(&'a self, email: &'a str) -> BoxFuture<'a, StoreResult<Option<LauncherAccount>>> {
        self.run(move |t| t.launcher(|a| a.email.as_deref() == Some(email)))
    }

    fn launcher_account_by_name<'a>(&'a self, name: &'a str) -> BoxFuture<'a, StoreResult<Option<LauncherAccount>>> {
        self.run(move |t| t.launcher(|a| a.display_name_normalized == name))
    }

    fn create_launcher_account<'a>(
        &'a self,
        _: &'a ActorContext,
        new: &'a NewLauncherAccount,
    ) -> BoxFuture<'a, StoreResult<CreateOutcome>> {
        self.run(move |t| {
            let account = &new.account;
            if account.email.is_some() && t.launcher(|a| a.email == account.email).is_some() {
                return CreateOutcome::EmailTaken;
            }
            if t.launcher(|a| a.display_name_normalized == account.display_name_normalized)
                .is_some()
            {
                return CreateOutcome::NameTaken;
            }
            if !t.games.iter().any(|(g, d)| !d && g.user_id == account.user_id) {
                t.games.push((
                    GameAccount {
                        id: account.account_id.clone(),
                        user_id: account.user_id.clone(),
                        name: new.game_name.clone(),
                        notes: 0,
                    },
                    false,
                ));
            }
            t.launchers.push((account.clone(), false));
            CreateOutcome::Created(account.clone())
        })
    }

    fn set_last_login<'a>(
        &'a self,
        _: &'a ActorContext,
        account_id: &'a str,
        at: &'a str,
    ) -> BoxFuture<'a, StoreResult<()>> {
        self.run(move |t| {
            if let Some(a) = t.launcher_mut(account_id) {
                a.last_login_at = Some(at.to_owned());
            }
        })
    }

    fn set_username<'a>(
        &'a self,
        _: &'a ActorContext,
        account_id: &'a str,
        display_name: &'a str,
        normalized: &'a str,
    ) -> BoxFuture<'a, StoreResult<bool>> {
        self.run(move |t| {
            if t.launcher(|a| a.display_name_normalized == normalized && a.account_id != account_id)
                .is_some()
            {
                return false;
            }
            if let Some(a) = t.launcher_mut(account_id) {
                a.display_name = display_name.to_owned();
                a.display_name_normalized = normalized.to_owned();
                a.username_set = Some(true);
            }
            if let Some(g) = t.game_mut(account_id) {
                g.name = display_name.to_owned();
            }
            true
        })
    }

    fn set_consent<'a>(
        &'a self,
        _: &'a ActorContext,
        account_id: &'a str,
        consent: &'a Consent,
    ) -> BoxFuture<'a, StoreResult<()>> {
        self.run(move |t| {
            if let Some(a) = t.launcher_mut(account_id) {
                a.consent = Some(consent.clone());
            }
        })
    }

    fn set_deletion<'a>(
        &'a self,
        _: &'a ActorContext,
        account_id: &'a str,
        deletion: Option<&'a Deletion>,
    ) -> BoxFuture<'a, StoreResult<()>> {
        self.run(move |t| {
            if let Some(a) = t.launcher_mut(account_id) {
                a.deletion = deletion.cloned();
            }
        })
    }

    fn set_email_verified<'a>(
        &'a self,
        _: &'a ActorContext,
        account_id: &'a str,
        at: &'a str,
    ) -> BoxFuture<'a, StoreResult<()>> {
        self.run(move |t| {
            if let Some(a) = t.launcher_mut(account_id) {
                a.email_verified_at = Some(at.to_owned());
                a.email_verification_pending_since = None;
            }
        })
    }

    fn reset_password<'a>(
        &'a self,
        _: &'a ActorContext,
        account_id: &'a str,
        password_hash: &'a str,
        at: &'a str,
    ) -> BoxFuture<'a, StoreResult<()>> {
        self.run(move |t| {
            if let Some(a) = t.launcher_mut(account_id) {
                a.password_hash = Some(password_hash.to_owned());
                a.email_verified_at = Some(at.to_owned());
                a.email_verification_pending_since = None;
            }
            for s in t
                .sessions
                .iter_mut()
                .filter(|s| s.account_id == account_id && s.revoked_at.is_none())
            {
                s.revoked_at = Some(at.to_owned());
            }
        })
    }

    fn due_deletions<'a>(&'a self, now: &'a str, limit: usize) -> BoxFuture<'a, StoreResult<Vec<LauncherAccount>>> {
        self.run(move |t| {
            let mut due: Vec<LauncherAccount> = t
                .launchers
                .iter()
                .filter(|(a, d)| !d && a.deletion.as_ref().is_some_and(|x| x.scheduled_for.as_str() <= now))
                .map(|(a, _)| a.clone())
                .collect();
            due.sort_by(|a, b| {
                let key = |x: &LauncherAccount| x.deletion.as_ref().map(|d| d.scheduled_for.clone());
                key(a).cmp(&key(b))
            });
            if limit > 0 {
                due.truncate(limit);
            }
            due
        })
    }

    fn stale_unverified<'a>(
        &'a self,
        before: &'a str,
        limit: usize,
    ) -> BoxFuture<'a, StoreResult<Vec<LauncherAccount>>> {
        self.run(move |t| {
            let mut stale: Vec<LauncherAccount> = t
                .launchers
                .iter()
                .filter(|(a, d)| {
                    !d && a.email_verified_at.is_none()
                        && a.email_verification_pending_since
                            .as_deref()
                            .is_some_and(|p| p < before)
                })
                .map(|(a, _)| a.clone())
                .collect();
            stale.sort_by(|a, b| {
                a.email_verification_pending_since
                    .cmp(&b.email_verification_pending_since)
            });
            if limit > 0 {
                stale.truncate(limit);
            }
            stale
        })
    }

    fn delete_unverified_registration<'a>(
        &'a self,
        _: &'a ActorContext,
        account_id: &'a str,
    ) -> BoxFuture<'a, StoreResult<bool>> {
        self.run(move |t| {
            let still = t
                .launcher(|a| a.account_id == account_id)
                .is_some_and(|a| a.email_verified_at.is_none() && a.email_verification_pending_since.is_some());
            if !still {
                return false;
            }
            t.soft_delete(account_id);
            t.codes.retain(|c| c.account_id != account_id);
            t.sessions.retain(|s| s.account_id != account_id);
            true
        })
    }

    fn erase_account<'a>(
        &'a self,
        _: &'a ActorContext,
        account_id: &'a str,
    ) -> BoxFuture<'a, StoreResult<ErasureCounts>> {
        self.run(move |t| {
            let count = |before: usize, after: usize| (before - after) as u64;
            let mut counts = ErasureCounts::default();
            t.sessions.retain(|s| s.account_id != account_id);
            let n = t.codes.len();
            t.codes.retain(|c| c.account_id != account_id);
            counts.email_codes = count(n, t.codes.len());
            let n = t.exchanges.len();
            t.exchanges.retain(|e| e.account_id != account_id);
            counts.game_exchange_codes = count(n, t.exchanges.len());
            let n = t.transactions.len();
            let mine = |x: &Option<String>| x.as_deref() == Some(account_id);
            t.transactions
                .retain(|x| !mine(&x.account_id) && !mine(&x.link_account_id));
            counts.discord_oauth_transactions = count(n, t.transactions.len());
            let n = t.identities.len();
            t.identities.retain(|i| i.account_id != account_id);
            counts.auth_identities = count(n, t.identities.len());
            let n = t.user_keys.len();
            t.user_keys.retain(|(_, a)| a != account_id);
            counts.user_api_keys = count(n, t.user_keys.len());
            counts.accounts = u64::from(t.soft_delete(account_id));
            counts
        })
    }

    fn identity_by_subject<'a>(&'a self, subject: &'a str) -> BoxFuture<'a, StoreResult<Option<Identity>>> {
        self.run(move |t| t.identities.iter().find(|i| i.provider_subject == subject).cloned())
    }

    fn identity_by_account<'a>(&'a self, account_id: &'a str) -> BoxFuture<'a, StoreResult<Option<Identity>>> {
        self.run(move |t| t.identities.iter().find(|i| i.account_id == account_id).cloned())
    }

    fn create_identity<'a>(&'a self, _: &'a ActorContext, identity: &'a Identity) -> BoxFuture<'a, StoreResult<bool>> {
        self.run(move |t| {
            let taken = t
                .identities
                .iter()
                .any(|i| i.provider_subject == identity.provider_subject || i.account_id == identity.account_id);
            if !taken {
                t.identities.push(identity.clone());
            }
            !taken
        })
    }

    fn delete_identities_for_account<'a>(
        &'a self,
        _: &'a ActorContext,
        account_id: &'a str,
    ) -> BoxFuture<'a, StoreResult<u64>> {
        self.run(move |t| {
            let n = t.identities.len();
            t.identities.retain(|i| i.account_id != account_id);
            (n - t.identities.len()) as u64
        })
    }

    fn create_session<'a>(
        &'a self,
        _: &'a ActorContext,
        session: &'a RefreshSession,
    ) -> BoxFuture<'a, StoreResult<()>> {
        self.run(move |t| t.sessions.push(session.clone()))
    }

    fn session_by_token_hash<'a>(&'a self, token_hash: &'a str) -> BoxFuture<'a, StoreResult<Option<RefreshSession>>> {
        self.run(move |t| t.sessions.iter().find(|s| s.token_hash == token_hash).cloned())
    }

    fn rotate_session<'a>(
        &'a self,
        _: &'a ActorContext,
        revoke_id: &'a str,
        at: &'a str,
        next: &'a RefreshSession,
    ) -> BoxFuture<'a, StoreResult<()>> {
        self.run(move |t| {
            if let Some(s) = t.sessions.iter_mut().find(|s| s.id == revoke_id) {
                s.revoked_at = Some(at.to_owned());
            }
            t.sessions.push(next.clone());
        })
    }

    fn revoke_family<'a>(
        &'a self,
        _: &'a ActorContext,
        family_id: &'a str,
        at: &'a str,
    ) -> BoxFuture<'a, StoreResult<u64>> {
        self.run(move |t| {
            let mut n = 0;
            for s in t
                .sessions
                .iter_mut()
                .filter(|s| s.family_id == family_id && s.revoked_at.is_none())
            {
                s.revoked_at = Some(at.to_owned());
                n += 1;
            }
            n
        })
    }

    fn revoke_all_sessions<'a>(
        &'a self,
        _: &'a ActorContext,
        account_id: &'a str,
        at: &'a str,
    ) -> BoxFuture<'a, StoreResult<u64>> {
        self.run(move |t| {
            let mut n = 0;
            for s in t
                .sessions
                .iter_mut()
                .filter(|s| s.account_id == account_id && s.revoked_at.is_none())
            {
                s.revoked_at = Some(at.to_owned());
                n += 1;
            }
            n
        })
    }

    fn is_family_active<'a>(
        &'a self,
        family_id: &'a str,
        account_id: &'a str,
        now: &'a str,
    ) -> BoxFuture<'a, StoreResult<bool>> {
        self.run(move |t| {
            t.sessions.iter().any(|s| {
                s.family_id == family_id
                    && s.account_id == account_id
                    && s.revoked_at.is_none()
                    && s.expires_at.as_str() > now
            })
        })
    }

    fn create_exchange_code<'a>(
        &'a self,
        _: &'a ActorContext,
        code: &'a ExchangeCode,
    ) -> BoxFuture<'a, StoreResult<()>> {
        self.run(move |t| t.exchanges.push(code.clone()))
    }

    fn consume_exchange_code<'a>(
        &'a self,
        _: &'a ActorContext,
        code_hash: &'a str,
        now: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<ExchangeCode>>> {
        self.run(move |t| {
            let code = t
                .exchanges
                .iter_mut()
                .find(|c| c.code_hash == code_hash && c.consumed_at.is_none() && c.expires_at.as_str() > now)?;
            code.consumed_at = Some(now.to_owned());
            Some(code.clone())
        })
    }

    fn create_discord_transaction<'a>(
        &'a self,
        _: &'a ActorContext,
        transaction: &'a DiscordTransaction,
    ) -> BoxFuture<'a, StoreResult<()>> {
        self.run(move |t| t.transactions.push(transaction.clone()))
    }

    fn consume_discord_state<'a>(
        &'a self,
        _: &'a ActorContext,
        state: &'a str,
        now: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<DiscordTransaction>>> {
        self.run(move |t| {
            let x = t
                .transactions
                .iter_mut()
                .find(|x| x.state == state && x.consumed_at.is_none() && x.expires_at.as_str() > now)?;
            x.consumed_at = Some(now.to_owned());
            Some(x.clone())
        })
    }

    fn attach_discord_completion<'a>(
        &'a self,
        _: &'a ActorContext,
        state: &'a str,
        completion_hash: &'a str,
        expires_at: &'a str,
        account_id: &'a str,
    ) -> BoxFuture<'a, StoreResult<()>> {
        self.run(move |t| {
            if let Some(x) = t.transactions.iter_mut().find(|x| x.state == state) {
                x.completion_code_hash = Some(completion_hash.to_owned());
                x.completion_expires_at = Some(expires_at.to_owned());
                x.account_id = Some(account_id.to_owned());
            }
        })
    }

    fn consume_discord_completion<'a>(
        &'a self,
        _: &'a ActorContext,
        completion_hash: &'a str,
        now: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<DiscordTransaction>>> {
        self.run(move |t| {
            let x = t.transactions.iter_mut().find(|x| {
                x.completion_code_hash.as_deref() == Some(completion_hash)
                    && x.completion_consumed_at.is_none()
                    && x.completion_expires_at.as_deref().is_some_and(|e| e > now)
            })?;
            x.completion_consumed_at = Some(now.to_owned());
            Some(x.clone())
        })
    }

    fn replace_email_code<'a>(&'a self, _: &'a ActorContext, code: &'a EmailCode) -> BoxFuture<'a, StoreResult<()>> {
        self.run(move |t| {
            t.codes
                .retain(|c| !(c.account_id == code.account_id && c.purpose == code.purpose));
            t.codes.push(code.clone());
        })
    }

    fn live_email_code<'a>(
        &'a self,
        account_id: &'a str,
        purpose: Purpose,
        now: &'a str,
    ) -> BoxFuture<'a, StoreResult<Option<EmailCode>>> {
        self.run(move |t| {
            t.codes
                .iter()
                .find(|c| {
                    c.account_id == account_id
                        && c.purpose == purpose
                        && live_code(c, now, crate::email::CODE_MAX_ATTEMPTS)
                })
                .cloned()
        })
    }

    fn consume_email_code<'a>(
        &'a self,
        _: &'a ActorContext,
        account_id: &'a str,
        purpose: Purpose,
        code_hash: &'a str,
        now: &'a str,
    ) -> BoxFuture<'a, StoreResult<bool>> {
        self.run(move |t| {
            let before = t.codes.len();
            t.codes.retain(|c| {
                !(c.account_id == account_id
                    && c.purpose == purpose
                    && c.code_hash == code_hash
                    && live_code(c, now, crate::email::CODE_MAX_ATTEMPTS))
            });
            t.codes.len() < before
        })
    }

    fn count_email_code_try<'a>(
        &'a self,
        _: &'a ActorContext,
        account_id: &'a str,
        purpose: Purpose,
        now: &'a str,
        max_attempts: u32,
    ) -> BoxFuture<'a, StoreResult<Option<u32>>> {
        self.run(move |t| {
            let index = t
                .codes
                .iter()
                .position(|c| c.account_id == account_id && c.purpose == purpose)?;
            if !live_code(&t.codes[index], now, max_attempts) {
                t.codes.remove(index);
                return None;
            }
            t.codes[index].attempts += 1;
            let attempts = t.codes[index].attempts;
            if attempts >= max_attempts {
                t.codes.remove(index);
            }
            Some(attempts)
        })
    }

    fn gameserver_key_hashes(&self) -> BoxFuture<'_, StoreResult<Vec<String>>> {
        self.run(|t| t.gameserver_keys.clone())
    }

    fn replace_gameserver_keys<'a>(
        &'a self,
        _: &'a ActorContext,
        hashes: &'a [String],
    ) -> BoxFuture<'a, StoreResult<()>> {
        self.run(move |t| t.gameserver_keys = hashes.to_vec())
    }

    fn user_key_hashes(&self) -> BoxFuture<'_, StoreResult<Vec<(String, String)>>> {
        self.run(|t| t.user_keys.clone())
    }

    fn health(&self) -> BoxFuture<'_, StoreResult<()>> {
        self.run(|_| ())
    }
}
