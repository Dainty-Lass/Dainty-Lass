// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The relay settings. A bad value falls back to its default instead of failing, so a typo never stops the server.

use std::ops::RangeInclusive;

use rifts_seam::{Secret, Settings};

use crate::wire::{self, JsonObject};

/// The settings `setup` keeps in `secrets.env`.
pub const SECRET_SETTINGS: [&str; 4] = [
    "P2P_CLOUDFLARE_TURN_API_TOKEN",
    "P2P_TURN_REST_SECRET",
    "P2P_CLOUDFLARE_ANALYTICS_API_TOKEN",
    "P2P_TRANSPORT_KEY_SECRET",
];

const DEFAULT_STUN: &str = "stun:stun.cloudflare.com:3478";
const BYTES_PER_GB: u64 = 1_000_000_000;

/// A whole-number setting: its name, the value it falls back to, and the values it may take.
struct Bounded {
    name: &'static str,
    fallback: u32,
    allowed: RangeInclusive<u32>,
}

const TURN_TTL: Bounded = Bounded {
    name: "P2P_TURN_MAX_TTL_SECONDS",
    fallback: 10_800,
    allowed: 300..=172_800,
};
const HUB_TURN_TTL: Bounded = Bounded {
    name: "P2P_REALM_HUB_TURN_TTL_SECONDS",
    fallback: 86_400,
    allowed: 300..=172_800,
};
const QUOTA_GB: Bounded = Bounded {
    name: "P2P_RELAY_MONTHLY_QUOTA_GB",
    fallback: 10,
    allowed: 0..=100_000,
};
const CONFIGS_PER_HOUR: Bounded = Bounded {
    name: "P2P_ICE_CONFIGS_PER_HOUR",
    fallback: 30,
    allowed: 1..=1000,
};

/// Where TURN credentials come from (`P2P_TURN_PROVIDER`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TurnProvider {
    /// Cloudflare's TURN service.
    #[default]
    Cloudflare,
    /// A TURN server sharing a secret with this server (TURN REST).
    TurnRest,
    /// No relay: transports get STUN only.
    None,
}

impl TurnProvider {
    /// The setting's word for the provider.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cloudflare => "cloudflare",
            Self::TurnRest => "turn-rest",
            Self::None => "none",
        }
    }

    /// Any case; anything unknown is the default, Cloudflare.
    fn named(text: &str) -> Self {
        let lowered = text.to_lowercase();
        [Self::TurnRest, Self::None]
            .into_iter()
            .find(|provider| provider.as_str() == lowered)
            .unwrap_or_default()
    }
}

/// A Cloudflare TURN key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CloudflareTurn {
    /// The key's id (`P2P_CLOUDFLARE_TURN_KEY_ID`).
    pub key_id: String,
    /// The API token (`P2P_CLOUDFLARE_TURN_API_TOKEN`).
    pub api_token: Secret,
}

/// A TURN REST server.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnRest {
    /// Its TURN URLs (`P2P_TURN_REST_URLS`).
    pub urls: Vec<String>,
    /// The shared secret (`P2P_TURN_REST_SECRET`), at least 16 characters.
    pub secret: Secret,
}

/// Cloudflare analytics access, for reading relay usage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CloudflareAnalytics {
    /// The Cloudflare account id (`P2P_CLOUDFLARE_ANALYTICS_ACCOUNT_ID`).
    pub account_id: String,
    /// The API token (`P2P_CLOUDFLARE_ANALYTICS_API_TOKEN`).
    pub api_token: Secret,
}

/// Ports of a self-hosted TURN server.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnRelayPorts {
    /// The listening port, UDP and TCP.
    pub listen: u16,
    /// The TLS port, if any.
    pub tls: Option<u16>,
    /// The ports relayed traffic uses.
    pub relay: RangeInclusive<u16>,
}

impl Default for TurnRelayPorts {
    fn default() -> Self {
        // TURN's registered port, and the dynamic range for the relayed ports.
        Self {
            relay: 49152..=u16::MAX,
            listen: 3478,
            tls: None,
        }
    }
}

/// The relay's settings, read from the server's settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelayConfig {
    /// STUN URLs every transport gets (`P2P_ICE_STUN_URLS`); Cloudflare's when none are valid.
    pub stun_urls: Vec<String>,
    /// Where TURN credentials come from.
    pub provider: TurnProvider,
    /// The Cloudflare key, when the provider is Cloudflare and both settings are present.
    pub cloudflare: Option<CloudflareTurn>,
    /// The TURN REST server, when that is the provider and its settings are valid.
    pub turn_rest: Option<TurnRest>,
    /// The longest a hunt's TURN credential lasts, in seconds (`P2P_TURN_MAX_TTL_SECONDS`).
    pub credential_ttl_seconds: u32,
    /// The longest a realm hub's TURN credential lasts, in seconds (`P2P_REALM_HUB_TURN_TTL_SECONDS`).
    pub hub_credential_ttl_seconds: u32,
    /// Relay bytes an account may use a month; 0 for no limit (`P2P_RELAY_MONTHLY_QUOTA_GB`).
    pub monthly_quota_bytes: u64,
    /// Transport configs an account may ask for an hour (`P2P_ICE_CONFIGS_PER_HOUR`).
    pub configs_per_hour: u32,
    /// Cloudflare analytics access, for measuring the quota.
    pub analytics: Option<CloudflareAnalytics>,
    /// The secret pair keys derive from (`P2P_TRANSPORT_KEY_SECRET`), at least 32 characters.
    pub transport_key_secret: Option<Secret>,
    /// Whether transports must use the relay and never connect directly (`P2P_FORCE_SDR`).
    pub force_relay: bool,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            credential_ttl_seconds: TURN_TTL.fallback,
            hub_credential_ttl_seconds: HUB_TURN_TTL.fallback,
            monthly_quota_bytes: u64::from(QUOTA_GB.fallback) * BYTES_PER_GB,
            configs_per_hour: CONFIGS_PER_HOUR.fallback,
            stun_urls: vec![DEFAULT_STUN.to_owned()],
            provider: TurnProvider::default(),
            force_relay: false,
            cloudflare: None,
            turn_rest: None,
            analytics: None,
            transport_key_secret: None,
        }
    }
}

/// Settings as the relay reads them: every value trimmed as JavaScript trims, a missing one empty.
struct Reader<'a>(&'a dyn Settings);

impl Reader<'_> {
    fn text(&self, name: &str) -> String {
        wire::js_trim(self.0.get(name).unwrap_or_default()).to_owned()
    }

    /// The comma-separated URLs that pass `keep`.
    fn urls(&self, name: &str, keep: fn(&str) -> bool) -> Vec<String> {
        let listed = self.0.get(name).unwrap_or_default();
        listed
            .split(',')
            .map(wire::js_trim)
            .filter(|url| keep(url))
            .map(String::from)
            .collect()
    }

    /// `Number()` of the setting when it is a whole number in range; otherwise the fallback.
    fn bounded(&self, setting: &Bounded) -> u32 {
        let text = self.text(setting.name);
        let number = wire::js_number(&text);
        let whole = !text.is_empty() && number >= 0.0 && wire::is_safe_integer(number);
        // A non-negative safe integer, so the cast is exact.
        let value = whole
            .then_some(number as u64)
            .and_then(|value| u32::try_from(value).ok());
        value
            .filter(|value| setting.allowed.contains(value))
            .unwrap_or(setting.fallback)
    }

    /// A key id or account id and its token, when both are well formed.
    fn id_and_token(&self, id: &str, token: &str) -> Option<(String, Secret)> {
        let (id, token) = (self.text(id), self.text(token));
        (is_account_like(&id) && is_api_token(&token)).then(|| (id, Secret::new(token)))
    }

    /// A secret of at least `units` UTF-16 units.
    fn secret(&self, name: &str, units: usize) -> Option<Secret> {
        let text = self.text(name);
        (text.encode_utf16().count() >= units).then(|| Secret::new(text))
    }
}

impl RelayConfig {
    /// Reads the settings; a missing or out-of-range value takes its default, and an invalid secret reads as unset.
    pub fn from_settings(settings: &dyn Settings) -> Self {
        let read = Reader(settings);
        let provider = TurnProvider::named(&read.text("P2P_TURN_PROVIDER"));
        let mut stun_urls = read.urls("P2P_ICE_STUN_URLS", is_stun_url);
        if stun_urls.is_empty() {
            stun_urls = vec![DEFAULT_STUN.to_owned()];
        }
        let cloudflare = match provider {
            TurnProvider::Cloudflare => {
                read.id_and_token("P2P_CLOUDFLARE_TURN_KEY_ID", "P2P_CLOUDFLARE_TURN_API_TOKEN")
            }
            _ => None,
        }
        .map(|(key_id, api_token)| CloudflareTurn { key_id, api_token });
        let turn_rest = match provider {
            TurnProvider::TurnRest => {
                let urls = read.urls("P2P_TURN_REST_URLS", is_turn_url);
                read.secret("P2P_TURN_REST_SECRET", 16)
                    .filter(|_| !urls.is_empty())
                    .map(|secret| TurnRest { urls, secret })
            }
            _ => None,
        };
        let analytics = read
            .id_and_token(
                "P2P_CLOUDFLARE_ANALYTICS_ACCOUNT_ID",
                "P2P_CLOUDFLARE_ANALYTICS_API_TOKEN",
            )
            .map(|(account_id, api_token)| CloudflareAnalytics { account_id, api_token });
        Self {
            credential_ttl_seconds: read.bounded(&TURN_TTL),
            hub_credential_ttl_seconds: read.bounded(&HUB_TURN_TTL),
            monthly_quota_bytes: u64::from(read.bounded(&QUOTA_GB)) * BYTES_PER_GB,
            configs_per_hour: read.bounded(&CONFIGS_PER_HOUR),
            transport_key_secret: read.secret("P2P_TRANSPORT_KEY_SECRET", 32),
            force_relay: read.text("P2P_FORCE_SDR").eq_ignore_ascii_case("true"),
            stun_urls,
            provider,
            cloudflare,
            turn_rest,
            analytics,
        }
    }

    /// Whether the provider has the settings it needs to issue credentials.
    pub fn relay_configured(&self) -> bool {
        match self.provider {
            TurnProvider::None => false,
            TurnProvider::TurnRest => self.turn_rest.is_some(),
            TurnProvider::Cloudflare => self.cloudflare.is_some(),
        }
    }

    /// What the startup summary shows; never a secret.
    pub fn summary_json(&self) -> String {
        let quota = self.monthly_quota_bytes;
        let measured = quota > 0 && self.analytics.is_some() && self.provider == TurnProvider::Cloudflare;
        JsonObject::new()
            .number("iceStunServers", self.stun_urls.len())
            .string("iceTurnProvider", self.provider.as_str())
            .boolean("iceRelayConfigured", self.relay_configured())
            .number("iceRelayQuotaGb", quota as f64 / 1e9)
            .boolean("iceRelayQuotaMeasured", measured)
            .boolean("iceTransportKeyConfigured", self.transport_key_secret.is_some())
            .finish()
    }
}

/// Cloudflare key ids and account ids: 8 to 64 ASCII letters and digits.
fn is_account_like(text: &str) -> bool {
    matches!(text.len(), 8..=64) && text.chars().all(|c| c.is_ascii_alphanumeric())
}

fn is_api_token(text: &str) -> bool {
    matches!(text.len(), 8..=512) && text.chars().all(|c| c.is_ascii_alphanumeric() || "._~+/=-".contains(c))
}

/// `scheme:host[:port]` with a host of letters, digits, dots and dashes; returns what follows the host and port.
fn host_and_port<'a>(text: &'a str, schemes: [&str; 2]) -> Option<&'a str> {
    let after_scheme = schemes.into_iter().find_map(|scheme| text.strip_prefix(scheme))?;
    let host_end = after_scheme
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-'))
        .unwrap_or(after_scheme.len());
    if host_end == 0 {
        return None;
    }
    let rest = &after_scheme[host_end..];
    match rest.strip_prefix(':') {
        None => Some(rest),
        Some(port) => {
            let digits = port.find(|c: char| !c.is_ascii_digit()).unwrap_or(port.len());
            matches!(digits, 1..=5).then(|| &port[digits..])
        }
    }
}

/// Whether `text` is a `stun:` or `stuns:` URL with a host and nothing after it.
pub fn is_stun_url(text: &str) -> bool {
    matches!(host_and_port(text, ["stun:", "stuns:"]), Some(""))
}

/// Whether `text` is a `turn:` or `turns:` URL with a host and at most a UDP or TCP `transport`.
pub fn is_turn_url(text: &str) -> bool {
    let tail = host_and_port(text, ["turn:", "turns:"]);
    tail.is_some_and(|tail| ["", "?transport=udp", "?transport=tcp"].contains(&tail))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn from(pairs: &[(&str, &str)]) -> RelayConfig {
        let settings: HashMap<String, String> = pairs
            .iter()
            .map(|&(name, value)| (name.to_owned(), value.to_owned()))
            .collect();
        RelayConfig::from_settings(&settings)
    }

    #[test]
    fn empty_settings_are_the_defaults() {
        let config = from(&[]);
        assert_eq!(config, RelayConfig::default());
        assert_eq!(config.stun_urls, ["stun:stun.cloudflare.com:3478"]);
        assert_eq!(config.monthly_quota_bytes, 10_000_000_000);
        assert!(!config.relay_configured());
        let ports = TurnRelayPorts::default();
        assert_eq!((ports.listen, ports.tls, ports.relay), (3478, None, 49152..=65535));
    }

    #[test]
    fn providers_need_their_keys() {
        let short = from(&[
            ("P2P_TURN_PROVIDER", " TURN-REST "),
            ("P2P_TURN_REST_URLS", "turn:relay.example:3478"),
            ("P2P_TURN_REST_SECRET", "too-short"),
            ("P2P_CLOUDFLARE_TURN_KEY_ID", "abcdef0123456789"),
            ("P2P_CLOUDFLARE_TURN_API_TOKEN", "token-0123456789"),
        ]);
        assert_eq!(
            (short.provider, &short.turn_rest, &short.cloudflare),
            (TurnProvider::TurnRest, &None, &None)
        );

        let rest = from(&[
            ("P2P_TURN_PROVIDER", "turn-rest"),
            (
                "P2P_TURN_REST_URLS",
                " turn:a.example:3478 , bad, turns:b.example:5349?transport=tcp",
            ),
            ("P2P_TURN_REST_SECRET", " 0123456789abcdef "),
        ]);
        let turn = rest.turn_rest.as_ref().unwrap();
        assert_eq!(turn.urls, ["turn:a.example:3478", "turns:b.example:5349?transport=tcp"]);
        assert_eq!(turn.secret.expose(), "0123456789abcdef");
        assert!(rest.relay_configured());

        let none = from(&[("P2P_TURN_PROVIDER", "none")]);
        assert_eq!(none.provider, TurnProvider::None);
        assert!(!none.relay_configured());
        assert_eq!(
            from(&[("P2P_TURN_PROVIDER", "coturn")]).provider,
            TurnProvider::Cloudflare
        );

        let cloudflare = from(&[
            ("P2P_CLOUDFLARE_TURN_KEY_ID", " abcdef0123456789 "),
            ("P2P_CLOUDFLARE_TURN_API_TOKEN", "tok.en_~+/=-0123"),
            ("P2P_CLOUDFLARE_ANALYTICS_ACCOUNT_ID", "acct0123"),
            ("P2P_CLOUDFLARE_ANALYTICS_API_TOKEN", "analytics-token"),
        ]);
        assert_eq!(cloudflare.cloudflare.as_ref().unwrap().key_id, "abcdef0123456789");
        assert_eq!(cloudflare.analytics.as_ref().unwrap().account_id, "acct0123");
        assert!(cloudflare.relay_configured());
        let bad_key = from(&[
            ("P2P_CLOUDFLARE_TURN_KEY_ID", "short"),
            ("P2P_CLOUDFLARE_TURN_API_TOKEN", "tok.en_~+/=-0123"),
        ]);
        assert_eq!(bad_key.cloudflare, None);
    }

    #[test]
    fn url_patterns() {
        let turn = [
            ("turns:relay.example:5349?transport=tcp", true),
            ("turn:1.2.3.4", true),
            ("turn:h:3478?transport=udp", true),
            ("turn:h:123456", false),
            ("turn:h?transport=sctp", false),
            ("turn::3478", false),
            ("stun:h", false),
            ("turn:h:", false),
            ("turn:h x", false),
        ];
        for (url, expected) in turn {
            assert_eq!(is_turn_url(url), expected, "{url}");
        }
        let stun = [
            ("stuns:host:5349", true),
            ("stun:stun.cloudflare.com:3478", true),
            ("stun:host?transport=udp", false),
            ("turn:host", false),
            ("stun:", false),
        ];
        for (url, expected) in stun {
            assert_eq!(is_stun_url(url), expected, "{url}");
        }
    }

    #[test]
    fn numbers_fall_back() {
        let parsed = from(&[
            ("P2P_TURN_MAX_TTL_SECONDS", "1e3"),
            ("P2P_REALM_HUB_TURN_TTL_SECONDS", "0x12c"),
            ("P2P_ICE_CONFIGS_PER_HOUR", "1e2"),
            ("P2P_RELAY_MONTHLY_QUOTA_GB", "0"),
            ("P2P_FORCE_SDR", " True "),
        ]);
        assert_eq!(
            (
                parsed.credential_ttl_seconds,
                parsed.hub_credential_ttl_seconds,
                parsed.configs_per_hour
            ),
            (1000, 300, 100)
        );
        assert_eq!(parsed.monthly_quota_bytes, 0);
        assert!(parsed.force_relay);

        let fallback = from(&[
            ("P2P_TURN_MAX_TTL_SECONDS", "-1"),
            ("P2P_REALM_HUB_TURN_TTL_SECONDS", "999999"),
            ("P2P_ICE_CONFIGS_PER_HOUR", "2.5"),
            ("P2P_RELAY_MONTHLY_QUOTA_GB", "lots"),
            ("P2P_FORCE_SDR", "1"),
            ("P2P_ICE_STUN_URLS", "nope, also-nope"),
        ]);
        let defaults = RelayConfig::default();
        assert_eq!(
            fallback,
            RelayConfig {
                force_relay: false,
                ..defaults
            }
        );
        assert_eq!(fallback.credential_ttl_seconds, 10_800);
        assert_eq!(fallback.hub_credential_ttl_seconds, 86_400);
        assert_eq!(fallback.configs_per_hour, 30);
        assert_eq!(fallback.monthly_quota_bytes, 10_000_000_000);
        assert_eq!(fallback.stun_urls, ["stun:stun.cloudflare.com:3478"]);
    }

    #[test]
    fn transport_key_needs_32_units() {
        let with_key = |length: usize| from(&[("P2P_TRANSPORT_KEY_SECRET", &"k".repeat(length))]).transport_key_secret;
        assert_eq!(with_key(31), None);
        assert!(with_key(32).is_some());
    }

    #[test]
    fn summary_has_no_secret() {
        let config = from(&[
            ("P2P_TURN_PROVIDER", "turn-rest"),
            ("P2P_TURN_REST_URLS", "turn:relay.example:3478"),
            ("P2P_TURN_REST_SECRET", "turn-rest-secret-value"),
            ("P2P_TRANSPORT_KEY_SECRET", "transport-key-secret-0123456789abcdef"),
            ("P2P_ICE_STUN_URLS", "stun:a.example,stun:b.example:19302"),
        ]);
        assert_eq!(
            config.summary_json(),
            r#"{"iceStunServers":2,"iceTurnProvider":"turn-rest","iceRelayConfigured":true,"iceRelayQuotaGb":10,"iceRelayQuotaMeasured":false,"iceTransportKeyConfigured":true}"#
        );
        let debug = format!("{config:?}");
        assert!(!debug.contains("turn-rest-secret-value") && !debug.contains("transport-key-secret"));
    }
}
