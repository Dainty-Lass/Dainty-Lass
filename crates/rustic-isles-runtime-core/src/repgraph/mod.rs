//! Slice R12's pure half (spec-runtime-r12): the identity tests, the controller channel quarantine, the four
//! guards' decisions (RG-4..7), the graph drive's checks and every `[MitRepGraph]` text. The detours, the call into
//! the graph and the game reads and writes behind `Game` are the DLL's `ffi::repgraph`.

mod drive;
mod guards;
mod identity;
mod lines;
mod quarantine;
#[cfg(test)]
mod tests;

pub use drive::{Checked, drive_line, drive_target, graph_of};
pub use guards::{PreChannel, Rpc, Verdict, bind, pre_channel, rpc, send};
pub use identity::{ClassMemo, chan, conn, is_controller, live_or_null, pc, remote};
pub use lines::*;
pub use quarantine::{Entry, Quarantine, Queued, Source};

use crate::net::{Admit, LineBudget};
use std::sync::atomic::{AtomicU32, Ordering};

/// The one flag R12 reads (13); the others the AGPL read are dropped (rulings R12, RT-3).
pub mod flags {
    /// RG-7 not installed.
    pub const DISABLE_RPC_UNBOUND_CHANNEL_GUARD: &str = "DISABLE_RPC_UNBOUND_CHANNEL_GUARD";
    /// Flags the AGPL read that have no effect here.
    pub const DROPPED: [&str; 5] = [
        "DISABLE_PC_CHANNEL_GUARD",
        "DISABLE_GRAPH_DRIVE",
        "REPGRAPH_DIAG",
        "SRA_RATE_CAP",
        "DIAG_NATURAL",
    ];
}

/// Install reasons (4.2).
pub mod install {
    /// RG-1 (12.1).
    pub const NOT_PORTED: &str = "not ported";
    /// RG-7 off by its flag.
    pub const FLAG_RPC_GUARD: &str = "flag DISABLE_RPC_UNBOUND_CHANNEL_GUARD";
}

/// SP-3 order of the drive (after; below R11's 800 manual tick, 11.1).
pub const DRIVE_ORDER: i32 = 500;
/// Lines per budget (4.3).
pub const BUDGET: u32 = 64;
/// Entries in the quarantine and in each "logged pair" set.
pub const TABLE: usize = 64;
/// Slots in the class memo (5.2).
pub const MEMO_SLOTS: usize = 1024;
/// The drive logs its first this many calls.
pub const DRIVES_LOGGED: u32 = 6;
/// RG-7 logs its first this many drops, then every `RPC_EVERY`-th.
pub const RPC_LOGGED: u32 = 64;
/// See `RPC_LOGGED`.
pub const RPC_EVERY: u32 = 256;
/// The fewest milliseconds between two "at most once a second" lines.
pub const SECOND_MS: u64 = 1000;
/// Distinct drive slot values remembered (11.2 step 6).
pub const CHECKED_SLOTS: usize = 16;

/// The classes R12 identifies by pointer (5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// `PlayerController`.
    PlayerController,
    /// `NetConnection`.
    NetConnection,
    /// `ActorChannel`.
    ActorChannel,
}

impl Class {
    /// The short name `find_class` takes.
    pub fn name(self) -> &'static str {
        match self {
            Self::PlayerController => "PlayerController",
            Self::NetConnection => "NetConnection",
            Self::ActorChannel => "ActorChannel",
        }
    }
}

/// What R12 asks of the game: R0's checked reads, the liveness test, the found classes, one checked write, the
/// clock and the log. The DLL answers from this process; the tests from a fake.
pub trait Game {
    /// R0 `readable`.
    fn readable(&self, ptr: usize, size: usize) -> bool;
    /// R0 `readable_this_frame`.
    fn readable_this_frame(&self, ptr: usize, size: usize) -> bool;
    /// A checked copy (region rule, no alignment rule).
    fn read(&self, addr: usize, out: &mut [u8]) -> bool;
    /// The checked write of a pointer (R17 3.5).
    fn write_ptr(&self, addr: usize, value: usize) -> bool;
    /// R0 `is_live_object`.
    fn is_live_object(&self, ptr: usize) -> bool;
    /// The class pointer, once found (5.1).
    fn class(&self, class: Class) -> Option<usize>;
    /// `GWorld`, 0 when unreadable (the memo's key, 5.2).
    fn world(&self) -> usize;
    /// `GetTickCount64`.
    fn now_ms(&self) -> u64;
    /// An object's short name.
    fn name(&self, object: usize) -> Option<String>;
    /// An object's class's short name.
    fn class_name(&self, object: usize) -> Option<String>;
    /// `address - image base` when inside the executable.
    fn image_rva(&self, address: usize) -> Option<u32>;
    /// R0 `function_entry(rva)`.
    fn function_entry(&self, rva: u32) -> Option<u32>;
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

    /// A pointer at `addr`, `None` when unreadable.
    fn ptr_checked(&self, addr: usize) -> Option<usize> {
        let mut bytes = [0u8; 8];
        self.read(addr, &mut bytes)
            .then(|| u64::from_le_bytes(bytes))
            .and_then(|value| usize::try_from(value).ok())
    }

    /// An `i32` at `addr`.
    fn i32_at(&self, addr: usize) -> Option<i32> {
        let mut bytes = [0u8; 4];
        self.read(addr, &mut bytes).then(|| i32::from_le_bytes(bytes))
    }

    /// A byte at `addr`.
    fn u8_at(&self, addr: usize) -> Option<u8> {
        let mut bytes = [0u8; 1];
        self.read(addr, &mut bytes).then_some(bytes[0])
    }
}

/// The per-process line budgets and counters (4.3): relaxed atomics.
pub struct Budgets {
    /// Quarantine queued.
    pub queued: LineBudget,
    /// Quarantine stale.
    pub stale: LineBudget,
    /// RG-4's reset.
    pub pre_reset: LineBudget,
    /// RG-4's unproven target.
    pub pre_unproven: LineBudget,
    /// RG-4's blocked, past its pair set.
    pub pre_blocked: LineBudget,
    /// RG-5's quarantined refusal.
    pub bind_quarantined: LineBudget,
    /// RG-5's refusal.
    pub bind_refused: LineBudget,
    /// RG-6's blocked, past its pair set.
    pub send_blocked: LineBudget,
    /// RG-7's drops so far.
    pub rpc_drops: AtomicU32,
    /// Drives logged so far.
    pub drives: AtomicU32,
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
            queued: LineBudget::new(BUDGET),
            stale: LineBudget::new(BUDGET),
            pre_reset: LineBudget::new(BUDGET),
            pre_unproven: LineBudget::new(BUDGET),
            pre_blocked: LineBudget::new(BUDGET),
            bind_quarantined: LineBudget::new(BUDGET),
            bind_refused: LineBudget::new(BUDGET),
            send_blocked: LineBudget::new(BUDGET),
            rpc_drops: AtomicU32::new(0),
            drives: AtomicU32::new(0),
        }
    }

    /// Counts one more RG-7 drop: its number, and whether it is logged (lines 1–64, then every 256th).
    pub fn rpc_drop(&self) -> (u32, bool) {
        let count = self.rpc_drops.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        (count, count <= RPC_LOGGED || count.is_multiple_of(RPC_EVERY))
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

/// What one more `(actor, connection)` pair means for a "once per pair" line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pair {
    /// First time: log it.
    New,
    /// Logged before: nothing.
    Seen,
    /// The set is full and the pair not in it: the budget decides.
    Full,
}

/// A "logged once per pair" set of `TABLE` pairs.
#[derive(Debug, Clone, Copy)]
pub struct PairSet {
    pairs: [(usize, usize); TABLE],
    len: usize,
}

impl Default for PairSet {
    fn default() -> Self {
        Self::new()
    }
}

impl PairSet {
    /// Empty.
    pub const fn new() -> Self {
        Self {
            pairs: [(0, 0); TABLE],
            len: 0,
        }
    }

    /// Records `pair` if there is room.
    pub fn note(&mut self, pair: (usize, usize)) -> Pair {
        if self.pairs[..self.len].contains(&pair) {
            return Pair::Seen;
        }
        if self.len == TABLE {
            return Pair::Full;
        }
        self.pairs[self.len] = pair;
        self.len += 1;
        Pair::New
    }
}

/// R12's single-thread state (4.4): the quarantine, the class memo, the pair sets and the drive's memory.
pub struct State {
    /// The controller channel quarantine (6).
    pub quarantine: Quarantine,
    /// Class pointer → "is a `PlayerController`" (5.2).
    pub memo: ClassMemo,
    /// RG-4's logged pairs.
    pub pre_pairs: PairSet,
    /// RG-6's logged pairs.
    pub send_pairs: PairSet,
    /// When RG-4's unproven-target line last went out.
    pub unproven_ms: Option<u64>,
    /// The drive slot values checked so far (11.2 step 6).
    pub checked: Checked,
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    /// Empty.
    pub const fn new() -> Self {
        Self {
            quarantine: Quarantine::new(),
            memo: ClassMemo::new(),
            pre_pairs: PairSet::new(),
            send_pairs: PairSet::new(),
            unproven_ms: None,
            checked: Checked::new(),
        }
    }
}

/// Whether `last` leaves room for one more "at most once a second" line at `now`; if so, records `now`.
pub fn once_a_second(last: &mut Option<u64>, now: u64) -> bool {
    if last.is_some_and(|last| now.saturating_sub(last) < SECOND_MS) {
        return false;
    }
    *last = Some(now);
    true
}
