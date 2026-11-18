use super::*;
use crate::memory::BufferMemory;

const BASE: usize = 0x20_0000;

fn put(bytes: &mut [u8], address: usize, value: usize) {
    let at = address - BASE;
    bytes[at..at + 8].copy_from_slice(&(value as u64).to_le_bytes());
}

fn object(n: usize) -> usize {
    BASE + n * 0x40
}

#[test]
fn level_chains() {
    let mut bytes = vec![0u8; 0x1000];
    let (level_class, other_class) = (object(30), object(31));
    // 1 -> 2 (a level) ; 3 -> 4 -> null ; 5 <-> 6 cycle
    let chain = [
        (1, other_class, 2),
        (2, level_class, 0),
        (3, other_class, 4),
        (4, other_class, 0),
    ];
    for (n, class, outer) in chain {
        put(&mut bytes, object(n) + layout::UOBJECT_CLASS, class);
        put(
            &mut bytes,
            object(n) + layout::UOBJECT_OUTER,
            if outer == 0 { 0 } else { object(outer) },
        );
    }
    put(&mut bytes, object(5) + layout::UOBJECT_CLASS, other_class);
    put(&mut bytes, object(5) + layout::UOBJECT_OUTER, object(6));
    put(&mut bytes, object(6) + layout::UOBJECT_CLASS, other_class);
    put(&mut bytes, object(6) + layout::UOBJECT_OUTER, object(5));
    let memory = BufferMemory {
        base: BASE,
        bytes: &bytes,
    };
    let asked = std::cell::Cell::new(0);
    let mut is_level = |class: usize| {
        asked.set(asked.get() + 1);
        class == level_class
    };
    let mut memo = HashMap::new();
    assert!(in_level(&memory, object(1), &mut is_level, &mut memo));
    assert!(!in_level(&memory, object(3), &mut is_level, &mut memo));
    assert!(!in_level(&memory, object(5), &mut is_level, &mut memo), "a cycle ends");
    assert!(!in_level(&memory, 0, &mut is_level, &mut memo));
    let before = asked.get();
    assert!(in_level(&memory, object(1), &mut is_level, &mut memo));
    assert_eq!(asked.get(), before, "memoized");
}

#[test]
fn schedule() {
    assert!(!due(1000, None, 5999));
    assert!(due(1000, None, 6000));
    assert!(!due(1000, Some(6000), 15_999));
    assert!(due(1000, Some(6000), 16_000));
}

#[test]
fn lines() {
    let quiet = Sweep {
        components: 466,
        replicated: 2,
        ms: 40,
        ..Sweep::default()
    };
    assert_eq!(
        sweep_line(&quiet, true).as_deref(),
        Some(
            "[MitRuntime] [Stub] effects: 466 particle components in levels, 0 deactivated, 0 unticked, 2 left \
             (replicated), took 40 ms"
        )
    );
    assert_eq!(sweep_line(&quiet, false), None, "nothing changed");
    let busy = Sweep { stopped: 3, ..quiet };
    assert!(sweep_line(&busy, false).is_some());
}

#[test]
fn bool_layouts() {
    assert_eq!(bool_layout(0xD8, [1, 0, 0x04, 0x04]), Some((0xD8, 0x04)));
    assert_eq!(
        bool_layout(0xD8, [1, 0, 0xFF, 0xFF]),
        None,
        "a native bool is not a bitfield"
    );
    assert_eq!(bool_layout(0xD8, [1, 0, 0x04, 0x08]), None);
    assert_eq!(bool_layout(0xD8, [4, 0, 0x04, 0x04]), None);
    assert_eq!(bool_layout(0xD8, [1, 1, 0x04, 0x04]), None);
    assert_eq!(
        replicates_line(Some((0xD8, 0x04))),
        "[MitRuntime] [Stub] ActorComponent.bReplicates at +0xD8 mask 0x04"
    );
    assert!(replicates_line(None).ends_with("effects sweep off"));
    assert_eq!(
        missing_line("IsActive"),
        "[MitRuntime] [Stub] ActorComponent.IsActive not found: effects sweep off"
    );
}
