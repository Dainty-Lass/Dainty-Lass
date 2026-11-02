// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The route dispatcher's checks and answers with the services wired.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::kit::{CountingIds, NoLimit, accounts};
use common::targets;
use rifts_realms::capabilities::HostCapabilities;
use rifts_realms::control::ControlChannel;
use rifts_realms::directory::{DirectoryParts, RealmDirectory};
use rifts_realms::leases::{Hosting, HostingParts, HostingSettings};
use rifts_realms::lifecycle::RealmsRecovery;
use rifts_realms::ports::{Clock, DefaultOperations, NoGuardSessions, QuietLog, SystemClock};
use rifts_realms::realm::DEFAULT_REALM_LIMITS;
use rifts_realms::{ROUTES, RealmsRoutes, RealmsRoutesParts};
use rifts_seam::{AccountId, Caller, NoCore, Refusal, RequestInfo, RouteCall, RouteHandler, SeamError};

fn routes(db: &rifts_realms::db::RealmsDb, realms_enabled: bool) -> RealmsRoutes {
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let settings: BTreeMap<String, String> = [("P2P_HUNTS_ENABLED".to_owned(), "true".to_owned())].into();
    RealmsRoutes::new(RealmsRoutesParts {
        directory: Some(Arc::new(RealmDirectory::new(DirectoryParts {
            db: db.clone(),
            accounts: Arc::new(accounts()),
            events: None,
            clock: clock.clone(),
            limiter: Arc::new(NoLimit),
            ids: Arc::new(CountingIds::default()),
            limits: DEFAULT_REALM_LIMITS,
            log: Arc::new(QuietLog),
        }))),
        control: Some(Arc::new(ControlChannel::new(db.clone(), clock.clone()))),
        hosting: Some(Arc::new(Hosting::new(HostingParts {
            db: db.clone(),
            settings: HostingSettings::from_settings(&settings),
            clock: clock.clone(),
            operations: Arc::new(DefaultOperations::default()),
            guard: Arc::new(NoGuardSessions),
            recovery: Arc::new(RealmsRecovery::new(
                db.clone(),
                Arc::new(NoCore),
                clock.clone(),
                Arc::new(QuietLog),
            )),
            expected_artifacts: Arc::new(|| Err("no feed".to_owned())),
            log: Arc::new(QuietLog),
        }))),
        capabilities: Some(Arc::new(HostCapabilities::new(db.clone()))),
        clock,
        realms_enabled,
    })
}

fn call(path: &str, method: &str, caller: Caller, params: &[(&str, &str)], body: &str) -> RouteCall {
    let route = *ROUTES
        .iter()
        .find(|r| r.path == path && r.method.as_str() == method)
        .unwrap();
    RouteCall {
        route,
        params: params.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect(),
        query: String::new(),
        request: RequestInfo::default(),
        caller,
        body: body.as_bytes().to_vec(),
    }
}

fn launcher(account: &str) -> Caller {
    Caller::Launcher(AccountId(account.to_owned()))
}

fn refusal(result: Result<rifts_seam::Reply, SeamError>) -> (String, String) {
    match result {
        Err(SeamError::Refused(Refusal { code, message })) => (code, message),
        other => (format!("{other:?}"), String::new()),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn realms_routes_check_then_answer() {
    for target in targets() {
        let r = routes(&target.db, true);
        let body = r#"{"hostPublicKey":"AAAAAAAAAAAAAAAAAAAAAAAA"}"#;
        let created = r
            .handle(call("/launcher/v1/realms", "POST", launcher("host-1"), &[], body))
            .await
            .unwrap();
        assert_eq!(created.status, 200);
        assert!(
            created
                .body
                .0
                .starts_with(r#"{"realm":{"realmId":"realm-1","name":"hosty""#)
        );
        let anonymous = r
            .handle(call("/launcher/v1/realms/mine", "GET", Caller::Anonymous, &[], ""))
            .await;
        assert_eq!(
            refusal(anonymous),
            ("AUTH_UNAUTHORIZED".into(), "Sign in required.".into())
        );
        let bad_json = r
            .handle(call("/launcher/v1/realms/join", "POST", launcher("x"), &[], "{"))
            .await;
        assert_eq!(refusal(bad_json).1, "The request body is not valid JSON.");
        let bad_account = r
            .handle(call(
                "/launcher/v1/realms/mine/members/:accountId/kick",
                "POST",
                launcher("host-1"),
                &[("accountId", "a b")],
                "",
            ))
            .await;
        assert_eq!(refusal(bad_account).1, "accountId is malformed.");
        let unknown = r
            .handle(call("/launcher/v1/realms/*", "ALL", launcher("x"), &[], ""))
            .await;
        assert_eq!(
            refusal(unknown),
            ("REALM_ROUTE_NOT_FOUND".into(), "Unknown realms route.".into())
        );
        let off = routes(&target.db, false)
            .handle(call("/launcher/v1/realms/mine", "GET", launcher("x"), &[], ""))
            .await;
        assert_eq!(refusal(off).0, "REALMS_DISABLED");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_routes_check_capabilities_first() {
    for target in targets() {
        let r = routes(&target.db, true);
        let ready = r
            .handle(call(
                "/rustic/v1/hunts/:sessionId/host-ready",
                "POST",
                Caller::Anonymous,
                &[("sessionId", "x")],
                "not json",
            ))
            .await;
        assert_eq!(refusal(ready).0, "P2P_CAPABILITY_INVALID");
        let status = r
            .handle(call(
                "/rustic/v1/hunts/:sessionId/status",
                "GET",
                Caller::Anonymous,
                &[("sessionId", "x")],
                "",
            ))
            .await;
        assert_eq!(refusal(status).0, "AUTH_UNAUTHORIZED");
        let missing = r
            .handle(call(
                "/rustic/v1/hunts/:sessionId/status",
                "GET",
                launcher("a"),
                &[("sessionId", "x")],
                "",
            ))
            .await;
        assert_eq!(
            refusal(missing),
            ("P2P_SESSION_NOT_FOUND".into(), "Hunt session not found.".into())
        );
        let admin = r
            .handle(call(
                "/admin/v1/operations/p2p-emergency-stop",
                "PATCH",
                launcher("a"),
                &[],
                "{}",
            ))
            .await;
        assert_eq!(refusal(admin).1, "Admin authentication required.");
        let stop = r
            .handle(call(
                "/admin/v1/operations/p2p-emergency-stop",
                "PATCH",
                Caller::Admin { key_id: "k1".into() },
                &[],
                r#"{"p2pEmergencyStop":true,"reason":"maintenance window"}"#,
            ))
            .await
            .unwrap();
        assert!(
            stop.body
                .0
                .starts_with(r#"{"policy":{"version":2,"p2pEmergencyStop":true,"coopTransport":"ICE""#)
        );
        let manifest = r
            .handle(call(
                "/api/matchmaker/resolve-hunt-launch-manifest",
                "POST",
                Caller::Anonymous,
                &[],
                r#"{"huntDefinitionId":"Hunt_A","expectedPlayers":["a","b",3]}"#,
            ))
            .await
            .unwrap();
        assert!(
            manifest.body.0.contains(r#""expectedPlayerString":"a,b""#),
            "{}",
            manifest.body.0
        );
        let p2p = r
            .handle(call(
                "/launcher/v1/p2p/host-presence",
                "POST",
                launcher("a"),
                &[],
                "[1]",
            ))
            .await;
        assert_eq!(refusal(p2p).1, "The request body must be a JSON object.");
        let hubs = r
            .handle(call("/launcher/v1/realm-hosting/status", "GET", launcher("a"), &[], ""))
            .await
            .unwrap();
        assert_eq!(
            hubs.body.0,
            r#"{"enabled":false,"realmId":null,"realmName":null,"hosting":null,"ramsgate":null,"training":null,"activeHunts":[]}"#
        );
    }
}
