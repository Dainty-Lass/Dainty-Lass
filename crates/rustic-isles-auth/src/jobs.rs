// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Start-up and background work (spec §8): the game server key sync, the unverified-registration sweep and the
//! 30-day erasure.

use std::time::Duration;

use tokio::task::JoinHandle;

use crate::errors::LauncherError;
use crate::store::{ErasureCounts, LauncherAccount, StoreError};
use crate::{Auth, AuthStore, hashes, policy, tokens};

const SWEEP_LIMIT: usize = 500;
const ERASURES_PER_RUN: usize = 50;
const HOURLY: Duration = Duration::from_secs(3600);

/// Why the auth jobs could not start.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StartError {
    /// `API_KEY_HASH_SECRET` is unset or too short.
    #[error("API_KEY_HASH_SECRET must contain at least 32 characters.")]
    Secret,
    /// The token signing keys could not be loaded.
    #[error("{0}")]
    Keys(tokens::TokenError),
    /// Email verification is required but the SMTP settings are not usable; holds the keys at fault.
    #[error(
        "RUSTIC_ISLES_EMAIL_VERIFICATION=required needs working SMTP settings ({0} not set or invalid; the required keys \
         are RUSTIC_ISLES_SMTP_HOST, RUSTIC_ISLES_SMTP_USER and RUSTIC_ISLES_SMTP_PASSWORD)"
    )]
    Smtp(String),
    /// The game server keys could not be synchronized with the store.
    #[error("the game server keys could not be synchronized: {0}")]
    Store(StoreError),
}

/// What erasing one due account did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErasureOutcome {
    /// Live activity keeps it for the next run
    Blocked(Vec<String>),
    /// Nothing blocked it, but it was a dry run.
    DryRun,
    /// The account was erased; the rows deleted, by kind.
    Erased(ErasureCounts),
}

impl<S: AuthStore> Auth<S> {
    /// §8.3; the handles are the jobs isles aborts at stop.
    pub async fn start(self: &std::sync::Arc<Self>) -> Result<Vec<JoinHandle<()>>, StartError> {
        if self.store.is_none() {
            return Ok(Vec::new());
        }
        self.secret().map_err(|_| StartError::Secret)?;
        self.token_keys().map_err(StartError::Keys)?;
        let (verification, unrecognised) = policy::email_verification(self.raw("RUSTIC_ISLES_EMAIL_VERIFICATION"));
        if unrecognised {
            tracing::warn!("RUSTIC_ISLES_EMAIL_VERIFICATION is not off or required; treating it as required");
        }
        tracing::info!("email verification: {}", if verification { "required" } else { "off" });
        if verification && let Err(problems) = self.smtp() {
            return Err(StartError::Smtp(problems.join(", ")));
        }
        let (approval, unrecognised) = policy::registration_approval(self.raw("RUSTIC_ISLES_REGISTRATION_APPROVAL"));
        if let Some(value) = unrecognised {
            tracing::warn!(
                "RUSTIC_ISLES_REGISTRATION_APPROVAL is not off or required (got \"{value}\"); treating it as required"
            );
        }
        if !dainty_service_kit::js::trim(self.raw("RUSTIC_ISLES_REGISTRATION_AUTO_APPROVE").unwrap_or("")).is_empty() {
            tracing::warn!(
                "RUSTIC_ISLES_REGISTRATION_AUTO_APPROVE has no effect; use RUSTIC_ISLES_REGISTRATION_APPROVAL"
            );
        }
        tracing::info!("registration approval: {}", if approval { "required" } else { "off" });
        self.synchronize_gameserver_keys(self.raw("GAMESERVER_API_KEYS"))
            .await
            .map_err(StartError::Store)?;
        let mut handles = Vec::new();
        if verification {
            let this = std::sync::Arc::clone(self);
            handles.push(tokio::spawn(async move {
                let mut ticks = tokio::time::interval(HOURLY);
                ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    ticks.tick().await;
                    if let Err(error) = this.sweep_unverified_registrations(this.now(), SWEEP_LIMIT).await {
                        tracing::warn!("[EmailSweep] failed: {error}");
                    }
                }
            }));
        }
        if policy::is_on(self.raw("RUSTIC_ISLES_ACCOUNT_ERASURE")) {
            let this = std::sync::Arc::clone(self);
            handles.push(tokio::spawn(async move {
                let mut ticks = tokio::time::interval(HOURLY);
                ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    ticks.tick().await;
                    this.run_erasures(&this.now_iso()).await;
                }
            }));
        }
        Ok(handles)
    }

    /// `GAMESERVER_API_KEYS`: a non-empty list is the complete set; empty keeps what is stored. Never logs a key.
    pub async fn synchronize_gameserver_keys(&self, value: Option<&str>) -> Result<usize, StoreError> {
        let keys = hashes::parse_gameserver_keys(value);
        if keys.is_empty() {
            tracing::info!("Registered 0 new Gameserver API Key(s) on boot!");
            return Ok(0);
        }
        let secret = self.secret().map_err(|e| StoreError::new(e.to_string()))?;
        let hashes: Vec<String> = keys.iter().map(|k| hashes::gameserver_key_hash(secret, k)).collect();
        self.store()?
            .replace_gameserver_keys(&self.system_actor(), &hashes)
            .await?;
        tracing::info!(
            "Synchronized {} Gameserver API key hash(es) from GAMESERVER_API_KEYS",
            hashes.len()
        );
        Ok(hashes.len())
    }

    /// §8.1: removes registrations still unverified 48 hours on.
    pub async fn sweep_unverified_registrations(&self, now: i64, limit: usize) -> Result<usize, StoreError> {
        let store = self.store()?;
        let stale = store.stale_unverified(&self.unverified_cutoff(now), limit).await?;
        let mut removed = 0;
        for account in stale {
            if store
                .delete_unverified_registration(&self.system_actor(), &account.account_id)
                .await?
            {
                removed += 1;
            }
        }
        if removed > 0 {
            tracing::info!("[EmailSweep] removed {removed} unverified registration(s) older than 48 h");
        }
        Ok(removed)
    }

    /// Live accounts whose deletion hold has ended at `now_iso`, oldest first, at most `limit`.
    pub async fn due_deletions(&self, now_iso: &str, limit: usize) -> Result<Vec<LauncherAccount>, StoreError> {
        self.store()?.due_deletions(now_iso, limit).await
    }

    /// §8.2: erases one due account (`apply` false is a dry run).
    pub async fn erase_account(&self, account: &LauncherAccount, apply: bool) -> Result<ErasureOutcome, LauncherError> {
        let Some(deletion) = &account.deletion else {
            return Err(LauncherError::internal());
        };
        let due = dainty_service_kit::js::date_parse(&deletion.scheduled_for).is_some_and(|at| at <= self.now());
        if !due {
            return Err(LauncherError::new(
                "DELETION_HOLD_EXPIRED",
                "The hold has not ended yet.",
            ));
        }
        let blockers = self.seams.erasure.blockers(account).await.map_err(|error| {
            tracing::error!("erasure blockers unavailable: {error}");
            LauncherError::internal()
        })?;
        if !blockers.is_empty() {
            return Ok(ErasureOutcome::Blocked(blockers));
        }
        if !apply {
            return Ok(ErasureOutcome::DryRun);
        }
        let actor = self.player_actor(&account.account_id)?;
        let counts = self.store()?.erase_account(&actor, &account.account_id).await?;
        if let Err(error) = self.seams.erasure.remove_uploaded_logs(account).await {
            tracing::warn!("uploaded logs of an erased account were not removed: {error}");
        }
        tracing::info!("Account {} erased", account.user_id);
        Ok(ErasureOutcome::Erased(counts))
    }

    /// One run of the erasure job: up to 50 due accounts; how many were erased.
    pub async fn run_erasures(&self, now_iso: &str) -> usize {
        let due = match self.due_deletions(now_iso, ERASURES_PER_RUN).await {
            Ok(due) => due,
            Err(error) => {
                tracing::warn!("erasure job could not list due accounts: {error}");
                return 0;
            }
        };
        let mut erased = 0;
        for account in due {
            match self.erase_account(&account, true).await {
                Ok(ErasureOutcome::Erased(_)) => erased += 1,
                Ok(ErasureOutcome::Blocked(reasons)) => {
                    tracing::info!("erasure of {} waits: {}", account.user_id, reasons.join(", "));
                }
                Ok(ErasureOutcome::DryRun) => {}
                Err(error) => tracing::warn!("erasure of {} failed: {}", account.user_id, error.code),
            }
        }
        erased
    }

    /// The erasure pseudonym of an account id under this module's secret.
    pub fn pseudonym(&self, account_id: &str) -> Result<String, hashes::SecretError> {
        Ok(hashes::pseudonymous_account_id(self.secret()?, account_id))
    }
}
