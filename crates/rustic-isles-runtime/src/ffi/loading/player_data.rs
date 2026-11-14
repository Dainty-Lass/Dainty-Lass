//! LD-2 (player data loaded, 5) and LD-3 (a load that failed, 6) on the server.

use super::super::{flags, log};
use super::game::{self, FStringParam, read_i32, read_ptr, read_u8, readable};
use super::restart_gate;
use crate::config;
use rustic_isles_runtime_core::loading::{
    LoaderEntry, MAX_ENTRIES, RetryTable, incomplete, lines, rearm_targets, recoverable, shared_hunt_id,
};
use rustic_isles_runtime_core::offsets::r7;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

static RETRIES: Mutex<RetryTable> = Mutex::new(RetryTable::new());
static HUNT_ID_FUNCTION: AtomicUsize = AtomicUsize::new(0);
static HUNT_ID_MISSING_LOGGED: AtomicBool = AtomicBool::new(false);

fn retries() -> MutexGuard<'static, RetryTable> {
    RETRIES.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How the hunt id backfill ended (5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backfill {
    /// The controller or its hunt system could not be read.
    Unreadable,
    /// `PlayerHuntId` was already set (an in-launch player).
    AlreadySet,
    /// The launch names no hunt id.
    NoHuntId,
    /// `DebugSetPlayerHuntID` is not there.
    NoFunction,
    /// The parameter guard or the call refused.
    NotCalled,
    /// Set.
    Set,
}

/// LD-2's body (5.1); how the backfill ended, when it ran.
pub fn data_load_complete(controller: usize, success: u8, next: &mut dyn FnMut()) -> Option<Backfill> {
    let manager = if readable(controller, r7::PC_SPAN_MANAGER) {
        read_ptr(controller + r7::PC_LOAD_MANAGER).unwrap_or(0)
    } else {
        0
    };
    if retries().clear(manager, controller) > 0 {
        log::line(&lines::completed_after_retry(success != 0));
    }
    // Before the original: it asks PlayerCanRestart, which the gate answers
    restart_gate::mark_loaded(controller);
    next();
    if success == 0 || flags::flag("DISABLE_PLAYER_HUNTID_BACKFILL") {
        return None;
    }
    let launch = config::launch().and_then(|launch| launch.expected_players.as_deref());
    let launch = launch.map(String::from_utf16_lossy);
    Some(backfill(controller, &shared_hunt_id(launch.as_deref())))
}

fn hunt_id_function(hunt_system: usize) -> Option<usize> {
    let cached = HUNT_ID_FUNCTION.load(Ordering::Relaxed);
    if cached != 0 && game::is_live(cached) {
        return Some(cached);
    }
    let found = game::find_function(hunt_system, "DebugSetPlayerHuntID")?;
    HUNT_ID_FUNCTION.store(found, Ordering::Relaxed);
    Some(found)
}

/// 5.2: a late joiner's empty `PlayerHuntId` set to the server's shared hunt id.
pub fn backfill(controller: usize, hunt_id: &str) -> Backfill {
    if !readable(controller, r7::PC_SPAN_HUNT) {
        return Backfill::Unreadable;
    }
    let hunt_system = read_ptr(controller + r7::PC_HUNT_SYSTEM).unwrap_or(0);
    if hunt_system == 0 || !readable(hunt_system, r7::HUNT_SYSTEM_SPAN) {
        return Backfill::Unreadable;
    }
    match read_i32(hunt_system + r7::HUNT_ID_NUM) {
        Some(num) if num >= 2 => return Backfill::AlreadySet,
        Some(_) => {}
        None => return Backfill::Unreadable,
    }
    if hunt_id.is_empty() {
        log::line(lines::HUNT_ID_NONE);
        return Backfill::NoHuntId;
    }
    let Some(function) = hunt_id_function(hunt_system) else {
        if !HUNT_ID_MISSING_LOGGED.swap(true, Ordering::Relaxed) {
            log::line(lines::HUNT_ID_FUNCTION_MISSING);
        }
        return Backfill::NoFunction;
    };
    let string = FStringParam::new(hunt_id);
    let mut block = game::params(r7::SET_HUNT_ID_PARAMS);
    string.put(&mut block);
    // Past every hook, as the AGPL made it; the function copies the string and it replicates to the client
    if !game::process_event_original(hunt_system, function, &mut block) {
        return Backfill::NotCalled;
    }
    log::line(&lines::hunt_id_set(hunt_id));
    Backfill::Set
}

/// The manager's entries, or `None` for a null array, a count outside 1..=128 or an unreadable array (6.1 step 4).
fn entries(manager: usize) -> Option<(usize, usize)> {
    let data = read_ptr(manager + r7::MANAGER_ENTRIES).unwrap_or(0);
    let count = read_i32(manager + r7::MANAGER_ENTRY_COUNT).unwrap_or(0);
    if data == 0 || count <= 0 || count > MAX_ENTRIES {
        return None;
    }
    let count = usize::try_from(count).ok()?;
    readable(data, count * r7::ENTRY_SIZE).then_some((data, count))
}

/// 6.2: each entry's resolved owner, loader and two bytes.
fn snapshot(data: usize, count: usize) -> Vec<LoaderEntry> {
    (0..count)
        .map(|index| {
            let entry = data + index * r7::ENTRY_SIZE;
            let weak = [read_i32(entry).unwrap_or(0), read_i32(entry + 4).unwrap_or(0)];
            let loader = read_ptr(entry + r7::ENTRY_LOADER).unwrap_or(0);
            let readable_loader = readable(loader, r7::LOADER_SPAN);
            let byte = |offset: usize| {
                if readable_loader {
                    read_u8(loader + offset)
                } else {
                    None
                }
            };
            LoaderEntry {
                owner: game::resolve_weak(weak),
                loader,
                loaded: byte(r7::LOADER_LOADED),
                loading: byte(r7::LOADER_LOADING),
            }
        })
        .collect()
}

/// 6.4: clear each unfinished loader's `bLoading` and the manager's `bBegun`, then `Begin` again.
fn rearm(manager: usize, entries: &[LoaderEntry], incomplete: usize) -> Option<usize> {
    let targets = rearm_targets(entries, incomplete)?;
    for &index in &targets {
        if !game::write::<u8>(entries[index].loader + r7::LOADER_LOADING, 0) {
            return None;
        }
    }
    if !game::write::<u8>(manager + r7::MANAGER_BEGUN, 0) || !game::begin(manager) {
        return None;
    }
    Some(targets.len())
}

/// LD-3's body (6.1).
pub fn load_failed(manager: usize, next: &mut dyn FnMut()) {
    failed(manager, || flags::flag("DISABLE_PLAYERDATA_LOAD_RETRY"), next);
}

/// LD-3 with `DISABLE_PLAYERDATA_LOAD_RETRY.flag` read by `retry_disabled`, checked on each call.
pub fn failed(manager: usize, retry_disabled: impl FnOnce() -> bool, next: &mut dyn FnMut()) {
    if !readable(manager, r7::MANAGER_SPAN) {
        return next();
    }
    let controller = read_ptr(manager + r7::MANAGER_CONTROLLER).unwrap_or(0);
    if controller != 0 {
        let state = read_ptr(controller + r7::PC_PLAYER_STATE).unwrap_or(0);
        // RT-12: a hub's phantom player never loads; its failure would reload the hub (10)
        if state != 0 && read_ptr(state + r7::PS_NET_ID) == Some(0) {
            log::line(lines::DROPPED_NO_NET_ID);
            return;
        }
    }
    let Some((data, count)) = entries(manager) else {
        return next();
    };
    let snapshot = snapshot(data, count);
    if retry_disabled() {
        log::line(lines::RETRY_DISABLED);
        return next();
    }
    let incomplete = incomplete(&snapshot);
    let controller_ok = controller != 0 && readable(controller, r7::PC_SPAN_MANAGER) && game::is_live(controller);
    if !recoverable(controller_ok, &snapshot) || incomplete == 0 {
        log::line(&lines::not_recoverable(incomplete));
        return next();
    }
    let Some(retry) = retries().claim(manager, controller, game::now()) else {
        log::line(lines::RETRY_USED);
        return next();
    };
    let Some(rearmed) = rearm(manager, &snapshot, incomplete) else {
        retries().clear(manager, controller);
        log::line(lines::REARM_FAILED);
        return next();
    };
    log::line(&lines::retried(retry, rearmed));
}
