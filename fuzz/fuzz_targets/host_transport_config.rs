// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The backend's `transport-config` JSON turned into the transport's files (spec-rd27-rifts-host.md §6.2.1
//! `ice_files`), for each role. Whatever is accepted writes only whole `key=value` lines: no CR or NUL anywhere and no
//! line break inside a value.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_rifts_host::{FilesFor, TransportConfig, ice_files};

const ROLES: [FilesFor; 4] = [FilesFor::Joiner, FilesFor::HuntHost, FilesFor::AwayHuntHost, FilesFor::HubHost];

fn whole_lines(text: &str) {
    assert!(!text.contains(['\r', '\0']));
    assert!(text.is_empty() || text.ends_with('\n'));
    for line in text.lines() {
        let (key, value) = line.split_once('=').expect("a key=value line");
        assert!(!key.is_empty() && !value.is_empty());
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(config) = serde_json::from_slice::<TransportConfig>(data) else {
        return;
    };
    for role in ROLES {
        if let Ok(files) = ice_files(&config, role, &config.host_peer_id) {
            whole_lines(&files.config);
            whole_lines(files.secrets.expose());
        }
    }
});
