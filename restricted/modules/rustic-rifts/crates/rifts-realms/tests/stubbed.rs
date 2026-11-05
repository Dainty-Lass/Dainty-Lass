//! The 1.14.7 module's view with rifts stubbed (RF2's verify): every server crate assembled into one `Rifts`, every
//! route answering 501 in the launcher envelope, as RU4 serves them.

use std::collections::BTreeSet;
use std::sync::Arc;

use rifts_seam::{
    AccountId, Caller, Reply, RequestInfo, Rifts, RiftsParts, RouteCall, RouteHandler, SeamError, poll_once,
};

fn stubbed() -> Rifts {
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
fn every_route_is_mounted_once_and_answers_501() {
    let rifts = stubbed();
    assert!(rifts.loaded());
    let mut seen = BTreeSet::new();
    for (route, handler) in rifts.routes() {
        assert!(
            seen.insert((route.method.as_str(), route.path)),
            "{} mounted twice",
            route.path
        );
        let call = RouteCall {
            route: *route,
            params: Vec::new(),
            query: String::new(),
            request: RequestInfo {
                request_id: "r1".to_owned(),
                ..RequestInfo::default()
            },
            caller: Caller::Launcher(AccountId("a".to_owned())),
            body: Vec::new(),
        };
        let Some(Err(SeamError::Unimplemented(unimplemented))) = poll_once(handler.handle(call)) else {
            panic!("{} did not answer as a stub", route.path);
        };
        assert_eq!(Reply::not_implemented(&unimplemented, "r1").status, 501);
    }
    // RF1's 68 less the 8 Launcher Guard and Guard manifest routes isles keeps
    assert_eq!(seen.len(), 60);
}

#[test]
fn hooks_answer_unimplemented_when_stubbed() {
    let rifts = stubbed();
    assert!(
        rifts
            .realms()
            .dedicated_hunts_disabled()
            .is_err_and(|error| error.is_unimplemented())
    );
    assert!(
        rifts
            .guard()
            .host_policy_refuses()
            .is_err_and(|error| error.is_unimplemented())
    );
    let touched = poll_once(rifts.entry().touch_presence(AccountId("a".to_owned()))).unwrap();
    assert!(touched.is_err_and(|error| error.is_unimplemented()));
}
