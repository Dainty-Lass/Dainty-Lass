//! The headers the services forward through dainty-proxy (docs/security/gateway.md): what a client sent, with
//! the hop-by-hop ones and the names its Connection headers list. None of it may panic; nothing that applies
//! to one connection, or that the caller drops, is forwarded, and everything else is, unchanged.
#![no_main]

use dainty_proxy::{connection_tokens, end_to_end, is_hop_by_hop};
use http::{HeaderMap, HeaderName, HeaderValue};
use libfuzzer_sys::arbitrary::{self, Arbitrary};
use libfuzzer_sys::fuzz_target;

#[derive(Debug, Arbitrary)]
struct Input<'a> {
    headers: Vec<(&'a str, &'a [u8])>,
    drop: &'a str,
}

fuzz_target!(|input: Input<'_>| {
    let mut headers = HeaderMap::new();
    for (name, value) in &input.headers {
        if let (Ok(name), Ok(value)) = (HeaderName::from_bytes(name.as_bytes()), HeaderValue::from_bytes(value)) {
            headers.append(name, value);
        }
    }
    let drop = input.drop.to_ascii_lowercase();
    let tokens = connection_tokens(&headers);
    assert!(tokens.iter().all(|t| !t.is_empty() && *t == t.to_ascii_lowercase()));
    let out = end_to_end(&headers, |name| name == drop);
    for name in headers.keys() {
        let n = name.as_str();
        let sent: Vec<_> = headers.get_all(name).iter().collect();
        let kept: Vec<_> = out.get_all(name).iter().collect();
        if is_hop_by_hop(n) || tokens.contains(n) || n == drop {
            assert!(kept.is_empty(), "{n} was forwarded");
        } else {
            assert_eq!(kept, sent, "{n} changed");
        }
    }
    assert!(out.keys().all(|name| headers.contains_key(name)));
});
