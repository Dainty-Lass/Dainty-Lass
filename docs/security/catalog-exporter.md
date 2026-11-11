<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: the catalog exporter (RX3)

The checklist for `native/rustic-isles-exporter`, the MIT clean-room rewrite of `CatalogExporter.dll`
([`docs/port/spec-catalog-exporter.md`](../port/spec-catalog-exporter.md), RD27, port plan item 8). `SECURITY_REVIEW.md`
indexes it.

How to use it: before the MIT DLL replaces the old one on the Host page, copy this list into the switch's commit or
review, check each item in the code (not from memory), link the code or test that shows it, and sign at the end
with the date and the commit reviewed. An item that does not hold is a bug to fix before the switch, not a box to
leave open.

## What it is and where the boundary is

The exporter runs inside the host's own game process, with the host's rights, and reads that process's memory. It
listens on nothing and sends nothing. Its trust boundary is the game process itself: everything it reads is
game state that may be half-built, freed or changing while it reads, so no pointer or count is trusted.

- **In:** the game's memory (the object array, the name pool, objects, data tables), the flags file
  (`export_flags.txt` under the build's export root, else the game's working folder), and at build time the host's
  Dumper-7 SDK (`DAINTY_SDK_DIR`).
- **Out:** files under `Items_Analysis` (fixed names, spec §6), the appended status file, the debugger output, and
  one call into game code (`GetAllItems` through `ProcessEvent`).
- **Who runs it:** the runtime loader, when the Host page's export session lists it in `mystic_loader.ini` (RD9), or
  a host by hand on their own client. The launcher never injects it.

Code: `native/rustic-isles-exporter/src/` (`engine.rs`, `memory.rs`, `flags.rs`, `paths.rs`, `status.rs`, `json.rs`,
`run.rs`, `export/`, `sdk_parse.rs`, `facts.rs`), `build.rs`, and the DLL's `dll/src/sys.rs`.

Fuzz target in `fuzz`:

- `catalog_exporter_input`: the flags parser gives the same flags for the same bytes and a later line wins; the SDK
  extractor resolves every fact or names what is missing, never panics; escaping leaves no raw control byte; the
  name, string, text, array, map and weak-pointer readers over memory of any content read or come back empty.

## Checklist

### Running in the game

- [ ] All `unsafe` is in `dll/src/sys.rs`, each block with a `// SAFETY:` comment; the library is
      `#![forbid(unsafe_code)]`.
- [ ] `DllMain` disables thread-library calls, starts one worker thread and returns at once; nothing on detach, no
      hooks, never unloaded (spec §3.1).
- [ ] The worker runs under `catch_unwind` with `panic = "unwind"`: a panic ends the export, not the game.
- [ ] It writes no game memory. The parameter block it hands `ProcessEvent` is its own buffer; the function's flags
      are not touched.
- [ ] The only call into game code is `UArchonCatalog::GetAllItems` (spec §4.2). Weapon slots, cells and the
      string-table functions are not carried; their flags are refused (`run.rs`, `Step::Refused`).
- [ ] Every read goes through `Memory::read`; in the game that is `ReadProcessMemory` on the own process, so an
      unmapped or freed page fails the read instead of faulting. A row with a failed read is skipped with a status
      line; a failed string, name or text read gives empty text (spec §4.4, §12 call 4).
- [ ] Counts and chains are bounded: objects 4,000,000, array elements 1,000,000, string units 1,048,576, name
      blocks 8,192, `Super` hops 256, table rows below 200,000; address arithmetic wraps rather than overflows
      (`engine.rs`; `tests/run.rs` `pointers_at_the_top_of_memory_read_as_faults`).
- [ ] A DLL built on the synthetic SDK exports nothing (`sys.rs`, `SDK_SYNTHETIC`).

### Files

- [ ] The only file it reads is the flags file; unknown keys are ignored, a value is on only as `1`, `true` or
      `TRUE` (`flags.rs`).
- [ ] Every file name it writes is a constant; no name or path read from the game becomes part of a path
      (`export/`, `run.rs`).
- [ ] Output folders are created one level at a time, never recursively (`paths.rs`).
- [ ] The status file holds ids, counts and table names only. The exporter handles no secret.
- [ ] Its output is the host's game data: never committed (AGENTS.md), and the diff folder of spec §10 stays private.

### Build

- [ ] `build.rs` reads only `.hpp` files under `DAINTY_SDK_DIR` (six levels deep) and writes only `OUT_DIR`; the
      generated layout holds numbers (offsets, sizes, enum values), never SDK text, and is never committed.
- [ ] A missing fact stops the build and names each one (`build.rs`, `fail`).
- [ ] `fixtures/synthetic-sdk` is made up (its first line says so) and holds no layout from a game.

## Sign-off

Reviewed at commit `________` on `________` by `________`.
