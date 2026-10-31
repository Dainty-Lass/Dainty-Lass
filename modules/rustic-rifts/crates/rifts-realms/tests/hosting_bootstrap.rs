//! End-to-end route integration test for player-hosted realm hub sessions, admission tickets, and ICE relay signaling.
#![allow(clippy::unwrap_used)]

mod common;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use common::kit::NoRateLimit;
use common::targets;
use rifts_admission::{
    Admission, AdmissionError, AdmissionRoutes, AdmissionService, CapabilityResolver, Es256Tickets, HostCapability,
    KeyId, MemoryAdmissionStore, P256KeyGenerator,
};
use rifts_realms::capabilities::HostCapabilities;
use rifts_realms::control::ControlChannel;
use rifts_realms::directory::{DirectoryParts, RealmDirectory};
use rifts_realms::ids::seam_session_id;
use rifts_realms::leases::{Hosting, HostingParts, HostingRuntime, HostingSettings};
use rifts_realms::lifecycle::RealmsRecovery;
use rifts_realms::ports::{
    Clock, DefaultOperations, FixedAccounts, ManualClock, NoGuardSessions, RandomIds, RealmEventSink, StderrLog,
};
use rifts_realms::realm::DEFAULT_REALM_LIMITS;
use rifts_realms::selection::derive_ice_peer_id;
use rifts_realms::{RealmsRoutes, RealmsRoutesParts};
use rifts_relay::{
    Ice, IceService, OperationsPolicy, ProviderAnswer, ProviderHttp, ProviderRequest, ProviderUnreachable, RelayConfig,
    RelayGrants, RelayRoutes, RelayUsageCache, TurnProvider, TurnRest,
};
use rifts_seam::{
    AccountId, BoxFuture, Caller, HOST_CAPABILITY_HEADER, NoCore, NodeId, PeerId, RequestInfo, Route, RouteCall,
    RouteHandler, Secret, UnixMillis,
};
use serde_json::{Value, json};

const NOW: UnixMillis = UnixMillis(1_791_892_800_000);
const HOST_ACC: &str = "0123456789abcdef0123456789abcdef";
const JOIN_ACC: &str = "fedcba9876543210fedcba9876543210";
const HOST_KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

struct NoProviderHttp;
impl ProviderHttp for NoProviderHttp {
    fn post(&self, _request: ProviderRequest) -> BoxFuture<'_, Result<ProviderAnswer, ProviderUnreachable>> {
        Box::pin(async { Err(ProviderUnreachable) })
    }
}

fn find_route(routes: &'static [Route], method: &str, path: &str) -> Route {
    *routes
        .iter()
        .find(|r| r.method.as_str() == method && r.path == path)
        .unwrap_or_else(|| panic!("missing route {method} {path}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn realm_hub_host_and_joiner_bootstrap_through_admission_and_relay_routes() {
    for target in targets() {
        let db = target.open();
        let clock = Arc::new(ManualClock::at(NOW));
        let node = NodeId::new("local").unwrap();

        let (pair, _, _) = P256KeyGenerator.generate_with_pems(KeyId("k1".to_owned())).unwrap();
        let key_settings: HashMap<String, String> = [
            ("P2P_ADMISSION_KEY_ID".to_owned(), pair.key_id.0.clone()),
            (
                "P2P_ADMISSION_SIGNING_PRIVKEY_B64".to_owned(),
                pair.signing_key.expose().to_owned(),
            ),
            ("P2P_ADMISSION_VERIFY_PUBKEY_B64".to_owned(), pair.verify_key.clone()),
        ]
        .into_iter()
        .collect();
        let tickets = Arc::new(Es256Tickets::from_settings(&key_settings).unwrap());
        let admission_store = Arc::new(MemoryAdmissionStore::default());
        let capabilities = Arc::new(HostCapabilities::new(db.clone()));

        let mut settings = HostingSettings::from_settings(&HashMap::<String, String>::new());
        settings.policy.enabled = true;
        settings.policy.allow_all_accounts = true;
        settings.realm_hosting = true;
        let recovery = Arc::new(RealmsRecovery::new(
            db.clone(),
            Arc::new(NoCore),
            clock.clone(),
            Arc::new(StderrLog),
        ));
        let module_node = module_api::NodeId::new("local").unwrap();
        let hosting = Arc::new(
            Hosting::new(HostingParts {
                db: db.clone(),
                settings,
                clock: clock.clone(),
                operations: Arc::new(DefaultOperations::default()),
                guard: Arc::new(NoGuardSessions),
                recovery,
                expected_artifacts: Arc::new(|| {
                    Ok(rifts_realms::hunt::ArtifactHashes {
                        game_exe: "f3f95fbf22709ef077a11632ed95d675b7209b7e0172d9ecb9ba7d02a1fbb66b".to_owned(),
                        internal_server: "1111111111111111111111111111111111111111111111111111111111111111".to_owned(),
                        winmm_proxy: "2222222222222222222222222222222222222222222222222222222222222222".to_owned(),
                        steam_bridge: "3333333333333333333333333333333333333333333333333333333333333333".to_owned(),
                        steam_runtime: "3333333333333333333333333333333333333333333333333333333333333333".to_owned(),
                        admission_public_key: "4444444444444444444444444444444444444444444444444444444444444444"
                            .to_owned(),
                    })
                }),
                log: Arc::new(StderrLog),
            })
            .with_runtime(HostingRuntime {
                capabilities: capabilities.clone(),
                tickets: tickets.clone(),
                admission_store: admission_store.clone(),
                core: Arc::new(NoCore),
                node: module_node.clone(),
                backend_url: "http://127.0.0.1:61000".to_owned(),
                host_server_udp_port: 8791,
                realms_enabled: true,
            }),
        );

        let mut names = BTreeMap::new();
        names.insert(HOST_ACC.to_owned(), "HostSlayer".to_owned());
        names.insert(JOIN_ACC.to_owned(), "JoinSlayer".to_owned());
        let mut usernames = BTreeMap::new();
        usernames.insert("hostslayer".to_owned(), HOST_ACC.to_owned());
        usernames.insert("joinslayer".to_owned(), JOIN_ACC.to_owned());
        let directory = Arc::new(RealmDirectory::new(DirectoryParts {
            db: db.clone(),
            accounts: Arc::new(FixedAccounts { names, usernames }),
            events: Some(hosting.clone() as Arc<dyn RealmEventSink>),
            clock: clock.clone(),
            limiter: Arc::new(NoRateLimit),
            ids: Arc::new(RandomIds),
            limits: DEFAULT_REALM_LIMITS,
            log: Arc::new(StderrLog),
        }));
        let control = Arc::new(ControlChannel::new(db.clone(), clock.clone()));

        let realms_routes = RealmsRoutes::new(RealmsRoutesParts {
            directory: Some(directory.clone()),
            control: Some(control),
            hosting: Some(hosting.clone()),
            capabilities: Some(capabilities.clone()),
            clock: clock.clone(),
            realms_enabled: true,
        });

        let admissions: Arc<dyn AdmissionService> = Arc::new(Admission::new(
            tickets.clone(),
            admission_store.clone(),
            hosting.clone(),
            Some(Secret::new("test-transport-secret-key".to_owned())),
            Arc::new({
                let c = clock.clone();
                move || c.now()
            }),
        ));
        let cap_check = capabilities.clone();
        let cap_clock = clock.clone();
        let cap_node = module_node.clone();
        let resolver: CapabilityResolver = Arc::new(move |header, session, scope| {
            let cap_check = cap_check.clone();
            let now = cap_clock.now();
            let raw = header.map(str::to_owned);
            let session_str = session.to_owned();
            let node_clone = cap_node.clone();
            Box::pin(async move {
                let rec = cap_check
                    .check(raw.as_deref(), &session_str, scope, now)
                    .await
                    .map_err(|e| match e {
                        rifts_realms::RealmsError::Refused(r) => AdmissionError::Refused(r),
                        other => AdmissionError::Refused(rifts_seam::Refusal {
                            code: "AUTH_UNAUTHORIZED".to_owned(),
                            message: other.to_string(),
                        }),
                    })?;
                let peer_str = if rec.host_steam_id.is_empty() {
                    derive_ice_peer_id(&rec.host_account_id)
                } else {
                    rec.host_steam_id.clone()
                };
                let peer_num = peer_str
                    .parse::<u64>()
                    .unwrap_or_else(|_| rifts_admission::derive_ice_peer_id(&AccountId(rec.host_account_id.clone())).0);
                let seam_session = seam_session_id(&rec.session_id, &node_clone).unwrap();
                Ok(HostCapability {
                    session: seam_session,
                    wire_session: rec.session_id,
                    generation: u64::try_from(rec.generation.max(1)).unwrap_or(1),
                    host_account: AccountId(rec.host_account_id),
                    host_peer: PeerId(peer_num),
                    bridge_instance_id: rec.bridge_instance_id,
                })
            })
        });
        let admission_routes = AdmissionRoutes::new(admissions, Some(resolver), node.clone());

        let mut relay_cfg = RelayConfig::from_settings(&HashMap::<String, String>::new());
        relay_cfg.stun_urls = vec!["stun:127.0.0.1:3478".to_owned()];
        relay_cfg.provider = TurnProvider::TurnRest;
        relay_cfg.turn_rest = Some(TurnRest {
            urls: vec!["turn:127.0.0.1:3478?transport=udp".to_owned()],
            secret: Secret::new("turn-secret-0123456789abcdef".to_owned()),
        });
        relay_cfg.transport_key_secret = Some(Secret::new("0123456789abcdef0123456789abcdef".to_owned()));
        let grants = RelayGrants {
            config: Arc::new(relay_cfg),
            http: Arc::new(NoProviderHttp),
            usage: Arc::new(RelayUsageCache::default()),
        };
        let relay_service: Arc<dyn IceService> = Arc::new(Ice::new(
            hosting.clone(),
            grants,
            hosting.clone() as Arc<dyn OperationsPolicy>,
            Arc::new({
                let c = clock.clone();
                move || c.now()
            }),
        ));
        let relay_routes = RelayRoutes::new(relay_service, node.clone());

        // 1. Host creates a public realm and sends a realm heartbeat.
        let create_route = find_route(realms_routes.routes(), "POST", "/launcher/v1/realms");
        let reply = realms_routes
            .handle(RouteCall {
                route: create_route,
                params: vec![],
                query: String::new(),
                request: RequestInfo::default(),
                caller: Caller::Launcher(AccountId(HOST_ACC.to_owned())),
                body: serde_json::to_vec(&json!({
                    "name": "ironhold",
                    "hostPublicKey": HOST_KEY,
                    "joinPolicy": "PUBLIC",
                    "listed": true,
                }))
                .unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(reply.status, 200);

        let hb_route = find_route(realms_routes.routes(), "POST", "/launcher/v1/realms/mine/heartbeat");
        let reply = realms_routes
            .handle(RouteCall {
                route: hb_route,
                params: vec![],
                query: String::new(),
                request: RequestInfo::default(),
                caller: Caller::Launcher(AccountId(HOST_ACC.to_owned())),
                body: serde_json::to_vec(&json!({
                    "hostPublicKey": HOST_KEY,
                    "maxServers": 2,
                    "maxPlayers": 6,
                }))
                .unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(reply.status, 200);

        // 2. Joiner joins the public realm.
        let join_route = find_route(realms_routes.routes(), "POST", "/launcher/v1/realms/join");
        let reply = realms_routes
            .handle(RouteCall {
                route: join_route,
                params: vec![],
                query: String::new(),
                request: RequestInfo::default(),
                caller: Caller::Launcher(AccountId(JOIN_ACC.to_owned())),
                body: serde_json::to_vec(&json!({ "name": "ironhold" })).unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(reply.status, 200);

        // 3. Host submits P2P host presence.
        let presence_route = find_route(realms_routes.routes(), "POST", "/launcher/v1/p2p/host-presence");
        let reply = realms_routes
            .handle(RouteCall {
                route: presence_route,
                params: vec![],
                query: String::new(),
                request: RequestInfo::default(),
                caller: Caller::Launcher(AccountId(HOST_ACC.to_owned())),
                body: serde_json::to_vec(&json!({
                    "observedAt": rifts_realms::time::iso(NOW),
                    "consentToHost": true,
                    "powerSaver": false,
                    "runtimeSupported": true,
                    "transportReady": true,
                    "peerId": derive_ice_peer_id(HOST_ACC),
                    "launcherVersion": "0.2.0",
                    "freePhysicalMemoryMiB": 8192,
                    "commitHeadroomMiB": 8192,
                    "logicalCpuCount": 8,
                    "cpuLoadPercent": 15.0,
                    "backendRttMs": 20.0,
                    "worstPeerRttMs": 25.0,
                    "jitterMs": 2.0,
                    "packetLossPercent": 0.0,
                    "realmHostSessions": 1,
                    "artifacts": {
                        "gameExe": "f3f95fbf22709ef077a11632ed95d675b7209b7e0172d9ecb9ba7d02a1fbb66b",
                        "internalServer": "1111111111111111111111111111111111111111111111111111111111111111",
                        "winmmProxy": "2222222222222222222222222222222222222222222222222222222222222222",
                        "transportModule": "3333333333333333333333333333333333333333333333333333333333333333",
                        "transportRuntime": "3333333333333333333333333333333333333333333333333333333333333333",
                        "admissionPublicKey": "4444444444444444444444444444444444444444444444444444444444444444",
                    }
                }))
                .unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(reply.status, 200);

        // 4. Host starts the Ramsgate hub session.
        let start_hub_route = find_route(realms_routes.routes(), "POST", "/launcher/v1/realm-hosting/hubs/start");
        let reply = realms_routes
            .handle(RouteCall {
                route: start_hub_route,
                params: vec![],
                query: String::new(),
                request: RequestInfo::default(),
                caller: Caller::Launcher(AccountId(HOST_ACC.to_owned())),
                body: serde_json::to_vec(&json!({ "hubKind": "RAMSGATE", "maxPlayers": 6 })).unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(reply.status, 200);
        let hub_start: Value = serde_json::from_str(&reply.body.0).unwrap();
        let session_id = hub_start["sessionId"].as_str().unwrap().to_owned();
        assert!(hub_start["hostTicket"].as_str().is_some());

        // 5. Host fetches CreateHostBootstrap and marks host ready.
        let host_boot_route = find_route(
            realms_routes.routes(),
            "POST",
            "/launcher/v1/p2p/hunts/:sessionId/host-bootstrap",
        );
        let reply = realms_routes
            .handle(RouteCall {
                route: host_boot_route,
                params: vec![("sessionId".to_owned(), session_id.clone())],
                query: String::new(),
                request: RequestInfo::default(),
                caller: Caller::Launcher(AccountId(HOST_ACC.to_owned())),
                body: b"{}".to_vec(),
            })
            .await
            .unwrap();
        assert_eq!(reply.status, 200);
        let host_boot: Value = serde_json::from_str(&reply.body.0).unwrap();
        let host_cap_2 = host_boot["capability"].as_str().unwrap().to_owned();
        let bridge_instance_id = host_boot["bridgeInstanceId"].as_str().unwrap().to_owned();

        let host_ready_route = find_route(realms_routes.routes(), "POST", "/mystic/v1/hunts/:sessionId/host-ready");
        let reply = realms_routes
            .handle(RouteCall {
                route: host_ready_route,
                params: vec![("sessionId".to_owned(), session_id.clone())],
                query: String::new(),
                request: RequestInfo {
                    headers: vec![(HOST_CAPABILITY_HEADER.to_owned(), host_cap_2.clone())],
                    ..RequestInfo::default()
                },
                caller: Caller::Anonymous,
                body: serde_json::to_vec(&json!({
                    "generation": 1,
                    "bridgeInstanceId": bridge_instance_id,
                    "lobbyId": "ICE",
                }))
                .unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(reply.status, 200);

        // 6. Joiner calls CreateJoinBootstrap via AdmissionRoutes.
        let join_boot_route = find_route(
            admission_routes.routes(),
            "POST",
            "/launcher/v1/p2p/hunts/:sessionId/join-bootstrap",
        );
        let reply = admission_routes
            .handle(RouteCall {
                route: join_boot_route,
                params: vec![("sessionId".to_owned(), session_id.clone())],
                query: String::new(),
                request: RequestInfo::default(),
                caller: Caller::Launcher(AccountId(JOIN_ACC.to_owned())),
                body: b"{}".to_vec(),
            })
            .await
            .unwrap();
        assert_eq!(reply.status, 200);
        let join_boot: Value = serde_json::from_str(&reply.body.0).unwrap();
        let join_ticket = join_boot["ticket"].as_str().unwrap().to_owned();
        let joiner_peer_id = derive_ice_peer_id(JOIN_ACC);

        // 7. Joiner and host both fetch ICE transport config via RelayRoutes.
        let tc_route = find_route(
            relay_routes.routes(),
            "POST",
            "/launcher/v1/p2p/hunts/:sessionId/transport-config",
        );
        let join_tc_reply = relay_routes
            .handle(RouteCall {
                route: tc_route,
                params: vec![("sessionId".to_owned(), session_id.clone())],
                query: String::new(),
                request: RequestInfo::default(),
                caller: Caller::Launcher(AccountId(JOIN_ACC.to_owned())),
                body: serde_json::to_vec(&json!({ "generation": 1 })).unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(join_tc_reply.status, 200);
        let join_tc: Value = serde_json::from_str(&join_tc_reply.body.0).unwrap();
        assert_eq!(join_tc["peers"].as_array().unwrap().len(), 1);

        let host_tc_reply = relay_routes
            .handle(RouteCall {
                route: tc_route,
                params: vec![("sessionId".to_owned(), session_id.clone())],
                query: String::new(),
                request: RequestInfo::default(),
                caller: Caller::Launcher(AccountId(HOST_ACC.to_owned())),
                body: serde_json::to_vec(&json!({ "generation": 1 })).unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(host_tc_reply.status, 200);
        let host_tc: Value = serde_json::from_str(&host_tc_reply.body.0).unwrap();
        assert_eq!(host_tc["peers"].as_array().unwrap().len(), 1);

        // 8. Joiner sends an ICE signal to the host via RelayRoutes, and host receives it via PollP2PControl.
        let signal_route = find_route(
            relay_routes.routes(),
            "POST",
            "/launcher/v1/p2p/hunts/:sessionId/signal",
        );
        let sig_reply = relay_routes
            .handle(RouteCall {
                route: signal_route,
                params: vec![("sessionId".to_owned(), session_id.clone())],
                query: String::new(),
                request: RequestInfo::default(),
                caller: Caller::Launcher(AccountId(JOIN_ACC.to_owned())),
                body: serde_json::to_vec(&json!({
                    "generation": 1,
                    "toAccountId": HOST_ACC,
                    "kind": "CANDIDATE",
                    "data": "a=candidate:1 1 UDP 2130706431 127.0.0.1 50000 typ host",
                }))
                .unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(sig_reply.status, 200);

        let poll_route = find_route(realms_routes.routes(), "GET", "/launcher/v1/p2p/control");
        let poll_reply = realms_routes
            .handle(RouteCall {
                route: poll_route,
                params: vec![],
                query: "afterSequence=0&waitMs=0".to_owned(),
                request: RequestInfo::default(),
                caller: Caller::Launcher(AccountId(HOST_ACC.to_owned())),
                body: vec![],
            })
            .await
            .unwrap();
        assert_eq!(poll_reply.status, 200);
        let polled: Value = serde_json::from_str(&poll_reply.body.0).unwrap();
        let msgs = polled["messages"].as_array().unwrap();
        assert!(msgs.iter().any(|m| m["messageType"] == "ICE_SIGNAL"));

        // 9. Host consumes the joiner's admission ticket via AdmissionRoutes.
        let consume_route = find_route(
            admission_routes.routes(),
            "POST",
            "/mystic/v1/hunts/:sessionId/tickets/consume",
        );
        let consume_reply = admission_routes
            .handle(RouteCall {
                route: consume_route,
                params: vec![("sessionId".to_owned(), session_id.clone())],
                query: String::new(),
                request: RequestInfo {
                    headers: vec![(HOST_CAPABILITY_HEADER.to_owned(), host_cap_2.clone())],
                    ..RequestInfo::default()
                },
                caller: Caller::Anonymous,
                body: serde_json::to_vec(&json!({
                    "ticket": join_ticket,
                    "authenticatedSteamId": joiner_peer_id,
                    "bridgeInstanceId": bridge_instance_id,
                    "connectionHandle": "conn-1",
                }))
                .unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(consume_reply.status, 200);
        let consumed: Value = serde_json::from_str(&consume_reply.body.0).unwrap();
        assert_eq!(consumed["admitted"], true);

        // 10. Joiner marks local route ready via RealmsRoutes.
        let route_ready = find_route(
            realms_routes.routes(),
            "POST",
            "/launcher/v1/p2p/hunts/:sessionId/route-ready",
        );
        let rr_reply = realms_routes
            .handle(RouteCall {
                route: route_ready,
                params: vec![("sessionId".to_owned(), session_id.clone())],
                query: String::new(),
                request: RequestInfo::default(),
                caller: Caller::Launcher(AccountId(JOIN_ACC.to_owned())),
                body: serde_json::to_vec(&json!({
                    "generation": 1,
                    "localPort": 19777,
                }))
                .unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(rr_reply.status, 200);

        // 11. Host checks GetRealmHostingStatus and stops the hub cleanly.
        let status_route = find_route(realms_routes.routes(), "GET", "/launcher/v1/realm-hosting/status");
        let st_reply = realms_routes
            .handle(RouteCall {
                route: status_route,
                params: vec![],
                query: String::new(),
                request: RequestInfo::default(),
                caller: Caller::Launcher(AccountId(HOST_ACC.to_owned())),
                body: vec![],
            })
            .await
            .unwrap();
        assert_eq!(st_reply.status, 200);
        let st_json: Value = serde_json::from_str(&st_reply.body.0).unwrap();
        assert_eq!(st_json["ramsgate"]["sessionId"], session_id);
        assert_eq!(st_json["ramsgate"]["admittedPlayers"], 1);

        let stop_hub_route = find_route(realms_routes.routes(), "POST", "/launcher/v1/realm-hosting/hubs/stop");
        let stop_reply = realms_routes
            .handle(RouteCall {
                route: stop_hub_route,
                params: vec![],
                query: String::new(),
                request: RequestInfo::default(),
                caller: Caller::Launcher(AccountId(HOST_ACC.to_owned())),
                body: serde_json::to_vec(&json!({ "hubKind": "RAMSGATE" })).unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(stop_reply.status, 200);
        let stop_json: Value = serde_json::from_str(&stop_reply.body.0).unwrap();
        assert_eq!(stop_json["stopped"], true);
    }
}
