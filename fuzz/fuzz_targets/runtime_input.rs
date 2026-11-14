//! What the launcher reads while it handles the server's game runtime (MIGRATION_PLAN.md RA7, RX3; RD27 spec
//! §2.6.3): the runtime manifest a content server sends, which decides which files are written next to the game,
//! and the hash cache file (`file-hashes.json`) read back at start.
#![no_main]

use dainty_launcher_kit::net::runtime_files::{AGPL_RUNTIME_DLL, SET_FILE_NAMES, parse_runtime_manifest, set_file_slot};
use dainty_launcher_kit::net::verify::{MAX_ENTRIES, parse_cache_file, write_cache_file};
use libfuzzer_sys::fuzz_target;
use dainty_protocol::manifest::{MAX_RUNTIME_FILE_BYTES, MIT_RUNTIME_DLL};
use serde_json::Value;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    if let Ok(value) = serde_json::from_str::<Value>(&text) {
        if let Some(parsed) = parse_runtime_manifest(&value) {
            // Only runtime names, each once and sorted, a runtime always (either set shape, spec-runtime-r21 7.2),
            // so nothing else is ever written next to the game; a plain hash and a size within the cap
            let mut names: Vec<&str> = parsed
                .files
                .iter()
                .filter_map(|file| set_file_slot(&file.name))
                .collect();
            assert_eq!(names.len(), parsed.files.len());
            assert!(parsed.files.windows(2).all(|w| w[0].name <= w[1].name));
            names.sort_unstable();
            names.dedup();
            assert_eq!(names.len(), parsed.files.len());
            assert!(names.iter().any(|n| [MIT_RUNTIME_DLL, AGPL_RUNTIME_DLL].contains(n)));
            assert!(names.iter().all(|n| SET_FILE_NAMES.contains(n)));
            for file in &parsed.files {
                assert!((1..=MAX_RUNTIME_FILE_BYTES).contains(&file.size));
                assert!(
                    file.sha256.len() == 64
                        && file
                            .sha256
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                );
            }
            assert!(!parsed.version.is_empty() && parsed.version.encode_utf16().count() <= 48);
            // And it writes back as a manifest with the same files and build (the version is cleaned
            // text, which a second cleaning may trim further)
            let again = serde_json::from_str::<Value>(&parsed.to_json())
                .ok()
                .as_ref()
                .and_then(parse_runtime_manifest);
            assert_eq!(again.map(|m| (m.files, m.build)), Some((parsed.files, parsed.build)));
        }
    }
    let entries = parse_cache_file(&text);
    assert!(entries.len() <= MAX_ENTRIES);
    assert_eq!(parse_cache_file(&write_cache_file(&entries)).len(), entries.len());
});
