//! `winmm.dll` stand-in: loads the Rustic Isles runtime beside the game and forwards every `winmm.dll` and
//! `version.dll` export to the system copy.
#![cfg(windows)]
#![allow(
    unsafe_code,
    reason = "a proxy DLL is FFI throughout: DllMain, kernel32 calls and no_mangle exports"
)]

mod attach;
mod exports;
mod forward;
mod ini;
mod runtime;
mod sys;
