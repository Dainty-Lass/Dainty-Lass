// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Game-data state inspection, example seeding, and generator orchestration.

use std::path::{Path, PathBuf};

use crate::captures::{CaptureFetch, CapturePin, ensure_captures, missing_captures, read_capture_pin};
use crate::error::HostError;
use crate::paths::Paths;
use crate::progression::{PROGRESSION_EXPORT_DIR, SourceWritten, write_progression_source};
use crate::run::RunResult;

pub const BACKEND_GAME_DATA: [&str; 7] = [
    "progression_config.json",
    "slayers_path.json",
    "slayers_path_definitions.json",
    "ladyluck_store.json",
    "platinum_store.json",
    "reward_cache_store.json",
    "inventory_storage_policy.json",
];

pub const DIRECTOR_GAME_DATA: [&str; 7] = [
    "player_hunts_table.json",
    "matchmaker_hunts_table.json",
    "arena_easy_matchmaker_hunts.json",
    "arena_hard_matchmaker_hunts.json",
    "arena_elite_matchmaker_hunts.json",
    "arena_hard_matchmaker_hunts_new.json",
    "arena_elite_matchmaker_hunts_new.json",
];

pub const SUPPLIED_SOURCES: [(&str, &str); 2] = [
    ("progression_config.json", "progression_config.source.json"),
    ("ladyluck_store.json", "ladyluck_store.source.json"),
];

const EXTRA_BACKEND_EXAMPLES: [&str; 7] = [
    "quests_definitions.json",
    "bounty_templates.json",
    "EscalationData.json",
    "TrialsData.json",
    "sanctum_lore_nodes.json",
    "ramsgate_gatherables.json",
    "island_fauna.json",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameDataKind {
    Missing,
    Synthetic,
    Real,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataState {
    pub state: GameDataKind,
    pub missing: Vec<String>,
    pub synthetic: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneratorKind {
    ProgressionConfig,
    HuntTables,
    SlayersPath,
    SlayersPathDefinitions,
    RewardCache,
    LadyLuckGenerate,
    LadyLuckEnrich,
    PlatinumAndStoragePolicy,
}

#[allow(non_upper_case_globals)]
impl GeneratorKind {
    pub const DirectorHunts: Self = Self::HuntTables;
    pub const PlatinumStore: Self = Self::PlatinumAndStoragePolicy;

    pub const fn label(self) -> &'static str {
        match self {
            Self::ProgressionConfig => "isles_core::gamedata::generate_progression_config",
            Self::HuntTables => "isles_director::hunt_tables::import_hunt_tables",
            Self::SlayersPath => "isles_core::gamedata::generate_slayers_path",
            Self::SlayersPathDefinitions => "isles_core::gamedata::generate_slayers_path_definitions",
            Self::RewardCache => "isles_core::gamedata::generate_reward_cache_store",
            Self::LadyLuckGenerate => "isles_core::gamedata::generate_ladyluck_store",
            Self::LadyLuckEnrich => "isles_core::gamedata::enrich_ladyluck_store",
            Self::PlatinumAndStoragePolicy => "isles_core::gamedata::generate_platinum_store",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step<'a> {
    pub title: &'static str,
    pub cwd: PathBuf,
    pub script: &'static str,
    pub generator: GeneratorKind,
    paths: &'a Paths,
    examples_for_missing: bool,
}

impl Step<'_> {
    pub fn skip(&self) -> Option<String> {
        let p = self.paths;
        let has_catalog =
            p.exports.join("catalog_1_14_7.jsonl").is_file() || p.exports.join("catalog_1_12.jsonl").is_file();
        match self.generator {
            GeneratorKind::ProgressionConfig => {
                let has_source = p.raw_game_data.join(SUPPLIED_SOURCES[0].1).is_file();
                let has_export = p
                    .exports
                    .join(PROGRESSION_EXPORT_DIR)
                    .join("player-experience-tracks.jsonl")
                    .is_file();
                if has_source || has_export {
                    None
                } else {
                    Some("progression_config.source.json or progression_1_14_7 export is not there".into())
                }
            }
            GeneratorKind::SlayersPath | GeneratorKind::SlayersPathDefinitions => {
                if p.exports
                    .join("slayers_path_1_14_7")
                    .join("player_journey_nodes.jsonl")
                    .is_file()
                {
                    None
                } else {
                    Some("slayers_path_1_14_7/player_journey_nodes.jsonl is not there".into())
                }
            }
            GeneratorKind::HuntTables => {
                if p.exports.join("hunts_1_14_7").exists() {
                    None
                } else {
                    Some("hunts_1_14_7 is not there".into())
                }
            }
            GeneratorKind::RewardCache => {
                if has_catalog {
                    None
                } else {
                    Some("catalog_1_14_7.jsonl is not there".into())
                }
            }
            GeneratorKind::LadyLuckGenerate => {
                if p.raw_game_data.join(SUPPLIED_SOURCES[1].1).is_file() {
                    None
                } else {
                    Some("ladyluck_store.source.json is not there".into())
                }
            }
            GeneratorKind::LadyLuckEnrich => {
                if !has_catalog {
                    return Some("catalog_1_14_7.jsonl is not there".into());
                }
                match read_capture_pin(&p.captures_pin) {
                    Ok(pin) => {
                        if !missing_captures(p, &pin).is_empty() {
                            return Some("store captures are not there".into());
                        }
                    }
                    Err(e) => {
                        let live = p
                            .store_captures
                            .join("Store")
                            .join("Product")
                            .join("Skus")
                            .join("ladyluckstore.json")
                            .is_file();
                        if !live {
                            return Some(format!("store captures are not there ({e})"));
                        }
                    }
                }
                None
            }
            GeneratorKind::PlatinumAndStoragePolicy => {
                let has_images = p.exports.join("store_item_images_1_14_7.jsonl").is_file()
                    || p.exports.join("store_item_images_1_12.jsonl").is_file();
                if !has_images {
                    return Some("store_item_images_1_14_7.jsonl is not there".into());
                }
                let has = |name: &str| p.backend_game_data.join(name).exists();
                if !(has("ladyluck_store.json") && has("reward_cache_store.json")) {
                    return Some("needs ladyluck_store.json and reward_cache_store.json first".into());
                }
                None
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct GenerateOptions {
    pub examples_for_missing: bool,
    pub log_file: PathBuf,
    pub only: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Generated {
    pub ok: bool,
    pub ran: Vec<String>,
    pub skipped: Vec<String>,
    pub failed: Vec<String>,
    pub examples: Vec<String>,
}

#[allow(clippy::type_complexity)]
pub struct Generators<'a> {
    pub fetch_captures: &'a dyn Fn(&Paths, &CapturePin, &Path) -> Result<CaptureFetch, HostError>,
    pub run_step: &'a dyn Fn(&Paths, &Step<'_>, &Path) -> Result<RunResult, HostError>,
}

pub fn example_file_name(name: &str) -> String {
    match name.strip_suffix(".json") {
        Some(stem) => format!("{stem}.example.json"),
        None => format!("{name}.example.json"),
    }
}

pub fn embedded_example(name: &str) -> Option<&'static str> {
    match name {
        "progression_config.json" => {
            Some("{\n  \"code\": null,\n  \"message\": \"OK\",\n  \"payload\": {\n    \"paths\": []\n  }\n}\n")
        }
        "slayers_path.json" => {
            Some("{\n  \"code\": null,\n  \"message\": \"OK\",\n  \"payload\": {\n    \"nodes\": {}\n  }\n}\n")
        }
        "slayers_path_definitions.json" => Some("{\n  \"nodes\": {}\n}\n"),
        "ladyluck_store.json" | "platinum_store.json" | "reward_cache_store.json" => Some("[]\n"),
        "inventory_storage_policy.json" => Some("{\n  \"entries\": {}\n}\n"),
        "quests_definitions.json" => Some("{\n  \"quests\": {}\n}\n"),
        "bounty_templates.json" => Some("{\n  \"templates\": []\n}\n"),
        "EscalationData.json" => Some("{\n  \"escalations\": {}\n}\n"),
        "TrialsData.json" => Some("{\n  \"trials\": {}\n}\n"),
        "sanctum_lore_nodes.json" => Some("{\n  \"nodes\": []\n}\n"),
        "ramsgate_gatherables.json" => Some("{\n  \"gatherables\": []\n}\n"),
        "island_fauna.json" => Some("{\n  \"fauna\": {}\n}\n"),
        "player_hunts_table.json"
        | "matchmaker_hunts_table.json"
        | "arena_easy_matchmaker_hunts.json"
        | "arena_hard_matchmaker_hunts.json"
        | "arena_elite_matchmaker_hunts.json"
        | "arena_hard_matchmaker_hunts_new.json"
        | "arena_elite_matchmaker_hunts_new.json" => Some("[{\n  \"Rows\": {}\n}]\n"),
        _ => None,
    }
}

fn normalize_lf_str(text: &str) -> String {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    text.replace("\r\n", "\n")
}

pub fn is_example_file(dir: &Path, name: &str) -> Result<bool, HostError> {
    let target = dir.join(name);
    if !target.is_file() {
        return Ok(false);
    }
    let example = dir.join(example_file_name(name));
    if !example.is_file() {
        return Ok(false);
    }
    let target_text = std::fs::read_to_string(&target).map_err(|e| HostError::io(&target, e))?;
    let example_text = std::fs::read_to_string(&example).map_err(|e| HostError::io(&example, e))?;
    Ok(normalize_lf_str(&target_text) == normalize_lf_str(&example_text))
}

pub fn game_data_state(dir: &Path, files: &[&str]) -> Result<GameDataState, HostError> {
    let mut missing = Vec::new();
    let mut synthetic = Vec::new();
    for &file in files {
        let path = dir.join(file);
        if !path.is_file() {
            missing.push(file.to_owned());
        } else if is_example_file(dir, file)? {
            synthetic.push(file.to_owned());
        }
    }
    let state = if !missing.is_empty() {
        GameDataKind::Missing
    } else if !synthetic.is_empty() {
        GameDataKind::Synthetic
    } else {
        GameDataKind::Real
    };
    Ok(GameDataState {
        state,
        missing,
        synthetic,
    })
}

pub fn use_examples(dir: &Path, files: &[&str]) -> Result<Vec<String>, HostError> {
    std::fs::create_dir_all(dir).map_err(|e| HostError::io(dir, e))?;
    let mut copied = Vec::new();
    for &file in files {
        let target = dir.join(file);
        if target.is_file() {
            continue;
        }
        let example = dir.join(example_file_name(file));
        if example.is_file() {
            std::fs::copy(&example, &target).map_err(|e| HostError::io(&target, e))?;
            copied.push(file.to_owned());
        }
    }
    Ok(copied)
}

fn seed_dir_examples(dir: &Path, files: &[&str]) -> Result<Vec<String>, HostError> {
    std::fs::create_dir_all(dir).map_err(|e| HostError::io(dir, e))?;
    let mut seeded = use_examples(dir, files)?;
    for &file in files {
        let target = dir.join(file);
        if target.is_file() {
            continue;
        }
        if let Some(seed) = embedded_example(file) {
            let example = dir.join(example_file_name(file));
            if !example.is_file() {
                std::fs::write(&example, seed).map_err(|e| HostError::io(&example, e))?;
            }
            std::fs::write(&target, seed).map_err(|e| HostError::io(&target, e))?;
            seeded.push(file.to_owned());
        }
    }
    Ok(seeded)
}

pub fn seed_embedded_examples(paths: &Paths) -> Result<Vec<String>, HostError> {
    let mut seeded = seed_dir_examples(&paths.backend_game_data, &BACKEND_GAME_DATA)?;
    seeded.extend(seed_dir_examples(&paths.backend_game_data, &EXTRA_BACKEND_EXAMPLES)?);
    seeded.extend(seed_dir_examples(&paths.director_game_data, &DIRECTOR_GAME_DATA)?);
    Ok(seeded)
}

pub fn copy_supplied_sources(p: &Paths, from_dir: &Path) -> Result<Vec<String>, HostError> {
    let mut copied = Vec::new();
    for (export_name, source_name) in SUPPLIED_SOURCES {
        let dest = p.raw_game_data.join(source_name);
        if dest.is_file() {
            continue;
        }
        let candidate_source = from_dir.join(source_name);
        let candidate_export = from_dir.join(export_name);
        let from = if candidate_source.is_file() {
            Some(candidate_source)
        } else if candidate_export.is_file() {
            Some(candidate_export)
        } else {
            None
        };
        if let Some(src) = from {
            std::fs::create_dir_all(&p.raw_game_data).map_err(|e| HostError::io(&p.raw_game_data, e))?;
            std::fs::copy(&src, &dest).map_err(|e| HostError::io(&dest, e))?;
            copied.push(source_name.to_owned());
        }
    }
    Ok(copied)
}

pub fn game_data_steps(p: &Paths, examples_for_missing: bool) -> Vec<Step<'_>> {
    let has_ladyluck_source = p.raw_game_data.join(SUPPLIED_SOURCES[1].1).is_file();
    let (ll_script, ll_gen) = if has_ladyluck_source {
        ("generate:ladyluck-store:apply", GeneratorKind::LadyLuckGenerate)
    } else {
        ("ladyluck:enrich", GeneratorKind::LadyLuckEnrich)
    };
    vec![
        Step {
            title: "Progression config",
            cwd: p.backend_dir.clone(),
            script: "generate:progression-config:apply",
            generator: GeneratorKind::ProgressionConfig,
            paths: p,
            examples_for_missing,
        },
        Step {
            title: "Director hunt tables",
            cwd: p.director_dir.clone(),
            script: "hunts:import:apply",
            generator: GeneratorKind::HuntTables,
            paths: p,
            examples_for_missing,
        },
        Step {
            title: "Slayer's Path graph",
            cwd: p.backend_dir.clone(),
            script: "generate:slayers-path:apply",
            generator: GeneratorKind::SlayersPath,
            paths: p,
            examples_for_missing,
        },
        Step {
            title: "Slayer's Path definitions",
            cwd: p.backend_dir.clone(),
            script: "generate:slayers-path-definitions:apply",
            generator: GeneratorKind::SlayersPathDefinitions,
            paths: p,
            examples_for_missing,
        },
        Step {
            title: "Reward Cache store",
            cwd: p.backend_dir.clone(),
            script: "reward-cache:generate",
            generator: GeneratorKind::RewardCache,
            paths: p,
            examples_for_missing,
        },
        Step {
            title: "Lady Luck store",
            cwd: p.backend_dir.clone(),
            script: ll_script,
            generator: ll_gen,
            paths: p,
            examples_for_missing,
        },
        Step {
            title: "Platinum store and storage policy",
            cwd: p.backend_dir.clone(),
            script: "platinum-store:generate",
            generator: GeneratorKind::PlatinumAndStoragePolicy,
            paths: p,
            examples_for_missing,
        },
    ]
}

pub fn run_generator(p: &Paths, step: &Step<'_>, _log_file: &Path) -> Result<RunResult, HostError> {
    crate::hooks::get().run_generator(p, step)
}

pub fn generate_game_data(
    p: &Paths,
    opts: &GenerateOptions,
    out: &mut dyn FnMut(&str),
) -> Result<Generated, HostError> {
    let fetch_captures = |paths: &Paths, pin: &CapturePin, log: &Path| ensure_captures(paths, pin, Some(log));
    let hooks = Generators {
        fetch_captures: &fetch_captures,
        run_step: &run_generator,
    };
    generate_game_data_with(p, opts, out, &hooks)
}

pub fn generate_game_data_with(
    p: &Paths,
    opts: &GenerateOptions,
    out: &mut dyn FnMut(&str),
    hooks: &Generators<'_>,
) -> Result<Generated, HostError> {
    std::fs::create_dir_all(&p.backend_game_data).map_err(|e| HostError::io(&p.backend_game_data, e))?;
    std::fs::create_dir_all(&p.director_game_data).map_err(|e| HostError::io(&p.director_game_data, e))?;
    std::fs::create_dir_all(&p.raw_game_data).map_err(|e| HostError::io(&p.raw_game_data, e))?;

    let wants = |title: &str| -> bool {
        opts.only
            .as_ref()
            .is_none_or(|list| list.iter().any(|item| item == title))
    };

    if wants("Progression config") {
        let source_file = p.raw_game_data.join(SUPPLIED_SOURCES[0].1);
        match write_progression_source(&p.exports, &source_file) {
            Ok(SourceWritten::Written { written, derived }) => {
                out(&format!(
                    "- Progression source: built from export ({written} tracks, {derived} derived)"
                ));
            }
            Ok(SourceWritten::Supplied) => {
                out("- Progression source: using supplied progression_config.source.json");
            }
            Ok(SourceWritten::NoExport) => {}
            Err(e) => {
                out(&format!("- Progression source: {e}"));
            }
        }
    }

    let needs_captures = !p.raw_game_data.join(SUPPLIED_SOURCES[1].1).is_file() && wants("Lady Luck store")
        || wants("Platinum store")
        || wants("Platinum store and storage policy");
    if needs_captures {
        if p.captures_pin.is_file() {
            match read_capture_pin(&p.captures_pin) {
                Ok(pin) => {
                    let short = &pin.commit[..pin.commit.len().min(12)];
                    match (hooks.fetch_captures)(p, &pin, &opts.log_file) {
                        Ok(CaptureFetch::Fetched) => {
                            out(&format!("- Store captures ({} at {short}): fetched", pin.name));
                        }
                        Ok(CaptureFetch::AlreadyReady) => {
                            out(&format!("- Store captures ({} at {short}): ready", pin.name));
                        }
                        Err(e) => {
                            out(&format!("- Store captures: not fetched, {e}"));
                        }
                    }
                }
                Err(e) => {
                    out(&format!("- Store captures: not fetched, {e}"));
                }
            }
        } else {
            let missing = p
                .store_captures
                .join("Store")
                .join("Product")
                .join("Skus")
                .join("ladyluckstore.json")
                .is_file();
            if !missing {
                out(&format!(
                    "- Store captures: not fetched, no {} in this checkout",
                    p.captures_pin.display()
                ));
            }
        }
    }

    let steps = game_data_steps(p, opts.examples_for_missing);
    let mut ran = Vec::new();
    let mut skipped = Vec::new();
    let mut failed = Vec::new();

    for step in &steps {
        if !wants(step.title) {
            continue;
        }
        if let Some(reason) = step.skip() {
            out(&format!("- {}: skipped ({reason})", step.title));
            skipped.push(step.title.to_owned());
            continue;
        }
        if step.generator == GeneratorKind::LadyLuckEnrich {
            let _ = use_examples(&p.backend_game_data, &["ladyluck_store.json"]);
            if !p.backend_game_data.join("ladyluck_store.json").is_file() && opts.examples_for_missing {
                let _ = seed_dir_examples(&p.backend_game_data, &["ladyluck_store.json"]);
            }
        }
        if step.generator == GeneratorKind::SlayersPath {
            let _ = use_examples(&p.backend_game_data, &["slayers_path.json"]);
            if !p.backend_game_data.join("slayers_path.json").is_file() {
                let _ = seed_dir_examples(&p.backend_game_data, &["slayers_path.json"]);
            }
        }
        if step.generator == GeneratorKind::PlatinumStore {
            for req in ["reward_cache_store.json", "ladyluck_store.json"] {
                let _ = use_examples(&p.backend_game_data, &[req]);
                if !p.backend_game_data.join(req).is_file() {
                    let _ = seed_dir_examples(&p.backend_game_data, &[req]);
                }
            }
        }
        match (hooks.run_step)(p, step, &opts.log_file) {
            Ok(res) if res.code == 0 => {
                out(&format!("- {}: ok", step.title));
                ran.push(step.title.to_owned());
            }
            Ok(res) => {
                let detail = res.last_line.unwrap_or_else(|| format!("exit {}", res.code));
                out(&format!("- {}: failed: {detail}", step.title));
                failed.push(step.title.to_owned());
            }
            Err(e) => {
                out(&format!("- {}: failed: {e}", step.title));
                failed.push(step.title.to_owned());
            }
        }
    }

    let mut examples = Vec::new();
    if opts.examples_for_missing {
        examples.extend(use_examples(&p.backend_game_data, &BACKEND_GAME_DATA)?);
        examples.extend(use_examples(&p.director_game_data, &DIRECTOR_GAME_DATA)?);
    }

    if !failed.is_empty() {
        out(&format!(
            "Failed: {}. The other steps ran; check {} for details.",
            failed.join(", "),
            opts.log_file.display()
        ));
    }

    let _ = missing_captures;
    Ok(Generated {
        ok: failed.is_empty(),
        ran,
        skipped,
        failed,
        examples,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn tracks_missing_synthetic_and_real_game_data_state() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        let state = game_data_state(dir, &BACKEND_GAME_DATA).unwrap();
        assert_eq!(state.state, GameDataKind::Missing);
        assert_eq!(state.missing.len(), BACKEND_GAME_DATA.len());

        for name in BACKEND_GAME_DATA {
            std::fs::write(dir.join(example_file_name(name)), format!("{{\"example\":\"{name}\"}}")).unwrap();
        }
        let copied = use_examples(dir, &BACKEND_GAME_DATA).unwrap();
        assert_eq!(copied.len(), BACKEND_GAME_DATA.len());

        let state = game_data_state(dir, &BACKEND_GAME_DATA).unwrap();
        assert_eq!(state.state, GameDataKind::Synthetic);
        assert!(state.missing.is_empty());
        assert_eq!(state.synthetic.len(), BACKEND_GAME_DATA.len());

        for name in BACKEND_GAME_DATA {
            std::fs::write(dir.join(name), r#"{"real":true}"#).unwrap();
        }
        let state = game_data_state(dir, &BACKEND_GAME_DATA).unwrap();
        assert_eq!(state.state, GameDataKind::Real);
        assert!(state.synthetic.is_empty());
    }
}
