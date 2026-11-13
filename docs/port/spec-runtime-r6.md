<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: runtime slice R6, widget crash guards (RD27, plan item 11)

R6 ports the thirteen crash guards a 1.14.7 game server (and, for one of them, the client) needs because UI code
dereferences sub-widgets that a headless process never binds: UI-1 to UI-13 (`spec-runtime-surface.md` sections 3.6,
3.11 and 8; `spec-runtime-r0.md` section 7.5). Every guard is a MinHook detour through R0's broker that calls the
game's function when the members it reads are bound, and otherwise skips it (or runs only its parent's part). R6 adds
no dispatch handler and no feature id.

An implementer builds from this page, `spec-runtime-surface.md`, `spec-runtime-r0.md`, `spec-runtime-r1.md` and
`spec-runtime-r5.md` alone, and never reads `restricted/`. What crossed is facts: RVAs into
`Dauntless-Win64-Shipping.exe` (1.14.7, CL 647472), prologue bytes (each one re-read from the installed executable for
this page), UE and game class and member names with their offsets, flag names, and behaviour. The AGPL side (seam patch
0041; R6 needs no patch of its own) is described only by the contract it keeps (section 9).

Conventions are R0's: an RVA is `+0x0XXXXXXX` from the executable's image base; **S** server, **C** client.
**(call)** marks a decision this page makes, **(departure)** a deliberate difference from the AGPL runtime,
**(ambiguity)** something the source did not settle; all are collected in section 12.

---

## 1. Scope

| Id | Role | Target | Guard | Section |
| --- | --- | --- | --- | --- |
| UI-1 | S | `UInteractionCalloutWidget::HideHoldText` | skip when `HoldText` is unbound | 4 |
| UI-2 | S | `UArchonLoadingScreen::ScreenFadeIn` | skip when either fade widget is unbound | 5 |
| UI-3 | S | `UInteractionCalloutWidget::OnSetDisabled` | skip (releasing the owned `FText`) when any of three members is unbound | 6 |
| UI-4 | S | `UInteractionCalloutWidget` input-key refresh | skip when `Keyboard`, `Gamepad` or the owning controller's `PlayerInput` is unbound | 7.2 |
| UI-5 | S | `UInteractionCalloutWidget::NativeConstruct` | parent only when `RadialAnimImage` is unbound | 7.3 |
| UI-6 | S | `UInteractionCalloutWidget::NativeOnInputSwap` | parent only when `Keyboard` or `Gamepad` is unbound | 7.4 |
| UI-7 | S | `UInteractionCalloutWidget::NativeTick` | parent only when `RadialAnimImage` or `ProgressMaterial` is unbound | 7.5 |
| UI-8 | S | `UInteractionCalloutWidget::OnShowFillProgress` | skip when `RadialAnimImage` or the member at `+0x3A0` is unbound | 7.6 |
| UI-9 | S | `UInteractionCalloutWidget::OnHideFillProgress` | same as UI-8 | 7.6 |
| UI-10 | S | `UInteractionCalloutWidget::OnSetShowDisabledIconWhenEnabled` | skip when `DisabledIcon` is unbound | 7.7 |
| UI-11 | S | `UInteractionCalloutWidget::OnSetShowDisabledTextWhenEnabled` | skip (releasing the owned `FText`) when the members it will use are unbound | 7.8 |
| UI-12 | S | `AArchonInventory`'s equipped-items refresh | skip while the joining player's active loadout slot or an equipped item is missing | 8 |
| UI-13 | C | `UHUDLegendaryAbilityWidget::HandleWeaponEquipped` | skip a null weapon | 10 |

UI-4 to UI-6 install as one all-or-nothing group (7.1); UI-7 to UI-11 each on their own. Flags (RT-14 for R6, 12.1):
`DISABLE_CALLOUT_GUARDS.flag` turns off UI-4 to UI-11, `DISABLE_INVENTORY_REFRESH_GUARD.flag` UI-12,
`DISABLE_HUD_LEGENDARY_GUARD.flag` UI-13. UI-1 to UI-3 have no switch.

Not R6: the phantom local player that made several of these paths reachable on a server (R5's HL-2); the loading
screen's own state machine; anything a dispatch point does.

**Why R6 depends on R5.** Servers build these widgets without the sub-widgets they bind (no viewport, `-nullrhi`), and
before R5's HL-2 a server also had a phantom local player whose screen-space widget components drove UI-4's path every
frame. R6's guards are the second line behind HL-2: they must hold whichever DLL owns HL-2, and the play-test (11.2)
assumes R5 has landed. **The R5 interface R6 consumes** is `headless::boot_patch_applied() -> bool`
(`spec-runtime-r5.md` 2.2), read once for R6's status line (13.2), so a reader of the log can tell a guard firing on a
server with the phantom from one firing without it. R6 calls nothing else of R5's, and orders its install after
`headless::install` in Init (2).

---

## 2. Where it lives

| File | Kind | Holds |
| --- | --- | --- |
| `crates/rustic-isles-runtime-core/src/widgets.rs` (new) | pure, every platform | the member tables and every guard's decision (7, 8) as functions over a member reader; the rate-limit rule (13.3); the install reason texts and log lines (13); the super-call check (3.3) over a byte slice; the `offsets::r6` facts' use |
| `crates/rustic-isles-runtime-core/src/weak.rs` (new) | pure, every platform | `resolve_weak(engine, index, serial) -> Option<usize>` (3.4), general, no R6 name in it |
| `crates/rustic-isles-runtime/src/widgets.rs` (new, safe) | Windows | `install(base, server, minhook_ok)`, called from Init; the read-once flags; the deferred install lines |
| `crates/rustic-isles-runtime/src/ffi/widgets.rs` (new, under `ffi`) | Windows, unsafe | the thirteen detour bodies, the calls to the parent bodies and the five getters, the `FText` release (3.5), the checked virtual calls (3.6) |

Lines R6 adds to files it does not own (each append-only, so slices landing beside it merge cleanly):

- `registry.rs`: `"R6"` in `LANDED`. UI-1..13 are already in `ENTRIES` as `Detour`, slice `R6`.
- `offsets.rs`: the thirteen `TARGETS` lines of 3.1 and a module `pub mod r6 { ... }` with the facts of 3.2.
- `lifecycle.rs`: one call, `widgets::install(base, server, initialized == minhook::MH_OK)`, after the dispatch
  installs (both roles) and, on a server, after R5's `headless::install`. R6 registers no handler, so its place relative
  to `handlers::freeze()` does not matter.
- `ffi/mod.rs`: `pub mod widgets;`. `lib.rs`: `pub mod widgets;` beside R5's. Core `lib.rs`: `pub mod widgets;
  pub mod weak;`.
- **The reflection kit.** UI-4 and UI-12 need "is this object an `ArchonPlayerController`". R6 uses the shared kit's
  `is_a(object, class_name)` (`spec-runtime-r17.md` 3.2; R15 7.2 states the same rule). **(call)** If neither R15 nor
  R17 has put `reflect.rs` on `plan` when R6 is implemented, R6 creates it with `class_chain` and `is_a` exactly as R17
  3.2 states and nothing else, as the kit's first author (R17 section 2's rule); the later slice only adds to it.

What CI tests: all of `widgets.rs` and `weak.rs` in the core crate (14.1), and the guards on synthetic widgets and test
functions in the DLL crate on the Windows runner. What only the play-test checks: every RVA, prologue and member offset
against the game.

---

## 3. Facts

### 3.1 `TARGETS` lines (RT-2)

| id | rva | found_by | what |
| --- | --- | --- | --- |
| UI-1 | `0x01E4_2F00` | `Rva` (the leaf bytes of 3.2 are checked by R6, not by `RvaAndBytes`: this function has no `.pdata` entry) | `UInteractionCalloutWidget::HideHoldText` |
| UI-2 | `0x01E0_FBB0` | `RvaAndBytes(&[0x48, 0x89, 0x5C, 0x24, 0x10, 0x57, 0x48, 0x83, 0xEC, 0x70, 0x48, 0x8B, 0xD9, 0x88, 0x91, 0xF0, 0x03, 0x00, 0x00])` | `UArchonLoadingScreen::ScreenFadeIn` |
| UI-3 | `0x01E5_4110` | `PdataOf(0x01E5_414E)` | `UInteractionCalloutWidget::OnSetDisabled` |
| UI-4 | `0x01E3_EB10` | `RvaAndBytes(&[0x40, 0x53, 0x57, 0x41, 0x57, 0x48, 0x83, 0xEC, 0x70, 0x4C, 0x8B, 0xF9, 0x48, 0x8B, 0x89, 0x88, 0x03, 0x00, 0x00])` | `UInteractionCalloutWidget` input-key refresh |
| UI-5 | `0x01E4_BA00` | `RvaAndBytes(&[0x40, 0x53, 0x48, 0x83, 0xEC, 0x20, 0x48, 0x8B, 0xD9, 0xE8, 0x72, 0xEA, 0xFF, 0xFF, 0x48, 0x8B, 0x8B, 0x98, 0x03, 0x00, 0x00])` | `UInteractionCalloutWidget::NativeConstruct` |
| UI-6 | `0x01E4_E050` | `RvaAndBytes(&[0x48, 0x89, 0x5C, 0x24, 0x08, 0x48, 0x89, 0x6C, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xEC, 0x20, 0x0F, 0xB6, 0xF2])` | `UInteractionCalloutWidget::NativeOnInputSwap` |
| UI-7 | `0x01E5_0A70` | `RvaAndBytes(&[0x48, 0x89, 0x5C, 0x24, 0x10, 0x57, 0x48, 0x83, 0xEC, 0x70, 0x48, 0x8B, 0xD9, 0xE8, 0x0E, 0xF9, 0xFF, 0xFF])` | `UInteractionCalloutWidget::NativeTick` |
| UI-8 | `0x01E5_44B0` | `RvaAndBytes(&[0x40, 0x53, 0x48, 0x83, 0xEC, 0x30, 0x0F, 0x29, 0x74, 0x24, 0x20, 0x48, 0x8B, 0xD9])` | `UInteractionCalloutWidget::OnShowFillProgress` |
| UI-9 | `0x01E5_2450` | `RvaAndBytes(&[0x48, 0x89, 0x5C, 0x24, 0x10, 0x57, 0x48, 0x83, 0xEC, 0x20, 0x48, 0x8B, 0xD9])` | `UInteractionCalloutWidget::OnHideFillProgress` |
| UI-10 | `0x01E5_42C0` | `RvaAndBytes(&[0x48, 0x89, 0x5C, 0x24, 0x08, 0x57, 0x48, 0x83, 0xEC, 0x20, 0x48, 0x8B, 0xFA, 0x48, 0x8B, 0xD9, 0x48, 0x8B, 0x89, 0xB8, 0x03, 0x00, 0x00])` | `UInteractionCalloutWidget::OnSetShowDisabledIconWhenEnabled` |
| UI-11 | `0x01E5_4320` | `RvaAndBytes(&[0x48, 0x89, 0x5C, 0x24, 0x10, 0x48, 0x89, 0x6C, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57, 0x48, 0x83, 0xEC, 0x50])` | `UInteractionCalloutWidget::OnSetShowDisabledTextWhenEnabled` |
| UI-12 | `0x01B8_6160` | `RvaAndBytes(&[0x40, 0x53, 0x57, 0x48, 0x83, 0xEC, 0x78, 0x48, 0x8B, 0x99, 0xE0, 0x00, 0x00, 0x00])` | `AArchonInventory` equipped-items refresh |
| UI-13 | `0x01F6_D4F0` | `RvaAndBytes(&[0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x48, 0x8D, 0xAC, 0x24, 0xB0, 0xFE, 0xFF, 0xFF, 0x48, 0x81, 0xEC, 0x50, 0x02, 0x00, 0x00])` | `UHUDLegendaryAbilityWidget::HandleWeaponEquipped` |

**(call)** UI-2 and UI-12 gain a byte check the AGPL does not make (it hooks UI-2 by RVA alone and UI-12 after a
`.pdata` check only), and UI-1 a byte check of its leaf; UI-3 also checks that the resolved entry is `+0x01E54110` with
the bytes of 3.2. Every R6 target fails closed: a mismatch installs nothing and says why (13.1).

### 3.2 `offsets::r6`

Code checks that are not `TARGETS` lines:

| Name | Value |
| --- | --- |
| UI-1 leaf bytes (no `.pdata` entry: a frameless tail call) | `48 8B 89 C0 03 00 00 B2 02 48 8B 01 48 FF A0 88 02 00 00`, checked with R0's unaligned read at the RVA |
| UI-3 entry bytes, at `+0x01E54110` | `48 89 5C 24 10 48 89 6C 24 18 56 57 41 56 48 83 EC 40 88 91 E8 03 00 00` |
| UI-5's parent call | at entry `+0x09`: `E8` with target `+0x01E4A480` (`UArchonUserWidgetBase::NativeConstruct`) |
| UI-6's parent call | at entry `+0x1A`: `E8` with target `+0x01E4DEE0` (`UArchonUserWidgetBase::NativeOnInputSwap`) |
| UI-7's parent call | at entry `+0x0D`: `E8` with target `+0x01E50390` (the parent `NativeTick`) |
| UI-12's five equipped-item getters | `+0x01BA4FF0`, `+0x01B9CA60`, `+0x01B9FF30`, `+0x01B9A620`, `+0x01BA08D0`; each a `.pdata` entry starting `48 83 EC 28 E8` |

`UInteractionCalloutWidget` members (each a pointer; "bound" means non-null):

| Offset | Member |
| --- | --- |
| `+0x318` | the cached owning player controller, an `FWeakObjectPtr` (object index `i32 +0x318`, serial `i32 +0x31C`) |
| `+0x388` | `Keyboard` |
| `+0x390` | `Gamepad` |
| `+0x398` | `RadialAnimImage` |
| `+0x3A0` | an unnamed member the fill-progress natives use (written `+0x3A0` in logs) |
| `+0x3A8` | `InteractionHint` |
| `+0x3B0` | `DisabledReason` |
| `+0x3B8` | `DisabledIcon` |
| `+0x3C0` | `HoldText` |
| `+0x3E0` | `ProgressMaterial` (written by `NativeConstruct` from `RadialAnimImage`'s dynamic material) |

Other layout facts:

| Type | Fact | Value |
| --- | --- | --- |
| `UArchonLoadingScreen` | fade widgets | primary `+0x3E8`, secondary `+0x3E0` (pointers); readable size checked `0x3F0` |
| `UObject` | vtable `+0x00`; `GetOwningPlayer` (a `UUserWidget` virtual) | byte offset `+0x2A0` in the vtable, signature `(widget) -> APlayerController*` |
| `APlayerController` | `PlayerInput` | `+0x348` (pointer) |
| `AArchonInventory` | owner | `+0xE0` (pointer); readable size checked `0xE8` |
| `AArchonPlayerController` | `LOADOUT` | `+0x9C8` (`AArchonLoadout*`, the fact R15 7.1 also records); readable size checked `0x9D0` |
| `AArchonLoadout` | `LoadoutSlotDataArray` | `+0x670` (`TArray<UArchonLoadoutSlotData*>`: data `+0x670`, `Num` `i32 +0x678`) |
| `AArchonLoadout` | `ActiveLoadoutSlotIndex` | `+0x680` (`i32`, -1 until the loadout picks its slot); readable size checked `0x690` |
| `FText` | size and shared reference | `0x18` bytes; the reference controller pointer at `+0x08` |
| The reference controller | counts and vtable | vtable `+0x00`; shared count `i32 +0x08`; weak count `i32 +0x0C`; vtable entry 0 destroys the object (`(controller)`), entry 1 deletes the controller (`(controller, 1)`) |
| `FUObjectItem` (R0's object array item, `0x18` bytes) | flags, serial | internal flags `i32 +0x08`, serial number `i32 +0x10`; `0x30000000` = unreachable or pending kill |

Class names R6 matches with `is_a`: `ArchonPlayerController` (short name; its Blueprint subclasses match through the
super chain).

### 3.3 Verifying a target

`verify(id)` before any hook, by `found_by`:

- `RvaAndBytes(b)`: R0's `verify_entry(rva, b)` (a `.pdata` function entry at `rva` whose first bytes are `b`).
- `PdataOf(x)` (UI-3): `function_entry(x)` is `0x01E54110` and its bytes are those of 3.2.
- UI-1: the 19 leaf bytes of 3.2 read with R0's unaligned read at the RVA (no entry check: the function has none).
- **Parent calls** (UI-5, UI-6, UI-7): the byte at entry `+o` is `0xE8`; `rva + o + 5 + rel32` (the next four bytes,
  little-endian, signed) equals the parent's RVA; and the parent is itself a `.pdata` function entry. The arithmetic
  is pure (`super_call_target(bytes, rva, o)`) and tested on the bytes of 3.1.
- UI-12: also each getter of 3.2 is a function entry starting `48 83 EC 28 E8`.

The result is `ok` or one reason of 13.1. **(call)** R0's `function_entry` (no chained unwind) is the lookup every R6
check uses, as the AGPL's does.

### 3.4 Resolving a weak object pointer (pure)

`resolve_weak(engine, index, serial)`, the 1.14.7 engine's own `FWeakObjectPtr::Get` behaviour (its body is the
frameless function at `+0x0287AA20`; R6 does not call it): `None` when `serial == 0`, `index < 0`, or `index >=` the
object array's element count (R0's `OBJECTS_COUNT`); otherwise the item at `index` (R0's chunked lookup); `None` when
the item cannot be read, its serial (`+0x10`) differs from `serial`, or its flags (`+0x08`) have any bit of
`0x30000000`; else the item's object pointer (`+0x00`). **(call)** Re-implemented over R0's checked reads instead of
calling the game function, so it is CI-tested and never faults.

### 3.5 Releasing an owned `FText` (UI-3, UI-11)

`OnSetDisabled` and `OnSetShowDisabledTextWhenEnabled` own the `FText` they are passed: their bodies release its
shared reference before returning. A guard that skips the body must do that release itself, or every skip with a
reason text leaks a reference. `release_owned_text(text)`:

1. `text` null, or not readable for `0x18`: do nothing.
2. `controller` = the pointer at `text + 0x08`; null, or not readable for `0x10`: do nothing.
3. Atomically decrement the shared count (`controller + 0x08`, a 32-bit interlocked decrement). If the new value is
   not 0, stop.
4. Read the controller's vtable (`+0x00`), readable for `0x10`; call entry 0 as `(controller)` if it is non-null.
5. Atomically decrement the weak count (`controller + 0x0C`); if the new value is 0 and entry 1 is non-null, call
   entry 1 as `(controller, 1)`.

**(call)** Before each call in steps 4 and 5 the entry must lie inside the executable's image (base to base +
`SizeOfImage`, R0's PE parse); an entry outside it is not called (a leak is preferred to a jump into unknown memory)
and is logged once per process: `[WidgetGuard] FText release skipped: reference controller vtable outside the game
image`. The pass-through path never touches the text.

### 3.6 Calling game code from a guard

R6's guards call four kinds of game code, all on the thread that called the guard:

- the hooked function itself, through the broker's `next` for that target;
- a parent body (UI-5, UI-6, UI-7) at `base + rva` from 3.2, directly (it is not hooked; calling it by name would
  dispatch virtually back into the guard);
- UI-12's five getters at `base + rva`, each `(loadout) -> pointer`;
- UI-4's virtual `GetOwningPlayer` (3.2) and the `FText` controller's entries (3.5): **(call)** the vtable and its entry
  are read with R0's checked reads, and the entry must lie in the executable's image, or the call is not made.

Signatures (Win64; the `extern "C"` the broker already uses):

| Id | Signature |
| --- | --- |
| UI-1, UI-4, UI-5, UI-9 | `(widget)` |
| UI-2 | `(screen, u8 mode)` |
| UI-3 | `(widget, bool disabled, *const FText reason, texture)` |
| UI-6 | `(widget, bool using_gamepad)` |
| UI-7 | `(widget, *const FGeometry geometry, f32 delta)` (the parent has the same) |
| UI-8 | `(widget, f32 duration)` |
| UI-10 | `(widget, texture)` |
| UI-11 | `(widget, *const FText text, f32 vertical_offset, bool mark_as_enabled)` |
| UI-12 | `(inventory, updated_items)` |
| UI-13 | `(widget, weapon)` |

All return nothing.

---

## 4. UI-1: `HideHoldText` (S)

`HideHoldText` reads `HoldText` (`+0x3C0`) and calls a virtual on it without a check. On a server Ramsgate boots
through `UAirshipSlotWidget::InitSlot`, which reaches it with `HoldText` unbound; the fault ends the server through the
engine's unhandled-exception path (exit code 3).

Guard: if the widget is not readable for `0x3C8`, skip; else if the pointer at `+0x3C0` is null or not readable for 8,
skip; else call `next(widget)`. Every skip logs (13.2, rate-limited by 13.3; **(departure)** the AGPL logs every
skip).

## 5. UI-2: `ScreenFadeIn` (S)

The 1.14.7 function at `+0x01E0FBB0` stores its mode byte at `+0x3F0` and then, on every path, calls a virtual on the
primary fade widget (`+0x3E8`; the load at `+0x01E0FC2B`), and on most paths on the secondary (`+0x3E0`). A server
reaches it on a failed player load with both unbound. On a client both are bound and the original must run, or the
loading screen never finishes fading (its later "hide" transitions wait on this one).

Guard: if the screen is readable for `0x3F0`, read both pointers; the original runs only when the screen is non-null
and both pointers are non-null and readable for 8. Otherwise skip and log (13.2). **Server only**: the AGPL compiles
it out on the client, and R6 installs nothing for UI-2 there. **(departure)** The AGPL's "pass-through" line (the first
call per mode) is a diagnostic and is not ported (RT-3).

## 6. UI-3: `OnSetDisabled` (S)

`OnSetDisabled(disabled, reason, icon)` stores `disabled` at `+0x3E8` and then dereferences, on every path,
`InteractionHint` (`+0x3A8`, the first fault, `+0x01E5414E`), `DisabledReason` (`+0x3B0`) and `DisabledIcon` (`+0x3B8`).
On the Training Grounds hub it faulted about 80 s after every spawn, which made the Director restart the hub in a loop.

Guard:

1. Widget null: release the `FText` (3.5), log, return.
2. All three members readable and non-null: call `next` with all four arguments unchanged. Done.
3. Otherwise (any member null, or the widget unreadable): release the `FText`, log the null members (13.2), return.

**(ambiguity, settled)** The surface page says UI-3 checks `InteractionHint` alone; the body needs all three, and the
AGPL guard checks all three (testing one left two faults). R6 checks three.

---

## 7. UI-4 to UI-11: the callout widget class (S)

### 7.1 Install

With `DISABLE_CALLOUT_GUARDS.flag` (read once, in Init) present, none of UI-4..11 is installed (13.1).

**The input group, UI-4 to UI-6, all or nothing.** Guarding one of them moves the fault to the next: the measured hunt
crash was in UI-4 (`+0x01E3EB23`, `Keyboard`); guarding UI-4 alone moved it to UI-6 (`Gamepad`, reached from the parent
`NativeConstruct`) and then to UI-5 (`RadialAnimImage`'s dynamic material). So:

1. Verify all three (3.3, with UI-5's and UI-6's parent calls). One fails: install none; each of the three logs
   `group aborted (<id>: <reason>)`.
2. Hook them in order UI-4, UI-5, UI-6 through `broker::hook` (`order = 0`). One fails: `unhook` the ones already
   added (the broker keeps its MinHook detour with an empty chain, which runs the game's function: the effect of no
   guard) and log the failing status for that id and `group aborted (<id>: <status>)` for the others. An `unhook` that
   fails is logged with its status: a half-installed group is exactly what moves the crash, so it must be visible.
3. All three hooked: `ok` for each.

**UI-7 to UI-11 each on their own:** verify (UI-7 with its parent call), hook, log.

**(departure)** Every UI-4..11 detour in the AGPL also asks whether the process is a server and calls the original if
not. They are installed on servers only, where the answer is always yes, so R6 drops the question.

### 7.2 UI-4: the input-key refresh

Reached from `NativePreConstruct`'s tail jump, `OnHighlightInteraction`, and on hubs `UAirshipSlotWidget::SelectSlot`.
It reads `Keyboard` and `Gamepad` unconditionally, then hands the owning controller's `PlayerInput` (`+0x348`) to a
helper that does not check it.

1. Widget null: skip, log `null=this`.
2. `Keyboard` or `Gamepad` null (or unreadable): skip, log the null ones.
3. Find the controller the function will use, in its own order: `resolve_weak` on the weak pointer at `+0x318`; if that
   is `None`, call `GetOwningPlayer` (3.6) and, when the result is non-null but not `is_a` `ArchonPlayerController`,
   treat it as none (the function's own cast fails there and it never reads `PlayerInput`).
4. No controller, or its `PlayerInput` non-null: call `next(widget)`.
5. Otherwise skip, log `null=PlayerInput(pc+0x348)`.

### 7.3 UI-5: `NativeConstruct`

Calls the parent, then takes `RadialAnimImage`'s dynamic material unchecked and stores it in `ProgressMaterial`.

- Widget null: return (a virtual call never passes null; nothing is logged).
- `RadialAnimImage` non-null: call `next`.
- Otherwise call the parent (`+0x01E4A480`) with the widget, skip the rest, and log `action=parent-only`. The parent
  keeps the Blueprint `Construct` and the input-swap delegate registration.

### 7.4 UI-6: `NativeOnInputSwap`

Calls the parent, then sets `Gamepad`'s and `Keyboard`'s visibility unchecked. Reached virtually and, by name, when the
controller's `OnInputSwap` delegate broadcasts.

- Widget null: return.
- `Keyboard` and `Gamepad` non-null: call `next(widget, using_gamepad)`.
- Otherwise the parent (`+0x01E4DEE0`) with the same two arguments, then log `action=parent-only` with the null ones.
  The parent keeps the cached gamepad flag.

### 7.5 UI-7: `NativeTick`

Per frame for every callout widget.

- Widget null: return.
- `RadialAnimImage` and `ProgressMaterial` non-null: call `next(widget, geometry, delta)`.
- Otherwise the parent (`+0x01E50390`) with the same arguments, then log `action=parent-only` (13.3's per-frame limits).

**(call)** UI-7 reads the widget's members through R0's `readable_this_frame` (the server's frame cache), so the
per-frame cost after the first call of a frame is a table lookup, not a `VirtualQuery`. The other guards use R0's
plain checked reads.

### 7.6 UI-8 and UI-9: the fill progress

`OnShowFillProgress(duration)` and `OnHideFillProgress()`, Blueprint-callable, read `RadialAnimImage` and the member at
`+0x3A0`. Not measured crashing on a server; guarded because their siblings were **(ambiguity)**.

- Widget null: skip, log `null=this`.
- Both members non-null: call `next`.
- Otherwise skip, log the null ones.

### 7.7 UI-10: `OnSetShowDisabledIconWhenEnabled`

Reads `DisabledIcon` (`+0x3B8`) first thing.

- Widget null: skip, log `null=this`.
- `DisabledIcon` non-null: call `next(widget, texture)`.
- Otherwise skip, log `null=DisabledIcon(+0x3B8)`.

### 7.8 UI-11: `OnSetShowDisabledTextWhenEnabled`

`(text, vertical_offset, mark_as_enabled)`; the body owns its `FText` copy (3.5), uses `DisabledReason` always and
`InteractionHint` and `DisabledIcon` when `mark_as_enabled` is true.

- Widget null: release the text, skip, log `null=this action=skipped+text-released`.
- Bound = `DisabledReason` non-null and (`mark_as_enabled` false, or `InteractionHint` and `DisabledIcon` both
  non-null). Bound: call `next` with all four arguments.
- Otherwise release the text, skip, log the null ones of the three with `action=skipped+text-released`.

---

## 8. UI-12: the equipped-items refresh (S)

The function at `+0x01B86160` runs after an inventory update on the server's `AArchonInventory`. It takes the owner
(`+0xE0`, an `AArchonPlayerController`), the controller's `LOADOUT`, and the five equipped-item getters of 3.2, and
compares the updated items with what they return, assuming all five exist. Each getter returns null while the loadout's
`ActiveLoadoutSlotIndex` is still -1 (the engine logs "Attempt to access invalid slot index" for it). An inventory
update that reaches the function before a joining player's loadout has picked its slot read through null (Ramsgate, a
player joining, exit code 3). The refresh only re-checks equipped items; loadout initialisation runs it again.

With `DISABLE_INVENTORY_REFRESH_GUARD.flag` (read once, in Init) present, UI-12 is not installed.

Guard, on `(inventory, updated_items)`:

1. `owner` = the pointer at `inventory + 0xE0` if the inventory is non-null and readable for `0xE8`, else none.
2. If `owner` is readable for `0x9D0` and `is_a` `ArchonPlayerController`, `loadout` = the pointer at `owner + 0x9C8`;
   if it is non-null and readable for `0x690`:
   - `slot` = `i32` at `loadout + 0x680`; `num` = `i32` at `loadout + 0x678`; `data` = pointer at `loadout + 0x670`.
   - `0 <= slot < num` is false, or the pointer at `data + slot * 8` is unreadable or null: **skip**, reason
     `no valid active loadout slot`.
   - Otherwise call the five getters in the order of 3.2 with `loadout`; the first that returns null: **skip**, reason
     `an equipped-item getter returned null`. (The getters are called only with a valid slot, so they log nothing.)
3. In every other case (no owner, not a player controller, no loadout, unreadable): call `next(inventory,
   updated_items)`, as the AGPL does.

A skip logs (13.2) with the slot read (`-2` when none was read) and the inventory pointer.

---

## 9. The AGPL side

### 9.1 What seam patch 0041 already does when the MIT owns UI-1..13

Every UI id is a plain detour the seam maps by its target RVA (R0 7.4; UI-3 by its entry `+0x01E54110`, UI-2 by
`+0x01E0FBB0`). When the MIT owns it the AGPL's hook creation adds no wrapper, its original pointer is set to the
broker's `original`, and its enable, queue and remove calls succeed as no-ops. None of its R6 detours ever runs, and
nothing outside them calls their originals. **R6 needs no runtime patch: number 0050 stays unused (call).**

What the AGPL still does, and what its log then says (harmless, listed so the play-test is not misread):

- It still reads `DISABLE_CALLOUT_GUARDS.flag`, `DISABLE_INVENTORY_REFRESH_GUARD.flag` and
  `DISABLE_HUD_LEGENDARY_GUARD.flag` itself; with a flag present neither DLL installs that guard.
- UI-1, UI-2, UI-3 and UI-12: its install lines report a successful create, which now means "left to the MIT runtime".
- UI-4 to UI-11 and UI-13: it checks each target's prologue **after** the MIT DLL has hooked it, so it finds the
  broker's jump and logs that it aborted its group or skipped each site, or (client) that the late-join guard is not
  installed and "left unguarded". With the MIT owning these ids those lines are expected and mean nothing: the MIT's
  own install lines (13.1) are the ones to read.
- A target the MIT could not verify (bytes differ) is not hooked by either DLL: the AGPL's check expects the same bytes.

### 9.2 Cross-slice state (R0 7.5's table)

None. R6's guards keep no state another slice reads, and read none an AGPL block sets. UI-12 reads the loadout the game
keeps (R15's client repair reads the same type on the client; they never meet).

### 9.3 Install order (RT-17 for R6)

All R6 hooks install in the MIT Init (`DllMain`), before `Rustic-Isles.dll` loads and long before any widget exists.
The surface's 3.11 places UI-1 and UI-2 early, UI-3 and UI-4..11 late and UI-12 last on a server, and UI-13 right after
SP-1 on a client: that is the AGPL's order of writing. **Settled:** no order dependency exists among R6's ids (thirteen
distinct targets, no shared state) or between them and other slices', beyond the group rule of 7.1. R6 installs, on a
server, UI-1, UI-2, UI-3, the group UI-4..6, UI-7..11, UI-12; on a client, UI-13.

---

## 10. UI-13: `HandleWeaponEquipped` (C)

`UHUDLegendaryAbilityWidget`'s construct function (`+0x01F7AA00`) calls the `HandleWeaponEquipped` native directly with
the pawn's current weapon, unchecked. When a player joins a hunt late, the controller's inventory has replicated but no
weapon is equipped yet; the native then reads the weapon's item id (`+0x248`, at `+0x01F6D6BE`) through null. The
crashing call is native to native and never reaches `ProcessEvent`, so the guard is on the native body, which covers all
three callers (the exec thunk and `HandleLoadoutChanged` never pass null). The same construct has bound the handler to
the character's `OnWeaponEquipped`, which the equip broadcasts with the real weapon, so skipping the null call loses
nothing. **(ambiguity)** Static analysis only: no live late-join run has shown a skip line followed by the weapon icon.
A skip may leave the default icon until `HandleLoadoutChanged` runs; never worse than the fault.

Client only (servers build no HUD); with `DISABLE_HUD_LEGENDARY_GUARD.flag` (read once, in Init) present, not
installed.

Guard: weapon null: skip and log (13.2); else `next(widget, weapon)`.

---

## 11. Detaching (R0 3.3), the play-test and the flags

### 11.1 The detaching rule for R6 (departure from R0 3.3)

R0 says a detour that sees the detaching flag only forwards to its `next`. For a crash guard that would forward exactly
the call that faults. **(call)** R6's guards, once detaching, still decide as above (their reads are checked reads of
game memory; UI-4's and UI-12's class check may fill R0's name cache, which is not logging) but log nothing and count
nothing. The AGPL's UI-13 does the same.

### 11.2 The maintainer's play-test (collected for the one hands-on pass, rulings "Order, amended")

Surface section 8: "Ramsgate boots; Training Grounds; a hunt with interaction callouts; late join a hunt with a
legendary weapon." Build the AGPL runtime with patches 0041 and 0045 and the MIT DLL with R0, R1, R5 and R6 landed,
and publish both (R0 11.1 step 1).

1. **Install lines.** Let the Director start Ramsgate. Its `rustic_isles_dll_port<N>.log` has `[MitRuntime] install
   UI-1 ok` through `install UI-12 ok`, and `[WidgetGuard] server guards on (boot patch applied: yes)`. The AGPL's
   lines for these ids are the ones 9.1 lists. Any `bytes differ` or `group aborted` from the MIT is a failed
   play-test: report the line.
2. **Ramsgate.** Log in and walk Ramsgate for five minutes, using the airship slots and talking to vendors. The server
   does not exit. `[WidgetGuard] UI-1 ...` skip lines may appear; note how many.
3. **Training Grounds.** Enter Training Grounds and stay at least three minutes (the old fault came about 80 s after
   each spawn). The hub does not restart. `[WidgetGuard] UI-3 ... null=...` lines are expected.
4. **A hunt with callouts.** Start a hunt, use interaction prompts (pick-ups, the hold-to-revive or hold-to-interact
   prompts, the airship) and finish it. The server does not exit. Note any UI-4 to UI-11 lines.
5. **Joining.** With the hub running, join Ramsgate from a second account (or a fresh client) right after the first
   one is in. The hub does not exit; if `[WidgetGuard] UI-12 skipped ... no valid active loadout slot` appears, the
   joining player's loadout and arsenal are correct once they are in.
6. **Late join (client, UI-13).** Equip a legendary weapon, have a friend start a hunt, and join it after it started.
   The client does not crash. In the client's `rustic_isles_dll_port0.log`: `install UI-13 ok` and, if it fired,
   `[WidgetGuard] UI-13 skipped HandleWeaponEquipped(weapon=null) ...`. Note whether the legendary ability icon shows
   (it should once the weapon is equipped; report if it stays on the default icon).
7. **Flags revert.** With `DISABLE_CALLOUT_GUARDS.flag` next to the server's executable, UI-4 to UI-11 log
   `off (DISABLE_CALLOUT_GUARDS.flag)`; with `DISABLE_INVENTORY_REFRESH_GUARD.flag`, UI-12 does; with
   `DISABLE_HUD_LEGENDARY_GUARD.flag` next to the client's, UI-13 does. Remove each afterwards.
8. **Without the MIT DLL.** Remove `rustic-isles-runtime.dll` and repeat step 3: the AGPL's own guards do it as before.

Only CI passed until these steps are done.

---

## 12. Calls, departures and ambiguities

### 12.1 RT-12 to RT-18 for R6

- **RT-14 (flags), settled for R6:**

  | Flag | Kept? | Meaning |
  | --- | --- | --- |
  | `DISABLE_CALLOUT_GUARDS` | kept | UI-4 to UI-11 not installed (read once at Init) |
  | `DISABLE_INVENTORY_REFRESH_GUARD` | kept | UI-12 not installed (read once at Init) |
  | `DISABLE_HUD_LEGENDARY_GUARD` | kept | UI-13 not installed (read once at Init, client) |

- **RT-17 (install order), settled for R6:** 9.3. No dependency beyond the input group's all-or-nothing rule.
- **RT-18 (matching):** R6 registers no dispatch handler and matches no UFunction name; its one name check is `is_a` by
  the class short name `ArchonPlayerController` (3.2).
- RT-12, RT-13, RT-15, RT-16: not R6's.

### 12.2 Calls

1. UI-1, UI-2, UI-3 and UI-12 gain byte checks the AGPL does not make; every target fails closed (3.1).
2. Weak pointers are resolved over R0's checked reads, not through the game's function (3.4).
3. Every call into game code through a vtable checks the entry lies in the executable's image (3.5, 3.6).
4. R6 creates the reflection kit's `class_chain` and `is_a` if neither R15 nor R17 has landed it (2).
5. UI-7 reads through the server's frame cache (7.5).
6. Guards still decide while detaching, without logging (11.1).
7. No runtime patch: 0050 stays unused (9.1). The AGPL's misleading install lines for owned ids are left as they are
   and listed for the play-test. **Recommended to the maintainer:** accept; a patch to quiet them would only change
   log text in a DLL that goes at R21.
8. R6's lines use the tag `[WidgetGuard]`, distinct from every AGPL tag (R0 call 6).
9. R6 reads `headless::boot_patch_applied()` once for its status line (1, 13.2).

### 12.3 Departures

1. UI-1 and UI-2 skip lines are rate-limited like the others (13.3); the AGPL logs every one.
2. UI-2's pass-through line and the callout class's one-time member census are diagnostics and are not ported (RT-3).
3. UI-4 to UI-11 no longer ask whether the process is a server (always true where installed, 7.1).
4. The input group's rollback removes the MIT wrappers; the broker's empty chain then runs the game's functions, which
   is the same effect as the AGPL removing its hooks (7.1).

### 12.4 Ambiguities

- **UI-2's history.** The AGPL names `ScreenFadeIn` by an older RVA and a fault site (`+0x01CADEA6`) that, in 1.14.7,
  lie inside an unrelated function (`+0x01CADA84` to `+0x01CADEAA`); they are left over from an earlier build. The
  surface page repeats that fault site. The 1.14.7 function at `+0x01E0FBB0` does dereference `+0x3E8` unchecked
  (`+0x01E0FC2B`); the name is the AGPL's and was not re-checked against a symbol.
- UI-8 to UI-11 were never measured crashing on a server; they are guarded because their siblings were.
- UI-13 is a static proof; no live late-join run has confirmed the icon (10).
- Whether every bound member of the callout widget is null on a server (only `Keyboard` was measured). The guards do
  not depend on it.
- `GetOwningPlayer`'s vtable offset (`+0x2A0`) and the member at `+0x3A0` come from the AGPL's reading of the dump, not
  re-read here; a wrong vtable entry outside the image is refused (3.6), one inside it is not detectable.

No question is left open for the maintainer.

---

## 13. Log lines

Every line goes through R0's log. Lines made in Init go through `log::defer`; later ones are written directly.
Pointers are written `0x` and lowercase hex, `null` for 0. Nothing is logged while detaching (11.1).

### 13.1 Install lines (one per id, deferred from Init)

`[MitRuntime] install <id> <reason>`, `<reason>` one of: `ok`; `off (DISABLE_CALLOUT_GUARDS.flag)`,
`off (DISABLE_INVENTORY_REFRESH_GUARD.flag)`, `off (DISABLE_HUD_LEGENDARY_GUARD.flag)`; `not a function entry`;
`bytes differ`; `unreadable`; `parent call differs` (UI-5..7); `parent not a function entry` (UI-5..7);
`getter <rva> not a function entry or bytes differ` (UI-12, `<rva>` as `+0x01BA4FF0`); `group aborted (<id>:
<reason or status>)` (UI-4..6); a MinHook status name; `unhook failed <status>, still installed` (UI-4..6 rollback).
UI-1 and UI-2 are never written on a client and UI-13 never on a server (R6 installs nothing for them there).

### 13.2 Other lines

| When | Text |
| --- | --- |
| first install, server | `[WidgetGuard] server guards on (boot patch applied: yes\|no)` (deferred; from R5's `boot_patch_applied()`) |
| UI-1 skip | `[WidgetGuard] UI-1 skipped HideHoldText widget=<p> null=this\|HoldText(+0x3c0) count=<n>` (`this` when the widget is unreadable) |
| UI-2 skip | `[WidgetGuard] UI-2 skipped ScreenFadeIn screen=<p> primary(+0x3e8)=<p> secondary(+0x3e0)=<p> mode=<m> count=<n>` |
| UI-3 skip | `[WidgetGuard] UI-3 skipped OnSetDisabled widget=<p> null=<members> disabled=0\|1 count=<n> (owned text released)` |
| UI-4..11 | `[WidgetGuard] <id> <site> widget=<p> null=<members> action=skipped\|parent-only\|skipped+text-released count=<n>` |
| UI-12 skip | `[WidgetGuard] UI-12 skipped equipped-item refresh #<n>: <reason> (activeSlot=<slot> inventory=<p>)` |
| UI-13 skip | `[WidgetGuard] UI-13 skipped HandleWeaponEquipped(weapon=null) widget=<p> n=<n> (runs again from the character's OnWeaponEquipped)` |
| `FText` vtable refused | 3.5 (once per process) |

`<site>` is `RefreshInputKeys`, `NativeConstruct`, `NativeOnInputSwap`, `NativeTick`, `OnShowFillProgress`,
`OnHideFillProgress`, `OnSetShowDisabledIconWhenEnabled`, `OnSetShowDisabledTextWhenEnabled`. `<members>` lists the
null members as `Name(+0xoff)` joined by `,` in the order of 3.2's table, `this` for a null widget,
`PlayerInput(pc+0x348)` for UI-4's controller case, `none` if none (cannot happen on a skip). Offsets are lowercase
hex. `count` and `n` are 1-based.

### 13.3 Rate limits (pure)

Each id has its own 64-bit counter of skips. With `n` the 0-based number of this skip:

| Ids | Written when |
| --- | --- |
| UI-1 to UI-6, UI-8 to UI-11 | `n < 8` or `n % 1024 == 0` |
| UI-7 | `n < 4` or `n % 100000 == 0` |
| UI-12 | `n < 20` or `n % 100 == 0` |
| UI-13 | `n + 1 <= 8` or `n + 1` is a power of two |

The null-member text is built only when a line is written (UI-7 runs every frame).

---

## 14. Tests

### 14.1 In CI

Core (`cargo test -p rustic-isles-runtime-core`, every platform):

- `super_call_target` on the bytes of 3.1: UI-5 gives `0x01E4A480`, UI-6 `0x01E4DEE0`, UI-7 `0x01E50390`; a non-`E8`
  byte and a short slice give none.
- Every guard's decision function over a `BufferMemory` widget of `0x400` bytes: each member bound or null in turn,
  the null widget, an unreadable widget; the verdicts of 4 to 8 and 10 (call next, parent only, skip, skip and release).
  UI-11 with `mark_as_enabled` both ways. UI-3 with each one of the three members null.
- UI-4's controller order: weak pointer resolving (call next when `PlayerInput` is set, skip when not); weak pointer
  dead and `GetOwningPlayer` (a fake) returning a non-player-controller (call next), a player controller without
  `PlayerInput` (skip), null (call next).
- UI-12 over a synthetic inventory → controller → loadout graph: slot -1, slot = `num`, null slot data, each getter
  (fakes) null in turn (skip with the right reason), all present (call next), owner not a player controller and no
  owner (call next).
- `resolve_weak`: serial 0, negative index, index at the count, serial mismatch, each flag bit of `0x30000000`, a good
  item, an item in the second chunk.
- `release_owned_text` decisions over a buffer with fake counts: shared count 2 (decrement only), shared 1 and weak 2
  (destroy only), shared 1 and weak 1 (destroy and delete), entries outside the image (neither called, the line once).
- The rate limits of 13.3 at their edges (7, 8, 1023, 1024; 3, 4, 99 999, 100 000; 19, 20, 100; 8, 9, 16).
- Every line of 13.1 and 13.2 for fixed inputs.
- The registry with `LANDED` holding R6: `owns` true for UI-1..13; the offset table test with R6's thirteen lines.

DLL (`cargo test -p rustic-isles-runtime`, Windows runner):

- The input group on three test functions behind the broker: all three hooked; a second run where the third "verify"
  fails installs none; a run where the third hook fails leaves the first two unhooked (their test functions run
  unguarded).
- A UI-6-shaped test function and its parent: with a null member the parent runs once and the original never.
- The `FText` release on a fake controller whose vtable points at two test functions inside the test executable's image:
  called in the right cases with the right arguments.
- A detour while the detaching flag is set: the decision holds, nothing is logged.

**Never in CI:** anything against the real executable: the RVAs, the bytes, the members, the parents and getters, and
every crash these guards prevent.

---

## 15. Size

Rust without tests: core `widgets.rs` about 220 lines (decisions 120, lines and limits 60, the super-call check 20,
tables 20), `weak.rs` about 30; the DLL's `widgets.rs` about 120 (install, group, flags) and `ffi/widgets.rs` about 300
(thirteen bodies, parents, getters, text release, checked virtual calls). `offsets.rs` gains about 90 lines. Tests about
300. No runtime patch.
