//! The widget crash guards (R6, spec-runtime-r6): what each of UI-1..13 decides over a widget it reads through
//! checked memory, how a target is verified, the rate limits and the log text. The DLL's `widgets` modules hook.

use crate::memory::Memory;
use crate::offsets::{FoundBy, Target, r6};

/// R6's flags (RT-14), each read once in Init.
pub mod flags {
    /// UI-4 to UI-11 off.
    pub const DISABLE_CALLOUT_GUARDS: &str = "DISABLE_CALLOUT_GUARDS";
    /// UI-12 off.
    pub const DISABLE_INVENTORY_REFRESH_GUARD: &str = "DISABLE_INVENTORY_REFRESH_GUARD";
    /// UI-13 off.
    pub const DISABLE_HUD_LEGENDARY_GUARD: &str = "DISABLE_HUD_LEGENDARY_GUARD";
}

/// Install reasons (13.1) other than `ok` and a MinHook status.
pub mod reason {
    /// No `.pdata` entry starts there.
    pub const NOT_ENTRY: &str = "not a function entry";
    /// The entry's bytes are not the spec's.
    pub const BYTES_DIFFER: &str = "bytes differ";
    /// The bytes could not be read.
    pub const UNREADABLE: &str = "unreadable";
    /// UI-5..7: the call at the parent offset is not to the parent.
    pub const PARENT_DIFFERS: &str = "parent call differs";
    /// UI-5..7: the parent is not a function entry.
    pub const PARENT_NOT_ENTRY: &str = "parent not a function entry";
}

/// `off (<FLAG>.flag)`.
pub fn off(flag: &str) -> String {
    format!("off ({flag}.flag)")
}

/// UI-12's getter reason, the RVA as `+0x01BA4FF0`.
pub fn getter_reason(rva: u32) -> String {
    format!("getter +0x{rva:08X} not a function entry or bytes differ")
}

/// UI-4..6: a sibling's failure.
pub fn group_aborted(id: &str, why: &str) -> String {
    format!("group aborted ({id}: {why})")
}

/// UI-4..6: a rollback `unhook` that failed.
pub fn unhook_failed(status: &str) -> String {
    format!("unhook failed {status}, still installed")
}

/// The input group, all or nothing (7.1).
pub const INPUT_GROUP: [&str; 3] = ["UI-4", "UI-5", "UI-6"];
/// Installed each on its own (7.1).
pub const CALLOUT_SINGLES: [&str; 5] = ["UI-7", "UI-8", "UI-9", "UI-10", "UI-11"];

/// One callout widget member, in 3.2's table order (the log's order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Member {
    /// Its name in the log (`""` for the unnamed one).
    pub name: &'static str,
    /// Its offset.
    pub offset: usize,
}

/// The callout members the guards read; bit `i` of a mask is `MEMBERS[i]`.
pub const MEMBERS: [Member; 9] = [
    Member {
        name: "Keyboard",
        offset: r6::CALLOUT_KEYBOARD,
    },
    Member {
        name: "Gamepad",
        offset: r6::CALLOUT_GAMEPAD,
    },
    Member {
        name: "RadialAnimImage",
        offset: r6::CALLOUT_RADIAL,
    },
    Member {
        name: "",
        offset: r6::CALLOUT_3A0,
    },
    Member {
        name: "InteractionHint",
        offset: r6::CALLOUT_HINT,
    },
    Member {
        name: "DisabledReason",
        offset: r6::CALLOUT_REASON,
    },
    Member {
        name: "DisabledIcon",
        offset: r6::CALLOUT_ICON,
    },
    Member {
        name: "HoldText",
        offset: r6::CALLOUT_HOLD_TEXT,
    },
    Member {
        name: "ProgressMaterial",
        offset: r6::CALLOUT_PROGRESS,
    },
];

/// Member masks.
pub mod m {
    /// `Keyboard`.
    pub const KEYBOARD: u16 = 1 << 0;
    /// `Gamepad`.
    pub const GAMEPAD: u16 = 1 << 1;
    /// `RadialAnimImage`.
    pub const RADIAL: u16 = 1 << 2;
    /// `+0x3A0`.
    pub const FILL: u16 = 1 << 3;
    /// `InteractionHint`.
    pub const HINT: u16 = 1 << 4;
    /// `DisabledReason`.
    pub const REASON: u16 = 1 << 5;
    /// `DisabledIcon`.
    pub const ICON: u16 = 1 << 6;
    /// `HoldText`.
    pub const HOLD_TEXT: u16 = 1 << 7;
    /// `ProgressMaterial`.
    pub const PROGRESS: u16 = 1 << 8;
}

/// What a skip names as null.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nulls {
    /// The widget itself (null or unreadable).
    This,
    /// These members (a mask over `MEMBERS`).
    Members(u16),
    /// UI-4's controller has no `PlayerInput`.
    PlayerInput,
}

/// What a guard does with the call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Call the game's function.
    Next,
    /// Call only the parent's body (UI-5..7), and log.
    ParentOnly,
    /// Return without calling anything, and log.
    Skip,
    /// Release the owned `FText`, return, and log (UI-3, UI-11).
    SkipRelease,
    /// Return without calling or logging (a null `this` on a virtual).
    Silent,
}

/// A guard's verdict and what it found null.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    /// What to do.
    pub verdict: Verdict,
    /// For the log line.
    pub nulls: Nulls,
}

const fn decide(verdict: Verdict, nulls: Nulls) -> Decision {
    Decision { verdict, nulls }
}

const NEXT: Decision = decide(Verdict::Next, Nulls::Members(0));

/// The members of `wanted` that are null or unreadable on `widget`.
pub fn null_members(memory: &dyn Memory, widget: usize, wanted: u16) -> u16 {
    MEMBERS
        .iter()
        .enumerate()
        .filter(|(index, _)| wanted & (1 << index) != 0)
        .filter(|(_, member)| {
            widget
                .checked_add(member.offset)
                .and_then(|at| memory.ptr_at(at))
                .is_none_or(|value| value == 0)
        })
        .fold(0, |mask, (index, _)| mask | (1 << index))
}

/// Next when every member of `wanted` is bound, else `otherwise` naming the null ones.
fn members(memory: &dyn Memory, widget: usize, wanted: u16, otherwise: Verdict) -> Decision {
    match null_members(memory, widget, wanted) {
        0 => NEXT,
        nulls => decide(otherwise, Nulls::Members(nulls)),
    }
}

/// UI-1 (section 4): `HoldText` bound and readable.
pub fn hide_hold_text(memory: &dyn Memory, widget: usize) -> Decision {
    if widget == 0 || !memory.readable(widget, r6::CALLOUT_HOLD_TEXT + 8) {
        return decide(Verdict::Skip, Nulls::This);
    }
    let hold = widget
        .checked_add(r6::CALLOUT_HOLD_TEXT)
        .and_then(|at| memory.ptr_at(at))
        .unwrap_or(0);
    if hold == 0 || !memory.readable(hold, 8) {
        return decide(Verdict::Skip, Nulls::Members(m::HOLD_TEXT));
    }
    NEXT
}

/// UI-2's reading of the screen (section 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fade {
    /// Both fade widgets bound: the original runs.
    pub run: bool,
    /// `+0x3E8` (0 when unread).
    pub primary: usize,
    /// `+0x3E0` (0 when unread).
    pub secondary: usize,
}

/// UI-2: the original runs only with both fade widgets bound and readable.
pub fn screen_fade_in(memory: &dyn Memory, screen: usize) -> Fade {
    let mut fade = Fade {
        run: false,
        primary: 0,
        secondary: 0,
    };
    if screen == 0 || !memory.readable(screen, r6::SCREEN_SIZE) {
        return fade;
    }
    let read = |offset: usize| screen.checked_add(offset).and_then(|at| memory.ptr_at(at)).unwrap_or(0);
    fade.primary = read(r6::SCREEN_PRIMARY);
    fade.secondary = read(r6::SCREEN_SECONDARY);
    fade.run = [fade.primary, fade.secondary]
        .into_iter()
        .all(|widget| widget != 0 && memory.readable(widget, 8));
    fade
}

/// UI-3 (section 6): all three members `OnSetDisabled` dereferences.
pub fn on_set_disabled(memory: &dyn Memory, widget: usize) -> Decision {
    if widget == 0 {
        return decide(Verdict::SkipRelease, Nulls::This);
    }
    members(memory, widget, m::HINT | m::REASON | m::ICON, Verdict::SkipRelease)
}

/// UI-4 (7.2). `weak` resolves the cached controller, `owning` is `GetOwningPlayer`, `is_controller` the class check.
pub fn refresh_input_keys(
    memory: &dyn Memory,
    widget: usize,
    weak: impl FnOnce(i32, i32) -> Option<usize>,
    owning: impl FnOnce() -> Option<usize>,
    is_controller: impl FnOnce(usize) -> bool,
) -> Decision {
    if widget == 0 {
        return decide(Verdict::Skip, Nulls::This);
    }
    let keys = members(memory, widget, m::KEYBOARD | m::GAMEPAD, Verdict::Skip);
    if keys.verdict != Verdict::Next {
        return keys;
    }
    let at = |offset: usize| widget.checked_add(r6::CALLOUT_OWNER_WEAK + offset);
    let cached = match (
        at(0).and_then(|at| memory.i32_at(at)),
        at(4).and_then(|at| memory.i32_at(at)),
    ) {
        (Some(index), Some(serial)) => weak(index, serial),
        _ => None,
    };
    // The function's own cast fails on another class and it never reads PlayerInput then
    let controller = cached.or_else(|| owning().filter(|&pc| pc != 0 && is_controller(pc)));
    let Some(controller) = controller.filter(|&pc| pc != 0) else {
        return NEXT;
    };
    let input = controller
        .checked_add(r6::PC_PLAYER_INPUT)
        .and_then(|at| memory.ptr_at(at))
        .unwrap_or(0);
    if input != 0 {
        NEXT
    } else {
        decide(Verdict::Skip, Nulls::PlayerInput)
    }
}

/// UI-5 (7.3).
pub fn native_construct(memory: &dyn Memory, widget: usize) -> Decision {
    if widget == 0 {
        return decide(Verdict::Silent, Nulls::This);
    }
    members(memory, widget, m::RADIAL, Verdict::ParentOnly)
}

/// UI-6 (7.4).
pub fn native_on_input_swap(memory: &dyn Memory, widget: usize) -> Decision {
    if widget == 0 {
        return decide(Verdict::Silent, Nulls::This);
    }
    members(memory, widget, m::KEYBOARD | m::GAMEPAD, Verdict::ParentOnly)
}

/// UI-7 (7.5).
pub fn native_tick(memory: &dyn Memory, widget: usize) -> Decision {
    if widget == 0 {
        return decide(Verdict::Silent, Nulls::This);
    }
    members(memory, widget, m::RADIAL | m::PROGRESS, Verdict::ParentOnly)
}

/// UI-8 and UI-9 (7.6).
pub fn fill_progress(memory: &dyn Memory, widget: usize) -> Decision {
    if widget == 0 {
        return decide(Verdict::Skip, Nulls::This);
    }
    members(memory, widget, m::RADIAL | m::FILL, Verdict::Skip)
}

/// UI-10 (7.7).
pub fn show_disabled_icon(memory: &dyn Memory, widget: usize) -> Decision {
    if widget == 0 {
        return decide(Verdict::Skip, Nulls::This);
    }
    members(memory, widget, m::ICON, Verdict::Skip)
}

/// UI-11 (7.8): `DisabledReason` always, `InteractionHint` and `DisabledIcon` when `mark_as_enabled`.
pub fn show_disabled_text(memory: &dyn Memory, widget: usize, mark_as_enabled: bool) -> Decision {
    if widget == 0 {
        return decide(Verdict::SkipRelease, Nulls::This);
    }
    let nulls = null_members(memory, widget, m::REASON | m::HINT | m::ICON);
    let needed = if mark_as_enabled {
        m::REASON | m::HINT | m::ICON
    } else {
        m::REASON
    };
    if nulls & needed == 0 {
        NEXT
    } else {
        decide(Verdict::SkipRelease, Nulls::Members(nulls))
    }
}

/// UI-12's skip reasons (section 8).
pub mod refresh {
    /// The active slot is out of range or its data is missing.
    pub const NO_SLOT: &str = "no valid active loadout slot";
    /// A getter returned null.
    pub const NULL_GETTER: &str = "an equipped-item getter returned null";
}

/// UI-12's verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refresh {
    /// `Some(reason)` to skip.
    pub skip: Option<&'static str>,
    /// The active slot read, `-2` when none was.
    pub slot: i32,
}

/// UI-12 (section 8). `is_controller` is the class check; `getter(i, loadout)` calls `r6::GETTERS[i]`.
pub fn equipped_refresh(
    memory: &dyn Memory,
    inventory: usize,
    is_controller: impl FnOnce(usize) -> bool,
    mut getter: impl FnMut(usize, usize) -> usize,
) -> Refresh {
    let mut verdict = Refresh { skip: None, slot: -2 };
    let owner = (inventory != 0 && memory.readable(inventory, r6::INVENTORY_SIZE))
        .then(|| {
            inventory
                .checked_add(r6::INVENTORY_OWNER)
                .and_then(|at| memory.ptr_at(at))
        })
        .flatten()
        .unwrap_or(0);
    if owner == 0 || !memory.readable(owner, r6::PC_SIZE) || !is_controller(owner) {
        return verdict;
    }
    let loadout = owner
        .checked_add(r6::PC_LOADOUT)
        .and_then(|at| memory.ptr_at(at))
        .unwrap_or(0);
    if loadout == 0 || !memory.readable(loadout, r6::LOADOUT_SIZE) {
        return verdict;
    }
    let read_i32 = |offset: usize| loadout.checked_add(offset).and_then(|at| memory.i32_at(at));
    let (Some(slot), Some(num), Some(data)) = (
        read_i32(r6::LOADOUT_ACTIVE),
        read_i32(r6::LOADOUT_SLOTS + 8),
        loadout.checked_add(r6::LOADOUT_SLOTS).and_then(|at| memory.ptr_at(at)),
    ) else {
        return verdict;
    };
    verdict.slot = slot;
    let slot_data = (0..num)
        .contains(&slot)
        .then(|| usize::try_from(slot).ok())
        .flatten()
        .and_then(|slot| slot.checked_mul(8))
        .and_then(|offset| data.checked_add(offset))
        .filter(|&at| memory.readable(at, 8))
        .and_then(|at| memory.ptr_at(at))
        .unwrap_or(0);
    if slot_data == 0 {
        verdict.skip = Some(refresh::NO_SLOT);
    } else if (0..r6::GETTERS.len()).any(|index| getter(index, loadout) == 0) {
        verdict.skip = Some(refresh::NULL_GETTER);
    }
    verdict
}

/// The executable's `SizeOfImage` (the optional header's +56).
pub fn image_size(memory: &dyn Memory, base: usize) -> Option<usize> {
    if memory.u16_at(base)? != u16::from_le_bytes(*b"MZ") {
        return None;
    }
    let lfanew = usize::try_from(memory.i32_at(base.checked_add(0x3C)?)?).ok()?;
    let nt = base.checked_add(lfanew)?;
    if memory.u32_at(nt)? != u32::from_le_bytes(*b"PE\0\0") {
        return None;
    }
    usize::try_from(memory.u32_at(nt.checked_add(24 + 56)?)?).ok()
}

/// Whether `address` lies in `[base, base + size)`: the only game code a guard calls through a vtable (3.6).
pub fn in_image(address: usize, base: usize, size: usize) -> bool {
    address >= base && base.checked_add(size).is_some_and(|end| address < end)
}

/// The `FText` reference controller's operations (3.5); the DLL's are interlocked and call game code.
pub trait TextRefs {
    /// An interlocked decrement of the `i32` at `address`: the new value.
    fn decrement(&self, address: usize) -> Option<i32>;
    /// Vtable entry 0 as `(controller)`: destroys the object.
    fn destroy(&self, entry: usize, controller: usize);
    /// Vtable entry 1 as `(controller, 1)`: deletes the controller.
    fn delete(&self, entry: usize, controller: usize);
}

/// What `release_owned_text` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Release {
    /// Nothing to release (null or unreadable).
    Untouched,
    /// The shared count dropped and others remain.
    Decremented,
    /// The object was destroyed; weak references remain.
    Destroyed,
    /// The object was destroyed and the controller deleted.
    Deleted,
    /// An entry lay outside the image and was not called (logged once).
    Refused,
}

/// Releases an `FText` the skipped body owned (3.5). `image` is the executable's `(base, size)`.
pub fn release_owned_text(memory: &dyn Memory, refs: &dyn TextRefs, text: usize, image: (usize, usize)) -> Release {
    if text == 0 || !memory.readable(text, r6::FTEXT_SIZE) {
        return Release::Untouched;
    }
    let controller = text
        .checked_add(r6::FTEXT_CONTROLLER)
        .and_then(|at| memory.ptr_at(at))
        .unwrap_or(0);
    if controller == 0 || !memory.readable(controller, 0x10) {
        return Release::Untouched;
    }
    let Some(shared) = controller.checked_add(r6::REF_SHARED).and_then(|at| refs.decrement(at)) else {
        return Release::Untouched;
    };
    if shared != 0 {
        return Release::Decremented;
    }
    let vtable = memory.ptr_at(controller).unwrap_or(0);
    if vtable == 0 || !memory.readable(vtable, 0x10) {
        return Release::Decremented;
    }
    let entry = |slot: usize| {
        vtable
            .checked_add(slot * 8)
            .and_then(|at| memory.ptr_at(at))
            .unwrap_or(0)
    };
    let (destroy, delete) = (entry(0), entry(1));
    let inside = |entry: usize| in_image(entry, image.0, image.1);
    // A leak is preferred to a jump into unknown memory
    if destroy != 0 && !inside(destroy) {
        return Release::Refused;
    }
    if destroy != 0 {
        refs.destroy(destroy, controller);
    }
    if controller.checked_add(r6::REF_WEAK).and_then(|at| refs.decrement(at)) != Some(0) || delete == 0 {
        return Release::Destroyed;
    }
    if !inside(delete) {
        return Release::Refused;
    }
    refs.delete(delete, controller);
    Release::Deleted
}

/// The `FText` refusal line, once per process.
pub const TEXT_REFUSED_LINE: &str =
    "[WidgetGuard] FText release skipped: reference controller vtable outside the game image";

/// What verification reads of the executable (the DLL's `.pdata` and checked reads; a fake in tests).
pub trait Code {
    /// R0's `function_entry(rva)` (no chained unwind).
    fn function_entry(&self, rva: u32) -> Option<u32>;
    /// The bytes at `rva`, read unaligned.
    fn bytes(&self, rva: u32, out: &mut [u8]) -> bool;
}

/// The callee of an `E8 rel32` at `offset` in `bytes` (a function at `rva`): `rva + offset + 5 + rel32`.
pub fn super_call_target(bytes: &[u8], rva: u32, offset: usize) -> Option<u32> {
    let call = bytes.get(offset..offset.checked_add(5)?)?;
    if call[0] != 0xE8 {
        return None;
    }
    let rel = i32::from_le_bytes([call[1], call[2], call[3], call[4]]);
    let next = i64::from(rva) + i64::try_from(offset).ok()? + 5;
    u32::try_from(next + i64::from(rel)).ok()
}

fn entry_bytes(code: &dyn Code, rva: u32, expected: &[u8]) -> Result<(), &'static str> {
    if code.function_entry(rva) != Some(rva) {
        return Err(reason::NOT_ENTRY);
    }
    let mut bytes = vec![0u8; expected.len()];
    if !code.bytes(rva, &mut bytes) {
        return Err(reason::UNREADABLE);
    }
    if bytes != expected {
        return Err(reason::BYTES_DIFFER);
    }
    Ok(())
}

/// `verify(id)` (3.3): `Ok` or the install reason.
pub fn verify(code: &dyn Code, target: &Target) -> Result<(), String> {
    match target.found_by {
        FoundBy::RvaAndBytes(prologue) => entry_bytes(code, target.rva, prologue)?,
        FoundBy::PdataOf(inside) => {
            if code.function_entry(inside) != Some(target.rva) {
                return Err(reason::NOT_ENTRY.to_owned());
            }
            entry_bytes(code, target.rva, &r6::UI3_ENTRY)?;
        }
        // UI-1: a leaf with no entry of its own
        FoundBy::Rva => {
            let mut leaf = [0u8; r6::UI1_LEAF.len()];
            if !code.bytes(target.rva, &mut leaf) {
                return Err(reason::UNREADABLE.to_owned());
            }
            if leaf != r6::UI1_LEAF {
                return Err(reason::BYTES_DIFFER.to_owned());
            }
        }
        FoundBy::Export(..) | FoundBy::UFunction(..) => return Err(reason::NOT_ENTRY.to_owned()),
    }
    if let Some(&(_, offset, parent)) = r6::PARENT_CALLS.iter().find(|(id, ..)| *id == target.id) {
        let mut bytes = vec![0u8; offset + 5];
        if !code.bytes(target.rva, &mut bytes) {
            return Err(reason::UNREADABLE.to_owned());
        }
        if super_call_target(&bytes, target.rva, offset) != Some(parent) {
            return Err(reason::PARENT_DIFFERS.to_owned());
        }
        if code.function_entry(parent) != Some(parent) {
            return Err(reason::PARENT_NOT_ENTRY.to_owned());
        }
    }
    if target.id == "UI-12" {
        for getter in r6::GETTERS {
            if entry_bytes(code, getter, &r6::GETTER_BYTES).is_err() {
                return Err(getter_reason(getter));
            }
        }
    }
    Ok(())
}

/// The parent RVA a guard calls (UI-5..7).
pub fn parent_of(id: &str) -> Option<u32> {
    r6::PARENT_CALLS
        .iter()
        .find(|(parent_id, ..)| *parent_id == id)
        .map(|&(_, _, parent)| parent)
}

/// How often an id's skips are written (13.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    /// UI-1..6, UI-8..11.
    Callout,
    /// UI-7, per frame.
    Tick,
    /// UI-12.
    Inventory,
    /// UI-13.
    Hud,
}

/// Each id's limit.
pub fn limit_of(id: &str) -> Limit {
    match id {
        "UI-7" => Limit::Tick,
        "UI-12" => Limit::Inventory,
        "UI-13" => Limit::Hud,
        _ => Limit::Callout,
    }
}

/// Whether skip `n` (0-based) of an id is written.
pub fn written(limit: Limit, n: u64) -> bool {
    match limit {
        Limit::Callout => n < 8 || n.is_multiple_of(1024),
        Limit::Tick => n < 4 || n.is_multiple_of(100_000),
        Limit::Inventory => n < 20 || n.is_multiple_of(100),
        Limit::Hud => n < 8 || (n + 1).is_power_of_two(),
    }
}

/// `0x…` lowercase, `null` for 0.
pub fn pointer(value: usize) -> String {
    if value == 0 {
        "null".to_owned()
    } else {
        format!("{value:#x}")
    }
}

/// The null members as `Name(+0xoff)` joined by `,` (13.2).
pub fn nulls_text(nulls: Nulls) -> String {
    match nulls {
        Nulls::This => "this".to_owned(),
        Nulls::PlayerInput => format!("PlayerInput(pc+{:#x})", r6::PC_PLAYER_INPUT),
        Nulls::Members(0) => "none".to_owned(),
        Nulls::Members(mask) => MEMBERS
            .iter()
            .enumerate()
            .filter(|(index, _)| mask & (1 << index) != 0)
            .map(|(_, member)| match member.name {
                "" => format!("+{:#x}", member.offset),
                name => format!("{name}(+{:#x})", member.offset),
            })
            .collect::<Vec<_>>()
            .join(","),
    }
}

/// The server status line (13.2).
pub fn status_line(boot_patch_applied: bool) -> String {
    let yes = if boot_patch_applied { "yes" } else { "no" };
    format!("[WidgetGuard] server guards on (boot patch applied: {yes})")
}

/// UI-1's skip.
pub fn hold_text_line(widget: usize, nulls: Nulls, count: u64) -> String {
    format!(
        "[WidgetGuard] UI-1 skipped HideHoldText widget={} null={} count={count}",
        pointer(widget),
        nulls_text(nulls)
    )
}

/// UI-2's skip.
pub fn fade_line(screen: usize, fade: Fade, mode: u8, count: u64) -> String {
    format!(
        "[WidgetGuard] UI-2 skipped ScreenFadeIn screen={} primary(+0x3e8)={} secondary(+0x3e0)={} mode={mode} \
         count={count}",
        pointer(screen),
        pointer(fade.primary),
        pointer(fade.secondary)
    )
}

/// UI-3's skip.
pub fn disabled_line(widget: usize, nulls: Nulls, disabled: bool, count: u64) -> String {
    format!(
        "[WidgetGuard] UI-3 skipped OnSetDisabled widget={} null={} disabled={} count={count} (owned text released)",
        pointer(widget),
        nulls_text(nulls),
        u8::from(disabled)
    )
}

/// Each callout id's site name (13.2).
pub fn site(id: &str) -> &'static str {
    match id {
        "UI-4" => "RefreshInputKeys",
        "UI-5" => "NativeConstruct",
        "UI-6" => "NativeOnInputSwap",
        "UI-7" => "NativeTick",
        "UI-8" => "OnShowFillProgress",
        "UI-9" => "OnHideFillProgress",
        "UI-10" => "OnSetShowDisabledIconWhenEnabled",
        "UI-11" => "OnSetShowDisabledTextWhenEnabled",
        _ => "",
    }
}

/// A UI-4..11 line.
pub fn callout_line(id: &str, widget: usize, decision: Decision, count: u64) -> String {
    let action = match decision.verdict {
        Verdict::ParentOnly => "parent-only",
        Verdict::SkipRelease => "skipped+text-released",
        Verdict::Next | Verdict::Skip | Verdict::Silent => "skipped",
    };
    format!(
        "[WidgetGuard] {id} {} widget={} null={} action={action} count={count}",
        site(id),
        pointer(widget),
        nulls_text(decision.nulls)
    )
}

/// UI-12's skip.
pub fn refresh_line(n: u64, why: &str, slot: i32, inventory: usize) -> String {
    format!(
        "[WidgetGuard] UI-12 skipped equipped-item refresh #{n}: {why} (activeSlot={slot} inventory={})",
        pointer(inventory)
    )
}

/// UI-13's skip.
pub fn weapon_line(widget: usize, n: u64) -> String {
    format!(
        "[WidgetGuard] UI-13 skipped HandleWeaponEquipped(weapon=null) widget={} n={n} (runs again from the \
         character's OnWeaponEquipped)",
        pointer(widget)
    )
}

#[cfg(test)]
mod tests;
