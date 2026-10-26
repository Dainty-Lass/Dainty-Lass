//! What the launcher's legacy edition reads from a Dauntless Revived server (RX3; DR's L17, L23): the content
//! server's 1.4.4 manifest, whose paths name the files the download writes under the game folder. The
//! launcher's own check (`core/manifest_check.rs`, with DR's pins), compiled in here, since the launcher
//! crate needs a webview to build.
#![no_main]
#![allow(dead_code)]

use libfuzzer_sys::fuzz_target;
use serde_json::Value;

mod pins {
    include!("../../launcher/src-tauri/src/core/legacy_pins.rs");
}

mod check {
    use super::pins::*;
    use serde_json::Value;
    use sha2::{Digest, Sha256};

    include!("../../launcher/src-tauri/src/core/manifest_check.rs");
}

use check::{compare_manifests, is_safe_relative_path, manifest_fingerprint, validate_manifest};

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data)
        && is_safe_relative_path(text)
    {
        // Nothing that climbs out, names a drive or a stream, or uses the other separator
        assert!(!text.contains(['\\', ':']) && !text.starts_with('/'));
        assert!(
            text.split('/')
                .all(|part| !part.is_empty() && part != "." && part != "..")
        );
    }
    let Ok(raw) = serde_json::from_slice::<Value>(data) else {
        return;
    };
    let Ok(manifest) = validate_manifest(&raw, pins::EXPECTED_BUILD, false) else {
        return;
    };
    let mut seen = std::collections::HashSet::new();
    let mut sum = 0u64;
    for file in &manifest.files {
        assert!(is_safe_relative_path(&file.path), "{}", file.path);
        assert!(seen.insert(file.path.to_ascii_lowercase()), "{}", file.path);
        assert!(
            !pins::MANAGED_FILES
                .iter()
                .any(|name| file
                    .path
                    .eq_ignore_ascii_case(&format!("{}/{}", pins::WIN64_RELATIVE_DIR, name)))
        );
        assert!(file.sha256.len() == 64);
        sum += file.size;
    }
    assert_eq!(sum, manifest.total_bytes);
    assert!(manifest.files.windows(2).all(|pair| pair[0].path < pair[1].path));
    // The same manifest compares equal to itself, in any order
    assert_eq!(compare_manifests(&manifest, &raw), Ok(()));
    assert_eq!(manifest_fingerprint(&manifest).len(), 64);
});
