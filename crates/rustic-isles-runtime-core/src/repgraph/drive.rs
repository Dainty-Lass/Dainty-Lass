//! The graph drive's checks (11.2 steps 2 to 6) and its first lines (step 9). The call is the DLL's.

use super::{Budgets, CHECKED_SLOTS, DRIVES_LOGGED, Game, lines};
use crate::offsets::repgraph as at;
use std::sync::atomic::Ordering;

/// The drive slot values checked so far and their answers (step 6: once per distinct value).
#[derive(Debug, Clone, Copy)]
pub struct Checked {
    seen: [(usize, bool); CHECKED_SLOTS],
    len: usize,
}

impl Default for Checked {
    fn default() -> Self {
        Self::new()
    }
}

impl Checked {
    /// Nothing checked.
    pub const fn new() -> Self {
        Self {
            seen: [(0, false); CHECKED_SLOTS],
            len: 0,
        }
    }

    /// Whether `function` is a function entry of the game; its refusal line is written once per value (past
    /// `CHECKED_SLOTS` values, checked every time and not logged again).
    fn allows(&mut self, game: &dyn Game, graph: usize, function: usize) -> bool {
        if let Some(&(_, ok)) = self.seen[..self.len].iter().find(|(seen, _)| *seen == function) {
            return ok;
        }
        let ok = game
            .image_rva(function)
            .is_some_and(|rva| game.function_entry(rva) == Some(rva));
        if self.len < CHECKED_SLOTS {
            self.seen[self.len] = (function, ok);
            self.len += 1;
            if !ok {
                game.log(&lines::drive_not_function(graph, function));
            }
        }
        ok
    }
}

/// The graph of `driver` after steps 3 and 4: `None` when it does not exist yet or belongs to another driver.
pub fn graph_of(game: &dyn Game, driver: usize) -> Option<usize> {
    if !game.readable_this_frame(driver, at::NETDRIVER_SPAN_FOR_GRAPH) {
        return None;
    }
    let graph = game.ptr(driver + at::NETDRIVER_REPLICATION_DRIVER);
    (graph != 0 && game.readable_this_frame(graph, at::GRAPH_SPAN) && game.ptr(graph + at::GRAPH_NET_DRIVER) == driver)
        .then_some(graph)
}

/// Steps 2 to 6: `(graph, ServerReplicateActors)` to call, or `None`.
pub fn drive_target(game: &dyn Game, checked: &mut Checked, driver: Option<usize>) -> Option<(usize, usize)> {
    let graph = graph_of(game, driver?)?;
    let vtable = game.ptr(graph);
    let slot = vtable.checked_add(at::GRAPH_VTABLE_SERVER_REPLICATE_ACTORS)?;
    if vtable == 0 || !game.readable(slot, 8) {
        return None;
    }
    let function = game.ptr(slot);
    checked.allows(game, graph, function).then_some((graph, function))
}

/// Step 9: the first six drives' line.
pub fn drive_line(game: &dyn Game, budgets: &Budgets, graph: usize, function: usize, result: i32) {
    if budgets.drives.fetch_add(1, Ordering::Relaxed) >= DRIVES_LOGGED {
        // Stop counting so it never wraps
        budgets.drives.store(DRIVES_LOGGED, Ordering::Relaxed);
        return;
    }
    let class = game.class_name(graph).unwrap_or_else(|| "unreadable".to_owned());
    let rva = game.image_rva(function).map_or(0, |rva| rva as usize);
    let count = |offset: usize| game.i32_at(graph + offset).unwrap_or(-1);
    game.log(&lines::drive(
        graph,
        &class,
        rva,
        result,
        count(at::GRAPH_CONNECTIONS_NUM),
        count(at::GRAPH_DEFERRED_NUM),
    ));
}
