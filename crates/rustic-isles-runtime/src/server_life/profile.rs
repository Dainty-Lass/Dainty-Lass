//! The sampling profiler (spec-runtime-profile): with `RUNTIME_PROFILE.flag`, a thread of its own samples the game
//! thread's stack about once a millisecond for 20 s windows and logs where the time went.

use super::{CONN_COUNT, hub_role, sim};
use crate::ffi::profile as raw;
use crate::ffi::{detaching, flags, log, memory, win};
use rustic_isles_runtime_core::profile::{self as core, Context, Profile};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::Instant;

/// Starts the profiler thread, once, from R8's start step (the game thread is known by then).
pub(super) fn start() {
    if !flags::flag(core::FLAG) {
        return;
    }
    let Some(thread) = raw::open(memory::frame_thread()) else {
        log::line("[MitRuntime] [Profile] could not open the game thread: profiler off");
        return;
    };
    match std::thread::Builder::new()
        .name("rir-profiler".into())
        .spawn(move || run(&thread))
    {
        Ok(_) => log::line(core::ON_LINE),
        Err(error) => log::line(&format!("[MitRuntime] [Profile] could not start: {error}")),
    }
}

fn run(thread: &raw::Thread) {
    let start = win::tick();
    let mut last = None;
    let mut primaries: HashMap<usize, usize> = HashMap::new();
    let mut modules: Vec<(usize, usize, String)> = Vec::new();
    loop {
        let begin = core::next_window(start, last);
        while win::tick() < begin {
            if detaching() {
                return;
            }
            win::sleep_ms(1000);
        }
        last = Some(begin);
        let Some(profile) = window(thread, &mut primaries) else {
            return;
        };
        let context = Context {
            ms: core::WINDOW_MS,
            connections: CONN_COUNT.load(Ordering::Relaxed),
            role: hub_role(),
            bots: sim::bots(),
        };
        let mut name = |address: usize| name(address, &mut modules);
        let names: HashMap<usize, String> = profile.functions().into_iter().map(|f| (f, name(f))).collect();
        for line in core::lines(&profile, &context, &|f| names.get(&f).cloned().unwrap_or_default()) {
            log::line(&line);
        }
    }
}

/// One window; `None` when the process is detaching.
fn window(thread: &raw::Thread, primaries: &mut HashMap<usize, usize>) -> Option<Profile> {
    let mut profile = Profile::default();
    let mut stack = [0usize; core::DEPTH];
    let started = Instant::now();
    while started.elapsed().as_millis() < u128::from(core::WINDOW_MS) {
        if detaching() {
            return None;
        }
        let written = raw::sample(thread, &mut stack);
        // After the resume: a chained unwind fragment of the game counts as its primary function
        for frame in &mut stack[..written] {
            *frame = *primaries.entry(*frame).or_insert_with(|| primary(*frame));
        }
        profile.add(&stack[..written]);
        win::sleep_ms(core::SLEEP_MS);
    }
    Some(profile)
}

fn primary(address: usize) -> usize {
    let base = win::image_base();
    address
        .checked_sub(base)
        .and_then(|rva| u32::try_from(rva).ok())
        .and_then(memory::primary_entry)
        .and_then(|rva| base.checked_add(usize::try_from(rva).ok()?))
        .unwrap_or(address)
}

fn name(address: usize, modules: &mut Vec<(usize, usize, String)>) -> String {
    // A module is looked up once; its end is the next lookup's guess, so a later address in it is found by range
    if let Some((base, _, file)) = modules.iter().find(|(base, end, _)| (*base..*end).contains(&address)) {
        return core::label(address, Some((file, *base)));
    }
    match raw::module_of(address) {
        Some((base, file)) => {
            let label = core::label(address, Some((&file, base)));
            let end = address.saturating_add(1).max(base);
            match modules.iter_mut().find(|(known, _, _)| *known == base) {
                Some(entry) => entry.1 = entry.1.max(end),
                None => modules.push((base, end, file)),
            }
            label
        }
        None => core::label(address, None),
    }
}
