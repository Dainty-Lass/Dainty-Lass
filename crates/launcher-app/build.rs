// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Tauri's app manifest from launcher-kit's command table (RD27 spec §2.1.4): each command gets its
//! `allow-`/`deny-` permissions, and the `app-commands` set grants exactly the table.

use std::error::Error;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const SPEC: &str = "../launcher-kit/src/bridge/spec.rs";
/// launcher-kit's generator test keeps this equal to its `commands.js`, banner aside (L4).
const COMMANDS_FIXTURE: &str = "../launcher-kit/tests/fixtures/bridge/commands.js";
/// Tauri inlines the isolation page's scripts at compile time, so the hook's table must exist before it.
const ISOLATION_COMMANDS: &str = "../../launcher/isolation/commands.js";
const BANNER: &str =
    "// Generated from launcher-kit's command table (crates/launcher-kit/src/bridge/spec.rs); do not edit.\n";

/// The names in `COMMANDS`, in order. A build script cannot use launcher-kit, so the table is read as text; a
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

/// The permission set, in the generated file's form (§2.3.6 item 3).
fn permission_set(names: &[String]) -> String {
    let mut out = String::from(
        "# Generated from launcher-kit's command table by launcher-app's build script; do not edit.\n\n\
         [[set]]\nidentifier = \"app-commands\"\n\
         description = \"Every command in src/bridge/spec.rs, and nothing else.\"\npermissions = [\n",
    );
    for name in names {
        let _ = writeln!(out, "  \"allow-{}\",", name.replace('_', "-"));
    }
    out.push_str("]\n");
    out
}

/// The file without its leading comment lines, as the fixture is normalised.
fn without_banner(text: &str) -> String {
    let text = text.replace("\r\n", "\n");
    let body: Vec<&str> = text.lines().skip_while(|line| line.starts_with("//")).collect();
    let mut out = body.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Writes the hook's table unless an equal one (from this or from `bun run restricted-data`) is there.
fn isolation_commands() -> Result<(), Box<dyn Error>> {
    let table = std::fs::read_to_string(COMMANDS_FIXTURE)?;
    let current = std::fs::read_to_string(ISOLATION_COMMANDS).unwrap_or_default();
    if without_banner(&current) != without_banner(&table) {
        std::fs::write(ISOLATION_COMMANDS, format!("{BANNER}{table}"))?;
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed={SPEC}");
    println!("cargo:rerun-if-changed={COMMANDS_FIXTURE}");
    isolation_commands()?;
    println!("cargo:rerun-if-changed=capabilities");
    println!("cargo:rerun-if-changed=../../launcher/isolation");
    let names = command_names(&std::fs::read_to_string(SPEC)?);
    if names.is_empty() {
        return Err(format!("no commands found in {SPEC}").into());
    }

    let dir = PathBuf::from(std::env::var("OUT_DIR")?).join("permissions");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("app-commands.toml"), permission_set(&names))?;
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
