//! The settings file (row L28) over launcher-kit's `platform::settings` (RL6), which sanitises and writes it
//! byte for byte. Here: the shell's names, and its `Language` (the bridge's) mapped to the MIT catalog's.

use std::path::Path;

use launcher_kit::platform::settings as kit;
use serde_json::{Map, Value};

use crate::core::i18n::Language;

/// `game` of a Dauntless Revived server (Dauntless 1.4.4), which 2.0.0 no longer plays (RD5)
pub const REVIVED: &str = "revived";
/// `game` of every other server; older settings have none
pub const REFORGED: &str = "reforged";

/// Which folder-path rule applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Linux,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(windows) { Self::Windows } else { Self::Linux }
    }

    fn os(self) -> kit::Os {
        match self {
            Self::Windows => kit::Os::Windows,
            Self::Linux => kit::Os::Linux,
        }
    }
}

fn kit_language(language: Language) -> launcher_kit::core::i18n::Language {
    launcher_kit::core::i18n::Language::from_code(language.code()).unwrap_or_default()
}

/// A saved Dauntless Revived (1.4.4) server: listed, not playable (RD5).
pub fn is_revived(server: &Map<String, Value>) -> bool {
    server.get("game").and_then(Value::as_str) == Some(REVIVED)
}

pub fn default_settings(language: Language) -> Map<String, Value> {
    kit::defaults(kit_language(language))
}

pub fn sanitize_settings(raw: &Value, language: Language, platform: Platform) -> Map<String, Value> {
    kit::sanitize(raw, kit_language(language), platform.os())
}

/// The file text, as `JSON.stringify(settings, null, 2)`.
pub fn settings_text(settings: &Map<String, Value>) -> String {
    String::from_utf8(kit::to_bytes(settings)).unwrap_or_default()
}

pub struct SettingsStore(kit::SettingsStore);

impl SettingsStore {
    pub fn load(dir: &Path, language: Language, platform: Platform) -> Self {
        Self(kit::SettingsStore::load(dir, kit_language(language), platform.os()))
    }

    pub fn get(&self) -> &Map<String, Value> {
        self.0.map()
    }

    /// Changes a copy, checks it again, and writes it atomically.
    pub fn update(&mut self, change: impl FnOnce(&mut Map<String, Value>)) -> std::io::Result<&Map<String, Value>> {
        self.0.update(change)?;
        Ok(self.0.map())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;

    #[derive(Deserialize)]
    struct Case {
        platform: String,
        #[serde(default)]
        input: Value,
        language: String,
        text: String,
    }

    /// Until RA3 makes the languages `en` and `pt-BR`, a case that stores another language is skipped.
    fn languages_are_reforged() -> bool {
        Language::from_code("pt-BR").is_some() && Language::from_code("fi").is_none()
    }

    /// `test/cases/settings.json`, made by Reforged's `sanitizeSettings` (the Node test
    /// `platform-tables.test.ts` checks it there) less the 1.4.4 parts this launcher drops.
    #[test]
    fn the_shared_table_byte_for_byte() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../test/cases/settings.json");
        let cases: Vec<Case> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert!(cases.len() > 100);
        let mut run = 0;
        for case in cases {
            let stored_language = case.input.get("language").and_then(Value::as_str);
            if !languages_are_reforged() && stored_language.is_some_and(|code| code != "en" && code != "xx") {
                continue;
            }
            let platform = if case.platform == "win32" {
                Platform::Windows
            } else {
                Platform::Linux
            };
            let language = Language::from_code(&case.language).unwrap();
            let settings = sanitize_settings(&case.input, language, platform);
            assert_eq!(settings_text(&settings), case.text, "{} {}", case.platform, case.input);
            run += 1;
        }
        assert!(run > 100);
    }

    #[test]
    fn the_tray_settings_are_kept_only_when_on() {
        let on = sanitize_settings(
            &json!({ "minimizeToTray": true, "closeToTray": "yes" }),
            Language::En,
            Platform::Windows,
        );
        assert_eq!(on.get("minimizeToTray"), Some(&Value::Bool(true)));
        assert_eq!(on.get("closeToTray"), None);
        let stored = crate::controller::stored::Stored::of(&on);
        assert!(stored.minimize_to_tray && !stored.close_to_tray);
        let off = sanitize_settings(&json!({ "minimizeToTray": false }), Language::En, Platform::Windows);
        assert_eq!(off.get("minimizeToTray"), None);
    }

    const FP: &str = "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1";

    #[test]
    fn a_profile_with_a_1_4_4_server_loads_and_keeps_it() {
        let revived = json!({ "game": "revived", "mode": "public", "host": "old.example.org", "port": 443, "name": "Old", "share": null, "fp": FP, "code": null });
        let raw = json!({
            "version": 1,
            "server": revived,
            "savedServers": [revived, { "mode": "private", "host": "100.64.0.1", "port": 61000, "name": "Friends" }],
            "revivedRoot": "D:\\Dauntless144",
            "revivedVerified": "D:\\Dauntless144",
        });
        let settings = sanitize_settings(&raw, Language::En, Platform::Windows);
        assert_eq!(
            settings["server"],
            Value::Null,
            "never the current server in the reborn edition"
        );
        let saved = settings["savedServers"].as_array().unwrap();
        assert_eq!(saved.len(), 2);
        assert!(is_revived(saved[0].as_object().unwrap()));
        assert_eq!(saved[1]["game"], "reforged", "no game: a Reforged server");
        assert!(settings.get("revivedRoot").is_none() && settings.get("revivedVerified").is_none());
        assert!(settings.get("gameEdition").is_none());
    }

    #[test]
    fn usernames_follow_the_registration_rule() {
        let slot = "0123456789abcdef01234567";
        let names = |name: Value| {
            sanitize_settings(&json!({ "usernames": { slot: name } }), Language::En, Platform::Linux)["usernames"]
                .get(slot)
                .cloned()
        };
        assert_eq!(names(json!("Slayer_01")), Some(json!("Slayer_01")));
        for bad in [
            json!("Sam Smith"),
            json!("ab"),
            json!("a".repeat(17)),
            json!("Bjørn"),
            json!(7),
        ] {
            assert_eq!(names(bad.clone()), None, "{bad}");
        }
    }

    #[test]
    fn a_saved_file_reads_back_the_same() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = SettingsStore::load(dir.path(), Language::En, Platform::Windows);
        store
            .update(|settings| {
                settings.insert("gameRoot".into(), Value::from("D:\\Games\\Dauntless"));
                settings.insert("runtimeProfile".into(), Value::from("development"));
                // Dropped from DR: ignored when stored
                settings.insert("huntRegion".into(), Value::from("ger"));
            })
            .unwrap();
        let text = std::fs::read_to_string(dir.path().join("settings.json")).unwrap();
        let other = Language::ALL
            .into_iter()
            .find(|language| *language != Language::En)
            .unwrap();
        let again = SettingsStore::load(dir.path(), other, Platform::Windows);
        assert_eq!(settings_text(again.get()), text);
        assert_eq!(again.get()["gameRoot"], "D:\\Games\\Dauntless");
        assert_eq!(again.get()["runtimeProfile"], "development");
        assert!(again.get().get("huntRegion").is_none());
        assert_eq!(again.get()["language"], "en", "a saved language wins over the system's");
    }
}
