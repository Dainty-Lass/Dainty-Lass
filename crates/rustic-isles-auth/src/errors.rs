// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The launcher error envelope and its status table (spec §2.3).

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value};

use crate::util;

/// A launcher route's refusal: `{"error":{<details>,"code","message","requestId"}}`.
#[derive(Debug, Clone, PartialEq)]
pub struct LauncherError {
    pub code: String,
    pub message: String,
    /// The error's own status; `None` takes the table's
    pub status: Option<u16>,
    pub details: Vec<(String, Value)>,
}

impl LauncherError {
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

    pub fn internal() -> Self {
        Self::new("INTERNAL", "Something went wrong. Please try again.")
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::new("AUTH_VALIDATION_FAILED", message)
    }

    pub fn status(&self) -> u16 {
        self.status.unwrap_or_else(|| status_for(&self.code))
    }

    /// The envelope; a detail named like a fixed field keeps its place and takes the fixed value.
    pub fn body(&self, request_id: &str) -> Value {
        let mut error = Map::new();
        for (name, value) in &self.details {
            error.insert(name.clone(), value.clone());
        }
        error.insert("code".into(), Value::String(self.code.clone()));
        error.insert("message".into(), Value::String(self.message.clone()));
        error.insert("requestId".into(), Value::String(request_id.to_owned()));
        let mut outer = Map::new();
        outer.insert("error".into(), Value::Object(error));
        Value::Object(outer)
    }

    pub fn response(&self, request_id: &str) -> Response {
        let status = StatusCode::from_u16(self.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        util::json(status, self.body(request_id))
    }
}

impl std::fmt::Display for LauncherError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for LauncherError {}

impl IntoResponse for LauncherError {
    fn into_response(self) -> Response {
        self.response(&util::random_uuid())
    }
}

/// The status a code answers with when the error carries none; a code not in the table is 500.
pub fn status_for(code: &str) -> u16 {
    match code {
        "AUTH_VALIDATION_FAILED"
        | "CONSENT_REQUIRED"
        | "CONSENT_AGE_REQUIRED"
        | "DELETION_CONFIRMATION_MISMATCH"
        | "AUTH_DISCORD_CANCELLED"
        | "GAME_BUILD_UNSUPPORTED"
        | "AUTH_CODE_INVALID"
        | "GUARD_REPORT_INVALID"
        | "NOTICES_INVALID"
        | "P2P_TICKET_INVALID"
        | "P2P_SIGNAL_INVALID"
        | "GUARD_BATCH_INVALID"
        | "REALM_VALIDATION_FAILED"
        | "REALM_NAME_INVALID"
        | "REALM_NAME_RESERVED"
        | "REALM_INVALID_TARGET" => 400,
        "AUTH_INVALID_CREDENTIALS"
        | "AUTH_REFRESH_INVALID"
        | "AUTH_UNAUTHORIZED"
        | "GUARD_SESSION_INVALID"
        | "GUARD_SIGNATURE_INVALID"
        | "P2P_CAPABILITY_INVALID" => 401,
        "AUTH_ACCOUNT_DISABLED"
        | "AUTH_ACCOUNT_BANNED"
        | "AUTH_APPROVAL_PENDING"
        | "AUTH_APPROVAL_REJECTED"
        | "AUTH_USERNAME_REQUIRED"
        | "AUTH_EMAIL_UNVERIFIED"
        | "AUTH_DISCORD_NOT_LINKED"
        | "AUTH_TESTER_REQUIRED"
        | "ACCOUNT_DEACTIVATED"
        | "LAUNCHER_UNOFFICIAL"
        | "P2P_NOT_ROSTERED"
        | "GUARD_REQUIRED"
        | "GUARD_SHADOW_INVALID"
        | "REALM_BANNED"
        | "REALM_INVITE_REQUIRED" => 403,
        "NOT_FOUND"
        | "P2P_SESSION_NOT_FOUND"
        | "HUNT_EVIDENCE_NOT_COLLECTED"
        | "HUNT_REWARDS_UNAVAILABLE"
        | "REALM_NOT_FOUND"
        | "REALM_NOT_MEMBER"
        | "REALM_INVITE_INVALID"
        | "REALM_REQUEST_NOT_FOUND"
        | "REALM_PLAYER_NOT_FOUND"
        | "REALM_PLAYER_INVITE_NOT_FOUND" => 404,
        "AUTH_EMAIL_TAKEN"
        | "AUTH_DISPLAY_NAME_TAKEN"
        | "AUTH_DISCORD_ALREADY_LINKED"
        | "AUTH_DISCORD_ACCOUNT_HAS_LINK"
        | "AUTH_DISCORD_ONLY_LOGIN"
        | "AUTH_CHANNEL_MISMATCH"
        | "LEGAL_VERSION_OUTDATED"
        | "DELETION_HOLD_EXPIRED"
        | "RUNTIME_BUILD_UNSUPPORTED"
        | "GUARD_MANIFEST_MISMATCH"
        | "GUARD_SEQUENCE_CONFLICT"
        | "NOTICES_VERSION_CONFLICT"
        | "P2P_SESSION_NOT_READY"
        | "P2P_STEAM_LINK_REQUIRED"
        | "P2P_TICKET_REPLAYED"
        | "P2P_GENERATION_FENCED"
        | "P2P_CONFLICT"
        | "P2P_TRANSPORT_MISMATCH"
        | "REALM_HUB_EXISTS"
        | "REALM_HOST_INELIGIBLE"
        | "REALM_TRANSPORT_UNSUPPORTED"
        | "REALM_HOST_OFFLINE"
        | "REALM_HUB_FULL"
        | "REALM_HUNT_RUNTIME_INCOMPATIBLE"
        | "P2P_HOST_PORTS_EXHAUSTED"
        | "GUARD_BATCH_REPLAY"
        | "GUARD_EVIDENCE_LIMIT"
        | "GUARD_CHAIN_CONFLICT"
        | "GUARD_CHAIN_FROZEN"
        | "GUARD_COMPLETION_INVALID"
        | "GUARD_COMPLETION_CONFLICT"
        | "GUARD_COMPLETION_PENDING"
        | "REALM_NAME_TAKEN"
        | "REALM_ALREADY_EXISTS"
        | "REALM_USERNAME_REQUIRED"
        | "REALM_NAME_IS_USERNAME"
        | "REALM_INVITE_LIMIT"
        | "REALM_FULL"
        | "REALM_OWNER_CHANGED"
        | "REALM_HOST_CANNOT_LEAVE"
        | "REALM_JOIN_REQUESTED"
        | "REALM_REQUESTS_FULL"
        | "REALM_PLAYER_INVITES_FULL"
        | "REALM_OFFLINE"
        | "REALM_SOLO" => 409,
        "GAME_EXCHANGE_CODE_EXPIRED" | "P2P_TICKET_EXPIRED" | "REALM_INVITE_EXPIRED" => 410,
        "GUARD_BATCH_TOO_LARGE" => 413,
        "AUTH_RATE_LIMITED" | "P2P_RELAY_RATE_LIMITED" | "P2P_SIGNAL_LIMIT" | "GUARD_TRAFFIC_RATE_LIMITED" => 429,
        "NOT_IMPLEMENTED" | "REALMS_DISABLED" => 501,
        "AUTH_DISCORD_NOT_CONFIGURED"
        | "AUTH_EMAIL_UNAVAILABLE"
        | "GUARD_MANIFEST_UNAVAILABLE"
        | "OFFICIAL_PLAY_UNAVAILABLE"
        | "P2P_DISABLED"
        | "P2P_EMERGENCY_STOP"
        | "P2P_ICE_UNAVAILABLE"
        | "REALM_UNAVAILABLE"
        | "GUARD_REQUEST_BUDGET_EXHAUSTED"
        | "GUARD_TRAFFIC_UNAVAILABLE"
        | "GUARD_FINALIZATION_BUSY"
        | "REWARD_CAPACITY_RECONCILIATION_REQUIRED" => 503,
        _ => 500,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn statuses_come_from_the_table_unless_the_error_has_one() {
        assert_eq!(LauncherError::new("AUTH_RATE_LIMITED", "x").status(), 429);
        assert_eq!(LauncherError::new("REALMS_DISABLED", "x").status(), 501);
        assert_eq!(LauncherError::new("SOMETHING_ELSE", "x").status(), 500);
        assert_eq!(LauncherError::new("AUTH_ACCOUNT_TIMED_OUT", "x").status(), 500);
        assert_eq!(
            LauncherError::new("AUTH_RATE_LIMITED", "x").with_status(418).status(),
            418
        );
    }

    #[test]
    fn details_come_first_and_never_replace_the_fixed_fields() {
        let error = LauncherError::new("ACCOUNT_DEACTIVATED", "gone")
            .with_detail("scheduledFor", json!("2026-11-01T00:00:00.000Z"))
            .with_detail("code", json!("mine"));
        assert_eq!(
            service_kit::jsjson::stringify(&error.body("rid")),
            r#"{"error":{"scheduledFor":"2026-11-01T00:00:00.000Z","code":"ACCOUNT_DEACTIVATED","message":"gone","requestId":"rid"}}"#
        );
    }
}
