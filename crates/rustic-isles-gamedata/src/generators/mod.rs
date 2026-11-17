// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The backend game-data generators `dainty-host gamedata` runs (spec §4, §5). Each returns a report or an
//! error text, both English host terminal text; an error writes nothing unless the spec says so.

mod io;
mod ladyluck;
mod platinum;
mod policy;
mod pretty;
mod progression;
mod reward_cache;
mod slayers;

use std::path::PathBuf;

pub use ladyluck::{enrich_ladyluck_store, generate_ladyluck_store};
pub use platinum::{generate_platinum_store, resolve_platinum_export_file};
pub use policy::{ITEM_CATALOG_FILE, TokenSource, generate_inventory_storage_policy, generate_item_catalog};
pub use pretty::stringify_pretty;
pub use progression::generate_progression_config;
pub use reward_cache::{RewardCacheCategory, RewardCachePlan, RewardCacheSelection, generate_reward_cache_store};
pub use slayers::{generate_slayers_path, generate_slayers_path_definitions};

/// Rule helpers the fuzz targets drive (not a contract).
#[doc(hidden)]
pub mod fuzzing {
    pub use super::platinum::{category, reconstructed_price};

    /// The reader of one catalog line and its item data.
    pub fn catalog_line(line: &str) -> Option<(i64, Option<String>)> {
        let row = super::io::parse(line).ok()?;
        let catalog = super::io::catalog_from_rows(vec![row]);
        let item = catalog.values().next()?;
        Some((item.rarity, item.image().map(str::to_owned)))
    }

    pub fn decode(row: &str, ids: &[&str]) -> Option<String> {
        let rows = ids.iter().map(|id| serde_json::json!({ "itemId": id })).collect();
        super::platinum::decode(row, &super::io::catalog_from_rows(rows))
    }

    pub fn checked_texture(path: &str) -> Result<String, String> {
        super::io::checked_texture(path)
    }

    pub fn slug(id: &str) -> String {
        super::reward_cache::slug(id)
    }

    /// One journey row's definition, as §5.3 reads it.
    pub fn journey_definition(row: &serde_json::Value) -> Option<serde_json::Value> {
        super::slayers::definition(row)
    }

    /// A Lady Luck source's SKUs and problems, as §5.5 shapes them.
    pub fn ladyluck_source(entries: &[serde_json::Value]) -> (Vec<serde_json::Value>, Vec<String>) {
        super::ladyluck::shape_source(entries)
    }
}

/// hostkit's `GeneratorKind` minus `HuntTables` (the Director's, which the host maps itself).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendStep {
    /// `progression_config.json` from the raw source.
    ProgressionConfig,
    /// `slayers_path.json` from the journey node export.
    SlayersPath,
    /// `slayers_path_definitions.json` from the journey node export.
    SlayersPathDefinitions,
    /// The Reward Cache store, then the storage policy.
    RewardCache,
    /// Lady Luck's store from the raw source.
    LadyLuckGenerate,
    /// Lady Luck's store from the live capture, then the storage policy.
    LadyLuckEnrich,
    /// The platinum store, then the storage policy.
    PlatinumAndStoragePolicy,
}

/// hostkit's `Paths` fields of the same names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratorPaths {
    /// The host's raw game data sources (`*.source.json`).
    pub raw_game_data: PathBuf,
    /// The backend's game data directory every step writes into.
    pub backend_game_data: PathBuf,
    /// The catalog and journey exports (`*_1_14_7.jsonl`).
    pub exports: PathBuf,
    /// The store captures, which hold `Store/Product/Skus`.
    pub store_captures: PathBuf,
}

/// What only the version knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionInputs {
    /// The version's curated Reward Cache selections.
    pub reward_cache: RewardCachePlan,
    /// Where the storage policy finds the version's token ids.
    pub storage_tokens: TokenSource,
}

/// One step, applied (spec §5.9); a step of two generators joins both reports, and the first error stops it.
pub fn run(step: BackendStep, paths: &GeneratorPaths, inputs: &VersionInputs) -> Result<String, String> {
    let backend = &paths.backend_game_data;
    let catalog = paths.exports.join("catalog_1_14_7.jsonl");
    let journey = paths
        .exports
        .join("slayers_path_1_14_7")
        .join("player_journey_nodes.jsonl");
    let skus = paths.store_captures.join("Store").join("Product").join("Skus");
    let policy = || {
        let written = generate_inventory_storage_policy(&catalog, &inputs.storage_tokens, backend)?;
        Ok::<_, String>(format!("{written}\n{}", generate_item_catalog(&catalog, backend)?))
    };
    let then_policy = |first: Result<String, String>| Ok(format!("{}\n{}", first?, policy()?));
    match step {
        BackendStep::ProgressionConfig => generate_progression_config(&paths.raw_game_data, backend, true),
        BackendStep::SlayersPath => generate_slayers_path(&journey, &backend.join("slayers_path.json"), true),
        BackendStep::SlayersPathDefinitions => {
            generate_slayers_path_definitions(&journey, &backend.join("slayers_path_definitions.json"), "1_14_7", true)
        }
        BackendStep::RewardCache => then_policy(generate_reward_cache_store(
            &catalog,
            &backend.join("reward_cache_store.json"),
            &inputs.reward_cache,
        )),
        BackendStep::LadyLuckGenerate => generate_ladyluck_store(&paths.raw_game_data, backend, true),
        BackendStep::LadyLuckEnrich => then_policy(enrich_ladyluck_store(
            &catalog,
            &skus.join("ladyluckstore.json"),
            &backend.join("ladyluck_store.json"),
        )),
        BackendStep::PlatinumAndStoragePolicy => then_policy(generate_platinum_store(
            &resolve_platinum_export_file(&paths.exports, "catalog"),
            &resolve_platinum_export_file(&paths.exports, "store_item_images"),
            &skus,
            backend,
        )),
    }
}
