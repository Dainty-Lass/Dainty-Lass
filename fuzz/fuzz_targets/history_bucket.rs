//! What a restore reads from the history bucket (docs/dainty-lass-plan.md DH4; docs/security/history.md): anyone
//! with the server's write-only key can add objects, so listings, object names and segment bodies are untrusted.
//! Whatever the bytes: no panic; a parsed name gives back its own key; an accepted segment's entries are in
//! `(after, through]`, increasing, ending at `through`. Segment bodies are wrapped in gzip under a name made from
//! their own hash, so the fuzzer reaches the JSON.
#![no_main]

use arbitrary::Arbitrary;
use dainty_history_ship::layout::{SegmentName, SnapshotName, decode_segment, gzip, sha256_hex};
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
enum Input<'a> {
    /// A ListObjectsV2 reply
    Listing(&'a str),
    /// An object key under a prefix
    Key { prefix: &'a str, key: &'a str },
    /// A segment's uncompressed body, under a name for it
    Segment { after: i64, through: i64, text: &'a [u8] },
    /// A segment body as stored, gzip or not
    Raw { after: i64, through: i64, body: &'a [u8] },
}

fn check(name: &SegmentName, body: &[u8]) {
    if let Ok(entries) = decode_segment(name, body) {
        let mut previous = name.after;
        for entry in &entries {
            assert!(entry.id > previous && entry.id <= name.through);
            previous = entry.id;
        }
        assert_eq!(previous, name.through);
    }
}

fuzz_target!(|input: Input<'_>| match input {
    Input::Listing(xml) => {
        let _ = dainty_history_ship::s3::parse_list(xml);
    }
    Input::Key { prefix, key } => {
        if let Some(name) = SegmentName::parse(prefix, key) {
            assert_eq!(name.key(prefix), key);
        }
        if let Some(name) = SnapshotName::parse(prefix, key) {
            assert_eq!(name.key(prefix), key);
        }
    }
    Input::Segment { after, through, text } => {
        if let Ok(body) = gzip(text) {
            let name = SegmentName {
                after,
                through,
                sha256: sha256_hex(&body),
            };
            check(&name, &body);
        }
    }
    Input::Raw { after, through, body } => {
        let name = SegmentName {
            after,
            through,
            sha256: sha256_hex(body),
        };
        check(&name, body);
    }
});
