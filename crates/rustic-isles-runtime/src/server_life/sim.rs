//! Simulated players (spec-runtime-sim): with `RUNTIME_SIM_PLAYERS.flag`, once a second on R8's after handler, the
//! port's command file is read and bots are spawned, wandered and removed through Blueprint-callable engine functions.

use crate::config;
use crate::ffi::{engine, flags, log, reflect};
use rustic_isles_runtime_core::sim::{self as core, Wander, layouts};
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

static BOTS: AtomicUsize = AtomicUsize::new(0);

const LIBRARY: &str = "AIBlueprintHelperLibrary";

struct Sim {
    started: bool,
    enabled: bool,
    last_poll: Option<u64>,
    target: u32,
    class: String,
    wander_s: u32,
    last_wander: Option<u64>,
    rng: Option<Wander>,
    origin: Option<Option<[f32; 3]>>,
    bots: Vec<(usize, usize)>,
    failures: u32,
    checked: HashMap<String, bool>,
    logged_classes: Vec<String>,
}

static SIM: Mutex<Option<Sim>> = Mutex::new(None);

/// The connection count R8's pacer sees: at least one while bots run (call 5).
pub fn pacing_count(connections: i32) -> i32 {
    if BOTS.load(Ordering::Relaxed) > 0 {
        connections.max(1)
    } else {
        connections
    }
}

/// The step, from R8's after handler while listening with a world (`now` is the tick clock).
pub(super) fn step(world: usize, now: u64) {
    let Ok(mut guard) = SIM.try_lock() else {
        return;
    };
    let sim = guard.get_or_insert_with(|| Sim {
        started: false,
        enabled: false,
        last_poll: None,
        target: 0,
        class: core::DEFAULT_CLASS.to_owned(),
        wander_s: core::DEFAULT_WANDER,
        last_wander: None,
        rng: None,
        origin: None,
        bots: Vec::new(),
        failures: 0,
        checked: HashMap::new(),
        logged_classes: Vec::new(),
    });
    if !sim.started {
        sim.started = true;
        sim.enabled = flags::flag(core::FLAG);
        if sim.enabled {
            log::line(&core::on_line(config::port()));
        }
    }
    if !sim.enabled
        || sim
            .last_poll
            .is_some_and(|last| now.saturating_sub(last) < core::POLL_MS)
    {
        return;
    }
    sim.last_poll = Some(now);
    read_commands(sim);
    let before = sim.bots.len();
    sim.bots.retain(|&(_, pawn)| engine::is_live_object(pawn));
    let target = usize::try_from(sim.target).unwrap_or(0);
    if sim.bots.len() < target {
        fill(sim, world);
    } else if sim.bots.len() > target {
        remove(sim);
    }
    let after = sim.bots.len();
    BOTS.store(after, Ordering::Relaxed);
    if after != before {
        let change = i64::try_from(after).unwrap_or(0) - i64::try_from(before).unwrap_or(0);
        log::line(&core::count_line(after, sim.target, change));
    }
    wander(sim, now);
}

fn read_commands(sim: &mut Sim) {
    let name = core::file_name(config::port());
    let Some(path) = flags::path_next_to_exe(&name).filter(|path| path.is_file()) else {
        return;
    };
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    // Removed before it is acted on, as the kick file (spec-runtime-kick 3)
    _ = std::fs::remove_file(&path);
    let commands = core::parse(&text);
    if let Some((target, class)) = commands.bots {
        sim.target = target;
        if let Some(class) = class {
            sim.class = class;
        }
        sim.failures = 0;
    }
    if let Some(seconds) = commands.wander {
        sim.wander_s = seconds;
    }
    log::line(&core::read_line(
        &name,
        sim.target,
        &sim.class,
        sim.wander_s,
        commands.ignored,
    ));
}

/// The function `name` of `object`'s class, once its parameters passed the layout check (2.2).
fn checked(sim: &mut Sim, object: usize, name: &str, expected: &[(&str, usize)]) -> Option<usize> {
    let class = reflect::class_of(object)?;
    let function = reflect::function(class, name)?;
    let label = format!(
        "{}.{name}",
        reflect::class_name_of(object).unwrap_or_else(|| "?".to_owned())
    );
    let ok = *sim.checked.entry(name.to_owned()).or_insert_with(|| {
        let found = reflect::engine().and_then(|engine| core::property_layout(&engine, function));
        match core::check_layout(&label, expected, found.as_deref()) {
            Ok(()) => true,
            Err(line) => {
                log::line(&line);
                false
            }
        }
    });
    ok.then_some(function)
}

fn floats(params: &reflect::Params, offset: usize) -> [f32; 3] {
    let bytes = params.get(offset, 12);
    let mut out = [0f32; 3];
    for (i, value) in out.iter_mut().enumerate() {
        let mut raw = [0u8; 4];
        raw.copy_from_slice(&bytes[i * 4..i * 4 + 4]);
        *value = f32::from_le_bytes(raw);
    }
    out
}

fn put_floats(params: &mut reflect::Params, offset: usize, values: [f32; 3]) {
    for (i, value) in values.iter().enumerate() {
        params.put(offset + i * 4, &value.to_le_bytes());
    }
}

/// The first `PlayerStart`'s location, found once per process (4.2).
fn origin(sim: &mut Sim) -> Option<[f32; 3]> {
    if let Some(origin) = sim.origin {
        return origin;
    }
    let found = reflect::first_instance("PlayerStart").and_then(|start| {
        let function = checked(sim, start, "K2_GetActorLocation", layouts::RETURN)?;
        let mut params = reflect::Params::new(12);
        reflect::call(start, function, &mut params).then(|| floats(&params, 0))
    });
    if found.is_none() {
        log::line(core::NO_START_LINE);
    }
    sim.origin = Some(found);
    found
}

fn library() -> Option<usize> {
    reflect::find_class(LIBRARY).and_then(reflect::default_object)
}

fn lower_target(sim: &mut Sim) {
    sim.target = u32::try_from(sim.bots.len()).unwrap_or(0);
}

fn fill(sim: &mut Sim, world: usize) {
    let Some(pawn_class) = reflect::find_class(&sim.class) else {
        if !sim.logged_classes.contains(&sim.class) {
            sim.logged_classes.push(sim.class.clone());
            log::line(&core::class_line(&sim.class));
        }
        lower_target(sim);
        return;
    };
    let Some(origin) = origin(sim) else {
        lower_target(sim);
        return;
    };
    let Some(library) = library() else {
        lower_target(sim);
        return;
    };
    let Some(spawn) = checked(sim, library, "SpawnAIFromClass", layouts::SPAWN) else {
        lower_target(sim);
        return;
    };
    let occupied: Vec<usize> = sim.bots.iter().map(|&(slot, _)| slot).collect();
    for slot in core::slots_to_fill(&occupied, sim.target) {
        let mut params = reflect::Params::new(layouts::SPAWN_SIZE);
        params.put_ptr(0x00, world);
        params.put_ptr(0x08, pawn_class);
        put_floats(&mut params, 0x18, core::ring(origin, slot));
        params.put(0x30, &[1]);
        let pawn = reflect::call(library, spawn, &mut params)
            .then(|| params.get_ptr(0x40))
            .filter(|&pawn| pawn != 0 && engine::is_live_object(pawn));
        match pawn {
            Some(pawn) => {
                sim.failures = 0;
                sim.bots.push((slot, pawn));
            }
            None => {
                sim.failures += 1;
                if sim.failures >= core::FAIL_STREAK {
                    sim.failures = 0;
                    lower_target(sim);
                    log::line(&core::streak_line(sim.bots.len()));
                    return;
                }
            }
        }
    }
}

fn controller_of(sim: &mut Sim, pawn: usize) -> Option<usize> {
    let function = checked(sim, pawn, "GetController", layouts::RETURN)?;
    let mut params = reflect::Params::new(8);
    reflect::call(pawn, function, &mut params)
        .then(|| params.get_ptr(0))
        .filter(|&controller| controller != 0 && engine::is_live_object(controller))
}

fn destroy(sim: &mut Sim, actor: usize) {
    if let Some(function) = checked(sim, actor, "K2_DestroyActor", layouts::NONE) {
        let mut params = reflect::Params::new(0);
        reflect::call(actor, function, &mut params);
    }
}

fn remove(sim: &mut Sim) {
    let occupied: Vec<usize> = sim.bots.iter().map(|&(slot, _)| slot).collect();
    for slot in core::slots_to_remove(&occupied, sim.target) {
        let Some(index) = sim.bots.iter().position(|&(have, _)| have == slot) else {
            continue;
        };
        let (_, pawn) = sim.bots.remove(index);
        if let Some(controller) = controller_of(sim, pawn) {
            destroy(sim, controller);
        }
        if engine::is_live_object(pawn) {
            destroy(sim, pawn);
        }
    }
}

/// 4.3: a new goal for every bot each `wander_s` seconds.
fn wander(sim: &mut Sim, now: u64) {
    if sim.wander_s == 0 || sim.bots.is_empty() {
        return;
    }
    let period = u64::from(sim.wander_s) * 1000;
    if sim.last_wander.is_some_and(|last| now.saturating_sub(last) < period) {
        return;
    }
    sim.last_wander = Some(now);
    let Some(Some(origin)) = sim.origin else {
        return;
    };
    let Some(library) = library() else {
        return;
    };
    let Some(mover) = checked(sim, library, "SimpleMoveToLocation", layouts::MOVE) else {
        return;
    };
    let pawns: Vec<usize> = sim.bots.iter().map(|&(_, pawn)| pawn).collect();
    for pawn in pawns {
        let Some(controller) = controller_of(sim, pawn) else {
            continue;
        };
        let goal = sim.rng.get_or_insert_with(|| Wander::new(now)).goal(origin);
        let mut params = reflect::Params::new(layouts::MOVE_SIZE);
        params.put_ptr(0x00, controller);
        put_floats(&mut params, 0x08, goal);
        reflect::call(library, mover, &mut params);
    }
}
