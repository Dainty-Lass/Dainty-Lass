// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Environment variables override settings.toml (RD24, docs/config-toml-plan.md "Environment variables override the
//! config"). A variable in the host's own environment wins over the line setup rendered when the service reads it:
//! a key its env file has, or one its typed table may set. Each override is a warning every time the service
//! starts, with a secret's or a safety setting's value redacted. The generic names a dev box or a hosting panel
//! sets for everything (`PORT`, `LOG_LEVEL`, …) are dropped instead, and take a service prefix to override:
//! `DAINTY_BACKEND_PORT`, `DAINTY_DIRECTOR_LOG_LEVEL`.

use super::{SECRET_KEYS, tunables};
use crate::services::service_environment;

/// Set globally too often to be meant for one service: overridden only as `DAINTY_<SERVICE>_<NAME>`.
pub const GENERIC: [&str; 6] = ["PORT", "BIND_HOST", "NODE_ENV", "LOG_LEVEL", "MY_IP", "BACKEND_URL"];

/// What decides who may connect or what a public server allows, besides the secrets: redacted like a secret.
const SAFETY: [&str; 5] = [
    "DAINTY_SERVER_MODE",
    "APPROVED_EXECUTABLE_SHA256",
    "DAINTY_XP_MULTIPLIER",
    "DAINTY_REWARD_MULTIPLIER",
    "REGISTRATION_MODE",
];
const SECRET_SUFFIXES: [&str; 6] = ["_SECRET", "_KEY", "_KEYS", "_TOKEN", "_PASSWORD", "_B64"];

/// A service's environment and the overrides in it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Merged {
    /// The variables the service gets.
    pub env: Vec<(String, String)>,
    /// One line per override or dropped generic variable, for the log.
    pub warnings: Vec<String>,
    /// The variables the environment set over settings.toml.
    pub overridden: Vec<String>,
}

/// Whether `key`'s value is never shown: a secret, an `AUTH_*` key or a safety setting.
#[must_use]
pub fn sensitive(key: &str) -> bool {
    SECRET_KEYS.contains(&key)
        || SAFETY.contains(&key)
        || key.starts_with("AUTH_")
        || SECRET_SUFFIXES.iter().any(|s| key.ends_with(s))
}

fn upsert(env: &mut Vec<(String, String)>, key: &str, value: &str) {
    match env.iter_mut().find(|(k, _)| k == key) {
        Some((_, v)) => value.clone_into(v),
        None => env.push((key.to_owned(), value.to_owned())),
    }
}

fn warning(service: &str, key: &str, value: &str, rendered: Option<&str>, via: Option<&str>) -> String {
    let source = via.map_or_else(String::new, |v| format!(" (as {v})"));
    if sensitive(key) {
        return format!(
            "{service}: {key}{source} from the environment overrides settings.toml, a secret or safety setting (value redacted)"
        );
    }
    let at = tunables::toml_key(service, key).unwrap_or_else(|| format!("{service}.env's {key}"));
    match rendered {
        Some(old) => {
            format!("{service}: {key}={value}{source} from the environment overrides settings.toml ({at} = {old})")
        }
        None => {
            format!(
                "{service}: {key}={value}{source} from the environment overrides settings.toml ({at} is not set there)"
            )
        }
    }
}

/// `service`'s environment: `inherited`, then its env file, then the inherited variables the service reads (its
/// file's and its table's), which win, then `extra`, the supervisor's own wiring, which nothing overrides.
/// `service` is the env file's name (`gateway`, `backend`, …).
pub fn merge(
    service: &str,
    inherited: &[(String, String)],
    file: &[(String, String)],
    extra: &[(String, String)],
) -> Merged {
    let mut env = service_environment(inherited.iter().cloned(), file.iter().cloned(), extra);
    let rendered = |key: &str| file.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
    let table = tunables::env_names(service);
    let prefix = format!("DAINTY_{}_", service.to_ascii_uppercase());
    let mut out = Merged::default();
    for (key, value) in inherited {
        if extra.iter().any(|(k, _)| k == key) {
            continue;
        }
        if GENERIC.contains(&key.as_str()) {
            if rendered(key).is_some_and(|old| old != value) {
                out.warnings.push(format!(
                    "{service}: {key} in the environment is ignored, since many programs share it; set {prefix}{key} to override settings.toml's"
                ));
            }
            continue;
        }
        if let Some(name) = key.strip_prefix(&prefix).filter(|n| GENERIC.contains(n))
            && (rendered(name).is_some() || table.contains(&name))
            && !extra.iter().any(|(k, _)| k == name)
        {
            if rendered(name) != Some(value.as_str()) {
                upsert(&mut env, name, value);
                out.warnings
                    .push(warning(service, name, value, rendered(name), Some(key.as_str())));
                out.overridden.push(name.to_owned());
            }
            continue;
        }
        let read = rendered(key).is_some() || table.contains(&key.as_str());
        if read && rendered(key) != Some(value.as_str()) {
            upsert(&mut env, key, value);
            out.warnings.push(warning(service, key, value, rendered(key), None));
            out.overridden.push(key.clone());
        }
    }
    out.env = env;
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    fn get<'a>(m: &'a Merged, key: &str) -> Option<&'a str> {
        m.env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    #[test]
    fn an_inherited_variable_the_service_reads_wins_with_a_warning() {
        let file = pairs(&[("GATEWAY_RATE_WS", "20,12"), ("GATEWAY_TLS", "1")]);
        let m = merge("gateway", &pairs(&[("GATEWAY_RATE_WS", "40,24")]), &file, &[]);
        assert_eq!(get(&m, "GATEWAY_RATE_WS"), Some("40,24"));
        assert_eq!(
            m.warnings,
            ["gateway: GATEWAY_RATE_WS=40,24 from the environment overrides settings.toml (gateway.rate.ws = 20,12)"]
        );
        let unset = merge("gateway", &pairs(&[("GATEWAY_MAX_CONNECTIONS", "9")]), &file, &[]);
        assert_eq!(get(&unset, "GATEWAY_MAX_CONNECTIONS"), Some("9"));
        assert!(unset.warnings[0].contains("gateway.limits.max_connections is not set there"));
    }

    #[test]
    fn the_same_value_and_unread_variables_stay_quiet() {
        let file = pairs(&[("GATEWAY_TLS", "1")]);
        let m = merge(
            "gateway",
            &pairs(&[("GATEWAY_TLS", "1"), ("PATH", "/bin"), ("CONTENT_X", "y")]),
            &file,
            &[],
        );
        assert!(m.warnings.is_empty(), "{:?}", m.warnings);
        assert_eq!(get(&m, "PATH"), Some("/bin"));
        assert_eq!(get(&m, "CONTENT_X"), Some("y"), "passed through as before");
    }

    #[test]
    fn a_generic_name_is_dropped_and_the_prefixed_one_applied() {
        let file = pairs(&[("PORT", "61000"), ("LOG_LEVEL", "info")]);
        let m = merge(
            "backend",
            &pairs(&[("PORT", "3000"), ("DAINTY_BACKEND_LOG_LEVEL", "debug")]),
            &file,
            &[],
        );
        assert_eq!(get(&m, "PORT"), Some("61000"));
        assert_eq!(get(&m, "LOG_LEVEL"), Some("debug"));
        assert!(m.warnings[0].contains("set DAINTY_BACKEND_PORT"), "{:?}", m.warnings);
        assert_eq!(
            m.warnings[1],
            "backend: LOG_LEVEL=debug (as DAINTY_BACKEND_LOG_LEVEL) from the environment overrides settings.toml (backend.log_level = info)"
        );
        // The gateway reads no PORT: DAINTY_GATEWAY_PORT (Docker's publish setting) is not for it
        let g = merge("gateway", &pairs(&[("DAINTY_GATEWAY_PORT", "443")]), &[], &[]);
        assert!(g.warnings.is_empty() && get(&g, "PORT").is_none());
    }

    #[test]
    fn a_secret_is_overridden_but_never_shown() {
        let file = pairs(&[
            ("GATEWAY_SECRET", "a".repeat(64).as_str()),
            ("DAINTY_SERVER_MODE", "public"),
        ]);
        let m = merge(
            "gateway",
            &pairs(&[("GATEWAY_SECRET", "hunter2hunter2"), ("DAINTY_SERVER_MODE", "private")]),
            &file,
            &[],
        );
        assert_eq!(get(&m, "GATEWAY_SECRET"), Some("hunter2hunter2"));
        assert_eq!(m.warnings.len(), 2);
        for w in &m.warnings {
            assert!(w.contains("value redacted"), "{w}");
            assert!(
                !w.contains("hunter2") && !w.contains("aaaa") && !w.contains("private"),
                "{w}"
            );
        }
    }

    #[test]
    fn the_supervisors_own_variables_are_never_overridden() {
        let file = pairs(&[("GREETING", "from-the-file")]);
        let extra = pairs(&[("ROLE", "service"), ("GREETING", "from-extra")]);
        let m = merge(
            "director",
            &pairs(&[("ROLE", "server"), ("GREETING", "from-env")]),
            &file,
            &extra,
        );
        assert_eq!(get(&m, "ROLE"), Some("service"));
        assert_eq!(get(&m, "GREETING"), Some("from-extra"));
        assert!(m.warnings.is_empty(), "{:?}", m.warnings);
    }

    #[test]
    fn every_merge_warns_again() {
        let file = pairs(&[("LOG_LEVEL", "info")]);
        let env = pairs(&[("DAINTY_DIRECTOR_LOG_LEVEL", "warn")]);
        assert_eq!(merge("director", &env, &file, &[]).warnings.len(), 1);
        assert_eq!(
            merge("director", &env, &file, &[]).warnings.len(),
            1,
            "a restart logs it again"
        );
    }
}
