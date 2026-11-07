//! The executable's Windows resources (icon, version, manifest) from tauri.conf.json. Everything else the app
//! compiles in (the context, the permissions, the isolation hook) is launcher-app's build.

fn main() {
    if let Err(error) = tauri_build::try_build(tauri_build::Attributes::new()) {
        panic!("tauri build step failed: {error:#}");
    }
}
