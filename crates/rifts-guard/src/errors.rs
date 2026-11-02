// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use rifts_seam::{Json, Refusal, Reply, SeamError, StoreError, Unimplemented};

use crate::json::push_js_string;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum GuardError {
    #[error(transparent)]
    Unimplemented(#[from] Unimplemented),
    #[error(transparent)]
    Refused(#[from] Refusal),
    #[error("{0}")]
    Internal(String),
}

pub(crate) const INTERNAL_CODE: &str = "INTERNAL";
pub(crate) const INTERNAL_MESSAGE: &str = "The player-hosted hunt request failed.";

impl GuardError {
    pub fn refused(code: &str, message: &str) -> Self {
        Self::Refused(Refusal {
            code: code.to_owned(),
            message: message.to_owned(),
        })
    }

    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Refused(refusal) => Some(&refusal.code),
            _ => None,
        }
    }
}

impl From<StoreError> for GuardError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::Unimplemented(missing) => Self::Unimplemented(missing),
            other => Self::Internal(other.to_string()),
        }
    }
}

impl From<SeamError> for GuardError {
    fn from(error: SeamError) -> Self {
        match error {
            SeamError::Unimplemented(missing) => Self::Unimplemented(missing),
            SeamError::Refused(refusal) => Self::Refused(refusal),
            other => Self::Internal(other.to_string()),
        }
    }
}

/// A hook's or route's answer: the detail of an internal failure never leaves the server.
impl From<GuardError> for SeamError {
    fn from(error: GuardError) -> Self {
        match error {
            GuardError::Unimplemented(missing) => Self::Unimplemented(missing),
            GuardError::Refused(refusal) => Self::Refused(refusal),
            GuardError::Internal(_) => Self::Refused(Refusal {
                code: INTERNAL_CODE.to_owned(),
                message: INTERNAL_MESSAGE.to_owned(),
            }),
        }
    }
}

/// The host's transport retries 408, 429 and 5xx and gives a hunt's evidence up on any other refusal, so these
/// statuses are part of the contract (spec-rd27-rifts-guard §3.2).
pub const ERROR_STATUSES: &[(&str, u16)] = &[
    ("P2P_SESSION_NOT_FOUND", 404),
    ("P2P_CAPABILITY_INVALID", 401),
    ("P2P_GENERATION_FENCED", 409),
    ("GUARD_REQUIRED", 403),
    ("GUARD_BATCH_INVALID", 400),
    ("GUARD_BATCH_TOO_LARGE", 413),
    ("GUARD_BATCH_REPLAY", 409),
    ("GUARD_EVIDENCE_LIMIT", 409),
    ("GUARD_TRAFFIC_RATE_LIMITED", 429),
    ("GUARD_REQUEST_BUDGET_EXHAUSTED", 503),
    ("GUARD_TRAFFIC_UNAVAILABLE", 503),
    ("GUARD_FINALIZATION_BUSY", 503),
    ("GUARD_CHAIN_CONFLICT", 409),
    ("GUARD_CHAIN_FROZEN", 409),
    ("GUARD_COMPLETION_INVALID", 409),
    ("GUARD_COMPLETION_CONFLICT", 409),
    ("GUARD_COMPLETION_PENDING", 409),
    ("GUARD_SHADOW_INVALID", 403),
    ("REWARD_CAPACITY_RECONCILIATION_REQUIRED", 503),
    ("HUNT_EVIDENCE_NOT_COLLECTED", 404),
    ("HUNT_REWARDS_UNAVAILABLE", 404),
];

pub fn status_of(code: &str) -> u16 {
    ERROR_STATUSES
        .iter()
        .find(|(known, _)| *known == code)
        .map_or(500, |(_, status)| *status)
}

/// The launcher's error envelope.
pub fn envelope(refusal: &Refusal, request_id: &str) -> Reply {
    let mut body = String::from(r#"{"error":{"code":"#);
    push_js_string(&mut body, &refusal.code);
    body.push_str(r#","message":"#);
    push_js_string(&mut body, &refusal.message);
    body.push_str(r#","requestId":"#);
    push_js_string(&mut body, request_id);
    body.push_str("}}");
    Reply {
        status: status_of(&refusal.code),
        body: Json(body),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rifts_seam::Component;

    #[test]
    fn statuses() {
        assert_eq!(ERROR_STATUSES.len(), 21);
        assert_eq!(status_of("GUARD_TRAFFIC_RATE_LIMITED"), 429);
        assert_eq!(status_of("GUARD_BATCH_TOO_LARGE"), 413);
        assert_eq!(status_of("P2P_CAPABILITY_INVALID"), 401);
        assert_eq!(status_of("HUNT_REWARDS_UNAVAILABLE"), 404);
        assert_eq!(status_of("AUTH_UNAUTHORIZED"), 500);
        assert_eq!(status_of("INTERNAL"), 500);
        for (code, status) in ERROR_STATUSES {
            assert_eq!(status_of(code), *status);
        }
    }

    #[test]
    fn conversions_and_internal_mapping() {
        let missing = Unimplemented::new(Component::Guard, "store.chain");
        assert_eq!(
            GuardError::from(StoreError::Unimplemented(missing)),
            GuardError::Unimplemented(missing)
        );
        assert_eq!(
            GuardError::from(StoreError::Database("disk".into())),
            GuardError::Internal("disk".into())
        );
        let refused = GuardError::refused("GUARD_BATCH_INVALID", "No.");
        assert_eq!(refused.code(), Some("GUARD_BATCH_INVALID"));
        assert_eq!(refused.to_string(), "GUARD_BATCH_INVALID: No.");
        assert_eq!(
            GuardError::from(SeamError::from(missing)),
            GuardError::Unimplemented(missing)
        );
        assert_eq!(
            SeamError::from(GuardError::Internal("secret detail".into())),
            SeamError::Refused(Refusal {
                code: "INTERNAL".into(),
                message: "The player-hosted hunt request failed.".into()
            })
        );
        assert_eq!(GuardError::Internal("x".into()).code(), None);
        assert_eq!(GuardError::Internal("x".into()).to_string(), "x");
    }

    #[test]
    fn envelope_text() {
        let reply = envelope(
            &Refusal {
                code: "GUARD_CHAIN_FROZEN".into(),
                message: "Closed \"now\".".into(),
            },
            "req-1",
        );
        assert_eq!(reply.status, 409);
        assert_eq!(
            reply.body.0,
            r#"{"error":{"code":"GUARD_CHAIN_FROZEN","message":"Closed \"now\".","requestId":"req-1"}}"#
        );
    }
}
