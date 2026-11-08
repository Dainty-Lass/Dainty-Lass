// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The launcher's release version from `launcher/package.json`, the one place it is set: Tauri's config reads the
//! same file, so the client, the manager and what the controller reports agree (RL14).

use std::error::Error;

const PACKAGE: &str = "../../launcher/package.json";

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed={PACKAGE}");
    let package: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(PACKAGE)?)?;
    let version = package
        .get("version")
        .and_then(serde_json::Value::as_str)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("{PACKAGE} has no version"))?;
    println!("cargo:rustc-env=DAINTY_LAUNCHER_VERSION={version}");
    Ok(())
}
