//! The registry's verify (DF3): two modules for one game version refused in either order, a module killed
//! mid-run shown failed and restarted on command, nothing left running after stop; and what a load does
//! between: ports moved off a clash, host.toml's settings, the bus, core plugins, plugins on a game module.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use controller::registry::system_environment;
use controller::{
    ModuleEntry, ModuleRegistry, ModuleSource, ModuleState, RegistryError, RegistryOptions, ServerController,
};
use module_api::host::file_sha256;
use module_api::manifest::{Capabilities, Transport};
use module_api::stub::{RecordingPlugin, StubModule, console};
use module_api::{
    BoxFuture, CommandRequest, ExternalModule, GameModule, GameVersion, ModuleConfig, ModuleContext, ModuleError,
    ModuleId, ModuleKind, ModuleManifest, ModuleStatus, Phase, PortRange, Registrar,
};
use service_kit::context::{ComponentPhase, StatusRegistry};

fn id(text: &str) -> ModuleId {
    text.parse().unwrap()
}

fn stub_version() -> GameVersion {
    "0.0.1".parse().unwrap()
}

fn compiled_stub() -> ModuleEntry {
    ModuleEntry {
        id: id("stub"),
        source: ModuleSource::CompiledIn(Arc::new(|| Arc::new(StubModule::new()) as Arc<dyn GameModule>)),
        autoload: true,
        settings: BTreeMap::new(),
    }
}

async fn external(name: &str, args: &[&str]) -> ModuleEntry {
    let path = PathBuf::from(env!("CARGO_BIN_EXE_controller-test-module"));
    let sha256 = file_sha256(&path).await.unwrap();
    let mut args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    if name != "stub" {
        args.extend(["--id".to_string(), name.to_string()]);
    }
    ModuleEntry {
        id: id(name),
        source: ModuleSource::External(ExternalModule {
            path,
            sha256,
            args,
            expected: None,
        }),
        autoload: true,
        settings: BTreeMap::new(),
    }
}

fn registry(options: RegistryOptions) -> (ModuleRegistry, StatusRegistry) {
    let status = StatusRegistry::new("private", Vec::new());
    (ModuleRegistry::new(options, status.clone()), status)
}

fn phase(status: &StatusRegistry, name: &str) -> Option<ComponentPhase> {
    status.snapshot().get(name).map(|c| c.phase)
}

async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    for _ in 0..100 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("{what} did not happen within 10 s");
}

fn echo() -> CommandRequest {
    CommandRequest {
        actor: console(),
        name: "echo".into(),
        args: vec!["there".into()],
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn two_modules_for_one_version_are_refused_in_either_order() {
    // Compiled in first, then an external one for the same version
    let (reg, _) = registry(RegistryOptions::standalone());
    reg.add_entry(compiled_stub()).unwrap();
    reg.add_entry(external("other-stub", &[]).await).unwrap();
    reg.load(&id("stub")).await.unwrap();
    let refused = reg.load(&id("other-stub")).await.unwrap_err();
    assert!(
        matches!(&refused, RegistryError::Duplicate { version, by, .. } if *version == stub_version() && by == "stub"),
        "{refused}"
    );
    assert_eq!(reg.info(&id("other-stub")).unwrap().state, ModuleState::Failed);
    assert_eq!(
        reg.info(&id("stub")).unwrap().state,
        ModuleState::Ready,
        "the first is untouched"
    );
    reg.unload_all().await;

    // External first, then the compiled-in one
    let (reg, _) = registry(RegistryOptions::standalone());
    reg.add_entry(external("other-stub", &[]).await).unwrap();
    reg.add_entry(compiled_stub()).unwrap();
    assert_eq!(reg.load_autoload().await.len(), 1, "the second of the two is refused");
    assert_eq!(reg.info(&id("other-stub")).unwrap().state, ModuleState::Ready);
    let stub = reg.info(&id("stub")).unwrap();
    assert_eq!(stub.state, ModuleState::Failed);
    assert!(stub.detail.unwrap().contains("already served by other-stub"));
    // Once the first goes, the version is free again
    reg.unload(&id("other-stub")).await.unwrap();
    reg.load(&id("stub")).await.unwrap();
    reg.unload_all().await;

    // A version the host kit's own stack serves is taken too, and a module host.toml doesn't list never loads
    let mut options = RegistryOptions::standalone();
    options.taken_versions = vec![(stub_version(), "the server's stub stack".into())];
    let (reg, _) = registry(options);
    reg.add_entry(compiled_stub()).unwrap();
    assert!(matches!(
        reg.load(&id("stub")).await,
        Err(RegistryError::Duplicate { .. })
    ));
    assert!(matches!(
        reg.load(&id("nobody")).await,
        Err(RegistryError::NotListed(_))
    ));
    assert!(matches!(
        reg.add_entry(compiled_stub()),
        Err(RegistryError::ListedTwice(_))
    ));
}

fn kill(pid: &str) {
    let status = if cfg!(windows) {
        std::process::Command::new("taskkill")
            .args(["/F", "/PID", pid])
            .output()
    } else {
        std::process::Command::new("kill").args(["-9", pid]).output()
    };
    assert!(status.unwrap().status.success(), "could not kill {pid}");
}

async fn read_when_written(file: &Path) -> String {
    for _ in 0..100 {
        if let Ok(text) = std::fs::read_to_string(file)
            && !text.is_empty()
        {
            return text;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("{} was never written", file.display());
}

fn answers(port: &str) -> bool {
    std::net::TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().unwrap(),
        Duration::from_millis(500),
    )
    .is_ok()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_module_killed_mid_run_shows_failed_and_restarts_on_command() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("pid");
    let grandchild = dir.path().join("grandchild");
    let (reg, status) = registry(RegistryOptions::standalone());
    let entry = external(
        "stub",
        &[
            "--pid-file",
            pid_file.to_str().unwrap(),
            "--grandchild",
            grandchild.to_str().unwrap(),
        ],
    )
    .await;
    reg.add_entry(entry).unwrap();
    reg.load(&id("stub")).await.unwrap();
    assert_eq!(phase(&status, "stub"), Some(ComponentPhase::Ready));
    let left_behind = read_when_written(&grandchild).await;
    assert!(answers(&left_behind));

    // Only the module's own process dies, as a crash would
    kill(&read_when_written(&pid_file).await);
    eventually("the failure", || {
        reg.info(&id("stub")).unwrap().state == ModuleState::Failed
    })
    .await;
    assert_eq!(phase(&status, "stub"), Some(ComponentPhase::Failed));
    assert!(reg.command(&id("stub"), echo()).await.is_err());

    // A load again is its restart: what the dead one left running goes with it
    std::fs::remove_file(&pid_file).unwrap();
    std::fs::remove_file(&grandchild).unwrap();
    reg.load(&id("stub")).await.unwrap();
    eventually("the old tree's end", || !answers(&left_behind)).await;
    assert_eq!(phase(&status, "stub"), Some(ComponentPhase::Ready));
    assert_eq!(reg.command(&id("stub"), echo()).await.unwrap().text, "hello there");
    reg.unload_all().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_is_left_running_after_stop() {
    let dir = tempfile::tempdir().unwrap();
    let grandchild = dir.path().join("grandchild");
    let mut controller = ServerController::new("private", Vec::new());
    let mut options = RegistryOptions::standalone();
    options.child.stop_grace = Duration::from_secs(2);
    let reg = ModuleRegistry::new(options, controller.status_registry().clone());
    reg.add_entry(external("stub", &["--grandchild", grandchild.to_str().unwrap()]).await)
        .unwrap();
    controller.set_registry(reg.clone());
    controller.start_all().await.unwrap();
    let port = read_when_written(&grandchild).await;
    assert!(answers(&port));
    // The console reaches the module by its id, as a service by its name
    controller.restart_component("stub").await.unwrap();
    eventually("the first tree's end", || !answers(&port)).await;
    let port = read_when_written(&grandchild).await;
    controller.shutdown().await;
    eventually("the tree's end", || !answers(&port)).await;
    assert_eq!(reg.info(&id("stub")).unwrap().state, ModuleState::NotLoaded);
    assert_eq!(
        controller.snapshot().get("stub").map(|c| c.phase),
        Some(ComponentPhase::Stopped)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_load_moves_clashing_ports_takes_its_settings_and_joins_the_bus() {
    let mut options = RegistryOptions::standalone();
    options.reserved_ports = vec![(Transport::Tcp, PortRange::single(65000))];
    let (reg, _) = registry(options);
    let plugin = Arc::new(RecordingPlugin::default());
    reg.add_plugin(plugin.clone());
    let mut entry = compiled_stub();
    entry.settings.insert(
        "stub".into(),
        BTreeMap::from([("greeting".to_string(), serde_json::json!("hi"))]),
    );
    reg.add_entry(entry).unwrap();
    let mut events = reg.subscribe_events();
    let info = reg.load(&id("stub")).await.unwrap();
    assert_eq!(
        info.ports["http"],
        PortRange::single(65001),
        "moved off the server's port"
    );
    assert_eq!(
        info.ports["game"],
        PortRange {
            first: 65010,
            last: 65019
        }
    );
    assert!(info.commands.iter().any(|c| c.name == "echo"));
    assert!(plugin.attached().contains_key(&id("stub")));
    assert_eq!(reg.command(&id("stub"), echo()).await.unwrap().text, "hi there");
    let missing = CommandRequest {
        name: "nope".into(),
        ..echo()
    };
    assert_eq!(
        reg.command(&id("stub"), missing).await.unwrap_err().kind,
        module_api::ErrorKind::NotFound
    );
    assert_eq!(reg.module_status(&id("stub")).await.unwrap().phase, Phase::Ready);
    // Its start event, then its answer to the host's server.ready
    let (from, started) = events.recv().await.unwrap();
    assert_eq!((from, started.name.as_str()), (Some(id("stub")), "stub.started"));
    reg.emit(module_api::Event {
        name: "server.ready".parse().unwrap(),
        payload: serde_json::Value::Null,
    });
    let mut saw = None;
    for _ in 0..3 {
        let (from, event) = events.recv().await.unwrap();
        if event.name.as_str() == "stub.saw" {
            saw = Some(from);
            break;
        }
    }
    assert_eq!(saw, Some(Some(id("stub"))));
    assert_eq!(reg.for_invite("stub").map(|m| m.id), Some(id("stub")));
    assert_eq!(reg.for_invite("0.0.1").map(|m| m.id), Some(id("stub")));
    reg.unload(&id("stub")).await.unwrap();
    assert!(plugin.attached().is_empty(), "detached on unload");
    assert!(reg.for_invite("stub").is_none());

    // A setting the module does not read is a typo in host.toml: refused, not ignored
    let (reg, _) = registry(RegistryOptions::standalone());
    let mut entry = compiled_stub();
    entry.settings.insert(
        "stub".into(),
        BTreeMap::from([("greting".to_string(), serde_json::json!("hi"))]),
    );
    reg.add_entry(entry).unwrap();
    let refused = reg.load(&id("stub")).await.unwrap_err().to_string();
    assert!(refused.contains("greting"), "{refused}");
}

/// A theme-only plugin for the stub's game version.
struct ThemePlugin;

impl GameModule for ThemePlugin {
    fn manifest(&self) -> ModuleManifest {
        ModuleManifest {
            id: id("stub-theme"),
            kind: ModuleKind::Plugin,
            display_name: "Stub theme".into(),
            game_version: stub_version(),
            invite_aliases: vec![],
            api: module_api::API_VERSION,
            ports: vec![],
            capabilities: Capabilities::default(),
            unverified: false,
            client: None,
        }
    }

    fn register(&self, _registrar: &mut Registrar) -> Result<(), ModuleError> {
        Ok(())
    }

    fn start(&self, _ctx: ModuleContext, _config: ModuleConfig) -> BoxFuture<'_, Result<(), ModuleError>> {
        Box::pin(async { Ok(()) })
    }

    fn stop(&self, _ctx: ModuleContext) -> BoxFuture<'_, Result<(), ModuleError>> {
        Box::pin(async { Ok(()) })
    }

    fn status(&self) -> ModuleStatus {
        ModuleStatus::new(Phase::Ready)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_needs_its_game_module_and_goes_with_it() {
    let (reg, _) = registry(RegistryOptions::standalone());
    reg.add_entry(ModuleEntry {
        id: id("stub-theme"),
        source: ModuleSource::CompiledIn(Arc::new(|| Arc::new(ThemePlugin) as Arc<dyn GameModule>)),
        autoload: true,
        settings: BTreeMap::new(),
    })
    .unwrap();
    reg.add_entry(compiled_stub()).unwrap();
    assert!(matches!(
        reg.load(&id("stub-theme")).await,
        Err(RegistryError::NoGameFor { .. })
    ));
    reg.load(&id("stub")).await.unwrap();
    reg.load(&id("stub-theme")).await.unwrap();
    // A plugin is not a second game module for the version
    assert_eq!(reg.for_invite("stub").map(|m| m.id), Some(id("stub")));
    reg.unload(&id("stub")).await.unwrap();
    assert_eq!(reg.info(&id("stub-theme")).unwrap().state, ModuleState::NotLoaded);
}

#[test]
fn children_get_the_os_variables_and_no_server_setting() {
    let vars = [
        ("PATH", "/bin"),
        ("SystemRoot", "C:\\Windows"),
        ("REFORGED_ADMIN_API_KEY", "secret"),
        ("GATEWAY_SECRET", "secret"),
        ("MONGODB_URI", "mongodb://x"),
    ]
    .map(|(k, v)| (k.to_string(), v.to_string()));
    let kept: Vec<String> = system_environment(vars).into_iter().map(|(k, _)| k).collect();
    assert_eq!(kept, ["PATH", "SystemRoot"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_registered_settings_show_and_the_next_load_takes_new_values() {
    let (reg, _) = registry(RegistryOptions::standalone());
    reg.add_entry(compiled_stub()).unwrap();
    assert!(reg.info(&id("stub")).unwrap().config.is_empty(), "known once it loads");
    reg.load(&id("stub")).await.unwrap();
    let config = reg.info(&id("stub")).unwrap().config;
    assert_eq!(config[0].name, "stub");
    assert_eq!(config[0].keys[0].name, "greeting");
    assert_eq!(reg.command(&id("stub"), echo()).await.unwrap().text, "hello there");
    reg.update_settings(
        &id("stub"),
        BTreeMap::from([(
            "stub".to_string(),
            BTreeMap::from([("greeting".to_string(), serde_json::json!("hey"))]),
        )]),
    )
    .unwrap();
    assert_eq!(
        reg.command(&id("stub"), echo()).await.unwrap().text,
        "hello there",
        "a running module keeps what it started with"
    );
    reg.restart(&id("stub")).await.unwrap();
    assert_eq!(reg.command(&id("stub"), echo()).await.unwrap().text, "hey there");
    assert_eq!(
        reg.update_settings(&id("nope"), BTreeMap::new()),
        Err(RegistryError::NotListed(id("nope")))
    );
    reg.unload_all().await;
}
