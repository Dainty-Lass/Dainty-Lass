//! `SP-3@R11` (7): before the engine tick, keep R9's hub driver bound to the current world and registered in its
//! gameplay level collections, so `SetActiveLevelCollection` never copies a null `UWorld.NetDriver` over it.

use super::{Callee, log, rva_address};
use crate::ffi::listen::gworld;
use crate::ffi::memory::{read, readable};
use crate::ffi::{detaching, reflect};
use crate::listen::{game_net_driver_name, listening, net_driver};
use rustic_isles_runtime_core::net::{
    Collections, ENGINE_CHECKED, FIND_NAMED_MISMATCH, MAX_COLLECTIONS, REGISTER_DRIVER_CHECKED, REGISTER_WORLD_CHECKED,
    RegisterStep, Registration, UPKEEP_DRIVER_CHECKED, UPKEEP_WORLD_CHECKED,
};
use rustic_isles_runtime_core::offsets::net::{
    FIND_NAMED_NET_DRIVER, FIND_NAMED_NET_DRIVER_BYTES, GENGINE, LEVEL_COLLECTION_NET_DRIVER, LEVEL_COLLECTION_SIZE,
    NETDRIVER_NAME, NETDRIVER_SERVER_CONNECTION, NETDRIVER_WORLD, WORLD_LEVEL_COLLECTIONS,
};
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

type FindNamedNetDriverFn = unsafe extern "C" fn(*mut c_void, *mut c_void, u64) -> *mut c_void;

static FIND_NAMED: Callee = Callee::new(FIND_NAMED_NET_DRIVER, FIND_NAMED_NET_DRIVER_BYTES);
static REGISTRATION: Mutex<Registration> = Mutex::new(Registration {
    registered: 0,
    deferred: 0,
    rejected: 0,
});
static PANICKED: AtomicBool = AtomicBool::new(false);

/// The before handler, order -500.
pub fn tick(_engine: usize, _delta: f32) {
    if detaching() {
        return;
    }
    crate::lifecycle::write_status_once();
    if catch_unwind(AssertUnwindSafe(upkeep)).is_err() && !PANICKED.swap(true, Ordering::SeqCst) {
        log::line("[Panic] SP-3@R11 upkeep panicked; the engine tick goes on");
    }
}

/// 7.2.
fn upkeep() {
    if !listening() {
        return;
    }
    let Some(driver) = net_driver() else {
        return;
    };
    if !readable(driver, UPKEEP_DRIVER_CHECKED) {
        return;
    }
    let Some(world) = gworld().filter(|&world| readable(world, UPKEEP_WORLD_CHECKED)) else {
        return;
    };
    register(driver, world);
    let _ = reflect::write::<usize>(driver + NETDRIVER_WORLD, world);
    if let Some((index, number)) = game_net_driver_name() {
        let name = u64::from(index as u32) | (u64::from(number) << 32);
        let _ = reflect::write::<u64>(driver + NETDRIVER_NAME, name);
    }
    let _ = reflect::write::<usize>(driver + NETDRIVER_SERVER_CONNECTION, 0);
}

/// What the world's level collections read as.
fn collections(world: usize) -> Collections {
    let header = world + WORLD_LEVEL_COLLECTIONS;
    let (Some(data), Some(num)) = (read::<usize>(header), read::<i32>(header + 8)) else {
        return Collections::Unavailable;
    };
    let usable = (1..=MAX_COLLECTIONS).contains(&num)
        && usize::try_from(num)
            .ok()
            .is_some_and(|count| readable(data, count * LEVEL_COLLECTION_SIZE));
    if !usable {
        return Collections::Unusable(num);
    }
    let count = usize::try_from(num).unwrap_or(0);
    let types: Option<Vec<u8>> = (0..count)
        .map(|index| reflect::u8_at(data + index * LEVEL_COLLECTION_SIZE))
        .collect();
    types.map_or(Collections::Unusable(num), Collections::Types)
}

/// 7.3.
fn register(driver: usize, world: usize) {
    if !readable(driver, REGISTER_DRIVER_CHECKED) || !readable(world, REGISTER_WORLD_CHECKED) {
        return;
    }
    let Ok(mut state) = REGISTRATION.lock() else {
        return;
    };
    let driver_world = read::<usize>(driver + NETDRIVER_WORLD).unwrap_or(0);
    let found = collections(world);
    match state.check(world, driver_world, &found) {
        RegisterStep::Stop(line) => {
            if let Some(line) = line {
                log::line(&line);
            }
            return;
        }
        RegisterStep::Resolve => {}
    }
    let Collections::Types(types) = found else {
        return;
    };
    let Some(engine) = rva_address(GENGINE)
        .and_then(read::<usize>)
        .filter(|&engine| readable(engine, ENGINE_CHECKED))
    else {
        return;
    };
    let Some(find) = FIND_NAMED.address(|| log::line(FIND_NAMED_MISMATCH)) else {
        return;
    };
    let Some(name) = read::<u64>(driver + NETDRIVER_NAME) else {
        return;
    };
    // SAFETY: the engine function at its checked entry, on the game thread, with the engine, the world and the
    // driver's own name read through checked reads
    let resolved = unsafe {
        let find = std::mem::transmute::<usize, FindNamedNetDriverFn>(find);
        find(engine as *mut c_void, world as *mut c_void, name) as usize
    };
    crate::ffi::memory::frame_tick(true);
    let (chosen, line) = state.resolved(world, driver, resolved, &types);
    let data = read::<usize>(world + WORLD_LEVEL_COLLECTIONS).unwrap_or(0);
    for index in chosen {
        let _ = reflect::write::<usize>(
            data + index * LEVEL_COLLECTION_SIZE + LEVEL_COLLECTION_NET_DRIVER,
            driver,
        );
    }
    if let Some(line) = line {
        log::line(&line);
    }
}
