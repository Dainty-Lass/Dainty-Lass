//! Host-only test cheats for play-testing on a private server: settings.toml's `[switches] test_cheats`, which the
//! host kit writes as `DAINTY_TEST_CHEATS` into the Director's environment (and so every game server's) and refuses
//! on a public server. Unset is off. Decided over the variable's value; the DLL reads it and does the calls.

use crate::names::{Kind, Matcher};

/// The variable and its `env_value` cap.
pub const VARIABLE: (&str, usize) = ("DAINTY_TEST_CHEATS", 15);

/// What a Behemoth's received damage is multiplied by: past any 1.14.7 health pool from one hit.
pub const ONE_HIT_FACTOR: f32 = 100_000.0;

/// The Gauntlet's state component and the shipping build's own cheat on it: a flag `AdvanceToNextState` reads,
/// making the round in progress the level's last (1.14.7 RVA 0x1B15550).
pub const GAUNTLET_STATE_CLASS: &str = "GauntletGameStateComponent";
/// No parameters.
pub const END_GAUNTLET_EARLY: &str = "CheatEndGauntletEarly";

/// The component's state and level timer multicasts: the server sends one as the level starts, so the first call
/// on a component is where the cheat goes.
pub const GAUNTLET_BROADCAST: Matcher = &[
    (
        Kind::Contains,
        ".GauntletGameStateComponent.BroadcastStateTimerToClients",
    ),
    (
        Kind::Contains,
        ".GauntletGameStateComponent.BroadcastLevelTimerToClients",
    ),
];

/// Which cheats are on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cheats {
    /// A Behemoth dies to one hit.
    pub one_hit_kills: bool,
    /// A Gauntlet level ends after its first round.
    pub end_gauntlet_early: bool,
}

impl Cheats {
    /// `on` is every cheat, `kills` one-hit kills only; anything else (unset, `off`, a typo) is none.
    pub fn parse(value: Option<&str>) -> Cheats {
        match value.map(str::trim) {
            Some("on") => Cheats {
                one_hit_kills: true,
                end_gauntlet_early: true,
            },
            Some("kills") => Cheats {
                one_hit_kills: true,
                end_gauntlet_early: false,
            },
            _ => Cheats::default(),
        }
    }

    /// Only a dedicated server takes them: a player's own game hosting a hunt never does.
    pub fn for_process(self, dedicated_server: bool) -> Cheats {
        if dedicated_server { self } else { Cheats::default() }
    }

    /// Whether any is on.
    pub fn any(self) -> bool {
        self.one_hit_kills || self.end_gauntlet_early
    }
}

/// The log lines.
pub mod lines {
    use super::Cheats;

    /// Once per process, when any cheat is on.
    pub fn enabled(cheats: Cheats) -> String {
        format!(
            "[TestCheats] on (DAINTY_TEST_CHEATS): one_hit_kills={} end_gauntlet_early={}",
            cheats.one_hit_kills, cheats.end_gauntlet_early
        )
    }

    /// Each Gauntlet state component the cheat was called on.
    pub fn gauntlet_ended_early(component: usize, called: bool) -> String {
        format!("[TestCheats] CheatEndGauntletEarly on GauntletGameStateComponent {component:#x}: called={called}")
    }

    /// The function could not be found on the component's class.
    pub const GAUNTLET_CHEAT_MISSING: &str =
        "[TestCheats] GauntletGameStateComponent.CheatEndGauntletEarly not found; the level runs every round";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::names::matches;

    #[test]
    fn parses_the_words() {
        assert_eq!(
            Cheats::parse(Some("on")),
            Cheats {
                one_hit_kills: true,
                end_gauntlet_early: true
            }
        );
        assert_eq!(
            Cheats::parse(Some(" kills ")),
            Cheats {
                one_hit_kills: true,
                end_gauntlet_early: false
            }
        );
        for off in [None, Some("off"), Some(""), Some("ON"), Some("1"), Some("yes")] {
            assert!(!Cheats::parse(off).any(), "{off:?}");
        }
    }

    #[test]
    fn only_a_dedicated_server_takes_them() {
        let on = Cheats::parse(Some("on"));
        assert_eq!(on.for_process(true), on);
        assert!(!on.for_process(false).any());
    }

    #[test]
    fn the_broadcast_matcher() {
        assert!(matches(
            GAUNTLET_BROADCAST,
            Some("Function Archon.GauntletGameStateComponent.BroadcastStateTimerToClients")
        ));
        assert!(matches(
            GAUNTLET_BROADCAST,
            Some("Function Archon.GauntletGameStateComponent.BroadcastLevelTimerToClients")
        ));
        assert!(!matches(
            GAUNTLET_BROADCAST,
            Some("Function Archon.EscalationGameStateComponent.BroadcastStateTimerToClients")
        ));
    }

    #[test]
    fn the_lines() {
        assert_eq!(
            lines::enabled(Cheats::parse(Some("kills"))),
            "[TestCheats] on (DAINTY_TEST_CHEATS): one_hit_kills=true end_gauntlet_early=false"
        );
        assert_eq!(
            lines::gauntlet_ended_early(0x1234, true),
            "[TestCheats] CheatEndGauntletEarly on GauntletGameStateComponent 0x1234: called=true"
        );
    }
}
