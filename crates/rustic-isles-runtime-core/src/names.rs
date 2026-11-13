//! UFunction full names and how dispatch handlers match them (R0 6.2, 8.3; RT-18 for the spine).

/// How one pattern matches a full name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The whole full name is equal.
    Exact,
    /// The full name starts with it.
    Prefix,
    /// The full name holds it (each slice spec lists what such a pattern catches).
    Contains,
    /// The part after the last `.` is equal.
    FunctionName,
}

/// A handler's matcher: it matches when any pattern does.
pub type Matcher = &'static [(Kind, &'static str)];

/// Whether `full_name` (`None` for a null function) matches.
pub fn matches(matcher: &[(Kind, &str)], full_name: Option<&str>) -> bool {
    let Some(name) = full_name else {
        return false;
    };
    matcher.iter().any(|&(kind, pattern)| match kind {
        Kind::Exact => name == pattern,
        Kind::Prefix => name.starts_with(pattern),
        Kind::Contains => name.contains(pattern),
        Kind::FunctionName => name.rsplit('.').next() == Some(pattern),
    })
}

/// The short form of a name: the text after its last `/`.
pub fn short(raw: &str) -> &str {
    raw.rsplit('/').next().unwrap_or(raw)
}

/// An `FName`'s text with its number: `_<n-1>` appended for a number above 0 (the exporter's answer).
pub fn with_number(text: &str, number: u32) -> String {
    if number > 0 {
        format!("{text}_{}", number - 1)
    } else {
        text.to_owned()
    }
}

/// `<class> <outer chain>.<name>`: `outers` outermost first; every part already in short form.
pub fn full_name(class: &str, outers: &[String], name: &str) -> String {
    let mut text =
        String::with_capacity(class.len() + name.len() + 1 + outers.iter().map(|o| o.len() + 1).sum::<usize>());
    text.push_str(class);
    text.push(' ');
    for outer in outers {
        text.push_str(outer);
        text.push('.');
    }
    text.push_str(name);
    text
}

/// What object 0's full name must be, or names stay off for the process (R0 call 9).
pub const SELF_CHECK: &str = "Package CoreUObject";

/// The self-check failure line.
pub fn self_check_failed_line(found: Option<&str>) -> String {
    format!(
        "[MitRuntime] name check failed: object 0 is \"{}\"",
        found.unwrap_or("null")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESTART: &str = "Function Engine.PlayerController.ClientRestart";
    const INPUT: &str = "Function BP_PlayerCharacter.BP_PlayerCharacter_C.OnInputModeChanged";

    #[test]
    fn each_kind() {
        assert!(matches(&[(Kind::Exact, RESTART)], Some(RESTART)));
        assert!(!matches(
            &[(Kind::Exact, "Function Engine.PlayerController")],
            Some(RESTART)
        ));
        assert!(matches(
            &[(Kind::Prefix, "Function Engine.PlayerController.")],
            Some(RESTART)
        ));
        assert!(!matches(&[(Kind::Prefix, "Function BP_")], Some(RESTART)));
        assert!(matches(&[(Kind::Contains, "BP_PlayerCharacter_C.")], Some(INPUT)));
        assert!(!matches(&[(Kind::Contains, "BP_PlayerCharacter_C.")], Some(RESTART)));
        assert!(matches(&[(Kind::FunctionName, "OnInputModeChanged")], Some(INPUT)));
        assert!(!matches(&[(Kind::FunctionName, "InputModeChanged")], Some(INPUT)));
        assert!(matches(
            &[(Kind::FunctionName, "Nope"), (Kind::FunctionName, "ClientRestart")],
            Some(RESTART)
        ));
        assert!(!matches(&[], Some(RESTART)));
    }

    #[test]
    fn null_never_matches() {
        assert!(!matches(&[(Kind::Contains, "")], None));
        assert!(!matches(&[(Kind::Exact, "null")], None));
    }

    #[test]
    fn building_names() {
        let outers = ["Engine".to_owned(), "PlayerController".to_owned()];
        assert_eq!(full_name("Function", &outers, "ClientRestart"), RESTART);
        assert_eq!(full_name("Package", &[], short("/Script/CoreUObject")), SELF_CHECK);
        assert_eq!(with_number("Actor", 0), "Actor");
        assert_eq!(with_number("Actor", 3), "Actor_2");
        assert_eq!(short("plain"), "plain");
        assert_eq!(
            self_check_failed_line(Some("x")),
            "[MitRuntime] name check failed: object 0 is \"x\""
        );
    }
}
