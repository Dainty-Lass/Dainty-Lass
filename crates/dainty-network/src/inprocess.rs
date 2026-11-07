//! Node links inside one process, for tests and DN12's harness before DN3: every node is a
//! [`NodeService`] on one [`InProcessNetwork`], and links between two of them can be cut and healed to
//! make partitions. Bodies cross as bytes with [`MAX_MESSAGE`] enforced, as they would on a real link.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use dainty_module_api::BoxFuture;
use dainty_module_api::network::NodeId;

use crate::link::{LinkError, MAX_MESSAGE, NodeService, PeerLink, PeerRoster, Service};

#[derive(Default)]
struct State {
    // Weak: a node's service usually holds its own link, which holds this network
    nodes: HashMap<NodeId, Weak<dyn NodeService>>,
    cut: BTreeSet<(NodeId, NodeId)>,
}

/// The nodes of one test network and the links cut between them.
#[derive(Clone, Default)]
pub struct InProcessNetwork {
    state: Arc<Mutex<State>>,
}

fn pair(a: &NodeId, b: &NodeId) -> (NodeId, NodeId) {
    if a <= b {
        (a.clone(), b.clone())
    } else {
        (b.clone(), a.clone())
    }
}

impl InProcessNetwork {
    /// A network with no nodes yet.
    pub fn new() -> Self {
        Self::default()
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The link `node` calls its peers through.
    pub fn link(&self, node: &NodeId) -> InProcessLink {
        InProcessLink {
            network: self.clone(),
            local: node.clone(),
        }
    }

    /// Puts `service` on the network as `node`, answering the calls made to it.
    pub fn attach(&self, node: &NodeId, service: &Arc<dyn NodeService>) {
        self.state().nodes.insert(node.clone(), Arc::downgrade(service));
    }

    /// Cuts the link between `a` and `b`, both ways.
    pub fn cut(&self, a: &NodeId, b: &NodeId) {
        self.state().cut.insert(pair(a, b));
    }

    /// Cuts `node` off from every other node.
    pub fn isolate(&self, node: &NodeId) {
        let mut state = self.state();
        let others: Vec<NodeId> = state.nodes.keys().filter(|n| *n != node).cloned().collect();
        for other in others {
            state.cut.insert(pair(node, &other));
        }
    }

    /// Restores every link.
    pub fn heal(&self) {
        self.state().cut.clear();
    }

    fn route(&self, from: &NodeId, to: &NodeId) -> Result<Arc<dyn NodeService>, LinkError> {
        let state = self.state();
        let unreachable = |reason: &str| LinkError::Unreachable {
            node: to.clone(),
            reason: reason.to_owned(),
        };
        if state.cut.contains(&pair(from, to)) {
            return Err(unreachable("the link is cut"));
        }
        state
            .nodes
            .get(to)
            .ok_or_else(|| LinkError::UnknownPeer(to.clone()))?
            .upgrade()
            .ok_or_else(|| unreachable("the node is down"))
    }
}

/// One node's side of an [`InProcessNetwork`].
#[derive(Clone)]
pub struct InProcessLink {
    network: InProcessNetwork,
    local: NodeId,
}

impl PeerLink for InProcessLink {
    fn call<'a>(
        &'a self,
        to: &'a NodeId,
        service: Service,
        body: Vec<u8>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LinkError>> {
        Box::pin(async move {
            if body.len() > MAX_MESSAGE {
                return Err(LinkError::TooLarge);
            }
            let target = self.network.route(&self.local, to)?;
            let answer = target.serve(&self.local, service, body).await?;
            if answer.len() > MAX_MESSAGE {
                return Err(LinkError::TooLarge);
            }
            // A partition that starts while the call is in flight loses the answer
            self.network.route(&self.local, to)?;
            Ok(answer)
        })
    }
}

/// Every attached node but this one is a peer; one the network has cut off, or whose service is gone, is down.
impl PeerRoster for InProcessLink {
    fn peers(&self) -> Vec<NodeId> {
        let state = self.network.state();
        let mut peers: Vec<NodeId> = state.nodes.keys().filter(|n| **n != self.local).cloned().collect();
        peers.sort();
        peers
    }

    fn is_up(&self, node: &NodeId) -> bool {
        self.network.route(&self.local, node).is_ok()
    }
}
