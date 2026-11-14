//! R12, the replication graph and channel guards (spec-runtime-r12): RG-4..7, the quarantine, and the graph drive
//! at SP-3 after the engine tick (`SP-3@R12`). The decisions and texts are `core::repgraph`'s.

mod drive;
mod guards;
mod identity;
#[cfg(test)]
mod tests;

use super::broker::{self, NextCell};
use super::exit::Install;
use super::memory::verify_entry;
use super::{flags, log, minhook};
use crate::handlers::{self, TickHandler};
use identity::ProcessGame;
use rustic_isles_runtime_core::net::install::PROLOGUE_MISMATCH;
use rustic_isles_runtime_core::offsets::{FoundBy, target};
use rustic_isles_runtime_core::registry;
use rustic_isles_runtime_core::repgraph::{self as core, Budgets, DRIVE_ORDER, State, install as reason};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

static BUDGETS: Budgets = Budgets::new();

thread_local! {
    // Single-thread state (4.4): only the game thread is expected; another thread starts from its own empty one
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

/// R13 and R14: the graph of R9's driver after the drive's checks (11.2 steps 2 to 4), on the game thread.
pub fn graph() -> Option<usize> {
    core::graph_of(&ProcessGame, crate::listen::net_driver()?)
}

/// R13 and R14: whether a channel is quarantined (this thread's table).
pub fn is_quarantined(channel: usize) -> bool {
    with_state(|state| state.quarantine.contains(channel))
}

/// R14: `pc(p)`, a live `PlayerController`.
pub fn pc(ptr: usize) -> bool {
    core::pc(&ProcessGame, ptr)
}

/// R14: `conn(p)`, a live `NetConnection`.
pub fn conn(ptr: usize) -> bool {
    core::conn(&ProcessGame, ptr)
}

/// R14: `chan(p)`, a live `ActorChannel`.
pub fn chan(ptr: usize) -> bool {
    core::chan(&ProcessGame, ptr)
}

/// Hooks `id` at its `TARGETS` line after its first-bytes check (4.2).
fn hook(
    id: &'static str,
    base: usize,
    wrapper: usize,
    next: &'static NextCell,
    hooks_off: Option<&'static str>,
) -> Install {
    let Some(line) = target(id) else {
        return (id, Err(PROLOGUE_MISMATCH));
    };
    let FoundBy::RvaAndBytes(prologue) = line.found_by else {
        return (id, Err(PROLOGUE_MISMATCH));
    };
    if !verify_entry(line.rva, prologue) {
        return (id, Err(PROLOGUE_MISMATCH));
    }
    if let Some(status) = hooks_off {
        return (id, Err(status));
    }
    let Some(address) = usize::try_from(line.rva).ok().and_then(|rva| base.checked_add(rva)) else {
        return (id, Err(PROLOGUE_MISMATCH));
    };
    (
        id,
        broker::hook(address, wrapper, 0, next).map_err(minhook::status_name),
    )
}

/// Server Init, before `handlers::freeze()`: the sweep and the drive (after, 500).
pub fn register() {
    if !registry::owns(b"SP-3@R12") {
        return;
    }
    let _ = handlers::register_tick(TickHandler {
        slice: "R12",
        order: DRIVE_ORDER,
        before: None,
        after: Some(drive::tick),
    });
}

/// The server's install lines in the AGPL's order (4.5): RG-1's "not ported", then RG-4..7.
pub fn install(base: usize, hooks_off: Option<&'static str>) -> Vec<Install> {
    identity::set_image(base);
    let owned = |id: &str| registry::owns(id.as_bytes());
    let mut installs: Vec<Install> = Vec::new();
    if owned("RG-1") {
        installs.push(("RG-1", Err(reason::NOT_PORTED)));
    }
    if owned("RG-4") {
        installs.push(guards::install_pre_channel(base, hooks_off));
    }
    if owned("RG-5") {
        installs.push(guards::install_bind(base, hooks_off));
    }
    if owned("RG-6") {
        installs.push(guards::install_send(base, hooks_off));
    }
    if owned("RG-7") {
        let off = flags::flag(core::flags::DISABLE_RPC_UNBOUND_CHANNEL_GUARD);
        installs.push(if off {
            ("RG-7", Err(reason::FLAG_RPC_GUARD))
        } else {
            guards::install_rpc(base, hooks_off)
        });
        log::line(&core::rpc_guard(!off));
    }
    installs
}
