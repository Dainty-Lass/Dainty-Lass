// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Talking to servers: endpoints, pinned TLS, region ping, the game's local relay, runtime downloads, game file checks and versions.

pub mod connector;
pub mod epic_install;
pub mod hostapi;
pub mod http;
pub mod oauth_redirect;
pub mod pinned;
pub mod regionping;
pub mod relay;
pub mod relay_owner;
pub mod runtime_files;
pub mod verify;
pub mod versions;
