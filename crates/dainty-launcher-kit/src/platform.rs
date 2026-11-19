// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Platform discovery, host workspace tools, server lifecycle, and OS integration helpers.

pub mod clipboard;
pub mod dialogs;
pub mod epic_library;
pub mod epic_session;
pub mod game_config;
pub mod game_console;
pub mod game_folder;
pub mod game_removal;
#[cfg(feature = "realms")]
pub mod hosted_server;
pub mod hostkit;
pub mod hostserver;
pub mod inject;
pub mod invite_link;
pub mod keystore;
pub mod launch;
pub mod links;
pub mod linux_runtime;
pub mod loader_session;
pub mod log;
pub mod paths;
pub mod requirements;
pub mod server_engine;
pub mod settings;
pub mod system;
