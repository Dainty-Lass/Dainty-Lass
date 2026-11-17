//! The F2 console, `CON-1` (spec-runtime-r17 6.2): made on the game thread 3 s after the game viewport appears.

use crate::ffi::broker::{self, NextCell};
use crate::ffi::memory::verify_entry;
use crate::ffi::{engine, flags, log, minhook, reflect, win};
use crate::handlers::PeCall;
use rustic_isles_runtime_core::branding::{ConsoleSchedule, console_line, flags as flag_names};
use rustic_isles_runtime_core::offsets::{FoundBy, r17, target};
use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static EXEC_NEXT: NextCell = NextCell::new();
static EXEC_LOGGED: AtomicBool = AtomicBool::new(false);

static DONE: AtomicBool = AtomicBool::new(false);
static SCHEDULE: Mutex<ConsoleSchedule> = Mutex::new(ConsoleSchedule::new());
static ENGINE: AtomicUsize = AtomicUsize::new(0);

pub(super) fn after(_call: &PeCall) {
    if DONE.load(Ordering::Relaxed) {
        return;
    }
    // try_lock: the console's own calls into the game come back through this handler
    let Ok(mut schedule) = SCHEDULE.try_lock() else {
        return;
    };
    let now = win::tick();
    if !schedule.check(now) {
        return;
    }
    let Some(viewport) = game_engine().and_then(|engine| reflect::live_ptr(engine + r17::ENGINE_GAME_VIEWPORT)) else {
        return;
    };
    if !schedule.viewport_seen(now) {
        return;
    }
    schedule.finish();
    DONE.store(true, Ordering::Relaxed);
    drop(schedule);
    make(viewport);
}

/// The game's engine: the first non-default object deriving from `Engine`, kept while live.
fn game_engine() -> Option<usize> {
    let kept = ENGINE.load(Ordering::Relaxed);
    if kept != 0 && engine::is_live_object(kept) {
        return Some(kept);
    }
    let found = reflect::first_instance("Engine")?;
    ENGINE.store(found, Ordering::Relaxed);
    Some(found)
}

fn make(viewport: usize) {
    let key_set = match set_key() {
        Ok(set) => set,
        Err(why) => {
            log::line(&format!("[MitConsole] console key not set: {why}"));
            return;
        }
    };
    let spawned = match console(viewport) {
        Ok(spawned) => spawned,
        Err(why) => {
            log::line(&format!("[MitConsole] no console: {why}"));
            return;
        }
    };
    log::line(&console_line(key_set, spawned));
}

/// Element 0 of `InputSettings.ConsoleKeys` becomes F2 (only its `KeyName`; keys compare by name).
fn set_key() -> Result<bool, &'static str> {
    let settings = reflect::find_class("InputSettings")
        .and_then(reflect::default_object)
        .ok_or("InputSettings default object not found")?;
    let keys = settings + r17::INPUT_CONSOLE_KEYS;
    let data = reflect::ptr(keys).ok_or("ConsoleKeys unreadable")?;
    let count = reflect::bytes_at(keys + 8, 4)
        .and_then(|bytes| bytes.try_into().ok())
        .map(i32::from_le_bytes)
        .ok_or("ConsoleKeys unreadable")?;
    if count < 1 || data == 0 {
        return Ok(false);
    }
    let name = reflect::name_for("F2").ok_or("Conv_StringToName failed")?;
    if !reflect::write(data, u64::from_le_bytes(name)) {
        return Err("ConsoleKeys[0] not writable");
    }
    Ok(true)
}

/// Keeps an existing viewport console; otherwise spawns one of the engine's console class with the viewport as
/// outer. True when spawned.
fn console(viewport: usize) -> Result<bool, &'static str> {
    let slot = viewport + r17::VIEWPORT_CONSOLE;
    if reflect::ptr(slot).ok_or("ViewportConsole unreadable")? != 0 {
        return Ok(false);
    }
    let engine = ENGINE.load(Ordering::Relaxed);
    let class = reflect::ptr(engine + r17::ENGINE_CONSOLE_CLASS)
        .filter(|&class| class != 0)
        .ok_or("the engine has no ConsoleClass")?;
    let mut params = reflect::Params::new(0x18);
    params.put_ptr(0, class);
    params.put_ptr(8, viewport);
    if !reflect::static_call("GameplayStatics", "SpawnObject", &mut params) {
        return Err("GameplayStatics.SpawnObject not called");
    }
    let made = params.get_ptr(0x10);
    if made == 0 {
        return Err("SpawnObject returned null");
    }
    if !reflect::write(slot, made) {
        return Err("ViewportConsole not writable");
    }
    Ok(true)
}

/// Installs CON-2: the Shipping `FExec::Exec` stub answers "not handled" instead of a fatal error. The engine asks
/// every registered command handler in turn, and one of the game's has no `Exec`, so any console command reached it.
pub fn install_exec_guard(base: usize) -> Result<(), &'static str> {
    if flags::flag(flag_names::DISABLE_CONSOLE_EXEC_GUARD) {
        return Err("off (DISABLE_CONSOLE_EXEC_GUARD.flag)");
    }
    let unknown = minhook::status_name(minhook::MH_UNKNOWN);
    let target = target("CON-2").ok_or(unknown)?;
    if let FoundBy::RvaAndBytes(prologue) = target.found_by
        && !verify_entry(target.rva, prologue)
    {
        return Err("prologue mismatch");
    }
    let address = usize::try_from(target.rva)
        .ok()
        .and_then(|rva| base.checked_add(rva))
        .ok_or(unknown)?;
    broker::hook(address, exec_stub_detour as *const () as usize, 0, &EXEC_NEXT).map_err(minhook::status_name)
}

// Never calls on: the original is the fatal error
extern "C" fn exec_stub_detour(this: *mut c_void, _world: *mut c_void, _cmd: *const u16, _ar: *mut c_void) -> u8 {
    if !EXEC_LOGGED.swap(true, Ordering::Relaxed) {
        let vtable = reflect::ptr(this as usize).unwrap_or(0);
        let rva = vtable.saturating_sub(win::image_base());
        log::line(&format!(
            "[MitConsole] a command handler without Exec answered not handled (vtable +0x{rva:X})"
        ));
    }
    0
}
