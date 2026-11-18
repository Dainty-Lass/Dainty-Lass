//! Simulated players (spec-runtime-sim): the command file's grammar, the spawn and removal plan, the ring and wander
//! positions, the parameter layout check and every log text. The DLL spawns and drives the bots.

use crate::engine::Engine;
use crate::offsets::sim;

/// The flag file that turns bots on (call 1).
pub const FLAG: &str = "RUNTIME_SIM_PLAYERS";
/// The pawn class until a command names another.
pub const DEFAULT_CLASS: &str = "BP_PlayerCharacter_C";
/// The most bots a server keeps.
pub const MAX_BOTS: u32 = 256;
/// Spawns per step (call 4).
pub const SPAWNS_PER_STEP: usize = 4;
/// Removals per step.
pub const REMOVALS_PER_STEP: usize = 8;
/// Failed spawns in a row that lower the target.
pub const FAIL_STREAK: u32 = 8;
/// The step's period.
pub const POLL_MS: u64 = 1000;
/// Wander seconds until a command says otherwise.
pub const DEFAULT_WANDER: u32 = 5;
/// The longest wander period.
pub const MAX_WANDER: u32 = 600;
/// The wander disc's radius, in units.
pub const WANDER_RADIUS: f32 = 1500.0;
/// The longest class name.
pub const MAX_CLASS: usize = 128;
/// The most fields the layout check walks.
pub const MAX_FIELDS: usize = 64;

/// The command file for a port (3).
pub fn file_name(port: u16) -> String {
    format!("rustic_isles_sim_port{port}.txt")
}

/// The start line (4.5).
pub fn on_line(port: u16) -> String {
    format!(
        "[MitRuntime] [Sim] on (RUNTIME_SIM_PLAYERS.flag): reading {} once a second",
        file_name(port)
    )
}

/// One read's commands; `None` where the file gave none of that kind.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Commands {
    /// `bots <n> [<class>]` (`clear` is `bots 0`).
    pub bots: Option<(u32, Option<String>)>,
    /// `wander <seconds>`.
    pub wander: Option<u32>,
    /// Malformed lines.
    pub ignored: usize,
}

fn class_ok(class: &str) -> bool {
    (1..=MAX_CLASS).contains(&class.len()) && class.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// The grammar (3): the last valid command of each kind wins.
pub fn parse(text: &str) -> Commands {
    let mut commands = Commands::default();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        let parsed = match words.as_slice() {
            ["clear"] => Some(Command::Bots(0, None)),
            ["bots", n] => n
                .parse()
                .ok()
                .filter(|&n| n <= MAX_BOTS)
                .map(|n| Command::Bots(n, None)),
            ["bots", n, class] => n
                .parse()
                .ok()
                .filter(|&n| n <= MAX_BOTS && class_ok(class))
                .map(|n| Command::Bots(n, Some((*class).to_owned()))),
            ["wander", s] => s.parse().ok().filter(|&s| s <= MAX_WANDER).map(Command::Wander),
            _ => None,
        };
        match parsed {
            Some(Command::Bots(n, class)) => commands.bots = Some((n, class)),
            Some(Command::Wander(seconds)) => commands.wander = Some(seconds),
            None => commands.ignored += 1,
        }
    }
    commands
}

enum Command {
    Bots(u32, Option<String>),
    Wander(u32),
}

/// The read line (3).
pub fn read_line(file: &str, target: u32, class: &str, wander: u32, ignored: usize) -> String {
    let ignored = if ignored > 0 {
        format!(", {ignored} lines ignored")
    } else {
        String::new()
    };
    format!("[MitRuntime] [Sim] read {file}: target {target} bots of {class}, wander {wander} s{ignored}")
}

/// The lowest free slots, at most `SPAWNS_PER_STEP` and no more than the target needs (4.2).
pub fn slots_to_fill(occupied: &[usize], target: u32) -> Vec<usize> {
    let target = usize::try_from(target).unwrap_or(0);
    let wanted = target.saturating_sub(occupied.len()).min(SPAWNS_PER_STEP);
    (0..).filter(|slot| !occupied.contains(slot)).take(wanted).collect()
}

/// The highest occupied slots to remove, at most `REMOVALS_PER_STEP` (4.2).
pub fn slots_to_remove(occupied: &[usize], target: u32) -> Vec<usize> {
    let target = usize::try_from(target).unwrap_or(0);
    let mut slots = occupied.to_vec();
    slots.sort_unstable_by(|a, b| b.cmp(a));
    slots.truncate(occupied.len().saturating_sub(target).min(REMOVALS_PER_STEP));
    slots
}

/// A count change (4.2).
pub fn count_line(count: usize, target: u32, change: i64) -> String {
    let sign = if change >= 0 { "+" } else { "" };
    format!("[MitRuntime] [Sim] bots {count}/{target} ({sign}{change})")
}

/// The failure streak's line.
pub fn streak_line(target: usize) -> String {
    format!("[MitRuntime] [Sim] {FAIL_STREAK} spawns failed in a row: target lowered to {target}")
}

/// A class that is not loaded.
pub fn class_line(class: &str) -> String {
    format!("[MitRuntime] [Sim] class {class} is not loaded: no bots")
}

/// No origin.
pub const NO_START_LINE: &str = "[MitRuntime] [Sim] no PlayerStart: no bots";

/// Slot `slot`'s place on the ring around `origin` (4.2).
#[allow(clippy::cast_precision_loss, reason = "slots are below 256")]
pub fn ring(origin: [f32; 3], slot: usize) -> [f32; 3] {
    let angle = (slot as f32 * 22.5).to_radians();
    let radius = 300.0 + 150.0 * (slot / 16) as f32;
    [
        origin[0] + radius * angle.cos(),
        origin[1] + radius * angle.sin(),
        origin[2],
    ]
}

/// The wander generator (4.3): a linear-congruential sequence.
#[derive(Debug, Clone, Copy)]
pub struct Wander {
    state: u64,
}

impl Wander {
    /// Seeded with the tick of the first wander.
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn unit(&mut self) -> f32 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        #[allow(clippy::cast_precision_loss, reason = "24 bits")]
        let bits = (self.state >> 40) as f32;
        bits / 16_777_216.0
    }

    /// A goal on the disc around `origin`.
    pub fn goal(&mut self, origin: [f32; 3]) -> [f32; 3] {
        let angle = self.unit() * std::f32::consts::TAU;
        let radius = WANDER_RADIUS * self.unit().sqrt();
        [
            origin[0] + radius * angle.cos(),
            origin[1] + radius * angle.sin(),
            origin[2],
        ]
    }
}

/// A function's parameters as (name, offset), from its `FProperty` chain (2.2); `None` when the chain does not read.
pub fn property_layout(engine: &Engine, function: usize) -> Option<Vec<(String, i32)>> {
    let memory = engine.memory();
    let mut field = memory.ptr_at(function.checked_add(sim::USTRUCT_CHILD_PROPERTIES)?)?;
    let mut found = Vec::new();
    while field != 0 && found.len() < MAX_FIELDS {
        let name = engine.fname_at(field.checked_add(sim::FFIELD_NAME)?)?;
        let offset = memory.i32_at(field.checked_add(sim::FPROPERTY_OFFSET)?)?;
        found.push((name, offset));
        field = memory.ptr_at(field.checked_add(sim::FFIELD_NEXT)?)?;
    }
    Some(found)
}

fn layout_text<'a>(fields: impl Iterator<Item = (&'a str, i64)>) -> String {
    let text: Vec<String> = fields.map(|(name, offset)| format!("{name}@0x{offset:X}")).collect();
    if text.is_empty() {
        "nothing".to_owned()
    } else {
        text.join(", ")
    }
}

/// The layout check (2.2): every expected name at its offset; `Err` is the refusal line.
pub fn check_layout(label: &str, expected: &[(&str, usize)], found: Option<&[(String, i32)]>) -> Result<(), String> {
    let ok = found.is_some_and(|found| {
        expected.iter().all(|&(name, offset)| {
            found
                .iter()
                .any(|(have, at)| have == name && usize::try_from(*at).ok() == Some(offset))
        })
    });
    if ok {
        return Ok(());
    }
    let want = layout_text(
        expected
            .iter()
            .map(|&(name, offset)| (name, i64::try_from(offset).unwrap_or(-1))),
    );
    let have = found.map_or_else(
        || "unreadable".to_owned(),
        |found| layout_text(found.iter().map(|(name, at)| (name.as_str(), i64::from(*at)))),
    );
    Err(format!(
        "[MitRuntime] [Sim] {label} refused: expected {want}; found {have}"
    ))
}

/// 2.1's layouts.
pub mod layouts {
    /// `SpawnAIFromClass`.
    pub const SPAWN: &[(&str, usize)] = &[
        ("WorldContextObject", 0x00),
        ("PawnClass", 0x08),
        ("BehaviorTree", 0x10),
        ("Location", 0x18),
        ("Rotation", 0x24),
        ("bNoCollisionFail", 0x30),
        ("Owner", 0x38),
        ("ReturnValue", 0x40),
    ];
    /// `SpawnAIFromClass`'s block.
    pub const SPAWN_SIZE: usize = 0x48;
    /// `SimpleMoveToLocation`.
    pub const MOVE: &[(&str, usize)] = &[("Controller", 0x00), ("Goal", 0x08)];
    /// Its block.
    pub const MOVE_SIZE: usize = 0x18;
    /// `GetController` and `K2_GetActorLocation`.
    pub const RETURN: &[(&str, usize)] = &[("ReturnValue", 0x00)];
    /// `K2_DestroyActor`.
    pub const NONE: &[(&str, usize)] = &[];
}

#[cfg(test)]
mod tests;
