// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use serde::{Deserialize, Serialize};

pub const RUNTIME_MANIFEST_PATH: &str = "/content/v1/runtime/manifest";
pub const RUNTIME_BUILD_NAME: &str = "rel-1.14.7-Archon";
pub const DEFAULT_CHANGELIST: u64 = 647_472;
pub const RUNTIME_DLL: &str = "Rustic-Isles.dll";
/// [`RUNTIME_DLL`]'s name before the Rustic Isles rename: a host that still publishes it is served for one release (DX4)
pub const LEGACY_RUNTIME_DLL: &str = "MysticParadox.dll";
pub const RUNTIME_FILE_NAMES: [&str; 2] = [RUNTIME_DLL, "winmm.dll"];
pub const MAX_RUNTIME_FILE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RuntimeManifest {
    pub version: String,
    pub build: RuntimeBuild,
    pub files: Vec<RuntimeFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RuntimeBuild {
    pub name: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub changelist: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RuntimeFile {
    pub name: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub size: u64,
    pub sha256: String,
}

/// The [`RUNTIME_FILE_NAMES`] entry `name` stands for, letter case included: itself, or [`RUNTIME_DLL`] for
/// [`LEGACY_RUNTIME_DLL`]
#[must_use]
pub fn runtime_file_slot(name: &str) -> Option<&'static str> {
    if name == LEGACY_RUNTIME_DLL {
        return Some(RUNTIME_DLL);
    }
    RUNTIME_FILE_NAMES.into_iter().find(|known| *known == name)
}

impl RuntimeManifest {
    /// Constructs a new [`RuntimeManifest`] with the fixed build name `rel-1.14.7-Archon`
    /// and files sorted lexicographically by name.
    #[must_use]
    pub fn new(version: String, changelist: u64, mut files: Vec<RuntimeFile>) -> Self {
        files.sort_by(|a, b| a.name.cmp(&b.name));
        Self {
            version,
            build: RuntimeBuild {
                name: RUNTIME_BUILD_NAME.to_owned(),
                changelist,
            },
            files,
        }
    }

    /// Serializes the manifest to compact JSON (`JSON.stringify`).
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}
