//! The launcher error codes Guard answers with, and the statuses `p2pErrors.ts` registers for them.

use rifts_seam::{Refusal, SeamError, StoreError, Unimplemented};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum GuardError {
    #[error(transparent)]
    Unimplemented(#[from] Unimplemented),
    /// A `LauncherApiError`: isles writes the envelope with the status in [`ERROR_STATUSES`].
    #[error(transparent)]
    Refused(#[from] Refusal),
    /// Anything else (a storage failure): Node's `INTERNAL`, logged with the request id.
    #[error("{0}")]
    Internal(String),
}

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
            StoreError::Unimplemented(unimplemented) => Self::Unimplemented(unimplemented),
            other => Self::Internal(other.to_string()),
        }
    }
}

impl From<SeamError> for GuardError {
    fn from(error: SeamError) -> Self {
        match error {
            SeamError::Unimplemented(unimplemented) => Self::Unimplemented(unimplemented),
            SeamError::Refused(refusal) => Self::Refused(refusal),
            other => Self::Internal(other.to_string()),
        }
    }
}

/// What a hook or route answers for a [`GuardError`]; `Internal` becomes Node's catch-all `INTERNAL`.
pub fn to_seam(error: GuardError) -> SeamError {
    match error {
        GuardError::Unimplemented(unimplemented) => SeamError::Unimplemented(unimplemented),
        GuardError::Refused(refusal) => SeamError::Refused(refusal),
        GuardError::Internal(_) => SeamError::Refused(Refusal {
            code: "INTERNAL".to_owned(),
            message: "The player-hosted hunt request failed.".to_owned(),
        }),
    }
}

pub fn store_to_seam(error: StoreError) -> SeamError {
    to_seam(error.into())
}

/// The codes Guard throws, with the status `RegisterLauncherErrorStatuses` gives each (`p2p/security/p2pErrors.ts`).
/// The `P2P_*` ones are registered by realms too, with the same statuses.
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

/// The status isles sends for `code` (500 for one Guard does not register, as `SendLauncherError` does).
pub fn status_of(code: &str) -> u16 {
    ERROR_STATUSES
        .iter()
        .find(|(known, _)| *known == code)
        .map_or(500, |(_, status)| *status)
}

pub fn not_collected() -> GuardError {
    GuardError::refused(
        "HUNT_EVIDENCE_NOT_COLLECTED",
        "This server does not collect hunt evidence.",
    )
}

pub fn fenced() -> GuardError {
    GuardError::refused(
        "P2P_GENERATION_FENCED",
        "The hunt generation no longer belongs to this host.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rifts_seam::Component;

    #[test]
    fn statuses_match_p2p_errors() {
        assert_eq!(status_of("HUNT_EVIDENCE_NOT_COLLECTED"), 404);
        assert_eq!(status_of("GUARD_BATCH_TOO_LARGE"), 413);
        assert_eq!(status_of("GUARD_TRAFFIC_RATE_LIMITED"), 429);
        assert_eq!(status_of("UNKNOWN"), 500);
    }

    #[test]
    fn internal_errors_reach_the_client_as_internal() {
        assert_eq!(
            to_seam(GuardError::Internal("disk on fire".to_owned())),
            SeamError::Refused(Refusal {
                code: "INTERNAL".to_owned(),
                message: "The player-hosted hunt request failed.".to_owned()
            })
        );
        let unimplemented = Unimplemented::new(Component::Guard, "x");
        assert_eq!(
            to_seam(StoreError::Unimplemented(unimplemented).into()),
            SeamError::Unimplemented(unimplemented)
        );
    }
}
