//! R6's guards on synthetic widgets and test functions (spec-runtime-r6 14.1). Hooks stay for the process, so each
//! test hooks its own functions and uses its own guard.

use super::*;
use crate::ffi::win;
use crate::widgets::install_group;
use std::hint::black_box;
use std::sync::Mutex;

static CALLS: Mutex<Vec<(&'static str, usize, u64)>> = Mutex::new(Vec::new());

fn record(what: &'static str, a: usize, b: u64) {
    CALLS.lock().unwrap().push((what, a, b));
}

fn calls(what: &str) -> Vec<(usize, u64)> {
    CALLS
        .lock()
        .unwrap()
        .iter()
        .filter(|(name, ..)| *name == what)
        .map(|&(_, a, b)| (a, b))
        .collect()
}

fn address(function: unsafe extern "C" fn(usize)) -> usize {
    function as *const () as usize
}

fn call(target: usize, argument: usize) {
    // SAFETY: a test function of this signature, called through its patched entry
    unsafe { black_box(std::mem::transmute::<usize, WidgetFn>(target))(argument) };
}

macro_rules! group_case {
    ($target:ident, $wrapper:ident, $cell:ident, $name:literal, $salt:literal) => {
        #[inline(never)]
        unsafe extern "C" fn $target(argument: usize) {
            record(concat!($name, "-original"), argument, black_box($salt));
        }
        static $cell: NextCell = NextCell::new();
        unsafe extern "C" fn $wrapper(argument: usize) {
            record(concat!($name, "-guard"), argument, $salt);
            let next = $cell.load();
            if next != 0 {
                call(next, argument);
            }
        }
    };
}

group_case!(a4, w_a4, C_A4, "a4", 41);
group_case!(a5, w_a5, C_A5, "a5", 51);
group_case!(a6, w_a6, C_A6, "a6", 61);
group_case!(b4, w_b4, C_B4, "b4", 42);
group_case!(b5, w_b5, C_B5, "b5", 52);
group_case!(b6, w_b6, C_B6, "b6", 62);
group_case!(c4, w_c4, C_C4, "c4", 43);
group_case!(c5, w_c5, C_C5, "c5", 53);
group_case!(c6, w_c6, C_C6, "c6", 63);

type Wired = (usize, usize, &'static NextCell);

fn hooker(wired: [Wired; 3], fail_third: bool) -> impl Fn(&str, usize) -> Result<(), i32> {
    move |id, at| {
        let index = INPUT_IDS.iter().position(|&group| group == id).unwrap();
        let (target, wrapper, cell) = wired[index];
        assert_eq!(at, target);
        if fail_third && index == 2 {
            return Err(9);
        }
        broker::hook(target, wrapper, 0, cell)
    }
}

fn unhooker(wired: [Wired; 3]) -> impl Fn(&str, usize) -> Result<(), i32> {
    move |id, at| {
        let index = INPUT_IDS.iter().position(|&group| group == id).unwrap();
        broker::unhook(at, wired[index].1)
    }
}

const INPUT_IDS: [&str; 3] = ["UI-4", "UI-5", "UI-6"];

fn verified(wired: [Wired; 3], third: Result<(), &str>) -> [(&'static str, Result<usize, String>); 3] {
    [
        (INPUT_IDS[0], Ok(wired[0].0)),
        (INPUT_IDS[1], Ok(wired[1].0)),
        (INPUT_IDS[2], third.map(|()| wired[2].0).map_err(str::to_owned)),
    ]
}

#[test]
fn input_group_all_or_nothing() {
    let all: [Wired; 3] = [
        (address(a4), address(w_a4), &C_A4),
        (address(a5), address(w_a5), &C_A5),
        (address(a6), address(w_a6), &C_A6),
    ];
    let outcomes = install_group(verified(all, Ok(())), hooker(all, false), unhooker(all));
    assert_eq!(outcomes, INPUT_IDS.map(|id| (id, Ok(()))));
    for (target, ..) in all {
        call(target, 7);
    }
    for name in ["a4", "a5", "a6"] {
        assert_eq!(calls(&format!("{name}-guard")).len(), 1, "{name} guarded");
        assert_eq!(calls(&format!("{name}-original")).len(), 1, "{name} reached");
    }

    // The third fails to verify: none hooked, each says why
    let unverified: [Wired; 3] = [
        (address(b4), address(w_b4), &C_B4),
        (address(b5), address(w_b5), &C_B5),
        (address(b6), address(w_b6), &C_B6),
    ];
    let outcomes = install_group(
        verified(unverified, Err("bytes differ")),
        hooker(unverified, false),
        unhooker(unverified),
    );
    let aborted = Err("group aborted (UI-6: bytes differ)".to_owned());
    assert_eq!(outcomes, INPUT_IDS.map(|id| (id, aborted.clone())));
    for (target, ..) in unverified {
        call(target, 8);
    }
    for name in ["b4", "b5", "b6"] {
        assert!(calls(&format!("{name}-guard")).is_empty(), "{name} not hooked");
        assert_eq!(calls(&format!("{name}-original")).len(), 1);
    }

    // The third fails to hook: the first two are unhooked and run unguarded
    let rolled_back: [Wired; 3] = [
        (address(c4), address(w_c4), &C_C4),
        (address(c5), address(w_c5), &C_C5),
        (address(c6), address(w_c6), &C_C6),
    ];
    let outcomes = install_group(
        verified(rolled_back, Ok(())),
        hooker(rolled_back, true),
        unhooker(rolled_back),
    );
    let aborted = Err("group aborted (UI-6: MH_ERROR_MEMORY_ALLOC)".to_owned());
    assert_eq!(
        outcomes,
        vec![
            ("UI-4", aborted.clone()),
            ("UI-5", aborted),
            ("UI-6", Err("MH_ERROR_MEMORY_ALLOC".to_owned())),
        ]
    );
    for (target, ..) in rolled_back {
        call(target, 9);
    }
    for name in ["c4", "c5", "c6"] {
        assert!(calls(&format!("{name}-guard")).is_empty(), "{name} unguarded");
        assert_eq!(calls(&format!("{name}-original")).len(), 1);
    }
}

/// A callout widget in this process: every member bound to a readable word, as `u64`s so it is aligned.
fn callout(nulls: &[usize]) -> Vec<u64> {
    let mut widget = vec![0u64; 0x400 / 8];
    let bound = widget.as_ptr() as u64;
    for member in core::MEMBERS {
        widget[member.offset / 8] = bound;
    }
    for &offset in nulls {
        widget[offset / 8] = 0;
    }
    widget
}

#[inline(never)]
unsafe extern "C" fn swap_original(widget: usize, gamepad: u8) {
    record("swap-original", widget, u64::from(black_box(gamepad)));
}

#[inline(never)]
unsafe extern "C" fn swap_parent(widget: usize, gamepad: u8) {
    record("swap-parent", widget, u64::from(black_box(gamepad)) + 100);
}

#[test]
fn input_swap_runs_the_parent_only() {
    PARENTS[1].store(swap_parent as *const () as usize, Ordering::SeqCst);
    let target = swap_original as *const () as usize;
    hook("UI-6", target).unwrap();
    let swap = |widget: &[u64], gamepad: u8| {
        // SAFETY: the hooked test function
        unsafe { black_box(std::mem::transmute::<usize, SwapFn>(target))(widget.as_ptr() as usize, gamepad) };
    };
    let bound = callout(&[]);
    swap(&bound, 1);
    assert_eq!(calls("swap-original"), [(bound.as_ptr() as usize, 1)]);
    assert!(calls("swap-parent").is_empty());

    let keyboard_null = callout(&[r6::CALLOUT_KEYBOARD]);
    let before = guard(5).skips();
    swap(&keyboard_null, 0);
    assert_eq!(
        calls("swap-parent"),
        [(keyboard_null.as_ptr() as usize, 100)],
        "the parent once"
    );
    assert_eq!(calls("swap-original").len(), 1, "the original never");
    assert_eq!(guard(5).skips(), before + 1);
}

#[inline(never)]
unsafe extern "C" fn fake_destroy(controller: usize) {
    record("destroy", controller, black_box(0));
}

#[inline(never)]
unsafe extern "C" fn fake_delete(controller: usize, flags: u32) {
    record("delete", controller, u64::from(black_box(flags)));
}

/// An `FText` whose reference controller has the given counts and a vtable of the two fakes.
struct FakeText {
    text: Vec<u64>,
    controller: Vec<u64>,
    _vtable: Vec<u64>,
}

impl FakeText {
    fn new(shared: i32, weak: i32) -> Self {
        let vtable = vec![fake_destroy as *const () as u64, fake_delete as *const () as u64];
        let counts = u64::from(shared as u32) | (u64::from(weak as u32) << 32);
        let controller = vec![vtable.as_ptr() as u64, counts];
        let text = vec![0, controller.as_ptr() as u64, 0];
        Self {
            text,
            controller,
            _vtable: vtable,
        }
    }
    fn text(&self) -> usize {
        self.text.as_ptr() as usize
    }
    fn controller(&self) -> usize {
        self.controller.as_ptr() as usize
    }
    fn counts(&self) -> (i32, i32) {
        let word = self.controller[1];
        (word as u32 as i32, (word >> 32) as u32 as i32)
    }
}

#[test]
fn text_release_calls_the_controller() {
    let base = win::image_base();
    IMAGE_BASE.store(base, Ordering::SeqCst);
    IMAGE_SIZE.store(core::image_size(&Plain, base).unwrap(), Ordering::SeqCst);
    let mine = |what: &str, controller: usize| calls(what).into_iter().filter(|(c, _)| *c == controller).count();

    let shared = FakeText::new(2, 1);
    assert_eq!(release_text(shared.text(), false), Release::Decremented);
    assert_eq!(shared.counts(), (1, 1));
    assert_eq!(mine("destroy", shared.controller()), 0);

    let weak = FakeText::new(1, 2);
    assert_eq!(release_text(weak.text(), false), Release::Destroyed);
    assert_eq!(weak.counts(), (0, 1));
    assert_eq!(mine("destroy", weak.controller()), 1);
    assert_eq!(mine("delete", weak.controller()), 0);

    let last = FakeText::new(1, 1);
    assert_eq!(release_text(last.text(), false), Release::Deleted);
    assert_eq!(last.counts(), (0, 0));
    assert_eq!(mine("destroy", last.controller()), 1);
    assert_eq!(
        calls("delete")
            .into_iter()
            .filter(|&(c, flag)| c == last.controller() && flag == 1)
            .count(),
        1
    );

    assert_eq!(release_text(0, false), Release::Untouched);
}

#[inline(never)]
unsafe extern "C" fn weapon_original(widget: usize, weapon: usize) {
    record("weapon-original", widget, black_box(weapon) as u64);
}

#[test]
fn detaching_decides_without_logging() {
    let target = weapon_original as *const () as usize;
    hook("UI-13", target).unwrap();
    let guard = guard(12);
    let before = guard.skips();
    handle_weapon_equipped(0x10, 0, true);
    assert_eq!(guard.skips(), before, "nothing counted while detaching");
    assert!(calls("weapon-original").is_empty(), "the null call is still skipped");
    handle_weapon_equipped(0x10, 0x20, true);
    assert_eq!(calls("weapon-original"), [(0x10, 0x20)], "a weapon still passes");
    handle_weapon_equipped(0x10, 0, false);
    assert_eq!(guard.skips(), before + 1);
    assert_eq!(calls("weapon-original").len(), 1);
}
