<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: runtime slice R17, client branding and console (RD27, plan item 11)

R17 is the client's front-end slice of the MIT game runtime (`spec-runtime-surface.md` section 4.3 and section 8's
R17 row; rulings in `rulings-2026-11-13.md`). It puts Dainty Lass's own words on the press-start popup and in the
credits, hides the stale version line of the login screen's patch notes, drives the login screen for scripted runs
when `AUTO_PRESS_START.flag` is present, and gives the client the in-game console on **F2**. All of it runs on the
client's `ProcessEvent` chain (SP-1); none of it is a hook of its own.

**RT-9 applies:** no login background until a new image is drawn. The background handler is **not ported yet**;
section 4.4 records its hook point and behaviour so it can be added once an MIT image exists. Nothing of Mystic
Paradox's image, resource script or resource header crosses, and neither do its credits or press-start texts.

An implementer builds from this page, `spec-runtime-r0.md`, `spec-runtime-r1.md` and the surface page alone, and never
reads `restricted/`. What crossed is facts: RVAs into `Dauntless-Win64-Shipping.exe` (1.14.7, CL 647472), UE and game
class, function and member names, member offsets from the maintainer's Dumper-7 dump of this build, parameter block
sizes, enum values, flag file names, and behaviour. The AGPL side of the console (runtime patch 0044) was written by the
spec writer in `restricted/` and is described here only by its contract (section 6.3).

Conventions are the surface page's and R0's. **(call)** marks a decision this page makes, **(departure)** a deliberate
difference from the AGPL runtime, **(ambiguity)** something the source did not settle; all are collected in section 9.

---

## 1. Scope

| R17 owns | Section | Id |
| --- | --- | --- |
| The reflection kit every R17 handler calls through (find a class, a default object, a UFunction; call it) | 3 | — |
| Login screen: the patch-notes version lines | 4.2 | `SP-1@R17` |
| Login screen: the background and the logo (**not ported yet**, RT-9) | 4.4 | `SP-1@R17` |
| Credits: the Dainty Lass block above Phoenix Labs | 4.5 | `SP-1@R17` |
| Press-start popup: the welcome text | 4.6 | `SP-1@R17` |
| `AUTO_PRESS_START.flag`: press start, Play, travel to Ramsgate | 5 | `SP-1@R17` |
| The F2 console | 6 | **`CON-1`** (new) |
| The player-visible texts in `en` and `pt-BR` | 7 | — |

`SP-1@R17` is already in R0's registry, and covers exactly the AGPL blocks R0 7.5 lists for it: the login-screen
automation (press start, Play, travel) and the login, credits and press-start branding. The F2 console is **not** an
SP-1 block on the AGPL side: it is the tail of the AGPL's client MainThread, which seam patch 0041 does not gate. So R17
introduces the id **`CON-1`** and runtime patch 0044 gates that tail on it (section 6.3).

Not R17: every other SP-1 feature (R14, R15, R16), MainThread itself (R9), and the logging-only AGPL blocks (RT-3).

---

## 2. Where the code goes

| File | Crate | Kind | Holds |
| --- | --- | --- | --- |
| `rustic-isles-runtime-core/src/reflect.rs` | core | pure | class chain walks, `is_a` by class name, finding a UFunction in a class chain, finding a class and its default object, the parameter-size guard (3.1–3.3), over R0's `Memory` trait |
| `rustic-isles-runtime-core/src/branding.rs` | core | pure | the texts of section 7 and the culture rule, the matchers of 4–6 as `Matcher` constants, the auto-login poll counters (5), the console schedule (6.2), the credits slot reordering as a function over a slice (4.5 step 7) |
| `rustic-isles-runtime/src/ffi/reflect.rs` | DLL | `unsafe` (under `ffi`) | the one `ProcessEvent` call (3.4), checked writes of game memory (3.5), building an `FString` parameter |
| `rustic-isles-runtime/src/branding/` (`mod.rs`, `login.rs`, `credits.rs`, `press_start.rs`, `auto_login.rs`, `console.rs`) | DLL | safe, Windows only | the handler bodies; they read game memory only through R0's `ffi::memory` and call game code only through `ffi::reflect` |

One line each in `rustic-isles-runtime/src/lib.rs` (`#[cfg(minhook)] pub mod branding;`), `ffi/mod.rs`
(`pub mod reflect;`) and `rustic-isles-runtime-core/src/lib.rs` (`pub mod reflect; pub mod branding;`). The
registration call goes into Init (R0 3.2) before `handlers::freeze()`, client only: `branding::register()`.

**Registry (`registry.rs`):** add `"R17"` to `LANDED`, and the line `e("CON-1", Setting, "R17")` under a comment
`// R17: the client console (MainThread on the AGPL side, patch 0044)`. **(call)** `Setting` is the closest kind (a
process setting with no hook); no new kind is added. `owns` is then true for `SP-1@R17` and `CON-1`, and the status
line's count grows by 2.

**Offset table (`offsets.rs`):** R17 adds no `TARGETS` line (it installs no hook). It adds a module
`pub mod r17 { ... }` holding the layout facts of section 8, each with its owner class in the doc comment. R0's table
test (every detour id has one `TARGETS` line) is unaffected.

**(call) Shared kit.** No earlier slice gives a way to find or call a UFunction, and R15 (landing beside R17) may need
one. `reflect.rs` in both crates is written as a general kit with no R17 name in it; whichever of R15 and R17 lands
first creates it, the other only adds to it. Its API shape is section 3.

What CI tests (section 10.1): all of `reflect.rs` and `branding.rs` in the core crate on every platform. What only the
play-test checks: every call into the game.

---

## 3. The reflection kit

Every game object R17 touches is reached from the object or function the dispatch handed it, or from the object array
(R0 6.2); every read goes through R0's checked reads; a read that fails ends that handler's work for the call, never
faults.

### 3.1 Engine layout facts (UE 4.26 as built into 1.14.7)

| Fact | Value |
| --- | --- |
| `UField.Next` | `+0x28` (pointer) |
| `UStruct.SuperStruct` | `+0x40` (pointer) |
| `UStruct.Children` | `+0x48` (pointer to the first `UField`) |
| `UClass.ClassDefaultObject` | `+0x118` (pointer) |
| `UFunction.FunctionFlags` | `+0xB0` (`u32`) |
| `UFunction.ParmsSize` | `+0xB6` (`u16`) **(ambiguity: the dump pads this region; the value is stock UE 4.26's layout, checked by the play-test's log, 11 step 2)** |
| Object flag `RF_ClassDefaultObject` | `0x10` in the object flags (`u32` at `+0x08`) |
| `FString` | data pointer `+0x00`, `Num` `i32` `+0x08`, `Max` `i32` `+0x0C`; `Num` counts the terminating NUL |
| `FText` | `0x18` bytes, opaque (a shared reference): never read as text, never byte-copied into game memory, only passed back to the game as a parameter |
| `FName` | comparison index `i32` `+0x00`, number `u32` `+0x04` |
| `TArray<T>` | data pointer `+0x00`, `Num` `i32` `+0x08`, `Max` `i32` `+0x0C` |

### 3.2 Lookups (pure, core)

- `class_chain(class)`: `class`, then each `SuperStruct`, at most 64 steps, stopping at null or an unreadable pointer.
- `is_a(object, class_name)`: the object's class (`+0x10`) or one of its supers has the short name `class_name`.
  (R0's names: the short name is the text after the last `/`.) Used with game class names, never with full paths.
- `find_function(class, name)`: walk `class_chain(class)`; in each, walk `Children` then `Next`, at most 8 192 fields
  per class; the first field whose own class's short name is `Function` and whose name equals `name`. `None` if none.
- `find_function_matching(class, pred)`: the same walk over the object's **own class only** (no supers), with a
  predicate on the field name (section 5.2 needs it).
- `find_class(name)`: the first object in the object array, by index, whose name is `name` and whose class's short
  name is `Class` or ends with `GeneratedClass`. Cached per name once found (a class is never collected).
- `default_object(class)`: `ClassDefaultObject` at `+0x118`, required live (R0's `is_live_object`).
- `first_instance(class_name)`: the first live object, by index, that is not a default object (flag `0x10` clear) and
  `is_a(class_name)`. A full scan; each caller says how often it may run.

### 3.3 The parameter guard (pure, core)

Every call names its expected parameter block size `expected` (the dump's parameter struct size, section 8.2). The
caller allocates a zeroed buffer of `max(round_up(expected, 8), 16)` bytes. `params_fit(function, buffer_len)` reads
`ParmsSize` and answers yes iff it is readable and `ParmsSize <= buffer_len`. A call whose guard fails is not made and
logs once per function name `[MitBranding] <Class>.<Function>: parameter size <n> exceeds <m>, call skipped`. **(call)**
This is new: it turns a wrong function or a wrong layout into a skipped feature instead of a stack overwrite.

### 3.4 `call(object, function, buffer) -> bool` (`ffi`)

- Preconditions: `object` live, `function` non-null and found by 3.2 in the object's class chain, `params_fit`.
- Calls the game's `UObject::ProcessEvent` at **`+0x02861880`** (the hooked entry, not the broker's trampoline), as
  `extern "C" fn(object, function, params)`. **(call)** Calling the hooked entry means the call goes down the whole
  SP-1 chain like any call the game makes: other MIT handlers, the AGPL wrapper and its blocks see it, exactly as when
  the AGPL calls a function through the object's virtual `ProcessEvent`. R17's own matchers never match a function it
  calls (section 10.1 tests this), so it never re-enters itself through them.
- Only on the game thread, from inside an SP-1 handler (every R17 call is). Never after detach.
- Returns false when a precondition failed; the call itself has no failure value.

`static_call(class_name, function_name, buffer)`: `call(default_object(find_class(class_name)), find_function(...),
buffer)`, for the Blueprint library functions of section 8.2.

### 3.5 Writes and parameters (`ffi`)

- `write<T: Copy>(addr, value) -> bool`: `readable(addr, size_of::<T>())` and the region is writable
  (`PAGE_READWRITE`, `PAGE_EXECUTE_READWRITE`, `PAGE_WRITECOPY` or `PAGE_EXECUTE_WRITECOPY`), then a volatile write.
  R17 writes game memory in exactly two places: the credits section's flag byte and the order of a panel's slots
  (4.5); everything else goes through the game's own setters.
- `FString` parameters: R17 owns the UTF-16 buffer (NUL-terminated, kept alive for the call) and writes data
  pointer, `Num = len + 1`, `Max = len + 1` into the parameter block. The game's thunk reads a `const FString&` and does
  not free it.
- `FText` values: `text_for(s)` calls `KismetTextLibrary.Conv_StringToText` once per distinct text and keeps the
  returned `0x18` bytes in a process-lifetime table, never destroyed. **(call)** The copy R17 holds is never released,
  so the text it references stays alive for the process (a few hundred bytes); each `SetText` passes a byte copy of
  it as a parameter, which the game's thunk copies properly.

---

## 4. Branding handlers on SP-1

### 4.1 Placement

All branding handlers are SP-1 **after** handlers (they run after the chain: after the game's own Blueprint and
native work, and after every AGPL block of the call). This is the AGPL's position too ("after the screen's own handler,
so its refreshes cannot undo it"), so moving them changes nothing. `order`: login 0, credits 10, press start 20.

Each `DISABLE_*_BRANDING` flag is read **once**, at the handler's first match, and kept (R0 4.2 "read-once"); with it
present the handler does nothing for the process. This matches the AGPL.

### 4.2 Login screen: the patch-notes version lines

- **Matcher (RT-18):** `Contains("LoginScreen")`. It catches every function whose full name holds `LoginScreen`: the
  functions of `LoginScreen_bps_C` (package `LoginScreen_bps`) and of the native `Archon.LoginScreen` class, and any
  other class with `LoginScreen` in its name. The body filters by object (next bullet). **(call)** The same breadth as
  the AGPL, because the screen's refreshes come from many of its events.
- **Body:** if `DISABLE_LOGIN_BRANDING.flag` (read once), stop. The object must be live and `is_a("LoginScreen_bps_C")`.
  Then collapse, each only when present, live, and not already collapsed:
  - `PatchNotesPreviewWidget` (`LoginScreen` `+0x500`) → its `PatchNotesVersionTextBlock` (`+0x398`);
  - `PatchNotesPopupWidget` (`LoginScreen` `+0x508`) → its `PatchVersion` (`+0x398`).
- **Collapse** = `Widget.SetVisibility(InVisibility = Collapsed)` (`ESlateVisibility::Collapsed = 1`, parameter block
  `0x01`), skipped when the widget's `Visibility` byte (`UWidget` `+0xCB`) already reads 1, so a repeat costs one
  read.
- **Why:** these lines show the game's own "Update <release version>" heading over patch notes that the Dainty Lass
  backend serves (its title and text), so they contradict it. **(call)** Ported without the background: unlike the
  logo (4.4), they do not depend on an image.
- Log: the first 16 collapses, `[MitBranding] collapsed <preview|popup> version line after <function short name>`.

### 4.3 What a dispatch looks like

The handler sees `PeCall { object, function, params, entry }` (R0's API). "Function short name" in log lines is the
text after the last `.` of the entry's full name.

### 4.4 Login screen: background and logo — not ported yet (RT-9)

Recorded so it can be added when an MIT image exists. Until then the login screen shows the game's own season art and
its DAUNTLESS logo: that is the stock screen, and it is what RT-9 asks for. **When MIT owns `SP-1@R17`, the AGPL's
background stops too** (it is one of the blocks that id gates), so the Paradox image is not shown from then on.

To add it later (one file, `branding/login_background.rs`, and the image beside it; no change to this page's other
parts):

- **Image:** a PNG drawn for Dainty Lass, MIT or CC-licensed with its source recorded, 16:9 like the season art,
  embedded with `include_bytes!` (**(call)** no `.rc` file or Windows resource: the DLL carries it as data).
- **Hook point:** the same after handler as 4.2, same matcher, same object filter, same flag
  (`DISABLE_LOGIN_BRANDING.flag`), before the version-line collapses.
- **Behaviour:** `LoginScreen_bps_C.SplashImage` (`+0x8F0`, a `UImage`) shows the season's login texture, set by the
  screen's `SetSplashImage` from the served hunt-pass season, loaded asynchronously. On every login-screen event:
  1. If R17 already made a texture and it is still live (R0's `is_live_object`), and the image's brush resource
     (`UImage.Brush` `+0x170`, `FSlateBrush.ResourceObject` `+0x48`, so `+0x1B8` from the image) is that texture: done.
  2. Else, if there is no live texture: `KismetRenderingLibrary.ImportBufferAsTexture2D(WorldContextObject = the
     screen, Buffer = TArray<uint8> pointing at the embedded bytes (Num = Max = length), ReturnValue)`, block
     `0x20` (the thunk only reads the array; R17's bytes stay owned by the DLL). A null result: log once, keep the
     season art, never try again in this process.
  3. `Image.SetBrushFromTexture(Texture, bMatchSize = false)` on `SplashImage`, block `0x10`: the screen keeps the
     brush size it laid out, and the season texture's pending load is cancelled by the new brush.
  4. Collapse `ArchonLogoMat` (`LoginScreen_bps_C` `+0x700`, `/Game/UI/Textures/Logo/ArchonLogo_Mat`) as in 4.2, **only
     if the new image carries the Dainty Lass name** (the AGPL collapsed the logo because its image carried its own).
- The texture is referenced only by the brush and is collected with the screen; the next screen imports it again.
- Play-test when added: the login screen shows the new image and keeps it through the screen's animations;
  `DISABLE_LOGIN_BRANDING.flag` brings the season art back.

### 4.5 Credits: the Dainty Lass block above Phoenix Labs

The credits widget `bpw_Credits_C` stacks studio sections in one scroll box; Phoenix Labs' is
`credits_section_phoenix` (`+0x3F8`, class `credits_section_name_title_bpw_C`, a `UArchonCreditsSection`). A section
is a title text block (`SectionTextBlock` `+0x3B0`) over a horizontal box (`ColumnsBox` `+0x3B8`) whose first two
children are text blocks holding all names and all titles as one multi-line string each, filled at construction from
the section's table. A section created without a table keeps whatever text is set on it (its native construct returns
early, logging only that no table is set). That is the fact R17 relies on.

**Matcher (RT-18):** `FunctionName("OnInitialized")` or `Contains("bpw_Credits_C.PreConstruct")`. **(call)** The first
is narrower than the AGPL's substring (`OnInitialized` anywhere in the name); both are then filtered by the object's
class short name being exactly `bpw_Credits_C` (cache the class pointer once seen; a class is never collected).

**Flag:** `DISABLE_CREDITS_BRANDING.flag`, read once.

**On `OnInitialized`** (the widget is initialised and Slate is not built yet, so the children's order can still be
set). The block, top to bottom, is: title; subheading; disclaimer; a names | roles section; then Phoenix Labs'.

1. `phoenix` = the pointer at `+0x3F8`, live. Its slot = `UWidget.Slot` (`+0x28`); the panel = `UPanelSlot.Parent`
   (`+0x28`); `index` = the position in the panel's `Slots` (`UPanelWidget` `+0x170`, a `TArray` of slot pointers)
   whose `Content` (`UPanelSlot` `+0x30`) is `phoenix`. Any of these missing: log
   `[MitCredits] Phoenix Labs section not as expected - credits left unchanged` and stop.
2. Styles: `title_style` = `phoenix.SectionTextBlock`; `roles_style` = the second child of `phoenix.ColumnsBox` (its
   `Slots[1].Content`), or `title_style` if absent (log once).
3. The section: `WidgetBlueprintLibrary.Create(WorldContextObject = the credits widget, WidgetType = phoenix's class,
   OwningPlayer = null)` (block `0x20`). Same class as Phoenix's, so the same fonts, colours and columns, and no
   table. Null, or its `SectionTextBlock` or `ColumnsBox` null: log and stop.
4. Copy `bShowContributerTitle` (byte at `+0x3A8`) from `phoenix` to the section (3.5's checked write); collapse the
   section's own `SectionTextBlock` (4.2's collapse); set its two column texts (section 7: names, roles).
5. Three text lines, each: `GameplayStatics.SpawnObject(ObjectClass = the style's class, Outer = the credits
   widget's WidgetTree (UUserWidget +0x260))` (block `0x18`), then style it from its style block through the
   game's setters, each with a parameter block that is a byte copy of the style's member:
   `TextBlock.SetFont` (from `Font` `+0x1F0`, `0x58`), `TextBlock.SetColorAndOpacity` (`ColorAndOpacity` `+0x1B8`,
   `0x28`), `TextBlock.SetShadowOffset` (`ShadowOffset` `+0x2D0`, `0x08`), `TextBlock.SetShadowColorAndOpacity`
   (`ShadowColorAndOpacity` `+0x2D8`, `0x10`), `TextLayoutWidget.SetJustification` (`Center = 1`, `0x01`), then
   `TextBlock.SetText`. The title takes `title_style`; subheading and disclaimer take `roles_style`.
   **(departure)** The AGPL assigned the font and colour members directly and skipped the colour when it was linked;
   the setters copy them as the engine does (its reference counts included), so the colour is always copied.
6. Add the four widgets in order (title, subheading, disclaimer, section): `PanelWidget.AddChild(Content)` on the
   panel (block `0x10`, returns the new slot). If the new slot's class equals `phoenix`'s slot's class, set its
   alignment from `phoenix`'s slot with the slot's own setters: `ScrollBoxSlot.SetHorizontalAlignment` (byte at slot
   `+0x48`), `SetVerticalAlignment` (`+0x49`), and **for the first widget only** `SetPadding` (the `FMargin` at
   `+0x38`, 16 bytes): only the first line takes the gap Phoenix has above it; the rest stack tight.
   **(departure)** The AGPL copied these members by reflected property lookup; the setters are the engine's.
7. Move each new slot into place: after `AddChild` it is the panel's last slot; move it to position `index + placed`
   by shifting the slots between down by one (3.5's checked writes of the slot pointers; the data pointer must be
   readable and writable for `Num` pointers). The shift is the pure function `reorder(slots, from, to)` in
   `branding.rs`. If the new slot is not last, or the target is out of range, stop placing (the widgets already added
   stay at the end, still readable). Slate later builds the box's children in `Slots` order, so this places them;
   the panel has no reflected insert-at, which is why the array is written directly.
8. If all four are placed, remember the section (pointer and object index) for the re-check. Log
   `[MitCredits] Dainty Lass block above Phoenix Labs: placed <n>/4 at index <index>`.

**On `bpw_Credits_C.PreConstruct`** (built by now): if the remembered section is still live (its index round-trips),
set its two column texts again; log the first 4 times `[MitCredits] credits constructed; Dainty Lass section
present|missing`.

### 4.6 Press-start popup: the welcome text

`PressStartScreen_bps_C` is the popup before the login screen. Its `PressStartPanel` (`+0x4D0`, a
`PressStartPanel_bpw_C`) shows the game's welcome text (an invitation to the game's community channels, from the game's
localisation) in `WelcomeText` (`+0x3D0`, a `UTextBlock`). The same panel class also backs the login screen's queue and
"cannot play online" dialogs, which must keep their text.

- **Matcher (RT-18):** `Contains("PressStart")`. It catches the functions of `PressStartScreen_bps_C`, of
  `PressStartPanel_bpw_C` and of the native `Archon.PressStartScreen`; the body ignores any other object.
- **Flag:** `DISABLE_PRESS_START_BRANDING.flag`, read once.
- **Body:** find the panel:
  - full name contains `PressStartScreen_bps_C.`: the object must be live and `is_a("PressStartScreen_bps_C")`; the
    panel is its `PressStartPanel`;
  - full name contains `PressStartPanel_bpw_C.`: the object is the panel only if its owner is a press-start screen:
    the object's outer (`+0x20`, its widget tree) has an outer that `is_a("PressStartScreen_bps_C")`;
  - otherwise nothing.
  Then `TextBlock.SetText(welcome)` on the panel's `WelcomeText` (section 7). Re-applied on every matching event,
  because the panel's own refresh may rewrite it. Log the first 6 times
  `[MitPressStart] welcome text set after <function short name>`.

---

## 5. `AUTO_PRESS_START.flag`: the scripted login

A test aid for a scripted run (the maintainer's repro scripts): the 1.14.7 press-start popup ignores synthetic input
(neither simulated keys nor a simulated click dismiss it, and it logs that it has no default focusable widget), so a
script cannot get past it. With the flag present, R17 makes the three calls a player's clicks would make. Without it,
nothing in this section runs.

### 5.1 Placement and flag

SP-1 **before** handler, `order` 0 (the AGPL ran these before the original too), never absorbing. The flag is probed
on every matching call through R0's flag cache (so it takes effect within 4 s). **(call)** It is not tied to the
development profile, as in the AGPL: the maintainer's scripted runs inject the DLLs by hand without the launcher.

**Matcher (RT-18):** `FunctionName("OnStartupMovieFinished")` or `FunctionName("OnPlayScreenPollTimer")`, and the body
also requires the full name to contain `LoginScreen` (both are `LoginScreen_bps_C` functions). **(call)** Narrower than
the AGPL, which ran the three steps on every `LoginScreen` call and filtered inside.

Every step below is once per process (a static), and logs with the tag `[MitAutoLogin]`.

### 5.2 The three steps

1. **Press start.** On `OnStartupMovieFinished` (the moment the login screen becomes interactive): find
   `OnPressToStartDismissed` in the object's class chain and call it with `UserIndex = 0` (block `0x04`). This is the
   function a click on the popup calls; the client's log shows it leading into the rest of the login chain. Not found:
   log `OnPressToStartDismissed not found; leaving the screen alone` and stop this step for the process.
2. **Play.** On `OnPlayScreenPollTimer` (fires once a second while the Play screen is up), from the **4th** poll seen
   (let the menu settle): find, among the object's **own class's** fields (`find_function_matching`), the function
   whose name contains both `w_loginbutton` and `Clicked`: the Blueprint event the Play button's click raises (its name
   carries a graph index, so it is matched by those two parts). Call it with a zeroed block of 256 bytes (it takes
   none). Not found: log every 10th poll and keep looking.
3. **Travel to Ramsgate.** On the same poll, after step 2, with its own count, from the 4th poll: find the first live
   non-default object that `is_a("ArchonPartyClient")` (an info actor that exists on the menu; there is no player
   controller yet), find `StartTravelToCity` in its class chain, and call it with `bLeaveWithParty = false` (block
   `0x01`). The function missing: log and stop this step. No party client yet: log every 10th poll and keep looking.
   **(call)** The object scan runs at most once per poll (once a second) until it succeeds.

---

## 6. The F2 console (`CON-1`)

### 6.1 What it is

A shipping client has no console. The AGPL made one: three seconds after the client's world exists, it set the first
console key of the input settings to **F2**, spawned an object of the engine's console class with the game viewport as
outer, and stored it as the viewport's console. F2 then opens the engine's console.

### 6.2 MIT behaviour

**(call)** On the game thread, from an SP-1 **after** handler (`order` 100), not from a thread of its own: the console
object is created where the engine creates objects. **Matcher:** `Prefix("Function ")` (every UFunction; the match bit
is cached per function, so the cost is one atomic load per call once done).

The handler, while not done (an atomic; once done it returns at once):

1. At most every 250 ms (a tick check; the schedule is the pure `ConsoleSchedule` in `branding.rs`).
2. Find the engine once: `first_instance("Engine")` (the game's engine object is the first non-default object whose
   class derives from `Engine`), kept while live.
3. `GameViewport` (`UEngine` `+0x780`) non-null and live; the first time it is seen, note the tick.
4. When 3 000 ms have passed since then (the AGPL's 3 s after the world appeared; **(departure)** measured from the
   viewport, which the console needs):
   - The key: `InputSettings`' default object (`default_object(find_class("InputSettings"))`), `ConsoleKeys`
     (`+0x130`, a `TArray<FKey>`, `FKey` 0x18 bytes, its `KeyName` `FName` at `+0x00`). If `Num >= 1`, write into
     element 0's `KeyName` the `FName` of `F2` from `KismetStringLibrary.Conv_StringToName("F2")` (block `0x18`, the
     name at `+0x10`). Only the name is written; the console compares keys by name. **(departure)** `Num == 0` is
     logged and the key left alone (the AGPL indexed element 0 unchecked).
   - The console: if the viewport's `ViewportConsole` (`UGameViewportClient` `+0x40`) is already non-null, keep it
     (**(departure)** the AGPL always replaced it; keeping it makes a second maker harmless). Else read the engine's
     `ConsoleClass` (`UEngine` `+0xF0`); null: log and give up. `GameplayStatics.SpawnObject(ObjectClass =
     ConsoleClass, Outer = the viewport)` (block `0x18`); a non-null result is written to `ViewportConsole`
     (3.5's checked write).
   - Done. Log `[MitConsole] console on F2 (key <set|left: no console keys>, console <spawned|kept>)`.
5. Any failure in step 4 other than those named: log once and mark done (no retry storm).

`CON-1` is a client-only id; the handler is registered only on a client (SP-1 is client only).

### 6.3 The AGPL side: runtime patch 0044 (contract)

Patch 0041's gate covers hooks and SP-1/2/3 feature blocks, not MainThread, so without 0044 both DLLs would make a
console. Patch 0044 (in `restricted/`, `docs/paradox-patches.md` row 0044) does exactly this:

- In the AGPL's client MainThread, after its world wait and before its 3 s sleep, it asks `rir_owns("CON-1")` (through
  the seam, once). Owned: it logs `[MitSeam] CON-1 owned by rustic-isles-runtime.dll: the F2 console is left to it`
  and the thread ends without touching the input settings or the viewport. Not owned, or the MIT DLL absent: as
  before.
- Nothing else changes. With the MIT DLL absent the AGPL runtime behaves exactly as before 0044.

So R17 ships with an AGPL runtime that has 0044. With an older AGPL runtime (0041 only) both make a console; R17's
"keep an existing console" rule makes that harmless (one console stays, the AGPL's later one may replace it).

---

## 7. The texts (DU2)

Every text below is Dainty Lass's own; none is taken from the AGPL runtime. They live in `branding.rs` as one table per
language with the same keys, and a test checks both tables have every key.

**Language (call):** read once, at the first text needed, `KismetInternationalizationLibrary.GetCurrentCulture()`
(block `0x10`, an `FString` return of at most 32 characters read through checked reads; the engine-allocated string is
not freed, once per process). A culture starting `pt` (case-insensitive) takes `pt-BR`; anything else, or a failed
read, takes `en`.

| Key | en | pt-BR |
| --- | --- | --- |
| `press_start.welcome` | `Welcome to Dainty Lass, Slayer.\nA community server that keeps Dauntless playable.\n\nNot affiliated with Phoenix Labs.` | `Bem-vindo ao Dainty Lass, Slayer.\nUm servidor da comunidade que mantém Dauntless jogável.\n\nSem vínculo com a Phoenix Labs.` |
| `credits.title` | `Dainty Lass` | `Dainty Lass` |
| `credits.subheading` | `Community server\n` | `Servidor da comunidade\n` |
| `credits.disclaimer` | `Dainty Lass is a community preservation project\nand is not affiliated with Phoenix Labs.\n` | `Dainty Lass é um projeto de preservação da comunidade\ne não tem vínculo com a Phoenix Labs.\n` |
| `credits.names` | `Silent Spud\nThe Undaunted and Mystic Paradox communities\n\n` | `Silent Spud\nAs comunidades Undaunted e Mystic Paradox\n\n` |
| `credits.roles` | `Dainty Lass\nThe servers it grew from\n\n` | `Dainty Lass\nOs servidores de onde ele veio\n\n` |

`\n` is a line break. The names and roles columns end with the same two line breaks so their rows stay aligned and the
block is set apart from Phoenix Labs' section; the subheading and disclaimer end with one.

**(call) Who the credits name.** The MIT credits name the maintainer by the handle the repository already carries
(`Silent Spud`) and credit the earlier server work by project, not by person. The AGPL credited three people of the
earlier projects by name; whether the MIT block names any upstream person, and which, is the maintainer's ruling
(section 9, question R17-Q1). "Slayer" is left in English in `pt-BR` (the game's own term; **(ambiguity)** the
1.14.7 Brazilian localisation's word for it was not checked) and the `pt-BR` wording wants a native speaker's review.

---

## 8. Offset-table lines (RT-2)

### 8.1 Members (`offsets::r17`)

| Constant | Owner | Offset |
| --- | --- | --- |
| `UFIELD_NEXT` | `UField` | `0x28` |
| `USTRUCT_SUPER` | `UStruct` | `0x40` |
| `USTRUCT_CHILDREN` | `UStruct` | `0x48` |
| `UCLASS_DEFAULT_OBJECT` | `UClass` | `0x118` |
| `UFUNCTION_FLAGS` | `UFunction` | `0xB0` |
| `UFUNCTION_PARMS_SIZE` | `UFunction` | `0xB6` (`u16`) |
| `RF_CLASS_DEFAULT_OBJECT` | object flags | `0x10` |
| `UWIDGET_SLOT` | `UWidget` | `0x28` |
| `UWIDGET_VISIBILITY` | `UWidget` | `0xCB` (`u8`) |
| `UPANELSLOT_PARENT` | `UPanelSlot` | `0x28` |
| `UPANELSLOT_CONTENT` | `UPanelSlot` | `0x30` |
| `SLOT_PADDING`, `SLOT_HALIGN`, `SLOT_VALIGN` | `UScrollBoxSlot` | `0x38` (16), `0x48` (`u8`), `0x49` (`u8`) |
| `UPANELWIDGET_SLOTS` | `UPanelWidget` | `0x170` |
| `UUSERWIDGET_WIDGET_TREE` | `UUserWidget` | `0x260` |
| `UTEXTBLOCK_COLOR`, `_FONT`, `_SHADOW_OFFSET`, `_SHADOW_COLOR` | `UTextBlock` | `0x1B8` (0x28), `0x1F0` (0x58), `0x2D0` (0x08), `0x2D8` (0x10) |
| `UIMAGE_BRUSH`, `SLATEBRUSH_RESOURCE` | `UImage`, `FSlateBrush` | `0x170`, `0x48` (4.4, later) |
| `LOGIN_PATCH_PREVIEW`, `LOGIN_PATCH_POPUP` | `ULoginScreen` | `0x500`, `0x508` |
| `PREVIEW_VERSION_TEXT` | `UPatchNotesPreviewWidget` | `0x398` |
| `POPUP_VERSION_TEXT` | `UPatchNotesPopupWidget` | `0x398` |
| `LOGIN_ARCHON_LOGO`, `LOGIN_SPLASH_IMAGE` | `LoginScreen_bps_C` | `0x700`, `0x8F0` (4.4, later) |
| `CREDITS_PHOENIX` | `bpw_Credits_C` | `0x3F8` |
| `SECTION_SHOW_TITLE`, `SECTION_TITLE_TEXT`, `SECTION_COLUMNS` | `UArchonCreditsSection` | `0x3A8` (`u8`), `0x3B0`, `0x3B8` |
| `PRESS_START_PANEL` | `PressStartScreen_bps_C` | `0x4D0` |
| `PANEL_WELCOME_TEXT` | `PressStartPanel_bpw_C` | `0x3D0` |
| `ENGINE_CONSOLE_CLASS`, `ENGINE_GAME_VIEWPORT` | `UEngine` | `0xF0`, `0x780` |
| `VIEWPORT_CONSOLE` | `UGameViewportClient` | `0x40` |
| `INPUT_CONSOLE_KEYS`, `FKEY_SIZE` | `UInputSettings`, `FKey` | `0x130`, `0x18` |

### 8.2 Functions and parameter blocks

| Class.Function | Block (bytes) | Parameters (offset) |
| --- | --- | --- |
| `Widget.SetVisibility` | 0x01 | `InVisibility` u8 (0) |
| `TextBlock.SetText` | 0x18 | `InText` FText (0) |
| `TextBlock.SetFont` | 0x58 | `InFontInfo` (0) |
| `TextBlock.SetColorAndOpacity` | 0x28 | `InColorAndOpacity` (0) |
| `TextBlock.SetShadowOffset` | 0x08 | `InShadowOffset` (0) |
| `TextBlock.SetShadowColorAndOpacity` | 0x10 | `InShadowColorAndOpacity` (0) |
| `TextLayoutWidget.SetJustification` | 0x01 | `InJustification` u8 (0); `Center` = 1 |
| `PanelWidget.AddChild` | 0x10 | `Content` (0), `ReturnValue` slot (8) |
| `ScrollBoxSlot.SetPadding`, `SetHorizontalAlignment`, `SetVerticalAlignment` | 0x10, 0x01, 0x01 | the value (0) |
| `WidgetBlueprintLibrary.Create` | 0x20 | `WorldContextObject` (0), `WidgetType` (8), `OwningPlayer` (0x10), `ReturnValue` (0x18) |
| `GameplayStatics.SpawnObject` | 0x18 | `ObjectClass` (0), `Outer` (8), `ReturnValue` (0x10) |
| `KismetTextLibrary.Conv_StringToText` | 0x28 | `InString` FString (0), `ReturnValue` FText (0x10) |
| `KismetStringLibrary.Conv_StringToName` | 0x18 | `InString` FString (0), `ReturnValue` FName (0x10) |
| `KismetInternationalizationLibrary.GetCurrentCulture` | 0x10 | `ReturnValue` FString (0) |
| `LoginScreen_bps_C.OnPressToStartDismissed` | 0x04 | `UserIndex` i32 (0) |
| `ArchonPartyClient.StartTravelToCity` | 0x01 | `bLeaveWithParty` bool (0) |
| the Play click event (`w_loginbutton` … `Clicked`) | 0 | none |
| later (4.4): `KismetRenderingLibrary.ImportBufferAsTexture2D` | 0x20 | `WorldContextObject` (0), `Buffer` TArray (8), `ReturnValue` (0x18) |
| later (4.4): `Image.SetBrushFromTexture` | 0x10 | `Texture` (0), `bMatchSize` bool (8) |

Static library functions are called on their class's default object (3.4 `static_call`).

---

## 9. Calls, departures, ambiguities and settled questions

Calls made here:

1. New id `CON-1` (kind `Setting`, slice R17) for the console; patch 0044 gates the AGPL's console on it.
2. The reflection kit is general and shared with R15; whichever lands first creates `reflect.rs`.
3. The parameter guard: a call is skipped when the function's `ParmsSize` exceeds the block R17 laid out.
4. R17 calls the game through the hooked `ProcessEvent` entry, so the whole chain sees its calls, as with the AGPL.
5. `FText`s made once and never released.
6. The patch-notes version lines are collapsed without the background; the logo stays until an MIT image carries the
   name (RT-9).
7. A future background is embedded with `include_bytes!`, no Windows resource.
8. Credits match `FunctionName("OnInitialized")` exactly, then the class `bpw_Credits_C`.
9. `AUTO_PRESS_START` stays flag-only (not tied to the development profile); its matcher is narrowed to the two
   events it acts on.
10. The party-client scan runs at most once per poll.
11. The console is made on the game thread from an SP-1 after handler, checked every 250 ms.
12. Texts in `en` and `pt-BR`, chosen by the game's culture (`pt*` → `pt-BR`); the engine's culture string is not freed.
13. The credits name `Silent Spud` and credit the earlier projects by name only (question R17-Q1).

Departures from the AGPL (each removes a crash path or a double; none changes what a player sees except the texts):

1. Text style through the game's setters; the colour is always copied.
2. Slot alignment and padding through the slot's setters.
3. The console timer starts at the viewport, not the world.
4. An empty console-key array is left alone.
5. An existing viewport console is kept.

Ambiguities:

- `UFunction.ParmsSize` at `+0xB6` is stock UE 4.26's layout (the dump pads that region); the play-test's log shows
  whether the guard lets the calls through (11 step 2). If it does not, the offset is wrong and every R17 call is
  skipped, failing closed.
- The 1.14.7 Brazilian localisation's word for "Slayer" was not checked.

Settled here:

- **RT-9:** the background is not ported (4.4); the stock season art and logo show. The Paradox image goes when MIT
  owns `SP-1@R17`.
- **RT-14 for R17:** all four flags are kept. `DISABLE_LOGIN_BRANDING` (read once; today it covers only the version
  lines), `DISABLE_CREDITS_BRANDING` (read once), `DISABLE_PRESS_START_BRANDING` (read once), `AUTO_PRESS_START`
  (probed per matching call). None is shipped to players.
- **RT-17 for R17:** R17 installs no hook; its handlers register in Init before the freeze. No order dependency.
- **RT-18 for R17:** every matcher is stated with its kind and what it catches (4.2, 4.5, 4.6, 5.1, 6.2).
- RT-12, RT-13, RT-15 and RT-16 are not R17's.

Open question for the maintainer:

- **R17-Q1. The credits' names.** Section 7 shows `Silent Spud` / `Dainty Lass` and "The Undaunted and Mystic
  Paradox communities" / "The servers it grew from". Keep that, name upstream people (which, and with what roles), or
  show only Dainty Lass? Changing it is a text change in `branding.rs` only.

---

## 10. Tests

### 10.1 In CI (core crate on every platform)

`cargo test -p rustic-isles-runtime-core`:

- `reflect.rs` over a fake memory (R0's `Memory` test buffer) holding a small synthetic class graph:
  - `class_chain` stops at null, at an unreadable pointer and at 64 steps (a cycle);
  - `is_a` true for the class and each super, false otherwise;
  - `find_function` finds a function on a super, ignores a non-function field of the same name, stops at 8 192 fields;
    `find_function_matching` looks at the own class only;
  - `find_class` takes a `Class` and a `…GeneratedClass` meta-class and refuses an object of another class with the
    name; `default_object` refuses a dead pointer; `first_instance` skips default objects.
  - `params_fit`: equal, smaller, larger, unreadable `ParmsSize`; buffer sizing `max(round_up(n, 8), 16)`.
- `branding.rs`:
  - both language tables have the same keys, no empty value; the culture rule: `pt-BR`, `pt-PT`, `PT`, `en-US`, empty
    and unreadable;
  - every R17 matcher on the full names it must catch and must not: `Function LoginScreen_bps.LoginScreen_bps_C.
    OnStartupMovieFinished`, `...OnPlayScreenPollTimer`, `Function UMG.UserWidget.OnInitialized`,
    `Function bpw_Credits.bpw_Credits_C.PreConstruct`, `Function PressStartScreen_bps.PressStartScreen_bps_C.Construct`,
    and that **none** matches a function R17 itself calls (`Function UMG.TextBlock.SetText`, `Function UMG.Widget.
    SetVisibility`, `Function UMG.PanelWidget.AddChild`, `Function Engine.GameplayStatics.SpawnObject`, the others of
    8.2) except the console's catch-all, which is an after handler that does no work once done;
  - `reorder(slots, from, to)`: last to first, last to middle, `from == to`, out of range refused;
  - the auto-login counters: press start once; Play and travel from the 4th poll, once each; the every-10th log;
  - `ConsoleSchedule`: no action before the viewport is seen, at 2 999 ms, action at 3 000 ms, the 250 ms throttle,
    done stays done.
- The registry test (R0 10.1) now expects `CON-1` with slice R17; `owns` true for `SP-1@R17` and `CON-1` with
  `LANDED` holding R17.

DLL crate (Windows runner): it builds, and its export and import checks (R0 10.1) still pass. Nothing in R17 can be
exercised without the game.

**Never in CI:** every call into the game, the screens, the credits, the console (surface section 9).

### 10.2 The AGPL patch 0044

It needs the host's SDK to build, so no CI builds it. The maintainer's steps are 11.2.

---

## 11. The maintainer's hands-on checks

### 11.1 R17's play-test

Surface section 8's R17 row, amended by RT-9: "credits; F2 opens the console" and the press-start text; **no login
background check** (dropped until an MIT image exists).

1. Build the AGPL runtime with patches 0041 and 0044 (11.2) and the MIT DLL with R17; inject both
   (`rustic-isles-runtime.dll` first) as in R0's 11.1 step 1.
2. Start the client. In `rustic_isles_dll_port0.log`: `[MitRuntime] ... owns=<R0+R1 count + 2> ids`, and no
   `[MitBranding] ...: parameter size ... call skipped` line (if there is one, R17's `ParmsSize` offset is wrong;
   report it with the line).
3. **Press start.** The popup shows the Dainty Lass welcome text (section 7), in English; with the game in Brazilian
   Portuguese, the `pt-BR` text. Log: `[MitPressStart] welcome text set after ...`.
4. **Login screen.** The season art and the DAUNTLESS logo show as in the stock game. The patch-notes preview and popup
   show no "Update ..." version line. Log: `[MitBranding] collapsed ...`. The AGPL's own login-branding lines are
   absent.
5. **Credits.** Open the credits from the login screen: the Dainty Lass block (title, subheading, disclaimer, names and
   roles) sits above Phoenix Labs, styled like it, and stays after scrolling and reopening. Log:
   `[MitCredits] ... placed 4/4 ...` and `... section present`.
6. **Console.** About 3 s into the login screen, press F2: the engine console opens; type `stat fps` and see it work.
   Logs: `[MitConsole] console on F2 (...)` and the AGPL's `[MitSeam] CON-1 owned by rustic-isles-runtime.dll ...`.
7. **Flags.** With `DISABLE_PRESS_START_BRANDING.flag`, `DISABLE_CREDITS_BRANDING.flag` and
   `DISABLE_LOGIN_BRANDING.flag` next to the executable, restart: the stock popup text, stock credits, and the version
   lines show. Remove them.
8. **Scripted login.** With `AUTO_PRESS_START.flag`, start the client and touch nothing: it passes the popup, presses
   Play and travels to Ramsgate. Logs: three `[MitAutoLogin]` lines. Remove the flag.
9. **Regression without the MIT DLL.** Remove `rustic-isles-runtime.dll`: the AGPL's branding and console behave as
   before 0041 and 0044.

Only CI passed until these steps are done.

### 11.2 Building and checking patch 0044

As R0's 11.2: build `Rustic-Isles.dll` with the host's SDK (`_build.bat`), check its imports, and play steps 6 and 9
above. Publish both DLLs together.

---

## 12. Size

Rust without tests: the core crate about 450 lines (`reflect.rs` 200, `branding.rs` 250 with the texts), the DLL crate
about 650 (`ffi/reflect.rs` 120, login 60, credits 220, press start 60, auto login 100, console 90). Tests about 400.
The background, when added, is about 120 more and the image. Patch 0044 is five lines of C++ and one table row.
