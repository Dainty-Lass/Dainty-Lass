// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The relay settings. A bad value falls back to its default instead of failing, so a typo never stops the server.

use std::ops::RangeInclusive;

use rifts_seam::{Secret, Settings};

use crate::wire::{JsonObject, is_safe_integer, js_number, js_trim};

/// The settings `setup` keeps in `secrets.env`.
pub const SECRET_SETTINGS: [&str; 4] = [
    "P2P_CLOUDFLARE_TURN_API_TOKEN",
    "P2P_TURN_REST_SECRET",
    "P2P_CLOUDFLARE_ANALYTICS_API_TOKEN",
    "P2P_TRANSPORT_KEY_SECRET",
];

const DEFAULT_STUN: &str = "stun:stun.cloudflare.com:3478";
const BYTES_PER_GB: u64 = 1_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TurnProvider {
    #[default]
    Cloudflare,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudflareTurn {
    pub key_id: String,
    pub api_token: Secret,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnRest {
    pub urls: Vec<String>,
    pub secret: Secret,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudflareAnalytics {
    pub account_id: String,
    pub api_token: Secret,
}

/// Ports of a self-hosted TURN server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnRelayPorts {
    pub listen: u16,
    pub tls: Option<u16>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayConfig {
    pub stun_urls: Vec<String>,
    pub provider: TurnProvider,
    pub cloudflare: Option<CloudflareTurn>,
    pub turn_rest: Option<TurnRest>,
    pub credential_ttl_seconds: u32,
    pub hub_credential_ttl_seconds: u32,
    pub monthly_quota_bytes: u64,
    pub configs_per_hour: u32,
    pub analytics: Option<CloudflareAnalytics>,
    pub transport_key_secret: Option<Secret>,
    pub force_relay: bool,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            stun_urls: vec![DEFAULT_STUN.to_owned()],
            provider: TurnProvider::Cloudflare,
            cloudflare: None,
            turn_rest: None,
            credential_ttl_seconds: 10_800,
            hub_credential_ttl_seconds: 86_400,
            monthly_quota_bytes: 10 * BYTES_PER_GB,
            configs_per_hour: 30,
            analytics: None,
            transport_key_secret: None,
            force_relay: false,
        }
    }
}

impl RelayConfig {
    pub fn from_settings(settings: &dyn Settings) -> Self {
        let trimmed = |name: &str| js_trim(settings.get(name).unwrap_or("")).to_owned();

        let mut stun_urls = url_list(settings.get("P2P_ICE_STUN_URLS"), is_stun_url);
        if stun_urls.is_empty() {
            stun_urls.push(DEFAULT_STUN.to_owned());
        }
        let provider = match trimmed("P2P_TURN_PROVIDER").to_lowercase().as_str() {
            "turn-rest" => TurnProvider::TurnRest,
            "none" => TurnProvider::None,
            _ => TurnProvider::Cloudflare,
        };

        let cloudflare = (provider == TurnProvider::Cloudflare)
            .then(|| {
                (
                    trimmed("P2P_CLOUDFLARE_TURN_KEY_ID"),
                    trimmed("P2P_CLOUDFLARE_TURN_API_TOKEN"),
                )
            })
            .filter(|(key_id, token)| is_account_like(key_id) && is_api_token(token))
            .map(|(key_id, token)| CloudflareTurn {
                key_id,
                api_token: Secret::new(token),
            });

        let turn_rest = if provider == TurnProvider::TurnRest {
            let urls = url_list(settings.get("P2P_TURN_REST_URLS"), is_turn_url);
            let secret = trimmed("P2P_TURN_REST_SECRET");
            (!urls.is_empty() && utf16_len(&secret) >= 16).then(|| TurnRest {
                urls,
                secret: Secret::new(secret),
            })
        } else {
            None
        };

        let account_id = trimmed("P2P_CLOUDFLARE_ANALYTICS_ACCOUNT_ID");
        let analytics_token = trimmed("P2P_CLOUDFLARE_ANALYTICS_API_TOKEN");
        let analytics = (is_account_like(&account_id) && is_api_token(&analytics_token)).then(|| CloudflareAnalytics {
            account_id,
            api_token: Secret::new(analytics_token),
        });

        let bounded = |name: &str, default: u32, range: RangeInclusive<u32>| {
            bounded_number(
                settings.get(name),
                u64::from(default),
                u64::from(*range.start())..=u64::from(*range.end()),
            )
        };
        let as_u32 = |value: u64, default: u32| u32::try_from(value).unwrap_or(default);

        let transport_key = trimmed("P2P_TRANSPORT_KEY_SECRET");
        Self {
            stun_urls,
            provider,
            cloudflare,
            turn_rest,
            credential_ttl_seconds: as_u32(bounded("P2P_TURN_MAX_TTL_SECONDS", 10_800, 300..=172_800), 10_800),
            hub_credential_ttl_seconds: as_u32(
                bounded("P2P_REALM_HUB_TURN_TTL_SECONDS", 86_400, 300..=172_800),
                86_400,
            ),
            monthly_quota_bytes: bounded("P2P_RELAY_MONTHLY_QUOTA_GB", 10, 0..=100_000) * BYTES_PER_GB,
            configs_per_hour: as_u32(bounded("P2P_ICE_CONFIGS_PER_HOUR", 30, 1..=1000), 30),
            analytics,
            transport_key_secret: (utf16_len(&transport_key) >= 32).then(|| Secret::new(transport_key)),
            force_relay: trimmed("P2P_FORCE_SDR").eq_ignore_ascii_case("true"),
        }
    }

    pub fn relay_configured(&self) -> bool {
        match self.provider {
            TurnProvider::Cloudflare => self.cloudflare.is_some(),
            TurnProvider::TurnRest => self.turn_rest.is_some(),
            TurnProvider::None => false,
        }
    }

    /// What the startup summary shows; never a secret.
    pub fn summary_json(&self) -> String {
        #[allow(clippy::cast_precision_loss)]
        let quota_gb = self.monthly_quota_bytes as f64 / 1e9;
        let measured =
            self.monthly_quota_bytes > 0 && self.provider == TurnProvider::Cloudflare && self.analytics.is_some();
        JsonObject::new()
            .number("iceStunServers", self.stun_urls.len())
            .string("iceTurnProvider", self.provider.as_str())
            .boolean("iceRelayConfigured", self.relay_configured())
            .number("iceRelayQuotaGb", quota_gb)
            .boolean("iceRelayQuotaMeasured", measured)
            .boolean("iceTransportKeyConfigured", self.transport_key_secret.is_some())
            .finish()
    }
}

fn url_list(value: Option<&str>, keep: fn(&str) -> bool) -> Vec<String> {
    value
        .unwrap_or("")
        .split(',')
        .map(js_trim)
        .filter(|url| keep(url))
        .map(str::to_owned)
        .collect()
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// Cloudflare key ids and account ids: 8 to 64 ASCII letters and digits.
fn is_account_like(text: &str) -> bool {
    (8..=64).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn is_api_token(text: &str) -> bool {
    (8..=512).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._~+/=-".contains(&b))
}

fn bounded_number(value: Option<&str>, default: u64, range: RangeInclusive<u64>) -> u64 {
    let text = js_trim(value.unwrap_or(""));
    if text.is_empty() {
        return default;
    }
    let number = js_number(text);
    if !is_safe_integer(number) || number < 0.0 {
        return default;
    }
    // A non-negative safe integer, so the cast is exact.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let number = number as u64;
    if range.contains(&number) { number } else { default }
}

/// `scheme:host[:port]` with a host of letters, digits, dots and dashes; returns what follows the host and port.
fn host_and_port<'a>(text: &'a str, schemes: [&str; 2]) -> Option<&'a str> {
    let rest = schemes.iter().find_map(|scheme| text.strip_prefix(scheme))?;
    let host = rest
        .bytes()
        .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
        .count();
    if host == 0 {
        return None;
    }
    let rest = &rest[host..];
    let Some(port) = rest.strip_prefix(':') else {
        return Some(rest);
    };
    let digits = port.bytes().take_while(u8::is_ascii_digit).count();
    (1..=5).contains(&digits).then(|| &port[digits..])
}

pub fn is_stun_url(text: &str) -> bool {
    host_and_port(text, ["stun:", "stuns:"]) == Some("")
}

pub fn is_turn_url(text: &str) -> bool {
    matches!(
        host_and_port(text, ["turn:", "turns:"]),
        Some("" | "?transport=udp" | "?transport=tcp")
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn settings(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    #[test]
    fn empty_settings_are_the_defaults() {
        let config = RelayConfig::from_settings(&settings(&[]));
        assert_eq!(config, RelayConfig::default());
        assert_eq!(config.stun_urls, ["stun:stun.cloudflare.com:3478"]);
        assert_eq!(config.monthly_quota_bytes, 10_000_000_000);
        assert!(!config.relay_configured());
        assert_eq!(
            TurnRelayPorts::default(),
            TurnRelayPorts {
                listen: 3478,
                tls: None,
                relay: 49152..=65535
            }
        );
    }

    #[test]
    fn providers_need_their_keys() {
        let short = RelayConfig::from_settings(&settings(&[
            ("P2P_TURN_PROVIDER", " TURN-REST "),
            ("P2P_TURN_REST_URLS", "turn:relay.example:3478"),
            ("P2P_TURN_REST_SECRET", "too-short"),
            ("P2P_CLOUDFLARE_TURN_KEY_ID", "abcdef0123456789"),
            ("P2P_CLOUDFLARE_TURN_API_TOKEN", "token-0123456789"),
        ]));
        assert_eq!(short.provider, TurnProvider::TurnRest);
        assert_eq!(short.turn_rest, None);
        assert_eq!(short.cloudflare, None);

        let rest = RelayConfig::from_settings(&settings(&[
            ("P2P_TURN_PROVIDER", "turn-rest"),
            (
                "P2P_TURN_REST_URLS",
                " turn:a.example:3478 , bad, turns:b.example:5349?transport=tcp",
            ),
            ("P2P_TURN_REST_SECRET", " 0123456789abcdef "),
        ]));
        let turn = rest.turn_rest.clone().unwrap();
        assert_eq!(turn.urls, ["turn:a.example:3478", "turns:b.example:5349?transport=tcp"]);
        assert_eq!(turn.secret.expose(), "0123456789abcdef");
        assert!(rest.relay_configured());

        let none = RelayConfig::from_settings(&settings(&[("P2P_TURN_PROVIDER", "none")]));
        assert_eq!(none.provider, TurnProvider::None);
        assert!(!none.relay_configured());
        let coturn = RelayConfig::from_settings(&settings(&[("P2P_TURN_PROVIDER", "coturn")]));
        assert_eq!(coturn.provider, TurnProvider::Cloudflare);

        let cloudflare = RelayConfig::from_settings(&settings(&[
            ("P2P_CLOUDFLARE_TURN_KEY_ID", " abcdef0123456789 "),
            ("P2P_CLOUDFLARE_TURN_API_TOKEN", "tok.en_~+/=-0123"),
            ("P2P_CLOUDFLARE_ANALYTICS_ACCOUNT_ID", "acct0123"),
            ("P2P_CLOUDFLARE_ANALYTICS_API_TOKEN", "analytics-token"),
        ]));
        assert_eq!(cloudflare.cloudflare.as_ref().unwrap().key_id, "abcdef0123456789");
        assert_eq!(cloudflare.analytics.as_ref().unwrap().account_id, "acct0123");
        assert!(cloudflare.relay_configured());
        let bad_key = RelayConfig::from_settings(&settings(&[
            ("P2P_CLOUDFLARE_TURN_KEY_ID", "short"),
            ("P2P_CLOUDFLARE_TURN_API_TOKEN", "tok.en_~+/=-0123"),
        ]));
        assert_eq!(bad_key.cloudflare, None);
    }

    #[test]
    fn url_patterns() {
        for good in [
            "turns:relay.example:5349?transport=tcp",
            "turn:1.2.3.4",
            "turn:h:3478?transport=udp",
        ] {
            assert!(is_turn_url(good), "{good}");
        }
        for bad in [
            "turn:h:123456",
            "turn:h?transport=sctp",
            "turn::3478",
            "stun:h",
            "turn:h:",
            "turn:h x",
        ] {
            assert!(!is_turn_url(bad), "{bad}");
        }
        assert!(is_stun_url("stuns:host:5349"));
        assert!(is_stun_url("stun:stun.cloudflare.com:3478"));
        assert!(!is_stun_url("stun:host?transport=udp"));
        assert!(!is_stun_url("turn:host"));
        assert!(!is_stun_url("stun:"));
    }

    #[test]
    fn numbers_fall_back() {
        let config = RelayConfig::from_settings(&settings(&[
            ("P2P_TURN_MAX_TTL_SECONDS", "1e3"),
            ("P2P_REALM_HUB_TURN_TTL_SECONDS", "0x12c"),
            ("P2P_ICE_CONFIGS_PER_HOUR", "1e2"),
            ("P2P_RELAY_MONTHLY_QUOTA_GB", "0"),
            ("P2P_FORCE_SDR", " True "),
        ]));
        assert_eq!(config.credential_ttl_seconds, 1000);
        assert_eq!(config.hub_credential_ttl_seconds, 300);
        assert_eq!(config.configs_per_hour, 100);
        assert_eq!(config.monthly_quota_bytes, 0);
        assert!(config.force_relay);

        let fallback = RelayConfig::from_settings(&settings(&[
            ("P2P_TURN_MAX_TTL_SECONDS", "-1"),
            ("P2P_REALM_HUB_TURN_TTL_SECONDS", "999999"),
            ("P2P_ICE_CONFIGS_PER_HOUR", "2.5"),
            ("P2P_RELAY_MONTHLY_QUOTA_GB", "lots"),
            ("P2P_FORCE_SDR", "1"),
            ("P2P_ICE_STUN_URLS", "nope, also-nope"),
        ]));
        assert_eq!(fallback.credential_ttl_seconds, 10_800);
        assert_eq!(fallback.hub_credential_ttl_seconds, 86_400);
        assert_eq!(fallback.configs_per_hour, 30);
        assert_eq!(fallback.monthly_quota_bytes, 10_000_000_000);
        assert!(!fallback.force_relay);
        assert_eq!(fallback.stun_urls, ["stun:stun.cloudflare.com:3478"]);
    }

    #[test]
    fn transport_key_needs_32_units() {
        let short = RelayConfig::from_settings(&settings(&[("P2P_TRANSPORT_KEY_SECRET", &"k".repeat(31))]));
        assert_eq!(short.transport_key_secret, None);
        let long = RelayConfig::from_settings(&settings(&[("P2P_TRANSPORT_KEY_SECRET", &"k".repeat(32))]));
        assert!(long.transport_key_secret.is_some());
    }

    #[test]
    fn summary_has_no_secret() {
        let config = RelayConfig::from_settings(&settings(&[
            ("P2P_TURN_PROVIDER", "turn-rest"),
            ("P2P_TURN_REST_URLS", "turn:relay.example:3478"),
            ("P2P_TURN_REST_SECRET", "turn-rest-secret-value"),
            ("P2P_TRANSPORT_KEY_SECRET", "transport-key-secret-0123456789abcdef"),
            ("P2P_ICE_STUN_URLS", "stun:a.example,stun:b.example:19302"),
        ]));
        assert_eq!(
            config.summary_json(),
            r#"{"iceStunServers":2,"iceTurnProvider":"turn-rest","iceRelayConfigured":true,"iceRelayQuotaGb":10,"iceRelayQuotaMeasured":false,"iceTransportKeyConfigured":true}"#
        );
        let debug = format!("{config:?}");
        assert!(!debug.contains("turn-rest-secret-value"));
        assert!(!debug.contains("transport-key-secret"));
    }
}
