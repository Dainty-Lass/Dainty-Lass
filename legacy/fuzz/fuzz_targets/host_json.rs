//! A Node kit's host.json is the host's own file, but it is parsed and rewritten as TOML: nothing may panic, and
//! whatever converts must be TOML that parses.
#![no_main]

use legacy_import::reforged::hostjson::convert;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(converted) = convert(text) {
        assert!(converted.toml.parse::<toml_edit::DocumentMut>().is_ok(), "{}", converted.toml);
    }
});
