// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! A player's crash report upload (RX3, docs/security/crash-reports.md): the gzip bundle the content server
//! parses before anything is written. Driven both as an upload and, unpacked, as the frame itself.
#![no_main]

use dainty_content::crash::{self, MAX_FILES, MAX_NAME, MAX_UNPACKED, MAX_VALUE};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    for parsed in [crash::parse(data), crash::parse_frame(data)] {
        let Ok(bundle) = parsed else { continue };
        // Every name becomes a file in the report's folder: never a path, never a hidden or unknown file
        assert!(bundle.files.len() <= MAX_FILES);
        let mut total = 0;
        for (name, body) in &bundle.files {
            assert!(crash::is_file_name(name) && name.len() <= MAX_NAME);
            assert!(!name.contains(['/', '\\', ':']) && !name.starts_with('.'));
            total += body.len();
        }
        assert!(total <= MAX_UNPACKED);
        assert!(bundle.crash_id.len() <= MAX_VALUE && !bundle.crash_id.contains(['/', '\\']));
    }
});
