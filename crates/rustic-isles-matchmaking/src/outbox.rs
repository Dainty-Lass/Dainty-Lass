// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The party outbox dispatcher (spec §8, `PartyOutboxDispatcher.ts`): the outbox as headlines, which the chat
//! plugin delivers to each recipient's connections (DD21).

use std::sync::Arc;
use std::time::Duration;

use module_api::chat::{HEADLINE_EVENT, Headline, HeadlineRecipient};
use module_api::{Actor, ActorContext, BoxFuture, Event, EventName, ModuleContext};
use rustic_isles_store::repo::parties::PartyOutboxRecord;
use rustic_isles_store::{IslesStore, RepoError};

pub const OUTBOX_POLL: Duration = Duration::from_millis(250);
pub const OUTBOX_BATCH: usize = 64;
/// A claim older than this is taken over, so a crash between claim and publish loses nothing.
pub const OUTBOX_STALE_CLAIM_MS: i64 = 30_000;
/// Node logged the first failure, then one each 30 s: 120 polls of 250 ms.
const FAILURE_LOG_EVERY: u64 = 120;

/// Where headlines go: the module context's event bus in production.
pub trait HeadlineSink: Send + Sync {
    fn emit<'a>(&'a self, headline: &'a Headline) -> BoxFuture<'a, Result<(), String>>;
}

/// The event bus of a module context; none answers an error for every headline.
pub struct ContextSink(pub Option<ModuleContext>);

impl HeadlineSink for ContextSink {
    fn emit<'a>(&'a self, headline: &'a Headline) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let context = self.0.as_ref().ok_or_else(|| "the module has no context".to_owned())?;
            let name = EventName::new(HEADLINE_EVENT).map_err(|e| e.to_string())?;
            let payload = serde_json::to_value(headline).map_err(|e| e.to_string())?;
            context.emit(Event { name, payload }).await.map_err(|e| e.to_string())
        })
    }
}

fn system() -> ActorContext {
    ActorContext::new(Actor::System)
}

fn iso(ms: i64) -> String {
    rustic_isles::season::to_iso_string(ms)
}

async fn headline(store: &IslesStore, event: &PartyOutboxRecord) -> Result<Headline, String> {
    let recipients = event.recipients.clone();
    let found = store
        .read(move |conn, repos| {
            let mut out = Vec::new();
            for user in &recipients {
                if let Some(account) = repos.accounts.find_by_user_id(conn, user)? {
                    out.push(HeadlineRecipient {
                        account: account.id.global().clone(),
                        user_id: user.as_str().to_owned(),
                    });
                }
            }
            Ok::<_, RepoError>(out)
        })
        .await
        .map_err(|e| e.to_string())?;
    let body = rustic_isles_chat::headline::public_event(
        &event.event_id,
        event.event_type.as_str(),
        &event.created_at,
        &event.party_id,
        event.party_revision,
        &event.payload,
    );
    Ok(Headline {
        id: event.event_id.clone(),
        recipients: found,
        body: service_kit::jsjson::stringify(&body),
    })
}

/// One pass of up to 64 events; a failed publish is released and stops the pass, so order is kept. `Err` is a store
/// failure of the pass itself.
pub async fn outbox_pass(
    store: &IslesStore,
    sink: &dyn HeadlineSink,
    owner: &str,
    now: i64,
) -> Result<usize, RepoError> {
    let mut published = 0;
    for _ in 0..OUTBOX_BATCH {
        let (who, at, stale) = (
            owner.to_owned(),
            iso(now),
            iso(now.saturating_sub(OUTBOX_STALE_CLAIM_MS)),
        );
        let claimed = store
            .write(&system(), move |tx, repos| {
                repos.parties.claim_next_outbox(tx, &who, &at, &stale)
            })
            .await?;
        let Some(event) = claimed else {
            break;
        };
        let sent = match headline(store, &event).await {
            Ok(headline) => sink.emit(&headline).await,
            Err(e) => Err(e),
        };
        let (who, id) = (owner.to_owned(), event.event_id.clone());
        if let Err(e) = sent {
            store
                .write(&system(), move |tx, repos| repos.parties.release_outbox(tx, &id, &who))
                .await?;
            tracing::error!("[PartyOutbox] publish failed event={}: {e}", event.event_id);
            break;
        }
        let at = iso(now);
        store
            .write(&system(), move |tx, repos| {
                repos.parties.mark_outbox_published(tx, &id, &who, &at)
            })
            .await?;
        tracing::info!(
            "[PartyOutbox] {} party={} revision={}",
            event.event_type.as_str(),
            event.party_id,
            event.party_revision
        );
        published += 1;
    }
    Ok(published)
}

/// The loop: a pass, then 250 ms, until the task is aborted.
pub async fn run_outbox(store: IslesStore, sink: Arc<dyn HeadlineSink>, clock: impl Fn() -> i64 + Send) {
    let owner = rustic_isles::replay::random_uuid();
    let mut failures: u64 = 0;
    loop {
        match outbox_pass(&store, sink.as_ref(), &owner, clock()).await {
            Ok(_) => {
                if failures > 0 {
                    tracing::info!("[PartyOutbox] database polling recovered after {failures} failed attempt(s)");
                }
                failures = 0;
            }
            Err(e) => {
                failures += 1;
                if failures == 1 || failures.is_multiple_of(FAILURE_LOG_EVERY) {
                    tracing::error!("[PartyOutbox] database polling failed; dispatcher will retry ({failures}): {e}");
                }
            }
        }
        tokio::time::sleep(OUTBOX_POLL).await;
    }
}
