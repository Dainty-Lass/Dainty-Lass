//! What Rustic Isles' pipeline reads as text (spec-rustic-isles-core §14): the origin strip, the request id, the
//! `/admin` and `/mystic/v1/hunts` prefixes, `decodeURIComponent`, the switches reader and the per-minute
//! parser. None may panic; a stripped target starts `/`, an accepted host ends in an allowed apex, a kept request
//! id is safe.
#![no_main]

use std::collections::BTreeMap;

use axum::http::{HeaderMap, HeaderValue};
use libfuzzer_sys::fuzz_target;
use rustic_isles::Settings;
use rustic_isles::pipeline::{OriginStrip, is_admin_path, is_hunts_path, request_id_of, safe_request_id, strip_origin};
use serde_json::Value;

const APEXES: [&str; 4] = ["steelyard.ca", "steelyard.online", "ol.epicgames.com", "api.epicgames.dev"];

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);

    for target in [text.to_string(), format!("/__origin/{text}")] {
        if let OriginStrip::Stripped { host, target } = strip_origin(&target) {
            assert!(target.starts_with('/'));
            assert!(APEXES.iter().any(|apex| host == *apex || host.ends_with(&format!(".{apex}"))));
        }
    }

    if let Some(id) = safe_request_id(&text) {
        assert!((8..=96).contains(&id.len()));
        assert!(id.bytes().all(|b| b.is_ascii_alphanumeric() || b".:_-".contains(&b)));
    }
    let mut headers = HeaderMap::new();
    if let Ok(value) = HeaderValue::from_str(&text) {
        headers.insert("x-rustic-isles-request-id", value);
    }
    let id = request_id_of(&headers);
    assert!(safe_request_id(&id) == Some(id.as_str()));

    let _ = is_admin_path(&text);
    let _ = is_hunts_path(&text);
    let _ = service_kit::js::decode_uri_component(&text);

    let value = serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text.to_string()));
    let section = BTreeMap::from([("qos_port".to_owned(), value.clone()), ("rate_limit".to_owned(), value)]);
    let _ = Settings::switches_from_config(Some(&section));
    let settings = Settings::from_pairs([
        ("RUSTIC_ISLES_RATE_LIMIT_PER_MINUTE", text.as_ref()),
        ("RUSTIC_ISLES_RATE_LIMIT", text.as_ref()),
    ]);
    assert!(settings.rate_limit_per_minute() >= 1);
    let _ = settings.rate_limit_enabled();
    let _ = settings.admin_api();
});
