//! 12: the airship HUD.

use super::*;

/// A HUD and a game state, captured as `OnAirshipStateUpdate` names them.
fn captured(fake: &Fake, in_airship: bool) -> (usize, usize) {
    let hud = fake.block(0x400);
    let game_state = fake.block(0x300);
    fake.put_u8(game_state + 0x2D0, u8::from(in_airship));
    let params = fake.block(0x10);
    fake.put(params, game_state);
    hud_capture(fake, &fake.state, hud, params);
    fake.on("HideGameplayHUD", |fake, hud, _| fake.put_u8(hud + 0x361, 1));
    fake.on("ShowGameplayHUD", |fake, hud, _| fake.put_u8(hud + 0x361, 0));
    (hud, game_state)
}

fn control(fake: &Fake, value: i8) {
    let mut input = fake.state.input.borrow_mut();
    input.game_input = value;
    input.game_input_ms = fake.now.get();
}

#[test]
fn hidden_in_the_airship() {
    let fake = Fake::new();
    let (hud, _) = captured(&fake, true);
    reconcile(&fake, &fake.state, "state-update");
    assert_eq!(fake.calls_of("HideGameplayHUD").len(), 1);
    assert_eq!(fake.calls_of("HideGameplayHUD")[0].object, hud);
    assert!(fake.has_line("[MitHud] hide trigger=state-update visibility=0->1"));
    reconcile(&fake, &fake.state, "pump");
    assert_eq!(fake.count_lines("[MitHud] hide"), 1, "no change: no second line");
    assert!(fake.state.hud.borrow().hidden_by_us);
}

#[test]
fn restored_a_second_after_control_seen_after_the_end() {
    let fake = Fake::new();
    let (_, game_state) = captured(&fake, true);
    reconcile(&fake, &fake.state, "state-update");
    control(&fake, 1);
    fake.advance(10);
    fake.put_u8(game_state + 0x2D0, 0);
    reconcile(&fake, &fake.state, "state-update");
    assert!(fake.has_line("airship ended trigger=state-update"));
    fake.advance(5000);
    reconcile(&fake, &fake.state, "pump");
    assert!(
        fake.calls_of("ShowGameplayHUD").is_empty(),
        "control seen before the end does not count"
    );
    control(&fake, 1);
    fake.advance(999);
    reconcile(&fake, &fake.state, "pump");
    assert!(fake.calls_of("ShowGameplayHUD").is_empty(), "999 ms of control");
    fake.advance(1);
    reconcile(&fake, &fake.state, "hud-visibility-changed");
    assert_eq!(fake.calls_of("ShowGameplayHUD").len(), 1);
    assert!(fake.has_line("restore trigger=hud-visibility-changed reason=player-control sinceAirshipEndMs=6000"));
    assert!(fake.has_line("visibility=1->0"));
    assert!(!fake.state.hud.borrow().hidden_by_us);
}

#[test]
fn the_timeout_restores() {
    let fake = Fake::new();
    let (_, game_state) = captured(&fake, true);
    reconcile(&fake, &fake.state, "state-update");
    fake.put_u8(game_state + 0x2D0, 0);
    reconcile(&fake, &fake.state, "state-update");
    fake.advance(44_999);
    reconcile(&fake, &fake.state, "pump");
    assert!(fake.calls_of("ShowGameplayHUD").is_empty());
    fake.advance(1);
    reconcile(&fake, &fake.state, "pump");
    assert!(fake.has_line("reason=timeout sinceAirshipEndMs=45000"));
}

#[test]
fn busy_blocks_the_nested_trigger() {
    let fake = Fake::new();
    captured(&fake, true);
    fake.on("HideGameplayHUD", |fake, _, _| {
        reconcile(fake, &fake.state, "hud-visibility-changed")
    });
    reconcile(&fake, &fake.state, "state-update");
    assert_eq!(fake.calls_of("HideGameplayHUD").len(), 1);
    assert!(!fake.state.hud_busy.get(), "cleared on exit");
}

#[test]
fn dead_cached_pointers_forget_the_state() {
    let fake = Fake::new();
    let (hud, _) = captured(&fake, true);
    reconcile(&fake, &fake.state, "state-update");
    fake.kill(hud);
    reconcile(&fake, &fake.state, "pump");
    assert_eq!(*fake.state.hud.borrow(), Hud::default());
}

#[test]
fn not_after_an_airship_r14_did_not_hide() {
    let fake = Fake::new();
    captured(&fake, false);
    reconcile(&fake, &fake.state, "pump");
    assert!(fake.calls.borrow().is_empty());
    assert!(fake.take_lines().is_empty());
}

#[test]
fn a_missing_function_logs_once() {
    let fake = Fake::new();
    captured(&fake, true);
    fake.missing.borrow_mut().insert("HideGameplayHUD");
    reconcile(&fake, &fake.state, "pump");
    reconcile(&fake, &fake.state, "pump");
    assert_eq!(fake.take_lines(), ["[MitHud] HideGameplayHUD not found"]);
}
