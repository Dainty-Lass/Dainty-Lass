//! "Rustic Isles 1.14.7" loads into DaintyLass and answers 501 (RU4's verify): compiled in through module-api's
//! `InProcessModule`, and out of process through `ChildModule::spawn` on its built executable, with every route of
//! both services requested.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use isles_core::routes::RouteSet;
use isles_core::testing::{Mounts, drive};
use module_api::host::file_sha256;
use module_api::stub::recording_host;
use module_api::{ChildModule, ChildOptions, ExternalModule, GameModule, InProcessModule, NodeIdentity};

const MOUNTS: Mounts = Mounts {
    set: RouteSet::V1147,
    rifts: cfg!(feature = "realms"),
    admin: true,
};

/// The admin API on, so its routes are mounted and swept too.
fn env_file() -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "MYSTICPARADOX_ADMIN_API=on").unwrap();
    file
}

#[test]
fn the_manifest_is_ddfours() {
    let manifest = isles_1_14_7::module().manifest();
    manifest.validate().unwrap();
    assert_eq!(manifest.id.as_str(), "isles-1-14-7");
    assert_eq!(manifest.display_name, "Rustic Isles 1.14.7");
    assert_eq!(manifest.game_version.to_string(), "1.14.7");
    assert!(manifest.answers_invite("reforged") && manifest.answers_invite("1.14.7"));
    assert_eq!(manifest.capabilities.realms, cfg!(feature = "realms"));
    assert!(manifest.capabilities.network);
    assert!(!manifest.unverified);
}

#[tokio::test(flavor = "multi_thread")]
async fn it_answers_501_compiled_in() {
    let host = recording_host();
    let module = InProcessModule::new(
        Arc::new(isles_1_14_7::module()),
        NodeIdentity::standalone(),
        host.clone(),
    )
    .unwrap();
    let env = env_file();
    let failures = drive(&module, &host, env.path(), MOUNTS).await;
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test(flavor = "multi_thread")]
async fn it_answers_501_out_of_process() {
    let path = PathBuf::from(env!("CARGO_BIN_EXE_isles-1-14-7"));
    let external = ExternalModule {
        sha256: file_sha256(&path).await.unwrap(),
        path,
        args: vec![],
        expected: Some("isles-1-14-7".parse().unwrap()),
    };
    let host = recording_host();
    let module = ChildModule::spawn(
        &external,
        NodeIdentity::standalone(),
        host.clone(),
        ChildOptions::default(),
    )
    .await
    .unwrap();
    let env = env_file();
    let failures = drive(&module, &host, env.path(), MOUNTS).await;
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn director_args_are_one_point_fourteens() {
    let args = isles_1_14_7::DIRECTOR.launch_args(&isles_core::Settings::default());
    assert_eq!(
        args.join(" "),
        "-EpicPortal -server -nullrhi -warp -nosound -RepDriverEnable \
         -LogCmds=\"LogPhoenixCharacter VeryVerbose, LogOnline Verbose, LogLoad Off\""
    );
}

/// RU5 game-data: the Rust answers equal the reference Node handlers' (`capture.mjs`, Node 22).
#[tokio::test(flavor = "multi_thread")]
async fn game_data_answers_equal_nodes() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/isles-core/tests/golden");
    let failures = isles_core::game_data::golden::check(isles_1_14_7::version(), "1.14.7", &fixtures).await;
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// RU5 game-data: the auth gate, the missing game data file and the bodies Express refuses.
#[tokio::test(flavor = "multi_thread")]
async fn game_data_edges() {
    let failures = isles_core::game_data::golden::check_edges(isles_1_14_7::version()).await;
    assert!(
        failures.is_empty(),
        "{}",
        failures.join(
            "
"
        )
    );
}

#[cfg(feature = "realms")]
#[tokio::test(flavor = "multi_thread")]
async fn realms_and_guard_routes_serve_live_implementations_when_database_is_configured() {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use rifts_seam::{
        AccountId, BoxFuture, Caller, Candidate, IslesCore, Json, Method, NoCore, Party, PartyId, RequestInfo,
        RouteCall, SeamError, SessionId,
    };

    struct TestCore {
        names: Mutex<BTreeMap<String, String>>,
    }

    impl IslesCore for TestCore {
        fn display_name(&self, account: AccountId) -> BoxFuture<'_, Result<Option<String>, SeamError>> {
            let found = self.names.lock().unwrap().get(&account.0).cloned();
            Box::pin(async move { Ok(found) })
        }

        fn find_by_display_name(&self, username: String) -> BoxFuture<'_, Result<Option<AccountId>, SeamError>> {
            let found = self
                .names
                .lock()
                .unwrap()
                .iter()
                .find(|(_, name)| name.eq_ignore_ascii_case(&username))
                .map(|(id, _)| AccountId(id.clone()));
            Box::pin(async move { Ok(found) })
        }

        fn launcher_account_eligible(&self, account: AccountId) -> BoxFuture<'_, Result<bool, SeamError>> {
            NoCore.launcher_account_eligible(account)
        }

        fn party(&self, party: PartyId) -> BoxFuture<'_, Result<Option<Party>, SeamError>> {
            NoCore.party(party)
        }

        fn get_or_create_party(&self, account: AccountId) -> BoxFuture<'_, Result<Party, SeamError>> {
            NoCore.get_or_create_party(account)
        }

        fn set_candidate(&self, account: AccountId, candidate: Candidate) -> Result<(), SeamError> {
            NoCore.set_candidate(account, candidate)
        }

        fn cancel_travel(&self, hunt_id: &str, accounts: &[AccountId], reason: &str) -> Result<bool, SeamError> {
            NoCore.cancel_travel(hunt_id, accounts, reason)
        }

        fn dedicated_servers_configured(&self) -> bool {
            false
        }

        fn touch_game_session(&self, account: AccountId) -> BoxFuture<'_, Result<(), SeamError>> {
            NoCore.touch_game_session(account)
        }

        fn record_hunt_activity(&self, session: SessionId, activity: Json) -> BoxFuture<'_, Result<(), SeamError>> {
            NoCore.record_hunt_activity(session, activity)
        }

        fn healthy_guard_session(&self, _: AccountId) -> BoxFuture<'_, Result<Option<Json>, SeamError>> {
            Box::pin(async { Ok(None) })
        }

        fn published_guard_artifacts(&self) -> BoxFuture<'_, Result<Json, SeamError>> {
            NoCore.published_guard_artifacts()
        }

        fn guard_mode(&self) -> rifts_seam::GuardMode {
            rifts_seam::GuardMode::Observe
        }

        fn operations_policy(&self) -> BoxFuture<'_, Result<Json, SeamError>> {
            Box::pin(async {
                Ok(Json(
                    r#"{"version":0,"p2pEmergencyStop":false,"coopTransport":"ICE"}"#.to_owned(),
                ))
            })
        }

        fn verify_operations_step_up(&self, key_id: String, step_up: String) -> BoxFuture<'_, Result<bool, SeamError>> {
            NoCore.verify_operations_step_up(key_id, step_up)
        }
    }

    let db_dir = tempfile::tempdir().unwrap();
    let db_path = db_dir.path().join("realms_live.sqlite");
    let mut env = BTreeMap::new();
    env.insert("REFORGED_DB_FILE".to_owned(), db_path.display().to_string());
    let settings = isles_core::Settings::new(env, BTreeMap::new());

    let host_account = "01J8B8N5Q8G2K0000000000001";
    let mut names = BTreeMap::new();
    names.insert(host_account.to_owned(), "SlayerHost".to_owned());
    let core: Arc<dyn IslesCore> = Arc::new(TestCore {
        names: Mutex::new(names),
    });

    let rifts = isles_1_14_7::rifts_with_core(&settings, core);
    assert!(rifts.loaded());

    // 1. Create a realm through POST /launcher/v1/realms and verify IslesCore resolved the host display name.
    let (create_route, create_handler) = rifts
        .routes()
        .find(|(r, _)| r.method == Method::Post && r.path == "/launcher/v1/realms")
        .expect("POST /launcher/v1/realms mounted");
    let create_reply = create_handler
        .handle(RouteCall {
            route: *create_route,
            params: Vec::new(),
            query: String::new(),
            request: RequestInfo::default(),
            caller: Caller::Launcher(AccountId(host_account.to_owned())),
            body: br#"{"name":"Horizon","hostPublicKey":"aG9zdC1wdWJsaWMta2V5LTAwMDE=","joinPolicy":"PUBLIC","listed":true}"#.to_vec(),
        })
        .await
        .expect("create_realm succeeds");
    assert_eq!(create_reply.status, 200);
    let created: serde_json::Value = serde_json::from_str(&create_reply.body.0).unwrap();
    assert_eq!(created["realm"]["name"], "horizon");
    assert_eq!(created["realm"]["host"]["displayName"], "SlayerHost");

    // 2. List listed realms through GET /launcher/v1/realms/listed.
    let (list_route, list_handler) = rifts
        .routes()
        .find(|(r, _)| r.method == Method::Get && r.path == "/launcher/v1/realms/listed")
        .expect("GET /launcher/v1/realms/listed mounted");
    let list_reply = list_handler
        .handle(RouteCall {
            route: *list_route,
            params: Vec::new(),
            query: String::new(),
            request: RequestInfo::default(),
            caller: Caller::Launcher(AccountId(host_account.to_owned())),
            body: Vec::new(),
        })
        .await
        .expect("list_listed succeeds");
    assert_eq!(list_reply.status, 200);
    let listed: serde_json::Value = serde_json::from_str(&list_reply.body.0).unwrap();
    assert_eq!(listed["realms"].as_array().unwrap().len(), 1);
    assert_eq!(listed["realms"][0]["name"], "horizon");

    // 3. Query Guard's GET /mystic/v1/hunts/:sessionId/reward/self and verify SqlGuardStore answers P2P_SESSION_NOT_FOUND (not 501 Unimplemented).
    let (reward_route, reward_handler) = rifts
        .routes()
        .find(|(r, _)| r.method == Method::Get && r.path == "/mystic/v1/hunts/:sessionId/reward/self")
        .expect("GET /mystic/v1/hunts/:sessionId/reward/self mounted");
    let reward_err = reward_handler
        .handle(RouteCall {
            route: *reward_route,
            params: vec![(
                "sessionId".to_owned(),
                "00000000-0000-4000-8000-000000000001".to_owned(),
            )],
            query: String::new(),
            request: RequestInfo::default(),
            caller: Caller::Launcher(AccountId(host_account.to_owned())),
            body: Vec::new(),
        })
        .await
        .expect_err("missing receipt refuses with P2P_SESSION_NOT_FOUND");
    match reward_err {
        rifts_seam::SeamError::Refused(refusal) => assert_eq!(refusal.code, "P2P_SESSION_NOT_FOUND"),
        other => panic!("expected Refused(P2P_SESSION_NOT_FOUND), got {other:?}"),
    }

    // 4. Call Guard's PATCH /admin/v1/operations/reward-rollout and verify CoreOperationsPolicy updates IslesCore.
    let (rollout_route, rollout_handler) = rifts
        .routes()
        .find(|(r, _)| r.method == Method::Patch && r.path == "/admin/v1/operations/reward-rollout")
        .expect("PATCH /admin/v1/operations/reward-rollout mounted");
    let rollout_reply = rollout_handler
        .handle(RouteCall {
            route: *rollout_route,
            params: Vec::new(),
            query: String::new(),
            request: RequestInfo::default(),
            caller: Caller::Admin {
                key_id: "admin-key-1".to_owned(),
            },
            body: br#"{"rewardRollout":{"version":2,"tier0ShadowEnabled":true,"tier1GrantsEnabled":false},"expectedVersion":0,"reason":"canary verified rollout"}"#.to_vec(),
        })
        .await
        .expect("set_reward_rollout succeeds");
    assert_eq!(rollout_reply.status, 200);

    // 5. Rate limit: createRealm allows 5 attempts per hour per account (realmsService.ts RATE_BUDGETS.create).
    // Attempts 2..=5 under the limit pass or reach validation/conflict, while the 6th is refused with Node's
    // AUTH_RATE_LIMITED (HTTP 429) and a different account still passes.
    let (release_route, release_handler) = rifts
        .routes()
        .find(|(r, _)| r.method == Method::Post && r.path == "/launcher/v1/realms/mine/release")
        .expect("POST /launcher/v1/realms/mine/release mounted");
    for idx in 2..=5 {
        release_handler
            .handle(RouteCall {
                route: *release_route,
                params: Vec::new(),
                query: String::new(),
                request: RequestInfo::default(),
                caller: Caller::Launcher(AccountId(host_account.to_owned())),
                body: Vec::new(),
            })
            .await
            .expect("release succeeds");
        let reply = create_handler
            .handle(RouteCall {
                route: *create_route,
                params: Vec::new(),
                query: String::new(),
                request: RequestInfo::default(),
                caller: Caller::Launcher(AccountId(host_account.to_owned())),
                body: format!(
                    r#"{{"name":"Horizon{idx}","hostPublicKey":"aG9zdC1wdWJsaWMta2V5LTAwMDE=","joinPolicy":"PUBLIC","listed":true}}"#
                )
                .into_bytes(),
            })
            .await
            .expect("attempt under the 5/hour create budget passes");
        assert_eq!(reply.status, 200);
    }
    let over_limit = create_handler
        .handle(RouteCall {
            route: *create_route,
            params: Vec::new(),
            query: String::new(),
            request: RequestInfo::default(),
            caller: Caller::Launcher(AccountId(host_account.to_owned())),
            body: br#"{"name":"Horizon6","hostPublicKey":"aG9zdC1wdWJsaWMta2V5LTAwMDE=","joinPolicy":"PUBLIC","listed":true}"#.to_vec(),
        })
        .await
        .expect_err("6th create attempt within the hour is rate-limited");
    match over_limit {
        rifts_seam::SeamError::Refused(ref refusal) => {
            assert_eq!(refusal.code, "AUTH_RATE_LIMITED");
            assert_eq!(
                refusal.message,
                "Too many realm requests. Please wait a moment and try again."
            );
            let launcher_err = isles_core::areas::auth::launcher::seam_error(over_limit.clone());
            assert_eq!(launcher_err.status(), 429);
            assert_eq!(
                launcher_err.body("req-1"),
                serde_json::json!({
                    "error": {
                        "code": "AUTH_RATE_LIMITED",
                        "message": "Too many realm requests. Please wait a moment and try again.",
                        "requestId": "req-1"
                    }
                })
            );
        }
        other => panic!("expected Refused(AUTH_RATE_LIMITED), got {other:?}"),
    }
    let other_account = "01J8B8N5Q8G2K0000000000002";
    let other_reply = create_handler
        .handle(RouteCall {
            route: *create_route,
            params: Vec::new(),
            query: String::new(),
            request: RequestInfo::default(),
            caller: Caller::Launcher(AccountId(other_account.to_owned())),
            body: br#"{"name":"OtherRealm","hostPublicKey":"aG9zdC1wdWJsaWMta2V5LTAwMDI=","joinPolicy":"PUBLIC","listed":true}"#.to_vec(),
        })
        .await
        .expect("a second account under the limit passes");
    assert_eq!(other_reply.status, 200);

    // 6. Recovery: a live hunt whose host reports an unhealthy process on POST /mystic/v1/hunts/:sessionId/lease is
    // recovered to ABORTED by RealmsRecovery (does not stay live), releases its lease and party key, and queues FENCE_NOW.
    let db_config = store::DatabaseConfig::from_lookup(|_| None, db_path.clone()).unwrap();
    let realms_db = rifts_realms::db::RealmsDb::open(
        &db_config,
        &store::DatabaseOptions::default(),
        module_api::NodeId::new("local").unwrap(),
    )
    .unwrap();
    let hunts = rifts_realms::repo::hunts::HuntRepository::new(realms_db.clone());
    let caps = rifts_realms::capabilities::HostCapabilities::new(realms_db);
    let now = rifts_realms::time::now();
    let session_id = "0d6f2b9c-5b1e-4c3a-9e57-2f1a8b7c4d10";
    let mut host_participant = rifts_realms::hunt::Participant::pending(host_account, "ice:host", "cand-1");
    host_participant.admission_state = rifts_realms::hunt::AdmissionState::Admitted;
    host_participant.local_bridge_instance_id = Some("bridge-1".to_owned());
    host_participant.local_bridge_port = Some(29000);
    host_participant.local_bridge_ready_at = Some(now);
    let mut other_participant = rifts_realms::hunt::Participant::pending(other_account, "ice:other", "cand-2");
    other_participant.admission_state = rifts_realms::hunt::AdmissionState::Admitted;
    other_participant.local_bridge_instance_id = Some("bridge-2".to_owned());
    other_participant.local_bridge_port = Some(29001);
    other_participant.local_bridge_ready_at = Some(now);
    let hunt = rifts_realms::hunt::HuntSession {
        session_id: session_id.to_owned(),
        generation: 1,
        state: rifts_realms::hunt::HuntState::Running,
        state_version: 1,
        party_id: "party-1".to_owned(),
        active_party_key: Some("party-1".to_owned()),
        party_revision: 1,
        hunt_definition_id: "Hunt_Test".to_owned(),
        game_mode: None,
        build: 647_472,
        bridge_protocol: 1,
        virtual_port: 1,
        host_account_id: host_account.to_owned(),
        host_steam_id: "ice:host".to_owned(),
        bridge_instance_id: "bridge-1".to_owned(),
        roster: vec![host_participant, other_participant],
        lobby_id: Some("lobby-1".to_owned()),
        coop_transport: Some("ICE".to_owned()),
        lease_expires_at: Some(rifts_seam::UnixMillis(now.0 + 10_000)),
        created_at: now,
        expires_at: rifts_seam::UnixMillis(now.0 + 3_600_000),
        metadata_nonce: None,
        hub_kind: None,
        realm_id: None,
        host_participates: None,
        host_local_bridge_port: None,
        server_udp_port: Some(7777),
        host_local_bridge_ready_at: None,
        hub_room_id: None,
        max_peers: None,
        guard_completion_mode: None,
        host_offer_id: None,
        offer_expires_at: None,
        startup_deadline: None,
        last_heartbeat_at: Some(now),
        last_lease_sequence: Some(0),
        suspect_since: None,
        suspect_deadline: None,
        pre_suspect_state: None,
        failure_reason: None,
        tried_host_account_ids: Vec::new(),
        terminal_at: None,
        recovery_deadline: None,
        recovery_retry_at: None,
        completion_reason: None,
        drain_deadline: None,
        cancelled_by_account_id: None,
        trust_tier: Some(0),
        guard_encounter_state: None,
        extra: serde_json::Map::new(),
    };
    hunts
        .create_hunt(&rifts_seam::Actor::System("test"), &hunt)
        .await
        .unwrap();
    let (secret, capability) = caps
        .mint(
            &hunt,
            "bridge-1",
            vec![rifts_seam::CapabilityScope::LeaseHeartbeat],
            now,
            rifts_seam::UnixMillis(now.0 + 3_600_000),
        )
        .await
        .unwrap();

    let (lease_route, lease_handler) = rifts
        .routes()
        .find(|(r, _)| r.method == Method::Post && r.path == "/mystic/v1/hunts/:sessionId/lease")
        .expect("POST /mystic/v1/hunts/:sessionId/lease mounted");
    let mut lease_req = RequestInfo::default();
    lease_req.headers.push((
        rifts_seam::HOST_CAPABILITY_HEADER.to_owned(),
        secret.expose().to_owned(),
    ));
    let lease_err = lease_handler
        .handle(RouteCall {
            route: *lease_route,
            params: vec![("sessionId".to_owned(), session_id.to_owned())],
            query: String::new(),
            request: lease_req,
            caller: Caller::Anonymous,
            body:
                br#"{"leaseSequence":1,"bridgeAlive":false,"serverReady":true,"admittedPeerCount":2,"queuePressure":0}"#
                    .to_vec(),
        })
        .await
        .expect_err("unhealthy host heartbeat fences and recovers the hunt");
    match lease_err {
        rifts_seam::SeamError::Refused(refusal) => assert_eq!(refusal.code, "P2P_GENERATION_FENCED"),
        other => panic!("expected Refused(P2P_GENERATION_FENCED), got {other:?}"),
    }
    let recovered = hunts.find_hunt(session_id).await.unwrap().unwrap();
    assert_eq!(recovered.state, rifts_realms::hunt::HuntState::Aborted);
    assert_eq!(recovered.failure_reason.as_deref(), Some("HOST_PROCESS_UNHEALTHY"));
    assert_eq!(recovered.lease_expires_at, None);
    assert_eq!(recovered.active_party_key, None);
    let stored_cap = hunts
        .find_host_capability_by_digest(&capability.secret_digest)
        .await
        .unwrap()
        .unwrap();
    assert!(stored_cap.revoked_at.is_some());
}
