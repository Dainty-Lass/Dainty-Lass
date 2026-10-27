//! `realtime/PartyOutboxDispatcher.ts`: publishes the party outbox. Node writes a `<message type="headline">` from
//! `party-events@` to each recipient's connections; the connections are the chat plugin's (DD21), so the module claims
//! the events, builds each headline's JSON ([`isles_chat::headline`]) and emits it as [`HEADLINE_EVENT`] for the plugin
//! to deliver. Evicting a leaving member from the party's room (`roomService.evictPartyMember`) is the plugin's too.
//!
//! Claim, publish, mark: an event whose publish fails is released and the pass stops (Node's `break`); a claim older
//! than 30 s is taken over by the next pass, so a crash between claim and publish loses nothing.

use std::time::Duration;

use isles_chat::headline::{HEADLINE_EVENT, Headline, Recipient, public_event};
use module_api::{Actor, ActorContext, BoxFuture, Event, EventName, ModuleContext};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::util::iso;
use crate::db::IslesStore;
use crate::repo::parties::PartyOutboxRecord;
use crate::repo::{RepoError, UserId};

const POLL: Duration = Duration::from_millis(250);
const CLAIM_STALE_MS: i64 = 30_000;
const MAX_BATCH: usize = 64;

/// Where headlines go: the module's event bus in production, a list in a test.
pub type Publish<'a> = &'a (dyn Fn(Headline) -> BoxFuture<'static, Result<(), String>> + Send + Sync);

/// The headline of one outbox record, for the recipients that still have an account.
pub async fn headline(store: &IslesStore, record: &PartyOutboxRecord) -> Result<Headline, RepoError> {
    let users: Vec<UserId> = record.recipients.clone();
    let wanted = users.clone();
    let accounts = store
        .read(move |conn, repos| {
            let mut found = Vec::new();
            for user in &wanted {
                if let Some(account) = repos.accounts.find_by_user_id(conn, user)? {
                    found.push(Recipient {
                        account: account.id.global().clone(),
                        user_id: user.to_string(),
                    });
                }
            }
            Ok(found)
        })
        .await?;
    let body = public_event(
        &record.event_id,
        record.event_type.as_str(),
        &record.created_at,
        &record.party_id,
        record.party_revision,
        &record.payload,
    );
    Ok(Headline {
        id: record.event_id.clone(),
        recipients: accounts,
        body: body.to_string(),
    })
}

/// `drain`: up to 64 events, oldest first. Returns how many were published.
pub async fn drain(store: &IslesStore, owner: &str, now_ms: i64, publish: Publish<'_>) -> Result<usize, RepoError> {
    let actor = ActorContext::new(Actor::System);
    let mut published = 0;
    for _ in 0..MAX_BATCH {
        let (now, stale_before) = (iso(now_ms), iso(now_ms - CLAIM_STALE_MS));
        let who = owner.to_owned();
        let Some(event) = store
            .write(&actor, move |tx, repos| {
                repos.parties.claim_next_outbox(tx, &who, &now, &stale_before)
            })
            .await?
        else {
            break;
        };
        let sent = match headline(store, &event).await {
            Ok(headline) => publish(headline).await,
            Err(error) => Err(error.to_string()),
        };
        let (id, who) = (event.event_id.clone(), owner.to_owned());
        match sent {
            Ok(()) => {
                let done = iso(now_ms);
                store
                    .write(&actor, move |tx, repos| {
                        repos.parties.mark_outbox_published(tx, &id, &who, &done)
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
            Err(error) => {
                store
                    .write(&actor, move |tx, repos| repos.parties.release_outbox(tx, &id, &who))
                    .await?;
                tracing::error!("[PartyOutbox] publish failed event={}: {error}", event.event_id);
                break;
            }
        }
    }
    Ok(published)
}

/// The dispatcher as a module task: every 250 ms until stopped.
pub fn start(store: IslesStore, ctx: ModuleContext) -> Option<(oneshot::Sender<()>, JoinHandle<()>)> {
    let name = EventName::new(HEADLINE_EVENT).ok()?;
    let (stop, mut stopped) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let owner = uuid::Uuid::new_v4().to_string();
        let mut failures = 0u32;
        loop {
            let publish = |headline: Headline| -> BoxFuture<'static, Result<(), String>> {
                let (ctx, name) = (ctx.clone(), name.clone());
                Box::pin(async move {
                    let payload = serde_json::to_value(&headline).map_err(|e| e.to_string())?;
                    ctx.emit(Event { name, payload }).await.map_err(|e| e.to_string())
                })
            };
            let now = i64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis()),
            )
            .unwrap_or(i64::MAX);
            match drain(&store, &owner, now, &publish).await {
                Ok(_) => {
                    if failures > 0 {
                        tracing::info!("[PartyOutbox] database polling recovered after {failures} failed attempt(s)");
                        failures = 0;
                    }
                }
                Err(error) => {
                    failures += 1;
                    // Node logs the first failure and then one every 30 s; here, the first and every 120th (250 ms)
                    if failures == 1 || failures.is_multiple_of(120) {
                        tracing::error!(
                            "[PartyOutbox] database polling failed; dispatcher will retry ({failures}): {error}"
                        );
                    }
                }
            }
            tokio::select! {
                () = tokio::time::sleep(POLL) => {}
                _ = &mut stopped => break,
            }
        }
    });
    Some((stop, task))
}
