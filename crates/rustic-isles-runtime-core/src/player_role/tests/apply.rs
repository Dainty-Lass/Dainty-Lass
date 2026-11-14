//! 5: applied, the validity note and the retry.

use super::*;

fn entries(fake: &Fake) -> Vec<RetryEntry> {
    fake.state.retry.borrow().iter().flatten().copied().collect()
}

#[test]
fn applied_needs_every_part_and_equal_ids() {
    let fake = Fake::new();
    let full = fake.world(Some("PR_TEMPEST"), Some("PR_TEMPEST"));
    assert!(snapshot(&fake, full.loadout).expect("readable").applied());
    let other = fake.world(Some("PR_TEMPEST"), Some("PR_ICEBORNE"));
    assert!(
        !snapshot(&fake, other.loadout).expect("readable").applied(),
        "different ids"
    );
    let empty = fake.world(Some(""), Some(""));
    assert!(
        !snapshot(&fake, empty.loadout).expect("readable").applied(),
        "an empty id"
    );
    let no_role = fake.world(Some("PR_TEMPEST"), None);
    assert!(
        !snapshot(&fake, no_role.loadout).expect("readable").applied(),
        "no pawn role"
    );
    let no_item = fake.world(None, Some("PR_TEMPEST"));
    let snap = snapshot(&fake, no_item.loadout).expect("readable");
    assert!(!snap.applied() && !snap.noted(), "no item, slot known: unequipped");
    let no_slot = fake.world(Some("PR_TEMPEST"), Some("PR_TEMPEST"));
    fake.put(no_slot.inter + 0x348, 0);
    let snap = snapshot(&fake, no_slot.loadout).expect("readable");
    assert!(!snap.applied() && snap.noted(), "no slot: not resolved yet");
    let no_pawn = fake.world(Some("PR_TEMPEST"), Some("PR_TEMPEST"));
    fake.put(no_pawn.pc + 0x250, 0);
    assert!(
        !snapshot(&fake, no_pawn.loadout).expect("readable").applied(),
        "no pawn"
    );
    assert!(snapshot(&fake, 0x8).is_none());
}

#[test]
fn note_and_the_unequipped_loadout() {
    let fake = Fake::new();
    let unequipped = fake.world(None, None);
    on_apply(&fake, &fake.state, unequipped.loadout, false);
    assert!(
        entries(&fake).is_empty(),
        "an explicitly unequipped loadout is never noted"
    );
    let pending = fake.world(Some("PR_TEMPEST"), None);
    on_apply(&fake, &fake.state, pending.loadout, false);
    assert_eq!(entries(&fake).len(), 1);
    assert!(fake.has_line("apply loadout=") && fake.has_line("applied=0"));
    note(&fake, &fake.state, pending.loadout, true);
    assert!(entries(&fake).is_empty(), "applied clears");
    note(&fake, &fake.state, pending.loadout, true);
    assert!(entries(&fake).is_empty(), "applied with no entry does nothing");
    for loadout in 1..=9 {
        note(&fake, &fake.state, loadout * 0x1000, false);
    }
    assert_eq!(entries(&fake).len(), 8, "the ninth is ignored");
    assert!(entries(&fake).iter().all(|entry| entry.loadout != 9 * 0x1000));
}

#[test]
fn retry_timing_and_attempts() {
    let fake = Fake::new();
    let world = fake.world(Some("PR_TEMPEST"), None);
    note(&fake, &fake.state, world.loadout, false);
    retry(&fake, &fake.state, false);
    assert!(fake.applies.borrow().is_empty(), "no retry in the tick it was noted");
    fake.advance(1499);
    retry(&fake, &fake.state, false);
    assert!(fake.applies.borrow().is_empty(), "1 499 ms waits");
    fake.advance(1);
    retry(&fake, &fake.state, false);
    assert_eq!(*fake.applies.borrow(), [(world.loadout, false)], "past the wrapper");
    assert!(fake.has_line("retry: attempt=1 loadout="));
    // The 20th attempt is the last; the window is wider than 20 intervals here only through the clock
    for _ in 1..25 {
        fake.advance(1200);
        retry(&fake, &fake.state, false);
    }
    assert!(fake.applies.borrow().len() <= 20, "the 21st attempt never happens");
}

#[test]
fn retry_attempt_limit() {
    let fake = Fake::new();
    let world = fake.world(Some("PR_TEMPEST"), None);
    note(&fake, &fake.state, world.loadout, false);
    // A clock that jumps only the interval: 20 attempts inside the window are impossible (30 000 / 1 500), so the
    // limit is checked on a table entry made by hand
    fake.state.retry.borrow_mut()[0] = Some(RetryEntry {
        loadout: world.loadout,
        first_seen_ms: fake.now.get(),
        last_retry_ms: 0,
        last_wait_log_ms: 0,
        retries: 20,
    });
    retry(&fake, &fake.state, false);
    assert!(fake.applies.borrow().is_empty());
    assert!(entries(&fake).is_empty(), "cleared silently");
    assert!(fake.take_lines().is_empty());
}

#[test]
fn retry_gives_up_after_the_window() {
    let fake = Fake::new();
    let world = fake.world(Some("PR_TEMPEST"), None);
    note(&fake, &fake.state, world.loadout, false);
    fake.advance(30_000);
    retry(&fake, &fake.state, false);
    assert!(fake.has_line("retry: gave up loadout="));
    assert!(fake.has_line("after 30000 ms retries=0"));
    assert!(entries(&fake).is_empty());
}

#[test]
fn a_dead_loadout_is_dropped_without_the_application() {
    let fake = Fake::new();
    let world = fake.world(Some("PR_TEMPEST"), None);
    note(&fake, &fake.state, world.loadout, false);
    fake.kill(world.loadout);
    fake.advance(1500);
    retry(&fake, &fake.state, false);
    assert!(fake.applies.borrow().is_empty(), "the application was not called");
    assert!(fake.has_line("is gone (world torn down), dropped"));
    assert!(entries(&fake).is_empty());
}

#[test]
fn the_wait_line_at_most_every_five_seconds() {
    let fake = Fake::new();
    let world = fake.world(Some("PR_TEMPEST"), None);
    fake.put(world.pc + 0x250, 0);
    note(&fake, &fake.state, world.loadout, false);
    for _ in 0..9 {
        fake.advance(1500);
        retry(&fake, &fake.state, false);
    }
    assert!(fake.applies.borrow().is_empty(), "no pawn: never applied");
    // 1 500, 7 500 and 13 500 ms after the note (the pumps are 1 500 ms apart)
    assert_eq!(fake.count_lines("retry: waiting loadout="), 3);
}

#[test]
fn an_applied_retry_joins_the_server_tables() {
    let fake = Fake::new();
    let world = fake.world(Some("PR_TEMPEST"), None);
    fake.apply_effect.set(Some(|fake, loadout, _| {
        let role = fake.role("PR_TEMPEST");
        let pawn = fake.ptr(fake.ptr(loadout + 0x630) + 0x250);
        fake.put(pawn + 0x738, role);
    }));
    note(&fake, &fake.state, world.loadout, false);
    fake.advance(1500);
    retry(&fake, &fake.state, true);
    assert!(fake.has_line("applied=1"));
    assert!(entries(&fake).is_empty());
    assert_eq!(fake.state.roles.borrow().len(), 1);
    assert_eq!(fake.state.tempest.borrow().len(), 1);
}

#[test]
fn the_apply_line_is_quiet_on_a_production_server() {
    let fake = Fake::new();
    fake.quiet.set(true);
    let world = fake.world(Some("PR_TEMPEST"), Some("PR_TEMPEST"));
    on_apply(&fake, &fake.state, world.loadout, true);
    assert!(fake.take_lines().iter().all(|line| !line.contains("apply loadout=")));
    fake.quiet.set(false);
    on_apply(&fake, &fake.state, world.loadout, false);
    assert!(fake.has_line("apply loadout="), "kept on a client");
}
