//! How slices register on the dispatch points (R0 8). Slices register in Init, before the chains are installed;
//! Init then freezes the lists, and the detours read them without a lock. R0 registers none.

use rustic_isles_runtime_core::names::{Matcher, matches};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};

/// The most handlers one `ProcessEvent` point holds (one match bit each in a name cache entry).
pub const MAX_PE_HANDLERS: usize = 64;

/// A dispatch point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Point {
    /// Client `ProcessEvent`.
    Sp1,
    /// Server `ProcessEvent`.
    Sp2,
    /// Server engine tick.
    Sp3,
}

/// What a before handler asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Go on: the next handler, then the chain.
    Continue,
    /// Stop: skip the rest of the before list, the chain (AGPL wrapper and original included) and the after list.
    Absorb,
}

/// One `ProcessEvent` call as a handler sees it.
pub struct PeCall<'a> {
    /// The object.
    pub object: usize,
    /// The UFunction.
    pub function: usize,
    /// The params block.
    pub params: usize,
    /// The function's name cache entry.
    pub entry: &'a NameEntry,
}

/// A handler on SP-1 or SP-2.
pub struct PeHandler {
    /// The registering slice.
    pub slice: &'static str,
    /// Smaller runs first.
    pub order: i32,
    /// Which UFunctions it runs for; each slice spec states the kind per name (RT-18).
    pub matcher: Matcher,
    /// Before the chain.
    pub before: Option<fn(&PeCall) -> Verdict>,
    /// After the chain.
    pub after: Option<fn(&PeCall)>,
}

/// A handler on SP-3. It cannot absorb: the engine tick always runs (R0 call 15).
pub struct TickHandler {
    /// The registering slice.
    pub slice: &'static str,
    /// Smaller runs first.
    pub order: i32,
    /// Before the engine tick: the engine and the frame's delta.
    pub before: Option<fn(usize, f32)>,
    /// After it.
    pub after: Option<fn(usize, f32)>,
}

struct List<T: 'static> {
    pending: Mutex<Vec<T>>,
    frozen: OnceLock<Vec<T>>,
}

impl<T: 'static> List<T> {
    const fn new() -> Self {
        Self {
            pending: Mutex::new(Vec::new()),
            frozen: OnceLock::new(),
        }
    }

    fn push(&self, handler: T, limit: usize) -> Result<(), &'static str> {
        if self.frozen.get().is_some() {
            return Err("registered after the lists were frozen");
        }
        let mut pending = self.pending.lock().map_err(|_| "handler list poisoned")?;
        if pending.len() >= limit {
            return Err("too many handlers on this point");
        }
        pending.push(handler);
        Ok(())
    }

    fn freeze(&self, order: fn(&T) -> i32) {
        let mut handlers = self
            .pending
            .lock()
            .map(|mut pending| std::mem::take(&mut *pending))
            .unwrap_or_default();
        handlers.sort_by_key(order);
        let _ = self.frozen.set(handlers);
    }

    fn get(&self) -> &[T] {
        self.frozen.get().map_or(&[], Vec::as_slice)
    }
}

static SP1: List<PeHandler> = List::new();
static SP2: List<PeHandler> = List::new();
static SP3: List<TickHandler> = List::new();

/// Registers a `ProcessEvent` handler on SP-1 or SP-2.
pub fn register_pe(point: Point, handler: PeHandler) -> Result<(), &'static str> {
    match point {
        Point::Sp1 => SP1.push(handler, MAX_PE_HANDLERS),
        Point::Sp2 => SP2.push(handler, MAX_PE_HANDLERS),
        Point::Sp3 => Err("SP-3 takes tick handlers"),
    }
}

/// Registers a tick handler on SP-3.
pub fn register_tick(handler: TickHandler) -> Result<(), &'static str> {
    SP3.push(handler, usize::MAX)
}

/// Init: sorts each list by `order` (stable, so ties keep registration order) and publishes it.
pub fn freeze() {
    SP1.freeze(|handler| handler.order);
    SP2.freeze(|handler| handler.order);
    SP3.freeze(|handler| handler.order);
}

/// The frozen `ProcessEvent` handlers of a point (empty before the freeze).
pub fn pe_handlers(point: Point) -> &'static [PeHandler] {
    match point {
        Point::Sp1 => SP1.get(),
        Point::Sp2 => SP2.get(),
        Point::Sp3 => &[],
    }
}

/// The frozen tick handlers.
pub fn tick_handlers() -> &'static [TickHandler] {
    SP3.get()
}

/// A UFunction's identity as read from it: object index, name (comparison index, number) and outer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Identity {
    /// `+0x0C`.
    pub index: i32,
    /// `+0x18`.
    pub name_index: i32,
    /// `+0x1C`.
    pub name_number: u32,
    /// `+0x20`.
    pub outer: usize,
}

/// One UFunction's cache entry on one thread (R0 8.3): rewritten in place, never removed, so a reference held
/// across a nested call stays valid.
pub struct NameEntry {
    function: usize,
    identity: Cell<Option<Identity>>,
    full: RefCell<Option<Option<Rc<str>>>>,
    computed: Cell<u64>,
    matched: Cell<u64>,
    resolve: fn(usize) -> Option<String>,
}

impl NameEntry {
    /// An entry for `function` with `identity` (`None` when the function could not be read), naming it with
    /// `resolve` the first time a handler needs the name.
    pub fn new(function: usize, identity: Option<Identity>, resolve: fn(usize) -> Option<String>) -> Self {
        Self {
            function,
            identity: Cell::new(identity),
            full: RefCell::new(None),
            computed: Cell::new(0),
            matched: Cell::new(0),
            resolve,
        }
    }

    /// Keeps the entry when the function still reads the same; otherwise rebuilds it in place, so a recycled
    /// address never returns a stale name.
    pub fn validate(&self, identity: Option<Identity>) {
        if self.identity.get() != identity {
            self.identity.set(identity);
            *self.full.borrow_mut() = None;
            self.computed.set(0);
            self.matched.set(0);
        }
    }

    /// The full name, built lazily; `None` for a null or unreadable function.
    pub fn full_name(&self) -> Option<Rc<str>> {
        if let Some(full) = self.full.borrow().as_ref() {
            return full.clone();
        }
        let full: Option<Rc<str>> = if self.identity.get().is_some() {
            (self.resolve)(self.function).map(Rc::from)
        } else {
            None
        };
        *self.full.borrow_mut() = Some(full.clone());
        full
    }

    /// Whether handler `slot`'s matcher matches, computed once per entry.
    pub fn matches(&self, slot: usize, matcher: Matcher) -> bool {
        let bit = 1u64.checked_shl(u32::try_from(slot).unwrap_or(u32::MAX)).unwrap_or(0);
        if bit != 0 && self.computed.get() & bit != 0 {
            return self.matched.get() & bit != 0;
        }
        let hit = matches(matcher, self.full_name().as_deref());
        if bit != 0 {
            self.computed.set(self.computed.get() | bit);
            if hit {
                self.matched.set(self.matched.get() | bit);
            }
        }
        hit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustic_isles_runtime_core::names::Kind;

    fn named(_: usize) -> Option<String> {
        Some("Function Engine.PlayerController.ClientRestart".into())
    }

    fn renamed(_: usize) -> Option<String> {
        Some("Function Engine.Actor.ReceiveTick".into())
    }

    const RESTART: Matcher = &[(Kind::FunctionName, "ClientRestart")];

    #[test]
    fn entry_matches_once_and_rebuilds() {
        let identity = Identity {
            index: 5,
            name_index: 7,
            name_number: 0,
            outer: 0x10,
        };
        let entry = NameEntry::new(0x1000, Some(identity), named);
        assert!(entry.matches(0, RESTART));
        assert!(entry.matches(0, &[]), "kept per slot once computed");
        entry.validate(Some(identity));
        assert!(entry.matches(0, &[]));
        entry.validate(Some(Identity { index: 6, ..identity }));
        assert!(!entry.matches(0, &[]), "a recycled address is rebuilt");
        let entry = NameEntry::new(0x1000, None, named);
        assert!(!entry.matches(0, RESTART), "an unreadable function never matches");
        let entry = NameEntry::new(0x1000, Some(identity), renamed);
        assert_eq!(entry.full_name().as_deref(), Some("Function Engine.Actor.ReceiveTick"));
        assert!(!entry.matches(63, RESTART));
        assert!(!entry.matches(64, RESTART), "past the bits: computed every time");
    }
}
