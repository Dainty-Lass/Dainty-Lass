//! The client (11): the possession reset, the pump, `OnInputModeChanged`, the input cascade, the role lifecycle
//! replay and the movement repair.

use super::apply::snapshot;
use super::lines::{self, INPUT, INPUT_MODE_KIND};
use super::{
    ACTIVATION_RETRY_MS, ACTIVATION_TIMEOUT_MS, Attempts, CLIENT_PUMP_MS, Game, Input, Invoke, LIFECYCLE_ATTEMPTS,
    LIFECYCLE_INTERVAL_MS, LIFECYCLE_PAUSE_MS, Lifecycle, REPLAYED_PAIRS, Restore, Route, STABLE_WAIT_LOG_MS, State,
    budgeted, fstring, hud, with,
};
use crate::offsets::r7;
use crate::offsets::r14 as at;

/// After `ClientRestart`, past R7's capture (11.1).
pub fn client_restart(game: &dyn Game, state: &State) {
    let Some(captured) = game.captured() else {
        return;
    };
    let fresh = with(&state.input, |input| {
        if input.generation == Some(captured.generation) {
            return false;
        }
        *input = Input {
            generation: Some(captured.generation),
            ..Input::new()
        };
        true
    });
    if fresh != Some(true) {
        return;
    }
    state.activation_depth.set(0);
    with(&state.retry, |table| *table = [None; super::RETRY_ENTRIES]);
    with(&state.lifecycle, |life| *life = Lifecycle::new());
    with(&state.sheathe, |watch| *watch = super::Sheathe::default());
    game.log(&lines::new_possession(captured.pc, captured.pawn));
}

fn input(state: &State) -> Input {
    with(&state.input, |input| *input).unwrap_or_default()
}

fn set_input(state: &State, body: impl FnOnce(&mut Input)) {
    with(&state.input, body);
}

/// The client pump (11.2): before every outermost call, at most every 100 ms.
pub fn client_pump(game: &dyn Game, state: &State, depth: u32) {
    if depth != 1 {
        return;
    }
    let now = game.now_ms();
    if input(state)
        .pump_ms
        .is_some_and(|last| now.saturating_sub(last) < CLIENT_PUMP_MS)
    {
        return;
    }
    set_input(state, |input| input.pump_ms = Some(now));
    let Some(captured) = game.captured() else {
        return;
    };
    let stable = game.possession_stable(captured.pc);
    if stable {
        super::retry(game, state, false);
        if !game.lifecycle_replay_off() {
            replay(game, state, captured.pc, "pump");
        }
        hud::reconcile(game, state, "pump");
        super::sheathe_check(game, state, captured.pc);
    } else {
        let due = input(state)
            .wait_log_ms
            .is_none_or(|last| now.saturating_sub(last) >= STABLE_WAIT_LOG_MS);
        if due {
            set_input(state, |input| input.wait_log_ms = Some(now));
            game.log(lines::WAITING_STABLE);
        }
        return;
    }
    let state_now = input(state);
    if state_now.activated
        || state_now
            .attempt_ms
            .is_some_and(|last| now.saturating_sub(last) < ACTIVATION_RETRY_MS)
    {
        return;
    }
    let since = now.saturating_sub(captured.since_ms);
    if state_now.pending {
        game.log(&lines::deferred_replayed(since));
        activate(game, state, captured.pc, "deferred-event", true);
        if input(state).activated {
            set_input(state, |input| input.pending = false);
        }
    }
    if !input(state).activated && since >= ACTIVATION_TIMEOUT_MS {
        game.log(&lines::activation_timeout(since));
        activate(game, state, captured.pc, "timeout", false);
    }
    if !input(state).activated && (state_now.pending || since >= ACTIVATION_TIMEOUT_MS) {
        set_input(state, |input| input.attempt_ms = Some(now));
    }
}

/// After `OnInputModeChanged` on `pawn` (11.3).
pub fn input_mode_changed(game: &dyn Game, state: &State, pawn: usize, params: usize) {
    let Some(game_input) = (params != 0).then(|| game.u8_at(params)).flatten() else {
        movement(game, state, pawn);
        return;
    };
    let game_input = game_input != 0;
    let from_runtime = state.activation_depth.get() > 0;
    let captured = game.captured();
    let current = captured.is_some_and(|captured| game.chain_for(captured.pc, Some(pawn)).is_some());
    let now = game.now_ms();
    if !from_runtime && current {
        set_input(state, |input| {
            input.game_input = i8::from(game_input);
            input.game_input_ms = now;
        });
    }
    budgeted(game, &state.budgets.input_mode, INPUT_MODE_KIND, false, || {
        lines::input_mode(game_input, from_runtime)
    });
    if let Some(captured) = captured.filter(|_| !input(state).activated) {
        if !current {
            game.log(lines::IGNORED_OTHER);
        } else {
            set_input(state, |input| input.edge = true);
            if game.possession_stable(captured.pc) {
                activate(game, state, captured.pc, "event", true);
            } else {
                set_input(state, |input| input.pending = true);
                game.log(&lines::activation_deferred(now.saturating_sub(captured.since_ms)));
            }
        }
    }
    movement(game, state, pawn);
}

/// A call whose missing function logs once per name (11.4).
fn call(game: &dyn Game, state: &State, object: usize, class: &str, function: &str, params: &mut [u8]) -> Invoke {
    let outcome = game.invoke(object, function, Route::Original, params);
    if outcome == Invoke::NotFound {
        let label = format!("{class}.{function}");
        state.once(game, &label, || lines::not_found(INPUT, &label));
    }
    outcome
}

/// The input cascade (11.4).
pub fn activate(game: &dyn Game, state: &State, pc: usize, trigger: &str, allow_cinematic_clear: bool) {
    let depth = state.activation_depth.get();
    let _restore = Restore {
        cell: &state.activation_depth,
        value: depth,
    };
    state.activation_depth.set(depth + 1);
    if pc == 0 || !game.readable(pc, r7::PC_SPAN_LOCAL) || !game.is_live_object(pc) {
        return;
    }
    let ready = pc + at::PC_ONLINE_DATA_READY;
    let Some(was) = game.u8_at(ready) else {
        return;
    };
    if was != 1 {
        game.write(ready, &[1]);
        let now = game.u8_at(ready).unwrap_or(was);
        game.log(&lines::online_data_ready(trigger, was, now));
        if now != 1 {
            return;
        }
    }
    // Before any call: EnableCharacterInput raises OnInputModeChanged itself, which re-entered the cascade
    set_input(state, |input| input.activated = true);
    call(
        game,
        state,
        pc,
        "ArchonPlayerController",
        "EnableCharacterInput",
        &mut [1],
    );
    loadout_and_role(game, state, pc);
    let mut none = [0u8; 0];
    call(game, state, pc, "Controller", "ResetIgnoreMoveInput", &mut none);
    call(game, state, pc, "Controller", "ResetIgnoreLookInput", &mut none);
    cinematic(game, state, pc, trigger, allow_cinematic_clear);
    input_configuration(game, state, pc);
    call(game, state, pc, "ArchonPlayerController", "SetupArchonInput", &mut none);
    let pawn = game.ptr(pc + r7::PC_PAWN);
    if pawn != 0 && game.is_live_object(pawn) {
        call(game, state, pawn, "ArchonCharacter", "SetupArchonInput", &mut none);
    }
    hud::reconcile(game, state, "input-activation");
}

fn ptr_of(bytes: &[u8]) -> usize {
    let mut word = [0u8; 8];
    word.copy_from_slice(&bytes[..8]);
    usize::try_from(u64::from_le_bytes(word)).unwrap_or(0)
}

/// `GetLoadout` on the controller: the loadout, 0 when none or not called.
pub(super) fn get_loadout(game: &dyn Game, state: &State, pc: usize) -> usize {
    let mut block = [0u8; 8];
    if call(game, state, pc, "ArchonPlayerController", "GetLoadout", &mut block) == Invoke::Called {
        ptr_of(&block)
    } else {
        0
    }
}

/// 11.4 step 4: the active slot rule, then the role re-applied through PR-1's wrapper.
fn loadout_and_role(game: &dyn Game, state: &State, pc: usize) {
    let loadout = get_loadout(game, state, pc);
    if loadout == 0 {
        game.log(lines::NO_LOADOUT);
        return;
    }
    let mut block = [0u8; 4];
    if call(
        game,
        state,
        loadout,
        "ArchonLoadout",
        "GetActiveLoadoutSlotIndex",
        &mut block,
    ) != Invoke::Called
    {
        return;
    }
    let mut slot = i32::from_le_bytes(block);
    if (0..6).contains(&slot) {
        game.log(&lines::slot_kept(slot));
    } else {
        let mut set = [0u8; 8];
        set[4] = 1;
        call(
            game,
            state,
            loadout,
            "ArchonLoadout",
            "ServerInternalSetActiveLoadoutSlotIndex",
            &mut set,
        );
        slot = 0;
        game.log(lines::SLOT_INVALID);
    }
    game.apply(loadout, true);
    game.log(&lines::role_reapplied(slot));
    if game.possession_stable(pc) && !game.lifecycle_replay_off() {
        replay(game, state, pc, "post-apply");
    } else {
        game.log(lines::LIFECYCLE_DEFERRED);
    }
}

/// 11.4 step 6: only once the game handed control to the player.
fn cinematic(game: &dyn Game, state: &State, pc: usize, trigger: &str, allow: bool) {
    let now = input(state);
    let stable = game.possession_stable(pc);
    if allow && now.edge && now.game_input == 1 && stable {
        call(game, state, pc, "PlayerController", "SetCinematicMode", &mut [0u8; 5]);
        game.log(lines::CINEMATIC_CLEARED);
    } else {
        game.log(&lines::cinematic_skipped(
            trigger,
            allow,
            now.edge,
            now.game_input,
            stable,
        ));
    }
}

/// 11.4 step 7: Hunter, then Default, whatever Hunter returned (call 4).
fn input_configuration(game: &dyn Game, state: &State, pc: usize) {
    let apply = |configuration: u8| {
        let mut block = [configuration, 0];
        (call(
            game,
            state,
            pc,
            "ArchonPlayerControllerBase",
            "ApplyInputConfiguration",
            &mut block,
        ) == Invoke::Called)
            .then_some(block[1])
    };
    let hunter = apply(1);
    let default = apply(0);
    game.log(&lines::input_configuration(hunter, default));
    call(
        game,
        state,
        pc,
        "ArchonPlayerControllerBase",
        "ApplyInputConfigurationFromSettings",
        &mut [0u8; 0],
    );
}

fn gameplay_bytes(game: &dyn Game, role: usize) -> lines::Bytes3 {
    [
        at::ROLE_GAMEPLAY_EQUIPPED,
        at::ROLE_ACTIVE_GAMEPLAY,
        at::ROLE_BP_EQUIP_CALLED,
    ]
    .map(|offset| game.u8_at(role + offset).unwrap_or(0))
}

/// Whether an attempt is due for (pawn, id), counting it (11.5 step 2).
fn attempt_due(game: &dyn Game, state: &State, pawn: usize, id: &str) -> Option<u32> {
    let now = game.now_ms();
    let (due, resumed) = with(&state.lifecycle, |life| {
        let attempts = life.attempts.get_or_insert_with(|| Attempts {
            pawn,
            id: id.to_owned(),
            ..Attempts::default()
        });
        if attempts.pawn != pawn || attempts.id != id {
            *attempts = Attempts {
                pawn,
                id: id.to_owned(),
                ..Attempts::default()
            };
        }
        let mut resumed = false;
        if attempts.count >= LIFECYCLE_ATTEMPTS {
            let exhausted = *attempts.exhausted_ms.get_or_insert(now);
            if now.saturating_sub(exhausted) < LIFECYCLE_PAUSE_MS {
                return (None, false);
            }
            attempts.count = 0;
            attempts.exhausted_ms = None;
            attempts.last_ms = None;
            resumed = true;
        }
        if attempts
            .last_ms
            .is_some_and(|last| now.saturating_sub(last) < LIFECYCLE_INTERVAL_MS)
        {
            return (None, resumed);
        }
        attempts.count += 1;
        attempts.last_ms = Some(now);
        if attempts.count >= LIFECYCLE_ATTEMPTS {
            attempts.exhausted_ms = Some(now);
        }
        (Some(attempts.count), resumed)
    })
    .unwrap_or((None, false));
    if resumed {
        game.log(lines::CLIENT_PAUSE_OVER);
    }
    due
}

/// 11.5 step 2: no role on the pawn though the replicated id names one.
fn install_role(game: &dyn Game, state: &State, pc: usize, pawn: usize, player_state: usize) -> usize {
    if !game.readable(player_state, at::PS_PLAYER_ROLE_ID_SPAN) {
        return 0;
    }
    let id = fstring(game, player_state + at::PS_PLAYER_ROLE_ID);
    if id.is_empty() {
        let first = with(&state.lifecycle, |life| {
            std::mem::replace(&mut life.empty_logged, pawn) != pawn
        });
        if first == Some(true) {
            game.log(lines::CLIENT_ID_EMPTY);
        }
        return 0;
    }
    let Some(attempt) = attempt_due(game, state, pawn, &id) else {
        return 0;
    };
    let loadout = get_loadout(game, state, pc);
    if loadout == 0 || !game.is_live_object(loadout) {
        game.log(lines::CLIENT_NO_LOADOUT);
        return 0;
    }
    if let Some(snap) = snapshot(game, loadout)
        && !snap.applied()
        && snap.role_item != 0
        && snap.slot != 0
        && snap.pawn != 0
    {
        game.apply(loadout, false);
    }
    let role = game.ptr(pawn + at::CHARACTER_PLAYER_ROLE);
    game.log(&lines::client_reapplied(attempt, &id, role));
    role
}

/// The role lifecycle replay (11.5).
pub fn replay(game: &dyn Game, state: &State, pc: usize, trigger: &str) {
    if pc == 0 || !game.readable(pc, at::PC_PAWN_SPAN) || !game.is_live_object(pc) {
        return;
    }
    let pawn = game.ptr(pc + r7::PC_PAWN);
    if pawn == 0 || !game.readable(pawn, at::CHARACTER_SPAN) {
        return;
    }
    let player_state = game.ptr(pawn + r7::PAWN_PLAYER_STATE);
    if player_state == 0 || !game.readable(player_state, at::PS_ACTIVE_GAMEPLAY_SPAN) {
        return;
    }
    let mut role = game.ptr(pawn + at::CHARACTER_PLAYER_ROLE);
    if role == 0 {
        role = install_role(game, state, pc, pawn, player_state);
        if role == 0 {
            return;
        }
    }
    if !game.readable(role, at::ROLE_LIFECYCLE_SPAN) {
        return;
    }
    let before = gameplay_bytes(game, role);
    if game.u8_at(player_state + at::PS_ACTIVE_GAMEPLAY) != Some(1) || before == [1, 1, 1] {
        return;
    }
    // Once per pair: on the owning client these bytes can stay 0 while the role works
    let first = with(&state.lifecycle, |life| {
        if life.replayed.contains(&(pawn, role)) {
            return false;
        }
        if life.replayed.len() >= REPLAYED_PAIRS {
            life.replayed.remove(0);
        }
        life.replayed.push((pawn, role));
        true
    });
    if first != Some(true) {
        return;
    }
    call(
        game,
        state,
        pawn,
        "ArchonCharacter",
        "HandleActiveGameplayStateChanged",
        &mut [1, 1],
    );
    let after = gameplay_bytes(game, role);
    game.log(&lines::client_replayed(trigger, before, after));
}

/// One movement write: logs once per process per field (11.6).
fn repair(game: &dyn Game, state: &State, field: &str, addr: usize, old: u8, new: u8) {
    if game.write(addr, &[new]) {
        state.once(game, &format!("movement:{field}"), || {
            lines::movement_repair(field, old, new)
        });
    }
}

/// The movement repair on `pawn` (11.6): each a no-op on a healthy pawn.
pub fn movement(game: &dyn Game, state: &State, pawn: usize) {
    if pawn == 0 || !game.readable(pawn, at::PAWN_MOVEMENT_SPAN) {
        return;
    }
    if let Some(old) = game.u8_at(pawn + at::PAWN_CAN_MOVE).filter(|&value| value != 1) {
        repair(game, state, "CanMove_", pawn + at::PAWN_CAN_MOVE, old, 1);
        let previous = game.u8_at(pawn + at::PAWN_PREVIOUS_CAN_MOVE).unwrap_or(0);
        repair(
            game,
            state,
            "PreviousCanMove",
            pawn + at::PAWN_PREVIOUS_CAN_MOVE,
            previous,
            1,
        );
    }
    let movement = game.ptr(pawn + at::PAWN_CHARACTER_MOVEMENT);
    if movement != 0 && game.readable(movement, at::CMC_SPAN) && game.u8_at(movement + at::CMC_MOVEMENT_MODE) == Some(0)
    {
        repair(game, state, "MovementMode", movement + at::CMC_MOVEMENT_MODE, 0, 1);
        let default = game.u8_at(movement + at::CMC_DEFAULT_LAND_MOVEMENT_MODE).unwrap_or(0);
        repair(
            game,
            state,
            "DefaultLandMovementMode",
            movement + at::CMC_DEFAULT_LAND_MOVEMENT_MODE,
            default,
            1,
        );
        let replicated = game.u8_at(pawn + at::PAWN_REPLICATED_MOVEMENT_MODE).unwrap_or(0);
        repair(
            game,
            state,
            "ReplicatedMovementMode",
            pawn + at::PAWN_REPLICATED_MOVEMENT_MODE,
            replicated,
            1,
        );
    }
    let root = game.ptr(pawn + at::PAWN_ROOT_COMPONENT);
    if root != 0
        && game.readable(root, at::SCENE_SPAN)
        && let Some(old) = game.u8_at(root + at::SCENE_MOBILITY).filter(|&value| value != 2)
    {
        repair(game, state, "Mobility", root + at::SCENE_MOBILITY, old, 2);
    }
    if let Some(old) = game.u8_at(pawn + at::ACTOR_ROLE).filter(|&role| role != 2 && role != 3) {
        repair(game, state, "Role", pawn + at::ACTOR_ROLE, old, 2);
    }
}
