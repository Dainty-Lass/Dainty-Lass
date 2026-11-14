//! The registry's verify (DF3): two modules for one game version refused in either order, a module killed
//! mid-run shown failed and restarted on command, nothing left running after stop; and what a load does
//! between: ports moved off a clash, settings.toml's settings, the bus, core plugins, plugins on a game module; and
//! MS2's game calls and restart with backoff.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use dainty_controller::registry::system_environment;
use dainty_controller::{
    MODULE_RESTARTED_EVENT, ModuleEntry, ModuleRegistry, ModuleSource, ModuleState, RegistryError, RegistryOptions,
    RestartPolicy, ServerController,
};
use dainty_module_api::host::file_sha256;
use dainty_module_api::manifest::{Capabilities, Transport};
use dainty_module_api::stub::{RecordingPlugin, StubModule, console};
use dainty_module_api::{
    BoxFuture, CommandRequest, ErrorKind, ExternalModule, GameAnswer, GameCall, GameModule, GameVersion, ModuleConfig,
    ModuleContext, ModuleError, ModuleId, ModuleKind, ModuleManifest, ModuleStatus, Phase, PortRange, Registrar,
    UnavailableReason,
};
use dainty_service_kit::context::{ComponentPhase, StatusRegistry};

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

    // A version the host kit's own stack serves is taken too, and a module settings.toml doesn't list never loads
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
        dainty_module_api::ErrorKind::NotFound
    );
    assert_eq!(reg.module_status(&id("stub")).await.unwrap().phase, Phase::Ready);
    // Its start event, then its answer to the host's server.ready
    let (from, started) = events.recv().await.unwrap();
    assert_eq!((from, started.name.as_str()), (Some(id("stub")), "stub.started"));
    reg.emit(dainty_module_api::Event {
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

    // A setting the module does not read is a typo in settings.toml: refused, not ignored
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
            api: dainty_module_api::API_VERSION,
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
        ("DAINTY_ADMIN_API_KEY", "secret"),
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

fn quick_restarts() -> RegistryOptions {
    let mut options = RegistryOptions::standalone();
    options.restart = RestartPolicy {
        first_delay: Duration::from_millis(50),
        max_delay: Duration::from_millis(200),
        window: Duration::from_secs(60),
        max_failures: 3,
    };
    options
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restartable_module_comes_back_after_it_dies() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("pid");
    let (reg, status) = registry(quick_restarts());
    reg.add_entry(external("stub", &["--pid-file", pid_file.to_str().unwrap()]).await)
        .unwrap();
    reg.set_restart(&id("stub"), true).unwrap();
    let mut events = reg.subscribe_events();
    reg.load(&id("stub")).await.unwrap();
    let first = read_when_written(&pid_file).await;
    std::fs::remove_file(&pid_file).unwrap();

    kill(&first);
    let restarted = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let (from, event) = events.recv().await.unwrap();
            if event.name.as_str() == MODULE_RESTARTED_EVENT {
                return (from, event);
            }
        }
    })
    .await
    .expect("no module.restarted event");
    assert_eq!(restarted.0, None, "the host sends it");
    assert_eq!(restarted.1.payload["module"], "stub");
    assert_eq!(restarted.1.payload["attempt"], 1);
    let second = read_when_written(&pid_file).await;
    assert_ne!(first, second);
    eventually("the restart", || {
        reg.info(&id("stub")).unwrap().state == ModuleState::Ready
    })
    .await;
    assert_eq!(phase(&status, "stub"), Some(ComponentPhase::Ready));
    assert_eq!(reg.command(&id("stub"), echo()).await.unwrap().text, "hello there");
    reg.unload_all().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_module_that_keeps_dying_is_left_failed() {
    let (reg, _) = registry(quick_restarts());
    reg.add_entry(external("stub", &["--die-after", "300"]).await).unwrap();
    reg.set_restart(&id("stub"), true).unwrap();
    _ = reg.load(&id("stub")).await;
    eventually("the give-up", || {
        reg.info(&id("stub"))
            .unwrap()
            .detail
            .is_some_and(|detail| detail.contains("not restarted: 3 failures"))
    })
    .await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    let info = reg.info(&id("stub")).unwrap();
    assert_eq!(info.state, ModuleState::Failed, "no further restart");
    assert!(info.detail.unwrap().contains("not restarted"));
    reg.unload_all().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_is_only_for_external_modules_and_off_by_default() {
    let (reg, _) = registry(quick_restarts());
    reg.add_entry(compiled_stub()).unwrap();
    assert!(matches!(
        reg.set_restart(&id("stub"), true),
        Err(RegistryError::Refused { .. })
    ));
    reg.set_restart(&id("stub"), false).unwrap();
    assert!(matches!(
        reg.set_restart(&id("absent"), true),
        Err(RegistryError::NotListed(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn game_calls_reach_the_game_module_of_the_version() {
    let (reg, _) = registry(RegistryOptions::standalone());
    reg.add_entry(compiled_stub()).unwrap();
    let missing = reg.game(&stub_version(), GameCall::Availability).await.unwrap_err();
    assert_eq!(missing.kind, ErrorKind::NotFound, "none loaded serves it");
    reg.load(&id("stub")).await.unwrap();

    match reg.game(&stub_version(), GameCall::Availability).await.unwrap() {
        GameAnswer::Availability(availability) => assert!(availability.up),
        other => panic!("{other:?}"),
    }
    let invite = reg
        .game(
            &stub_version(),
            GameCall::CreateInvite {
                uses: 1,
                expires_in_secs: Some(60),
                note: None,
                require_up: true,
            },
        )
        .await
        .unwrap();
    let GameAnswer::Invite { code } = invite else {
        panic!("not an invite");
    };
    assert_eq!(
        reg.game(&stub_version(), GameCall::RevokeInvite { code })
            .await
            .unwrap(),
        GameAnswer::Done
    );
    let other: GameVersion = "9.9.9".parse().unwrap();
    assert_eq!(
        reg.game(&other, GameCall::Servers).await.unwrap_err().kind,
        ErrorKind::NotFound
    );

    let all = reg.game_servers().await;
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].module, id("stub"));
    assert_eq!(all[0].version, stub_version());
    assert_eq!(all[0].servers.as_ref().unwrap().len(), 1, "the stub's hub");
    reg.unload_all().await;
    assert!(reg.game_servers().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn game_calls_cross_to_a_child_and_a_dead_one_is_not_ready() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("pid");
    let (reg, _) = registry(RegistryOptions::standalone());
    reg.add_entry(external("other-stub", &["--pid-file", pid_file.to_str().unwrap()]).await)
        .unwrap();
    reg.load(&id("other-stub")).await.unwrap();
    let all = reg.game_servers().await;
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].module, id("other-stub"));
    assert!(all[0].servers.is_ok());

    kill(&read_when_written(&pid_file).await);
    eventually("the failure", || {
        reg.info(&id("other-stub")).unwrap().state == ModuleState::Failed
    })
    .await;
    let error = reg.game(&stub_version(), GameCall::Availability).await.unwrap_err();
    assert_eq!(
        UnavailableReason::from_error(&error),
        Some(UnavailableReason::ModuleNotReady)
    );
    assert!(reg.game_servers().await.is_empty());
    reg.unload_all().await;
}
