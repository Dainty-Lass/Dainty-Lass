//! The loadout slot repair (section 6) on the client game thread, and the local controller it starts from.

use super::{Busy, Class, class, functions, lock, on_game_thread};
use crate::ffi::economy::{has_finished_loading, process_event};
use crate::ffi::memory::{ProcessMemory, read};
use crate::ffi::{engine, flags, log, win};
use rustic_isles_runtime_core::economy::loadout::{self, Checks, Controller, Repair, SLOT_NAMES};
use rustic_isles_runtime_core::economy::text;
use rustic_isles_runtime_core::offsets::data;
use std::sync::atomic::AtomicBool;
use std::sync::{LazyLock, Mutex, OnceLock};

const DISABLE_FLAG: &str = "DISABLE_LOADOUT_SLOT_REPAIR";

static CONTROLLER: Mutex<Controller> = Mutex::new(Controller::new());
static REPAIR: LazyLock<Mutex<Repair>> = LazyLock::new(Mutex::default);
/// The pass dispatches `ProcessEvent`, which re-enters this handler.
static PASSING: AtomicBool = AtomicBool::new(false);
static ENABLED: OnceLock<bool> = OnceLock::new();

/// The old `FItemInstanceIdRepl` the slot notification takes: an empty one, in a zeroed block.
#[repr(C, align(8))]
struct Params([u8; 0x40]);

fn world() -> usize {
    usize::try_from(data::GWORLD)
        .ok()
        .and_then(|rva| win::image_base().checked_add(rva))
        .and_then(read::<usize>)
        .unwrap_or(0)
}

/// `ClientRestart`: the local controller and its world (6.2).
pub fn capture(controller: usize) {
    if controller != 0 {
        let world = world();
        lock(&CONTROLLER).capture(controller, world);
    }
}

/// One repair pass (6.3).
pub fn pass() {
    if !*ENABLED.get_or_init(|| !flags::flag(DISABLE_FLAG)) || !on_game_thread() {
        return;
    }
    let Some(_busy) = Busy::enter(&PASSING) else {
        return;
    };
    if !lock(&REPAIR).due(win::tick()) {
        return;
    }
    let world = world();
    let controller = lock(&CONTROLLER).current(world);
    if controller == 0 {
        return;
    }
    let Some(controller_class) = class(Class::Controller) else {
        return;
    };
    let collected = loadout::collect(
        &ProcessMemory,
        controller,
        &Checks {
            controller_class,
            live: &engine::is_live_object,
            loaded: &has_finished_loading,
        },
    );
    if let Some(line) = lock(&REPAIR).waiting_line(&collected) {
        log::line(&line);
    }
    if collected.slots.is_empty() {
        return;
    }
    lock(&REPAIR).begin(collected.loadout);
    let [on_rep_id, cells, parts, dye, transmog, loadout_changed] = functions();
    let mut any = false;
    for slot in &collected.slots {
        let Some(attempt) = lock(&REPAIR).attempt(slot.slot) else {
            continue;
        };
        if on_rep_id == 0 {
            log::line(text::ON_REP_MISSING);
            break;
        }
        let mut params = Params([0; 0x40]);
        let called = process_event(slot.slot, on_rep_id, std::ptr::addr_of_mut!(params) as usize);
        let resolved = called && loadout::resolved(&ProcessMemory, slot.slot);
        if resolved {
            any = true;
            for function in [cells, parts, dye, transmog] {
                if function != 0 {
                    process_event(slot.slot, function, 0);
                }
            }
        }
        log::line(&text::slot(
            slot.data_index,
            SLOT_NAMES.get(slot.position).copied().unwrap_or("?"),
            slot.crc,
            resolved,
            attempt,
        ));
    }
    if any && loadout_changed != 0 {
        // The menus redraw
        process_event(collected.loadout, loadout_changed, 0);
    }
}
