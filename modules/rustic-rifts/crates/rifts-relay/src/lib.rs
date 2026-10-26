//! Relay (RF4): the ICE servers and TURN credentials a session's transports use, and the pair keys that bind a
//! joiner to its host.
//!
//! Ported from `p2p/config/p2pIcePolicy.ts`, `p2p/controllers/p2pIce.ts` and the `coop-transport` admin route (RF1).
//! The TURN relay itself is turn-rs, run by the host kit as its own process (DD24): this crate describes its ports
//! ([`TurnRelayPorts`]) and mints its credentials, and listens on nothing.

mod config;
mod routes;

pub use config::{RelayConfig, RelaySecrets, TurnProvider, TurnRelayPorts};
pub use routes::{ROUTES, stub_routes};

use rifts_seam::{AccountId, BoxFuture, Component, PeerId, Secret, SessionId, Unimplemented, UnixMillis, stub};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RelayError {
    #[error(transparent)]
    Unimplemented(#[from] Unimplemented),
}

/// Who the credential is for: a hub keeps its credential for the hub's life (`P2P_REALM_HUB_TURN_TTL_SECONDS`), since
/// no transport exports `UpdateRelayCredentialH` (RF1, decision 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialPurpose {
    Hunt,
    Hub,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialRequest {
    pub account: AccountId,
    pub session: SessionId,
    pub purpose: CredentialPurpose,
}

/// `transport.secret`'s `turn_username` and `turn_credential`, with the URLs they are good for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayCredential {
    pub username: String,
    pub credential: Secret,
    pub urls: Vec<String>,
    pub expires_at: UnixMillis,
}

/// One provider's credentials: `turn-rest` (HMAC over the shared secret, for turn-rs and coturn) or Cloudflare's API.
pub trait CredentialIssuer: Send + Sync {
    fn provider(&self) -> TurnProvider;
    fn issue(&self, request: CredentialRequest) -> BoxFuture<'_, Result<RelayCredential, RelayError>>;
}

/// The per-pair keys (`P2P_TRANSPORT_KEY_SECRET`) the transport authenticates peers with.
pub trait PairKeys: Send + Sync {
    /// 43 base64url characters.
    fn pair_key(&self, session: &SessionId, generation: u64, account: &AccountId) -> Result<Secret, RelayError>;
}

/// `POST .../transport-config`: what goes into `bridge.conf` and `transport.secret`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportConfig {
    pub stun_urls: Vec<String>,
    pub turn_urls: Vec<String>,
    pub credential: Option<RelayCredential>,
    /// `force_relay` (`P2P_FORCE_SDR`): relay-only, no direct path.
    pub force_relay: bool,
    /// `peer=<accountId>,<peerId>,<pairKey>` lines.
    pub peers: Vec<(AccountId, PeerId, Secret)>,
}

/// `POST .../signal`, queued to the other side as `ICE_SIGNAL`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IceSignal {
    pub to: AccountId,
    pub kind: String,
    /// Up to 4097 bytes, the transport's `ReadSignal` buffer.
    pub data: String,
}

/// `operationsPolicy.coopTransport`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoopTransport {
    Ice,
    SteamExperimental,
}

/// The relay routes' operations.
pub trait IceService: Send + Sync {
    /// `CreateIceTransportConfig`; `P2P_ICE_CONFIGS_PER_HOUR` per account.
    fn transport_config(
        &self,
        account: AccountId,
        session: SessionId,
    ) -> BoxFuture<'_, Result<TransportConfig, RelayError>>;
    /// `SubmitIceSignal`.
    fn submit_signal(
        &self,
        account: AccountId,
        session: SessionId,
        signal: IceSignal,
    ) -> BoxFuture<'_, Result<(), RelayError>>;
    /// `UpdateCoopTransport`.
    fn set_coop_transport(&self, admin_key: String, mode: CoopTransport) -> BoxFuture<'_, Result<(), RelayError>>;
}

/// Every operation answers [`Unimplemented`] until RF4.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stub;

const COMPONENT: Component = Component::Relay;

impl CredentialIssuer for Stub {
    fn provider(&self) -> TurnProvider {
        TurnProvider::None
    }
    fn issue(&self, _: CredentialRequest) -> BoxFuture<'_, Result<RelayCredential, RelayError>> {
        stub(COMPONENT, "issue")
    }
}

impl PairKeys for Stub {
    fn pair_key(&self, _: &SessionId, _: u64, _: &AccountId) -> Result<Secret, RelayError> {
        Err(Unimplemented::new(COMPONENT, "pair_key").into())
    }
}

impl IceService for Stub {
    fn transport_config(&self, _: AccountId, _: SessionId) -> BoxFuture<'_, Result<TransportConfig, RelayError>> {
        stub(COMPONENT, "transport_config")
    }
    fn submit_signal(&self, _: AccountId, _: SessionId, _: IceSignal) -> BoxFuture<'_, Result<(), RelayError>> {
        stub(COMPONENT, "submit_signal")
    }
    fn set_coop_transport(&self, _: String, _: CoopTransport) -> BoxFuture<'_, Result<(), RelayError>> {
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
        let request = CredentialRequest {
            account: account.clone(),
            session: session.clone(),
            purpose: CredentialPurpose::Hub,
        };
        assert_eq!(poll_once(Stub.issue(request)).unwrap(), Err(unimplemented("issue")));
        assert_eq!(Stub.pair_key(&session, 1, &account), Err(unimplemented("pair_key")));
        assert_eq!(
            poll_once(Stub.transport_config(account, session)).unwrap(),
            Err(unimplemented("transport_config"))
        );
    }
}
