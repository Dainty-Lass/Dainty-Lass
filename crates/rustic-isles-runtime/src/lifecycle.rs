//! Init (R0 3.2) and the status lines the first detour call writes (R0 4.3).

use crate::config;
use crate::ffi::exit::{self, Install};
use crate::ffi::{
    broker, dispatch, engine, flags, http, loading, log, minhook, net, player_role, relevance, repgraph, travel, win,
};
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
    hooks: broker::Applied,
    base: usize,
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
    broker::queue_enables();
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
    let server_life = if server {
        crate::server_life::install()
    } else {
        Vec::new()
    };
    loading::register(server);
    if server {
        net::register();
        repgraph::register();
        relevance::register();
    } else {
        let _ = travel::register(base);
    }
    player_role::register(server);
    // Slices register their handlers before this point; the lists are read lock-free from here on
    crate::economy::register();
    if !server {
        crate::branding::register();
        crate::save_reset::register();
        crate::trials_board::register();
        crate::relay::start();
    }
    crate::ffi::server_events::register(server);
    crate::escalation::register();
    crate::occupancy::register(server);
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
        installs.push(chain("ESC-1", crate::escalation::install_esc1));
        // R12 after NETD-8 and before NETD-3, the AGPL's positions (R12 4.5)
        let mut net = net::install(true, base, hooks_off);
        let at = net.iter().position(|(id, _)| *id == "NETD-3").unwrap_or(net.len());
        let mut graph = repgraph::install(base, hooks_off);
        // RG-2 right after RG-1, the AGPL's position (R13 4.4)
        let after_rg1 = graph.iter().position(|(id, _)| *id == "RG-1").map_or(0, |at| at + 1);
        graph.splice(after_rg1..after_rg1, relevance::install(base, hooks_off));
        // RG-3 after RG-2's position and before RG-4 (R14 4.4)
        let after_rg2 = graph
            .iter()
            .position(|(id, _)| *id == "RG-2")
            .map_or(after_rg1, |at| at + 1);
        graph.splice(after_rg2..after_rg2, player_role::install_rg3(base, hooks_off));
        net.splice(at..at, graph);
        installs.extend(net);
        installs.push(chain("HTTP-1", http::install));
        installs.push(crate::player_host::install_header(base, hooks_off));
        // PR-1 before SRV-1 (R14 4.4)
        installs.extend(player_role::install_pr1(base, hooks_off));
        installs.extend(listen);
        installs.extend(server_life);
        installs.push((
            "PKG-1",
            hooks_off.map_or_else(|| crate::ffi::packages::install(base), Err),
        ));
        installs.push(exit::install_engine_error(base, hooks_off));
        installs.push(exit::install_crash_wait());
    } else {
        installs.push(chain("SP-1", dispatch::install_sp1));
        installs.push(("TR-1", hooks_off.map_or_else(|| travel::install(base), Err)));
        installs.push((
            "CON-2",
            hooks_off.map_or_else(|| crate::branding::install_exec_guard(base), Err),
        ));
        installs.extend(net::install(false, base, hooks_off));
        installs.extend(login::install(false, base));
        // PR-1 after TR-1 and before HTTP-1 (R14 4.4)
        installs.extend(player_role::install_pr1(base, hooks_off));
        installs.push(chain("HTTP-1", http::install));
        installs.extend(exit::install_exports(hooks_off));
        installs.push(exit::install_veh(base));
    }
    installs.extend(loading::install(base, server));
    // R6 after the dispatch installs and R5; its lines are deferred
    crate::widgets::install(base, server, initialized);
    let hooks = broker::apply_queued();
    let _ = STATUS.set(Status {
        server,
        port,
        installs,
        hooks,
        base,
    });
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
    log::line(&hooks_line(&status.hooks, status.base));
    log::line(mode_line(flags::uncached()));
    log::line(&format!("[MitRuntime] {}", config::resolve_backend().log_line()));
    log::flush_deferred();
}

/// The batched enable's result: one line, plus each target left disabled as an image offset.
fn hooks_line(applied: &broker::Applied, base: usize) -> String {
    let mut line = format!("[MitRuntime] hooks: {} enabled together at init", applied.enabled);
    for (target, status) in &applied.failed {
        line.push_str(&format!(
            "; +0x{:X} not enabled ({})",
            target.wrapping_sub(base),
            minhook::status_name(*status)
        ));
    }
    line
}
