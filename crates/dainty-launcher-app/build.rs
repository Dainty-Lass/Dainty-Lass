// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tauri's app manifest from dainty-launcher-kit's command table (RD27 spec §2.1.4): each command gets its
//! `allow-`/`deny-` permissions, and the `app-commands` set grants exactly the table. The set and the isolation
//! hook's table are dainty-launcher-kit's generated files.

use std::error::Error;
use std::path::{Path, PathBuf};

use serde_json::Value;

const SPEC: &str = "../dainty-launcher-kit/src/bridge/spec.rs";
/// dainty-launcher-kit's generator output; its `generated` test keeps both current.
const COMMANDS: &str = "../dainty-launcher-kit/generated/commands.js";
const PERMISSIONS: &str = "../dainty-launcher-kit/generated/app-commands.toml";
/// Tauri inlines the isolation page's scripts at compile time, so the hook's table must exist before it.
const ISOLATION_COMMANDS: &str = "../../launcher/isolation/commands.js";
/// The manager's differences from tauri.conf.json, for the compiled-in context.
const MANAGER_CONFIG: &str = "tauri.server-gui.conf.json";
const CLIENT_CONFIG: &str = "tauri.conf.json";
/// What a `custom-protocol` build embeds when its page was never built.
const NO_PAGE: &str = "no-page";

/// The names in `COMMANDS`, in order. A build script cannot use dainty-launcher-kit, so the table is read as text; a
/// name may sit on the line after its `cmd(`.
fn command_names(spec: &str) -> Vec<String> {
    let Some(start) = spec.find("pub const COMMANDS") else {
        return Vec::new();
    };
    let table = &spec[start..];
    let table = table.find("\n];").map_or(table, |end| &table[..end]);
    let mut names = Vec::new();
    let mut rest = table;
    while let Some(at) = rest.find("cmd(") {
        rest = &rest[at + 4..];
        let trimmed = rest.trim_start();
        if let Some(body) = trimmed.strip_prefix('"')
            && let Some(end) = body.find('"')
        {
            names.push(body[..end].to_owned());
        }
    }
    names
}

fn unix_lines(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// Writes the hook's table unless an equal one (from this or from `bun run restricted-data`) is there.
fn isolation_commands() -> Result<(), Box<dyn Error>> {
    let table = unix_lines(&std::fs::read_to_string(COMMANDS)?);
    let current = unix_lines(&std::fs::read_to_string(ISOLATION_COMMANDS).unwrap_or_default());
    if current != table {
        std::fs::write(ISOLATION_COMMANDS, table)?;
    }
    Ok(())
}

/// The compiled-in context's config. Tauri's codegen merges `TAURI_CONFIG` over tauri.conf.json, and the value
/// set here wins over one the CLI passes. The manager build sets its own (the CLI's paths are relative to
/// restricted's crate); a `custom-protocol` build whose page is missing (`--all-features`) embeds [`NO_PAGE`].
fn context_config() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed={MANAGER_CONFIG}");
    let mut patch = if std::env::var_os("CARGO_FEATURE_SERVER_GUI").is_some() {
        Some(serde_json::from_str::<Value>(&std::fs::read_to_string(
            MANAGER_CONFIG,
        )?)?)
    } else {
        None
    };
    if std::env::var_os("CARGO_FEATURE_CUSTOM_PROTOCOL").is_some() {
        let base = serde_json::from_str::<Value>(&std::fs::read_to_string(CLIENT_CONFIG)?)?;
        let page = [patch.as_ref(), Some(&base)]
            .into_iter()
            .flatten()
            .find_map(|config| config.pointer("/build/frontendDist")?.as_str())
            .unwrap_or_default()
            .to_owned();
        println!("cargo:rerun-if-changed={page}");
        if !Path::new(&page).is_dir() {
            println!("cargo:warning={page} is not built; this binary shows a placeholder page");
            // Kept whole when not the manager: it carries the CLI's release overlay
            let config = patch.get_or_insert_with(|| {
                std::env::var("TAURI_CONFIG")
                    .ok()
                    .and_then(|env| serde_json::from_str(&env).ok())
                    .unwrap_or_else(|| Value::Object(serde_json::Map::new()))
            });
            if let Some(config) = config.as_object_mut() {
                let build = config
                    .entry("build")
                    .or_insert_with(|| Value::Object(serde_json::Map::new()));
                if let Some(build) = build.as_object_mut() {
                    build.insert("frontendDist".to_owned(), Value::from(NO_PAGE));
                }
            }
        }
    }
    if let Some(patch) = patch {
        println!("cargo:rustc-env=TAURI_CONFIG={patch}");
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed={SPEC}");
    println!("cargo:rerun-if-changed={COMMANDS}");
    println!("cargo:rerun-if-changed={PERMISSIONS}");
    isolation_commands()?;
    context_config()?;
    println!("cargo:rerun-if-changed=capabilities");
    println!("cargo:rerun-if-changed=../../launcher/isolation");
    let names = command_names(&std::fs::read_to_string(SPEC)?);
    if names.is_empty() {
        return Err(format!("no commands found in {SPEC}").into());
    }

    let dir = PathBuf::from(std::env::var("OUT_DIR")?).join("permissions");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(
        dir.join("app-commands.toml"),
        unix_lines(&std::fs::read_to_string(PERMISSIONS)?),
    )?;
    let pattern = format!("{}/*.toml", slashes(&dir));

    // Tauri keeps both for the whole build
    let names: &'static [&'static str] = Vec::leak(names.into_iter().map(|n| &*String::leak(n)).collect());
    let manifest = tauri_build::AppManifest::new()
        .commands(names)
        .permissions_path_pattern(String::leak(pattern));
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))?;
    Ok(())
}

fn slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
