// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! hostkit's `gamedata` steps, run in process by the 1.14.7 module's generators (spec-rd27-host §2.1).

use dainty_hostkit::gamedata::{GeneratorKind, Step};

use crate::HostError;
use crate::paths::Paths;
use crate::run::RunResult;

/// The most of a summary line hostkit is handed.
const SUMMARY_CHARS: usize = 300;

/// Runs `step`'s generator (spec-rd27-host §2.1). A failed generator is a `RunResult` with code 1, never an `Err`.
pub(crate) fn run_generator(paths: &Paths, step: &Step<'_>) -> Result<RunResult, HostError> {
    Ok(run_result(generate(paths, step.generator)))
}

// A report ends with its verdict; an error starts with its cause.
fn run_result(outcome: Result<String, String>) -> RunResult {
    match outcome {
        Ok(output) => RunResult {
            code: 0,
            last_line: summary(output.lines().rev()),
            output,
        },
        Err(output) => RunResult {
            code: 1,
            last_line: summary(output.lines()),
            output,
        },
    }
}

fn summary<'a>(lines: impl Iterator<Item = &'a str>) -> Option<String> {
    lines
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(SUMMARY_CHARS).collect())
}

#[cfg(any(test, not(feature = "isles-1-14-7")))]
fn unavailable(kind: GeneratorKind) -> String {
    format!(
        "{} is unavailable: this binary was built without the isles-1-14-7 feature",
        kind.label()
    )
}

#[cfg(not(feature = "isles-1-14-7"))]
fn generate(_paths: &Paths, kind: GeneratorKind) -> Result<String, String> {
    Err(unavailable(kind))
}

#[cfg(feature = "isles-1-14-7")]
fn generate(paths: &Paths, kind: GeneratorKind) -> Result<String, String> {
    use rustic_isles_gamedata::generators::{self, BackendStep, GeneratorPaths};

    let step = match kind {
        GeneratorKind::HuntTables => return hunt_tables(paths),
        GeneratorKind::ProgressionConfig => BackendStep::ProgressionConfig,
        GeneratorKind::SlayersPath => BackendStep::SlayersPath,
        GeneratorKind::SlayersPathDefinitions => BackendStep::SlayersPathDefinitions,
        GeneratorKind::RewardCache => BackendStep::RewardCache,
        GeneratorKind::LadyLuckGenerate => BackendStep::LadyLuckGenerate,
        GeneratorKind::LadyLuckEnrich => BackendStep::LadyLuckEnrich,
        GeneratorKind::PlatinumAndStoragePolicy => BackendStep::PlatinumAndStoragePolicy,
    };
    let loaded = crate::version_data::load(paths)?;
    crate::version_data::install(&loaded, &paths.backend_game_data).map_err(|e| {
        format!(
            "could not install {} in {}: {e}",
            rustic_isles_1_14_7::DATA_FILE,
            paths.backend_game_data.display()
        )
    })?;
    let inputs = loaded
        .data
        .generator_inputs()
        .ok_or_else(|| format!("{} has no rewardCachePlan or storageTokenSeeds", loaded.path.display()))?;
    let generator_paths = GeneratorPaths {
        raw_game_data: paths.raw_game_data.clone(),
        backend_game_data: paths.backend_game_data.clone(),
        exports: paths.exports.clone(),
        store_captures: paths.store_captures.clone(),
    };
    generators::run(step, &generator_paths, &inputs)
}

#[cfg(feature = "isles-1-14-7")]
fn hunt_tables(paths: &Paths) -> Result<String, String> {
    use rustic_isles_director::hunt_tables::{ImportHuntTablesOptions, import_hunt_tables};

    let options = ImportHuntTablesOptions {
        export_dir: paths.exports.join("hunts_1_14_7"),
        vendor_dir: paths.director_game_data.clone(),
        repo_root: Some(paths.repo.clone()),
        apply: true,
        update_existing: false,
        verbose: false,
        backup_stamp: None,
    };
    import_hunt_tables(&options).map(|report| report.output)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]
mod tests {
    use super::*;

    #[cfg(feature = "isles-1-14-7")]
    fn run_kind(repo: &std::path::Path, kind: GeneratorKind) -> (Paths, RunResult) {
        let paths = Paths::new(repo);
        let result = {
            let steps = dainty_hostkit::gamedata::game_data_steps(&paths, false);
            let step = steps.iter().find(|step| step.generator == kind).unwrap();
            run_generator(&paths, step).unwrap()
        };
        (paths, result)
    }

    #[test]
    fn a_success_is_summed_up_by_its_last_line() {
        let result = run_result(Ok("first\n  middle  \n\n  wrote 3 files  \n \n".to_owned()));
        assert_eq!(result.code, 0);
        assert_eq!(result.last_line.as_deref(), Some("wrote 3 files"));
        assert_eq!(result.output, "first\n  middle  \n\n  wrote 3 files  \n \n");
    }

    #[test]
    fn a_failure_is_summed_up_by_its_first_line() {
        let result = run_result(Err("\n \t\n  no export found  \nhow to make one\n".to_owned()));
        assert_eq!(result.code, 1);
        assert_eq!(result.last_line.as_deref(), Some("no export found"));
        assert_eq!(result.output, "\n \t\n  no export found  \nhow to make one\n");
    }

    #[test]
    fn the_summary_is_cut_at_300_characters() {
        let long = "é".repeat(SUMMARY_CHARS + 20);
        let ok = run_result(Ok(format!("  {long}  ")));
        let err = run_result(Err(format!("{long}\nnext")));
        for result in [ok, err] {
            let line = result.last_line.unwrap();
            assert_eq!(line.chars().count(), SUMMARY_CHARS);
            assert!(long.starts_with(&line));
        }
    }

    #[test]
    fn blank_text_has_no_summary() {
        assert_eq!(run_result(Ok(" \n\t\n".to_owned())).last_line, None);
        assert_eq!(run_result(Err(String::new())).last_line, None);
    }

    #[test]
    fn the_feature_off_message_names_the_generator() {
        assert_eq!(
            unavailable(GeneratorKind::HuntTables),
            "isles_director::hunt_tables::import_hunt_tables is unavailable: this binary was built without the \
             isles-1-14-7 feature"
        );
    }

    #[cfg(not(feature = "isles-1-14-7"))]
    #[test]
    fn without_the_module_every_kind_fails_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        for step in dainty_hostkit::gamedata::game_data_steps(&paths, false) {
            let result = run_generator(&paths, &step).unwrap();
            assert_eq!(result.code, 1);
            assert_eq!(result.output, unavailable(step.generator));
            assert_eq!(result.last_line.as_deref(), Some(result.output.as_str()));
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[cfg(feature = "isles-1-14-7")]
    #[test]
    fn hunt_tables_without_an_export_fail() {
        let dir = tempfile::tempdir().unwrap();
        let (paths, result) = run_kind(dir.path(), GeneratorKind::HuntTables);
        assert_eq!(result.code, 1);
        let first = result.output.lines().map(str::trim).find(|l| !l.is_empty()).unwrap();
        assert_eq!(result.last_line.as_deref(), Some(first));
        assert!(!paths.director_game_data.exists());
    }

    #[cfg(feature = "isles-1-14-7")]
    #[test]
    fn a_backend_kind_without_version_data_fails_with_the_load_error() {
        let dir = tempfile::tempdir().unwrap();
        let (paths, result) = run_kind(dir.path(), GeneratorKind::RewardCache);
        assert_eq!(result.code, 1);
        assert_eq!(result.output, crate::version_data::load(&paths).err().unwrap());
        assert!(!paths.backend_game_data.join(rustic_isles_1_14_7::DATA_FILE).exists());
    }
}
