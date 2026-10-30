// SPDX-License-Identifier: MIT
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
    #[cfg(feature = "gateway")]
    gateway_slot: gateway::server::NodeLinkSlot,
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
                #[cfg(feature = "gateway")]
                gateway_slot: Arc::new(std::sync::Mutex::new(None)),
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
        #[cfg(feature = "gateway")]
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
        #[cfg(feature = "gateway")]
        if let Some(links) = self
            .inner
            .gateway_slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .cloned()
        {
            links.endpoint().add_peer(node);
        }
    }

    pub fn remove_dynamic_peer(&self, node: &NodeId) {
        self.inner
            .dynamic_peers
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(node);
        #[cfg(feature = "gateway")]
        if let Some(links) = self
            .inner
            .gateway_slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .cloned()
        {
            links.endpoint().remove_peer(node);
        }
    }

    /// Answers `service` with `handler` from now on, replacing an earlier one.
    pub fn attach(&self, service: Service, handler: Arc<dyn NodeService>) {
        let mut attached = self.inner.attached.write().unwrap_or_else(PoisonError::into_inner);
        attached.insert(service, handler);
        self.rebuild(&attached);
        drop(attached);
        #[cfg(feature = "gateway")]
        self.bind_gateway();
    }

    #[cfg(feature = "gateway")]
    pub fn gateway_slot(&self) -> gateway::server::NodeLinkSlot {
        self.inner.gateway_slot.clone()
    }

    /// Plugs [`NodeServices`] into the gateway's `/_rr/node/*` endpoint once the gateway has placed its
    /// [`gateway::nodelink::NodeLinks`] in [`Self::gateway_slot`].
    #[cfg(feature = "gateway")]
    pub fn bind_gateway(&self) {
        if let Some(links) = self
            .inner
            .gateway_slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .cloned()
        {
            links.endpoint().attach(Arc::new(self.clone()));
            let peers: Vec<NodeId> = self
                .inner
                .dynamic_peers
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .cloned()
                .collect();
            for p in peers {
                links.endpoint().add_peer(p);
            }
            if let Some(cb) = self.enroll_handler() {
                links.set_enroll_handler(Some(Arc::new(move |body| cb(body))));
            } else {
                links.set_enroll_handler(None);
            }
        }
    }

    /// Builds a [`gateway::network::RegionReportHandler`] that forwards `POST /network/v1/regions` reports
    /// to [`network_plugin::ring::RingNode::record_region_pings_authenticated`] when the network plugin is
    /// running, or accepts standalone shape-checked reports when no network ring is active (DN7, DN8).
    #[cfg(all(feature = "gateway", feature = "network"))]
    pub fn region_report_handler(&self) -> gateway::network::RegionReportHandler {
        let services = self.clone();
        Arc::new(move |api_key, ticket_token, regions| {
            let services = services.clone();
            Box::pin(async move {
                let Some(ring) = services.ring() else {
                    return true;
                };
                let mut pings = Vec::with_capacity(regions.len());
                for r in regions {
                    let Ok(region) = r.region.parse() else {
                        return false;
                    };
                    let rtt_ms = r.rtt_ms.and_then(|ms| {
                        if ms.is_finite() && (0.0..=f64::from(u32::MAX)).contains(&ms) {
                            Some(ms.round() as u32)
                        } else {
                            None
                        }
                    });
                    pings.push(network_plugin::ring::RegionPing { region, rtt_ms });
                }
                let parsed_ticket = match ticket_token.as_deref() {
                    Some(token) => match network_plugin::home::ticket::session_ticket_from_token(token) {
                        Ok(pair) => Some(pair),
                        Err(_) => return false,
                    },
                    None => None,
                };
                let ticket_ref = parsed_ticket.as_ref().map(|(t, g)| (t, g));
                ring.record_region_pings_authenticated(api_key.as_deref(), ticket_ref, pings)
                    .await
                    .is_ok()
            })
        })
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
