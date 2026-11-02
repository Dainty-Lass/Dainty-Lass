// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! TURN credentials: minted here for a turn-rest server, or asked of Cloudflare, with the monthly relay quota read
//! from Cloudflare's analytics. Providers see a hashed tag, never an account id.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use base64::Engine as _;
use hmac::{Hmac, KeyInit as _, Mac as _};
use rifts_seam::{AccountId, BoxFuture, Secret, UnixMillis};
use serde_json::Value;
use sha1::Sha1;
use sha2::{Digest as _, Sha256};

use crate::config::{
    CloudflareAnalytics, CloudflareTurn, RelayConfig, TurnProvider, TurnRest, is_stun_url, is_turn_url,
};
use crate::wire::{JsonObject, hex, iso_time, js_number_value, json_array, json_string};
use crate::{CredentialIssuer, RelayCredential, RelayError};

pub const PROVIDER_TIMEOUT_MS: u64 = 5000;
pub const CLOUDFLARE_RTC_API: &str = "https://rtc.live.cloudflare.com/v1";
pub const CLOUDFLARE_GRAPHQL_API: &str = "https://api.cloudflare.com/client/v4/graphql";

// Umbrella spec D3: the Rustic label. Changing it changes every tag, and so the provider's per-tag usage history.
const RELAY_TAG_LABEL: &str = "rustic-relay-tag-v1";
const USAGE_TTL_MS: i64 = 600_000;
const HOUR_MS: i64 = 3_600_000;
const MAX_TRACKED: usize = 10_000;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // The maps hold plain counters; a panic elsewhere leaves them usable.
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What a provider is told instead of the account id.
pub fn relay_account_tag(account: &AccountId) -> String {
    let digest = Sha256::digest(format!("{RELAY_TAG_LABEL}\n{}", account.0).as_bytes());
    let mut tag = hex(&digest);
    tag.truncate(32);
    tag
}

/// A TURN REST credential (coturn's `use-auth-secret`, turn-rs's `static-auth-secret`).
pub fn mint_turn_rest(turn_rest: &TurnRest, account: &AccountId, ttl_seconds: u32, now: UnixMillis) -> RelayCredential {
    let expiry = now.0.div_euclid(1000) + i64::from(ttl_seconds);
    let username = format!("{expiry}:{}", relay_account_tag(account));
    let credential = match <Hmac<Sha1>>::new_from_slice(turn_rest.secret.expose().as_bytes()) {
        Ok(mut mac) => {
            mac.update(username.as_bytes());
            base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
        }
        // HMAC takes a key of any length.
        Err(_) => String::new(),
    };
    RelayCredential {
        username,
        credential: Secret::new(credential),
        urls: turn_rest.urls.clone(),
        expires_at: UnixMillis(now.0 + i64::from(ttl_seconds) * 1000),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedCredential {
    /// STUN URLs the provider adds to the configured ones.
    pub stun_urls: Vec<String>,
    pub credential: RelayCredential,
}

#[derive(Debug, Clone)]
pub struct TurnRestIssuer(pub TurnRest);

impl CredentialIssuer for TurnRestIssuer {
    fn provider(&self) -> TurnProvider {
        TurnProvider::TurnRest
    }

    fn issue(
        &self,
        account: AccountId,
        ttl_seconds: u32,
        now: UnixMillis,
    ) -> BoxFuture<'_, Result<IssuedCredential, RelayError>> {
        let credential = mint_turn_rest(&self.0, &account, ttl_seconds, now);
        Box::pin(std::future::ready(Ok(IssuedCredential {
            stun_urls: Vec::new(),
            credential,
        })))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRequest {
    pub url: String,
    pub bearer: Secret,
    /// JSON text.
    pub body: String,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderAnswer {
    pub status: u16,
    /// `None` when the body was not JSON.
    pub json: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("relay provider request failed")]
pub struct ProviderUnreachable;

/// HTTPS to the provider, written by the module: a POST with `Authorization: Bearer`, the JSON body and the
/// timeout. It logs neither the token nor the body.
pub trait ProviderHttp: Send + Sync {
    fn post(&self, request: ProviderRequest) -> BoxFuture<'_, Result<ProviderAnswer, ProviderUnreachable>>;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderFailure {
    #[error("Cloudflare TURN credential request answered {0}")]
    Status(u16),
    #[error("Cloudflare TURN response carried no usable relay entry")]
    NoRelayEntry,
    #[error(transparent)]
    Unreachable(#[from] ProviderUnreachable),
}

pub async fn mint_cloudflare(
    cloudflare: &CloudflareTurn,
    account: &AccountId,
    ttl_seconds: u32,
    now: UnixMillis,
    http: &dyn ProviderHttp,
) -> Result<IssuedCredential, ProviderFailure> {
    let request = |body: String| ProviderRequest {
        url: format!(
            "{CLOUDFLARE_RTC_API}/turn/keys/{}/credentials/generate-ice-servers",
            cloudflare.key_id
        ),
        bearer: cloudflare.api_token.clone(),
        body,
        timeout_ms: PROVIDER_TIMEOUT_MS,
    };
    let tagged = JsonObject::new()
        .number("ttl", ttl_seconds)
        .string("customIdentifier", &relay_account_tag(account))
        .finish();
    let mut answer = http.post(request(tagged)).await?;
    // A key without custom identifiers refuses the tagged request; the untagged one still issues.
    if answer.status == 400 {
        answer = http
            .post(request(JsonObject::new().number("ttl", ttl_seconds).finish()))
            .await?;
    }
    if !(200..=299).contains(&answer.status) {
        return Err(ProviderFailure::Status(answer.status));
    }
    let servers = normalize_provider_ice_servers(answer.json.as_ref());
    let (urls, username, credential) = servers.turn.ok_or(ProviderFailure::NoRelayEntry)?;
    Ok(IssuedCredential {
        stun_urls: servers.stun_urls,
        credential: RelayCredential {
            username,
            credential,
            urls,
            expires_at: UnixMillis(now.0 + i64::from(ttl_seconds) * 1000),
        },
    })
}

#[derive(Clone)]
pub struct CloudflareIssuer {
    pub key: CloudflareTurn,
    pub http: Arc<dyn ProviderHttp>,
}

impl CredentialIssuer for CloudflareIssuer {
    fn provider(&self) -> TurnProvider {
        TurnProvider::Cloudflare
    }

    fn issue(
        &self,
        account: AccountId,
        ttl_seconds: u32,
        now: UnixMillis,
    ) -> BoxFuture<'_, Result<IssuedCredential, RelayError>> {
        Box::pin(async move {
            mint_cloudflare(&self.key, &account, ttl_seconds, now, self.http.as_ref())
                .await
                .map_err(RelayError::from)
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderIceServers {
    pub stun_urls: Vec<String>,
    /// TURN URLs, username and credential of the first usable TURN entry.
    pub turn: Option<(Vec<String>, String, Secret)>,
}

fn is_turn_secret_text(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    ((1..=512).contains(&text.len()) && text.bytes().all(|b| (0x21..=0x7e).contains(&b))).then(|| text.to_owned())
}

pub fn normalize_provider_ice_servers(body: Option<&Value>) -> ProviderIceServers {
    let entries: Vec<&Value> = match body.and_then(|body| body.get("iceServers")) {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items.iter().collect(),
        Some(other) => vec![other],
    };
    let mut servers = ProviderIceServers::default();
    for entry in entries {
        let urls: Vec<&str> = match entry.get("urls") {
            Some(Value::String(url)) => vec![url.as_str()],
            Some(Value::Array(items)) => items.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        // Port 53 is DNS; a relay there is refused by most networks and confuses firewalls.
        let urls: Vec<&str> = urls
            .into_iter()
            .filter(|url| !url.ends_with(":53") && !url.contains(":53?"))
            .collect();
        for url in &urls {
            if is_stun_url(url) && !servers.stun_urls.iter().any(|known| known == url) {
                servers.stun_urls.push((*url).to_owned());
            }
        }
        if servers.turn.is_some() {
            continue;
        }
        let turn_urls: Vec<String> = urls
            .iter()
            .filter(|url| is_turn_url(url))
            .map(|url| (*url).to_owned())
            .collect();
        if turn_urls.is_empty() {
            continue;
        }
        if let (Some(username), Some(credential)) = (
            is_turn_secret_text(entry.get("username")),
            is_turn_secret_text(entry.get("credential")),
        ) {
            servers.turn = Some((turn_urls, username, Secret::new(credential)));
        }
    }
    servers
}

/// The month so far, from `now`: (first day, today, month), as Cloudflare's analytics take them.
pub fn month_window(now: UnixMillis) -> (String, String, String) {
    let iso = iso_time(now);
    (format!("{}-01", &iso[..7]), iso[..10].to_owned(), iso[..7].to_owned())
}

/// The relay bytes this account's tag has used this month, or `None` when the figure cannot be read.
pub async fn read_monthly_relay_egress_bytes(
    analytics: &CloudflareAnalytics,
    account: &AccountId,
    now: UnixMillis,
    http: &dyn ProviderHttp,
) -> Option<f64> {
    let (from, to, _) = month_window(now);
    let query = format!(
        "query {{ viewer {{ usage: accounts(filter: {{ accountTag: {} }}) {{\n        \
         callsTurnUsageAdaptiveGroups(filter: {{ date_geq: {}, date_leq: {},\n            \
         customIdentifier: {} }}, limit: 1, orderBy: [customIdentifier_ASC]) {{ sum {{ egressBytes }} }} }} }} }}",
        json_string(&analytics.account_id),
        json_string(&from),
        json_string(&to),
        json_string(&relay_account_tag(account)),
    );
    let request = ProviderRequest {
        url: CLOUDFLARE_GRAPHQL_API.to_owned(),
        bearer: analytics.api_token.clone(),
        body: JsonObject::new().string("query", &query).finish(),
        timeout_ms: PROVIDER_TIMEOUT_MS,
    };
    let answer = http.post(request).await.ok()?;
    if answer.status != 200 {
        return None;
    }
    let json = answer.json?;
    if json
        .get("errors")
        .and_then(Value::as_array)
        .is_some_and(|errors| !errors.is_empty())
    {
        return None;
    }
    let viewer = json.get("data")?.get("viewer")?;
    let usage = match viewer.get("usage") {
        None | Some(Value::Null) => viewer.get("accounts")?,
        Some(usage) => usage,
    };
    let groups = usage.get(0)?.get("callsTurnUsageAdaptiveGroups")?.as_array()?;
    let total: f64 = groups
        .iter()
        .map(|group| {
            let bytes = group
                .get("sum")
                .and_then(|sum| sum.get("egressBytes"))
                .map_or(f64::NAN, js_number_value);
            if bytes.is_nan() { 0.0 } else { bytes }
        })
        .sum();
    (total.is_finite() && total >= 0.0).then_some(total)
}

/// Monthly usage per account, kept ten minutes so a burst of configs asks Cloudflare once.
#[derive(Debug, Default)]
pub struct RelayUsageCache {
    entries: Mutex<HashMap<String, (String, f64, UnixMillis)>>,
}

impl RelayUsageCache {
    pub fn get(&self, account: &AccountId, month: &str, now: UnixMillis) -> Option<f64> {
        let entries = lock(&self.entries);
        let (cached_month, bytes, fetched_at) = entries.get(&account.0)?;
        (cached_month == month && now.0 - fetched_at.0 < USAGE_TTL_MS).then_some(*bytes)
    }

    pub fn set(&self, account: &AccountId, month: &str, bytes: f64, now: UnixMillis) {
        let mut entries = lock(&self.entries);
        if entries.len() > MAX_TRACKED {
            entries.clear();
        }
        entries.insert(account.0.clone(), (month.to_owned(), bytes, now));
    }
}

/// A sliding one-hour limit per key, in process only.
#[derive(Debug, Default)]
pub struct HourlyCounter {
    hits: Mutex<HashMap<String, Vec<i64>>>,
}

impl HourlyCounter {
    pub fn allow(&self, key: &str, limit: u32, now: UnixMillis) -> bool {
        let mut hits = lock(&self.hits);
        let mut recent: Vec<i64> = hits
            .get(key)
            .map(|list| list.iter().copied().filter(|at| now.0 - at < HOUR_MS).collect())
            .unwrap_or_default();
        if recent.len() >= limit as usize {
            hits.insert(key.to_owned(), recent);
            return false;
        }
        recent.push(now.0);
        hits.insert(key.to_owned(), recent);
        if hits.len() > MAX_TRACKED {
            hits.retain(|_, list| list.iter().any(|at| now.0 - at < HOUR_MS));
        }
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayDenied {
    NotConfigured,
    QuotaExceeded,
    ProviderError,
}

impl RelayDenied {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotConfigured => "RELAY_NOT_CONFIGURED",
            Self::QuotaExceeded => "RELAY_QUOTA_EXCEEDED",
            Self::ProviderError => "RELAY_PROVIDER_ERROR",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayStatus {
    Allowed { ttl_seconds: u32, expires_at: UnixMillis },
    Denied(RelayDenied),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IceServer {
    pub urls: Vec<String>,
    pub username: Option<String>,
    pub credential: Option<Secret>,
}

impl IceServer {
    pub fn stun(urls: Vec<String>) -> Self {
        Self {
            urls,
            username: None,
            credential: None,
        }
    }

    pub fn turn(credential: &RelayCredential) -> Self {
        Self {
            urls: credential.urls.clone(),
            username: Some(credential.username.clone()),
            credential: Some(credential.credential.clone()),
        }
    }

    /// The only place a relay credential is written out.
    pub fn to_json(&self) -> String {
        JsonObject::new()
            .strings("urls", &self.urls)
            .optional_string("username", self.username.as_deref())
            .optional_string("credential", self.credential.as_ref().map(Secret::expose))
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayGrant {
    pub ice_servers: Vec<IceServer>,
    pub relay: RelayStatus,
}

impl RelayGrant {
    pub fn ice_servers_json(&self) -> String {
        json_array(self.ice_servers.iter().map(IceServer::to_json))
    }

    pub fn relay_json(&self) -> String {
        match self.relay {
            RelayStatus::Allowed {
                ttl_seconds,
                expires_at,
            } => JsonObject::new()
                .boolean("allowed", true)
                .number("ttlSeconds", ttl_seconds)
                .string("expiresAt", &iso_time(expires_at))
                .finish(),
            RelayStatus::Denied(reason) => JsonObject::new()
                .boolean("allowed", false)
                .string("reason", reason.as_str())
                .finish(),
        }
    }

    pub fn result_label(&self) -> &'static str {
        match self.relay {
            RelayStatus::Allowed { .. } => "ISSUED",
            RelayStatus::Denied(reason) => reason.as_str(),
        }
    }
}

#[derive(Clone)]
pub struct RelayGrants {
    pub config: Arc<RelayConfig>,
    pub http: Arc<dyn ProviderHttp>,
    pub usage: Arc<RelayUsageCache>,
}

impl RelayGrants {
    pub fn issuer(&self) -> Option<Box<dyn CredentialIssuer>> {
        match (self.config.provider, &self.config.cloudflare, &self.config.turn_rest) {
            (TurnProvider::Cloudflare, Some(key), _) => Some(Box::new(CloudflareIssuer {
                key: key.clone(),
                http: Arc::clone(&self.http),
            })),
            (TurnProvider::TurnRest, _, Some(turn_rest)) => Some(Box::new(TurnRestIssuer(turn_rest.clone()))),
            _ => None,
        }
    }

    /// The ICE servers for one transport; STUN alone whenever the relay cannot be granted.
    pub async fn grant(&self, account: &AccountId, ttl_seconds: u32, now: UnixMillis) -> RelayGrant {
        let stun_only = |reason| RelayGrant {
            ice_servers: vec![IceServer::stun(self.config.stun_urls.clone())],
            relay: RelayStatus::Denied(reason),
        };
        let Some(issuer) = self.issuer() else {
            return stun_only(RelayDenied::NotConfigured);
        };
        if let Some(analytics) = &self.config.analytics
            && self.config.provider == TurnProvider::Cloudflare
            && self.config.monthly_quota_bytes > 0
        {
            let (_, _, month) = month_window(now);
            let usage = match self.usage.get(account, &month, now) {
                Some(bytes) => Some(bytes),
                None => {
                    let read = read_monthly_relay_egress_bytes(analytics, account, now, self.http.as_ref()).await;
                    if let Some(bytes) = read {
                        self.usage.set(account, &month, bytes, now);
                    }
                    read
                }
            };
            #[allow(clippy::cast_precision_loss)]
            let quota = self.config.monthly_quota_bytes as f64;
            if usage.is_some_and(|bytes| bytes >= quota) {
                return stun_only(RelayDenied::QuotaExceeded);
            }
        }
        match issuer.issue(account.clone(), ttl_seconds, now).await {
            Ok(issued) => {
                let mut stun_urls = self.config.stun_urls.clone();
                for url in issued.stun_urls {
                    if !stun_urls.contains(&url) {
                        stun_urls.push(url);
                    }
                }
                RelayGrant {
                    ice_servers: vec![IceServer::stun(stun_urls), IceServer::turn(&issued.credential)],
                    relay: RelayStatus::Allowed {
                        ttl_seconds,
                        expires_at: UnixMillis(now.0 + i64::from(ttl_seconds) * 1000),
                    },
                }
            }
            Err(_) => stun_only(RelayDenied::ProviderError),
        }
    }
}

/// A credential that outlives the hunt by ten minutes, never under five minutes, never over the cap.
pub fn relay_ttl_seconds(hunt_expires_at: UnixMillis, now: UnixMillis, max_ttl_seconds: u32) -> u32 {
    let left = hunt_expires_at.0.saturating_sub(now.0);
    let remaining = left.div_euclid(1000) + i64::from(left.rem_euclid(1000) != 0);
    let wanted = remaining.saturating_add(600).max(300).min(i64::from(max_ttl_seconds));
    u32::try_from(wanted).unwrap_or(max_ttl_seconds)
}

#[cfg(test)]
mod tests {
    use rifts_seam::poll_once;
    use serde_json::json;

    use super::*;

    /// Answers each POST from a queue and records the requests.
    #[derive(Default)]
    struct Recording {
        requests: Mutex<Vec<ProviderRequest>>,
        answers: Mutex<Vec<Result<ProviderAnswer, ProviderUnreachable>>>,
    }

    impl Recording {
        fn answering(answers: Vec<Result<ProviderAnswer, ProviderUnreachable>>) -> Arc<Self> {
            Arc::new(Self {
                requests: Mutex::default(),
                answers: Mutex::new(answers),
            })
        }

        fn requests(&self) -> Vec<ProviderRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl ProviderHttp for Recording {
        fn post(&self, request: ProviderRequest) -> BoxFuture<'_, Result<ProviderAnswer, ProviderUnreachable>> {
            self.requests.lock().unwrap().push(request);
            let mut answers = self.answers.lock().unwrap();
            let answer = if answers.is_empty() {
                Err(ProviderUnreachable)
            } else {
                answers.remove(0)
            };
            Box::pin(std::future::ready(answer))
        }
    }

    fn ok(status: u16, json: Value) -> Result<ProviderAnswer, ProviderUnreachable> {
        Ok(ProviderAnswer {
            status,
            json: Some(json),
        })
    }

    fn account() -> AccountId {
        AccountId("account-0001".to_owned())
    }

    fn cloudflare_answer() -> Value {
        json!({"iceServers": [
            {"urls": ["stun:stun.cloudflare.com:3478", "stun:stun.cloudflare.com:53", "stun:extra.example:3478"]},
            {"urls": ["turn:turn.cloudflare.com:3478?transport=udp", "turn:turn.cloudflare.com:53?transport=udp",
                      "turns:turn.cloudflare.com:5349?transport=tcp"],
             "username": "cf-user", "credential": "cf-pass"}
        ]})
    }

    fn turn_rest() -> TurnRest {
        TurnRest {
            urls: vec!["turn:relay.example:3478".to_owned()],
            secret: Secret::new("0123456789abcdef".to_owned()),
        }
    }

    fn config(provider: TurnProvider) -> RelayConfig {
        RelayConfig {
            provider,
            cloudflare: Some(CloudflareTurn {
                key_id: "abcdef0123456789".to_owned(),
                api_token: Secret::new("cf-api-token".to_owned()),
            }),
            turn_rest: Some(turn_rest()),
            ..RelayConfig::default()
        }
    }

    fn grants(config: RelayConfig, http: Arc<Recording>) -> RelayGrants {
        RelayGrants {
            config: Arc::new(config),
            http,
            usage: Arc::default(),
        }
    }

    #[test]
    fn provider_answers_in_both_shapes() {
        let servers = normalize_provider_ice_servers(Some(&cloudflare_answer()));
        assert_eq!(
            servers.stun_urls,
            ["stun:stun.cloudflare.com:3478", "stun:extra.example:3478"]
        );
        let (urls, username, credential) = servers.turn.unwrap();
        assert_eq!(
            urls,
            [
                "turn:turn.cloudflare.com:3478?transport=udp",
                "turns:turn.cloudflare.com:5349?transport=tcp"
            ]
        );
        assert_eq!((username.as_str(), credential.expose()), ("cf-user", "cf-pass"));

        let single = json!({"iceServers": {"urls": "turn:one.example:3478", "username": "u", "credential": "p"}});
        let servers = normalize_provider_ice_servers(Some(&single));
        assert_eq!(servers.turn.unwrap().0, ["turn:one.example:3478"]);

        assert_eq!(normalize_provider_ice_servers(None), ProviderIceServers::default());
        assert_eq!(
            normalize_provider_ice_servers(Some(&Value::Null)),
            ProviderIceServers::default()
        );
        assert_eq!(
            normalize_provider_ice_servers(Some(&json!({"iceServers": null}))),
            ProviderIceServers::default()
        );
    }

    #[test]
    fn bad_provider_credentials_are_skipped() {
        let body = json!({"iceServers": [
            {"urls": "turn:a.example:3478", "username": "has space", "credential": "p"},
            {"urls": "turn:b.example:3478", "username": "u", "credential": ""},
            {"urls": "turn:c.example:3478", "username": "u", "credential": 5},
            {"urls": "turn:d.example:3478", "username": "u", "credential": "p"}
        ]});
        assert_eq!(
            normalize_provider_ice_servers(Some(&body)).turn.unwrap().0,
            ["turn:d.example:3478"]
        );
        let long = json!({"iceServers": [{"urls": "turn:a.example", "username": "u".repeat(513), "credential": "p"}]});
        assert_eq!(normalize_provider_ice_servers(Some(&long)).turn, None);
    }

    #[test]
    fn turn_rest_credentials() {
        let credential = mint_turn_rest(&turn_rest(), &account(), 600, UnixMillis(1_700_000_000_999));
        let tag = relay_account_tag(&account());
        assert_eq!(credential.username, format!("1700000600:{tag}"));
        assert_eq!(credential.credential.expose().len(), 28);
        assert_eq!(credential.urls, ["turn:relay.example:3478"]);
        assert_eq!(credential.expires_at, UnixMillis(1_700_000_600_999));

        let mut mac = <Hmac<Sha1>>::new_from_slice(b"0123456789abcdef").unwrap();
        mac.update(credential.username.as_bytes());
        let expected = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
        assert_eq!(credential.credential.expose(), expected);
    }

    #[test]
    fn hmac_sha1_rfc_2202_case_2() {
        let mut mac = <Hmac<Sha1>>::new_from_slice(b"Jefe").unwrap();
        mac.update(b"what do ya want for nothing?");
        assert_eq!(
            hex(&mac.finalize().into_bytes()),
            "effcdf6ae5eb2fa2d27416d5f184df9c259a7c79"
        );
    }

    #[test]
    fn relay_tag_hides_the_account() {
        let tag = relay_account_tag(&account());
        assert_eq!(tag.len(), 32);
        assert!(tag.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        assert_eq!(tag, relay_account_tag(&account()));
        assert_ne!(tag, relay_account_tag(&AccountId("account-0002".to_owned())));
        assert!(!tag.contains("account"));
        let expected = hex(&Sha256::digest(b"rustic-relay-tag-v1\naccount-0001"));
        assert_eq!(tag, expected[..32]);
    }

    #[test]
    fn stun_only_until_configured() {
        let http = Recording::answering(Vec::new());
        let unconfigured = RelayConfig {
            provider: TurnProvider::None,
            ..config(TurnProvider::None)
        };
        let grant = poll_once(grants(unconfigured, Arc::clone(&http)).grant(&account(), 600, UnixMillis(0))).unwrap();
        assert_eq!(grant.relay, RelayStatus::Denied(RelayDenied::NotConfigured));
        assert_eq!(
            grant.ice_servers_json(),
            r#"[{"urls":["stun:stun.cloudflare.com:3478"]}]"#
        );
        assert_eq!(
            grant.relay_json(),
            r#"{"allowed":false,"reason":"RELAY_NOT_CONFIGURED"}"#
        );
        assert_eq!(grant.result_label(), "RELAY_NOT_CONFIGURED");
        let no_key = RelayConfig {
            cloudflare: None,
            ..config(TurnProvider::Cloudflare)
        };
        assert!(grants(no_key, Arc::clone(&http)).issuer().is_none());
        assert!(http.requests().is_empty());
    }

    #[test]
    fn cloudflare_tagged_then_untagged() {
        let http = Recording::answering(vec![ok(400, json!({})), ok(201, cloudflare_answer())]);
        let grant = poll_once(grants(config(TurnProvider::Cloudflare), Arc::clone(&http)).grant(
            &account(),
            4200,
            UnixMillis(1_791_878_400_000),
        ))
        .unwrap();
        let requests = http.requests();
        assert_eq!(requests.len(), 2);
        let url = "https://rtc.live.cloudflare.com/v1/turn/keys/abcdef0123456789/credentials/generate-ice-servers";
        let tag = relay_account_tag(&account());
        for request in &requests {
            assert_eq!(request.url, url);
            assert_eq!(request.bearer.expose(), "cf-api-token");
            assert_eq!(request.timeout_ms, 5000);
            assert!(!request.body.contains("account-0001"));
        }
        assert_eq!(
            requests[0].body,
            format!(r#"{{"ttl":4200,"customIdentifier":"{tag}"}}"#)
        );
        assert_eq!(requests[1].body, r#"{"ttl":4200}"#);
        assert_eq!(grant.result_label(), "ISSUED");
        assert_eq!(
            grant.ice_servers_json(),
            r#"[{"urls":["stun:stun.cloudflare.com:3478","stun:extra.example:3478"]},{"urls":["turn:turn.cloudflare.com:3478?transport=udp","turns:turn.cloudflare.com:5349?transport=tcp"],"username":"cf-user","credential":"cf-pass"}]"#
        );
        assert_eq!(
            grant.relay_json(),
            r#"{"allowed":true,"ttlSeconds":4200,"expiresAt":"2026-10-13T09:10:00.000Z"}"#
        );
    }

    #[test]
    fn provider_failures() {
        for answers in [
            vec![ok(500, json!({}))],
            vec![Err(ProviderUnreachable)],
            vec![ok(200, json!({"x": 1}))],
        ] {
            let http = Recording::answering(answers);
            let grant = poll_once(grants(config(TurnProvider::Cloudflare), http).grant(&account(), 600, UnixMillis(0)))
                .unwrap();
            assert_eq!(grant.result_label(), "RELAY_PROVIDER_ERROR");
            assert_eq!(grant.ice_servers.len(), 1);
        }
        let key = config(TurnProvider::Cloudflare).cloudflare.unwrap();
        let http = Recording::answering(vec![ok(403, json!({}))]);
        let failure = poll_once(mint_cloudflare(&key, &account(), 600, UnixMillis(0), http.as_ref())).unwrap();
        assert_eq!(
            failure.unwrap_err().to_string(),
            "Cloudflare TURN credential request answered 403"
        );
        let http = Recording::answering(vec![ok(200, json!({"iceServers": []}))]);
        let failure = poll_once(mint_cloudflare(&key, &account(), 600, UnixMillis(0), http.as_ref())).unwrap();
        assert_eq!(failure.unwrap_err(), ProviderFailure::NoRelayEntry);
        assert_eq!(ProviderUnreachable.to_string(), "relay provider request failed");
    }

    fn usage_answer(groups: Value) -> Result<ProviderAnswer, ProviderUnreachable> {
        ok(
            200,
            json!({"data": {"viewer": {"usage": [{"callsTurnUsageAdaptiveGroups": groups}]}}}),
        )
    }

    fn analytics() -> CloudflareAnalytics {
        CloudflareAnalytics {
            account_id: "acct0123".to_owned(),
            api_token: Secret::new("analytics-token".to_owned()),
        }
    }

    #[test]
    fn usage_query_text() {
        let http = Recording::answering(vec![usage_answer(json!([]))]);
        let now = UnixMillis(1_791_878_400_000);
        let read = poll_once(read_monthly_relay_egress_bytes(
            &analytics(),
            &account(),
            now,
            http.as_ref(),
        ))
        .unwrap();
        assert_eq!(read, Some(0.0));
        let request = &http.requests()[0];
        assert_eq!(request.url, CLOUDFLARE_GRAPHQL_API);
        assert_eq!(request.bearer.expose(), "analytics-token");
        assert_eq!(request.timeout_ms, 5000);
        let tag = relay_account_tag(&account());
        let query = format!(
            "query {{ viewer {{ usage: accounts(filter: {{ accountTag: \"acct0123\" }}) {{\n        callsTurnUsageAdaptiveGroups(filter: {{ date_geq: \"2026-10-01\", date_leq: \"2026-10-13\",\n            customIdentifier: \"{tag}\" }}, limit: 1, orderBy: [customIdentifier_ASC]) {{ sum {{ egressBytes }} }} }} }} }}"
        );
        assert_eq!(request.body, format!("{{\"query\":{}}}", json_string(&query)));
        assert!(request.body.contains("{\\n        callsTurn"));
        assert!(!request.body.contains("account-0001"));
    }

    #[test]
    fn usage_sums() {
        let read = |answer| {
            let http = Recording::answering(vec![answer]);
            poll_once(read_monthly_relay_egress_bytes(
                &analytics(),
                &account(),
                UnixMillis(0),
                http.as_ref(),
            ))
            .unwrap()
        };
        assert_eq!(
            read(usage_answer(
                json!([{"sum": {"egressBytes": "5"}}, {"sum": {"egressBytes": 7}}])
            )),
            Some(12.0)
        );
        assert_eq!(
            read(usage_answer(json!([{"sum": {"egressBytes": "x"}}, {"sum": {}}]))),
            Some(0.0)
        );
        assert_eq!(read(usage_answer(json!([{"sum": {"egressBytes": -3}}]))), None);
        assert_eq!(read(usage_answer(json!({}))), None);
        assert_eq!(read(ok(500, json!({}))), None);
        assert_eq!(read(Err(ProviderUnreachable)), None);
        assert_eq!(
            read(ok(200, json!({"errors": [{"message": "no"}], "data": null}))),
            None
        );
        let accounts = ok(
            200,
            json!({"data": {"viewer": {"usage": null,
            "accounts": [{"callsTurnUsageAdaptiveGroups": [{"sum": {"egressBytes": 9}}]}]}}}),
        );
        assert_eq!(read(accounts), Some(9.0));
    }

    #[test]
    fn quota_from_analytics_is_cached() {
        let mut quota_config = config(TurnProvider::Cloudflare);
        quota_config.analytics = Some(analytics());
        quota_config.monthly_quota_bytes = 1000;
        let http = Recording::answering(vec![usage_answer(json!([{"sum": {"egressBytes": 1000}}]))]);
        let grants = grants(quota_config, Arc::clone(&http));
        let first = poll_once(grants.grant(&account(), 600, UnixMillis(0))).unwrap();
        assert_eq!(first.relay, RelayStatus::Denied(RelayDenied::QuotaExceeded));
        let cached = poll_once(grants.grant(&account(), 600, UnixMillis(599_999))).unwrap();
        assert_eq!(cached.relay, RelayStatus::Denied(RelayDenied::QuotaExceeded));
        assert_eq!(http.requests().len(), 1);
        // The cache has lapsed and the analytics answer is gone: an unreadable figure does not block.
        let unreadable = Recording::answering(vec![Err(ProviderUnreachable), ok(200, cloudflare_answer())]);
        let grants = RelayGrants {
            http: unreadable,
            ..grants
        };
        let after = poll_once(grants.grant(&account(), 600, UnixMillis(600_000))).unwrap();
        assert_eq!(after.result_label(), "ISSUED");
    }

    #[test]
    fn usage_cache_and_hourly_counter() {
        let cache = RelayUsageCache::default();
        cache.set(&account(), "2026-10", 5.0, UnixMillis(0));
        assert_eq!(cache.get(&account(), "2026-10", UnixMillis(599_999)), Some(5.0));
        assert_eq!(cache.get(&account(), "2026-10", UnixMillis(600_000)), None);
        assert_eq!(cache.get(&account(), "2026-11", UnixMillis(1)), None);

        let counter = HourlyCounter::default();
        assert!(counter.allow("k", 2, UnixMillis(0)));
        assert!(counter.allow("k", 2, UnixMillis(1)));
        assert!(!counter.allow("k", 2, UnixMillis(2)));
        assert!(counter.allow("other", 2, UnixMillis(2)));
        assert!(counter.allow("k", 2, UnixMillis(3_600_000)));
        assert!(!counter.allow("k", 2, UnixMillis(3_600_000)));
        assert!(counter.allow("k", 2, UnixMillis(3_600_001)));
    }

    #[test]
    fn month_window_and_ttls() {
        let window = month_window(UnixMillis(1_791_878_400_000));
        assert_eq!(
            window,
            ("2026-10-01".to_owned(), "2026-10-13".to_owned(), "2026-10".to_owned())
        );
        let now = UnixMillis(1_000_000_000);
        assert_eq!(relay_ttl_seconds(UnixMillis(now.0 + 3_600_000), now, 10_800), 4200);
        assert_eq!(relay_ttl_seconds(UnixMillis(now.0 - 3_600_000), now, 10_800), 300);
        assert_eq!(relay_ttl_seconds(UnixMillis(now.0 + 1500), now, 10_800), 602);
        assert_eq!(relay_ttl_seconds(UnixMillis(now.0 + 86_400_000), now, 10_800), 10_800);
    }

    #[test]
    fn both_providers_through_the_issuer() {
        let http = Recording::answering(vec![ok(200, cloudflare_answer())]);
        let cloudflare = grants(config(TurnProvider::Cloudflare), Arc::clone(&http))
            .issuer()
            .unwrap();
        assert_eq!(cloudflare.provider(), TurnProvider::Cloudflare);
        let issued = poll_once(cloudflare.issue(account(), 600, UnixMillis(0)))
            .unwrap()
            .unwrap();
        assert_eq!(issued.credential.username, "cf-user");
        assert_eq!(
            issued.stun_urls,
            ["stun:stun.cloudflare.com:3478", "stun:extra.example:3478"]
        );

        let rest = grants(config(TurnProvider::TurnRest), http).issuer().unwrap();
        assert_eq!(rest.provider(), TurnProvider::TurnRest);
        let issued = poll_once(rest.issue(account(), 600, UnixMillis(0))).unwrap().unwrap();
        assert!(issued.stun_urls.is_empty());
        assert_eq!(issued.credential.urls, ["turn:relay.example:3478"]);
    }

    #[test]
    fn turn_rest_grant_lists_stun_then_turn() {
        let http = Recording::answering(Vec::new());
        let grant =
            poll_once(grants(config(TurnProvider::TurnRest), Arc::clone(&http)).grant(&account(), 600, UnixMillis(0)))
                .unwrap();
        assert!(http.requests().is_empty());
        assert_eq!(grant.ice_servers.len(), 2);
        assert_eq!(grant.ice_servers[1].urls, ["turn:relay.example:3478"]);
        assert!(grant.ice_servers[1].to_json().contains("\"credential\":\""));
        let debug = format!("{grant:?}");
        assert!(!debug.contains(grant.ice_servers[1].credential.as_ref().unwrap().expose()));
    }
}
