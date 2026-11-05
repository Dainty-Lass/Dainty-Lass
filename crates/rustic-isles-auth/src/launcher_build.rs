// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Launcher builds (spec §6.7): the report a launch carries, its verdict, the host's policy and version order.

use std::cmp::Ordering;

use serde_json::Value;

use crate::AuthSettings;
use crate::util;

/// A well-formed `launcher` report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherReport {
    pub version: String,
    /// Lower-cased
    pub sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LauncherVerdict {
    Official,
    Outdated,
    Unofficial,
    Unknown,
}

impl LauncherVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Official => "official",
            Self::Outdated => "outdated",
            Self::Unofficial => "unofficial",
            Self::Unknown => "unknown",
        }
    }
}

fn is_digits(text: &str, max: usize) -> bool {
    (1..=max).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit())
}

/// Three groups of 1 to 5 digits, optionally `-` and 1 to 32 of `A-Za-z0-9.-`.
fn is_report_version(text: &str) -> bool {
    let (core, pre) = match text.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (text, None),
    };
    let groups: Vec<&str> = core.split('.').collect();
    let pre_ok = pre.is_none_or(|p| {
        (1..=32).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    });
    groups.len() == 3 && groups.iter().all(|g| is_digits(g, 5)) && pre_ok
}

/// Anything malformed is no report, never a refusal.
pub fn parse_launcher_report(value: Option<&Value>) -> Option<LauncherReport> {
    let Some(Value::Object(report)) = value else {
        return None;
    };
    let version = report.get("version")?.as_str()?;
    let sha256 = report.get("sha256")?.as_str()?;
    (is_report_version(version) && util::is_hex64(sha256)).then(|| LauncherReport {
        version: version.to_owned(),
        sha256: sha256.to_lowercase(),
    })
}

/// Official on the pinned hash; outdated below the served version; else unofficial; unknown without a report or pin.
pub fn launcher_verdict(
    report: Option<&LauncherReport>,
    pinned: Option<&str>,
    served: Option<&str>,
) -> LauncherVerdict {
    let (Some(report), Some(pinned)) = (report, pinned) else {
        return LauncherVerdict::Unknown;
    };
    if report.sha256.eq_ignore_ascii_case(pinned) {
        return LauncherVerdict::Official;
    }
    if let Some(served) = served
        && compare_versions(&report.version, served) == Some(Ordering::Less)
    {
        return LauncherVerdict::Outdated;
    }
    LauncherVerdict::Unofficial
}

/// The host's `RUSTIC_ISLES_UNOFFICIAL_LAUNCHER*` settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherPolicy {
    pub refuse: bool,
    pub allowed_hashes: Vec<String>,
    pub admins_exempt: bool,
    pub warnings: Vec<String>,
}

impl LauncherPolicy {
    pub fn from_settings(settings: &dyn AuthSettings) -> Self {
        let mut warnings = Vec::new();
        let mode = util::normalize_lower(settings.raw("RUSTIC_ISLES_UNOFFICIAL_LAUNCHER").unwrap_or(""));
        let refuse = match mode.as_str() {
            "refuse" => true,
            "" | "observe" => false,
            other => {
                let shown: String = other.chars().take(32).collect();
                warnings.push(format!(
                    "RUSTIC_ISLES_UNOFFICIAL_LAUNCHER must be observe or refuse (got \"{shown}\"); observing"
                ));
                false
            }
        };
        let mut allowed_hashes = Vec::new();
        let mut malformed = 0usize;
        for item in settings
            .raw("RUSTIC_ISLES_LAUNCHER_ALLOWED_HASHES")
            .unwrap_or("")
            .split(|c: char| c == ',' || service_kit::js::is_space(c))
            .filter(|t| !t.is_empty())
        {
            let hash = item.to_lowercase();
            if util::is_hex64(&hash) {
                allowed_hashes.push(hash);
            } else {
                malformed += 1;
            }
        }
        if malformed > 0 {
            warnings.push(format!(
                "RUSTIC_ISLES_LAUNCHER_ALLOWED_HASHES ignored {malformed} value(s) that are not SHA-256 hashes"
            ));
        }
        let admins_exempt = !matches!(
            util::normalize_lower(
                settings
                    .raw("RUSTIC_ISLES_UNOFFICIAL_LAUNCHER_TESTERS_EXEMPT")
                    .unwrap_or("")
            )
            .as_str(),
            "0" | "false" | "off" | "no"
        );
        Self {
            refuse,
            allowed_hashes,
            admins_exempt,
            warnings,
        }
    }

    /// Only an unofficial verdict is refused, and not for an allowed hash or an exempt admin.
    pub fn refuses(&self, verdict: LauncherVerdict, report: Option<&LauncherReport>, roles: &[String]) -> bool {
        if !self.refuse || verdict != LauncherVerdict::Unofficial {
            return false;
        }
        if report.is_some_and(|r| self.allowed_hashes.contains(&r.sha256)) {
            return false;
        }
        !(self.admins_exempt && roles.iter().any(|r| r == "admin"))
    }
}

struct Semver<'a> {
    core: [&'a str; 3],
    pre: Vec<&'a str>,
}

fn parse_semver(text: &str) -> Option<Semver<'_>> {
    let ident_ok = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    let text = match text.split_once('+') {
        Some((head, build)) if build.split('.').all(ident_ok) => head,
        Some(_) => return None,
        None => text,
    };
    let (core, pre) = match text.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (text, None),
    };
    let mut parts = core.split('.');
    let (Some(a), Some(b), Some(c), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return None;
    };
    if ![a, b, c]
        .iter()
        .all(|p| !p.is_empty() && p.bytes().all(|d| d.is_ascii_digit()))
    {
        return None;
    }
    let pre: Vec<&str> = match pre {
        Some(p) => p.split('.').collect(),
        None => Vec::new(),
    };
    if !pre.iter().all(|s| ident_ok(s)) {
        return None;
    }
    Some(Semver { core: [a, b, c], pre })
}

// Decimal digit strings as big integers
fn compare_numeric(a: &str, b: &str) -> Ordering {
    let a = a.trim_start_matches('0');
    let b = b.trim_start_matches('0');
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

/// Semver precedence; `None` when either side doesn't parse.
pub fn compare_versions(a: &str, b: &str) -> Option<Ordering> {
    let (a, b) = (parse_semver(a)?, parse_semver(b)?);
    for (x, y) in a.core.iter().zip(b.core.iter()) {
        let order = compare_numeric(x, y);
        if order != Ordering::Equal {
            return Some(order);
        }
    }
    match (a.pre.is_empty(), b.pre.is_empty()) {
        (true, true) => return Some(Ordering::Equal),
        (true, false) => return Some(Ordering::Greater),
        (false, true) => return Some(Ordering::Less),
        (false, false) => {}
    }
    for (x, y) in a.pre.iter().zip(b.pre.iter()) {
        let numeric = |s: &str| s.bytes().all(|d| d.is_ascii_digit());
        let order = match (numeric(x), numeric(y)) {
            (true, true) => compare_numeric(x, y),
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => x.cmp(y),
        };
        if order != Ordering::Equal {
            return Some(order);
        }
    }
    Some(a.pre.len().cmp(&b.pre.len()))
}

/// Whether `version` is at least `minimum`; unparseable is never.
pub fn version_at_least(version: &str, minimum: &str) -> bool {
    matches!(
        compare_versions(version, minimum),
        Some(Ordering::Greater | Ordering::Equal)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::MapSettings;
    use serde_json::json;

    #[test]
    fn versions_order_by_semver() {
        assert!(version_at_least("0.1.35", "0.1.35"));
        assert!(version_at_least("0.1.35+build.7", "0.1.35"));
        assert!(!version_at_least("0.1.35-rc.1", "0.1.35"));
        assert!(version_at_least("0.1.35-rc.2", "0.1.35-rc.1"));
        assert!(!version_at_least("0.1.35-alpha", "0.1.35-rc.1"));
        assert!(version_at_least("999999999999999999.0.0", "2.0.0"));
        assert!(!version_at_least("not-a-version", "0.0.1"));
    }

    #[test]
    fn reports_parse_or_are_dropped() {
        let hash = "A".repeat(64);
        let report = parse_launcher_report(Some(&json!({"version": "0.1.68", "sha256": hash}))).unwrap();
        assert_eq!(report.sha256, "a".repeat(64));
        assert!(parse_launcher_report(Some(&json!({"version": "0.1.68-rc.1", "sha256": hash}))).is_some());
        for bad in [
            json!({"version": "0.1", "sha256": hash}),
            json!({"version": "0.1.68", "sha256": "abc"}),
            json!([]),
            json!("0.1.68"),
            json!({"version": 68, "sha256": hash}),
        ] {
            assert!(parse_launcher_report(Some(&bad)).is_none());
        }
    }

    #[test]
    fn verdicts() {
        let pinned = "a1".repeat(32);
        let report = |version: &str, sha: &str| LauncherReport {
            version: version.into(),
            sha256: sha.into(),
        };
        let other = "b".repeat(64);
        let served = Some("0.1.68");
        let v = |r: Option<&LauncherReport>, p: Option<&str>| launcher_verdict(r, p, served);
        assert_eq!(
            v(Some(&report("0.1.60", &pinned)), Some(&pinned)),
            LauncherVerdict::Official
        );
        assert_eq!(
            v(Some(&report("0.1.67", &other)), Some(&pinned)),
            LauncherVerdict::Outdated
        );
        assert_eq!(
            v(Some(&report("0.1.68", &other)), Some(&pinned)),
            LauncherVerdict::Unofficial
        );
        assert_eq!(
            v(Some(&report("0.1.69", &other)), Some(&pinned)),
            LauncherVerdict::Unofficial
        );
        assert_eq!(v(None, Some(&pinned)), LauncherVerdict::Unknown);
        assert_eq!(v(Some(&report("0.1.69", &other)), None), LauncherVerdict::Unknown);
    }

    #[test]
    fn policy() {
        let read = |pairs: &[(&str, &str)]| LauncherPolicy::from_settings(&MapSettings::new(pairs));
        let default = read(&[]);
        assert!(!default.refuse && default.admins_exempt && default.warnings.is_empty());
        assert!(read(&[("RUSTIC_ISLES_UNOFFICIAL_LAUNCHER", " Refuse ")]).refuse);
        let typo = read(&[("RUSTIC_ISLES_UNOFFICIAL_LAUNCHER", "refuze")]);
        assert!(!typo.refuse && typo.warnings.len() == 1);
        assert!(!read(&[("RUSTIC_ISLES_UNOFFICIAL_LAUNCHER_TESTERS_EXEMPT", "off")]).admins_exempt);
        let upper = "C".repeat(64);
        let hashes = read(&[("RUSTIC_ISLES_LAUNCHER_ALLOWED_HASHES", &format!("{upper}, nope"))]);
        assert_eq!(hashes.allowed_hashes, ["c".repeat(64)]);
        assert_eq!(hashes.warnings.len(), 1);

        let allowed = "c".repeat(64);
        let refuse = read(&[
            ("RUSTIC_ISLES_UNOFFICIAL_LAUNCHER", "refuse"),
            ("RUSTIC_ISLES_LAUNCHER_ALLOWED_HASHES", &allowed),
        ]);
        let player = vec!["player".to_owned()];
        let admin = vec!["admin".to_owned()];
        let report = LauncherReport {
            version: "0.1.68".into(),
            sha256: "d".repeat(64),
        };
        assert!(refuse.refuses(LauncherVerdict::Unofficial, Some(&report), &player));
        for verdict in [
            LauncherVerdict::Official,
            LauncherVerdict::Outdated,
            LauncherVerdict::Unknown,
        ] {
            assert!(!refuse.refuses(verdict, Some(&report), &player));
        }
        let allowed_report = LauncherReport {
            version: "0.1.68".into(),
            sha256: allowed,
        };
        assert!(!refuse.refuses(LauncherVerdict::Unofficial, Some(&allowed_report), &player));
        assert!(!refuse.refuses(LauncherVerdict::Unofficial, Some(&report), &admin));
        let strict = read(&[
            ("RUSTIC_ISLES_UNOFFICIAL_LAUNCHER", "refuse"),
            ("RUSTIC_ISLES_UNOFFICIAL_LAUNCHER_TESTERS_EXEMPT", "0"),
        ]);
        assert!(strict.refuses(LauncherVerdict::Unofficial, Some(&report), &admin));
    }
}
