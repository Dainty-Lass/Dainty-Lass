//! The module panel against a registry holding the stub module (`dainty_module_api::stub`), and a host.toml in a
//! temporary server folder.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use dainty_module_api::GameModule;
use dainty_module_api::stub::StubModule;
use dainty_service_kit::context::StatusRegistry;
use serde_json::json;
use server_controller::{ModuleEntry, ModuleRegistry, ModuleSource, RegistryOptions};

use super::*;
use dainty_launcher_kit::bridge::types::{ModuleSettingValue, ModuleView};

const HOST_TOML: &str = "version = 1\nmode = \"private\"\nname = \"Friday Hunts\"\nhost = \"100.101.102.103\"\n\
    game_address = \"100.101.102.103\"\nregistration = \"INVITECODE\"\nmax_hunt_servers = 3\n";

struct Fixture {
    panel: Panel,
    registry: ModuleRegistry,
    dir: tempfile::TempDir,
}

impl Fixture {
    fn host_toml(&self) -> std::path::PathBuf {
        dainty_host::paths::Paths::new(self.dir.path()).host_toml
    }
}

fn running() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
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
    let panel = Panel::default();
    *lock(&panel.registry) = Some(registry.clone());
    *lock(&panel.workspace) = Some(dir.path().to_path_buf());
    let fixture = Fixture { panel, registry, dir };
    let file = fixture.host_toml();
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, HOST_TOML).unwrap();
    fixture
}

fn module(outcome: ModuleOutcome) -> ModuleView {
    match outcome {
        ModuleOutcome::Ok(view) => *view,
        ModuleOutcome::Err(error) => panic!("{error:?}"),
    }
}

fn refused(outcome: ModuleOutcome) -> (ErrorCode, String) {
    match outcome {
        ModuleOutcome::Err(error) => (error.code, error.detail.unwrap_or_default()),
        ModuleOutcome::Ok(view) => panic!("{view:?}"),
    }
}

fn said(outcome: ModuleCommandOutcome) -> Result<(String, Option<String>), (ErrorCode, String)> {
    match outcome {
        ModuleCommandOutcome::Ok { text, data } => Ok((text, data)),
        ModuleCommandOutcome::Err(error) => Err((error.code, error.detail.unwrap_or_default())),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_server_has_no_modules() {
    let panel: Arc<dyn ModulePanel> = Arc::new(Panel::default());
    assert_eq!(
        panel.list().await,
        ModuleListOutcome::Err(LauncherError::new(ErrorCode::ServerStopped))
    );
    assert_eq!(refused(panel.load("stub".into()).await).0, ErrorCode::ServerStopped);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_listed_modules_load_run_a_command_and_unload() {
    let f = running();
    let panel = shared(&f.panel);
    let ModuleListOutcome::Ok(listed) = panel.list().await else {
        panic!("the list")
    };
    assert_eq!(listed.len(), 1);
    // A compiled-in module's manifest is known before it loads; its commands and settings are not
    let before = &listed[0];
    assert_eq!(before.state, ModuleRunState::NotLoaded);
    assert_eq!(before.display_name.as_deref(), Some("Stub"));
    assert!(before.unverified);
    assert!(before.commands.is_empty() && before.settings.is_empty());

    let ready = module(panel.load("stub".into()).await);
    assert_eq!(ready.state, ModuleRunState::Ready);
    assert!(ready.commands.iter().any(|c| c.name == "echo"));
    assert_eq!(ready.settings[0].keys[0].name, "greeting");
    assert_eq!(
        refused(panel.load("stub".into()).await),
        (ErrorCode::ModuleBadState, "loaded".into())
    );

    let echo = |input: serde_json::Value| panel.run("stub".into(), "echo".into(), input.to_string());
    assert_eq!(said(echo(json!({ "words": "there" })).await).unwrap().0, "hello there");
    assert_eq!(said(echo(json!(null)).await).unwrap().0, "hello");
    assert_eq!(
        said(echo(json!({ "words": 7 })).await).unwrap_err(),
        (ErrorCode::ModuleInputInvalid, "argument_text:words".into())
    );
    assert_eq!(
        said(panel.run("stub".into(), "nope".into(), "{}".into()).await).unwrap_err(),
        (ErrorCode::ModuleInputInvalid, "command".into())
    );
    assert_eq!(
        said(panel.run("stub".into(), "fail".into(), "null".into()).await)
            .unwrap_err()
            .0,
        ErrorCode::ModuleFailed
    );

    let stopped = module(panel.unload("stub".into()).await);
    assert_eq!(stopped.state, ModuleRunState::NotLoaded);
    assert_eq!(
        refused(panel.unload("stub".into()).await),
        (ErrorCode::ModuleBadState, "not_loaded".into())
    );
    assert_eq!(
        said(echo(json!(null)).await).unwrap_err(),
        (ErrorCode::ModuleBadState, "not_loaded".into())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn only_listed_ids_are_modules() {
    let f = running();
    let panel = shared(&f.panel);
    for id in ["nope", "../stub", "C:\\modules\\stub.exe", "Stub", ""] {
        let (code, _) = refused(panel.load(id.into()).await);
        assert_eq!(code, ErrorCode::ModuleUnknown, "{id}");
        assert_eq!(refused(panel.get(id.into()).await).0, ErrorCode::ModuleUnknown, "{id}");
        assert_eq!(
            said(panel.run(id.into(), "echo".into(), "null".into()).await)
                .unwrap_err()
                .0,
            ErrorCode::ModuleUnknown,
            "{id}"
        );
    }
    assert_eq!(f.registry.list().len(), 1, "nothing was added");
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_are_saved_to_host_toml_and_taken_by_the_next_load() {
    let f = running();
    let panel = shared(&f.panel);
    module(panel.load("stub".into()).await);
    let saved = module(
        panel
            .set_settings("stub".into(), json!({ "stub": { "greeting": "hey" } }).to_string())
            .await,
    );
    assert_eq!(saved.settings[0].keys[0].value, ModuleSettingValue::Text("hey".into()));
    assert_eq!(
        saved.settings[0].keys[0].default,
        ModuleSettingValue::Text("hello".into())
    );
    let text = std::fs::read_to_string(f.host_toml()).unwrap();
    assert!(
        text.contains("[modules.stub.stub]") && text.contains("greeting = \"hey\""),
        "{text}"
    );
    assert!(
        text.contains("name = \"Friday Hunts\""),
        "the rest of the file stays: {text}"
    );

    // A running module keeps what it started with; a restart takes the new value
    let echo = || panel.run("stub".into(), "echo".into(), "null".into());
    assert_eq!(said(echo().await).unwrap().0, "hello");
    module(panel.unload("stub".into()).await);
    module(panel.load("stub".into()).await);
    assert_eq!(said(echo().await).unwrap().0, "hey");

    // What does not fit is refused and the file is as it was
    let before = std::fs::read_to_string(f.host_toml()).unwrap();
    for (values, why) in [
        (json!({ "stub": { "greeting": 3 } }), "type:stub.greeting"),
        (json!({ "stub": { "nope": "x" } }), "unknown_key:stub.nope"),
        (json!({ "other": { "greeting": "x" } }), "unknown_section"),
    ] {
        assert_eq!(
            refused(panel.set_settings("stub".into(), values.to_string()).await),
            (ErrorCode::ModuleSettingsInvalid, why.into())
        );
    }
    assert_eq!(std::fs::read_to_string(f.host_toml()).unwrap(), before);
    // And a module that has not loaded has registered nothing to set
    module(panel.unload("stub".into()).await);
    assert_eq!(
        refused(
            panel
                .set_settings("stub".into(), json!({ "stub": { "greeting": "x" } }).to_string())
                .await
        )
        .0,
        ErrorCode::ModuleSettingsInvalid
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_need_a_host_toml() {
    let f = running();
    std::fs::remove_file(f.host_toml()).unwrap();
    let panel = shared(&f.panel);
    module(panel.load("stub".into()).await);
    assert_eq!(
        refused(
            panel
                .set_settings("stub".into(), json!({ "stub": { "greeting": "x" } }).to_string())
                .await
        ),
        (ErrorCode::ModuleFailed, "host_toml".into())
    );
}
