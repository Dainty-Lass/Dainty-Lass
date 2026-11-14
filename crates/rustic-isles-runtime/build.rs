//! The runtime is built for x64 Windows only, where the `minhook` crate builds MinHook (RT-1); elsewhere it is empty.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(minhook)");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if os == "windows" && arch == "x86_64" {
        println!("cargo::rustc-cfg=minhook");
    }
}
