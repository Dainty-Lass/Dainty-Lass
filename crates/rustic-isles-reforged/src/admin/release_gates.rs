// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `GET /health/release-gates` (admin spec §8): loopback only, from the env file map. A detail names counts, modes
//! and booleans, never a value.

use std::collections::BTreeMap;
use std::path::Path;

use axum::response::Response;
use http::StatusCode;
use rustic_isles::{Call, Shared};
use serde_json::Value;

use crate::state::ReforgedState;
use crate::util::{self, object};

/// One release gate's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gate {
    /// The gate's name.
    pub name: &'static str,
    /// Whether it passes.
    pub ok: bool,
    /// Counts, modes and booleans that explain it; never a value.
    pub detail: String,
}

fn gate(name: &'static str, ok: bool, detail: impl Into<String>) -> Gate {
    Gate {
        name,
        ok,
        detail: detail.into(),
    }
}

fn read(env: &BTreeMap<String, String>, name: &str) -> String {
    env.get(name).map(|v| v.trim().to_lowercase()).unwrap_or_default()
}

/// The active-play gate: `active` players over 2 minutes, and whether the override file is there.
pub fn active_play_gate(active: usize, override_present: bool) -> Gate {
    if active == 0 {
        return gate("no_active_play", true, "nobody in game");
    }
    if override_present {
        return gate(
            "no_active_play",
            true,
            format!("{active} in game; held release let through by RELEASE_DURING_PLAY.allow"),
        );
    }
    gate(
        "no_active_play",
        false,
        format!(
            "{active} player(s) in game within the last 2 minutes: release held until they stop (emergency \
             override: RELEASE_DURING_PLAY.allow in the backend's shared folder)"
        ),
    )
}

/// The four configuration gates, in order.
pub fn config_gates(env: &BTreeMap<String, String>) -> Vec<Gate> {
    let mut gates = Vec::new();
    gates.push(match read(env, "RUSTIC_ISLES_GUARD_MODE").as_str() {
        "" => gate("guard_mode", true, "mode=observe (default)"),
        "observe" => gate("guard_mode", true, "mode=observe"),
        "enforce" => gate("guard_mode", true, "mode=enforce"),
        _ => gate(
            "guard_mode",
            false,
            "RUSTIC_ISLES_GUARD_MODE has an unrecognised value (must be observe or enforce)",
        ),
    });
    let mut approval = match read(env, "RUSTIC_ISLES_REGISTRATION_APPROVAL").as_str() {
        "" => gate("registration_approval_mode", true, "mode=off (default)"),
        "off" => gate("registration_approval_mode", true, "mode=off"),
        "required" => gate("registration_approval_mode", true, "mode=required"),
        _ => gate(
            "registration_approval_mode",
            false,
            "RUSTIC_ISLES_REGISTRATION_APPROVAL has an unrecognised value (treated as required)",
        ),
    };
    if approval.ok && !read(env, "RUSTIC_ISLES_REGISTRATION_AUTO_APPROVE").is_empty() {
        approval
            .detail
            .push_str("; RUSTIC_ISLES_REGISTRATION_AUTO_APPROVE is set and has no effect");
    }
    gates.push(approval);
    let mut tls = read(env, "RUSTIC_ISLES_TLS_TERMINATED_BY_PROXY");
    if tls.is_empty() && matches!(read(env, "DAINTY_HTTP_ONLY").as_str(), "1" | "true" | "yes" | "on") {
        tls = "on".into();
    }
    gates.push(match tls.as_str() {
        "" | "0" | "off" | "false" | "no" => gate("tls_proxy_consistency", true, "proxy mode off (in-process HTTPS)"),
        "1" | "on" | "true" | "yes" => match read(env, "BIND_HOST").as_str() {
            "" => gate(
                "tls_proxy_consistency",
                false,
                "proxy mode on but BIND_HOST is unset (every interface)",
            ),
            "127.0.0.1" | "::1" | "localhost" => {
                gate("tls_proxy_consistency", true, "proxy mode on, BIND_HOST is loopback")
            }
            _ => gate(
                "tls_proxy_consistency",
                false,
                "proxy mode on but BIND_HOST is not a loopback address",
            ),
        },
        _ => gate(
            "tls_proxy_consistency",
            false,
            "RUSTIC_ISLES_TLS_TERMINATED_BY_PROXY must be on or off",
        ),
    });
    let legacy: Vec<&String> = env.keys().filter(|k| k.starts_with("MYSTPAX_")).collect();
    gates.push(if legacy.is_empty() {
        gate("no_legacy_env_keys", true, "0 MYSTPAX_ keys")
    } else {
        let names: Vec<&str> = legacy.iter().map(|k| k.as_str()).collect();
        gate(
            "no_legacy_env_keys",
            false,
            format!("{} MYSTPAX_ key(s): {}", names.len(), names.join(", ")),
        )
    });
    gates
}

fn override_file(update_root: &Path) -> bool {
    update_root
        .parent()
        .is_some_and(|parent| parent.join("RELEASE_DURING_PLAY.allow").exists())
}

/// Every gate, in order.
pub fn collect_release_gates(shared: &Shared) -> Vec<Gate> {
    let active = ReforgedState::of(shared).active_players(shared, 120_000).len();
    let root = crate::updates::update_root(shared.settings());
    let mut gates = vec![active_play_gate(active, override_file(&root))];
    gates.extend(config_gates(shared.settings().env_map()));
    gates
}

/// The status (200 when every gate passes, else 503) and the JSON body.
pub fn gates_body(gates: &[Gate]) -> (StatusCode, Value) {
    let ok = !gates.is_empty() && gates.iter().all(|g| g.ok);
    let list: Vec<Value> = gates
        .iter()
        .map(|g| {
            object(vec![
                ("name", Value::from(g.name)),
                ("ok", Value::Bool(g.ok)),
                ("detail", Value::from(g.detail.clone())),
            ])
        })
        .collect();
    let status = if ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        object(vec![("ok", Value::Bool(ok)), ("gates", Value::Array(list))]),
    )
}

/// `GET /health/release-gates`: loopback only, refused through a proxy.
pub async fn release_gates(call: Call) -> Response {
    let peer = util::peer_text(&call.request);
    let headers = call.request.headers();
    let forwarded = ["x-forwarded-for", "x-real-ip", "forwarded"]
        .iter()
        .any(|h| headers.contains_key(*h));
    if !matches!(peer.as_str(), "127.0.0.1" | "::1" | "::ffff:127.0.0.1") || forwarded {
        return util::empty(StatusCode::NOT_FOUND);
    }
    let (status, body) = gates_body(&collect_release_gates(&call.shared));
    util::json(status, &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    fn detail(gates: &[Gate], name: &str) -> (bool, String) {
        let g = gates.iter().find(|g| g.name == name).unwrap();
        (g.ok, g.detail.clone())
    }

    #[test]
    fn gates() {
        let g = config_gates(&env(&[("RUSTIC_ISLES_GUARD_MODE", " Observe ")]));
        assert_eq!(detail(&g, "guard_mode"), (true, "mode=observe".into()));
        let g = config_gates(&env(&[("RUSTIC_ISLES_GUARD_MODE", "sometimes")]));
        let (ok, text) = detail(&g, "guard_mode");
        assert!(!ok && !text.contains("sometimes"));
        let g = config_gates(&env(&[
            ("RUSTIC_ISLES_REGISTRATION_APPROVAL", "required"),
            ("RUSTIC_ISLES_REGISTRATION_AUTO_APPROVE", "x"),
        ]));
        assert_eq!(
            detail(&g, "registration_approval_mode").1,
            "mode=required; RUSTIC_ISLES_REGISTRATION_AUTO_APPROVE is set and has no effect"
        );
        let g = config_gates(&env(&[("RUSTIC_ISLES_REGISTRATION_APPROVAL", "maybe")]));
        assert!(!detail(&g, "registration_approval_mode").1.contains("maybe"));
        assert_eq!(
            detail(&config_gates(&env(&[])), "tls_proxy_consistency"),
            (true, "proxy mode off (in-process HTTPS)".into())
        );
        for host in ["127.0.0.1", "::1", "localhost"] {
            let g = config_gates(&env(&[
                ("RUSTIC_ISLES_TLS_TERMINATED_BY_PROXY", "on"),
                ("BIND_HOST", host),
            ]));
            assert!(detail(&g, "tls_proxy_consistency").0, "{host}");
        }
        for host in ["0.0.0.0", "203.0.113.9", ""] {
            let g = config_gates(&env(&[
                ("RUSTIC_ISLES_TLS_TERMINATED_BY_PROXY", "on"),
                ("BIND_HOST", host),
            ]));
            let (ok, text) = detail(&g, "tls_proxy_consistency");
            assert!(!ok && (host.is_empty() || !text.contains(host)), "{host}");
        }
        let g = config_gates(&env(&[("RUSTIC_ISLES_TLS_TERMINATED_BY_PROXY", "perhaps")]));
        assert!(!detail(&g, "tls_proxy_consistency").0);
        let g = config_gates(&env(&[("MYSTPAX_B", "secret-b"), ("MYSTPAX_A", "secret-a")]));
        let (ok, text) = detail(&g, "no_legacy_env_keys");
        assert!(!ok && text == "2 MYSTPAX_ key(s): MYSTPAX_A, MYSTPAX_B");
        assert_eq!(active_play_gate(0, false).detail, "nobody in game");
        assert!(!active_play_gate(3, false).ok && active_play_gate(3, false).detail.contains("2 minutes"));
        assert!(active_play_gate(3, true).ok);
        assert_eq!(gates_body(&[]).0, StatusCode::SERVICE_UNAVAILABLE);
        let names: Vec<&str> = config_gates(&env(&[])).iter().map(|g| g.name).collect();
        assert_eq!(
            names,
            [
                "guard_mode",
                "registration_approval_mode",
                "tls_proxy_consistency",
                "no_legacy_env_keys"
            ]
        );
    }
}
