# rustic-isles-exporter

`CatalogExporter.dll`, re-created clean-room as MIT from [`docs/port/spec-catalog-exporter.md`](../../docs/port/spec-catalog-exporter.md)
(RD27). The runtime loader loads it into the game (`mystic_loader.ini`, RD9); it waits for the live catalog, writes
the catalog and the tables the Rustic Isles generators read into `Items_Analysis`, then beeps. Phase 1 carries the
catalog, equipment, Slayer's Path, hunts, store runtime, progression and mastery exports, the run manifest and the
status file, and the weapon slots (spec §6.3), whose `defaultWeaponPartId`s give the 1.14.7 module its starter
weapons' default parts (ruling V-4). A flags file that turns on a research export (cells, combat, skins, drop
tables, currency names, table inventory) gets `EXPORT_<X>=1 is not supported by this build, skipping`.

Two crates, outside both workspaces:

- `.` (`rustic-isles-exporter`, lib `catalog_exporter`): flags, paths, the readers, the exports, the output layer.
  No `unsafe`; its tests run on any OS against a synthetic memory image.
- `dll/` (`catalog-exporter-dll`, lib `CatalogExporter`): the `cdylib`. `dll/src/sys.rs` is the only `unsafe`:
  `DllMain`, `ReadProcessMemory` on the game's own process, and the `ProcessEvent` call.

## Build

The layouts come from the host's own Dumper-7 SDK at build time (spec §9); nothing from it is committed. Point
`DAINTY_SDK_DIR` at the folder that holds the dump's `.hpp` files (searched six levels deep):

```powershell
$env:DAINTY_SDK_DIR = "$PWD\..\..\.workspace\sdk"   # where the Host page's Import SDK puts it
cargo build --release --target x86_64-pc-windows-msvc -p catalog-exporter-dll
```

The DLL lands at `target/x86_64-pc-windows-msvc/release/CatalogExporter.dll`. Without `DAINTY_SDK_DIR` the build
uses `fixtures/synthetic-sdk` (made up, for the tests) and the DLL only writes `built without DAINTY_SDK_DIR` to its
status file. `RUSTIC_ISLES_EXPORT_ROOT`, also read at build time, moves the flags file and the output under a root
(spec §3.3: `<root>\native\catalog-exporter\export_flags.txt`, `<root>\Items_Analysis`); a host's build leaves it
unset.

**First build against a real SDK.** `src/facts.rs` names every member, size, enum value and offset the exporter
needs, with the SDK paths the spec gives (§6). If the first build stops with `the SDK lacks N fact(s)`, report only
the fact ids it lists for a spec correction, then fix the path in `facts.rs` from the corrected spec; never paste SDK
text into the repository. The weapon-slot facts (`WEAPON_FACTS`) are guessed spellings the spec does not name, so
one the SDK lacks does not stop the build: it is a `cargo` warning naming the fact, and the DLL writes that file
empty with a status line naming it. Report those ids the same way.

## Tests

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Golden-byte tests of every phase-1 file, the flags, the manifest, the run order and the timeout run on the synthetic
image (`tests/`). Nothing here runs against the real 1.14.7 client: that is the check below.

## The maintainer's check: old and new on the real client

Spec §10 has the whole procedure; these are its steps for this crate. Run them on the PC with the game, the Host
page, the imported SDK and a local server.

1. Build the old DLL with root `C:\dl-diff\old` (spec §10 step 1, unchanged).
2. Build this one with root `C:\dl-diff\new`, from the workspace root:

   ```powershell
   $env:RUSTIC_ISLES_EXPORT_ROOT = 'C:\dl-diff\new'
   $env:DAINTY_SDK_DIR = (Resolve-Path .workspace\sdk).Path
   cargo build --release --target x86_64-pc-windows-msvc --manifest-path native\rustic-isles-exporter\Cargo.toml -p catalog-exporter-dll
   Copy-Item native\rustic-isles-exporter\target\x86_64-pc-windows-msvc\release\CatalogExporter.dll C:\dl-diff\CatalogExporter-new.dll
   Remove-Item Env:RUSTIC_ISLES_EXPORT_ROOT
   ```

3. Write the flags file under both roots (spec §10 step 3). The new one refuses the research flags, so compare only
   the phase-1 files.
4. Runs `old-1`, `old-2` and `new-1`, the same way and in the same game state (spec §10 step 4).
5. Compare with `Compare-Run` (spec §10 step 5): `old-1`/`old-2` gives the noise floor, `old-1`/`new-1` must be the
   same or better for every phase-1 file. Check `exportedAt`, `extracted` and CRLF as the spec says.
6. Put the old DLL back and keep `C:\dl-diff` private (spec §10 step 6). Report only file names, verdicts and row
   counts.

What the diff settles (spec §12): the name suffix and UTF-16 handling, the store offsets, member types, the soft
pointer layout, and whether a run loads the tables in time.

Licensed under MIT (`LICENSE`).
