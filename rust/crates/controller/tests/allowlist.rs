//! The allowlist as a core plugin on the registry (DF4d): a game module's UDP ranges join the firewall rule when it
//! loads and leave it when it unloads, a server with no module guards exactly its own game ports, and a public
//! server whose allowlist runs elsewhere refuses a module that wants UDP ports. Every helper here is a dry run.
#![cfg(feature = "allowlist")]
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use controller::{
    ModuleEntry, ModuleRegistry, ModuleSource, ModuleState, RegistryError, RegistryOptions, ServerController, modules,
};
use module_api::manifest::Transport;
use module_api::stub::StubModule;
use module_api::{GameModule, ModuleId, PortRange};
use serde_json::Value;
use service_kit::env::Env;

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

async fn server(dir: &Path, options: RegistryOptions) -> (ServerController, ModuleRegistry) {
    let mut controller = ServerController::new("public", Vec::new());
    let registry = ModuleRegistry::new(options, controller.status_registry().clone());
    let slot = modules::allowlist_slot();
    controller.add(modules::allowlist(&allowlist_env(dir), slot, Some(registry.clone())).unwrap());
    controller.set_registry(registry.clone());
    controller.start_all().await.unwrap();
    (controller, registry)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_loaded_module_s_udp_range_is_guarded_and_taken_back_on_unload() {
    let dir = tempfile::tempdir().unwrap();
    let (mut controller, registry) = server(dir.path(), RegistryOptions::standalone()).await;
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
    let dir = tempfile::tempdir().unwrap();
    let mut options = RegistryOptions::standalone();
    options.reserved_ports = vec![(
        Transport::Udp,
        PortRange {
            first: 65010,
            last: 65019,
        },
    )];
    let (mut controller, registry) = server(dir.path(), options).await;
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
    let dir = tempfile::tempdir().unwrap();
    // The controller is told none of the server's ports, so only the allowlist stands in the way
    let (mut controller, registry) = server(dir.path(), RegistryOptions::standalone()).await;
    let mut entry = stub_entry();
    entry.source = ModuleSource::CompiledIn(Arc::new(|| {
        Arc::new(OnGamePorts(StubModule::new())) as Arc<dyn GameModule>
    }));
    registry.add_entry(entry).unwrap();
    let error = registry.load(&stub_id()).await.unwrap_err();
    assert!(matches!(error, RegistryError::Refused { .. }), "{error}");
    assert!(error.to_string().contains("allowlist"), "{error}");
    assert_eq!(registry.info(&stub_id()).unwrap().state, ModuleState::Failed);
    eventually("the first rule", || applied(dir.path()) == ["8770-8777"]).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(applied(dir.path()), ["8770-8777"]);
    controller.shutdown().await;
}

/// The stub, with its UDP range on the server's own game ports.
struct OnGamePorts(StubModule);

impl GameModule for OnGamePorts {
    fn manifest(&self) -> module_api::ModuleManifest {
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

    fn register(&self, registrar: &mut module_api::Registrar) -> Result<(), module_api::ModuleError> {
        self.0.register(registrar)
    }

    fn start(
        &self,
        ctx: module_api::ModuleContext,
        config: module_api::ModuleConfig,
    ) -> module_api::BoxFuture<'_, Result<(), module_api::ModuleError>> {
        self.0.start(ctx, config)
    }

    fn stop(&self, ctx: module_api::ModuleContext) -> module_api::BoxFuture<'_, Result<(), module_api::ModuleError>> {
        self.0.stop(ctx)
    }

    fn status(&self) -> module_api::ModuleStatus {
        self.0.status()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_public_server_whose_allowlist_runs_elsewhere_refuses_udp_ports() {
    let controller = ServerController::new("public", Vec::new());
    let registry = ModuleRegistry::new(RegistryOptions::standalone(), controller.status_registry().clone());
    modules::allowlist_absent(&registry);
    registry.add_entry(stub_entry()).unwrap();
    let error = registry.load(&stub_id()).await.unwrap_err().to_string();
    assert!(error.contains("outside this process"), "{error}");
    registry.unload_all().await;
}

#[cfg(feature = "network")]
#[tokio::test(flavor = "multi_thread")]
async fn matched_player_is_admitted_into_in_process_allowlist_and_unmatched_is_not() {
    use module_api::GameVersion;
    use module_api::network::{GlobalId, IdKind, NodeId, NodeIdentity, StandaloneHome};
    use network_plugin::Clock;
    use network_plugin::inprocess::InProcessNetwork;
    use network_plugin::ring::{DirectorPlacement, DirectorPlacementError, RingConfig, RingNode};

    struct StubUdpDirector;
    impl DirectorPlacement for StubUdpDirector {
        fn place_hunt(
            &self,
            _hunt_id: &GlobalId,
            _queue: &str,
            _game: &GameVersion,
            _players: &[GlobalId],
        ) -> Result<(String, u16), DirectorPlacementError> {
            Ok(("127.0.0.1".into(), 65012))
        }

        fn place_hub(
            &self,
            _hub_id: &GlobalId,
            _region: &module_api::network::Region,
            _shard: u32,
            _game: &GameVersion,
        ) -> Result<(String, u16), DirectorPlacementError> {
            Ok(("127.0.0.1".into(), 65013))
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let mut controller = ServerController::new("public", Vec::new());
    let registry = ModuleRegistry::new(RegistryOptions::standalone(), controller.status_registry().clone());
    let slot = modules::allowlist_slot();
    controller.add(modules::allowlist(&allowlist_env(dir.path()), slot.clone(), Some(registry.clone())).unwrap());
    controller.set_registry(registry.clone());
    controller.start_all().await.unwrap();

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
        Arc::new(modules::InProcessAllowlistSink::new(Some(slot.clone()))),
    );

    let game: GameVersion = "0.0.1".parse().unwrap();
    let matched_account = GlobalId::new(IdKind::Account, &node);
    ring.submit_ticket(matched_account, "patrol_blaze", game.clone(), 1, "203.0.113.42")
        .await
        .unwrap();
    let formed = ring.try_form_matches("patrol_blaze", &game).await.unwrap();
    assert_eq!(formed.len(), 1);
    assert_eq!(formed[0].server_port, 65012);

    let helper = slot.lock().unwrap().as_ref().cloned().unwrap();
    // Matched player's IP is admitted on the guarded UDP port 65012
    assert!(helper.is_admitted("203.0.113.42", 65012));
    // Unmatched player's IP is not admitted
    assert!(!helper.is_admitted("203.0.113.99", 65012));
    // Port outside the guarded ranges is not admitted even for the matched IP
    assert!(!helper.is_admitted("203.0.113.42", 59999));

    controller.shutdown().await;
}
