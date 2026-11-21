// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

use std::fmt;

use rustic_rifts_seam::{Json, Refusal, Reply, SeamError, StoreError, Unimplemented};

use crate::json::js_string;

/// Guard's failures: not written yet, a refusal with its code, or an internal fault whose text stays on the server.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum GuardError {
    /// A route or operation that is not implemented yet.
    Unimplemented(Unimplemented),
    /// Refused with a code and message the caller sees.
    Refused(Refusal),
    /// An internal fault; the text is logged, never sent.
    Internal(String),
}

pub(crate) const INTERNAL_CODE: &str = "INTERNAL";
pub(crate) const INTERNAL_MESSAGE: &str = "The player-hosted hunt request failed.";

impl GuardError {
    /// A refusal with this code and message.
    pub fn refused(code: &str, message: &str) -> Self {
        let refusal = Refusal {
            code: code.into(),
            message: message.into(),
        };
        refusal.into()
    }

    /// The refusal's code, `None` for any other failure.
    pub fn code(&self) -> Option<&str> {
        let Self::Refused(refusal) = self else {
            return None;
        };
        Some(refusal.code.as_str())
    }
}

impl fmt::Display for GuardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Internal(detail) => f.write_str(detail),
            Self::Refused(refusal) => fmt::Display::fmt(refusal, f),
            Self::Unimplemented(missing) => fmt::Display::fmt(missing, f),
        }
    }
}

impl std::error::Error for GuardError {}

impl From<Unimplemented> for GuardError {
    fn from(missing: Unimplemented) -> Self {
        Self::Unimplemented(missing)
    }
}

impl From<Refusal> for GuardError {
    fn from(refusal: Refusal) -> Self {
        Self::Refused(refusal)
    }
}

impl From<StoreError> for GuardError {
    fn from(error: StoreError) -> Self {
        if let StoreError::Unimplemented(missing) = error {
            return missing.into();
        }
        Self::Internal(error.to_string())
    }
}

impl From<SeamError> for GuardError {
    fn from(error: SeamError) -> Self {
        match error {
            SeamError::Refused(refusal) => refusal.into(),
            SeamError::Unimplemented(missing) => missing.into(),
            unknown => Self::Internal(unknown.to_string()),
        }
    }
}

/// A hook's or route's answer: the detail of an internal failure never leaves the server.
impl From<GuardError> for SeamError {
    fn from(error: GuardError) -> Self {
        let refusal = match error {
            GuardError::Unimplemented(missing) => return missing.into(),
            GuardError::Refused(refusal) => refusal,
            GuardError::Internal(_) => Refusal {
                code: INTERNAL_CODE.into(),
                message: INTERNAL_MESSAGE.into(),
            },
        };
        refusal.into()
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

/// Any code not in the table is a 500.
pub fn status_of(code: &str) -> u16 {
    let known = ERROR_STATUSES
        .iter()
        .find_map(|&(name, status)| (name == code).then_some(status));
    known.unwrap_or(500)
}

/// The launcher's error envelope.
pub fn envelope(refusal: &Refusal, request_id: &str) -> Reply {
    let (code, message, id) = (
        js_string(&refusal.code),
        js_string(&refusal.message),
        js_string(request_id),
    );
    let body = format!(r#"{{"error":{{"code":{code},"message":{message},"requestId":{id}}}}}"#);
    Reply {
        body: Json(body),
        status: status_of(&refusal.code),
    }
}

#[cfg(test)]
mod tests {
    use rustic_rifts_seam::Component;

    use super::*;

    #[test]
    fn statuses() {
        assert_eq!(ERROR_STATUSES.len(), 21);
        let spot_checks = [
            ("GUARD_TRAFFIC_RATE_LIMITED", 429),
            ("GUARD_BATCH_TOO_LARGE", 413),
            ("P2P_CAPABILITY_INVALID", 401),
            ("HUNT_REWARDS_UNAVAILABLE", 404),
            ("AUTH_UNAUTHORIZED", 500),
            ("INTERNAL", 500),
        ];
        for (code, status) in spot_checks.iter().chain(ERROR_STATUSES) {
            assert_eq!(status_of(code), *status, "{code}");
        }
    }

    #[test]
    fn conversions_and_internal_mapping() {
        let missing = Unimplemented::new(Component::Guard, "store.chain");
        let from_store = [
            (StoreError::Unimplemented(missing), GuardError::Unimplemented(missing)),
            (StoreError::Database("disk".into()), GuardError::Internal("disk".into())),
        ];
        for (store, guard) in from_store {
            assert_eq!(GuardError::from(store), guard);
        }
        assert_eq!(
            GuardError::from(SeamError::from(missing)),
            GuardError::Unimplemented(missing)
        );

        let refused = GuardError::refused("GUARD_BATCH_INVALID", "No.");
        assert_eq!(
            (refused.code(), refused.to_string()),
            (Some("GUARD_BATCH_INVALID"), "GUARD_BATCH_INVALID: No.".into())
        );
        let internal = GuardError::Internal("x".into());
        assert_eq!((internal.code(), internal.to_string()), (None, "x".into()));

        let hidden = SeamError::from(GuardError::Internal("secret detail".into()));
        let public = Refusal {
            code: "INTERNAL".into(),
            message: "The player-hosted hunt request failed.".into(),
        };
        assert_eq!(hidden, SeamError::Refused(public));
    }

    #[test]
    fn envelope_text() {
        let refusal = Refusal {
            code: "GUARD_CHAIN_FROZEN".into(),
            message: "Closed \"now\".".into(),
        };
        let Reply { status, body } = envelope(&refusal, "req-1");
        assert_eq!(status, 409);
        let want = r#"{"error":{"code":"GUARD_CHAIN_FROZEN","message":"Closed \"now\".","requestId":"req-1"}}"#;
        assert_eq!(body.0, want);
    }
}
