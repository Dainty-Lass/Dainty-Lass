//! PR-1 (5): the role, the slot and "applied", the validity note, and the bounded retry.

use super::lines::{self, APPLY_KIND};
use super::server::add_role;
use super::{
    Game, RETRY_ATTEMPTS, RETRY_INTERVAL_MS, RETRY_WAIT_LOG_MS, RETRY_WINDOW_MS, RetryEntry, State, budgeted, fstring,
    readable_ptr, role_item_id, with,
};
use crate::offsets::r14 as at;

/// What 5.1 read off one loadout.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Snapshot {
    /// The role inventory item.
    pub role_item: usize,
    /// The loadout slot (0 unresolved).
    pub slot: usize,
    /// The controller's pawn.
    pub pawn: usize,
    /// The pawn's `PlayerRole`.
    pub pawn_role: usize,
    /// The item's `ItemId`.
    pub item_id: String,
    /// The pawn role's `ItemId`.
    pub role_id: String,
}

impl Snapshot {
    /// 5.1's "applied".
    pub fn applied(&self) -> bool {
        self.role_item != 0
            && self.slot != 0
            && self.pawn != 0
            && self.pawn_role != 0
            && !self.item_id.is_empty()
            && self.item_id == self.role_id
    }

    /// 5.2 step 2: an omnicell equipped, or the slot not resolved yet. An unequipped slot is never noted.
    pub fn noted(&self) -> bool {
        self.role_item != 0 || self.slot == 0
    }
}

/// 5.1 on `loadout`; `None` when the loadout is not readable for `0x40`.
pub fn snapshot(game: &dyn Game, loadout: usize) -> Option<Snapshot> {
    if loadout == 0 || !game.readable(loadout, at::LOADOUT_SPAN) {
        return None;
    }
    let mut snap = Snapshot::default();
    let inter = game.active_slot_data(loadout);
    if inter != 0 && game.readable(inter, at::LOADOUT_SPAN) {
        snap.role_item = game.slot_role_item(inter);
        snap.slot = game.ptr(inter + at::INTER_SLOT);
    }
    if snap.role_item != 0 {
        snap.item_id = fstring(game, snap.role_item + at::ITEM_ID);
    }
    let pc = if game.readable(loadout, at::LOADOUT_CONTROLLER_SPAN) {
        readable_ptr(game, loadout + at::LOADOUT_CONTROLLER, at::PC_PAWN_SPAN)
    } else {
        0
    };
    if pc != 0 {
        snap.pawn = game.ptr(pc + crate::offsets::r7::PC_PAWN);
    }
    if snap.pawn != 0 && game.readable(snap.pawn, at::CHARACTER_SPAN) {
        snap.pawn_role = game.ptr(snap.pawn + at::CHARACTER_PLAYER_ROLE);
    }
    if snap.pawn_role != 0 && game.readable(snap.pawn_role, at::ROLE_SPAN) {
        snap.role_id = role_item_id(game, snap.pawn_role);
    }
    Some(snap)
}

/// `note(loadout, applied)` (5.3).
pub fn note(game: &dyn Game, state: &State, loadout: usize, applied: bool) {
    let now = game.now_ms();
    with(&state.retry, |table| {
        if let Some(slot) = table
            .iter_mut()
            .find(|entry| entry.is_some_and(|entry| entry.loadout == loadout))
        {
            if applied {
                *slot = None;
            }
            return;
        }
        if applied {
            return;
        }
        if let Some(slot) = table.iter_mut().find(|entry| entry.is_none()) {
            *slot = Some(RetryEntry {
                loadout,
                first_seen_ms: now,
                last_retry_ms: now,
                last_wait_log_ms: 0,
                retries: 0,
            });
        }
    });
}

/// An applied role on a server joins the refresh and the Tempest watch (5.2 step 3).
fn applied_on_server(game: &dyn Game, state: &State, snap: &Snapshot, server: bool) {
    if server && snap.applied() {
        add_role(game, state, snap.pawn_role);
    }
}

/// PR-1's body after `next(loadout)` ran (5.2).
pub fn on_apply(game: &dyn Game, state: &State, loadout: usize, server: bool) {
    let Some(snap) = snapshot(game, loadout) else {
        return;
    };
    if snap.noted() {
        note(game, state, loadout, snap.applied());
    }
    applied_on_server(game, state, &snap, server);
    budgeted(game, &state.budgets.apply, APPLY_KIND, true, || {
        lines::apply(loadout, &snap.item_id, snap.pawn_role, &snap.role_id, snap.applied())
    });
}

fn clear(state: &State, index: usize, loadout: usize) {
    with(&state.retry, |table| {
        if table[index].is_some_and(|entry| entry.loadout == loadout) {
            table[index] = None;
        }
    });
}

fn update(state: &State, index: usize, loadout: usize, body: impl FnOnce(&mut RetryEntry)) {
    with(&state.retry, |table| {
        if let Some(entry) = table[index].as_mut().filter(|entry| entry.loadout == loadout) {
            body(entry);
        }
    });
}

/// The retry (5.4), from either pump. The table is re-read per entry: the application can re-enter R14.
pub fn retry(game: &dyn Game, state: &State, server: bool) {
    for index in 0..super::RETRY_ENTRIES {
        let Some(entry) = with(&state.retry, |table| table[index]).flatten() else {
            continue;
        };
        let loadout = entry.loadout;
        let now = game.now_ms();
        let elapsed = now.saturating_sub(entry.first_seen_ms);
        if elapsed >= RETRY_WINDOW_MS {
            game.log(&lines::retry_gave_up(loadout, elapsed, entry.retries));
            clear(state, index, loadout);
            continue;
        }
        if entry.retries >= RETRY_ATTEMPTS || !game.readable(loadout, at::LOADOUT_SPAN) {
            clear(state, index, loadout);
            continue;
        }
        if now.saturating_sub(entry.last_retry_ms) < RETRY_INTERVAL_MS {
            continue;
        }
        if !game.is_live_object(loadout) {
            game.log(&lines::retry_gone(loadout));
            clear(state, index, loadout);
            continue;
        }
        let Some(snap) = snapshot(game, loadout) else {
            clear(state, index, loadout);
            continue;
        };
        if snap.applied() {
            clear(state, index, loadout);
            continue;
        }
        if snap.role_item == 0 || snap.slot == 0 || snap.pawn == 0 {
            // A live loadout can hold its role before its controller owns a pawn: applying then can stop
            // ClientRestart from completing
            if entry.last_wait_log_ms == 0 || now.saturating_sub(entry.last_wait_log_ms) >= RETRY_WAIT_LOG_MS {
                game.log(&lines::retry_waiting(loadout, snap.role_item, snap.slot, snap.pawn));
                update(state, index, loadout, |entry| entry.last_wait_log_ms = now.max(1));
            }
            continue;
        }
        game.apply(loadout, false);
        let after = snapshot(game, loadout).unwrap_or_default();
        let attempt = entry.retries + 1;
        update(state, index, loadout, |entry| {
            entry.last_retry_ms = now;
            entry.retries = attempt;
        });
        game.log(&lines::retry_attempt(attempt, loadout, elapsed, after.applied()));
        if after.applied() {
            clear(state, index, loadout);
            applied_on_server(game, state, &after, server);
        }
    }
}
