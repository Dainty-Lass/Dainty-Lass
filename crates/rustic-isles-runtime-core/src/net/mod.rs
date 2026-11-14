//! Slice R11's pure half (spec-runtime-r11): the connection-list sanitiser's decisions, the level-collection
//! choice, the spline curve check, the manual tick's re-entry guard, the line budgets and every `[MitNet]` text.
//! The detours, the calls into game code and the writes are the DLL's `ffi::net`.

mod lines;
mod sanitizer;
#[cfg(test)]
mod tests;

pub use lines::*;
pub use sanitizer::{Plan, Store, plan_sanitize};

use std::cell::Cell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

/// The flags R11 reads (14); `ENABLE_SPLINE_EVAL_HOOK`, `MANUAL_NET_TICK`, `NATIVE_NET_TICK`,
/// `MANUAL_TICK_ZERO_DT`, `MANUAL_TICK_HALF_RATE` and `EMERGENCY_LEGACY_REPLICATION` are dropped (rulings R11).
pub mod flags {
    /// NETD-7 not installed.
    pub const DISABLE_SPLINE_GUARD: &str = "DISABLE_SPLINE_GUARD";
    /// No rate boost on the player character's default object (R13's flag, read here too).
    pub const DISABLE_PLAYER_REP_BOOST: &str = "DISABLE_PLAYER_REP_BOOST";
    /// The always-relevant bit on the player character's default object (R13's flag, read here too).
    pub const PLAYER_ALWAYS_RELEVANT: &str = "PLAYER_ALWAYS_RELEVANT";
    /// Flags the AGPL read that have no effect here.
    pub const DROPPED: [&str; 6] = [
        "ENABLE_SPLINE_EVAL_HOOK",
        "MANUAL_NET_TICK",
        "NATIVE_NET_TICK",
        "MANUAL_TICK_ZERO_DT",
        "MANUAL_TICK_HALF_RATE",
        "EMERGENCY_LEGACY_REPLICATION",
    ];
}

/// SP-3 order of the hub driver's upkeep (before; after R5's -1000).
pub const UPKEEP_ORDER: i32 = -500;
/// SP-3 order of the manual net tick (after; after R9's 700 upkeep, before its 900 listen).
pub const MANUAL_TICK_ORDER: i32 = 800;

/// The sanitiser's bound on either array.
pub const MAX_CONNECTIONS: i32 = 1024;
/// Bytes of a connection checked readable before it is kept.
pub const CONNECTION_CHECKED: usize = 0x138;
/// Bytes of the driver the sanitiser checks first.
pub const DRIVER_CHECKED: usize = 0xA0;
/// One element of the driver's pending-cleanup array.
pub const CLEANUP_ELEMENT: usize = 0x18;
/// Bytes of the driver the upkeep checks (R9's driver span).
pub const UPKEEP_DRIVER_CHECKED: usize = 0x2B0;
/// Bytes of the world the upkeep checks.
pub const UPKEEP_WORLD_CHECKED: usize = 0x40;
/// Bytes of the driver the registration checks.
pub const REGISTER_DRIVER_CHECKED: usize = 0x198;
/// Bytes of the world the registration checks.
pub const REGISTER_WORLD_CHECKED: usize = 0x160;
/// Bytes of the engine checked before `FindNamedNetDriver`.
pub const ENGINE_CHECKED: usize = 0xC48;
/// The most level collections a world is trusted to hold.
pub const MAX_COLLECTIONS: i32 = 8;
/// Bytes of a connection NETD-6 checks.
pub const CLOSE_CHECKED: usize = 0x1D8;
/// Frames NETD-6 captures, and keeps after the trim.
pub const CLOSE_CAPTURE: usize = 24;
/// Frames NETD-6 logs.
pub const CLOSE_KEEP: usize = 12;
/// NETD-6 logs this many closes per process.
pub const CLOSE_LOGGED: u32 = 16;
/// NETD-2 logs this many calls.
pub const SET_REP_LOGGED: u32 = 16;
/// Bytes of the player default object checked.
pub const DEFAULT_OBJECT_CHECKED: usize = 0x110;
/// NETD-3 logs its gate for calls `0..GATE_LOGGED`.
pub const GATE_LOGGED: u32 = 3;
/// NETD-3 logs its other lines for calls `0..CREATE_LOGGED`.
pub const CREATE_LOGGED: u32 = 6;
/// Bytes of the driver NETD-3 checks before the class.
pub const CREATE_DRIVER_CHECKED: usize = 0x200;
/// `StaticConstructObject_Internal`'s parameter block (stock UE 4.26's; the game reads up to `+0x38`).
pub const CONSTRUCT_PARAMS_WORDS: usize = 10;
/// The spline component bytes checked readable (`+0x448` curve, `0x18` of it).
pub const SPLINE_COMPONENT_CHECKED: usize = 0x460;
/// The most curve points trusted.
pub const MAX_CURVE_POINTS: i32 = 10_000;

/// Budgets (4.3).
pub mod budgets {
    /// Per sanitiser site.
    pub const SANITIZE: u32 = 64;
    /// NETD-1's skipped line.
    pub const DISPATCH_SKIPPED: u32 = 64;
    /// NETD-2's "not loaded yet".
    pub const DEFAULT_OBJECT_MISSING: u32 = 4;
    /// NETD-7's unusable-curve line.
    pub const SPLINE: u32 = 8;
}

/// What a budget says about one more line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admit {
    /// Write it.
    Line,
    /// Write the "further lines suppressed" line instead.
    Suppressed,
    /// Write nothing.
    Drop,
}

/// A per-process line budget (4.3): `limit` lines, then one "suppressed" line, then nothing.
pub struct LineBudget {
    limit: u32,
    used: AtomicU32,
}

impl LineBudget {
    /// A fresh budget of `limit` lines.
    pub const fn new(limit: u32) -> Self {
        Self {
            limit,
            used: AtomicU32::new(0),
        }
    }

    /// Counts one more line of this kind.
    pub fn admit(&self) -> Admit {
        // Past the suppressed line the count stops, so it never wraps
        if self.used.load(Ordering::Relaxed) > self.limit {
            return Admit::Drop;
        }
        let used = self.used.fetch_add(1, Ordering::Relaxed);
        match used.cmp(&self.limit) {
            std::cmp::Ordering::Less => Admit::Line,
            std::cmp::Ordering::Equal => Admit::Suppressed,
            std::cmp::Ordering::Greater => Admit::Drop,
        }
    }

    /// The text to write for `line` of `kind`, if any.
    pub fn take(&self, kind: &str, line: impl FnOnce() -> String) -> Option<String> {
        match self.admit() {
            Admit::Line => Some(line()),
            Admit::Suppressed => Some(suppressed(kind)),
            Admit::Drop => None,
        }
    }
}

/// The sanitiser's sites with a budget each; past `SITES` names, the last slot is shared.
pub struct SiteBudgets {
    names: Mutex<Vec<&'static str>>,
    budgets: [LineBudget; SITES],
}

/// Distinct sanitiser sites with their own budget (R11's three and R12's).
pub const SITES: usize = 8;

impl Default for SiteBudgets {
    fn default() -> Self {
        Self::new()
    }
}

impl SiteBudgets {
    /// No site seen yet.
    pub const fn new() -> Self {
        Self {
            names: Mutex::new(Vec::new()),
            budgets: [const { LineBudget::new(budgets::SANITIZE) }; SITES],
        }
    }

    /// The budget of `site`.
    pub fn of(&self, site: &'static str) -> &LineBudget {
        let index = match self.names.lock() {
            Ok(mut names) => match names.iter().position(|name| *name == site) {
                Some(index) => index,
                None if names.len() < SITES => {
                    names.push(site);
                    names.len() - 1
                }
                None => SITES - 1,
            },
            Err(_) => SITES - 1,
        };
        &self.budgets[index.min(SITES - 1)]
    }
}

/// The level-collection registration's remembered worlds (7.3).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Registration {
    /// The world the driver was set on.
    pub registered: usize,
    /// The last world deferred (its line written).
    pub deferred: usize,
    /// The last world refused (its line written).
    pub rejected: usize,
}

/// What the world's level collections read as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Collections {
    /// `Num` could not be read.
    Unavailable,
    /// `Num` read, but out of `1..=8` or its data unreadable.
    Unusable(i32),
    /// The type byte of each collection.
    Types(Vec<u8>),
}

/// The registration's next step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterStep {
    /// Nothing to do this frame; the line, when one is due.
    Stop(Option<String>),
    /// Ask the world's context which driver it resolves by the driver's name.
    Resolve,
}

/// Collection types R11 sets the driver on: dynamic source (0) and static (2), not the duplicated (1).
pub fn gameplay_collection(kind: u8) -> bool {
    kind == 0 || kind == 2
}

impl Registration {
    /// Steps 2 and 3, before the engine is asked.
    pub fn check(&mut self, world: usize, driver_world: usize, collections: &Collections) -> RegisterStep {
        if world == self.registered {
            return RegisterStep::Stop(None);
        }
        let usable = matches!(collections, Collections::Types(types) if !types.is_empty());
        if driver_world != world || !usable {
            if world == self.deferred {
                return RegisterStep::Stop(None);
            }
            self.deferred = world;
            return RegisterStep::Stop(Some(registration_deferred(world, driver_world, collections)));
        }
        RegisterStep::Resolve
    }

    /// Steps 4 and 5: the collections to write (indices) and the line, when the context resolved `driver`.
    pub fn resolved(
        &mut self,
        world: usize,
        driver: usize,
        resolved: usize,
        types: &[u8],
    ) -> (Vec<usize>, Option<String>) {
        if resolved != driver {
            if world == self.rejected {
                return (Vec::new(), None);
            }
            self.rejected = world;
            return (Vec::new(), Some(registration_refused(world, resolved, driver)));
        }
        let chosen: Vec<usize> = types
            .iter()
            .enumerate()
            .filter(|(_, kind)| gameplay_collection(**kind))
            .map(|(index, _)| index)
            .collect();
        self.registered = world;
        let line = registration_done(chosen.len(), world, driver);
        (chosen, Some(line))
    }
}

/// What the spline query reads off a component (13.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Curve {
    /// The component.
    pub component: usize,
    /// The points array's data.
    pub data: usize,
    /// Its `Num`.
    pub num: i32,
}

/// 13.1 step 2: `curve` is `None` for a null or unreadable component. A fresh acceptance replaces `remembered`.
pub fn spline_usable(
    remembered: &mut Option<Curve>,
    curve: Option<Curve>,
    points_readable: impl FnOnce(usize, usize) -> bool,
) -> bool {
    let Some(curve) = curve else {
        return false;
    };
    if curve.data == 0 {
        return false;
    }
    if *remembered == Some(curve) {
        return true;
    }
    let fresh = (1..=MAX_CURVE_POINTS).contains(&curve.num)
        && usize::try_from(curve.num)
            .ok()
            .and_then(|num| num.checked_mul(crate::offsets::net::CURVE_POINT_SIZE))
            .is_some_and(|size| points_readable(curve.data, size));
    if fresh {
        *remembered = Some(curve);
    }
    fresh
}

/// The manual tick's re-entry guard (8.3 step 2): set while the drive runs, cleared on every exit path.
pub struct Reentry<'a>(&'a Cell<bool>);

impl<'a> Reentry<'a> {
    /// `None` when the drive is already running on this thread.
    pub fn enter(flag: &'a Cell<bool>) -> Option<Self> {
        if flag.replace(true) {
            return None;
        }
        Some(Self(flag))
    }
}

impl Drop for Reentry<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

/// NETD-3's class choice (10.1): the game's own graph, else the basic one.
pub fn graph_class(archon: Option<usize>, basic: Option<usize>) -> Option<(usize, &'static str)> {
    archon
        .map(|class| (class, "ArchonReplicationGraph"))
        .or_else(|| basic.map(|class| (class, "BasicReplicationGraph")))
}

/// `StaticConstructObject_Internal`'s parameters: class at `+0x00`, outer at `+0x08`, every other byte zero.
pub fn construct_params(class: usize, outer: usize) -> [u64; CONSTRUCT_PARAMS_WORDS] {
    let mut params = [0u64; CONSTRUCT_PARAMS_WORDS];
    params[0] = class as u64;
    params[1] = outer as u64;
    params
}

/// The player default object's two rates as one aligned word: `NetUpdateFrequency` 60, `MinNetUpdateFrequency` 30.
pub fn boosted_rates() -> u64 {
    u64::from(60.0f32.to_bits()) | (u64::from(30.0f32.to_bits()) << 32)
}

/// The two rates from that word.
pub fn rates_of(word: u64) -> (f32, f32) {
    (f32::from_bits(word as u32), f32::from_bits((word >> 32) as u32))
}
