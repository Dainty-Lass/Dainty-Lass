//! The admin relay's node-link messages (docs/dainty-lass-plan.md DN11, DX2; docs/security/network.md):
//! an admin relay request or response from another node is read whole or refused, never panics, holds
//! nothing shaped like an account key (`RRK_`) or admin secret key (`RRA_`) in any field or body, stays
//! inside `MAX_ADMIN_WIRE`, and round-trips through `encode_*` / `decode_*`.
#![no_main]

use libfuzzer_sys::fuzz_target;
use dainty_network::home::protocol::looks_like_account_key;
use dainty_network::relay::{
    AdminRelayReply, MAX_ADMIN_WIRE, decode_request, decode_response, encode_request, encode_response,
    holds_secret_key, looks_like_admin_key,
};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    if looks_like_admin_key(&text) {
        assert!(looks_like_admin_key(&text.to_ascii_uppercase()));
        assert!(holds_secret_key(&serde_json::Value::String(text.to_string())));
    }
    if looks_like_account_key(&text) {
        assert!(holds_secret_key(&serde_json::Value::String(text.to_string())));
    }

    if let Ok(request) = decode_request(data) {
        assert!(data.len() <= MAX_ADMIN_WIRE);
        let action = &request.action;
        assert!(!looks_like_account_key(action.key.as_str()));
        assert!(!looks_like_admin_key(action.key.as_str()));
        assert!(!looks_like_account_key(action.actor_key.as_str()));
        assert!(!looks_like_admin_key(action.actor_key.as_str()));
        assert!(!looks_like_account_key(action.operator.as_str()));
        assert!(!looks_like_admin_key(action.operator.as_str()));
        if let Some(second) = &action.approved_by {
            assert!(!looks_like_account_key(second.as_str()));
            assert!(!looks_like_admin_key(second.as_str()));
        }
        assert!(!holds_secret_key(action.body.value()));
        let written = encode_request(&request).expect("writes back");
        assert_eq!(decode_request(&written).expect("reads back"), request);
    }

    if let Ok(response) = decode_response(data) {
        assert!(data.len() <= MAX_ADMIN_WIRE);
        if let AdminRelayReply::Applied { body } = &response.reply {
            assert!(!holds_secret_key(body.value()));
        }
        let written = encode_response(&response).expect("writes back");
        assert_eq!(decode_response(&written).expect("reads back"), response);
    }
});
