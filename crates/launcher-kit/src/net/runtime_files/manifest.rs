// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The runtime marker file and the manifest's limits; the manifest is read by `hostapi::parse`.

/// File in the game's `Win64` folder recording which runtime was installed and from where.
pub const RUNTIME_MARKER_FILE: &str = ".reforged-runtime.json";
/// JavaScript's `Number.MAX_SAFE_INTEGER`, the largest changelist the Node services could send.
pub const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

pub use crate::net::hostapi::parse::parse_runtime_manifest;
