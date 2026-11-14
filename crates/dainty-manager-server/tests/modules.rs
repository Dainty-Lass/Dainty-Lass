// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The server GUI's module panel against a registry holding the stub module (spec-tauri-shell §6, §12.2).

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use dainty_controller::{ModuleEntry, ModuleRegistry, ModuleSource, RegistryOptions};
use dainty_launcher_kit::bridge::modules::ModulePanel;
use dainty_launcher_kit::bridge::types::{
    ErrorCode, LauncherError, ModuleCommandOutcome, ModuleListOutcome, ModuleOutcome, ModuleRunState,
    ModuleSettingValue, ModuleView,
};
use dainty_manager_server::ServerModules;
use dainty_module_api::GameModule;
use dainty_module_api::stub::StubModule;
use dainty_service_kit::context::StatusRegistry;

const SETTINGS_TOML: &str = "version = 1\nmode = \"private\"\nname = \"Friday Hunts\"\nhost = \"100.101.102.103\"\n\
    game_address = \"100.101.102.103\"\nregistration = \"INVITECODE\"\nmax_hunt_servers = 3\n\
    [mongo]\nuri = \"mongodb://127.0.0.1:27017\"\nmanage = false\n";

fn write_settings_toml(workspace: &Path) -> std::path::PathBuf {
    let file = dainty_host::paths::Paths::new(workspace).settings_toml;
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, SETTINGS_TOML).unwrap();
    file
}

fn registry() -> ModuleRegistry {
    let registry = ModuleRegistry::new(
        RegistryOptions::standalone(),
        StatusRegistry::new("private", Vec::new()),
    );
    registry
        .add_entry(ModuleEntry {
            id: "stub".parse().unwrap(),
            source: ModuleSource::CompiledIn(Arc::new(|| Arc::new(StubModule::new()) as Arc<dyn GameModule>)),
            autoload: false,
            settings: BTreeMap::new(),
        })
        .unwrap();
    registry
}

fn panel(workspace: &Path) -> ServerModules {
    let panel = ServerModules::new();
    panel.attach_registry(Some(registry()));
    panel.set_workspace(Some(workspace.to_path_buf()));
    panel
}

fn module(outcome: ModuleOutcome) -> ModuleView {
    match outcome {
        ModuleOutcome::Ok(view) => *view,
        ModuleOutcome::Err(error) => panic!("refused: {error:?}"),
    }
}

fn refused(outcome: ModuleOutcome) -> LauncherError {
    match outcome {
        ModuleOutcome::Ok(view) => panic!("answered: {view:?}"),
        ModuleOutcome::Err(error) => error,
    }
}

fn code(error: &LauncherError) -> (ErrorCode, &str) {
    (error.code, error.detail.as_deref().unwrap_or(""))
}

async fn run(panel: &ServerModules, command: &str, input: &str) -> Result<String, LauncherError> {
    match panel.run("stub".into(), command.into(), input.into()).await {
        ModuleCommandOutcome::Ok { text, .. } => Ok(text),
        ModuleCommandOutcome::Err(error) => Err(error),
    }
}

fn greeting(view: &ModuleView) -> ModuleSettingValue {
    view.settings[0].keys[0].value.clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_server_refuses_every_call() {
    let panel = ServerModules::new();
    match panel.list().await {
        ModuleListOutcome::Err(error) => assert_eq!(error.code, ErrorCode::ServerStopped),
        ModuleListOutcome::Ok(modules) => panic!("listed {modules:?}"),
    }
    assert_eq!(refused(panel.get("stub".into()).await).code, ErrorCode::ServerStopped);
    assert_eq!(refused(panel.load("stub".into()).await).code, ErrorCode::ServerStopped);
    assert_eq!(
        run(&panel, "echo", "null").await.unwrap_err().code,
        ErrorCode::ServerStopped
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_stub_loads_answers_and_unloads() {
    let dir = tempfile::tempdir().unwrap();
    write_settings_toml(dir.path());
    let panel = panel(dir.path());

    let before = module(panel.get("stub".into()).await);
    assert_eq!(before.state, ModuleRunState::NotLoaded);
    assert_eq!(before.display_name.as_deref(), Some("Stub"));
    assert!(before.unverified);
    assert!(before.commands.is_empty() && before.settings.is_empty());

    let loaded = module(panel.load("stub".into()).await);
    assert_eq!(loaded.state, ModuleRunState::Ready);
    assert!(loaded.commands.iter().any(|c| c.name == "echo"));
    assert_eq!(loaded.settings[0].keys[0].name, "greeting");
    assert_eq!(
        code(&refused(panel.load("stub".into()).await)),
        (ErrorCode::ModuleBadState, "loaded")
    );

    assert_eq!(
        run(&panel, "echo", r#"{"words":"there"}"#).await.unwrap(),
        "hello there"
    );
    assert_eq!(run(&panel, "echo", "null").await.unwrap(), "hello");
    assert_eq!(
        run(&panel, "nope", "null").await.unwrap_err().code,
        ErrorCode::ModuleInputInvalid
    );
    assert_eq!(
        run(&panel, "fail", "null").await.unwrap_err().code,
        ErrorCode::ModuleFailed
    );

    let unloaded = module(panel.unload("stub".into()).await);
    assert_eq!(unloaded.state, ModuleRunState::NotLoaded);
    assert_eq!(
        code(&refused(panel.unload("stub".into()).await)),
        (ErrorCode::ModuleBadState, "not_loaded")
    );
    assert_eq!(
        code(&run(&panel, "echo", "null").await.unwrap_err()),
        (ErrorCode::ModuleBadState, "not_loaded")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ids_not_listed_are_unknown_and_nothing_is_added() {
    let dir = tempfile::tempdir().unwrap();
    write_settings_toml(dir.path());
    let panel = panel(dir.path());
    for id in ["other", "../etc"] {
        assert_eq!(refused(panel.load(id.into()).await).code, ErrorCode::ModuleUnknown);
        assert_eq!(refused(panel.get(id.into()).await).code, ErrorCode::ModuleUnknown);
        match panel.run(id.into(), "echo".into(), "null".into()).await {
            ModuleCommandOutcome::Err(error) => assert_eq!(error.code, ErrorCode::ModuleUnknown),
            ModuleCommandOutcome::Ok { text, .. } => panic!("ran: {text}"),
        }
    }
    match panel.list().await {
        ModuleListOutcome::Ok(modules) => {
            assert_eq!(modules.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["stub"]);
        }
        ModuleListOutcome::Err(error) => panic!("refused: {error:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_are_saved_to_settings_toml_and_taken_on_the_next_load() {
    let dir = tempfile::tempdir().unwrap();
    let file = write_settings_toml(dir.path());
    let panel = panel(dir.path());

    // An unloaded module registered nothing, so nothing fits it
    assert_eq!(
        refused(
            panel
                .set_settings("stub".into(), r#"{"stub":{"greeting":"hi"}}"#.into())
                .await
        )
        .code,
        ErrorCode::ModuleSettingsInvalid
    );
    module(panel.load("stub".into()).await);
    let saved = module(
        panel
            .set_settings("stub".into(), r#"{"stub":{"greeting":"hi"}}"#.into())
            .await,
    );
    assert_eq!(greeting(&saved), ModuleSettingValue::Text("hi".into()));
    let config = dainty_host::hostconfig::load_host_config(&dainty_host::paths::Paths::new(dir.path()))
        .unwrap()
        .unwrap();
    let settings = &config.modules.unwrap().settings[&"stub".parse().unwrap()];
    assert_eq!(settings["stub"]["greeting"], serde_json::json!("hi"));

    // A running module keeps what it started with until it loads again
    assert_eq!(run(&panel, "echo", "null").await.unwrap(), "hello");
    module(panel.unload("stub".into()).await);
    module(panel.load("stub".into()).await);
    assert_eq!(run(&panel, "echo", "null").await.unwrap(), "hi");

    let written = std::fs::read(&file).unwrap();
    assert_eq!(
        refused(
            panel
                .set_settings("stub".into(), r#"{"stub":{"greeting":3}}"#.into())
                .await
        )
        .code,
        ErrorCode::ModuleSettingsInvalid
    );
    assert_eq!(
        std::fs::read(&file).unwrap(),
        written,
        "a refused value leaves the file as it was"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_without_a_settings_toml_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let panel = panel(dir.path());
    module(panel.load("stub".into()).await);
    assert_eq!(
        code(&refused(
            panel
                .set_settings("stub".into(), r#"{"stub":{"greeting":"hi"}}"#.into())
                .await
        )),
        (ErrorCode::ModuleFailed, "settings_toml")
    );
    module(panel.unload("stub".into()).await);
}
