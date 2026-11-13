//! `rustic-isles-runtime.dll`, the MIT game runtime (plan item 11, RT-1): it loads before the AGPL
//! `Rustic-Isles.dll`, owns MinHook's patches through its hook broker, and answers which ids it owns (R0 7).
//! Its pure half is `rustic-isles-runtime-core`. On any target but x64 Windows the crate is empty.

#[cfg(minhook)]
pub mod config;
#[cfg(minhook)]
pub mod handlers;
#[cfg(minhook)]
mod lifecycle;
#[cfg(minhook)]
pub mod login;

/// Every raw pointer read, Win32 call, MinHook call, detour body and C export (RT-1).
#[cfg(minhook)]
#[allow(unsafe_code, reason = "FFI: DllMain, the seam exports, MinHook and game memory")]
pub mod ffi;
