//! A Node kit's host.json, which anything with the old checkout could have written, through the legacy importer's
//! host.toml conversion (spec-legacy-import §11.2, §12): an error or a host.toml that reads back, never a panic.
#![no_main]

use dainty_legacy::data;
use dainty_legacy::reforged::hostjson::convert;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(reforged) = data::reforged() else {
        return;
    };
    if let Ok(converted) = convert(text, reforged) {
        assert!(converted.toml.starts_with(&reforged.header));
        // Whatever the keys and strings, what is written is TOML
        assert!(toml::from_str::<toml::Table>(&converted.toml).is_ok());
    }
});
