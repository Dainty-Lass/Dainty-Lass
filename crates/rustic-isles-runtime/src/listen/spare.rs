//! The warm spare's hold (`rustic_isles_runtime_core::spare`): on the SRV-1 call the engine makes just before its
//! first map load, the game thread waits for the Director's command file, then the getter answers the hunt's URL.
//! Nothing else runs on the game thread meanwhile; the watchdog and the reaper start only after the listen.

use crate::config;
use crate::ffi::{flags, log, win};
use rustic_isles_runtime_core::spare as core;
use std::sync::atomic::{AtomicBool, Ordering};

static HELD: AtomicBool = AtomicBool::new(false);

fn hold_call() -> u32 {
    win::env_value(core::HOLD_CALL_VARIABLE, 8)
        .and_then(|text| text.parse::<u32>().ok())
        .filter(|&call| call > 0)
        .unwrap_or(core::HOLD_CALL)
}

/// SRV-1's call `call`: holds once, on the configured call, until a hunt arrives.
pub fn hold_on(call: usize) {
    if config::has_spare_hunt() || u32::try_from(call).ok() != Some(hold_call()) || HELD.swap(true, Ordering::SeqCst) {
        return;
    }
    let port = config::port();
    let name = core::file_name(port);
    log::line(&core::held_line(hold_call(), port));
    let since = win::tick();
    loop {
        if let Some(path) = flags::path_next_to_exe(&name).filter(|path| path.is_file()) {
            let text = std::fs::read(&path).unwrap_or_default();
            // Removed before it is acted on, as the kick and sim files
            _ = std::fs::remove_file(&path);
            let text = String::from_utf8_lossy(&text[..text.len().min(core::MAX_BYTES)]).into_owned();
            match core::parse(&text) {
                Ok(hunt) => {
                    config::set_spare_hunt(&hunt);
                    log::line(&core::arrived_line(&hunt, win::tick().saturating_sub(since)));
                    return;
                }
                Err(refusal) => log::line(&core::refused_line(refusal)),
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(u64::from(core::POLL_MS)));
    }
}
