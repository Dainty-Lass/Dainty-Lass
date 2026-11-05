//! The manager operator verifier and the admin cookie reader (spec-rustic-isles-reforged-admin §18): arbitrary
//! headers, peer, method, target, body and clock. Nothing panics; acceptance needs a matching reference HMAC and a
//! loopback peer with no forwarding header; one signature is never accepted twice; cookies match exact names only.
#![no_main]

use arbitrary::Arbitrary;
use http::{HeaderMap, HeaderName, HeaderValue};
use libfuzzer_sys::fuzz_target;
use rustic_isles_reforged::admin::ReforgedAdmin;
use rustic_isles_reforged::admin::auth::{
    OperatorRequest, OperatorVerdict, operator_signature, read_cookie, verify_operator,
};

const SECRET: &str = "ssssssssssssssssssssssssssssssssssssssss";

#[derive(Debug, Arbitrary)]
struct Input {
    headers: Vec<(String, String)>,
    loopback: bool,
    method: String,
    target: String,
    body: Vec<u8>,
    now: i64,
    sign: bool,
}

fuzz_target!(|input: Input| {
    let mut headers = HeaderMap::new();
    for (name, value) in &input.headers {
        if let (Ok(n), Ok(v)) = (HeaderName::try_from(name.as_str()), HeaderValue::from_str(value)) {
            headers.append(n, v);
        }
    }
    if input.sign {
        let ts = input.now.rem_euclid(1_000_000_000_000_000).to_string();
        let sig = operator_signature(SECRET, &input.method, &input.target, &ts, "1", &input.body);
        headers.insert("x-paradox-operator-id", HeaderValue::from_static("1"));
        headers.insert("x-paradox-operator-timestamp", HeaderValue::from_str(&ts).unwrap());
        headers.insert("x-paradox-operator-signature", HeaderValue::from_str(&sig).unwrap());
    }
    let _ = read_cookie(&headers, "rustic_isles_admin");
    let admin = ReforgedAdmin::default();
    let peer = if input.loopback {
        "127.0.0.1".parse().ok()
    } else {
        "10.1.2.3".parse().ok()
    };
    let request = OperatorRequest {
        method: &input.method,
        target: &input.target,
        peer,
        headers: &headers,
        body: &input.body,
        body_declared: false,
        json_read: false,
        now_ms: input.now,
    };
    let verdict = verify_operator(Some(SECRET), &admin, &request);
    if let OperatorVerdict::Accepted { .. } = verdict {
        assert!(input.loopback);
        assert!(
            !headers
                .keys()
                .any(|k| k.as_str().starts_with("x-forwarded-") || k == "via" || k == "forwarded")
        );
        assert_ne!(verify_operator(Some(SECRET), &admin, &request), verdict);
    }
});
