//! The 1.14.7 game module as a library the core loads from `host.toml`'s `[[modules.external]]` (RD27): dainty-lass's
//! clean-room `rustic-isles-1-14-7` with restricted's version data compiled in, as the compiled-in host has it
//! (`crates/dainty-host/src/version_data.rs`). isles-mit-parity's `version_data` test keeps the document current.

use rustic_isles_1_14_7::VersionData;

const DOCUMENT: &str = include_str!("../../../version-data/1.14.7/version_data.json");

/// The id is a constant that parses, so the one construction failure cannot happen; if it did, the panic unwinds
/// inside module-api's open guard and the core sees a failed load. A broken document falls back to the installed
/// `version_data.json`, whose error the start names.
fn module() -> rustic_isles::IslesModule {
    let data = serde_json::from_str(DOCUMENT)
        .map_err(|e| e.to_string())
        .and_then(|value| VersionData::from_json(&value));
    let spec = match data {
        Ok(data) => rustic_isles_1_14_7::spec_with(data),
        Err(_) => rustic_isles_1_14_7::spec(),
    };
    match spec.and_then(rustic_isles::IslesModule::new) {
        Ok(module) => module,
        Err(error) => panic!("isles-1-14-7-plugin: {error}"),
    }
}

dainty_module_api::export_plugin!(module());
