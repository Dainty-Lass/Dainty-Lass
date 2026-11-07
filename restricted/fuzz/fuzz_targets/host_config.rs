//! The server's settings as `serve` and setup read them (docs/security/controller.md): host.toml,
//! host.yaml (the kit's own small YAML reader) and a Node kit's host.json. A broken or hostile file is
//! only ever refused, never a panic or a recursion without end, and a file that is accepted writes back
//! a host.toml that reads as the same settings.
#![no_main]

use dainty_host::hostconfig::{HostConfig, yaml};
use libfuzzer_sys::fuzz_target;
use dainty_service_kit::jsjson;

fuzz_target!(|data: &[u8]| {
    let Some((&form, rest)) = data.split_first() else {
        return;
    };
    let text = String::from_utf8_lossy(rest);
    let config = match form % 3 {
        0 => HostConfig::from_toml(&text).ok(),
        1 => yaml::parse_yaml_to_json(&text)
            .ok()
            .and_then(|document| HostConfig::from_json(&document).ok()),
        _ => jsjson::parse(&text)
            .ok()
            .and_then(|document| HostConfig::from_json(&document).ok()),
    };
    let Some(config) = config else {
        return;
    };
    let written = config.to_toml().expect("an accepted config writes a host.toml");
    let again = HostConfig::from_toml(&written).expect("the host.toml written reads back");
    assert_eq!(again.to_toml().expect("and writes the same again"), written);
});
