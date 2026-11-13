//! Progression on the server (8): the configuration query at boot (LD-5), the bookkeeping of parked and completed
//! components (LD-6, LD-7), and `SP-3@R7`'s resume poll.

use super::super::{engine, flags, log};
use super::game::{self, read_u8, readable};
use crate::config;
use rustic_isles_runtime_core::loading::{Poll, Progression, ResumeBudget, lines, poll_gives_up};
use rustic_isles_runtime_core::offsets::r7;
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard, OnceLock, PoisonError};

/// What LD-5 answered on this thread during the current `ReadFromBackend`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    NotAsked,
    Loaded,
    NotLoaded,
}

thread_local! {
    static PREDICATE: Cell<Answer> = const { Cell::new(Answer::NotAsked) };
}

static STATE: LazyLock<Mutex<Progression>> = LazyLock::new(|| Mutex::new(Progression::new()));
static WAITERS: AtomicUsize = AtomicUsize::new(0);
static BUDGET: Mutex<ResumeBudget> = Mutex::new(ResumeBudget::new());
static POLL: Mutex<Poll> = Mutex::new(Poll::new());
static GAME_INSTANCE: AtomicUsize = AtomicUsize::new(0);
static QUERIED: AtomicBool = AtomicBool::new(false);
static COMPLETE_LOGGED: AtomicBool = AtomicBool::new(false);
static TRUE_LOGGED: AtomicBool = AtomicBool::new(false);
static FALSE_LOGGED: AtomicBool = AtomicBool::new(false);

fn state() -> MutexGuard<'static, Progression> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

fn sync_count(state: &Progression) {
    WAITERS.store(state.waiter_count(), Ordering::Release);
}

/// Whether `component` was parked or completed in this process's bookkeeping (tests).
#[cfg(test)]
pub fn bookkeeping(component: usize) -> (bool, bool) {
    let state = state();
    (state.candidates().contains(&component), state.is_completed(component))
}

/// LD-6's body: record the completion, then the original.
pub fn complete_load(component: usize, next: &mut dyn FnMut()) {
    if !COMPLETE_LOGGED.swap(true, Ordering::Relaxed) {
        log::line(lines::COMPLETE_RUNS);
    }
    if component != 0 {
        let mut state = state();
        state.complete(component);
        sync_count(&state);
    }
    next();
}

/// LD-7's body: the original; a component whose read asked the predicate and heard "not loaded" parked itself.
pub fn read_from_backend(interface: usize, next: &mut dyn FnMut()) {
    PREDICATE.with(|answer| answer.set(Answer::NotAsked));
    next();
    if PREDICATE.with(Cell::get) != Answer::NotLoaded || interface == 0 {
        return;
    }
    let component = interface.wrapping_sub(r7::PROGRESSION_LOADER);
    let waiters = {
        let mut state = state();
        let waiters = state.park(component, game::now());
        sync_count(&state);
        waiters
    };
    if !flags::production_log_profile() {
        log::line(&lines::parked(component, waiters));
    }
}

/// A live, non-default `ProgressionComponent`, readable for its span.
fn is_component(component: usize) -> bool {
    readable(component, r7::PROGRESSION_SPAN)
        && game::is_live(component)
        && game::object_flags(component).is_some_and(|flags| flags & r7::RF_CLASS_DEFAULT_OBJECT == 0)
        && game::is_a(component, "ProgressionComponent")
}

/// Configuration received and setup not finished.
fn stranded(component: usize) -> bool {
    read_u8(component + r7::PROGRESSION_RECEIVED).is_some_and(|received| received != 0)
        && read_u8(component + r7::PROGRESSION_FINISHED) == Some(0)
}

fn every_object() -> Vec<usize> {
    let Some(engine) = engine::engine() else {
        return Vec::new();
    };
    let count = engine.object_count().unwrap_or(0);
    (0..count).filter_map(|index| engine.object_at(index)).collect()
}

/// 8.3 step 2: components that got their configuration and never finished, resumed at most eight times.
fn resume_stranded() {
    let (candidates, overflow) = {
        let state = state();
        (state.candidates(), state.overflow())
    };
    let walk = if overflow { every_object() } else { candidates };
    for component in walk {
        if !BUDGET.lock().unwrap_or_else(PoisonError::into_inner).left() {
            return;
        }
        if !is_component(component) {
            state().remove_candidate(component);
            continue;
        }
        if state().is_completed(component) || !stranded(component) {
            continue;
        }
        if !BUDGET.lock().unwrap_or_else(PoisonError::into_inner).take() {
            return;
        }
        log::line(&lines::stranded(component));
        game::complete_load_internal(component);
    }
}

/// 8.3 step 4: `QueryCurrentHuntPassSeasonData` with an unbound delegate, `FUNC_Native` set for the call.
fn query(game_instance: usize) {
    log::line(&lines::querying(game_instance));
    let Some(function) = game::find_function(game_instance, "QueryCurrentHuntPassSeasonData") else {
        log::line(lines::QUERY_MISSING);
        return;
    };
    let mut block = game::params(r7::QUERY_PARAMS);
    if let Err(size) = game::params_fit(function, block.len() * 8) {
        log::line(&lines::query_skipped(size, block.len() * 8));
        return;
    }
    let flags_at = function + r7::FUNCTION_FLAGS;
    let previous = game::read_u32(flags_at);
    let marked = previous.is_some_and(|flags| game::write::<u32>(flags_at, flags | r7::FUNC_NATIVE));
    game::call(game_instance, function, &mut block);
    if marked && let Some(previous) = previous {
        game::write::<u32>(flags_at, previous);
    }
    log::line(lines::QUERY_RETURNED);
}

/// LD-5's body (8.3).
pub fn predicate(game_instance: usize, next: &mut dyn FnMut() -> u8) -> u8 {
    let answer = next();
    let loaded = answer != 0;
    let budget_left = BUDGET.lock().unwrap_or_else(PoisonError::into_inner).left();
    if loaded && game_instance != 0 && budget_left {
        resume_stranded();
    }
    if game_instance != 0 {
        GAME_INSTANCE.store(game_instance, Ordering::Release);
        if !QUERIED.swap(true, Ordering::SeqCst) {
            query(game_instance);
        }
    }
    let first = if loaded { &TRUE_LOGGED } else { &FALSE_LOGGED };
    if !first.swap(true, Ordering::Relaxed) {
        log::line(lines::configuration(loaded));
    }
    PREDICATE.with(|predicate| {
        predicate.set(if loaded { Answer::Loaded } else { Answer::NotLoaded });
    });
    answer
}

/// 8.4 step 2: on only without a phantom local player, decided at the first poll with a waiter.
fn resume_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        let on = game::no_phantom(config::is_server());
        log::line(lines::resume_mode(on));
        on
    })
}

/// SP-3 after the engine tick, `order = 100` (8.4).
pub fn poll(_engine: usize, _delta: f32) {
    if WAITERS.load(Ordering::Acquire) == 0 {
        return;
    }
    if !resume_enabled() {
        let mut state = state();
        state.clear_waiters();
        sync_count(&state);
        return;
    }
    let now = game::now();
    if !POLL.lock().unwrap_or_else(PoisonError::into_inner).due(now) {
        return;
    }
    let game_instance = GAME_INSTANCE.load(Ordering::Acquire);
    if game_instance == 0 || !game::is_live(game_instance) {
        return;
    }
    // Past the chain, so 8.3's resume and query do not run from the tick
    let Some(loaded) = game::predicate_original(game_instance) else {
        return;
    };
    if !loaded {
        let mut state = state();
        if poll_gives_up(state.wait_since(), now) {
            let waiters = state.clear_waiters();
            sync_count(&state);
            drop(state);
            log::line(&lines::poll_gave_up(waiters));
        }
        return;
    }
    let (waiters, since) = {
        let mut state = state();
        let taken = state.take_waiters();
        sync_count(&state);
        taken
    };
    for component in waiters {
        if state().is_completed(component) {
            log::line(&lines::already_completed(component));
        } else if !is_component(component) {
            log::line(&lines::gone(component));
        } else {
            let after = since.map_or(0, |since| now.saturating_sub(since));
            log::line(&lines::resumed(after, component));
            game::complete_load_internal(component);
        }
    }
}
