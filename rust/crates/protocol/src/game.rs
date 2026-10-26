//! Which Dauntless a server plays: Reforged's 1.14.7 on Mystic Paradox ("reborn"), or Dauntless Revived's
//! 1.4.4 ("legacy"). Legacy servers are carried only with the `legacy` feature; without it a
//! `dauntless-revived://` invite is refused as RD5 had it.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum Game {
    /// Dauntless 1.14.7 on Mystic Paradox
    #[default]
    Reborn,
    /// Dauntless Revived's Dauntless 1.4.4
    Legacy,
}

impl Game {
    /// Every game this code knows, built in or not.
    pub const ALL: [Game; 2] = [Game::Reborn, Game::Legacy];

    /// The Dauntless version the game is, as an invite's `version` names it.
    pub fn version(self) -> &'static str {
        match self {
            Self::Reborn => "1.14.7",
            Self::Legacy => "1.4.4",
        }
    }

    /// The other name an invite's `version` may give it: the project that serves that version.
    pub fn version_alias(self) -> &'static str {
        match self {
            Self::Reborn => "reforged",
            Self::Legacy => "revived",
        }
    }

    /// The game a version (or its alias) names, built in or not.
    pub fn from_version(version: &str) -> Option<Game> {
        Self::ALL
            .into_iter()
            .find(|game| game.version() == version || game.version_alias() == version)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reborn => "reborn",
            Self::Legacy => "legacy",
        }
    }

    pub fn is_reborn(&self) -> bool {
        *self == Self::Reborn
    }

    /// Whether this build carries the game at all.
    pub fn supported(self) -> bool {
        self == Self::Reborn || cfg!(feature = "legacy")
    }
}

/// The games this build carries, set by its features.
pub fn supported_games() -> Vec<Game> {
    Game::ALL.into_iter().filter(|game| game.supported()).collect()
}

/// The game versions this build plays: what an invite's `version` may name.
pub fn supported_versions() -> Vec<&'static str> {
    supported_games().into_iter().map(Game::version).collect()
}

impl std::fmt::Display for Game {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_versions_follow_the_features() {
        let expected: &[&str] = if cfg!(feature = "legacy") {
            &["1.14.7", "1.4.4"]
        } else {
            &["1.14.7"]
        };
        assert_eq!(supported_versions(), expected);
        for game in Game::ALL {
            assert_eq!(Game::from_version(game.version()), Some(game));
            assert_eq!(Game::from_version(game.version_alias()), Some(game));
        }
        assert_eq!(Game::from_version("reforged"), Some(Game::Reborn));
        assert_eq!(Game::from_version("revived"), Some(Game::Legacy));
        for unknown in ["1.4", "Revived", "reborn", "legacy"] {
            assert_eq!(Game::from_version(unknown), None, "{unknown}");
        }
    }
}
