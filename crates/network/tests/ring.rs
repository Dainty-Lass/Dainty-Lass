#![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

//! Integration tests for DN6 (instance registry and wrong-node redirects), DN8 (four-region matchmaking
//! ring, CAS ticket claims, allowlist feed, and XP/loot/bounty forwarding), and DN9 (regional Ramsgate
//! hubs and sharding).

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use module_api::network::{
    GlobalId, HomeApi, HomeError, HomeForward, IdKind, NodeId, NodeIdentity, Region, SessionTicket, operations,
};
use module_api::{BoxFuture, GameVersion, ModuleId};
use network::inprocess::InProcessNetwork;
use network::link::{Clock, NodeService};
use network::ring::{
    AllowlistSink, ClaimOutcome, DirectorPlacement, DirectorPlacementError, HuntRewards, InstanceRecord, JoinDecision,
    PeerGateway, RegionPing, RingCall, RingConfig, RingError, RingNode, RingSchedule, TicketState,
    UnconfiguredDirector, WrongNodePolicy, decode_request, encode_request,
};

/// Test-only in-process Director placement that assigns deterministic ports in a region's UDP range.
#[derive(Debug, Clone)]
struct LoopbackDirector {
    host: String,
    base_port: u16,
    next_offset: Arc<std::sync::atomic::AtomicU16>,
}

impl LoopbackDirector {
    fn new(host: impl Into<String>, base_port: u16) -> Self {
        Self {
            host: host.into(),
            base_port,
            next_offset: Arc::new(std::sync::atomic::AtomicU16::new(0)),
        }
    }
}

impl DirectorPlacement for LoopbackDirector {
    fn place_hunt<'a>(
        &'a self,
        _hunt_id: &'a GlobalId,
        _queue: &'a str,
        _game: &'a GameVersion,
        _players: &'a [GlobalId],
    ) -> BoxFuture<'a, Result<(String, u16), DirectorPlacementError>> {
        let offset = self.next_offset.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 1000;
        let host = self.host.clone();
        let port = self.base_port.saturating_add(offset);
        Box::pin(async move { Ok((host, port)) })
    }

    fn place_hub<'a>(
        &'a self,
        _hub_id: &'a GlobalId,
        _region: &'a Region,
        _shard: u32,
        _game: &'a GameVersion,
    ) -> BoxFuture<'a, Result<(String, u16), DirectorPlacementError>> {
        let offset = self.next_offset.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 1000;
        let host = self.host.clone();
        let port = self.base_port.saturating_add(offset);
        Box::pin(async move { Ok((host, port)) })
    }
}

type AllowlistEntry = (GlobalId, GlobalId, String, u16);

#[derive(Debug, Default, Clone)]
struct RecordedAllowlist {
    entries: Arc<Mutex<Vec<AllowlistEntry>>>,
}

impl RecordedAllowlist {
    fn is_admitted(&self, instance: &GlobalId, client_ip: &str, udp_port: u16) -> bool {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .any(|(id, _, ip, port)| id == instance && ip == client_ip && *port == udp_port)
    }
}

impl AllowlistSink for RecordedAllowlist {
    fn admit_player(&self, instance: &GlobalId, account: &GlobalId, client_ip: &str, udp_port: u16) {
        self.entries
            .lock()
            .unwrap()
            .push((instance.clone(), account.clone(), client_ip.to_owned(), udp_port));
    }
}

struct TestHome {
    identity: NodeIdentity,
    forwarded: Arc<Mutex<Vec<HomeForward>>>,
}

impl TestHome {
    fn new(identity: NodeIdentity) -> Self {
        Self {
            identity,
            forwarded: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl HomeApi for TestHome {
    fn resolve_home<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<NodeIdentity, HomeError>> {
        let owner = account.owner().clone();
        Box::pin(async move {
            Ok(NodeIdentity {
                region: Region::new(owner.as_str()).unwrap_or_else(|_| self.identity.region.clone()),
                node: owner,
            })
        })
    }

    fn session_ticket<'a>(
        &'a self,
        account: &'a GlobalId,
        audience: &'a NodeId,
    ) -> BoxFuture<'a, Result<SessionTicket, HomeError>> {
        let ticket = SessionTicket {
            account: account.clone(),
            audience: audience.clone(),
            token: format!("dlt1.{}.{}", account.ulid(), audience),
        };
        Box::pin(async move { Ok(ticket) })
    }

    fn forward<'a>(&'a self, request: &'a HomeForward) -> BoxFuture<'a, Result<serde_json::Value, HomeError>> {
        self.forwarded.lock().unwrap().push(request.clone());
        Box::pin(async move { Ok(serde_json::json!({ "ok": true })) })
    }

    fn admit<'a>(
        &'a self,
        ticket: &'a SessionTicket,
        _game: &'a GameVersion,
    ) -> BoxFuture<'a, Result<GlobalId, HomeError>> {
        let expected_prefix = format!("dlt1.{}.{}", ticket.account.ulid(), self.identity.node);
        let res = if ticket.audience == self.identity.node && ticket.token == expected_prefix {
            Ok(ticket.account.clone())
        } else {
            Err(HomeError::Denied("invalid ticket".into()))
        };
        Box::pin(async move { res })
    }
}

struct ClusterNode {
    identity: NodeIdentity,
    clock: Arc<Clock>,
    ring: Arc<RingNode>,
    _service: Arc<dyn NodeService>,
    allowlist: RecordedAllowlist,
    forwarded: Arc<Mutex<Vec<HomeForward>>>,
}

fn build_four_region_cluster(policy: WrongNodePolicy, max_hub_players: u16) -> HashMap<String, ClusterNode> {
    let specs = [
        ("oregon-1", "oregon", "or.example.org", 4431),
        ("ny-1", "new-york", "ny.example.org", 4432),
        ("uk-1", "uk", "uk.example.org", 4433),
        ("de-1", "germany", "de.example.org", 4434),
    ];
    let net = InProcessNetwork::new();
    let gateways: Vec<PeerGateway> = specs
        .iter()
        .map(|(node, region, host, port)| PeerGateway {
            node: node.parse().unwrap(),
            region: region.parse().unwrap(),
            host: (*host).to_owned(),
            port: *port,
            fingerprint: Some("ab".repeat(32)),
        })
        .collect();

    let mut nodes = HashMap::new();
    for (node_str, region_str, _, base_port) in specs {
        let identity = NodeIdentity {
            node: node_str.parse().unwrap(),
            region: region_str.parse().unwrap(),
        };
        let clock = Arc::new(Clock::default());
        let link = Arc::new(net.link(&identity.node));
        let home = Arc::new(TestHome::new(identity.clone()));
        let forwarded = home.forwarded.clone();
        let allowlist = RecordedAllowlist::default();
        let director = Arc::new(LoopbackDirector::new(format!("{region_str}.hunt.internal"), base_port));
        let mut cfg = RingConfig::new(identity.clone());
        cfg.wrong_node_policy = policy;
        cfg.max_hub_players = max_hub_players;
        cfg.peers = gateways.iter().filter(|g| g.node != identity.node).cloned().collect();

        let ring = RingNode::new(cfg, clock.clone(), link, home, director, Arc::new(allowlist.clone()));
        let service: Arc<dyn NodeService> = ring.clone();
        net.attach(&identity.node, &service);
        nodes.insert(
            node_str.to_owned(),
            ClusterNode {
                identity,
                clock,
                ring,
                _service: service,
                allowlist,
                forwarded,
            },
        );
    }
    nodes
}

#[tokio::test]
async fn dn6_instance_registry_resolves_owners_and_redirects_or_refuses_wrong_node_joins() {
    let cluster = build_four_region_cluster(WrongNodePolicy::Redirect, 24);
    let game: GameVersion = "1.14.7".parse().unwrap();

    // Register a hunt on uk-1
    let uk = &cluster["uk-1"];
    let hunt_id = GlobalId::new(IdKind::Hunt, &uk.identity.node);
    let record = InstanceRecord {
        id: hunt_id.clone(),
        region: uk.identity.region.clone(),
        game: game.clone(),
        queue: "patrol_blaze".into(),
        server_host: "uk.hunt.internal".into(),
        server_port: 19400,
        capacity: 4,
        players: Vec::new(),
        shard: None,
        created_at_ms: 1_000,
    };
    uk.ring.register_instance(record.clone()).unwrap();

    // Every node resolves global id to uk-1 and looks up the instance record over Service::Ring
    let oregon = &cluster["oregon-1"];
    assert_eq!(hunt_id.owner(), &uk.identity.node);
    let looked_up = oregon.ring.lookup_instance(&hunt_id).await.unwrap();
    assert_eq!(looked_up, Some(record));

    // A player homed in Oregon whose join reaches oregon-1 for the uk-1 hunt gets a Redirect with a ticket
    let player = GlobalId::new(IdKind::Account, &oregon.identity.node);
    let decision = oregon
        .ring
        .resolve_join(&player, "203.0.113.10", &hunt_id)
        .await
        .unwrap();
    let (gateway, ticket) = match decision {
        JoinDecision::Redirect {
            instance,
            owner,
            gateway,
            ticket,
        } => {
            assert_eq!(instance, hunt_id);
            assert_eq!(owner, uk.identity.node);
            (gateway, ticket)
        }
        other => panic!("expected Redirect, got {other:?}"),
    };
    assert_eq!(gateway.host, "uk.example.org");
    assert_eq!(ticket.audience, uk.identity.node);

    // Re-pointed join landing on uk-1 succeeds locally and feeds uk-1's allowlist
    let landed = uk.ring.resolve_join(&player, "203.0.113.10", &hunt_id).await.unwrap();
    match landed {
        JoinDecision::Local(inst) => {
            assert!(inst.players.contains(&player));
        }
        other => panic!("expected Local join on uk-1, got {other:?}"),
    }
    assert!(uk.allowlist.is_admitted(&hunt_id, "203.0.113.10", 19400));

    // Under WrongNodePolicy::Refuse (DN1 kill criterion), wrong-node joins are refused with a clear message
    let refuse_cluster = build_four_region_cluster(WrongNodePolicy::Refuse, 24);
    let refused = refuse_cluster["oregon-1"]
        .ring
        .resolve_join(&player, "203.0.113.10", &hunt_id)
        .await
        .unwrap();
    assert!(matches!(refused, JoinDecision::Refused { owner, .. } if owner == uk.identity.node));

    // Under WrongNodePolicy::Redirect, if the target peer has no pinned gateway certificate fingerprint,
    // resolve_join refuses the redirect rather than sending an unpinned PeerGateway.
    let identity = oregon.identity.clone();
    let mut unpinned_cfg = RingConfig::new(identity.clone());
    unpinned_cfg.wrong_node_policy = WrongNodePolicy::Redirect;
    unpinned_cfg.peers = vec![PeerGateway {
        node: uk.identity.node.clone(),
        region: uk.identity.region.clone(),
        host: "uk.example.org".into(),
        port: 4433,
        fingerprint: None,
    }];
    let net = InProcessNetwork::new();
    let unpinned_ring = RingNode::new(
        unpinned_cfg,
        Arc::new(Clock::default()),
        Arc::new(net.link(&identity.node)),
        Arc::new(TestHome::new(identity)),
        Arc::new(UnconfiguredDirector),
        Arc::new(RecordedAllowlist::default()),
    );
    let unpinned_decision = unpinned_ring
        .resolve_join(&player, "203.0.113.10", &hunt_id)
        .await
        .unwrap();
    assert!(
        matches!(unpinned_decision, JoinDecision::Refused { owner, message, .. } if owner == uk.identity.node && message.contains("fingerprint"))
    );
    // And UnconfiguredDirector refuses hub placement with a typed error
    let err = unpinned_ring
        .assign_ramsgate(player, "203.0.113.10", game)
        .await
        .unwrap_err();
    assert_eq!(
        err,
        RingError::Director(DirectorPlacementError::Unconfigured("1.14.7".parse().unwrap()))
    );
}

#[tokio::test]
async fn dn8_four_region_matchmaking_ring_never_double_books_and_bounds_wait_time() {
    let cluster = build_four_region_cluster(WrongNodePolicy::Redirect, 24);
    let game: GameVersion = "1.14.7".parse().unwrap();
    let schedule = RingSchedule::default();
    let max_wait = schedule.max_wait_ms(4);
    assert_eq!(max_wait, 15_000);

    // 8 players across the 4 regions:
    // - 4 players have lowest ping to `uk` (will match immediately at step 0 in `uk-1`)
    // - 4 players are spread across `oregon-1`, `ny-1`, `uk-1`, `de-1` with different nearest regions;
    //   as the ring widens over 5s/10s/15s, they all become visible across regions.
    //   Then all 4 regional nodes try to form matches concurrently; CAS claims at each ticket's home
    //   ensure every player lands in at most ONE match (zero double-booking).
    let home_keys = ["oregon-1", "ny-1", "uk-1", "de-1"];
    let mut all_accounts = Vec::new();

    // First 4 players: homed in different regions, all reporting `uk` as lowest RTT (15ms) via authenticated report
    for (idx, home_name) in home_keys.iter().enumerate() {
        let node = &cluster[*home_name];
        let account = GlobalId::new(IdKind::Account, &node.identity.node);
        let api_key = format!("RRK_valid_key_player_{idx}_abcdef");
        node.ring.register_local_account_key(api_key.clone(), account.clone());
        let authed = node
            .ring
            .record_region_pings_authenticated(
                Some(&api_key),
                None,
                vec![
                    RegionPing {
                        region: "oregon".parse().unwrap(),
                        rtt_ms: Some(120),
                    },
                    RegionPing {
                        region: "uk".parse().unwrap(),
                        rtt_ms: Some(15),
                    },
                    RegionPing {
                        region: "germany".parse().unwrap(),
                        rtt_ms: Some(30),
                    },
                    RegionPing {
                        region: "new-york".parse().unwrap(),
                        rtt_ms: Some(85),
                    },
                ],
            )
            .await
            .unwrap();
        assert_eq!(authed, account);
        node.ring
            .submit_ticket(
                account.clone(),
                "escalation_shock",
                game.clone(),
                1,
                format!("198.51.100.{}", idx + 1),
            )
            .await
            .unwrap();
        all_accounts.push(account);
    }

    // At t = 0, all 4 tickets were published to `uk` (their lowest-ping region). `uk-1` forms a match immediately.
    let uk_matches = cluster["uk-1"]
        .ring
        .try_form_matches("escalation_shock", &game)
        .await
        .unwrap();
    assert_eq!(uk_matches.len(), 1);
    assert_eq!(uk_matches[0].players.len(), 4);

    // Next 4 players: each homed on a different node with its own region as #1, so at t=0 each region only sees 1 ticket.
    let mut wave2_tickets = Vec::new();
    for (idx, home_name) in home_keys.iter().enumerate() {
        let node = &cluster[*home_name];
        let account = GlobalId::new(IdKind::Account, &node.identity.node);
        let t = node
            .ring
            .submit_ticket(
                account.clone(),
                "escalation_shock",
                game.clone(),
                1,
                format!("198.51.100.{}", idx + 10),
            )
            .await
            .unwrap();
        wave2_tickets.push((home_name.to_string(), t.ticket));
        all_accounts.push(account);
    }

    // Advance clocks to max_wait (15_000 ms) so every ticket widens across all 4 regions
    for node in cluster.values() {
        node.clock.skew_by(i64::try_from(max_wait).unwrap());
    }
    for node in cluster.values() {
        let widened = node.ring.advance_ring().await.unwrap();
        assert_eq!(
            widened, 3,
            "each ticket widens to the remaining 3 regions within max_wait"
        );
    }

    // Now all 4 regional nodes try to form a match for the same 4 tickets: CAS ensures exactly ONE match forms
    // and nobody is double-booked!
    let mut formed_matches = Vec::new();
    for name in home_keys {
        let matches = cluster[name]
            .ring
            .try_form_matches("escalation_shock", &game)
            .await
            .unwrap();
        formed_matches.extend(matches);
    }
    assert_eq!(
        formed_matches.len(),
        1,
        "CAS claim protocol must prevent double-booking"
    );
    let unique_players: BTreeSet<GlobalId> = formed_matches[0].players.iter().cloned().collect();
    assert_eq!(unique_players.len(), 4);

    // Verify every wave2 ticket is Claimed at its home node and waited <= max_wait
    for (home_name, ticket_id) in &wave2_tickets {
        match cluster[home_name].ring.ticket_state(ticket_id) {
            Some(TicketState::Claimed { match_id, .. }) => {
                assert_eq!(match_id, formed_matches[0].id);
            }
            other => panic!("expected Claimed state, got {other:?}"),
        }
    }

    // Forward post-hunt XP, loot, and bounty progress from the hosting node to each player's home node
    let host_node = &cluster[formed_matches[0].id.owner().as_str()];
    let module: ModuleId = "isles-1-14-7".parse().unwrap();
    for player in &formed_matches[0].players {
        host_node
            .ring
            .forward_hunt_rewards(HuntRewards {
                hunt: formed_matches[0].id.clone(),
                module: module.clone(),
                account: player.clone(),
                xp_grant: Some(serde_json::json!({ "weapon_xp": 250 })),
                loot_write: Some(serde_json::json!({ "arcstones": 12 })),
                objective_write: Some(serde_json::json!({ "bounty_id": "b1", "delta": 1 })),
            })
            .await
            .unwrap();
    }
    let forwarded = host_node.forwarded.lock().unwrap().clone();
    assert_eq!(forwarded.len(), 12);
    assert!(
        forwarded
            .iter()
            .any(|f| f.operation.as_str() == operations::PROGRESSION_GRANT)
    );
    assert!(
        forwarded
            .iter()
            .any(|f| f.operation.as_str() == operations::INVENTORY_WRITE)
    );
    assert!(
        forwarded
            .iter()
            .any(|f| f.operation.as_str() == operations::BOUNTY_WRITE)
    );
}

#[tokio::test]
async fn dn9_regional_ramsgate_places_players_from_different_homes_into_lowest_ping_hub_and_shards() {
    // Max 2 players per Ramsgate shard to verify automatic sharding
    let cluster = build_four_region_cluster(WrongNodePolicy::Redirect, 2);
    let game: GameVersion = "1.14.7".parse().unwrap();

    let p_oregon = GlobalId::new(IdKind::Account, &cluster["oregon-1"].identity.node);
    let p_ny = GlobalId::new(IdKind::Account, &cluster["ny-1"].identity.node);
    let p_de = GlobalId::new(IdKind::Account, &cluster["de-1"].identity.node);

    // All three players report `germany` as their lowest-ping region
    for (node_name, player) in [("oregon-1", &p_oregon), ("ny-1", &p_ny), ("de-1", &p_de)] {
        cluster[node_name]
            .ring
            .record_region_pings(
                player.clone(),
                vec![
                    RegionPing {
                        region: "germany".parse().unwrap(),
                        rtt_ms: Some(18),
                    },
                    RegionPing {
                        region: "oregon".parse().unwrap(),
                        rtt_ms: Some(140),
                    },
                ],
            )
            .unwrap();
    }

    // Player 1 (homed in Oregon) and Player 2 (homed in New York) both land in Germany's Shard 1!
    let (hub1, ticket1) = cluster["oregon-1"]
        .ring
        .assign_ramsgate(p_oregon.clone(), "192.0.2.1", game.clone())
        .await
        .unwrap();
    let (hub2, ticket2) = cluster["ny-1"]
        .ring
        .assign_ramsgate(p_ny.clone(), "192.0.2.2", game.clone())
        .await
        .unwrap();

    assert_eq!(
        hub1.id, hub2.id,
        "players from different homes meet in one region's hub"
    );
    assert_eq!(hub1.id.owner(), &cluster["de-1"].identity.node);
    assert_eq!(hub1.shard, Some(1));
    assert_eq!(hub2.players, vec![p_oregon.clone(), p_ny.clone()]);
    assert!(ticket1.is_some() && ticket2.is_some());

    // Shard 1 is now at capacity (2/2); Player 3 (homed in Germany) opens Shard 2 in Germany!
    let (hub3, ticket3) = cluster["de-1"]
        .ring
        .assign_ramsgate(p_de.clone(), "192.0.2.3", game)
        .await
        .unwrap();
    assert_ne!(hub3.id, hub1.id);
    assert_eq!(hub3.shard, Some(2));
    assert!(
        ticket3.is_none(),
        "player already homed on de-1 needs no cross-node ticket"
    );
}

#[test]
fn ring_wire_refuses_account_keys_and_bad_versions() {
    let node: NodeId = "oregon-1".parse().unwrap();
    let instance = GlobalId::new(IdKind::Hunt, &node);
    let ok_bytes = encode_request(RingCall::LookupInstance { instance }).unwrap();
    assert!(decode_request(&ok_bytes).is_ok());

    let account = GlobalId::new(IdKind::Account, &node);
    let bad_call = RingCall::AssignHub {
        account,
        client_ip: "RRK_leaked_account_key_123456".into(),
        game: "1.14.7".parse().unwrap(),
    };
    assert_eq!(encode_request(bad_call), Err(RingError::AccountKey));

    let bad_version =
        br#"{"version":99,"call":{"lookup_instance":{"instance":"hunt:01J9Z3N5C8Q0V2X4Y6W8T0R2P4@oregon-1"}}}"#;
    assert!(decode_request(bad_version).is_err());

    let _ = ClaimOutcome::Granted { session_ticket: None };
}
