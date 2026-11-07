//! The stub module as a library (tests/stub-plugin) answers as it does compiled in, makes its calls into the host
//! across the boundary, and a library that doesn't match its pin is never loaded (RD27).
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use dainty_module_api::host::file_sha256;
use dainty_module_api::stub::{StubModule, console, recording_host};
use dainty_module_api::{
    ChildOptions, CommandRequest, ErrorKind, ExternalModule, GlobalId, HostCall, IdKind, InProcessModule, LoadError,
    ModuleConfig, ModuleDriver, ModuleDriverExt, NodeIdentity, Sha256Hex,
};
use dainty_plugin_host::{DylibModule, is_library};

/// Builds the fixture once per test binary, into its own target folder (the outer cargo holds this one's lock).
fn stub_library() -> &'static Path {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT.get_or_init(|| {
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        let target = here.join("../../target/plugin-fixtures");
        let status = std::process::Command::new(env!("CARGO"))
            .args(["build", "--quiet", "--manifest-path"])
            .arg(here.join("tests/stub-plugin/Cargo.toml"))
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
        assert!(status.status.success(), "the stub plugin builds: {stderr}");
        let name = format!(
            "{}stub_plugin{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_SUFFIX
        );
        target.join("debug").join(name)
    })
}

async fn external(path: &Path) -> ExternalModule {
    ExternalModule {
        path: path.to_owned(),
        sha256: file_sha256(path).await.unwrap(),
        args: vec![],
        expected: Some("stub".parse().unwrap()),
    }
}

/// The calls the comparison makes, with what came back, as text.
async fn script(driver: &dyn ModuleDriver) -> Vec<String> {
    let mut out = vec![format!(
        "manifest: {}",
        serde_json::to_string(driver.manifest()).unwrap()
    )];
    let registration = driver.register().await.unwrap();
    out.push(format!(
        "registration: {}",
        serde_json::to_string(&registration).unwrap()
    ));
    let config = ModuleConfig::defaults(driver.manifest(), &registration);
    out.push(format!("start: {:?}", driver.start(config).await));
    let command = |name: &str, args: &[&str]| CommandRequest {
        actor: console(),
        name: name.into(),
        args: args.iter().map(|s| s.to_string()).collect(),
    };
    out.push(format!(
        "echo: {:?}",
        driver.command(command("echo", &["a", "b"])).await
    ));
    // A host call the module makes while the host waits on its answer
    let ours = GlobalId::new(IdKind::Account, &NodeIdentity::standalone().node);
    out.push(format!(
        "home: {:?}",
        driver.command(command("home", &[&ours.to_string()])).await
    ));
    out.push(format!("fail: {:?}", driver.command(command("fail", &[])).await));
    out.push(format!("status: {:?}", driver.status().await));
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn the_stub_answers_the_same_compiled_in_and_as_a_library() {
    let in_host = recording_host();
    let in_process =
        InProcessModule::new(Arc::new(StubModule::new()), NodeIdentity::standalone(), in_host.clone()).unwrap();
    let in_answers = script(&in_process).await;

    let path = stub_library();
    assert!(is_library(path));
    let lib_host = recording_host();
    let library = DylibModule::load(
        &external(path).await,
        NodeIdentity::standalone(),
        lib_host.clone(),
        ChildOptions::default(),
    )
    .await
    .unwrap();
    assert!(library.failure().is_none());
    let lib_answers = script(&library).await;
    assert_eq!(in_answers, lib_answers);
    assert!(
        lib_answers.iter().any(|line| line.starts_with("echo: Ok(")),
        "{lib_answers:#?}"
    );

    let calls = |host: &dainty_module_api::stub::RecordingHost| {
        host.calls()
            .into_iter()
            .filter(|(_, call)| matches!(call, HostCall::Home(_)))
            .count()
    };
    assert_eq!(calls(&in_host), calls(&lib_host));
    assert!(calls(&lib_host) > 0);

    library.unload().await.unwrap();
    assert_eq!(library.status().await.unwrap_err().kind, ErrorKind::Unavailable);

    // Loaded again from the same file: the library stays loaded, the module is a new one
    let again = DylibModule::load(
        &external(path).await,
        NodeIdentity::standalone(),
        recording_host(),
        ChildOptions::default(),
    )
    .await
    .unwrap();
    assert!(again.status().await.is_ok());
    again.unload().await.unwrap();
}

#[tokio::test]
async fn a_library_that_does_not_match_its_pin_is_never_loaded() {
    let path = stub_library();
    let mut module = external(path).await;
    module.sha256 = Sha256Hex::from_bytes([0; 32]);
    let refused = DylibModule::load(
        &module,
        NodeIdentity::standalone(),
        recording_host(),
        ChildOptions::default(),
    )
    .await;
    assert!(
        matches!(refused, Err(LoadError::HashMismatch { .. })),
        "{:?}",
        refused.err()
    );
}

#[tokio::test]
async fn a_file_that_is_not_a_module_library_is_refused() {
    let dir = std::env::temp_dir().join(format!("plugin-host-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("not-a-plugin{}", std::env::consts::DLL_SUFFIX));
    std::fs::write(&path, b"not a library").unwrap();
    let refused = DylibModule::load(
        &external(&path).await,
        NodeIdentity::standalone(),
        recording_host(),
        ChildOptions::default(),
    )
    .await;
    assert!(matches!(refused, Err(LoadError::Spawn { .. })), "{:?}", refused.err());
    _ = std::fs::remove_dir_all(&dir);
}
