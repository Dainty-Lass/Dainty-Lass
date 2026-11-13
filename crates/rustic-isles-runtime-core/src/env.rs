//! Environment value checks (R0 4.1, R1 2). The DLL reads the UTF-16 value; these decide what it means.

/// The variable that unlocks developer behaviour; only the launcher sets it (R0 4.1).
pub const PROFILE_VARIABLE: &str = "RUSTIC_ISLES_RUNTIME_PROFILE";

/// R0's `env_value` rule: `None` when unset, empty, over `max` units, or holding NUL, CR, LF or non-ASCII.
pub fn check_env_value(value: Option<&[u16]>, max: usize) -> Option<String> {
    let units = value?;
    if units.is_empty() || units.len() > max {
        return None;
    }
    // Non-ASCII refused: the AGPL read the ANSI form, so no setting can legitimately hold one (R0 call 4).
    if units
        .iter()
        .any(|&unit| unit == 0 || unit == u16::from(b'\r') || unit == u16::from(b'\n') || unit > 0x7F)
    {
        return None;
    }
    Some(units.iter().map(|&unit| char::from(unit as u8)).collect())
}

/// Whether the profile variable's checked value selects the development profile (exactly `development`).
pub fn is_development(value: Option<&str>) -> bool {
    value == Some("development")
}

/// R1's raw read: unset (absent or empty), too long, or the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Raw {
    /// Absent or empty.
    Unset,
    /// More than the cap; the value's length in UTF-16 units.
    TooLong(usize),
    /// The value, lossless from UTF-16 (an unpaired surrogate is U+FFFD, which every grammar refuses).
    Value(String),
}

impl Raw {
    /// Classifies a UTF-16 value against `cap` units.
    pub fn from_units(value: Option<&[u16]>, cap: usize) -> Raw {
        match value {
            None => Raw::Unset,
            Some([]) => Raw::Unset,
            Some(units) if units.len() > cap => Raw::TooLong(units.len()),
            Some(units) => Raw::Value(String::from_utf16_lossy(units)),
        }
    }

    /// Convenience for tests and callers holding text.
    pub fn from_text(value: Option<&str>, cap: usize) -> Raw {
        let units: Option<Vec<u16>> = value.map(|text| text.encode_utf16().collect());
        Raw::from_units(units.as_deref(), cap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    fn check(text: &str, max: usize) -> Option<String> {
        check_env_value(Some(&units(text)), max)
    }

    #[test]
    fn env_value_rules() {
        assert_eq!(check_env_value(None, 8), None);
        assert_eq!(check("", 8), None);
        assert_eq!(check("12345678", 8).as_deref(), Some("12345678"));
        assert_eq!(check("123456789", 8), None);
        assert_eq!(check("a\0b", 8), None);
        assert_eq!(check("a\rb", 8), None);
        assert_eq!(check("a\nb", 8), None);
        assert_eq!(check("caf\u{e9}", 8), None);
        assert_eq!(check("a b\t", 8).as_deref(), Some("a b\t"));
    }

    #[test]
    fn development_profile_is_exact() {
        assert!(is_development(check("development", 32).as_deref()));
        assert!(!is_development(check("Development", 32).as_deref()));
        assert!(!is_development(check("development ", 32).as_deref()));
        assert!(!is_development(check("production", 32).as_deref()));
        assert!(!is_development(None));
    }

    #[test]
    fn raw_reads() {
        assert_eq!(Raw::from_text(None, 4), Raw::Unset);
        assert_eq!(Raw::from_text(Some(""), 4), Raw::Unset);
        assert_eq!(Raw::from_text(Some("abcd"), 4), Raw::Value("abcd".into()));
        assert_eq!(Raw::from_text(Some("abcde"), 4), Raw::TooLong(5));
        assert_eq!(
            Raw::from_units(Some(&[0xD800, 0x41]), 4),
            Raw::Value("\u{fffd}A".into())
        );
    }
}
