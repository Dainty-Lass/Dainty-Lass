// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The language the game is started in: a locale from the launcher's setting, or from the OS's locale for `auto`.

/// The locales the game accepts, in the order `auto` matches them.
pub const GAME_LOCALES: [&str; 8] = ["en-US", "de-DE", "es-ES", "fr-FR", "it-IT", "ja-JP", "pt-BR", "ru-RU"];

/// The locale used when nothing else matches.
pub const DEFAULT_LOCALE: &str = "en-US";

/// The setting value that follows the OS's locale.
pub const AUTO: &str = "auto";

/// The locale for the game-language setting: a listed locale as it is, `auto` from `os_locale`, anything else
/// [`DEFAULT_LOCALE`].
#[must_use]
pub fn resolve(setting: &str, os_locale: Option<&str>) -> &'static str {
    if setting == AUTO {
        return os_locale.map_or(DEFAULT_LOCALE, from_os_locale);
    }
    launch_locale(setting)
}

/// The locale for an OS locale such as `es-419` or `pt_PT`: the first listed locale with the same two-letter
/// language, else [`DEFAULT_LOCALE`].
#[must_use]
pub fn from_os_locale(os_locale: &str) -> &'static str {
    // Two characters only: a region the game lacks (`es-419`, `pt-PT`) lands on its one locale for the language.
    let language: String = os_locale.chars().take(2).flat_map(char::to_lowercase).collect();
    GAME_LOCALES
        .iter()
        .find(|locale| locale.split('-').next() == Some(language.as_str()))
        .copied()
        .unwrap_or(DEFAULT_LOCALE)
}

/// The locale passed on the game's command line: only a listed one, else [`DEFAULT_LOCALE`].
#[must_use]
pub fn launch_locale(locale: &str) -> &'static str {
    GAME_LOCALES
        .iter()
        .find(|listed| **listed == locale)
        .copied()
        .unwrap_or(DEFAULT_LOCALE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_set_language_is_its_locale() {
        for locale in GAME_LOCALES {
            assert_eq!(resolve(locale, Some("ja-JP")), locale);
        }
    }

    #[test]
    fn auto_matches_the_language_part_in_order() {
        assert_eq!(resolve(AUTO, Some("es-419")), "es-ES");
        assert_eq!(resolve(AUTO, Some("pt-PT")), "pt-BR");
        assert_eq!(resolve(AUTO, Some("DE_at")), "de-DE");
        assert_eq!(resolve(AUTO, Some("ja")), "ja-JP");
        assert_eq!(resolve(AUTO, Some("en-GB")), "en-US");
    }

    #[test]
    fn auto_falls_back_to_english() {
        assert_eq!(resolve(AUTO, Some("nl-NL")), "en-US");
        assert_eq!(resolve(AUTO, Some("")), "en-US");
        assert_eq!(resolve(AUTO, Some("p")), "en-US");
        assert_eq!(resolve(AUTO, None), "en-US");
    }

    #[test]
    fn launch_accepts_only_listed_locales() {
        assert_eq!(launch_locale("fr-FR"), "fr-FR");
        assert_eq!(launch_locale("fr-fr"), "en-US");
        assert_eq!(launch_locale("fr"), "en-US");
        assert_eq!(launch_locale("es-419"), "en-US");
        assert_eq!(launch_locale(""), "en-US");
        assert_eq!(resolve("klingon", Some("de-DE")), "en-US");
    }
}
