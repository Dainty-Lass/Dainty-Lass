//! The DLL's only unsafe code. Outside this module the crate is safe and calls the wrappers here.

pub mod broker;
pub mod dispatch;
pub mod engine;
pub mod exit;
pub mod exports;
pub mod flags;
pub mod headless;
pub mod http;
pub mod listen;
pub mod loading;
pub mod log;
pub mod login;
pub mod memory;
pub mod minhook;
pub mod reflect;
pub mod widgets;
pub mod win;

mod dllmain;
#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicBool, Ordering};

static DETACHING: AtomicBool = AtomicBool::new(false);

/// True once `DLL_PROCESS_DETACH` arrived: nothing logs, opens a file or reads a flag after it (R0 3.3).
pub fn detaching() -> bool {
    DETACHING.load(Ordering::SeqCst)
}

fn set_detaching() {
    DETACHING.store(true, Ordering::SeqCst);
}
