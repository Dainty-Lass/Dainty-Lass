//! The directory under failover (`docs/distributed-mode.md`): three nodes elect their leader, a write is
//! acknowledged only once a majority holds it, and a leader that goes down loses nothing it acknowledged.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use common::{Engine, Target, engines};

use std::sync::Arc;
use std::time::Duration;

use module_api::network::{GlobalId, IdKind, NodeId};
use network::election::{Election, ElectionConfig, MemoryTermStore};
use network::inprocess::InProcessNetwork;
use network::{
    Directory, DirectoryChange, DirectoryConfig, DirectoryError, NodeService, Refusal, Role, Service, ServiceMux,
};
use store::Actor;

const NODES: [&str; 3] = ["oregon-1", "uk-1", "germany-1"];

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
    election: Arc<Election>,
    _mux: Arc<dyn NodeService>,
    _target: Target,
    _tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for Node {
    fn drop(&mut self) {
        for task in &self._tasks {
            task.abort();
        }
    }
}

struct Cluster {
    net: InProcessNetwork,
    nodes: Vec<Node>,
}

async fn start(net: &InProcessNetwork, engine: Engine, name: &'static str) -> Node {
    let peers: Vec<NodeId> = NODES.iter().filter(|n| **n != name).map(|n| id(n)).collect();
    let target = Target::new(engine);
    let mut config = DirectoryConfig::failover(id(name), peers.clone()).with_page(4);
    if let Some(failover) = config.failover.as_mut() {
        failover.commit_timeout = Duration::from_millis(400);
    }
    let link = Arc::new(net.link(&id(name)));
    let dir = Arc::new(Directory::new(target.open(), config, link.clone()).await.unwrap());
    let election = Election::new(
        ElectionConfig {
            node: id(name),
            peers,
            election_timeout: (Duration::from_millis(200), Duration::from_millis(400)),
            heartbeat: Duration::from_millis(40),
            call_timeout: Duration::from_millis(150),
        },
        link,
        dir.clone(),
        Arc::new(MemoryTermStore::default()),
    )
    .unwrap();
    let mux: Arc<dyn NodeService> = Arc::new(
        ServiceMux::new(id(name))
            .with(Service::Directory, dir.clone())
            .with(Service::Election, election.clone()),
    );
    net.attach(&id(name), &mux);
    let mut tasks = vec![dir.follow(election.subscribe()), election.spawn()];
    tasks.extend(dir.spawn_replication(Duration::from_millis(50)));
    Node {
        name,
        dir,
        election,
        _mux: mux,
        _target: target,
        _tasks: tasks,
    }
}

impl Cluster {
    async fn new(engine: Engine) -> Self {
        let net = InProcessNetwork::new();
        let mut nodes = Vec::new();
        for name in NODES {
            nodes.push(start(&net, engine, name).await);
        }
        Self { net, nodes }
    }

    fn node(&self, name: &str) -> &Node {
        self.nodes.iter().find(|n| n.name == name).unwrap()
    }

    /// Waits until every node in `among` agrees on one leader outside `except`, and its directory leads.
    async fn leader(&self, among: &[&str]) -> &'static str {
        for _ in 0..300 {
            let views: Vec<_> = among.iter().map(|n| self.node(n).election.leadership()).collect();
            if let Some(Some(leader)) = views.first().map(|v| v.leader.clone())
                && views.iter().all(|v| v.leader.as_ref() == Some(&leader))
                && among.contains(&leader.as_str())
                && self.node(leader.as_str()).dir.role() == Role::Leader
            {
                return self.node(leader.as_str()).name;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("no leader among {among:?}");
    }
}

fn others<'a>(all: &'a [&'a str], but: &str) -> Vec<&'a str> {
    all.iter().copied().filter(|n| *n != but).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_acknowledged_name_outlives_its_leader() {
    for engine in engines() {
        let cluster = Cluster::new(engine).await;
        let leader = cluster.leader(&NODES).await;
        let home = others(&NODES, leader)[0];

        let fox = account(home);
        let entry = cluster
            .node(home)
            .dir
            .register(player(&fox), &fox, "Fox")
            .await
            .unwrap();
        assert!(entry.term > 0, "{engine:?}: an entry carries its term");

        cluster.net.isolate(&id(leader));
        let rest = others(&NODES, leader);
        let next = cluster.leader(&rest).await;
        assert_ne!(next, leader);
        let new_dir = &cluster.node(next).dir;
        assert!(new_dir.term() > entry.term, "{engine:?}");
        let found = new_dir
            .find("fox")
            .await
            .unwrap()
            .expect("the new leader holds the acknowledged name");
        assert_eq!(found.account, fox, "{engine:?}");

        // Writes go on without the old leader, and the name stays taken
        let other = account(rest[1]);
        let taken = cluster.node(rest[1]).dir.register(player(&other), &other, "FOX").await;
        assert!(
            matches!(taken, Err(DirectoryError::Refused(Refusal::UsernameTaken))),
            "{engine:?}: {taken:?}"
        );
        let wolf = account(rest[1]);
        cluster
            .node(rest[1])
            .dir
            .register(player(&wolf), &wolf, "Wolf")
            .await
            .unwrap();

        // The new leader's log opens its term with a lead entry
        let entries = new_dir.entries(0, 256).await.unwrap();
        assert!(
            entries
                .iter()
                .any(|e| e.change == DirectoryChange::Lead { node: id(next) } && e.term == new_dir.term()),
            "{engine:?}: {entries:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cut_off_leader_acknowledges_nothing_and_drops_it_when_back() {
    for engine in engines() {
        let cluster = Cluster::new(engine).await;
        let leader = cluster.leader(&NODES).await;

        // Before it notices the partition, the old leader writes locally but can't reach a majority
        cluster.net.isolate(&id(leader));
        let lost = account(leader);
        let attempt = cluster.node(leader).dir.register(player(&lost), &lost, "Raven").await;
        assert!(
            matches!(
                attempt,
                Err(DirectoryError::Refused(Refusal::NoQuorum) | DirectoryError::NoLeader)
            ),
            "{engine:?}: {attempt:?}"
        );

        // The majority elects again and gives the name to someone else
        let rest = others(&NODES, leader);
        let next = cluster.leader(&rest).await;
        let winner = account(next);
        cluster
            .node(next)
            .dir
            .register(player(&winner), &winner, "raven")
            .await
            .unwrap();

        // Back on the network, the old leader follows and drops what it never got acknowledged
        cluster.net.heal();
        let old = &cluster.node(leader).dir;
        let mut held = None;
        for _ in 0..200 {
            let _ = old.sync().await;
            held = old.find("Raven").await.unwrap();
            if held.as_ref().is_some_and(|r| r.account == winner) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(held.map(|r| r.account), Some(winner), "{engine:?}");
        assert_eq!(old.account(&lost).await.unwrap(), None, "{engine:?}");
        assert_eq!(
            old.head().await.unwrap(),
            cluster.node(next).dir.head().await.unwrap(),
            "{engine:?}"
        );
        assert!(matches!(old.role(), Role::Replica { .. }), "{engine:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_majority_nothing_is_written_anywhere() {
    for engine in engines() {
        let cluster = Cluster::new(engine).await;
        cluster.leader(&NODES).await;
        for name in NODES {
            cluster.net.isolate(&id(name));
        }
        // Check-quorum and the election timeout both pass: nobody leads
        tokio::time::sleep(Duration::from_millis(1200)).await;
        for name in NODES {
            let node = cluster.node(name);
            assert_eq!(node.dir.role(), Role::Electing, "{engine:?} {name}");
            let fox = account(name);
            let refused = node.dir.register(player(&fox), &fox, "Fox").await;
            assert!(
                matches!(refused, Err(DirectoryError::NoLeader)),
                "{engine:?} {name}: {refused:?}"
            );
        }
        cluster.net.heal();
        let leader = cluster.leader(&NODES).await;
        let fox = account(leader);
        cluster
            .node(leader)
            .dir
            .register(player(&fox), &fox, "Fox")
            .await
            .unwrap();
    }
}
