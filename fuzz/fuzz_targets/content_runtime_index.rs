// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `runtime.json` as the content server reads it (RX3, docs/security/content.md): what it accepts decides which
//! files players are told to write next to the game.
#![no_main]

use dainty_content::input::parse_runtime_json;
use libfuzzer_sys::fuzz_target;
use dainty_protocol::manifest::{MAX_RUNTIME_FILE_BYTES, RUNTIME_FILE_NAMES, runtime_file_slot};

fuzz_target!(|data: &[u8]| {
    let Ok(manifest) = parse_runtime_json(data) else {
        return;
    };
    // The two runtime files, each once, sorted, within the size cap and with a plain hash
    let mut slots: Vec<&str> = manifest.files.iter().filter_map(|f| runtime_file_slot(&f.name)).collect();
    assert_eq!(slots.len(), manifest.files.len());
    slots.sort_unstable();
    assert_eq!(slots, RUNTIME_FILE_NAMES);
    assert!(manifest.files.windows(2).all(|w| w[0].name <= w[1].name));
    for file in &manifest.files {
        assert!((1..=MAX_RUNTIME_FILE_BYTES).contains(&file.size));
        assert!(file.sha256.len() == 64 && file.sha256.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
    }
    assert!((1..=48).contains(&manifest.version.len()));
    assert!(manifest.build.changelist > 0);
    // The body it sends reads back as the same manifest
    assert_eq!(parse_runtime_json(manifest.to_json().as_bytes()), Ok(manifest));
});
