// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The gateway's route policy (DR X9, RD27): any method, target and headers give a decision without panicking,
//! and nothing it lets through can reach the host's own routes.
#![no_main]

use gateway::policy::{
    Decision, FeedKind, HOST_ONLY_HEADERS, HOST_ONLY_PREFIXES, PUBLIC_LAUNCHER_API, Upstream, canonical_path,
    check_path, classify, split_target, under, unwrap_origin,
};
use http::{HeaderMap, HeaderName, HeaderValue};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: (String, String, Vec<(String, Vec<u8>)>, bool)| {
    let (method, target, raw_headers, upgrade) = input;
    let mut headers = HeaderMap::new();
    for (name, value) in raw_headers {
        if let (Ok(name), Ok(value)) = (HeaderName::from_bytes(name.as_bytes()), HeaderValue::from_bytes(&value)) {
            headers.append(name, value);
        }
    }
    let decision = classify(&method, &target, &headers, upgrade);
    let Some((path, _)) = split_target(&target) else {
        assert!(matches!(decision, Decision::Reject { status: 400, .. }));
        return;
    };
    let Decision::Proxy { upstream, feed, .. } = decision else {
        return;
    };
    assert!(check_path(path));
    let inner = unwrap_origin(path).expect("a proxied path unwraps");
    assert!(inner.path.starts_with('/'));
    let canonical = canonical_path(&inner.path).expect("a proxied path decodes");
    assert!(canonical.starts_with('/'));
    assert!(HOST_ONLY_HEADERS.iter().all(|h| !headers.contains_key(*h)));
    assert!(HOST_ONLY_PREFIXES.iter().all(|p| !under(&canonical, p)));
    let method = method.to_ascii_uppercase();
    if under(&canonical, "/reforged") {
        let name = canonical.strip_prefix("/dainty/api/").expect("only the public API");
        let (_, methods) = PUBLIC_LAUNCHER_API
            .iter()
            .find(|(n, _)| *n == name)
            .expect("a public route");
        assert!(methods.contains(&method.as_str()) && !upgrade);
    }
    if upstream == Upstream::Content {
        assert!(inner.origin.is_none() && under(&canonical, "/content"));
    }
    match feed {
        Some(FeedKind::Heartbeat) => assert!(method == "POST" && canonical == "/heartbeat"),
        Some(FeedKind::Token) => assert!(method == "POST" && canonical == "/account/api/oauth/token"),
        None => {}
    }
});
