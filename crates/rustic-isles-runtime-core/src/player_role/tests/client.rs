//! 11: the possession reset, input activation, the lifecycle replay and the movement repair.

use super::*;

/// A captured possession of `world`'s controller and pawn, the controller's online data ready.
fn possessed(fake: &Fake, world: &World) {
    fake.captured.set(Some(Captured {
        pc: world.pc,
        pawn: world.pawn,
        since_ms: fake.now.get(),
        generation: 1,
    }));
    fake.chain_pawn.set(world.pawn);
    fake.put_u8(world.pc + 0xC69, 1);
    fake.loadout.set(world.loadout);
    fake.on("GetLoadout", |fake, _, params| {
        params[..8].copy_from_slice(&(fake.loadout.get() as u64).to_le_bytes());
    });
    fake.on("GetActiveLoadoutSlotIndex", |fake, _, params| {
        params[..4].copy_from_slice(&fake.slot_index.get().to_le_bytes());
    });
    client_restart(fake, &fake.state);
}

fn input(fake: &Fake) -> Input {
    *fake.state.input.borrow()
}

/// `OnInputModeChanged` on `pawn` with `bGameInputEnabled`.
fn event(fake: &Fake, pawn: usize, enabled: bool) {
    let params = fake.block(0x10);
    fake.put_u8(params, u8::from(enabled));
    input_mode_changed(fake, &fake.state, pawn, params);
}

fn enables(fake: &Fake) -> usize {
    fake.calls_of("EnableCharacterInput").len()
}

#[test]
fn a_new_possession_resets_once() {
    let fake = Fake::new();
    let world = fake.world(Some("PR_TEMPEST"), None);
    note(&fake, &fake.state, world.loadout, false);
    possessed(&fake, &world);
    assert!(fake.has_line("new possession pc="));
    assert!(
        fake.state.retry.borrow().iter().all(Option::is_none),
        "a new possession clears the retry table"
    );
    fake.state.input.borrow_mut().activated = true;
    client_restart(&fake, &fake.state);
    assert!(input(&fake).activated, "the same pair changes nothing");
    assert_eq!(fake.count_lines("new possession"), 1);
}

#[test]
fn a_runtime_raised_event_does_not_update_the_game_input() {
    let fake = Fake::new();
    let world = fake.world(None, None);
    possessed(&fake, &world);
    fake.state.activation_depth.set(1);
    event(&fake, world.pawn, true);
    assert_eq!(input(&fake).game_input, -1);
    assert!(fake.has_line("input mode gameInput=1 source=runtime"));
    fake.state.activation_depth.set(0);
    event(&fake, world.pawn, true);
    assert_eq!(input(&fake).game_input, 1);
    assert!(fake.has_line("source=game"));
}

#[test]
fn a_pawn_that_is_not_current_is_ignored() {
    let fake = Fake::new();
    let world = fake.world(None, None);
    possessed(&fake, &world);
    fake.stable.set(true);
    let other = fake.block(0x1700);
    event(&fake, other, true);
    assert!(fake.has_line("ignored an input mode change from another controller or pawn"));
    assert_eq!(enables(&fake), 0);
    assert_eq!(input(&fake).game_input, -1);
}

#[test]
fn stable_runs_the_cascade_and_unstable_defers_it_to_the_pump() {
    let fake = Fake::new();
    let world = fake.world(None, None);
    possessed(&fake, &world);
    fake.advance(1200);
    event(&fake, world.pawn, true);
    assert!(input(&fake).pending);
    assert!(fake.has_line("activate: deferred, 1200 ms since the possession (settle 3000 ms)"));
    assert_eq!(enables(&fake), 0);
    client_pump(&fake, &fake.state, 1);
    assert!(fake.has_line("waiting for a stable possession; pump skipped"));
    fake.stable.set(true);
    fake.advance(2000);
    client_pump(&fake, &fake.state, 2);
    assert_eq!(enables(&fake), 0, "only outermost calls pump");
    client_pump(&fake, &fake.state, 1);
    assert!(fake.has_line("activate: deferred event replayed after 3200 ms"));
    assert_eq!(enables(&fake), 1);
    assert!(input(&fake).activated && !input(&fake).pending);

    let fake = Fake::new();
    let world = fake.world(None, None);
    possessed(&fake, &world);
    fake.stable.set(true);
    event(&fake, world.pawn, true);
    assert_eq!(enables(&fake), 1, "stable: the cascade at once");
    event(&fake, world.pawn, false);
    assert_eq!(enables(&fake), 1, "once activated, events do not run it again");
}

#[test]
fn the_timeout_restores_input_without_the_cinematic_clear() {
    let fake = Fake::new();
    let world = fake.world(None, None);
    possessed(&fake, &world);
    fake.stable.set(true);
    fake.advance(7999);
    client_pump(&fake, &fake.state, 1);
    assert_eq!(enables(&fake), 0);
    fake.advance(101);
    client_pump(&fake, &fake.state, 1);
    assert!(fake.has_line("activate: timeout after 8100 ms, restoring input without the cinematic clear"));
    assert_eq!(enables(&fake), 1);
    assert!(fake.calls_of("SetCinematicMode").is_empty());
    assert!(fake.has_line("cinematic clear skipped trigger=timeout allow=0"));
}

#[test]
fn the_cinematic_clear_needs_all_four() {
    let run = |allow: bool, edge: bool, game_input: i8, stable: bool| {
        let fake = Fake::new();
        let world = fake.world(None, None);
        possessed(&fake, &world);
        fake.stable.set(stable);
        {
            let mut held = fake.state.input.borrow_mut();
            held.edge = edge;
            held.game_input = game_input;
        }
        activate(&fake, &fake.state, world.pc, "test", allow);
        fake.calls_of("SetCinematicMode")
    };
    let cleared = run(true, true, 1, true);
    assert_eq!(cleared.len(), 1);
    assert_eq!(cleared[0].params, [0u8; 5]);
    assert!(run(false, true, 1, true).is_empty());
    assert!(run(true, false, 1, true).is_empty());
    assert!(run(true, true, 0, true).is_empty());
    assert!(run(true, true, -1, true).is_empty());
    assert!(run(true, true, 1, false).is_empty());
}

#[test]
fn online_data_ready_is_written_and_read_back() {
    let fake = Fake::new();
    let world = fake.world(None, None);
    possessed(&fake, &world);
    fake.put_u8(world.pc + 0xC69, 0);
    activate(&fake, &fake.state, world.pc, "event", true);
    assert_eq!(fake.byte(world.pc + 0xC69), 1);
    assert!(fake.has_line("activate trigger=event: bOnlineDataReady was 0, now 1"));
    assert_eq!(enables(&fake), 1);

    let fake = Fake::new();
    let world = fake.world(None, None);
    possessed(&fake, &world);
    fake.put_u8(world.pc + 0xC69, 0);
    fake.read_only(world.pc);
    activate(&fake, &fake.state, world.pc, "event", true);
    assert!(fake.has_line("bOnlineDataReady was 0, now 0"));
    assert_eq!(enables(&fake), 0, "a failed write stops the cascade");
    assert!(!input(&fake).activated);
}

#[test]
fn input_is_activated_before_the_first_call() {
    let fake = Fake::new();
    let world = fake.world(None, None);
    possessed(&fake, &world);
    fake.stable.set(true);
    fake.on("EnableCharacterInput", |fake, _, _| {
        // The game raises OnInputModeChanged from inside EnableCharacterInput
        let pawn = fake.chain_pawn.get();
        let params = fake.block(0x10);
        fake.put_u8(params, 1);
        input_mode_changed(fake, &fake.state, pawn, params);
    });
    event(&fake, world.pawn, true);
    assert_eq!(enables(&fake), 1, "the nested event does not re-run the cascade");
    assert!(fake.has_line("source=runtime"));
    assert_eq!(fake.state.activation_depth.get(), 0);
}

#[test]
fn the_slot_rule() {
    for (slot, kept) in [(-1, false), (6, false), (0, true), (3, true), (5, true)] {
        let fake = Fake::new();
        let world = fake.world(None, None);
        possessed(&fake, &world);
        fake.slot_index.set(slot);
        activate(&fake, &fake.state, world.pc, "event", true);
        let set = fake.calls_of("ServerInternalSetActiveLoadoutSlotIndex");
        if kept {
            assert!(set.is_empty(), "{slot}");
            assert!(fake.has_line(&format!("kept the active loadout slot {slot}")));
            assert!(fake.has_line(&format!("re-applied the role for slot {slot}")));
        } else {
            assert_eq!(set.len(), 1, "{slot}");
            assert_eq!(set[0].params, [0, 0, 0, 0, 1, 0, 0, 0]);
            assert!(fake.has_line("active loadout slot was invalid; slot 0 applied"));
            assert!(fake.has_line("re-applied the role for slot 0"));
        }
        assert_eq!(*fake.applies.borrow(), [(world.loadout, true)], "through the wrapper");
    }
    let fake = Fake::new();
    let world = fake.world(None, None);
    possessed(&fake, &world);
    fake.loadout.set(0);
    activate(&fake, &fake.state, world.pc, "event", true);
    assert!(fake.has_line("no loadout; slot and role skipped"));
    assert!(fake.applies.borrow().is_empty());
}

#[test]
fn both_input_configurations_whatever_the_return() {
    for answer in [0u8, 1] {
        let fake = Fake::new();
        let world = fake.world(None, None);
        possessed(&fake, &world);
        fake.slot_index.set(answer.into());
        fake.on("ApplyInputConfiguration", |fake, _, params| {
            params[1] = u8::try_from(fake.slot_index.get()).unwrap_or(0);
        });
        activate(&fake, &fake.state, world.pc, "event", true);
        let calls = fake.calls_of("ApplyInputConfiguration");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].params[0], 1, "Hunter first");
        assert_eq!(calls[1].params[0], 0, "then Default");
        assert!(fake.has_line(&format!("input configuration Hunter -> {answer}, Default -> {answer}")));
        assert_eq!(fake.calls_of("ApplyInputConfigurationFromSettings").len(), 1);
    }
}

#[test]
fn the_cascade_calls_in_order_and_logs_missing_functions_once() {
    let fake = Fake::new();
    let world = fake.world(None, None);
    possessed(&fake, &world);
    fake.missing.borrow_mut().insert("ResetIgnoreLookInput");
    activate(&fake, &fake.state, world.pc, "event", true);
    activate(&fake, &fake.state, world.pc, "event", true);
    assert_eq!(fake.count_lines("Controller.ResetIgnoreLookInput not found"), 1);
    let names: Vec<String> = fake.call_names().into_iter().take(10).collect();
    assert_eq!(
        names,
        [
            "EnableCharacterInput",
            "GetLoadout",
            "GetActiveLoadoutSlotIndex",
            "ResetIgnoreMoveInput",
            "ApplyInputConfiguration",
            "ApplyInputConfiguration",
            "ApplyInputConfigurationFromSettings",
            "SetupArchonInput",
            "SetupArchonInput",
            "EnableCharacterInput",
        ]
    );
    let setups = fake.calls_of("SetupArchonInput");
    assert_eq!((setups[0].object, setups[1].object), (world.pc, world.pawn));
}

/// A pawn whose player state names `id` and is active.
fn lifecycle_world(fake: &Fake, id: &str) -> World {
    let world = fake.world(Some(id), None);
    fake.fstring(world.player_state + 0x648, id);
    fake.put_u8(world.player_state + 0x6B0, 1);
    world
}

#[test]
fn an_empty_id_logs_once_per_pawn() {
    let fake = Fake::new();
    let world = fake.world(None, None);
    fake.put_u8(world.player_state + 0x6B0, 1);
    possessed(&fake, &world);
    replay(&fake, &fake.state, world.pc, "pump");
    replay(&fake, &fake.state, world.pc, "pump");
    assert_eq!(fake.count_lines("PlayerRoleId empty, waiting for replication"), 1);
}

#[test]
fn lifecycle_attempts_pause_and_resume() {
    let fake = Fake::new();
    let world = lifecycle_world(&fake, "PR_TEMPEST");
    possessed(&fake, &world);
    replay(&fake, &fake.state, world.pc, "pump");
    assert_eq!(*fake.applies.borrow(), [(world.loadout, false)]);
    assert!(fake.has_line("client: re-applied the role attempt=1 id=PR_TEMPEST roleAfter=null"));
    fake.advance(1499);
    replay(&fake, &fake.state, world.pc, "pump");
    assert_eq!(fake.applies.borrow().len(), 1, "1 500 ms between attempts");
    for _ in 1..25 {
        fake.advance(1500);
        replay(&fake, &fake.state, world.pc, "pump");
    }
    assert_eq!(fake.applies.borrow().len(), 20, "a batch is 20");
    fake.advance(30_000);
    replay(&fake, &fake.state, world.pc, "pump");
    assert!(fake.has_line("retrying the role after a 30 s pause"));
    assert_eq!(fake.applies.borrow().len(), 21);
}

#[test]
fn no_live_loadout_is_logged() {
    let fake = Fake::new();
    let world = lifecycle_world(&fake, "PR_TEMPEST");
    possessed(&fake, &world);
    fake.kill(world.loadout);
    replay(&fake, &fake.state, world.pc, "pump");
    assert!(fake.has_line("client: no live loadout to re-apply; will retry"));
    assert!(fake.applies.borrow().is_empty());
}

#[test]
fn the_replay_runs_once_per_pawn_and_role() {
    let fake = Fake::new();
    let world = lifecycle_world(&fake, "PR_TEMPEST");
    let role = fake.role("PR_TEMPEST");
    fake.put(world.pawn + 0x738, role);
    fake.put_u8(role + 0x2F4, 1);
    possessed(&fake, &world);
    replay(&fake, &fake.state, world.pc, "pump");
    let calls = fake.calls_of("HandleActiveGameplayStateChanged");
    assert_eq!(calls.len(), 1);
    assert_eq!((calls[0].object, calls[0].params.clone()), (world.pawn, vec![1, 1]));
    assert!(
        fake.has_line(
            "client: lifecycle replayed trigger=pump gameplayEquipped=0->0 roleActive=0->0 bpEquipCalled=0->0"
        )
    );
    replay(&fake, &fake.state, world.pc, "post-apply");
    assert_eq!(
        fake.calls_of("HandleActiveGameplayStateChanged").len(),
        1,
        "once per pair"
    );

    // Not while the player state is inactive, nor once the role is active
    let fake = Fake::new();
    let world = lifecycle_world(&fake, "PR_TEMPEST");
    let role = fake.role("PR_TEMPEST");
    fake.put(world.pawn + 0x738, role);
    possessed(&fake, &world);
    fake.put_u8(world.player_state + 0x6B0, 0);
    replay(&fake, &fake.state, world.pc, "pump");
    fake.put_u8(world.player_state + 0x6B0, 1);
    fake.set_active(role, true);
    replay(&fake, &fake.state, world.pc, "pump");
    assert!(fake.calls_of("HandleActiveGameplayStateChanged").is_empty());
}

#[test]
fn the_replay_flag_turns_off_the_pump_replay() {
    let fake = Fake::new();
    let world = lifecycle_world(&fake, "PR_TEMPEST");
    possessed(&fake, &world);
    fake.stable.set(true);
    fake.replay_off.set(true);
    fake.state.input.borrow_mut().activated = true;
    client_pump(&fake, &fake.state, 1);
    assert!(fake.applies.borrow().is_empty());
    activate(&fake, &fake.state, world.pc, "event", true);
    assert!(fake.has_line("role lifecycle deferred until the possession settles"));
}

/// A pawn that needs no movement repair.
fn healthy_pawn(fake: &Fake) -> (usize, usize, usize) {
    let pawn = fake.block(0x1700);
    let movement = fake.block(0x3C0);
    let root = fake.block(0x150);
    fake.put(pawn + 0x288, movement);
    fake.put(pawn + 0x130, root);
    fake.put_u8(pawn + 0x15E0, 1);
    fake.put_u8(movement + 0x168, 1);
    fake.put_u8(root + 0x14F, 2);
    fake.put_u8(pawn + 0xF0, 2);
    (pawn, movement, root)
}

#[test]
fn a_healthy_pawn_is_not_written() {
    let fake = Fake::new();
    let (pawn, component, root) = healthy_pawn(&fake);
    fake.read_only(pawn);
    fake.read_only(component);
    fake.read_only(root);
    movement(&fake, &fake.state, pawn);
    assert!(fake.take_lines().is_empty());
    fake.put_u8(pawn + 0xF0, 3);
    movement(&fake, &fake.state, pawn);
    assert!(fake.take_lines().is_empty(), "Authority is kept");
}

#[test]
fn each_movement_repair_alone() {
    let fake = Fake::new();
    let (pawn, _, _) = healthy_pawn(&fake);
    fake.put_u8(pawn + 0x15E0, 0);
    movement(&fake, &fake.state, pawn);
    assert_eq!((fake.byte(pawn + 0x15E0), fake.byte(pawn + 0x1604)), (1, 1));
    assert_eq!(
        fake.take_lines(),
        [
            "[MitInput] movement repair: CanMove_ 0 -> 1",
            "[MitInput] movement repair: PreviousCanMove 0 -> 1"
        ]
    );

    let fake = Fake::new();
    let (pawn, movement_component, _) = healthy_pawn(&fake);
    fake.put_u8(movement_component + 0x168, 0);
    movement(&fake, &fake.state, pawn);
    assert_eq!(fake.byte(movement_component + 0x168), 1);
    assert_eq!(fake.byte(movement_component + 0x3B8), 1);
    assert_eq!(fake.byte(pawn + 0x328), 1);
    assert!(fake.has_line("movement repair: MovementMode 0 -> 1"));

    let fake = Fake::new();
    let (pawn, _, root) = healthy_pawn(&fake);
    fake.put_u8(root + 0x14F, 0);
    movement(&fake, &fake.state, pawn);
    assert_eq!(fake.byte(root + 0x14F), 2);
    assert_eq!(fake.take_lines(), ["[MitInput] movement repair: Mobility 0 -> 2"]);

    let fake = Fake::new();
    let (pawn, _, _) = healthy_pawn(&fake);
    fake.put_u8(pawn + 0xF0, 1);
    movement(&fake, &fake.state, pawn);
    assert_eq!(fake.byte(pawn + 0xF0), 2);
    assert_eq!(fake.take_lines(), ["[MitInput] movement repair: Role 1 -> 2"]);
    fake.put_u8(pawn + 0xF0, 1);
    movement(&fake, &fake.state, pawn);
    assert!(fake.take_lines().is_empty(), "once per process per field");
}
