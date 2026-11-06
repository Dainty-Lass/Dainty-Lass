// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The pure rules, the host PC's files, the result codes and the stub (spec §8.2 items 2, 3, 6, 8 and the additions).

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

use rifts_host::session::{HUNT_JOIN_LEAVE_MAX, RELAY_REFRESH_MARGIN_SECONDS};
use rifts_host::{
    CapacityDecision, FileHostState, FilesFor, GameTrafficWatch, HostError, HostState, HostedRole, HubLimits,
    IcePeerEntry, IceServerEntry, LaunchEnvironmentSpec, PeerDiff, PeerEvent, PeerListEntry, PeerLive, RealmHosting,
    RelayGrant, SessionLive, SessionResult, Stub, TransportAbi, TransportConfig, TransportLoader, WantedPeer,
    apply_peer_line, build_launch_environment, capacity_decision, default_max_servers, hunt_join_leave_outcome,
    ice_files, is_local_admission, parse_ice_ready, parse_peer_event, peer_diff, relay_refresh_due, relay_restart_due,
    restart_allowed, restart_backoff, safe_account, safe_config_value, safe_credential, safe_key, safe_peer_id,
    safe_url, sanitize_central_log_line, scheduled_restart_due, training_should_stop, trials_week, udp_stats_local_rx,
    wanted_peers, windows_command_line, wire_session_id,
};
use rifts_seam::{AccountId, Component, HubKind, Secret, SessionId, Unimplemented, poll_once};

const KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const UNSAFE_CONFIG: &str = "the backend returned an unsafe co-op transport configuration";

#[test]
fn session_result_codes() {
    let table = [
        (0, SessionResult::Clean),
        (2, SessionResult::BadInput),
        (3, SessionResult::Setup),
        (4, SessionResult::RefusedAdmission),
        (5, SessionResult::LeaseFailed),
        (-1, SessionResult::UnknownHandle),
        (1, SessionResult::Other(1)),
        (9, SessionResult::Other(9)),
    ];
    for (code, result) in table {
        assert_eq!(SessionResult::from_code(code), result);
    }
}

#[test]
fn error_texts() {
    assert_eq!(
        HostError::HashMismatch {
            expected: [0; 32],
            actual: [1; 32]
        }
        .to_string(),
        "transport SHA-256 mismatch"
    );
    assert_eq!(
        HostError::MissingExport("X").to_string(),
        "transport is missing export X"
    );
    assert_eq!(
        HostError::UnsupportedAbi(2).to_string(),
        "transport speaks ABI version 2; expected 3"
    );
    assert_eq!(HostError::Busy("b".into()).to_string(), "b");
    let unimplemented = Unimplemented::new(Component::Host, "transport.load");
    assert_eq!(HostError::from(unimplemented).to_string(), unimplemented.to_string());
}

#[test]
fn stub_refuses_everything() {
    let refused = |operation| Err::<(), _>(HostError::Unimplemented(Unimplemented::new(Component::Host, operation)));
    assert_eq!(
        Stub.load(Path::new("x"), [0; 32]).map(|_| ()),
        refused("transport.load")
    );
    assert_eq!(Stub.abi_version().map(|_| ()), refused("transport.abi_version"));
    assert_eq!(
        Stub.update_relay_credential(rifts_host::Handle(1), "u", &Secret::new("c".into())),
        refused("transport.update_relay_credential")
    );
    assert_eq!(
        poll_once(Stub.coop_available()).unwrap().map(|_| ()),
        refused("coop_available")
    );
    assert_eq!(poll_once(Stub.sessions()).unwrap().map(|_| ()), refused("sessions"));
    assert_eq!(Stub.hub_limits().map(|_| ()), refused("state.hub_limits"));
    assert_eq!(
        Stub.set_control_cursor(&AccountId("a".into()), 1),
        refused("state.set_control_cursor")
    );
}

#[test]
fn host_state_limits_round_trip_and_damage() {
    let dir = tempfile::tempdir().unwrap();
    let state = FileHostState::for_server(dir.path(), "srv-1");
    assert_eq!(state.server_id(), "srv-1");
    assert_eq!(state.config_dir(), dir.path());
    assert_eq!(state.hub_limits().unwrap(), None);
    let limits = HubLimits {
        max_players: 24,
        max_servers: 3,
        afk_seconds: 900,
    };
    state.save_hub_limits(&limits).unwrap();
    assert_eq!(state.hub_limits().unwrap(), Some(limits));
    let path = dir.path().join("realm-host-limits.json");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "{\n  \"version\": 1,\n  \"servers\": {\n    \"srv-1\": {\n      \"maxPlayers\": 24,\n      \
         \"maxServers\": 3,\n      \"afkSeconds\": 900\n    }\n  }\n}"
    );
    FileHostState::for_server(dir.path(), "aaa")
        .save_hub_limits(&HubLimits::default())
        .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.find("\"aaa\"").unwrap() < text.find("\"srv-1\"").unwrap(),
        "servers sorted"
    );

    let refusals = [
        (
            HubLimits {
                max_players: 0,
                ..limits
            },
            "the player limit must be from 1 to 64",
        ),
        (
            HubLimits {
                max_players: 65,
                ..limits
            },
            "the player limit must be from 1 to 64",
        ),
        (
            HubLimits {
                max_servers: 9,
                ..limits
            },
            "the server limit must be from 1 to 8",
        ),
        (
            HubLimits {
                afk_seconds: 7201,
                ..limits
            },
            "the AFK limit must be from 0 to 7200 seconds",
        ),
    ];
    for (bad, text) in refusals {
        assert_eq!(
            state.save_hub_limits(&bad),
            Err(HostError::InvalidInput(text.to_owned()))
        );
    }
    assert_eq!(
        FileHostState::for_server(dir.path(), "").save_hub_limits(&limits),
        Err(HostError::InvalidInput("invalid server id".to_owned()))
    );

    std::fs::write(&path, "{ not json").unwrap();
    assert_eq!(state.hub_limits().unwrap(), None);
    state.save_hub_limits(&limits).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("realm-host-limits.json.bad")).unwrap(),
        "{ not json"
    );
    assert_eq!(state.hub_limits().unwrap(), Some(limits));
    assert!(!dir.path().join("realm-host-limits.json.tmp").exists());

    std::fs::write(&path, "{\"maxPlayers\":8,\"maxServers\":1}").unwrap();
    assert_eq!(
        state.hub_limits().unwrap(),
        Some(HubLimits {
            max_players: 8,
            max_servers: 1,
            afk_seconds: 0
        })
    );
    std::fs::write(&path, "{\"servers\":{\"srv-1\":{\"maxPlayers\":0,\"maxServers\":1}}}").unwrap();
    assert_eq!(state.hub_limits().unwrap(), None);
}

#[test]
fn host_state_cursors_per_account_never_move_back() {
    let dir = tempfile::tempdir().unwrap();
    let state = FileHostState::new(dir.path());
    assert_eq!(state.server_id(), "default");
    let a = AccountId("acct-a".to_owned());
    let b = AccountId("acct-b".to_owned());
    assert_eq!(state.control_cursor(&a).unwrap(), None);
    state.set_control_cursor(&a, 10).unwrap();
    state.set_control_cursor(&b, 3).unwrap();
    state.set_control_cursor(&a, 4).unwrap();
    assert_eq!(state.control_cursor(&a).unwrap(), Some(10));
    assert_eq!(state.control_cursor(&b).unwrap(), Some(3));
    assert_eq!(
        state.control_cursor(&AccountId(String::new())),
        Err(HostError::InvalidInput("account id must not be empty".to_owned()))
    );
    assert_eq!(
        state.set_control_cursor(&AccountId("x".repeat(129)), 1),
        Err(HostError::InvalidInput("invalid account id".to_owned()))
    );
    let path = dir.path().join("p2p-host-settings.json");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "{\n  \"controlCursors\": {\n    \"acct-a\": 10,\n    \"acct-b\": 3\n  }\n}"
    );
    std::fs::write(&path, "{\"control_cursors\":{\"acct-a\":7}}").unwrap();
    assert_eq!(state.control_cursor(&a).unwrap(), Some(7));
    std::fs::write(&path, "{oops").unwrap();
    assert_eq!(state.control_cursor(&a).unwrap(), None);
    state.set_control_cursor(&a, 1).unwrap();
    assert!(dir.path().join("p2p-host-settings.json.bad").exists());
}

fn session(text: &str) -> SessionId {
    text.parse().unwrap()
}

#[test]
fn wire_ids_and_value_checks() {
    let id = session("01ARZ3NDEKTSV4RRFFQ69G5FAV@local");
    let wire = wire_session_id(&id);
    assert_eq!(wire.len(), 36);
    assert_eq!(
        wire,
        format!("{:032x}", id.0.ulid.0)
            .chars()
            .enumerate()
            .fold(String::new(), |mut s, (i, c)| {
                if [8, 12, 16, 20].contains(&i) {
                    s.push('-');
                }
                s.push(c);
                s
            })
    );
    assert_eq!(
        wire_session_id(&session("00000000000000000000000001@x")),
        "00000000-0000-0000-0000-000000000001"
    );

    let unsafe_data = HostError::InvalidInput("the backend returned unsafe player-host bootstrap data".into());
    assert_eq!(safe_config_value("ok", 2), Ok("ok"));
    for bad in ["", "abcd", "a\nb", "a\rb", "a\0b"] {
        assert_eq!(safe_config_value(bad, 3), Err(unsafe_data.clone()), "{bad:?}");
    }
    assert!(safe_account("a-b_c:1") && !safe_account("") && !safe_account("a b") && !safe_account(&"a".repeat(65)));
    assert!(safe_peer_id("1") && safe_peer_id("18446744073709551615"));
    for bad in ["", "0", "01", "18446744073709551616", "1a", "123456789012345678901"] {
        assert!(!safe_peer_id(bad), "{bad}");
    }
    assert!(safe_key(KEY) && !safe_key(&KEY[1..]) && !safe_key(&format!("{}=", &KEY[1..])));
    assert!(safe_url("stun:a:1") && safe_url("turn:a") && safe_url("turns:a"));
    assert!(
        !safe_url("http://a")
            && !safe_url("turn:a,b")
            && !safe_url("turn:a b")
            && !safe_url(&format!("turn:{}", "a".repeat(252)))
    );
    assert!(
        safe_credential("x") && !safe_credential("") && !safe_credential("a b") && !safe_credential(&"a".repeat(513))
    );
}

fn peer(account: &str, id: &str) -> IcePeerEntry {
    IcePeerEntry {
        account_id: account.to_owned(),
        peer_id: id.to_owned(),
        transport_key: KEY.to_owned(),
    }
}

fn config(peers: Vec<IcePeerEntry>) -> TransportConfig {
    TransportConfig {
        generation: 1,
        local_account_id: "local".to_owned(),
        local_peer_id: "200".to_owned(),
        host_account_id: "host".to_owned(),
        host_peer_id: "100".to_owned(),
        ice_servers: vec![
            IceServerEntry {
                urls: vec!["stun:s1:3478".into(), "turn:ignored".into()],
                username: None,
                credential: None,
            },
            IceServerEntry {
                urls: vec!["turn:t1".into(), "turns:t2".into(), "stun:not-taken".into()],
                username: Some("user".into()),
                credential: Some("pass".into()),
            },
            IceServerEntry {
                urls: vec!["turn:later".into()],
                username: Some("u2".into()),
                credential: Some("p2".into()),
            },
            IceServerEntry {
                urls: vec!["stun:s2".into()],
                username: Some("only-user".into()),
                credential: None,
            },
        ],
        peers,
        relay: None,
    }
}

#[test]
fn ice_files_for_each_role() {
    let joiner = ice_files(&config(vec![peer("host", "100")]), FilesFor::Joiner, "100").unwrap();
    assert_eq!(
        joiner.config,
        "ice_stun_urls=stun:s1:3478,stun:s2\nice_turn_urls=turn:t1,turns:t2\nlocal_account_id=local\n\
         local_peer_id=200\nhost_account_id=host\n"
    );
    assert_eq!(
        joiner.secrets.expose(),
        format!("turn_username=user\nturn_credential=pass\npeer=host,100,{KEY}\n")
    );

    let peers = vec![peer("m1", "301"), peer("m2", "302"), peer("m3", "303")];
    let host = ice_files(&config(peers.clone()), FilesFor::HuntHost, "100").unwrap();
    assert_eq!(
        host.config,
        "ice_stun_urls=stun:s1:3478,stun:s2\nice_turn_urls=turn:t1,turns:t2\nlocal_account_id=local\n"
    );
    assert_eq!(
        host.secrets.expose(),
        format!("turn_username=user\nturn_credential=pass\npeer=m1,301,{KEY}\npeer=m2,302,{KEY}\npeer=m3,303,{KEY}\n")
    );
    let mut four = peers.clone();
    four.push(peer("m4", "304"));
    assert!(ice_files(&config(four.clone()), FilesFor::AwayHuntHost, "100").is_ok());
    let hub = ice_files(&config(vec![peer("bad account", "x")]), FilesFor::HubHost, "100").unwrap();
    assert_eq!(hub.secrets.expose(), "turn_username=user\nturn_credential=pass\n");

    let mut no_turn = config(Vec::new());
    no_turn.ice_servers[1].urls = vec!["stun:x".into()];
    let files = ice_files(&no_turn, FilesFor::HubHost, "100").unwrap();
    assert_eq!(
        files.config,
        "ice_stun_urls=stun:s1:3478,stun:s2\nlocal_account_id=local\n"
    );
    assert_eq!(files.secrets.expose(), "", "no credential without a TURN url");
    let mut bare = config(Vec::new());
    bare.ice_servers.clear();
    assert_eq!(
        ice_files(&bare, FilesFor::HubHost, "100").unwrap().config,
        "local_account_id=local\n"
    );

    let refused = |c: &TransportConfig, f: FilesFor| {
        assert_eq!(
            ice_files(c, f, "100"),
            Err(HostError::InvalidInput(UNSAFE_CONFIG.to_owned()))
        );
    };
    refused(&config(peers.clone()), FilesFor::Joiner);
    refused(&config(vec![peer("host", "101")]), FilesFor::Joiner);
    refused(&config(Vec::new()), FilesFor::Joiner);
    refused(&config(four.clone()), FilesFor::HuntHost);
    let mut five = four;
    five.push(peer("m5", "305"));
    refused(&config(five), FilesFor::AwayHuntHost);
    refused(&config(vec![peer("local", "301")]), FilesFor::HuntHost);
    refused(&config(vec![peer("m1", "0")]), FilesFor::HuntHost);
    let mut short_key = peer("m1", "301");
    short_key.transport_key.pop();
    refused(&config(vec![short_key]), FilesFor::HuntHost);
    assert!(ice_files(&config(Vec::new()), FilesFor::HuntHost, "999").is_err());
    let mut bad = config(Vec::new());
    bad.local_account_id = "a b".into();
    refused(&bad, FilesFor::HubHost);
    let mut bad = config(Vec::new());
    bad.local_peer_id = "0".into();
    refused(&bad, FilesFor::HubHost);
    let mut bad = config(Vec::new());
    bad.ice_servers[0].urls.push("http://x".into());
    refused(&bad, FilesFor::HubHost);
    let mut bad = config(Vec::new());
    bad.ice_servers[2].credential = Some("has space".into());
    refused(&bad, FilesFor::HubHost);
}

#[test]
fn transport_config_json() {
    let text = r#"{"generation":2,"localAccountId":"a","localPeerId":"1","hostAccountId":"b","hostPeerId":"2",
        "iceServers":[{"urls":["stun:x"]}],"peers":[{"accountId":"b","peerId":"2","transportKey":"k"}],
        "relay":{"allowed":false,"expiresAt":"t"}}"#;
    let config: TransportConfig = serde_json::from_str(text).unwrap();
    assert_eq!(config.ice_servers[0].username, None);
    assert_eq!(
        config.relay,
        Some(RelayGrant {
            allowed: false,
            expires_at: Some("t".into())
        })
    );
    assert_eq!(config.relay_expires_at(), None);
    assert_eq!(config.relay_turn_credential(), None);
    let peers: Vec<PeerListEntry> =
        serde_json::from_str(r#"[{"accountId":"a","peerId":"5","transportKey":"k"}]"#).unwrap();
    assert_eq!(peers[0].peer_id, "5");
    let mut unsafe_first = config;
    unsafe_first.ice_servers = vec![
        IceServerEntry {
            urls: vec![],
            username: Some("a b".into()),
            credential: Some("c".into()),
        },
        IceServerEntry {
            urls: vec![],
            username: Some("ok".into()),
            credential: Some("ok".into()),
        },
    ];
    assert_eq!(
        unsafe_first.relay_turn_credential(),
        None,
        "the first credentialed server decides"
    );
}

#[test]
fn ready_lines() {
    let host = "HOST_READY localHost=127.0.0.1 localPort=8791 transport=ice routePolicy=AUTOMATIC";
    assert_eq!(parse_ice_ready(host, "HOST_READY", false, true), Ok(8791));
    let route = "ROUTE_READY localHost=127.0.0.1 localPort=29100 transport=ice routePolicy=AUTOMATIC route=direct";
    assert_eq!(parse_ice_ready(route, "ROUTE_READY", false, true), Ok(29100));
    let relay = "ROUTE_READY localHost=127.0.0.1 localPort=9 transport=ice routePolicy=RELAY_ONLY route=relay";
    assert_eq!(parse_ice_ready(relay, "ROUTE_READY", true, true), Ok(9));
    let away = "HOST_READY localHost=127.0.0.1 localPort=0 transport=ice routePolicy=RELAY_ONLY";
    assert_eq!(parse_ice_ready(away, "HOST_READY", true, false), Ok(0));

    let error = |line: &str, state: &str, relay: bool, local: bool| match parse_ice_ready(line, state, relay, local) {
        Err(HostError::Transport(text)) => text,
        other => panic!("{other:?}"),
    };
    let cases = [
        (
            host,
            "ROUTE_READY",
            false,
            true,
            "the co-op transport module returned an unexpected readiness state",
        ),
        (
            "",
            "HOST_READY",
            false,
            true,
            "the co-op transport module returned an unexpected readiness state",
        ),
        (
            "HOST_READY bare",
            "HOST_READY",
            false,
            true,
            "the co-op transport module returned a malformed local route",
        ),
        (
            "HOST_READY a=",
            "HOST_READY",
            false,
            true,
            "the co-op transport module returned a malformed local route",
        ),
        (
            "HOST_READY =b",
            "HOST_READY",
            false,
            true,
            "the co-op transport module returned a malformed local route",
        ),
        (
            "HOST_READY a=1 a=2",
            "HOST_READY",
            false,
            true,
            "the co-op transport module returned a malformed local route",
        ),
        (
            "HOST_READY localHost=0.0.0.0 transport=ice",
            "HOST_READY",
            false,
            true,
            "the co-op transport module did not return a verified loopback route",
        ),
        (
            "HOST_READY localHost=127.0.0.1 transport=udp",
            "HOST_READY",
            false,
            true,
            "the co-op transport module did not return a verified loopback route",
        ),
        (
            host,
            "HOST_READY",
            true,
            true,
            "the co-op transport module returned a route outside the requested transport policy",
        ),
        (
            "ROUTE_READY localHost=127.0.0.1 localPort=9 transport=ice routePolicy=RELAY_ONLY route=direct",
            "ROUTE_READY",
            true,
            true,
            "the co-op transport module returned a route outside the requested transport policy",
        ),
        (
            host,
            "HOST_READY",
            false,
            false,
            "the co-op transport module returned an invalid local route",
        ),
        (
            "HOST_READY localHost=127.0.0.1 localPort=0 transport=ice routePolicy=AUTOMATIC",
            "HOST_READY",
            false,
            true,
            "the co-op transport module returned an invalid local route",
        ),
        (
            "HOST_READY localHost=127.0.0.1 localPort=70000 transport=ice routePolicy=AUTOMATIC",
            "HOST_READY",
            false,
            true,
            "the co-op transport module returned an invalid local route",
        ),
    ];
    for (line, state, relay, local, text) in cases {
        assert_eq!(error(line, state, relay, local), text, "{line}");
    }
}

#[test]
fn peer_lines_and_live_view() {
    assert_eq!(
        parse_peer_event("PEER_ADMITTED account=peer-acct-1 route=direct"),
        Some(PeerEvent::Admitted {
            account: "peer-acct-1".into(),
            route: Some("direct".into())
        })
    );
    assert_eq!(
        parse_peer_event("PEER_REMOVED account=a reason=roster admitted=1 junk"),
        Some(PeerEvent::Removed {
            account: "a".into(),
            reason: Some("roster".into()),
            admitted: true
        })
    );
    assert_eq!(
        parse_peer_event("PEER_REMOVED account=a admitted=true account=b"),
        Some(PeerEvent::Removed {
            account: "b".into(),
            reason: None,
            admitted: false
        })
    );
    assert_eq!(
        parse_peer_event("PEER_ADD_REJECTED account=a reason=invalid_peer"),
        Some(PeerEvent::AddRejected {
            account: "a".into(),
            reason: Some("invalid_peer".into())
        })
    );
    assert_eq!(parse_peer_event("PEER_CONNECTED route=direct"), None);
    assert_eq!(parse_peer_event("PEER_WHO account=a"), None);
    assert!(is_local_admission(
        "ADMISSION_ACCEPTED role=host peer=100001 source=local"
    ));
    assert!(!is_local_admission("ADMISSION_ACCEPTED role=host source=remote"));
    assert!(!is_local_admission("X role=host source=local"));

    let mut live = SessionLive::default();
    let lines = [
        ("ADMISSION_ACCEPTED role=host peer=100001 source=local", "t0"),
        ("PEER_CONNECTED account=a route=direct", "t1"),
        ("PEER_ADMITTED account=a route=direct", "t2"),
        ("PEER_ADMITTED account=b", "t3"),
        ("garbage", "t4"),
        ("PEER_LOST account=b reason=timeout", "t5"),
        ("PEER_ADMITTED account=a route=relay", "t6"),
        ("PEER_ADMITTED account=c", "t7"),
        ("PEER_REMOVED account=c reason=roster admitted=1", "t8"),
    ];
    for (line, now) in lines {
        apply_peer_line(&mut live, line, now);
    }
    assert_eq!(live.local_since.as_deref(), Some("t0"));
    assert_eq!(
        live.peers,
        BTreeMap::from([(
            "a".to_owned(),
            PeerLive {
                route: Some("relay".into()),
                since: "t6".into()
            }
        )])
    );
}

#[test]
fn roster_rules() {
    let entry = |account: &str, peer: &str, key: &str| PeerListEntry {
        account_id: account.into(),
        peer_id: peer.into(),
        transport_key: key.into(),
    };
    let entries = [
        entry("m1", "301", KEY),
        entry("host", "100", KEY),
        entry("m2", "0", KEY),
        entry("m1", "302", KEY),
    ];
    let (wanted, refused) = wanted_peers(&entries, Some("host"));
    assert_eq!(
        wanted,
        vec![WantedPeer {
            account_id: "m1".into(),
            peer_id: 301,
            transport_key: KEY.into()
        }]
    );
    assert_eq!(refused, 2);
    let many: Vec<PeerListEntry> = (0..70)
        .map(|i| entry(&format!("m{i}"), &format!("{}", i + 1), KEY))
        .collect();
    assert_eq!(wanted_peers(&many, None).0.len(), 64);

    let linked: HashSet<String> = ["z".to_owned(), "m1".to_owned(), "b".to_owned()].into();
    let wanted = vec![
        WantedPeer {
            account_id: "m2".into(),
            peer_id: 2,
            transport_key: KEY.into(),
        },
        WantedPeer {
            account_id: "m1".into(),
            peer_id: 1,
            transport_key: KEY.into(),
        },
        WantedPeer {
            account_id: "a".into(),
            peer_id: 3,
            transport_key: KEY.into(),
        },
    ];
    let diff = peer_diff(&linked, &wanted);
    assert_eq!(
        diff,
        PeerDiff {
            add: vec![wanted[0].clone(), wanted[2].clone()],
            remove: vec!["b".into(), "z".into()]
        }
    );
}

#[test]
fn hosted_server_launch() {
    assert_eq!(trials_week("Arena_MatchmakerHunt_Easy_001"), Some("001"));
    assert_eq!(trials_week("Arena_MatchmakerHunt_Elite_123"), Some("123"));
    for bad in [
        "Arena_MatchmakerHunt_Medium_001",
        "Arena_MatchmakerHunt_Hard_01",
        "Arena_MatchmakerHunt_Hard_0012",
        "Hunt_Easy_001",
        "Arena_MatchmakerHunt_Hard_00a",
    ] {
        assert_eq!(trials_week(bad), None, "{bad}");
    }
    let program = Path::new(r"C:\Games\Dauntless\Dauntless.exe");
    let arguments: Vec<String> = [
        "-",
        "",
        "a b",
        "tab\there",
        r#"say "hi""#,
        r"back\slash",
        "trail\\",
        r#"q\"x"#,
        "end space\\",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    assert_eq!(
        windows_command_line(program, &arguments),
        "\"C:\\Games\\Dauntless\\Dauntless.exe\" - \"\" \"a b\" \"tab\there\" \"say \\\"hi\\\"\" back\\slash trail\\ \
         q\\\\\\\"x \"end space\\\\\""
    );
    assert_eq!(
        windows_command_line(program, &[r"a\\".into(), r"b c\\".into()]),
        r#""C:\Games\Dauntless\Dauntless.exe" a\\ "b c\\\\""#
    );
}

#[test]
fn launch_environment() {
    let spec = LaunchEnvironmentSpec {
        capability: Secret::new("cap".into()),
        role: HostedRole::Hunt,
        session_id: session("01ARZ3NDEKTSV4RRFFQ69G5FAV@local"),
        generation: 3,
        launch_id: "host-x".into(),
        ready_pipe: "r".into(),
        control_pipe: "c".into(),
        guard_pipe: "g".into(),
        hunt_evidence: true,
        guard_completion_mode: "FIRST_DEFEAT".into(),
        runtime_profile: "production".into(),
        launcher_pid: 12,
        max_players: Some(8),
        hub_afk_seconds: Some(60),
        trials_week: Some("002".into()),
    };
    let inherited = [
        ("PATH", "p"),
        ("rustic_rifts_allow_damage_debug", "1"),
        ("Rustic_Rifts_Session_Id", "stale"),
        ("RUSTIC_RIFTS_OTHER", "kept"),
    ]
    .map(|(k, v)| (k.to_owned(), v.to_owned()));
    let environment = build_launch_environment(&spec, inherited);
    assert_eq!(environment["PATH"], "p");
    assert_eq!(environment["RUSTIC_RIFTS_OTHER"], "kept");
    assert!(!environment.contains_key("rustic_rifts_allow_damage_debug"));
    assert!(!environment.contains_key("Rustic_Rifts_Session_Id"));
    assert_eq!(environment["RUSTIC_RIFTS_HUNT_GUARD_EVENTS"], "1");
    assert_eq!(environment["RUSTIC_RIFTS_SESSION_GENERATION"], "3");
    assert_eq!(environment["RUSTIC_RIFTS_TRIALS_WEEK"], "002");
    assert!(
        !environment.contains_key("RUSTIC_RIFTS_MAX_PLAYERS"),
        "hub limits are for hubs only"
    );
    let hub = build_launch_environment(
        &LaunchEnvironmentSpec {
            role: HostedRole::Hub(HubKind::Training),
            hunt_evidence: false,
            ..spec
        },
        [],
    );
    assert_eq!(hub["RUSTIC_RIFTS_MAX_PLAYERS"], "8");
    assert_eq!(hub["RUSTIC_RIFTS_HUB_AFK_SECONDS"], "60");
    assert_eq!(hub["RUSTIC_RIFTS_HUNT_GUARD_EVENTS"], "0");
    assert_eq!(hub.len(), 17);
}

#[test]
fn central_log_lines() {
    assert_eq!(
        sanitize_central_log_line("p2p ticket=secret123 capability=cap456 ok=1"),
        "p2p ticket=[redacted] capability=[redacted] ok=1"
    );
    assert_eq!(
        sanitize_central_log_line("  a   MyToken  b=x=y Auth_Password=z "),
        "a [redacted] b=x=y Auth_Password=[redacted]"
    );
    let long = format!("{} tail", "a".repeat(3000));
    assert_eq!(sanitize_central_log_line(&long), "a".repeat(2048));
    // The stop is at 2048 bytes, the cut at 2048 characters.
    let wide = format!("{} tail", "é".repeat(1500));
    assert_eq!(sanitize_central_log_line(&wide), "é".repeat(1500));
}

#[test]
fn timers_and_capacity() {
    let start = Instant::now();
    let mut watch = GameTrafficWatch::new(start);
    watch.observe("UDP_STATS role=joiner localRx=10", start);
    watch.observe("UDP_STATS role=joiner localRx=10", start + Duration::from_secs(2));
    assert_eq!(
        hunt_join_leave_outcome(start, &watch, start + Duration::from_secs(3)),
        None
    );
    assert_eq!(
        hunt_join_leave_outcome(start, &watch, start + Duration::from_secs(8)),
        Some("game_left")
    );
    watch.observe("UDP_STATS localRx=11", start + Duration::from_secs(8));
    assert_eq!(watch.idle(start + Duration::from_secs(9)), Duration::from_secs(1));
    watch.restart(start + Duration::from_secs(9));
    assert_eq!(watch.idle(start), Duration::ZERO);
    let mut busy = GameTrafficWatch::new(start);
    let late = start + HUNT_JOIN_LEAVE_MAX;
    busy.observe("UDP_STATS localRx=1", late);
    assert_eq!(hunt_join_leave_outcome(start, &busy, late), Some("timeout"));
    assert_eq!(udp_stats_local_rx("UDP_STATS localRx=5 localRx=6"), Some(5));
    assert_eq!(udp_stats_local_rx("OTHER localRx=5"), None);

    assert!(!relay_refresh_due(None, None));
    assert!(relay_refresh_due(
        Some(RELAY_REFRESH_MARGIN_SECONDS),
        Some(Duration::ZERO)
    ));
    assert!(relay_refresh_due(Some(43_200), None));
    assert!(!relay_refresh_due(Some(43_200), Some(Duration::from_secs(1799))));
    assert!(relay_refresh_due(Some(43_200), Some(Duration::from_secs(1800))));
    assert!(!relay_refresh_due(Some(43_201), None));
    assert!(relay_restart_due(Some(300)) && !relay_restart_due(Some(301)) && !relay_restart_due(None));

    assert_eq!(default_max_servers(16), 6);
    assert_eq!(default_max_servers(2), 1);
    assert_eq!(default_max_servers(4), 1);
    assert_eq!(default_max_servers(10), 4);
    assert_eq!(capacity_decision(4096, 0, 1), CapacityDecision::Accept);
    assert_eq!(
        capacity_decision(4095, 0, 1),
        CapacityDecision::Refuse { reason: "HOST_BUSY" }
    );
    assert_eq!(
        capacity_decision(8192, 2, 2),
        CapacityDecision::Refuse { reason: "HOST_BUSY" }
    );

    assert_eq!(restart_backoff(0), Duration::from_secs(5));
    assert_eq!(restart_backoff(1), Duration::from_secs(15));
    assert_eq!(restart_backoff(7), Duration::from_secs(60));
    let now = start + Duration::from_secs(1000);
    let attempts = [start, now - Duration::from_secs(10), now - Duration::from_secs(20)];
    assert!(restart_allowed(&attempts, now));
    let five = [now; 5];
    assert!(!restart_allowed(&five, now));
    assert_eq!(rifts_host::next_idle_since(Some(start), 1, now), None);
    assert_eq!(rifts_host::next_idle_since(Some(start), 0, now), Some(start));
    assert_eq!(rifts_host::next_idle_since(None, 0, now), Some(now));
    assert!(training_should_stop(Some(now - Duration::from_secs(300)), now));
    assert!(!training_should_stop(Some(now), now) && !training_should_stop(None, now));
    assert!(scheduled_restart_due(Duration::from_secs(72_001), false));
    assert!(!scheduled_restart_due(Duration::from_secs(72_001), true));
    assert!(!scheduled_restart_due(Duration::from_secs(72_000), false));
}

#[test]
fn common_config_lines_check_each_value() {
    let common = rifts_host::CommonBootstrapConfig {
        key_id: "k".into(),
        session_id: session("00000000000000000000000001@x"),
        party_id: "p".into(),
        party_revision: 2,
        hunt_definition_id: "h".into(),
        generation: 3,
        build: 4,
        bridge_protocol: 5,
        virtual_port: 6,
        host_peer_id: "7".into(),
        bridge_instance_id: "b".into(),
        metadata_nonce: "n".into(),
        force_relay: true,
        guard_completion_mode: "m".into(),
    };
    assert_eq!(
        common.to_config_lines().unwrap(),
        "key_id=k\nsession_id=00000000-0000-0000-0000-000000000001\nparty_id=p\nparty_revision=2\nhunt_definition_id=h\n\
         generation=3\nbuild=4\nbridge_protocol=5\nvirtual_port=6\nhost_peer_id=7\nbridge_instance_id=b\nmetadata_nonce=n\n"
    );
    for change in [
        |c: &mut rifts_host::CommonBootstrapConfig| c.key_id = "k".repeat(33),
        |c: &mut rifts_host::CommonBootstrapConfig| c.guard_completion_mode = String::new(),
        |c: &mut rifts_host::CommonBootstrapConfig| c.host_peer_id = "1".repeat(21),
        |c: &mut rifts_host::CommonBootstrapConfig| c.metadata_nonce = "a\nb".into(),
    ] {
        let mut bad = common.clone();
        change(&mut bad);
        assert!(matches!(bad.to_config_lines(), Err(HostError::InvalidInput(_))));
    }
}
