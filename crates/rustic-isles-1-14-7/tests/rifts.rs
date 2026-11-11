// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The rifts composition (spec §6, §12.2): off, with a SQLite realms database, and without one.

#![cfg(feature = "realms")]
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::sync::Arc;

use rustic_isles::Settings;
use rustic_isles::routes::{Service, When, all};
use rustic_isles_1_14_7::rifts_with_core;
use rustic_rifts_seam::{
    ADMISSION_KEY_PATH, AccountId, BoxFuture, Caller, Candidate, GuardMode, IslesCore, Json, NoCore, Party, PartyId,
    Refusal, RequestInfo, Rifts, RouteCall, SeamError, SessionId,
};

type Fut<'a, T> = BoxFuture<'a, Result<T, SeamError>>;

/// Every account is eligible and named `SlayerHost`.
struct Named;

impl IslesCore for Named {
    fn display_name(&self, _: AccountId) -> Fut<'_, Option<String>> {
        Box::pin(async { Ok(Some("SlayerHost".to_owned())) })
    }
    fn find_by_display_name(&self, _: String) -> Fut<'_, Option<AccountId>> {
        Box::pin(async { Ok(None) })
    }
    fn launcher_account_eligible(&self, _: AccountId) -> Fut<'_, bool> {
        Box::pin(async { Ok(true) })
    }
    fn party(&self, party: PartyId) -> Fut<'_, Option<Party>> {
        NoCore.party(party)
    }
    fn get_or_create_party(&self, account: AccountId) -> Fut<'_, Party> {
        NoCore.get_or_create_party(account)
    }
    fn dedicated_servers_configured(&self) -> bool {
        false
    }
    fn set_candidate(&self, account: AccountId, candidate: Candidate) -> Result<(), SeamError> {
        NoCore.set_candidate(account, candidate)
    }
    fn cancel_travel(&self, hunt: &str, accounts: &[AccountId], reason: &str) -> Result<bool, SeamError> {
        NoCore.cancel_travel(hunt, accounts, reason)
    }
    fn touch_game_session(&self, account: AccountId) -> Fut<'_, ()> {
        NoCore.touch_game_session(account)
    }
    fn record_hunt_activity(&self, session: SessionId, activity: Json) -> Fut<'_, ()> {
        NoCore.record_hunt_activity(session, activity)
    }
    fn guard_mode(&self) -> GuardMode {
        GuardMode::Observe
    }
    fn healthy_guard_session(&self, account: AccountId) -> Fut<'_, Option<Json>> {
        NoCore.healthy_guard_session(account)
    }
    fn published_guard_artifacts(&self) -> Fut<'_, Json> {
        NoCore.published_guard_artifacts()
    }
    fn operations_policy(&self) -> Fut<'_, Json> {
        Box::pin(async { Ok(Json::empty_object()) })
    }
    fn verify_operations_step_up(&self, _: String, _: String) -> Fut<'_, bool> {
        Box::pin(async { Ok(false) })
    }
}

async fn call(
    rifts: &Rifts,
    method: &str,
    path: &str,
    caller: Caller,
    params: &[(&str, &str)],
    body: &str,
) -> Result<rustic_rifts_seam::Reply, SeamError> {
    let (route, handler) = rifts
        .routes()
        .find(|(route, _)| route.path == path && route.method.as_str() == method)
        .unwrap_or_else(|| panic!("no rifts route {method} {path}"));
    handler
        .handle(RouteCall {
            route: *route,
            params: params.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect(),
            query: String::new(),
            request: RequestInfo::default(),
            caller,
            body: body.as_bytes().to_vec(),
        })
        .await
}

fn code(answer: Result<rustic_rifts_seam::Reply, SeamError>) -> String {
    match answer {
        Err(SeamError::Refused(Refusal { code, .. })) => code,
        other => format!("{other:?}"),
    }
}

fn host() -> Caller {
    Caller::Launcher(AccountId("host-1".to_owned()))
}

#[test]
fn off_is_dedicated_only() {
    let settings = Settings::from_pairs([("RUSTIC_RIFTS_P2P_MODULE", "off")]);
    let rifts = rifts_with_core(&settings, Arc::new(Named));
    assert!(!rifts.loaded());
    assert!(rifts.routes().next().is_none());
}

/// Every rifts row of core's table has a rifts route of the same method and path (D3 landed in core).
fn assert_covers(rifts: &Rifts) {
    for row in all().iter().filter(|r| r.when == When::Rifts) {
        let found = rifts.routes().any(|(route, _)| {
            route.path == row.path && (route.method.as_str() == row.method.as_str() || route.method.as_str() == "ALL")
        });
        assert!(found, "{} ({:?})", row.name(), row.service == Service::Director);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_a_database_the_stubs_answer() {
    let rifts = rifts_with_core(&Settings::default(), Arc::new(Named));
    assert!(rifts.loaded());
    assert_covers(&rifts);
    let ticket = call(
        &rifts,
        "POST",
        "/rustic/v1/hunts/:sessionId/tickets/self",
        host(),
        &[("sessionId", "x")],
        "{}",
    )
    .await;
    assert!(matches!(ticket, Err(SeamError::Unimplemented(_))), "{ticket:?}");
    let relay = rifts
        .routes()
        .find(|(route, _)| {
            route
                .path
                .starts_with("/launcher/v1/p2p/hunts/:sessionId/transport-config")
        })
        .map(|(route, handler)| (*route, handler));
    assert!(relay.is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn with_a_sqlite_database() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("realms.sqlite");
    let path = file.display().to_string();
    let settings = Settings::from_pairs([("DAINTY_DB_FILE", path.as_str())]);
    let rifts = tokio::task::spawn_blocking(move || rifts_with_core(&settings, Arc::new(Named)))
        .await
        .unwrap();
    assert!(rifts.loaded());
    assert_covers(&rifts);

    let body = r#"{"name":"horizon","hostPublicKey":"AAAAAAAAAAAAAAAAAAAAAAAA"}"#;
    let created = call(&rifts, "POST", "/launcher/v1/realms", host(), &[], body)
        .await
        .unwrap();
    assert_eq!(created.status, 200, "{}", created.body.0);
    assert!(created.body.0.contains(r#""name":"horizon""#), "{}", created.body.0);
    assert!(created.body.0.contains("SlayerHost"), "{}", created.body.0);
    let mine = call(&rifts, "GET", "/launcher/v1/realms/mine", host(), &[], "")
        .await
        .unwrap();
    assert!(mine.body.0.contains("horizon"), "{}", mine.body.0);

    let missing = call(
        &rifts,
        "GET",
        "/rustic/v1/hunts/:sessionId/status",
        host(),
        &[("sessionId", "x")],
        "",
    )
    .await;
    assert_eq!(code(missing), "P2P_SESSION_NOT_FOUND");

    // The directory's own limiter: five creates an hour, the first one included
    let mut last = String::new();
    for _ in 0..5 {
        last = code(call(&rifts, "POST", "/launcher/v1/realms", host(), &[], body).await);
    }
    assert_eq!(last, "AUTH_RATE_LIMITED");
}

/// The admission key the server signs with reaches a signed-in launcher, for the game folder (no Node route).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_admission_key_is_served_to_launchers() {
    use rustic_rifts_admission::{KeyId, P256KeyGenerator, keys};
    let (pair, _, _) = P256KeyGenerator.generate_with_pems(KeyId("k7".to_owned())).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("realms.sqlite").display().to_string();
    let settings = Settings::from_pairs([
        ("DAINTY_DB_FILE", path.as_str()),
        (keys::KEY_ID_SETTING, "k7"),
        (keys::SIGNING_KEY_SETTING, pair.signing_key.expose()),
        (keys::VERIFY_KEY_SETTING, pair.verify_key.as_str()),
    ]);
    let rifts = tokio::task::spawn_blocking(move || rifts_with_core(&settings, Arc::new(Named)))
        .await
        .unwrap();
    let served = call(&rifts, "GET", ADMISSION_KEY_PATH, host(), &[], "").await.unwrap();
    assert_eq!(served.status, 200);
    assert_eq!(
        served.body.0,
        keys::admission_key_json(&KeyId("k7".to_owned()), &pair.public_xy)
    );
    let anonymous = call(&rifts, "GET", ADMISSION_KEY_PATH, Caller::Anonymous, &[], "").await;
    assert_eq!(code(anonymous), "AUTH_UNAUTHORIZED");

    let stubbed = rifts_with_core(&Settings::default(), Arc::new(Named));
    let unset = call(&stubbed, "GET", ADMISSION_KEY_PATH, host(), &[], "").await;
    assert!(matches!(unset, Err(SeamError::Unimplemented(_))), "{unset:?}");
}
