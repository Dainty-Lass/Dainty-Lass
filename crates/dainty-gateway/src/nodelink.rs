// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Node links on the public port (DN3, DD16): the gateway terminates mutual TLS for the other nodes of a
//! network on the listener it already has. A client that offers the node ALPN name
//! ([`NODE_ALPN`]) is shown the node certificate and must present one
//! that chains to the cluster CA; every other client gets the gateway's public certificate and the public
//! policy, unchanged, and `/__node` stays refused for it (`modules.rs`). On a node connection only
//! `/__node/*` is served: nothing reaches the game, content or chat, and a peer's calls are never
//! captured. Who the caller is comes from its verified certificate and nothing else.

use std::sync::Arc;

use dainty_module_api::network::NodeId;
use network_plugin::cluster::{NODE_ALPN, NodeCertInfo, NodeCredentials, peer_identity};
use network_plugin::endpoint::NodeEndpoint;
use rustls::pki_types::CertificateDer;

use crate::config::NodeLinkConfig;

/// A server's node-link side: its endpoint (who may call, which service answers) and the TLS settings.
pub struct NodeLinks {
    endpoint: NodeEndpoint,
    tls: Arc<rustls::ServerConfig>,
    info: NodeCertInfo,
    enroll_handler: std::sync::RwLock<Option<crate::network::EnrollHandler>>,
}

/// Who is on the other end of a node connection, as its certificate says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeCaller {
    /// The caller's node id.
    pub node: NodeId,
}

impl NodeLinks {
    /// Reads the node's certificate, key and the CA's certificate, and refuses a certificate that chains
    /// to another CA, is not valid now, or names a node other than `GATEWAY_NODE_ID`.
    pub fn load(config: &NodeLinkConfig) -> Result<Self, String> {
        let credentials = NodeCredentials::load(&config.cert_file, &config.key_file, &config.ca_file).map_err(|e| {
            format!(
                "Node certificate {} / key {} / CA {}: {e}",
                config.cert_file.display(),
                config.key_file.display(),
                config.ca_file.display()
            )
        })?;
        if credentials.identity() != &config.identity {
            return Err(format!(
                "The node certificate is for {} in {}, but this node is {} in {} (GATEWAY_NODE_ID, GATEWAY_NODE_REGION)",
                credentials.identity().node,
                credentials.identity().region,
                config.identity.node,
                config.identity.region
            ));
        }
        let tls = credentials.server_config().map_err(|e| e.to_string())?;
        let endpoint = NodeEndpoint::new(config.identity.clone(), config.peers.iter().map(|peer| peer.id.clone()));
        Ok(Self {
            endpoint,
            tls,
            info: credentials.info().clone(),
            enroll_handler: std::sync::RwLock::new(None),
        })
    }

    /// Attach the [`NodeService`](network_plugin::NodeService) that answers calls here.
    pub fn endpoint(&self) -> &NodeEndpoint {
        &self.endpoint
    }

    /// Sets or clears what redeems `POST /__node/enroll`.
    pub fn set_enroll_handler(&self, handler: Option<crate::network::EnrollHandler>) {
        *self
            .enroll_handler
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = handler;
    }

    /// What redeems `POST /__node/enroll`, if set.
    pub fn enroll_handler(&self) -> Option<crate::network::EnrollHandler> {
        self.enroll_handler
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// TLS server settings for node connections.
    pub fn server_config(&self) -> &Arc<rustls::ServerConfig> {
        &self.tls
    }

    /// The node certificate's details.
    pub fn certificate(&self) -> &NodeCertInfo {
        &self.info
    }
}

/// Whether a client asks for a node link: it offers the node ALPN name.
pub fn is_node_hello(hello: &rustls::server::ClientHello<'_>) -> bool {
    hello
        .alpn()
        .is_some_and(|mut names| names.any(|name| name == NODE_ALPN))
}

/// The caller of a connection whose handshake the node-link settings completed (so the client certificate
/// chained to the cluster CA). `None` if the certificate does not name a node.
pub fn caller(chain: Option<&[CertificateDer<'_>]>) -> Option<NodeCaller> {
    peer_identity(chain?)
        .ok()
        .map(|identity| NodeCaller { node: identity.node })
}
