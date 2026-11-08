//! Realm hosting and joining tests (`realms` feature, RF7): drives `Controller::realm_host`,
//! `realm_join`, `realm_stop`, and `realm_get` against a scripted [`TransportAbi`] (the MIT `rustic-rifts-host` links its
//! transport in, RD27, so there is no C ABI to fake), checks RD9 (no injection or loader `.ini` written by realms,
//! and leftover loader files refused by both `realm_host` and `play`), and verifies that the account key in
//! `KeyStore` never reaches `rustic-rifts-host`, `p2p-sessions/`, or the dedicated server launch environment.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use rustic_rifts_host::{
    ADMISSION_PUBLIC_KEY_FILE, DefaultHostBackend, Handle, HostBackend, HostError, HostOpen, JoinOpen,
    NoopServerSpawner, ReadyState, SESSION_FILES, ServerSpawner, SessionResult, Signal, TRANSPORT_DLL, TransportAbi,
    TransportLoader, sha256_file,
};
use rustic_rifts_seam::{AccountId, PeerId, Secret};

use super::*;
use dainty_launcher_kit::bridge::types::{RealmKind, RealmOutcome, RealmState};
use dainty_launcher_kit::controller::realms::RealmsSeams;
use dainty_launcher_kit::platform::loader_session::LOADER_INI;

/// The launcher still looks for the transport file beside the game and checks its digest; any bytes do.
fn write_transport_placeholder(out_path: &Path) {
    std::fs::write(out_path, b"placeholder transport, never loaded (RD27)").unwrap();
}

/// A transport whose sessions are ready at once: a host answers `HOST_READY`, a join `ROUTE_READY` and one offer to
/// the host's account.
#[derive(Default)]
struct FakeTransport {
    next_handle: AtomicI32,
    is_join: AtomicBool,
    signal_pending: AtomicBool,
}

fn known(handle: Handle) -> Result<(), HostError> {
    if handle.0 > 0 {
        Ok(())
    } else {
        Err(HostError::NotFound(format!("unknown session handle {}", handle.0)))
    }
}

impl FakeTransport {
    fn open(&self) -> Handle {
        Handle(self.next_handle.fetch_add(1, Ordering::SeqCst) + 1)
    }
}

impl TransportAbi for FakeTransport {
    fn abi_version(&self) -> Result<u32, HostError> {
        Ok(3)
    }
    fn initialize(&self) -> Result<(), HostError> {
        Ok(())
    }
    fn networking_ready(&self) -> Result<bool, HostError> {
        Ok(true)
    }
    fn peer_id(&self) -> Result<PeerId, HostError> {
        Ok(PeerId(0xCAFE_1234_5678))
    }
    fn shutdown(&self) -> Result<(), HostError> {
        Ok(())
    }
    fn host_open(&self, _: &HostOpen) -> Result<Handle, HostError> {
        self.is_join.store(false, Ordering::SeqCst);
        Ok(self.open())
    }
    fn join_open(&self, _: &JoinOpen) -> Result<Handle, HostError> {
        self.is_join.store(true, Ordering::SeqCst);
        self.signal_pending.store(true, Ordering::SeqCst);
        Ok(self.open())
    }
    fn wait_ready(&self, handle: Handle, _: u32) -> Result<ReadyState, HostError> {
        known(handle)?;
        let line = if self.is_join.load(Ordering::SeqCst) {
            "ROUTE_READY localHost=127.0.0.1 localPort=29100 transport=ice routePolicy=AUTOMATIC route=direct"
        } else {
            "HOST_READY localHost=127.0.0.1 localPort=8791 transport=ice routePolicy=AUTOMATIC"
        };
        Ok(ReadyState::Ready(line.to_owned()))
    }
    fn session_running(&self, handle: Handle) -> Result<bool, HostError> {
        Ok(handle.0 > 0)
    }
    fn last_session_result(&self, handle: Handle) -> Result<SessionResult, HostError> {
        known(handle).map(|()| SessionResult::Clean)
    }
    fn stop_session(&self, handle: Handle) -> Result<(), HostError> {
        known(handle)
    }
    fn read_peer_event(&self, _: Handle) -> Result<Option<String>, HostError> {
        Ok(None)
    }
    fn read_diagnostic(&self, _: Handle) -> Result<Option<String>, HostError> {
        Ok(None)
    }
    fn read_signal(&self, handle: Handle) -> Result<Option<Signal>, HostError> {
        if handle.0 > 0 && self.signal_pending.swap(false, Ordering::SeqCst) {
            return Ok(Some(Signal {
                to_account: AccountId("user-slayer-1".to_owned()),
                kind: "DESCRIPTION".to_owned(),
                data: "v=0\r\na=ice-ufrag:daintylass\r\n".to_owned(),
            }));
        }
        Ok(None)
    }
    fn submit_signal(&self, handle: Handle, _: &AccountId, _: &str, _: &str) -> Result<(), HostError> {
        known(handle)
    }
    fn add_peer(&self, handle: Handle, _: &AccountId, _: PeerId, _: &Secret) -> Result<(), HostError> {
        known(handle)
    }
    fn remove_peer(&self, handle: Handle, _: &AccountId, _: &str) -> Result<(), HostError> {
        known(handle)
    }
    fn update_relay_credential(&self, handle: Handle, _: &str, _: &Secret) -> Result<(), HostError> {
        known(handle)
    }
}

struct FakeLoader;

impl TransportLoader for FakeLoader {
    fn load(&self, _: &Path, _: [u8; 32]) -> Result<Box<dyn TransportAbi>, HostError> {
        Ok(Box::new(FakeTransport::default()))
    }
}

#[tokio::test]
async fn realm_commands_host_join_stop_keep_rd9_and_never_expose_account_key() {
    let hx = Harness::new();
    let secret_key = "RRK_super_secret_account_key_never_in_realms_998877";
    let mut host = Host::new("RealmsSrv");
    host.registration = "OPEN";
    host.users
        .insert(secret_key.into(), ("Slayer".into(), "user-slayer-1".into()));
    host.runtime = Some(make_runtime(1));
    let (_shared, port) = lan(host).await;

    let inv = invite(ServerMode::Private, "127.0.0.1", port, None, "OPEN", "RealmsSrv");
    assert_eq!(hx.c.submit_invite(&inv, false, false).await, ActionResult::Ok);
    assert_eq!(hx.c.use_existing_key(secret_key).await, ActionResult::Ok);

    let game_root = hx.game();
    let win64 = game_root.join("Archon").join("Binaries").join("Win64");
    let dll_path = win64.join(TRANSPORT_DLL);
    write_transport_placeholder(&dll_path);
    std::fs::write(win64.join(ADMISSION_PUBLIC_KEY_FILE), [0x11u8; 64]).unwrap();
    let dll_sha = sha256_file(&dll_path).unwrap();

    assert_eq!(hx.c.use_game_path(game_root.to_str().unwrap()).await, ActionResult::Ok);
    assert_eq!(hx.c.verify_game().await, ActionResult::Ok);

    let spawner = Arc::new(NoopServerSpawner::new());
    hx.c.set_realms_seams(RealmsSeams {
        loader: Arc::new(FakeLoader) as Arc<dyn TransportLoader>,
        expected_sha256: Some(dll_sha),
        backend: Arc::new(DefaultHostBackend::new()) as Arc<dyn HostBackend>,
        spawner: spawner.clone() as Arc<dyn ServerSpawner>,
    });

    // Initial realm_get reports available with no sessions.
    let RealmOutcome::Ok(initial) = hx.c.realm_get().await else {
        panic!("expected RealmOutcome::Ok from realm_get");
    };
    assert!(initial.available);
    assert!(initial.active.is_none());
    assert!(initial.sessions.is_empty());

    // Host a hub.
    let RealmOutcome::Ok(after_hub) = hx.c.realm_host(RealmKind::Hub).await else {
        panic!("expected RealmOutcome::Ok from realm_host(Hub)");
    };
    assert_eq!(after_hub.sessions.len(), 1);
    assert_eq!(after_hub.active.as_ref().map(|s| s.kind), Some(RealmKind::Hub));
    assert_eq!(after_hub.active.as_ref().map(|s| s.state), Some(RealmState::Running));

    // Host a hunt alongside the hub.
    let RealmOutcome::Ok(after_hunt) = hx.c.realm_host(RealmKind::Hunt).await else {
        panic!("expected RealmOutcome::Ok from realm_host(Hunt)");
    };
    assert_eq!(after_hunt.sessions.len(), 2);
    assert_eq!(after_hunt.active.as_ref().map(|s| s.kind), Some(RealmKind::Hunt));

    // Join a hunt.
    let RealmOutcome::Ok(after_join) = hx.c.realm_join("01ARZ3NDEKTSV4RRFFQ69G5FAV@local").await else {
        panic!("expected RealmOutcome::Ok from realm_join");
    };
    assert_eq!(after_join.sessions.len(), 3);
    let joined = after_join.active.as_ref().unwrap();
    assert!(!joined.hosting);
    assert_eq!(joined.local_port, Some(29_100));

    // RD9: realm hosting and joining never write mystic_loader.ini or any DLL beside the loader.
    assert!(!win64.join(LOADER_INI).exists());
    assert!(!win64.join(LOADER_INI).exists());

    // Account key isolation: neither the session files under userData/p2p-sessions/ nor the server
    // launch environment may contain the player's account key.
    let sessions_root = hx.user.path().join("p2p-sessions");
    assert!(sessions_root.is_dir());
    let mut checked_dirs = 0usize;
    for entry in std::fs::read_dir(&sessions_root).unwrap() {
        let dir = entry.unwrap().path();
        if !dir.is_dir() {
            continue;
        }
        checked_dirs += 1;
        for file_name in SESSION_FILES {
            let content = std::fs::read_to_string(dir.join(file_name)).unwrap();
            assert!(
                !content.contains(secret_key),
                "session file {file_name} must never contain the account key"
            );
        }
    }
    assert_eq!(checked_dirs, 3);
    let env = spawner.last_environment().unwrap();
    for (k, v) in &env {
        assert!(
            !v.contains(secret_key),
            "launch variable {k} must never contain the account key"
        );
    }

    // Stop all realm sessions and verify session directories are cleaned up.
    let RealmOutcome::Ok(after_stop) = hx.c.realm_stop().await else {
        panic!("expected RealmOutcome::Ok from realm_stop");
    };
    assert!(after_stop.sessions.is_empty());
    assert!(after_stop.active.is_none());
    let remaining_dirs = std::fs::read_dir(&sessions_root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .count();
    assert_eq!(
        remaining_dirs, 0,
        "all session directories must be deleted on realm_stop"
    );

    // RD9: if a leftover mystic_loader.ini sits in Win64, both realm_host and play refuse with LoaderLeftovers.
    std::fs::write(win64.join(LOADER_INI), "[loader]\ndll=Dumper-7.dll\n").unwrap();
    assert_eq!(
        hx.c.realm_host(RealmKind::Hub).await,
        RealmOutcome::Err(LauncherError {
            code: ErrorCode::LoaderLeftovers,
            detail: Some(LOADER_INI.to_owned()),
        })
    );
    assert_eq!(
        hx.c.play().await,
        ActionResult::Err(LauncherError {
            code: ErrorCode::LoaderLeftovers,
            detail: Some(LOADER_INI.to_owned()),
        })
    );
    std::fs::remove_file(win64.join(LOADER_INI)).unwrap();

    // No SHA-256 mismatch case: the linked transport has no file to check (spec-rd27-rifts D1)
    hx.c.shutdown().await;
}

const TEST_HUB_SESSION_UUID: &str = "018f0000-0000-7000-8000-000000000001";

pub(super) fn handle_realms_route(
    host: &Shared,
    method: &hyper::Method,
    path: &str,
    key: Option<&str>,
    _body: &[u8],
) -> Option<Response<Full<Bytes>>> {
    if !path.starts_with("/launcher/v1/") && !path.starts_with("/rustic/v1/") {
        return None;
    }
    let user = {
        let h = host.lock().unwrap();
        key.and_then(|k| h.users.get(k).cloned())
    };
    if path.starts_with("/launcher/v1/") && user.is_none() {
        return Some(reply(
            401,
            json!({ "error": { "code": "AUTH_UNAUTHORIZED", "message": "Sign in required." } }),
        ));
    }
    let (_, user_id) = user.unwrap_or_else(|| ("Host".to_owned(), "user-slayer-1".to_owned()));

    match (method, path) {
        (&hyper::Method::GET, "/launcher/v1/realms/mine") => Some(reply(200, json!({ "realm": null }))),
        (&hyper::Method::POST, "/launcher/v1/realms") => Some(reply(
            201,
            json!({
                "realm": {
                    "id": "realm-1",
                    "name": "slayerrealm",
                    "joinPolicy": "PUBLIC",
                    "listed": true
                }
            }),
        )),
        (&hyper::Method::POST, "/launcher/v1/realms/mine/heartbeat") => Some(reply(200, json!({ "ok": true }))),
        (&hyper::Method::POST, "/launcher/v1/p2p/host-presence") => Some(reply(200, json!({ "eligible": true }))),
        (&hyper::Method::POST, "/launcher/v1/realm-hosting/hubs/start") => Some(reply(
            200,
            json!({
                "keyId": "local-key-1",
                "sessionId": TEST_HUB_SESSION_UUID,
                "partyId": "realm-hub:realm-1:ramsgate",
                "partyRevision": 1,
                "huntDefinitionId": "ShatteredIsles_ReturnToRamsgate",
                "generation": 1,
                "build": 647472,
                "bridgeProtocol": 1,
                "virtualPort": 1,
                "hostPeerId": "100001",
                "bridgeInstanceId": "bridge-hub-0001",
                "metadataNonce": "0123456789abcdef0123456789abcdef",
                "forceRelay": false,
                "guardCompletionMode": "NO_COMPLETION",
                "capability": "cap-hub-1",
                "hostTicket": "MPT2.hostticket",
                "serverUdpPort": 8791,
                "maximumPeers": 16,
                "hostLocalClient": true,
                "huntEvidence": false,
                "matchmakerHuntId": "ShatteredIsles_ReturnToRamsgate",
                "mapPath": "/Game/Maps/ramsgate/ramsgate_01_persistent",
                "behemothPath": "NO_BEHEMOTH",
                "expectedPlayers": "1",
                "runtimeProfile": "production",
                "maxPlayers": 16,
                "hubAfkSeconds": 600
            }),
        )),
        (&hyper::Method::GET, "/launcher/v1/realms/listed") => Some(reply(
            200,
            json!({
                "realms": [{ "id": "realm-1", "name": "slayerrealm" }]
            }),
        )),
        (&hyper::Method::POST, "/launcher/v1/realms/join") => Some(reply(200, json!({ "joined": true }))),
        (&hyper::Method::POST, "/launcher/v1/realms/presence/heartbeat") => Some(reply(200, json!({ "ok": true }))),
        (&hyper::Method::POST, "/launcher/v1/realm-hosting/hubs/peers") => Some(reply(200, json!({ "peers": [] }))),
        (&hyper::Method::POST, "/launcher/v1/realm-hosting/hubs/stop") => Some(reply(
            200,
            json!({
                "stopped": true,
                "hubKind": "RAMSGATE",
                "sessionId": TEST_HUB_SESSION_UUID
            }),
        )),
        (&hyper::Method::GET, "/launcher/v1/p2p/control") => Some(reply(
            200,
            json!({
                "messages": [{ "id": "ctrl-msg-1", "type": "REALM_HUB_STOPPED" }]
            }),
        )),
        _ => {
            if let Some(rest) = path.strip_prefix("/rustic/v1/hunts/") {
                if rest.ends_with("/host-ready") {
                    return Some(reply(
                        200,
                        json!({
                            "ready": true,
                            "sessionId": TEST_HUB_SESSION_UUID,
                            "generation": 1,
                            "state": "HOST_READY"
                        }),
                    ));
                }
                if rest.ends_with("/tickets/consume") {
                    return Some(reply(200, json!({ "admitted": true })));
                }
            }
            if let Some(rest) = path.strip_prefix("/launcher/v1/p2p/hunts/") {
                if rest.ends_with("/transport-config") {
                    let is_host = user_id == "user-slayer-1";
                    let peers = if is_host {
                        json!([])
                    } else {
                        json!([{
                            "accountId": "user-slayer-1",
                            "peerId": "100001",
                            "transportKey": "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef_"
                        }])
                    };
                    return Some(reply(
                        200,
                        json!({
                            "sessionId": TEST_HUB_SESSION_UUID,
                            "generation": 1,
                            "localAccountId": user_id,
                            "localPeerId": if is_host { "100001" } else { "100002" },
                            "hostAccountId": "user-slayer-1",
                            "hostPeerId": "100001",
                            "iceServers": [
                                { "urls": ["stun:127.0.0.1:3478"] }
                            ],
                            "peers": peers
                        }),
                    ));
                }
                if rest.ends_with("/route-ready") {
                    return Some(reply(200, json!({ "ready": true })));
                }
                if rest.ends_with("/join-bootstrap") {
                    return Some(reply(
                        200,
                        json!({
                            "keyId": "local-key-1",
                            "sessionId": TEST_HUB_SESSION_UUID,
                            "sessionRole": "HUB",
                            "partyId": "realm-hub:realm-1:ramsgate",
                            "partyRevision": 1,
                            "huntDefinitionId": "ShatteredIsles_ReturnToRamsgate",
                            "generation": 1,
                            "build": 647472,
                            "bridgeProtocol": 1,
                            "virtualPort": 1,
                            "hostPeerId": "100001",
                            "bridgeInstanceId": "bridge-hub-0001",
                            "metadataNonce": "0123456789abcdef0123456789abcdef",
                            "forceRelay": false,
                            "guardCompletionMode": "NO_COMPLETION",
                            "ticket": "MPT2.jointicket"
                        }),
                    ));
                }
                if rest.ends_with("/signal") {
                    return Some(reply(200, json!({ "accepted": true })));
                }
                if rest.ends_with("/peers") {
                    return Some(reply(200, json!({ "generation": 1, "peers": [] })));
                }
            }
            if path.starts_with("/launcher/v1/p2p/control/") && path.ends_with("/ack") {
                return Some(reply(200, json!({ "acknowledged": true })));
            }
            Some(reply(
                404,
                json!({ "error": { "code": "P2P_SESSION_NOT_FOUND", "message": "Session not found." } }),
            ))
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn server_host_backend_drives_realm_hub_and_join_across_two_launchers() {
    let host_hx = Harness::new();
    let join_hx = Harness::new();
    let host_key = "RRK_host_account_key_for_server_realms_1122334455";
    let join_key = "RRK_join_account_key_for_server_realms_6677889900";

    let host_game = host_hx.game();
    let host_win64 = host_game.join("Archon").join("Binaries").join("Win64");
    let host_dll = host_win64.join(TRANSPORT_DLL);
    write_transport_placeholder(&host_dll);
    std::fs::write(host_win64.join(ADMISSION_PUBLIC_KEY_FILE), [0x22u8; 64]).unwrap();
    let dll_bytes = std::fs::read(&host_dll).unwrap();

    let join_game = join_hx.game();
    let join_win64 = join_game.join("Archon").join("Binaries").join("Win64");
    std::fs::write(join_win64.join(TRANSPORT_DLL), &dll_bytes).unwrap();
    std::fs::write(join_win64.join(ADMISSION_PUBLIC_KEY_FILE), [0x22u8; 64]).unwrap();

    let mut srv = Host::new("LiveRealmsSrv");
    srv.registration = "OPEN";
    srv.users
        .insert(host_key.into(), ("HostSlayer".into(), "user-slayer-1".into()));
    srv.users
        .insert(join_key.into(), ("JoinSlayer".into(), "user-slayer-2".into()));
    srv.runtime = Some(make_runtime(1));
    let (shared, port) = lan(srv).await;

    let inv = invite(ServerMode::Private, "127.0.0.1", port, None, "OPEN", "LiveRealmsSrv");
    for (hx, key, game_root) in [(&host_hx, host_key, &host_game), (&join_hx, join_key, &join_game)] {
        assert_eq!(hx.c.submit_invite(&inv, false, false).await, ActionResult::Ok);
        assert_eq!(hx.c.use_existing_key(key).await, ActionResult::Ok);
        assert_eq!(hx.c.use_game_path(game_root.to_str().unwrap()).await, ActionResult::Ok);
        assert_eq!(hx.c.verify_game().await, ActionResult::Ok);
        assert_eq!(hx.c.trust_runtime().await, ActionResult::Ok);
    }

    // The live ServerHostBackend against the server; only the transport is scripted (the linked one needs a peer)
    for hx in [&host_hx, &join_hx] {
        hx.c.set_realms_seams(RealmsSeams {
            loader: Arc::new(FakeLoader) as Arc<dyn TransportLoader>,
            expected_sha256: None,
            backend: hx.c.live_host_backend(),
            spawner: Arc::new(NoopServerSpawner::new()) as Arc<dyn ServerSpawner>,
        });
    }

    // Host launcher starts a realm hub through the default ServerHostBackend.
    let outcome = host_hx.c.realm_host(RealmKind::Hub).await;
    let RealmOutcome::Ok(hosted_view) = outcome else {
        panic!("expected RealmOutcome::Ok from host_hx.c.realm_host(Hub), got {outcome:?}");
    };
    assert_eq!(hosted_view.sessions.len(), 1);
    let hub_session = hosted_view.active.as_ref().unwrap();
    assert!(hub_session.hosting);
    assert_eq!(hub_session.kind, RealmKind::Hub);
    assert_eq!(hub_session.state, RealmState::Running);

    // Join launcher joins the hosted hub using the UUID or session identifier through ServerHostBackend.
    let join_outcome = join_hx.c.realm_join(TEST_HUB_SESSION_UUID).await;
    let RealmOutcome::Ok(joined_view) = join_outcome else {
        panic!("expected RealmOutcome::Ok from join_hx.c.realm_join, got {join_outcome:?}");
    };
    assert_eq!(joined_view.sessions.len(), 1);
    let joined_session = joined_view.active.as_ref().unwrap();
    assert!(!joined_session.hosting);
    assert_eq!(joined_session.state, RealmState::Running);
    assert_eq!(joined_session.local_port, Some(29_100));
    assert_eq!(joined_session.session, hub_session.session);

    // Stop both launchers cleanly and verify all server routes were exercised.
    assert!(matches!(join_hx.c.realm_stop().await, RealmOutcome::Ok(_)));
    assert!(matches!(host_hx.c.realm_stop().await, RealmOutcome::Ok(_)));

    let seen = shared.lock().unwrap().seen.clone();
    for expected in [
        "/launcher/v1/realms/mine",
        "/launcher/v1/realms",
        "/launcher/v1/realms/mine/heartbeat",
        "/launcher/v1/p2p/host-presence",
        "/launcher/v1/realm-hosting/hubs/start",
        &format!("/rustic/v1/hunts/{TEST_HUB_SESSION_UUID}/host-ready"),
        &format!("/rustic/v1/hunts/{TEST_HUB_SESSION_UUID}/tickets/consume"),
        &format!("/launcher/v1/p2p/hunts/{TEST_HUB_SESSION_UUID}/transport-config"),
        &format!("/launcher/v1/p2p/hunts/{TEST_HUB_SESSION_UUID}/route-ready"),
        "/launcher/v1/realms/listed",
        "/launcher/v1/realms/join",
        "/launcher/v1/realms/presence/heartbeat",
        &format!("/launcher/v1/p2p/hunts/{TEST_HUB_SESSION_UUID}/join-bootstrap"),
        &format!("/launcher/v1/p2p/hunts/{TEST_HUB_SESSION_UUID}/signal"),
        "/launcher/v1/realm-hosting/hubs/stop",
        "/launcher/v1/p2p/control",
        "/launcher/v1/p2p/control/ctrl-msg-1/ack",
    ] {
        assert!(
            seen.iter().any(|p| p == expected),
            "expected server to receive {expected}, seen: {seen:?}"
        );
    }

    join_hx.c.shutdown().await;
    host_hx.c.shutdown().await;
}
