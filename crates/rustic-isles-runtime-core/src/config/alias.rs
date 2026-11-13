//! The `MYSTICPARADOX_` to `RUSTIC_ISLES_` alias (R1 3), for launchers and Directors built before the rename.
//! R10's joint cutover (RT-8) deletes it.

/// The old prefix (case-sensitive).
pub const OLD_PREFIX: &str = "MYSTICPARADOX_";
/// The new prefix.
pub const NEW_PREFIX: &str = "RUSTIC_ISLES_";

/// The variables to set, in block order: each `MYSTICPARADOX_<X>=<v>` whose `RUSTIC_ISLES_<X>` does not exist (an
/// existing empty variable counts as existing).
pub fn alias_plan(entries: &[&str], exists: impl Fn(&str) -> bool) -> Vec<(String, String)> {
    let mut plan: Vec<(String, String)> = Vec::new();
    for entry in entries {
        let Some(rest) = entry.strip_prefix(OLD_PREFIX) else {
            continue;
        };
        let Some((suffix, value)) = rest.split_once('=') else {
            continue;
        };
        let name = format!("{NEW_PREFIX}{suffix}");
        if !exists(&name) && !plan.iter().any(|(planned, _)| *planned == name) {
            plan.push((name, value.to_owned()));
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan() {
        let entries = [
            "=C:=C:\\x",
            "MYSTICPARADOX_A=1",
            "MYSTICPARADOX_B=",
            "RUSTIC_ISLES_C=kept",
            "MYSTICPARADOX_C=1",
            "mysticparadox_d=1",
            "MYSTICPARADOX_E=a=b",
            "MYSTICPARADOX_NOEQUALS",
            "PATH=x",
        ];
        let plan = alias_plan(&entries, |name| name == "RUSTIC_ISLES_C");
        assert_eq!(
            plan,
            [
                ("RUSTIC_ISLES_A".to_owned(), "1".to_owned()),
                ("RUSTIC_ISLES_B".to_owned(), String::new()),
                ("RUSTIC_ISLES_E".to_owned(), "a=b".to_owned()),
            ]
        );
    }
}
