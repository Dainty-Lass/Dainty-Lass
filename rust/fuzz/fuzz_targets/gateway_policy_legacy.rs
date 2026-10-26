//! The gateway's request policy in front of a Dauntless Revived 1.4.4 server (`GATEWAY_GAME=legacy`;
//! docs/security/gateway.md): a method, target and headers from the network must give a decision,
//! never a panic, and nothing proxied may reach a host-only part of DR's metagame: no host-only
//! header, no host-only prefix (/internal holds the metagame's own), nothing under /reforged but its
//! six public routes, whatever the spelling.
#![no_main]

use arbitrary::Arbitrary;
use gateway::policy::legacy::{PUBLIC_LEGACY_API, classify};
use gateway::policy::{
    Decision, HOST_ONLY_HEADERS, HOST_ONLY_PREFIXES, Upstream, canonical_path, check_path, split_target,
};
use http::{HeaderMap, HeaderName, HeaderValue};
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
struct Request<'a> {
    method: &'a str,
    target: &'a str,
    headers: Vec<(&'a [u8], &'a [u8])>,
    upgrade: bool,
}

fn under(path: &str, prefix: &str) -> bool {
    path == prefix || path.starts_with(&format!("{prefix}/"))
}

fuzz_target!(|request: Request<'_>| {
    let mut headers = HeaderMap::new();
    for (name, value) in request.headers.iter().take(32) {
        if let (Ok(name), Ok(value)) = (HeaderName::from_bytes(name), HeaderValue::from_bytes(value)) {
            headers.append(name, value);
        }
    }
    let decision = classify(request.method, request.target, &headers, request.upgrade);
    let Some((path, _query)) = split_target(request.target) else {
        assert!(matches!(decision, Decision::Reject { status: 400, .. }));
        return;
    };
    let Decision::Proxy { upstream, .. } = decision else {
        return;
    };
    assert!(check_path(path));
    let canonical = canonical_path(path).expect("a proxied path decodes");
    for name in HOST_ONLY_HEADERS {
        assert!(!headers.contains_key(name), "{name} went through");
    }
    for prefix in HOST_ONLY_PREFIXES {
        assert!(!under(&canonical, prefix), "{canonical} went through");
    }
    if under(&canonical, "/reforged") {
        let name = canonical.strip_prefix("/reforged/api/").unwrap_or("");
        let verb = request.method.to_uppercase();
        assert!(
            PUBLIC_LEGACY_API
                .iter()
                .any(|(route, methods)| *route == name && methods.contains(&verb.as_str())),
            "{verb} {canonical} went through"
        );
        assert_eq!(upstream, Upstream::Metagame);
    }
    // Reborn's upstream never appears in front of the metagame
    assert_ne!(upstream, Upstream::Backend);
    if upstream == Upstream::Content {
        assert!(under(&canonical, "/content"));
    }
});
