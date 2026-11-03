// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The launcher's long poll for the messages queued for one account (offers, bootstraps, fences, ICE signals) and
//! their acknowledgement.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use rifts_seam::{AccountId, Actor, Refusal};
use serde::Serialize;
use serde_json::Value;

use crate::{
    RealmsError,
    db::RealmsDb,
    hunt::ControlMessageRecord,
    ports::Clock,
    repo::hunts::{HuntRepository, NewControlMessage},
    tiers::js_number,
    time::iso,
};

const LONGEST_WAIT_MS: f64 = 25_000.0;
const POLL_STEP_MS: u64 = 250;
const MAX_MESSAGES: u32 = 32;
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// One message as the launcher reads it; fields in wire order.
#[derive(Clone, Debug, PartialEq, Serialize)]
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
    pub payload: Value,
    pub created_at: String,
    pub expires_at: String,
    pub sequence: i64,
    pub ttl_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acknowledged_at: Option<String>,
    pub message_id: String,
}

impl From<&ControlMessageRecord> for ControlMessageView {
    /// `_id` and `messageId` both carry the id, and `ttlAt` repeats the expiry, as the stored documents did.
    fn from(record: &ControlMessageRecord) -> Self {
        let expires = iso(record.expires_at);
        Self {
            payload: record.payload.clone(),
            sequence: record.sequence,
            generation: record.generation,
            session_id: record.session_id.clone(),
            message_type: record.message_type.clone(),
            account_id: record.account_id.clone(),
            created_at: iso(record.created_at),
            acknowledged_at: record.acknowledged_at.map(iso),
            ttl_at: expires.clone(),
            expires_at: expires,
            message_id: record.message_id.clone(),
            id: record.message_id.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PollReply {
    pub messages: Vec<ControlMessageView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_cursor: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AckReply {
    pub acknowledged: bool,
    pub message_id: String,
}

/// The channel's refusals: (code, message).
const BAD_CURSOR: (&str, &str) = ("P2P_TICKET_INVALID", "The control cursor is invalid.");
const BAD_MESSAGE_ID: (&str, &str) = ("P2P_TICKET_INVALID", "The control message ID is invalid.");
const NO_MESSAGE: (&str, &str) = ("P2P_SESSION_NOT_FOUND", "Control message not found.");

fn refusal((code, message): (&str, &str)) -> RealmsError {
    RealmsError::Refused(Refusal {
        code: code.into(),
        message: message.into(),
    })
}

/// A query number as JavaScript's `Number()` reads it; absent is `default`, unreadable is a bad cursor.
fn query_number(raw: Option<&str>, default: f64) -> Result<f64, RealmsError> {
    raw.map_or(Ok(default), |text| js_number(text).ok_or_else(|| refusal(BAD_CURSOR)))
}

/// A UUID's shape: 36 hex digits and dashes.
fn uuid_shaped(id: &str) -> bool {
    id.len() == 36 && id.bytes().all(|b| b == b'-' || b.is_ascii_hexdigit())
}

pub struct ControlChannel {
    hunts: HuntRepository,
    clock: Arc<dyn Clock>,
}

impl ControlChannel {
    pub fn new(db: RealmsDb, clock: Arc<dyn Clock>) -> Self {
        let hunts = HuntRepository::new(db);
        Self { clock, hunts }
    }

    /// `after` and `wait` are the raw `afterSequence` and `waitMs` query values.
    pub async fn poll(&self, account: &str, after: Option<&str>, wait: Option<&str>) -> Result<PollReply, RealmsError> {
        let after = query_number(after, 0.0)?;
        let whole = after >= 0.0 && after.fract() == 0.0 && after <= MAX_SAFE_INTEGER;
        if !whole {
            return Err(refusal(BAD_CURSOR));
        }
        let wait = query_number(wait, LONGEST_WAIT_MS)?.clamp(0.0, LONGEST_WAIT_MS);
        // Exact: an integer at most 2^53 - 1, and a wait within 0-25000.
        let (after, wait) = (after as i64, wait as u64);
        // A cursor past the last message belongs to another account (launchers to 0.1.72 keep one per PC):
        // answer from the start, unacknowledged only.
        let stale = after > 0 && self.hunts.latest_control_sequence(account).await? < after;
        let from = if stale { 0 } else { after };
        let deadline = Instant::now() + Duration::from_millis(wait);
        let step = Duration::from_millis(POLL_STEP_MS);
        loop {
            let found = self
                .hunts
                .list_control_messages(account, from, self.clock.now(), MAX_MESSAGES, stale)
                .await?;
            let left = deadline.saturating_duration_since(Instant::now());
            match (found.is_empty(), left.is_zero()) {
                (false, _) => {
                    let messages = found.iter().map(ControlMessageView::from).collect();
                    let stale_cursor = stale.then_some(true);
                    return Ok(PollReply { messages, stale_cursor });
                }
                (true, true) => {
                    let messages = Vec::new();
                    return Ok(PollReply {
                        messages,
                        stale_cursor: None,
                    });
                }
                (true, false) => tokio::time::sleep(step.min(left)).await,
            }
        }
    }

    pub async fn ack(&self, account: &str, message_id: &str) -> Result<AckReply, RealmsError> {
        if !uuid_shaped(message_id) {
            return Err(refusal(BAD_MESSAGE_ID));
        }
        let owner = Actor::Account(AccountId(account.into()));
        let now = self.clock.now();
        let acknowledged = self
            .hunts
            .acknowledge_control_message(&owner, account, message_id, now)
            .await?;
        acknowledged
            .then(|| AckReply {
                acknowledged,
                message_id: message_id.into(),
            })
            .ok_or_else(|| refusal(NO_MESSAGE))
    }

    /// Queues one message; its sequence.
    pub async fn send(&self, actor: &Actor, message: &NewControlMessage) -> Result<i64, RealmsError> {
        let queued = self.hunts.enqueue_control_message(actor, message).await?;
        Ok(queued.sequence)
    }
}
