//! The operations policy (`services/operationsPolicy.ts`, `controllers/adminOperations.ts`): how Guard verdicts are
//! enforced and for how long verbose diagnostics may run, one document changed by version from the dashboard, with a
//! fresh authenticator code for anything that makes the server less safe.
//!
//! Not here: `ApplyDiagnosticsLogProfile` (the log level follows the process' own setting for now) and the optional
//! module's `operations` section of the overview, which rustic-rifts will add through its seam.

use axum::response::Response;
use serde_json::{Map, Value, json};

use super::admin::{Ctx, credentials, route, user_id_of};
use super::util::{self, date_parse, is_rate_limited};
use crate::http::{Handlers, Shared};
use crate::repo::reforged::{AdminAuditRecord, OperationsPolicyRecord};
use crate::routes::{Method, Service};

const DIAGNOSTICS_LEASE_MS: i64 = 2 * 60 * 60_000;

/// `OperationsPolicy`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    pub guard_enforcement: &'static str,
    pub diagnostics_profile: &'static str,
    pub version: u64,
    pub updated_at: Option<String>,
    pub updated_by: Option<String>,
    pub diagnostics_expires_at: Option<String>,
}

impl Policy {
    /// `DEFAULT_OPERATIONS_POLICY`: a missing singleton is a deployment fault; these cover the interval before repair.
    pub fn default_policy() -> Self {
        Self {
            guard_enforcement: "ENFORCE",
            diagnostics_profile: "PRODUCTION",
            version: 0,
            updated_at: None,
            updated_by: None,
            diagnostics_expires_at: None,
        }
    }

    /// The policy as the dashboard reads it; what Node leaves undefined is left out.
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("guardEnforcement".into(), json!(self.guard_enforcement));
        out.insert("diagnosticsProfile".into(), json!(self.diagnostics_profile));
        out.insert("version".into(), json!(self.version));
        for (key, value) in [
            ("updatedAt", &self.updated_at),
            ("updatedBy", &self.updated_by),
            ("diagnosticsExpiresAt", &self.diagnostics_expires_at),
        ] {
            if let Some(value) = value {
                out.insert(key.into(), json!(value));
            }
        }
        Value::Object(out)
    }
}

/// `OperationsPolicyWeakeningCodes`.
pub fn weakening_codes(before: &Policy, guard: &str, diagnostics: &str) -> Vec<&'static str> {
    let mut codes = Vec::new();
    if before.guard_enforcement == "ENFORCE" && guard == "OBSERVE" {
        codes.push("GUARD_DOWNGRADE");
    }
    if before.diagnostics_profile == "PRODUCTION" && diagnostics == "DEVELOPMENT" {
        codes.push("DEVELOPMENT_DIAGNOSTICS");
    }
    codes
}

/// `NormalizeOperationsPolicyRecord`: a malformed record forces Guard to ENFORCE and diagnostics to production.
pub fn normalize(record: &OperationsPolicyRecord, now: i64) -> (Policy, bool) {
    let guard_valid = matches!(record.guard_enforcement.as_str(), "OBSERVE" | "ENFORCE");
    let diagnostics_valid = matches!(record.diagnostics_profile.as_str(), "PRODUCTION" | "DEVELOPMENT");
    let malformed = !guard_valid || !diagnostics_valid;
    let expires = record.diagnostics_expires_at.clone();
    let active = !malformed
        && record.diagnostics_profile == "DEVELOPMENT"
        && expires.as_deref().and_then(date_parse).is_some_and(|at| at > now);
    let guard = if !malformed && record.guard_enforcement == "OBSERVE" {
        "OBSERVE"
    } else {
        "ENFORCE"
    };
    (
        Policy {
            guard_enforcement: guard,
            diagnostics_profile: if active { "DEVELOPMENT" } else { "PRODUCTION" },
            version: record.version,
            updated_at: record.updated_at.clone(),
            updated_by: record.updated_by.clone(),
            diagnostics_expires_at: if active { expires } else { None },
        },
        malformed,
    )
}

/// `ResolveDiagnosticsExpiry`: a lease still running is kept (so an unrelated edit can't renew verbose logging); a new
/// one is issued only when production, or an expired lease, goes back to development.
pub fn diagnostics_expiry(current: &Policy, requested: &str, now: i64) -> Option<String> {
    if requested != "DEVELOPMENT" {
        return None;
    }
    let existing = (current.diagnostics_profile == "DEVELOPMENT")
        .then(|| current.diagnostics_expires_at.clone())
        .flatten();
    if let Some(existing) = existing
        && date_parse(&existing).is_some_and(|at| at > now)
    {
        return Some(existing);
    }
    Some(util::iso(now + DIAGNOSTICS_LEASE_MS))
}

/// `ResolveOperationsPolicy` over what the store holds, with `GUARD_ENFORCEMENT_OVERRIDE`.
pub fn resolve(shared: &Shared, record: Option<&OperationsPolicyRecord>, now: i64) -> Policy {
    let Some(record) = record else {
        return Policy::default_policy();
    };
    let (mut policy, malformed) = normalize(record, now);
    if malformed {
        tracing::error!("[OperationsPolicy] malformed global policy; forcing Guard ENFORCE and production diagnostics");
    }
    if shared.settings.raw("GUARD_ENFORCEMENT_OVERRIDE") == Some("OBSERVE") {
        policy.guard_enforcement = "OBSERVE";
    }
    policy
}

/// `GetOperationsPolicy`, for the areas that judge a launch by it.
pub async fn current_policy(shared: &Shared) -> Option<Policy> {
    let store = shared.store.clone()?;
    let record = store
        .read(|conn, repos| repos.admin.read_operations_policy(conn))
        .await
        .ok()?;
    Some(resolve(shared, record.as_ref(), util::now_ms()))
}

/// `VerifyOperationsStepUp`.
fn step_up(shared: &Shared, actor_user_id: &str, code: Option<&Value>) -> bool {
    let secret = shared
        .settings
        .get("ADMIN_OPERATIONS_TOTP_SECRET")
        .map(service_kit::js::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            shared
                .settings
                .get("ADMIN_TOTP_SECRET")
                .map(service_kit::js::trim)
                .filter(|s| !s.is_empty())
        });
    let limited = is_rate_limited(&format!("admin-operations-step-up:{actor_user_id}"), 6, 15 * 60_000);
    match secret {
        Some(secret) if !limited => credentials().is_some_and(|c| c.verify_totp(secret, code)),
        _ => false,
    }
}

async fn overview(ctx: Ctx) -> Response {
    let shared = ctx.shared.clone();
    let result = ctx
        .store
        .read(move |conn, repos| {
            let record = repos.admin.read_operations_policy(conn)?;
            let (sessions, _) = repos.launcher_guard.list_sessions(conn, None, None, 0, 100)?;
            let mut guard = Vec::new();
            for session in &sessions {
                let user_id = user_id_of(conn, repos, &session.account)?;
                let mut entry = Map::new();
                entry.insert("accountId".into(), json!(user_id));
                entry.insert("launcherVersion".into(), json!(session.launcher_version));
                entry.insert("manifestSequence".into(), json!(session.manifest_sequence));
                entry.insert("status".into(), json!(session.status.as_str()));
                entry.insert("riskScore".into(), json!(session.risk_score));
                if let Some(at) = &session.last_heartbeat_at {
                    entry.insert("lastHeartbeatAt".into(), json!(at));
                }
                entry.insert("expiresAt".into(), json!(session.expires_at));
                if let Some(signals) = &session.last_signals {
                    entry.insert("lastSignals".into(), signals.clone());
                }
                entry.insert("guardSessionId".into(), json!(session.guard_session_id));
                guard.push(Value::Object(entry));
            }
            Ok((record, guard))
        })
        .await;
    match result {
        Ok((record, guard)) => {
            let policy = resolve(&shared, record.as_ref(), util::now_ms());
            util::json(
                200,
                &json!({ "observedAt": util::now_iso(), "policy": policy.to_json(), "launcherGuard": guard }),
            )
        }
        Err(error) => {
            tracing::error!("[ADMIN] operations overview: {error}");
            util::empty(500)
        }
    }
}

enum Outcome {
    Conflict(Policy),
    StepUp(Policy, Vec<&'static str>),
    Updated(Policy),
}

async fn update_policy(ctx: Ctx) -> Response {
    let caller = ctx.caller().clone();
    let fields = ctx.fields();
    // The closure that writes outlives this scope, so the choices are the static spellings
    let guard = match fields.get("guardEnforcement").and_then(Value::as_str) {
        Some("OBSERVE") => Some("OBSERVE"),
        Some("ENFORCE") => Some("ENFORCE"),
        _ => None,
    };
    let diagnostics = match fields.get("diagnosticsProfile").and_then(Value::as_str) {
        Some("PRODUCTION") => Some("PRODUCTION"),
        Some("DEVELOPMENT") => Some("DEVELOPMENT"),
        _ => None,
    };
    let expected = service_kit::js::number(fields.get("expectedVersion"));
    let expected = (expected.fract() == 0.0 && expected.abs() <= 9_007_199_254_740_991.0 && expected >= 0.0)
        .then_some(expected as u64);
    let reason: String = fields
        .get("reason")
        .and_then(Value::as_str)
        .map_or_else(String::new, |r| service_kit::js::trim(r).chars().take(500).collect());
    let (Some(guard), Some(diagnostics), Some(expected), true) =
        (guard, diagnostics, expected, reason.encode_utf16().count() >= 8)
    else {
        return super::admin::error(
            400,
            "ADMIN_VALIDATION",
            "A complete versioned policy and an audit reason of at least 8 characters are required.",
        );
    };
    let request_id = uuid::Uuid::new_v4().to_string();
    let shared = ctx.shared.clone();
    let ip = ctx.req.ip();
    let totp = fields.get("breakGlassTotp").cloned();
    let (actor_id, rid, why) = (caller.user_id.clone(), request_id.clone(), reason);
    let actor = module_api::ActorContext::new(module_api::Actor::Admin { key: caller.user_id });
    let outcome = ctx
        .store
        .write(&actor, move |tx, repos| {
            let now = util::now_ms();
            let read = |tx: &mut crate::repo::Tx<'_>| -> crate::repo::RepoResult<Policy> {
                let record = repos.admin.read_operations_policy(tx.conn())?;
                Ok(resolve(&shared, record.as_ref(), now))
            };
            let before = read(tx)?;
            if before.version != expected {
                return Ok(Outcome::Conflict(before));
            }
            let codes = weakening_codes(&before, guard, diagnostics);
            let audit = |action: &str, old: Value, new: Value| AdminAuditRecord {
                id: uuid::Uuid::new_v4().to_string(),
                actor_user_id: actor_id.clone(),
                target_user_id: None,
                action: action.to_owned(),
                old_state: Some(old),
                new_state: Some(new),
                reason: Some(why.clone()),
                ip: ip.clone(),
                request_id: rid.clone(),
                created_at: util::iso(now),
            };
            if !codes.is_empty() && !step_up(&shared, &actor_id, totp.as_ref()) {
                repos.admin.append_audit(
                    tx,
                    &audit(
                        "operations.policy.weakening_rejected",
                        before.to_json(),
                        json!({ "guardEnforcement": guard, "diagnosticsProfile": diagnostics, "weakeningCodes": codes }),
                    ),
                )?;
                return Ok(Outcome::StepUp(before, codes));
            }
            let record = OperationsPolicyRecord {
                guard_enforcement: guard.to_owned(),
                diagnostics_profile: diagnostics.to_owned(),
                version: expected,
                updated_at: Some(util::iso(now)),
                updated_by: Some(actor_id.clone()),
                diagnostics_expires_at: diagnostics_expiry(&before, diagnostics, now),
            };
            if !repos.admin.update_operations_policy(tx, expected, &record)? {
                return Ok(Outcome::Conflict(before));
            }
            let after = read(tx)?;
            let mut new_state = after.to_json();
            if let Value::Object(map) = &mut new_state {
                map.insert("weakeningCodes".into(), json!(codes));
            }
            repos.admin.append_audit(tx, &audit("operations.policy.update", before.to_json(), new_state))?;
            Ok(Outcome::Updated(after))
        })
        .await;
    match outcome {
        Ok(Outcome::Conflict(before)) => util::json(
            409,
            &json!({
                "error": { "code": "ADMIN_POLICY_CONFLICT", "message": "The operations policy changed. Refresh it before trying again." },
                "policy": before.to_json(),
            }),
        ),
        Ok(Outcome::StepUp(before, codes)) => util::json(
            403,
            &json!({
                "error": { "code": "ADMIN_POLICY_STEP_UP_REQUIRED", "message":
                    "A fresh operations authenticator code is required for this safety reduction." },
                "policy": before.to_json(),
                "weakeningCodes": codes,
            }),
        ),
        Ok(Outcome::Updated(after)) => util::json(200, &json!({ "policy": after.to_json(), "requestId": request_id })),
        Err(error) => {
            tracing::error!("[ADMIN] operations policy: {error}");
            util::empty(500)
        }
    }
}

pub fn register(handlers: &mut Handlers) {
    let b = Service::Backend;
    handlers.on(b, Method::Get, "/admin/v1/operations/overview", |call| {
        Box::pin(route(call, true, overview))
    });
    handlers.on(b, Method::Patch, "/admin/v1/operations/policy", |call| {
        Box::pin(route(call, true, update_policy))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(guard: &str, profile: &str, expires: Option<&str>) -> OperationsPolicyRecord {
        OperationsPolicyRecord {
            guard_enforcement: guard.into(),
            diagnostics_profile: profile.into(),
            version: 3,
            updated_at: Some("u".into()),
            updated_by: Some("b".into()),
            diagnostics_expires_at: expires.map(str::to_owned),
        }
    }

    // services/operationsPolicy.test.ts
    #[test]
    fn weakening_is_a_downgrade_or_verbose_diagnostics() {
        let base = Policy::default_policy();
        assert_eq!(
            weakening_codes(&base, "OBSERVE", "DEVELOPMENT"),
            ["GUARD_DOWNGRADE", "DEVELOPMENT_DIAGNOSTICS"]
        );
        assert_eq!(weakening_codes(&base, "ENFORCE", "PRODUCTION"), Vec::<&str>::new());
        let loose = Policy {
            guard_enforcement: "OBSERVE",
            diagnostics_profile: "DEVELOPMENT",
            ..base
        };
        assert!(
            weakening_codes(&loose, "OBSERVE", "DEVELOPMENT").is_empty(),
            "no change is no weakening"
        );
        assert!(
            weakening_codes(&loose, "ENFORCE", "PRODUCTION").is_empty(),
            "tightening needs no code"
        );
    }

    #[test]
    fn a_malformed_record_is_enforced_and_an_expired_lease_is_production() {
        let now = date_parse("2026-10-26T12:00:00Z").unwrap();
        let (good, malformed) = normalize(&record("OBSERVE", "DEVELOPMENT", Some("2026-10-26T13:00:00.000Z")), now);
        assert!(!malformed);
        assert_eq!(
            (good.guard_enforcement, good.diagnostics_profile),
            ("OBSERVE", "DEVELOPMENT")
        );
        assert_eq!(good.diagnostics_expires_at.as_deref(), Some("2026-10-26T13:00:00.000Z"));
        let (expired, _) = normalize(&record("OBSERVE", "DEVELOPMENT", Some("2026-10-26T11:00:00.000Z")), now);
        assert_eq!(
            (expired.diagnostics_profile, expired.diagnostics_expires_at),
            ("PRODUCTION", None)
        );
        let (no_lease, _) = normalize(&record("ENFORCE", "DEVELOPMENT", None), now);
        assert_eq!(no_lease.diagnostics_profile, "PRODUCTION");
        let (bad, malformed) = normalize(&record("MAYBE", "DEVELOPMENT", Some("2026-10-26T13:00:00.000Z")), now);
        assert!(malformed);
        assert_eq!(
            (bad.guard_enforcement, bad.diagnostics_profile),
            ("ENFORCE", "PRODUCTION")
        );
        assert_eq!(bad.version, 3);
        assert_eq!(
            good.to_json().to_string(),
            r#"{"guardEnforcement":"OBSERVE","diagnosticsProfile":"DEVELOPMENT","version":3,"updatedAt":"u","updatedBy":"b","diagnosticsExpiresAt":"2026-10-26T13:00:00.000Z"}"#
        );
        assert_eq!(
            Policy::default_policy().to_json().to_string(),
            r#"{"guardEnforcement":"ENFORCE","diagnosticsProfile":"PRODUCTION","version":0}"#
        );
    }

    #[test]
    fn a_running_diagnostics_lease_is_kept_and_a_new_one_lasts_two_hours() {
        let now = date_parse("2026-10-26T12:00:00Z").unwrap();
        let running = Policy {
            diagnostics_profile: "DEVELOPMENT",
            diagnostics_expires_at: Some("2026-10-26T12:30:00.000Z".into()),
            ..Policy::default_policy()
        };
        assert_eq!(
            diagnostics_expiry(&running, "DEVELOPMENT", now).as_deref(),
            Some("2026-10-26T12:30:00.000Z")
        );
        assert_eq!(diagnostics_expiry(&running, "PRODUCTION", now), None);
        assert_eq!(
            diagnostics_expiry(&Policy::default_policy(), "DEVELOPMENT", now).as_deref(),
            Some("2026-10-26T14:00:00.000Z")
        );
        let lapsed = Policy {
            diagnostics_expires_at: Some("2026-10-26T11:00:00.000Z".into()),
            ..running
        };
        assert_eq!(
            diagnostics_expiry(&lapsed, "DEVELOPMENT", now).as_deref(),
            Some("2026-10-26T14:00:00.000Z")
        );
    }
}
