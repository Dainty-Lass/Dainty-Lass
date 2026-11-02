// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Every rifts crate stubbed: the module is loaded, each of the 60 routes is mounted once (RF1's 68 less the 8
//! Launcher Guard and manifest routes isles keeps) and answers 501, and the hooks answer unimplemented.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashSet;
use std::sync::Arc;

use rifts_seam::{
    AccountId, Caller, Component, Reply, RequestInfo, Rifts, RiftsParts, RouteCall, RouteHandler, SeamError, poll_once,
};

fn rifts() -> Rifts {
    let routes: Vec<Arc<dyn RouteHandler>> = vec![
        Arc::new(rifts_admission::stub_routes()),
        Arc::new(rifts_relay::stub_routes()),
        Arc::new(rifts_realms::stub_routes()),
        Arc::new(rifts_guard::stub_routes()),
    ];
    Rifts::new(RiftsParts {
        realms: Arc::new(rifts_realms::Stub),
        entry: Arc::new(rifts_realms::Stub),
        guard: Arc::new(rifts_guard::Stub),
        routes,
    })
}

#[test]
fn sixty_routes_each_mounted_once_and_answering_501() {
    let rifts = rifts();
    assert!(rifts.loaded());
    let mounted: Vec<_> = rifts.routes().collect();
    assert_eq!(mounted.len(), 60);
    let unique: HashSet<(&str, &str)> = mounted.iter().map(|(r, _)| (r.method.as_str(), r.path)).collect();
    assert_eq!(unique.len(), 60);
    for (route, handler) in mounted {
        let call = RouteCall {
            route: *route,
            params: Vec::new(),
            query: String::new(),
            request: RequestInfo::default(),
            caller: Caller::Anonymous,
            body: Vec::new(),
        };
        let Some(Err(SeamError::Unimplemented(missing))) = poll_once(handler.handle(call)) else {
            panic!("{} answered", route.path);
        };
        assert_eq!(missing.operation, route.handler);
        let reply = Reply::not_implemented(&missing, "req-1");
        assert_eq!(reply.status, 501);
        assert!(
            reply
                .body
                .0
                .starts_with(r#"{"error":{"code":"NOT_IMPLEMENTED","message":"rifts-"#)
        );
    }
    let realms: Vec<_> = rifts_realms::ROUTES.iter().map(|r| r.handler).collect();
    assert_eq!(realms.len(), 46);
    let reply = Reply::not_implemented(
        &rifts_seam::Unimplemented::new(Component::Realms, "realms.join"),
        "r\"1",
    );
    assert_eq!(
        reply.body.0,
        r#"{"error":{"code":"NOT_IMPLEMENTED","message":"rifts-realms: realms.join is not implemented yet","requestId":"r\"1"}}"#
    );
}

#[test]
fn the_hooks_answer_unimplemented() {
    let rifts = rifts();
    let realms = poll_once(rifts.realms().authenticate(RequestInfo::default())).unwrap();
    assert_eq!(realms, Err(SeamError::unimplemented(Component::Realms, "authenticate")));
    let touch = poll_once(rifts.entry().touch_presence(AccountId("a".into()))).unwrap();
    assert_eq!(
        touch,
        Err(SeamError::unimplemented(Component::Realms, "touch_presence"))
    );
    let guard = poll_once(rifts.guard().rate_limit_bucket(RequestInfo::default())).unwrap();
    assert!(matches!(guard, Err(SeamError::Unimplemented(u)) if u.component == Component::Guard));
}
