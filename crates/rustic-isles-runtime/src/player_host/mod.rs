//! Slice R10 (spec-runtime-r10): the player host for realms. Init checks the launcher's bootstrap; the first frame
//! reports it and starts the control pipe; every frame applies a fence; R9's listen calls `PlayerHostListen`; and
//! HTTP-1@R10 sends the host capability. The raw calls are `ffi::player_host`.

use crate::config;
use crate::ffi::exit::Install;
use crate::ffi::{log, minhook, player_host as raw, win};
use crate::handlers::{self, TickHandler};
use crate::listen::{self, PlayerHostListen, Refused};
use rustic_isles_runtime_core::config::auth::AuthMode;
use rustic_isles_runtime_core::player_host::{
    BIND, Bootstrap, FENCE_EXIT_CODE, HostedRole, INVALID_EXIT_CODE, THREAD_EXIT_CODE, TICK_ORDER, Verdict, bootstrap,
    guard_line, invalid_lines, lines, ready_message, role_line,
};
use rustic_isles_runtime_core::registry;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

static VERDICT: OnceLock<Verdict> = OnceLock::new();
static ROLE: OnceLock<HostedRole> = OnceLock::new();
static CAPABILITY: OnceLock<Vec<u16>> = OnceLock::new();
static FENCED: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);
static APPLYING: AtomicBool = AtomicBool::new(false);

struct Host;

static HOST: Host = Host;

/// `Some` only in `PlayerHost` mode with a valid bootstrap. Set in Init, read from any thread (11).
pub fn hosted_role() -> Option<&'static HostedRole> {
    ROLE.get()
}

/// A launcher `HUB`.
pub fn is_hub() -> bool {
    hosted_role().is_some_and(|role| role.hub)
}

fn valid() -> Option<&'static Bootstrap> {
    match VERDICT.get()? {
        Verdict::Valid(bootstrap) => Some(bootstrap),
        Verdict::Invalid { .. } => None,
    }
}

/// The capability as UTF-16, for HTTP-1@R10. Never logged.
pub(crate) fn capability() -> Option<&'static [u16]> {
    CAPABILITY.get().map(Vec::as_slice)
}

/// 7.2: set by the control thread and a failed listen; never cleared.
pub(crate) fn fence() {
    FENCED.store(true, Ordering::SeqCst);
}

/// Init, on a server in `PlayerHost` mode, before `handlers::freeze()`: decide the bootstrap (no log, no thread),
/// register the SP-3 handler and, for a valid bootstrap, R9's interface (5.1, 8).
pub fn register() {
    if config::auth_mode() != AuthMode::PlayerHost || !registry::owns(b"SP-3@R10") {
        return;
    }
    let Some(launch) = config::launch() else {
        return;
    };
    let verdict = bootstrap(win::env_raw, win::env_exists, launch, &win::argv());
    if let Verdict::Valid(bootstrap) = &verdict {
        let _ = ROLE.set(bootstrap.role.hosted());
        let _ = CAPABILITY.set(bootstrap.capability.encode_utf16().collect());
        if let Err(reason) = listen::register_player_host(&HOST) {
            log::defer(format!("[MitRuntime] [PlayerHost] {reason}"));
        }
    }
    let _ = VERDICT.set(verdict);
    let handler = TickHandler {
        slice: "R10",
        order: TICK_ORDER,
        before: Some(tick),
        after: None,
    };
    if let Err(reason) = handlers::register_tick(handler) {
        log::defer(format!("[MitRuntime] install SP-3@R10 {reason}"));
    }
}

/// The server install list's `HTTP-1@R10` line (2).
pub fn install_header(base: usize, hooks_off: Option<&'static str>) -> Install {
    if capability().is_none() {
        return ("HTTP-1@R10", Err(lines::NOT_A_PLAYER_HOST));
    }
    let result = match hooks_off {
        Some(status) => Err(status),
        None => raw::install_header(base).map_err(minhook::status_name),
    };
    ("HTTP-1@R10", result)
}

/// The SP-3 before handler (6, 7.3), order -2000.
fn tick(_engine: usize, _delta: f32) {
    if !STARTED.swap(true, Ordering::SeqCst) {
        first_frame();
    }
    if FENCED.load(Ordering::SeqCst) && !APPLYING.swap(true, Ordering::SeqCst) {
        log::line(lines::APPLYING);
        win::terminate(FENCE_EXIT_CODE);
    }
}

/// 6 steps 1 to 4, once.
fn first_frame() {
    let Some(verdict) = VERDICT.get() else {
        return;
    };
    let bootstrap = match verdict {
        Verdict::Invalid {
            failed_check,
            role_problem,
        } => {
            for line in invalid_lines(*failed_check, *role_problem) {
                log::line(&line);
            }
            win::terminate(INVALID_EXIT_CODE);
            return;
        }
        Verdict::Valid(bootstrap) => bootstrap,
    };
    log::line(&role_line(&bootstrap.role));
    log::line(&guard_line(bootstrap.guard_opt_in));
    let started = raw::start_control(
        bootstrap.control_pipe.clone(),
        bootstrap.launcher_pid,
        bootstrap.session.clone(),
        bootstrap.generation.clone(),
    );
    if started.is_err() {
        log::line(lines::THREAD_FAILED);
        win::terminate(THREAD_EXIT_CODE);
        return;
    }
    log::line(lines::WORKER_STARTED);
}

impl PlayerHostListen for Host {
    fn prepare_listen(&self) -> Result<Option<&'static str>, Refused> {
        if valid().is_some() {
            Ok(Some(BIND))
        } else {
            log::line(lines::NOT_LOOPBACK);
            Err(Refused)
        }
    }

    fn report_ready(&self, port: u16) -> bool {
        let Some(bootstrap) = valid() else {
            return false;
        };
        let message = ready_message(&bootstrap.session, &bootstrap.generation, &bootstrap.launch_id, port);
        raw::report_ready(&bootstrap.ready_pipe, &message)
    }

    fn on_listen_failed(&self) {
        fence();
    }
}
