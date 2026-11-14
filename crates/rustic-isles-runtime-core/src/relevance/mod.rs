//! Slice R13's pure half (spec-runtime-r13): the graph's settings toolkit, RG-2's route decisions and writes, and
//! `SP-3@R13`'s weapon wake, pawn rate write and relevance pass, over `Game`. The detour, the reads and writes of
//! this process and the one `ProcessEvent` call are the DLL's `ffi::relevance`.

mod lines;
mod route;
#[cfg(test)]
mod tests;
mod tick;
mod toolkit;

pub use lines::*;
pub use route::{Branch, WeaponPlan, controller_flags, global_flags, route, weapon_plan, weapon_route};
pub use tick::{Triple, pass, pawn_rates, tick, wake};
pub use toolkit::{
    Settings, apply_settings, clear_connection_cull, connection_info, connection_manager, global_info, map_find,
};

pub use crate::offsets::relevance::PLAYER_CHANNEL_GRACE_FRAMES;

use crate::memory::Memory;
use crate::net::{Admit, LineBudget};
use crate::offsets::relevance::WOKEN_RING;
use crate::reflect;

/// Flags (12): one read, the rest dropped (rulings 2026-11-13).
pub mod flags {
    /// No per-tick pawn rate write (8); R11 reads it for the default object.
    pub use crate::net::flags::DISABLE_PLAYER_REP_BOOST;
    /// Flags the AGPL read for R13 that have no effect here (`PLAYER_ALWAYS_RELEVANT` is R11's,
    /// `DISABLE_OWNER_PAWN_RELEVANCY` R20's).
    pub const DROPPED: [&str; 9] = [
        "DISABLE_PLAYER_ROLE_GLOBAL_ROUTE",
        "DISABLE_WEAPON_DORMANCY_FIX",
        "DISABLE_WEAPON_CULL_FIX",
        "DISABLE_REPGRAPH_RELEVANCE_FIX",
        "DISABLE_SERVER_WEAPON_GAP_LOG",
        "RELEVANCE_PASS_EVERY_TICK",
        "ENABLE_BASE_ACTOR_BOOTSTRAP",
        "ENABLE_PLAYERSTATE_OWNER_ROUTE",
        "ENABLE_HUB_REP_DIAGNOSTICS",
    ];
}

/// SP-3 after order: below R12's drive (500), the AGPL's order (7.1).
pub const TICK_ORDER: i32 = 400;
/// The fewest milliseconds between two searches for a class not found yet (6.1).
pub const CLASS_RETRY_MS: u64 = 1000;

/// The classes R13 tests by pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// `PlayerController`.
    PlayerController,
    /// `PlayerState`.
    PlayerState,
    /// `ArchonPlayerRole`.
    PlayerRole,
    /// `ArchonWeapon`.
    Weapon,
    /// `ArchonCharacter`.
    Character,
    /// `BP_PlayerCharacter_C` (8).
    PlayerCharacter,
}

impl Class {
    /// Every class, in `index` order.
    pub const ALL: [Self; 6] = [
        Self::PlayerController,
        Self::PlayerState,
        Self::PlayerRole,
        Self::Weapon,
        Self::Character,
        Self::PlayerCharacter,
    ];

    /// The short name `find_class` takes.
    pub fn name(self) -> &'static str {
        match self {
            Self::PlayerController => "PlayerController",
            Self::PlayerState => "PlayerState",
            Self::PlayerRole => "ArchonPlayerRole",
            Self::Weapon => "ArchonWeapon",
            Self::Character => "ArchonCharacter",
            Self::PlayerCharacter => "BP_PlayerCharacter_C",
        }
    }

    /// Its slot in a per-class table.
    pub fn index(self) -> usize {
        self as usize
    }
}

/// What the one wake call did (7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeCall {
    /// `SetNetDormancy(DORM_Awake)` called.
    Made,
    /// No `SetNetDormancy` on the weapon's class chain.
    NotFound,
    /// `params_fit` refused the 8-byte block.
    Skipped,
}

impl WakeCall {
    /// The line's `call=` text.
    pub fn text(self) -> &'static str {
        match self {
            Self::Made => "made",
            Self::NotFound => "not found",
            Self::Skipped => "skipped",
        }
    }
}

/// What R13 asks of the game: R0's checked reads, the liveness tests, the found classes, R17's checked write, the
/// wake call, the clock and the log. The DLL answers from this process; the tests from a fake.
pub trait Game {
    /// R0 `readable`.
    fn readable(&self, ptr: usize, size: usize) -> bool;
    /// A checked copy (region rule, no alignment rule).
    fn read(&self, addr: usize, out: &mut [u8]) -> bool;
    /// A checked write of `bytes` at `addr` (readable and writable).
    fn write(&self, addr: usize, bytes: &[u8]) -> bool;
    /// R0 `is_live_object`.
    fn is_live_object(&self, ptr: usize) -> bool;
    /// R0 `is_actor_alive_for_replication`.
    fn is_alive_for_replication(&self, ptr: usize) -> bool;
    /// The class pointer, once found.
    fn class(&self, class: Class) -> Option<usize>;
    /// `GetTickCount64`.
    fn now_ms(&self) -> u64;
    /// An object's short name.
    fn name(&self, object: usize) -> Option<String>;
    /// An object's class's short name.
    fn class_name(&self, object: usize) -> Option<String>;
    /// `SetNetDormancy(DORM_Awake)` on `actor` through `process_event_original` (7.3).
    fn wake(&self, actor: usize) -> WakeCall;
    /// Writes one log line.
    fn log(&self, line: &str);

    /// A pointer at `addr`, 0 when unreadable.
    fn ptr(&self, addr: usize) -> usize {
        let mut bytes = [0u8; 8];
        if !self.read(addr, &mut bytes) {
            return 0;
        }
        usize::try_from(u64::from_le_bytes(bytes)).unwrap_or(0)
    }
    /// A byte.
    fn u8_at(&self, addr: usize) -> Option<u8> {
        let mut bytes = [0u8; 1];
        self.read(addr, &mut bytes).then_some(bytes[0])
    }
    /// A `u16`.
    fn u16_at(&self, addr: usize) -> Option<u16> {
        let mut bytes = [0u8; 2];
        self.read(addr, &mut bytes).then(|| u16::from_le_bytes(bytes))
    }
    /// A `u32`.
    fn u32_at(&self, addr: usize) -> Option<u32> {
        let mut bytes = [0u8; 4];
        self.read(addr, &mut bytes).then(|| u32::from_le_bytes(bytes))
    }
    /// An `i32`.
    fn i32_at(&self, addr: usize) -> Option<i32> {
        let mut bytes = [0u8; 4];
        self.read(addr, &mut bytes).then(|| i32::from_le_bytes(bytes))
    }
    /// An `f32`.
    fn f32_at(&self, addr: usize) -> Option<f32> {
        self.u32_at(addr).map(f32::from_bits)
    }
}

/// R0's `Memory` over `Game`, for the shared reflection walks.
struct Reads<'a>(&'a dyn Game);

impl Memory for Reads<'_> {
    fn readable(&self, ptr: usize, size: usize) -> bool {
        self.0.readable(ptr, size)
    }
    fn read(&self, addr: usize, out: &mut [u8]) -> bool {
        self.0.read(addr, out)
    }
}

/// `object`'s class chain, nearest first (R17's `class_chain`).
fn chain(game: &dyn Game, object: usize) -> Vec<usize> {
    let memory = Reads(game);
    reflect::class_of(&memory, object).map_or_else(Vec::new, |class| reflect::class_chain(&memory, class))
}

/// `is_a` by class pointer; false while the class is not found.
pub fn is_a(game: &dyn Game, object: usize, class: Class) -> bool {
    object != 0
        && game
            .class(class)
            .is_some_and(|target| chain(game, object).contains(&target))
}

/// The first of `classes` that `object` is, walking its chain once (6.1 step 2).
pub fn first_class(game: &dyn Game, object: usize, classes: &[Class]) -> Option<Class> {
    let links = chain(game, object);
    classes
        .iter()
        .copied()
        .find(|&class| game.class(class).is_some_and(|target| links.contains(&target)))
}

/// The pointer at `addr` when it is non-null and readable for `span`, else 0.
fn readable_ptr(game: &dyn Game, addr: usize, span: usize) -> usize {
    let ptr = game.ptr(addr);
    if ptr != 0 && game.readable(ptr, span) { ptr } else { 0 }
}

/// The per-process line budgets (4.3): relaxed atomics.
pub struct Budgets {
    /// 6.2.
    pub controller: LineBudget,
    /// 6.3.
    pub player_state: LineBudget,
    /// 6.4.
    pub role: LineBudget,
    /// 6.5.
    pub weapon: LineBudget,
    /// 6.6.
    pub character: LineBudget,
    /// 7.2 step 8.
    pub cull_cleared: LineBudget,
    /// 7.2 step 9.
    pub woke: LineBudget,
    /// 9.3 step 1.
    pub pawn_grace: LineBudget,
    /// 9.3 step 2.
    pub role_cull: LineBudget,
}

impl Default for Budgets {
    fn default() -> Self {
        Self::new()
    }
}

impl Budgets {
    /// Nothing written yet.
    pub const fn new() -> Self {
        Self {
            controller: LineBudget::new(64),
            player_state: LineBudget::new(12),
            role: LineBudget::new(64),
            weapon: LineBudget::new(32),
            character: LineBudget::new(32),
            cull_cleared: LineBudget::new(32),
            woke: LineBudget::new(32),
            pawn_grace: LineBudget::new(32),
            role_cull: LineBudget::new(32),
        }
    }
}

/// Writes `line` through `budget` (4.3).
pub fn budgeted(game: &dyn Game, budget: &LineBudget, kind: &str, line: impl FnOnce() -> String) {
    match budget.admit() {
        Admit::Line => game.log(&line()),
        Admit::Suppressed => game.log(&suppressed(kind)),
        Admit::Drop => {}
    }
}

/// The woken ring (7.2 step 3): `(weapon, object index)`, overwritten in turn.
#[derive(Debug, Clone, Copy)]
pub struct Ring {
    entries: [(usize, i32); WOKEN_RING],
    next: usize,
}

impl Default for Ring {
    fn default() -> Self {
        Self::new()
    }
}

impl Ring {
    /// Empty.
    pub const fn new() -> Self {
        Self {
            entries: [(0, 0); WOKEN_RING],
            next: 0,
        }
    }

    /// Whether `(weapon, index)` is held.
    pub fn holds(&self, weapon: usize, index: i32) -> bool {
        weapon != 0 && self.entries.contains(&(weapon, index))
    }

    /// Records `(weapon, index)` over the oldest entry.
    pub fn record(&mut self, weapon: usize, index: i32) {
        self.entries[self.next] = (weapon, index);
        self.next = (self.next + 1) % WOKEN_RING;
    }
}

/// R13's game-thread state (4.4): the ring and the clocks of the wake and the pass.
pub struct State {
    /// 7.2 step 3.
    pub ring: Ring,
    /// When the wake last ran.
    pub wake_ms: Option<u64>,
    /// When the pass last looked at its inputs.
    pub pass_checked_ms: Option<u64>,
    /// When the pass last ran.
    pub pass_ran_ms: Option<u64>,
    /// The last inputs: the graph and its triples.
    pub pass_inputs: Option<(usize, Vec<Triple>)>,
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    /// Nothing run yet.
    pub const fn new() -> Self {
        Self {
            ring: Ring::new(),
            wake_ms: None,
            pass_checked_ms: None,
            pass_ran_ms: None,
            pass_inputs: None,
        }
    }
}
