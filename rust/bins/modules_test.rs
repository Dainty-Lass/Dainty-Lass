//! Shared by both bins' `tests/modules.rs` (DF5): whatever game modules the bin was built with start on the
//! registry, and stop with it. The bin's Cargo features decide the set, so the verify runs this with none, one and
//! every module (docs/server-modules.md has the three commands).
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::collections::BTreeSet;

use controller::{ModuleEntry, ModuleRegistry, ModuleSource, ModuleState, RegistryOptions, ServerController};

/// The ids the enabled features link.
fn expected() -> BTreeSet<&'static str> {
    let mut ids = BTreeSet::new();
    if cfg!(feature = "stub-module") {
        ids.insert("stub");
    }
    if cfg!(feature = "isles-1-14-7") {
        ids.insert("isles-1-14-7");
    }
    if cfg!(feature = "isles-1-12-0") {
        ids.insert("isles-1-12-0");
    }
    ids
}

#[tokio::test(flavor = "multi_thread")]
async fn the_modules_this_build_links_are_the_ones_its_features_name_and_all_of_them_start() {
    let factories = controller::modules::game_modules();
    let linked: BTreeSet<String> = factories.iter().map(|f| f().manifest().id.to_string()).collect();
    assert_eq!(linked, expected().into_iter().map(str::to_owned).collect());
    assert_eq!(
        controller::modules::UNLINKED.contains(&"revived-1-4-4"),
        cfg!(feature = "revived-1-4-4"),
        "revived-1-4-4 links nothing yet (RV1)"
    );

    let mut server = ServerController::new("private", Vec::new());
    let registry = ModuleRegistry::new(RegistryOptions::standalone(), server.status_registry().clone());
    for factory in factories {
        let id = factory().manifest().id;
        registry
            .add_entry(ModuleEntry {
                id,
                source: ModuleSource::CompiledIn(factory),
                autoload: true,
                settings: Default::default(),
            })
            .unwrap();
    }
    server.set_registry(registry.clone());
    server.start_all().await.unwrap();

    let listed = registry.list();
    assert_eq!(listed.len(), linked.len());
    for info in &listed {
        assert_eq!(info.state, ModuleState::Ready, "{}: {:?}", info.id, info.detail);
    }
    // Two modules never share a port: the controller moved the second off the first's
    let mut taken = BTreeSet::new();
    for info in &listed {
        for (name, range) in &info.ports {
            for port in range.first..=range.last {
                assert!(taken.insert(port), "{} {name}: port {port} is held twice", info.id);
            }
        }
    }
    server.shutdown().await;
    for info in registry.list() {
        assert_eq!(
            info.state,
            ModuleState::NotLoaded,
            "{} stopped with the server",
            info.id
        );
    }
}
