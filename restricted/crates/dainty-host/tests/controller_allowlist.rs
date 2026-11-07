//! The allowlist as a core plugin on the registry (DF4d), inside the gateway service: a game module's UDP ranges join
//! the firewall rule when it loads and leave it when it unloads, a server with no module guards exactly its own game
//! ports, and a public server whose allowlist runs elsewhere refuses a module that wants UDP ports. Every helper here
//! is a dry run, and every script runs with the gateway compiled in and loaded as a library (RD27):
//! `cargo build -p dainty-gateway-plugin` first.
#![cfg(feature = "allowlist")]
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use dainty_host::compose as modules;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use dainty_controller::service_library::gateway::{AllowlistMode, GatewayHandle};
use dainty_controller::{
    ModuleEntry, ModuleRegistry, ModuleSource, ModuleState, RegistryError, RegistryOptions, ServerController,
};
use dainty_module_api::manifest::Transport;
use dainty_module_api::stub::StubModule;
use dainty_module_api::{GameModule, ModuleId, PortRange};
use dainty_service_kit::env::Env;
use serde_json::Value;

fn stub_id() -> ModuleId {
    "stub".parse().unwrap()
}

fn stub_entry() -> ModuleEntry {
    ModuleEntry {
        id: stub_id(),
        source: ModuleSource::CompiledIn(Arc::new(|| Arc::new(StubModule::new()) as Arc<dyn GameModule>)),
        autoload: false,
        settings: BTreeMap::new(),
    }
}

fn allowlist_env(dir: &Path) -> Env {
    // The helper's loopback port: any free one
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let file = |name: &str| dir.join(name).to_string_lossy().into_owned();
    let (port, secret) = (port.to_string(), "a".repeat(40));
    let (audit, state) = (file("audit.log"), file("state.json"));
    Env::from_pairs([
        ("ALLOWLIST_DRY_RUN", "1"),
        ("ALLOWLIST_SECRET", secret.as_str()),
        ("ALLOWLIST_PORT", port.as_str()),
        ("ALLOWLIST_FIREWALL", "nftables"),
        ("ALLOWLIST_PORTS", "8770-8777"),
        ("ALLOWLIST_AUDIT_LOG", audit.as_str()),
        ("ALLOWLIST_STATE_FILE", state.as_str()),
    ])
}

/// The ports of every rule the helper applied, in order.
fn applied(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("audit.log"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|event| event["event"] == "dry_run")
        .filter_map(|event| event["ports"].as_str().map(str::to_owned))
        .collect()
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

/// How the gateway is built into the core: from its library or compiled in.
#[derive(Clone, Copy, Debug)]
enum Built {
    Library,
    CompiledIn,
}

const BOTH: [Built; 2] = [Built::CompiledIn, Built::Library];

fn gateway_env() -> Env {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
        .to_string();
    let secret = "s".repeat(64);
    Env::from_pairs([
        ("GATEWAY_BIND", "127.0.0.1"),
        ("GATEWAY_TLS", "0"),
        ("GATEWAY_PORT", port.as_str()),
        ("GATEWAY_SECRET", secret.as_str()),
        ("GATEWAY_ALLOWLIST", "0"),
    ])
}

/// The gateway with `allowlist`, as `host` composes it, not yet added to a controller.
async fn gateway(
    built: Built,
    allowlist: AllowlistMode,
    registry: &ModuleRegistry,
) -> (dainty_controller::Module, GatewayHandle) {
    match built {
        Built::CompiledIn => modules::gateway(&gateway_env(), allowlist, Some(registry.clone())).unwrap(),
        Built::Library => modules::gateway_library(
            gateway_library().await,
            &gateway_env(),
            allowlist,
            Some(registry.clone()),
        )
        .unwrap(),
    }
}

/// A public server whose gateway runs the allowlist in `dir`, as `serve` composes it.
async fn server(
    dir: &Path,
    options: RegistryOptions,
    built: Built,
) -> (ServerController, ModuleRegistry, GatewayHandle) {
    let mut controller = ServerController::new("public", Vec::new());
    let registry = ModuleRegistry::new(options, controller.status_registry().clone());
    let allowlist = modules::allowlist(&allowlist_env(dir)).unwrap();
    let (module, handle) = gateway(built, allowlist, &registry).await;
    controller.add(module);
    controller.set_registry(registry.clone());
    controller.start_all().await.unwrap();
    (controller, registry, handle)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_loaded_module_s_udp_range_is_guarded_and_taken_back_on_unload() {
    for built in BOTH {
        guarded_and_taken_back(built).await;
    }
}

async fn guarded_and_taken_back(built: Built) {
    let dir = tempfile::tempdir().unwrap();
    let (mut controller, registry, _) = server(dir.path(), RegistryOptions::standalone(), built).await;
    registry.add_entry(stub_entry()).unwrap();

    // No module loaded: today's range, exactly
    eventually("the first rule", || applied(dir.path()) == ["8770-8777"]).await;

    registry.load(&stub_id()).await.unwrap();
    eventually("the module's range in the rule", || {
        applied(dir.path())
            .last()
            .is_some_and(|ports| ports == "8770-8777,65010-65019")
    })
    .await;

    registry.unload(&stub_id()).await.unwrap();
    eventually("the range out of the rule", || {
        applied(dir.path()).last().is_some_and(|ports| ports == "8770-8777")
    })
    .await;

    // Loaded again: it is guarded again
    registry.load(&stub_id()).await.unwrap();
    eventually("the range back", || {
        applied(dir.path())
            .last()
            .is_some_and(|ports| ports == "8770-8777,65010-65019")
    })
    .await;
    controller.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_range_the_controller_moved_to_is_the_one_guarded() {
    for built in BOTH {
        moved_range_guarded(built).await;
    }
}

async fn moved_range_guarded(built: Built) {
    let dir = tempfile::tempdir().unwrap();
    let mut options = RegistryOptions::standalone();
    options.reserved_ports = vec![(
        Transport::Udp,
        PortRange {
            first: 65010,
            last: 65019,
        },
    )];
    let (mut controller, registry, _) = server(dir.path(), options, built).await;
    registry.add_entry(stub_entry()).unwrap();
    let info = registry.load(&stub_id()).await.unwrap();
    assert_eq!(
        info.ports["game"],
        PortRange {
            first: 65020,
            last: 65029
        }
    );
    eventually("the moved range in the rule", || {
        applied(dir.path())
            .last()
            .is_some_and(|ports| ports == "8770-8777,65020-65029")
    })
    .await;
    controller.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_range_inside_the_server_s_own_game_ports_is_refused_and_leaves_the_rule_alone() {
    for built in BOTH {
        own_game_ports_refused(built).await;
    }
}

async fn own_game_ports_refused(built: Built) {
    let dir = tempfile::tempdir().unwrap();
    // The controller is told none of the server's ports, so only the allowlist stands in the way
    let (mut controller, registry, _) = server(dir.path(), RegistryOptions::standalone(), built).await;
    let mut entry = stub_entry();
    entry.source = ModuleSource::CompiledIn(Arc::new(|| {
        Arc::new(OnGamePorts(StubModule::new())) as Arc<dyn GameModule>
    }));
    registry.add_entry(entry).unwrap();
    let error = registry.load(&stub_id()).await.unwrap_err();
    assert!(matches!(error, RegistryError::Refused { .. }), "{error}");
    // The gateway service is the plugin that refuses it, the allowlist inside it the reason
    assert!(error.to_string().contains("the server's own game ports"), "{error}");
    assert_eq!(registry.info(&stub_id()).unwrap().state, ModuleState::Failed);
    eventually("the first rule", || applied(dir.path()) == ["8770-8777"]).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(applied(dir.path()), ["8770-8777"]);
    controller.shutdown().await;
}

/// The stub, with its UDP range on the server's own game ports.
struct OnGamePorts(StubModule);

impl GameModule for OnGamePorts {
    fn manifest(&self) -> dainty_module_api::ModuleManifest {
        let mut manifest = self.0.manifest();
        for port in &mut manifest.ports {
            if port.transport == Transport::Udp {
                port.range = PortRange {
                    first: 8774,
                    last: 8783,
                };
            }
        }
        manifest
    }

    fn register(&self, registrar: &mut dainty_module_api::Registrar) -> Result<(), dainty_module_api::ModuleError> {
        self.0.register(registrar)
    }

    fn start(
        &self,
        ctx: dainty_module_api::ModuleContext,
        config: dainty_module_api::ModuleConfig,
    ) -> dainty_module_api::BoxFuture<'_, Result<(), dainty_module_api::ModuleError>> {
        self.0.start(ctx, config)
    }

    fn stop(
        &self,
        ctx: dainty_module_api::ModuleContext,
    ) -> dainty_module_api::BoxFuture<'_, Result<(), dainty_module_api::ModuleError>> {
        self.0.stop(ctx)
    }

    fn status(&self) -> dainty_module_api::ModuleStatus {
        self.0.status()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_public_server_whose_allowlist_runs_elsewhere_refuses_udp_ports() {
    for built in BOTH {
        let mut controller = ServerController::new("public", Vec::new());
        let registry = ModuleRegistry::new(RegistryOptions::standalone(), controller.status_registry().clone());
        let (module, _) = gateway(built, AllowlistMode::Absent, &registry).await;
        controller.add(module);
        controller.set_registry(registry.clone());
        controller.start_all().await.unwrap();
        registry.add_entry(stub_entry()).unwrap();
        let error = registry.load(&stub_id()).await.unwrap_err().to_string();
        assert!(error.contains("outside this process"), "{built:?}: {error}");
        controller.shutdown().await;
    }
}

/// The addresses the dry-run helper added, in order.
fn added(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("audit.log"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|event| event["event"] == "added")
        .filter_map(|event| event["ip"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn matched_player_is_admitted_into_in_process_allowlist_and_unmatched_is_not() {
    for built in BOTH {
        matched_player_admitted(built).await;
    }
}

async fn matched_player_admitted(built: Built) {
    use dainty_module_api::GameVersion;
    use dainty_module_api::network::{GlobalId, IdKind, NodeId, NodeIdentity, StandaloneHome};
    use network_plugin::Clock;
    use network_plugin::inprocess::InProcessNetwork;
    use network_plugin::ring::{DirectorPlacement, DirectorPlacementError, RingConfig, RingNode};

    struct StubUdpDirector;
    impl DirectorPlacement for StubUdpDirector {
        fn place_hunt<'a>(
            &'a self,
            _hunt_id: &'a GlobalId,
            _queue: &'a str,
            _game: &'a GameVersion,
            _players: &'a [GlobalId],
        ) -> dainty_module_api::BoxFuture<'a, Result<(String, u16), DirectorPlacementError>> {
            Box::pin(async { Ok(("127.0.0.1".into(), 65012)) })
        }

        fn place_hub<'a>(
            &'a self,
            _hub_id: &'a GlobalId,
            _region: &'a dainty_module_api::network::Region,
            _shard: u32,
            _game: &'a GameVersion,
        ) -> dainty_module_api::BoxFuture<'a, Result<(String, u16), DirectorPlacementError>> {
            Box::pin(async { Ok(("127.0.0.1".into(), 65013)) })
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let (mut controller, registry, handle) = server(dir.path(), RegistryOptions::standalone(), built).await;

    // Load stub game module so its UDP range 65010-65019 is guarded by the allowlist helper (DF4d)
    registry.add_entry(stub_entry()).unwrap();
    registry.load(&stub_id()).await.unwrap();
    eventually("the module's range in the rule", || {
        applied(dir.path())
            .last()
            .is_some_and(|ports| ports == "8770-8777,65010-65019")
    })
    .await;

    let node: NodeId = "oregon-1".parse().unwrap();
    let identity = NodeIdentity {
        node: node.clone(),
        region: "oregon".parse().unwrap(),
    };
    let net = InProcessNetwork::new();
    let mut cfg = RingConfig::new(identity.clone());
    cfg.match_size = 1;
    let ring = RingNode::new(
        cfg,
        Arc::new(Clock::default()),
        Arc::new(net.link(&node)),
        Arc::new(StandaloneHome { identity }),
        Arc::new(StubUdpDirector),
        handle.allowlist_sink(),
    );

    let game: GameVersion = "0.0.1".parse().unwrap();
    let matched_account = GlobalId::new(IdKind::Account, &node);
    ring.submit_ticket(matched_account, "patrol_blaze", game.clone(), 1, "203.0.113.42")
        .await
        .unwrap();
    let formed = ring.try_form_matches("patrol_blaze", &game).await.unwrap();
    assert_eq!(formed.len(), 1);
    assert_eq!(formed[0].server_port, 65012);

    // Matched player's IP is admitted on the guarded UDP port 65012
    eventually("the matched player admitted", || added(dir.path()) == ["203.0.113.42"]).await;
    // A port outside the guarded ranges admits no one
    handle.allowlist_sink().admit_player(
        &GlobalId::new(IdKind::Hunt, &node),
        &GlobalId::new(IdKind::Account, &node),
        "203.0.113.77",
        59999,
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    // Unmatched player's IP is not admitted
    assert_eq!(added(dir.path()), ["203.0.113.42"]);

    controller.shutdown().await;
}

#[tokio::test]
async fn refused_udp_admission_logs_warn_with_instance_and_port_without_panicking() {
    use std::fmt;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};

    use dainty_module_api::network::{GlobalId, IdKind, NodeId};
    use tracing::field::{Field, Visit};
    use tracing::{Event, Level, Metadata, Subscriber, span};

    #[derive(Default)]
    struct CapturedWarns {
        events: Mutex<Vec<String>>,
        next_id: AtomicU64,
    }

    struct FieldPrinter(String);

    impl Visit for FieldPrinter {
        fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
            use fmt::Write as _;
            let _ = write!(&mut self.0, "{}={:?} ", field.name(), value);
        }
    }

    impl Subscriber for CapturedWarns {
        fn enabled(&self, metadata: &Metadata<'_>) -> bool {
            metadata.level() <= &Level::WARN
        }

        fn new_span(&self, _span: &span::Attributes<'_>) -> span::Id {
            span::Id::from_u64(self.next_id.fetch_add(1, Ordering::Relaxed) + 1)
        }

        fn record(&self, _span: &span::Id, _values: &span::Record<'_>) {}

        fn record_follows_from(&self, _span: &span::Id, _follows: &span::Id) {}

        fn event(&self, event: &Event<'_>) {
            if *event.metadata().level() == Level::WARN {
                let mut printer = FieldPrinter(String::new());
                event.record(&mut printer);
                self.events.lock().unwrap().push(printer.0);
            }
        }

        fn enter(&self, _span: &span::Id) {}

        fn exit(&self, _span: &span::Id) {}
    }

    let node: NodeId = "oregon-1".parse().unwrap();
    let instance = GlobalId::new(IdKind::Hunt, &node);
    let account = GlobalId::new(IdKind::Account, &node);
    // A gateway that isn't running admits no one
    let controller = ServerController::new("public", Vec::new());
    let registry = ModuleRegistry::new(RegistryOptions::standalone(), controller.status_registry().clone());
    let (_, handle) = gateway(Built::CompiledIn, AllowlistMode::Off, &registry).await;
    let sink = handle.allowlist_sink();
    let subscriber = Arc::new(CapturedWarns::default());

    tracing::subscriber::with_default(subscriber.clone(), || {
        sink.admit_player(&instance, &account, "203.0.113.77", 59999);
    });

    let events = subscriber.events.lock().unwrap();
    assert_eq!(events.len(), 1, "expected one warn event, got {events:?}");
    let line = &events[0];
    assert!(line.contains(&instance.to_string()), "missing instance id in {line}");
    assert!(line.contains("59999"), "missing udp port in {line}");
    assert!(
        !line.contains("203.0.113.77"),
        "warn event must not log full client IP: {line}"
    );
}

/// The gateway library cargo built beside this test (target/<profile>/): `cargo build -p dainty-gateway-plugin` first.
async fn gateway_library() -> dainty_controller::ServiceLibrary {
    let exe = std::env::current_exe().unwrap();
    let profile = exe.parent().and_then(|deps| deps.parent()).unwrap();
    let path = profile.join(format!(
        "{}dainty_gateway_plugin{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    assert!(
        path.is_file(),
        "{} is missing: run `cargo build -p dainty-gateway-plugin` before the tests (CI does)",
        path.display()
    );
    dainty_controller::ServiceLibrary {
        name: "gateway",
        sha256: dainty_module_api::host::file_sha256(&path).await.unwrap(),
        path,
        serves: &[
            dainty_module_api::CoreCapability::Routes,
            dainty_module_api::CoreCapability::UdpPorts,
        ],
    }
}
