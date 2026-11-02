// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! PLACEHOLDER for a Category A file (DN3, RM2). The real `nodelink.rs` lives in dainty-restricted and moves here
//! unchanged at integration (docs/port/clean-room/gateway.md); the clean-room implementer never saw it. This
//! stand-in holds only what the rewritten gateway calls, from spec-rd27-gateway-front.md sections 9.1 and 9.2.

use std::sync::{Arc, PoisonError, RwLock};

use module_api::BoxFuture;
use module_api::network::{NodeId, NodeIdentity};
use network_plugin::NodeEndpoint;
use network_plugin::cluster::{self, NodeCertInfo, NodeCredentials};
use rustls::pki_types::CertificateDer;

use crate::config::NodeLinkConfig;

/// `POST /__node/enroll`'s answer from the core: a status and a JSON body.
pub type EnrollHandler = Arc<dyn Fn(Vec<u8>) -> BoxFuture<'static, (u16, Vec<u8>)> + Send + Sync>;

/// This node's links on the public listener.
pub struct NodeLinks {
    credentials: NodeCredentials,
    endpoint: Arc<NodeEndpoint>,
    server_config: Arc<rustls::ServerConfig>,
    enroll: RwLock<Option<EnrollHandler>>,
}

impl NodeLinks {
    pub fn load(config: &NodeLinkConfig) -> Result<Self, String> {
        let credentials = NodeCredentials::load(&config.cert_file, &config.key_file, &config.ca_file).map_err(|e| {
            format!(
                "Node certificate {} / key {} / CA {}: {e}",
                config.cert_file.display(),
                config.key_file.display(),
                config.ca_file.display()
            )
        })?;
        let theirs = credentials.identity();
        if theirs != &config.identity {
            return Err(format!(
                "The node certificate is for {} in {}, but this node is {} in {} (GATEWAY_NODE_ID, GATEWAY_NODE_REGION)",
                theirs.node, theirs.region, config.identity.node, config.identity.region
            ));
        }
        let server_config = credentials.server_config().map_err(|e| e.to_string())?;
        let endpoint = Arc::new(NodeEndpoint::new(
            config.identity.clone(),
            config.peers.iter().map(|p| p.id.clone()),
        ));
        Ok(Self {
            credentials,
            endpoint,
            server_config,
            enroll: RwLock::default(),
        })
    }

    pub fn identity(&self) -> &NodeIdentity {
        self.credentials.identity()
    }

    pub fn info(&self) -> &NodeCertInfo {
        self.credentials.info()
    }

    pub fn endpoint(&self) -> &Arc<NodeEndpoint> {
        &self.endpoint
    }

    /// Mutual TLS, offered only to a client that asks for the node ALPN name.
    pub fn server_config(&self) -> Arc<rustls::ServerConfig> {
        self.server_config.clone()
    }

    /// The node a verified chain names, if any.
    pub fn caller(chain: &[CertificateDer<'_>]) -> Option<NodeId> {
        cluster::peer_identity(chain).ok().map(|identity| identity.node)
    }

    pub fn set_enroll(&self, handler: Option<EnrollHandler>) {
        *self.enroll.write().unwrap_or_else(PoisonError::into_inner) = handler;
    }

    pub fn enroll(&self) -> Option<EnrollHandler> {
        self.enroll.read().unwrap_or_else(PoisonError::into_inner).clone()
    }
}
