//! The two desktop apps restricted builds (`dainty-launcher`, `dainty-manager`) over dainty-launcher-app: today's icons
//! (L2), each its own, and the identity, page and update feed of dainty-launcher-app's configs, which the compiled-in
//! context takes; and the version and features the release builds take.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::path::{Path, PathBuf};

use serde_json::Value;

fn src_tauri() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn launcher_app() -> PathBuf {
    src_tauri().join("../../../crates/dainty-launcher-app")
}

fn json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn text<'a>(config: &'a Value, pointer: &str) -> &'a str {
    config.pointer(pointer).and_then(Value::as_str).unwrap_or_default()
}

fn icons(config: &Value) -> Vec<String> {
    let list = config
        .pointer("/bundle/icon")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    list.iter().filter_map(Value::as_str).map(str::to_owned).collect()
}

#[test]
fn client_and_manager_are_distinct_apps_with_todays_icons() {
    let client = json(&src_tauri().join("tauri.conf.json"));
    let manager = json(&src_tauri().join("tauri.server-gui.conf.json"));
    let mit_client = json(&launcher_app().join("tauri.conf.json"));
    let mit_manager = json(&launcher_app().join("tauri.server-gui.conf.json"));

    for (ours, mit) in [(&client, &mit_client), (&manager, &mit_manager)] {
        for pointer in ["/productName", "/mainBinaryName", "/identifier", "/build/devUrl"] {
            assert_eq!(text(ours, pointer), text(mit, pointer), "{pointer}");
        }
        assert_eq!(
            ours.pointer("/plugins/updater/endpoints"),
            mit.pointer("/plugins/updater/endpoints")
        );
    }
    assert_eq!(text(&client, "/productName"), "Dainty Lass Launcher");
    assert_eq!(text(&manager, "/productName"), "Dainty Lass Manager");

    // The page dainty-launcher-app's build makes, where the bundler looks for it
    for (config, page) in [(&client, "launcher"), (&manager, "server-manager")] {
        assert_eq!(
            text(config, "/build/frontendDist"),
            format!("../../../crates/dainty-launcher-app/dist/{page}")
        );
        for (pointer, mode) in [
            ("/build/beforeBuildCommand", "build"),
            ("/build/beforeDevCommand", "dev"),
        ] {
            assert_eq!(
                text(config, pointer),
                format!("bun ../../crates/dainty-launcher-app/scripts/page.ts {mode} {page}")
            );
        }
        for format in ["deb", "rpm"] {
            let entry = src_tauri().join(text(config, &format!("/bundle/linux/{format}/desktopTemplate")));
            assert!(entry.is_file(), "missing {}", entry.display());
        }
    }

    let (client_icons, manager_icons) = (icons(&client), icons(&manager));
    assert!(
        client_icons.len() >= 4 && manager_icons.len() >= 4,
        "both configs list their icons"
    );
    for icon in client_icons.iter().chain(&manager_icons) {
        assert!(src_tauri().join(icon).is_file(), "missing icon {icon}");
    }
    assert!(
        client_icons
            .iter()
            .all(|icon| icon.starts_with("icons/") && !icon.starts_with("icons/server/"))
    );
    assert!(manager_icons.iter().all(|icon| icon.starts_with("icons/server/")));
    let installer = |config: &Value| text(config, "/bundle/windows/nsis/installerIcon").to_owned();
    let (client_ico, manager_ico) = (installer(&client), installer(&manager));
    assert!(src_tauri().join(&client_ico).is_file() && src_tauri().join(&manager_ico).is_file());
    assert_ne!(client_ico, manager_ico);

    // The window's and the tray's icon, handed to dainty-launcher-app (lib.rs)
    let lib = std::fs::read_to_string(src_tauri().join("src/lib.rs")).unwrap();
    assert!(lib.contains("include_image!(\"icons/32x32.png\")"));
    assert!(lib.contains("include_image!(\"icons/server/32x32.png\")"));
}

#[test]
fn release_builds_take_one_version_and_the_game_module() {
    let package = json(&src_tauri().join("../package.json"));
    let version = text(&package, "/version");
    assert!(!version.is_empty());
    assert_eq!(
        version,
        env!("CARGO_PKG_VERSION"),
        "package.json and Cargo.toml disagree"
    );

    let scripts = package.get("scripts").cloned().unwrap_or_default();
    assert!(text(&scripts, "/build:client").contains("tauri build"));
    let manager = text(&scripts, "/build:manager");
    assert!(manager.contains("tauri.server-gui.conf.json") && manager.contains("--features server-gui"));

    let manifest = std::fs::read_to_string(src_tauri().join("Cargo.toml")).unwrap();
    let server_gui = manifest
        .split("server-gui = [")
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .unwrap_or_default();
    assert!(
        server_gui.contains("\"isles-1-14-7\"") && server_gui.contains("\"dainty-launcher-app/server-gui\""),
        "server-gui is dainty-launcher-app's manager with the 1.14.7 module"
    );
}
