//! The game's engine in this process (R0 6.2): the object array and name pool at the executable's data RVAs,
//! read through checked memory. Names stay off for the process if object 0 is not `Package CoreUObject`.

use super::log;
use super::memory::ProcessMemory;
use rustic_isles_runtime_core::engine::{Engine, NameCache};
use rustic_isles_runtime_core::names::self_check_failed_line;
use rustic_isles_runtime_core::offsets::data;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, OnceLock};

static IMAGE_BASE: AtomicUsize = AtomicUsize::new(0);
static NAMES: LazyLock<NameCache> = LazyLock::new(NameCache::default);
static CHECKED: OnceLock<bool> = OnceLock::new();

/// Init records the game's image base; until then (and in tests) every name is `None`.
pub fn set_image_base(base: usize) {
    IMAGE_BASE.store(base, Ordering::SeqCst);
}

/// The engine over this process's memory, or `None` before Init.
fn engine() -> Option<Engine<'static>> {
    let base = IMAGE_BASE.load(Ordering::SeqCst);
    if base == 0 {
        return None;
    }
    let objects = base.checked_add(usize::try_from(data::GOBJECTS).ok()?)?;
    let names = base.checked_add(usize::try_from(data::GNAMES).ok()?)?;
    Some(Engine::new(&ProcessMemory, objects, names, Some(&NAMES)))
}

/// Whether names may be used: the self-check ran once and passed. The pool is decoded directly; `AppendString`
/// is not called (R0 6.2's optional retry is not taken).
fn names_on() -> bool {
    *CHECKED.get_or_init(|| {
        let Some(engine) = engine() else {
            return false;
        };
        match engine.self_check() {
            Ok(()) => {
                log::line("[MitRuntime] names: decoded from the name pool");
                true
            }
            Err(found) => {
                log::line(&self_check_failed_line(found.as_deref()));
                false
            }
        }
    })
}

/// An object's full name (`Function Engine.PlayerController.ClientRestart`), `None` for null, unreadable, or
/// with names off.
pub fn full_name(object: usize) -> Option<String> {
    if object == 0 || IMAGE_BASE.load(Ordering::SeqCst) == 0 || !names_on() {
        return None;
    }
    engine()?.full_name(object)
}

/// `is_live_object` against the game's object array.
pub fn is_live_object(ptr: usize) -> bool {
    engine().is_some_and(|engine| engine.is_live_object(ptr))
}

/// `is_actor_alive_for_replication`.
pub fn is_actor_alive_for_replication(ptr: usize) -> bool {
    engine().is_some_and(|engine| engine.is_actor_alive_for_replication(ptr))
}

/// The object whose full name is exactly `name` (R5's HL-11): short names are compared first, and the full name is
/// built only for those. `None` with names off.
pub fn find_object_by_full_name(name: &str) -> Option<usize> {
    if IMAGE_BASE.load(Ordering::SeqCst) == 0 || !names_on() {
        return None;
    }
    let engine = engine()?;
    let short = name.rsplit('.').next()?;
    (0..engine.object_count()?)
        .filter_map(|index| engine.object_at(index))
        .find(|&object| {
            engine.object_name(object).as_deref() == Some(short) && engine.full_name(object).as_deref() == Some(name)
        })
}
