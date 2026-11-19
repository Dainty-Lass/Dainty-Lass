//! Slice R20 (spec-runtime-r20): the server's `ProcessEvent` behaviours on SP-2 (the first login's match start, the
//! `ClientRestart` pawn fixes, the vendor camera absorb, the `PlayerCanRestart` force, the development damage
//! multiplier) and the connected-player tick on SP-3 (the bleedout floor, the manual stamina tick). The decisions
//! are `rustic_isles_runtime_core::server_events`.

use super::loading::restart_gate;
use super::memory::{query, read, read_unaligned, readable, readable_this_frame, readable_unaligned};
use super::{detaching, engine, flags, listen as raw_listen, log, reflect, win};
use crate::config;
use crate::handlers::{self, PeCall, PeHandler, Point, TickHandler, Verdict};
use rustic_isles_runtime_core::offsets::r20;
use rustic_isles_runtime_core::reflect::is_writable_protect;
use rustic_isles_runtime_core::registry;
use rustic_isles_runtime_core::server_events::{self as core, PawnFields, flags as names, lines, matchers, vars};
use rustic_isles_runtime_core::test_cheats::{self, Cheats};
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex, OnceLock};

type SetPackageNameFn = unsafe extern "C" fn(*mut c_void, u64);

const SLICE: &str = "R20";

/// The SP-2 handlers (3.1), and the host's test cheats.
fn pe_handlers() -> [PeHandler; 6] {
    let before = |order, matcher, before| PeHandler {
        slice: SLICE,
        order,
        matcher,
        before: Some(before),
        after: None,
    };
    [
        before(
            matchers::DAMAGE_ORDER,
            matchers::DAMAGE,
            damage as fn(&PeCall) -> Verdict,
        ),
        before(0, matchers::POST_LOGIN, post_login),
        before(0, matchers::CLIENT_RESTART, client_restart),
        before(0, matchers::VENDOR, vendor),
        before(0, matchers::CAN_RESTART, player_can_restart),
        before(0, test_cheats::GAUNTLET_BROADCAST, gauntlet_broadcast),
    ]
}

/// The SP-3 handler (3.1).
fn tick_handler() -> TickHandler {
    TickHandler {
        slice: SLICE,
        order: matchers::TICK_ORDER,
        before: None,
        after: Some(connected_players),
    }
}

/// What Init registers for a role: nothing on a client (3.3).
fn table(server: bool, owns: fn(&[u8]) -> bool) -> (Vec<PeHandler>, Option<TickHandler>) {
    if !server {
        return (Vec::new(), None);
    }
    let pe = if owns(b"SP-2@R20") {
        pe_handlers().into_iter().collect()
    } else {
        Vec::new()
    };
    (pe, owns(b"SP-3@R20").then(tick_handler))
}

/// Init, before `handlers::freeze()` (3.3). R20 hooks nothing.
pub fn register(server: bool) {
    let (pe, tick) = table(server, registry::owns);
    for handler in pe {
        let _ = handlers::register_pe(Point::Sp2, handler);
    }
    if let Some(handler) = tick {
        let _ = handlers::register_tick(handler);
        let _ = handlers::register_tick(TickHandler {
            slice: SLICE,
            order: matchers::TICK_ORDER,
            before: None,
            after: Some(hold_behemoth_health),
        });
    }
}

// ---- memory helpers: game fields at odd offsets, so no alignment rule for these ----

fn ptr_at(addr: usize) -> usize {
    read::<usize>(addr).unwrap_or(0)
}

fn live(object: usize) -> bool {
    object != 0 && engine::is_live_object(object)
}

fn writable(addr: usize, size: usize) -> bool {
    readable_unaligned(addr, size) && query(addr).is_some_and(|region| is_writable_protect(region.protect))
}

/// R17's checked write without its alignment rule (R7 writes single bytes the same way).
fn put<T: Copy>(addr: usize, value: T) -> bool {
    if !writable(addr, std::mem::size_of::<T>()) {
        return false;
    }
    // SAFETY: the range was just checked committed and writable
    unsafe { std::ptr::write_unaligned(addr as *mut T, value) };
    true
}

fn params_ptr(params: usize) -> Option<usize> {
    readable_unaligned(params, r20::POINTER_PARAMS)
        .then(|| read_unaligned::<usize>(params))
        .flatten()
}

// ---- 9: the development damage multiplier ----

fn damage_factor() -> Option<f32> {
    static FACTOR: OnceLock<Option<f32>> = OnceLock::new();
    *FACTOR.get_or_init(|| {
        let allow = win::env_value(vars::ALLOW_DAMAGE_DEBUG.0, vars::ALLOW_DAMAGE_DEBUG.1);
        let factor = win::env_value(vars::DEBUG_DAMAGE_MULTIPLIER.0, vars::DEBUG_DAMAGE_MULTIPLIER.1);
        core::multiplier(
            flags::development_profile(),
            allow.as_deref(),
            flags::flag(names::DEBUG_DAMAGE_MULTIPLIER),
            factor.as_deref(),
        )
    })
}

fn damage(call: &PeCall) -> Verdict {
    if detaching() {
        return Verdict::Continue;
    }
    let Some(factor) = damage_factor() else {
        return Verdict::Continue;
    };
    let Some(name) = call.entry.full_name() else {
        return Verdict::Continue;
    };
    let Some((size, offset)) = core::damage_field(&name) else {
        return Verdict::Continue;
    };
    if !readable_unaligned(call.params, size) {
        return Verdict::Continue;
    }
    let Some(before) = read_unaligned::<f32>(call.params + offset) else {
        return Verdict::Continue;
    };
    let after = before * factor;
    if put::<f32>(call.params + offset, after) {
        let object = reflect::engine()
            .and_then(|engine| engine.object_name(call.object))
            .unwrap_or_else(|| "?".to_owned());
        log::line(&lines::damage(&name, &object, before, after, factor));
    }
    Verdict::Continue
}

// ---- the host's test cheats (`[switches] test_cheats`) ----

fn cheats() -> Cheats {
    static CHEATS: OnceLock<Cheats> = OnceLock::new();
    *CHEATS.get_or_init(|| {
        let value = win::env_value(test_cheats::VARIABLE.0, test_cheats::VARIABLE.1);
        let dedicated = config::is_server() && crate::player_host::hosted_role().is_none();
        let cheats = Cheats::parse(value.as_deref()).for_process(dedicated);
        if cheats.any() {
            log::line(&test_cheats::lines::enabled(cheats));
        }
        cheats
    })
}

/// Where the sweep of the object array goes on next frame.
static SWEEP_AT: AtomicUsize = AtomicUsize::new(0);
/// Whether each class seen is a health set class; classes are never collected, so asked once a process.
static SET_CLASSES: LazyLock<Mutex<HashMap<usize, bool>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
/// Each Behemoth health set found, with its Behemoth and whether it has been held yet.
static SETS: LazyLock<Mutex<HashMap<usize, (usize, bool)>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
/// When the sets were last held.
static HELD_AT: Mutex<Option<std::time::Instant>> = Mutex::new(None);
static HOLD_LOGS: AtomicU32 = AtomicU32::new(0);
static COUNT_LOGS: AtomicU32 = AtomicU32::new(0);
static LAST_COUNT: AtomicUsize = AtomicUsize::new(0);

/// The Behemoth an attribute set belongs to: its outer, or that one's outer.
fn behemoth_owning(set: usize) -> Option<usize> {
    let mut object = set;
    for _ in 0..test_cheats::MAX_OWNER_STEPS {
        object = ptr_at(object + rustic_isles_runtime_core::offsets::layout::UOBJECT_OUTER);
        if !live(object) {
            return None;
        }
        if reflect::is_a(object, test_cheats::BEHEMOTH_CLASS) {
            return Some(object);
        }
    }
    None
}

/// This frame's share of the object array (a full scan in one frame stalled the server for up to 0.6 s), through
/// the census's region cache; new Behemoth health sets join `sets`.
fn sweep(sets: &mut HashMap<usize, (usize, bool)>) {
    let cache = super::census::RegionCache::default();
    let Some(engine) = engine::engine_over(&cache) else {
        return;
    };
    let Ok(mut classes) = SET_CLASSES.lock() else {
        return;
    };
    let mut found = Vec::new();
    let start = SWEEP_AT.load(Ordering::Relaxed);
    let next = rustic_isles_runtime_core::reflect::scan_slice(&engine, start, test_cheats::SWEEP_SLOTS, |header| {
        let class = header.class;
        let is_set = *classes.entry(class).or_insert_with(|| {
            rustic_isles_runtime_core::reflect::class_is_a(&engine, class, test_cheats::HEALTH_SET_CLASS)
        });
        if is_set && !sets.contains_key(&header.object) {
            found.push(header.object);
        }
    });
    SWEEP_AT.store(next, Ordering::Relaxed);
    if next == 0 && !classes.values().any(|is_set| *is_set) {
        static MISSING: AtomicBool = AtomicBool::new(false);
        if !MISSING.swap(true, Ordering::Relaxed) {
            log::line(test_cheats::lines::HEALTH_SET_MISSING);
        }
    }
    drop(classes);
    for set in found {
        if let Some(behemoth) = behemoth_owning(set) {
            sets.insert(set, (behemoth, false));
        }
    }
}

/// One-hit kills: every Behemoth's health is held at 1, so the next hit of any size kills it.
fn hold_behemoth_health(_engine: usize, _delta: f32) {
    if detaching() || !cheats().one_hit_kills {
        return;
    }
    let Ok(mut sets) = SETS.lock() else {
        return;
    };
    sweep(&mut sets);
    let now = std::time::Instant::now();
    let due = HELD_AT.lock().is_ok_and(|mut held_at| {
        let due = held_at.is_none_or(|at| now.duration_since(at) >= HOLD_EVERY);
        if due {
            *held_at = Some(now);
        }
        due
    });
    if !due {
        return;
    }
    sets.retain(|&set, &mut (behemoth, _)| live(set) && live(behemoth));
    if LAST_COUNT.swap(sets.len(), Ordering::Relaxed) != sets.len()
        && COUNT_LOGS.fetch_add(1, Ordering::Relaxed) < test_cheats::HOLD_LOG_BUDGET
    {
        log::line(&test_cheats::lines::health_sets(sets.len()));
    }
    for (&set, (behemoth, held)) in sets.iter_mut() {
        let span = test_cheats::MAX_HEALTH + 4 - test_cheats::HEALTH;
        if !readable_unaligned(set + test_cheats::HEALTH, span) {
            continue;
        }
        // Writes only when above the hold, so a held Behemoth costs two reads
        if read_unaligned::<f32>(set + test_cheats::SHIELD).is_some_and(|shield| shield > 0.0) {
            put::<f32>(set + test_cheats::SHIELD, 0.0);
        }
        let Some(before) = read_unaligned::<f32>(set + test_cheats::HEALTH) else {
            continue;
        };
        let Some(after) = test_cheats::hold(before) else {
            continue;
        };
        if !put::<f32>(set + test_cheats::HEALTH, after) {
            continue;
        }
        let first = !*held;
        *held = true;
        if HOLD_LOGS.fetch_add(1, Ordering::Relaxed) >= test_cheats::HOLD_LOG_BUDGET {
            continue;
        }
        let name = reflect::engine()
            .and_then(|engine| engine.object_name(*behemoth))
            .unwrap_or_else(|| "?".to_owned());
        log::line(&if first {
            let max = read_unaligned::<f32>(set + test_cheats::MAX_HEALTH).unwrap_or(f32::NAN);
            test_cheats::lines::health_held(&name, set, before, max)
        } else {
            test_cheats::lines::health_restored(&name, before)
        });
    }
}

const HOLD_EVERY: std::time::Duration = std::time::Duration::from_millis(test_cheats::HOLD_INTERVAL_MS);

static GAUNTLET_CHEATED: AtomicUsize = AtomicUsize::new(0);

/// The first state or level timer broadcast of each Gauntlet state component sets the game's own end-early flag.
fn gauntlet_broadcast(call: &PeCall) -> Verdict {
    if detaching()
        || !cheats().end_gauntlet_early
        || GAUNTLET_CHEATED.load(Ordering::Relaxed) == call.object
        || !live(call.object)
        || !reflect::is_a(call.object, test_cheats::GAUNTLET_STATE_CLASS)
    {
        return Verdict::Continue;
    }
    GAUNTLET_CHEATED.store(call.object, Ordering::Relaxed);
    let function = reflect::find_class(test_cheats::GAUNTLET_STATE_CLASS)
        .and_then(|class| reflect::function(class, test_cheats::END_GAUNTLET_EARLY));
    let Some(function) = function else {
        log::line(test_cheats::lines::GAUNTLET_CHEAT_MISSING);
        return Verdict::Continue;
    };
    let mut params = reflect::Params::new(0);
    let called = reflect::call(call.object, function, &mut params);
    log::line(&test_cheats::lines::gauntlet_ended_early(call.object, called));
    Verdict::Continue
}

// ---- 5: the match start at the first login ----

static LAST_STARTED: AtomicUsize = AtomicUsize::new(0);

fn start_match_function() -> Option<usize> {
    static FOUND: OnceLock<Option<usize>> = OnceLock::new();
    *FOUND.get_or_init(|| {
        let found = reflect::find_class("GameMode").and_then(|class| reflect::function(class, "StartMatch"));
        log::line(if found.is_some() {
            lines::START_MATCH_RESOLVED
        } else {
            lines::START_MATCH_MISSING
        });
        found
    })
}

/// Steps 1 to 6; true when the call reached step 7 (an earlier stop returns at once).
fn start_match(game_mode: usize) -> bool {
    if !crate::headless::boot_patch_applied()
        || !live(game_mode)
        || !readable(game_mode, r20::GAME_MODE_MATCH_STATE_SPAN)
        || !reflect::is_a(game_mode, "GameMode")
    {
        return false;
    }
    let state = reflect::engine().and_then(|engine| engine.fname_at(game_mode + r20::GAME_MODE_MATCH_STATE));
    if !core::waiting_to_start(state.as_deref()) || LAST_STARTED.load(Ordering::Relaxed) == game_mode {
        return false;
    }
    let Some(function) = start_match_function() else {
        return false;
    };
    LAST_STARTED.store(game_mode, Ordering::Relaxed);
    let mut params = reflect::Params::new(0);
    let called = reflect::call(game_mode, function, &mut params);
    let class = reflect::class_name_of(game_mode).unwrap_or_else(|| "?".to_owned());
    log::line(&lines::start_match(game_mode, &class, called));
    true
}

fn post_login(call: &PeCall) -> Verdict {
    if detaching() {
        return Verdict::Continue;
    }
    if start_match(call.object) && !flags::production_log_profile() {
        log::line(&lines::post_login(call.object, params_ptr(call.params)));
    }
    Verdict::Continue
}

// ---- 6: the ClientRestart pawn fixes ----

static OWNER_FIX_LOGS: AtomicU32 = AtomicU32::new(0);

fn owner_fix_on() -> bool {
    static DISABLED: OnceLock<bool> = OnceLock::new();
    !*DISABLED.get_or_init(|| flags::flag(names::DISABLE_OWNER_PAWN_RELEVANCY))
}

fn pawn_fields(pawn: usize) -> Option<PawnFields> {
    Some(PawnFields {
        relevancy: read_unaligned::<u8>(pawn + r20::ACTOR_RELEVANCY_FLAGS)?,
        dormancy: read_unaligned::<u8>(pawn + r20::ACTOR_NET_DORMANCY)?,
        update_frequency: read_unaligned::<f32>(pawn + r20::ACTOR_NET_UPDATE_FREQUENCY)?,
        priority: read_unaligned::<f32>(pawn + r20::ACTOR_NET_PRIORITY)?,
        remote_role: read_unaligned::<u8>(pawn + r20::ACTOR_REMOTE_ROLE)?,
    })
}

fn fix_pawn(pawn: usize, owner_fix: bool) {
    if !live(pawn) || !readable(pawn, r20::PAWN_SCAN_SPAN) {
        return;
    }
    let Some(read) = pawn_fields(pawn) else {
        return;
    };
    let plan = core::pawn_plan(read, owner_fix);
    if plan.relevancy != read.relevancy {
        put::<u8>(pawn + r20::ACTOR_RELEVANCY_FLAGS, plan.relevancy);
    }
    if plan.dormancy != read.dormancy {
        put::<u8>(pawn + r20::ACTOR_NET_DORMANCY, plan.dormancy);
    }
    if plan.update_frequency.to_bits() != read.update_frequency.to_bits() {
        put::<f32>(pawn + r20::ACTOR_NET_UPDATE_FREQUENCY, plan.update_frequency);
    }
    if plan.priority.to_bits() != read.priority.to_bits() {
        put::<f32>(pawn + r20::ACTOR_NET_PRIORITY, plan.priority);
    }
    if plan.remote_role != read.remote_role {
        put::<u8>(pawn + r20::ACTOR_REMOTE_ROLE, plan.remote_role);
    }
}

fn point_view_at_pawn(controller: usize, pawn: usize) {
    if !readable(controller, r20::PC_SPAN) {
        return;
    }
    let connection = ptr_at(controller + r20::PC_NET_CONNECTION);
    if connection == 0 || !readable(connection, r20::CONN_VIEW_SPAN) {
        return;
    }
    let old = ptr_at(connection + r20::CONN_VIEW_TARGET);
    if old == pawn || !put::<usize>(connection + r20::CONN_VIEW_TARGET, pawn) {
        return;
    }
    if OWNER_FIX_LOGS.fetch_add(1, Ordering::Relaxed) < core::OWNER_FIX_LOG_BUDGET {
        log::line(&lines::owner_fix(connection, old, pawn));
    }
}

fn client_restart(call: &PeCall) -> Verdict {
    if detaching() {
        return Verdict::Continue;
    }
    let Some(pawn) = params_ptr(call.params) else {
        return Verdict::Continue;
    };
    let owner_fix = owner_fix_on();
    if pawn != 0 {
        fix_pawn(pawn, owner_fix);
        if owner_fix {
            point_view_at_pawn(call.object, pawn);
        }
    }
    Verdict::Continue
}

// ---- 7: the vendor camera ----

static VENDOR_ABSORBS: AtomicU64 = AtomicU64::new(0);

fn vendor(_: &PeCall) -> Verdict {
    if detaching() {
        return Verdict::Continue;
    }
    let slot = VENDOR_ABSORBS.fetch_add(1, Ordering::Relaxed);
    if core::vendor_logs(slot) {
        log::line(&lines::vendor(slot));
    }
    Verdict::Absorb
}

// ---- 8: PlayerCanRestart ----

static FORCED_LOGGED: AtomicBool = AtomicBool::new(false);

fn connection_of(player: usize) -> Option<usize> {
    if player == 0 || !readable(player, r20::PC_SPAN) {
        return None;
    }
    let from_418 = ptr_at(player + r20::PC_NET_CONNECTION);
    let from_298 = ptr_at(player + r20::PC_PLAYER);
    let class = (from_418 == 0 && from_298 != 0)
        .then(|| reflect::class_name_of(from_298))
        .flatten();
    core::choose_connection(from_418, from_298, class.as_deref())
}

/// 8.2 step 2: the 8 name bytes of the top of the player's outer chain (the map's package).
fn world_package_name(player: usize) -> u64 {
    let top = core::walk_outers(player, |object| {
        readable(object, r20::OUTER_SPAN)
            .then(|| ptr_at(object + rustic_isles_runtime_core::offsets::layout::UOBJECT_OUTER))
    });
    if !readable(top, r20::NAME_SPAN) {
        return 0;
    }
    read::<u64>(top + rustic_isles_runtime_core::offsets::layout::UOBJECT_NAME).unwrap_or(0)
}

fn set_client_world_package_name(connection: usize, name: u64) {
    static ENTRY: OnceLock<Option<usize>> = OnceLock::new();
    let entry = *ENTRY.get_or_init(|| {
        let rva = r20::SET_CLIENT_WORLD_PACKAGE_NAME;
        (super::memory::function_entry(rva) == Some(rva))
            .then(|| win::image_base().checked_add(usize::try_from(rva).ok()?))
            .flatten()
    });
    let Some(entry) = entry else {
        return;
    };
    // SAFETY: the setter's entry in this build (RT-2, checked against `.pdata`), on a live `NetConnection`
    // readable through the field it stores, with an `FName` passed by value as its 8 bytes
    unsafe {
        let setter = std::mem::transmute::<usize, SetPackageNameFn>(entry);
        setter(connection as *mut c_void, name);
    }
}

fn fix_connection(player: usize, connection: usize) {
    let name = world_package_name(player);
    if name != 0
        && live(connection)
        && reflect::is_a(connection, "NetConnection")
        && readable(connection, r20::CONN_PACKAGE_SPAN)
    {
        set_client_world_package_name(connection, name);
    }
    if readable(connection, r20::CONN_SPAN) {
        let owning = ptr_at(connection + r20::CONN_OWNING_ACTOR);
        let mut view = owning;
        if owning != 0 && readable(owning, r20::CONTROLLER_SPAN) {
            let pawn = ptr_at(owning + r20::CONTROLLER_PAWN);
            if pawn != 0 {
                view = pawn;
            }
        }
        // A null owner writes null, as the AGPL did (ambiguity 1)
        put::<usize>(connection + r20::CONN_VIEW_TARGET, view);
    }
}

fn player_can_restart(call: &PeCall) -> Verdict {
    if detaching() || !readable_unaligned(call.params, r20::CAN_RESTART_PARAMS) {
        return Verdict::Continue;
    }
    let ret = call.params + r20::CAN_RESTART_RETURN;
    let (Some(player), Some(prior)) = (read_unaligned::<usize>(call.params), read_unaligned::<u8>(ret)) else {
        return Verdict::Continue;
    };
    if !writable(ret, 1) {
        return Verdict::Continue;
    }
    if let Some(connection) = connection_of(player) {
        fix_connection(player, connection);
    }
    if restart_gate::hold(call.object, player) {
        put::<u8>(ret, 0);
        return Verdict::Absorb;
    }
    put::<u8>(ret, 1);
    if !FORCED_LOGGED.swap(true, Ordering::Relaxed) {
        log::line(&lines::forced_true(player, prior));
    }
    Verdict::Absorb
}

// ---- 10: connected players ----

struct Tick {
    floor_done: bool,
    announced: bool,
    tick_stamina: HashMap<usize, usize>,
}

static TICK: LazyLock<Mutex<Tick>> = LazyLock::new(|| {
    Mutex::new(Tick {
        floor_done: false,
        announced: false,
        tick_stamina: HashMap::new(),
    })
});

/// 10.1: R9's driver, else the world's.
fn driver() -> Option<usize> {
    if let Some(driver) = crate::listen::net_driver() {
        return Some(driver);
    }
    let world = raw_listen::gworld()?;
    if !readable(world, r20::WORLD_DRIVER_SPAN) {
        return None;
    }
    Some(ptr_at(world + r20::WORLD_NET_DRIVER)).filter(|&driver| live(driver))
}

fn player_pawn(connection: usize) -> Option<usize> {
    if !readable_this_frame(connection, r20::CONN_SCAN_SPAN) {
        return None;
    }
    let controller = ptr_at(connection + r20::CONN_PLAYER_CONTROLLER);
    if controller == 0 || !readable(controller, r20::CONTROLLER_SPAN) {
        return None;
    }
    let pawn = ptr_at(controller + r20::CONTROLLER_PAWN);
    (live(pawn) && reflect::is_a(pawn, "BP_PlayerCharacter_C")).then_some(pawn)
}

/// 10.2; true once done.
fn bleedout_floor() -> bool {
    let Some(world) = raw_listen::gworld() else {
        return false;
    };
    if !readable(world, rustic_isles_runtime_core::offsets::r8::WORLD_SPAN) {
        return false;
    }
    let gm = ptr_at(world + r20::WORLD_AUTHORITY_GAME_MODE);
    if !live(gm) || !readable(gm, r20::ISLAND_BLEEDOUT_SPAN) {
        return false;
    }
    if !reflect::is_a(gm, "ArchonGameMode_Island") {
        return true;
    }
    let field = gm + r20::ISLAND_BLEEDOUT_DURATION;
    let Some(duration) = read::<f32>(field) else {
        return false;
    };
    if !core::needs_floor(duration) {
        log::line(&lines::bleedout_ok(duration));
        return true;
    }
    let fallback =
        core::bleedout_fallback(win::env_value(vars::BLEEDOUT_SECONDS.0, vars::BLEEDOUT_SECONDS.1).as_deref());
    if put::<f32>(field, fallback) {
        log::line(&lines::bleedout_set(duration, fallback));
    }
    true
}

fn tick_stamina(tick: &mut Tick, pawn: usize) {
    let Some(class) = reflect::class_of(pawn) else {
        return;
    };
    let function = *tick
        .tick_stamina
        .entry(class)
        .or_insert_with(|| reflect::function(class, "TickStamina").unwrap_or(0));
    if function == 0 {
        return;
    }
    let mut params = reflect::Params::new(r20::TICK_STAMINA_PARAMS);
    params.put(0, &[core::EXEC_BOTH, core::REMOTE_ALL]);
    reflect::call(pawn, function, &mut params);
}

/// The SP-3 after handler, order 650 (10).
fn connected_players(_engine: usize, _delta: f32) {
    if detaching() {
        return;
    }
    let Ok(mut tick) = TICK.try_lock() else {
        return;
    };
    let Some(driver) = driver() else {
        return;
    };
    if !tick.floor_done && config::launch().is_some_and(|launch| launch.watchdog) {
        tick.floor_done = bleedout_floor();
    }
    let pawns: Vec<usize> = crate::listen::client_connections(driver)
        .into_iter()
        .filter_map(player_pawn)
        .collect();
    if pawns.is_empty() {
        return;
    }
    if !tick.announced {
        tick.announced = true;
        log::line(lines::STAMINA_MANUAL);
    }
    for pawn in pawns {
        tick_stamina(&mut tick, pawn);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustic_isles_runtime_core::names::matches;

    #[test]
    fn registers_on_the_server_only() {
        let (pe, tick) = table(false, |_| true);
        assert!(pe.is_empty() && tick.is_none(), "nothing on a client");
        let (pe, tick) = table(true, |_| true);
        let shape: Vec<_> = pe
            .iter()
            .map(|handler| {
                (
                    handler.slice,
                    handler.order,
                    handler.before.is_some(),
                    handler.after.is_some(),
                )
            })
            .collect();
        assert_eq!(
            shape,
            [
                ("R20", -500, true, false),
                ("R20", 0, true, false),
                ("R20", 0, true, false),
                ("R20", 0, true, false),
                ("R20", 0, true, false),
                ("R20", 0, true, false),
            ]
        );
        let names = [
            "Function Archon.ArchonBehemoth.ReceiveArchonDamage",
            "Function Engine.GameModeBase.K2_PostLogin",
            "Function Engine.PlayerController.ClientRetryClientRestart",
            "Function vendor_interactive_bp.vendor_interactive_bp_C.UpdateCameraForAspect",
            "Function Engine.GameModeBase.PlayerCanRestart",
            "Function Archon.GauntletGameStateComponent.BroadcastStateTimerToClients",
        ];
        for (handler, name) in pe.iter().zip(names) {
            assert!(matches(handler.matcher, Some(name)), "{name}");
        }
        let tick = tick.unwrap();
        assert_eq!((tick.slice, tick.order), ("R20", 650));
        assert!(tick.before.is_none() && tick.after.is_some());
        let (pe, tick) = table(true, |id| id == b"SP-3@R20");
        assert!(pe.is_empty() && tick.is_some(), "each id alone");
    }
}
