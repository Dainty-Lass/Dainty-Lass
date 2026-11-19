//! Host-only test cheats for play-testing on a private server: settings.toml's `[switches] test_cheats`, which the
//! host kit writes as `DAINTY_TEST_CHEATS` into the Director's environment (and so every game server's) and refuses
//! on a public server. Unset is off. Decided over the variable's value; the DLL reads it and does the calls.

use crate::names::{Kind, Matcher};

/// The variable and its `env_value` cap.
pub const VARIABLE: (&str, usize) = ("DAINTY_TEST_CHEATS", 15);

/// A Behemoth's health is a float on its gameplay-ability attribute set, changed by gameplay effects rather than
/// `TakeDamage` (whose `Receive*Damage` events only get a copy). 1.14.7's `ArchonBehemoth::GetCurrentHealth`
/// (RVA 0x188AE40) and `GetMaxHealth` (0x188C4E0) read these offsets on this class.
pub const HEALTH_SET_CLASS: &str = "ArchonHealthAttributeSet";
/// `CurrentHealth`, a replicated float (the class's property table).
pub const HEALTH: usize = 0x30;
/// `CurrentShield`, which takes a hit before health: held at 0.
pub const SHIELD: usize = 0x34;
/// `MaxHealth`.
pub const MAX_HEALTH: usize = 0x38;
/// What an attribute set must belong to (its outer, or its outer's outer) to be held low.
pub const BEHEMOTH_CLASS: &str = "ArchonBehemoth";
/// How many outers up the Behemoth may be.
pub const MAX_OWNER_STEPS: usize = 2;
/// How often the found sets are held, in milliseconds.
pub const HOLD_INTERVAL_MS: u64 = 250;
/// How often the object array is scanned for sets (a query per object, so seldom): a fresh Behemoth is full for at
/// most this long.
pub const SCAN_INTERVAL_MS: u64 = 3_000;
/// The health a Behemoth is held at: any hit takes it past 0.
pub const HELD_HEALTH: f32 = 1.0;
/// Behemoths whose first hold is logged.
pub const HOLD_LOG_BUDGET: u32 = 16;

/// The health to write over `current`, `None` when it is already at or below the hold (or not a number).
pub fn hold(current: f32) -> Option<f32> {
    (current.is_finite() && current > HELD_HEALTH).then_some(HELD_HEALTH)
}

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

    /// The first time each Behemoth's health is held (budgeted).
    pub fn health_held(behemoth: &str, set: usize, before: f32, max: f32) -> String {
        format!("[TestCheats] one-hit kills: {behemoth} health {before:.1} of {max:.1} held at 1 (set {set:#x})")
    }

    /// A scan found a different number of Behemoth health sets than the last (budgeted).
    pub fn health_sets(count: usize) -> String {
        format!("[TestCheats] one-hit kills: {count} Behemoth health set(s) found")
    }

    /// A held Behemoth found above the hold again: the game restored it, so a hit may not kill (budgeted).
    pub fn health_restored(behemoth: &str, health: f32) -> String {
        format!("[TestCheats] one-hit kills: {behemoth} health back at {health:.1}; held at 1 again")
    }

    /// The scan could not find the class, so no health is held.
    pub const HEALTH_SET_MISSING: &str =
        "[TestCheats] ArchonHealthAttributeSet not found; one-hit kills are off for this process";

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
    fn holds_only_above_one() {
        assert_eq!(hold(52_000.0), Some(1.0));
        assert_eq!(hold(1.5), Some(1.0));
        assert_eq!(hold(1.0), None);
        assert_eq!(hold(0.0), None);
        assert_eq!(hold(f32::NAN), None);
        assert_eq!(hold(f32::INFINITY), None);
    }

    #[test]
    fn the_lines() {
        assert_eq!(
            lines::health_held("mcbeaver_normal_bp_C_1", 0xab, 5200.0, 5200.0),
            "[TestCheats] one-hit kills: mcbeaver_normal_bp_C_1 health 5200.0 of 5200.0 held at 1 (set 0xab)"
        );
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
