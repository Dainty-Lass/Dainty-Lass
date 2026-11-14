//! Slice R20 (spec-runtime-r20): the server's `ProcessEvent` behaviours (`SP-2@R20`) and the connected-player tick
//! (`SP-3@R20`). Everything decided over values already read; the reads, writes and calls are the DLL's.

use crate::loading::ptr;
use crate::names::{Kind, Matcher};

/// The flags R20 reads (11).
pub mod flags {
    /// Present: no view target write at `ClientRestart`; the pawn is marked always-relevant instead (6.2).
    pub const DISABLE_OWNER_PAWN_RELEVANCY: &str = "DISABLE_OWNER_PAWN_RELEVANCY";
    /// The third switch of the development damage multiplier (9.1).
    pub const DEBUG_DAMAGE_MULTIPLIER: &str = "DEBUG_DAMAGE_MULTIPLIER";
}

/// The variables R20 reads through `env_value`, with their caps (9.1, 10.2).
pub mod vars {
    /// Must be exactly `1` for the multiplier.
    pub const ALLOW_DAMAGE_DEBUG: (&str, usize) = ("RUSTIC_ISLES_ALLOW_DAMAGE_DEBUG", 7);
    /// The multiplier's factor.
    pub const DEBUG_DAMAGE_MULTIPLIER: (&str, usize) = ("RUSTIC_ISLES_DEBUG_DAMAGE_MULTIPLIER", 63);
    /// The bleedout fallback in seconds.
    pub const BLEEDOUT_SECONDS: (&str, usize) = ("RUSTIC_ISLES_BLEEDOUT_SECONDS", 31);
}

/// The handlers' matchers and orders (3.1).
pub mod matchers {
    use super::{DAMAGE_FIELDS, Kind, Matcher};

    /// The damage multiplier's order: before every other SP-2 block.
    pub const DAMAGE_ORDER: i32 = -500;
    /// The SP-3 handler's order: after R8 (600) and R12's drive (500).
    pub const TICK_ORDER: i32 = 650;

    /// The five `ArchonBehemoth` damage events, exact (9.2).
    pub const DAMAGE: Matcher = &[
        (Kind::Exact, DAMAGE_FIELDS[0].0),
        (Kind::Exact, DAMAGE_FIELDS[1].0),
        (Kind::Exact, DAMAGE_FIELDS[2].0),
        (Kind::Exact, DAMAGE_FIELDS[3].0),
        (Kind::Exact, DAMAGE_FIELDS[4].0),
    ];
    /// `K2_PostLogin` on any game mode, Blueprint implementations included (5).
    pub const POST_LOGIN: Matcher = &[(Kind::FunctionName, "K2_PostLogin")];
    /// The restart and its retry (6).
    pub const CLIENT_RESTART: Matcher = &[
        (Kind::Exact, "Function Engine.PlayerController.ClientRestart"),
        (Kind::Exact, "Function Engine.PlayerController.ClientRetryClientRestart"),
    ];
    /// The vendor Blueprint's own function only (7).
    pub const VENDOR: Matcher = &[(Kind::Contains, "vendor_interactive_bp_C.UpdateCameraForAspect")];
    /// `PlayerCanRestart` (8).
    pub const CAN_RESTART: Matcher = &[(Kind::Exact, "Function Engine.GameModeBase.PlayerCanRestart")];
}

/// Each damage event: full name, params size, offset of its `f32` damage field (4.3).
pub const DAMAGE_FIELDS: [(&str, usize, usize); 5] = [
    ("Function Archon.ArchonBehemoth.ReceiveArchonDamage", 0x48, 0x00),
    ("Function Archon.ArchonBehemoth.ReceiveArchonPointDamage", 0x78, 0x00),
    ("Function Archon.ArchonBehemoth.ReceiveArchonRadialDamage", 0x60, 0x00),
    (
        "Function Archon.ArchonBehemoth.ReceivedArchonAbilityDamage",
        0x100,
        0x00,
    ),
    (
        "Function Archon.ArchonBehemoth.ReceivedArchonAbilityDamageOnMonsterPart",
        0x108,
        0x08,
    ),
];

/// The params size and damage offset for a damage event's full name.
pub fn damage_field(full_name: &str) -> Option<(usize, usize)> {
    DAMAGE_FIELDS
        .iter()
        .find(|(name, _, _)| *name == full_name)
        .map(|&(_, size, offset)| (size, offset))
}

// ---- 5: the match start ----

/// The match state a first login starts from.
pub const WAITING_TO_START: &str = "WaitingToStart";

/// 5 step 3.
pub fn waiting_to_start(match_state: Option<&str>) -> bool {
    match_state == Some(WAITING_TO_START)
}

// ---- 8.2: the connection and the world's package ----

/// 8.2 step 1: `NetConnection` when set, else `Player` when its class is a connection and not a local player.
pub fn choose_connection(from_418: usize, from_298: usize, class_298: Option<&str>) -> Option<usize> {
    if from_418 != 0 {
        return Some(from_418);
    }
    let class = class_298?;
    (from_298 != 0 && class.contains("Connection") && !class.contains("local_player")).then_some(from_298)
}

/// The most outers 8.2 step 2 follows.
pub const MAX_OUTER_STEPS: usize = 8;

/// 8.2 step 2: from `start`, follow `outer_of` (`None` when the object is unreadable, `Some(0)` for a null outer)
/// at most 8 steps; the last object reached.
pub fn walk_outers(start: usize, outer_of: impl Fn(usize) -> Option<usize>) -> usize {
    let mut top = start;
    for _ in 0..MAX_OUTER_STEPS {
        match outer_of(top) {
            Some(outer) if outer != 0 => top = outer,
            _ => break,
        }
    }
    top
}

// ---- 6.2: the pawn's flags ----

/// `bAlwaysRelevant` in the actor flag byte at `+0x58`.
pub const ALWAYS_RELEVANT_BIT: u8 = 0x08;
/// `DORM_Never`.
pub const DORM_NEVER: u8 = 0;
/// `ROLE_AutonomousProxy`.
pub const ROLE_AUTONOMOUS_PROXY: u8 = 2;
/// The update frequency floor.
pub const MIN_UPDATE_FREQUENCY: f32 = 60.0;
/// The priority floor.
pub const MIN_PRIORITY: f32 = 3.0;

/// The five pawn fields 6.2 writes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PawnFields {
    /// `+0x58`.
    pub relevancy: u8,
    /// `+0xF1`.
    pub dormancy: u8,
    /// `+0x108`.
    pub update_frequency: f32,
    /// `+0x110`.
    pub priority: f32,
    /// `+0x5F`.
    pub remote_role: u8,
}

/// 6.2 step 2: the values to hold. `bAlwaysRelevant` only without the owner fix (marking both faults the grid).
pub fn pawn_plan(read: PawnFields, owner_fix: bool) -> PawnFields {
    PawnFields {
        relevancy: if owner_fix {
            read.relevancy
        } else {
            read.relevancy | ALWAYS_RELEVANT_BIT
        },
        dormancy: DORM_NEVER,
        update_frequency: if read.update_frequency < MIN_UPDATE_FREQUENCY {
            MIN_UPDATE_FREQUENCY
        } else {
            read.update_frequency
        },
        priority: if read.priority < MIN_PRIORITY {
            MIN_PRIORITY
        } else {
            read.priority
        },
        remote_role: ROLE_AUTONOMOUS_PROXY,
    }
}

/// The `[OwnerPawnRelevancy]` lines the process writes.
pub const OWNER_FIX_LOG_BUDGET: u32 = 8;

// ---- 7: the vendor throttle ----

/// Whether the `n`-th absorb (from 0) logs.
pub fn vendor_logs(n: u64) -> bool {
    n < 3 || n.is_multiple_of(60)
}

// ---- 9.1: the multiplier settings ----

/// A whole trimmed value as an `f32` (call 10).
pub fn parse_f32(value: Option<&str>) -> Option<f32> {
    value?.trim().parse::<f32>().ok()
}

/// The factor when all switches are on and it is not 1.0; `None` is off for the process.
pub fn multiplier(development: bool, allow: Option<&str>, flag: bool, factor: Option<&str>) -> Option<f32> {
    if !development || allow != Some("1") || !flag {
        return None;
    }
    let factor = parse_f32(factor)
        .filter(|factor| factor.is_finite() && *factor > 0.0)
        .unwrap_or(1.0);
    (factor != 1.0).then_some(factor)
}

// ---- 10.2: the bleedout floor ----

/// The fallback when the variable is unset or out of range.
pub const BLEEDOUT_DEFAULT_SECONDS: f32 = 30.0;
/// The variable's exclusive upper bound.
pub const BLEEDOUT_MAX_SECONDS: f32 = 600.0;

/// The fallback: the variable when `0 < v < 600`, else 30.
pub fn bleedout_fallback(value: Option<&str>) -> f32 {
    parse_f32(value)
        .filter(|v| *v > 0.0 && *v < BLEEDOUT_MAX_SECONDS)
        .unwrap_or(BLEEDOUT_DEFAULT_SECONDS)
}

/// Whether the read duration needs the floor (NaN does: it is not above 0).
pub fn needs_floor(duration: f32) -> bool {
    duration.is_nan() || duration <= 0.0
}

/// `ECityExecFilter::Both`.
pub const EXEC_BOTH: u8 = 2;
/// `ERemoteExecFilter::All`.
pub const REMOTE_ALL: u8 = 5;

// ---- 13: the log texts ----

/// The texts (13).
pub mod lines {
    use super::ptr;

    /// `StartMatch` found.
    pub const START_MATCH_RESOLVED: &str = "[NoLocalPlayer] GameMode.StartMatch resolved";
    /// `StartMatch` not found.
    pub const START_MATCH_MISSING: &str =
        "[NoLocalPlayer] GameMode.StartMatch not found; the match starts at the game mode's retry";
    /// The stamina tick on.
    pub const STAMINA_MANUAL: &str = "[Stamina] manual TickStamina each frame for connected players";

    /// The match start's outcome.
    pub fn start_match(game_mode: usize, class: &str, called: bool) -> String {
        if called {
            format!(
                "[NoLocalPlayer] StartMatch at the first login: called gm={} class={class}",
                ptr(game_mode)
            )
        } else {
            format!(
                "[NoLocalPlayer] StartMatch at the first login: skipped gm={}",
                ptr(game_mode)
            )
        }
    }

    /// The post-login line (not in the production log profile).
    pub fn post_login(game_mode: usize, new_player: Option<usize>) -> String {
        let player = new_player.map_or_else(|| "null".to_owned(), ptr);
        format!("[PostLogin] gm={} newPlayer={player}", ptr(game_mode))
    }

    /// An owner fix write.
    pub fn owner_fix(connection: usize, old: usize, pawn: usize) -> String {
        format!(
            "[OwnerPawnRelevancy] conn={} ViewTarget {} -> pawn {}",
            ptr(connection),
            ptr(old),
            ptr(pawn)
        )
    }

    /// A vendor absorb.
    pub fn vendor(slot: u64) -> String {
        format!("[VendorFix] absorbed UpdateCameraForAspect on the server (slot={slot})")
    }

    /// The first forced `PlayerCanRestart`.
    pub fn forced_true(player: usize, prior: u8) -> String {
        format!(
            "[PlayerCanRestart] forced true: player={} priorRet={}",
            ptr(player),
            u8::from(prior != 0)
        )
    }

    /// One multiplied damage event.
    pub fn damage(function: &str, object: &str, before: f32, after: f32, factor: f32) -> String {
        format!(
            "[DebugDamageMultiplier] fn={function} obj={object} before={before:.6} after={after:.6} \
             multiplier={factor:.6}"
        )
    }

    /// The floor not needed.
    pub fn bleedout_ok(duration: f32) -> String {
        format!("[Bleedout] BleedoutDuration={duration}s on the island game mode; no floor needed")
    }

    /// The floor written.
    pub fn bleedout_set(duration: f32, fallback: f32) -> String {
        format!("[Bleedout] BleedoutDuration was {duration}; set to {fallback}s so a downed player's timer starts")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::names::matches;

    #[test]
    fn matchers() {
        use matchers::*;
        let all: [Matcher; 5] = [DAMAGE, POST_LOGIN, CLIENT_RESTART, VENDOR, CAN_RESTART];
        let hits = |name: &str| all.iter().filter(|m| matches(m, Some(name))).count();
        for (name, _, _) in DAMAGE_FIELDS {
            assert!(matches(DAMAGE, Some(name)), "{name}");
            assert_eq!(hits(name), 1, "{name}");
        }
        let part = "Function Archon.ArchonBehemoth.ReceivedArchonAbilityDamageOnMonsterPart";
        assert_eq!(
            DAMAGE.iter().filter(|p| matches(&[**p], Some(part))).count(),
            1,
            "only its own Exact"
        );
        assert!(matches(POST_LOGIN, Some("Function Engine.GameModeBase.K2_PostLogin")));
        assert!(matches(
            POST_LOGIN,
            Some("Function /Game/Prototype/BPGM_Archon_Prototype.BPGM_Archon_Prototype_C.K2_PostLogin")
        ));
        assert!(!matches(
            POST_LOGIN,
            Some("Function Engine.PlayerController.ClientOnPostLogin")
        ));
        assert!(matches(
            CLIENT_RESTART,
            Some("Function Engine.PlayerController.ClientRestart")
        ));
        assert!(matches(
            CLIENT_RESTART,
            Some("Function Engine.PlayerController.ClientRetryClientRestart")
        ));
        assert!(matches(
            CAN_RESTART,
            Some("Function Engine.GameModeBase.PlayerCanRestart")
        ));
        for name in [
            "Function Engine.GameMode.StartMatch",
            "Function BP_PlayerCharacter.BP_PlayerCharacter_C.TickStamina",
            "Function UserWidgetFunctions.UserWidgetFunctions_C.GetCurrentScreenAspectRatio",
        ] {
            assert_eq!(hits(name), 0, "{name}");
        }
        assert!(matches(
            VENDOR,
            Some("Function vendor_interactive_bp.vendor_interactive_bp_C.UpdateCameraForAspect")
        ));
        assert_eq!((DAMAGE_ORDER, TICK_ORDER), (-500, 650));
    }

    #[test]
    fn damage_table() {
        let sizes: Vec<_> = DAMAGE_FIELDS.iter().map(|&(_, size, offset)| (size, offset)).collect();
        assert_eq!(sizes, [(0x48, 0), (0x78, 0), (0x60, 0), (0x100, 0), (0x108, 8)]);
        assert_eq!(damage_field(DAMAGE_FIELDS[4].0), Some((0x108, 8)));
        assert_eq!(damage_field("Function Engine.Actor.ReceiveTick"), None);
    }

    #[test]
    fn match_state() {
        assert!(waiting_to_start(Some("WaitingToStart")));
        assert!(!waiting_to_start(Some("InProgress")));
        assert!(!waiting_to_start(None));
    }

    #[test]
    fn connection_choice() {
        assert_eq!(choose_connection(0x10, 0x20, Some("IpConnection")), Some(0x10));
        assert_eq!(choose_connection(0x10, 0, None), Some(0x10));
        assert_eq!(choose_connection(0, 0x20, Some("IpConnection")), Some(0x20));
        assert_eq!(choose_connection(0, 0x20, Some("bp_archon_local_player_C")), None);
        assert_eq!(choose_connection(0, 0x20, Some("LocalPlayer")), None);
        assert_eq!(choose_connection(0, 0x20, None), None, "class unreadable");
        assert_eq!(choose_connection(0, 0, Some("IpConnection")), None);
        assert_eq!(choose_connection(0, 0, None), None);
    }

    #[test]
    fn outer_walk() {
        // 1 -> 2 -> 3 -> 4 -> null
        let chain = |object: usize| Some(if object < 4 { object + 1 } else { 0 });
        assert_eq!(walk_outers(1, chain), 4, "three outers end at the third");
        let unreadable = |object: usize| (object == 1).then_some(2);
        assert_eq!(
            walk_outers(1, unreadable),
            2,
            "an unreadable object ends the walk on it"
        );
        assert_eq!(walk_outers(1, |_| None), 1, "an unreadable start stays put");
        assert_eq!(walk_outers(1, |object| Some(object + 1)), 9, "ten outers: 8 steps");
        assert_eq!(walk_outers(7, |_| Some(0)), 7, "a null outer stops there");
    }

    #[test]
    fn pawn_writes() {
        let read = PawnFields {
            relevancy: 0x41,
            dormancy: 1,
            update_frequency: 59.9,
            priority: 1.0,
            remote_role: 1,
        };
        let on = pawn_plan(read, true);
        assert_eq!(on.relevancy, 0x41, "owner fix on leaves +0x58 alone");
        assert_eq!(pawn_plan(read, false).relevancy, 0x49, "off sets 0x08, keeps the rest");
        assert_eq!(on.update_frequency, 60.0);
        assert_eq!(on.priority, 3.0);
        assert_eq!(on.dormancy, 0);
        assert_eq!(on.remote_role, 2);
        let high = pawn_plan(
            PawnFields {
                update_frequency: 75.0,
                priority: 4.0,
                ..read
            },
            true,
        );
        assert_eq!((high.update_frequency, high.priority), (75.0, 4.0));
    }

    #[test]
    fn bleedout() {
        assert!(needs_floor(0.0));
        assert!(needs_floor(-1.0));
        assert!(needs_floor(f32::NAN));
        assert!(!needs_floor(45.0));
        for (value, expected) in [
            (None, 30.0),
            (Some("45"), 45.0),
            (Some("0"), 30.0),
            (Some("600"), 30.0),
            (Some("599.5"), 599.5),
            (Some("45s"), 30.0),
            (Some(" 45 "), 45.0),
        ] {
            assert_eq!(bleedout_fallback(value), expected, "{value:?}");
        }
    }

    #[test]
    fn multiplier_settings() {
        let on = |factor| multiplier(true, Some("1"), true, factor);
        assert_eq!(multiplier(false, Some("1"), true, Some("2")), None);
        assert_eq!(multiplier(true, Some("2"), true, Some("2")), None);
        assert_eq!(multiplier(true, None, true, Some("2")), None);
        assert_eq!(multiplier(true, Some("1"), false, Some("2")), None);
        for factor in [None, Some("abc"), Some("0"), Some("-2"), Some("inf"), Some("1")] {
            assert_eq!(on(factor), None, "{factor:?}");
        }
        assert_eq!(on(Some("2.5")), Some(2.5));
        assert_eq!(on(Some("10")), Some(10.0));
    }

    #[test]
    fn vendor_throttle() {
        for n in [0, 1, 2, 60, 120] {
            assert!(vendor_logs(n), "{n}");
        }
        for n in [3, 59, 61] {
            assert!(!vendor_logs(n), "{n}");
        }
    }

    #[test]
    fn stamina() {
        assert_eq!((EXEC_BOTH, REMOTE_ALL), (2, 5));
    }

    #[test]
    fn texts() {
        let p = 0x1234_5678_9abc;
        assert_eq!(START_MATCH_RESOLVED_TEXT, lines::START_MATCH_RESOLVED);
        assert_eq!(
            lines::START_MATCH_MISSING,
            "[NoLocalPlayer] GameMode.StartMatch not found; the match starts at the game mode's retry"
        );
        assert_eq!(
            lines::start_match(p, "BPGM_ArchonTrainingGrounds_C", true),
            "[NoLocalPlayer] StartMatch at the first login: called gm=0x0000123456789abc \
             class=BPGM_ArchonTrainingGrounds_C"
        );
        assert_eq!(
            lines::start_match(p, "x", false),
            "[NoLocalPlayer] StartMatch at the first login: skipped gm=0x0000123456789abc"
        );
        assert_eq!(
            lines::post_login(p, Some(0x10)),
            "[PostLogin] gm=0x0000123456789abc newPlayer=0x0000000000000010"
        );
        assert_eq!(
            lines::post_login(p, None),
            "[PostLogin] gm=0x0000123456789abc newPlayer=null"
        );
        assert_eq!(
            lines::owner_fix(0x10, 0, 0x20),
            "[OwnerPawnRelevancy] conn=0x0000000000000010 ViewTarget 0x0000000000000000 -> pawn 0x0000000000000020"
        );
        assert_eq!(
            lines::vendor(60),
            "[VendorFix] absorbed UpdateCameraForAspect on the server (slot=60)"
        );
        assert_eq!(
            lines::forced_true(p, 0),
            "[PlayerCanRestart] forced true: player=0x0000123456789abc priorRet=0"
        );
        assert_eq!(
            lines::forced_true(p, 7),
            "[PlayerCanRestart] forced true: player=0x0000123456789abc priorRet=1"
        );
        assert_eq!(
            lines::damage(DAMAGE_FIELDS[0].0, "Behemoth_1", 12.5, 125.0, 10.0),
            "[DebugDamageMultiplier] fn=Function Archon.ArchonBehemoth.ReceiveArchonDamage obj=Behemoth_1 \
             before=12.500000 after=125.000000 multiplier=10.000000"
        );
        assert_eq!(
            lines::bleedout_ok(45.0),
            "[Bleedout] BleedoutDuration=45s on the island game mode; no floor needed"
        );
        assert_eq!(
            lines::bleedout_set(0.0, 30.0),
            "[Bleedout] BleedoutDuration was 0; set to 30s so a downed player's timer starts"
        );
        assert_eq!(
            lines::STAMINA_MANUAL,
            "[Stamina] manual TickStamina each frame for connected players"
        );
    }

    const START_MATCH_RESOLVED_TEXT: &str = "[NoLocalPlayer] GameMode.StartMatch resolved";
}
