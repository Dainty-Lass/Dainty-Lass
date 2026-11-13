//! Runtime slice R16, travel (spec-runtime-r16): the leftover-world pass that clears the stuck `Async` keep flag a
//! left map's input component keeps, so `UEngine::LoadMap` never reuses a pending-kill world on a second arrival;
//! the world-change watcher and the opt-in guard. Pure: the DLL reads `GWorld`, does the atomic clear and logs.

use crate::engine::Engine;
use crate::memory::Memory;
use crate::offsets::layout;
use std::collections::HashMap;
use std::ops::ControlFlow;

/// R16's flag files (RT-14 for R16).
pub mod flag_names {
    /// Report only: the revert switch for the release.
    pub const KEEP_STUCK_ASYNC_FLAGS: &str = "KEEP_STUCK_ASYNC_FLAGS";
    /// The guard of 4.3.
    pub const ENABLE_ZOMBIE_LOADMAP_GUARD: &str = "ENABLE_ZOMBIE_LOADMAP_GUARD";
    /// TR-1 not installed.
    pub const NO_LOADMAP_PROBE: &str = "NO_LOADMAP_PROBE";
}

/// The world class's full name (call 1).
pub const WORLD_CLASS: &str = "Class Engine.World";
const WORLD_CLASS_SHORT: &str = "World";
/// Leftovers one pass collects.
pub const MAX_LEFTOVERS: usize = 64;
/// `SuperStruct` steps from a class to the world class.
pub const MAX_SUPER_STEPS: usize = 16;
/// Outers checked above an object (the object itself is checked too).
pub const MAX_OUTERS: usize = 16;
/// Worlds each ring remembers.
pub const RING_SIZE: usize = 16;
/// Objects a release line names.
pub const MAX_LISTED: usize = 8;
/// The watcher's slot.
pub const WATCH_INTERVAL_MS: u64 = 250;
/// The first arrival pass after a world change.
pub const ARRIVAL_FIRST_MS: u64 = 10_000;
/// The last one.
pub const ARRIVAL_LAST_MS: u64 = 60_000;

const ELIGIBLE_MASK: u32 =
    layout::ITEM_ROOT_SET | layout::ITEM_PENDING_KILL | layout::ITEM_ASYNC_LOADING | layout::ITEM_ASYNC;
const ELIGIBLE: u32 = layout::ITEM_PENDING_KILL | layout::ITEM_ASYNC;

/// Why a pass runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// TR-1's entry.
    LoadMapEnter,
    /// The watcher saw `GWorld` change.
    WorldChange,
    /// 10 s after it.
    Arrival10s,
    /// 60 s after it.
    Arrival60s,
}

impl Reason {
    /// The log text.
    pub fn text(self) -> &'static str {
        match self {
            Reason::LoadMapEnter => "loadmap-enter",
            Reason::WorldChange => "world-change",
            Reason::Arrival10s => "arrival+10s",
            Reason::Arrival60s => "arrival+60s",
        }
    }
}

/// Object names as the pass needs them.
pub trait Names {
    /// The short object name (R0).
    fn short_name(&self, object: usize) -> Option<String>;
    /// The full name (R0 6.2).
    fn full_name(&self, object: usize) -> Option<String>;
}

impl Names for Engine<'_> {
    fn short_name(&self, object: usize) -> Option<String> {
        self.object_name(object)
    }

    fn full_name(&self, object: usize) -> Option<String> {
        Engine::full_name(self, object)
    }
}

/// An item that may be a leftover: pending-kill.
pub fn is_pending_kill(flags: u32) -> bool {
    flags & layout::ITEM_PENDING_KILL != 0
}

/// An item whose `Async` may be cleared: pending-kill and `Async`, neither rooted nor still loading.
pub fn is_eligible(flags: u32) -> bool {
    flags & ELIGIBLE_MASK == ELIGIBLE
}

/// One `FUObjectItem`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Item {
    /// The item's address.
    pub address: usize,
    /// Its object (may be null).
    pub object: usize,
    /// Its internal flags.
    pub flags: u32,
}

impl Item {
    /// Where its flags word lives (the atomic clear's target).
    pub fn flags_address(&self) -> usize {
        self.address.wrapping_add(layout::OBJECT_ITEM_FLAGS)
    }
}

/// Walks the object array chunk by chunk with one check per chunk (3.5): an unreadable or null chunk is skipped,
/// a count past `MAX_OBJECTS` walks nothing. False when the array header itself could not be read.
pub fn walk(memory: &dyn Memory, objects: usize, visit: &mut dyn FnMut(Item) -> ControlFlow<()>) -> bool {
    let header = |offset: usize| objects.checked_add(offset).and_then(|at| memory.i32_at(at));
    let Some(count) = header(layout::OBJECTS_COUNT).filter(|count| (0..=layout::MAX_OBJECTS).contains(count)) else {
        return false;
    };
    let Some(chunks) = header(layout::OBJECTS_CHUNK_COUNT).and_then(|chunks| usize::try_from(chunks).ok()) else {
        return false;
    };
    let Some(table) = objects
        .checked_add(layout::OBJECTS_CHUNKS)
        .and_then(|at| memory.ptr_at(at))
    else {
        return false;
    };
    let Ok(count) = usize::try_from(count) else {
        return false;
    };
    let needed = count.div_ceil(layout::OBJECTS_PER_CHUNK).min(chunks);
    if needed == 0 {
        return true;
    }
    if !memory.readable(table, needed * 8) {
        return false;
    }
    let mut span = Vec::new();
    for chunk in 0..needed {
        let Some(items) = memory.ptr_at(table + chunk * 8).filter(|&items| items != 0) else {
            continue;
        };
        let in_chunk = (count - chunk * layout::OBJECTS_PER_CHUNK).min(layout::OBJECTS_PER_CHUNK);
        let size = in_chunk * layout::OBJECT_ITEM_SIZE;
        if items.checked_add(size).is_none() || !memory.readable(items, size) {
            continue;
        }
        span.resize(size, 0);
        if !memory.read(items, &mut span) {
            continue;
        }
        for (index, raw) in span.as_chunks::<{ layout::OBJECT_ITEM_SIZE }>().0.iter().enumerate() {
            let object = u64::from_le_bytes(raw[..8].try_into().unwrap_or_default());
            let flags_at = layout::OBJECT_ITEM_FLAGS;
            let flags = u32::from_le_bytes(raw[flags_at..flags_at + 4].try_into().unwrap_or_default());
            let item = Item {
                address: items + index * layout::OBJECT_ITEM_SIZE,
                object: usize::try_from(object).unwrap_or(0),
                flags,
            };
            if visit(item).is_break() {
                return true;
            }
        }
    }
    true
}

/// The object whose full name is `Class Engine.World` (3.2 step 1).
pub fn find_world_class(memory: &dyn Memory, objects: usize, names: &dyn Names) -> Option<usize> {
    let mut found = None;
    walk(memory, objects, &mut |item| {
        if item.object != 0
            && names.short_name(item.object).as_deref() == Some(WORLD_CLASS_SHORT)
            && names.full_name(item.object).as_deref() == Some(WORLD_CLASS)
        {
            found = Some(item.object);
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    });
    found
}

/// Whether a class is the world class or reaches it through at most 16 `SuperStruct` steps.
fn reaches_world(memory: &dyn Memory, class: usize, world_class: usize) -> bool {
    let mut current = class;
    for step in 0..=MAX_SUPER_STEPS {
        if current == 0 {
            return false;
        }
        if current == world_class {
            return true;
        }
        if step == MAX_SUPER_STEPS || !memory.readable(current, layout::USTRUCT_SUPER + 8) {
            return false;
        }
        match memory.ptr_at(current + layout::USTRUCT_SUPER) {
            Some(parent) => current = parent,
            None => return false,
        }
    }
    false
}

/// The leftover worlds (3.2 step 3): pending-kill worlds that are neither a class default object nor `gworld`.
pub fn find_leftovers(memory: &dyn Memory, objects: usize, world_class: usize, gworld: usize) -> Vec<usize> {
    let mut is_world: HashMap<usize, bool> = HashMap::new();
    let mut leftovers = Vec::new();
    walk(memory, objects, &mut |item| {
        let object = item.object;
        if object == 0 || object == gworld || !is_pending_kill(item.flags) {
            return ControlFlow::Continue(());
        }
        if !memory.readable(object, layout::UOBJECT_SIZE) {
            return ControlFlow::Continue(());
        }
        let (Some(class), Some(flags)) = (
            memory.ptr_at(object + layout::UOBJECT_CLASS),
            memory.u32_at(object + layout::UOBJECT_FLAGS),
        ) else {
            return ControlFlow::Continue(());
        };
        if flags & layout::RF_CLASS_DEFAULT_OBJECT != 0 {
            return ControlFlow::Continue(());
        }
        let world = *is_world
            .entry(class)
            .or_insert_with(|| reaches_world(memory, class, world_class));
        if world {
            leftovers.push(object);
            if leftovers.len() == MAX_LEFTOVERS {
                return ControlFlow::Break(());
            }
        }
        ControlFlow::Continue(())
    });
    leftovers
}

/// The leftover world `object` lies in: itself or one of its first 16 outers (17 checks).
pub fn world_holding(memory: &dyn Memory, object: usize, leftovers: &[usize]) -> Option<usize> {
    let mut current = object;
    for step in 0..=MAX_OUTERS {
        if current == 0 {
            return None;
        }
        if leftovers.contains(&current) {
            return Some(current);
        }
        if step == MAX_OUTERS || !memory.readable(current, layout::UOBJECT_SIZE) {
            return None;
        }
        current = memory.ptr_at(current + layout::UOBJECT_OUTER)?;
    }
    None
}

/// An eligible item and the leftover world it lies in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Eligible {
    /// The item.
    pub item: Item,
    /// Its leftover world.
    pub world: usize,
}

/// Every eligible item inside a leftover (3.3).
pub fn find_eligible(memory: &dyn Memory, objects: usize, leftovers: &[usize]) -> Vec<Eligible> {
    let mut eligible = Vec::new();
    if leftovers.is_empty() {
        return eligible;
    }
    walk(memory, objects, &mut |item| {
        if item.object != 0
            && is_eligible(item.flags)
            && let Some(world) = world_holding(memory, item.object, leftovers)
        {
            eligible.push(Eligible { item, world });
        }
        ControlFlow::Continue(())
    });
    eligible
}

/// Sixteen world addresses, oldest overwritten, each held once (3.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ring {
    slots: [usize; RING_SIZE],
    next: usize,
}

impl Default for Ring {
    fn default() -> Self {
        Self::new()
    }
}

impl Ring {
    /// Empty.
    pub const fn new() -> Self {
        Self {
            slots: [0; RING_SIZE],
            next: 0,
        }
    }

    /// Whether `world` is held.
    pub fn contains(&self, world: usize) -> bool {
        world != 0 && self.slots.contains(&world)
    }

    /// Holds `world` unless it already is; true when added.
    pub fn insert_once(&mut self, world: usize) -> bool {
        if world == 0 || self.contains(world) {
            return false;
        }
        self.slots[self.next] = world;
        self.next = (self.next + 1) % RING_SIZE;
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Idle,
    FirstPending,
    LastPending,
}

/// The world-change watcher's state machine (5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watcher {
    last_poll: Option<u64>,
    seen: usize,
    changed_at: u64,
    stage: Stage,
}

impl Default for Watcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Watcher {
    /// No call yet; the world last seen is null.
    pub const fn new() -> Self {
        Self {
            last_poll: None,
            seen: 0,
            changed_at: 0,
            stage: Stage::Idle,
        }
    }

    /// One SP-1 call at `now`: at most every 250 ms reads `GWorld` and says which pass to run.
    pub fn poll(&mut self, now: u64, gworld: &mut dyn FnMut() -> usize) -> Option<Reason> {
        if let Some(last) = self.last_poll
            && now.saturating_sub(last) < WATCH_INTERVAL_MS
        {
            return None;
        }
        self.last_poll = Some(now);
        let world = gworld();
        let since = now.saturating_sub(self.changed_at);
        if world != self.seen {
            self.seen = world;
            self.changed_at = now;
            self.stage = Stage::FirstPending;
            Some(Reason::WorldChange)
        } else if self.stage == Stage::FirstPending && since >= ARRIVAL_FIRST_MS {
            self.stage = Stage::LastPending;
            Some(Reason::Arrival10s)
        } else if self.stage == Stage::LastPending && since >= ARRIVAL_LAST_MS {
            self.stage = Stage::Idle;
            Some(Reason::Arrival60s)
        } else {
            None
        }
    }
}

/// What a pass found and asks the DLL to do.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// The leftover worlds (the guard reuses them).
    pub leftovers: Vec<usize>,
    /// Flags words whose `Async` bit to clear (empty in report-only mode).
    pub clear: Vec<usize>,
    /// Lines to log, in order.
    pub lines: Vec<String>,
}

/// What the guard decided.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Guard {
    /// Fail this travel.
    pub refused: bool,
    /// Lines to log.
    pub lines: Vec<String>,
}

/// R16's process state: the world class, the once-read switches and the two rings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Travel {
    world_class: Option<usize>,
    report_only: Option<bool>,
    guard_on: Option<bool>,
    released: Ring,
    warned: Ring,
    /// The watcher (5.1).
    pub watcher: Watcher,
}

impl Default for Travel {
    fn default() -> Self {
        Self::new()
    }
}

impl Travel {
    /// Nothing known yet.
    pub const fn new() -> Self {
        Self {
            world_class: None,
            report_only: None,
            guard_on: None,
            released: Ring::new(),
            warned: Ring::new(),
            watcher: Watcher::new(),
        }
    }

    /// The worlds released so far.
    pub fn released(&self) -> &Ring {
        &self.released
    }

    /// The leftover-world pass (3). `report_only` reads `KEEP_STUCK_ASYNC_FLAGS.flag`, called once, by the first
    /// pass that finds a leftover.
    pub fn pass(
        &mut self,
        memory: &dyn Memory,
        objects: usize,
        names: &dyn Names,
        gworld: usize,
        reason: Reason,
        report_only: &mut dyn FnMut() -> bool,
    ) -> Outcome {
        let mut outcome = Outcome::default();
        let world_class = match self.world_class {
            Some(class) => class,
            None => match find_world_class(memory, objects, names) {
                Some(class) => *self.world_class.insert(class),
                None => return outcome,
            },
        };
        outcome.leftovers = find_leftovers(memory, objects, world_class, gworld);
        if outcome.leftovers.is_empty() {
            return outcome;
        }
        let report_only = match self.report_only {
            Some(mode) => mode,
            None => {
                let mode = report_only();
                outcome.lines.push(release_mode_line(mode).to_owned());
                *self.report_only.insert(mode)
            }
        };
        if reason == Reason::WorldChange {
            for &world in &outcome.leftovers {
                if self.released.contains(world) && self.warned.insert_once(world) {
                    let name = names.short_name(world).unwrap_or_else(|| "?".to_owned());
                    outcome.lines.push(survivor_line(world, &name));
                }
            }
        }
        let eligible = find_eligible(memory, objects, &outcome.leftovers);
        if eligible.is_empty() {
            return outcome;
        }
        let mut worlds: Vec<usize> = Vec::new();
        for found in &eligible {
            if !worlds.contains(&found.world) {
                worlds.push(found.world);
            }
        }
        if !report_only {
            outcome.clear = eligible.iter().map(|found| found.item.flags_address()).collect();
            for &world in &worlds {
                self.released.insert_once(world);
            }
        }
        let name = |object: usize| names.short_name(object).unwrap_or_else(|| "?".to_owned());
        let listed: Vec<Listed> = eligible
            .iter()
            .take(MAX_LISTED)
            .map(|found| {
                let class = memory
                    .ptr_at(found.item.object.wrapping_add(layout::UOBJECT_CLASS))
                    .unwrap_or(0);
                Listed {
                    object: name(found.item.object),
                    class: name(class),
                    world: name(found.world),
                }
            })
            .collect();
        outcome
            .lines
            .push(release_line(reason, report_only, eligible.len(), worlds.len(), &listed));
        outcome
    }

    /// The guard (4.3), after the `loadmap-enter` pass. `enabled` reads `ENABLE_ZOMBIE_LOADMAP_GUARD.flag`, once,
    /// at the first call; `path` reads the destination's map path, only when the guard is on.
    pub fn guard(
        &mut self,
        enabled: &mut dyn FnMut() -> bool,
        path: &mut dyn FnMut() -> Option<String>,
        leftovers: &[usize],
        context_world: usize,
        names: &dyn Names,
    ) -> Guard {
        let mut guard = Guard::default();
        let on = match self.guard_on {
            Some(on) => on,
            None => {
                let on = enabled();
                guard.lines.push(guard_mode_line(on).to_owned());
                *self.guard_on.insert(on)
            }
        };
        if !on {
            return guard;
        }
        let Some(path) = path().filter(|path| !path.is_empty()) else {
            return guard;
        };
        let short = map_short_name(&path);
        let found = leftovers.iter().copied().find(|&world| {
            world != context_world
                && !self.released.contains(world)
                && names.short_name(world).as_deref() == Some(short)
        });
        if let Some(world) = found {
            guard.refused = true;
            guard.lines.push(refused_line(&path, world));
        }
        guard
    }
}

/// An `FString`'s text (`num` counts the NUL, 2 to 1024): every unit above 127 is `?`; `None` when out of bounds,
/// unreadable or empty.
pub fn fstring_text(memory: &dyn Memory, data: usize, num: i32) -> Option<String> {
    if !(2..=layout::FSTRING_MAP_MAX_NUM).contains(&num) {
        return None;
    }
    let units = usize::try_from(num).ok()?;
    let mut bytes = vec![0u8; units * 2];
    if !memory.read(data, &mut bytes) {
        return None;
    }
    let text: String = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .take_while(|&unit| unit != 0)
        .map(|unit| u8::try_from(unit).ok().filter(u8::is_ascii).map_or('?', char::from))
        .collect();
    (!text.is_empty()).then_some(text)
}

/// The map path of an `FURL` (4.1).
pub fn read_map_path(memory: &dyn Memory, url: usize) -> Option<String> {
    if !memory.readable(url, layout::FURL_SIZE) {
        return None;
    }
    let at = url + layout::FURL_MAP;
    fstring_text(memory, memory.ptr_at(at)?, memory.i32_at(at + 8)?)
}

/// The world a `FWorldContext` holds (the world being left, at `LoadMap`'s entry); 0 when unreadable.
pub fn context_world(memory: &dyn Memory, context: usize) -> usize {
    if !memory.readable(context, layout::WORLD_CONTEXT_SIZE) {
        return 0;
    }
    memory.ptr_at(context + layout::WORLD_CONTEXT_WORLD).unwrap_or(0)
}

/// A map path's short name: after its last `/`, before its first `.`.
pub fn map_short_name(path: &str) -> &str {
    let tail = path.rsplit('/').next().unwrap_or(path);
    tail.split('.').next().unwrap_or(tail)
}

/// One object a release line names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// Its short name.
    pub object: String,
    /// Its class's short name.
    pub class: String,
    /// Its leftover world's short name.
    pub world: String,
}

/// Said once, at the first pass that finds a leftover.
pub fn release_mode_line(report_only: bool) -> &'static str {
    if report_only {
        "[Travel] leftover-world release off (KEEP_STUCK_ASYNC_FLAGS.flag; report only)"
    } else {
        "[Travel] leftover-world release on"
    }
}

/// Said once, at the first TR-1 call.
pub fn guard_mode_line(on: bool) -> &'static str {
    if on {
        "[Travel] leftover-world guard on (ENABLE_ZOMBIE_LOADMAP_GUARD.flag)"
    } else {
        "[Travel] leftover-world guard off"
    }
}

/// The release line (3.3); at most 8 objects listed.
pub fn release_line(reason: Reason, report_only: bool, objects: usize, worlds: usize, listed: &[Listed]) -> String {
    let verb = if report_only {
        "found (not released)"
    } else {
        "released"
    };
    let entries: Vec<String> = listed
        .iter()
        .take(MAX_LISTED)
        .map(|entry| format!("{}({}) in {}", entry.object, entry.class, entry.world))
        .collect();
    format!(
        "[Travel] {}: {verb} the stuck Async keep flag on {objects} object(s) in {worlds} leftover world(s): {}",
        reason.text(),
        entries.join(", ")
    )
}

/// A released world that survived the purge (3.4).
pub fn survivor_line(world: usize, name: &str) -> String {
    format!(
        "[Travel] leftover world {world:#x} {name} survived the purge after its keep flag was released: something \
         else holds it"
    )
}

/// The guard's refusal (4.3).
pub fn refused_line(path: &str, world: usize) -> String {
    format!(
        "[Travel] REFUSED map={path}: its previous world {world:#x} is a pending-kill leftover; failing this travel \
         instead of reusing it (ENABLE_ZOMBIE_LOADMAP_GUARD.flag)"
    )
}

#[cfg(test)]
mod tests;
