// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The built library loads through plugin-host, pinned by its SHA-256, and answers as the compiled-in module.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::path::PathBuf;

use dainty_module_api::host::file_sha256;
use dainty_module_api::{
    ChildOptions, ExternalModule, GameModule, ModuleDriver, ModuleDriverExt, ModuleId, NodeIdentity,
};
use dainty_plugin_host::{DylibModule, is_library};

/// Cargo builds this crate's cdylib beside the test binary's `deps` folder; a nested cargo would wait on the outer
/// one's lock, so the test does not build it itself.
fn library() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let profile = exe.parent().and_then(|deps| deps.parent()).unwrap();
    let name = format!(
        "{}rustic_isles_1_14_7_plugin{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let path = profile.join(name);
    assert!(
        path.is_file(),
        "{} is missing: run `cargo build -p rustic-isles-1-14-7-plugin` first",
        path.display()
    );
    path
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_library_is_the_compiled_in_module() {
    let path = library();
    assert!(is_library(&path));
    let external = ExternalModule {
        sha256: file_sha256(&path).await.unwrap(),
        path,
        args: vec![],
        expected: Some(ModuleId::new("isles-1-14-7").unwrap()),
    };
    let host = dainty_module_api::stub::recording_host();
    let driver = DylibModule::load(&external, NodeIdentity::standalone(), host, ChildOptions::default())
        .await
        .unwrap();
    let compiled = rustic_isles_1_14_7::module().unwrap().manifest();
    assert_eq!(
        serde_json::to_value(driver.manifest()).unwrap(),
        serde_json::to_value(&compiled).unwrap()
    );
    let registration = driver.register().await.unwrap();
    assert!(!registration.routes.is_empty());
    driver.unload().await.unwrap();
}
