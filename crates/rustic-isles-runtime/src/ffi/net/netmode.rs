//! NETD-4 `InternalGetNetMode` and NETD-5 `UWorld::GetNetMode` (11): the process is a dedicated server. The whole
//! `EAX` is 1: a one-byte answer once left driver-pointer bits above it and the bounty draft stalled.
//! `+0x042CC6E0` is a different function on this build (it returns `GIsClient`) and must never be hooked.

use super::hook;
use crate::ffi::broker::NextCell;
use crate::ffi::exit::Install;
use std::ffi::c_void;

/// `NM_DedicatedServer`.
pub const DEDICATED_SERVER: i32 = 1;

static INTERNAL_NEXT: NextCell = NextCell::new();
static WORLD_NEXT: NextCell = NextCell::new();

// Absorbing detours that read and write nothing: they answer 1 even after detach, since the original would call
// into a driver that may be torn down (R11 call 11)
pub(super) unsafe extern "C" fn internal_detour(_this: *mut c_void) -> i32 {
    DEDICATED_SERVER
}

pub(super) unsafe extern "C" fn world_detour(_this: *mut c_void) -> i32 {
    DEDICATED_SERVER
}

/// Installs NETD-4.
pub fn install_internal(base: usize, hooks_off: Option<&'static str>) -> Install {
    hook(
        "NETD-4",
        base,
        internal_detour as *const () as usize,
        &INTERNAL_NEXT,
        hooks_off,
    )
}

/// Installs NETD-5.
pub fn install_world(base: usize, hooks_off: Option<&'static str>) -> Install {
    hook(
        "NETD-5",
        base,
        world_detour as *const () as usize,
        &WORLD_NEXT,
        hooks_off,
    )
}
