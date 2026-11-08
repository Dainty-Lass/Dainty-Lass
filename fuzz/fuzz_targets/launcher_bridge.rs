//! What the launcher's page hands its commands (RD27 spec §2.3.4, RX3): the isolation hook's argument shapes for
//! every command in the table, and each command's own checks, which run again behind the hook.
#![no_main]

use dainty_launcher_kit::bridge::checks;
use dainty_launcher_kit::bridge::spec::{
    COMMANDS, DETECTED_INDEX_MAX, PUBLIC_HOST_MAX, SERVER_NAME_MAX, fits, fits_fields, is_server_id,
};
use dainty_launcher_kit::bridge::types::MAX_MULTIPLIER;
use libfuzzer_sys::fuzz_target;
use serde_json::{Value, json};

fn plain(text: &str) -> bool {
    !text.chars().any(checks::is_control)
}

fn lower_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn arguments(value: &Value) {
    for command in COMMANDS {
        let _ = fits_fields(command.args, value);
    }
    if let Ok(index) = checks::detected_index(value) {
        assert!(index <= DETECTED_INDEX_MAX as usize);
    }
    if let Ok(form) = checks::host_setup(value) {
        assert!(!form.name.is_empty() && fits(&form.name, SERVER_NAME_MAX) && plain(&form.name));
        if let Some(host) = &form.public_host {
            assert!(fits(host, PUBLIC_HOST_MAX));
            assert!(
                host.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
            );
        }
    }
    // Off, or finite values within the range and not both 1
    if let Ok(Some(set)) = checks::multipliers(value) {
        for f in [set.xp, set.rewards] {
            assert!(f.is_finite() && (1.0..=MAX_MULTIPLIER).contains(&f));
        }
        assert!(set.xp != 1.0 || set.rewards != 1.0);
    }
    if let Ok(Some(name)) = checks::invite_for(Some(value)) {
        assert!(!name.is_empty() && plain(&name));
    }
    let _ = checks::settings_patch(value);
    let _ = checks::history_filter(value);
    let _ = checks::revert_selection(value);
    let _ = checks::server_config(value);
    let _ = checks::admin_request(value);
    let _ = checks::remote_init(value);
    let _ = checks::remote_invite(value);
    let _ = checks::remote_enroll(value);
    let _ = checks::remote_exec(value);
}

fn text(text: &str) {
    if checks::server_id(text).is_ok() {
        assert!(is_server_id(text));
    }
    if let Ok(id) = checks::guarded_id(text) {
        assert!(lower_hex(id, 16));
    }
    if let Ok(realm) = checks::realm_id(text) {
        assert!(!realm.is_empty() && plain(realm));
    }
    if let Ok(node) = checks::remote_node(text) {
        assert!(!node.is_empty() && plain(node));
    }
    let _ = checks::invite_text(text);
    let _ = checks::username(text);
    let _ = checks::key_text(text);
    let _ = checks::game_path(text);
    let _ = checks::copy_text(text);
    let _ = checks::epic_code(text);
    let _ = checks::game_version(text);
    let _ = checks::revert_plan(text);
}

fuzz_target!(|data: &[u8]| {
    if let Ok(value) = serde_json::from_slice::<Value>(data) {
        arguments(&value);
        // The same value as a form's fields, which is how the page sends most arguments
        arguments(&json!({ "name": value, "mode": "public", "publicHost": value, "xp": value, "rewards": 2 }));
    }
    text(&String::from_utf8_lossy(data));
});
