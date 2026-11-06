//! The two desktop apps' bundle configs (`dainty-launcher`, `dainty-manager`) are wired and distinct:
//! names, identifiers, icons, their dashboard page, and the version and features the release builds take.

use std::path::{Path, PathBuf};

use serde_json::Value;

fn src_tauri() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn json(name: &str) -> Value {
    let path = src_tauri().join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
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
fn client_and_manager_are_distinct_apps() {
    let client = json("tauri.conf.json");
    let manager = json("tauri.server-gui.conf.json");

    assert_eq!(text(&client, "/productName"), "DaintyLass Launcher");
    assert_eq!(text(&client, "/mainBinaryName"), "dainty-launcher");
    // Unchanged since the Electron launcher, so installs update in place
    assert_eq!(
        text(&client, "/identifier"),
        "io.github.freezamotoqueiro.dauntlessreforgedrevival"
    );
    assert_eq!(text(&manager, "/productName"), "DaintyLass Manager");
    assert_eq!(text(&manager, "/mainBinaryName"), "dainty-manager");
    assert_eq!(
        text(&manager, "/identifier"),
        "io.github.freezamotoqueiro.dauntlessreforgedrevival.server"
    );

    for (config, page) in [(&client, "launcher"), (&manager, "server-manager")] {
        assert_eq!(text(config, "/build/frontendDist"), format!("../dist/{page}"));
        assert_eq!(
            text(config, "/build/beforeBuildCommand"),
            format!("bun scripts/lass.ts dashboard-page build {page}")
        );
        assert_eq!(
            text(config, "/build/beforeDevCommand"),
            format!("bun scripts/lass.ts dashboard-page dev {page}")
        );
    }

    let (client_icons, manager_icons) = (icons(&client), icons(&manager));
    assert!(
        client_icons.len() >= 4 && manager_icons.len() >= 4,
        "both configs list their icons"
    );
    for icon in client_icons.iter().chain(&manager_icons) {
        assert!(src_tauri().join(icon).is_file(), "missing icon {icon}");
    }
    assert!(manager_icons.iter().all(|icon| icon.starts_with("icons/server/")));
    let installer = |config: &Value| text(config, "/bundle/windows/nsis/installerIcon").to_owned();
    let (client_ico, manager_ico) = (installer(&client), installer(&manager));
    assert!(src_tauri().join(&client_ico).is_file() && src_tauri().join(&manager_ico).is_file());
    assert_ne!(client_ico, manager_ico);
}

#[test]
fn release_builds_take_one_version_and_the_game_module() {
    let package = json("../package.json");
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
        .lines()
        .find(|line| line.trim_start().starts_with("server-gui ="))
        .unwrap_or_default();
    assert!(
        server_gui.contains("\"isles-1-14-7\""),
        "server-gui links the 1.14.7 module"
    );
}
