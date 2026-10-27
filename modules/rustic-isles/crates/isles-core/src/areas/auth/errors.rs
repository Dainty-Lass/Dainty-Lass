//! `security/launcherErrors.ts`: the launcher's error envelope and the status each code answers with. The
//! extension codes (`p2pErrors.ts`, `realmErrors.ts`) are in the table too, since rustic-rifts refuses with them.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use service_kit::json::Json;

/// Core codes, then the player-hosted hunt module's, then the realms module's. A code not here answers 500.
const STATUSES: &[(&str, u16)] = &[
    ("AUTH_VALIDATION_FAILED", 400),
    ("AUTH_INVALID_CREDENTIALS", 401),
    ("AUTH_EMAIL_TAKEN", 409),
    ("AUTH_DISPLAY_NAME_TAKEN", 409),
    ("AUTH_ACCOUNT_DISABLED", 403),
    ("AUTH_ACCOUNT_BANNED", 403),
    ("AUTH_APPROVAL_PENDING", 403),
    ("AUTH_APPROVAL_REJECTED", 403),
    ("AUTH_USERNAME_REQUIRED", 403),
    ("AUTH_REFRESH_INVALID", 401),
    ("AUTH_UNAUTHORIZED", 401),
    ("AUTH_RATE_LIMITED", 429),
    ("AUTH_DISCORD_NOT_CONFIGURED", 503),
    ("AUTH_DISCORD_CANCELLED", 400),
    ("AUTH_DISCORD_ALREADY_LINKED", 409),
    ("AUTH_EMAIL_UNVERIFIED", 403),
    ("AUTH_CODE_INVALID", 400),
    ("AUTH_EMAIL_UNAVAILABLE", 503),
    ("AUTH_DISCORD_NOT_LINKED", 403),
    ("AUTH_DISCORD_ACCOUNT_HAS_LINK", 409),
    ("AUTH_DISCORD_ONLY_LOGIN", 409),
    ("AUTH_TESTER_REQUIRED", 403),
    ("AUTH_CHANNEL_MISMATCH", 409),
    ("CONSENT_REQUIRED", 400),
    ("CONSENT_AGE_REQUIRED", 400),
    ("LEGAL_VERSION_OUTDATED", 409),
    ("DELETION_CONFIRMATION_MISMATCH", 400),
    ("DELETION_HOLD_EXPIRED", 409),
    ("ACCOUNT_DEACTIVATED", 403),
    ("GAME_EXCHANGE_CODE_EXPIRED", 410),
    ("GAME_BUILD_UNSUPPORTED", 400),
    ("RUNTIME_BUILD_UNSUPPORTED", 409),
    ("GUARD_MANIFEST_UNAVAILABLE", 503),
    ("GUARD_MANIFEST_MISMATCH", 409),
    ("GUARD_REPORT_INVALID", 400),
    ("GUARD_SESSION_INVALID", 401),
    ("GUARD_SIGNATURE_INVALID", 401),
    ("GUARD_SEQUENCE_CONFLICT", 409),
    ("OFFICIAL_PLAY_UNAVAILABLE", 503),
    ("LAUNCHER_UNOFFICIAL", 403),
    ("NOTICES_INVALID", 400),
    ("NOTICES_VERSION_CONFLICT", 409),
    ("NOT_FOUND", 404),
    ("INTERNAL", 500),
    // isles-core's own answer for a route nothing has ported (http.rs) and for a rifts stub
    ("NOT_IMPLEMENTED", 501),
    // p2p/security/p2pErrors.ts
    ("P2P_DISABLED", 503),
    ("P2P_EMERGENCY_STOP", 503),
    ("P2P_SESSION_NOT_FOUND", 404),
    ("P2P_SESSION_NOT_READY", 409),
    ("P2P_NOT_ROSTERED", 403),
    ("P2P_STEAM_LINK_REQUIRED", 409),
    ("P2P_TICKET_INVALID", 400),
    ("P2P_TICKET_EXPIRED", 410),
    ("P2P_TICKET_REPLAYED", 409),
    ("P2P_CAPABILITY_INVALID", 401),
    ("P2P_GENERATION_FENCED", 409),
    ("P2P_CONFLICT", 409),
    ("P2P_TRANSPORT_MISMATCH", 409),
    ("P2P_ICE_UNAVAILABLE", 503),
    ("P2P_RELAY_RATE_LIMITED", 429),
    ("P2P_SIGNAL_INVALID", 400),
    ("P2P_SIGNAL_LIMIT", 429),
    ("REALM_HUB_EXISTS", 409),
    ("REALM_HOST_INELIGIBLE", 409),
    ("REALM_TRANSPORT_UNSUPPORTED", 409),
    ("REALM_HOST_OFFLINE", 409),
    ("REALM_HUB_FULL", 409),
    ("REALM_HUNT_RUNTIME_INCOMPATIBLE", 409),
    ("REALM_UNAVAILABLE", 503),
    ("P2P_HOST_PORTS_EXHAUSTED", 409),
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
    // realms/realmErrors.ts
    ("REALM_VALIDATION_FAILED", 400),
    ("REALM_NAME_INVALID", 400),
    ("REALM_NAME_RESERVED", 400),
    ("REALM_NAME_TAKEN", 409),
    ("REALM_ALREADY_EXISTS", 409),
    ("REALM_USERNAME_REQUIRED", 409),
    ("REALM_NAME_IS_USERNAME", 409),
    ("REALM_NOT_FOUND", 404),
    ("REALM_NOT_MEMBER", 404),
    ("REALM_BANNED", 403),
    ("REALM_INVITE_REQUIRED", 403),
    ("REALM_INVITE_INVALID", 404),
    ("REALM_INVITE_EXPIRED", 410),
    ("REALM_INVITE_LIMIT", 409),
    ("REALM_FULL", 409),
    ("REALM_OWNER_CHANGED", 409),
    ("REALM_INVALID_TARGET", 400),
    ("REALM_HOST_CANNOT_LEAVE", 409),
    ("REALM_JOIN_REQUESTED", 409),
    ("REALM_REQUESTS_FULL", 409),
    ("REALM_REQUEST_NOT_FOUND", 404),
    ("REALM_PLAYER_NOT_FOUND", 404),
    ("REALM_PLAYER_INVITES_FULL", 409),
    ("REALM_PLAYER_INVITE_NOT_FOUND", 404),
    ("REALM_OFFLINE", 409),
    ("REALM_SOLO", 409),
    ("REALMS_DISABLED", 501),
];

/// The status a code answers with, 500 for one nothing registered.
pub fn status_for(code: &str) -> u16 {
    STATUSES
        .iter()
        .find(|(known, _)| *known == code)
        .map_or(500, |(_, status)| *status)
}

/// `LauncherApiError`: a code, a message, and optionally its own status and fields beside them.
#[derive(Debug, Clone, PartialEq)]
pub struct LauncherApiError {
    pub code: String,
    pub message: String,
    /// One code may be refused with different statuses (CONSENT_REQUIRED: 400 at sign-up, 403 at play)
    pub status: Option<u16>,
    /// Extra fields of the error object (`scheduledFor`...), which come first, so no detail can replace the code
    pub details: Vec<(String, Value)>,
}

impl LauncherApiError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
            status: None,
            details: Vec::new(),
        }
    }

    pub fn with_status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_detail(mut self, name: &str, value: Value) -> Self {
        self.details.push((name.to_owned(), value));
        self
    }

    pub fn status(&self) -> u16 {
        self.status.unwrap_or_else(|| status_for(&self.code))
    }

    /// `{error: {...details, code, message, requestId}}`
    pub fn body(&self, request_id: &str) -> Value {
        let mut error = Map::new();
        for (name, value) in &self.details {
            error.insert(name.clone(), value.clone());
        }
        // The fixed fields are written last, so a detail can never replace them (a repeated key keeps its place, as in JS)
        error.insert("code".into(), json!(self.code));
        error.insert("message".into(), json!(self.message));
        error.insert("requestId".into(), json!(request_id));
        json!({ "error": error })
    }
}

impl std::fmt::Display for LauncherApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for LauncherApiError {}

/// `SendLauncherError`. Its request id is a fresh UUID, as Node's is (not the `x-mysticparadox-request-id`).
impl IntoResponse for LauncherApiError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, Json(self.body(&uuid::Uuid::new_v4().to_string()))).into_response()
    }
}

/// What Node's route wrapper answers for anything it did not expect.
pub fn internal() -> LauncherApiError {
    LauncherApiError::new("INTERNAL", "Something went wrong. Please try again.")
}

/// A refused or failed store call, logged and answered as `INTERNAL`.
impl From<crate::repo::RepoError> for LauncherApiError {
    fn from(error: crate::repo::RepoError) -> Self {
        tracing::error!("auth: {error}");
        internal()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_follow_the_table() {
        assert_eq!(status_for("AUTH_RATE_LIMITED"), 429);
        assert_eq!(status_for("REALMS_DISABLED"), 501);
        assert_eq!(status_for("NOT_A_CODE"), 500);
        assert_eq!(
            LauncherApiError::new("CONSENT_REQUIRED", "x").with_status(403).status(),
            403
        );
    }

    #[test]
    fn details_come_first_and_never_replace_the_fixed_fields() {
        let error = LauncherApiError::new("ACCOUNT_DEACTIVATED", "gone")
            .with_detail("scheduledFor", json!("2026-11-01T00:00:00.000Z"))
            .with_detail("code", json!("EVIL"));
        let body = serde_json::to_string(&error.body("rid")).unwrap();
        assert_eq!(
            body,
            r#"{"error":{"scheduledFor":"2026-11-01T00:00:00.000Z","code":"ACCOUNT_DEACTIVATED","message":"gone","requestId":"rid"}}"#
        );
    }
}
