// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Launcher versions as Semantic Versioning 2.0 orders them, for the minimum a server asks of launchers
//! (`minLauncherVersion` in the status): `2.0.0-beta` is below `2.0.0`, and build metadata is ignored.

use std::cmp::Ordering;

/// The longest version text accepted.
pub const MAX_LAUNCHER_VERSION_LEN: usize = 64;

/// A pre-release identifier: numeric ones sort below alphanumeric ones.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ident {
    Number(u64),
    Text(String),
}

impl Ord for Ident {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Number(a), Self::Number(b)) => a.cmp(b),
            (Self::Number(_), Self::Text(_)) => Ordering::Less,
            (Self::Text(_), Self::Number(_)) => Ordering::Greater,
            (Self::Text(a), Self::Text(b)) => a.as_bytes().cmp(b.as_bytes()),
        }
    }
}

impl PartialOrd for Ident {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A parsed `MAJOR.MINOR.PATCH[-pre][+build]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherVersion {
    core: [u64; 3],
    pre: Vec<Ident>,
}

impl Ord for LauncherVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core
            .cmp(&other.core)
            .then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                // A release sorts above its pre-releases
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => self.pre.cmp(&other.pre),
            })
    }
}

impl PartialOrd for LauncherVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn number(part: &str) -> Option<u64> {
    let leading_zero = part.len() > 1 && part.starts_with('0');
    if part.is_empty() || leading_zero || !part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    part.parse().ok()
}

fn ident_chars(part: &str) -> bool {
    !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// `text` as a launcher version, or none when it is not strict Semantic Versioning.
#[must_use]
pub fn parse_launcher_version(text: &str) -> Option<LauncherVersion> {
    if text.is_empty() || text.len() > MAX_LAUNCHER_VERSION_LEN {
        return None;
    }
    let (rest, build) = match text.split_once('+') {
        Some((rest, build)) => (rest, Some(build)),
        None => (text, None),
    };
    if build.is_some_and(|b| !b.split('.').all(ident_chars)) {
        return None;
    }
    let (core_text, pre_text) = match rest.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (rest, None),
    };
    let mut parts = core_text.split('.');
    let mut core = [0u64; 3];
    for slot in &mut core {
        *slot = number(parts.next()?)?;
    }
    if parts.next().is_some() {
        return None;
    }
    let mut pre = Vec::new();
    if let Some(pre_text) = pre_text {
        for part in pre_text.split('.') {
            if !ident_chars(part) {
                return None;
            }
            if part.bytes().all(|b| b.is_ascii_digit()) {
                pre.push(Ident::Number(number(part)?));
            } else {
                pre.push(Ident::Text(part.to_owned()));
            }
        }
    }
    Some(LauncherVersion { core, pre })
}

/// Whether a launcher at `running` is below `minimum`. An unreadable minimum asks for nothing, and an unreadable
/// running version is let through: a bad value must not lock every player out.
#[must_use]
pub fn below_minimum(running: &str, minimum: &str) -> bool {
    match (parse_launcher_version(running), parse_launcher_version(minimum)) {
        (Some(running), Some(minimum)) => running < minimum,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_precedence() {
        // The specification's own example chain, lowest first
        let chain = [
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
            "1.0.1",
            "1.1.0",
            "2.0.0-beta",
            "2.0.0",
            "10.0.0",
        ];
        for pair in chain.windows(2) {
            assert!(below_minimum(pair[0], pair[1]), "{} < {}", pair[0], pair[1]);
            assert!(!below_minimum(pair[1], pair[0]), "{} > {}", pair[1], pair[0]);
        }
        assert!(!below_minimum("2.0.0-beta", "2.0.0-beta"));
        assert!(!below_minimum("2.0.0+abc", "2.0.0+def"), "build metadata is ignored");
        assert!(below_minimum("2.0.0-beta+7", "2.0.0"));
    }

    #[test]
    fn only_strict_versions_parse() {
        for good in [
            "0.0.0",
            "2.0.0-beta",
            "1.2.3-rc.1+build.5",
            "1.2.3-x-y",
            "1.2.3+sha.0abc",
        ] {
            assert!(parse_launcher_version(good).is_some(), "{good}");
        }
        for bad in [
            "",
            "2",
            "2.0",
            "2.0.0.0",
            "v2.0.0",
            "02.0.0",
            "2.0.0-",
            "2.0.0-01",
            "2.0.0-a..b",
            "2.0.0+",
            " 2.0.0",
            "2.0.0 ",
            "2.0.0-ü",
            "x.y.z",
        ] {
            assert!(parse_launcher_version(bad).is_none(), "{bad:?}");
        }
        assert!(parse_launcher_version(&format!("1.0.0-{}", "a".repeat(MAX_LAUNCHER_VERSION_LEN))).is_none());
    }

    #[test]
    fn a_bad_value_locks_no_one_out() {
        assert!(!below_minimum("2.0.0", "nonsense"));
        assert!(!below_minimum("dev", "9.9.9"));
        assert!(!below_minimum("2.0.0", ""));
    }
}
