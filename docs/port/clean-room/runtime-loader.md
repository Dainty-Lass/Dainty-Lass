# Clean-room Provenance Record: the runtime loader (`native/runtime-loader`)

## 1. Specification

- **Spec file**: `docs/port/spec-runtime-loader.md`, commit `1d02ef22` on `feat/runtime-loader-clean-room` (base
  `plan` at `d80cff53`).
- **Spec writer**: the session that read the replaced loader (`restricted/native/runtime-loader`, Mystic Paradox's
  `tools/RuntimeLoader/`) and wrote the spec from it. That session wrote no code in `native/runtime-loader`.

## 2. Implementer isolation

- **Implementer**: a fresh agent with none of the spec writer's context, given the spec and told which files it
  could read (the spec, the root `AGENTS.md` and `rust-toolchain.toml`, and the launcher's `RUNTIME_DLLS` constant).
  It was barred from git history, `restricted/native/**`, the Paradox patch and notice documents, `agpl-inventory.md`
  and any outside copy of coldloader-proxy, Mystic Paradox or Reforged.
- **Pre-invocation removal**: `restricted/native/runtime-loader` was removed in `eafe6c92` before the implementer
  started, so the old source was absent from the working tree.

## 3. Commits

- Spec: `1d02ef22`. Removal: `eafe6c92`. Implementation: the `RD27: MIT runtime loader` commit after them.

## 4. Questions and choices

The implementer asked nothing during the work. It reported these choices, accepted on review:

- The built-DLL test runs `cargo build --lib` with the test's own profile first, because `cargo test` does not
  build a cdylib for integration tests.
- `DllMain` catches a panic in debug builds and still returns `TRUE`; release builds abort, as the spec says.
- A path or ini value containing a NUL is never loaded.
- No dependencies: the kernel32 functions are declared by hand.

## 5. Verification

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`: clean.
- `cargo test`: 15 unit and 5 integration tests pass. The integration tests check the built DLL's export directory
  against the spec's 197 names plus `DllMain`, check that it imports neither `winmm.dll` nor `version.dll`, and
  compare forwarded calls with the system DLLs.
- `cargo build --release --target x86_64-pc-windows-msvc`: `winmm.dll`, 240 640 bytes. `cargo check` for
  `x86_64-unknown-linux-gnu` passes (the crate is empty there).
- Provenance grep over the crate: no match for coldloader, Mystic Paradox, Reforged, Freeza or pranav.
- Not run against the 1.14.7 client.
