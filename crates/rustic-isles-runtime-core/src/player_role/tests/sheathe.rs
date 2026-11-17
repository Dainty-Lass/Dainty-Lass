//! r14-omnicell-fix: the game-thread gate and the sheathe watch.

use super::*;

/// A captured, stable possession whose pawn holds a working role: character active, extension set.
fn armed(fake: &Fake) -> (World, usize) {
    let world = fake.world(Some("PR_FRANK"), None);
    let role = fake.role("PR_FRANK");
    fake.put(world.pawn + 0x738, role);
    fake.put_u8(world.pawn + 0xA1A, 1);
    fake.put_bytes(world.pawn + 0xA1C, &[7; 16]);
    fake.put_u8(role + 0x2F4, 1);
    fake.captured.set(Some(Captured {
        pc: world.pc,
        pawn: world.pawn,
        since_ms: fake.now.get(),
        generation: 1,
    }));
    fake.chain_pawn.set(world.pawn);
    fake.stable.set(true);
    fake.loadout.set(world.loadout);
    fake.on("GetLoadout", |fake, _, params| {
        params[..8].copy_from_slice(&(fake.loadout.get() as u64).to_le_bytes());
    });
    client_restart(fake, &fake.state);
    (world, role)
}

/// `OnSheathedStateChanged(previous, new)` on `pawn`.
fn sheathed(fake: &Fake, pawn: usize, previous: u8, new: u8) {
    let params = fake.block(0x10);
    fake.put_bytes(params, &[previous, new]);
    sheathe_changed(fake, &fake.state, pawn, params);
}

/// An unsheathe, then the pump past the settle.
fn unsheathe(fake: &Fake, world: &World) {
    sheathed(fake, world.pawn, 2, UNSHEATHED);
    fake.advance(999);
    sheathe_check(fake, &fake.state, world.pc);
    fake.advance(1);
    sheathe_check(fake, &fake.state, world.pc);
}

fn repairs(fake: &Fake) -> usize {
    fake.calls_of("HandleActiveGameplayStateChanged").len()
}

#[test]
fn only_the_recorded_thread_is_admitted() {
    assert!(!game_thread_admits(0, 7), "nothing recorded: no thread");
    assert!(game_thread_admits(7, 7));
    assert!(!game_thread_admits(7, 8), "a worker thread");
}

#[test]
fn every_edge_is_logged_with_the_role_state() {
    let fake = Fake::new();
    let (world, role) = armed(&fake);
    sheathed(&fake, world.pawn, 3, 0);
    assert!(fake.has_line(&format!(
        "[MitPlayerRole] client: sheathe 3->0 role=0x{role:016x} live=1 characterActive=1 extension=1 roleBytes=1000"
    )));
    // Another pawn's edge is not the player's
    let other = fake.world(None, None);
    sheathed(&fake, other.pawn, 3, 0);
    assert_eq!(fake.count_lines("client: sheathe"), 1);
}

#[test]
fn the_first_unsheathe_sets_the_baseline_after_the_settle() {
    let fake = Fake::new();
    let (world, _) = armed(&fake);
    sheathed(&fake, world.pawn, 2, UNSHEATHED);
    fake.advance(999);
    sheathe_check(&fake, &fake.state, world.pc);
    assert!(!fake.has_line("role baseline"), "1 000 ms settle");
    fake.advance(1);
    sheathe_check(&fake, &fake.state, world.pc);
    assert!(fake.has_line("client: role baseline after unsheathe"));
    sheathe_check(&fake, &fake.state, world.pc);
    assert_eq!(fake.count_lines("role baseline"), 1, "one check per edge");
    assert!(fake.applies.borrow().is_empty() && repairs(&fake) == 0);
}

#[test]
fn a_sheathe_disarms_a_pending_check() {
    let fake = Fake::new();
    let (world, _) = armed(&fake);
    sheathed(&fake, world.pawn, 2, UNSHEATHED);
    sheathed(&fake, world.pawn, 3, 0);
    fake.advance(5000);
    sheathe_check(&fake, &fake.state, world.pc);
    assert!(!fake.has_line("role baseline"));
}

#[test]
fn an_intact_role_is_left_alone() {
    let fake = Fake::new();
    let (world, _) = armed(&fake);
    unsheathe(&fake, &world);
    // A new but live role is not a loss
    let other = fake.role("PR_FRANK");
    fake.put_u8(other + 0x2F4, 1);
    fake.put(world.pawn + 0x738, other);
    unsheathe(&fake, &world);
    assert!(fake.has_line("client: role intact after unsheathe"));
    assert!(fake.applies.borrow().is_empty() && repairs(&fake) == 0);
}

#[test]
fn each_loss_is_named_and_repaired() {
    for (lose, reason) in [
        (0, "lost=role "),
        (1, "lost=character-active "),
        (2, "lost=anim-extension "),
        (3, "lost=role-lifecycle "),
    ] {
        let fake = Fake::new();
        let (world, role) = armed(&fake);
        unsheathe(&fake, &world);
        match lose {
            0 => fake.kill(role),
            1 => fake.put_u8(world.pawn + 0xA1A, 0),
            2 => fake.put_bytes(world.pawn + 0xA1C, &[0; 16]),
            _ => fake.put_u8(role + 0x2F4, 0),
        }
        unsheathe(&fake, &world);
        assert!(fake.has_line(reason), "{reason}");
        assert!(fake.has_line("client: role repaired after unsheathe repair=1"));
        assert_eq!(
            *fake.applies.borrow(),
            [(world.loadout, true)],
            "PR-1 through R14's wrapper"
        );
        let calls = fake.calls_of("HandleActiveGameplayStateChanged");
        assert_eq!(
            (calls.len(), calls[0].object, calls[0].params.clone()),
            (1, world.pawn, vec![1, 1])
        );
    }
}

#[test]
fn repairs_stop_after_ten_per_pawn_and_a_new_possession_resets() {
    let fake = Fake::new();
    let (world, _) = armed(&fake);
    unsheathe(&fake, &world);
    fake.put_u8(world.pawn + 0xA1A, 0);
    for _ in 0..12 {
        unsheathe(&fake, &world);
    }
    assert_eq!(repairs(&fake), 10);
    assert_eq!(fake.count_lines("repairs exhausted for this pawn"), 2);
    let captured = fake.captured.get().map(|c| Captured { generation: 2, ..c });
    fake.captured.set(captured);
    client_restart(&fake, &fake.state);
    assert_eq!(*fake.state.sheathe.borrow(), Sheathe::default());
}

#[test]
fn the_pump_runs_the_check() {
    let fake = Fake::new();
    let (world, _) = armed(&fake);
    fake.state.input.borrow_mut().activated = true;
    sheathed(&fake, world.pawn, 2, UNSHEATHED);
    fake.advance(1000);
    client_pump(&fake, &fake.state, 1);
    assert!(fake.has_line("client: role baseline after unsheathe"));
}

#[test]
fn losses_against_the_baseline() {
    let base = RoleSign {
        role: 0x10,
        live: true,
        character_active: 1,
        extension: true,
        bytes: [1, 0, 0, 0],
    };
    assert!(lost(&base, &base).is_empty());
    let gone = RoleSign::default();
    assert_eq!(lost(&base, &gone), ["role", "character-active", "anim-extension"]);
    // What the baseline never had is not lost
    let weak = RoleSign {
        extension: false,
        character_active: 0,
        ..base
    };
    assert!(lost(&weak, &RoleSign { role: 0x20, ..weak }).is_empty());
}
