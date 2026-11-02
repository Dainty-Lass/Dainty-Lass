//! The launcher's control channel (`p2p/controllers/p2pControl.ts`): a long poll for the messages queued for one
//! account (offers, bootstraps, fences, ICE signals) and their acknowledgement.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rifts_seam::{AccountId, Actor, Refusal};
use serde::Serialize;

use crate::RealmsError;
use crate::db::RealmsDb;
use crate::hunt::ControlMessageRecord;
use crate::ports::Clock;
use crate::repo::hunts::{HuntRepository, NewControlMessage};
use crate::tiers::js_number;
use crate::time::iso;

const MAX_WAIT_MS: f64 = 25_000.0;
const POLL_MS: u64 = 250;
const MAX_MESSAGES: u32 = 32;

/// A message as Node answers it: the stored document (`_id`, the fields, `sequence`, the TTL date) plus `messageId`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlMessageView {
    #[serde(rename = "_id")]
    pub id: String,
    pub account_id: String,
    pub message_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation: Option<i64>,
    pub payload: serde_json::Value,
    pub created_at: String,
    pub expires_at: String,
    pub sequence: i64,
    pub ttl_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acknowledged_at: Option<String>,
    pub message_id: String,
}

impl From<&ControlMessageRecord> for ControlMessageView {
    fn from(m: &ControlMessageRecord) -> Self {
        Self {
            id: m.message_id.clone(),
            account_id: m.account_id.clone(),
            message_type: m.message_type.clone(),
            session_id: m.session_id.clone(),
            generation: m.generation,
            payload: m.payload.clone(),
            created_at: iso(m.created_at),
            expires_at: iso(m.expires_at),
            sequence: m.sequence,
            ttl_at: iso(m.expires_at),
            acknowledged_at: m.acknowledged_at.map(iso),
            message_id: m.message_id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PollReply {
    pub messages: Vec<ControlMessageView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_cursor: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AckReply {
    pub acknowledged: bool,
    pub message_id: String,
}

fn refused(code: &str, message: &str) -> RealmsError {
    RealmsError::Refused(Refusal {
        code: code.to_owned(),
        message: message.to_owned(),
    })
}

pub struct ControlChannel {
    hunts: HuntRepository,
    clock: Arc<dyn Clock>,
}

impl ControlChannel {
    pub fn new(db: RealmsDb, clock: Arc<dyn Clock>) -> Self {
        Self {
            hunts: HuntRepository::new(db),
            clock,
        }
    }

    /// `PollP2PControl` with the query's `afterSequence` and `waitMs` as text. A cursor beyond the account's last
    /// message belongs to another account (launchers to 0.1.72 keep one per PC): it is answered from the start with
    /// the unacknowledged messages, flagged `staleCursor`.
    pub async fn poll(&self, account: &str, after: Option<&str>, wait: Option<&str>) -> Result<PollReply, RealmsError> {
        let after = after.map_or(Some(0.0), js_number);
        let wait = wait
            .map_or(Some(MAX_WAIT_MS), js_number)
            .map(|w| w.clamp(0.0, MAX_WAIT_MS));
        let (Some(after), Some(wait)) = (after, wait) else {
            return Err(refused("P2P_TICKET_INVALID", "The control cursor is invalid."));
        };
        if after < 0.0 || after.fract() != 0.0 || after > 9_007_199_254_740_991.0 || wait.is_nan() {
            return Err(refused("P2P_TICKET_INVALID", "The control cursor is invalid."));
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (after, wait) = (after as i64, wait as u64);
        let stale = after > 0 && after > self.hunts.latest_control_sequence(account).await?;
        let deadline = Instant::now() + Duration::from_millis(wait);
        loop {
            let messages = self
                .hunts
                .list_control_messages(
                    account,
                    if stale { 0 } else { after },
                    self.clock.now(),
                    MAX_MESSAGES,
                    stale,
                )
                .await?;
            if !messages.is_empty() {
                return Ok(PollReply {
                    messages: messages.iter().map(ControlMessageView::from).collect(),
                    stale_cursor: stale.then_some(true),
                });
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            tokio::time::sleep(left.min(Duration::from_millis(POLL_MS))).await;
        }
        Ok(PollReply {
            messages: Vec::new(),
            stale_cursor: None,
        })
    }

    /// `AcknowledgeP2PControl`.
    pub async fn ack(&self, account: &str, message_id: &str) -> Result<AckReply, RealmsError> {
        let shaped = message_id.len() == 36 && message_id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-');
        if !shaped {
            return Err(refused("P2P_TICKET_INVALID", "The control message ID is invalid."));
        }
        let actor = Actor::Account(AccountId(account.to_owned()));
        if !self
            .hunts
            .acknowledge_control_message(&actor, account, message_id, self.clock.now())
            .await?
        {
            return Err(refused("P2P_SESSION_NOT_FOUND", "Control message not found."));
        }
        Ok(AckReply {
            acknowledged: true,
            message_id: message_id.to_owned(),
        })
    }

    /// Queues one message; its sequence.
    pub async fn send(&self, actor: &Actor, message: &NewControlMessage) -> Result<i64, RealmsError> {
        Ok(self.hunts.enqueue_control_message(actor, message).await?.sequence)
    }
}
