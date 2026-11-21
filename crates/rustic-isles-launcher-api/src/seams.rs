// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! `rustic_isles_auth::Seams` implemented over the module's state (public spec §13.1). The composer hands these to
//! auth before the module's `Shared` exists, so each is bound to it at start-up; unbound, each answers the default.

use std::sync::{Arc, OnceLock, Weak};

use dainty_module_api::BoxFuture;
use rustic_isles::Shared;
use rustic_isles_auth::{
    Activity, Erasure, LauncherAccount, LauncherReport, LauncherVerdict, Notices, Operations, OperationsPolicy,
};
use serde_json::Value;

use crate::state::LauncherApiState;
use crate::util::{self, MINUTE_MS};

#[derive(Default)]
struct Binding(OnceLock<Weak<Shared>>);

impl Binding {
    fn get(&self) -> Option<Arc<Shared>> {
        self.0.get()?.upgrade()
    }
}

/// The four seams auth takes from this area, over one binding.
#[derive(Clone, Default)]
pub struct LauncherApiSeams {
    binding: Arc<Binding>,
}

impl std::fmt::Debug for LauncherApiSeams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LauncherApiSeams")
            .field("bound", &self.binding.0.get().is_some())
            .finish()
    }
}

impl LauncherApiSeams {
    /// Seams not yet bound to a module.
    pub fn new() -> Self {
        Self::default()
    }

    /// Points every seam at the module (the public start-up does it when the seams are in the slots).
    pub fn bind(&self, shared: &Arc<Shared>) {
        let _ = self.binding.0.set(Arc::downgrade(shared));
    }

    /// Auth's activity seam.
    pub fn activity(&self) -> Arc<dyn Activity> {
        Arc::new(self.clone())
    }

    /// Auth's operations seam.
    pub fn operations(&self) -> Arc<dyn Operations> {
        Arc::new(self.clone())
    }

    /// Auth's notices seam.
    pub fn notices(&self) -> Arc<dyn Notices> {
        Arc::new(self.clone())
    }

    /// Auth's erasure seam.
    pub fn erasure(&self) -> Arc<dyn Erasure> {
        Arc::new(self.clone())
    }

    fn shared(&self) -> Option<Arc<Shared>> {
        self.binding.get()
    }
}

/// The pinned launcher hash: Guard's `stable` manifest on `win64`, which no valid manifest has (§10.4, parity).
fn pinned_launcher(shared: &Shared) -> Option<String> {
    let (_, payload) = crate::updates::published_guard_manifest(shared, "stable", "win64")?;
    payload
        .artifacts
        .into_iter()
        .find(|a| a.role == "LAUNCHER")
        .map(|a| a.sha256)
}

impl Activity for LauncherApiSeams {
    fn touch_game_session<'a>(&'a self, user_id: &'a str, at_ms: i64) -> BoxFuture<'a, ()> {
        if let Some(shared) = self.shared() {
            crate::activity::touch_game_session(&shared, user_id, at_ms);
        }
        Box::pin(async {})
    }

    fn classify_launcher_build<'a>(&'a self, report: Option<&'a LauncherReport>) -> BoxFuture<'a, LauncherVerdict> {
        Box::pin(async move {
            let Some(shared) = self.shared() else {
                return LauncherVerdict::Unknown;
            };
            let pinned = pinned_launcher(&shared);
            let served = crate::updates::served_launcher_version(&shared, "windows", "x86_64");
            rustic_isles_auth::launcher_build::launcher_verdict(report, pinned.as_deref(), served.as_deref())
        })
    }

    fn record_launcher_build<'a>(
        &'a self,
        user_id: &'a str,
        report: Option<&'a LauncherReport>,
        verdict: LauncherVerdict,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let Some(shared) = self.shared() else {
                return;
            };
            if verdict != LauncherVerdict::Unofficial {
                return;
            }
            let (Some(report), Some(pinned)) = (report, pinned_launcher(&shared)) else {
                return;
            };
            let served = crate::updates::served_launcher_version(&shared, "windows", "x86_64");
            crate::activity::record_unofficial_launcher(&shared, user_id, report, &pinned, served.as_deref());
        })
    }

    fn touch_matchmaking_presence<'a>(&'a self, user_id: &'a str, at_ms: i64) -> BoxFuture<'a, ()> {
        if let Some(shared) = self.shared()
            && let Some(presence) = LauncherApiState::of(&shared).matchmaking_presence()
        {
            presence.touch(user_id, at_ms);
        }
        Box::pin(async {})
    }
}

impl Operations for LauncherApiSeams {
    fn policy(&self) -> BoxFuture<'_, OperationsPolicy> {
        Box::pin(async move {
            let Some(shared) = self.shared() else {
                return OperationsPolicy::default();
            };
            match crate::operations::current_policy(&shared).await {
                Some(p) => OperationsPolicy {
                    guard_enforcement: p.guard_enforcement,
                    diagnostics_profile: p.diagnostics_profile,
                    version: p.version,
                },
                None => OperationsPolicy::default(),
            }
        })
    }
}

impl Notices for LauncherApiSeams {
    fn public_view(&self, now_ms: i64) -> BoxFuture<'_, Result<Value, String>> {
        Box::pin(async move {
            let Some(store) = self.shared().and_then(|s| util::store_of(&s)) else {
                return Ok(rustic_isles_auth::empty_notices(now_ms));
            };
            match store.read(|conn, repos| repos.launcher_notices.get(conn)).await {
                Ok(record) => Ok(crate::notices::public_view(record.as_ref(), now_ms)),
                Err(e) => Err(e.to_string()),
            }
        })
    }
}

impl Erasure for LauncherApiSeams {
    fn blockers<'a>(&'a self, account: &'a LauncherAccount) -> BoxFuture<'a, Result<Vec<String>, String>> {
        Box::pin(async move {
            let Some(shared) = self.shared() else {
                return Ok(Vec::new());
            };
            let state = LauncherApiState::of(&shared);
            let mut blockers = Vec::new();
            if state.active_players(&shared, 3 * MINUTE_MS).contains(&account.user_id) {
                blockers.push("account is currently online".to_owned());
            }
            if state.has_matchmaking_candidate(&account.user_id) {
                blockers.push("account has an active matchmaking candidate".to_owned());
            }
            Ok(blockers)
        })
    }

    fn remove_uploaded_logs<'a>(&'a self, account: &'a LauncherAccount) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let Some(shared) = self.shared() else {
                return Ok(());
            };
            let root = crate::logs::log_storage_root(shared.settings());
            let user = account.user_id.clone();
            tokio::task::spawn_blocking(move || crate::logs::remove_account_logs(&root, &user))
                .await
                .map_err(|e| e.to_string())?
                .map(|_| ())
        })
    }
}
