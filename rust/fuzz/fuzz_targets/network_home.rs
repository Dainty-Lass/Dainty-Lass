//! The home API's node-link messages (docs/dainty-lass-plan.md DN5, DX2; docs/security/network.md): a request or
//! answer from another node is read whole or refused, never panics, holds nothing shaped like an account key in
//! any free-form field, stays inside its bounds and writes back as the same message.
#![no_main]

use libfuzzer_sys::fuzz_target;
use network::home::protocol::{
    HomeReply, HomeRequestCall, MAX_BODY, decode_request, decode_response, encode_request, encode_response,
    holds_account_key, looks_like_account_key,
};
use network::link::MAX_MESSAGE;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    // The scanner itself: whatever it flags is flagged however the text is cut or cased
    if looks_like_account_key(&text) {
        assert!(looks_like_account_key(&text.to_ascii_uppercase()));
        assert!(holds_account_key(&serde_json::Value::String(text.to_string())));
    }

    if let Ok(request) = decode_request(data) {
        assert!(data.len() <= MAX_MESSAGE);
        if let HomeRequestCall::Run(call) = &request.call {
            assert!(!holds_account_key(call.body.value()));
            assert!(serde_json::to_vec(call.body.value()).expect("a value writes").len() <= MAX_BODY);
            assert!(!looks_like_account_key(call.key.as_str()));
            let actor = serde_json::to_string(call.actor.actor()).expect("an actor writes");
            assert!(!looks_like_account_key(&actor));
            assert!(call.actor.actor().is_valid());
        }
        let written = encode_request(&request).expect("writes back");
        assert_eq!(decode_request(&written).expect("reads back"), request);
    }
    if let Ok(response) = decode_response(data) {
        assert!(data.len() <= MAX_MESSAGE);
        match &response.reply {
            HomeReply::Done { body } => assert!(!holds_account_key(body.value())),
            HomeReply::PublicKeys { keys, .. } => assert!((1..=4).contains(&keys.len())),
            _ => {}
        }
        let written = encode_response(&response).expect("writes back");
        assert_eq!(decode_response(&written).expect("reads back"), response);
    }
});
