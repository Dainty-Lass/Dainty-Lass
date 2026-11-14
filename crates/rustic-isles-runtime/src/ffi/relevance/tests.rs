//! R13's DLL tests (15.1) on test-allocated memory; the hook against the game and the `ProcessEvent` call are not
//! run.

use super::route::route_with;
use super::tick::call_wake;
use super::write_bytes;
use rustic_isles_runtime_core::offsets::r7::FUNCTION_PARMS_SIZE;
use rustic_isles_runtime_core::relevance::WakeCall;
use std::ffi::c_void;
use std::sync::Mutex;

static SEEN: Mutex<Vec<(usize, usize, usize)>> = Mutex::new(Vec::new());

unsafe extern "C" fn route_next(graph: *mut c_void, info: *mut c_void, global: *mut c_void) {
    if let Ok(mut seen) = SEEN.lock() {
        seen.push((graph as usize, info as usize, global as usize));
    }
}

#[test]
fn rg2_passes_a_dead_actor_through_untouched() {
    // An info whose actor is a block that is no live object (no engine in tests)
    let mut actor = vec![0u64; 0x60];
    actor[0x58 / 8] = 0x88;
    let before = actor.clone();
    let mut info = [actor.as_mut_ptr() as u64, 0, 0];
    let mut global = vec![0u64; 0x110 / 8];
    let graph = 0x1234_5678usize as *mut c_void;
    route_with(
        Some(route_next),
        graph,
        info.as_mut_ptr().cast(),
        global.as_mut_ptr().cast(),
    );
    let seen = SEEN.lock().expect("lock").clone();
    assert_eq!(
        seen,
        [(graph as usize, info.as_ptr() as usize, global.as_ptr() as usize)]
    );
    assert_eq!(actor, before);
    assert!(global.iter().all(|&word| word == 0));
}

#[test]
fn the_wake_refuses_a_function_whose_parameters_do_not_fit() {
    // A test function block whose ParmsSize is 16, more than the 8-byte block
    let mut function = vec![0u64; 0x100 / 8];
    let at = function.as_mut_ptr() as usize + FUNCTION_PARMS_SIZE;
    assert!(write_bytes(at, &16u16.to_le_bytes()));
    let mut weapon = vec![0u64; 0x300 / 8];
    assert_eq!(
        call_wake(weapon.as_mut_ptr() as usize, function.as_ptr() as usize),
        WakeCall::Skipped
    );
}

#[test]
fn unaligned_checked_writes() {
    let mut words = vec![0u64; 2];
    let base = words.as_mut_ptr() as usize;
    assert!(write_bytes(base + 0x9, &[0xAB]));
    assert!(write_bytes(base + 0xC, &30.0f32.to_le_bytes()));
    assert_eq!(words[1] & 0xFF00, 0xAB00);
    assert_eq!((words[1] >> 32) as u32, 30.0f32.to_bits());
    assert!(!write_bytes(0, &[1]));
    assert!(!write_bytes(base, &[]));
    let code = write_bytes as *const () as usize;
    assert!(!write_bytes(code, &[0]), "code is not writable");
}
