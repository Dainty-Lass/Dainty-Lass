// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The launcher's MIT logic (DL4b), shared by dainty-restricted's Tauri shells: the bridge's command table and
//! types, server and content networking, game file checks, and the host kit's platform helpers.

/// The launcher's release version, from `launcher/package.json` (build.rs).
pub const VERSION: &str = env!("DAINTY_LAUNCHER_VERSION");

pub mod bridge;
pub mod controller;
pub mod core;
pub mod net;
pub mod platform;
