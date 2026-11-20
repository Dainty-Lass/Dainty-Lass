//! The Trials boards in Ramsgate, refreshed on every arrival (spec-runtime-trials-board). The client reads them once
//! at login and keeps the answer, so a run finished since never shows until the next login.

/// The class whose instance on the local controller asks for the boards.
pub const ARENA_COMPONENT: &str = "PlayerArenaComponent";
/// Drops the cached boards; no parameters.
pub const INVALIDATE: &str = "InvalidateLeaderboardCache";
/// Asks for one board: `ELeaderboardType LeaderboardTypeForQuery`, one byte.
pub const QUERY: &str = "QueryTopLeaderboardsForDiegetic";
/// `ELeaderboardType::Solo` and `::Group`, the two boards in Ramsgate.
pub const BOARDS: [u8; 2] = [1, 2];
/// The flag file that turns the refresh off.
pub const DISABLE_FLAG: &str = "DISABLE_TRIALS_BOARD_REFRESH";

/// Whether a world's full name is Ramsgate's (`/Game/Maps/ramsgate/ramsgate_01_persistent`).
pub fn is_ramsgate(world_full_name: &str) -> bool {
    world_full_name.to_ascii_lowercase().contains("ramsgate_01_persistent")
}

/// One refresh per world: the next world to refresh, given the last one refreshed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Once {
    last: usize,
}

impl Once {
    /// A fresh tracker.
    pub const fn new() -> Self {
        Self { last: 0 }
    }

    /// True the first time `world` is seen since another one was.
    pub fn take(&mut self, world: usize) -> bool {
        if world == 0 || world == self.last {
            return false;
        }
        self.last = world;
        true
    }
}

/// The line logged for a refresh.
pub fn refresh_line(invalidated: bool, queried: [bool; 2]) -> String {
    format!(
        "[TrialsBoard] Ramsgate: cache dropped={invalidated}, solo asked={}, group asked={}",
        queried[0], queried[1]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_ramsgate_is_refreshed() {
        assert!(is_ramsgate(
            "World /Game/Maps/ramsgate/ramsgate_01_persistent.ramsgate_01_persistent"
        ));
        assert!(is_ramsgate("World /Game/Maps/Ramsgate/Ramsgate_01_Persistent.x"));
        assert!(!is_ramsgate(
            "World /Game/Maps/islands/arenas/arena_ramsgate_00.arena_ramsgate_00"
        ));
        assert!(!is_ramsgate(""));
    }

    #[test]
    fn once_per_world() {
        let mut once = Once::new();
        assert!(!once.take(0));
        assert!(once.take(10));
        assert!(!once.take(10));
        assert!(once.take(20));
        assert!(once.take(10));
    }

    #[test]
    fn the_line() {
        assert_eq!(
            refresh_line(true, [true, false]),
            "[TrialsBoard] Ramsgate: cache dropped=true, solo asked=true, group asked=false"
        );
    }
}
