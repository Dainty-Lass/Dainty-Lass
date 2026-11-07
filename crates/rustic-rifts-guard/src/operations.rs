// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The two admin operations routes: the reward rollout and alert acknowledgement. Their replies are the admin
//! API's own shapes, not the launcher envelope.

use rustic_rifts_seam::{Actor, IslesCore, Json, Reply, UnixMillis};
use serde_json::Value;

use crate::errors::GuardError;
use crate::json::{js_length, js_safe_integer, js_slice, js_string, js_to_number, js_trim, non_negative};
use crate::ports::{AuditEntry, Clock, OperationsPolicyPort, iso_time};
use crate::store::GuardStore;

/// The reward rollout flags of the operations policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RewardRollout {
    /// The rollout's version; a change must advance it.
    pub version: u64,
    /// Tier 0 hunts may run reward shadow comparisons.
    pub tier0_shadow_enabled: bool,
    /// Tier 1 hunts may be granted rewards.
    pub tier1_grants_enabled: bool,
}

/// The rollout before any change: version 0, tier 0 shadow on, tier 1 grants off.
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
    /// The rollout as the policy document's `rewardRollout` object.
    pub fn to_json(&self) -> Json {
        Json(format!(
            r#"{{"version":{},"tier0ShadowEnabled":{},"tier1GrantsEnabled":{}}}"#,
            self.version, self.tier0_shadow_enabled, self.tier1_grants_enabled
        ))
    }

    /// Reads a `rewardRollout` object; `None` unless all three fields are present and well typed.
    pub fn from_value(value: Option<&Value>) -> Option<Self> {
        let object = value?.as_object()?;
        Some(Self {
            version: object.get("version").and_then(non_negative)?,
            tier0_shadow_enabled: object.get("tier0ShadowEnabled")?.as_bool()?,
            tier1_grants_enabled: object.get("tier1GrantsEnabled")?.as_bool()?,
        })
    }
}

const ROLLOUT_INVALID: &str =
    "A complete reward rollout, expectedVersion, and audit reason of at least 8 characters are required.";
const ALERT_INVALID: &str = "A valid alert ID and an audit reason of at least 8 characters are required.";
const POLICY_CONFLICT: &str = "The operations policy changed. Refresh it before trying again.";
const ROLLOUT_CONFLICT: &str = "The reward rollout version must advance beyond the current central version.";
const STEP_UP_REQUIRED: &str = "A fresh operations authenticator code is required for this safety reduction.";
const ALERT_NOT_OPEN: &str = "The alert is missing or has already been acknowledged.";

fn error_reply(status: u16, code: &str, message: &str, extra: &str) -> Reply {
    Reply {
        status,
        body: Json(format!(
            r#"{{"error":{{"code":{},"message":{}}}{extra}}}"#,
            js_string(code),
            js_string(message)
        )),
    }
}

/// The audit reason: trimmed, at most 500 UTF-16 units, at least 8.
fn reason_of(body: Option<&Value>) -> Option<String> {
    let text = body?.get("reason")?.as_str()?;
    let reason = js_slice(js_trim(text), 500);
    (js_length(reason) >= 8).then(|| reason.to_owned())
}

fn codes_json(codes: &[&str]) -> String {
    format!("[{}]", codes.iter().map(|c| js_string(c)).collect::<Vec<_>>().join(","))
}

/// The policy text with `"weakeningCodes"` added as its last field.
fn with_codes(policy: &str, codes: &str) -> String {
    match policy.strip_suffix('}') {
        Some(head) if head.trim_end().ends_with('{') => format!("{head}\"weakeningCodes\":{codes}}}"),
        Some(head) => format!("{head},\"weakeningCodes\":{codes}}}"),
        None => format!("{{\"weakeningCodes\":{codes}}}"),
    }
}

pub(crate) struct Operations<'a> {
    pub policy: &'a dyn OperationsPolicyPort,
    pub core: &'a dyn IslesCore,
    pub store: &'a dyn GuardStore,
    pub clock: &'a dyn Clock,
}

pub(crate) struct AuditContext<'a> {
    admin_key: &'a str,
    reason: &'a str,
    ip: &'a str,
    request_id: &'a str,
}

impl Operations<'_> {
    async fn audit(
        &self,
        context: &AuditContext<'_>,
        action: &str,
        old_state: String,
        new_state: String,
        created_at: String,
    ) -> Result<(), GuardError> {
        self.policy
            .append_audit(AuditEntry {
                id: self.clock.uuid(),
                actor: context.admin_key.to_owned(),
                action: action.to_owned(),
                old_state: Json(old_state),
                new_state: Json(new_state),
                reason: context.reason.to_owned(),
                ip: context.ip.to_owned(),
                request_id: context.request_id.to_owned(),
                created_at,
            })
            .await?;
        Ok(())
    }

    pub async fn set_reward_rollout(
        &self,
        admin_key: &str,
        body: Option<&Value>,
        ip: &str,
    ) -> Result<Reply, GuardError> {
        let rollout = RewardRollout::from_value(body.and_then(|b| b.get("rewardRollout")));
        let expected = body
            .and_then(|b| b.get("expectedVersion"))
            .and_then(|v| js_safe_integer(js_to_number(v)))
            .and_then(|n| u64::try_from(n).ok());
        let reason = reason_of(body);
        let step_up = match body.and_then(|b| b.get("breakGlassTotp")) {
            None => Ok(None),
            Some(Value::String(code)) => Ok(Some(code.clone())),
            Some(_) => Err(()),
        };
        let (Some(rollout), Some(expected), Some(reason), Ok(step_up)) = (rollout, expected, reason, step_up) else {
            return Ok(error_reply(400, "ADMIN_VALIDATION", ROLLOUT_INVALID, ""));
        };
        let request_id = self.clock.uuid();
        let context = AuditContext {
            admin_key,
            reason: &reason,
            ip,
            request_id: &request_id,
        };
        let current = self.policy.policy().await?;
        let policy_extra = format!(",\"policy\":{}", current.json.0);
        if current.version != expected {
            return Ok(error_reply(
                409,
                "ADMIN_POLICY_CONFLICT",
                POLICY_CONFLICT,
                &policy_extra,
            ));
        }
        if rollout.version <= current.reward_rollout.version {
            return Ok(error_reply(
                409,
                "ADMIN_REWARD_ROLLOUT_CONFLICT",
                ROLLOUT_CONFLICT,
                &policy_extra,
            ));
        }
        let weakening: Vec<&str> = if !current.reward_rollout.tier1_grants_enabled && rollout.tier1_grants_enabled {
            vec!["TIER1_REWARD_GRANTS_ENABLE"]
        } else {
            Vec::new()
        };
        let codes = codes_json(&weakening);
        if !weakening.is_empty() {
            let stepped_up = match step_up {
                Some(code) => self.core.verify_operations_step_up(admin_key.to_owned(), code).await?,
                None => false,
            };
            if !stepped_up {
                let attempted = format!(
                    r#"{{"rewardRollout":{},"weakeningCodes":{codes}}}"#,
                    rollout.to_json().0
                );
                let at = iso_time(self.clock.now());
                self.audit(
                    &context,
                    "operations.policy.weakening_rejected",
                    current.json.0.clone(),
                    attempted,
                    at,
                )
                .await?;
                return Ok(error_reply(
                    403,
                    "ADMIN_POLICY_STEP_UP_REQUIRED",
                    STEP_UP_REQUIRED,
                    &format!("{policy_extra},\"weakeningCodes\":{codes}"),
                ));
            }
        }
        let Some(after) = self
            .policy
            .set_reward_rollout(rollout, admin_key.to_owned(), expected)
            .await?
        else {
            return Ok(error_reply(
                409,
                "ADMIN_POLICY_CONFLICT",
                POLICY_CONFLICT,
                &policy_extra,
            ));
        };
        let at = iso_time(self.clock.now());
        self.audit(
            &context,
            "operations.reward_rollout.update",
            current.json.0.clone(),
            with_codes(&after.json.0, &codes),
            at,
        )
        .await?;
        Ok(Reply {
            status: 200,
            body: Json(format!(
                r#"{{"policy":{},"requestId":{}}}"#,
                after.json.0,
                js_string(&request_id)
            )),
        })
    }

    pub async fn acknowledge_alert(
        &self,
        admin_key: &str,
        alert_id: &str,
        body: Option<&Value>,
        ip: &str,
    ) -> Result<Reply, GuardError> {
        let id = js_trim(alert_id);
        let id_ok = (8..=128).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        let (true, Some(reason)) = (id_ok, reason_of(body)) else {
            return Ok(error_reply(400, "ADMIN_VALIDATION", ALERT_INVALID, ""));
        };
        let request_id = self.clock.uuid();
        let now: UnixMillis = self.clock.now();
        let actor = Actor::Admin {
            key_id: admin_key.to_owned(),
        };
        if !self.store.acknowledge_alert(&actor, id, now)? {
            return Ok(error_reply(404, "ADMIN_ALERT_NOT_OPEN", ALERT_NOT_OPEN, ""));
        }
        let at = iso_time(now);
        let quoted_id = js_string(id);
        let context = AuditContext {
            admin_key,
            reason: &reason,
            ip,
            request_id: &request_id,
        };
        self.audit(
            &context,
            "operations.alert.acknowledge",
            format!(r#"{{"alertId":{quoted_id},"status":"OPEN"}}"#),
            format!(r#"{{"alertId":{quoted_id},"status":"ACKNOWLEDGED","acknowledgedAt":"{at}"}}"#),
            at.clone(),
        )
        .await?;
        Ok(Reply {
            status: 200,
            body: Json(format!(
                r#"{{"alertId":{quoted_id},"status":"ACKNOWLEDGED","acknowledgedAt":"{at}","requestId":{}}}"#,
                js_string(&request_id)
            )),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rollout_state_validation() {
        assert_eq!(
            DEFAULT_REWARD_ROLLOUT.to_json().0,
            r#"{"version":0,"tier0ShadowEnabled":true,"tier1GrantsEnabled":false}"#
        );
        let good = json!({"version": 3, "tier0ShadowEnabled": false, "tier1GrantsEnabled": true});
        assert_eq!(
            RewardRollout::from_value(Some(&good)),
            Some(RewardRollout {
                version: 3,
                tier0_shadow_enabled: false,
                tier1_grants_enabled: true
            })
        );
        for bad in [
            json!({"version": -1, "tier0ShadowEnabled": true, "tier1GrantsEnabled": true}),
            json!({"version": "3", "tier0ShadowEnabled": true, "tier1GrantsEnabled": true}),
            json!({"version": 3, "tier0ShadowEnabled": 1, "tier1GrantsEnabled": true}),
            json!({"version": 3, "tier1GrantsEnabled": true}),
            json!([1]),
        ] {
            assert_eq!(RewardRollout::from_value(Some(&bad)), None, "{bad}");
        }
        assert_eq!(RewardRollout::from_value(None), None);
    }

    #[test]
    fn codes_are_added_as_the_last_field() {
        assert_eq!(with_codes(r#"{"a":1}"#, "[]"), r#"{"a":1,"weakeningCodes":[]}"#);
        assert_eq!(with_codes("{}", r#"["X"]"#), r#"{"weakeningCodes":["X"]}"#);
        assert_eq!(with_codes("nope", "[]"), r#"{"weakeningCodes":[]}"#);
    }

    #[test]
    fn reasons() {
        assert_eq!(reason_of(Some(&json!({"reason": "  short  "}))), None);
        assert_eq!(
            reason_of(Some(&json!({"reason": " long enough "}))).as_deref(),
            Some("long enough")
        );
        assert_eq!(reason_of(Some(&json!({"reason": 12_345_678}))), None);
        let long = "x".repeat(600);
        assert_eq!(reason_of(Some(&json!({ "reason": long }))).map(|r| r.len()), Some(500));
    }
}
