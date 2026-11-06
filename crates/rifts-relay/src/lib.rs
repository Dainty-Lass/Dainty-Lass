// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The co-op transport's network side: the ICE servers and TURN credentials a session's transports use, the pair
//! keys that bind a joiner to its host, the ICE signal relay and the admin `coop-transport` switch. It listens on
//! nothing; the module supplies HTTPS to the relay provider through [`ProviderHttp`].

use std::error::Error as StdError;
use std::fmt;

use rifts_seam as seam;
use seam::{AccountId, Json, Reply, Secret, SessionId, UnixMillis};
use serde_json::Value;

/// An enum whose variants travel as fixed words: one table gives the variants, `as_str` and, when asked for,
/// `parse`. Defined before the modules so each of them can use it.
macro_rules! wire_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $word:literal,)+ }) => {
        $(#[$meta])*
        #[allow(missing_docs, reason = "the variant table takes no attributes; each variant is its wire word")]
        pub enum $name {
            $($variant,)+
        }

        impl $name {
            /// The variant's wire word.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $word,)+
                }
            }
        }
    };
    ($(#[$meta:meta])* $name:ident, read by $reader:vis parse { $($variant:ident = $word:literal,)+ }) => {
        wire_enum!($(#[$meta])* $name { $($variant = $word,)+ });

        impl $name {
            /// The variant for a wire word; `None` for a word it does not know.
            $reader fn parse(text: &str) -> Option<Self> {
                [$(Self::$variant,)+].into_iter().find(|known| known.as_str() == text)
            }
        }
    };
}

pub mod rules;
pub mod wire;

mod config;
mod credentials;
mod routes;
mod service;

pub use crate::{
    config::{
        CloudflareAnalytics, CloudflareTurn, RelayConfig, SECRET_SETTINGS, TurnProvider, TurnRelayPorts, TurnRest,
        is_stun_url, is_turn_url,
    },
    credentials::{
        CLOUDFLARE_GRAPHQL_API, CLOUDFLARE_RTC_API, CloudflareIssuer, HourlyCounter, IceServer, IssuedCredential,
        PROVIDER_TIMEOUT_MS, ProviderAnswer, ProviderFailure, ProviderHttp, ProviderIceServers, ProviderRequest,
        ProviderUnreachable, RelayDenied, RelayGrant, RelayGrants, RelayStatus, RelayUsageCache, TurnRestIssuer,
        mint_cloudflare, mint_turn_rest, month_window, normalize_provider_ice_servers, read_monthly_relay_egress_bytes,
        relay_account_tag, relay_ttl_seconds,
    },
    routes::{ROUTES, RelayRoutes, stub_routes},
    rules::{CoopTransport, HuntSnapshot, derive_pair_key},
    service::{ControlMessage, CoopTransportOutcome, Ice, IceHunts, OperationsPolicy, P2PGate, refusal},
};

/// What every relay operation resolves to.
type Answer<'a, T> = seam::BoxFuture<'a, Result<T, RelayError>>;

/// Why a relay operation failed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RelayError {
    /// The operation is not written yet.
    Unimplemented(seam::Unimplemented),
    /// A refusal with a status code, answered to the caller.
    Refused(seam::Refusal),
    /// The relay provider failed.
    Provider(ProviderFailure),
    /// The store failed.
    Store(seam::StoreError),
    /// The system random source failed.
    Random,
}

impl RelayError {
    /// The wrapped error every variant but `Random` shows through unchanged.
    fn wrapped(&self) -> Option<&(dyn StdError + 'static)> {
        Some(match self {
            Self::Unimplemented(inner) => inner,
            Self::Refused(inner) => inner,
            Self::Provider(inner) => inner,
            Self::Store(inner) => inner,
            Self::Random => return None,
        })
    }
}

impl fmt::Display for RelayError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.wrapped() {
            Some(inner) => fmt::Display::fmt(inner, out),
            None => out.write_str("the system random source failed"),
        }
    }
}

impl StdError for RelayError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.wrapped()?.source()
    }
}

macro_rules! relay_error_from {
    ($($variant:ident <- $inner:ty),* $(,)?) => {$(
        impl From<$inner> for RelayError {
            fn from(inner: $inner) -> Self {
                Self::$variant(inner)
            }
        }
    )*};
}

relay_error_from! {
    Unimplemented <- seam::Unimplemented,
    Refused <- seam::Refusal,
    Provider <- ProviderFailure,
    Store <- seam::StoreError,
}

/// A TURN credential for one account.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelayCredential {
    /// The TURN username.
    pub username: String,
    /// The TURN password; never logged.
    pub credential: Secret,
    /// The TURN URLs it works on.
    pub urls: Vec<String>,
    /// When it stops working.
    pub expires_at: UnixMillis,
}

/// Issues TURN credentials from the configured provider.
pub trait CredentialIssuer: Send + Sync {
    /// The provider it issues from.
    fn provider(&self) -> TurnProvider;
    /// A credential for `to`, valid for `ttl_seconds` from `at`.
    fn issue(&self, to: AccountId, ttl_seconds: u32, at: UnixMillis) -> Answer<'_, IssuedCredential>;
}

/// Gives a host and a joiner the key their transports share.
pub trait PairKeys: Send + Sync {
    /// The key for this host and joiner in this session's generation; a refusal when the server has no transport key secret.
    fn pair_key(
        &self,
        session: &str,
        generation: u64,
        host: &AccountId,
        joiner: &AccountId,
    ) -> Result<Secret, RelayError>;
}

/// The transport key secret, when the server has one (`P2P_TRANSPORT_KEY_SECRET`).
#[derive(Clone, Debug)]
pub struct TransportKeys(pub Option<Secret>);

impl PairKeys for TransportKeys {
    fn pair_key(
        &self,
        session: &str,
        generation: u64,
        host: &AccountId,
        joiner: &AccountId,
    ) -> Result<Secret, RelayError> {
        match &self.0 {
            Some(secret) => Ok(derive_pair_key(secret, session, generation, host, joiner)),
            None => Err(refusal(
                "P2P_ICE_UNAVAILABLE",
                "The co-op transport is not configured on this server.",
            )),
        }
    }
}

/// The relay's operations, as the routes call them.
pub trait IceService: Send + Sync {
    /// The ICE servers, relay credential and peers `who` uses in the hunt; refused when hosting is stopped, the hunt is not live or `who` is not in it.
    fn transport_config(&self, who: AccountId, hunt: SessionId, generation: Option<i64>) -> Answer<'_, Json>;
    /// Validates an ICE signal from `who` and queues it for its addressee.
    fn submit_signal(&self, who: AccountId, hunt: SessionId, body: Value) -> Answer<'_, Json>;
    /// The admin switch between co-op transports; answers 400, 200 or 409 itself.
    fn set_coop_transport(&self, admin: String, body: Value) -> Answer<'_, Reply>;
}

/// Answers every operation as not written yet.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stub;

fn not_yet<'a, T: Send + 'static>(operation: &'static str) -> Answer<'a, T> {
    seam::stub(seam::Component::Relay, operation)
}

impl CredentialIssuer for Stub {
    fn provider(&self) -> TurnProvider {
        TurnProvider::None
    }

    fn issue(&self, _: AccountId, _: u32, _: UnixMillis) -> Answer<'_, IssuedCredential> {
        not_yet("issue")
    }
}

impl PairKeys for Stub {
    fn pair_key(&self, _: &str, _: u64, _: &AccountId, _: &AccountId) -> Result<Secret, RelayError> {
        Err(seam::Unimplemented::new(seam::Component::Relay, "pair_key").into())
    }
}

impl IceService for Stub {
    fn transport_config(&self, _: AccountId, _: SessionId, _: Option<i64>) -> Answer<'_, Json> {
        not_yet("transport_config")
    }

    fn submit_signal(&self, _: AccountId, _: SessionId, _: Value) -> Answer<'_, Json> {
        not_yet("submit_signal")
    }

    fn set_coop_transport(&self, _: String, _: Value) -> Answer<'_, Reply> {
        not_yet("set_coop_transport")
    }
}

#[cfg(test)]
mod tests {
    use seam::{GlobalId, NodeId, Ulid, poll_once};

    use super::*;

    fn account(name: &str) -> AccountId {
        AccountId(name.to_owned())
    }

    #[test]
    fn stubs_answer_unimplemented() {
        let who = account("account-0001");
        let hunt = SessionId(GlobalId {
            node: NodeId::new("node").unwrap(),
            ulid: Ulid(1),
        });
        assert_eq!(CredentialIssuer::provider(&Stub), TurnProvider::None);
        let answers = [
            (
                "issue",
                poll_once(Stub.issue(who.clone(), 1, UnixMillis(0))).unwrap().map(drop),
            ),
            ("pair_key", Stub.pair_key("w", 1, &who, &who).map(drop)),
            (
                "transport_config",
                poll_once(Stub.transport_config(who.clone(), hunt.clone(), None))
                    .unwrap()
                    .map(drop),
            ),
            (
                "submit_signal",
                poll_once(Stub.submit_signal(who, hunt, Value::Null)).unwrap().map(drop),
            ),
            (
                "set_coop_transport",
                poll_once(Stub.set_coop_transport("k".to_owned(), Value::Null))
                    .unwrap()
                    .map(drop),
            ),
        ];
        for (operation, answer) in answers {
            let expected = RelayError::Unimplemented(seam::Unimplemented::new(seam::Component::Relay, operation));
            assert_eq!(answer, Err(expected), "{operation}");
        }
    }

    #[test]
    fn transport_keys() {
        let (host, joiner) = (account("host-0001"), account("joiner-01"));
        let missing = TransportKeys(None).pair_key("w", 1, &host, &joiner).unwrap_err();
        assert_eq!(
            missing.to_string(),
            "P2P_ICE_UNAVAILABLE: The co-op transport is not configured on this server."
        );
        let keys = TransportKeys(Some(Secret::new("k".repeat(32))));
        assert_eq!(keys.pair_key("w", 1, &host, &joiner).unwrap().expose().len(), 43);
        assert_eq!(RelayError::Random.to_string(), "the system random source failed");
    }
}
