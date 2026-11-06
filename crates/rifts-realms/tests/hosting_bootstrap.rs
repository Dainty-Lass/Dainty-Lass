// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! One realm hub end to end: a realm, its Ramsgate hub started, host ready and route ready, a member routed in,
//! admitted through rifts-admission with a ticket, its route ready, the hosting status, and the stop.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::kit::{T0, kit};
use common::{node, targets};
use rifts_admission::{
    Admission, AdmissionService, ConsumeTicket, Es256Tickets, HostCapability, KeyGenerator, KeyId,
    MemoryAdmissionStore, P256KeyGenerator,
};
use rifts_realms::capabilities::{HostCapabilities, SignedUser, runtime_identity};
use rifts_realms::control::ControlChannel;
use rifts_realms::directory::{DirectoryParts, RealmDirectory};
use rifts_realms::input::{CreateRealmInput, JoinInput};
use rifts_realms::leases::{Hosting, HostingParts, HostingRuntime, HostingSettings};
use rifts_realms::lifecycle::RealmsRecovery;
use rifts_realms::ports::{Clock, DefaultOperations, NoGuardSessions, QuietLog};
use rifts_realms::realm::{DEFAULT_REALM_LIMITS, JoinPolicy};
use rifts_realms::selection::derive_ice_peer_id;
use rifts_realms::{RealmsService, RealmsServiceParts};
use rifts_seam::{
    AccountId, CapabilityScope, HuntRouteContext, HuntRouteOutcome, NoCore, RealmsHooks, Secret, UnixMillis,
};
use serde_json::{Map, Value, json};

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

fn tickets() -> Es256Tickets {
    let pair = P256KeyGenerator.generate(KeyId("test-key-1".into())).unwrap();
    let settings: BTreeMap<String, String> = pair
        .env_lines()
        .expose()
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    Es256Tickets::from_settings(&settings).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_realm_hub_end_to_end() {
    for target in targets() {
        let k = kit(target);
        let db = k.target.db.clone();
        let clock: Arc<dyn Clock> = k.clock.clone();
        let realm = k
            .directory
            .create_realm(
                "host-1",
                &CreateRealmInput {
                    name: None,
                    host_public_key: "A".repeat(24),
                    join_policy: JoinPolicy::Public,
                    listed: true,
                    version: None,
                },
            )
            .await
            .unwrap()
            .realm
            .info
            .realm_id;
        let join = JoinInput {
            name: Some("hosty".into()),
            invite_code: None,
            known_host_key: None,
        };
        assert!(k.directory.join("friend-1", &join).await.unwrap().joined);

        let settings: BTreeMap<String, String> = [("P2P_HUNTS_ENABLED", "true"), ("RUSTIC_RIFTS_REALMS_HOSTING", "on")]
            .iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
            .collect();
        let capabilities = Arc::new(HostCapabilities::new(db.clone()));
        let store = Arc::new(MemoryAdmissionStore::default());
        let tickets = Arc::new(tickets());
        let hosting = Arc::new(
            Hosting::new(HostingParts {
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
            })
            .with_runtime(HostingRuntime {
                capabilities: capabilities.clone(),
                admission_store: store.clone(),
                tickets: tickets.clone(),
                core: Arc::new(NoCore),
                backend_url: "http://127.0.0.1:9100".into(),
                host_server_udp_port: 9000,
                realms_enabled: true,
                node: node(),
            }),
        );

        // The hub starts with a self ticket for the host, who also plays.
        let boot = hosting
            .start_realm_hub("host-1", &object(json!({ "hubKind": "RAMSGATE" })))
            .await
            .unwrap();
        let keys: Vec<&str> = boot.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(&keys[..4], ["keyId", "sessionId", "partyId", "partyRevision"]);
        assert_eq!(keys.last(), Some(&"hubAfkSeconds"));
        assert_eq!(boot["keyId"], "test-key-1");
        assert_eq!(boot["sessionRole"], "HUB");
        assert_eq!(boot["hubKind"], "RAMSGATE");
        assert_eq!(
            (boot["serverUdpPort"].clone(), boot["maxPlayers"].clone()),
            (json!(9000), json!(16))
        );
        assert!(boot["hostTicket"].as_str().is_some_and(|t| t.starts_with("rpt1.")));
        assert_eq!(boot["hostPeerId"], derive_ice_peer_id("host-1"));
        let session = boot["sessionId"].as_str().unwrap().to_owned();
        let secret = boot["capability"].as_str().unwrap().to_owned();
        assert_eq!(secret.len(), 43);

        let now = UnixMillis(T0);
        let ready_cap = capabilities
            .check(Some(&secret), &session, CapabilityScope::HostReady, now)
            .await
            .unwrap();
        let ready = hosting.mark_host_ready(&ready_cap, &Map::new()).await.unwrap();
        assert_eq!(ready["state"], "HOST_READY");
        let routed = hosting
            .mark_local_route_ready("host-1", &session, &object(json!({ "localBridgePort": 7777 })))
            .await
            .unwrap();
        assert_eq!(routed["state"], "RUNNING");

        // A member is routed into the hub and told to join.
        let context = HuntRouteContext {
            game_mode: "CITY".into(),
            hunt_id: "ShatteredIsles_ReturnToRamsgate".into(),
            player: AccountId("friend-1".into()),
            party: None,
            party_members: Vec::new(),
            party_revision: None,
            build_id: None,
            is_hub: true,
        };
        k.directory.bind_presence("friend-1", Some(&realm)).await.unwrap();
        let service = RealmsService::new(RealmsServiceParts {
            directory: Some(Arc::new(RealmDirectory::new(DirectoryParts {
                db: db.clone(),
                accounts: Arc::new(common::kit::accounts()),
                events: None,
                clock: clock.clone(),
                limiter: Arc::new(common::kit::NoLimit),
                ids: Arc::new(common::kit::CountingIds::default()),
                limits: DEFAULT_REALM_LIMITS,
                log: Arc::new(QuietLog),
            }))),
            capabilities: Some(capabilities.clone()),
            hosting: Some(hosting.clone()),
            clock: clock.clone(),
            core: Arc::new(NoCore),
            policy: HostingSettings::from_settings(&settings).policy,
            realms_enabled: true,
            node: node(),
        });
        let outcome = service.route_hunt(context).await.unwrap();
        assert_eq!(outcome, HuntRouteOutcome::Handled { result: true });
        let control = ControlChannel::new(db.clone(), clock.clone());
        let polled = control.poll("friend-1", None, Some("0")).await.unwrap();
        assert!(polled.messages.iter().any(|m| m.message_type == "REALM_HUB_JOIN"));

        // Admission: the member's join bootstrap carries a ticket the host's transport consumes once.
        let admission = Admission::new(
            tickets.clone(),
            store.clone(),
            hosting.clone(),
            Some(Secret::new("k".repeat(32))),
            Arc::new(|| UnixMillis(T0)),
        );
        let seam = rifts_realms::ids::seam_session_id(&session, &node()).unwrap();
        let join_boot = admission
            .create_join_bootstrap(AccountId("friend-1".into()), seam.clone())
            .await
            .unwrap();
        let join_json: Value = serde_json::from_str(&join_boot.to_json()).unwrap();
        assert_eq!(join_json["sessionRole"], "HUB");
        let runtime_cap = capabilities.resolve_runtime(Some(&secret), now).await.unwrap().unwrap();
        let identity = runtime_identity(&runtime_cap, SignedUser::Absent, None).unwrap();
        let runtime = capabilities.runtime_auth(&runtime_cap, &identity).unwrap();
        let host = HostCapability::from_runtime(&runtime).unwrap();
        let consumed = admission
            .consume_ticket(
                host,
                ConsumeTicket {
                    ticket: join_boot.ticket.clone(),
                    authenticated_peer: derive_ice_peer_id("friend-1"),
                    bridge_instance_id: runtime.bridge_instance.clone().unwrap(),
                    connection_handle: "conn-1".into(),
                    request_id: "req-1".into(),
                },
            )
            .await
            .unwrap();
        assert!(consumed.to_json().contains(r#""admitted":true"#));

        let member = hosting
            .mark_local_route_ready("friend-1", &session, &object(json!({ "localBridgePort": 7778 })))
            .await
            .unwrap();
        assert_eq!(member["localBridgePort"], 7778);
        let status = hosting.get_realm_hosting_status("host-1").await.unwrap();
        assert_eq!(status["ramsgate"]["admittedPlayers"], 2);
        assert_eq!(status["ramsgate"]["sessionId"], session.as_str());
        assert_eq!(status["realmName"], "hosty");
        let stopped = hosting
            .stop_realm_hub("host-1", &object(json!({ "hubKind": "RAMSGATE" })))
            .await
            .unwrap();
        assert_eq!(stopped["stopped"], true);
        let again = hosting
            .stop_realm_hub("host-1", &object(json!({ "hubKind": "RAMSGATE" })))
            .await
            .unwrap();
        assert_eq!(
            again,
            json!({ "stopped": false, "hubKind": "RAMSGATE", "sessionId": null })
        );
    }
}
