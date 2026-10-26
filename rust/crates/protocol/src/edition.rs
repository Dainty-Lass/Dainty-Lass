//! Game editions (`Refined` and `Reforged`) supported by the multi-edition controller and client.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// A game edition supported by a server or launcher binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edition {
    Refined,
    Reforged,
}

impl Edition {
    pub const ALL: [Self; 2] = [Self::Refined, Self::Reforged];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Refined => "refined",
            Self::Reforged => "reforged",
        }
    }

    /// Maps `Edition` to [`crate::game::Game`] (`Reforged` -> `Reborn`, `Refined` -> `Legacy`).
    pub const fn to_game(self) -> crate::game::Game {
        match self {
            Self::Reforged => crate::game::Game::Reborn,
            Self::Refined => crate::game::Game::Legacy,
        }
    }

    /// Maps [`crate::game::Game`] to `Edition` (`Reborn` -> `Reforged`, `Legacy` -> `Refined`).
    pub const fn from_game(game: crate::game::Game) -> Self {
        match game {
            crate::game::Game::Reborn => Self::Reforged,
            crate::game::Game::Legacy => Self::Refined,
        }
    }

    /// Default loopback port for this edition's backend when running single-edition or concurrently.
    pub const fn default_backend_port(self) -> u16 {
        match self {
            Self::Reforged => 61000,
            Self::Refined => 61010,
        }
    }

    /// Default loopback port for this edition's director when running single-edition or concurrently.
    pub const fn default_director_port(self) -> u16 {
        match self {
            Self::Reforged => 61001,
            Self::Refined => 61011,
        }
    }

    /// Default UDP hunt port range `(first, last)` for this edition.
    pub const fn default_udp_ports(self) -> (u16, u16) {
        match self {
            Self::Reforged => (8770, 8777),
            Self::Refined => (8780, 8787),
        }
    }

    /// Alias for [`Self::default_udp_ports`].
    pub const fn default_udp_port_range(self) -> (u16, u16) {
        self.default_udp_ports()
    }

    /// Parses `"refined"` or `"reforged"` (case-insensitive).
    pub fn parse(s: &str) -> Option<Self> {
        s.parse().ok()
    }

    /// Whether this binary was compiled with support for `self`.
    pub const fn is_compiled(self) -> bool {
        match self {
            Self::Refined => cfg!(feature = "edition-refined"),
            Self::Reforged => cfg!(feature = "edition-reforged"),
        }
    }

    /// Editions enabled in this binary at compile time.
    pub fn compiled_editions() -> Vec<Self> {
        Self::ALL.into_iter().filter(|e| e.is_compiled()).collect()
    }
}

impl fmt::Display for Edition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Edition {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "refined" => Ok(Self::Refined),
            "reforged" => Ok(Self::Reforged),
            other => Err(format!("unknown edition {other:?} (expected 'refined' or 'reforged')")),
        }
    }
}

/// Which edition(s) a server or client profile targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EditionSelection {
    Refined,
    Reforged,
    Both,
}

impl EditionSelection {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Refined => "refined",
            Self::Reforged => "reforged",
            Self::Both => "both",
        }
    }

    pub fn editions(self) -> Vec<Edition> {
        match self {
            Self::Refined => vec![Edition::Refined],
            Self::Reforged => vec![Edition::Reforged],
            Self::Both => vec![Edition::Reforged, Edition::Refined],
        }
    }

    pub const fn includes(self, edition: Edition) -> bool {
        matches!(
            (self, edition),
            (Self::Both, _) | (Self::Refined, Edition::Refined) | (Self::Reforged, Edition::Reforged)
        )
    }
}

impl FromStr for EditionSelection {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "refined" => Ok(Self::Refined),
            "reforged" => Ok(Self::Reforged),
            "both" => Ok(Self::Both),
            other => Err(format!(
                "unknown edition selection {other:?} (expected 'refined', 'reforged', or 'both')"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_editions_and_defaults() {
        assert_eq!("refined".parse::<Edition>().unwrap(), Edition::Refined);
        assert_eq!("Reforged".parse::<Edition>().unwrap(), Edition::Reforged);
        assert!("invalid".parse::<Edition>().is_err());
        assert_eq!(Edition::Reforged.default_backend_port(), 61000);
        assert_eq!(Edition::Refined.default_backend_port(), 61010);
        assert_eq!(Edition::Reforged.default_director_port(), 61001);
        assert_eq!(Edition::Refined.default_director_port(), 61011);
        assert_eq!(Edition::Reforged.default_udp_ports(), (8770, 8777));
        assert_eq!(Edition::Refined.default_udp_ports(), (8780, 8787));
        assert_eq!(
            "both".parse::<EditionSelection>().unwrap().editions(),
            vec![Edition::Reforged, Edition::Refined]
        );
    }
}
