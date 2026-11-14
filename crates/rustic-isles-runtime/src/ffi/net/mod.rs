//! R11, net driver safety (spec-runtime-r11): NETD-1..7, the hub driver's upkeep before the engine tick
//! (`SP-3@R11`) and the manual net tick after it. The decisions and texts are `core::net`'s.

mod close;
mod dispatch;
mod manual_tick;
mod netmode;
mod repdriver;
mod sanitize;
mod spline;
#[cfg(test)]
mod tests;
mod upkeep;

pub use repdriver::{graph_class, graph_self_constructed};
pub use sanitize::sanitize;

use super::broker::{self, NextCell};
use super::exit::Install;
use super::memory::{query, readable_unaligned, verify_entry};
use super::{log, minhook, win};
use crate::handlers::{self, TickHandler};
use rustic_isles_runtime_core::net::{LineBudget, MANUAL_TICK_ORDER, UPKEEP_ORDER, install as reason};
use rustic_isles_runtime_core::offsets::{FoundBy, target};
use rustic_isles_runtime_core::reflect::is_writable_protect;
use rustic_isles_runtime_core::registry;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// The address of an RVA in the game executable.
fn rva_address(rva: u32) -> Option<usize> {
    usize::try_from(rva)
        .ok()
        .and_then(|rva| win::image_base().checked_add(rva))
}

/// A checked write of a field at its natural alignment. The reflection kit's `write` wants 8-byte alignment, which
/// the 4-byte fields here (`Num`, class flags, the gate) do not all have.
fn store<T: Copy>(addr: usize, value: T) -> bool {
    let size = std::mem::size_of::<T>();
    if addr == 0
        || !addr.is_multiple_of(std::mem::align_of::<T>())
        || !readable_unaligned(addr, size)
        || !query(addr).is_some_and(|region| is_writable_protect(region.protect))
    {
        return false;
    }
    // SAFETY: committed, readable, writable, aligned for T; game memory written on the game thread
    unsafe { std::ptr::write_volatile(addr as *mut T, value) };
    true
}

/// A budgeted line (4.3).
fn budgeted(budget: &LineBudget, kind: &str, line: impl FnOnce() -> String) {
    if let Some(text) = budget.take(kind, line) {
        log::line(&text);
    }
}

/// The one `[Panic]` line of an id (R0 2).
fn panicked(id: &str, flag: &AtomicBool) {
    if !flag.swap(true, Ordering::SeqCst) {
        log::line(&format!("[Panic] {id} body panicked; the call goes on to the game"));
    }
}

/// A function R11 calls but does not hook, checked against its first bytes before the first call (4.2).
struct Callee {
    rva: u32,
    bytes: &'static [u8],
    /// 0 unchecked, 1 passed, 2 failed.
    state: AtomicU8,
}

impl Callee {
    const fn new(rva: u32, bytes: &'static [u8]) -> Self {
        Self {
            rva,
            bytes,
            state: AtomicU8::new(0),
        }
    }

    /// Its address once its check passed; `on_fail` runs once, the first time it fails.
    fn address(&self, on_fail: impl FnOnce()) -> Option<usize> {
        match self.state.load(Ordering::Acquire) {
            1 => rva_address(self.rva),
            2 => None,
            _ => {
                let passed = verify_entry(self.rva, self.bytes);
                if self.state.swap(if passed { 1 } else { 2 }, Ordering::AcqRel) != 2 && !passed {
                    on_fail();
                }
                if passed { rva_address(self.rva) } else { None }
            }
        }
    }
}

/// Hooks `id` at its `TARGETS` line after its first-bytes check (4.2): fail closed on a wrong build.
fn hook(
    id: &'static str,
    base: usize,
    wrapper: usize,
    next: &'static NextCell,
    hooks_off: Option<&'static str>,
) -> Install {
    let Some(line) = target(id) else {
        return (id, Err(reason::PROLOGUE_MISMATCH));
    };
    let FoundBy::RvaAndBytes(prologue) = line.found_by else {
        return (id, Err(reason::PROLOGUE_MISMATCH));
    };
    if !verify_entry(line.rva, prologue) {
        return (id, Err(reason::PROLOGUE_MISMATCH));
    }
    if let Some(status) = hooks_off {
        return (id, Err(status));
    }
    let Some(address) = usize::try_from(line.rva).ok().and_then(|rva| base.checked_add(rva)) else {
        return (id, Err(reason::PROLOGUE_MISMATCH));
    };
    (
        id,
        broker::hook(address, wrapper, 0, next).map_err(minhook::status_name),
    )
}

/// Server Init, before `handlers::freeze()`: the upkeep (before, -500) and the manual net tick (after, 800).
pub fn register() {
    if !registry::owns(b"SP-3@R11") {
        return;
    }
    let _ = handlers::register_tick(TickHandler {
        slice: "R11",
        order: UPKEEP_ORDER,
        before: Some(upkeep::tick),
        after: None,
    });
    let _ = handlers::register_tick(TickHandler {
        slice: "R11",
        order: MANUAL_TICK_ORDER,
        before: None,
        after: Some(manual_tick::tick),
    });
}

/// The install lines in the AGPL's order (4.4): the server's eight, the client's NETD-6.
pub fn install(server: bool, base: usize, hooks_off: Option<&'static str>) -> Vec<Install> {
    close::set_role(server);
    let mut installs: Vec<Install> = Vec::new();
    let owned = |id: &str| registry::owns(id.as_bytes());
    if server {
        if owned("NETD-1") {
            installs.push(dispatch::install(base, hooks_off));
        }
        if owned("NETD-2") {
            installs.push(repdriver::install_set(base, hooks_off));
        }
        if owned("NETD-7") {
            installs.push(spline::install(base, hooks_off));
        }
        if owned("NETD-8") {
            installs.push(("NETD-8", Err(reason::NOT_PORTED)));
        }
        if owned("NETD-3") {
            installs.push(repdriver::install_create(base, hooks_off));
        }
        if owned("NETD-4") {
            installs.push(netmode::install_internal(base, hooks_off));
        }
        if owned("NETD-5") {
            installs.push(netmode::install_world(base, hooks_off));
        }
    }
    if owned("NETD-6") {
        installs.push(close::install(base, hooks_off));
    }
    installs
}
