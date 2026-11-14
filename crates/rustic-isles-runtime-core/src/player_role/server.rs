//! The server (6 to 10): the pump, the post-activation refresh, the Tempest modifier repair, RG-3's purge and the
//! ability activation route.

use super::lines::{self, ABILITY_KIND, PurgeReason};
use super::{
    COUNT_LIMIT, Game, Invoke, PR_TEMPEST, REFRESH_PUSHES, REFRESH_SETTLE_MS, ROLE_ENTRIES, RoleEntry, Route,
    SERVER_PUMP_MS, State, TEMPEST_ENTRIES, TEMPEST_REPAIRS, TEMPEST_RETRY_MS, TEMPEST_SETTLE_MS, TEMPEST_STATE_LOG_MS,
    TempestEntry, budgeted, fully_active, quiet_line, role_item_id, role_live, with,
};
use crate::offsets::r14 as at;

/// `add(role)` (7.1) and `watch(role)` (8.1): an applied role on the server.
pub fn add_role(game: &dyn Game, state: &State, role: usize) {
    if role == 0 {
        return;
    }
    let full = with(&state.roles, |roles| {
        if roles.iter().any(|entry| entry.role == role) {
            return false;
        }
        if roles.len() >= ROLE_ENTRIES {
            return true;
        }
        roles.push(RoleEntry {
            role,
            ..RoleEntry::default()
        });
        false
    });
    if full == Some(true) {
        state.once(game, "role-table-full", || lines::role_table_full(role));
    }
    watch(game, state, role);
}

fn watch(game: &dyn Game, state: &State, role: usize) {
    if !role_live(game, role) || !game.readable(role, at::ROLE_SPAN) || role_item_id(game, role) != PR_TEMPEST {
        return;
    }
    let outcome = with(&state.tempest, |watch| {
        if watch.iter().any(|entry| entry.role == role) {
            return None;
        }
        if watch.len() >= TEMPEST_ENTRIES {
            return Some(false);
        }
        watch.push(TempestEntry {
            role,
            ..TempestEntry::default()
        });
        Some(true)
    })
    .flatten();
    match outcome {
        Some(true) => game.log(&lines::tempest_watching(role)),
        Some(false) => state.once(game, "tempest-watch-full", || lines::tempest_full(role)),
        None => {}
    }
}

/// Purge (9): every entry for `role` in both tables, with one line when anything went.
pub fn purge(game: &dyn Game, state: &State, role: usize, reason: PurgeReason) {
    let roles = with(&state.roles, |roles| {
        let before = roles.len();
        roles.retain(|entry| entry.role != role);
        before - roles.len()
    })
    .unwrap_or(0);
    let tempest = with(&state.tempest, |watch| {
        let before = watch.len();
        watch.retain(|entry| entry.role != role);
        before - watch.len()
    })
    .unwrap_or(0);
    if roles + tempest > 0 {
        game.log(&lines::purge(role, reason, roles, tempest));
    }
}

/// RG-3's body before `next(graph, info)` (9).
pub fn route_remove(game: &dyn Game, state: &State, info: usize) {
    let actor = if info != 0 && game.readable(info, 8) {
        game.ptr(info)
    } else {
        0
    };
    if actor != 0 && game.readable(actor, at::ACTOR_LIVE_SPAN) && game.is_live_object(actor) && game.is_role(actor) {
        purge(game, state, actor, PurgeReason::RouteRemove);
    }
}

/// `SP-3@R14` (6): at most once a second while any table holds an entry.
pub fn server_pump(game: &dyn Game, state: &State) {
    let empty = with(&state.retry, |table| table.iter().all(Option::is_none)).unwrap_or(true)
        && with(&state.roles, |roles| roles.is_empty()).unwrap_or(true)
        && with(&state.tempest, |watch| watch.is_empty()).unwrap_or(true);
    let now = game.now_ms();
    if empty
        || state
            .server_pump_ms
            .get()
            .is_some_and(|last| now.saturating_sub(last) < SERVER_PUMP_MS)
    {
        return;
    }
    state.server_pump_ms.set(Some(now));
    super::retry(game, state, true);
    refresh(game, state);
    tempest(game, state);
}

fn role_at(state: &State, index: usize) -> Option<RoleEntry> {
    with(&state.roles, |roles| roles.get(index).copied()).flatten()
}

fn set_role(state: &State, index: usize, entry: RoleEntry) {
    with(&state.roles, |roles| {
        if let Some(slot) = roles.get_mut(index).filter(|slot| slot.role == entry.role) {
            *slot = entry;
        }
    });
}

/// The post-activation refresh (7).
fn refresh(game: &dyn Game, state: &State) {
    if let Some(count) = game.connection_count() {
        let count = i64::try_from(count).unwrap_or(i64::MAX);
        let old = state.connections.replace(count);
        if old != count {
            game.log(&lines::connections(old, count));
            with(&state.roles, |roles| roles.iter_mut().for_each(RoleEntry::rearm));
        }
    }
    let mut index = 0;
    while let Some(mut entry) = role_at(state, index) {
        let role = entry.role;
        if !role_live(game, role) {
            purge(game, state, role, PurgeReason::TickInvalidActor);
            continue;
        }
        index += 1;
        let now = game.now_ms();
        if !fully_active(game, role) {
            entry.rearm();
            set_role(state, index - 1, entry);
            continue;
        }
        if !entry.was_active {
            entry.was_active = true;
            entry.active_since_ms = now;
            set_role(state, index - 1, entry);
            quiet_line(game, || lines::refresh_wait(role, &role_item_id(game, role)));
            continue;
        }
        if entry.done
            || now.saturating_sub(entry.active_since_ms) < REFRESH_SETTLE_MS
            || (entry.last_push_ms != 0 && now.saturating_sub(entry.last_push_ms) < REFRESH_SETTLE_MS)
        {
            continue;
        }
        if entry.pushes >= REFRESH_PUSHES {
            entry.done = true;
            set_role(state, index - 1, entry);
            quiet_line(game, || {
                lines::refresh_done(role, &role_item_id(game, role), entry.pushes)
            });
            continue;
        }
        let mut none = [0u8; 0];
        game.invoke(role, "FlushNetDormancy", Route::Original, &mut none);
        game.invoke(role, "ForceNetUpdate", Route::Original, &mut none);
        entry.last_push_ms = now.max(1);
        entry.pushes += 1;
        set_role(state, index - 1, entry);
        let dormancy = game.u8_at(role + at::ACTOR_NET_DORMANCY).unwrap_or(0);
        quiet_line(game, || {
            lines::refresh_push(entry.pushes, role, &role_item_id(game, role), dormancy)
        });
    }
}

/// The modifier counts of 8.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    /// Desired buffs, effects, abilities (the catalog).
    pub desired: [i32; 3],
    /// Applied buffs, effects, abilities.
    pub applied: [i32; 3],
    /// Pending buffs, effects, abilities.
    pub pending: [i32; 3],
}

/// 8.2's snapshot; `None` when a count is unreadable or outside `0..=256`.
pub fn modifiers(game: &dyn Game, role: usize) -> Option<Modifiers> {
    if !game.readable(role, at::ROLE_SPAN) {
        return None;
    }
    let count = |addr: usize| game.i32_at(addr).filter(|count| (0..=COUNT_LIMIT).contains(count));
    let mut snap = Modifiers::default();
    for (slot, array) in [at::ROLE_BUFF_CLASSES, at::ROLE_EFFECT_CLASSES, at::ROLE_ABILITY_CLASSES]
        .into_iter()
        .enumerate()
    {
        snap.desired[slot] = count(role + array + at::ARRAY_NUM)?;
    }
    let group = game.ptr(role + at::ROLE_MODIFIER_GROUP);
    if group != 0 {
        if !game.readable(group, at::GROUP_SPAN) {
            return None;
        }
        snap.applied = [
            count(group + at::GROUP_APPLIED_BUFFS)?,
            count(group + at::GROUP_APPLIED_EFFECTS)?,
            count(group + at::GROUP_APPLIED_ABILITIES)?,
        ];
        snap.pending = [
            count(group + at::GROUP_PENDING_BUFFS)?,
            count(group + at::GROUP_PENDING_EFFECTS)?,
            count(group + at::GROUP_PENDING_ABILITIES)?,
        ];
    }
    Some(snap)
}

/// The `TArray` header at `array` as a 16-byte block, when its data is non-null, `0 < Num <= 256`, `Max >= Num`.
pub fn array_header(game: &dyn Game, array: usize) -> Option<[u8; 16]> {
    let mut header = [0u8; 16];
    if !game.read(array, &mut header) {
        return None;
    }
    let data = game.ptr(array);
    let num = game.i32_at(array + at::ARRAY_NUM)?;
    let max = game.i32_at(array + at::ARRAY_MAX)?;
    (data != 0 && num > 0 && num <= COUNT_LIMIT && max >= num).then_some(header)
}

fn tempest_at(state: &State, index: usize) -> Option<TempestEntry> {
    with(&state.tempest, |watch| watch.get(index).copied()).flatten()
}

fn set_tempest(state: &State, index: usize, entry: TempestEntry) {
    with(&state.tempest, |watch| {
        if let Some(slot) = watch.get_mut(index).filter(|slot| slot.role == entry.role) {
            *slot = entry;
        }
    });
}

fn drop_tempest(state: &State, role: usize) {
    with(&state.tempest, |watch| watch.retain(|entry| entry.role != role));
}

/// A state line at most every 10 s per entry.
fn state_line(game: &dyn Game, state: &State, index: usize, mut entry: TempestEntry, line: impl FnOnce() -> String) {
    let now = game.now_ms();
    if now.saturating_sub(entry.last_state_log_ms) >= TEMPEST_STATE_LOG_MS {
        entry.last_state_log_ms = now;
        set_tempest(state, index, entry);
        game.log(&line());
    }
}

/// Dispatches one repair call with a copy of the array header; whether it was made.
fn repair_call(game: &dyn Game, role: usize, function: &str, array: usize) -> bool {
    let Some(mut header) = array_header(game, role + array) else {
        return false;
    };
    game.invoke(role, function, Route::Original, &mut header) == Invoke::Called
}

/// The Tempest modifier repair (8.3).
fn tempest(game: &dyn Game, state: &State) {
    let mut index = 0;
    while let Some(mut entry) = tempest_at(state, index) {
        let role = entry.role;
        if !role_live(game, role) || !game.readable(role, at::ROLE_SPAN) || role_item_id(game, role) != PR_TEMPEST {
            drop_tempest(state, role);
            continue;
        }
        index += 1;
        let at_index = index - 1;
        let now = game.now_ms();
        if !fully_active(game, role) {
            entry.active_since_ms = 0;
            set_tempest(state, at_index, entry);
            continue;
        }
        if entry.active_since_ms == 0 {
            entry.active_since_ms = now.max(1);
            entry.last_state_log_ms = now;
            set_tempest(state, at_index, entry);
            game.log(&lines::tempest_active(role));
            continue;
        }
        let active_for = now.saturating_sub(entry.active_since_ms);
        if active_for < TEMPEST_SETTLE_MS {
            continue;
        }
        let Some(snap) = modifiers(game, role) else {
            state_line(game, state, at_index, entry, || lines::tempest_unreadable(role));
            continue;
        };
        let [applied_buffs, _, applied_abilities] = snap.applied;
        if applied_buffs > 0 && applied_abilities > 0 {
            game.log(&lines::tempest_healthy(role));
            drop_tempest(state, role);
            index -= 1;
            continue;
        }
        if snap.desired[0] <= 0 || snap.desired[2] <= 0 {
            state_line(game, state, at_index, entry, || lines::tempest_catalog_empty(role));
            continue;
        }
        if snap.pending[0] > 0 || snap.pending[2] > 0 {
            state_line(game, state, at_index, entry, || {
                lines::tempest_pending(role, active_for)
            });
            continue;
        }
        let need_buffs = applied_buffs <= 0;
        let need_abilities = applied_abilities <= 0;
        if !(need_buffs || need_abilities)
            || (entry.last_repair_ms != 0 && now.saturating_sub(entry.last_repair_ms) < TEMPEST_RETRY_MS)
        {
            continue;
        }
        if entry.repairs >= TEMPEST_REPAIRS {
            game.log(&lines::tempest_gave_up(role, entry.repairs));
            drop_tempest(state, role);
            index -= 1;
            continue;
        }
        entry.last_repair_ms = now.max(1);
        entry.repairs += 1;
        set_tempest(state, at_index, entry);
        let buffs = need_buffs && repair_call(game, role, "ApplyGameplayBuffsToOwner", at::ROLE_BUFF_CLASSES);
        let abilities =
            need_abilities && repair_call(game, role, "ApplyGameplayAbilitiesToOwner", at::ROLE_ABILITY_CLASSES);
        game.log(&lines::tempest_repair(
            entry.repairs,
            role,
            (need_buffs, buffs),
            (need_abilities, abilities),
        ));
    }
}

/// `ServerTryActivateAbility`'s block size.
pub const ABILITY_PARAMS: usize = 0x18;
/// `ServerTryActivateAbilityWithEventData`'s.
pub const ABILITY_EVENT_PARAMS: usize = 0xC8;
/// `TriggerEventData` in the long form.
const EVENT_DATA: usize = 0x18;

/// `SP-2@R14` (10), before the RPC; the original RPC always runs after.
pub fn ability(game: &dyn Game, state: &State, component: usize, params: usize, with_event_data: bool) {
    let size = if with_event_data {
        ABILITY_EVENT_PARAMS
    } else {
        ABILITY_PARAMS
    };
    if component == 0 || params == 0 || !game.readable(params, size) || !game.ability_ready() {
        return;
    }
    let (Some(handle), Some(pressed)) = (game.u32_at(params), game.u8_at(params + 4)) else {
        return;
    };
    let mut key = [0u8; 16];
    if !game.read(params + 8, &mut key) {
        return;
    }
    let pressed = pressed != 0;
    if pressed {
        game.invoke(
            component,
            "ServerSetInputPressed",
            Route::Hooked,
            &mut handle.to_le_bytes(),
        );
    }
    let event_data = if with_event_data { params + EVENT_DATA } else { 0 };
    let activated = game.try_activate(component, handle, &mut key, event_data);
    if !activated && pressed {
        game.invoke(
            component,
            "ServerSetInputReleased",
            Route::Hooked,
            &mut handle.to_le_bytes(),
        );
    }
    budgeted(game, &state.budgets.ability, ABILITY_KIND, true, || {
        let owner = if game.readable(component, at::ASC_OWNER_SPAN) {
            game.ptr(component + at::ASC_OWNER_ACTOR)
        } else {
            0
        };
        let name = (owner != 0).then(|| game.name(owner)).flatten();
        lines::ability(handle, pressed, activated, name.as_deref())
    });
}
