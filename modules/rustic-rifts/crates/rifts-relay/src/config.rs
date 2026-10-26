//! Relay settings with Node's defaults (`p2p/config/p2pIcePolicy.ts`, RF1 section 5.1). Reading them from the env
//! file and `host.toml` is RF4's, with the secrets in `secrets.env`.

use std::ops::RangeInclusive;

use rifts_seam::Secret;

/// `P2P_TURN_PROVIDER`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TurnProvider {
    #[default]
    Cloudflare,
    /// turn-rs, or coturn as the fallback (DD24), with the shared secret.
    TurnRest,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayConfig {
    /// `P2P_ICE_STUN_URLS`.
    pub stun_urls: Vec<String>,
    pub provider: TurnProvider,
    /// `P2P_CLOUDFLARE_TURN_KEY_ID`.
    pub cloudflare_key_id: Option<String>,
    /// `P2P_TURN_REST_URLS`: `turn:` or `turns:`.
    pub turn_rest_urls: Vec<String>,
    /// `P2P_TURN_MAX_TTL_SECONDS` (300 to 172800).
    pub credential_ttl_seconds: u32,
    /// `P2P_REALM_HUB_TURN_TTL_SECONDS` (300 to 172800).
    pub hub_credential_ttl_seconds: u32,
    /// `P2P_RELAY_MONTHLY_QUOTA_GB`; 0 is none.
    pub monthly_quota_gb: u32,
    /// `P2P_ICE_CONFIGS_PER_HOUR` (1 to 1000).
    pub configs_per_hour: u32,
    /// `P2P_FORCE_SDR`: relay only (RF1, decision 4 puts it here, not in realms).
    pub force_relay: bool,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            stun_urls: vec!["stun:stun.cloudflare.com:3478".to_owned()],
            provider: TurnProvider::Cloudflare,
            cloudflare_key_id: None,
            turn_rest_urls: Vec::new(),
            credential_ttl_seconds: 10_800,
            hub_credential_ttl_seconds: 86_400,
            monthly_quota_gb: 10,
            configs_per_hour: 30,
            force_relay: false,
        }
    }
}

/// The secrets, made once and never logged.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RelaySecrets {
    /// `P2P_CLOUDFLARE_TURN_API_TOKEN`.
    pub cloudflare_api_token: Option<Secret>,
    /// `P2P_TURN_REST_SECRET`, at least 16 characters; turn-rs' `static-auth-secret` (DD24).
    pub turn_rest_secret: Option<Secret>,
    /// `P2P_TRANSPORT_KEY_SECRET`, at least 32 characters; without it ICE is off.
    pub transport_key_secret: Option<Secret>,
    /// `P2P_CLOUDFLARE_ANALYTICS_API_TOKEN`, for the quota.
    pub cloudflare_analytics_token: Option<Secret>,
}

/// The turn-rs process's ports (DD24): the one written exception to "the gateway is the only listener off
/// loopback". The host kit binds them and opens them through the allowlist's firewall code; nothing here listens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnRelayPorts {
    /// UDP and TCP.
    pub listen: u16,
    /// TLS, only when the host has a certificate for it.
    pub tls: Option<u16>,
    /// The relayed ports; coturn's default until RF4 sets turn-rs' and its firewall rule.
    pub relay: RangeInclusive<u16>,
}

impl Default for TurnRelayPorts {
    fn default() -> Self {
        Self {
            listen: 3478,
            tls: None,
            relay: 49152..=65535,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_nodes() {
        let config = RelayConfig::default();
        assert_eq!(config.stun_urls, ["stun:stun.cloudflare.com:3478"]);
        assert_eq!(config.provider, TurnProvider::Cloudflare);
        assert_eq!(
            (config.credential_ttl_seconds, config.hub_credential_ttl_seconds),
            (10_800, 86_400)
        );
        assert!(!config.force_relay);
        assert_eq!(TurnRelayPorts::default().listen, 3478);
    }
}
