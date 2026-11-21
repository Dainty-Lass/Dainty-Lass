//! The launcher's OAuth sign-in (docs/security/oauth.md, RX3): a request head to the loopback redirect listener,
//! `OAuthStart`'s and `OAuthFinish`'s replies, and the authorize URL check. The first two bytes pick the status.
#![no_main]

use dainty_launcher_kit::net::hostapi::parse;
use dainty_launcher_kit::net::oauth_redirect::{Callback, parse_request};
use dainty_launcher_kit::platform::links::check_oauth;
use dainty_protocol::account::is_plausible_account_key;
use libfuzzer_sys::fuzz_target;

const STATE: &str = "00112233445566778899aabbccddeeff";

fuzz_target!(|data: &[u8]| {
    // A code only for our state, and only a bounded printable one
    if let Callback::Code(code) = parse_request(data, STATE) {
        assert!(!code.is_empty() && code.len() <= 2048 && code.bytes().all(|b| b.is_ascii_graphic()));
        let text = String::from_utf8_lossy(data);
        assert!(text.starts_with("GET /oauth/callback?"));
    }
    assert!(!matches!(parse_request(data, ""), Callback::Code(_)));
    if let [hi, lo, body @ ..] = data {
        let status = u16::from_be_bytes([*hi, *lo]) % 600;
        for status in [status, 200] {
            if let Ok(start) = parse::oauth_start_reply(status, body) {
                assert_eq!(status, 200);
                assert_eq!(start.ticket.len(), 64);
                if let Some(url) = check_oauth(&start.authorize_url, &start.provider) {
                    assert_eq!(url.scheme(), "https");
                    assert!(url.username().is_empty() && url.password().is_none());
                    if start.provider == "discord" {
                        assert_eq!(url.host_str(), Some("discord.com"));
                    }
                }
            }
            if let Ok((key, name)) = parse::oauth_finish_reply(status, body) {
                assert!(status == 200 && is_plausible_account_key(&key) && !name.is_empty());
            }
        }
    }
    if let Ok(text) = std::str::from_utf8(data) {
        if let Some(url) = check_oauth(text, "discord") {
            assert!(url.scheme() == "https" && url.host_str() == Some("discord.com"));
        }
    }
});
