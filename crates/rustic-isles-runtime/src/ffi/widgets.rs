//! R6's detour bodies (spec-runtime-r6 sections 4 to 10): each reads the widget through checked memory, asks the
//! core crate's decision, and calls the game's function, its parent's body, or nothing.

use super::broker::{self, NextCell};
use super::memory as checked;
use super::{detaching, engine, log};
use rustic_isles_runtime_core::memory::Memory;
use rustic_isles_runtime_core::offsets::r6;
use rustic_isles_runtime_core::reflect::{self, is_writable_protect};
use rustic_isles_runtime_core::weak::resolve_weak;
use rustic_isles_runtime_core::widgets::{self as core, Code, Decision, Release, TextRefs, Verdict};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicUsize, Ordering};

type WidgetFn = unsafe extern "C" fn(usize);
type ScreenFn = unsafe extern "C" fn(usize, u8);
type DisabledFn = unsafe extern "C" fn(usize, u8, usize, usize);
type SwapFn = unsafe extern "C" fn(usize, u8);
type TickFn = unsafe extern "C" fn(usize, usize, f32);
type FillFn = unsafe extern "C" fn(usize, f32);
type PairFn = unsafe extern "C" fn(usize, usize);
type TextFn = unsafe extern "C" fn(usize, usize, f32, u8);
type GetterFn = unsafe extern "C" fn(usize) -> usize;
type DeleteFn = unsafe extern "C" fn(usize, u32);

/// One guard: its broker cell and its skip counter.
pub struct Guard {
    /// The surface id.
    pub id: &'static str,
    /// The wrapper's next.
    pub next: NextCell,
    skips: AtomicU64,
}

impl Guard {
    const fn new(id: &'static str) -> Self {
        Self {
            id,
            next: NextCell::new(),
            skips: AtomicU64::new(0),
        }
    }

    /// Skips counted so far.
    pub fn skips(&self) -> u64 {
        self.skips.load(Ordering::Relaxed)
    }

    /// The next function as `F`, or `None` before the broker wired it.
    fn next<F: Copy>(&self) -> Option<F> {
        let next = self.next.load();
        // SAFETY: the broker wrote a function of the hooked target's signature, which each caller names as `F`
        (next != 0).then(|| unsafe { std::mem::transmute_copy::<usize, F>(&next) })
    }

    /// Counts a skip and writes its line when 13.3 says so. Detaching: neither (11.1).
    fn skip(&self, quiet: bool, line: impl FnOnce(u64) -> String) {
        if quiet {
            return;
        }
        let n = self.skips.fetch_add(1, Ordering::Relaxed);
        if core::written(core::limit_of(self.id), n) {
            let _ = catch_unwind(AssertUnwindSafe(|| log::line(&line(n + 1))));
        }
    }
}

/// UI-1..13 in order.
pub static GUARDS: [Guard; 13] = [
    Guard::new("UI-1"),
    Guard::new("UI-2"),
    Guard::new("UI-3"),
    Guard::new("UI-4"),
    Guard::new("UI-5"),
    Guard::new("UI-6"),
    Guard::new("UI-7"),
    Guard::new("UI-8"),
    Guard::new("UI-9"),
    Guard::new("UI-10"),
    Guard::new("UI-11"),
    Guard::new("UI-12"),
    Guard::new("UI-13"),
];

fn guard(index: usize) -> &'static Guard {
    &GUARDS[index]
}

static IMAGE_BASE: AtomicUsize = AtomicUsize::new(0);
static IMAGE_SIZE: AtomicUsize = AtomicUsize::new(0);
/// UI-5, UI-6, UI-7's parent bodies.
static PARENTS: [AtomicUsize; 3] = [const { AtomicUsize::new(0) }; 3];
/// UI-12's getters.
static GETTERS: [AtomicUsize; 5] = [const { AtomicUsize::new(0) }; 5];
static TEXT_REFUSED: AtomicBool = AtomicBool::new(false);

/// Records the executable's range and the parent and getter addresses (Init, before any hook).
pub fn set_image(base: usize) {
    IMAGE_BASE.store(base, Ordering::SeqCst);
    IMAGE_SIZE.store(core::image_size(&Plain, base).unwrap_or(0), Ordering::SeqCst);
    let at = |rva: u32| {
        usize::try_from(rva)
            .ok()
            .and_then(|rva| base.checked_add(rva))
            .unwrap_or(0)
    };
    for (cell, (_, _, parent)) in PARENTS.iter().zip(r6::PARENT_CALLS) {
        cell.store(at(parent), Ordering::SeqCst);
    }
    for (cell, getter) in GETTERS.iter().zip(r6::GETTERS) {
        cell.store(at(getter), Ordering::SeqCst);
    }
}

fn image() -> (usize, usize) {
    (IMAGE_BASE.load(Ordering::Relaxed), IMAGE_SIZE.load(Ordering::Relaxed))
}

fn in_image(address: usize) -> bool {
    let (base, size) = image();
    core::in_image(address, base, size)
}

/// R0's plain checked reads (every guard but UI-7).
struct Plain;

impl Memory for Plain {
    fn readable(&self, ptr: usize, size: usize) -> bool {
        checked::readable(ptr, size)
    }
    fn read(&self, addr: usize, out: &mut [u8]) -> bool {
        checked::read_bytes(addr, out)
    }
}

/// UI-7's reads through the server's frame cache: after a frame's first call, a table lookup (7.5).
struct Frame;

impl Memory for Frame {
    fn readable(&self, ptr: usize, size: usize) -> bool {
        checked::readable_this_frame(ptr, size)
    }
    fn read(&self, addr: usize, out: &mut [u8]) -> bool {
        if !checked::readable_this_frame(addr, out.len()) {
            return false;
        }
        // SAFETY: the range was found mapped and readable this frame; `out` is a distinct buffer
        unsafe { std::ptr::copy_nonoverlapping(addr as *const u8, out.as_mut_ptr(), out.len()) };
        true
    }
}

/// The executable as `verify` reads it.
pub struct GameCode;

impl Code for GameCode {
    fn function_entry(&self, rva: u32) -> Option<u32> {
        checked::function_entry(rva)
    }
    fn bytes(&self, rva: u32, out: &mut [u8]) -> bool {
        usize::try_from(rva)
            .ok()
            .and_then(|rva| IMAGE_BASE.load(Ordering::SeqCst).checked_add(rva))
            .is_some_and(|address| checked::read_bytes(address, out))
    }
}

/// The `FText` controller in game memory: interlocked counts, calls only into the executable.
struct GameRefs;

impl TextRefs for GameRefs {
    fn decrement(&self, address: usize) -> Option<i32> {
        let writable = checked::query(address).is_some_and(|region| is_writable_protect(region.protect));
        if !address.is_multiple_of(4) || !writable || !checked::readable_unaligned(address, 4) {
            return None;
        }
        // SAFETY: an aligned, committed, writable i32 the game itself updates with interlocked operations
        let count = unsafe { AtomicI32::from_ptr(address as *mut i32) };
        Some(count.fetch_sub(1, Ordering::SeqCst) - 1)
    }
    fn destroy(&self, entry: usize, controller: usize) {
        // SAFETY: `release_owned_text` checked the entry lies in the executable; vtable entry 0 is `(controller)`
        unsafe { std::mem::transmute::<usize, WidgetFn>(entry)(controller) };
    }
    fn delete(&self, entry: usize, controller: usize) {
        // SAFETY: as `destroy`; entry 1 is `(controller, 1)`
        unsafe { std::mem::transmute::<usize, DeleteFn>(entry)(controller, 1) };
    }
}

/// Releases a skipped body's `FText` (3.5).
pub(crate) fn release_text(text: usize, quiet: bool) -> Release {
    let release = core::release_owned_text(&Plain, &GameRefs, text, image());
    if release == Release::Refused && !quiet && !TEXT_REFUSED.swap(true, Ordering::SeqCst) {
        log::line(core::TEXT_REFUSED_LINE);
    }
    release
}

/// `is_a(object, "ArchonPlayerController")`; may fill R0's name cache, which is not logging (11.1).
fn is_player_controller(object: usize) -> bool {
    engine::engine().is_some_and(|engine| reflect::is_a(&engine, object, r6::PLAYER_CONTROLLER))
}

/// The cached controller's weak pointer (3.4).
fn resolve(index: i32, serial: i32) -> Option<usize> {
    engine::engine().and_then(|engine| resolve_weak(&engine, index, serial))
}

/// `GetOwningPlayer` through the widget's vtable, only when the entry is in the executable (3.6).
fn owning_player(widget: usize) -> Option<usize> {
    let vtable = Plain.ptr_at(widget)?;
    let entry = Plain.ptr_at(vtable.checked_add(r6::GET_OWNING_PLAYER_VSLOT)?)?;
    if !in_image(entry) {
        return None;
    }
    // SAFETY: an executable entry of the widget's own vtable, `(widget) -> APlayerController*`
    Some(unsafe { std::mem::transmute::<usize, GetterFn>(entry)(widget) })
}

fn parent<F: Copy>(index: usize) -> Option<F> {
    let address = PARENTS[index].load(Ordering::Relaxed);
    // SAFETY: Init stored the parent body verified for this id (3.3), of the hooked function's signature
    (address != 0).then(|| unsafe { std::mem::transmute_copy::<usize, F>(&address) })
}

fn call_getter(index: usize, loadout: usize) -> usize {
    let address = GETTERS.get(index).map_or(0, |cell| cell.load(Ordering::Relaxed));
    if address == 0 {
        return 0;
    }
    // SAFETY: a getter verified at install (3.3), `(loadout) -> pointer`, called only with a valid slot
    unsafe { std::mem::transmute::<usize, GetterFn>(address)(loadout) }
}

// UI-1
pub(crate) fn hide_hold_text(widget: usize, quiet: bool) {
    let guard = guard(0);
    let decision = core::hide_hold_text(&Plain, widget);
    if decision.verdict == Verdict::Next {
        if let Some(next) = guard.next::<WidgetFn>() {
            // SAFETY: the game's function with its own argument
            unsafe { next(widget) };
        }
        return;
    }
    guard.skip(quiet, |count| core::hold_text_line(widget, decision.nulls, count));
}

unsafe extern "C" fn ui1(widget: usize) {
    hide_hold_text(widget, detaching());
}

// UI-2
pub(crate) fn screen_fade_in(screen: usize, mode: u8, quiet: bool) {
    let guard = guard(1);
    let fade = core::screen_fade_in(&Plain, screen);
    if fade.run {
        if let Some(next) = guard.next::<ScreenFn>() {
            // SAFETY: as UI-1
            unsafe { next(screen, mode) };
        }
        return;
    }
    guard.skip(quiet, |count| core::fade_line(screen, fade, mode, count));
}

unsafe extern "C" fn ui2(screen: usize, mode: u8) {
    screen_fade_in(screen, mode, detaching());
}

// UI-3
pub(crate) fn on_set_disabled(widget: usize, disabled: u8, reason: usize, icon: usize, quiet: bool) {
    let guard = guard(2);
    let decision = core::on_set_disabled(&Plain, widget);
    if decision.verdict == Verdict::Next {
        if let Some(next) = guard.next::<DisabledFn>() {
            // SAFETY: as UI-1; the body releases the text itself
            unsafe { next(widget, disabled, reason, icon) };
        }
        return;
    }
    release_text(reason, quiet);
    guard.skip(quiet, |count| {
        core::disabled_line(widget, decision.nulls, disabled != 0, count)
    });
}

unsafe extern "C" fn ui3(widget: usize, disabled: u8, reason: usize, icon: usize) {
    on_set_disabled(widget, disabled, reason, icon, detaching());
}

/// The skip line for a UI-4..11 decision.
fn callout_skip(index: usize, widget: usize, decision: Decision, quiet: bool) {
    let guard = guard(index);
    guard.skip(quiet, |count| core::callout_line(guard.id, widget, decision, count));
}

// UI-4
pub(crate) fn refresh_input_keys(widget: usize, quiet: bool) {
    let decision = core::refresh_input_keys(&Plain, widget, resolve, || owning_player(widget), is_player_controller);
    if decision.verdict == Verdict::Next {
        if let Some(next) = guard(3).next::<WidgetFn>() {
            // SAFETY: as UI-1
            unsafe { next(widget) };
        }
        return;
    }
    callout_skip(3, widget, decision, quiet);
}

unsafe extern "C" fn ui4(widget: usize) {
    refresh_input_keys(widget, detaching());
}

// UI-5
pub(crate) fn native_construct(widget: usize, quiet: bool) {
    let decision = core::native_construct(&Plain, widget);
    match decision.verdict {
        Verdict::Next => {
            if let Some(next) = guard(4).next::<WidgetFn>() {
                // SAFETY: as UI-1
                unsafe { next(widget) };
            }
        }
        Verdict::ParentOnly => {
            if let Some(parent) = parent::<WidgetFn>(0) {
                // SAFETY: the parent body, not hooked, with the same argument (3.6)
                unsafe { parent(widget) };
            }
            callout_skip(4, widget, decision, quiet);
        }
        _ => {}
    }
}

unsafe extern "C" fn ui5(widget: usize) {
    native_construct(widget, detaching());
}

// UI-6
pub(crate) fn native_on_input_swap(widget: usize, gamepad: u8, quiet: bool) {
    let decision = core::native_on_input_swap(&Plain, widget);
    match decision.verdict {
        Verdict::Next => {
            if let Some(next) = guard(5).next::<SwapFn>() {
                // SAFETY: as UI-1
                unsafe { next(widget, gamepad) };
            }
        }
        Verdict::ParentOnly => {
            if let Some(parent) = parent::<SwapFn>(1) {
                // SAFETY: as UI-5
                unsafe { parent(widget, gamepad) };
            }
            callout_skip(5, widget, decision, quiet);
        }
        _ => {}
    }
}

unsafe extern "C" fn ui6(widget: usize, gamepad: u8) {
    native_on_input_swap(widget, gamepad, detaching());
}

// UI-7
pub(crate) fn native_tick(widget: usize, geometry: usize, delta: f32, quiet: bool) {
    let decision = core::native_tick(&Frame, widget);
    match decision.verdict {
        Verdict::Next => {
            if let Some(next) = guard(6).next::<TickFn>() {
                // SAFETY: as UI-1
                unsafe { next(widget, geometry, delta) };
            }
        }
        Verdict::ParentOnly => {
            if let Some(parent) = parent::<TickFn>(2) {
                // SAFETY: as UI-5
                unsafe { parent(widget, geometry, delta) };
            }
            callout_skip(6, widget, decision, quiet);
        }
        _ => {}
    }
}

unsafe extern "C" fn ui7(widget: usize, geometry: usize, delta: f32) {
    native_tick(widget, geometry, delta, detaching());
}

// UI-8
unsafe extern "C" fn ui8(widget: usize, duration: f32) {
    let decision = core::fill_progress(&Plain, widget);
    if decision.verdict == Verdict::Next {
        if let Some(next) = guard(7).next::<FillFn>() {
            // SAFETY: as UI-1
            unsafe { next(widget, duration) };
        }
        return;
    }
    callout_skip(7, widget, decision, detaching());
}

// UI-9
unsafe extern "C" fn ui9(widget: usize) {
    let decision = core::fill_progress(&Plain, widget);
    if decision.verdict == Verdict::Next {
        if let Some(next) = guard(8).next::<WidgetFn>() {
            // SAFETY: as UI-1
            unsafe { next(widget) };
        }
        return;
    }
    callout_skip(8, widget, decision, detaching());
}

// UI-10
unsafe extern "C" fn ui10(widget: usize, texture: usize) {
    let decision = core::show_disabled_icon(&Plain, widget);
    if decision.verdict == Verdict::Next {
        if let Some(next) = guard(9).next::<PairFn>() {
            // SAFETY: as UI-1
            unsafe { next(widget, texture) };
        }
        return;
    }
    callout_skip(9, widget, decision, detaching());
}

// UI-11
pub(crate) fn show_disabled_text(widget: usize, text: usize, offset: f32, mark: u8, quiet: bool) {
    let decision = core::show_disabled_text(&Plain, widget, mark != 0);
    if decision.verdict == Verdict::Next {
        if let Some(next) = guard(10).next::<TextFn>() {
            // SAFETY: as UI-1; the body releases the text itself
            unsafe { next(widget, text, offset, mark) };
        }
        return;
    }
    release_text(text, quiet);
    callout_skip(10, widget, decision, quiet);
}

unsafe extern "C" fn ui11(widget: usize, text: usize, offset: f32, mark: u8) {
    show_disabled_text(widget, text, offset, mark, detaching());
}

// UI-12
pub(crate) fn equipped_refresh(inventory: usize, updated: usize, quiet: bool) {
    let guard = guard(11);
    let refresh = core::equipped_refresh(&Plain, inventory, is_player_controller, call_getter);
    let Some(why) = refresh.skip else {
        if let Some(next) = guard.next::<PairFn>() {
            // SAFETY: as UI-1
            unsafe { next(inventory, updated) };
        }
        return;
    };
    guard.skip(quiet, |n| core::refresh_line(n, why, refresh.slot, inventory));
}

unsafe extern "C" fn ui12(inventory: usize, updated: usize) {
    equipped_refresh(inventory, updated, detaching());
}

// UI-13
pub(crate) fn handle_weapon_equipped(widget: usize, weapon: usize, quiet: bool) {
    let guard = guard(12);
    if weapon != 0 {
        if let Some(next) = guard.next::<PairFn>() {
            // SAFETY: as UI-1
            unsafe { next(widget, weapon) };
        }
        return;
    }
    guard.skip(quiet, |n| core::weapon_line(widget, n));
}

unsafe extern "C" fn ui13(widget: usize, weapon: usize) {
    handle_weapon_equipped(widget, weapon, detaching());
}

/// The detour for a guard index.
fn detour(index: usize) -> usize {
    match index {
        0 => ui1 as *const () as usize,
        1 => ui2 as *const () as usize,
        2 => ui3 as *const () as usize,
        3 => ui4 as *const () as usize,
        4 => ui5 as *const () as usize,
        5 => ui6 as *const () as usize,
        6 => ui7 as *const () as usize,
        7 => ui8 as *const () as usize,
        8 => ui9 as *const () as usize,
        9 => ui10 as *const () as usize,
        10 => ui11 as *const () as usize,
        11 => ui12 as *const () as usize,
        _ => ui13 as *const () as usize,
    }
}

/// The guard index of a UI id.
pub fn index_of(id: &str) -> Option<usize> {
    GUARDS.iter().position(|guard| guard.id == id)
}

/// Hooks guard `id` on `address` through the broker (`order = 0`).
pub fn hook(id: &str, address: usize) -> Result<(), i32> {
    let index = index_of(id).ok_or(-1)?;
    broker::hook(address, detour(index), 0, &GUARDS[index].next)
}

/// Removes guard `id`'s wrapper from `address`.
pub fn unhook(id: &str, address: usize) -> Result<(), i32> {
    let index = index_of(id).ok_or(-1)?;
    broker::unhook(address, detour(index))
}

#[cfg(test)]
mod tests;
