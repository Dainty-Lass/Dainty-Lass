# MinHook v1.3.3 (vendored)

Upstream: <https://github.com/TsudaKageyu/minhook>, tag `v1.3.3` (commit `9fbd087432700d73fc571118d6a9697a36443d88`),
taken from upstream's tag, not from `restricted/` (spec-runtime-r0 section 2, call 2). BSD-2-Clause: `LICENSE.txt`.

Only the x64 sources `build.rs` compiles are kept: `src/buffer.c`, `src/hook.c`, `src/trampoline.c`,
`src/hde/hde64.c`, their headers and `include/MinHook.h`. They are unmodified.
