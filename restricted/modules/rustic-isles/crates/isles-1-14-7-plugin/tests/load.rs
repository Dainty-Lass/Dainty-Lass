//! The core loads the built library and it is the MIT module the compiled-in build is (RD27).
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::path::PathBuf;

use module_api::host::file_sha256;
use module_api::stub::recording_host;
use module_api::{ChildOptions, ExternalModule, GameModule, ModuleDriver, ModuleDriverExt, NodeIdentity};
use plugin_host::DylibModule;

/// The library cargo built beside this test (target/<profile>/), before running it.
fn library() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let profile = exe.parent().and_then(|deps| deps.parent()).unwrap();
    let name = format!(
        "{}isles_1_14_7_plugin{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    profile.join(name)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_core_loads_the_library_and_it_is_the_1_14_7_module() {
    let path = library();
    // A cdylib is not built for its own tests, and a nested cargo would wait on this one's lock
    assert!(
        path.is_file(),
        "{} is missing: run `cargo build -p isles-1-14-7-plugin` before the tests (CI does)",
        path.display()
    );
    let compiled_in = rustic_isles_1_14_7::module().unwrap().manifest();
    let module = ExternalModule {
        sha256: file_sha256(&path).await.unwrap(),
        path,
        args: vec![],
        expected: Some(compiled_in.id.clone()),
    };
    let library = DylibModule::load(
        &module,
        NodeIdentity::standalone(),
        recording_host(),
        ChildOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::to_value(library.manifest()).unwrap(),
        serde_json::to_value(&compiled_in).unwrap()
    );
    let registration = library.register().await.unwrap();
    assert!(!registration.routes.is_empty());
    library.unload().await.unwrap();
}
