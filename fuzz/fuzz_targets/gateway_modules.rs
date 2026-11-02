//! The gateway on the module registry (DF4a; docs/security/gateway.md): a module's registration arrives
//! from its own process, so routes of any shape must attach or be refused, never panic, and a request from
//! the network must never reach a module through one of its host-only routes, the node-link group or a
//! host-only prefix, nor reach anything off loopback.
#![no_main]

use std::sync::Arc;

use arbitrary::Arbitrary;
use gateway::modules::{ManifestDefaults, ModuleRoutes, NODE_LINKS};
use gateway::policy::{HOST_ONLY_PREFIXES, canonical_path, split_target, unwrap_origin};
use http::{HeaderMap, HeaderName, HeaderValue};
use libfuzzer_sys::fuzz_target;
use module_api::registrar::RouteSpec;
use module_api::stub::StubModule;
use module_api::{CorePlugin, GameModule, Registration};
use protocol::invite::reforged_version;

#[derive(Arbitrary, Debug)]
struct Input<'a> {
    routes: Vec<(&'a str, bool, bool)>,
    method: &'a str,
    target: &'a str,
    headers: Vec<(&'a [u8], &'a [u8])>,
    upgrade: bool,
}

fn covers(path: &str, prefix: &str) -> bool {
    prefix == "/" || path == prefix || path.starts_with(&format!("{prefix}/"))
}

fuzz_target!(|input: Input<'_>| {
    let manifest = StubModule::new().manifest();
    let registration = Registration {
        routes: input
            .routes
            .iter()
            .take(8)
            .map(|(prefix, host_only, websocket)| RouteSpec {
                prefix: (*prefix).to_owned(),
                port: "http".into(),
                host_only: *host_only,
                websocket: *websocket,
            })
            .collect(),
        ..Registration::default()
    };
    let routes = ModuleRoutes::new(Arc::new(ManifestDefaults));
    if routes.attach(&manifest, &registration).is_err() {
        return;
    }
    let mut headers = HeaderMap::new();
    for (name, value) in input.headers.iter().take(32) {
        if let (Ok(name), Ok(value)) = (HeaderName::from_bytes(name), HeaderValue::from_bytes(value)) {
            headers.append(name, value);
        }
    }
    let routed = routes.route(
        "0.0.1",
        &reforged_version(),
        input.method,
        input.target,
        &headers,
        input.upgrade,
    );
    let Some(endpoint) = routed.endpoint else {
        return;
    };
    assert_eq!((endpoint.host.as_str(), endpoint.port), ("127.0.0.1", 65000));
    let (path, _) = split_target(input.target).expect("a routed target splits");
    let unwrapped = unwrap_origin(path).expect("a routed path unwraps");
    let inner = canonical_path(&unwrapped.path).expect("a routed path decodes");
    let raw = canonical_path(path).expect("a routed path decodes");
    for route in registration.routes.iter().filter(|route| route.host_only) {
        let prefix = route.prefix.to_ascii_lowercase();
        assert!(!covers(&inner, &prefix), "{inner} reached host-only {prefix}");
    }
    assert!(
        !covers(&inner, NODE_LINKS.prefix) && !covers(&raw, NODE_LINKS.prefix),
        "{raw} reached a node link"
    );
    for prefix in HOST_ONLY_PREFIXES {
        assert!(!covers(&inner, prefix), "{inner} went through");
    }
});
