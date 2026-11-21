// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The hosting and joining flows against a scripted transport (spec §8.2 item 7 and the additions): files, bytes,
//! environment, failure cleanup, stopping, snapshots and the pumps.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use rifts_host::{
    CLEARED_VARIABLES, DISABLE_COOP_FLAG_FILE, DefaultHostBackend, HostError, HostedRole, HubLimits, LAUNCH_VARIABLES,
    LocalRealmHosting, LocalSession, LocalState, NoopServerSpawner, PeerListEntry, ReadyState, RealmHosting,
    SESSION_FILES, SessionResult, Signal, TransportAbi, wire_session_id,
};
use rifts_seam::{AccountId, HubKind, SessionId, poll_once};

use common::{FakeTransport, RecordingSpawner, TestBackend, session, wait_until};

const HUNT: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV@local";
const JOIN: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAW@local";
const KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

struct Rig {
    data: tempfile::TempDir,
    game: tempfile::TempDir,
    transport: Arc<FakeTransport>,
    backend: Arc<TestBackend>,
    spawner: Arc<RecordingSpawner>,
    hosting: Option<LocalRealmHosting>,
}

impl Rig {
    fn new(backend: TestBackend) -> Self {
        let data = tempfile::tempdir().unwrap();
        let game = tempfile::tempdir().unwrap();
        let transport = Arc::new(FakeTransport::new());
        let backend = Arc::new(backend);
        let spawner = Arc::new(RecordingSpawner::default());
        let hosting = LocalRealmHosting::with_backend_and_spawner(
            data.path(),
            game.path(),
            transport.clone(),
            backend.clone(),
            spawner.clone(),
        );
        Self {
            data,
            game,
            transport,
            backend,
            spawner,
            hosting: Some(hosting),
        }
    }

    fn hosting(&self) -> &LocalRealmHosting {
        self.hosting.as_ref().unwrap()
    }

    fn sessions_dir(&self) -> std::path::PathBuf {
        self.data.path().join("p2p-sessions")
    }

    fn folders(&self) -> usize {
        std::fs::read_dir(self.sessions_dir()).map_or(0, Iterator::count)
    }
}

fn account() -> AccountId {
    AccountId("acct-1".to_owned())
}

fn read(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap()
}

fn common_lines(wire: &str, hunt_definition: &str) -> String {
    format!(
        "key_id=local-key-1\nsession_id={wire}\nparty_id=party-{wire}\nparty_revision=1\n\
         hunt_definition_id={hunt_definition}\ngeneration=1\nbuild=647472\nbridge_protocol=1\nvirtual_port=1\n\
         host_peer_id=100001\nbridge_instance_id=bridge-{wire}\nmetadata_nonce=0123456789abcdef0123456789abcdef\n"
    )
}

const ICE_LINES: &str =
    "ice_stun_urls=stun:stun.cloudflare.com:3478\nice_turn_urls=turn:127.0.0.1:3478?transport=udp\n";
const TURN_SECRETS: &str = "turn_username=1700000000:local\nturn_credential=local-turn-secret\n";

/// The 32 hex digits a host flow drew, from its launch id.
fn draw(rig: &Rig, session: &SessionId) -> String {
    let environment = rig.hosting().session_launch_environment(session).unwrap();
    environment["RUSTIC_RIFTS_GAMESERVER_LAUNCH_ID"]
        .strip_prefix("host-")
        .unwrap()
        .to_owned()
}

#[test]
fn hosting_end_to_end_with_the_default_backend() {
    let rig = Rig::new(TestBackend::default());
    let hosting = rig.hosting();
    assert_eq!(poll_once(hosting.coop_available()).unwrap(), Ok(true));
    std::fs::write(rig.data.path().join(DISABLE_COOP_FLAG_FILE), "").unwrap();
    assert_eq!(poll_once(hosting.coop_available()).unwrap(), Ok(false));
    assert_eq!(
        poll_once(hosting.host_hunt(account(), session(HUNT))).unwrap(),
        Err(HostError::Transport(
            "co-op transport is disabled or not ready".to_owned()
        ))
    );
    std::fs::remove_file(rig.data.path().join(DISABLE_COOP_FLAG_FILE)).unwrap();
    for dir in [rig.data.path().join("RusticRifts"), rig.game.path().to_owned()] {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(DISABLE_COOP_FLAG_FILE), "").unwrap();
        assert_eq!(poll_once(hosting.coop_available()).unwrap(), Ok(false));
        std::fs::remove_file(dir.join(DISABLE_COOP_FLAG_FILE)).unwrap();
    }
    assert_eq!(poll_once(hosting.coop_available()).unwrap(), Ok(true));

    let limits = HubLimits {
        max_players: 16,
        max_servers: 2,
        afk_seconds: 300,
    };
    let hub = poll_once(hosting.start_hub(account(), HubKind::Ramsgate, limits))
        .unwrap()
        .unwrap();
    assert_eq!(hub.role, HostedRole::Hub(HubKind::Ramsgate));
    assert!(hub.hosting);
    assert_eq!(hub.state, LocalState::Running);
    assert_eq!(hub.generation, 1);
    let hub_dir = hosting.session_directory(&hub.session).unwrap();
    assert_eq!(hub_dir.parent().unwrap(), rig.sessions_dir());
    let name = hub_dir.file_name().unwrap().to_string_lossy().into_owned();
    let wire = wire_session_id(&hub.session);
    let suffix = name.strip_prefix(&format!("{wire}-1-")).unwrap();
    assert!(suffix.len() == 16 && suffix.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
    for file in SESSION_FILES {
        let path = hub_dir.join(file);
        assert!(path.is_file(), "{file}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(std::fs::metadata(&hub_dir).unwrap().permissions().mode() & 0o777, 0o700);
    }
    let environment = hosting.session_launch_environment(&hub.session).unwrap();
    assert_eq!(environment["RUSTIC_RIFTS_AUTH_MODE"], "HUNT_CAPABILITY");
    assert_eq!(environment["RUSTIC_RIFTS_BIND_ADDRESS"], "127.0.0.1");
    assert_eq!(environment["RUSTIC_RIFTS_HOSTED_ROLE"], "HUB");
    assert_eq!(environment["RUSTIC_RIFTS_MAX_PLAYERS"], "16");
    assert_eq!(environment["RUSTIC_RIFTS_HUB_AFK_SECONDS"], "300");
    assert_eq!(environment["RUSTIC_RIFTS_SESSION_ID"], wire);
    assert_eq!(environment["RUSTIC_RIFTS_LAUNCHER_PID"], std::process::id().to_string());
    for cleared in CLEARED_VARIABLES {
        assert!(!environment.contains_key(cleared));
    }
    assert!(!environment.contains_key("RUSTIC_RIFTS_TRIALS_WEEK"));
    let x = draw(&rig, &hub.session);
    assert_eq!(
        environment["RUSTIC_RIFTS_READY_PIPE"],
        format!(r"\\.\pipe\RusticInternalServer.ready.{x}")
    );
    assert_eq!(
        environment["RUSTIC_RIFTS_GUARD_PIPE"],
        format!(r"\\.\pipe\RusticHuntGuard.events.{x}")
    );
    let (program, arguments, _) = rig.spawner.spawned.lock().unwrap()[0].clone();
    assert_eq!(Path::new(&program), rig.game.path().join("Dauntless.exe"));
    assert_eq!(
        arguments,
        [
            "-",
            "8792",
            "/Game/Maps/Ramsgate",
            "None",
            "ShatteredIsles_ReturnToRamsgate",
            "16",
            "127.0.0.1:8792",
            "-server",
            "-log",
            "-nullrhi",
            "-MULTIHOME=127.0.0.1"
        ]
    );

    let hunt = poll_once(hosting.host_hunt(account(), session(HUNT))).unwrap().unwrap();
    assert_eq!(hunt.role, HostedRole::Hunt);
    let environment = hosting.session_launch_environment(&hunt.session).unwrap();
    assert_eq!(environment["RUSTIC_RIFTS_TRIALS_WEEK"], "001");
    assert_eq!(environment["RUSTIC_RIFTS_HOSTED_ROLE"], "HUNT");
    for name in LAUNCH_VARIABLES {
        let hub_only = name == "RUSTIC_RIFTS_MAX_PLAYERS" || name == "RUSTIC_RIFTS_HUB_AFK_SECONDS";
        assert_eq!(environment.contains_key(name), !hub_only, "{name}");
    }
    assert_eq!(hosting.session_local_port(&hunt.session), Some(8791));
    assert_eq!(
        poll_once(hosting.host_hunt(account(), session(HUNT))).unwrap(),
        Err(HostError::Busy(format!("session {HUNT} is already running")))
    );

    let join = poll_once(hosting.join_hunt(account(), session(JOIN))).unwrap().unwrap();
    assert!(!join.hosting);
    assert_eq!(hosting.session_local_port(&join.session), Some(29100));
    assert_eq!(
        hosting.session_launch_environment(&join.session),
        Some(Default::default())
    );
    assert_eq!(rig.backend.inner.recorded_route(&join.session), Some(29100));
    assert_eq!(rig.backend.inner.recorded_route(&hunt.session), Some(8791));

    let all = poll_once(hosting.sessions()).unwrap().unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(rig.folders(), 3);

    let dirs: Vec<_> = [&hub, &hunt, &join]
        .iter()
        .map(|s| hosting.session_directory(&s.session).unwrap())
        .collect();
    poll_once(hosting.stop_hub(account(), HubKind::Ramsgate))
        .unwrap()
        .unwrap();
    assert!(!dirs[0].exists());
    poll_once(hosting.stop(hunt.session.clone())).unwrap().unwrap();
    assert!(!dirs[1].exists());
    poll_once(hosting.stop(join.session.clone())).unwrap().unwrap();
    assert!(!dirs[2].exists());
    assert_eq!(
        poll_once(hosting.sessions()).unwrap().unwrap(),
        Vec::<LocalSession>::new()
    );
    assert_eq!(rig.spawner.stopped.lock().unwrap().as_slice(), [77, 77]);
    assert_eq!(rig.transport.running_count(), 0);
    assert_eq!(
        poll_once(hosting.stop(hunt.session.clone())).unwrap(),
        Err(HostError::NotFound(format!("session {HUNT} is not running")))
    );
    assert_eq!(
        poll_once(hosting.stop_hub(account(), HubKind::Training)).unwrap(),
        Err(HostError::NotFound("no running hub session for that kind".to_owned()))
    );
}

#[test]
fn bridge_conf_bytes_for_hub_and_local_hunt() {
    let rig = Rig::new(TestBackend::default());
    let hosting = rig.hosting();
    let hub = poll_once(hosting.start_hub(account(), HubKind::Ramsgate, HubLimits::default()))
        .unwrap()
        .unwrap();
    let dir = hosting.session_directory(&hub.session).unwrap();
    let wire = wire_session_id(&hub.session);
    let x = draw(&rig, &hub.session);
    let expected = format!(
        "{}backend_url=https://127.0.0.1:8443\nserver_udp_port=8792\nmaximum_peers=16\nforce_relay=false\n\
         guard_pipe_name=RusticHuntGuard.events.{x}\nhunt_evidence=false\nexpected_server_pid=77\n\
         session_role=HUB\nhost_local_client=1\n{ICE_LINES}local_account_id=acct-1\n",
        common_lines(&wire, "ShatteredIsles_ReturnToRamsgate")
    );
    assert_eq!(read(&dir, "bridge.conf"), expected);
    assert_eq!(read(&dir, "transport.secret"), TURN_SECRETS);
    assert_eq!(read(&dir, "capability.secret"), format!("cap_HUB_{}", "A".repeat(35)));
    assert_eq!(read(&dir, "ticket.secret"), "rpt1.hub_host_ticket.signature");
    let open = rig.transport.host_opens.lock().unwrap()[0].clone();
    assert_eq!(open.public_key, rig.game.path().join("p2p-admission-public.xy"));
    assert_eq!(open.config, dir.join("bridge.conf"));
    assert_eq!(open.capability, dir.join("capability.secret"));
    assert_eq!(open.secrets, dir.join("transport.secret"));
    assert_eq!(open.host_ticket, Some(dir.join("ticket.secret")));

    let hunt = poll_once(hosting.host_hunt(account(), session(HUNT))).unwrap().unwrap();
    let dir = hosting.session_directory(&hunt.session).unwrap();
    let wire = wire_session_id(&hunt.session);
    let x = draw(&rig, &hunt.session);
    let expected = format!(
        "{}backend_url=https://127.0.0.1:8443\nserver_udp_port=8791\nmaximum_peers=4\nforce_relay=false\n\
         guard_pipe_name=RusticHuntGuard.events.{x}\nhunt_evidence=false\nexpected_server_pid=77\n\
         {ICE_LINES}local_account_id=acct-1\n",
        common_lines(&wire, "Hunt_Default")
    );
    assert_eq!(read(&dir, "bridge.conf"), expected);
    assert_eq!(read(&dir, "ticket.secret"), "rpt1.ticket.signature");
}

#[test]
fn bridge_conf_bytes_for_away_hunt_and_join() {
    let rig = Rig::new(TestBackend {
        away: true,
        ..TestBackend::default()
    });
    *rig.transport.host_line.lock().unwrap() =
        "HOST_READY localHost=127.0.0.1 localPort=0 transport=ice routePolicy=AUTOMATIC".to_owned();
    let hosting = rig.hosting();
    let hunt = poll_once(hosting.host_hunt(account(), session(HUNT))).unwrap().unwrap();
    let dir = hosting.session_directory(&hunt.session).unwrap();
    let wire = wire_session_id(&hunt.session);
    let x = draw(&rig, &hunt.session);
    let expected = format!(
        "{}backend_url=https://127.0.0.1:8443\nserver_udp_port=8791\nmaximum_peers=4\nforce_relay=false\n\
         guard_pipe_name=RusticHuntGuard.events.{x}\nhunt_evidence=false\nexpected_server_pid=77\n\
         host_local_client=0\n{ICE_LINES}local_account_id=acct-1\n",
        common_lines(&wire, "Hunt_Default")
    );
    assert_eq!(read(&dir, "bridge.conf"), expected);
    assert_eq!(rig.transport.host_opens.lock().unwrap()[0].host_ticket, None);
    assert_eq!(hosting.session_local_port(&hunt.session), Some(0));
    assert!(
        rig.backend.routes.lock().unwrap().is_empty(),
        "no route-ready without a local client"
    );

    let join = poll_once(hosting.join_hunt(account(), session(JOIN))).unwrap().unwrap();
    let dir = hosting.session_directory(&join.session).unwrap();
    let wire = wire_session_id(&join.session);
    let expected = format!(
        "{}local_udp_port=0\nforce_relay=false\n{ICE_LINES}local_account_id=acct-1\nlocal_peer_id=200002\n\
         host_account_id=host-account-1\n",
        common_lines(&wire, "Hunt_Default")
    );
    assert_eq!(read(&dir, "bridge.conf"), expected);
    assert_eq!(
        read(&dir, "transport.secret"),
        format!("{TURN_SECRETS}peer=host-account-1,100001,{KEY}\n")
    );
    assert_eq!(read(&dir, "capability.secret"), "");
    assert_eq!(read(&dir, "ticket.secret"), "rpt1.joiner_ticket.signature");
    let open = rig.transport.join_opens.lock().unwrap()[0].clone();
    assert_eq!(open.ticket, dir.join("ticket.secret"));
    assert_eq!(open.config, dir.join("bridge.conf"));
    assert_eq!(open.secrets, dir.join("transport.secret"));
}

#[test]
fn failures_after_the_folder_leave_nothing_behind() {
    let rig = Rig::new(TestBackend::default());
    let hosting = rig.hosting();
    rig.transport.fail_host_open.store(true, Ordering::SeqCst);
    assert_eq!(
        poll_once(hosting.host_hunt(account(), session(HUNT))).unwrap(),
        Err(HostError::Transport("fake host_open refused".to_owned()))
    );
    assert_eq!(rig.folders(), 0);
    assert_eq!(rig.spawner.stopped.lock().unwrap().as_slice(), [77]);
    rig.transport.fail_host_open.store(false, Ordering::SeqCst);

    *rig.transport.host_ready_state.lock().unwrap() = Some(ReadyState::TimedOut);
    assert_eq!(
        poll_once(hosting.host_hunt(account(), session(HUNT))).unwrap(),
        Err(HostError::Transport(
            "transport timed out waiting for HOST_READY".to_owned()
        ))
    );
    *rig.transport.host_ready_state.lock().unwrap() = Some(ReadyState::Ended);
    assert_eq!(
        poll_once(hosting.host_hunt(account(), session(HUNT))).unwrap(),
        Err(HostError::Transport(
            "transport session ended before HOST_READY".to_owned()
        ))
    );
    *rig.transport.host_ready_state.lock().unwrap() = None;
    *rig.transport.host_line.lock().unwrap() =
        "HOST_READY localHost=127.0.0.2 localPort=8791 transport=ice routePolicy=AUTOMATIC".to_owned();
    assert_eq!(
        poll_once(hosting.host_hunt(account(), session(HUNT))).unwrap(),
        Err(HostError::Transport(
            "the co-op transport module did not return a verified loopback route".to_owned()
        ))
    );
    assert_eq!(rig.folders(), 0);
    assert_eq!(rig.transport.running_count(), 0, "every opened session was stopped");
    assert_eq!(rig.spawner.stopped.lock().unwrap().len(), 4);

    *rig.transport.join_line.lock().unwrap() = "ROUTE_READY localHost=127.0.0.1 transport=ice".to_owned();
    assert!(matches!(
        poll_once(hosting.join_hunt(account(), session(JOIN))).unwrap(),
        Err(HostError::Transport(_))
    ));
    *rig.transport.join_line.lock().unwrap() = "SOMETHING_ELSE".to_owned();
    let join = poll_once(hosting.join_hunt(account(), session(JOIN))).unwrap().unwrap();
    assert_eq!(hosting.session_local_port(&join.session), Some(29001));
    assert_eq!(rig.folders(), 1);
}

#[test]
fn ended_sessions_are_seen_and_stop_reports_a_forgotten_handle() {
    let rig = Rig::new(TestBackend::default());
    let hosting = rig.hosting();
    let hunt = poll_once(hosting.host_hunt(account(), session(HUNT))).unwrap().unwrap();
    let dir = hosting.session_directory(&hunt.session).unwrap();
    let handle = rifts_host::Handle(1);
    rig.transport.end(handle, 5);
    let all = poll_once(hosting.sessions()).unwrap().unwrap();
    assert_eq!(all[0].state, LocalState::Ended(SessionResult::LeaseFailed));
    assert!(!dir.exists(), "a snapshot cleans an ended session's folder");
    assert_eq!(poll_once(hosting.sessions()).unwrap().unwrap().len(), 1);
    rig.transport.forget(handle);
    assert_eq!(
        poll_once(hosting.stop(hunt.session.clone())).unwrap(),
        Err(HostError::NotFound("unknown session handle 1".to_owned()))
    );
    assert!(poll_once(hosting.sessions()).unwrap().unwrap().is_empty());
}

#[test]
fn dropping_the_hosting_stops_every_session() {
    let mut rig = Rig::new(TestBackend::default());
    let hosting = rig.hosting();
    poll_once(hosting.host_hunt(account(), session(HUNT))).unwrap().unwrap();
    poll_once(hosting.join_hunt(account(), session(JOIN))).unwrap().unwrap();
    assert_eq!(rig.folders(), 2);
    rig.hosting = None;
    assert_eq!(rig.folders(), 0);
    assert_eq!(rig.transport.running_count(), 0);
}

#[test]
fn join_waits_in_slices_while_pumping_signals() {
    let rig = Rig::new(TestBackend {
        signalling: true,
        ..TestBackend::default()
    });
    let hosting = rig.hosting();
    rig.transport.join_timeouts.store(4, Ordering::SeqCst);
    rig.transport.outgoing.lock().unwrap().push_back(Signal {
        to_account: AccountId("host-account-1".to_owned()),
        kind: "DESCRIPTION".to_owned(),
        data: "a=ice-ufrag:x\r\n".to_owned(),
    });
    rig.backend.inbox.lock().unwrap().push((
        AccountId("host-account-1".to_owned()),
        Signal {
            to_account: AccountId(String::new()),
            kind: "CANDIDATE".to_owned(),
            data: "a=candidate:1".to_owned(),
        },
    ));
    let join = poll_once(hosting.join_hunt(account(), session(JOIN))).unwrap().unwrap();
    let waits = rig.transport.wait_timeouts.lock().unwrap().clone();
    assert_eq!(waits.len(), 5);
    assert!(waits.iter().all(|w| (1..=50).contains(w)), "{waits:?}");
    assert_eq!(rig.backend.sent.lock().unwrap()[0].kind, "DESCRIPTION");
    assert_eq!(
        rig.transport.submitted.lock().unwrap()[0],
        (
            "host-account-1".to_owned(),
            "CANDIDATE".to_owned(),
            "a=candidate:1".to_owned()
        )
    );
    rig.transport
        .events
        .lock()
        .unwrap()
        .push_back("PEER_ADMITTED account=x".to_owned());
    assert!(wait_until(Duration::from_secs(5), || rig
        .transport
        .events
        .lock()
        .unwrap()
        .is_empty()));
    poll_once(hosting.stop(join.session)).unwrap().unwrap();
}

#[test]
fn join_times_out_after_the_whole_deadline() {
    let rig = Rig::new(TestBackend {
        signalling: true,
        ..TestBackend::default()
    });
    rig.transport.join_timeouts.store(u32::MAX, Ordering::SeqCst);
    let hosting = LocalRealmHosting::with_backend_and_spawner(
        rig.data.path(),
        rig.game.path(),
        rig.transport.clone(),
        rig.backend.clone(),
        Arc::new(NoopServerSpawner::new()),
    )
    .with_wait_ready_timeout_ms(200);
    let started = std::time::Instant::now();
    assert_eq!(
        poll_once(hosting.join_hunt(account(), session(JOIN))).unwrap(),
        Err(HostError::Transport(
            "transport timed out waiting for ROUTE_READY".to_owned()
        ))
    );
    assert!(started.elapsed() >= Duration::from_millis(200));
    assert_eq!(rig.folders(), 0);
    assert_eq!(rig.transport.running_count(), 0);
}

#[test]
fn the_host_pump_keeps_the_roster() {
    let rig = Rig::new(TestBackend {
        signalling: true,
        ..TestBackend::default()
    });
    let hosting = rig.hosting();
    let entry = |account: &str, peer: &str| PeerListEntry {
        account_id: account.to_owned(),
        peer_id: peer.to_owned(),
        transport_key: KEY.to_owned(),
    };
    *rig.backend.peers.lock().unwrap() = vec![
        entry("member-1", "300003"),
        entry("acct-1", "100001"),
        entry("bad account", "1"),
    ];
    let hub = poll_once(hosting.start_hub(account(), HubKind::Training, HubLimits::default()))
        .unwrap()
        .unwrap();
    assert!(wait_until(Duration::from_secs(5), || !rig
        .transport
        .added
        .lock()
        .unwrap()
        .is_empty()));
    assert_eq!(
        rig.transport.added.lock().unwrap()[0],
        ("member-1".to_owned(), 300_003, KEY.to_owned())
    );
    *rig.backend.peers.lock().unwrap() = Vec::new();
    assert!(wait_until(Duration::from_secs(5), || !rig
        .transport
        .removed
        .lock()
        .unwrap()
        .is_empty()));
    assert_eq!(
        rig.transport.removed.lock().unwrap()[0],
        ("member-1".to_owned(), "roster".to_owned())
    );
    assert_eq!(
        rig.transport.added.lock().unwrap().len(),
        1,
        "the host's own account is never added"
    );
    poll_once(hosting.stop(hub.session)).unwrap().unwrap();
}

#[test]
fn the_default_backend_is_deterministic() {
    let backend = DefaultHostBackend::new();
    let a = AccountId("acct-1".to_owned());
    let (spec, config) =
        rifts_host::HostBackend::start_hub(&backend, &a, HubKind::Ramsgate, &HubLimits::default()).unwrap();
    let h = "acct-1"
        .bytes()
        .fold(0_u64, |h, b| h.wrapping_mul(131).wrapping_add(u64::from(b)));
    assert_eq!(
        spec.common.session_id.0.ulid,
        rifts_seam::Ulid::from_parts(1_792_000_000_000, (1_u128 << 64) | u128::from(h))
    );
    assert_eq!(spec.common.session_id.0.node.as_str(), "local");
    assert_eq!(spec.capability.expose().len(), 43);
    assert_eq!(config.relay_expires_at().as_deref(), Some("2026-10-28T00:00:00Z"));
    let (user, credential) = config.relay_turn_credential().unwrap();
    assert_eq!(
        (user.as_str(), credential.expose()),
        ("1700000000:local", "local-turn-secret")
    );
    assert!(
        rifts_host::HostBackend::start_hub(
            &backend,
            &a,
            HubKind::Ramsgate,
            &HubLimits {
                max_players: 0,
                ..HubLimits::default()
            }
        )
        .is_err()
    );
    assert_eq!(
        rifts_host::HostBackend::stop_hub(&backend, &a, HubKind::Ramsgate).unwrap(),
        Some(spec.common.session_id)
    );
    assert_eq!(
        rifts_host::HostBackend::stop_hub(&backend, &a, HubKind::Ramsgate).unwrap(),
        None
    );
    let fake = FakeTransport::new();
    assert_eq!(fake.abi_version().unwrap(), 3);
}
