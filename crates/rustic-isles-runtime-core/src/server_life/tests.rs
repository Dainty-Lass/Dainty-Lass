use super::*;
use crate::registry::{LANDED, owns};

const S: u64 = 1_000_000;

#[test]
fn pacer_active_cap() {
    let mut pacer = Pacer::new();
    let first = pacer.step(S, 1000, 30, 0, true, 2);
    assert_eq!(first.sleep_us, 0, "the first frame does not wait");
    pacer.mark(S);
    assert_eq!(pacer.step(S, 1000, 30, 0, true, 2).sleep_us, 33_333);
    assert_eq!(pacer.step(S + 10_000, 1010, 30, 0, true, 2).sleep_us, 23_333);
    assert_eq!(
        pacer.step(S + 40_000, 1040, 30, 0, true, 2).sleep_us,
        0,
        "already past the target"
    );
    let mut none = Pacer::new();
    none.mark(S);
    assert_eq!(none.step(S, 1000, 0, 0, true, 0).sleep_us, 0, "active 0 never sleeps");
}

#[test]
fn pacer_idle_cap() {
    let mut pacer = Pacer::new();
    pacer.mark(S);
    // idle 0 never idles
    let mut same = Pacer::new();
    same.mark(S);
    assert!(same.step(S, 0, 30, 0, true, 0).edge.is_none());
    assert!(same.step(S, 20_000, 30, 0, true, 0).edge.is_none());

    let at = |pacer: &mut Pacer, ms: u64, count: i32| pacer.step(S, ms, 30, 5, true, count);
    assert_eq!(at(&mut pacer, 1000, 0).edge, None);
    let before = at(&mut pacer, 15_999, 0);
    assert_eq!((before.edge, before.sleep_us), (None, 33_333), "14 999 ms: not idle");
    let idle = at(&mut pacer, 16_000, 0);
    assert_eq!(
        idle.edge.as_deref(),
        Some("[MitRuntime] [FramePace] no client for 15 s: 5 fps")
    );
    assert_eq!(idle.sleep_us, 200_000);
    assert_eq!(at(&mut pacer, 17_000, 0).edge, None);
    let back = at(&mut pacer, 18_000, 1);
    assert_eq!(
        back.edge.as_deref(),
        Some("[MitRuntime] [FramePace] client connected: 30 fps")
    );
    assert_eq!(back.sleep_us, 33_333, "leaves idle on that frame");
    assert_eq!(at(&mut pacer, 19_000, 0).edge, None);
    assert_eq!(at(&mut pacer, 33_999, 0).edge, None, "a later 0 restarts the 15 s");
    assert!(at(&mut pacer, 34_000, 0).edge.is_some());

    let mut unknown = Pacer::new();
    assert!(unknown.step(S, 0, 30, 5, true, -1).edge.is_none());
    assert!(
        unknown.step(S, 60_000, 30, 5, true, -1).edge.is_none(),
        "count -1 never idle"
    );
    let mut quiet = Pacer::new();
    assert!(quiet.step(S, 0, 30, 5, false, 0).edge.is_none());
    assert!(
        quiet.step(S, 60_000, 30, 5, false, 0).edge.is_none(),
        "not listening never idle"
    );
    assert_eq!(
        idle_edge_line(false, 0, 5),
        "[MitRuntime] [FramePace] client connected: no cap"
    );
}

#[test]
fn hub_roles() {
    use AuthMode::{GlobalGameserver, PlayerHost};
    assert_eq!(
        hub_role("BPGM_City_C", GlobalGameserver, false, false),
        (HubRole::Hub, None)
    );
    assert_eq!(
        hub_role("BPGM_ArchonTrainingGrounds_C", GlobalGameserver, false, false),
        (HubRole::Hub, None)
    );
    assert_eq!(
        hub_role("BPGM_Hunt_C", GlobalGameserver, true, true),
        (HubRole::Disposable, None)
    );
    assert_eq!(
        hub_role("BPGM_city_C", GlobalGameserver, false, false).0,
        HubRole::Disposable
    );

    let (role, line) = hub_role("BPGM_City_C", PlayerHost, true, false);
    assert_eq!(role, HubRole::Hub);
    assert_eq!(
        line.as_deref(),
        Some("[MitRuntime] [HubRole] BPGM_City_C -> persistent hub (launcher role)")
    );
    let (role, line) = hub_role("BPGM_City_C", PlayerHost, false, false);
    assert_eq!(role, HubRole::Disposable);
    assert!(
        line.is_some_and(|line| line.ends_with("disposable (launcher role HUNT): reaped when empty, hunt AFK rule"))
    );
    let (role, line) = hub_role("BPGM_ArchonTrainingGrounds_C", PlayerHost, false, true);
    assert_eq!(role, HubRole::Hub);
    assert!(line.is_some_and(|line| line.contains("(KEEP_HOSTED_HUB_ROLE.flag, launcher role HUNT)")));
    let (role, line) = hub_role("BPGM_Hunt_C", PlayerHost, true, true);
    assert_eq!(role, HubRole::Disposable);
    assert!(line.is_some_and(|line| {
        line.starts_with("[MitRuntime] [HubRole] WARNING: BPGM_Hunt_C is disposable although the launcher role is HUB")
    }));
    assert_eq!(
        hub_role("BPGM_Hunt_C", PlayerHost, false, false),
        (HubRole::Disposable, None)
    );
    for role in [HubRole::Unknown, HubRole::Hub, HubRole::Disposable] {
        assert_eq!(HubRole::from_u8(role.to_u8()), role);
    }
}

#[test]
fn tick_reap() {
    let mut reap = TickReap::new();
    for _ in 0..59 {
        assert_eq!(reap.step(Some(true), false, 1.0, HubRole::Disposable), None);
    }
    assert_eq!(
        reap.step(Some(true), false, 1.0, HubRole::Disposable).as_deref(),
        Some("tick: disposable instance empty 60 s (game mode empty)")
    );
    let mut reset = TickReap::new();
    reset.step(Some(true), false, 59.0, HubRole::Disposable);
    reset.step(Some(false), false, 1.0, HubRole::Disposable);
    assert_eq!(reset.step(Some(true), false, 1.0, HubRole::Disposable), None);
    let mut unknown = TickReap::new();
    assert_eq!(
        unknown.step(None, true, 100.0, HubRole::Disposable),
        None,
        "a live connection"
    );
    assert_eq!(
        unknown.step(None, false, 60.0, HubRole::Disposable).as_deref(),
        Some("tick: disposable instance empty 60 s (game mode unknown)")
    );
    let mut hub = TickReap::new();
    assert_eq!(hub.step(Some(true), false, 600.0, HubRole::Hub), None);
}

#[test]
fn afk_plans() {
    assert_eq!(
        afk_plan(None, (0.0, 0.0), Some((0.0, 0.0))),
        AfkWrites {
            value: None,
            game_state: false,
            game_mode: false
        }
    );
    let plan = afk_plan(Some(1200), (1200.0, 0.0), Some((1200.0, 1200.0)));
    assert_eq!(
        (plan.value, plan.game_state, plan.game_mode),
        (Some(1200.0), true, false)
    );
    let plan = afk_plan(Some(1200), (1200.0, 1200.0), Some((300.0, 1200.0)));
    assert_eq!((plan.game_state, plan.game_mode), (false, true));
    let plan = afk_plan(Some(1200), (1.0, 1.0), None);
    assert_eq!((plan.game_state, plan.game_mode), (true, false), "game mode absent");
    let plan = afk_plan(Some(0), (900.0, 900.0), Some((0.0, 0.0)));
    assert_eq!((plan.value, plan.game_state, plan.game_mode), (Some(0.0), true, false));
}

#[test]
fn afk_lines() {
    let hub = AfkSetting {
        seconds: Some(120),
        source: "env",
    };
    let hunt = AfkSetting {
        seconds: None,
        source: "default",
    };
    let cap = HubCap {
        value: 4,
        source: "default",
    };
    assert_eq!(
        afk_config_line(hub, hunt, cap),
        "[MitRuntime] [AfkTimeout] config hub=120 s (env) hunt=native (default) maxPlayers=4 (default)"
    );
    assert!(afk_warnings(hub, hunt).is_empty());
    let zero = AfkSetting {
        seconds: Some(0),
        source: "env (0 = never kick)",
    };
    let warnings = afk_warnings(zero, zero);
    assert_eq!(warnings.len(), 2);
    assert!(warnings[1].contains("the hunt timeout is 0 (env (0 = never kick))"));
    let line = afk_apply_line(&AfkReport {
        hub: true,
        class: Some("BPGM_City_C"),
        gm_before: Some((1200.0, 1200.0)),
        gs_before: (0.0, 300.5),
        seconds: Some(120),
        why: "postlogin",
        game_state: 0xABC0,
    });
    assert_eq!(
        line,
        "[MitRuntime] [AfkTimeout] role=hub class=BPGM_City_C before GM=1200/1200 GS=0/300 -> applied 120 s \
         (why=postlogin) GS=0xabc0"
    );
    let line = afk_apply_line(&AfkReport {
        hub: false,
        class: None,
        gm_before: None,
        gs_before: (0.0, 0.0),
        seconds: None,
        why: "tick",
        game_state: 1,
    });
    assert!(line.contains("role=hunt class=? before GM=? GS=0/0 -> no override (native) (why=tick)"));
    let never = AfkReport {
        hub: true,
        class: None,
        gm_before: None,
        gs_before: (0.0, 0.0),
        seconds: Some(0),
        why: "tick",
        game_state: 1,
    };
    assert!(afk_apply_line(&never).contains("-> applied 0 s (never kick)"));
}

fn published(players: i32, travelling: i32, conn: i32, stamp: u64) -> Published {
    Published {
        conn_count: conn,
        conn_stamp: stamp,
        num_players: players,
        num_travelling: travelling,
        counts_stamp: stamp,
    }
}

#[test]
fn samples() {
    let now = 100_000;
    let zero = Sample::from(published(0, 0, 0, 0), now);
    assert!(!zero.counts_usable && !zero.conn_usable && !zero.empty && !zero.implausible);
    assert!(
        Sample::from(published(0, 0, 0, now - 10_000), now).empty,
        "10 000 ms old is usable"
    );
    let old = Sample::from(published(0, 0, 0, now - 10_001), now);
    assert!(!old.counts_usable && !old.empty);
    assert!(Sample::from(published(64, 0, 0, now), now).counts_usable);
    let high = Sample::from(published(65, 0, 0, now), now);
    assert!(!high.counts_usable && high.implausible && high.players == -1);
    let travelling = Sample::from(published(0, -1, 0, now), now);
    assert!(!travelling.counts_usable && travelling.implausible);
    let occupied = Sample::from(published(1, 0, 1, now), now);
    assert!(occupied.counts_usable && occupied.conn_usable && !occupied.empty);
    assert_eq!((occupied.players, occupied.connections), (1, 1));
    let mut conn_stale = published(0, 0, 0, now);
    conn_stale.conn_stamp = now - 20_000;
    let conn_stale = Sample::from(conn_stale, now);
    assert!(conn_stale.counts_usable && !conn_stale.conn_usable && !conn_stale.empty);
    assert_eq!(conn_stale.connections, -1);
    let mut no_conn = published(0, 0, 65, now);
    no_conn.conn_count = 65;
    assert!(!Sample::from(no_conn, now).conn_usable);
    // A player still counted in the game mode but no connection: empty (the ghost rule is the stale tick's)
    assert!(Sample::from(published(1, 0, 0, now), now).empty);
    assert!(!Sample::from(published(0, 1, 0, now), now).empty, "a traveller");
    assert_eq!(
        implausible_line(65, 0),
        "[MitRuntime] [Watchdog] ignoring implausible counts (players=65 travelling=0, at most 64): no reap"
    );
}

#[test]
fn heartbeat() {
    assert!(heartbeat_due(15, false, HubRole::Hub));
    assert!(!heartbeat_due(5, false, HubRole::Disposable));
    assert!(heartbeat_due(5, true, HubRole::Disposable));
    assert!(!heartbeat_due(5, true, HubRole::Hub));
    let sample = Sample::from(published(0, 0, 0, 1), 1);
    assert_eq!(
        heartbeat_line(HubRole::Disposable, &sample, true, 10, 60),
        "[MitRuntime] [Watchdog] role=disposable players=0 travelling=0 connections=0 hadClient=1 countsUsable=1 \
         empty=1 emptySec=10/60"
    );
}

#[test]
fn empty_reap() {
    let mut reap = EmptyReap::new();
    for _ in 0..29 {
        assert_eq!(reap.step(true, HubRole::Disposable, true), None);
    }
    assert_eq!(
        reap.step(true, HubRole::Disposable, true),
        Some("disposable server empty 60 s after its last client left")
    );
    let mut reset = EmptyReap::new();
    for _ in 0..29 {
        reset.step(true, HubRole::Disposable, true);
    }
    assert_eq!(reset.step(false, HubRole::Disposable, true), None);
    assert_eq!(reset.empty_seconds, 0);
    assert_eq!(reset.step(true, HubRole::Disposable, true), None);
    for role in [HubRole::Hub, HubRole::Unknown] {
        let mut never = EmptyReap::new();
        for _ in 0..1000 {
            assert_eq!(never.step(true, role, true), None);
        }
    }
    let mut late = EmptyReap::new();
    for _ in 0..149 {
        assert_eq!(late.step(true, HubRole::Disposable, false), None);
    }
    assert_eq!(
        late.step(true, HubRole::Disposable, false),
        Some("disposable server never joined by a client within 300 s")
    );
    assert_eq!((empty_limit(true), empty_limit(false)), (60, 300));
}

#[test]
fn stale_ticks() {
    let last = 1000;
    assert_eq!(
        stale_tick(HubRole::Disposable, false, last, last + 500_000, 15, 1),
        Stale::Quiet
    );
    assert_eq!(
        stale_tick(HubRole::Hub, true, last, last + 500_000, 15, 1),
        Stale::Quiet
    );
    assert_eq!(stale_tick(HubRole::Disposable, true, 0, 500_000, 15, 1), Stale::Quiet);
    assert_eq!(
        stale_tick(HubRole::Disposable, true, last, last + 179_999, 1, 1),
        Stale::Quiet
    );
    assert_eq!(
        stale_tick(HubRole::Disposable, true, last, last + 180_000, 1, 1),
        Stale::Reap(
            "[MitRuntime] [Watchdog] disposable server's game thread silent for 180 s (players=1): a hung game \
             thread or a ghost connection"
                .to_owned()
        )
    );
    assert_eq!(
        stale_tick(HubRole::Disposable, true, last, last + 59_999, 15, 1),
        Stale::Quiet
    );
    assert_eq!(
        stale_tick(HubRole::Disposable, true, last, last + 60_000, 15, 2),
        Stale::Notice("[MitRuntime] [Watchdog] game thread silent for 60 s (players=2); reap at 180 s".to_owned())
    );
    assert_eq!(
        stale_tick(HubRole::Disposable, true, last, last + 90_000, 16, 2),
        Stale::Quiet
    );
}

#[test]
fn port_timer() {
    let mut timer = PortTimer::new();
    assert_eq!(timer.step(true, true, 1000, 7777), None);
    assert_eq!(timer.step(true, true, 46_000, 7777), None, "45 000 ms: not yet");
    let line = timer.step(true, true, 46_001, 7777);
    assert!(line.is_some_and(|line| line.starts_with("[MitRuntime] [Watchdog] UDP port 7777 has been free for 45 s")));
    let mut reset = PortTimer::new();
    reset.step(true, true, 1000, 1);
    reset.step(true, false, 30_000, 1);
    assert_eq!(reset.step(true, true, 46_001, 1), None, "one not-free poll resets");
    let mut loading = PortTimer::new();
    for now in [0, 50_000, 100_000] {
        assert_eq!(loading.step(false, true, now, 1), None);
    }
    assert_eq!(probe_address(None), Ok([0, 0, 0, 0]));
    assert_eq!(probe_address(Some("127.0.0.1")), Ok([127, 0, 0, 1]));
    assert_eq!(
        probe_address(Some("::1")),
        Err("[MitRuntime] [Watchdog] port probe off: MULTIHOME ::1 is not an IPv4 address".to_owned())
    );
    assert!(probe_address(Some("host.example")).is_err());
}

fn trim_input(last_client_ms: u64, conn_stamp: u64) -> TrimInput {
    TrimInput {
        hub: true,
        counts_usable: true,
        conn_stamp,
        conn_count: 0,
        travelling: 0,
        last_client_ms,
        trim_allowed: true,
    }
}

#[test]
fn trim_timer() {
    let mut timer = TrimTimer::new();
    let mut now = 10_000;
    assert_eq!(timer.step(trim_input(5000, now), now), None);
    now += 59_999;
    assert_eq!(timer.step(trim_input(5000, now), now), None);
    now += 1;
    assert_eq!(timer.step(trim_input(5000, now), now), Some(60));
    now += 2000;
    assert_eq!(timer.step(trim_input(5000, now), now), None, "once per idle period");
    // A client arrives and leaves between polls: re-armed, and the 60 s restart
    let client = now + 1000;
    now += 2000;
    assert_eq!(timer.step(trim_input(client, now), now), None);
    now += 59_999;
    assert_eq!(timer.step(trim_input(client, now), now), None);
    now += 1;
    assert_eq!(timer.step(trim_input(client, now), now), Some(60));

    let mut restart = TrimTimer::new();
    restart.step(trim_input(0, 1000), 1000);
    restart.step(trim_input(30_000, 31_000), 31_000);
    assert_eq!(restart.step(trim_input(30_000, 61_000), 61_000), None);
    assert_eq!(restart.step(trim_input(30_000, 91_000), 91_000), Some(60));

    let mut stale = TrimTimer::new();
    stale.step(trim_input(0, 1000), 1000);
    assert_eq!(
        stale.step(trim_input(0, 61_000 - 1001), 61_000),
        None,
        "1001 ms old is not idle"
    );
    let mut off = TrimTimer::new();
    let mut input = trim_input(0, 1000);
    input.trim_allowed = false;
    off.step(input, 1000);
    input.conn_stamp = 100_000;
    assert_eq!(off.step(input, 100_000), None);
}

#[test]
fn watchdog_lines() {
    let [started, memory] = watchdog_start_lines(false, trim_state(true, false), true);
    assert!(started.starts_with("[MitRuntime] [Watchdog] started (poll 2 s;"));
    assert_eq!(
        memory,
        "[MitRuntime] [Memory] reserve: released at listen + 30 s; idle hub trim: on (hubs only, after 60 s with no \
         client, once per idle period); [Perf]/[Memory] lines: every 60 s"
    );
    let [_, memory] = watchdog_start_lines(true, trim_state(false, false), false);
    assert!(memory.contains("kept (KEEP_OOM_POOL.flag)"));
    assert!(memory.contains("off (not a Director server)"));
    assert!(memory.contains("off (DISABLE_PERF_LOG.flag)"));
    assert_eq!(trim_state(true, true), Err("off (KEEP_WORKING_SET.flag)"));
}

fn good_sites() -> ReserveSites {
    ReserveSites {
        setup_entry: Some(0x0252_5890),
        setup_bytes: Some(SETUP_POOLS_BYTES.to_vec()),
        oom_entry: Some(0x0252_9C00),
        oom_load: Some(OOM_LOAD_BYTES.to_vec()),
        oom_test: Some(OOM_TEST_BYTES.to_vec()),
    }
}

#[test]
fn reserve_sites() {
    use crate::offsets::r8::{ON_OUT_OF_MEMORY, SETUP_MEMORY_POOLS};
    let check = |sites: &ReserveSites| reserve_sites_ok(sites, SETUP_MEMORY_POOLS, ON_OUT_OF_MEMORY);
    assert_eq!(check(&good_sites()), Ok(()));
    let mut setup = good_sites();
    if let Some(bytes) = setup.setup_bytes.as_mut() {
        bytes[20] ^= 1;
    }
    assert_eq!(check(&setup), Err("SetupMemoryPools not as verified"));
    let mut entry = good_sites();
    entry.setup_entry = Some(0x0252_5880);
    assert_eq!(check(&entry), Err("SetupMemoryPools not as verified"));
    let mut load = good_sites();
    if let Some(bytes) = load.oom_load.as_mut() {
        bytes[3] ^= 1;
    }
    assert_eq!(check(&load), Err("OnOutOfMemory not as verified"));
    let mut test = good_sites();
    if let Some(bytes) = test.oom_test.as_mut() {
        bytes[0] ^= 1;
    }
    assert_eq!(check(&test), Err("OnOutOfMemory not as verified"));
    let mut missing = good_sites();
    missing.oom_entry = None;
    assert!(check(&missing).is_err());
}

#[test]
fn reserve_block() {
    let block = 0x1_0000_0000;
    let good = BlockRegion {
        base: block,
        allocation_base: block,
        size: RESERVE_SIZE,
        state: crate::memory::MEM_COMMIT,
        kind: MEM_PRIVATE,
        protect: PAGE_READWRITE,
    };
    assert!(reserve_block_ok(block, Some(good)));
    assert!(!reserve_block_ok(0, Some(good)));
    assert!(!reserve_block_ok(block, None));
    let variants = [
        BlockRegion {
            base: block + 0x1000,
            ..good
        },
        BlockRegion {
            allocation_base: block - 0x1000,
            ..good
        },
        BlockRegion {
            size: 0x100_0000,
            ..good
        },
        BlockRegion { state: 0x2000, ..good },
        BlockRegion { kind: 0x4_0000, ..good },
        BlockRegion { protect: 0x02, ..good },
    ];
    for region in variants {
        assert!(!reserve_block_ok(block, Some(region)), "{region:?}");
    }
}

#[test]
fn memory_lines() {
    let before = MemoryInfo {
        working_set: 300 * 1_048_576,
        peak_working_set: 400 * 1_048_576,
        private_commit: 512 * 1_048_576,
        page_faults: 1000,
    };
    let after = MemoryInfo {
        private_commit: 480 * 1_048_576,
        ..before
    };
    assert_eq!(
        reserve_released_line(Some(before), Some(after)),
        "[MitRuntime] [Memory] released the engine's 32 MB out-of-memory reserve: private commit 512.0 -> 480.0 MiB, \
         working set 300.0 -> 300.0 MiB"
    );
    assert!(reserve_released_line(None, None).contains("private commit ? -> ? MiB"));
    assert_eq!(
        trim_line(60, Some(before), None, 12, Some(5)),
        "[MitRuntime] [WorkingSet] trimmed (hub idle, no client for 60 s): working set 300.0 -> ? MiB, private commit \
         512.0 MiB, 12 ms (failed: error 5)"
    );
    assert_eq!(
        reserve_left_line(BLOCK_REASON),
        format!("[MitRuntime] [Memory] reserve left alone: {BLOCK_REASON}")
    );
    assert_eq!(
        reserve_kept_line(87),
        "[MitRuntime] [Memory] reserve kept: VirtualFree failed (error 87)"
    );
}

#[test]
fn perf_window() {
    let context = PerfContext {
        connections: 2,
        stale: false,
        role: HubRole::Hub,
        first: true,
    };
    let window = PerfWindow {
        frames: 1800,
        sum_us: 60_000_000,
        max_us: 150_500,
        hitches: 3,
    };
    assert_eq!(
        perf_line(window, &context),
        "[MitRuntime] [Perf] 60.0s: 1800 frames (30.0/s), frame avg 33.33 ms / max 150.5 ms, hitches>100ms 3, \
         connections 2, role hub (first window since listen)"
    );
    assert_eq!(
        perf_line(PerfWindow::default(), &context),
        "[MitRuntime] [Perf] no frames measured in this window (game thread not ticking?)"
    );
    let stale = PerfContext {
        stale: true,
        first: false,
        ..context
    };
    assert!(perf_line(window, &stale).ends_with("connections 2 (stale), role hub"));
    let info = MemoryInfo {
        working_set: 1_048_576,
        peak_working_set: 2 * 1_048_576,
        private_commit: 3 * 1_048_576,
        page_faults: 500,
    };
    assert_eq!(
        memory_line(Some(info), None, &context),
        "[MitRuntime] [Memory] connections 2, working set 1.0 MiB (peak 2.0), private commit 3.0 MiB, page faults \
         500 total (first window since listen)"
    );
    assert!(
        memory_line(Some(info), Some(450), &stale).ends_with(
            "(stale), working set 1.0 MiB (peak 2.0), private commit 3.0 MiB, page faults +50 since last line"
        )
    );
}

#[test]
fn exit_texts() {
    assert_eq!(
        reap_line("tick stale"),
        "[MitRuntime] [Reap] tick stale -> TerminateProcess(0)"
    );
    assert_eq!(
        quit_line("fence"),
        "[MitRuntime] [ServerQuit] quit requested (fence): ending through the reap exit"
    );
    assert_eq!(quit_reason("fence"), "server quit (fence)");
    assert_eq!(
        roster_line(true, 0),
        "[MitRuntime] [ExpectedPlayers] roster sync off (DISABLE_EXPECTED_PLAYER_ZERO.flag) launchRoster=0"
    );
    assert_eq!(
        expected_write_line("GameMode", 0x10, "BPGM_City_C", 1, 0),
        "[MitRuntime] [ExpectedPlayers] GameMode 0x10 class=BPGM_City_C 1 -> 0"
    );
}

#[test]
fn registry_r8() {
    assert_eq!(owns(b"SP-3@R8"), LANDED.contains(&"R8"));
    assert_eq!(owns(b"SP-2@R8"), LANDED.contains(&"R8"));
    // R8 lands with or after R9 and R10 (spec-runtime-r8 call 3)
    if LANDED.contains(&"R8") {
        assert!(LANDED.contains(&"R9") && LANDED.contains(&"R10"));
    }
}
