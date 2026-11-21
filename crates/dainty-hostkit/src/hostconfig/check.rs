// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The early check (RD24 phase 2): the gateway's, the allowlist helper's and content's own loaders run on the
//! env each would start with, the overrides applied, so a bad value stops `setup` with the service's message
//! rather than a crash loop at start. The backend's and the Director's loaders take any value; their tables are
//! checked as settings.toml is read (`tunables.rs`).

use dainty_service_kit::env::Env;

use super::{EnvFiles, overrides, tunables};
use crate::error::HostError;

/// What the check found: each service's override warnings, for setup to print.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Checked {
    /// One line per override or dropped generic variable, every service's.
    pub warnings: Vec<String>,
}

type Loader = fn(&Env) -> Result<(), String>;

fn gateway(env: &Env) -> Result<(), String> {
    dainty_gateway::config::Config::from_env(env).map(drop)
}

fn allowlist(env: &Env) -> Result<(), String> {
    dainty_gateway::allowlist::config::AllowlistConfig::from_env(env).map(drop)
}

fn content(env: &Env) -> Result<(), String> {
    dainty_content::config::Config::from_env(env).map(drop)
}

fn loader(service: &str) -> Option<Loader> {
    match service {
        "gateway" => Some(gateway),
        "allowlist" => Some(allowlist),
        "content" => Some(content),
        _ => None,
    }
}

/// Where the variable a loader's message names was set: the settings.toml key, or the environment.
fn hint(service: &str, message: &str, overridden: &[String]) -> String {
    let Some(t) = tunables::table(service) else {
        return String::new();
    };
    let named = |key: &str| {
        message
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .any(|w| w == key)
    };
    if let Some(key) = overridden.iter().find(|k| named(k)) {
        return format!(" ({key} is set in the environment, over settings.toml)");
    }
    t.settings
        .iter()
        .find(|s| named(s.env))
        .and_then(|s| tunables::toml_key(service, s.env))
        .map_or_else(String::new, |key| format!(" (settings.toml: {key})"))
}

/// One service's file merged with `inherited` as `start` will: its override warnings, and its loader's
/// refusal, if any, naming where the bad value was set.
pub fn service(
    service: &str,
    file: &[(String, String)],
    inherited: &[(String, String)],
) -> (Vec<String>, Option<String>) {
    let merged = overrides::merge(service, inherited, file, &[]);
    let refused = loader(service).and_then(|load| {
        let env = Env::from_pairs(merged.env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        load(&env)
            .err()
            .map(|message| format!("{service}: {message}{}", hint(service, &message, &merged.overridden)))
    });
    (merged.warnings, refused)
}

/// Merges each file with `inherited` as `start` will and runs the services' loaders on the result.
pub fn check(files: &EnvFiles, inherited: &[(String, String)]) -> Result<Checked, HostError> {
    let mut out = Checked::default();
    let mut errors = Vec::new();
    for (name, file) in files.files() {
        let Some(file) = file else { continue };
        let (warnings, refused) = service(name, file, inherited);
        out.warnings.extend(warnings);
        errors.extend(refused);
    }
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(HostError::message(format!(
            "these settings would stop a service from starting:\n  {}",
            errors.join("\n  ")
        )))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;

    const SECRET: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn files(gateway: &[(&str, &str)]) -> EnvFiles {
        let mut g: Vec<(String, String)> = vec![
            ("GATEWAY_TLS".into(), "0".into()),
            ("GATEWAY_BIND".into(), "127.0.0.1".into()),
            ("GATEWAY_SECRET".into(), SECRET.into()),
            ("GATEWAY_ALLOWLIST".into(), "0".into()),
        ];
        g.extend(gateway.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())));
        EnvFiles {
            backend: vec![("LOG_LEVEL".into(), "info".into())],
            director: Vec::new(),
            content: Vec::new(),
            gateway: g,
            admin: Vec::new(),
            allowlist: None,
            discord: None,
            passthrough_refused: Vec::new(),
        }
    }

    #[test]
    fn good_files_pass_and_report_the_overrides() {
        let inherited = vec![("DAINTY_BACKEND_LOG_LEVEL".to_owned(), "debug".to_owned())];
        let checked = check(&files(&[("GATEWAY_RATE_WS", "40,24")]), &inherited).unwrap();
        assert_eq!(checked.warnings.len(), 1, "{:?}", checked.warnings);
    }

    #[test]
    fn a_bad_value_fails_with_the_service_message_and_its_settings_toml_key() {
        let err = check(&files(&[("GATEWAY_MAX_BODY_BYTES", "10")]), &[])
            .unwrap_err()
            .to_string();
        assert!(err.contains("gateway: GATEWAY_MAX_BODY_BYTES must be"), "{err}");
        assert!(err.contains("(settings.toml: gateway.limits.max_body_bytes)"), "{err}");
    }

    #[test]
    fn a_bad_override_fails_naming_the_environment() {
        let inherited = vec![("GATEWAY_RATE_WS".to_owned(), "fast".to_owned())];
        let err = check(&files(&[]), &inherited).unwrap_err().to_string();
        assert!(err.contains("GATEWAY_RATE_WS must look like"), "{err}");
        assert!(err.contains("set in the environment"), "{err}");
    }
}
