use super::*;
use crate::memory::{BufferMemory, Memory};
use std::panic::{AssertUnwindSafe, catch_unwind};

const BASE: usize = 0x10_0000;
const SIZE: usize = 0x4000;
const DRIVER: usize = BASE;
const LIST_DATA: usize = BASE + 0x400;
const CLEANUP_DATA: usize = BASE + 0x600;
const CONN_A: usize = BASE + 0x1000;
const CONN_B: usize = BASE + 0x1200;
const CONN_C: usize = BASE + 0x1400;
const BAD: usize = 0x7FFF_0000;

struct Fake(Vec<u8>);

impl Fake {
    /// A driver with `connections` and an empty, valid cleanup list.
    fn driver(connections: &[usize]) -> Self {
        let mut fake = Fake(vec![0; SIZE]);
        let num = i32::try_from(connections.len()).unwrap();
        fake.header(DRIVER + 0x90, LIST_DATA, num, num.max(4));
        for (index, &connection) in connections.iter().enumerate() {
            fake.word(LIST_DATA + index * 8, connection as u64);
        }
        fake.header(DRIVER + 0xF0, CLEANUP_DATA, 0, 4);
        fake
    }
    fn word(&mut self, at: usize, value: u64) {
        let at = at - BASE;
        self.0[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn half(&mut self, at: usize, value: i32) {
        let at = at - BASE;
        self.0[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn header(&mut self, at: usize, data: usize, num: i32, max: i32) {
        self.word(at, data as u64);
        self.half(at + 8, num);
        self.half(at + 12, max);
    }
    fn memory(&self) -> BufferMemory<'_> {
        BufferMemory {
            base: BASE,
            bytes: &self.0,
        }
    }
    fn apply(&mut self, plan: &Plan) {
        for store in &plan.stores {
            match *store {
                Store::Word(at, value) => self.word(at, value),
                Store::Num(at, value) => self.half(at, value),
            }
        }
    }
    fn plan(&self, driver: usize) -> Plan {
        plan_sanitize(&self.memory(), driver)
    }
}

#[test]
fn good_driver_is_safe_and_untouched() {
    let fake = Fake::driver(&[CONN_A, CONN_B]);
    let plan = fake.plan(DRIVER);
    assert!(plan.safe);
    assert!(plan.stores.is_empty());
    assert!(plan.texts.is_empty());
    assert!(Fake::driver(&[]).plan(DRIVER).safe, "an empty list is safe");
}

#[test]
fn unreadable_driver_and_header() {
    let fake = Fake::driver(&[CONN_A]);
    assert_eq!(fake.plan(BAD).texts, ["invalid driver"]);
    assert!(!fake.plan(BAD).safe);
    let holey = Holey {
        inner: fake.memory(),
        refused: DRIVER + 0x90,
    };
    let plan = plan_sanitize(&holey, DRIVER);
    assert!(!plan.safe);
    assert_eq!(plan.texts, ["unreadable connection list"]);
    assert!(plan.stores.is_empty());
    // the connection list fits, the cleanup header at +0xF0 does not
    let plan = fake.plan(BASE + SIZE - 0xA0);
    assert!(!plan.safe);
    assert_eq!(plan.texts, ["unreadable cleanup list"]);
    assert!(plan.stores.is_empty());
}

/// A memory whose `readable` refuses every range starting at `refused` (the driver's own check spans it).
struct Holey<'a> {
    inner: BufferMemory<'a>,
    refused: usize,
}

impl Memory for Holey<'_> {
    fn readable(&self, ptr: usize, size: usize) -> bool {
        ptr != self.refused && self.inner.readable(ptr, size)
    }
    fn read(&self, addr: usize, out: &mut [u8]) -> bool {
        self.inner.read(addr, out)
    }
}

#[test]
fn bad_connection_arrays_are_reset() {
    let cases = [
        (LIST_DATA, -1, 4),
        (LIST_DATA, 5, 4),
        (LIST_DATA, 1025, 2000),
        (BAD, 2, 4),
    ];
    for (data, num, max) in cases {
        let mut fake = Fake::driver(&[]);
        fake.header(DRIVER + 0x90, data, num, max);
        let plan = fake.plan(DRIVER);
        assert!(!plan.safe, "{num} {max}");
        assert_eq!(
            plan.stores,
            [Store::Word(DRIVER + 0x90, 0), Store::Word(DRIVER + 0x98, 0)]
        );
        assert_eq!(
            plan.texts,
            [format!("reset connection list data={} num={num} max={max}", addr(data))]
        );
        fake.apply(&plan);
        assert!(fake.plan(DRIVER).safe, "the reset list is safe next frame");
    }
}

#[test]
fn compaction_keeps_order_and_zeroes_the_tail() {
    let mut fake = Fake::driver(&[CONN_A, BAD, CONN_B, 0, CONN_C]);
    let plan = fake.plan(DRIVER);
    assert!(!plan.safe);
    assert_eq!(
        plan.texts,
        [
            format!("dropped connection index=1 ptr=0x{BAD:X}"),
            "dropped connection index=3 ptr=null".to_owned(),
            "compacted connection list 5 -> 3".to_owned(),
        ]
    );
    fake.apply(&plan);
    let memory = fake.memory();
    let kept: Vec<usize> = (0..5)
        .map(|slot| memory.ptr_at(LIST_DATA + slot * 8).unwrap())
        .collect();
    assert_eq!(kept, [CONN_A, CONN_B, CONN_C, 0, 0]);
    assert_eq!(memory.i32_at(DRIVER + 0x98), Some(3));
    assert_eq!(memory.i32_at(DRIVER + 0x9C), Some(5), "max untouched");
    assert!(fake.plan(DRIVER).safe);
}

#[test]
fn bad_cleanup_arrays_are_reset() {
    let cases = [
        (CLEANUP_DATA, -1, 4),
        (CLEANUP_DATA, 2, -1),
        (CLEANUP_DATA, 5, 4),
        (CLEANUP_DATA, 1025, 1025),
        (BAD, 1, 4),
        (BASE + SIZE - 0x18, 2, 4),
    ];
    for (data, num, max) in cases {
        let mut fake = Fake::driver(&[CONN_A]);
        fake.header(DRIVER + 0xF0, data, num, max);
        let plan = fake.plan(DRIVER);
        assert!(!plan.safe, "{data:x} {num} {max}");
        assert_eq!(
            plan.stores,
            [Store::Word(DRIVER + 0xF0, 0), Store::Word(DRIVER + 0xF8, 0)]
        );
        assert_eq!(
            plan.texts,
            [format!("reset cleanup list data={} num={num} max={max}", addr(data))]
        );
    }
    let mut fake = Fake::driver(&[CONN_A]);
    fake.header(DRIVER + 0xF0, BASE + SIZE - 0x30, 2, 4);
    assert!(fake.plan(DRIVER).safe, "2 * 0x18 readable");
}

#[test]
fn budget_suppresses_then_drops() {
    let budget = LineBudget::new(budgets::SANITIZE);
    let kind = sanitize_kind("before-flush");
    for n in 0..64 {
        assert_eq!(budget.take(&kind, || format!("line {n}")), Some(format!("line {n}")));
    }
    assert_eq!(
        budget.take(&kind, || "x".to_owned()),
        Some("[MitNet] sanitize before-flush: further lines suppressed".to_owned())
    );
    assert_eq!(budget.take(&kind, || "x".to_owned()), None);
    assert_eq!(budget.admit(), Admit::Drop);
}

#[test]
fn sites_have_their_own_budgets() {
    let sites = SiteBudgets::new();
    for _ in 0..64 {
        assert_eq!(sites.of("dispatch-inner").admit(), Admit::Line);
    }
    assert_eq!(sites.of("dispatch-inner").admit(), Admit::Suppressed);
    assert_eq!(sites.of("before-dispatch").admit(), Admit::Line);
    assert!(std::ptr::eq(sites.of("before-dispatch"), sites.of("before-dispatch")));
}

#[test]
fn level_collection_outcomes() {
    const WORLD: usize = 0xA000;
    const DRIVER: usize = 0xD000;
    let mut state = Registration::default();
    // deferred: the driver's world differs, once per world
    let types = Collections::Types(vec![0, 1, 2]);
    assert_eq!(
        state.check(WORLD, 0, &types),
        RegisterStep::Stop(Some(
            "[MitNet] level collections: registration deferred world=0xA000 driverWorld=null collections=3".into()
        ))
    );
    assert_eq!(state.check(WORLD, 0, &types), RegisterStep::Stop(None));
    // a new world defers again; unreadable and unusable collections
    assert_eq!(
        state.check(0xB000, 0xB000, &Collections::Unavailable),
        RegisterStep::Stop(Some(
            "[MitNet] level collections: registration deferred world=0xB000 driverWorld=0xB000 \
             collections=unavailable"
                .into()
        ))
    );
    assert_eq!(
        state.check(0xC000, 0xC000, &Collections::Unusable(9)),
        RegisterStep::Stop(Some(
            "[MitNet] level collections: registration deferred world=0xC000 driverWorld=0xC000 collections=9".into()
        ))
    );
    // refused: the context resolves another driver, once per world
    assert_eq!(state.check(WORLD, WORLD, &types), RegisterStep::Resolve);
    let (chosen, line) = state.resolved(WORLD, DRIVER, 0xE000, &[0, 1, 2]);
    assert!(chosen.is_empty());
    assert_eq!(
        line.as_deref(),
        Some(
            "[MitNet] level collections: registration refused, the world's context does not resolve this driver \
             world=0xA000 resolved=0xE000 driver=0xD000"
        )
    );
    assert_eq!(state.resolved(WORLD, DRIVER, 0, &[0, 1, 2]), (Vec::new(), None));
    // registered: only types 0 and 2
    let (chosen, line) = state.resolved(WORLD, DRIVER, DRIVER, &[0, 1, 2, 2]);
    assert_eq!(chosen, [0, 2, 3]);
    assert_eq!(
        line.as_deref(),
        Some("[MitNet] level collections: driver set on 3 gameplay collection(s) world=0xA000 driver=0xD000")
    );
    // already registered
    assert_eq!(state.check(WORLD, WORLD, &types), RegisterStep::Stop(None));
    assert!(gameplay_collection(0) && gameplay_collection(2));
    assert!(!gameplay_collection(1) && !gameplay_collection(3));
}

#[test]
fn spline_check() {
    let readable = |_: usize, _: usize| true;
    let unreadable = |_: usize, _: usize| false;
    let curve = |data, num| {
        Some(Curve {
            component: 0x5000,
            data,
            num,
        })
    };
    let mut cache = None;
    assert!(
        !spline_usable(&mut cache, None, readable),
        "null or unreadable component"
    );
    assert!(!spline_usable(&mut cache, curve(0, 4), readable), "data 0");
    assert!(!spline_usable(&mut cache, curve(0x8000, 0), readable));
    assert!(!spline_usable(&mut cache, curve(0x8000, -1), readable));
    assert!(!spline_usable(&mut cache, curve(0x8000, 10_001), readable));
    assert!(
        !spline_usable(&mut cache, curve(0x8000, 4), unreadable),
        "unreadable points"
    );
    assert_eq!(cache, None);
    let mut asked = 0;
    assert!(spline_usable(&mut cache, curve(0x8000, 10_000), |data, size| {
        asked += 1;
        data == 0x8000 && size == 10_000 * 0x2C
    }));
    assert_eq!(asked, 1);
    assert!(
        spline_usable(&mut cache, curve(0x8000, 10_000), unreadable),
        "remembered"
    );
    assert!(
        !spline_usable(&mut cache, curve(0x8000, 3), unreadable),
        "another triple is checked"
    );
    assert_eq!(cache, curve(0x8000, 10_000), "a refusal keeps the last acceptance");
    // a remembered triple never short-circuits with data 0
    let mut zero = Some(Curve {
        component: 0x5000,
        data: 0,
        num: 4,
    });
    assert!(!spline_usable(&mut zero, curve(0, 4), readable));
}

#[test]
fn reentry_guard() {
    let flag = std::cell::Cell::new(false);
    {
        let outer = Reentry::enter(&flag);
        assert!(outer.is_some());
        assert!(Reentry::enter(&flag).is_none(), "a nested drive returns");
    }
    assert!(!flag.get());
    let result = catch_unwind(AssertUnwindSafe(|| {
        let _guard = Reentry::enter(&flag);
        panic!("inside the drive");
    }));
    assert!(result.is_err());
    assert!(!flag.get(), "cleared after a panic");
    assert!(Reentry::enter(&flag).is_some());
}

#[test]
fn graph_class_and_params() {
    assert_eq!(graph_class(Some(1), Some(2)), Some((1, "ArchonReplicationGraph")));
    assert_eq!(graph_class(None, Some(2)), Some((2, "BasicReplicationGraph")));
    assert_eq!(graph_class(None, None), None);
    let params = construct_params(0x1234, 0x5678);
    assert_eq!(params.len() * 8, 0x50);
    assert_eq!(params[0], 0x1234);
    assert_eq!(params[1], 0x5678);
    assert!(params[2..].iter().all(|&word| word == 0));
    assert_eq!(rates_of(boosted_rates()), (60.0, 30.0));
}

#[test]
fn lines() {
    assert_eq!(addr(0), "null");
    assert_eq!(addr(0x7ff6_1234_abcd), "0x7FF61234ABCD");
    assert_eq!(
        sanitize_line("dispatch-inner", "invalid driver"),
        "[MitNet] sanitize dispatch-inner: invalid driver"
    );
    assert_eq!(dispatch_first(0x10), "[MitNet] TickDispatch first call driver=0x10");
    assert_eq!(
        DISPATCH_SKIPPED,
        "[MitNet] TickDispatch skipped this frame: unsafe connection state"
    );
    assert_eq!(
        manual_tick_mismatch(TickCall::Dispatch, 0x00A8_2CF0),
        "[MitNet] manual net tick: TickDispatch prologue mismatch at +0x00A82CF0, call off"
    );
    assert_eq!(
        manual_tick_mismatch(TickCall::Flush, 0x03FA_E700),
        "[MitNet] manual net tick: TickFlush prologue mismatch at +0x03FAE700, call off"
    );
    assert_eq!(manual_tick_on(0xAB), "[MitNet] manual net tick on driver=0xAB");
    assert_eq!(class_text(0, None), "null");
    assert_eq!(class_text(1, None), "no-class");
    assert_eq!(class_text(1, Some("ArchonReplicationGraph")), "ArchonReplicationGraph");
    assert_eq!(
        set_rep_driver(0, 0x10, 0, "null"),
        "[MitNet] SetReplicationDriver #0 driver=0x10 repDriver=null class=null"
    );
    assert_eq!(
        rates_boosted(10.0, 2.5),
        "[MitNet] player default object: NetUpdateFrequency 10 -> 60, MinNetUpdateFrequency 2.5 -> 30"
    );
    assert_eq!(
        always_relevant_set(0x02, 0x0A),
        "[MitNet] player default object: bAlwaysRelevant set (byte 0x02 -> 0x0A)"
    );
    assert_eq!(create_gate(0, 0), "[MitNet] CreateReplicationDriver #0 gate 0 -> 1");
    assert_eq!(
        create_class(1, 0, 0x20, "ArchonReplicationGraph"),
        "[MitNet] CreateReplicationDriver #1 class before=null after=0x20 (ArchonReplicationGraph)"
    );
    assert_eq!(
        create_constructed(0, 0x30, "ArchonReplicationGraph"),
        "[MitNet] CreateReplicationDriver #0 constructed 0x30 (ArchonReplicationGraph)"
    );
    assert_eq!(
        create_construct_null(2),
        "[MitNet] CreateReplicationDriver #2 construct returned null, calling the game's own"
    );
    assert_eq!(
        create_game_returned(3, 0, "null"),
        "[MitNet] CreateReplicationDriver #3 game returned null (null)"
    );
    assert_eq!(
        graph_class_line(Some((0x40, "BasicReplicationGraph"))),
        "[MitNet] replication graph class: BasicReplicationGraph 0x40"
    );
    assert_eq!(graph_class_line(None), "[MitNet] replication graph class: none null");
    assert_eq!(
        close_invalid(true, 3, 0),
        "[MitNet] Close server #3 invalid connection=null"
    );
    assert_eq!(
        close_enter(false, 0, 0x50, 3, 12.5, "a+0x1 b+0x2"),
        "[MitNet] Close client #0 enter conn=0x50 state=3 lastReceive=12.500000 stack=a+0x1 b+0x2"
    );
    assert_eq!(close_exit(true, 15, 0x50), "[MitNet] Close server #15 exit conn=0x50");
    assert_eq!(
        spline_unusable(0x60, 0, -1),
        "[MitNet] spline guard: unusable curve on component=0x60 data=null num=-1, returned a zero direction"
    );
    assert_eq!(spline_guard(true), "[MitNet] spline guard on");
    assert_eq!(
        spline_guard(false),
        "[MitNet] spline guard off (DISABLE_SPLINE_GUARD.flag)"
    );
    assert_eq!(
        FIND_NAMED_MISMATCH,
        "[MitNet] level collections: FindNamedNetDriver prologue mismatch, registration off"
    );
    assert_eq!(
        DEFAULT_OBJECT_MISSING,
        "[MitNet] player default object: BP_PlayerCharacter_C not loaded yet"
    );
    assert_eq!(
        suppressed(SPLINE_KIND),
        "[MitNet] spline guard: further lines suppressed"
    );
}

#[test]
fn flag_table() {
    assert_eq!(flags::DISABLE_SPLINE_GUARD, "DISABLE_SPLINE_GUARD");
    assert!(!flags::DROPPED.contains(&flags::DISABLE_SPLINE_GUARD));
    assert!(flags::DROPPED.contains(&"EMERGENCY_LEGACY_REPLICATION"));
}

// After R5's -1000; after R9's 700 upkeep and before its 900 listen (8.2)
const _: () = assert!(UPKEEP_ORDER > -1000 && MANUAL_TICK_ORDER > 700 && MANUAL_TICK_ORDER < 900);
