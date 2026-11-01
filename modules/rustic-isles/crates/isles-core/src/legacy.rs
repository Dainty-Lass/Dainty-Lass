//! The names from before the Rustic Isles rename (2026-11-01), still accepted for one release so existing env files,
//! runtimes and launchers keep working. Remove with the release after.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, OnceLock};

use axum::http::{HeaderMap, HeaderName};

pub const ENV_PREFIX: &str = "RUSTIC_ISLES_";
pub const LEGACY_ENV_PREFIX: &str = "MYSTICPARADOX_";
pub const HEADER_PREFIX: &str = "x-rustic-isles-";
pub const LEGACY_HEADER_PREFIX: &str = "x-mysticparadox-";
pub const RUNTIME_DLL: &str = "Rustic-Isles.dll";
pub const LEGACY_RUNTIME_DLL: &str = "MysticParadox.dll";

/// The `RUSTIC_ISLES_*` name for a `MYSTICPARADOX_*` one.
pub fn renamed_env(name: &str) -> Option<String> {
    name.strip_prefix(LEGACY_ENV_PREFIX)
        .map(|rest| format!("{ENV_PREFIX}{rest}"))
}

/// The `MYSTICPARADOX_*` name a runtime from before the rename reads for a `RUSTIC_ISLES_*` one.
pub fn legacy_env(name: &str) -> Option<String> {
    name.strip_prefix(ENV_PREFIX)
        .map(|rest| format!("{LEGACY_ENV_PREFIX}{rest}"))
}

/// Gives each `MYSTICPARADOX_*` variable the new name doesn't set its `RUSTIC_ISLES_*` name. The old key stays, so a
/// game server running an older runtime still finds it. One warning per name per process, naming never the value.
pub fn accept_legacy_env(env: &mut BTreeMap<String, String>) {
    let renamed: Vec<(String, String)> = env
        .iter()
        .filter_map(|(name, value)| Some((renamed_env(name)?, value.clone())))
        .filter(|(name, _)| !env.contains_key(name))
        .collect();
    for (name, value) in renamed {
        warn_once(&name);
        env.insert(name, value);
    }
}

fn warn_once(name: &str) {
    static WARNED: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
    let mut warned = WARNED
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if warned.insert(name.to_owned()) {
        let old = legacy_env(name).unwrap_or_default();
        tracing::warn!("{old} is deprecated and will stop working in the next release: rename it to {name}");
    }
}

/// Copies each `x-mysticparadox-*` header the request doesn't also send under its `x-rustic-isles-*` name, so the
/// handlers read only the new names.
pub fn accept_legacy_headers(headers: &mut HeaderMap) {
    let renamed: Vec<(HeaderName, axum::http::HeaderValue)> = headers
        .iter()
        .filter_map(|(name, value)| {
            let rest = name.as_str().strip_prefix(LEGACY_HEADER_PREFIX)?;
            let new = HeaderName::try_from(format!("{HEADER_PREFIX}{rest}")).ok()?;
            Some((new, value.clone()))
        })
        .filter(|(name, _)| !headers.contains_key(name))
        .collect();
    for (name, value) in renamed {
        headers.append(name, value);
    }
}

/// Whether `name` is the runtime DLL, under either name (Windows file names ignore case).
pub fn is_runtime_dll(name: &str) -> bool {
    name.eq_ignore_ascii_case(RUNTIME_DLL) || name.eq_ignore_ascii_case(LEGACY_RUNTIME_DLL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_env_names_are_read_under_the_new_ones() {
        let mut env = BTreeMap::from([
            ("MYSTICPARADOX_METRICS_TOKEN".to_owned(), "old".to_owned()),
            ("MYSTICPARADOX_GUARD_MODE".to_owned(), "observe".to_owned()),
            ("RUSTIC_ISLES_GUARD_MODE".to_owned(), "enforce".to_owned()),
        ]);
        accept_legacy_env(&mut env);
        assert_eq!(env["RUSTIC_ISLES_METRICS_TOKEN"], "old");
        assert_eq!(env["RUSTIC_ISLES_GUARD_MODE"], "enforce", "the new name wins");
        assert_eq!(env["MYSTICPARADOX_METRICS_TOKEN"], "old", "kept for older runtimes");
    }

    #[test]
    fn old_headers_are_read_under_the_new_ones() {
        let mut headers = HeaderMap::new();
        headers.insert("x-mysticparadox-gameserver-apikey", "old".parse().unwrap());
        headers.insert("x-mysticparadox-request-id", "old-request".parse().unwrap());
        headers.insert("x-rustic-isles-request-id", "new-request".parse().unwrap());
        accept_legacy_headers(&mut headers);
        assert_eq!(headers["x-rustic-isles-gameserver-apikey"], "old");
        assert_eq!(headers["x-rustic-isles-request-id"], "new-request");
        assert_eq!(headers.get_all("x-rustic-isles-request-id").iter().count(), 1);
    }

    #[test]
    fn settings_read_an_old_name_under_the_new_one() {
        let settings = crate::settings::Settings::from_pairs([("MYSTICPARADOX_RATE_LIMIT", "off")]);
        assert!(!settings.rate_limit_enabled());
    }

    #[test]
    fn both_dll_names_are_the_runtime() {
        assert!(is_runtime_dll("Rustic-Isles.dll"));
        assert!(is_runtime_dll("mysticparadox.dll"));
        assert!(!is_runtime_dll("winmm.dll"));
    }
}
