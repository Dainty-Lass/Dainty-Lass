// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Wire contracts the launcher and the servers share: invites, registration, server status and the runtime
//! manifest, validated with the JavaScript text rules the Node services used.

pub mod account;
pub mod invite;
pub mod manifest;
pub mod status;
pub mod text;

pub use dainty_module_api::GameVersion;

/// The shared tables and fixtures the launcher and the servers check against (`tests/cases`, `tests/fixtures`), for
/// dainty-restricted's tests, which cannot read this repository's files by path (RD27).
#[cfg(feature = "test-data")]
pub mod test_data {
    /// Invite parser cases (`tests/cases/invite.json`).
    pub const INVITE_CASES: &str = include_str!("../tests/cases/invite.json");
    /// Server status parser cases (`tests/cases/status.json`).
    pub const STATUS_CASES: &str = include_str!("../tests/cases/status.json");
    /// Username check cases (`tests/cases/username.json`).
    pub const USERNAME_CASES: &str = include_str!("../tests/cases/username.json");
    /// `(name, input, expected body)` for the content server's runtime manifest
    pub const RUNTIME_MANIFESTS: [(&str, &str, &str); 2] = [
        (
            "runtime-manifest",
            include_str!("../tests/fixtures/runtime-manifest.input.json"),
            include_str!("../tests/fixtures/runtime-manifest.json"),
        ),
        (
            "runtime-manifest-defaults",
            include_str!("../tests/fixtures/runtime-manifest-defaults.input.json"),
            include_str!("../tests/fixtures/runtime-manifest-defaults.json"),
        ),
    ];
}
