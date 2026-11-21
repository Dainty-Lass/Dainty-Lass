// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The backend's own text: the few strings the launcher's Rust side shows itself (dialogs, pickers, the tray),
//! from the MIT catalog in `catalog` (L1). The page's text is its own.

mod catalog;

/// The app's name, as the window's title.
pub const APP_TITLE: &str = "app_title";
/// The keep-open dialog's title.
pub const CLOSE_TITLE: &str = "close_title";
/// The keep-open dialog's question.
pub const CLOSE_TEXT: &str = "close_text";
/// The keep-open dialog's button that keeps the launcher open.
pub const CLOSE_KEEP: &str = "close_keep";
/// The keep-open dialog's button that closes it anyway.
pub const CLOSE_ANYWAY: &str = "close_anyway";
/// The game-folder picker's title.
pub const DIALOG_GAME_FOLDER: &str = "dialog_game_folder";
/// The local-runtime-folder picker's title.
pub const DIALOG_RUNTIME_FOLDER: &str = "dialog_runtime_folder";
/// The Host-workspace picker's title.
pub const DIALOG_WORKSPACE_FOLDER: &str = "dialog_workspace_folder";
/// The SDK-dump-folder picker's title.
pub const DIALOG_SDK_FOLDER: &str = "dialog_sdk_folder";
/// The Dumper-7 DLL picker's title.
pub const DIALOG_DUMPER_DLL: &str = "dialog_dumper_dll";
/// The game-data-pack picker's title.
pub const DIALOG_GAMEDATA_PACK: &str = "dialog_gamedata_pack";
/// The Epic sign-in window's title.
pub const EPIC_SIGN_IN_TITLE: &str = "epic_sign_in_title";
/// The tray item that shows the window.
pub const TRAY_OPEN: &str = "tray_open";
/// The tray item that quits the app.
pub const TRAY_QUIT: &str = "tray_quit";
/// The Epic install folder picker's title.
pub const DIALOG_GAME_INSTALL_FOLDER: &str = "dialog_game_install_folder";
/// The browser page after an OAuth sign-in reached the launcher.
pub const OAUTH_PAGE_DONE: &str = "oauth_page_done";
/// The browser page after an OAuth sign-in that did not.
pub const OAUTH_PAGE_FAILED: &str = "oauth_page_failed";

/// Every key the backend shows.
pub const KEYS: [&str; 17] = [
    APP_TITLE,
    CLOSE_TITLE,
    CLOSE_TEXT,
    CLOSE_KEEP,
    CLOSE_ANYWAY,
    DIALOG_GAME_FOLDER,
    DIALOG_RUNTIME_FOLDER,
    DIALOG_WORKSPACE_FOLDER,
    DIALOG_SDK_FOLDER,
    DIALOG_DUMPER_DLL,
    DIALOG_GAMEDATA_PACK,
    EPIC_SIGN_IN_TITLE,
    TRAY_OPEN,
    TRAY_QUIT,
    DIALOG_GAME_INSTALL_FOLDER,
    OAUTH_PAGE_DONE,
    OAUTH_PAGE_FAILED,
];

/// A language the backend's text is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(any(test, feature = "ts"), derive(ts_rs::TS))]
pub enum Language {
    /// English, the fallback.
    #[default]
    #[serde(rename = "en")]
    En,
    /// Brazilian Portuguese.
    #[serde(rename = "pt-BR")]
    PtBr,
}

impl Language {
    /// Every language, English first.
    pub const ALL: [Self; 2] = [Self::En, Self::PtBr];

    /// The language's code, as the page and the settings write it.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::PtBr => "pt-BR",
        }
    }

    /// The language with exactly this code.
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|language| language.code() == code)
    }

    /// The language for an OS locale such as `pt_BR` or `en-GB`: the same code in any case, else the first language
    /// with the same language part, else English.
    #[must_use]
    pub fn from_system(locale: &str) -> Self {
        let locale = locale.to_lowercase().replace('_', "-");
        if let Some(exact) = Self::ALL
            .into_iter()
            .find(|language| language.code().eq_ignore_ascii_case(&locale))
        {
            return exact;
        }
        let part = locale.split('-').next().unwrap_or_default();
        Self::ALL
            .into_iter()
            .find(|language| language_part(language.code()) == part)
            .unwrap_or_default()
    }
}

fn language_part(code: &str) -> String {
    code.split('-').next().unwrap_or_default().to_lowercase()
}

/// The text for `key` in `language`, else in English, else the key itself, with every `{name}` in `vars` replaced
/// by its value. A `{name}` not in `vars` stays as written.
#[must_use]
pub fn translate(language: Language, key: &str, vars: &[(&str, &str)]) -> String {
    let template = catalog::lookup(language, key)
        .or_else(|| catalog::lookup(Language::En, key))
        .unwrap_or(key);
    fill(template, vars)
}

// One pass, so a value that itself contains `{name}` is never expanded again.
fn fill(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let value = after.find('}').and_then(|close| {
            let name = &after[..close];
            vars.iter()
                .find(|(var, _)| *var == name)
                .map(|(_, value)| (*value, close))
        });
        match value {
            Some((value, close)) => {
                out.push_str(value);
                rest = &after[close + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn placeholders(text: &str) -> BTreeSet<String> {
        let mut found = BTreeSet::new();
        let mut rest = text;
        while let Some(open) = rest.find('{') {
            rest = &rest[open + 1..];
            if let Some(close) = rest.find('}') {
                found.insert(rest[..close].to_owned());
                rest = &rest[close + 1..];
            }
        }
        found
    }

    #[test]
    fn every_key_is_in_every_language_and_not_empty() {
        for language in Language::ALL {
            for key in KEYS {
                let text = catalog::lookup(language, key);
                assert!(
                    text.is_some_and(|text| !text.trim().is_empty()),
                    "{key} missing or empty in {language:?}"
                );
            }
        }
    }

    #[test]
    fn the_catalog_holds_only_the_known_keys_once() {
        for language in Language::ALL {
            let keys: Vec<&str> = catalog::entries(language).iter().map(|(key, _)| *key).collect();
            let unique: BTreeSet<&str> = keys.iter().copied().collect();
            assert_eq!(keys.len(), unique.len(), "duplicate key in {language:?}");
            assert_eq!(
                unique,
                KEYS.into_iter().collect(),
                "unknown or missing key in {language:?}"
            );
        }
    }

    #[test]
    fn every_language_has_englishs_placeholders() {
        for key in KEYS {
            let english = placeholders(&translate(Language::En, key, &[]));
            for language in Language::ALL {
                assert_eq!(
                    placeholders(&translate(language, key, &[])),
                    english,
                    "{key} in {language:?}"
                );
            }
        }
    }

    #[test]
    fn languages_differ_where_they_should() {
        assert_ne!(
            translate(Language::En, TRAY_QUIT, &[]),
            translate(Language::PtBr, TRAY_QUIT, &[])
        );
    }

    #[test]
    fn an_unknown_key_falls_back_to_itself() {
        assert_eq!(translate(Language::PtBr, "no_such_key", &[]), "no_such_key");
        assert_eq!(translate(Language::En, "no_such_{x}", &[("x", "key")]), "no_such_key");
    }

    #[test]
    fn placeholders_are_filled_once() {
        assert_eq!(fill("Join {name}?", &[("name", "Isle")]), "Join Isle?");
        assert_eq!(fill("{a}{a} {b}", &[("a", "x"), ("b", "y")]), "xx y");
        assert_eq!(fill("{a}", &[("a", "{a}")]), "{a}");
        assert_eq!(fill("{missing} {", &[("a", "x")]), "{missing} {");
        assert_eq!(fill("{{a}}", &[("a", "x")]), "{x}");
        assert_eq!(fill("é {a} ü", &[("a", "ß")]), "é ß ü");
    }

    #[test]
    fn codes_are_exact() {
        assert_eq!(Language::from_code("en"), Some(Language::En));
        assert_eq!(Language::from_code("pt-BR"), Some(Language::PtBr));
        assert_eq!(Language::from_code("pt-br"), None);
        assert_eq!(Language::from_code("pt"), None);
        assert_eq!(Language::from_code(""), None);
        for language in Language::ALL {
            assert_eq!(Language::from_code(language.code()), Some(language));
        }
    }

    #[test]
    fn system_language() {
        assert_eq!(Language::from_system("pt_BR"), Language::PtBr);
        assert_eq!(Language::from_system("PT-br"), Language::PtBr);
        assert_eq!(Language::from_system("pt-PT"), Language::PtBr);
        assert_eq!(Language::from_system("pt"), Language::PtBr);
        assert_eq!(Language::from_system("en_GB"), Language::En);
        assert_eq!(Language::from_system("de-DE"), Language::En);
        assert_eq!(Language::from_system(""), Language::En);
    }
}
