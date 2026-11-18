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

#[test]
fn package_rules() {
    assert!(
        !Rules::defaults().refuses("/Game/UI/Menus/main_menu_bpw"),
        "nothing by default"
    );
    let rules = Rules::parse(
        "/Game/UI/
/Game/Effects/
",
    );
    assert!(rules.refuses("/Game/UI/Menus/main_menu_bpw"));
    assert!(rules.refuses("/game/effects/fire"), "no case");
    assert!(!rules.refuses("/Game/Maps/ramsgate/ramsgate_01_persistent"));
    assert!(!rules.refuses("/Game/UIX/thing"));
    let list = Rules::parse(
        "# servers
/Game/UI/
!/Game/UI/Data/
  /Game/Audio/ 
not-a-path
! /Game/UI/Fonts/
",
    );
    assert_eq!(list.refuse, ["/game/ui/", "/game/audio/"]);
    assert_eq!(list.keep, ["/game/ui/data/", "/game/ui/fonts/"]);
    assert!(list.refuses("/Game/UI/Menus/x"));
    assert!(!list.refuses("/Game/UI/Data/item_icons"), "kept wins");
    assert!(list.refuses("/Game/Audio/amb"));
    assert!(!Rules::parse("").refuses("/Game/UI/x"));
    let named = Rules::parse(
        "/Game/UI/
!encounter_marker_bpc
",
    );
    assert_eq!(named.keep_names, ["encounter_marker_bpc"]);
    assert!(
        !named.refuses("/Game/UI/Markers/Encounter_Marker_BPC"),
        "kept by name, any folder"
    );
    assert!(named.refuses("/Game/UI/Markers/encounter_marker_bpc_old"));
}

#[test]
fn package_names() {
    assert_eq!(long_name("/Game/UI/X").as_deref(), Some("/Game/UI/X"));
    assert_eq!(
        long_name("../../../Archon/Content/UI/Menus/X.uasset").as_deref(),
        Some("/Game/UI/Menus/X")
    );
    assert_eq!(
        long_name("..\\..\\Archon\\Content\\UI\\X.umap").as_deref(),
        Some("/Game/UI/X")
    );
    assert_eq!(long_name("Engine/Something"), None);
    assert_eq!(
        packages_on_line(&Rules::defaults(), false),
        "[MitRuntime] [Stub] packages: refusing nothing (keeping none) from the defaults"
    );
    assert_eq!(
        refused_line("/Game/UI/X"),
        "[MitRuntime] [Stub] package refused: /Game/UI/X"
    );
    assert_eq!(
        refusals_line(12, 3),
        "[MitRuntime] [Stub] packages refused so far: 12 lookups of 3 packages"
    );
}
