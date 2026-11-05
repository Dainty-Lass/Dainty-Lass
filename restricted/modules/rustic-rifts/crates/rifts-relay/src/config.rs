//! Relay settings, read as `p2p/config/p2pIcePolicy.ts` reads them (RF1 section 5.1): a bad value falls back to the
//! default rather than stopping the server, and a provider missing its keys is simply not configured. The secrets
//! ([`SECRET_SETTINGS`]) come from the host kit's `secrets.env`; the rest from the service's env file.

use std::ops::RangeInclusive;

use rifts_seam::{Secret, Settings};

use crate::wire::{is_safe_integer, js_number, js_trim};

/// `P2P_TURN_PROVIDER`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TurnProvider {
    #[default]
    Cloudflare,
    /// turn-rs, or coturn as the fallback (DD24), with the shared secret.
    TurnRest,
    None,
}

impl TurnProvider {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cloudflare => "cloudflare",
            Self::TurnRest => "turn-rest",
            Self::None => "none",
        }
    }
}

/// The settings that are secrets: made once, kept in `secrets.env`, never logged.
pub const SECRET_SETTINGS: &[&str] = &[
    "P2P_CLOUDFLARE_TURN_API_TOKEN",
    "P2P_TURN_REST_SECRET",
    "P2P_CLOUDFLARE_ANALYTICS_API_TOKEN",
    "P2P_TRANSPORT_KEY_SECRET",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudflareTurn {
    /// `P2P_CLOUDFLARE_TURN_KEY_ID`.
    pub key_id: String,
    /// `P2P_CLOUDFLARE_TURN_API_TOKEN`.
    pub api_token: Secret,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnRest {
    /// `P2P_TURN_REST_URLS`: `turn:` or `turns:`.
    pub urls: Vec<String>,
    /// `P2P_TURN_REST_SECRET`, at least 16 characters; turn-rs' `static-auth-secret` (DD24).
    pub secret: Secret,
}

/// Cloudflare's GraphQL analytics, which measure the monthly relay quota.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudflareAnalytics {
    /// `P2P_CLOUDFLARE_ANALYTICS_ACCOUNT_ID`.
    pub account_id: String,
    /// `P2P_CLOUDFLARE_ANALYTICS_API_TOKEN`.
    pub api_token: Secret,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayConfig {
    /// `P2P_ICE_STUN_URLS`.
    pub stun_urls: Vec<String>,
    pub provider: TurnProvider,
    /// Set only when the provider is Cloudflare and both values are valid.
    pub cloudflare: Option<CloudflareTurn>,
    /// Set only when the provider is `turn-rest` and both values are valid.
    pub turn_rest: Option<TurnRest>,
    /// `P2P_TURN_MAX_TTL_SECONDS` (300 to 172800); Cloudflare allows at most 48 hours.
    pub credential_ttl_seconds: u32,
    /// `P2P_REALM_HUB_TURN_TTL_SECONDS` (300 to 172800): a hub's transport can't take a new credential while it runs.
    pub hub_credential_ttl_seconds: u32,
    /// `P2P_RELAY_MONTHLY_QUOTA_GB` in bytes; 0 is none.
    pub monthly_quota_bytes: u64,
    /// `P2P_ICE_CONFIGS_PER_HOUR` (1 to 1000).
    pub configs_per_hour: u32,
    pub analytics: Option<CloudflareAnalytics>,
    /// `P2P_TRANSPORT_KEY_SECRET`, at least 32 characters; without it ICE is off.
    pub transport_key_secret: Option<Secret>,
    /// `P2P_FORCE_SDR`: relay only (RF1, decision 4 puts it here, not in realms).
    pub force_relay: bool,
}

const DEFAULT_STUN_URL: &str = "stun:stun.cloudflare.com:3478";
const GB: u64 = 1_000_000_000;

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            stun_urls: vec![DEFAULT_STUN_URL.to_owned()],
            provider: TurnProvider::Cloudflare,
            cloudflare: None,
            turn_rest: None,
            credential_ttl_seconds: 10_800,
            hub_credential_ttl_seconds: 86_400,
            monthly_quota_bytes: 10 * GB,
            configs_per_hour: 30,
            analytics: None,
            transport_key_secret: None,
            force_relay: false,
        }
    }
}

fn host_and_port(text: &str) -> bool {
    let (host, port) = text
        .split_once(':')
        .map_or((text, None), |(host, port)| (host, Some(port)));
    !host.is_empty()
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && port.is_none_or(|port| (1..=5).contains(&port.len()) && port.bytes().all(|b| b.is_ascii_digit()))
}

/// `^stuns?:[A-Za-z0-9.-]+(?::[0-9]{1,5})?$`
pub fn is_stun_url(url: &str) -> bool {
    url.strip_prefix("stuns:")
        .or_else(|| url.strip_prefix("stun:"))
        .is_some_and(host_and_port)
}

/// `^turns?:[A-Za-z0-9.-]+(?::[0-9]{1,5})?(?:\?transport=(?:udp|tcp))?$`
pub fn is_turn_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("turns:").or_else(|| url.strip_prefix("turn:")) else {
        return false;
    };
    let address = rest
        .strip_suffix("?transport=udp")
        .or_else(|| rest.strip_suffix("?transport=tcp"))
        .unwrap_or(rest);
    host_and_port(address)
}

fn read_urls(value: Option<&str>, valid: fn(&str) -> bool) -> Vec<String> {
    value
        .unwrap_or_default()
        .split(',')
        .map(js_trim)
        .filter(|url| valid(url))
        .map(str::to_owned)
        .collect()
}

fn read_bounded(value: Option<&str>, fallback: u32, min: u32, max: u32) -> u32 {
    let Some(value) = value.filter(|value| !js_trim(value).is_empty()) else {
        return fallback;
    };
    let parsed = js_number(value);
    if is_safe_integer(parsed) && parsed >= f64::from(min) && parsed <= f64::from(max) {
        // Within [min, max] by the check
        parsed as u32
    } else {
        fallback
    }
}

/// `^[A-Za-z0-9._~+/=-]{8,512}$`, after trimming.
fn read_token(value: Option<&str>) -> Option<Secret> {
    let token = js_trim(value?);
    let valid = (8..=512).contains(&token.len())
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._~+/=-".contains(&b));
    valid.then(|| Secret::new(token.to_owned()))
}

/// `^[A-Za-z0-9]{8,64}$`
fn account_tag(value: &str) -> bool {
    (8..=64).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_alphanumeric())
}

impl RelayConfig {
    /// `ReadP2PIceConfig`, plus `P2P_FORCE_SDR` as `p2pPolicy.ts` reads it (trimmed, any case, `true` only).
    pub fn from_settings(settings: &dyn Settings) -> Self {
        let defaults = Self::default();
        let stun = read_urls(settings.get("P2P_ICE_STUN_URLS"), is_stun_url);
        let provider = match settings
            .get("P2P_TURN_PROVIDER")
            .map(|value| js_trim(value).to_lowercase())
        {
            Some(value) if value == "turn-rest" => TurnProvider::TurnRest,
            Some(value) if value == "none" => TurnProvider::None,
            _ => TurnProvider::Cloudflare,
        };

        let key_id = settings.get("P2P_CLOUDFLARE_TURN_KEY_ID").map(js_trim);
        let api_token = read_token(settings.get("P2P_CLOUDFLARE_TURN_API_TOKEN"));
        let cloudflare = match (provider, key_id, api_token) {
            (TurnProvider::Cloudflare, Some(key_id), Some(api_token)) if account_tag(key_id) => Some(CloudflareTurn {
                key_id: key_id.to_owned(),
                api_token,
            }),
            _ => None,
        };

        let rest_urls = read_urls(settings.get("P2P_TURN_REST_URLS"), is_turn_url);
        let rest_secret = settings.get("P2P_TURN_REST_SECRET").map(js_trim);
        let turn_rest = match rest_secret {
            Some(secret) if provider == TurnProvider::TurnRest && !rest_urls.is_empty() && utf16_len(secret) >= 16 => {
                Some(TurnRest {
                    urls: rest_urls,
                    secret: Secret::new(secret.to_owned()),
                })
            }
            _ => None,
        };

        let analytics_account = settings.get("P2P_CLOUDFLARE_ANALYTICS_ACCOUNT_ID").map(js_trim);
        let analytics_token = read_token(settings.get("P2P_CLOUDFLARE_ANALYTICS_API_TOKEN"));
        let analytics = match (analytics_account, analytics_token) {
            (Some(account), Some(api_token)) if account_tag(account) => Some(CloudflareAnalytics {
                account_id: account.to_owned(),
                api_token,
            }),
            _ => None,
        };

        let key_secret = settings.get("P2P_TRANSPORT_KEY_SECRET").map(js_trim);
        Self {
            stun_urls: if stun.is_empty() { defaults.stun_urls } else { stun },
            provider,
            cloudflare,
            turn_rest,
            credential_ttl_seconds: read_bounded(settings.get("P2P_TURN_MAX_TTL_SECONDS"), 10_800, 300, 172_800),
            hub_credential_ttl_seconds: read_bounded(
                settings.get("P2P_REALM_HUB_TURN_TTL_SECONDS"),
                86_400,
                300,
                172_800,
            ),
            monthly_quota_bytes: u64::from(read_bounded(settings.get("P2P_RELAY_MONTHLY_QUOTA_GB"), 10, 0, 100_000))
                * GB,
            configs_per_hour: read_bounded(settings.get("P2P_ICE_CONFIGS_PER_HOUR"), 30, 1, 1_000),
            analytics,
            transport_key_secret: key_secret
                .filter(|secret| utf16_len(secret) >= 32)
                .map(|secret| Secret::new(secret.to_owned())),
            force_relay: settings
                .get("P2P_FORCE_SDR")
                .is_some_and(|value| js_trim(value).eq_ignore_ascii_case("true")),
        }
    }

    /// `RelayConfigured`: the provider can issue relay credentials.
    pub fn relay_configured(&self) -> bool {
        match self.provider {
            TurnProvider::Cloudflare => self.cloudflare.is_some(),
            TurnProvider::TurnRest => self.turn_rest.is_some(),
            TurnProvider::None => false,
        }
    }

    /// `P2PIceSummary`: the startup log fields, with no secret.
    pub fn summary_json(&self) -> String {
        crate::wire::JsonObject::new()
            .number("iceStunServers", self.stun_urls.len())
            .string("iceTurnProvider", self.provider.as_str())
            .boolean("iceRelayConfigured", self.relay_configured())
            .number("iceRelayQuotaGb", self.monthly_quota_bytes / GB)
            .boolean(
                "iceRelayQuotaMeasured",
                self.monthly_quota_bytes > 0 && self.provider == TurnProvider::Cloudflare && self.analytics.is_some(),
            )
            .boolean("iceTransportKeyConfigured", self.transport_key_secret.is_some())
            .finish()
    }
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// The turn-rs process's ports (DD24): the one written exception to "the gateway is the only listener off
/// loopback". The host kit binds them and opens them through the allowlist's firewall code; nothing here listens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnRelayPorts {
    /// UDP and TCP.
    pub listen: u16,
    /// TLS, only when the host has a certificate for it.
    pub tls: Option<u16>,
    /// The relayed ports, which the firewall opens for UDP.
    pub relay: RangeInclusive<u16>,
}

impl Default for TurnRelayPorts {
    fn default() -> Self {
        Self {
            listen: 3478,
            tls: None,
            // coturn's default range; the host kit narrows it (see docs/security/relay-credentials.md)
            relay: 49152..=65535,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn read(pairs: &[(&str, &str)]) -> RelayConfig {
        let settings: HashMap<String, String> = pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
        RelayConfig::from_settings(&settings)
    }

    // relayCredentials.test.ts "config: STUN by default, no relay without credentials, 10 GB quota, bounded values"
    #[test]
    fn defaults_and_bounds_are_nodes() {
        let empty = read(&[]);
        assert_eq!(empty.stun_urls, ["stun:stun.cloudflare.com:3478"]);
        assert_eq!(empty.provider, TurnProvider::Cloudflare);
        assert_eq!(empty.cloudflare, None);
        assert_eq!(empty.transport_key_secret, None);
        assert_eq!(empty.monthly_quota_bytes, 10_000_000_000);
        assert_eq!(empty.credential_ttl_seconds, 10_800);
        assert_eq!(empty, RelayConfig::default());

        let odd = read(&[
            ("P2P_TURN_MAX_TTL_SECONDS", "999999"),
            ("P2P_RELAY_MONTHLY_QUOTA_GB", "-1"),
            ("P2P_TRANSPORT_KEY_SECRET", "short"),
            ("P2P_ICE_STUN_URLS", "http://evil, stun:stun.example.org:3478"),
        ]);
        assert_eq!(odd.credential_ttl_seconds, 10_800);
        assert_eq!(odd.monthly_quota_bytes, 10_000_000_000);
        assert_eq!(odd.transport_key_secret, None);
        assert_eq!(odd.stun_urls, ["stun:stun.example.org:3478"]);

        let rest = read(&[
            ("P2P_TURN_PROVIDER", "turn-rest"),
            ("P2P_TURN_REST_URLS", "turn:turn.example.org:3478?transport=udp"),
            ("P2P_TURN_REST_SECRET", &"x".repeat(16)),
        ]);
        assert_eq!(
            rest.turn_rest.as_ref().map(|rest| rest.urls.clone()),
            Some(vec!["turn:turn.example.org:3478?transport=udp".to_owned()])
        );
        assert!(rest.relay_configured());

        let cloudflare = read(&[
            ("P2P_CLOUDFLARE_TURN_KEY_ID", "0123456789abcdef0123456789abcdef"),
            ("P2P_CLOUDFLARE_TURN_API_TOKEN", "token-value-123456"),
        ]);
        assert!(cloudflare.cloudflare.is_some() && cloudflare.relay_configured());
    }

    #[test]
    fn providers_need_their_keys() {
        let short = read(&[
            ("P2P_TURN_PROVIDER", " TURN-REST "),
            ("P2P_TURN_REST_URLS", "turn:turn.example.org"),
            ("P2P_TURN_REST_SECRET", "too-short"),
        ]);
        assert_eq!(short.provider, TurnProvider::TurnRest);
        assert!(short.turn_rest.is_none() && !short.relay_configured());
        // A Cloudflare key set while the provider is turn-rest is ignored
        let mixed = read(&[
            ("P2P_TURN_PROVIDER", "turn-rest"),
            ("P2P_CLOUDFLARE_TURN_KEY_ID", "0123456789abcdef"),
            ("P2P_CLOUDFLARE_TURN_API_TOKEN", "token-value-123456"),
        ]);
        assert!(mixed.cloudflare.is_none());
        assert_eq!(read(&[("P2P_TURN_PROVIDER", "none")]).provider, TurnProvider::None);
        assert_eq!(
            read(&[("P2P_TURN_PROVIDER", "coturn")]).provider,
            TurnProvider::Cloudflare
        );
    }

    #[test]
    fn urls_follow_nodes_patterns() {
        assert!(is_turn_url("turns:turn.example.org:5349?transport=tcp"));
        assert!(is_turn_url("turn:1.2.3.4"));
        assert!(!is_turn_url("turn:host:123456"));
        assert!(!is_turn_url("turn:host?transport=sctp"));
        assert!(!is_turn_url("turn::3478"));
        assert!(!is_stun_url("stun:host?transport=udp"));
        assert!(is_stun_url("stuns:host:5349"));
    }

    #[test]
    fn numbers_and_flags_read_as_node_reads_them() {
        let config = read(&[
            ("P2P_ICE_CONFIGS_PER_HOUR", "1e2"),
            ("P2P_TURN_MAX_TTL_SECONDS", "0x12c"),
            ("P2P_FORCE_SDR", " True "),
        ]);
        assert_eq!(config.configs_per_hour, 100);
        assert_eq!(config.credential_ttl_seconds, 300);
        assert!(config.force_relay);
        assert!(!read(&[("P2P_FORCE_SDR", "1")]).force_relay);
        assert_eq!(read(&[("P2P_RELAY_MONTHLY_QUOTA_GB", "0")]).monthly_quota_bytes, 0);
    }

    #[test]
    fn the_summary_names_no_secret() {
        let config = read(&[
            ("P2P_TURN_PROVIDER", "turn-rest"),
            ("P2P_TURN_REST_URLS", "turn:turn.example.org:3478"),
            ("P2P_TURN_REST_SECRET", "s3cret-s3cret-s3cret"),
            ("P2P_TRANSPORT_KEY_SECRET", &"k".repeat(32)),
        ]);
        let summary = config.summary_json();
        assert_eq!(
            summary,
            concat!(
                r#"{"iceStunServers":1,"iceTurnProvider":"turn-rest","iceRelayConfigured":true,"#,
                r#""iceRelayQuotaGb":10,"iceRelayQuotaMeasured":false,"iceTransportKeyConfigured":true}"#
            )
        );
        assert!(!format!("{config:?}").contains("s3cret"));
        assert_eq!(TurnRelayPorts::default().listen, 3478);
    }
}
