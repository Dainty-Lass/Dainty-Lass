//! What the launcher reads from a server's host API (RD27 spec §2.6.3, RX3): the status, probe, register, user
//! info, runtime manifest, news and branding replies, and a branding image's first bytes. The first two bytes
//! pick the status; every reply is also read as a 200.
#![no_main]

use bytes::Bytes;
use launcher_kit::net::hostapi::parse;
use libfuzzer_sys::fuzz_target;
use protocol::account::is_plausible_account_key;

fn reply(status: u16, body: &[u8]) {
    let _ = parse::status_reply(status, body);
    let _ = parse::probe_reply(status, body);
    let _ = parse::runtime_manifest_reply(status, body);
    // A key only from a 200, and only a plausible one
    if let Ok(key) = parse::register_reply(status, body) {
        assert!(status == 200 && is_plausible_account_key(&key));
    }
    if let Ok(info) = parse::user_info_reply(status, body) {
        assert!(status == 200 && !info.user_id.is_empty() && info.user_id.len() <= 128);
        assert!(
            info.user_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        );
        assert!(info.username.encode_utf16().count() <= 32);
    }
    if let Some(items) = parse::news_reply(status, body) {
        assert!(items.len() <= parse::MAX_NEWS_ITEMS);
        for item in &items {
            assert!(!item.title.is_empty() && !item.body.is_empty() && !item.body.contains("\n\n\n"));
        }
    }
    // Only paths under the branding route reach the image request
    let branding = parse::branding_reply(status, body);
    assert!(branding.backgrounds.len() <= parse::MAX_BACKGROUNDS);
    for background in &branding.backgrounds {
        assert!(parse::is_branding_path(&background.url) && !background.url.contains(".."));
    }
    if let Some(accent) = &branding.accent {
        assert!(accent.len() == 7 && accent.starts_with('#'));
        assert!(accent[1..].bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
    }
    if let Some(image) = parse::branding_image_reply(status, Bytes::copy_from_slice(body)) {
        assert!(status == 200 && parse::sniff_image(&image.bytes) == Some(image.kind));
    }
}

fuzz_target!(|data: &[u8]| {
    if let [hi, lo, body @ ..] = data {
        reply(u16::from_be_bytes([*hi, *lo]) % 600, body);
    }
    reply(200, data);
    let text = String::from_utf8_lossy(data);
    if let Some(clean) = parse::clean_news_body(&text) {
        // Cut to 4000 UTF-16 units, plus the ellipsis
        assert!(!clean.is_empty() && !clean.contains('\r') && clean.encode_utf16().count() <= 4001);
    }
    let _ = parse::is_branding_path(&text);
});
