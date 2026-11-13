//! Init (R0 3.2) and the status lines the first detour call writes (R0 4.3).

use crate::config;
use crate::ffi::{dispatch, engine, flags, log, minhook, win};
use crate::handlers;
use rustic_isles_runtime_core::config::launch::PORT_EXIT_CODE;
use rustic_isles_runtime_core::flags::mode_line;
use rustic_isles_runtime_core::log::{install_line, status_line};
use rustic_isles_runtime_core::registry::owned_count;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

struct Status {
    server: bool,
    port: u16,
    installs: Vec<(&'static str, Result<(), i32>)>,
}

static STATUS: OnceLock<Status> = OnceLock::new();
static WRITTEN: AtomicBool = AtomicBool::new(false);

/// Runs once from `DllMain(PROCESS_ATTACH)`.
pub fn init() {
    if STATUS.get().is_some() {
        return;
    }
    flags::init();
    config::run_alias();
    config::resolve_backend();
    let Ok(role) = config::resolve_role() else {
        // A full launch whose port is refused ends here, before any hook, with nothing logged (loader lock)
        win::terminate(PORT_EXIT_CODE);
        return;
    };
    let server = role.server;
    let port = config::port();
    log::set_port(port);
    let base = win::image_base();
    engine::set_image_base(base);
    let initialized = minhook::initialize();
    // Slices register their handlers before this point; the lists are read lock-free from here on
    handlers::freeze();
    let installs: Vec<(&'static str, Result<(), i32>)> = if initialized != minhook::MH_OK {
        let ids: &[&'static str] = if server { &["SP-2", "SP-3"] } else { &["SP-1"] };
        ids.iter().map(|&id| (id, Err(initialized))).collect()
    } else if server {
        vec![
            ("SP-2", dispatch::install_sp2(base)),
            ("SP-3", dispatch::install_sp3(base)),
        ]
    } else {
        vec![("SP-1", dispatch::install_sp1(base))]
    };
    let _ = STATUS.set(Status { server, port, installs });
    log::set_ready();
}

/// The first call into any R0 detour writes the status once.
pub fn write_status_once() {
    let Some(status) = STATUS.get() else {
        return;
    };
    if WRITTEN.swap(true, Ordering::SeqCst) {
        return;
    }
    log::line(&status_line(
        env!("CARGO_PKG_VERSION"),
        status.server,
        status.port,
        owned_count(),
    ));
    for (id, result) in &status.installs {
        log::line(&install_line(id, result.map_err(minhook::status_name)));
    }
    log::line(mode_line(flags::uncached()));
    log::line(&format!("[MitRuntime] {}", config::resolve_backend().log_line()));
    log::flush_deferred();
}
