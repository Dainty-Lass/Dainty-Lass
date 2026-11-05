//! Relay (RF4): the ICE servers and TURN credentials a session's transports use, and the pair keys that bind a
//! joiner to its host.
//!
//! Ported from `p2p/config/p2pIcePolicy.ts`, `p2p/services/relayCredentials.ts`, `p2p/security/iceSessionRules.ts`,
//! `p2p/controllers/p2pIce.ts` and the `coop-transport` admin route (RF1). The TURN relay itself is turn-rs, run by
//! the host kit as its own process (DD24): this crate describes its ports ([`TurnRelayPorts`]) and mints its
//! credentials ([`TurnRestIssuer`]), and listens on nothing. Cloudflare's API is reached through [`ProviderHttp`],
//! which isles implements with its HTTPS client.

mod config;
mod credentials;
mod routes;
pub mod rules;
mod service;
pub mod wire;

pub use config::{
    CloudflareAnalytics, CloudflareTurn, RelayConfig, SECRET_SETTINGS, TurnProvider, TurnRelayPorts, TurnRest,
    is_stun_url, is_turn_url,
};
pub use credentials::{
    CLOUDFLARE_GRAPHQL_API, CLOUDFLARE_RTC_API, CloudflareIssuer, HourlyCounter, IceServer, IssuedCredential,
    PROVIDER_TIMEOUT_MS, ProviderAnswer, ProviderFailure, ProviderHttp, ProviderIceServers, ProviderRequest,
    ProviderUnreachable, RelayDenied, RelayGrant, RelayGrants, RelayStatus, RelayUsageCache, TurnRestIssuer,
    mint_cloudflare, mint_turn_rest, month_window, normalize_provider_ice_servers, read_monthly_relay_egress_bytes,
    relay_account_tag, relay_ttl_seconds,
};
pub use routes::{ROUTES, RelayRoutes, stub_routes};
pub use rules::{CoopTransport, HuntSnapshot, derive_pair_key};
pub use service::{ControlMessage, CoopTransportOutcome, Ice, IceHunts, OperationsPolicy, P2PGate, refusal};

use rifts_seam::{
    AccountId, BoxFuture, Component, Json, Refusal, Reply, Secret, SessionId, StoreError, Unimplemented, UnixMillis,
    stub,
};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RelayError {
    #[error(transparent)]
    Unimplemented(#[from] Unimplemented),
    /// A launcher error code (`P2P_*`); isles answers it in the launcher envelope with its registered status.
    #[error(transparent)]
    Refused(#[from] Refusal),
    #[error(transparent)]
    Provider(#[from] ProviderFailure),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("the system random source failed")]
    Random,
}

/// `transport.secret`'s `turn_username` and `turn_credential`, with the URLs they are good for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayCredential {
    pub username: String,
    pub credential: Secret,
    pub urls: Vec<String>,
    pub expires_at: UnixMillis,
}

/// One provider's credentials: `turn-rest` (an HMAC over the shared secret, for turn-rs and coturn) or Cloudflare's
/// API. The provider sees [`relay_account_tag`], never the account id.
pub trait CredentialIssuer: Send + Sync {
    fn provider(&self) -> TurnProvider;
    fn issue(
        &self,
        account: AccountId,
        ttl_seconds: u32,
        now: UnixMillis,
    ) -> BoxFuture<'_, Result<IssuedCredential, RelayError>>;
}

/// The per-pair keys (`P2P_TRANSPORT_KEY_SECRET`) the transport authenticates peers with.
pub trait PairKeys: Send + Sync {
    /// 43 base64url characters; `wire_session` is Node's session id text.
    fn pair_key(
        &self,
        wire_session: &str,
        generation: u64,
        host: &AccountId,
        joiner: &AccountId,
    ) -> Result<Secret, RelayError>;
}

/// [`derive_pair_key`] with the configured secret.
#[derive(Debug, Clone)]
pub struct TransportKeys(pub Option<Secret>);

impl PairKeys for TransportKeys {
    fn pair_key(
        &self,
        wire_session: &str,
        generation: u64,
        host: &AccountId,
        joiner: &AccountId,
    ) -> Result<Secret, RelayError> {
        let secret = self.0.as_ref().ok_or_else(|| {
            refusal(
                "P2P_ICE_UNAVAILABLE",
                "The co-op transport is not configured on this server.",
            )
        })?;
        Ok(derive_pair_key(secret, wire_session, generation, host, joiner))
    }
}

/// The relay routes' operations. Each answers the JSON body Node sends, or a [`Refusal`] in the launcher envelope.
pub trait IceService: Send + Sync {
    /// `CreateIceTransportConfig`; `generation` is the body's when it is a safe integer. `P2P_ICE_CONFIGS_PER_HOUR`
    /// per account.
    fn transport_config(
        &self,
        account: AccountId,
        session: SessionId,
        generation: Option<i64>,
    ) -> BoxFuture<'_, Result<Json, RelayError>>;
    /// `SubmitIceSignal`, queued to the other side as `ICE_SIGNAL`.
    fn submit_signal(
        &self,
        account: AccountId,
        session: SessionId,
        body: serde_json::Value,
    ) -> BoxFuture<'_, Result<Json, RelayError>>;
    /// `UpdateCoopTransport`: the whole reply, since its 400 and 409 are the admin API's, not the launcher envelope.
    fn set_coop_transport(
        &self,
        admin_key_id: String,
        body: serde_json::Value,
    ) -> BoxFuture<'_, Result<Reply, RelayError>>;
}

/// Every operation answers [`Unimplemented`]: what isles links before it wires [`Ice`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Stub;

const COMPONENT: Component = Component::Relay;

impl CredentialIssuer for Stub {
    fn provider(&self) -> TurnProvider {
        TurnProvider::None
    }
    fn issue(&self, _: AccountId, _: u32, _: UnixMillis) -> BoxFuture<'_, Result<IssuedCredential, RelayError>> {
        stub(COMPONENT, "issue")
    }
}

impl PairKeys for Stub {
    fn pair_key(&self, _: &str, _: u64, _: &AccountId, _: &AccountId) -> Result<Secret, RelayError> {
        Err(Unimplemented::new(COMPONENT, "pair_key").into())
    }
}

impl IceService for Stub {
    fn transport_config(&self, _: AccountId, _: SessionId, _: Option<i64>) -> BoxFuture<'_, Result<Json, RelayError>> {
        stub(COMPONENT, "transport_config")
    }
    fn submit_signal(
        &self,
        _: AccountId,
        _: SessionId,
        _: serde_json::Value,
    ) -> BoxFuture<'_, Result<Json, RelayError>> {
        stub(COMPONENT, "submit_signal")
    }
    fn set_coop_transport(&self, _: String, _: serde_json::Value) -> BoxFuture<'_, Result<Reply, RelayError>> {
        stub(COMPONENT, "set_coop_transport")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rifts_seam::poll_once;

    fn unimplemented(operation: &'static str) -> RelayError {
        Unimplemented::new(COMPONENT, operation).into()
    }

    #[test]
    fn stub_refuses_credentials_and_keys() {
        let session: SessionId = "01ARZ3NDEKTSV4RRFFQ69G5FAV@local".parse().unwrap();
        let account = AccountId("a".to_owned());
        assert_eq!(
            poll_once(Stub.issue(account.clone(), 600, UnixMillis(0))).unwrap(),
            Err(unimplemented("issue"))
        );
        assert_eq!(
            Stub.pair_key("s", 1, &account, &account),
            Err(unimplemented("pair_key"))
        );
        assert_eq!(
            poll_once(Stub.transport_config(account, session, Some(1))).unwrap(),
            Err(unimplemented("transport_config"))
        );
    }

    #[test]
    fn transport_keys_need_the_secret() {
        let host = AccountId("host".to_owned());
        let joiner = AccountId("joiner".to_owned());
        let refused = TransportKeys(None).pair_key("s", 1, &host, &joiner).unwrap_err();
        assert!(matches!(refused, RelayError::Refused(Refusal { ref code, .. }) if code == "P2P_ICE_UNAVAILABLE"));
        let keys = TransportKeys(Some(Secret::new("k".repeat(32))));
        assert_eq!(keys.pair_key("s", 1, &host, &joiner).unwrap().expose().len(), 43);
    }
}
