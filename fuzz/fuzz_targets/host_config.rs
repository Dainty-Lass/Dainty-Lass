//! What the host kit reads from a host's settings.toml and the env files it writes for each service
//! (docs/security/host-start.md). None of it may panic; rewriting an unchanged file reads back the same settings, a
//! new file reads back as the document it was made from, and a value written to an env file reads back as
//! itself, on its own line, never as a second variable.
#![no_main]

use dainty_hostkit::envfile::{format_value, parse_env};
use dainty_hostkit::hostconfig::HostConfig;
use dainty_hostkit::settingstoml::{document, message, text};
use libfuzzer_sys::arbitrary::{self, Arbitrary};
use libfuzzer_sys::fuzz_target;
use serde_json::Map;

#[derive(Debug, Arbitrary)]
struct Input<'a> {
    settings: &'a str,
    env: &'a str,
    value: &'a str,
}

fuzz_target!(|input: Input<'_>| {
    if let Ok(doc) = document(input.settings) {
        let fields = doc.as_object().cloned().unwrap_or_default();
        // Rewriting an unchanged document (the writer drops a BOM and normalises line ends) reads back the same
        if let Ok(same) = text(&fields, &fields, Some(input.settings)) {
            assert_eq!(document(&same).ok(), Some(doc.clone()));
        }
        if let Ok(fresh) = text(&fields, &Map::new(), None) {
            assert_eq!(document(&fresh).ok(), Some(doc.clone()));
        }
        if let Err(error) = HostConfig::from_json(&doc) {
            let _ = message(&error.to_string());
        }
    }

    let _ = parse_env(input.env);
    if let Ok(formatted) = format_value(input.value) {
        let env = parse_env(&format!("DAINTY_FUZZ={formatted}\nDAINTY_AFTER=1\n"));
        assert_eq!(env.get("DAINTY_FUZZ"), Some(input.value));
        assert_eq!(env.len(), 2);
    } else {
        assert!(input.value.contains(['\n', '\r']));
    }
});
