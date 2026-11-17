//! Slice R14's pure half (spec-runtime-r14): the player role (omnicell). PR-1's validity record and retry, the
//! server's pump (the post-activation refresh and the Tempest repair), RG-3's purge, the ability route, and the
//! client's input activation, role lifecycle replay, movement repair and airship HUD, over `Game`. The detours and
//! every read, write and call into this process are the DLL's `ffi::player_role`.

mod apply;
mod client;
mod hud;
mod lines;
mod server;
mod sheathe;
#[cfg(test)]
mod tests;

pub use apply::{Snapshot, note, on_apply, retry, snapshot};
pub use client::{activate, client_pump, client_restart, input_mode_changed, movement, replay};
pub use hud::{capture as hud_capture, reconcile};
pub use lines::*;
pub use server::{ability, purge, route_remove, server_pump};
pub use sheathe::{Sheathe, UNSHEATHED, lost, sheathe_changed, sheathe_check, sign};

use crate::memory::Memory;
use crate::net::{Admit, LineBudget};
use crate::offsets::r14 as at;
use std::cell::{Cell, RefCell};

/// Flags (14): one kept, the rest dropped (rulings 2026-11-13).
pub mod flags {
    /// Client, read once: no lifecycle replay (11.2 step 3, 11.4 step 4).
    pub const DISABLE_CLIENT_ROLE_LIFECYCLE_REPLAY: &str = "DISABLE_CLIENT_ROLE_LIFECYCLE_REPLAY";
    /// Flags the AGPL read for R14 that have no effect here.
    pub const DROPPED: [&str; 11] = [
        "DISABLE_PLAYER_ROLE_GLOBAL_ROUTE",
        "DISABLE_PLAYER_ROLE_OWNER_ROUTE",
        "DISABLE_PLAYER_ROLE_REMOTE_ROUTE",
        "USE_LEGACY_ROLE_ONREP_REPAIR",
        "ROLE_PASS_SNAPSHOT_OFF",
        "IMMEDIATE_AIRSHIP_HUD_RESTORE",
        "FORCE_CINEMATIC_CLEAR",
        "NO_PLAYER_ROLE_CHANNEL_LOG",
        "NO_PAWN_ROLE_CENSUS",
        "NO_CONTROLLER_SPLIT_LOG",
        "ENABLE_CONTROLLER_WATCHPOINT",
    ];
}

/// Handler orders (4.4).
pub mod order {
    /// SP-1 before `OnAirshipStateUpdate`.
    pub const AIRSHIP_CAPTURE: i32 = 0;
    /// SP-1 before every call: the client pump.
    pub const PUMP: i32 = 50;
    /// SP-1 after `ClientRestart`: after R7's capture (0).
    pub const CLIENT_RESTART: i32 = 10;
    /// SP-1 after `OnInputModeChanged`.
    pub const INPUT_MODE: i32 = 20;
    /// SP-1 after the HUD names.
    pub const HUD: i32 = 30;
    /// SP-1 after `OnSheathedStateChanged` (r14-omnicell-fix 3.2).
    pub const SHEATHE: i32 = 40;
    /// SP-2 before the ability RPCs.
    pub const ABILITY: i32 = 0;
    /// SP-3 after: after R12's drive (500), before R9's upkeep (700).
    pub const SERVER_PUMP: i32 = 600;
}

/// The retry table's size (5.3).
pub const RETRY_ENTRIES: usize = 8;
/// Between two retries of one loadout.
pub const RETRY_INTERVAL_MS: u64 = 1500;
/// Retries of one loadout at most.
pub const RETRY_ATTEMPTS: u32 = 20;
/// The window after which the retry gives up.
pub const RETRY_WINDOW_MS: u64 = 30_000;
/// Between two wait lines of one entry.
pub const RETRY_WAIT_LOG_MS: u64 = 5000;
/// The server pump's period (6).
pub const SERVER_PUMP_MS: u64 = 1000;
/// Roles the refresh watches (7.1).
pub const ROLE_ENTRIES: usize = 256;
/// The refresh's settle and retry (7.2).
pub const REFRESH_SETTLE_MS: u64 = 1500;
/// Pushes per active cycle.
pub const REFRESH_PUSHES: u32 = 3;
/// Roles the Tempest watch holds (8.1).
pub const TEMPEST_ENTRIES: usize = 64;
/// The Tempest repair's settle (8.3).
pub const TEMPEST_SETTLE_MS: u64 = 3000;
/// Between two Tempest repairs.
pub const TEMPEST_RETRY_MS: u64 = 3500;
/// Tempest repairs at most.
pub const TEMPEST_REPAIRS: u32 = 3;
/// Between two Tempest state lines.
pub const TEMPEST_STATE_LOG_MS: u64 = 10_000;
/// The Tempest role's item id.
pub const PR_TEMPEST: &str = "PR_TEMPEST";
/// A modifier count is valid up to this.
pub const COUNT_LIMIT: i32 = 256;
/// The client pump's period (11.2).
pub const CLIENT_PUMP_MS: u64 = 100;
/// Between two "waiting for a stable possession" lines.
pub const STABLE_WAIT_LOG_MS: u64 = 5000;
/// The input activation's timeout from the possession's start.
pub const ACTIVATION_TIMEOUT_MS: u64 = 8000;
/// Between two pump-driven cascades that did not activate (a call: the pump otherwise retries every 100 ms).
pub const ACTIVATION_RETRY_MS: u64 = 1000;
/// Lifecycle replay attempts before a pause (11.5).
pub const LIFECYCLE_ATTEMPTS: u32 = 20;
/// The pause.
pub const LIFECYCLE_PAUSE_MS: u64 = 30_000;
/// Between two attempts.
pub const LIFECYCLE_INTERVAL_MS: u64 = 1500;
/// (pawn, role) pairs remembered as replayed.
pub const REPLAYED_PAIRS: usize = 32;
/// The HUD restore waits this long after player control (12.3).
pub const HUD_CONTROL_MS: u64 = 1000;
/// The HUD restore's timeout after the airship.
pub const HUD_TIMEOUT_MS: u64 = 45_000;
/// Item ids read at most this many UTF-16 units.
pub const ITEM_ID_UNITS: usize = 128;
/// The role check waits this long after an unsheathe (r14-omnicell-fix 3.3).
pub const SHEATHE_SETTLE_MS: u64 = 1000;
/// Sheathe repairs per pawn at most (3.4).
pub const SHEATHE_REPAIRS: u32 = 10;

/// Whether a client handler runs on this thread (r14-omnicell-fix 2): only on the thread `ClientRestart` was seen on.
/// `ProcessEvent` also runs on the engine's worker threads (thread-safe animation updates), which must never call
/// into the game or keep R14 state.
pub fn game_thread_admits(recorded: u32, me: u32) -> bool {
    recorded != 0 && recorded == me
}

/// How a UFunction is called (4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// `process_event_original`: past every handler of both DLLs.
    Original,
    /// R17's `call`: the hooked entry.
    Hooked,
}

/// What a UFunction call did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invoke {
    /// Called; the block holds what it returned.
    Called,
    /// No function of that name on the object's class chain.
    NotFound,
    /// Found but not called (a dead object, the parameter guard, detaching).
    Refused,
}

/// R7's captured possession (3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Captured {
    /// The controller.
    pub pc: usize,
    /// The pawn `ClientRestart` named.
    pub pawn: usize,
    /// When the possession began.
    pub since_ms: u64,
    /// R7's new-possession counter.
    pub generation: u64,
}

/// What R14 asks of the game. The DLL answers from this process; the tests from a fake.
pub trait Game {
    /// R0 `readable`.
    fn readable(&self, ptr: usize, size: usize) -> bool;
    /// A checked copy (region rule, no alignment rule).
    fn read(&self, addr: usize, out: &mut [u8]) -> bool;
    /// A checked write (readable and writable, no alignment rule).
    fn write(&self, addr: usize, bytes: &[u8]) -> bool;
    /// R0 `is_live_object`.
    fn is_live_object(&self, ptr: usize) -> bool;
    /// `is_a(actor, "ArchonPlayerRole")`.
    fn is_role(&self, actor: usize) -> bool;
    /// `GetTickCount64`.
    fn now_ms(&self) -> u64;
    /// Writes one line.
    fn log(&self, line: &str);
    /// A server under `PRODUCTION_LOG_PROFILE.flag`: the *quiet* lines are skipped (4.3).
    fn quiet(&self) -> bool;
    /// An object's short name.
    fn name(&self, object: usize) -> Option<String>;
    /// The game's `(loadout) -> inter`, the loadout checked (5.1 step 1); 0 when not callable.
    fn active_slot_data(&self, loadout: usize) -> usize;
    /// The game's `(inter) -> item` (5.1 step 2); 0 when not callable.
    fn slot_role_item(&self, inter: usize) -> usize;
    /// PR-1: through its hooked entry (`wrapper`, so R14's own wrapper notes it) or `broker::original(PR-1)`.
    fn apply(&self, loadout: usize, wrapper: bool);
    /// The UFunction `function` of `object`'s class chain, with `params` as its block (copied back after).
    fn invoke(&self, object: usize, function: &str, route: Route, params: &mut [u8]) -> Invoke;
    /// Whether `InternalTryActivateAbility`'s first bytes checked (once; a mismatch logs once).
    fn ability_ready(&self) -> bool;
    /// `InternalTryActivateAbility(component, handle, key, &instanced, null, event_data)` (10 step 2).
    fn try_activate(&self, component: usize, handle: u32, key: &mut [u8; 16], event_data: usize) -> bool;
    /// R7's `captured()`.
    fn captured(&self) -> Option<Captured>;
    /// R7's `chain_for(pc, expected_pawn)`.
    fn chain_for(&self, pc: usize, expected_pawn: Option<usize>) -> Option<usize>;
    /// R7's `possession_stable(pc)`.
    fn possession_stable(&self, pc: usize) -> bool;
    /// `client_connections(net_driver())`'s count, `None` without a driver.
    fn connection_count(&self) -> Option<usize>;
    /// `DISABLE_CLIENT_ROLE_LIFECYCLE_REPLAY.flag`, read once.
    fn lifecycle_replay_off(&self) -> bool;

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
    /// An `i32`.
    fn i32_at(&self, addr: usize) -> Option<i32> {
        let mut bytes = [0u8; 4];
        self.read(addr, &mut bytes).then(|| i32::from_le_bytes(bytes))
    }
    /// A `u32`.
    fn u32_at(&self, addr: usize) -> Option<u32> {
        let mut bytes = [0u8; 4];
        self.read(addr, &mut bytes).then(|| u32::from_le_bytes(bytes))
    }
}

/// R0's `Memory` over `Game`, for the shared `FString` read.
struct Reads<'a>(&'a dyn Game);

impl Memory for Reads<'_> {
    fn readable(&self, ptr: usize, size: usize) -> bool {
        self.0.readable(ptr, size)
    }
    fn read(&self, addr: usize, out: &mut [u8]) -> bool {
        self.0.read(addr, out)
    }
}

/// The `FString` at `addr`, empty when unreadable.
fn fstring(game: &dyn Game, addr: usize) -> String {
    crate::reflect::read_fstring(&Reads(game), addr, ITEM_ID_UNITS).unwrap_or_default()
}

/// The pointer at `addr` when non-null and readable for `span`, else 0.
fn readable_ptr(game: &dyn Game, addr: usize, span: usize) -> usize {
    let ptr = game.ptr(addr);
    if ptr != 0 && game.readable(ptr, span) { ptr } else { 0 }
}

/// `role_live(p)` (7.1): not being destroyed.
pub fn role_live(game: &dyn Game, role: usize) -> bool {
    role != 0
        && game.readable(role, at::ACTOR_LIVE_SPAN)
        && game.is_live_object(role)
        && game
            .u8_at(role + at::ACTOR_FLAGS_5C)
            .is_some_and(|flags| flags & at::ACTOR_DESTROYING == 0)
        && game
            .u32_at(role + crate::offsets::layout::UOBJECT_FLAGS)
            .is_some_and(|flags| flags & (at::RF_BEGIN_DESTROYED | at::RF_FINISH_DESTROYED) == 0)
}

/// The four lifecycle bytes all 1 (7.1); the role readable for `0x300`.
pub fn fully_active(game: &dyn Game, role: usize) -> bool {
    game.readable(role, at::ROLE_SPAN)
        && [
            at::ROLE_EQUIPPED,
            at::ROLE_GAMEPLAY_EQUIPPED,
            at::ROLE_ACTIVE_GAMEPLAY,
            at::ROLE_BP_EQUIP_CALLED,
        ]
        .iter()
        .all(|&offset| game.u8_at(role + offset) == Some(1))
}

/// A role's `ItemId`.
fn role_item_id(game: &dyn Game, role: usize) -> String {
    fstring(game, role + at::EQUIPMENT_ITEM_ID)
}

/// The per-process line budgets (4.3).
pub struct Budgets {
    /// 5.2 step 4.
    pub apply: LineBudget,
    /// 10 step 4.
    pub ability: LineBudget,
    /// 11.3 step 3.
    pub input_mode: LineBudget,
    /// 12.3 step 4.
    pub hide: LineBudget,
    /// r14-omnicell-fix 3.2.
    pub sheathe: LineBudget,
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
            apply: LineBudget::new(256),
            ability: LineBudget::new(128),
            input_mode: LineBudget::new(256),
            hide: LineBudget::new(32),
            sheathe: LineBudget::new(256),
        }
    }
}

/// Writes `line` through `budget`, skipped when `quiet` (4.3).
fn budgeted(game: &dyn Game, budget: &LineBudget, kind: &str, quiet: bool, line: impl FnOnce() -> String) {
    if quiet && game.quiet() {
        return;
    }
    match budget.admit() {
        Admit::Line => game.log(&line()),
        Admit::Suppressed => game.log(&suppressed(kind)),
        Admit::Drop => {}
    }
}

/// A *quiet* line (4.3).
fn quiet_line(game: &dyn Game, line: impl FnOnce() -> String) {
    if !game.quiet() {
        game.log(&line());
    }
}

/// One retry table entry (5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryEntry {
    /// The loadout.
    pub loadout: usize,
    /// When it was noted.
    pub first_seen_ms: u64,
    /// When it was last retried (the note's time first).
    pub last_retry_ms: u64,
    /// When its wait line was last written (0 never).
    pub last_wait_log_ms: u64,
    /// Retries made.
    pub retries: u32,
}

/// One role table entry (7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RoleEntry {
    /// The role.
    pub role: usize,
    /// When it was first seen fully active this cycle.
    pub active_since_ms: u64,
    /// When it was last pushed (0 never).
    pub last_push_ms: u64,
    /// Pushes this cycle.
    pub pushes: u32,
    /// Seen fully active this cycle.
    pub was_active: bool,
    /// Three pushes made.
    pub done: bool,
}

impl RoleEntry {
    fn rearm(&mut self) {
        *self = Self {
            role: self.role,
            ..Self::default()
        };
    }
}

/// One Tempest watch entry (8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TempestEntry {
    /// The role.
    pub role: usize,
    /// When it was first seen fully active (0 not yet).
    pub active_since_ms: u64,
    /// When it was last repaired (0 never).
    pub last_repair_ms: u64,
    /// When a state line was last written.
    pub last_state_log_ms: u64,
    /// Repairs made.
    pub repairs: u32,
}

/// The client's per-possession input state (11.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Input {
    /// R7's generation last seen.
    pub generation: Option<u64>,
    /// The cascade ran past `bOnlineDataReady`.
    pub activated: bool,
    /// An `OnInputModeChanged` was deferred to the pump.
    pub pending: bool,
    /// A natural `OnInputModeChanged` of the current pawn was seen.
    pub edge: bool,
    /// The game's last `bGameInputEnabled` (-1 none).
    pub game_input: i8,
    /// When it was seen.
    pub game_input_ms: u64,
    /// When the pump last ran.
    pub pump_ms: Option<u64>,
    /// When the stable-possession wait line was last written.
    pub wait_log_ms: Option<u64>,
    /// When the pump last ran a cascade that did not activate.
    pub attempt_ms: Option<u64>,
}

impl Input {
    /// A fresh possession.
    pub const fn new() -> Self {
        Self {
            generation: None,
            activated: false,
            pending: false,
            edge: false,
            game_input: -1,
            game_input_ms: 0,
            pump_ms: None,
            wait_log_ms: None,
            attempt_ms: None,
        }
    }
}

impl Default for Input {
    fn default() -> Self {
        Self::new()
    }
}

/// The lifecycle replay's attempt state for one (pawn, id) (11.5 step 2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Attempts {
    /// The pawn.
    pub pawn: usize,
    /// The replicated id.
    pub id: String,
    /// Attempts in this batch.
    pub count: u32,
    /// The last attempt.
    pub last_ms: Option<u64>,
    /// When the batch ran out.
    pub exhausted_ms: Option<u64>,
}

/// The lifecycle replay's per-pawn state (11.5).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Lifecycle {
    /// The pawn whose empty id was logged.
    pub empty_logged: usize,
    /// The attempts.
    pub attempts: Option<Attempts>,
    /// (pawn, role) pairs replayed, newest last.
    pub replayed: Vec<(usize, usize)>,
}

impl Lifecycle {
    /// Nothing seen.
    pub const fn new() -> Self {
        Self {
            empty_logged: 0,
            attempts: None,
            replayed: Vec::new(),
        }
    }
}

/// The airship HUD state (12.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Hud {
    /// The captured HUD.
    pub hud: usize,
    /// The captured game state.
    pub game_state: usize,
    /// R14 hid the combat HUD.
    pub hidden_by_us: bool,
    /// When the airship ended (0 not yet).
    pub airship_ended_ms: u64,
}

/// R14's game-thread state (the client's only on the thread of `game_thread_admits`). Each table is borrowed only between calls into the game, which can re-enter R14.
pub struct State {
    /// 5.3.
    pub retry: RefCell<[Option<RetryEntry>; RETRY_ENTRIES]>,
    /// 7.1.
    pub roles: RefCell<Vec<RoleEntry>>,
    /// 8.1.
    pub tempest: RefCell<Vec<TempestEntry>>,
    /// 6: the pump's last run.
    pub server_pump_ms: Cell<Option<u64>>,
    /// 7.3: the connection count last seen (-1 none).
    pub connections: Cell<i64>,
    /// 11.
    pub input: RefCell<Input>,
    /// 11.4: the cascade's nesting.
    pub activation_depth: Cell<u32>,
    /// 11.5.
    pub lifecycle: RefCell<Lifecycle>,
    /// 12.
    pub hud: RefCell<Hud>,
    /// 12.3 step 1.
    pub hud_busy: Cell<bool>,
    /// r14-omnicell-fix 3.
    pub sheathe: RefCell<Sheathe>,
    /// Once-per-process lines already written.
    pub once: RefCell<Vec<String>>,
    /// 4.3.
    pub budgets: Budgets,
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    /// Nothing seen.
    pub const fn new() -> Self {
        Self {
            retry: RefCell::new([None; RETRY_ENTRIES]),
            roles: RefCell::new(Vec::new()),
            tempest: RefCell::new(Vec::new()),
            server_pump_ms: Cell::new(None),
            connections: Cell::new(-1),
            input: RefCell::new(Input::new()),
            activation_depth: Cell::new(0),
            lifecycle: RefCell::new(Lifecycle::new()),
            hud: RefCell::new(Hud {
                hud: 0,
                game_state: 0,
                hidden_by_us: false,
                airship_ended_ms: 0,
            }),
            hud_busy: Cell::new(false),
            sheathe: RefCell::new(Sheathe {
                pawn: 0,
                baseline: None,
                check_at_ms: None,
                repairs: 0,
            }),
            once: RefCell::new(Vec::new()),
            budgets: Budgets::new(),
        }
    }

    /// Writes `line` the first time `key` is seen in this process.
    fn once(&self, game: &dyn Game, key: &str, line: impl FnOnce() -> String) {
        let first = self.once.try_borrow_mut().is_ok_and(|mut seen| {
            if seen.iter().any(|seen| seen == key) {
                false
            } else {
                seen.push(key.to_owned());
                true
            }
        });
        if first {
            game.log(&line());
        }
    }
}

/// Runs `body` on the cell's value, `None` when it is borrowed (a re-entry R14 does not expect).
fn with<T, R>(cell: &RefCell<T>, body: impl FnOnce(&mut T) -> R) -> Option<R> {
    cell.try_borrow_mut().ok().map(|mut value| body(&mut value))
}

/// Restores a flag or a depth on every exit, a panic included (11.4, 12.3).
struct Restore<'a, T: Copy> {
    cell: &'a Cell<T>,
    value: T,
}

impl<T: Copy> Drop for Restore<'_, T> {
    fn drop(&mut self) {
        self.cell.set(self.value);
    }
}
