use super::*;
use crate::memory::BufferMemory;
use crate::offsets::target;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

const BASE: usize = 0x10_0000;
const WIDGET: usize = BASE;
const BOUND: usize = BASE + 0x800;

/// A `0x400`-byte callout widget with every member bound, plus a bound pointee at `BOUND`.
fn widget() -> Vec<u8> {
    let mut bytes = vec![0u8; 0x1000];
    for member in MEMBERS {
        put_ptr(&mut bytes, WIDGET + member.offset, BOUND);
    }
    bytes
}

fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at - BASE..at - BASE + value.len()].copy_from_slice(value);
}

fn put_ptr(bytes: &mut [u8], at: usize, value: usize) {
    put(bytes, at, &(value as u64).to_le_bytes());
}

fn with_null(mask: u16) -> Vec<u8> {
    let mut bytes = widget();
    for (index, member) in MEMBERS.iter().enumerate() {
        if mask & (1 << index) != 0 {
            put_ptr(&mut bytes, WIDGET + member.offset, 0);
        }
    }
    bytes
}

fn mem(bytes: &[u8]) -> BufferMemory<'_> {
    BufferMemory { base: BASE, bytes }
}

fn next() -> Decision {
    decide(Verdict::Next, Nulls::Members(0))
}

#[test]
fn super_call_targets() {
    let bytes = |id: &str| match target(id).map(|line| line.found_by) {
        Some(FoundBy::RvaAndBytes(bytes)) => bytes,
        _ => panic!("{id}"),
    };
    assert_eq!(super_call_target(bytes("UI-5"), 0x01E4_BA00, 0x09), Some(0x01E4_A480));
    assert_eq!(super_call_target(bytes("UI-7"), 0x01E5_0A70, 0x0D), Some(0x01E5_0390));
    // 3.1's UI-6 bytes stop before +0x1A: the call is laid out from its parent's RVA
    let mut ui6 = bytes("UI-6").to_vec();
    ui6.resize(0x1A, 0x90);
    ui6.push(0xE8);
    ui6.extend_from_slice(&(0x01E4_DEE0i64 - (0x01E4_E050 + 0x1A + 5)).to_le_bytes()[..4]);
    assert_eq!(super_call_target(&ui6, 0x01E4_E050, 0x1A), Some(0x01E4_DEE0));
    assert_eq!(super_call_target(bytes("UI-5"), 0x01E4_BA00, 0x08), None, "not E8");
    assert_eq!(
        super_call_target(&bytes("UI-5")[..12], 0x01E4_BA00, 0x09),
        None,
        "short"
    );
}

#[test]
fn hide_hold_text_decisions() {
    let bound = widget();
    assert_eq!(hide_hold_text(&mem(&bound), WIDGET), next());
    let skip = |nulls| decide(Verdict::Skip, nulls);
    assert_eq!(hide_hold_text(&mem(&bound), 0), skip(Nulls::This));
    assert_eq!(
        hide_hold_text(&mem(&bound), BASE + 0xD00),
        skip(Nulls::This),
        "unreadable for 0x3c8"
    );
    let null = with_null(m::HOLD_TEXT);
    assert_eq!(hide_hold_text(&mem(&null), WIDGET), skip(Nulls::Members(m::HOLD_TEXT)));
    let mut wild = widget();
    put_ptr(&mut wild, WIDGET + r6::CALLOUT_HOLD_TEXT, 0xDEAD_0000);
    assert_eq!(
        hide_hold_text(&mem(&wild), WIDGET),
        skip(Nulls::Members(m::HOLD_TEXT)),
        "unreadable pointee"
    );
}

#[test]
fn screen_fade_in_decisions() {
    let screen = || {
        let mut bytes = widget();
        put_ptr(&mut bytes, WIDGET + r6::SCREEN_PRIMARY, BOUND);
        bytes
    };
    let bound = screen();
    let fade = screen_fade_in(&mem(&bound), WIDGET);
    assert!(fade.run);
    assert_eq!((fade.primary, fade.secondary), (BOUND, BOUND));
    for offset in [r6::SCREEN_PRIMARY, r6::SCREEN_SECONDARY] {
        let mut one = screen();
        put_ptr(&mut one, WIDGET + offset, 0);
        assert!(!screen_fade_in(&mem(&one), WIDGET).run, "{offset:#x}");
    }
    assert!(!screen_fade_in(&mem(&bound), 0).run);
    assert!(!screen_fade_in(&mem(&bound), BASE + 0xD00).run);
}

#[test]
fn on_set_disabled_decisions() {
    assert_eq!(on_set_disabled(&mem(&widget()), WIDGET), next());
    assert_eq!(
        on_set_disabled(&mem(&widget()), 0),
        decide(Verdict::SkipRelease, Nulls::This)
    );
    for one in [m::HINT, m::REASON, m::ICON] {
        assert_eq!(
            on_set_disabled(&mem(&with_null(one)), WIDGET),
            decide(Verdict::SkipRelease, Nulls::Members(one))
        );
    }
    assert_eq!(
        on_set_disabled(&mem(&widget()), BASE + 0xE00).verdict,
        Verdict::SkipRelease,
        "unreadable"
    );
}

#[test]
fn callout_member_decisions() {
    type Guard = fn(&dyn Memory, usize) -> Decision;
    let cases: [(Guard, u16, Verdict, Verdict); 5] = [
        (native_construct, m::RADIAL, Verdict::ParentOnly, Verdict::Silent),
        (
            native_on_input_swap,
            m::KEYBOARD | m::GAMEPAD,
            Verdict::ParentOnly,
            Verdict::Silent,
        ),
        (
            native_tick,
            m::RADIAL | m::PROGRESS,
            Verdict::ParentOnly,
            Verdict::Silent,
        ),
        (fill_progress, m::RADIAL | m::FILL, Verdict::Skip, Verdict::Skip),
        (show_disabled_icon, m::ICON, Verdict::Skip, Verdict::Skip),
    ];
    for (guard, wanted, unbound, null_this) in cases {
        assert_eq!(guard(&mem(&widget()), WIDGET), next());
        assert_eq!(guard(&mem(&widget()), 0), decide(null_this, Nulls::This));
        for index in 0..MEMBERS.len() {
            let bit = 1u16 << index;
            let decision = guard(&mem(&with_null(bit)), WIDGET);
            if wanted & bit == 0 {
                assert_eq!(decision, next(), "{wanted:#x} ignores {bit:#x}");
            } else {
                assert_eq!(decision, decide(unbound, Nulls::Members(bit)));
            }
        }
        assert_eq!(
            guard(&mem(&with_null(wanted)), WIDGET),
            decide(unbound, Nulls::Members(wanted))
        );
        assert_eq!(guard(&mem(&widget()), BASE + 0xF00).verdict, unbound, "unreadable");
    }
}

#[test]
fn show_disabled_text_decisions() {
    let release = |nulls| decide(Verdict::SkipRelease, Nulls::Members(nulls));
    for mark in [false, true] {
        assert_eq!(show_disabled_text(&mem(&widget()), WIDGET, mark), next());
        assert_eq!(
            show_disabled_text(&mem(&widget()), 0, mark),
            decide(Verdict::SkipRelease, Nulls::This)
        );
        assert_eq!(
            show_disabled_text(&mem(&with_null(m::REASON)), WIDGET, mark),
            release(m::REASON)
        );
    }
    for one in [m::HINT, m::ICON] {
        assert_eq!(show_disabled_text(&mem(&with_null(one)), WIDGET, false), next());
        assert_eq!(show_disabled_text(&mem(&with_null(one)), WIDGET, true), release(one));
    }
}

const PC: usize = BASE + 0x900;

fn ui4(bytes: &[u8], weak: Option<usize>, owning: Option<usize>, is_pc: bool) -> Decision {
    let owning_called = Cell::new(false);
    let decision = refresh_input_keys(
        &mem(bytes),
        WIDGET,
        |index, serial| {
            assert_eq!((index, serial), (5, 9));
            weak
        },
        || {
            owning_called.set(true);
            owning
        },
        |_| is_pc,
    );
    if weak.is_some() {
        assert!(!owning_called.get(), "the weak pointer is tried first");
    }
    decision
}

#[test]
fn refresh_input_keys_controller_order() {
    let mut bytes = widget();
    put(&mut bytes, WIDGET + r6::CALLOUT_OWNER_WEAK, &5i32.to_le_bytes());
    put(&mut bytes, WIDGET + r6::CALLOUT_OWNER_WEAK + 4, &9i32.to_le_bytes());
    let with_input = bytes.clone();
    put_ptr(&mut bytes, PC + r6::PC_PLAYER_INPUT, 0);
    let no_input = bytes;
    let mut with_input = with_input;
    put_ptr(&mut with_input, PC + r6::PC_PLAYER_INPUT, BOUND);
    let skip_input = decide(Verdict::Skip, Nulls::PlayerInput);

    assert_eq!(ui4(&with_input, Some(PC), None, true), next());
    assert_eq!(ui4(&no_input, Some(PC), None, true), skip_input);
    // Weak pointer dead: GetOwningPlayer, and its class matters
    assert_eq!(ui4(&no_input, None, Some(PC), false), next(), "not a player controller");
    assert_eq!(ui4(&no_input, None, Some(PC), true), skip_input);
    assert_eq!(ui4(&with_input, None, Some(PC), true), next());
    assert_eq!(ui4(&no_input, None, None, true), next(), "no controller");

    for keys in [m::KEYBOARD, m::GAMEPAD, m::KEYBOARD | m::GAMEPAD] {
        let null = with_null(keys);
        assert_eq!(
            refresh_input_keys(&mem(&null), WIDGET, |_, _| Some(PC), || None, |_| true),
            decide(Verdict::Skip, Nulls::Members(keys))
        );
    }
    assert_eq!(
        refresh_input_keys(&mem(&with_input), 0, |_, _| None, || None, |_| true),
        decide(Verdict::Skip, Nulls::This)
    );
}

const INVENTORY: usize = BASE + 0x100;
const OWNER: usize = BASE + 0x1000;
const LOADOUT: usize = BASE + 0x2000;
const SLOTS: usize = BASE + 0x2800;

/// inventory -> controller -> loadout with two slots, the active one (1) holding data.
fn graph() -> Vec<u8> {
    let mut bytes = vec![0u8; 0x3000];
    put_ptr(&mut bytes, INVENTORY + r6::INVENTORY_OWNER, OWNER);
    put_ptr(&mut bytes, OWNER + r6::PC_LOADOUT, LOADOUT);
    put_ptr(&mut bytes, LOADOUT + r6::LOADOUT_SLOTS, SLOTS);
    put(&mut bytes, LOADOUT + r6::LOADOUT_SLOTS + 8, &2i32.to_le_bytes());
    put(&mut bytes, LOADOUT + r6::LOADOUT_ACTIVE, &1i32.to_le_bytes());
    put_ptr(&mut bytes, SLOTS + 8, BOUND);
    bytes
}

fn ui12(bytes: &[u8], is_pc: bool, null_getter: Option<usize>) -> (Refresh, Vec<usize>) {
    let called = RefCell::new(Vec::new());
    let refresh = equipped_refresh(
        &mem(bytes),
        INVENTORY,
        |_| is_pc,
        |index, loadout| {
            assert_eq!(loadout, LOADOUT);
            called.borrow_mut().push(index);
            if Some(index) == null_getter { 0 } else { BOUND }
        },
    );
    (refresh, called.into_inner())
}

#[test]
fn equipped_refresh_decisions() {
    let (all, called) = ui12(&graph(), true, None);
    assert_eq!(all, Refresh { skip: None, slot: 1 });
    assert_eq!(called, [0, 1, 2, 3, 4]);
    for index in 0..5 {
        let (refresh, called) = ui12(&graph(), true, Some(index));
        assert_eq!(refresh.skip, Some(refresh::NULL_GETTER), "getter {index}");
        assert_eq!(called.last(), Some(&index), "stops at the first null");
    }
    for (slot, why) in [(-1i32, "slot -1"), (2, "slot = num")] {
        let mut bytes = graph();
        put(&mut bytes, LOADOUT + r6::LOADOUT_ACTIVE, &slot.to_le_bytes());
        let (refresh, called) = ui12(&bytes, true, None);
        assert_eq!(
            refresh,
            Refresh {
                skip: Some(refresh::NO_SLOT),
                slot
            },
            "{why}"
        );
        assert!(called.is_empty(), "getters only run with a valid slot");
    }
    let mut empty = graph();
    put_ptr(&mut empty, SLOTS + 8, 0);
    assert_eq!(
        ui12(&empty, true, None).0.skip,
        Some(refresh::NO_SLOT),
        "null slot data"
    );

    let passes = Refresh { skip: None, slot: -2 };
    assert_eq!(ui12(&graph(), false, None).0, passes, "not a player controller");
    let mut ownerless = graph();
    put_ptr(&mut ownerless, INVENTORY + r6::INVENTORY_OWNER, 0);
    assert_eq!(ui12(&ownerless, true, None).0, passes, "no owner");
    let mut no_loadout = graph();
    put_ptr(&mut no_loadout, OWNER + r6::PC_LOADOUT, 0);
    assert_eq!(ui12(&no_loadout, true, None).0, passes, "no loadout");
    assert_eq!(
        equipped_refresh(&mem(&graph()), 0, |_| true, |_, _| 0),
        passes,
        "no inventory"
    );
}

const TEXT: usize = BASE + 0x100;
const CONTROLLER: usize = BASE + 0x200;
const VTABLE: usize = BASE + 0x300;
const IMAGE: (usize, usize) = (0x7000_0000, 0x1000);

#[derive(Default)]
struct FakeRefs {
    counts: RefCell<HashMap<usize, i32>>,
    calls: RefCell<Vec<(&'static str, usize, usize)>>,
}

impl TextRefs for FakeRefs {
    fn decrement(&self, address: usize) -> Option<i32> {
        let mut counts = self.counts.borrow_mut();
        let count = counts.get_mut(&address)?;
        *count -= 1;
        Some(*count)
    }
    fn destroy(&self, entry: usize, controller: usize) {
        self.calls.borrow_mut().push(("destroy", entry, controller));
    }
    fn delete(&self, entry: usize, controller: usize) {
        self.calls.borrow_mut().push(("delete", entry, controller));
    }
}

fn release(shared: i32, weak: i32, entries: (usize, usize)) -> (Release, FakeRefs) {
    let mut bytes = vec![0u8; 0x400];
    put_ptr(&mut bytes, TEXT + r6::FTEXT_CONTROLLER, CONTROLLER);
    put_ptr(&mut bytes, CONTROLLER, VTABLE);
    put_ptr(&mut bytes, VTABLE, entries.0);
    put_ptr(&mut bytes, VTABLE + 8, entries.1);
    let refs = FakeRefs::default();
    refs.counts
        .borrow_mut()
        .extend([(CONTROLLER + r6::REF_SHARED, shared), (CONTROLLER + r6::REF_WEAK, weak)]);
    (release_owned_text(&mem(&bytes), &refs, TEXT, IMAGE), refs)
}

#[test]
fn release_owned_text_decisions() {
    let inside = (IMAGE.0 + 0x10, IMAGE.0 + 0x20);
    let (result, refs) = release(2, 1, inside);
    assert_eq!(result, Release::Decremented);
    assert!(refs.calls.borrow().is_empty());
    assert_eq!(refs.counts.borrow()[&(CONTROLLER + r6::REF_SHARED)], 1);

    let (result, refs) = release(1, 2, inside);
    assert_eq!(result, Release::Destroyed);
    assert_eq!(*refs.calls.borrow(), [("destroy", inside.0, CONTROLLER)]);

    let (result, refs) = release(1, 1, inside);
    assert_eq!(result, Release::Deleted);
    assert_eq!(
        *refs.calls.borrow(),
        [("destroy", inside.0, CONTROLLER), ("delete", inside.1, CONTROLLER)]
    );

    let outside = (IMAGE.0 + IMAGE.1, IMAGE.0 - 8);
    let (result, refs) = release(1, 1, outside);
    assert_eq!(result, Release::Refused);
    assert!(refs.calls.borrow().is_empty(), "neither called");

    let refs = FakeRefs::default();
    assert_eq!(
        release_owned_text(&mem(&[0u8; 0x400]), &refs, 0, IMAGE),
        Release::Untouched
    );
    assert_eq!(
        release_owned_text(&mem(&[0u8; 0x400]), &refs, TEXT, IMAGE),
        Release::Untouched,
        "null controller"
    );
}

#[test]
fn image_range() {
    assert!(in_image(IMAGE.0, IMAGE.0, IMAGE.1));
    assert!(!in_image(IMAGE.0 + IMAGE.1, IMAGE.0, IMAGE.1));
    assert!(!in_image(IMAGE.0 - 1, IMAGE.0, IMAGE.1));
    assert!(!in_image(usize::MAX, usize::MAX - 1, 8));
    let mut pe = vec![0u8; 0x200];
    put(&mut pe, BASE, b"MZ");
    put(&mut pe, BASE + 0x3C, &0x80i32.to_le_bytes());
    put(&mut pe, BASE + 0x80, b"PE\0\0");
    put(&mut pe, BASE + 0x80 + 24 + 56, &0x0712_3000u32.to_le_bytes());
    assert_eq!(image_size(&mem(&pe), BASE), Some(0x0712_3000));
    assert_eq!(image_size(&mem(&pe), BASE + 8), None);
}

#[test]
fn rate_limits() {
    let edges = [
        (Limit::Callout, [(7, true), (8, false), (1023, false), (1024, true)]),
        (Limit::Tick, [(3, true), (4, false), (99_999, false), (100_000, true)]),
        (Limit::Inventory, [(19, true), (20, false), (99, false), (100, true)]),
        (Limit::Hud, [(7, true), (8, false), (15, true), (16, false)]),
    ];
    for (limit, cases) in edges {
        for (n, expected) in cases {
            assert_eq!(written(limit, n), expected, "{limit:?} {n}");
        }
    }
    assert!(written(Limit::Hud, 0));
    assert_eq!(limit_of("UI-1"), Limit::Callout);
    assert_eq!(limit_of("UI-2"), Limit::Callout);
    assert_eq!(limit_of("UI-7"), Limit::Tick);
    assert_eq!(limit_of("UI-12"), Limit::Inventory);
    assert_eq!(limit_of("UI-13"), Limit::Hud);
}

#[test]
fn lines() {
    assert_eq!(
        status_line(true),
        "[WidgetGuard] server guards on (boot patch applied: yes)"
    );
    assert_eq!(
        status_line(false),
        "[WidgetGuard] server guards on (boot patch applied: no)"
    );
    assert_eq!(
        hold_text_line(0x1a2b, Nulls::Members(m::HOLD_TEXT), 3),
        "[WidgetGuard] UI-1 skipped HideHoldText widget=0x1a2b null=HoldText(+0x3c0) count=3"
    );
    assert_eq!(
        hold_text_line(0, Nulls::This, 1),
        "[WidgetGuard] UI-1 skipped HideHoldText widget=null null=this count=1"
    );
    let fade = Fade {
        run: false,
        primary: 0,
        secondary: 0xABC0,
    };
    assert_eq!(
        fade_line(0x10, fade, 2, 1),
        "[WidgetGuard] UI-2 skipped ScreenFadeIn screen=0x10 primary(+0x3e8)=null secondary(+0x3e0)=0xabc0 mode=2 \
         count=1"
    );
    assert_eq!(
        disabled_line(0x20, Nulls::Members(m::HINT | m::ICON), true, 4),
        "[WidgetGuard] UI-3 skipped OnSetDisabled widget=0x20 null=InteractionHint(+0x3a8),DisabledIcon(+0x3b8) \
         disabled=1 count=4 (owned text released)"
    );
    let parent = decide(Verdict::ParentOnly, Nulls::Members(m::RADIAL | m::PROGRESS));
    assert_eq!(
        callout_line("UI-7", 0x30, parent, 1),
        "[WidgetGuard] UI-7 NativeTick widget=0x30 null=RadialAnimImage(+0x398),ProgressMaterial(+0x3e0) \
         action=parent-only count=1"
    );
    assert_eq!(
        callout_line("UI-4", 0x30, decide(Verdict::Skip, Nulls::PlayerInput), 2),
        "[WidgetGuard] UI-4 RefreshInputKeys widget=0x30 null=PlayerInput(pc+0x348) action=skipped count=2"
    );
    assert_eq!(
        callout_line("UI-8", 0x30, decide(Verdict::Skip, Nulls::Members(m::FILL)), 1),
        "[WidgetGuard] UI-8 OnShowFillProgress widget=0x30 null=+0x3a0 action=skipped count=1"
    );
    assert_eq!(
        callout_line("UI-11", 0, decide(Verdict::SkipRelease, Nulls::This), 1),
        "[WidgetGuard] UI-11 OnSetShowDisabledTextWhenEnabled widget=null null=this action=skipped+text-released \
         count=1"
    );
    for (id, site_name) in [
        ("UI-5", "NativeConstruct"),
        ("UI-6", "NativeOnInputSwap"),
        ("UI-9", "OnHideFillProgress"),
        ("UI-10", "OnSetShowDisabledIconWhenEnabled"),
    ] {
        assert_eq!(site(id), site_name);
    }
    assert_eq!(nulls_text(Nulls::Members(0)), "none");
    assert_eq!(
        refresh_line(3, refresh::NO_SLOT, -1, 0x40),
        "[WidgetGuard] UI-12 skipped equipped-item refresh #3: no valid active loadout slot (activeSlot=-1 \
         inventory=0x40)"
    );
    assert_eq!(
        weapon_line(0x50, 1),
        "[WidgetGuard] UI-13 skipped HandleWeaponEquipped(weapon=null) widget=0x50 n=1 (runs again from the \
         character's OnWeaponEquipped)"
    );
    assert_eq!(
        TEXT_REFUSED_LINE,
        "[WidgetGuard] FText release skipped: reference controller vtable outside the game image"
    );
    assert_eq!(off(flags::DISABLE_CALLOUT_GUARDS), "off (DISABLE_CALLOUT_GUARDS.flag)");
    assert_eq!(
        off(flags::DISABLE_INVENTORY_REFRESH_GUARD),
        "off (DISABLE_INVENTORY_REFRESH_GUARD.flag)"
    );
    assert_eq!(
        off(flags::DISABLE_HUD_LEGENDARY_GUARD),
        "off (DISABLE_HUD_LEGENDARY_GUARD.flag)"
    );
    assert_eq!(
        getter_reason(0x01BA_4FF0),
        "getter +0x01BA4FF0 not a function entry or bytes differ"
    );
    assert_eq!(
        group_aborted("UI-5", reason::BYTES_DIFFER),
        "group aborted (UI-5: bytes differ)"
    );
    assert_eq!(
        unhook_failed("MH_ERROR_NOT_CREATED"),
        "unhook failed MH_ERROR_NOT_CREATED, still installed"
    );
}

/// The executable as `verify` sees it: function entries and bytes at RVAs.
#[derive(Default)]
struct FakeCode {
    entries: Vec<(u32, u32)>,
    bytes: HashMap<u32, Vec<u8>>,
}

impl FakeCode {
    fn function(&mut self, rva: u32, len: u32, bytes: &[u8]) {
        self.entries.push((rva, rva + len));
        self.bytes.insert(rva, bytes.to_vec());
    }
}

impl Code for FakeCode {
    fn function_entry(&self, rva: u32) -> Option<u32> {
        self.entries
            .iter()
            .find(|(begin, end)| (*begin..*end).contains(&rva))
            .map(|(begin, _)| *begin)
    }
    fn bytes(&self, rva: u32, out: &mut [u8]) -> bool {
        self.bytes
            .get(&rva)
            .and_then(|bytes| bytes.get(..out.len()))
            .map(|bytes| out.copy_from_slice(bytes))
            .is_some()
    }
}

fn prologue(id: &str) -> &'static [u8] {
    match target(id).map(|line| line.found_by) {
        Some(FoundBy::RvaAndBytes(bytes)) => bytes,
        _ => panic!("{id}"),
    }
}

/// The game as the spec describes it, for the ids `verify` reads.
fn game() -> FakeCode {
    let mut code = FakeCode::default();
    code.bytes.insert(0x01E4_2F00, r6::UI1_LEAF.to_vec());
    code.function(0x01E5_4110, 0x100, &r6::UI3_ENTRY);
    code.function(0x01E4_BA00, 0x100, prologue("UI-5"));
    let mut ui6 = prologue("UI-6").to_vec();
    ui6.resize(0x1A, 0x90);
    ui6.push(0xE8);
    ui6.extend_from_slice(&(0x01E4_DEE0i64 - (0x01E4_E050 + 0x1A + 5)).to_le_bytes()[..4]);
    code.function(0x01E4_E050, 0x100, &ui6);
    for (_, _, parent) in r6::PARENT_CALLS {
        code.function(parent, 0x10, &[0xCC; 16]);
    }
    code.function(0x01B8_6160, 0x100, prologue("UI-12"));
    for getter in r6::GETTERS {
        code.function(getter, 0x10, &r6::GETTER_BYTES);
    }
    code
}

#[test]
fn verify_targets() {
    let check = |code: &FakeCode, id: &str| verify(code, target(id).unwrap());
    let good = game();
    for id in ["UI-1", "UI-3", "UI-5", "UI-6", "UI-12"] {
        assert_eq!(check(&good, id), Ok(()), "{id}");
    }
    assert_eq!(check(&good, "UI-2"), Err(reason::NOT_ENTRY.to_owned()));

    let mut leaf = game();
    leaf.bytes.insert(0x01E4_2F00, vec![0x90; 19]);
    assert_eq!(check(&leaf, "UI-1"), Err(reason::BYTES_DIFFER.to_owned()));
    leaf.bytes.remove(&0x01E4_2F00);
    assert_eq!(check(&leaf, "UI-1"), Err(reason::UNREADABLE.to_owned()));

    let mut moved = game();
    moved.entries.retain(|(begin, _)| *begin != 0x01E5_4110);
    moved.entries.push((0x01E5_4120, 0x01E5_4200));
    assert_eq!(
        check(&moved, "UI-3"),
        Err(reason::NOT_ENTRY.to_owned()),
        "entry not 0x01E54110"
    );
    let mut ui3 = game();
    ui3.bytes.insert(0x01E5_4110, vec![0x90; 24]);
    assert_eq!(check(&ui3, "UI-3"), Err(reason::BYTES_DIFFER.to_owned()));

    let mut parent = game();
    parent.entries.retain(|(begin, _)| *begin != 0x01E4_A480);
    assert_eq!(check(&parent, "UI-5"), Err(reason::PARENT_NOT_ENTRY.to_owned()));
    let mut call = game();
    let mut ui6 = call.bytes[&0x01E4_E050].clone();
    ui6[0x1B] ^= 1;
    call.bytes.insert(0x01E4_E050, ui6);
    assert_eq!(check(&call, "UI-6"), Err(reason::PARENT_DIFFERS.to_owned()));

    let mut getter = game();
    getter.bytes.insert(r6::GETTERS[2], vec![0x90; 5]);
    assert_eq!(check(&getter, "UI-12"), Err(getter_reason(0x01B9_FF30)));
    assert_eq!(parent_of("UI-7"), Some(0x01E5_0390));
    assert_eq!(parent_of("UI-4"), None);
}
