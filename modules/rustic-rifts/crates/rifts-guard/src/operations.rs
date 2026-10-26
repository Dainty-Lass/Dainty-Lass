//! Guard's admin operations routes (`p2pAdminOperations.ts`): the Tier 1 reward rollout and the acknowledgement of
//! central Guard alerts, with the operations policy rules they use (`p2pOperationsPolicy.ts`).

use rifts_seam::{Actor, Json, Reply};
use serde_json::Value;

use crate::json::{js_length, js_safe_integer, js_slice, js_string, js_trim, safe_integer};

/// `RewardRolloutState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RewardRollout {
    /// Monotonic, operator controlled.
    pub version: u64,
    pub tier0_shadow_enabled: bool,
    pub tier1_grants_enabled: bool,
}

/// `DEFAULT_REWARD_ROLLOUT`: a missing or malformed row means Tier 1 grants off.
pub const DEFAULT_REWARD_ROLLOUT: RewardRollout = RewardRollout {
    version: 0,
    tier0_shadow_enabled: true,
    tier1_grants_enabled: false,
};

impl Default for RewardRollout {
    fn default() -> Self {
        DEFAULT_REWARD_ROLLOUT
    }
}

impl RewardRollout {
    pub fn to_json(&self) -> Json {
        Json(format!(
            r#"{{"version":{},"tier0ShadowEnabled":{},"tier1GrantsEnabled":{}}}"#,
            self.version, self.tier0_shadow_enabled, self.tier1_grants_enabled
        ))
    }

    /// The rollout of a request or a stored row: a safe non-negative integer version and two booleans.
    pub fn from_value(value: Option<&Value>) -> Option<Self> {
        let object = value?.as_object()?;
        let version = object.get("version").and_then(safe_integer)?;
        Some(Self {
            version: u64::try_from(version).ok()?,
            tier0_shadow_enabled: object.get("tier0ShadowEnabled")?.as_bool()?,
            tier1_grants_enabled: object.get("tier1GrantsEnabled")?.as_bool()?,
        })
    }
}

/// `RewardRolloutWeakeningCodes`: turning Tier 1 grants on is a break-glass weakening.
pub fn weakening_codes(before: &RewardRollout, after: &RewardRollout) -> Vec<&'static str> {
    if !before.tier1_grants_enabled && after.tier1_grants_enabled {
        vec!["TIER1_REWARD_GRANTS_ENABLE"]
    } else {
        Vec::new()
    }
}

/// `typeof reason === "string" ? reason.trim().slice(0, 500) : ""`.
pub fn audit_reason(body: Option<&Value>) -> String {
    body.and_then(|body| body.get("reason"))
        .and_then(Value::as_str)
        .map(|reason| js_slice(js_trim(reason), 500).to_owned())
        .unwrap_or_default()
}

pub fn reason_long_enough(reason: &str) -> bool {
    js_length(reason) >= 8
}

/// A validated `PATCH /admin/v1/operations/reward-rollout` body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RolloutRequest {
    pub rollout: RewardRollout,
    pub expected_version: u64,
    pub reason: String,
    pub break_glass_totp: Option<String>,
}

pub fn parse_rollout_request(body: Option<&Value>) -> Option<RolloutRequest> {
    let rollout = RewardRollout::from_value(body.and_then(|body| body.get("rewardRollout")))?;
    let expected_version = js_safe_integer(body.and_then(|body| body.get("expectedVersion")))
        .and_then(|version| u64::try_from(version).ok())?;
    let reason = audit_reason(body);
    if !reason_long_enough(&reason) {
        return None;
    }
    Some(RolloutRequest {
        rollout,
        expected_version,
        reason,
        break_glass_totp: body
            .and_then(|body| body.get("breakGlassTotp"))
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn admin_error(status: u16, code: &str, message: &str, extra: &str) -> Reply {
    Reply {
        status,
        body: Json(format!(
            r#"{{"error":{{"code":{},"message":{}}}{extra}}}"#,
            js_string(code),
            js_string(message)
        )),
    }
}

pub fn rollout_validation_error() -> Reply {
    admin_error(
        400,
        "ADMIN_VALIDATION",
        "A complete reward rollout, expectedVersion, and audit reason of at least 8 characters are required.",
        "",
    )
}

pub fn policy_conflict(policy: &Json) -> Reply {
    admin_error(
        409,
        "ADMIN_POLICY_CONFLICT",
        "The operations policy changed. Refresh it before trying again.",
        &format!(r#","policy":{}"#, policy.0),
    )
}

pub fn rollout_conflict(policy: &Json) -> Reply {
    admin_error(
        409,
        "ADMIN_REWARD_ROLLOUT_CONFLICT",
        "The reward rollout version must advance beyond the current central version.",
        &format!(r#","policy":{}"#, policy.0),
    )
}

pub fn codes_json(codes: &[&str]) -> String {
    let quoted: Vec<String> = codes.iter().map(|code| js_string(code)).collect();
    format!("[{}]", quoted.join(","))
}

pub fn step_up_required(policy: &Json, codes: &[&str]) -> Reply {
    admin_error(
        403,
        "ADMIN_POLICY_STEP_UP_REQUIRED",
        "A fresh operations authenticator code is required for this safety reduction.",
        &format!(r#","policy":{},"weakeningCodes":{}"#, policy.0, codes_json(codes)),
    )
}

pub fn rollout_updated(policy: &Json, request_id: &str) -> Reply {
    Reply {
        status: 200,
        body: Json(format!(
            r#"{{"policy":{},"requestId":{}}}"#,
            policy.0,
            js_string(request_id)
        )),
    }
}

/// `{ ...after, weakeningCodes }`: the policy object with the codes as its last field.
pub fn with_weakening_codes(policy: &Json, codes: &[&str]) -> Json {
    let text = policy.0.trim_end();
    match text.strip_suffix('}') {
        Some(head) if head.trim_end().ends_with('{') => {
            Json(format!(r#"{head}"weakeningCodes":{}}}"#, codes_json(codes)))
        }
        Some(head) => Json(format!(r#"{head},"weakeningCodes":{}}}"#, codes_json(codes))),
        None => Json(format!(r#"{{"weakeningCodes":{}}}"#, codes_json(codes))),
    }
}

/// The alert id and reason of `PATCH /admin/v1/operations/alerts/:alertId/ack`, when both are valid.
pub fn parse_ack(alert_id: &str, body: Option<&Value>) -> Option<(String, String)> {
    let alert_id = js_trim(alert_id);
    let valid_id =
        (8..=128).contains(&alert_id.len()) && alert_id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    let reason = audit_reason(body);
    (valid_id && reason_long_enough(&reason)).then(|| (alert_id.to_owned(), reason))
}

pub fn ack_validation_error() -> Reply {
    admin_error(
        400,
        "ADMIN_VALIDATION",
        "A valid alert ID and an audit reason of at least 8 characters are required.",
        "",
    )
}

pub fn alert_not_open() -> Reply {
    admin_error(
        404,
        "ADMIN_ALERT_NOT_OPEN",
        "The alert is missing or has already been acknowledged.",
        "",
    )
}

pub fn alert_acknowledged(alert_id: &str, acknowledged_at: &str, request_id: &str) -> Reply {
    Reply {
        status: 200,
        body: Json(format!(
            r#"{{"alertId":{},"status":"ACKNOWLEDGED","acknowledgedAt":{},"requestId":{}}}"#,
            js_string(alert_id),
            js_string(acknowledged_at),
            js_string(request_id)
        )),
    }
}

pub fn admin_actor(key_id: &str) -> Actor {
    Actor::Admin {
        key_id: key_id.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // p2pOperationsPolicy.test.ts: "enabling central Tier 1 grants is a break-glass weakening"
    #[test]
    fn enabling_tier1_grants_is_a_weakening() {
        let on = RewardRollout {
            version: 1,
            tier0_shadow_enabled: true,
            tier1_grants_enabled: true,
        };
        assert_eq!(
            weakening_codes(&DEFAULT_REWARD_ROLLOUT, &on),
            ["TIER1_REWARD_GRANTS_ENABLE"]
        );
        assert!(weakening_codes(&on, &on).is_empty());
        assert!(weakening_codes(&on, &DEFAULT_REWARD_ROLLOUT).is_empty());
    }

    // p2pOperationsPolicy.test.ts: "central reward rollout normalizes legacy rows and rejects malformed rollout state"
    #[test]
    fn rollout_state_is_validated() {
        assert_eq!(
            RewardRollout::from_value(Some(
                &json!({"version": 3, "tier0ShadowEnabled": false, "tier1GrantsEnabled": true})
            )),
            Some(RewardRollout {
                version: 3,
                tier0_shadow_enabled: false,
                tier1_grants_enabled: true
            })
        );
        for bad in [
            json!({"version": -1, "tier0ShadowEnabled": true, "tier1GrantsEnabled": false}),
            json!({"version": 1.5, "tier0ShadowEnabled": true, "tier1GrantsEnabled": false}),
            json!({"version": "1", "tier0ShadowEnabled": true, "tier1GrantsEnabled": false}),
            json!({"version": 1, "tier0ShadowEnabled": "yes", "tier1GrantsEnabled": false}),
            json!({"version": 1, "tier0ShadowEnabled": true}),
        ] {
            assert_eq!(RewardRollout::from_value(Some(&bad)), None, "{bad}");
        }
        assert_eq!(RewardRollout::default(), DEFAULT_REWARD_ROLLOUT);
    }

    #[test]
    fn rollout_requests_need_every_field() {
        let good = json!({"rewardRollout": {"version": 2, "tier0ShadowEnabled": true, "tier1GrantsEnabled": false},
            "expectedVersion": "4", "reason": "  rolling out tier one  ", "breakGlassTotp": "123456"});
        let request = parse_rollout_request(Some(&good)).unwrap();
        assert_eq!(request.expected_version, 4, "Number(\"4\")");
        assert_eq!(request.reason, "rolling out tier one");
        assert_eq!(request.break_glass_totp.as_deref(), Some("123456"));
        let mut missing_version = good.clone();
        missing_version.as_object_mut().unwrap().remove("expectedVersion");
        assert!(
            parse_rollout_request(Some(&missing_version)).is_none(),
            "Number(undefined) is NaN"
        );
        let mut short = good.clone();
        short["reason"] = json!("short");
        assert!(parse_rollout_request(Some(&short)).is_none());
        let mut negative = good.clone();
        negative["expectedVersion"] = json!(-1);
        assert!(parse_rollout_request(Some(&negative)).is_none());
        assert!(parse_rollout_request(None).is_none());
    }

    #[test]
    fn ack_requests_need_an_id_and_a_reason() {
        let body = json!({"reason": "seen and handled"});
        assert_eq!(
            parse_ack(" alert-0001 ", Some(&body)),
            Some(("alert-0001".to_owned(), "seen and handled".to_owned()))
        );
        assert!(parse_ack("short", Some(&body)).is_none());
        assert!(parse_ack("alert_0001", Some(&body)).is_none());
        assert!(parse_ack("alert-0001", Some(&json!({"reason": "tiny"}))).is_none());
        assert!(parse_ack("alert-0001", None).is_none());
    }

    #[test]
    fn replies_keep_node_field_order() {
        let policy = Json(r#"{"version":1}"#.to_owned());
        assert_eq!(
            step_up_required(&policy, &["TIER1_REWARD_GRANTS_ENABLE"]).body.0,
            concat!(
                r#"{"error":{"code":"ADMIN_POLICY_STEP_UP_REQUIRED","message":"A fresh operations authenticator code "#,
                r#"is required for this safety reduction."},"policy":{"version":1},"#,
                r#""weakeningCodes":["TIER1_REWARD_GRANTS_ENABLE"]}"#
            )
        );
        assert_eq!(
            with_weakening_codes(&policy, &[]).0,
            r#"{"version":1,"weakeningCodes":[]}"#
        );
        assert_eq!(
            with_weakening_codes(&Json("{}".to_owned()), &[]).0,
            r#"{"weakeningCodes":[]}"#
        );
        assert_eq!(
            alert_acknowledged("alert-0001", "2026-10-26T00:00:00.000Z", "r-1")
                .body
                .0,
            concat!(
                r#"{"alertId":"alert-0001","status":"ACKNOWLEDGED","#,
                r#""acknowledgedAt":"2026-10-26T00:00:00.000Z","requestId":"r-1"}"#
            )
        );
    }
}
