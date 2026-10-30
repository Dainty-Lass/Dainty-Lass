// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Game-data state inspection, example seeding, and generator orchestration.

use std::path::{Path, PathBuf};

use crate::captures::{CaptureFetch, CapturePin, ensure_captures, read_capture_pin};
use crate::error::HostError;
use crate::paths::Paths;
use crate::progression::{SourceWritten, write_progression_source};

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
pub enum StepDir {
    Backend,
    Director,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneratorKind {
    Catalog,
    Progression,
    Quests,
    Store,
    PlatinumEnrich,
    PlatinumGenerate,
    LadyluckEnrich,
    LadyluckGenerate,
    DirectorGamedata,
    DirectorWorlddata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step<'a> {
    pub label: &'a str,
    pub script: &'a str,
    pub dir: StepDir,
    pub kind: GeneratorKind,
}

#[derive(Debug, Clone, Default)]
pub struct GenerateOptions {
    pub examples_for_missing: bool,
    pub log_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Generated {
    pub copied_sources: Vec<String>,
    pub captures: Option<CaptureFetch>,
    pub progression: Option<SourceWritten>,
    pub steps: Vec<String>,
    pub examples: Vec<String>,
}

#[allow(clippy::type_complexity)]
pub struct Generators<'a> {
    pub captures: &'a dyn Fn(&Paths, &CapturePin, Option<&Path>) -> Result<CaptureFetch, HostError>,
    pub step: &'a dyn Fn(&Paths, &Step<'_>, Option<&Path>) -> Result<(), HostError>,
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
            Some("{\n  \"code\": null,\n  \"message\": \"OK\",\n  \"payload\": {\n    \"nodes\": []\n  }\n}\n")
        }
        "slayers_path_definitions.json" => Some("{\n  \"nodes\": {}\n}\n"),
        "ladyluck_store.json" | "platinum_store.json" => {
            Some("{\n  \"code\": null,\n  \"message\": \"OK\",\n  \"payload\": {\n    \"stores\": {}\n  }\n}\n")
        }
        "reward_cache_store.json" => Some(
            "{\n  \"code\": null,\n  \"message\": \"OK\",\n  \"payload\": {\n    \"season_info\": {},\n    \"stores\": {}\n  }\n}\n",
        ),
        "inventory_storage_policy.json" => Some("{\n  \"items\": {}\n}\n"),
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
        | "arena_elite_matchmaker_hunts_new.json" => Some("{\n  \"Rows\": {}\n}\n"),
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
    let target_text = std::fs::read_to_string(&target).map_err(|e| HostError::io(&target, e))?;
    let target_norm = normalize_lf_str(&target_text);

    let example = dir.join(example_file_name(name));
    if example.is_file() {
        let example_text =
            std::fs::read_to_string(&example).map_err(|e| HostError::io(&example, e))?;
        return Ok(target_norm == normalize_lf_str(&example_text));
    }

    if let Some(seed) = embedded_example(name) {
        return Ok(target_norm == normalize_lf_str(seed));
    }

    Ok(false)
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
        } else if let Some(seed) = embedded_example(file) {
            std::fs::write(&target, seed).map_err(|e| HostError::io(&target, e))?;
            copied.push(file.to_owned());
        }
    }
    Ok(copied)
}

pub fn seed_embedded_examples(paths: &Paths) -> Result<Vec<String>, HostError> {
    let mut seeded = use_examples(&paths.backend_game_data, &BACKEND_GAME_DATA)?;
    seeded.extend(use_examples(&paths.backend_game_data, &EXTRA_BACKEND_EXAMPLES)?);
    seeded.extend(use_examples(&paths.director_game_data, &DIRECTOR_GAME_DATA)?);
    Ok(seeded)
}

pub fn copy_supplied_sources(p: &Paths) -> Result<Vec<String>, HostError> {
    let mut copied = Vec::new();
    for (export_name, source_name) in SUPPLIED_SOURCES {
        let dest = p.raw_game_data.join(source_name);
        if dest.is_file() {
            continue;
        }
        let candidate_source = p.exports.join(source_name);
        let candidate_export = p.exports.join(export_name);
        let from = if candidate_source.is_file() {
            Some(candidate_source)
        } else if candidate_export.is_file() {
            Some(candidate_export)
        } else {
            None
        };
        if let Some(src) = from {
            std::fs::create_dir_all(&p.raw_game_data)
                .map_err(|e| HostError::io(&p.raw_game_data, e))?;
            std::fs::copy(&src, &dest).map_err(|e| HostError::io(&dest, e))?;
            copied.push(source_name.to_owned());
        }
    }
    Ok(copied)
}

pub fn game_data_steps(p: &Paths) -> Vec<Step<'static>> {
    let mut steps = vec![Step {
        label: "item catalog and Slayer's Path",
        script: "gamedata:generate:catalog",
        dir: StepDir::Backend,
        kind: GeneratorKind::Catalog,
    }];

    if p.raw_game_data.join("progression_config.source.json").is_file() {
        steps.push(Step {
            label: "progression config",
            script: "gamedata:generate:progression",
            dir: StepDir::Backend,
            kind: GeneratorKind::Progression,
        });
    }

    steps.push(Step {
        label: "quests and bounties",
        script: "gamedata:generate:quests",
        dir: StepDir::Backend,
        kind: GeneratorKind::Quests,
    });
    steps.push(Step {
        label: "reward cache and storage policy",
        script: "gamedata:generate:store",
        dir: StepDir::Backend,
        kind: GeneratorKind::Store,
    });

    if p.raw_game_data.join("platinum_store.source.json").is_file() {
        steps.push(Step {
            label: "platinum store (enriched)",
            script: "platinum:enrich",
            dir: StepDir::Backend,
            kind: GeneratorKind::PlatinumEnrich,
        });
    } else {
        steps.push(Step {
            label: "platinum store",
            script: "platinum:generate",
            dir: StepDir::Backend,
            kind: GeneratorKind::PlatinumGenerate,
        });
    }

    if p.raw_game_data.join("ladyluck_store.source.json").is_file() {
        steps.push(Step {
            label: "Lady Luck store (enriched)",
            script: "ladyluck:enrich",
            dir: StepDir::Backend,
            kind: GeneratorKind::LadyluckEnrich,
        });
    } else {
        steps.push(Step {
            label: "Lady Luck store",
            script: "ladyluck:generate",
            dir: StepDir::Backend,
            kind: GeneratorKind::LadyluckGenerate,
        });
    }

    steps.push(Step {
        label: "Director hunt tables",
        script: "gamedata:generate",
        dir: StepDir::Director,
        kind: GeneratorKind::DirectorGamedata,
    });
    steps.push(Step {
        label: "Director world data",
        script: "worlddata:generate",
        dir: StepDir::Director,
        kind: GeneratorKind::DirectorWorlddata,
    });

    steps
}

pub fn run_generator(
    p: &Paths,
    step: &Step<'_>,
    log_file: Option<&Path>,
) -> Result<(), HostError> {
    #[cfg(feature = "paradox")]
    {
        let cwd = match step.dir {
            StepDir::Backend => p.backend_dir.clone(),
            StepDir::Director => p.director_dir.clone(),
        };
        let exports_str = p.exports.to_string_lossy().into_owned();
        let captures_str = p.store_captures.to_string_lossy().into_owned();
        let res = crate::run::run(
            "npm",
            &[
                "run",
                step.script,
                "--",
                "--exports",
                &exports_str,
                "--captures",
                &captures_str,
            ],
            &crate::run::RunOptions {
                cwd,
                env: None,
                log_file: log_file.map(Path::to_path_buf),
                quiet: true,
            },
        )?;
        if res.code != 0 {
            return Err(HostError::message(format!(
                "generator {} ({}) failed: {}",
                step.label,
                step.script,
                res.last_line.unwrap_or_else(|| format!("exit {}", res.code))
            )));
        }
        Ok(())
    }

    #[cfg(not(feature = "paradox"))]
    {
        let _ = log_file;
        crate::hooks::get().run_generator(p, step)
    }
}

pub fn generate_game_data(
    p: &Paths,
    opts: &GenerateOptions,
    print: impl FnMut(&str),
) -> Result<Generated, HostError> {
    let hooks = Generators {
        captures: &ensure_captures,
        step: &run_generator,
    };
    generate_game_data_with(p, opts, print, &hooks)
}

pub fn generate_game_data_with(
    p: &Paths,
    opts: &GenerateOptions,
    mut print: impl FnMut(&str),
    hooks: &Generators<'_>,
) -> Result<Generated, HostError> {
    let has_exports = std::fs::read_dir(&p.exports)
        .ok()
        .is_some_and(|mut it| it.next().is_some());
    if !has_exports {
        return Err(HostError::message(format!(
            "CatalogExporter export folder {} is missing or empty",
            p.exports.display()
        )));
    }

    std::fs::create_dir_all(&p.backend_game_data)
        .map_err(|e| HostError::io(&p.backend_game_data, e))?;
    std::fs::create_dir_all(&p.director_game_data)
        .map_err(|e| HostError::io(&p.director_game_data, e))?;
    std::fs::create_dir_all(&p.raw_game_data).map_err(|e| HostError::io(&p.raw_game_data, e))?;

    let copied_sources = copy_supplied_sources(p)?;

    let captures = if p.captures_pin.is_file() {
        let pin = read_capture_pin(&p.captures_pin)?;
        Some((hooks.captures)(p, &pin, opts.log_file.as_deref())?)
    } else {
        None
    };

    let progression_source = p.raw_game_data.join("progression_config.source.json");
    let progression = Some(write_progression_source(&p.exports, &progression_source)?);

    let planned = game_data_steps(p);
    let mut steps = Vec::with_capacity(planned.len());
    for step in &planned {
        print(&format!("  - {}...", step.label));
        (hooks.step)(p, step, opts.log_file.as_deref())?;
        steps.push(step.script.to_owned());
    }

    let mut examples = Vec::new();
    if opts.examples_for_missing {
        examples.extend(use_examples(&p.backend_game_data, &BACKEND_GAME_DATA)?);
        examples.extend(use_examples(&p.director_game_data, &DIRECTOR_GAME_DATA)?);
    }

    Ok(Generated {
        copied_sources,
        captures,
        progression,
        steps,
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

    #[test]
    fn orchestrates_generation_steps_and_copies_supplied_sources() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path());
        std::fs::create_dir_all(&paths.exports).unwrap();
        std::fs::write(
            paths.exports.join("progression_config.json"),
            r#"{"payload":{"paths":[]}}"#,
        )
        .unwrap();
        std::fs::write(
            paths.exports.join("ladyluck_store.json"),
            r#"{"payload":{"stores":{}}}"#,
        )
        .unwrap();

        let hooks = Generators {
            captures: &|_, _, _| Ok(CaptureFetch::AlreadyReady),
            step: &|_, _, _| Ok(()),
        };
        let generated = generate_game_data_with(
            &paths,
            &GenerateOptions {
                examples_for_missing: true,
                log_file: None,
            },
            |_| {},
            &hooks,
        )
        .unwrap();

        assert_eq!(
            generated.copied_sources,
            vec![
                "progression_config.source.json".to_owned(),
                "ladyluck_store.source.json".to_owned(),
            ]
        );
        assert_eq!(generated.progression, Some(SourceWritten::Supplied));
        assert!(generated.steps.contains(&"gamedata:generate:progression".to_owned()));
        assert!(generated.steps.contains(&"ladyluck:enrich".to_owned()));
        assert!(generated.steps.contains(&"platinum:generate".to_owned()));
        assert_eq!(
            generated.examples.len(),
            BACKEND_GAME_DATA.len() + DIRECTOR_GAME_DATA.len()
        );
    }
}
