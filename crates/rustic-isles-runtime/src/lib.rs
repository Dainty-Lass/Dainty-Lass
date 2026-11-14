//! `rustic-isles-runtime.dll`, the game runtime (plan item 11, RT-1; the only one since R21): it owns MinHook's
//! patches through its hook broker and installs every id its registry owns (R0 7).
//! Its pure half is `rustic-isles-runtime-core`. On any target but x64 Windows the crate is empty.

#[cfg(minhook)]
pub mod branding;
#[cfg(minhook)]
pub mod config;
#[cfg(minhook)]
mod economy;
#[cfg(minhook)]
mod escalation;
#[cfg(minhook)]
pub mod handlers;
#[cfg(minhook)]
pub mod headless;
#[cfg(minhook)]
mod lifecycle;
#[cfg(minhook)]
pub mod listen;
#[cfg(minhook)]
pub mod login;
#[cfg(minhook)]
pub mod occupancy;
#[cfg(minhook)]
pub mod player_host;
#[cfg(minhook)]
pub mod server_life;
#[cfg(minhook)]
pub mod widgets;

/// Every raw pointer read, Win32 call, MinHook call and detour body (RT-1).
#[cfg(minhook)]
#[allow(unsafe_code, reason = "FFI: DllMain, MinHook and game memory")]
pub mod ffi;
