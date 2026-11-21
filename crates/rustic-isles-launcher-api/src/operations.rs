// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The operations policy (public spec §9): Guard enforcement and the diagnostics profile, its two admin routes, and
//! the reads auth's `Operations` seam and rustic-rifts' adapter make.

use std::sync::Arc;

use axum::response::Response;
use http::StatusCode;
use rustic_isles::{Settings, Shared};
use rustic_isles_store::repo::reforged::{AdminAuditRecord, OperationsPolicyRecord};
use serde_json::Value;

use crate::admin::{AdminContext, admin_actor, admin_error, fresh_id, user_id_of};
use crate::util::{self, MAX_SAFE, iso, json, now_ms, object, system_now_ms};

const STEP_UP_WINDOW_MS: i64 = 15 * 60_000;
const LEASE_MS: i64 = 2 * 3_600_000;

/// The operations policy (§9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    /// `ENFORCE` or `OBSERVE`.
    pub guard_enforcement: String,
    /// `PRODUCTION` or `DEVELOPMENT`.
    pub diagnostics_profile: String,
    /// Bumped by each change; 0 is the untouched default.
    pub version: u64,
    /// When it last changed, ISO text.
    pub updated_at: Option<String>,
    /// Who changed it.
    pub updated_by: Option<String>,
    /// When `DEVELOPMENT` reverts to `PRODUCTION`, ISO text.
    pub diagnostics_expires_at: Option<String>,
}

/// The untouched policy: `ENFORCE`, `PRODUCTION`, version 0, never changed.
pub fn default_policy() -> Policy {
    Policy {
        guard_enforcement: "ENFORCE".into(),
        diagnostics_profile: "PRODUCTION".into(),
        version: 0,
        updated_at: None,
        updated_by: None,
        diagnostics_expires_at: None,
    }
}

impl Policy {
    /// The policy as the dashboard reads it.
    pub fn to_json(&self) -> Value {
        let mut pairs = vec![
            ("guardEnforcement", Value::from(self.guard_enforcement.clone())),
            ("diagnosticsProfile", Value::from(self.diagnostics_profile.clone())),
            ("version", Value::from(self.version)),
        ];
        for (name, value) in [
            ("updatedAt", &self.updated_at),
            ("updatedBy", &self.updated_by),
            ("diagnosticsExpiresAt", &self.diagnostics_expires_at),
        ] {
            if let Some(value) = value {
                pairs.push((name, Value::from(value.clone())));
            }
        }
        object(pairs)
    }
}

fn lease_running(expires: Option<&str>, now_ms: i64) -> bool {
    expires
        .and_then(dainty_service_kit::js::date_parse)
        .is_some_and(|at| at > now_ms)
}

/// A stored record read at `now`; the second value is whether it was malformed.
pub fn normalize(record: &OperationsPolicyRecord, now_ms: i64) -> (Policy, bool) {
    let malformed = !matches!(record.guard_enforcement.as_str(), "OBSERVE" | "ENFORCE")
        || !matches!(record.diagnostics_profile.as_str(), "PRODUCTION" | "DEVELOPMENT");
    let observe = !malformed && record.guard_enforcement == "OBSERVE";
    let development = !malformed
        && record.diagnostics_profile == "DEVELOPMENT"
        && lease_running(record.diagnostics_expires_at.as_deref(), now_ms);
    let policy = Policy {
        guard_enforcement: if observe { "OBSERVE" } else { "ENFORCE" }.into(),
        diagnostics_profile: if development { "DEVELOPMENT" } else { "PRODUCTION" }.into(),
        version: record.version,
        updated_at: record.updated_at.clone(),
        updated_by: record.updated_by.clone(),
        diagnostics_expires_at: if development {
            record.diagnostics_expires_at.clone()
        } else {
            None
        },
    };
    (policy, malformed)
}

/// The record (or the default), with the host's override.
pub fn resolve(settings: &Settings, record: Option<&OperationsPolicyRecord>, now_ms: i64) -> Policy {
    let mut policy = match record {
        None => default_policy(),
        Some(record) => {
            let (policy, malformed) = normalize(record, now_ms);
            if malformed {
                tracing::error!(
                    "[OperationsPolicy] malformed global policy; forcing Guard ENFORCE and production diagnostics"
                );
            }
            policy
        }
    };
    if settings.raw("GUARD_ENFORCEMENT_OVERRIDE") == Some("OBSERVE") {
        policy.guard_enforcement = "OBSERVE".into();
    }
    policy
}

/// The codes a change from `before` to the request would weaken, in order.
pub fn weakening_codes(before: &Policy, guard: &str, profile: &str) -> Vec<&'static str> {
    let mut codes = Vec::new();
    if before.guard_enforcement == "ENFORCE" && guard == "OBSERVE" {
        codes.push("GUARD_DOWNGRADE");
    }
    if before.diagnostics_profile == "PRODUCTION" && profile == "DEVELOPMENT" {
        codes.push("DEVELOPMENT_DIAGNOSTICS");
    }
    codes
}

/// A running development lease is kept; a new one lasts two hours.
pub fn diagnostics_expiry(current: &Policy, requested_profile: &str, now_ms: i64) -> Option<String> {
    if requested_profile == "PRODUCTION" {
        return None;
    }
    if current.diagnostics_profile == "DEVELOPMENT" && lease_running(current.diagnostics_expires_at.as_deref(), now_ms)
    {
        return current.diagnostics_expires_at.clone();
    }
    Some(iso(now_ms + LEASE_MS))
}

/// The policy now; none without a store or on a read error.
pub async fn current_policy(shared: &Shared) -> Option<Policy> {
    let store = util::store_of(shared)?;
    let record = store
        .read(|conn, repos| repos.admin.read_operations_policy(conn))
        .await
        .ok()?;
    Some(resolve(shared.settings(), record.as_ref(), now_ms()))
}

/// rustic-rifts' adapter: the current policy (or the default) as JSON text.
pub async fn operations_policy_json(shared: &Shared) -> String {
    let policy = current_policy(shared).await.unwrap_or_else(default_policy);
    dainty_service_kit::jsjson::stringify(&policy.to_json())
}

/// The break-glass code: the window not exhausted, a secret set, and auth's TOTP check.
pub fn verify_operations_step_up(shared: &Shared, key_id: &str, code: Option<&Value>) -> bool {
    let key = format!("admin-operations-step-up:{key_id}");
    if shared
        .auth()
        .limiter()
        .is_limited_at(&key, 6, STEP_UP_WINDOW_MS, now_ms())
    {
        return false;
    }
    let settings = shared.settings();
    let secret = ["ADMIN_OPERATIONS_TOTP_SECRET", "ADMIN_TOTP_SECRET"]
        .iter()
        .map(|name| util::trim(settings.raw(name).unwrap_or("")))
        .find(|s| !s.is_empty());
    secret.is_some_and(|s| rustic_isles_auth::verify_totp(s, code, system_now_ms()))
}

// ------------------------------------------------------------------ routes (§9.2, §9.3)

fn session_json(
    conn: &mut rustic_isles_store::AnyConnection,
    repos: &rustic_isles_store::Repositories,
    s: &rustic_isles_store::repo::reforged::LauncherGuardSessionRecord,
) -> Value {
    let mut pairs = vec![
        ("accountId", Value::from(user_id_of(conn, repos, &s.account))),
        ("launcherVersion", Value::from(s.launcher_version.clone())),
        ("manifestSequence", Value::from(s.manifest_sequence)),
        ("status", Value::from(s.status.as_str())),
        ("riskScore", util::number(s.risk_score)),
    ];
    if let Some(at) = &s.last_heartbeat_at {
        pairs.push(("lastHeartbeatAt", Value::from(at.clone())));
    }
    pairs.push(("expiresAt", Value::from(s.expires_at.clone())));
    if let Some(signals) = &s.last_signals {
        pairs.push(("lastSignals", signals.clone()));
    }
    pairs.push(("guardSessionId", Value::from(s.guard_session_id.clone())));
    object(pairs)
}

/// `GET /admin/v1/operations/overview`: the policy and recent Guard sessions.
pub async fn overview(ctx: AdminContext) -> Response {
    let read = ctx
        .store
        .read(|conn, repos| {
            let record = repos.admin.read_operations_policy(conn)?;
            let (sessions, _) = repos.launcher_guard.list_sessions(conn, None, None, 0, 100)?;
            let list: Vec<Value> = sessions.iter().map(|s| session_json(conn, repos, s)).collect();
            Ok((record, list))
        })
        .await;
    match read {
        Ok((record, list)) => {
            let now = now_ms();
            let policy = resolve(ctx.shared.settings(), record.as_ref(), now);
            util::ok(&object(vec![
                ("observedAt", Value::from(iso(now))),
                ("policy", policy.to_json()),
                ("launcherGuard", Value::Array(list)),
            ]))
        }
        Err(e) => {
            tracing::error!("[ADMIN] operations overview: {e}");
            util::empty(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

enum Changed {
    Conflict(Policy),
    StepUp(Policy, Vec<&'static str>),
    Updated(Policy),
}

fn whole_version(value: Option<&Value>) -> Option<u64> {
    let n = dainty_service_kit::js::number(value);
    (n.is_finite() && n.fract() == 0.0 && (0.0..=MAX_SAFE).contains(&n)).then(|| {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "whole and inside 2^53"
        )]
        let v = n as u64;
        v
    })
}

/// `PATCH /admin/v1/operations/policy`: changes the policy, refusing a stale version.
pub async fn update_policy(ctx: AdminContext) -> Response {
    let request = &ctx.request;
    let guard = request
        .field("guardEnforcement")
        .and_then(Value::as_str)
        .filter(|g| matches!(*g, "OBSERVE" | "ENFORCE"))
        .map(str::to_owned);
    let profile = request
        .field("diagnosticsProfile")
        .and_then(Value::as_str)
        .filter(|p| matches!(*p, "PRODUCTION" | "DEVELOPMENT"))
        .map(str::to_owned);
    let expected = whole_version(request.field("expectedVersion"));
    let reason = request
        .field("reason")
        .and_then(Value::as_str)
        .map(|r| util::take_chars(util::trim(r), 500))
        .filter(|r| util::utf16_len(r) >= 8);
    let (Some(guard), Some(profile), Some(expected), Some(reason)) = (guard, profile, expected, reason) else {
        return admin_error(
            StatusCode::BAD_REQUEST,
            "ADMIN_VALIDATION",
            "A complete versioned policy and an audit reason of at least 8 characters are required.",
        );
    };
    let Some(caller) = ctx.caller.clone() else {
        return crate::admin::admin_internal("the policy change had no caller");
    };
    let shared = Arc::clone(&ctx.shared);
    let code = request.field("breakGlassTotp").cloned();
    let ip = request.client_ip.clone();
    let request_id = fresh_id();
    let rid = request_id.clone();
    let operator = caller.user_id.clone();
    let changed = ctx
        .store
        .write(&admin_actor(&caller), move |tx, repos| {
            let now = now_ms();
            let at = iso(now);
            let settings = shared.settings();
            let audit = |action: &str, old: Value, new: Value| AdminAuditRecord {
                id: fresh_id(),
                actor_user_id: operator.clone(),
                target_user_id: None,
                action: action.to_owned(),
                old_state: Some(old),
                new_state: Some(new),
                reason: Some(reason.clone()),
                ip: ip.clone(),
                request_id: rid.clone(),
                created_at: at.clone(),
            };
            let stored = repos.admin.read_operations_policy(tx.conn())?;
            let before = resolve(settings, stored.as_ref(), now);
            if before.version != expected {
                return Ok(Changed::Conflict(before));
            }
            let codes = weakening_codes(&before, &guard, &profile);
            if !codes.is_empty() && !verify_operations_step_up(&shared, &operator, code.as_ref()) {
                let new = object(vec![
                    ("guardEnforcement", Value::from(guard.clone())),
                    ("diagnosticsProfile", Value::from(profile.clone())),
                    (
                        "weakeningCodes",
                        Value::Array(codes.iter().map(|c| Value::from(*c)).collect()),
                    ),
                ]);
                repos.admin.append_audit(
                    tx,
                    &audit("operations.policy.weakening_rejected", before.to_json(), new),
                )?;
                return Ok(Changed::StepUp(before, codes));
            }
            let record = OperationsPolicyRecord {
                guard_enforcement: guard.clone(),
                diagnostics_profile: profile.clone(),
                version: expected,
                updated_at: Some(at.clone()),
                updated_by: Some(operator.clone()),
                diagnostics_expires_at: diagnostics_expiry(&before, &profile, now),
            };
            if !repos.admin.update_operations_policy(tx, expected, &record)? {
                return Ok(Changed::Conflict(before));
            }
            let reread = repos.admin.read_operations_policy(tx.conn())?;
            let after = resolve(settings, reread.as_ref(), now);
            let mut new = after.to_json();
            if let Value::Object(map) = &mut new {
                map.insert(
                    "weakeningCodes".into(),
                    Value::Array(codes.iter().map(|c| Value::from(*c)).collect()),
                );
            }
            repos
                .admin
                .append_audit(tx, &audit("operations.policy.update", before.to_json(), new))?;
            Ok(Changed::Updated(after))
        })
        .await;
    match changed {
        Ok(Changed::Conflict(before)) => {
            let mut body = util::error_object(
                "ADMIN_POLICY_CONFLICT",
                "The operations policy changed. Refresh it before trying again.",
            );
            if let Value::Object(map) = &mut body {
                map.insert("policy".into(), before.to_json());
            }
            json(StatusCode::CONFLICT, &body)
        }
        Ok(Changed::StepUp(before, codes)) => {
            let mut body = util::error_object(
                "ADMIN_POLICY_STEP_UP_REQUIRED",
                "A fresh operations authenticator code is required for this safety reduction.",
            );
            if let Value::Object(map) = &mut body {
                map.insert("policy".into(), before.to_json());
                map.insert(
                    "weakeningCodes".into(),
                    Value::Array(codes.iter().map(|c| Value::from(*c)).collect()),
                );
            }
            json(StatusCode::FORBIDDEN, &body)
        }
        Ok(Changed::Updated(after)) => util::ok(&object(vec![
            ("policy", after.to_json()),
            ("requestId", Value::from(request_id)),
        ])),
        Err(e) => {
            tracing::error!("[ADMIN] operations policy: {e}");
            util::empty(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy_row(expires_at: Option<&str>, profile: &str, enforcement: &str) -> OperationsPolicyRecord {
        OperationsPolicyRecord {
            guard_enforcement: enforcement.into(),
            diagnostics_profile: profile.into(),
            version: 3,
            updated_at: Some("u".into()),
            updated_by: Some("b".into()),
            diagnostics_expires_at: expires_at.map(str::to_owned),
        }
    }

    #[test]
    fn normalizing() {
        let now = dainty_service_kit::js::date_parse("2026-10-26T12:00:00Z").unwrap();
        let (kept, bad) = normalize(
            &policy_row(Some("2026-10-26T13:00:00.000Z"), "DEVELOPMENT", "OBSERVE"),
            now,
        );
        assert!(!bad);
        assert_eq!(
            dainty_service_kit::jsjson::stringify(&kept.to_json()),
            r#"{"guardEnforcement":"OBSERVE","diagnosticsProfile":"DEVELOPMENT","version":3,"updatedAt":"u","updatedBy":"b","diagnosticsExpiresAt":"2026-10-26T13:00:00.000Z"}"#
        );
        let (lapsed, _) = normalize(
            &policy_row(Some("2026-10-26T11:00:00.000Z"), "DEVELOPMENT", "OBSERVE"),
            now,
        );
        assert_eq!(
            (lapsed.diagnostics_profile.as_str(), lapsed.diagnostics_expires_at),
            ("PRODUCTION", None)
        );
        let (none, _) = normalize(&policy_row(None, "DEVELOPMENT", "OBSERVE"), now);
        assert_eq!(none.diagnostics_profile, "PRODUCTION");
        let (maybe, bad) = normalize(
            &policy_row(Some("2026-10-26T13:00:00.000Z"), "DEVELOPMENT", "MAYBE"),
            now,
        );
        assert!(bad);
        assert_eq!(
            (
                maybe.guard_enforcement.as_str(),
                maybe.diagnostics_profile.as_str(),
                maybe.version
            ),
            ("ENFORCE", "PRODUCTION", 3)
        );
        assert_eq!(
            dainty_service_kit::jsjson::stringify(&default_policy().to_json()),
            r#"{"guardEnforcement":"ENFORCE","diagnosticsProfile":"PRODUCTION","version":0}"#
        );
        let over = resolve(
            &Settings::from_pairs([("GUARD_ENFORCEMENT_OVERRIDE", "OBSERVE")]),
            None,
            now,
        );
        assert_eq!(over.guard_enforcement, "OBSERVE");
    }

    #[test]
    fn weakening_and_leases() {
        let base = default_policy();
        assert_eq!(
            weakening_codes(&base, "OBSERVE", "DEVELOPMENT"),
            ["GUARD_DOWNGRADE", "DEVELOPMENT_DIAGNOSTICS"]
        );
        assert!(weakening_codes(&base, "ENFORCE", "PRODUCTION").is_empty());
        let mut loose = base.clone();
        loose.guard_enforcement = "OBSERVE".into();
        assert!(weakening_codes(&loose, "ENFORCE", "PRODUCTION").is_empty());
        let now = dainty_service_kit::js::date_parse("2026-10-26T12:00:00Z").unwrap();
        assert_eq!(diagnostics_expiry(&base, "PRODUCTION", now), None);
        assert_eq!(
            diagnostics_expiry(&base, "DEVELOPMENT", now).unwrap(),
            "2026-10-26T14:00:00.000Z"
        );
        let mut running = base;
        running.diagnostics_profile = "DEVELOPMENT".into();
        running.diagnostics_expires_at = Some("2026-10-26T13:00:00.000Z".into());
        assert_eq!(
            diagnostics_expiry(&running, "DEVELOPMENT", now).unwrap(),
            "2026-10-26T13:00:00.000Z"
        );
        assert_eq!(whole_version(Some(&Value::from("3"))), Some(3));
        assert_eq!(whole_version(Some(&Value::Null)), Some(0));
        assert_eq!(whole_version(None), None);
        assert_eq!(whole_version(Some(&Value::from(1.5))), None);
    }
}
