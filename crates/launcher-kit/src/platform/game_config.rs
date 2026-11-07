// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The game's own config files: Engine.ini points its XMPP chat at the relay, GameUserSettings.ini is repaired
//! when its window would be off-screen, and Game.ini loses the stale Phoenix endpoints (spec §2.7.6).

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::bridge::types::GraphicsPreset;
use crate::core::text::js_number;

/// The config folder below `%LOCALAPPDATA%` (and below a Wine prefix's AppData\Local).
pub const CONFIG_SUBDIR: &str = "Archon/Saved/Config/WindowsClient";
/// The graphics groups a forced preset sets, as `sg.<group>Quality`.
pub const QUALITY_GROUPS: [&str; 8] = [
    "ViewDistance",
    "AntiAliasing",
    "Shadow",
    "PostProcess",
    "Texture",
    "Effects",
    "Foliage",
    "Shading",
];
/// What a repaired window becomes: 1280×720, windowed, centred.
pub const SAFE_WINDOW_LINES: [&str; 9] = [
    "ResolutionSizeX=1280",
    "ResolutionSizeY=720",
    "LastUserConfirmedResolutionSizeX=1280",
    "LastUserConfirmedResolutionSizeY=720",
    "FullscreenMode=2",
    "LastConfirmedFullscreenMode=2",
    "PreferredFullscreenMode=2",
    "WindowPosX=-1",
    "WindowPosY=-1",
];
/// The sections added when GameUserSettings.ini has none.
pub const USER_SETTINGS_SECTIONS: [&str; 2] = [
    "[/Script/Engine.GameUserSettings]",
    "[/Script/Archon.ArchonGameUserSettings]",
];

const SIZE_KEYS: [&str; 8] = [
    "ResolutionSizeX",
    "ResolutionSizeY",
    "LastUserConfirmedResolutionSizeX",
    "LastUserConfirmedResolutionSizeY",
    "DesiredScreenWidth",
    "DesiredScreenHeight",
    "LastUserConfirmedDesiredScreenWidth",
    "LastUserConfirmedDesiredScreenHeight",
];
const WINDOW_KEYS: [&str; 5] = [
    "FullscreenMode",
    "LastConfirmedFullscreenMode",
    "PreferredFullscreenMode",
    "WindowPosX",
    "WindowPosY",
];
const MAX_DIMENSION: f64 = 16384.0;
const ENGINE_DROPPED: [&str; 2] = ["[systemsettings]", "[onlinesubsystemmcp.xmpp]"];
const PHOENIX_HEADER: &str = "[onlinesubsystemphoenix]";
const TMP_SUFFIX: &str = ".rr-tmp";

/// The config folder on Windows, from a variable lookup.
#[must_use]
pub fn windows_config_dir(var: impl Fn(&str) -> Option<OsString>) -> PathBuf {
    let set = |name: &str| var(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    let local = set("LOCALAPPDATA")
        .or_else(|| set("USERPROFILE").map(|home| home.join("AppData").join("Local")))
        .unwrap_or_else(|| PathBuf::from(r"C:\AppData\Local"));
    CONFIG_SUBDIR.split('/').fold(local, |dir, part| dir.join(part))
}

/// The config folder on Windows for this process.
#[must_use]
pub fn default_config_dir() -> PathBuf {
    windows_config_dir(|name| std::env::var_os(name))
}

/// Text from a config file: UTF-16 LE with its BOM, else UTF-8 without one.
#[must_use]
pub fn decode(bytes: &[u8]) -> String {
    if let Some(body) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = body.as_chunks::<2>().0.iter().map(|p| u16::from_le_bytes(*p)).collect();
        return String::from_utf16_lossy(&units);
    }
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8_lossy(body).into_owned()
}

/// The file's lines, on CR LF, LF or CR, without the empty one a final line break leaves.
#[must_use]
pub fn split_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(['\r', '\n']) {
        lines.push(rest[..at].to_owned());
        let skip = if rest[at..].starts_with("\r\n") { 2 } else { 1 };
        rest = &rest[at + skip..];
    }
    // The tail after the last break; empty means the file ended with one.
    if !rest.is_empty() {
        lines.push(rest.to_owned());
    }
    lines
}

/// `lines` each followed by CR LF, as the game writes them.
#[must_use]
pub fn join_lines<S: AsRef<str>>(lines: &[S]) -> String {
    let mut text = String::new();
    for line in lines {
        text.push_str(line.as_ref());
        text.push_str("\r\n");
    }
    text
}

/// The bytes written: ASCII as it is, anything else as UTF-16 LE with a BOM, which the game reads.
#[must_use]
pub fn encode(text: &str) -> Vec<u8> {
    if text.is_ascii() {
        return text.as_bytes().to_vec();
    }
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
    bytes
}

fn read_lines(path: &Path) -> io::Result<Option<Vec<String>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(split_lines(&decode(&bytes)))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Writes `text` through `<file>.rr-tmp` and a rename, so the game never reads half a file.
pub fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(TMP_SUFFIX);
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, encode(text))?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

fn is_header(line: &str) -> bool {
    line.trim_start().starts_with('[')
}

fn header_starts(line: &str, lower_header: &str) -> bool {
    line.trim_start()
        .get(..lower_header.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(lower_header))
}

fn is_key_line(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty() && !trimmed.starts_with(';') && !trimmed.starts_with('#') && trimmed.contains('=')
}

/// Game.ini without its `[OnlineSubsystemPhoenix]` sections, and how many keys they held; none when there were none.
#[must_use]
pub fn remove_phoenix<S: AsRef<str>>(lines: &[S]) -> Option<(Vec<String>, usize)> {
    let mut kept = Vec::with_capacity(lines.len());
    let (mut inside, mut found, mut keys) = (false, false, 0);
    for line in lines {
        let line = line.as_ref();
        if is_header(line) {
            inside = line.trim().eq_ignore_ascii_case(PHOENIX_HEADER);
            found |= inside;
        }
        if inside {
            keys += usize::from(is_key_line(line));
        } else {
            kept.push(line.to_owned());
        }
    }
    found.then_some((kept, keys))
}

/// Why a config file was not written.
#[derive(Debug)]
pub enum ConfigError {
    /// The XMPP host is not a plain host name or IPv4 address.
    InvalidHost,
    /// Port 0.
    InvalidPort,
    /// Reading or writing failed.
    Io(io::Error),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidHost => f.write_str("the XMPP host is not valid"),
            Self::InvalidPort => f.write_str("the XMPP port is not valid"),
            Self::Io(e) => write!(f, "the game's config could not be written: {e}"),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for ConfigError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// A host the game's `ws://` URL can carry as it is: letters, digits, `.` and `-`, not starting or ending in
/// either.
#[must_use]
pub fn is_valid_host(host: &str) -> bool {
    (1..=253).contains(&host.len())
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && !host.starts_with(['.', '-'])
        && !host.ends_with(['.', '-'])
}

fn forced_level(graphics: GraphicsPreset) -> Option<i8> {
    let level = graphics.get();
    (level >= 0).then_some(level)
}

/// Engine.ini with the launcher's sections first and the rest kept as it was.
pub fn edit_engine_ini<S: AsRef<str>>(
    lines: &[S],
    host: &str,
    port: u16,
    graphics: GraphicsPreset,
) -> Result<Vec<String>, ConfigError> {
    if !is_valid_host(host) {
        return Err(ConfigError::InvalidHost);
    }
    if port == 0 {
        return Err(ConfigError::InvalidPort);
    }
    let mut kept: Vec<String> = Vec::with_capacity(lines.len());
    let mut dropping = false;
    for line in lines {
        let line = line.as_ref();
        if is_header(line) {
            dropping = ENGINE_DROPPED.iter().any(|h| header_starts(line, h));
        }
        if !dropping && !(kept.is_empty() && line.trim().is_empty()) {
            kept.push(line.to_owned());
        }
    }
    let mut out = Vec::with_capacity(kept.len() + 16);
    if let Some(level) = forced_level(graphics) {
        out.push("[SystemSettings]".to_owned());
        out.extend(QUALITY_GROUPS.iter().map(|g| format!("sg.{g}Quality={level}")));
        out.push("sg.ResolutionQuality=100".to_owned());
        out.push("r.ScreenPercentage=100".to_owned());
        out.push(String::new());
    }
    out.push("[OnlineSubsystemMcp.XMPP]".to_owned());
    out.push(format!("ServerAddr=\"ws://{host}\""));
    out.push(format!("ServerPort={port}"));
    out.push("bUseSSL=false".to_owned());
    out.push(String::new());
    out.extend(kept);
    Ok(out)
}

/// A section whose window settings the game reads: its name's last `.`-part is letters ending in
/// `GameUserSettings`.
#[must_use]
pub fn is_user_settings_section(header: &str) -> bool {
    let trimmed = header.trim();
    let Some(name) = trimmed.strip_prefix('[').and_then(|h| h.strip_suffix(']')) else {
        return false;
    };
    let last = name.rsplit_once('.').map_or(name, |(_, last)| last);
    const SUFFIX: &str = "gameusersettings";
    last.len() >= SUFFIX.len()
        && last.bytes().all(|b| b.is_ascii_alphabetic())
        && last[last.len() - SUFFIX.len()..].eq_ignore_ascii_case(SUFFIX)
}

fn key_of(line: &str) -> Option<&str> {
    line.split_once('=').map(|(key, _)| key.trim())
}

fn is_one_of(key: &str, names: &[&str]) -> bool {
    names.iter().any(|n| n.eq_ignore_ascii_case(key))
}

fn bad_size(line: &str) -> bool {
    let Some((key, value)) = line.split_once('=') else {
        return false;
    };
    if !is_one_of(key.trim(), &SIZE_KEYS) {
        return false;
    }
    let value = value.split_once(';').map_or(value, |(v, _)| v);
    let n = js_number(value);
    !n.is_finite() || !(1.0..=MAX_DIMENSION).contains(&n)
}

/// What [`repair_user_settings`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserSettingsEdit {
    /// The file's new lines.
    pub lines: Vec<String>,
    /// The window settings were rewritten.
    pub repaired: bool,
}

/// GameUserSettings.ini with an impossible window size (or, when `safe_window`, any size) replaced by a
/// 1280×720 window, and a forced preset's quality lines set.
#[must_use]
pub fn repair_user_settings<S: AsRef<str>>(
    lines: &[S],
    safe_window: bool,
    graphics: GraphicsPreset,
) -> UserSettingsEdit {
    let lines: Vec<String> = lines.iter().map(|l| l.as_ref().to_owned()).collect();
    let mut in_section = false;
    let mut has_section = false;
    let mut bad = false;
    for line in &lines {
        if is_header(line) {
            in_section = is_user_settings_section(line);
            has_section |= in_section;
        } else if in_section && bad_size(line) {
            bad = true;
        }
    }
    let mut out = lines.clone();
    if bad || safe_window {
        if !has_section {
            out.extend(USER_SETTINGS_SECTIONS.iter().map(|h| (*h).to_owned()));
        }
        out = rewrite_window(&out);
    }
    let repaired = out != lines;
    if let Some(level) = forced_level(graphics) {
        for line in &mut out {
            if let Some(group) = quality_group(line) {
                *line = format!("sg.{group}Quality={level}");
            }
        }
    }
    UserSettingsEdit { lines: out, repaired }
}

fn rewrite_window(lines: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(lines.len() + 18);
    let mut in_section = false;
    for line in lines {
        if is_header(line) {
            in_section = is_user_settings_section(line);
            out.push(line.clone());
            if in_section {
                out.extend(SAFE_WINDOW_LINES.iter().map(|l| (*l).to_owned()));
            }
            continue;
        }
        let dropped =
            in_section && key_of(line).is_some_and(|k| is_one_of(k, &SIZE_KEYS) || is_one_of(k, &WINDOW_KEYS));
        if !dropped {
            out.push(line.clone());
        }
    }
    out
}

// The forced group a `sg.<group>Quality=` line sets, matched in any case; the rewrite spells it canonically.
fn quality_group(line: &str) -> Option<&'static str> {
    let (key, _) = line.split_once('=')?;
    QUALITY_GROUPS
        .iter()
        .copied()
        .find(|g| key.eq_ignore_ascii_case(&format!("sg.{g}Quality")))
}

/// What writing the config did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigOutcome {
    /// Engine.ini, as written.
    pub engine_ini: PathBuf,
    /// GameUserSettings.ini's window was repaired.
    pub display_repaired: bool,
    /// Phoenix keys removed from Game.ini.
    pub phoenix_keys_removed: usize,
}

/// What to write.
#[derive(Debug, Clone, Copy)]
pub struct ConfigRequest<'a> {
    /// The XMPP host the game connects to (the relay's).
    pub host: &'a str,
    /// Its port.
    pub port: u16,
    /// The preset to force, or unset.
    pub graphics: GraphicsPreset,
    /// Put the game in a 1280×720 window whatever its settings say.
    pub safe_window: bool,
    /// Remove Game.ini's Phoenix sections.
    pub clean_phoenix: bool,
}

/// Writes the game's config files in `dir`, creating it.
pub fn write_game_config(dir: &Path, request: &ConfigRequest<'_>) -> Result<ConfigOutcome, ConfigError> {
    std::fs::create_dir_all(dir)?;
    let mut phoenix_keys_removed = 0;
    if request.clean_phoenix {
        let game_ini = dir.join("Game.ini");
        if let Some(lines) = read_lines(&game_ini)?
            && let Some((kept, keys)) = remove_phoenix(&lines)
        {
            std::fs::copy(&game_ini, dir.join("Game.ini.before-reforged"))?;
            write_atomic(&game_ini, &join_lines(&kept))?;
            phoenix_keys_removed = keys;
        }
    }

    let engine_ini = dir.join("Engine.ini");
    let engine = read_lines(&engine_ini)?.unwrap_or_default();
    let engine = edit_engine_ini(&engine, request.host, request.port, request.graphics)?;
    write_atomic(&engine_ini, &join_lines(&engine))?;

    let user_ini = dir.join("GameUserSettings.ini");
    let existing = read_lines(&user_ini)?;
    let mut display_repaired = false;
    if existing.is_some() || request.safe_window {
        let lines = existing.clone().unwrap_or_default();
        let edit = repair_user_settings(&lines, request.safe_window, request.graphics);
        if edit.repaired && existing.is_some() {
            let ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_millis());
            std::fs::copy(
                &user_ini,
                dir.join(format!("GameUserSettings.ini.before-display-repair-{ms}")),
            )?;
        }
        if edit.lines != lines || existing.is_none() {
            write_atomic(&user_ini, &join_lines(&edit.lines))?;
        }
        display_repaired = edit.repaired;
    }

    Ok(ConfigOutcome {
        engine_ini,
        display_repaired,
        phoenix_keys_removed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dir_fallbacks() {
        let none = |_: &str| None;
        let base = |dir: PathBuf| CONFIG_SUBDIR.split('/').fold(dir, |d, p| d.join(p));
        assert_eq!(windows_config_dir(none), base(PathBuf::from(r"C:\AppData\Local")));
        assert_eq!(
            windows_config_dir(|n| (n == "USERPROFILE").then(|| OsString::from("H"))),
            base(PathBuf::from("H").join("AppData").join("Local"))
        );
        assert_eq!(
            windows_config_dir(|n| match n {
                "LOCALAPPDATA" => Some(OsString::from("L")),
                "USERPROFILE" => Some(OsString::from("H")),
                _ => None,
            }),
            base(PathBuf::from("L"))
        );
        assert_eq!(
            windows_config_dir(|n| match n {
                "LOCALAPPDATA" => Some(OsString::new()),
                "USERPROFILE" => Some(OsString::from("H")),
                _ => None,
            }),
            base(PathBuf::from("H").join("AppData").join("Local"))
        );
    }

    #[test]
    fn decoding_and_lines() {
        assert_eq!(decode(b"\xEF\xBB\xBFa=1"), "a=1");
        assert_eq!(decode(b"\xFF\xFEa\x00=\x00\xe9\x00"), "a=é");
        assert_eq!(split_lines("a\r\nb\nc\rd"), ["a", "b", "c", "d"]);
        assert_eq!(split_lines("a\r\n"), ["a"]);
        assert_eq!(split_lines("a\n\n"), ["a", ""]);
        assert_eq!(split_lines("\r\r\n"), ["", ""]);
        assert!(split_lines("").is_empty());
    }

    #[test]
    fn encoding() {
        assert_eq!(encode("a=1\r\n"), b"a=1\r\n");
        assert_eq!(encode("é\r\n"), b"\xFF\xFE\xe9\x00\r\x00\n\x00");
        assert_eq!(decode(&encode("x=日本\r\n")), "x=日本\r\n");
        assert_eq!(join_lines(&["a", ""]), "a\r\n\r\n");
    }

    #[test]
    fn hosts() {
        for ok in ["127.0.0.1", "localhost", "a-b.example"] {
            assert!(is_valid_host(ok), "{ok}");
        }
        for bad in ["", "a b", "-a", "a.", "[::1]", "a:1", "a/b", "é"] {
            assert!(!is_valid_host(bad), "{bad}");
        }
        assert!(matches!(
            edit_engine_ini::<&str>(&[], "a b", 1, GraphicsPreset::UNSET),
            Err(ConfigError::InvalidHost)
        ));
        assert!(matches!(
            edit_engine_ini::<&str>(&[], "h", 0, GraphicsPreset::UNSET),
            Err(ConfigError::InvalidPort)
        ));
    }

    #[test]
    fn section_names() {
        for ok in [
            "[/Script/Engine.GameUserSettings]",
            "[/Script/Archon.ArchonGameUserSettings]",
            "[GameUserSettings]",
            " [x.gameusersettings] ",
        ] {
            assert!(is_user_settings_section(ok), "{ok}");
        }
        for bad in [
            "[GameUserSettingsX]",
            "[/Script/Engine.Game_UserSettings]",
            "[a.b1GameUserSettings]",
            "Game",
        ] {
            assert!(!is_user_settings_section(bad), "{bad}");
        }
    }

    fn read(path: &Path) -> String {
        decode(&std::fs::read(path).unwrap())
    }

    #[test]
    fn writes_the_files_and_their_backups() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().join("Config");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("Game.ini"),
            "[OnlineSubsystemPhoenix]\nA=1\n;B=2\n[Keep]\nC=3\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("GameUserSettings.ini"),
            "[/Script/Engine.GameUserSettings]\nResolutionSizeX=0\n",
        )
        .unwrap();
        let outcome = write_game_config(
            &dir,
            &ConfigRequest {
                host: "127.0.0.1",
                port: 61099,
                graphics: GraphicsPreset::UNSET,
                safe_window: false,
                clean_phoenix: true,
            },
        )
        .unwrap();
        assert_eq!(outcome.engine_ini, dir.join("Engine.ini"));
        assert!(outcome.display_repaired);
        assert_eq!(outcome.phoenix_keys_removed, 1);
        assert_eq!(read(&dir.join("Game.ini")), "[Keep]\r\nC=3\r\n");
        assert!(read(&dir.join("Game.ini.before-reforged")).starts_with("[OnlineSubsystemPhoenix]"));
        assert_eq!(
            read(&dir.join("Engine.ini")),
            "[OnlineSubsystemMcp.XMPP]\r\nServerAddr=\"ws://127.0.0.1\"\r\nServerPort=61099\r\nbUseSSL=false\r\n\r\n"
        );
        let backups: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("GameUserSettings.ini.before-display-repair-"))
            .collect();
        assert_eq!(backups.len(), 1);
        assert!(read(&dir.join("GameUserSettings.ini")).contains("ResolutionSizeX=1280\r\n"));
        assert!(!dir.join("Engine.ini.rr-tmp").exists());
    }

    #[test]
    fn absent_user_settings_are_left_absent_unless_the_safe_window_is_asked() {
        let dir = tempfile::tempdir().unwrap();
        let mut request = ConfigRequest {
            host: "h",
            port: 1,
            graphics: GraphicsPreset::new(2).unwrap(),
            safe_window: false,
            clean_phoenix: true,
        };
        let outcome = write_game_config(dir.path(), &request).unwrap();
        assert!(!outcome.display_repaired);
        assert_eq!(outcome.phoenix_keys_removed, 0);
        assert!(!dir.path().join("GameUserSettings.ini").exists());
        assert!(!dir.path().join("Game.ini").exists());

        request.safe_window = true;
        let outcome = write_game_config(dir.path(), &request).unwrap();
        assert!(outcome.display_repaired);
        let text = read(&dir.path().join("GameUserSettings.ini"));
        assert!(text.starts_with("[/Script/Engine.GameUserSettings]\r\nResolutionSizeX=1280\r\n"));
        let backups = std::fs::read_dir(dir.path())
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains("before-display-repair")
            })
            .count();
        assert_eq!(backups, 0);
    }

    #[test]
    fn non_ascii_text_is_written_as_utf16() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Engine.ini"), "[Core.Log]\nNome=Ação\n").unwrap();
        write_game_config(
            dir.path(),
            &ConfigRequest {
                host: "h",
                port: 1,
                graphics: GraphicsPreset::UNSET,
                safe_window: false,
                clean_phoenix: false,
            },
        )
        .unwrap();
        let bytes = std::fs::read(dir.path().join("Engine.ini")).unwrap();
        assert_eq!(&bytes[..2], &[0xFF, 0xFE]);
        assert!(decode(&bytes).ends_with("[Core.Log]\r\nNome=Ação\r\n"));
    }
}
