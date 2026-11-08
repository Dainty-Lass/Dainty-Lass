//! The names DaintyLass gave its wire after the Reforged Revival line (2026-11-06, `chore/dainty-protocol-names`):
//! what a recording made under the old names says now. The parity harnesses read the Node reference's recordings
//! through [`renamed`] so they stay as recorded (RD2) while the services answer under the new names.

/// Old spelling, new spelling: applied in order, each one everywhere it occurs.
pub const RENAMES: &[(&str, &str)] = &[
    ("x-reforged-", "x-dainty-"),
    ("X-Reforged-", "X-Dainty-"),
    ("/reforged/api", "/dainty/api"),
    ("REFORGED_", "DAINTY_"),
    ("RRK_", "DLK_"),
    ("RRA_", "DLA_"),
];

/// `name` as a recording made under the old names spells it: [`renamed`] backwards, for a test double that reports
/// what it was sent the way the recording does.
pub fn as_recorded(name: &str) -> String {
    RENAMES
        .iter()
        .fold(name.to_owned(), |name, (old, new)| name.replace(new, old))
}

/// `text` with every name in [`RENAMES`] in its new spelling.
pub fn renamed(text: &str) -> String {
    RENAMES
        .iter()
        .fold(text.to_owned(), |text, (old, new)| text.replace(old, new))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recording_reads_under_the_new_names() {
        let recorded = r#"{"headers":{"x-reforged-user-api-key":"<key>","X-Reforged-Gateway":"s"}}"#;
        assert_eq!(
            renamed(recorded),
            r#"{"headers":{"x-dainty-user-api-key":"<key>","X-Dainty-Gateway":"s"}}"#
        );
        assert_eq!(renamed("nothing to do"), "nothing to do");
        assert_eq!(as_recorded("x-dainty-gateway"), "x-reforged-gateway");
        assert_eq!(as_recorded(&renamed("x-reforged-admin-key")), "x-reforged-admin-key");
    }
}
