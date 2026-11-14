//! Slice R8 (spec-runtime-r8): the server's life. Frame pacing before every frame; after it, the expected-player sync,
//! the connection count, the game-mode sample, the AFK write and the tick-side reap; the AFK write at each
//! `K2_PostLogin`; the watchdog thread; and the one way a server ends itself. The raw calls are `ffi::server_life`.

use crate::config;
use crate::ffi::exit::Install;
use crate::ffi::memory::{read, read_bytes, read_unaligned, readable, readable_this_frame};
use crate::ffi::server_life as raw;
use crate::ffi::{detaching, engine, flags, listen as raw_listen, log, memory, reflect, win};
use crate::handlers::{self, PeCall, PeHandler, Point, TickHandler, Verdict};
use crate::listen;
use rustic_isles_runtime_core::config::auth::AuthMode;
use rustic_isles_runtime_core::config::hub::AfkSetting;
use rustic_isles_runtime_core::config::roster;
use rustic_isles_runtime_core::names::Kind;
use rustic_isles_runtime_core::offsets::{layout, r8, r9, r17};
use rustic_isles_runtime_core::registry;
use rustic_isles_runtime_core::server_life::{
    self as core, AfkPair, AfkReport, EmptyReap, FRESH_MS, HubRole, MemoryInfo, POLL_MS, Pacer, PerfContext,
    PerfWindow, PortTimer, Published, RESERVE_AFTER_MS, ReserveSites, Sample, Stale, TickReap, TrimInput, TrimTimer,
    flags as names,
};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

// 5.6: written on the game thread, read by the watchdog; each value stands alone and stamps decide freshness
static CONN_COUNT: AtomicI32 = AtomicI32::new(-1);
static CONN_STAMP: AtomicU64 = AtomicU64::new(0);
static HAD_CLIENT: AtomicBool = AtomicBool::new(false);
static LAST_CLIENT_MS: AtomicU64 = AtomicU64::new(0);
static NUM_PLAYERS: AtomicI32 = AtomicI32::new(-1);
static NUM_TRAVELLING: AtomicI32 = AtomicI32::new(-1);
static COUNTS_STAMP: AtomicU64 = AtomicU64::new(0);
static EVER_HAD_COUNTS: AtomicBool = AtomicBool::new(false);
static SAW_PLAYER: AtomicBool = AtomicBool::new(false);
static LAST_TICK_MS: AtomicU64 = AtomicU64::new(0);
static ROLE: AtomicU8 = AtomicU8::new(0);
// 7.8's frame sample, in µs since `EPOCH` (+1, so 0 is "no start")
static LAST_SAMPLE_US: AtomicU64 = AtomicU64::new(0);
static FRAMES: AtomicU64 = AtomicU64::new(0);
static SUM_US: AtomicU64 = AtomicU64::new(0);
static MAX_US: AtomicU64 = AtomicU64::new(0);
static HITCHES: AtomicU64 = AtomicU64::new(0);
static FIRST_WINDOW: AtomicBool = AtomicBool::new(true);
static REAPING: AtomicBool = AtomicBool::new(false);
static EPOCH: OnceLock<Instant> = OnceLock::new();

struct Pacing {
    pacer: Pacer,
    logged: bool,
}

static PACING: Mutex<Pacing> = Mutex::new(Pacing {
    pacer: Pacer::new(),
    logged: false,
});

struct Afk {
    disabled: bool,
    hub: AfkSetting,
    hunt: AfkSetting,
}

/// The game thread's own state.
struct Game {
    started: bool,
    roster: Option<(u32, bool)>,
    mode_written: bool,
    state_written: bool,
    last_game_mode: usize,
    keep_hub: Option<bool>,
    tick_reap: TickReap,
    afk: Option<Afk>,
    afk_tick_ms: Option<u64>,
    afk_logged_state: usize,
}

static GAME: Mutex<Game> = Mutex::new(Game {
    started: false,
    roster: None,
    mode_written: false,
    state_written: false,
    last_game_mode: 0,
    keep_hub: None,
    tick_reap: TickReap::new(),
    afk: None,
    afk_tick_ms: None,
    afk_logged_state: 0,
});

fn now_us() -> u64 {
    let epoch = EPOCH.get_or_init(Instant::now);
    u64::try_from(epoch.elapsed().as_micros()).unwrap_or(u64::MAX - 1) + 1
}

/// The hub role (9.2), for a later slice that treats hubs differently.
pub fn hub_role() -> HubRole {
    HubRole::from_u8(ROLE.load(Ordering::Relaxed))
}

/// The last published connection count while fresh (10 s).
pub fn connection_count() -> Option<i32> {
    let stamp = CONN_STAMP.load(Ordering::Relaxed);
    (stamp != 0 && win::tick().saturating_sub(stamp) <= FRESH_MS).then(|| CONN_COUNT.load(Ordering::Relaxed))
}

/// The reap exit (8.1): one line, then `TerminateProcess(0)`. Callable from any thread.
pub fn reap_exit(reason: &str) -> ! {
    if !REAPING.swap(true, Ordering::SeqCst) {
        log::line(&core::reap_line(reason));
    }
    // R0's writer wrote the line with one WriteFile: no flush wait (departure 4)
    raw::end_process()
}

/// The server quit (8.2): the engine's quit cannot end a server without a local player, so R10's fence ends here.
pub fn server_quit(reason: &str) -> ! {
    log::line(&core::quit_line(reason));
    reap_exit(&core::quit_reason(reason))
}

/// Server Init, after `minhook::initialize()` and before `handlers::freeze()`. R8 hooks nothing.
pub fn install() -> Vec<Install> {
    let mut installs: Vec<Install> = Vec::new();
    if registry::owns(b"SP-3@R8") {
        // Pacing first of every before handler; the steps below R9's maintenance (RT-17)
        let handlers = [
            TickHandler {
                slice: "R8",
                order: -2000,
                before: Some(pace),
                after: None,
            },
            TickHandler {
                slice: "R8",
                order: 600,
                before: None,
                after: Some(after_tick),
            },
        ];
        for handler in handlers {
            if let Err(reason) = handlers::register_tick(handler) {
                installs.push(("SP-3@R8", Err(reason)));
            }
        }
    }
    if registry::owns(b"SP-2@R8") {
        let handler = PeHandler {
            slice: "R8",
            order: 0,
            matcher: &[(Kind::FunctionName, "K2_PostLogin")],
            before: Some(post_login),
            after: None,
        };
        if let Err(reason) = handlers::register_pe(Point::Sp2, handler) {
            installs.push(("SP-2@R8", Err(reason)));
        }
    }
    installs
}

// ---- 4: pacing and the frame sample ----

fn frame_sample(now: u64) {
    let previous = LAST_SAMPLE_US.swap(now, Ordering::Relaxed);
    if previous == 0 {
        return;
    }
    let interval = now.saturating_sub(previous);
    FRAMES.fetch_add(1, Ordering::Relaxed);
    SUM_US.fetch_add(interval, Ordering::Relaxed);
    MAX_US.fetch_max(interval, Ordering::Relaxed);
    if interval > core::HITCH_US {
        HITCHES.fetch_add(1, Ordering::Relaxed);
    }
}

fn reset_perf() {
    LAST_SAMPLE_US.store(0, Ordering::Relaxed);
    for counter in [&FRAMES, &SUM_US, &MAX_US, &HITCHES] {
        counter.store(0, Ordering::Relaxed);
    }
    FIRST_WINDOW.store(true, Ordering::Relaxed);
}

/// The SP-3 before handler (4.2), order -2000.
fn pace(_engine: usize, _delta: f32) {
    if detaching() {
        return;
    }
    let now = now_us();
    frame_sample(now);
    let Ok(mut pacing) = PACING.try_lock() else {
        return;
    };
    let (max, idle) = config::frame_pacing();
    if !pacing.logged {
        pacing.logged = true;
        let line = rustic_isles_runtime_core::config::pacing::frame_pace_line(max.clone(), idle.clone());
        log::line(&format!("[MitRuntime] {line}"));
    }
    let count = CONN_COUNT.load(Ordering::Relaxed);
    let step = pacing
        .pacer
        .step(now, win::tick(), max.0, idle.0, listen::listening(), count);
    if let Some(edge) = &step.edge {
        log::line(edge);
    }
    if step.sleep_us > 0 {
        std::thread::sleep(Duration::from_micros(step.sleep_us));
    }
    pacing.pacer.mark(now_us());
}

// ---- 5: the steps after the engine tick ----

/// The live-instance test (5.2).
fn live_instance(object: usize, class: &str) -> bool {
    object != 0
        && engine::is_live_object(object)
        && read::<u32>(object + layout::UOBJECT_FLAGS).is_some_and(|flags| flags & r17::RF_CLASS_DEFAULT_OBJECT == 0)
        && read::<usize>(object + layout::UOBJECT_CLASS).is_some_and(engine::is_live_object)
        && reflect::is_a(object, class)
}

fn world_object(world: usize, offset: usize) -> usize {
    read::<usize>(world + offset).unwrap_or(0)
}

/// The SP-3 after handler (5.1), order 600: only while R9 listens, only with a world.
fn after_tick(_engine: usize, delta: f32) {
    if detaching() {
        return;
    }
    let Some(state) = listen::state() else {
        return;
    };
    let Some(world) = raw_listen::gworld() else {
        return;
    };
    let Ok(mut game) = GAME.try_lock() else {
        return;
    };
    let now = win::tick();
    if !game.started {
        game.started = true;
        reset_perf();
        prime_afk(&mut game);
        start_watchdog();
    }
    sync_expected(&mut game, world);
    count_connections(state.driver, now);
    if config::launch().is_some_and(|launch| launch.watchdog) {
        connected_step(&mut game, world, state.driver, delta, now);
    }
}

/// 5.2: `ExpectedPlayerCount` equals the launch roster, every frame.
fn sync_expected(game: &mut Game, world: usize) {
    let (value, disabled) = *game.roster.get_or_insert_with(|| {
        let roster = config::launch().and_then(|launch| launch.expected_players.as_deref());
        let cap = u32::try_from(config::hub_cap().value).unwrap_or(0);
        let value = roster::expected_player_count(roster, cap);
        let disabled = flags::flag(names::DISABLE_EXPECTED_PLAYER_ZERO);
        log::line(&core::roster_line(disabled, value));
        (value, disabled)
    });
    if disabled || !readable(world, r8::WORLD_SPAN) {
        return;
    }
    let mode = world_object(world, r8::WORLD_AUTHORITY_GAME_MODE);
    if live_instance(mode, "ArchonGameMode") {
        let logged = game.mode_written;
        game.mode_written |= write_expected(mode, r8::GAME_MODE_EXPECTED_PLAYERS, value, "GameMode", logged);
    }
    let state = world_object(world, r8::WORLD_GAME_STATE);
    if live_instance(state, "ArchonGameState") {
        let logged = game.state_written;
        game.state_written |= write_expected(state, r8::GAME_STATE_EXPECTED_PLAYERS, value, "GameState", logged);
    }
}

/// True when it wrote.
fn write_expected(object: usize, offset: usize, value: u32, which: &str, logged: bool) -> bool {
    let address = object + offset;
    let Ok(target) = i32::try_from(value) else {
        return false;
    };
    let Some(before) = read::<i32>(address) else {
        return false;
    };
    if before == target || !reflect::write::<i32>(address, target) {
        return false;
    }
    if !logged {
        let class = reflect::class_name_of(object).unwrap_or_else(|| "?".to_owned());
        log::line(&core::expected_write_line(which, object, &class, before, value));
    }
    true
}

/// 5.3: the raw `ClientConnections.Num`, the emptiness signal.
fn count_connections(driver: usize, now: u64) {
    let num = driver + r8::NET_DRIVER_CLIENT_COUNT;
    if !readable_this_frame(num, 8) {
        return;
    }
    let Some(count) = read::<i32>(num).filter(|&count| count >= 0) else {
        return;
    };
    CONN_COUNT.store(count, Ordering::Relaxed);
    CONN_STAMP.store(now, Ordering::Relaxed);
    if count > 0 {
        HAD_CLIENT.store(true, Ordering::Relaxed);
        LAST_CLIENT_MS.store(now, Ordering::Relaxed);
    }
}

/// 5.4: the world's game mode, else the last one while it is still live (departure 1).
fn sample_game_mode(game: &mut Game, world: usize) -> Option<usize> {
    let mode = readable(world, r8::WORLD_SPAN)
        .then(|| world_object(world, r8::WORLD_AUTHORITY_GAME_MODE))
        .filter(|&mode| live_instance(mode, "ArchonGameMode") && readable(mode, r8::GAME_MODE_SAMPLE_SPAN));
    if let Some(mode) = mode {
        game.last_game_mode = mode;
        return Some(mode);
    }
    let last = game.last_game_mode;
    (last != 0 && engine::is_live_object(last) && readable(last, r8::GAME_MODE_SAMPLE_SPAN)).then_some(last)
}

/// The role, classified once from the first game mode (5.4).
fn classify(game: &mut Game, mode: usize) -> HubRole {
    let current = hub_role();
    if current != HubRole::Unknown {
        return current;
    }
    let Some(class) = reflect::class_name_of(mode) else {
        return current;
    };
    let keep = *game
        .keep_hub
        .get_or_insert_with(|| flags::flag(names::KEEP_HOSTED_HUB_ROLE));
    let (role, line) = core::hub_role(&class, config::auth_mode(), crate::player_host::is_hub(), keep);
    ROLE.store(role.to_u8(), Ordering::Relaxed);
    if let Some(line) = line {
        log::line(&line);
    }
    role
}

/// A client connection with an owning actor in state open (5.5).
fn has_connection(driver: usize) -> bool {
    listen::client_connections(driver).into_iter().any(|connection| {
        readable_this_frame(connection, r8::CONNECTION_SPAN)
            && read::<usize>(connection + r8::CONNECTION_OWNING_ACTOR).is_some_and(|actor| actor != 0)
            && read_unaligned::<u32>(connection + r9::CONNECTION_STATE) == Some(3)
    })
}

/// 5.1 step 4: the game-mode sample, the hub role, the tick-side reap and the AFK tick write.
fn connected_step(game: &mut Game, world: usize, driver: usize, delta: f32, now: u64) {
    let empty = sample_game_mode(game, world).map(|mode| {
        let players = read_unaligned::<i32>(mode + r8::GAME_MODE_NUM_PLAYERS).unwrap_or(-1);
        let travelling = read_unaligned::<i32>(mode + r8::GAME_MODE_NUM_TRAVELLING).unwrap_or(-1);
        classify(game, mode);
        NUM_PLAYERS.store(players, Ordering::Relaxed);
        NUM_TRAVELLING.store(travelling, Ordering::Relaxed);
        COUNTS_STAMP.store(now, Ordering::Relaxed);
        EVER_HAD_COUNTS.store(true, Ordering::Relaxed);
        LAST_TICK_MS.store(now, Ordering::Relaxed);
        if players > 0 {
            SAW_PLAYER.store(true, Ordering::Relaxed);
        }
        players <= 0 && travelling <= 0
    });
    let connected = empty.is_none() && has_connection(driver);
    if let Some(reason) = game.tick_reap.step(empty, connected, delta, hub_role()) {
        reap_exit(&reason);
    }
    if game.afk_tick_ms.is_none_or(|last| now.saturating_sub(last) >= 1000) {
        game.afk_tick_ms = Some(now);
        apply_afk(game, "tick", None);
    }
}

// ---- 6: AFK ----

/// 6.1: primed once, at the start step or the first write.
fn prime_afk(game: &mut Game) {
    if game.afk.is_some() {
        return;
    }
    let disabled = flags::flag(names::DISABLE_AFK_TIMEOUT_OVERRIDE);
    let (hub, hunt) = (config::hub_afk(), config::hunt_afk());
    if disabled {
        log::line(core::AFK_OFF);
    } else {
        log::line(&core::afk_config_line(hub, hunt, config::hub_cap()));
        for line in core::afk_warnings(hub, hunt) {
            log::line(&line);
        }
    }
    game.afk = Some(Afk { disabled, hub, hunt });
}

/// Both floats as one aligned read (the pair starts 8-aligned in both classes).
fn read_pair(address: usize) -> Option<AfkPair> {
    let bits = read::<u64>(address)?;
    #[allow(clippy::cast_possible_truncation, reason = "the two halves of the pair")]
    Some((f32::from_bits(bits as u32), f32::from_bits((bits >> 32) as u32)))
}

fn write_pair(address: usize, value: f32) -> bool {
    let bits = u64::from(value.to_bits());
    reflect::write::<u64>(address, bits | (bits << 32))
}

/// `apply(why, hint)` (6.3), on the game thread.
fn apply_afk(game: &mut Game, why: &str, hint: Option<usize>) {
    prime_afk(game);
    let Some(afk) = &game.afk else {
        return;
    };
    if afk.disabled {
        return;
    }
    let (hub_setting, hunt_setting) = (afk.hub, afk.hunt);
    let Some(world) = raw_listen::gworld().filter(|&world| readable(world, r8::WORLD_SPAN)) else {
        return;
    };
    let state = world_object(world, r8::WORLD_GAME_STATE);
    if !live_instance(state, "ArchonGameState") || !readable(state, r8::GAME_STATE_AFK_SPAN) {
        return;
    }
    let mode = Some(world_object(world, r8::WORLD_AUTHORITY_GAME_MODE))
        .filter(|&mode| live_instance(mode, "ArchonGameMode") && readable(mode, r8::GAME_MODE_AFK_SPAN));
    let mut role = hub_role();
    if role == HubRole::Unknown {
        // A hint only classifies; it is never written
        if let Some(classifier) = mode.or(hint.filter(|&hint| live_instance(hint, "ArchonGameMode"))) {
            role = classify(game, classifier);
        }
    }
    if role == HubRole::Unknown {
        return;
    }
    let setting = if role == HubRole::Hub {
        hub_setting
    } else {
        hunt_setting
    };
    let Some(gs_before) = read_pair(state + r8::GAME_STATE_AFK_TIMEOUT) else {
        return;
    };
    let gm_before = mode.and_then(|mode| read_pair(mode + r8::GAME_MODE_AFK_TIMEOUT));
    let plan = core::afk_plan(setting.seconds, gs_before, gm_before);
    if let Some(value) = plan.value {
        if plan.game_state {
            write_pair(state + r8::GAME_STATE_AFK_TIMEOUT, value);
        }
        if plan.game_mode
            && let Some(mode) = mode
        {
            write_pair(mode + r8::GAME_MODE_AFK_TIMEOUT, value);
        }
    }
    if game.afk_logged_state != state {
        game.afk_logged_state = state;
        let class = mode.and_then(reflect::class_name_of);
        log::line(&core::afk_apply_line(&AfkReport {
            hub: role == HubRole::Hub,
            class: class.as_deref(),
            gm_before,
            gs_before,
            seconds: setting.seconds,
            why,
            game_state: state,
        }));
    }
}

/// The SP-2 before handler (6.4): the object receiving `K2_PostLogin` is the game mode. Never absorbs.
fn post_login(call: &PeCall) -> Verdict {
    if !detaching()
        && let Ok(mut game) = GAME.try_lock()
    {
        apply_afk(&mut game, "postlogin", Some(call.object));
    }
    Verdict::Continue
}

// ---- 7: the watchdog thread ----

struct Watch {
    keep_oom: bool,
    perf: bool,
    trim_allowed: bool,
    full_launch: bool,
    port: u16,
}

/// 7.1: started once from the game thread at listen (R0 3.1 forbids a thread from `DllMain`).
fn start_watchdog() {
    let keep_oom = flags::flag(names::KEEP_OOM_POOL);
    let perf = !flags::flag(names::DISABLE_PERF_LOG);
    let trim = core::trim_state(
        config::auth_mode() == AuthMode::GlobalGameserver,
        flags::flag(names::KEEP_WORKING_SET),
    );
    let watch = Watch {
        keep_oom,
        perf,
        trim_allowed: trim.is_ok(),
        full_launch: config::launch().is_some_and(|launch| launch.watchdog),
        port: config::port(),
    };
    let lines = core::watchdog_start_lines(keep_oom, trim, perf);
    match std::thread::Builder::new()
        .name("rir-watchdog".into())
        .spawn(move || watchdog(&watch))
    {
        Ok(_) => {
            for line in lines {
                log::line(&line);
            }
        }
        Err(error) => log::line(&core::watchdog_failed_line(&error.to_string())),
    }
}

fn published() -> Published {
    Published {
        conn_count: CONN_COUNT.load(Ordering::Relaxed),
        conn_stamp: CONN_STAMP.load(Ordering::Relaxed),
        num_players: NUM_PLAYERS.load(Ordering::Relaxed),
        num_travelling: NUM_TRAVELLING.load(Ordering::Relaxed),
        counts_stamp: COUNTS_STAMP.load(Ordering::Relaxed),
    }
}

/// The port probe's plan, decided at the first probe; `None` never acts.
fn probe_plan(port: u16) -> Option<[u8; 4]> {
    if port == 0 {
        return None;
    }
    match core::probe_address(crate::login::cmdline::engine_multihome()) {
        Ok(address) => Some(address),
        Err(line) => {
            log::line(&line);
            None
        }
    }
}

/// The thread never reads game memory: only the atomics of 5.6.
fn watchdog(watch: &Watch) {
    let start = win::tick();
    let mut iter: u64 = 0;
    let mut reserve_done = watch.keep_oom;
    let mut empty = EmptyReap::new();
    let mut port = PortTimer::new();
    let mut trim = TrimTimer::new();
    let mut probe: Option<Option<[u8; 4]>> = None;
    let mut implausible_lines = 0;
    let mut previous_faults: Option<u32> = None;
    loop {
        win::sleep_ms(POLL_MS);
        iter += 1;
        // Departure 5: the thread ends with the process's detach instead of idling
        if detaching() {
            return;
        }
        let now = win::tick();
        if !reserve_done && now.saturating_sub(start) >= RESERVE_AFTER_MS {
            reserve_done = true;
            release_reserve();
        }
        if watch.perf && iter.is_multiple_of(30) {
            perf_lines(now, &mut previous_faults);
        }
        if !watch.full_launch {
            empty.empty_seconds = 0;
            continue;
        }
        let sample = Sample::from(published(), now);
        if sample.implausible && implausible_lines < core::IMPLAUSIBLE_LINES {
            implausible_lines += 1;
            let counts = published();
            log::line(&core::implausible_line(counts.num_players, counts.num_travelling));
        }
        let role = hub_role();
        let had_client = HAD_CLIENT.load(Ordering::Relaxed);
        if core::heartbeat_due(iter, sample.empty, role) {
            let limit = core::empty_limit(had_client);
            log::line(&core::heartbeat_line(
                role,
                &sample,
                had_client,
                empty.empty_seconds,
                limit,
            ));
        }
        let input = TrimInput {
            hub: role == HubRole::Hub,
            counts_usable: sample.counts_usable,
            conn_stamp: CONN_STAMP.load(Ordering::Relaxed),
            conn_count: CONN_COUNT.load(Ordering::Relaxed),
            travelling: NUM_TRAVELLING.load(Ordering::Relaxed),
            last_client_ms: LAST_CLIENT_MS.load(Ordering::Relaxed),
            trim_allowed: watch.trim_allowed,
        };
        if let Some(idle_seconds) = trim.step(input, now) {
            trim_working_set(idle_seconds);
        }
        let ever = EVER_HAD_COUNTS.load(Ordering::Relaxed);
        if ever && let Some(address) = *probe.get_or_insert_with(|| probe_plan(watch.port)) {
            match raw::port_free(address, watch.port) {
                Ok(free) => {
                    if let Some(line) = port.step(true, free, now, watch.port) {
                        log::line(&line);
                        reap_exit("port no longer served");
                    }
                }
                Err(error) => {
                    log::line(&format!(
                        "[MitRuntime] [Watchdog] port probe off: WSAStartup failed (error {error})"
                    ));
                    probe = Some(None);
                }
            }
        }
        if let Some(reason) = empty.step(sample.empty, role, had_client) {
            reap_exit(reason);
        }
        let players = NUM_PLAYERS.load(Ordering::Relaxed);
        let last_tick = LAST_TICK_MS.load(Ordering::Relaxed);
        match core::stale_tick(role, SAW_PLAYER.load(Ordering::Relaxed), last_tick, now, iter, players) {
            Stale::Quiet => {}
            Stale::Notice(line) => log::line(&line),
            Stale::Reap(line) => {
                log::line(&line);
                reap_exit("tick stale");
            }
        }
    }
}

/// 7.6: the engine's 32 MB out-of-memory reserve, released only when every fact checks out.
fn release_reserve() {
    if let Err(line) = try_release_reserve() {
        log::line(&line);
    }
}

fn code_bytes(rva: u32, offset: usize, count: usize) -> Option<Vec<u8>> {
    let address = win::image_base()
        .checked_add(usize::try_from(rva).ok()?)?
        .checked_add(offset)?;
    let mut bytes = vec![0u8; count];
    read_bytes(address, &mut bytes).then_some(bytes)
}

fn try_release_reserve() -> Result<(), String> {
    let sites = ReserveSites {
        setup_entry: memory::function_entry(r8::SETUP_MEMORY_POOLS),
        setup_bytes: code_bytes(r8::SETUP_MEMORY_POOLS, 0, core::SETUP_POOLS_BYTES.len()),
        oom_entry: memory::function_entry(r8::ON_OUT_OF_MEMORY),
        oom_load: code_bytes(r8::ON_OUT_OF_MEMORY, core::OOM_LOAD_OFFSET, core::OOM_LOAD_BYTES.len()),
        oom_test: code_bytes(r8::ON_OUT_OF_MEMORY, core::OOM_TEST_OFFSET, core::OOM_TEST_BYTES.len()),
    };
    core::reserve_sites_ok(&sites, r8::SETUP_MEMORY_POOLS, r8::ON_OUT_OF_MEMORY).map_err(core::reserve_left_line)?;
    let global = usize::try_from(r8::BACKUP_MEMORY_POOL)
        .ok()
        .and_then(|rva| win::image_base().checked_add(rva))
        .ok_or_else(|| core::reserve_left_line("global unreadable"))?;
    let block = read::<usize>(global).ok_or_else(|| core::reserve_left_line("global unreadable"))?;
    if !core::reserve_block_ok(block, raw::block_region(block)) {
        return Err(core::reserve_left_line(core::BLOCK_REASON));
    }
    let before = raw::memory_info();
    if !raw::swap_global(global, block, 0) {
        return Err(core::reserve_left_line("the global changed"));
    }
    if let Err(error) = raw::release(block) {
        raw::swap_global(global, 0, block);
        return Err(core::reserve_kept_line(error));
    }
    log::line(&core::reserve_released_line(before, raw::memory_info()));
    Ok(())
}

/// 7.7: empties the working set of a hub idle for 60 s.
fn trim_working_set(idle_seconds: u64) {
    let before = raw::memory_info();
    let started = Instant::now();
    let result = raw::trim_working_set();
    let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    log::line(&core::trim_line(
        idle_seconds,
        before,
        raw::memory_info(),
        ms,
        result.err(),
    ));
}

/// 7.8: the window, drained with exchange-to-zero.
fn perf_lines(now: u64, previous_faults: &mut Option<u32>) {
    let window = PerfWindow {
        frames: FRAMES.swap(0, Ordering::Relaxed),
        sum_us: SUM_US.swap(0, Ordering::Relaxed),
        max_us: MAX_US.swap(0, Ordering::Relaxed),
        hitches: HITCHES.swap(0, Ordering::Relaxed),
    };
    let stamp = CONN_STAMP.load(Ordering::Relaxed);
    let context = PerfContext {
        connections: CONN_COUNT.load(Ordering::Relaxed),
        stale: stamp == 0 || now.saturating_sub(stamp) > FRESH_MS,
        role: hub_role(),
        first: FIRST_WINDOW.swap(false, Ordering::Relaxed),
    };
    log::line(&core::perf_line(window, &context));
    let info: Option<MemoryInfo> = raw::memory_info();
    log::line(&core::memory_line(info, *previous_faults, &context));
    if let Some(info) = info {
        *previous_faults = Some(info.page_faults);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_reason_texts() {
        // reap_exit and server_quit end the process: only their texts are checked
        assert_eq!(
            core::reap_line(&core::quit_reason("player host fence")),
            "[MitRuntime] [Reap] server quit (player host fence) -> TerminateProcess(0)"
        );
        for reason in [
            "disposable server empty 60 s after its last client left",
            "disposable server never joined by a client within 300 s",
            "tick stale",
            "port no longer served",
        ] {
            assert!(core::reap_line(reason).starts_with("[MitRuntime] [Reap] "));
        }
    }

    #[test]
    fn trim_logs_a_line() {
        // Behaviour only: the call succeeds on the test process; no figure is checked
        assert!(raw::trim_working_set().is_ok());
        let line = core::trim_line(60, raw::memory_info(), raw::memory_info(), 0, None);
        assert!(line.starts_with("[MitRuntime] [WorkingSet] trimmed (hub idle, no client for 60 s)"));
    }

    #[test]
    fn afk_pairs_round_trip() {
        let mut pair = Box::new(0u64);
        let address = &mut *pair as *mut u64 as usize;
        assert!(write_pair(address, 120.0));
        assert_eq!(read_pair(address), Some((120.0, 120.0)));
    }
}
