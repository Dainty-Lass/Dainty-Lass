//! The airship HUD (12): hidden while `IsInPreMatchAirship`, back once the player has control. Only the game's
//! reflected functions are called; no widget or visibility byte is written.

use super::lines::{self, HIDE_KIND, HUD, RestoreReason};
use super::{Game, HUD_CONTROL_MS, HUD_TIMEOUT_MS, Hud, Invoke, Restore, Route, State, budgeted, with};
use crate::offsets::r14 as at;

/// Before `OnAirshipStateUpdate` (12.1): the HUD and the game state its params name.
pub fn capture(game: &dyn Game, state: &State, hud: usize, params: usize) {
    if params == 0 || !game.readable(params, 8) {
        return;
    }
    let game_state = game.ptr(params);
    if game_state != 0 && game.readable(game_state, at::GAME_STATE_SPAN) {
        with(&state.hud, |held| {
            held.hud = hud;
            held.game_state = game_state;
        });
    }
}

fn visibility(game: &dyn Game, hud: usize) -> u8 {
    game.u8_at(hud + at::HUD_GAMEPLAY_VISIBILITY).unwrap_or(0)
}

/// `HideGameplayHUD` or `ShowGameplayHUD`; false (after one line per process) when not found.
fn show(game: &dyn Game, state: &State, hud: usize, function: &str) -> bool {
    let outcome = game.invoke(hud, function, Route::Original, &mut [0u8; 0]);
    if outcome == Invoke::NotFound {
        state.once(game, function, || lines::not_found(HUD, function));
        return false;
    }
    true
}

/// The reconcile (12.3).
pub fn reconcile(game: &dyn Game, state: &State, trigger: &str) {
    if state.hud_busy.get() {
        return;
    }
    let Some(held) = with(&state.hud, |held| *held) else {
        return;
    };
    if held.hud == 0 || held.game_state == 0 {
        return;
    }
    if !game.is_live_object(held.hud) || !game.is_live_object(held.game_state) {
        with(&state.hud, |held| *held = Hud::default());
        return;
    }
    if !game.readable(held.hud, at::HUD_SPAN) || !game.readable(held.game_state, at::GAME_STATE_SPAN) {
        return;
    }
    let in_airship = game.u8_at(held.game_state + at::GAME_STATE_IN_PRE_MATCH_AIRSHIP) == Some(1);
    let before = visibility(game, held.hud);
    let now = game.now_ms();
    let _restore = Restore {
        cell: &state.hud_busy,
        value: false,
    };
    if in_airship {
        with(&state.hud, |held| held.airship_ended_ms = 0);
        state.hud_busy.set(true);
        if !show(game, state, held.hud, "HideGameplayHUD") {
            return;
        }
        let after = visibility(game, held.hud);
        if !held.hidden_by_us || before != after {
            budgeted(game, &state.budgets.hide, HIDE_KIND, false, || {
                lines::hud_hide(trigger, before, after)
            });
        }
        with(&state.hud, |held| held.hidden_by_us = true);
        return;
    }
    if !held.hidden_by_us {
        return;
    }
    let mut ended = held.airship_ended_ms;
    if ended == 0 {
        ended = now.max(1);
        with(&state.hud, |held| held.airship_ended_ms = ended);
        game.log(&lines::airship_ended(trigger));
    }
    let input = with(&state.input, |input| *input).unwrap_or_default();
    let reason = if input.game_input == 1
        && input.game_input_ms >= ended
        && now.saturating_sub(input.game_input_ms) >= HUD_CONTROL_MS
    {
        RestoreReason::PlayerControl
    } else if now.saturating_sub(ended) >= HUD_TIMEOUT_MS {
        RestoreReason::Timeout
    } else {
        return;
    };
    state.hud_busy.set(true);
    if !show(game, state, held.hud, "ShowGameplayHUD") {
        return;
    }
    let after = visibility(game, held.hud);
    game.log(&lines::hud_restore(
        trigger,
        reason,
        now.saturating_sub(ended),
        before,
        after,
    ));
    with(&state.hud, |held| {
        held.hidden_by_us = false;
        held.airship_ended_ms = 0;
    });
}
