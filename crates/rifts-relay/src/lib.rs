// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The co-op transport's network side: the ICE servers and TURN credentials a session's transports use, the pair
//! keys that bind a joiner to its host, the ICE signal relay and the admin `coop-transport` switch. It listens on
//! nothing; the module supplies HTTPS to the relay provider through [`ProviderHttp`].

mod config;
mod credentials;
mod routes;
pub mod rules;
mod service;
pub mod wire;

use rifts_seam::{
    AccountId, BoxFuture, Component, Json, Refusal, Reply, Secret, SessionId, StoreError, Unimplemented, UnixMillis,
    stub,
};

pub use crate::config::{
    CloudflareAnalytics, CloudflareTurn, RelayConfig, SECRET_SETTINGS, TurnProvider, TurnRelayPorts, TurnRest,
    is_stun_url, is_turn_url,
};
pub use crate::credentials::{
    CLOUDFLARE_GRAPHQL_API, CLOUDFLARE_RTC_API, CloudflareIssuer, HourlyCounter, IceServer, IssuedCredential,
    PROVIDER_TIMEOUT_MS, ProviderAnswer, ProviderFailure, ProviderHttp, ProviderIceServers, ProviderRequest,
    ProviderUnreachable, RelayDenied, RelayGrant, RelayGrants, RelayStatus, RelayUsageCache, TurnRestIssuer,
    mint_cloudflare, mint_turn_rest, month_window, normalize_provider_ice_servers, read_monthly_relay_egress_bytes,
    relay_account_tag, relay_ttl_seconds,
};
pub use crate::routes::{ROUTES, RelayRoutes, stub_routes};
pub use crate::rules::{CoopTransport, HuntSnapshot, derive_pair_key};
pub use crate::service::{ControlMessage, CoopTransportOutcome, Ice, IceHunts, OperationsPolicy, P2PGate, refusal};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RelayError {
    #[error(transparent)]
    Unimplemented(#[from] Unimplemented),
    #[error(transparent)]
    Refused(#[from] Refusal),
    #[error(transparent)]
    Provider(#[from] ProviderFailure),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("the system random source failed")]
    Random,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayCredential {
    pub username: String,
    pub credential: Secret,
    pub urls: Vec<String>,
    pub expires_at: UnixMillis,
}

pub trait CredentialIssuer: Send + Sync {
    fn provider(&self) -> TurnProvider;
    fn issue(
        &self,
        account: AccountId,
        ttl_seconds: u32,
        now: UnixMillis,
    ) -> BoxFuture<'_, Result<IssuedCredential, RelayError>>;
}

pub trait PairKeys: Send + Sync {
    fn pair_key(
        &self,
        wire_session: &str,
        generation: u64,
        host: &AccountId,
        joiner: &AccountId,
    ) -> Result<Secret, RelayError>;
}

/// The transport key secret, when the server has one (`P2P_TRANSPORT_KEY_SECRET`).
#[derive(Debug, Clone)]
pub struct TransportKeys(pub Option<Secret>);

const ICE_UNAVAILABLE: (&str, &str) = (
    "P2P_ICE_UNAVAILABLE",
    "The co-op transport is not configured on this server.",
);

impl PairKeys for TransportKeys {
    fn pair_key(
        &self,
        wire_session: &str,
        generation: u64,
        host: &AccountId,
        joiner: &AccountId,
    ) -> Result<Secret, RelayError> {
        let secret = self
            .0
            .as_ref()
            .ok_or_else(|| refusal(ICE_UNAVAILABLE.0, ICE_UNAVAILABLE.1))?;
        Ok(derive_pair_key(secret, wire_session, generation, host, joiner))
    }
}

pub trait IceService: Send + Sync {
    fn transport_config(
        &self,
        account: AccountId,
        session: SessionId,
        generation: Option<i64>,
    ) -> BoxFuture<'_, Result<Json, RelayError>>;
    fn submit_signal(
        &self,
        account: AccountId,
        session: SessionId,
        body: serde_json::Value,
    ) -> BoxFuture<'_, Result<Json, RelayError>>;
    fn set_coop_transport(
        &self,
        admin_key_id: String,
        body: serde_json::Value,
    ) -> BoxFuture<'_, Result<Reply, RelayError>>;
}

/// Answers every operation as not written yet.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stub;

impl CredentialIssuer for Stub {
    fn provider(&self) -> TurnProvider {
        TurnProvider::None
    }

    fn issue(&self, _: AccountId, _: u32, _: UnixMillis) -> BoxFuture<'_, Result<IssuedCredential, RelayError>> {
        stub(Component::Relay, "issue")
    }
}

impl PairKeys for Stub {
    fn pair_key(&self, _: &str, _: u64, _: &AccountId, _: &AccountId) -> Result<Secret, RelayError> {
        Err(Unimplemented::new(Component::Relay, "pair_key").into())
    }
}

impl IceService for Stub {
    fn transport_config(&self, _: AccountId, _: SessionId, _: Option<i64>) -> BoxFuture<'_, Result<Json, RelayError>> {
        stub(Component::Relay, "transport_config")
    }

    fn submit_signal(
        &self,
        _: AccountId,
        _: SessionId,
        _: serde_json::Value,
    ) -> BoxFuture<'_, Result<Json, RelayError>> {
        stub(Component::Relay, "submit_signal")
    }

    fn set_coop_transport(&self, _: String, _: serde_json::Value) -> BoxFuture<'_, Result<Reply, RelayError>> {
        stub(Component::Relay, "set_coop_transport")
    }
}

#[cfg(test)]
mod tests {
    use rifts_seam::{GlobalId, NodeId, Ulid, poll_once};

    use super::*;

    fn unimplemented(operation: &'static str) -> RelayError {
        RelayError::Unimplemented(Unimplemented::new(Component::Relay, operation))
    }

    #[test]
    fn stubs_answer_unimplemented() {
        let account = AccountId("account-0001".to_owned());
        let session = SessionId(GlobalId {
            ulid: Ulid(1),
            node: NodeId::new("node").unwrap(),
        });
        assert_eq!(CredentialIssuer::provider(&Stub), TurnProvider::None);
        assert_eq!(
            poll_once(Stub.issue(account.clone(), 1, UnixMillis(0))).unwrap(),
            Err(unimplemented("issue"))
        );
        assert_eq!(
            Stub.pair_key("w", 1, &account, &account),
            Err(unimplemented("pair_key"))
        );
        assert_eq!(
            poll_once(Stub.transport_config(account.clone(), session.clone(), None)).unwrap(),
            Err(unimplemented("transport_config"))
        );
        assert_eq!(
            poll_once(Stub.submit_signal(account, session, serde_json::Value::Null)).unwrap(),
            Err(unimplemented("submit_signal"))
        );
        assert_eq!(
            poll_once(Stub.set_coop_transport("k".to_owned(), serde_json::Value::Null)).unwrap(),
            Err(unimplemented("set_coop_transport"))
        );
    }

    #[test]
    fn transport_keys() {
        let host = AccountId("host-0001".to_owned());
        let joiner = AccountId("joiner-01".to_owned());
        let refused = TransportKeys(None).pair_key("w", 1, &host, &joiner).unwrap_err();
        assert_eq!(
            refused.to_string(),
            "P2P_ICE_UNAVAILABLE: The co-op transport is not configured on this server."
        );
        let key = TransportKeys(Some(Secret::new("k".repeat(32))))
            .pair_key("w", 1, &host, &joiner)
            .unwrap();
        assert_eq!(key.expose().len(), 43);
        assert_eq!(RelayError::Random.to_string(), "the system random source failed");
    }
}
