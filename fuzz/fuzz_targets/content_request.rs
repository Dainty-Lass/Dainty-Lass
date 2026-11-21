// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! What a request brings the content server (RX3, docs/security/content.md): its raw path, its `If-None-Match`,
//! its account key and its version query. The first line is the path, the rest header values.
#![no_main]

use content::input::{
    Route, VersionKey, header_text, if_none_match_hits, is_plausible_key, is_runtime_file_name, is_version_segment,
    version_query,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let mut lines = text.split('\n');
    let target = lines.next().unwrap_or_default();
    let (path, query) = target.split_once('?').map_or((target, None), |(p, q)| (p, Some(q)));
    match Route::of(path) {
        // A version route only ever names a plain segment: no `/`, `%` or uppercase reaches the module table
        Route::ClientProfile(v) | Route::EpicManifest(v) => assert!(is_version_segment(v)),
        Route::RuntimeFile(name) => {
            assert!(path.starts_with("/content/v1/runtime/files/") && path.ends_with(name));
            // A served name is one of the runtime's own, never a path
            if is_runtime_file_name(name) {
                assert!(name.ends_with(".dll") && !name.contains(['/', '%']));
            }
        }
        Route::BrandingImage(name) => assert!(path.ends_with(name)),
        _ => {}
    }
    if let Some((key, value)) = version_query(query) {
        assert!(matches!(key, VersionKey::Version | VersionKey::Edition));
        _ = value;
    }
    let mut headers = http::HeaderMap::new();
    for line in lines {
        if let Ok(value) = http::HeaderValue::from_bytes(line.as_bytes()) {
            headers.append("x-reforged-user-api-key", value.clone());
            headers.append("if-none-match", value);
        }
    }
    if let Some(key) = header_text(&headers, "x-reforged-user-api-key")
        && is_plausible_key(&key)
    {
        // A key the backend is asked about is one header line of printable ASCII
        assert!(key.len() <= 256 && !key.contains([' ', ',', '\r', '\n']));
        assert_eq!(headers.get_all("x-reforged-user-api-key").iter().count(), 1);
    }
    let condition = header_text(&headers, "if-none-match");
    assert!(!if_none_match_hits(None, "\"x\""));
    if condition.as_deref().is_some_and(|c| c.trim() == "*") {
        assert!(if_none_match_hits(condition.as_deref(), "\"anything\""));
    }
});
