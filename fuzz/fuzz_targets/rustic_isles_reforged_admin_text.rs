//! The admin half's text rules (spec-rustic-isles-reforged-admin §18): the secure-URL reader, the Discord state
//! check, `parseInt`, the run id and the kit validator, the local admin check. Nothing panics; an accepted URL has no
//! `@` in its authority, no `#`, and is https off localhost in production; only states made for a binding verify.
#![no_main]

use std::sync::LazyLock;

use http::{HeaderMap, HeaderValue};
use libfuzzer_sys::fuzz_target;
use rustic_isles_reforged::admin::discord::{make_state, secure_url, verify_state};
use rustic_isles_reforged::admin::host_routes::local_admin;
use rustic_isles_reforged::admin::parse_int;
use rustic_isles_reforged::admin::tester_max::{is_run_id, validate_kit};
use rustic_isles_reforged::test_support::FakeFunding;

static SHARED: LazyLock<std::sync::Arc<rustic_isles::Shared>> =
    LazyLock::new(|| rustic_isles::Shared::bare(rustic_isles::testing::sample_version()).unwrap());

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    if let Ok(url) = secure_url("X", Some(&text), true) {
        assert!(!url.origin.contains('@') && !text.contains('#') && url.origin.starts_with("https://"));
    }
    let _ = verify_state(Some(&text), &text);
    let binding = [3u8; 32];
    let state = make_state(&binding, &"a".repeat(48));
    assert!(!verify_state(Some(&text), &state) || text.len() == 43);
    let _ = (parse_int(&text), is_run_id(&text));
    if let Ok(kit) = serde_json::from_slice::<serde_json::Value>(data) {
        let _ = validate_kit(&kit, &SHARED, &FakeFunding::default());
    }
    let mut headers = HeaderMap::new();
    if let Ok(value) = HeaderValue::from_str(&text) {
        headers.insert("x-dainty-admin-key", value);
    }
    let _ = local_admin(&rustic_isles::Settings::default(), &text, &headers);
});
