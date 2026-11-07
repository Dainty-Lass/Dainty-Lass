//! A service built as a library (tests/echo-service) runs as a module of the controller on its env file, reports its
//! status through it, takes a module's registration as a core plugin, takes it again after a restart, and stops when
//! the controller does (RD27).
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use dainty_controller::{
    ModuleEntry, ModuleRegistry, ModuleSource, RegistryOptions, ServerController, ServiceLibrary, service_compiled_in,
    service_library,
};
use dainty_module_api::host::file_sha256;
use dainty_module_api::stub::StubModule;
use dainty_module_api::{CoreCapability, GameModule};
use dainty_service_kit::context::ComponentPhase;
use dainty_service_kit::env::Env;

/// Builds the fixture once per test binary, into its own target folder (the outer cargo holds this one's lock); two
/// builds at once would race on a library one test already has loaded.
fn echo_library() -> PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            let here = Path::new(env!("CARGO_MANIFEST_DIR"));
            let target = here.join("../../target/plugin-fixtures");
            let status = std::process::Command::new(env!("CARGO"))
                .args(["build", "--quiet", "--manifest-path"])
                .arg(here.join("tests/echo-service/Cargo.toml"))
                .arg("--target-dir")
                .arg(&target)
                // Not the outer cargo's jobserver or console: a nested build waiting on either hung the tests
                .env_remove("CARGO_MAKEFLAGS")
                .env_remove("MAKEFLAGS")
                .env_remove("MFLAGS")
                .stdin(std::process::Stdio::null())
                .output()
                .expect("cargo runs");
            let stderr = String::from_utf8_lossy(&status.stderr);
            assert!(status.status.success(), "the echo service builds: {stderr}");
            let name = format!(
                "{}echo_service{}",
                std::env::consts::DLL_PREFIX,
                std::env::consts::DLL_SUFFIX
            );
            target.join("debug").join(name)
        })
        .clone()
}

#[path = "support/watchdog.rs"]
mod watchdog;
use watchdog::watchdog;

async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    // 30 s: a freshly built library is scanned and loaded slowly on a busy machine
    for _ in 0..300 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("never: {what}");
}

fn lines(file: &Path) -> Vec<String> {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// The fixture's service itself, for the compiled-in run: the same code its library exports.
#[path = "echo-service/src/lib.rs"]
mod echo;

/// How the service is built into the controller: from its library or compiled in.
enum Built {
    Library(PathBuf),
    CompiledIn,
}

#[tokio::test(flavor = "multi_thread")]
async fn a_service_library_runs_reports_takes_registrations_and_stops() {
    let _watchdog = watchdog("the library run", Duration::from_secs(180));
    exercise(Built::Library(echo_library())).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_same_service_compiled_in_behaves_the_same() {
    let _watchdog = watchdog("the compiled-in run", Duration::from_secs(180));
    exercise(Built::CompiledIn).await;
}

/// The whole script, whichever way the service is built in: both must pass it unchanged.
async fn exercise(built: Built) {
    let dir = tempfile::tempdir().unwrap();
    let echo_file = dir.path().join("echo.txt");
    let env = Env::from_pairs([("ECHO_FILE", echo_file.to_str().unwrap()), ("ECHO_GREETING", "hi")]);

    let mut controller = ServerController::new("private", Vec::new());
    let registry = ModuleRegistry::new(RegistryOptions::standalone(), controller.status_registry().clone());
    let module = match built {
        Built::Library(path) => {
            let library = ServiceLibrary {
                name: "echo",
                sha256: file_sha256(&path).await.unwrap(),
                path,
                serves: &[CoreCapability::ContentFiles],
            };
            service_library(library, env, Some(&registry))
        }
        Built::CompiledIn => service_compiled_in(echo::Echo::default, env, Some(&registry)),
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
    let echo = || status.snapshot().get("echo").cloned();
    eventually("echo ready", || {
        echo().is_some_and(|c| c.phase == ComponentPhase::Ready)
    })
    .await;
    assert_eq!(echo().unwrap().detail, "greeting=hi");
    eventually("the stub attached", || {
        lines(&echo_file).contains(&String::from("attach stub dir=false"))
    })
    .await;
    assert_eq!(lines(&echo_file)[0], "start");

    // A restart is a new instance of the library, given what the registry attached again
    controller.restart_component("echo").await.unwrap();
    eventually("attached again after the restart", || {
        let seen = lines(&echo_file);
        seen.iter().filter(|line| *line == "start").count() == 2
            && seen.iter().filter(|line| *line == "attach stub dir=false").count() == 2
    })
    .await;
    eventually("echo ready again", || {
        echo().is_some_and(|c| c.phase == ComponentPhase::Ready)
    })
    .await;

    registry.unload(&"stub".parse().unwrap()).await.unwrap();
    eventually("the stub detached", || {
        lines(&echo_file).contains(&String::from("detach stub"))
    })
    .await;

    controller.shutdown().await;
    eventually("echo stopped", || {
        lines(&echo_file).last().is_some_and(|line| line == "stop")
    })
    .await;
}

#[tokio::test]
async fn a_service_library_that_does_not_match_its_pin_never_runs() {
    let _watchdog = watchdog("the pin check", Duration::from_secs(180));
    let path = echo_library();
    let library = ServiceLibrary {
        name: "echo",
        sha256: dainty_module_api::Sha256Hex::from_bytes([0; 32]),
        path,
        serves: &[],
    };
    let module = service_library(library, Env::from_pairs([]), None);
    let ctx = dainty_service_kit::context::ServiceContext::standalone("echo");
    let refused = (module.start)(ctx).await.unwrap_err();
    assert!(refused.contains("SHA-256"), "{refused}");
}
