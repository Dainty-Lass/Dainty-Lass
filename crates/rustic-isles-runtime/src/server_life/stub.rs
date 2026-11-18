//! Server stubs (spec-runtime-stub): with `RUNTIME_STUB_EFFECTS.flag`, R8's after handler sweeps the object array and
//! deactivates and unticks every particle component placed or spawned in a level.

use crate::ffi::census::RegionCache;
use crate::ffi::memory::read;
use crate::ffi::{engine, flags, log, reflect, win};
use rustic_isles_runtime_core::census::{Item, visit};
use rustic_isles_runtime_core::reflect as core_reflect;
use rustic_isles_runtime_core::sim::{check_layout, property_layout};
use rustic_isles_runtime_core::stub::{self as core, Sweep, layouts};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Instant;

struct Stub {
    enabled: bool,
    start: u64,
    last: Option<u64>,
    first: bool,
    calls: Option<Option<Calls>>,
    particle: HashMap<usize, bool>,
    level: HashMap<usize, bool>,
    unticked: HashSet<usize>,
    reported: Option<(u64, usize)>,
    totals_at: Option<u64>,
}

#[derive(Clone, Copy)]
struct Calls {
    is_active: usize,
    deactivate: usize,
    set_tick: usize,
    replicates: (usize, u8),
}

static STUB: Mutex<Option<Stub>> = Mutex::new(None);

/// The step, from R8's after handler while listening with a world.
pub(super) fn step() {
    let Ok(mut guard) = STUB.try_lock() else {
        return;
    };
    let now = win::tick();
    let stub = guard.get_or_insert_with(|| {
        let enabled = flags::flag(core::EFFECTS_FLAG);
        if enabled {
            log::line(core::ON_LINE);
        }
        Stub {
            enabled,
            start: now,
            last: None,
            first: true,
            calls: None,
            particle: HashMap::new(),
            level: HashMap::new(),
            unticked: HashSet::new(),
            reported: None,
            totals_at: None,
        }
    });
    let totals = crate::ffi::packages::totals();
    if totals.0 > 0 && stub.reported != Some(totals) && stub.totals_at.is_none_or(|at| now.saturating_sub(at) >= 60_000)
    {
        stub.reported = Some(totals);
        stub.totals_at = Some(now);
        log::line(&core::refusals_line(totals.0, totals.1));
    }
    if !stub.enabled || !core::due(stub.start, stub.last, now) || !engine::names_enabled() {
        return;
    }
    stub.last = Some(now);
    let Some(calls) = *stub.calls.get_or_insert_with(resolve) else {
        stub.enabled = false;
        return;
    };
    let sweep = sweep(stub, calls);
    if let Some(line) = core::sweep_line(&sweep, stub.first) {
        log::line(&line);
    }
    stub.first = false;
}

/// Particle components whose outer chain reaches a level, templates excluded (2.1).
fn candidates(stub: &mut Stub) -> Vec<usize> {
    let memory = RegionCache::default();
    let Some(engine) = engine::engine_over(&memory) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let Stub { particle, level, .. } = stub;
    let mut level_memo: HashMap<usize, bool> = HashMap::new();
    visit(&memory, engine.objects(), |item| {
        let Item::Object {
            object,
            class,
            outer,
            flags,
        } = item
        else {
            return;
        };
        if flags & core::TEMPLATE_FLAGS != 0 {
            return;
        }
        let is_particle = *particle
            .entry(class)
            .or_insert_with(|| core_reflect::class_is_a(&engine, class, "ParticleSystemComponent"));
        if !is_particle {
            return;
        }
        let mut is_level = |class: usize| {
            *level
                .entry(class)
                .or_insert_with(|| core_reflect::class_is_a(&engine, class, "Level"))
        };
        if core::in_level(&memory, outer, &mut is_level, &mut level_memo) {
            found.push(object);
        }
    });
    found
}

/// The three functions and the replication bit, resolved once on `ActorComponent` (2.3); `None` turns the sweep off.
fn resolve() -> Option<Calls> {
    let class = reflect::find_class("ActorComponent")?;
    let engine = reflect::engine()?;
    let function = |name: &'static str, expected: &[(&str, usize)]| {
        let Some(function) = reflect::function(class, name) else {
            log::line(&core::missing_line(name));
            return None;
        };
        let found = property_layout(&engine, function);
        match check_layout(&format!("ActorComponent.{name}"), expected, found.as_deref()) {
            Ok(()) => Some(function),
            Err(line) => {
                log::line(&line.replace("[Sim]", "[Stub]"));
                None
            }
        }
    };
    let is_active = function("IsActive", layouts::RETURN);
    let deactivate = function("Deactivate", layouts::NONE);
    let set_tick = function("SetComponentTickEnabled", layouts::TICK);
    let replicates = core::bool_property(&engine, class, "bReplicates");
    log::line(&core::replicates_line(replicates));
    Some(Calls {
        is_active: is_active?,
        deactivate: deactivate?,
        set_tick: set_tick?,
        replicates: replicates?,
    })
}

fn sweep(stub: &mut Stub, calls: Calls) -> Sweep {
    let started = Instant::now();
    let found = candidates(stub);
    let mut sweep = Sweep {
        components: found.len(),
        ..Sweep::default()
    };
    let live: HashSet<usize> = found.iter().copied().collect();
    stub.unticked.retain(|component| live.contains(component));
    let (offset, mask) = calls.replicates;
    for component in found {
        if !engine::is_live_object(component) {
            continue;
        }
        // A replicated component's state reaches clients: leave their effects alone
        match read::<u8>(component + offset) {
            Some(byte) if byte & mask == 0 => {}
            Some(_) => {
                sweep.replicated += 1;
                continue;
            }
            None => continue,
        }
        let mut params = reflect::Params::new(1);
        let active = reflect::call(component, calls.is_active, &mut params)
            && params.get(0, 1).first().is_some_and(|&byte| byte != 0);
        if active && reflect::call(component, calls.deactivate, &mut reflect::Params::new(0)) {
            sweep.stopped += 1;
        }
        if !stub.unticked.contains(&component) && reflect::call(component, calls.set_tick, &mut reflect::Params::new(1))
        {
            stub.unticked.insert(component);
            sweep.unticked += 1;
        }
    }
    sweep.ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    sweep
}
