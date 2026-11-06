//! The game's own text language (row L16) over launcher-kit's `core::game_language` (RL4): the setting's
//! enum is the bridge's, the rules are the MIT crate's. The setting is sent to the game as `-epiclocale`.
#![cfg_attr(not(test), allow(dead_code, reason = "the game launch (L31) sends it"))]

use launcher_kit::core::game_language as kit;

use crate::bridge::types::GameLanguage;

impl GameLanguage {
    /// The culture name, or None for "auto".
    pub fn locale(self) -> Option<&'static str> {
        Some(match self {
            Self::Auto => return None,
            Self::EnUs => "en-US",
            Self::DeDe => "de-DE",
            Self::EsEs => "es-ES",
            Self::FrFr => "fr-FR",
            Self::ItIt => "it-IT",
            Self::JaJp => "ja-JP",
            Self::PtBr => "pt-BR",
            Self::RuRu => "ru-RU",
        })
    }
}

/// The `-epiclocale` for the Game language setting; "auto" follows the system where the game has text.
pub fn game_locale(setting: GameLanguage, system_locale: &str) -> &'static str {
    setting.locale().unwrap_or_else(|| kit::from_os_locale(system_locale))
}

/// Never an argument the game has no text for.
pub fn launch_locale(locale: Option<&str>) -> &'static str {
    locale.map_or(kit::DEFAULT_LOCALE, kit::launch_locale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use launcher_kit::core::game_language::GAME_LOCALES;

    #[test]
    fn auto_follows_the_system_when_the_game_has_that_language() {
        assert_eq!(launch_locale(None), "en-US");
        assert_eq!(launch_locale(Some("es-ES")), "es-ES");
        assert_eq!(launch_locale(Some("pt-BR")), "pt-BR");
        assert_eq!(launch_locale(Some("xx-XX")), "en-US");
        assert_eq!(game_locale(GameLanguage::Auto, "es-ES"), "es-ES");
        assert_eq!(game_locale(GameLanguage::Auto, "es-419"), "es-ES");
        assert_eq!(game_locale(GameLanguage::Auto, "de"), "de-DE");
        assert_eq!(game_locale(GameLanguage::Auto, "pt-PT"), "pt-BR");
        assert_eq!(
            game_locale(GameLanguage::Auto, "fi-FI"),
            "en-US",
            "no Finnish text in the game"
        );
        assert_eq!(game_locale(GameLanguage::Auto, ""), "en-US");
        assert_eq!(
            game_locale(GameLanguage::JaJp, "en-US"),
            "ja-JP",
            "a chosen language wins over the system"
        );
    }

    #[test]
    fn every_setting_but_auto_is_a_game_locale() {
        let all = [
            GameLanguage::EnUs,
            GameLanguage::DeDe,
            GameLanguage::EsEs,
            GameLanguage::FrFr,
            GameLanguage::ItIt,
            GameLanguage::JaJp,
            GameLanguage::PtBr,
            GameLanguage::RuRu,
        ];
        for setting in all {
            let locale = setting.locale().unwrap();
            assert!(GAME_LOCALES.contains(&locale));
            assert_eq!(serde_json::to_value(setting).unwrap(), serde_json::Value::from(locale));
        }
        assert_eq!(all.len(), GAME_LOCALES.len());
    }
}
