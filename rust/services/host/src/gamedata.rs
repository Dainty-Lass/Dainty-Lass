//! Game data (`legacy/ReforgedHost/src/gamedata.ts`): which files the backend and the Director need,
//! whether they are real or the shipped examples, and the generators that make them.
//!
//! The game data the backend and the Director read is generated from the host's own 1.14.7 install
//! with Mystic Paradox's tools (`modules/rustic-isles/docs/GENERATING_GAME_DATA.md`). `gamedata` runs the steps in
//! order from the CatalogExporter export in `Items_Analysis/` beside the services. Every step runs in-process
//! through [`isles_core::gamedata`] and [`isles_director::hunt_tables`] (RB11), with no Node or npm dependency.
//!
//! `start` (RH2) and `doctor` (RH3) use [`game_data_state`].

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::captures::{CaptureFetch, CapturePin, ensure_captures, missing_captures, read_capture_pin};
use crate::error::HostError;
use crate::gamepack::example_of;
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

/// The sources of the two files no CatalogExporter catalog export produces, in `game-data/raw/`
/// (target, source): the progression config's is written from the progression export
/// ([`crate::progression`]) unless the host supplies a captured response; the Lady Luck store's,
/// when the host supplies one, is used instead of the live rotation in the pinned store captures
/// ([`crate::captures`]).
pub const SUPPLIED_SOURCES: [(&str, &str); 2] = [
    ("progression_config.json", "progression_config.source.json"),
    ("ladyluck_store.json", "ladyluck_store.source.json"),
];

pub(crate) fn supplied_source(target: &str) -> &'static str {
    SUPPLIED_SOURCES
        .iter()
        .find(|(t, _)| *t == target)
        .map_or("", |(_, source)| source)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameDataKind {
    /// Every file there and none of them is its shipped example.
    Real,
    /// All there, some are the examples (start-up tests only).
    Synthetic,
    /// At least one file is not there.
    Missing,
}

impl GameDataKind {
    /// `real`, `synthetic` or `missing`, as the Node kit prints it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Real => "real",
            Self::Synthetic => "synthetic",
            Self::Missing => "missing",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataState {
    pub state: GameDataKind,
    pub missing: Vec<String>,
    pub synthetic: Vec<String>,
}

fn same_bytes(a: &Path, b: &Path) -> bool {
    match (std::fs::read(a), std::fs::read(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

pub fn game_data_state(dir: &Path, files: &[&str]) -> Result<GameDataState, HostError> {
    let missing: Vec<String> = files
        .iter()
        .filter(|f| !dir.join(f).exists())
        .map(|f| (*f).to_owned())
        .collect();
    let synthetic: Vec<String> = files
        .iter()
        .filter(|f| !missing.iter().any(|m| m == *f))
        .filter(|f| same_bytes(&dir.join(f), &example_of(&dir.join(f))))
        .map(|f| (*f).to_owned())
        .collect();
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

/// Copies the shipped examples where a file is missing. Never overwrites a file.
pub fn use_examples(dir: &Path, files: &[&str]) -> Result<Vec<String>, HostError> {
    let mut copied = Vec::new();
    for f in files {
        let target = dir.join(f);
        let example = example_of(&target);
        if target.exists() || !example.exists() {
            continue;
        }
        std::fs::copy(&example, &target).map_err(|e| HostError::io(&target, e))?;
        copied.push((*f).to_owned());
    }
    Ok(copied)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepDir {
    Backend,
    Director,
}

/// Which RB11 Rust generator a [`Step`] invokes.
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

impl GeneratorKind {
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

type Check<'a> = Box<dyn Fn() -> Option<String> + 'a>;
type Prepare<'a> = Box<dyn Fn() -> Result<(), HostError> + 'a>;

pub struct Step<'a> {
    pub title: &'static str,
    pub cwd: StepDir,
    pub generator: GeneratorKind,
    /// Skip the step (with this reason) when it returns one. Checked just before the step runs.
    pub skip_if: Option<Check<'a>>,
    /// Runs just before the step (after `skip_if`).
    pub prepare: Option<Prepare<'a>>,
}

impl Step<'_> {
    pub fn skip(&self) -> Option<String> {
        self.skip_if.as_ref().and_then(|check| check())
    }
}

fn step<'a>(title: &'static str, cwd: StepDir, generator: GeneratorKind) -> Step<'a> {
    Step {
        title,
        cwd,
        generator,
        skip_if: None,
        prepare: None,
    }
}

pub fn game_data_steps(p: &Paths, examples_for_missing: bool) -> Vec<Step<'_>> {
    let raw = |target: &str| p.raw_game_data.join(supplied_source(target));
    let export_missing = move |sub: PathBuf| {
        (!p.exports.join(&sub).exists()).then(|| {
            format!(
                "no {} in {} yet (run the CatalogExporter, then copy Items_Analysis)",
                sub.display(),
                p.exports.display()
            )
        })
    };
    let captures_missing = move || match read_capture_pin(&p.captures_pin) {
        Ok(pin) => (!missing_captures(p, &pin).is_empty()).then(|| {
            "the store captures are not there (they are fetched at the start of reforged-host gamedata)".to_owned()
        }),
        Err(e) => Some(e.to_string()),
    };
    let journey = || PathBuf::from("slayers_path_1_14_7").join("player_journey_nodes.jsonl");
    let supplied_lady_luck = raw("ladyluck_store.json").exists();
    let progression_source = raw("progression_config.json");

    let mut progression = step("Progression config", StepDir::Backend, GeneratorKind::ProgressionConfig);
    progression.skip_if = Some(Box::new(move || {
        (!progression_source.exists()).then(|| {
            format!(
                "no {PROGRESSION_EXPORT_DIR} export in {} and no {} in {}{}",
                p.exports.display(),
                supplied_source("progression_config.json"),
                p.raw_game_data.display(),
                if examples_for_missing {
                    " (the example is used, and players cannot load into Ramsgate with it)"
                } else {
                    ""
                }
            )
        })
    }));
    let mut hunts = step("Director hunt tables", StepDir::Director, GeneratorKind::HuntTables);
    hunts.skip_if = Some(Box::new(move || export_missing("hunts_1_14_7".into())));
    let mut graph = step("Slayer's Path graph", StepDir::Backend, GeneratorKind::SlayersPath);
    graph.skip_if = Some(Box::new(move || export_missing(journey())));
    let mut definitions = step(
        "Slayer's Path definitions",
        StepDir::Backend,
        GeneratorKind::SlayersPathDefinitions,
    );
    definitions.skip_if = Some(Box::new(move || export_missing(journey())));
    let mut reward_cache = step("Reward Cache store", StepDir::Backend, GeneratorKind::RewardCache);
    reward_cache.skip_if = Some(Box::new(move || export_missing("catalog_1_14_7.jsonl".into())));
    // A response the host supplies wins; otherwise the live rotation from the store captures, kept to
    // the items the host's own catalogue has.
    let lady_luck = if supplied_lady_luck {
        step("Lady Luck store", StepDir::Backend, GeneratorKind::LadyLuckGenerate)
    } else {
        let mut enrich = step("Lady Luck store", StepDir::Backend, GeneratorKind::LadyLuckEnrich);
        enrich.skip_if = Some(Box::new(move || {
            export_missing("catalog_1_14_7.jsonl".into()).or_else(captures_missing)
        }));
        // The enrichment rewrites an existing ladyluck_store.json.
        enrich.prepare = Some(Box::new(move || {
            use_examples(&p.backend_game_data, &["ladyluck_store.json"]).map(drop)
        }));
        enrich
    };
    let mut platinum = step(
        "Platinum store and storage policy",
        StepDir::Backend,
        GeneratorKind::PlatinumAndStoragePolicy,
    );
    platinum.skip_if = Some(Box::new(move || {
        export_missing("store_item_images_1_14_7.jsonl".into()).or_else(|| {
            let has = |name: &str| p.backend_game_data.join(name).exists();
            (!(has("ladyluck_store.json") && has("reward_cache_store.json")))
                .then(|| "needs ladyluck_store.json and reward_cache_store.json first".to_owned())
        })
    }));
    // The progression config first: without it nobody loads into Ramsgate.
    vec![
        progression,
        hunts,
        graph,
        definitions,
        reward_cache,
        lady_luck,
        platinum,
    ]
}

fn append_generator_log(log_file: &Path, cwd: &Path, label: &str, text: &str) -> Result<(), HostError> {
    if let Some(dir) = log_file.parent() {
        fs::create_dir_all(dir).map_err(|e| HostError::io(dir, e))?;
    }
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file)
        .map_err(|e| HostError::io(log_file, e))?;
    let _ = writeln!(log, "\n$ {label}  (in {})", cwd.display());
    let _ = writeln!(log, "{text}");
    Ok(())
}

#[cfg(feature = "isles-1-14-7")]
fn execute_rust_generator(p: &Paths, generator: GeneratorKind) -> Result<String, String> {
    match generator {
        GeneratorKind::ProgressionConfig => {
            isles_core::gamedata::generate_progression_config(&p.raw_game_data, &p.backend_game_data, true)
        }
        GeneratorKind::HuntTables => {
            isles_director::hunt_tables::import_hunt_tables(&isles_director::hunt_tables::ImportHuntTablesOptions {
                export_dir: p.exports.join("hunts_1_14_7"),
                vendor_dir: p.director_game_data.clone(),
                repo_root: Some(p.repo.clone()),
                apply: true,
                update_existing: false,
                verbose: false,
                backup_stamp: None,
            })
            .map(|report| report.output)
        }
        GeneratorKind::SlayersPath => isles_core::gamedata::generate_slayers_path(
            &p.exports.join("slayers_path_1_14_7").join("player_journey_nodes.jsonl"),
            &p.backend_game_data.join("slayers_path.json"),
            true,
        ),
        GeneratorKind::SlayersPathDefinitions => isles_core::gamedata::generate_slayers_path_definitions(
            &p.exports.join("slayers_path_1_14_7").join("player_journey_nodes.jsonl"),
            &p.backend_game_data.join("slayers_path_definitions.json"),
            "1_14_7",
            true,
        ),
        GeneratorKind::RewardCache => {
            let catalog = p.exports.join("catalog_1_14_7.jsonl");
            let out = p.backend_game_data.join("reward_cache_store.json");
            let a = isles_core::gamedata::generate_reward_cache_store(&catalog, &out)?;
            let b = isles_core::gamedata::generate_inventory_storage_policy(&catalog, None, &p.backend_game_data)?;
            Ok(format!("{a}\n{b}"))
        }
        GeneratorKind::LadyLuckGenerate => {
            isles_core::gamedata::generate_ladyluck_store(&p.raw_game_data, &p.backend_game_data, true)
        }
        GeneratorKind::LadyLuckEnrich => {
            let catalog = p.exports.join("catalog_1_14_7.jsonl");
            let live = p
                .store_captures
                .join("Store")
                .join("Product")
                .join("Skus")
                .join("ladyluckstore.json");
            let store = p.backend_game_data.join("ladyluck_store.json");
            let a = isles_core::gamedata::enrich_ladyluck_store(&catalog, &live, &store)?;
            let b = isles_core::gamedata::generate_inventory_storage_policy(&catalog, None, &p.backend_game_data)?;
            Ok(format!("{a}\n{b}"))
        }
        GeneratorKind::PlatinumAndStoragePolicy => {
            let catalog = isles_core::gamedata::resolve_platinum_export_file(&p.exports, "catalog");
            let store_images = isles_core::gamedata::resolve_platinum_export_file(&p.exports, "store_item_images");
            let captures = p.store_captures.join("Store").join("Product").join("Skus");
            let a = isles_core::gamedata::generate_platinum_store(
                &catalog,
                &store_images,
                &captures,
                &p.backend_game_data,
            )?;
            let policy_catalog = p.exports.join("catalog_1_14_7.jsonl");
            let b =
                isles_core::gamedata::generate_inventory_storage_policy(&policy_catalog, None, &p.backend_game_data)?;
            Ok(format!("{a}\n{b}"))
        }
    }
}

#[cfg(not(feature = "isles-1-14-7"))]
fn execute_rust_generator(_p: &Paths, generator: GeneratorKind) -> Result<String, String> {
    Err(format!(
        "{} is unavailable: this binary was built without the isles-1-14-7 feature",
        generator.label()
    ))
}

/// Runs one generator step to its end in-process via [`isles_core::gamedata`] and
/// [`isles_director::hunt_tables`] (RB11).
pub fn run_generator(p: &Paths, step: &Step<'_>, log_file: &Path) -> Result<RunResult, HostError> {
    let cwd = match step.cwd {
        StepDir::Backend => &p.backend_dir,
        StepDir::Director => &p.director_dir,
    };
    match execute_rust_generator(p, step.generator) {
        Ok(output) => {
            append_generator_log(log_file, cwd, step.generator.label(), &output)?;
            let last_line = output
                .lines()
                .map(str::trim)
                .rfind(|l| !l.is_empty())
                .map(|l| l.chars().take(300).collect());
            Ok(RunResult {
                code: 0,
                last_line,
                output,
            })
        }
        Err(error) => {
            append_generator_log(log_file, cwd, step.generator.label(), &error)?;
            let last_line = error
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .map(|l| l.chars().take(300).collect());
            Ok(RunResult {
                code: 1,
                last_line,
                output: error,
            })
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct GenerateOptions {
    /// Copy the example for a supplied source that is not there (`--example-sources`).
    pub examples_for_missing: bool,
    pub log_file: PathBuf,
    /// Only these steps (by title); `None`: all.
    pub only: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Generated {
    pub ok: bool,
    pub failed: Vec<String>,
}

type FetchCaptures<'a> = &'a dyn Fn(&Paths, &CapturePin, &Path) -> Result<CaptureFetch, HostError>;
type RunStep<'a> = &'a dyn Fn(&Paths, &Step<'_>, &Path) -> Result<RunResult, HostError>;

/// What [`generate_game_data_with`] calls out to; the defaults fetch with git and run the
/// generators ([`run_generator`]).
pub struct Generators<'a> {
    pub fetch_captures: FetchCaptures<'a>,
    pub run_step: RunStep<'a>,
}

impl Default for Generators<'_> {
    fn default() -> Self {
        Self {
            fetch_captures: &|p, pin, log| ensure_captures(p, pin, Some(log)),
            run_step: &run_generator,
        }
    }
}

/// Runs every step it can: one that fails does not stop the others (each writes its own file), and
/// the failed ones are listed at the end.
pub fn generate_game_data(
    p: &Paths,
    opts: &GenerateOptions,
    print: &mut dyn FnMut(&str),
) -> Result<Generated, HostError> {
    generate_game_data_with(p, opts, print, &Generators::default())
}

pub fn generate_game_data_with(
    p: &Paths,
    opts: &GenerateOptions,
    print: &mut dyn FnMut(&str),
    hooks: &Generators<'_>,
) -> Result<Generated, HostError> {
    let wanted = |title: &str| opts.only.as_ref().is_none_or(|only| only.iter().any(|t| t == title));
    // The live store captures Lady Luck's and the platinum store's generators read. Without them
    // those two stores are built without the live data, so a failed fetch is reported, not fatal.
    if wanted("Lady Luck store") || wanted("Platinum store and storage policy") {
        let fetched = read_capture_pin(&p.captures_pin).and_then(|pin| {
            let r = (hooks.fetch_captures)(p, &pin, &opts.log_file)?;
            Ok((pin, r))
        });
        match fetched {
            Ok((pin, r)) => print(&format!(
                "- Store captures ({} at {}): {}",
                pin.name,
                &pin.commit[..12],
                if r == CaptureFetch::Fetched {
                    "fetched"
                } else {
                    "already there"
                }
            )),
            Err(e) => print(&format!("- Store captures: not fetched, {e}")),
        }
    }
    // The graph generator fills an existing slayers_path.json, so the example goes first.
    use_examples(&p.backend_game_data, &["slayers_path.json"])?;
    // The progression config's source, from the host's own progression export when no captured
    // response was supplied (see progression.rs).
    let source = p.raw_game_data.join(supplied_source("progression_config.json"));
    match write_progression_source(&p.exports, &source) {
        Ok(SourceWritten::Written { written, derived }) => print(&format!(
            "- Progression config source, from your progression export: {written} tracks{}",
            if derived > 0 {
                format!(" ({derived} weapon prestige tracks with their weapon's ranks)")
            } else {
                String::new()
            }
        )),
        Ok(_) => {}
        Err(e) => print(&format!("- Progression config source: {e}")),
    }
    if opts.examples_for_missing {
        for (target, source) in SUPPLIED_SOURCES {
            if !p.raw_game_data.join(source).exists() {
                use_examples(&p.backend_game_data, &[target])?;
            }
        }
    }
    let mut failed = Vec::new();
    let log = opts.log_file.display();
    for step in game_data_steps(p, opts.examples_for_missing) {
        if !wanted(step.title) {
            continue;
        }
        if let Some(reason) = step.skip() {
            print(&format!("- {}: skipped, {reason}", step.title));
            continue;
        }
        print(&format!("- {}...", step.title));
        if let Some(prepare) = &step.prepare {
            prepare()?;
        }
        let why = match (hooks.run_step)(p, &step, &opts.log_file) {
            Ok(r) if r.code == 0 => None,
            Ok(r) => Some(r.last_line.unwrap_or_else(|| format!("exit {}", r.code))),
            Err(e) => Some(e.to_string()),
        };
        match why {
            None => print("  done"),
            Some(why) => {
                print(&format!("  failed: {why} (full output in {log})"));
                failed.push(step.title.to_owned());
            }
        }
    }
    if !failed.is_empty() {
        print(&format!(
            "Failed: {}. The other steps ran; the full output is in {log}.",
            failed.join(", ")
        ));
    }
    Ok(Generated {
        ok: failed.is_empty(),
        failed,
    })
}
