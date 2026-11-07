//! The backend's own text (rows L14, RA3): the dialogs and menus the backend shows, in the player's language.
//! The text and `Language` are launcher-kit's (RL4, L1; RL8 gave `Language` its wire form).
#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the close confirmation (L5) and the file dialogs (L36) show this text"
    )
)]

use launcher_kit::core::i18n as kit;

pub use kit::{APP_TITLE, CLOSE_ANYWAY, CLOSE_KEEP, CLOSE_TEXT, CLOSE_TITLE, EPIC_SIGN_IN_TITLE, TRAY_OPEN, TRAY_QUIT};

pub use kit::Language;

pub use kit::translate;

/// The language a first start picks from the operating system's locale ("pt-BR", "pt_PT", "en-US"...).
pub fn system_language(locale: &str) -> Language {
    Language::from_system(locale)
}

/// The operating system's language, as `system_language` reads it.
pub fn os_language() -> Language {
    sys_locale::get_locale().map_or(Language::En, |locale| system_language(&locale))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_the_backend_shows_has_text_in_every_language() {
        for language in Language::ALL {
            for key in kit::KEYS {
                let text = translate(language, key, &[]);
                assert!(!text.trim().is_empty() && text != key, "{} {key}", language.code());
            }
        }
    }

    #[test]
    fn every_page_language_is_one_the_catalog_has() {
        for language in Language::ALL {
            assert_eq!(Language::from_code(language.code()), Some(language));
        }
    }

    #[test]
    fn translates_like_the_page() {
        assert_ne!(
            translate(Language::PtBr, CLOSE_ANYWAY, &[]),
            translate(Language::En, CLOSE_ANYWAY, &[])
        );
        assert_eq!(translate(Language::PtBr, "x {a}, {a}", &[("a", "Ana")]), "x Ana, Ana");
        assert_eq!(translate(Language::En, "no_such_key", &[]), "no_such_key");
        assert_eq!(translate(Language::En, "x {a} {a}", &[("a", "1")]), "x 1 1");
        assert_eq!(Language::from_code("pt-BR"), Some(Language::PtBr));
        assert_eq!(Language::from_code("pt"), None);
        assert_eq!(Language::ALL[0], Language::En);
    }

    #[test]
    fn system_language_is_the_pages() {
        for (locale, language) in [
            ("pt-BR", Language::PtBr),
            ("pt-PT", Language::PtBr),
            ("pt_BR", Language::PtBr),
            ("PT-br", Language::PtBr),
            ("pt", Language::PtBr),
            ("ptx", Language::En),
            ("en-US", Language::En),
            ("en", Language::En),
            ("fi", Language::En),
            ("", Language::En),
        ] {
            assert_eq!(system_language(locale), language, "{locale}");
        }
    }
}
