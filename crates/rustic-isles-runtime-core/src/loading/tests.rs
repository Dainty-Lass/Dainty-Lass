//! spec-runtime-r7 12.1, the pure part.

use super::lines::*;
use super::*;
use crate::offsets::{self, FoundBy};
use crate::registry::{self, ENTRIES, owned_count, owns};

#[test]
fn retry_budget() {
    let mut table = RetryTable::new();
    assert_eq!(table.claim(0x10, 0x20, 0), Some(1));
    assert_eq!(table.claim(0x10, 0x20, 5), None, "one retry per pair");
    assert_eq!(table.claim(0x11, 0x20, 5), Some(1), "another pair");
    assert_eq!(table.claim(0, 0x20, 5), None);
    assert_eq!(table.claim(0x10, 0, 5), None);

    // Fill the table: pairs 1..=32 at times 1000 + i
    let mut table = RetryTable::new();
    for i in 1..=32usize {
        assert_eq!(table.claim(i, i, 1000 + i as u64), Some(1));
    }
    // No empty slot, none stale: the oldest (pair 1) is reused, with a fresh budget
    assert_eq!(table.claim(100, 100, 2000), Some(1));
    assert_eq!(
        table.claim(1, 1, 2000),
        Some(1),
        "pair 1's slot was taken, so it starts again"
    );
    // The first slot 5 min stale (exactly 300 000 ms counts) goes before the older pair 5
    let mut table = RetryTable::new();
    for i in 1..=32usize {
        let at = match i {
            2 => 10_000,
            5 => 9_000,
            _ => 305_000,
        };
        table.claim(i, i, at);
    }
    assert_eq!(table.claim(200, 200, 310_000), Some(1));
    assert_eq!(
        table.claim(5, 5, 310_000),
        None,
        "pair 5 kept its slot and its used retry"
    );
    // An empty slot is taken while one exists
    table.clear(6, 0);
    assert_eq!(table.claim(300, 300, 310_001), Some(1));
    assert_eq!(table.claim(7, 7, 310_001), None, "slot 7 kept");
    assert_eq!(
        table.claim(2, 2, 310_002),
        Some(1),
        "pair 2's slot had gone to pair 200"
    );
}

#[test]
fn retry_clear() {
    let mut table = RetryTable::new();
    table.claim(1, 2, 0);
    table.claim(3, 4, 0);
    table.claim(5, 6, 0);
    assert_eq!(table.clear(1, 0), 1, "by manager");
    assert_eq!(table.claim(1, 2, 0), Some(1));
    assert_eq!(table.clear(0, 4), 1, "by controller");
    assert_eq!(table.claim(3, 4, 0), Some(1));
    assert_eq!(table.clear(0, 0), 0, "null matches nothing");
    assert_eq!(table.clear(5, 2), 2, "either");
}

fn entry(owner: usize, loader: usize, loaded: Option<u8>, loading: Option<u8>) -> LoaderEntry {
    LoaderEntry {
        owner,
        loader,
        loaded,
        loading,
    }
}

#[test]
fn rearm_decision() {
    let done = entry(1, 0x100, Some(1), Some(0));
    let stuck = entry(2, 0x200, Some(0), Some(1));
    let all_loaded = [done, done];
    assert_eq!(incomplete(&all_loaded), 0);
    assert!(recoverable(true, &all_loaded));
    assert_eq!(rearm_targets(&all_loaded, 0), None, "nothing to rearm");
    let one_stuck = [done, stuck];
    assert_eq!(incomplete(&one_stuck), 1);
    assert!(recoverable(true, &one_stuck));
    assert!(!recoverable(false, &one_stuck), "the controller's checks");
    assert_eq!(rearm_targets(&one_stuck, 1), Some(vec![1]));
    assert_eq!(rearm_targets(&one_stuck, 2), None, "the count must equal incomplete");
    assert!(!recoverable(true, &[done, entry(2, 0x200, Some(2), Some(0))]));
    assert!(!recoverable(true, &[done, entry(2, 0x200, Some(0), Some(2))]));
    assert!(!recoverable(true, &[done, entry(2, 0x200, None, Some(0))]));
    assert!(
        !recoverable(true, &[done, entry(0, 0x200, Some(0), Some(1))]),
        "null owner"
    );
    let no_loader = [done, entry(2, 0, Some(0), Some(1))];
    assert!(!recoverable(true, &no_loader), "null loader");
    assert_eq!(rearm_targets(&no_loader, 1), None);
}

#[test]
fn gate() {
    let mut gate = RestartGate::new();
    assert_eq!(gate.should_hold(0xA, 1_000), Hold::First);
    assert_eq!(gate.should_hold(0xA, 1_000 + 89_999), Hold::Held);
    assert_eq!(gate.should_hold(0xA, 1_000 + 90_000), Hold::GaveUp);
    assert_eq!(gate.should_hold(0xA, 1_000 + 90_001), Hold::Allow, "stays released");
    assert!(Hold::First.holds() && Hold::Held.holds() && !Hold::GaveUp.holds() && !Hold::Allow.holds());

    gate.mark_loaded(0xB);
    assert!(gate.loaded(0xB));
    assert_eq!(gate.should_hold(0xB, 0), Hold::Allow, "loaded before any ask");
    assert_eq!(gate.should_hold(0xC, 0), Hold::First);
    gate.mark_loaded(0xC);
    assert_eq!(gate.should_hold(0xC, 10), Hold::Allow, "loaded during a hold");
    gate.mark_loaded(0);
    assert_eq!(gate.should_hold(0, 0), Hold::Allow);
    assert!(!gate.loaded(0));

    // 32 controllers fill the ring; the 33rd overwrites the oldest slot
    let mut gate = RestartGate::new();
    for controller in 1..=32 {
        gate.mark_loaded(controller);
    }
    assert!(gate.loaded(1));
    assert_eq!(gate.should_hold(33, 0), Hold::First);
    assert!(!gate.loaded(1), "slot 1 overwritten");
    assert!(gate.loaded(2));
    assert_eq!(gate.should_hold(1, 0), Hold::First, "back with a fresh entry");
}

#[test]
fn hub_classes() {
    assert!(is_hub_class("BPGM_City_C"));
    assert!(is_hub_class("BPGM_ArchonTrainingGrounds_C"));
    assert!(!is_hub_class("BPGM_Hunt_C"));
    assert!(!is_hub_class(""));
}

#[test]
fn hunt_ids() {
    assert_eq!(shared_hunt_id(Some("a:h1,b:h1")), "h1");
    assert_eq!(shared_hunt_id(Some("a:h1")), "h1");
    assert_eq!(shared_hunt_id(Some("a:")), "");
    assert_eq!(shared_hunt_id(Some("a:h1, b:h2")), "h1");
    assert_eq!(shared_hunt_id(Some("a:b:c,d")), "b:c");
    assert_eq!(shared_hunt_id(Some("NO_EXPECTED_PLAYERS")), "");
    assert_eq!(shared_hunt_id(None), "");
    assert_eq!(shared_hunt_id(Some("")), "");
    assert_eq!(shared_hunt_id(Some(":h1,b:h2")), "h1");
    assert_eq!(shared_hunt_id(Some("a: h1 ,b")), " h1 ", "verbatim");
}

#[test]
fn progression_bookkeeping() {
    let mut progression = Progression::new();
    assert_eq!(progression.park(0x100, 10), 1);
    assert_eq!(progression.wait_since(), Some(10));
    assert_eq!(progression.park(0x100, 20), 1, "unique");
    assert_eq!(progression.park(0x200, 30), 2);
    assert_eq!(
        progression.wait_since(),
        Some(10),
        "set only when the waiters were empty"
    );
    assert_eq!(progression.candidates(), [0x100, 0x200]);
    progression.complete(0x100);
    assert!(progression.is_completed(0x100));
    assert_eq!(progression.waiter_count(), 1);
    assert_eq!(progression.candidates(), [0x200]);
    assert_eq!(progression.park(0x100, 40), 2, "a recycled address parks again");
    assert!(!progression.is_completed(0x100));
    let (waiters, since) = progression.take_waiters();
    assert_eq!((waiters, since), (vec![0x200, 0x100], Some(10)));
    assert_eq!(progression.wait_since(), None);
    progression.park(0x300, 50);
    assert_eq!(progression.wait_since(), Some(50));
    assert_eq!(progression.clear_waiters(), 1);
    assert_eq!(progression.waiter_count(), 0);
    progression.remove_candidate(0x300);
    assert!(!progression.candidates().contains(&0x300));
    progression.complete(0);
    assert!(!progression.is_completed(0));

    let mut progression = Progression::new();
    for component in 1..=MAX_CANDIDATES {
        progression.park(component, 0);
    }
    assert!(!progression.overflow());
    progression.park(MAX_CANDIDATES + 1, 0);
    assert!(progression.overflow());
    assert_eq!(progression.candidates().len(), MAX_CANDIDATES);
    progression.clear_waiters();
    progression.park(1, 0);
    assert!(progression.overflow(), "permanent");

    let mut progression = Progression::new();
    for component in 1..=MAX_COMPLETED {
        progression.complete(component);
    }
    assert!(progression.is_completed(1));
    progression.complete(1);
    assert!(progression.is_completed(MAX_COMPLETED), "a known one does not clear");
    progression.complete(MAX_COMPLETED + 1);
    assert!(!progression.is_completed(1), "emptied at the 4097th");
    assert!(progression.is_completed(MAX_COMPLETED + 1));
}

#[test]
fn resume_budget_and_poll() {
    let mut budget = ResumeBudget::default();
    for _ in 0..8 {
        assert!(budget.left());
        assert!(budget.take());
    }
    assert!(!budget.left());
    assert!(!budget.take());

    let mut poll = Poll::default();
    assert!(poll.due(1_000));
    assert!(!poll.due(1_199));
    assert!(poll.due(1_200));
    assert!(!poll.due(1_300));
    assert!(poll.due(5_000));

    assert!(!poll_gives_up(None, 1_000_000));
    assert!(!poll_gives_up(Some(1_000), 91_000));
    assert!(poll_gives_up(Some(1_000), 91_001));
}

#[test]
fn possession() {
    let mut possession = Possession::new();
    assert_eq!(possession.stable(0xC, Some(0xD), 0), None, "nothing captured");
    possession.capture(0xC, 0xD, 0x1, 1_000);
    assert_eq!(possession.since_began(1_500), Some(500));
    assert_eq!(possession.stable(0xC, Some(0xD), 1_000), None);
    assert_eq!(
        possession.stable(0xC, Some(0xD), 3_999),
        None,
        "2 999 ms after possession"
    );
    assert_eq!(
        possession.stable(0xC, Some(0xD), 4_000),
        Some(0xD),
        "3 000 ms and 750 ms of the pair"
    );
    assert_eq!(possession.stable(0xE, Some(0xD), 4_000), None, "another controller");
    assert_eq!(possession.stable(0xC, Some(0xF), 4_000), None, "not the captured pawn");
    assert_eq!(possession.stable(0xC, None, 4_000), None, "a failed read");
    assert_eq!(possession.stable(0xC, Some(0xD), 4_000), None, "tracking restarted");
    assert_eq!(possession.stable(0xC, Some(0xD), 4_749), None);
    assert_eq!(possession.stable(0xC, Some(0xD), 4_750), Some(0xD));

    // With no pawn captured, a changed pawn restarts the 750 ms
    let mut possession = Possession::new();
    possession.capture(0xC, 0, 0x1, 0);
    assert_eq!(possession.stable(0xC, Some(0xD), 5_000), None);
    assert_eq!(possession.stable(0xC, Some(0xD), 5_750), Some(0xD));
    assert_eq!(possession.stable(0xC, Some(0xE), 6_000), None);
    assert_eq!(possession.stable(0xC, Some(0xE), 6_749), None);
    assert_eq!(possession.stable(0xC, Some(0xE), 6_750), Some(0xE));

    // The same pair again is not a new possession; a new pawn is
    possession.capture(0xC, 0, 0x1, 9_000);
    assert_eq!(possession.since_began(9_000), Some(9_000));
    possession.capture(0xC, 0xE, 0x1, 9_000);
    assert_eq!(possession.since_began(9_000), Some(0));

    // A world change forgets the pair; the time stays
    possession.check_world(0x1);
    assert_eq!(possession.controller(), 0xC);
    possession.check_world(0x2);
    assert_eq!((possession.controller(), possession.pawn()), (0, 0));
    assert_eq!(possession.since_began(9_100), Some(100));
    assert_eq!(possession.stable(0xC, Some(0xE), 20_000), None);

    assert!(force_controller(true, 0x1, 20_000));
    assert!(!force_controller(true, 0x1, 19_999), "20 000 ms rule");
    assert!(!force_controller(true, 0, 60_000), "never without a weapon");
    assert!(!force_controller(false, 0x1, 60_000));
}

#[test]
fn idle_trigger() {
    assert!(TriggerSigns::default().idle());
    let signs = [
        TriggerSigns {
            slayer_actors: 1,
            ..TriggerSigns::default()
        },
        TriggerSigns {
            intro_refs: 1,
            ..TriggerSigns::default()
        },
        TriggerSigns {
            client_started: 1,
            ..TriggerSigns::default()
        },
        TriggerSigns {
            loading_state: 2,
            ..TriggerSigns::default()
        },
        TriggerSigns {
            primary_cine_actor: 0x10,
            ..TriggerSigns::default()
        },
        TriggerSigns {
            approach_actor: 0x10,
            ..TriggerSigns::default()
        },
    ];
    for sign in signs {
        assert!(!sign.idle(), "{sign:?}");
    }
}

#[test]
fn log_texts() {
    let p = 0x1234_5678_9abc_usize;
    assert_eq!(ptr(p), "0x0000123456789abc");
    assert_eq!(
        server_forced(Some("ArchonPlayerController Lvl.PC_0")),
        "[Readiness] server: forced ready obj=ArchonPlayerController Lvl.PC_0"
    );
    assert_eq!(server_forced(None), "[Readiness] server: forced ready obj=null");
    assert_eq!(
        client_forced(Some("PC X.Y"), 20_001),
        "[Readiness] client: forced ready local controller=PC X.Y sinceClientRestartMs=20001"
    );
    assert_eq!(
        trigger_release(true),
        "[Readiness] idle arrival trigger release on (default)"
    );
    assert_eq!(
        trigger_release(false),
        "[Readiness] idle arrival trigger release off (KEEP_ARRIVAL_TRIGGER_WAIT.flag)"
    );
    assert_eq!(
        trigger_released(3_012),
        "[Readiness] client: released the idle arrival trigger sinceClientRestartMs=3012 (its own timeout is 18 s)"
    );
    assert_eq!(
        completed_after_retry(true),
        "[PlayerData] load completed after a retry success=1"
    );
    assert_eq!(
        completed_after_retry(false),
        "[PlayerData] load completed after a retry success=0"
    );
    assert_eq!(hunt_id_set("h1"), "[PlayerData] hunt id was empty: set 'h1'");
    assert_eq!(
        not_recoverable(2),
        "[PlayerData] load failure not recoverable incomplete=2; the game handles it"
    );
    assert_eq!(retried(1, 3), "[PlayerData] retry 1/1 rearmed=3");
    assert_eq!(
        restart_held(p),
        "[PlayerData] restart held until the player's data loads: controller=0x0000123456789abc"
    );
    assert_eq!(
        restart_released(p),
        "[PlayerData] data still not loaded after 90 s; restart allowed: controller=0x0000123456789abc"
    );
    assert_eq!(
        parked(p, 2),
        "[Progression] parked component=0x0000123456789abc waiters=2"
    );
    assert_eq!(
        stranded(p),
        "[Progression] stranded component=0x0000123456789abc: calling CompleteLoadInternal"
    );
    assert_eq!(
        querying(p),
        "[Progression] querying the hunt pass season data instance=0x0000123456789abc"
    );
    assert_eq!(configuration(true), "[Progression] configuration loaded");
    assert_eq!(
        configuration(false),
        "[Progression] configuration not loaded yet: progression reads will park"
    );
    assert_eq!(
        resume_mode(true),
        "[Progression] resume of parked components on (no phantom local player)"
    );
    assert_eq!(
        resume_mode(false),
        "[Progression] resume of parked components off (this server has a phantom local player, whose own load covers it)"
    );
    assert_eq!(
        poll_gave_up(3),
        "[Progression] configuration still not loaded after 90 s; leaving 3 parked component(s) to the load-failure retry"
    );
    assert_eq!(
        already_completed(p),
        "[Progression] component=0x0000123456789abc already completed"
    );
    assert_eq!(gone(p), "[Progression] component=0x0000123456789abc is gone; skipped");
    assert_eq!(
        resumed(412, p),
        "[Progression] configuration loaded 412 ms after the first park: CompleteLoadInternal component=0x0000123456789abc"
    );
    assert_eq!(
        query_skipped(Some(24), 16),
        "[Progression] ArchonGameInstance.QueryCurrentHuntPassSeasonData: parameter size 24 exceeds 16, call skipped"
    );
    assert_eq!(
        crate::log::install_line("LD-4", Err(SKIPPED_LOGGING_ONLY)),
        "[MitRuntime] install LD-4 skipped: logging only (RT-3)"
    );
    for line in [
        HUNT_ID_NONE,
        HUNT_ID_FUNCTION_MISSING,
        DROPPED_NO_NET_ID,
        RETRY_DISABLED,
        RETRY_USED,
        REARM_FAILED,
        COMPLETE_RUNS,
        QUERY_RETURNED,
        QUERY_MISSING,
    ] {
        assert!(
            line.starts_with("[PlayerData] ") || line.starts_with("[Progression] "),
            "{line}"
        );
    }
}

const R7_IDS: [&str; 11] = [
    "LD-1", "LD-2", "LD-3", "LD-4", "LD-5", "LD-6", "LD-7", "LD-8", "LD-9", "SP-2@R7", "SP-3@R7",
];

#[test]
fn registry_and_targets() {
    assert!(registry::LANDED.contains(&"R7"));
    for id in R7_IDS {
        assert!(owns(id.as_bytes()), "{id}");
    }
    assert_eq!(owns(b"SP-2@R20"), registry::LANDED.contains(&"R20"));
    let r7 = ENTRIES.iter().filter(|entry| entry.slice == "R7").count();
    assert_eq!(r7, R7_IDS.len(), "R7 adds 11 owned ids");
    assert!(owned_count() >= 5 + R7_IDS.len());
    for id in &R7_IDS[..9] {
        let target = offsets::target(id).unwrap_or_else(|| panic!("{id} has no target line"));
        assert_eq!(target.found_by, FoundBy::Rva);
        assert_ne!(target.rva, 0);
    }
    assert_eq!(
        offsets::target("LD-6").map(|t| t.rva),
        Some(offsets::r7::COMPLETE_LOAD_INTERNAL)
    );
    assert_eq!(
        offsets::target("LD-4").map(|t| t.rva),
        Some(offsets::r7::LOAD_MANAGER_BEGIN)
    );
}
