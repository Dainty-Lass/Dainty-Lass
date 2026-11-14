<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Security checklist: module libraries and the plugin host (RD27)

The checklist for loading a game module or a service as a library the core loads at start: the C ABI and the
library side in `crates/dainty-module-api/src/plugin.rs`, the loader in `crates/dainty-plugin-host`, and the registry's choice of
driver (`crates/dainty-controller/src/registry.rs`, `bring_up`). The frame protocol a library speaks is `module-host.md`'s;
everything that checklist says about what a module sends holds here unchanged. `SECURITY_REVIEW.md` indexes it.

How to use it: before the first module library ships to hosts, copy this list into the cutover commit, check each
item in the code (not from memory), link the code or test that shows it, and sign at the end with the date and the
commit reviewed.

## What it is and where the boundary is

A library runs in the core's process, so unlike a child it can do anything the core can: read its memory, its keys,
its files. Loading one is a decision to trust its code fully. The boundary this checklist guards is therefore not the
library's behaviour but **which file gets loaded**, and **that a library which is broken or out of date fails closed**
rather than corrupting the core.

- **In, the file:** the path `settings.toml` names for an external module whose extension is `.dll`, `.so` or `.dylib`,
  with its SHA-256 (DD5). Nothing else is loaded.
- **In, the entry point:** `dainty_plugin_v1`, returning a `PluginV1` vtable with an ABI version.
- **In, the library's bytes:** through `HostSink::send`, the same hello and frames a child writes to stdout.
- **Out, the core's bytes:** through `PluginV1::send`, the same welcome, calls and replies a child reads on stdin.
- **Out, nothing else:** no pointer to a Rust type, no allocation to free, no callback into the core's own code but
  the two sink functions.

Fuzz target: `plugin_bytes` in `fuzz` (a library's bytes in any chunking, through `ChannelReader` and `read_frame`:
the same frames or the same refusal however they are split). The frames themselves are `module_frame`'s.

## Checklist

### The file

- [ ] The file is hashed through a handle opened with `module_api::host::open_pinned`, and loaded while that handle is
  held, so on Windows nobody can write, rename or delete it between the check and the load. On Linux it is loaded
  through `/proc/self/fd`, so a file renamed over it is not the one loaded (`plugin_host::load`).
- [ ] A file whose SHA-256 differs from `settings.toml`'s is never loaded: no initialiser runs
  (`a_library_that_does_not_match_its_pin_is_never_loaded`).
- [ ] A file that is not a library, or exports no `dainty_plugin_v1`, or returns null, or speaks another ABI version,
  is refused with a reason and nothing of it runs after (`a_file_that_is_not_a_module_library_is_refused`).
- [ ] The folder module libraries live in is the host's alone, as for executables (`module-host.md`): a library a
  user can replace is code that user runs as the server.

### The boundary

- [ ] Every byte crossing is copied by the side it is handed to before the call returns (`sink_send`,
  `module_api::plugin::send`); no buffer is freed by the other side's allocator.
- [ ] Every entry point the core calls into and every sink function the library calls into catches panics, so none
  unwinds across `extern "C"` (`catch_unwind` in `plugin.rs` and `plugin-host`).
- [ ] A library's bytes are read by the same `read_frame`, at the same limits, with the same handshake checks, as a
  child's (`ChildModule::connect`): an unknown frame, an oversized length or a hello naming another module ends it.
- [ ] `close` is called once per handle, off the core's async threads, and the sink is freed only after it returns
  (`Open::drop`).
- [ ] A library is never unloaded while the core runs (`plugin_host::loaded`): a runtime thread that outlives its
  module's grace would otherwise run unmapped code.

### What a library gets

- [ ] Everything a compiled-in module gets, and only that by design; but as code in the core's process it can reach
  the rest regardless. A library is shipped and pinned exactly as the core binary is, from the same release.

Reviewed by: \_\_\_\_\_\_ on \_\_\_\_\_\_ at commit \_\_\_\_\_\_
