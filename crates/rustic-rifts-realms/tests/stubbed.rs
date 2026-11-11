// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Every rifts crate stubbed: the module is loaded, each of the 61 routes is mounted once (RF1's 68 less the 8
//! Launcher Guard and manifest routes isles keeps, plus the admission key's) and answers 501, and the hooks answer
//! unimplemented.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::collections::HashSet;
use std::sync::Arc;

use rustic_rifts_seam::{
    AccountId, Caller, Component, Reply, RequestInfo, Rifts, RiftsParts, RouteCall, RouteHandler, SeamError,
    Unimplemented, poll_once,
};

/// The four crates' stub route tables over the two stub hooks.
fn stubbed() -> Rifts {
    let routes: Vec<Arc<dyn RouteHandler>> = vec![
        Arc::new(rustic_rifts_admission::stub_routes()),
        Arc::new(rustic_rifts_relay::stub_routes()),
        Arc::new(rustic_rifts_realms::stub_routes()),
        Arc::new(rustic_rifts_guard::stub_routes()),
    ];
    let (realms, entry) = (Arc::new(rustic_rifts_realms::Stub), Arc::new(rustic_rifts_realms::Stub));
    let guard = Arc::new(rustic_rifts_guard::Stub);
    Rifts::new(RiftsParts {
        routes,
        guard,
        entry,
        realms,
    })
}

/// A body-less anonymous call of `route`.
fn bare_call(route: rustic_rifts_seam::Route) -> RouteCall {
    RouteCall {
        body: Vec::new(),
        caller: Caller::Anonymous,
        request: RequestInfo::default(),
        query: String::new(),
        params: Vec::new(),
        route,
    }
}

#[test]
fn sixty_one_routes_each_mounted_once_and_answering_501() {
    let rifts = stubbed();
    assert!(rifts.loaded());
    let mounted: Vec<_> = rifts.routes().collect();
    let distinct: HashSet<_> = mounted.iter().map(|(r, _)| (r.method.as_str(), r.path)).collect();
    assert_eq!((mounted.len(), distinct.len()), (61, 61));
    for (route, handler) in mounted {
        let answer = poll_once(handler.handle(bare_call(*route)));
        let Some(Err(SeamError::Unimplemented(missing))) = answer else {
            panic!("{} answered", route.path);
        };
        assert_eq!(missing.operation, route.handler);
        let reply = Reply::not_implemented(&missing, "req-1");
        assert_eq!(reply.status, 501);
        let prefix = r#"{"error":{"code":"NOT_IMPLEMENTED","message":"rifts-"#;
        assert!(reply.body.0.starts_with(prefix), "{}", reply.body.0);
    }
    assert_eq!(rustic_rifts_realms::ROUTES.len(), 46);
    let join = Unimplemented::new(Component::Realms, "realms.join");
    let quoted = Reply::not_implemented(&join, "r\"1");
    let expected = concat!(
        r#"{"error":{"code":"NOT_IMPLEMENTED","#,
        r#""message":"rifts-realms: realms.join is not implemented yet","requestId":"r\"1"}}"#,
    );
    assert_eq!(quoted.body.0, expected);
}

#[test]
fn the_hooks_answer_unimplemented() {
    let rifts = stubbed();
    let missing = |operation| SeamError::unimplemented(Component::Realms, operation);
    let authenticated = poll_once(rifts.realms().authenticate(RequestInfo::default())).unwrap();
    assert_eq!(authenticated.unwrap_err(), missing("authenticate"));
    let touched = poll_once(rifts.entry().touch_presence(AccountId("a".into()))).unwrap();
    assert_eq!(touched.unwrap_err(), missing("touch_presence"));
    let bucket = poll_once(rifts.guard().rate_limit_bucket(RequestInfo::default())).unwrap();
    assert!(matches!(bucket, Err(SeamError::Unimplemented(u)) if u.component == Component::Guard));
}
