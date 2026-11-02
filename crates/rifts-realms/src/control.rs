// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The launcher's long poll for the messages queued for one account (offers, bootstraps, fences, ICE signals) and
//! their acknowledgement.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rifts_seam::{AccountId, Actor, Refusal};
use serde::Serialize;
use serde_json::Value;

use crate::RealmsError;
use crate::db::RealmsDb;
use crate::hunt::ControlMessageRecord;
use crate::ports::Clock;
use crate::repo::hunts::{HuntRepository, NewControlMessage};
use crate::tiers::js_number;
use crate::time::iso;

const LONGEST_WAIT_MS: f64 = 25_000.0;
const POLL_STEP_MS: u64 = 250;
const MAX_MESSAGES: u32 = 32;
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// One message as the launcher reads it; fields in wire order.
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
    fn from(record: &ControlMessageRecord) -> Self {
        Self {
            id: record.message_id.clone(),
            account_id: record.account_id.clone(),
            message_type: record.message_type.clone(),
            session_id: record.session_id.clone(),
            generation: record.generation,
            payload: record.payload.clone(),
            created_at: iso(record.created_at),
            expires_at: iso(record.expires_at),
            sequence: record.sequence,
            ttl_at: iso(record.expires_at),
            acknowledged_at: record.acknowledged_at.map(iso),
            message_id: record.message_id.clone(),
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

fn bad_cursor() -> RealmsError {
    refused("P2P_TICKET_INVALID", "The control cursor is invalid.")
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

    /// `after` and `wait` are the raw `afterSequence` and `waitMs` query values.
    pub async fn poll(&self, account: &str, after: Option<&str>, wait: Option<&str>) -> Result<PollReply, RealmsError> {
        let after = match after {
            None => 0.0,
            Some(text) => js_number(text).ok_or_else(bad_cursor)?,
        };
        if after < 0.0 || after.fract() != 0.0 || after > MAX_SAFE_INTEGER {
            return Err(bad_cursor());
        }
        let wait = match wait {
            None => LONGEST_WAIT_MS,
            Some(text) => js_number(text).ok_or_else(bad_cursor)?.clamp(0.0, LONGEST_WAIT_MS),
        };
        // Exact: an integer at most 2^53 - 1, and a wait within 0-25000.
        let (after, wait) = (after as i64, wait as u64);
        // A cursor past the account's last message belongs to another account (launchers to 0.1.72 keep one
        // per PC): answer from the start, unacknowledged only.
        let stale = after > 0 && after > self.hunts.latest_control_sequence(account).await?;
        let deadline = Instant::now() + Duration::from_millis(wait);
        loop {
            let from = if stale { 0 } else { after };
            let records = self
                .hunts
                .list_control_messages(account, from, self.clock.now(), MAX_MESSAGES, stale)
                .await?;
            if !records.is_empty() {
                return Ok(PollReply {
                    messages: records.iter().map(ControlMessageView::from).collect(),
                    stale_cursor: stale.then_some(true),
                });
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(PollReply {
                    messages: Vec::new(),
                    stale_cursor: None,
                });
            }
            tokio::time::sleep(left.min(Duration::from_millis(POLL_STEP_MS))).await;
        }
    }

    pub async fn ack(&self, account: &str, message_id: &str) -> Result<AckReply, RealmsError> {
        let shaped = message_id.len() == 36 && message_id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-');
        if !shaped {
            return Err(refused("P2P_TICKET_INVALID", "The control message ID is invalid."));
        }
        let actor = Actor::Account(AccountId(account.to_owned()));
        let found = self
            .hunts
            .acknowledge_control_message(&actor, account, message_id, self.clock.now())
            .await?;
        if !found {
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
