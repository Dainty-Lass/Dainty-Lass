// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Core constants, text utilities, and protocol re-exports for the launcher.

pub mod pins;
pub mod text;

pub mod legacy_pins {
    pub const PINNED_EXE_SHA256: &str =
        "d3d41e614908d2befd518b27046d9822d6130ef12ba3504babbdb786bef9cff4";
}

pub mod invite {
    pub use protocol::invite::*;
}

pub mod username {
    pub use protocol::account::*;
}
