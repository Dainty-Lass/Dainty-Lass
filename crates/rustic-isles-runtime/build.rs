//! Builds the vendored MinHook v1.3.3 (BSD-2-Clause, `minhook/LICENSE.txt`) for the x64 Windows DLL only (RT-1).
//! `cc` follows the workspace's `+crt-static`, so it compiles `/MT` and the DLL imports no CRT DLL.

fn main() {
    println!("cargo::rerun-if-changed=minhook");
    println!("cargo::rustc-check-cfg=cfg(minhook)");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if os != "windows" || arch != "x86_64" {
        return;
    }
    cc::Build::new()
        .files([
            "minhook/src/buffer.c",
            "minhook/src/hook.c",
            "minhook/src/trampoline.c",
            "minhook/src/hde/hde64.c",
        ])
        .include("minhook/include")
        .warnings(false)
        .compile("minhook");
    println!("cargo::rustc-cfg=minhook");
}
