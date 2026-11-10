// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Copies the 1.14.7 version data `DAINTY_VERSION_DATA` names into `OUT_DIR`, or an empty file when it names none:
//! the document is the game module's, not this crate's, so a build that wants it compiled in says where it is.

use std::path::{Path, PathBuf};

fn main() -> Result<(), String> {
    println!("cargo::rerun-if-env-changed=DAINTY_VERSION_DATA");
    println!("cargo::rustc-check-cfg=cfg(dainty_version_data)");
    let out = PathBuf::from(std::env::var("OUT_DIR").map_err(|e| format!("OUT_DIR: {e}"))?).join("version_data.json");
    let document = match std::env::var_os("DAINTY_VERSION_DATA") {
        Some(path) if !path.is_empty() => {
            let path = Path::new(&path);
            println!("cargo::rerun-if-changed={}", path.display());
            println!("cargo::rustc-cfg=dainty_version_data");
            std::fs::read(path).map_err(|e| format!("DAINTY_VERSION_DATA ({}): {e}", path.display()))?
        }
        _ => Vec::new(),
    };
    std::fs::write(&out, document).map_err(|e| format!("{}: {e}", out.display()))
}
