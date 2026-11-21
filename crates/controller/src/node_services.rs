// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! [`NodeServices`]: what this node answers over its node links, by [`Service`] (DD16). The gateway's endpoint
//! holds one of these for good; a core plugin plugs its service in with one line, wherever it is built:
//! `registry.node_services().attach(Service::Admin, service)`. The network module attaches the directory's and
//! the home API's when it starts and takes them back when it stops, so a restarted module is picked up by the
//! endpoint that never changed.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};

use module_api::BoxFuture;
use module_api::network::NodeId;
use network_plugin::{LinkError, NodeService, Service, ServiceMux};

pub type RemoteEnrollCallback = Arc<dyn Fn(Vec<u8>) -> BoxFuture<'static, (u16, Vec<u8>)> + Send + Sync>;

/// The gateway's `/_rr/node/*` side, as the host composing the gateway hands it over: the MIT controller never
/// names the AGPL gateway's types (RD27).
pub trait GatewayLinks: Send + Sync {
    fn endpoint(&self) -> &network_plugin::NodeEndpoint;
    fn set_enroll_handler(&self, handler: Option<RemoteEnrollCallback>);

    /// The endpoint's, unless the endpoint is across a library's protocol (`service_library::gateway`).
    fn attach(&self, services: Arc<dyn NodeService>) {
        self.endpoint().attach(services);
    }

    fn add_peer(&self, node: NodeId) {
        self.endpoint().add_peer(node);
    }

    fn remove_peer(&self, node: &NodeId) {
        self.endpoint().remove_peer(node);
    }
}

struct Inner {
    node: NodeId,
    attached: RwLock<HashMap<Service, Arc<dyn NodeService>>>,
    /// Rebuilt when a service comes or goes, so a call takes a read lock and no more
    mux: RwLock<Arc<ServiceMux>>,
    peer_link: RwLock<Option<Arc<dyn network_plugin::PeerLink>>>,
    peer_roster: RwLock<Option<Arc<dyn network_plugin::PeerRoster>>>,
    enroll_handler: RwLock<Option<RemoteEnrollCallback>>,
    dynamic_peers: RwLock<std::collections::BTreeSet<NodeId>>,
    #[cfg(feature = "network")]
    directory: RwLock<Option<Arc<network_plugin::directory::Directory>>>,
    #[cfg(feature = "network")]
    tickets: RwLock<Option<Arc<network_plugin::home::Tickets>>>,
    #[cfg(feature = "network")]
    ring: RwLock<Option<Arc<network_plugin::ring::RingNode>>>,
    gateway: RwLock<Option<Arc<dyn GatewayLinks>>>,
}

#[derive(Clone)]
pub struct NodeServices {
    inner: Arc<Inner>,
}

impl NodeServices {
    pub fn new(node: NodeId) -> Self {
        Self {
            inner: Arc::new(Inner {
                mux: RwLock::new(Arc::new(ServiceMux::new(node.clone()))),
                attached: RwLock::default(),
                node,
                peer_link: RwLock::new(None),
                peer_roster: RwLock::new(None),
                enroll_handler: RwLock::new(None),
                dynamic_peers: RwLock::default(),
                #[cfg(feature = "network")]
                directory: RwLock::new(None),
                #[cfg(feature = "network")]
                tickets: RwLock::new(None),
                #[cfg(feature = "network")]
                ring: RwLock::new(None),
                gateway: RwLock::new(None),
            }),
        }
    }

    pub fn node(&self) -> &NodeId {
        &self.inner.node
    }

    pub fn set_peer_link(&self, link: Option<Arc<dyn network_plugin::PeerLink>>) {
        *self.inner.peer_link.write().unwrap_or_else(PoisonError::into_inner) = link;
    }

    pub fn peer_link(&self) -> Option<Arc<dyn network_plugin::PeerLink>> {
        self.inner
            .peer_link
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn set_peer_roster(&self, roster: Option<Arc<dyn network_plugin::PeerRoster>>) {
        *self.inner.peer_roster.write().unwrap_or_else(PoisonError::into_inner) = roster;
    }

    pub fn peer_roster(&self) -> Option<Arc<dyn network_plugin::PeerRoster>> {
        self.inner
            .peer_roster
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    #[cfg(feature = "network")]
    pub fn set_directory(&self, directory: Option<Arc<network_plugin::directory::Directory>>) {
        *self.inner.directory.write().unwrap_or_else(PoisonError::into_inner) = directory;
    }

    #[cfg(feature = "network")]
    pub fn directory(&self) -> Option<Arc<network_plugin::directory::Directory>> {
        self.inner
            .directory
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    #[cfg(feature = "network")]
    pub fn set_tickets(&self, tickets: Option<Arc<network_plugin::home::Tickets>>) {
        *self.inner.tickets.write().unwrap_or_else(PoisonError::into_inner) = tickets;
    }

    #[cfg(feature = "network")]
    pub fn tickets(&self) -> Option<Arc<network_plugin::home::Tickets>> {
        self.inner
            .tickets
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    #[cfg(feature = "network")]
    pub fn set_ring(&self, ring: Option<Arc<network_plugin::ring::RingNode>>) {
        *self.inner.ring.write().unwrap_or_else(PoisonError::into_inner) = ring;
    }

    #[cfg(feature = "network")]
    pub fn ring(&self) -> Option<Arc<network_plugin::ring::RingNode>> {
        self.inner.ring.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub fn set_enroll_handler(&self, handler: Option<RemoteEnrollCallback>) {
        *self
            .inner
            .enroll_handler
            .write()
            .unwrap_or_else(PoisonError::into_inner) = handler;
        self.bind_gateway();
    }

    pub fn enroll_handler(&self) -> Option<RemoteEnrollCallback> {
        self.inner
            .enroll_handler
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn add_dynamic_peer(&self, node: NodeId) {
        self.inner
            .dynamic_peers
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(node.clone());
        if let Some(links) = self.gateway() {
            links.add_peer(node);
        }
    }

    pub fn remove_dynamic_peer(&self, node: &NodeId) {
        self.inner
            .dynamic_peers
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(node);
        if let Some(links) = self.gateway() {
            links.remove_peer(node);
        }
    }

    /// Answers `service` with `handler` from now on, replacing an earlier one.
    pub fn attach(&self, service: Service, handler: Arc<dyn NodeService>) {
        let mut attached = self.inner.attached.write().unwrap_or_else(PoisonError::into_inner);
        attached.insert(service, handler);
        self.rebuild(&attached);
        drop(attached);
        self.bind_gateway();
    }

    /// The gateway's node links, each time the gateway (re)starts with them; `None` when it stops.
    pub fn set_gateway(&self, links: Option<Arc<dyn GatewayLinks>>) {
        *self.inner.gateway.write().unwrap_or_else(PoisonError::into_inner) = links;
        self.bind_gateway();
    }

    fn gateway(&self) -> Option<Arc<dyn GatewayLinks>> {
        self.inner
            .gateway
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Plugs [`NodeServices`] into the gateway's `/_rr/node/*` endpoint once [`Self::set_gateway`] has its links.
    fn bind_gateway(&self) {
        if let Some(links) = self.gateway() {
            links.attach(Arc::new(self.clone()));
            let peers: Vec<NodeId> = self
                .inner
                .dynamic_peers
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .cloned()
                .collect();
            for p in peers {
                links.add_peer(p);
            }
            if let Some(cb) = self.enroll_handler() {
                links.set_enroll_handler(Some(cb));
            } else {
                links.set_enroll_handler(None);
            }
        }
    }

    pub fn detach(&self, service: Service) {
        let mut attached = self.inner.attached.write().unwrap_or_else(PoisonError::into_inner);
        if attached.remove(&service).is_some() {
            self.rebuild(&attached);
        }
    }

    pub fn is_attached(&self, service: Service) -> bool {
        self.inner
            .attached
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(&service)
    }

    fn rebuild(&self, attached: &HashMap<Service, Arc<dyn NodeService>>) {
        let mux = attached
            .iter()
            .fold(ServiceMux::new(self.inner.node.clone()), |mux, (service, handler)| {
                mux.with(*service, handler.clone())
            });
        *self.inner.mux.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(mux);
    }
}

impl NodeService for NodeServices {
    fn serve<'a>(
        &'a self,
        from: &'a NodeId,
        service: Service,
        body: Vec<u8>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LinkError>> {
        let mux = self.inner.mux.read().unwrap_or_else(PoisonError::into_inner).clone();
        Box::pin(async move { mux.serve(from, service, body).await })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    struct Echo(&'static str);

    impl NodeService for Echo {
        fn serve<'a>(&'a self, _: &'a NodeId, _: Service, body: Vec<u8>) -> BoxFuture<'a, Result<Vec<u8>, LinkError>> {
            Box::pin(async move { Ok([self.0.as_bytes(), &body].concat()) })
        }
    }

    #[tokio::test]
    async fn a_service_is_answered_while_attached_and_not_before_or_after() {
        let node: NodeId = "oregon-1".parse().unwrap();
        let from: NodeId = "uk-1".parse().unwrap();
        let services = NodeServices::new(node);
        let unserved = |error: Result<Vec<u8>, LinkError>| matches!(error, Err(LinkError::NoService { .. }));
        assert!(unserved(services.serve(&from, Service::Home, vec![]).await));
        services.attach(Service::Home, Arc::new(Echo("home:")));
        services.attach(Service::Directory, Arc::new(Echo("dir:")));
        assert_eq!(
            services.serve(&from, Service::Home, b"x".to_vec()).await.unwrap(),
            b"home:x"
        );
        assert_eq!(
            services.serve(&from, Service::Directory, b"y".to_vec()).await.unwrap(),
            b"dir:y"
        );
        // Replaced, then taken back: the other is untouched
        services.attach(Service::Home, Arc::new(Echo("new:")));
        assert_eq!(services.serve(&from, Service::Home, vec![]).await.unwrap(), b"new:");
        services.detach(Service::Home);
        assert!(!services.is_attached(Service::Home) && services.is_attached(Service::Directory));
        assert!(unserved(services.serve(&from, Service::Home, vec![]).await));
        assert_eq!(
            services.serve(&from, Service::Directory, vec![]).await.unwrap(),
            b"dir:"
        );
    }
}
