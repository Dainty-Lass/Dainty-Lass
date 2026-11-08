//! The directory's node-link messages (docs/dainty-lass-plan.md DN4, DX2; docs/security/network.md): a
//! request or answer from another node is read whole or refused, never panics, holds only usernames the
//! protocol's rule accepts and account ids of one spelling, and writes back as the same message.
#![no_main]

use libfuzzer_sys::fuzz_target;
use dainty_network::Username;
use dainty_network::link::MAX_MESSAGE;
use dainty_network::messages::{DirectoryCall, DirectoryReply, decode_request, decode_response, encode};
use dainty_protocol::account::is_valid_username;

fuzz_target!(|data: &[u8]| {
    if let Ok(request) = decode_request(data) {
        assert!(data.len() <= MAX_MESSAGE);
        if let DirectoryCall::Pull { last: Some(last), .. } = &request.call
            && let Some(name) = last.change.username()
        {
            assert!(is_valid_username(name.as_str()));
        }
        let again = decode_request(&encode(&request).expect("writes back")).expect("reads back");
        assert_eq!(again, request);
    }
    if let Ok(response) = decode_response(data) {
        if let DirectoryReply::Changes { entries, .. } = &response.reply {
            for entry in entries {
                let (Some(name), Some(account)) = (entry.change.username(), entry.change.account()) else {
                    continue;
                };
                assert!(is_valid_username(name.as_str()));
                let account = account.to_string();
                assert_eq!(
                    account
                        .parse::<dainty_module_api::GlobalId>()
                        .expect("one spelling")
                        .to_string(),
                    account
                );
            }
        }
        let again = decode_response(&encode(&response).expect("writes back")).expect("reads back");
        assert_eq!(again, response);
    }
    let text = String::from_utf8_lossy(data);
    if let Ok(name) = Username::new(text.as_ref()) {
        assert_eq!(name.as_str(), text);
        assert!(name.key().eq_ignore_ascii_case(&text));
    }
});
