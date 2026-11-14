//! Runtime slice R7, player data and loading (spec-runtime-r7): the readiness gate (LD-1), the player-data load and
//! its one retry (LD-2, LD-3), the restart data gate (`SP-2@R7`), and the progression configuration query with the
//! resume of parked components (LD-5 to LD-7, `SP-3@R7`). LD-4, LD-8 and LD-9 are owned and never installed (RT-3).

pub mod game;
pub mod player_data;
pub mod progression;
pub mod readiness;
pub mod restart_gate;

#[cfg(test)]
mod tests;

use super::broker::{self, NextCell};
use super::{detaching, log, minhook};
use crate::config;
use crate::handlers::{self, PeHandler, Point, TickHandler};
use rustic_isles_runtime_core::loading::lines;
use rustic_isles_runtime_core::names::Kind;
use rustic_isles_runtime_core::offsets::target;
use rustic_isles_runtime_core::registry::owns;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};

/// One wrapper's `next` cell and its panic latch.
pub struct Hook {
    id: &'static str,
    next: NextCell,
    panicked: AtomicBool,
}

impl Hook {
    const fn new(id: &'static str) -> Self {
        Self {
            id,
            next: NextCell::new(),
            panicked: AtomicBool::new(false),
        }
    }
}

static LD1: Hook = Hook::new("LD-1");
static LD2: Hook = Hook::new("LD-2");
static LD3: Hook = Hook::new("LD-3");
static LD5: Hook = Hook::new("LD-5");
static LD6: Hook = Hook::new("LD-6");
static LD7: Hook = Hook::new("LD-7");

/// The wrapper rule (3.5): detaching calls `next` alone; otherwise the status, then `body` under `catch_unwind`. A
/// panic logs once and returns `next`'s result, calling it only if the body had not.
pub fn run<R: Copy>(hook: &Hook, mut next: impl FnMut() -> R, body: impl FnOnce(&mut dyn FnMut() -> R) -> R) -> R {
    if detaching() {
        return next();
    }
    crate::lifecycle::write_status_once();
    let mut result = None;
    let outcome = {
        let mut tracked = || {
            let value = next();
            result = Some(value);
            value
        };
        catch_unwind(AssertUnwindSafe(|| body(&mut tracked)))
    };
    match outcome {
        Ok(value) => value,
        Err(_) => {
            if !hook.panicked.swap(true, Ordering::SeqCst) {
                log::line(&lines::panicked(hook.id));
            }
            match result {
                Some(value) => value,
                None => next(),
            }
        }
    }
}

type ObjectU8Fn = unsafe extern "C" fn(*mut c_void) -> u8;
type ObjectFn = unsafe extern "C" fn(*mut c_void);
type ObjectFlagFn = unsafe extern "C" fn(*mut c_void, u8);

fn next_of<F: Copy>(hook: &Hook) -> Option<F> {
    let next = hook.next.load();
    // SAFETY: the broker wrote a function of the hooked target's signature, `F`, into this cell
    (next != 0).then(|| unsafe { std::mem::transmute_copy::<usize, F>(&next) })
}

unsafe extern "C" fn has_finished_loading(object: *mut c_void) -> u8 {
    let Some(next) = next_of::<ObjectU8Fn>(&LD1) else {
        return 0;
    };
    // SAFETY: the original argument
    let call = || unsafe { next(object) };
    run(&LD1, call, |next| {
        readiness::has_finished_loading(object as usize, config::is_server(), next)
    })
}

unsafe extern "C" fn on_player_data_load_complete(controller: *mut c_void, success: u8) {
    let Some(next) = next_of::<ObjectFlagFn>(&LD2) else {
        return;
    };
    // SAFETY: the original arguments
    let call = || unsafe { next(controller, success) };
    run(&LD2, call, |next| {
        player_data::data_load_complete(controller as usize, success, next);
    });
}

unsafe extern "C" fn load_failed(manager: *mut c_void) {
    let Some(next) = next_of::<ObjectFn>(&LD3) else {
        return;
    };
    // SAFETY: the original argument
    let call = || unsafe { next(manager) };
    run(&LD3, call, |next| player_data::load_failed(manager as usize, next));
}

unsafe extern "C" fn configuration_loaded(game_instance: *mut c_void) -> u8 {
    let Some(next) = next_of::<ObjectU8Fn>(&LD5) else {
        return 0;
    };
    // SAFETY: the original argument
    let call = || unsafe { next(game_instance) };
    run(&LD5, call, |next| progression::predicate(game_instance as usize, next))
}

unsafe extern "C" fn complete_load_internal(component: *mut c_void) {
    let Some(next) = next_of::<ObjectFn>(&LD6) else {
        return;
    };
    // SAFETY: the original argument
    let call = || unsafe { next(component) };
    run(&LD6, call, |next| progression::complete_load(component as usize, next));
}

unsafe extern "C" fn read_from_backend(interface: *mut c_void) {
    let Some(next) = next_of::<ObjectFn>(&LD7) else {
        return;
    };
    // SAFETY: the original argument
    let call = || unsafe { next(interface) };
    run(&LD7, call, |next| {
        progression::read_from_backend(interface as usize, next)
    });
}

/// Init, before `handlers::freeze()`: the client's possession capture, the server's gate and resume poll (3.5).
pub fn register(server: bool) {
    if server {
        // While R20 does not own its PlayerCanRestart force, the gate turns that block's answer back (7.3)
        if !owns(b"SP-2@R20") {
            let _ = handlers::register_pe(
                Point::Sp2,
                PeHandler {
                    slice: "R7",
                    order: 0,
                    matcher: &[(Kind::Exact, "Function Engine.GameModeBase.PlayerCanRestart")],
                    before: None,
                    after: Some(restart_gate::after),
                },
            );
        }
        let _ = handlers::register_tick(TickHandler {
            slice: "R7",
            order: 100,
            before: None,
            after: Some(progression::poll),
        });
    } else {
        let _ = handlers::register_pe(
            Point::Sp1,
            PeHandler {
                slice: "R7",
                order: 0,
                matcher: &[(Kind::Exact, "Function Engine.PlayerController.ClientRestart")],
                before: None,
                after: Some(readiness::capture),
            },
        );
    }
}

fn hook_one(
    id: &'static str,
    base: usize,
    wrapper: usize,
    hook: &'static Hook,
) -> (&'static str, Result<(), &'static str>) {
    let address = target(id)
        .and_then(|target| usize::try_from(target.rva).ok())
        .and_then(|rva| base.checked_add(rva));
    let result = match address {
        Some(address) => broker::hook(address, wrapper, 0, &hook.next).map_err(minhook::status_name),
        None => Err(minhook::status_name(minhook::MH_UNKNOWN)),
    };
    (id, result)
}

/// Init: R7's install lines, after the dispatch chains (3.5).
pub fn install(base: usize, server: bool) -> Vec<(&'static str, Result<(), &'static str>)> {
    game::set_base(base);
    let skipped = |id: &'static str| (id, Err(lines::SKIPPED_LOGGING_ONLY));
    if server {
        vec![
            hook_one("LD-2", base, on_player_data_load_complete as *const () as usize, &LD2),
            hook_one("LD-1", base, has_finished_loading as *const () as usize, &LD1),
            hook_one("LD-3", base, load_failed as *const () as usize, &LD3),
            hook_one("LD-5", base, configuration_loaded as *const () as usize, &LD5),
            hook_one("LD-6", base, complete_load_internal as *const () as usize, &LD6),
            hook_one("LD-7", base, read_from_backend as *const () as usize, &LD7),
            skipped("LD-4"),
            skipped("LD-8"),
            skipped("LD-9"),
        ]
    } else {
        vec![
            hook_one("LD-1", base, has_finished_loading as *const () as usize, &LD1),
            skipped("LD-4"),
        ]
    }
}

/// The six wrappers and their cells, for the broker test.
#[cfg(test)]
pub(crate) fn wrappers() -> [(usize, &'static NextCell); 6] {
    [
        (has_finished_loading as *const () as usize, &LD1.next),
        (on_player_data_load_complete as *const () as usize, &LD2.next),
        (load_failed as *const () as usize, &LD3.next),
        (configuration_loaded as *const () as usize, &LD5.next),
        (complete_load_internal as *const () as usize, &LD6.next),
        (read_from_backend as *const () as usize, &LD7.next),
    ]
}
