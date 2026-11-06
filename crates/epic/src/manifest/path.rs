//! A manifest's file names, kept inside the install folder. The rules are the strictest of Windows and Linux, so a
//! name means the same file on both and never another place: no root, drive, UNC or `\\?\` prefix, no `..`, no
//! stream (`file:stream`), no device (`NUL.txt`), and no component Windows would silently rename.

use std::fmt;
use std::path::{Path, PathBuf};

/// Why a manifest's file name was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PathProblem {
    /// The name is empty.
    #[error("the name is empty")]
    Empty,
    /// The name starts with `/`.
    #[error("the name starts at a root")]
    Absolute,
    /// The name has a backslash.
    #[error("the name has a backslash")]
    Backslash,
    /// The name has a colon: a drive or a stream.
    #[error("the name has a colon (a drive or a stream)")]
    Colon,
    /// The name has a control character or one of `<`, `>`, `"`, `|`, `?` and `*`.
    #[error("the name has a control character or one of < > \" | ? *")]
    Character,
    /// Two slashes in a row, or a trailing one.
    #[error("the name has an empty component")]
    EmptyComponent,
    /// A `.` or `..` component.
    #[error("the name has a . or .. component")]
    DotComponent,
    /// A component ends in a dot or a space, which Windows drops.
    #[error("a component ends in a dot or a space")]
    TrailingDotOrSpace,
    /// A component is a Windows device name (`CON`, `NUL.txt`).
    #[error("a component is a Windows device name")]
    DeviceName,
}

/// A file name from a manifest that passed [`InstallPath::new`]: relative, `/`-separated, inside the folder.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InstallPath(String);

impl InstallPath {
    /// Checks a manifest's file name against the rules above.
    pub fn new(name: &str) -> Result<Self, PathProblem> {
        if name.is_empty() {
            return Err(PathProblem::Empty);
        }
        if name.starts_with('/') {
            return Err(PathProblem::Absolute);
        }
        if name.contains('\\') {
            return Err(PathProblem::Backslash);
        }
        if name.contains(':') {
            return Err(PathProblem::Colon);
        }
        if name
            .chars()
            .any(|c| c.is_control() || matches!(c, '<' | '>' | '"' | '|' | '?' | '*'))
        {
            return Err(PathProblem::Character);
        }
        for component in name.split('/') {
            check_component(component)?;
        }
        Ok(Self(name.to_owned()))
    }

    /// The name as the manifest gives it, `/`-separated.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The name's components, in order.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }

    /// The file's place under `root`, joined one component at a time.
    pub fn under(&self, root: &Path) -> PathBuf {
        let mut path = root.to_path_buf();
        path.extend(self.components());
        path
    }
}

impl fmt::Debug for InstallPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Display for InstallPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn check_component(component: &str) -> Result<(), PathProblem> {
    if component.is_empty() {
        return Err(PathProblem::EmptyComponent);
    }
    if component == "." || component == ".." {
        return Err(PathProblem::DotComponent);
    }
    // Windows drops these, so `a.` and `a` would be one file
    if component.ends_with('.') || component.ends_with(' ') {
        return Err(PathProblem::TrailingDotOrSpace);
    }
    let stem = component.split('.').next().unwrap_or(component).trim_end_matches(' ');
    if is_device(stem) {
        return Err(PathProblem::DeviceName);
    }
    Ok(())
}

/// Windows opens these as devices whatever the folder or extension.
fn is_device(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$") {
        return true;
    }
    let mut chars = upper.chars();
    let prefix: String = chars.by_ref().take(3).collect();
    let rest: String = chars.collect();
    (prefix == "COM" || prefix == "LPT")
        && matches!(
            rest.as_str(),
            "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_names_pass() {
        for name in [
            "Dauntless.exe",
            "Version.txt",
            "Archon/Binaries/Win64/Dauntless-Win64-Shipping.exe",
            "Archon/Content/Paks/pakchunk0-WindowsNoEditor.pak",
            "Engine/Extras/Redist/en-us/UE4PrereqSetup_x64.exe",
            ".egstore/x",
            "a/b c/d",
            "CONSOLE.txt",
            "COM10",
            "über/ファイル.bin",
        ] {
            let path = InstallPath::new(name).unwrap_or_else(|p| panic!("{name}: {p}"));
            assert_eq!(path.as_str(), name);
        }
    }

    #[test]
    fn names_that_leave_the_folder_or_alias_a_file_are_refused() {
        use PathProblem::*;
        for (name, problem) in [
            ("", Empty),
            ("/etc/passwd", Absolute),
            ("//server/share/x", Absolute),
            ("\\\\server\\share\\x", Backslash),
            ("\\\\?\\C:\\x", Backslash),
            ("..\\x", Backslash),
            ("C:/Windows/x.dll", Colon),
            ("C:x", Colon),
            ("file.txt:stream", Colon),
            ("..", DotComponent),
            ("../x", DotComponent),
            ("a/../../x", DotComponent),
            ("a/./b", DotComponent),
            ("a//b", EmptyComponent),
            ("a/", EmptyComponent),
            ("a.", TrailingDotOrSpace),
            ("a /b", TrailingDotOrSpace),
            ("NUL", DeviceName),
            ("x/nul.txt", DeviceName),
            ("Con .log", DeviceName),
            ("COM1.dll", DeviceName),
            ("lpt9", DeviceName),
            ("COM¹", DeviceName),
            ("conin$", DeviceName),
            ("a\0b", Character),
            ("a\nb", Character),
            ("a?b", Character),
            ("a*b", Character),
            ("a<b", Character),
        ] {
            assert_eq!(InstallPath::new(name), Err(problem), "{name:?}");
        }
    }

    #[test]
    fn under_joins_component_by_component() {
        let root = Path::new("install");
        let path = InstallPath::new("Archon/Binaries/Win64/x.exe").unwrap();
        assert_eq!(
            path.under(root),
            root.join("Archon").join("Binaries").join("Win64").join("x.exe")
        );
        assert!(path.under(root).starts_with(root));
    }
}
