//! `AUTO_PRESS_START.flag`: the three calls a player's clicks would make, for scripted runs (spec-runtime-r17 5).
//! The 1.14.7 press-start popup ignores synthetic input, so a script cannot get past it otherwise.

use crate::ffi::{engine, flags, log, reflect};
use crate::handlers::{PeCall, Verdict};
use rustic_isles_runtime_core::branding::{AutoLogin, flags as names, function_short_name};
use std::sync::Mutex;

static STATE: Mutex<AutoLogin> = Mutex::new(AutoLogin::new());

/// Takes the state; a nested call (the steps call into the game) finds it taken and does nothing.
fn with_state<R>(work: impl FnOnce(&mut AutoLogin) -> R) -> Option<R> {
    STATE.try_lock().ok().map(|mut state| work(&mut state))
}

pub(super) fn before(call: &PeCall) -> Verdict {
    if !flags::flag(names::AUTO_PRESS_START) {
        return Verdict::Continue;
    }
    let Some(full_name) = call.entry.full_name() else {
        return Verdict::Continue;
    };
    if !full_name.contains("LoginScreen") || !engine::is_live_object(call.object) {
        return Verdict::Continue;
    }
    match function_short_name(&full_name) {
        "OnStartupMovieFinished" => press_start(call.object),
        "OnPlayScreenPollTimer" => poll(call.object),
        _ => {}
    }
    Verdict::Continue
}

fn press_start(screen: usize) {
    if with_state(AutoLogin::press_start) != Some(true) {
        return;
    }
    let Some(function) =
        reflect::class_of(screen).and_then(|class| reflect::function(class, "OnPressToStartDismissed"))
    else {
        log::line("[MitAutoLogin] OnPressToStartDismissed not found; leaving the screen alone");
        return;
    };
    let mut params = reflect::Params::new(4);
    let called = reflect::call(screen, function, &mut params);
    log::line(&format!(
        "[MitAutoLogin] press start: OnPressToStartDismissed(UserIndex = 0) {}",
        if called { "called" } else { "not called" }
    ));
}

fn poll(screen: usize) {
    // The counts are taken under the lock; the calls into the game run after it is released
    let Some(plan) = with_state(AutoLogin::poll) else {
        return;
    };
    if let Some(count) = plan.play {
        play(screen, count);
    }
    if let Some(count) = plan.travel {
        travel(count);
    }
}

fn play(screen: usize, count: u32) {
    let clicked = reflect::function_matching(screen, |name| {
        name.contains("w_loginbutton") && name.contains("Clicked")
    });
    let Some(function) = clicked else {
        if AutoLogin::log_miss(count) {
            log::line(&format!("[MitAutoLogin] Play click event not found yet (poll {count})"));
        }
        return;
    };
    // The event takes no parameters; a zeroed block of 256 bytes as the AGPL passed
    let mut params = reflect::Params::new(256);
    if reflect::call(screen, function, &mut params) {
        with_state(AutoLogin::play_done);
        log::line(&format!("[MitAutoLogin] Play pressed (poll {count})"));
    }
}

fn travel(count: u32) {
    let Some(party) = reflect::first_instance("ArchonPartyClient") else {
        if AutoLogin::log_miss(count) {
            log::line(&format!("[MitAutoLogin] no ArchonPartyClient yet (poll {count})"));
        }
        return;
    };
    let Some(function) = reflect::class_of(party).and_then(|class| reflect::function(class, "StartTravelToCity"))
    else {
        with_state(AutoLogin::travel_done);
        log::line("[MitAutoLogin] ArchonPartyClient.StartTravelToCity not found; not travelling");
        return;
    };
    let mut params = reflect::Params::new(1);
    if reflect::call(party, function, &mut params) {
        with_state(AutoLogin::travel_done);
        log::line(&format!(
            "[MitAutoLogin] travel to Ramsgate: StartTravelToCity(bLeaveWithParty = false) (poll {count})"
        ));
    }
}
