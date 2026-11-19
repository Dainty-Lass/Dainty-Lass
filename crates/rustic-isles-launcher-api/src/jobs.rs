// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The public half's start-up (public spec §12.3): it binds the seams and hooks the composer put in the slots, and
//! with a store runs the log purge and the TTL purge at start and hourly.

use std::sync::Arc;
use std::time::Duration;

use dainty_module_api::{Actor, ActorContext, BoxFuture};
use rustic_isles::{Shared, Started, Startup};

use crate::activity::ActivityHooks;
use crate::seams::LauncherApiSeams;
use crate::util::{self, now_ms};

const EVERY: Duration = Duration::from_secs(3600);

/// `RUSTIC_ISLES_LOG_PURGE` reads `1`, `true`, `on` or `yes` (JS-trimmed, any ASCII case).
pub fn log_purge_on(settings: &rustic_isles::Settings) -> bool {
    let value = util::trim(settings.raw("RUSTIC_ISLES_LOG_PURGE").unwrap_or("")).to_ascii_lowercase();
    matches!(value.as_str(), "1" | "true" | "on" | "yes")
}

/// One 90-day purge pass, logged when it did anything.
pub async fn run_log_purge(shared: &Shared) {
    let root = crate::logs::log_storage_root(shared.settings());
    let now = now_ms();
    let report =
        tokio::task::spawn_blocking(move || crate::logs::purge_expired(&root, now, crate::logs::RETENTION_DAYS))
            .await
            .unwrap_or_default();
    if report.removed > 0 || report.failed > 0 {
        tracing::info!("[LogPurge] removed {}, failed {}", report.removed, report.failed);
    }
}

/// One TTL purge pass.
pub async fn run_ttl_purge(shared: &Shared) {
    let Some(store) = util::store_of(shared) else {
        return;
    };
    let now = now_ms();
    let purged = store
        .write(&ActorContext::new(Actor::System), move |tx, _| {
            let rows = rustic_isles_store::repo::reforged::purge_expired_ttl_rows(tx, now)?;
            // Save snapshots past their 30 days, with the rows they hold
            Ok(rows + rustic_isles_store::repo::resets::prune(tx, now)?)
        })
        .await;
    match purged {
        Ok(0) => {}
        Ok(n) => tracing::info!("[TtlPurge] purged {n} expired row(s)"),
        Err(e) => tracing::warn!("[TtlPurge] run failed: {e}"),
    }
}

struct PublicStartup;

impl Startup for PublicStartup {
    fn start<'a>(&'a self, shared: &'a Arc<Shared>) -> BoxFuture<'a, Result<Started, String>> {
        Box::pin(async move {
            if let Some(seams) = shared.slot::<LauncherApiSeams>() {
                seams.bind(shared);
            }
            if let Some(hooks) = shared.slot::<ActivityHooks>() {
                hooks.bind(shared);
            }
            let mut started = Started::default();
            if util::store_of(shared).is_none() {
                return Ok(started);
            }
            if log_purge_on(shared.settings()) {
                let weak = Arc::downgrade(shared);
                started.tasks.push(tokio::spawn(async move {
                    loop {
                        let Some(shared) = weak.upgrade() else {
                            return;
                        };
                        run_log_purge(&shared).await;
                        drop(shared);
                        tokio::time::sleep(EVERY).await;
                    }
                }));
            }
            let weak = Arc::downgrade(shared);
            started.tasks.push(tokio::spawn(async move {
                loop {
                    let Some(shared) = weak.upgrade() else {
                        return;
                    };
                    run_ttl_purge(&shared).await;
                    drop(shared);
                    tokio::time::sleep(EVERY).await;
                }
            }));
            Ok(started)
        })
    }
}

/// The public half's start-ups, for core's `Composition.startups`.
pub fn public_startups() -> Vec<Arc<dyn Startup>> {
    vec![Arc::new(PublicStartup)]
}
