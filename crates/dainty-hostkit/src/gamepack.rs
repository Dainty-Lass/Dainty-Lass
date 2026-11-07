// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Export and import of portable gzip-compressed game-data packs.

use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, SecondsFormat, Utc};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::error::HostError;
use crate::gamedata::{BACKEND_GAME_DATA, DIRECTOR_GAME_DATA, SUPPLIED_SOURCES, is_example_file};
use crate::paths::{GAME_CHANGELIST, Paths};

/// The `format` a pack names.
pub const PACK_FORMAT: &str = "dauntless-reforged-revival-gamedata";
/// The pack version this kit reads and writes.
pub const PACK_VERSION: u64 = 1;

/// One file a pack carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackFile {
    /// Its path in the game data, like `backend/slayers_path.json`.
    pub path: String,
    /// Its SHA-256, lowercase hex.
    pub sha256: String,
    /// Its content.
    pub data: String,
}

/// A game-data pack, as its gzipped JSON holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pack {
    /// Always [`PACK_FORMAT`].
    pub format: String,
    /// Always [`PACK_VERSION`].
    pub version: u64,
    /// The game build it is for.
    pub game: String,
    /// When it was made, RFC 3339 in UTC.
    pub created: String,
    /// The files it carries.
    pub files: Vec<PackFile>,
}

/// What [`export_game_data_pack`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exported {
    /// The pack written.
    pub file: PathBuf,
    /// The files it carries.
    pub included: Vec<String>,
    /// Files left out because they are missing or still the shipped examples.
    pub left: Vec<String>,
}

/// What [`import_game_data_pack`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    /// Files written.
    pub written: Vec<String>,
    /// Files that already held the pack's content.
    pub unchanged: Vec<String>,
    /// Backups made of the generated files the pack replaced.
    pub backups: Vec<PathBuf>,
}

/// The example file beside `file`: `x.json` gives `x.example.json`.
pub fn example_of(file: &Path) -> PathBuf {
    let Some(name) = file.file_name().and_then(|s| s.to_str()) else {
        return file.to_path_buf();
    };
    let ex = crate::gamedata::example_file_name(name);
    file.with_file_name(ex)
}

/// The SHA-256 of `text`'s UTF-8 bytes, lowercase hex.
pub fn sha256_hex(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Every file a pack may carry: its path in the pack and where it is on this server.
pub fn pack_entries(p: &Paths) -> Vec<(String, PathBuf)> {
    let mut entries = Vec::with_capacity(BACKEND_GAME_DATA.len() + DIRECTOR_GAME_DATA.len() + SUPPLIED_SOURCES.len());
    for name in BACKEND_GAME_DATA {
        entries.push((format!("backend/{name}"), p.backend_game_data.join(name)));
    }
    for name in DIRECTOR_GAME_DATA {
        entries.push((format!("director/{name}"), p.director_game_data.join(name)));
    }
    for (_, source_name) in SUPPLIED_SOURCES {
        entries.push((format!("backend/raw/{source_name}"), p.raw_game_data.join(source_name)));
    }
    entries
}

/// Where a pack made on `now`'s date goes by default, in `.workspace`.
pub fn default_pack_file(p: &Paths, now: DateTime<Utc>) -> PathBuf {
    p.workspace.join(format!(
        "gamedata-1.14.7-{GAME_CHANGELIST}-{}.json.gz",
        now.format("%Y%m%d")
    ))
}

fn game_tag() -> String {
    format!("1.14.7 ({GAME_CHANGELIST})")
}

fn iso_millis(now: DateTime<Utc>) -> String {
    now.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// The pack's JSON text.
pub fn pack_json(pack: &Pack) -> String {
    let files: Vec<Value> = pack
        .files
        .iter()
        .map(|f| {
            json!({
                "path": f.path,
                "sha256": f.sha256,
                "data": f.data,
            })
        })
        .collect();
    let value = json!({
        "format": pack.format,
        "version": pack.version,
        "game": pack.game,
        "created": pack.created,
        "files": files,
    });
    serde_json::to_string(&value).unwrap_or_default()
}

pub(crate) fn is_example_path(full_path: &Path) -> Result<bool, HostError> {
    let (Some(dir), Some(name)) = (full_path.parent(), full_path.file_name().and_then(|n| n.to_str())) else {
        return Ok(false);
    };
    is_example_file(dir, name)
}

/// Writes the server's generated game data to `out_file` as a pack; fails when there is nothing generated.
pub fn export_game_data_pack(p: &Paths, out_file: &Path, now: DateTime<Utc>) -> Result<Exported, HostError> {
    let mut pack_files = Vec::new();
    let mut included = Vec::new();
    let mut left = Vec::new();

    for (rel, full) in pack_entries(p) {
        if full.is_file() && !is_example_path(&full)? {
            let data = std::fs::read_to_string(&full).map_err(|e| HostError::io(&full, e))?;
            let sha256 = sha256_hex(&data);
            included.push(rel.clone());
            pack_files.push(PackFile {
                path: rel,
                sha256,
                data,
            });
        } else {
            left.push(rel);
        }
    }

    if pack_files.is_empty() {
        return Err(HostError::message(
            "no generated game data to export (only the example files, or nothing at all): run dainty-host gamedata first",
        ));
    }

    let pack = Pack {
        format: PACK_FORMAT.to_owned(),
        version: PACK_VERSION,
        game: game_tag(),
        created: iso_millis(now),
        files: pack_files,
    };

    let json_text = pack_json(&pack);
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(json_text.as_bytes())
        .map_err(|e| HostError::io(out_file, e))?;
    let compressed = encoder.finish().map_err(|e| HostError::io(out_file, e))?;

    if let Some(parent) = out_file.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| HostError::io(parent, e))?;
    }
    std::fs::write(out_file, compressed).map_err(|e| HostError::io(out_file, e))?;

    Ok(Exported {
        file: out_file.to_path_buf(),
        included,
        left,
    })
}

/// Reads and checks the pack at `file`.
pub fn read_game_data_pack(p: &Paths, file: &Path) -> Result<Pack, HostError> {
    let compressed = std::fs::read(file).map_err(|e| HostError::io(file, e))?;
    parse_game_data_pack(p, file, &compressed)
}

/// Unzips and checks a pack: its format, version, game build and every file's path and SHA-256.
pub fn parse_game_data_pack(p: &Paths, file: &Path, compressed: &[u8]) -> Result<Pack, HostError> {
    let mut decoder = GzDecoder::new(compressed);
    let mut text = String::new();
    decoder
        .read_to_string(&mut text)
        .map_err(|e| HostError::message(format!("{}: not a game data pack ({e})", file.display())))?;

    let value: Value = serde_json::from_str(&text)
        .map_err(|e| HostError::message(format!("{}: not a game data pack ({e})", file.display())))?;
    let Some(obj) = value.as_object() else {
        return Err(HostError::message(format!("{}: not a game data pack", file.display())));
    };

    let format = obj.get("format").and_then(Value::as_str).unwrap_or_default();
    if format != PACK_FORMAT {
        return Err(HostError::message(format!(
            "{}: not a game data pack (format is {format:?})",
            file.display()
        )));
    }

    let version = obj.get("version").and_then(Value::as_u64).unwrap_or(0);
    if version != PACK_VERSION {
        return Err(HostError::message(format!(
            "{}: unsupported game data pack version {version}",
            file.display()
        )));
    }

    let expected_game = game_tag();
    let game = obj.get("game").and_then(Value::as_str).unwrap_or_default();
    if game != expected_game {
        return Err(HostError::message(format!(
            "{}: game data pack is for {game}, expected {expected_game}",
            file.display()
        )));
    }

    let created = obj
        .get("created")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();

    let Some(files_arr) = obj.get("files").and_then(Value::as_array) else {
        return Err(HostError::message(format!(
            "{}: game data pack has no files list",
            file.display()
        )));
    };
    if files_arr.is_empty() {
        return Err(HostError::message(format!(
            "{}: game data pack is empty",
            file.display()
        )));
    }

    let known = pack_entries(p);
    let mut seen = HashSet::with_capacity(files_arr.len());
    let mut files = Vec::with_capacity(files_arr.len());

    for item in files_arr {
        let Some(fobj) = item.as_object() else {
            return Err(HostError::message(format!(
                "{}: invalid file entry in game data pack",
                file.display()
            )));
        };
        let Some(rel_path) = fobj.get("path").and_then(Value::as_str) else {
            return Err(HostError::message(format!(
                "{}: file entry missing path",
                file.display()
            )));
        };
        if !known.iter().any(|(k, _)| k == rel_path) {
            return Err(HostError::message(format!(
                "{}: unknown file {rel_path} in game data pack",
                file.display()
            )));
        }
        if !seen.insert(rel_path.to_owned()) {
            return Err(HostError::message(format!(
                "{}: duplicate file {rel_path}",
                file.display()
            )));
        }
        let Some(expected_sha) = fobj.get("sha256").and_then(Value::as_str) else {
            return Err(HostError::message(format!(
                "{}: {rel_path} is missing sha256",
                file.display()
            )));
        };
        let Some(data) = fobj.get("data").and_then(Value::as_str) else {
            return Err(HostError::message(format!(
                "{}: {rel_path} is missing data",
                file.display()
            )));
        };
        let actual_sha = sha256_hex(data);
        if actual_sha != expected_sha {
            return Err(HostError::message(format!(
                "{}: {rel_path} in pack is damaged (expected {expected_sha}, got {actual_sha})",
                file.display()
            )));
        }
        serde_json::from_str::<Value>(data)
            .map_err(|e| HostError::message(format!("{}: {rel_path} is not JSON ({e})", file.display())))?;
        files.push(PackFile {
            path: rel_path.to_owned(),
            sha256: expected_sha.to_owned(),
            data: data.to_owned(),
        });
    }

    Ok(Pack {
        format: format.to_owned(),
        version,
        game: game.to_owned(),
        created,
        files,
    })
}

/// Writes a pack's files into the game data, backing up each generated file it replaces.
pub fn import_game_data_pack(p: &Paths, file: &Path, now: DateTime<Utc>) -> Result<Imported, HostError> {
    let pack = read_game_data_pack(p, file)?;
    let known = pack_entries(p);
    let stamp = iso_millis(now).replace([':', '.'], "-");

    let mut written = Vec::new();
    let mut unchanged = Vec::new();
    let mut backups = Vec::new();

    for entry in &pack.files {
        let Some((_, dest)) = known.iter().find(|(k, _)| *k == entry.path) else {
            continue;
        };
        if dest.is_file() {
            let existing = std::fs::read_to_string(dest).map_err(|e| HostError::io(dest, e))?;
            if existing == entry.data {
                unchanged.push(entry.path.clone());
                continue;
            }
            if !is_example_path(dest)? {
                let backup = PathBuf::from(format!("{}.{stamp}.bak", dest.display()));
                std::fs::copy(dest, &backup).map_err(|e| HostError::io(&backup, e))?;
                backups.push(backup);
            }
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| HostError::io(parent, e))?;
        }
        std::fs::write(dest, &entry.data).map_err(|e| HostError::io(dest, e))?;
        written.push(entry.path.clone());
    }

    Ok(Imported {
        written,
        unchanged,
        backups,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

    use chrono::TimeZone;

    use super::*;
    use crate::gamedata::seed_embedded_examples;

    #[test]
    fn exports_real_files_and_imports_with_backups() {
        let src_tmp = tempfile::tempdir().unwrap();
        let src_paths = Paths::new(src_tmp.path());
        seed_embedded_examples(&src_paths).unwrap();

        let now = Utc.with_ymd_and_hms(2026, 4, 17, 12, 0, 0).unwrap();
        let pack_file = default_pack_file(&src_paths, now);

        // Write one real file
        std::fs::write(
            src_paths.backend_game_data.join("progression_config.json"),
            "{\"real\":1}\n",
        )
        .unwrap();

        let exported = export_game_data_pack(&src_paths, &pack_file, now).unwrap();
        assert!(
            exported
                .included
                .contains(&"backend/progression_config.json".to_owned())
        );

        // Import into a second workspace where progression_config.json already has different real data
        let dst_tmp = tempfile::tempdir().unwrap();
        let dst_paths = Paths::new(dst_tmp.path());
        std::fs::create_dir_all(&dst_paths.backend_game_data).unwrap();
        std::fs::write(
            dst_paths.backend_game_data.join("progression_config.json"),
            "{\"old\":true}\n",
        )
        .unwrap();

        let imported = import_game_data_pack(&dst_paths, &pack_file, now).unwrap();
        assert!(imported.written.contains(&"backend/progression_config.json".to_owned()));
        assert_eq!(imported.backups.len(), 1);
        assert_eq!(
            std::fs::read_to_string(&imported.backups[0]).unwrap(),
            "{\"old\":true}\n"
        );

        // Importing again marks it unchanged
        let imported2 = import_game_data_pack(&dst_paths, &pack_file, now).unwrap();
        assert!(
            imported2
                .unchanged
                .contains(&"backend/progression_config.json".to_owned())
        );
        assert!(imported2.written.is_empty());
    }
}
