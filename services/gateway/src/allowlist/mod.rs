// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The allowlist helper: keeps the game's UDP ports open only to the addresses of players who logged in, as the
//! gateway reports them. It runs with administrator rights (Windows) or as root or with `CAP_NET_ADMIN` (Linux),
//! so it listens on loopback only, wants a shared secret, never runs a shell, and only ever changes its own
//! firewall rule or table.

pub mod cli;
pub mod config;
pub mod firewall;
pub mod guard;
pub mod helper;
pub mod modules;
pub mod nft;
pub mod ports;
pub mod state;
pub mod sync;
#[cfg(windows)]
pub mod windows;
