//! R13, relevance and weapons (spec-runtime-r13): RG-2, the route at registration, and `SP-3@R13` after the engine
//! tick (the weapon wake, the pawn rate write, the relevance pass). The decisions and texts are `core::relevance`'s.

mod route;
#[cfg(test)]
mod tests;
mod tick;

use super::broker;
use super::exit::Install;
use super::memory::{query, read_bytes_this_frame, readable_region_this_frame, readable_unaligned, verify_entry};
use super::{engine, log, minhook, reflect, win};
use crate::handlers::{self, TickHandler};
use rustic_isles_runtime_core::net::install::PROLOGUE_MISMATCH;
use rustic_isles_runtime_core::offsets::{FoundBy, target};
use rustic_isles_runtime_core::reflect::is_writable_protect;
use rustic_isles_runtime_core::registry;
use rustic_isles_runtime_core::relevance::{Budgets, CLASS_RETRY_MS, Class, Game, State, TICK_ORDER, WakeCall};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

static BUDGETS: Budgets = Budgets::new();
static FOUND: [AtomicUsize; Class::ALL.len()] = [const { AtomicUsize::new(0) }; Class::ALL.len()];
static TRIED_MS: [AtomicU64; Class::ALL.len()] = [const { AtomicU64::new(0) }; Class::ALL.len()];

thread_local! {
    // Game-thread state (4.4): another thread starts from its own empty one
    static STATE: RefCell<State> = const { RefCell::new(State::new()) };
}

/// Runs `body` on this thread's state; a nested borrow (none is expected) gets an empty one.
fn with_state<R>(mut body: impl FnMut(&mut State) -> R) -> R {
    if let Ok(Some(answer)) = STATE.try_with(|cell| cell.try_borrow_mut().ok().map(|mut state| body(&mut state))) {
        return answer;
    }
    body(&mut State::new())
}

/// The one `[Panic]` line of an id (R0 2).
fn panicked(id: &str, flag: &AtomicBool) {
    if !flag.swap(true, Ordering::SeqCst) {
        log::line(&format!("[Panic] {id} body panicked; the call goes on to the game"));
    }
}

/// The class, found once and kept; while missing, searched again at most once a second (6.1).
fn class(class: Class) -> Option<usize> {
    let slot = class.index();
    let found = FOUND[slot].load(Ordering::Relaxed);
    if found != 0 {
        return Some(found);
    }
    let now = win::tick();
    let tried = TRIED_MS[slot].load(Ordering::Relaxed);
    if tried != 0 && now.saturating_sub(tried) < CLASS_RETRY_MS {
        return None;
    }
    TRIED_MS[slot].store(now.max(1), Ordering::Relaxed);
    let found = reflect::find_class(class.name())?;
    FOUND[slot].store(found, Ordering::Relaxed);
    Some(found)
}

/// R17's checked write without its alignment rule: R13 writes single flag bytes and fields at odd offsets.
fn write_bytes(addr: usize, bytes: &[u8]) -> bool {
    if bytes.is_empty()
        || !readable_unaligned(addr, bytes.len())
        || !query(addr).is_some_and(|region| is_writable_protect(region.protect))
    {
        return false;
    }
    // SAFETY: the range was just checked committed, readable and writable, inside one region
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), addr as *mut u8, bytes.len()) };
    true
}

/// This process.
pub struct ProcessGame;

impl Game for ProcessGame {
    fn readable(&self, ptr: usize, size: usize) -> bool {
        // Per actor and connection each tick: by region, one query per region and frame (profile 4)
        readable_region_this_frame(ptr, size)
    }
    fn read(&self, addr: usize, out: &mut [u8]) -> bool {
        read_bytes_this_frame(addr, out)
    }
    fn write(&self, addr: usize, bytes: &[u8]) -> bool {
        write_bytes(addr, bytes)
    }
    fn is_live_object(&self, ptr: usize) -> bool {
        engine::is_live_object(ptr)
    }
    fn is_alive_for_replication(&self, ptr: usize) -> bool {
        engine::is_actor_alive_for_replication(ptr)
    }
    fn class(&self, which: Class) -> Option<usize> {
        class(which)
    }
    fn now_ms(&self) -> u64 {
        win::tick()
    }
    fn name(&self, object: usize) -> Option<String> {
        reflect::engine()?.object_name(object)
    }
    fn class_name(&self, object: usize) -> Option<String> {
        reflect::class_name_of(object)
    }
    fn wake(&self, actor: usize) -> WakeCall {
        tick::wake(actor)
    }
    fn log(&self, line: &str) {
        log::line(line);
    }
}

/// Server Init, before `handlers::freeze()`: the one handler (after, 400).
pub fn register() {
    if !registry::owns(b"SP-3@R13") {
        return;
    }
    let _ = handlers::register_tick(TickHandler {
        slice: "R13",
        order: TICK_ORDER,
        before: None,
        after: Some(tick::tick),
    });
}

/// RG-2's install line, hooked after its first-bytes check (4.2); `None` when R13 does not own it.
pub fn install(base: usize, hooks_off: Option<&'static str>) -> Option<Install> {
    if !registry::owns(b"RG-2") {
        return None;
    }
    let id = "RG-2";
    let Some(FoundBy::RvaAndBytes(prologue)) = target(id).map(|line| line.found_by) else {
        return Some((id, Err(PROLOGUE_MISMATCH)));
    };
    let Some(rva) = target(id).map(|line| line.rva) else {
        return Some((id, Err(PROLOGUE_MISMATCH)));
    };
    if !verify_entry(rva, prologue) {
        return Some((id, Err(PROLOGUE_MISMATCH)));
    }
    if let Some(status) = hooks_off {
        return Some((id, Err(status)));
    }
    let Some(address) = usize::try_from(rva).ok().and_then(|rva| base.checked_add(rva)) else {
        return Some((id, Err(PROLOGUE_MISMATCH)));
    };
    Some((
        id,
        broker::hook(address, route::DETOUR as usize, 0, &route::NEXT).map_err(minhook::status_name),
    ))
}
