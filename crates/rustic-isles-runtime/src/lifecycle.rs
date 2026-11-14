//! Init (R0 3.2) and the status lines the first detour call writes (R0 4.3).

use crate::config;
use crate::ffi::exit::{self, Install};
use crate::ffi::{dispatch, engine, flags, http, loading, log, minhook, net, win};
use crate::handlers;
use crate::login;
use rustic_isles_runtime_core::config::launch::PORT_EXIT_CODE;
use rustic_isles_runtime_core::flags::mode_line;
use rustic_isles_runtime_core::log::{install_line, status_line};
use rustic_isles_runtime_core::registry::owned_count;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

struct Status {
    server: bool,
    port: u16,
    installs: Vec<Install>,
}

static STATUS: OnceLock<Status> = OnceLock::new();
static WRITTEN: AtomicBool = AtomicBool::new(false);

/// Runs once from `DllMain(PROCESS_ATTACH)`.
pub fn init() {
    if STATUS.get().is_some() {
        return;
    }
    flags::init();
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
    exit::register();
    if server {
        crate::headless::register();
        crate::player_host::register();
    }
    let listen = if server {
        crate::listen::install(base, initialized)
    } else {
        Vec::new()
    };
    loading::register(server);
    if server {
        net::register();
    }
    // Slices register their handlers before this point; the lists are read lock-free from here on
    handlers::freeze();
    let hooks_off = (initialized != minhook::MH_OK).then(|| minhook::status_name(initialized));
    let chain = |id: &'static str, install: fn(usize) -> Result<(), i32>| -> Install {
        match hooks_off {
            Some(status) => (id, Err(status)),
            None => (id, install(base).map_err(minhook::status_name)),
        }
    };
    exit::prepare(base);
    // R3 3.4: the install lines keep the AGPL runtime's order
    let mut installs: Vec<Install> = Vec::new();
    if server {
        // CL-1 before every other hook (R0 7.6, R4 7.3)
        installs.extend(login::install(true, base));
        // HL-4 right after CL-1, then the rest of R5 (R0 7.6); its lines are deferred
        crate::headless::install(initialized);
        installs.extend(exit::install_exports(hooks_off));
        installs.push(exit::install_veh(base));
        installs.push(chain("SP-2", dispatch::install_sp2));
        installs.push(chain("SP-3", dispatch::install_sp3));
        installs.extend(net::install(true, base, hooks_off));
        installs.push(chain("HTTP-1", http::install));
        installs.push(crate::player_host::install_header(base, hooks_off));
        installs.extend(listen);
        installs.push(exit::install_engine_error(base, hooks_off));
        installs.push(exit::install_crash_wait());
    } else {
        installs.push(chain("SP-1", dispatch::install_sp1));
        installs.extend(net::install(false, base, hooks_off));
        installs.extend(login::install(false, base));
        installs.push(chain("HTTP-1", http::install));
        installs.extend(exit::install_exports(hooks_off));
        installs.push(exit::install_veh(base));
    }
    installs.extend(loading::install(base, server));
    // R6 after the dispatch installs and R5; its lines are deferred
    crate::widgets::install(base, server, initialized);
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
        log::line(&install_line(id, *result));
    }
    log::line(mode_line(flags::uncached()));
    log::line(&format!("[MitRuntime] {}", config::resolve_backend().log_line()));
    log::flush_deferred();
}
