// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The local relay, now `dainty-relay`'s: the launcher runs it only for a server whose runtime does not run it
//! inside the game.

/// The level relay lines carry; the launcher maps it onto its log's.
pub use dainty_relay::Level;
pub use dainty_relay::relay::*;
