// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The 1.14.7 module's version data, read at start from the two files host.toml names (RD27, port plan item 9):
//! `version_facts` (MIT, rulings V-1 and V-3) and `version_data` (Mystic Paradox's design, V-2), merged and checked;
//! handed to `spec_with` for the compiled-in module, to the generators and the db tool, and installed in the backend
//! game data folder for a module loaded as a library or a child, which reads both there. Where the host's own export
//! has them, the starter weapons' default parts come from it and every id is checked against its catalog (V-4).
//! A `version_data` that still holds the whole document (a release before the split) needs no facts.

use std::path::{Path, PathBuf};

use dainty_hostkit::hostconfig::{self, HostConfig};
use dainty_hostkit::paths::Paths;
use rustic_isles_1_14_7::export::{self, CATALOG_EXPORT, WEAPON_SLOTS_EXPORT};
use rustic_isles_1_14_7::{DATA_FILE, FACTS_FILE, VersionData};
use serde_json::Value;

/// The facts as installed: the shipped file, or what the host's export made of it.
#[derive(Clone)]
pub struct Facts {
    /// The file they were read from.
    pub path: PathBuf,
    /// The text `install` writes.
    pub document: String,
}

/// The version data as read, with where it came from.
#[derive(Clone)]
pub struct Loaded {
    /// The design file it was read from.
    pub path: PathBuf,
    /// The design file's bytes, as `install` copies them.
    pub document: String,
    /// The facts, none when the design file holds the whole document.
    pub facts: Option<Facts>,
    /// The checked values.
    pub data: VersionData,
    /// What the host's export changed or found, for the generators' report.
    pub notes: Vec<String>,
}

/// The design file host.toml names, or the shipped default.
pub fn path(paths: &Paths, config: Option<&HostConfig>) -> PathBuf {
    hostconfig::version_data_path(paths, config)
}

/// The facts file host.toml names, or the shipped default.
pub fn facts_path(paths: &Paths, config: Option<&HostConfig>) -> PathBuf {
    hostconfig::version_facts_path(paths, config)
}

/// The file's text; none when it is missing and `setting` is empty (optional), else the error says how to fix it.
fn read(path: &Path, what: &str, default: &str, setting: &str) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && setting.is_empty() => Ok(None),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(format!(
            "the 1.14.7 {what} is missing: {} does not exist. The server release ships it as {default}; set \
             {setting} in host.toml to read it from somewhere else",
            path.display()
        )),
        Err(e) => Err(format!("could not read the 1.14.7 {what} {}: {e}", path.display())),
    }
}

fn json(text: &str, what: &str, path: &Path) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|e| format!("the 1.14.7 {what} {} is not JSON: {e}", path.display()))
}

/// Reads and checks the design at `design` and the facts at `facts`, applying the export in `exports` when it is
/// there. The error says what is wrong and how a host fixes it.
pub fn load_files(design: &Path, facts: &Path, exports: Option<&Path>) -> Result<Loaded, String> {
    let document = read(design, "version data", hostconfig::DEFAULT_VERSION_DATA, "version_data")?.unwrap_or_default();
    let design_value = json(&document, "version data", design)?;
    // A whole document (before the split) reads alone when there are no facts
    let whole = design_value.pointer("/starterKit/instanced").is_some();
    let optional = if whole { "" } else { "version_facts" };
    let mut facts_document = read(facts, "version facts", hostconfig::DEFAULT_VERSION_FACTS, optional)?;
    let mut facts_value = facts_document
        .as_deref()
        .map(|text| json(text, "version facts", facts))
        .transpose()?;
    let mut notes = Vec::new();
    if let (Some(value), Some(exports)) = (facts_value.as_mut(), exports)
        && let Ok(slots) = std::fs::read_to_string(exports.join(WEAPON_SLOTS_EXPORT))
    {
        let before = value.clone();
        notes.push(export::default_weapon_parts(value, &slots));
        if *value != before {
            let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
            facts_document = Some(format!("{text}\n"));
        }
    }
    let merged = match &facts_value {
        Some(value) => rustic_isles_1_14_7::merge(value, &design_value),
        None => Ok(design_value),
    };
    let (merged, data) = merged
        .and_then(|m| VersionData::from_json(&m).map(|data| (m, data)))
        .map_err(|e| {
            format!(
                "the 1.14.7 version data {} (with {}) is not valid: {e}",
                design.display(),
                facts.display()
            )
        })?;
    if let Some(exports) = exports
        && let Ok(catalog) = std::fs::read_to_string(exports.join(CATALOG_EXPORT))
    {
        let missing = export::missing_ids(&merged, &export::catalog_ids(&catalog));
        if !missing.is_empty() {
            notes.push(format!(
                "the catalog export lacks {} id(s) the version data names: {}",
                missing.len(),
                missing.join(", ")
            ));
        }
    }
    Ok(Loaded {
        path: design.to_path_buf(),
        document,
        facts: facts_document.map(|document| Facts {
            path: facts.to_path_buf(),
            document,
        }),
        data,
        notes,
    })
}

/// Reads the files `config` names (the defaults without one), with `paths`' export.
pub fn load_with(paths: &Paths, config: Option<&HostConfig>) -> Result<Loaded, String> {
    load_files(&path(paths, config), &facts_path(paths, config), Some(&paths.exports))
}

/// Reads the files `paths`' host.toml names (or the defaults when it has none or cannot be read).
pub fn load(paths: &Paths) -> Result<Loaded, String> {
    let config = hostconfig::load_host_config(paths).ok().flatten();
    load_with(paths, config.as_ref())
}

/// The legacy starter `db migrate` and `verify` repair with.
pub fn legacy_starter(paths: &Paths) -> Result<rustic_isles_db::LegacyStarter, String> {
    let loaded = load(paths)?;
    loaded
        .data
        .legacy_starter()
        .ok_or_else(|| format!("{} has no legacyStarter", loaded.path.display()))
}

fn write_if_changed(target: &Path, text: &str) -> std::io::Result<bool> {
    if std::fs::read(target).is_ok_and(|on_disk| on_disk == text.as_bytes()) {
        return Ok(false);
    }
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(target, text)?;
    Ok(true)
}

/// Writes the design and the facts into `backend_game_data` where they differ; true when it wrote.
pub fn install(loaded: &Loaded, backend_game_data: &Path) -> std::io::Result<bool> {
    let mut wrote = write_if_changed(&backend_game_data.join(DATA_FILE), &loaded.document)?;
    if let Some(facts) = &loaded.facts {
        wrote |= write_if_changed(&backend_game_data.join(FACTS_FILE), &facts.document)?;
    }
    Ok(wrote)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]
mod tests {
    use rustic_isles_1_14_7::test_support::{synthetic, synthetic_document, synthetic_split};
    use serde_json::json;

    use super::*;

    fn write(path: &Path, value: &Value) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, value.to_string()).unwrap();
    }

    fn write_split(root: &Path) -> (PathBuf, PathBuf) {
        let (facts, design) = synthetic_split();
        let (d, f) = (root.join("version_data.json"), root.join("version_facts.json"));
        write(&d, &design);
        write(&f, &facts);
        (d, f)
    }

    #[test]
    fn the_defaults_are_read_from_the_server_folder() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let (facts, design) = synthetic_split();
        write(&dir.path().join(hostconfig::DEFAULT_VERSION_DATA), &design);
        write(&dir.path().join(hostconfig::DEFAULT_VERSION_FACTS), &facts);
        let loaded = load(&paths).unwrap();
        assert_eq!(loaded.path, dir.path().join(hostconfig::DEFAULT_VERSION_DATA));
        assert_eq!(loaded.data, synthetic());
        assert!(loaded.data.legacy_starter().is_some());
    }

    #[test]
    fn a_whole_document_needs_no_facts() {
        let dir = tempfile::tempdir().unwrap();
        let design = dir.path().join("version_data.json");
        write(&design, &synthetic_document());
        let loaded = load_files(&design, &dir.path().join("none.json"), None).unwrap();
        assert!(loaded.facts.is_none());
        assert_eq!(loaded.data, synthetic());
    }

    #[test]
    fn a_missing_or_broken_file_says_where_and_what() {
        let dir = tempfile::tempdir().unwrap();
        let (design, facts) = (
            dir.path().join("version_data.json"),
            dir.path().join("version_facts.json"),
        );
        let missing = load_files(&design, &facts, None).err().unwrap();
        assert!(
            missing.contains("is missing") && missing.contains("version_data in host.toml"),
            "{missing}"
        );
        write(&design, &synthetic_split().1);
        let no_facts = load_files(&design, &facts, None).err().unwrap();
        assert!(no_facts.contains("version_facts in host.toml"), "{no_facts}");
        std::fs::write(&facts, "{").unwrap();
        assert!(load_files(&design, &facts, None).err().unwrap().contains("is not JSON"));
        std::fs::write(&facts, "{}").unwrap();
        let invalid = load_files(&design, &facts, None).err().unwrap();
        assert!(invalid.contains("is not valid: version_data.json: "), "{invalid}");
    }

    #[test]
    fn install_writes_once_and_then_leaves_the_files() {
        let dir = tempfile::tempdir().unwrap();
        let (design, facts) = write_split(&dir.path().join("in"));
        let loaded = load_files(&design, &facts, None).unwrap();
        let backend = dir.path().join("game-data");
        assert!(install(&loaded, &backend).unwrap());
        assert!(!install(&loaded, &backend).unwrap());
        std::fs::write(backend.join(FACTS_FILE), "{}").unwrap();
        assert!(install(&loaded, &backend).unwrap());
        assert_eq!(
            std::fs::read_to_string(backend.join(FACTS_FILE)).unwrap(),
            std::fs::read_to_string(&facts).unwrap()
        );
        let root = rustic_isles::GameDataRoot::new(Some(backend), None);
        assert_eq!(VersionData::load(&root).unwrap(), synthetic());
    }

    #[test]
    fn the_hosts_export_checks_every_id() {
        let dir = tempfile::tempdir().unwrap();
        let (design, facts) = write_split(&dir.path().join("in"));
        let exports = dir.path().join("exports");
        std::fs::create_dir_all(&exports).unwrap();
        // The synthetic kit has no WP_* weapon, so the slots leave the facts alone
        std::fs::write(exports.join(WEAPON_SLOTS_EXPORT), "").unwrap();
        let catalog: String = (1..=6)
            .map(|n| format!("{}\n", json!({ "itemId": format!("TEST_ITEM_{n}") })))
            .collect();
        std::fs::write(exports.join(CATALOG_EXPORT), catalog).unwrap();
        let loaded = load_files(&design, &facts, Some(&exports)).unwrap();
        assert_eq!(loaded.data, synthetic());
        assert!(
            loaded.notes[0].contains("does not cover any starter weapon"),
            "{:?}",
            loaded.notes
        );
        assert!(
            !loaded.notes[1].contains("TEST_ITEM_4") && loaded.notes[1].contains("introEmote: TEST_EMOTE"),
            "{:?}",
            loaded.notes
        );
    }

    #[test]
    fn the_hosts_weapon_slots_replace_the_shipped_parts() {
        let dir = tempfile::tempdir().unwrap();
        let (mut facts, design) = synthetic_split();
        facts["starterKit"]["instanced"] = json!([
            "WP_TEST",
            "TEST_ITEM_1",
            "TEST_ITEM_2",
            "TEST_ITEM_3",
            "TEST_ITEM_4",
            "TEST_ITEM_5",
            "TEST_ITEM_6"
        ]);
        let (d, f) = (
            dir.path().join("version_data.json"),
            dir.path().join("version_facts.json"),
        );
        write(&d, &design);
        write(&f, &facts);
        let exports = dir.path().join("exports");
        std::fs::create_dir_all(&exports).unwrap();
        let slots = json!({ "itemId": "WP_TEST", "slots": [{ "defaultWeaponPartId": "TEST_ITEM_2" }] });
        std::fs::write(exports.join(WEAPON_SLOTS_EXPORT), format!("{slots}\n")).unwrap();
        let loaded = load_files(&d, &f, Some(&exports)).unwrap();
        assert_eq!(loaded.data.starter_kit().default_weapon_parts, ["TEST_ITEM_2"]);
        let installed: Value = serde_json::from_str(&loaded.facts.unwrap().document).unwrap();
        assert_eq!(installed["starterKit"]["defaultWeaponParts"], json!(["TEST_ITEM_2"]));
    }
}
