// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The two desktop apps' bundle configs (`dainty-launcher`, `dainty-manager`) are wired and distinct: names,
//! identifiers, icons, their dashboard page, their update feeds and the version the release builds take.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::path::{Path, PathBuf};

use serde_json::Value;

fn crate_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn json(name: &str) -> Value {
    let path = crate_dir().join(name);
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

    assert_eq!(text(&client, "/productName"), "Dainty Lass Launcher");
    assert_eq!(text(&client, "/mainBinaryName"), "dainty-launcher");
    // Unchanged since the Electron launcher, so installs update in place
    assert_eq!(
        text(&client, "/identifier"),
        "io.github.freezamotoqueiro.dauntlessreforgedrevival"
    );
    assert_eq!(text(&manager, "/productName"), "Dainty Lass Manager");
    assert_eq!(text(&manager, "/mainBinaryName"), "dainty-manager");
    assert_eq!(
        text(&manager, "/identifier"),
        "io.github.freezamotoqueiro.dauntlessreforgedrevival.server"
    );

    // The CLI runs them from the folder above this crate (crates/), as it does any app folder
    for (config, page, port) in [(&client, "launcher", 4321), (&manager, "server-manager", 4322)] {
        assert_eq!(text(config, "/build/frontendDist"), format!("dist/{page}"));
        assert_eq!(text(config, "/build/devUrl"), format!("http://localhost:{port}"));
        assert_eq!(
            text(config, "/build/beforeBuildCommand"),
            format!("bun dainty-launcher-app/scripts/page.ts build {page}")
        );
        assert_eq!(
            text(config, "/build/beforeDevCommand"),
            format!("bun dainty-launcher-app/scripts/page.ts dev {page}")
        );
    }

    // The placeholders until new icons are drawn (L2); restricted's builds hand in today's
    for config in [&client, &manager] {
        let list = icons(config);
        assert!(list.len() >= 4, "the config lists its icons");
        for icon in &list {
            assert!(crate_dir().join(icon).is_file(), "missing icon {icon}");
        }
        let installer = text(config, "/bundle/windows/nsis/installerIcon");
        assert!(
            crate_dir().join(installer).is_file(),
            "missing installer icon {installer}"
        );
    }

    for (config, entry) in [(&client, "linux/launcher.desktop"), (&manager, "linux/manager.desktop")] {
        for format in ["deb", "rpm"] {
            assert_eq!(text(config, &format!("/bundle/linux/{format}/desktopTemplate")), entry);
        }
        assert!(crate_dir().join(entry).is_file(), "missing {entry}");
    }
    let manager_entry = std::fs::read_to_string(crate_dir().join("linux/manager.desktop")).unwrap();
    assert!(
        !manager_entry.contains("MimeType="),
        "only the launcher takes invite links"
    );
}

#[test]
fn each_app_has_its_own_update_feed() {
    let feed = |config: &Value| {
        config
            .pointer("/plugins/updater/endpoints")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let base = "https://github.com/Dainty-Lass/Dainty-Lass/releases/download/launcher-updates/";
    assert_eq!(
        feed(&json("tauri.conf.json")),
        [Value::from(format!("{base}latest.json"))]
    );
    assert_eq!(
        feed(&json("tauri.server-gui.conf.json")),
        [Value::from(format!("{base}latest-manager.json"))]
    );
}

#[test]
fn release_builds_take_the_launchers_version() {
    let package = json("../../launcher/package.json");
    let version = text(&package, "/version");
    assert!(!version.is_empty());
    assert_eq!(
        text(&json("tauri.conf.json"), "/version"),
        "../../launcher/package.json"
    );
    assert!(
        json("tauri.server-gui.conf.json").get("version").is_none(),
        "the manager takes the client's version"
    );
}

#[test]
fn each_app_has_its_own_icon_set_and_the_installer_embeds_webview2() {
    let client = json("tauri.conf.json");
    let manager = json("tauri.server-gui.conf.json");
    // T-2: the manager's are copies of the placeholders until new icons are drawn
    assert!(icons(&client).iter().all(|icon| !icon.starts_with("icons/manager/")));
    assert!(icons(&manager).iter().all(|icon| icon.starts_with("icons/manager/")));
    assert_ne!(
        text(&client, "/bundle/windows/nsis/installerIcon"),
        text(&manager, "/bundle/windows/nsis/installerIcon")
    );
    // T-5: as the restricted build's installer did
    assert_eq!(
        text(&client, "/bundle/windows/webviewInstallMode/type"),
        "embedBootstrapper"
    );
}

#[test]
fn the_manager_serves_1_14_7() {
    let manifest = std::fs::read_to_string(crate_dir().join("Cargo.toml")).unwrap();
    let table: toml_edit::DocumentMut = manifest.parse().unwrap();
    let server_gui = table["features"]["server-gui"].as_array().unwrap();
    let names: Vec<&str> = server_gui.iter().filter_map(|v| v.as_str()).collect();
    assert!(names.contains(&"isles-1-14-7"), "{names:?}");
    assert!(names.contains(&"dep:dainty-manager-server"), "{names:?}");
}
