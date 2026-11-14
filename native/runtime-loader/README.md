# rustic-isles-loader

The runtime loader: a `winmm.dll` that sits beside `Dauntless-Win64-Shipping.exe`. When the game loads it, it loads
the Rustic Isles runtime, `rustic-isles-runtime.dll`, from the same folder by its fixed name, then any extra DLLs
listed in `mystic_loader.ini`. It still knows the AGPL runtime's names (the newest of `Rustic-Isles.dll`,
`MysticParadox.dll` and `MystPaxInternalServer.dll`) for a set from before R21; an R21 set has none of them, and the
launcher removes one it finds (`docs/port/spec-runtime-r21.md`, L-3 and L-6). Every `winmm.dll` and
`version.dll` function it exports forwards to the real one in the Windows system folder. The behaviour is specified
in `docs/port/spec-runtime-loader.md`.

On a game the launcher started with `DAINTY_RUNTIME_INJECTED=1`, it loads nothing. On Windows the launcher loads
the runtime into the game itself, so a host whose servers use this loader can play from the same folder.

## Build

```powershell
cargo build --release --target x86_64-pc-windows-msvc
```

The DLL lands at `target/x86_64-pc-windows-msvc/release/winmm.dll`, where `dainty-host publish` and the launcher
look for it. `cargo test` runs the unit tests and loads the debug build to check its exports against the system DLLs.

## `mystic_loader.ini`

Optional, beside the loader. One DLL per line, loaded in order after the runtime; `;` and `#` start a comment,
`[section]` lines are skipped and `key = value` lines use the value. A relative name is taken from the loader's
folder; the runtime names are ignored, so the ini can't load a second runtime.

```ini
[mods]
dll1 = extra.dll
```

**Every DLL the ini lists runs inside the game with the player's rights.** List only DLLs you trust; the launcher
refuses to start the game while an ini or DLL it doesn't know sits next to the loader.

Licensed under MIT (`LICENSE`).
