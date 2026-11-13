//! The dispatch chains (R0 8): SP-1 and SP-2 on `UObject::ProcessEvent`, SP-3 on the engine tick. With no slice
//! landed they guard and forward. Each is a MIT wrapper (band `Mit`, order 0) on the broker's chain.

use super::broker::{self, NextCell};
use super::memory::{frame_tick, read_bytes};
use super::{detaching, engine, flags, log};
use crate::handlers::{self, Identity, NameEntry, PeCall, Point, Verdict};
use rustic_isles_runtime_core::offsets::{layout, target};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Calls nested deeper than this are absorbed: a game recursion once overflowed the stack.
pub const MAX_DEPTH: u32 = 32;

type ProcessEventFn = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void);
type TickFn = unsafe extern "C" fn(*mut c_void, f32, u8);

/// A `ProcessEvent` dispatch point: its id, its handlers and its wrapper's next cell.
pub struct PePoint {
    /// `SP-1`, `SP-2`, or a test's id.
    pub id: &'static str,
    /// Whose handlers run.
    pub handlers: Option<Point>,
    /// The server point decides the name mode on its first call.
    pub server: bool,
    /// The wrapper's next.
    pub next: NextCell,
    first: AtomicBool,
    panicked: AtomicBool,
}

impl PePoint {
    /// A point with no calls yet.
    pub const fn new(id: &'static str, handlers: Option<Point>, server: bool) -> Self {
        Self {
            id,
            handlers,
            server,
            next: NextCell::new(),
            first: AtomicBool::new(true),
            panicked: AtomicBool::new(false),
        }
    }
}

/// SP-1, the client's `ProcessEvent`.
pub static SP1: PePoint = PePoint::new("SP-1", Some(Point::Sp1), false);
/// SP-2, the server's `ProcessEvent`.
pub static SP2: PePoint = PePoint::new("SP-2", Some(Point::Sp2), true);
static SP3_NEXT: NextCell = NextCell::new();
static SP3_FIRST: AtomicBool = AtomicBool::new(true);
static SP3_PANICKED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static DEPTH: Cell<u32> = const { Cell::new(0) };
    static TRIPPED: Cell<bool> = const { Cell::new(false) };
    static NAMES: RefCell<HashMap<usize, Rc<NameEntry>>> = RefCell::new(HashMap::new());
}

/// Counts this thread's nesting; the drop unwinds it on every exit path.
struct Depth(u32);

impl Depth {
    fn enter() -> Self {
        Depth(DEPTH.with(|depth| {
            depth.set(depth.get() + 1);
            depth.get()
        }))
    }
}

impl Drop for Depth {
    fn drop(&mut self) {
        DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

fn name_or_null(object: usize) -> String {
    engine::full_name(object).unwrap_or_else(|| "null".to_owned())
}

/// What the function reads as now, for the cache's validation.
fn identity(function: usize) -> Option<Identity> {
    if function == 0 || !super::memory::readable(function, layout::UOBJECT_SIZE) {
        return None;
    }
    let mut bytes = [0u8; layout::UOBJECT_SIZE];
    if !read_bytes(function, &mut bytes) {
        return None;
    }
    let i32_at = |at: usize| i32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let outer = u64::from_le_bytes(
        bytes[layout::UOBJECT_OUTER..layout::UOBJECT_OUTER + 8]
            .try_into()
            .ok()?,
    );
    Some(Identity {
        index: i32_at(layout::UOBJECT_INDEX),
        name_index: i32_at(layout::UOBJECT_NAME),
        name_number: u32::from_le_bytes(
            bytes[layout::UOBJECT_NAME + 4..layout::UOBJECT_NAME + 8]
                .try_into()
                .ok()?,
        ),
        outer: usize::try_from(outer).ok()?,
    })
}

/// The function's entry on this thread; with `PE_FULLNAME_UNCACHED.flag` on a server, a fresh one per call.
fn entry_for(function: usize, server: bool) -> Rc<NameEntry> {
    let identity = identity(function);
    if server && flags::pe_fullname_uncached() {
        return Rc::new(NameEntry::new(function, identity, engine::full_name));
    }
    NAMES.with(|names| {
        let mut names = names.borrow_mut();
        let entry = names
            .entry(function)
            .or_insert_with(|| Rc::new(NameEntry::new(function, identity, engine::full_name)));
        entry.validate(identity);
        Rc::clone(entry)
    })
}

/// The `ProcessEvent` wrapper body (R0 8.1).
pub(crate) fn pe_dispatch(point: &PePoint, object: *mut c_void, function: *mut c_void, params: *mut c_void) {
    let next = point.next.load();
    if next == 0 {
        return;
    }
    // SAFETY: the broker wrote a function of ProcessEvent's signature into this cell
    let next: ProcessEventFn = unsafe { std::mem::transmute::<usize, ProcessEventFn>(next) };
    let call_next = || {
        // SAFETY: the original arguments, to the next wrapper or the original
        unsafe { next(object, function, params) }
    };
    if detaching() {
        call_next();
        return;
    }
    crate::lifecycle::write_status_once();
    let depth = Depth::enter();
    if depth.0 > MAX_DEPTH {
        if !TRIPPED.with(|tripped| tripped.replace(true)) {
            log::line(&format!(
                "[Dispatch] {} depth {}: call absorbed fn={} obj={}",
                point.id,
                depth.0,
                name_or_null(function as usize),
                name_or_null(object as usize)
            ));
        }
        return;
    }
    if point.server && point.first.swap(false, Ordering::SeqCst) {
        let mode = if flags::pe_fullname_uncached() { "off" } else { "on" };
        log::line(&format!(
            "[Dispatch] {} name cache {mode} (PE_FULLNAME_UNCACHED.flag)",
            point.id
        ));
    }
    let called = Cell::new(false);
    let body = catch_unwind(AssertUnwindSafe(|| {
        let handlers = point.handlers.map_or(&[][..], handlers::pe_handlers);
        if handlers.is_empty() {
            called.set(true);
            call_next();
            return;
        }
        let entry = entry_for(function as usize, point.server);
        let call = PeCall {
            object: object as usize,
            function: function as usize,
            params: params as usize,
            entry: &entry,
        };
        for (slot, handler) in handlers.iter().enumerate() {
            if let Some(before) = handler.before
                && entry.matches(slot, handler.matcher)
                && before(&call) == Verdict::Absorb
            {
                return;
            }
        }
        called.set(true);
        call_next();
        for (slot, handler) in handlers.iter().enumerate() {
            if let Some(after) = handler.after
                && entry.matches(slot, handler.matcher)
            {
                after(&call);
            }
        }
    }));
    if body.is_err() {
        if !point.panicked.swap(true, Ordering::SeqCst) {
            log::line(&format!(
                "[Panic] {} handler panicked; the call goes on to the chain",
                point.id
            ));
        }
        if !called.get() {
            call_next();
        }
    }
    drop(depth);
}

unsafe extern "C" fn sp1_detour(object: *mut c_void, function: *mut c_void, params: *mut c_void) {
    pe_dispatch(&SP1, object, function, params);
}

unsafe extern "C" fn sp2_detour(object: *mut c_void, function: *mut c_void, params: *mut c_void) {
    pe_dispatch(&SP2, object, function, params);
}

/// The engine tick wrapper body (R0 8.2). Its handlers cannot absorb.
unsafe extern "C" fn sp3_detour(engine: *mut c_void, delta: f32, can_render: u8) {
    let next = SP3_NEXT.load();
    if next == 0 {
        return;
    }
    // SAFETY: the broker wrote a function of the tick's signature into this cell
    let next: TickFn = unsafe { std::mem::transmute::<usize, TickFn>(next) };
    let call_next = || {
        // SAFETY: the original arguments
        unsafe { next(engine, delta, can_render) }
    };
    if detaching() {
        call_next();
        return;
    }
    crate::lifecycle::write_status_once();
    frame_tick(false);
    if SP3_FIRST.swap(false, Ordering::SeqCst) {
        log::line("[Dispatch] SP-3 first tick");
        let mode = if flags::no_frame_pointer_cache() { "off" } else { "on" };
        log::line(&format!(
            "[Dispatch] SP-3 frame pointer cache {mode} (NO_FRAME_POINTER_CACHE.flag)"
        ));
    }
    let called = Cell::new(false);
    let body = catch_unwind(AssertUnwindSafe(|| {
        let handlers = handlers::tick_handlers();
        for handler in handlers {
            if let Some(before) = handler.before {
                before(engine as usize, delta);
            }
        }
        // Garbage collection runs inside the engine tick: no remembered answer spans it
        frame_tick(true);
        called.set(true);
        call_next();
        frame_tick(true);
        for handler in handlers {
            if let Some(after) = handler.after {
                after(engine as usize, delta);
            }
        }
    }));
    if body.is_err() {
        if !SP3_PANICKED.swap(true, Ordering::SeqCst) {
            log::line("[Panic] SP-3 handler panicked; the engine tick goes on");
        }
        if !called.get() {
            call_next();
        }
    }
}

fn install(id: &str, base: usize, wrapper: usize, next: &'static NextCell) -> Result<(), i32> {
    let target = target(id).ok_or(super::minhook::MH_UNKNOWN)?;
    let address = base.checked_add(usize::try_from(target.rva).map_err(|_| super::minhook::MH_UNKNOWN)?);
    broker::hook(address.ok_or(super::minhook::MH_UNKNOWN)?, wrapper, 0, next)
}

/// Installs SP-1 (client).
pub fn install_sp1(base: usize) -> Result<(), i32> {
    install("SP-1", base, sp1_detour as *const () as usize, &SP1.next)
}

/// Installs SP-2 (server).
pub fn install_sp2(base: usize) -> Result<(), i32> {
    install("SP-2", base, sp2_detour as *const () as usize, &SP2.next)
}

/// Installs SP-3 (server).
pub fn install_sp3(base: usize) -> Result<(), i32> {
    install("SP-3", base, sp3_detour as *const () as usize, &SP3_NEXT)
}
