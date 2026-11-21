//! Slice R9 (spec-runtime-r9): the server's map URL (SRV-1), the listen on the first frame with a world, the ready
//! lines, the per-frame driver upkeep, and the listening state later slices read. The raw calls are `ffi::listen`.

use crate::config;
use crate::ffi::exit::Install;
use crate::ffi::listen::{self as raw, EngineString};
use crate::ffi::memory::{ProcessMemory, read, readable};
use crate::ffi::{engine, log, minhook, reflect, win};
use crate::handlers::{self, TickHandler};
use crate::login::cmdline::engine_multihome;
use rustic_isles_runtime_core::config::auth::{AuthMode, INVALID_EXIT_CODE};
use rustic_isles_runtime_core::config::roster;
use rustic_isles_runtime_core::listen::{
    self as core, Decision, GAME_NET_DRIVER, LAUNCH_ID_MAX, LAUNCH_ID_VARIABLE, ListenSchedule, Name, Request, Skip,
    lines,
};
use rustic_isles_runtime_core::offsets::{r9, r17};
use rustic_isles_runtime_core::reflect::{class_is_a, scan};
use rustic_isles_runtime_core::registry;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// The listen R9 made (8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Listening {
    /// The `GameNetDriver`.
    pub driver: usize,
    /// The port it listens on.
    pub port: u16,
    /// `GetTickCount64` when the listen succeeded.
    pub since_ms: u64,
}

/// A player host's refusal of the listen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refused;

/// What a player host's module (R10) does around the listen (8.2).
pub trait PlayerHostListen: Sync {
    /// `Err` refuses the listen; `Ok(Some(host))` pins the address the engine must already bind.
    fn prepare_listen(&self) -> Result<Option<&'static str>, Refused>;
    /// Reports readiness to the launcher (R10's ready pipe); false fails the listen.
    fn report_ready(&self, port: u16) -> bool;
    /// The listen failed: fence the host.
    fn on_listen_failed(&self);
}

static STATE: OnceLock<Listening> = OnceLock::new();
static NAME: OnceLock<Name> = OnceLock::new();
static PLAYER_HOST: OnceLock<&'static dyn PlayerHostListen> = OnceLock::new();
static URL: OnceLock<Option<Vec<u16>>> = OnceLock::new();
static HOST_UNITS: OnceLock<Vec<u16>> = OnceLock::new();
static SKIPPING: AtomicBool = AtomicBool::new(false);

struct Progress {
    started: bool,
    schedule: ListenSchedule,
    tried: bool,
}

static PROGRESS: Mutex<Progress> = Mutex::new(Progress {
    started: false,
    schedule: ListenSchedule::new(),
    tried: false,
});

/// The listen, once it succeeded. Readable from any thread.
pub fn state() -> Option<Listening> {
    STATE.get().copied()
}

/// True from the frame the listen succeeded.
pub fn listening() -> bool {
    STATE.get().is_some()
}

/// The `GameNetDriver` R9's listen made or found.
pub fn net_driver() -> Option<usize> {
    STATE.get().map(|state| state.driver)
}

/// The `GameNetDriver` name (comparison index, number), once known.
pub fn game_net_driver_name() -> Option<Name> {
    NAME.get().copied()
}

/// Who listens in this process; `None` for an unknown auth mode, which ends at the first frame.
pub fn decision() -> Option<Decision> {
    core::listen_decision(config::auth_mode())
}

/// R10 registers its module once, in its own install before `handlers::freeze()`.
pub fn register_player_host(listen: &'static dyn PlayerHostListen) -> Result<(), &'static str> {
    PLAYER_HOST
        .set(listen)
        .map_err(|_| "a player host listen is already registered")
}

/// The map URL with its NUL, built once; `None` without a launch (R9 call 9). Logs the summary line once.
pub fn map_url() -> Option<&'static [u16]> {
    fn build() -> Option<Vec<u16>> {
        let launch = config::launch()?;
        let cap = u32::try_from(config::hub_cap().value).unwrap_or(1);
        let mut url = roster::map_url(launch, cap, false);
        log::line(&core::map_url_line(launch, url.len(), cap));
        url.push(0);
        Some(url)
    }
    // A spare answers its placeholder until its hunt arrives, then the hunt's URL from then on
    static PLACEHOLDER: OnceLock<Option<Vec<u16>>> = OnceLock::new();
    if config::is_spare() && !config::has_spare_hunt() {
        return PLACEHOLDER.get_or_init(build).as_deref();
    }
    URL.get_or_init(build).as_deref()
}

pub mod spare;

/// Server Init, after `minhook::initialize()` and before `handlers::freeze()`: the two SP-3 handlers and SRV-1.
pub fn install(base: usize, initialized: i32) -> Vec<Install> {
    let mut installs: Vec<Install> = Vec::new();
    if registry::owns(b"SP-3@R9") {
        let handlers = [
            TickHandler {
                slice: "R9",
                order: 700,
                before: None,
                after: Some(maintain_tick),
            },
            TickHandler {
                slice: "R9",
                order: 900,
                before: None,
                after: Some(listen_tick),
            },
        ];
        for handler in handlers {
            if let Err(reason) = handlers::register_tick(handler) {
                installs.push(("SP-3@R9", Err(reason)));
            }
        }
    }
    if registry::owns(b"SRV-1") {
        let result = if initialized == minhook::MH_OK {
            raw::install_srv1(base)
        } else {
            Err(minhook::status_name(initialized))
        };
        installs.push(("SRV-1", result));
    }
    installs
}

/// The listen handler (5.1), order 900, after every other after handler of the frame.
fn listen_tick(engine: usize, _delta: f32) {
    let Ok(mut progress) = PROGRESS.try_lock() else {
        return;
    };
    let auth = config::auth_mode();
    if !progress.started {
        progress.started = true;
        if auth == AuthMode::Invalid {
            log::line(lines::INVALID_AUTH);
            win::terminate(INVALID_EXIT_CODE);
            return;
        }
    }
    if listening() {
        return;
    }
    if decision().is_none() {
        return;
    }
    match progress.schedule.poll(win::tick(), raw::gworld().is_some()) {
        Some(Request::WorldPresent) => log::line(lines::REQUESTED_WORLD),
        Some(Request::Timeout) => log::line(lines::REQUESTED_TIMEOUT),
        None => {}
    }
    if !progress.schedule.requested() || progress.tried {
        return;
    }
    // A failed listen is not retried (as the AGPL)
    progress.tried = true;
    let player_host = auth == AuthMode::PlayerHost;
    let port = config::port();
    match listen(engine, port, player_host) {
        Ok(driver) => {
            let _ = STATE.set(Listening {
                driver,
                port,
                since_ms: win::tick(),
            });
            log::line(&core::listening_line(port, driver));
        }
        Err(failure) => {
            log::line(&failure.line);
            if player_host
                && failure.fence
                && let Some(host) = PLAYER_HOST.get()
            {
                host.on_listen_failed();
            }
        }
    }
}

struct Failure {
    line: String,
    fence: bool,
}

impl From<&str> for Failure {
    fn from(reason: &str) -> Self {
        Self {
            line: core::failed_line(reason),
            fence: true,
        }
    }
}

/// 6.2 then 6.3.
fn listen(engine: usize, port: u16, player_host: bool) -> Result<usize, Failure> {
    let host = if player_host {
        Some(*PLAYER_HOST.get().ok_or(Failure {
            line: core::failed_line("player host has no listen interface"),
            fence: false,
        })?)
    } else {
        None
    };
    let world = raw::gworld().ok_or("no world")?;
    let name = net_driver_name().ok_or("GameNetDriver name unavailable")?;
    let memory = ProcessMemory;
    let context = core::find_world_context(&memory, engine, world).ok_or("no world context for the world")?;
    let created = raw::create_named_net_driver(engine, context, core::name_value(name));
    log::line(&core::created_line(created));
    let driver = find_driver(context, name).ok_or("named net driver not found")?;
    bind(driver, world, name);

    let mut url = [0u64; r9::FURL_SIZE / 8];
    let mut pinned: Option<(usize, i32)> = None;
    if let Some(host) = host {
        match host.prepare_listen() {
            Err(Refused) => return Err("player host refused the listen".into()),
            Ok(None) => {}
            Ok(Some(address)) => {
                match core::pin_check(address, engine_multihome()) {
                    Ok(line) => log::line(&line),
                    Err(line) => return Err(Failure { line, fence: true }),
                }
                let units = HOST_UNITS.get_or_init(|| address.encode_utf16().chain(std::iter::once(0)).collect());
                pinned = Some((units.as_ptr() as usize, i32::try_from(units.len()).unwrap_or(0)));
            }
        }
    }
    let block = core::furl_block(port, pinned);
    for (word, bytes) in url.iter_mut().zip(block.as_chunks::<8>().0) {
        *word = u64::from_le_bytes(*bytes);
    }
    let mut error = EngineString::empty();
    let notify = world.checked_add(r9::WORLD_NETWORK_NOTIFY).ok_or("no world")?;
    let ok = raw::init_listen(driver, notify, &mut url, &mut error)?;
    if let Some(text) = raw::error_text(&error) {
        log::line(&core::init_listen_error_line(&text));
    }
    if !ok {
        return Err("InitListen returned false".into());
    }
    bind(driver, world, name);
    bind_context_world(context, driver);
    if !report_ready(port, host) {
        return Err("readiness report failed".into());
    }
    Ok(driver)
}

/// `Conv_StringToName("GameNetDriver")` once, through the reflection kit (6.1).
fn net_driver_name() -> Option<Name> {
    if let Some(name) = NAME.get() {
        return Some(*name);
    }
    let name = core::name_from_bytes(reflect::name_for(GAME_NET_DRIVER)?);
    if name == (0, 0) {
        return None;
    }
    Some(*NAME.get_or_init(|| name))
}

/// 6.2 steps 3 and 4: the context's active drivers first, then the object array.
fn find_driver(context: usize, name: Name) -> Option<usize> {
    let memory = ProcessMemory;
    let is_net_driver = |driver: usize| reflect::is_a(driver, "NetDriver");
    match core::active_drivers(&memory, context) {
        Some(drivers) => {
            if let Some(driver) = drivers
                .into_iter()
                .find(|&driver| core::driver_matches(&memory, driver, name, is_net_driver))
            {
                log::line(&core::found_line(driver, false));
                return Some(driver);
            }
        }
        None => log::line("[MitRuntime] [Listen] the world context's net driver list is refused; scanning objects"),
    }
    let engine = engine::engine()?;
    // One class walk per class, not per object: the scan reads ~500k headers
    let mut classes: HashMap<usize, bool> = HashMap::new();
    let driver = scan(&engine, |header| {
        header.flags & r17::RF_CLASS_DEFAULT_OBJECT == 0
            && *classes
                .entry(header.class)
                .or_insert_with(|| class_is_a(&engine, header.class, "NetDriver"))
            && core::driver_matches(&memory, header.object, name, |_| true)
    })?;
    log::line(&core::found_line(driver, true));
    Some(driver)
}

/// The driver's name, no server connection, and its world (6.3 steps 1 and 7, 7.2).
fn bind(driver: usize, world: usize, name: Name) {
    reflect::write::<u64>(driver + r9::NET_DRIVER_NAME, core::name_value(name));
    reflect::write::<usize>(driver + r9::NET_DRIVER_SERVER_CONNECTION, 0);
    reflect::write::<usize>(driver + r9::NET_DRIVER_WORLD, world);
}

/// 6.3 step 7: the context's own world drives the driver; only that world is written.
fn bind_context_world(context: usize, driver: usize) {
    let world = readable(context, r9::WORLD_CONTEXT_SIZE)
        .then(|| read::<usize>(context + r9::WORLD_CONTEXT_WORLD))
        .flatten()
        .filter(|&world| world != 0 && readable(world, r9::WORLD_SIZE));
    let Some(world) = world else {
        log::line(lines::WORLD_UNREADABLE);
        return;
    };
    let current = read::<usize>(world + r9::WORLD_NET_DRIVER).unwrap_or(0);
    if current != driver {
        reflect::write::<usize>(world + r9::WORLD_NET_DRIVER, driver);
    }
    log::line(&core::world_driver_line(core::world_driver_note(current, driver)));
    if read::<usize>(driver + r9::NET_DRIVER_WORLD) != Some(world) {
        reflect::write::<usize>(driver + r9::NET_DRIVER_WORLD, world);
        log::line(lines::REALIGNED);
    }
}

/// 7.1: R10's ready pipe for a player host, else the stdout lines the Director waits for.
fn report_ready(port: u16, host: Option<&dyn PlayerHostListen>) -> bool {
    if let Some(host) = host {
        return host.report_ready(port);
    }
    let Some(id) = win::env_value(LAUNCH_ID_VARIABLE, LAUNCH_ID_MAX) else {
        // A server started by hand has no launch id; readiness still succeeds
        log::line(lines::NO_LAUNCH_ID);
        return true;
    };
    if raw::write_stdout(&core::ready_lines(&id, port)) {
        log::line(&core::ready_written_line(&id));
    } else {
        log::line(lines::STDOUT_UNAVAILABLE);
    }
    true
}

/// The per-frame driver upkeep (7.2), order 700: the engine resets these on this build, and a server driver with a
/// stale world or a server connection faults its dispatch.
fn maintain_tick(_engine: usize, _delta: f32) {
    let (Some(state), Some(name)) = (STATE.get(), NAME.get()) else {
        return;
    };
    let driver = state.driver;
    let world = raw::gworld();
    let skip = if !engine::is_live_object(driver) {
        Some(Skip::NotLive)
    } else if !readable(driver, r9::NET_DRIVER_SIZE) {
        Some(Skip::Unreadable)
    } else if world.is_none() {
        Some(Skip::NoWorld)
    } else {
        None
    };
    match (skip, world) {
        (Some(skip), _) => {
            if !SKIPPING.swap(true, Ordering::SeqCst) {
                log::line(&core::skipped_line(skip));
            }
        }
        (None, Some(world)) => {
            SKIPPING.store(false, Ordering::SeqCst);
            bind(driver, world, *name);
        }
        (None, None) => {}
    }
}

/// `client_connections(driver)` (8.3).
pub fn client_connections(driver: usize) -> Vec<usize> {
    core::client_connections(&ProcessMemory, driver)
}

/// `actor_channel(connection, actor)` (8.3): only an exact `ActorChannel`.
pub fn actor_channel(connection: usize, actor: usize) -> Option<usize> {
    core::actor_channel(&ProcessMemory, connection, actor, reflect::find_class("ActorChannel")?)
}

/// `channel_presence(actor, connection)` (8.3).
pub fn channel_presence(actor: usize, connection: usize) -> i32 {
    let class = reflect::find_class("ActorChannel").unwrap_or(0);
    core::channel_presence(&ProcessMemory, actor, connection, class)
}
