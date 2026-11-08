// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Core constants, text utilities, and protocol re-exports for the launcher.

pub mod game_language;
pub mod i18n;
pub mod pins;
pub mod text;

/// The server status reply, shared with the servers through `protocol`.
pub mod status {
    pub use dainty_protocol::status::*;
}

/// The invite parser, shared with the servers through `protocol`.
pub mod invite {
    pub use dainty_protocol::invite::*;
}

/// The username rules, shared with the servers through `protocol`.
pub mod username {
    pub use dainty_protocol::account::*;
}
