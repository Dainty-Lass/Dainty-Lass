//! The chat service (tests/chat-service) over its own messages: it is set up, learns which modules answer chat,
//! asks one, reaches the home API, the event bus and a peer through the core, and answers a peer's call (RD27). Every
//! script runs both ways: loaded as a library and compiled in.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use dainty_controller::service_library::{chat_compiled_in, chat_library};
use dainty_controller::{ModuleEntry, ModuleRegistry, ModuleSource, RegistryOptions, ServerController, ServiceLibrary};
use dainty_module_api::host::file_sha256;
use dainty_module_api::network::{HomeApi, HomeError, HomeForward, NodeId, SessionTicket};
use dainty_module_api::stub::StubModule;
use dainty_module_api::{BoxFuture, CoreCapability, GameModule, GlobalId, NodeIdentity};
use dainty_service_kit::context::ComponentPhase;
use dainty_service_kit::env::Env;
use dainty_service_plugin::chat::{ChatNetwork, ChatSetup};
use network_plugin::{LinkError, NodeService, PeerLink, Service};

// If the binary ever freezes whole (0 CPU, the watchdog silent), attach `cdb -p <pid> -pv -c "~*kn 40;q"` for
// every thread's stack before killing it.
#[path = "support/watchdog.rs"]
mod watchdog;
use watchdog::watchdog;

fn chat_library_path() -> PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            let here = Path::new(env!("CARGO_MANIFEST_DIR"));
            let target = here.join("../../target/plugin-fixtures");
            let status = std::process::Command::new(env!("CARGO"))
                .args(["build", "--quiet", "--manifest-path"])
                .arg(here.join("tests/chat-service/Cargo.toml"))
                .arg("--target-dir")
                .arg(&target)
                // Not the outer cargo's jobserver or console, as tests/service_library.rs builds its fixture
                .env_remove("CARGO_MAKEFLAGS")
                .env_remove("MAKEFLAGS")
                .env_remove("MFLAGS")
                // The toolchain these tests were built with: unpinned, the nested rustc runs from the registry
                // folder, resolves the default `stable` there and has rustup update it mid-build
                .envs(option_env!("RUSTUP_TOOLCHAIN").map(|toolchain| ("RUSTUP_TOOLCHAIN", toolchain)))
                .stdin(std::process::Stdio::null())
                .output()
                .expect("cargo runs");
            let stderr = String::from_utf8_lossy(&status.stderr);
            assert!(status.status.success(), "the chat service builds: {stderr}");
            let name = format!(
                "{}chat_service{}",
                std::env::consts::DLL_PREFIX,
                std::env::consts::DLL_SUFFIX
            );
            target.join("debug").join(name)
        })
        .clone()
}

struct Home;

impl HomeApi for Home {
    fn resolve_home<'a>(&'a self, _: &'a GlobalId) -> BoxFuture<'a, Result<NodeIdentity, HomeError>> {
        Box::pin(async { Err(HomeError::NotInNetwork) })
    }

    fn session_ticket<'a>(&'a self, _: &'a GlobalId, _: &'a NodeId) -> BoxFuture<'a, Result<SessionTicket, HomeError>> {
        Box::pin(async { Err(HomeError::NotInNetwork) })
    }

    fn forward<'a>(&'a self, call: &'a HomeForward) -> BoxFuture<'a, Result<serde_json::Value, HomeError>> {
        Box::pin(async move {
            assert_eq!(call.operation.to_string(), "chat.login");
            assert_eq!(call.body["token"], "tok");
            Ok(serde_json::json!({ "name": "Homed" }))
        })
    }
}

/// A node link whose every peer answers with what it was sent.
struct Link;

impl PeerLink for Link {
    fn call<'a>(
        &'a self,
        to: &'a NodeId,
        service: Service,
        body: Vec<u8>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LinkError>> {
        Box::pin(async move {
            assert_eq!(service, Service::Chat);
            let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
            Ok(serde_json::to_vec(&serde_json::json!({ "to": to.to_string(), "echo": body })).unwrap())
        })
    }
}

async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    for _ in 0..100 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("never: {what}");
}

/// The fixture's service itself, for the compiled-in run: the same code its library exports.
#[path = "chat-service/src/lib.rs"]
mod chat_service;

fn lines(file: &Path) -> Vec<String> {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_chat_library_reaches_modules_home_events_and_peers_through_the_core() {
    let _watchdog = watchdog("the chat library run", Duration::from_secs(180));
    reaches_modules_home_events_and_peers(Some(chat_library_path())).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_same_chat_compiled_in_behaves_the_same() {
    let _watchdog = watchdog("the compiled-in chat run", Duration::from_secs(180));
    reaches_modules_home_events_and_peers(None).await;
}

/// The whole script, from the library at `path` or compiled in: both must pass it unchanged.
async fn reaches_modules_home_events_and_peers(path: Option<PathBuf>) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("chat.txt");
    let env = Env::from_pairs([("CHAT_FILE", file.to_str().unwrap())]);

    let mut controller = ServerController::new("private", Vec::new());
    let registry = ModuleRegistry::new(RegistryOptions::standalone(), controller.status_registry().clone());
    registry.set_home(Arc::new(Home));
    registry.node_services().set_peer_link(Some(Arc::new(Link)));
    let mut events = registry.subscribe_events();
    let node = NodeIdentity::standalone();
    let peer: NodeId = "uk-1".parse().unwrap();
    let setup = ChatSetup {
        network: Some(ChatNetwork {
            leader: node.node.clone(),
            peers: vec![(peer.clone(), node.region.clone())],
        }),
        node,
    };
    let module = match path {
        Some(path) => {
            let library = ServiceLibrary {
                name: "chat",
                sha256: file_sha256(&path).await.unwrap(),
                path,
                serves: &[],
            };
            chat_library(library, env, &registry, setup)
        }
        None => chat_compiled_in(chat_service::Chat::default, env, &registry, setup),
    };
    controller.add(module);
    registry
        .add_entry(ModuleEntry {
            id: "stub".parse().unwrap(),
            source: ModuleSource::CompiledIn(Arc::new(|| Arc::new(StubModule::new()) as Arc<dyn GameModule>)),
            autoload: true,
            settings: BTreeMap::new(),
        })
        .unwrap();
    controller.set_registry(registry.clone());
    controller.start_all().await.unwrap();

    let status = controller.status_registry().clone();
    assert_eq!(status.snapshot().get("chat").unwrap().phase, ComponentPhase::Ready);
    eventually("chat asked", || lines(&file).contains(&String::from("asked"))).await;
    let seen = lines(&file);
    assert_eq!(seen[0], "setup local true");
    assert_eq!(seen[1], "loaded stub");
    assert_eq!(seen[2], "jid 01j00000000000000000000000@stub.local");
    assert_eq!(seen[3], "nobody Unsupported");
    assert_eq!(seen[4], "account None");
    assert_eq!(seen[5], r#"home Some("Homed")"#);
    assert_eq!(seen[6], r#"peer {"to":"uk-1","echo":{"hi":1}}"#);

    let presence = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let (_, event) = events.recv().await.unwrap();
            if event.name.as_str() == dainty_module_api::chat::PRESENCE_EVENT {
                break event.payload;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(presence["user_id"], "user-1");
    assert_eq!(presence["online"], true);

    // A peer's call to this node's Service::Chat reaches the library and comes back
    let answer = registry
        .node_services()
        .serve(&peer, Service::Chat, br#"{"ping":true}"#.to_vec())
        .await
        .unwrap();
    let answer: serde_json::Value = serde_json::from_slice(&answer).unwrap();
    assert_eq!(answer, serde_json::json!({ "from": "uk-1", "body": { "ping": true } }));
    let refused = registry
        .node_services()
        .serve(&peer, Service::Chat, b"not json".to_vec())
        .await;
    assert!(matches!(refused, Err(LinkError::Protocol { .. })), "{refused:?}");

    registry.unload(&"stub".parse().unwrap()).await.unwrap();
    eventually("the stub unloaded", || {
        lines(&file).contains(&String::from("unloaded stub"))
    })
    .await;

    controller.shutdown().await;
    eventually("chat stopped", || {
        lines(&file).last().is_some_and(|line| line == "stop")
    })
    .await;
    assert!(!registry.node_services().is_attached(Service::Chat));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chat_library_run_outside_chat_library_gets_no_setup() {
    let _watchdog = watchdog("the library without setup", Duration::from_secs(180));
    let path = chat_library_path();
    let library = ServiceLibrary {
        name: "chat",
        sha256: file_sha256(&path).await.unwrap(),
        path,
        serves: &[CoreCapability::Chat],
    };
    no_setup(dainty_controller::service_library(library, Env::from_pairs([]), None)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chat_compiled_in_outside_chat_compiled_in_gets_no_setup() {
    let _watchdog = watchdog("the compiled-in without setup", Duration::from_secs(180));
    no_setup(dainty_controller::service_compiled_in(
        chat_service::Chat::default,
        Env::from_pairs([]),
        None,
    ))
    .await;
}

/// Without chat's own core side there is no setup, so the service's run ends on its own.
async fn no_setup(module: dainty_controller::Module) {
    let ctx = dainty_service_kit::context::ServiceContext::standalone("chat");
    let ended = tokio::time::timeout(Duration::from_secs(10), (module.start)(ctx))
        .await
        .unwrap();
    assert_eq!(ended.unwrap_err(), "no setup");
}
