// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Repairs the function lookups of a Dumper-7 SDK dumped at or after upstream commit `42f37f6`, which
//! writes the outer class's name where the function's belongs:
//! `FindFunctionChecked(this, GetStaticName(L"BP_PlayerCharacter_C", FnName))` inside `TickStamina`.
//! The runtime then dies with "Failed to find function" as soon as a player joins.

use std::path::Path;

const COMMENT: &str = "// Function ";
const LOOKUP: &str = "GetStaticName(";

/// Rewrites every lookup in one `*_functions.cpp` whose name is the outer class instead of the function
/// named by the `// Function Package.Outer.Name` comment above it. `None` when nothing needed fixing.
#[must_use]
pub fn repair_lookups(text: &str) -> Option<(String, u32)> {
    let mut out = String::with_capacity(text.len());
    let mut current: Option<(String, String)> = None;
    let mut fixed = 0u32;
    for line in text.split_inclusive('\n') {
        if let Some(path) = line.strip_prefix(COMMENT) {
            // Only the line ending: UE names can end in a space (`CanUserStartInteraction `).
            current = outer_and_name(path.trim_end_matches(['\r', '\n']));
            out.push_str(line);
            continue;
        }
        let start = current.as_ref().and_then(|(outer, _)| outer_literal(line, outer));
        match (&current, start) {
            (Some((outer, name)), Some(start)) => {
                out.push_str(&line[..start]);
                out.push_str(name);
                out.push_str(&line[start + outer.len()..]);
                fixed = fixed.saturating_add(1);
            }
            _ => out.push_str(line),
        }
    }
    (fixed > 0).then_some((out, fixed))
}

/// Repairs every `*_functions.cpp` directly in `sdk_dir`, returning the number of lookups rewritten.
pub fn repair_sdk_dir(sdk_dir: &Path) -> std::io::Result<u32> {
    let mut fixed = 0u32;
    for entry in std::fs::read_dir(sdk_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let is_functions = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.to_ascii_lowercase().ends_with("_functions.cpp"));
        if !is_functions {
            continue;
        }
        let path = entry.path();
        // Dumper-7 writes UTF-8; anything else is left alone rather than rewritten lossily.
        let Ok(text) = String::from_utf8(std::fs::read(&path)?) else {
            continue;
        };
        if let Some((repaired, count)) = repair_lookups(&text) {
            std::fs::write(&path, repaired)?;
            fixed = fixed.saturating_add(count);
        }
    }
    Ok(fixed)
}

/// `Package.Outer.Name` → `(Outer, Name)` with quotes escaped as Dumper-7 writes them into the literal
/// (`PrefixQuotsWithBackslash`). The UE name, not the sanitised C++ identifier, is what the lookup takes.
/// `None` when the two are equal, since such a lookup is right either way.
fn outer_and_name(path: &str) -> Option<(String, String)> {
    // From the left: function names can hold dots (`GameplayCue.Combat.OnDamageGiven`), packages can't.
    let mut parts = path.splitn(3, '.');
    parts.next().filter(|s| !s.is_empty())?;
    let outer = parts.next().filter(|s| !s.is_empty())?;
    let name = parts.next().filter(|s| !s.is_empty())?;
    (outer != name).then(|| (outer.replace('"', "\\\""), name.replace('"', "\\\"")))
}

/// Byte offset of `outer` when it is the whole string in the line's `GetStaticName(L"...", ...)`, plain
/// or wrapped in the XORString macro.
fn outer_literal(line: &str, outer: &str) -> Option<usize> {
    let call = line.find(LOOKUP)? + LOOKUP.len();
    let start = call + line[call..].find("L\"")? + 2;
    line[start..]
        .strip_prefix(outer)
        .is_some_and(|rest| rest.starts_with('"'))
        .then_some(start)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BROKEN: &str = "#include \"Basic.hpp\"\r\n\
\r\n\
// Function BP_PlayerCharacter.BP_PlayerCharacter_C.TickStamina\r\n\
// (Public, BlueprintCallable, BlueprintEvent)\r\n\
\r\n\
void ABP_PlayerCharacter_C::TickStamina(float DeltaTime)\r\n\
{\r\n\
\tstatic class FName FnName;\r\n\
\tclass UFunction* Func = InSDKUtils::FindFunctionChecked(this, GetStaticName(L\"BP_PlayerCharacter_C\", FnName));\r\n\
}\r\n\
\r\n\
// Function BP_PlayerCharacter.BP_PlayerCharacter_C.Get Name\r\n\
\r\n\
void ABP_PlayerCharacter_C::Get_Name()\r\n\
{\r\n\
\tstatic class FName FnName;\r\n\
\tclass UFunction* Func = InSDKUtils::FindFunctionChecked(AsUObject(), GetStaticName(L\"BP_PlayerCharacter_C\", FnName));\r\n\
}\r\n";

    #[test]
    fn rewrites_lookups_that_name_the_outer_class() {
        let (fixed, count) = repair_lookups(BROKEN).unwrap();
        assert_eq!(count, 2);
        assert!(fixed.contains("FindFunctionChecked(this, GetStaticName(L\"TickStamina\", FnName));\r\n"));
        // The UE name from the comment, not the sanitised C++ identifier.
        assert!(fixed.contains("FindFunctionChecked(AsUObject(), GetStaticName(L\"Get Name\", FnName));\r\n"));
        assert!(!fixed.contains("L\"BP_PlayerCharacter_C\""));
    }

    #[test]
    fn leaves_a_good_sdk_alone() {
        let (good, _) = repair_lookups(BROKEN).unwrap();
        assert_eq!(repair_lookups(&good), None);
    }

    #[test]
    fn leaves_lookups_it_cannot_attribute() {
        let no_comment = "void A::F()\n{\n\tFunc = X(this, GetStaticName(L\"A\", FnName));\n}\n";
        assert_eq!(repair_lookups(no_comment), None);
        let other = "// Function P.A.F\nvoid A::F()\n{\n\tFunc = X(this, GetStaticName(L\"G\", FnName));\n}\n";
        assert_eq!(repair_lookups(other), None);
        let same = "// Function P.F.F\nvoid UF::F()\n{\n\tFunc = X(this, GetStaticName(L\"F\", FnName));\n}\n";
        assert_eq!(repair_lookups(same), None);
    }

    #[test]
    fn keeps_dots_and_trailing_spaces_in_ue_names() {
        let text = "// Function P.A.Cue.Combat.Hit\r\n\tX(this, GetStaticName(L\"A\", FnName));\r\n\
// Function P.A.CanUse \r\n\tX(this, GetStaticName(L\"A\", FnName));\r\n";
        let (fixed, count) = repair_lookups(text).unwrap();
        assert_eq!(count, 2);
        assert!(fixed.contains("GetStaticName(L\"Cue.Combat.Hit\", FnName)"));
        assert!(fixed.contains("GetStaticName(L\"CanUse \", FnName)"));
    }

    #[test]
    fn handles_xorstring_and_whole_names_only() {
        let xor = "// Function P.A.F\n\tFunc = X(this, GetStaticName(XORSTR(L\"A\"), FnName));\n";
        let (fixed, count) = repair_lookups(xor).unwrap();
        assert_eq!(count, 1);
        assert!(fixed.contains("GetStaticName(XORSTR(L\"F\"), FnName)"));
        let prefix = "// Function P.A.F\n\tFunc = X(this, GetStaticName(L\"AB\", FnName));\n";
        assert_eq!(repair_lookups(prefix), None);
    }

    #[test]
    fn repairs_function_files_in_a_directory() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("BP_PlayerCharacter_functions.cpp"), BROKEN).unwrap();
        std::fs::write(tmp.path().join("BP_PlayerCharacter_classes.hpp"), BROKEN).unwrap();
        assert_eq!(repair_sdk_dir(tmp.path()).unwrap(), 2);
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("BP_PlayerCharacter_classes.hpp")).unwrap(),
            BROKEN
        );
        assert_eq!(repair_sdk_dir(tmp.path()).unwrap(), 0);
    }
}
