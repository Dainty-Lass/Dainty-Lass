//! `secrets.env`: carried as it is, since every invite's and session's validity hangs on it. It is only read to
//! say which secrets are missing (`reforged-host setup` makes those) and, on Unix, made private to the host's user.
//! No value is ever copied, printed or logged: only the names.

use std::collections::BTreeSet;
use std::path::Path;

use crate::error::{ImportError, Result};
use crate::report::Report;

/// DaintyLass's `hostconfig::SECRET_KEYS` (plan `fb46a4a`)
pub const SECRET_KEYS: [&str; 11] = [
    "GATEWAY_SECRET",
    "ALLOWLIST_SECRET",
    "REFORGED_ADMIN_KEY",
    "GAMESERVER_API_KEY",
    "API_KEY_HASH_SECRET",
    "MYSTICPARADOX_METRICS_TOKEN",
    "AUTH_SIGNING_PRIVKEY_B64",
    "AUTH_SIGNING_PUBKEY_B64",
    "REFORGED_ADMIN_API_KEY",
    "MYSTICPARADOX_MANAGER_OPERATOR_SECRET",
    "REFORGED_DB_PASSWORD",
];

/// The names that have a non-empty value: `NAME=value` lines, `#` comments and blank lines, as the kit writes it.
fn names_with_values(text: &str) -> BTreeSet<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.strip_prefix("export ").unwrap_or(line).split_once('='))
        .filter(|(_, value)| !value.trim().trim_matches(['"', '\'']).is_empty())
        .map(|(name, _)| name.trim().to_owned())
        .collect()
}

pub fn check(file: &Path, dry_run: bool, report: &mut Report) -> Result<()> {
    if !file.is_file() {
        report.note("there is no secrets.env: reforged-host setup makes a new one, which logs every player out");
        return Ok(());
    }
    let text = std::fs::read_to_string(file).map_err(|e| ImportError::io(file, e))?;
    let present = names_with_values(&text);
    let missing: Vec<&str> = SECRET_KEYS.iter().copied().filter(|k| !present.contains(*k)).collect();
    report.unchanged("secrets.env");
    if !missing.is_empty() {
        report.note(format!(
            "secrets.env is kept; reforged-host setup adds what it lacks: {}",
            missing.join(", ")
        ));
    }
    #[cfg(unix)]
    if !dry_run {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600)).map_err(|e| ImportError::io(file, e))?;
    }
    #[cfg(not(unix))]
    let _ = dry_run;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_names_are_read_and_reported() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("secrets.env");
        std::fs::write(
            &file,
            "# made by setup\nGATEWAY_SECRET=s3cr3t-gateway\nALLOWLIST_SECRET=\nexport REFORGED_ADMIN_KEY=\"k\"\n",
        )
        .unwrap();
        let mut report = Report::new("reforged", false);
        check(&file, false, &mut report).unwrap();
        let text = report.text();
        assert!(!text.contains("s3cr3t"), "{text}");
        assert!(text.contains("ALLOWLIST_SECRET"));
        assert!(!text.contains("GATEWAY_SECRET,") && !text.contains("REFORGED_ADMIN_KEY,"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }
}
