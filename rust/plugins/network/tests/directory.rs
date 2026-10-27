//! The directory across nodes in one process (DN4): four regions on an in-process network, each with its
//! own database. SQLite always; Postgres too when `REFORGED_TEST_POSTGRES_URL` names a server the tests
//! may create databases on, as `store`'s tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

mod common;

use common::{Engine, Target, engines};

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use module_api::BoxFuture;
use module_api::network::{GlobalId, IdKind, NodeId, NodeIdentity};
use network::inprocess::InProcessNetwork;
use network::messages::{DIRECTORY_VERSION, DirectoryCall, DirectoryReply, DirectoryRequest, decode_response, encode};
use network::{Directory, DirectoryConfig, DirectoryError, LinkError, NodeService, PeerLink, Refusal, Role, Service};
use protocol::account::UsernameCheck;
use store::{Actor, Database};

// ------------------------------------------------------------------ the nodes

const LEADER: &str = "oregon-1";
const REGIONS: [&str; 4] = ["oregon-1", "new-york-1", "uk-1", "germany-1"];

fn id(name: &str) -> NodeId {
    NodeId::new(name).unwrap()
}

fn account(home: &str) -> GlobalId {
    GlobalId::new(IdKind::Account, &id(home))
}

fn player(account: &GlobalId) -> Actor {
    Actor::Player(account.to_string())
}

struct Node {
    name: &'static str,
    dir: Arc<Directory>,
    target: Target,
}

struct Cluster {
    net: InProcessNetwork,
    nodes: Vec<Node>,
    page: u32,
}

async fn start(net: &InProcessNetwork, name: &str, db: Database, page: u32) -> Arc<Directory> {
    let config = DirectoryConfig::in_network(id(name), id(LEADER)).with_page(page);
    let dir = Arc::new(Directory::new(db, config, Arc::new(net.link(&id(name)))).await.unwrap());
    let service: Arc<dyn NodeService> = dir.clone();
    net.attach(&id(name), &service);
    dir
}

impl Cluster {
    async fn new(engine: Engine, page: u32) -> Self {
        let net = InProcessNetwork::new();
        let mut nodes = Vec::new();
        for name in REGIONS {
            let target = Target::new(engine);
            let dir = start(&net, name, target.open(), page).await;
            nodes.push(Node { name, dir, target });
        }
        Self { net, nodes, page }
    }

    fn node(&self, name: &str) -> &Directory {
        &self.nodes.iter().find(|n| n.name == name).unwrap().dir
    }

    /// Stops a node and starts it again on the same database.
    async fn restart(&mut self, name: &str) {
        let node = self.nodes.iter_mut().find(|n| n.name == name).unwrap();
        let db = node.target.open();
        node.dir = start(&self.net, name, db, self.page).await;
    }

    async fn sync_all(&self) {
        for node in &self.nodes {
            node.dir.sync().await.unwrap();
        }
    }
}

fn refused(result: Result<impl std::fmt::Debug, DirectoryError>) -> Refusal {
    match result {
        Err(DirectoryError::Refused(refusal)) => refusal,
        other => panic!("not a refusal: {other:?}"),
    }
}

// ------------------------------------------------------------------ the row's Verify

#[tokio::test(flavor = "multi_thread")]
async fn a_name_taken_in_one_region_is_refused_in_every_other() {
    for engine in engines() {
        let cluster = Cluster::new(engine, 3).await;
        assert_eq!(cluster.node(LEADER).role(), &Role::Leader);
        assert_eq!(cluster.node("uk-1").role(), &Role::Replica { leader: id(LEADER) });

        let fox = account("germany-1");
        let entry = cluster
            .node("germany-1")
            .register(player(&fox), &fox, "Fox")
            .await
            .unwrap();
        assert_eq!(entry.seq, 1);

        // Before anyone else pulled: the leader decides, not the replica's copy
        let early = account("uk-1");
        assert_eq!(
            refused(cluster.node("uk-1").register(player(&early), &early, "fOX").await),
            Refusal::UsernameTaken,
            "{engine:?}"
        );

        cluster.sync_all().await;
        for region in REGIONS {
            let node = cluster.node(region);
            for name in ["Fox", "fox", "FOX", "fOx"] {
                let other = account(region);
                assert_eq!(
                    refused(node.register(player(&other), &other, name).await),
                    Refusal::UsernameTaken,
                    "{engine:?} {region} {name}"
                );
            }
            let found = node.find("FOX").await.unwrap().expect("every region resolves the name");
            assert_eq!(found.account, fox);
            assert_eq!(found.username.as_str(), "Fox");
            assert_eq!(found.home, id("germany-1"));
            assert_eq!(node.home_of(&fox).await.unwrap(), Some(id("germany-1")));
            assert_eq!(
                node.head().await.unwrap(),
                1,
                "{engine:?} {region}: refusals write nothing"
            );
        }

        // Two regions asking at once: one gets the name, the other is refused
        let (a, b) = (account("uk-1"), account("new-york-1"));
        let (first, second) = tokio::join!(
            cluster.node("uk-1").register(player(&a), &a, "Wolf"),
            cluster.node("new-york-1").register(player(&b), &b, "WOLF"),
        );
        let wins = [first.is_ok(), second.is_ok()];
        assert_eq!(
            wins.iter().filter(|w| **w).count(),
            1,
            "{engine:?}: {first:?} {second:?}"
        );
        let loser = if wins[0] { second } else { first };
        assert_eq!(refused(loser), Refusal::UsernameTaken);
        cluster.sync_all().await;
        let winner = if wins[0] { &a } else { &b };
        for region in REGIONS {
            assert_eq!(
                &cluster.node(region).find("wolf").await.unwrap().unwrap().account,
                winner
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_replica_catches_up_after_a_partition() {
    for engine in engines() {
        let mut cluster = Cluster::new(engine, 2).await;
        let early = account("uk-1");
        cluster
            .node("uk-1")
            .register(player(&early), &early, "Early")
            .await
            .unwrap();
        assert_eq!(
            cluster.node("uk-1").head().await.unwrap(),
            1,
            "a home pulls after its own change"
        );

        cluster.net.isolate(&id("uk-1"));
        let mut names = Vec::new();
        for (i, region) in ["oregon-1", "germany-1", "new-york-1"]
            .into_iter()
            .cycle()
            .take(7)
            .enumerate()
        {
            let who = account(region);
            let name = format!("Hunter_{i}");
            cluster.node(region).register(player(&who), &who, &name).await.unwrap();
            names.push((who, name));
        }
        let (renamed, _) = names[1].clone();
        cluster
            .node("germany-1")
            .rename(player(&renamed), &renamed, "Slayer")
            .await
            .unwrap();

        // Cut off: reads still answer from the copy it has, writes wait for the leader
        let uk = cluster.node("uk-1");
        assert!(matches!(uk.sync().await, Err(DirectoryError::LeaderUnreachable(_))));
        let late = account("uk-1");
        assert!(matches!(
            uk.register(player(&late), &late, "Late").await,
            Err(DirectoryError::LeaderUnreachable(_))
        ));
        assert_eq!(uk.find("early").await.unwrap().unwrap().account, early);
        assert_eq!(uk.find("Hunter_0").await.unwrap(), None);

        // A restart during the partition loses nothing
        cluster.restart("uk-1").await;
        assert_eq!(cluster.node("uk-1").head().await.unwrap(), 1);

        cluster.net.heal();
        let leader_head = cluster.node(LEADER).head().await.unwrap();
        assert_eq!(leader_head, 9);
        let uk = cluster.node("uk-1");
        assert_eq!(
            uk.sync().await.unwrap(),
            leader_head - 1,
            "{engine:?}: pulled in pages of 2"
        );
        assert_eq!(uk.head().await.unwrap(), leader_head);
        assert_eq!(
            uk.entries(0, 100).await.unwrap(),
            cluster.node(LEADER).entries(0, 100).await.unwrap(),
            "{engine:?}: the same log"
        );
        for (who, name) in &names[..] {
            let found = uk.find(name).await.unwrap();
            if *who == renamed {
                assert_eq!(found, None, "the old name is free");
            } else {
                assert_eq!(&found.unwrap().account, who, "{name}");
            }
        }
        assert_eq!(uk.find("slayer").await.unwrap().unwrap().account, renamed);
        assert_eq!(uk.sync().await.unwrap(), 0, "nothing left");

        // The freed name can be claimed again, and the late registration goes through now
        uk.register(player(&late), &late, "Hunter_1").await.unwrap();
        uk.register(player(&early), &early, "Early").await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn replicas_pull_in_the_background() {
    let cluster = Cluster::new(Engine::Sqlite, 3).await;
    let task = cluster.nodes[2]
        .dir
        .spawn_replication(Duration::from_millis(20))
        .unwrap();
    assert!(
        cluster.nodes[0]
            .dir
            .spawn_replication(Duration::from_millis(20))
            .is_none()
    );

    cluster.net.isolate(&id("uk-1"));
    let who = account(LEADER);
    cluster
        .node(LEADER)
        .register(player(&who), &who, "Quiet")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(cluster.node("uk-1").find("quiet").await.unwrap(), None);

    cluster.net.heal();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while cluster.node("uk-1").find("quiet").await.unwrap().is_none() {
        assert!(tokio::time::Instant::now() < deadline, "the replica never caught up");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    task.abort();
}

// ------------------------------------------------------------------ outside a network

/// A link that counts the calls made through it, and has no one to call.
#[derive(Default)]
struct Counting(AtomicUsize);

impl PeerLink for Counting {
    fn call<'a>(
        &'a self,
        _to: &'a NodeId,
        _service: Service,
        _body: Vec<u8>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LinkError>> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Box::pin(async { Err(LinkError::NoPeers) })
    }
}

async fn standalone(engine: Engine) -> (Arc<Directory>, Arc<Counting>, Target) {
    let target = Target::new(engine);
    let link = Arc::new(Counting::default());
    let node = NodeIdentity::standalone().node;
    let dir = Directory::new(target.open(), DirectoryConfig::standalone(node), link.clone())
        .await
        .unwrap();
    (Arc::new(dir), link, target)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_outside_a_network_keeps_its_directory_to_itself() {
    for engine in engines() {
        let (dir, link, _target) = standalone(engine).await;
        assert_eq!(dir.role(), &Role::Standalone);
        let local = NodeIdentity::standalone().node;
        let fox = GlobalId::new(IdKind::Account, &local);
        dir.register(player(&fox), &fox, "Fox").await.unwrap();
        let other = GlobalId::new(IdKind::Account, &local);
        assert_eq!(
            refused(dir.register(player(&other), &other, "FOX").await),
            Refusal::UsernameTaken
        );
        dir.rename(player(&fox), &fox, "Vixen").await.unwrap();
        dir.register(player(&other), &other, "fox").await.unwrap();
        assert_eq!(dir.home_of(&fox).await.unwrap(), Some(local.clone()));
        assert_eq!(dir.sync().await.unwrap(), 0);
        assert!(dir.spawn_replication(Duration::from_millis(10)).is_none());
        let foreign = account("germany-1");
        assert_eq!(
            refused(dir.register(player(&foreign), &foreign, "Stranger").await),
            Refusal::NotHome
        );
        assert_eq!(
            link.0.load(Ordering::Relaxed),
            0,
            "{engine:?}: nothing leaves the server"
        );
    }
}

/// The protocol's shared username table (`crates/protocol/tests/cases/username.json`): what registration
/// accepts there, the directory accepts, and it refuses the rest for the same reason.
#[tokio::test(flavor = "multi_thread")]
async fn usernames_follow_the_protocol_table() {
    #[derive(serde::Deserialize)]
    struct Table {
        check: Vec<Case>,
    }
    #[derive(serde::Deserialize)]
    struct Case {
        input: serde_json::Value,
        result: String,
    }
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/protocol/tests/cases/username.json"
    );
    let table: Table = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let (dir, _, _target) = standalone(Engine::Sqlite).await;
    let local = NodeIdentity::standalone().node;
    let mut checked = 0;
    for case in table.check {
        let Some(input) = case.input.as_str() else { continue };
        let who = GlobalId::new(IdKind::Account, &local);
        let result = dir.register(player(&who), &who, input).await;
        if case.result == "ok" {
            result.unwrap();
            assert_eq!(dir.find(input).await.unwrap().unwrap().account, who);
        } else {
            let Refusal::InvalidUsername { check } = refused(result) else {
                panic!("{input:?}: not refused as a username");
            };
            assert_eq!(check.as_str(), case.result, "{input:?}");
            assert_eq!(dir.find(input).await.unwrap(), None);
        }
        checked += 1;
    }
    assert!(checked >= 10, "the table has its cases");
}

// ------------------------------------------------------------------ what peers may ask

async fn ask(service: &dyn NodeService, from: &str, call: DirectoryCall) -> DirectoryReply {
    let body = encode(&DirectoryRequest {
        version: DIRECTORY_VERSION,
        call,
    })
    .unwrap();
    let answer = service.serve(&id(from), Service::Directory, body).await.unwrap();
    decode_response(&answer).unwrap().reply
}

fn refusal(reply: DirectoryReply) -> Refusal {
    match reply {
        DirectoryReply::Refused(refusal) => refusal,
        other => panic!("not a refusal: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn peers_change_only_their_own_accounts_through_the_leader() {
    let cluster = Cluster::new(Engine::Sqlite, 10).await;
    let leader = cluster.nodes[0].dir.clone();
    let register = |account: &GlobalId, username: &str| DirectoryCall::Register {
        account: account.clone(),
        username: username.into(),
        actor: player(account).as_text(),
    };
    let fox = account("germany-1");

    // Only the account's home, by the name its link authenticated
    assert_eq!(
        refusal(ask(&*leader, "uk-1", register(&fox, "Fox")).await),
        Refusal::NotHome
    );
    let hunt = GlobalId::new(IdKind::Hunt, &id("germany-1"));
    assert_eq!(
        refusal(ask(&*leader, "germany-1", register(&hunt, "Fox")).await),
        Refusal::NotAnAccount
    );
    let bad_actor = DirectoryCall::Register {
        account: fox.clone(),
        username: "Fox".into(),
        actor: "nobody".into(),
    };
    assert_eq!(refusal(ask(&*leader, "germany-1", bad_actor).await), Refusal::BadActor);
    assert_eq!(
        refusal(ask(&*leader, "germany-1", register(&fox, "F")).await),
        Refusal::InvalidUsername {
            check: UsernameCheck::TooShort
        }
    );

    // A repeat after a lost answer gets the same entry; another name for the account doesn't
    let DirectoryReply::Applied(first) = ask(&*leader, "germany-1", register(&fox, "Fox")).await else {
        panic!("registered");
    };
    let DirectoryReply::Applied(again) = ask(&*leader, "germany-1", register(&fox, "Fox")).await else {
        panic!("repeated");
    };
    assert_eq!(first, again);
    assert_eq!(
        refusal(ask(&*leader, "germany-1", register(&fox, "Fox_2")).await),
        Refusal::AccountExists
    );

    // Renames: the home only, never onto another's name, a change of case is fine
    let wolf = account("uk-1");
    cluster
        .node("uk-1")
        .register(player(&wolf), &wolf, "Wolf")
        .await
        .unwrap();
    let rename = |account: &GlobalId, username: &str| DirectoryCall::Rename {
        account: account.clone(),
        username: username.into(),
        actor: player(account).as_text(),
    };
    assert_eq!(
        refusal(ask(&*leader, "uk-1", rename(&fox, "Stolen")).await),
        Refusal::NotHome
    );
    assert_eq!(
        refusal(ask(&*leader, "uk-1", rename(&wolf, "fox")).await),
        Refusal::UsernameTaken
    );
    assert!(matches!(
        ask(&*leader, "uk-1", rename(&wolf, "WOLF")).await,
        DirectoryReply::Applied(_)
    ));
    assert_eq!(
        refusal(ask(&*leader, "uk-1", rename(&account("uk-1"), "Ghost")).await),
        Refusal::UnknownAccount
    );

    // Replicas send writers to the leader
    let replica = cluster.nodes[2].dir.clone();
    assert_eq!(
        refusal(ask(&*replica, "germany-1", register(&account("germany-1"), "Lynx")).await),
        Refusal::NotLeader {
            leader: Some(id(LEADER))
        }
    );

    // Pulls must say where they stand; the bytes must be a request of this version
    let pull = DirectoryCall::Pull {
        after: 0,
        last: Some(first.clone()),
        limit: 5,
    };
    assert_eq!(refusal(ask(&*leader, "uk-1", pull).await), Refusal::Malformed);
    let DirectoryReply::Changes { entries, head } = ask(
        &*leader,
        "uk-1",
        DirectoryCall::Pull {
            after: 0,
            last: None,
            limit: 0,
        },
    )
    .await
    else {
        panic!("a page");
    };
    assert_eq!((entries.len(), head), (1, 3), "a limit of 0 is read as 1");
    let cases: [(&[u8], Refusal); 3] = [
        (b"not json", Refusal::Malformed),
        (br#"{"version":1,"call":{"pull":{"after":0}}}"#, Refusal::Malformed),
        (
            br#"{"version":9,"call":{"pull":{"after":0,"limit":1}}}"#,
            Refusal::UnsupportedVersion {
                supported: DIRECTORY_VERSION,
            },
        ),
    ];
    for (body, expected) in cases {
        let answer = leader
            .serve(&id("uk-1"), Service::Directory, body.to_vec())
            .await
            .unwrap();
        assert_eq!(refusal(decode_response(&answer).unwrap().reply), expected);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_replica_ahead_of_its_leader_stops_replicating() {
    for engine in engines() {
        let cluster = Cluster::new(engine, 10).await;
        for name in ["One", "Two", "Three"] {
            let who = account(LEADER);
            cluster.node(LEADER).register(player(&who), &who, name).await.unwrap();
        }
        cluster.sync_all().await;
        assert_eq!(cluster.node("uk-1").head().await.unwrap(), 3);

        // The leader comes back from an older backup: empty, then with one different entry
        let restored = Target::new(engine);
        let leader = start(&cluster.net, LEADER, restored.open(), 10).await;
        assert!(matches!(
            cluster.node("uk-1").sync().await,
            Err(DirectoryError::Diverged(_))
        ));
        let who = account(LEADER);
        leader.register(player(&who), &who, "Other").await.unwrap();
        assert!(matches!(
            cluster.node("uk-1").sync().await,
            Err(DirectoryError::Diverged(_))
        ));
        assert_eq!(
            cluster.node("uk-1").head().await.unwrap(),
            3,
            "{engine:?}: nothing applied"
        );
        assert_eq!(cluster.node("uk-1").find("other").await.unwrap(), None);
    }
}
