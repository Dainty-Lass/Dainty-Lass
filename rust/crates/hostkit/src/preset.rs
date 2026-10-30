// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Creation, validation, and application of the `preset/` game-data directory.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{Value, json};

use crate::error::HostError;
use crate::gamepack::{is_example_path, pack_entries, sha256_hex};
use crate::paths::{GAME_CHANGELIST, Paths};

pub const PRESET_FORMAT: &str = "dauntless-reforged-revival-preset";
pub const PRESET_MANIFEST: &str = "preset.json";
pub const PRESET_EXCLUDED: [&str; 2] = [
    "backend/ladyluck_store.json",
    "backend/platinum_store.json",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetFile {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetManifest {
    pub comment: Vec<String>,
    pub name: String,
    pub game: String,
    pub executable_sha256: Option<String>,
    pub created: String,
    pub sources: Vec<String>,
    pub files: Vec<PresetFile>,
}

impl PresetManifest {
    pub fn to_json(&self) -> String {
        let files: Vec<Value> = self
            .files
            .iter()
            .map(|f| {
                json!({
                    "path": f.path,
                    "sha256": f.sha256,
                    "bytes": f.bytes,
                })
            })
            .collect();
        let value = json!({
            "_comment": self.comment,
            "format": PRESET_FORMAT,
            "version": 1,
            "name": self.name,
            "game": self.game,
            "executableSha256": self.executable_sha256,
            "created": self.created,
            "sources": self.sources,
            "files": files,
        });
        let mut text = crate::hostconfig::stringify_pretty(&value);
        text.push('\n');
        text
    }
}

#[derive(Debug, Clone, Default)]
pub struct CreateOptions {
    pub name: String,
    pub executable: Option<PathBuf>,
    pub executable_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetCreated {
    pub manifest: PresetManifest,
    pub left: Vec<String>,
    pub excluded: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetApplied {
    pub name: String,
    pub written: Vec<String>,
    pub unchanged: Vec<String>,
    pub kept: Vec<String>,
    pub backups: Vec<String>,
}

fn game_tag() -> String {
    format!("1.14.7 ({GAME_CHANGELIST})")
}

fn join_rel(base: &Path, rel: &str) -> PathBuf {
    let mut out = base.to_path_buf();
    for part in rel.split('/') {
        out.push(part);
    }
    out
}

fn normalize_lf(text: &str) -> String {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    text.replace("\r\n", "\n")
}

pub fn read_preset_manifest(p: &Paths) -> Result<Option<PresetManifest>, HostError> {
    let file = p.preset.join(PRESET_MANIFEST);
    if !file.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&file).map_err(|e| HostError::io(&file, e))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let value: Value = serde_json::from_str(text)
        .map_err(|e| HostError::message(format!("{}: {e}", file.display())))?;
    let Some(obj) = value.as_object() else {
        return Err(HostError::message(format!(
            "{}: preset manifest must be a JSON object",
            file.display()
        )));
    };

    let format = obj.get("format").and_then(Value::as_str).unwrap_or_default();
    if format != PRESET_FORMAT {
        return Err(HostError::message(format!(
            "{}: invalid preset format {format:?}",
            file.display()
        )));
    }
    let version = obj.get("version").and_then(Value::as_u64).unwrap_or(0);
    if version != 1 {
        return Err(HostError::message(format!(
            "{}: unsupported preset version {version}",
            file.display()
        )));
    }

    let comment = obj
        .get("_comment")
        .or_else(|| obj.get("comment"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let name = obj
        .get("name")
        .and_then(Value::as_str)
        .map(service_kit::js::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| HostError::message(format!("{}: preset needs a name", file.display())))?
        .to_owned();

    let game = obj
        .get("game")
        .and_then(Value::as_str)
        .ok_or_else(|| HostError::message(format!("{}: preset is missing game", file.display())))?
        .to_owned();

    let executable_sha256 = obj
        .get("executableSha256")
        .or_else(|| obj.get("executable_sha256"))
        .and_then(Value::as_str)
        .map(str::to_owned);

    let created = obj
        .get("created")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();

    let sources = obj
        .get("sources")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let Some(files_arr) = obj.get("files").and_then(Value::as_array) else {
        return Err(HostError::message(format!(
            "{}: preset has no files list",
            file.display()
        )));
    };
    if files_arr.is_empty() {
        return Err(HostError::message(format!(
            "{}: preset files list is empty",
            file.display()
        )));
    }

    let mut files = Vec::with_capacity(files_arr.len());
    for item in files_arr {
        let Some(fobj) = item.as_object() else {
            return Err(HostError::message(format!(
                "{}: invalid file entry in preset",
                file.display()
            )));
        };
        let path = fobj
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| HostError::message(format!("{}: file entry missing path", file.display())))?
            .to_owned();
        let sha256 = fobj
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                HostError::message(format!("{}: {path} missing sha256", file.display()))
            })?
            .to_owned();
        let bytes = fobj.get("bytes").and_then(Value::as_u64).unwrap_or(0);
        files.push(PresetFile {
            path,
            sha256,
            bytes,
        });
    }

    Ok(Some(PresetManifest {
        comment,
        name,
        game,
        executable_sha256,
        created,
        sources,
        files,
    }))
}

fn clean_stale_preset_files(preset_dir: &Path, dir: &Path, keep: &HashSet<String>) -> Result<(), HostError> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries {
        let entry = entry.map_err(|e| HostError::io(dir, e))?;
        let path = entry.path();
        if path.is_dir() {
            clean_stale_preset_files(preset_dir, &path, keep)?;
            let is_empty = std::fs::read_dir(&path)
                .ok()
                .is_some_and(|mut it| it.next().is_none());
            if is_empty {
                let _ = std::fs::remove_dir(&path);
            }
        } else if let Ok(rel) = path.strip_prefix(preset_dir) {
            let rel_str = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            if !keep.contains(&rel_str) {
                std::fs::remove_file(&path).map_err(|e| HostError::io(&path, e))?;
            }
        }
    }
    Ok(())
}

pub fn create_preset(
    p: &Paths,
    opts: &CreateOptions,
    now: DateTime<Utc>,
) -> Result<PresetCreated, HostError> {
    let mut staged: Vec<(String, String, PresetFile)> = Vec::new();
    let mut left = Vec::new();
    let mut excluded = Vec::new();

    for (rel, full) in pack_entries(p) {
        if PRESET_EXCLUDED.contains(&rel.as_str()) {
            if full.is_file() && !is_example_path(&full)? {
                excluded.push(rel);
            }
            continue;
        }
        if full.is_file() && !is_example_path(&full)? {
            let raw = std::fs::read_to_string(&full).map_err(|e| HostError::io(&full, e))?;
            let sha256 = sha256_hex(&raw);
            let bytes = raw.len() as u64;
            staged.push((
                rel.clone(),
                raw,
                PresetFile {
                    path: rel,
                    sha256,
                    bytes,
                },
            ));
        } else {
            left.push(rel);
        }
    }

    if staged.is_empty() {
        return Err(HostError::message(
            "no generated game data to make a preset from yet (reforged-host gamedata first, or pass --generate)",
        ));
    }

    let executable_sha256 = match &opts.executable {
        Some(exe) if exe.is_file() => Some(crate::publish::sha256_file(exe)?),
        _ => opts.executable_sha256.clone(),
    };

    let trimmed_name = service_kit::js::trim(&opts.name);
    let name = if trimmed_name.is_empty() {
        "Custom".to_owned()
    } else {
        trimmed_name.to_owned()
    };

    let mut keep: HashSet<String> = HashSet::new();
    keep.insert(PRESET_MANIFEST.to_owned());
    keep.insert("README.md".to_owned());
    for (rel, _, _) in &staged {
        keep.insert(rel.clone());
    }

    std::fs::create_dir_all(&p.preset).map_err(|e| HostError::io(&p.preset, e))?;
    clean_stale_preset_files(&p.preset, &p.preset, &keep)?;

    let mut files = Vec::with_capacity(staged.len());
    for (rel, raw, pf) in staged {
        let dest = join_rel(&p.preset, &rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| HostError::io(parent, e))?;
        }
        std::fs::write(&dest, &raw).map_err(|e| HostError::io(&dest, e))?;
        files.push(pf);
    }

    let exe_line = match &executable_sha256 {
        Some(sha) => format!("Game executable SHA-256 {sha}."),
        None => "Game executable SHA-256 not recorded (no game folder in host.json).".to_owned(),
    };

    let manifest = PresetManifest {
        comment: vec![
            format!(
                "Server preset \"{name}\": the game data every server set up from this project starts from (reforged-host preset)."
            ),
            format!(
                "Read from an installed Dauntless {} client for interoperability: identifiers and tuning values (item catalogue, store, progression and hunt tables), no game assets.",
                game_tag()
            ),
            "Sources: CatalogExporter's export from the running game, converted by Mystic Paradox's game-data generators and the host kit (npm run gamedata).".to_owned(),
            exe_line,
            "Lady Luck's Store and the platinum store are not in it: npm run gamedata builds them on each server from the live store captures (paradox/backend/store-captures.json).".to_owned(),
            "A host's own generated game data takes precedence over this preset. Regenerate it with npm run preset -- --create; do not edit the files by hand.".to_owned(),
        ],
        name,
        game: game_tag(),
        executable_sha256,
        created: now.to_rfc3339_opts(SecondsFormat::Millis, true),
        sources: vec![
            "CatalogExporter (native/catalog-exporter, from Mystic Paradox)".to_owned(),
            "Mystic Paradox game-data generators".to_owned(),
            "DaintyLass host kit".to_owned(),
        ],
        files,
    };

    let manifest_file = p.preset.join(PRESET_MANIFEST);
    std::fs::write(&manifest_file, manifest.to_json())
        .map_err(|e| HostError::io(&manifest_file, e))?;

    Ok(PresetCreated {
        manifest,
        left,
        excluded,
    })
}

pub fn apply_preset(paths: &Paths, force: bool) -> Result<Option<PresetApplied>, HostError> {
    apply_preset_at(paths, force, Utc::now())
}

pub fn apply_preset_at(
    p: &Paths,
    force: bool,
    now: DateTime<Utc>,
) -> Result<Option<PresetApplied>, HostError> {
    let Some(manifest) = read_preset_manifest(p)? else {
        return Ok(None);
    };

    let expected_game = game_tag();
    if manifest.game != expected_game {
        return Err(HostError::message(format!(
            "preset is for game {}, expected {expected_game}",
            manifest.game
        )));
    }

    let known = pack_entries(p);
    let mut seen = HashSet::with_capacity(manifest.files.len());
    let mut verified: Vec<(String, PathBuf, String)> = Vec::with_capacity(manifest.files.len());

    for entry in &manifest.files {
        if PRESET_EXCLUDED.contains(&entry.path.as_str()) {
            return Err(HostError::message(format!(
                "preset cannot include {}: npm run gamedata builds from the store captures",
                entry.path
            )));
        }
        let Some((_, dest)) = known.iter().find(|(k, _)| *k == entry.path) else {
            return Err(HostError::message(format!(
                "preset lists unknown game-data file {}",
                entry.path
            )));
        };
        if !seen.insert(entry.path.clone()) {
            return Err(HostError::message(format!(
                "preset lists duplicate file {}",
                entry.path
            )));
        }
        let src = join_rel(&p.preset, &entry.path);
        if !src.is_file() {
            return Err(HostError::message(format!(
                "preset file {} is missing ({})",
                entry.path,
                src.display()
            )));
        }
        let raw = std::fs::read_to_string(&src).map_err(|e| HostError::io(&src, e))?;
        let lf = normalize_lf(&raw);
        let raw_sha = sha256_hex(&raw);
        let lf_sha = sha256_hex(&lf);
        if raw_sha != entry.sha256 && lf_sha != entry.sha256 {
            return Err(HostError::message(format!(
                "preset file {} does not match its hash (expected {}, got {raw_sha})",
                entry.path, entry.sha256
            )));
        }
        serde_json::from_str::<Value>(&raw).map_err(|e| {
            HostError::message(format!("preset file {} is not valid JSON ({e})", entry.path))
        })?;
        verified.push((entry.path.clone(), dest.clone(), raw));
    }

    let stamp = now
        .to_rfc3339_opts(SecondsFormat::Millis, true)
        .replace([':', '.'], "-");
    let mut written = Vec::new();
    let mut unchanged = Vec::new();
    let mut kept = Vec::new();
    let mut backups = Vec::new();

    for (rel_path, dest, raw_content) in verified {
        if dest.is_file() {
            let existing = std::fs::read_to_string(&dest).map_err(|e| HostError::io(&dest, e))?;
            if existing == raw_content || normalize_lf(&existing) == normalize_lf(&raw_content) {
                unchanged.push(rel_path);
                continue;
            }
            if !is_example_path(&dest)? {
                if !force {
                    kept.push(rel_path);
                    continue;
                }
                let backup = PathBuf::from(format!("{}.{stamp}.bak", dest.display()));
                std::fs::copy(&dest, &backup).map_err(|e| HostError::io(&backup, e))?;
                backups.push(backup.to_string_lossy().into_owned());
            }
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| HostError::io(parent, e))?;
        }
        std::fs::write(&dest, &raw_content).map_err(|e| HostError::io(&dest, e))?;
        written.push(rel_path);
    }

    Ok(Some(PresetApplied {
        name: manifest.name,
        written,
        unchanged,
        kept,
        backups,
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use chrono::TimeZone;

    use super::*;

    #[test]
    fn creates_preset_excluding_stores_and_applies_respecting_force() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path());
        std::fs::create_dir_all(&paths.backend_game_data).unwrap();
        std::fs::write(
            paths.backend_game_data.join("progression_config.json"),
            "{\"preset\":1}\n",
        )
        .unwrap();
        std::fs::write(
            paths.backend_game_data.join("ladyluck_store.json"),
            "{\"store\":1}\n",
        )
        .unwrap();

        let now = Utc.with_ymd_and_hms(2026, 4, 17, 12, 0, 0).unwrap();
        let created = create_preset(
            &paths,
            &CreateOptions {
                name: "TestPreset".into(),
                executable: None,
                executable_sha256: None,
            },
            now,
        )
        .unwrap();
        assert_eq!(created.manifest.name, "TestPreset");
        assert_eq!(created.excluded, vec!["backend/ladyluck_store.json"]);
        assert_eq!(created.manifest.files.len(), 1);
    }
}
