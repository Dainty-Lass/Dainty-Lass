# Mod Settings: one settings page that client mods share

Status: **design, nothing built.** Asked for by the maintainer on 2026-11-20. Needs the decisions in section 9
before any row starts.

## 1. Goal

A client mod (a UE4SS Lua or C++ mod, or a native DLL of its own) adds its options to a **Mod Settings** tab in the
game's settings menu by calling the runtime, instead of injecting its own UI. The runtime draws the page, stores the
values and tells the mod when one changes.

Not in scope:
- Server-side settings, or sending a mod's values to the backend. The page is client-only, and the runtime refuses
  every call in a `-server` process.
- Loading mods. The runtime offers the API; UE4SS (or the mod's own loader) loads the mod. Section 7 covers how
  UE4SS gets into a launcher-started game.
- A mod browser, installer or update check.

## 2. The parts

```text
Lua mod ──► DaintyLass global ──► UE4SS bridge mod ──┐
                                (native/ue4ss-bridge) │ GetProcAddress
UE4SS C++ mod / any native DLL ───────────────────────┼──► rustic-isles-runtime.dll
                                                      │      dainty_mod_api(version)
                                                      │      ├─ model: pages, options, values
                                                      │      ├─ store: one JSON file per mod
                                                      │      └─ page: rows built in the settings menu
```

1. **The C ABI**, exported by `rustic-isles-runtime.dll` (section 3). This is the only contract. Everything else is
   a client of it.
2. **The bridge**, a UE4SS C++ mod that exposes the ABI to Lua (section 4). UE4SS Lua has no FFI, so Lua can't call
   the export directly.
3. **The store** (section 5) and **the page** (section 6), inside the runtime.

The model and the store go in `rustic-isles-runtime-core` (pure, tested with `cargo test`). The export and the widget
code go in `rustic-isles-runtime` (`ffi/mod_settings.rs`, `ffi/mod_settings_page.rs`). The bridge is C++ in
`native/ue4ss-bridge`, a project of its own outside both workspaces like `native/runtime-loader`. All of it is new
MIT code: nothing is ported, so no clean-room spec is needed.

## 3. The C ABI

One export. The mod asks for the version it was built against and gets the table, or null if the runtime is older,
or this is a server.

```c
#define DAINTY_MOD_API_VERSION 1

typedef struct { const uint8_t *ptr; size_t len; } DaintyStr;          /* UTF-8, not NUL-terminated */

typedef enum {
  DAINTY_OK = 0, DAINTY_BAD_ARG, DAINTY_DUPLICATE, DAINTY_UNKNOWN_KEY,
  DAINTY_WRONG_TYPE, DAINTY_CLOSED, DAINTY_INTERNAL
} DaintyStatus;

typedef enum { DAINTY_BOOL, DAINTY_INT, DAINTY_FLOAT, DAINTY_CHOICE, DAINTY_KEYBIND } DaintyKind;

typedef struct {
  DaintyKind kind;
  union { uint8_t b; int64_t i; double f; DaintyStr choice; DaintyStr key; } v;  /* choice: option id; key: FKey name */
} DaintyValue;

typedef struct DaintyPage DaintyPage;                                  /* opaque, owned by the runtime */
typedef void (*DaintyOnChange)(void *user, DaintyStr key, const DaintyValue *value);

typedef struct {
  uint32_t version;                                                    /* the runtime's version */
  uint32_t size;                                                       /* sizeof this table: fields are append-only */
  DaintyStatus (*open_page)(DaintyStr mod_id, DaintyStr title, DaintyPage **out);
  DaintyStatus (*close_page)(DaintyPage *page);
  DaintyStatus (*add_bool)(DaintyPage *, DaintyStr key, DaintyStr label, uint8_t def);
  DaintyStatus (*add_int)(DaintyPage *, DaintyStr key, DaintyStr label, int64_t min, int64_t max, int64_t step,
                          int64_t def);
  DaintyStatus (*add_float)(DaintyPage *, DaintyStr key, DaintyStr label, double min, double max, double step,
                            double def);
  DaintyStatus (*add_choice)(DaintyPage *, DaintyStr key, DaintyStr label, const DaintyStr *ids,
                             const DaintyStr *labels, size_t count, size_t def);
  DaintyStatus (*add_keybind)(DaintyPage *, DaintyStr key, DaintyStr label, DaintyStr def);
  DaintyStatus (*add_heading)(DaintyPage *, DaintyStr label);
  DaintyStatus (*get)(DaintyPage *, DaintyStr key, DaintyValue *out);  /* strings in out live until the next set */
  DaintyStatus (*set)(DaintyPage *, DaintyStr key, const DaintyValue *value);
  DaintyStatus (*subscribe)(DaintyPage *, DaintyStr key, DaintyOnChange cb, void *user);  /* empty key: every key */
} DaintyModApi;

__declspec(dllexport) const DaintyModApi *dainty_mod_api(uint32_t want_version);
```

Rules:
- **Versions.** A new function goes at the end of the table and raises `version`. A mod reads only fields that
  `size` covers. A change that breaks a field is a new export (`dainty_mod_api2`), never a reuse.
- **Ids.** `mod_id` and `key` match `[a-z0-9_.-]{1,64}`: the mod id is a file name (section 5). A second
  `open_page` for an open id is `DAINTY_DUPLICATE`.
- **Labels** are text the mod supplies, shown as given. A label may be a locale table: `en|My option\npt-BR|Minha
  opção`, where the runtime picks the game's culture and falls back to `en`, then to the first entry.
- **Threads.** Every function may be called from any thread; the model sits behind one mutex. Callbacks run on the
  game thread, after the player changes a row or after a `set`, never while the mutex is held, so a callback may call
  `get` and `set`.
- **Unload.** `close_page` drops the page's callbacks before it returns. A mod calls it before its DLL unloads
  (UE4SS hot reload), so no callback runs into freed code. Values stay in the store.
- **Failure.** Every export runs inside `catch_unwind`, as the detours do. A panic becomes `DAINTY_INTERNAL` and a log
  line, never an unwind across the boundary.
- **What it never exposes:** the account key, the backend address or token, the player's identity, or any game
  pointer. The API takes no callbacks for anything but value changes.

## 4. The UE4SS bridge and the Lua surface

`native/ue4ss-bridge` builds `Mods/DaintyLass/dlls/main.dll`, a UE4SS C++ mod. In `on_lua_start` it finds
`rustic-isles-runtime.dll` with `GetModuleHandleW` (it never loads it), asks for version 1, and registers a
`DaintyLass` global in each Lua state. Without the runtime (vanilla game, a server) the global is still there and
`DaintyLass.Available()` is false, so a mod can fall back.

```lua
if DaintyLass.Available() then
  local page = DaintyLass.Settings.Page("my-mod", { en = "My Mod", ["pt-BR"] = "Meu Mod" })
  page:Heading("Movement")
  page:Toggle("enabled", "Enabled", true)
  page:Slider("speed", "Speed", { min = 0.5, max = 2.0, step = 0.1, default = 1.0 })
  page:IntSlider("count", "Count", { min = 1, max = 10, default = 3 })
  page:Choice("mode", "Mode", { { "fast", "Fast" }, { "safe", "Safe" } }, "safe")
  page:Keybind("toggle", "Toggle key", "F6")
  page:OnChange("enabled", function(value) print("enabled is now", value) end)
  local speed = page:Get("speed")
end
```

- The bridge keeps each Lua state's pages and calls `close_page` for them in `on_lua_stop`, so a Lua hot reload
  re-registers cleanly.
- A Lua callback runs where UE4SS allows Lua to run. The bridge queues the change from the runtime's callback and
  runs the queue on UE4SS's game-thread hook. It never calls into Lua from the runtime's callback.
- The UE4SS C++ mod ABI is a C++ class tied to one UE4SS release. The bridge is built against one pinned UE4SS
  version (MIT; its headers fetched at build time, never committed) and shipped with it. The runtime's C ABI doesn't
  change when UE4SS does.

## 5. The store

One file per mod: `<game>\Dauntless\Saved\DaintyLass\ModSettings\<mod_id>.json`, a flat object of key to value,
written as `JSON.stringify` would (no spaces) so a person can read and diff it.

- Read when the page opens. A value of the wrong kind, out of range or not one of the choices falls back to the
  default, with a log line naming the mod and key.
- Keys no option claims are kept on write, so removing a mod for a while, or a mod version that drops an option,
  loses nothing.
- Written on change, at most once a second per mod, through a temp file and a rename. Written again on exit.
- Under the game's `Saved` folder so it follows the install and works the same under Proton. Nothing goes to the
  launcher, the content server or the backend.

## 6. The page

The tab is built at runtime, from the game's own settings-row widget classes, so it looks like the rest of the menu:
a tab button beside the game's tabs, and a scrolling list with one row per option (toggle, slider, dropdown, key
capture, heading). It appears only when at least one page is open.

What isn't known yet, and needs the SDK generated on the maintainer's machine (`docs/sdk-and-game-data.md`):
- The settings menu's widget class, how it builds its tabs, and the function that switches between them.
- The row classes for each kind, how a row is built and given its label and value, and the event it fires on a
  change.
- Whether a key-capture row exists, or the keybind row has to be a button that listens for the next key.

That's a spike (section 8, M3) with a kill criterion: if rows can't be built from the game's classes without cooked
assets, the page falls back to one plain UMG list the runtime draws itself, and the ABI doesn't change.

Text the runtime shows itself (the tab's title "Mod Settings", "No mods have settings", reset buttons) comes from a
catalog in `en` and `pt-BR` (DU2), with a code-to-text table in the runtime like the launcher kit's `core::i18n`.

## 7. UE4SS in a launcher-started game (amends RD9)

The launcher injects only DLLs it chose, by absolute path, with checked hashes, into a game it just started
suspended (`docs/launcher-injection.md`). UE4SS usually installs as a proxy DLL (`dwmapi.dll`) in `Win64`, which
loads it outside that rule. Today PLAY doesn't name that DLL, so whether it's refused depends on the leftover
check, and that's not a decision anyone made.

Proposed: **an opt-in "Enable UE4SS mods" switch on the Settings page** (off by default):
1. On, the launcher injects `UE4SS.dll` from `Win64\ue4ss\` after the runtime, as one more DLL in the plan, with the
   same all-or-nothing rule. The runtime goes first, so the bridge always finds it.
2. Its hash is checked against an allowlist of UE4SS releases the launcher knows. An unknown hash refuses PLAY, with
   a dialog naming the file and the version expected. Mods inside UE4SS are the player's own choice and aren't
   hashed.
3. A UE4SS proxy DLL (`dwmapi.dll` that is UE4SS's) in `Win64` refuses PLAY whether the switch is on or off, as the
   unmarked `winmm.dll` does: it would load UE4SS a second time, or load it without the player's choice.
4. Off, nothing about UE4SS is loaded or checked beyond point 3.

Security notes for the checklist (`docs/security/mod-settings.md`, written with M1):
- A mod runs with the game's rights whether or not this API exists. The API grants no new reach: no key, no token,
  no pointers (section 3).
- The trust change is point 1, loading UE4SS at all, which is why it's opt-in and hash-pinned.
- The store parser is the one untrusted input the runtime adds (a player may edit the files), so it gets a fuzz
  target in `fuzz/`.

**Hooks.** UE4SS hooks `ProcessEvent` and other engine functions with PolyHook. The runtime's broker
(`ffi/broker.rs`) owns one MinHook hook per target and only ever rewrites its stub's pointer cell, so it never
unhooks. With the runtime first, UE4SS hooks over the broker's hook, and both chains run. The hands-on check in M4
confirms it on the 1.14.7 client.

## 8. Rows

| Row | Work | Done when |
| --- | --- | --- |
| M1 | The model, value checks and the store in `rustic-isles-runtime-core`. The `dainty_mod_api` export and its table in `rustic-isles-runtime`, with no page yet. The store's fuzz target, the security checklist, a C header (`native/include/dainty_mod_api.h`) checked against the Rust table by a test | `cargo test` covers registration, every refusal, versioning, the store's round trip and recovery; a test DLL calls the export through `GetProcAddress` |
| M2 | `native/ue4ss-bridge`: the Lua global, the per-state page list, the callback queue. An example Lua mod in `native/ue4ss-bridge/example/` | It builds against the pinned UE4SS. Hands-on: the example mod's values round-trip through the store |
| M3 | Spike, hands-on: the settings menu's classes from the SDK (section 6). Then the page in `ffi/mod_settings_page.rs` | Hands-on: the tab shows, every kind of row works, a change reaches the mod and the file. Or the kill criterion's fallback |
| M4 | The launcher's switch, UE4SS's hash allowlist, the proxy-DLL refusal, the catalog strings (en and pt-BR) | Launcher tests for each refusal. Hands-on: PLAY with the switch on loads UE4SS once, and the runtime's hooks and UE4SS's both run |

M1 and M2 can run in parallel once the header is fixed. M3 needs the maintainer's machine. M4 needs the RD9 decision.

## 9. Decisions for the maintainer

1. **RD9:** the opt-in switch and hash allowlist in section 7, or another way (accept a player-picked UE4SS with an
   "unverified" warning, or don't support UE4SS in launcher-started games at all, so mods use the ABI from their own
   loader).
2. **Which UE4SS release** to pin first, and who updates the allowlist when UE4SS ships one.
3. **Where the bridge ships:** in the client feed next to the runtime (offered to every player), or as a separate
   download a player adds to `ue4ss\Mods` themselves.
4. **The store's folder:** the game's `Saved` folder (proposed), or the launcher's data folder, which would keep
   values across reinstalls but needs the path passed to the runtime.
5. **Value kinds for v1:** the five in section 3, or also free text and colour.
