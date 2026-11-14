//! CL-1's served line in this process (R4 3) and the engine `MULTIHOME` it publishes for R8, R9 and R10 (R4 3.5).

use crate::config;
use crate::ffi::{flags, log, win};
use rustic_isles_runtime_core::login::cmdline::{CULTURE_FLAG, build_served_line};
use std::sync::OnceLock;

static MULTIHOME: OnceLock<Option<String>> = OnceLock::new();

/// The address the engine binds: `None` before the served line is built and when it has no valid `MULTIHOME`.
pub fn engine_multihome() -> Option<&'static str> {
    MULTIHOME.get()?.as_deref()
}

/// Builds the served line once, on the engine's first call (never under the loader lock), publishes the
/// `MULTIHOME` and logs the summary. The line itself is never logged.
pub(crate) fn serve(engine: &[u16]) -> Vec<u16> {
    let server = config::is_server();
    let culture_fix = server && !flags::flag(CULTURE_FLAG);
    let served = build_served_line(engine, &win::argv(), &win::command_line(), server, culture_fix);
    let _ = MULTIHOME.set(served.multihome.clone());
    crate::lifecycle::write_status_once();
    log::line(&served.log_line());
    for note in served.note_lines() {
        log::line(&note);
    }
    served.line
}
