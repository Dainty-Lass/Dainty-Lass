// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Finding the game: a folder the player pasted or picked, and the installs Epic, Legendary and Heroic know of
//! (spec §2.7.7).

use std::io::Read as _;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::bridge::types::{DetectedGame, GameSource};
use crate::core::pins::{EXE_NAME, EXE_RELATIVE_PATH, WIN64_RELATIVE_DIR};
use crate::core::text::{js_length, js_trim, truncate_utf16};

/// The longest path accepted as input.
pub const MAX_INPUT_UNITS: usize = 260;
/// The longest game root used; the game's own deeper paths must still fit Windows' limit.
pub const MAX_ROOT_UNITS: usize = 180;
/// The exact version text of the supported build.
pub const SUPPORTED_VERSION: &str = "dauntless_rel-1.14.7_Shipping_2024-04-01_14-59-21_647472";
/// The store's id for Dauntless.
pub const APP_NAME: &str = "Jackal";

const VERSION_FILE: &str = "Version.txt";
const VERSION_READ_BYTES: u64 = 256;
const MAX_VERSION_UNITS: usize = 128;
const MAX_STORE_VERSION_UNITS: usize = 64;
const MAX_MANIFESTS: usize = 500;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_INSTALLED_BYTES: u64 = 4 * 1024 * 1024;

/// Why a path is not a game folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocateError {
    /// Not a path this platform accepts.
    Invalid,
    /// The game is there, but its folder is too long a path.
    TooLong,
    /// No game executable in or around it.
    NotFound,
}

impl std::fmt::Display for LocateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Invalid => "the folder path is not valid",
            Self::TooLong => "the game folder's path is too long",
            Self::NotFound => "the game was not found there",
        })
    }
}

impl std::error::Error for LocateError {}

/// The game executable under `root`.
#[must_use]
pub fn exe_path(root: &Path) -> PathBuf {
    EXE_RELATIVE_PATH
        .split('/')
        .fold(root.to_path_buf(), |p, part| p.join(part))
}

/// The Win64 folder under `root`, where the game runs.
#[must_use]
pub fn win64_dir(root: &Path) -> PathBuf {
    WIN64_RELATIVE_DIR
        .split('/')
        .fold(root.to_path_buf(), |p, part| p.join(part))
}

/// A normalised folder: its root (`C:` or empty for `/`) and its parts.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Folder {
    root: String,
    parts: Vec<String>,
    windows: bool,
}

impl Folder {
    fn render(&self) -> String {
        let sep = if self.windows { '\\' } else { '/' };
        let mut out = self.root.clone();
        out.push(sep);
        out.push_str(&self.parts.join(&sep.to_string()));
        out
    }

    fn parent(&self) -> Self {
        let mut up = self.clone();
        up.parts.pop();
        up
    }

    fn child(&self, name: &str) -> Self {
        let mut down = self.clone();
        down.parts.push(name.to_owned());
        down
    }

    fn last_is(&self, name: &str) -> bool {
        self.parts.last().is_some_and(|p| p.eq_ignore_ascii_case(name))
    }
}

fn parse_input(raw: &str, windows: bool) -> Option<Folder> {
    let mut text = js_trim(raw);
    if let Some(inner) = text.strip_prefix('"').and_then(|t| t.strip_suffix('"')) {
        text = js_trim(inner);
    }
    if text.is_empty() || js_length(text) > MAX_INPUT_UNITS || text.contains('\0') {
        return None;
    }
    let (root, rest) = if windows {
        // Only `X:\` or `X:/`: UNC, `\\?\` device and root-relative paths never reach the file system.
        let bytes = text.as_bytes();
        let drive = bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
        if !drive || !matches!(bytes[2], b'\\' | b'/') {
            return None;
        }
        (text[..2].to_owned(), &text[3..])
    } else {
        (String::new(), text.strip_prefix('/')?)
    };
    let mut parts: Vec<String> = Vec::new();
    let separators: &[char] = if windows { &['\\', '/'] } else { &['/'] };
    for part in rest.split(separators) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part.to_owned()),
        }
    }
    Some(Folder { root, parts, windows })
}

/// `raw` as a normalised absolute folder for this kind of platform, or none when it is not one.
#[must_use]
pub fn normalize(raw: &str, windows: bool) -> Option<String> {
    parse_input(raw, windows).map(|f| f.render())
}

/// The folders tried for `raw`, in order, without repeats.
#[must_use]
pub fn candidates(raw: &str, windows: bool) -> Option<Vec<String>> {
    let input = parse_input(raw, windows)?;
    let folder = if input.last_is(EXE_NAME) { input.parent() } else { input };
    let mut list = vec![folder.clone(), folder.child("Dauntless")];
    if folder.last_is("archon") {
        list.push(folder.parent());
    }
    if folder.last_is("win64") {
        list.push(folder.parent().parent().parent());
    }
    let mut out: Vec<String> = Vec::new();
    for candidate in list.iter().map(Folder::render) {
        if !out.contains(&candidate) {
            out.push(candidate);
        }
    }
    Some(out)
}

/// The game root `raw` points at, checking for the executable with `is_file`.
pub fn locate_with(raw: &str, windows: bool, is_file: impl Fn(&Path) -> bool) -> Result<PathBuf, LocateError> {
    let candidates = candidates(raw, windows).ok_or(LocateError::Invalid)?;
    let mut found = false;
    for candidate in candidates {
        let root = PathBuf::from(&candidate);
        if is_file(&exe_path(&root)) {
            if js_length(&candidate) <= MAX_ROOT_UNITS {
                return Ok(root);
            }
            found = true;
        }
    }
    Err(if found {
        LocateError::TooLong
    } else {
        LocateError::NotFound
    })
}

/// The game root `raw` points at on this PC.
pub fn locate(raw: &str) -> Result<PathBuf, LocateError> {
    locate_with(raw, cfg!(windows), Path::is_file)
}

/// The version line of a `Version.txt`'s first bytes, when it is short printable ASCII.
#[must_use]
pub fn parse_version_text(head: &[u8]) -> Option<String> {
    let head = &head[..head.len().min(VERSION_READ_BYTES as usize)];
    let text = String::from_utf8_lossy(head);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let line = js_trim(text).split('\n').next().unwrap_or_default().replace('\r', "");
    let line = js_trim(&line);
    let ok = (1..=MAX_VERSION_UNITS).contains(&line.len()) && line.bytes().all(|b| (0x20..=0x7e).contains(&b));
    ok.then(|| line.to_owned())
}

/// The version `root`'s `Version.txt` reports.
#[must_use]
pub fn read_version_text(root: &Path) -> Option<String> {
    let file = std::fs::File::open(root.join(VERSION_FILE)).ok()?;
    let mut head = Vec::new();
    file.take(VERSION_READ_BYTES).read_to_end(&mut head).ok()?;
    parse_version_text(&head)
}

/// The version text names the build this launcher plays.
#[must_use]
pub fn is_supported_version(text: &str) -> bool {
    if text == SUPPORTED_VERSION {
        return true;
    }
    if !text.to_ascii_lowercase().contains("rel-1.14.7") {
        return false;
    }
    let bytes = text.as_bytes();
    text.match_indices("647472").any(|(at, m)| {
        let before = at.checked_sub(1).and_then(|i| bytes.get(i));
        let after = bytes.get(at + m.len());
        !before.is_some_and(u8::is_ascii_digit) && !after.is_some_and(u8::is_ascii_digit)
    })
}

/// An install a store lists: its path and the version the store records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreInstall {
    /// The folder the store installed into.
    pub path: String,
    /// The store's version string, when it has one.
    pub version: Option<String>,
}

fn parse_json(bytes: &[u8]) -> Option<Value> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    serde_json::from_slice(bytes).ok()
}

fn store_version(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(|v| truncate_utf16(v, MAX_STORE_VERSION_UNITS).to_owned())
}

fn install_path(value: Option<&Value>) -> Option<String> {
    let path = value.and_then(Value::as_str)?;
    (!path.is_empty() && js_length(path) <= MAX_INPUT_UNITS).then(|| path.to_owned())
}

/// Dauntless's entries in Legendary's (or Heroic's) `installed.json`.
#[must_use]
pub fn parse_legendary_installed(bytes: &[u8]) -> Vec<StoreInstall> {
    let Some(Value::Object(entries)) = parse_json(bytes) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|(key, entry)| {
            let name = entry.get("app_name").and_then(Value::as_str).unwrap_or(key);
            if name != APP_NAME {
                return None;
            }
            Some(StoreInstall {
                path: install_path(entry.get("install_path"))?,
                version: store_version(entry.get("version")),
            })
        })
        .collect()
}

/// Dauntless's install from one of the Epic Games launcher's `.item` manifests.
#[must_use]
pub fn parse_epic_item(bytes: &[u8]) -> Option<StoreInstall> {
    let item = parse_json(bytes)?;
    let is = |field: &str, want: &str| item.get(field).and_then(Value::as_str) == Some(want);
    if !(is("AppName", APP_NAME) || is("MainGameAppName", APP_NAME) || is("DisplayName", "Dauntless")) {
        return None;
    }
    Some(StoreInstall {
        path: install_path(item.get("InstallLocation"))?,
        version: store_version(item.get("AppVersionString")),
    })
}

/// Where each store keeps its list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreFolders {
    /// Legendary's config folders, each holding `installed.json`.
    pub legendary: Vec<PathBuf>,
    /// Heroic's `installed.json`.
    pub heroic: Option<PathBuf>,
    /// The Epic Games launcher's manifest folder.
    pub epic_manifests: Option<PathBuf>,
}

impl StoreFolders {
    /// This PC's, from a variable lookup; the stores are only looked for on Windows.
    #[must_use]
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> Self {
        let set = |name: &str| var(name).filter(|v| !v.is_empty()).map(PathBuf::from);
        let mut legendary: Vec<PathBuf> = set("LEGENDARY_CONFIG_PATH").into_iter().collect();
        if let Some(home) = set("USERPROFILE") {
            legendary.push(home.join(".config").join("legendary"));
        }
        Self {
            legendary,
            heroic: set("APPDATA").map(|d| d.join("heroic/legendaryConfig/legendary/installed.json")),
            epic_manifests: set("ProgramData").map(|d| d.join("Epic/EpicGamesLauncher/Data/Manifests")),
        }
    }
}

fn read_capped(path: &Path, cap: u64) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > cap {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(cap + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= cap).then_some(bytes)
}

fn epic_items(dir: &Path) -> Vec<StoreInstall> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("item")))
        .collect();
    files.sort();
    files.truncate(MAX_MANIFESTS);
    files
        .iter()
        .filter_map(|f| read_capped(f, MAX_MANIFEST_BYTES))
        .filter_map(|bytes| parse_epic_item(&bytes))
        .collect()
}

/// The game installs the stores in `folders` list, located (§2.7.7) and each kept once.
#[must_use]
pub fn detect_in(folders: &StoreFolders) -> Vec<DetectedGame> {
    let installed = |path: &Path| {
        read_capped(path, MAX_INSTALLED_BYTES)
            .map(|b| parse_legendary_installed(&b))
            .unwrap_or_default()
    };
    let mut listed: Vec<(GameSource, StoreInstall)> = Vec::new();
    for dir in &folders.legendary {
        listed.extend(
            installed(&dir.join("installed.json"))
                .into_iter()
                .map(|i| (GameSource::Legendary, i)),
        );
    }
    if let Some(file) = &folders.heroic {
        listed.extend(installed(file).into_iter().map(|i| (GameSource::Heroic, i)));
    }
    if let Some(dir) = &folders.epic_manifests {
        listed.extend(epic_items(dir).into_iter().map(|i| (GameSource::Epic, i)));
    }
    let mut found: Vec<DetectedGame> = Vec::new();
    for (source, install) in listed {
        let Ok(root) = locate(&install.path) else {
            continue;
        };
        let root_text = root.to_string_lossy().into_owned();
        if found.iter().any(|g| g.root.to_lowercase() == root_text.to_lowercase()) {
            continue;
        }
        let version = install.version.or_else(|| read_version_text(&root));
        found.push(DetectedGame {
            root: root_text,
            source,
            version,
        });
    }
    found
}

/// The game installs the stores on this PC list; none off Windows.
#[must_use]
pub fn detect() -> Vec<DetectedGame> {
    if !cfg!(windows) {
        return Vec::new();
    }
    detect_in(&StoreFolders::from_vars(|name| std::env::var(name).ok()))
}

/// Beside a Dauntless Revived 1.4.4 executable: its in-process server, which no 1.14.7 install has.
pub const REVIVED_MARKER: &str = "UndauntedInternalServer.dll";

/// Whether the game at `root` is a Dauntless Revived copy: its marker, or a 1.4 version text.
pub fn is_revived_with(root: &Path, version: Option<&str>, is_file: impl Fn(&Path) -> bool) -> bool {
    is_file(&win64_dir(root).join(REVIVED_MARKER))
        || version.is_some_and(|v| v.to_ascii_lowercase().contains("rel-1.4."))
}

/// [`is_revived_with`] on this PC.
#[must_use]
pub fn is_revived(root: &Path, version: Option<&str>) -> bool {
    is_revived_with(root, version, Path::is_file)
}

/// Whether a folder `raw` points at holds game files (`Version.txt`, `Engine`) but not `Archon` with the executable:
/// a copy missing its main folder, or the wrong level of the install picked.
pub fn missing_archon_with(raw: &str, windows: bool, exists: impl Fn(&Path) -> bool) -> bool {
    let Some(candidates) = candidates(raw, windows) else {
        return false;
    };
    candidates
        .iter()
        .map(PathBuf::from)
        .any(|root| (exists(&root.join(VERSION_FILE)) || exists(&root.join("Engine"))) && !exists(&exe_path(&root)))
}

/// [`missing_archon_with`] on this PC.
#[must_use]
pub fn missing_archon(raw: &str) -> bool {
    missing_archon_with(raw, cfg!(windows), Path::exists)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revived_copies_by_marker_or_version() {
        let root = Path::new("/games/Dauntless");
        let marker = win64_dir(root).join(REVIVED_MARKER);
        assert!(is_revived_with(root, Some(SUPPORTED_VERSION), |p| p == marker));
        assert!(is_revived_with(root, Some("dauntless_REL-1.4.4_Shipping"), |_| false));
        assert!(!is_revived_with(root, Some(SUPPORTED_VERSION), |_| false));
        assert!(!is_revived_with(root, None, |_| false));
    }

    #[test]
    fn game_files_without_archon() {
        let at = |files: &'static [&'static str]| move |p: &Path| files.iter().any(|f| p == Path::new(f));
        assert!(missing_archon_with(
            "/g/Dauntless",
            false,
            at(&["/g/Dauntless/Version.txt"])
        ));
        assert!(missing_archon_with("/g/Dauntless", false, at(&["/g/Dauntless/Engine"])));
        // The parent of the install picked: its Dauntless child is tried too
        assert!(missing_archon_with("/g", false, at(&["/g/Dauntless/Engine"])));
        let whole = at(&[
            "/g/Dauntless/Version.txt",
            "/g/Dauntless/Archon/Binaries/Win64/Dauntless-Win64-Shipping.exe",
        ]);
        assert!(!missing_archon_with("/g/Dauntless", false, whole));
        assert!(!missing_archon_with("/g/Pictures", false, at(&[])));
        assert!(!missing_archon_with("not a path", false, at(&["Version.txt"])));
    }

    #[test]
    fn normalising_windows_paths() {
        assert_eq!(
            normalize(r"  C:\Games\Dauntless\ ", true).unwrap(),
            r"C:\Games\Dauntless"
        );
        assert_eq!(normalize(r#" " d:/a//b/./c/../e " "#, true).unwrap(), r"d:\a\b\e");
        assert_eq!(normalize(r"C:\", true).unwrap(), r"C:\");
        assert_eq!(normalize(r"C:\..\..", true).unwrap(), r"C:\");
        for bad in [
            "",
            "   ",
            "\"\"",
            r"\\server\share",
            r"\\?\C:\x",
            r"\Games",
            "C:",
            "C:Games",
            "1:\\x",
            "/usr/games",
            "C:\\a\0b",
        ] {
            assert_eq!(normalize(bad, true), None, "{bad:?}");
        }
        let long = format!("C:\\{}", "a".repeat(257));
        assert!(normalize(&long, true).is_some());
        assert_eq!(normalize(&format!("{long}a"), true), None);
    }

    #[test]
    fn normalising_linux_paths() {
        assert_eq!(normalize("/home/p/Games/", false).unwrap(), "/home/p/Games");
        assert_eq!(normalize("/a/../../b", false).unwrap(), "/b");
        assert_eq!(normalize("/", false).unwrap(), "/");
        assert_eq!(normalize(r"/a\b", false).unwrap(), r"/a\b");
        assert_eq!(normalize("relative/x", false), None);
        assert_eq!(normalize(r"C:\x", false), None);
    }

    #[test]
    fn candidate_order() {
        assert_eq!(
            candidates(r"C:\G\Archon\Binaries\Win64\dauntless-win64-shipping.EXE", true).unwrap(),
            [
                r"C:\G\Archon\Binaries\Win64",
                r"C:\G\Archon\Binaries\Win64\Dauntless",
                r"C:\G"
            ]
        );
        assert_eq!(
            candidates(r"C:\G\ARCHON", true).unwrap(),
            [r"C:\G\ARCHON", r"C:\G\ARCHON\Dauntless", r"C:\G"]
        );
        assert_eq!(candidates("/epic", false).unwrap(), ["/epic", "/epic/Dauntless"]);
        assert_eq!(
            candidates(r"C:\Win64", true).unwrap(),
            [r"C:\Win64", r"C:\Win64\Dauntless", r"C:\"]
        );
    }

    #[test]
    fn locating() {
        let exe_under = |root: &'static str| move |p: &Path| p == exe_path(Path::new(root));
        assert_eq!(
            locate_with(r"C:\G", true, exe_under(r"C:\G\Dauntless")).unwrap(),
            PathBuf::from(r"C:\G\Dauntless")
        );
        assert_eq!(
            locate_with(r"C:\G\Archon\Binaries\Win64", true, exe_under(r"C:\G")).unwrap(),
            PathBuf::from(r"C:\G")
        );
        assert_eq!(locate_with(r"C:\G", true, |_| false), Err(LocateError::NotFound));
        assert_eq!(locate_with(r"\\srv\G", true, |_| true), Err(LocateError::Invalid));

        let fits = format!(r"C:\{}", "d".repeat(177));
        assert_eq!(js_length(&fits), 180);
        assert_eq!(locate_with(&fits, true, |_| true).unwrap(), PathBuf::from(&fits));
        let long = format!(r"C:\{}", "d".repeat(178));
        assert_eq!(locate_with(&long, true, |_| true), Err(LocateError::TooLong));
        // A short candidate further down the list still wins over long ones before it.
        let top = format!(r"C:\{}", "w".repeat(170));
        let win64 = format!(r"{top}\Archon\Binaries\Win64");
        assert_eq!(locate_with(&win64, true, |_| true).unwrap(), PathBuf::from(&top));
    }

    #[test]
    fn version_text() {
        assert_eq!(
            parse_version_text(format!("\u{feff}  {SUPPORTED_VERSION}\r\nmore").as_bytes()).unwrap(),
            SUPPORTED_VERSION
        );
        assert_eq!(parse_version_text(b"\n\n  v1 \r\n").unwrap(), "v1");
        assert_eq!(parse_version_text(b"a\rb\nc").unwrap(), "ab");
        assert_eq!(parse_version_text(b""), None);
        assert_eq!(parse_version_text(b"tab\there"), None);
        assert_eq!(parse_version_text("caf\u{e9}".as_bytes()), None);
        assert_eq!(parse_version_text(&[b'x'; 129]), None);
        assert_eq!(parse_version_text(&[b'x'; 128]).unwrap().len(), 128);
        let mut long = vec![b'y'; 300];
        long[100] = b'\n';
        assert_eq!(parse_version_text(&long).unwrap().len(), 100);
    }

    #[test]
    fn supported_versions() {
        assert!(is_supported_version(SUPPORTED_VERSION));
        assert!(is_supported_version("Dauntless REL-1.14.7 cl 647472"));
        assert!(is_supported_version("rel-1.14.7_647472"));
        assert!(!is_supported_version("rel-1.14.7_6474721"));
        assert!(!is_supported_version("rel-1.14.7_1647472"));
        assert!(!is_supported_version("rel-1.14.6_647472"));
        assert!(!is_supported_version("rel-1.14.7"));
        assert!(is_supported_version("rel-1.14.7 1647472 647472"));
    }

    #[test]
    fn store_lists() {
        let legendary = br#"{"Jackal":{"install_path":"C:\\Epic\\Dauntless","version":"1.14.7"},
            "Other":{"app_name":"Jackal","install_path":"D:\\X"},
            "Jackal2":{"app_name":"Nope","install_path":"E:\\Y"},
            "Jackal3":{"app_name":"Jackal","install_path":""}}"#;
        assert_eq!(
            parse_legendary_installed(legendary),
            [
                StoreInstall {
                    path: r"C:\Epic\Dauntless".to_owned(),
                    version: Some("1.14.7".to_owned())
                },
                StoreInstall {
                    path: r"D:\X".to_owned(),
                    version: None
                },
            ]
        );
        assert!(parse_legendary_installed(b"[]").is_empty());
        assert!(parse_legendary_installed(b"nope").is_empty());

        let item = "\u{feff}{\"DisplayName\":\"Dauntless\",\"InstallLocation\":\"C:\\\\D\",\"AppVersionString\":\"v\"}";
        assert_eq!(
            parse_epic_item(item.as_bytes()),
            Some(StoreInstall {
                path: r"C:\D".to_owned(),
                version: Some("v".to_owned())
            })
        );
        let long_version = format!(
            r#"{{"MainGameAppName":"Jackal","InstallLocation":"x","AppVersionString":"{}"}}"#,
            "9".repeat(80)
        );
        assert_eq!(
            parse_epic_item(long_version.as_bytes()).unwrap().version.unwrap().len(),
            64
        );
        assert_eq!(
            parse_epic_item(br#"{"AppName":"Fortnite","InstallLocation":"x"}"#),
            None
        );
        assert_eq!(parse_epic_item(br#"{"AppName":"Jackal"}"#), None);
    }

    fn fake_game(root: &Path) {
        let exe = exe_path(root);
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(exe, b"MZ").unwrap();
    }

    fn json_path(path: &Path) -> String {
        serde_json::to_string(&path.to_string_lossy()).unwrap()
    }

    #[test]
    fn detecting_installs() {
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("Epic").join("Dauntless");
        fake_game(&game);
        std::fs::write(game.join("Version.txt"), SUPPORTED_VERSION).unwrap();
        let other = tmp.path().join("Other");
        fake_game(&other);

        let legendary = tmp.path().join("legendary");
        std::fs::create_dir_all(&legendary).unwrap();
        std::fs::write(
            legendary.join("installed.json"),
            format!(
                r#"{{"Jackal":{{"install_path":{},"version":"L1"}}}}"#,
                json_path(&game.join("Archon"))
            ),
        )
        .unwrap();
        let heroic = tmp.path().join("heroic.json");
        let upper = game.to_string_lossy().to_uppercase();
        std::fs::write(
            &heroic,
            format!(r#"{{"Jackal":{{"install_path":{}}}}}"#, json_path(Path::new(&upper))),
        )
        .unwrap();
        let manifests = tmp.path().join("Manifests");
        std::fs::create_dir_all(&manifests).unwrap();
        std::fs::write(
            manifests.join("a.item"),
            format!(
                r#"{{"AppName":"Jackal","InstallLocation":{}}}"#,
                json_path(&tmp.path().join("Other"))
            ),
        )
        .unwrap();
        std::fs::write(
            manifests.join("b.txt"),
            format!(r#"{{"AppName":"Jackal","InstallLocation":{}}}"#, json_path(&game)),
        )
        .unwrap();

        let found = detect_in(&StoreFolders {
            legendary: vec![tmp.path().join("missing"), legendary],
            heroic: Some(heroic),
            epic_manifests: Some(manifests),
        });
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].source, GameSource::Legendary);
        assert_eq!(found[0].version.as_deref(), Some("L1"));
        assert_eq!(found[1].source, GameSource::Epic);
        assert_eq!(found[1].version, None);
        assert!(detect_in(&StoreFolders::default()).is_empty());
    }

    #[test]
    fn store_folders_from_the_environment() {
        let folders = StoreFolders::from_vars(|name| match name {
            "LEGENDARY_CONFIG_PATH" => Some("L".to_owned()),
            "USERPROFILE" => Some("U".to_owned()),
            "APPDATA" => Some(String::new()),
            "ProgramData" => Some("P".to_owned()),
            _ => None,
        });
        assert_eq!(
            folders.legendary,
            [PathBuf::from("L"), PathBuf::from("U").join(".config").join("legendary")]
        );
        assert_eq!(folders.heroic, None);
        assert_eq!(
            folders.epic_manifests,
            Some(PathBuf::from("P").join("Epic/EpicGamesLauncher/Data/Manifests"))
        );
    }
}
