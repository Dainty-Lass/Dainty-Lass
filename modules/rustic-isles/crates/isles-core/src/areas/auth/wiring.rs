//! What this area gives the Reforged API at start: the launcher authentication, the admin sign-in's password and TOTP
//! checks, and the two periodic jobs the Privacy Policy owes (the 30-day erasure and the 90-day log purge), each off
//! unless the host turns it on.

use std::sync::Arc;
use std::time::Duration;

use module_api::BoxFuture;
use serde_json::Value;

use super::errors::{LauncherApiError, status_for};
use super::{crypto, erasure, middleware, totp, util};
use crate::areas::reforged_api::admin::{self, AdminCredentials};
use crate::areas::reforged_api::launcher_auth::{self, LauncherAuthenticator, LauncherCaller, LauncherFailure};
use crate::areas::reforged_api::player_logs;
use crate::http::Shared;
use crate::settings::Settings;

/// `HasLauncherAuth` for the Reforged API's launcher routes.
#[derive(Debug, Clone, Copy, Default)]
pub struct LauncherSessions;

fn failure(error: LauncherApiError) -> LauncherFailure {
    let status = error.status.unwrap_or_else(|| status_for(&error.code));
    LauncherFailure::new(status, &error.code, &error.message)
}

impl LauncherAuthenticator for LauncherSessions {
    fn authenticate<'a>(
        &'a self,
        shared: &'a Shared,
        token: &'a str,
    ) -> BoxFuture<'a, Result<LauncherCaller, LauncherFailure>> {
        Box::pin(async move {
            let header = format!("Bearer {token}");
            let (auth, record) = middleware::launcher_auth_account(shared, Some(&header))
                .await
                .map_err(failure)?;
            Ok(LauncherCaller {
                account: auth.account,
                user_id: auth.user_id,
                session: auth.sid,
                record,
            })
        })
    }
}

/// `VerifyPassword` and `VerifyTotp` for the admin sign-in.
#[derive(Debug, Clone, Copy, Default)]
pub struct AdminSignIn;

impl AdminCredentials for AdminSignIn {
    fn verify_password(&self, hash: Option<&str>, password: &str) -> bool {
        crypto::verify_password(hash, password)
    }

    fn verify_totp(&self, secret: &str, code: Option<&Value>) -> bool {
        totp::verify_totp(secret, code)
    }
}

/// Registers both with the Reforged API (process-wide, as its registries are).
pub fn register_seams() {
    launcher_auth::set_authenticator(Some(Arc::new(LauncherSessions)));
    admin::set_credentials(Some(Arc::new(AdminSignIn)));
}

const JOB_INTERVAL: Duration = Duration::from_secs(3600);
const ERASURE_BATCH: u32 = 50;

fn switched_on(settings: &Settings, name: &str) -> bool {
    matches!(
        service_kit::js::trim(settings.raw(name).unwrap_or_default())
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "on" | "yes"
    )
}

/// One run of the erasure: every account whose hold has ended, oldest first; how many were erased.
pub async fn run_erasures(shared: &Shared, now_iso: &str) -> Result<usize, LauncherApiError> {
    let mut erased = 0;
    for account in erasure::due_deletions(shared, now_iso, ERASURE_BATCH).await? {
        match erasure::erase_account(shared, &account, true).await {
            Ok(erasure::ErasureOutcome::Erased(_)) => erased += 1,
            Ok(erasure::ErasureOutcome::Blocked(why)) => {
                tracing::info!("[Erasure] {} waits: {}", account.user_id, why.join(", "));
            }
            Ok(erasure::ErasureOutcome::DryRun) => {}
            Err(error) => tracing::warn!("[Erasure] {} failed ({})", account.user_id, error.code),
        }
    }
    Ok(erased)
}

/// `PurgeExpiredUploadedLogs` over the host's log folder, off the async threads.
pub async fn run_log_purge(shared: &Shared) -> player_logs::RetentionPass {
    let root = crate::areas::reforged_api::logs::log_storage_root(shared);
    tokio::task::spawn_blocking(move || player_logs::purge_expired(&root, util::now_ms(), player_logs::RETENTION_DAYS))
        .await
        .unwrap_or_default()
}

/// The jobs the host switched on: `MYSTICPARADOX_ACCOUNT_ERASURE` (the 30-day erasure, Privacy Policy section 5) and
/// `MYSTICPARADOX_LOG_PURGE` (the 90-day purge of uploaded launcher logs). Both are off unless on, because the erasure
/// is irreversible past the soft-delete retention and Node's script needed an operator's `--apply`.
pub fn start_jobs(shared: &Arc<Shared>) -> Vec<tokio::task::JoinHandle<()>> {
    let mut tasks = Vec::new();
    if switched_on(&shared.settings, "MYSTICPARADOX_ACCOUNT_ERASURE") {
        let shared = shared.clone();
        tasks.push(tokio::spawn(async move {
            let mut interval = tokio::time::interval(JOB_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                match run_erasures(&shared, &util::iso_now()).await {
                    Ok(0) => {}
                    Ok(erased) => tracing::info!("[Erasure] {erased} account(s) erased"),
                    Err(error) => tracing::warn!("[Erasure] run failed ({})", error.code),
                }
            }
        }));
    }
    if switched_on(&shared.settings, "MYSTICPARADOX_LOG_PURGE") {
        let shared = shared.clone();
        tasks.push(tokio::spawn(async move {
            let mut interval = tokio::time::interval(JOB_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                let pass = run_log_purge(&shared).await;
                if pass.removed > 0 || pass.failed > 0 {
                    tracing::info!("[LogPurge] removed {}, failed {}", pass.removed, pass.failed);
                }
            }
        }));
    }
    tasks
}
