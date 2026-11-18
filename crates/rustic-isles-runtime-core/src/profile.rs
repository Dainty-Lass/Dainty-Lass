//! The sampling profiler's pure half (spec-runtime-profile): the schedule, the tallies of sampled stacks and every log
//! text. The DLL suspends the game thread, unwinds it and hands each stack here as function start addresses.

use crate::server_life::HubRole;
use std::collections::{HashMap, HashSet};

/// The flag file that turns the profiler on (call 1).
pub const FLAG: &str = "RUNTIME_PROFILE";
/// The first window starts this long after the start step.
pub const FIRST_MS: u64 = 90_000;
/// Between window starts.
pub const EVERY_MS: u64 = 300_000;
/// One window's length.
pub const WINDOW_MS: u64 = 20_000;
/// The pause between samples (`Sleep`; the real rate is logged).
pub const SLEEP_MS: u32 = 1;
/// The deepest stack unwound.
pub const DEPTH: usize = 48;
/// Lines per list.
pub const TOP: usize = 25;
/// Whole stacks logged.
pub const TOP_STACKS: usize = 6;
/// Frames of a logged stack.
pub const STACK_FRAMES: usize = 24;
/// Distinct stacks kept in one window; later new ones only count in the lists.
pub const MAX_STACKS: usize = 20_000;

/// The start line.
pub const ON_LINE: &str = "[MitRuntime] [Profile] on (RUNTIME_PROFILE.flag): the game thread, 20 s windows from 90 s \
                           after listen, every 300 s";

/// When the window after `start` (the start step) and `last` (the last window's start) begins.
pub fn next_window(start: u64, last: Option<u64>) -> u64 {
    last.map_or(start + FIRST_MS, |last| last + EVERY_MS)
}

/// One window's tallies, by function start address.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Profile {
    /// Stacks taken.
    pub samples: u64,
    /// Samples whose thread could not be suspended or read.
    pub missed: u64,
    /// Samples by the innermost function.
    pub own: HashMap<usize, u64>,
    /// Samples with the function anywhere in the stack (once per sample).
    pub total: HashMap<usize, u64>,
    /// Samples by whole stack, innermost first.
    pub stacks: HashMap<Vec<usize>, u64>,
}

impl Profile {
    /// Adds one stack, innermost first.
    pub fn add(&mut self, stack: &[usize]) {
        let Some(&leaf) = stack.first() else {
            self.missed += 1;
            return;
        };
        self.samples += 1;
        *self.own.entry(leaf).or_default() += 1;
        if self.stacks.len() < MAX_STACKS || self.stacks.contains_key(stack) {
            *self.stacks.entry(stack.to_vec()).or_default() += 1;
        }
        let mut seen = HashSet::new();
        for &function in stack {
            if seen.insert(function) {
                *self.total.entry(function).or_default() += 1;
            }
        }
    }

    /// Every function key seen, for the DLL to name.
    pub fn functions(&self) -> Vec<usize> {
        self.total.keys().copied().collect()
    }
}

/// What the summary line adds.
#[derive(Debug, Clone, Copy)]
pub struct Context {
    /// The window's real length.
    pub ms: u64,
    /// R8's connection count (`-1` unknown).
    pub connections: i32,
    /// R8's role.
    pub role: HubRole,
    /// Running bots (spec-runtime-sim).
    pub bots: usize,
}

#[allow(clippy::cast_precision_loss, reason = "display figures")]
fn percent(count: u64, of: u64) -> f64 {
    if of == 0 { 0.0 } else { count as f64 * 100.0 / of as f64 }
}

fn ranked(counts: &HashMap<usize, u64>, name: &dyn Fn(usize) -> String) -> Vec<(String, u64)> {
    let mut rows: Vec<(String, u64)> = counts
        .iter()
        .map(|(&function, &count)| (name(function), count))
        .collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    rows.truncate(TOP);
    rows
}

/// Every line of one window; `name` labels a function key (`module+0xRVA`).
#[allow(clippy::cast_precision_loss, reason = "display figures")]
pub fn lines(profile: &Profile, context: &Context, name: &dyn Fn(usize) -> String) -> Vec<String> {
    let secs = context.ms as f64 / 1000.0;
    let rate = if secs > 0.0 { profile.samples as f64 / secs } else { 0.0 };
    let mut out = vec![format!(
        "[MitRuntime] [Profile] game thread: {} samples in {secs:.1} s ({rate:.0}/s, {} missed), connections {}, role {}, \
         bots {}",
        profile.samples,
        profile.missed,
        context.connections,
        context.role.text(),
        context.bots,
    )];
    for (title, counts) in [("self", &profile.own), ("total", &profile.total)] {
        let rows = ranked(counts, name);
        let shown = rows.len();
        for (i, (label, count)) in rows.into_iter().enumerate() {
            out.push(format!(
                "[MitRuntime] [Profile] {title} {}/{shown} {label}: {:.1}% ({count})",
                i + 1,
                percent(count, profile.samples),
            ));
        }
    }
    let mut stacks: Vec<(&Vec<usize>, u64)> = profile.stacks.iter().map(|(stack, &count)| (stack, count)).collect();
    stacks.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    stacks.truncate(TOP_STACKS);
    let shown = stacks.len();
    for (i, (stack, count)) in stacks.into_iter().enumerate() {
        let frames: Vec<String> = stack.iter().take(STACK_FRAMES).map(|&f| name(f)).collect();
        out.push(format!(
            "[MitRuntime] [Profile] stack {}/{shown} {:.1}% ({count}): {}",
            i + 1,
            percent(count, profile.samples),
            frames.join(" < "),
        ));
    }
    out
}

/// A function's label: its module's file name and the function's offset in it, or the bare address.
pub fn label(address: usize, module: Option<(&str, usize)>) -> String {
    module.map_or_else(
        || format!("0x{address:X}"),
        |(name, base)| format!("{name}+0x{:X}", address.saturating_sub(base)),
    )
}

#[cfg(test)]
mod tests;
