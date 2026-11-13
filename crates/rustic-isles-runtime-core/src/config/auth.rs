//! The server auth mode (R1 5). Server role only.

use crate::env::Raw;

/// The variable.
pub const VARIABLE: &str = "RUSTIC_ISLES_AUTH_MODE";
/// Its raw read's cap.
pub const READ_CAP: usize = 32;
/// The exit code R9 ends an `Invalid` server with, before it listens.
pub const INVALID_EXIT_CODE: u32 = 0xE102;

/// Who started this server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    /// A DeployServer gameserver (unset, or `GLOBAL_GAMESERVER`).
    GlobalGameserver,
    /// A player host (`HUNT_CAPABILITY`); whether it is honoured is R10's.
    PlayerHost,
    /// Anything else, including a value too long to read (fail closed, R1 departure 2).
    Invalid,
}

/// Classifies the raw read; exact, case-sensitive matches.
pub fn auth_mode(raw: &Raw) -> AuthMode {
    match raw {
        Raw::Unset => AuthMode::GlobalGameserver,
        Raw::Value(value) if value == "GLOBAL_GAMESERVER" => AuthMode::GlobalGameserver,
        Raw::Value(value) if value == "HUNT_CAPABILITY" => AuthMode::PlayerHost,
        Raw::Value(_) | Raw::TooLong(_) => AuthMode::Invalid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(value: Option<&str>) -> AuthMode {
        auth_mode(&Raw::from_text(value, READ_CAP))
    }

    #[test]
    fn table() {
        assert_eq!(mode(None), AuthMode::GlobalGameserver);
        assert_eq!(mode(Some("")), AuthMode::GlobalGameserver);
        assert_eq!(mode(Some("GLOBAL_GAMESERVER")), AuthMode::GlobalGameserver);
        assert_eq!(mode(Some("HUNT_CAPABILITY")), AuthMode::PlayerHost);
        assert_eq!(mode(Some("hunt_capability")), AuthMode::Invalid);
        assert_eq!(mode(Some("GLOBAL_GAMESERVER ")), AuthMode::Invalid);
        assert_eq!(mode(Some("HUNT_CAPABILITY\r")), AuthMode::Invalid);
        assert_eq!(mode(Some(&"X".repeat(33))), AuthMode::Invalid);
    }
}
