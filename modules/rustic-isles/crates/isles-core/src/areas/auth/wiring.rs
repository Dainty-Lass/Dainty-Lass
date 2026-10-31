//! What this area gives the Reforged API at start: the launcher authentication, the admin sign-in's password and TOTP
//! checks, and the two periodic jobs the Privacy Policy owes (the 30-day erasure and the 90-day log purge), each off
//! unless the host turns it on.

use std::sync::Arc;
use std::time::Duration;

use module_api::BoxFuture;
use rifts_seam::{AccountId as SeamAccountId, GuardMode, IslesCore, Json, Party, PartyId, SeamError, SessionId};
use serde_json::Value;

use super::errors::{LauncherApiError, status_for};
use super::{crypto, erasure, middleware, policy, totp, util};
use crate::areas::reforged_api::admin::{self, AdminCredentials};
use crate::areas::reforged_api::launcher_auth::{self, LauncherAuthenticator, LauncherCaller, LauncherFailure};
use crate::areas::reforged_api::player_logs;
use crate::db::IslesStore;
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

/// Accounts, Launcher Guard and operations for `rifts_seam::IslesCore` (`MatchmakingCore::fallback`).
pub struct AuthReforgedCore {
    pub store: Option<IslesStore>,
    pub settings: Settings,
}

fn refused(code: &str, message: impl Into<String>) -> SeamError {
    SeamError::Refused(rifts_seam::Refusal {
        code: code.to_owned(),
        message: message.into(),
    })
}

impl IslesCore for AuthReforgedCore {
    fn display_name(&self, account: SeamAccountId) -> BoxFuture<'_, Result<Option<String>, SeamError>> {
        let store = self.store.clone();
        Box::pin(async move {
            let Some(store) = store else {
                return Ok(None);
            };
            store
                .read(move |conn, repos| {
                    if let Ok(global) = account.0.parse::<module_api::GlobalId>()
                        && let Ok(id) = crate::ids::AccountId::try_from(global)
                        && let Some(la) = repos.launcher_accounts.find_by_account(conn, &id)?
                    {
                        return Ok(Some(la.display_name));
                    }
                    if let Some(acc) = repos
                        .accounts
                        .find_by_user_id(conn, &crate::ids::UserId(account.0.clone()))?
                    {
                        if let Some(la) = repos.launcher_accounts.find_by_account(conn, &acc.id)? {
                            return Ok(Some(la.display_name));
                        }
                        return Ok(Some(acc.name));
                    }
                    Ok(None)
                })
                .await
                .map_err(|e| refused("ACCOUNT_READ_FAILED", e.to_string()))
        })
    }

    fn find_by_display_name(&self, name: String) -> BoxFuture<'_, Result<Option<SeamAccountId>, SeamError>> {
        let store = self.store.clone();
        Box::pin(async move {
            let Some(store) = store else {
                return Ok(None);
            };
            let normalized = name.trim().to_lowercase();
            store
                .read(move |conn, repos| {
                    if let Some(la) = repos
                        .launcher_accounts
                        .find_by_display_name_normalized(conn, &normalized)?
                    {
                        return Ok(Some(SeamAccountId(la.user_id.to_string())));
                    }
                    Ok(None)
                })
                .await
                .map_err(|e| refused("ACCOUNT_READ_FAILED", e.to_string()))
        })
    }

    fn launcher_account_eligible(&self, account: SeamAccountId) -> BoxFuture<'_, Result<bool, SeamError>> {
        let store = self.store.clone();
        Box::pin(async move {
            let Some(store) = store else {
                return Ok(true);
            };
            store
                .read(move |conn, repos| {
                    if let Ok(global) = account.0.parse::<module_api::GlobalId>()
                        && let Ok(id) = crate::ids::AccountId::try_from(global)
                        && let Some(la) = repos.launcher_accounts.find_by_account(conn, &id)?
                    {
                        return Ok(policy::is_account_eligible(&la));
                    }
                    if let Some(acc) = repos.accounts.find_by_user_id(conn, &crate::ids::UserId(account.0))?
                        && let Some(la) = repos.launcher_accounts.find_by_account(conn, &acc.id)?
                    {
                        return Ok(policy::is_account_eligible(&la));
                    }
                    Ok(true)
                })
                .await
                .map_err(|e| refused("ACCOUNT_READ_FAILED", e.to_string()))
        })
    }

    fn party(&self, _: PartyId) -> BoxFuture<'_, Result<Option<Party>, SeamError>> {
        rifts_seam::NoCore.party(
            "01ARZ3NDEKTSV4RRFFQ69G5FAV@local"
                .parse()
                .unwrap_or_else(|_| unreachable!()),
        )
    }

    fn get_or_create_party(&self, account: SeamAccountId) -> BoxFuture<'_, Result<Party, SeamError>> {
        rifts_seam::NoCore.get_or_create_party(account)
    }

    fn set_candidate(&self, account: SeamAccountId, candidate: rifts_seam::Candidate) -> Result<(), SeamError> {
        rifts_seam::NoCore.set_candidate(account, candidate)
    }

    fn cancel_travel(&self, hunt_id: &str, accounts: &[SeamAccountId], reason: &str) -> Result<bool, SeamError> {
        rifts_seam::NoCore.cancel_travel(hunt_id, accounts, reason)
    }

    fn dedicated_servers_configured(&self) -> bool {
        false
    }

    fn touch_game_session(&self, account: SeamAccountId) -> BoxFuture<'_, Result<(), SeamError>> {
        rifts_seam::NoCore.touch_game_session(account)
    }

    fn record_hunt_activity(&self, session: SessionId, activity: Json) -> BoxFuture<'_, Result<(), SeamError>> {
        rifts_seam::NoCore.record_hunt_activity(session, activity)
    }

    fn healthy_guard_session(&self, account: SeamAccountId) -> BoxFuture<'_, Result<Option<Json>, SeamError>> {
        rifts_seam::NoCore.healthy_guard_session(account)
    }

    fn published_guard_artifacts(&self) -> BoxFuture<'_, Result<Json, SeamError>> {
        rifts_seam::NoCore.published_guard_artifacts()
    }

    fn guard_mode(&self) -> GuardMode {
        GuardMode::from_setting(self.settings.raw("MYSTICPARADOX_GUARD_MODE"))
    }

    fn operations_policy(&self) -> BoxFuture<'_, Result<Json, SeamError>> {
        let store = self.store.clone();
        let observe_override = self.settings.raw("GUARD_ENFORCEMENT_OVERRIDE") == Some("OBSERVE");
        Box::pin(async move {
            let record = match store {
                Some(store) => store
                    .read(|conn, repos| repos.admin.read_operations_policy(conn))
                    .await
                    .ok()
                    .flatten(),
                None => None,
            };
            let mut policy = match record {
                Some(record) => crate::areas::reforged_api::operations::normalize(&record, util::now_ms()).0,
                None => crate::areas::reforged_api::operations::Policy::default_policy(),
            };
            if observe_override {
                policy.guard_enforcement = "OBSERVE";
            }
            Ok(Json(policy.to_json().to_string()))
        })
    }

    fn verify_operations_step_up(&self, key_id: String, step_up: String) -> BoxFuture<'_, Result<bool, SeamError>> {
        let secret = self
            .settings
            .get("ADMIN_OPERATIONS_TOTP_SECRET")
            .map(service_kit::js::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                self.settings
                    .get("ADMIN_TOTP_SECRET")
                    .map(service_kit::js::trim)
                    .filter(|s| !s.is_empty())
            })
            .map(str::to_owned);
        Box::pin(async move {
            let limited = crate::areas::reforged_api::util::is_rate_limited(
                &format!("admin-operations-step-up:{key_id}"),
                6,
                15 * 60_000,
            );
            let code = Value::String(step_up);
            Ok(secret.is_some_and(|s| !limited && totp::verify_totp(&s, Some(&code))))
        })
    }
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
    if let Some(store) = shared.store.clone() {
        tasks.push(tokio::spawn(async move {
            let mut interval = tokio::time::interval(JOB_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                let now = util::now_ms();
                let actor = module_api::ActorContext::new(module_api::Actor::System);
                match store
                    .write(&actor, move |tx, _| {
                        crate::repo::reforged::purge_expired_ttl_rows(tx, now)
                    })
                    .await
                {
                    Ok(0) => {}
                    Ok(purged) => tracing::info!("[TtlPurge] purged {purged} expired row(s)"),
                    Err(error) => tracing::warn!("[TtlPurge] run failed: {error}"),
                }
            }
        }));
    }
    tasks
}
