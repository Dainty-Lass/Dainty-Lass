//! `observability/releaseGates.ts`: the manager's preflight gates, pure over the env file's variables. A detail
//! carries names, counts, modes and booleans only, never a value.

use std::collections::BTreeMap;

use serde_json::{Value, json};

pub type Env = BTreeMap<String, String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gate {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

impl Gate {
    pub fn new(name: &'static str, ok: bool, detail: impl Into<String>) -> Self {
        Self {
            name,
            ok,
            detail: detail.into(),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({ "name": self.name, "ok": self.ok, "detail": self.detail })
    }
}

pub const CORE_GATE_NAMES: [&str; 4] = [
    "guard_mode",
    "registration_approval_mode",
    "tls_proxy_consistency",
    "no_legacy_env_keys",
];

pub const LEGACY_ENV_PREFIX: &str = "MYSTPAX_";

fn var<'a>(env: &'a Env, name: &str) -> &'a str {
    env.get(name).map_or("", String::as_str)
}

/// `{ ok, gates }`: ok only with at least one gate and all of them ok.
pub fn body(gates: &[Gate]) -> Value {
    json!({ "ok": !gates.is_empty() && gates.iter().all(|g| g.ok), "gates": gates.iter().map(Gate::to_json).collect::<Vec<_>>() })
}

/// `ResolveGuardMode`: `(enforce, is_default, refused)`.
pub fn guard_mode(env: &Env) -> (bool, bool, bool) {
    match var(env, "RUSTIC_ISLES_GUARD_MODE").trim().to_lowercase().as_str() {
        "" => (false, true, false),
        "observe" => (false, false, false),
        "enforce" => (true, false, false),
        _ => (true, false, true),
    }
}

pub fn guard_mode_gate(env: &Env) -> Gate {
    let (enforce, is_default, refused) = guard_mode(env);
    if refused {
        return Gate::new(
            "guard_mode",
            false,
            "RUSTIC_ISLES_GUARD_MODE has an unrecognised value (must be observe or enforce)",
        );
    }
    Gate::new(
        "guard_mode",
        true,
        format!(
            "mode={}{}",
            if enforce { "enforce" } else { "observe" },
            if is_default { " (default)" } else { "" }
        ),
    )
}

pub fn registration_approval_gate(env: &Env) -> Gate {
    let raw = var(env, "RUSTIC_ISLES_REGISTRATION_APPROVAL").trim().to_lowercase();
    let legacy = !var(env, "RUSTIC_ISLES_REGISTRATION_AUTO_APPROVE").trim().is_empty();
    let (mode, is_default, refused) = match raw.as_str() {
        "" => ("off", true, false),
        "off" => ("off", false, false),
        "required" => ("required", false, false),
        _ => ("required", false, true),
    };
    if refused {
        return Gate::new(
            "registration_approval_mode",
            false,
            "RUSTIC_ISLES_REGISTRATION_APPROVAL has an unrecognised value (treated as required)",
        );
    }
    let legacy = if legacy {
        "; RUSTIC_ISLES_REGISTRATION_AUTO_APPROVE is set and has no effect"
    } else {
        ""
    };
    Gate::new(
        "registration_approval_mode",
        true,
        format!("mode={mode}{}{legacy}", if is_default { " (default)" } else { "" }),
    )
}

/// `ReadTlsTerminatedByProxy`; `Err` where it throws.
pub fn tls_terminated_by_proxy(env: &Env) -> Result<bool, String> {
    let mut raw = var(env, "RUSTIC_ISLES_TLS_TERMINATED_BY_PROXY").trim().to_lowercase();
    if raw.is_empty()
        && matches!(
            var(env, "REFORGED_HTTP_ONLY").trim().to_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    {
        raw = "on".to_owned();
    }
    match raw.as_str() {
        "" | "0" | "off" | "false" | "no" => Ok(false),
        "1" | "on" | "true" | "yes" => Ok(true),
        other => Err(format!(
            "RUSTIC_ISLES_TLS_TERMINATED_BY_PROXY must be on or off (got '{other}')"
        )),
    }
}

pub fn tls_proxy_gate(env: &Env) -> Gate {
    let name = "tls_proxy_consistency";
    let Ok(on) = tls_terminated_by_proxy(env) else {
        return Gate::new(name, false, "RUSTIC_ISLES_TLS_TERMINATED_BY_PROXY must be on or off");
    };
    if !on {
        return Gate::new(name, true, "proxy mode off (in-process HTTPS)");
    }
    let host = var(env, "BIND_HOST").trim().to_lowercase();
    if host.is_empty() {
        return Gate::new(name, false, "proxy mode on but BIND_HOST is unset (every interface)");
    }
    if ["127.0.0.1", "::1", "localhost"].contains(&host.as_str()) {
        return Gate::new(name, true, "proxy mode on, BIND_HOST is loopback");
    }
    Gate::new(name, false, "proxy mode on but BIND_HOST is not a loopback address")
}

pub fn legacy_env_key_names(env: &Env) -> Vec<String> {
    let mut names: Vec<String> = env
        .keys()
        .filter(|k| k.starts_with(LEGACY_ENV_PREFIX))
        .cloned()
        .collect();
    names.sort();
    names
}

pub fn legacy_env_keys_gate(env: &Env) -> Gate {
    let names = legacy_env_key_names(env);
    if names.is_empty() {
        return Gate::new("no_legacy_env_keys", true, format!("0 {LEGACY_ENV_PREFIX} keys"));
    }
    Gate::new(
        "no_legacy_env_keys",
        false,
        format!("{} {LEGACY_ENV_PREFIX} key(s): {}", names.len(), names.join(", ")),
    )
}

pub fn core_gates(env: &Env) -> Vec<Gate> {
    vec![
        guard_mode_gate(env),
        registration_approval_gate(env),
        tls_proxy_gate(env),
        legacy_env_keys_gate(env),
    ]
}

pub const ACTIVE_PLAY_WINDOW_MS: i64 = 2 * 60_000;
pub const RELEASE_DURING_PLAY_FILE: &str = "RELEASE_DURING_PLAY.allow";

pub fn active_play_gate(playing: usize, overridden: bool) -> Gate {
    let name = "no_active_play";
    if playing == 0 {
        return Gate::new(name, true, "nobody in game");
    }
    if overridden {
        return Gate::new(
            name,
            true,
            format!("{playing} in game; held release let through by {RELEASE_DURING_PLAY_FILE}"),
        );
    }
    Gate::new(
        name,
        false,
        format!(
            "{playing} player(s) in game within the last {} minutes: release held until they stop \
             (emergency override: {RELEASE_DURING_PLAY_FILE} in the backend's shared folder)",
            ACTIVE_PLAY_WINDOW_MS / 60_000
        ),
    )
}

/// The headers a reverse proxy adds; a request carrying one is refused.
pub fn has_forwarding_header(headers: &axum::http::HeaderMap) -> bool {
    ["x-forwarded-for", "x-real-ip", "forwarded"]
        .iter()
        .any(|name| headers.contains_key(*name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Env {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    #[test]
    fn the_aggregate_is_ok_only_with_gates_and_all_ok() {
        let ok = Gate::new("a", true, "x");
        let bad = Gate::new("b", false, "y");
        assert_eq!(body(&[ok.clone(), ok.clone()])["ok"], true);
        assert_eq!(body(&[ok.clone(), bad])["ok"], false);
        assert_eq!(body(&[])["ok"], false);
        assert_eq!(
            body(&[ok]).as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()),
            Some(vec!["ok".into(), "gates".into()])
        );
    }

    #[test]
    fn guard_mode_reads_like_node() {
        assert_eq!(guard_mode_gate(&env(&[])).detail, "mode=observe (default)");
        assert_eq!(
            guard_mode_gate(&env(&[("RUSTIC_ISLES_GUARD_MODE", " Observe ")])).detail,
            "mode=observe"
        );
        let enforce = guard_mode_gate(&env(&[("RUSTIC_ISLES_GUARD_MODE", "enforce")]));
        assert_eq!((enforce.ok, enforce.detail.as_str()), (true, "mode=enforce"));
        let bad = guard_mode_gate(&env(&[("RUSTIC_ISLES_GUARD_MODE", "sometimes")]));
        assert!(!bad.ok && bad.name == "guard_mode" && !bad.detail.contains("sometimes"));
    }

    #[test]
    fn registration_approval_reads_like_node() {
        assert_eq!(registration_approval_gate(&env(&[])).detail, "mode=off (default)");
        let required = registration_approval_gate(&env(&[("RUSTIC_ISLES_REGISTRATION_APPROVAL", "required")]));
        assert_eq!((required.ok, required.detail.as_str()), (true, "mode=required"));
        let bad = registration_approval_gate(&env(&[("RUSTIC_ISLES_REGISTRATION_APPROVAL", "maybe")]));
        assert!(!bad.ok && !bad.detail.contains("maybe"));
        let legacy = registration_approval_gate(&env(&[("RUSTIC_ISLES_REGISTRATION_AUTO_APPROVE", "true")]));
        assert!(legacy.ok && legacy.detail.contains("has no effect"));
    }

    #[test]
    fn the_tls_proxy_gate_wants_a_loopback_bind() {
        assert_eq!(
            tls_proxy_gate(&env(&[("RUSTIC_ISLES_TLS_TERMINATED_BY_PROXY", "off")])).detail,
            "proxy mode off (in-process HTTPS)"
        );
        let on = |host: &str| {
            let mut pairs = vec![("RUSTIC_ISLES_TLS_TERMINATED_BY_PROXY", "on")];
            if !host.is_empty() {
                pairs.push(("BIND_HOST", host));
            }
            tls_proxy_gate(&env(&pairs))
        };
        assert!(on("127.0.0.1").ok && on("::1").ok && on("localhost").ok);
        assert!(!on("0.0.0.0").ok && !on("203.0.113.9").ok && !on("").ok);
        assert!(!on("0.0.0.0").detail.contains("0.0.0.0"));
        let bad = tls_proxy_gate(&env(&[
            ("RUSTIC_ISLES_TLS_TERMINATED_BY_PROXY", "perhaps"),
            ("BIND_HOST", "127.0.0.1"),
        ]));
        assert!(!bad.ok && bad.name == "tls_proxy_consistency");
    }

    #[test]
    fn legacy_keys_show_names_never_values() {
        assert!(legacy_env_keys_gate(&env(&[("RUSTIC_ISLES_GUARD_MODE", "observe")])).ok);
        let gate = legacy_env_keys_gate(&env(&[
            ("MYSTPAX_B", "super-secret-value-7731"),
            ("MYSTPAX_A", "x"),
            ("RUSTIC_ISLES_X", "y"),
        ]));
        assert!(!gate.ok);
        assert_eq!(gate.detail, "2 MYSTPAX_ key(s): MYSTPAX_A, MYSTPAX_B");
    }

    #[test]
    fn the_core_gates_come_in_manager_order() {
        let gates = core_gates(&env(&[]));
        assert_eq!(gates.iter().map(|g| g.name).collect::<Vec<_>>(), CORE_GATE_NAMES);
        assert_eq!(body(&gates)["ok"], true);
        assert_eq!(body(&core_gates(&env(&[("MYSTPAX_X", "1")])))["ok"], false);
    }

    #[test]
    fn active_play_holds_a_release() {
        assert_eq!(active_play_gate(0, false).detail, "nobody in game");
        assert!(!active_play_gate(3, false).ok);
        assert!(
            active_play_gate(3, false)
                .detail
                .starts_with("3 player(s) in game within the last 2 minutes")
        );
        assert!(active_play_gate(3, true).ok);
    }
}
