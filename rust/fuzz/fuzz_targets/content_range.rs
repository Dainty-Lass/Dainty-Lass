//! A legacy content server's game download (DR's `content_range`, X9; docs/security/content.md): a
//! `Range` header from the network must give a range inside the file or a refusal, a request path
//! never becomes a manifest key that leaves the game folder, and no manifest text makes the server
//! accept such a path either.
#![no_main]

use arbitrary::Arbitrary;
use content::input::if_none_match_hits;
use content::legacy::manifest::{is_safe_manifest_path, parse};
use content::legacy::paths::parse_file_request_path;
use content::legacy::range::{Range, if_range_allows_partial, parse_range};
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
struct Input<'a> {
    range: &'a str,
    size: u64,
    etag: &'a str,
    target: &'a str,
    manifest: &'a str,
}

fuzz_target!(|input: Input<'_>| {
    if let Range::Partial { start, end } = parse_range(Some(input.range), input.size) {
        assert!(start <= end && end < input.size, "{start}-{end} of {}", input.size);
    }
    let _ = if_range_allows_partial(Some(input.range), input.etag);
    let _ = if_none_match_hits(Some(input.range), input.etag);
    if let Ok(key) = parse_file_request_path(input.target) {
        assert!(!key.is_empty() && !key.contains(['\\', '\0', ':', '%']), "{key:?}");
        assert!(
            !key.split('/')
                .any(|part| part.is_empty() || part == "." || part == ".."),
            "{key:?}"
        );
    }
    if let Ok(manifest) = parse(input.manifest) {
        for file in &manifest.files {
            assert!(
                is_safe_manifest_path(&file.path),
                "an accepted manifest holds {:?}",
                file.path
            );
            assert!(!file.path.split('/').any(|part| part == ".."), "{:?}", file.path);
        }
    }
});
