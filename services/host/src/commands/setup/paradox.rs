//! Mystic Paradox is part of this repository (RP2-RP4, `legacy/ReforgedHost/src/paradox.ts`): since
//! DS6 in the rustic-isles submodule, its backend and Director in
//! `modules/rustic-isles/reference/1.14.7`, the game runtime and the CatalogExporter in
//! `modules/rustic-isles/native`; the runtime loader stays in `native/` (DD13). Nothing is downloaded
//! from Paradox's repository any more.
//!
//! The host's own files (the SDK from their game, game data, exports and builds) sat beside the code
//! in two older places: `.workspace/Mystic-Paradox`, a patched checkout from before RP4, and
//! `paradox/` and `native/` from RP4 to DS6. [`migrate_old_workspace`] copies those into the new
//! places once, where nothing of the host's is there yet, and never deletes the old folders.
//!
//! [`Layout`] holds the code side of the Node kit's `paths()` since RP4.

use std::path::{Path, PathBuf};

use crate::error::HostError;
use crate::paths::Paths;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// The server's folder (the checkout or the release).
    pub root: PathBuf,
    pub paradox: PathBuf,
    /// Where setup took the host's own files from before Mystic Paradox was part of the repository.
    pub old_upstream: PathBuf,
    /// Where Mystic Paradox was from RP4 to DS6: `paradox/` and `native/` in the repository.
    pub old_paradox: PathBuf,
    pub old_native: PathBuf,
    pub backend_dir: PathBuf,
    pub director_dir: PathBuf,
    pub gateway_dir: PathBuf,
    pub content_dir: PathBuf,
    pub runtime_source: PathBuf,
    pub sdk_header: PathBuf,
    pub sdk_dir: PathBuf,
    pub runtime_dll: PathBuf,
    pub loader_dir: PathBuf,
    pub loader_dll: PathBuf,
    pub exporter_dir: PathBuf,
    pub exporter_dll: PathBuf,
    pub backend_game_data: PathBuf,
    pub director_game_data: PathBuf,
    pub raw_game_data: PathBuf,
    pub exports: PathBuf,
    pub store_captures: PathBuf,
}

impl Layout {
    pub fn new(p: &Paths) -> Self {
        Self {
            root: p.repo.clone(),
            paradox: p.paradox.clone(),
            old_upstream: p.old_upstream.clone(),
            old_paradox: p.repo.join("paradox"),
            old_native: p.repo.join("native"),
            backend_dir: p.backend_dir.clone(),
            director_dir: p.director_dir.clone(),
            gateway_dir: p.gateway_dir.clone(),
            content_dir: p.content_dir.clone(),
            runtime_source: p.runtime_source.clone(),
            sdk_header: p.sdk_header.clone(),
            sdk_dir: p.sdk_dir.clone(),
            runtime_dll: p.runtime_dll.clone(),
            loader_dir: p.loader_dir.clone(),
            loader_dll: p.loader_dll.clone(),
            exporter_dir: p.exporter_dir.clone(),
            exporter_dll: p.exporter_dll.clone(),
            backend_game_data: p.backend_game_data.clone(),
            director_game_data: p.director_game_data.clone(),
            raw_game_data: p.raw_game_data.clone(),
            exports: p.exports.clone(),
            store_captures: p.store_captures.clone(),
        }
    }
}

/// The folders the host kit builds from, relative to the repository. A checkout without one of them
/// is incomplete (a sparse or damaged clone, or one made without `--recurse-submodules`).
pub fn missing_paradox(l: &Layout) -> Vec<&'static str> {
    let needed: [(&str, PathBuf); 5] = [
        (
            "modules/rustic-isles/reference/1.14.7/ParadoxBackend",
            l.backend_dir.join("package.json"),
        ),
        (
            "modules/rustic-isles/reference/1.14.7/ParadoxDirector",
            l.director_dir.join("package.json"),
        ),
        (
            "modules/rustic-isles/native/runtime",
            l.runtime_source.join("MysticParadox.vcxproj"),
        ),
        ("native/runtime-loader", l.loader_dir.join("Cargo.toml")),
        (
            "modules/rustic-isles/native/catalog-exporter",
            l.exporter_dir.join("CatalogExporter.vcxproj"),
        ),
    ];
    needed
        .into_iter()
        .filter(|(_, file)| !file.exists())
        .map(|(name, _)| name)
        .collect()
}

/// The Dumper-7 SDK files next to SDK.hpp (the SDK folder is copied as a whole).
pub const SDK_FILES: [&str; 6] = [
    "SDK.hpp",
    "Assertions.inl",
    "NameCollisions.inl",
    "PropertyFixup.hpp",
    "UnrealContainers.hpp",
    "UtfN.hpp",
];

/// What the old `.workspace/Mystic-Paradox` held of the host's own, as (old path, new path) pairs.
/// Game data is listed file by file, so a file the preset or the host already put in the new place
/// is kept.
pub fn old_workspace_items(l: &Layout) -> Vec<(PathBuf, PathBuf)> {
    let old = &l.old_upstream;
    let runtime = old.join("ParadoxRuntime");
    let mut items: Vec<(PathBuf, PathBuf)> = SDK_FILES
        .iter()
        .chain(&["SDK"])
        .map(|f| (runtime.join(f), l.runtime_source.join(f)))
        .collect();
    items.push((
        runtime.join("x64").join("Release").join("MysticParadox.dll"),
        l.runtime_dll.clone(),
    ));
    items.push((
        join(old, &["tools", "RuntimeLoader", "target", "release", "winmm.dll"]),
        l.loader_dll.clone(),
    ));
    items.push((
        join(
            old,
            &["tools", "CatalogExporter", "x64", "Release", "CatalogExporter.dll"],
        ),
        l.exporter_dll.clone(),
    ));
    items.push((old.join("Items_Analysis"), l.exports.clone()));
    items.push((old.join("DauntlessEndpointDocumentation"), l.store_captures.clone()));
    game_data_items(
        &mut items,
        [
            (join(old, &["ParadoxBackend", "game-data"]), &l.backend_game_data),
            (join(old, &["ParadoxDirector", "game-data"]), &l.director_game_data),
        ],
    );
    items
}

/// What `paradox/` and `native/` held of the host's own before DS6 moved the code to rustic-isles, as
/// (old path, new path) pairs. The runtime loader did not move (DD13).
pub fn pre_submodule_items(l: &Layout) -> Vec<(PathBuf, PathBuf)> {
    let (paradox, native) = (&l.old_paradox, &l.old_native);
    let runtime = native.join("paradox-runtime");
    let exporter = native.join("catalog-exporter");
    let mut items: Vec<(PathBuf, PathBuf)> = SDK_FILES
        .iter()
        .chain(&["SDK", "deployment_config.generated.h"])
        .map(|f| (runtime.join(f), l.runtime_source.join(f)))
        .collect();
    items.push((
        join(&runtime, &["x64", "Release", "MysticParadox.dll"]),
        l.runtime_dll.clone(),
    ));
    items.push((
        join(&exporter, &["x64", "Release", "CatalogExporter.dll"]),
        l.exporter_dll.clone(),
    ));
    for file in ["ExportPaths.local.h", "export_flags.txt"] {
        items.push((exporter.join(file), l.exporter_dir.join(file)));
    }
    items.push((paradox.join("Items_Analysis"), l.exports.clone()));
    items.push((paradox.join("DauntlessEndpointDocumentation"), l.store_captures.clone()));
    game_data_items(
        &mut items,
        [
            (join(paradox, &["backend", "game-data"]), &l.backend_game_data),
            (join(paradox, &["director", "game-data"]), &l.director_game_data),
        ],
    );
    items
}

/// The host's generated game data in each `(old, new)` folder, file by file.
fn game_data_items(items: &mut Vec<(PathBuf, PathBuf)>, dirs: [(PathBuf, &PathBuf); 2]) {
    for (from, to) in dirs {
        let Ok(entries) = std::fs::read_dir(&from) else {
            continue;
        };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else { continue };
            // The committed examples are already in place; raw/ is the host's own sources.
            if (kind.is_file() && name.ends_with(".json") && !name.ends_with(".example.json"))
                || (kind.is_dir() && name == "raw")
            {
                items.push((from.join(&name), to.join(&name)));
            }
        }
    }
}

fn join(base: &Path, parts: &[&str]) -> PathBuf {
    parts.iter().fold(base.to_path_buf(), |p, part| p.join(part))
}

fn is_empty_dir(path: &Path) -> bool {
    std::fs::read_dir(path).is_ok_and(|mut d| d.next().is_none())
}

/// `fs.cpSync(from, to, { recursive: true, force: false })`: files already at `to` are kept.
fn copy_tree(from: &Path, to: &Path) -> Result<(), HostError> {
    if from.is_dir() {
        std::fs::create_dir_all(to).map_err(|e| HostError::io(to, e))?;
        for entry in std::fs::read_dir(from).map_err(|e| HostError::io(from, e))? {
            let entry = entry.map_err(|e| HostError::io(from, e))?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else if to.exists() {
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ()).map_err(|e| HostError::io(to, e))
    }
}

/// The older folders still there that may hold the host's files: `.workspace/Mystic-Paradox`, and
/// `paradox/`, `native/paradox-runtime` and `native/catalog-exporter` from before DS6.
pub fn old_folders(l: &Layout) -> Vec<PathBuf> {
    [
        l.old_upstream.clone(),
        l.old_paradox.clone(),
        l.old_native.join("paradox-runtime"),
        l.old_native.join("catalog-exporter"),
    ]
    .into_iter()
    .filter(|d| d.exists())
    .collect()
}

/// Copies the host's own files from the older layouts. Returns what was copied, relative to the
/// repository with `/`. Anything already in the new place is left as it is.
pub fn migrate_old_workspace(l: &Layout, repo: &Path) -> Result<Vec<String>, HostError> {
    let old = old_folders(l);
    let mut items = Vec::new();
    if old.contains(&l.old_upstream) {
        items.extend(old_workspace_items(l));
    }
    if old.iter().any(|d| *d != l.old_upstream) {
        items.extend(pre_submodule_items(l));
    }
    let mut copied = Vec::new();
    for (from, to) in items {
        if !from.exists() {
            continue;
        }
        if to.exists() && !(to.is_dir() && is_empty_dir(&to)) {
            continue;
        }
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir).map_err(|e| HostError::io(dir, e))?;
        }
        copy_tree(&from, &to)?;
        let relative = to.strip_prefix(repo).unwrap_or(&to);
        copied.push(
            relative
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/"),
        );
    }
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(file: &Path, text: &str) {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("..")
    }

    #[test]
    fn this_checkout_has_everything_the_host_kit_builds_from() {
        assert_eq!(missing_paradox(&Layout::new(&Paths::new(repo()))), Vec::<&str>::new());
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            missing_paradox(&Layout::new(&Paths::new(dir.path()))),
            [
                "modules/rustic-isles/reference/1.14.7/ParadoxBackend",
                "modules/rustic-isles/reference/1.14.7/ParadoxDirector",
                "modules/rustic-isles/native/runtime",
                "native/runtime-loader",
                "modules/rustic-isles/native/catalog-exporter"
            ]
        );
    }

    #[test]
    fn the_hosts_own_files_move_over_from_the_old_workspace_once() {
        let dir = tempfile::tempdir().unwrap();
        let p = Paths::new(dir.path());
        let l = Layout::new(&p);
        let old = &l.old_upstream;
        assert!(
            migrate_old_workspace(&l, &p.repo).unwrap().is_empty(),
            "no old folder: nothing to do"
        );

        write(
            &join(old, &["ParadoxRuntime", "SDK.hpp"]),
            "sdk
",
        );
        write(
            &join(old, &["ParadoxRuntime", "SDK", "Basic_functions.cpp"]),
            "fn
",
        );
        write(
            &join(old, &["ParadoxRuntime", "x64", "Release", "MysticParadox.dll"]),
            "dll",
        );
        write(
            &join(old, &["tools", "RuntimeLoader", "target", "release", "winmm.dll"]),
            "loader",
        );
        write(
            &join(old, &["Items_Analysis", "catalog_1_14_7.jsonl"]),
            "{}
",
        );
        write(
            &join(old, &["ParadoxBackend", "game-data", "progression.json"]),
            "{\"mine\":1}
",
        );
        write(
            &join(old, &["ParadoxBackend", "game-data", "progression.example.json"]),
            "{}
",
        );
        write(
            &join(
                old,
                &["ParadoxBackend", "game-data", "raw", "ladyluck_store.source.json"],
            ),
            "[]
",
        );
        write(
            &join(old, &["ParadoxDirector", "game-data", "hunts.json"]),
            "{\"old\":1}
",
        );
        // Already there (from the preset, say): kept.
        write(
            &l.director_game_data.join("hunts.json"),
            "{\"new\":1}
",
        );
        std::fs::create_dir_all(&l.raw_game_data).unwrap();

        let mut copied = migrate_old_workspace(&l, &p.repo).unwrap();
        copied.sort();
        assert_eq!(
            copied,
            [
                "modules/rustic-isles/native/runtime/SDK",
                "modules/rustic-isles/native/runtime/SDK.hpp",
                "modules/rustic-isles/native/runtime/x64/Release/MysticParadox.dll",
                "modules/rustic-isles/reference/1.14.7/Items_Analysis",
                "modules/rustic-isles/reference/1.14.7/ParadoxBackend/game-data/progression.json",
                "modules/rustic-isles/reference/1.14.7/ParadoxBackend/game-data/raw",
                "native/runtime-loader/target/release/winmm.dll",
            ]
        );
        let read = |f: PathBuf| std::fs::read_to_string(f).unwrap();
        assert_eq!(
            read(l.sdk_dir.join("Basic_functions.cpp")),
            "fn
"
        );
        assert_eq!(
            read(l.backend_game_data.join("progression.json")),
            "{\"mine\":1}
"
        );
        assert!(
            !l.backend_game_data.join("progression.example.json").exists(),
            "the committed examples are not copied"
        );
        assert_eq!(
            read(l.director_game_data.join("hunts.json")),
            "{\"new\":1}
"
        );
        assert!(
            old.join("ParadoxRuntime").join("SDK.hpp").exists(),
            "the old folder is left alone"
        );
        assert!(
            migrate_old_workspace(&l, &p.repo).unwrap().is_empty(),
            "the second time there is nothing left to copy"
        );
    }

    #[test]
    fn the_hosts_own_files_move_over_from_paradox_and_native_once() {
        let dir = tempfile::tempdir().unwrap();
        let p = Paths::new(dir.path());
        let l = Layout::new(&p);
        assert!(old_folders(&l).is_empty());
        let runtime = l.old_native.join("paradox-runtime");
        write(&runtime.join("SDK.hpp"), "sdk");
        write(&join(&runtime, &["SDK", "Basic.hpp"]), "basic");
        write(&join(&runtime, &["x64", "Release", "MysticParadox.dll"]), "dll");
        write(
            &join(&l.old_native, &["catalog-exporter", "export_flags.txt"]),
            "EXPORT_CATALOG=1",
        );
        write(&join(&l.old_paradox, &["Items_Analysis", "catalog_1_14_7.jsonl"]), "{}");
        write(
            &join(&l.old_paradox, &["backend", "game-data", "store.json"]),
            "{\"mine\":1}",
        );
        write(
            &join(&l.old_paradox, &["backend", "game-data", "store.example.json"]),
            "{}",
        );
        write(&join(&l.old_paradox, &["director", "game-data", "hunts.json"]), "{}");
        // The loader did not move (DD13): its build is already in place
        write(&l.loader_dll, "loader");

        let mut copied = migrate_old_workspace(&l, &p.repo).unwrap();
        copied.sort();
        assert_eq!(
            copied,
            [
                "modules/rustic-isles/native/catalog-exporter/export_flags.txt",
                "modules/rustic-isles/native/runtime/SDK",
                "modules/rustic-isles/native/runtime/SDK.hpp",
                "modules/rustic-isles/native/runtime/x64/Release/MysticParadox.dll",
                "modules/rustic-isles/reference/1.14.7/Items_Analysis",
                "modules/rustic-isles/reference/1.14.7/ParadoxBackend/game-data/store.json",
                "modules/rustic-isles/reference/1.14.7/ParadoxDirector/game-data/hunts.json",
            ]
        );
        assert_eq!(std::fs::read_to_string(l.sdk_dir.join("Basic.hpp")).unwrap(), "basic");
        assert!(!l.backend_game_data.join("store.example.json").exists());
        assert!(runtime.join("SDK.hpp").exists(), "the old folders are left alone");
        assert!(migrate_old_workspace(&l, &p.repo).unwrap().is_empty());
    }
}
